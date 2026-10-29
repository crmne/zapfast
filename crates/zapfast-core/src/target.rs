//! The chat a launch argument names, and the text it asks to start with.
//!
//! ZapFast is asked to open a chat by a number on its own, a `wa.me` link, or
//! the `whatsapp://send?phone=` URI the desktop entry registers. All three
//! name the same chat, filed under the number's JID, and any of them may
//! carry a `text=` template to start the conversation with.

/// Where a one-to-one chat is filed. A chat behind a privacy id keeps that id
/// in the archive until the mapping is known, so a link opens the phone-number
/// chat, which the archive merges once WhatsApp answers.
const PHONE_SERVER: &str = "s.whatsapp.net";

/// Hosts that keep a phone number in the last segment of their path.
/// Fewest digits a number may have. E.164 allows up to fifteen; the lower
/// bound keeps a year, a short code, or a stray digit out of a chat.
const MIN_DIGITS: usize = 6;
/// Most digits E.164 allows.
const MAX_DIGITS: usize = 15;

/// What a launch argument asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    /// The chat to open, as `<digits>@s.whatsapp.net`.
    pub chat: String,
    /// The template to start the conversation with, from `text=`. `None`
    /// when the argument carries none, or carries only blanks.
    pub text: Option<String>,
}

impl Request {
    /// The name this chat shows until the contact is known.
    pub fn display_name(&self) -> String {
        crate::util::phone(&number_of_jid(&self.chat).unwrap_or_default())
    }

    /// The single-instance verb another launch sends this as.
    ///
    /// `open <chat>`, then a tab and the escaped template when there is one.
    /// The request is one line, so the text is escaped rather than written
    /// out: a space would end the verb and a newline would end the line.
    /// A blank template is left off, as [`parse`] leaves it out.
    pub fn verb(&self) -> String {
        match self.text.as_deref().filter(|text| !text.trim().is_empty()) {
            Some(text) => format!("open {}\t{}", self.chat, encode(text)),
            None => format!("open {}", self.chat),
        }
    }

    /// Reads back what [`Request::verb`] wrote, the `open ` already removed.
    ///
    /// The chat is held to the same rule [`parse`] applies, so a request from
    /// another launch cannot name an id this one would not have opened.
    pub fn from_verb(verb: &str) -> Option<Self> {
        let (chat, text) = match verb.split_once('\t') {
            Some((chat, text)) => (chat, Some(decode(text))),
            None => (verb, None),
        };
        number_of_jid(chat)?;
        Some(Self {
            chat: chat.to_owned(),
            text: text.filter(|text| !text.trim().is_empty()),
        })
    }
}

/// Reads what `target` asks for.
///
/// Takes a number on its own in any formatting, a `wa.me` or
/// `api.whatsapp.com/send?phone=` link, and the `whatsapp://send?phone=` URI
/// the desktop entry passes, with an optional `text=` template on any of them.
/// Anything that does not carry a number where one of those keeps it is
/// refused, so a launch argument that is not a chat opens nothing rather than
/// the wrong one.
pub fn parse(target: &str) -> Option<Request> {
    let trimmed = target.trim();
    if trimmed.is_empty() {
        return None;
    }
    let (_, query) = split_query(trimmed);
    Some(Request {
        chat: format!("{}@{PHONE_SERVER}", number_of(trimmed)?),
        text: parameter(query, "text")
            .map(decode)
            .filter(|text| !text.trim().is_empty()),
    })
}

/// The number `target` names, read where its own shape keeps it.
///
/// The shape decides where to look: a `wa.me` link keeps its number in the
/// path, the web send link and the `whatsapp:` URI keep it in the query, and a
/// number on its own is the whole argument. A link of any other shape names no
/// chat, however its query is spelled.
fn number_of(target: &str) -> Option<String> {
    let (head, query) = split_query(target);
    let lower = head.to_ascii_lowercase();
    if lower.starts_with("whatsapp:") || lower.starts_with("whatsapp-send:") {
        return parameter(query, "phone").and_then(number_in);
    }
    if let Some((host, path)) = http_host_path(head) {
        return match host.to_ascii_lowercase().as_str() {
            "wa.me" | "www.wa.me" => last_segment(path)
                .and_then(number_in)
                .or_else(|| parameter(query, "phone").and_then(number_in)),
            "api.whatsapp.com" | "web.whatsapp.com" => {
                parameter(query, "phone").and_then(number_in)
            }
            _ => None,
        };
    }
    // A number on its own, or a `wa.me` link typed without its scheme.
    let bare = match lower.strip_prefix("wa.me/") {
        Some(_) => &head["wa.me/".len()..],
        None => head,
    };
    number_in(bare)
}

