//! The translucent window behind the Messages theme, with what each platform
//! offers: AppKit's sidebar material on macOS, Mica through DWM on Windows,
//! and on Wayland a transparent window the compositor can blur (KDE,
//! Hyprland and others do). Elsewhere, or where the platform refuses, the
//! window stays opaque in the theme's colours, which still looks right.
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

/// Whether to make the window transparent when it is created. macOS always
/// does, since every theme there paints over it; elsewhere only when the
/// Messages theme is on, so other themes keep the opaque window they had.
/// Transparency cannot be added to a window later, so a theme chosen while a
/// window is open turns translucent from the next one.
pub fn transparent_window(messages: bool) -> bool {
    cfg!(target_os = "macos") || (messages && cfg!(any(windows, target_os = "linux")))
}

/// Gives the window its material while `palette` wants one, takes it away
/// otherwise, and records which for [`active`]. Cheap to call every frame.
pub fn apply(frame: &eframe::Frame, palette: &crate::theme::Palette, transparent: bool) {
    let wanted = palette.vibrant() && transparent;
    ACTIVE.store(platform::apply(frame, palette, wanted), Ordering::Relaxed);
}

#[cfg(target_os = "macos")]
mod platform {
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
            // client area; all of it, while wanted.
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

    /// On Wayland every window is composited, and compositors that blur
    /// (KDE, Hyprland) blur what shows through. Under X11 a compositor may
    /// be missing, and the window would show black behind it, so it stays
    /// opaque there.
    pub fn apply(frame: &eframe::Frame, _palette: &crate::theme::Palette, wanted: bool) -> bool {
        wanted
            && frame
                .window_handle()
                .is_ok_and(|handle| matches!(handle.as_raw(), RawWindowHandle::Wayland(_)))
    }
}

#[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
mod platform {
    pub fn apply(_frame: &eframe::Frame, _palette: &crate::theme::Palette, _wanted: bool) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Other themes keep an opaque window wherever they always had one.
    #[test]
    fn only_messages_makes_a_window_transparent_off_macos() {
        assert!(transparent_window(true) || !cfg!(any(windows, target_os = "linux")));
        assert_eq!(transparent_window(false), cfg!(target_os = "macos"));
    }
}
