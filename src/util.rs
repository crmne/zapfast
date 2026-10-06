//! Small formatting helpers shared by the views.

use jiff::civil::Date;
use jiff::{Timestamp, Zoned};

use crate::i18n::Locale;

/// File-loader identifier for a native path. egui requires a slash after
/// `file://` on Windows or it interprets a drive path as a UNC hostname.
/// Keep native characters: egui's loader does not percent-decode URLs.
pub fn image_uri(path: &std::path::Path) -> String {
    image_uri_for_platform(&path.to_string_lossy(), cfg!(windows))
}

fn image_uri_for_platform(path: &str, windows: bool) -> String {
    format!("file://{}{path}", if windows { "/" } else { "" })
}

/// The local time zone.
fn zone() -> jiff::tz::TimeZone {
    #[cfg(test)]
    if let Some(clock) = fixed_clock::get() {
        return clock.zone;
    }
    jiff::tz::TimeZone::system()
}

/// Converts a Unix timestamp to local time.
fn zoned(unix_seconds: i64) -> Option<Zoned> {
    let timestamp = Timestamp::from_second(unix_seconds).ok()?;
    Some(timestamp.to_zoned(zone()))
}

/// Today's local date.
pub fn today() -> Date {
    #[cfg(test)]
    if let Some(when) = fixed_clock::get().and_then(|clock| zoned(clock.now)) {
        return when.date();
    }
    Zoned::now().date()
}

/// Whether the system shows times on a 12-hour clock. Read once per run.
pub fn twelve_hour_clock() -> bool {
    #[cfg(test)]
    if let Some(clock) = fixed_clock::get() {
        return clock.twelve_hour;
    }
    static TWELVE_HOUR: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *TWELVE_HOUR.get_or_init(clock_preference::twelve_hour)
}

/// Whether a time pattern uses a 12-hour hour field. Windows patterns spell
/// it `h`, ICU patterns `h` or `K`, and C formats `%I`, `%l`, or `%r`.
fn pattern_is_twelve_hour(pattern: &str) -> bool {
    if pattern.contains('%') {
        return ["%I", "%l", "%r", "%p"]
            .iter()
            .any(|field| pattern.contains(field));
    }
    // Skip quoted literals such as 'h' in "HH 'h' mm".
    let mut quoted = false;
    for character in pattern.chars() {
        match character {
            '\'' => quoted = !quoted,
            'h' | 'K' if !quoted => return true,
            _ => {}
        }
    }
    false
}

#[cfg(target_os = "linux")]
mod clock_preference {
    pub fn twelve_hour() -> bool {
        gnome().unwrap_or_else(locale)
    }

    /// GNOME keeps its own clock format, independent of the locale.
    fn gnome() -> Option<bool> {
        let desktop = std::env::var("XDG_CURRENT_DESKTOP").ok()?;
        if !desktop
            .split(':')
            .any(|name| name.eq_ignore_ascii_case("GNOME"))
        {
            return None;
        }
        let output = std::process::Command::new("gsettings")
            .args(["get", "org.gnome.desktop.interface", "clock-format"])
            .stderr(std::process::Stdio::null())
            .output()
            .ok()?;
        match String::from_utf8_lossy(&output.stdout).trim() {
            "'12h'" => Some(true),
            "'24h'" => Some(false),
            _ => None,
        }
    }

    /// The time locale's own format, read without changing the process locale.
    fn locale() -> bool {
        // SAFETY: an empty name selects the environment's LC_TIME; the locale
        // is freed after its format string has been copied.
        unsafe {
            let locale = libc::newlocale(libc::LC_TIME_MASK, c"".as_ptr(), std::ptr::null_mut());
            if locale.is_null() {
                return false;
            }
            let format = libc::nl_langinfo_l(libc::T_FMT, locale);
            let twelve = !format.is_null()
                && super::pattern_is_twelve_hour(
                    &std::ffi::CStr::from_ptr(format).to_string_lossy(),
                );
            libc::freelocale(locale);
            twelve
        }
    }
}

#[cfg(target_os = "macos")]
mod clock_preference {
    use objc2_foundation::{NSDateFormatter, NSLocale, NSString};

