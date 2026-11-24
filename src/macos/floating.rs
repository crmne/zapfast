//! An optional macOS message presentation. AppKit work stays on the main
//! thread. Authorization is checked asynchronously for each live message.

mod panel;

use std::time::{Duration, Instant};

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject};
use objc2::{MainThreadMarker, msg_send};
use objc2_app_kit::{
    NSAppearance, NSAppearanceCustomization, NSAppearanceNameAqua, NSAppearanceNameDarkAqua,
    NSScreen,
};
use objc2_foundation::{NSBundle, NSNumber, NSPoint, NSRect, NSSize, NSString};
use objc2_user_notifications::{
    UNAlertStyle, UNAuthorizationStatus, UNNotificationSetting, UNNotificationSettings,
    UNUserNotificationCenter,
};
use serde::{Deserialize, Serialize};

use crate::i18n::{Locale, gettext};
use crate::notify::{Delivery, NotificationTarget, Stop};
use crate::settings::NotificationSound;

#[link(name = "Intents", kind = "framework")]
unsafe extern "C" {}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Corner {
    TopLeft,
    #[default]
    TopRight,
    BottomLeft,
    BottomRight,
}

impl Corner {
    pub(crate) const ALL: [Self; 4] = [
        Self::TopLeft,
        Self::TopRight,
        Self::BottomLeft,
        Self::BottomRight,
    ];

    pub(crate) fn label(self, locale: Locale) -> std::borrow::Cow<'static, str> {
        match self {
            Self::TopLeft => gettext(locale, "Top left"),
            Self::TopRight => gettext(locale, "Top right"),
            Self::BottomLeft => gettext(locale, "Bottom left"),
            Self::BottomRight => gettext(locale, "Bottom right"),
        }
    }

    fn top(self) -> bool {
        matches!(self, Self::TopLeft | Self::TopRight)
    }

    fn left(self) -> bool {
        matches!(self, Self::TopLeft | Self::BottomLeft)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub(crate) struct Settings {
    pub(crate) enabled: bool,
    pub(crate) corner: Corner,
    pub(crate) display: Option<String>,
    pub(crate) margin: u16,
    /// Zero keeps the notification until dismissed.
    pub(crate) seconds: u16,
    pub(crate) maximum: u8,
    pub(crate) group: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: false,
            corner: Corner::TopRight,
            display: None,
            margin: 16,
            seconds: 6,
            maximum: 3,
            group: true,
        }
    }
}

impl Settings {
    fn duration(&self) -> Option<Duration> {
        (self.seconds != 0).then(|| Duration::from_secs(self.seconds.clamp(3, 60).into()))
    }
}

fn display_id(screen: &NSScreen) -> Option<String> {
    let id = screen
        .deviceDescription()
        .objectForKey(&NSString::from_str("NSScreenNumber"))?
        .downcast::<NSNumber>()
        .ok()?
        .unsignedIntValue();
    display_uuid(id)
}

fn display_uuid(id: u32) -> Option<String> {
    use std::ffi::{CStr, c_char, c_void};
    #[link(name = "ColorSync", kind = "framework")]
    unsafe extern "C" {
        fn CGDisplayCreateUUIDFromDisplayID(display: u32) -> *const c_void;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFUUIDCreateString(allocator: *const c_void, uuid: *const c_void) -> *const c_void;
        fn CFStringGetCString(
            text: *const c_void,
            buffer: *mut c_char,
            size: isize,
            encoding: u32,
        ) -> u8;
        fn CFRelease(object: *const c_void);
    }
    // SAFETY: both Create calls return owned objects or null. The UUID text
    // is an ASCII UUID and fits in the terminating-NUL buffer.
    unsafe {
        let uuid = CGDisplayCreateUUIDFromDisplayID(id);
        if uuid.is_null() {
            return None;
        }
        let string = CFUUIDCreateString(std::ptr::null(), uuid);
        CFRelease(uuid);
        if string.is_null() {
            return None;
        }
        let mut buffer = [0; 37];
        let ok = CFStringGetCString(string, buffer.as_mut_ptr(), 37, 0x0800_0100);
        CFRelease(string);
        (ok != 0).then(|| {
            CStr::from_ptr(buffer.as_ptr())
                .to_string_lossy()
                .into_owned()
        })
    }
}

