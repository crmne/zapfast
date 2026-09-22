//! Poll transport and decryption belong to whatsapp-rust; this module archives results.

use super::*;
use crate::archive::PollVote;
use crate::model::{PollDraft, PollState};
use whatsapp_rust::features::PollVoteCiphertext;

pub(super) fn definition(message: &wa::Message) -> Option<&wa::message::PollCreationMessage> {
    message
        .poll_creation_message
        .as_option()
        .or(message.poll_creation_message_v2.as_option())
        .or(message.poll_creation_message_v3.as_option())
}

fn secret(message: &wa::Message) -> Option<&[u8]> {
    let base = message.get_base_message();
    message
        .message_context_info
        .as_option()
        .or(base.message_context_info.as_option())
        .and_then(|context| context.message_secret.as_deref())
        .or_else(|| definition(base)?.enc_key.as_deref())
        .filter(|secret| secret.len() == 32)
}

pub(super) fn choices_for(options: &[String], hashes: &[Vec<u8>]) -> Option<Vec<usize>> {
    let mut choices = Vec::new();
    for hash in hashes {
        let index = options.iter().position(|option| {
            whatsapp_rust::wacore::poll::compute_option_hash(option).as_slice() == hash
        })?;
        if !choices.contains(&index) {
            choices.push(index);
        }
    }
    choices.sort_unstable();
    Some(choices)
}

/// Preserve the library-created poll for later reply quotes and lazy archive upgrades.
/// This body is never sent; Polls::create owns protocol construction and transmission.
fn creation_archive(content: &Content, secret: &[u8]) -> wa::Message {
    let Content::Poll {
        question,
        options,
        state,
    } = content
    else {
        unreachable!()
    };
    let poll = wa::message::PollCreationMessage {
        name: Some(question.clone()),
        options: options
            .iter()
            .map(|name| wa::message::poll_creation_message::Option {
                option_name: Some(name.clone()),
                ..Default::default()
            })
            .collect(),
        selectable_options_count: Some(state.selectable as u32),
        poll_content_type: Some(wa::message::PollContentType::TEXT),
        ..Default::default()
    };
    let mut message = wa::Message {
        message_context_info: MessageField::some(wa::MessageContextInfo {
            message_secret: Some(secret.into()),
            ..Default::default()
        }),
        ..Default::default()
    };
    if state.selectable == 1 {
        message.poll_creation_message_v3 = MessageField::some(poll);
    } else {
        message.poll_creation_message = MessageField::some(poll);
    }
    message
}

impl Worker {
    pub(super) fn refresh_poll(&mut self, chat: ChatId, id: String) {
        self.poll_history.request(&chat, &id, Instant::now());
        self.emit_message(&chat, &id);
        self.pump_poll_history();
    }

    pub(super) fn pump_poll_history(&mut self) {
        let now = Instant::now();
        if let Some((chat, id)) = self.poll_history.expire(now) {
            log::info!("poll recovery: phone history timed out; retry scheduled");
            self.emit_message(&chat, &id);
        }
        if !self.status.is_connected() || !self.pending_older.is_empty() {
            return;
        }
        let Some(client) = self.client.clone() else {
            return;
        };
        let Some((chat, id)) = self.poll_history.next(now) else {
            return;
        };
        let prepared = (|| {
            let jid = Self::jid_of(&chat)?;
            let row = self.archive.message(&chat, &id).ok()??;
            if !matches!(row.content, Content::Poll { .. }) {
                return None;
            }
            let anchor = self.archive.poll_history_anchor(&row).ok()?;
            Some((jid, anchor))
        })();
        let Some((jid, (anchor, from_me, timestamp))) = prepared else {
            self.poll_history.finish(&chat, &id);
            self.emit_message(&chat, &id);
            return;
        };
        log::info!(
            "poll recovery: requesting phone history; anchor_present={}",
            !anchor.is_empty()
        );
        let commands = self.commands.clone();
        tokio::spawn(async move {
            if client
                .fetch_message_history(
                    &jid,
                    &anchor,
                    from_me,
                    // The misleadingly named oldestMsgTimestampMs wire field
                    // takes Unix seconds, as does the archive.
                    // https://github.com/tulir/whatsmeow/commit/54650307d891f89ab346a57953d316106caee371
                    timestamp,
                    PHONE_BATCH,
                )
                .await
                .is_err()
            {
                log::info!("poll recovery: phone history request failed");
                let _ = commands.send(Command::PollHistoryFailed {
                    chat,
                    message: id,
                    requested: now,
                });
            } else {
                log::info!("poll recovery: phone history request sent");
            }
        });
    }

