//! The text cursor's blink, as the system sets it.
//!
//! egui blinks every text cursor half a second on and half a second off.
//! Each platform lets people change that rate or stop the blink, often for
//! comfort or accessibility, and native text fields follow the choice. So
//! ZapFast reads it once, the first time the theme is applied:
//!
//! - Windows: `GetCaretBlinkTime`, how long each phase lasts; `INFINITE`
//!   means the cursor does not blink.
//! - macOS: the `NSTextInsertionPointBlinkPeriodOn` and `…Off` defaults,
//!   which AppKit's text views read, in milliseconds (or the older
//!   `NSTextInsertionPointBlinkPeriod`, a whole cycle). A huge period, the
//!   usual way to stop the blink there, counts as steady.
//! - Linux: GNOME's `cursor-blink` and `cursor-blink-time` (a whole cycle),
//!   else KDE's `CursorBlinkRate` (a whole cycle, 0 for none), read through
//!   the desktop portal, which works inside a Flatpak too.
//!
//! Where the platform says nothing, egui's own rate stays. The pause after
//! a few idle seconds ([`crate::motion::idle_text_cursor`]) applies either
//! way.

use std::sync::OnceLock;

/// How the system blinks a text cursor.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Blink {
    /// The cursor stays visible.
    Steady,
    /// The cursor shows for `on` seconds, then hides for `off` seconds.
    Period { on: f32, off: f32 },
}

/// A phase this long (in milliseconds) no longer reads as blinking.
const STEADY_PHASE_MS: f64 = 60_000.0;

impl Blink {
    /// From each phase's length in milliseconds; `None` for nonsense.
    fn phases(on_ms: f64, off_ms: f64) -> Option<Self> {
        if !(on_ms.is_finite() && off_ms.is_finite()) || on_ms <= 0.0 || off_ms < 0.0 {
            return None;
        }
        if off_ms == 0.0 || on_ms >= STEADY_PHASE_MS || off_ms >= STEADY_PHASE_MS {
            return Some(Self::Steady);
        }
        Some(Self::Period {
            on: (on_ms / 1000.0) as f32,
            off: (off_ms / 1000.0) as f32,
        })
    }

    /// From a whole cycle in milliseconds, shown for half and hidden for half.
    #[cfg(any(target_os = "linux", target_os = "macos", test))]
    fn cycle(ms: f64) -> Option<Self> {
        Self::phases(ms / 2.0, ms / 2.0)
    }
}

/// Sets `cursor` to blink as the system does, or leaves egui's rate when the
/// system says nothing.
pub fn follow_system(cursor: &mut egui::style::TextCursorStyle) {
    apply(cursor, system());
}

fn apply(cursor: &mut egui::style::TextCursorStyle, blink: Option<Blink>) {
    match blink {
        None => {}
        Some(Blink::Steady) => cursor.blink = false,
        Some(Blink::Period { on, off }) => {
            cursor.blink = true;
            cursor.on_duration = on;
            cursor.off_duration = off;
        }
    }
}

/// The system's blink, read once. Tests keep egui's, so the machine running
/// them does not decide it.
pub fn system() -> Option<Blink> {
    static SYSTEM: OnceLock<Option<Blink>> = OnceLock::new();
    *SYSTEM.get_or_init(|| {
        if cfg!(test) {
            return None;
        }
        let blink = read();
        log::debug!("the system's text cursor blink: {blink:?}");
        blink
    })
}

#[cfg(windows)]
fn read() -> Option<Blink> {
    // SAFETY: takes no arguments and only reads a user setting.
    windows_caret(unsafe { windows::Win32::UI::WindowsAndMessaging::GetCaretBlinkTime() })
}

/// `GetCaretBlinkTime`'s answer: each phase's length, `INFINITE` for no
/// blink, or 0 when the call failed.
#[cfg(any(windows, test))]
fn windows_caret(ms: u32) -> Option<Blink> {
    match ms {
        0 => None,
        u32::MAX => Some(Blink::Steady),
        ms => Blink::phases(f64::from(ms), f64::from(ms)),
    }
}

#[cfg(target_os = "macos")]
fn read() -> Option<Blink> {
    use objc2_foundation::{NSString, NSUserDefaults};
    let defaults = NSUserDefaults::standardUserDefaults();
    let number = |key: &str| {
        let key = NSString::from_str(key);
        defaults
            .objectForKey(&key)
            .is_some()
            .then(|| defaults.doubleForKey(&key))
    };
    macos_insertion_point(
        number("NSTextInsertionPointBlinkPeriodOn"),
        number("NSTextInsertionPointBlinkPeriodOff"),
        number("NSTextInsertionPointBlinkPeriod"),
    )
}

/// AppKit's defaults, in milliseconds: each phase, or the older whole cycle.
/// A phase set alone stands for both.
#[cfg(any(target_os = "macos", test))]
fn macos_insertion_point(on: Option<f64>, off: Option<f64>, cycle: Option<f64>) -> Option<Blink> {
    match (on, off) {
        (None, None) => Blink::cycle(cycle?),
        (on, off) => Blink::phases(on.or(off)?, off.or(on)?),
    }
}

