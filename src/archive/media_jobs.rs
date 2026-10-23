//! Durable, account-local media recovery jobs. Keys remain in message protobufs.

use super::{Archive, Result};
use rusqlite::{OptionalExtension, params};
use std::path::Path;

pub const MEDIA_CDN_READY: i64 = 0;
pub const MEDIA_DOWNLOADED: i64 = 1;
pub const MEDIA_UNAVAILABLE: i64 = 2;
pub const MEDIA_CANCELLED: i64 = 3;
pub const MEDIA_NEEDS_PHONE: i64 = 4;
pub const MEDIA_POLICY_TOO_LARGE: i64 = 5;

const ELIGIBLE: &str = "c.locked=0 AND c.left=0
 AND NOT EXISTS(SELECT 1 FROM message_removals r WHERE r.chat=q.chat AND r.id=q.id)
 AND NOT EXISTS(SELECT 1 FROM pending_message_removals r WHERE r.chat=q.chat AND r.id=q.id)
 AND NOT EXISTS(SELECT 1 FROM chat_removals r WHERE r.chat=q.chat AND m.timestamp<=r.through)";

pub(super) const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS media_jobs (
    chat TEXT NOT NULL,
    id TEXT NOT NULL,
    card INTEGER NOT NULL,
    attempts INTEGER NOT NULL DEFAULT 0,
    next_attempt INTEGER NOT NULL DEFAULT 0,
    state INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (chat, id, card)
);
CREATE INDEX IF NOT EXISTS media_jobs_due ON media_jobs(state, next_attempt);
CREATE TABLE IF NOT EXISTS media_import_state (
 id INTEGER PRIMARY KEY CHECK(id=1), phone_day INTEGER NOT NULL DEFAULT 0,
 phone_requests INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS messages_saved_timestamp ON messages(timestamp);
CREATE TRIGGER IF NOT EXISTS delete_media_jobs AFTER DELETE ON messages BEGIN
    DELETE FROM media_jobs WHERE chat=OLD.chat AND id=OLD.id;
END;
";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MediaJob {
    pub chat: String,
    pub id: String,
    /// -1 is the primary file, -2 the motion clip, others carousel indices.
    pub card: i64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MediaJobCounts {
    pub cdn_pending: u64,
    pub needs_phone: u64,
    pub downloaded: u64,
    pub unavailable: u64,
    pub cancelled: u64,
    pub policy_too_large: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MediaJobSchedule {
    pub cdn_due: u64,
    pub phone_due: u64,
    pub retry_at: Option<i64>,
}

fn enqueue_sql(source: &str, predicate: &str) -> String {
    format!(
        "INSERT INTO media_jobs(chat,id,card)
         SELECT m.chat,m.id,-1 FROM {source} m
         WHERE {predicate} AND m.raw IS NOT NULL AND json_valid(m.content)
          AND ((json_extract(m.content,'$.kind') IN ('image','video','audio','document','sticker')
                 AND json_extract(m.content,'$.media.path') IS NULL)
            OR (json_extract(m.content,'$.kind')='interactive'
                 AND json_type(m.content,'$.card.image')='object'
                 AND json_extract(m.content,'$.card.image.path') IS NULL)) ON CONFLICT(chat,id,card) DO UPDATE SET state=CASE WHEN media_jobs.state=1 THEN 0 ELSE media_jobs.state END;
         INSERT INTO media_jobs(chat,id,card)
         SELECT m.chat,m.id,CAST(c.key AS INTEGER) FROM {source} m,
          json_each(CASE WHEN json_valid(m.content) THEN
             CASE WHEN json_type(m.content,'$.card.carousel')='array'
               THEN json_extract(m.content,'$.card.carousel') ELSE '[]' END
             ELSE '[]' END) c
         WHERE {predicate} AND m.raw IS NOT NULL AND json_valid(c.value)
           AND json_type(c.value,'$.image')='object'
           AND json_extract(c.value,'$.image.path') IS NULL ON CONFLICT(chat,id,card) DO UPDATE SET state=CASE WHEN media_jobs.state=1 THEN 0 ELSE media_jobs.state END;
         INSERT INTO media_jobs(chat,id,card)
         SELECT m.chat,m.id,-2 FROM {source} m
         WHERE {predicate} AND json_valid(m.content)
           AND json_extract(m.content,'$.kind')='image'
           AND json_type(m.content,'$.motion')='object'
           AND json_extract(m.content,'$.motion.path') IS NULL ON CONFLICT(chat,id,card) DO UPDATE SET state=CASE WHEN media_jobs.state=1 THEN 0 ELSE media_jobs.state END;"
    )
}

pub(super) fn install(connection: &rusqlite::Connection) -> Result<()> {
    connection.execute_batch(SCHEMA)?;
    // Install after the generated-column migration, which can rebuild messages.
    for operation in ["INSERT", "UPDATE OF content, raw"] {
        let name = if operation == "INSERT" {
            "insert"
        } else {
            "update"
        };
        connection.execute_batch(&format!(
            "CREATE TRIGGER IF NOT EXISTS enqueue_media_{name} AFTER {operation} ON messages BEGIN {} END;",
            enqueue_sql("messages", "m.chat=NEW.chat AND m.id=NEW.id")
        ))?;
    }
    // Retry only newly usable keys, never a repeated history snapshot or an
    // explicit cancellation. The parent sender owns the motion identity.
    connection.execute_batch("CREATE TRIGGER IF NOT EXISTS retry_motion_identity_insert AFTER INSERT ON motion_clips
     WHEN NEW.child_id IS NOT NULL AND NEW.child_id<>'' BEGIN
      UPDATE media_jobs SET state=0,attempts=0,next_attempt=0 WHERE state IN (2,4) AND card=-2 AND chat=NEW.chat AND id=NEW.parent
       AND EXISTS(SELECT 1 FROM messages m WHERE m.chat=NEW.chat AND m.id=NEW.parent AND m.sender=NEW.sender AND json_extract(m.content,'$.motion.path') IS NULL);
     END;
     CREATE TRIGGER IF NOT EXISTS retry_motion_identity_update AFTER UPDATE OF child_id,raw ON motion_clips
     WHEN NEW.child_id IS NOT NULL AND NEW.child_id<>'' AND (NEW.child_id IS NOT OLD.child_id OR NEW.raw IS NOT OLD.raw) BEGIN
      UPDATE media_jobs SET state=0,attempts=0,next_attempt=0 WHERE state IN (2,4) AND card=-2 AND chat=NEW.chat AND id=NEW.parent
       AND EXISTS(SELECT 1 FROM messages m WHERE m.chat=NEW.chat AND m.id=NEW.parent AND m.sender=NEW.sender AND json_extract(m.content,'$.motion.path') IS NULL);
     END;
     CREATE TRIGGER IF NOT EXISTS retry_changed_media_keys AFTER UPDATE OF raw ON messages
     WHEN NEW.raw IS NOT NULL AND NEW.raw IS NOT OLD.raw BEGIN
      UPDATE media_jobs SET state=0,attempts=0,next_attempt=0 WHERE state IN (2,4) AND chat=NEW.chat AND id=NEW.id AND card<>-2;
     END;")?;
    Ok(())
}

impl Archive {
    /// At most 32 completed jobs per tick. Cursor and the five-minute round
    /// deadline survive restart; paused jobs are never inspected or revived.
    pub fn completed_media_audit_batch(&self, now: i64) -> Result<Vec<(i64, MediaJob)>> {
        let due = self
            .meta("media_jobs_audit_due")?
            .and_then(|value| value.parse::<i64>().ok())
            .unwrap_or(0);
        if now < due {
            return Ok(Vec::new());
        }
        let cursor = self
            .meta("media_jobs_audit_rowid")?
            .and_then(|value| value.parse::<i64>().ok())
            .unwrap_or(0);
        let rows: Vec<_> = self
            .connection
            .prepare(&format!(
                "SELECT q.rowid,q.chat,q.id,q.card FROM media_jobs q
         JOIN messages m ON m.chat=q.chat AND m.id=q.id JOIN chats c ON c.id=q.chat
         WHERE q.state=1 AND q.rowid>?1 AND {ELIGIBLE} ORDER BY q.rowid LIMIT 32"
            ))?
            .query_map([cursor], |row| {
                Ok((
                    row.get(0)?,
                    MediaJob {
                        chat: row.get(1)?,
                        id: row.get(2)?,
                        card: row.get(3)?,
                    },
                ))
            })?
            .collect::<Result<_>>()?;
        if rows.is_empty() {
            let transaction = self.connection.unchecked_transaction()?;
            self.set_meta("media_jobs_audit_rowid", "0")?;
            self.set_meta("media_jobs_audit_due", &now.saturating_add(300).to_string())?;
            transaction.commit()?;
        }
        Ok(rows)
    }

    pub fn advance_completed_media_audit(&self, rowid: i64) -> Result<()> {
        self.set_meta("media_jobs_audit_rowid", &rowid.to_string())
    }

    /// Revalidate the snapshot and privacy barriers atomically before clearing
    /// a missing path. The worker confirms NotFound only inside managed storage.
    pub fn requeue_missing_media(&self, job: &MediaJob, expected: Option<&Path>) -> Result<bool> {
        let transaction = self.connection.unchecked_transaction()?;
        let eligible: bool = transaction.query_row(
            &format!(
                "SELECT EXISTS(SELECT 1 FROM media_jobs q
         JOIN messages m ON m.chat=q.chat AND m.id=q.id JOIN chats c ON c.id=q.chat
         WHERE q.state=1 AND q.chat=?1 AND q.id=?2 AND q.card=?3 AND {ELIGIBLE})"
            ),
            params![job.chat, job.id, job.card],
            |row| row.get(0),
        )?;
        if !eligible {
            return Ok(false);
        }
        let Some(mut message) = self.message(&job.chat, &job.id)? else {
            return Ok(false);
        };
        if job.card == -2 {
            let crate::model::Content::Image {
                motion: Some(motion),
                ..
            } = &mut message.content
            else {
                return Ok(false);
            };
            if motion.path.as_deref() != expected {
                return Ok(false);
            }
            motion.path = None;
        } else {
            let Some(media) = message
                .content
                .media_at_mut((job.card >= 0).then_some(job.card as usize))
            else {
                return Ok(false);
            };
            if media.path.as_deref() != expected {
                return Ok(false);
            }
            media.path = None;
        }
        self.set_content(&job.chat, &job.id, &message.content, message.edited)?;
        transaction.execute("UPDATE media_jobs SET state=0,attempts=0,next_attempt=0 WHERE chat=?1 AND id=?2 AND card=?3",params![job.chat,job.id,job.card])?;
        transaction.commit()?;
        Ok(true)
    }
    #[cfg(test)]
    pub(crate) fn stage_media_job_fixture(&self, job: &MediaJob) {
        self.connection
            .execute(
                "INSERT INTO media_jobs(chat,id,card) VALUES (?1,?2,?3)",
                params![job.chat, job.id, job.card],
            )
            .unwrap();
    }

    pub fn pause_media_job(&self, job: &MediaJob) -> Result<()> {
        self.connection.execute(
            "UPDATE media_jobs SET state=?4 WHERE chat=?1 AND id=?2 AND card=?3",
            params![job.chat, job.id, job.card, MEDIA_CANCELLED],
        )?;
        Ok(())
    }

    pub fn mark_media_job_policy_too_large(&self, job: &MediaJob) -> Result<()> {
        self.connection.execute(
            "UPDATE media_jobs SET state=?4 WHERE chat=?1 AND id=?2 AND card=?3",
            params![job.chat, job.id, job.card, MEDIA_POLICY_TOO_LARGE],
        )?;
        Ok(())
    }

    /// Re-admit files excluded by the default automatic size limit after the
    /// user explicitly enables large-file import. Safe to call on every
    /// startup while the opt-in remains enabled.
    pub fn resume_policy_too_large_media_jobs(&self) -> Result<u64> {
        let changed = self.connection.execute(
            "UPDATE media_jobs SET state=?1,attempts=0,next_attempt=0 WHERE state=?2",
            params![MEDIA_CDN_READY, MEDIA_POLICY_TOO_LARGE],
        )?;
        Ok(changed.min(u64::MAX as usize) as u64)
    }

    /// The CDN copy expired. Retain the job without scheduling requests that
    /// wake the phone. An explicit download can still complete it normally.
    pub fn defer_media_job_to_phone(&self, job: &MediaJob) -> Result<()> {
        self.connection.execute(
            "UPDATE media_jobs SET state=?4 WHERE chat=?1 AND id=?2 AND card=?3",
            params![job.chat, job.id, job.card, MEDIA_NEEDS_PHONE],
        )?;
        Ok(())
    }

    pub fn paused_media_job_count(&self) -> Result<u64> {
        self.connection
            .query_row("SELECT COUNT(*) FROM media_jobs WHERE state=3", [], |row| {
                row.get::<_, i64>(0).map(|count| count as u64)
            })
    }

    /// Timestamp metadata only; empty archives have no known oldest date.
    pub fn oldest_saved_timestamp(&self) -> Result<Option<i64>> {
        self.connection.query_row(
            "SELECT MIN(timestamp) FROM messages WHERE timestamp>0",
            [],
            |row| row.get(0),
        )
    }

    /// Scan old archives in bounded batches; live/history writes enqueue through
    /// triggers, avoiding a full archive scan on every scheduler tick.
    pub fn seed_media_jobs(&self) -> Result<()> {
        let cursor = self
            .meta("media_jobs_scan_rowid")?
            .and_then(|value| value.parse::<i64>().ok())
            .unwrap_or(0);
        let next: Option<i64> = self.connection.query_row(
            "SELECT MAX(rowid) FROM (SELECT rowid FROM messages WHERE rowid>?1 ORDER BY rowid LIMIT 200)",
            params![cursor], |row| row.get(0),
        )?;
        let Some(next) = next else {
            return Ok(());
        };
        let transaction = self.connection.unchecked_transaction()?;
        transaction.execute_batch(&enqueue_sql(
            "messages",
            &format!("m.rowid>{cursor} AND m.rowid<={next}"),
        ))?;
        transaction.execute("INSERT INTO meta(key,value) VALUES ('media_jobs_scan_rowid',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![next.to_string()])?;
        transaction.commit()
    }

    pub fn due_media_jobs(&self, now: i64, limit: usize) -> Result<Vec<MediaJob>> {
        self.due_media_jobs_since(now, i64::MIN, limit)
    }

    pub fn due_media_jobs_since(
        &self,
        now: i64,
        target_since: i64,
        limit: usize,
    ) -> Result<Vec<MediaJob>> {
        self.connection.prepare(
            "SELECT q.chat,q.id,q.card FROM media_jobs q JOIN messages m ON m.chat=q.chat AND m.id=q.id
             JOIN chats c ON c.id=q.chat
             WHERE q.state=0 AND q.next_attempt<=?1 AND m.timestamp>=?2 AND c.locked=0 AND c.left=0
              AND NOT EXISTS(SELECT 1 FROM message_removals r WHERE r.chat=q.chat AND r.id=q.id)
              AND NOT EXISTS(SELECT 1 FROM pending_message_removals r WHERE r.chat=q.chat AND r.id=q.id)
              AND NOT EXISTS(SELECT 1 FROM chat_removals r WHERE r.chat=q.chat AND m.timestamp<=r.through)
             ORDER BY m.timestamp DESC,q.next_attempt,q.chat,q.id,q.card LIMIT ?3"
        )?.query_map(params![now,target_since,limit as i64], |row| Ok(MediaJob{chat:row.get(0)?,id:row.get(1)?,card:row.get(2)?}))?.collect()
    }

    /// Lease before starting. A crash retries later; no permanent in-flight bit.
    pub fn start_media_job(&self, job: &MediaJob, now: i64) -> Result<()> {
        self.connection.execute("UPDATE media_jobs SET attempts=attempts+1,next_attempt=?4 WHERE chat=?1 AND id=?2 AND card=?3 AND state=0", params![job.chat,job.id,job.card,now+120])?;
        Ok(())
    }

    pub fn finish_media_job(
        &self,
        job: &MediaJob,
        now: i64,
        success: bool,
        unavailable: bool,
    ) -> Result<()> {
        self.connection.execute(
            "UPDATE media_jobs SET state=CASE WHEN ?4 THEN 1 WHEN ?5 THEN 2 ELSE 0 END,
             next_attempt=?6+MIN(86400,30*(1 << MIN(attempts,11))) WHERE chat=?1 AND id=?2 AND card=?3",
            params![job.chat,job.id,job.card,success,unavailable,now])?;
        Ok(())
    }

    /// Counts only, suitable for progress without reading private message text.
    pub fn media_job_counts(&self) -> Result<(u64, u64, u64)> {
        self.connection.query_row("SELECT COALESCE(SUM(state IN (0,4)),0),COALESCE(SUM(state=1),0),COALESCE(SUM(state=2),0) FROM media_jobs",[],|row|Ok((row.get::<_,i64>(0)? as u64,row.get::<_,i64>(1)? as u64,row.get::<_,i64>(2)? as u64)))
    }

    pub fn media_import_counts(&self, target_since: i64) -> Result<MediaJobCounts> {
        self.connection.query_row(
            "SELECT COALESCE(SUM(q.state=?2),0),COALESCE(SUM(q.state=?3),0),
             COALESCE(SUM(q.state=?4),0),COALESCE(SUM(q.state=?5),0),
             COALESCE(SUM(q.state=?6),0),COALESCE(SUM(q.state=?7),0)
             FROM media_jobs q JOIN messages m ON m.chat=q.chat AND m.id=q.id
             WHERE m.timestamp>=?1",
            params![
                target_since,
                MEDIA_CDN_READY,
                MEDIA_NEEDS_PHONE,
                MEDIA_DOWNLOADED,
                MEDIA_UNAVAILABLE,
                MEDIA_CANCELLED,
                MEDIA_POLICY_TOO_LARGE
            ],
            |row| {
                Ok(MediaJobCounts {
                    cdn_pending: row.get::<_, i64>(0)? as u64,
                    needs_phone: row.get::<_, i64>(1)? as u64,
                    downloaded: row.get::<_, i64>(2)? as u64,
                    unavailable: row.get::<_, i64>(3)? as u64,
                    cancelled: row.get::<_, i64>(4)? as u64,
                    policy_too_large: row.get::<_, i64>(5)? as u64,
                })
            },
        )
    }

    /// Aggregate scheduling state only. No chat or message identifier leaves the archive.
    pub fn media_import_schedule(&self, target_since: i64, now: i64) -> Result<MediaJobSchedule> {
        self.connection.query_row(
            &format!(
                "SELECT COALESCE(SUM(q.state=?3 AND q.next_attempt<=?2 AND {ELIGIBLE}),0),
                 COALESCE(SUM(q.state=?4 AND q.next_attempt<=?2 AND {ELIGIBLE}),0),
                 MIN(CASE WHEN q.state IN (?3,?4) AND q.next_attempt>?2 AND {ELIGIBLE}
                          THEN q.next_attempt END)
                 FROM media_jobs q JOIN messages m ON m.chat=q.chat AND m.id=q.id
                 JOIN chats c ON c.id=q.chat WHERE m.timestamp>=?1"
            ),
            params![target_since, now, MEDIA_CDN_READY, MEDIA_NEEDS_PHONE],
            |row| {
                Ok(MediaJobSchedule {
                    cdn_due: row.get::<_, i64>(0)? as u64,
                    phone_due: row.get::<_, i64>(1)? as u64,
                    retry_at: row.get(2)?,
                })
            },
        )
    }

    pub fn due_phone_media_jobs(
        &self,
        now: i64,
        target_since: i64,
        limit: usize,
    ) -> Result<Vec<MediaJob>> {
        self.connection.prepare(
            "SELECT q.chat,q.id,q.card FROM media_jobs q
             JOIN messages m ON m.chat=q.chat AND m.id=q.id JOIN chats c ON c.id=q.chat
             WHERE q.state=?1 AND q.next_attempt<=?2 AND m.timestamp>=?3 AND c.locked=0 AND c.left=0
              AND NOT EXISTS(SELECT 1 FROM message_removals r WHERE r.chat=q.chat AND r.id=q.id)
              AND NOT EXISTS(SELECT 1 FROM pending_message_removals r WHERE r.chat=q.chat AND r.id=q.id)
              AND NOT EXISTS(SELECT 1 FROM chat_removals r WHERE r.chat=q.chat AND m.timestamp<=r.through)
             ORDER BY m.timestamp DESC,q.next_attempt,q.chat,q.id,q.card LIMIT ?4"
        )?.query_map(params![MEDIA_NEEDS_PHONE,now,target_since,limit as i64], |row| Ok(MediaJob {
            chat:row.get(0)?,id:row.get(1)?,card:row.get(2)?
        }))?.collect()
    }

    /// Reserve only the media-specific daily quota. The scheduler must reserve
    /// the shared account phone gate immediately before sending the reupload.
    pub fn media_phone_quota_available(&self, now: i64, requests_per_day: u32) -> Result<bool> {
        let day = now.div_euclid(86_400);
        let used: Option<(i64, i64)> = self
            .connection
            .query_row(
                "SELECT phone_day,phone_requests FROM media_import_state WHERE id=1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        Ok(used.is_none_or(|(saved_day, requests)| {
            saved_day != day || requests < i64::from(requests_per_day.max(1))
        }))
    }

    pub fn media_job_needs_phone(&self, job: &MediaJob) -> Result<bool> {
        self.connection.query_row(
            "SELECT state=?4 FROM media_jobs WHERE chat=?1 AND id=?2 AND card=?3",
            params![job.chat, job.id, job.card, MEDIA_NEEDS_PHONE],
            |row| row.get(0),
        )
    }

    pub fn reserve_phone_media_job(
        &self,
        job: &MediaJob,
        now: i64,
        requests_per_day: u32,
    ) -> Result<bool> {
        let transaction = self.connection.unchecked_transaction()?;
        let day = now.div_euclid(86_400);
        transaction.execute(
            "INSERT INTO media_import_state(id,phone_day,phone_requests) VALUES(1,?1,0)
             ON CONFLICT(id) DO UPDATE SET phone_day=?1,phone_requests=CASE WHEN phone_day=?1 THEN phone_requests ELSE 0 END",
            [day],
        )?;
        let requests: i64 = transaction.query_row(
            "SELECT phone_requests FROM media_import_state WHERE id=1",
            [],
            |row| row.get(0),
        )?;
        if requests >= i64::from(requests_per_day.max(1)) {
            return Ok(false);
        }
        let reserved=transaction.execute(
            "UPDATE media_jobs SET attempts=attempts+1,next_attempt=?4 WHERE chat=?1 AND id=?2 AND card=?3 AND state=?5 AND next_attempt<=?6",
            params![job.chat,job.id,job.card,now.saturating_add(120),MEDIA_NEEDS_PHONE,now],
        )? > 0;
        if reserved {
            transaction.execute(
                "UPDATE media_import_state SET phone_requests=phone_requests+1 WHERE id=1",
                [],
            )?;
        }
        transaction.commit()?;
        Ok(reserved)
    }

    pub fn finish_phone_media_job(
        &self,
        job: &MediaJob,
        now: i64,
        recovered: bool,
        unavailable: bool,
    ) -> Result<()> {
        self.connection.execute(
            "UPDATE media_jobs SET state=CASE WHEN ?4 THEN ?6 WHEN ?5 THEN ?7 ELSE ?8 END,
             next_attempt=?9+MIN(3600,60*(1 << MIN(attempts,6)))
             WHERE chat=?1 AND id=?2 AND card=?3 AND state=?8",
            params![
                job.chat,
                job.id,
                job.card,
                recovered,
                unavailable,
                MEDIA_DOWNLOADED,
                MEDIA_UNAVAILABLE,
                MEDIA_NEEDS_PHONE,
                now
            ],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn insert(archive: &Archive, id: &str, content: &str) {
        archive.connection.execute(
            "INSERT INTO messages(chat,id,sender,from_me,timestamp,content,raw) VALUES ('fixture',?1,'fixture',0,1,?2,X'00')",
            params![id,content],
        ).unwrap();
    }

    #[test]
    fn fresh_motion_identity_retries_unavailable_but_not_duplicates_strangers_or_cancellation() {
        let archive = Archive::in_memory().unwrap();
        archive.ensure_chat("fixture", "Synthetic").unwrap();
        insert(
            &archive,
            "photo",
            r#"{"kind":"image","media":{},"motion":{}}"#,
        );
        archive
            .put_motion_clip("fixture", "photo", "fixture", b"clip")
            .unwrap();
        let job = MediaJob {
            chat: "fixture".into(),
            id: "photo".into(),
            card: -2,
        };
        archive.finish_media_job(&job, 1, false, true).unwrap();
        archive
            .put_motion_clip_with_id("fixture", "photo", "stranger", b"clip", Some("stranger-id"))
            .unwrap();
        assert_eq!(archive.media_job_counts().unwrap(), (1, 0, 1));
        archive
            .put_motion_clip_with_id("fixture", "photo", "fixture", b"clip", Some("child"))
            .unwrap();
        assert_eq!(archive.media_job_counts().unwrap(), (2, 0, 0));
        archive.finish_media_job(&job, 1, false, true).unwrap();
        archive
            .put_motion_clip_with_id("fixture", "photo", "fixture", b"clip", Some("child"))
            .unwrap();
        assert_eq!(archive.media_job_counts().unwrap(), (1, 0, 1));
        archive.pause_media_job(&job).unwrap();
        archive
            .put_motion_clip_with_id("fixture", "photo", "fixture", b"fresh", Some("new-child"))
            .unwrap();
        assert_eq!(archive.paused_media_job_count().unwrap(), 1);
        archive.remove_chat_through("fixture", 1, true).unwrap();
        archive
            .put_motion_clip_with_id("fixture", "photo", "fixture", b"late", Some("late-child"))
            .unwrap();
        assert!(archive.due_media_jobs(i64::MAX, 10).unwrap().is_empty());
    }

    #[test]
    fn changed_raw_keys_retry_only_unavailable_jobs() {
        let archive = Archive::in_memory().unwrap();
        archive.ensure_chat("fixture", "Synthetic").unwrap();
        insert(&archive, "photo", r#"{"kind":"image","media":{}}"#);
        let job = MediaJob {
            chat: "fixture".into(),
            id: "photo".into(),
            card: -1,
        };
        archive.finish_media_job(&job, 1, false, true).unwrap();
        archive
            .connection
            .execute("UPDATE messages SET raw=X'00'", [])
            .unwrap();
        assert_eq!(archive.media_job_counts().unwrap(), (0, 0, 1));
        archive
            .connection
            .execute("UPDATE messages SET raw=X'01'", [])
            .unwrap();
        assert_eq!(archive.media_job_counts().unwrap(), (1, 0, 0));
        archive.pause_media_job(&job).unwrap();
        archive
            .connection
            .execute("UPDATE messages SET raw=X'02'", [])
            .unwrap();
        assert_eq!(archive.paused_media_job_count().unwrap(), 1);
    }

    #[test]
    fn completed_audit_is_bounded_resumes_and_requeues_with_privacy_barriers() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("synthetic.db");
        let archive = Archive::prepare(rusqlite::Connection::open(&path).unwrap()).unwrap();
        archive.ensure_chat("fixture", "Synthetic").unwrap();
        for index in 0..33 {
            let id = format!("photo-{index}");
            let content = serde_json::json!({"kind":"image","media":{"mime":"image/jpeg","size":1,"path":"/managed/missing"}}).to_string();
            insert(&archive, &id, &content);
            let job = MediaJob {
                chat: "fixture".into(),
                id,
                card: -1,
            };
            archive.stage_media_job_fixture(&job);
            archive.finish_media_job(&job, 1, true, false).unwrap();
        }
        let first = archive.completed_media_audit_batch(100).unwrap();
        assert_eq!(first.len(), 32);
        let job = &first[0].1;
        assert!(
            !archive
                .requeue_missing_media(job, Some(Path::new("/different")))
                .unwrap()
        );
        archive.set_locked("fixture", true).unwrap();
        assert!(
            !archive
                .requeue_missing_media(job, Some(Path::new("/managed/missing")))
                .unwrap()
        );
        archive.set_locked("fixture", false).unwrap();
        assert!(
            archive
                .requeue_missing_media(job, Some(Path::new("/managed/missing")))
                .unwrap()
        );
        assert!(
            archive
                .message("fixture", &job.id)
                .unwrap()
                .unwrap()
                .content
                .media()
                .unwrap()
                .path
                .is_none()
        );
        archive
            .advance_completed_media_audit(first.last().unwrap().0)
            .unwrap();
        drop(archive);
        let archive = Archive::prepare(rusqlite::Connection::open(&path).unwrap()).unwrap();
        let last = archive.completed_media_audit_batch(100).unwrap();
        assert_eq!(last.len(), 1);
        archive.advance_completed_media_audit(last[0].0).unwrap();
        assert!(archive.completed_media_audit_batch(100).unwrap().is_empty());
        assert!(archive.completed_media_audit_batch(399).unwrap().is_empty());
        assert_eq!(archive.completed_media_audit_batch(400).unwrap().len(), 32);
        archive.remove_chat_through("fixture", 1, true).unwrap();
        assert!(
            !archive
                .requeue_missing_media(&last[0].1, Some(Path::new("/managed/missing")))
                .unwrap()
        );
        assert!(archive.completed_media_audit_batch(700).unwrap().is_empty());
    }

    #[test]
    fn oldest_timestamp_is_unknown_when_empty_and_cancelled_jobs_stay_paused() {
        let archive = Archive::in_memory().unwrap();
        assert_eq!(archive.oldest_saved_timestamp().unwrap(), None);
        archive.ensure_chat("fixture", "Synthetic").unwrap();
        insert(&archive, "image", r#"{"kind":"image","media":{}}"#);
        assert_eq!(archive.oldest_saved_timestamp().unwrap(), Some(1));
        let job = archive.due_media_jobs(1, 1).unwrap().pop().unwrap();
        archive.pause_media_job(&job).unwrap();
        assert_eq!(archive.paused_media_job_count().unwrap(), 1);
        assert!(archive.due_media_jobs(i64::MAX, 10).unwrap().is_empty());
        archive.finish_media_job(&job, 1, true, false).unwrap();
        assert_eq!(archive.media_job_counts().unwrap(), (0, 1, 0));
        assert_eq!(archive.paused_media_job_count().unwrap(), 0);
    }

    #[test]
    fn large_file_opt_in_requeues_only_policy_exclusions() {
        let archive = Archive::in_memory().unwrap();
        archive.ensure_chat("fixture", "Synthetic").unwrap();
        insert(&archive, "large", r#"{"kind":"image","media":{}}"#);
        insert(&archive, "cancelled", r#"{"kind":"image","media":{}}"#);
        let large = MediaJob {
            chat: "fixture".into(),
            id: "large".into(),
            card: -1,
        };
        let cancelled = MediaJob {
            chat: "fixture".into(),
            id: "cancelled".into(),
            card: -1,
        };
        archive.mark_media_job_policy_too_large(&large).unwrap();
        archive.pause_media_job(&cancelled).unwrap();

        assert_eq!(archive.resume_policy_too_large_media_jobs().unwrap(), 1);
        assert_eq!(archive.resume_policy_too_large_media_jobs().unwrap(), 0);
        assert_eq!(archive.paused_media_job_count().unwrap(), 1);
        assert_eq!(archive.due_media_jobs(1, 10).unwrap(), vec![large]);
    }

    #[test]
    fn expired_archive_files_wait_for_a_click_across_restart_and_history_replay() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("synthetic.db");
        let archive = Archive::open_with_key(&database, &[7; 32]).unwrap();
        archive.ensure_chat("fixture", "Synthetic").unwrap();
        insert(&archive, "expired", r#"{"kind":"image","media":{}}"#);
        insert(&archive, "available", r#"{"kind":"image","media":{}}"#);
        let job = MediaJob {
            chat: "fixture".into(),
            id: "expired".into(),
            card: -1,
        };
        archive.start_media_job(&job, 1).unwrap();
        archive.defer_media_job_to_phone(&job).unwrap();
        assert_eq!(archive.media_job_counts().unwrap(), (2, 0, 0));
        assert_eq!(archive.paused_media_job_count().unwrap(), 0);
        drop(archive);

        let archive = Archive::open_with_key(&database, &[7; 32]).unwrap();
        // A duplicate history snapshot and a scan must not wake the phone again.
        archive
            .connection
            .execute("UPDATE messages SET content=content WHERE id='expired'", [])
            .unwrap();
        archive.seed_media_jobs().unwrap();
        let due = archive.due_media_jobs(i64::MAX, 10).unwrap();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].id, "available");
        // A later explicit download still completes the retained job.
        archive.finish_media_job(&job, 2, true, false).unwrap();
        assert_eq!(archive.media_job_counts().unwrap(), (1, 1, 0));
    }

    #[test]
    fn discovers_hidden_media_cards_motion_but_not_view_once() {
        let archive = Archive::in_memory().unwrap();
        archive.ensure_chat("fixture", "Synthetic").unwrap();
        insert(
            &archive,
            "image",
            r#"{"kind":"image","media":{"path":null},"motion":{}}"#,
        );
        insert(
            &archive,
            "carousel",
            r#"{"kind":"interactive","card":{"image":{},"carousel":[{"image":{}},{"image":{"path":"already"}}]}}"#,
        );
        insert(&archive, "private", r#"{"kind":"viewonce"}"#);
        insert(&archive, "text", r#"{"kind":"text"}"#);
        let jobs = archive.due_media_jobs(1, 20).unwrap();
        assert_eq!(jobs.len(), 4);
        assert!(jobs.iter().any(|job| job.id == "carousel" && job.card == 0));
        assert!(jobs.iter().any(|job| job.id == "image" && job.card == -2));
        archive.set_locked("fixture", true).unwrap();
        assert!(archive.due_media_jobs(1, 20).unwrap().is_empty());
    }

    #[test]
    fn leases_resume_with_backoff_and_deletion_removes_jobs() {
        let archive = Archive::in_memory().unwrap();
        archive.ensure_chat("fixture", "Synthetic").unwrap();
        insert(&archive, "image", r#"{"kind":"image","media":{}}"#);
        let job = archive.due_media_jobs(1, 1).unwrap().pop().unwrap();
        archive.start_media_job(&job, 1).unwrap();
        assert_eq!(
            archive.media_import_schedule(i64::MIN, 2).unwrap(),
            MediaJobSchedule {
                cdn_due: 0,
                phone_due: 0,
                retry_at: Some(121),
            }
        );
        assert!(archive.due_media_jobs(2, 1).unwrap().is_empty());
        assert_eq!(archive.due_media_jobs(121, 1).unwrap(), vec![job.clone()]);
        archive.finish_media_job(&job, 121, false, false).unwrap();
        assert!(archive.due_media_jobs(122, 1).unwrap().is_empty());
        archive.finish_media_job(&job, 182, false, true).unwrap();
        assert_eq!(archive.media_job_counts().unwrap(), (0, 0, 1));
        archive
            .connection
            .execute("DELETE FROM messages", [])
            .unwrap();
        assert_eq!(archive.media_job_counts().unwrap(), (0, 0, 0));
    }

    #[test]
    fn selected_range_counts_and_schedules_newest_with_separate_phone_quota() {
        let archive = Archive::in_memory().unwrap();
        archive.ensure_chat("fixture", "Synthetic").unwrap();
        for (id, timestamp) in [("old", 100), ("recent", 900), ("newest", 1000)] {
            archive
                .connection
                .execute(
                    "INSERT INTO messages(chat,id,sender,from_me,timestamp,content,raw)
                 VALUES('fixture',?1,'fixture',0,?2,'{\"kind\":\"image\",\"media\":{}}',X'00')",
                    params![id, timestamp],
                )
                .unwrap();
        }
        let due = archive.due_media_jobs_since(1, 500, 10).unwrap();
        assert_eq!(archive.media_import_schedule(500, 1).unwrap().cdn_due, 2);
        assert_eq!(
            due.iter().map(|job| job.id.as_str()).collect::<Vec<_>>(),
            vec!["newest", "recent"]
        );
        let phone = due[0].clone();
        let second = due[1].clone();
        archive.defer_media_job_to_phone(&phone).unwrap();
        archive.defer_media_job_to_phone(&second).unwrap();
        assert_eq!(archive.media_import_schedule(500, 1).unwrap().phone_due, 2);
        let counts = archive.media_import_counts(500).unwrap();
        assert_eq!((counts.cdn_pending, counts.needs_phone), (0, 2));
        assert_eq!(
            archive.due_phone_media_jobs(1, 500, 10).unwrap(),
            vec![phone.clone(), second.clone()]
        );
        assert!(archive.reserve_phone_media_job(&phone, 1, 1).unwrap());
        assert!(!archive.reserve_phone_media_job(&second, 1, 1).unwrap());
        archive
            .finish_phone_media_job(&phone, 2, true, false)
            .unwrap();
        assert_eq!(archive.media_import_counts(500).unwrap().downloaded, 1);
        assert_eq!(archive.media_import_counts(500).unwrap().needs_phone, 1);
    }

    #[test]
    fn canonical_mapping_keeps_media_jobs_and_exact_motion_identity() {
        let archive = Archive::in_memory().unwrap();
        let lid_chat = "123@lid";
        let phone_chat = "456@s.whatsapp.net";
        archive.ensure_chat(lid_chat, "Synthetic").unwrap();
        archive
            .connection
            .execute(
                "INSERT INTO messages(chat,id,sender,from_me,timestamp,content,raw)
                 VALUES(?1,'photo',?1,0,10,'{\"kind\":\"image\",\"media\":{},\"motion\":{}}',X'00')",
                [lid_chat],
            )
            .unwrap();
        archive
            .put_motion_clip_with_id(lid_chat, "photo", lid_chat, b"clip", Some("child"))
            .unwrap();

        assert!(archive.put_lid("123", "456").unwrap());
        let jobs = archive.due_media_jobs(1, 10).unwrap();
        assert!(jobs.iter().all(|job| job.chat == phone_chat));
        assert!(jobs.iter().any(|job| job.card == -2));
        assert_eq!(
            archive.motion_child_id(phone_chat, "photo").unwrap(),
            Some("child".into())
        );
        assert!(
            archive
                .due_media_jobs(1, 10)
                .unwrap()
                .iter()
                .all(|job| job.chat != lid_chat)
        );
    }
}
