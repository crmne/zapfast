//! Group delivery is the least advanced recipient, using the audience saved
//! when we send. A later membership change must not change that audience.
//!
//! Each group recipient's receipt is kept for "Message info". A direct chat's
//! receipts live on the message row; they pass through here only while they
//! wait for a message that has not arrived yet.

use super::{Archive, Result, params, status_from_rank, status_rank};
use crate::model::{Delivery, Recipient};

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
        if !self.file_receipt(chat, id, recipient, status, at)? {
            return Ok(false);
        }
        self.settle_group(chat, id)
    }

    /// Keeps one recipient's receipt without touching the message's ticks.
    /// Returns whether `status` is a receipt worth keeping.
    ///
    /// A receipt can arrive before the message it names: our other devices'
    /// sends are decrypted while receipts are handled, so those rows wait here
    /// for [`Self::settle_group`] or [`Self::settle_direct`].
    pub fn file_receipt(
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
        self.connection.execute(
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
        Ok(true)
    }

    /// Files many receipts for one group message at once, as history sync
    /// reports them. Returns whether the message's ticks moved.
    pub fn file_receipts(
        &self,
        chat: &str,
        id: &str,
        receipts: &[(String, Delivery, i64)],
    ) -> Result<bool> {
        let transaction = self.connection.unchecked_transaction()?;
        for (recipient, status, at) in receipts {
            self.file_receipt(chat, id, recipient, *status, *at)?;
        }
        transaction.commit()?;
        if self.has_group_audience(chat, id)? {
            self.settle_group(chat, id)
        } else {
            Ok(false)
        }
    }

    /// Whether the audience of a group message was saved.
    pub fn has_group_audience(&self, chat: &str, id: &str) -> Result<bool> {
        self.connection.query_row(
            "SELECT EXISTS (SELECT 1 FROM group_receipts WHERE chat = ?1 AND id = ?2 AND expected = 1)",
            params![chat, id],
            |row| row.get(0),
        )
    }

    /// Moves a group message's ticks to its least advanced saved recipient.
    /// Returns whether the message's state changed.
    pub fn settle_group(&self, chat: &str, id: &str) -> Result<bool> {
        let transaction = self.connection.unchecked_transaction()?;
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
        let now = crate::util::now();
        let mut changed = false;
        if aggregate >= Delivery::Delivered {
            changed |=
                self.set_status(chat, id, Delivery::Delivered, delivered_at.unwrap_or(now))?;
        }
        if aggregate >= Delivery::Read {
            changed |= self.set_status(chat, id, Delivery::Read, read_at.unwrap_or(now))?;
        }
        if aggregate >= Delivery::Played {
            changed |= self.set_status(chat, id, Delivery::Played, played_at.unwrap_or(now))?;
        }
        transaction.commit()?;
        Ok(changed)
    }

    /// Applies receipts that arrived before a direct message, then forgets
    /// them: the message row keeps a direct chat's delivery times. Returns the
    /// furthest state reached and when, if the message moved.
    pub fn settle_direct(&self, chat: &str, id: &str) -> Result<Option<(Delivery, i64)>> {
        let transaction = self.connection.unchecked_transaction()?;
        let (rank, delivered_at, read_at, played_at): (
            Option<i64>,
            Option<i64>,
            Option<i64>,
            Option<i64>,
        ) = transaction.query_row(
            "SELECT MAX(status), MIN(delivered_at), MIN(read_at), MIN(played_at)
             FROM group_receipts WHERE chat = ?1 AND id = ?2",
            params![chat, id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?;
        let Some(rank) = rank else {
            return Ok(None);
        };
        if self.message(chat, id)?.is_none() {
            return Ok(None);
        }
        let furthest = status_from_rank(rank);
        let now = crate::util::now();
        let mut changed = None;
        for (status, at) in [
            (Delivery::Delivered, delivered_at),
            (Delivery::Read, read_at),
            (Delivery::Played, played_at),
        ] {
            let at = at.unwrap_or(now);
            if furthest >= status && self.set_status(chat, id, status, at)? {
                changed = Some((status, at));
            }
        }
        transaction.execute(
            "DELETE FROM group_receipts WHERE chat = ?1 AND id = ?2",
            params![chat, id],
        )?;
        transaction.commit()?;
        Ok(changed)
    }

    /// Ids of our messages in `chat` with receipts waiting to be applied.
    pub fn waiting_receipts(&self, chat: &str) -> Result<Vec<String>> {
        let mut statement = self.connection.prepare(
            "SELECT DISTINCT r.id FROM group_receipts r
             JOIN messages m ON m.chat = r.chat AND m.id = r.id AND m.from_me = 1
             WHERE r.chat = ?1",
        )?;
        statement
            .query_map(params![chat], |row| row.get(0))?
            .collect()
    }

    /// Every receipt kept for one message, saved audience included.
    pub fn receipts(&self, chat: &str, id: &str) -> Result<Vec<Recipient>> {
        let mut statement = self.connection.prepare(
            "SELECT recipient, expected, delivered_at, read_at, played_at
             FROM group_receipts WHERE chat = ?1 AND id = ?2 ORDER BY recipient",
        )?;
        statement
            .query_map(params![chat, id], |row| {
                Ok(Recipient {
                    id: row.get(0)?,
                    expected: row.get(1)?,
                    delivered_at: row.get(2)?,
                    read_at: row.get(3)?,
                    played_at: row.get(4)?,
                })
            })?
            .collect()
    }

    /// Drops receipts whose message has not arrived within a day. It is not
    /// coming: receipts also name our reactions, edits, and votes, which are
    /// never archived as messages, and messages deleted here.
    pub fn prune_waiting_receipts(&self) -> Result<()> {
        Self::prune_receipts(&self.connection)
    }

    pub(super) fn prune_receipts(connection: &rusqlite::Connection) -> Result<()> {
        connection.execute(
            "DELETE FROM group_receipts
             WHERE NOT EXISTS (
                 SELECT 1 FROM messages m WHERE m.chat = group_receipts.chat AND m.id = group_receipts.id
             )
             AND COALESCE(delivered_at, read_at, played_at, 0) < ?1",
            params![crate::util::now() - 24 * 60 * 60],
        )?;
        Ok(())
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
        // A direct chat's waiting receipts are keyed by the chat as well.
        transaction.execute(
            "INSERT OR IGNORE INTO group_receipts (chat, id, recipient, expected, status, delivered_at, read_at, played_at)
             SELECT ?2, id, recipient, expected, status, delivered_at, read_at, played_at
             FROM group_receipts WHERE chat = ?1",
            params![lid, pn],
        )?;
        transaction.execute("DELETE FROM group_receipts WHERE chat = ?1", [lid])?;
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

    #[test]
    fn each_group_recipients_times_survive_reopening() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("archive.db");
        let group = "123-456@g.us";
        let now = crate::util::now();
        {
            let archive = Archive::open_with_key(&path, &[7; 32]).unwrap();
            archive.ensure_chat(group, "Group").unwrap();
            archive
                .insert_message(&super::super::tests::message(group, "m", now, true), None)
                .unwrap();
            archive
                .snapshot_group_recipients(group, "m", &["a@lid".into(), "b@s.whatsapp.net".into()])
                .unwrap();
            archive
                .group_receipt(group, "m", "a@lid", Delivery::Delivered, now + 1)
                .unwrap();
            archive
                .group_receipt(group, "m", "a@lid", Delivery::Read, now + 5)
                .unwrap();
            // A late delivery receipt cannot move the first delivery time.
            archive
                .group_receipt(group, "m", "a@lid", Delivery::Delivered, now + 9)
                .unwrap();
            // Not in the saved audience: joined later, or an unmerged alias.
            archive
                .group_receipt(group, "m", "c@s.whatsapp.net", Delivery::Played, now + 7)
                .unwrap();
        }
        let archive = Archive::open_with_key(&path, &[7; 32]).unwrap();
        let rows = archive.receipts(group, "m").unwrap();
        assert_eq!(
            rows,
            [
                Recipient {
                    id: "a@lid".into(),
                    expected: true,
                    delivered_at: Some(now + 1),
                    read_at: Some(now + 5),
                    played_at: None,
                },
                Recipient {
                    id: "b@s.whatsapp.net".into(),
                    expected: true,
                    delivered_at: None,
                    read_at: None,
                    played_at: None,
                },
                Recipient {
                    id: "c@s.whatsapp.net".into(),
                    expected: false,
                    delivered_at: Some(now + 7),
                    read_at: Some(now + 7),
                    played_at: Some(now + 7),
                },
            ]
        );
        // Learning that a privacy id is a phone number renames its receipts.
        archive.put_lid("a", "a").unwrap();
        let rows = archive.receipts(group, "m").unwrap();
        assert!(
            rows.iter()
                .any(|row| row.id == "a@s.whatsapp.net" && row.expected)
        );
        assert!(!rows.iter().any(|row| row.id == "a@lid"));
        assert!(archive.receipts(group, "other").unwrap().is_empty());
    }

    #[test]
    fn receipts_that_beat_their_message_apply_when_it_arrives() {
        let archive = Archive::in_memory().unwrap();
        let peer = "4917663430455@s.whatsapp.net";
        let group = "123-456@g.us";
        let now = crate::util::now();
        archive.ensure_chat(peer, "Peer").unwrap();
        archive.ensure_chat(group, "Group").unwrap();
        // Sent from the phone: the peer's receipts outran the message itself.
        archive
            .file_receipt(peer, "early", peer, Delivery::Delivered, now)
            .unwrap();
        archive
            .file_receipt(peer, "early", peer, Delivery::Read, now + 3)
            .unwrap();
        archive
            .file_receipt(group, "early", "a@s.whatsapp.net", Delivery::Read, now + 4)
            .unwrap();
        assert_eq!(archive.settle_direct(peer, "early").unwrap(), None);
        let mut row = super::super::tests::message(peer, "early", now - 10, true);
        row.status = Delivery::Sent;
        archive.insert_message(&row, None).unwrap();
        assert_eq!(archive.waiting_receipts(peer).unwrap(), ["early"]);
        assert_eq!(
            archive.settle_direct(peer, "early").unwrap(),
            Some((Delivery::Read, now + 3))
        );
        let stored = archive.message(peer, "early").unwrap().unwrap();
        assert_eq!(stored.status, Delivery::Read);
        assert_eq!(stored.delivered_at, Some(now));
        assert_eq!(stored.read_at, Some(now + 3));
        assert!(archive.waiting_receipts(peer).unwrap().is_empty());

        let mut row = super::super::tests::message(group, "early", now - 10, true);
        row.status = Delivery::Sent;
        archive.insert_message(&row, None).unwrap();
        assert!(!archive.has_group_audience(group, "early").unwrap());
        archive
            .snapshot_group_recipients(group, "early", &["a@s.whatsapp.net".into()])
            .unwrap();
        assert!(archive.has_group_audience(group, "early").unwrap());
        assert!(archive.settle_group(group, "early").unwrap());
        assert_eq!(
            archive.message(group, "early").unwrap().unwrap().status,
            Delivery::Read
        );
    }

    #[test]
    fn waiting_receipts_follow_a_direct_chat_to_its_phone_number() {
        let archive = Archive::in_memory().unwrap();
        let lid = "167650256810092@lid";
        let pn = "4917663430455@s.whatsapp.net";
        archive.ensure_chat(pn, "Peer").unwrap();
        let mut row = super::super::tests::message(pn, "m", 10, true);
        row.status = Delivery::Sent;
        archive.insert_message(&row, None).unwrap();
        archive
            .file_receipt(lid, "m", lid, Delivery::Delivered, 20)
            .unwrap();
        archive.put_lid("167650256810092", "4917663430455").unwrap();
        assert_eq!(archive.waiting_receipts(pn).unwrap(), ["m"]);
        assert_eq!(
            archive.settle_direct(pn, "m").unwrap(),
            Some((Delivery::Delivered, 20))
        );
    }

    #[test]
    fn receipts_for_a_message_that_never_came_are_pruned_after_a_day() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("archive.db");
        let now = crate::util::now();
        {
            let archive = Archive::open_with_key(&path, &[7; 32]).unwrap();
            for (id, at) in [("stale", now - 25 * 60 * 60), ("fresh", now)] {
                archive
                    .file_receipt(
                        "p@s.whatsapp.net",
                        id,
                        "p@s.whatsapp.net",
                        Delivery::Read,
                        at,
                    )
                    .unwrap();
            }
        }
        let archive = Archive::open_with_key(&path, &[7; 32]).unwrap();
        let ids: Vec<String> = archive
            .connection
            .prepare("SELECT id FROM group_receipts")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_>>()
            .unwrap();
        assert_eq!(ids, ["fresh"]);
    }
}
