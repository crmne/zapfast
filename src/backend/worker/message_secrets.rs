//! Resolve retained message secrets through the owning account's worker. The
//! protocol library uses this fallback when its shorter-lived session store
//! no longer has an announcement's encryption key.

use super::*;
use whatsapp_rust::cache_config::OriginalMessageResolver;
use whatsapp_rust::wacore::msg_secret::ResolvedMessageSecret;

pub(super) struct Resolver {
    pub commands: mpsc::UnboundedSender<Command>,
    pub session: u64,
}

#[whatsapp_rust::async_trait]
impl OriginalMessageResolver for Resolver {
    async fn resolve_msg_secret(&self, chat: &str, sender: &str, id: &str) -> Option<[u8; 32]> {
        self.resolve_msg_secret_with_metadata(chat, sender, id)
            .await
            .map(|found| found.secret)
    }
    async fn resolve_msg_secret_with_metadata(
        &self,
        chat: &str,
        sender: &str,
        id: &str,
    ) -> Option<ResolvedMessageSecret> {
        let (reply, result) = oneshot::channel();
        self.commands
            .send(Command::ResolveMessageSecret {
                session: self.session,
                chat: chat.to_owned(),
                sender: sender.to_owned(),
                message: id.to_owned(),
                reply,
            })
            .ok()?;
        result.await.ok().flatten()
    }
}

impl Worker {
    pub(super) fn resolve_message_secret(
        &self,
        session: u64,
        chat: &str,
        sender: &str,
        id: &str,
    ) -> Option<ResolvedMessageSecret> {
        if session != self.account_session {
            return None;
        }
        let chat = self.canonical_str(chat);
        let row = self.archive.message(&chat, id).ok().flatten()?;
        if self.canonical_str(sender) != row.sender && !(row.from_me && self.is_me(sender)) {
            return None;
        }
        let raw = self.archive.raw(&chat, id).ok().flatten()?;
        Some(ResolvedMessageSecret {
            secret: message_secret_from_raw(&raw)?.try_into().ok()?,
            message_ts: Some(row.timestamp),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn failed_and_stale_reactions_do_not_replace_the_confirmed_reaction() {
        let (mut worker, events, _, _) = super::super::receipt_tests::worker();
        let chat = "announcements@g.us";
        let id = "announcement";
        worker.archive.ensure_chat(chat, "Announcements").unwrap();
        worker
            .archive
            .set_group_info(chat, None, &["member@s.whatsapp.net".into()], true)
            .unwrap();
        let group = worker.archive.chat(chat).unwrap().unwrap();
        assert!(!group.can_send());
        assert!(group.can_react());
        let row = crate::archive::tests::message(chat, id, 100, false);
        worker.archive.insert_message(&row, None).unwrap();
        let me = worker.me();
        worker
            .archive
            .set_reaction(chat, id, &me, true, "👍")
            .unwrap();
        worker
            .reaction_generation
            .insert((chat.into(), id.into()), 2);
        worker.reaction_inflight.insert((chat.into(), id.into()));
        for (generation, succeeded) in [(1, true), (2, false)] {
            worker
                .handle_command(Command::ReactionSent {
                    session: worker.account_session,
                    generation,
                    chat: chat.into(),
                    message: id.into(),
                    emoji: "❤️".into(),
                    succeeded,
                })
                .await;
            assert_eq!(
                worker.archive.message(chat, id).unwrap().unwrap().reactions[0].emoji,
                "👍"
            );
        }
        assert!(
            events
                .try_iter()
                .any(|event| matches!(event, Event::Error(_)))
        );
        worker.reaction_inflight.insert((chat.into(), id.into()));
        worker
            .handle_command(Command::ReactionSent {
                session: worker.account_session,
                generation: 2,
                chat: chat.into(),
                message: id.into(),
                emoji: String::new(),
                succeeded: true,
            })
            .await;
        assert!(
            worker
                .archive
                .message(chat, id)
                .unwrap()
                .unwrap()
                .reactions
                .is_empty()
        );
    }

    #[tokio::test]
    async fn a_success_is_retained_when_the_next_queued_intent_cannot_send() {
        let (mut worker, _, _, _) = super::super::receipt_tests::worker();
        let chat = "announcements@g.us";
        let id = "post";
        worker.archive.ensure_chat(chat, "Announcements").unwrap();
        worker
            .archive
            .insert_message(&crate::archive::tests::message(chat, id, 100, false), None)
            .unwrap();
        worker
            .archive
            .set_reaction(chat, id, &worker.me(), true, "👍")
            .unwrap();
        let key = (chat.to_owned(), id.to_owned());
        worker.reaction_generation.insert(key.clone(), 1);
        worker.reaction_inflight.insert(key.clone());
        worker.reaction_pending.insert(key.clone(), "😂".into());
        worker
            .handle_command(Command::ReactionSent {
                session: worker.account_session,
                generation: 1,
                chat: chat.into(),
                message: id.into(),
                emoji: "❤️".into(),
                succeeded: true,
            })
            .await;
        // This synthetic worker is disconnected: the queued request fails,
        // preserving the successful first send rather than reverting to 👍.
        assert_eq!(
            worker.archive.message(chat, id).unwrap().unwrap().reactions[0].emoji,
            "❤️"
        );
        assert!(!worker.reaction_inflight.contains(&key));
        assert!(!worker.reaction_pending.contains_key(&key));
    }

    #[test]
    fn an_archived_announcement_supplies_its_secret_only_to_its_author_and_session() {
        let (worker, _, _, _) = super::super::receipt_tests::worker();
        let chat = "announcements@g.us";
        worker.archive.ensure_chat(chat, "Announcements").unwrap();
        let row = crate::archive::tests::message(chat, "parent", 100, false);
        let mut raw = wa::Message::default();
        let mut context = wa::MessageContextInfo::default();
        context.message_secret = Some(vec![7; 32]);
        raw.message_context_info = MessageField::some(context);
        worker
            .archive
            .insert_message(&row, Some(&raw.encode_to_vec()))
            .unwrap();
        let found = worker
            .resolve_message_secret(worker.account_session, chat, &row.sender, "parent")
            .unwrap();
        assert_eq!(found.secret, [7; 32]);
        assert_eq!(found.message_ts, Some(100));
        assert!(
            worker
                .resolve_message_secret(worker.account_session + 1, chat, &row.sender, "parent")
                .is_none()
        );
        assert!(
            worker
                .resolve_message_secret(
                    worker.account_session,
                    chat,
                    "someone-else@s.whatsapp.net",
                    "parent"
                )
                .is_none()
        );
        worker.archive.delete_message(chat, "parent").unwrap();
        assert!(
            worker
                .resolve_message_secret(worker.account_session, chat, &row.sender, "parent")
                .is_none()
        );
    }
}
