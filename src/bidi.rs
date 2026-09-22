//! Unicode bidirectional layout for egui galleys.
//!
//! egui 0.36 shapes each font run on its own. A Hebrew or Arabic run is
//! shaped right to left, then the runs are placed in logical order. Neutrals
//! (spaces, numbers, punctuation, emoji) stay where that left-to-right
//! placement put them, and a paragraph that starts with Latin never repairs
//! the Hebrew inside it.
//!
//! After line breaking, this module resolves one Unicode bidi line per row,
//! moves shaped runs into visual order, and keeps the glyph vector in logical
//! order so copy, links, and carets use character indices. Glyph positions are
//! the visual ones. epaint hit-testing reads the `rtl` flag set here.

use std::ops::Range;
use std::sync::Arc;

use egui::epaint::text::{Galley, Glyph, TextFormat};
use egui::epaint::{Mesh, Rect, Vec2};
use icu_properties::{CodePointMapData, props::BidiClass};
use unicode_bidi::BidiInfo;

/// Lays out `job` and reorders each line with the Unicode bidi algorithm.
pub fn layout_job(ui: &egui::Ui, job: egui::text::LayoutJob) -> Arc<Galley> {
    let mut galley = ui.painter().layout_job(job);
    reorder_rtl_runs(Arc::make_mut(&mut galley));
    galley
}

/// Lays out editor text without changing the logical buffer.
///
/// Emoji stay in the text so character offsets match the buffer. The returned
/// clusters are the ones the caller paints over the transparent glyphs.
pub fn layout_editor(
    ui: &egui::Ui,
    text: &str,
    format: &TextFormat,
    wrap: f32,
    multiline: bool,
) -> (Arc<Galley>, Vec<(usize, usize, String)>) {
    let (mut job, clusters) = crate::emoji::editor_job(text, format);
    job.wrap.max_width = wrap;
    job.break_on_newline = multiline;
    job.keep_trailing_whitespace = true;
    if !multiline {
        let line_height = ui.fonts_mut(|fonts| fonts.row_height(&format.font_id));
        for section in &mut job.sections {
            section.format.line_height = Some(line_height);
        }
    }
    let mut galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
    reorder_rtl_runs(Arc::make_mut(&mut galley));
    (galley, clusters)
}

/// Single-line field galley. Emoji stay visible; the logical buffer is unchanged.
pub fn layout_field(ui: &egui::Ui, text: &str, format: &TextFormat, wrap: f32) -> Arc<Galley> {
    let mut format = format.clone();
    let line_height = ui.fonts_mut(|fonts| fonts.row_height(&format.font_id));
    format.line_height = Some(line_height);
    let mut job = egui::text::LayoutJob::single_section(text.to_owned(), format);
    job.wrap.max_width = wrap;
    job.break_on_newline = false;
    job.keep_trailing_whitespace = true;
    let mut galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
    reorder_rtl_runs(Arc::make_mut(&mut galley));
    galley
}

/// Visual bounds of the glyphs covering the logical character range.
///
/// The start caret of a right-to-left cluster is to the right of the end
/// caret, so callers must not assume the first cursor is the left edge.
pub fn char_bounds(galley: &Galley, start: usize, end: usize) -> Option<Rect> {
    let mut rect: Option<Rect> = None;
    let mut index = 0usize;
    for placed in &galley.rows {
        for glyph in &placed.row.glyphs {
            if index >= start && index < end && glyph.advance_width > 0.01 {
                let glyph_rect = glyph.logical_rect().translate(placed.pos.to_vec2());
                rect = Some(match rect {
                    Some(rect) => rect.union(glyph_rect),
                    None => glyph_rect,
                });
            }
            index += 1;
        }
        if placed.ends_with_newline {
            index += 1;
        }
    }
    rect
}

/// Grows every row on the left so a footer can sit beside a right-aligned last line.
pub fn reserve_leading(galley: &mut Galley, reserve: f32) {
    if reserve <= 0.0 {
        return;
    }
    let Some(last) = galley.rows.last() else {
        return;
    };
    let last_left = content_left(&last.row.glyphs);
    let gap = if last_left.is_finite() {
        last_left
    } else {
        0.0
    };
    if gap + 0.5 >= reserve {
        return;
    }
    let extra = reserve - gap;
    for placed in &mut galley.rows {
        let row = Arc::make_mut(&mut placed.row);
        shift_all(row, extra);
        row.size.x += extra;
    }
    refresh_bounds(galley);
}

