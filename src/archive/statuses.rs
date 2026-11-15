//! Expiring Status records in the account's encrypted database, separate from chats.
use super::{Archive, Result};
use rusqlite::params;

pub(super) const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS status_updates (
    sender TEXT NOT NULL, id TEXT NOT NULL, posted_at INTEGER NOT NULL,
    own INTEGER NOT NULL, name TEXT, raw BLOB NOT NULL, media_path TEXT,
    metadata TEXT NOT NULL, PRIMARY KEY(sender,id)
);
CREATE INDEX IF NOT EXISTS status_updates_by_time ON status_updates(posted_at);
CREATE TABLE IF NOT EXISTS status_revocations (
    sender TEXT NOT NULL, id TEXT NOT NULL, expires INTEGER NOT NULL,
    PRIMARY KEY(sender,id)
);
";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusRecord {
    pub sender: String,
    pub id: String,
    pub posted_at: i64,
    pub own: bool,
    pub name: Option<String>,
    pub raw: Vec<u8>,
    pub media_path: Option<String>,
    pub metadata: String,
}

impl Archive {
    pub fn save_statuses(&self, records: &[StatusRecord], now: i64) -> Result<()> {
        let transaction = self.connection.unchecked_transaction()?;
        {
            let mut insert=transaction.prepare_cached("INSERT INTO status_updates(sender,id,posted_at,own,name,raw,media_path,metadata)
                SELECT ?1,?2,?3,?4,?5,?6,?7,?8 WHERE NOT EXISTS
                (SELECT 1 FROM status_revocations WHERE sender=?1 AND id=?2 AND expires>?9)
                ON CONFLICT(sender,id) DO UPDATE SET posted_at=excluded.posted_at,own=excluded.own,
                name=excluded.name,raw=excluded.raw,media_path=excluded.media_path,metadata=excluded.metadata")?;
            for record in records {
                if record.posted_at > now
                    || now.saturating_sub(record.posted_at) >= crate::status::LIFETIME
                    || record.raw.len() > crate::status::RAW_LIMIT
                    || record.metadata.len() > crate::status::METADATA_LIMIT
                {
                    continue;
                }
                insert.execute(params![
                    record.sender,
                    record.id,
                    record.posted_at,
                    record.own,
                    record.name,
                    record.raw,
                    record.media_path,
                    record.metadata,
                    now
                ])?;
            }
        }
        transaction.execute(
            "DELETE FROM status_updates WHERE posted_at<=?1 OR posted_at>?2",
            params![now.saturating_sub(crate::status::LIFETIME), now],
        )?;
        transaction.execute("DELETE FROM status_updates WHERE rowid NOT IN (SELECT rowid FROM status_updates ORDER BY own DESC,posted_at DESC,sender,id LIMIT ?1)",params![crate::status::LIMIT as i64])?;
        transaction.execute("DELETE FROM status_revocations WHERE expires<=?1", [now])?;
        transaction.commit()
    }

    pub fn saved_statuses(&self, now: i64) -> Result<Vec<StatusRecord>> {
        let mut query=self.connection.prepare("SELECT sender,id,posted_at,own,name,raw,media_path,metadata FROM status_updates
            WHERE posted_at>?1 AND posted_at<=?2 AND NOT EXISTS (SELECT 1 FROM status_revocations r WHERE r.sender=status_updates.sender AND r.id=status_updates.id AND r.expires>?2)
            ORDER BY own DESC,posted_at DESC,sender,id LIMIT ?3")?;
        query
            .query_map(
                params![
                    now.saturating_sub(crate::status::LIFETIME),
                    now,
                    crate::status::LIMIT as i64
                ],
                |row| {
                    Ok(StatusRecord {
                        sender: row.get(0)?,
                        id: row.get(1)?,
                        posted_at: row.get(2)?,
                        own: row.get(3)?,
                        name: row.get(4)?,
                        raw: row.get(5)?,
                        media_path: row.get(6)?,
                        metadata: row.get(7)?,
                    })
                },
            )?
            .collect()
    }

    pub fn revoke_status(&self, sender: &str, id: &str, expires: i64) -> Result<()> {
        let transaction = self.connection.unchecked_transaction()?;
        transaction.execute("INSERT INTO status_revocations(sender,id,expires) VALUES (?1,?2,?3) ON CONFLICT(sender,id) DO UPDATE SET expires=MAX(expires,excluded.expires)",params![sender,id,expires])?;
        transaction.execute(
            "DELETE FROM status_updates WHERE sender=?1 AND id=?2",
            params![sender, id],
        )?;
        transaction.execute("DELETE FROM status_revocations WHERE rowid NOT IN (SELECT rowid FROM status_revocations ORDER BY expires DESC LIMIT ?1)",params![crate::status::REVOKE_LIMIT as i64])?;
        transaction.commit()
    }

    pub fn status_revocations(&self, now: i64) -> Result<Vec<(String, String, i64)>> {
        self.connection.prepare("SELECT sender,id,expires FROM status_revocations WHERE expires>?1 ORDER BY expires DESC LIMIT ?2")?
            .query_map(params![now,crate::status::REVOKE_LIMIT as i64],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?)))?.collect()
    }

