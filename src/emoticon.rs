//! Typing a known emoticon in the composer turns it into its emoji.
//!
//! The table follows the Western emoticon families (`:)`, `:-)`, `;)`, `8-D`,
//! `>_<`, `<3`, and their variants) and the emoji each one reads as. UTS #51
//! notes the same idea when it says the emoticon `;-)` can be mapped to 😉.
//!
//! The composer offers this only while a character is being typed, never on
//! paste, and the longest sequence ending at the cursor is the one that
//! converts. An unfinished sequence stays text: typing `:-` is still text,
//! and `:-)` becomes the emoji on the character that finishes it.
//!
//! Spellings that ordinary text uses are deliberately absent. `=3` (as in
//! `x=3`), `o/` (as in `video/`), `DX` (as in `DX11`), `d:` and `x_x` read as
//! prose or code far more often than as a smiley, and a composer that turned
//! them into emoji would eat real words.

/// Emoticon spellings and the emoji they mean.
pub const TABLE: &[(&str, &str)] = &[
    // Smiling.
    (":-)", "😄"),
    (":)", "😄"),
    (":]", "😄"),
    (":^)", "😄"),
    ("=)", "😄"),
    (":3", "😍"),
    ("8)", "😄"),
    ("^_^", "😄"),
    // Grinning.
    (":-D", "😁"),
    (":D", "😁"),
    ("8-D", "😁"),
    ("8D", "😁"),
    ("x-D", "😁"),
    ("X-D", "😁"),
    ("xD", "😁"),
    ("XD", "😁"),
    ("=-D", "😁"),
    ("=D", "😁"),
    // Cool.
    ("8-)", "😎"),
    // Winking.
    (";-)", "😉"),
    (";)", "😉"),
    (";-]", "😉"),
    (";]", "😉"),
    (";^)", "😉"),
    (";D", "😉"),
    ("*-)", "😉"),
    ("*)", "😉"),
    (":-,", "😉"),
    // Kissing.
    (":-*", "😘"),
    (":*", "😘"),
    // Neutral.
    (":-|", "😐"),
    (":|", "😐"),
    // Unamused and sad.
    (":-(", "😒"),
    (":(", "😒"),
    (":-[", "😒"),
    (":[", "😒"),
    (":-<", "😒"),
    (":<", "😒"),
    (":-{", "😒"),
    (":{", "😒"),
    (":-c", "😒"),
    (":c", "😒"),
    (":S", "😡"),
    // Frowning.
    (":-/", "😡"),
    (":/", "😡"),
    (":L", "😡"),
    (":-.", "😡"),
    (":\\", "😡"),
    ("=/", "😡"),
    ("=L", "😡"),
    ("=\\", "😡"),
    // Angry.
    (":-@", "😠"),
    (":@", "😠"),
    (":-||", "😠"),
    (">:(", "😠"),
    (">:-(", "😠"),
    // Laughing through tears.
    (":'-)", "😂"),
    (":')", "😂"),
    // Crying.
    (":'-(", "😢"),
    (":'(", "😢"),
    ("T_T", "😢"),
    // Stunned.
    (":-O", "😲"),
    (":O", "😲"),
    (":-o", "😲"),
    (":o", "😲"),
    ("8-0", "😲"),
    ("O_O", "😲"),
    (">:O", "😲"),
    // Flushed.
    (":$", "😳"),
    // Confounded.
    ("%)", "😖"),
    ("%-)", "😖"),
    // Tongue.
    (":-P", "😜"),
    (":P", "😜"),
    (":p", "😜"),
    (":-p", "😜"),
    (":-b", "😜"),
    ("X-P", "😝"),
    ("XP", "😝"),
    (">:P", "😜"),
    (">:-P", "😜"),
    // Impish.
    (":-))", "😃"),
    (":>)", "😈"),
    (">:)", "😈"),
    (">:-)", "😈"),
    (">:/", "😡"),
    (">:\\", "😡"),
    (">:[", "😒"),
    (">:<", "😒"),
    // Awkward and weary.
    (">_<", "😣"),
    ("-_-", "😑"),
    // Hearts.
    ("<3", "❤️"),
    ("</3", "💔"),
];

/// The longest known emoticon that ends at `cursor`, with the byte range it
/// covers. `cursor` is a character offset, the unit the composer reports.
///
/// A sequence is only a smiley when it stands on its own. A digit in front of
/// it belongs to something else, which is what keeps the `:3` of `12:30` a
/// time; a letter in front is how people write (`oi:-)`), so it is allowed in
/// front of the emoticons that begin with punctuation.
pub fn match_before(text: &str, cursor: usize) -> Option<(usize, usize, &'static str)> {
    let end = text
        .char_indices()
        .nth(cursor)
        .map_or(text.len(), |(at, _)| at);
    let typed = text.get(..end)?;
    let (sequence, emoji) = TABLE
        .iter()
        .filter(|(sequence, _)| typed.ends_with(sequence))
        .max_by_key(|(sequence, _)| sequence.len())?;
    let start = end - sequence.len();
    let own = start == 0
        || typed[..start].chars().next_back().is_none_or(|before| {
            !word(before) || (before.is_alphabetic() && punctuation_leads(sequence))
        });
    own.then_some((start, end, *emoji))
}

