//! Messages pinned in a chat for everyone, kept in the encrypted archive.
//!
//! A pin is written here only after WhatsApp confirms it, so the archive never
//! claims something the server refused. Incoming `pin_in_chat_message` rows
//! from others land here at once. WhatsApp keeps at most three active pins
//! per chat; this table stores that cap and drops expired rows on read.

use std::collections::HashSet;

use rusqlite::params;

use super::{Archive, Result};

pub const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS message_pins (
    chat TEXT NOT NULL,
    id TEXT NOT NULL,
    pinned_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    PRIMARY KEY (chat, id)
);
CREATE INDEX IF NOT EXISTS message_pins_by_chat ON message_pins (chat, pinned_at);
";

/// Active pins one chat may keep, matching WhatsApp.
pub const MAX_ACTIVE: usize = 3;

/// One pinned message, as the list shows it.
#[derive(Clone, Debug)]
pub struct Pinned {
    pub chat: String,
    pub id: String,
    pub pinned_at: i64,
    pub expires_at: i64,
    /// The message's own words, whole, for the bubble in the list.
    pub text: String,
    pub from_me: bool,
    pub sent_at: i64,
}

impl Archive {
    /// Whether this chat already holds the WhatsApp pin cap.
    pub fn pin_full(&self, chat: &str, id: &str, now: i64) -> Result<bool> {
        let ids = self.pinned_ids(chat, now)?;
        Ok(!ids.contains(id) && ids.len() >= MAX_ACTIVE)
    }

    /// Marks a message as pinned until `expires_at`.
    pub fn pin(&self, chat: &str, id: &str, at: i64, expires_at: i64) -> Result<()> {
        self.connection.execute(
            "INSERT INTO message_pins (chat, id, pinned_at, expires_at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(chat, id) DO UPDATE SET pinned_at = excluded.pinned_at, expires_at = excluded.expires_at",
            params![chat, id, at, expires_at],
        )?;
        Ok(())
    }

    pub fn unpin(&self, chat: &str, id: &str) -> Result<()> {
        self.connection.execute(
            "DELETE FROM message_pins WHERE chat = ?1 AND id = ?2",
            params![chat, id],
        )?;
        Ok(())
    }

    /// Active pins of one chat, newest first, at most three.
    pub fn pinned_ids(&self, chat: &str, now: i64) -> Result<HashSet<String>> {
        let mut statement = self.connection.prepare(
            "SELECT id FROM message_pins
             WHERE chat = ?1 AND expires_at > ?2
             ORDER BY pinned_at DESC, rowid DESC
             LIMIT ?3",
        )?;
        let rows = statement.query_map(params![chat, now, MAX_ACTIVE as i64], |row| {
            row.get::<_, String>(0)
        })?;
        let mut ids = HashSet::new();
        for row in rows {
            ids.insert(row?);
        }
        Ok(ids)
    }

    /// Active pins of one chat in pin order (newest first).
    pub fn chat_pins(&self, chat: &str, now: i64) -> Result<Vec<Pinned>> {
        self.pinned_rows(
            "SELECT p.chat, p.id, p.pinned_at, p.expires_at, m.content, m.from_me, m.timestamp
             FROM message_pins p JOIN messages m ON m.chat = p.chat AND m.id = p.id
             WHERE p.chat = ?1 AND p.expires_at > ?2
             ORDER BY p.pinned_at DESC, p.rowid DESC LIMIT ?3",
            params![chat, now, MAX_ACTIVE as i64],
        )
    }

    /// Every active pin, newest first, for the left list.
    pub fn pinned(&self, now: i64, limit: usize) -> Result<Vec<Pinned>> {
        self.pinned_rows(
            "SELECT p.chat, p.id, p.pinned_at, p.expires_at, m.content, m.from_me, m.timestamp
             FROM message_pins p JOIN messages m ON m.chat = p.chat AND m.id = p.id
             WHERE p.expires_at > ?1
             ORDER BY p.pinned_at DESC, p.rowid DESC LIMIT ?2",
            params![now, limit as i64],
        )
    }

    fn pinned_rows(&self, sql: &str, params: impl rusqlite::Params) -> Result<Vec<Pinned>> {
        let mut statement = self.connection.prepare(sql)?;
        let rows = statement.query_map(params, |row| {
            let raw: String = row.get(4)?;
            let content: crate::model::Content =
                serde_json::from_str(&raw).unwrap_or(crate::model::Content::Unsupported {
                    what: "pinned".to_owned(),
                });
            Ok(Pinned {
                chat: row.get(0)?,
                id: row.get(1)?,
                pinned_at: row.get(2)?,
                expires_at: row.get(3)?,
                text: content.body(),
                from_me: row.get(5)?,
                sent_at: row.get(6)?,
            })
        })?;
        let mut list = Vec::new();
        for row in rows {
            list.push(row?);
        }
        Ok(list)
    }
}
