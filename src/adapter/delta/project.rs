//! Pure projections from Delta Chat's JSON-RPC objects into the window's
//! models.
//!
//! The server speaks JSON, so every function here takes `serde_json::Value`
//! and returns window values. Nothing in this module talks to the network or
//! to the server process, which keeps the fixture tests honest: they feed
//! recorded response shapes through the same code the runtime uses.

use std::path::PathBuf;

use serde_json::Value;

use crate::account::{
    AccountId, CallAccess, Capabilities, GifSource, ReactionStyle, StickerAccess,
};
use crate::model::{
    Chat, ChatId, ChatKind, Content, Delivery, LastMessage, Media, MediaState, Message, Quoted,
};

/// The contact id Delta Chat uses for the account owner.
pub const SELF_CONTACT: u64 = 1;

/// The message id Delta Chat reserves for day markers.
const DAYMARKER: u64 = 9;

/// The chat row of a Delta Chat chat.
pub fn chat_id(account: AccountId, chat: u64) -> ChatId {
    ChatId::new(account, format!("d{chat}"))
}

/// Reads a chat id back into the Delta Chat chat number.
pub fn parse_chat(chat: &ChatId) -> Option<u64> {
    let mut chars = chat.peer().chars();
    match chars.next()? {
        'd' => chars.as_str().parse().ok(),
        _ => None,
    }
}

/// What a Delta Chat account can do in a chat.
pub fn capabilities() -> Capabilities {
    Capabilities {
        text: true,
        files: true,
        voice_notes: true,
        video_notes: false,
        custom_emoji: false,
        reactions: ReactionStyle::None,
        gif: GifSource::None,
        stickers: StickerAccess::None,
        threads: false,
        spaces: false,
        typing: false,
        read_receipts: false,
        edit: false,
        delete_for_everyone: false,
        polls: false,
        calls: CallAccess::None,
    }
}

/// The row of a chat list entry. Archive links and loading errors are not
/// chats, so they return nothing. Contact requests are rows, but nothing can
/// be sent into them until the user accepts.
pub fn chat(account: AccountId, item: &Value) -> Option<Chat> {
    if item.get("kind").and_then(Value::as_str)? != "ChatListItem" {
        return None;
    }
    let id = item.get("id").and_then(Value::as_u64)?;
    let name = item
        .get("name")
        .and_then(Value::as_str)
        .filter(|name| !name.is_empty())
        .unwrap_or("Chat");
    let chat_type = item
        .get("chatType")
        .and_then(Value::as_str)
        .unwrap_or("Single");
    let mut chat = Chat::new(chat_id(account, id), name.to_owned());
    chat.kind = match chat_type {
        "Single" => ChatKind::Direct,
        "OutBroadcast" | "InBroadcast" => ChatKind::Broadcast,
        _ => ChatKind::Group,
    };
    chat.unread = item
        .get("freshMessageCounter")
        .and_then(Value::as_u64)
        .unwrap_or(0)
        .min(u64::from(u32::MAX)) as u32;
    chat.archived = item
        .get("isArchived")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    chat.pinned = item
        .get("isPinned")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if item
        .get("isMuted")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        chat.muted_until = Some(i64::MAX);
    }
    chat.read_only = item
        .get("isContactRequest")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    chat.last_activity = item.get("lastUpdated").and_then(Value::as_i64).unwrap_or(0) / 1000;
    if let Some(summary) = item
        .get("summaryText1")
        .and_then(Value::as_str)
        .filter(|summary| !summary.is_empty())
    {
        chat.last = Some(LastMessage {
            from_me: false,
            sender: String::new(),
            sender_name: None,
            summary: summary.to_owned(),
            full: summary.to_owned(),
            status: Delivery::None,
        });
    }
    Some(chat)
}

