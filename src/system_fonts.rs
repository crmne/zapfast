//! Installed font families and fallbacks for scripts Inter does not support.
//!
//! Installed fonts are scanned once. The best regular sans-serif face for each
//! missing script is added to egui's fallback list.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use skrifa::MetadataProvider as _;

/// Registered fallback name, font bytes, and face index.
pub struct Fallback {
    pub name: String,
    pub bytes: Vec<u8>,
    pub index: u32,
}

/// Missing scripts represented by a probe character and preferred name hint.
///
/// Entries are ordered so one font covering several scripts is registered once.
const FALLBACK_SCRIPTS: &[(&str, char, &str)] = &[
    ("han", '\u{4e2d}', "cjk"),
    ("kana", '\u{3042}', "cjk"),
    ("hangul", '\u{d55c}', "cjk"),
    ("arabic", '\u{0627}', "arabic"),
    ("hebrew", '\u{05d0}', "hebrew"),
    ("thai", '\u{0e01}', "thai"),
    ("lao", '\u{0e81}', "lao"),
    ("khmer", '\u{1780}', "khmer"),
    ("myanmar", '\u{1000}', "myanmar"),
    ("devanagari", '\u{0915}', "devanagari"),
    ("bengali", '\u{0995}', "bengali"),
    ("gurmukhi", '\u{0a15}', "gurmukhi"),
    ("gujarati", '\u{0a95}', "gujarati"),
    ("tamil", '\u{0ba4}', "tamil"),
    ("telugu", '\u{0c15}', "telugu"),
    ("kannada", '\u{0c95}', "kannada"),
    ("malayalam", '\u{0d15}', "malayalam"),
    ("sinhala", '\u{0d9a}', "sinhala"),
    ("armenian", '\u{0531}', "armenian"),
    ("georgian", '\u{10d0}', "georgian"),
    ("ethiopic", '\u{1200}', "ethiopic"),
    ("cherokee", '\u{13a0}', "cherokee"),
    // Include ornamental and styled characters commonly used in display names.
    ("javanese", '\u{a9c1}', "javanese"),
    ("math", '\u{1d4d0}', "math"),
    ("enclosed", '\u{24b6}', "symbol"),
    ("symbols", '\u{2661}', "symbol"),
];

/// Preferred pan-CJK region by locale prefix, longest first.
const HAN_REGIONS: &[(&str, &str)] = &[
    ("zh_tw", "tc"),
    ("zh_hant", "tc"),
    ("zh_hk", "hk"),
    ("zh_mo", "hk"),
    ("zh", "sc"),
    ("ja", "jp"),
    ("ko", "kr"),
];

/// Region names used by pan-CJK font families.
const HAN_REGION_NAMES: &[&str] = &["sc", "tc", "hk", "jp", "kr"];

/// Maximum font-directory depth, also limiting symlink loops.
const FONT_SCAN_DEPTH: usize = 4;

/// Maximum accepted faces in a collection to reject invalid counts.
const MAX_FACES: u32 = 64;

/// One system fallback per unsupported Inter script.
///
/// The font scan runs once per process, across recreated windows.
pub fn fallbacks() -> &'static [Fallback] {
    &discovery().fallbacks
}

/// Installed upright faces, grouped under their typographic family name.
struct Discovery {
    fallbacks: Vec<Fallback>,
    families: BTreeMap<String, Vec<Face>>,
}

#[derive(Clone, Debug)]
struct Face {
    path: PathBuf,
    index: u32,
    weight: f32,
    stretch: f32,
    weight_range: Option<(f32, f32)>,
}

fn discovery() -> &'static Discovery {
    static FONTS: OnceLock<Discovery> = OnceLock::new();
    FONTS.get_or_init(load)
}

/// Available families in deterministic name order; no font bytes are loaded here.
pub fn families() -> impl Iterator<Item = &'static str> {
    discovery().families.keys().map(String::as_str)
}

