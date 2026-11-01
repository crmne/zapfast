//! The translucent window behind the Messages theme, with what each platform
//! offers: AppKit's sidebar material on macOS, Mica through DWM on Windows,
//! and on Hyprland a transparent window its compositor blurs. Elsewhere, or
//! where the platform refuses, the window stays opaque in the theme's
//! colours, which still looks right.
//!
//! The theme draws its frosted header, glass and tints itself, everywhere;
//! only whether the desktop shows through the window is decided here, once
//! per frame, and [`active`] tells the rest of the interface.

use std::sync::atomic::{AtomicBool, Ordering};

static ACTIVE: AtomicBool = AtomicBool::new(false);

/// Whether the desktop shows through the window this frame: the Messages
/// theme is on and the platform gave the window its material.
pub fn active() -> bool {
    ACTIVE.load(Ordering::Relaxed)
}

/// Whether to make the window transparent when it is created. The Messages
/// theme needs it where the platform can show the desktop through it. On
/// macOS and Linux a theme whose window colour has alpha (`see_through`)
/// also needs it: it shows the desktop, which a compositor may blur. On an
/// opaque surface its translucent pixels are never composited and leave
/// trails (#593). Other themes keep the opaque window they had. Transparency
/// cannot be added to a window later, so macOS replaces the window when a
/// theme changes this choice.
pub fn transparent_window(messages: bool, see_through: bool) -> bool {
    (messages && platform::may_show_through())
        || (see_through && cfg!(any(target_os = "macos", target_os = "linux")))
}

/// Whether the window on screen was made transparent, so a frame may clear
/// to a colour with alpha. Set by `main` as each window is made.
static WINDOW_TRANSPARENT: AtomicBool = AtomicBool::new(false);

/// Records whether the window just made is transparent.
pub fn set_window_transparent(transparent: bool) {
    WINDOW_TRANSPARENT.store(transparent, Ordering::Relaxed);
}

/// The colour a frame clears to: `window`, made opaque on an opaque window,
/// where alpha would leave the last frame's pixels behind (a theme with
/// alpha chosen while the window is open, until the next one is made).
pub fn clear_color(window: egui::Color32) -> egui::Color32 {
    if WINDOW_TRANSPARENT.load(Ordering::Relaxed) {
        window
    } else {
        egui::Color32::from_rgb(window.r(), window.g(), window.b())
    }
}

/// Gives the window its material while `palette` wants one, takes it away
/// otherwise, and records which for [`active`]. Cheap to call every frame.
pub fn apply(frame: &eframe::Frame, palette: &crate::theme::Palette, transparent: bool) {
    let wanted = palette.vibrant() && transparent;
    ACTIVE.store(platform::apply(frame, palette, wanted), Ordering::Relaxed);
}

#[cfg(target_os = "macos")]
mod platform {
    pub fn may_show_through() -> bool {
        true
    }

    pub fn apply(frame: &eframe::Frame, palette: &crate::theme::Palette, wanted: bool) -> bool {
        crate::macos::vibrancy(frame, palette, wanted);
        wanted
    }
}

#[cfg(windows)]
mod platform {
    use std::sync::Mutex;

    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Graphics::Dwm::{
        DWMSBT_MAINWINDOW, DWMSBT_NONE, DWMWA_SYSTEMBACKDROP_TYPE, DWMWA_USE_IMMERSIVE_DARK_MODE,
        DwmExtendFrameIntoClientArea, DwmSetWindowAttribute,
    };
    use windows::Win32::UI::Controls::MARGINS;
    use windows::core::BOOL;

    /// Windows 11 22H2 and later; earlier versions refuse Mica in `apply`,
    /// and the window stays opaque.
    pub fn may_show_through() -> bool {
        true
    }

    /// The window last given a backdrop, whether it was dark, and whether
    /// Windows accepted it, so DWM is asked only when something changes.
    static APPLIED: Mutex<Option<(isize, bool, bool, bool)>> = Mutex::new(None);

