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
        if let Some(rest) = content.strip_prefix("> ") {
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
    let byte_offsets: Vec<usize> = line.char_indices().map(|(offset, _)| offset).collect();
    let index = LineIndex::build(line, &chars, &byte_offsets);
    let mut spans = Vec::new();
    let mut run = String::new();
    // bold, italic, strike
    let mut flags = [false; 3];
    // Once a marker fails to find a closer with no other markers open, no later
    // opener of that marker with no other markers open can close either.
    let mut no_closer = [false; 3];
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
    while i < chars.len() {
        if let Some((start, end, after)) = index.mono[i] {
            flush(&mut run, &mut spans, flags);
            spans.push(Span {
                text: chars[start..end].iter().collect(),
                mono: true,
                ..Default::default()
            });
            i = after;
            continue;
        }
        // Keep formatting characters inside URLs and email addresses literal.
        if let Some(end) = index.link_end_at(line, &byte_offsets, i, flags) {
            run.extend(&chars[i..end]);
            i = end;
            continue;
        }
        let c = chars[i];
        if let Some(marker_idx) = marker_index(c) {
            if flags[marker_idx] {
                if is_closer(&chars, i) {
                    flush(&mut run, &mut spans, flags);
                    flags[marker_idx] = false;
                    i += 1;
                    continue;
                }
            } else if is_opener(&chars, i) && !(flags == [false; 3] && no_closer[marker_idx]) {
                let mut candidate = flags;
                candidate[marker_idx] = true;
                if has_closer(line, &chars, &byte_offsets, &index, i + 1, c, candidate) {
                    flush(&mut run, &mut spans, flags);
                    flags[marker_idx] = true;
                    i += 1;
                    continue;
                } else if flags == [false; 3] {
                    no_closer[marker_idx] = true;
                }
            }
        }
        run.push(c);
        i += 1;
    }
    flush(&mut run, &mut spans, flags);
    spans
}

fn marker_index(c: char) -> Option<usize> {
    match c {
        '*' => Some(0),
        '_' => Some(1),
        '~' => Some(2),
        _ => None,
    }
}

fn is_triple_backtick(chars: &[char], at: usize) -> bool {
    at + 2 < chars.len() && chars[at] == '`' && chars[at + 1] == '`' && chars[at + 2] == '`'
}

/// Precomputed inline monospace and link candidates for one line so `inline()`
/// and `has_closer()` stay linear even with many unmatched markers.
struct LineIndex {
    mono: Vec<Option<(usize, usize, usize)>>,
    link_raw_end: Vec<usize>,
}

