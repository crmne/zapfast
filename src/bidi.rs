//! Unicode bidirectional layout for egui galleys.
//!
//! The patched epaint splits font runs where the Unicode bidi level changes
//! and shapes each in its resolved direction, so brackets in right-to-left
//! runs are mirrored. It still places the shaped runs in logical order, left
//! to right.
//!
//! After line breaking, this module resolves each row with its paragraph's
//! bidi levels, moves shaped runs into visual order, and keeps the glyph
//! vector in logical order so copy, links, and carets use character indices.
//! Glyph positions are the visual ones. epaint hit-testing reads the `rtl`
//! flag set here.

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

/// Whether the first paragraph's base direction is right to left.
pub fn base_rtl(text: &str) -> bool {
    let info = BidiInfo::new(text, None);
    info.paragraphs
        .first()
        .is_some_and(|paragraph| paragraph.level.is_rtl())
}

/// Whether the first strong character, in any paragraph, is right to left.
///
/// This picks the side a multi-line message is aligned to.
pub fn message_rtl(text: &str) -> bool {
    unicode_bidi::get_base_direction_full(text) == unicode_bidi::Direction::Rtl
}

/// Places each line in visual order and records logical caret direction.
pub fn reorder_rtl_runs(galley: &mut Galley) {
    if !galley.text().chars().any(is_strong_rtl) {
        return;
    }
    // A later paint can widen the galley for selection. Running alignment and
    // bound refresh again would change that rect without moving any glyphs.
    if galley
        .rows
        .iter()
        .all(|placed| placed.row.glyphs.is_empty() || already_visual(&placed.row.glyphs))
    {
        return;
    }
    let text = galley.job.text.clone();
    let overflow = galley.job.wrap.overflow_character;
    let decorated: Vec<Range<usize>> = galley
        .job
        .sections
        .iter()
        .filter(|section| {
            let format = &section.format;
            !format.underline.is_empty()
                || !format.strikethrough.is_empty()
                || format.background != egui::Color32::TRANSPARENT
        })
        .map(|section| section.byte_range.start.0..section.byte_range.end.0)
        .collect();
    let mut paragraph = None;
    for placed in &mut galley.rows {
        let row = Arc::make_mut(&mut placed.row);
        reorder_row(row, &text, &decorated, &mut paragraph, overflow);
    }
    if message_rtl(&text) {
        align_right(galley);
    }
    refresh_bounds(galley);
}

/// The bidi paragraph last used, keyed by its byte offset in the galley text.
type ParagraphCache<'a> = Option<(usize, BidiInfo<'a>)>;