fn closest_face(faces: &[Face], weight: f32) -> Option<&Face> {
    faces.iter().min_by(|a, b| {
        let distance = |face: &Face| {
            let actual = face
                .weight_range
                .map_or(face.weight, |(min, max)| weight.clamp(min, max));
            (actual - weight).abs()
        };
        distance(a)
            .total_cmp(&distance(b))
            .then_with(|| {
                (a.stretch - 100.0)
                    .abs()
                    .total_cmp(&(b.stretch - 100.0).abs())
            })
            .then_with(|| a.path.cmp(&b.path))
            .then_with(|| a.index.cmp(&b.index))
    })
}

/// Load all four UI weights atomically. A missing or invalid file uses Inter instead.
pub fn selected(name: &str) -> Option<Vec<egui::FontData>> {
    load_family(discovery().families.get(name)?)
}

fn load_family(faces: &[Face]) -> Option<Vec<egui::FontData>> {
    let mut loaded = BTreeMap::new();
    [400.0, 500.0, 600.0, 700.0]
        .into_iter()
        .map(|weight| {
            let face = closest_face(faces, weight)?;
            let bytes = loaded
                .entry(&face.path)
                .or_insert_with(|| std::fs::read(&face.path).ok())
                .as_ref()?;
            let font = skrifa::FontRef::from_index(bytes, face.index).ok()?;
            // Reject unreadable faces before handing the definitions to egui.
            if !has_text_outlines(&font) {
                return None;
            }
            let mut data = egui::FontData::from_owned(bytes.clone());
            data.index = face.index;
            if let Some((min, max)) = face.weight_range {
                data.tweak.coords =
                    egui::epaint::text::VariationCoords::new([(b"wght", weight.clamp(min, max))]);
            }
            Some(data)
        })
        .collect()
}

/// Exclude bitmap-only faces while allowing fonts for non-Latin scripts.
fn has_text_outlines(font: &skrifa::FontRef<'_>) -> bool {
    let charmap = font.charmap();
    let outlines = font.outline_glyphs();
    std::iter::once('A')
        .chain(FALLBACK_SCRIPTS.iter().map(|(_, probe, _)| *probe))
        .any(|probe| {
            charmap
                .map(probe)
                .is_some_and(|glyph| outlines.get(glyph).is_some())
        })
}

/// Candidate font face and interface-suitability score.
struct Candidate {
    score: u32,
    path: PathBuf,
    index: u32,
}

/// Finds and reads the best installed face for each [`FALLBACK_SCRIPTS`] entry.
fn load() -> Discovery {
    let mut families = BTreeMap::new();
    let han = han_region(&locale());
    let started = std::time::Instant::now();
    let mut best: BTreeMap<&str, Candidate> = BTreeMap::new();
    for dir in font_dirs() {
        probe_dir(&dir, 0, han, &mut best, &mut families);
    }
    log::debug!(
        "probed the system fonts in {:.1} ms, {} of {} scripts covered",
        started.elapsed().as_secs_f32() * 1e3,
        best.len(),
        FALLBACK_SCRIPTS.len()
    );

    // Read and register a face only once when it covers several scripts.
    let mut fonts: Vec<Fallback> = Vec::new();
    let mut taken: Vec<(PathBuf, u32)> = Vec::new();
    for (script, _, _) in FALLBACK_SCRIPTS {
        let Some(candidate) = best.get(script) else {
            log::debug!("no fallback face covers {script}");
            continue;
        };
        if taken.contains(&(candidate.path.clone(), candidate.index)) {
            continue;
        }
        let bytes = match std::fs::read(&candidate.path) {
            Ok(bytes) => bytes,
            Err(error) => {
                log::warn!("cannot read {}: {error}", candidate.path.display());
                continue;
            }
        };
        log::debug!(
            "{script} fallback: {} (face {})",
            candidate.path.display(),
            candidate.index
        );
        taken.push((candidate.path.clone(), candidate.index));
        fonts.push(Fallback {
            name: format!("fallback-{script}"),
            bytes,
            index: candidate.index,
        });
    }
    Discovery {
        fallbacks: fonts,
        families,
    }
}