impl LineIndex {
    fn build(line: &str, chars: &[char], byte_offsets: &[usize]) -> Self {
        let len = chars.len();
        let mut mono = vec![None; len];
        let mut no_triple_close = false;
        let mut no_single_close = false;
        let mut i = 0;
        while i < len {
            if !no_triple_close && is_triple_backtick(chars, i) {
                match (i + 3..len).find(|&at| is_triple_backtick(chars, at)) {
                    Some(close) if close > i + 3 => {
                        mono[i] = Some((i + 3, close, close + 3));
                        i = close + 3;
                        continue;
                    }
                    None => no_triple_close = true,
                    Some(_) => {}
                }
            }
            if !no_single_close && chars[i] == '`' {
                match chars[i + 1..].iter().position(|c| *c == '`') {
                    Some(offset) => {
                        let close = i + 1 + offset;
                        if close > i + 1 {
                            mono[i] = Some((i + 1, close, close + 1));
                            i = close + 1;
                            continue;
                        }
                    }
                    None => no_single_close = true,
                }
            }
            i += 1;
        }

        let mut next_delim = vec![line.len(); len];
        let mut next_slash = vec![usize::MAX; len];
        let mut next_at = vec![usize::MAX; len];
        let mut next_dot = vec![usize::MAX; len];
        let mut delim = line.len();
        let mut slash = usize::MAX;
        let mut at = usize::MAX;
        let mut dot = usize::MAX;
        for idx in (0..len).rev() {
            let c = chars[idx];
            let byte = byte_offsets[idx];
            if is_link_delimiter(c) {
                delim = byte;
                slash = usize::MAX;
                at = usize::MAX;
                dot = usize::MAX;
            } else {
                if c == '/' {
                    slash = byte;
                } else if c == '@' {
                    at = byte;
                } else if c == '.' {
                    dot = byte;
                }
            }
            next_delim[idx] = delim;
            next_slash[idx] = slash;
            next_at[idx] = at;
            next_dot[idx] = dot;
        }

        let mut link_raw_end = vec![0usize; len];
        let mut i = 0;
        while i < len {
            if let Some((_, _, after)) = mono[i] {
                i = after;
                continue;
            }
            let at_boundary = i == 0 || !chars[i - 1].is_alphanumeric();
            if at_boundary && chars[i].is_ascii_alphanumeric() {
                let start_byte = byte_offsets[i];
                let delim_byte = next_delim[i];
                let chunk = &line[start_byte..delim_byte];
                let maybe_scheme = starts_with_link_scheme(chunk);
                let maybe_email = next_at[i] < delim_byte
                    && next_at[i] > start_byte
                    && next_slash[i] >= delim_byte;
                let maybe_domain = if !maybe_scheme && !maybe_email {
                    let host_limit = next_slash[i].min(delim_byte);
                    next_dot[i] < host_limit && {
                        let host_chunk = &line[start_byte..host_limit];
                        let host_chunk = host_chunk
                            .split(['(', ')', '[', ']'])
                            .next()
                            .unwrap_or(host_chunk);
                        let mut host = host_chunk;
                        while let Some(stripped) = strip_trailing_link_punct(host, [true; 3]) {
                            host = stripped;
                        }
                        is_domain(host)
                    }
                } else {
                    false
                };
                if maybe_scheme || maybe_email || maybe_domain {
                    let raw_end_byte = link_token_end(line, start_byte);
                    if let Some((trimmed_end_byte, _)) =
                        classify_link_token(line, start_byte, raw_end_byte, [true; 3])
                    {
                        let raw_end_char = byte_offsets.binary_search(&raw_end_byte).unwrap_or(len);
                        let min_end_char =
                            byte_offsets.binary_search(&trimmed_end_byte).unwrap_or(len);
                        link_raw_end[i] = raw_end_char;
                        i = min_end_char.max(i + 1);
                        continue;
                    }
                }
            }
            i += 1;
        }

        Self { mono, link_raw_end }
    }

