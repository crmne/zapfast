//! One running ZapFast per user, through fastframe-instance.
//!
//! The crate holds the lock in the per-user runtime directory and serves the
//! private channel a later launch hands its request over (a socket only the
//! user can open, or a token-checked loopback port on Windows). ZapFast keeps
//! its verbs, its directory, and the `fastsapp:` prefix, so a new launch still
//! reaches an older copy that is already running, and an older launch this
//! one. Copies from before the lock (0.15 and earlier) only know a fixed
//! port, which this module still answers.

use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Stable wire identity shared with FastsApp so upgrades surface a running
/// older copy before migrating its session files. On the wire every request
/// and reply starts with `fastsapp:`.
const NAME: &str = "fastsapp";
const PREFIX: &str = "fastsapp:";
/// What ZapFast answers a request it takes: `fastsapp:ok` on the wire.
const OK: &str = "ok";

/// Fixed port of copies that predate the lock. They never take it.
const LEGACY_PORT: u16 = 47_119;
/// Longest request accepted on the legacy port.
const LEGACY_REQUEST_LIMIT: usize = 256;
/// Time a client gets to send its whole request on the legacy port.
const REQUEST_TIME: Duration = Duration::from_secs(1);
/// Time to wait for an older copy's reply.
const REPLY_TIME: Duration = Duration::from_secs(5);

pub enum Outcome {
    /// This process owns the instance guard.
    Only(Guard),
    /// Another instance is running and received the request.
    Surfaced,
    /// Another instance holds the lock but did not answer.
    Unanswered,
}

/// Request from another launch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControlCommand {
    /// Shows or creates the window.
    Show,
    /// Reload local theme files without opening the window.
    ReloadThemes,
    /// Quits, as the tray's Quit does: from the launcher's Quit action.
    Quit,
    /// Confirms an instance is running and changes nothing.
    Ping,
}

type Queue = Arc<Mutex<Vec<ControlCommand>>>;

/// Marks this process as the running instance until it exits.
pub struct Guard {
    /// Requests queued by later launches.
    commands: Queue,
    /// Held until the process exits.
    _claim: fastframe_instance::Guard,
}

impl Guard {
    /// Shared request queue drained by the app.
    pub fn commands(&self) -> Queue {
        Arc::clone(&self.commands)
    }
}

/// ZapFast's slot: its runtime directory, with the prefix older copies use.
fn slot(dir: &Path) -> fastframe_instance::Slot {
    fastframe_instance::Slot::at(dir, NAME)
}

/// Becomes the running instance, or hands `verb` to the one already running.
pub fn acquire(dir: &Path, waker: &crate::backend::Waker, verb: &str) -> Outcome {
    let commands = Queue::default();
    let claim = match claim(dir, verb, &commands, waker) {
        fastframe_instance::Claim::First(claim) => claim,
        fastframe_instance::Claim::Running(_) => return Outcome::Surfaced,
        // A copy that refuses the verb is reported as before, when it
        // simply did not answer.
        fastframe_instance::Claim::Unanswered | fastframe_instance::Claim::Declined => {
            return Outcome::Unanswered;
        }
    };
    if legacy_instance_answers(verb) {
        return Outcome::Surfaced;
    }
    listen_legacy(Arc::clone(&commands), waker.clone());
    Outcome::Only(Guard {
        commands,
        _claim: claim,
    })
}

/// Takes the slot, queueing what later launches ask for, or hands `verb` to
/// the copy that holds it.
fn claim(
    dir: &Path,
    verb: &str,
    commands: &Queue,
    waker: &crate::backend::Waker,
) -> fastframe_instance::Claim {
    let (commands, waker) = (Arc::clone(commands), waker.clone());
    slot(dir).claim(verb, move |request| {
        let command = parse(request)?;
        commands
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(command);
        waker.wake();
        Some(OK.to_owned())
    })
}

/// Sends one request to the running instance. An error of kind `NotFound`
/// or `ConnectionRefused` means none is running.
pub fn send(dir: &Path, verb: &str) -> std::io::Result<()> {
    slot(dir).send(verb).map(drop)
}

