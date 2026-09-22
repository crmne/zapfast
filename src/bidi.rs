//! Paragraph-level run order for RTL scripts.
//!
//! egui 0.36 shapes each font run with harfrust (within-run RTL shaping is
//! already correct) and then places those runs left-to-right. Inter has no
//! Hebrew/Arabic coverage, so spaces stay on Inter while letters fall back to
//! another face. Each word is its own RTL run. Laid out LTR, the first logical
//! word sits on the left; a Hebrew reader starting from the right reads the
//! last word first (`הכלב הגדול קפץ` reads as `קפץ הגדול הכלב`).
//!
//! After line breaking, reverse the order of runs on each RTL paragraph and
//! leave glyphs inside each run alone. Glyph positions, the glyph vec (so
//! `Row::char_at` keeps increasing centres), and decoration mesh vertices move
//! together. `LayoutJob` text stays logical for copy.
//!
//! Call once on a fresh egui galley (`layout_job` does). A second pass is a
//! no-op when the row already matches the paragraph's expected visual order.

use icu_properties::{CodePointMapData, props::BidiClass};
use std::ops::Range;
use std::sync::Arc;

use egui::epaint::text::{Galley, Glyph, RowVisuals};
use egui::epaint::{Mesh, Vec2};

/// Lays out `job` and reorders RTL paragraph runs for visual word order.
pub fn layout_job(ui: &egui::Ui, job: egui::text::LayoutJob) -> std::sync::Arc<Galley> {
    let mut galley = ui.painter().layout_job(job);
    reorder_rtl_runs(std::sync::Arc::make_mut(&mut galley));
    galley
}

