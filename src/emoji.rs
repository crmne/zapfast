//! Color emoji from the desktop's bitmap emoji font, with a bundled fallback.
//!
//! Layout replaces each emoji sequence with a transparent placeholder, then
//! paints the bitmap over it. The font's ligature table resolves flags, skin
//! tones, and joined sequences.

use std::collections::HashMap;
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use egui::text::LayoutJob;
use egui::{
    Color32, ColorImage, Pos2, Rect, Stroke, TextFormat, TextureHandle, TextureOptions, vec2,
};
use skrifa::bitmap::BitmapData;
use skrifa::instance::Size;
use skrifa::raw::TableProvider;
use skrifa::raw::tables::gsub::{ExtensionSubtable, LigatureSubstFormat1, SubstitutionLookup};
use skrifa::{FontRef, GlyphId, MetadataProvider};
use unicode_segmentation::UnicodeSegmentation;

/// Full-square placeholder used for each emoji sequence during layout.
pub const PLACEHOLDER: char = '\u{2B1B}';

/// Cached emoji bitmap size.
const TEXTURE_WIDTH: u32 = 72;

struct Font {
    bytes: Vec<u8>,
    index: u32,
    /// Maps a glyph sequence to its ligature glyph.
    ligatures: HashMap<Vec<u32>, u32>,
}

static FONT: OnceLock<Option<Font>> = OnceLock::new();

/// Noto Color Emoji supplies bitmap glyphs on systems such as Windows whose
/// installed emoji font uses an outline colour format this renderer cannot
/// rasterize. macOS uses it too: Apple Color Emoji joins flags, skin tones,
/// and ZWJ sequences through an AAT `morx` table rather than GSUB ligatures,
/// so every sequence would fall back to its first part, and the 190 MB font
/// would stay in memory for nothing.
const BUNDLED: &[u8] = include_bytes!("../assets/fonts/NotoColorEmoji.ttf");

/// Whether a color emoji font is available.
pub fn available() -> bool {
    font().is_some()
}

/// Loads the emoji font before the first frame needs it.
pub fn warm_up() {
    let _ = font();
}

fn font() -> Option<&'static Font> {
    FONT.get_or_init(load).as_ref()
}

fn load() -> Option<Font> {
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    if let Some((path, index)) = find()
        && let Ok(bytes) = std::fs::read(&path)
        && let Some(font) = load_bytes(bytes, index, &path.display().to_string())
    {
        return Some(font);
    }
    load_bytes(BUNDLED.to_vec(), 0, "bundled Noto Color Emoji")
}

fn load_bytes(bytes: Vec<u8>, index: u32, source: &str) -> Option<Font> {
    let font = FontRef::from_index(&bytes, index).ok()?;
    if font.bitmap_strikes().is_empty() {
        log::info!("{source} has no bitmap emoji");
        return None;
    }
    let ligatures = read_ligatures(&font);
    log::info!("colour emoji from {source} ({} sequences)", ligatures.len());
    Some(Font {
        bytes,
        index,
        ligatures,
    })
}

/// Desktop color emoji font and selected face. macOS and Windows always use
/// the bundled font, so an installed copy cannot change the result there.
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn find() -> Option<(PathBuf, u32)> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    for dir in [
        "/usr/share/fonts/noto",
        "/usr/share/fonts/truetype/noto",
        "/usr/share/fonts/google-noto-emoji",
        "/usr/share/fonts/noto-emoji",
        "/usr/share/fonts/TTF",
        "/usr/share/fonts",
        "/usr/local/share/fonts",
    ] {
        candidates.push(PathBuf::from(dir).join("NotoColorEmoji.ttf"));
    }
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        candidates.push(home.join(".local/share/fonts/NotoColorEmoji.ttf"));
        candidates.push(home.join(".fonts/NotoColorEmoji.ttf"));
    }
    if let Some(path) = candidates.into_iter().find(|path| path.is_file()) {
        return Some((path, 0));
    }
    // Search all font directories because distributions use different paths.
    for root in ["/usr/share/fonts", "/usr/local/share/fonts"] {
        if let Some(path) = search(&PathBuf::from(root), 0) {
            return Some((path, 0));
        }
    }
    None
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn search(dir: &std::path::Path, depth: usize) -> Option<PathBuf> {
    if depth > 4 {
        return None;
    }
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if path.is_dir() {
            if let Some(found) = search(&path, depth + 1) {
                return Some(found);
            }
        } else if name.eq_ignore_ascii_case("NotoColorEmoji.ttf") {
            return Some(path);
        }
    }
    None
}