/// The verbs another launch may send. Anything else is declined.
fn parse(verb: &str) -> Option<ControlCommand> {
    match verb {
        "show" => Some(ControlCommand::Show),
        "reload-themes" => Some(ControlCommand::ReloadThemes),
        "quit" => Some(ControlCommand::Quit),
        "ping" => Some(ControlCommand::Ping),
        _ => None,
    }
}

/// Asks a running copy that predates the lock to handle `verb`.
fn legacy_instance_answers(verb: &str) -> bool {
    // The port is free, and released at once, when no older copy runs.
    if TcpListener::bind((Ipv4Addr::LOCALHOST, LEGACY_PORT)).is_ok() {
        return false;
    }
    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, LEGACY_PORT));
    let answered = TcpStream::connect_timeout(&address, REPLY_TIME)
        .and_then(|stream| legacy_request(stream, verb))
        .is_ok();
    // A background start never runs beside a copy that may be ZapFast,
    // including older ones that do not answer `ping`.
    answered || verb == "ping"
}

/// Sends `verb` the way copies before the lock expect it, and checks the
/// reply.
fn legacy_request(mut stream: TcpStream, verb: &str) -> std::io::Result<()> {
    stream.set_read_timeout(Some(REPLY_TIME))?;
    stream.set_write_timeout(Some(REPLY_TIME))?;
    stream.write_all(format!("{PREFIX}{verb}\n").as_bytes())?;
    let mut reply = String::new();
    stream
        .take(LEGACY_REQUEST_LIMIT as u64)
        .read_to_string(&mut reply)?;
    if reply.lines().next() == Some(&format!("{PREFIX}{OK}")) {
        Ok(())
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "the port is held by something other than ZapFast",
        ))
    }
}

/// Answers copies that predate the lock (0.15 and earlier), which only look
/// for the fixed port: without a reply they would start beside this one on
/// the same archive and linked device. Only `show` and `ping` are accepted
/// there, as nothing on that port proves who is asking, and they at most
/// bring the window forward.
fn listen_legacy(commands: Queue, waker: crate::backend::Waker) {
    let listener = match TcpListener::bind((Ipv4Addr::LOCALHOST, LEGACY_PORT)) {
        Ok(listener) => listener,
        Err(error) => {
            log::debug!("cannot answer older launches: {error}");
            return;
        }
    };
    let spawned = std::thread::Builder::new()
        .name("zapfast-legacy-instance".to_owned())
        .spawn(move || serve_legacy(listener, &commands, &waker));
    if let Err(error) = spawned {
        log::debug!("cannot answer older launches: {error}");
    }
}

fn serve_legacy(
    listener: TcpListener,
    commands: &Mutex<Vec<ControlCommand>>,
    waker: &crate::backend::Waker,
) {
    for mut stream in listener.incoming().flatten() {
        let command = read_legacy_line(&mut stream)
            .as_deref()
            .and_then(|line| line.trim_end().strip_prefix(PREFIX))
            .and_then(parse);
        if let Some(command @ (ControlCommand::Show | ControlCommand::Ping)) = command {
            let _ = stream.write_all(format!("{PREFIX}{OK}\n").as_bytes());
            commands
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .push(command);
            waker.wake();
        }
    }
}

