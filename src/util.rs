//! Small formatting helpers shared by the views.

use jiff::civil::Date;
use jiff::{Timestamp, Zoned};

/// File-loader identifier for a native path. egui requires a slash after
/// `file://` on Windows or it interprets a drive path as a UNC hostname.
/// Keep native characters: egui's loader does not percent-decode URLs.
pub fn image_uri(path: &std::path::Path) -> String {
    image_uri_for_platform(&path.to_string_lossy(), cfg!(windows))
}

fn image_uri_for_platform(path: &str, windows: bool) -> String {
    format!("file://{}{path}", if windows { "/" } else { "" })
}

/// Converts a Unix timestamp to local time.
fn zoned(unix_seconds: i64) -> Option<Zoned> {
    let timestamp = Timestamp::from_second(unix_seconds).ok()?;
    Some(timestamp.to_zoned(jiff::tz::TimeZone::system()))
}

fn today() -> Date {
    Zoned::now().date()
}

/// Local message time such as "14:05".
pub fn clock(unix_seconds: i64) -> String {
    zoned(unix_seconds)
        .map(|when| format!("{:02}:{:02}", when.hour(), when.minute()))
        .unwrap_or_default()
}

/// WhatsApp transcript timestamp such as `22:41, 8/18/2026`.
pub fn copy_stamp(unix_seconds: i64) -> String {
    zoned(unix_seconds)
        .map(|when| {
            format!(
                "{}:{:02}, {}/{}/{}",
                when.hour(),
                when.minute(),
                when.month(),
                when.day(),
                when.year()
            )
        })
        .unwrap_or_default()
}

/// Chat-row timestamp: time today, weekday this week, or date.
pub fn chat_stamp(unix_seconds: i64) -> String {
    chat_stamp_in(unix_seconds, crate::i18n::Language::English)
}

/// Chat-row timestamp translated for `lang`.
pub fn chat_stamp_in(unix_seconds: i64, lang: crate::i18n::Language) -> String {
    let Some(when) = zoned(unix_seconds) else {
        return String::new();
    };
    stamp_relative_to(when.date(), today(), &when, lang)
}

fn stamp_relative_to(date: Date, today: Date, when: &Zoned, lang: crate::i18n::Language) -> String {
    let days = today
        .since(date)
        .map(|span| span.get_days())
        .unwrap_or(i32::MAX);
    match days {
        0 => format!("{:02}:{:02}", when.hour(), when.minute()),
        1 => crate::i18n::t(lang, "date.yesterday").to_owned(),
        2..=6 => crate::i18n::weekday(lang, date.weekday()).to_owned(),
        _ => short_date_in(date, lang),
    }
}

/// Splits a display name into first name and surname for editor defaults.
pub fn split_name(name: &str) -> (String, String) {
    let name = name.trim();
    match name.split_once(' ') {
        Some((first, rest)) => (first.to_owned(), rest.trim().to_owned()),
        None => (name.to_owned(), String::new()),
    }
}

/// Message-info timestamp with date and minute.
pub fn moment_stamp(unix_seconds: i64) -> String {
    moment_stamp_in(unix_seconds, crate::i18n::Language::English)
}

/// Message-info timestamp with date and minute, translated for `lang`.
pub fn moment_stamp_in(unix_seconds: i64, lang: crate::i18n::Language) -> String {
    let Some(when) = zoned(unix_seconds) else {
        return String::new();
    };
    let time = format!("{:02}:{:02}", when.hour(), when.minute());
    let days = today()
        .since(when.date())
        .map(|span| span.get_days())
        .unwrap_or(i32::MAX);
    match days {
        0 => time,
        1 => format!("{} at {time}", crate::i18n::t(lang, "date.yesterday")),
        2..=6 => format!(
            "{} at {time}",
            crate::i18n::weekday(lang, when.date().weekday())
        ),
        _ => format!("{} at {time}", short_date_in(when.date(), lang)),
    }
}

/// Conversation day-separator label.
pub fn day_label(unix_seconds: i64) -> String {
    day_label_in(unix_seconds, crate::i18n::Language::English)
}

/// Conversation day-separator label, translated for `lang`.
pub fn day_label_in(unix_seconds: i64, lang: crate::i18n::Language) -> String {
    let Some(when) = zoned(unix_seconds) else {
        return String::new();
    };
    let date = when.date();
    let today = today();
    let days = today
        .since(date)
        .map(|span| span.get_days())
        .unwrap_or(i32::MAX);
    match days {
        0 => crate::i18n::t(lang, "date.today").to_owned(),
        1 => crate::i18n::t(lang, "date.yesterday").to_owned(),
        2..=6 => crate::i18n::weekday(lang, date.weekday()).to_owned(),
        _ => long_date_in(date, lang),
    }
}

/// Local calendar day used to group messages.
pub fn day_key(unix_seconds: i64) -> Option<Date> {
    zoned(unix_seconds).map(|when| when.date())
}

