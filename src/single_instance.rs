//! Single-instance coordination over a loopback socket.
//!
//! A second launch asks the existing process to show its window and exits.
//! The operating system releases the loopback port when the process ends.
//!
//! The default profile keeps the fixed port as its only guard, so an upgrade
//! behaves exactly as before. A named profile locks its own state directory
//! instead: the lock is what keeps two copies off one session, and the port
//! is only the letterbox for `show`. Two profiles may then share a port
//! without either of them surfacing the other's window.

use std::io::{Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::paths::AppDirs;
use crate::profile::Profile;

/// Stable wire identity shared with FastsApp so upgrades surface a running
/// older copy before migrating its session files. A named profile appends its
/// name, which an older copy does not understand and therefore declines.
const PREFIX: &str = "fastsapp:";

/// How long a launch waits for the running copy to publish its port. The
/// holder of the lock writes the file just after binding, so a launch racing
/// it only has to wait out that gap.
const REACH_FOR: Duration = Duration::from_secs(2);
const BETWEEN_TRIES: Duration = Duration::from_millis(100);

pub enum Outcome {
    /// This process owns the instance guard.
    Only(Guard),
    /// The existing instance was asked to show its window.
    Surfaced,
    /// This profile is already running but did not answer. Starting anyway
    /// would put two copies on one session.
    Blocked,
}

/// Request from another launch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControlCommand {
    /// Shows or creates the window.
    Show,
    /// Reload local theme files without opening the window.
    ReloadThemes,
}

/// Owns the listener that marks this process as the running instance.
pub struct Guard {
    /// Requests queued by later launches.
    commands: Arc<Mutex<Vec<ControlCommand>>>,
    /// Held open for the life of the process: dropping the file, or dying,
    /// releases the profile for the next launch.
    _lock: Option<std::fs::File>,
}

impl Guard {
    /// Shared request queue drained by the app.
    pub fn commands(&self) -> Arc<Mutex<Vec<ControlCommand>>> {
        Arc::clone(&self.commands)
    }
}

/// One line of the control protocol: `fastsapp:show` for the default profile,
/// `fastsapp:show:work` for a named one.
fn line(profile: &Profile, verb: &str) -> String {
    if profile.is_default() {
        format!("{PREFIX}{verb}")
    } else {
        format!("{PREFIX}{verb}:{}", profile.name())
    }
}

/// Sends one request and verifies the reply comes from the same profile.
fn send_to(port: u16, profile: &Profile, verb: &str) -> std::io::Result<()> {
    let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port))?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.write_all(format!("{}\n", line(profile, verb)).as_bytes())?;
    // Read the one-line reply until the connection closes.
    let mut reply = String::new();
    stream.read_to_string(&mut reply)?;
    if reply.lines().next() == Some(line(profile, "ok").as_str()) {
        Ok(())
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "the port is held by something other than this profile",
        ))
    }
}

/// Sends one request to the copy of `dirs.profile` that is running. The port
/// it recorded wins over the configured one, so a profile that had to take a
/// free port is still reachable.
pub fn send(dirs: &AppDirs, port: u16, verb: &str) -> std::io::Result<()> {
    let mut failure = None;
    for candidate in recorded_port(&dirs.instance_port())
        .into_iter()
        .chain([port])
    {
        match send_to(candidate, &dirs.profile, verb) {
            Ok(()) => return Ok(()),
            Err(error) => failure = Some(error),
        }
    }
    Err(failure.unwrap_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::NotFound, "no running instance")
    }))
}

/// Claims the running-instance role for `dirs.profile`, listening on `port`.
pub fn acquire(waker: &crate::backend::Waker, dirs: &AppDirs, port: u16) -> Outcome {
    if dirs.profile.is_default() {
        return guard_by_port(waker, &dirs.profile, port);
    }
    // A named profile owns fresh directories, so its state can be created
    // before the migration that only the default profile runs.
    if let Err(error) = std::fs::create_dir_all(&dirs.state) {
        log::warn!("cannot create the profile state directory: {error}");
        return guard_by_port(waker, &dirs.profile, port);
    }
    match hold(&dirs.instance_lock()) {
        Held::Ours(file) => {
            let listener = listen(port);
            if let Some(listener) = &listener {
                publish_port(&dirs.instance_port(), listener);
            }
            Outcome::Only(serve_with(waker, &dirs.profile, listener, Some(file)))
        }
        Held::Theirs => reach(dirs, port),
        // Without a working lock, fall back to the port alone rather than
        // starting a second copy on the same session.
        Held::Unavailable(error) => {
            log::warn!("cannot lock the profile state directory: {error}");
            guard_by_port(waker, &dirs.profile, port)
        }
    }
}

