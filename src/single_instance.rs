//! Single-instance coordination over a private local channel.
//!
//! The running instance holds an exclusive lock on a file in the per-user
//! runtime directory. The operating system releases the lock when the process
//! ends, even after a crash, so leftover files never block a later launch. A
//! second launch sends the running instance one request and exits.
//!
//! On Unix requests travel over a socket in that directory, which only the
//! user can open. Windows listens on an ephemeral loopback port that any local
//! process can reach, so every connection must first present a random token
//! that the running instance writes to the directory.

use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const LOCK_FILE: &str = "instance.lock";
#[cfg(unix)]
const SOCKET_FILE: &str = "instance.sock";
#[cfg(any(not(unix), test))]
const KEY_FILE: &str = "instance.key";

/// Fixed port of copies that predate the lock. They never take it.
const LEGACY_PORT: u16 = 47_119;

/// Stable wire identity shared with FastsApp so upgrades surface a running
/// older copy before migrating its session files.
const PREFIX: &str = "fastsapp:";
const OK_REPLY: &str = "fastsapp:ok";

/// Longest request accepted, token included.
const REQUEST_LIMIT: usize = 256;
/// Time a client gets to send its whole request. Requests are served one at
/// a time, so this bounds how long a stray connection holds up the others.
const REQUEST_TIME: Duration = Duration::from_secs(1);
/// Time a client waits for the reply, which may queue behind a stray one.
const REPLY_TIME: Duration = Duration::from_secs(5);
/// How long a later launch waits for a starting instance to listen.
const STARTUP_WAIT: Duration = Duration::from_secs(3);

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
    /// Confirms an instance is running and changes nothing.
    Ping,
}

type Queue = Arc<Mutex<Vec<ControlCommand>>>;

/// Owns the lock that marks this process as the running instance.
pub struct Guard {
    /// Requests queued by later launches.
    commands: Queue,
    /// Held until the process exits.
    _lock: Option<std::fs::File>,
}

impl Guard {
    /// Shared request queue drained by the app.
    pub fn commands(&self) -> Queue {
        Arc::clone(&self.commands)
    }
}

/// Becomes the running instance, or hands `verb` to the one already running.
pub fn acquire(dir: &Path, waker: &crate::backend::Waker, verb: &str) -> Outcome {
    let lock = match lock(dir) {
        Ok(Some(lock)) => lock,
        Ok(None) => return hand_over(dir, verb),
        Err(error) => {
            log::warn!("cannot take the instance lock; running unguarded: {error}");
            return Outcome::Only(Guard {
                commands: Default::default(),
                _lock: None,
            });
        }
    };
    if legacy_instance_answers(verb) {
        return Outcome::Surfaced;
    }
    let guard = Guard {
        commands: Default::default(),
        _lock: Some(lock),
    };
    if let Err(error) = listen(dir, guard.commands(), waker.clone()) {
        log::warn!("cannot listen for other launches: {error}");
    }
    Outcome::Only(guard)
}

/// Takes the instance lock, or returns `None` when another process holds it.
fn lock(dir: &Path) -> std::io::Result<Option<std::fs::File>> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    let mut options = std::fs::OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
        builder.mode(0o700);
        options.mode(0o600);
        builder.create(dir)?;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    #[cfg(not(unix))]
    builder.create(dir)?;
    let file = options.open(dir.join(LOCK_FILE))?;
    match file.try_lock() {
        Ok(()) => Ok(Some(file)),
        Err(std::fs::TryLockError::WouldBlock) => Ok(None),
        Err(std::fs::TryLockError::Error(error)) => Err(error),
    }
}