    pub(super) fn history_poll_votes(&self, row: &Message, updates: &[wa::PollUpdate]) -> bool {
        let Content::Poll { options, state, .. } = &row.content else {
            return false;
        };
        let mut accepted = 0;
        for update in updates {
            let Some(key) = update.poll_update_message_key.as_option() else {
                continue;
            };
            let Some(value) = update.vote.as_option() else {
                continue;
            };
            let Some(choices) = choices_for(options, &value.selected_options) else {
                continue;
            };
            if state.selectable > 0 && choices.len() > state.selectable {
                continue;
            }
            let from_me = key.from_me.unwrap_or(false);
            let sender = if from_me {
                self.me()
            } else {
                let Some(sender) = key
                    .participant
                    .as_deref()
                    .filter(|sender| !sender.is_empty())
                    .or_else(|| {
                        (ChatKind::from_id(&row.chat) == ChatKind::Direct)
                            .then_some(row.chat.as_str())
                    })
                else {
                    continue;
                };
                sender.into()
            };
            let Some(update_id) = key.id.clone() else {
                continue;
            };
            let vote = PollVote {
                chat: row.chat.clone(),
                poll: row.id.clone(),
                voter: self.canonical_str(&sender),
                sender,
                update_id,
                at: update
                    .sender_timestamp_ms
                    .or(update.server_timestamp_ms)
                    .unwrap_or_default(),
                from_me,
                choices: Some(choices),
                encrypted: None,
            };
            if let Err(error) = self.archive.save_poll_vote(&vote) {
                log::warn!("could not store historical poll votes: {error}");
            } else {
                accepted += 1;
            }
        }
        log::info!(
            "poll recovery: phone snapshot contained {} votes; {accepted} usable",
            updates.len()
        );
        // Receiving the question again does not prove the phone sent its votes.
        // Empty or unusable snapshots must not end automatic recovery.
        !updates.is_empty() && accepted == updates.len()
    }

    pub(super) fn remember_poll(
        &self,
        row: &Message,
        message: &wa::Message,
        creator: &str,
        history_secret: Option<&[u8]>,
    ) {
        if !matches!(row.content, Content::Poll { .. }) {
            return;
        }
        if let Some(secret) = history_secret
            .filter(|secret| secret.len() == 32)
            .or_else(|| secret(message))
            && let Err(error) = self.archive.save_poll(&row.chat, &row.id, creator, secret)
        {
            log::warn!("could not save a poll definition: {error}");
        }
    }

    pub(super) fn polish_poll(&self, row: &mut Message) {
        if !matches!(row.content, Content::Poll { .. }) {
            return;
        }
        // Older archives already retain the creation protobuf. Upgrade lazily,
        // without resetting the session or scanning all message bodies at startup.
        let raw = self
            .archive
            .raw(&row.chat, &row.id)
            .ok()
            .flatten()
            .and_then(|raw| wa::Message::decode_from_slice(&raw).ok());
        if let Some(raw) = &raw {
            self.remember_poll(row, raw, &row.sender, None);
        }
        let Content::Poll { options, state, .. } = &mut row.content else {
            return;
        };
        if let Some(poll) = raw
            .as_ref()
            .and_then(|raw| definition(raw.get_base_message()))
        {
            state.selectable = poll.selectable_options_count.unwrap_or(0) as usize;
        }
        state.selectable = if state.selectable == 0 {
            options.len()
        } else {
            state.selectable.min(options.len())
        };
        state.can_vote = self
            .archive
            .poll_key(&row.chat, &row.id)
            .ok()
            .flatten()
            .is_some();
        state.history_complete = self
            .archive
            .has_poll_history(&row.chat, &row.id)
            .unwrap_or(false);
        let (pending, tried, waiting) = self.poll_history.state(&row.chat, &row.id);
        state.refreshing = pending;
        state.refresh_failed = waiting;
        state.refresh_needed = !tried;
        state.counts = vec![0; options.len()];
        state.selected.clear();
        state.voters = 0;
        let mut latest = HashMap::new();
        for vote in self
            .archive
            .poll_votes(&row.chat, &row.id)
            .unwrap_or_default()
        {
            let voter = if vote.from_me {
                self.me()
            } else {
                self.canonical_str(&vote.voter)
            };
            latest.insert(voter, vote);
        }
        for vote in latest.into_values() {
            let Some(choices) = vote.choices else {
                continue;
            };
            if vote.from_me || self.is_me(&vote.voter) {
                state.selected = choices.clone();
            }
            if !choices.is_empty() {
                state.voters += 1;
            }
            for index in choices {
                if let Some(count) = state.counts.get_mut(index) {
                    *count += 1;
                }
            }
        }
    }

