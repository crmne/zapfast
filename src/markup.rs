//! WhatsApp message markup for egui.
//!
//! `*bold*`, `_italic_`, `~struck~`, `` `mono` ``, fenced blocks, `> `
//! quotes, `* ` lists, links, phone numbers, email addresses, and named
//! `@mentions`. Emoji go through [`crate::emoji`].

use std::ops::Range;
use std::sync::Arc;

use egui::text::LayoutJob;
use egui::{Color32, FontId, Galley, Pos2, Stroke, TextFormat};
use fastframe_text::snap_to_pixels;

use crate::bidi;
use crate::emoji;
use crate::theme;

/// WhatsApp mention id and display name.
#[derive(Clone, Debug, PartialEq)]
pub struct Mention {
    pub user: String,
    pub name: String,
}

#[derive(Clone, Copy, Debug)]
pub struct Style {
    pub size: f32,
    pub color: Color32,
    pub secondary: Color32,
    pub link: Color32,
    pub mention: Color32,
}

/// Message text prepared for painting.
pub struct Text {
    pub galley: Arc<Galley>,
    placements: Vec<String>,
    /// Character ranges linked to web addresses.
    pub links: Vec<(Range<usize>, String)>,
    /// Whether the message is emoji-only and should use a larger size.
    pub big: bool,
    accessible_text: String,
}

impl Text {
    /// Emoji sequences represented by placeholder glyphs.
    pub fn placements(&self) -> &[String] {
        &self.placements
    }

    /// Returns the link at a character index.
    pub fn link_at(&self, character: usize) -> Option<&str> {
        self.links
            .iter()
            .find(|(range, _)| range.contains(&character))
            .map(|(_, url)| url.as_str())
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
struct Span {
    text: String,
    bold: bool,
    italic: bool,
    strike: bool,
    mono: bool,
    link: Option<String>,
    mention: bool,
    /// Paints a quoted line's bar and indent.
    quote: bool,
}

/// Lays text out within `max_width`.
pub fn layout(
    ui: &egui::Ui,
    text: &str,
    mentions: &[Mention],
    style: &Style,
    max_width: f32,
) -> Text {
    let big = emoji::only_emoji(text).is_some_and(|count| count <= 3);
    let size = if big { style.size * 2.4 } else { style.size };
    let mut job = LayoutJob::default();
    job.wrap.max_width = max_width;
    let mut placements = Vec::new();
    let mut links = Vec::new();
    let mut characters = 0;
    for span in parse(text, mentions) {
        let font_id = if span.mono {
            FontId::monospace(size * 0.95)
        } else if span.bold || span.mention {
            theme::bold(size)
        } else {
            theme::regular(size)
        };
        let color = if span.link.is_some() {
            style.link
        } else if span.mention {
            style.mention
        } else if span.quote && span.text.starts_with('▎') {
            style.secondary
        } else {
            style.color
        };
        let format = TextFormat {
            font_id,
            color,
            italics: span.italic,
            underline: if span.link.is_some() {
                Stroke::new(1.0, style.link)
            } else {
                Stroke::NONE
            },
            strikethrough: if span.strike {
                Stroke::new(1.0, color)
            } else {
                Stroke::NONE
            },
            ..Default::default()
        };
        let before = characters;
        let after = before + emoji::append(ui, &mut job, &mut placements, &span.text, &format);
        if let Some(url) = span.link {
            links.push((before..after, url));
        }
        characters = after;
    }
    if job.text.is_empty() {
        job.append(
            " ",
            0.0,
            TextFormat::simple(theme::regular(size), style.color),
        );
    }
    let galley = bidi::layout_job(ui, job);
    Text {
        galley,
        placements,
        links,
        big,
        accessible_text: plain(text, mentions),
    }
}

/// Paints laid-out text at `pos`.
pub fn paint(ui: &egui::Ui, text: &Text, pos: Pos2, fallback: Color32) {
    ui.painter().galley(pos, text.galley.clone(), fallback);
    emoji::paint(ui, &text.galley, pos, &text.placements);
}

/// Paints selectable text. The response must sense clicks and drags. Set
/// `visible` false for off-screen galleys kept only for selection state.
pub fn paint_selectable(
    ui: &egui::Ui,
    text: &Text,
    response: &egui::Response,
    pos: Pos2,
    fallback: Color32,
    visible: bool,
) {
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Label,
            ui.is_enabled(),
            &text.accessible_text,
        )
    });
    // egui treats non-overlapping text bounds as separate columns. Incoming
    // and outgoing bubbles are one transcript even when both contain short
    // text. Give selection a shared column while keeping every glyph in its
    // original screen position; layout, hit targets, and link offsets stay put.
    let mut galley = (*text.galley).clone();
    let column = ui.clip_rect().x_range();
    // Both ends on whole physical pixels: egui rounds where it paints the
    // galley, not this shift, so a fractional offset left every glyph of a
    // bubble between pixels and blurred it (0.21 px at 133%).
    let ppp = ui.pixels_per_point();
    let pos = Pos2::new(snap_to_pixels(pos.x, ppp), pos.y);
    let offset = pos.x - snap_to_pixels(column.min, ppp);
    for row in &mut galley.rows {
        row.pos.x += offset;
    }
    galley.rect.min.x = 0.0;
    galley.rect.max.x = column.span();
    galley.mesh_bounds = galley.mesh_bounds.translate(egui::vec2(offset, 0.0));
    let selection_pos = Pos2::new(snap_to_pixels(column.min, ppp), pos.y);
    egui::text_selection::LabelSelectionState::label_text_selection(
        ui,
        response,
        selection_pos,
        Arc::new(galley),
        fallback,
        egui::Stroke::NONE,
    );
    if visible {
        emoji::paint(ui, &text.galley, pos, &text.placements);
    }
}