/// Whether the first paragraph's base direction is right to left.
pub fn base_rtl(text: &str) -> bool {
    let info = BidiInfo::new(text, None);
    info.paragraphs
        .first()
        .is_some_and(|paragraph| paragraph.level.is_rtl())
}

/// Base direction of the last paragraph, for the message footer.
pub fn last_base_rtl(text: &str) -> bool {
    text.split('\n').next_back().is_some_and(base_rtl)
}

/// Places each line in visual order and records logical caret direction.
pub fn reorder_rtl_runs(galley: &mut Galley) {
    if !galley.text().chars().any(is_strong_rtl) {
        return;
    }
    let text = galley.job.text.clone();
    let overflow = galley.job.wrap.overflow_character;
    for placed in &mut galley.rows {
        let row = Arc::make_mut(&mut placed.row);
        reorder_row(row, &text, overflow);
    }
    align_rtl_paragraphs(galley, &text);
    refresh_bounds(galley);
}

fn reorder_row(row: &mut egui::epaint::text::Row, text: &str, overflow: Option<char>) {
    if row.glyphs.is_empty() || already_visual(&row.glyphs) {
        return;
    }
    let Some((start, end)) = line_span(&row.glyphs, text) else {
        return;
    };
    let line = &text[start..end];
    let info = BidiInfo::new(line, None);
    let Some(paragraph) = info.paragraphs.first() else {
        return;
    };
    let levels = info.reordered_levels_per_char(paragraph, 0..line.len());
    let visual_of_logical = visual_indices(&levels);
    let char_at_byte = char_starts(line);

    let visual_keys: Vec<usize> = row
        .glyphs
        .iter()
        .map(|glyph| {
            if is_overflow_replacement(glyph, text, overflow) {
                return usize::MAX;
            }
            let byte = glyph.cluster as usize;
            byte.checked_sub(start)
                .and_then(|offset| char_at_byte.get(offset).copied())
                .map(|logical| visual_of_logical.get(logical).copied().unwrap_or(logical))
                .unwrap_or(usize::MAX)
        })
        .collect();
    let replacement: Vec<usize> = row
        .glyphs
        .iter()
        .enumerate()
        .filter(|(_, glyph)| is_overflow_replacement(glyph, text, overflow))
        .map(|(index, _)| index)
        .collect();
    let atoms = split_atoms(&row.glyphs, &visual_keys, &replacement);
    let mut placed: Vec<AtomPlace> = atoms
        .into_iter()
        .filter_map(|atom| {
            let glyphs = &row.glyphs[atom.clone()];
            let key = glyphs
                .iter()
                .filter_map(|glyph| {
                    let byte = glyph.cluster as usize;
                    if byte < start {
                        return None;
                    }
                    char_at_byte.get(byte - start).copied()
                })
                .map(|logical| visual_of_logical.get(logical).copied().unwrap_or(logical))
                .min()?;
            let min_x = glyphs
                .iter()
                .map(|glyph| glyph.pos.x)
                .fold(f32::INFINITY, f32::min);
            let max_x = glyphs
                .iter()
                .map(Glyph::max_x)
                .fold(f32::NEG_INFINITY, f32::max);
            (min_x.is_finite() && max_x.is_finite()).then_some(AtomPlace {
                glyphs: atom,
                key,
                min_x,
                width: (max_x - min_x).max(0.0),
            })
        })
        .collect();
    placed.sort_by(|a, b| a.key.cmp(&b.key).then(a.glyphs.start.cmp(&b.glyphs.start)));

    let glyph_vertices = row.visuals.glyph_vertex_range.clone();
    let mut cursor = 0.0f32;
    let mut deco: Vec<(f32, f32, f32)> = Vec::new();
    for atom in &placed {
        let delta = cursor - atom.min_x;
        if delta.abs() > 0.01 {
            for glyph in &mut row.glyphs[atom.glyphs.clone()] {
                glyph.pos.x += delta;
                shift_glyph_mesh(&mut row.visuals.mesh, glyph, egui::vec2(delta, 0.0));
            }
            deco.push((atom.min_x, atom.min_x + atom.width, delta));
        }
        cursor += atom.width;
    }
    if paragraph.level.is_rtl() && !replacement.is_empty() {
        let extra: f32 = replacement
            .iter()
            .map(|&index| row.glyphs[index].advance_width)
            .sum();
        if extra > 0.01 {
            for item in &mut deco {
                item.2 += extra;
            }
            for (index, glyph) in row.glyphs.iter_mut().enumerate() {
                if replacement.contains(&index) {
                    continue;
                }
                glyph.pos.x += extra;
                shift_glyph_mesh(&mut row.visuals.mesh, glyph, egui::vec2(extra, 0.0));
            }
            let mut pen = 0.0f32;
            for index in replacement {
                let glyph = &mut row.glyphs[index];
                let delta = pen - glyph.pos.x;
                glyph.pos.x = pen;
                shift_glyph_mesh(&mut row.visuals.mesh, glyph, egui::vec2(delta, 0.0));
                pen += glyph.advance_width;
            }
            cursor += extra;
        }
    }
    shift_decorations(&mut row.visuals.mesh, &glyph_vertices, &deco);

    for glyph in &mut row.glyphs {
        let byte = glyph.cluster as usize;
        let logical = byte
            .checked_sub(start)
            .and_then(|offset| char_at_byte.get(offset).copied());
        glyph.rtl = logical
            .and_then(|index| levels.get(index))
            .is_some_and(|level| level.is_rtl());
    }

    let mut order: Vec<usize> = (0..row.glyphs.len()).collect();
    order.sort_by(|&left, &right| {
        row.glyphs[left]
            .cluster
            .cmp(&row.glyphs[right].cluster)
            .then(left.cmp(&right))
    });
    let glyphs = std::mem::take(&mut row.glyphs);
    row.glyphs = order.into_iter().map(|index| glyphs[index]).collect();
    repack_glyph_vertices(row);
    let content_right = row.glyphs.iter().map(Glyph::max_x).fold(0.0_f32, f32::max);
    row.size.x = row.size.x.max(content_right).max(cursor);
}