/// The original guard: whoever binds the port is the running instance.
fn guard_by_port(waker: &crate::backend::Waker, profile: &Profile, port: u16) -> Outcome {
    let listener = match TcpListener::bind((Ipv4Addr::LOCALHOST, port)) {
        Ok(listener) => listener,
        Err(_) => {
            // If the port is held, continue only when it is not this profile.
            if send_to(port, profile, "show").is_ok() {
                return Outcome::Surfaced;
            }
            log::warn!("port {port} is busy but not with {profile}; running unguarded");
            return Outcome::Only(Guard {
                commands: Default::default(),
                _lock: None,
            });
        }
    };
    Outcome::Only(serve_with(waker, profile, Some(listener), None))
}

/// Starts the control thread and returns the guard that owns the profile.
fn serve_with(
    waker: &crate::backend::Waker,
    profile: &Profile,
    listener: Option<TcpListener>,
    lock: Option<std::fs::File>,
) -> Guard {
    let guard = Guard {
        commands: Default::default(),
        _lock: lock,
    };
    let Some(listener) = listener else {
        return guard;
    };
    let commands = Arc::clone(&guard.commands);
    let waker = waker.clone();
    let profile = profile.clone();
    let spawned = std::thread::Builder::new()
        .name("zapfast-instance".to_owned())
        .spawn(move || serve(listener, &profile, &commands, &waker));
    if let Err(error) = spawned {
        log::warn!("cannot listen for other launches: {error}");
    }
    guard
}

/// Binds the requested port, or any free one when it is taken by another
/// profile or program. The lock, not the port, is what guards the session.
fn listen(port: u16) -> Option<TcpListener> {
    match TcpListener::bind((Ipv4Addr::LOCALHOST, port)) {
        Ok(listener) => Some(listener),
        Err(error) => {
            log::warn!("port {port} is not available ({error}); taking a free one");
            match TcpListener::bind((Ipv4Addr::LOCALHOST, 0)) {
                Ok(listener) => Some(listener),
                Err(error) => {
                    log::warn!("cannot listen for other launches: {error}");
                    None
                }
            }
        }
    }
}

/// Opens the lock file and tries to take it for this process.
enum Held {
    Ours(std::fs::File),
    Theirs,
    Unavailable(std::io::Error),
}

fn hold(path: &Path) -> Held {
    let file = match std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(path)
    {
        Ok(file) => file,
        Err(error) => return Held::Unavailable(error),
    };
    match file.try_lock() {
        Ok(()) => Held::Ours(file),
        Err(std::fs::TryLockError::WouldBlock) => Held::Theirs,
        Err(std::fs::TryLockError::Error(error)) => Held::Unavailable(error),
    }
}

/// Records where this profile listens, so a later launch can reach it even
/// when the requested port was taken. Written whole, then renamed into place.
fn publish_port(path: &Path, listener: &TcpListener) {
    let Ok(address) = listener.local_addr() else {
        return;
    };
    let temporary = path.with_extension("port.tmp");
    let written = std::fs::write(&temporary, address.port().to_string().as_bytes())
        .and_then(|()| std::fs::rename(&temporary, path));
    if let Err(error) = written {
        log::warn!("cannot record the instance port: {error}");
    }
}

fn recorded_port(path: &Path) -> Option<u16> {
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}

/// Asks the copy that holds the lock to show its window. It may not have
/// published its port yet, so keep trying for a moment before giving up.
fn reach(dirs: &AppDirs, port: u16) -> Outcome {
    let deadline = Instant::now() + REACH_FOR;
    loop {
        if send(dirs, port, "show").is_ok() {
            return Outcome::Surfaced;
        }
        if Instant::now() >= deadline {
            return Outcome::Blocked;
        }
        std::thread::sleep(BETWEEN_TRIES);
    }
}

/// Handles one request and reply per connection until the listener closes.
fn serve(
    listener: TcpListener,
    profile: &Profile,
    commands: &Mutex<Vec<ControlCommand>>,
    waker: &crate::backend::Waker,
) {
    for mut stream in listener.incoming().flatten() {
        let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
        let Some(line) = read_line(&mut stream) else {
            continue;
        };
        // Ignore clients without the ZapFast prefix, and launches of another
        // profile that happen to share this port.
        if let Some(command) = parse(&line, profile) {
            let _ = stream.write_all(format!("{}\n", self::line(profile, "ok")).as_bytes());
            commands
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .push(command);
            waker.wake();
        }
    }
}

fn parse(line: &str, profile: &Profile) -> Option<ControlCommand> {
    let request = line.trim_end().strip_prefix(PREFIX)?;
    // An unnamed request is FastsApp or an older ZapFast: the default profile.
    let (verb, name) = request
        .split_once(':')
        .unwrap_or((request, crate::profile::DEFAULT));
    if name != profile.name() {
        return None;
    }
    match verb {
        "show" => Some(ControlCommand::Show),
        "reload-themes" => Some(ControlCommand::ReloadThemes),
        _ => None,
    }
}

