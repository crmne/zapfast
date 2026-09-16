//! Desktop notifications when the app is hidden, unfocused, or on another chat.
//!
//! Delivery uses the platform notification service. Each notification runs on
//! its own thread because delivery and click handling can block.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// Cancellation is registered before delivery starts, so reading a chat while
/// its notification is still being delivered cannot leave a stale notification.
#[derive(Default)]
pub struct Notifications {
    pending: std::collections::HashMap<String, Vec<tokio::sync::oneshot::Sender<()>>>,
}

impl Notifications {
    fn register(&mut self, chat: &str) -> tokio::sync::oneshot::Receiver<()> {
        self.pending.retain(|_, entries| {
            entries.retain(|entry| !entry.is_closed());
            !entries.is_empty()
        });
        let (cancel, cancelled) = tokio::sync::oneshot::channel();
        self.pending
            .entry(chat.to_owned())
            .or_default()
            .push(cancel);
        cancelled
    }

    pub fn clear(&mut self, chat: &str) {
        if let Some(entries) = self.pending.remove(chat) {
            for cancel in entries {
                let _ = cancel.send(());
            }
        }
    }

    pub fn clear_all(&mut self) {
        self.pending.clear();
    }

    /// Shows a notification; platform delivery runs outside the interface thread.
    pub fn show(
        &mut self,
        title: String,
        body: String,
        picture: Option<PathBuf>,
        chat: String,
        opened: Arc<Mutex<Vec<String>>>,
        wake: impl Fn() + Send + 'static,
    ) {
        let cancelled = self.register(&chat);
        let spawned = std::thread::Builder::new()
            .name("notification".into())
            .spawn(move || {
                deliver(
                    &title,
                    &body,
                    picture.as_deref(),
                    chat,
                    opened,
                    wake,
                    cancelled,
                )
            });
        if let Err(error) = spawned {
            log::debug!("no thread for a notification: {error}");
        }
    }
}

/// Builds the notification title and body, including the group sender.
pub fn lines(chat_name: &str, is_group: bool, sender: &str, summary: &str) -> (String, String) {
    let body = if is_group {
        format!("{sender}: {summary}")
    } else {
        summary.to_owned()
    };
    (chat_name.to_owned(), body)
}

#[cfg(target_os = "linux")]
fn deliver(
    title: &str,
    body: &str,
    picture: Option<&std::path::Path>,
    chat: String,
    opened: Arc<Mutex<Vec<String>>>,
    wake: impl Fn() + Send + 'static,
    mut cancelled: tokio::sync::oneshot::Receiver<()>,
) {
    if !matches!(
        cancelled.try_recv(),
        Err(tokio::sync::oneshot::error::TryRecvError::Empty)
    ) {
        return;
    }
    let mut notification = notify_rust::Notification::new();
    notification
        .appname(&crate::profile::running().label())
        .summary(title)
        .body(body)
        .icon("zapfast")
        .action("default", "Open");
    if let Some(picture) = picture {
        notification.image_path(&picture.to_string_lossy());
    }
    match notification.show() {
        Ok(handle) => {
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(error) => {
                    handle.close();
                    log::debug!("no notification action runtime: {error}");
                    return;
                }
            };
            runtime.block_on(async {
                tokio::select! {
                    biased;
                    _ = &mut cancelled => handle.close_async().await,
                    _ = handle.wait_for_action_async(|action| {
                        if matches!(action, notify_rust::NotificationResponse::Default) {
                            opened.lock().unwrap_or_else(|p| p.into_inner()).push(chat);
                            wake();
                        }
                    }) => {}
                }
            });
        }
        Err(error) => log::debug!("no notification: {error}"),
    }
}

#[cfg(not(target_os = "linux"))]
fn deliver(
    title: &str,
    body: &str,
    picture: Option<&std::path::Path>,
    _chat: String,
    _opened: Arc<Mutex<Vec<String>>>,
    _wake: impl Fn() + Send + 'static,
    mut cancelled: tokio::sync::oneshot::Receiver<()>,
) {
    if !matches!(
        cancelled.try_recv(),
        Err(tokio::sync::oneshot::error::TryRecvError::Empty)
    ) {
        return;
    }
    let mut notification = notify_rust::Notification::new();
    notification
        .appname(&crate::profile::running().label())
        .summary(title)
        .body(body);
    // Windows uses the image; macOS always uses the app icon.
    if let Some(picture) = picture {
        notification.image_path(&picture.to_string_lossy());
    }
    if let Err(error) = notification.show() {
        log::debug!("no notification: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reading_cancels_delivered_and_pending_notifications_for_only_that_chat() {
        let mut notifications = Notifications::default();
        let mut first = notifications.register("a");
        let mut second = notifications.register("a");
        let mut other = notifications.register("b");
        notifications.clear("a");
        assert_eq!(first.try_recv(), Ok(()));
        assert_eq!(second.try_recv(), Ok(()));
        assert_eq!(
            other.try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Empty)
        );
        let mut next = notifications.register("a");
        assert_eq!(
            next.try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Empty)
        );
        notifications.clear_all();
        assert!(other.try_recv().is_err());
        assert_eq!(
            next.try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Closed)
        );
    }

    #[test]
    fn expired_notifications_do_not_accumulate() {
        let mut notifications = Notifications::default();
        drop(notifications.register("a"));
        let _next = notifications.register("b");
        assert!(!notifications.pending.contains_key("a"));
    }

    /// Shows a test notification with an optional cached picture:
    /// `cargo test --all-features shows_one -- --ignored --nocapture`.
    #[test]
    #[ignore = "shows a real notification"]
    fn shows_one_on_this_desktop() {
        let picture = std::fs::read_dir(
            crate::paths::AppDirs::discover(Default::default()).avatar_cache_dir(),
        )
        .ok()
        .and_then(|entries| entries.flatten().map(|entry| entry.path()).next());
        let mut notifications = Notifications::default();
        notifications.show(
            "Ada Lovelace".into(),
            "A test from ZapFast, with a picture".into(),
            picture,
            "test".into(),
            Default::default(),
            || {},
        );
        std::thread::sleep(std::time::Duration::from_secs(2));
    }

    #[test]
    fn a_group_names_the_sender_and_a_chat_does_not() {
        assert_eq!(
            lines("Rust Berlin", true, "Mira", "Save me a seat"),
            ("Rust Berlin".to_owned(), "Mira: Save me a seat".to_owned())
        );
        assert_eq!(
            lines("Ada Lovelace", false, "Ada Lovelace", "Photo"),
            ("Ada Lovelace".to_owned(), "Photo".to_owned())
        );
    }
}