/// The overflow glyph epaint appends when a line is cut short.
///
/// Its cluster still belongs to a removed character, so it must not follow that
/// character's bidi position. A typed ellipsis keeps its own cluster and stays.
fn is_overflow_replacement(glyph: &Glyph, text: &str, overflow: Option<char>) -> bool {
    let Some(mark) = overflow else {
        return false;
    };
    if glyph.chr != mark {
        return false;
    }
    let byte = glyph.cluster as usize;
    text.get(byte..).and_then(|rest| rest.chars().next()) != Some(mark)
}

struct AtomPlace {
    glyphs: Range<usize>,
    key: usize,
    min_x: f32,
    width: f32,
}

/// True when a previous pass already stored logical order and bidi levels.
fn already_visual(glyphs: &[Glyph]) -> bool {
    glyphs.iter().any(|glyph| glyph.rtl)
        && glyphs
            .windows(2)
            .all(|pair| pair[0].cluster <= pair[1].cluster)
}

fn line_span(glyphs: &[Glyph], text: &str) -> Option<(usize, usize)> {
    let start = glyphs.iter().map(|glyph| glyph.cluster as usize).min()?;
    if start > text.len() {
        return None;
    }
    let mut end = start;
    for glyph in glyphs {
        let byte = glyph.cluster as usize;
        let next = text.get(byte..)?.chars().next()?.len_utf8();
        end = end.max(byte + next);
    }
    if end > text.len() {
        return None;
    }
    Some((start, end))
}

fn char_starts(text: &str) -> Vec<usize> {
    let mut starts = vec![0; text.len() + 1];
    for (index, (byte, _)) in text.char_indices().enumerate() {
        starts[byte] = index;
    }
    starts
}

fn visual_indices(levels: &[unicode_bidi::Level]) -> Vec<usize> {
    let map = BidiInfo::reorder_visual(levels);
    let mut logical_to_visual = vec![0; map.len()];
    for (visual, &logical) in map.iter().enumerate() {
        if logical < logical_to_visual.len() {
            logical_to_visual[logical] = visual;
        }
    }
    logical_to_visual
}

