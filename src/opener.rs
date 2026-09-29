//! Opens a local file whose opening must be seen to work, such as the log.
//!
//! `open::that_detached` reports success once a launcher starts, so on Linux
//! a `.log` file with no usable handler opened nothing and said nothing:
//! `xdg-open` outside GNOME and KDE runs the default application itself, and
//! runs a terminal program (Neovim is Omarchy's text editor) without a
//! terminal. Here each way of opening is waited for, and a failure moves on to
//! showing the file in its folder, so the reader always ends up somewhere, or
//! learns that nothing worked.

use std::path::Path;

/// Opens `path` in its default application or, failing that, shows it in its
/// folder. Blocks for up to a few seconds per attempt, so call it off the
/// interface thread. The error names the file so the reader can go there.
pub fn open_or_reveal(path: &Path) -> Result<(), String> {
    first_success(&attempts(path)).map_err(|error| {
        format!(
            "Could not open {}: {error}. Open it from its folder instead.",
            path.display()
        )
    })
}

type Attempt<'a> = Box<dyn Fn() -> Result<(), String> + 'a>;

/// Runs the attempts in order and stops at the first that works; if none
/// does, the last error explains why.
fn first_success(attempts: &[Attempt<'_>]) -> Result<(), String> {
    let mut last = String::from("no way to open files was found");
    for attempt in attempts {
        match attempt() {
            Ok(()) => return Ok(()),
            Err(error) => {
                log::warn!("an open attempt failed: {error}");
                last = error;
            }
        }
    }
    Err(last)
}

#[cfg(all(unix, not(target_os = "macos")))]
fn attempts(path: &Path) -> Vec<Attempt<'_>> {
    use std::process::Command;
    let mut attempts: Vec<Attempt<'_>> = vec![
        // GLib launches the default application itself, in a terminal when
        // its desktop entry asks for one, and exits non-zero when it cannot.
        Box::new(move || linux::launch(Command::new("gio").arg("open").arg(path))),
        // The file manager, with the file selected.
        Box::new(move || linux::show_items(path)),
    ];
    if let Some(folder) = path.parent() {
        attempts.push(Box::new(move || {
            linux::launch(Command::new("xdg-open").arg(folder))
        }));
    }
    attempts
}

/// macOS's `open` and Windows' `start` report a file nothing can open.
#[cfg(not(all(unix, not(target_os = "macos"))))]
fn attempts(path: &Path) -> Vec<Attempt<'_>> {
    let mut attempts: Vec<Attempt<'_>> = vec![Box::new(move || {
        open::that(path).map_err(|error| error.to_string())
    })];
    if let Some(folder) = path.parent() {
        attempts.push(Box::new(move || {
            open::that(folder).map_err(|error| error.to_string())
        }));
    }
    attempts
}

#[cfg(all(unix, not(target_os = "macos")))]
mod linux {
    use std::path::Path;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    /// How long a launcher gets to fail. One still running by then is taken
    /// to have started the program: some launchers run it in the foreground.
    pub(super) const LAUNCH_GRACE: Duration = Duration::from_secs(3);

    /// Starts a launcher and waits up to [`LAUNCH_GRACE`] for its verdict.
    pub(super) fn launch(command: &mut Command) -> Result<(), String> {
        let program = command.get_program().to_string_lossy().into_owned();
        let mut child = command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| format!("{program}: {error}"))?;
        let deadline = Instant::now() + LAUNCH_GRACE;
        loop {
            match child.try_wait() {
                Ok(Some(status)) if status.success() => return Ok(()),
                Ok(Some(status)) => return Err(format!("{program} failed ({status})")),
                Ok(None) if Instant::now() >= deadline => {
                    // Still running: reap it whenever it ends so it leaves no
                    // zombie behind.
                    std::thread::spawn(move || {
                        let _ = child.wait();
                    });
                    return Ok(());
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(50)),
                Err(error) => return Err(format!("{program}: {error}")),
            }
        }
    }

    /// Asks the file manager to show the file selected in its folder.
    pub(super) fn show_items(path: &Path) -> Result<(), String> {
        let uri = file_uri(path);
        let connection =
            zbus::blocking::Connection::session().map_err(|error| error.to_string())?;
        connection
            .call_method(
                Some("org.freedesktop.FileManager1"),
                "/org/freedesktop/FileManager1",
                Some("org.freedesktop.FileManager1"),
                "ShowItems",
                &(vec![uri.as_str()], ""),
            )
            .map(|_| ())
            .map_err(|error| format!("the file manager: {error}"))
    }

    /// A `file://` URI for an absolute path, percent-encoding every byte
    /// outside the characters a path segment may carry as they are.
    pub(super) fn file_uri(path: &Path) -> String {
        use std::fmt::Write;
        let mut uri = String::from("file://");
        for byte in path.as_os_str().as_encoded_bytes() {
            match byte {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                    uri.push(char::from(*byte));
                }
                _ => {
                    let _ = write!(uri, "%{byte:02X}");
                }
            }
        }
        uri
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn the_first_attempt_that_works_ends_the_chain() {
        let tried = Cell::new(0);
        let attempts: Vec<Attempt<'_>> = vec![
            Box::new(|| {
                tried.set(tried.get() + 1);
                Err("no handler".into())
            }),
            Box::new(|| {
                tried.set(tried.get() + 1);
                Ok(())
            }),
            Box::new(|| {
                tried.set(tried.get() + 1);
                Ok(())
            }),
        ];
        assert_eq!(first_success(&attempts), Ok(()));
        assert_eq!(tried.get(), 2, "the fallback ran and nothing after it");
    }

    #[test]
    fn when_nothing_works_the_last_reason_is_reported() {
        let attempts: Vec<Attempt<'_>> = vec![
            Box::new(|| Err("gio failed".into())),
            Box::new(|| Err("no file manager".into())),
        ];
        assert_eq!(first_success(&attempts), Err("no file manager".into()));
        assert!(first_success(&[]).is_err());
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn file_uris_escape_what_a_path_segment_cannot_carry() {
        use linux::file_uri;
        assert_eq!(
            file_uri(Path::new("/home/ada/.local/state/zapfast/zapfast.log")),
            "file:///home/ada/.local/state/zapfast/zapfast.log"
        );
        assert_eq!(
            file_uri(Path::new("/home/José Ω/a#b%c.log")),
            "file:///home/Jos%C3%A9%20%CE%A9/a%23b%25c.log"
        );
    }

    /// A launcher's exit status decides: a failure is reported, a success
    /// or one still running after the grace period counts as opened.
    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn a_launcher_is_judged_by_its_exit() {
        use linux::{LAUNCH_GRACE, launch};
        use std::process::Command;
        use std::time::{Duration, Instant};
        assert_eq!(launch(Command::new("sh").args(["-c", "exit 0"])), Ok(()));
        let failed = launch(Command::new("sh").args(["-c", "exit 4"])).unwrap_err();
        assert!(failed.starts_with("sh failed"), "{failed}");
        let missing = launch(&mut Command::new("zapfast-no-such-launcher")).unwrap_err();
        assert!(
            missing.starts_with("zapfast-no-such-launcher: "),
            "{missing}"
        );
        let started = Instant::now();
        assert_eq!(launch(Command::new("sh").args(["-c", "sleep 30"])), Ok(()));
        assert!(started.elapsed() < LAUNCH_GRACE + Duration::from_secs(2));
    }
}
