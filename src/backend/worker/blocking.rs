//! Use the library's blocking API; the archive keeps no copy of the list.

use super::*;

impl Worker {
    pub(super) fn publish_blocklist(&self, error: Option<crate::blocking::Failure>) {
        self.emit(Event::Blocklist {
            state: self.blocklist.clone(),
            error,
        });
    }

    pub(super) fn fetch_blocklist(&mut self) {
        let Some(client) = self.connected_client() else {
            return;
        };
        if self.blocklist.loading || self.blocklist.pending {
            return;
        }
        self.blocklist.loading = true;
        self.blocklist.failed = false;
        self.blocklist.generation = self.blocklist.generation.wrapping_add(1);
        let generation = self.blocklist.generation;
        self.publish_blocklist(None);
        let commands = self.commands.clone();
        tokio::spawn(async move {
            let result = client
                .blocking()
                .get_blocklist()
                .await
                .map(|entries| {
                    entries
                        .into_iter()
                        .map(|entry| entry.jid.to_non_ad_string())
                        .collect()
                })
                .map_err(|_| ());
            let _ = commands.send(Command::BlocklistFetched { generation, result });
        });
    }

    pub(super) async fn blocklist_fetched(
        &mut self,
        generation: u64,
        result: Result<Vec<ChatId>, ()>,
    ) {
        if generation != self.blocklist.generation {
            return;
        }
        self.blocklist.failed = result.is_err();
        if let Ok(entries) = result {
            let mut ids = HashSet::new();
            for id in entries {
                if let Some(jid) = Self::jid_of(&id) {
                    let id = self.canonical_sync_chat(&jid).await;
                    if crate::blocking::eligible(&id, self.me_pn.as_deref(), self.me_lid.as_deref())
                    {
                        ids.insert(id);
                    }
                }
            }
            self.blocklist.ids = Some(ids);
        }
        self.blocklist.loading = false;
        self.publish_blocklist(None);
    }

    pub(super) fn set_blocked(&mut self, chat: ChatId, blocked: bool) {
        let chat = self.canonical_str(&chat);
        if !crate::blocking::eligible(&chat, self.me_pn.as_deref(), self.me_lid.as_deref()) {
            return;
        }
        let (Some(client), Some(jid)) = (self.connected_client(), Self::jid_of(&chat)) else {
            self.publish_blocklist(Some(crate::blocking::Failure::Offline));
            return;
        };
        let Some(generation) = self.blocklist.begin_change() else {
            return;
        };
        self.publish_blocklist(None);
        let commands = self.commands.clone();
        tokio::spawn(async move {
            let result = if blocked {
                client.blocking().block(&jid).await
            } else {
                client.blocking().unblock(&jid).await
            }
            .map_err(|_| ());
            let _ = commands.send(Command::BlockedSet {
                generation,
                chat,
                blocked,
                result,
            });
        });
    }

    pub(super) fn blocked_set(
        &mut self,
        generation: u64,
        chat: ChatId,
        blocked: bool,
        result: Result<(), ()>,
    ) {
        if generation != self.blocklist.generation {
            return;
        }
        self.blocklist.pending = false;
        if result.is_err() {
            self.publish_blocklist(Some(crate::blocking::Failure::Refused));
            return;
        }
        let chat = self.canonical_str(&chat);
        let ids = self.blocklist.ids.get_or_insert_default();
        if blocked {
            ids.insert(chat);
        } else {
            ids.remove(&chat);
        }
        self.publish_blocklist(None);
        self.fetch_blocklist();
    }