/// Plain text with resolved mentions, used in previews.
pub fn plain(text: &str, mentions: &[Mention]) -> String {
    parse(text, mentions)
        .into_iter()
        .map(|span| span.text)
        .collect()
}

/// Replaces mention ids with names without parsing other markup.
pub fn name_mentions(text: &str, mentions: &[Mention]) -> String {
    if mentions.is_empty() {
        return text.to_owned();
    }
    plain(text, mentions)
}

fn parse(text: &str, mentions: &[Mention]) -> Vec<Span> {
    let mut spans = Vec::new();
    let mut in_block = false;
    let mut first = true;
    for line in text.split('\n') {
        if !first {
            spans.push(Span {
                text: "\n".to_owned(),
                ..Default::default()
            });
        }
        first = false;
        let trimmed = line.trim_end();
        // A standalone ``` toggles a block. Inline ``` is handled below.
        if trimmed.trim() == "```" {
            in_block = !in_block;
            continue;
        }
        if in_block {
            spans.push(Span {
                text: line.to_owned(),
                mono: true,
                ..Default::default()
            });
            continue;
        }
        let mut quote = false;
        let mut content = line;
        if let Some(rest) = content
            .strip_prefix("> ")
            .or_else(|| content.strip_prefix(">"))
        {
            quote = true;
            content = rest;
            spans.push(Span {
                text: "▎ ".to_owned(),
                quote: true,
                ..Default::default()
            });
        }
        let indent = content.len() - content.trim_start().len();
        let body = &content[indent..];
        let bullet = ["* ", "- ", "• ", "◦ "]
            .into_iter()
            .find_map(|marker| body.strip_prefix(marker));
        if let Some(rest) = bullet {
            spans.push(Span {
                text: format!("{}•  ", &content[..indent]),
                quote,
                ..Default::default()
            });
            content = rest;
        }
        for mut span in inline(content) {
            span.quote = quote;
            if span.mono {
                spans.push(span);
            } else {
                spans.extend(link_and_mention(span, mentions));
            }
        }
    }
    spans
}