/// Font ligatures keyed by their input glyphs.
fn read_ligatures(font: &FontRef<'_>) -> HashMap<Vec<u32>, u32> {
    let mut map = HashMap::new();
    let Ok(gsub) = font.gsub() else {
        return map;
    };
    let Ok(lookups) = gsub.lookup_list() else {
        return map;
    };
    for lookup in lookups.lookups().iter().flatten() {
        match lookup {
            SubstitutionLookup::Ligature(lookup) => {
                for subtable in lookup.subtables().iter().flatten() {
                    add_ligatures(&mut map, &subtable);
                }
            }
            SubstitutionLookup::Extension(lookup) => {
                for subtable in lookup.subtables().iter().flatten() {
                    if let ExtensionSubtable::Ligature(extension) = subtable
                        && let Ok(subtable) = extension.extension()
                    {
                        add_ligatures(&mut map, &subtable);
                    }
                }
            }
            _ => {}
        }
    }
    map
}

fn add_ligatures(map: &mut HashMap<Vec<u32>, u32>, subtable: &LigatureSubstFormat1<'_>) {
    let Ok(coverage) = subtable.coverage() else {
        return;
    };
    let firsts: Vec<u32> = coverage
        .iter()
        .map(|glyph| u32::from(glyph.to_u16()))
        .collect();
    for (index, set) in subtable.ligature_sets().iter().enumerate() {
        let (Ok(set), Some(first)) = (set, firsts.get(index)) else {
            continue;
        };
        for ligature in set.ligatures().iter().flatten() {
            let mut key = vec![*first];
            key.extend(
                ligature
                    .component_glyph_ids()
                    .iter()
                    .map(|component| u32::from(component.get().to_u16())),
            );
            map.insert(key, u32::from(ligature.ligature_glyph().to_u16()));
        }
    }
}

impl Font {
    fn font_ref(&self) -> Option<FontRef<'_>> {
        FontRef::from_index(&self.bytes, self.index).ok()
    }

    /// Resolves a sequence to its final glyph through font ligatures.
    fn glyph(&self, font: &FontRef<'_>, cluster: &[char]) -> Option<GlyphId> {
        let charmap = font.charmap();
        let sequence = |chars: &[char]| -> Option<u32> {
            let glyphs: Vec<u32> = chars
                .iter()
                .map(|character| charmap.map(*character as u32).map(|glyph| glyph.to_u32()))
                .collect::<Option<_>>()?;
            match glyphs.as_slice() {
                [single] => Some(*single),
                _ => self.ligatures.get(&glyphs).copied(),
            }
        };
        if let Some(glyph) = sequence(cluster) {
            return Some(GlyphId::new(glyph));
        }
        let mut stripped: Vec<char> = cluster
            .iter()
            .copied()
            .filter(|character| *character != '\u{FE0F}')
            .collect();
        if stripped.len() != cluster.len()
            && let Some(glyph) = sequence(&stripped)
        {
            return Some(GlyphId::new(glyph));
        }
        // Fall back to the known part of an unsupported joined sequence.
        while stripped.len() > 1 {
            stripped.pop();
            while stripped.last().is_some_and(|c| *c == '\u{200D}') {
                stripped.pop();
            }
            if let Some(glyph) = sequence(&stripped) {
                return Some(GlyphId::new(glyph));
            }
        }
        None
    }

    fn image(&self, font: &FontRef<'_>, glyph: GlyphId) -> Option<ColorImage> {
        let strikes = font.bitmap_strikes();
        let bitmap = strikes.glyph_for_size(Size::new(TEXTURE_WIDTH as f32), glyph)?;
        let rgba = match bitmap.data {
            BitmapData::Png(png) => {
                image::load_from_memory_with_format(png, image::ImageFormat::Png)
                    .ok()?
                    .to_rgba8()
            }
            BitmapData::Bgra(bgra) => {
                let mut buffer =
                    image::RgbaImage::from_raw(bitmap.width, bitmap.height, bgra.to_vec())?;
                for pixel in buffer.pixels_mut() {
                    pixel.0.swap(0, 2);
                }
                buffer
            }
            BitmapData::Mask(_) => return None,
        };
        if rgba.width() == 0 || rgba.height() == 0 {
            return None;
        }
        let height = (TEXTURE_WIDTH * rgba.height() / rgba.width()).max(1);
        let resized = image::imageops::resize(
            &rgba,
            TEXTURE_WIDTH,
            height,
            image::imageops::FilterType::Triangle,
        );
        Some(ColorImage::from_rgba_unmultiplied(
            [TEXTURE_WIDTH as usize, height as usize],
            resized.as_raw(),
        ))
    }
}

