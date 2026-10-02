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
    path: PathBuf,
    /// The font has a `wght` axis, so each weight gets its own coordinate.
    variable_weight: bool,
}

/// Checks that `bytes` hold a font that maps characters, and tells whether it
/// has a `wght` axis. For a collection, the first font is the one used.
fn validate(bytes: &[u8]) -> Result<bool, String> {
    let font = skrifa::FontRef::from_index(bytes, 0).map_err(|_| "not a font".to_owned())?;
    if font.charmap().mappings().next().is_none() {
        return Err("font has no characters".to_owned());
    }
    let wght = skrifa::Tag::new(b"wght");
    Ok(font.axes().iter().any(|axis| axis.tag() == wght))
}

/// Reads and validates the font file at `path`.
pub fn load(path: &Path) -> Result<Face, String> {
    let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
    Ok(Face {
        path: path.to_owned(),
        variable_weight: validate(&bytes)?,
    })
}

/// Marks `request` as the newest font action, so an import still running for
/// an older one is dropped instead of applied. Call it when the action is
/// issued, before the dialog opens.
pub fn claim(request: u64) {
    LATEST.store(request, Ordering::SeqCst);
}

/// The newest font action, and the lock that keeps imports and clean-up from
/// touching the font directory at once.
static LATEST: AtomicU64 = AtomicU64::new(0);
static DIRECTORY: Mutex<()> = Mutex::new(());

/// Copies a chosen font file into a new folder of ZapFast's font directory,
/// so the original may move or go. Earlier copies are left alone, so an
/// import that loses to a newer request can never delete the font in use;
/// [`keep_only`] clears them once a result is applied.
/// `None`: a newer request arrived first, and nothing was kept.
pub fn import(source: &Path, dirs: &AppDirs, request: u64) -> Option<Result<PathBuf, String>> {
    let bytes = match std::fs::read(source) {
        Ok(bytes) => bytes,
        Err(error) => return Some(Err(error.to_string())),
    };
    if let Err(error) = validate(&bytes) {
        return Some(Err(error));
    }
    let Some(name) = source.file_name() else {
        return Some(Err("not a file".to_owned()));
    };
    let _guard = DIRECTORY.lock().unwrap_or_else(|e| e.into_inner());
    if LATEST.load(Ordering::SeqCst) != request {
        return None;
    }
    let staged = stage(name, &bytes, dirs);
    if LATEST.load(Ordering::SeqCst) != request {
        if let Ok(target) = &staged
            && let Some(folder) = target.parent()
        {
            let _ = std::fs::remove_dir_all(folder);
        }
        return None;
    }
    Some(staged)
}

/// Writes `bytes` as `name` in a folder no earlier import used.
fn stage(name: &std::ffi::OsStr, bytes: &[u8], dirs: &AppDirs) -> Result<PathBuf, String> {
    let root = dirs.custom_font_dir();
    std::fs::create_dir_all(&root).map_err(|error| error.to_string())?;
    let mut number = 0u32;
    let folder = loop {
        let folder = root.join(number.to_string());
        match std::fs::create_dir(&folder) {
            Ok(()) => break folder,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => number += 1,
            Err(error) => return Err(error.to_string()),
        }
    };
    let target = folder.join(name);
    std::fs::write(&target, bytes).map_err(|error| {
        let _ = std::fs::remove_dir_all(&folder);
        error.to_string()
    })?;
    Ok(target)
}

/// Deletes every copied font except the one at `keep`. Run it after a
/// result is applied, so what the settings point to is all that remains.
pub fn keep_only(dirs: &AppDirs, keep: &Path) {
    let root = dirs.custom_font_dir();
    let Some(current) = keep.parent().filter(|_| keep.starts_with(&root)) else {
        return;
    };
    let _guard = DIRECTORY.lock().unwrap_or_else(|e| e.into_inner());
    let Ok(entries) = std::fs::read_dir(&root) else {
        return;
    };
    for path in entries.flatten().map(|entry| entry.path()) {
        if path != current {
            let _ = std::fs::remove_dir_all(&path).or_else(|_| std::fs::remove_file(&path));
        }
    }
}

