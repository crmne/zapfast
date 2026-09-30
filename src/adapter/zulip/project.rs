//! Projects Zulip objects into ZapFast's models.
//!
//! The runtime keeps the protocol pieces (the register queue and the event
//! long poll) and hands raw JSON here, so the projections stay pure and
//! testable without a server.

use serde_json::Value;

use crate::account::AccountId;
use crate::model::{
    Chat, ChatId, ChatKind, Content, Delivery, Media, MediaState, Message, Reaction,
};

/// Where a chat's peer string points.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// The stream row, which holds the general topic.
    Stream { stream: i64 },
    /// One topic inside a stream.
    Topic { stream: i64, topic: String },
    /// A direct message conversation with these users, without us.
    Dm { users: Vec<i64> },
}

/// The topic a stream row sends into and reads. Zulip used to call this the
/// default topic, and servers still accept it.
pub const GENERAL_TOPIC: &str = "general";

/// The peer string of a stream row.
pub fn stream_id(account: AccountId, stream: i64) -> ChatId {
    ChatId::new(account, format!("s{stream}"))
}

/// The peer string of one topic inside a stream. The topic may contain any
/// characters, including a colon; only the first colon separates it from the
/// stream id.
pub fn topic_id(account: AccountId, stream: i64, topic: &str) -> ChatId {
    ChatId::new(account, format!("t{stream}:{topic}"))
}

/// The peer string of a direct message conversation. The user ids are sorted
/// and deduplicated, so every participant computes the same string.
pub fn dm_id(account: AccountId, users: &[i64]) -> ChatId {
    let mut users: Vec<i64> = users.iter().copied().filter(|user| *user != 0).collect();
    users.sort_unstable();
    users.dedup();
    let users: Vec<String> = users.into_iter().map(|user| user.to_string()).collect();
    ChatId::new(account, format!("d{}", users.join(",")))
}

/// Reads a peer string back into its target.
pub fn parse(chat: &ChatId) -> Option<Target> {
    let peer = chat.peer();
    if let Some(rest) = peer.strip_prefix('s') {
        return Some(Target::Stream {
            stream: rest.parse().ok()?,
        });
    }
    if let Some(rest) = peer.strip_prefix('t') {
        let (stream, topic) = rest.split_once(':')?;
        return Some(Target::Topic {
            stream: stream.parse().ok()?,
            topic: topic.to_owned(),
        });
    }
    if let Some(rest) = peer.strip_prefix('d') {
        if rest.is_empty() {
            return None;
        }
        let users = rest
            .split(',')
            .map(str::parse)
            .collect::<Result<Vec<i64>, _>>()
            .ok()?;
        return Some(Target::Dm { users });
    }
    None
}

/// The chat a message object belongs to. Stream messages go to their topic,
/// except the general topic, which stays on the stream row. Direct messages
/// go to the conversation with everyone but us.
pub fn message_chat(account: AccountId, object: &Value, me: i64) -> Option<ChatId> {
    match object.get("type").and_then(Value::as_str) {
        Some("stream") => {
            let stream = object.get("stream_id").and_then(Value::as_i64)?;
            let subject = object.get("subject").and_then(Value::as_str).unwrap_or("");
            if subject.is_empty() || subject == GENERAL_TOPIC {
                Some(stream_id(account, stream))
            } else {
                Some(topic_id(account, stream, subject))
            }
        }
        Some("private") => {
            let users = object
                .get("display_recipient")
                .and_then(Value::as_array)?
                .iter()
                .filter_map(|user| user.get("id").and_then(Value::as_i64))
                .filter(|user| *user != me);
            let mut users: Vec<i64> = users.collect();
            if users.is_empty() {
                users.push(me);
            }
            Some(dm_id(account, &users))
        }
        _ => None,
    }
}

/// What a Zulip account can do here. Everything unlisted is either absent in
/// the network or waits for its own work: GIF search, custom emoji images,
/// sticker packs, voice-note flags, and calls.
pub fn capabilities() -> crate::account::Capabilities {
    crate::account::Capabilities {
        text: true,
        files: true,
        voice_notes: false,
        video_notes: false,
        custom_emoji: false,
        reactions: crate::account::ReactionStyle::Counts,
        gif: crate::account::GifSource::None,
        stickers: crate::account::StickerAccess::None,
        threads: true,
        spaces: false,
        typing: true,
        read_receipts: false,
        edit: true,
        delete_for_everyone: true,
        polls: false,
        calls: crate::account::CallAccess::None,
    }
}