    fn link_end_at(
        &self,
        line: &str,
        byte_offsets: &[usize],
        i: usize,
        open_markers: [bool; 3],
    ) -> Option<usize> {
        let raw_end_char = self.link_raw_end[i];
        if raw_end_char == 0 {
            return None;
        }
        let start_byte = byte_offsets[i];
        let raw_end_byte = byte_offsets
            .get(raw_end_char)
            .copied()
            .unwrap_or(line.len());
        let (trimmed_end_byte, _) =
            classify_link_token(line, start_byte, raw_end_byte, open_markers)?;
        Some(
            byte_offsets
                .binary_search(&trimmed_end_byte)
                .unwrap_or(byte_offsets.len()),
        )
    }
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

fn has_closer(
    line: &str,
    chars: &[char],
    byte_offsets: &[usize],
    index: &LineIndex,
    from: usize,
    marker: char,
    mut open_markers: [bool; 3],
) -> bool {
    let mut j = from;
    while j < chars.len() {
        if let Some((_, _, after)) = index.mono[j] {
            j = after;
            continue;
        }
        if let Some(end) = index.link_end_at(line, byte_offsets, j, open_markers) {
            j = end;
            continue;
        }
        if chars[j] == marker && is_closer(chars, j) {
            return true;
        }
        if let Some(other) = marker_index(chars[j])
            && open_markers[other]
            && is_closer(chars, j)
        {
            open_markers[other] = false;
        }
        j += 1;
    }
    false
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

#[derive(Clone, Copy)]
enum LinkScheme {
    Exact,
    Https,
    Mailto,
}

fn is_link_delimiter(c: char) -> bool {
    c.is_whitespace() || matches!(c, '<' | '>' | '"' | '\'' | '{' | '}')
}

fn starts_with_ignore_ascii_case(text: &str, prefix: &str) -> bool {
    text.get(..prefix.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
}

fn starts_with_link_scheme(text: &str) -> bool {
    starts_with_ignore_ascii_case(text, "http://")
        || starts_with_ignore_ascii_case(text, "https://")
        || starts_with_ignore_ascii_case(text, "www.")
        || starts_with_ignore_ascii_case(text, "mailto:")
}

/// Scans the raw link token starting at `at`, keeping brackets opened inside
/// the address (such as `https://en.wikipedia.org/wiki/Rust_(programming_language)`)
/// and stopping before an unmatched closing bracket (such as `(see example.com)`).
fn link_token_end(text: &str, at: usize) -> usize {
    let rest = &text[at..];
    let (mut parens, mut squares) = (0usize, 0usize);
    let offset = rest
        .find(|c: char| {
            let depth = match c {
                '(' | ')' => &mut parens,
                '[' | ']' => &mut squares,
                _ => return is_link_delimiter(c),
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
    at + offset
}

/// Classifies `text[at..raw_end]` after trimming sentence punctuation and any
/// currently open inline markers (`*`, `_`, `~`) without allocating.
fn classify_link_token(
    text: &str,
    at: usize,
    raw_end: usize,
    open_markers: [bool; 3],
) -> Option<(usize, LinkScheme)> {
    let mut token = &text[at..raw_end];
    while let Some(stripped) = strip_trailing_link_punct(token, open_markers) {
        token = stripped;
    }
    if token.len() < 4 {
        return None;
    }
    let end = at + token.len();
    if starts_with_ignore_ascii_case(token, "http://")
        || starts_with_ignore_ascii_case(token, "https://")
    {
        return (token.len() > 8).then_some((end, LinkScheme::Exact));
    }
    if starts_with_ignore_ascii_case(token, "www.") {
        return Some((end, LinkScheme::Https));
    }
    if starts_with_ignore_ascii_case(token, "mailto:") {
        return Some((end, LinkScheme::Exact));
    }
    if let Some((user, host)) = token.split_once('@')
        && !user.is_empty()
        && !user.contains('/')
        && is_domain(host)
    {
        return Some((end, LinkScheme::Mailto));
    }
    let (host, _) = token.split_once('/').unwrap_or((token, ""));
    if is_domain(host) {
        return Some((end, LinkScheme::Https));
    }
    None
}

/// Parses a web or email address at `at` and returns its end and target.
fn link_at(text: &str, at: usize) -> Option<(usize, String)> {
    let raw_end = link_token_end(text, at);
    let (end, scheme) = classify_link_token(text, at, raw_end, [false; 3])?;
    let token = &text[at..end];
    let target = match scheme {
        LinkScheme::Exact => token.to_owned(),
        LinkScheme::Https => format!("https://{token}"),
        LinkScheme::Mailto => format!("mailto:{token}"),
    };
    Some((end, target))
}

fn strip_trailing_link_punct(token: &str, open_markers: [bool; 3]) -> Option<&str> {
    let last = token.chars().next_back()?;
    let strip = matches!(last, '.' | ',' | ';' | ':' | '!' | '?')
        || (open_markers[0] && last == '*')
        || (open_markers[1] && last == '_')
        || (open_markers[2] && last == '~');
    strip.then(|| &token[..token.len() - last.len_utf8()])
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
    fn addresses_keep_inline_formatting_characters() {
        let all_links = |text: &str| -> Vec<String> {
            parse(text, &[])
                .into_iter()
                .filter_map(|span| span.link)
                .collect()
        };

        // Issue #810 exact reproduction: tildes inside a URL path stay intact.
        let tilde_url = "https://example.com/test~~/token~/example";
        assert_eq!(plain(tilde_url, &[]), tilde_url);
        assert_eq!(all_links(tilde_url), vec![tilde_url]);
        let spans = parse(tilde_url, &[]);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].text, tilde_url);
        assert_eq!(spans[0].link.as_deref(), Some(tilde_url));
        assert!(!spans[0].strike);

        // Other inline markers inside URLs and email addresses also stay literal.
        for (input, expected_url) in [
            (
                "https://example.com/_foo_/bar",
                "https://example.com/_foo_/bar",
            ),
            (
                "https://example.com/*foo*/bar",
                "https://example.com/*foo*/bar",
            ),
            (
                "https://example.com/~user/a~b~c?x=_1_&y=*2*",
                "https://example.com/~user/a~b~c?x=_1_&y=*2*",
            ),
            ("https://example.com/token~", "https://example.com/token~"),
            ("https://example.com/path_", "https://example.com/path_"),
            (
                "example.com/test~~/token~/example",
                "https://example.com/test~~/token~/example",
            ),
            ("www.example.com/_a_/b", "https://www.example.com/_a_/b"),
            ("user._name_@example.com", "mailto:user._name_@example.com"),
        ] {
            assert_eq!(plain(input, &[]), input);
            assert_eq!(all_links(input), vec![expected_url]);
            let spans = parse(input, &[]);
            assert_eq!(spans.len(), 1, "expected single link span for {input}");
            assert!(!spans[0].bold && !spans[0].italic && !spans[0].strike);
        }

        // Non-ASCII prefix exercises UTF-8 byte/char offset mapping.
        let prefixed = format!("🚀 দেখো {tilde_url}");
        assert_eq!(plain(&prefixed, &[]), prefixed);
        assert_eq!(all_links(&prefixed), vec![tilde_url]);

        // Formatting markers wrapping a URL still apply and close cleanly.
        let struck = parse(&format!("~{tilde_url}~"), &[]);
        assert_eq!(struck.len(), 1);
        assert_eq!(struck[0].text, tilde_url);
        assert_eq!(struck[0].link.as_deref(), Some(tilde_url));
        assert!(struck[0].strike);

        let bold = parse("*https://example.com/*foo*/bar*", &[]);
        assert_eq!(bold.len(), 1);
        assert_eq!(bold[0].text, "https://example.com/*foo*/bar");
        assert_eq!(
            bold[0].link.as_deref(),
            Some("https://example.com/*foo*/bar")
        );
        assert!(bold[0].bold);

        let italic = parse("_https://example.com/_foo_/bar_", &[]);
        assert_eq!(italic.len(), 1);
        assert_eq!(italic[0].text, "https://example.com/_foo_/bar");
        assert_eq!(
            italic[0].link.as_deref(),
            Some("https://example.com/_foo_/bar")
        );
        assert!(italic[0].italic);

        let nested = parse("*_https://example.com_*", &[]);
        assert_eq!(nested.len(), 1);
        assert_eq!(nested[0].link.as_deref(), Some("https://example.com"));
        assert!(nested[0].bold && nested[0].italic);

        let with_period = parse("~see https://example.com/token~.", &[]);
        assert_eq!(
            all_links("~see https://example.com/token~."),
            vec!["https://example.com/token"]
        );
        assert_eq!(
            with_period.last().map(|s| (s.text.as_str(), s.strike)),
            Some((".", false))
        );

        // An unclosed marker before a URL must not match markers inside the URL.
        let unclosed = format!("~unclosed {tilde_url}");
        assert_eq!(plain(&unclosed, &[]), unclosed);
        assert_eq!(all_links(&unclosed), vec![tilde_url]);
        assert!(parse(&unclosed, &[]).iter().all(|span| !span.strike));

        let unclosed_bold = "*unclosed https://example.com/*foo*/bar";
        assert_eq!(plain(unclosed_bold, &[]), unclosed_bold);
        assert_eq!(
            all_links(unclosed_bold),
            vec!["https://example.com/*foo*/bar"]
        );
        assert!(parse(unclosed_bold, &[]).iter().all(|span| !span.bold));

        // Monospace URLs remain literal monospace and are not linkified.
        let mono = parse(&format!("`{tilde_url}`"), &[]);
        assert_eq!(mono.len(), 1);
        assert_eq!(mono[0].text, tilde_url);
        assert!(mono[0].mono);
        assert!(mono[0].link.is_none());

        // Closer lookahead tracks outer marker transitions so closing `*` does
        // not let an unclosed `_` strip `_*` off a later URL.
        let transition = "*bold _unclosed* https://example.com/path_*";
        let spans = parse(transition, &[]);
        assert_eq!(
            plain(transition, &[]),
            "bold _unclosed https://example.com/path_*"
        );
        assert_eq!(all_links(transition), vec!["https://example.com/path_*"]);
        assert!(spans.iter().all(|span| !span.italic));
        assert!(
            spans
                .iter()
                .any(|span| span.bold && span.text == "bold _unclosed")
        );
    }

    #[test]
    fn unmatched_markers_with_long_suffixes_stay_linear() {
        let repeated_underscore = "_a!".repeat(2_000);
        assert_eq!(plain(&repeated_underscore, &[]), repeated_underscore);

        let repeated_star_and_slashes = format!("{}{}", "*a ".repeat(1_000), "a/".repeat(5_000));
        assert_eq!(
            plain(&repeated_star_and_slashes, &[]),
            repeated_star_and_slashes
        );
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