/// Deletes ZapFast's copies of the custom font.
pub fn remove(dirs: &AppDirs) {
    let _guard = DIRECTORY.lock().unwrap_or_else(|e| e.into_inner());
    match std::fs::remove_dir_all(dirs.custom_font_dir()) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => log::debug!("could not delete the custom font: {error}"),
    }
}

impl Face {
    /// Puts this font first in each interface family, so it draws what it
    /// has and the families' own fonts draw the rest. The file is read again
    /// here, so only egui holds the font's bytes while it is installed.
    pub fn lead(&self, fonts: &mut egui::FontDefinitions) {
        use fastframe_fonts::Weight;
        let bytes = match std::fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(error) => {
                log::warn!("custom font not read: {error}");
                return;
            }
        };
        if self.variable_weight {
            // egui's font data owns its bytes, so each weight holds a copy.
            for weight in Weight::ALL {
                let mut data = egui::FontData::from_owned(bytes.clone());
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
            let data = std::sync::Arc::new(egui::FontData::from_owned(bytes));
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

    fn written(root: &Path, name: &str, bytes: &[u8]) -> PathBuf {
        let path = root.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn load_tells_static_from_variable_fonts() {
        let root = tempfile::tempdir().unwrap();
        let fixed = written(root.path(), "fixed.ttf", &static_font());
        assert!(!load(&fixed).unwrap().variable_weight);
        let inter = written(root.path(), "inter.ttf", fastframe_fonts::INTER);
        assert!(load(&inter).unwrap().variable_weight);
        let text = written(root.path(), "text.ttf", b"not a font");
        assert!(load(&text).is_err());
    }

    #[test]
    fn a_stale_import_never_deletes_the_font_in_use() {
        let root = tempfile::tempdir().unwrap();
        let dirs = AppDirs::under(&root.path().join("app"));
        let chosen = |name: &str| written(root.path(), name, &static_font());

        claim(1);
        let first = import(&chosen("a.ttf"), &dirs, 1).unwrap().unwrap();
        keep_only(&dirs, &first);

        // A same-named font from a newer pick does not overwrite the copy.
        claim(2);
        let second = import(&chosen("a.ttf"), &dirs, 2).unwrap().unwrap();
        assert_ne!(first, second);
        assert!(first.exists() && second.exists());

        // Request 2 is canceled by a third, whose dialog is then dismissed:
        // the import overtaken by it is dropped, and the copy in use stays.
        claim(3);
        assert!(import(&chosen("c.ttf"), &dirs, 2).is_none());
        assert!(first.exists());
        assert!(!first.parent().unwrap().join("c.ttf").exists());

        // Applying a result clears every other copy.
        keep_only(&dirs, &second);
        assert!(!first.exists());
        assert!(second.exists());
        let folders = std::fs::read_dir(dirs.custom_font_dir()).unwrap().count();
        assert_eq!(folders, 1);

        // A rejected font leaves the copy alone.
        claim(4);
        let text = written(root.path(), "x.ttf", b"plain text");
        assert!(import(&text, &dirs, 4).unwrap().is_err());
        assert!(second.exists());

        // A path outside the font directory prunes nothing.
        keep_only(&dirs, &root.path().join("elsewhere").join("b.ttf"));
        assert!(second.exists());

        // Choosing System or Inter removes the copy.
        remove(&dirs);
        assert!(!dirs.custom_font_dir().exists());
    }

    #[test]
    fn static_font_leads_every_weight_with_one_face() {
        let root = tempfile::tempdir().unwrap();
        let mut fonts = definitions();
        load(&written(root.path(), "fixed.ttf", &static_font()))
            .unwrap()
            .lead(&mut fonts);
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
        let root = tempfile::tempdir().unwrap();
        let mut fonts = definitions();
        load(&written(root.path(), "inter.ttf", fastframe_fonts::INTER))
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