/// A stream as a parent row. Topics are rows of their own beside it.
pub fn stream_chat(account: AccountId, subscription: &Value) -> Option<Chat> {
    let id = subscription.get("stream_id").and_then(Value::as_i64)?;
    let name = subscription.get("name").and_then(Value::as_str)?;
    let mut chat = Chat::new(stream_id(account, id), name.to_owned());
    chat.kind = ChatKind::Group;
    Some(chat)
}

/// One topic inside a stream, as a child row whose subtitle names the stream.
pub fn topic_chat(account: AccountId, stream: i64, topic: &str) -> Chat {
    let name = if topic.is_empty() {
        "(no topic)".to_owned()
    } else {
        topic.to_owned()
    };
    let mut chat = Chat::new(topic_id(account, stream, topic), name);
    chat.kind = ChatKind::Group;
    chat.parent = Some(stream_id(account, stream));
    chat
}

/// A direct message conversation row, named after the people in it.
pub fn dm_chat(account: AccountId, users: &[i64], names: &[String]) -> Chat {
    let mut chat = Chat::new(dm_id(account, users), names.join(", "));
    chat.kind = ChatKind::Direct;
    chat
}

/// The attachment of a message, when it has one.
pub fn attachment(object: &Value) -> Option<&Value> {
    object.get("attachments")?.as_array()?.first()
}

/// The URL to download a message's first attachment from, relative to the
/// server. Zulip serves it to the signed-in user only.
pub fn attachment_url(object: &Value) -> Option<String> {
    let attachment = attachment(object)?;
    let url = attachment.get("url").and_then(Value::as_str)?;
    Some(url.to_owned())
}

/// One message object as a bubble.
pub fn message(
    account: AccountId,
    object: &Value,
    me: i64,
    sender_name: Option<String>,
    reactions: Vec<Reaction>,
) -> Option<Message> {
    let id = object.get("id").and_then(Value::as_i64)?.to_string();
    let chat = message_chat(account, object, me)?;
    let sender_id = object.get("sender_id").and_then(Value::as_i64).unwrap_or(0);
    let from_me = sender_id == me;
    let timestamp = object.get("timestamp").and_then(Value::as_i64).unwrap_or(0);
    let text = object.get("content").and_then(Value::as_str).unwrap_or("");
    let content = match attachment(object) {
        Some(attachment) => attachment_content(attachment, text),
        None if !text.is_empty() => Content::Text {
            text: text.to_owned(),
            preview: None,
        },
        None if !reactions.is_empty() => Content::Text {
            text: String::new(),
            preview: None,
        },
        None => return None,
    };
    Some(Message {
        id,
        chat,
        sender: sender_id.to_string(),
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
        quoted: None,
        reactions,
        edited: object
            .get("last_edit_timestamp")
            .and_then(Value::as_i64)
            .is_some(),
        mentions: Vec::new(),
        forwarded: false,
        thumbnail: None,
    })
}

/// The reactions a message object carries, as `(emoji, user id)` rows for the
/// runtime's reaction map.
pub fn reaction_rows(object: &Value) -> Vec<(String, String)> {
    let Some(reactions) = object.get("reactions").and_then(Value::as_array) else {
        return Vec::new();
    };
    reactions
        .iter()
        .filter_map(|reaction| {
            let name = reaction.get("emoji_name").and_then(Value::as_str)?;
            let user = reaction.get("user_id").and_then(Value::as_i64).unwrap_or(0);
            Some((reaction_name(name), user.to_string()))
        })
        .collect()
}

/// A reaction event's emoji as `(emoji, user id)` rows.
pub fn reaction_event_rows(event: &Value) -> Option<(String, String)> {
    let name = event.get("emoji_name").and_then(Value::as_str)?;
    let user = event.get("user_id").and_then(Value::as_i64).unwrap_or(0);
    Some((reaction_name(name), user.to_string()))
}

/// Groups reaction rows by emoji, counting people, in first-seen order.
pub fn reactions(rows: &[(String, String)], me: i64) -> Vec<Reaction> {
    let mut grouped: Vec<Reaction> = Vec::new();
    for (emoji, sender) in rows {
        match grouped.iter_mut().find(|reaction| &reaction.emoji == emoji) {
            Some(reaction) => {
                reaction.count += 1;
                if sender == &me.to_string() {
                    reaction.from_me = true;
                }
            }
            None => grouped.push(Reaction {
                sender: String::new(),
                from_me: sender == &me.to_string(),
                emoji: emoji.clone(),
                count: 1,
            }),
        }
    }
    grouped
}

