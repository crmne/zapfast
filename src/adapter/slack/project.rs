//! Turns Slack events and conversations into the shared models.
//!
//! Slack's Web API answers with JSON; this module is the one place that
//! knows what those fields mean. Channels become chats, thread replies become
//! child chats whose peer carries the thread timestamp, and files become the
//! usual media contents.

use std::collections::HashMap;

use serde_json::Value;

use crate::account::{
    AccountId, CallAccess, Capabilities, GifSource, ReactionStyle, StickerAccess,
};
use crate::model::{
    Chat, ChatId, ChatKind, Content, Delivery, Media, MediaState, Message, Quoted, Reaction,
};

/// The peer for a channel: its Slack id.
pub fn chat_id(account: AccountId, channel: &str) -> ChatId {
    ChatId::new(account, channel)
}

/// The peer for one thread: the channel and the thread's timestamp.
pub fn thread_id(account: AccountId, channel: &str, thread: &str) -> ChatId {
    ChatId::new(account, format!("{channel}:{thread}"))
}

/// Splits a peer into its channel and optional thread timestamp.
pub fn parse_chat(chat: &ChatId) -> Option<(&str, Option<&str>)> {
    let (channel, thread) = match chat.peer().split_once(':') {
        Some((channel, thread)) => (channel, Some(thread)),
        None => (chat.peer(), None),
    };
    if channel.is_empty() || thread.is_some_and(str::is_empty) {
        return None;
    }
    Some((channel, thread))
}

/// What a Slack account can do here.
pub fn capabilities() -> Capabilities {
    Capabilities {
        text: true,
        files: true,
        // Slack has no voice messages or video notes.
        voice_notes: false,
        video_notes: false,
        // Custom emoji stay as their shortcodes for now.
        custom_emoji: false,
        reactions: ReactionStyle::Counts,
        gif: GifSource::None,
        stickers: StickerAccess::None,
        threads: true,
        spaces: false,
        // No typing indicator, read receipts, polls, huddles, or calls come
        // through the bot API.
        typing: false,
        read_receipts: false,
        edit: true,
        delete_for_everyone: true,
        polls: false,
        calls: CallAccess::None,
    }
}

/// One conversation from `conversations.list` as a chat.
pub fn channel(account: AccountId, raw: &Value, users: &HashMap<String, String>) -> Option<Chat> {
    if raw.get("is_archived").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    let id = raw.get("id").and_then(Value::as_str)?;
    let direct = raw.get("is_im").and_then(Value::as_bool) == Some(true);
    let name = if direct {
        raw.get("user")
            .and_then(Value::as_str)
            .and_then(|user| users.get(user).cloned())
            .unwrap_or_else(|| id.to_owned())
    } else {
        raw.get("name")
            .and_then(Value::as_str)
            .map(|name| format!("#{name}"))
            .unwrap_or_else(|| id.to_owned())
    };
    let mut chat = Chat::new(chat_id(account, id), name);
    chat.kind = if direct {
        ChatKind::Direct
    } else {
        ChatKind::Group
    };
    Some(chat)
}

/// A thread as its own chat row, under the channel.
pub fn thread_chat(account: AccountId, channel: &str, thread: &str, name: String) -> Chat {
    let mut chat = Chat::new(thread_id(account, channel, thread), name);
    chat.kind = ChatKind::Group;
    chat.parent = Some(chat_id(account, channel));
    chat
}

fn timestamp(ts: &str) -> Option<i64> {
    ts.parse::<f64>().ok().map(|seconds| seconds as i64)
}

/// A message event as a message. Events with neither text nor files (joins,
/// topic changes) are not messages.
#[allow(clippy::too_many_arguments)]
pub fn message(
    account: AccountId,
    event: &Value,
    me: &str,
    sender_name: Option<String>,
    quoted: Option<Quoted>,
    reactions: Vec<Reaction>,
) -> Option<Message> {
    let ts = event.get("ts").and_then(Value::as_str)?;
    let channel = event.get("channel").and_then(Value::as_str)?;
    let timestamp = timestamp(ts)?;
    let thread = event
        .get("thread_ts")
        .and_then(Value::as_str)
        .filter(|thread| *thread != ts);
    let chat = match thread {
        Some(thread) => thread_id(account, channel, thread),
        None => chat_id(account, channel),
    };
    let sender = event
        .get("user")
        .and_then(Value::as_str)
        .or_else(|| event.get("bot_id").and_then(Value::as_str))
        .unwrap_or("slack")
        .to_owned();
    let text = event.get("text").and_then(Value::as_str).unwrap_or("");
    let content = match event.get("files").and_then(Value::as_array) {
        Some(files) if !files.is_empty() => file_content(&files[0], text),
        _ if !text.is_empty() => Content::Text {
            text: text.to_owned(),
            preview: None,
        },
        _ => return None,
    };
    let from_me = sender == me;
    Some(Message {
        id: ts.to_owned(),
        chat,
        sender,
        sender_name,
        from_me,
        timestamp,
        content,
        status: if from_me {
            Delivery::Sent
        } else {
            Delivery::None
        },
        delivered_at: None,
        read_at: None,
        quoted,
        reactions,
        edited: false,
        mentions: Vec::new(),
        forwarded: false,
        thumbnail: None,
    })
}

