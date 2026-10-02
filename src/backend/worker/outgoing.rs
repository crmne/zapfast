//! Serial outgoing messages and explicit pre-send rate-limit recovery.
//!
//! Sends go out one at a time, in order, with no pause between them, so a
//! normal burst looks as it always did: a clock tick, then sent. Only a
//! typed pre-transmission rate refusal (IQ 429) changes that: the refused
//! message and everything behind it wait, visibly and cancellably, until the
//! server's cooldown ends, then go out in order at their dispatch time.

use super::{ChatId, Delivery, Event, Instant, Jid, VecDeque, Worker, wa};
use crate::backend::SendFailure;
use std::time::Duration;
use whatsapp_rust::{request::IqError, send::SendError, wacore_binary::jid::JidExt};

#[derive(Clone)]
pub(super) struct Job {
    pub chat: ChatId,
    pub id: String,
    pub jid: Jid,
    pub message: wa::Message,
    pub expiration: Option<u32>,
    /// Shown as waiting: it met a rate-limit cooldown, or stood behind one.
    pub waited: bool,
    /// The time it waited at, once it went out from waiting.
    pub queued_at: Option<i64>,
}

#[derive(Default)]
pub(super) struct Outgoing {
    pub waiting: VecDeque<Job>,
    pub running: Option<Job>,
    task: Option<tokio::task::AbortHandle>,
    /// The end of the server's cooldown after a rate refusal.
    pub retry_at: Option<Instant>,
    /// Demos and tests: started sends land here instead of going to
    /// WhatsApp, and the caller reports each one as it would have ended.
    #[cfg(any(test, feature = "demo"))]
    pub synthetic: Option<Vec<Job>>,
}

impl Outgoing {
    fn cooling_down(&self, now: Instant) -> bool {
        self.retry_at.is_some_and(|due| due > now)
    }

    /// A new send waits visibly when a cooldown runs, or when a message that
    /// met one is still ahead of it: sending order is the chat's order.
    fn must_wait(&self, now: Instant) -> bool {
        self.cooling_down(now) || self.waiting.iter().any(|job| job.waited)
    }
}

pub(super) fn classify_send_error(error: SendError) -> SendFailure {
    // This typed IQ failure is a pre-transmission query (routing/devices/keys),
    // not an acknowledgement timeout or an uncertain socket write. Never infer
    // retry safety from Display text, a transport error, or an unknown variant.
    // The kind names the variant only: Display text can carry stanza text.
    let (kind, code) = match error {
        SendError::Iq(IqError::ServerError {
            code: 429, backoff, ..
        }) => {
            return SendFailure::RateLimited {
                retry_after: backoff.unwrap_or(60).max(1),
            };
        }
        SendError::Iq(IqError::ServerError { code, .. }) => ("iq server error", Some(code)),
        SendError::Iq(IqError::Timeout) => ("iq timeout", None),
        SendError::Iq(IqError::NotConnected) => ("iq not connected", None),
        SendError::Iq(IqError::Disconnected(_)) => ("iq disconnected", None),
        SendError::Iq(_) => ("iq", None),
        SendError::Client(_) => ("client", None),
        SendError::NotLoggedIn => ("not logged in", None),
        SendError::InvalidRequest(_) => ("invalid request", None),
        SendError::NoRecipientDevice(_) => ("no recipient device", None),
        SendError::PrimaryDeviceRejected(_) => ("primary device rejected", None),
        SendError::Internal(_) => ("internal", None),
        _ => ("unknown", None),
    };
    SendFailure::Failed { kind, code }
}

/// A lost or slow link, as opposed to a refusal: the reader is told to check
/// the connection.
fn connection_kind(kind: &str) -> bool {
    matches!(kind, "iq timeout" | "iq not connected" | "iq disconnected")
}

impl Worker {
    /// Queues a stored pending message. It goes out now unless a send is
    /// running or a cooldown is on; behind a cooldown it shows as waiting.
    pub(super) fn queue_outgoing(
        &mut self,
        chat: ChatId,
        jid: Jid,
        id: String,
        message: wa::Message,
        expiration: Option<u32>,
    ) {
        let now = Instant::now();
        let mut job = Job {
            chat,
            jid,
            id,
            message,
            expiration,
            waited: false,
            queued_at: None,
        };
        // Without a link it does not wait: the pump fails it at once.
        if self.link_up() && self.outgoing.must_wait(now) {
            self.mark_waiting(&mut job);
        }
        self.outgoing.waiting.push_back(job);
        self.pump_outgoing_at(now);
    }

    fn link_up(&self) -> bool {
        self.client.is_some() && self.status == super::LinkStatus::Connected
    }

    /// D6: without a link, every message that is not waiting out a rate
    /// limit fails at once, as before the queue. Messages that wait keep
    /// waiting, with their Cancel, and go out in order once the link is back
    /// and the cooldown is over. `true` when a message failed.
    fn fail_while_offline(&mut self) -> bool {
        if self.link_up() {
            return false;
        }
        let (waited, unsent): (VecDeque<Job>, VecDeque<Job>) =
            std::mem::take(&mut self.outgoing.waiting)
                .into_iter()
                .partition(|job| job.waited);
        self.outgoing.waiting = waited;
        if unsent.is_empty() {
            return false;
        }
        log::warn!("send failed: not connected, {} not sent", unsent.len());
        for job in &unsent {
            self.finish_outgoing(job, Delivery::Failed);
        }
        true
    }

    /// Shows the row as waiting. `false` when the row is no longer a
    /// pending send of ours: deleted, cleared, or confirmed by a receipt.
    fn mark_waiting(&mut self, job: &mut Job) -> bool {
        job.waited = self
            .archive
            .set_outgoing_state(&job.chat, &job.id, Delivery::Queued)
            .unwrap_or(false);
        if job.waited {
            self.emit_message(&job.chat, &job.id);
            self.emit_chat(&job.chat);
        }
        job.waited
    }

