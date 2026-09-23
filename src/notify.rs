//! Desktop notifications when the app is hidden, unfocused, or on another chat.
//!
//! Delivery uses the platform notification service. Each notification runs on
//! its own thread because delivery and click handling can block.

use crate::settings::NotificationSound;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

#[cfg(target_os = "linux")]
mod badge;
#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "linux")]
pub use badge::Badge;

/// A no-op taskbar badge where the desktop has no Unity Launcher API.
#[cfg(not(target_os = "linux"))]
#[derive(Default)]
pub struct Badge;

#[cfg(not(target_os = "linux"))]
impl Badge {
    /// Does nothing; no desktop here reads a taskbar badge.
    pub fn set(&mut self, _count: u32) {}
}

#[cfg(any(target_os = "macos", test))]
const MACOS_APPLICATION_ID: &str = "me.paolino.fastsapp";

#[cfg(target_os = "macos")]
fn macos_application_ready() -> bool {
    static READY: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *READY.get_or_init(|| {
        // The library's implicit default looks up an app named "use_default"
        // through AppleScript, which opens macOS's application chooser.
        match notify_rust::set_application(MACOS_APPLICATION_ID) {
            Ok(()) => true,
            Err(error) => {
                log::debug!("could not initialize notification application: {error}");
                false
            }
        }
    })
}

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
    #[expect(clippy::too_many_arguments)]
    pub fn show(
        &mut self,
        title: String,
        body: String,
        picture: Option<PathBuf>,
        sound: NotificationSound,
        chat: String,
        opened: Arc<Mutex<Vec<String>>>,
        wake: impl Fn() + Send + 'static,
    ) {
        let cancelled = self.register(&chat);
        let spawned = std::thread::Builder::new()
            .name("notification".into())
            .spawn(move || {
                let system_sound = sound == NotificationSound::System;
                play_sound(sound);
                deliver(
                    &title,
                    &body,
                    picture.as_deref(),
                    system_sound,
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

/// ZapFast's own sounds, synthesized by `assets/sounds/generate.py`.
const CHIME: &[u8] = include_bytes!("../assets/sounds/chime.ogg");
const RIPPLE: &[u8] = include_bytes!("../assets/sounds/ripple.ogg");

/// Plays a notification sound on its own thread, for notifications and
/// their preview in Settings. System sounds and silence play nothing here.
pub fn play_sound(sound: NotificationSound) {
    let source: Box<dyn Fn() -> std::io::Result<Box<dyn ReadSeek>> + Send> = match sound {
        NotificationSound::Chime => Box::new(|| Ok(Box::new(std::io::Cursor::new(CHIME)))),
        NotificationSound::Ripple => Box::new(|| Ok(Box::new(std::io::Cursor::new(RIPPLE)))),
        NotificationSound::Custom(path) => Box::new(move || {
            Ok(Box::new(std::io::BufReader::new(std::fs::File::open(
                &path,
            )?)))
        }),
        NotificationSound::System | NotificationSound::None => return,
    };
    let spawned = std::thread::Builder::new()
        .name("notification-sound".into())
        .spawn(move || {
            let played = (|| -> Result<(), String> {
                let reader = source().map_err(|error| error.to_string())?;
                let decoder = rodio::Decoder::new(reader).map_err(|error| error.to_string())?;
                let device = rodio::DeviceSinkBuilder::open_default_sink()
                    .map_err(|error| error.to_string())?;
                let player = rodio::Player::connect_new(device.mixer());
                player.append(decoder);
                player.sleep_until_end();
                Ok(())
            })();
            if let Err(error) = played {
                log::debug!("notification sound not played: {error}");
            }
        });
    if let Err(error) = spawned {
        log::debug!("no thread for a notification sound: {error}");
    }
}

trait ReadSeek: std::io::Read + std::io::Seek + Send + Sync {}
impl<T: std::io::Read + std::io::Seek + Send + Sync> ReadSeek for T {}

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
#[expect(clippy::too_many_arguments)]
fn deliver(
    title: &str,
    body: &str,
    picture: Option<&std::path::Path>,
    system_sound: bool,
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
        .appname("ZapFast")
        .summary(title)
        .body(body)
        .icon("zapfast")
        .action("default", "Open");
    if !system_sound {
        notification.hint(notify_rust::Hint::SuppressSound(true));
    }
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

#[cfg(target_os = "windows")]
#[expect(clippy::too_many_arguments)]
fn deliver(
    title: &str,
    body: &str,
    picture: Option<&std::path::Path>,
    system_sound: bool,
    _chat: String,
    _opened: Arc<Mutex<Vec<String>>>,
    _wake: impl Fn() + Send + 'static,
    mut cancelled: tokio::sync::oneshot::Receiver<()>,
) {
    if matches!(
        cancelled.try_recv(),
        Err(tokio::sync::oneshot::error::TryRecvError::Empty)
    ) && let Err(error) = windows::show(title, body, picture, system_sound)
    {
        log::debug!("no Windows notification: {error}");
    }
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
#[expect(clippy::too_many_arguments)]
fn deliver(
    title: &str,
    body: &str,
    picture: Option<&std::path::Path>,
    system_sound: bool,
    _chat: String,
    _opened: Arc<Mutex<Vec<String>>>,
    _wake: impl Fn() + Send + 'static,
    mut cancelled: tokio::sync::oneshot::Receiver<()>,
) {
    // Never fall back to application discovery, including for unbundled builds.
    #[cfg(target_os = "macos")]
    if !macos_application_ready() {
        return;
    }
    if !matches!(
        cancelled.try_recv(),
        Err(tokio::sync::oneshot::error::TryRecvError::Empty)
    ) {
        return;
    }
    let mut notification = notify_rust::Notification::new();
    notification.appname("ZapFast").summary(title).body(body);
    #[cfg(target_os = "macos")]
    if system_sound {
        // The notification system's default sound; custom sounds are played
        // by ZapFast, and None stays silent.
        notification.sound_name("NSUserNotificationDefaultSoundName");
    }
    #[cfg(not(target_os = "macos"))]
    let _ = system_sound;
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
    fn macos_notification_identity_matches_the_packaged_application() {
        let plist = include_str!("../packaging/macos/Info.plist");
        assert!(plist.contains(&format!(
            "<key>CFBundleIdentifier</key><string>{MACOS_APPLICATION_ID}</string>"
        )));
    }

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
        let picture = std::fs::read_dir(crate::paths::AppDirs::discover().avatar_cache_dir())
            .ok()
            .and_then(|entries| entries.flatten().map(|entry| entry.path()).next());
        let mut notifications = Notifications::default();
        notifications.show(
            "Ada Lovelace".into(),
            "A test from ZapFast, with a picture".into(),
            picture,
            NotificationSound::System,
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
