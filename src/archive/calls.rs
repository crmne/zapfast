//! The call log: what became of every 1:1 call this account placed or took.
//!
//! A call is not a message. WhatsApp delivers call history over its own stream, and this app writes
//! a row here when a call it handled reaches its end, from what the peer's signaling and the media
//! plane said rather than from the button that was pressed: a call that was never answered is not
//! an answered one, whatever the dialer did.
//!
//! The chat id is the canonical one, so a call to a number reached through a privacy id lands on the
//! same chat as that chat's messages, and the privacy-id mapping moves a chat's calls onto its
//! number when the mapping arrives. Nothing here holds message content, and nothing here is sent
//! anywhere: the log is local to this computer.
//!
//! Deleting or clearing a chat leaves the log alone, as WhatsApp's own Calls tab does: a call is its
//! own record, and a log the user never asked to clear is not theirs to lose.

use super::{Archive, Result, params};
use crate::model::{CallDirection, CallMedia, CallRecord, CallStatus};

pub const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS calls (
    id TEXT PRIMARY KEY,
    chat TEXT NOT NULL,
    started_at INTEGER NOT NULL,
    ended_at INTEGER NOT NULL,
    direction TEXT NOT NULL,
    media TEXT NOT NULL,
    status TEXT NOT NULL,
    duration INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS calls_by_time ON calls (started_at);
CREATE INDEX IF NOT EXISTS calls_by_chat ON calls (chat, started_at);
";

impl Archive {
    /// Writes one finished call. The WhatsApp call id is the key, so a call resolved twice (a
    /// terminate and a media close, say) stays one row.
    pub fn save_call(&self, call: &CallRecord) -> Result<()> {
        self.connection.execute(
            "INSERT OR REPLACE INTO calls
                 (id, chat, started_at, ended_at, direction, media, status, duration)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                call.id,
                call.chat,
                call.started_at,
                call.ended_at,
                direction_key(call.direction),
                media_key(call.media),
                status_key(call.status),
                call.duration as i64,
            ],
        )?;
        Ok(())
    }

    /// Every call, newest first.
    pub fn calls(&self) -> Result<Vec<CallRecord>> {
        self.calls_where("", [])
    }

    /// One chat's calls, newest first.
    pub fn calls_for_chat(&self, chat: &str) -> Result<Vec<CallRecord>> {
        self.calls_where("WHERE chat = ?1", params![chat])
    }

    /// How many calls the log holds, which is all a chat row needs to know whether to show the
    /// call-history entry.
    pub fn call_count(&self) -> Result<i64> {
        self.connection
            .query_row("SELECT COUNT(*) FROM calls", [], |row| row.get(0))
    }

    /// Moves one chat's calls onto another id, the way a privacy-id chat moves onto its number.
    pub(super) fn move_calls(&self, from: &str, to: &str) -> Result<()> {
        self.connection.execute(
            "UPDATE OR REPLACE calls SET chat = ?2 WHERE chat = ?1",
            params![from, to],
        )?;
        Ok(())
    }

    fn calls_where(
        &self,
        filter: &str,
        arguments: impl rusqlite::Params,
    ) -> Result<Vec<CallRecord>> {
        let sql = format!(
            "SELECT id, chat, started_at, ended_at, direction, media, status, duration FROM calls {filter} ORDER BY started_at DESC, id"
        );
        let mut statement = self.connection.prepare(&sql)?;
        let rows = statement.query_map(arguments, |row| {
            Ok(CallRecord {
                id: row.get(0)?,
                chat: row.get(1)?,
                started_at: row.get(2)?,
                ended_at: row.get(3)?,
                direction: direction_from_key(row.get::<_, String>(4)?.as_str()),
                media: media_from_key(row.get::<_, String>(5)?.as_str()),
                status: status_from_key(row.get::<_, String>(6)?.as_str()),
                duration: row.get::<_, i64>(7)?.max(0) as u64,
            })
        })?;
        rows.collect()
    }
}

/// The stored spelling of a direction. Kept beside its reader so the two cannot drift, and spelled
/// out rather than derived so an enum rename cannot silently reinterpret old rows.
fn direction_key(direction: CallDirection) -> &'static str {
    match direction {
        CallDirection::Incoming => "incoming",
        CallDirection::Outgoing => "outgoing",
    }
}

fn direction_from_key(key: &str) -> CallDirection {
    match key {
        "incoming" => CallDirection::Incoming,
        _ => CallDirection::Outgoing,
    }
}

fn media_key(media: CallMedia) -> &'static str {
    match media {
        CallMedia::Voice => "voice",
        CallMedia::Video => "video",
    }
}

