//! Messages a contact sent to one of their broadcast lists.
//!
//! WhatsApp delivers such a message addressed to the list
//! (`<id>@broadcast`) with the contact as its participant, and shows it in
//! the chat with that contact. It is filed there; `messages.broadcast_list`
//! keeps the list only so that its receipts can go back the way WhatsApp
//! addresses them.

use std::collections::HashSet;

use rusqlite::{OptionalExtension, params};

use super::{Archive, ChatKind, Reactions, Result, kind_name};

/// What moving one broadcast-list chat did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Adopted {
    /// Messages now in their senders' chats.
    pub moved: usize,
    /// Messages left in the list chat: their sender has no chat to go to,
    /// or that chat removed them.
    pub left: usize,
}

/// Finds a received message by the list it came through.
pub(super) const INDEX: &str = "
CREATE INDEX IF NOT EXISTS messages_by_broadcast_list ON messages (broadcast_list, id)
    WHERE broadcast_list IS NOT NULL;
";

impl Archive {
    /// Remembers that a received message came through `list`.
    pub fn set_broadcast_list(&self, chat: &str, id: &str, list: &str) -> Result<()> {
        self.connection.execute(
            "UPDATE messages SET broadcast_list = ?3 WHERE chat = ?1 AND id = ?2 AND NOT from_me",
            params![chat, id, list],
        )?;
        Ok(())
    }

    /// The broadcast list a received message came through, if any.
    pub fn broadcast_list(&self, chat: &str, id: &str) -> Result<Option<String>> {
        self.connection
            .query_row(
                "SELECT broadcast_list FROM messages WHERE chat = ?1 AND id = ?2",
                params![chat, id],
                |row| row.get(0),
            )
            .optional()
            .map(Option::flatten)
    }

    /// The chat that holds the message `id` received through `list`.
    pub fn broadcast_list_chat(&self, list: &str, id: &str) -> Result<Option<String>> {
        self.connection
            .query_row(
                "SELECT chat FROM messages WHERE broadcast_list = ?1 AND id = ?2 LIMIT 1",
                params![list, id],
                |row| row.get(0),
            )
            .optional()
    }

