//! Durable, correlated traversal of phone history for one account.

use super::{Archive, Result};
use crate::model::{ChatId, ChatImportReason, ImportRunState};
use rusqlite::{OptionalExtension, Transaction, params};

const JOB_RUNNING: i64 = 0;
const JOB_PAUSED: i64 = 1;
const JOB_FINISHED: i64 = 2;
const JOB_NEEDS_ATTENTION: i64 = 3;

const CHAT_ACTIVE: i64 = 0;
const CHAT_REACHED_CUTOFF: i64 = 1;
const CHAT_PHONE_EXHAUSTED: i64 = 2;
const CHAT_EXCLUDED: i64 = 3;
const CHAT_STOPPED: i64 = 4;

const MAX_CHAT_SILENT_FAILURES: i64 = 3;

const REASON_QUEUED: i64 = 0;
const REASON_WAITING_FOR_PHONE: i64 = 1;
const REASON_ADVANCING: i64 = 2;
const REASON_RETRY_SEND: i64 = 3;
const REASON_RETRY_TIMEOUT: i64 = 4;
const REASON_RETRY_NO_PROGRESS: i64 = 5;
const REASON_RETRY_PARSE: i64 = 6;
const REASON_RETRY_STORAGE: i64 = 7;
const REASON_WAITING_FOR_ANCHOR: i64 = 8;
const REASON_REACHED_CUTOFF: i64 = 9;
const REASON_PHONE_EXHAUSTED: i64 = 10;
const REASON_EXCLUDED_LOCKED: i64 = 11;
const REASON_EXCLUDED_LEFT: i64 = 12;
const REASON_EXCLUDED_UNSUPPORTED: i64 = 13;
const REASON_CORRELATION_UNAVAILABLE: i64 = 14;
const REASON_RETRY_LIMIT_REACHED: i64 = 15;

