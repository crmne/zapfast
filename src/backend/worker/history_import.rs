//! Manual, paced history requests using the existing per-chat phone API.

use super::*;
use crate::archive::history_import::{Job, Page, Token};
use crate::model::HistoryImportProgress;

#[derive(Default)]
pub(super) struct State {
    pub allowed: bool,
    initialized: bool,
    pub dirty: bool,
    failed: bool,
    last: Option<HistoryImportProgress>,
    pub pages: HashMap<ChatId, Page>,
    early: VecDeque<Early>,
}

struct Early {
    chat: ChatId,
    serial: u64,
    session: String,
    page: Page,
}

impl Worker {
    /// Stops admission and publishes a recoverable failure without logging private job data.
    fn import_error(&mut self) {
        self.import.failed = true;
        let mut progress = self.import.last.clone().unwrap_or_default();
        progress.failed = true;
        self.import.last = Some(progress.clone());
        self.emit(Event::HistoryImport(progress));
        log::warn!("could not save history import progress");
    }

    /// Emits changed account progress and keeps the scheduler's admission snapshot current.
    fn import_publish(&mut self, job: &Job) {
        let progress = job.progress();
        if self.import.last.as_ref() != Some(&progress) {
            self.emit(Event::HistoryImport(progress.clone()));
            self.import.last = Some(progress);
        }
    }

    /// Persists an explicit start, pause or retry before admitting further requests.
    pub(super) fn change_import(&mut self, change: Command) {
        let result = (|| -> crate::archive::Result<()> {
            match change {
                Command::StartHistoryImport(since) => {
                    self.archive
                        .start_history_import(since, crate::util::now())?;
                    let old: Vec<_> = self
                        .pending_older
                        .iter()
                        .filter(|(_, request)| request.import.is_some())
                        .map(|(chat, _)| chat.clone())
                        .collect();
                    for chat in old {
                        if let Some(request) = self.pending_older.remove(&chat) {
                            self.remember_late(chat, request);
                        }
                    }
                    self.import.early.clear();
                    self.import.dirty = true;
                }
                Command::PauseHistoryImport(paused) => {
                    if let Some(mut job) = self.archive.history_import()? {
                        job.pause(paused);
                        self.archive.save_history_import(&job)?;
                    }
                }
                Command::RetryHistoryImport => {
                    if let Some(mut job) = self.archive.history_import()? {
                        if self.import.failed {
                            job.recover();
                        }
                        job.retry();
                        self.archive.save_history_import(&job)?;
                    }
                }
                _ => unreachable!(),
            }
            Ok(())
        })();
        if result.is_err() {
            self.import_error();
            return;
        }
        self.import.failed = false;
        self.pump_import();
    }

    /// Canonicalization uses the archive's existing merge transaction. Keep
    /// only the request's routing key in step with it, never merge data here.
    pub(super) fn rekey_import_requests(&mut self) {
        let moved: Vec<_> = self
            .pending_older
            .iter()
            .filter(|(_, request)| request.import.is_some())
            .map(|(chat, _)| (chat.clone(), self.canonical_str(chat)))
            .filter(|(chat, canonical)| chat != canonical)
            .collect();
        for (old, new) in moved {
            if let Some(request) = self.pending_older.remove(&old) {
                match self.pending_older.entry(new.clone()) {
                    std::collections::hash_map::Entry::Vacant(entry) => {
                        entry.insert(request);
                    }
                    std::collections::hash_map::Entry::Occupied(_) => {
                        if let Some(token) = request.import {
                            self.import_failure(token, &new, false);
                        }
                        self.remember_late(new, request)
                    }
                }
            }
        }
        let keys: Vec<_> = self
            .late_older
            .iter()
            .map(|late| {
                if late.import.is_some() {
                    self.canonical_str(&late.chat)
                } else {
                    late.chat.clone()
                }
            })
            .collect();
        for (late, key) in self.late_older.iter_mut().zip(keys) {
            late.chat = key;
        }
    }

