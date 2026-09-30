//! Pure projections from X API values into the window's models.
//!
//! The API returns pages of direct-message events newest first, with media
//! and user expansions next to them. Everything here is a plain function over
//! parsed JSON, so the runtime stays about HTTP.

use serde::Deserialize;

use crate::account::{
    AccountId, CallAccess, Capabilities, GifSource, ReactionStyle, StickerAccess,
};
use crate::model::{Chat, ChatId, ChatKind, Content, Delivery, Media, MediaState, Message, Quoted};

/// A conversation as the list endpoint reports it.
#[derive(Clone, Debug, Deserialize)]
pub struct Conversation {
    pub dm_conversation_id: String,
}

impl Conversation {
    /// The id, checked by the caller's fixture tests.
    pub fn id(&self) -> &str {
        &self.dm_conversation_id
    }
}

/// One direct-message event.
#[derive(Clone, Debug, Deserialize)]
pub struct Event {
    pub id: String,
    pub event_type: String,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub sender_id: Option<String>,
    #[serde(default)]
    pub dm_conversation_id: Option<String>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub attachments: Option<Attachments>,
}

/// The attachment keys of an event.
#[derive(Clone, Debug, Deserialize)]
pub struct Attachments {
    #[serde(default)]
    pub media_keys: Vec<String>,
}

/// A media object from the includes block.
#[derive(Clone, Debug, Deserialize)]
pub struct XMedia {
    pub media_key: String,
    #[serde(default, rename = "type")]
    pub kind: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub preview_image_url: Option<String>,
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub height: Option<u32>,
}

/// A user object from the includes block.
#[derive(Clone, Debug, Deserialize)]
pub struct User {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub username: Option<String>,
}

/// The includes block of a page.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct Includes {
    #[serde(default)]
    pub media: Vec<XMedia>,
    #[serde(default)]
    pub users: Vec<User>,
}

/// A page of events.
#[derive(Clone, Debug, Deserialize)]
pub struct Page {
    #[serde(default)]
    pub data: Vec<Event>,
    #[serde(default)]
    pub includes: Includes,
    #[serde(default)]
    pub meta: Meta,
}

/// The page cursor.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct Meta {
    #[serde(default)]
    pub next_token: Option<String>,
    #[serde(default)]
    pub result_count: Option<usize>,
}

/// The chat row of a conversation.
pub fn chat_id(account: AccountId, conversation: &str) -> ChatId {
    ChatId::new(account, format!("x{conversation}"))
}

/// Reads the conversation id back out of a chat.
pub fn parse_chat(chat: &ChatId) -> Option<&str> {
    chat.peer().strip_prefix('x')
}