fn short_date_in(date: Date, lang: crate::i18n::Language) -> String {
    format!(
        "{} {} {}",
        date.day(),
        &crate::i18n::month(lang, date.month())[..3],
        date.year()
    )
}

fn long_date_in(date: Date, lang: crate::i18n::Language) -> String {
    format!(
        "{}, {} {} {}",
        crate::i18n::weekday(lang, date.weekday()),
        date.day(),
        crate::i18n::month(lang, date.month()),
        date.year()
    )
}

/// The current time as a Unix timestamp.
pub fn now() -> i64 {
    Timestamp::now().as_second()
}

/// Case- and accent-insensitive matching without changing displayed names.
pub fn search_key(text: &str) -> String {
    use icu_normalizer::DecomposingNormalizerBorrowed;
    use icu_properties::{CodePointMapData, props::GeneralCategory};

    if text.is_ascii() {
        return text.to_ascii_lowercase();
    }
    DecomposingNormalizerBorrowed::new_nfd()
        .normalize_iter(text.chars())
        .filter(|c| {
            CodePointMapData::<GeneralCategory>::new().get(*c) != GeneralCategory::NonspacingMark
        })
        .flat_map(char::to_lowercase)
        .collect()
}

/// Duration such as "0:12".
pub fn duration(seconds: u32) -> String {
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

/// File size such as "1.2 MB".
pub fn bytes(size: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut value = size as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{size} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// Up to two initials for a fallback avatar.
pub fn initials(name: &str) -> String {
    let mut words = name
        .split(|character: char| character.is_whitespace() || character == '-')
        .filter(|word| word.chars().any(char::is_alphanumeric));
    let first = words.next();
    let last = words.next_back();
    let mut initials = String::new();
    for word in [first, last].into_iter().flatten() {
        if let Some(character) = word.chars().find(|character| character.is_alphanumeric()) {
            initials.extend(character.to_uppercase());
        }
    }
    if initials.is_empty() {
        initials.push('#');
    }
    initials
}

/// Formats a phone number in international form, like WhatsApp Web
/// (`+55 11 91234-5678`). Falls back to the plus sign with grouped digits
/// when the number does not parse.
pub fn phone(digits: &str) -> String {
    let digits: String = digits.chars().filter(char::is_ascii_digit).collect();
    if digits.is_empty() {
        return String::new();
    }
    // Real phone numbers have at least seven digits; shorter strings use
    // the grouped fallback below instead of a misleading country split.
    if digits.len() >= 7
        && let Ok(parsed) = format_international(&digits)
    {
        return parsed;
    }
    let mut out = String::from("+");
    for (index, character) in digits.chars().enumerate() {
        // Approximate a country code followed by groups of three digits.
        if index == 2 || (index > 2 && (index - 2) % 3 == 0) {
            out.push(' ');
        }
        out.push(character);
    }
    out
}

fn format_international(digits: &str) -> Result<String, phonenumber::ParseError> {
    use phonenumber::Mode;
    let number = phonenumber::parse(None, format!("+{digits}"))?;
    Ok(number.format().mode(Mode::International).to_string())
}

/// Stable id-derived avatar hue.
pub fn hue(seed: &str) -> f32 {
    let mut hash: u32 = 2_166_136_261;
    for byte in seed.bytes() {
        hash ^= u32::from(byte);
        hash = hash.wrapping_mul(16_777_619);
    }
    (hash % 360) as f32
}

/// Embedded SVG app logo used across platform surfaces.
const MARK: &[u8] = include_bytes!("../packaging/icons/zapfast.svg");

/// Rasterizes the logo to straight-alpha RGBA.
pub fn app_icon_rgba(size: usize) -> Vec<u8> {
    let side = size.max(1) as u32;
    let rendered = resvg::usvg::Tree::from_data(MARK, &resvg::usvg::Options::default())
        .ok()
        .and_then(|tree| {
            let mut pixmap = resvg::tiny_skia::Pixmap::new(side, side)?;
            let scale = side as f32 / tree.size().width();
            resvg::render(
                &tree,
                resvg::tiny_skia::Transform::from_scale(scale, scale),
                &mut pixmap.as_mut(),
            );
            Some(
                pixmap
                    .pixels()
                    .iter()
                    .flat_map(|pixel| {
                        let color = pixel.demultiply();
                        [color.red(), color.green(), color.blue(), color.alpha()]
                    })
                    .collect::<Vec<u8>>(),
            )
        });
    match rendered {
        Some(rgba) => rgba,
        // Fall back to an accent disc if the embedded SVG cannot render.
        None => plain_disc(size),
    }
}

fn plain_disc(size: usize) -> Vec<u8> {
    let mut rgba = vec![0u8; size * size * 4];
    let center = size as f32 / 2.0;
    let radius = center - 2.0;
    for y in 0..size {
        for x in 0..size {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let distance = ((px - center).powi(2) + (py - center).powi(2)).sqrt();
            let coverage = (radius - distance + 0.5).clamp(0.0, 1.0);
            let index = (y * size + x) * 4;
            rgba[index] = 0;
            rgba[index + 1] = 168;
            rgba[index + 2] = 132;
            rgba[index + 3] = (coverage * 255.0) as u8;
        }
    }
    rgba
}

/// Converts the logo to a monochrome macOS menu-bar template.
pub fn tray_template_rgba(size: usize) -> Vec<u8> {
    let mut rgba = app_icon_rgba(size);
    for pixel in rgba.as_chunks_mut::<4>().0 {
        if pixel[0] > 200 && pixel[1] > 200 && pixel[2] > 200 {
            pixel[3] = 0;
        }
        pixel[0] = 0;
        pixel[1] = 0;
        pixel[2] = 0;
    }
    rgba
}

#[cfg(test)]
mod tests {
    #[test]
    fn image_paths_keep_the_native_path_after_loader_conversion() {
        for path in [
            r"C:\Users\Ada\photo.jpg",
            r"C:\Users\A B\100% #猫.png",
            r"\\server\share\photo.jpg",
            r"\\?\C:\cache\photo.jpg",
        ] {
            let uri = super::image_uri_for_platform(path, true);
            // Mirrors egui_extras' Windows file loader: its first slash
            // selects a native path, otherwise it prepends a UNC prefix.
            assert_eq!(uri.strip_prefix("file:///").unwrap(), path);
        }
        assert_eq!(
            super::image_uri_for_platform("/home/ada/猫 #1.png", false),
            "file:///home/ada/猫 #1.png"
        );
    }

    #[test]
    fn image_loader_reads_native_paths() {
        use egui::load::BytesPoll;
        let dir = std::env::temp_dir().join(format!("zapfast-image-paths-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("猫 photo 100% #1.png");
        std::fs::write(&path, b"image bytes").unwrap();
        let ctx = egui::Context::default();
        egui_extras::install_image_loaders(&ctx);
        let uri = super::image_uri(&path);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            match ctx.try_load_bytes(&uri).unwrap() {
                BytesPoll::Ready { bytes, .. } => {
                    assert_eq!(bytes.as_ref(), b"image bytes");
                    break;
                }
                BytesPoll::Pending { .. } => {
                    assert!(std::time::Instant::now() < deadline);
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
            }
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    use super::*;

    #[test]
    fn initials_take_first_and_last_word() {
        assert_eq!(initials("Ada Lovelace"), "AL");
        assert_eq!(initials("ada"), "A");
        assert_eq!(initials("  "), "#");
        assert_eq!(initials("🎉 Party Planning"), "PP");
    }

    #[test]
    fn phone_numbers_use_international_format() {
        // WhatsApp Web style masks; Brazil first since most reports come
        // from there.
        assert_eq!(phone("5511912345678"), "+55 11 91234-5678");
        assert_eq!(phone("551133334444"), "+55 11 3333-4444");
        assert_eq!(phone("15551234567"), "+1 555-123-4567");
        assert_eq!(phone("393331234567"), "+39 333 123 4567");
        assert_eq!(phone(""), "");
        // Unparseable input keeps the old grouped fallback.
        assert_eq!(phone("12"), "+12");
    }

    #[test]
    fn stamps_fall_back_to_dates() {
        use crate::i18n::Language;
        let when = Timestamp::from_second(1_700_000_000)
            .expect("valid")
            .to_zoned(jiff::tz::TimeZone::UTC);
        let date = when.date();
        assert_eq!(
            stamp_relative_to(date, date, &when, Language::English),
            "22:13"
        );
        assert_eq!(
            stamp_relative_to(
                date,
                date.tomorrow().expect("date"),
                &when,
                Language::English
            ),
            "Yesterday"
        );
        assert_eq!(
            stamp_relative_to(
                date,
                date.tomorrow().expect("date"),
                &when,
                Language::Portugues
            ),
            "Ontem"
        );
        assert_eq!(
            stamp_relative_to(
                date,
                date.checked_add(jiff::Span::new().days(3)).expect("date"),
                &when,
                Language::English
            ),
            "Tuesday"
        );
        assert_eq!(
            stamp_relative_to(
                date,
                date.checked_add(jiff::Span::new().days(30)).expect("date"),
                &when,
                Language::English
            ),
            "14 Nov 2023"
        );
    }

    #[test]
    fn sizes_and_durations_read_naturally() {
        assert_eq!(bytes(512), "512 B");
        assert_eq!(bytes(2_048), "2.0 KB");
        assert_eq!(bytes(5 * 1024 * 1024), "5.0 MB");
        assert_eq!(duration(75), "1:15");
    }

    #[test]
    fn icon_is_opaque_in_the_middle_and_clear_at_the_corners() {
        let icon = app_icon_rgba(32);
        assert_eq!(icon[3], 0);
        let middle = (16 * 32 + 16) * 4;
        assert_eq!(icon[middle + 3], 255);
    }
}