    pub(super) fn create_poll(&mut self, chat: ChatId, draft: PollDraft) {
        let error = if !self.status.is_connected() {
            Some("Not connected to WhatsApp")
        } else if ChatKind::from_id(&chat) == ChatKind::Broadcast
            || self
                .archive
                .chat(&chat)
                .ok()
                .flatten()
                .is_some_and(|chat| chat.read_only)
        {
            Some("Polls cannot be sent to this chat.")
        } else if self.ephemeral_expiration(&chat).is_some() {
            Some("Poll creation in disappearing-message chats is not supported yet.")
        } else {
            None
        };
        let draft = draft.validated();
        if let Some(error) = error.or_else(|| draft.as_ref().err().copied()) {
            self.emit(Event::PollCreated {
                chat,
                error: Some(error.into()),
            });
            return;
        }
        let (Some(client), Some(jid)) = (self.client.clone(), Self::jid_of(&chat)) else {
            self.emit(Event::PollCreated {
                chat,
                error: Some("Not connected to WhatsApp".into()),
            });
            return;
        };
        let draft = draft.unwrap();
        let commands = self.commands.clone();
        tokio::spawn(async move {
            let result = async {
                let recipients = if jid.is_group() {
                    client
                        .groups()
                        .query_info(&jid)
                        .await
                        .map_err(|_| "Could not load the group recipients")?
                        .participants
                        .iter()
                        .map(Jid::to_non_ad_string)
                        .collect()
                } else {
                    Vec::new()
                };
                let creator = client
                    .pn()
                    .ok_or("Not connected to WhatsApp")?
                    .to_non_ad_string();
                let (sent, secret) = client
                    .polls()
                    .create(
                        jid,
                        &draft.question,
                        &draft.options,
                        draft.selectable() as u32,
                    )
                    .await
                    .map_err(|_| "Could not send the poll. Please try again.")?;
                Ok(super::super::CreatedPoll {
                    id: sent.message_id,
                    secret,
                    creator,
                    recipients,
                })
            }
            .await
            .map_err(str::to_owned);
            let _ = commands.send(Command::PollCreated {
                chat,
                draft,
                result,
            });
        });
    }

    pub(super) fn poll_created(
        &mut self,
        chat: ChatId,
        draft: PollDraft,
        result: Result<super::super::CreatedPoll, String>,
    ) {
        let created = match result {
            Ok(created) => created,
            Err(error) => {
                self.emit(Event::PollCreated {
                    chat,
                    error: Some(error),
                });
                return;
            }
        };
        if !self.is_me(&created.creator) {
            self.emit(Event::PollCreated {
                chat,
                error: Some("The account changed while the poll was being sent.".into()),
            });
            return;
        }
        let selectable = draft.selectable();
        let row = Message {
            id: created.id.clone(),
            chat: chat.clone(),
            sender: self.me(),
            sender_name: None,
            from_me: true,
            timestamp: crate::util::now(),
            content: Content::Poll {
                question: draft.question,
                options: draft.options,
                state: PollState {
                    selectable,
                    ..Default::default()
                },
            },
            status: Delivery::Sent,
            delivered_at: None,
            read_at: None,
            quoted: None,
            reactions: Vec::new(),
            edited: false,
            mentions: Vec::new(),
            forwarded: false,
            thumbnail: None,
        };
        if let Err(error) =
            self.archive
                .save_poll(&chat, &row.id, &created.creator, &created.secret)
        {
            log::warn!("could not archive a sent poll definition: {error}");
            self.emit(Event::PollCreated {
                chat,
                error: Some("The poll was sent, but its voting key could not be saved.".into()),
            });
            return;
        }
        let raw = creation_archive(&row.content, &created.secret).encode_to_vec();
        let _ = self.archive.mark_poll_history(&chat, &row.id);
        self.poll_history.finish(&chat, &row.id);
        self.store_message(row, Some(raw), None);
        if ChatKind::from_id(&chat) == ChatKind::Group {
            self.save_group_recipients(&chat, &created.id, &created.recipients);
        }
        self.emit(Event::PollCreated { chat, error: None });
        self.pump_poll_votes();
    }