/// Sends `verb` to the lock holder, waiting while it may still be starting.
fn hand_over(dir: &Path, verb: &str) -> Outcome {
    let deadline = Instant::now() + STARTUP_WAIT;
    loop {
        match send(dir, verb) {
            Ok(()) => return Outcome::Surfaced,
            Err(error) if Instant::now() >= deadline => {
                log::warn!("the running instance did not answer: {error}");
                return Outcome::Unanswered;
            }
            Err(_) => std::thread::sleep(Duration::from_millis(100)),
        }
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
        .and_then(|stream| request(stream, None, verb))
        .is_ok();
    // A background start never runs beside a copy that may be ZapFast,
    // including older ones that do not answer `ping`.
    answered || verb == "ping"
}

/// Sends one request to the running instance and verifies its reply.
#[cfg(unix)]
pub fn send(dir: &Path, verb: &str) -> std::io::Result<()> {
    let stream = std::os::unix::net::UnixStream::connect(dir.join(SOCKET_FILE))?;
    request(stream, None, verb)
}

/// Sends one request to the running instance and verifies its reply.
#[cfg(not(unix))]
pub fn send(dir: &Path, verb: &str) -> std::io::Result<()> {
    let (port, token) = read_key(dir)?;
    let stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port))?;
    request(stream, Some(&token), verb)
}

/// Listens on a socket only the user can open. Only the lock holder gets
/// here, so a socket that already exists was left by an instance that ended.
#[cfg(unix)]
fn listen(dir: &Path, commands: Queue, waker: crate::backend::Waker) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let path = dir.join(SOCKET_FILE);
    match std::fs::remove_file(&path) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error),
        _ => {}
    }
    let listener = std::os::unix::net::UnixListener::bind(&path)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    spawn(move || serve(listener.incoming(), None, &commands, &waker))
}

/// Listens on an ephemeral loopback port and publishes it with a new token.
/// Only the lock holder gets here, so it replaces the file of an earlier run.
#[cfg(not(unix))]
fn listen(dir: &Path, commands: Queue, waker: crate::backend::Waker) -> std::io::Result<()> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    let token = new_token()?;
    write_key(dir, listener.local_addr()?.port(), &token)?;
    spawn(move || serve(listener.incoming(), Some(&token), &commands, &waker))
}

fn spawn(serve: impl FnOnce() + Send + 'static) -> std::io::Result<()> {
    std::thread::Builder::new()
        .name("zapfast-instance".to_owned())
        .spawn(serve)
        .map(drop)
}

/// Secret that authenticates requests over loopback TCP, as 64 hex digits.
#[cfg(any(not(unix), test))]
fn new_token() -> std::io::Result<String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(std::io::Error::other)?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

/// Compares without stopping at the first difference, so response time
/// reveals nothing about the token.
fn token_matches(expected: &str, presented: &[u8]) -> bool {
    let expected = expected.as_bytes();
    presented.len() == expected.len()
        && presented
            .iter()
            .zip(expected)
            .fold(0u8, |difference, (a, b)| difference | (a ^ b))
            == 0
}

/// Writes the port and token readable only by the user. Windows keeps the
/// runtime directory in the user's profile, which other users cannot read.
#[cfg(any(not(unix), test))]
fn write_key(dir: &Path, port: u16, token: &str) -> std::io::Result<()> {
    let partial = dir.join(format!("{KEY_FILE}.partial"));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
        .open(&partial)?
        .write_all(format!("{port}\n{token}\n").as_bytes())?;
    // Clients never read a half-written file.
    std::fs::rename(partial, dir.join(KEY_FILE))
}

#[cfg(any(not(unix), test))]
fn read_key(dir: &Path) -> std::io::Result<(u16, String)> {
    let text = std::fs::read_to_string(dir.join(KEY_FILE))?;
    let mut lines = text.lines();
    let port = lines.next().and_then(|port| port.parse().ok());
    match (port, lines.next()) {
        (Some(port), Some(token)) => Ok((port, token.to_owned())),
        _ => Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "the instance key file is damaged",
        )),
    }
}

/// Stream either side of the control channel.
trait Connection: Read + Write {
    fn set_timeouts(&self, timeout: Option<Duration>) -> std::io::Result<()>;
}

impl Connection for TcpStream {
    fn set_timeouts(&self, timeout: Option<Duration>) -> std::io::Result<()> {
        self.set_read_timeout(timeout)?;
        self.set_write_timeout(timeout)
    }
}

#[cfg(unix)]
impl Connection for std::os::unix::net::UnixStream {
    fn set_timeouts(&self, timeout: Option<Duration>) -> std::io::Result<()> {
        self.set_read_timeout(timeout)?;
        self.set_write_timeout(timeout)
    }
}

