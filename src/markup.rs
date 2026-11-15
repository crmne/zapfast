//! WhatsApp message markup for egui.
//!
//! `*bold*`, `_italic_`, `~struck~`, `` `mono` ``, fenced blocks, `> `
//! quotes, `* ` lists, links, phone numbers, email addresses, and named
//! `@mentions`. Emoji go through [`crate::emoji`].

use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use egui::epaint::text::ByteRangeExt;
use egui::text::{ByteIndex, LayoutJob, LayoutSection};
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
    /// Markup itself, such as a `*` around bold: a draft shows it, a
    /// message leaves it out.
    marker: bool,
}

impl Span {
    fn marker(text: impl Into<String>) -> Self {
        Span {
            text: text.into(),
            marker: true,
            ..Default::default()
        }
    }
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

/// The web addresses in `text`, each once, in reading order, found the way
/// a bubble underlines them. Mail addresses are left out: they do not open
/// in a browser.
pub fn links(text: &str) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for span in parse(text, &[]) {
        if let Some(link) = span.link
            && !link.starts_with("mailto:")
            && !found.contains(&link)
        {
            found.push(link);
        }
    }
    found
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

/// How a stretch of a draft looks while it is typed.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Look {
    marker: bool,
    bold: bool,
    italic: bool,
    strike: bool,
    mono: bool,
}

/// Shows a draft's markup in its editor layout while it is typed: bold,
/// italic, strikethrough and monospace (in `mono`) as a sent message shows
/// them, with the markers kept in place in `markers`. Every character keeps
/// its place, so carets and selections still match the buffer. Emoji, which
/// the editor paints over transparent glyphs, are left whole.
pub fn style_draft(job: &mut LayoutJob, markers: Color32, mono: &FontId) {
    type Looks = Rc<[(Range<usize>, Look)]>;
    thread_local! {
        // The composer lays its draft out several times a frame and again on
        // every repaint, so each draft is parsed once.
        static LAST: RefCell<(String, Looks)> = RefCell::default();
    }
    let looks = LAST.with_borrow_mut(|(text, looks)| {
        if *text != job.text {
            *looks = draft_looks(&job.text).into();
            text.clone_from(&job.text);
        }
        Rc::clone(looks)
    });
    if looks.is_empty() {
        return;
    }
    let old = std::mem::take(&mut job.sections);
    let mut sections = Vec::with_capacity(old.len() + 2 * looks.len());
    let mut next = 0;
    for section in old {
        let Range { start, end } = section.byte_range.as_usize();
        while looks.get(next).is_some_and(|(range, _)| range.end <= start) {
            next += 1;
        }
        // Splitting an emoji would shape its parts apart.
        if section.format.color == Color32::TRANSPARENT {
            sections.push(section);
            continue;
        }
        let piece = |range: Range<usize>, format: TextFormat| LayoutSection {
            leading_space: if range.start == start {
                section.leading_space
            } else {
                0.0
            },
            byte_range: ByteIndex(range.start)..ByteIndex(range.end),
            format,
        };
        let mut at = start;
        for (range, look) in &looks[next..] {
            if range.start >= end {
                break;
            }
            if range.start > at {
                sections.push(piece(at..range.start, section.format.clone()));
                at = range.start;
            }
            let stop = range.end.min(end);
            sections.push(piece(
                at..stop,
                draft_format(&section.format, *look, markers, mono),
            ));
            at = stop;
        }
        if at < end {
            sections.push(piece(at..end, section.format.clone()));
        }
    }
    job.sections = sections;
}

/// The byte ranges of a draft that do not look like plain text.
fn draft_looks(text: &str) -> Vec<(Range<usize>, Look)> {
    let mut looks = Vec::new();
    let mut at = 0;
    for span in draft(text) {
        let end = at + span.text.len();
        let look = Look {
            marker: span.marker,
            bold: span.bold,
            italic: span.italic,
            strike: span.strike,
            mono: span.mono,
        };
        if end > at && look != Look::default() {
            looks.push((at..end, look));
        }
        at = end;
    }
    debug_assert_eq!(at, text.len(), "a draft's spans cover its text");
    if at != text.len() {
        looks.clear();
    }
    looks
}