    /// Refreshes eligible chats and durably reserves work before calling the existing phone API.
    pub(super) fn pump_import(&mut self) {
        if self.import.failed {
            return;
        }
        let result = (|| -> crate::archive::Result<()> {
            let Some(mut job) = self.archive.history_import()? else {
                return Ok(());
            };
            if !self.import.initialized {
                job.recover();
                self.archive.save_history_import(&job)?;
                self.import.initialized = true;
                self.import.dirty = true;
            }
            if self.import.dirty
                && self.status.is_connected()
                && self.privacy_ready
                && !self.syncing
            {
                let mut chats = self.archive.chats()?;
                chats.retain(|chat| chat.last.is_some() || self.canonical_str(&chat.id) == chat.id);
                self.archive
                    .discover_history_import(&mut job, &chats, crate::util::now())?;
                self.archive.save_history_import(&job)?;
                self.import.dirty = false;
            }
            self.import_publish(&job);
            if !self.import.allowed
                || !self.status.is_connected()
                || !self.privacy_ready
                || self.syncing
                || !self.pending_older.is_empty()
                || self.poll_history.in_flight()
            {
                return Ok(());
            }
            let now = crate::util::now();
            let Some(request) = job.reserve(now)? else {
                return Ok(());
            };
            // Recheck access at admission, even if a single-chat update did
            // not refresh the full chat list used for discovery.
            let chat = self.archive.chat(&request.chat)?;
            if chat.is_none_or(|chat| {
                chat.locked || chat.left || ChatKind::from_id(&chat.id) == ChatKind::Broadcast
            }) || self.archive.removal_point(&request.chat)?.is_some()
            {
                job.recover();
                self.import.dirty = true;
                self.archive.save_history_import(&job)?;
                return Ok(());
            }
            // Reserve durably before asking the phone. A failed save sends nothing.
            self.archive.save_history_import(&job)?;
            let chat = request.chat;
            if self.request_history(
                chat.clone(),
                request.before.id,
                request.before.from_me,
                request.before.at,
                false,
                true,
            ) {
                if let Some(pending) = self.pending_older.get_mut(&chat) {
                    pending.import = Some(request.token);
                }
            } else {
                job.fail(request.token, &chat, now, true);
                self.archive.save_history_import(&job)?;
                self.import_publish(&job);
            }
            Ok(())
        })();
        if result.is_err() {
            self.import_error();
        }
    }

    /// Routes a matching import callback after its raw privacy ID becomes canonical.
    pub(super) fn import_callback_chat(&self, chat: ChatId, serial: u64) -> ChatId {
        let canonical = self.canonical_str(&chat);
        if self
            .pending_older
            .get(&canonical)
            .is_some_and(|pending| pending.import.is_some() && pending.serial == serial)
        {
            canonical
        } else {
            chat
        }
    }

    /// Persists backoff only for the attempt that still owns the saved cursor.
    pub(super) fn import_failure(&mut self, token: Token, chat: &str, transport: bool) {
        let result = (|| -> crate::archive::Result<()> {
            if let Some(mut job) = self.archive.history_import()? {
                job.fail(token, chat, crate::util::now(), transport);
                self.archive.save_history_import(&job)?;
                self.import_publish(&job);
            }
            Ok(())
        })();
        if result.is_err() {
            self.import_error();
        }
    }

    /// Applies correlated page metadata and publishes its durably saved progress.
    pub(super) fn import_complete(&mut self, token: Token, chat: &str, page: Page) {
        let result = (|| -> crate::archive::Result<()> {
            if let Some(mut job) = self.archive.history_import()? {
                job.complete(token, chat, &page, crate::util::now());
                self.archive.save_history_import(&job)?;
                self.import_publish(&job);
            }
            Ok(())
        })();
        if result.is_err() {
            self.import_error();
        }
    }

    /// Bounds early tagged replies until dispatch confirms their exact session ID.
    pub(super) fn import_buffer(&mut self, chat: &str, session: Option<&str>, page: Page) {
        if let Some(session) = session
            && let Some(pending) = self.pending_older.get(chat)
            && pending.import.is_some()
            && pending.request.is_none()
        {
            if self.import.early.len() == LATE_OLDER_LIMIT {
                self.import.early.pop_front();
            }
            self.import.early.push_back(Early {
                chat: chat.to_owned(),
                serial: pending.serial,
                session: session.to_owned(),
                page,
            });
        }
    }

