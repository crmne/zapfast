//! Safe, readable paths for attachments saved outside the account cache.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::settings::DownloadOrganization;

/// Publish a verified local copy without replacing an existing user file.
/// A retry recognizes an identical published copy after a crash before SQL ACK.
#[cfg(test)]
pub(super) fn copy_verified(
    source: &Path,
    root: &Path,
    destination: &Path,
) -> std::io::Result<PathBuf> {
    copy_verified_when(source, root, destination, || true)
}

/// Checks whether a copy is still allowed during staging and before publication.
pub(super) fn copy_verified_when(
    source: &Path,
    root: &Path,
    destination: &Path,
    allowed: impl Fn() -> bool,
) -> std::io::Result<PathBuf> {
    copy_verified_with_gate(source, root, destination, allowed, None)
}

/// Serialize managed publication with revocation through the account's shared gate.
pub(super) fn copy_verified_guarded(
    source: &Path,
    root: &Path,
    destination: &Path,
    allowed: impl Fn() -> bool,
    publication: &std::sync::Mutex<()>,
) -> std::io::Result<PathBuf> {
    copy_verified_with_gate(source, root, destination, allowed, Some(publication))
}

/// Stages and verifies a copy, then publishes without overwriting under an optional revocation gate.
fn copy_verified_with_gate(
    source: &Path,
    root: &Path,
    destination: &Path,
    allowed: impl Fn() -> bool,
    publication: Option<&std::sync::Mutex<()>>,
) -> std::io::Result<PathBuf> {
    use std::io::{Error, ErrorKind, Read, Write};
    let check = || {
        if allowed() {
            Ok(())
        } else {
            Err(Error::new(
                ErrorKind::Interrupted,
                "Attachment saving cancelled",
            ))
        }
    };
    check()?;
    /// Rejects symlinks and special files before reading or creating an export.
    fn plain(path: &Path) -> std::io::Result<std::fs::Metadata> {
        let metadata = std::fs::symlink_metadata(path)?;
        if metadata.file_type().is_symlink() {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                "Linked export paths are not allowed",
            ));
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes() & 0x400 != 0 {
                return Err(Error::new(
                    ErrorKind::InvalidInput,
                    "Linked export paths are not allowed",
                ));
            }
        }
        Ok(metadata)
    }
    /// Hashes a file without holding its entire contents in memory.
    fn digest(path: &Path) -> std::io::Result<[u8; 32]> {
        let mut input = std::fs::File::open(path)?;
        let mut hash = Sha256::new();
        let mut bytes = [0u8; 64 * 1024];
        loop {
            let count = input.read(&mut bytes)?;
            if count == 0 {
                break;
            }
            hash.update(&bytes[..count]);
        }
        Ok(hash.finalize().into())
    }
    if !plain(source)?.is_file() {
        return Err(Error::new(ErrorKind::InvalidInput, "Invalid export source"));
    }
    let folder = destination
        .parent()
        .ok_or_else(|| Error::new(ErrorKind::InvalidInput, "Invalid export destination"))?;
    let relative = folder
        .strip_prefix(root)
        .map_err(|_| Error::new(ErrorKind::InvalidInput, "Export outside selected folder"))?;
    std::fs::create_dir_all(root)?;
    // The reader may choose a linked folder. Resolve that choice once, while
    // continuing to reject links anywhere below the selected root.
    let root = std::fs::canonicalize(root)?;
    if !plain(&root)?.is_dir() {
        return Err(Error::new(ErrorKind::InvalidInput, "Invalid export folder"));
    }
    let mut current = root.to_path_buf();
    for component in relative.components() {
        let std::path::Component::Normal(component) = component else {
            return Err(Error::new(ErrorKind::InvalidInput, "Invalid export folder"));
        };
        current.push(component);
        match std::fs::create_dir(&current) {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
        if !plain(&current)?.is_dir() {
            return Err(Error::new(ErrorKind::InvalidInput, "Invalid export folder"));
        }
    }
    let folder = current;
    let destination = folder.join(
        destination
            .file_name()
            .ok_or_else(|| Error::new(ErrorKind::InvalidInput, "Invalid export name"))?,
    );
    let mut input = std::fs::File::open(source)?;
    let mut staging = tempfile::Builder::new()
        .prefix(".zapfast-export-")
        .tempfile_in(&folder)?;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        check()?;
        let count = input.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        staging.as_file_mut().write_all(&buffer[..count])?;
    }
    staging.as_file().sync_all()?;
    let length = staging.as_file().metadata()?.len();
    let expected = digest(staging.path())?;
    let name = destination
        .file_name()
        .ok_or_else(|| Error::new(ErrorKind::InvalidInput, "Invalid export name"))?;
    let stem = Path::new(name)
        .file_stem()
        .unwrap_or(name)
        .to_string_lossy();
    let extension = destination
        .extension()
        .map(|extension| format!(".{}", extension.to_string_lossy()))
        .unwrap_or_default();
    for number in 1..=1024 {
        check()?;
        let target = if number == 1 {
            destination.to_path_buf()
        } else {
            folder.join(format!("{stem} ({number}){extension}"))
        };
        match plain(&target) {
            Ok(metadata)
                if metadata.is_file()
                    && metadata.len() == length
                    && digest(&target)? == expected =>
            {
                return Ok(target);
            }
            Ok(_) => continue,
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        // Revocation takes the same gate before advancing the account generation.
        // Keep authorization and publication indivisible; copying/hashing never hold it.
        let _publication = publication.map(|gate| {
            gate.lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
        });
        check()?;
        match staging.persist_noclobber(&target) {
            Ok(_) => return Ok(target),
            Err(error) if error.error.kind() == ErrorKind::AlreadyExists => staging = error.file,
            Err(error) => return Err(error.error),
        }
    }
    Err(Error::new(
        ErrorKind::AlreadyExists,
        "Too many existing export names",
    ))
}