/// Uploaded emoji textures for each egui context.
#[derive(Clone, Default)]
struct Cache(Arc<Mutex<HashMap<String, Option<TextureHandle>>>>);

fn texture(ctx: &egui::Context, cluster: &str) -> Option<TextureHandle> {
    let cache: Cache = ctx.data_mut(|data| {
        data.get_temp_mut_or_default::<Cache>(egui::Id::new("emoji-cache"))
            .clone()
    });
    let mut map = cache
        .0
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(known) = map.get(cluster) {
        return known.clone();
    }
    let handle = font().and_then(|font| {
        let font_ref = font.font_ref()?;
        let chars: Vec<char> = cluster.chars().collect();
        let glyph = font.glyph(&font_ref, &chars)?;
        let image = font.image(&font_ref, glyph)?;
        Some(ctx.load_texture(format!("emoji-{cluster}"), image, TextureOptions::LINEAR))
    });
    map.insert(cluster.to_owned(), handle.clone());
    handle
}

/// Plain text or one emoji sequence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Piece<'a> {
    Text(&'a str),
    Emoji(&'a str),
}

/// Splits text into plain runs and emoji sequences.
pub fn pieces(text: &str) -> Vec<Piece<'_>> {
    let mut pieces = Vec::new();
    let mut run_start = 0;
    let mut offset = 0;
    for cluster in text.graphemes(true) {
        if is_emoji(cluster) {
            if run_start < offset {
                pieces.push(Piece::Text(&text[run_start..offset]));
            }
            pieces.push(Piece::Emoji(&text[offset..offset + cluster.len()]));
            run_start = offset + cluster.len();
        }
        offset += cluster.len();
    }
    if run_start < text.len() {
        pieces.push(Piece::Text(&text[run_start..]));
    }
    pieces
}

/// Whether a grapheme cluster needs an emoji bitmap.
pub fn is_emoji(cluster: &str) -> bool {
    let Some(first) = cluster.chars().next() else {
        return false;
    };
    if (first as u32) < 0xA9 {
        // Digits, `#`, and `*` are emoji only in keycap sequences.
        return cluster.contains('\u{20E3}');
    }
    if is_presentation(first) {
        return true;
    }
    // Variation selectors, emoji modifiers, and joiners can request emoji
    // presentation. Keep joiner-using writing systems as text.
    let capable = matches!(first, '\u{A9}' | '\u{AE}')
        || ('\u{2000}'..='\u{33FF}').contains(&first)
        || ('\u{1F000}'..='\u{1FAFF}').contains(&first);
    capable
        && cluster
            .chars()
            .any(|c| matches!(c, '\u{FE0F}' | '\u{20E3}' | '\u{200D}') || is_skin_tone(c))
}