/// What an X account can do in a conversation.
pub fn capabilities() -> Capabilities {
    Capabilities {
        text: true,
        // Only images can be attached, and only when the tier allows it.
        files: false,
        voice_notes: false,
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

/// The row of a conversation.
pub fn chat(
    account: AccountId,
    conversation: &str,
    name: Option<String>,
    latest: Option<&Event>,
) -> Chat {
    let mut chat = Chat::new(
        chat_id(account, conversation),
        name.unwrap_or_else(|| "X conversation".to_owned()),
    );
    chat.kind = ChatKind::Direct;
    if let Some(event) = latest {
        if let Some(timestamp) = timestamp_of(event) {
            chat.last_activity = timestamp;
        }
        chat.last = Some(crate::model::LastMessage {
            from_me: false,
            sender: event.sender_id.clone().unwrap_or_default(),
            sender_name: None,
            summary: summary_of(event),
            full: summary_of(event),
            status: Delivery::None,
        });
    }
    chat
}

/// Projects one event. Only message events with something in them become
/// window messages.
pub fn message(
    account: AccountId,
    conversation: &str,
    event: &Event,
    me: &str,
    sender_name: Option<String>,
    includes: &Includes,
    quoted: Option<Quoted>,
) -> Option<Message> {
    if event.event_type != "MessageCreate" {
        return None;
    }
    let text = event.text.clone().unwrap_or_default();
    let media = media_of(event, includes);
    let content = if let Some(media) = media {
        content_with_media(media, &text)
    } else if text.is_empty() {
        return None;
    } else {
        Content::Text {
            text,
            preview: None,
        }
    };
    let from_me = event.sender_id.as_deref() == Some(me);
    Some(Message {
        id: event.id.clone(),
        chat: chat_id(account, conversation),
        sender: event.sender_id.clone().unwrap_or_default(),
        sender_name,
        from_me,
        timestamp: timestamp_of(event).unwrap_or_default(),
        content,
        status: if from_me {
            Delivery::Sent
        } else {
            Delivery::None
        },
        delivered_at: None,
        read_at: None,
        quoted,
        reactions: Vec::new(),
        edited: false,
        mentions: Vec::new(),
        forwarded: false,
        thumbnail: None,
    })
}

/// The unix seconds of an event.
pub fn timestamp_of(event: &Event) -> Option<i64> {
    let created = event.created_at.as_deref()?;
    created
        .parse::<jiff::Timestamp>()
        .ok()
        .map(|timestamp| timestamp.as_second())
}

/// The first media object an event points at, if the expansion carried it.
pub fn media_of<'a>(event: &Event, includes: &'a Includes) -> Option<&'a XMedia> {
    let keys = event.attachments.as_ref()?.media_keys.as_slice();
    let key = keys.first()?;
    includes.media.iter().find(|media| &media.media_key == key)
}

/// The display name of a user from the includes block.
pub fn user_name(includes: &Includes, user_id: &str) -> Option<String> {
    includes
        .users
        .iter()
        .find(|user| user.id == user_id)
        .and_then(|user| user.name.clone().or_else(|| user.username.clone()))
}

fn content_with_media(media: &XMedia, text: &str) -> Content {
    let caption = (!text.is_empty()).then(|| text.to_owned());
    let width = media.width;
    let height = media.height;
    Content::Image {
        caption,
        media: Media {
            mime: "image/jpeg".to_owned(),
            size: 0,
            width,
            height,
            path: None,
            state: MediaState::Idle,
        },
    }
}

