//! The Windows taskbar badge, applied as a shell overlay to the app's button.

use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HWND, RPC_E_CHANGED_MODE};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
    CoUninitialize,
};
use windows::Win32::UI::Shell::{ITaskbarList3, TaskbarList};
use windows::Win32::UI::WindowsAndMessaging::{CreateIcon, DestroyIcon, HICON};
use windows::core::PCWSTR;

const ICON_SIZE: usize = 16;

const PADDING_X: usize = 1;
const BORDER: usize = 1;

const BORDER_COLOR: [u8; 4] = [27, 31, 35, 255];
const BADGE_COLOR: [u8; 4] = [220, 53, 69, 255];
const TEXT_COLOR: [u8; 4] = [255, 255, 255, 255];

/// Attempts at one count before waiting for the taskbar to announce itself.
const MAX_ATTEMPTS: u8 = 5;
const FIRST_RETRY: Duration = Duration::from_millis(500);

/// The unread total, kept on the interface thread for the window to apply.
#[derive(Default)]
pub struct Badge {
    shown: Option<u32>,
}

impl Badge {
    /// Records the unread total.
    pub fn set(&mut self, count: u32) {
        self.shown = Some(count);
    }

    /// The latest unread total, once one is known.
    pub fn count(&self) -> Option<u32> {
        self.shown
    }
}

/// One window's taskbar overlay: what it shows and when to try again.
#[derive(Default)]
pub struct Taskbar {
    // Declared before `_com` so it is released while COM is still initialized.
    list: Option<ITaskbarList3>,
    _com: Option<ComGuard>,
    schedule: Schedule,
}

impl Taskbar {
    /// Forgets the applied overlay, as when Explorer recreates the button.
    pub fn reset(&mut self) {
        self.list = None;
        self.schedule = Schedule::default();
    }

    /// Shows `count` on the window's button when it differs from what is shown
    /// and no retry is pending. Returns when to try again after a failure.
    pub fn update(&mut self, hwnd: isize, count: u32) -> Option<Instant> {
        let now = Instant::now();
        if !self.schedule.due(count, now) {
            return None;
        }
        match self.apply(hwnd, count) {
            Ok(()) => {
                self.schedule.succeeded(count);
                None
            }
            Err(error) => {
                // Explorer may have restarted; the next attempt asks it afresh.
                self.list = None;
                let retry = self.schedule.failed(count, now);
                if retry.is_none() {
                    log::warn!("could not update the Windows taskbar badge: {error}");
                }
                retry
            }
        }
    }

    fn apply(&mut self, hwnd: isize, count: u32) -> windows::core::Result<()> {
        let list = match &self.list {
            Some(list) => list,
            None => {
                if self._com.is_none() {
                    self._com = Some(ComGuard::new()?);
                }
                // SAFETY: COM is initialized on this thread, and TaskbarList is an
                // in-process server.
                let list: ITaskbarList3 =
                    unsafe { CoCreateInstance(&TaskbarList, None, CLSCTX_INPROC_SERVER)? };
                // SAFETY: HrInit must precede all other ITaskbarList calls.
                unsafe { list.HrInit()? };
                self.list.insert(list)
            }
        };
        apply_overlay(list, HWND(hwnd as *mut std::ffi::c_void), count)
    }
}

/// Keeps COM initialized on the window thread while a taskbar list is held.
struct ComGuard(bool);

impl ComGuard {
    fn new() -> windows::core::Result<Self> {
        // SAFETY: the window shell calls this on the window thread.
        let initialized = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        if initialized.is_err() && initialized != RPC_E_CHANGED_MODE {
            return Err(initialized.into());
        }
        Ok(Self(initialized.is_ok()))
    }
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        if self.0 {
            // SAFETY: each successful CoInitializeEx on this thread needs one matching call.
            unsafe { CoUninitialize() };
        }
    }
}

/// Which count is shown, and the backoff for one that failed to apply.
#[derive(Default)]
struct Schedule {
    applied: Option<u32>,
    retry: Option<Retry>,
}

struct Retry {
    count: u32,
    attempts: u8,
    /// `None` once the attempts are spent.
    at: Option<Instant>,
}

impl Schedule {
    fn due(&self, count: u32, now: Instant) -> bool {
        if self.applied == Some(count) {
            return false;
        }
        match &self.retry {
            Some(retry) if retry.count == count => retry.at.is_some_and(|at| now >= at),
            _ => true,
        }
    }

    fn succeeded(&mut self, count: u32) {
        self.applied = Some(count);
        self.retry = None;
    }