/// Sends the token line, when there is one, and the verb, then verifies the
/// ZapFast reply prefix.
fn request(mut stream: impl Connection, token: Option<&str>, verb: &str) -> std::io::Result<()> {
    stream.set_timeouts(Some(REPLY_TIME))?;
    let token = token.map(|token| format!("{token}\n")).unwrap_or_default();
    stream.write_all(format!("{token}{PREFIX}{verb}\n").as_bytes())?;
    // Read the one-line reply until the connection closes.
    let mut reply = String::new();
    stream
        .take(REQUEST_LIMIT as u64)
        .read_to_string(&mut reply)?;
    if reply.lines().next() == Some(OK_REPLY) {
        Ok(())
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "the channel is held by something other than ZapFast",
        ))
    }
}

/// Handles one request and reply per connection until the listener closes.
fn serve<C: Connection>(
    incoming: impl Iterator<Item = std::io::Result<C>>,
    token: Option<&str>,
    commands: &Mutex<Vec<ControlCommand>>,
    waker: &crate::backend::Waker,
) {
    for mut stream in incoming.flatten() {
        // Ignore clients without the token or the ZapFast prefix.
        if let Some(command) = receive(&mut stream, token) {
            let _ = stream.write_all(format!("{OK_REPLY}\n").as_bytes());
            commands
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .push(command);
            waker.wake();
        }
    }
}

/// Reads one request, rejecting it unless its first line is the token when
/// the transport needs one.
fn receive(stream: &mut impl Connection, token: Option<&str>) -> Option<ControlCommand> {
    let lines = 1 + usize::from(token.is_some());
    let request = read_lines(stream, lines)?;
    let mut request = request.split(|&byte| byte == b'\n');
    if let Some(token) = token
        && !token_matches(token, request.next()?)
    {
        return None;
    }
    parse(std::str::from_utf8(request.next()?).ok()?)
}

fn parse(line: &str) -> Option<ControlCommand> {
    match line.trim_end().strip_prefix(PREFIX)? {
        "show" => Some(ControlCommand::Show),
        "reload-themes" => Some(ControlCommand::ReloadThemes),
        "ping" => Some(ControlCommand::Ping),
        _ => None,
    }
}

