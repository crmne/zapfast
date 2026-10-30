//! What the voice messages say, transcribed on this computer.
//!
//! The text is derived here and never leaves the machine. It lives in its own
//! table so the message content stays as WhatsApp sent it, and the trigger
//! removes it with the message: a deleted, revoked or cleared message takes
//! its transcript along with it.

use std::collections::HashMap;

use rusqlite::{params, params_from_iter};

use super::{Archive, Result};

pub const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS transcripts (
    chat TEXT NOT NULL,
    id TEXT NOT NULL,
    text TEXT NOT NULL,
    engine TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (chat, id)
);
CREATE TRIGGER IF NOT EXISTS delete_transcripts AFTER DELETE ON messages BEGIN
    DELETE FROM transcripts WHERE chat = OLD.chat AND id = OLD.id;
END;
-- A delete for everyone keeps the row and replaces its content; the text the
-- sender removed must go with it.
CREATE TRIGGER IF NOT EXISTS revoke_transcripts AFTER UPDATE OF content ON messages
WHEN json_valid(NEW.content) AND json_extract(NEW.content, '$.kind') = 'revoked' BEGIN
    DELETE FROM transcripts WHERE chat = NEW.chat AND id = NEW.id;
END;
";

impl Archive {
    /// Canonical rows whose previously unknown LID copy was revoked. The
    /// worker reads this before merging so already-drawn words are refreshed
    /// afterward, rather than lingering until the chat is opened again.
    pub(crate) fn lid_revocations(&self, lid_chat: &str, phone_chat: &str) -> Result<Vec<String>> {
        let mut statement = self.connection.prepare(
            "SELECT p.id FROM messages p JOIN messages l ON p.id = l.id
             WHERE p.chat = ?2 AND l.chat = ?1
                 AND json_extract(l.content, '$.kind') = 'revoked'
                 AND json_extract(p.content, '$.kind') != 'revoked'",
        )?;
        statement
            .query_map(params![lid_chat, phone_chat], |row| row.get(0))?
            .collect()
    }

    /// Stores a transcript, replacing an older one of the same message. A
    /// message deleted while it was being transcribed writes nothing, so the
    /// table never holds a row without its message.
    pub fn set_transcript(&self, chat: &str, id: &str, text: &str, engine: &str) -> Result<bool> {
        let stored = self.connection.execute(
            "INSERT INTO transcripts (chat, id, text, engine, created_at)
             SELECT ?1, ?2, ?3, ?4, ?5
             WHERE EXISTS (SELECT 1 FROM messages WHERE chat = ?1 AND id = ?2
                 AND NOT (json_valid(content) AND json_extract(content, '$.kind') = 'revoked'))
             ON CONFLICT(chat, id) DO UPDATE SET
                text = excluded.text,
                engine = excluded.engine,
                created_at = excluded.created_at",
            params![chat, id, text, engine, crate::util::now()],
        )?;
        Ok(stored > 0)
    }

    /// The transcripts among `ids` in one chat, by message id.
    pub fn transcripts(&self, chat: &str, ids: &[String]) -> Result<HashMap<String, String>> {
        if ids.is_empty() {
            return Ok(HashMap::new());
        }
        let placeholders = std::iter::repeat_n("?", ids.len())
            .collect::<Vec<_>>()
            .join(",");
        let mut statement = self.connection.prepare(&format!(
            "SELECT id, text FROM transcripts WHERE chat = ?1 AND id IN ({placeholders})"
        ))?;
        let mut values: Vec<&dyn rusqlite::ToSql> = Vec::with_capacity(ids.len() + 1);
        values.push(&chat);
        for id in ids {
            values.push(id);
        }
        let rows = statement.query_map(params_from_iter(values), |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut found = HashMap::new();
        for row in rows {
            let (id, text) = row?;
            found.insert(id, text);
        }
        Ok(found)
    }

    /// Fills each message's `transcript` from the archive.
    pub(crate) fn attach_transcripts(
        &self,
        chat: &str,
        messages: &mut [crate::model::Message],
    ) -> Result<()> {
        let ids: Vec<String> = messages.iter().map(|message| message.id.clone()).collect();
        let mut found = self.transcripts(chat, &ids)?;
        for message in messages {
            message.transcript = found.remove(&message.id);
        }
        Ok(())
    }
}