/// Reads a bounded line and rejects read errors or oversized input.
fn read_line(stream: &mut TcpStream) -> Option<String> {
    let mut buffer = [0u8; 256];
    let mut filled = 0;
    loop {
        if filled == buffer.len() {
            return None;
        }
        match stream.read(&mut buffer[filled..]) {
            Ok(0) => break,
            Ok(read) => {
                filled += read;
                if buffer[..filled].contains(&b'\n') {
                    break;
                }
            }
            Err(_) => return None,
        }
    }
    let line = buffer[..filled].split(|&byte| byte == b'\n').next()?;
    String::from_utf8(line.to_vec()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root(name: &str) -> std::path::PathBuf {
        let root =
            std::env::temp_dir().join(format!("zapfast-instance-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn only_our_own_show_is_understood() {
        let default = Profile::default();
        assert_eq!(
            parse("fastsapp:show\n", &default),
            Some(ControlCommand::Show)
        );
        assert_eq!(parse("fastsapp:show", &default), Some(ControlCommand::Show));
        assert_eq!(parse("GET / HTTP/1.1", &default), None);
        assert_eq!(parse("fastsapp:frobnicate", &default), None);
        assert_eq!(parse("", &default), None);
    }

    /// Two profiles may end up on one port; neither may raise the other.
    #[test]
    fn a_show_for_another_profile_is_declined() {
        let default = Profile::default();
        let work: Profile = "work".parse().unwrap();
        assert_eq!(
            parse("fastsapp:show:work", &work),
            Some(ControlCommand::Show)
        );
        // An older copy speaks for the default profile only.
        assert_eq!(parse("fastsapp:show", &work), None);
        assert_eq!(parse("fastsapp:show:work", &default), None);
        assert_eq!(parse("fastsapp:show:other", &work), None);
    }

    /// Verifies a request crosses the socket into the app queue.
    #[test]
    fn a_second_launch_reaches_the_queue() {
        for profile in [Profile::default(), "work".parse().unwrap()] {
            let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("a loopback port");
            let port = listener.local_addr().expect("a bound address").port();
            let commands: Arc<Mutex<Vec<ControlCommand>>> = Default::default();
            let served = {
                let commands = Arc::clone(&commands);
                let waker = crate::backend::Waker::default();
                let profile = profile.clone();
                std::thread::spawn(move || serve(listener, &profile, &commands, &waker))
            };

            send_to(port, &profile, "show").expect("answered as this profile");
            // Unknown verbs close the connection without a reply.
            assert!(send_to(port, &profile, "frobnicate").is_err());
            // So does a launch of a different profile on the same port.
            let other: Profile = "other".parse().unwrap();
            assert!(send_to(port, &other, "show").is_err());

            assert_eq!(
                *commands.lock().expect("the queue"),
                vec![ControlCommand::Show]
            );
            drop(served);
        }
    }

    /// The lock, not the port, is what keeps two copies off one session.
    #[test]
    fn a_profile_is_locked_while_it_runs() {
        let root = root("lock");
        let path = root.join("instance.lock");
        let first = match hold(&path) {
            Held::Ours(file) => file,
            _ => panic!("the first launch takes the lock"),
        };
        assert!(matches!(hold(&path), Held::Theirs));
        drop(first);
        assert!(matches!(hold(&path), Held::Ours(_)));
        std::fs::remove_dir_all(root).unwrap();
    }

    /// A launch that cannot take the lock finds the port in the state file,
    /// which is what lets two profiles share the configured port.
    #[test]
    fn the_running_copy_publishes_the_port_it_took() {
        let root = root("port");
        let path = root.join("instance.port");
        assert_eq!(recorded_port(&path), None);
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("a loopback port");
        let port = listener.local_addr().expect("a bound address").port();
        publish_port(&path, &listener);
        assert_eq!(recorded_port(&path), Some(port));
        // A later run overwrites a stale file rather than appending to it.
        let again = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("a loopback port");
        publish_port(&path, &again);
        assert_eq!(
            recorded_port(&path),
            Some(again.local_addr().expect("a bound address").port())
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    /// Nobody is listening, so the launch reports the profile as busy instead
    /// of opening a second copy of it.
    #[test]
    fn an_unreachable_profile_blocks_the_launch() {
        let root = root("blocked");
        let unreachable = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("a loopback port");
        let port = unreachable.local_addr().expect("a bound address").port();
        drop(unreachable);
        let dirs = AppDirs {
            profile: "work".parse().unwrap(),
            ..AppDirs::under(&root)
        };
        assert!(matches!(reach(&dirs, port), Outcome::Blocked));
        std::fs::remove_dir_all(root).unwrap();
    }
}