    /// Returns when to try again, or `None` when this count has had its attempts.
    fn failed(&mut self, count: u32, now: Instant) -> Option<Instant> {
        let attempts = match &self.retry {
            Some(retry) if retry.count == count => retry.attempts + 1,
            _ => 1,
        };
        let at = (attempts < MAX_ATTEMPTS)
            .then(|| now + FIRST_RETRY * 2u32.pow(u32::from(attempts - 1)));
        self.retry = Some(Retry {
            count,
            attempts,
            at,
        });
        at
    }
}

/// Uses the taskbar's overlay API, which also updates pinned and grouped buttons.
fn apply_overlay(list: &ITaskbarList3, hwnd: HWND, count: u32) -> windows::core::Result<()> {
    if count == 0 {
        // SAFETY: a null icon removes this window's overlay.
        return unsafe { list.SetOverlayIcon(hwnd, HICON::default(), PCWSTR::null()) };
    }

    let mut pixels = overlay_rgba(count);
    let mut and_mask = [0u8; ICON_SIZE * ICON_SIZE / 8];
    for (index, pixel) in pixels.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        if pixel[3] == 0 {
            and_mask[index / 8] |= 1 << (7 - index % 8);
        }
        // CreateIcon takes BGRA.
        pixel.swap(0, 2);
    }
    // SAFETY: both buffers cover the entire 16x16 image for this call.
    let icon = unsafe {
        CreateIcon(
            None,
            ICON_SIZE as i32,
            ICON_SIZE as i32,
            1,
            32,
            and_mask.as_ptr(),
            pixels.as_ptr(),
        )?
    };
    let description: Vec<u16> = description(count)
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    // SAFETY: the window and icon are valid for the duration of this call.
    let result = unsafe { list.SetOverlayIcon(hwnd, icon, PCWSTR(description.as_ptr())) };
    // SAFETY: the taskbar copies the icon before SetOverlayIcon returns. A
    // failure here leaks one small icon and must not undo a shown overlay.
    let _ = unsafe { DestroyIcon(icon) };
    result
}

/// The accessible text of the overlay.
fn description(count: u32) -> String {
    match count {
        1 => "1 unread message".to_owned(),
        _ => format!("{count} unread messages"),
    }
}

fn label(count: u32) -> Option<String> {
    match count {
        0 => None,
        1..=99 => Some(count.to_string()),
        _ => Some("99+".to_owned()),
    }
}

fn overlay_rgba(count: u32) -> Vec<u8> {
    let mut rgba = vec![0; ICON_SIZE * ICON_SIZE * 4];
    let Some(label) = label(count) else {
        return rgba;
    };

    let characters = label.chars().count();
    let compact = characters > 2;
    let glyph_width = if compact { 3 } else { 5 };
    let glyph_height = if compact { 5 } else { 7 };
    let padding_y = if compact { 3 } else { 2 };
    let text_width = characters * glyph_width + characters.saturating_sub(1);
    let inner_height = glyph_height + padding_y * 2;
    let inner_width = (text_width + PADDING_X * 2).max(inner_height);
    let outer_width = inner_width + BORDER * 2;
    let outer_height = inner_height + BORDER * 2;
    let left = (ICON_SIZE - outer_width) / 2;
    let top = (ICON_SIZE - outer_height) / 2;

    fill_capsule(
        &mut rgba,
        left,
        top,
        outer_width,
        outer_height,
        BORDER_COLOR,
    );
    fill_capsule(
        &mut rgba,
        left + BORDER,
        top + BORDER,
        inner_width,
        inner_height,
        BADGE_COLOR,
    );

    let text_left = left + (outer_width - text_width) / 2;
    let text_top = top + (outer_height - glyph_height) / 2;
    for (index, character) in label.chars().enumerate() {
        let rows = if compact {
            glyph_small(character).map(|rows| [rows[0], rows[1], rows[2], rows[3], rows[4], 0, 0])
        } else {
            glyph_large(character)
        };
        let Some(rows) = rows else {
            continue;
        };
        let glyph_left = text_left + index * (glyph_width + 1);
        for (row, bits) in rows.into_iter().take(glyph_height).enumerate() {
            for column in 0..glyph_width {
                if bits & (1 << (glyph_width - column - 1)) != 0 {
                    set_pixel(&mut rgba, glyph_left + column, text_top + row, TEXT_COLOR);
                }
            }
        }
    }
    rgba
}

fn glyph_small(character: char) -> Option<[u8; 5]> {
    Some(match character {
        '0' => [0b111, 0b101, 0b101, 0b101, 0b111],
        '1' => [0b010, 0b110, 0b010, 0b010, 0b111],
        '2' => [0b111, 0b001, 0b111, 0b100, 0b111],
        '3' => [0b111, 0b001, 0b111, 0b001, 0b111],
        '4' => [0b101, 0b101, 0b111, 0b001, 0b001],
        '5' => [0b111, 0b100, 0b111, 0b001, 0b111],
        '6' => [0b111, 0b100, 0b111, 0b101, 0b111],
        '7' => [0b111, 0b001, 0b010, 0b010, 0b010],
        '8' => [0b111, 0b101, 0b111, 0b101, 0b111],
        '9' => [0b111, 0b101, 0b111, 0b001, 0b111],
        '+' => [0b000, 0b010, 0b111, 0b010, 0b000],
        _ => return None,
    })
}