    pub fn apply(frame: &eframe::Frame, palette: &crate::theme::Palette, wanted: bool) -> bool {
        let Ok(handle) = frame.window_handle() else {
            return false;
        };
        let RawWindowHandle::Win32(handle) = handle.as_raw() else {
            return false;
        };
        let hwnd = HWND(handle.hwnd.get() as *mut _);
        let key = (handle.hwnd.get(), wanted, palette.dark);
        let mut applied = APPLIED.lock().unwrap_or_else(|p| p.into_inner());
        if let Some((window, was_wanted, dark, accepted)) = *applied
            && (window, was_wanted, dark) == key
        {
            return accepted;
        }
        // SAFETY: a live window of this process, on its own thread; the
        // attributes are plain values of the sizes DWM expects.
        let accepted = unsafe {
            let dark = BOOL::from(palette.dark);
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_USE_IMMERSIVE_DARK_MODE,
                (&raw const dark).cast(),
                size_of::<BOOL>() as u32,
            );
            let backdrop = if wanted {
                DWMSBT_MAINWINDOW
            } else {
                DWMSBT_NONE
            };
            // Mica needs Windows 11 22H2; earlier versions refuse, and the
            // window stays opaque.
            let set = DwmSetWindowAttribute(
                hwnd,
                DWMWA_SYSTEMBACKDROP_TYPE,
                (&raw const backdrop).cast(),
                size_of_val(&backdrop) as u32,
            );
            // The backdrop shows only where the frame extends over the
            // client area; all of it, while wanted. The custom title bar
            // leaves the frame alone: without decorations the window has no
            // caption for DWM to draw, and the buttons ZapFast draws sit in
            // the client area over the same Mica.
            let inset = if wanted { -1 } else { 0 };
            let margins = MARGINS {
                cxLeftWidth: inset,
                cxRightWidth: inset,
                cyTopHeight: inset,
                cyBottomHeight: inset,
            };
            let extended = DwmExtendFrameIntoClientArea(hwnd, &margins);
            wanted && set.is_ok() && extended.is_ok()
        };
        if wanted && !accepted {
            log::info!("Mica is unavailable here; the Messages theme stays opaque");
        }
        *applied = Some((key.0, key.1, key.2, accepted));
        accepted
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    /// Translucent only where the compositor is known to blur what shows
    /// through: Hyprland blurs translucent windows unless told not to. Other
    /// Wayland compositors blur only windows that ask (KDE's blur protocol,
    /// which ZapFast does not speak yet) or not at all (GNOME), and an
    /// unblurred desktop behind the sidebar would cost its text contrast; X11
    /// may lack a compositor entirely. All of them keep the opaque window.
    pub fn apply(frame: &eframe::Frame, _palette: &crate::theme::Palette, wanted: bool) -> bool {
        wanted
            && may_show_through()
            && frame
                .window_handle()
                .is_ok_and(|handle| matches!(handle.as_raw(), RawWindowHandle::Wayland(_)))
    }

    pub fn may_show_through() -> bool {
        blurs(
            std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some(),
            std::env::var("XDG_CURRENT_DESKTOP").ok().as_deref(),
        )
    }

    /// Whether the session's compositor blurs translucent windows by itself:
    /// Hyprland, when both its instance signature and the desktop name say
    /// so, since either alone can be left over from another session.
    pub fn blurs(signature: bool, desktop: Option<&str>) -> bool {
        signature
            && desktop.is_some_and(|desktop| {
                desktop
                    .split(':')
                    .any(|name| name.eq_ignore_ascii_case("Hyprland"))
            })
    }

    #[cfg(test)]
    mod tests {
        /// Only a compositor known to blur gets the translucent window.
        #[test]
        fn only_a_blurring_compositor_gets_translucency() {
            use super::blurs;
            assert!(blurs(true, Some("Hyprland")));
            assert!(blurs(true, Some("Hyprland:wlroots")));
            // A signature left over, or a desktop name alone, is not enough.
            assert!(!blurs(true, Some("GNOME")));
            assert!(!blurs(true, None));
            assert!(!blurs(false, Some("Hyprland")));
            assert!(!blurs(false, Some("KDE")));
        }
    }
}

#[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
mod platform {
    pub fn may_show_through() -> bool {
        false
    }

    pub fn apply(_frame: &eframe::Frame, _palette: &crate::theme::Palette, _wanted: bool) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Other themes keep an opaque window wherever they always had one, and
    /// Messages asks for a transparent one only where it can show through.
    #[test]
    fn opaque_themes_use_an_opaque_window() {
        assert!(!transparent_window(false, false));
        assert_eq!(
            transparent_window(true, false),
            platform::may_show_through()
        );
    }

    /// A Linux theme whose window colour has alpha gets a transparent window
    /// (#593), and an opaque window never clears to alpha.
    #[test]
    fn a_see_through_theme_gets_a_transparent_window_on_linux() {
        assert_eq!(
            transparent_window(false, true),
            cfg!(any(target_os = "macos", target_os = "linux"))
        );
        let glass = egui::Color32::from_rgba_premultiplied(0, 0, 0, 0);
        set_window_transparent(false);
        assert_eq!(clear_color(glass).a(), 255);
        set_window_transparent(true);
        assert_eq!(clear_color(glass), glass);
        set_window_transparent(false);
    }
}
