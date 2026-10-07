//! Safe typed-emoticon conversion for the message composer.

#[cfg(test)]
mod tests {
    use super::{convert_typed_emoticons, match_before};

    #[test]
    fn cursor_match_requires_whitespace_boundaries_and_skips_code() {
        let matched = match_before("hi :)", 5).expect("standalone face");
        assert_eq!((&"hi :)"[matched.0..matched.1], matched.2), (&":)", "😊"));
        assert!(match_before("x:)", 3).is_none());
        assert!(
            match_before(
                "https://example.com/:)",
                "https://example.com/:)".chars().count()
            )
            .is_none()
        );
        assert!(match_before("😀:)", "😀:)".chars().count()).is_none());
        assert!(match_before("`:)`", 3).is_none());
    }

    #[test]
    fn converts_only_whitespace_or_boundary_delimited_emoticons() {
        assert_eq!(convert_typed_emoticons(":)"), "😊");
        assert_eq!(convert_typed_emoticons("hello :) there"), "hello 😊 there");
        assert_eq!(convert_typed_emoticons("x:)y"), "x:)y");
        assert_eq!(convert_typed_emoticons("(:)"), "(:)");
    }

    #[test]
    fn skips_urls_and_inline_code() {
        assert_eq!(
            convert_typed_emoticons("https://example.com/:) :)"),
            "https://example.com/:) 😊"
        );
        assert_eq!(convert_typed_emoticons("`:)` :)"), "`:)` 😊");
    }

    #[test]
    fn recognizes_common_faces() {
        assert_eq!(convert_typed_emoticons(":( ;) :D :P <3"), "😞 😉 😄 😛 ❤️");
    }
}

/// The supported emoticon ending exactly at `cursor`, with its byte range and emoji.
pub fn match_before(text: &str, cursor: usize) -> Option<(usize, usize, &'static str)> {
    let end = text
        .char_indices()
        .nth(cursor)
        .map_or(text.len(), |(byte, _)| byte);
    let prefix = text.get(..end)?;
    if inside_code_span(text, end) {
        return None;
    }
    for (token, emoji) in TOKENS {
        let Some(before_token) = prefix.strip_suffix(token) else {
            continue;
        };
        let start = before_token.len();
        let before = before_token.chars().next_back();
        let after = text[end..].chars().next();
        if before.is_none_or(char::is_whitespace) && after.is_none_or(char::is_whitespace) {
            return Some((start, end, emoji));
        }
    }
    None
}

fn inside_code_span(text: &str, before: usize) -> bool {
    let mut open_ticks = None;
    let mut at = 0;
    while at < before {
        let ch = text[at..].chars().next().expect("valid char boundary");
        if ch != '`' {
            at += ch.len_utf8();
            continue;
        }
        let slashes = text[..at]
            .bytes()
            .rev()
            .take_while(|byte| *byte == b'\\')
            .count();
        let mut end = at;
        while end < before && text[end..].starts_with('`') {
            end += 1;
        }
        let run = end - at;
        if slashes % 2 == 0 {
            match open_ticks {
                Some(open) if open == run => open_ticks = None,
                None => open_ticks = Some(run),
                _ => {}
            }
        }
        at = end;
    }
    open_ticks.is_some()
}

/// Replace supported plain-text emoticons when isolated by composer boundaries or whitespace.
pub fn convert_typed_emoticons(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut i = 0;
    let mut in_code = false;
    while i < text.len() {
        let ch = text[i..].chars().next().expect("valid char boundary");
        if ch == '`' {
            in_code = !in_code;
            result.push(ch);
            i += ch.len_utf8();
            continue;
        }
        if !in_code {
            if let Some(end) = url_end(text, i) {
                result.push_str(&text[i..end]);
                i = end;
                continue;
            }
            let previous = text[..i].chars().next_back();
            if previous.is_none_or(char::is_whitespace) {
                if let Some((token, emoji)) = TOKENS
                    .iter()
                    .find(|(token, _)| text[i..].starts_with(token))
                {
                    let end = i + token.len();
                    let next = text[end..].chars().next();
                    if next.is_none_or(char::is_whitespace) {
                        result.push_str(emoji);
                        i = end;
                        continue;
                    }
                }
            }
        }
        result.push(ch);
        i += ch.len_utf8();
    }
    result
}

const TOKENS: &[(&str, &str)] = &[
    (":-)", "😊"),
    (":)", "😊"),
    (":-D", "😄"),
    (":D", "😄"),
    (":-(", "😞"),
    (":(", "😞"),
    (";-)", "😉"),
    (";)", "😉"),
    (":-P", "😛"),
    (":P", "😛"),
    ("<3", "❤️"),
];

fn url_end(text: &str, start: usize) -> Option<usize> {
    let rest = &text[start..];
    let scheme = rest.find("://")?;
    let prefix = &rest[..scheme];
    if prefix.is_empty()
        || !prefix
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
    {
        return None;
    }
    let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
    Some(start + end)
}