    /// Broadcast-list chats that hold only received messages, each with the
    /// senders of its messages, in order. Earlier versions filed a contact's
    /// list messages there. A list chat with a message of ours in it is ours,
    /// and is left out, as is a locked one: its messages must not move into
    /// a chat that is not.
    ///
    /// It runs at every start, so it reads the chat list rather than every
    /// message, and tests each list for a message of ours once, not once
    /// per message: the time grows with the size of the lists, not with
    /// the archive.
    pub fn received_broadcast_lists(&self) -> Result<Vec<(String, Vec<String>)>> {
        let mut statement = self.connection.prepare(
            "SELECT m.chat, m.sender FROM messages m
             WHERE m.chat IN (SELECT c.id FROM chats c
                WHERE c.id LIKE '%@broadcast' AND c.id <> 'status@broadcast' AND NOT c.locked
                AND NOT EXISTS (SELECT 1 FROM messages o WHERE o.chat = c.id AND o.from_me))
             GROUP BY m.chat, m.sender ORDER BY m.chat, m.sender",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut lists: Vec<(String, Vec<String>)> = Vec::new();
        for row in rows {
            let (list, sender) = row?;
            match lists.last_mut() {
                Some((last, senders)) if *last == list => senders.push(sender),
                _ => lists.push((list, vec![sender])),
            }
        }
        Ok(lists)
    }

    /// Moves the messages of a broadcast-list chat into the chats of their
    /// senders, in one transaction. `moves` names, for each sender as
    /// stored, the chat its messages go to and the name to give that chat if
    /// it does not exist yet; the messages of a sender it does not name stay.
    ///
    /// A message the sender's chat removed (the chat was deleted or cleared
    /// past it, or the message was deleted there) stays in the list too, as
    /// does one the list chat removed or still has a deletion waiting for,
    /// and a sender's chat is only made for a message that moves. Ids, times,
    /// delivery state, reactions, media paths, stars, motion clips, polls and
    /// their votes go with the messages that move. Where a sender's chat
    /// already holds the same message, that row is kept and gains what it
    /// lacks: a deletion for everyone on either copy stands, a downloaded
    /// file on either copy stays, and the newer vote and star win. The list
    /// chat goes once it is empty, with its own state (muted, archived,
    /// pinned, labels, favorite mark, drafts), none of which is carried to
    /// the sender's chat. Nothing is sent to WhatsApp. A list chat with no
    /// messages, or with a message of ours, is left as it is.
    pub fn adopt_broadcast_list(
        &self,
        list: &str,
        moves: &[(String, String, String)],
    ) -> Result<Adopted> {
        let transaction = self.connection.unchecked_transaction()?;
        let (any, ours): (bool, bool) = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM messages WHERE chat = ?1),
                EXISTS(SELECT 1 FROM messages WHERE chat = ?1 AND from_me)",
            params![list],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        if !any || ours {
            return Ok(Adopted::default());
        }
        // The list's unread messages are its newest ones, whoever sent them.
        let unread: HashSet<String> = {
            let mut statement = transaction.prepare(
                "SELECT id FROM messages WHERE chat = ?1
                 ORDER BY timestamp DESC, COALESCE(history_order, 9223372036854775807) DESC, rowid DESC
                 LIMIT COALESCE((SELECT unread FROM chats WHERE id = ?1), 0)",
            )?;
            statement
                .query_map(params![list], |row| row.get(0))?
                .collect::<Result<_>>()?
        };
        let mut adopted = Adopted::default();
        let mut unread_left = unread.len();
        for (sender, chat, name) in moves {
            // What the sender's chat removed stays removed: its removal point
            // and its deleted messages are kept out. So is what the list chat
            // removed, and a message with a deletion still waiting under the
            // list: moving it would leave that deletion naming no message.
            let taken: Vec<(String, i64, bool)> = {
                let mut statement = transaction.prepare(
                    "SELECT m.id, m.timestamp, EXISTS(SELECT 1 FROM messages t WHERE t.chat = ?2 AND t.id = m.id)
                     FROM messages m WHERE m.chat = ?1 AND m.sender = ?3
                     AND m.timestamp > COALESCE((SELECT through FROM chat_removals WHERE chat = ?2), -1)
                     AND NOT EXISTS (SELECT 1 FROM message_removals r WHERE r.chat = ?2 AND r.id = m.id
                        AND r.account = COALESCE((SELECT value FROM meta WHERE key = 'me_pn'), ''))
                     AND NOT EXISTS (SELECT 1 FROM pending_message_removals p WHERE p.chat = ?2 AND p.id = m.id
                        AND p.confirmed = 1 AND p.account = (SELECT value FROM meta WHERE key = 'me_pn'))
                     AND m.timestamp > COALESCE((SELECT through FROM chat_removals WHERE chat = ?1), -1)
                     AND NOT EXISTS (SELECT 1 FROM message_removals r WHERE r.chat = ?1 AND r.id = m.id
                        AND r.account = COALESCE((SELECT value FROM meta WHERE key = 'me_pn'), ''))
                     AND NOT EXISTS (SELECT 1 FROM pending_message_removals p WHERE p.chat = ?1 AND p.id = m.id)
                     ORDER BY m.timestamp, m.rowid",
                )?;
                statement
                    .query_map(params![list, chat, sender], |row| {
                        Ok((row.get(0)?, row.get(1)?, row.get(2)?))
                    })?
                    .collect::<Result<_>>()?
            };
            if taken.is_empty() {
                continue;
            }
            let ids = serde_json::to_string(
                &taken
                    .iter()
                    .map(|(id, _, _)| id.as_str())
                    .collect::<Vec<_>>(),
            )
            .unwrap_or_default();
            transaction.execute(
                "INSERT OR IGNORE INTO chats (id, name, kind) VALUES (?1, ?2, ?3)",
                params![chat, name, kind_name(ChatKind::from_id(chat))],
            )?;
            // Unread as a message arriving now would be: new to the chat, and
            // not older than what was read there.
            let through = self.read_through(chat)?;
            let fresh = taken
                .iter()
                .filter(|(id, timestamp, held)| {
                    !held
                        && unread.contains(id)
                        && through.is_none_or(|through| *timestamp >= through)
                })
                .count();
            unread_left = unread_left.saturating_sub(
                taken
                    .iter()
                    .filter(|(id, _, _)| unread.contains(id))
                    .count(),
            );
            // Copied before the messages: deleting a list message below
            // also deletes what its triggers tie to it.
            for (table, columns) in [("polls", "id, creator, secret"), ("poll_history", "id")] {
                transaction.execute(
                    &format!(
                        "INSERT OR IGNORE INTO {table} (chat, {columns})
                         SELECT ?2, {columns} FROM {table} WHERE chat = ?1
                         AND id IN (SELECT value FROM json_each(?3))"
                    ),
                    params![list, chat, ids],
                )?;
            }
            transaction.execute(
                "INSERT INTO stars (chat, id, starred, starred_at)
                 SELECT ?2, id, starred, starred_at FROM stars WHERE chat = ?1
                 AND id IN (SELECT value FROM json_each(?3))
                 ON CONFLICT(chat, id) DO UPDATE SET
                    starred = excluded.starred, starred_at = excluded.starred_at
                 WHERE excluded.starred_at > stars.starred_at",
                params![list, chat, ids],
            )?;
            transaction.execute(
                "INSERT INTO poll_votes (chat, poll, voter, sender, update_id, at, from_me, choices, encrypted, attempted)
                 SELECT ?2, poll, voter, sender, update_id, at, from_me, choices, encrypted, attempted
                 FROM poll_votes WHERE chat = ?1 AND poll IN (SELECT value FROM json_each(?3))
                 ON CONFLICT(chat, poll, voter) DO UPDATE SET
                    sender = excluded.sender, update_id = excluded.update_id, at = excluded.at,
                    from_me = excluded.from_me, choices = excluded.choices,
                    encrypted = excluded.encrypted, attempted = excluded.attempted
                 WHERE (excluded.at, excluded.update_id) > (poll_votes.at, poll_votes.update_id)
                    OR ((excluded.at, excluded.update_id) = (poll_votes.at, poll_votes.update_id)
                        AND poll_votes.choices IS NULL AND excluded.choices IS NOT NULL)",
                params![list, chat, ids],
            )?;
            transaction.execute(
                "INSERT OR IGNORE INTO motion_clips (chat, parent, sender, raw)
                 SELECT ?2, parent, CASE WHEN sender = ?4 THEN ?2 ELSE sender END, raw
                 FROM motion_clips WHERE chat = ?1 AND parent IN (SELECT value FROM json_each(?3))",
                params![list, chat, ids, sender],
            )?;
            let mut copies = self.duplicate_copies(list, chat)?;
            copies.retain(|(id, _)| taken.iter().any(|(taken, _, _)| taken == id));
            // A person's messages in the chat with them are filed under that
            // chat's id, as they arrive.
            transaction.execute(
                "INSERT INTO messages (chat, id, sender, sender_name, from_me, timestamp, content, status, quoted, reactions, edited, raw, thumbnail, mentions, forwarded, delivered_at, read_at, history_order, broadcast_list)
                 SELECT ?2, id, ?2, sender_name, from_me, timestamp, content, status, quoted, reactions, edited, raw, thumbnail, mentions, forwarded, delivered_at, read_at, history_order, ?1
                 FROM messages WHERE chat = ?1 AND id IN (SELECT value FROM json_each(?3))
                 ON CONFLICT(chat, id) DO UPDATE SET
                    content = CASE WHEN excluded.edited > messages.edited THEN excluded.content ELSE messages.content END,
                    sender_name = COALESCE(messages.sender_name, excluded.sender_name),
                    status = MAX(messages.status, excluded.status),
                    quoted = COALESCE(messages.quoted, excluded.quoted),
                    reactions = CASE WHEN messages.reactions = '[]' THEN excluded.reactions ELSE messages.reactions END,
                    edited = MAX(messages.edited, excluded.edited),
                    raw = COALESCE(messages.raw, excluded.raw),
                    thumbnail = COALESCE(messages.thumbnail, excluded.thumbnail),
                    mentions = CASE WHEN excluded.edited > messages.edited OR messages.mentions = '[]'
                        THEN excluded.mentions ELSE messages.mentions END,
                    forwarded = MAX(messages.forwarded, excluded.forwarded),
                    delivered_at = COALESCE(messages.delivered_at, excluded.delivered_at),
                    read_at = COALESCE(messages.read_at, excluded.read_at),
                    history_order = COALESCE(messages.history_order, excluded.history_order),
                    broadcast_list = COALESCE(messages.broadcast_list, excluded.broadcast_list)",
                params![list, chat, ids],
            )?;
            self.settle_duplicates(chat, copies, Reactions::Both)?;
            transaction.execute(
                "UPDATE chats SET
                    last_activity = MAX(last_activity, COALESCE((SELECT MAX(timestamp) FROM messages WHERE chat = ?1), 0)),
                    unread = unread + ?2
                 WHERE id = ?1",
                params![chat, i64::try_from(fresh).unwrap_or(i64::MAX)],
            )?;
            for (id, _, _) in &taken {
                // A list chat has no pins: a pin row there is an orphan.
                self.delete_pin(list, id)?;
            }
            adopted.moved += transaction.execute(
                "DELETE FROM messages WHERE chat = ?1 AND id IN (SELECT value FROM json_each(?2))",
                params![list, ids],
            )?;
        }
        let left: i64 = transaction.query_row(
            "SELECT COUNT(*) FROM messages WHERE chat = ?1",
            params![list],
            |row| row.get(0),
        )?;
        adopted.left = usize::try_from(left).unwrap_or(usize::MAX);
        if adopted.left == 0 {
            self.purge_chat_rows(list)?;
            transaction.execute("DELETE FROM message_pins WHERE chat = ?1", params![list])?;
            // Only the mark here goes; a change queued for the phone stays.
            transaction.execute("DELETE FROM favorites WHERE chat = ?1", params![list])?;
            transaction.execute("DELETE FROM chats WHERE id = ?1", params![list])?;
        } else if adopted.moved > 0 {
            // It is dated and counted by what it kept.
            transaction.execute(
                "UPDATE chats SET unread = MIN(unread, ?2),
                    last_activity = COALESCE((SELECT MAX(timestamp) FROM messages WHERE chat = ?1), last_activity)
                 WHERE id = ?1",
                params![list, i64::try_from(unread_left).unwrap_or(i64::MAX)],
            )?;
        }
        transaction.commit()?;
        Ok(adopted)
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::archive::PollVote;
    use crate::model::{Content, Media, Message};

    const LIST: &str = "1234567890@broadcast";
    const PEER: &str = "4917663430455@s.whatsapp.net";

    fn received(chat: &str, id: &str, timestamp: i64) -> Message {
        Message {
            sender: PEER.into(),
            ..crate::archive::tests::message(chat, id, timestamp, false)
        }
    }

    fn photo(caption: Option<&str>) -> Content {
        Content::Image {
            motion: None,
            caption: caption.map(Into::into),
            media: Media {
                mime: "image/jpeg".into(),
                size: 10,
                width: None,
                height: None,
                album: None,
                path: None,
                state: Default::default(),
            },
        }
    }

    fn path_of(archive: &Archive, id: &str) -> Option<std::path::PathBuf> {
        archive
            .message(PEER, id)
            .unwrap()
            .expect("filed")
            .content
            .media()
            .and_then(|media| media.path.clone())
    }

    fn moves() -> Vec<(String, String, String)> {
        vec![(PEER.into(), PEER.into(), "Peer".into())]
    }

    /// A list chat holding `messages`, every one of them unread.
    fn list_with(archive: &Archive, messages: &[Message]) {
        archive.ensure_chat(LIST, "1234567890").unwrap();
        for message in messages {
            archive.insert_message(message, None).unwrap();
            archive.bump_unread(LIST).unwrap();
        }
    }

    fn ids(archive: &Archive, chat: &str) -> Vec<String> {
        archive
            .messages(chat, None, 10)
            .unwrap()
            .into_iter()
            .map(|message| message.id)
            .collect()
    }

    #[test]
    fn a_message_met_twice_keeps_what_either_copy_knew() {
        let archive = Archive::in_memory().unwrap();
        // The list's copy downloaded the photo; the chat's did not.
        let downloaded = Message {
            content: photo(None),
            ..received(LIST, "downloaded", 100)
        };
        // The chat's copy downloaded it; the list's copy was edited since.
        let edited = Message {
            content: photo(Some("Edited fixture")),
            edited: true,
            ..received(LIST, "edited", 110)
        };
        // Deleted for everyone on one side or the other.
        let revoked = Message {
            content: Content::Revoked,
            ..received(LIST, "revoked", 120)
        };
        let revoked_there = Message {
            content: Content::text("Edited fixture"),
            edited: true,
            ..received(LIST, "revoked-there", 130)
        };
        list_with(&archive, &[downloaded, edited, revoked, revoked_there]);
        archive
            .set_media_path(LIST, "downloaded", Path::new("C:/fixture/list.jpg"))
            .unwrap();
        archive.ensure_chat(PEER, "Peer").unwrap();
        for (id, timestamp, content) in [
            ("downloaded", 100, photo(None)),
            ("edited", 110, photo(None)),
            ("revoked", 120, Content::text("Hello fixture")),
            ("revoked-there", 130, Content::Revoked),
        ] {
            let message = Message {
                content,
                ..received(PEER, id, timestamp)
            };
            archive.insert_message(&message, None).unwrap();
        }
        archive
            .set_media_path(PEER, "edited", Path::new("C:/fixture/chat.jpg"))
            .unwrap();

        let adopted = archive.adopt_broadcast_list(LIST, &moves()).unwrap();

        assert_eq!(adopted, Adopted { moved: 4, left: 0 });
        assert_eq!(
            path_of(&archive, "downloaded").as_deref(),
            Some(Path::new("C:/fixture/list.jpg"))
        );
        let edited = archive.message(PEER, "edited").unwrap().unwrap();
        assert!(edited.edited);
        assert!(matches!(
            &edited.content,
            Content::Image { caption: Some(caption), .. } if caption == "Edited fixture"
        ));
        assert_eq!(
            path_of(&archive, "edited").as_deref(),
            Some(Path::new("C:/fixture/chat.jpg"))
        );
        for id in ["revoked", "revoked-there"] {
            assert_eq!(
                archive.message(PEER, id).unwrap().unwrap().content,
                Content::Revoked,
                "{id}"
            );
        }
        // Every one of them was in the chat already.
        assert_eq!(archive.chat(PEER).unwrap().unwrap().unread, 0);
    }

    #[test]
    fn what_the_senders_chat_removed_stays_removed() {
        let archive = Archive::in_memory().unwrap();
        list_with(
            &archive,
            &[
                received(LIST, "cleared", 100),
                received(LIST, "deleted", 200),
                received(LIST, "kept", 300),
            ],
        );
        archive.ensure_chat(PEER, "Peer").unwrap();
        archive.remove_chat_through(PEER, 150, false).unwrap();
        archive
            .confirm_message_removal("fixture-account", &[PEER.to_owned()], "deleted")
            .unwrap();

        let adopted = archive.adopt_broadcast_list(LIST, &moves()).unwrap();

        assert_eq!(adopted, Adopted { moved: 1, left: 2 });
        assert_eq!(ids(&archive, PEER), ["kept"]);
        assert_eq!(ids(&archive, LIST), ["cleared", "deleted"]);
        assert_eq!(archive.chat(PEER).unwrap().unwrap().unread, 1);
        // What stays in the list keeps its share of the unread count.
        assert_eq!(archive.chat(LIST).unwrap().unwrap().unread, 2);
    }

    #[test]
    fn a_chat_removed_past_every_message_is_not_made_again() {
        let archive = Archive::in_memory().unwrap();
        list_with(&archive, &[received(LIST, "m", 100)]);
        archive.remove_chat_through(PEER, 500, true).unwrap();

        let adopted = archive.adopt_broadcast_list(LIST, &moves()).unwrap();

        assert_eq!(adopted, Adopted { moved: 0, left: 1 });
        assert!(archive.chat(PEER).unwrap().is_none());
        assert!(archive.message(LIST, "m").unwrap().is_some());
        assert_eq!(archive.chat(LIST).unwrap().unwrap().unread, 1);
    }

    #[test]
    fn polls_votes_stars_and_clips_go_only_with_the_messages_that_move() {
        let archive = Archive::in_memory().unwrap();
        list_with(
            &archive,
            &[received(LIST, "old", 100), received(LIST, "poll", 200)],
        );
        archive.ensure_chat(PEER, "Peer").unwrap();
        archive.remove_chat_through(PEER, 150, false).unwrap();
        for id in ["old", "poll"] {
            archive
                .save_poll(LIST, id, PEER, b"fixture secret")
                .unwrap();
            archive.mark_poll_history(LIST, id).unwrap();
            archive.set_star(LIST, id, true, 300, false).unwrap();
            archive
                .put_motion_clip(LIST, id, PEER, b"fixture clip")
                .unwrap();
        }
        let vote = |chat: &str, at: i64, choice: usize| PollVote {
            chat: chat.into(),
            poll: "poll".into(),
            voter: PEER.into(),
            sender: PEER.into(),
            update_id: format!("vote-{at}"),
            at,
            from_me: false,
            choices: Some(vec![choice]),
            encrypted: None,
        };
        archive.save_poll_vote(&vote(LIST, 20, 1)).unwrap();
        // The chat already holds the same poll, with an older vote and an
        // older unstar.
        archive
            .insert_message(&received(PEER, "poll", 200), None)
            .unwrap();
        archive.save_poll_vote(&vote(PEER, 10, 0)).unwrap();
        archive.set_star(PEER, "poll", false, 250, false).unwrap();

        archive.adopt_broadcast_list(LIST, &moves()).unwrap();

        assert_eq!(
            archive
                .poll_votes(PEER, "poll")
                .unwrap()
                .iter()
                .map(|vote| (vote.at, vote.choices.clone()))
                .collect::<Vec<_>>(),
            [(20, Some(vec![1]))],
            "the newer vote wins"
        );
        assert!(archive.poll_key(PEER, "poll").unwrap().is_some());
        assert!(archive.has_poll_history(PEER, "poll").unwrap());
        assert_eq!(
            archive.motion_clip(PEER, "poll").unwrap().as_deref(),
            Some(&b"fixture clip"[..])
        );
        assert_eq!(
            archive
                .starred_ids(PEER)
                .unwrap()
                .into_iter()
                .collect::<Vec<_>>(),
            ["poll"],
            "the newer star wins"
        );
        // The message that stayed keeps its own, and none of it moved.
        assert!(archive.poll_key(PEER, "old").unwrap().is_none());
        assert!(!archive.has_poll_history(PEER, "old").unwrap());
        assert!(archive.motion_clip(PEER, "old").unwrap().is_none());
        assert!(archive.poll_key(LIST, "old").unwrap().is_some());
        assert!(archive.starred_ids(LIST).unwrap().contains("old"));
    }

    #[test]
    fn unread_counts_what_a_message_arriving_now_would_count() {
        let archive = Archive::in_memory().unwrap();
        list_with(
            &archive,
            &[received(LIST, "before", 200), received(LIST, "after", 400)],
        );
        archive.ensure_chat(PEER, "Peer").unwrap();
        archive
            .insert_message(&received(PEER, "read", 300), None)
            .unwrap();
        archive.mark_read_through(PEER, 300).unwrap();

        archive.adopt_broadcast_list(LIST, &moves()).unwrap();

        assert_eq!(archive.chat(PEER).unwrap().unwrap().unread, 1);
    }

    #[test]
    fn a_failure_on_the_way_changes_nothing() {
        let archive = Archive::in_memory().unwrap();
        list_with(&archive, &[received(LIST, "m", 100)]);
        archive.set_star(LIST, "m", true, 300, false).unwrap();
        archive
            .connection
            .execute_batch(
                "CREATE TRIGGER refuse_delete BEFORE DELETE ON messages
                 WHEN OLD.chat = '1234567890@broadcast'
                 BEGIN SELECT RAISE(ABORT, 'fixture failure'); END;",
            )
            .unwrap();

        assert!(archive.adopt_broadcast_list(LIST, &moves()).is_err());

        assert!(archive.chat(PEER).unwrap().is_none());
        assert!(archive.message(PEER, "m").unwrap().is_none());
        assert!(archive.starred_ids(PEER).unwrap().is_empty());
        assert_eq!(ids(&archive, LIST), ["m"]);
        assert_eq!(archive.chat(LIST).unwrap().unwrap().unread, 1);
        assert!(archive.starred_ids(LIST).unwrap().contains("m"));
    }

    #[test]
    fn the_list_chats_own_state_stays_behind() {
        let archive = Archive::in_memory().unwrap();
        list_with(&archive, &[received(LIST, "m", 100)]);
        archive.set_muted(LIST, Some(i64::MAX)).unwrap();
        archive.set_archived(LIST, true).unwrap();
        archive.set_pinned(LIST, true).unwrap();
        assert!(archive.set_favorite(LIST, true).unwrap());
        assert!(archive.pin(LIST, "m", 100, i64::MAX).unwrap());
        let queued = archive.pending_favorites().unwrap();
        assert!(queued.is_some());

        archive.adopt_broadcast_list(LIST, &moves()).unwrap();

        assert!(archive.chat(LIST).unwrap().is_none());
        let chat = archive.chat(PEER).unwrap().unwrap();
        assert_eq!(
            (chat.muted_until, chat.archived, chat.pinned),
            (None, false, false)
        );
        assert!(archive.favorites().unwrap().is_empty());
        // What the phone was asked for is still asked for.
        assert_eq!(archive.pending_favorites().unwrap(), queued);
        let pins: i64 = archive
            .connection
            .query_row("SELECT COUNT(*) FROM message_pins", [], |row| row.get(0))
            .unwrap();
        assert_eq!(pins, 0);
    }

    #[test]
    fn a_sender_without_a_chat_keeps_their_messages_in_the_list() {
        let archive = Archive::in_memory().unwrap();
        let stranger = Message {
            sender: "fixture@g.us".into(),
            ..received(LIST, "stranger", 100)
        };
        list_with(&archive, &[stranger, received(LIST, "m", 200)]);

        let adopted = archive.adopt_broadcast_list(LIST, &moves()).unwrap();

        assert_eq!(adopted, Adopted { moved: 1, left: 1 });
        assert_eq!(ids(&archive, PEER), ["m"]);
        assert_eq!(ids(&archive, LIST), ["stranger"]);
        // Both were unread: one moved, one stayed.
        assert_eq!(archive.chat(PEER).unwrap().unwrap().unread, 1);
        assert_eq!(archive.chat(LIST).unwrap().unwrap().unread, 1);
    }
    #[test]
    fn what_the_list_chat_removed_or_is_deleting_stays_in_it() {
        let archive = Archive::in_memory().unwrap();
        list_with(
            &archive,
            &[
                received(LIST, "cleared", 100),
                received(LIST, "deleted", 200),
                received(LIST, "deleting", 300),
                received(LIST, "kept", 400),
            ],
        );
        // An older build's list chat: cleared up to a point (a newer message
        // that arrived since is still there), one message with a deletion
        // barrier (the row survived a failed cleanup), one deletion waiting.
        archive
            .connection
            .execute(
                "INSERT INTO chat_removals (chat, through) VALUES (?1, 150)",
                params![LIST],
            )
            .unwrap();
        archive
            .connection
            .execute(
                "INSERT INTO message_removals (account, chat, id) VALUES ('', ?1, 'deleted')",
                params![LIST],
            )
            .unwrap();
        archive
            .queue_message_removal("fixture-account", LIST, "deleting")
            .unwrap();

        let adopted = archive.adopt_broadcast_list(LIST, &moves()).unwrap();

        assert_eq!(adopted, Adopted { moved: 1, left: 3 });
        assert_eq!(ids(&archive, PEER), ["kept"]);
        assert_eq!(ids(&archive, LIST), ["cleared", "deleted", "deleting"]);
    }

    #[test]
    fn a_list_chat_that_keeps_messages_is_dated_by_them() {
        let archive = Archive::in_memory().unwrap();
        let stranger = Message {
            sender: "fixture@g.us".into(),
            ..received(LIST, "stranger", 50)
        };
        list_with(&archive, &[stranger, received(LIST, "m", 200)]);
        assert_eq!(archive.chat(LIST).unwrap().unwrap().last_activity, 200);

        archive.adopt_broadcast_list(LIST, &moves()).unwrap();

        assert_eq!(archive.chat(LIST).unwrap().unwrap().last_activity, 50);
        assert_eq!(archive.chat(PEER).unwrap().unwrap().last_activity, 200);
    }

    #[test]
    fn finding_the_lists_does_not_grow_with_the_square_of_their_size() {
        let archive = Archive::in_memory().unwrap();
        archive.ensure_chat(LIST, "1234567890").unwrap();
        // 20,000 received list messages from 20 senders, and 100,000 in
        // another chat that need not be read. Small enough that the
        // quadratic query this replaced fails in about a minute, not hours.
        archive
            .connection
            .execute_batch(
                "WITH RECURSIVE n(i) AS (SELECT 0 UNION ALL SELECT i + 1 FROM n WHERE i < 19999)
                 INSERT INTO messages (chat, id, sender, from_me, timestamp, content)
                 SELECT '1234567890@broadcast', 'l' || i, (5000 + i % 20) || '@s.whatsapp.net', 0, i, '{}'
                 FROM n;
                 WITH RECURSIVE n(i) AS (SELECT 0 UNION ALL SELECT i + 1 FROM n WHERE i < 99999)
                 INSERT INTO messages (chat, id, sender, from_me, timestamp, content)
                 SELECT '1@s.whatsapp.net', 'd' || i, '1@s.whatsapp.net', i % 2, i, '{}' FROM n;",
            )
            .unwrap();
        let started = std::time::Instant::now();
        let lists = archive.received_broadcast_lists().unwrap();
        let took = started.elapsed();
        assert_eq!(lists.len(), 1);
        assert_eq!(lists[0].1.len(), 20);
        // Linear work takes well under a second even in a debug build.
        assert!(took < std::time::Duration::from_secs(10), "{took:?}");
    }

    #[test]
    fn a_placeholder_in_one_copy_gives_way_to_the_real_message_and_its_raw() {
        let archive = Archive::in_memory().unwrap();
        let real = Message {
            content: Content::text("Hello fixture"),
            ..received(LIST, "m", 100)
        };
        archive.ensure_chat(LIST, "1234567890").unwrap();
        archive.insert_message(&real, Some(b"real raw")).unwrap();
        archive.ensure_chat(PEER, "Peer").unwrap();
        let placeholder = Message {
            content: Content::Unsupported {
                what: "Waiting for this message".into(),
            },
            ..received(PEER, "m", 100)
        };
        archive
            .insert_message(&placeholder, Some(b"placeholder raw"))
            .unwrap();

        archive.adopt_broadcast_list(LIST, &moves()).unwrap();

        assert_eq!(
            archive.message(PEER, "m").unwrap().unwrap().content,
            Content::text("Hello fixture")
        );
        assert_eq!(
            archive.raw(PEER, "m").unwrap().as_deref(),
            Some(&b"real raw"[..])
        );
    }

    /// A received message from someone else in the list.
    fn from(id: &str, timestamp: i64, sender: &str) -> Message {
        Message {
            sender: sender.into(),
            ..received(LIST, id, timestamp)
        }
    }

    fn reaction(sender: &str, emoji: &str, from_me: bool) -> crate::model::Reaction {
        crate::model::Reaction {
            sender: sender.into(),
            from_me,
            emoji: emoji.into(),
        }
    }

    #[test]
    fn reactions_on_both_copies_are_kept_one_per_person() {
        let archive = Archive::in_memory().unwrap();
        archive
            .connection
            .execute(
                "INSERT INTO lids (lid, pn) VALUES ('167650256810092', '4917663430455')",
                [],
            )
            .unwrap();
        // Ours and the contact's, under their privacy id, on the list copy;
        // the contact's under their number on the chat's copy.
        let listed = Message {
            reactions: vec![
                reaction("me@s.whatsapp.net", "\u{1f44d}", true),
                reaction("167650256810092@lid", "\u{2764}", false),
            ],
            ..received(LIST, "m", 100)
        };
        list_with(&archive, &[listed]);
        archive.ensure_chat(PEER, "Peer").unwrap();
        let there = Message {
            reactions: vec![reaction(PEER, "\u{1f602}", false)],
            ..received(PEER, "m", 100)
        };
        archive.insert_message(&there, None).unwrap();

        archive.adopt_broadcast_list(LIST, &moves()).unwrap();

        assert_eq!(
            archive.message(PEER, "m").unwrap().unwrap().reactions,
            [
                reaction(PEER, "\u{1f602}", false),
                reaction("me@s.whatsapp.net", "\u{1f44d}", true),
            ]
        );
    }

    #[test]
    fn a_second_run_moves_nothing_and_changes_nothing() {
        let archive = Archive::in_memory().unwrap();
        list_with(
            &archive,
            &[
                from("stranger", 50, "fixture@g.us"),
                received(LIST, "a", 100),
                received(LIST, "b", 200),
            ],
        );
        let first = archive.adopt_broadcast_list(LIST, &moves()).unwrap();
        let state = |archive: &Archive| {
            let chat = archive.chat(PEER).unwrap().unwrap();
            let list = archive.chat(LIST).unwrap().unwrap();
            (
                chat.unread,
                chat.last_activity,
                list.unread,
                list.last_activity,
            )
        };
        let before = state(&archive);

        let second = archive.adopt_broadcast_list(LIST, &moves()).unwrap();

        assert_eq!(first, Adopted { moved: 2, left: 1 });
        assert_eq!(second, Adopted { moved: 0, left: 1 });
        assert_eq!(state(&archive), before);
        assert_eq!(before, (2, 200, 1, 50));
    }

    #[test]
    fn privacy_ids_phone_numbers_and_strangers_in_one_list() {
        let archive = Archive::in_memory().unwrap();
        let unmapped = "99999@lid";
        list_with(
            &archive,
            &[
                received(LIST, "pn", 100),
                from("mapped", 200, "111@lid"),
                from("unmapped", 300, unmapped),
                from("itself", 400, LIST),
                from("group", 500, "1-2@g.us"),
            ],
        );
        let moves = vec![
            (PEER.to_owned(), PEER.to_owned(), "Peer".to_owned()),
            ("111@lid".to_owned(), PEER.to_owned(), "Peer".to_owned()),
            (unmapped.to_owned(), unmapped.to_owned(), "Lid".to_owned()),
        ];

        let adopted = archive.adopt_broadcast_list(LIST, &moves).unwrap();

        assert_eq!(adopted, Adopted { moved: 3, left: 2 });
        let unread = |chat: &str| archive.chat(chat).unwrap().unwrap().unread;
        assert_eq!((unread(PEER), unread(unmapped), unread(LIST)), (2, 1, 2));
        assert_eq!(
            archive.message(PEER, "mapped").unwrap().unwrap().sender,
            PEER
        );
        // The privacy-id chat later merges into the phone-number chat and
        // keeps the list its message came through.
        archive.put_lid("99999", "4917663430455").unwrap();
        assert_eq!(
            archive.broadcast_list(PEER, "unmapped").unwrap().as_deref(),
            Some(LIST)
        );
    }

    #[test]
    fn a_failure_at_a_later_sender_undoes_the_earlier_ones() {
        let archive = Archive::in_memory().unwrap();
        let other = "99999@lid";
        list_with(&archive, &[received(LIST, "a", 100), from("b", 200, other)]);
        archive
            .connection
            .execute_batch(
                "CREATE TRIGGER refuse_chat BEFORE INSERT ON chats WHEN NEW.id = '99999@lid'
                 BEGIN SELECT RAISE(ABORT, 'fixture failure'); END;",
            )
            .unwrap();
        let moves = vec![
            (PEER.to_owned(), PEER.to_owned(), "Peer".to_owned()),
            (other.to_owned(), other.to_owned(), "Lid".to_owned()),
        ];

        assert!(archive.adopt_broadcast_list(LIST, &moves).is_err());

        assert!(archive.chat(PEER).unwrap().is_none());
        assert_eq!(ids(&archive, LIST), ["a", "b"]);
        assert_eq!(archive.chat(LIST).unwrap().unwrap().unread, 2);
    }

    #[test]
    fn messages_move_into_a_locked_senders_chat() {
        let archive = Archive::in_memory().unwrap();
        list_with(&archive, &[received(LIST, "m", 100)]);
        archive.ensure_chat(PEER, "Peer").unwrap();
        archive.set_locked(PEER, true).unwrap();

        let adopted = archive.adopt_broadcast_list(LIST, &moves()).unwrap();

        assert_eq!(adopted, Adopted { moved: 1, left: 0 });
        assert!(archive.chat(PEER).unwrap().unwrap().locked);
    }

    /// A deletion of the chat's copy, sent from here and not confirmed yet,
    /// still names the message the list copy merged into.
    #[test]
    fn a_deletion_waiting_in_the_senders_chat_still_finds_its_message() {
        let archive = Archive::in_memory().unwrap();
        list_with(&archive, &[received(LIST, "m", 100)]);
        archive.ensure_chat(PEER, "Peer").unwrap();
        archive
            .insert_message(&received(PEER, "m", 100), None)
            .unwrap();
        archive
            .queue_message_removal("fixture-account", PEER, "m")
            .unwrap();

        archive.adopt_broadcast_list(LIST, &moves()).unwrap();

        assert_eq!(
            archive.pending_message_removals("fixture-account").unwrap(),
            [(PEER.to_owned(), "m".to_owned())]
        );
        assert!(archive.message(PEER, "m").unwrap().is_some());
    }

    #[test]
    fn finding_a_message_by_its_list_uses_the_index() {
        let archive = Archive::in_memory().unwrap();
        let plan: Vec<String> = archive
            .connection
            .prepare(
                "EXPLAIN QUERY PLAN
                 SELECT chat FROM messages WHERE broadcast_list = ?1 AND id = ?2 LIMIT 1",
            )
            .unwrap()
            .query_map(params!["list", "id"], |row| row.get::<_, String>(3))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert!(
            plan.iter()
                .any(|step| step.contains("messages_by_broadcast_list")),
            "{plan:?}"
        );
    }

    /// The phone refused a deletion that kept a message in the list; once
    /// it is cancelled, the next start moves the message.
    #[test]
    fn a_refused_deletion_lets_the_message_move_at_the_next_start() {
        let archive = Archive::in_memory().unwrap();
        list_with(&archive, &[received(LIST, "m", 100)]);
        archive
            .queue_message_removal("fixture-account", LIST, "m")
            .unwrap();
        let first = archive.adopt_broadcast_list(LIST, &moves()).unwrap();
        archive.cancel_message_removal(LIST, "m").unwrap();

        let second = archive.adopt_broadcast_list(LIST, &moves()).unwrap();

        assert_eq!(first, Adopted { moved: 0, left: 1 });
        assert_eq!(second, Adopted { moved: 1, left: 0 });
        assert_eq!(ids(&archive, PEER), ["m"]);
    }

    /// A list we send to also held a contact's message. Once our own
    /// messages there are deleted, it holds only what it received, and
    /// that message goes to the contact's chat like any other.
    #[test]
    fn our_list_left_with_only_a_received_message_moves_it() {
        let archive = Archive::in_memory().unwrap();
        archive.ensure_chat(LIST, "1234567890").unwrap();
        archive
            .insert_message(
                &crate::archive::tests::message(LIST, "ours", 100, true),
                None,
            )
            .unwrap();
        archive
            .insert_message(&received(LIST, "theirs", 110), None)
            .unwrap();
        assert!(archive.received_broadcast_lists().unwrap().is_empty());
        archive.delete_message(LIST, "ours").unwrap();

        assert_eq!(
            archive.received_broadcast_lists().unwrap(),
            [(LIST.to_owned(), vec![PEER.to_owned()])]
        );
        let adopted = archive.adopt_broadcast_list(LIST, &moves()).unwrap();

        assert_eq!(adopted, Adopted { moved: 1, left: 0 });
    }

    /// Content a newer build wrote, which this one cannot read, is kept as
    /// it is, whether or not the list's copy was edited.
    #[test]
    fn content_this_build_cannot_read_is_kept_as_it_is() {
        let archive = Archive::in_memory().unwrap();
        let edited = Message {
            content: Content::text("Edited fixture"),
            edited: true,
            ..received(LIST, "edited", 100)
        };
        list_with(&archive, &[received(LIST, "plain", 100), edited]);
        archive.ensure_chat(PEER, "Peer").unwrap();
        let future = r#"{"kind":"futurekind","body":"from a newer build"}"#;
        for id in ["plain", "edited"] {
            archive
                .insert_message(&received(PEER, id, 100), None)
                .unwrap();
            archive
                .connection
                .execute(
                    "UPDATE messages SET content = ?1 WHERE chat = ?2 AND id = ?3",
                    params![future, PEER, id],
                )
                .unwrap();
        }

        archive.adopt_broadcast_list(LIST, &moves()).unwrap();

        for id in ["plain", "edited"] {
            let (stored, edited): (String, bool) = archive
                .connection
                .query_row(
                    "SELECT content, edited FROM messages WHERE chat = ?1 AND id = ?2",
                    params![PEER, id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();
            assert_eq!((stored.as_str(), edited), (future, false), "{id}");
        }
    }

    /// The other way round: the chat holds a placeholder, and the list's
    /// copy holds content a newer build wrote. That content goes in as it
    /// is stored.
    #[test]
    fn content_a_newer_build_wrote_replaces_a_placeholder() {
        let archive = Archive::in_memory().unwrap();
        list_with(&archive, &[received(LIST, "m", 100)]);
        let future = r#"{"kind":"futurekind","body":"from a newer build"}"#;
        archive
            .connection
            .execute(
                "UPDATE messages SET content = ?1 WHERE chat = ?2 AND id = 'm'",
                params![future, LIST],
            )
            .unwrap();
        archive.ensure_chat(PEER, "Peer").unwrap();
        let placeholder = Message {
            content: Content::Unsupported {
                what: "Waiting for this message".into(),
            },
            ..received(PEER, "m", 100)
        };
        archive.insert_message(&placeholder, None).unwrap();

        archive.adopt_broadcast_list(LIST, &moves()).unwrap();

        let stored: String = archive
            .connection
            .query_row(
                "SELECT content FROM messages WHERE chat = ?1 AND id = 'm'",
                params![PEER],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(stored, future);
    }

    /// Fills an in-memory archive with about a million messages and big
    /// broadcast lists, then times the start-up scan and the moves. It
    /// asserts nothing about time; it is for reading the numbers by hand.
    #[test]
    #[ignore = "a timing run on about a million messages: run it with --ignored --nocapture"]
    fn timing_on_a_million_messages() {
        let archive = Archive::in_memory().unwrap();
        let text = serde_json::to_string(&Content::text("fixture text body")).unwrap();
        let connection = &archive.connection;
        connection.execute_batch("BEGIN").unwrap();
        // 2000 direct chats of 450 messages, 200 lists of ours of 400.
        connection
            .execute(
                "WITH RECURSIVE n(i) AS (SELECT 0 UNION ALL SELECT i + 1 FROM n WHERE i < 899999)
                 INSERT INTO messages (chat, id, sender, from_me, timestamp, content)
                 SELECT (i % 2000) || '@s.whatsapp.net', 'h' || i, (i % 2000) || '@s.whatsapp.net',
                    i % 3 = 0, i, ?1 FROM n",
                params![text],
            )
            .unwrap();
        connection
            .execute(
                "WITH RECURSIVE n(i) AS (SELECT 0 UNION ALL SELECT i + 1 FROM n WHERE i < 79999)
                 INSERT INTO messages (chat, id, sender, from_me, timestamp, content)
                 SELECT (900 + i % 200) || '@broadcast', 'o' || i, 'me@s.whatsapp.net', 1, i, ?1 FROM n",
                params![text],
            )
            .unwrap();
        // A contact's list of 60,000, 5,000 of them in the chat already,
        // and one of 40,000 from 20 contacts.
        connection
            .execute(
                "WITH RECURSIVE n(i) AS (SELECT 0 UNION ALL SELECT i + 1 FROM n WHERE i < 59999)
                 INSERT INTO messages (chat, id, sender, from_me, timestamp, content)
                 SELECT '111@broadcast', 'a' || i, '7@s.whatsapp.net', 0, 1000000 + i, ?1 FROM n",
                params![text],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO messages (chat, id, sender, from_me, timestamp, content)
                 SELECT '7@s.whatsapp.net', id, sender, 0, timestamp, content FROM messages
                 WHERE chat = '111@broadcast' AND CAST(substr(id, 2) AS INTEGER) < 5000",
                [],
            )
            .unwrap();
        connection
            .execute(
                "WITH RECURSIVE n(i) AS (SELECT 0 UNION ALL SELECT i + 1 FROM n WHERE i < 39999)
                 INSERT INTO messages (chat, id, sender, from_me, timestamp, content)
                 SELECT '222@broadcast', 'b' || i, (5000 + i % 20) || '@s.whatsapp.net', 0,
                    2000000 + i, ?1 FROM n",
                params![text],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO chats (id, name, kind) SELECT DISTINCT chat, 'fixture', 'direct' FROM messages",
                [],
            )
            .unwrap();
        connection.execute_batch("COMMIT").unwrap();

        let started = std::time::Instant::now();
        let lists = archive.received_broadcast_lists().unwrap();
        let scan = started.elapsed();
        let mut moves = Vec::new();
        for (list, senders) in &lists {
            let names: Vec<_> = senders
                .iter()
                .map(|sender| (sender.clone(), sender.clone(), "fixture".to_owned()))
                .collect();
            let started = std::time::Instant::now();
            let adopted = archive.adopt_broadcast_list(list, &names).unwrap();
            moves.push((senders.len(), adopted, started.elapsed()));
        }
        let started = std::time::Instant::now();
        let again = archive.received_broadcast_lists().unwrap();
        eprintln!(
            "scan {scan:?} for {} lists; moves {moves:?}; next start {:?} for {} lists",
            lists.len(),
            started.elapsed(),
            again.len()
        );
    }
}
