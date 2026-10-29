//! Call entries: the calls of a chat as messages of their own kind, which is
//! how WhatsApp files them too. They come from the phone's call-log messages
//! in history and from the call records ZapFast files (the phone's call log
//! and the calls it took part in, under ids that start with
//! [`crate::model::CALL_ENTRY_PREFIX`]).

use super::{Archive, Result, params};
use crate::model::{CALL_ENTRY_WINDOW, Message};

/// How many entries the Calls page lists.
pub const CALL_LOG_LIMIT: usize = 500;

impl Archive {
    /// Whether the chat already has an entry for a call placed by the same
    /// side around `timestamp`, other than `except`: the phone's own message
    /// for a call this side recorded, or the other way round.
    pub fn call_entry_near(
        &self,
        chat: &str,
        timestamp: i64,
        from_me: bool,
        except: &str,
    ) -> Result<bool> {
        self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM messages
             WHERE chat = ?1 AND from_me = ?2 AND id <> ?3
             AND json_extract(content, '$.kind') = 'call'
             AND timestamp BETWEEN ?4 - ?5 AND ?4 + ?5)",
            params![chat, from_me, except, timestamp, CALL_ENTRY_WINDOW],
            |row| row.get(0),
        )
    }

    /// ZapFast's own entries for a call the phone has now filed itself: the
    /// same side's local entries around `timestamp`.
    pub fn local_call_twins(
        &self,
        chat: &str,
        timestamp: i64,
        from_me: bool,
    ) -> Result<Vec<String>> {
        let mut statement = self.connection.prepare(
            "SELECT id FROM messages
             WHERE chat = ?1 AND from_me = ?2 AND substr(id, 1, length(?3)) = ?3
             AND json_extract(content, '$.kind') = 'call'
             AND timestamp BETWEEN ?4 - ?5 AND ?4 + ?5",
        )?;
        let ids = statement.query_map(
            params![
                chat,
                from_me,
                crate::model::CALL_ENTRY_PREFIX,
                timestamp,
                CALL_ENTRY_WINDOW
            ],
            |row| row.get(0),
        )?;
        ids.collect()
    }

    /// The account's call entries, newest first.
    pub fn call_log(&self, limit: usize) -> Result<Vec<Message>> {
        let mut statement = self.connection.prepare(
            "SELECT chat, id FROM messages
             WHERE json_extract(content, '$.kind') = 'call'
             ORDER BY timestamp DESC, rowid DESC LIMIT ?1",
        )?;
        let keys: Vec<(String, String)> = statement
            .query_map(params![limit as i64], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<Result<_>>()?;
        let mut entries = Vec::with_capacity(keys.len());
        for (chat, id) in keys {
            if let Some(message) = self.message(&chat, &id)? {
                entries.push(Message { chat, ..message });
            }
        }
        Ok(entries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{CALL_ENTRY_PREFIX, CallStatus, Content, Delivery};

    fn entry(chat: &str, id: &str, from_me: bool, timestamp: i64) -> Message {
        Message {
            id: id.to_owned(),
            chat: chat.to_owned(),
            sender: chat.to_owned(),
            sender_name: None,
            from_me,
            timestamp,
            history_order: None,
            content: Content::Call {
                video: false,
                status: CallStatus::Missed,
                duration: 0,
                group: false,
                participants: None,
            },
            status: Delivery::None,
            delivered_at: None,
            read_at: None,
            quoted: None,
            reactions: Vec::new(),
            edited: false,
            mentions: Vec::new(),
            forwarded: false,
            thumbnail: None,
        }
    }

    #[test]
    fn a_call_recorded_twice_is_found_near_its_other_entry() {
        let archive = Archive::in_memory().unwrap();
        let chat = "15550001111@s.whatsapp.net";
        archive.ensure_chat(chat, "Ada").unwrap();
        archive
            .insert_message(&entry(chat, "PHONE-CALL-LOG", false, 1_000), None)
            .unwrap();
        let ours = format!("{CALL_ENTRY_PREFIX}call-1");
        assert!(archive.call_entry_near(chat, 1_090, false, &ours).unwrap());
        assert!(
            !archive.call_entry_near(chat, 1_090, true, &ours).unwrap(),
            "a call the other way is another call"
        );
        assert!(
            !archive.call_entry_near(chat, 2_000, false, &ours).unwrap(),
            "a call minutes later is another call"
        );
        assert!(
            !archive
                .call_entry_near(chat, 1_000, false, "PHONE-CALL-LOG")
                .unwrap(),
            "an entry is not near itself"
        );
    }

    #[test]
    fn the_call_log_lists_every_chats_calls_newest_first() {
        let archive = Archive::in_memory().unwrap();
        let ada = "15550001111@s.whatsapp.net";
        let grace = "15550002222@s.whatsapp.net";
        archive.ensure_chat(ada, "Ada").unwrap();
        archive.ensure_chat(grace, "Grace").unwrap();
        archive
            .insert_message(&entry(ada, "zapfast-call-a", true, 100), None)
            .unwrap();
        archive
            .insert_message(&entry(grace, "zapfast-call-b", false, 300), None)
            .unwrap();
        let mut text = entry(ada, "TEXT", false, 200);
        text.content = Content::Text {
            text: "Hello".into(),
            preview: None,
        };
        archive.insert_message(&text, None).unwrap();
        let log = archive.call_log(CALL_LOG_LIMIT).unwrap();
        let ids: Vec<&str> = log.iter().map(|entry| entry.id.as_str()).collect();
        assert_eq!(ids, ["zapfast-call-b", "zapfast-call-a"]);
        assert_eq!(log[0].chat, grace);
    }

    #[test]
    fn the_phones_own_entry_finds_ours_for_the_same_call() {
        let archive = Archive::in_memory().unwrap();
        let chat = "15550001111@s.whatsapp.net";
        archive.ensure_chat(chat, "Ada").unwrap();
        let ours = format!("{CALL_ENTRY_PREFIX}call-1");
        archive
            .insert_message(&entry(chat, &ours, true, 1_000), None)
            .unwrap();
        archive
            .insert_message(&entry(chat, "PHONE-CALL-LOG", true, 1_030), None)
            .unwrap();
        assert_eq!(archive.local_call_twins(chat, 1_030, true).unwrap(), [ours]);
        assert!(
            archive
                .local_call_twins(chat, 1_030, false)
                .unwrap()
                .is_empty()
        );
        assert!(
            archive
                .local_call_twins(chat, 5_000, true)
                .unwrap()
                .is_empty()
        );
    }

    /// An entry filed before a count was kept reads back without one, so no migration is needed:
    /// the content is JSON and the field is simply absent.
    #[test]
    fn an_entry_filed_before_the_count_reads_back_without_one() {
        let archive = Archive::in_memory().unwrap();
        let chat = "15550001111@s.whatsapp.net";
        archive.ensure_chat(chat, "Ada").unwrap();
        archive
            .connection
            .execute(
                "INSERT INTO messages (chat, id, sender, from_me, timestamp, content)
                 VALUES (?1, ?2, ?3, 0, 100, ?4)",
                params![
                    chat,
                    "PHONE-CALL-LOG",
                    chat,
                    r#"{"kind":"call","video":true,"status":"missed","duration":0,"group":true}"#
                ],
            )
            .unwrap();

        let log = archive.call_log(10).unwrap();
        assert_eq!(log.len(), 1);
        assert!(
            matches!(
                log[0].content,
                Content::Call {
                    group: true,
                    participants: None,
                    ..
                }
            ),
            "an older entry keeps its call and gains no count: {:?}",
            log[0].content
        );
        // And it is not rewritten with one: the field is left out of what is stored.
        let stored = serde_json::to_string(&log[0].content).unwrap();
        assert!(!stored.contains("participants"), "{stored}");
    }

    #[test]
    fn our_own_call_entries_never_wait_for_a_read_receipt() {
        let archive = Archive::in_memory().unwrap();
        let chat = "15550001111@s.whatsapp.net";
        archive.ensure_chat(chat, "Ada").unwrap();
        archive
            .insert_message(&entry(chat, "zapfast-call-1", false, 100), None)
            .unwrap();
        archive
            .insert_message(&entry(chat, "PHONE-CALL-LOG", false, 200), None)
            .unwrap();
        let unread = archive.unread_incoming(chat, 10).unwrap();
        let ids: Vec<&str> = unread.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(ids, ["PHONE-CALL-LOG"]);
    }
}
