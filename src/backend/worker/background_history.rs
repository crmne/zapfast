//! Bounded account-local history recovery and ownership of unsolicited pages.
//!
//! Match the phone's optional PDO session id to the library's returned id.
//! Keep a conservative chat owner for responses that omit that id.

use crate::model::ChatId;
use std::{
    collections::{HashMap, HashSet},
    time::{Duration, Instant},
};

const PACE: Duration = Duration::from_secs(20);

#[derive(Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
struct Checkpoint {
    cursor: usize,
    // Legacy accounting is kept for migration to the shared automatic budget.
    day: i64,
    requests: u32,
    next_request: i64,
    retries: HashMap<ChatId, (u8, i64)>,
}

#[derive(Default)]
pub(super) struct Recovery {
    pub full: bool,
    pub daily_budget: u32,
    checkpoint: Checkpoint,
    pub enabled: bool,
    pub paused: bool,
    pub focused: Option<ChatId>,
    due: Option<Instant>,
    cursor: usize,
    /// Automatic requests, including timed-out ones still owned by the phone.
    silent: HashSet<ChatId>,
    polls: HashSet<ChatId>,
    started: HashMap<ChatId, (Instant, Option<i64>)>,
    sessions: HashMap<String, (ChatId, Instant, Option<i64>)>,
    early: HashMap<String, (super::FiledHistory, bool)>,
}