fn file_content(file: &Value, caption: &str) -> Content {
    let mime = file
        .get("mimetype")
        .and_then(Value::as_str)
        .unwrap_or("application/octet-stream")
        .to_owned();
    let size = file.get("size").and_then(Value::as_u64).unwrap_or(0);
    let name = file
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("file")
        .to_owned();
    let media = Media {
        mime: mime.clone(),
        size,
        width: None,
        height: None,
        path: None,
        state: MediaState::Idle,
    };
    if mime.starts_with("image/") {
        Content::Image {
            caption: (!caption.is_empty()).then(|| caption.to_owned()),
            media,
        }
    } else if mime.starts_with("video/") {
        Content::Video {
            caption: (!caption.is_empty()).then(|| caption.to_owned()),
            media,
            seconds: None,
            gif: false,
            note: false,
        }
    } else if mime.starts_with("audio/") {
        Content::Audio {
            media,
            seconds: None,
            voice_note: mime.contains("ogg") || mime.contains("opus"),
            waveform: Vec::new(),
        }
    } else {
        Content::Document {
            media,
            file_name: name,
            caption: (!caption.is_empty()).then(|| caption.to_owned()),
            pages: None,
        }
    }
}

/// Slack names reactions as shortcodes; the window shows one emoji.
pub fn reaction_name(name: &str) -> String {
    let emoji = match name {
        "thumbsup" | "+1" => "\u{1f44d}",
        "thumbsdown" | "-1" => "\u{1f44e}",
        "heart" => "\u{2764}\u{fe0f}",
        "joy" => "\u{1f602}",
        "tada" => "\u{1f389}",
        "eyes" => "\u{1f440}",
        "fire" => "\u{1f525}",
        "pray" => "\u{1f64f}",
        "clap" => "\u{1f44f}",
        "laughing" => "\u{1f606}",
        "sob" => "\u{1f62d}",
        "rage" => "\u{1f621}",
        "thinking_face" => "\u{1f914}",
        "100" => "\u{1f4af}",
        "ok_hand" => "\u{1f44c}",
        "raised_hands" => "\u{1f64c}",
        "muscle" => "\u{1f4aa}",
        "wink" => "\u{1f609}",
        "sunglasses" => "\u{1f60e}",
        "partying_face" => "\u{1f973}",
        "heart_eyes" => "\u{1f60d}",
        "kissing_heart" => "\u{1f618}",
        "scream" => "\u{1f631}",
        _ => return format!(":{name}:"),
    };
    emoji.to_owned()
}

