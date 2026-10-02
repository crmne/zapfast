//! Messages the user starred, kept in the encrypted archive.
//!
//! A star is written here only after WhatsApp confirms it, or when a
//! `StarUpdate` arrives from the phone. The row keeps the latest event time,
//! so a late or replayed star cannot undo a newer unstar. Two events that
//! carry the same second are ordered by where they came from: a live event
//! wins over a replayed one, and only a live event may replace a row it did
//! not move forward in time.
//!
//! The table holds one row per starred message and no copy of it: the message
//! is read from its own row, whole, so the right-click menu of a row in the
//! Starred panel offers every field an in-chat menu does. An edited message
//! shows its current words, and one deleted here, or revoked for everyone,
//! leaves the list on its own. The row draws those words inside a bubble of
//! the message's own side and fill, not the conversation's full renderer.

use std::collections::HashSet;

use rusqlite::params;

use super::{Archive, Result};
use crate::model::Content;

pub const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS stars (
    chat TEXT NOT NULL,
    id TEXT NOT NULL,
    starred INTEGER NOT NULL,
    starred_at INTEGER NOT NULL,
    PRIMARY KEY (chat, id)
);
CREATE INDEX IF NOT EXISTS stars_by_time ON stars (starred_at);
";

/// One starred message, as the list shows it.
#[derive(Clone, Debug)]
pub struct Starred {
    /// The message itself, whole. Personal data: never logged.
    pub message: crate::model::Message,
    /// Unix seconds of the moment the star was confirmed.
    pub starred_at: i64,
}

impl Archive {
    /// Records a star or an unstar at `at` (Unix seconds).
    ///
    /// An older event loses to the time already stored, so a response that
    /// finished late cannot put the row back. `replayed` says the event came
    /// from a history replay, which the phone may send again at any time:
    /// with the times equal to the second, only a live event is allowed to
    /// replace the row. A replay that carried the same second would otherwise
    /// put back a star the user has already removed.
    pub fn set_star(
        &self,
        chat: &str,
        id: &str,
        starred: bool,
        at: i64,
        replayed: bool,
    ) -> Result<bool> {
        let changed = self.connection.execute(
            "INSERT INTO stars (chat, id, starred, starred_at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(chat, id) DO UPDATE SET
                starred = excluded.starred,
                starred_at = excluded.starred_at
             WHERE excluded.starred_at > stars.starred_at
                OR (excluded.starred_at = stars.starred_at AND ?5 = 0)",
            params![chat, id, starred, at, replayed],
        )?;
        Ok(changed > 0)
    }

    /// Marks a message as starred, remembering when. The callers here are
    /// local writes and confirmed answers, so the event is never a replay.
    pub fn star(&self, chat: &str, id: &str, at: i64) -> Result<()> {
        self.set_star(chat, id, true, at, false)?;
        Ok(())
    }

    pub fn unstar(&self, chat: &str, id: &str, at: i64) -> Result<()> {
        self.set_star(chat, id, false, at, false)?;
        Ok(())
    }

    /// Drops the star row of one message. A deleted message must not leave a
    /// row that a reused id could inherit.
    pub fn delete_star(&self, chat: &str, id: &str) -> Result<()> {
        self.connection.execute(
            "DELETE FROM stars WHERE chat = ?1 AND id = ?2",
            params![chat, id],
        )?;
        Ok(())
    }

    /// The starred messages of one chat, for the mark in the conversation.
    pub fn starred_ids(&self, chat: &str) -> Result<HashSet<String>> {
        let mut statement = self
            .connection
            .prepare("SELECT id FROM stars WHERE chat = ?1 AND starred = 1")?;
        let rows = statement.query_map(params![chat], |row| row.get::<_, String>(0))?;
        let mut ids = HashSet::new();
        for row in rows {
            ids.insert(row?);
        }
        Ok(ids)
    }

    /// Every starred message, newest star first, for the list. The message is
    /// read from its own row, so the row has its current words and its menu
    /// every field an in-chat menu has; a message deleted here leaves the list
    /// on its own. The join on `chats` keeps a locked chat out until the folder
    /// is open.
    pub fn starred(&self, limit: usize) -> Result<Vec<Starred>> {
        let rows: Vec<(i64, String, String)> = {
            let mut statement = self.connection.prepare(
                "SELECT s.starred_at, s.chat, s.id
                 FROM stars s
                 JOIN chats c ON c.id = s.chat AND c.locked = 0
                 WHERE s.starred = 1
                 ORDER BY s.starred_at DESC, s.rowid DESC LIMIT ?1",
            )?;
            let rows = statement.query_map(params![limit as i64], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })?;
            rows.collect::<Result<_>>()?
        };
        let mut list = Vec::new();
        for (starred_at, chat, id) in rows {
            if let Some(message) = self.message(&chat, &id)? {
                // A message revoked for everyone keeps its row, with the
                // revoked content in it. It has no words and no Unstar left, so
                // it is not a row the list can offer.
                if matches!(message.content, Content::Revoked) {
                    continue;
                }
                list.push(Starred {
                    message,
                    starred_at,
                });
            }
        }
        Ok(list)
    }
}