fn glyph_large(character: char) -> Option<[u8; 7]> {
    Some(match character {
        '0' => [
            0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110,
        ],
        '1' => [
            0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110,
        ],
        '2' => [
            0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111,
        ],
        '3' => [
            0b11110, 0b00001, 0b00001, 0b01110, 0b00001, 0b00001, 0b11110,
        ],
        '4' => [
            0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010,
        ],
        '5' => [
            0b11111, 0b10000, 0b10000, 0b11110, 0b00001, 0b00001, 0b11110,
        ],
        '6' => [
            0b01110, 0b10000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110,
        ],
        '7' => [
            0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000,
        ],
        '8' => [
            0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110,
        ],
        '9' => [
            0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00001, 0b01110,
        ],
        _ => return None,
    })
}

fn fill_capsule(
    rgba: &mut [u8],
    left: usize,
    top: usize,
    width: usize,
    height: usize,
    color: [u8; 4],
) {
    let radius = height as f32 / 2.0;
    let center_y = top as f32 + radius;
    let left_center = left as f32 + radius;
    let right_center = left as f32 + width as f32 - radius;
    for y in top..top + height {
        for x in left..left + width {
            let px = x as f32 + 0.5;
            let center_x = if px < left_center {
                left_center
            } else if px > right_center {
                right_center
            } else {
                set_pixel(rgba, x, y, color);
                continue;
            };
            if (px - center_x).powi(2) + (y as f32 + 0.5 - center_y).powi(2) <= radius.powi(2) {
                set_pixel(rgba, x, y, color);
            }
        }
    }
}

fn set_pixel(rgba: &mut [u8], x: usize, y: usize, color: [u8; 4]) {
    let start = (y * ICON_SIZE + x) * 4;
    rgba[start..start + 4].copy_from_slice(&color);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn large_counts_use_a_compact_label() {
        assert_eq!(label(0), None);
        assert_eq!(label(7).as_deref(), Some("7"));
        assert_eq!(label(42).as_deref(), Some("42"));
        assert_eq!(label(100).as_deref(), Some("99+"));
    }

    #[test]
    fn zero_has_no_overlay_pixels() {
        assert!(overlay_rgba(0).iter().all(|channel| *channel == 0));
    }

    #[test]
    fn unread_count_is_drawn_on_a_transparent_overlay() {
        for count in [1, 42, 100] {
            let icon = overlay_rgba(count);
            let pixels = icon.as_chunks::<4>().0;
            assert!(pixels.contains(&BADGE_COLOR));
            assert!(pixels.contains(&TEXT_COLOR));
            assert_eq!(pixels[0], [0; 4]);
        }
    }

    #[test]
    fn plus_sign_is_centred() {
        assert_eq!(glyph_small('+'), Some([0b000, 0b010, 0b111, 0b010, 0b000]));
    }

    #[test]
    fn description_agrees_with_the_count() {
        assert_eq!(description(1), "1 unread message");
        assert_eq!(description(3), "3 unread messages");
    }

    #[test]
    fn an_applied_count_is_not_applied_again() {
        let now = Instant::now();
        let mut schedule = Schedule::default();
        assert!(schedule.due(2, now));
        schedule.succeeded(2);
        assert!(!schedule.due(2, now));
        assert!(schedule.due(3, now));
    }

    #[test]
    fn a_failed_count_waits_for_its_backoff() {
        let now = Instant::now();
        let mut schedule = Schedule::default();
        let at = schedule.failed(2, now).unwrap();
        assert_eq!(at, now + FIRST_RETRY);
        assert!(!schedule.due(2, now));
        assert!(schedule.due(2, at));
        assert_eq!(schedule.failed(2, at), Some(at + FIRST_RETRY * 2));
        // A new count does not wait for the old one's backoff.
        assert!(schedule.due(3, now));
    }

    #[test]
    fn a_count_stops_retrying_after_its_attempts() {
        let now = Instant::now();
        let mut schedule = Schedule::default();
        for _ in 1..MAX_ATTEMPTS {
            assert!(schedule.failed(2, now).is_some());
        }
        assert_eq!(schedule.failed(2, now), None);
        assert!(!schedule.due(2, now + Duration::from_secs(3600)));
        // A changed count, or a recreated button, tries again.
        assert!(schedule.due(3, now));
        schedule = Schedule::default();
        assert!(schedule.due(2, now));
    }
}