    /// A send that waited and is refused again waits once more at the time
    /// it waited at, ahead of the rows queued behind it. `false` as in
    /// `mark_waiting`.
    fn requeue(&mut self, job: &mut Job, queued_at: i64) -> bool {
        job.waited = self
            .archive
            .requeue_outgoing(&job.chat, &job.id, queued_at)
            .unwrap_or(false);
        if job.waited {
            if let Ok(Some(mut message)) = self.archive.message(&job.chat, &job.id) {
                self.polish(&mut message);
                self.emit(Event::MessageRequeued(Box::new(message)));
            }
            self.emit_chat(&job.chat);
        }
        job.waited
    }

    pub(super) fn outgoing_deadline(&self) -> Option<Instant> {
        if self.client.is_none()
            || self.status != super::LinkStatus::Connected
            || self.outgoing.running.is_some()
            || self.outgoing.waiting.is_empty()
        {
            return None;
        }
        Some(self.outgoing.retry_at.unwrap_or_else(Instant::now))
    }

    pub(super) fn pump_outgoing(&mut self) {
        self.pump_outgoing_at(Instant::now());
    }

    pub(super) fn pump_outgoing_at(&mut self, now: Instant) {
        if self.fail_while_offline() {
            self.emit(Event::SendFailed { connection: true });
        }
        if self.outgoing.running.is_some() || self.outgoing.cooling_down(now) {
            return;
        }
        let Some(client) = self.client.clone().filter(|_| self.link_up()) else {
            return;
        };
        let Some(mut job) = self.outgoing.waiting.pop_front() else {
            return;
        };
        // The cooldown is over once a send goes out past it.
        self.outgoing.retry_at = None;
        if job.waited {
            // The row is what the reader sees. Gone (deleted, cleared, or
            // confirmed by a receipt meanwhile) means nothing to send.
            match self
                .archive
                .dispatch_outgoing(&job.chat, &job.id, crate::util::now())
            {
                Ok(Some(queued_at)) => {
                    job.queued_at = Some(queued_at);
                    self.emit_message(&job.chat, &job.id);
                    self.emit_chat(&job.chat);
                }
                Ok(None) => {
                    log::warn!("a waiting message is no longer waiting; its send is dropped");
                    self.finish_interactive(&job.chat, &job.id);
                    return self.pump_outgoing_at(now);
                }
                Err(error) => {
                    log::warn!("could not start a waiting message: {error}");
                    self.finish_interactive(&job.chat, &job.id);
                    return self.pump_outgoing_at(now);
                }
            }
        }
        // Claim synchronously on the worker before spawning. A cancel command
        // processed after this point finds nothing waiting under this id.
        self.outgoing.running = Some(job.clone());
        #[cfg(any(test, feature = "demo"))]
        if let Some(started) = self.outgoing.synthetic.as_mut() {
            started.push(job);
            return;
        }
        self.outgoing.task = Some(
            tokio::spawn(super::send_outgoing(
                client,
                self.commands.clone(),
                job.chat,
                job.jid,
                job.id,
                job.message,
                job.expiration,
            ))
            .abort_handle(),
        );
    }

    pub(super) fn outgoing_finished(
        &mut self,
        chat: ChatId,
        id: String,
        result: Result<(), SendFailure>,
    ) {
        if !self
            .outgoing
            .running
            .as_ref()
            .is_some_and(|job| job.chat == chat && job.id == id)
        {
            return; // A late completion belongs to an abandoned session.
        }
        let mut job = self.outgoing.running.take().expect("matched running send");
        self.outgoing.task = None;
        match result {
            Err(SendFailure::RateLimited { retry_after }) => {
                log::warn!(
                    "send rate limited: cooldown {retry_after} s, {} waiting",
                    self.outgoing.waiting.len() + 1
                );
                self.outgoing.retry_at =
                    Some(Instant::now() + Duration::from_secs(u64::from(retry_after)));
                let waiting = match job.queued_at {
                    Some(queued_at) => self.requeue(&mut job, queued_at),
                    None => self.mark_waiting(&mut job),
                };
                if waiting {
                    if job.jid.is_group()
                        && let Err(error) = self.archive.discard_unsent_group_audience(&chat, &id)
                    {
                        log::warn!("could not discard a refused group audience: {error}");
                    }
                    // Everything behind it waits too, visibly, in order.
                    let mut rest: Vec<Job> = self.outgoing.waiting.drain(..).collect();
                    for other in &mut rest {
                        if !other.waited {
                            self.mark_waiting(other);
                        }
                    }
                    self.outgoing.waiting.push_back(job);
                    self.outgoing.waiting.extend(rest);
                } else {
                    // Only a receipt for this very id, or a local delete, takes
                    // a pending row out of our hands: nothing left to retry.
                    log::warn!("a rate-limited message is no longer pending; its send is dropped");
                    self.finish_interactive(&chat, &id);
                }
            }
            Ok(()) => self.finish_outgoing(&job, Delivery::Sent),
            Err(SendFailure::Failed { kind, code }) => {
                log::warn!(
                    "send failed: {kind}{}, {} waiting",
                    code.map(|code| format!(" (iq code {code})"))
                        .unwrap_or_default(),
                    self.outgoing.waiting.len()
                );
                self.finish_outgoing(&job, Delivery::Failed);
                // One toast for this message and any that fail behind it.
                let connection = connection_kind(kind) || !self.link_up();
                self.fail_while_offline();
                self.emit(Event::SendFailed { connection });
            }
        }
        self.pump_outgoing();
    }