/// Splits a target at its query, so a query never lends its digits to a
/// number that lives in the path.
///
/// A fragment is neither path nor query and is dropped first, so
/// `wa.me/20123456789#section` names its chat and a `#` after a template does
/// not reach the composer. A `#` inside a template is written `%23`, as a
/// link must write it.
fn split_query(target: &str) -> (&str, &str) {
    let target = target.split('#').next().unwrap_or(target);
    match target.split_once('?') {
        Some((head, query)) => (head, query),
        None => (target, ""),
    }
}

/// The host and path of an `http(s)` link, the scheme off.
fn http_host_path(target: &str) -> Option<(&str, &str)> {
    let rest = target
        .strip_prefix("https://")
        .or_else(|| target.strip_prefix("http://"))?;
    Some(rest.split_once('/').unwrap_or((rest, "")))
}

/// The last segment of a path, which is where a `wa.me` link keeps its number.
fn last_segment(path: &str) -> Option<&str> {
    let path = path.trim_end_matches('/');
    path.rsplit('/')
        .next()
        .filter(|segment| !segment.is_empty())
}

/// The digits of the number written in `text`, when every other character is
/// formatting a phone number may wear.
///
/// A letter, a slash, or anything else is refused rather than dropped, so
/// `call 20123456789` names no chat instead of one nobody asked for.
fn number_in(text: &str) -> Option<String> {
    let text = decode(text);
    let mut digits = String::with_capacity(text.len());
    for character in text.chars() {
        if character.is_ascii_digit() {
            digits.push(character);
        } else if !matches!(character, '+' | ' ' | '-' | '(' | ')' | '.') {
            return None;
        }
    }
    is_a_number(&digits).then_some(digits)
}

/// Whether `digits` can be a phone number's user part.
fn is_a_number(digits: &str) -> bool {
    (MIN_DIGITS..=MAX_DIGITS).contains(&digits.len())
}

/// The digits of the phone number `id` names, when `id` is one. A JID whose
/// user part is not all digits, or whose server is another one, is not.
fn number_of_jid(id: &str) -> Option<String> {
    let (user, server) = id.split_once('@')?;
    (server == PHONE_SERVER && user.chars().all(|c| c.is_ascii_digit()) && is_a_number(user))
        .then(|| user.to_owned())
}

/// One parameter of a query, still escaped.
fn parameter<'a>(query: &'a str, key: &str) -> Option<&'a str> {
    if query.is_empty() {
        return None;
    }
    query.split('&').find_map(|pair| {
        let (name, value) = pair.split_once('=')?;
        (name == key).then_some(value)
    })
}