/// Scans font files below `dir` and keeps the best face per script.
fn probe_dir(
    dir: &Path,
    depth: usize,
    han: &str,
    best: &mut BTreeMap<&str, Candidate>,
    families: &mut BTreeMap<String, Vec<Face>>,
) {
    if depth >= FONT_SCAN_DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        // Resolve file type only for symlinks used by Debian and Flatpak paths.
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if kind.is_dir() || (kind.is_symlink() && path.is_dir()) {
            probe_dir(&path, depth + 1, han, best, families);
        } else if is_font_file(&path) {
            probe_file(&path, han, best, families);
        }
    }
}

/// Whether the path has a supported font extension.
fn is_font_file(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "ttf" | "otf" | "ttc" | "otc"
            )
        })
}

/// Scores each face in a font file for missing scripts.
fn probe_file(
    path: &Path,
    han: &str,
    best: &mut BTreeMap<&str, Candidate>,
    families: &mut BTreeMap<String, Vec<Face>>,
) {
    let Ok(file) = std::fs::File::open(path) else {
        return;
    };
    // Memory-map files so scanning touches only headers, names, and charmaps.
    //
    // Safety: the read-only mapping does not outlive this call. Replacing the
    // file concurrently could fault, as with other platform font scanners.
    let Ok(map) = (unsafe { memmap2::Mmap::map(&file) }) else {
        return;
    };
    let faces: Vec<(u32, skrifa::FontRef)> = match skrifa::raw::FileRef::new(&map) {
        Ok(skrifa::raw::FileRef::Font(font)) => vec![(0, font)],
        Ok(skrifa::raw::FileRef::Collection(collection)) => (0..collection.len().min(MAX_FACES))
            .filter_map(|index| collection.get(index).ok().map(|font| (index, font)))
            .collect(),
        Err(_) => return,
    };
    for (index, font) in faces {
        let attributes = font.attributes();
        if attributes.style != skrifa::attribute::Style::Normal {
            continue;
        }
        let family = font
            .localized_strings(skrifa::string::StringId::FAMILY_NAME)
            .english_or_first()
            .map(|name| name.to_string())
            .unwrap_or_default()
            .to_lowercase();
        let charmap = font.charmap();
        let outlines = font.outline_glyphs();
        // Include script-specific text faces as well as Latin families.
        if has_text_outlines(&font) {
            let name = font
                .localized_strings(skrifa::string::StringId::TYPOGRAPHIC_FAMILY_NAME)
                .english_or_first()
                .or_else(|| {
                    font.localized_strings(skrifa::string::StringId::FAMILY_NAME)
                        .english_or_first()
                })
                .map(|name| name.to_string());
            if let Some(name) = name.filter(|name| !name.trim().is_empty()) {
                let weight_range = font
                    .axes()
                    .iter()
                    .find(|axis| axis.tag() == skrifa::raw::types::Tag::new(b"wght"))
                    .map(|axis| (axis.min_value(), axis.max_value()))
                    .filter(|(min, max)| min.is_finite() && max.is_finite() && min <= max);
                families.entry(name).or_default().push(Face {
                    path: path.to_owned(),
                    index,
                    weight: attributes.weight.value(),
                    stretch: attributes.stretch.percentage(),
                    weight_range,
                });
            }
        }

        for (script, probe, hint) in FALLBACK_SCRIPTS {
            // Require an outline, not only a charmap entry from a bitmap font.
            let covers = charmap
                .map(*probe)
                .is_some_and(|glyph| outlines.get(glyph).is_some());
            if !covers {
                continue;
            }
            let score = face_score(&family, attributes.weight.value(), han, hint);
            // Break score ties by path for deterministic selection.
            if best
                .get(script)
                .is_none_or(|held| (score, path) < (held.score, held.path.as_path()))
            {
                best.insert(
                    script,
                    Candidate {
                        score,
                        path: path.to_path_buf(),
                        index,
                    },
                );
            }
        }
    }
}