    fn finish_outgoing(&mut self, job: &Job, status: Delivery) {
        if status == Delivery::Sent {
            let _ = self
                .archive
                .set_status(&job.chat, &job.id, status, crate::util::now());
        } else {
            let _ = self.archive.set_outgoing_state(&job.chat, &job.id, status);
        }
        self.finish_interactive(&job.chat, &job.id);
        self.emit_message(&job.chat, &job.id);
        self.emit_chat(&job.chat);
    }

    pub(super) fn finish_interactive(&mut self, chat: &str, id: &str) {
        let source =
            self.interactive_sending
                .iter()
                .find_map(|((pending_chat, source), pending_id)| {
                    (pending_chat == chat && pending_id == id).then(|| source.clone())
                });
        if let Some(message) = source {
            self.interactive_sending
                .remove(&(chat.to_owned(), message.clone()));
            self.emit(Event::InteractiveReplyState {
                chat: chat.to_owned(),
                message,
                pending: false,
            });
        }
    }

    /// Deletes a message that is still waiting, so it is never sent. A send
    /// that already started, or one that is not ours to cancel, is left
    /// alone: the reply is `false` and the row goes on as before.
    pub(super) fn cancel_queued(&mut self, chat: &str, id: &str) -> bool {
        let Some(index) = self
            .outgoing
            .waiting
            .iter()
            .position(|job| job.chat == chat && job.id == id)
        else {
            return false;
        };
        let job = self
            .outgoing
            .waiting
            .remove(index)
            .expect("queued job exists");
        self.finish_interactive(&job.chat, &job.id);
        if let Ok(true) = self.archive.delete_message(&job.chat, &job.id) {
            self.emit(Event::MessageDeleted {
                chat: job.chat.clone(),
                id: job.id,
            });
            self.emit_chat(&job.chat);
        }
        true
    }

    pub(super) fn abandon_outgoing(&mut self) {
        if let Some(task) = self.outgoing.task.take() {
            task.abort();
        }
        if let Some(job) = self.outgoing.running.take() {
            // Aborting cannot prove whether the socket already transmitted it.
            // Release active ownership without claiming failure or retrying.
            self.finish_outgoing(&job, Delivery::Unconfirmed);
        }
        let waiting = std::mem::take(&mut self.outgoing.waiting);
        self.outgoing.retry_at = None;
        if waiting.is_empty() {
            return;
        }
        log::warn!(
            "send stopped with the connection: {} not sent",
            waiting.len()
        );
        for job in &waiting {
            self.finish_outgoing(job, Delivery::Failed);
        }
        self.emit(Event::SendFailed { connection: true });
    }
}

#[cfg(test)]
mod tests {
    use super::super::LinkStatus;
    use super::super::receipt_tests::{PEER, own_message, worker};
    use super::*;
    use crate::backend::Command;

    fn job(chat: &str, id: &str) -> Job {
        Job {
            chat: chat.into(),
            id: id.into(),
            jid: chat.parse().unwrap(),
            message: wa::Message::default(),
            expiration: None,
            waited: false,
            queued_at: None,
        }
    }

    async fn attach_offline_client(worker: &mut Worker) -> tempfile::TempDir {
        let directory = tempfile::tempdir().unwrap();
        let store = whatsapp_rust::store::SqliteStore::new(
            directory.path().join("fixture.db").to_str().unwrap(),
        )
        .await
        .unwrap();
        let bot = super::super::Bot::builder()
            .with_backend(store)
            .build()
            .await
            .unwrap();
        worker.client = Some(bot.client()); // Never run/connect this bot.
        directory
    }

    async fn visible_rows(
        worker: &mut Worker,
        events: &std::sync::mpsc::Receiver<Event>,
    ) -> Vec<crate::model::Message> {
        events.try_iter().for_each(drop);
        worker
            .handle_command(Command::LoadChat {
                chat: PEER.into(),
                before: None,
            })
            .await;
        events
            .try_iter()
            .filter_map(|event| match event {
                Event::Messages { messages, .. } => Some(messages),
                _ => None,
            })
            .flatten()
            .collect()
    }

    async fn send_text(worker: &mut Worker, text: &str) {
        worker
            .handle_command(Command::SendText {
                chat: PEER.into(),
                text: text.into(),
                quoting: None,
                mentions: Vec::new(),
            })
            .await;
    }

    async fn finish(worker: &mut Worker, id: &str, result: Result<(), SendFailure>) {
        worker
            .handle_command(Command::OutgoingFinished {
                chat: PEER.into(),
                id: id.into(),
                result,
            })
            .await;
    }

    fn rate_error(code: u16, backoff: Option<u32>) -> SendError {
        use whatsapp_rust::wacore_binary::{
            OwnedNodeRef, builder::NodeBuilder, marshal::marshal, util::unpack,
        };
        let bytes = marshal(&NodeBuilder::new("iq").attr("type", "error").build()).unwrap();
        let response =
            std::sync::Arc::new(OwnedNodeRef::new(unpack(&bytes).unwrap().into_owned()).unwrap())
                .into();
        SendError::Iq(IqError::ServerError {
            code,
            backoff,
            response,
            text: "private upstream detail".into(),
            error_type: None,
        })
    }

    fn statuses(rows: &[crate::model::Message]) -> Vec<Delivery> {
        rows.iter().map(|row| row.status).collect()
    }