/// Inline markers within one line.
fn inline(line: &str) -> Vec<Span> {
    let chars: Vec<char> = line.chars().collect();
    let mut spans = Vec::new();
    let mut run = String::new();
    // bold, italic, strike
    let mut flags = [false; 3];
    let flush = |run: &mut String, spans: &mut Vec<Span>, flags: [bool; 3]| {
        if !run.is_empty() {
            spans.push(Span {
                text: std::mem::take(run),
                bold: flags[0],
                italic: flags[1],
                strike: flags[2],
                ..Default::default()
            });
        }
    };
    let mut i = 0;
    let triple = |at: usize| {
        at + 2 < chars.len() && chars[at] == '`' && chars[at + 1] == '`' && chars[at + 2] == '`'
    };
    while i < chars.len() {
        let c = chars[i];
        // WhatsApp inline monospace: ```text```.
        if triple(i)
            && let Some(close) = (i + 3..chars.len()).find(|&at| triple(at))
            && close > i + 3
        {
            flush(&mut run, &mut spans, flags);
            spans.push(Span {
                text: chars[i + 3..close].iter().collect(),
                mono: true,
                ..Default::default()
            });
            i = close + 3;
            continue;
        }
        if c == '`'
            && let Some(close) = chars[i + 1..].iter().position(|c| *c == '`')
        {
            let close = i + 1 + close;
            if close > i + 1 {
                flush(&mut run, &mut spans, flags);
                spans.push(Span {
                    text: chars[i + 1..close].iter().collect(),
                    mono: true,
                    ..Default::default()
                });
                i = close + 1;
                continue;
            }
        }
        if matches!(c, '*' | '_' | '~') {
            let index = match c {
                '*' => 0,
                '_' => 1,
                _ => 2,
            };
            if flags[index] {
                if is_closer(&chars, i) {
                    flush(&mut run, &mut spans, flags);
                    flags[index] = false;
                    i += 1;
                    continue;
                }
            } else if is_opener(&chars, i) && has_closer(&chars, i + 1, c) {
                flush(&mut run, &mut spans, flags);
                flags[index] = true;
                i += 1;
                continue;
            }
        }
        run.push(c);
        i += 1;
    }
    flush(&mut run, &mut spans, flags);
    spans
}

fn is_opener(chars: &[char], i: usize) -> bool {
    let before_ok = i == 0 || !chars[i - 1].is_alphanumeric();
    let after_ok = chars
        .get(i + 1)
        .is_some_and(|next| !next.is_whitespace() && *next != chars[i]);
    before_ok && after_ok
}

fn is_closer(chars: &[char], i: usize) -> bool {
    let before_ok = i > 0 && !chars[i - 1].is_whitespace() && chars[i - 1] != chars[i];
    let after_ok = chars.get(i + 1).is_none_or(|next| !next.is_alphanumeric());
    before_ok && after_ok
}

fn has_closer(chars: &[char], from: usize, marker: char) -> bool {
    (from..chars.len()).any(|j| chars[j] == marker && is_closer(chars, j))
}

/// Splits a plain span around web addresses and mentions.
fn link_and_mention(span: Span, mentions: &[Mention]) -> Vec<Span> {
    let mut out = Vec::new();
    let text = span.text.as_str();
    let mut plain_start = 0;
    let mut i = 0;
    let bytes = text.as_bytes();
    while i < text.len() {
        if !text.is_char_boundary(i) {
            i += 1;
            continue;
        }
        let at_boundary = i == 0
            || !text[..i]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_alphanumeric());
        if at_boundary {
            if (bytes[i].is_ascii_digit() || matches!(bytes[i], b'+' | b'('))
                && let Some((end, digits, valid)) = phone_at(text, i)
            {
                if valid {
                    push_plain(&mut out, &span, &text[plain_start..i]);
                    out.push(Span {
                        text: text[i..end].to_owned(),
                        link: Some(format!("tel:{digits}")),
                        bold: span.bold,
                        italic: span.italic,
                        strike: span.strike,
                        quote: span.quote,
                        ..Default::default()
                    });
                    plain_start = end;
                    i = end;
                    continue;
                }
                if digits.is_empty() && end > i {
                    i = end;
                    continue;
                }
                // Don't retry a suffix of an oversized number as a new link.
                // Let the email parser inspect the whole token when it ends in @.
                if digits.len() > 15 && !text[end..].starts_with('@') {
                    i = end;
                    continue;
                }
            }
            if bytes[i] == b'@'
                && let Some((end, mention)) = mention_at(text, i, mentions)
            {
                push_plain(&mut out, &span, &text[plain_start..i]);
                out.push(Span {
                    text: format!("@{}", mention.name),
                    mention: true,
                    quote: span.quote,
                    ..Default::default()
                });
                plain_start = end;
                i = end;
                continue;
            }
            // Links start with a letter or digit. Skip punctuation runs.
            if bytes[i].is_ascii_alphanumeric()
                && let Some((end, url)) = link_at(text, i)
            {
                push_plain(&mut out, &span, &text[plain_start..i]);
                out.push(Span {
                    text: text[i..end].to_owned(),
                    link: Some(url),
                    bold: span.bold,
                    italic: span.italic,
                    strike: span.strike,
                    quote: span.quote,
                    ..Default::default()
                });
                plain_start = end;
                i = end;
                continue;
            }
        }
        i += 1;
    }
    push_plain(&mut out, &span, &text[plain_start..]);
    out
}

