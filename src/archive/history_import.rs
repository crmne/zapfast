//! One resumable, date-scoped request pass, kept inside its account's archive.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};

use super::{Archive, Result};
use crate::history_retry::HistoryRetry;
use crate::model::{Chat, ChatKind, HistoryImportProgress, Message};

const KEY: &str = "history_import_v1";
const GENERATION: &str = "history_import_generation";
const NEXT_REQUEST: &str = "history_import_next_request";

/// A stored message the phone can use as a history boundary.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Anchor {
    pub id: String,
    pub at: i64,
    pub from_me: bool,
    pub order: Option<i64>,
}

impl Anchor {
    /// Copies only the stored fields needed to anchor the next phone request.
    pub fn of(message: &Message) -> Self {
        Self {
            id: message.id.clone(),
            at: message.timestamp,
            from_me: message.from_me,
            order: message.history_order,
        }
    }
}

/// Only successfully stored rows from this response may advance its cursor.
#[derive(Clone, Debug, Default)]
pub(crate) struct Page {
    pub oldest: Option<Anchor>,
    pub exhausted: bool,
    pub failed: bool,
    pub malformed: bool,
    pub ambiguous: bool,
}

impl Page {
    /// Tracks the oldest saved row and flags timestamp ties without reliable ordering.
    pub fn observe(&mut self, message: &Message) {
        if message.timestamp <= 0 {
            return;
        }
        let anchor = Anchor::of(message);
        match self.oldest.as_ref() {
            None => self.oldest = Some(anchor),
            Some(old) if anchor.at < old.at => {
                self.oldest = Some(anchor);
                self.ambiguous = false;
            }
            Some(old) if anchor.at == old.at => {
                if anchor.id != old.id && (anchor.order.is_none() || old.order.is_none()) {
                    self.ambiguous = true;
                }
                if (anchor.order.unwrap_or(i64::MAX), &anchor.id)
                    < (old.order.unwrap_or(i64::MAX), &old.id)
                {
                    self.oldest = Some(anchor);
                }
            }
            _ => {}
        }
    }
}

/// A callback is valid only for the job and attempt that reserved it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Token {
    generation: i64,
    sequence: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Cursor {
    before: Anchor,
    visited: BTreeSet<(i64, String)>,
    activity: i64,
    round: u64,
    #[serde(flatten)]
    retry: HistoryRetry<i64>,
    done: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Flight {
    token: Token,
    chat: String,
    before: Anchor,
}

#[derive(Clone, Debug)]
pub(crate) struct Request {
    pub token: Token,
    pub chat: String,
    pub before: Anchor,
}

/// This is request progress, never a claim that all WhatsApp data was delivered.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Job {
    version: u8,
    generation: i64,
    pub since: i64,
    paused: bool,
    finished: bool,
    failed: bool,
    discovered: bool,
    excluded: usize,
    sequence: u64,
    send_failures: u32,
    next_request: i64,
    chats: BTreeMap<String, Cursor>,
    flight: Option<Flight>,
}

impl Job {
    /// Reports completed, excluded and stopped work without promising complete history.
    pub fn progress(&self) -> HistoryImportProgress {
        HistoryImportProgress {
            since: Some(self.since),
            total: self.chats.len() + self.excluded,
            completed: self.chats.values().filter(|chat| chat.done).count(),
            excluded: self.excluded,
            stopped: self
                .chats
                .values()
                .filter(|chat| !chat.done && chat.retry.stopped())
                .count(),
            paused: self.paused,
            finished: self.finished,
            failed: self.failed,
        }
    }

    /// Uses the same runnable-work predicate as the background-prefetch admission gate.
    pub fn active(&self) -> bool {
        self.progress().active()
    }

    /// Changes user pause state without discarding cursors or completed chats.
    pub fn pause(&mut self, paused: bool) {
        self.paused = paused;
    }

    /// Reopens unfinished chats while preserving completed work and account pacing.
    pub fn retry(&mut self) {
        self.paused = false;
        self.failed = false;
        for cursor in self.chats.values_mut().filter(|cursor| !cursor.done) {
            cursor.retry = HistoryRetry::new(0);
        }
    }

    /// Restart retries the saved frontier; replies from the previous process
    /// may be archived, but cannot acknowledge an attempt in this process.
    pub fn recover(&mut self) {
        self.flight = None;
    }