    /// The "j" template asks for the locale's preferred hour, which follows
    /// the 24-hour switch in System Settings.
    pub fn twelve_hour() -> bool {
        let locale = NSLocale::currentLocale();
        NSDateFormatter::dateFormatFromTemplate_options_locale(
            &NSString::from_str("j"),
            0,
            Some(&locale),
        )
        .is_some_and(|pattern| super::pattern_is_twelve_hour(&pattern.to_string()))
    }
}

#[cfg(windows)]
mod clock_preference {
    use windows_sys::Win32::Globalization::{GetLocaleInfoEx, LOCALE_STIMEFORMAT};

    /// The user's time format from Region settings, such as "h:mm:ss tt".
    pub fn twelve_hour() -> bool {
        let mut buffer = [0u16; 80];
        // SAFETY: a null name means the user's default locale; the buffer
        // length is in UTF-16 units.
        let written = unsafe {
            GetLocaleInfoEx(
                std::ptr::null(),
                LOCALE_STIMEFORMAT,
                buffer.as_mut_ptr(),
                buffer.len() as i32,
            )
        };
        written > 0
            && super::pattern_is_twelve_hour(&String::from_utf16_lossy(
                &buffer[..written as usize - 1],
            ))
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
mod clock_preference {
    pub fn twelve_hour() -> bool {
        false
    }
}

/// Time of day on the system's clock, such as "14:05" or "2:05 PM".
fn hour_minute(when: &Zoned) -> String {
    time_of_day(when.hour(), when.minute(), twelve_hour_clock())
}

/// Formats an hour and minute of day as 24-hour "14:05" or 12-hour "2:05 PM".
fn time_of_day(hour: i8, minute: i8, twelve_hour: bool) -> String {
    if twelve_hour {
        let (hour, meridiem) = match hour {
            0 => (12, "AM"),
            1..=11 => (hour, "AM"),
            12 => (12, "PM"),
            _ => (hour - 12, "PM"),
        };
        format!("{hour}:{minute:02} {meridiem}")
    } else {
        format!("{hour:02}:{minute:02}")
    }
}

/// Local message time such as "14:05".
pub fn clock(unix_seconds: i64) -> String {
    zoned(unix_seconds)
        .map(|when| hour_minute(&when))
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
pub fn chat_stamp(locale: Locale, unix_seconds: i64) -> String {
    let Some(when) = zoned(unix_seconds) else {
        return String::new();
    };
    stamp_relative_to(locale, when.date(), today(), &when)
}

fn stamp_relative_to(locale: Locale, date: Date, today: Date, when: &Zoned) -> String {
    let days = today
        .since(date)
        .map(|span| span.get_days())
        .unwrap_or(i32::MAX);
    match days {
        0 => hour_minute(when),
        1 => crate::i18n::gettext(locale, "Yesterday").into_owned(),
        2..=6 => weekday_name(locale, date.weekday()),
        _ => short_date(locale, date),
    }
}

/// The first-name and last-name fields of the contact editor for a saved
/// `name`, given the first name saved with it.
///
/// Both may hold several words, so the name is never cut at a space: the
/// last name is what follows the saved first name. Without a first name to go
/// by (a profile name, a contact synced before first names were kept, or a
/// first name the full name does not start with) the whole name stays in the
/// first field, so saving it unchanged keeps it whole.
pub fn editor_names(name: &str, first: Option<&str>) -> (String, String) {
    let name = name.trim();
    if let Some(first) = first.map(str::trim).filter(|first| !first.is_empty())
        && let Some(rest) = name.strip_prefix(first)
        && (rest.is_empty() || rest.starts_with(char::is_whitespace))
    {
        return (first.to_owned(), rest.trim().to_owned());
    }
    (name.to_owned(), String::new())
}

/// Message-info timestamp with date and minute.
pub fn moment_stamp(locale: Locale, unix_seconds: i64) -> String {
    let Some(when) = zoned(unix_seconds) else {
        return String::new();
    };
    let time = hour_minute(&when);
    let days = today()
        .since(when.date())
        .map(|span| span.get_days())
        .unwrap_or(i32::MAX);
    match days {
        0 => time,
        1 => crate::i18n::gettext(locale, "Yesterday at {time}").replace("{time}", &time),
        2..=6 => crate::i18n::gettext(locale, "{weekday} at {time}")
            .replace("{weekday}", &weekday_name(locale, when.date().weekday()))
            .replace("{time}", &time),
        _ => crate::i18n::gettext(locale, "{date} at {time}")
            .replace("{date}", &short_date(locale, when.date()))
            .replace("{time}", &time),
    }
}

/// A contact's last-seen line as WhatsApp words it: "last seen today at
/// 14:05", "last seen yesterday at 14:05", a weekday within the week, and
/// only the date before that. The time follows the system's clock.
pub fn last_seen(locale: Locale, unix_seconds: i64) -> String {
    let Some(when) = zoned(unix_seconds) else {
        return String::new();
    };
    last_seen_relative_to(locale, today(), &when)
}

fn last_seen_relative_to(locale: Locale, today: Date, when: &Zoned) -> String {
    use crate::i18n::gettext;
    let date = when.date();
    let time = hour_minute(when);
    let days = today
        .since(date)
        .map(|span| span.get_days())
        .unwrap_or(i32::MAX);
    match days {
        // A clock slightly ahead of ours still means today.
        ..=0 => gettext(locale, "last seen today at {time}").replace("{time}", &time),
        1 => gettext(locale, "last seen yesterday at {time}").replace("{time}", &time),
        2..=6 => {
            let weekday = weekday_name(locale, date.weekday());
            // Mid-sentence, only English and German keep the capital.
            let weekday = match locale {
                Locale::English | Locale::German => weekday,
                _ => weekday.to_lowercase(),
            };
            gettext(locale, "last seen {weekday} at {time}")
                .replace("{weekday}", &weekday)
                .replace("{time}", &time)
        }
        _ => gettext(locale, "last seen {date}").replace("{date}", &short_date(locale, date)),
    }
}

/// Conversation day-separator label.
pub fn day_label(locale: Locale, unix_seconds: i64) -> String {
    let Some(when) = zoned(unix_seconds) else {
        return String::new();
    };
    day_label_relative_to(locale, when.date(), today())
}

/// Formats a localized day label relative to a supplied reference date.
fn day_label_relative_to(locale: Locale, date: Date, today: Date) -> String {
    let days = today
        .since(date)
        .map(|span| span.get_days())
        .unwrap_or(i32::MAX);
    let label = match days {
        0 => return crate::i18n::gettext(locale, "Today").into_owned(),
        1 => crate::i18n::gettext(locale, "Yesterday").into_owned(),
        2..=6 => weekday_name(locale, date.weekday()),
        _ => return long_date(locale, date),
    };
    let calendar = if locale == Locale::English {
        let day = date.day();
        let suffix = match (day % 100, day % 10) {
            (11..=13, _) => "th",
            (_, 1) => "st",
            (_, 2) => "nd",
            (_, 3) => "rd",
            _ => "th",
        };
        format!("{} {day}{suffix}", month_name(locale, date.month()))
    } else {
        crate::i18n::gettext(locale, "{day} {month}")
            .replace("{day}", &date.day().to_string())
            .replace("{month}", &date_month_name(locale, date.month()))
    };
    let calendar = if date.year() == today.year() {
        calendar
    } else {
        crate::i18n::gettext(locale, "{date}, {year}")
            .replace("{date}", &calendar)
            .replace("{year}", &date.year().to_string())
    };
    crate::i18n::gettext(locale, "{label} ({date})")
        .replace("{label}", &label)
        .replace("{date}", &calendar)
}

/// Local calendar day used to group messages.
pub fn day_key(unix_seconds: i64) -> Option<Date> {
    zoned(unix_seconds).map(|when| when.date())
}

/// Unix-second half-open range for a local calendar day.
pub fn day_bounds(date: Date) -> Option<(i64, i64)> {
    day_bounds_in(date, &zone())
}

/// [`day_bounds`] in `zone`. A day starts at its first instant, so one whose
/// midnight a clock change skips starts when the clock resumes, and the
/// range is 23 or 25 hours long around a change.
fn day_bounds_in(date: Date, zone: &jiff::tz::TimeZone) -> Option<(i64, i64)> {
    let start = date.to_zoned(zone.clone()).ok()?.start_of_day().ok()?;
    let end = date
        .tomorrow()
        .ok()?
        .to_zoned(zone.clone())
        .ok()?
        .start_of_day()
        .ok()?;
    Some((start.timestamp().as_second(), end.timestamp().as_second()))
}

fn weekday_name(locale: Locale, weekday: jiff::civil::Weekday) -> String {
    use crate::i18n::gettext;
    // Each literal sits in its own call so xgettext can extract it.
    match weekday {
        jiff::civil::Weekday::Monday => gettext(locale, "Monday"),
        jiff::civil::Weekday::Tuesday => gettext(locale, "Tuesday"),
        jiff::civil::Weekday::Wednesday => gettext(locale, "Wednesday"),
        jiff::civil::Weekday::Thursday => gettext(locale, "Thursday"),
        jiff::civil::Weekday::Friday => gettext(locale, "Friday"),
        jiff::civil::Weekday::Saturday => gettext(locale, "Saturday"),
        jiff::civil::Weekday::Sunday => gettext(locale, "Sunday"),
    }
    .into_owned()
}

fn month_name(locale: Locale, month: i8) -> String {
    use crate::i18n::gettext;
    match month {
        1 => gettext(locale, "January"),
        2 => gettext(locale, "February"),
        3 => gettext(locale, "March"),
        4 => gettext(locale, "April"),
        5 => gettext(locale, "May"),
        6 => gettext(locale, "June"),
        7 => gettext(locale, "July"),
        8 => gettext(locale, "August"),
        9 => gettext(locale, "September"),
        10 => gettext(locale, "October"),
        11 => gettext(locale, "November"),
        _ => gettext(locale, "December"),
    }
    .into_owned()
}

/// Returns a date-context month name, allowing language-specific grammatical forms.
fn date_month_name(locale: Locale, month: i8) -> String {
    use crate::i18n::pgettext;
    match month {
        1 => pgettext(locale, "date month", "January"),
        2 => pgettext(locale, "date month", "February"),
        3 => pgettext(locale, "date month", "March"),
        4 => pgettext(locale, "date month", "April"),
        5 => pgettext(locale, "date month", "May"),
        6 => pgettext(locale, "date month", "June"),
        7 => pgettext(locale, "date month", "July"),
        8 => pgettext(locale, "date month", "August"),
        9 => pgettext(locale, "date month", "September"),
        10 => pgettext(locale, "date month", "October"),
        11 => pgettext(locale, "date month", "November"),
        _ => pgettext(locale, "date month", "December"),
    }
    .into_owned()
}

/// A month's name and year, for the day filter's header.
pub fn month_heading(locale: Locale, month: Date) -> String {
    format!("{} {}", month_name(locale, month.month()), month.year())
}

/// The two-letter weekday headings, Monday first, for the day filter's grid.
/// Taken by character, so a name that is not ASCII is not split mid-glyph.
pub fn weekday_headings(locale: Locale) -> [String; 7] {
    use jiff::civil::Weekday;
    [
        Weekday::Monday,
        Weekday::Tuesday,
        Weekday::Wednesday,
        Weekday::Thursday,
        Weekday::Friday,
        Weekday::Saturday,
        Weekday::Sunday,
    ]
    .map(|weekday| weekday_name(locale, weekday).chars().take(2).collect())
}

/// A date as "23 Sep 2026".
pub fn short_date(locale: Locale, date: Date) -> String {
    let month: String = month_name(locale, date.month()).chars().take(3).collect();
    format!("{} {month} {}", date.day(), date.year())
}

/// A date with its weekday, as "Wednesday, 23 September 2026".
pub fn long_date(locale: Locale, date: Date) -> String {
    format!(
        "{}, {} {} {}",
        weekday_name(locale, date.weekday()),
        date.day(),
        month_name(locale, date.month()),
        date.year()
    )
}

/// The current time as a Unix timestamp.
pub fn now() -> i64 {
    #[cfg(test)]
    if let Some(clock) = fixed_clock::get() {
        return clock.now;
    }
    Timestamp::now().as_second()
}

/// A clock for tests whose layout depends on the time: the current time, the
/// time zone, and the clock format, fixed for the calling thread until the
/// returned guard drops. Otherwise a sample built relative to now gains a day
/// separator or wider times depending on when and where the test runs.
#[cfg(test)]
pub mod fixed_clock {
    use std::cell::RefCell;

    #[derive(Clone)]
    pub struct Clock {
        pub now: i64,
        pub zone: jiff::tz::TimeZone,
        pub twelve_hour: bool,
    }

    thread_local! {
        static CLOCK: RefCell<Option<Clock>> = const { RefCell::new(None) };
    }

    pub(super) fn get() -> Option<Clock> {
        CLOCK.with(|clock| clock.borrow().clone())
    }

    /// Restores the thread's previous clock when dropped.
    #[must_use]
    pub struct Guard(Option<Clock>);

    impl Drop for Guard {
        fn drop(&mut self) {
            CLOCK.with(|clock| *clock.borrow_mut() = self.0.take());
        }
    }

    pub fn set(clock: Clock) -> Guard {
        Guard(CLOCK.with(|slot| slot.borrow_mut().replace(clock)))
    }
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

/// Formats a phone number with a plus sign and country-appropriate grouping.
pub fn phone(digits: &str) -> String {
    let digits: String = digits.chars().filter(char::is_ascii_digit).collect();
    if digits.is_empty() {
        return String::new();
    }
    if let Some(formatted) = brazilian_phone(&digits) {
        return formatted;
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

/// Formats Brazil's `+55` numbers as `(DDD) XXXX-XXXX` or `(DDD) XXXXX-XXXX`.
///
/// WhatsApp stores direct-chat ids in international form, while Brazilian
/// numbers use a two-digit area code and either eight-digit fixed lines or
/// nine-digit mobile numbers.
fn brazilian_phone(digits: &str) -> Option<String> {
    let national = digits.strip_prefix("55")?;
    let subscriber_len = match national.len() {
        10 | 11 => national.len() - 2,
        _ => return None,
    };
    let area = &national[..2];
    let subscriber = &national[2..2 + subscriber_len];
    let split = subscriber.len() - 4;
    Some(format!(
        "+55 ({area}) {}-{}",
        &subscriber[..split],
        &subscriber[split..]
    ))
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
/// The same mark without its rim and shading, which blur below this size.
const SMALL_MARK: &[u8] = include_bytes!("../packaging/icons/zapfast-small.svg");
const SMALL_BELOW: usize = 40;

/// Rasterizes the logo to straight-alpha RGBA.
pub fn app_icon_rgba(size: usize) -> Vec<u8> {
    let mark = if size < SMALL_BELOW { SMALL_MARK } else { MARK };
    match render_mark(mark, size) {
        Some(rgba) => rgba,
        // Fall back to an accent disc if the embedded SVG cannot render.
        None => plain_disc(size),
    }
}

fn render_mark(mark: &[u8], size: usize) -> Option<Vec<u8>> {
    let side = size.max(1) as u32;
    let tree = resvg::usvg::Tree::from_data(mark, &resvg::usvg::Options::default()).ok()?;
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
            .collect(),
    )
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

/// Converts the logo to a monochrome macOS menu-bar template: the disc,
/// with the bubble cut out of it.
pub fn tray_template_rgba(size: usize) -> Vec<u8> {
    // The flat mark at every size: a template has no room for shading.
    let mut rgba = render_mark(SMALL_MARK, size).unwrap_or_else(|| plain_disc(size));
    // The disc's green against the ink's says how much of a pixel is disc.
    const INK: f32 = 14.0;
    const DISC: f32 = 168.0;
    for pixel in rgba.as_chunks_mut::<4>().0 {
        let disc = ((f32::from(pixel[1]) - INK) / (DISC - INK)).clamp(0.0, 1.0);
        pixel[3] = (f32::from(pixel[3]) * disc).round() as u8;
        pixel[0] = 0;
        pixel[1] = 0;
        pixel[2] = 0;
    }
    rgba
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_contact_editor_keeps_names_as_they_were_saved() {
        let names = |name, first| super::editor_names(name, first);
        let pair = |first: &str, last: &str| (first.to_owned(), last.to_owned());
        // A first name of several words, alone or with a last name (#314).
        assert_eq!(
            names("first second", Some("first second")),
            pair("first second", "")
        );
        assert_eq!(
            names("first second third", Some("first second")),
            pair("first second", "third")
        );
        assert_eq!(names("My Dih", Some("My Dih")), pair("My Dih", ""));
        // A first name the full name merely starts with, letter for letter,
        // is not the first word.
        assert_eq!(names("Mary Ann", Some("Mar")), pair("Mary Ann", ""));
        // Without a first name nothing is guessed at a space.
        assert_eq!(names(" Mary Ann Evans ", None), pair("Mary Ann Evans", ""));
        assert_eq!(names("Bob", Some("  ")), pair("Bob", ""));
        // Names written without spaces stay whole.
        assert_eq!(names("山田太郎", None), pair("山田太郎", ""));
        assert_eq!(names("山田太郎", Some("太郎")), pair("山田太郎", ""));
        assert_eq!(names("محمد علي", Some("محمد")), pair("محمد", "علي"));
    }

    #[test]
    fn a_day_filter_covers_the_local_day_across_clock_changes() {
        use jiff::civil::date;
        use jiff::tz::TimeZone;
        let second = |text: &str| {
            text.parse::<jiff::Timestamp>()
                .expect("a timestamp")
                .as_second()
        };
        // POSIX rules, so the test needs no time zone database.
        let berlin = TimeZone::posix("CET-1CEST,M3.5.0,M10.5.0/3").expect("a zone");
        assert_eq!(
            super::day_bounds_in(date(2026, 9, 23), &berlin),
            Some((
                second("2026-09-22T22:00:00Z"),
                second("2026-09-23T22:00:00Z")
            )),
            "an ordinary day runs from local midnight to local midnight"
        );
        let (from, until) = super::day_bounds_in(date(2026, 3, 29), &berlin).expect("a range");
        assert_eq!(until - from, 23 * 3_600, "spring forward loses an hour");
        let (from, until) = super::day_bounds_in(date(2026, 10, 25), &berlin).expect("a range");
        assert_eq!(until - from, 25 * 3_600, "fall back gains one");
        // Brazil once sprang forward at midnight: that day began at 01:00.
        let brasilia = TimeZone::posix("<-03>3<-02>,M11.1.0/0,M2.3.0/0").expect("a zone");
        assert_eq!(
            super::day_bounds_in(date(2018, 11, 4), &brasilia),
            Some((
                second("2018-11-04T03:00:00Z"),
                second("2018-11-05T02:00:00Z")
            ))
        );
    }

    #[test]
    fn clock_patterns_from_every_platform_are_recognized() {
        for pattern in [
            "h:mm:ss tt",
            "h a",
            "K:mm a",
            "%r",
            "%I:%M:%S %p",
            "%l:%M %p",
        ] {
            assert!(super::pattern_is_twelve_hour(pattern), "{pattern}");
        }
        for pattern in ["HH:mm:ss", "H:mm", "%T", "%H:%M:%S", "HH 'h' mm", "HH"] {
            assert!(!super::pattern_is_twelve_hour(pattern), "{pattern}");
        }
    }

    #[test]
    fn time_of_day_switches_between_24_and_12_hour() {
        assert_eq!(time_of_day(0, 5, false), "00:05");
        assert_eq!(time_of_day(14, 5, false), "14:05");
        assert_eq!(time_of_day(0, 5, true), "12:05 AM");
        assert_eq!(time_of_day(9, 5, true), "9:05 AM");
        assert_eq!(time_of_day(12, 0, true), "12:00 PM");
        assert_eq!(time_of_day(14, 5, true), "2:05 PM");
        assert_eq!(time_of_day(23, 59, true), "11:59 PM");
    }

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
    /// Checks recent relative day labels include the calendar date while Today stays concise.
    fn recent_day_labels_include_the_calendar_date_except_today() {
        let today = Date::new(2026, 9, 30).unwrap();
        for (day, expected) in [
            (30, "Today"),
            (29, "Yesterday (September 29th)"),
            (26, "Saturday (September 26th)"),
            (23, "Wednesday, 23 September 2026"),
        ] {
            assert_eq!(
                day_label_relative_to(Locale::English, Date::new(2026, 9, day).unwrap(), today),
                expected
            );
        }
        assert_eq!(
            day_label_relative_to(
                Locale::English,
                Date::new(2026, 12, 31).unwrap(),
                Date::new(2027, 1, 1).unwrap()
            ),
            "Yesterday (December 31st, 2026)"
        );
        assert_eq!(day_label(Locale::English, i64::MAX), "");
    }

    #[test]
    /// Checks English calendar dates include the correct ordinal suffixes, including teens.
    fn english_calendar_dates_handle_ordinal_endings() {
        for (day, suffix) in [
            (1, "st"),
            (2, "nd"),
            (3, "rd"),
            (11, "th"),
            (12, "th"),
            (13, "th"),
            (21, "st"),
            (22, "nd"),
            (23, "rd"),
            (31, "st"),
        ] {
            let date = Date::new(2026, 8, day).unwrap();
            assert_eq!(
                day_label_relative_to(Locale::English, date, date.tomorrow().unwrap()),
                format!("Yesterday (August {day}{suffix})")
            );
        }
    }

    #[test]
    /// Checks relative date templates and month names use each tested locale's grammar.
    fn translated_recent_dates_use_local_grammar() {
        let date = Date::new(2026, 9, 29).unwrap();
        for (locale, expected) in [
            (Locale::PortugueseBrazil, "Ontem (29 de setembro)"),
            (Locale::Spanish, "Ayer (29 de septiembre)"),
            (Locale::Russian, "Вчера (29 сентября)"),
            (Locale::ChineseSimplified, "昨天（9月29日）"),
            (Locale::Turkish, "Dün (29 Eylül)"),
        ] {
            assert_eq!(
                day_label_relative_to(locale, date, date.tomorrow().unwrap()),
                expected
            );
        }
    }

    #[test]
    fn initials_take_first_and_last_word() {
        assert_eq!(initials("Ada Lovelace"), "AL");
        assert_eq!(initials("ada"), "A");
        assert_eq!(initials("  "), "#");
        assert_eq!(initials("🎉 Party Planning"), "PP");
    }

    #[test]
    fn phone_numbers_are_grouped() {
        assert_eq!(phone("393331234567"), "+39 333 123 456 7");
        assert_eq!(phone("15551234567"), "+15 551 234 567");
        assert_eq!(phone("5511999999999"), "+55 (11) 99999-9999");
        assert_eq!(phone("551140028922"), "+55 (11) 4002-8922");
        assert_eq!(phone("551149508333"), "+55 (11) 4950-8333");
        assert_eq!(phone("+55 (11) 99999-9999"), "+55 (11) 99999-9999");
        assert_eq!(phone(""), "");
    }

    #[test]
    fn stamps_fall_back_to_dates() {
        let when = Timestamp::from_second(1_700_000_000)
            .expect("valid")
            .to_zoned(jiff::tz::TimeZone::UTC);
        let date = when.date();
        assert_eq!(
            stamp_relative_to(Locale::English, date, date, &when),
            time_of_day(22, 13, twelve_hour_clock())
        );
        assert_eq!(
            stamp_relative_to(Locale::English, date, date.tomorrow().expect("date"), &when),
            "Yesterday"
        );
        assert_eq!(
            stamp_relative_to(
                Locale::English,
                date,
                date.checked_add(jiff::Span::new().days(3)).expect("date"),
                &when
            ),
            "Tuesday"
        );
        assert_eq!(
            stamp_relative_to(
                Locale::English,
                date,
                date.checked_add(jiff::Span::new().days(30)).expect("date"),
                &when
            ),
            "14 Nov 2023"
        );
    }

    /// Last seen reads as on the phone: the time today and yesterday, the
    /// weekday and time within the week, and the date after that.
    #[test]
    fn last_seen_names_the_day_and_the_time() {
        // Tuesday 14 November 2023, 22:13 UTC.
        let when = Timestamp::from_second(1_700_000_000)
            .expect("valid")
            .to_zoned(jiff::tz::TimeZone::UTC);
        let date = when.date();
        let time = time_of_day(22, 13, twelve_hour_clock());
        let later = |days: i64| {
            date.checked_add(jiff::Span::new().days(days))
                .expect("date")
        };
        let english = |today| last_seen_relative_to(Locale::English, today, &when);
        assert_eq!(english(date), format!("last seen today at {time}"));
        assert_eq!(
            english(date.yesterday().expect("date")),
            format!("last seen today at {time}"),
            "a timestamp slightly ahead of our clock is still today"
        );
        assert_eq!(english(later(1)), format!("last seen yesterday at {time}"));
        assert_eq!(english(later(3)), format!("last seen Tuesday at {time}"));
        assert_eq!(english(later(6)), format!("last seen Tuesday at {time}"));
        assert_eq!(english(later(7)), "last seen 14 Nov 2023");
        assert_eq!(english(later(30)), "last seen 14 Nov 2023");

        // Mid-sentence weekdays keep their capital only where the language
        // writes one.
        assert_eq!(
            last_seen_relative_to(Locale::Italian, later(3), &when),
            format!("ultimo accesso martedì alle {time}")
        );
        assert_eq!(
            last_seen_relative_to(Locale::German, later(3), &when),
            format!("zuletzt online am Dienstag um {time}")
        );
        for locale in Locale::ALL {
            for days in [0, 1, 3, 30] {
                let line = last_seen_relative_to(locale, later(days), &when);
                assert!(!line.contains('{'), "{locale:?} {days}: {line}");
            }
        }
    }

    #[test]
    fn short_dates_take_whole_characters_in_every_locale() {
        for locale in Locale::ALL {
            for month in 1..=12 {
                let date = jiff::civil::date(2024, month, 5);
                let short = short_date(locale, date);
                assert!(
                    short.starts_with("5 ") && short.ends_with(" 2024"),
                    "{short}"
                );
            }
        }
        assert_eq!(
            short_date(Locale::German, jiff::civil::date(2024, 3, 5)),
            "5 Mär 2024"
        );
    }

    #[test]
    fn brazilian_portuguese_stamps_are_translated() {
        let when = Timestamp::from_second(1_700_000_000)
            .expect("valid")
            .to_zoned(jiff::tz::TimeZone::UTC);
        let date = when.date();
        assert_eq!(
            stamp_relative_to(
                Locale::PortugueseBrazil,
                date,
                date.tomorrow().expect("date"),
                &when
            ),
            "Ontem"
        );
        assert_eq!(
            stamp_relative_to(
                Locale::PortugueseBrazil,
                date,
                date.checked_add(jiff::Span::new().days(3)).expect("date"),
                &when
            ),
            "Terça-feira"
        );
    }

    #[test]
    fn turkish_stamps_are_translated() {
        let when = Timestamp::from_second(1_700_000_000)
            .expect("valid")
            .to_zoned(jiff::tz::TimeZone::UTC);
        let date = when.date();
        assert_eq!(
            stamp_relative_to(Locale::Turkish, date, date.tomorrow().expect("date"), &when),
            "Dün"
        );
        assert_eq!(
            stamp_relative_to(
                Locale::Turkish,
                date,
                date.checked_add(jiff::Span::new().days(3)).expect("date"),
                &when
            ),
            "Salı"
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

    /// The menu-bar template is the disc with the bubble cut out: solid in
    /// the bubble's middle, clear along its outline and outside the disc.
    #[test]
    fn the_tray_template_cuts_the_bubble_out_of_the_disc() {
        let size = 64;
        let template = tray_template_rgba(size);
        let alpha = |x: usize, y: usize| template[(y * size + x) * 4 + 3];
        assert_eq!(alpha(0, 0), 0);
        assert_eq!(alpha(32, 32), 255, "inside the bubble is disc");
        assert_eq!(alpha(32, 4), 255, "so is the rim above it");
        let outline = (0..32).map(|y| alpha(32, y)).min().unwrap();
        assert!(outline < 40, "the outline is cut out: {outline}");
        assert!(
            template
                .as_chunks::<4>()
                .0
                .iter()
                .all(|p| p[..3] == [0, 0, 0])
        );
    }

    /// Both marks render, the large one with its filters.
    #[test]
    fn both_marks_render() {
        assert!(render_mark(MARK, 128).is_some());
        assert!(render_mark(SMALL_MARK, 22).is_some());
    }
}
