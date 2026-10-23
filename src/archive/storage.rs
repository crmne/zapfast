//! Account media disk usage, without reading file contents or following links.

use std::path::Path;

/// Result returned to the worker after a bounded off-thread usage scan.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MediaScanResult {
    pub bytes: u64,
}

pub(crate) fn is_link(metadata: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        // Junctions and other reparse points can redirect directory traversal
        // too, although they are not necessarily reported as symbolic links.
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

/// Measure the managed media folder, including staging and orphaned files.
/// Fail closed on unreadable entries so recovery cannot bypass a storage limit.
pub fn managed_media_bytes(root: &Path) -> std::io::Result<u64> {
    managed_media_bytes_with(root, |_| {})
}

fn managed_media_bytes_with(
    root: &Path,
    mut before_folder: impl FnMut(&Path),
) -> std::io::Result<u64> {
    let metadata = match std::fs::symlink_metadata(root) {
        Ok(metadata) => metadata,
        // Account setup creates the cache parent, while its media child is
        // created only by the first download. A fresh missing root is known
        // empty; disappearance after traversal starts is handled below.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(error),
    };
    if is_link(&metadata) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Managed media folder cannot be a symbolic link",
        ));
    }
    let mut folders = vec![root.to_path_buf()];
    let mut total = 0u64;
    while let Some(folder) = folders.pop() {
        before_folder(&folder);
        let entries = match std::fs::read_dir(&folder) {
            Ok(entries) => entries,
            Err(error) if folder != root && error.kind() == std::io::ErrorKind::NotFound => {
                continue;
            }
            Err(error) => return Err(error),
        };
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error),
            };
            let metadata = match entry.path().symlink_metadata() {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error),
            };
            if is_link(&metadata) {
                continue;
            }
            if metadata.is_dir() {
                folders.push(entry.path());
            } else if metadata.is_file() {
                total = total
                    .checked_add(metadata.len())
                    .ok_or_else(|| std::io::Error::other("Managed media size overflow"))?;
            }
        }
    }
    Ok(total)
}

pub fn scan_managed_media(root: &Path) -> std::io::Result<MediaScanResult> {
    managed_media_bytes(root).map(|bytes| MediaScanResult { bytes })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_actual_nested_files_and_unreferenced_staging() {
        let root = tempfile::tempdir().unwrap();
        assert_eq!(managed_media_bytes(&root.path().join("absent")).unwrap(), 0);
        std::fs::create_dir(root.path().join("nested")).unwrap();
        std::fs::write(root.path().join("nested/file"), b"1234").unwrap();
        std::fs::write(root.path().join(".staging"), b"123").unwrap();
        assert_eq!(managed_media_bytes(root.path()).unwrap(), 7);
        assert!(root.path().join("nested/file").exists());
    }

    #[test]
    fn a_fresh_root_is_empty_and_a_vanished_nested_folder_is_a_race() {
        let root = tempfile::tempdir().unwrap();
        assert_eq!(
            managed_media_bytes(&root.path().join("missing")).unwrap(),
            0
        );
        let nested = root.path().join("nested");
        std::fs::create_dir(&nested).unwrap();
        std::fs::write(nested.join("gone"), b"1234").unwrap();
        std::fs::write(root.path().join("kept"), b"123").unwrap();

        let mut removed = false;
        let bytes = managed_media_bytes_with(root.path(), |folder| {
            if folder == nested && !removed {
                std::fs::remove_dir_all(&nested).unwrap();
                removed = true;
            }
        })
        .unwrap();
        assert_eq!(bytes, 3);

        let file = root.path().join("not-a-folder");
        std::fs::write(&file, b"x").unwrap();
        assert!(managed_media_bytes(&file).is_err());
    }
}
