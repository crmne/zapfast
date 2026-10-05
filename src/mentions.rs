//! Mentioning everyone in a group.
//!
//! The message body keeps the visible token `@all`. A localized word, such
//! as `@todos`, is only a way to type it: the composer rewrites that word to
//! `@all` when the sender is allowed to mention everyone. Bit 1 of
//! `ContextInfo.non_jid_mentions` is what notifies the group. The stored
//! mention id is [`ALL_ID`], which is not a JID.

use crate::i18n::{self, Locale};
use crate::model::MentionRef;

/// Visible token, without the `@`. Case-sensitive on the wire.
pub const ALL_USER: &str = "all";
/// Stored mention id. It has no `@`, so it is never parsed as a chat.
pub const ALL_ID: &str = "mention-all";
/// `non_jid_mentions` bit that notifies everyone.
pub const ALL_FLAG: u32 = 1;
/// At this size and below, every member who can post may mention everyone.
pub const OPEN_LIMIT: usize = 32;

/// The mention row stored for `@all`.
pub fn everyone_ref() -> MentionRef {
    MentionRef {
        user: ALL_USER.to_owned(),
        id: ALL_ID.to_owned(),
    }
}

pub fn is_everyone(mention: &MentionRef) -> bool {
    mention.id == ALL_ID
}

/// Appends the everyone mention once.
pub fn push_everyone(mentions: &mut Vec<MentionRef>) {
    if !mentions.iter().any(is_everyone) {
        mentions.push(everyone_ref());
    }
}

/// Appends [`ALL_ID`] once, beside real participant JIDs.
pub fn push_everyone_id(ids: &mut Vec<String>) {
    if !ids.iter().any(|id| id == ALL_ID) {
        ids.push(ALL_ID.to_owned());
    }
}

pub fn flagged(bits: Option<u32>) -> bool {
    bits.is_some_and(|bits| bits & ALL_FLAG != 0)
}

/// Sets bit 1 without clearing any other bit.
pub fn with_flag(bits: Option<u32>) -> u32 {
    bits.unwrap_or(0) | ALL_FLAG
}

/// Clears bit 1. `None` stays `None`, and a zero result is dropped.
pub fn without_flag(bits: Option<u32>) -> Option<u32> {
    let cleared = bits.unwrap_or(0) & !ALL_FLAG;
    (cleared != 0).then_some(cleared)
}

/// One-word alias for the interface language. English is `all`.
///
/// A translation with a space, or an empty one, is ignored: the composer
/// would otherwise rewrite ordinary words.
pub fn everyone_alias(locale: Locale) -> String {
    // Translators: one word, no spaces. The word after @ that mentions everyone.
    let alias = i18n::pgettext(locale, "mention", "all");
    let alias = alias.trim();
    if alias.is_empty() || alias.chars().any(char::is_whitespace) {
        ALL_USER.to_owned()
    } else {
        alias.to_owned()
    }
}

/// Whether `text` contains the wire token `@all` on its own.
pub fn has_everyone_token(text: &str) -> bool {
    text.char_indices()
        .any(|(at, character)| character == '@' && everyone_token_end(text, at).is_some())
}

/// End byte of an `@all` token whose `@` is at `at`.
pub fn everyone_token_end(text: &str, at: usize) -> Option<usize> {
    token_end(text, at, ALL_USER, false)
}

/// Rewrites each bounded localized alias to `@all`.
///
/// The alias match ignores case (`@Todos` becomes `@all`). The English alias
/// does not rewrite `@ALL`: only the exact wire token counts. Returns whether
/// a real `@all` token is present afterwards.
pub fn rewrite_alias(text: &mut String, alias: &str) -> bool {
    let alias = alias.trim();
    if !alias.is_empty()
        && !alias.eq_ignore_ascii_case(ALL_USER)
        && !alias.chars().any(char::is_whitespace)
    {
        let mut out = String::with_capacity(text.len());
        let mut skip_until = 0;
        for (at, character) in text.char_indices() {
            if at < skip_until {
                continue;
            }
            if character == '@'
                && let Some(end) = token_end(text, at, alias, true)
            {
                out.push_str("@all");
                skip_until = end;
                continue;
            }
            out.push(character);
        }
        *text = out;
    }
    has_everyone_token(text)
}

/// Whether the composer query should offer everyone.
///
/// An empty query matches, and so does a prefix of `all` or of `alias`.
pub fn everyone_query_matches(query: &str, alias: &str) -> bool {
    let query = query.trim().trim_start_matches('@').to_lowercase();
    query.is_empty() || ALL_USER.starts_with(&query) || alias.to_lowercase().starts_with(&query)
}