fn is_skin_tone(c: char) -> bool {
    ('\u{1F3FB}'..='\u{1F3FF}').contains(&c)
}

/// Characters with default emoji presentation in Unicode data.
fn is_presentation(c: char) -> bool {
    let code = c as u32;
    if (0x1F000..=0x1FAFF).contains(&code) {
        return !matches!(
            code,
            0x1F170 | 0x1F171 | 0x1F17E | 0x1F17F | 0x1F202 | 0x1F237
        );
    }
    matches!(
        code,
        0x231A | 0x231B | 0x23E9..=0x23EC | 0x23F0 | 0x23F3 | 0x25FD | 0x25FE | 0x2614 | 0x2615
            | 0x2648..=0x2653 | 0x267F | 0x2693 | 0x26A1 | 0x26AA | 0x26AB | 0x26BD | 0x26BE
            | 0x26C4 | 0x26C5 | 0x26CE | 0x26D4 | 0x26EA | 0x26F2 | 0x26F3 | 0x26F5 | 0x26FA
            | 0x26FD | 0x2705 | 0x270A | 0x270B | 0x2728 | 0x274C | 0x274E | 0x2753..=0x2755
            | 0x2757 | 0x2795..=0x2797 | 0x27B0 | 0x27BF | 0x2B1B | 0x2B1C | 0x2B50 | 0x2B55
    )
}

/// Counts emoji when the input contains only emoji and spaces.
pub fn only_emoji(text: &str) -> Option<usize> {
    let mut count = 0;
    for piece in pieces(text) {
        match piece {
            Piece::Emoji(_) => count += 1,
            Piece::Text(run) if run.trim().is_empty() => {}
            Piece::Text(_) => return None,
        }
    }
    (count > 0).then_some(count)
}

/// Appends text with placeholders and records their emoji sequences.
pub fn append(
    job: &mut LayoutJob,
    placements: &mut Vec<String>,
    text: &str,
    format: &TextFormat,
) -> usize {
    let start = job.text.len();
    if !available() {
        job.append(text, 0.0, format.clone());
        return job.text[start..].chars().count();
    }
    for piece in pieces(text) {
        match piece {
            Piece::Text(run) => job.append(run, 0.0, format.clone()),
            Piece::Emoji(cluster) => {
                let mut hidden = format.clone();
                hidden.color = Color32::TRANSPARENT;
                hidden.underline = Stroke::NONE;
                hidden.strikethrough = Stroke::NONE;
                job.append(&PLACEHOLDER.to_string(), 0.0, hidden);
                placements.push(cluster.to_owned());
            }
        }
    }
    job.text[start..].chars().count()
}

