//! A chat's disappearing-message timer: changing it, and the notice a change
//! leaves in the conversation.
//!
//! A direct change is an `EPHEMERAL_SETTING` message. The send's `Ok` only
//! means the stanza left the socket, so the timer is written from the server's
//! ack, through the worker's shared ack map (`AwaitingAck::Timer`). A group
//! change is an IQ, and the server's own group notification carries the
//! change's timestamp and actor; the accepted IQ only refreshes the metadata,
//! so a newer change is never overwritten and no notice is invented.
//!
//! Both paths file one system row, whose id is the change's own, so a
//! redelivery replaces it rather than making a second one.

use super::*;

/// How long a sent timer change waits for its server ack before the control
/// is re-enabled and the change is reported as failed.
const TIMER_ACK_PATIENCE: Duration = Duration::from_secs(30);
/// How long after that a late ack is still applied, without a second report.
const TIMER_LATE_ACK_PATIENCE: Duration = Duration::from_secs(5 * 60);
/// How long an ack that outran its `TimerSet` is held, and how many.
const EARLY_ACK_TTL: Duration = Duration::from_secs(60);
const EARLY_ACK_LIMIT: usize = 256;

/// A direct timer change whose sent message is waiting for the server's ack.
#[derive(Clone)]
pub(super) struct TimerAck {
    chat: ChatId,
    seconds: u32,
    /// When the send built the setting, the clock the archive compares against
    /// the phone's settings. The ack's own time is a different clock and
    /// ordering by it could overwrite a newer change made on the phone.
    sent_at: i64,
    /// When to report the change as unanswered.
    deadline: Instant,
    /// The deadline was already reported: the control is enabled again, and an
    /// ack that arrives later still applies the timer without a second report.
    reported: bool,
}

/// A server ack that arrived before its `TimerSet`, held until the send names
/// the id it belongs to. Exact id matching keeps an ordinary message's ack
/// from being mistaken for a timer change.
pub(super) struct EarlyAck {
    pub(super) error: Option<String>,
    at: Instant,
}

impl Worker {
    /// Changes a chat's disappearing-message timer using the direct or group
    /// WhatsApp API. A direct change is written only once the server acks it;
    /// a group's is written from the server's group notification.
    pub(super) fn set_disappearing_timer(&mut self, chat: ChatId, seconds: u32) {
        let is_group = ChatKind::from_id(&chat) == ChatKind::Group;
        // A member of a locked group gets no menu, and one that asks anyway
        // gets nothing sent: WhatsApp would refuse the change.
        if is_group
            && !self
                .archive
                .chat(&chat)
                .ok()
                .flatten()
                .is_some_and(|row| row.can_edit_info())
        {
            self.emit(Event::Error(
                "Only admins can change disappearing messages.".to_owned(),
            ));
            return;
        }
        // A send on a dropped socket would fail as "client is not connected";
        // say to reconnect instead, as the other send paths do.
        let (Some(client), Some(jid)) = (self.connected_client(), Self::jid_of(&chat)) else {
            self.emit(Event::Error(
                "Connect to WhatsApp to change disappearing messages.".to_owned(),
            ));
            return;
        };
        self.timer_saving(&chat, true);
        let session = self.account_session;
        let commands = self.commands.clone();
        let waker = self.waker.clone();
        tokio::spawn(async move {
            if is_group {
                let result = client
                    .groups()
                    .set_ephemeral(jid, seconds)
                    .await
                    .map_err(|error| error.to_string());
                let _ = commands.send(Command::GroupEdited {
                    chat,
                    edit: GroupEdit::Timer(seconds),
                    session,
                    result,
                });
            } else {
                // The timer's own setting timestamp versions it; the library
                // embeds it in the message it sent, so read it from there
                // rather than from any other clock.
                let result = client
                    .set_chat_disappearing_timer(jid, seconds)
                    .await
                    .map(|sent| {
                        let setting = sent
                            .message
                            .get_base_message()
                            .protocol_message
                            .as_option()
                            .and_then(|protocol| protocol.ephemeral_setting_timestamp);
                        (
                            sent.message_id.to_string(),
                            setting.unwrap_or_else(crate::util::now),
                        )
                    })
                    .map_err(|error| error.to_string());
                let _ = commands.send(Command::TimerSet {
                    chat,
                    session,
                    seconds,
                    result,
                });
            }
            waker.wake();
        });
    }

    /// Tells the interface whether a timer change for `chat` is in flight. A
    /// privacy-id mapping learned mid-flight can change the chat's id, so both
    /// the stored id and the canonical one hear about it.
    pub(super) fn timer_saving(&self, chat: &ChatId, saving: bool) {
        self.emit(Event::TimerSaving {
            chat: chat.clone(),
            saving,
        });
        let canonical = self.canonical_str(chat);
        if canonical != *chat {
            self.emit(Event::TimerSaving {
                chat: canonical,
                saving,
            });
        }
    }

    /// Takes a direct timer change off the wire. `Ok` carries the sent
    /// message's id and setting time: the change is written only when the
    /// server acks it, so the control stays disabled until then. A send that
    /// never left the socket reports and re-enables it.
    pub(super) fn timer_set(
        &mut self,
        chat: ChatId,
        session: u64,
        seconds: u32,
        result: Result<(String, i64), String>,
    ) {
        if session != self.account_session {
            return;
        }
        match result {
            Ok((id, sent_at)) => {
                self.awaiting_ack.insert(
                    id.clone(),
                    AwaitingAck::Timer(TimerAck {
                        chat,
                        seconds,
                        sent_at,
                        deadline: Instant::now() + TIMER_ACK_PATIENCE,
                        reported: false,
                    }),
                );
                // An ack that arrived before this completion waited by id.
                if let Some(early) = self.take_early_ack(&id) {
                    self.resolve_timer_ack(&id, early.error.as_deref());
                }
            }
            Err(error) => {
                self.timer_saving(&chat, false);
                log::warn!("could not change disappearing messages ({error})");
                self.emit(Event::Error(
                    "Could not change disappearing messages.".to_owned(),
                ));
            }
        }
    }

    /// Applies the server's ack of a direct timer change, or reports its nack.
    /// Only an entry this map holds is settled, so an ordinary message's ack
    /// can never be taken for a timer change.
    pub(super) fn settle_timer_ack(&mut self, ack: &wa_events::ServerAck) {
        self.resolve_timer_ack(&ack.id, ack.error.as_deref());
    }