fn draft_format(base: &TextFormat, look: Look, markers: Color32, mono: &FontId) -> TextFormat {
    let mut format = base.clone();
    if look.marker {
        format.color = markers;
    } else if look.mono {
        format.font_id = mono.clone();
    } else if look.bold {
        format.font_id = theme::bold(format.font_id.size);
    }
    format.italics |= look.italic;
    if look.strike {
        format.strikethrough = Stroke::new(1.0, format.color);
    }
    format
}

fn parse(text: &str, mentions: &[Mention]) -> Vec<Span> {
    spans(text, mentions, false)
}

/// A draft as typed: its spans hold every character of `text`, markers
/// included, and quote and list markers stay as they were typed.
fn draft(text: &str) -> Vec<Span> {
    spans(text, &[], true)
}

fn spans(text: &str, mentions: &[Mention], typed: bool) -> Vec<Span> {
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
            if typed {
                spans.push(Span::marker(line));
            }
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
        if let Some(rest) = content.strip_prefix("> ") {
            quote = true;
            content = rest;
            spans.push(Span {
                text: if typed { "> " } else { "▎ " }.to_owned(),
                quote: true,
                marker: typed,
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
                text: if typed {
                    content[..content.len() - rest.len()].to_owned()
                } else {
                    format!("{}•  ", &content[..indent])
                },
                quote,
                ..Default::default()
            });
            content = rest;
        }
        for mut span in inline(content) {
            span.quote = quote;
            if span.marker && !typed {
                continue;
            }
            if typed || span.mono {
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
            spans.push(Span::marker("```"));
            spans.push(Span {
                text: chars[i + 3..close].iter().collect(),
                mono: true,
                ..Default::default()
            });
            spans.push(Span::marker("```"));
            i = close + 3;
            continue;
        }
        if c == '`'
            && let Some(close) = chars[i + 1..].iter().position(|c| *c == '`')
        {
            let close = i + 1 + close;
            if close > i + 1 {
                flush(&mut run, &mut spans, flags);
                spans.push(Span::marker("`"));
                spans.push(Span {
                    text: chars[i + 1..close].iter().collect(),
                    mono: true,
                    ..Default::default()
                });
                spans.push(Span::marker("`"));
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
                    spans.push(Span::marker(c));
                    flags[index] = false;
                    i += 1;
                    continue;
                }
            } else if is_opener(&chars, i) && has_closer(&chars, i + 1, c) {
                flush(&mut run, &mut spans, flags);
                spans.push(Span::marker(c));
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

    fn draft_kinds(text: &str) -> Vec<(String, bool, bool, bool, bool, bool)> {
        draft(text)
            .into_iter()
            .map(|span| {
                let Span {
                    text,
                    marker,
                    bold,
                    italic,
                    strike,
                    mono,
                    ..
                } = span;
                (text, marker, bold, italic, strike, mono)
            })
            .collect()
    }

    #[test]
    fn a_draft_keeps_every_character() {
        for text in [
            "a *bold* and _it_ and ~no~ and `code`",
            "inline ```mono``` text",
            "*_both_* ~*nested*~",
            "```\nlet x = 1;\n  ```  \nafter",
            "```\nunclosed block",
            "> *quoted*\n- one\n  * two\n• three",
            "2 * 3 = 6, snake_case_name, *unclosed, a**b*",
            "café *crème* ~مرحبا~ 👍🏽 *שלום*",
            "visit https://example.com/a_b_c and mail me@example.com",
            "",
            "\n\n",
        ] {
            let spans = draft(text);
            let kept: String = spans.iter().map(|span| span.text.as_str()).collect();
            assert_eq!(kept, text);
        }
    }

    #[test]
    fn a_draft_styles_what_its_message_styles() {
        // Quotes and lists are left out: a message redraws their markers.
        for text in [
            "a *bold* and _it_ and ~no~ and `code`",
            "inline ```mono``` text",
            "*_both_* ~*nested*~",
            "```\nlet x = 1;\n```\nafter",
            "2 * 3 = 6, snake_case_name, *unclosed, a**b*",
            "café *crème* ~مرحبا~ 👍🏽",
        ] {
            let shown: Vec<_> = parse(text, &[])
                .into_iter()
                .flat_map(|span| {
                    let style = (span.bold, span.italic, span.strike, span.mono);
                    span.text
                        .chars()
                        .map(move |c| (c, style))
                        .collect::<Vec<_>>()
                })
                .collect();
            let typed: Vec<_> = draft(text)
                .into_iter()
                .filter(|span| !span.marker)
                .flat_map(|span| {
                    let style = (span.bold, span.italic, span.strike, span.mono);
                    span.text
                        .chars()
                        .map(move |c| (c, style))
                        .collect::<Vec<_>>()
                })
                .collect();
            assert_eq!(typed, shown, "{text}");
        }
    }

    #[test]
    fn a_draft_marks_its_markers() {
        assert_eq!(
            draft_kinds("a *b* ~c~ `d`"),
            vec![
                ("a ".into(), false, false, false, false, false),
                ("*".into(), true, false, false, false, false),
                ("b".into(), false, true, false, false, false),
                ("*".into(), true, false, false, false, false),
                (" ".into(), false, false, false, false, false),
                ("~".into(), true, false, false, false, false),
                ("c".into(), false, false, false, true, false),
                ("~".into(), true, false, false, false, false),
                (" ".into(), false, false, false, false, false),
                ("`".into(), true, false, false, false, false),
                ("d".into(), false, false, false, false, true),
                ("`".into(), true, false, false, false, false),
            ]
        );
        // A closing marker followed by the same character as text.
        assert_eq!(
            draft_kinds("*a**"),
            vec![
                ("*".into(), true, false, false, false, false),
                ("a".into(), false, true, false, false, false),
                ("*".into(), true, false, false, false, false),
                ("*".into(), false, false, false, false, false),
            ]
        );
        // A quote's marker is markup; a list's bullet is text.
        let quoted = draft_kinds("> - item");
        assert_eq!(quoted[0], ("> ".into(), true, false, false, false, false));
        assert_eq!(quoted[1], ("- ".into(), false, false, false, false, false));
        // Fences are markup; the block between them is monospace.
        let block = draft_kinds("```\ncode\n```");
        assert_eq!(block[0], ("```".into(), true, false, false, false, false));
        assert_eq!(block[2], ("code".into(), false, false, false, false, true));
        assert_eq!(block[4], ("```".into(), true, false, false, false, false));
    }

    #[test]
    fn styling_a_draft_keeps_its_text_and_leaves_emoji_alone() {
        let format = TextFormat::simple(theme::regular(15.0), Color32::WHITE);
        let hidden = TextFormat {
            color: Color32::TRANSPARENT,
            ..format.clone()
        };
        let markers = Color32::GRAY;
        // As the editor job lays it out: emoji in their own transparent sections.
        let mut job = LayoutJob::default();
        job.append("x *b", 0.0, format.clone());
        job.append("👍", 0.0, hidden.clone());
        job.append("ld* ~gone~ _it_ `m`", 0.0, format.clone());
        let text = job.text.clone();
        let mono = FontId::monospace(16.0);
        style_draft(&mut job, markers, &mono);
        assert_eq!(job.text, text);
        let mut at = 0;
        for section in &job.sections {
            assert_eq!(section.byte_range.start.0, at, "sections tile the text");
            assert!(section.byte_range.end.0 > at, "no empty sections here");
            at = section.byte_range.end.0;
        }
        assert_eq!(at, text.len());
        let format_at = |needle: &str| {
            let start = text.find(needle).unwrap();
            job.sections
                .iter()
                .find(|section| section.byte_range.as_usize().contains(&start))
                .unwrap()
                .format
                .clone()
        };
        assert_eq!(format_at("x ").font_id, theme::regular(15.0));
        assert_eq!(format_at("x ").color, Color32::WHITE);
        assert_eq!(format_at("*b").color, markers);
        assert_eq!(format_at("b👍").font_id, theme::bold(15.0));
        assert_eq!(format_at("👍"), hidden);
        assert_eq!(format_at("ld*").font_id, theme::bold(15.0));
        assert_eq!(format_at("* ~").color, markers);
        assert_eq!(
            format_at("gone").strikethrough,
            Stroke::new(1.0, Color32::WHITE)
        );
        assert!(format_at("it_").italics);
        assert_eq!(format_at("m`").font_id, mono);
        assert_eq!(format_at("`m").color, markers);
    }

    #[test]
    fn a_draft_without_markup_keeps_its_sections() {
        let format = TextFormat::simple(theme::regular(15.0), Color32::WHITE);
        let mut job = LayoutJob::single_section("2 * 3 = 6".to_owned(), format);
        let before = job.sections.clone();
        style_draft(&mut job, Color32::GRAY, &FontId::monospace(15.0));
        assert_eq!(job.sections, before);
    }

    #[test]
    fn a_keycap_emoji_stays_whole() {
        // The keycap's own asterisk reads as an opener, as it does in a message.
        let format = TextFormat::simple(theme::regular(15.0), Color32::WHITE);
        let hidden = TextFormat {
            color: Color32::TRANSPARENT,
            ..format.clone()
        };
        let keycap = "*\u{fe0f}\u{20e3}";
        let mut job = LayoutJob::default();
        job.append(keycap, 0.0, hidden.clone());
        job.append(" call me*", 0.0, format);
        style_draft(&mut job, Color32::GRAY, &FontId::monospace(15.0));
        assert_eq!(job.sections[0].byte_range.as_usize(), 0..keycap.len());
        assert_eq!(job.sections[0].format, hidden);
    }

    #[test]
    fn each_draft_is_styled_by_its_own_text() {
        let format = TextFormat::simple(theme::regular(15.0), Color32::WHITE);
        let mono = FontId::monospace(15.0);
        let styled = |text: &str| {
            let mut job = LayoutJob::single_section(text.to_owned(), format.clone());
            style_draft(&mut job, Color32::GRAY, &mono);
            job.sections.len()
        };
        assert_eq!(styled("a *b* c"), 5);
        assert_eq!(styled("a *b* c"), 5);
        assert_eq!(styled("plain"), 1);
        assert_eq!(styled("a *b* c"), 5);
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
    fn greater_than_without_a_space_stays_literal() {
        for text in [">.<", ">", ">quoted", ">= 3", ">\ttext", "hello\n>.<"] {
            assert_eq!(plain(text, &[]), text);
            assert!(parse(text, &[]).iter().all(|span| !span.quote));
        }
    }

    #[test]
    fn a_quote_requires_a_greater_than_and_space_at_line_start() {
        let spans = parse("> *quoted*\n> >.<\n> ", &[]);
        assert_eq!(plain("> *quoted*\n> >.<\n> ", &[]), "▎ quoted\n▎ >.<\n▎ ");
        assert!(
            spans
                .iter()
                .any(|span| span.quote && span.bold && span.text == "quoted")
        );
        assert!(spans.iter().any(|span| span.quote && span.text == ">.<"));
        for text in ["text > quoted", " > quoted"] {
            assert_eq!(plain(text, &[]), text);
            assert!(parse(text, &[]).iter().all(|span| !span.quote));
        }
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

    #[test]
    fn web_links_are_listed_once_without_mail_addresses() {
        assert_eq!(
            links("see https://a.b/c, then spotifast.rocks and https://a.b/c again"),
            vec!["https://a.b/c", "https://spotifast.rocks"]
        );
        assert!(links("mail hello@section8berlin.com or nothing").is_empty());
        assert!(links("version 0.3.0 of main.rs").is_empty());
    }
}