/// The emoji to show for a Zulip reaction name. Unicode reactions already
/// carry their character; named ones map to the common set, and anything
/// else keeps its `:name:` form.
pub fn reaction_name(name: &str) -> String {
    if !name.is_ascii() {
        return name.to_owned();
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '+')
    {
        return name.to_owned();
    }
    match name {
        "thumbs_up" | "+1" => "👍",
        "thumbs_down" | "-1" => "👎",
        "heart" => "❤️",
        "joy" => "😂",
        "tada" => "🎉",
        "eyes" => "👀",
        "fire" => "🔥",
        "pray" => "🙏",
        "clap" => "👏",
        "laughing" => "😆",
        "sob" => "😭",
        "rage" => "😡",
        "thinking" | "thinking_face" => "🤔",
        "100" => "💯",
        "ok_hand" => "👌",
        "raised_hands" => "🙌",
        "muscle" => "💪",
        "wink" => "😉",
        "sunglasses" => "😎",
        "star" => "⭐",
        "rocket" => "🚀",
        "check" => "✔️",
        "smile" => "🙂",
        other => return format!(":{other}:"),
    }
    .to_owned()
}

/// The emoji name to send for a picker emoji: minus any surrounding colons.
pub fn reaction_key(emoji: &str) -> String {
    emoji.trim_matches(':').to_owned()
}

/// The parts of an event the runtime acts on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StoredEvent {
    /// A new message; the object travels beside it.
    Message,
    /// A reaction was added or removed.
    Reaction {
        op: String,
        message_id: i64,
        emoji_name: String,
        user_id: i64,
    },
    /// A message was edited, moved, or resolved.
    Update { message_id: i64 },
    /// A message was deleted.
    Delete { message_id: i64 },
    /// Someone started or stopped typing.
    Typing {
        op: String,
        sender_id: i64,
        sender_name: String,
        to: Vec<i64>,
    },
    /// Anything else, which the runtime ignores.
    Other,
}

