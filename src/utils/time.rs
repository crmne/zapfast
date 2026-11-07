use jiff::civil::Date;
use jiff::{Timestamp, Zoned};

use crate::i18n::Locale;

pub fn today() -> Date {
    #[cfg(test)]
    if let Some(when) = fixed_clock::get().and_then(|clock| zoned(clock.now)) {
        return when.date();
    }
    Zoned::now().date()
}

pub fn now() -> i64 {
    #[cfg(test)]
    if let Some(clock) = fixed_clock::get() {
        return clock.now;
    }
    Timestamp::now().as_second()
}

pub fn twelve_hour_clock() -> bool {
    #[cfg(test)]
    if let Some(clock) = fixed_clock::get() {
        return clock.twelve_hour;
    }
    static TWELVE_HOUR: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *TWELVE_HOUR.get_or_init(clock_preference::twelve_hour)
}

fn zone() -> jiff::tz::TimeZone {
    #[cfg(test)]
    if let Some(clock) = fixed_clock::get() {
        return clock.zone;
    }
    jiff::tz::TimeZone::system()
}

pub fn zoned(unix_seconds: i64) -> Option<Zoned> {
    let timestamp = Timestamp::from_second(unix_seconds).ok()?;
    Some(timestamp.to_zoned(zone()))
}

fn pattern_is_twelve_hour(pattern: &str) -> bool {
    if pattern.contains('%') {
        return ["%I", "%l", "%r", "%p"]
            .iter()
            .any(|field| pattern.contains(field));
    }
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

    fn gnome() -> Option<bool> {
        if !fastframe_desktop::is_gnome() {
            return None;
        }
        clock_format(
            fastframe_desktop::setting("org.gnome.desktop.interface", "clock-format").as_deref(),
        )
    }

    pub(super) fn clock_format(value: Option<&str>) -> Option<bool> {
        match value? {
            "12h" => Some(true),
            "24h" => Some(false),
            _ => None,
        }
    }

    fn locale() -> bool {
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

    pub fn twelve_hour() -> bool {
        let mut buffer = [0u16; 80];
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

pub fn hour_minute(when: &Zoned) -> String {
    time_of_day(when.hour(), when.minute(), twelve_hour_clock())
}

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

pub fn clock(unix_seconds: i64) -> String {
    zoned(unix_seconds)
        .map(|when| hour_minute(&when))
        .unwrap_or_default()
}

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

pub fn disappearing_line(locale: Locale, seconds: Option<u32>) -> Option<String> {
    let seconds = seconds?;
    let label = crate::i18n::gettext(locale, "Disappearing messages");
    Some(format!("{label}: {}", disappearing_value(locale, seconds)))
}

pub fn disappearing_value(locale: Locale, seconds: u32) -> String {
    use crate::i18n::gettext;
    match seconds {
        0 => gettext(locale, "Off").into_owned(),
        86_400 => gettext(locale, "24 hours").into_owned(),
        604_800 => gettext(locale, "7 days").into_owned(),
        7_776_000 => gettext(locale, "90 days").into_owned(),
        _ => format!("{seconds}s"),
    }
}

pub fn disappearing_change(locale: Locale, name: &str, seconds: u32) -> String {
    use crate::i18n::gettext;
    if seconds == 0 {
        gettext(locale, "{name} turned off disappearing messages").replace("{name}", name)
    } else {
        gettext(
            locale,
            "{name} changed the disappearing messages setting to {duration}",
        )
        .replace("{name}", name)
        .replace("{duration}", &disappearing_value(locale, seconds))
    }
}

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
        ..=0 => gettext(locale, "last seen today at {time}").replace("{time}", &time),
        1 => gettext(locale, "last seen yesterday at {time}").replace("{time}", &time),
        2..=6 => {
            let weekday = weekday_name(locale, date.weekday());
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

pub fn day_label(locale: Locale, unix_seconds: i64) -> String {
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
        0 => crate::i18n::gettext(locale, "Today").into_owned(),
        1 => crate::i18n::gettext(locale, "Yesterday").into_owned(),
        2..=6 => weekday_name(locale, date.weekday()),
        _ => long_date(locale, date),
    }
}

pub fn day_key(unix_seconds: i64) -> Option<Date> {
    zoned(unix_seconds).map(|when| when.date())
}

pub fn day_bounds(date: Date) -> Option<(i64, i64)> {
    day_bounds_in(date, &zone())
}

pub fn day_bounds_in(date: Date, zone: &jiff::tz::TimeZone) -> Option<(i64, i64)> {
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

pub fn month_heading(locale: Locale, month: Date) -> String {
    format!("{} {}", month_name(locale, month.month()), month.year())
}

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

pub fn short_date(locale: Locale, date: Date) -> String {
    let month: String = month_name(locale, date.month()).chars().take(3).collect();
    format!("{} {month} {}", date.day(), date.year())
}

pub fn long_date(locale: Locale, date: Date) -> String {
    format!(
        "{}, {} {} {}",
        weekday_name(locale, date.weekday()),
        date.day(),
        month_name(locale, date.month()),
        date.year()
    )
}

pub fn call_length(locale: Locale, seconds: u64) -> String {
    use crate::i18n::gettext;
    let (hours, minutes, seconds) = (seconds / 3600, seconds % 3600 / 60, seconds % 60);
    let text = match (hours, minutes, seconds) {
        (0, 0, _) => gettext(locale, "{seconds} s"),
        (0, _, 0) => gettext(locale, "{minutes} min"),
        (0, _, _) => gettext(locale, "{minutes} min {seconds} s"),
        (_, 0, _) => gettext(locale, "{hours} h"),
        _ => gettext(locale, "{hours} h {minutes} min"),
    };
    text.replace("{hours}", &hours.to_string())
        .replace("{minutes}", &minutes.to_string())
        .replace("{seconds}", &seconds.to_string())
}

pub fn call_duration(seconds: u64) -> String {
    let (hours, minutes, seconds) = (seconds / 3600, seconds % 3600 / 60, seconds % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

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