    /// Writes the change an ack accepted. A privacy-id mapping learned since
    /// the send may have merged the chat, so the destination is resolved
    /// again here, and the notice stays out of a range the reader cleared.
    fn resolve_timer_ack(&mut self, id: &str, error: Option<&str>) {
        let Some(AwaitingAck::Timer(pending)) = self.awaiting_ack.remove(id) else {
            return;
        };
        // A retry started after this change timed out may still be waiting for
        // its own answer, and its answer or deadline re-enables the control.
        // Clearing it here would let a third change start over the retry.
        let chat = self.canonical_str(&pending.chat);
        let newer_waiting = self.awaiting_ack.values().any(|awaiting| match awaiting {
            AwaitingAck::Timer(other) => !other.reported && self.canonical_str(&other.chat) == chat,
            AwaitingAck::Message(_) => false,
        });
        // A timed-out attempt already cleared saving at its deadline. A retry
        // may now be sending but not yet registered in awaiting_ack.
        if !pending.reported && !newer_waiting {
            self.timer_saving(&pending.chat, false);
        }
        if let Some(error) = error {
            if !pending.reported {
                log::warn!("WhatsApp refused a disappearing-messages change ({error})");
                self.emit(Event::Error(
                    "Could not change disappearing messages.".to_owned(),
                ));
            }
            return;
        }
        if self
            .archive
            .set_ephemeral(&chat, pending.seconds, pending.sent_at)
            .unwrap_or(false)
        {
            self.file_timer_notice(
                &chat,
                &format!("{}{}", crate::model::TIMER_NOTICE_PREFIX, id),
                self.me(),
                true,
                pending.sent_at,
                pending.seconds,
                None,
            );
            self.emit_chat(&chat);
        }
    }

    /// A connection stop drops its pending acks, so no deadline remains to
    /// release their controls. Notify both aliases before discarding them.
    pub(super) fn clear_timer_saving(&self) {
        for awaiting in self.awaiting_ack.values() {
            if let AwaitingAck::Timer(timer) = awaiting {
                self.timer_saving(&timer.chat, false);
            }
        }
    }

    /// Reports timer changes the server never acked, re-enabling the control.
    /// The entry is kept a while longer, so an ack that arrives late still
    /// applies the timer it accepted without a second report; only then is the
    /// entry dropped. Held acks age out here too, so their bound holds without
    /// traffic.
    pub(super) fn expire_timer_acks(&mut self) {
        let now = Instant::now();
        self.early_acks
            .retain(|_, held| now.duration_since(held.at) < EARLY_ACK_TTL);
        let due: Vec<(String, ChatId, bool)> = self
            .awaiting_ack
            .iter()
            .filter_map(|(id, awaiting)| match awaiting {
                AwaitingAck::Timer(pending) if pending.deadline <= now => {
                    Some((id.clone(), pending.chat.clone(), pending.reported))
                }
                _ => None,
            })
            .collect();
        for (id, chat, reported) in due {
            if reported {
                self.awaiting_ack.remove(&id);
                continue;
            }
            if let Some(AwaitingAck::Timer(pending)) = self.awaiting_ack.get_mut(&id) {
                pending.reported = true;
                pending.deadline = now + TIMER_LATE_ACK_PATIENCE;
            }
            self.timer_saving(&chat, false);
            self.emit(Event::Error(
                "Could not change disappearing messages.".to_owned(),
            ));
        }
    }

    /// Holds a message ack that matches nothing yet, bounded in age and count,
    /// so an ack that outran its `TimerSet` still lands.
    pub(super) fn hold_early_ack(&mut self, ack: &wa_events::ServerAck) {
        let now = Instant::now();
        self.early_acks
            .retain(|_, held| now.duration_since(held.at) < EARLY_ACK_TTL);
        if self.early_acks.len() >= EARLY_ACK_LIMIT
            && let Some(oldest) = self
                .early_acks
                .iter()
                .min_by_key(|(_, held)| held.at)
                .map(|(id, _)| id.clone())
        {
            self.early_acks.remove(&oldest);
        }
        self.early_acks.insert(
            ack.id.clone(),
            EarlyAck {
                error: ack.error.clone(),
                at: now,
            },
        );
    }

    /// Takes a held ack for `id`, unless it sat past its age bound.
    fn take_early_ack(&mut self, id: &str) -> Option<EarlyAck> {
        let held = self.early_acks.remove(id)?;
        (Instant::now().duration_since(held.at) < EARLY_ACK_TTL).then_some(held)
    }