    /// Restore composed content if the block arrived after the UI queued a send.
    pub(super) fn refuse_blocked(&self, command: Command) {
        let (chat, quoting, unsent) = match command {
            Command::SendText {
                chat,
                text,
                quoting,
                ..
            } => (chat, quoting, Unsent::Text(text)),
            Command::SendLocation {
                chat,
                latitude,
                longitude,
                quoting,
            } => (
                chat,
                quoting,
                Unsent::Location {
                    latitude,
                    longitude,
                },
            ),
            Command::SendFiles {
                chat,
                paths,
                documents,
                view_once,
                caption,
                quoting,
                ..
            } => (
                chat,
                quoting,
                Unsent::Files {
                    paths,
                    documents,
                    view_once,
                    caption,
                },
            ),
            Command::SendImage {
                chat,
                width,
                height,
                rgba,
                view_once,
                caption,
                quoting,
                ..
            } => (
                chat,
                quoting,
                Unsent::Image {
                    width,
                    height,
                    rgba,
                    view_once,
                    caption,
                },
            ),
            Command::SendVoice {
                chat,
                samples,
                quoting,
            } => (chat, quoting, Unsent::Voice(samples)),
            Command::SendSticker { chat, quoting, .. }
            | Command::StickerMade {
                chat: Some(chat),
                quoting,
                ..
            } => (chat, quoting, Unsent::Sticker),
            Command::SendGif { chat, quoting, .. } => (chat, quoting, Unsent::Gif),
            Command::VotePoll { chat, message, .. } => {
                self.emit(Event::PollVoted {
                    chat,
                    message,
                    error: Some("Unblock this contact to send messages.".into()),
                });
                return;
            }
            Command::EditText { chat, id, .. } => {
                self.emit_message(&chat, &id);
                self.emit(Event::Error(
                    "Unblock this contact to send messages.".into(),
                ));
                return;
            }
            Command::CreatePoll { chat, .. } => {
                self.emit(Event::PollCreated {
                    chat,
                    error: Some("Unblock this contact to send messages.".into()),
                });
                return;
            }
            _ => {
                self.emit(Event::Error(
                    "Unblock this contact to send messages.".into(),
                ));
                return;
            }
        };
        self.refuse(chat, quoting, unsent, Refusal::Blocked);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PEER: &str = "15550002222@s.whatsapp.net";

    #[test]
    fn blocked_contact_mapping_never_exposes_a_locked_contact_between_events() {
        for blocked in [false, true] {
            let (mut worker, events, _, _) = super::super::receipt_tests::worker();
            let lid = "200000000000001@lid";
            worker.archive.ensure_chat(lid, "Private fixture").unwrap();
            worker.archive.set_locked(lid, true).unwrap();
            worker.blocklist.ids = Some(if blocked {
                [lid.into()].into()
            } else {
                HashSet::new()
            });

            let root = tempfile::tempdir().unwrap();
            let (mut app, incoming) = crate::app::App::headless(
                crate::paths::AppDirs::under(root.path()),
                crate::settings::Settings::default(),
            );
            crate::demo::populate(&mut app);
            let mut chat = crate::model::Chat::new(lid.into(), "Private fixture".into());
            chat.locked = true;
            app.chats = vec![chat];
            app.blocklist = worker.blocklist.clone();
            app.dialog = Some(crate::model::Dialog::ConfirmBlock {
                chat: lid.into(),
                blocked: !blocked,
                from_list: blocked,
            });
            let ctx = egui::Context::default();
            app.attach(&ctx);
            worker.learn_lid("200000000000001", "15550002222");
            // A frame may consume just one event before the next one is queued.
            for event in events.try_iter() {
                incoming.send(event).unwrap();
                let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                    app.background_frame(ui.ctx());
                    app.frame_ui(ui);
                });
                output.textures_delta.clear();
                for id in app.blocklist.ids.iter().flatten() {
                    assert!(
                        app.chat(id).is_some_and(|chat| chat.locked),
                        "lost lock for {id}"
                    );
                }
                if let Some(crate::model::Dialog::ConfirmBlock { chat, .. }) = &app.dialog {
                    assert!(
                        app.chat(chat).is_some_and(|chat| chat.locked),
                        "lost confirmation lock"
                    );
                }
            }
            assert_eq!(app.blocklist.contains(PEER), blocked);
            assert!(!app.blocklist.contains(lid));
            assert!(
                matches!(app.dialog, Some(crate::model::Dialog::ConfirmBlock { chat, .. }) if chat == PEER)
            );
        }
    }

    #[test]
    fn blocklist_mapping_publishes_chat_locks_and_ids_together() {
        let (mut worker, events, _, _) = super::super::receipt_tests::worker();
        let lid = "200000000000001@lid";
        worker.archive.ensure_chat(lid, "Private fixture").unwrap();
        worker.archive.set_locked(lid, true).unwrap();
        worker.blocklist.ids = Some([lid.into()].into());
        worker.learn_lid("200000000000001", "15550002222");
        assert!(events.try_iter().any(|event| matches!(event,
            Event::BlocklistMapped { from, to, chats, state }
                if from == lid && to == PEER && state.contains(PEER)
                    && chats.iter().any(|chat| chat.id == PEER && chat.locked)
        )));
    }

    #[tokio::test]
    async fn blocklist_failures_and_stale_replies_keep_confirmed_state() {
        let (mut worker, events, _, _) = super::super::receipt_tests::worker();
        worker.blocklist.ids = Some([PEER.into()].into());
        worker.blocklist.generation = 2;
        worker.blocklist.loading = true;
        worker.blocklist_fetched(1, Ok(vec![])).await;
        assert!(worker.blocklist.loading);
        worker.blocklist_fetched(2, Err(())).await;
        assert!(worker.blocklist.contains(PEER));
        assert!(worker.blocklist.failed);
        assert!(!worker.blocklist.loading);
        worker.blocklist.pending = true;
        worker.blocked_set(2, PEER.into(), false, Err(()));
        assert!(worker.blocklist.contains(PEER));
        assert!(!worker.blocklist.pending);
        assert!(
            events
                .try_iter()
                .any(|e| matches!(e, Event::Blocklist { error: Some(_), .. }))
        );
        worker.blocklist.loading = true;
        let change = worker
            .blocklist
            .begin_change()
            .expect("a write supersedes a read");
        assert!(
            worker.blocklist.begin_change().is_none(),
            "one write at a time"
        );
        worker.blocklist_fetched(2, Ok(vec![])).await;
        assert!(worker.blocklist.contains(PEER) && worker.blocklist.pending);
        worker.blocked_set(change, PEER.into(), false, Ok(()));
        assert!(!worker.blocklist.contains(PEER));
    }

    #[tokio::test]
    async fn blocklist_resolves_ids_and_unblocks_contacts_without_chats() {
        let (mut worker, _, _, _) = super::super::receipt_tests::worker();
        worker.learn_lid("200000000000001", "15550002222");
        worker
            .blocklist_fetched(0, Ok(vec!["200000000000001@lid".into()]))
            .await;
        assert!(worker.blocklist.contains(PEER));
        assert!(worker.archive.chat(PEER).unwrap().is_none());
        worker.blocked_set(0, PEER.into(), false, Ok(()));
        assert!(!worker.blocklist.contains(PEER));
        worker.blocklist.ids = Some(["200000000000002@lid".into()].into());
        worker.learn_lid("200000000000002", "15550003333");
        assert!(worker.blocklist.contains("15550003333@s.whatsapp.net"));
    }

    #[tokio::test]
    async fn blocked_sends_are_returned_and_groups_remain_writable() {
        let (mut worker, events, _, _) = super::super::receipt_tests::worker();
        worker.blocklist.ids = Some([PEER.into()].into());
        worker
            .handle_command(Command::SendText {
                chat: PEER.into(),
                text: "Fixture draft".into(),
                quoting: None,
                mentions: vec![],
            })
            .await;
        assert!(events.try_iter().any(|event| matches!(event, Event::SendRefused { unsent: Unsent::Text(text), reason: Refusal::Blocked, .. } if text == "Fixture draft")));
        assert!(!worker.destination_writable(PEER));
        assert!(!worker.forward_writable(PEER));
        assert!(worker.destination_writable("120363000000001@g.us"));
        let generation = crate::calls::next_generation();
        worker
            .start_call(
                PEER.into(),
                false,
                generation,
                Arc::new(std::sync::atomic::AtomicBool::new(false)),
            )
            .await;
        assert!(worker.call.is_none());
        assert!(events.try_iter().any(|event| matches!(event, Event::Call(call) if call.generation == generation && !call.phase.is_live())));
    }

    #[tokio::test]
    async fn blocked_sends_preserve_every_payload_and_quote() {
        let (mut worker, events, _, _) = super::super::receipt_tests::worker();
        let peer = super::super::receipt_tests::PEER;
        worker.blocklist.ids = Some([peer.into()].into());
        let quoting = Some(crate::model::ReplyTarget::from("missing-quote"));
        let mut sends = super::super::receipt_tests::reply_sends(quoting.clone());
        sends.push((
            Command::SendLocation {
                chat: peer.into(),
                latitude: 1.25,
                longitude: 2.5,
                quoting: quoting.clone(),
            },
            Unsent::Location {
                latitude: 1.25,
                longitude: 2.5,
            },
        ));
        for (command, expected) in sends {
            worker.handle_command(command).await;
            let events: Vec<_> = events.try_iter().collect();
            assert!(matches!(events.as_slice(), [Event::SendRefused {
                chat, quoting: target, unsent, reason: Refusal::Blocked,
            }] if chat == peer && target == &quoting && unsent == &expected));
        }
    }

    #[tokio::test]
    async fn a_blocked_recipient_refuses_the_whole_forward_batch() {
        let (mut worker, events, _, _) = super::super::receipt_tests::worker();
        let group = "120363000000001@g.us";
        worker.blocklist.ids = Some([PEER.into()].into());
        worker
            .handle_command(Command::Forward {
                from_chat: "15550004444@s.whatsapp.net".into(),
                messages: vec!["forward-fixture".into()],
                to_chats: vec![group.into(), PEER.into()],
            })
            .await;
        assert!(worker.forward_queue.is_none());
        assert!(worker.archive.messages(group, None, 10).unwrap().is_empty());
        let events: Vec<_> = events.try_iter().collect();
        assert!(matches!(events.as_slice(), [Event::Error(error)]
            if error == "Unblock this contact to send messages."));
    }

    #[tokio::test]
    async fn blocked_sticker_completion_is_refused_at_the_worker_boundary() {
        let (mut worker, events, _, _) = super::super::receipt_tests::worker();
        worker.blocklist.ids = Some([PEER.into()].into());
        worker
            .handle_command(Command::StickerMade {
                result: Ok("fixture-made-sticker.webp".into()),
                chat: Some(PEER.into()),
                quoting: None,
            })
            .await;
        let events: Vec<_> = events.try_iter().collect();
        assert!(matches!(events.as_slice(), [Event::SendRefused {
            chat,
            unsent: Unsent::Sticker,
            reason: Refusal::Blocked,
            ..
        }] if chat == PEER));
    }

    #[tokio::test]
    async fn a_blocked_edit_restores_only_the_message_not_a_composer_draft() {
        let (mut worker, events, _, _) = super::super::receipt_tests::worker();
        let mut original = super::super::receipt_tests::own_message("edit-fixture", 1);
        original.chat = PEER.into();
        worker.archive.insert_message(&original, None).unwrap();
        worker.blocklist.ids = Some([PEER.into()].into());
        worker
            .handle_command(Command::EditText {
                chat: PEER.into(),
                id: original.id.clone(),
                text: "Unaccepted edit".into(),
                mentions: vec![],
            })
            .await;
        let events: Vec<_> = events.try_iter().collect();
        assert!(
            matches!(events.as_slice(), [Event::MessageUpdated(message), Event::Error(error)]
            if message.id == original.id && message.content == original.content
                && error == "Unblock this contact to send messages.")
        );
    }

    #[tokio::test]
    async fn disconnect_invalidates_tasks_and_private_recovery_withholds_the_list() {
        let (mut worker, events, _, _) = super::super::receipt_tests::worker();
        worker.blocklist.ids = Some([PEER.into()].into());
        worker.blocklist.pending = true;
        worker.blocklist.invalidate();
        worker.blocked_set(0, PEER.into(), false, Ok(()));
        assert!(worker.blocklist.contains(PEER));
        assert!(!worker.blocklist.pending);
        assert!(events.try_iter().next().is_none());
        worker.privacy_ready = false;
        worker.publish_blocklist(None);
        worker.emit(Event::BlocklistMapped {
            from: "200000000000001@lid".into(),
            to: PEER.into(),
            chats: vec![],
            state: worker.blocklist.clone(),
        });
        assert!(events.try_iter().next().is_none());
        worker.privacy_ready = true;
        worker.set_blocked(PEER.into(), false);
        assert!(worker.blocklist.contains(PEER));
        assert!(
            events
                .try_iter()
                .any(|e| matches!(e, Event::Blocklist { error: Some(_), .. }))
        );
    }
}
