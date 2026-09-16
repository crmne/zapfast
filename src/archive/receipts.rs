//! Group delivery is the least advanced recipient, using the audience saved
//! when we send. A later membership change must not change that audience.

use super::{Archive, Result, params, status_from_rank, status_rank};
use crate::model::Delivery;

impl Archive {
    /// Called once for a newly filed outgoing message, before sending it.
    /// An empty/unknown audience cannot establish that everyone has read.
    pub fn snapshot_group_recipients(
        &self,
        chat: &str,
        id: &str,
        recipients: &[String],
    ) -> Result<()> {
        let transaction = self.connection.unchecked_transaction()?;
        transaction.query_row(
            "SELECT 1 FROM messages WHERE chat = ?1 AND id = ?2 AND from_me = 1",
            params![chat, id],
            |row| row.get::<_, i64>(0),
        )?;
        for recipient in recipients {
            transaction.execute(
                "INSERT INTO group_receipts (chat, id, recipient, expected) VALUES (?1, ?2, ?3, 1)
                 ON CONFLICT(chat, id, recipient) DO UPDATE SET expected = 1",
                params![chat, id, recipient],
            )?;
        }
        transaction.commit()
    }

    /// Records only the named message: a group member reading a later message
    /// does not prove that all members read any earlier ones.
    pub fn group_receipt(
        &self,
        chat: &str,
        id: &str,
        recipient: &str,
        status: Delivery,
        at: i64,
    ) -> Result<bool> {
        if !matches!(
            status,
            Delivery::Delivered | Delivery::Read | Delivery::Played
        ) {
            return Ok(false);
        }
        let transaction = self.connection.unchecked_transaction()?;
        transaction.execute(
            "INSERT INTO group_receipts (chat, id, recipient, status, delivered_at, read_at, played_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(chat, id, recipient) DO UPDATE SET
                status = MAX(status, excluded.status),
                delivered_at = COALESCE(MIN(delivered_at, excluded.delivered_at), delivered_at, excluded.delivered_at),
                read_at = COALESCE(MIN(read_at, excluded.read_at), read_at, excluded.read_at),
                played_at = COALESCE(MIN(played_at, excluded.played_at), played_at, excluded.played_at)",
            params![chat, id, recipient, status_rank(status), at,
                (status >= Delivery::Read).then_some(at),
                (status == Delivery::Played).then_some(at)],
        )?;
        let (rank, delivered_at, read_at, played_at): (
            Option<i64>,
            Option<i64>,
            Option<i64>,
            Option<i64>,
        ) = transaction.query_row(
            "SELECT MIN(status), MAX(delivered_at), MAX(read_at), MAX(played_at)
             FROM group_receipts WHERE chat = ?1 AND id = ?2 AND expected = 1",
            params![chat, id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?;
        let aggregate = status_from_rank(rank.unwrap_or(0));
        let mut changed = false;
        if aggregate >= Delivery::Delivered {
            changed |=
                self.set_status(chat, id, Delivery::Delivered, delivered_at.unwrap_or(at))?;
        }
        if aggregate >= Delivery::Read {
            changed |= self.set_status(chat, id, Delivery::Read, read_at.unwrap_or(at))?;
        }
        if aggregate >= Delivery::Played {
            changed |= self.set_status(chat, id, Delivery::Played, played_at.unwrap_or(at))?;
        }
        transaction.commit()?;
        Ok(changed)
    }

    /// A privacy id and a phone number identify one person, not two readers.
    pub(super) fn merge_group_recipient(&self, lid: &str, pn: &str) -> Result<()> {
        let transaction = self.connection.unchecked_transaction()?;
        transaction.execute(
            "INSERT INTO group_receipts (chat, id, recipient, expected, status, delivered_at, read_at, played_at)
             SELECT chat, id, ?2, expected, status, delivered_at, read_at, played_at
             FROM group_receipts WHERE recipient = ?1
             ON CONFLICT(chat, id, recipient) DO UPDATE SET
                expected = MAX(expected, excluded.expected),
                status = MAX(status, excluded.status),
                delivered_at = COALESCE(MIN(delivered_at, excluded.delivered_at), delivered_at, excluded.delivered_at),
                read_at = COALESCE(MIN(read_at, excluded.read_at), read_at, excluded.read_at),
                played_at = COALESCE(MIN(played_at, excluded.played_at), played_at, excluded.played_at)",
            params![lid, pn],
        )?;
        transaction.execute("DELETE FROM group_receipts WHERE recipient = ?1", [lid])?;
        transaction.commit()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn group_receipts_survive_reopening_and_use_the_last_readers_time() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("archive.db");
        let group = "123-456@g.us";
        {
            let archive = Archive::open_with_key(&path, &[7; 32]).unwrap();
            archive.ensure_chat(group, "Group").unwrap();
            archive
                .insert_message(&super::super::tests::message(group, "m", 100, true), None)
                .unwrap();
            archive.set_status(group, "m", Delivery::Sent, 100).unwrap();
            archive
                .snapshot_group_recipients(group, "m", &["a@lid".into(), "b@lid".into()])
                .unwrap();
            assert!(
                !archive
                    .group_receipt(group, "m", "a@lid", Delivery::Read, 150)
                    .unwrap()
            );
        }
        let archive = Archive::open_with_key(&path, &[7; 32]).unwrap();
        assert!(
            archive
                .group_receipt(group, "m", "b@lid", Delivery::Delivered, 130)
                .unwrap()
        );
        let row = archive.message(group, "m").unwrap().unwrap();
        assert_eq!(row.status, Delivery::Delivered);
        assert_eq!(row.delivered_at, Some(150));
        assert_eq!(row.read_at, None);
        assert!(
            archive
                .group_receipt(group, "m", "b@lid", Delivery::Read, 170)
                .unwrap()
        );
        let row = archive.message(group, "m").unwrap().unwrap();
        assert_eq!(row.status, Delivery::Read);
        assert_eq!(row.read_at, Some(170));
        assert!(
            !archive
                .group_receipt(group, "m", "a@lid", Delivery::Played, 180)
                .unwrap()
        );
        assert!(
            archive
                .group_receipt(group, "m", "b@lid", Delivery::Played, 190)
                .unwrap()
        );
        archive.delete_message(group, "m").unwrap();
        assert_eq!(
            archive
                .connection
                .query_row("SELECT COUNT(*) FROM group_receipts", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        drop(archive);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn unknown_group_audience_cannot_prove_everyone_has_read() {
        let archive = Archive::in_memory().unwrap();
        let group = "123-456@g.us";
        archive.ensure_chat(group, "Group").unwrap();
        archive
            .insert_message(
                &super::super::tests::message(group, "unknown", 50, true),
                None,
            )
            .unwrap();
        assert!(
            !archive
                .group_receipt("123-456@g.us", "unknown", "a@lid", Delivery::Read, 100)
                .unwrap()
        );
        archive.clear().unwrap();
        assert_eq!(
            archive
                .connection
                .query_row("SELECT COUNT(*) FROM group_receipts", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}