/// Places RTL-paragraph runs in visual order without touching within-run shaping.
pub fn reorder_rtl_runs(galley: &mut Galley) {
    let paragraphs: Vec<String> = paragraph_slices(galley.text())
        .into_iter()
        .map(str::to_owned)
        .collect();
    if paragraphs.is_empty() {
        return;
    }
    let mut para_index = 0;
    for placed in &mut galley.rows {
        let paragraph = paragraphs
            .get(para_index)
            .map(String::as_str)
            .unwrap_or_else(|| paragraphs.last().map(String::as_str).unwrap_or(""));
        if paragraph_rtl(paragraph) {
            let row = Arc::make_mut(&mut placed.row);
            reorder_row(&mut row.glyphs, &mut row.visuals, paragraph);
            row.visuals.mesh_bounds = row.visuals.mesh.calc_bounds();
        }
        if placed.ends_with_newline {
            para_index = (para_index + 1).min(paragraphs.len().saturating_sub(1));
        }
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

fn reorder_row(glyphs: &mut Vec<Glyph>, visuals: &mut RowVisuals, paragraph: &str) {
    let runs = split_runs(glyphs);
    if runs.len() < 2 || already_rtl_visual(glyphs, &runs, paragraph) {
        return;
    }

    let packed: Vec<(f32, f32, Vec<egui::Pos2>)> = runs
        .iter()
        .map(|run| {
            let slice = &glyphs[run.clone()];
            let origin = min_x(slice);
            let max = slice
                .iter()
                .map(Glyph::max_x)
                .fold(f32::NEG_INFINITY, f32::max);
            let rel = slice
                .iter()
                .map(|glyph| egui::Pos2::new(glyph.pos.x - origin, glyph.pos.y))
                .collect();
            (origin, (max - origin).max(0.0), rel)
        })
        .collect();

    let line_origin = min_x(glyphs);
    let line_max = glyphs
        .iter()
        .map(Glyph::max_x)
        .fold(f32::NEG_INFINITY, f32::max);
    let mut new_glyphs = Vec::with_capacity(glyphs.len());
    let mut x = line_origin;
    let mut deltas: Vec<(f32, f32, f32)> = Vec::with_capacity(runs.len());

    for (run, (old_origin, width, rel)) in runs.iter().rev().zip(packed.iter().rev()) {
        let delta_x = x - old_origin;
        deltas.push((*old_origin, old_origin + width, delta_x));
        for (glyph, rel_pos) in glyphs[run.clone()].iter().zip(rel) {
            let mut moved = *glyph;
            let new_pos = egui::Pos2::new(x + rel_pos.x, rel_pos.y);
            let delta = new_pos.to_vec2() - glyph.pos.to_vec2();
            moved.pos = new_pos;
            shift_glyph_mesh(&mut visuals.mesh, glyph, delta);
            new_glyphs.push(moved);
        }
        x += width;
    }

    shift_decoration_mesh(
        &mut visuals.mesh,
        &visuals.glyph_vertex_range,
        &deltas,
        line_origin,
        line_max,
    );

    *glyphs = new_glyphs;
}

/// True when letter runs left-to-right already match the RTL visual order implied
/// by `paragraph` (last logical token on the left). Uses the job text, so it
/// stays correct after the glyph vec itself has been permuted.
fn already_rtl_visual(glyphs: &[Glyph], runs: &[Range<usize>], paragraph: &str) -> bool {
    let row_sets = letter_run_charsets(glyphs, runs);
    if row_sets.len() < 2 {
        // One letter run (plus spaces / LTR): still needs a reverse when an LTR
        // run sits to its right under egui's LTR placement (`הכלב OK`).
        return !ltr_run_right_of_rtl(glyphs, runs);
    }
    let para_tokens = paragraph_letter_tokens(paragraph);
    let on_row: Vec<Vec<char>> = para_tokens
        .into_iter()
        .filter(|token| row_sets.iter().any(|run| run == token))
        .collect();
    if on_row.len() < 2 {
        return !ltr_run_right_of_rtl(glyphs, runs);
    }
    let expected_left = on_row.last().expect("len >= 2");
    let Some(actual_left) = leftmost_letter_charset(glyphs, runs) else {
        return true;
    };
    &actual_left == expected_left
}

fn ltr_run_right_of_rtl(glyphs: &[Glyph], runs: &[Range<usize>]) -> bool {
    let mut rtl_x = None;
    let mut ltr_x = None;
    for run in runs {
        let slice = &glyphs[run.clone()];
        if slice.iter().any(is_rtl_letter) {
            let x = min_x(slice);
            rtl_x = Some(rtl_x.map_or(x, |old: f32| old.min(x)));
        } else if slice.iter().any(is_ltr_letter) {
            let x = min_x(slice);
            ltr_x = Some(ltr_x.map_or(x, |old: f32| old.max(x)));
        }
    }
    match (rtl_x, ltr_x) {
        (Some(rtl), Some(ltr)) => ltr > rtl,
        _ => false,
    }
}

// Compare character sets rather than counts: shaped RTL runs can repeat a
// character in their zero-width continuation entries. Dropping those entries
// instead loses letters consumed by ligatures and prevents matching the word.
fn paragraph_letter_tokens(paragraph: &str) -> Vec<Vec<char>> {
    paragraph
        .split_whitespace()
        .filter(|token| token.chars().any(|c| is_strong_rtl(c) || is_strong_ltr(c)))
        .map(|token| {
            let mut chars: Vec<char> = token
                .chars()
                .filter(|c| (is_rtl(*c) || is_strong_ltr(*c)) && !is_nonspacing_mark(*c))
                .collect();
            chars.sort_unstable();
            chars.dedup();
            chars
        })
        .filter(|chars| !chars.is_empty())
        .collect()
}

fn letter_run_charsets(glyphs: &[Glyph], runs: &[Range<usize>]) -> Vec<Vec<char>> {
    let mut out = Vec::new();
    for run in runs {
        let mut chars: Vec<char> = glyphs[run.clone()]
            .iter()
            // Shaping can put a ligature's remaining letters in zero-width
            // continuation glyphs. Their characters still identify the word.
            .filter(|glyph| is_strong_rtl(glyph.chr) || is_strong_ltr(glyph.chr))
            .map(|glyph| glyph.chr)
            .collect();
        if chars.is_empty() {
            continue;
        }
        chars.sort_unstable();
        chars.dedup();
        out.push(chars);
    }
    out
}

fn leftmost_letter_charset(glyphs: &[Glyph], runs: &[Range<usize>]) -> Option<Vec<char>> {
    let mut best: Option<(f32, Vec<char>)> = None;
    for run in runs {
        let slice = &glyphs[run.clone()];
        let mut chars: Vec<char> = slice
            .iter()
            .filter(|glyph| is_strong_rtl(glyph.chr) || is_strong_ltr(glyph.chr))
            .map(|glyph| glyph.chr)
            .collect();
        if chars.is_empty() {
            continue;
        }
        chars.sort_unstable();
        chars.dedup();
        let x = min_x(slice);
        if best.as_ref().is_none_or(|(best_x, _)| x < *best_x) {
            best = Some((x, chars));
        }
    }
    best.map(|(_, chars)| chars)
}

fn min_x(glyphs: &[Glyph]) -> f32 {
    glyphs
        .iter()
        .map(|glyph| glyph.pos.x)
        .fold(f32::INFINITY, f32::min)
}

fn split_runs(glyphs: &[Glyph]) -> Vec<Range<usize>> {
    let mut runs = Vec::new();
    let mut index = 0;
    while index < glyphs.len() {
        let rtl = is_rtl_item(&glyphs[index]);
        let start = index;
        index += 1;
        while index < glyphs.len() && is_rtl_item(&glyphs[index]) == rtl {
            index += 1;
        }
        runs.push(start..index);
    }
    runs
}

/// Script run membership. Diacritics (often ~0 advance) stay with the letter;
/// never early-out on `advance_width`. Weak digits/punctuation in RTL blocks
/// stay out so they can form their own runs and reverse with the paragraph.
fn is_rtl_item(glyph: &Glyph) -> bool {
    is_strong_rtl(glyph.chr) || is_nonspacing_mark(glyph.chr)
}

fn is_rtl_letter(glyph: &Glyph) -> bool {
    glyph.advance_width > 0.01 && is_strong_rtl(glyph.chr)
}

fn is_ltr_letter(glyph: &Glyph) -> bool {
    glyph.advance_width > 0.01 && is_strong_ltr(glyph.chr)
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

/// Move underline / strikethrough / background geometry with the runs.
///
/// A decoration that already spans the whole line (same overall coverage after
/// reversing runs) is left alone. Narrower decorations are assigned to a run by
/// their pre-reorder x and shifted once, so sequential updates cannot tear them.
fn shift_decoration_mesh(
    mesh: &mut Mesh,
    glyph_vertex_range: &Range<usize>,
    deltas: &[(f32, f32, f32)],
    line_min: f32,
    line_max: f32,
) {
    if deltas.is_empty() {
        return;
    }
    let line_width = (line_max - line_min).max(0.0);
    let pad = 1.5;

    let mut deco_min = f32::INFINITY;
    let mut deco_max = f32::NEG_INFINITY;
    for (index, vertex) in mesh.vertices.iter().enumerate() {
        if glyph_vertex_range.contains(&index) {
            continue;
        }
        deco_min = deco_min.min(vertex.pos.x);
        deco_max = deco_max.max(vertex.pos.x);
    }
    if deco_min.is_finite() && line_width > 0.0 && (deco_max - deco_min) >= line_width * 0.9 {
        return;
    }

    for (index, vertex) in mesh.vertices.iter_mut().enumerate() {
        if glyph_vertex_range.contains(&index) {
            continue;
        }
        for &(old_min, old_max, delta_x) in deltas {
            if vertex.pos.x >= old_min - pad && vertex.pos.x <= old_max + pad {
                vertex.pos.x += delta_x;
                break;
            }
        }
    }
}

fn paragraph_rtl(text: &str) -> bool {
    text.chars().find_map(strong_direction).unwrap_or(false)
}

fn strong_direction(c: char) -> Option<bool> {
    if is_strong_rtl(c) {
        Some(true)
    } else if is_strong_ltr(c) {
        Some(false)
    } else {
        None
    }
}

/// Unicode L includes all scripts; numbers, symbols and marks are not strong.
fn is_strong_ltr(c: char) -> bool {
    CodePointMapData::<BidiClass>::new().get(c) == BidiClass::LeftToRight
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

fn is_nonspacing_mark(c: char) -> bool {
    CodePointMapData::<BidiClass>::new().get(c) == BidiClass::NonspacingMark
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::text::{FontData, FontDefinitions, FontFamily, LayoutJob, TextFormat};
    use egui::{Color32, FontId, Pos2, vec2};
    use std::{path::PathBuf, sync::Arc};

    const RTL_FONT_CANDIDATES: &[&str] = &[
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
        "/usr/share/fonts/liberation/LiberationSans-Regular.ttf",
        "/usr/share/fonts/truetype/noto/NotoSansHebrew-Regular.ttf",
        "/usr/share/fonts/truetype/noto/NotoSansArabic-Regular.ttf",
        "/usr/share/fonts/TTF/DejaVuSans.ttf",
        "/System/Library/Fonts/Supplemental/Arial.ttf",
        "/Library/Fonts/Arial Unicode.ttf",
        r"C:\Windows\Fonts\arial.ttf",
        r"C:\Windows\Fonts\tahoma.ttf",
    ];

    fn rtl_font() -> PathBuf {
        std::env::var_os("ZAPFAST_TEST_RTL_FONT")
            .map(PathBuf::from)
            .filter(|path| path.is_file())
            .or_else(|| {
                RTL_FONT_CANDIDATES
                    .iter()
                    .map(PathBuf::from)
                    .find(|path| path.is_file())
            })
            .expect(
                "set ZAPFAST_TEST_RTL_FONT or install a Hebrew/Arabic-capable sans \
                 (DejaVu, Liberation, Arial) for RTL layout tests",
            )
    }

    #[test]
    fn first_strong_direction_uses_unicode_bidi_classes() {
        for text in ["Привет הכלב", "Καλημέρα הכלב", "你好 הכלב", "नमस्ते הכלב"]
        {
            assert!(!paragraph_rtl(text), "{text}");
        }
        for text in [
            "× הכלב הגדול",
            "123 הכלב הגדול",
            "١٢٣ הכלב הגדול",
            "َ הכלב הגדול",
        ] {
            assert!(paragraph_rtl(text), "{text}");
        }
        for c in ['×', '1', '١', '\u{064e}'] {
            assert_eq!(strong_direction(c), None, "{c}");
        }
    }

    #[test]
    fn reordered_emoji_keep_their_logical_identities() {
        let placeholder = crate::emoji::PLACEHOLDER;
        let mut galley = layout_raw(&format!(
            "הכלב {placeholder} הגדול {placeholder} קפץ\nOK {placeholder}"
        ));
        let original: Vec<_> = galley
            .rows
            .iter()
            .flat_map(|row| {
                row.glyphs
                    .iter()
                    .filter(|glyph| glyph.chr == placeholder)
                    .map(|glyph| glyph.first_vertex)
            })
            .collect();
        reorder_rtl_runs(&mut galley);
        let rects: Vec<_> = crate::emoji::placeholder_rects(&galley).collect();
        assert_eq!(rects.len(), 3);
        assert!(
            rects[0].left() > rects[1].left(),
            "the first logical emoji moves to the right"
        );
        assert!(rects[2].top() > rects[0].top());
        // Painting follows each placeholder's original mesh identity, even
        // though the glyph vec is now in visual order.
        let first_row = &galley.rows[0];
        for (index, vertex) in original[..2].iter().enumerate() {
            let glyph = first_row
                .glyphs
                .iter()
                .find(|glyph| glyph.chr == placeholder && glyph.first_vertex == *vertex)
                .unwrap();
            assert_eq!(
                rects[index],
                glyph.logical_rect().translate(first_row.pos.to_vec2())
            );
        }
        let once = rects;
        reorder_rtl_runs(&mut galley);
        assert_eq!(
            crate::emoji::placeholder_rects(&galley).collect::<Vec<_>>(),
            once
        );
    }

    fn layout_raw(text: &str) -> Galley {
        let path = rtl_font();
        let ctx = egui::Context::default();
        let mut fonts = FontDefinitions::default();
        let inter = include_bytes!("../assets/fonts/InterVariable.ttf");
        fonts
            .font_data
            .insert("inter".into(), Arc::new(FontData::from_static(inter)));
        let face =
            std::fs::read(&path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        fonts
            .font_data
            .insert("rtl-fallback".into(), Arc::new(FontData::from_owned(face)));
        fonts.families.insert(
            FontFamily::Proportional,
            vec!["inter".into(), "rtl-fallback".into()],
        );
        fonts
            .families
            .insert(FontFamily::Monospace, vec!["inter".into()]);
        ctx.set_fonts(fonts);

        let galley = std::cell::RefCell::new(None);
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(400.0, 120.0))),
                ..Default::default()
            },
            |ui| {
                let mut job = LayoutJob::default();
                for run in text.split_inclusive(crate::emoji::PLACEHOLDER) {
                    let text = run.trim_end_matches(crate::emoji::PLACEHOLDER);
                    job.append(
                        text,
                        0.0,
                        TextFormat::simple(FontId::proportional(14.0), Color32::WHITE),
                    );
                    if run.ends_with(crate::emoji::PLACEHOLDER) {
                        job.append(
                            &crate::emoji::PLACEHOLDER.to_string(),
                            0.0,
                            TextFormat::simple(FontId::proportional(14.0), Color32::TRANSPARENT),
                        );
                    }
                }
                *galley.borrow_mut() = Some(ui.painter().layout_job(job));
            },
        );
        output.textures_delta.clear();
        let galley = galley.into_inner().expect("galley");
        Arc::try_unwrap(galley).unwrap_or_else(|arc| (*arc).clone())
    }

    fn layout_fixed(text: &str) -> Galley {
        let mut galley = layout_raw(text);
        reorder_rtl_runs(&mut galley);
        galley
    }

    /// Visible RTL letter groups in left-to-right x order (word/run metric).
    fn rtl_words_ltr(galley: &Galley) -> Vec<Vec<char>> {
        let glyphs = &galley.rows[0].glyphs;
        let mut words = Vec::new();
        for run in split_runs(glyphs) {
            if !is_rtl_item(&glyphs[run.start]) {
                continue;
            }
            let mut letters = Vec::new();
            let mut x = f32::INFINITY;
            for glyph in &glyphs[run] {
                if is_rtl_letter(glyph) {
                    x = x.min(glyph.pos.x);
                    letters.push(glyph.chr);
                }
            }
            if !letters.is_empty() {
                words.push((x, letters));
            }
        }
        words.sort_by(|a, b| a.0.total_cmp(&b.0));
        words.into_iter().map(|(_, letters)| letters).collect()
    }

    /// RTL letters inside each word, ordered by increasing x (visual within-run).
    fn rtl_word_letters_by_x(galley: &Galley) -> Vec<Vec<char>> {
        let glyphs = &galley.rows[0].glyphs;
        let mut words = Vec::new();
        for run in split_runs(glyphs) {
            if !is_rtl_item(&glyphs[run.start]) {
                continue;
            }
            let mut letters: Vec<&Glyph> = glyphs[run.clone()]
                .iter()
                .filter(|glyph| is_rtl_letter(glyph))
                .collect();
            if letters.is_empty() {
                continue;
            }
            let x = letters
                .iter()
                .map(|glyph| glyph.pos.x)
                .fold(f32::INFINITY, f32::min);
            letters.sort_by(|a, b| a.pos.x.total_cmp(&b.pos.x));
            words.push((x, letters.into_iter().map(|glyph| glyph.chr).collect()));
        }
        words.sort_by(|a, b| a.0.total_cmp(&b.0));
        words.into_iter().map(|(_, letters)| letters).collect()
    }

    fn sorted(letters: &[char]) -> Vec<char> {
        let mut letters = letters.to_vec();
        letters.sort_unstable();
        letters
    }

    fn charset(word: &str) -> Vec<char> {
        sorted(
            &word
                .chars()
                .filter(|c| is_strong_rtl(*c))
                .collect::<Vec<_>>(),
        )
    }

    fn visible_by_x(galley: &Galley) -> Vec<char> {
        let mut glyphs: Vec<_> = galley.rows[0]
            .glyphs
            .iter()
            .filter(|glyph| glyph.advance_width > 0.01)
            .cloned()
            .collect();
        glyphs.sort_by(|a, b| a.pos.x.total_cmp(&b.pos.x));
        glyphs.into_iter().map(|glyph| glyph.chr).collect()
    }

    fn glyph_centers_increasing(galley: &Galley) -> bool {
        let centres: Vec<f32> = galley.rows[0]
            .glyphs
            .iter()
            .map(|glyph| glyph.logical_rect().center().x)
            .collect();
        centres.windows(2).all(|pair| pair[0] <= pair[1] + 0.01)
    }

    #[test]
    fn hebrew_word_order_is_ltr_before_reorder() {
        let logical = "הכלב הגדול קפץ";
        let galley = layout_raw(logical);
        let words = rtl_words_ltr(&galley);
        assert_eq!(
            words.len(),
            3,
            "Inter fallback should split on spaces: {words:?}"
        );
        assert_eq!(
            sorted(&words[0]),
            charset("הכלב"),
            "failure mode: first logical word is leftmost, so reading RTL hits the last word first"
        );
        assert_eq!(sorted(&words[2]), charset("קפץ"));
        assert_eq!(galley.text(), logical);
    }

    #[test]
    fn hebrew_contact_name_with_an_embedded_quote_keeps_word_order() {
        let logical = "מטיאס יזמות ונדל\"ן";
        let galley = layout_fixed(logical);
        let words = rtl_words_ltr(&galley);
        assert_eq!(sorted(words.last().unwrap()), charset("מטיאס"));
        assert_eq!(sorted(&words[words.len() - 2]), charset("יזמות"));
        assert_eq!(galley.text(), logical, "copy must retain logical text");
        assert!(glyph_centers_increasing(&galley));
    }

    #[test]
    fn hebrew_words_read_rtl_after_run_reorder() {
        let logical = "הכלב הגדול קפץ";
        let before = layout_raw(logical);
        let before_by_x = rtl_word_letters_by_x(&before);
        let mut galley = before.clone();
        reorder_rtl_runs(&mut galley);
        let after = rtl_words_ltr(&galley);
        assert_eq!(sorted(&after[0]), charset("קפץ"), "{after:?}");
        assert_eq!(sorted(&after[1]), charset("הגדול"));
        assert_eq!(sorted(&after[2]), charset("הכלב"));
        let after_by_x = rtl_word_letters_by_x(&galley);
        let dog = before_by_x
            .iter()
            .find(|word| sorted(word) == charset("הכלב"))
            .expect("dog");
        let dog_after = after_by_x
            .iter()
            .find(|word| sorted(word) == charset("הכלב"))
            .expect("dog after");
        assert_eq!(
            dog, dog_after,
            "within-run x-ordered letters must stay (do not reverse letters inside a word)"
        );
        assert!(
            glyph_centers_increasing(&galley),
            "glyph vec must stay x-ordered for Row::char_at"
        );
        assert_eq!(galley.text(), logical);
        reorder_rtl_runs(&mut galley);
        assert_eq!(
            rtl_words_ltr(&galley),
            after,
            "run reorder must be idempotent"
        );
        assert!(glyph_centers_increasing(&galley));
    }

    #[test]
    fn mixed_ltr_paragraph_keeps_latin_on_the_left() {
        let text = "OK הכלב end";
        let galley = layout_fixed(text);
        let visible = visible_by_x(&galley);
        assert_eq!(visible.first().copied(), Some('O'));
        assert_eq!(visible.last().copied(), Some('d'));
        let words = rtl_words_ltr(&galley);
        assert_eq!(words.len(), 1);
        assert_eq!(sorted(&words[0]), charset("הכלב"));
        assert_eq!(galley.text(), text);
    }

    #[test]
    fn single_rtl_run_with_ltr_moves_latin_left() {
        let text = "הכלב OK";
        let before = layout_raw(text);
        let before_words = rtl_words_ltr(&before);
        let before_visible = visible_by_x(&before);
        assert!(
            before_visible
                .iter()
                .position(|c| *c == 'O')
                .zip(before_visible.iter().position(|c| is_strong_rtl(*c)))
                .is_some_and(|(ok, he)| he < ok),
            "failure mode before fix: Hebrew run sits left of OK: {before_visible:?}"
        );
        assert_eq!(before_words.len(), 1);
        let mut galley = before;
        reorder_rtl_runs(&mut galley);
        let visible = visible_by_x(&galley);
        let first_letter = visible.iter().copied().find(|c| !c.is_whitespace());
        assert_eq!(first_letter, Some('O'), "{visible:?}");
        assert_eq!(
            sorted(&rtl_words_ltr(&galley)[0]),
            charset("הכלב"),
            "Hebrew stays a single shaped run on the right"
        );
        assert!(glyph_centers_increasing(&galley));
        reorder_rtl_runs(&mut galley);
        let again = visible_by_x(&galley);
        assert_eq!(
            again.iter().copied().find(|c| !c.is_whitespace()),
            Some('O'),
            "single-RTL+LTR reorder must be idempotent: {again:?}"
        );
    }

    #[test]
    fn arabic_words_read_rtl_after_run_reorder() {
        let logical = "مرحبا بالعالم";
        let before = layout_raw(logical);
        let before_by_x = rtl_word_letters_by_x(&before);
        let galley = layout_fixed(logical);
        let words = rtl_words_ltr(&galley);
        assert!(
            words.len() >= 2,
            "expected space-split Arabic runs, got {words:?}"
        );
        assert_eq!(sorted(words.first().unwrap()), charset("بالعالم"));
        assert_eq!(sorted(words.last().unwrap()), charset("مرحبا"));
        let after_by_x = rtl_word_letters_by_x(&galley);
        let hello = before_by_x
            .iter()
            .find(|word| sorted(word) == charset("مرحبا"))
            .expect("مرحبا");
        let hello_after = after_by_x
            .iter()
            .find(|word| sorted(word) == charset("مرحبا"))
            .expect("مرحبا after");
        assert_eq!(
            hello, hello_after,
            "Arabic within-run x order must stay after run reorder"
        );
        assert_eq!(galley.text(), logical);
        assert!(glyph_centers_increasing(&galley));
    }

    #[test]
    fn hebrew_niqqud_stays_in_the_letter_run() {
        let logical = "שָׁלוֹם עוֹלָם";
        let galley = layout_fixed(logical);
        let glyphs = &galley.rows[0].glyphs;
        let mark_runs = split_runs(glyphs).into_iter().filter(|run| {
            glyphs[run.clone()]
                .iter()
                .any(|glyph| is_nonspacing_mark(glyph.chr))
        });
        for run in mark_runs {
            assert!(
                glyphs[run].iter().any(is_rtl_letter),
                "diacritics must stay with their letter run (no advance_width early-out)"
            );
        }
        let words = rtl_words_ltr(&galley);
        assert_eq!(words.len(), 2, "{words:?}");
        assert_eq!(sorted(&words[0]), charset("עוֹלָם"));
        assert_eq!(sorted(&words[1]), charset("שָׁלוֹם"));
    }

    #[test]
    fn per_paragraph_base_direction() {
        let text = "הכלב הגדול\nOK הכלב השני";
        let galley = layout_fixed(text);
        assert!(galley.rows.len() >= 2, "expected two paragraphs");
        let first = rtl_words_ltr_row(&galley, 0);
        assert_eq!(sorted(&first[0]), charset("הגדול"), "{first:?}");
        assert_eq!(sorted(first.last().unwrap()), charset("הכלב"));
        let second_visible = visible_by_x_row(&galley, galley.rows.len() - 1);
        assert_eq!(
            second_visible.first().copied(),
            Some('O'),
            "LTR paragraph must not reverse its runs: {second_visible:?}"
        );
    }

    #[test]
    fn leading_digits_do_not_force_ltr() {
        let text = "123 הכלב הגדול";
        assert!(
            paragraph_rtl(text),
            "ASCII digits are not strong LTR; first strong is Hebrew"
        );
        let galley = layout_fixed(text);
        let words = rtl_words_ltr(&galley);
        assert_eq!(sorted(&words[0]), charset("הגדול"), "{words:?}");
        assert_eq!(sorted(words.last().unwrap()), charset("הכלב"));
        let visible = visible_by_x(&galley);
        let first_digit = visible.iter().position(|c| c.is_ascii_digit());
        let last_hebrew = visible.iter().rposition(|c| is_strong_rtl(*c));
        assert!(
            first_digit.zip(last_hebrew).is_some_and(|(d, h)| h < d),
            "digits end up on the visual right in an RTL paragraph: {visible:?}"
        );
    }

    #[test]
    fn struck_underline_mesh_moves_with_runs() {
        let ctx = egui::Context::default();
        let mut fonts = FontDefinitions::default();
        let inter = include_bytes!("../assets/fonts/InterVariable.ttf");
        fonts
            .font_data
            .insert("inter".into(), Arc::new(FontData::from_static(inter)));
        let face = std::fs::read(rtl_font()).unwrap();
        fonts
            .font_data
            .insert("rtl-fallback".into(), Arc::new(FontData::from_owned(face)));
        fonts.families.insert(
            FontFamily::Proportional,
            vec!["inter".into(), "rtl-fallback".into()],
        );
        ctx.set_fonts(fonts);

        // Underline only the first logical word so decoration is run-local, not
        // a full-line span that should stay put.
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
        let dog = charset("הכלב");
        let before_dog_x = rtl_words_ltr(&galley)
            .into_iter()
            .zip(rtl_word_min_x(&galley))
            .find(|(word, _)| sorted(word) == dog)
            .map(|(_, x)| x)
            .expect("dog before");
        let glyph_range = galley.rows[0].visuals.glyph_vertex_range.clone();
        let before_deco_x: Vec<f32> = galley.rows[0]
            .visuals
            .mesh
            .vertices
            .iter()
            .enumerate()
            .filter(|(index, _)| !glyph_range.contains(index))
            .map(|(_, vertex)| vertex.pos.x)
            .collect();
        assert!(
            !before_deco_x.is_empty(),
            "expected decoration vertices beyond glyph quads"
        );
        let before_deco_mid =
            before_deco_x.iter().copied().sum::<f32>() / before_deco_x.len() as f32;
        assert!(
            (before_deco_mid - before_dog_x).abs() < 40.0,
            "decorations should start over the first logical word"
        );

        reorder_rtl_runs(&mut galley);
        let after_dog_x = rtl_words_ltr(&galley)
            .into_iter()
            .zip(rtl_word_min_x(&galley))
            .find(|(word, _)| sorted(word) == dog)
            .map(|(_, x)| x)
            .expect("dog after");
        assert!(
            after_dog_x > before_dog_x + 5.0,
            "dog word should move right: before={before_dog_x} after={after_dog_x}"
        );
        let after_deco_x: Vec<f32> = galley.rows[0]
            .visuals
            .mesh
            .vertices
            .iter()
            .enumerate()
            .filter(|(index, _)| !glyph_range.contains(index))
            .map(|(_, vertex)| vertex.pos.x)
            .collect();
        let after_deco_mid = after_deco_x.iter().copied().sum::<f32>() / after_deco_x.len() as f32;
        assert!(
            (after_deco_mid - after_dog_x).abs() < 40.0,
            "decorations must travel with the struck word; deco_mid={after_deco_mid} dog_x={after_dog_x}"
        );
    }

    fn rtl_word_min_x(galley: &Galley) -> Vec<f32> {
        let glyphs = &galley.rows[0].glyphs;
        let mut xs = Vec::new();
        for run in split_runs(glyphs) {
            if !is_rtl_item(&glyphs[run.start]) {
                continue;
            }
            if glyphs[run.clone()].iter().any(is_rtl_letter) {
                xs.push(min_x(&glyphs[run]));
            }
        }
        xs.sort_by(|a, b| a.total_cmp(b));
        xs
    }

    fn rtl_words_ltr_row(galley: &Galley, row: usize) -> Vec<Vec<char>> {
        let glyphs = &galley.rows[row].glyphs;
        let mut words = Vec::new();
        for run in split_runs(glyphs) {
            if !is_rtl_item(&glyphs[run.start]) {
                continue;
            }
            let mut letters = Vec::new();
            let mut x = f32::INFINITY;
            for glyph in &glyphs[run] {
                if is_rtl_letter(glyph) {
                    x = x.min(glyph.pos.x);
                    letters.push(glyph.chr);
                }
            }
            if !letters.is_empty() {
                words.push((x, letters));
            }
        }
        words.sort_by(|a, b| a.0.total_cmp(&b.0));
        words.into_iter().map(|(_, letters)| letters).collect()
    }

    fn visible_by_x_row(galley: &Galley, row: usize) -> Vec<char> {
        let mut glyphs: Vec<_> = galley.rows[row]
            .glyphs
            .iter()
            .filter(|glyph| glyph.advance_width > 0.01)
            .cloned()
            .collect();
        glyphs.sort_by(|a, b| a.pos.x.total_cmp(&b.pos.x));
        glyphs.into_iter().map(|glyph| glyph.chr).collect()
    }

    #[test]
    fn rtl_detection_covers_hebrew_and_arabic() {
        assert!(is_rtl('א'));
        assert!(is_rtl('ب'));
        assert!(!is_rtl('A'));
        assert!(!is_rtl(' '));
        assert!(is_strong_rtl('א'));
        assert!(is_strong_rtl('ب'));
        assert!(!is_strong_rtl('1'));
        assert!(!is_strong_rtl('٠')); // Arabic-Indic digit, not strong
        assert!(!is_strong_ltr('1'));
        assert!(paragraph_rtl("הכלב הגדול קפץ"));
        assert!(!paragraph_rtl("OK הכלב end"));
        assert!(paragraph_rtl("123 הכלב הגדול"));
        assert!(is_nonspacing_mark('\u{05B8}'));
        assert!(!is_rtl(' '));
    }
}