/// Sanitizes one readable filename component with a byte bound and safe fallback.
fn component(value: &str, fallback: &str) -> String {
    let mut value_bytes = 0;
    let value: String = value
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, ' ' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        // File names contain both the chat and original document stems.
        // Bound UTF-8 bytes as well as characters to keep the combined name
        // below the common 255-byte component limit, including collision suffixes.
        .take(48)
        .take_while(|c| {
            value_bytes += c.len_utf8();
            value_bytes <= 64
        })
        .collect();
    let value = value.trim();
    if value.is_empty() {
        fallback.into()
    } else {
        // Prefix prevents Windows device names such as CON and AUX.
        format!("_{value}")
    }
}

/// Makes a stable suffix so equal display names cannot merge distinct identities.
fn identity(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    digest[..6]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub(super) struct Metadata<'a> {
    pub account: &'a str,
    pub chat: &'a str,
    pub chat_name: &'a str,
    pub message: &'a str,
    pub timestamp: i64,
    pub mime: &'a str,
    pub original_name: Option<&'a str>,
}

#[cfg(test)]
fn path(
    root: &Path,
    organization: DownloadOrganization,
    descriptive: bool,
    metadata: Metadata<'_>,
) -> PathBuf {
    path_with_original_names(root, organization, descriptive, false, metadata)
}

pub(super) fn path_with_original_names(
    root: &Path,
    organization: DownloadOrganization,
    descriptive: bool,
    original_names: bool,
    metadata: Metadata<'_>,
) -> PathBuf {
    let chat = format!(
        "{}-{}",
        component(metadata.chat_name, "Chat"),
        identity(metadata.chat)
    );
    let date = jiff::Timestamp::from_second(metadata.timestamp)
        .ok()
        .map(|timestamp| timestamp.to_zoned(jiff::tz::TimeZone::UTC));
    let mut folder = root.to_path_buf();
    if organization != DownloadOrganization::Flat {
        folder = folder
            .join(format!(
                "Account-{}",
                component(metadata.account, "Unknown")
            ))
            .join(&chat);
        if organization == DownloadOrganization::ByChatMonth {
            folder = folder.join(date.as_ref().map_or_else(
                || "Unknown-date".into(),
                |date| date.strftime("%Y-%m").to_string(),
            ));
        }
    }
    if original_names && let Some(name) = metadata.original_name.and_then(original_filename) {
        return folder.join(name);
    }
    if !descriptive {
        return super::media_path(
            &folder,
            metadata.chat,
            metadata.message,
            metadata.mime,
            metadata.original_name,
        );
    }
    let date = date.as_ref().map_or_else(
        || "Unknown-date".into(),
        |date| date.strftime("%Y-%m-%d_%H-%M-%S_UTC").to_string(),
    );
    let kind = match metadata.mime.split('/').next() {
        Some("image") => "Image",
        Some("video") => "Video",
        Some("audio") => "Audio",
        _ => "Document",
    };
    let original = metadata
        .original_name
        .and_then(|name| Path::new(name).file_stem()?.to_str())
        .map(|name| component(name, "File"))
        .unwrap_or_default();
    let id = identity(&format!("{}:{}", metadata.chat, metadata.message));
    let extension = super::extension_for(metadata.mime, metadata.original_name);
    folder.join(format!("{date}-{chat}-{kind}{original}-{id}.{extension}"))
}