pub(crate) fn displays() -> Vec<(String, String)> {
    let Some(main) = MainThreadMarker::new() else {
        return Vec::new();
    };
    NSScreen::screens(main)
        .iter()
        .filter_map(|screen| Some((display_id(&screen)?, screen.localizedName().to_string())))
        .collect()
}

fn screen_frame(main: MainThreadMarker, preferred: Option<&str>) -> Option<NSRect> {
    let screens = NSScreen::screens(main);
    let screen = preferred
        .and_then(|id| {
            screens
                .iter()
                .find(|screen| display_id(screen).as_deref() == Some(id))
        })
        .or_else(|| NSScreen::mainScreen(main))
        .or_else(|| screens.firstObject())?;
    Some(screen.visibleFrame())
}

const GAP: f64 = 10.0;

fn positions(area: NSRect, settings: &Settings) -> Vec<NSRect> {
    let size = NSSize::new(panel::WIDTH, panel::HEIGHT);
    if area.size.width < size.width || area.size.height < size.height {
        return Vec::new();
    }
    let margin = f64::from(settings.margin.min(80))
        .min((area.size.width - size.width) / 2.0)
        .min((area.size.height - size.height) / 2.0);
    let x = if settings.corner.left() {
        area.origin.x + margin
    } else {
        area.origin.x + area.size.width - margin - size.width
    };
    let count = ((area.size.height - margin * 2.0 + GAP) / (size.height + GAP)) as usize;
    (0..count.min(usize::from(settings.maximum.clamp(1, 8))))
        .map(|index| {
            let offset = margin + index as f64 * (size.height + GAP);
            let y = if settings.corner.top() {
                area.origin.y + area.size.height - size.height - offset
            } else {
                area.origin.y + offset
            };
            NSRect::new(NSPoint::new(x, y), size)
        })
        .collect()
}

fn bundle() -> bool {
    let bundle = NSBundle::mainBundle();
    bundle.bundleIdentifier().is_some()
        && bundle
            .objectForInfoDictionaryKey(&NSString::from_str("NSFocusStatusUsageDescription"))
            .is_some()
}

fn focus_center() -> Option<Retained<AnyObject>> {
    let class = AnyClass::get(c"INFocusStatusCenter")?;
    // SAFETY: public Intents class property, available when the class exists.
    unsafe { msg_send![class, defaultCenter] }
}

pub(crate) fn authorize_focus() {
    if !bundle() {
        return;
    }
    if let Some(center) = focus_center() {
        // SAFETY: this public Intents method accepts a nil completion block.
        unsafe {
            let status: isize = msg_send![&*center, authorizationStatus];
            if status == 0 {
                let handler: Option<&block2::DynBlock<dyn Fn(isize)>> = None;
                let _: () = msg_send![&*center, requestAuthorizationWithCompletionHandler: handler];
            }
        }
    }
}

fn focus_clear() -> bool {
    let Some(center) = focus_center() else {
        return false;
    };
    // SAFETY: the public Intents selectors return an NSInteger, FocusStatus,
    // and an optional NSNumber. Only an explicit false is actionable.
    unsafe {
        let status: isize = msg_send![&*center, authorizationStatus];
        if status != 3 {
            return false;
        }
        let focus: Retained<AnyObject> = msg_send![&*center, focusStatus];
        let focused: Option<Retained<NSNumber>> = msg_send![&*focus, isFocused];
        focused.is_some_and(|value| !value.boolValue())
    }
}

struct Pending {
    delivery: Box<Delivery>,
    answer: std::sync::mpsc::Receiver<(Permissions, Option<Vec<u8>>)>,
    until: Instant,
}

#[derive(Clone, Copy)]
struct Permissions {
    banners: bool,
    sounds: bool,
}

struct Card {
    panel: Retained<panel::Panel>,
    delivery: Option<Box<Delivery>>,
    remaining: Option<Duration>,
    count: u32,
}

#[derive(Default)]
pub(crate) struct Floating {
    settings: Settings,
    pending: Vec<Pending>,
    cards: Vec<Card>,
    last_focus_check: Option<(Instant, bool)>,
    preview_appearance: Option<bool>,
    enabled: bool,
    visible_content: bool,
}

impl Floating {
    pub(crate) fn enabled(&self) -> bool {
        self.enabled
    }