    pub fn remap_status_identity(&self, from: &str, to: &str) -> Result<()> {
        if from == to {
            return Ok(());
        }
        let transaction = self.connection.unchecked_transaction()?;
        transaction.execute(
            "UPDATE OR IGNORE status_updates SET sender=?2 WHERE sender=?1",
            params![from, to],
        )?;
        transaction.execute("DELETE FROM status_updates WHERE sender=?1", [from])?;
        transaction.execute("INSERT INTO status_revocations(sender,id,expires) SELECT ?2,id,expires FROM status_revocations WHERE sender=?1
            ON CONFLICT(sender,id) DO UPDATE SET expires=MAX(expires,excluded.expires)",params![from,to])?;
        transaction.execute("DELETE FROM status_revocations WHERE sender=?1", [from])?;
        transaction.execute("DELETE FROM status_updates WHERE EXISTS (SELECT 1 FROM status_revocations r WHERE r.sender=status_updates.sender AND r.id=status_updates.id)",[])?;
        transaction.commit()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn record(at: i64) -> StatusRecord {
        StatusRecord {
            sender: "15550002222@s.whatsapp.net".into(),
            id: "post".into(),
            posted_at: at,
            own: false,
            name: Some("Ada".into()),
            raw: b"fixture".to_vec(),
            media_path: None,
            metadata: "{}".into(),
        }
    }
    #[test]
    fn status_survives_encrypted_reopening_but_expires_and_never_becomes_a_chat() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("archive.db");
        let key = [17; 32];
        let row = record(1000);
        {
            let archive = Archive::open_with_key(&path, &key).unwrap();
            archive
                .save_statuses(std::slice::from_ref(&row), 1000)
                .unwrap();
        }
        let archive = Archive::open_with_key(&path, &key).unwrap();
        assert_eq!(archive.saved_statuses(1001).unwrap(), vec![row.clone()]);
        assert!(archive.chat(&row.sender).unwrap().is_none());
        assert!(
            archive
                .message("status@broadcast", &row.id)
                .unwrap()
                .is_none()
        );
        archive
            .save_statuses(&[], 1000 + crate::status::LIFETIME)
            .unwrap();
        assert!(
            archive
                .saved_statuses(1000 + crate::status::LIFETIME)
                .unwrap()
                .is_empty()
        );
    }
    #[test]
    fn revocations_survive_identity_mapping_and_replay_and_unlink_clears_them() {
        let archive = Archive::in_memory().unwrap();
        let mut row = record(1000);
        row.sender = "123456789@lid".into();
        archive
            .save_statuses(std::slice::from_ref(&row), 1000)
            .unwrap();
        archive
            .revoke_status(&row.sender, &row.id, 1000 + crate::status::LIFETIME)
            .unwrap();
        archive
            .remap_status_identity(&row.sender, "15550002222@s.whatsapp.net")
            .unwrap();
        row.sender = "15550002222@s.whatsapp.net".into();
        archive.save_statuses(&[row], 1001).unwrap();
        assert!(archive.saved_statuses(1001).unwrap().is_empty());
        assert_eq!(archive.status_revocations(1001).unwrap().len(), 1);
        archive.clear().unwrap();
        assert!(archive.status_revocations(1001).unwrap().is_empty());
    }
    #[test]
    fn retention_keeps_own_posts_and_identity_noop_keeps_rows() {
        let archive = Archive::in_memory().unwrap();
        let mut own = record(1000);
        own.own = true;
        own.sender = "15550009999@s.whatsapp.net".into();
        let mut rows = vec![own.clone()];
        for index in 0..crate::status::LIMIT + 2 {
            let mut peer = record(1001);
            peer.id = format!("peer-{index}");
            rows.push(peer);
        }
        archive.save_statuses(&rows, 1001).unwrap();
        archive
            .remap_status_identity(&own.sender, &own.sender)
            .unwrap();
        let kept = archive.saved_statuses(1001).unwrap();
        assert_eq!(kept.len(), crate::status::LIMIT);
        assert!(
            kept.contains(&own),
            "newer contacts cannot evict our own post"
        );
    }

    #[test]
    fn invalid_times_and_oversized_records_are_not_retained_and_accounts_are_isolated() {
        let archive = Archive::in_memory().unwrap();
        let other = Archive::in_memory().unwrap();
        let good = record(1000);
        let mut huge = good.clone();
        huge.id = "huge".into();
        huge.raw = vec![0; crate::status::RAW_LIMIT + 1];
        archive
            .save_statuses(&[good.clone(), record(1001), huge], 1000)
            .unwrap();
        assert_eq!(archive.saved_statuses(1000).unwrap(), vec![good]);
        assert!(other.saved_statuses(1000).unwrap().is_empty());
    }
}