/// `decorated` holds the byte ranges of sections that draw an underline,
/// strikethrough, or background, which has to move with its glyphs.
fn reorder_row<'a>(
    row: &mut egui::epaint::text::Row,
    text: &'a str,
    decorated: &[Range<usize>],
    paragraph: &mut ParagraphCache<'a>,
    overflow: Option<char>,
) {
    if row.glyphs.is_empty() || already_visual(&row.glyphs) {
        return;
    }
    let Some((start, end)) = line_span(&row.glyphs, text) else {
        return;
    };
    let (levels, base_rtl, end) = line_levels(paragraph, text, start, end);
    let line = &text[start..end];
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
    let mut deco: Vec<Moved> = Vec::new();
    for atom in &placed {
        let delta = cursor - atom.min_x;
        if delta.abs() > 0.01 {
            for glyph in &mut row.glyphs[atom.glyphs.clone()] {
                glyph.pos.x += delta;
                shift_glyph_mesh(&mut row.visuals.mesh, glyph, egui::vec2(delta, 0.0));
            }
        }
        deco.push(Moved {
            min: atom.min_x,
            max: atom.min_x + atom.width,
            delta,
            decorated: row.glyphs[atom.glyphs.clone()].iter().any(|glyph| {
                decorated
                    .iter()
                    .any(|range| range.contains(&(glyph.cluster as usize)))
            }),
        });
        cursor += atom.width;
    }
    if base_rtl && !replacement.is_empty() {
        let extra: f32 = replacement
            .iter()
            .map(|&index| row.glyphs[index].advance_width)
            .sum();
        if extra > 0.01 {
            for item in &mut deco {
                item.delta += extra;
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

/// Levels of the characters in `text[start..end]`, whether their paragraph is
/// right to left, and the end clipped to that paragraph.
///
/// Levels come from the whole paragraph, so a wrapped row keeps the
/// paragraph's base direction instead of guessing one from its own first word.
fn line_levels<'a>(
    cache: &mut ParagraphCache<'a>,
    text: &'a str,
    start: usize,
    end: usize,
) -> (Vec<unicode_bidi::Level>, bool, usize) {
    let paragraph_start = text[..start].rfind('\n').map_or(0, |at| at + 1);
    let paragraph_end = text[start..].find('\n').map_or(text.len(), |at| start + at);
    let end = end.min(paragraph_end);
    if cache.as_ref().is_none_or(|(at, _)| *at != paragraph_start) {
        let info = BidiInfo::new(&text[paragraph_start..paragraph_end], None);
        *cache = Some((paragraph_start, info));
    }
    let Some((_, info)) = cache.as_ref() else {
        return (Vec::new(), false, end);
    };
    let line = start - paragraph_start..end - paragraph_start;
    let mut by_byte = vec![unicode_bidi::Level::ltr(); line.len()];
    let mut base_rtl = false;
    for paragraph in &info.paragraphs {
        let overlap = paragraph.range.start.max(line.start)..paragraph.range.end.min(line.end);
        if overlap.is_empty() {
            continue;
        }
        if paragraph.range.contains(&line.start) {
            base_rtl = paragraph.level.is_rtl();
        }
        let levels = info.reordered_levels(paragraph, overlap.clone());
        by_byte[overlap.start - line.start..overlap.end - line.start]
            .copy_from_slice(&levels[overlap]);
    }
    let levels = text[start..end]
        .char_indices()
        .map(|(byte, _)| by_byte[byte])
        .collect();
    (levels, base_rtl, end)
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

/// End of the shaped cluster starting at `start`.
///
/// A ligature such as لا or لى is one glyph followed by zero-width continuation
/// glyphs for the letters it absorbed. Those carry their own letters' byte
/// offsets, which are higher than the ligature's, so they belong to the
/// cluster before them rather than breaking a descending right-to-left run.
fn same_cluster_end(glyphs: &[Glyph], start: usize) -> usize {
    let mut end = start + 1;
    while end < glyphs.len()
        && (glyphs[end].cluster == glyphs[start].cluster || is_continuation(&glyphs[end]))
    {
        end += 1;
    }
    end
}

/// A zero-width stand-in epaint emits for a character its shaped cluster covers.
fn is_continuation(glyph: &Glyph) -> bool {
    glyph.advance_width == 0.0 && glyph.uv_rect.is_nothing()
}

fn rtl_run_end(glyphs: &[Glyph], start: usize) -> Option<usize> {
    let mut previous = start;
    let mut end = same_cluster_end(glyphs, start);
    if end >= glyphs.len() || glyphs[end].cluster >= glyphs[previous].cluster {
        return None;
    }
    while end < glyphs.len() && glyphs[end].cluster < glyphs[previous].cluster {
        previous = end;
        end = same_cluster_end(glyphs, end);
    }
    Some(end)
}

/// Right-aligns every row, including left-to-right paragraphs.
///
/// Official WhatsApp aligns a message to the side of its first strong
/// character while each paragraph keeps its own base direction, so an English
/// line inside a Hebrew message still reads left to right, flush right.
fn align_right(galley: &mut Galley) {
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
    for placed in &mut galley.rows {
        let row = Arc::make_mut(&mut placed.row);
        let delta = width - content_right(&row.glyphs);
        if delta > 0.01 {
            shift_all(row, delta);
            row.size.x = (row.size.x + delta).max(width);
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

/// Where an atom sat before reordering and how far it moved.
struct Moved {
    min: f32,
    max: f32,
    delta: f32,
    /// Whether its glyphs belong to a section that draws decorations.
    decorated: bool,
}

/// Moves underline, strikethrough, and background vertices with their atom.
///
/// A decoration's end vertex sits on the boundary it shares with the next
/// atom. An atom whose section draws decorations claims it before a plain one,
/// so a link's underline does not stretch under the space beside it.
fn shift_decorations(mesh: &mut Mesh, glyph_vertices: &Range<usize>, moved: &[Moved]) {
    if moved.iter().all(|atom| atom.delta.abs() <= 0.01) {
        return;
    }
    let pad = 1.5;
    for (index, vertex) in mesh.vertices.iter_mut().enumerate() {
        if glyph_vertices.contains(&index) {
            continue;
        }
        let mut best: Option<(bool, f32, f32)> = None;
        for atom in moved {
            if vertex.pos.x >= atom.min - pad && vertex.pos.x <= atom.max + pad {
                let distance = (vertex.pos.x - (atom.min + atom.max) * 0.5).abs();
                let better = best.is_none_or(|(decorated, best_distance, _)| {
                    (atom.decorated && !decorated)
                        || (atom.decorated == decorated && distance < best_distance)
                });
                if better {
                    best = Some((atom.decorated, distance, atom.delta));
                }
            }
        }
        if let Some((_, _, delta)) = best {
            vertex.pos.x += delta;
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

/// Checks every row against `unicode-bidi`'s reordered line for its paragraph.
///
/// The drawn glyphs, left to right, must spell the reordered line, less the
/// characters that draw nothing of their own. ASCII paired
/// brackets must also face their resolved direction: the glyph's ink in the font
/// atlas leans the way a mirrored or unmirrored bracket would.
#[cfg(test)]
pub(crate) fn assert_rows_follow_uba(galley: &Galley, atlas: &egui::ColorImage) {
    use unicode_bidi::BidiDataSource as _;
    let text = galley.text();
    // Rows hold one glyph per character in logical order, so the row's text
    // follows from glyph counts alone, independent of the cluster offsets.
    let byte_at: Vec<usize> = text
        .char_indices()
        .map(|(byte, _)| byte)
        .chain(std::iter::once(text.len()))
        .collect();
    let mut first_char = 0usize;
    for (index, placed) in galley.rows.iter().enumerate() {
        let glyphs = &placed.row.glyphs;
        let row_chars = first_char..first_char + glyphs.len();
        first_char = row_chars.end + usize::from(placed.ends_with_newline);
        if glyphs.is_empty() {
            continue;
        }
        let (start, end) = (byte_at[row_chars.start], byte_at[row_chars.end]);
        let paragraph_start = text[..start].rfind('\n').map_or(0, |at| at + 1);
        let paragraph_end = text[start..].find('\n').map_or(text.len(), |at| start + at);
        let paragraph = &text[paragraph_start..paragraph_end];
        let info = BidiInfo::new(paragraph, None);
        let line = start - paragraph_start..end.min(paragraph_end) - paragraph_start;
        let resolved = info
            .paragraphs
            .iter()
            .find(|candidate| candidate.range.contains(&line.start))
            .expect("bidi paragraph for the row");
        let levels = info.reordered_levels(resolved, line);
        let row_text: Vec<char> = text[start..end].chars().collect();
        let row_levels: Vec<unicode_bidi::Level> = (0..row_text.len())
            .map(|offset| levels[byte_at[row_chars.start + offset] - paragraph_start])
            .collect();
        // Marks, joiners, and the letters a ligature absorbs draw no glyph of
        // their own, so only characters whose glyph advances are compared.
        let expected: String = BidiInfo::reorder_visual(&row_levels)
            .into_iter()
            .filter(|&offset| glyphs[offset].advance_width > 0.01)
            .map(|offset| row_text[offset])
            .collect();
        let mut drawn: Vec<&Glyph> = glyphs
            .iter()
            .filter(|glyph| glyph.advance_width > 0.01)
            .collect();
        drawn.sort_by(|a, b| a.pos.x.total_cmp(&b.pos.x));
        let visual: String = drawn.iter().map(|glyph| glyph.chr).collect();
        assert_eq!(visual, expected, "row {index} of {paragraph:?}");
        for (offset, glyph) in glyphs.iter().enumerate() {
            let Some(bracket) =
                unicode_bidi::HardcodedBidiData.bidi_matched_opening_bracket(glyph.chr)
            else {
                continue;
            };
            if !glyph.chr.is_ascii() || glyph.uv_rect.is_nothing() {
                continue;
            }
            let rtl = levels[byte_at[row_chars.start + offset] - paragraph_start].is_rtl();
            // An opening bracket's ink sits left of centre; mirrored, it sits right.
            assert_eq!(
                ink_leans_right(atlas, glyph),
                bracket.is_open == rtl,
                "{:?} faces the wrong way in row {index} of {paragraph:?}",
                glyph.chr
            );
        }
    }
}

/// Whether the glyph's middle bulges right of its tips, as `)`, `]`, and `}` do.
#[cfg(test)]
fn ink_leans_right(atlas: &egui::ColorImage, glyph: &Glyph) -> bool {
    let [left, top] = glyph.uv_rect.min;
    let [right, bottom] = glyph.uv_rect.max;
    let height = bottom - top;
    assert!(height >= 4, "{:?} is too small to inspect", glyph.chr);
    let centre_of = |rows: &mut dyn Iterator<Item = u16>| {
        let mut mass = 0.0f32;
        let mut moment = 0.0f32;
        for y in rows {
            for x in left..right {
                let alpha = f32::from(atlas.pixels[y as usize * atlas.size[0] + x as usize].a());
                mass += alpha;
                moment += alpha * f32::from(x);
            }
        }
        assert!(mass > 0.0, "{:?} has no ink in the atlas", glyph.chr);
        moment / mass
    };
    let quarter = height / 4;
    let tips = centre_of(&mut (top..top + quarter).chain(bottom - quarter..bottom));
    let middle = centre_of(&mut (top + quarter..bottom - quarter));
    middle > tips
}

/// Every row's ink ends at the same right edge.
#[cfg(test)]
pub(crate) fn assert_right_aligned(galley: &Galley) {
    let edges: Vec<f32> = galley
        .rows
        .iter()
        .filter(|placed| !placed.row.glyphs.is_empty())
        .map(|placed| content_right(&placed.row.glyphs))
        .collect();
    let widest = edges.iter().copied().fold(0.0_f32, f32::max);
    for (index, edge) in edges.iter().enumerate() {
        assert!(
            (widest - edge).abs() < 1.0,
            "row {index} ends at {edge}, not the right edge {widest}"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::text::{FontData, FontDefinitions, FontFamily, LayoutJob};
    use egui::{Color32, FontId, Pos2, vec2};
    use std::path::PathBuf;

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
            // A mark the font merges into its letter's glyph becomes a
            // continuation at epaint's pen, which is rounded to whole pixels
            // while the letter keeps its fractional advance.
            let on_letter = glyphs.iter().any(|base| {
                base.advance_width > 0.01
                    && glyph.pos.x >= base.pos.x - 1.0
                    && glyph.pos.x <= base.max_x() + 1.0
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

    /// Distributions put the same fonts in different folders (Fedora uses
    /// `dejavu-sans-fonts/`, `liberation-sans/`, `gnu-free/`), so look for
    /// the known file names under the usual font roots.
    fn find_rtl_font() -> Option<PathBuf> {
        const NAMES: &[&str] = &[
            "DejaVuSans.ttf",
            "LiberationSans-Regular.ttf",
            "FreeSans.ttf",
            "FreeSans.otf",
        ];
        let mut roots = vec![
            PathBuf::from("/usr/share/fonts"),
            PathBuf::from("/usr/local/share/fonts"),
        ];
        if let Some(data) = std::env::var_os("XDG_DATA_DIRS") {
            roots.extend(std::env::split_paths(&data).map(|dir| dir.join("fonts")));
        }
        if let Some(home) = std::env::var_os("HOME") {
            roots.push(PathBuf::from(&home).join(".local/share/fonts"));
            roots.push(PathBuf::from(home).join(".fonts"));
        }
        fn search(dir: &std::path::Path, depth: usize, names: &[&str]) -> Option<PathBuf> {
            let mut subdirs = Vec::new();
            for entry in std::fs::read_dir(dir).ok()?.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    subdirs.push(path);
                } else if path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| names.contains(&name))
                {
                    return Some(path);
                }
            }
            if depth == 0 {
                return None;
            }
            subdirs.sort();
            subdirs
                .iter()
                .find_map(|subdir| search(subdir, depth - 1, names))
        }
        // Prefer the first name in the list across all roots.
        NAMES.iter().find_map(|name| {
            roots
                .iter()
                .find_map(|root| search(root, 3, std::slice::from_ref(name)))
        })
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

    #[test]
    fn arabic_lam_ligatures_keep_their_place() {
        let (galley, atlas) = bubble("إلى السطر التالي", 2000.0);
        assert_rows_follow_uba(&galley, &atlas);
    }

    #[test]
    fn ligatures_inside_right_to_left_words_keep_their_place() {
        for text in [
            "لا بأس",
            "سلام عليكم",
            "الاسم والعلامة",
            "إلى التالي\nفي الليل",
            "שלום עולם אב גד",
        ] {
            let (galley, atlas) = bubble(text, 2000.0);
            assert_rows_follow_uba(&galley, &atlas);
        }
        let (galley, atlas) = bubble("إلى السطر التالي إلى السطر التالي", 90.0);
        assert!(galley.rows.len() > 1, "the sample wraps");
        assert_rows_follow_uba(&galley, &atlas);
    }

    #[test]
    fn wrapped_right_to_left_paragraphs_keep_logical_row_order() {
        for text in [
            "הכלב הגדול קפץ מעל החתול והמשיך לרוץ לאורך הרחוב עד שהגיע אל הגינה השקטה",
            "هذا نص عربي طويل يختبر ترتيب الأسطر عندما تلتف الكلمات داخل فقاعة رسالة ضيقة",
        ] {
            let (galley, atlas) = bubble(text, 110.0);
            assert!(galley.rows.len() >= 3, "{text:?} should wrap narrowly");
            assert_rows_follow_uba(&galley, &atlas);

            let spans: Vec<(u32, u32)> = galley
                .rows
                .iter()
                .filter(|placed| !placed.row.glyphs.is_empty())
                .map(|placed| {
                    let clusters = placed.row.glyphs.iter().map(|glyph| glyph.cluster);
                    (
                        clusters.clone().min().expect("row starts"),
                        clusters.max().expect("row ends"),
                    )
                })
                .collect();
            assert_eq!(
                spans[0].0, 0,
                "the first visual row must contain the logical start of {text:?}: {spans:?}"
            );
            for rows in spans.windows(2) {
                assert!(
                    rows[0].1 < rows[1].0,
                    "wrapped rows must stay in logical order for {text:?}: {spans:?}"
                );
            }
        }
    }

    /// Lays `text` out as a message body with the app's own fonts.
    fn bubble(text: &str, width: f32) -> (Arc<Galley>, egui::ColorImage) {
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        let galley = std::cell::RefCell::new(None);
        // Fonts are installed at the start of the first pass.
        for _ in 0..2 {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))),
                    ..Default::default()
                },
                |ui| {
                    let style = crate::markup::Style {
                        size: 14.5,
                        color: Color32::WHITE,
                        secondary: Color32::GRAY,
                        link: Color32::LIGHT_BLUE,
                        mention: Color32::GREEN,
                    };
                    let laid = crate::markup::layout(ui, text, &[], &style, width);
                    *galley.borrow_mut() = Some(laid.galley);
                },
            );
            output.textures_delta.clear();
        }
        let atlas = ctx.fonts(|fonts| fonts.image());
        (galley.into_inner().expect("galley"), atlas)
    }

    #[test]
    fn message_bubbles_follow_the_bidi_algorithm_on_every_row() {
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        let galleys = std::cell::RefCell::new(Vec::new());
        // Fonts are installed at the start of the first pass.
        for _ in 0..2 {
            galleys.borrow_mut().clear();
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))),
                    ..Default::default()
                },
                |ui| {
                    let style = crate::markup::Style {
                        size: 14.5,
                        color: Color32::WHITE,
                        secondary: Color32::GRAY,
                        link: Color32::LIGHT_BLUE,
                        mention: Color32::GREEN,
                    };
                    for text in crate::demo::RTL_SELF_CHAT {
                        let laid = crate::markup::layout(ui, text, &[], &style, 400.0);
                        galleys.borrow_mut().push(laid.galley);
                    }
                },
            );
            output.textures_delta.clear();
        }
        let atlas = ctx.fonts(|fonts| fonts.image());
        for galley in galleys.into_inner() {
            assert!(galley.rows.len() > 1, "each sample is a multi-line message");
            assert_rows_follow_uba(&galley, &atlas);
            assert_right_aligned(&galley);
        }
    }

    #[test]
    fn a_link_underline_stays_under_the_link() {
        let ctx = egui::Context::default();
        install_fonts(&ctx);
        let galley = std::cell::RefCell::new(None);
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(480.0, 160.0))),
                ..Default::default()
            },
            |ui| {
                let plain = TextFormat::simple(FontId::proportional(14.0), Color32::WHITE);
                let mut link = plain.clone();
                link.underline = egui::Stroke::new(1.0, Color32::LIGHT_BLUE);
                let mut job = LayoutJob::default();
                job.append("שלום ", 0.0, plain.clone());
                job.append("https://example.com", 0.0, link);
                job.append(" עולם", 0.0, plain);
                *galley.borrow_mut() = Some(ui.painter().layout_job(job));
            },
        );
        output.textures_delta.clear();
        let mut galley = Arc::try_unwrap(galley.into_inner().expect("galley"))
            .unwrap_or_else(|arc| (*arc).clone());
        reorder_rtl_runs(&mut galley);
        let row = &galley.rows[0].row;
        let url = "שלום ".chars().count().."שלום https://example.com".chars().count();
        let left = row.glyphs[url.clone()]
            .iter()
            .map(|glyph| glyph.pos.x)
            .fold(f32::INFINITY, f32::min);
        let right = row.glyphs[url]
            .iter()
            .map(Glyph::max_x)
            .fold(f32::NEG_INFINITY, f32::max);
        let glyph_range = row.visuals.glyph_vertex_range.clone();
        let underline: Vec<f32> = row
            .visuals
            .mesh
            .vertices
            .iter()
            .enumerate()
            .filter(|(index, _)| !glyph_range.contains(index))
            .map(|(_, vertex)| vertex.pos.x)
            .collect();
        assert!(!underline.is_empty(), "expected underline vertices");
        // Feathering puts stroke vertices up to a pixel past the glyphs; the
        // neighbouring space is about four.
        for x in underline {
            assert!(
                (left - 1.5..=right + 1.5).contains(&x),
                "underline vertex at {x} outside the link {left}..{right}"
            );
        }
    }

    #[test]
    fn brackets_face_their_paragraph_direction() {
        for text in [
            "שלום (עולם)\n(Hello) [x] עולם",
            "Hello (שלום) end\nמחיר [50] ₪",
            "مرحبا (123)\n{שלום}",
        ] {
            let (galley, atlas) = layout_with_atlas(text);
            assert_rows_follow_uba(&galley, &atlas);
        }
    }

    fn layout_fixed(text: &str) -> Galley {
        layout_with_atlas(text).0
    }

    fn layout_with_atlas(text: &str) -> (Galley, egui::ColorImage) {
        let ctx = egui::Context::default();
        let mut galley = layout_raw(&ctx, text);
        reorder_rtl_runs(&mut galley);
        (galley, ctx.fonts(|fonts| fonts.image()))
    }

    fn layout_raw(ctx: &egui::Context, text: &str) -> Galley {
        install_fonts(ctx);
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
            "/usr/share/fonts/dejavu-sans-fonts/DejaVuSans.ttf",
            "/usr/share/fonts/liberation-sans/LiberationSans-Regular.ttf",
            "/usr/share/fonts/gnu-free/FreeSans.ttf",
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
            .or_else(find_rtl_font)
            .expect(
                "set ZAPFAST_TEST_RTL_FONT or install a Hebrew/Arabic-capable sans \
                 (DejaVu, Liberation, FreeSans, Arial) for RTL layout tests",
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