    pub(crate) fn configure(&mut self, settings: &Settings, allowed: bool, content: bool) {
        let enabled = allowed && settings.enabled;
        if self.enabled != enabled || self.visible_content != content || !allowed {
            self.clear();
        }
        self.enabled = enabled;
        self.visible_content = content;
        self.settings = settings.clone();
    }

    pub(crate) fn offer(&mut self, delivery: Box<Delivery>) -> Result<(), Box<Delivery>> {
        if !self.enabled || self.pending.len() >= 16 || !bundle() || !focus_clear() {
            return Err(delivery);
        }
        let (send, answer) = std::sync::mpsc::channel();
        let wake = delivery.wake.clone();
        let picture = delivery.notice.picture.clone();
        let reply = RcBlock::new(move |settings: std::ptr::NonNull<UNNotificationSettings>| {
            // SAFETY: UserNotifications passes a live settings object for this call.
            let settings = unsafe { settings.as_ref() };
            let permissions = Permissions {
                banners: settings.authorizationStatus() == UNAuthorizationStatus::Authorized
                    && settings.alertSetting() == UNNotificationSetting::Enabled
                    && settings.alertStyle() != UNAlertStyle::None,
                sounds: settings.soundSetting() == UNNotificationSetting::Enabled,
            };
            let avatar = picture
                .as_deref()
                .filter(|_| permissions.banners)
                .and_then(read_avatar);
            let _ = send.send((permissions, avatar));
            wake();
        });
        UNUserNotificationCenter::currentNotificationCenter()
            .getNotificationSettingsWithCompletionHandler(&reply);
        self.pending.push(Pending {
            delivery,
            answer,
            until: Instant::now() + Duration::from_secs(1),
        });
        Ok(())
    }

    pub(crate) fn tick(
        &mut self,
        ctx: &egui::Context,
        locale: Locale,
        eligible: impl Fn(&NotificationTarget) -> bool,
    ) -> Vec<Delivery> {
        let mut system = Vec::new();
        let Some(main) = MainThreadMarker::new() else {
            return system;
        };
        if self.pending.is_empty() && self.cards.is_empty() {
            return system;
        }
        let now = Instant::now();
        let live_cards = self.cards.iter().any(|card| card.delivery.is_some());
        let mut focus_is_clear = true;
        if live_cards {
            // Refresh while a card is visible so a new Focus can withdraw a
            // persistent card. A repainting window shares that same snapshot.
            let check = Duration::from_secs(1);
            let (checked, clear) = self
                .last_focus_check
                .filter(|(checked, _)| now.saturating_duration_since(*checked) < check)
                .unwrap_or_else(|| (now, focus_clear()));
            self.last_focus_check = Some((checked, clear));
            focus_is_clear = clear;
            ctx.request_repaint_after(check.saturating_sub(now.saturating_duration_since(checked)));
        } else {
            self.last_focus_check = None;
        }
        let frame = screen_frame(main, self.settings.display.as_deref());
        let slots = frame.map_or_else(Vec::new, |frame| positions(frame, &self.settings));
        // Withdraw live cards immediately for Focus, privacy changes, and
        // destroyed screens; previews belong to an explicit Settings action.
        self.cards.retain_mut(|card| {
            if (card.delivery.is_some() && !focus_is_clear)
                || slots.is_empty()
                || card.delivery.as_mut().is_some_and(|delivery| {
                    !eligible(&delivery.target) || withdrawn(&mut delivery.cancelled)
                })
            {
                card.panel.orderOut(None);
                return false;
            }
            let event = card.panel.take_event();
            let (elapsed, hovering) = card.panel.tick_hover(now);
            card.remaining = card
                .remaining
                .map(|remaining| remaining.saturating_sub(elapsed));
            if event == Some(panel::Event::Open)
                && let Some(delivery) = &card.delivery
            {
                delivery.open();
            }
            if event.is_some() || card.remaining == Some(Duration::ZERO) {
                card.panel.dismiss();
                return false;
            }
            if !hovering && let Some(wait) = card.remaining {
                ctx.request_repaint_after(wait);
            }
            true
        });
        for mut pending in std::mem::take(&mut self.pending) {
            if !eligible(&pending.delivery.target) || withdrawn(&mut pending.delivery.cancelled) {
                (pending.delivery.failed)();
                continue;
            }
            match pending.answer.try_recv() {
                Ok((permission, avatar))
                    if permission.banners && !slots.is_empty() && focus_clear() =>
                {
                    let wake = pending.delivery.wake.clone();
                    self.present(
                        main,
                        &slots,
                        locale,
                        Some(pending.delivery),
                        permission.sounds,
                        avatar.as_deref(),
                        wake,
                    );
                }
                Err(std::sync::mpsc::TryRecvError::Empty) if now < pending.until => {
                    ctx.request_repaint_after(pending.until - now);
                    self.pending.push(pending);
                }
                _ => system.push(*pending.delivery),
            }
        }
        self.reflow(&slots);
        system
    }