/// Finds a phone candidate with optional spaces, hyphens, and parentheses.
/// Dots are excluded so formatted tax IDs such as XXX.XXX.XXX-XX stay plain.
/// The validity flag lets the caller skip an oversized number as one token.
fn phone_at(text: &str, at: usize) -> Option<(usize, String, bool)> {
    // Reject an ISO date before scanning through the space after it; the next
    // token may be a time or a separate phone number.
    let date_at = at + usize::from(text[at..].starts_with('('));
    let iso_date = text
        .as_bytes()
        .get(date_at..date_at + 10)
        .is_some_and(|date| {
            date[4] == b'-'
                && date[7] == b'-'
                && date[..4].iter().all(u8::is_ascii_digit)
                && date[5..7].iter().all(u8::is_ascii_digit)
                && date[8..].iter().all(u8::is_ascii_digit)
        });
    if iso_date {
        return Some((date_at + 10, String::new(), false));
    }

    let mut end = at;
    let mut digits = String::new();
    for (offset, c) in text[at..].char_indices() {
        if c.is_ascii_digit() {
            digits.push(c);
        } else if matches!(c, '+' | ' ' | '-' | '(' | ')') {
            if c == '+' && offset != 0 {
                break;
            }
        } else {
            break;
        }
        end = at + offset + c.len_utf8();
    }
    while end > at && text[..end].ends_with([' ', '-', '(']) {
        end -= text[..end].chars().next_back()?.len_utf8();
    }
    let following = text[end..].chars().next();
    let valid = digits.len() >= 7
        && digits.len() <= 15
        && following.is_none_or(|c| !c.is_ascii_alphanumeric() && c != '@');
    (!digits.is_empty()).then_some((
        end,
        if text[at..].starts_with('+') {
            format!("+{digits}")
        } else {
            digits
        },
        valid,
    ))
}

fn push_plain(out: &mut Vec<Span>, template: &Span, text: &str) {
    if !text.is_empty() {
        out.push(Span {
            text: text.to_owned(),
            link: None,
            mention: false,
            ..template.clone()
        });
    }
}

fn mention_at<'m>(text: &str, at: usize, mentions: &'m [Mention]) -> Option<(usize, &'m Mention)> {
    if let Some(end) = crate::mentions::everyone_token_end(text, at)
        && let Some(mention) = mentions
            .iter()
            .find(|mention| mention.user == crate::mentions::ALL_USER)
    {
        return Some((end, mention));
    }
    let rest = &text[at + 1..];
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    if digits.len() < 5 {
        return None;
    }
    let mention = mentions.iter().find(|mention| mention.user == digits)?;
    Some((at + 1 + digits.len(), mention))
}