    pub(super) fn vote_poll(&mut self, chat: ChatId, id: String, mut choices: Vec<usize>) {
        let request = (chat.clone(), id.clone());
        if self.poll_sending.contains(&request) {
            return;
        }
        let prepared = (|| {
            if !self.status.is_connected() {
                return None;
            }
            let client = self.client.clone()?;
            let jid = Self::jid_of(&chat)?;
            let mut row = self.archive.message(&chat, &id).ok()??;
            self.polish_poll(&mut row);
            let Content::Poll { options, state, .. } = row.content else {
                return None;
            };
            choices.sort_unstable();
            choices.dedup();
            if choices.len() > state.selectable
                || choices.iter().any(|&index| index >= options.len())
            {
                return None;
            }
            let (creator, secret) = self.archive.poll_key(&chat, &id).ok()??;
            let creator = Self::jid_of(&creator)?;
            let names: Vec<_> = choices
                .iter()
                .map(|&index| options[index].clone())
                .collect();
            Some((client, jid, creator, secret, names))
        })();
        let Some((client, jid, creator, secret, names)) = prepared else {
            self.emit(Event::PollVoted {
                chat,
                message: id,
                error: Some("This poll is not ready for voting. Reconnect and try again.".into()),
            });
            return;
        };
        self.poll_sending.insert(request);
        let commands = self.commands.clone();
        tokio::spawn(async move {
            let at = jiff::Timestamp::now().as_millisecond();
            let result = client
                .polls()
                .vote(jid, &id, &creator, &secret, &names)
                .await
                .map(|sent| sent.message_id)
                .map_err(|_| "Could not send your vote. Please try again.".into());
            let _ = commands.send(Command::PollVoted {
                chat,
                message: id,
                choices,
                at,
                result,
            });
        });
    }

    pub(super) fn poll_voted(
        &mut self,
        chat: ChatId,
        id: String,
        choices: Vec<usize>,
        at: i64,
        result: Result<String, String>,
    ) {
        if !self.poll_sending.remove(&(chat.clone(), id.clone())) {
            return;
        }
        if !self
            .archive
            .message(&chat, &id)
            .ok()
            .flatten()
            .is_some_and(|message| matches!(message.content, Content::Poll { .. }))
        {
            self.emit(Event::PollVoted {
                chat,
                message: id,
                error: None,
            });
            return;
        }
        let error = match result {
            Ok(update_id) => {
                let vote = PollVote {
                    chat: chat.clone(),
                    poll: id.clone(),
                    voter: self.me(),
                    sender: self.me(),
                    update_id,
                    at,
                    from_me: true,
                    choices: Some(choices),
                    encrypted: None,
                };
                match self.archive.save_poll_vote(&vote) {
                    Ok(_) => {
                        self.emit_message(&chat, &id);
                        None
                    }
                    Err(_) => Some("Your vote was sent, but could not be saved locally.".into()),
                }
            }
            Err(error) => Some(error),
        };
        self.emit(Event::PollVoted {
            chat,
            message: id,
            error,
        });
    }

    pub(super) fn ingest_poll_vote(
        &mut self,
        chat: &str,
        update_id: &str,
        sender: &str,
        from_me: bool,
        timestamp: i64,
        update: &wa::message::PollUpdateMessage,
    ) {
        let Some(poll) = update
            .poll_creation_message_key
            .as_option()
            .and_then(|key| key.id.clone())
        else {
            return;
        };
        let vote = PollVote {
            chat: chat.into(),
            poll,
            voter: if from_me {
                self.me()
            } else {
                self.canonical_str(sender)
            },
            sender: sender.into(),
            update_id: update_id.into(),
            at: update
                .sender_timestamp_ms
                .unwrap_or(timestamp.saturating_mul(1000)),
            from_me,
            choices: None,
            encrypted: Some(update.encode_to_vec()),
        };
        if let Err(error) = self.archive.save_poll_vote(&vote) {
            log::warn!("could not store a poll vote: {error}");
        }
        self.pump_poll_votes();
    }