/// Shaped runs to move as a block.
///
/// A descending cluster sequence is one font run that harfrust already shaped
/// right to left. Splitting it would reverse Arabic letters a second time.
/// A single Hebrew letter has no descent, so it stays its own run and can
/// still move relative to the neighbouring space.
fn split_atoms(glyphs: &[Glyph], visual_keys: &[usize], skip: &[usize]) -> Vec<Range<usize>> {
    let mut atoms = Vec::new();
    let mut index = 0;
    while index < glyphs.len() {
        if skip.contains(&index) {
            index += 1;
            continue;
        }
        if let Some(end) = rtl_run_end(glyphs, index) {
            atoms.push(index..end);
            index = end;
            continue;
        }
        if is_strong_rtl(glyphs[index].chr) {
            let end = same_cluster_end(glyphs, index);
            atoms.push(index..end);
            index = end;
            continue;
        }
        // Keep a left-to-right run together only while its visual order matches
        // the buffer. A space before a number is earlier in the buffer and later
        // on screen, so it has to move on its own.
        let start = index;
        let mut previous = visual_keys.get(index).copied().unwrap_or(usize::MAX);
        index += 1;
        while index < glyphs.len()
            && rtl_run_end(glyphs, index).is_none()
            && !is_strong_rtl(glyphs[index].chr)
        {
            let key = visual_keys.get(index).copied().unwrap_or(usize::MAX);
            if key < previous {
                break;
            }
            previous = key;
            index += 1;
        }
        atoms.push(start..index);
    }
    atoms
}

fn same_cluster_end(glyphs: &[Glyph], start: usize) -> usize {
    let mut end = start + 1;
    while end < glyphs.len() && glyphs[end].cluster == glyphs[start].cluster {
        end += 1;
    }
    end
}

fn rtl_run_end(glyphs: &[Glyph], start: usize) -> Option<usize> {
    let mut end = same_cluster_end(glyphs, start);
    if end >= glyphs.len() || glyphs[end].cluster >= glyphs[end - 1].cluster {
        return None;
    }
    while end < glyphs.len() && glyphs[end].cluster < glyphs[end - 1].cluster {
        end = same_cluster_end(glyphs, end);
    }
    Some(end)
}

fn align_rtl_paragraphs(galley: &mut Galley, text: &str) {
    // Align the ink, not `row.size.x`. Shaping can leave the row box a fraction
    // of a pixel wider than the last glyph, and that slack depends on the font.
    let width = galley
        .rows
        .iter()
        .map(|placed| content_right(&placed.row.glyphs))
        .fold(0.0_f32, f32::max);
    if width <= 0.0 {
        return;
    }
    let paragraphs = paragraph_slices(text);
    let mut paragraph = 0usize;
    for index in 0..galley.rows.len() {
        let last = galley.rows[index].ends_with_newline || index + 1 == galley.rows.len();
        if paragraphs.get(paragraph).is_some_and(|text| base_rtl(text)) {
            let row = Arc::make_mut(&mut galley.rows[index].row);
            let delta = width - content_right(&row.glyphs);
            if delta > 0.01 {
                shift_all(row, delta);
                row.size.x = (row.size.x + delta).max(width);
            }
        }
        if last {
            paragraph += 1;
        }
    }
}

fn content_right(glyphs: &[Glyph]) -> f32 {
    glyphs.iter().map(Glyph::max_x).fold(0.0_f32, f32::max)
}

fn shift_all(row: &mut egui::epaint::text::Row, delta: f32) {
    for glyph in &mut row.glyphs {
        glyph.pos.x += delta;
        shift_glyph_mesh(&mut row.visuals.mesh, glyph, egui::vec2(delta, 0.0));
    }
    let glyphs = row.visuals.glyph_vertex_range.clone();
    for (index, vertex) in row.visuals.mesh.vertices.iter_mut().enumerate() {
        if !glyphs.contains(&index) {
            vertex.pos.x += delta;
        }
    }
}

fn content_left(glyphs: &[Glyph]) -> f32 {
    glyphs
        .iter()
        .filter(|glyph| glyph.advance_width > 0.01)
        .map(|glyph| glyph.pos.x)
        .fold(f32::INFINITY, f32::min)
}

fn refresh_bounds(galley: &mut Galley) {
    let mut rect: Option<Rect> = None;
    let mut mesh_bounds: Option<Rect> = None;
    for placed in &mut galley.rows {
        let row = Arc::make_mut(&mut placed.row);
        row.visuals.mesh_bounds = row.visuals.mesh.calc_bounds();
        let row_rect = Rect::from_min_size(placed.pos, row.size);
        rect = Some(match rect {
            Some(rect) => rect.union(row_rect),
            None => row_rect,
        });
        let moved = row.visuals.mesh_bounds.translate(placed.pos.to_vec2());
        mesh_bounds = Some(match mesh_bounds {
            Some(bounds) => bounds.union(moved),
            None => moved,
        });
    }
    if let Some(rect) = rect {
        galley.rect = rect;
    }
    if let Some(bounds) = mesh_bounds {
        galley.mesh_bounds = bounds;
    }
}