/// Accepts a bounded document basename while rejecting paths and unsafe names.
fn original_filename(value: &str) -> Option<String> {
    let basename = value.rsplit(['/', '\\']).next()?.trim_matches([' ', '.']);
    let mut bytes = 0;
    let name: String = basename
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') {
                '_'
            } else {
                c
            }
        })
        .take_while(|c| {
            bytes += c.len_utf8();
            bytes <= 180
        })
        .collect();
    let extension = Path::new(basename)
        .extension()
        .and_then(|extension| extension.to_str())
        .filter(|extension| {
            !extension.is_empty()
                && extension.len() <= 16
                && extension.chars().all(char::is_alphanumeric)
        });
    let name = if let Some(extension) = extension
        && !name.ends_with(&format!(".{extension}"))
    {
        format!("{name}.{extension}")
    } else {
        name
    };
    let name = name.trim_end_matches([' ', '.']);
    if name.is_empty() {
        return None;
    }
    let stem = name.split('.').next()?.to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ["COM", "LPT"].iter().any(|prefix| {
            stem.strip_prefix(*prefix).is_some_and(|suffix| {
                matches!(
                    suffix,
                    "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                )
            })
        });
    Some(if reserved {
        format!("_{name}")
    } else {
        name.to_owned()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Both managed subfolders and direct picker destinations accept a linked root.
    #[cfg(unix)]
    #[test]
    fn selected_linked_roots_preserve_confinement_below_the_resolved_directory() {
        use std::os::unix::fs::symlink;
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("cache.pdf");
        std::fs::write(&source, b"synthetic attachment").unwrap();
        let actual = directory.path().join("actual");
        std::fs::create_dir(&actual).unwrap();
        let selected = directory.path().join("selected");
        symlink(&actual, &selected).unwrap();
        for name in ["Account-1/chat/invoice.pdf", "picker.pdf"] {
            let target = selected.join(name);
            let copied = if name.starts_with("Account-") {
                copy_verified_guarded(
                    &source,
                    &selected,
                    &target,
                    || true,
                    &std::sync::Mutex::new(()),
                )
            } else {
                copy_verified_when(&source, &selected, &target, || true)
            }
            .unwrap();
            assert_eq!(copied, std::fs::canonicalize(&actual).unwrap().join(name));
            assert_eq!(std::fs::read(copied).unwrap(), b"synthetic attachment");
        }
        let outside = directory.path().join("outside");
        std::fs::create_dir(&outside).unwrap();
        symlink(&outside, actual.join("escape")).unwrap();
        assert!(copy_verified(&source, &selected, &selected.join("escape/hidden.pdf")).is_err());
        assert_eq!(std::fs::read_dir(outside).unwrap().count(), 0);
    }

    #[test]
    fn revoked_copy_never_publishes_a_partial_attachment() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("cache.pdf");
        std::fs::write(&source, vec![7; 256 * 1024]).unwrap();
        let root = directory.path().join("exports");
        let target = root.join("invoice.pdf");
        let checks = std::cell::Cell::new(0);
        assert!(
            copy_verified_when(&source, &root, &target, || {
                checks.set(checks.get() + 1);
                checks.get() < 3
            })
            .is_err()
        );
        assert!(!target.exists());
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
        assert!(source.is_file());
    }

    #[test]
    fn original_filename_policy_keeps_documents_readable_and_confined_to_the_chosen_folder() {
        let root = Path::new("exports");
        let path = path_with_original_names(
            root,
            DownloadOrganization::ByChatMonth,
            true,
            true,
            Metadata {
                account: "fixture-account",
                chat: "fixture-chat",
                chat_name: "Synthetic",
                message: "fixture-message",
                timestamp: 1_700_000_000,
                mime: "application/pdf",
                original_name: Some("Quarterly invoice.pdf"),
            },
        );
        assert_eq!(path.file_name().unwrap(), "Quarterly invoice.pdf");
        assert!(path.starts_with(root));
        assert_eq!(
            original_filename("../../invoice.pdf").unwrap(),
            "invoice.pdf"
        );
        assert_eq!(
            original_filename("C:\\private\\AUX.pdf").unwrap(),
            "_AUX.pdf"
        );
        assert_eq!(
            original_filename("invoice:final?.pdf").unwrap(),
            "invoice_final_.pdf"
        );
        assert!(original_filename("..").is_none());
        let long = original_filename(&format!("{}.pdf", "文件".repeat(90))).unwrap();
        assert!(long.len() <= 200);
        assert!(long.ends_with(".pdf"));
    }

    #[test]
    fn organized_copies_are_atomic_non_overwriting_and_idempotent_after_restart() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("verified-cache-file");
        std::fs::write(&source, b"synthetic verified bytes").unwrap();
        let root = directory.path().join("user-exports");
        let target = root.join("Account-1").join("Readable.pdf");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, b"user's existing file").unwrap();
        let published = copy_verified(&source, &root, &target).unwrap();
        assert_ne!(published, target);
        assert_eq!(std::fs::read(&target).unwrap(), b"user's existing file");
        assert_eq!(copy_verified(&source, &root, &target).unwrap(), published);
        assert_eq!(
            std::fs::read(&published).unwrap(),
            b"synthetic verified bytes"
        );
        assert_eq!(
            std::fs::read_dir(target.parent().unwrap()).unwrap().count(),
            2
        );
        assert!(copy_verified(&source, &root, &root.join("../escape.pdf")).is_err());
    }

    #[test]
    fn readable_paths_remain_inside_root_and_keep_accounts_and_chats_distinct() {
        let make = |account, chat, name| {
            path(
                Path::new("exports"),
                DownloadOrganization::ByChatMonth,
                true,
                Metadata {
                    account,
                    chat,
                    chat_name: name,
                    message: "same-message",
                    timestamp: 0,
                    mime: "audio/ogg",
                    original_name: None,
                },
            )
        };
        let first = make("1", "chat-a", "../../CON:evil\\name");
        assert!(first.starts_with("exports"));
        assert_eq!(first.components().count(), 5);
        assert!(first.to_string_lossy().contains("1970-01"));
        assert!(first.to_string_lossy().contains("UTC"));
        assert!(first.to_string_lossy().contains("Audio"));
        assert_ne!(first, make("2", "chat-a", "../../CON:evil\\name"));
        assert_ne!(first, make("1", "chat-b", "../../CON:evil\\name"));
    }

    #[test]
    fn hostile_original_names_and_unicode_chat_names_are_bounded() {
        let name = "家庭".repeat(200);
        let target = path(
            Path::new("exports"),
            DownloadOrganization::ByChat,
            true,
            Metadata {
                account: "1",
                chat: "chat",
                chat_name: &name,
                message: "id",
                timestamp: i64::MAX,
                mime: "application/pdf",
                original_name: Some("../../AUX:invoice.pdf"),
            },
        );
        assert_eq!(target.components().count(), 4);
        assert!(target.to_string_lossy().contains("家庭"));
        assert!(target.to_string_lossy().contains("Unknown-date"));
        assert_eq!(target.extension().unwrap(), "pdf");
        assert!(
            target
                .file_name()
                .unwrap()
                .to_string_lossy()
                .chars()
                .count()
                < 150
        );
    }

    #[test]
    fn multibyte_chat_and_original_names_fit_filesystem_component_limit() {
        let long_name = "家庭".repeat(200);
        let original_name = format!("{long_name}.pdf");
        let target = path(
            Path::new("exports"),
            DownloadOrganization::ByChatMonth,
            true,
            Metadata {
                account: "1",
                chat: "chat",
                chat_name: &long_name,
                message: "id",
                timestamp: 0,
                mime: "application/pdf",
                original_name: Some(&original_name),
            },
        );
        assert!(target.file_name().unwrap().to_string_lossy().len() < 220);
        assert!(
            target
                .components()
                .all(|part| part.as_os_str().to_string_lossy().len() < 255)
        );
    }

    #[test]
    fn flat_original_policy_preserves_existing_filename_behavior() {
        let root = Path::new("exports");
        let target = path(
            root,
            DownloadOrganization::Flat,
            false,
            Metadata {
                account: "1",
                chat: "chat",
                chat_name: "Chat",
                message: "id",
                timestamp: 0,
                mime: "application/pdf",
                original_name: Some("invoice.pdf"),
            },
        );
        assert_eq!(
            target,
            super::super::media_path(root, "chat", "id", "application/pdf", Some("invoice.pdf"))
        );
    }
}