/// Groups one reaction row per emoji, newest sender list included.
pub fn reactions(rows: &[(String, String)], me: &str) -> Vec<Reaction> {
    let mut grouped: Vec<(String, u32, bool)> = Vec::new();
    for (emoji, sender) in rows {
        match grouped.iter_mut().find(|(known, _, _)| known == emoji) {
            Some((_, count, from_me)) => {
                *count += 1;
                *from_me |= sender == me;
            }
            None => grouped.push((emoji.clone(), 1, sender == me)),
        }
    }
    grouped
        .into_iter()
        .map(|(emoji, count, from_me)| Reaction {
            sender: String::new(),
            from_me,
            emoji,
            count,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn account() -> AccountId {
        AccountId(2)
    }

    #[test]
    fn peers_round_trip_through_their_stored_form() {
        let channel = chat_id(account(), "C0123");
        assert_eq!(parse_chat(&channel), Some(("C0123", None)));
        let thread = thread_id(account(), "C0123", "1712345678.000100");
        assert_eq!(
            parse_chat(&thread),
            Some(("C0123", Some("1712345678.000100")))
        );
        assert_eq!(thread.peer(), "C0123:1712345678.000100");
        assert_eq!(parse_chat(&ChatId::new(account(), "C:")), None);
        assert_eq!(parse_chat(&ChatId::new(account(), "")), None);
    }

    #[test]
    fn a_channel_message_projects_text_and_sender() {
        let event = json!({
            "type": "message",
            "channel": "C0123",
            "user": "U1",
            "text": "hello",
            "ts": "1712345678.000100"
        });
        let message = message(
            account(),
            &event,
            "U2",
            Some("Ada".to_owned()),
            None,
            Vec::new(),
        )
        .unwrap();
        assert_eq!(message.id, "1712345678.000100");
        assert_eq!(message.chat, chat_id(account(), "C0123"));
        assert_eq!(message.sender, "U1");
        assert_eq!(message.sender_name.as_deref(), Some("Ada"));
        assert!(!message.from_me);
        assert_eq!(message.timestamp, 1_712_345_678);
        assert!(matches!(
            &message.content,
            Content::Text { text, .. } if text == "hello"
        ));
    }

    #[test]
    fn our_own_message_is_sent() {
        let event = json!({
            "type": "message",
            "channel": "C0123",
            "user": "U2",
            "text": "hi",
            "ts": "1712345678.000200"
        });
        let message = message(account(), &event, "U2", None, None, Vec::new()).unwrap();
        assert!(message.from_me);
        assert_eq!(message.status, Delivery::Sent);
    }

    #[test]
    fn a_thread_reply_opens_its_thread_row() {
        let event = json!({
            "type": "message",
            "channel": "C0123",
            "user": "U1",
            "text": "in thread",
            "ts": "1712345679.000100",
            "thread_ts": "1712345678.000100"
        });
        let message = message(account(), &event, "U2", None, None, Vec::new()).unwrap();
        assert_eq!(
            message.chat,
            thread_id(account(), "C0123", "1712345678.000100")
        );
    }

    #[test]
    fn a_file_projects_as_media_and_keeps_the_caption() {
        let event = json!({
            "type": "message",
            "channel": "C0123",
            "user": "U1",
            "text": "look",
            "ts": "1712345678.000300",
            "files": [{
                "mimetype": "image/png",
                "size": 2048,
                "name": "shot.png"
            }]
        });
        let message = message(account(), &event, "U2", None, None, Vec::new()).unwrap();
        match &message.content {
            Content::Image { caption, media } => {
                assert_eq!(caption.as_deref(), Some("look"));
                assert_eq!(media.mime, "image/png");
                assert_eq!(media.size, 2048);
                assert!(media.path.is_none());
            }
            other => panic!("expected an image, got {other:?}"),
        }
    }

    #[test]
    fn an_audio_file_is_a_voice_note_only_when_it_is_ogg() {
        let event = json!({
            "type": "message",
            "channel": "D0123",
            "user": "U1",
            "ts": "1712345678.000400",
            "files": [{
                "mimetype": "audio/ogg",
                "size": 512,
                "name": "clip.ogg"
            }]
        });
        let message = message(account(), &event, "U2", None, None, Vec::new()).unwrap();
        assert!(matches!(
            &message.content,
            Content::Audio {
                voice_note: true,
                ..
            }
        ));
    }

    #[test]
    fn system_messages_are_not_messages() {
        let event = json!({
            "type": "message",
            "subtype": "channel_join",
            "channel": "C0123",
            "user": "U1",
            "ts": "1712345678.000500"
        });
        assert!(message(account(), &event, "U2", None, None, Vec::new()).is_none());
    }

    #[test]
    fn conversations_project_into_rows() {
        let users = HashMap::from([("U1".to_owned(), "Ada".to_owned())]);
        let general = channel(account(), &json!({"id": "C1", "name": "general"}), &users).unwrap();
        assert_eq!(general.name, "#general");
        assert_eq!(general.kind, ChatKind::Group);
        let direct = channel(
            account(),
            &json!({"id": "D1", "is_im": true, "user": "U1"}),
            &users,
        )
        .unwrap();
        assert_eq!(direct.name, "Ada");
        assert_eq!(direct.kind, ChatKind::Direct);
        assert!(
            channel(
                account(),
                &json!({"id": "C2", "name": "old", "is_archived": true}),
                &users
            )
            .is_none()
        );
    }

    #[test]
    fn threads_hang_under_their_channel() {
        let thread = thread_chat(
            account(),
            "C1",
            "1712345678.000100",
            "About lunch".to_owned(),
        );
        assert_eq!(thread.parent, Some(chat_id(account(), "C1")));
    }

    #[test]
    fn reaction_names_map_to_emoji() {
        assert_eq!(reaction_name("thumbsup"), "\u{1f44d}");
        assert_eq!(reaction_name("+1"), "\u{1f44d}");
        assert_eq!(reaction_name("shipit"), ":shipit:");
    }

    #[test]
    fn reactions_group_by_emoji_with_counts() {
        let rows = vec![
            ("\u{1f44d}".to_owned(), "U1".to_owned()),
            ("\u{1f44d}".to_owned(), "U2".to_owned()),
            ("\u{1f389}".to_owned(), "U1".to_owned()),
        ];
        let reactions = reactions(&rows, "U2");
        assert_eq!(reactions.len(), 2);
        assert_eq!(reactions[0].emoji, "\u{1f44d}");
        assert_eq!(reactions[0].count, 2);
        assert!(reactions[0].from_me);
        assert_eq!(reactions[1].count, 1);
        assert!(!reactions[1].from_me);
    }

    #[test]
    fn capabilities_stay_honest() {
        let capabilities = capabilities();
        assert!(capabilities.text && capabilities.files && capabilities.threads);
        assert!(capabilities.edit && capabilities.delete_for_everyone);
        assert!(!capabilities.voice_notes && !capabilities.video_notes);
        assert!(!capabilities.typing && !capabilities.read_receipts);
        assert!(!capabilities.polls && !capabilities.spaces);
        assert_eq!(capabilities.reactions, ReactionStyle::Counts);
        assert_eq!(capabilities.gif, GifSource::None);
        assert_eq!(capabilities.stickers, StickerAccess::None);
        assert_eq!(capabilities.calls, CallAccess::None);
    }
}