/// Reads an event from the queue without interpreting it further.
pub fn parse_event(event: &Value) -> StoredEvent {
    match event.get("type").and_then(Value::as_str) {
        Some("message") => StoredEvent::Message,
        Some("reaction") => StoredEvent::Reaction {
            op: string(event, "op"),
            message_id: integer(event, "message_id"),
            emoji_name: string(event, "emoji_name"),
            user_id: integer(event, "user_id"),
        },
        Some("update_message") => StoredEvent::Update {
            message_id: integer(event, "message_id"),
        },
        Some("delete_message") => StoredEvent::Delete {
            message_id: integer(event, "message_id"),
        },
        Some("typing") => StoredEvent::Typing {
            op: string(event, "op"),
            sender_id: event
                .get("sender")
                .map(|sender| integer(sender, "user_id"))
                .unwrap_or(0),
            sender_name: event
                .get("sender")
                .and_then(|sender| sender.get("full_name"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned(),
            to: event
                .get("recipients")
                .and_then(Value::as_array)
                .map(|recipients| {
                    recipients
                        .iter()
                        .filter_map(|user| user.get("user_id").and_then(Value::as_i64))
                        .collect()
                })
                .unwrap_or_default(),
        },
        _ => StoredEvent::Other,
    }
}

fn string(object: &Value, key: &str) -> String {
    object
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned()
}

fn integer(object: &Value, key: &str) -> i64 {
    object.get(key).and_then(Value::as_i64).unwrap_or(0)
}

fn attachment_content(object: &Value, text: &str) -> Content {
    let mime = object
        .get("content_type")
        .and_then(Value::as_str)
        .unwrap_or("application/octet-stream");
    let size = object
        .get("size")
        .and_then(Value::as_i64)
        .unwrap_or(0)
        .max(0) as u64;
    let name = object.get("name").and_then(Value::as_str).unwrap_or("");
    let caption = (!text.is_empty()).then(|| text.to_owned());
    if mime.starts_with("image/") {
        Content::Image {
            caption,
            media: media(mime, size),
        }
    } else if mime.starts_with("audio/") {
        Content::Audio {
            media: media(mime, size),
            seconds: None,
            voice_note: mime.contains("ogg") || mime.contains("opus"),
            waveform: Vec::new(),
        }
    } else if mime.starts_with("video/") {
        Content::Video {
            caption,
            media: media(mime, size),
            seconds: None,
            gif: false,
            note: false,
        }
    } else {
        Content::Document {
            media: media(mime, size),
            file_name: name.to_owned(),
            caption,
            pages: None,
        }
    }
}

fn media(mime: &str, size: u64) -> Media {
    Media {
        mime: mime.to_owned(),
        size,
        width: None,
        height: None,
        path: None,
        state: MediaState::Idle,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn account() -> AccountId {
        AccountId(9)
    }

    #[test]
    fn peers_round_trip_through_their_stored_form() {
        assert_eq!(
            parse(&stream_id(account(), 5)),
            Some(Target::Stream { stream: 5 })
        );
        assert_eq!(
            parse(&topic_id(account(), 5, "design")),
            Some(Target::Topic {
                stream: 5,
                topic: "design".to_owned()
            })
        );
        assert_eq!(
            parse(&topic_id(account(), 5, "design: the sequel")),
            Some(Target::Topic {
                stream: 5,
                topic: "design: the sequel".to_owned()
            })
        );
        assert_eq!(
            parse(&dm_id(account(), &[12, 3, 12])),
            Some(Target::Dm { users: vec![3, 12] })
        );
        assert_eq!(parse(&ChatId::new(account(), "nonsense")), None);
        assert_eq!(parse(&ChatId::new(account(), "d")), None);
        assert_eq!(
            parse(&ChatId::new(account(), "t5:")),
            Some(Target::Topic {
                stream: 5,
                topic: String::new()
            })
        );
    }

    #[test]
    fn capabilities_match_what_ships() {
        let capabilities = capabilities();
        assert!(capabilities.text);
        assert!(capabilities.files);
        assert!(!capabilities.voice_notes);
        assert!(!capabilities.custom_emoji);
        assert_eq!(
            capabilities.reactions,
            crate::account::ReactionStyle::Counts
        );
        assert_eq!(capabilities.gif, crate::account::GifSource::None);
        assert!(capabilities.threads);
        assert!(capabilities.typing);
        assert!(capabilities.edit);
        assert!(capabilities.delete_for_everyone);
        assert!(!capabilities.polls);
        assert_eq!(capabilities.calls, crate::account::CallAccess::None);
    }

    #[test]
    fn subscriptions_become_stream_rows() {
        let chat = stream_chat(account(), &json!({"stream_id": 7, "name": "Engineering"})).unwrap();
        assert_eq!(chat.id, stream_id(account(), 7));
        assert_eq!(chat.name, "Engineering");
        assert_eq!(chat.kind, ChatKind::Group);
        assert_eq!(chat.parent, None);
    }

    #[test]
    fn topics_are_child_rows_of_their_stream() {
        let chat = topic_chat(account(), 7, "design");
        assert_eq!(chat.id, topic_id(account(), 7, "design"));
        assert_eq!(chat.name, "design");
        assert_eq!(chat.kind, ChatKind::Group);
        assert_eq!(chat.parent, Some(stream_id(account(), 7)));
        assert_eq!(topic_chat(account(), 7, "").name, "(no topic)");
    }

    #[test]
    fn direct_messages_name_their_people() {
        let chat = dm_chat(account(), &[3, 12], &["Ada".to_owned(), "Grace".to_owned()]);
        assert_eq!(chat.id, dm_id(account(), &[12, 3]));
        assert_eq!(chat.name, "Ada, Grace");
        assert_eq!(chat.kind, ChatKind::Direct);
    }

    #[test]
    fn a_topic_message_projects_into_its_row() {
        let object = json!({
            "id": 41,
            "type": "stream",
            "stream_id": 7,
            "subject": "design",
            "content": "The plan is ready",
            "sender_id": 12,
            "sender_full_name": "Grace",
            "timestamp": 1712345678,
        });
        let message = message(account(), &object, 3, Some("Grace".to_owned()), Vec::new()).unwrap();
        assert_eq!(message.id, "41");
        assert_eq!(message.chat, topic_id(account(), 7, "design"));
        assert_eq!(message.sender, "12");
        assert_eq!(message.sender_name.as_deref(), Some("Grace"));
        assert!(!message.from_me);
        assert_eq!(message.timestamp, 1712345678);
        assert_eq!(
            message.content,
            Content::Text {
                text: "The plan is ready".to_owned(),
                preview: None,
            }
        );
        assert!(!message.edited);
    }

    #[test]
    fn the_general_topic_stays_on_its_stream_row() {
        for subject in ["general", ""] {
            let object = json!({
                "id": 42,
                "type": "stream",
                "stream_id": 7,
                "subject": subject,
                "content": "Hello",
                "sender_id": 3,
                "timestamp": 1712345678,
            });
            let message = message(account(), &object, 3, None, Vec::new()).unwrap();
            assert_eq!(message.chat, stream_id(account(), 7));
            assert!(message.from_me);
            assert_eq!(message.status, Delivery::Sent);
        }
    }

    #[test]
    fn a_direct_message_projects_into_its_row() {
        let object = json!({
            "id": 43,
            "type": "private",
            "display_recipient": [
                {"id": 3, "email": "me@example.com", "full_name": "Me"},
                {"id": 12, "email": "grace@example.com", "full_name": "Grace"},
            ],
            "content": "Lunch?",
            "sender_id": 12,
            "timestamp": 1712345678,
        });
        let message = message(account(), &object, 3, Some("Grace".to_owned()), Vec::new()).unwrap();
        assert_eq!(message.chat, dm_id(account(), &[12]));
        assert!(!message.from_me);
    }

    #[test]
    fn an_attachment_becomes_a_media_bubble() {
        let object = json!({
            "id": 44,
            "type": "stream",
            "stream_id": 7,
            "subject": "photos",
            "content": "From the trip",
            "sender_id": 12,
            "timestamp": 1712345678,
            "attachments": [{
                "id": 90,
                "name": "trip.jpg",
                "size": 4096,
                "url": "/user_uploads/1/ab/trip.jpg",
                "content_type": "image/jpeg",
            }],
        });
        let message = message(account(), &object, 3, None, Vec::new()).unwrap();
        let Content::Image { caption, media } = &message.content else {
            panic!("expected an image");
        };
        assert_eq!(caption.as_deref(), Some("From the trip"));
        assert_eq!(media.mime, "image/jpeg");
        assert_eq!(media.size, 4096);
        assert_eq!(
            attachment_url(&object).as_deref(),
            Some("/user_uploads/1/ab/trip.jpg")
        );
    }

    #[test]
    fn edits_and_reactions_ride_along() {
        let object = json!({
            "id": 45,
            "type": "stream",
            "stream_id": 7,
            "subject": "design",
            "content": "Fixed",
            "sender_id": 12,
            "timestamp": 1712345678,
            "last_edit_timestamp": 1712345700,
            "reactions": [
                {"emoji_name": "thumbs_up", "user_id": 3},
                {"emoji_name": "thumbs_up", "user_id": 12},
                {"emoji_name": "🎉", "user_id": 12},
            ],
        });
        let rows = reaction_rows(&object);
        assert_eq!(
            rows,
            vec![
                ("👍".to_owned(), "3".to_owned()),
                ("👍".to_owned(), "12".to_owned()),
                ("🎉".to_owned(), "12".to_owned()),
            ]
        );
        let reactions = reactions(&rows, 3);
        assert_eq!(reactions.len(), 2);
        assert_eq!(reactions[0].emoji, "👍");
        assert_eq!(reactions[0].count, 2);
        assert!(reactions[0].from_me);
        assert!(!reactions[1].from_me);
        let message = message(account(), &object, 3, None, reactions).unwrap();
        assert!(message.edited);
    }

    #[test]
    fn reaction_names_known_and_unknown() {
        assert_eq!(reaction_name("thumbs_up"), "👍");
        assert_eq!(reaction_name("100"), "💯");
        assert_eq!(reaction_name("🎉"), "🎉");
        assert_eq!(reaction_name("custom_party"), ":custom_party:");
        assert_eq!(reaction_key(":tada:"), "tada");
        assert_eq!(reaction_key("👍"), "👍");
    }

    #[test]
    fn events_carry_the_parts_the_runtime_needs() {
        assert_eq!(
            parse_event(&json!({"type": "message"})),
            StoredEvent::Message
        );
        assert_eq!(
            parse_event(&json!({
                "type": "reaction",
                "op": "add",
                "message_id": 41,
                "emoji_name": "heart",
                "user_id": 3,
            })),
            StoredEvent::Reaction {
                op: "add".to_owned(),
                message_id: 41,
                emoji_name: "heart".to_owned(),
                user_id: 3,
            }
        );
        assert_eq!(
            parse_event(&json!({"type": "update_message", "message_id": 41})),
            StoredEvent::Update { message_id: 41 }
        );
        assert_eq!(
            parse_event(&json!({"type": "delete_message", "message_id": 41})),
            StoredEvent::Delete { message_id: 41 }
        );
        assert_eq!(
            parse_event(&json!({
                "type": "typing",
                "op": "start",
                "sender": {"user_id": 12, "full_name": "Grace"},
                "recipients": [{"user_id": 12}, {"user_id": 3}],
            })),
            StoredEvent::Typing {
                op: "start".to_owned(),
                sender_id: 12,
                sender_name: "Grace".to_owned(),
                to: vec![12, 3],
            }
        );
        assert_eq!(
            parse_event(&json!({"type": "presence"})),
            StoredEvent::Other
        );
    }
}