    /// Files the system row for a timer change the archive accepted. The row
    /// shares the change's stable id, so a redelivery replaces it rather than
    /// making a second notice.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn file_timer_notice(
        &mut self,
        chat: &str,
        id: &str,
        sender: String,
        from_me: bool,
        timestamp: i64,
        seconds: u32,
        push_name: Option<&str>,
    ) {
        // A notice removed by clearing or deleting the chat must stay gone,
        // even if the same change is delivered again.
        if self.predates_removal(chat, timestamp) {
            return;
        }
        let notice = Message {
            id: id.to_owned(),
            chat: chat.to_owned(),
            sender,
            sender_name: if from_me {
                None
            } else {
                push_name.map(str::to_owned)
            },
            from_me,
            timestamp,
            content: Content::DisappearingTimer { seconds },
            status: Delivery::None,
            delivered_at: None,
            read_at: None,
            quoted: None,
            reactions: Vec::new(),
            edited: false,
            mentions: Vec::new(),
            forwarded: false,
            thumbnail: None,
            history_order: None,
        };
        if let Err(error) = self.archive.insert_message(&notice, None) {
            log::warn!("could not file a disappearing-messages notice: {error}");
            return;
        }
        self.emit(Event::Messages {
            chat: chat.to_owned(),
            messages: vec![notice],
            older: false,
            complete: false,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::super::receipt_tests::{PEER, worker};
    use super::*;

    const ME: &str = "15550001111@s.whatsapp.net";

    /// A direct timer change is written only once the server acks the sent
    /// message, and with the ack's own timestamp; a send that never left the
    /// socket, a nack, and silence all leave the old timer and report.
    #[tokio::test]
    async fn a_direct_timer_change_lands_only_when_the_server_acks_it() {
        let (mut worker, events, _inbox, _wa) = worker();
        worker.ensure_chat(PEER, None);
        worker.archive.set_ephemeral(PEER, 86_400, 100).unwrap();
        let timer = |worker: &Worker| {
            worker
                .archive
                .chat(PEER)
                .unwrap()
                .unwrap()
                .disappearing_timer()
        };
        let ack = |id: &str, at: i64| {
            wa_events::ServerAck::builder()
                .id(id.to_owned())
                .class("message".to_owned())
                .timestamp(whatsapp_rust::wacore::time::from_secs(at).unwrap())
                .build()
        };

        // A send that reached the socket is not yet an accepted change.
        let session = worker.account_session;
        worker
            .handle_command(Command::TimerSet {
                chat: PEER.into(),
                session,
                seconds: 604_800,
                result: Ok(("sent-1".into(), 100)),
            })
            .await;
        assert_eq!(
            timer(&worker),
            Some(86_400),
            "nothing is written before ack"
        );

        // The server's ack applies it, stamped by the server's own time.
        worker
            .handle_wa_event(Arc::new(wa_events::Event::ServerAck(ack("sent-1", 150))))
            .await;
        assert_eq!(timer(&worker), Some(604_800));

        // A newer phone change survives a late ack from before it.
        worker.archive.set_ephemeral(PEER, 0, 200).unwrap();
        worker
            .handle_command(Command::TimerSet {
                chat: PEER.into(),
                session,
                seconds: 604_800,
                result: Ok(("sent-2".into(), 100)),
            })
            .await;
        // The ack's own timestamp is later than the phone's change; only the
        // send's own time may version ours, so the phone still wins.
        worker
            .handle_wa_event(Arc::new(wa_events::Event::ServerAck(ack("sent-2", 250))))
            .await;
        assert_eq!(timer(&worker), None, "the phone's newer change wins");

        // A nack, and a send that never left the socket, change nothing.
        worker
            .handle_command(Command::TimerSet {
                chat: PEER.into(),
                session,
                seconds: 86_400,
                result: Ok(("sent-3".into(), 100)),
            })
            .await;
        let nack = wa_events::ServerAck::builder()
            .id("sent-3".to_owned())
            .class("message".to_owned())
            .error("479".to_owned())
            .build();
        worker
            .handle_wa_event(Arc::new(wa_events::Event::ServerAck(nack)))
            .await;
        assert_eq!(timer(&worker), None);
        worker
            .handle_command(Command::TimerSet {
                chat: PEER.into(),
                session,
                seconds: 86_400,
                result: Err("boom".into()),
            })
            .await;
        assert_eq!(timer(&worker), None);

        let errors: Vec<_> = events
            .try_iter()
            .filter_map(|event| match event {
                Event::Error(message) => Some(message),
                _ => None,
            })
            .collect();
        assert!(
            errors.contains(&"Could not change disappearing messages.".to_owned()),
            "{errors:?}"
        );
    }

    /// An ack can outrun the send's own completion. It is held by id, then
    /// applied when the completion names it; an ordinary message's ack for the
    /// same chat must never be mistaken for the timer's.
    #[tokio::test]
    async fn an_ack_that_outruns_the_send_completion_still_applies() {
        let (mut worker, _events, _inbox, _wa) = worker();
        worker.ensure_chat(PEER, None);
        worker.archive.set_ephemeral(PEER, 86_400, 100).unwrap();
        let session = worker.account_session;
        let early = wa_events::ServerAck::builder()
            .id("sent-early".to_owned())
            .class("message".to_owned())
            .timestamp(whatsapp_rust::wacore::time::from_secs(150).unwrap())
            .build();
        worker
            .handle_wa_event(Arc::new(wa_events::Event::ServerAck(early)))
            .await;
        assert_eq!(
            worker
                .archive
                .chat(PEER)
                .unwrap()
                .unwrap()
                .disappearing_timer(),
            Some(86_400),
            "an ack with no id matching yet is held, not applied"
        );
        // The completion names the id, so the held ack resolves it now.
        worker
            .handle_command(Command::TimerSet {
                chat: PEER.into(),
                session,
                seconds: 604_800,
                result: Ok(("sent-early".into(), 100)),
            })
            .await;
        assert_eq!(
            worker
                .archive
                .chat(PEER)
                .unwrap()
                .unwrap()
                .disappearing_timer(),
            Some(604_800)
        );
        assert!(worker.awaiting_ack.is_empty());
    }

    /// An ordinary message's ack or nack for the same chat must not touch a
    /// timer change that is still waiting for its own ack.
    #[tokio::test]
    async fn an_ordinary_message_ack_does_not_resolve_a_timer_change() {
        let (mut worker, _events, _inbox, _wa) = worker();
        worker.ensure_chat(PEER, None);
        worker.archive.set_ephemeral(PEER, 86_400, 100).unwrap();
        let session = worker.account_session;
        worker
            .handle_command(Command::TimerSet {
                chat: PEER.into(),
                session,
                seconds: 604_800,
                result: Ok(("timer-1".into(), 100)),
            })
            .await;
        // A different message's nack for the same chat.
        let nack = wa_events::ServerAck::builder()
            .id("an-ordinary-message".to_owned())
            .class("message".to_owned())
            .from(PEER.parse().unwrap())
            .error("479".to_owned())
            .build();
        worker
            .handle_wa_event(Arc::new(wa_events::Event::ServerAck(nack)))
            .await;
        assert!(
            worker.awaiting_ack.values().any(
                |awaiting| matches!(awaiting, AwaitingAck::Timer(pending) if pending.chat == PEER)
            ),
            "the timer still waits for its own ack"
        );
        assert_eq!(
            worker
                .archive
                .chat(PEER)
                .unwrap()
                .unwrap()
                .disappearing_timer(),
            Some(86_400)
        );
        // The timer's own ack still applies it.
        let ack = wa_events::ServerAck::builder()
            .id("timer-1".to_owned())
            .class("message".to_owned())
            .timestamp(whatsapp_rust::wacore::time::from_secs(150).unwrap())
            .build();
        worker
            .handle_wa_event(Arc::new(wa_events::Event::ServerAck(ack)))
            .await;
        assert_eq!(
            worker
                .archive
                .chat(PEER)
                .unwrap()
                .unwrap()
                .disappearing_timer(),
            Some(604_800)
        );
    }

    /// A retry does not forget a change the server already accepted: if the
    /// later attempt fails, the earlier attempt's late ack still applies it,
    /// ordered by the archive's own timestamp gate.
    #[tokio::test]
    async fn a_retry_keeps_an_earlier_attempts_accepted_outcome() {
        let (mut worker, _events, _inbox, _wa) = worker();
        worker.ensure_chat(PEER, None);
        worker.archive.set_ephemeral(PEER, 86_400, 100).unwrap();
        let session = worker.account_session;
        // A asks for seven days; its send completes and its ack is delayed.
        worker
            .handle_command(Command::TimerSet {
                chat: PEER.into(),
                session,
                seconds: 604_800,
                result: Ok(("a-1".into(), 100)),
            })
            .await;
        // A times out and the user retries with Off, which fails.
        worker
            .handle_command(Command::TimerSet {
                chat: PEER.into(),
                session,
                seconds: 0,
                result: Err("boom".into()),
            })
            .await;
        assert_eq!(
            worker
                .archive
                .chat(PEER)
                .unwrap()
                .unwrap()
                .disappearing_timer(),
            Some(86_400),
            "the failed retry changes nothing"
        );
        // A's accepted change is still the truth.
        let ack = wa_events::ServerAck::builder()
            .id("a-1".to_owned())
            .class("message".to_owned())
            .timestamp(whatsapp_rust::wacore::time::from_secs(150).unwrap())
            .build();
        worker
            .handle_wa_event(Arc::new(wa_events::Event::ServerAck(ack)))
            .await;
        assert_eq!(
            worker
                .archive
                .chat(PEER)
                .unwrap()
                .unwrap()
                .disappearing_timer(),
            Some(604_800),
            "the accepted earlier change still lands"
        );
        assert!(worker.awaiting_ack.is_empty());
    }

    /// A late ack still applies the change it accepted, after the wait had
    /// already re-enabled the control and reported.
    #[tokio::test]
    async fn a_late_ack_applies_after_the_wait_reported() {
        let (mut worker, events, _inbox, _wa) = worker();
        worker.ensure_chat(PEER, None);
        worker.archive.set_ephemeral(PEER, 86_400, 100).unwrap();
        let session = worker.account_session;
        worker
            .handle_command(Command::TimerSet {
                chat: PEER.into(),
                session,
                seconds: 604_800,
                result: Ok(("sent-late".into(), 100)),
            })
            .await;
        if let Some(AwaitingAck::Timer(pending)) = worker.awaiting_ack.get_mut("sent-late") {
            pending.deadline = Instant::now() - Duration::from_secs(1);
        }
        worker.expire_timer_acks();
        assert!(
            events
                .try_iter()
                .any(|event| matches!(event, Event::Error(_))),
            "the wait reports once"
        );
        assert!(
            worker.awaiting_ack.contains_key("sent-late"),
            "the entry is kept for a late ack"
        );
        let ack = wa_events::ServerAck::builder()
            .id("sent-late".to_owned())
            .class("message".to_owned())
            .timestamp(whatsapp_rust::wacore::time::from_secs(150).unwrap())
            .build();
        worker
            .handle_wa_event(Arc::new(wa_events::Event::ServerAck(ack)))
            .await;
        assert_eq!(
            worker
                .archive
                .chat(PEER)
                .unwrap()
                .unwrap()
                .disappearing_timer(),
            Some(604_800),
            "the accepted change lands even after the report"
        );
        assert!(worker.awaiting_ack.is_empty());
    }

    /// A completion from a departed session is ignored rather than written to
    /// the account that replaced it.
    #[tokio::test]
    async fn a_timer_completion_from_an_old_session_is_ignored() {
        let (mut worker, _events, _inbox, _wa) = worker();
        worker.ensure_chat(PEER, None);
        worker.archive.set_ephemeral(PEER, 86_400, 100).unwrap();
        let old = worker.account_session;
        worker.account_session = old + 1;
        // Only the account session check can reject the completion; without
        // it the sent id would be registered under the new account, and its
        // ack would write the old account's change to the new one.
        worker
            .handle_command(Command::TimerSet {
                chat: PEER.into(),
                session: old,
                seconds: 604_800,
                result: Ok(("sent-old".into(), 100)),
            })
            .await;
        assert!(
            worker.awaiting_ack.is_empty(),
            "a stale completion must not register with the new account"
        );
        assert_eq!(
            worker
                .archive
                .chat(PEER)
                .unwrap()
                .unwrap()
                .disappearing_timer(),
            Some(86_400)
        );
    }

    /// A failed retry does not stay disabled behind an earlier attempt that
    /// already timed out and is kept only for a late ack.
    #[tokio::test]
    async fn a_failed_retry_is_not_held_saving_by_an_earlier_timeout() {
        let (mut worker, events, _inbox, _wa) = worker();
        worker.ensure_chat(PEER, None);
        worker.archive.set_ephemeral(PEER, 86_400, 100).unwrap();
        let session = worker.account_session;
        worker
            .handle_command(Command::TimerSet {
                chat: PEER.into(),
                session,
                seconds: 604_800,
                result: Ok(("a-1".into(), 100)),
            })
            .await;
        // A times out and is kept for a late ack; the control re-enables.
        if let Some(AwaitingAck::Timer(pending)) = worker.awaiting_ack.get_mut("a-1") {
            pending.deadline = Instant::now() - Duration::from_secs(1);
        }
        worker.expire_timer_acks();
        // The user retries, and the retry fails.
        worker
            .handle_command(Command::TimerSet {
                chat: PEER.into(),
                session,
                seconds: 0,
                result: Err("boom".into()),
            })
            .await;
        let last = events
            .try_iter()
            .filter_map(|event| match event {
                Event::TimerSaving { saving, .. } => Some(saving),
                _ => None,
            })
            .last();
        assert_eq!(
            last,
            Some(false),
            "the earlier, already-reported attempt must not hold the control disabled"
        );
    }

    /// An older change's late answer does not re-enable the control while the
    /// retry that replaced it still waits for its own.
    #[tokio::test]
    async fn an_older_ack_does_not_re_enable_a_newer_change() {
        for error in [None, Some("500")] {
            for send_completed in [false, true] {
                let (mut worker, events, _inbox, _wa) = worker();
                worker.ensure_chat(PEER, None);
                worker.archive.set_ephemeral(PEER, 86_400, 100).unwrap();
                let session = worker.account_session;
                let ack = |id: &str, at: i64| {
                    wa_events::ServerAck::builder()
                        .id(id.to_owned())
                        .class("message".to_owned())
                        .timestamp(whatsapp_rust::wacore::time::from_secs(at).unwrap())
                        .build()
                };
                worker
                    .handle_command(Command::TimerSet {
                        chat: PEER.into(),
                        session,
                        seconds: 604_800,
                        result: Ok(("a-1".into(), 100)),
                    })
                    .await;
                // A times out, so the control is enabled again for a retry.
                if let Some(AwaitingAck::Timer(pending)) = worker.awaiting_ack.get_mut("a-1") {
                    pending.deadline = Instant::now() - Duration::from_secs(1);
                }
                worker.expire_timer_acks();
                events.try_iter().for_each(drop);

                // Starting the retry disables the control before its send completes.
                worker.timer_saving(&PEER.to_owned(), true);
                let retry_sent = || Command::TimerSet {
                    chat: PEER.into(),
                    session,
                    seconds: 0,
                    result: Ok(("b-1".into(), 150)),
                };
                if send_completed {
                    worker.handle_command(retry_sent()).await;
                }
                // A's late ack or nack can land before or after the retry's send
                // completes, while the control must stay disabled in both cases.
                let mut late = ack("a-1", 160);
                late.error = error.map(str::to_owned);
                worker
                    .handle_wa_event(Arc::new(wa_events::Event::ServerAck(late)))
                    .await;
                let saving: Vec<bool> = events
                    .try_iter()
                    .filter_map(|event| match event {
                        Event::TimerSaving { saving, .. } => Some(saving),
                        _ => None,
                    })
                    .collect();
                assert_eq!(
                    saving,
                    [true],
                    "the retry keeps the control disabled (send_completed={send_completed}, error={error:?})"
                );
                if !send_completed {
                    worker.handle_command(retry_sent()).await;
                }

                // The retry's own ack is what enables it again.
                worker
                    .handle_wa_event(Arc::new(wa_events::Event::ServerAck(ack("b-1", 170))))
                    .await;
                let saving: Vec<bool> = events
                    .try_iter()
                    .filter_map(|event| match event {
                        Event::TimerSaving { saving, .. } => Some(saving),
                        _ => None,
                    })
                    .collect();
                assert_eq!(saving, [false], "{saving:?}");
            }
        }
    }

    /// Held acks age out on the timer tick even when no change is waiting.
    #[tokio::test]
    async fn held_acks_age_out_without_a_waiting_change() {
        let (mut worker, _events, _inbox, _wa) = worker();
        let ack = wa_events::ServerAck::builder()
            .id("held".to_owned())
            .class("message".to_owned())
            .build();
        worker.hold_early_ack(&ack);
        assert_eq!(worker.early_acks.len(), 1);
        if let Some(held) = worker.early_acks.get_mut("held") {
            held.at = Instant::now() - Duration::from_secs(90);
        }
        worker.expire_timer_acks();
        assert!(worker.early_acks.is_empty());
    }

    /// A privacy-id mapping learned while a change is in flight must not lose
    /// its accepted outcome: the destination is resolved when the ack lands.
    #[tokio::test]
    async fn an_ack_after_a_lid_merge_lands_on_the_phone_chat() {
        let (mut worker, events, _inbox, _wa) = worker();
        let lid = "123456789012345@lid";
        let phone = "4915112345678@s.whatsapp.net";
        worker.ensure_chat(lid, None);
        worker.archive.set_ephemeral(lid, 86_400, 100).unwrap();
        let session = worker.account_session;
        worker
            .handle_command(Command::TimerSet {
                chat: lid.into(),
                session,
                seconds: 604_800,
                result: Ok(("merged-1".into(), 150)),
            })
            .await;
        // The mapping is learned, merging the chat under its phone number,
        // before the ack arrives.
        worker.learn_pair(&lid.parse().unwrap(), &phone.parse().unwrap());
        let ack = wa_events::ServerAck::builder()
            .id("merged-1".to_owned())
            .class("message".to_owned())
            .build();
        worker
            .handle_wa_event(Arc::new(wa_events::Event::ServerAck(ack)))
            .await;
        assert_eq!(
            worker
                .archive
                .chat(phone)
                .unwrap()
                .map(|chat| chat.disappearing_timer()),
            Some(Some(604_800)),
            "the accepted change lands on the merged chat"
        );
        // The saving flag is cleared under the old privacy id too, so the
        // control is not left disabled after the merge.
        let cleared: Vec<String> = events
            .try_iter()
            .filter_map(|event| match event {
                Event::TimerSaving {
                    chat,
                    saving: false,
                } => Some(chat),
                _ => None,
            })
            .collect();
        assert!(cleared.contains(&lid.to_owned()), "{cleared:?}");
        assert!(cleared.contains(&phone.to_owned()), "{cleared:?}");
    }

    /// A change and the ack it still waits for belong to the account that sent
    /// them, so logging out forgets both. The interface drops the control's
    /// saving state with the rest of that account's state.
    #[tokio::test]
    async fn logout_forgets_a_waiting_timer_change() {
        let (mut worker, _events, _inbox, _wa) = worker();
        let lid = "123456789012345@lid";
        let phone = "4915112345678@s.whatsapp.net";
        worker.ensure_chat(lid, None);
        let session = worker.account_session;
        worker
            .handle_command(Command::TimerSet {
                chat: lid.into(),
                session,
                seconds: 604_800,
                result: Ok(("waiting-1".into(), 100)),
            })
            .await;
        assert_eq!(worker.awaiting_ack.len(), 1);
        worker.learn_pair(&lid.parse().unwrap(), &phone.parse().unwrap());
        worker.hold_early_ack(
            &wa_events::ServerAck::builder()
                .id("held".to_owned())
                .class("message".to_owned())
                .build(),
        );
        worker.on_logged_out().await;
        assert!(worker.awaiting_ack.is_empty());
        assert!(worker.early_acks.is_empty());
    }

    /// A timer change a stopped connection still reports must not be filed
    /// into the account that replaced it.
    #[tokio::test]
    async fn a_timer_change_from_a_stopped_session_is_dropped() {
        use whatsapp_rust::wacore::stanza::groups::GroupNotificationAction;
        let (mut worker, _events, _inbox, _wa) = worker();
        let group = "123-456@g.us";
        let admin = "111@s.whatsapp.net";
        let notice_id = format!("{}{}-{}", crate::model::TIMER_NOTICE_PREFIX, "notice-1", 0);
        let update = wa_events::GroupUpdate::builder()
            .group_jid(group.parse().unwrap())
            .notification_id("notice-1".to_owned())
            .participant(admin.parse().unwrap())
            .timestamp(whatsapp_rust::wacore::time::from_secs(200).unwrap())
            .is_lid_addressing_mode(false)
            .action(Box::new(GroupNotificationAction::Ephemeral {
                expiration: 86_400,
                trigger: None,
            }))
            .build();
        let event = Arc::new(wa_events::Event::GroupUpdate(update));
        let stopped = worker.account_session;
        // The account unlinks before the queued callback is taken.
        worker.account_session = stopped.wrapping_add(1);

        worker.whatsapp_event(stopped, event.clone()).await;

        assert!(
            worker.archive.chat(group).unwrap().is_none(),
            "a stopped connection's group must not be filed"
        );
        assert!(
            worker.archive.message(group, &notice_id).unwrap().is_none(),
            "a stopped connection's timer notice must not be filed"
        );
        // The session that is still this account's is taken as usual.
        worker.whatsapp_event(worker.account_session, event).await;
        assert!(
            worker.archive.message(group, &notice_id).unwrap().is_some(),
            "the current session's change still lands"
        );
    }

    /// A connection restart discards its acks, but must not strand the timer
    /// control, including after a privacy-id merge.
    #[tokio::test]
    async fn stopping_the_connection_clears_timer_saving() {
        let (mut worker, events, _inbox, _wa) = worker();
        let lid = "123456789012345@lid";
        worker.ensure_chat(lid, None);
        worker.timer_saving(&lid.to_owned(), true);
        worker.timer_set(
            lid.into(),
            worker.account_session,
            604_800,
            Ok(("waiting-1".into(), 100)),
        );
        worker.learn_pair(&lid.parse().unwrap(), &PEER.parse().unwrap());
        worker.awaiting_ack.insert(
            "ordinary-message".into(),
            AwaitingAck::Message("15550002222@s.whatsapp.net".into()),
        );
        events.try_iter().for_each(drop);

        worker.stop_bot().await;

        let saving: Vec<_> = events
            .try_iter()
            .filter_map(|event| match event {
                Event::TimerSaving { chat, saving } => Some((chat, saving)),
                _ => None,
            })
            .collect();
        assert_eq!(saving, [(lid.to_owned(), false), (PEER.to_owned(), false)]);
        assert!(worker.awaiting_ack.is_empty());
        worker.stop_bot().await;
        assert!(
            events.try_iter().next().is_none(),
            "stopping twice is harmless"
        );
    }

    /// A member of a group that lets only admins edit its info cannot change
    /// the timer: nothing is sent, and the refusal says what it was about. A
    /// group that lets members edit passes the check and tries to send.
    #[tokio::test]
    async fn a_locked_group_takes_no_timer_change_from_a_member() {
        let (mut worker, events, mut inbox, _wa) = worker();
        let locked = "120363000000000001@g.us";
        let open = "120363000000000002@g.us";
        for group in [locked, open] {
            worker.archive.ensure_chat(group, "Weekend plans").unwrap();
        }
        worker
            .archive
            .set_group_rights(locked, true, false)
            .unwrap();
        worker.archive.set_group_rights(open, false, false).unwrap();

        worker.set_disappearing_timer(locked.into(), 604_800);
        let reported: Vec<_> = events
            .try_iter()
            .filter_map(|event| match event {
                Event::Error(message) => Some(message),
                Event::TimerSaving { saving, .. } => Some(format!("saving {saving}")),
                _ => None,
            })
            .collect();
        assert_eq!(
            reported,
            ["Only admins can change disappearing messages.".to_owned()],
            "a locked group takes nothing"
        );
        assert!(inbox.try_recv().is_err(), "nothing was sent");
        assert!(worker.awaiting_ack.is_empty());

        worker.set_disappearing_timer(open.into(), 604_800);
        let reported: Vec<_> = events
            .try_iter()
            .filter_map(|event| match event {
                Event::Error(message) => Some(message),
                _ => None,
            })
            .collect();
        assert_eq!(
            reported,
            ["Connect to WhatsApp to change disappearing messages.".to_owned()],
            "an open group gets past the rights check"
        );
    }

    /// A group notification that names the actor only by an unmapped privacy
    /// id is still attributed to the contact, using the phone number it
    /// carries.
    #[tokio::test]
    async fn a_group_timer_notice_uses_the_phone_identity_it_carries() {
        use whatsapp_rust::wacore::stanza::groups::GroupNotificationAction;
        let (mut worker, _events, _inbox, _wa) = worker();
        let group = "123-456@g.us";
        let lid = "123456789012345@lid";
        let phone = "4915112345678@s.whatsapp.net";
        let update = wa_events::GroupUpdate::builder()
            .group_jid(group.parse().unwrap())
            .maybe_notification_id(Some("act-1".to_owned()))
            .participant(lid.parse().unwrap())
            .participant_pn(phone.parse().unwrap())
            .timestamp(whatsapp_rust::wacore::time::from_secs(200).unwrap())
            .is_lid_addressing_mode(false)
            .action(Box::new(GroupNotificationAction::Ephemeral {
                expiration: 86_400,
                trigger: None,
            }))
            .build();
        worker
            .handle_wa_event(Arc::new(wa_events::Event::GroupUpdate(update)))
            .await;
        let filed = worker
            .archive
            .messages(group, None, 100)
            .unwrap()
            .into_iter()
            .find(|message| matches!(message.content, Content::DisappearingTimer { .. }))
            .expect("a notice");
        assert_eq!(filed.sender, phone);
    }

    /// An accepted direct timer change files one centred system row; a stale
    /// change files none, and a redelivery replaces rather than duplicates it.
    #[test]
    fn a_protocol_timer_change_files_one_notice() {
        let (mut worker, _events, _inbox, _wa) = worker();
        worker.ensure_chat(PEER, None);
        worker.archive.set_ephemeral(PEER, 604_800, 100).unwrap();
        let setting = |expiration: u32, setting_time: Option<i64>, envelope: i64, id: &str| {
            let mut raw = wa::Message::default();
            raw.protocol_message = MessageField::some({
                let mut proto = wa::message::ProtocolMessage::default();
                proto.r#type = Some(wa::message::protocol_message::Type::EPHEMERAL_SETTING);
                proto.ephemeral_expiration = Some(expiration);
                proto.ephemeral_setting_timestamp = setting_time;
                proto
            });
            let info = MessageInfo {
                id: id.into(),
                source: MessageSource {
                    chat: PEER.parse().unwrap(),
                    sender: PEER.parse().unwrap(),
                    ..Default::default()
                },
                timestamp: whatsapp_rust::wacore::time::from_secs(envelope).unwrap(),
                ..Default::default()
            };
            (Arc::new(raw), info)
        };
        let notice = |id: &str| format!("{}{}", crate::model::TIMER_NOTICE_PREFIX, id);

        let (raw, info) = setting(86_400, Some(200), 200, "change-1");
        worker.ingest(&raw, &info);
        let filed = worker
            .archive
            .message(PEER, &notice("change-1"))
            .unwrap()
            .expect("an accepted change leaves a notice");
        assert_eq!(
            filed.content,
            Content::DisappearingTimer { seconds: 86_400 }
        );

        // A stale change is refused by the archive, so it leaves no notice.
        let (raw, info) = setting(7_776_000, Some(150), 150, "stale");
        worker.ingest(&raw, &info);
        assert!(
            worker
                .archive
                .message(PEER, &notice("stale"))
                .unwrap()
                .is_none(),
            "a stale change leaves no notice"
        );

        // The same change redelivered replaces its row rather than adding one.
        let (raw, info) = setting(86_400, Some(200), 200, "change-1");
        worker.ingest(&raw, &info);
        let notices = worker
            .archive
            .messages(PEER, None, 100)
            .unwrap()
            .iter()
            .filter(|message| matches!(message.content, Content::DisappearingTimer { .. }))
            .count();
        assert_eq!(notices, 1, "one change, one notice");

        // A change the reader cleared away stays gone when it is delivered
        // again: the removal barrier outlives the notice's own id.
        worker
            .archive
            .remove_chat_through(PEER, 300, false)
            .unwrap();
        worker.ingest(&raw, &info);
        assert!(
            worker
                .archive
                .message(PEER, &notice("change-1"))
                .unwrap()
                .is_none(),
            "a cleared notice stays gone"
        );
    }

    /// A group timer change is written from the server's own group
    /// notification, not from the accepted IQ: the notification's timestamp
    /// orders it, and a newer phone change is never overwritten. A failure
    /// keeps the old value and only reports.
    #[tokio::test]
    async fn a_group_timer_change_lands_only_when_whatsapp_accepts_it() {
        use whatsapp_rust::wacore::stanza::groups::GroupNotificationAction;
        let (mut worker, events, _inbox, _wa) = worker();
        let group = "123-456@g.us";
        worker.archive.ensure_chat(group, "Weekend plans").unwrap();
        worker.archive.set_ephemeral(group, 86_400, 100).unwrap();

        // The accepted IQ alone writes nothing: the server's notification does.
        worker
            .handle_command(Command::GroupEdited {
                chat: group.into(),
                edit: GroupEdit::Timer(604_800),
                session: 0,
                result: Ok(()),
            })
            .await;
        assert_eq!(
            worker
                .archive
                .chat(group)
                .unwrap()
                .unwrap()
                .disappearing_timer(),
            Some(86_400),
            "the IQ alone does not change the timer"
        );

        let update = wa_events::GroupUpdate::builder()
            .group_jid(group.parse().unwrap())
            .timestamp(whatsapp_rust::wacore::time::from_secs(150).unwrap())
            .is_lid_addressing_mode(false)
            .action(Box::new(GroupNotificationAction::Ephemeral {
                expiration: 604_800,
                trigger: None,
            }))
            .build();
        worker
            .handle_wa_event(Arc::new(wa_events::Event::GroupUpdate(update)))
            .await;
        assert_eq!(
            worker
                .archive
                .chat(group)
                .unwrap()
                .unwrap()
                .disappearing_timer(),
            Some(604_800),
            "the server's notification applies the change"
        );

        worker
            .handle_command(Command::GroupEdited {
                chat: group.into(),
                edit: GroupEdit::Timer(0),
                session: 0,
                result: Err("boom".into()),
            })
            .await;
        assert_eq!(
            worker
                .archive
                .chat(group)
                .unwrap()
                .unwrap()
                .disappearing_timer(),
            Some(604_800),
            "a failure changes nothing"
        );
        let errors: Vec<_> = events
            .try_iter()
            .filter_map(|event| match event {
                Event::Error(message) => Some(message),
                _ => None,
            })
            .collect();
        assert!(
            errors.contains(&"Could not change disappearing messages.".to_owned()),
            "{errors:?}"
        );
    }

    /// A timer change that predates a chat's removal leaves no notice and does
    /// not bring the chat back when the same change is delivered again.
    #[tokio::test]
    async fn a_removed_chat_keeps_a_redelivered_timer_change_out() {
        use whatsapp_rust::wacore::stanza::groups::GroupNotificationAction;
        let (mut worker, _events, _inbox, _wa) = worker();
        let chat = "123-456@g.us";
        worker.archive.ensure_chat(chat, "Weekend plans").unwrap();
        // The chat was cleared through 300: anything at or before then is gone.
        worker
            .archive
            .remove_chat_through(chat, 300, false)
            .unwrap();
        let update = wa_events::GroupUpdate::builder()
            .group_jid(chat.parse().unwrap())
            .notification_id("notice-1".to_owned())
            .timestamp(whatsapp_rust::wacore::time::from_secs(200).unwrap())
            .is_lid_addressing_mode(false)
            .action(Box::new(GroupNotificationAction::Ephemeral {
                expiration: 86_400,
                trigger: None,
            }))
            .build();
        worker
            .handle_wa_event(Arc::new(wa_events::Event::GroupUpdate(update)))
            .await;
        let notices = worker
            .archive
            .messages(chat, None, 100)
            .unwrap()
            .iter()
            .filter(|message| matches!(message.content, Content::DisappearingTimer { .. }))
            .count();
        assert_eq!(notices, 0, "a removed change leaves no notice behind");
    }

    /// A timer setting newer than the stored value still applies when it
    /// predates a chat clear: message removal does not order timer metadata.
    /// Only its notice is kept out.
    #[tokio::test]
    async fn a_newer_timer_survives_a_clear_that_postdates_it() {
        use whatsapp_rust::wacore::stanza::groups::GroupNotificationAction;
        let (mut worker, _events, _inbox, _wa) = worker();
        let group = "123-456@g.us";
        worker.archive.ensure_chat(group, "Weekend plans").unwrap();
        worker.archive.set_ephemeral(group, 604_800, 100).unwrap();
        // Cleared through 200, then the phone's Off at 150 finally arrives.
        worker
            .archive
            .remove_chat_through(group, 200, false)
            .unwrap();
        let update = wa_events::GroupUpdate::builder()
            .group_jid(group.parse().unwrap())
            .timestamp(whatsapp_rust::wacore::time::from_secs(150).unwrap())
            .is_lid_addressing_mode(false)
            .action(Box::new(GroupNotificationAction::Ephemeral {
                expiration: 0,
                trigger: None,
            }))
            .build();
        worker
            .handle_wa_event(Arc::new(wa_events::Event::GroupUpdate(update)))
            .await;
        assert_eq!(
            worker
                .archive
                .chat(group)
                .unwrap()
                .unwrap()
                .disappearing_timer(),
            None,
            "the newer setting is not discarded as a removed message"
        );
        let notices = worker
            .archive
            .messages(group, None, 100)
            .unwrap()
            .iter()
            .filter(|message| matches!(message.content, Content::DisappearingTimer { .. }))
            .count();
        assert_eq!(notices, 0, "its notice stays out of the cleared range");
    }

    /// A chat removed outright is not brought back by a timer setting from
    /// before its removal.
    #[tokio::test]
    async fn a_deleted_chat_is_not_brought_back_by_an_older_timer_change() {
        use whatsapp_rust::wacore::stanza::groups::GroupNotificationAction;
        let (mut worker, _events, _inbox, _wa) = worker();
        let group = "123-456@g.us";
        worker.archive.ensure_chat(group, "Weekend plans").unwrap();
        worker
            .archive
            .remove_chat_through(group, 300, true)
            .unwrap();
        let update = wa_events::GroupUpdate::builder()
            .group_jid(group.parse().unwrap())
            .timestamp(whatsapp_rust::wacore::time::from_secs(200).unwrap())
            .is_lid_addressing_mode(false)
            .action(Box::new(GroupNotificationAction::Ephemeral {
                expiration: 86_400,
                trigger: None,
            }))
            .build();
        worker
            .handle_wa_event(Arc::new(wa_events::Event::GroupUpdate(update)))
            .await;
        assert!(
            worker.archive.chat(group).unwrap().is_none(),
            "the deleted chat stays gone"
        );
    }

    /// An accepted group timer change files one centred notice naming the
    /// admin who made it, and a redelivery replaces it rather than adding one.
    #[tokio::test]
    async fn a_group_timer_change_files_one_notice() {
        use whatsapp_rust::wacore::stanza::groups::GroupNotificationAction;
        let (mut worker, _events, _inbox, _wa) = worker();
        let group = "123-456@g.us";
        let admin = "111@s.whatsapp.net";
        let notice_id = format!("{}{}-{}", crate::model::TIMER_NOTICE_PREFIX, "notice-1", 0);
        let update = |expiration: u32| {
            wa_events::GroupUpdate::builder()
                .group_jid(group.parse().unwrap())
                .notification_id("notice-1".to_owned())
                .participant(admin.parse().unwrap())
                .timestamp(whatsapp_rust::wacore::time::from_secs(200).unwrap())
                .is_lid_addressing_mode(false)
                .action(Box::new(GroupNotificationAction::Ephemeral {
                    expiration,
                    trigger: None,
                }))
                .build()
        };

        worker
            .handle_wa_event(Arc::new(wa_events::Event::GroupUpdate(update(86_400))))
            .await;
        let filed = worker
            .archive
            .message(group, &notice_id)
            .unwrap()
            .expect("an accepted change leaves a notice");
        assert_eq!(
            filed.content,
            Content::DisappearingTimer { seconds: 86_400 }
        );
        assert_eq!(filed.sender, admin);
        assert!(!filed.from_me);

        // The same change redelivered replaces its row rather than adding one.
        worker
            .handle_wa_event(Arc::new(wa_events::Event::GroupUpdate(update(86_400))))
            .await;
        let notices = worker
            .archive
            .messages(group, None, 100)
            .unwrap()
            .iter()
            .filter(|message| matches!(message.content, Content::DisappearingTimer { .. }))
            .count();
        assert_eq!(notices, 1, "one change, one notice");
    }

    /// A group timer notice survives a notification that names no actor and
    /// one that carries no id: the actor is left unknown rather than blamed on
    /// us, and two id-less changes at different times do not share a row.
    #[tokio::test]
    async fn group_timer_notices_handle_a_missing_actor_and_missing_id() {
        use whatsapp_rust::wacore::stanza::groups::GroupNotificationAction;
        let (mut worker, _events, _inbox, _wa) = worker();
        let group = "123-456@g.us";
        let update = |at: i64, expiration: u32, id: Option<&str>| {
            wa_events::GroupUpdate::builder()
                .group_jid(group.parse().unwrap())
                .maybe_notification_id(id.map(str::to_owned))
                .timestamp(whatsapp_rust::wacore::time::from_secs(at).unwrap())
                .is_lid_addressing_mode(false)
                .action(Box::new(GroupNotificationAction::Ephemeral {
                    expiration,
                    trigger: None,
                }))
                .build()
        };

        // No participant at all: the notice names nobody, not "You".
        worker
            .handle_wa_event(Arc::new(wa_events::Event::GroupUpdate(update(
                200, 86_400, None,
            ))))
            .await;
        let filed = worker
            .archive
            .messages(group, None, 100)
            .unwrap()
            .into_iter()
            .find(|message| matches!(message.content, Content::DisappearingTimer { .. }))
            .expect("an accepted change leaves a notice");
        assert!(!filed.from_me, "an absent actor is not us");
        assert_ne!(filed.sender, ME);

        // A second id-less change in the same second, with a different
        // duration, is a second notice rather than an overwrite.
        worker
            .handle_wa_event(Arc::new(wa_events::Event::GroupUpdate(update(
                200, 0, None,
            ))))
            .await;
        let notices = worker
            .archive
            .messages(group, None, 100)
            .unwrap()
            .iter()
            .filter(|message| matches!(message.content, Content::DisappearingTimer { .. }))
            .count();
        assert_eq!(notices, 2, "id-less changes do not overwrite each other");
    }
}
