//! A font file the person chose in Settings to lead the interface fonts.
//!
//! ZapFast keeps its own copy in the state directory, so the original may
//! move or go, the same way it keeps the chat wallpaper image.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use skrifa::MetadataProvider;

use crate::paths::AppDirs;

/// File extensions the font picker offers.
pub const EXTENSIONS: [&str; 4] = ["ttf", "otf", "ttc", "otc"];

/// Where the file picker starts: the first of the platform's installed font
/// folders that exists.
pub fn system_directory() -> Option<PathBuf> {
    let windows = std::env::var_os("WINDIR")
        .map_or_else(|| PathBuf::from("C:\\Windows"), PathBuf::from)
        .join("Fonts");
    let candidates: Vec<PathBuf> = if cfg!(target_os = "windows") {
        vec![windows]
    } else if cfg!(target_os = "macos") {
        vec!["/Library/Fonts".into(), "/System/Library/Fonts".into()]
    } else {
        vec!["/usr/share/fonts".into(), "/usr/local/share/fonts".into()]
    };
    candidates.into_iter().find(|directory| directory.is_dir())
}

/// A validated font file, ready to lead the interface font families.
pub struct Face {
    bytes: Vec<u8>,
    /// The font has a `wght` axis, so each weight gets its own coordinate.
    variable_weight: bool,
}

/// Checks that `bytes` hold a font that maps characters. For a collection,
/// the first font is the one used.
pub fn parse(bytes: Vec<u8>) -> Result<Face, String> {
    let variable_weight = {
        let font = skrifa::FontRef::from_index(&bytes, 0).map_err(|_| "not a font".to_owned())?;
        if font.charmap().mappings().next().is_none() {
            return Err("font has no characters".to_owned());
        }
        let wght = skrifa::Tag::new(b"wght");
        font.axes().iter().any(|axis| axis.tag() == wght)
    };
    Ok(Face {
        bytes,
        variable_weight,
    })
}

/// Reads and validates the font file at `path`.
pub fn load(path: &Path) -> Result<Face, String> {
    parse(std::fs::read(path).map_err(|error| error.to_string())?)
}

/// Marks `request` as the newest font action, so an import still running for
/// an older one is dropped instead of applied.
pub fn claim(request: u64) {
    LATEST.store(request, Ordering::SeqCst);
}

/// The newest font action, and the lock that keeps imports and removals
/// from touching the font directory at once.
static LATEST: AtomicU64 = AtomicU64::new(0);
static DIRECTORY: Mutex<()> = Mutex::new(());

/// Copies a chosen font file into ZapFast's font directory, so the original
/// may move or go. Any earlier copy is removed once the new one is in place.
/// `None`: a newer request arrived first, and nothing was written.
pub fn import(source: &Path, dirs: &AppDirs, request: u64) -> Option<Result<PathBuf, String>> {
    let bytes = match std::fs::read(source) {
        Ok(bytes) => bytes,
        Err(error) => return Some(Err(error.to_string())),
    };
    let _guard = DIRECTORY.lock().unwrap_or_else(|e| e.into_inner());
    if LATEST.load(Ordering::SeqCst) != request {
        return None;
    }
    Some(copy(source, bytes, dirs))
}

fn copy(source: &Path, bytes: Vec<u8>, dirs: &AppDirs) -> Result<PathBuf, String> {
    parse(bytes.clone())?;
    let name = source.file_name().ok_or_else(|| "not a file".to_owned())?;
    let directory = dirs.custom_font_dir();
    std::fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    let target = directory.join(name);
    let temporary = directory.join("import.tmp");
    std::fs::write(&temporary, bytes)
        .and_then(|()| std::fs::rename(&temporary, &target))
        .map_err(|error| {
            let _ = std::fs::remove_file(&temporary);
            error.to_string()
        })?;
    if let Ok(entries) = std::fs::read_dir(&directory) {
        for entry in entries.flatten().filter(|entry| entry.path() != target) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
    Ok(target)
}

/// Deletes ZapFast's copy of the custom font, and with it any import still
/// running for an older request.
pub fn remove(dirs: &AppDirs, request: u64) {
    claim(request);
    let _guard = DIRECTORY.lock().unwrap_or_else(|e| e.into_inner());
    match std::fs::remove_dir_all(dirs.custom_font_dir()) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => log::debug!("could not delete the custom font: {error}"),
    }
}