/// Parses a web or email address at `at` and returns its end and target.
fn link_at(text: &str, at: usize) -> Option<(usize, String)> {
    let rest = &text[at..];
    // Brackets opened inside the address belong to it, as in
    // `https://en.wikipedia.org/wiki/Rust_(programming_language)`; a closing
    // one without its opener closes the text around it: `(see example.com)`.
    let (mut parens, mut squares) = (0usize, 0usize);
    let token_end = rest
        .find(|c: char| {
            let depth = match c {
                '(' | ')' => &mut parens,
                '[' | ']' => &mut squares,
                _ => return c.is_whitespace() || matches!(c, '<' | '>' | '"' | '\'' | '{' | '}'),
            };
            if matches!(c, '(' | '[') {
                *depth += 1;
                false
            } else if *depth > 0 {
                *depth -= 1;
                false
            } else {
                true
            }
        })
        .unwrap_or(rest.len());
    let mut token = &rest[..token_end];
    while let Some(stripped) = token.strip_suffix(['.', ',', ';', ':', '!', '?', '*', '_', '~']) {
        token = stripped;
    }
    if token.len() < 4 {
        return None;
    }
    let lower = token.to_ascii_lowercase();
    let end = at + token.len();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        return (token.len() > 8).then(|| (end, token.to_owned()));
    }
    if lower.starts_with("www.") {
        return Some((end, format!("https://{token}")));
    }
    if lower.starts_with("mailto:") {
        return Some((end, token.to_owned()));
    }
    if let Some((user, host)) = token.split_once('@')
        && !user.is_empty()
        && !user.contains('/')
        && is_domain(host)
    {
        return Some((end, format!("mailto:{token}")));
    }
    let (host, _) = token.split_once('/').unwrap_or((token, ""));
    if is_domain(host) {
        return Some((end, format!("https://{token}")));
    }
    None
}

/// Whether a token is a hostname without a scheme.
fn is_domain(host: &str) -> bool {
    let host = host.split_once(':').map_or(host, |(name, _)| name);
    let labels: Vec<&str> = host.split('.').collect();
    if labels.len() < 2 || labels.iter().any(|label| label.is_empty()) {
        return false;
    }
    if !labels.iter().all(|label| {
        label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
            && !label.starts_with('-')
            && !label.ends_with('-')
    }) {
        return false;
    }
    let tld = labels[labels.len() - 1].to_ascii_lowercase();
    if !tld.chars().all(|c| c.is_ascii_alphabetic()) {
        return false;
    }
    match tld.len() {
        0..=1 => false,
        2 => !matches!(
            tld.as_str(),
            "js" | "rs"
                | "py"
                | "ts"
                | "md"
                | "sh"
                | "go"
                | "rb"
                | "cs"
                | "cc"
                | "hs"
                | "ml"
                | "so"
        ),
        _ => KNOWN_TLDS.contains(&tld.as_str()),
    }
}

