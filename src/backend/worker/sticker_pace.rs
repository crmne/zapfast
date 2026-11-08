//! Paces sticker downloads so a burst of them never trips WhatsApp's rate
//! limit. Fetching every missing favorite at once on connecting (126 for one
//! reader) answered `429 rate-overlimit` and left the account throttled, so
//! the next picture or sticker the reader sent failed too (#298, #307).
//! Opening the picker did the same with the chat stickers it fetches (#405).

use std::collections::{HashSet, VecDeque};
use std::time::{Duration, Instant};

/// Favorites started per tick of the worker's five-second clock.
const PER_TICK: usize = 2;
/// Favorite downloads, and picker downloads (recent and chat stickers
/// together), allowed in flight at once.
pub(super) const IN_FLIGHT: usize = 2;
/// The first pause after the server says to slow down, doubled each time
/// it says so again, up to [`LONGEST`].
const FIRST: Duration = Duration::from_secs(30);
const LONGEST: Duration = Duration::from_secs(15 * 60);
/// How long a favorite whose file is gone from the servers rests before it
/// is asked for again.
pub(super) const GONE_FOR: Duration = Duration::from_secs(7 * 24 * 60 * 60);

#[derive(Default)]
pub(super) struct Pace {
    queue: VecDeque<String>,
    queued: HashSet<String>,
    until: Option<Instant>,
    pause: Duration,
}

impl Pace {
    /// Queues a favorite for fetching, once.
    pub(super) fn push(&mut self, hash: String) {
        if self.queued.insert(hash.clone()) {
            self.queue.push_back(hash);
        }
    }

    /// Whether downloads may start now, or the server asked to wait.
    pub(super) fn open(&self, now: Instant) -> bool {
        self.until.is_none_or(|until| now >= until)
    }

    /// The favorites to start now, given how many are already in flight.
    pub(super) fn take(&mut self, now: Instant, in_flight: usize) -> Vec<String> {
        if !self.open(now) {
            return Vec::new();
        }
        let room = PER_TICK.min(IN_FLIGHT.saturating_sub(in_flight));
        let taken: Vec<String> = (0..room).map_while(|_| self.queue.pop_front()).collect();
        for hash in &taken {
            self.queued.remove(hash);
        }
        taken
    }

    /// The server said to slow down: pause every sticker download, longer
    /// each time it says so again before a pause has passed.
    pub(super) fn limited(&mut self, now: Instant) {
        let again = self.until.is_some_and(|until| now < until + self.pause);
        self.pause = if again {
            (self.pause * 2).min(LONGEST)
        } else {
            FIRST
        };
        self.until = Some(now + self.pause);
    }

    pub(super) fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }
}

/// Whether a download error is the server's rate limit.
pub(super) fn rate_limited(error: &str) -> bool {
    error.contains("rate-overlimit") || error.contains("code=429") || names_status(error, "429")
}

/// Whether a download error means the file is gone from the servers, so
/// asking again soon cannot help.
pub(super) fn gone(error: &str) -> bool {
    ["403", "404", "410"]
        .iter()
        .any(|status| names_status(error, status))
}

/// Whether a download error carries this HTTP status, written either as
/// `status: 403` or, inside the library's `MediaFailure`, as
/// `http_status: Some(403)`.
fn names_status(error: &str, code: &str) -> bool {
    error.contains(&format!("status: {code}"))
        || error.contains(&format!("http_status: Some({code})"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn favorites_start_two_at_a_time_and_only_once_each() {
        let mut pace = Pace::default();
        let now = Instant::now();
        for hash in ["a", "b", "c", "a"] {
            pace.push(hash.to_owned());
        }
        assert_eq!(pace.take(now, 0), ["a", "b"]);
        assert_eq!(pace.take(now, 2), Vec::<String>::new(), "two in flight");
        assert_eq!(pace.take(now, 1), ["c"]);
        assert!(pace.is_empty());
    }

    #[test]
    fn a_rate_limit_pauses_and_a_second_one_pauses_longer() {
        let mut pace = Pace::default();
        pace.push("a".to_owned());
        let now = Instant::now();
        pace.limited(now);
        assert!(!pace.open(now + Duration::from_secs(29)));
        assert!(pace.take(now + Duration::from_secs(29), 0).is_empty());
        let later = now + Duration::from_secs(30);
        assert!(pace.open(later));
        pace.limited(later);
        assert!(!pace.open(later + Duration::from_secs(59)));
        assert!(pace.open(later + Duration::from_secs(60)));
        // Long after the last pause, the next one starts short again.
        let much_later = later + Duration::from_secs(3600);
        pace.limited(much_later);
        assert!(pace.open(much_later + FIRST));
    }

    #[test]
    fn errors_are_read_the_way_the_library_writes_them() {
        assert!(rate_limited(
            "received a server error response: code=429, text='rate-overlimit'"
        ));
        assert!(gone("Download failed with status: 403"));
        assert!(gone("Download media not found/expired with status: 410"));
        assert!(!gone(
            "received a server error response: code=429, text='rate-overlimit'"
        ));
        assert!(!rate_limited("Download failed with status: 403"));
    }

    /// The library's `MediaFailure` prints the status as `http_status:
    /// Some(403)`, which holds no `status: 403` (#888).
    #[test]
    fn a_status_inside_a_media_failure_is_read_too() {
        let failure = |status: u16| {
            format!(
                "MediaFailure {{ preparation: None, iq_kind: None, iq_code: None, \
                 http_status: Some({status}), io_kind: None, media_validation: false, .. }}"
            )
        };
        let rejected =
            |status| format!("the CDN rejected the media reference: {}", failure(status));
        let exhausted = |status| format!("every media host failed: {}", failure(status));
        for status in [403, 404, 410] {
            assert!(gone(&rejected(status)), "{status}");
            assert!(gone(&exhausted(status)), "{status}");
            assert!(!rate_limited(&rejected(status)), "{status}");
        }
        assert!(!gone(&rejected(500)));
        assert!(!gone(&rejected(429)));
        assert!(rate_limited(&exhausted(429)));
        assert!(rate_limited(&rejected(429)));
        assert!(!rate_limited(&exhausted(500)));
    }
}
