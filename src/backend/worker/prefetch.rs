//! Serial background phone-history and attachment prefetch.

use crate::model::{Chat, ChatId};
use crate::settings::HistoryPrefetch;
use std::collections::HashSet;
use std::time::{Duration, Instant};

pub const HISTORY_GAP: Duration = Duration::from_secs(20);
pub const MEDIA_GAP: Duration = Duration::from_secs(3);
pub const MEDIA_MAX: u64 = 64 * 1024 * 1024;
const RECENT_LIMIT: usize = 10;

pub(super) struct State {
    pub mode: HistoryPrefetch,
    pub focused: Option<ChatId>,
    /// Whether attachments download on their own. The background fetches a
    /// file only when the reader asked for that; with it off it still fetches
    /// history, but not a single attachment.
    pub auto_download: bool,
    exhausted: HashSet<ChatId>,
    history_due: Option<Instant>,
    history_failures: u32,
    history_chat: Option<ChatId>,
    media_due: Option<Instant>,
    /// The attachment being fetched, with the card that addresses it: a
    /// carousel card is a file of its own, so the message id alone would treat
    /// one card's completion as another's.
    media: Option<(ChatId, String, Option<usize>)>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            mode: HistoryPrefetch::Off,
            focused: None,
            auto_download: false,
            exhausted: HashSet::new(),
            history_due: None,
            history_failures: 0,
            history_chat: None,
            media_due: None,
            media: None,
        }
    }
}

impl State {
    pub fn configure(
        &mut self,
        mode: HistoryPrefetch,
        focused: Option<ChatId>,
        auto_download: bool,
    ) {
        self.mode = mode;
        self.focused = focused;
        self.auto_download = auto_download;
        if mode == HistoryPrefetch::Off {
            self.history_chat = None;
            // The attachment already downloading keeps its record. Turning the
            // mode off stops the pump from starting another one, but it cannot
            // cancel the task in flight, and `finish_media` has to still
            // recognize that completion as the background's: dropping the
            // record made it read as the reader's own, so a failure reopened
            // the thirty-day retry window that a manual click opens, for a
            // download nobody asked for.
        }
    }

    /// After reconnect, phone history may be available again.
    pub fn on_connected(&mut self) {
        self.exhausted.clear();
        self.history_failures = 0;
        self.history_due = None;
        self.history_chat = None;
    }

    pub fn reset_session(&mut self) {
        let mode = self.mode;
        let focused = self.focused.clone();
        let auto_download = self.auto_download;
        *self = Self::default();
        self.mode = mode;
        self.focused = focused;
        // The setting is the reader's, not the session's: a relink must not
        // silently stop fetching files for a mode that is still configured.
        self.auto_download = auto_download;
    }

    pub fn next_history(
        &self,
        now: Instant,
        phone_busy: bool,
        targets: &[ChatId],
    ) -> Option<ChatId> {
        if self.mode == HistoryPrefetch::Off || phone_busy || self.history_chat.is_some() {
            return None;
        }
        if self.history_due.is_some_and(|due| now < due) {
            return None;
        }
        targets
            .iter()
            .find(|chat| !self.exhausted.contains(*chat))
            .cloned()
    }

    pub fn start_history(&mut self, chat: ChatId) {
        self.history_chat = Some(chat);
    }

    /// True when this completion belongs to the prefetch request.
    pub fn finish_history(&mut self, chat: &str, more: bool, now: Instant) -> bool {
        if self.history_chat.as_deref() != Some(chat) {
            return false;
        }
        self.history_chat = None;
        self.history_failures = 0;
        self.history_due = Some(now + HISTORY_GAP);
        if !more {
            self.exhausted.insert(chat.to_owned());
        }
        true
    }

    pub fn fail_history(&mut self, chat: &str, now: Instant) -> bool {
        if self.history_chat.as_deref() != Some(chat) {
            return false;
        }
        self.history_chat = None;
        self.history_failures = self.history_failures.saturating_add(1);
        let shift = (self.history_failures - 1).min(5);
        let delay = (30 * (1_u64 << shift)).min(900);
        self.history_due = Some(now + Duration::from_secs(delay));
        true
    }

    /// Forgets a chat that is gone or emptied. It holds neither the history
    /// turn nor a slot in the pump, so a request for it that can never be
    /// answered does not keep the queue waiting until the next reconnect.
    pub fn forget_chat(&mut self, chat: &str) {
        if self.history_chat.as_deref() == Some(chat) {
            self.history_chat = None;
        }
        if self
            .media
            .as_ref()
            .is_some_and(|(active, _, _)| active == chat)
        {
            self.media = None;
        }
        self.exhausted.remove(chat);
    }

    pub fn next_media_ready(&self, now: Instant) -> bool {
        self.mode != HistoryPrefetch::Off
            && self.auto_download
            && self.media.is_none()
            && self.media_due.is_none_or(|due| now >= due)
    }

    pub fn skip_media(&self, chat: &str, id: &str, card: Option<usize>) -> bool {
        self.media
            .as_ref()
            .is_some_and(|(active_chat, active_id, active_card)| {
                active_chat == chat && active_id == id && *active_card == card
            })
    }

