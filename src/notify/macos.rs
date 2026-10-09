//! macOS notifications through `UNUserNotificationCenter`, with click handling.
//!
//! The framework only works for a bundled, signed application, and needs the
//! main run loop turning for clicks: the window's event loop does that, and
//! `fastframe_tray::idle` while no window exists. A notification waits for its
//! click the way Linux ones do, until its chat is read or newer notifications
//! release it; dropping the response future stops observing it and leaves it
//! in Notification Center.

use super::{NotificationTarget, Stop};
use mac_usernotifications::Notification;
use std::sync::{Arc, Mutex, OnceLock};

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
    system_sound: bool,
    target: NotificationTarget,
    opened: Arc<Mutex<Vec<NotificationTarget>>>,
    wake: impl Fn() + Send + 'static,
    mut cancelled: tokio::sync::oneshot::Receiver<Stop>,
    may_wait: bool,
) {
    if !authorized() {
        return;
    }
    // macOS always shows the application icon; custom sounds are played by
    // ZapFast, and None stays silent.
    let mut notification = Notification::new().title(title).message(body);
    if system_sound {
        notification = notification.default_sound();
    }
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            log::debug!("no notification runtime: {error}");
            return;
        }
    };
    runtime.block_on(async {
        let handle = match notification.send().await {
            Ok(handle) => handle,
            Err(error) => {
                log::debug!("no notification: {error}");
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