const KNOWN_TLDS: &[&str] = &[
    "com",
    "net",
    "org",
    "edu",
    "gov",
    "mil",
    "int",
    "info",
    "biz",
    "name",
    "pro",
    "app",
    "dev",
    "io",
    "ai",
    "xyz",
    "site",
    "online",
    "tech",
    "cloud",
    "page",
    "shop",
    "store",
    "blog",
    "news",
    "berlin",
    "rocks",
    "world",
    "live",
    "life",
    "media",
    "email",
    "digital",
    "design",
    "studio",
    "agency",
    "club",
    "social",
    "space",
    "wiki",
    "zone",
    "today",
    "art",
    "fun",
    "one",
    "top",
    "link",
    "lol",
    "eu",
    "asia",
    "travel",
    "museum",
    "coop",
    "jobs",
    "mobi",
    "tel",
    "aero",
    "photos",
    "pics",
    "video",
    "music",
    "games",
    "team",
    "run",
    "ninja",
    "guru",
    "expert",
    "chat",
    "codes",
    "cool",
    "events",
    "health",
    "house",
    "land",
    "law",
    "money",
    "network",
    "party",
    "pub",
    "rest",
    "school",
    "science",
    "software",
    "solutions",
    "systems",
    "tools",
    "toys",
    "works",
    "academy",
    "bar",
    "beer",
    "bio",
    "cafe",
    "camp",
    "care",
    "center",
    "city",
    "coffee",
    "company",
    "fit",
    "fitness",
    "gallery",
    "group",
    "help",
    "host",
    "kitchen",
    "love",
    "market",
    "pizza",
    "press",
    "recipes",
    "review",
    "sale",
    "style",
    "tips",
    "vision",
    "watch",
    "wine",
    "yoga",
];

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(text: &str) -> Vec<(String, bool, bool, bool, bool)> {
        parse(text, &[])
            .into_iter()
            .map(|span| (span.text, span.bold, span.italic, span.strike, span.mono))
            .collect()
    }

    #[test]
    fn inline_markers_pair_up() {
        assert_eq!(
            kinds("a *bold* and _it_ and ~no~ and `code`"),
            vec![
                ("a ".into(), false, false, false, false),
                ("bold".into(), true, false, false, false),
                (" and ".into(), false, false, false, false),
                ("it".into(), false, true, false, false),
                (" and ".into(), false, false, false, false),
                ("no".into(), false, false, true, false),
                (" and ".into(), false, false, false, false),
                ("code".into(), false, false, false, true),
            ]
        );
    }

    #[test]
    fn stray_markers_stay_literal() {
        assert_eq!(plain("2 * 3 = 6", &[]), "2 * 3 = 6");
        assert_eq!(plain("snake_case_name", &[]), "snake_case_name");
        assert_eq!(plain("*unclosed", &[]), "*unclosed");
        assert_eq!(plain("* Guestlist", &[]), "•  Guestlist");
    }

    #[test]
    fn blocks_and_lists() {
        let spans = parse("> quoted\n- one\n```\nlet x = 1;\n```", &[]);
        assert_eq!(spans[0].text, "▎ ");
        assert!(spans[0].quote);
        assert_eq!(spans[1].text, "quoted");
        assert!(spans.iter().any(|span| span.text == "•  "));
        assert!(
            spans
                .iter()
                .any(|span| span.mono && span.text == "let x = 1;")
        );
    }

    #[test]
    fn addresses_are_found_with_and_without_a_scheme() {
        let links = |text: &str| -> Vec<String> {
            parse(text, &[])
                .into_iter()
                .filter_map(|span| span.link)
                .collect()
        };
        assert_eq!(links("see https://a.b/c?d=1."), vec!["https://a.b/c?d=1"]);
        assert_eq!(
            links("go to spotifast.rocks!"),
            vec!["https://spotifast.rocks"]
        );
        assert_eq!(
            links("mail hello@section8berlin.com or dm"),
            vec!["mailto:hello@section8berlin.com"]
        );
        assert_eq!(
            links("(www.rust-lang.org)"),
            vec!["https://www.rust-lang.org"]
        );
        assert!(links("version 0.3.0 of main.rs and e.g. this").is_empty());
        assert!(links("abchttp://x").is_empty());
    }

    #[test]
    fn addresses_keep_the_brackets_they_open() {
        let links = |text: &str| -> Vec<String> {
            parse(text, &[])
                .into_iter()
                .filter_map(|span| span.link)
                .collect()
        };
        assert_eq!(
            links("https://example.com/testing(testing)"),
            vec!["https://example.com/testing(testing)"]
        );
        assert_eq!(
            links("read https://en.wikipedia.org/wiki/Foo_(bar)."),
            vec!["https://en.wikipedia.org/wiki/Foo_(bar)"]
        );
        assert_eq!(
            links("(see https://en.wikipedia.org/wiki/Foo_(bar))"),
            vec!["https://en.wikipedia.org/wiki/Foo_(bar)"]
        );
        assert_eq!(
            links("(see https://example.com)"),
            vec!["https://example.com"]
        );
        assert_eq!(
            links("[https://example.com/a[1]]"),
            vec!["https://example.com/a[1]"]
        );
        assert_eq!(
            links("example.com/a(b)c, then"),
            vec!["https://example.com/a(b)c"]
        );
        let spans = parse("(see https://example.com)", &[]);
        assert_eq!(spans.last().map(|span| span.text.as_str()), Some(")"));
    }

    #[test]
    fn phone_numbers_become_links_without_linking_tax_ids() {
        let links = |text: &str| -> Vec<String> {
            parse(text, &[])
                .into_iter()
                .filter_map(|span| span.link)
                .collect()
        };
        assert_eq!(
            links("CPF 000.000.000-00; call 000 0000-0000 or +00 (00) 00000-0000."),
            vec!["tel:00000000000", "tel:+0000000000000"]
        );
        assert_eq!(links("Call (212) 555-1212"), vec!["tel:2125551212"]);
        assert_eq!(
            parse("Call (212) 555-1212", &[])
                .into_iter()
                .find(|span| span.link.is_some())
                .unwrap()
                .text,
            "(212) 555-1212"
        );
        assert_eq!(links("Call +1 (212) 555-1212"), vec!["tel:+12125551212"]);
        let spans = parse("Call +1 (212) 555-1212", &[]);
        assert_eq!(
            spans.iter().find(|span| span.link.is_some()).unwrap().text,
            "+1 (212) 555-1212"
        );
        assert!(links("Date 2026-10-01").is_empty());
        assert!(links("Date (2026-10-01)").is_empty());
        assert!(links("Date 2026-10-01 12:30").is_empty());
        assert_eq!(links("Date 2026-10-01 1234567"), vec!["tel:1234567"]);
        assert!(links("Reference 12345678-90123456").is_empty());
        assert_eq!(
            links("1234567@example.com"),
            vec!["mailto:1234567@example.com"]
        );
        assert_eq!(
            links("1234567890123456@example.com"),
            vec!["mailto:1234567890123456@example.com"]
        );
        assert!(links("id 000000; long id 0000000000000000").is_empty());
        assert!(links("abc00000000000").is_empty());
    }

    #[test]
    fn mentions_become_names() {
        let mentions = [Mention {
            user: "174057861464188".into(),
            name: "+49 176 31141665".into(),
        }];
        let spans = parse("20:00 @174057861464188 (no pronouns)", &mentions);
        let mention = spans.iter().find(|span| span.mention).expect("mention");
        assert_eq!(mention.text, "@+49 176 31141665");
        assert_eq!(
            plain("hi @174057861464188", &mentions),
            "hi @+49 176 31141665"
        );
        assert_eq!(plain("hi @123", &mentions), "hi @123");
    }

    #[test]
    fn everyone_is_highlighted_only_with_its_mention() {
        let everyone = [Mention {
            user: "all".into(),
            name: "all".into(),
        }];
        let spans = parse("heading over @all tonight", &everyone);
        let mention = spans.iter().find(|span| span.mention).expect("mention");
        assert_eq!(mention.text, "@all");
        assert!(
            parse("heading over @all tonight", &[])
                .iter()
                .all(|span| !span.mention)
        );
        assert!(
            parse("mail@all.com and @alligator", &everyone)
                .iter()
                .all(|span| !span.mention)
        );
        assert_eq!(plain("ping @all", &everyone), "ping @all");
    }
}

#[cfg(test)]
mod monospace_tests {
    use super::*;

    fn mono(text: &str) -> Vec<(String, bool)> {
        parse(text, &[])
            .into_iter()
            .map(|span| (span.text, span.mono))
            .collect()
    }

    #[test]
    fn triple_backticks_mark_monospace_on_a_line() {
        assert_eq!(
            mono("say ```hello``` now"),
            vec![
                ("say ".into(), false),
                ("hello".into(), true),
                (" now".into(), false),
            ]
        );
        assert_eq!(mono("```hello```"), vec![("hello".into(), true)]);
        // Preserve all text after an unmatched marker.
        assert_eq!(
            mono("```a```\nb"),
            vec![
                ("a".into(), true),
                ("\n".into(), false),
                ("b".into(), false)
            ]
        );
    }

    #[test]
    fn a_fence_line_still_opens_a_block() {
        let spans = mono("```\ncode\n```\nafter");
        assert!(spans.iter().any(|(text, mono)| text == "code" && *mono));
        assert!(spans.iter().any(|(text, mono)| text == "after" && !*mono));
    }
}