/// Scores a face for interface use with `hint`; lower is better.
///
/// Prefer regular sans-serif faces over serif, monospace, and display styles.
fn face_score(family: &str, weight: f32, han: &str, hint: &str) -> u32 {
    let mut score = ((weight - 400.0).abs() / 25.0) as u32;
    // Prefer families designed for the target script over incidental coverage.
    if !family.contains(hint) {
        score += 25;
    }
    // Prefer sans families at interface sizes over specialized display cuts.
    if !family.contains("sans") {
        score += 50;
    }
    for (fragment, penalty) in [
        ("serif", 200),
        ("mono", 120),
        ("kufi", 80),
        ("naskh", 80),
        ("looped", 80),
        ("display", 80),
        ("condensed", 60),
        ("caption", 40),
    ] {
        if family.contains(fragment) {
            score += penalty;
        }
    }
    // Prefer the locale's regional pan-CJK glyph forms.
    if let Some(region) = family
        .rsplit(' ')
        .next()
        .filter(|region| HAN_REGION_NAMES.contains(region))
        && region != han
    {
        score += 40;
    }
    score
}

/// Lowercase user locale, or an empty string when unset.
fn locale() -> String {
    ["LC_ALL", "LC_CTYPE", "LANG"]
        .iter()
        .find_map(|key| std::env::var(key).ok().filter(|value| !value.is_empty()))
        .unwrap_or_default()
        .to_lowercase()
}

/// Preferred pan-CJK region, defaulting to Simplified Chinese.
fn han_region(locale: &str) -> &'static str {
    HAN_REGIONS
        .iter()
        .find(|(prefix, _)| locale.starts_with(prefix))
        .map_or("sc", |(_, region)| *region)
}

