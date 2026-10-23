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
    let metadata = match std::fs::symlink_metadata(root) {
        Ok(metadata) => metadata,
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
        for entry in std::fs::read_dir(folder)? {
            let entry = entry?;
            let metadata = entry.path().symlink_metadata()?;
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
}
