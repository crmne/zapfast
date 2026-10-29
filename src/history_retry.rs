//! Per-chat phone-history retry state shared by background and date-scoped requests.

use serde::{Deserialize, Serialize};
use std::time::Duration;

pub(crate) const HISTORY_MAX_TIMEOUTS: u32 = 3;

/// Three unanswered requests pause a chat until recovery is retried.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct HistoryRetry<T> {
    pub failures: u32,
    #[serde(rename = "retry_at")]
    pub due: T,
}

impl<T> HistoryRetry<T> {
    pub fn new(due: T) -> Self {
        Self { failures: 0, due }
    }

    pub fn stopped(&self) -> bool {
        self.failures >= HISTORY_MAX_TIMEOUTS
    }

    /// Counts one unanswered attempt and returns its per-chat backoff.
    pub fn fail(&mut self) -> Duration {
        self.failures = self.failures.saturating_add(1);
        Duration::from_secs((30 * (1_u64 << (self.failures - 1).min(5))).min(900))
    }
}
