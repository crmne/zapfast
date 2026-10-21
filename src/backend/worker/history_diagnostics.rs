//! Fixed labels and numeric counters only, never protocol identities or payloads.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Flow {
    Initial,
    OnDemand,
}

impl Flow {
    /// Separates unsolicited history from chunks answering a phone-history request.
    pub fn incoming(kind: i32, has_request_session: bool) -> Self {
        if kind == super::ON_DEMAND || has_request_session {
            Self::OnDemand
        } else {
            Self::Initial
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Stage {
    Received,
    Applied,
    ParseFailed,
    Quiet,
    RequestSent,
    RequestFailed,
    RequestTimeout,
}

pub(super) struct Record {
    pub account: u32,
    pub flow: Flow,
    pub stage: Stage,
    pub sync_type: i32,
    pub percent: i32,
    pub chats: usize,
    pub entries: usize,
    pub elapsed_ms: u128,
}

impl Record {
    /// Starts a count-only diagnostic with unknown protocol fields left unset.
    pub fn new(account: u32, flow: Flow, stage: Stage) -> Self {
        Self {
            account,
            flow,
            stage,
            sync_type: -1,
            percent: -1,
            chats: 0,
            entries: 0,
            elapsed_ms: 0,
        }
    }

    /// Writes fixed labels and aggregate counters without identifying chats or messages.
    pub fn emit(&self) {
        #[cfg(test)]
        CAPTURE.with(|capture| {
            if let Some(stages) = capture.borrow_mut().as_mut() {
                stages.push(self.stage);
            }
        });
        log::info!("{self}");
    }
}

#[cfg(test)]
thread_local! {
    static CAPTURE: std::cell::RefCell<Option<Vec<Stage>>> = const { std::cell::RefCell::new(None) };
}

/// Captures emitted diagnostic stages on this test thread without a process-wide logger.
#[cfg(test)]
pub(super) fn capture(operation: impl FnOnce()) -> Vec<Stage> {
    CAPTURE.with(|capture| *capture.borrow_mut() = Some(Vec::new()));
    operation();
    CAPTURE.with(|capture| capture.borrow_mut().take().unwrap())
}

impl std::fmt::Display for Record {
    /// Formats the versioned diagnostic using only fixed labels and numeric fields.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let flow = match self.flow {
            Flow::Initial => "initial",
            Flow::OnDemand => "on_demand",
        };
        let stage = match self.stage {
            Stage::Received => "received",
            Stage::Applied => "applied",
            Stage::ParseFailed => "parse_failed",
            Stage::Quiet => "quiet",
            Stage::RequestSent => "request_sent",
            Stage::RequestFailed => "request_failed",
            Stage::RequestTimeout => "request_timeout",
        };
        write!(
            f,
            "[HISTORY] version=1 account={} flow={flow} stage={stage} sync_type={} percent={} chats={} entries={} elapsed_ms={}",
            self.account, self.sync_type, self.percent, self.chats, self.entries, self.elapsed_ms
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn incoming_history_keeps_initial_and_requested_delivery_separate() {
        assert_eq!(Flow::incoming(2, false), Flow::Initial);
        assert_eq!(Flow::incoming(3, false), Flow::Initial);
        assert_eq!(
            Flow::incoming(super::super::ON_DEMAND, false),
            Flow::OnDemand
        );
        assert_eq!(Flow::incoming(2, true), Flow::OnDemand);
    }

    #[test]
    fn received_applied_and_quiet_records_do_not_certify_storage_or_coverage() {
        let mut record = Record::new(4, Flow::Initial, Stage::Received);
        record.sync_type = 3;
        record.percent = 100;
        for stage in [Stage::Received, Stage::Applied, Stage::Quiet] {
            record.stage = stage;
            let line = record.to_string();
            assert!(line.contains("flow=initial"));
            assert!(!line.contains("stored") && !line.contains("complete"));
        }
    }
}
