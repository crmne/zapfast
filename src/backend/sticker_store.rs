//! Sticker files on disk: saved stickers and sticker packs.
//!
//! Saved stickers are WebP files named by their content hash. Every pack is a
//! folder of WebP files under `packs/`. A pack put together in ZapFast is
//! marked local in its `pack.json` and names each file by its content hash, so
//! a sticker belongs to a local pack by what it is, not where it came from:
//! the same picture filed from Saved, Recent or another pack is one member,
//! and moving the profile keeps every pack intact. Deleting a local pack
//! removes its copies only.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::model::StickerPack;

/// The file that describes a pack's name, kind, and sticker order.
const MANIFEST: &str = "pack.json";

/// A pack's `pack.json`. Imported packs written before it existed have none.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
struct Manifest {
    name: String,
    /// Put together in ZapFast rather than imported.
    #[serde(default)]
    local: bool,
    /// Creation time in Unix seconds, which orders local packs.
    #[serde(default)]
    created: i64,
    /// File names in the order the stickers were added.
    #[serde(default)]
    stickers: Vec<String>,
}

fn read_manifest(dir: &Path) -> Option<Manifest> {
    serde_json::from_slice(&std::fs::read(dir.join(MANIFEST)).ok()?).ok()
}

fn write_manifest(dir: &Path, manifest: &Manifest) -> Result<(), String> {
    let json = serde_json::to_vec_pretty(manifest).map_err(|error| error.to_string())?;
    std::fs::write(dir.join(MANIFEST), json).map_err(|error| error.to_string())
}

/// Lowercase hex SHA-256 of a sticker's bytes, its identity everywhere.
pub fn content_hash(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn is_webp(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension == "webp")
}

fn modified(path: &Path) -> std::time::SystemTime {
    std::fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
}

/// Every pack under `root`, newest first. Local packs keep their creation
/// order even as stickers are added, and an empty local pack still lists so
/// it can receive its first sticker.
pub fn packs(root: &Path) -> Vec<StickerPack> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut packs: Vec<(i64, StickerPack)> = entries
        .flatten()
        .filter_map(|entry| {
            let dir = entry.path();
            if !dir.is_dir() {
                return None;
            }
            let manifest = read_manifest(&dir);
            let mut stickers: Vec<PathBuf> = std::fs::read_dir(&dir)
                .ok()?
                .flatten()
                .map(|file| file.path())
                .filter(|path| is_webp(path))
                .collect();
            stickers.sort();
            let local = manifest.as_ref().is_some_and(|manifest| manifest.local);
            if let Some(manifest) = &manifest {
                // Listed files first, in the order they were added.
                let rank = |path: &PathBuf| {
                    let name = path.file_name().map(|name| name.to_string_lossy());
                    manifest
                        .stickers
                        .iter()
                        .position(|listed| Some(listed.as_str()) == name.as_deref())
                        .unwrap_or(usize::MAX)
                };
                stickers.sort_by_key(rank);
            }
            if stickers.is_empty() && !local {
                return None;
            }
            let when = match &manifest {
                Some(manifest) if manifest.local => manifest.created,
                _ => modified(&dir)
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |age| age.as_secs() as i64),
            };
            let name = manifest
                .map(|manifest| manifest.name)
                .filter(|name| !name.trim().is_empty())
                .unwrap_or_else(|| entry.file_name().to_string_lossy().into_owned());
            Some((
                when,
                StickerPack {
                    name,
                    dir,
                    stickers,
                    local,
                },
            ))
        })
        .collect();
    // Equal times keep a stable order by name.
    packs.sort_by(|(a_when, a), (b_when, b)| b_when.cmp(a_when).then(a.name.cmp(&b.name)));
    packs.into_iter().map(|(_, pack)| pack).collect()
}

/// Creates an empty local pack and returns its folder.
pub fn create_local_pack(root: &Path, name: &str, now: i64) -> Result<PathBuf, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("A pack needs a name".to_owned());
    }
    let dir = super::sticker_import::unique_pack_dir(root, name)?;
    write_manifest(
        &dir,
        &Manifest {
            name: name.to_owned(),
            local: true,
            created: now,
            stickers: Vec::new(),
        },
    )?;
    Ok(dir)
}

/// Files a sticker into a local pack under its content hash, or takes it out.
/// Imported packs stay as they were imported.
pub fn set_member(pack: &Path, sticker: &Path, member: bool) -> Result<(), String> {
    let mut manifest = read_manifest(pack)
        .filter(|manifest| manifest.local)
        .ok_or("Only packs made in ZapFast can change")?;
    let bytes = std::fs::read(sticker).map_err(|error| error.to_string())?;
    let file = format!("{}.webp", content_hash(&bytes));
    let target = pack.join(&file);
    if member {
        if !target.exists() {
            std::fs::write(&target, &bytes).map_err(|error| error.to_string())?;
        }
        if !manifest.stickers.contains(&file) {
            manifest.stickers.push(file);
        }
    } else {
        if target.exists() {
            std::fs::remove_file(&target).map_err(|error| error.to_string())?;
        }
        manifest.stickers.retain(|listed| *listed != file);
    }
    write_manifest(pack, &manifest)
}