/// Percent-decodes one query value.
///
/// `+` stays a plus rather than becoming a space: a number writes it as
/// `%2B`, so this query is escaped, not form-encoded. An escape that is not
/// two hex digits is kept as written, so a malformed link still opens its
/// chat and still carries whatever template it could.
fn decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && let Some(high) = bytes.get(index + 1).copied().and_then(hex_digit)
            && let Some(low) = bytes.get(index + 2).copied().and_then(hex_digit)
        {
            out.push(high * 16 + low);
            index += 3;
            continue;
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Percent-encodes what a one-line verb cannot carry verbatim.
fn encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

fn hex_digit(byte: u8) -> Option<u8> {
    char::from(byte).to_digit(16).map(|digit| digit as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHAT: &str = "20123456789@s.whatsapp.net";

    fn chat_of_argument(target: &str) -> String {
        parse(target).expect("a chat").chat
    }

    fn text_of(target: &str) -> Option<String> {
        parse(target).expect("a chat").text
    }

    #[test]
    fn a_number_in_any_formatting_names_one_chat() {
        for target in [
            "20123456789",
            "+20123456789",
            "+20 123 456 789",
            "+20 (123) 456-789",
            "  20123456789  ",
        ] {
            assert_eq!(chat_of_argument(target), CHAT, "{target}");
        }
    }

    #[test]
    fn a_wa_me_link_names_the_same_chat() {
        for target in [
            "https://wa.me/20123456789",
            "http://wa.me/20123456789",
            "https://wa.me/20123456789/",
            "https://www.wa.me/20123456789",
            "wa.me/20123456789",
            "https://wa.me/%2B20123456789",
        ] {
            assert_eq!(chat_of_argument(target), CHAT, "{target}");
        }
    }

    #[test]
    fn the_uri_the_desktop_entry_passes_names_the_chat() {
        for target in [
            "whatsapp://send?phone=20123456789",
            "whatsapp-send://?phone=20123456789",
            "whatsapp://send?phone=20123456789&text=hello",
            "whatsapp://send?phone=%2B20123456789",
            "https://api.whatsapp.com/send?phone=20123456789",
        ] {
            assert_eq!(chat_of_argument(target), CHAT, "{target}");
        }
    }

    #[test]
    fn a_link_that_only_looks_like_one_is_refused() {
        for target in [
            "",
            "   ",
            "hello",
            "reload-themes",
            "--verbose",
            "https://wa.me/",
            "https://wa.me",
            "https://example.com/20123456789",
            "https://example.com/wa.me/20123456789",
            "whatsapp://send?text=hello",
            // A `phone=` on a host nobody vouched for names no chat.
            "https://example.com/?phone=20123456789",
            "https://evil.test/send?phone=20123456789",
            // Nor does a bare argument that spells one.
            "phone=20123456789",
        ] {
            assert_eq!(parse(target), None, "{target}");
        }
    }

    /// A letter is not formatting a number may wear, so text that merely
    /// contains digits names no chat.
    #[test]
    fn digits_among_words_are_not_a_number() {
        for target in [
            "call 20123456789",
            "20123456789 please",
            "https://wa.me/abc20123456789",
            "wa.me/20123456789abc",
        ] {
            assert_eq!(parse(target), None, "{target}");
        }
    }

    /// The shape decides where the number is. A `wa.me` link keeps it in the
    /// path, so a query cannot lend it digits, and a path number wins over a
    /// `phone=` that disagrees with it.
    #[test]
    fn a_query_never_lends_its_digits_to_the_number() {
        // The path has no number, and the query's must not become one.
        assert_eq!(parse("https://wa.me/?text=20123456789"), None);
        assert_eq!(parse("https://wa.me/2012?text=999999"), None);
        // The path number is the chat, whatever the query says.
        assert_eq!(
            chat_of_argument("https://wa.me/20123456789?phone=999999999"),
            CHAT
        );
    }

    /// A fragment is neither path nor query: it must not hide a number, and
    /// it must not reach the composer. A `#` in a template is `%23`.
    #[test]
    fn a_fragment_is_not_part_of_the_number_or_the_template() {
        assert_eq!(chat_of_argument("https://wa.me/20123456789#section"), CHAT);
        assert_eq!(chat_of_argument("20123456789#section"), CHAT);
        assert_eq!(
            text_of("https://wa.me/20123456789?text=Hello#section"),
            Some("Hello".to_owned())
        );
        assert_eq!(
            text_of("whatsapp://send?phone=20123456789&text=a%23b"),
            Some("a#b".to_owned()),
            "an escaped hash is part of the template"
        );
    }

    /// A number on its own may be written with the formatting a phone number
    /// wears, and nothing else.
    #[test]
    fn a_bare_number_takes_formatting_but_not_words() {
        for target in ["+20 (123) 456-789", "+20-123-456-789", "+20.123.456.789"] {
            assert_eq!(chat_of_argument(target), CHAT, "{target}");
        }
    }

    /// A year, a short code, and a number past E.164's length all read as
    /// digits, and none of them is a phone number.
    #[test]
    fn a_number_that_cannot_be_one_is_refused() {
        for target in ["2026", "12345", "1234567890123456"] {
            assert_eq!(parse(target), None, "{target}");
        }
    }

    /// The shortest and the longest number E.164 allows are both read.
    #[test]
    fn the_bounds_of_e_164_are_inclusive() {
        assert!(parse("123456").is_some());
        assert!(parse("123456789012345").is_some());
    }

    #[test]
    fn a_template_arrives_decoded() {
        assert_eq!(
            text_of("whatsapp://send?phone=20123456789&text=Hello%20there"),
            Some("Hello there".to_owned())
        );
        assert_eq!(
            text_of("https://wa.me/20123456789?text=Hi%2C%20how%20are%20you%3F"),
            Some("Hi, how are you?".to_owned())
        );
        assert_eq!(
            text_of("whatsapp://send?phone=20123456789&text=Caf%C3%A9%20%2B%2020"),
            Some("Café + 20".to_owned())
        );
    }

    /// The template may sit before the number, and a share link keeps the
    /// number in the path and the template in the query.
    #[test]
    fn the_template_is_read_wherever_it_sits() {
        assert_eq!(
            text_of("whatsapp://send?text=Hi&phone=20123456789"),
            Some("Hi".to_owned())
        );
        assert_eq!(
            text_of("https://wa.me/20123456789?text=%E2%9C%93"),
            Some("✓".to_owned())
        );
    }

    /// A template with nothing to say is no template, and must not leave an
    /// empty string in the composer.
    #[test]
    fn a_blank_template_is_no_template() {
        for target in [
            "whatsapp://send?phone=20123456789&text=",
            "whatsapp://send?phone=20123456789&text=%20%20",
            "20123456789",
        ] {
            assert_eq!(text_of(target), None, "{target}");
        }
    }

    /// A malformed escape is kept as written rather than losing the rest of
    /// the template to it.
    #[test]
    fn a_malformed_escape_does_not_lose_the_text() {
        assert_eq!(
            text_of("whatsapp://send?phone=20123456789&text=a%ZZb"),
            Some("a%ZZb".to_owned())
        );
    }

    /// What `verb` writes, `from_verb` reads back unchanged, for every shape
    /// the two can be handed.
    #[test]
    fn a_request_survives_the_wire() {
        for request in [
            Request {
                chat: CHAT.to_owned(),
                text: None,
            },
            Request {
                chat: CHAT.to_owned(),
                text: Some("Hello there".to_owned()),
            },
            Request {
                chat: CHAT.to_owned(),
                text: Some("Café ✓ +20, 100% sure\ttabbed\nnewline".to_owned()),
            },
        ] {
            let verb = request.verb();
            assert!(!verb.contains('\n'), "one line: {verb}");
            assert_eq!(
                Request::from_verb(verb.strip_prefix("open ").unwrap()),
                Some(request),
                "{verb}"
            );
        }
    }

    /// A template with nothing to say is left off the wire rather than
    /// carried as blanks, so a request holds the same rule on both sides.
    #[test]
    fn a_blank_template_is_not_carried() {
        let request = Request {
            chat: CHAT.to_owned(),
            text: Some("  \t ".to_owned()),
        };
        assert_eq!(request.verb(), format!("open {CHAT}"));
        assert_eq!(
            Request::from_verb(request.verb().strip_prefix("open ").unwrap()),
            Some(Request {
                chat: CHAT.to_owned(),
                text: None
            })
        );
    }

    /// A verb is one line, so the separator appears exactly once however the
    /// template is written.
    #[test]
    fn a_template_never_breaks_the_line() {
        let verb = Request {
            chat: CHAT.to_owned(),
            text: Some("a\tb\nc d".to_owned()),
        }
        .verb();
        assert_eq!(verb.lines().count(), 1, "{verb}");
        assert_eq!(verb.matches('\t').count(), 1, "{verb}");
    }

    #[test]
    fn a_verb_naming_anything_else_is_refused() {
        for verb in [
            "",
            "20123456789",
            "12345@s.whatsapp.net",
            "1234567890123456@s.whatsapp.net",
            "notadigits@s.whatsapp.net",
            "20123456789@lid",
            "20123456789@s.whatsapp.net.evil",
            "\u{2028}20123456789@s.whatsapp.net",
        ] {
            assert_eq!(Request::from_verb(verb), None, "{verb}");
        }
    }

    #[test]
    fn the_display_name_is_the_number_grouped() {
        let request = parse("wa.me/20123456789?text=Hi").unwrap();
        assert_eq!(request.display_name(), crate::util::phone("20123456789"));
        assert!(
            request.display_name().starts_with('+'),
            "a number reads as one"
        );
    }
}
