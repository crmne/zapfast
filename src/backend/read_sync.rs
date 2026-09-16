//! Schedule private read-state writes, which all share one app-state collection.

use std::time::{Duration, Instant};

use crate::model::ChatId;

/// A failed collection blocks every pending chat, not just the failed chat.
/// The archive owns the durable queue; this only limits work on the connection.
#[derive(Default)]
pub(super) struct ReadSync {
    in_flight: Option<(ChatId, i64)>,
    retry_at: Option<Instant>,
    failures: u32,
}

impl ReadSync {
    pub fn ready(&self, now: Instant) -> bool {
        self.in_flight.is_none() && self.retry_at.is_none_or(|retry| now >= retry)
    }

    pub fn start(&mut self, chat: &str, through: i64, now: Instant) -> bool {
        if !self.ready(now) {
            return false;
        }
        self.in_flight = Some((chat.to_owned(), through));
        true
    }

    /// Ignore a completion from a request which is no longer ours.
    pub fn finish(&mut self, chat: &str, through: i64, success: bool, now: Instant) -> bool {
        if !self
            .in_flight
            .as_ref()
            .is_some_and(|(active, position)| active == chat && *position == through)
        {
            return false;
        }
        self.in_flight = None;
        if success {
            self.failures = 0;
            self.retry_at = None;
        } else {
            self.failures = self.failures.saturating_add(1);
            let seconds = (30 * (1_u64 << (self.failures - 1).min(5))).min(15 * 60);
            self.retry_at = Some(now + Duration::from_secs(seconds));
            log::warn!("read-state sync paused; retrying in {seconds} seconds");
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_failed_collection_holds_back_every_chat_with_capped_backoff() {
        let mut sync = ReadSync::default();
        let mut now = Instant::now();
        for delay in [30, 60, 120, 240, 480, 900, 900] {
            assert!(sync.start("first", 100, now));
            assert!(!sync.start("second", 200, now + Duration::from_secs(3600)));
            assert!(sync.finish("first", 100, false, now));
            let deadline = now + Duration::from_secs(delay);
            assert!(!sync.start("second", 200, deadline - Duration::from_nanos(1)));
            assert!(sync.ready(deadline));
            now = deadline;
        }
        assert!(sync.start("second", 200, now));
        assert!(sync.finish("second", 200, true, now));
        assert!(sync.start("third", 300, now));
        assert!(sync.finish("third", 300, false, now));
        assert!(
            sync.ready(now + Duration::from_secs(30)),
            "success resets the backoff"
        );
    }

    #[test]
    fn an_unrelated_completion_cannot_release_the_active_request() {
        let mut sync = ReadSync::default();
        let now = Instant::now();
        assert!(sync.start("current", 200, now));
        assert!(!sync.finish("old", 200, true, now));
        assert!(!sync.finish("current", 100, true, now));
        assert!(!sync.ready(now));
        assert!(sync.finish("current", 200, true, now));
        assert!(sync.ready(now));
    }
}