/// Paints emoji bitmaps over a galley's placeholders.
/// Falls back to monochrome text when no bitmap exists.
pub fn paint_cluster(ui: &egui::Ui, cluster: &str, rect: Rect) {
    let painter = ui.painter();
    match texture(ui.ctx(), cluster) {
        Some(texture) => {
            let side = rect.height() * 1.08;
            let size = texture.size_vec2();
            let scale = (side / size.x).min(side / size.y);
            let image_rect = Rect::from_center_size(
                rect.center() + vec2(0.0, rect.height() * 0.02),
                size * scale,
            );
            painter.image(
                texture.id(),
                image_rect,
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        None => {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                cluster,
                egui::FontId::proportional(rect.height() * 0.8),
                ui.visuals().text_color(),
            );
        }
    }
}

/// Builds an editor layout without changing character offsets. Emoji glyphs
/// are transparent and returned for bitmap overlay.
pub fn editor_job(
    text: &str,
    format: &egui::TextFormat,
) -> (egui::text::LayoutJob, Vec<(usize, usize, String)>) {
    let mut job = egui::text::LayoutJob::default();
    let mut clusters = Vec::new();
    if !available() {
        job.append(text, 0.0, format.clone());
        return (job, clusters);
    }
    let mut at = 0usize;
    for piece in pieces(text) {
        match piece {
            Piece::Text(run) => {
                job.append(run, 0.0, format.clone());
                at += run.chars().count();
            }
            Piece::Emoji(cluster) => {
                let mut hidden = format.clone();
                hidden.color = Color32::TRANSPARENT;
                job.append(cluster, 0.0, hidden);
                let length = cluster.chars().count();
                clusters.push((at, length, cluster.to_owned()));
                at += length;
            }
        }
    }
    (job, clusters)
}

pub fn paint(ui: &egui::Ui, galley: &egui::Galley, origin: Pos2, placements: &[String]) {
    if placements.is_empty() {
        return;
    }
    for (rect, cluster) in placeholder_rects(galley).zip(placements) {
        paint_cluster(ui, cluster, rect.translate(origin.to_vec2()));
    }
}

/// Placeholder rectangles in logical order. Run reordering moves the mesh and
/// glyphs together but retains each glyph's original mesh offset. Placeholder
/// quads are tessellated even when their text color is transparent.
pub(crate) fn placeholder_rects(galley: &egui::Galley) -> impl Iterator<Item = Rect> + '_ {
    galley.rows.iter().flat_map(|row| {
        let mut placeholders: Vec<_> = row
            .glyphs
            .iter()
            .filter(|glyph| glyph.chr == PLACEHOLDER)
            .collect();
        placeholders.sort_by_key(|glyph| glyph.first_vertex);
        placeholders
            .into_iter()
            .map(move |glyph| glyph.logical_rect().translate(row.pos.to_vec2()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bundled_font_renders_colour_emoji() {
        let font = load_bytes(BUNDLED.to_vec(), 0, "test font").expect("bundled font");
        let font_ref = font.font_ref().expect("font face");
        let glyph = font
            .glyph(&font_ref, &['\u{1F600}'])
            .expect("grinning face glyph");
        let image = font.image(&font_ref, glyph).expect("colour bitmap");
        assert_eq!(image.size[0], TEXTURE_WIDTH as usize);
        assert!(
            image
                .pixels
                .iter()
                .any(|pixel| { pixel.a() > 0 && pixel.r() != pixel.g() && pixel.g() != pixel.b() })
        );
    }

    #[test]
    fn an_editor_job_keeps_the_text_and_places_the_emoji() {
        let format = egui::TextFormat::simple(egui::FontId::proportional(14.0), Color32::WHITE);
        let text = "hi 😊 and 👍🏽!";
        let (job, clusters) = editor_job(text, &format);
        assert_eq!(job.text, text, "the galley must mirror the buffer");
        if available() {
            assert_eq!(clusters.len(), 2);
            let (start, length, cluster) = &clusters[0];
            assert_eq!(
                text.chars().skip(*start).take(*length).collect::<String>(),
                *cluster
            );
            let (start, length, cluster) = &clusters[1];
            assert_eq!(
                text.chars().skip(*start).take(*length).collect::<String>(),
                *cluster
            );
        }
    }

    #[test]
    fn pieces_split_emoji_out_of_text() {
        assert_eq!(
            pieces("hi 😀 there"),
            vec![
                Piece::Text("hi "),
                Piece::Emoji("😀"),
                Piece::Text(" there")
            ]
        );
        assert_eq!(pieces("plain"), vec![Piece::Text("plain")]);
        // Cover regional-indicator flags and ZWJ family sequences.
        assert_eq!(pieces("🇩🇪"), vec![Piece::Emoji("🇩🇪")]);
        assert_eq!(pieces("👨‍👩‍👧"), vec![Piece::Emoji("👨‍👩‍👧")]);
        assert_eq!(pieces("👍🏽!"), vec![Piece::Emoji("👍🏽"), Piece::Text("!")]);
    }

    #[test]
    fn presentation_follows_unicode() {
        assert!(is_emoji("❤️"));
        assert!(!is_emoji("❤"));
        assert!(is_emoji("⭐"));
        assert!(is_emoji("1️⃣"));
        assert!(!is_emoji("1"));
        assert!(!is_emoji("©"));
        assert!(is_emoji("©️"));
        assert!(!is_emoji("a"));
    }

    #[test]
    fn emoji_only_messages_are_counted() {
        assert_eq!(only_emoji("😂"), Some(1));
        assert_eq!(only_emoji("😂 🙏"), Some(2));
        assert_eq!(only_emoji("ok 😂"), None);
        assert_eq!(only_emoji(""), None);
    }

    #[test]
    fn placeholders_line_up_with_placements() {
        let mut job = LayoutJob::default();
        let mut placements = Vec::new();
        append(&mut job, &mut placements, "a 😀 b", &TextFormat::default());
        if available() {
            assert_eq!(placements, vec!["😀".to_owned()]);
            assert_eq!(job.text.matches(PLACEHOLDER).count(), 1);
        } else {
            assert!(placements.is_empty());
            assert_eq!(job.text, "a 😀 b");
        }
    }

    /// A sequence the font cannot join falls back to its first part, which
    /// still yields a glyph, so compare against that part.
    fn assert_joined(font: &Font, font_ref: &FontRef<'_>, sequence: &str) {
        let chars: Vec<char> = sequence.chars().collect();
        let joined = font.glyph(font_ref, &chars).expect("sequence glyph");
        let first = font.glyph(font_ref, &chars[..1]);
        assert_ne!(
            Some(joined),
            first,
            "{sequence} fell back to its first part"
        );
    }

    #[test]
    fn the_bundled_font_joins_sequences() {
        let font = load_bytes(BUNDLED.to_vec(), 0, "test font").expect("bundled font");
        let font_ref = font.font_ref().expect("font face");
        for sequence in ["🇩🇪", "👍🏽", "👨‍👩‍👧"] {
            assert_joined(&font, &font_ref, sequence);
        }
    }

    #[test]
    fn the_system_font_resolves_sequences_when_present() {
        let Some(font) = font() else {
            return;
        };
        let font_ref = font.font_ref().expect("font parses");
        assert!(font.glyph(&font_ref, &['😀']).is_some());
        for sequence in ["🇩🇪", "👍🏽", "👨‍👩‍👧"] {
            assert_joined(font, &font_ref, sequence);
        }
        let glyph = font.glyph(&font_ref, &['😀']).expect("glyph");
        let image = font.image(&font_ref, glyph).expect("picture");
        assert_eq!(image.size[0], TEXTURE_WIDTH as usize);
    }
}

#[cfg(test)]
mod script_tests {
    use super::*;

    #[test]
    fn joiners_in_ordinary_words_are_not_emoji() {
        // These writing-system clusters use ZWJ but are not emoji.
        for word in [
            "\u{0DC1}\u{0DCA}\u{200D}\u{0DBB}\u{0DD3}",
            "\u{09B0}\u{200D}\u{09CD}\u{09AF}",
            "\u{0D28}\u{0D4D}\u{200D}",
        ] {
            assert!(!is_emoji(word), "{word:?} is text");
            assert_eq!(only_emoji(word), None);
        }
    }

    #[test]
    fn sequences_that_ask_to_be_pictures_still_are() {
        for picture in [
            "\u{2764}\u{FE0F}",                    // red heart
            "\u{1F3F3}\u{FE0F}\u{200D}\u{1F308}",  // rainbow flag
            "\u{1F9D1}\u{1F3FD}\u{200D}\u{1F4BB}", // technologist, medium skin
            "\u{23}\u{FE0F}\u{20E3}",              // keycap #
            "\u{1F1EE}\u{1F1F9}",                  // flag of Italy
            "\u{A9}\u{FE0F}",                      // copyright as emoji
        ] {
            assert!(is_emoji(picture), "{picture:?} is a picture");
        }
        assert!(!is_emoji("\u{A9}"), "a bare copyright sign is text");
        assert!(!is_emoji("a"));
    }
}