    pub(super) fn pump_poll_votes(&mut self) {
        let Some(client) = self.client.clone() else {
            return;
        };
        if self.poll_decrypting >= 8 {
            return;
        }
        for vote in self
            .archive
            .pending_poll_votes(8 - self.poll_decrypting)
            .unwrap_or_default()
        {
            if self.archive.attempt_poll_vote(&vote).is_err() {
                continue;
            }
            let prepared = (|| {
                let row = self.archive.message(&vote.chat, &vote.poll).ok()??;
                let Content::Poll { options, state, .. } = row.content else {
                    return None;
                };
                let (creator, secret) = self.archive.poll_key(&vote.chat, &vote.poll).ok()??;
                let creator = Self::jid_of(&creator)?;
                let voter = Self::jid_of(&vote.sender)?;
                let update =
                    wa::message::PollUpdateMessage::decode_from_slice(vote.encrypted.as_deref()?)
                        .ok()?;
                Some((options, state.selectable, creator, secret, voter, update))
            })();
            let Some((options, selectable, creator, secret, voter, update)) = prepared else {
                continue;
            };
            let client = client.clone();
            let commands = self.commands.clone();
            self.poll_decrypting += 1;
            tokio::spawn(async move {
                let choices = if let Some(value) = update.vote.as_option() {
                    let ciphertext = PollVoteCiphertext {
                        enc_payload: value.enc_payload.as_deref().unwrap_or_default(),
                        enc_iv: value.enc_iv.as_deref().unwrap_or_default(),
                    };
                    client
                        .polls()
                        .decrypt_vote(ciphertext, &secret, &vote.poll, &creator, &voter)
                        .await
                        .ok()
                        .and_then(|hashes| choices_for(&options, &hashes))
                        .filter(|choices| {
                            choices.len()
                                <= if selectable == 0 {
                                    options.len()
                                } else {
                                    selectable
                                }
                        })
                } else {
                    None
                };
                let _ = commands.send(Command::PollDecoded { vote, choices });
            });
        }
    }