    /// Consumes only buffered replies matching the confirmed dispatch and clears reader spinners.
    pub(super) fn import_sent(&mut self, serial: u64, session: &str) {
        let early = std::mem::take(&mut self.import.early);
        for reply in early {
            if reply.serial == serial {
                if reply.session == session {
                    let chat = self.canonical_str(&reply.chat);
                    if let Some(pending) = self.pending_older.get(&chat)
                        && pending.serial == serial
                        && let Some(token) = pending.import
                    {
                        let explicit = pending.explicit;
                        let before = pending.before.clone();
                        self.pending_older.remove(&chat);
                        let more = !reply.page.exhausted;
                        if explicit {
                            self.show_older_page(&chat, (before.0, &before.1));
                        }
                        self.import_complete(token, &chat, reply.page);
                        self.emit(Event::OlderFetched {
                            chat,
                            more,
                            silent: !explicit,
                        });
                    }
                }
            } else {
                self.import.early.push_back(reply);
            }
        }
    }

    /// Reports runnable import work that should temporarily take priority over prefetch.
    pub(super) fn import_active(&self) -> bool {
        self.import
            .last
            .as_ref()
            .is_some_and(HistoryImportProgress::active)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHAT: &str = "4915700000002@s.whatsapp.net";

    /// Clearing or deleting an in-flight chat must release the pass for other chats.
    #[test]
    fn removing_an_importing_chat_does_not_strand_other_chats() {
        for delete in [false, true] {
            let (mut worker, _, _, _) = receipt_tests::worker();
            reserve(&mut worker, Some("removed-chat"), 1);
            let other = "4915700000003@s.whatsapp.net";
            worker.archive.ensure_chat(other, "Fixture").unwrap();
            let mut job = worker.archive.history_import().unwrap().unwrap();
            worker
                .archive
                .discover_history_import(
                    &mut job,
                    &worker.archive.chats().unwrap(),
                    crate::util::now(),
                )
                .unwrap();
            worker.archive.save_history_import(&job).unwrap();
            worker.import.initialized = true;
            worker.import.dirty = false;
            if delete {
                worker.remove_chat(CHAT, crate::util::now(), false);
            } else {
                assert!(worker.empty_chat(CHAT, crate::util::now(), false));
            }
            assert!(worker.pending_older.is_empty());
            worker.pump_import();
            let mut job = worker.archive.history_import().unwrap().unwrap();
            let request = job
                .reserve(crate::util::now() + 2_000)
                .unwrap()
                .expect("another chat can continue");
            assert_eq!(request.chat, other);
            assert_eq!(worker.archive.chat(CHAT).unwrap().is_none(), delete);
        }
    }

    /// Logout clears the old job without losing either unlocked or locked permission.
    #[tokio::test]
    async fn logout_preserves_import_permission_without_an_app_lock_change() {
        for allowed in [true, false] {
            let (mut worker, _, _, _) = receipt_tests::worker();
            reserve(&mut worker, Some("old-session"), 1);
            worker.import.allowed = allowed;
            worker.import.initialized = true;
            worker.import.failed = true;
            worker.import.last = Some(worker.archive.history_import().unwrap().unwrap().progress());
            // Refuse the synthetic device-store open before start_bot can dial.
            let session = worker.dirs.session_db();
            std::fs::create_dir_all(&session).unwrap();
            worker.on_logged_out().await;
            assert_eq!(worker.import.allowed, allowed);
            assert!(!worker.import.initialized && !worker.import.failed);
            assert!(worker.import.last.is_none());
            assert!(worker.pending_older.is_empty());
            assert!(worker.archive.history_import().unwrap().is_none());
            assert!(
                worker.client.is_none(),
                "no live connection in this regression"
            );
        }
    }

    /// A stopped pass releases prefetch after the reader explicitly enables it again.
    #[test]
    fn stopped_import_does_not_block_reenabled_background_history() {
        let (mut worker, _, _, _) = receipt_tests::worker();
        for serial in 1..=3 {
            reserve(&mut worker, Some("silent-phone"), serial);
            let now = worker.pending_older[CHAT].asked + PHONE_PATIENCE * 2;
            worker.expire_older_requests(now);
        }
        assert_eq!(worker.import.last.as_ref().unwrap().stopped, 1);
        assert!(!worker.import_active());
        assert!(!worker.archive.history_import().unwrap().unwrap().active());
        worker.prefetch.configure(true, None);
        assert!(worker.prefetch.needs_chats());
        worker.pump_prefetch();
        assert!(
            !worker.prefetch.needs_chats(),
            "the background pump reached its queue rebuild"
        );
        worker.change_import(Command::RetryHistoryImport);
        assert!(worker.import_active(), "retrying work takes priority again");
        worker.change_import(Command::PauseHistoryImport(true));
        assert!(
            !worker.import_active(),
            "paused work does not block prefetch either"
        );
    }

    /// Creates a synthetic durable attempt and matching worker request without sending to WhatsApp.
    fn reserve(worker: &mut Worker, session: Option<&str>, serial: u64) {
        let now = crate::util::now();
        worker.archive.ensure_chat(CHAT, "Fixture").unwrap();
        if worker.archive.history_import().unwrap().is_none() {
            worker
                .archive
                .start_history_import(now.div_euclid(86_400) * 86_400 - 86_400, now)
                .unwrap();
        }
        let mut job = worker.archive.history_import().unwrap().unwrap();
        worker
            .archive
            .discover_history_import(&mut job, &worker.archive.chats().unwrap(), now)
            .unwrap();
        let request = job.reserve(now + 1_000).unwrap().unwrap();
        worker.archive.save_history_import(&job).unwrap();
        worker.pending_older.insert(
            CHAT.into(),
            OlderRequest {
                asked: Instant::now(),
                before: (request.before.at, request.before.id),
                explicit: false,
                background: true,
                serial,
                request: session.map(str::to_owned),
                import: Some(request.token),
            },
        );
    }

    /// Files a synthetic response through the normal archive and correlation paths.
    fn answer(worker: &mut Worker, session: Option<&str>, exhausted: bool) {
        let mut history = chat_removal_tests::history(CHAT, &[crate::util::now() - 10]);
        history.chats[0].import_exhausted = exhausted;
        let filed = worker.apply_history(history, false);
        worker.answer_older(filed, session);
    }

    /// Rejects missing or unrelated session IDs before advancing the saved job.
    #[test]
    fn date_import_requires_exact_session_even_before_other_ids_have_matched() {
        let (mut worker, _, _, _) = receipt_tests::worker();
        reserve(&mut worker, Some("ours"), 1);
        for session in [None, Some("unrelated")] {
            answer(&mut worker, session, true);
            assert!(worker.pending_older.contains_key(CHAT));
            assert!(
                !worker
                    .archive
                    .history_import()
                    .unwrap()
                    .unwrap()
                    .progress()
                    .finished
            );
        }
        answer(&mut worker, Some("ours"), true);
        assert!(worker.pending_older.is_empty());
        assert!(
            worker
                .archive
                .history_import()
                .unwrap()
                .unwrap()
                .progress()
                .finished
        );
    }

    /// A reader joining an import sees the page for normal, early and timed-out replies.
    #[test]
    fn a_reader_joining_an_import_receives_the_older_page() {
        for timing in ["normal", "early", "late"] {
            let early = timing == "early";
            for explicit in [false, true] {
                let (mut worker, events, _, _) = receipt_tests::worker();
                reserve(&mut worker, (!early).then_some("ours"), 1);
                if explicit {
                    assert!(!worker.fetch_older(CHAT.into(), true, false));
                    assert!(worker.pending_older[CHAT].explicit);
                }
                while events.try_recv().is_ok() {}
                if timing == "late" {
                    let now = worker.pending_older[CHAT].asked + PHONE_PATIENCE * 2;
                    worker.expire_older_requests(now);
                    assert!(worker.pending_older.is_empty());
                    while events.try_recv().is_ok() {}
                }
                answer(&mut worker, Some("ours"), true);
                if early {
                    assert!(
                        events
                            .try_iter()
                            .all(|event| !matches!(event, Event::Messages { older: true, .. }))
                    );
                    worker.older_sent(CHAT.into(), 1, "ours".into(), Instant::now());
                }
                let mut displayed = false;
                let mut completed = false;
                for event in events.try_iter() {
                    match event {
                        Event::Messages {
                            chat,
                            messages,
                            older: true,
                            ..
                        } => {
                            assert_eq!(chat, CHAT);
                            assert!(!messages.is_empty());
                            assert!(!completed, "display the page before ending the request");
                            displayed = true;
                        }
                        Event::OlderFetched { chat, silent, .. } => {
                            assert_eq!(chat, CHAT);
                            assert_eq!(silent, !explicit);
                            assert_eq!(displayed, explicit);
                            completed = true;
                        }
                        _ => {}
                    }
                }
                assert!(completed);
                assert_eq!(
                    displayed, explicit,
                    "background imports must not move the view"
                );
                assert!(worker.pending_older.is_empty());
                assert_eq!(
                    worker
                        .archive
                        .history_import()
                        .unwrap()
                        .unwrap()
                        .progress()
                        .finished,
                    timing != "late",
                    "a late page cannot advance an expired import lease"
                );
            }
        }
    }

    /// Checks early buffering and the separate dispatch and phone-response deadlines.
    #[test]
    fn early_reply_waits_for_dispatch_confirmation_and_response_clock_restarts() {
        let (mut worker, _, _, _) = receipt_tests::worker();
        reserve(&mut worker, None, 1);
        let now = worker.pending_older[CHAT].asked + PHONE_PATIENCE * 2;
        worker.expire_older_requests(now);
        assert!(
            worker.pending_older.contains_key(CHAT),
            "preparing is not a silent phone"
        );
        answer(&mut worker, Some("ours"), true);
        assert!(
            !worker
                .archive
                .history_import()
                .unwrap()
                .unwrap()
                .progress()
                .finished
        );
        worker.older_sent(CHAT.into(), 1, "ours".into(), now);
        assert!(
            worker
                .archive
                .history_import()
                .unwrap()
                .unwrap()
                .progress()
                .finished
        );

        let (mut worker, _, _, _) = receipt_tests::worker();
        reserve(&mut worker, None, 1);
        let sent_at = worker.pending_older[CHAT].asked + PHONE_PATIENCE * 2;
        worker.older_sent(CHAT.into(), 1, "ours".into(), sent_at);
        assert_eq!(worker.pending_older[CHAT].asked, sent_at);
        worker.expire_older_requests(sent_at + PHONE_PATIENCE);
        assert!(
            worker.pending_older.contains_key(CHAT),
            "the new response lease is still valid"
        );
        worker.expire_older_requests(sent_at + PHONE_PATIENCE + Duration::from_secs(1));
        assert!(
            worker.pending_older.is_empty(),
            "only the new response deadline expires it"
        );
    }

    /// Keeps late responses and stale send failures from taking over a newer request.
    #[test]
    fn expired_reply_and_stale_callback_cannot_complete_a_new_attempt() {
        let (mut worker, _, _, _) = receipt_tests::worker();
        reserve(&mut worker, Some("old"), 1);
        let now = worker.pending_older[CHAT].asked + PHONE_PATIENCE * 2;
        worker.expire_older_requests(now);
        reserve(&mut worker, Some("new"), 2);
        answer(&mut worker, Some("old"), true);
        worker.older_failed(CHAT.into(), 1, "fixture".into());
        assert_eq!(worker.pending_older[CHAT].serial, 2);
        assert!(
            !worker
                .archive
                .history_import()
                .unwrap()
                .unwrap()
                .progress()
                .finished
        );
        answer(&mut worker, Some("new"), true);
        assert!(
            worker
                .archive
                .history_import()
                .unwrap()
                .unwrap()
                .progress()
                .finished
        );
    }

    /// Distinguishes inaccessible phone history from an explicit no-more-messages response.
    #[test]
    fn inaccessible_history_is_not_phone_exhaustion() {
        let mut conversation = wa::Conversation::default();
        conversation.id = CHAT.into();
        conversation.end_of_history_transfer_type = Some(wa::conversation::EndOfHistoryTransferType::COMPLETE_ON_DEMAND_SYNC_WITH_MORE_MSG_ON_PRIMARY_BUT_NO_ACCESS);
        let parsed = parse_conversation(conversation);
        assert!(!parsed.import_exhausted);
    }

    /// Checks visible corruption handling and isolation from another synthetic account.
    #[test]
    fn corrupt_progress_stops_requests_and_events_remain_account_local() {
        let (mut worker, events, _, _) = receipt_tests::worker();
        let (other, other_events, _, _) = receipt_tests::worker();
        worker
            .archive
            .set_meta("history_import_v1", "broken")
            .unwrap();
        worker.pump_import();
        assert!(worker.import.failed);
        assert!(
            events
                .try_iter()
                .any(|event| matches!(event, Event::HistoryImport(progress) if progress.failed))
        );
        assert!(other.archive.history_import().unwrap().is_none());
        assert!(other_events.try_iter().next().is_none());
    }

    /// Ensures a successful retry clears stale error UI before completion counts change.
    #[test]
    fn retry_clears_the_displayed_save_failure_even_before_a_chat_finishes() {
        let (mut worker, events, _, _) = receipt_tests::worker();
        reserve(&mut worker, Some("ours"), 1);
        let job = worker.archive.history_import().unwrap().unwrap();
        worker.import_publish(&job);
        worker.import_error();
        assert!(worker.import.last.as_ref().unwrap().failed);
        worker.change_import(Command::RetryHistoryImport);
        assert!(!worker.import.failed);
        assert!(!worker.import.last.as_ref().unwrap().failed);
        assert!(
            matches!(events.try_iter().last(), Some(Event::HistoryImport(progress)) if !progress.failed)
        );
    }

    /// Exercises dispatch confirmation across the existing privacy-ID merge transaction.
    #[test]
    fn a_raw_privacy_id_dispatch_callback_follows_the_canonical_frontier() {
        let (mut worker, _, _, _) = receipt_tests::worker();
        let lid = "9@lid";
        let now = crate::util::now();
        worker.archive.ensure_chat(lid, "Fixture").unwrap();
        worker
            .archive
            .start_history_import(now.div_euclid(86_400) * 86_400 - 86_400, now)
            .unwrap();
        let mut job = worker.archive.history_import().unwrap().unwrap();
        worker
            .archive
            .discover_history_import(&mut job, &worker.archive.chats().unwrap(), now)
            .unwrap();
        let request = job.reserve(now).unwrap().unwrap();
        worker.archive.save_history_import(&job).unwrap();
        worker.pending_older.insert(
            lid.into(),
            OlderRequest {
                asked: Instant::now(),
                before: (request.before.at, request.before.id),
                explicit: false,
                background: true,
                serial: 1,
                request: None,
                import: Some(request.token),
            },
        );
        worker.learn_lid("9", "4915700000002");
        worker.older_sent(lid.into(), 1, "ours".into(), Instant::now());
        assert_eq!(worker.pending_older[CHAT].request.as_deref(), Some("ours"));
        answer(&mut worker, Some("ours"), true);
        assert!(
            worker
                .archive
                .history_import()
                .unwrap()
                .unwrap()
                .progress()
                .finished
        );
    }

    /// Gives a reader priority without leaving the import's durable attempt stranded.
    #[test]
    fn a_reader_request_collision_releases_the_import_instead_of_stranding_it() {
        let (mut worker, _, _, _) = receipt_tests::worker();
        reserve(&mut worker, Some("import"), 1);
        let request = worker.pending_older.remove(CHAT).unwrap();
        worker.pending_older.insert("9@lid".into(), request);
        worker.pending_older.insert(
            CHAT.into(),
            OlderRequest {
                import: None,
                asked: Instant::now(),
                before: (100, "fixture".into()),
                explicit: true,
                background: false,
                serial: 2,
                request: Some("reader".into()),
            },
        );
        worker.learn_lid("9", "4915700000002");
        worker.rekey_import_requests();
        assert_eq!(worker.pending_older[CHAT].serial, 2);
        let mut job = worker.archive.history_import().unwrap().unwrap();
        assert!(
            job.reserve(crate::util::now() + 2_000).unwrap().is_some(),
            "the reader keeps priority, and the import can retry afterwards"
        );
    }

    /// Prevents discovery and network admission before account privacy is ready.
    #[test]
    fn discovery_waits_until_initial_sync_and_privacy_recovery_finish() {
        let (mut worker, _, _, _) = receipt_tests::worker();
        worker.archive.ensure_chat(CHAT, "Fixture").unwrap();
        let now = crate::util::now();
        worker
            .archive
            .start_history_import(now.div_euclid(86_400) * 86_400, now)
            .unwrap();
        worker.status = LinkStatus::Connected;
        worker.syncing = true;
        worker.pump_import();
        assert_eq!(worker.import.last.as_ref().unwrap().total, 0);
        worker.syncing = false;
        worker.privacy_ready = false;
        worker.pump_import();
        assert_eq!(worker.import.last.as_ref().unwrap().total, 0);
        worker.privacy_ready = true;
        worker.pump_import();
        assert_eq!(worker.import.last.as_ref().unwrap().total, 1);
        assert!(
            worker.pending_older.is_empty(),
            "no requests before the interface permits them"
        );
    }
}