fn token_end(text: &str, at: usize, token: &str, ignore_case: bool) -> Option<usize> {
    if !text.get(at..)?.starts_with('@') || !boundary_before(text, at) {
        return None;
    }
    let rest = text.get(at + 1..)?;
    let len = if ignore_case {
        starts_with_ignore_case(rest, token)?
    } else if rest.starts_with(token) {
        token.len()
    } else {
        return None;
    };
    let end = at + 1 + len;
    boundary_after(text, end).then_some(end)
}

/// Characters that keep an `@` from starting a mention, matching the wire
/// rule: ASCII letters and digits, plus `_ @ . + -`.
fn boundary_before(text: &str, at: usize) -> bool {
    text[..at].chars().next_back().is_none_or(|prev| {
        !prev.is_ascii_alphanumeric() && !matches!(prev, '_' | '@' | '.' | '+' | '-')
    })
}

/// A following letter, digit, or `_` continues the token, so `@alligator`
/// and `@all_more` stay ordinary text. Other scripts count too, which keeps
/// a localized word from swallowing the next character.
fn boundary_after(text: &str, end: usize) -> bool {
    text[end..]
        .chars()
        .next()
        .is_none_or(|next| next != '_' && !next.is_alphanumeric())
}

fn starts_with_ignore_case(text: &str, alias: &str) -> Option<usize> {
    let mut chars = text.chars();
    let mut bytes = 0;
    for expected in alias.chars() {
        let found = chars.next()?;
        if !same_letter(expected, found) {
            return None;
        }
        bytes += found.len_utf8();
    }
    Some(bytes)
}

fn same_letter(left: char, right: char) -> bool {
    left == right || left.to_lowercase().eq(right.to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wire_token_is_a_whole_word() {
        assert!(has_everyone_token("Hello @all"));
        assert!(has_everyone_token("@all"));
        assert!(has_everyone_token("(@all)"));
        assert!(has_everyone_token("see @all."));
        assert!(!has_everyone_token("@alligator"));
        assert!(!has_everyone_token("mail@all.com"));
        assert!(!has_everyone_token("me@all.com"));
        assert!(!has_everyone_token("@all_"));
        assert!(!has_everyone_token("@all1"));
        assert!(!has_everyone_token(".@all"));
        assert!(!has_everyone_token("@ALL"));
        assert!(!has_everyone_token("@All"));
    }

    #[test]
    fn a_localized_alias_becomes_the_wire_token() {
        let mut text = "oi @todos e @Todos, nao mail@todos.com nem @todosX".to_owned();
        assert!(rewrite_alias(&mut text, "todos"));
        assert_eq!(text, "oi @all e @all, nao mail@todos.com nem @todosX");

        let mut english = "@ALL stays, and so does @todos".to_owned();
        assert!(!rewrite_alias(&mut english, "all"));
        assert_eq!(english, "@ALL stays, and so does @todos");

        let mut already = "ping @all".to_owned();
        assert!(rewrite_alias(&mut already, "todos"));
        assert_eq!(already, "ping @all");
    }

    #[test]
    fn the_picker_query_matches_either_word() {
        assert!(everyone_query_matches("", "todos"));
        assert!(everyone_query_matches("a", "todos"));
        assert!(everyone_query_matches("all", "todos"));
        assert!(everyone_query_matches("t", "todos"));
        assert!(everyone_query_matches("Todos", "todos"));
        assert!(everyone_query_matches("@to", "todos"));
        assert!(!everyone_query_matches("tom", "todos"));
        assert!(!everyone_query_matches("todosa", "todos"));
        assert!(everyone_query_matches("所", "所有人"));
        assert!(!everyone_query_matches("人", "所有人"));
    }

    #[test]
    fn aliases_come_from_the_catalog() {
        assert_eq!(everyone_alias(Locale::English), "all");
        assert_eq!(everyone_alias(Locale::PortugueseBrazil), "todos");
        assert_eq!(everyone_alias(Locale::German), "alle");
        assert_eq!(everyone_alias(Locale::Spanish), "todos");
        assert_eq!(everyone_alias(Locale::Italian), "tutti");
        assert_eq!(everyone_alias(Locale::French), "tous");
        assert_eq!(everyone_alias(Locale::Russian), "все");
        assert_eq!(everyone_alias(Locale::ChineseSimplified), "所有人");
        assert_eq!(everyone_alias(Locale::ChineseTraditional), "所有人");
        assert_eq!(everyone_alias(Locale::Turkish), "herkes");
        assert_eq!(everyone_alias(Locale::Indonesian), "semua");
    }

    #[test]
    fn the_flag_keeps_other_bits() {
        assert_eq!(with_flag(None), 1);
        assert_eq!(with_flag(Some(2)), 3);
        assert_eq!(without_flag(Some(1)), None);
        assert_eq!(without_flag(Some(3)), Some(2));
        assert_eq!(without_flag(None), None);
        assert!(flagged(Some(1)));
        assert!(!flagged(Some(2)));
        assert!(!flagged(None));
    }
}
