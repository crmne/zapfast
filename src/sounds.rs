//! ZapFast's own copies of the audio files chosen as notification sounds.
//!
//! A chosen file is copied under a folder named by a hash of its contents,
//! keeping its own name (the menus show it), so moving or deleting the
//! original never silences a notification. The message and mention sounds
//! are process-wide and keep their copies in the app's state folder; chats'
//! own sounds keep theirs in their account's. Nothing is deleted while
//! ZapFast runs: at the next start, before anything imports, the copies no
//! saved setting names are deleted.

use std::ffi::OsStr;
use std::io::Read;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::archive::Archive;
use crate::settings::{NotificationSound, Settings};

/// The largest audio file kept as a notification sound.
pub const MAX_BYTES: u64 = 10 * 1024 * 1024;

/// Where the message and mention sounds' copies live, under the app's state
/// folder.
pub fn settings_root(state: &Path) -> PathBuf {
    state.join("sounds")
}

/// Where chats' own sounds' copies live, under their account's state folder.
pub fn chat_root(account_state: &Path) -> PathBuf {
    account_state.join("chat-sounds")
}

/// Copies `source` under `root` and returns the copy. The file is read once,
/// and only audio ZapFast can play, up to [`MAX_BYTES`], is copied. Nothing
/// under `root` is replaced or deleted, so imports never undo each other.
pub fn import(source: &Path, root: &Path) -> Result<PathBuf, String> {
    let name = source.file_name().ok_or_else(|| "not a file".to_owned())?;
    let mut bytes = Vec::new();
    std::fs::File::open(source)
        .and_then(|file| file.take(MAX_BYTES + 1).read_to_end(&mut bytes))
        .map_err(|error| error.to_string())?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_BYTES {
        return Err(format!("larger than {} MB", MAX_BYTES / (1024 * 1024)));
    }
    if !playable(bytes.clone()) {
        return Err("not an audio file ZapFast can play".to_owned());
    }
    let target = root.join(content_folder(&bytes)).join(name);
    // The same contents under the same name are already there: a copy
    // picked again, or one file for two sounds.
    if !target.is_file() {
        fastframe_file::write(&target, &bytes).map_err(|error| error.to_string())?;
    }
    Ok(target)
}

/// Whether rodio, which plays notification sounds, decodes a sample from
/// `bytes`: a stream it accepts may still hold no audio.
fn playable(bytes: Vec<u8>) -> bool {
    rodio::Decoder::new(std::io::Cursor::new(bytes))
        .is_ok_and(|mut decoder| decoder.next().is_some())
}

