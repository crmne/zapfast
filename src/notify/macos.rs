//! macOS notifications through `UNUserNotificationCenter`, with click handling.
//!
//! The framework only works for a bundled, signed application, and needs the
//! main run loop turning for clicks: the window's event loop does that, and
//! `fastframe_tray::idle` while no window exists. A notification waits for its
//! click the way Linux ones do, until its chat is read or newer notifications
//! release it; dropping the response future stops observing it and leaves it
//! in Notification Center.

use super::{NotificationTarget, Stop};
use crate::settings::NotificationSound;
use mac_usernotifications::Notification;
use std::sync::{Arc, Mutex, OnceLock};

/// The file name `packaging/macos/bundle.sh` gives a built-in sound in the
/// bundle's `Contents/Resources`, where the notification looks it up by name.
/// The prefix keeps a same-named file in `~/Library/Sounds`, which macOS
/// searches first, from replacing it.
fn sound_file(sound: &NotificationSound) -> Option<&'static str> {
    match sound {
        NotificationSound::Receive => Some("zapfast-receive.wav"),
        NotificationSound::Alert => Some("zapfast-alert.wav"),
        NotificationSound::System | NotificationSound::None | NotificationSound::Custom(_) => None,
    }
}

/// The name of a built-in sound the notification can play from the
/// application bundle, so Do Not Disturb and Focus silence it with the
/// banner. `None` for the other sounds, and for a run outside a bundle that
/// carries the file (`cargo run`), where ZapFast plays the sound itself. An
/// audio file of the reader's own is not in the bundle and is played by
/// ZapFast.
pub(super) fn bundled_sound(sound: &NotificationSound) -> Option<&'static str> {
    let name = sound_file(sound)?;
    let executable = std::env::current_exe().ok()?;
    let contents = executable.parent()?.parent()?;
    contents
        .join("Resources")
        .join(name)
        .is_file()
        .then_some(name)
}

/// Whether notifications can be shown: the process has a bundle identifier
/// (the framework aborts without one, so unbundled builds stay silent) and the
/// reader allows them. macOS asks only once per application and afterwards
/// answers with the current setting, so a permission granted later in System
/// Settings applies without a restart.
fn authorized() -> bool {
    static BUNDLED: OnceLock<bool> = OnceLock::new();
    let bundled = *BUNDLED.get_or_init(|| match mac_usernotifications::check_bundle() {
        Ok(()) => true,
        Err(error) => {
            log::debug!("no notifications outside the application bundle: {error}");
            false
        }
    });
    if !bundled {
        return false;
    }
    match mac_usernotifications::blocking::request_auth() {
        Ok(granted) => {
            if !granted {
                log::debug!("notifications are not allowed in System Settings");
            }
            granted
        }
        Err(error) => {
            log::debug!("could not ask for notification permission: {error}");
            false
        }
    }
}

#[expect(clippy::too_many_arguments)]
pub(super) fn deliver(
    title: &str,
    body: &str,
    sound: &NotificationSound,
    target: NotificationTarget,
    opened: Arc<Mutex<Vec<NotificationTarget>>>,
    wake: impl Fn() + Send + 'static,
    mut cancelled: tokio::sync::oneshot::Receiver<Stop>,
    may_wait: bool,
    failed: impl FnOnce(),
) {
    if !authorized() {
        failed();
        return;
    }
    // macOS always shows the application icon. Built-in sounds play from the
    // bundle with the notification, an audio file of the reader's own is
    // played by ZapFast, and None stays silent.
    let mut notification = Notification::new().title(title).message(body);
    if *sound == NotificationSound::System {
        notification = notification.default_sound();
    } else if let Some(name) = bundled_sound(sound) {
        notification = notification.sound(name);
    }
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            log::debug!("no notification runtime: {error}");
            failed();
            return;
        }
    };
    runtime.block_on(async {
        let handle = match notification.send().await {
            Ok(handle) => handle,
            Err(error) => {
                log::debug!("no notification: {error}");
                failed();
                return;
            }
        };
        if !may_wait {
            // Shown, and left to Notification Center.
            return;
        }
        let id = handle.notification_id().to_owned();
        tokio::select! {
            biased;
            stop = &mut cancelled => {
                if stop != Ok(Stop::Release) {
                    mac_usernotifications::close_delivered(&id).await;
                }
            }
            response = handle.response() => {
                if response.is_ok_and(|response| response.is_default_action()) {
                    opened.lock().unwrap_or_else(|p| p.into_inner()).push(target);
                    wake();
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bundle script copies each built-in sound under the name the
    /// notification asks for.
    #[test]
    fn the_bundle_script_ships_every_built_in_sound() {
        let script = include_str!("../../packaging/macos/bundle.sh");
        for sound in [NotificationSound::Receive, NotificationSound::Alert] {
            let name = sound_file(&sound).unwrap();
            assert!(script.contains(name), "bundle.sh does not ship {name}");
        }
        assert_eq!(sound_file(&NotificationSound::System), None);
        assert_eq!(
            sound_file(&NotificationSound::Custom("/a.wav".into())),
            None
        );
    }
}