    /// D1: a burst that is not rate limited looks as it did before the queue.
    #[tokio::test]
    async fn a_normal_burst_keeps_every_row_pending_and_sends_the_next_at_once() {
        let (mut worker, events, _, _) = worker();
        let _directory = attach_offline_client(&mut worker).await;
        send_text(&mut worker, "First synthetic message").await;
        send_text(&mut worker, "Second synthetic message").await;
        send_text(&mut worker, "Third synthetic message").await;
        let rows = visible_rows(&mut worker, &events).await;
        assert_eq!(statuses(&rows), [Delivery::Pending; 3]);
        assert_eq!(
            worker.outgoing.running.as_ref().map(|job| job.id.as_str()),
            Some(rows[0].id.as_str())
        );
        finish(&mut worker, &rows[0].id, Ok(())).await;
        // No pacing: the next send starts inside the completion itself.
        assert_eq!(
            worker.outgoing.running.as_ref().map(|job| job.id.as_str()),
            Some(rows[1].id.as_str())
        );
        let updates: Vec<_> = events
            .try_iter()
            .filter_map(|event| match event {
                Event::MessageUpdated(row) => Some(row.status),
                _ => None,
            })
            .collect();
        assert!(!updates.contains(&Delivery::Queued), "{updates:?}");
        assert_eq!(
            statuses(&visible_rows(&mut worker, &events).await),
            [Delivery::Sent, Delivery::Pending, Delivery::Pending]
        );
        worker.stop_bot().await;
    }

