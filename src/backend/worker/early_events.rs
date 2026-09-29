//! The phone's own read receipts and reactions that name a message not
//! archived yet. During the offline drain, messages wait in the library's
//! commit batch while receipts are dispatched at once, and a message can also
//! reach us later through history sync. These events wait here, in memory,
//! until their message is filed, and are dropped after [`MAX_AGE`] or beyond
//! [`MAX_WAITING`] of a kind.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use super::HistoryReactionBody;

/// How long an event may wait for its message.
const MAX_AGE: Duration = Duration::from_secs(60 * 60);
/// How many events of a kind may wait; the oldest go first.
const MAX_WAITING: usize = 512;

struct WaitingRead {
    chat: String,
    id: String,
    queued: Instant,
}

/// A reaction, or its removal, whose target has not been filed yet.
pub(super) struct WaitingReaction {
    pub(super) chat: String,
    pub(super) target: String,
    pub(super) sender: String,
    pub(super) from_me: bool,
    pub(super) body: HistoryReactionBody,
    /// When the reactor sent it, in milliseconds; the newest per sender wins.
    pub(super) sent_at: i64,
    queued: Instant,
}

impl WaitingReaction {
    pub(super) fn new(
        chat: &str,
        target: &str,
        sender: &str,
        from_me: bool,
        body: HistoryReactionBody,
        sent_at: i64,
    ) -> Self {
        Self {
            chat: chat.to_owned(),
            target: target.to_owned(),
            sender: sender.to_owned(),
            from_me,
            body,
            sent_at,
            queued: Instant::now(),
        }
    }
}

#[derive(Default)]
pub(super) struct EarlyEvents {
    reads: VecDeque<WaitingRead>,
    reactions: VecDeque<WaitingReaction>,
}

impl EarlyEvents {
    /// Keeps the phone's read of a message we do not have yet.
    pub(super) fn wait_read(&mut self, chat: &str, id: &str) {
        self.prune(Instant::now());
        if self
            .reads
            .iter()
            .any(|read| read.chat == chat && read.id == id)
        {
            return;
        }
        if self.reads.len() >= MAX_WAITING {
            self.reads.pop_front();
        }
        self.reads.push_back(WaitingRead {
            chat: chat.to_owned(),
            id: id.to_owned(),
            queued: Instant::now(),
        });
    }

    /// Keeps a reaction for a message we do not have yet. A sender's newer
    /// reaction replaces an older one; an older one arriving late is ignored.
    pub(super) fn wait_reaction(&mut self, reaction: WaitingReaction) {
        self.prune(Instant::now());
        if let Some(waiting) = self.reactions.iter_mut().find(|waiting| {
            waiting.chat == reaction.chat
                && waiting.target == reaction.target
                && waiting.sender == reaction.sender
        }) {
            if waiting.sent_at <= reaction.sent_at {
                *waiting = reaction;
            }
            return;
        }
        if self.reactions.len() >= MAX_WAITING {
            self.reactions.pop_front();
        }
        self.reactions.push_back(reaction);
    }

    /// Whether the phone read this message before we had it.
    pub(super) fn take_read(&mut self, chat: &str, id: &str) -> bool {
        let before = self.reads.len();
        self.reads.retain(|read| read.chat != chat || read.id != id);
        self.reads.len() != before
    }

    /// The reactions waiting for this message, oldest first.
    pub(super) fn take_reactions(&mut self, chat: &str, id: &str) -> Vec<WaitingReaction> {
        let mut taken = Vec::new();
        let mut kept = VecDeque::with_capacity(self.reactions.len());
        for reaction in self.reactions.drain(..) {
            if reaction.chat == chat && reaction.target == id {
                taken.push(reaction);
            } else {
                kept.push_back(reaction);
            }
        }
        self.reactions = kept;
        taken.sort_by_key(|reaction| reaction.sent_at);
        taken
    }

