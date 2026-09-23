//! Per-chat read-receipt preferences: one chat can override the global setting.
//!
//! Only chats that disagree with the global setting have a row here. No row
//! means the chat follows the global setting, which is also how a chat goes
//! back to it. The preference lives in the encrypted archive because it names
//! the chats it applies to.

use super::{Archive, Result, params};

pub const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS chat_read_receipts (
    chat TEXT PRIMARY KEY,
    receipts INTEGER NOT NULL
);
";

impl Archive {
    /// Every chat that overrides the global read-receipt setting.
    pub fn read_receipts(&self) -> Result<Vec<(String, bool)>> {
        let mut statement = self
            .connection
            .prepare("SELECT chat, receipts FROM chat_read_receipts")?;
        let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get::<_, i64>(1)? != 0)))?;
        rows.collect()
    }

    /// Sets one chat's override. `None` returns the chat to the global setting
    /// by dropping the row, so the table never grows stale entries.
    pub fn set_read_receipts(&self, chat: &str, receipts: Option<bool>) -> Result<()> {
        let Some(receipts) = receipts else {
            self.connection.execute(
                "DELETE FROM chat_read_receipts WHERE chat = ?1",
                params![chat],
            )?;
            return Ok(());
        };
        self.connection.execute(
            "INSERT INTO chat_read_receipts (chat, receipts) VALUES (?1, ?2)
             ON CONFLICT(chat) DO UPDATE SET receipts = excluded.receipts",
            params![chat, i64::from(receipts)],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ReadReceiptPreference;

    #[test]
    fn a_chat_preference_inherits_forces_or_blocks_and_cycles_back() {
        use crate::model::ReadReceiptPreference::{Explicit, InheritGlobal};
        assert!(InheritGlobal.resolve(true));
        assert!(!InheritGlobal.resolve(false));
        assert!(
            Explicit(true).resolve(false),
            "an explicit yes beats the global setting"
        );
        assert!(
            !Explicit(false).resolve(true),
            "an explicit no beats it too"
        );
        assert_eq!(InheritGlobal.next(), Explicit(true));
        assert_eq!(Explicit(true).next(), Explicit(false));
        assert_eq!(
            Explicit(false).next(),
            InheritGlobal,
            "the cycle returns to the default"
        );
        assert_eq!(InheritGlobal.stored(), None);
        assert_eq!(Explicit(false).stored(), Some(false));
        assert_eq!(
            ReadReceiptPreference::from_stored(Some(true)),
            Explicit(true)
        );
        assert_eq!(ReadReceiptPreference::from_stored(None), InheritGlobal);
    }

    #[test]
    fn an_override_survives_a_restart_and_a_chat_can_follow_the_global_again() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("fixture.db");
        let key = [7; 32];
        {
            let archive = Archive::open_with_key(&path, &key).unwrap();
            archive.ensure_chat("1@s.whatsapp.net", "A").unwrap();
            archive.ensure_chat("2@s.whatsapp.net", "B").unwrap();
            archive
                .set_read_receipts("1@s.whatsapp.net", Some(false))
                .unwrap();
            archive
                .set_read_receipts("2@s.whatsapp.net", Some(true))
                .unwrap();
            // One row per chat, holding the latest answer.
            archive
                .set_read_receipts("1@s.whatsapp.net", Some(true))
                .unwrap();
        }
        let archive = Archive::open_with_key(&path, &key).unwrap();
        let mut overrides = archive.read_receipts().unwrap();
        overrides.sort();
        assert_eq!(
            overrides,
            vec![
                ("1@s.whatsapp.net".to_owned(), true),
                ("2@s.whatsapp.net".to_owned(), true),
            ]
        );

        // Following the global setting again leaves no row behind.
        archive.set_read_receipts("2@s.whatsapp.net", None).unwrap();
        assert_eq!(
            archive.read_receipts().unwrap(),
            vec![("1@s.whatsapp.net".to_owned(), true)]
        );

        // Deleting the chat drops its override with its other rows.
        archive.clear_chat("1@s.whatsapp.net").unwrap();
        assert!(archive.read_receipts().unwrap().is_empty());
    }
}
