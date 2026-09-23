//! The Linux badge through the Unity Launcher API.
//!
//! Launchers that implement the API listen for an `Update` signal on
//! `com.canonical.Unity.LauncherEntry` and draw the unread count on the icon of
//! the matching desktop file. KDE Plasma's task manager is the one this targets;
//! GNOME's Dash to Dock and Dash to Panel, and Plank, read it too. The signal
//! carries the desktop file as `application://<name>.desktop` and a
//! `count`/`count-visible` property pair. Nothing is called and no name is
//! claimed, so emitting it is the whole implementation; other launchers ignore
//! it.
//!
//! A worker thread owns the session-bus connection, so a slow or missing bus
//! never delays a frame. The connection stays open while the app runs; some
//! launchers clear the badge when its sender disconnects.

use std::collections::HashMap;
use std::sync::mpsc;

use zbus::zvariant::Value;

/// KDE shows at most four digits.
const MAX_COUNT: u32 = 9999;

/// The Unity Launcher URI for a desktop file name.
fn launcher_uri(desktop_file: &str) -> String {
    format!("application://{desktop_file}.desktop")
}

/// The desktop file name for this installation, which is also the app id the
/// window uses (see `main.rs`). A Flatpak install ships
/// `rocks.zapfast.ZapFast.desktop` and sets the same id in `FLATPAK_ID`.
fn desktop_file() -> String {
    std::env::var("FLATPAK_ID").unwrap_or_else(|_| "zapfast".to_owned())
}

/// Counts unread messages on the taskbar icon through the Unity Launcher API.
#[derive(Default)]
pub struct Badge {
    shown: Option<u32>,
    worker: Option<mpsc::Sender<u32>>,
}

impl Badge {
    /// Publishes the count, clearing the badge at zero. A repeat is not sent.
    pub fn set(&mut self, count: u32) {
        let Some(count) = self.pending(count) else {
            return;
        };
        // A worker that could not reach the bus has exited; sending then fails.
        let _ = self.worker.get_or_insert_with(spawn).send(count);
    }

    /// The count to publish, or `None` when it matches the last one.
    fn pending(&mut self, count: u32) -> Option<u32> {
        let count = count.min(MAX_COUNT);
        if self.shown == Some(count) {
            return None;
        }
        self.shown = Some(count);
        Some(count)
    }
}

/// Starts the thread that connects to the session bus and emits each count.
fn spawn() -> mpsc::Sender<u32> {
    let (sender, counts) = mpsc::channel::<u32>();
    let spawned = std::thread::Builder::new()
        .name("taskbar-badge".into())
        .spawn(move || {
            let connection = match zbus::blocking::Connection::session() {
                Ok(connection) => connection,
                Err(error) => {
                    log::debug!("no session bus for the taskbar badge: {error}");
                    return;
                }
            };
            let uri = launcher_uri(&desktop_file());
            while let Ok(mut count) = counts.recv() {
                // Only the latest of several queued counts matters.
                while let Ok(next) = counts.try_recv() {
                    count = next;
                }
                emit(&connection, &uri, count);
            }
        });
    if let Err(error) = spawned {
        log::debug!("could not start the taskbar badge thread: {error}");
    }
    sender
}

fn emit(connection: &zbus::blocking::Connection, uri: &str, count: u32) {
    let mut properties: HashMap<&str, Value> = HashMap::new();
    if count > 0 {
        properties.insert("count", Value::from(i64::from(count)));
        properties.insert("count-visible", Value::from(true));
    } else {
        // Zero clears the badge; the count itself is not meaningful then.
        properties.insert("count-visible", Value::from(false));
    }
    if let Err(error) = connection.emit_signal(
        None::<&str>,
        "/com/canonical/unity/launcherentry/zapfast",
        "com.canonical.Unity.LauncherEntry",
        "Update",
        &(uri, properties),
    ) {
        log::debug!("taskbar badge not updated: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launcher_uri_names_the_desktop_file() {
        assert_eq!(launcher_uri("zapfast"), "application://zapfast.desktop");
        assert_eq!(
            launcher_uri("rocks.zapfast.ZapFast"),
            "application://rocks.zapfast.ZapFast.desktop"
        );
    }

    #[test]
    fn counts_clamp_and_repeats_do_not_publish() {
        let mut badge = Badge::default();
        assert_eq!(badge.pending(3), Some(3));
        assert_eq!(badge.pending(3), None);
        assert_eq!(badge.pending(0), Some(0));
        assert_eq!(badge.pending(0), None);
        assert_eq!(badge.pending(MAX_COUNT + 10), Some(MAX_COUNT));
        assert_eq!(badge.pending(MAX_COUNT + 10), None);
    }
}