fn media_from_key(key: &str) -> CallMedia {
    match key {
        "video" => CallMedia::Video,
        _ => CallMedia::Voice,
    }
}

fn status_key(status: CallStatus) -> &'static str {
    match status {
        CallStatus::Answered => "answered",
        CallStatus::AnsweredElsewhere => "answered_elsewhere",
        CallStatus::Missed => "missed",
        CallStatus::Declined => "declined",
        CallStatus::Busy => "busy",
        CallStatus::Failed => "failed",
        CallStatus::NoAnswer => "no_answer",
        CallStatus::ConnectionLost => "connection_lost",
    }
}

/// A status this build does not know, from a file written by a newer one, reads as `Failed`: the
/// call happened and did not end well, which is closer than claiming it was answered.
fn status_from_key(key: &str) -> CallStatus {
    match key {
        "answered" => CallStatus::Answered,
        "answered_elsewhere" => CallStatus::AnsweredElsewhere,
        "missed" => CallStatus::Missed,
        "declined" => CallStatus::Declined,
        "busy" => CallStatus::Busy,
        "no_answer" => CallStatus::NoAnswer,
        "connection_lost" => CallStatus::ConnectionLost,
        _ => CallStatus::Failed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(id: &str, chat: &str, started_at: i64, status: CallStatus) -> CallRecord {
        CallRecord {
            id: id.to_owned(),
            chat: chat.to_owned(),
            started_at,
            ended_at: started_at + 90,
            direction: CallDirection::Outgoing,
            media: CallMedia::Voice,
            status,
            duration: 60,
        }
    }

    #[test]
    fn a_call_is_filed_with_its_chat_and_read_back_whole() {
        let archive = Archive::in_memory().unwrap();
        let call = CallRecord {
            id: "abc".into(),
            chat: "15551234567@s.whatsapp.net".into(),
            started_at: 1_700_000_000,
            ended_at: 1_700_000_221,
            direction: CallDirection::Incoming,
            media: CallMedia::Video,
            status: CallStatus::Missed,
            duration: 0,
        };
        archive.save_call(&call).unwrap();
        assert_eq!(archive.calls().unwrap(), vec![call.clone()]);
        assert_eq!(
            archive
                .calls_for_chat("15551234567@s.whatsapp.net")
                .unwrap()
                .len(),
            1
        );
        assert!(
            archive
                .calls_for_chat("other@s.whatsapp.net")
                .unwrap()
                .is_empty()
        );
        assert_eq!(archive.call_count().unwrap(), 1);
    }

    #[test]
    fn the_newest_call_comes_first_and_one_call_is_one_row() {
        let archive = Archive::in_memory().unwrap();
        archive
            .save_call(&record(
                "one",
                "a@s.whatsapp.net",
                100,
                CallStatus::Answered,
            ))
            .unwrap();
        archive
            .save_call(&record("two", "a@s.whatsapp.net", 200, CallStatus::Missed))
            .unwrap();
        // The same call resolving twice stays one row, with the later reading.
        let mut again = record("one", "a@s.whatsapp.net", 100, CallStatus::Answered);
        again.status = CallStatus::Declined;
        archive.save_call(&again).unwrap();
        let calls = archive.calls().unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].id, "two");
        assert_eq!(calls[1].status, CallStatus::Declined);
    }

    #[test]
    fn every_status_survives_a_round_trip() {
        for status in [
            CallStatus::Answered,
            CallStatus::AnsweredElsewhere,
            CallStatus::Missed,
            CallStatus::Declined,
            CallStatus::Busy,
            CallStatus::Failed,
            CallStatus::NoAnswer,
            CallStatus::ConnectionLost,
        ] {
            assert_eq!(status_from_key(status_key(status)), status);
            assert_eq!(
                media_from_key(media_key(CallMedia::Voice)),
                CallMedia::Voice
            );
            assert_eq!(
                direction_from_key(direction_key(CallDirection::Incoming)),
                CallDirection::Incoming
            );
        }
        // A file from a newer build: an unknown status is not read as an answered call.
        assert_eq!(status_from_key("something_new"), CallStatus::Failed);
    }

    #[test]
    fn a_chat_moved_onto_its_number_takes_its_calls_along() {
        let archive = Archive::in_memory().unwrap();
        archive
            .save_call(&record("one", "123@lid", 100, CallStatus::Answered))
            .unwrap();
        archive
            .move_calls("123@lid", "15551234567@s.whatsapp.net")
            .unwrap();
        assert!(archive.calls_for_chat("123@lid").unwrap().is_empty());
        assert_eq!(
            archive
                .calls_for_chat("15551234567@s.whatsapp.net")
                .unwrap()
                .len(),
            1
        );
    }
}
