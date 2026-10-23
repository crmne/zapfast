//! Counter-only history diagnostics. No protocol identifiers or payloads enter a record.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Flow {
    Initial,
    OnDemand,
}

impl Flow {
    pub fn incoming(sync_type: i32, has_request_session: bool) -> Self {
        if sync_type == super::ON_DEMAND || has_request_session {
            Self::OnDemand
        } else {
            Self::Initial
        }
    }
}

#[derive(Clone, Copy)]
pub(super) enum Stage {
    Received,
    Stored,
    ParseFailed,
    StoreFailed,
    Quiet,
    DispatchStarted,
    DispatchTimeout,
    RequestRejectedNotConnected,
    RequestRejectedInvalidChat,
    RequestDeferred,
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
    /// Supported entries processed, including duplicates, not newly added messages.
    pub entries: usize,
    pub elapsed_ms: u128,
}

impl Record {
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

    pub fn emit(&self) {
        log::info!("{self}");
    }
}

impl std::fmt::Display for Record {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let flow = match self.flow {
            Flow::Initial => "initial",
            Flow::OnDemand => "on_demand",
        };
        let stage = match self.stage {
            Stage::Received => "received",
            Stage::Stored => "stored",
            Stage::ParseFailed => "parse_failed",
            Stage::StoreFailed => "store_failed",
            Stage::Quiet => "quiet",
            Stage::DispatchStarted => "dispatch_started",
            Stage::DispatchTimeout => "dispatch_timeout",
            Stage::RequestRejectedNotConnected => "request_rejected_not_connected",
            Stage::RequestRejectedInvalidChat => "request_rejected_invalid_chat",
            Stage::RequestDeferred => "request_deferred",
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
    fn requested_pages_stay_separate_from_initial_chunks_including_session_tagged_answers() {
        assert_eq!(Flow::incoming(2, false), Flow::Initial);
        assert_eq!(Flow::incoming(3, false), Flow::Initial);
        assert_eq!(
            Flow::incoming(super::super::ON_DEMAND, false),
            Flow::OnDemand
        );
        assert_eq!(Flow::incoming(2, true), Flow::OnDemand);
    }

    #[test]
    fn records_keep_delivery_and_storage_separate_without_claiming_completion() {
        let mut record = Record::new(4, Flow::Initial, Stage::Received);
        record.sync_type = 2;
        record.percent = 100;
        assert!(
            record
                .to_string()
                .contains("stage=received sync_type=2 percent=100")
        );
        record.stage = Stage::StoreFailed;
        assert!(record.to_string().contains("stage=store_failed"));
        assert!(!record.to_string().contains("complete"));
        record.stage = Stage::Quiet;
        assert!(record.to_string().contains("stage=quiet"));
        assert!(!record.to_string().contains("complete"));
    }
}