/// Projects one message object. Info entries stay as plain text; they are the
/// mail client's own notices.
pub fn message(
    account: AccountId,
    object: &Value,
    me: u64,
    quoted: Option<Quoted>,
) -> Option<Message> {
    let id = object.get("id").and_then(Value::as_u64)?;
    if id == DAYMARKER {
        return None;
    }
    if id == 0 {
        return None;
    }
    let chat = chat_id(account, object.get("chatId").and_then(Value::as_u64)?);
    let from = object.get("fromId").and_then(Value::as_u64).unwrap_or(me);
    let text = object
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let from_me = from == me;
    let content = content_of(object, text)?;
    let sender_name = object
        .get("sender")
        .and_then(|sender| {
            sender
                .get("displayName")
                .and_then(Value::as_str)
                .filter(|name| !name.is_empty())
                .or_else(|| sender.get("name").and_then(Value::as_str))
        })
        .map(str::to_owned);
    Some(Message {
        id: id.to_string(),
        chat,
        sender: from.to_string(),
        sender_name,
        from_me,
        timestamp: object.get("timestamp").and_then(Value::as_i64).unwrap_or(0),
        content,
        status: if from_me {
            status_of(object)
        } else {
            Delivery::None
        },
        delivered_at: None,
        read_at: None,
        quoted,
        reactions: Vec::new(),
        edited: object
            .get("isEdited")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        mentions: Vec::new(),
        forwarded: object
            .get("isForwarded")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        thumbnail: None,
    })
}

/// The quote a message carries, when it replies to another one.
pub fn quoted(object: &Value) -> Option<Quoted> {
    let quote = object.get("quote")?;
    let kind = quote.get("kind").and_then(Value::as_str)?;
    if kind != "WithMessage" && kind != "JustText" {
        return None;
    }
    Some(Quoted {
        id: quote
            .get("messageId")
            .and_then(Value::as_u64)
            .map(|id| id.to_string())
            .unwrap_or_default(),
        sender: String::new(),
        sender_name: quote
            .get("authorDisplayName")
            .and_then(Value::as_str)
            .map(str::to_owned),
        summary: quote
            .get("text")
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty())
            .unwrap_or("Message")
            .to_owned(),
        mentions: Vec::new(),
    })
}

/// A delta event that matters to the window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StoredEvent {
    /// A message arrived.
    Incoming { chat: u64, message: u64 },
    /// A message changed, including its own sends echoing back.
    Changed { chat: u64, message: u64 },
    /// The network delivered a message this account sent.
    Delivered { chat: u64, message: u64 },
    /// The other side read a message.
    Read { chat: u64, message: u64 },
    /// A message was deleted.
    Deleted { chat: u64, message: u64 },
    /// One chat row changed.
    ChatModified(u64),
    /// The chat list as a whole changed.
    Chatlist,
    /// Some events were dropped; the runtime refreshes what it can.
    Overflow,
    /// Anything else; the runtime ignores it.
    Other(String),
}