    #[tokio::test]
    async fn rate_limit_keeps_the_message_queued_without_a_technical_toast() {
        let (mut worker, events, _, _) = worker();
        worker.store_message(
            crate::model::Message {
                status: Delivery::Pending,
                ..own_message("queued-fixture", 1)
            },
            None,
            None,
        );
        worker.outgoing.running = Some(job(PEER, "queued-fixture"));
        events.try_iter().for_each(drop);
        finish(
            &mut worker,
            "queued-fixture",
            Err(SendFailure::RateLimited { retry_after: 60 }),
        )
        .await;
        let events: Vec<_> = events.try_iter().collect();
        assert!(events.iter().any(|event| matches!(event,
            Event::MessageUpdated(message)
                if message.id == "queued-fixture" && message.status == Delivery::Queued)));
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, Event::Error(_) | Event::SendFailed { .. }))
        );
    }

    #[tokio::test]
    async fn every_forward_waiting_behind_a_cooldown_is_cancellable() {
        let (mut worker, events, _, _) = worker();
        let _directory = attach_offline_client(&mut worker).await;
        worker.outgoing.retry_at = Some(Instant::now() + Duration::from_secs(60));
        let raw = super::super::outgoing_text("Synthetic source".into(), None, &[]);
        use whatsapp_rust::waproto::buffa::Message as _;
        worker.store_message(
            own_message("source", crate::util::now()),
            Some(raw.encode_to_vec()),
            None,
        );
        worker
            .handle_command(Command::Forward {
                from_chat: PEER.into(),
                messages: vec!["source".into(), "source".into()],
                to_chat: PEER.into(),
            })
            .await;
        let rows: Vec<_> = visible_rows(&mut worker, &events)
            .await
            .into_iter()
            .filter(|row| row.forwarded)
            .collect();
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|row| row.status == Delivery::Queued));
        worker
            .handle_command(Command::CancelQueued {
                chat: PEER.into(),
                id: rows[1].id.clone(),
            })
            .await;
        assert!(
            events
                .try_iter()
                .any(|event| matches!(event, Event::MessageDeleted { id, .. } if id == rows[1].id))
        );
        assert!(worker.archive.message(PEER, &rows[1].id).unwrap().is_none());
    }

    /// D1 and D2: a rate limit puts the refused send and everything behind
    /// it into the waiting state; the cooldown drains in order, one at a
    /// time, and only a waiting message can be cancelled.
    #[tokio::test]
    async fn a_rate_limit_waits_out_the_cooldown_in_order_and_cancel_deletes() {
        let (mut worker, events, _, _) = worker();
        let _directory = attach_offline_client(&mut worker).await;
        send_text(&mut worker, "First synthetic message").await;
        send_text(&mut worker, "Second synthetic message").await;
        let first = visible_rows(&mut worker, &events).await;
        assert_eq!(statuses(&first), [Delivery::Pending, Delivery::Pending]);
        finish(
            &mut worker,
            &first[0].id,
            Err(classify_send_error(rate_error(429, Some(60)))),
        )
        .await;
        send_text(&mut worker, "Third synthetic message").await;
        let waiting = visible_rows(&mut worker, &events).await;
        assert_eq!(statuses(&waiting), [Delivery::Queued; 3]);
        assert!(
            !events
                .try_iter()
                .any(|event| matches!(event, Event::Error(_) | Event::SendFailed { .. }))
        );
        worker.pump_outgoing_at(Instant::now() + Duration::from_secs(59));
        assert!(worker.outgoing.running.is_none());
        let cancelled = waiting[1].id.clone();
        worker
            .handle_command(Command::CancelQueued {
                chat: PEER.into(),
                id: cancelled.clone(),
            })
            .await;
        assert!(
            events
                .try_iter()
                .any(|event| matches!(event, Event::MessageDeleted { id, .. } if id == cancelled))
        );
        worker.pump_outgoing_at(Instant::now() + Duration::from_secs(61));
        assert!(events.try_iter().any(|event| matches!(event, Event::MessageUpdated(row)
            if row.id == first[0].id && row.status == Delivery::Pending && row.content == first[0].content)));
        // The running send cannot be cancelled: nothing is deleted.
        worker
            .handle_command(Command::CancelQueued {
                chat: PEER.into(),
                id: first[0].id.clone(),
            })
            .await;
        assert!(
            !events
                .try_iter()
                .any(|event| matches!(event, Event::MessageDeleted { .. }))
        );
        finish(&mut worker, &first[0].id, Ok(())).await;
        let rows = visible_rows(&mut worker, &events).await;
        assert_eq!(statuses(&rows), [Delivery::Sent, Delivery::Pending]);
        assert_eq!(rows[1].id, waiting[2].id);
        assert_eq!(
            worker.outgoing.running.as_ref().map(|job| job.id.as_str()),
            Some(waiting[2].id.as_str())
        );
        worker.stop_bot().await;
    }

    /// A message that waited, went out, and is refused again waits once more
    /// at the time it waited before: it stays ahead of the messages queued
    /// behind it in the same second, in the archive as in the queue.
    #[tokio::test]
    async fn a_send_refused_again_keeps_its_place_ahead_of_the_rows_behind_it() {
        let (mut worker, events, _, _) = worker();
        let _directory = attach_offline_client(&mut worker).await;
        let ids = ["a", "b", "c"];
        for id in ids {
            worker.store_message(
                crate::model::Message {
                    status: Delivery::Pending,
                    ..own_message(id, 100)
                },
                None,
                None,
            );
        }
        worker.outgoing.running = Some(job(PEER, "a"));
        worker.outgoing.waiting = ["b", "c"].map(|id| job(PEER, id)).into();
        let limited = || Err(SendFailure::RateLimited { retry_after: 60 });
        finish(&mut worker, "a", limited()).await;
        worker.pump_outgoing_at(Instant::now() + Duration::from_secs(61));
        assert_eq!(
            worker.outgoing.running.as_ref().map(|job| job.id.as_str()),
            Some("a")
        );
        events.try_iter().for_each(drop);
        finish(&mut worker, "a", limited()).await;
        let requeued = events.try_iter().find_map(|event| match event {
            Event::MessageRequeued(row) => Some(row),
            _ => None,
        });
        assert_eq!(
            requeued.map(|row| (row.id, row.status, row.timestamp)),
            Some(("a".into(), Delivery::Queued, 100))
        );
        let rows = visible_rows(&mut worker, &events).await;
        assert_eq!(
            rows.iter()
                .map(|row| (row.id.as_str(), row.status, row.timestamp))
                .collect::<Vec<_>>(),
            ids.map(|id| (id, Delivery::Queued, 100))
        );
        assert_eq!(
            worker
                .outgoing
                .waiting
                .iter()
                .map(|job| job.id.as_str())
                .collect::<Vec<_>>(),
            ids
        );
        worker.stop_bot().await;
    }

    /// D2: a message the phone sent later, and that the peer read, must not
    /// promote a send that is still in flight; its later rate limit keeps it.
    #[tokio::test]
    async fn a_later_read_receipt_never_promotes_an_in_flight_send() {
        use super::super::{MessageSource, ReceiptType, wa_events};
        let (mut worker, events, _, _) = worker();
        let _directory = attach_offline_client(&mut worker).await;
        send_text(&mut worker, "In flight").await;
        let first = visible_rows(&mut worker, &events).await.remove(0);
        worker.store_message(own_message("from-phone", first.timestamp + 1), None, None);
        let receipt = wa_events::Receipt::builder()
            .message_ids(vec!["from-phone".into()])
            .source(MessageSource {
                chat: PEER.parse().unwrap(),
                sender: PEER.parse().unwrap(),
                ..Default::default()
            })
            .timestamp(whatsapp_rust::wacore::time::now_utc())
            .r#type(ReceiptType::Read)
            .offline(false)
            .build();
        worker.on_receipt(&receipt);
        assert_eq!(
            worker
                .archive
                .message(PEER, "from-phone")
                .unwrap()
                .unwrap()
                .status,
            Delivery::Read
        );
        assert_eq!(
            worker
                .archive
                .message(PEER, &first.id)
                .unwrap()
                .unwrap()
                .status,
            Delivery::Pending
        );
        finish(
            &mut worker,
            &first.id,
            Err(SendFailure::RateLimited { retry_after: 30 }),
        )
        .await;
        assert_eq!(
            worker
                .archive
                .message(PEER, &first.id)
                .unwrap()
                .unwrap()
                .status,
            Delivery::Queued
        );
        assert!(worker.outgoing_deadline().is_some(), "the send is retried");
        assert!(
            !events
                .try_iter()
                .any(|event| matches!(event, Event::MessageUpdated(row)
            if row.id == first.id && row.status == Delivery::Read))
        );
        worker.stop_bot().await;
    }

    /// D2: the first page always carries the waiting rows, however many
    /// messages arrived during the cooldown.
    #[tokio::test]
    async fn waiting_rows_are_on_the_first_page_behind_many_newer_messages() {
        let (mut worker, events, _, _) = worker();
        let _directory = attach_offline_client(&mut worker).await;
        send_text(&mut worker, "Waiting").await;
        let first = visible_rows(&mut worker, &events).await.remove(0);
        finish(
            &mut worker,
            &first.id,
            Err(SendFailure::RateLimited { retry_after: 600 }),
        )
        .await;
        for index in 1..=(crate::app::PAGE as i64 + 2) {
            worker.store_message(
                crate::model::Message {
                    from_me: false,
                    sender: PEER.into(),
                    status: Delivery::None,
                    ..own_message(&format!("incoming-{index}"), first.timestamp + index)
                },
                None,
                None,
            );
        }
        let rows = visible_rows(&mut worker, &events).await;
        let waiting = rows
            .iter()
            .find(|row| row.id == first.id)
            .expect("waiting row on page");
        assert_eq!(waiting.status, Delivery::Queued);
        worker.stop_bot().await;
    }

    fn disconnected() -> LinkStatus {
        LinkStatus::Disconnected {
            reason: String::new(),
        }
    }

    /// Each failure toast since the last call: whether it names the connection.
    fn send_failures(events: &std::sync::mpsc::Receiver<Event>) -> Vec<bool> {
        events
            .try_iter()
            .filter_map(|event| match event {
                Event::SendFailed { connection } => Some(connection),
                _ => None,
            })
            .collect()
    }

    /// D6: without a working link, a send that is not rate limited fails at
    /// once, as before the queue: a Failed row and one toast, nothing kept.
    #[tokio::test]
    async fn a_send_while_the_link_is_down_fails_at_once() {
        for status in [disconnected(), LinkStatus::Connecting] {
            let (mut worker, events, _, _) = worker();
            let _directory = attach_offline_client(&mut worker).await;
            worker.status = status;
            send_text(&mut worker, "Offline synthetic message").await;
            assert_eq!(send_failures(&events), [true]);
            assert_eq!(
                statuses(&visible_rows(&mut worker, &events).await),
                [Delivery::Failed]
            );
            assert!(worker.outgoing.running.is_none());
            assert!(worker.outgoing.waiting.is_empty());
            worker.stop_bot().await;
        }
    }

    /// D6: a send that fails while the link is down takes the messages
    /// behind it along, with one toast. None stays pending until a reconnect.
    #[tokio::test]
    async fn a_failure_while_disconnected_fails_every_message_behind_it() {
        let (mut worker, events, _, _) = worker();
        let _directory = attach_offline_client(&mut worker).await;
        send_text(&mut worker, "On the wire").await;
        send_text(&mut worker, "Never left").await;
        let rows = visible_rows(&mut worker, &events).await;
        worker.status = disconnected();
        finish(
            &mut worker,
            &rows[0].id,
            Err(classify_send_error(SendError::Iq(IqError::NotConnected))),
        )
        .await;
        assert_eq!(send_failures(&events), [true]);
        assert_eq!(
            statuses(&visible_rows(&mut worker, &events).await),
            [Delivery::Failed, Delivery::Failed]
        );
        assert!(worker.outgoing.running.is_none());
        assert!(worker.outgoing.waiting.is_empty());
        worker.set_status(LinkStatus::Connected);
        assert!(worker.outgoing.running.is_none(), "nothing is resent");
        worker.stop_bot().await;
    }

    /// D6: when the link drops, a message behind the running send fails at
    /// once. The running send still reports for itself.
    #[tokio::test]
    async fn a_lost_link_fails_the_messages_behind_the_running_send_at_once() {
        let (mut worker, events, _, _) = worker();
        let _directory = attach_offline_client(&mut worker).await;
        send_text(&mut worker, "On the wire").await;
        send_text(&mut worker, "Never left").await;
        let rows = visible_rows(&mut worker, &events).await;
        worker.set_status(disconnected());
        assert_eq!(send_failures(&events), [true]);
        assert_eq!(
            statuses(&visible_rows(&mut worker, &events).await),
            [Delivery::Pending, Delivery::Failed]
        );
        finish(&mut worker, &rows[0].id, Ok(())).await;
        assert_eq!(
            statuses(&visible_rows(&mut worker, &events).await),
            [Delivery::Sent, Delivery::Failed]
        );
        worker.stop_bot().await;
    }

    /// D6 exception: messages already waiting out a rate limit keep waiting
    /// through a lost link, with their Cancel, and go out in order once the
    /// link is back and the cooldown is over. A new send without a link
    /// still fails at once.
    #[tokio::test]
    async fn waiting_messages_keep_waiting_through_a_lost_link() {
        let (mut worker, events, _, _) = worker();
        let _directory = attach_offline_client(&mut worker).await;
        send_text(&mut worker, "First synthetic message").await;
        send_text(&mut worker, "Second synthetic message").await;
        let rows = visible_rows(&mut worker, &events).await;
        finish(
            &mut worker,
            &rows[0].id,
            Err(SendFailure::RateLimited { retry_after: 60 }),
        )
        .await;
        worker.set_status(disconnected());
        assert!(send_failures(&events).is_empty());
        assert_eq!(
            statuses(&visible_rows(&mut worker, &events).await),
            [Delivery::Queued, Delivery::Queued]
        );
        send_text(&mut worker, "Third synthetic message").await;
        assert_eq!(send_failures(&events), [true]);
        let status = |rows: &[crate::model::Message], id: &str| {
            rows.iter().find(|row| row.id == id).map(|row| row.status)
        };
        let after = visible_rows(&mut worker, &events).await;
        assert_eq!(after.len(), 3);
        assert_eq!(status(&after, &rows[0].id), Some(Delivery::Queued));
        assert_eq!(status(&after, &rows[1].id), Some(Delivery::Queued));
        assert!(
            after
                .iter()
                .any(|row| !rows.iter().any(|old| old.id == row.id)
                    && row.status == Delivery::Failed)
        );
        worker.set_status(LinkStatus::Connected);
        assert!(worker.outgoing.running.is_none(), "the cooldown still runs");
        worker.pump_outgoing_at(Instant::now() + Duration::from_secs(61));
        assert_eq!(
            worker.outgoing.running.as_ref().map(|job| job.id.as_str()),
            Some(rows[0].id.as_str())
        );
        worker.stop_bot().await;
    }

    #[tokio::test]
    async fn only_typed_rate_refusals_retry_and_real_failures_have_safe_copy() {
        // The toast names the connection only when the link was the cause.
        for (error, expected, toast) in [
            (
                rate_error(429, None),
                SendFailure::RateLimited { retry_after: 60 },
                None,
            ),
            (
                rate_error(429, Some(0)),
                SendFailure::RateLimited { retry_after: 1 },
                None,
            ),
            (
                rate_error(403, Some(60)),
                SendFailure::Failed {
                    kind: "iq server error",
                    code: Some(403),
                },
                Some(false),
            ),
            (
                SendError::Iq(IqError::Timeout),
                SendFailure::Failed {
                    kind: "iq timeout",
                    code: None,
                },
                Some(true),
            ),
            (
                SendError::Iq(IqError::NotConnected),
                SendFailure::Failed {
                    kind: "iq not connected",
                    code: None,
                },
                Some(true),
            ),
            (
                SendError::Internal(anyhow::anyhow!("429 private upstream detail")),
                SendFailure::Failed {
                    kind: "internal",
                    code: None,
                },
                Some(false),
            ),
        ] {
            let failure = classify_send_error(error);
            assert_eq!(failure, expected);
            let queued = matches!(failure, SendFailure::RateLimited { .. });
            let (mut worker, events, _, _) = worker();
            let _directory = attach_offline_client(&mut worker).await;
            worker.store_message(
                crate::model::Message {
                    status: Delivery::Pending,
                    ..own_message("attempt", 1)
                },
                None,
                None,
            );
            worker.outgoing.running = Some(job(PEER, "attempt"));
            finish(&mut worker, "attempt", Err(failure)).await;
            let observed: Vec<_> = events.try_iter().collect();
            assert!(
                observed
                    .iter()
                    .any(|event| matches!(event, Event::MessageUpdated(row)
                if row.status == if queued { Delivery::Queued } else { Delivery::Failed }))
            );
            assert_eq!(
                observed
                    .iter()
                    .filter_map(|event| match event {
                        Event::SendFailed { connection } => Some(*connection),
                        _ => None,
                    })
                    .collect::<Vec<_>>(),
                Vec::from_iter(toast)
            );
            assert!(
                !observed
                    .iter()
                    .any(|event| matches!(event, Event::Error(_)))
            );
        }
    }

    #[tokio::test]
    async fn stopping_the_bot_fails_waiting_rows_and_ignores_stale_completions() {
        let (mut worker, events, _, _) = worker();
        let _directory = attach_offline_client(&mut worker).await;
        send_text(&mut worker, "In flight").await;
        send_text(&mut worker, "Still queued").await;
        send_text(&mut worker, "Also queued").await;
        let before = visible_rows(&mut worker, &events).await;
        worker.stop_bot().await;
        // One toast for every row that failed, not one per row.
        assert_eq!(send_failures(&events), [true]);
        finish(
            &mut worker,
            &before[0].id,
            Err(SendFailure::RateLimited { retry_after: 1 }),
        )
        .await;
        let after = visible_rows(&mut worker, &events).await;
        assert_eq!(after[0].status, Delivery::Unconfirmed);
        assert_eq!(after[1].status, Delivery::Failed);
        assert_eq!(after[0].content, before[0].content);
        assert_eq!(after[1].content, before[1].content);
        let _reconnected = attach_offline_client(&mut worker).await;
        worker.pump_outgoing_at(Instant::now() + Duration::from_secs(120));
        assert!(worker.outgoing_deadline().is_none());
        assert_eq!(
            visible_rows(&mut worker, &events).await[0].status,
            Delivery::Unconfirmed
        );
        worker
            .handle_command(Command::DeleteLocal {
                chat: PEER.into(),
                id: before[0].id.clone(),
            })
            .await;
        assert!(
            events.try_iter().any(
                |event| matches!(event, Event::MessageDeleted { id, .. } if id == before[0].id)
            )
        );
        worker.stop_bot().await;
    }

    #[tokio::test]
    async fn historical_pending_is_not_presented_as_a_live_send() {
        use super::super::{ParsedHistory, parse_conversation};
        use whatsapp_rust::waproto::buffa::MessageField;
        let (mut worker, events, _, _) = worker();
        worker.apply_history(
            ParsedHistory {
                chats: vec![parse_conversation(wa::Conversation {
                    id: PEER.into(),
                    messages: vec![wa::HistorySyncMsg {
                        message: MessageField::some(wa::WebMessageInfo {
                            key: MessageField::some(wa::MessageKey {
                                id: Some("historical-pending".into()),
                                from_me: Some(true),
                                ..Default::default()
                            }),
                            message: MessageField::some(wa::Message {
                                conversation: Some("Synthetic interrupted history".into()),
                                ..Default::default()
                            }),
                            status: Some(wa::web_message_info::Status::PENDING),
                            ..Default::default()
                        }),
                        ..Default::default()
                    }],
                    ..Default::default()
                })],
                push_names: Vec::new(),
                lids: Vec::new(),
                stickers: Vec::new(),
            },
            true,
        );
        let rows = visible_rows(&mut worker, &events).await;
        assert_eq!(rows[0].status, Delivery::Unconfirmed);
        assert!(worker.outgoing_deadline().is_none());
    }

    #[tokio::test]
    async fn abandoned_sends_keep_known_receipts_and_accept_late_direct_and_group_receipts() {
        use super::super::{MessageSource, ReceiptType, wa_events};
        for chat in [PEER, "120363000000000001@g.us"] {
            for initial in [
                Delivery::Pending,
                Delivery::Sent,
                Delivery::Delivered,
                Delivery::Read,
                Delivery::Played,
            ] {
                let (mut worker, events, _, _) = worker();
                let row = crate::model::Message {
                    chat: chat.into(),
                    status: initial,
                    ..own_message("interrupted", 100)
                };
                worker.store_message(row, None, None);
                let grouped = chat.ends_with("@g.us");
                let recipients = if grouped {
                    vec![PEER, "200@s.whatsapp.net"]
                } else {
                    vec![PEER]
                };
                if grouped {
                    let (stored, mut ack) = tokio::sync::mpsc::unbounded_channel();
                    worker
                        .handle_command(Command::GroupRecipients {
                            chat: chat.into(),
                            id: "interrupted".into(),
                            recipients: recipients.iter().map(|s| (*s).to_owned()).collect(),
                            lids: Vec::new(),
                            stored,
                        })
                        .await;
                    assert_eq!(ack.recv().await, Some(true));
                }
                worker.outgoing.running = Some(job(chat, "interrupted"));
                events.try_iter().for_each(drop);
                worker.stop_bot().await;
                let observed: Vec<_> = events.try_iter().collect();
                // A running send is unconfirmed, not failed: no toast.
                assert!(
                    !observed
                        .iter()
                        .any(|event| matches!(event, Event::SendFailed { .. }))
                );
                let released = observed
                    .into_iter()
                    .find_map(|event| match event {
                        Event::MessageUpdated(row) => Some(row.status),
                        _ => None,
                    })
                    .unwrap();
                assert_eq!(
                    released,
                    if initial == Delivery::Pending {
                        Delivery::Unconfirmed
                    } else {
                        initial
                    }
                );
                let mut confirmed = released;
                for (kind, expected) in [
                    (ReceiptType::Delivered, Delivery::Delivered),
                    (ReceiptType::Read, Delivery::Read),
                    (ReceiptType::Played, Delivery::Played),
                    (ReceiptType::Delivered, Delivery::Played),
                ] {
                    for sender in &recipients {
                        let receipt = wa_events::Receipt::builder()
                            .message_ids(vec!["interrupted".into()])
                            .source(MessageSource {
                                chat: chat.parse().unwrap(),
                                sender: sender.parse().unwrap(),
                                is_group: grouped,
                                ..Default::default()
                            })
                            .timestamp(whatsapp_rust::wacore::time::now_utc())
                            .r#type(kind.clone())
                            .offline(false)
                            .build();
                        worker
                            .handle_wa_event(std::sync::Arc::new(wa_events::Event::Receipt(
                                receipt,
                            )))
                            .await;
                        if grouped && *sender == PEER {
                            worker
                                .handle_command(Command::LoadChat {
                                    chat: chat.into(),
                                    before: None,
                                })
                                .await;
                            let partial = events
                                .try_iter()
                                .find_map(|event| match event {
                                    Event::Messages { messages, .. } => {
                                        messages.into_iter().find(|row| row.id == "interrupted")
                                    }
                                    _ => None,
                                })
                                .unwrap();
                            assert_eq!(
                                partial.status, confirmed,
                                "one recipient must not restore Sending or advance the group"
                            );
                        }
                    }
                    worker
                        .handle_command(Command::LoadChat {
                            chat: chat.into(),
                            before: None,
                        })
                        .await;
                    let observed = events
                        .try_iter()
                        .find_map(|event| match event {
                            Event::Messages { messages, .. } => {
                                messages.into_iter().find(|row| row.id == "interrupted")
                            }
                            _ => None,
                        })
                        .unwrap();
                    confirmed = expected.max(initial);
                    assert_eq!(observed.status, confirmed);
                }
                assert!(worker.outgoing_deadline().is_none());
                worker
                    .handle_command(Command::DeleteLocal {
                        chat: chat.into(),
                        id: "interrupted".into(),
                    })
                    .await;
                // Receipts confirmed this row, so main's delete-for-me
                // sync requires the phone's acceptance before local removal.
                assert!(!events.try_iter().any(
                    |event| matches!(event, Event::MessageDeleted { id, .. } if id == "interrupted")
                ));
                assert!(
                    worker
                        .archive
                        .message(chat, "interrupted")
                        .unwrap()
                        .is_some()
                );
                worker
                    .handle_command(Command::MessageDeletedForMe {
                        generation: worker.privacy_generation,
                        chat: chat.into(),
                        id: "interrupted".into(),
                        outcome: super::super::super::MessageRemovalOutcome::Accepted,
                    })
                    .await;
                assert!(events.try_iter().any(
                    |event| matches!(event, Event::MessageDeleted { id, .. } if id == "interrupted")
                ));
            }
        }
    }

    #[tokio::test]
    async fn a_late_failure_never_regresses_an_observed_delivery_receipt() {
        let (mut worker, events, _, _) = worker();
        let _directory = attach_offline_client(&mut worker).await;
        send_text(&mut worker, "Receipt fixture").await;
        let first = visible_rows(&mut worker, &events).await.remove(0);
        worker
            .archive
            .set_status(PEER, &first.id, Delivery::Read, crate::util::now())
            .unwrap();
        finish(
            &mut worker,
            &first.id,
            Err(SendFailure::RateLimited { retry_after: 1 }),
        )
        .await;
        assert_eq!(
            visible_rows(&mut worker, &events).await[0].status,
            Delivery::Read
        );
        assert!(worker.outgoing_deadline().is_none());
        worker.stop_bot().await;
    }

    #[tokio::test]
    async fn a_group_retry_uses_the_new_attempts_audience_not_the_refused_attempt() {
        let (mut worker, events, _, _) = worker();
        let group = "120363000000000001@g.us";
        worker.store_message(
            crate::model::Message {
                chat: group.into(),
                status: Delivery::Pending,
                ..own_message("audience-fixture", 1)
            },
            None,
            None,
        );
        worker.outgoing.running = Some(job(group, "audience-fixture"));
        for (attempt, recipients) in [
            (0, vec!["100@s.whatsapp.net", "200@s.whatsapp.net"]),
            (1, vec!["200@s.whatsapp.net", "300@s.whatsapp.net"]),
        ] {
            let (stored, mut ack) = tokio::sync::mpsc::unbounded_channel();
            worker
                .handle_command(Command::GroupRecipients {
                    chat: group.into(),
                    id: "audience-fixture".into(),
                    recipients: recipients.into_iter().map(str::to_owned).collect(),
                    lids: Vec::new(),
                    stored,
                })
                .await;
            assert_eq!(ack.recv().await, Some(true));
            if attempt == 0 {
                worker
                    .handle_command(Command::OutgoingFinished {
                        chat: group.into(),
                        id: "audience-fixture".into(),
                        result: Err(SendFailure::RateLimited { retry_after: 60 }),
                    })
                    .await;
            }
        }
        worker
            .handle_command(Command::WatchReceipts(Some((
                group.into(),
                "audience-fixture".into(),
            ))))
            .await;
        let observed = events
            .try_iter()
            .find_map(|event| match event {
                Event::Receipts(receipts) => Some(receipts),
                _ => None,
            })
            .unwrap();
        assert_eq!(
            observed
                .recipients
                .iter()
                .map(|recipient| recipient.id.as_str())
                .collect::<Vec<_>>(),
            ["200@s.whatsapp.net", "300@s.whatsapp.net"]
        );
    }
}
