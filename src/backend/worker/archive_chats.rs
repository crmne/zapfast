use super::*;
use crate::backend::ArchiveFailure;

impl Worker {
    pub(super) fn cancel_archive(&mut self, chat: &str) {
        let pending: Vec<_> = self
            .archive_requests
            .keys()
            .filter(|pending| self.canonical_str(pending) == chat)
            .cloned()
            .collect();
        for pending in pending {
            self.archive_requests.remove(&pending);
        }
    }

    pub(super) fn archive_on_phone(&mut self, chat: &str, archived: bool) {
        let chat = self.canonical_str(chat);
        if self
            .archive_requests
            .keys()
            .any(|pending| self.canonical_str(pending) == chat)
        {
            return;
        }
        let (Some(client), Some(jid)) = (self.connected_client(), Self::jid_of(&chat)) else {
            self.emit(Event::ArchiveFailed {
                archived,
                failure: ArchiveFailure::Offline,
            });
            return;
        };
        let last = match self.newest_on_phone(&chat) {
            Ok(last) => last,
            Err(_) => {
                log::warn!("could not read a chat to archive it on the phone");
                self.emit(Event::ArchiveFailed {
                    archived,
                    failure: ArchiveFailure::Read,
                });
                return;
            }
        };
        self.archive_sequence = self.archive_sequence.wrapping_add(1);
        let request = self.archive_sequence;
        self.archive_requests.insert(chat.clone(), request);
        let at = jiff::Timestamp::now().as_millisecond();
        let range = Some(archive_range(&jid, last.as_ref()));
        let commands = self.commands.clone();
        tokio::spawn(async move {
            let result = if archived {
                client.chat_actions().archive_chat(&jid, range).await
            } else {
                client.chat_actions().unarchive_chat(&jid, range).await
            }
            .map_err(|_| ());
            let _ = commands.send(Command::Archived {
                chat,
                archived,
                at,
                request,
                result,
            });
        });
    }

    pub(super) fn archived(
        &mut self,
        chat: ChatId,
        archived: bool,
        at: i64,
        request: u64,
        result: Result<(), ()>,
    ) {
        if self.archive_requests.get(&chat) != Some(&request) {
            return;
        }
        self.archive_requests.remove(&chat);
        let chat = self.canonical_str(&chat);
        if result.is_err() {
            log::warn!("could not synchronize a chat's archive state");
            self.emit(Event::ArchiveFailed {
                archived,
                failure: ArchiveFailure::Sync,
            });
            return;
        }
        if self.archive.set_archived_at(&chat, archived, at).is_err() {
            log::warn!("could not store a confirmed chat archive change");
            self.emit(Event::ArchiveFailed {
                archived,
                failure: ArchiveFailure::Write,
            });
            return;
        }
        self.emit_chat(&chat);
    }
}

#[cfg(test)]
mod tests {
    use super::super::receipt_tests::worker;
    use super::*;

    const CHAT: &str = "15550002222@s.whatsapp.net";

    #[tokio::test]
    async fn archive_changes_while_offline_preserve_the_phone_state() {
        for archived in [false, true] {
            let (mut worker, events, _, _) = worker();
            worker.archive.ensure_chat(CHAT, "Fixture").unwrap();
            worker
                .archive
                .set_archived_at(CHAT, !archived, 100)
                .unwrap();

            worker
                .handle_command(Command::SetArchived(CHAT.into(), archived))
                .await;

            assert_eq!(
                worker.archive.chat(CHAT).unwrap().unwrap().archived,
                !archived
            );
            assert!(worker.archive_requests.is_empty());
            assert!(matches!(events.try_recv(), Ok(Event::ArchiveFailed {
                archived: requested,
                failure: ArchiveFailure::Offline,
            }) if requested == archived));
            worker.archive.set_archived_at(CHAT, archived, 200).unwrap();
            assert_eq!(
                worker.archive.chat(CHAT).unwrap().unwrap().archived,
                archived
            );
        }
    }

    #[tokio::test]
    async fn a_disconnected_client_does_not_accept_an_archive_change() {
        let directory = tempfile::tempdir().unwrap();
        let store = whatsapp_rust::store::SqliteStore::open(
            directory.path().join("session.db").to_str().unwrap(),
        )
        .await
        .unwrap();
        let bot = Bot::builder().with_backend(store).build().await.unwrap();
        let (mut worker, events, _, _) = worker();
        worker.client = Some(bot.client());
        worker.archive.ensure_chat(CHAT, "Fixture").unwrap();

        worker
            .handle_command(Command::SetArchived(CHAT.into(), true))
            .await;

        assert!(!worker.archive.chat(CHAT).unwrap().unwrap().archived);
        assert!(worker.archive_requests.is_empty());
        assert!(matches!(
            events.try_recv(),
            Ok(Event::ArchiveFailed {
                failure: ArchiveFailure::Offline,
                ..
            })
        ));
    }

