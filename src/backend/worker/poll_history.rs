//! Serialize and back off automatic phone-history requests for poll results.

use crate::model::ChatId;
use std::{
    collections::{HashMap, VecDeque},
    time::{Duration, Instant},
};

type Key = (ChatId, String);
#[derive(Default)]
pub(super) struct Requests {
    queue: VecDeque<(Key, Instant)>,
    active: Option<(Key, Instant)>,
    tried: HashMap<Key, u32>,
}

impl Requests {
    /// Pending, requested this session, and waiting after a failed attempt.
    pub fn state(&self, chat: &str, poll: &str) -> (bool, bool, bool) {
        let key = (chat.to_owned(), poll.to_owned());
        let queued = self.queue.iter().any(|(pending, _)| pending == &key);
        (
            queued
                || self
                    .active
                    .as_ref()
                    .is_some_and(|(active, _)| active == &key),
            self.tried.contains_key(&key),
            queued && self.tried.get(&key).is_some_and(|&failures| failures > 0),
        )
    }

    pub fn request(&mut self, chat: &str, poll: &str, now: Instant) {
        let key = (chat.to_owned(), poll.to_owned());
        if self.state(chat, poll).0 || self.queue.len() >= 64 {
            return;
        }
        self.tried.entry(key.clone()).or_insert(0);
        self.queue.push_back((key, now));
    }

    pub fn next(&mut self, now: Instant) -> Option<Key> {
        if self.active.is_some() {
            return None;
        }
        let index = self.queue.iter().position(|(_, due)| *due <= now)?;
        let (key, _) = self.queue.remove(index)?;
        self.active = Some((key.clone(), now));
        Some(key)
    }

    pub fn finish(&mut self, chat: &str, poll: &str) {
        let key = (chat.to_owned(), poll.to_owned());
        self.queue.retain(|(pending, _)| pending != &key);
        if self
            .active
            .as_ref()
            .is_some_and(|(active, _)| active == &key)
        {
            self.active = None;
        }
        self.tried.insert(key, 0);
    }

    pub fn fail(&mut self, chat: &str, poll: &str, requested: Instant, now: Instant) {
        if self
            .active
            .as_ref()
            .is_some_and(|((active_chat, active_poll), started)| {
                active_chat == chat && active_poll == poll && *started == requested
            })
        {
            self.defer(now);
        }
    }

    fn defer(&mut self, now: Instant) -> Option<Key> {
        let (key, _) = self.active.take()?;
        let failures = self.tried.entry(key.clone()).or_default();
        *failures = failures.saturating_add(1);
        let delay = (30 * (1_u64 << (*failures - 1).min(5))).min(900);
        self.queue
            .push_back((key.clone(), now + Duration::from_secs(delay)));
        Some(key)
    }

    pub fn expire(&mut self, now: Instant) -> Option<Key> {
        if self.active.as_ref().is_some_and(|(_, started)| {
            now.saturating_duration_since(*started) >= Duration::from_secs(30)
        }) {
            return self.defer(now);
        }
        None
    }

    pub fn reconnect(&mut self, now: Instant) {
        for (_, due) in &mut self.queue {
            *due = now;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn poll_history_retries_automatically_without_spinning_or_parallel_requests() {
        let mut requests = Requests::default();
        let now = Instant::now();
        requests.request("chat", "one", now);
        requests.request("chat", "one", now);
        requests.request("chat", "two", now);
        assert_eq!(requests.next(now), Some(("chat".into(), "one".into())));
        assert!(requests.next(now).is_none());
        assert!(requests.expire(now + Duration::from_secs(29)).is_none());
        assert_eq!(
            requests.expire(now + Duration::from_secs(30)),
            Some(("chat".into(), "one".into()))
        );
        assert_eq!(requests.state("chat", "one"), (true, true, true));
        assert_eq!(
            requests.next(now + Duration::from_secs(30)),
            Some(("chat".into(), "two".into()))
        );
        requests.finish("chat", "two");
        assert!(requests.next(now + Duration::from_secs(59)).is_none());
        assert_eq!(
            requests.next(now + Duration::from_secs(60)),
            Some(("chat".into(), "one".into()))
        );
        // An error from the expired attempt must not cancel its replacement.
        requests.fail("chat", "one", now, now + Duration::from_secs(61));
        assert_eq!(requests.state("chat", "one"), (true, true, false));
        requests.fail(
            "chat",
            "one",
            now + Duration::from_secs(60),
            now + Duration::from_secs(61),
        );
        assert!(requests.next(now + Duration::from_secs(120)).is_none());
        assert_eq!(
            requests.next(now + Duration::from_secs(121)),
            Some(("chat".into(), "one".into()))
        );
        requests.finish("chat", "one");
        assert_eq!(requests.state("chat", "one"), (false, true, false));
        assert!(requests.next(now + Duration::from_secs(1000)).is_none());
    }
}