/// Reads one long-polled event.
pub fn stored_event(event: &Value) -> StoredEvent {
    let kind = event.get("kind").and_then(Value::as_str).unwrap_or("");
    let chat = event
        .get("chatId")
        .or_else(|| event.get("chat_id"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let message = event
        .get("msgId")
        .or_else(|| event.get("msg_id"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    match kind {
        "IncomingMsg" => StoredEvent::Incoming { chat, message },
        "MsgsChanged" => StoredEvent::Changed { chat, message },
        "MsgDelivered" => StoredEvent::Delivered { chat, message },
        "MsgRead" => StoredEvent::Read { chat, message },
        "MsgDeleted" => StoredEvent::Deleted { chat, message },
        "ChatModified" => StoredEvent::ChatModified(chat),
        "ChatlistChanged" | "ChatlistItemChanged" => StoredEvent::Chatlist,
        "EventChannelOverflow" => StoredEvent::Overflow,
        other => StoredEvent::Other(other.to_owned()),
    }
}

fn content_of(object: &Value, text: &str) -> Option<Content> {
    if object
        .get("isInfo")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        let text = if text.is_empty() {
            "System message"
        } else {
            text
        };
        return Some(Content::Text {
            text: text.to_owned(),
            preview: None,
        });
    }
    let view = object
        .get("viewType")
        .and_then(Value::as_str)
        .unwrap_or("Text");
    let has_file = object
        .get("file")
        .and_then(Value::as_str)
        .is_some_and(|file| !file.is_empty())
        || object.get("fileMime").and_then(Value::as_str).is_some();
    let caption = (!text.is_empty()).then(|| text.to_owned());
    match view {
        "Image" | "Gif" => Some(Content::Image {
            caption,
            media: media_of(object),
        }),
        "Sticker" => Some(Content::Sticker {
            media: media_of(object),
            animated: false,
        }),
        "Voice" => Some(Content::Audio {
            media: media_of(object),
            seconds: seconds_of(object),
            voice_note: true,
            waveform: Vec::new(),
        }),
        "Audio" => Some(Content::Audio {
            media: media_of(object),
            seconds: seconds_of(object),
            voice_note: false,
            waveform: Vec::new(),
        }),
        "Video" => Some(Content::Video {
            caption,
            media: media_of(object),
            seconds: seconds_of(object),
            gif: false,
            note: false,
        }),
        "File" => Some(Content::Document {
            media: media_of(object),
            file_name: file_name(object),
            caption,
            pages: None,
        }),
        "Call" => Some(Content::Unsupported {
            what: "Call".to_owned(),
        }),
        "Webxdc" => Some(Content::Unsupported {
            what: "Webxdc app".to_owned(),
        }),
        "Vcard" => Some(Content::Contact {
            display_name: text.to_owned(),
            vcard: text.to_owned(),
        }),
        _ if !text.is_empty() => Some(Content::Text {
            text: text.to_owned(),
            preview: None,
        }),
        _ if has_file => Some(Content::Document {
            media: media_of(object),
            file_name: file_name(object),
            caption: None,
            pages: None,
        }),
        _ => None,
    }
}

fn media_of(object: &Value) -> Media {
    let mime = object
        .get("fileMime")
        .and_then(Value::as_str)
        .filter(|mime| !mime.is_empty())
        .unwrap_or("application/octet-stream")
        .to_owned();
    let size = object.get("fileBytes").and_then(Value::as_u64).unwrap_or(0);
    let width = object
        .get("dimensionsWidth")
        .and_then(Value::as_u64)
        .filter(|width| *width > 0)
        .map(|width| width.min(u64::from(u32::MAX)) as u32);
    let height = object
        .get("dimensionsHeight")
        .and_then(Value::as_u64)
        .filter(|height| *height > 0)
        .map(|height| height.min(u64::from(u32::MAX)) as u32);
    let downloaded = object
        .get("downloadState")
        .and_then(Value::as_str)
        .is_some_and(|state| state == "Done");
    let path = object
        .get("file")
        .and_then(Value::as_str)
        .filter(|file| !file.is_empty() && downloaded)
        .map(PathBuf::from);
    Media {
        mime,
        size,
        width,
        height,
        path,
        state: MediaState::Idle,
    }
}

fn seconds_of(object: &Value) -> Option<u32> {
    object
        .get("duration")
        .and_then(Value::as_u64)
        .filter(|duration| *duration > 0)
        .map(|duration| duration.min(u64::from(u32::MAX)) as u32)
}

fn file_name(object: &Value) -> String {
    if let Some(name) = object
        .get("fileName")
        .and_then(Value::as_str)
        .filter(|name| !name.is_empty())
    {
        return name.to_owned();
    }
    if let Some(file) = object
        .get("file")
        .and_then(Value::as_str)
        .filter(|file| !file.is_empty())
        && let Some(name) = PathBuf::from(file).file_name()
    {
        return name.to_string_lossy().into_owned();
    }
    "Attachment".to_owned()
}

fn status_of(object: &Value) -> Delivery {
    match object.get("state").and_then(Value::as_u64) {
        Some(19) | Some(20) => Delivery::Pending,
        Some(24) => Delivery::Failed,
        Some(26) => Delivery::Delivered,
        Some(28) => Delivery::Read,
        _ => Delivery::Sent,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn account() -> AccountId {
        AccountId(7)
    }

    fn message_fixture() -> Value {
        json!({
            "id": 101,
            "chatId": 10,
            "fromId": 5,
            "text": "hi",
            "viewType": "Text",
            "state": 10,
            "isInfo": false,
            "file": Value::Null,
            "fileMime": Value::Null,
            "fileName": Value::Null,
            "fileBytes": 0,
            "timestamp": 1_750_000_000,
            "quote": Value::Null,
            "reactions": Value::Null,
            "sender": {
                "id": 5,
                "address": "bob@example.org",
                "displayName": "Bob",
                "name": "Bob"
            }
        })
    }

    #[test]
    fn peers_round_trip_through_their_stored_form() {
        assert_eq!(parse_chat(&chat_id(account(), 10)), Some(10));
        assert_eq!(parse_chat(&ChatId::new(account(), "nonsense")), None);
        assert_eq!(parse_chat(&ChatId::new(account(), "d")), None);
        assert_eq!(parse_chat(&ChatId::new(account(), "x10")), None);
    }

    #[test]
    fn capabilities_stay_honest() {
        let capabilities = capabilities();
        assert!(capabilities.text && capabilities.files && capabilities.voice_notes);
        assert!(!capabilities.video_notes && !capabilities.read_receipts);
        assert!(!capabilities.edit && !capabilities.delete_for_everyone);
        assert_eq!(capabilities.reactions, ReactionStyle::None);
        assert_eq!(capabilities.calls, CallAccess::None);
    }

    #[test]
    fn chat_rows_carry_their_counters() {
        let item = json!({
            "kind": "ChatListItem",
            "id": 10,
            "name": "Bob",
            "chatType": "Single",
            "freshMessageCounter": 3,
            "isArchived": false,
            "isPinned": true,
            "isMuted": false,
            "isContactRequest": false,
            "lastUpdated": 1_750_000_000_000u64,
            "summaryText1": "hi"
        });
        let chat = chat(account(), &item).expect("a chat");
        assert_eq!(chat.id, chat_id(account(), 10));
        assert_eq!(chat.kind, ChatKind::Direct);
        assert_eq!(chat.unread, 3);
        assert!(chat.pinned);
        assert_eq!(chat.last_activity, 1_750_000_000);
        assert_eq!(
            chat.last.as_ref().map(|last| last.summary.as_str()),
            Some("hi")
        );
    }

    #[test]
    fn groups_and_broadcasts_get_their_kinds() {
        let group = json!({"kind": "ChatListItem", "id": 2, "name": "Team", "chatType": "Group"});
        assert_eq!(chat(account(), &group).unwrap().kind, ChatKind::Group);
        let channel =
            json!({"kind": "ChatListItem", "id": 3, "name": "News", "chatType": "InBroadcast"});
        assert_eq!(chat(account(), &channel).unwrap().kind, ChatKind::Broadcast);
        let link = json!({"kind": "ArchiveLink", "id": 6});
        assert!(chat(account(), &link).is_none());
        let request =
            json!({"kind": "ChatListItem", "id": 4, "name": "Ada", "isContactRequest": true});
        assert!(chat(account(), &request).unwrap().read_only);
    }

    #[test]
    fn text_messages_project_their_sender() {
        let projected =
            message(account(), &message_fixture(), SELF_CONTACT, None).expect("a message");
        assert_eq!(projected.id, "101");
        assert_eq!(projected.chat, chat_id(account(), 10));
        assert_eq!(projected.sender, "5");
        assert_eq!(projected.sender_name.as_deref(), Some("Bob"));
        assert!(!projected.from_me);
        assert_eq!(projected.timestamp, 1_750_000_000);
        match projected.content {
            Content::Text { text, .. } => assert_eq!(text, "hi"),
            other => panic!("unexpected content: {other:?}"),
        }
    }

    #[test]
    fn own_messages_carry_their_delivery_state() {
        let mut fixture = message_fixture();
        fixture["fromId"] = json!(SELF_CONTACT);
        fixture["state"] = json!(28);
        let projected = message(account(), &fixture, SELF_CONTACT, None).expect("a message");
        assert!(projected.from_me);
        assert_eq!(projected.status, Delivery::Read);
        fixture["state"] = json!(24);
        assert_eq!(
            message(account(), &fixture, SELF_CONTACT, None)
                .unwrap()
                .status,
            Delivery::Failed
        );
    }

    #[test]
    fn images_and_voice_notes_become_media() {
        let mut image = message_fixture();
        image["viewType"] = json!("Image");
        image["text"] = json!("look");
        image["fileMime"] = json!("image/jpeg");
        image["fileBytes"] = json!(1234);
        image["dimensionsWidth"] = json!(640);
        image["dimensionsHeight"] = json!(480);
        image["downloadState"] = json!("Done");
        image["file"] = json!("/home/user/.local/share/delta/blob/cat.jpg");
        let projected = message(account(), &image, SELF_CONTACT, None).expect("an image");
        match projected.content {
            Content::Image { caption, media } => {
                assert_eq!(caption.as_deref(), Some("look"));
                assert_eq!(media.mime, "image/jpeg");
                assert_eq!(media.width, Some(640));
                assert!(media.path.is_some());
            }
            other => panic!("unexpected content: {other:?}"),
        }
        let mut voice = message_fixture();
        voice["viewType"] = json!("Voice");
        voice["fileMime"] = json!("audio/ogg");
        voice["duration"] = json!(5);
        voice["downloadState"] = json!("Available");
        let projected = message(account(), &voice, SELF_CONTACT, None).expect("a voice note");
        match projected.content {
            Content::Audio {
                voice_note,
                seconds,
                media,
                ..
            } => {
                assert!(voice_note);
                assert_eq!(seconds, Some(5));
                assert!(media.path.is_none(), "not downloaded yet");
            }
            other => panic!("unexpected content: {other:?}"),
        }
    }

    #[test]
    fn documents_keep_their_file_names() {
        let mut fixture = message_fixture();
        fixture["viewType"] = json!("File");
        fixture["text"] = json!("");
        fixture["fileMime"] = json!("application/pdf");
        fixture["fileName"] = json!("plan.pdf");
        let projected = message(account(), &fixture, SELF_CONTACT, None).expect("a file");
        match projected.content {
            Content::Document { file_name, .. } => assert_eq!(file_name, "plan.pdf"),
            other => panic!("unexpected content: {other:?}"),
        }
    }

    #[test]
    fn info_entries_stay_text() {
        let mut fixture = message_fixture();
        fixture["isInfo"] = json!(true);
        fixture["text"] = json!("");
        let projected = message(account(), &fixture, SELF_CONTACT, None).expect("a notice");
        match projected.content {
            Content::Text { text, .. } => assert_eq!(text, "System message"),
            other => panic!("unexpected content: {other:?}"),
        }
    }

    #[test]
    fn quotes_come_across() {
        let mut fixture = message_fixture();
        fixture["quote"] = json!({
            "kind": "WithMessage",
            "text": "the earlier line",
            "messageId": 99,
            "authorDisplayName": "Ada"
        });
        let quoted = quoted(&fixture).expect("a quote");
        assert_eq!(quoted.id, "99");
        assert_eq!(quoted.sender_name.as_deref(), Some("Ada"));
        assert_eq!(quoted.summary, "the earlier line");
    }

    #[test]
    fn events_parse_into_their_kinds() {
        assert_eq!(
            stored_event(&json!({"kind": "IncomingMsg", "chatId": 10, "msgId": 101})),
            StoredEvent::Incoming {
                chat: 10,
                message: 101
            }
        );
        assert_eq!(
            stored_event(&json!({"kind": "ChatModified", "chatId": 10})),
            StoredEvent::ChatModified(10)
        );
        assert_eq!(
            stored_event(&json!({"kind": "EventChannelOverflow", "n": 2})),
            StoredEvent::Overflow
        );
        assert_eq!(
            stored_event(&json!({"kind": "SomethingElse"})),
            StoredEvent::Other("SomethingElse".to_owned())
        );
    }
}