/// The folder a copy of `bytes` goes in.
fn content_folder(bytes: &[u8]) -> String {
    Sha256::digest(bytes)[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Whether `name` is a folder [`import`] makes.
fn is_copy_folder(name: &OsStr) -> bool {
    name.to_str()
        .is_some_and(|name| name.len() == 16 && name.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

/// A custom sound's copy under `root`: the path itself when it is there, or
/// the same copy under `root` when the state folder moved since it was saved.
/// `None` for a file of the person's own, which [`import`] copies. Only
/// `root` is looked at on disk: the person's own path may be slow to reach.
fn managed(path: &Path, root: &Path) -> Option<PathBuf> {
    if path.starts_with(root) {
        return Some(path.to_path_buf());
    }
    let folder = path.parent()?.file_name()?;
    if !is_copy_folder(folder) {
        return None;
    }
    let moved = root.join(folder).join(path.file_name()?);
    moved.is_file().then_some(moved)
}

/// Deletes every copy under `root` that `keep` does not name, and the folders
/// it leaves empty.
fn collect(root: &Path, keep: &[PathBuf]) {
    let Ok(folders) = std::fs::read_dir(root) else {
        return;
    };
    for folder in folders
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
    {
        if let Ok(files) = std::fs::read_dir(&folder) {
            for file in files.flatten().map(|entry| entry.path()) {
                if !keep.contains(&file)
                    && let Err(error) = std::fs::remove_file(&file)
                {
                    log::debug!("could not delete an unused notification sound: {error}");
                }
            }
        }
        // Only a folder left empty goes.
        let _ = std::fs::remove_dir(&folder);
    }
}

/// At startup, before anything imports: points the message and mention
/// sounds at their copies if the state folder moved, and deletes the copies
/// neither names. Returns whether `settings` changed.
pub fn tidy(settings: &mut Settings, state: &Path) -> bool {
    let root = settings_root(state);
    let mut changed = false;
    let mut keep = Vec::new();
    for sound in [&mut settings.message_sound, &mut settings.mention_sound] {
        if let NotificationSound::Custom(path) = sound
            && let Some(copy) = managed(path, &root)
        {
            if copy != *path {
                *path = copy.clone();
                changed = true;
            }
            keep.push(copy);
        }
    }
    collect(&root, &keep);
    changed
}

/// The message (`false`) and mention (`true`) sounds that name a file of
/// the person's own, as releases before these copies saved them, for
/// [`import`] off the startup path.
pub fn external(settings: &Settings, state: &Path) -> Vec<(bool, PathBuf)> {
    let root = settings_root(state);
    [
        (false, &settings.message_sound),
        (true, &settings.mention_sound),
    ]
    .into_iter()
    .filter_map(|(mention, sound)| match sound {
        NotificationSound::Custom(path) if !path.starts_with(&root) => {
            Some((mention, path.clone()))
        }
        _ => None,
    })
    .collect()
}

/// Stores a message or mention sound's copy unless the sound stopped naming
/// `from` meanwhile. Returns whether it did.
pub fn adopt_setting(settings: &mut Settings, mention: bool, from: &Path, to: PathBuf) -> bool {
    let sound = if mention {
        &mut settings.mention_sound
    } else {
        &mut settings.message_sound
    };
    let current = matches!(sound, NotificationSound::Custom(path) if path == from);
    if current {
        *sound = NotificationSound::Custom(to);
    }
    current
}

/// [`tidy`] for the chats' own sounds in one account's archive.
pub fn tidy_chat_sounds(archive: &Archive, account_state: &Path) {
    let root = chat_root(account_state);
    let sounds = match archive.chat_sounds() {
        Ok(sounds) => sounds,
        Err(error) => {
            log::warn!("could not read the chats' notification sounds: {error}");
            return;
        }
    };
    let mut keep = Vec::new();
    for (chat, sound) in sounds {
        let NotificationSound::Custom(path) = sound else {
            continue;
        };
        let Some(copy) = managed(&path, &root) else {
            continue;
        };
        if copy != path
            && let Err(error) = archive
                .set_notification_sound(&chat, Some(&NotificationSound::Custom(copy.clone())))
        {
            log::warn!("could not store a chat's notification sound: {error}");
        }
        keep.push(copy);
    }
    collect(&root, &keep);
}

/// The chats whose own sounds name a file of the person's own.
pub fn external_chat_sounds(archive: &Archive, account_state: &Path) -> Vec<(String, PathBuf)> {
    let root = chat_root(account_state);
    match archive.chat_sounds() {
        Ok(sounds) => sounds
            .into_iter()
            .filter_map(|(chat, sound)| match sound {
                NotificationSound::Custom(path) if !path.starts_with(&root) => Some((chat, path)),
                _ => None,
            })
            .collect(),
        Err(error) => {
            log::warn!("could not read the chats' notification sounds: {error}");
            Vec::new()
        }
    }
}

/// Stores each chat's copy unless its sound stopped naming the file copied
/// meanwhile. Returns the chats whose sound changed.
pub fn apply_adopted(archive: &Archive, copied: Vec<(String, PathBuf, PathBuf)>) -> Vec<String> {
    let mut changed = Vec::new();
    for (chat, from, to) in copied {
        let current = archive
            .chat(&chat)
            .ok()
            .flatten()
            .and_then(|chat| chat.notification_sound);
        if current != Some(NotificationSound::Custom(from)) {
            continue;
        }
        match archive.set_notification_sound(&chat, Some(&NotificationSound::Custom(to))) {
            Ok(()) => changed.push(chat),
            Err(error) => log::warn!("could not store a chat's notification sound: {error}"),
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::NotificationSound::Custom;

    /// `samples` of silence as 16-bit mono WAV at 8 kHz.
    fn wav(samples: u32) -> Vec<u8> {
        let data = samples * 2;
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&(36 + data).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&8000u32.to_le_bytes());
        bytes.extend_from_slice(&16000u32.to_le_bytes());
        bytes.extend_from_slice(&2u16.to_le_bytes());
        bytes.extend_from_slice(&16u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&data.to_le_bytes());
        bytes.resize(bytes.len() + data as usize, 0);
        bytes
    }

    #[test]
    fn a_chosen_sound_is_copied_under_its_own_name_and_kept_while_running() {
        let root = tempfile::tempdir().unwrap();
        let sounds = settings_root(&root.path().join("state"));
        let first = root.path().join("ding.wav");
        std::fs::write(&first, wav(800)).unwrap();
        let copy = import(&first, &sounds).unwrap();
        assert_eq!(copy.file_name().unwrap(), "ding.wav");
        assert!(copy.starts_with(&sounds));
        assert_eq!(std::fs::read(&copy).unwrap(), wav(800));
        std::fs::remove_file(&first).unwrap();
        assert!(copy.exists(), "the copy outlives the original");
        assert_eq!(
            import(&copy, &sounds).unwrap(),
            copy,
            "a copy picked again stays where it is"
        );

        let second = root.path().join("bell.wav");
        std::fs::write(&second, wav(400)).unwrap();
        let other = import(&second, &sounds).unwrap();
        assert_ne!(other.parent(), copy.parent());
        assert!(copy.exists(), "nothing is deleted while ZapFast runs");
    }

    #[test]
    fn only_playable_audio_of_a_bounded_size_is_kept() {
        let root = tempfile::tempdir().unwrap();
        let sounds = settings_root(&root.path().join("state"));
        let text = root.path().join("notes.wav");
        std::fs::write(&text, "not a sound").unwrap();
        assert!(import(&text, &sounds).is_err());
        let empty = root.path().join("empty.wav");
        std::fs::write(&empty, wav(0)).unwrap();
        assert!(import(&empty, &sounds).is_err(), "a stream without audio");
        let huge = root.path().join("huge.wav");
        std::fs::File::create(&huge)
            .unwrap()
            .set_len(MAX_BYTES + 1)
            .unwrap();
        assert!(import(&huge, &sounds).is_err());
        assert!(!sounds.exists(), "nothing was copied");
    }

    #[test]
    fn startup_keeps_named_copies_follows_a_moved_folder_and_drops_the_rest() {
        let root = tempfile::tempdir().unwrap();
        let state = root.path().join("state");
        let sounds = settings_root(&state);
        let source = root.path().join("ding.wav");
        std::fs::write(&source, wav(800)).unwrap();
        let named = import(&source, &sounds).unwrap();
        std::fs::write(&source, wav(400)).unwrap();
        let moved = import(&source, &sounds).unwrap();
        std::fs::write(&source, wav(200)).unwrap();
        let unused = import(&source, &sounds).unwrap();
        let old_home = root
            .path()
            .join("old")
            .join("sounds")
            .join(moved.parent().unwrap().file_name().unwrap())
            .join("ding.wav");
        let mut settings = Settings {
            message_sound: Custom(named.clone()),
            mention_sound: Custom(old_home),
            ..Settings::default()
        };
        assert!(tidy(&mut settings, &state));
        assert_eq!(settings.message_sound, Custom(named.clone()));
        assert_eq!(settings.mention_sound, Custom(moved.clone()));
        assert!(named.exists() && moved.exists());
        assert!(!unused.exists() && !unused.parent().unwrap().exists());
        assert!(!tidy(&mut settings, &state), "nothing more to change");
        assert!(external(&settings, &state).is_empty());

        let own = root.path().join("mine.wav");
        settings.message_sound = Custom(own.clone());
        assert_eq!(external(&settings, &state), vec![(false, own.clone())]);
        assert!(
            !adopt_setting(&mut settings, true, &own, named.clone()),
            "the mention sound never named it"
        );
        assert!(adopt_setting(&mut settings, false, &own, named.clone()));
        assert_eq!(settings.message_sound, Custom(named));
    }

    #[test]
    fn chats_keep_their_named_copies_and_adopt_their_own_files() {
        let root = tempfile::tempdir().unwrap();
        let account = root.path().join("account");
        let sounds = chat_root(&account);
        let source = root.path().join("ada.wav");
        std::fs::write(&source, wav(800)).unwrap();
        let named = import(&source, &sounds).unwrap();
        std::fs::write(&source, wav(400)).unwrap();
        let unused = import(&source, &sounds).unwrap();
        let archive = Archive::in_memory().unwrap();
        let (ada, bea) = ("1@s.whatsapp.net", "2@s.whatsapp.net");
        archive.ensure_chat(ada, "Ada").unwrap();
        archive.ensure_chat(bea, "Bea").unwrap();
        archive
            .set_notification_sound(ada, Some(&Custom(named.clone())))
            .unwrap();
        archive
            .set_notification_sound(bea, Some(&Custom(source.clone())))
            .unwrap();
        tidy_chat_sounds(&archive, &account);
        assert!(named.exists());
        assert!(!unused.exists(), "a copy no chat names goes");
        assert_eq!(
            external_chat_sounds(&archive, &account),
            vec![(bea.to_owned(), source.clone())]
        );

        let copy = import(&source, &sounds).unwrap();
        let copied = vec![
            (bea.to_owned(), source.clone(), copy.clone()),
            (ada.to_owned(), source.clone(), copy.clone()),
        ];
        assert_eq!(apply_adopted(&archive, copied), vec![bea.to_owned()]);
        assert_eq!(
            archive.chat(bea).unwrap().unwrap().notification_sound,
            Some(Custom(copy))
        );
        assert_eq!(
            archive.chat(ada).unwrap().unwrap().notification_sound,
            Some(Custom(named)),
            "a chat whose sound changed meanwhile keeps it"
        );
    }
}