    /// Reserves the next eligible chat fairly and fences its callbacks with a new token.
    pub fn reserve(&mut self, now: i64) -> Result<Option<Request>> {
        if !self.active() || self.flight.is_some() || now < self.next_request {
            return Ok(None);
        }
        let Some(chat) = self
            .chats
            .iter()
            .filter(|(_, cursor)| {
                !cursor.done && !cursor.retry.stopped() && cursor.retry.due <= now
            })
            .min_by_key(|(id, cursor)| (cursor.round, std::cmp::Reverse(cursor.activity), *id))
            .map(|(id, _)| id.clone())
        else {
            return Ok(None);
        };
        self.sequence = self
            .sequence
            .checked_add(1)
            .ok_or(rusqlite::Error::InvalidQuery)?;
        let cursor = self.chats.get_mut(&chat).expect("selected cursor");
        cursor.round = self.sequence;
        let token = Token {
            generation: self.generation,
            sequence: self.sequence,
        };
        let before = cursor.before.clone();
        self.flight = Some(Flight {
            token,
            chat: chat.clone(),
            before: before.clone(),
        });
        self.next_request = now + 20;
        Ok(Some(Request {
            token,
            chat,
            before,
        }))
    }

    /// Checks both attempt identity and its unchanged canonical chat boundary.
    fn owns(&self, token: Token, chat: &str) -> bool {
        self.flight.as_ref().is_some_and(|flight| {
            flight.token == token
                && flight.chat == chat
                && self
                    .chats
                    .get(chat)
                    .is_some_and(|cursor| cursor.before == flight.before)
        })
    }

    /// Releases this attempt and applies account transport or per-chat response backoff.
    pub fn fail(&mut self, token: Token, chat: &str, now: i64, transport: bool) {
        if !self.owns(token, chat) {
            return;
        }
        self.flight = None;
        if transport {
            self.send_failures = self.send_failures.saturating_add(1);
            self.next_request = now + (30_i64 << (self.send_failures - 1).min(5)).min(900);
        } else {
            let cursor = self.chats.get_mut(chat).expect("owned cursor");
            cursor.retry.due = now + cursor.retry.fail().as_secs() as i64;
            self.next_request = now + 20;
        }
    }

    /// Advances only an owned, safely stored page toward the inclusive date boundary.
    pub fn complete(&mut self, token: Token, chat: &str, page: &Page, now: i64) {
        if !self.owns(token, chat) {
            return;
        }
        if page.failed {
            self.flight = None;
            self.failed = true;
            self.paused = true;
            return;
        }
        if page.malformed {
            self.fail(token, chat, now, false);
            return;
        }
        let cursor = &self.chats[chat];
        // A UTC date is inclusive: finish only after crossing before midnight,
        // so messages sharing the cutoff's second are not skipped.
        let reached = page
            .oldest
            .as_ref()
            .is_some_and(|anchor| anchor.at < self.since);
        let progress = page.oldest.as_ref().is_some_and(|anchor| {
            anchor.at <= cursor.before.at
                && anchor.id != cursor.before.id
                && !cursor.visited.contains(&(anchor.at, anchor.id.clone()))
        });
        if !page.exhausted && !reached && (!progress || page.ambiguous) {
            self.fail(token, chat, now, false);
            return;
        }
        self.flight = None;
        self.send_failures = 0;
        self.next_request = now + 20;
        let cursor = self.chats.get_mut(chat).expect("owned cursor");
        cursor.retry = HistoryRetry::new(0);
        cursor.done = reached || page.exhausted;
        if let Some(anchor) = &page.oldest {
            cursor.visited.insert((anchor.at, anchor.id.clone()));
            cursor.before = anchor.clone();
        }
        self.finished = self.discovered && self.chats.values().all(|cursor| cursor.done);
    }
}

/// Rejects invalid or unsupported job data without exposing its private values.
fn decode(text: &str) -> Result<Job> {
    // A caller may log this error. Never include values from the private job.
    let job: Job = serde_json::from_str(text).map_err(|_| rusqlite::Error::InvalidQuery)?;
    if job.version != 1 {
        return Err(rusqlite::Error::InvalidQuery);
    }
    Ok(job)
}