#[cfg(target_os = "linux")]
fn read() -> Option<Blink> {
    portal::read()
}

/// GNOME's `cursor-blink` and `cursor-blink-time` (a whole cycle in ms).
#[cfg(any(target_os = "linux", test))]
fn gnome(blink: Option<bool>, cycle_ms: Option<i64>) -> Option<Blink> {
    if blink == Some(false) {
        return Some(Blink::Steady);
    }
    Blink::cycle(cycle_ms? as f64)
}

/// KDE's `CursorBlinkRate`: a whole cycle in ms, 0 for no blink.
#[cfg(any(target_os = "linux", test))]
fn kde(cycle_ms: Option<i64>) -> Option<Blink> {
    match cycle_ms? {
        0 => Some(Blink::Steady),
        ms => Blink::cycle(ms as f64),
    }
}

#[cfg(target_os = "linux")]
mod portal {
    use std::time::Duration;

    use zbus::blocking::{Proxy, connection};
    use zbus::zvariant::{OwnedValue, Value};

    const GNOME: &str = "org.gnome.desktop.interface";
    const KDE: &str = "org.kde.kdeglobals.KDE";

    pub(super) fn read() -> Option<super::Blink> {
        // Half a second per call, so a portal that is still starting cannot
        // hold the first frame up for long.
        let connection = connection::Builder::session()
            .ok()?
            .method_timeout(Duration::from_millis(500))
            .build()
            .ok()?;
        let proxy = Proxy::new(
            &connection,
            "org.freedesktop.portal.Desktop",
            "/org/freedesktop/portal/desktop",
            "org.freedesktop.portal.Settings",
        )
        .ok()?;
        let blink = setting(&proxy, GNOME, "cursor-blink")?;
        let cycle = setting(&proxy, GNOME, "cursor-blink-time")?;
        if blink.is_some() || cycle.is_some() {
            return super::gnome(
                blink.as_deref().and_then(boolean),
                cycle.as_deref().and_then(integer),
            );
        }
        let rate = setting(&proxy, KDE, "CursorBlinkRate")?;
        super::kde(rate.as_deref().and_then(integer))
    }

    /// One setting: `Some(None)` when the portal does not have it, `None`
    /// when the portal does not answer, so the rest are not asked.
    fn setting(proxy: &Proxy<'_>, namespace: &str, key: &str) -> Option<Option<OwnedValue>> {
        // `ReadOne` arrived in version 2 of the interface; `Read` wraps the
        // same answer in one more variant.
        let answer = proxy
            .call("ReadOne", &(namespace, key))
            .or_else(|error| match error {
                zbus::Error::MethodError(ref name, ..)
                    if name.as_str() == "org.freedesktop.DBus.Error.UnknownMethod" =>
                {
                    proxy.call("Read", &(namespace, key))
                }
                error => Err(error),
            });
        match answer {
            Ok(value) => Some(Some(value)),
            Err(zbus::Error::MethodError(name, ..))
                if name.as_str() == "org.freedesktop.portal.Error.NotFound" =>
            {
                Some(None)
            }
            Err(error) => {
                log::debug!("the desktop portal gave no {namespace} {key}: {error}");
                None
            }
        }
    }

    pub(super) fn boolean(value: &Value<'_>) -> Option<bool> {
        match value {
            Value::Bool(value) => Some(*value),
            Value::Str(text) => text.as_str().trim().parse().ok(),
            Value::Value(inner) => boolean(inner),
            _ => None,
        }
    }

