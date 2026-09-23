//! Per-chat composer drafts, so unsent text survives a restart.
//!
//! Drafts hold text the user typed and did not send, so they live in the
//! encrypted archive next to the messages they belong to and are dropped with
//! the chat.

use super::{Archive, Result, params};

pub const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS drafts (
    chat TEXT PRIMARY KEY,
    text TEXT NOT NULL,
    updated_at INTEGER NOT NULL DEFAULT 0
);
";

impl Archive {
    /// Every stored draft, so the composer can restore them at startup.
    pub fn drafts(&self) -> Result<Vec<(String, String)>> {
        let mut statement = self.connection.prepare("SELECT chat, text FROM drafts")?;
        let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        rows.collect()
    }

    /// Stores a draft for a chat. An empty or blank text clears the row
    /// instead, because a blank composer is not a draft.
    pub fn set_draft(&self, chat: &str, text: &str, at: i64) -> Result<()> {
        if text.trim().is_empty() {
            return self.clear_draft(chat);
        }
        self.connection.execute(
            "INSERT INTO drafts (chat, text, updated_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(chat) DO UPDATE SET
                text = excluded.text,
                updated_at = excluded.updated_at",
            params![chat, text, at],
        )?;
        Ok(())
    }

    /// Drops the draft for a chat, for example once its text is sent.
    pub fn clear_draft(&self, chat: &str) -> Result<()> {
        self.connection
            .execute("DELETE FROM drafts WHERE chat = ?1", params![chat])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drafts_round_trip_survive_a_restart_and_clear() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("fixture.db");
        let key = [17; 32];
        {
            let archive = Archive::open_with_key(&path, &key).unwrap();
            archive.ensure_chat("1@s.whatsapp.net", "A").unwrap();
            archive.ensure_chat("2@s.whatsapp.net", "B").unwrap();
            archive.set_draft("1@s.whatsapp.net", "unsent", 10).unwrap();
            archive.set_draft("2@s.whatsapp.net", "other", 11).unwrap();
            // The same chat keeps one row, whatever the text.
            archive
                .set_draft("1@s.whatsapp.net", "unsent again", 12)
                .unwrap();
        }
        let archive = Archive::open_with_key(&path, &key).unwrap();
        let mut drafts = archive.drafts().unwrap();
        drafts.sort();
        assert_eq!(
            drafts,
            vec![
                ("1@s.whatsapp.net".to_owned(), "unsent again".to_owned()),
                ("2@s.whatsapp.net".to_owned(), "other".to_owned()),
            ]
        );

        // A blank composer is not a draft.
        archive.set_draft("1@s.whatsapp.net", "   ", 13).unwrap();
        assert_eq!(
            archive.drafts().unwrap(),
            vec![("2@s.whatsapp.net".to_owned(), "other".to_owned())]
        );

        // Sending the text, or dropping the chat, drops the draft.
        archive.clear_draft("2@s.whatsapp.net").unwrap();
        assert!(archive.drafts().unwrap().is_empty());
        archive.set_draft("2@s.whatsapp.net", "again", 14).unwrap();
        archive.clear_chat("2@s.whatsapp.net").unwrap();
        assert!(archive.drafts().unwrap().is_empty());
    }
}
