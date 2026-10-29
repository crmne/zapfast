//! Durable poll definitions and the latest vote from each sender.

use super::{Archive, Result};
use rusqlite::{OptionalExtension, params};

pub const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS polls (
    chat TEXT NOT NULL, id TEXT NOT NULL, creator TEXT NOT NULL, secret BLOB NOT NULL,
    PRIMARY KEY (chat, id)
);
CREATE TABLE IF NOT EXISTS poll_history (
    chat TEXT NOT NULL, id TEXT NOT NULL, PRIMARY KEY (chat, id)
);
CREATE TABLE IF NOT EXISTS poll_votes (
    chat TEXT NOT NULL, poll TEXT NOT NULL, voter TEXT NOT NULL, sender TEXT NOT NULL,
    update_id TEXT NOT NULL, at INTEGER NOT NULL, from_me INTEGER NOT NULL,
    choices TEXT, encrypted BLOB, attempted INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (chat, poll, voter)
);
CREATE INDEX IF NOT EXISTS pending_poll_votes ON poll_votes (at)
    WHERE choices IS NULL AND encrypted IS NOT NULL AND attempted = 0;
CREATE TRIGGER IF NOT EXISTS delete_poll_history AFTER DELETE ON messages BEGIN
    DELETE FROM poll_history WHERE chat = OLD.chat AND id = OLD.id;
END;
CREATE TRIGGER IF NOT EXISTS delete_polls AFTER DELETE ON messages BEGIN
    DELETE FROM polls WHERE chat = OLD.chat AND id = OLD.id;
    DELETE FROM poll_votes WHERE chat = OLD.chat AND poll = OLD.id;
END;
";

#[derive(Clone, Debug)]
pub struct PollVote {
    pub chat: String,
    pub poll: String,
    pub voter: String,
    /// Original wire identity, retained for vote decryption across LID migration.
    pub sender: String,
    pub update_id: String,
    pub at: i64,
    pub from_me: bool,
    pub choices: Option<Vec<usize>>,
    pub encrypted: Option<Vec<u8>>,
}

impl Archive {
    pub fn mark_poll_history(&self, chat: &str, id: &str) -> Result<()> {
        self.connection.execute(
            "INSERT OR IGNORE INTO poll_history (chat, id) VALUES (?1, ?2)",
            params![chat, id],
        )?;
        Ok(())
    }