    pub(crate) fn preview(
        &mut self,
        ctx: &egui::Context,
        locale: Locale,
        dark: bool,
        settings: &Settings,
    ) {
        let Some(main) = MainThreadMarker::new() else {
            return;
        };
        let Some(frame) = screen_frame(main, settings.display.as_deref()) else {
            return;
        };
        self.settings = settings.clone();
        self.preview_appearance = Some(dark);
        let slots = positions(frame, settings);
        if !slots.is_empty() {
            let wake = ctx.clone();
            let picture = self
                .visible_content
                .then_some(include_bytes!("../../assets/demo/avatars/ada.jpg").as_slice());
            self.present(
                main,
                &slots,
                locale,
                None,
                false,
                picture,
                std::sync::Arc::new(move || wake.request_repaint()),
            );
            ctx.request_repaint();
        }
    }

    #[expect(clippy::too_many_arguments)]
    fn present(
        &mut self,
        main: MainThreadMarker,
        slots: &[NSRect],
        locale: Locale,
        delivery: Option<Box<Delivery>>,
        sound: bool,
        avatar: Option<&[u8]>,
        wake: std::sync::Arc<dyn Fn() + Send + Sync>,
    ) {
        let grouped = (self.settings.group && self.visible_content)
            .then(|| {
                self.cards.iter().position(|card| {
                    same_group(
                        card.delivery.as_ref().map(|delivery| &delivery.target),
                        delivery.as_ref().map(|delivery| &delivery.target),
                    )
                })
            })
            .flatten();
        let count = grouped.map_or(1, |index| {
            let old = self.cards.remove(index);
            old.panel.orderOut(None);
            old.count.saturating_add(1)
        });
        let panel = panel::Panel::new(main, slots[0], wake);
        let spec = delivery.as_ref().map(|delivery| &delivery.notice);
        let preview_content = spec.is_none() && self.visible_content;
        let title: std::borrow::Cow<'_, str> = spec.map_or_else(
            || {
                if preview_content {
                    "Ada Lovelace".into()
                } else {
                    "ZapFast".into()
                }
            },
            |notice| notice.title.as_str().into(),
        );
        let body = spec.map_or_else(
            || {
                if preview_content {
                    gettext(locale, "I saved you a seat by the window.")
                } else {
                    gettext(locale, "New message")
                }
            },
            |notice| notice.body.as_str().into(),
        );
        if spec.is_none()
            && let Some(dark) = self.preview_appearance
        {
            // SAFETY: both AppKit appearance names are available on macOS 11+.
            let name = unsafe {
                if dark {
                    NSAppearanceNameDarkAqua
                } else {
                    NSAppearanceNameAqua
                }
            };
            panel.setAppearance(NSAppearance::appearanceNamed(name).as_deref());
        }
        let count_label = (count > 1 && self.visible_content).then(|| {
            crate::i18n::ngettext(locale, "{} message", "{} messages", count)
                .replace("{}", &count.to_string())
        });
        panel.populate(
            main,
            &title,
            &body,
            count_label.as_deref(),
            avatar,
            spec.is_some(),
            &gettext(locale, "Dismiss notification"),
        );
        let remaining = self.settings.duration();
        if let Some(notice) = spec.filter(|_| sound) {
            match &notice.sound {
                NotificationSound::System => objc2_app_kit::NSBeep(),
                sound => crate::notify::play_sound(sound.clone()),
            }
        }
        self.cards.insert(
            0,
            Card {
                panel,
                delivery,
                remaining,
                count,
            },
        );
        self.reflow(slots);
        self.cards[0].panel.appear(self.settings.corner.left());
    }

    fn reflow(&mut self, slots: &[NSRect]) {
        for card in self.cards.drain(slots.len().min(self.cards.len())..) {
            card.panel.orderOut(None);
        }
        for (card, frame) in self.cards.iter().zip(slots) {
            card.panel.move_to(*frame);
        }
    }

    pub(crate) fn clear(&mut self) {
        for card in self.cards.drain(..) {
            card.panel.orderOut(None);
        }
        for pending in self.pending.drain(..) {
            (pending.delivery.failed)();
        }
    }
}

