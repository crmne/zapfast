//! Creates a single-file ZIP without changing the source or uploading anything.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug)]
pub struct ZipCopy {
    pub dir: tempfile::TempDir,
    pub path: PathBuf,
    pub before: u64,
    pub after: u64,
}

pub(crate) fn compress(
    cache: &Path,
    path: &Path,
    cancelled: &AtomicBool,
) -> Result<Option<ZipCopy>, ()> {
    let check = || (!cancelled.load(Ordering::Relaxed)).then_some(()).ok_or(());
    check()?;
    if !path.metadata().map_err(|_| ())?.is_file() {
        return Err(());
    }
    let mut source = std::fs::File::open(path).map_err(|_| ())?;
    let metadata = source.metadata().map_err(|_| ())?;
    if !metadata.is_file() {
        return Err(());
    }
    let before = metadata.len();
    let name = path.file_name().ok_or(())?.to_string_lossy();
    let entry = portable_entry_name(&name);
    std::fs::create_dir_all(cache).map_err(|_| ())?;
    let dir = tempfile::Builder::new()
        .prefix("zip-")
        .tempdir_in(cache)
        .map_err(|_| ())?;
    let staging = dir.path().join(".compression.part");
    let mut writer = zip::ZipWriter::new(std::fs::File::create(&staging).map_err(|_| ())?);
    writer
        .start_file(
            entry,
            zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated)
                .large_file(before >= u64::from(u32::MAX)),
        )
        .map_err(|_| ())?;
    let mut remaining = before;
    let mut chunk = vec![0; 64 * 1024];
    while remaining > 0 {
        check()?;
        let to_read = remaining.min(chunk.len() as u64) as usize;
        source.read_exact(&mut chunk[..to_read]).map_err(|_| ())?;
        writer.write_all(&chunk[..to_read]).map_err(|_| ())?;
        remaining -= to_read as u64;
    }
    if source.read(&mut chunk[..1]).map_err(|_| ())? != 0 {
        return Err(());
    }
    let file = writer.finish().map_err(|_| ())?;
    let after = file.metadata().map_err(|_| ())?.len();
    drop(file);
    if after >= before {
        return Ok(None);
    }
    check()?;
    let path = dir.path().join(format!("{entry}.zip"));
    std::fs::rename(staging, &path).map_err(|_| ())?;
    check()?;
    Ok(Some(ZipCopy {
        dir,
        path,
        before,
        after,
    }))
}

/// A single portable ZIP entry name, not an absolute path or a directory.
fn portable_entry_name(name: &str) -> String {
    let entry: String = name
        .chars()
        .map(|c| {
            if c.is_control() || "/\\:<>\"|?*".contains(c) {
                '_'
            } else {
                c
            }
        })
        .scan(0, |bytes, c| {
            *bytes += c.len_utf8();
            (*bytes <= 200).then_some(c)
        })
        .collect();
    let entry = entry.trim_matches([' ', '.']);
    if entry.is_empty() {
        "attachment".to_owned()
    } else if is_reserved_device_name(entry) {
        // Windows refuses to create or extract `CON`, `COM1` and the like,
        // even with an extension, so keep the name portable.
        format!("_{entry}")
    } else {
        entry.to_owned()
    }
}

/// Windows reserves these device names, with or without an extension.
fn is_reserved_device_name(entry: &str) -> bool {
    let stem = entry.split('.').next().unwrap_or(entry);
    let upper = stem.to_ascii_uppercase();
    matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (upper.len() == 4
            && (upper.starts_with("COM") || upper.starts_with("LPT"))
            && matches!(upper.as_bytes()[3], b'1'..=b'9'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_portable_entry_name_avoids_windows_device_names() {
        assert_eq!(portable_entry_name("CON"), "_CON");
        assert_eq!(portable_entry_name("con.txt"), "_con.txt");
        assert_eq!(portable_entry_name(".CON.txt"), "_CON.txt");
        assert_eq!(portable_entry_name("COM1.log"), "_COM1.log");
        assert_eq!(portable_entry_name("lpt9"), "_lpt9");
        assert_eq!(portable_entry_name("COM0"), "COM0");
        assert_eq!(portable_entry_name("COM10"), "COM10");
        assert_eq!(portable_entry_name("console.txt"), "console.txt");
        assert_eq!(portable_entry_name("a/b:c"), "a_b_c");
        assert_eq!(portable_entry_name("..."), "attachment");
    }

    #[test]
    fn only_smaller_zip_copies_survive_and_the_source_is_unchanged() {
        let root = tempfile::tempdir().unwrap();
        let cache = root.path().join("media");
        let path = root.path().join("fixture.txt");
        let bytes = vec![b'a'; 150_000];
        std::fs::write(&path, &bytes).unwrap();
        let copy = compress(&cache, &path, &AtomicBool::new(false))
            .unwrap()
            .unwrap();
        assert!(copy.after < copy.before);
        let mut archive = zip::ZipArchive::new(std::fs::File::open(&copy.path).unwrap()).unwrap();
        let mut extracted = Vec::new();
        archive
            .by_name("fixture.txt")
            .unwrap()
            .read_to_end(&mut extracted)
            .unwrap();
        assert_eq!(extracted, bytes);
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        drop(archive);
        drop(copy);
        std::fs::write(&path, b"tiny").unwrap();
        assert!(
            compress(&cache, &path, &AtomicBool::new(false))
                .unwrap()
                .is_none()
        );
        assert!(compress(&cache, &path, &AtomicBool::new(true)).is_err());
        assert_eq!(std::fs::read_dir(cache).unwrap().count(), 0);
    }
}