    pub(super) fn poll_decoded(&mut self, vote: PollVote, choices: Option<Vec<usize>>) {
        self.poll_decrypting = self.poll_decrypting.saturating_sub(1);
        if let Some(choices) = choices {
            if self
                .archive
                .finish_poll_vote(&vote, &choices)
                .unwrap_or(false)
            {
                self.emit_message(&vote.chat, &vote.poll);
            }
        } else {
            log::debug!("a poll vote could not be decrypted");
        }
        self.pump_poll_votes();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn content() -> Content {
        Content::Poll {
            question: "Lunch? 🍕".into(),
            options: vec!["Pizza".into(), "Pasta".into()],
            state: PollState {
                selectable: 1,
                ..Default::default()
            },
        }
    }

    #[test]
    fn empty_or_unusable_phone_snapshots_do_not_stop_automatic_recovery() {
        let (mut worker, _events, _commands, _wa) = super::super::receipt_tests::worker();
        let chat = "123@g.us";
        let mut row = crate::archive::tests::message(chat, "poll", 1_789_551_600, false);
        row.sender = "100@s.whatsapp.net".into();
        row.content = content();
        let raw = creation_archive(&row.content, &[7; 32]);
        worker.store_message(row.clone(), Some(raw.encode_to_vec()), None);
        worker.refresh_poll(chat.into(), row.id.clone());
        let now = Instant::now();
        assert!(worker.poll_history.next(now).is_some());

        for updates in [
            Vec::new(),
            vec![wa::PollUpdate {
                poll_update_message_key: MessageField::some(wa::MessageKey {
                    id: Some("unmatched-vote".into()),
                    participant: Some("200@s.whatsapp.net".into()),
                    ..Default::default()
                }),
                vote: MessageField::some(wa::message::PollVoteMessage {
                    selected_options: vec![vec![0; 32]],
                }),
                ..Default::default()
            }],
        ] {
            worker.apply_history(
                ParsedHistory {
                    chats: vec![parse_conversation(wa::Conversation {
                        id: chat.into(),
                        messages: vec![wa::HistorySyncMsg {
                            message: MessageField::some(wa::WebMessageInfo {
                                key: MessageField::some(wa::MessageKey {
                                    id: Some(row.id.clone()),
                                    remote_jid: Some(chat.into()),
                                    participant: Some(row.sender.clone()),
                                    from_me: Some(false),
                                }),
                                message: MessageField::some(raw.clone()),
                                message_timestamp: Some(row.timestamp as u64),
                                poll_updates: updates,
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
                false,
            );
            worker.polish_poll(&mut row);
            let Content::Poll { state, .. } = &row.content else {
                panic!("poll")
            };
            assert!(!state.history_complete);
            assert!(state.refreshing);
        }
        assert!(
            worker
                .poll_history
                .expire(now + Duration::from_secs(30))
                .is_some()
        );
        assert!(
            worker
                .poll_history
                .next(now + Duration::from_secs(59))
                .is_none()
        );
        assert!(
            worker
                .poll_history
                .next(now + Duration::from_secs(60))
                .is_some()
        );
    }

    #[test]
    fn old_polls_recover_all_eleven_phone_votes_without_overwriting_a_newer_own_vote() {
        let (mut worker, _events, _commands, _wa) = super::super::receipt_tests::worker();
        let chat = "123@g.us";
        let mut row = crate::archive::tests::message(chat, "poll", 100, false);
        row.sender = "100@s.whatsapp.net".into();
        row.content = content();
        let raw = creation_archive(&row.content, &[7; 32]);
        worker.store_message(row.clone(), Some(raw.encode_to_vec()), None);
        worker
            .archive
            .save_poll_vote(&PollVote {
                chat: chat.into(),
                poll: "poll".into(),
                voter: worker.me(),
                sender: worker.me(),
                update_id: "own-new".into(),
                at: 2000,
                from_me: true,
                choices: Some(vec![1]),
                encrypted: None,
            })
            .unwrap();
        worker.polish_poll(&mut row);
        let Content::Poll { state, .. } = &row.content else {
            panic!("poll")
        };
        assert_eq!(state.counts, vec![0, 1]);
        assert!(!state.history_complete);
        assert!(state.refresh_needed);
        worker.refresh_poll(chat.into(), "poll".into());
        let updates = (0..11)
            .map(|index| wa::PollUpdate {
                poll_update_message_key: MessageField::some(wa::MessageKey {
                    id: Some(format!("history-{index}")),
                    from_me: Some(index == 10),
                    participant: Some(format!("{}@s.whatsapp.net", 200 + index)),
                    ..Default::default()
                }),
                vote: MessageField::some(wa::message::PollVoteMessage {
                    selected_options: vec![
                        whatsapp_rust::wacore::poll::compute_option_hash(if index < 8 {
                            "Pizza"
                        } else {
                            "Pasta"
                        })
                        .to_vec(),
                    ],
                }),
                sender_timestamp_ms: Some(1500),
                ..Default::default()
            })
            .collect();
        let parsed = parse_conversation(wa::Conversation {
            id: chat.into(),
            messages: vec![wa::HistorySyncMsg {
                message: MessageField::some(wa::WebMessageInfo {
                    key: MessageField::some(wa::MessageKey {
                        id: Some("poll".into()),
                        remote_jid: Some(chat.into()),
                        participant: Some(row.sender.clone()),
                        from_me: Some(false),
                    }),
                    message: MessageField::some(raw),
                    message_timestamp: Some(100),
                    poll_updates: updates,
                    ..Default::default()
                }),
                msg_order_id: None,
            }],
            ..Default::default()
        });
        worker.apply_history(
            ParsedHistory {
                chats: vec![parsed],
                push_names: Vec::new(),
                lids: Vec::new(),
                stickers: Vec::new(),
            },
            false,
        );
        worker.polish_poll(&mut row);
        let Content::Poll { state, .. } = row.content else {
            panic!("poll")
        };
        assert_eq!(state.counts, vec![8, 3]);
        assert_eq!(state.voters, 11);
        assert_eq!(state.selected, vec![1]);
        assert!(state.history_complete);
        assert!(!state.refreshing);
        assert!(!state.refresh_needed);
    }

    #[test]
    fn creation_keys_and_exact_option_hashes_are_preserved() {
        let raw = creation_archive(&content(), &[7; 32]);
        assert_eq!(secret(&raw), Some([7; 32].as_slice()));
        assert!(raw.poll_creation_message_v3.is_set());
        assert_eq!(classify(&raw), Some(content()));
        let options = vec!["Pizza".into(), " Pizza ".into()];
        let hash = whatsapp_rust::wacore::poll::compute_option_hash(" Pizza ").to_vec();
        assert_eq!(choices_for(&options, &[hash]), Some(vec![1]));
        assert_eq!(choices_for(&options, &[]), Some(vec![]));
        assert_eq!(choices_for(&options, &[vec![0; 32]]), None);
    }

    #[tokio::test]
    async fn an_encrypted_vote_before_its_poll_is_decrypted_without_network_access() {
        let (mut worker, _events, mut commands, _wa) = super::super::receipt_tests::worker();
        let directory = tempfile::tempdir().unwrap();
        let session_path = directory.path().join("session.db");
        let bot = Bot::builder()
            .with_backend(
                whatsapp_rust::store::SqliteStore::new(session_path.to_str().unwrap())
                    .await
                    .unwrap(),
            )
            .build()
            .await
            .unwrap();
        // Building a client initializes the protocol store; never run/connect this bot.
        worker.client = Some(bot.client());
        let creator = "100@s.whatsapp.net";
        let voter = "200@s.whatsapp.net";
        let key = [7; 32];
        let hashes = vec![whatsapp_rust::wacore::poll::compute_option_hash("Pasta").to_vec()];
        let (payload, iv) = whatsapp_rust::wacore::poll::encrypt_poll_vote_with_secret(
            &hashes, &key, "poll", creator, voter,
        )
        .unwrap();
        let update = wa::message::PollUpdateMessage {
            poll_creation_message_key: MessageField::some(wa::MessageKey {
                id: Some("poll".into()),
                ..Default::default()
            }),
            vote: MessageField::some(wa::message::PollEncValue {
                enc_payload: Some(payload),
                enc_iv: Some(iv.to_vec()),
            }),
            sender_timestamp_ms: Some(20_000),
            ..Default::default()
        };
        worker.ingest_poll_vote("chat", "vote", voter, false, 20, &update);
        assert_eq!(worker.poll_decrypting, 0);
        let mut row = crate::archive::tests::message("chat", "poll", 1, false);
        row.sender = creator.into();
        row.content = content();
        let raw = creation_archive(&row.content, &key);
        worker.remember_poll(&row, &raw, creator, None);
        worker.store_message(row.clone(), Some(raw.encode_to_vec()), None);
        worker.pump_poll_votes();
        assert_eq!(worker.poll_decrypting, 1);
        let result = tokio::time::timeout(Duration::from_secs(5), commands.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(result, Command::PollDecoded { .. }));
        worker.handle_command(result).await;
        worker.polish_poll(&mut row);
        let Content::Poll { state, .. } = row.content else {
            panic!("poll")
        };
        assert_eq!(state.counts, vec![0, 1]);
        assert_eq!(state.voters, 1);
        assert!(state.can_vote);
        assert_eq!(worker.poll_decrypting, 0);
    }

    #[test]
    fn privacy_id_re_votes_count_once_and_history_cannot_undo_them() {
        let (mut worker, _events, _commands, _wa) = super::super::receipt_tests::worker();
        let mut row = crate::archive::tests::message("chat", "poll", 1, false);
        row.content = content();
        worker.archive.insert_message(&row, None).unwrap();
        for (voter, at, selected) in [
            ("200@lid", 10, vec![0]),
            ("300@s.whatsapp.net", 20, vec![1]),
        ] {
            worker
                .archive
                .save_poll_vote(&PollVote {
                    chat: "chat".into(),
                    poll: "poll".into(),
                    voter: voter.into(),
                    sender: voter.into(),
                    update_id: at.to_string(),
                    at,
                    from_me: false,
                    choices: Some(selected),
                    encrypted: None,
                })
                .unwrap();
        }
        worker.learn_lid("200", "300");
        let update = wa::PollUpdate {
            poll_update_message_key: MessageField::some(wa::MessageKey {
                id: Some("5".into()),
                participant: Some("200@lid".into()),
                ..Default::default()
            }),
            vote: MessageField::some(wa::message::PollVoteMessage {
                selected_options: vec![
                    whatsapp_rust::wacore::poll::compute_option_hash("Pizza").to_vec(),
                ],
            }),
            sender_timestamp_ms: Some(5),
            ..Default::default()
        };
        worker.history_poll_votes(&row, &[update]);
        worker.polish_poll(&mut row);
        let Content::Poll { state, .. } = row.content else {
            panic!("poll")
        };
        assert_eq!(state.counts, vec![0, 1]);
        assert_eq!(state.voters, 1);
    }
}