/// Whether the character belongs to a word rather than standing between one.
fn word(character: char) -> bool {
    character.is_alphanumeric() || character == '_'
}

/// Whether the spelling opens with punctuation, which is what a letter may
/// precede (`oi:-)`) without the two reading as one word.
fn punctuation_leads(sequence: &str) -> bool {
    sequence.starts_with([':', ';', '=', '<', '>'])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> Option<(usize, usize, &'static str)> {
        match_before(text, text.chars().count())
    }

    #[test]
    fn the_canonical_smileys_become_emoji() {
        assert_eq!(at(":-)").map(|(_, _, e)| e), Some("😄"));
        assert_eq!(at(":)").map(|(_, _, e)| e), Some("😄"));
        assert_eq!(at(";-)").map(|(_, _, e)| e), Some("😉"));
        assert_eq!(at(":-D").map(|(_, _, e)| e), Some("😁"));
        assert_eq!(at("8-)").map(|(_, _, e)| e), Some("😎"));
        assert_eq!(at("8)").map(|(_, _, e)| e), Some("😄"));
        assert_eq!(at("<3").map(|(_, _, e)| e), Some("❤️"));
        assert_eq!(at("</3").map(|(_, _, e)| e), Some("💔"));
    }

    #[test]
    fn a_smiley_after_a_word_still_converts() {
        assert_eq!(at("obrigado :-)").map(|(_, _, e)| e), Some("😄"));
        assert_eq!(at("kkk xD").map(|(_, _, e)| e), Some("😁"));
    }

    #[test]
    fn a_letter_in_front_does_not_swallow_a_word() {
        // `oi:-)` is how people write, `oiXD` is not a smiley.
        assert_eq!(at("oi:-)").map(|(_, _, e)| e), Some("😄"));
        assert_eq!(at("oiXD"), None);
        assert_eq!(at("oi_xD"), None);
    }

    #[test]
    fn a_digit_in_front_is_never_a_smiley() {
        for text in ["12:3", "10:)", "8XD", "2:']"] {
            assert_eq!(at(text), None, "{text} should not convert");
        }
    }

    #[test]
    fn the_range_covers_exactly_the_sequence() {
        let text = "oi :-) tudo bem";
        let (start, end, emoji) = match_before(text, 6).expect("converts");
        assert_eq!(&text[start..end], ":-)");
        assert_eq!((start, end), (3, 6));
        assert_eq!(emoji, "😄");
    }

    #[test]
    fn the_cursor_need_not_be_at_the_end() {
        let (start, end, emoji) = match_before(":-) depois", 3).expect("converts");
        assert_eq!((start, end), (0, 3));
        assert_eq!(emoji, "😄");
    }

    #[test]
    fn the_longest_sequence_ending_at_the_cursor_wins() {
        assert_eq!(at(":-))").map(|(_, _, e)| e), Some("😃"));
        assert_eq!(at("8-)").map(|(_, _, e)| e), Some("😎"));
    }

    #[test]
    fn an_unfinished_sequence_stays_text() {
        for text in [
            ":-", ":- ", ":", "::", "8-", "8", "x-", "x", ";-", ";", ">_", "T_", ":-:-",
        ] {
            assert_eq!(at(text), None, "{text} should not convert");
        }
    }

    #[test]
    fn ordinary_text_is_left_alone() {
        for text in [
            "x=3",
            "video/",
            "DX11",
            "d:",
            "sla_x_x",
            "hi",
            "8 out of 10",
            "meet at 3:00",
            "e-mail: ana@example.com",
            "12:30",
        ] {
            assert_eq!(at(text), None, "{text} should not convert");
        }
    }

    #[test]
    fn every_entry_is_a_real_emoji() {
        for (sequence, emoji) in TABLE {
            assert!(
                emojis::get(emoji).is_some(),
                "{sequence} maps to {emoji}, which the emoji set does not know"
            );
            assert!(!sequence.is_empty());
            assert!(!sequence.contains(char::is_whitespace));
        }
    }

    #[test]
    fn the_table_has_no_duplicate_spellings() {
        let mut seen: Vec<&str> = TABLE.iter().map(|(sequence, _)| *sequence).collect();
        seen.sort_unstable();
        let count = seen.len();
        seen.dedup();
        assert_eq!(seen.len(), count, "the table repeats a spelling");
    }
}