/// Reads one line within the size and time limits. Refuses read errors,
/// oversized input, and clients that stall.
fn read_legacy_line(stream: &mut TcpStream) -> Option<String> {
    let deadline = Instant::now() + REQUEST_TIME;
    let mut buffer = [0u8; LEGACY_REQUEST_LIMIT];
    let mut filled = 0;
    loop {
        if filled == buffer.len() {
            return None;
        }
        let left = deadline.checked_duration_since(Instant::now())?;
        stream
            .set_read_timeout(Some(left.max(Duration::from_millis(1))))
            .ok()?;
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

    #[test]
    fn only_our_own_verbs_are_understood() {
        assert_eq!(parse("show"), Some(ControlCommand::Show));
        assert_eq!(parse("ping"), Some(ControlCommand::Ping));
        assert_eq!(parse("reload-themes"), Some(ControlCommand::ReloadThemes));
        assert_eq!(parse("quit"), Some(ControlCommand::Quit));
        assert_eq!(parse("GET / HTTP/1.1"), None);
        assert_eq!(parse("frobnicate"), None);
        assert_eq!(parse(""), None);
    }

    fn connect(port: u16) -> TcpStream {
        TcpStream::connect((Ipv4Addr::LOCALHOST, port)).expect("a connection")
    }

    #[test]
    fn older_copies_may_only_ask_for_the_window() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let commands = Queue::default();
        let queue = Arc::clone(&commands);
        std::thread::spawn(move || {
            serve_legacy(listener, &queue, &crate::backend::Waker::default())
        });
        legacy_request(connect(port), "show").expect("an older launch surfaces this one");
        legacy_request(connect(port), "ping").expect("a background start sees this one");
        assert!(legacy_request(connect(port), "reload-themes").is_err());
        assert_eq!(
            *commands.lock().unwrap(),
            vec![ControlCommand::Show, ControlCommand::Ping]
        );
    }

    /// Sends `line` the way a copy from before fastframe-instance does, with
    /// `token` first on Windows, and returns the raw reply.
    fn raw_request(dir: &Path, line: &str, token: Option<&str>) -> String {
        #[cfg(unix)]
        let mut stream = {
            let _ = token;
            std::os::unix::net::UnixStream::connect(dir.join("instance.sock")).unwrap()
        };
        #[cfg(not(unix))]
        let mut stream = {
            let key = std::fs::read_to_string(dir.join("instance.key")).unwrap();
            let mut key = key.lines();
            let port: u16 = key.next().unwrap().parse().unwrap();
            let written = key.next().unwrap().to_owned();
            let mut stream = connect(port);
            let token = token.map_or(written, str::to_owned);
            stream.write_all(format!("{token}\n").as_bytes()).unwrap();
            stream
        };
        stream.write_all(format!("{line}\n").as_bytes()).unwrap();
        let mut reply = String::new();
        let _ = stream.read_to_string(&mut reply);
        reply
    }

    /// A second launch reaches the first, whether it is this version or one
    /// from before fastframe-instance, which writes and expects the same
    /// lines: `fastsapp:show` in, `fastsapp:ok` out. Themes reload without a
    /// window. Unknown verbs are declined, and on Windows a wrong token gets
    /// no reply.
    #[test]
    fn a_second_launch_reaches_the_queue_on_the_old_wire() {
        let home = tempfile::tempdir().unwrap();
        let dir = home.path().join("runtime");
        let waker = crate::backend::Waker::default();
        let commands = Queue::default();
        let fastframe_instance::Claim::First(first) = claim(&dir, "show", &commands, &waker) else {
            panic!("the first launch takes the slot");
        };

        // A launch of this version.
        let second = claim(&dir, "show", &Queue::default(), &waker);
        assert!(matches!(second, fastframe_instance::Claim::Running(reply) if reply == OK));
        send(&dir, "reload-themes").expect("themes reload without a window");
        let declined = send(&dir, "frobnicate").unwrap_err();
        assert_eq!(declined.kind(), std::io::ErrorKind::PermissionDenied);
        assert!(matches!(
            claim(&dir, "frobnicate", &Queue::default(), &waker),
            fastframe_instance::Claim::Declined
        ));

        // A launch of an older version.
        assert_eq!(raw_request(&dir, "fastsapp:show", None), "fastsapp:ok\n");
        assert_eq!(
            raw_request(&dir, "fastsapp:frobnicate", None),
            "fastsapp!declined\n",
            "which an older copy reads as no answer"
        );
        #[cfg(not(unix))]
        assert_eq!(
            raw_request(&dir, "fastsapp:show", Some(&"0".repeat(64))),
            "",
            "a wrong token is refused"
        );

        assert_eq!(
            *commands.lock().unwrap(),
            vec![
                ControlCommand::Show,
                ControlCommand::ReloadThemes,
                ControlCommand::Show,
            ]
        );
        drop(first);
    }
}
