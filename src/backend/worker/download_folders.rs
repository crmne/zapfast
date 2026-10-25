//! One fixed layout for downloads saved outside the account's normal cache:
//! `<account>/<chat>/file` inside the chosen folder.
//!
//! Folder names are what a person reads. An account's folder is its own phone
//! number, which neither changes nor repeats; a chat's folder is the chat's
//! name. Two chats of one account whose names make the same folder (the same
//! words, or the same once unsafe characters are replaced or letters folded to
//! one case) get `Name`, `Name (2)` and so on. Each chat keeps the folder it was
//! given in the archive, so a later chat with the same name never takes it over,
//! and a renamed chat gets a folder for its new name.

use std::path::{Path, PathBuf};

use crate::archive::Archive;
use crate::model::AccountId;

/// Archive key prefix: the folder a chat was given, and the name it came from.
const CHAT_FOLDER: &str = "download_folder:";
/// Archive key prefix: which chat a folder name (case folded) belongs to.
const FOLDER_OWNER: &str = "download_folder_owner:";
/// Separates the name a folder came from and the folder in a stored value.
const SEPARATOR: char = '\u{1f}';

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

/// Sweeps only this account's real chat directories, never another account or symlink.
pub(super) fn discard_staging(root: &Path, slot: &AccountId, phone: Option<&str>) {
    let account = account_folder(root, slot, phone);
    if !real_directory(&account) {
        return;
    }
    let Ok(chats) = std::fs::read_dir(account) else {
        return;
    };
    for chat in chats.flatten() {
        if real_directory(&chat.path()) {
            super::discard_attachment_staging(&chat.path());
        }
    }
}

/// Directory metadata is read without following symbolic links or junctions.
fn real_directory(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_dir())
}

/// The account's folder: its phone number, which stays the same whichever
/// form (phone number or privacy id) WhatsApp names the account by, and which
/// a different account linked in its place never shares. Only while the number
/// is not known yet does the folder take the account's place in ZapFast.
pub(super) fn account_folder(root: &Path, slot: &AccountId, phone: Option<&str>) -> PathBuf {
    let digits = phone
        .and_then(|phone| phone.split(['@', ':']).next())
        .filter(|user| !user.is_empty() && user.bytes().all(|byte| byte.is_ascii_digit()));
    root.join(match digits {
        Some(digits) => format!("+{digits}"),
        None => component(&format!("Account {slot}")),
    })
}

/// The chat's folder inside `account`: its readable name, with a number added
/// only when another chat of this account already has a folder by that name.
/// The choice is kept in the archive so it never moves to another chat.
pub(super) fn chat_folder(archive: &Archive, account: &Path, chat: &str, name: &str) -> PathBuf {
    let source = component(name);
    let stored = archive
        .meta(&format!("{CHAT_FOLDER}{chat}"))
        .ok()
        .flatten()
        .and_then(|value| {
            let (from, folder) = value.split_once(SEPARATOR)?;
            (from == source).then(|| folder.to_owned())
        });
    if let Some(folder) = stored {
        return account.join(folder);
    }
    let owner = |folder: &str| {
        archive
            .meta(&format!("{FOLDER_OWNER}{}", folder.to_lowercase()))
            .ok()
            .flatten()
    };
    let folder = (1..)
        .map(|n| {
            if n == 1 {
                source.clone()
            } else {
                format!("{source} ({n})")
            }
        })
        .find(|folder| owner(folder).is_none_or(|owner| owner == chat))
        .unwrap_or_else(|| source.clone());
    // An archive that cannot record the choice still gives a usable folder;
    // the next download simply chooses again.
    let _ = archive.set_meta(&format!("{FOLDER_OWNER}{}", folder.to_lowercase()), chat);
    let _ = archive.set_meta(
        &format!("{CHAT_FOLDER}{chat}"),
        &format!("{source}{SEPARATOR}{folder}"),
    );
    account.join(folder)
}

