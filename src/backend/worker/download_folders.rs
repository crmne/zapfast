//! One fixed layout for downloads saved outside the account's normal cache.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::model::AccountId;

/// Keeps account slots and WhatsApp identities separate, even after relinking.
pub(super) fn folder(
    root: &Path,
    slot: &AccountId,
    account: &str,
    chat: &str,
    name: &str,
) -> PathBuf {
    root.join(format!(
        "{}-{}",
        component(&format!("Account {slot}")),
        identity(account)
    ))
    .join(format!("{}-{}", component(name), identity(chat)))
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
