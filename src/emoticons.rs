//! Safe typed-emoticon conversion for the message composer.

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

#[cfg(test)]
mod tests {
    use super::{TOKENS, match_before};

    #[test]
    fn cursor_match_requires_whitespace_boundaries_and_skips_code() {
        let matched = match_before("hi :)", 5).expect("standalone face");
        assert_eq!((&"hi :)"[matched.0..matched.1], matched.2), (":)", "😊"));
        assert!(match_before("x:)", 3).is_none());
        assert!(match_before("😀:)", "😀:)".chars().count()).is_none());
        assert!(match_before("`:)`", 3).is_none());
        assert!(match_before("``:)``", 4).is_none());
        assert!(match_before(r"\` :)", 5).is_some());
    }

    #[test]
    fn recognizes_every_supported_emoticon() {
        for &(token, emoji) in TOKENS {
            let (start, end, matched) = match_before(token, token.chars().count())
                .unwrap_or_else(|| panic!("{token} should match"));
            assert_eq!(&token[start..end], token);
            assert_eq!(matched, emoji);
        }
    }
}
