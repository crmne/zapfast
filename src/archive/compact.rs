//! Compact the duplicate raw protocol data while leaving searchable content
//! and message ids untouched. All reads restore the original protobuf bytes.
use super::*;
use crate::smart_storage::{self, Stamp};

pub(super) const SCHEMA: &str =
    "CREATE TABLE IF NOT EXISTS optimized_videos(path TEXT PRIMARY KEY, stamp TEXT NOT NULL);";

pub(super) fn unpack_row(row: &rusqlite::Row<'_>, index: usize) -> Result<Vec<u8>> {
    smart_storage::unpack(row.get(index)?).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            index,
            rusqlite::types::Type::Blob,
            Box::new(error),
        )
    })
}
pub(super) fn optional_raw(row: &rusqlite::Row<'_>, index: usize) -> Result<Option<Vec<u8>>> {
    row.get::<_, Option<Vec<u8>>>(index)?
        .map(smart_storage::unpack)
        .transpose()
        .map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                index,
                rusqlite::types::Type::Blob,
                Box::new(error),
            )
        })
}

pub struct RawRow {
    pub rowid: i64,
    pub raw: Vec<u8>,
}
impl std::fmt::Debug for RawRow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RawRow { .. }")
    }
}

impl Archive {
    pub fn storage_raw_batch(&self, before: i64, after: i64) -> Result<Vec<RawRow>> {
        let mut query=self.connection.prepare("SELECT rowid,raw FROM messages WHERE rowid>?2 AND timestamp>0 AND timestamp<?1 AND length(raw)>=256 AND length(raw)<=1048576 AND substr(raw,1,1)<>x'00' ORDER BY rowid LIMIT 32")?;
        query
            .query_map(params![before, after], |row| {
                Ok(RawRow {
                    rowid: row.get(0)?,
                    raw: row.get(1)?,
                })
            })?
            .collect()
    }
    pub fn store_packed_raw(&self, row: &RawRow, packed: &[u8]) -> Result<bool> {
        self.connection
            .execute(
                "UPDATE messages SET raw=?3 WHERE rowid=?1 AND raw=?2",
                params![row.rowid, row.raw, packed],
            )
            .map(|count| count > 0)
    }
    /// Publish one bounded batch in one transaction; concurrent edits win.
    pub fn store_packed_batch(&self, packed: &[(RawRow, Vec<u8>)]) -> Result<(u64, u64)> {
        let transaction = self.connection.unchecked_transaction()?;
        let mut messages = 0;
        let mut bytes = 0;
        for (row, packed) in packed {
            if self.store_packed_raw(row, packed)? {
                messages += 1;
                bytes += row.raw.len().saturating_sub(packed.len()) as u64;
            }
        }
        if messages > 0 {
            self.set_meta("storage_compaction_due", "1")?;
        }
        transaction.commit()?;
        Ok((messages, bytes))
    }
    pub fn compaction_due(&self) -> Result<bool> {
        Ok(self.meta("storage_compaction_due")?.as_deref() == Some("1"))
    }
    /// The progress hook lets foreground work abort an idle rewrite before
    /// attempting its own writes. SQLite rolls an interrupted VACUUM back.
    pub fn reclaim_storage_cancelable(
        &self,
        cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) -> Result<()> {
        use std::sync::atomic::Ordering;
        if cancel.load(Ordering::Relaxed) {
            return Err(rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_INTERRUPT),
                None,
            ));
        }
        self.connection
            .progress_handler(100, Some(move || cancel.load(Ordering::Relaxed)))?;
        let result = self.reclaim_storage();
        let reset = self.connection.progress_handler(0, None::<fn() -> bool>);
        result?;
        reset?;
        self.set_meta("storage_compaction_due", "0")?;
        Ok(())
    }
    pub fn old_cached_videos(&self, before: i64) -> Result<Vec<PathBuf>> {
        // MAX spans every reference to a path: a video shared again recently
        // must not be reduced just because an older message references it too.
        let mut query=self.connection.prepare("SELECT json_extract(content,'$.media.path') AS path FROM messages WHERE json_valid(content) AND path IS NOT NULL GROUP BY path HAVING MAX(timestamp)<?1 AND MIN(timestamp)>0 AND MIN(json_extract(content,'$.kind')='video' AND COALESCE(json_extract(content,'$.gif'),0)=0)=1")?;
        query
            .query_map(params![before], |row| {
                row.get::<_, String>(0).map(PathBuf::from)
            })?
            .collect()
    }
    /// A new reference may have arrived while a video was being encoded.
    pub fn video_is_old(&self, path: &Path, before: i64) -> Result<bool> {
        self.connection.query_row(
            "SELECT COUNT(*)>0 AND MAX(timestamp)<?2 AND MIN(timestamp)>0 AND MIN(json_extract(content,'$.kind')='video' AND COALESCE(json_extract(content,'$.gif'),0)=0)=1 FROM messages WHERE json_valid(content) AND json_extract(content,'$.media.path')=?1",
            params![path.to_string_lossy(), before], |row| row.get(0),
        )
    }
    pub fn video_optimized(&self, path: &Path, stamp: &Stamp) -> Result<bool> {
        let old: Option<String> = self
            .connection
            .query_row(
                "SELECT stamp FROM optimized_videos WHERE path=?1",
                params![path.to_string_lossy()],
                |row| row.get(0),
            )
            .optional()?;
        Ok(old
            .and_then(|json| serde_json::from_str::<Stamp>(&json).ok())
            .as_ref()
            == Some(stamp))
    }
    pub fn mark_video_optimized(&self, path: &Path, stamp: &Stamp) -> Result<()> {
        let json = serde_json::to_string(stamp)
            .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
        self.connection.execute("INSERT INTO optimized_videos(path,stamp) VALUES(?1,?2) ON CONFLICT(path) DO UPDATE SET stamp=excluded.stamp",params![path.to_string_lossy(),json])?;
        Ok(())
    }
    /// Reclaims freed database pages after a completed optimization. SQLite's
    /// VACUUM is transactional, so an interrupted rewrite retains the archive.
    pub fn reclaim_storage(&self) -> Result<()> {
        self.connection.execute_batch(
            "PRAGMA wal_checkpoint(TRUNCATE); VACUUM; PRAGMA wal_checkpoint(TRUNCATE);",
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn batch_packing_is_resumable_and_canceled_compaction_preserves_rows() {
        use std::sync::{Arc, atomic::AtomicBool};
        let archive = Archive::in_memory().unwrap();
        let chat = "synthetic@s.whatsapp.net";
        archive.ensure_chat(chat, "Synthetic").unwrap();
        let raw = b"original protocol data ".repeat(100);
        for id in ["one", "two"] {
            archive
                .insert_message(
                    &super::super::tests::message(chat, id, 10, false),
                    Some(&raw),
                )
                .unwrap();
        }
        let packed = archive
            .storage_raw_batch(20, 0)
            .unwrap()
            .into_iter()
            .map(|row| {
                let bytes = smart_storage::pack(&row.raw).unwrap().unwrap();
                (row, bytes)
            })
            .collect::<Vec<_>>();
        let (messages, bytes) = archive.store_packed_batch(&packed).unwrap();
        assert_eq!(messages, 2);
        assert!(bytes > 0);
        assert!(archive.compaction_due().unwrap());
        assert!(archive.storage_raw_batch(20, 0).unwrap().is_empty());
        assert!(
            archive
                .reclaim_storage_cancelable(Arc::new(AtomicBool::new(true)))
                .is_err()
        );
        assert!(archive.compaction_due().unwrap());
        assert_eq!(archive.raw(chat, "one").unwrap().unwrap(), raw);
        archive
            .reclaim_storage_cancelable(Arc::new(AtomicBool::new(false)))
            .unwrap();
        assert!(!archive.compaction_due().unwrap());
        assert_eq!(archive.raw(chat, "two").unwrap().unwrap(), raw);
    }

    #[test]
    fn age_cutoff_and_newer_shared_video_references_are_respected() {
        let archive = Archive::in_memory().unwrap();
        let chat = "peer@s.whatsapp.net";
        archive.ensure_chat(chat, "Peer").unwrap();
        let cutoff = crate::util::now() - smart_storage::AGE;
        let raw = b"retained protocol data".repeat(100);
        let path = PathBuf::from("/synthetic/cache/video.mp4");
        let mut old = super::super::tests::message(chat, "old", cutoff - 1, false);
        old.content = Content::Video {
            caption: Some("searchable older caption".into()),
            media: crate::model::Media {
                path: Some(path.clone()),
                mime: "video/mp4".into(),
                size: 1_000_000,
                width: None,
                height: None,
                album: None,
                state: Default::default(),
            },
            seconds: Some(3),
            gif: false,
            note: false,
        };
        archive.insert_message(&old, Some(&raw)).unwrap();
        let mut recent = old.clone();
        recent.id = "recent".into();
        recent.timestamp = cutoff;
        archive.insert_message(&recent, Some(&raw)).unwrap();
        assert_eq!(archive.storage_raw_batch(cutoff, 0).unwrap().len(), 1);
        assert!(archive.old_cached_videos(cutoff).unwrap().is_empty());
        assert!(!archive.video_is_old(&path, cutoff).unwrap());
        archive.delete_message(chat, "recent").unwrap();
        assert_eq!(
            archive.old_cached_videos(cutoff).unwrap(),
            std::slice::from_ref(&path)
        );
        assert!(archive.video_is_old(&path, cutoff).unwrap());
        // A deleted id stays barred, so the video arrives again as a new message.
        recent.id = "shared-again".into();
        archive.insert_message(&recent, Some(&raw)).unwrap();
        assert!(!archive.video_is_old(&path, cutoff).unwrap());
        assert!(
            !archive
                .video_is_old(Path::new("/unreferenced.mp4"), cutoff)
                .unwrap()
        );
    }
    #[test]
    fn old_messages_remain_searchable_and_raw_data_remains_exact() {
        let archive = Archive::in_memory().unwrap();
        let chat = "peer@s.whatsapp.net";
        archive.ensure_chat(chat, "Peer").unwrap();
        let mut row = super::super::tests::message(chat, "old", 10, false);
        row.content = Content::text("needle in an older message");
        let raw = b"protocol data, including the original keys and text. ".repeat(100);
        archive.insert_message(&row, Some(&raw)).unwrap();
        let batch = archive.storage_raw_batch(20, 0).unwrap();
        assert_eq!(batch.len(), 1);
        let packed = smart_storage::pack(&batch[0].raw).unwrap().unwrap();
        assert!(archive.store_packed_raw(&batch[0], &packed).unwrap());
        assert_eq!(archive.raw(chat, "old").unwrap().unwrap(), raw);
        assert_eq!(archive.rows_with_raw().unwrap()[0].2, raw);
        assert_eq!(
            archive
                .search_chat_messages(chat, "needle", None, None, 10)
                .unwrap()
                .len(),
            1
        );
        assert!(archive.storage_raw_batch(20, 0).unwrap().is_empty());
        archive.reclaim_storage().unwrap();
        assert_eq!(archive.raw(chat, "old").unwrap().unwrap(), raw);
    }
    #[test]
    fn a_concurrent_edit_is_not_overwritten_by_packing() {
        let archive = Archive::in_memory().unwrap();
        let chat = "peer@s.whatsapp.net";
        archive.ensure_chat(chat, "Peer").unwrap();
        let row = super::super::tests::message(chat, "old", 10, false);
        let raw = b"old repeated data".repeat(100);
        archive.insert_message(&row, Some(&raw)).unwrap();
        let batch = archive.storage_raw_batch(20, 0).unwrap();
        archive.insert_message(&row, Some(b"new raw data")).unwrap();
        assert!(
            !archive
                .store_packed_raw(&batch[0], &smart_storage::pack(&raw).unwrap().unwrap())
                .unwrap()
        );
        assert_eq!(archive.raw(chat, "old").unwrap().unwrap(), b"new raw data");
    }
}