fn summary_of(event: &Event) -> String {
    match &event.text {
        Some(text) if !text.is_empty() => text.clone(),
        _ => "[photo]".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account() -> AccountId {
        AccountId(11)
    }

    fn page(json: &str) -> Page {
        serde_json::from_str(json).expect("fixture page")
    }

    #[test]
    fn chat_ids_round_trip() {
        let chat = chat_id(account(), "123-456");
        assert_eq!(parse_chat(&chat), Some("123-456"));
        assert_eq!(parse_chat(&ChatId::new(account(), "nope")), None);
    }

    #[test]
    fn capabilities_stay_honest() {
        let capabilities = capabilities();
        assert!(capabilities.text);
        assert!(!capabilities.files, "only images are sent, not files");
        assert!(!capabilities.typing && !capabilities.read_receipts);
        assert_eq!(capabilities.reactions, ReactionStyle::None);
        assert_eq!(capabilities.calls, CallAccess::None);
    }

    #[test]
    fn a_page_projects_its_events_oldest_first_by_timestamp() {
        let fixture = page(
            r#"{
                "data": [
                    {"id": "2", "event_type": "MessageCreate", "text": "second",
                     "sender_id": "8", "dm_conversation_id": "77",
                     "created_at": "2024-05-01T12:05:00.000Z"},
                    {"id": "1", "event_type": "MessageCreate", "text": "first",
                     "sender_id": "8", "dm_conversation_id": "77",
                     "created_at": "2024-05-01T12:00:00.000Z"}
                ],
                "includes": {"users": [{"id": "8", "name": "Grace"}]},
                "meta": {"result_count": 2, "next_token": "older"}
            }"#,
        );
        assert_eq!(fixture.meta.next_token.as_deref(), Some("older"));
        let first = message(
            account(),
            "77",
            &fixture.data[1],
            "7",
            user_name(&fixture.includes, "8"),
            &fixture.includes,
            None,
        )
        .expect("a message");
        assert_eq!(first.id, "1");
        assert_eq!(first.chat, chat_id(account(), "77"));
        assert_eq!(first.timestamp, 1_714_564_800);
        assert_eq!(first.sender_name.as_deref(), Some("Grace"));
        assert!(!first.from_me);
        match first.content {
            Content::Text { text, .. } => assert_eq!(text, "first"),
            other => panic!("unexpected content: {other:?}"),
        }
    }

    #[test]
    fn the_last_page_has_no_cursor() {
        let fixture = page(
            r#"{
                "data": [
                    {"id": "9", "event_type": "MessageCreate", "text": "bye",
                     "sender_id": "7", "dm_conversation_id": "77",
                     "created_at": "2024-05-01T12:00:00.000Z"}
                ],
                "meta": {"result_count": 1}
            }"#,
        );
        assert!(fixture.meta.next_token.is_none());
        let own = message(
            account(),
            "77",
            &fixture.data[0],
            "7",
            None,
            &fixture.includes,
            None,
        )
        .expect("a message");
        assert!(own.from_me);
        assert_eq!(own.status, Delivery::Sent);
    }

    #[test]
    fn attachments_resolve_through_the_includes_block() {
        let fixture = page(
            r#"{
                "data": [
                    {"id": "3", "event_type": "MessageCreate", "text": "",
                     "sender_id": "8", "dm_conversation_id": "77",
                     "created_at": "2024-05-01T12:00:00.000Z",
                     "attachments": {"media_keys": ["3_1"]}}
                ],
                "includes": {"media": [
                    {"media_key": "3_1", "type": "photo",
                     "url": "https://pbs.twimg.com/media/3_1.jpg",
                     "width": 640, "height": 480}
                ]}
            }"#,
        );
        let projected = message(
            account(),
            "77",
            &fixture.data[0],
            "7",
            None,
            &fixture.includes,
            None,
        )
        .expect("a photo");
        match projected.content {
            Content::Image { caption, media } => {
                assert!(caption.is_none());
                assert_eq!(media.width, Some(640));
                assert_eq!(media.height, Some(480));
            }
            other => panic!("unexpected content: {other:?}"),
        }
    }

    #[test]
    fn an_event_with_nothing_in_it_is_not_a_message() {
        let fixture = page(
            r#"{
                "data": [
                    {"id": "4", "event_type": "ParticipantsJoin", "sender_id": "8",
                     "dm_conversation_id": "77", "created_at": "2024-05-01T12:00:00.000Z"},
                    {"id": "5", "event_type": "MessageCreate", "text": "",
                     "sender_id": "8", "dm_conversation_id": "77",
                     "created_at": "2024-05-01T12:00:00.000Z"}
                ]
            }"#,
        );
        for event in &fixture.data {
            assert!(message(account(), "77", event, "7", None, &fixture.includes, None).is_none());
        }
    }

    #[test]
    fn chats_are_named_after_the_other_person() {
        let fixture = page(
            r#"{
                "data": [
                    {"id": "6", "event_type": "MessageCreate", "text": "hello",
                     "sender_id": "8", "dm_conversation_id": "77",
                     "created_at": "2024-05-01T12:00:00.000Z"}
                ]
            }"#,
        );
        let chat = chat(
            account(),
            "77",
            Some("Grace".to_owned()),
            fixture.data.first(),
        );
        assert_eq!(chat.kind, ChatKind::Direct);
        assert_eq!(chat.id, chat_id(account(), "77"));
        assert_eq!(chat.last_activity, 1_714_564_800);
        assert_eq!(
            chat.last.as_ref().map(|last| last.summary.as_str()),
            Some("hello")
        );
    }
}