    pub fn has_poll_history(&self, chat: &str, id: &str) -> Result<bool> {
        self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM poll_history WHERE chat = ?1 AND id = ?2)",
            params![chat, id],
            |row| row.get(0),
        )
    }

    /// Anchor after the poll, so the older-history response includes its creation
    /// message and attached vote snapshot. Anchoring at the poll would omit it.
    pub fn poll_history_anchor(&self, row: &crate::model::Message) -> Result<(String, bool, i64)> {
        let next = self
            .connection
            .query_row(
                "SELECT id, from_me, timestamp FROM messages WHERE chat = ?1
             AND (timestamp, id) > (?2, ?3) ORDER BY timestamp, id LIMIT 1",
                params![row.chat, row.timestamp, row.id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        Ok(next.unwrap_or_else(|| (String::new(), false, row.timestamp.saturating_add(1))))
    }

    pub fn save_poll(&self, chat: &str, id: &str, creator: &str, secret: &[u8]) -> Result<()> {
        self.connection.execute(
            "INSERT INTO polls (chat, id, creator, secret) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(chat, id) DO NOTHING",
            params![chat, id, creator, secret],
        )?;
        Ok(())
    }

    pub fn poll_key(&self, chat: &str, id: &str) -> Result<Option<(String, Vec<u8>)>> {
        self.connection
            .query_row(
                "SELECT creator, secret FROM polls WHERE chat = ?1 AND id = ?2",
                params![chat, id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
    }

    /// Replays and delayed older votes must never undo a later selection or withdrawal.
    pub fn save_poll_vote(&self, vote: &PollVote) -> Result<bool> {
        self.connection.execute(
            "INSERT INTO poll_votes (chat, poll, voter, sender, update_id, at, from_me, choices, encrypted)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(chat, poll, voter) DO UPDATE SET
                sender = excluded.sender, update_id = excluded.update_id, at = excluded.at,
                from_me = excluded.from_me, choices = excluded.choices, encrypted = excluded.encrypted,
                attempted = 0
             WHERE (excluded.at, excluded.update_id) > (poll_votes.at, poll_votes.update_id)
                OR ((excluded.at, excluded.update_id) = (poll_votes.at, poll_votes.update_id)
                    AND poll_votes.choices IS NULL AND excluded.choices IS NOT NULL)",
            params![vote.chat, vote.poll, vote.voter, vote.sender, vote.update_id, vote.at,
                vote.from_me, vote.choices.as_ref().map(|choices| serde_json::to_string(choices).unwrap()), vote.encrypted],
        ).map(|changed| changed > 0)
    }

    pub fn poll_votes(&self, chat: &str, poll: &str) -> Result<Vec<PollVote>> {
        let mut statement = self.connection.prepare(
            "SELECT chat, poll, voter, sender, update_id, at, from_me, choices, encrypted
             FROM poll_votes WHERE chat = ?1 AND poll = ?2 ORDER BY at, update_id",
        )?;
        statement
            .query_map(params![chat, poll], vote_row)?
            .collect()
    }

    /// At most one small batch is decrypted at a time. Unknown parent polls stay queued.
    pub fn pending_poll_votes(&self, limit: usize) -> Result<Vec<PollVote>> {
        let mut statement = self.connection.prepare(
            "SELECT v.chat, v.poll, v.voter, v.sender, v.update_id, v.at, v.from_me, v.choices, v.encrypted
             FROM poll_votes v JOIN polls p ON p.chat = v.chat AND p.id = v.poll
             JOIN messages m ON m.chat = v.chat AND m.id = v.poll
             WHERE v.choices IS NULL AND v.encrypted IS NOT NULL AND v.attempted = 0
               AND json_extract(m.content, '$.kind') = 'poll'
             ORDER BY v.at LIMIT ?1")?;
        statement.query_map([limit as i64], vote_row)?.collect()
    }

    pub fn attempt_poll_vote(&self, vote: &PollVote) -> Result<()> {
        self.connection.execute(
            "UPDATE poll_votes SET attempted = 1
             WHERE chat = ?1 AND poll = ?2 AND voter = ?3 AND at = ?4 AND update_id = ?5",
            params![vote.chat, vote.poll, vote.voter, vote.at, vote.update_id],
        )?;
        Ok(())
    }

    /// A late decryption must not resurrect a deleted poll or replace a newer vote.
    pub fn finish_poll_vote(&self, vote: &PollVote, choices: &[usize]) -> Result<bool> {
        self.connection
            .execute(
                "UPDATE poll_votes SET choices = ?6 WHERE chat = ?1 AND poll = ?2 AND voter = ?3
             AND at = ?4 AND update_id = ?5 AND choices IS NULL",
                params![
                    vote.chat,
                    vote.poll,
                    vote.voter,
                    vote.at,
                    vote.update_id,
                    serde_json::to_string(choices).unwrap()
                ],
            )
            .map(|changed| changed > 0)
    }

    pub fn retry_poll_votes(&self) -> Result<()> {
        self.connection.execute(
            "UPDATE poll_votes SET attempted = 0 WHERE choices IS NULL",
            [],
        )?;
        Ok(())
    }
}

fn vote_row(row: &rusqlite::Row<'_>) -> Result<PollVote> {
    let choices: Option<String> = row.get(7)?;
    Ok(PollVote {
        chat: row.get(0)?,
        poll: row.get(1)?,
        voter: row.get(2)?,
        sender: row.get(3)?,
        update_id: row.get(4)?,
        at: row.get(5)?,
        from_me: row.get(6)?,
        choices: choices.map(|value| serde_json::from_str(&value).unwrap_or_default()),
        encrypted: row.get(8)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Content, PollState};

    fn vote(at: i64, choices: Option<Vec<usize>>) -> PollVote {
        PollVote {
            chat: "chat".into(),
            poll: "poll".into(),
            voter: "voter".into(),
            sender: "voter".into(),
            update_id: format!("vote-{at}"),
            at,
            from_me: true,
            choices,
            encrypted: Some(vec![1, 2, 3]),
        }
    }

    #[test]
    fn poll_recovery_anchors_after_the_parent_including_the_latest_message() {
        let archive = Archive::in_memory().unwrap();
        let poll = super::super::tests::message("chat", "poll", 100, false);
        archive.insert_message(&poll, None).unwrap();
        assert_eq!(
            archive.poll_history_anchor(&poll).unwrap(),
            (String::new(), false, 101)
        );
        archive
            .insert_message(
                &super::super::tests::message("chat", "next", 110, true),
                None,
            )
            .unwrap();
        assert_eq!(
            archive.poll_history_anchor(&poll).unwrap(),
            ("next".into(), true, 110)
        );
        archive.mark_poll_history("chat", "poll").unwrap();
        assert!(archive.has_poll_history("chat", "poll").unwrap());
        archive.delete_message("chat", "poll").unwrap();
        assert!(!archive.has_poll_history("chat", "poll").unwrap());
    }

    #[test]
    fn re_votes_withdrawals_and_pending_votes_survive_encrypted_restart() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("archive.db");
        let archive = Archive::open_with_key(&path, &[42; 32]).unwrap();
        let mut row = super::super::tests::message("chat", "poll", 1, false);
        row.content = Content::Poll {
            question: "Lunch?".into(),
            options: vec!["A".into(), "B".into()],
            state: PollState::default(),
        };
        // A vote may arrive before the creation message, then becomes decryptable.
        archive.save_poll_vote(&vote(10, None)).unwrap();
        assert!(archive.pending_poll_votes(8).unwrap().is_empty());
        archive.insert_message(&row, None).unwrap();
        archive
            .save_poll("chat", "poll", "creator", &[7; 32])
            .unwrap();
        assert_eq!(archive.pending_poll_votes(8).unwrap().len(), 1);
        archive.attempt_poll_vote(&vote(10, None)).unwrap();
        assert!(archive.pending_poll_votes(8).unwrap().is_empty());
        assert!(archive.finish_poll_vote(&vote(10, None), &[0]).unwrap());
        assert!(archive.save_poll_vote(&vote(20, Some(vec![1]))).unwrap());
        assert!(!archive.save_poll_vote(&vote(10, Some(vec![0]))).unwrap());
        assert!(!archive.finish_poll_vote(&vote(10, None), &[0]).unwrap());
        assert!(archive.save_poll_vote(&vote(30, Some(vec![]))).unwrap());
        // Replaying the parent cannot reset votes kept separately from its content.
        archive.insert_message(&row, None).unwrap();
        drop(archive);
        let archive = Archive::open_with_key(&path, &[42; 32]).unwrap();
        assert_eq!(
            archive.poll_key("chat", "poll").unwrap(),
            Some(("creator".into(), vec![7; 32]))
        );
        let votes = archive.poll_votes("chat", "poll").unwrap();
        assert_eq!(votes.len(), 1);
        assert_eq!(votes[0].choices, Some(vec![]));
        assert_eq!(votes[0].at, 30);
        archive.delete_message("chat", "poll").unwrap();
        assert!(archive.poll_key("chat", "poll").unwrap().is_none());
        assert!(archive.poll_votes("chat", "poll").unwrap().is_empty());
        assert!(!archive.finish_poll_vote(&vote(30, None), &[1]).unwrap());
        archive.save_poll_vote(&vote(40, None)).unwrap();
        archive.clear().unwrap();
        assert!(archive.poll_votes("chat", "poll").unwrap().is_empty());
    }
}