/// Reads until `lines` newlines within the size and time limits. Rejects read
/// errors, oversized input, and clients that stall.
fn read_lines(stream: &mut impl Connection, lines: usize) -> Option<Vec<u8>> {
    let deadline = Instant::now() + REQUEST_TIME;
    let mut buffer = [0u8; REQUEST_LIMIT];
    let mut filled = 0;
    loop {
        if filled == buffer.len() {
            return None;
        }
        let left = deadline.checked_duration_since(Instant::now())?;
        stream
            .set_timeouts(Some(left.max(Duration::from_millis(1))))
            .ok()?;
        match stream.read(&mut buffer[filled..]) {
            Ok(0) => break,
            Ok(read) => {
                filled += read;
                let newlines = buffer[..filled].iter().filter(|&&b| b == b'\n').count();
                if newlines >= lines {
                    break;
                }
            }
            Err(_) => return None,
        }
    }
    Some(buffer[..filled].to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_our_own_show_is_understood() {
        assert_eq!(parse("fastsapp:show\n"), Some(ControlCommand::Show));
        assert_eq!(parse("fastsapp:show"), Some(ControlCommand::Show));
        assert_eq!(parse("fastsapp:ping"), Some(ControlCommand::Ping));
        assert_eq!(parse("GET / HTTP/1.1"), None);
        assert_eq!(parse("fastsapp:frobnicate"), None);
        assert_eq!(parse(""), None);
    }

    fn dir(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("zapfast-instance-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    /// Serves loopback TCP with a token, as on Windows.
    fn tcp_server() -> (u16, String, Queue) {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("a loopback port");
        let port = listener.local_addr().expect("a bound address").port();
        let token = new_token().expect("random bytes");
        let served = token.clone();
        let commands: Queue = Default::default();
        let queue = Arc::clone(&commands);
        std::thread::spawn(move || {
            let waker = crate::backend::Waker::default();
            serve(listener.incoming(), Some(&served), &queue, &waker);
        });
        (port, token, commands)
    }

    fn connect(port: u16) -> TcpStream {
        TcpStream::connect((Ipv4Addr::LOCALHOST, port)).expect("a connection")
    }

    #[test]
    fn tokens_are_random_and_compared_whole() {
        let token = new_token().unwrap();
        assert_eq!(token.len(), 64);
        assert_ne!(token, new_token().unwrap());
        assert!(token_matches(&token, token.as_bytes()));
        assert!(!token_matches(&token, &token.as_bytes()[..63]));
        assert!(!token_matches(&token, format!("{token}0").as_bytes()));
        assert!(!token_matches(&token, b""));
    }

    #[test]
    fn tcp_requests_need_the_token() {
        let (port, token, commands) = tcp_server();
        request(connect(port), Some(&token), "show").expect("answered as ZapFast");
        let wrong = new_token().unwrap();
        assert!(request(connect(port), Some(&wrong), "show").is_err());
        assert!(request(connect(port), None, "reload-themes").is_err());
        // Browsers reaching localhost send HTTP, which never carries the token.
        let mut browser = connect(port);
        browser
            .write_all(b"GET /fastsapp:show HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .unwrap();
        let mut reply = Vec::new();
        browser.read_to_end(&mut reply).unwrap();
        assert!(reply.is_empty());
        assert!(request(connect(port), Some(&token), "frobnicate").is_err());
        assert_eq!(*commands.lock().unwrap(), vec![ControlCommand::Show]);
    }

    #[test]
    fn oversized_and_stalled_clients_do_not_block_the_listener() {
        let (port, token, commands) = tcp_server();
        let mut flood = connect(port);
        // The listener stops reading at the limit and closes the connection,
        // so later writes may fail.
        let _ = flood.write_all(&[b'a'; REQUEST_LIMIT * 4]);
        // A client that sends a partial request and then waits is dropped
        // when its time runs out.
        let mut stalled = connect(port);
        stalled.write_all(token.as_bytes()).unwrap();
        let started = Instant::now();
        request(connect(port), Some(&token), "ping").expect("served after the others");
        assert!(started.elapsed() < REQUEST_TIME * 3);
        let mut reply = Vec::new();
        stalled.read_to_end(&mut reply).unwrap();
        assert!(reply.is_empty());
        assert_eq!(*commands.lock().unwrap(), vec![ControlCommand::Ping]);
    }

    #[test]
    fn key_file_round_trips_and_stays_private() {
        let dir = dir("key");
        std::fs::create_dir_all(&dir).unwrap();
        let token = new_token().unwrap();
        write_key(&dir, 4242, &token).unwrap();
        assert_eq!(read_key(&dir).unwrap(), (4242, token));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.join(KEY_FILE))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// Verifies a request crosses the socket into the app queue, that the
    /// lock keeps a second holder out, and that files are private.
    #[cfg(unix)]
    #[test]
    fn a_second_launch_reaches_the_queue() {
        use std::os::unix::fs::PermissionsExt;

        let dir = dir("unix");
        let first = lock(&dir).unwrap().expect("the first lock");
        assert!(lock(&dir).unwrap().is_none());
        // A socket left by a crash is replaced.
        std::fs::write(dir.join(SOCKET_FILE), b"stale").unwrap();
        let commands: Queue = Default::default();
        listen(
            &dir,
            Arc::clone(&commands),
            crate::backend::Waker::default(),
        )
        .unwrap();

        send(&dir, "show").expect("answered as ZapFast");
        // Unknown verbs close the connection without a reply.
        assert!(send(&dir, "frobnicate").is_err());
        assert_eq!(*commands.lock().unwrap(), vec![ControlCommand::Show]);

        let mode = |name: &str| {
            let path = if name.is_empty() {
                dir.clone()
            } else {
                dir.join(name)
            };
            std::fs::metadata(path).unwrap().permissions().mode() & 0o777
        };
        assert_eq!(mode(""), 0o700);
        assert_eq!(mode(LOCK_FILE), 0o600);
        assert_eq!(mode(SOCKET_FILE), 0o600);

        // The lock is free again once its holder is gone.
        drop(first);
        assert!(lock(&dir).unwrap().is_some());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