/// Saved sticker files, newest first.
pub fn saved(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut saved: Vec<(std::time::SystemTime, PathBuf)> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| is_webp(path))
        .map(|path| (modified(&path), path))
        .collect();
    saved.sort_by_key(|(when, _)| std::cmp::Reverse(*when));
    saved.into_iter().map(|(_, path)| path).collect()
}

/// Saves a sticker under its content hash to deduplicate copies, and
/// returns that hash.
pub fn save(dir: &Path, path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
    std::fs::create_dir_all(dir).map_err(|error| error.to_string())?;
    let hash = content_hash(&bytes);
    let target = dir.join(format!("{hash}.webp"));
    if !target.exists() {
        std::fs::write(&target, &bytes).map_err(|error| error.to_string())?;
    }
    Ok(hash)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sticker(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
        std::fs::create_dir_all(dir).expect("dirs");
        let path = dir.join(name);
        std::fs::write(&path, bytes).expect("writes");
        path
    }

    #[test]
    fn a_sticker_joins_a_local_pack_by_its_content() {
        let root = tempfile::tempdir().expect("temp");
        let packs_root = root.path().join("packs");
        let pack = create_local_pack(&packs_root, "  Bom dia  ", 10).expect("creates");
        let listed = packs(&packs_root);
        assert_eq!(listed.len(), 1, "an empty local pack still lists");
        assert_eq!(listed[0].name, "Bom dia");
        assert!(listed[0].local);
        assert!(listed[0].stickers.is_empty());

        // The same picture from two places is one member.
        let cached = sticker(&root.path().join("cache"), "msg-1.webp", b"sun");
        let copy = sticker(&root.path().join("other"), "000.webp", b"sun");
        set_member(&pack, &cached, true).expect("adds");
        set_member(&pack, &copy, true).expect("adds again");
        let moon = sticker(&root.path().join("cache"), "msg-2.webp", b"moon");
        set_member(&pack, &moon, true).expect("adds");
        let stickers = &packs(&packs_root)[0].stickers;
        assert_eq!(
            stickers
                .iter()
                .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
                .collect::<Vec<_>>(),
            vec![
                format!("{}.webp", content_hash(b"sun")),
                format!("{}.webp", content_hash(b"moon")),
            ],
            "members keep the order they were added in"
        );

        // Removing through a different copy of the picture still removes it,
        // and the source files stay where they were.
        set_member(&pack, &copy, false).expect("removes");
        assert_eq!(packs(&packs_root)[0].stickers.len(), 1);
        assert!(cached.exists() && copy.exists());
    }

    #[test]
    fn imported_packs_do_not_change_and_empty_ones_are_hidden() {
        let root = tempfile::tempdir().expect("temp");
        let packs_root = root.path().join("packs");
        let imported = packs_root.join("Frogs");
        sticker(&imported, "000.webp", b"frog");
        std::fs::create_dir_all(packs_root.join("Empty")).expect("dirs");
        let loose = sticker(root.path(), "x.webp", b"x");
        assert!(set_member(&imported, &loose, true).is_err());
        let listed = packs(&packs_root);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "Frogs");
        assert!(!listed[0].local);
    }

    #[test]
    fn local_packs_keep_their_creation_order() {
        let root = tempfile::tempdir().expect("temp");
        let packs_root = root.path().join("packs");
        let first = create_local_pack(&packs_root, "Futebol", 10).expect("creates");
        create_local_pack(&packs_root, "Trabalho", 20).expect("creates");
        // Filing into the older pack must not move it ahead.
        let loose = sticker(&root.path().join("loose"), "a.webp", b"a");
        set_member(&first, &loose, true).expect("adds");
        let names: Vec<String> = packs(&packs_root)
            .into_iter()
            .map(|pack| pack.name)
            .collect();
        assert_eq!(names, vec!["Trabalho", "Futebol"]);
        // The same name makes a second pack, not a merge.
        create_local_pack(&packs_root, "Futebol", 30).expect("creates");
        assert_eq!(packs(&packs_root).len(), 3);
        assert!(create_local_pack(&packs_root, "   ", 40).is_err());
    }

    #[test]
    fn saving_twice_keeps_one_copy() {
        let root = tempfile::tempdir().expect("temp");
        let source = sticker(&root.path().join("cache"), "a.webp", b"a");
        let saved_dir = root.path().join("saved");
        save(&saved_dir, &source).expect("saves");
        save(&saved_dir, &source).expect("saves");
        assert_eq!(saved(&saved_dir).len(), 1);
    }
}
