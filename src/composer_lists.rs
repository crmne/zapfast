//! List editing keeps the ordinary WhatsApp text buffer and character offsets.
fn prefix(line: &str) -> Option<(&str, &str)> {
    for marker in ["• ", "* ", "- "] {
        if let Some(body) = line.strip_prefix(marker) {
            return Some((marker, body));
        }
    }
    let (number, body) = line.split_once(". ")?;
    if number.is_empty() || !number.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    number.parse::<u32>().ok()?;
    Some((&line[..number.len() + 2], body))
}
fn byte_at(text: &str, cursor: usize) -> usize {
    text.char_indices()
        .nth(cursor)
        .map_or(text.len(), |(byte, _)| byte)
}
pub fn active(text: &str, cursor: usize) -> bool {
    let before = &text[..byte_at(text, cursor)];
    prefix(before.rsplit('\n').next().unwrap_or("")).is_some()
}
/// Called only for a typed space or an inserted newline, never for pasted text.
pub fn edit(text: &mut String, cursor: usize, space: bool, newline: bool) -> Option<usize> {
    let end = byte_at(text, cursor);
    let before = &text[..end];
    if space {
        let start = before.rfind('\n').map_or(0, |i| i + 1);
        let line = &before[start..];
        let replacement = if matches!(line, "* " | "- ") {
            Some("• ".to_owned())
        } else if let Some(number) = line.strip_suffix(' ') {
            (!number.is_empty()
                && number.bytes().all(|b| b.is_ascii_digit())
                && number.parse::<u32>().is_ok())
            .then(|| format!("{number}. "))
        } else {
            None
        };
        if let Some(replacement) = replacement {
            let position = text[..start].chars().count() + replacement.chars().count();
            text.replace_range(start..end, &replacement);
            return Some(position);
        }
    }
    if newline && before.ends_with('\n') {
        let previous_end = end - 1;
        let start = text[..previous_end].rfind('\n').map_or(0, |i| i + 1);
        let (marker, body) = prefix(&text[start..previous_end])?;
        if body.trim().is_empty() {
            let position = text[..start].chars().count();
            text.replace_range(start..end, "");
            return Some(position);
        }
        let next = if let Some(number) = marker.strip_suffix(". ") {
            format!("{}. ", number.parse::<u32>().ok()?.checked_add(1)?)
        } else {
            marker.to_owned()
        };
        text.insert_str(end, &next);
        return Some(cursor + next.chars().count());
    }
    None
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn typing_starts_continues_and_exits_lists() {
        for marker in ["* ", "- "] {
            let mut text = marker.to_owned();
            assert_eq!(edit(&mut text, 2, true, false), Some(2));
            assert_eq!(text, "• ");
        }
        let mut text = "1 ".to_owned();
        assert_eq!(edit(&mut text, 2, true, false), Some(3));
        assert_eq!(text, "1. ");
        let mut text = "9. hello\n".to_owned();
        let cursor = text.chars().count();
        edit(&mut text, cursor, false, true);
        assert_eq!(text, "9. hello\n10. ");
        text.push('\n');
        let cursor = text.chars().count();
        edit(&mut text, cursor, false, true);
        assert_eq!(text, "9. hello\n");
    }
    #[test]
    fn unicode_offsets_mid_document_and_ordinary_prose_are_preserved() {
        let mut text = "你好\n- \nlast".to_owned();
        assert_eq!(edit(&mut text, 5, true, false), Some(5));
        assert_eq!(text, "你好\n• \nlast");
        let mut text = "cost - ".to_owned();
        assert_eq!(edit(&mut text, 7, true, false), None);
        let mut text = "* ".to_owned();
        assert_eq!(edit(&mut text, 2, false, false), None);
    }
}