impl Recovery {
    pub fn restore(&mut self, value: &str) -> Result<(), serde_json::Error> {
        self.checkpoint = serde_json::from_str(value)?;
        self.cursor = self.checkpoint.cursor;
        Ok(())
    }
    #[cfg(test)]
    pub fn checkpoint(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(&self.checkpoint)
    }
    pub fn canonicalize(&mut self, old: &str, new: &str) {
        if let Some(retry) = self.checkpoint.retries.remove(old) {
            let target = self.checkpoint.retries.entry(new.to_owned()).or_default();
            target.0 = target.0.max(retry.0);
            target.1 = target.1.max(retry.1);
        }
        if self.silent.remove(old) {
            self.silent.insert(new.to_owned());
        }
        if self.polls.remove(old) {
            self.polls.insert(new.to_owned());
        }
        if let Some(started) = self.started.remove(old) {
            let target = self.started.entry(new.to_owned()).or_insert(started);
            *target = (*target).max(started);
        }
        for (chat, _, _) in self.sessions.values_mut() {
            if chat == old {
                *chat = new.to_owned();
            }
        }
        for (filed, _) in self.early.values_mut() {
            for (chat, _, _, _) in filed {
                if chat == old {
                    *chat = new.to_owned();
                }
            }
        }
        if self.focused.as_deref() == Some(old) {
            self.focused = Some(new.to_owned());
        }
    }
    pub fn started(&mut self, chat: &str, requested: Instant, import_token: Option<i64>) {
        self.started
            .insert(chat.to_owned(), (requested, import_token));
    }
    pub fn session(&self, id: &str) -> Option<&(ChatId, Instant, Option<i64>)> {
        self.sessions.get(id)
    }
    pub fn sent(
        &mut self,
        chat: ChatId,
        requested: Instant,
        import_token: Option<i64>,
        id: String,
    ) -> Option<(super::FiledHistory, bool)> {
        // Dropping old identities is safe: unrecognized pages never own UI.
        if self.sessions.len() >= 128
            && let Some(oldest) = self
                .sessions
                .iter()
                .min_by_key(|(_, (_, at, _))| *at)
                .map(|(id, _)| id.clone())
        {
            self.sessions.remove(&oldest);
        }
        let early = self.early.remove(&id);
        self.sessions.insert(id, (chat, requested, import_token));
        early
    }
    pub fn early(&mut self, id: String, filed: super::FiledHistory, durable: bool) {
        if self.early.len() < 8 {
            self.early.insert(id, (filed, durable));
        }
    }
    pub fn owns_attempt(&self, chat: &str, requested: Instant) -> bool {
        self.started.get(chat).map(|(at, _)| *at) == Some(requested)
    }
    pub fn attempted(&self, chat: &str) -> bool {
        self.started.contains_key(chat)
    }
    pub fn correlated(&self, chat: &str) -> bool {
        self.started.get(chat).is_some_and(|(at, token)| {
            self.sessions
                .values()
                .any(|(owner, started, session_token)| {
                    owner == chat && started == at && session_token == token
                })
        })
    }
    pub fn owns(&self, chat: &str) -> bool {
        self.silent.contains(chat)
    }
    pub fn release_unsent(&mut self, chat: &str) {
        self.silent.remove(chat);
        self.due = None;
    }
    pub fn claim(&mut self, chat: ChatId) {
        self.silent.insert(chat);
    }
    pub fn claim_poll(&mut self, chat: ChatId) {
        self.polls.insert(chat.clone());
        self.claim(chat);
    }
    pub fn poll_owned(&self, chat: &str) -> bool {
        self.polls.contains(chat)
    }
    pub fn stop_poll(&mut self, chat: &str) {
        self.polls.remove(chat);
        // Known session identities still identify late poll replies. An
        // uncorrelated owner remains held until reconnect, as for older pages.
        if self.correlated(chat) {
            self.silent.remove(chat);
        }
    }
    pub fn answered(&mut self, chat: &str, progress: bool) -> bool {
        self.polls.remove(chat);
        let owned = self.silent.remove(chat);
        if owned && !progress {
            self.stop(chat);
        } else if owned {
            self.checkpoint.retries.remove(chat);
        }
        owned
    }
    pub fn stop(&mut self, chat: &str) {
        let retry = self.checkpoint.retries.entry(chat.to_owned()).or_default();
        retry.0 = retry.0.saturating_add(1).min(10);
        retry.1 = crate::util::now() + (60_i64 * (1 << retry.0)).min(3600);
        // An exact old session identity still protects late responses.
        // Untagged sends retain their owner until reconnect.
        if self.correlated(chat) {
            self.silent.remove(chat);
        }
    }
    /// Reconnect releases uncorrelated leases but never renews session budgets.
    pub fn reconnect(&mut self) {
        self.silent.clear();
        self.polls.clear();
        self.started.clear();
        self.early.clear();
    }
    pub fn next(&mut self, candidates: &[ChatId], now: Instant) -> Option<ChatId> {
        let timestamp = crate::util::now();
        if !self.enabled || self.paused || self.due.is_some_and(|due| now < due) {
            return None;
        }
        let len = candidates.len();
        for step in 0..len {
            let index = (self.cursor + step) % len;
            let chat = &candidates[index];
            if self.owns(chat)
                || self
                    .checkpoint
                    .retries
                    .get(chat)
                    .is_some_and(|(_, due)| timestamp < *due)
            {
                continue;
            }
            self.cursor = (index + 1) % len;
            self.checkpoint.cursor = self.cursor;
            self.due = Some(now + PACE);
            self.claim(chat.clone());
            return Some(chat.clone());
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_withheld_request_does_not_delay_other_chats_or_claim_a_phone_response() {
        let now = Instant::now();
        let chats = vec!["stopped".into(), "recovering".into()];
        let mut recovery = Recovery {
            enabled: true,
            ..Default::default()
        };
        assert_eq!(recovery.next(&chats, now).as_deref(), Some("stopped"));
        recovery.release_unsent("stopped");
        assert!(!recovery.owns("stopped"));
        assert_eq!(recovery.next(&chats, now).as_deref(), Some("recovering"));
    }

    #[test]
    fn a_stopped_poll_releases_known_ownership_but_retains_late_reply_identity() {
        let now = Instant::now();
        let mut recovery = Recovery::default();
        recovery.claim_poll("chat".into());
        recovery.started("chat", now, None);
        recovery.sent("chat".into(), now, None, "poll-session".into());
        recovery.stop_poll("chat");
        assert!(!recovery.owns("chat"));
        assert!(!recovery.poll_owned("chat"));
        assert_eq!(
            recovery.session("poll-session"),
            Some(&("chat".into(), now, None))
        );
        recovery.claim_poll("uncorrelated".into());
        recovery.stop_poll("uncorrelated");
        assert!(recovery.owns("uncorrelated"));
        assert!(!recovery.poll_owned("uncorrelated"));
    }

    #[test]
    fn canonical_identity_merge_keeps_the_stronger_persisted_backoff() {
        let mut recovery = Recovery::default();
        recovery
            .checkpoint
            .retries
            .insert("old@lid".into(), (4, 900));
        recovery
            .checkpoint
            .retries
            .insert("new@s.whatsapp.net".into(), (1, 100));
        recovery.canonicalize("old@lid", "new@s.whatsapp.net");
        assert_eq!(
            recovery.checkpoint.retries.get("new@s.whatsapp.net"),
            Some(&(4, 900))
        );
        assert!(!recovery.checkpoint.retries.contains_key("old@lid"));
        let saved = recovery.checkpoint().unwrap();
        let mut restarted = Recovery::default();
        restarted.restore(&saved).unwrap();
        assert_eq!(
            restarted.checkpoint.retries.get("new@s.whatsapp.net"),
            Some(&(4, 900))
        );
    }
    #[test]
    fn full_mode_rotates_beyond_ten_chats_with_shared_daily_budget() {
        let mut recovery = Recovery {
            enabled: true,
            full: true,
            daily_budget: 60,
            ..Default::default()
        };
        let candidates: Vec<_> = (0..12).map(|index| format!("fixture-{index}")).collect();
        let now = Instant::now();
        let mut budget = super::super::automatic_history::Budget::default();
        for attempt in 0..60 {
            let chat = recovery.next(&candidates, now + PACE * attempt).unwrap();
            assert_eq!(chat, candidates[attempt as usize % candidates.len()]);
            assert_eq!(
                budget.reserve(
                    &chat,
                    None,
                    (100, format!("boundary-{attempt}")),
                    true,
                    60,
                    100 + i64::from(attempt) * 20
                ),
                super::super::automatic_history::Admission::Allowed
            );
            recovery.answered(&chat, true);
        }
        let budget =
            super::super::automatic_history::Budget::restore(&budget.checkpoint().unwrap())
                .unwrap();
        assert!(!budget.ready(true, 60, 2000));
        let saved = recovery.checkpoint().unwrap();
        let mut restarted = Recovery {
            enabled: true,
            full: true,
            daily_budget: 60,
            ..Default::default()
        };
        restarted.restore(&saved).unwrap();
        assert_eq!(restarted.cursor, recovery.cursor);
    }

    #[test]
    fn full_mode_no_progress_backoff_and_pacing_survive_restart() {
        let mut recovery = Recovery {
            enabled: true,
            full: true,
            daily_budget: 100,
            ..Default::default()
        };
        let candidates = vec!["fixture-chat".into()];
        let now = Instant::now();
        let chat = recovery.next(&candidates, now).unwrap();
        recovery.answered(&chat, false);
        let saved = recovery.checkpoint().unwrap();
        let mut restarted = Recovery {
            enabled: true,
            full: true,
            daily_budget: 100,
            ..Default::default()
        };
        restarted.restore(&saved).unwrap();
        assert!(restarted.next(&candidates, now + PACE * 10).is_none());
        restarted.checkpoint.retries.get_mut(&chat).unwrap().1 = 0;
        assert_eq!(
            restarted.next(&candidates, now + PACE * 11).as_deref(),
            Some(chat.as_str())
        );
    }

    #[test]
    fn late_privacy_mapping_preserves_correlated_ownership() {
        let now = Instant::now();
        let mut recovery = Recovery {
            enabled: true,
            ..Default::default()
        };
        assert_eq!(
            recovery.next(&["9@lid".into()], now).as_deref(),
            Some("9@lid")
        );
        recovery.started("9@lid", now, Some(7));
        recovery.sent("9@lid".into(), now, Some(7), "session".into());
        recovery.canonicalize("9@lid", "1@s.whatsapp.net");
        assert_eq!(
            recovery.session("session"),
            Some(&("1@s.whatsapp.net".into(), now, Some(7)))
        );
        assert!(recovery.owns("1@s.whatsapp.net"));
        assert!(!recovery.owns("9@lid"));
        assert!(recovery.answered("1@s.whatsapp.net", true));
    }
    #[test]
    fn changing_chat_candidates_cannot_exceed_the_account_session_budget() {
        let mut recovery = Recovery {
            enabled: true,
            ..Default::default()
        };
        let now = Instant::now();
        let mut budget = super::super::automatic_history::Budget::default();
        for attempt in 0..50 {
            let candidate = vec![format!("chat-{attempt}")];
            let chat = recovery.next(&candidate, now + PACE * attempt).unwrap();
            assert_eq!(
                budget.reserve(
                    &chat,
                    None,
                    (100, "boundary".into()),
                    false,
                    1000,
                    100 + i64::from(attempt) * 20
                ),
                super::super::automatic_history::Admission::Allowed
            );
            recovery.answered(&chat, true);
        }
        recovery.reconnect();
        assert!(!budget.ready(false, 1000, 2000));
    }
    #[test]
    fn early_response_registration_is_bounded_and_correlated_to_its_attempt() {
        let now = Instant::now();
        let mut recovery = Recovery::default();
        let page = vec![(
            "chat".into(),
            1,
            Some(true),
            Some((10, "old".into(), false)),
        )];
        recovery.early("early".into(), page.clone(), true);
        assert_eq!(
            recovery.sent("chat".into(), now, Some(8), "early".into()),
            Some((page, true))
        );
        recovery.claim("chat".into());
        recovery.started("chat", now, Some(8));
        assert!(recovery.correlated("chat"));
        assert!(recovery.owns_attempt("chat", now));
        assert!(!recovery.owns_attempt("chat", now + PACE));
        recovery.reconnect();
        assert!(!recovery.owns_attempt("chat", now));
        assert_eq!(
            recovery.session("early"),
            Some(&("chat".into(), now, Some(8)))
        );
        for index in 0..20 {
            recovery.early(index.to_string(), Vec::new(), true);
        }
        assert_eq!(recovery.early.len(), 8);
    }
    #[test]
    fn opt_in_pacing_and_fairness_survive_reconnect_and_cancellation() {
        let now = Instant::now();
        let chats = vec!["a".into(), "b".into()];
        let mut recovery = Recovery::default();
        assert!(recovery.next(&chats, now).is_none());
        recovery.enabled = true;
        assert_eq!(recovery.next(&chats, now).as_deref(), Some("a"));
        recovery.answered("a", true);
        assert!(
            recovery
                .next(&chats, now + Duration::from_secs(19))
                .is_none()
        );
        assert_eq!(recovery.next(&chats, now + PACE).as_deref(), Some("b"));
        recovery.enabled = false;
        assert!(
            recovery.owns("b"),
            "cancel keeps ownership of transmitted requests"
        );
        assert!(recovery.next(&chats, now + PACE * 2).is_none());
        recovery.enabled = true;
        recovery.reconnect();
        for attempt in 2..10 {
            let chat = recovery.next(&chats, now + PACE * attempt).unwrap();
            recovery.answered(&chat, true);
        }
        let chat = recovery.next(&chats, now + PACE * 10).unwrap();
        assert_eq!(chat, "a");
        recovery.reconnect();
        assert_eq!(recovery.next(&chats, now + PACE * 11).as_deref(), Some("b"));
    }
    #[test]
    fn timeout_late_response_no_progress_lock_pause_and_account_isolation() {
        let now = Instant::now();
        let chats = vec!["a".into()];
        let mut first = Recovery {
            enabled: true,
            ..Default::default()
        };
        let mut second = Recovery {
            enabled: true,
            ..Default::default()
        };
        first.paused = true;
        assert!(first.next(&chats, now).is_none());
        first.paused = false;
        assert!(first.next(&chats, now).is_some());
        assert!(first.next(&chats, now + PACE * 10).is_none());
        assert!(second.next(&chats, now).is_some());
        assert!(first.answered("a", false));
        assert!(
            !first.answered("a", true),
            "stale duplicate cannot release another owner"
        );
        assert!(second.owns("a"));
        assert!(first.next(&chats, now + PACE * 11).is_none());
        assert!(
            !Recovery::default().enabled,
            "restart retains opt-in policy via account settings"
        );
    }
}