impl Face {
    /// Puts this font first in each interface family, so it draws what it
    /// has and the families' own fonts draw the rest.
    pub fn lead(&self, fonts: &mut egui::FontDefinitions) {
        use fastframe_fonts::Weight;
        if self.variable_weight {
            for weight in Weight::ALL {
                let mut data = egui::FontData::from_owned(self.bytes.clone());
                data.tweak.coords =
                    egui::epaint::text::VariationCoords::new([(b"wght", weight.value())]);
                lead_family(
                    fonts,
                    weight,
                    format!("zapfast-custom-{}", weight.name()),
                    data,
                );
            }
        } else {
            let data = std::sync::Arc::new(egui::FontData::from_owned(self.bytes.clone()));
            fonts.font_data.insert(CUSTOM_KEY.to_owned(), data);
            for weight in Weight::ALL {
                if let Some(family) = fonts.families.get_mut(&weight.family()) {
                    family.insert(0, CUSTOM_KEY.to_owned());
                }
            }
        }
    }
}

/// Key of a static font's data, shared by all four weights.
const CUSTOM_KEY: &str = "zapfast-custom";

fn lead_family(
    fonts: &mut egui::FontDefinitions,
    weight: fastframe_fonts::Weight,
    key: String,
    data: egui::FontData,
) {
    fonts
        .font_data
        .insert(key.clone(), std::sync::Arc::new(data));
    if let Some(family) = fonts.families.get_mut(&weight.family()) {
        family.insert(0, key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fastframe_fonts::Weight;

    fn static_font() -> Vec<u8> {
        egui::FontDefinitions::default().font_data["Ubuntu-Light"]
            .font
            .to_vec()
    }

    fn definitions() -> egui::FontDefinitions {
        fastframe_fonts::FontSetup::default().definitions()
    }

    #[test]
    fn parse_tells_static_from_variable_fonts() {
        assert!(!parse(static_font()).unwrap().variable_weight);
        assert!(
            parse(fastframe_fonts::INTER.to_vec())
                .unwrap()
                .variable_weight
        );
        assert!(parse(b"not a font".to_vec()).is_err());
    }

    #[test]
    fn import_keeps_only_the_latest_font() {
        let root = tempfile::tempdir().unwrap();
        let dirs = AppDirs::under(&root.path().join("app"));
        let chosen = |name: &str| {
            let path = root.path().join(name);
            std::fs::write(&path, static_font()).unwrap();
            path
        };
        claim(1);
        import(&chosen("a.ttf"), &dirs, 1).unwrap().unwrap();
        let copy = import(&chosen("b.ttf"), &dirs, 1).unwrap().unwrap();
        assert_eq!(copy, dirs.custom_font_dir().join("b.ttf"));
        let names: Vec<_> = std::fs::read_dir(dirs.custom_font_dir())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names, ["b.ttf"]);

        let text = root.path().join("x.ttf");
        std::fs::write(&text, "plain text").unwrap();
        assert!(import(&text, &dirs, 1).unwrap().is_err());
        assert!(!dirs.custom_font_dir().join("x.ttf").exists());
        assert!(copy.exists(), "a rejected font keeps the earlier copy");

        claim(2);
        assert!(
            import(&chosen("c.ttf"), &dirs, 1).is_none(),
            "an import overtaken by a newer request is dropped"
        );
        assert!(!dirs.custom_font_dir().join("c.ttf").exists());
        assert!(copy.exists());

        remove(&dirs, 3);
        assert!(
            import(&chosen("d.ttf"), &dirs, 2).is_none(),
            "removing the font also cancels an import still running"
        );
        assert!(!dirs.custom_font_dir().exists());
    }

    #[test]
    fn static_font_leads_every_weight_with_one_face() {
        let mut fonts = definitions();
        parse(static_font()).unwrap().lead(&mut fonts);
        assert_eq!(
            fonts
                .font_data
                .keys()
                .filter(|key| key.starts_with("zapfast-custom"))
                .count(),
            1
        );
        for weight in Weight::ALL {
            assert_eq!(fonts.families[&weight.family()][0], CUSTOM_KEY);
        }
    }

    #[test]
    fn variable_font_leads_each_weight_at_its_coordinate() {
        let mut fonts = definitions();
        parse(fastframe_fonts::INTER.to_vec())
            .unwrap()
            .lead(&mut fonts);
        for weight in Weight::ALL {
            let key = format!("zapfast-custom-{}", weight.name());
            assert_eq!(fonts.families[&weight.family()][0], key);
            assert_eq!(
                fonts.font_data[&key].tweak.coords,
                egui::epaint::text::VariationCoords::new([(b"wght", weight.value())])
            );
        }
    }
}