    /// GNOME's numbers arrive as integers; KDE's portal hands its
    /// configuration over as text.
    pub(super) fn integer(value: &Value<'_>) -> Option<i64> {
        match value {
            Value::I32(value) => Some(i64::from(*value)),
            Value::U32(value) => Some(i64::from(*value)),
            Value::I64(value) => Some(*value),
            Value::U64(value) => i64::try_from(*value).ok(),
            Value::Str(text) => text.as_str().trim().parse().ok(),
            Value::Value(inner) => integer(inner),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn period(on: f32, off: f32) -> Option<Blink> {
        Some(Blink::Period { on, off })
    }

    #[test]
    fn windows_phases_and_infinite() {
        assert_eq!(windows_caret(530), period(0.53, 0.53));
        assert_eq!(windows_caret(u32::MAX), Some(Blink::Steady));
        assert_eq!(windows_caret(0), None, "a failed call keeps egui's rate");
    }

    #[test]
    fn macos_phases_cycles_and_the_huge_period_trick() {
        assert_eq!(
            macos_insertion_point(Some(800.0), Some(400.0), None),
            period(0.8, 0.4)
        );
        assert_eq!(
            macos_insertion_point(Some(300.0), None, None),
            period(0.3, 0.3)
        );
        assert_eq!(
            macos_insertion_point(None, None, Some(1000.0)),
            period(0.5, 0.5)
        );
        assert_eq!(
            macos_insertion_point(Some(200_000.0), None, None),
            Some(Blink::Steady)
        );
        assert_eq!(
            macos_insertion_point(Some(500.0), Some(0.0), None),
            Some(Blink::Steady)
        );
        assert_eq!(macos_insertion_point(None, None, None), None);
        assert_eq!(macos_insertion_point(Some(-1.0), None, None), None);
    }

    #[test]
    fn gnome_and_kde_cycles() {
        assert_eq!(gnome(Some(true), Some(1200)), period(0.6, 0.6));
        assert_eq!(gnome(None, Some(1000)), period(0.5, 0.5));
        assert_eq!(gnome(Some(false), Some(1200)), Some(Blink::Steady));
        assert_eq!(gnome(Some(true), None), None);
        assert_eq!(kde(Some(1000)), period(0.5, 0.5));
        assert_eq!(kde(Some(0)), Some(Blink::Steady));
        assert_eq!(kde(None), None);
    }

    #[test]
    fn applying_sets_the_style_or_leaves_egui_s() {
        let default = egui::style::TextCursorStyle::default();
        let mut cursor = default.clone();
        apply(&mut cursor, None);
        assert_eq!(cursor, default);
        apply(&mut cursor, period(0.6, 0.4));
        assert!(cursor.blink);
        assert_eq!((cursor.on_duration, cursor.off_duration), (0.6, 0.4));
        apply(&mut cursor, Some(Blink::Steady));
        assert!(!cursor.blink);
    }

    #[test]
    fn tests_keep_egui_s_rate_whatever_the_machine_says() {
        assert_eq!(system(), None);
        let mut cursor = egui::style::TextCursorStyle::default();
        follow_system(&mut cursor);
        assert_eq!(cursor, egui::style::TextCursorStyle::default());
    }

    /// Draws a focused text field at `time` with the composer's idle pause
    /// and returns how long egui asks to wait before the next frame.
    fn field_frame(ctx: &egui::Context, time: f64, input: bool) -> std::time::Duration {
        let field = egui::Id::new("caret-field");
        ctx.memory_mut(|memory| memory.request_focus(field));
        let mut text = String::new();
        let mut output = ctx.run_ui(
            egui::RawInput {
                time: Some(time),
                ..Default::default()
            },
            |ui| {
                let _cursor_style = crate::motion::idle_text_cursor(ui, input);
                ui.add(egui::TextEdit::singleline(&mut text).id(field));
            },
        );
        output.textures_delta.clear();
        output.viewport_output[&egui::ViewportId::ROOT].repaint_delay
    }

    fn context_with(blink: Blink) -> egui::Context {
        let ctx = crate::motion::tests::reading(false);
        ctx.global_style_mut(|style| apply(&mut style.visuals.text_cursor, Some(blink)));
        ctx
    }

    #[test]
    fn a_steady_cursor_never_asks_for_blink_frames() {
        let ctx = context_with(Blink::Steady);
        for (time, input) in [(1.0, false), (1.3, true), (7.0, false), (8.0, true)] {
            let delay = field_frame(&ctx, time, input);
            assert!(
                delay > std::time::Duration::from_secs(1),
                "repaint in {delay:?} at {time}"
            );
            assert!(!ctx.global_style().visuals.text_cursor.blink);
        }
    }

    #[test]
    fn the_system_rate_paces_the_blink_and_survives_the_idle_pause() {
        let ctx = context_with(Blink::Period { on: 1.6, off: 0.4 });
        // 1.7 s falls in the 0.4 s off phase, which ends at 2.0 s.
        let hidden = field_frame(&ctx, 1.7, false);
        assert!(
            hidden > std::time::Duration::from_millis(250)
                && hidden <= std::time::Duration::from_millis(300),
            "shown again in {hidden:?}"
        );
        // The cycle counts from the last interaction, at time 0 here: 2.5 s
        // is 0.5 s into the second cycle's 1.6 s on, so the cursor hides in
        // 1.1 s, where egui's own rate would hide or show it within 0.5 s.
        let delay = field_frame(&ctx, 2.5, false);
        assert!(
            delay > std::time::Duration::from_millis(1000),
            "repaint in {delay:?}"
        );
        assert!(delay <= std::time::Duration::from_millis(1100));
        // Five seconds after the first frame the idle pause stops the
        // blink, then hands the context back the same rate.
        let idle = field_frame(&ctx, 8.0, false);
        assert!(idle > std::time::Duration::from_secs(2), "idle in {idle:?}");
        let cursor = ctx.global_style().visuals.text_cursor.clone();
        assert!(cursor.blink);
        assert_eq!((cursor.on_duration, cursor.off_duration), (1.6, 0.4));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn portal_values_are_read_through_either_wrapping() {
        use zbus::zvariant::Value;
        let wrapped = |value: Value<'static>| Value::Value(Box::new(value));
        assert_eq!(portal::boolean(&wrapped(Value::Bool(false))), Some(false));
        assert_eq!(portal::integer(&wrapped(Value::I32(1200))), Some(1200));
        assert_eq!(portal::integer(&Value::from("1000")), Some(1000));
        assert_eq!(portal::integer(&Value::from("fast")), None);
    }
}