pub(super) const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS history_import_job(
 id INTEGER PRIMARY KEY CHECK(id=1), target_since INTEGER NOT NULL,
 state INTEGER NOT NULL, reason INTEGER, discovery_generation INTEGER NOT NULL DEFAULT 0,
 round INTEGER NOT NULL DEFAULT 0, cursor INTEGER NOT NULL DEFAULT 0,
 day INTEGER NOT NULL DEFAULT 0, day_requests INTEGER NOT NULL DEFAULT 0,
 next_request INTEGER NOT NULL DEFAULT 0, failure_streak INTEGER NOT NULL DEFAULT 0,
 retry_at INTEGER NOT NULL DEFAULT 0, request_token INTEGER NOT NULL DEFAULT 0,
 started_at INTEGER NOT NULL, updated_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS history_import_chats(
 chat TEXT PRIMARY KEY REFERENCES chats(id) ON DELETE CASCADE,
 activity INTEGER NOT NULL DEFAULT 0, round INTEGER NOT NULL DEFAULT 0,
 state INTEGER NOT NULL, reason INTEGER NOT NULL,
 frontier_timestamp INTEGER, frontier_id TEXT, frontier_from_me INTEGER,
 pages INTEGER NOT NULL DEFAULT 0, messages INTEGER NOT NULL DEFAULT 0,
 attempts INTEGER NOT NULL DEFAULT 0, retry_at INTEGER NOT NULL DEFAULT 0,
 request_token INTEGER, request_session TEXT, request_started INTEGER
);
CREATE TABLE IF NOT EXISTS history_import_frontiers(
 chat TEXT NOT NULL, timestamp INTEGER NOT NULL, id TEXT NOT NULL,
 PRIMARY KEY(chat,timestamp,id)
);
CREATE INDEX IF NOT EXISTS history_import_due
 ON history_import_chats(state,round,activity DESC,retry_at);
CREATE TRIGGER IF NOT EXISTS delete_history_import_chat AFTER DELETE ON chats BEGIN
 DELETE FROM history_import_chats WHERE chat=OLD.id;
 DELETE FROM history_import_frontiers WHERE chat=OLD.id;
END;
";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistoryImportFailure {
    Send,
    Timeout,
    NoProgress,
    Parse,
    Storage,
    CorrelationUnavailable,
    RetryLimitReached,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistoryPageOutcome {
    AppliedProgress,
    Exhausted,
    NoProgress,
    Stale,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistoryChatFailureOutcome {
    RetryScheduled,
    ChatStopped,
    Stale,
}

impl HistoryImportFailure {
    fn reason(self) -> i64 {
        match self {
            Self::Send => REASON_RETRY_SEND,
            Self::Timeout => REASON_RETRY_TIMEOUT,
            Self::NoProgress => REASON_RETRY_NO_PROGRESS,
            Self::Parse => REASON_RETRY_PARSE,
            Self::Storage => REASON_RETRY_STORAGE,
            Self::CorrelationUnavailable => REASON_CORRELATION_UNAVAILABLE,
            Self::RetryLimitReached => REASON_RETRY_LIMIT_REACHED,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryImportRequest {
    pub token: i64,
    pub chat: ChatId,
    pub id: String,
    pub from_me: bool,
    pub timestamp: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistoryAnchorKind {
    DirectPn,
    DirectLid,
    Group,
    Other,
}

/// Aggregate protocol-shape metadata for diagnostics. Contains no archive ids,
/// message ids, contacts, names, or content.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HistoryImportAnchorMetadata {
    pub active: u32,
    pub reserved: u32,
    pub direct_pn: u32,
    pub direct_lid: u32,
    pub groups: u32,
    pub other: u32,
    pub empty_anchor: u32,
    pub nonempty_anchor: u32,
    pub archived: u32,
    pub reserved_kind: Option<HistoryAnchorKind>,
    pub reserved_empty: bool,
    pub reserved_archived: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryImportChatSnapshot {
    pub chat: ChatId,
    pub reason: ChatImportReason,
    pub frontier: Option<i64>,
    pub pages: u32,
    pub messages: u64,
    pub retry_at: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryImportSnapshot {
    pub target_since: Option<i64>,
    pub state: ImportRunState,
    pub failure_streak: u8,
    pub reason: Option<ChatImportReason>,
    pub completed: u32,
    pub total: u32,
    pub excluded: u32,
    pub retry_at: Option<i64>,
    pub chats: Vec<HistoryImportChatSnapshot>,
}

impl From<HistoryImportSnapshot> for crate::model::HistoryImportProgress {
    fn from(value: HistoryImportSnapshot) -> Self {
        Self {
            target_since: value.target_since,
            state: value.state,
            failure_streak: value.failure_streak,
            reason: value.reason,
            completed: value.completed,
            total: value.total,
            excluded: value.excluded,
            retry_at: value.retry_at,
            chats: value
                .chats
                .into_iter()
                .map(|row| crate::model::ChatImportProgress {
                    chat: row.chat,
                    reason: row.reason,
                    frontier: row.frontier,
                    pages: row.pages,
                    messages: row.messages,
                    retry_at: row.retry_at,
                })
                .collect(),
        }
    }
}

pub(super) fn install(connection: &rusqlite::Connection) -> Result<()> {
    connection.execute_batch(SCHEMA)
}

fn reason_from_db(value: i64) -> ChatImportReason {
    match value {
        REASON_WAITING_FOR_PHONE => ChatImportReason::WaitingForPhone,
        REASON_ADVANCING => ChatImportReason::Advancing,
        REASON_RETRY_SEND => ChatImportReason::RetrySend,
        REASON_RETRY_TIMEOUT => ChatImportReason::RetryTimeout,
        REASON_RETRY_NO_PROGRESS => ChatImportReason::RetryNoProgress,
        REASON_RETRY_PARSE => ChatImportReason::RetryParse,
        REASON_RETRY_STORAGE => ChatImportReason::RetryStorage,
        REASON_WAITING_FOR_ANCHOR => ChatImportReason::WaitingForAnchor,
        REASON_REACHED_CUTOFF => ChatImportReason::ReachedCutoff,
        REASON_PHONE_EXHAUSTED => ChatImportReason::PhoneExhausted,
        REASON_EXCLUDED_LOCKED => ChatImportReason::ExcludedLocked,
        REASON_EXCLUDED_LEFT => ChatImportReason::ExcludedLeft,
        REASON_EXCLUDED_UNSUPPORTED => ChatImportReason::ExcludedUnsupported,
        REASON_CORRELATION_UNAVAILABLE => ChatImportReason::CorrelationUnavailable,
        REASON_RETRY_LIMIT_REACHED => ChatImportReason::RetryLimitReached,
        _ => ChatImportReason::Queued,
    }
}

impl Archive {
    pub fn configure_history_import(
        &self,
        target: Option<i64>,
        paused: bool,
        now: i64,
    ) -> Result<()> {
        let transaction = self.connection.unchecked_transaction()?;
        let Some(target) = target else {
            transaction.execute("DELETE FROM history_import_chats", [])?;
            transaction.execute("DELETE FROM history_import_frontiers", [])?;
            transaction.execute("DELETE FROM history_import_job", [])?;
            transaction.commit()?;
            return Ok(());
        };
        let current: Option<i64> = transaction
            .query_row(
                "SELECT target_since FROM history_import_job WHERE id=1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        if current != Some(target) {
            transaction.execute("DELETE FROM history_import_chats", [])?;
            transaction.execute("DELETE FROM history_import_frontiers", [])?;
            let sequence: i64 = transaction.query_row(
                "SELECT COALESCE((SELECT CAST(value AS INTEGER) FROM meta WHERE key='history_import_token_sequence_v1'),0)",
                [], |row| row.get(0))?;
            transaction.execute(
                "INSERT INTO history_import_job(id,target_since,state,started_at,updated_at,request_token)
                 VALUES(1,?1,?2,?3,?3,?4)
                 ON CONFLICT(id) DO UPDATE SET target_since=excluded.target_since,state=excluded.state,
                  reason=NULL,discovery_generation=history_import_job.discovery_generation+1,
                  round=0,cursor=0,failure_streak=0,retry_at=0,request_token=?4,
                  started_at=excluded.started_at,updated_at=excluded.updated_at",
                params![target, if paused { JOB_PAUSED } else { JOB_RUNNING }, now, sequence],
            )?;
        } else {
            transaction.execute(
                "UPDATE history_import_job SET state=CASE WHEN ?1 THEN ?2 WHEN state=?2 THEN ?3 ELSE state END,
                 reason=CASE WHEN state=?2 AND NOT ?1 THEN NULL ELSE reason END,updated_at=?4 WHERE id=1",
                params![paused, JOB_PAUSED, JOB_RUNNING, now],
            )?;
        }
        transaction.commit()
    }

    pub fn reconcile_history_import(&self, now: i64) -> Result<()> {
        let transaction = self.connection.unchecked_transaction()?;
        let exists: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM history_import_job WHERE id=1)",
            [],
            |row| row.get(0),
        )?;
        if !exists {
            return Ok(());
        }
        let before_active: i64 = transaction.query_row(
            "SELECT COUNT(*) FROM history_import_chats WHERE state=?1",
            [CHAT_ACTIVE],
            |row| row.get(0),
        )?;
        transaction.execute(
            "INSERT INTO history_import_chats(chat,activity,state,reason,frontier_timestamp,frontier_id,frontier_from_me)
             SELECT c.id,c.last_activity,
              CASE WHEN c.locked<>0 OR c.left<>0 OR c.kind NOT IN ('direct','group') THEN ?1 ELSE ?2 END,
              CASE WHEN c.locked<>0 THEN ?3 WHEN c.left<>0 THEN ?4
                   WHEN c.kind NOT IN ('direct','group') THEN ?5
                   WHEN EXISTS(SELECT 1 FROM messages any WHERE any.chat=c.id) AND anchor.id IS NULL THEN ?6
                   ELSE ?7 END,
              CASE WHEN anchor.id IS NULL AND NOT EXISTS(SELECT 1 FROM messages any WHERE any.chat=c.id) THEN ?8 ELSE anchor.timestamp END,
              CASE WHEN anchor.id IS NULL AND NOT EXISTS(SELECT 1 FROM messages any WHERE any.chat=c.id) THEN '' ELSE anchor.id END,
              COALESCE(anchor.from_me,0)
             FROM chats c
             LEFT JOIN messages anchor ON anchor.rowid=(
               SELECT m.rowid FROM messages m
               WHERE c.locked=0 AND m.chat=c.id AND m.id NOT LIKE 'zapfast-call-%'
                AND NOT(m.from_me=1 AND m.status IN (1,6))
                AND NOT EXISTS(SELECT 1 FROM message_removals r WHERE r.chat=m.chat AND r.id=m.id)
                AND NOT EXISTS(SELECT 1 FROM pending_message_removals r WHERE r.chat=m.chat AND r.id=m.id)
                AND m.timestamp>COALESCE((SELECT through FROM chat_removals r WHERE r.chat=m.chat),-1)
               ORDER BY m.timestamp DESC,COALESCE(m.history_order,9223372036854775807) DESC,m.rowid DESC LIMIT 1)
             ON CONFLICT(chat) DO UPDATE SET activity=MAX(history_import_chats.activity,excluded.activity),
              state=CASE WHEN history_import_chats.state IN (?9,?10,?11) THEN history_import_chats.state
                WHEN history_import_chats.state=?1 AND history_import_chats.reason=?12 THEN history_import_chats.state
                WHEN excluded.state=?1 THEN ?1 WHEN history_import_chats.state=?1 THEN excluded.state
                ELSE history_import_chats.state END,
              reason=CASE WHEN history_import_chats.state IN (?9,?10,?11) THEN history_import_chats.reason
                WHEN history_import_chats.state=?1 AND history_import_chats.reason=?12 THEN history_import_chats.reason
                WHEN excluded.state=?1 THEN excluded.reason WHEN history_import_chats.state=?1 THEN excluded.reason
                WHEN history_import_chats.frontier_id IS NULL AND excluded.frontier_id IS NOT NULL THEN excluded.reason
                ELSE history_import_chats.reason END,
              frontier_timestamp=CASE WHEN (history_import_chats.state=?1 AND excluded.state=?2)
                OR (history_import_chats.frontier_id IS NULL AND excluded.frontier_id IS NOT NULL) THEN excluded.frontier_timestamp ELSE history_import_chats.frontier_timestamp END,
              frontier_id=CASE WHEN (history_import_chats.state=?1 AND excluded.state=?2)
                OR (history_import_chats.frontier_id IS NULL AND excluded.frontier_id IS NOT NULL) THEN excluded.frontier_id ELSE history_import_chats.frontier_id END,
              frontier_from_me=CASE WHEN (history_import_chats.state=?1 AND excluded.state=?2)
                OR (history_import_chats.frontier_id IS NULL AND excluded.frontier_id IS NOT NULL) THEN excluded.frontier_from_me ELSE history_import_chats.frontier_from_me END",
            params![CHAT_EXCLUDED, CHAT_ACTIVE, REASON_EXCLUDED_LOCKED, REASON_EXCLUDED_LEFT,
                REASON_EXCLUDED_UNSUPPORTED, REASON_WAITING_FOR_ANCHOR, REASON_QUEUED, now,
                CHAT_REACHED_CUTOFF, CHAT_PHONE_EXHAUSTED, CHAT_STOPPED, REASON_EXCLUDED_UNSUPPORTED],
        )?;
        transaction.execute(
            "INSERT OR IGNORE INTO history_import_frontiers(chat,timestamp,id)
             SELECT chat,frontier_timestamp,frontier_id FROM history_import_chats
             WHERE frontier_timestamp IS NOT NULL AND frontier_id IS NOT NULL",
            [],
        )?;
        let after_active: i64 = transaction.query_row(
            "SELECT COUNT(*) FROM history_import_chats WHERE state=?1",
            [CHAT_ACTIVE],
            |row| row.get(0),
        )?;
        transaction.execute(
            "UPDATE history_import_job SET discovery_generation=discovery_generation+1,
             state=CASE WHEN state=?1 AND ?2>0 THEN ?3 ELSE state END,updated_at=?4 WHERE id=1",
            params![
                JOB_FINISHED,
                after_active.saturating_sub(before_active),
                JOB_RUNNING,
                now
            ],
        )?;
        Self::finish_job_if_terminal(&transaction, now)?;
        transaction.commit()
    }

    pub fn next_history_import(&self, now: i64) -> Result<Option<HistoryImportRequest>> {
        let transaction = self.connection.unchecked_transaction()?;
        let state: Option<(i64, i64)> = transaction
            .query_row(
                "SELECT state,retry_at FROM history_import_job WHERE id=1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((state, retry_at)) = state else {
            return Ok(None);
        };
        if state != JOB_RUNNING || now < retry_at {
            return Ok(None);
        }
        transaction.execute(
            "UPDATE history_import_chats SET reason=?1,attempts=attempts+1,retry_at=?2,
             request_token=NULL,request_session=NULL,request_started=NULL
             WHERE state=?3 AND request_started IS NOT NULL AND request_started<=?4",
            params![
                REASON_RETRY_TIMEOUT,
                now.saturating_add(60),
                CHAT_ACTIVE,
                // Longer than the worker's bounded preflight/dispatch deadline.
                // This is the restart recovery fence, not the phone response timer.
                now.saturating_sub(240)
            ],
        )?;
        let candidate: Option<(String, String, bool, i64)> = transaction
            .query_row(
                "SELECT chat,frontier_id,frontier_from_me,frontier_timestamp FROM history_import_chats
                 WHERE state=?1 AND frontier_id IS NOT NULL AND retry_at<=?2 AND request_token IS NULL
                 ORDER BY round ASC,activity DESC,chat ASC LIMIT 1",
                params![CHAT_ACTIVE, now],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()?;
        let Some((chat, id, from_me, timestamp)) = candidate else {
            transaction.commit()?;
            return Ok(None);
        };
        transaction.execute(
            "UPDATE history_import_job SET request_token=request_token+1,updated_at=?1 WHERE id=1",
            [now],
        )?;
        let token: i64 = transaction.query_row(
            "SELECT request_token FROM history_import_job WHERE id=1",
            [],
            |row| row.get(0),
        )?;
        transaction.execute(
            "INSERT INTO meta(key,value) VALUES('history_import_token_sequence_v1',?1)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            [token.to_string()],
        )?;
        transaction.execute(
            "UPDATE history_import_chats SET round=round+1,reason=?1,request_token=?2,request_started=?3,
             request_session=NULL WHERE chat=?4 AND request_token IS NULL",
            params![REASON_WAITING_FOR_PHONE, token, now, chat],
        )?;
        transaction.commit()?;
        Ok(Some(HistoryImportRequest {
            token,
            chat,
            id,
            from_me,
            timestamp,
        }))
    }

    pub fn bind_history_import_session(&self, token: i64, session: &str, now: i64) -> Result<bool> {
        Ok(self.connection.execute(
            "UPDATE history_import_chats SET request_session=?2,request_started=?3 WHERE request_token=?1
             AND request_started IS NOT NULL AND request_session IS NULL",
            params![token, session, now],
        )? > 0)
    }

    pub fn history_import_request_for_session(
        &self,
        session: &str,
    ) -> Result<Option<(ChatId, i64)>> {
        self.connection
            .query_row(
                "SELECT c.chat,c.request_token FROM history_import_chats c
             JOIN history_import_job j ON j.id=1
             WHERE c.request_session=?1 AND c.request_token IS NOT NULL AND c.state=?2",
                params![session, CHAT_ACTIVE],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
    }

    pub fn finish_history_import_page(
        &self,
        token: i64,
        page_oldest: Option<(i64, String, bool)>,
        filed: usize,
        more_on_phone: Option<bool>,
        durable: bool,
        now: i64,
    ) -> Result<HistoryPageOutcome> {
        let transaction = self.connection.unchecked_transaction()?;
        let row: Option<(String, i64, String, i64)> = transaction
            .query_row(
                "SELECT chat,frontier_timestamp,frontier_id,(SELECT target_since FROM history_import_job WHERE id=1)
                 FROM history_import_chats WHERE request_token=?1 AND request_session IS NOT NULL",
                [token],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()?;
        let Some((chat, old_timestamp, old_id, target)) = row else {
            return Ok(HistoryPageOutcome::Stale);
        };
        if !durable {
            Self::fail_page(
                &transaction,
                token,
                HistoryImportFailure::Storage,
                now.saturating_add(60),
                now,
            )?;
            transaction.commit()?;
            return Ok(HistoryPageOutcome::NoProgress);
        }
        let advanced = if let Some((timestamp, id, _)) = page_oldest.as_ref() {
            *timestamp < old_timestamp || (*timestamp == old_timestamp && id != &old_id
                && !transaction.query_row(
                    "SELECT EXISTS(SELECT 1 FROM history_import_frontiers WHERE chat=?1 AND timestamp=?2 AND id=?3)",
                    params![chat,timestamp,id], |row| row.get::<_,bool>(0))?)
        } else {
            false
        };
        let successful = if more_on_phone == Some(false) {
            let (frontier_timestamp, frontier_id, frontier_from_me) = if advanced {
                page_oldest.unwrap()
            } else {
                let from_me: bool = transaction.query_row(
                    "SELECT frontier_from_me FROM history_import_chats WHERE chat=?1",
                    [&chat],
                    |row| row.get(0),
                )?;
                (old_timestamp, old_id, from_me)
            };
            let reached = frontier_timestamp <= target;
            transaction.execute(
                "UPDATE history_import_chats SET state=?1,reason=?2,frontier_timestamp=?3,frontier_id=?4,
                 frontier_from_me=?5,pages=pages+1,messages=messages+?6,attempts=0,retry_at=0,
                 request_token=NULL,request_session=NULL,request_started=NULL WHERE chat=?7",
                params![if reached { CHAT_REACHED_CUTOFF } else { CHAT_PHONE_EXHAUSTED },
                    if reached { REASON_REACHED_CUTOFF } else { REASON_PHONE_EXHAUSTED },
                    frontier_timestamp, frontier_id, frontier_from_me,
                    filed.min(i64::MAX as usize) as i64, chat],
            )?;
            transaction.execute("INSERT OR IGNORE INTO history_import_frontiers(chat,timestamp,id) VALUES(?1,?2,?3)",params![chat,frontier_timestamp,frontier_id])?;
            true
        } else if advanced {
            let (frontier_timestamp, frontier_id, frontier_from_me) = page_oldest.unwrap();
            let reached = frontier_timestamp <= target;
            transaction.execute(
                "UPDATE history_import_chats SET state=?1,reason=?2,frontier_timestamp=?3,frontier_id=?4,
                 frontier_from_me=?5,pages=pages+1,messages=messages+?6,attempts=0,retry_at=0,
                 request_token=NULL,request_session=NULL,request_started=NULL WHERE chat=?7",
                params![if reached { CHAT_REACHED_CUTOFF } else { CHAT_ACTIVE },
                    if reached { REASON_REACHED_CUTOFF } else { REASON_ADVANCING },
                    frontier_timestamp, frontier_id, frontier_from_me,
                    filed.min(i64::MAX as usize) as i64, chat],
            )?;
            transaction.execute("INSERT OR IGNORE INTO history_import_frontiers(chat,timestamp,id) VALUES(?1,?2,?3)",params![chat,frontier_timestamp,frontier_id])?;
            true
        } else {
            Self::fail_chat_page(&transaction, token, HistoryImportFailure::NoProgress, now)?;
            // A correlated durable page proves the phone answered even when it
            // could not advance this chat. Clear only systemic failure accounting.
            transaction.execute(
                "UPDATE history_import_job SET failure_streak=0,retry_at=0,
                 reason=CASE WHEN state=?1 THEN reason ELSE NULL END,updated_at=?2 WHERE id=1",
                params![JOB_NEEDS_ATTENTION, now],
            )?;
            false
        };
        if successful {
            transaction.execute(
                "UPDATE history_import_job SET failure_streak=0,retry_at=0,reason=NULL,updated_at=?1 WHERE id=1",
                [now],
            )?;
        }
        Self::finish_job_if_terminal(&transaction, now)?;
        transaction.commit()?;
        Ok(if !successful {
            HistoryPageOutcome::NoProgress
        } else if more_on_phone == Some(false) {
            HistoryPageOutcome::Exhausted
        } else {
            HistoryPageOutcome::AppliedProgress
        })
    }

    pub fn fail_history_import(
        &self,
        token: i64,
        reason: HistoryImportFailure,
        retry_at: i64,
    ) -> Result<()> {
        debug_assert!(!matches!(
            reason,
            HistoryImportFailure::Timeout | HistoryImportFailure::NoProgress
        ));
        let now = crate::util::now();
        let transaction = self.connection.unchecked_transaction()?;
        Self::fail_page(&transaction, token, reason, retry_at, now)?;
        transaction.commit()
    }

    pub fn fail_history_import_chat(
        &self,
        token: i64,
        reason: HistoryImportFailure,
        now: i64,
    ) -> Result<HistoryChatFailureOutcome> {
        debug_assert!(matches!(
            reason,
            HistoryImportFailure::Timeout | HistoryImportFailure::NoProgress
        ));
        let transaction = self.connection.unchecked_transaction()?;
        let outcome = Self::fail_chat_page(&transaction, token, reason, now)?;
        transaction.commit()?;
        Ok(outcome)
    }

    pub fn release_deferred_history_import_request(&self, token: i64, now: i64) -> Result<bool> {
        let transaction = self.connection.unchecked_transaction()?;
        let changed = transaction.execute(
            "UPDATE history_import_chats SET reason=?1,retry_at=0,request_token=NULL,
             request_session=NULL,request_started=NULL WHERE request_token=?2 AND state=?3",
            params![REASON_QUEUED, token, CHAT_ACTIVE],
        )?;
        if changed > 0 {
            transaction.execute(
                "UPDATE history_import_job SET updated_at=?1 WHERE id=1",
                [now],
            )?;
        }
        transaction.commit()?;
        Ok(changed > 0)
    }

    pub fn exclude_unsupported_history_import_request(&self, token: i64, now: i64) -> Result<bool> {
        let transaction = self.connection.unchecked_transaction()?;
        let changed = transaction.execute(
            "UPDATE history_import_chats SET state=?1,reason=?2,retry_at=0,request_token=NULL,
             request_session=NULL,request_started=NULL WHERE request_token=?3 AND state=?4",
            params![
                CHAT_EXCLUDED,
                REASON_EXCLUDED_UNSUPPORTED,
                token,
                CHAT_ACTIVE
            ],
        )?;
        if changed > 0 {
            Self::finish_job_if_terminal(&transaction, now)?;
        }
        transaction.commit()?;
        Ok(changed > 0)
    }

    fn fail_chat_page(
        transaction: &Transaction<'_>,
        token: i64,
        reason: HistoryImportFailure,
        now: i64,
    ) -> Result<HistoryChatFailureOutcome> {
        let attempts: Option<i64> = transaction
            .query_row(
                "SELECT attempts FROM history_import_chats WHERE request_token=?1 AND state=?2",
                params![token, CHAT_ACTIVE],
                |row| row.get(0),
            )
            .optional()?;
        let Some(attempts) = attempts else {
            return Ok(HistoryChatFailureOutcome::Stale);
        };
        let attempts = attempts.saturating_add(1);
        let stopped = attempts >= MAX_CHAT_SILENT_FAILURES;
        let retry_at = if stopped {
            0
        } else {
            now.saturating_add(60_i64.saturating_mul(1_i64 << attempts.saturating_sub(1).min(6)))
        };
        transaction.execute(
            "UPDATE history_import_chats SET state=?1,reason=?2,attempts=?3,retry_at=?4,
             request_token=NULL,request_session=NULL,request_started=NULL WHERE request_token=?5 AND state=?6",
            params![
                if stopped { CHAT_STOPPED } else { CHAT_ACTIVE },
                if stopped {
                    REASON_RETRY_LIMIT_REACHED
                } else {
                    reason.reason()
                },
                attempts,
                retry_at,
                token,
                CHAT_ACTIVE
            ],
        )?;
        Self::finish_job_if_terminal(transaction, now)?;
        Ok(if stopped {
            HistoryChatFailureOutcome::ChatStopped
        } else {
            HistoryChatFailureOutcome::RetryScheduled
        })
    }

    fn fail_page(
        transaction: &Transaction<'_>,
        token: i64,
        reason: HistoryImportFailure,
        retry_at: i64,
        now: i64,
    ) -> Result<()> {
        let changed = transaction.execute(
            "UPDATE history_import_chats SET reason=?1,attempts=attempts+1,retry_at=MAX(retry_at,?2),
             request_token=NULL,request_session=NULL,request_started=NULL WHERE request_token=?3",
            params![reason.reason(), retry_at, token],
        )?;
        if changed > 0 {
            transaction.execute(
                "UPDATE history_import_job SET failure_streak=failure_streak+1,reason=?1,
                 retry_at=MAX(retry_at,?2),updated_at=?3 WHERE id=1",
                params![reason.reason(), retry_at, now],
            )?;
        }
        Ok(())
    }

    pub fn pause_history_import(&self, reason: HistoryImportFailure, now: i64) -> Result<()> {
        self.connection.execute(
            "UPDATE history_import_job SET state=?1,reason=?2,updated_at=?3 WHERE id=1",
            params![JOB_NEEDS_ATTENTION, reason.reason(), now],
        )?;
        Ok(())
    }

    /// Preserve an uncertain in-flight request so neither restart recovery nor
    /// Resume can overlap a phone send whose transport outcome is unknown.
    pub fn pause_history_import_request(
        &self,
        token: i64,
        reason: HistoryImportFailure,
        now: i64,
    ) -> Result<bool> {
        let transaction = self.connection.unchecked_transaction()?;
        let changed = transaction.execute(
            "UPDATE history_import_chats SET reason=?1 WHERE request_token=?2 AND state=?3",
            params![reason.reason(), token, CHAT_ACTIVE],
        )?;
        if changed > 0 {
            transaction.execute(
                "UPDATE history_import_job SET state=?1,reason=?2,failure_streak=failure_streak+1,
                 updated_at=?3 WHERE id=1",
                params![JOB_NEEDS_ATTENTION, reason.reason(), now],
            )?;
        }
        transaction.commit()?;
        Ok(changed > 0)
    }

    /// Release one retained uncertain request after its dispatch task has been
    /// cancelled or otherwise fenced. Does not alter the overall run state.
    pub fn release_history_import_request(
        &self,
        token: i64,
        reason: HistoryImportFailure,
        now: i64,
    ) -> Result<bool> {
        let transaction = self.connection.unchecked_transaction()?;
        let changed = transaction.execute(
            "UPDATE history_import_chats SET reason=?1,retry_at=0,request_token=NULL,
             request_session=NULL,request_started=NULL WHERE request_token=?2 AND state=?3",
            params![reason.reason(), token, CHAT_ACTIVE],
        )?;
        if changed > 0 {
            transaction.execute(
                "UPDATE history_import_job SET updated_at=?1 WHERE id=1",
                [now],
            )?;
        }
        transaction.commit()?;
        Ok(changed > 0)
    }

    /// Release dispatch-uncertain reservations after restart, when the worker's
    /// in-memory task fence is gone. Bound phone sessions are deliberately kept
    /// so a late correlated HistorySync can still complete them.
    pub fn release_uncertain_history_import_requests(&self, now: i64) -> Result<u32> {
        let transaction = self.connection.unchecked_transaction()?;
        let uncertain_job: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM history_import_job
             WHERE id=1 AND state=?1 AND reason=?2)",
            params![JOB_NEEDS_ATTENTION, REASON_CORRELATION_UNAVAILABLE],
            |row| row.get(0),
        )?;
        let changed = if uncertain_job {
            transaction.execute(
                "UPDATE history_import_chats SET reason=?1,retry_at=0,request_token=NULL,
                 request_session=NULL,request_started=NULL
                 WHERE state=?2 AND reason=?3 AND request_token IS NOT NULL
                  AND request_session IS NULL",
                params![
                    REASON_RETRY_SEND,
                    CHAT_ACTIVE,
                    REASON_CORRELATION_UNAVAILABLE
                ],
            )?
        } else {
            0
        };
        if changed > 0 {
            transaction.execute(
                "UPDATE history_import_job SET updated_at=?1 WHERE id=1",
                [now],
            )?;
        }
        transaction.commit()?;
        Ok(changed.min(u32::MAX as usize) as u32)
    }

    pub fn resume_history_import(&self, now: i64) -> Result<()> {
        let transaction = self.connection.unchecked_transaction()?;
        transaction.execute(
            "UPDATE history_import_job SET state=?1,reason=NULL,failure_streak=0,retry_at=0,updated_at=?2 WHERE id=1",
            params![JOB_RUNNING, now],
        )?;
        transaction.execute(
            "UPDATE history_import_chats SET state=?1,reason=?2,attempts=0,retry_at=0
             WHERE state=?3 AND reason=?4",
            params![
                CHAT_ACTIVE,
                REASON_QUEUED,
                CHAT_STOPPED,
                REASON_RETRY_LIMIT_REACHED
            ],
        )?;
        transaction.execute(
            "UPDATE history_import_chats SET retry_at=0,reason=CASE WHEN reason IN (?1,?2) THEN ?3 ELSE reason END
             WHERE state=?4",
            params![REASON_CORRELATION_UNAVAILABLE, REASON_RETRY_LIMIT_REACHED, REASON_QUEUED, CHAT_ACTIVE],
        )?;
        transaction.commit()
    }

    pub fn history_import_progress(&self, now: i64) -> Result<HistoryImportSnapshot> {
        let job: Option<(i64, i64, i64, Option<i64>, i64)> = self
            .connection
            .query_row(
                "SELECT target_since,state,retry_at,reason,failure_streak FROM history_import_job WHERE id=1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
            )
            .optional()?;
        let Some((target, state, retry_at, reason, failure_streak)) = job else {
            return Ok(HistoryImportSnapshot {
                target_since: None,
                state: ImportRunState::Idle,
                failure_streak: 0,
                reason: None,
                completed: 0,
                total: 0,
                excluded: 0,
                retry_at: None,
                chats: Vec::new(),
            });
        };
        let (total, completed, excluded): (u32, u32, u32) = self.connection.query_row(
            "SELECT COUNT(*),COALESCE(SUM(state IN (?1,?2)),0),COALESCE(SUM(state=?3),0) FROM history_import_chats",
            params![CHAT_REACHED_CUTOFF, CHAT_PHONE_EXHAUSTED, CHAT_EXCLUDED],
            |row| {
                Ok((
                    row.get::<_, i64>(0)? as u32,
                    row.get::<_, i64>(1)? as u32,
                    row.get::<_, i64>(2)? as u32,
                ))
            },
        )?;
        let chat_retry: Option<(i64, i64)> = if state == JOB_RUNNING && retry_at <= now {
            let (runnable, inflight, waiting_for_anchor): (bool, bool, bool) =
                self.connection.query_row(
                    "SELECT
                     EXISTS(SELECT 1 FROM history_import_chats WHERE state=?1 AND frontier_id IS NOT NULL AND request_token IS NULL AND retry_at<=?2),
                     EXISTS(SELECT 1 FROM history_import_chats WHERE state=?1 AND request_token IS NOT NULL),
                     EXISTS(SELECT 1 FROM history_import_chats WHERE state=?1 AND frontier_id IS NULL)",
                    params![CHAT_ACTIVE, now],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )?;
            if runnable || inflight || waiting_for_anchor {
                None
            } else {
                self.connection.query_row(
                    "SELECT retry_at,reason FROM history_import_chats
                     WHERE state=?1 AND frontier_id IS NOT NULL AND request_token IS NULL AND retry_at>?2
                     ORDER BY retry_at,round,activity DESC,chat LIMIT 1",
                    params![CHAT_ACTIVE, now],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                ).optional()?
            }
        } else {
            None
        };
        let display_retry_at = retry_at.max(chat_retry.map_or(0, |(at, _)| at));
        let display_reason = reason.or_else(|| chat_retry.map(|(_, reason)| reason));
        let chats = self.connection
            .prepare(
                "SELECT chat,reason,frontier_timestamp,pages,messages,retry_at FROM history_import_chats
                 WHERE state IN (?1,?2) ORDER BY round,activity DESC,chat LIMIT 500",
            )?
            .query_map(params![CHAT_ACTIVE, CHAT_STOPPED], |row| {
                Ok(HistoryImportChatSnapshot {
                    chat: row.get(0)?,
                    reason: reason_from_db(row.get(1)?),
                    frontier: row.get(2)?,
                    pages: row.get::<_, i64>(3)? as u32,
                    messages: row.get::<_, i64>(4)? as u64,
                    retry_at: {
                        let at: i64 = row.get(5)?;
                        (at > 0).then_some(at)
                    },
                })
            })?
            .collect::<Result<Vec<_>>>()?;
        Ok(HistoryImportSnapshot {
            target_since: Some(target),
            state: match state {
                JOB_RUNNING if display_retry_at > now => ImportRunState::BackingOff,
                JOB_RUNNING => ImportRunState::Running,
                JOB_PAUSED => ImportRunState::Paused,
                JOB_NEEDS_ATTENTION => ImportRunState::NeedsAttention,
                _ => ImportRunState::Finished,
            },
            failure_streak: failure_streak.clamp(0, u8::MAX as i64) as u8,
            reason: display_reason.map(reason_from_db),
            completed,
            total,
            excluded,
            retry_at: (display_retry_at > 0).then_some(display_retry_at),
            chats,
        })
    }

    pub fn history_import_anchor_metadata(&self) -> Result<HistoryImportAnchorMetadata> {
        let mut metadata = self.connection.query_row(
            "SELECT COUNT(*),COALESCE(SUM(request_token IS NOT NULL),0),
             COALESCE(SUM(chat LIKE '%@s.whatsapp.net'),0),
             COALESCE(SUM(chat LIKE '%@lid'),0),COALESCE(SUM(chat LIKE '%@g.us'),0),
             COALESCE(SUM(chat NOT LIKE '%@s.whatsapp.net' AND chat NOT LIKE '%@lid' AND chat NOT LIKE '%@g.us'),0),
             COALESCE(SUM(COALESCE(frontier_id,'')=''),0),COALESCE(SUM(COALESCE(frontier_id,'')<>''),0),
             COALESCE(SUM(c.archived),0)
             FROM history_import_chats h JOIN chats c ON c.id=h.chat WHERE h.state=?1",
            [CHAT_ACTIVE],
            |row| {
                Ok(HistoryImportAnchorMetadata {
                    active: row.get::<_, i64>(0)? as u32,
                    reserved: row.get::<_, i64>(1)? as u32,
                    direct_pn: row.get::<_, i64>(2)? as u32,
                    direct_lid: row.get::<_, i64>(3)? as u32,
                    groups: row.get::<_, i64>(4)? as u32,
                    other: row.get::<_, i64>(5)? as u32,
                    empty_anchor: row.get::<_, i64>(6)? as u32,
                    nonempty_anchor: row.get::<_, i64>(7)? as u32,
                    archived: row.get::<_, i64>(8)? as u32,
                    ..Default::default()
                })
            },
        )?;
        let reserved: Option<(String, bool, bool)> = self
            .connection
            .query_row(
                "SELECT h.chat,COALESCE(h.frontier_id,'')='',c.archived
                 FROM history_import_chats h JOIN chats c ON c.id=h.chat
                 WHERE h.state=?1 AND h.request_token IS NOT NULL LIMIT 1",
                [CHAT_ACTIVE],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        if let Some((chat, empty, archived)) = reserved {
            metadata.reserved_kind = Some(if chat.ends_with("@s.whatsapp.net") {
                HistoryAnchorKind::DirectPn
            } else if chat.ends_with("@lid") {
                HistoryAnchorKind::DirectLid
            } else if chat.ends_with("@g.us") {
                HistoryAnchorKind::Group
            } else {
                HistoryAnchorKind::Other
            });
            metadata.reserved_empty = empty;
            metadata.reserved_archived = archived;
        }
        Ok(metadata)
    }

    fn finish_job_if_terminal(transaction: &Transaction<'_>, now: i64) -> Result<()> {
        transaction.execute(
            "UPDATE history_import_job SET state=CASE
               WHEN EXISTS(SELECT 1 FROM history_import_chats WHERE state=?1) THEN ?2 ELSE ?3 END,
             reason=CASE WHEN EXISTS(SELECT 1 FROM history_import_chats WHERE state=?1) THEN ?4 ELSE reason END,
             updated_at=?5 WHERE id=1 AND state=?6
             AND NOT EXISTS(SELECT 1 FROM history_import_chats WHERE state=?7)",
            params![
                CHAT_STOPPED,
                JOB_NEEDS_ATTENTION,
                JOB_FINISHED,
                REASON_RETRY_LIMIT_REACHED,
                now,
                JOB_RUNNING,
                CHAT_ACTIVE
            ],
        )?;
        Ok(())
    }
}

pub(super) fn merge_chat(transaction: &Transaction<'_>, old: &str, new: &str) -> Result<()> {
    transaction.execute(
        "INSERT OR IGNORE INTO history_import_frontiers(chat,timestamp,id)
         SELECT ?2,timestamp,id FROM history_import_frontiers WHERE chat=?1",
        params![old, new],
    )?;
    transaction.execute("DELETE FROM history_import_frontiers WHERE chat=?1", [old])?;
    transaction.execute(
        "INSERT INTO history_import_chats(chat,activity,round,state,reason,frontier_timestamp,frontier_id,
          frontier_from_me,pages,messages,attempts,retry_at,request_token,request_session,request_started)
         SELECT ?2,activity,round,state,reason,frontier_timestamp,frontier_id,frontier_from_me,pages,messages,attempts,retry_at,
          request_token,request_session,request_started
         FROM history_import_chats WHERE chat=?1
         ON CONFLICT(chat) DO UPDATE SET
          activity=MAX(history_import_chats.activity,excluded.activity),round=MAX(history_import_chats.round,excluded.round),
          frontier_timestamp=CASE WHEN excluded.frontier_timestamp IS NULL THEN history_import_chats.frontier_timestamp
            WHEN history_import_chats.frontier_timestamp IS NULL OR excluded.frontier_timestamp<history_import_chats.frontier_timestamp THEN excluded.frontier_timestamp ELSE history_import_chats.frontier_timestamp END,
          frontier_id=CASE WHEN excluded.frontier_timestamp IS NOT NULL AND (history_import_chats.frontier_timestamp IS NULL OR excluded.frontier_timestamp<history_import_chats.frontier_timestamp) THEN excluded.frontier_id ELSE history_import_chats.frontier_id END,
          frontier_from_me=CASE WHEN excluded.frontier_timestamp IS NOT NULL AND (history_import_chats.frontier_timestamp IS NULL OR excluded.frontier_timestamp<history_import_chats.frontier_timestamp) THEN excluded.frontier_from_me ELSE history_import_chats.frontier_from_me END,
          pages=MAX(history_import_chats.pages,excluded.pages),messages=MAX(history_import_chats.messages,excluded.messages),
          attempts=MAX(history_import_chats.attempts,excluded.attempts),retry_at=MAX(history_import_chats.retry_at,excluded.retry_at),
          state=CASE WHEN history_import_chats.state IN (?3,?4) THEN history_import_chats.state
            WHEN excluded.state IN (?3,?4) THEN excluded.state
            WHEN history_import_chats.state=?6 OR excluded.state=?6 THEN ?6
            WHEN history_import_chats.state=?5 THEN excluded.state ELSE history_import_chats.state END,
          reason=CASE WHEN history_import_chats.state IN (?3,?4) THEN history_import_chats.reason
            WHEN excluded.state IN (?3,?4) THEN excluded.reason
            WHEN history_import_chats.state=?6 OR excluded.state=?6 THEN ?7
            WHEN history_import_chats.state=?5 THEN excluded.reason ELSE history_import_chats.reason END,
          request_token=CASE WHEN excluded.request_token IS NOT NULL AND
            (history_import_chats.request_token IS NULL
             OR excluded.request_token>history_import_chats.request_token)
            THEN excluded.request_token ELSE history_import_chats.request_token END,
          request_session=CASE WHEN excluded.request_token IS NOT NULL AND
            (history_import_chats.request_token IS NULL
             OR excluded.request_token>history_import_chats.request_token)
            THEN excluded.request_session ELSE history_import_chats.request_session END,
          request_started=CASE WHEN excluded.request_token IS NOT NULL AND
            (history_import_chats.request_token IS NULL
             OR excluded.request_token>history_import_chats.request_token)
            THEN excluded.request_started ELSE history_import_chats.request_started END",
        params![
            old,
            new,
            CHAT_REACHED_CUTOFF,
            CHAT_PHONE_EXHAUSTED,
            CHAT_EXCLUDED,
            CHAT_STOPPED,
            REASON_RETRY_LIMIT_REACHED
        ],
    )?;
    transaction.execute(
        "UPDATE history_import_chats
         SET request_token=NULL,request_session=NULL,request_started=NULL
         WHERE chat=?1 AND state<>?2",
        params![new, CHAT_ACTIVE],
    )?;
    transaction.execute("DELETE FROM history_import_chats WHERE chat=?1", [old])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive::tests::message;

    #[test]
    fn import_ledger_is_local_to_each_account_archive() {
        let first = Archive::in_memory().unwrap();
        let second = Archive::in_memory().unwrap();
        first.ensure_chat("first", "Synthetic").unwrap();
        first
            .insert_message(&message("first", "anchor", 1000, false), None)
            .unwrap();
        first.configure_history_import(Some(500), false, 1).unwrap();
        first.reconcile_history_import(1).unwrap();

        assert_eq!(first.history_import_progress(1).unwrap().total, 1);
        assert_eq!(second.history_import_progress(1).unwrap().total, 0);
        assert!(second.next_history_import(1).unwrap().is_none());
    }

    #[test]
    fn unbound_lid_request_survives_canonicalization_and_can_bind() {
        let archive = Archive::in_memory().unwrap();
        archive.ensure_chat("9@lid", "Synthetic").unwrap();
        archive
            .insert_message(&message("9@lid", "anchor", 1000, false), None)
            .unwrap();
        archive
            .configure_history_import(Some(500), false, 1)
            .unwrap();
        archive.reconcile_history_import(1).unwrap();
        let request = archive.next_history_import(1).unwrap().unwrap();

        assert!(archive.put_lid("9", "1").unwrap());
        assert!(
            archive
                .bind_history_import_session(request.token, "mapped-session", 2)
                .unwrap()
        );
        assert_eq!(
            archive
                .history_import_request_for_session("mapped-session")
                .unwrap(),
            Some(("1@s.whatsapp.net".into(), request.token))
        );
    }

    #[test]
    fn bound_lid_session_survives_canonicalization_for_restart_lookup() {
        let archive = Archive::in_memory().unwrap();
        archive.ensure_chat("9@lid", "Synthetic").unwrap();
        archive
            .insert_message(&message("9@lid", "anchor", 1000, false), None)
            .unwrap();
        archive
            .configure_history_import(Some(500), false, 1)
            .unwrap();
        archive.reconcile_history_import(1).unwrap();
        let request = archive.next_history_import(1).unwrap().unwrap();
        assert!(
            archive
                .bind_history_import_session(request.token, "durable-session", 2)
                .unwrap()
        );

        assert!(archive.put_lid("9", "1").unwrap());
        assert_eq!(
            archive
                .history_import_request_for_session("durable-session")
                .unwrap(),
            Some(("1@s.whatsapp.net".into(), request.token))
        );
    }

    #[test]
    fn canonicalization_preserves_destination_and_prefers_the_newer_token() {
        let archive = Archive::in_memory().unwrap();
        for (chat, timestamp) in [("9@lid", 1000), ("1@s.whatsapp.net", 900)] {
            archive.ensure_chat(chat, "Synthetic").unwrap();
            archive
                .insert_message(&message(chat, "anchor", timestamp, false), None)
                .unwrap();
        }
        archive
            .configure_history_import(Some(500), false, 1)
            .unwrap();
        archive.reconcile_history_import(1).unwrap();

        archive
            .connection
            .execute(
                "UPDATE history_import_chats SET request_token=10,request_session='destination',
                 request_started=10 WHERE chat='1@s.whatsapp.net'",
                [],
            )
            .unwrap();
        assert!(archive.put_lid("9", "1").unwrap());
        assert_eq!(
            archive
                .history_import_request_for_session("destination")
                .unwrap(),
            Some(("1@s.whatsapp.net".into(), 10))
        );
        let destination: (i64, String, i64) = archive
            .connection
            .query_row(
                "SELECT request_token,request_session,request_started
                 FROM history_import_chats WHERE chat='1@s.whatsapp.net'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(destination, (10, "destination".into(), 10));

        let archive = Archive::in_memory().unwrap();
        for (chat, timestamp) in [("9@lid", 1000), ("1@s.whatsapp.net", 900)] {
            archive.ensure_chat(chat, "Synthetic").unwrap();
            archive
                .insert_message(&message(chat, "anchor", timestamp, false), None)
                .unwrap();
        }
        archive
            .configure_history_import(Some(500), false, 1)
            .unwrap();
        archive.reconcile_history_import(1).unwrap();
        archive
            .connection
            .execute(
                // Binding an older dispatch later resets its lease timestamp,
                // but cannot make it newer than a later reservation token.
                "UPDATE history_import_chats SET request_token=10,request_session='stale',
                 request_started=20 WHERE chat='1@s.whatsapp.net'",
                [],
            )
            .unwrap();
        archive
            .connection
            .execute(
                "UPDATE history_import_chats SET request_token=11,request_session='current',
                 request_started=11 WHERE chat='9@lid'",
                [],
            )
            .unwrap();

        assert!(archive.put_lid("9", "1").unwrap());
        assert!(
            archive
                .history_import_request_for_session("stale")
                .unwrap()
                .is_none()
        );
        assert_eq!(
            archive
                .history_import_request_for_session("current")
                .unwrap(),
            Some(("1@s.whatsapp.net".into(), 11))
        );
        let current: (i64, String, i64) = archive
            .connection
            .query_row(
                "SELECT request_token,request_session,request_started
                 FROM history_import_chats WHERE chat='1@s.whatsapp.net'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(current, (11, "current".into(), 11));
    }

    #[test]
    fn canonicalization_does_not_restore_a_request_on_a_completed_chat() {
        let archive = Archive::in_memory().unwrap();
        for chat in ["9@lid", "1@s.whatsapp.net"] {
            archive.ensure_chat(chat, "Synthetic").unwrap();
            archive
                .insert_message(&message(chat, "anchor", 1000, false), None)
                .unwrap();
        }
        archive
            .configure_history_import(Some(500), false, 1)
            .unwrap();
        archive.reconcile_history_import(1).unwrap();
        archive
            .connection
            .execute(
                "UPDATE history_import_chats SET state=?1,reason=?2
                 WHERE chat='1@s.whatsapp.net'",
                params![CHAT_REACHED_CUTOFF, REASON_REACHED_CUTOFF],
            )
            .unwrap();
        archive
            .connection
            .execute(
                "UPDATE history_import_chats SET request_token=10,request_session='late',
                 request_started=10 WHERE chat='9@lid'",
                [],
            )
            .unwrap();

        assert!(archive.put_lid("9", "1").unwrap());
        assert!(
            archive
                .history_import_request_for_session("late")
                .unwrap()
                .is_none()
        );
        let (state, token): (i64, Option<i64>) = archive
            .connection
            .query_row(
                "SELECT state,request_token FROM history_import_chats
                 WHERE chat='1@s.whatsapp.net'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(state, CHAT_REACHED_CUTOFF);
        assert_eq!(token, None);
        assert_eq!(
            archive
                .finish_history_import_page(10, None, 0, None, true, 2)
                .unwrap(),
            HistoryPageOutcome::Stale
        );
    }

    #[test]
    fn newest_anchor_and_correlated_page_crossing_cutoff_are_required() {
        let archive = Archive::in_memory().unwrap();
        archive.ensure_chat("fixture", "Synthetic").unwrap();
        archive
            .insert_message(&message("fixture", "old-island", 100, false), None)
            .unwrap();
        archive
            .insert_message(&message("fixture", "newest", 1000, false), None)
            .unwrap();
        archive
            .configure_history_import(Some(500), false, 1)
            .unwrap();
        archive.reconcile_history_import(1).unwrap();
        let request = archive.next_history_import(1).unwrap().unwrap();
        assert_eq!((request.timestamp, request.id.as_str()), (1000, "newest"));
        archive
            .finish_history_import_page(
                request.token,
                Some((100, "old-island".into(), false)),
                50,
                None,
                true,
                2,
            )
            .unwrap();
        assert_eq!(archive.history_import_progress(2).unwrap().completed, 0);
        assert!(
            archive
                .bind_history_import_session(request.token, "synthetic-session", 2)
                .unwrap()
        );
        archive
            .finish_history_import_page(
                request.token,
                Some((400, "page-oldest".into(), true)),
                50,
                None,
                true,
                2,
            )
            .unwrap();
        assert_eq!(archive.history_import_progress(2).unwrap().completed, 1);
    }

    #[test]
    fn progress_exposes_job_failure_metadata_without_archive_content() {
        let archive = Archive::in_memory().unwrap();
        archive.ensure_chat("fixture", "Synthetic").unwrap();
        archive
            .insert_message(&message("fixture", "anchor", 1000, false), None)
            .unwrap();
        archive
            .configure_history_import(Some(500), false, 1)
            .unwrap();
        archive.reconcile_history_import(1).unwrap();
        let request = archive.next_history_import(1).unwrap().unwrap();
        let anchors = archive.history_import_anchor_metadata().unwrap();
        assert_eq!(
            (anchors.active, anchors.reserved, anchors.direct_pn),
            (1, 1, 0)
        );
        assert_eq!(anchors.reserved_kind, Some(HistoryAnchorKind::Other));
        assert!(!anchors.reserved_empty);
        archive
            .fail_history_import(request.token, HistoryImportFailure::Send, 90)
            .unwrap();
        let progress = archive.history_import_progress(2).unwrap();
        assert_eq!(progress.failure_streak, 1);
        assert_eq!(progress.reason, Some(ChatImportReason::RetrySend));
        assert_eq!(progress.retry_at, Some(90));
    }

    #[test]
    fn anchor_metadata_reports_only_aggregate_protocol_shapes() {
        let archive = Archive::in_memory().unwrap();
        for chat in ["1@s.whatsapp.net", "2@lid", "3@g.us", "other", "empty@g.us"] {
            archive.ensure_chat(chat, "Synthetic").unwrap();
            if chat != "empty@g.us" {
                archive
                    .insert_message(&message(chat, "anchor", 1000, false), None)
                    .unwrap();
            }
        }
        archive
            .connection
            .execute("UPDATE chats SET archived=1 WHERE id='3@g.us'", [])
            .unwrap();
        archive
            .configure_history_import(Some(500), false, 1)
            .unwrap();
        archive.reconcile_history_import(1).unwrap();
        let metadata = archive.history_import_anchor_metadata().unwrap();
        assert_eq!(metadata.active, 5);
        assert_eq!((metadata.direct_pn, metadata.direct_lid), (1, 1));
        assert_eq!((metadata.groups, metadata.other), (2, 1));
        assert_eq!((metadata.empty_anchor, metadata.nonempty_anchor), (1, 4));
        assert_eq!(metadata.archived, 1);
    }

    #[test]
    fn uncertain_dispatch_pause_survives_restart_and_releases_only_unbound_request() {
        let archive = Archive::in_memory().unwrap();
        archive.ensure_chat("fixture", "Synthetic").unwrap();
        archive
            .insert_message(&message("fixture", "anchor", 1000, false), None)
            .unwrap();
        archive
            .configure_history_import(Some(500), false, 1)
            .unwrap();
        archive.reconcile_history_import(1).unwrap();
        let request = archive.next_history_import(1).unwrap().unwrap();
        assert!(
            archive
                .pause_history_import_request(
                    request.token,
                    HistoryImportFailure::CorrelationUnavailable,
                    2,
                )
                .unwrap()
        );
        let paused = archive.history_import_progress(2).unwrap();
        assert_eq!(paused.state, ImportRunState::NeedsAttention);
        assert_eq!(paused.failure_streak, 1);
        assert_eq!(
            paused.reason,
            Some(ChatImportReason::CorrelationUnavailable)
        );
        assert_eq!(
            archive
                .history_import_request_for_session("not-bound")
                .unwrap(),
            None
        );
        assert!(archive.next_history_import(2).unwrap().is_none());
        assert_eq!(
            archive
                .release_uncertain_history_import_requests(3)
                .unwrap(),
            1
        );
        assert_eq!(
            archive
                .release_uncertain_history_import_requests(3)
                .unwrap(),
            0
        );
        assert!(
            !archive
                .release_history_import_request(request.token, HistoryImportFailure::Send, 3)
                .unwrap()
        );
        archive.resume_history_import(3).unwrap();
        assert!(archive.next_history_import(3).unwrap().unwrap().token > request.token);
    }

    #[test]
    fn restart_persists_frontier_and_late_chat_reopens_finished_job() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("archive.db");
        let archive = Archive::open_with_key(&path, &[9; 32]).unwrap();
        archive.ensure_chat("first", "Synthetic").unwrap();
        archive
            .insert_message(&message("first", "new", 1000, false), None)
            .unwrap();
        archive
            .configure_history_import(Some(500), false, 1)
            .unwrap();
        archive.reconcile_history_import(1).unwrap();
        let request = archive.next_history_import(1).unwrap().unwrap();
        archive
            .bind_history_import_session(request.token, "session", 2)
            .unwrap();
        archive
            .finish_history_import_page(
                request.token,
                Some((400, "older".into(), false)),
                1,
                None,
                true,
                2,
            )
            .unwrap();
        drop(archive);
        let archive = Archive::open_with_key(&path, &[9; 32]).unwrap();
        assert_eq!(archive.history_import_progress(3).unwrap().completed, 1);
        archive.ensure_chat("late", "Late synthetic").unwrap();
        archive
            .insert_message(&message("late", "late-new", 900, false), None)
            .unwrap();
        archive.reconcile_history_import(3).unwrap();
        let progress = archive.history_import_progress(3).unwrap();
        assert_eq!((progress.completed, progress.total), (1, 2));
        assert_eq!(
            archive.next_history_import(3).unwrap().unwrap().chat,
            "late"
        );
    }

    #[test]
    fn late_anchor_and_unlock_reopen_without_using_legacy_end_as_completion() {
        let archive = Archive::in_memory().unwrap();
        archive.ensure_chat("waiting", "Synthetic").unwrap();
        let mut local = message("waiting", "local", 1000, true);
        local.status = crate::model::Delivery::Pending;
        archive.insert_message(&local, None).unwrap();
        let mut failed = message("waiting", "failed", 1100, true);
        failed.status = crate::model::Delivery::Failed;
        archive.insert_message(&failed, None).unwrap();
        archive
            .configure_history_import(Some(500), false, 1)
            .unwrap();
        archive.reconcile_history_import(1).unwrap();
        assert!(archive.next_history_import(1).unwrap().is_none());
        archive
            .insert_message(&message("waiting", "safe", 900, false), None)
            .unwrap();
        archive.reconcile_history_import(2).unwrap();
        assert_eq!(archive.next_history_import(2).unwrap().unwrap().id, "safe");

        archive.ensure_chat("locked", "Synthetic").unwrap();
        archive
            .insert_message(&message("locked", "newest", 800, false), None)
            .unwrap();
        archive.set_locked("locked", true).unwrap();
        archive
            .configure_history_import(Some(400), false, 3)
            .unwrap();
        archive.reconcile_history_import(3).unwrap();
        archive.set_locked("locked", false).unwrap();
        archive
            .connection
            .execute("UPDATE chats SET history_start=1 WHERE id='locked'", [])
            .unwrap();
        archive.reconcile_history_import(4).unwrap();
        assert_eq!(
            archive.history_import_progress(4).unwrap().state,
            ImportRunState::Running
        );
        assert!(
            archive
                .history_import_progress(4)
                .unwrap()
                .chats
                .iter()
                .any(|row| row.chat == "locked")
        );
    }

    #[test]
    fn replacement_never_reuses_tokens_and_policy_resends_preserve_terminal_states() {
        let archive = Archive::in_memory().unwrap();
        archive.ensure_chat("fixture", "Synthetic").unwrap();
        archive
            .insert_message(&message("fixture", "newest", 1000, false), None)
            .unwrap();
        archive
            .configure_history_import(Some(500), false, 1)
            .unwrap();
        archive.reconcile_history_import(1).unwrap();
        let stale = archive.next_history_import(1).unwrap().unwrap();
        archive.configure_history_import(None, false, 2).unwrap();
        archive
            .configure_history_import(Some(400), false, 2)
            .unwrap();
        archive.reconcile_history_import(2).unwrap();
        let current = archive.next_history_import(2).unwrap().unwrap();
        assert!(current.token > stale.token);
        assert!(
            !archive
                .bind_history_import_session(stale.token, "stale", 2)
                .unwrap()
        );
        archive
            .bind_history_import_session(current.token, "current", 2)
            .unwrap();
        archive
            .finish_history_import_page(
                current.token,
                Some((300, "older".into(), false)),
                1,
                None,
                true,
                3,
            )
            .unwrap();
        archive
            .configure_history_import(Some(400), false, 4)
            .unwrap();
        assert_eq!(
            archive.history_import_progress(4).unwrap().state,
            ImportRunState::Finished
        );
        archive
            .pause_history_import(HistoryImportFailure::RetryLimitReached, 5)
            .unwrap();
        archive
            .configure_history_import(Some(400), false, 6)
            .unwrap();
        assert_eq!(
            archive.history_import_progress(6).unwrap().state,
            ImportRunState::NeedsAttention
        );
    }

    #[test]
    fn session_correlation_survives_restart_but_not_job_replacement() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("archive.db");
        let archive = Archive::open_with_key(&path, &[8; 32]).unwrap();
        archive.ensure_chat("fixture", "Synthetic").unwrap();
        archive
            .insert_message(&message("fixture", "newest", 1000, false), None)
            .unwrap();
        archive
            .configure_history_import(Some(500), false, 1)
            .unwrap();
        archive.reconcile_history_import(1).unwrap();
        let request = archive.next_history_import(1).unwrap().unwrap();
        archive
            .bind_history_import_session(request.token, "durable-session", 2)
            .unwrap();
        drop(archive);
        let archive = Archive::open_with_key(&path, &[8; 32]).unwrap();
        assert_eq!(
            archive
                .history_import_request_for_session("durable-session")
                .unwrap(),
            Some(("fixture".into(), request.token))
        );
        archive
            .configure_history_import(Some(400), false, 2)
            .unwrap();
        assert_eq!(
            archive
                .history_import_request_for_session("durable-session")
                .unwrap(),
            None
        );
    }

    #[test]
    fn sender_direction_advances_and_same_second_frontiers_cannot_cycle() {
        let archive = Archive::in_memory().unwrap();
        archive.ensure_chat("fixture", "Synthetic").unwrap();
        archive
            .insert_message(&message("fixture", "start", 1000, false), None)
            .unwrap();
        archive
            .configure_history_import(Some(500), false, 1)
            .unwrap();
        archive.reconcile_history_import(1).unwrap();
        let first = archive.next_history_import(1).unwrap().unwrap();
        archive
            .bind_history_import_session(first.token, "one", 2)
            .unwrap();
        archive
            .finish_history_import_page(
                first.token,
                Some((900, "same-a".into(), true)),
                1,
                None,
                true,
                2,
            )
            .unwrap();
        let second = archive.next_history_import(2).unwrap().unwrap();
        assert!(second.from_me);
        archive
            .bind_history_import_session(second.token, "two", 3)
            .unwrap();
        archive
            .finish_history_import_page(
                second.token,
                Some((900, "same-b".into(), false)),
                1,
                None,
                true,
                3,
            )
            .unwrap();
        let third = archive.next_history_import(3).unwrap().unwrap();
        assert!(!third.from_me);
        archive
            .bind_history_import_session(third.token, "three", 4)
            .unwrap();
        archive
            .finish_history_import_page(
                third.token,
                Some((900, "same-a".into(), true)),
                1,
                None,
                true,
                4,
            )
            .unwrap();
        assert_eq!(
            archive.history_import_progress(4).unwrap().chats[0].reason,
            ChatImportReason::RetryNoProgress
        );
    }

    #[test]
    fn silent_chats_do_not_block_the_next_runnable_chat_or_global_budget() {
        let archive = Archive::in_memory().unwrap();
        for index in 0..8 {
            let chat = format!("silent-{index}");
            archive.ensure_chat(&chat, "Synthetic").unwrap();
            archive
                .insert_message(
                    &message(&chat, &format!("anchor-{index}"), 1_000 - index, false),
                    None,
                )
                .unwrap();
        }
        archive
            .configure_history_import(Some(500), false, 1)
            .unwrap();
        archive.reconcile_history_import(1).unwrap();

        for now in 1..=7 {
            let request = archive.next_history_import(now).unwrap().unwrap();
            assert_eq!(
                archive
                    .fail_history_import_chat(request.token, HistoryImportFailure::Timeout, now)
                    .unwrap(),
                HistoryChatFailureOutcome::RetryScheduled
            );
        }

        let next = archive.next_history_import(8).unwrap().unwrap();
        assert_eq!(next.chat, "silent-7");
        let progress = archive.history_import_progress(8).unwrap();
        assert_eq!(progress.state, ImportRunState::Running);
        assert_eq!(progress.failure_streak, 0);
        assert_eq!(progress.retry_at, None);
    }

    #[test]
    fn all_cooling_chats_report_the_earliest_retry_without_global_backoff() {
        let archive = Archive::in_memory().unwrap();
        for chat in ["first", "second"] {
            archive.ensure_chat(chat, "Synthetic").unwrap();
            archive
                .insert_message(&message(chat, "anchor", 1_000, false), None)
                .unwrap();
        }
        archive
            .configure_history_import(Some(500), false, 1)
            .unwrap();
        archive.reconcile_history_import(1).unwrap();
        for _ in 0..2 {
            let request = archive.next_history_import(1).unwrap().unwrap();
            archive
                .fail_history_import_chat(request.token, HistoryImportFailure::Timeout, 1)
                .unwrap();
        }

        let cooling = archive.history_import_progress(1).unwrap();
        assert_eq!(cooling.state, ImportRunState::BackingOff);
        assert_eq!(cooling.retry_at, Some(61));
        assert_eq!(cooling.reason, Some(ChatImportReason::RetryTimeout));
        assert_eq!(cooling.failure_streak, 0);
        assert!(archive.next_history_import(1).unwrap().is_none());
    }

    #[test]
    fn cooling_no_progress_reason_is_derived_without_globalizing_it() {
        let archive = Archive::in_memory().unwrap();
        archive.ensure_chat("fixture", "Synthetic").unwrap();
        archive
            .insert_message(&message("fixture", "anchor", 1_000, false), None)
            .unwrap();
        archive
            .configure_history_import(Some(500), false, 1)
            .unwrap();
        archive.reconcile_history_import(1).unwrap();
        let request = archive.next_history_import(1).unwrap().unwrap();
        archive
            .fail_history_import_chat(request.token, HistoryImportFailure::NoProgress, 1)
            .unwrap();

        let cooling = archive.history_import_progress(1).unwrap();
        assert_eq!(cooling.state, ImportRunState::BackingOff);
        assert_eq!(cooling.reason, Some(ChatImportReason::RetryNoProgress));
        assert_eq!(cooling.failure_streak, 0);
        let stored_job_reason: Option<i64> = archive
            .connection
            .query_row(
                "SELECT reason FROM history_import_job WHERE id=1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(stored_job_reason, None);
    }

    #[test]
    fn deferred_and_unsupported_reservations_have_local_exact_token_outcomes() {
        let archive = Archive::in_memory().unwrap();
        for chat in ["deferred", "unsupported"] {
            archive.ensure_chat(chat, "Synthetic").unwrap();
            archive
                .insert_message(&message(chat, "anchor", 1_000, false), None)
                .unwrap();
        }
        archive
            .configure_history_import(Some(500), false, 1)
            .unwrap();
        archive.reconcile_history_import(1).unwrap();
        let deferred = archive.next_history_import(1).unwrap().unwrap();
        assert!(
            archive
                .release_deferred_history_import_request(deferred.token, 1)
                .unwrap()
        );
        assert!(
            !archive
                .release_deferred_history_import_request(deferred.token, 1)
                .unwrap()
        );
        let unsupported = archive.next_history_import(1).unwrap().unwrap();
        assert_ne!(unsupported.chat, deferred.chat);
        assert!(
            archive
                .exclude_unsupported_history_import_request(unsupported.token, 1)
                .unwrap()
        );
        archive.reconcile_history_import(2).unwrap();
        let progress = archive.history_import_progress(2).unwrap();
        assert_eq!(progress.excluded, 1);
        assert_eq!(progress.failure_streak, 0);
    }

    #[test]
    fn bounded_silence_stops_only_that_chat_and_resume_reactivates_it() {
        let archive = Archive::in_memory().unwrap();
        archive.ensure_chat("silent", "Synthetic").unwrap();
        archive
            .insert_message(&message("silent", "anchor", 1_000, false), None)
            .unwrap();
        archive
            .configure_history_import(Some(500), false, 1)
            .unwrap();
        archive.reconcile_history_import(1).unwrap();

        for (now, expected) in [
            (1, HistoryChatFailureOutcome::RetryScheduled),
            (61, HistoryChatFailureOutcome::RetryScheduled),
            (181, HistoryChatFailureOutcome::ChatStopped),
        ] {
            let request = archive.next_history_import(now).unwrap().unwrap();
            assert_eq!(
                archive
                    .fail_history_import_chat(request.token, HistoryImportFailure::NoProgress, now)
                    .unwrap(),
                expected
            );
        }

        let stopped = archive.history_import_progress(181).unwrap();
        assert_eq!(stopped.state, ImportRunState::NeedsAttention);
        assert_eq!(stopped.reason, Some(ChatImportReason::RetryLimitReached));
        assert_eq!(stopped.completed, 0);
        assert_eq!(stopped.chats[0].reason, ChatImportReason::RetryLimitReached);
        assert!(archive.next_history_import(10_000).unwrap().is_none());

        archive.resume_history_import(10_001).unwrap();
        let resumed = archive.history_import_progress(10_001).unwrap();
        assert_eq!(resumed.state, ImportRunState::Running);
        assert_eq!(resumed.chats[0].reason, ChatImportReason::Queued);
        assert!(archive.next_history_import(10_001).unwrap().is_some());
    }

    #[test]
    fn overall_needs_attention_only_after_every_chat_is_stopped() {
        let archive = Archive::in_memory().unwrap();
        for chat in ["first", "second"] {
            archive.ensure_chat(chat, "Synthetic").unwrap();
            archive
                .insert_message(&message(chat, "anchor", 1_000, false), None)
                .unwrap();
        }
        archive
            .configure_history_import(Some(500), false, 1)
            .unwrap();
        archive.reconcile_history_import(1).unwrap();

        for round in 0..3 {
            let now = 1 + round * 120;
            for expected_chat in ["first", "second"] {
                let request = archive.next_history_import(now).unwrap().unwrap();
                assert_eq!(request.chat, expected_chat);
                let outcome = archive
                    .fail_history_import_chat(request.token, HistoryImportFailure::Timeout, now)
                    .unwrap();
                if round < 2 {
                    assert_eq!(outcome, HistoryChatFailureOutcome::RetryScheduled);
                    let state = archive.history_import_progress(now).unwrap().state;
                    assert_eq!(
                        state,
                        if expected_chat == "first" {
                            ImportRunState::Running
                        } else {
                            ImportRunState::BackingOff
                        }
                    );
                }
            }
        }
        assert_eq!(
            archive.history_import_progress(241).unwrap().state,
            ImportRunState::NeedsAttention
        );
    }
}