    pub fn start_media(&mut self, chat: ChatId, id: String, card: Option<usize>) {
        self.media = Some((chat, id, card));
    }

    /// True when this completion belongs to the prefetch download.
    pub fn finish_media(
        &mut self,
        chat: &str,
        id: &str,
        card: Option<usize>,
        now: Instant,
    ) -> bool {
        if self
            .media
            .as_ref()
            .is_none_or(|(active_chat, active_id, active_card)| {
                active_chat != chat || active_id != id || *active_card != card
            })
        {
            return false;
        }
        self.media = None;
        self.media_due = Some(now + MEDIA_GAP);
        true
    }
}

/// Open chat first when it belongs in the mode, then pinned (top first), then
/// the ten most recently active unpinned chats that are not archived.
pub(super) fn targets(chats: &[Chat], mode: HistoryPrefetch, focused: Option<&str>) -> Vec<ChatId> {
    match mode {
        HistoryPrefetch::Off => Vec::new(),
        HistoryPrefetch::Focused => {
            // The chat that was open can be gone: it was deleted while it was
            // focused, and the mode must not keep asking for it.
            let mut out = Vec::new();
            if let Some(id) = focused
                && chats.iter().any(|chat| chat.id == id)
            {
                out.push(id.to_owned());
            }
            out
        }
        HistoryPrefetch::RecentAndPinned => {
            let mut out = Vec::new();
            if let Some(id) = focused
                && chats.iter().any(|chat| chat.id == id)
            {
                out.push(id.to_owned());
            }
            let mut pinned: Vec<&Chat> = chats.iter().filter(|chat| chat.pinned).collect();
            pinned.sort_by_key(|chat| std::cmp::Reverse(chat.pinned_at));
            for chat in pinned {
                push_unique(&mut out, &chat.id);
            }
            let mut recent: Vec<&Chat> = chats
                .iter()
                .filter(|chat| !chat.pinned && !chat.archived)
                .collect();
            recent.sort_by_key(|chat| std::cmp::Reverse(chat.last_activity));
            for chat in recent.into_iter().take(RECENT_LIMIT) {
                push_unique(&mut out, &chat.id);
            }
            out
        }
    }
}