/// Where the platform keeps installed fonts.
fn font_dirs() -> Vec<PathBuf> {
    let user = directories::UserDirs::new();
    let mut dirs: Vec<PathBuf> = Vec::new();
    let mut add = |dir: PathBuf| {
        if !dirs.contains(&dir) {
            dirs.push(dir);
        }
    };
    if cfg!(target_os = "macos") {
        add(PathBuf::from("/System/Library/Fonts"));
        add(PathBuf::from("/Library/Fonts"));
    } else if cfg!(target_os = "windows") {
        add(std::env::var_os("SystemRoot")
            .map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from)
            .join("Fonts"));
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            add(PathBuf::from(local).join(r"Microsoft\Windows\Fonts"));
        }
    } else {
        // Include XDG data directories used by distributions such as NixOS and Guix.
        let data_dirs = std::env::var("XDG_DATA_DIRS")
            .ok()
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "/usr/local/share:/usr/share".to_string());
        for dir in data_dirs.split(':').filter(|dir| !dir.is_empty()) {
            add(PathBuf::from(dir).join("fonts"));
        }
        // Flatpak host fonts.
        add(PathBuf::from("/run/host/fonts"));
        // Legacy per-user font directory still supported by fontconfig.
        if let Some(user) = &user {
            add(user.home_dir().join(".fonts"));
        }
    }
    // Add the platform's per-user font directory when it is separate.
    if let Some(font_dir) = user.as_ref().and_then(|user| user.font_dir()) {
        add(font_dir.to_path_buf());
    }
    dirs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn face(path: &str, weight: f32) -> Face {
        Face {
            path: path.into(),
            index: 0,
            weight,
            stretch: 100.0,
            weight_range: None,
        }
    }

    #[test]
    fn chooses_available_weights_and_variable_ranges() {
        let faces = [face("regular", 400.0), face("bold", 700.0)];
        assert_eq!(
            closest_face(&faces, 500.0).unwrap().path,
            Path::new("regular")
        );
        assert_eq!(closest_face(&faces, 600.0).unwrap().path, Path::new("bold"));
        let variable = Face {
            weight_range: Some((100.0, 900.0)),
            ..face("variable", 400.0)
        };
        let faces = [faces[0].clone(), variable];
        assert_eq!(
            closest_face(&faces, 700.0).unwrap().path,
            Path::new("variable")
        );
        let faces = [
            Face {
                stretch: 75.0,
                ..face("condensed", 400.0)
            },
            face("regular", 400.0),
        ];
        assert_eq!(
            closest_face(&faces, 400.0).unwrap().path,
            Path::new("regular")
        );
    }

    #[test]
    fn discovers_and_loads_a_variable_family_and_handles_deleted_fonts() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Inter.ttf");
        std::fs::write(&path, include_bytes!("../assets/fonts/InterVariable.ttf")).unwrap();
        let mut families = BTreeMap::new();
        probe_file(&path, "sc", &mut BTreeMap::new(), &mut families);
        let faces = families.get("Inter Variable").expect("typographic family");
        let fonts = load_family(faces).unwrap();
        assert_eq!(fonts.len(), 4);
        for (font, weight) in fonts.iter().zip([400.0, 500.0, 600.0, 700.0]) {
            assert_eq!(
                font.tweak.coords,
                egui::epaint::text::VariationCoords::new([(b"wght", weight)])
            );
        }
        std::fs::write(&path, b"broken font").unwrap();
        assert!(load_family(faces).is_none());
        std::fs::remove_file(path).unwrap();
        assert!(load_family(faces).is_none());
    }

    #[test]
    fn locales_choose_a_pan_cjk_cut() {
        assert_eq!(han_region("zh_cn.utf-8"), "sc");
        assert_eq!(han_region("zh_tw.utf-8"), "tc");
        assert_eq!(han_region("zh_hk.utf-8"), "hk");
        assert_eq!(han_region("ja_jp.utf-8"), "jp");
        assert_eq!(han_region("ko_kr.utf-8"), "kr");
        assert_eq!(han_region("en_us.utf-8"), "sc", "the default");
        assert_eq!(han_region(""), "sc", "no locale set");
    }

    #[test]
    fn interface_faces_outrank_display_ones() {
        let sans = face_score("noto sans arabic", 400.0, "sc", "arabic");
        assert!(sans < face_score("noto naskh arabic", 400.0, "sc", "arabic"));
        assert!(sans < face_score("noto kufi arabic", 400.0, "sc", "arabic"));
        assert!(sans < face_score("noto nastaliq urdu", 400.0, "sc", "arabic"));
        assert!(sans < face_score("noto serif arabic", 400.0, "sc", "arabic"));
        assert!(sans < face_score("noto sans arabic", 700.0, "sc", "arabic"));
    }

    #[test]
    fn a_face_drawn_for_the_script_wins() {
        // Prefer a script-specific family over incidental glyph coverage.
        assert!(
            face_score("noto sans hebrew", 400.0, "sc", "hebrew")
                < face_score("liberation sans", 400.0, "sc", "hebrew")
        );
    }

    #[test]
    fn the_locale_picks_between_regional_cuts() {
        let simplified = face_score("noto sans cjk sc", 400.0, "sc", "cjk");
        assert!(simplified < face_score("noto sans cjk jp", 400.0, "sc", "cjk"));
        assert_eq!(
            face_score("noto sans cjk tc", 400.0, "tc", "cjk"),
            simplified,
            "each locale ranks its own cut the same"
        );
    }

    /// Lists the fallback selected for each script:
    /// `cargo test system_fonts -- --ignored --nocapture`.
    #[test]
    #[ignore = "reads this machine's fonts"]
    fn which_scripts_have_faces_here() {
        let found = fallbacks();
        eprintln!("{} faces registered:", found.len());
        for fallback in found {
            eprintln!("  {} ({} KB)", fallback.name, fallback.bytes.len() / 1024);
        }
    }

    #[test]
    fn only_font_files_are_probed() {
        assert!(is_font_file(Path::new("/x/NotoSans.ttf")));
        assert!(is_font_file(Path::new("/x/NotoSansCJK.TTC")));
        assert!(is_font_file(Path::new("/x/PingFang.otf")));
        assert!(!is_font_file(Path::new("/x/fonts.dir")));
        assert!(!is_font_file(Path::new("/x/README")));
    }

    #[test]
    fn probing_the_system_never_panics() {
        // The result depends on installed fonts and may be empty.
        let _ = fallbacks();
    }
}
