//! ZapFast's own copies of the audio files chosen as notification sounds.
//!
//! A chosen file is copied into a folder of its own, its slot, under its own
//! name (the menus show it), so moving or deleting the original never
//! silences a notification. The message and mention sounds are process-wide
//! and keep their slots in the app's state folder; a chat's sound lives with
//! its account, in a slot named by a hash of the chat id, so no file name
//! carries a phone number.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::settings::{NotificationSound, Settings};

/// The largest audio file kept as a notification sound.
pub const MAX_BYTES: u64 = 10 * 1024 * 1024;

/// The slot of the message or mention sound, under the app's state folder.
pub fn global_slot(state: &Path, mention: bool) -> PathBuf {
    state
        .join("sounds")
        .join(if mention { "mention" } else { "message" })
}

/// The slot of one chat's sound, under its account's state folder.
pub fn chat_slot(account_state: &Path, chat: &str) -> PathBuf {
    let digest = Sha256::digest(chat.as_bytes());
    let name: String = digest[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    account_state.join("sounds").join(format!("chat-{name}"))
}

/// Copies `source` into `slot`, replacing whatever the slot held, and returns
/// the copy. A file already in the slot is returned as it is. Only audio
/// ZapFast can play, up to [`MAX_BYTES`], is copied.
pub fn import(source: &Path, slot: &Path) -> Result<PathBuf, String> {
    if source.parent() == Some(slot) {
        return Ok(source.to_path_buf());
    }
    let name = source.file_name().ok_or_else(|| "not a file".to_owned())?;
    let size = std::fs::metadata(source)
        .map_err(|error| error.to_string())?
        .len();
    if size > MAX_BYTES {
        return Err(format!("larger than {} MB", MAX_BYTES / (1024 * 1024)));
    }
    let file = std::fs::File::open(source).map_err(|error| error.to_string())?;
    if rodio::Decoder::new(std::io::BufReader::new(file)).is_err() {
        return Err("not an audio file ZapFast can play".to_owned());
    }
    let bytes = std::fs::read(source).map_err(|error| error.to_string())?;
    std::fs::create_dir_all(slot).map_err(|error| error.to_string())?;
    let target = slot.join(name);
    fastframe_file::write(&target, bytes).map_err(|error| error.to_string())?;
    clear(slot, &target);
    Ok(target)
}

/// Deletes a slot and its copy, once no setting uses it.
pub fn remove(slot: &Path) {
    match std::fs::remove_dir_all(slot) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => log::debug!("could not delete a notification sound's copy: {error}"),
    }
}

/// Deletes every file in `slot` but `keep`.
fn clear(slot: &Path, keep: &Path) {
    let Ok(entries) = std::fs::read_dir(slot) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path != keep
            && let Err(error) = std::fs::remove_file(&path)
        {
            log::debug!("could not delete an earlier notification sound: {error}");
        }
    }
}

/// Copies the message and mention sounds that still point at the user's own
/// files, as releases before these copies kept them, into their slots. A
/// file that is gone is left as it is. Returns whether anything changed.
pub fn adopt(settings: &mut Settings, state: &Path) -> bool {
    let mut changed = false;
    for mention in [false, true] {
        let sound = if mention {
            &mut settings.mention_sound
        } else {
            &mut settings.message_sound
        };
        if let Some(copy) = adopted(sound, &global_slot(state, mention)) {
            *sound = NotificationSound::Custom(copy);
            changed = true;
        }
    }
    changed
}

/// [`adopt`] for the chats' own sounds in one account's archive.
pub fn adopt_chat_sounds(archive: &crate::archive::Archive, account_state: &Path) {
    let sounds = match archive.chat_sounds() {
        Ok(sounds) => sounds,
        Err(error) => {
            log::warn!("could not read the chats' notification sounds: {error}");
            return;
        }
    };
    for (chat, sound) in sounds {
        if let Some(copy) = adopted(&sound, &chat_slot(account_state, &chat)) {
            let copy = NotificationSound::Custom(copy);
            if let Err(error) = archive.set_notification_sound(&chat, Some(&copy)) {
                log::warn!("could not store a chat's notification sound: {error}");
            }
        }
    }
}