fn push_unique(out: &mut Vec<ChatId>, id: &str) {
    if !out.iter().any(|known| known == id) {
        out.push(id.to_owned());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chat(id: &str, last_activity: i64, pinned: bool, pinned_at: i64, archived: bool) -> Chat {
        let mut chat = Chat::new(id.into(), id.into());
        chat.last_activity = last_activity;
        chat.pinned = pinned;
        chat.pinned_at = pinned_at;
        chat.archived = archived;
        chat
    }

    /// Turning the background off stops the pump, but it cannot cancel the
    /// attachment already downloading. Its record survives, so the completion
    /// is still read as the background's and a failure keeps its own backoff
    /// instead of reopening the window a manual click opens.
    #[test]
    fn turning_the_mode_off_keeps_the_download_that_is_running() {
        let mut state = State::default();
        let now = Instant::now();
        state.configure(HistoryPrefetch::RecentAndPinned, Some("a".into()), true);
        state.start_media("a".into(), "m1".into(), None);
        state.configure(HistoryPrefetch::Off, Some("a".into()), true);
        assert!(!state.next_media_ready(now), "the mode is off");
        assert!(
            state.finish_media("a", "m1", None, now),
            "the download the background started is still the background's"
        );
    }

    #[test]
    fn targets_put_focused_then_pinned_then_recent() {
        let chats = vec![
            chat("pin-low", 1, true, 1, false),
            chat("pin-high", 2, true, 9, false),
            chat("old", 10, false, 0, false),
            chat("focus", 50, false, 0, false),
            chat("archived", 90, false, 0, true),
            chat("r1", 40, false, 0, false),
        ];
        assert!(targets(&chats, HistoryPrefetch::Off, Some("focus")).is_empty());
        assert_eq!(
            targets(&chats, HistoryPrefetch::Focused, Some("focus")),
            vec!["focus".to_owned()]
        );
        assert_eq!(
            targets(&chats, HistoryPrefetch::RecentAndPinned, Some("focus")),
            vec![
                "focus".to_owned(),
                "pin-high".to_owned(),
                "pin-low".to_owned(),
                "r1".to_owned(),
                "old".to_owned(),
            ]
        );
    }

    #[test]
    fn recent_mode_keeps_every_pin_and_ten_unpinned() {
        let mut chats = vec![chat("pin", 1, true, 1, false)];
        for index in 0..12 {
            chats.push(chat(&format!("r{index}"), 100 - index, false, 0, false));
        }
        let ids = targets(&chats, HistoryPrefetch::RecentAndPinned, None);
        assert_eq!(ids[0], "pin");
        assert_eq!(ids.len(), 11);
        assert!(!ids.iter().any(|id| id == "r10" || id == "r11"));
    }

    #[test]
    fn history_waits_for_user_and_backs_off_on_failure() {
        let mut state = State::default();
        state.configure(HistoryPrefetch::Focused, Some("a".into()), false);
        let now = Instant::now();
        let targets = vec!["a".into(), "b".into()];
        assert!(state.next_history(now, true, &targets).is_none());
        assert_eq!(
            state.next_history(now, false, &targets).as_deref(),
            Some("a")
        );
        state.start_history("a".into());
        assert!(state.next_history(now, false, &targets).is_none());
        assert!(state.fail_history("a", now));
        assert!(state.next_history(now, false, &targets).is_none());
        assert_eq!(
            state
                .next_history(now + Duration::from_secs(30), false, &targets)
                .as_deref(),
            Some("a")
        );
        state.start_history("a".into());
        assert!(state.finish_history("a", false, now + Duration::from_secs(30)));
        assert!(
            state
                .next_history(now + Duration::from_secs(31), false, &targets)
                .is_none()
        );
        assert_eq!(
            state
                .next_history(now + Duration::from_secs(51), false, &targets)
                .as_deref(),
            Some("b")
        );
    }

    #[test]
    fn a_foreign_history_ack_does_not_advance_the_queue() {
        let mut state = State::default();
        state.configure(HistoryPrefetch::Focused, Some("a".into()), false);
        state.start_history("a".into());
        assert!(!state.finish_history("other", true, Instant::now()));
        assert!(
            state
                .next_history(Instant::now(), false, &["a".into()])
                .is_none()
        );
    }

    #[test]
    fn a_failed_prefetch_file_is_not_skipped_forever() {
        let mut state = State::default();
        state.configure(HistoryPrefetch::Focused, Some("a".into()), true);
        let now = Instant::now();
        state.start_media("a".into(), "m1".into(), None);
        assert!(state.skip_media("a", "m1", None));
        assert!(state.finish_media("a", "m1", None, now));
        assert!(!state.skip_media("a", "m1", None));
        assert!(!state.next_media_ready(now));
        assert!(state.next_media_ready(now + MEDIA_GAP));
        assert!(!state.finish_media("a", "m1", None, now + MEDIA_GAP));
    }

    /// A file downloads on its own only when the reader asked for that. With
    /// "Download attachments automatically" off, the background may still fetch
    /// history, but it must not fetch a single attachment.
    #[test]
    fn media_prefetch_follows_the_auto_download_setting() {
        let mut state = State::default();
        let now = Instant::now();
        state.configure(HistoryPrefetch::Focused, Some("a".into()), false);
        assert!(!state.next_media_ready(now));
        state.configure(HistoryPrefetch::Focused, Some("a".into()), true);
        assert!(state.next_media_ready(now));
        // And history is unaffected by the setting.
        assert_eq!(
            state.next_history(now, false, &["a".into()]).as_deref(),
            Some("a")
        );
    }

    /// A relink is not a settings change. `reset_session` keeps the mode, so it
    /// has to keep the reader's automatic-download choice with it: otherwise a
    /// mode that is still configured quietly stops fetching files after a
    /// logout and nothing syncs the switch back.
    #[test]
    fn a_relink_keeps_the_auto_download_choice() {
        let mut state = State::default();
        state.configure(HistoryPrefetch::RecentAndPinned, Some("a".into()), true);
        state.reset_session();
        assert_eq!(state.mode, HistoryPrefetch::RecentAndPinned);
        assert_eq!(state.focused.as_deref(), Some("a"));
        assert!(state.auto_download);
        assert!(
            state.next_media_ready(Instant::now()),
            "the configured mode still fetches files"
        );
    }

    /// A carousel card is a file of its own: a manual download of one card
    /// must not read as the completion of the prefetch of another, or the
    /// prefetch's own completion would be ignored and the slot freed while it
    /// is still in flight.
    #[test]
    fn another_cards_file_does_not_release_the_prefetch_slot() {
        let mut state = State::default();
        state.configure(HistoryPrefetch::Focused, Some("a".into()), true);
        let now = Instant::now();
        state.start_media("a".into(), "m1".into(), Some(0));
        assert!(state.skip_media("a", "m1", Some(0)));
        assert!(!state.skip_media("a", "m1", Some(1)));
        assert!(!state.finish_media("a", "m1", Some(1), now));
        assert!(
            state.skip_media("a", "m1", Some(0)),
            "the prefetch of card 0 is still in flight"
        );
        assert!(state.finish_media("a", "m1", Some(0), now));
        assert!(!state.skip_media("a", "m1", Some(0)));
    }

    /// The chat that was open can be gone: a deleted chat must not keep the
    /// pump asking the phone for history nobody can read.
    #[test]
    fn a_focused_chat_that_is_gone_is_not_asked_for() {
        let chats = vec![chat("a", 1, false, 0, false)];
        assert_eq!(
            targets(&chats, HistoryPrefetch::Focused, Some("gone")),
            Vec::<ChatId>::new()
        );
        assert_eq!(
            targets(&chats, HistoryPrefetch::Focused, Some("a")),
            vec!["a".to_owned()]
        );
    }
}
