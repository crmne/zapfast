//! Local voice transcripts live on their encrypted message rows, separate from protocol content.

use super::{Archive, Result};
use rusqlite::{OptionalExtension, params};

pub(super) const SCHEMA: &str = "
CREATE TRIGGER IF NOT EXISTS clear_voice_transcription AFTER UPDATE OF content ON messages
WHEN COALESCE(json_extract(NEW.content, '$.kind') = 'audio'
    AND json_extract(NEW.content, '$.voice_note') = 1, 0) = 0
BEGIN
    UPDATE messages SET transcription = NULL WHERE chat = NEW.chat AND id = NEW.id;
END;
";

impl Archive {
    /// A late result never recreates a deleted or revoked message.
    pub fn save_voice_transcript(&self, chat: &str, message: &str, text: &str) -> Result<bool> {
        if text.trim().is_empty() {
            return Ok(false);
        }
        let changed = self.connection.execute(
            "UPDATE messages SET transcription = ?3 WHERE chat = ?1 AND id = ?2
             AND json_extract(content, '$.kind') = 'audio'
             AND json_extract(content, '$.voice_note') = 1",
            params![chat, message, text],
        )?;
        Ok(changed > 0)
    }

    /// Restores just the rows being delivered to the interface, including older pages.
    pub fn voice_transcripts(
        &self,
        chat: &str,
        messages: &[String],
    ) -> Result<Vec<(String, String)>> {
        let mut statement = self.connection.prepare_cached(
            "SELECT transcription FROM messages WHERE chat = ?1 AND id = ?2
             AND transcription IS NOT NULL",
        )?;
        let mut transcripts = Vec::new();
        for message in messages {
            if let Some(text) = statement
                .query_row(params![chat, message], |row| row.get::<_, String>(0))
                .optional()?
            {
                transcripts.push((message.clone(), text));
            }
        }
        Ok(transcripts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Content, Media, MediaState};

    fn voice(chat: &str) -> crate::model::Message {
        let mut message = super::super::tests::message(chat, "voice", 1, false);
        message.content = Content::Audio {
            media: Media {
                mime: "audio/ogg".into(),
                size: 0,
                width: None,
                height: None,
                path: None,
                state: MediaState::Idle,
            },
            seconds: Some(10),
            voice_note: true,
            waveform: Vec::new(),
        };
        message
    }

    #[test]
    fn transcripts_survive_encrypted_restart_and_history_replay() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("archive.db");
        let key = [42; 32];
        let archive = Archive::open_with_key(&path, &key).unwrap();
        archive.insert_message(&voice("chat"), None).unwrap();
        archive.insert_message(&voice("other"), None).unwrap();
        assert!(
            archive
                .save_voice_transcript("chat", "voice", "Synthetic speech")
                .unwrap()
        );
        drop(archive);
        let archive = Archive::open_with_key(&path, &key).unwrap();
        archive.insert_message(&voice("chat"), None).unwrap();
        assert_eq!(
            archive
                .voice_transcripts("chat", &["voice".into()])
                .unwrap(),
            vec![("voice".into(), "Synthetic speech".into())]
        );
        assert!(
            archive
                .voice_transcripts("other", &["voice".into()])
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn revoked_deleted_and_non_voice_messages_cannot_keep_transcripts() {
        let archive = Archive::in_memory().unwrap();
        archive.insert_message(&voice("chat"), None).unwrap();
        archive
            .save_voice_transcript("chat", "voice", "Synthetic speech")
            .unwrap();
        archive
            .set_content("chat", "voice", &Content::Revoked, false)
            .unwrap();
        assert!(
            archive
                .voice_transcripts("chat", &["voice".into()])
                .unwrap()
                .is_empty()
        );
        assert!(
            !archive
                .save_voice_transcript("chat", "voice", "Late speech")
                .unwrap()
        );
        archive.insert_message(&voice("chat"), None).unwrap();
        archive
            .save_voice_transcript("chat", "voice", "Synthetic speech")
            .unwrap();
        archive
            .connection
            .execute("DELETE FROM messages WHERE chat = 'chat'", [])
            .unwrap();
        assert!(
            !archive
                .save_voice_transcript("chat", "voice", "Late speech")
                .unwrap()
        );
    }
}