fn paragraph_slices(text: &str) -> Vec<&str> {
    if text.is_empty() {
        return vec![""];
    }
    let mut out = Vec::new();
    let mut start = 0;
    for (index, _) in text.match_indices('\n') {
        out.push(&text[start..index]);
        start = index + 1;
    }
    out.push(&text[start..]);
    out
}

fn shift_glyph_mesh(mesh: &mut Mesh, glyph: &Glyph, delta: Vec2) {
    if glyph.uv_rect.is_nothing() || delta == Vec2::ZERO {
        return;
    }
    let start = glyph.first_vertex as usize;
    let end = (start + 4).min(mesh.vertices.len());
    for vertex in &mut mesh.vertices[start..end] {
        vertex.pos += delta;
    }
}

fn shift_decorations(mesh: &mut Mesh, glyph_vertices: &Range<usize>, deltas: &[(f32, f32, f32)]) {
    if deltas.is_empty() {
        return;
    }
    let pad = 1.5;
    for (index, vertex) in mesh.vertices.iter_mut().enumerate() {
        if glyph_vertices.contains(&index) {
            continue;
        }
        let mut best: Option<(f32, f32)> = None;
        for &(old_min, old_max, delta_x) in deltas {
            if vertex.pos.x >= old_min - pad && vertex.pos.x <= old_max + pad {
                let mid = (old_min + old_max) * 0.5;
                let distance = (vertex.pos.x - mid).abs();
                if best.is_none_or(|(best_distance, _)| distance < best_distance) {
                    best = Some((distance, delta_x));
                }
            }
        }
        if let Some((_, delta_x)) = best {
            vertex.pos.x += delta_x;
        }
    }
}

/// Pack glyph quads into logical order so selection vertex ranges stay contiguous.
fn repack_glyph_vertices(row: &mut egui::epaint::text::Row) {
    let range = row.visuals.glyph_vertex_range.clone();
    if range.start > range.end || range.end > row.visuals.mesh.vertices.len() {
        return;
    }
    let mut packed = Vec::new();
    let mut remap = vec![u32::MAX; row.visuals.mesh.vertices.len()];
    for (index, slot) in remap.iter_mut().enumerate().take(range.start) {
        *slot = index as u32;
    }
    for glyph in &mut row.glyphs {
        let source = glyph.first_vertex as usize;
        let count = if glyph.uv_rect.is_nothing() { 0 } else { 4 };
        glyph.first_vertex = (range.start + packed.len()) as u32;
        if count == 0 || source.saturating_add(count) > row.visuals.mesh.vertices.len() {
            continue;
        }
        for offset in 0..count {
            remap[source + offset] = (range.start + packed.len()) as u32;
            packed.push(row.visuals.mesh.vertices[source + offset]);
        }
    }
    let old_len = range.end - range.start;
    let shift = packed.len() as i32 - old_len as i32;
    for (index, slot) in remap.iter_mut().enumerate().skip(range.end) {
        *slot = (index as i32 + shift) as u32;
    }
    let mut vertices =
        Vec::with_capacity(range.start + packed.len() + remap.len().saturating_sub(range.end));
    vertices.extend_from_slice(&row.visuals.mesh.vertices[..range.start]);
    vertices.extend(packed);
    vertices.extend_from_slice(&row.visuals.mesh.vertices[range.end..]);
    row.visuals.mesh.vertices = vertices;
    for index in &mut row.visuals.mesh.indices {
        if let Some(mapped) = remap.get(*index as usize)
            && *mapped != u32::MAX
        {
            *index = *mapped;
        }
    }
    let glyph_len = row
        .glyphs
        .iter()
        .map(|glyph| usize::from(!glyph.uv_rect.is_nothing()) * 4)
        .sum::<usize>();
    row.visuals.glyph_vertex_range = range.start..range.start + glyph_len;
}

fn is_strong_rtl(c: char) -> bool {
    matches!(
        CodePointMapData::<BidiClass>::new().get(c),
        BidiClass::RightToLeft | BidiClass::ArabicLetter
    )
}

