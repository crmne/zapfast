//! Account-wide admission for every automatic phone-history request.
//!
//! Persist reservations before sending. History and media keep independent daily
//! quotas, but every automatic phone request shares one pace and failure backoff.

use crate::model::ChatId;
use std::collections::HashMap;

const PACE: i64 = 20;
const MAX_BACKOFF: i64 = 60 * 60;
const MAX_FAILURES: u8 = 7;
const RECENT_BOUNDARY_ATTEMPTS: u32 = 3;

#[derive(Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub(super) struct Budget {
    day: i64,
    requests: u32,
    media_requests: u32,
    next_request: i64,
    failure_streak: u8,
    retry_at: i64,
    needs_attention: bool,
    chats: HashMap<ChatId, Chat>,
    #[serde(skip)]
    session_requests: u32,
    #[serde(skip)]
    session_pages: HashMap<ChatId, u32>,
}

#[derive(Default, serde::Serialize, serde::Deserialize)]
struct Chat {
    older: Option<Attempt>,
    polls: HashMap<String, Attempt>,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct Attempt {
    anchor: (i64, String),
    count: u32,
    next: i64,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Admission {
    Allowed,
    Wait,
    Stopped,
}

impl Budget {
    pub fn restore(value: &str) -> Result<Self, serde_json::Error> {
        // The old background-only checkpoint has these same accounting fields.
        // It can seed a first upgrade without renewing today's request budget.
        serde_json::from_str(value)
    }

    pub fn checkpoint(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    pub fn canonicalize(&mut self, old: &str, new: &str) {
        if let Some(source) = self.chats.remove(old) {
            let target = self.chats.entry(new.to_owned()).or_default();
            merge(&mut target.older, source.older);
            for (id, attempt) in source.polls {
                target
                    .polls
                    .entry(id)
                    .and_modify(|current| merge_attempt(current, attempt.clone()))
                    .or_insert(attempt);
            }
        }
        if let Some(pages) = self.session_pages.remove(old) {
            let target = self.session_pages.entry(new.to_owned()).or_default();
            *target = target.saturating_add(pages);
        }
    }

    pub fn ready(&self, full: bool, daily: u32, now: i64) -> bool {
        let requests = if self.day == now.div_euclid(86_400) {
            self.requests
        } else {
            0
        };
        requests < daily.max(1)
            && !self.needs_attention
            && now >= self.next_request.max(self.retry_at)
            && (full || self.session_requests < 50)
    }

    pub fn media_ready(&self, daily: u32, now: i64) -> bool {
        let requests = if self.day == now.div_euclid(86_400) {
            self.media_requests
        } else {
            0
        };
        requests < daily.max(1)
            && !self.needs_attention
            && now >= self.next_request.max(self.retry_at)
    }

    /// `poll == None` shares the same boundary for short-chat and background
    /// paging. Poll snapshots use a separate boundary from older-message pages.
    pub fn reserve(
        &mut self,
        chat: &str,
        poll: Option<&str>,
        anchor: (i64, String),
        full: bool,
        daily: u32,
        now: i64,
    ) -> Admission {
        let anchor = if anchor.1.is_empty() {
            // Empty chats are anchored at "now" on the wire, but that changing
            // timestamp must not turn every retry into apparent progress.
            (0, String::new())
        } else {
            anchor
        };
        let previous = self.chats.get(chat).and_then(|chat| match poll {
            Some(id) => chat.polls.get(id),
            None => chat.older.as_ref(),
        });
        let previous = previous.filter(|attempt| attempt.anchor == anchor);
        if !full && previous.is_some_and(|attempt| attempt.count >= RECENT_BOUNDARY_ATTEMPTS) {
            return Admission::Stopped;
        }
        if !self.ready(full, daily, now)
            || (!full && self.session_pages.get(chat).copied().unwrap_or(0) >= 5)
            || previous.is_some_and(|attempt| now < attempt.next)
        {
            return Admission::Wait;
        }
        let count = previous.map_or(1, |attempt| attempt.count.saturating_add(1));
        let attempt = Attempt {
            anchor,
            count,
            next: now + retry_delay(count),
        };
        let target = self.chats.entry(chat.to_owned()).or_default();
        if let Some(id) = poll {
            target.polls.insert(id.to_owned(), attempt);
        } else {
            target.older = Some(attempt);
        }
        self.roll_day(now);
        self.requests = self.requests.saturating_add(1);
        self.next_request = now + PACE;
        self.session_requests = self.session_requests.saturating_add(1);
        *self.session_pages.entry(chat.to_owned()).or_default() += 1;
        Admission::Allowed
    }

    /// Reserves a media re-upload against its own daily quota and the shared
    /// account-wide phone pace/backoff gate.
    pub fn reserve_media(&mut self, daily: u32, now: i64) -> Admission {
        if !self.media_ready(daily, now) {
            return Admission::Wait;
        }
        self.roll_day(now);
        self.media_requests = self.media_requests.saturating_add(1);
        self.next_request = now + PACE;
        Admission::Allowed
    }

    pub fn failed(&mut self, now: i64) {
        self.failure_streak = self.failure_streak.saturating_add(1);
        self.retry_at = now.saturating_add(retry_delay(u32::from(self.failure_streak)));
        self.needs_attention = self.failure_streak >= MAX_FAILURES;
    }

    pub fn succeeded(&mut self) {
        self.failure_streak = 0;
        self.retry_at = 0;
    }

    pub fn retry_at(&self) -> i64 {
        self.retry_at
    }

    /// The first UTC second when the history request quota is available again.
    /// This is status-only; admission still uses `ready`.
    pub fn history_daily_retry_at(&self, daily: u32, now: i64) -> Option<i64> {
        let day = now.div_euclid(86_400);
        (self.day == day && self.requests >= daily.max(1))
            .then_some(day.saturating_add(1).saturating_mul(86_400))
    }

    pub fn needs_attention(&self) -> bool {
        self.needs_attention
    }

    pub fn resume(&mut self) {
        self.failure_streak = 0;
        self.retry_at = 0;
        self.needs_attention = false;
    }

    fn roll_day(&mut self, now: i64) {
        let day = now.div_euclid(86_400);
        if self.day != day {
            self.day = day;
            self.requests = 0;
            self.media_requests = 0;
        }
    }
}

fn retry_delay(attempt: u32) -> i64 {
    60_i64
        .saturating_mul(1_i64 << attempt.saturating_sub(1).min(6))
        .min(MAX_BACKOFF)
}

fn merge(target: &mut Option<Attempt>, source: Option<Attempt>) {
    if let Some(source) = source {
        if let Some(target) = target {
            merge_attempt(target, source);
        } else {
            *target = Some(source);
        }
    }
}

fn merge_attempt(target: &mut Attempt, source: Attempt) {
    if target.anchor == source.anchor {
        target.count = target.count.saturating_add(source.count);
        target.next = target.next.max(source.next);
    } else if source.next > target.next {
        *target = source;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn anchor(id: &str) -> (i64, String) {
        (100, id.into())
    }

    #[test]
    fn all_sources_share_pacing_daily_budget_and_restart_accounting() {
        let mut budget = Budget::default();
        assert_eq!(
            budget.reserve("a", None, anchor("old"), true, 2, 100),
            Admission::Allowed
        );
        assert_eq!(
            budget.reserve("b", Some("poll"), anchor("poll"), true, 2, 119),
            Admission::Wait
        );
        assert_eq!(
            budget.reserve("b", Some("poll"), anchor("poll"), true, 2, 120),
            Admission::Allowed
        );
        let mut restarted = Budget::restore(&budget.checkpoint().unwrap()).unwrap();
        assert_eq!(restarted.history_daily_retry_at(2, 140), Some(86_400));
        assert_eq!(
            restarted.reserve("c", None, anchor("old"), true, 2, 140),
            Admission::Wait
        );
        assert_eq!(
            restarted.reserve("c", None, anchor("old"), true, 2, 86_400),
            Admission::Allowed
        );
        assert_eq!(restarted.history_daily_retry_at(2, 86_400), None);
        assert!(
            !restarted.ready(true, 2, 86_399),
            "clock rollback cannot bypass pacing"
        );
        assert!(
            Budget::default().ready(true, 2, 140),
            "accounts are independent"
        );
    }

    #[test]
    fn unchanged_boundaries_remain_resumable_across_restarts() {
        let mut budget = Budget::default();
        for now in [100, 160, 280] {
            assert_eq!(
                budget.reserve("a", None, anchor("old"), true, 100, now),
                Admission::Allowed
            );
            budget = Budget::restore(&budget.checkpoint().unwrap()).unwrap();
        }
        assert_eq!(
            budget.reserve("a", None, anchor("old"), true, 100, 100_000),
            Admission::Allowed
        );
        assert_eq!(
            budget.reserve("a", None, (90, "older".into()), true, 100, 100_000),
            Admission::Wait
        );
        assert_eq!(
            budget.reserve("a", Some("poll"), anchor("old"), true, 100, 100_020),
            Admission::Allowed
        );
    }

    #[test]
    fn empty_chats_share_one_backoff_boundary_despite_changing_wall_time() {
        let mut budget = Budget::default();
        for now in [100, 160, 280] {
            assert_eq!(
                budget.reserve("a", None, (now, String::new()), true, 100, now),
                Admission::Allowed
            );
        }
        assert_eq!(
            budget.reserve("a", None, (1000, String::new()), true, 100, 1000),
            Admission::Allowed
        );
    }

    #[test]
    fn recent_mode_session_caps_include_polls_and_survive_mode_changes() {
        let mut budget = Budget::default();
        for page in 0..5 {
            assert_eq!(
                budget.reserve(
                    "a",
                    Some(&page.to_string()),
                    anchor("p"),
                    false,
                    100,
                    100 + page * 20
                ),
                Admission::Allowed
            );
        }
        assert_eq!(
            budget.reserve("a", None, anchor("old"), false, 100, 200),
            Admission::Wait
        );
        for page in 5..50 {
            assert_eq!(
                budget.reserve(
                    &page.to_string(),
                    None,
                    anchor("old"),
                    true,
                    100,
                    100 + page * 20
                ),
                Admission::Allowed
            );
        }
        assert!(!budget.ready(false, 100, 2000));
    }

    #[test]
    fn legacy_checkpoint_migrates_accounting_and_identity_merge_preserves_backoff() {
        let mut budget = Budget::restore(r#"{"day":0,"requests":9,"next_request":200}"#).unwrap();
        assert!(!budget.ready(true, 9, 300));
        for now in [300, 360, 480] {
            assert_eq!(
                budget.reserve("lid", None, anchor("old"), true, 100, now),
                Admission::Allowed
            );
        }
        budget.canonicalize("lid", "phone");
        assert_eq!(
            budget.reserve("phone", None, anchor("old"), true, 100, 1000),
            Admission::Allowed
        );
    }

    #[test]
    fn poll_retries_remain_resumable_across_reconnect_and_day_rollover() {
        let mut budget = Budget::default();
        assert_eq!(
            budget.reserve("a", Some("poll"), anchor("p"), true, 100, 100),
            Admission::Allowed
        );
        assert_eq!(
            budget.reserve("a", Some("poll"), anchor("p"), true, 100, 120),
            Admission::Wait
        );
        assert_eq!(
            budget.reserve("a", Some("poll"), anchor("p"), true, 100, 160),
            Admission::Allowed
        );
        let mut budget = Budget::restore(&budget.checkpoint().unwrap()).unwrap();
        assert_eq!(
            budget.reserve("a", Some("poll"), anchor("p"), true, 100, 200),
            Admission::Wait
        );
        assert_eq!(
            budget.reserve("a", Some("poll"), anchor("p"), true, 100, 280),
            Admission::Allowed
        );
        assert_eq!(
            budget.reserve("a", Some("poll"), anchor("p"), true, 100, 100_000),
            Admission::Allowed
        );
        assert_eq!(
            budget.reserve("a", Some("other-poll"), anchor("p"), true, 100, 100_000),
            Admission::Wait
        );
        assert_eq!(
            budget.reserve("a", Some("other-poll"), anchor("p"), true, 100, 100_020),
            Admission::Allowed
        );
    }

    #[test]
    fn merging_the_same_boundary_never_lowers_attempts_or_shortens_backoff() {
        let mut target = Some(Attempt {
            anchor: anchor("old"),
            count: 1,
            next: 1000,
        });
        merge(
            &mut target,
            Some(Attempt {
                anchor: anchor("old"),
                count: 3,
                next: 500,
            }),
        );
        let merged = target.unwrap();
        assert_eq!((merged.count, merged.next), (4, 1000));
        let mut aliases = Attempt {
            anchor: anchor("same"),
            count: 2,
            next: 1000,
        };
        merge_attempt(
            &mut aliases,
            Attempt {
                anchor: anchor("same"),
                count: 2,
                next: 900,
            },
        );
        assert_eq!(aliases.count, 4, "requests under both identities count");
    }

    #[test]
    fn media_has_its_own_quota_but_shares_phone_pace_and_backoff() {
        let mut budget = Budget::default();
        assert_eq!(budget.reserve_media(1, 100), Admission::Allowed);
        assert_eq!(
            budget.reserve("a", None, anchor("old"), true, 10, 119),
            Admission::Wait
        );
        assert_eq!(
            budget.reserve("a", None, anchor("old"), true, 10, 120),
            Admission::Allowed
        );
        assert_eq!(
            budget.reserve_media(1, 140),
            Admission::Wait,
            "media quota is independent"
        );
        budget.failed(140);
        assert!(!budget.ready(true, 10, 199));
        assert!(budget.ready(true, 10, 200));
        budget.succeeded();
        assert_eq!(budget.retry_at(), 0);
    }

    #[test]
    fn history_and_media_ledgers_survive_restart_without_resetting_each_other() {
        let mut budget = Budget::default();
        assert_eq!(
            budget.reserve("a", None, anchor("old"), true, 1, 100),
            Admission::Allowed
        );
        assert_eq!(budget.reserve_media(1, 120), Admission::Allowed);
        let mut restarted = Budget::restore(&budget.checkpoint().unwrap()).unwrap();
        assert_eq!(
            restarted.reserve("b", None, anchor("old"), true, 1, 140),
            Admission::Wait,
            "media accounting must not renew the exhausted history quota"
        );
        assert_eq!(
            restarted.reserve_media(1, 140),
            Admission::Wait,
            "history accounting must not renew the exhausted media quota"
        );
        assert_eq!(restarted.reserve_media(1, 86_400), Admission::Allowed);
        assert_eq!(
            restarted.reserve("b", None, anchor("old"), true, 1, 86_420),
            Admission::Allowed
        );
    }

    #[test]
    fn sustained_failures_pause_until_explicit_resume() {
        let mut budget = Budget::default();
        for attempt in 0..MAX_FAILURES {
            budget.failed(100 + i64::from(attempt));
        }
        assert!(budget.needs_attention());
        assert!(!budget.ready(true, 100, 1_000_000));
        assert!(!budget.media_ready(100, 1_000_000));
        let restored = Budget::restore(&budget.checkpoint().unwrap()).unwrap();
        assert!(restored.needs_attention(), "visible pause survives restart");
        budget.resume();
        assert!(budget.ready(true, 100, 1_000_000));
    }

    #[test]
    fn recent_mode_keeps_its_bounded_unchanged_boundary_policy() {
        let mut budget = Budget::default();
        for now in [100, 160, 280] {
            assert_eq!(
                budget.reserve("a", None, anchor("old"), false, 100, now),
                Admission::Allowed
            );
        }
        assert_eq!(
            budget.reserve("a", None, anchor("old"), false, 100, 100_000),
            Admission::Stopped
        );
        assert_eq!(
            budget.reserve("a", None, (90, "older".into()), false, 100, 100_000),
            Admission::Allowed
        );
    }
}