/// The copy of a custom sound outside its slot whose file is still there.
fn adopted(sound: &NotificationSound, slot: &Path) -> Option<PathBuf> {
    let NotificationSound::Custom(path) = sound else {
        return None;
    };
    if path.parent() == Some(slot) || !path.is_file() {
        return None;
    }
    match import(path, slot) {
        Ok(copy) => Some(copy),
        Err(error) => {
            log::warn!("could not copy a notification sound: {error}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tenth of a second of silence as 16-bit mono WAV.
    fn wav() -> Vec<u8> {
        let data: u32 = 800 * 2;
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
    fn a_chosen_sound_is_copied_under_its_own_name_and_replaces_the_last() {
        let root = tempfile::tempdir().unwrap();
        let slot = global_slot(&root.path().join("state"), false);
        let first = root.path().join("ding.wav");
        std::fs::write(&first, wav()).unwrap();
        let copy = import(&first, &slot).unwrap();
        assert_eq!(copy, slot.join("ding.wav"));
        assert_eq!(std::fs::read(&copy).unwrap(), wav());
        std::fs::remove_file(&first).unwrap();
        assert!(copy.exists(), "the copy outlives the original");
        assert_eq!(
            import(&copy, &slot).unwrap(),
            copy,
            "a copy is not copied again"
        );

        let second = root.path().join("bell.wav");
        std::fs::write(&second, wav()).unwrap();
        assert_eq!(import(&second, &slot).unwrap(), slot.join("bell.wav"));
        assert!(!copy.exists(), "the earlier copy is gone");

        remove(&slot);
        assert!(!slot.exists());
        remove(&slot);
    }

    #[test]
    fn only_playable_audio_of_a_bounded_size_is_kept() {
        let root = tempfile::tempdir().unwrap();
        let slot = global_slot(&root.path().join("state"), true);
        let text = root.path().join("notes.wav");
        std::fs::write(&text, "not a sound").unwrap();
        assert!(import(&text, &slot).is_err());
        let huge = root.path().join("huge.wav");
        std::fs::File::create(&huge)
            .unwrap()
            .set_len(MAX_BYTES + 1)
            .unwrap();
        assert!(import(&huge, &slot).is_err());
        assert!(!slot.exists(), "nothing was copied");
    }

    #[test]
    fn each_chat_has_its_own_slot_without_its_number() {
        let state = Path::new("state");
        let ada = chat_slot(state, "5511999990000@s.whatsapp.net");
        assert_eq!(ada, chat_slot(state, "5511999990000@s.whatsapp.net"));
        assert_ne!(ada, chat_slot(state, "5511999990001@s.whatsapp.net"));
        assert!(!ada.to_string_lossy().contains("5511"));
        assert_ne!(ada, global_slot(state, false));
    }

    #[test]
    fn sounds_from_older_releases_are_copied_once_and_missing_ones_left() {
        let root = tempfile::tempdir().unwrap();
        let state = root.path().join("state");
        let chosen = root.path().join("ding.wav");
        std::fs::write(&chosen, wav()).unwrap();
        let gone = NotificationSound::Custom(root.path().join("gone.wav"));
        let mut settings = Settings {
            message_sound: NotificationSound::Custom(chosen),
            mention_sound: gone.clone(),
            ..Settings::default()
        };
        assert!(adopt(&mut settings, &state));
        assert_eq!(
            settings.message_sound,
            NotificationSound::Custom(global_slot(&state, false).join("ding.wav"))
        );
        assert_eq!(settings.mention_sound, gone);
        assert!(!adopt(&mut settings, &state), "a copy is not copied again");
    }

    #[test]
    fn a_chat_sound_from_an_older_release_is_copied_into_its_account() {
        let root = tempfile::tempdir().unwrap();
        let chosen = root.path().join("ada.wav");
        std::fs::write(&chosen, wav()).unwrap();
        let archive = crate::archive::Archive::in_memory().unwrap();
        let id = "1@s.whatsapp.net";
        archive.ensure_chat(id, "Ada").unwrap();
        archive
            .set_notification_sound(id, Some(&NotificationSound::Custom(chosen)))
            .unwrap();
        let account = root.path().join("account");
        adopt_chat_sounds(&archive, &account);
        assert_eq!(
            archive.chat(id).unwrap().unwrap().notification_sound,
            Some(NotificationSound::Custom(
                chat_slot(&account, id).join("ada.wav")
            ))
        );
    }
}