/// Characters with Unicode bidi class R or AL.
pub fn is_rtl(c: char) -> bool {
    is_strong_rtl(c)
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::text::{FontData, FontDefinitions, FontFamily, LayoutJob};
    use egui::{Color32, FontId, Pos2, vec2};

    fn assert_visual(text: &str) {
        let galley = layout_fixed(text);
        let visual = visual_chars(&galley);
        let expected = uba_visual(text);
        assert_eq!(visual, expected, "logical {text:?}");
        assert_eq!(
            galley.rows[0].glyphs.len(),
            text.chars().count(),
            "glyph count must match the logical buffer for {text:?}"
        );
        assert_eq!(galley.text(), text);
        let again = {
            let mut clone = galley.clone();
            reorder_rtl_runs(&mut clone);
            visual_chars(&clone)
        };
        assert_eq!(again, visual, "reorder must be idempotent for {text:?}");
    }

    fn uba_visual(text: &str) -> String {
        let info = BidiInfo::new(text, None);
        let paragraph = &info.paragraphs[0];
        info.reorder_line(paragraph, 0..text.len()).into_owned()
    }

    fn visual_chars(galley: &Galley) -> String {
        let mut glyphs: Vec<&Glyph> = galley.rows[0]
            .glyphs
            .iter()
            .filter(|glyph| glyph.advance_width > 0.01)
            .collect();
        glyphs.sort_by(|a, b| a.pos.x.total_cmp(&b.pos.x));
        glyphs.into_iter().map(|glyph| glyph.chr).collect()
    }

    fn cursor_at(galley: &Galley, x: f32) -> usize {
        let y = galley.rows[0].rect().center().y;
        galley.cursor_from_pos(vec2(x, y)).index.0
    }

    #[test]
    fn confirmed_mixed_text_matches_the_bidi_algorithm() {
        for text in [
            "שלום!",
            "שלום 123",
            "שלום (עולם)",
            "Hello שלום עולם end",
            "אב גד בא",
            "הכלב הגדול קפץ",
            "OK הכלב end",
            "הכלב OK",
            "123 הכלב הגדול",
            "مرحبا بالعالم",
            "שלום https://example.com עולם",
            "שלום עולם",
            "שלום + עולם = ❤",
            "مرحبا بالعالم (123)",
        ] {
            assert_visual(text);
        }
    }

    #[test]
    fn clicking_the_visual_ends_uses_logical_offsets() {
        let text = "שלום עולם";
        let galley = layout_fixed(text);
        let right = galley.rows[0]
            .glyphs
            .iter()
            .map(Glyph::max_x)
            .fold(0.0_f32, f32::max);
        assert_eq!(
            cursor_at(&galley, right + 4.0),
            0,
            "visual right is the logical start"
        );
        assert_eq!(
            cursor_at(&galley, -4.0),
            text.chars().count(),
            "visual left is the logical end"
        );
        let at_start = egui::text::CCursor::new(0);
        let moved = galley.cursor_left_one_character(&at_start);
        assert!(
            moved.index.0 > at_start.index.0,
            "left arrow from the visual right walks into the text"
        );
    }

    #[test]
    fn a_url_inside_hebrew_keeps_its_logical_range() {
        let text = "שלום https://example.com עולם";
        let url = "https://example.com";
        let start = text.find(url).unwrap();
        let start_chars = text[..start].chars().count();
        let end_chars = start_chars + url.chars().count();
        let galley = layout_fixed(text);
        let mut left = f32::INFINITY;
        let mut right = f32::NEG_INFINITY;
        for (index, glyph) in galley.rows[0].glyphs.iter().enumerate() {
            if index >= start_chars && index < end_chars && glyph.advance_width > 0.01 {
                left = left.min(glyph.pos.x);
                right = right.max(glyph.max_x());
            }
        }
        let hit = cursor_at(&galley, (left + right) * 0.5);
        assert!(
            (start_chars..end_chars).contains(&hit),
            "hit {hit} outside {start_chars}..{end_chars}"
        );
    }

    #[test]
    fn rtl_lines_share_a_right_edge() {
        let galley = layout_fixed("הכלב הגדול קפץ\nקפץ");
        assert!(galley.rows.len() >= 2);
        let right = |row: usize| {
            galley.rows[row]
                .glyphs
                .iter()
                .map(Glyph::max_x)
                .fold(0.0_f32, f32::max)
        };
        assert!(
            (right(0) - right(1)).abs() < 1.0,
            "short line {} should meet the long line {}",
            right(1),
            right(0)
        );
    }

    #[test]
    fn hebrew_niqqud_stays_with_its_letter() {
        let galley = layout_fixed("שָׁלוֹם");
        let glyphs = &galley.rows[0].glyphs;
        assert_eq!(glyphs.len(), "שָׁלוֹם".chars().count());
        for glyph in glyphs {
            if glyph.advance_width > 0.01 {
                continue;
            }
            let on_letter = glyphs.iter().any(|base| {
                base.advance_width > 0.01
                    && glyph.pos.x >= base.pos.x - 0.5
                    && glyph.pos.x <= base.max_x() + 0.5
            });
            assert!(on_letter, "mark {:?} sits at {}", glyph.chr, glyph.pos.x);
        }
    }

    #[test]
    fn narrow_hebrew_ellipsis_sits_on_the_visual_left() {
        let ctx = egui::Context::default();
        install_fonts(&ctx);
        let galley = std::cell::RefCell::new(None);
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(200.0, 80.0))),
                ..Default::default()
            },
            |ui| {
                let mut job = LayoutJob::default();
                job.wrap.max_width = 36.0;
                job.wrap.max_rows = 1;
                job.wrap.break_anywhere = true;
                job.wrap.overflow_character = Some('…');
                job.append(
                    "הכלב הגדול קפץ מעל החתול",
                    0.0,
                    TextFormat::simple(FontId::proportional(14.0), Color32::WHITE),
                );
                *galley.borrow_mut() = Some(ui.painter().layout_job(job));
            },
        );
        output.textures_delta.clear();
        let mut galley = Arc::try_unwrap(galley.into_inner().expect("galley"))
            .unwrap_or_else(|arc| (*arc).clone());
        reorder_rtl_runs(&mut galley);
        let glyphs = &galley.rows[0].glyphs;
        let ellipsis = glyphs
            .iter()
            .find(|glyph| glyph.chr == '…')
            .expect("overflow ellipsis");
        let leftmost = glyphs
            .iter()
            .filter(|glyph| glyph.advance_width > 0.01)
            .map(|glyph| glyph.pos.x)
            .fold(f32::INFINITY, f32::min);
        assert!(
            (ellipsis.pos.x - leftmost).abs() < 1.0,
            "ellipsis at {} should be the left edge {leftmost}",
            ellipsis.pos.x
        );
    }

    #[test]
    fn first_strong_direction_uses_unicode_bidi_classes() {
        for text in ["Привет הכלב", "Καλημέρα הכלב", "你好 הכלב", "नमस्ते הכלב"]
        {
            assert!(!base_rtl(text), "{text}");
        }
        for text in [
            "× הכלב הגדול",
            "123 הכלב הגדול",
            "١٢٣ הכלב הגדול",
            "َ הכלב הגדול",
        ] {
            assert!(base_rtl(text), "{text}");
        }
    }

    #[test]
    fn struck_underline_mesh_moves_with_the_word() {
        let ctx = egui::Context::default();
        install_fonts(&ctx);
        let galley = std::cell::RefCell::new(None);
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(400.0, 120.0))),
                ..Default::default()
            },
            |ui| {
                let mut job = LayoutJob::default();
                let mut struck = TextFormat::simple(FontId::proportional(14.0), Color32::WHITE);
                struck.strikethrough = egui::Stroke::new(1.0, Color32::RED);
                struck.underline = egui::Stroke::new(1.0, Color32::GREEN);
                struck.background = Color32::from_gray(40);
                let plain = TextFormat::simple(FontId::proportional(14.0), Color32::WHITE);
                job.append("הכלב", 0.0, struck);
                job.append(" הגדול", 0.0, plain);
                *galley.borrow_mut() = Some(ui.painter().layout_job(job));
            },
        );
        output.textures_delta.clear();
        let mut galley = Arc::try_unwrap(galley.into_inner().expect("galley"))
            .unwrap_or_else(|arc| (*arc).clone());
        reorder_rtl_runs(&mut galley);
        let glyph_range = galley.rows[0].visuals.glyph_vertex_range.clone();
        let deco: Vec<f32> = galley.rows[0]
            .visuals
            .mesh
            .vertices
            .iter()
            .enumerate()
            .filter(|(index, _)| !glyph_range.contains(index))
            .map(|(_, vertex)| vertex.pos.x)
            .collect();
        assert!(!deco.is_empty(), "expected underline vertices");
        let dog_right = galley.rows[0]
            .glyphs
            .iter()
            .take(4)
            .map(Glyph::max_x)
            .fold(0.0_f32, f32::max);
        let deco_mid = deco.iter().copied().sum::<f32>() / deco.len() as f32;
        assert!(
            (deco_mid - dog_right).abs() < 40.0,
            "decorations should sit on the struck word, deco {deco_mid} word {dog_right}"
        );
    }

    fn layout_fixed(text: &str) -> Galley {
        let mut galley = layout_raw(text);
        reorder_rtl_runs(&mut galley);
        galley
    }

    fn layout_raw(text: &str) -> Galley {
        let ctx = egui::Context::default();
        install_fonts(&ctx);
        let galley = std::cell::RefCell::new(None);
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(480.0, 160.0))),
                ..Default::default()
            },
            |ui| {
                let mut job = LayoutJob::default();
                job.append(
                    text,
                    0.0,
                    TextFormat::simple(FontId::proportional(14.0), Color32::WHITE),
                );
                *galley.borrow_mut() = Some(ui.painter().layout_job(job));
            },
        );
        output.textures_delta.clear();
        let galley = galley.into_inner().expect("galley");
        Arc::try_unwrap(galley).unwrap_or_else(|arc| (*arc).clone())
    }

    fn install_fonts(ctx: &egui::Context) {
        // DejaVu, Liberation, and Arial stay ahead of the extra fallbacks so the
        // font that already satisfies these tests is unchanged when it is installed.
        // `ZAPFAST_TEST_RTL_FONT` overrides the search.
        const CANDIDATES: &[&str] = &[
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
            "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
            "/usr/share/fonts/liberation/LiberationSans-Regular.ttf",
            "/usr/share/fonts/truetype/noto/NotoSansArabic-Regular.ttf",
            "/usr/share/fonts/truetype/noto/NotoNaskhArabic-Regular.ttf",
            "/usr/share/fonts/truetype/noto/NotoSansHebrew-Regular.ttf",
            "/usr/share/fonts/TTF/DejaVuSans.ttf",
            "/System/Library/Fonts/Supplemental/Arial.ttf",
            "/Library/Fonts/Arial Unicode.ttf",
            r"C:\Windows\Fonts\arial.ttf",
            r"C:\Windows\Fonts\tahoma.ttf",
        ];
        let path = std::env::var_os("ZAPFAST_TEST_RTL_FONT")
            .map(std::path::PathBuf::from)
            .filter(|path| path.is_file())
            .or_else(|| {
                CANDIDATES
                    .iter()
                    .map(std::path::PathBuf::from)
                    .find(|path| path.is_file())
            })
            .expect(
                "set ZAPFAST_TEST_RTL_FONT or install a Hebrew/Arabic-capable sans \
                 (DejaVu, Liberation, Arial) for RTL layout tests",
            );
        let path = path.to_str().expect("utf-8 font path");
        let mut fonts = FontDefinitions::default();
        let inter = include_bytes!("../assets/fonts/InterVariable.ttf");
        fonts
            .font_data
            .insert("inter".into(), Arc::new(FontData::from_static(inter)));
        let face = std::fs::read(path).unwrap_or_else(|error| panic!("read {path}: {error}"));
        fonts
            .font_data
            .insert("rtl-fallback".into(), Arc::new(FontData::from_owned(face)));
        if (path.contains("DejaVu") || path.contains("Liberation"))
            && let Ok(arabic) =
                std::fs::read("/usr/share/fonts/truetype/noto/NotoNaskhArabic-Regular.ttf")
        {
            fonts
                .font_data
                .insert("arabic".into(), Arc::new(FontData::from_owned(arabic)));
            fonts.families.insert(
                FontFamily::Proportional,
                vec!["inter".into(), "rtl-fallback".into(), "arabic".into()],
            );
            ctx.set_fonts(fonts);
            return;
        }
        fonts.families.insert(
            FontFamily::Proportional,
            vec!["inter".into(), "rtl-fallback".into()],
        );
        fonts
            .families
            .insert(FontFamily::Monospace, vec!["inter".into()]);
        ctx.set_fonts(fonts);
    }
}