    /// Files events kept under a privacy id under the id it now maps to, and
    /// returns the messages they wait for in that chat.
    pub(super) fn rekey(&mut self, from: &str, to: &str) -> Vec<String> {
        if from == to {
            return Vec::new();
        }
        for read in &mut self.reads {
            if read.chat == from {
                read.chat = to.to_owned();
            }
        }
        for reaction in &mut self.reactions {
            if reaction.chat == from {
                reaction.chat = to.to_owned();
            }
            if reaction.sender == from {
                reaction.sender = to.to_owned();
            }
        }
        let mut ids: Vec<String> = self
            .reads
            .iter()
            .filter(|read| read.chat == to)
            .map(|read| read.id.clone())
            .chain(
                self.reactions
                    .iter()
                    .filter(|reaction| reaction.chat == to)
                    .map(|reaction| reaction.target.clone()),
            )
            .collect();
        ids.sort();
        ids.dedup();
        ids
    }

    /// Drops events whose message never came.
    pub(super) fn prune(&mut self, now: Instant) {
        let fresh = |queued: Instant| now.saturating_duration_since(queued) < MAX_AGE;
        self.reads.retain(|read| fresh(read.queued));
        self.reactions.retain(|reaction| fresh(reaction.queued));
    }

    pub(super) fn clear(&mut self) {
        self.reads.clear();
        self.reactions.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(sender: &str, emoji: &str, sent_at: i64) -> WaitingReaction {
        WaitingReaction::new(
            "chat",
            "m",
            sender,
            false,
            HistoryReactionBody::Plain(emoji.into()),
            sent_at,
        )
    }

    fn emoji(reaction: &WaitingReaction) -> &str {
        match &reaction.body {
            HistoryReactionBody::Plain(emoji) => emoji,
            HistoryReactionBody::Encrypted { .. } => panic!("plain expected"),
        }
    }

    #[test]
    fn the_newest_reaction_per_sender_waits() {
        let mut early = EarlyEvents::default();
        early.wait_reaction(plain("a", "👍", 200));
        early.wait_reaction(plain("a", "❤️", 100));
        early.wait_reaction(plain("b", "😂", 150));
        early.wait_reaction(plain("b", "", 300));
        let taken = early.take_reactions("chat", "m");
        assert_eq!(taken.len(), 2);
        assert_eq!(
            emoji(&taken[0]),
            "👍",
            "an older reaction arriving late loses"
        );
        assert_eq!(
            emoji(&taken[1]),
            "",
            "a newer removal replaces the reaction"
        );
        assert!(early.take_reactions("chat", "m").is_empty());
    }

    #[test]
    fn waiting_is_bounded_by_count_and_age() {
        let mut early = EarlyEvents::default();
        for n in 0..=MAX_WAITING {
            early.wait_read("chat", &n.to_string());
        }
        assert!(!early.take_read("chat", "0"), "the oldest went first");
        assert!(early.take_read("chat", &MAX_WAITING.to_string()));
        early.wait_reaction(plain("a", "👍", 1));
        early.prune(Instant::now() + MAX_AGE);
        assert!(!early.take_read("chat", "1"));
        assert!(early.take_reactions("chat", "m").is_empty());
    }

    #[test]
    fn a_privacy_id_moves_to_its_phone_number() {
        let mut early = EarlyEvents::default();
        early.wait_read("1@lid", "r");
        early.wait_reaction(WaitingReaction::new(
            "1@lid",
            "m",
            "1@lid",
            false,
            HistoryReactionBody::Plain("👍".into()),
            1,
        ));
        assert_eq!(
            early.rekey("1@lid", "2@s.whatsapp.net"),
            ["m".to_owned(), "r".to_owned()]
        );
        assert!(early.take_read("2@s.whatsapp.net", "r"));
        let taken = early.take_reactions("2@s.whatsapp.net", "m");
        assert_eq!(taken[0].sender, "2@s.whatsapp.net");
    }
}