/// Reads the account job, distinguishing absence from corrupt or unreadable progress.
fn load(connection: &Connection) -> Result<Option<Job>> {
    let text: Option<String> = connection
        .query_row("SELECT value FROM meta WHERE key=?1", [KEY], |row| {
            row.get(0)
        })
        .optional()?;
    text.as_deref().map(decode).transpose()
}

/// Writes job JSON through the supplied connection or enclosing identity transaction.
fn save(connection: &Connection, job: &Job) -> Result<()> {
    let text = serde_json::to_string(job)
        .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
    connection.execute("INSERT INTO meta(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![KEY, text])?;
    Ok(())
}

impl Archive {
    /// Loads this archive account's saved import without silently replacing errors.
    pub(crate) fn history_import(&self) -> Result<Option<Job>> {
        load(&self.connection)
    }

    /// Atomically saves progress and retains the longest reserved account cooldown.
    pub(crate) fn save_history_import(&self, job: &Job) -> Result<()> {
        let transaction = self.connection.unchecked_transaction()?;
        save(&transaction, job)?;
        transaction.execute("INSERT INTO meta(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=MAX(CAST(meta.value AS INTEGER),CAST(excluded.value AS INTEGER))", params![NEXT_REQUEST, job.next_request.to_string()])?;
        transaction.commit()
    }

    /// Validates a new date and durably replaces the job with a fresh generation.
    pub(crate) fn start_history_import(&self, since: i64, now: i64) -> Result<()> {
        if !crate::model::valid_history_import_date(since, now) {
            return Err(rusqlite::Error::InvalidQuery);
        }
        let transaction = self.connection.unchecked_transaction()?;
        let generation: i64 = transaction
            .query_row(
                "SELECT CAST(value AS INTEGER) FROM meta WHERE key=?1",
                [GENERATION],
                |row| row.get(0),
            )
            .optional()?
            .unwrap_or(0);
        let generation = generation
            .checked_add(1)
            .ok_or(rusqlite::Error::InvalidQuery)?;
        let due: i64 = transaction
            .query_row(
                "SELECT CAST(value AS INTEGER) FROM meta WHERE key=?1",
                [NEXT_REQUEST],
                |row| row.get(0),
            )
            .optional()?
            .unwrap_or(0);
        transaction.execute("INSERT INTO meta(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![GENERATION, generation.to_string()])?;
        save(
            &transaction,
            &Job {
                version: 1,
                generation,
                since,
                paused: false,
                finished: false,
                failed: false,
                discovered: false,
                excluded: 0,
                sequence: 0,
                send_failures: 0,
                next_request: due.max(now),
                chats: BTreeMap::new(),
                flight: None,
            },
        )?;
        transaction.commit()
    }

    /// Refresh only when chat metadata changes. Starting at the newest stored
    /// message checks the window instead of assuming old local rows prove no gaps.
    pub(crate) fn discover_history_import(
        &self,
        job: &mut Job,
        chats: &[Chat],
        now: i64,
    ) -> Result<()> {
        if job.finished {
            return Ok(());
        }
        let removals: HashSet<String> = self
            .connection
            .prepare("SELECT chat FROM chat_removals")?
            .query_map([], |row| row.get(0))?
            .collect::<Result<_>>()?;
        let eligible: Vec<_> = chats
            .iter()
            .filter(|chat| {
                !chat.locked
                    && !chat.left
                    && ChatKind::from_id(&chat.id) != ChatKind::Broadcast
                    && !removals.contains(&chat.id)
            })
            .collect();
        job.excluded = chats.len() - eligible.len();
        let ids: HashSet<_> = eligible.iter().map(|chat| chat.id.as_str()).collect();
        job.chats.retain(|id, _| ids.contains(id.as_str()));
        if job
            .flight
            .as_ref()
            .is_some_and(|flight| !ids.contains(flight.chat.as_str()))
        {
            job.flight = None;
        }
        for chat in eligible {
            if let Some(cursor) = job.chats.get_mut(&chat.id) {
                cursor.activity = chat.last_activity;
                continue;
            }
            let newest = self.connection.query_row(
                "SELECT id,timestamp,from_me,history_order FROM messages WHERE chat=?1 ORDER BY timestamp DESC,COALESCE(history_order,9223372036854775807) DESC,rowid DESC LIMIT 1", [&chat.id],
                |row| Ok(Anchor { id: row.get(0)?, at: row.get(1)?, from_me: row.get(2)?, order: row.get(3)? })).optional()?;
            let done = newest
                .as_ref()
                .is_some_and(|anchor| anchor.at < job.since && chat.last_activity < job.since);
            let before = newest
                .filter(|anchor| anchor.at > 0 && anchor.at >= chat.last_activity)
                .unwrap_or(Anchor {
                    id: String::new(),
                    at: now,
                    from_me: false,
                    order: None,
                });
            let visited = BTreeSet::from([(before.at, before.id.clone())]);
            job.chats.insert(
                chat.id.clone(),
                Cursor {
                    before,
                    visited,
                    activity: chat.last_activity,
                    round: 0,
                    retry: HistoryRetry::new(0),
                    done,
                },
            );
        }
        job.discovered |= !chats.is_empty();
        job.finished = job.discovered && job.chats.values().all(|cursor| cursor.done);
        Ok(())
    }
}

/// Called only by the existing identity-merge transaction in reconcile.rs.
pub(super) fn merge_alias(transaction: &Transaction<'_>, source: &str, target: &str) -> Result<()> {
    let Some(mut job) = load(transaction)? else {
        return Ok(());
    };
    if let Some(source_cursor) = job.chats.remove(source) {
        match job.chats.get(target) {
            Some(target_cursor) if target_cursor.before.at >= source_cursor.before.at => {}
            _ => {
                job.chats.insert(target.to_owned(), source_cursor);
            }
        }
        // Taking the newer frontier may repeat pages, but cannot skip a gap.
        if let Some(cursor) = job.chats.get_mut(target) {
            cursor.done = false;
        }
        job.finished = false;
    }
    if let Some(flight) = &mut job.flight {
        if flight.chat == source {
            flight.chat = target.to_owned();
        }
        if !job
            .chats
            .get(&flight.chat)
            .is_some_and(|cursor| cursor.before == flight.before)
        {
            job.flight = None;
        }
    }
    save(transaction, &job)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive::tests::message;

    /// Reusing the background retry state preserves saved import retry deadlines.
    #[test]
    fn shared_retry_state_reads_existing_saved_counters() {
        let retry: HistoryRetry<i64> =
            serde_json::from_str(r#"{"failures":2,"retry_at":1800000060}"#).unwrap();
        assert!(!retry.stopped());
        let mut restored = retry.clone();
        assert_eq!(restored.fail().as_secs(), 120);
        assert!(restored.stopped());
        assert_eq!(retry.due, 1_800_000_060);
    }

    const NOW: i64 = 1_800_000_000;
    const SINCE: i64 = 1_790_035_200;
    const CHAT: &str = "1@s.whatsapp.net";

    /// Builds an in-memory account with one synthetic chat and a discoverable job.
    fn fixture() -> (Archive, Job) {
        let archive = Archive::in_memory().unwrap();
        archive.ensure_chat(CHAT, "Fixture").unwrap();
        archive
            .insert_message(&message(CHAT, "newest", NOW - 100, false), None)
            .unwrap();
        archive.start_history_import(SINCE, NOW).unwrap();
        let mut job = archive.history_import().unwrap().unwrap();
        archive
            .discover_history_import(&mut job, &archive.chats().unwrap(), NOW)
            .unwrap();
        (archive, job)
    }

    /// Builds response metadata from one synthetic successfully stored message.
    fn page(id: &str, at: i64) -> Page {
        let mut page = Page::default();
        page.observe(&message(CHAT, id, at, false));
        page
    }

    /// Rejects malformed, future and out-of-window dates and checks inclusive limits.
    #[test]
    fn date_import_validates_exact_utc_dates() {
        let now = crate::model::parse_history_import_date("2026-10-24", 1_800_000_000).unwrap();
        assert_eq!(now % 86_400, 0);
        for text in [
            "2026-2-01",
            "2026-02-30",
            "2020-01-01",
            "2028-01-01",
            " 2026-10-24",
        ] {
            assert!(
                crate::model::parse_history_import_date(text, 1_800_000_000).is_none(),
                "{text}"
            );
        }
        assert!(crate::model::valid_history_import_date(
            now - 1_095 * 86_400,
            now
        ));
        assert!(!crate::model::valid_history_import_date(
            now - 1_096 * 86_400,
            now
        ));
    }

    /// Requires crossing before midnight and refuses to treat an empty response as completion.
    #[test]
    fn cutoff_is_inclusive_and_empty_is_not_exhaustion() {
        let (_, mut job) = fixture();
        let request = job.reserve(NOW).unwrap().unwrap();
        job.complete(request.token, CHAT, &page("midnight", SINCE), NOW);
        assert!(!job.progress().finished);
        let request = job.reserve(NOW + 20).unwrap().unwrap();
        job.complete(request.token, CHAT, &Page::default(), NOW + 20);
        assert!(!job.progress().finished);
        let request = job.reserve(NOW + 50).unwrap().unwrap();
        job.complete(
            request.token,
            CHAT,
            &page("previous-day", SINCE - 1),
            NOW + 50,
        );
        assert!(job.progress().finished);
    }

    /// Distinguishes per-chat silence from the account-wide transport cooldown.
    #[test]
    fn retries_rotate_and_only_transport_failure_blocks_other_chats() {
        let (_, mut job) = fixture();
        job.chats
            .insert("2@s.whatsapp.net".into(), job.chats[CHAT].clone());
        let first = job.reserve(NOW).unwrap().unwrap();
        job.fail(first.token, &first.chat, NOW, false);
        let other = job.reserve(NOW + 20).unwrap().unwrap();
        assert_ne!(first.chat, other.chat);
        job.fail(other.token, &other.chat, NOW + 20, true);
        assert!(job.reserve(NOW + 40).unwrap().is_none());
        assert!(job.reserve(NOW + 50).unwrap().is_some());
    }

    /// Bounds silent retries and proves manual retry retains the saved boundary.
    #[test]
    fn unanswered_chat_stops_after_three_attempts_and_retry_keeps_frontier() {
        let (_, mut job) = fixture();
        for now in [NOW, NOW + 30, NOW + 90] {
            let request = job.reserve(now).unwrap().unwrap();
            job.complete(request.token, CHAT, &Page::default(), now);
        }
        assert_eq!(job.progress().stopped, 1);
        assert!(!job.progress().finished);
        assert!(job.reserve(NOW + 1_000).unwrap().is_none());
        job.retry();
        assert_eq!(
            job.reserve(NOW + 1_000).unwrap().unwrap().before.id,
            "newest"
        );
    }

    /// Rejects cyclic pages and ambiguous ordering instead of skipping gaps.
    #[test]
    fn duplicate_cycles_and_unordered_ties_do_not_claim_progress() {
        let (_, mut job) = fixture();
        let request = job.reserve(NOW).unwrap().unwrap();
        job.complete(request.token, CHAT, &page("older", NOW - 200), NOW);
        let request = job.reserve(NOW + 20).unwrap().unwrap();
        let mut tied = page("a", NOW - 200);
        tied.observe(&message(CHAT, "b", NOW - 200, false));
        job.complete(request.token, CHAT, &tied, NOW + 20);
        assert_eq!(job.chats[CHAT].before.id, "older");
        let request = job.reserve(NOW + 50).unwrap().unwrap();
        job.complete(request.token, CHAT, &page("newest", NOW - 100), NOW + 50);
        assert_eq!(job.chats[CHAT].before.id, "older");
        assert_eq!(job.chats[CHAT].retry.failures, 2);
    }

    /// Keeps failed or malformed pages from falsely completing a chat.
    #[test]
    fn storage_or_malformed_page_cannot_complete_even_with_exhaustion() {
        for failed in [true, false] {
            let (_, mut job) = fixture();
            let request = job.reserve(NOW).unwrap().unwrap();
            let mut page = page("old", SINCE - 1);
            page.failed = failed;
            page.malformed = !failed;
            page.exhausted = true;
            job.complete(request.token, CHAT, &page, NOW);
            assert!(!job.progress().finished);
            assert_eq!(job.chats[CHAT].before.id, "newest");
            assert_eq!(job.progress().failed, failed);
        }
    }

    /// Checks persisted pause and pacing together with restart and replacement token fences.
    #[test]
    fn restart_keeps_pause_and_pace_and_rejects_old_attempts() {
        let (archive, mut job) = fixture();
        let old = job.reserve(NOW).unwrap().unwrap();
        job.pause(true);
        archive.save_history_import(&job).unwrap();
        let mut loaded = archive.history_import().unwrap().unwrap();
        loaded.recover();
        assert!(loaded.progress().paused);
        loaded.pause(false);
        assert!(loaded.reserve(NOW + 19).unwrap().is_none());
        let new = loaded.reserve(NOW + 20).unwrap().unwrap();
        loaded.complete(old.token, CHAT, &page("old", SINCE - 1), NOW + 20);
        assert!(!loaded.progress().finished);
        loaded.complete(
            new.token,
            CHAT,
            &Page {
                exhausted: true,
                ..Default::default()
            },
            NOW + 20,
        );
        assert!(loaded.progress().finished);
        archive.start_history_import(SINCE, NOW).unwrap();
        let mut replacement = archive.history_import().unwrap().unwrap();
        archive
            .discover_history_import(&mut replacement, &archive.chats().unwrap(), NOW)
            .unwrap();
        assert!(replacement.reserve(NOW + 19).unwrap().is_none());
        replacement.complete(new.token, CHAT, &page("old", SINCE - 1), NOW + 20);
        assert!(!replacement.progress().finished);
    }

    /// Keeps inaccessible and locally removed chats outside the request pass.
    #[test]
    fn discovery_excludes_locked_left_channels_and_deleted_chats() {
        let (archive, mut job) = fixture();
        for chat in [
            "2@s.whatsapp.net",
            "3@g.us",
            "4@newsletter",
            "5@s.whatsapp.net",
        ] {
            archive.ensure_chat(chat, "Fixture").unwrap();
        }
        archive
            .connection
            .execute("UPDATE chats SET locked=1 WHERE id='2@s.whatsapp.net'", [])
            .unwrap();
        archive
            .connection
            .execute("UPDATE chats SET left=1 WHERE id='3@g.us'", [])
            .unwrap();
        archive
            .connection
            .execute(
                "INSERT INTO chat_removals(chat,through) VALUES('5@s.whatsapp.net',10)",
                [],
            )
            .unwrap();
        archive
            .discover_history_import(&mut job, &archive.chats().unwrap(), NOW)
            .unwrap();
        assert_eq!(job.progress().excluded, 4);
        assert_eq!(job.chats.keys().collect::<Vec<_>>(), [CHAT]);
    }

    /// Checks canonical cursor migration and preservation of archived raw attachment data.
    #[test]
    fn privacy_mapping_moves_the_saved_frontier_in_existing_transaction() {
        let (archive, mut job) = fixture();
        archive.ensure_chat("9@lid", "Fixture").unwrap();
        archive
            .insert_message(
                &message("9@lid", "lid-message", NOW, false),
                Some(b"fixture-raw"),
            )
            .unwrap();
        archive
            .discover_history_import(&mut job, &archive.chats().unwrap(), NOW)
            .unwrap();
        let request = job.reserve(NOW).unwrap().unwrap();
        assert_eq!(request.chat, "9@lid");
        archive.save_history_import(&job).unwrap();
        archive.put_lid("9", "1").unwrap();
        let mut job = archive.history_import().unwrap().unwrap();
        assert!(!job.chats.contains_key("9@lid"));
        assert_eq!(job.chats[CHAT].before.id, "lid-message");
        job.complete(
            request.token,
            CHAT,
            &Page {
                exhausted: true,
                ..Default::default()
            },
            NOW,
        );
        assert!(job.progress().finished);
        assert_eq!(
            archive.raw(CHAT, "lid-message").unwrap().unwrap(),
            b"fixture-raw"
        );
    }

    /// Uses a synthetic SQL failure to verify rollback and explicit corruption errors.
    #[test]
    fn failed_progress_write_rolls_back_and_corrupt_progress_is_not_replaced() {
        let (archive, mut job) = fixture();
        archive.save_history_import(&job).unwrap();
        archive.connection.execute_batch("CREATE TRIGGER fail_import BEFORE UPDATE ON meta WHEN NEW.key='history_import_v1' BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
        job.pause(true);
        assert!(archive.save_history_import(&job).is_err());
        assert!(!archive.history_import().unwrap().unwrap().progress().paused);
        archive
            .connection
            .execute_batch("DROP TRIGGER fail_import")
            .unwrap();
        archive.set_meta(KEY, "not-json").unwrap();
        assert!(archive.history_import().is_err());
    }
}
