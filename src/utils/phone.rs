pub fn international_phone(input: &str) -> Option<String> {
    let input = input.trim();
    let input = input.strip_prefix('+').unwrap_or(input);
    let mut digits = String::new();
    let mut bracket = false;
    for character in input.chars() {
        match character {
            '0'..='9' => digits.push(character),
            ' ' | '-' => {}
            '(' if !bracket => bracket = true,
            ')' if bracket => bracket = false,
            _ => return None,
        }
    }
    (!bracket && (7..=15).contains(&digits.len()) && !digits.starts_with('0')).then_some(digits)
}

pub fn phone(digits: &str) -> String {
    let digits: String = digits.chars().filter(char::is_ascii_digit).collect();
    if digits.is_empty() {
        return String::new();
    }
    if let Some(formatted) = brazilian_phone(&digits).or_else(|| french_phone(&digits)) {
        return formatted;
    }
    let (country, national) = digits.split_at(country_code_len(&digits).min(digits.len()));
    let mut out = format!("+{country}");
    let mut start = 0;
    while start < national.len() {
        let end = match national.len() - start {
            4 => national.len(),
            _ => (start + 3).min(national.len()),
        };
        out.push(' ');
        out.push_str(&national[start..end]);
        start = end;
    }
    out
}

fn country_code_len(digits: &str) -> usize {
    const TWO_DIGITS: [&str; 44] = [
        "20", "27", "30", "31", "32", "33", "34", "36", "39", "40", "41", "43", "44", "45", "46",
        "47", "48", "49", "51", "52", "53", "54", "55", "56", "57", "58", "60", "61", "62", "63",
        "64", "65", "66", "81", "82", "84", "86", "90", "91", "92", "93", "94", "95", "98",
    ];
    if digits.starts_with(['1', '7']) {
        1
    } else if TWO_DIGITS.iter().any(|code| digits.starts_with(code)) {
        2
    } else {
        3
    }
}

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

fn french_phone(digits: &str) -> Option<String> {
    let (country, national) = ["33", "262", "590", "594", "596"]
        .into_iter()
        .find_map(|code| Some((code, digits.strip_prefix(code)?)))?;
    if national.len() != 9 {
        return None;
    }
    let lead = if country == "33" && !national.starts_with('8') {
        1
    } else {
        3
    };
    let mut out = format!("+{country} {}", &national[..lead]);
    for start in (lead..national.len()).step_by(2) {
        out.push(' ');
        out.push_str(&national[start..start + 2]);
    }
    Some(out)
}
