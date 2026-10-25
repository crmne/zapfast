//! One fixed layout for downloads saved outside the account's normal cache.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::model::AccountId;

/// Creates one layout level at a time, rejecting links before touching their children.
pub(super) fn create_folder(root: &Path, folder: &Path) -> std::io::Result<()> {
    let account = folder
        .parent()
        .filter(|account| account.parent() == Some(root))
        .ok_or_else(|| std::io::Error::other("invalid download folder layout"))?;
    std::fs::create_dir_all(root)?;
    for directory in [account, folder] {
        match std::fs::create_dir(directory) {
            Ok(()) => {}
            Err(error)
                if error.kind() == std::io::ErrorKind::AlreadyExists
                    && real_directory(directory) => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

/// Sweeps only this identity's real chat directories, never another account or symlink.
pub(super) fn discard_staging(root: &Path, slot: &AccountId, account: &str) {
    let account = account_folder(root, slot, account);
    if !real_directory(&account) {
        return;
    }
    let Ok(chats) = std::fs::read_dir(account) else {
        return;
    };
    for chat in chats.flatten() {
        if real_directory(&chat.path())
            && chat
                .file_name()
                .to_string_lossy()
                .rsplit_once('-')
                .is_some_and(|(_, key)| is_identity(key))
        {
            super::discard_attachment_staging(&chat.path());
        }
    }
}

/// Directory metadata is read without following symbolic links or junctions.
fn real_directory(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_dir())
}

/// Only generated identity suffixes belong to the fixed layout.
fn is_identity(value: &str) -> bool {
    value.len() == 16 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// Keeps account slots and WhatsApp identities separate, even after relinking.
pub(super) fn folder(
    root: &Path,
    slot: &AccountId,
    account: &str,
    chat: &str,
    name: &str,
) -> PathBuf {
    account_folder(root, slot, account).join(format!("{}-{}", component(name), identity(chat)))
}

/// The full identity, as well as the local slot, keeps cleanup and downloads account-bound.
fn account_folder(root: &Path, slot: &AccountId, account: &str) -> PathBuf {
    root.join(format!(
        "{}-{}",
        component(&format!("Account {slot}")),
        identity(account)
    ))
}

/// A short stable suffix distinguishes equal, truncated and case-folded names.
fn identity(value: &str) -> String {
    Sha256::digest(value.as_bytes())[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Bounds UTF-8 without splitting characters; the appended key prevents device names.
fn component(value: &str) -> String {
    let mut name = String::new();
    for character in value.trim().chars() {
        // Dots are replaced too: CON.anything is a Windows device name even
        // with a suffix, whereas CON-anything is an ordinary directory.
        let character = if character.is_control()
            || matches!(
                character,
                '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' | '.'
            ) {
            '_'
        } else {
            character
        };
        if name.len() + character.len_utf8() > 48 {
            break;
        }
        name.push(character);
    }
    let name = name.trim();
    if name.is_empty() {
        "Chat".into()
    } else {
        name.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Crash leftovers are removed only from this account's generated chat directories.
    #[test]
    fn cleanup_preserves_other_accounts_and_finished_files() {
        let root = tempfile::tempdir().unwrap();
        let first = AccountId::first();
        let ours = folder(root.path(), &first, "account-a", "chat", "Alice");
        let other = folder(
            root.path(),
            &AccountId::parse("2").unwrap(),
            "account-b",
            "chat",
            "Alice",
        );
        let replacement = folder(root.path(), &first, "replacement-account", "chat", "Alice");
        for dir in [&ours, &other, &replacement] {
            std::fs::create_dir_all(dir).unwrap();
            std::fs::write(dir.join(".photo.0123456789abcdef.part"), b"partial").unwrap();
            std::fs::write(dir.join("photo.jpg"), b"finished").unwrap();
        }
        let unrelated = root.path().join("unrelated");
        std::fs::create_dir(&unrelated).unwrap();
        std::fs::write(unrelated.join(".unrelated.part"), b"keep").unwrap();
        discard_staging(root.path(), &first, "account-a");
        assert!(!ours.join(".photo.0123456789abcdef.part").exists());
        assert_eq!(std::fs::read(ours.join("photo.jpg")).unwrap(), b"finished");
        assert!(other.join(".photo.0123456789abcdef.part").exists());
        assert!(replacement.join(".photo.0123456789abcdef.part").exists());
        assert!(unrelated.join(".unrelated.part").exists());
    }

    /// Account and chat symlinks cannot turn cleanup into a walk outside the chosen layout.
    #[cfg(unix)]
    #[test]
    fn cleanup_skips_symbolic_directories() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let first = AccountId::first();
        let target = outside.path().join(".keep.part");
        std::fs::write(&target, b"keep").unwrap();
        let account = folder(root.path(), &first, "account-a", "chat", "Alice");
        std::os::unix::fs::symlink(outside.path(), account.parent().unwrap()).unwrap();
        discard_staging(root.path(), &first, "account-a");
        assert!(target.exists());
        std::fs::remove_file(account.parent().unwrap()).unwrap();
        std::fs::create_dir(account.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(outside.path(), &account).unwrap();
        discard_staging(root.path(), &first, "account-a");
        assert!(target.exists());
    }

    /// Account slots, replacement phone identities and equal chat names never share a folder.
    #[test]
    fn identities_keep_shared_download_roots_separate() {
        let root = Path::new("downloads");
        let first = AccountId::first();
        let path = folder(root, &first, "account-a", "chat-a", "Alice");
        assert_ne!(
            path,
            folder(
                root,
                &AccountId::parse("2").unwrap(),
                "account-a",
                "chat-a",
                "Alice"
            )
        );
        assert_ne!(path, folder(root, &first, "account-b", "chat-a", "Alice"));
        assert_ne!(path, folder(root, &first, "account-a", "chat-b", "Alice"));
        assert!(
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("Alice-")
        );
        assert!(
            path.parent()
                .unwrap()
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("Account 1-")
        );
    }

    /// Remote display names cannot add path levels or exceed a filesystem component limit.
    #[test]
    fn hostile_and_unicode_names_remain_readable_direct_children() {
        let root = tempfile::tempdir().unwrap();
        for name in [
            "Alice",
            " Família 🎉 ",
            "../outside",
            "C:\\outside/..",
            "CON.anything",
            "NUL",
            " . ",
            "",
            "\0\n\t",
            &"日本語🎉".repeat(100),
        ] {
            let path = folder(root.path(), &AccountId::first(), "account", "chat", name);
            let relative = path.strip_prefix(root.path()).unwrap();
            assert_eq!(relative.components().count(), 2);
            assert!(
                relative
                    .components()
                    .all(|part| matches!(part, std::path::Component::Normal(_)))
            );
            let label = path.file_name().unwrap().to_str().unwrap();
            assert!(label.len() <= 65, "{label}");
            assert!(!label.contains(['/', '\\', ':', '.', '<', '>', '"', '|', '?', '*']));
            std::fs::create_dir_all(&path).unwrap();
            assert!(path.is_dir());
        }
        assert!(
            folder(
                root.path(),
                &AccountId::first(),
                "account",
                "chat",
                "Família 🎉"
            )
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("Família 🎉-")
        );
    }
}