    #[tokio::test]
    async fn archive_changes_wait_for_whatsapp_and_failed_sends_leave_no_timestamp() {
        for archived in [false, true] {
            for succeeded in [false, true] {
                let (mut worker, events, _, _) = worker();
                worker.archive.ensure_chat(CHAT, "Fixture").unwrap();
                worker
                    .archive
                    .set_archived_at(CHAT, !archived, 100)
                    .unwrap();
                worker.archive_requests.insert(CHAT.into(), 1);
                assert_eq!(
                    worker.archive.chat(CHAT).unwrap().unwrap().archived,
                    !archived
                );

                worker
                    .handle_command(Command::Archived {
                        chat: CHAT.into(),
                        archived,
                        at: 300,
                        request: 1,
                        result: if succeeded { Ok(()) } else { Err(()) },
                    })
                    .await;

                assert!(worker.archive_requests.is_empty());
                assert_eq!(
                    worker.archive.chat(CHAT).unwrap().unwrap().archived,
                    if succeeded { archived } else { !archived }
                );
                if succeeded {
                    assert!(
                        matches!(events.try_recv(), Ok(Event::ChatUpdated(chat)) if chat.archived == archived)
                    );
                } else {
                    assert!(matches!(events.try_recv(), Ok(Event::ArchiveFailed {
                        archived: requested, failure: ArchiveFailure::Sync,
                    }) if requested == archived));
                    worker.archive.set_archived_at(CHAT, archived, 200).unwrap();
                    assert_eq!(
                        worker.archive.chat(CHAT).unwrap().unwrap().archived,
                        archived
                    );
                }
            }
        }
    }

    #[tokio::test]
    async fn a_newer_phone_archive_update_outlives_an_older_confirmation() {
        let (mut worker, events, _, _) = worker();
        worker.archive.ensure_chat(CHAT, "Fixture").unwrap();
        worker.archive_requests.insert(CHAT.into(), 1);
        worker.archive.set_archived_at(CHAT, false, 400).unwrap();

        worker
            .handle_command(Command::Archived {
                chat: CHAT.into(),
                archived: true,
                at: 300,
                request: 1,
                result: Ok(()),
            })
            .await;

        assert!(!worker.archive.chat(CHAT).unwrap().unwrap().archived);
        assert!(matches!(events.try_recv(), Ok(Event::ChatUpdated(chat)) if !chat.archived));
    }

    #[tokio::test]
    async fn a_late_archive_answer_cannot_write_the_next_account() {
        for result in [Ok(()), Err(())] {
            let (mut worker, events, _, _) = worker();
            worker.archive_requests.insert(CHAT.into(), 1);
            worker.archive_sequence = 1;
            worker.on_logged_out().await;
            assert!(worker.archive_requests.is_empty());
            assert_eq!(worker.archive_sequence, 1);
            events.try_iter().for_each(drop);
            worker.archive.ensure_chat(CHAT, "Next account").unwrap();
            worker.archive_requests.insert(CHAT.into(), 2);

            worker
                .handle_command(Command::Archived {
                    chat: CHAT.into(),
                    archived: true,
                    at: 300,
                    request: 1,
                    result,
                })
                .await;

            assert!(!worker.archive.chat(CHAT).unwrap().unwrap().archived);
            assert_eq!(worker.archive_requests.get(CHAT), Some(&2));
            assert!(events.try_recv().is_err());
        }
    }

    #[tokio::test]
    async fn an_archive_answer_follows_a_new_privacy_id_mapping() {
        let (mut worker, events, _, _) = worker();
        let lid = "100000001@lid";
        worker.archive.ensure_chat(CHAT, "Fixture").unwrap();
        worker.archive_requests.insert(lid.into(), 1);
        worker
            .lid_to_pn
            .insert("100000001".into(), "15550002222".into());

        worker
            .handle_command(Command::Archived {
                chat: lid.into(),
                archived: true,
                at: 300,
                request: 1,
                result: Ok(()),
            })
            .await;

        assert!(worker.archive.chat(CHAT).unwrap().unwrap().archived);
        assert!(
            matches!(events.try_recv(), Ok(Event::ChatUpdated(chat)) if chat.id == CHAT && chat.archived)
        );
        assert!(worker.archive.chat(lid).unwrap().is_none());
    }

    #[tokio::test]
    async fn a_confirmed_archive_write_failure_is_visible() {
        let (mut worker, events, _, _) = worker();
        worker.archive.ensure_chat(CHAT, "Fixture").unwrap();
        worker.archive_requests.insert(CHAT.into(), 1);
        crate::archive::tests::reject_archive_updates(&worker.archive);

        worker
            .handle_command(Command::Archived {
                chat: CHAT.into(),
                archived: true,
                at: 300,
                request: 1,
                result: Ok(()),
            })
            .await;

        assert!(worker.archive_requests.is_empty());
        assert!(matches!(
            events.try_recv(),
            Ok(Event::ArchiveFailed {
                failure: ArchiveFailure::Write,
                ..
            })
        ));
    }

    #[tokio::test]
    async fn deleting_a_chat_retires_its_pending_archive_request() {
        let (mut worker, events, _, _) = worker();
        worker.archive.ensure_chat(CHAT, "Fixture").unwrap();
        worker.archive_requests.insert(CHAT.into(), 1);
        worker.remove_chat(CHAT, 100, false);
        assert!(worker.archive_requests.is_empty());
        events.try_iter().for_each(drop);
        worker
            .archive
            .ensure_chat(CHAT, "New conversation")
            .unwrap();

        worker
            .handle_command(Command::Archived {
                chat: CHAT.into(),
                archived: true,
                at: 300,
                request: 1,
                result: Ok(()),
            })
            .await;

        assert!(!worker.archive.chat(CHAT).unwrap().unwrap().archived);
        assert!(events.try_recv().is_err());
    }
}