impl Drop for Floating {
    fn drop(&mut self) {
        self.clear();
    }
}

fn withdrawn(cancelled: &mut tokio::sync::oneshot::Receiver<Stop>) -> bool {
    use tokio::sync::oneshot::error::TryRecvError;
    matches!(
        cancelled.try_recv(),
        Ok(Stop::Close | Stop::Release) | Err(TryRecvError::Closed)
    )
}

fn same_group(a: Option<&NotificationTarget>, b: Option<&NotificationTarget>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => a.account == b.account && a.chat == b.chat,
        (None, None) => true,
        _ => false,
    }
}

fn read_avatar(path: &std::path::Path) -> Option<Vec<u8>> {
    use std::io::Read;
    const LIMIT: u64 = 2 * 1024 * 1024;
    let mut file = std::fs::File::open(path).ok()?.take(LIMIT + 1);
    let mut data = Vec::new();
    file.read_to_end(&mut data).ok()?;
    (data.len() as u64 <= LIMIT).then_some(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_settings_default_to_system_and_floating_choices_round_trip() {
        let mut settings: crate::settings::Settings =
            serde_json::from_str(r#"{"notification_content":false}"#).unwrap();
        assert!(!settings.notification_content);
        assert_eq!(settings.floating_notifications, Settings::default());
        settings.floating_notifications = Settings {
            enabled: true,
            corner: Corner::BottomLeft,
            display: Some("display-uuid".into()),
            margin: 24,
            seconds: 0,
            maximum: 5,
            group: false,
        };
        let restored: crate::settings::Settings =
            serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
        assert_eq!(
            restored.floating_notifications,
            settings.floating_notifications
        );
    }

    #[test]
    fn grouping_preserves_account_boundaries_and_does_not_mix_previews() {
        let target = NotificationTarget {
            account: crate::model::AccountId::first(),
            chat: "first chat".into(),
            message: Some("1".into()),
        };
        let newer = NotificationTarget {
            message: Some("2".into()),
            ..target.clone()
        };
        assert!(same_group(Some(&target), Some(&newer)));
        let other_account = NotificationTarget {
            account: crate::model::AccountId("second".into()),
            ..newer.clone()
        };
        let other_chat = NotificationTarget {
            chat: "second chat".into(),
            ..newer
        };
        assert!(!same_group(Some(&target), Some(&other_account)));
        assert!(!same_group(Some(&target), Some(&other_chat)));
        assert!(!same_group(Some(&target), None));
    }

    #[test]
    fn corners_stack_inward_on_a_secondary_display() {
        let area = NSRect::new(NSPoint::new(-1800.0, -80.0), NSSize::new(1800.0, 990.0));
        for corner in Corner::ALL {
            let settings = Settings {
                corner,
                ..Default::default()
            };
            let positions = positions(area, &settings);
            assert_eq!(positions.len(), 3);
            let [first, second, ..] = positions.as_slice() else {
                unreachable!()
            };
            assert_eq!(
                first.origin.x,
                if corner.left() {
                    -1784.0
                } else {
                    -16.0 - panel::WIDTH
                }
            );
            assert_eq!(
                first.origin.y,
                if corner.top() {
                    894.0 - panel::HEIGHT
                } else {
                    -64.0
                }
            );
            assert_eq!(
                (first.origin.y - second.origin.y).abs(),
                panel::HEIGHT + GAP
            );
        }
    }

    #[test]
    fn extreme_settings_stay_within_the_work_area() {
        let area = NSRect::new(
            NSPoint::ZERO,
            NSSize::new(panel::WIDTH + 12.0, panel::HEIGHT + 2.0),
        );
        let settings = Settings {
            margin: u16::MAX,
            maximum: u8::MAX,
            seconds: u16::MAX,
            ..Default::default()
        };
        let positions = positions(area, &settings);
        assert_eq!(positions.len(), 1);
        assert!(positions[0].origin.y >= 0.0);
        assert_eq!(settings.duration(), Some(Duration::from_secs(60)));
        assert_eq!(
            Settings {
                seconds: 0,
                ..settings
            }
            .duration(),
            None
        );
    }
}