/// Names Windows keeps for devices, whatever follows them after a dot.
const RESERVED: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// A readable folder name: bounded UTF-8 without splitting characters, with
/// characters a file system refuses replaced, and never a device name.
fn component(value: &str) -> String {
    let mut name = String::new();
    for character in value.trim().chars() {
        // Dots are replaced too: CON.anything is a Windows device name, and a
        // trailing dot is dropped by Windows.
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
    } else if RESERVED
        .iter()
        .any(|reserved| reserved.eq_ignore_ascii_case(name))
    {
        format!("{name}_")
    } else {
        name.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(archive: &Archive, root: &Path, chat: &str, name: &str) -> PathBuf {
        let account = account_folder(
            root,
            &AccountId::first(),
            Some("15550001111@s.whatsapp.net"),
        );
        chat_folder(archive, &account, chat, name)
    }

    fn label(path: &Path) -> String {
        path.file_name().unwrap().to_string_lossy().into_owned()
    }

    /// Crash leftovers are removed only from this account's chat directories.
    #[test]
    fn cleanup_preserves_other_accounts_and_finished_files() {
        let root = tempfile::tempdir().unwrap();
        let first = AccountId::first();
        let ours =
            account_folder(root.path(), &first, Some("15550001111@s.whatsapp.net")).join("Alice");
        let other = account_folder(
            root.path(),
            &AccountId::parse("2").unwrap(),
            Some("15550002222@s.whatsapp.net"),
        )
        .join("Alice");
        let replacement =
            account_folder(root.path(), &first, Some("15550003333@s.whatsapp.net")).join("Alice");
        for dir in [&ours, &other, &replacement] {
            std::fs::create_dir_all(dir).unwrap();
            std::fs::write(dir.join(".photo.0123456789abcdef.part"), b"partial").unwrap();
            std::fs::write(dir.join("photo.jpg"), b"finished").unwrap();
        }
        let unrelated = root.path().join("unrelated");
        std::fs::create_dir(&unrelated).unwrap();
        std::fs::write(unrelated.join(".unrelated.part"), b"keep").unwrap();
        discard_staging(root.path(), &first, Some("15550001111@s.whatsapp.net"));
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
        let phone = Some("15550001111@s.whatsapp.net");
        let target = outside.path().join(".keep.part");
        std::fs::write(&target, b"keep").unwrap();
        let account = account_folder(root.path(), &first, phone);
        std::os::unix::fs::symlink(outside.path(), &account).unwrap();
        discard_staging(root.path(), &first, phone);
        assert!(target.exists());
        std::fs::remove_file(&account).unwrap();
        std::fs::create_dir(&account).unwrap();
        std::os::unix::fs::symlink(outside.path(), account.join("Alice")).unwrap();
        discard_staging(root.path(), &first, phone);
        assert!(target.exists());
    }

    /// The account folder is its number, in whichever form the number is
    /// given, and a number not known yet leaves the account's place.
    #[test]
    fn account_folders_are_the_phone_number() {
        let root = Path::new("downloads");
        let first = AccountId::first();
        let path = account_folder(root, &first, Some("15550001111@s.whatsapp.net"));
        assert_eq!(label(&path), "+15550001111");
        assert_eq!(
            account_folder(root, &first, Some("15550001111:7@s.whatsapp.net")),
            path
        );
        // Another account linked in the same place keeps its own folder.
        assert_ne!(
            account_folder(root, &first, Some("15550002222@s.whatsapp.net")),
            path
        );
        assert_eq!(label(&account_folder(root, &first, None)), "Account 1");
        assert_eq!(
            label(&account_folder(root, &first, Some("me"))),
            "Account 1"
        );
    }

    /// A chat's folder is its name; a number is added only when another chat
    /// already has that folder, and the first one keeps it.
    #[test]
    fn chat_folders_are_readable_and_numbered_only_on_collision() {
        let archive = Archive::in_memory().unwrap();
        let root = Path::new("downloads");
        let alice = folder(&archive, root, "1@s.whatsapp.net", "Alice");
        assert_eq!(label(&alice), "Alice");
        assert_eq!(folder(&archive, root, "1@s.whatsapp.net", "Alice"), alice);
        // Equal once case is folded, as on Windows and macOS.
        let other = folder(&archive, root, "2@s.whatsapp.net", "alice");
        assert_eq!(label(&other), "alice (2)");
        let third = folder(&archive, root, "3@s.whatsapp.net", "Alice");
        assert_eq!(label(&third), "Alice (3)");
        // Asking again in any order keeps each chat's folder.
        assert_eq!(folder(&archive, root, "2@s.whatsapp.net", "alice"), other);
        assert_eq!(folder(&archive, root, "1@s.whatsapp.net", "Alice"), alice);
        // A rename gets a folder for the new name; the old one stays its own.
        let renamed = folder(&archive, root, "1@s.whatsapp.net", "Alice Cooper");
        assert_eq!(label(&renamed), "Alice Cooper");
        assert_eq!(
            label(&folder(&archive, root, "4@s.whatsapp.net", "Alice")),
            "Alice (4)"
        );
        // Renamed back, the chat finds its old folder free for it.
        assert_eq!(folder(&archive, root, "1@s.whatsapp.net", "Alice"), alice);
    }

    /// Remote display names cannot add path levels or exceed a filesystem component limit.
    #[test]
    fn hostile_and_unicode_names_remain_readable_direct_children() {
        let root = tempfile::tempdir().unwrap();
        let archive = Archive::in_memory().unwrap();
        for (index, name) in [
            "Alice",
            " Família 🎉 ",
            "../outside",
            "C:\\outside/..",
            "CON.anything",
            "NUL",
            "com1",
            " . ",
            "",
            "\0\n\t",
            &"日本語🎉".repeat(100),
        ]
        .into_iter()
        .enumerate()
        {
            let path = folder(
                &archive,
                root.path(),
                &format!("{index}@s.whatsapp.net"),
                name,
            );
            let relative = path.strip_prefix(root.path()).unwrap();
            assert_eq!(relative.components().count(), 2);
            assert!(
                relative
                    .components()
                    .all(|part| matches!(part, std::path::Component::Normal(_)))
            );
            let label = label(&path);
            assert!(label.len() <= 56, "{label}");
            assert!(!label.contains(['/', '\\', ':', '.', '<', '>', '"', '|', '?', '*']));
            assert!(
                !RESERVED
                    .iter()
                    .any(|reserved| reserved.eq_ignore_ascii_case(&label)),
                "{label}"
            );
            std::fs::create_dir_all(&path).unwrap();
            assert!(path.is_dir());
        }
        assert_eq!(
            label(&folder(
                &archive,
                root.path(),
                "1@s.whatsapp.net",
                "Família 🎉"
            )),
            "Família 🎉"
        );
    }
}
