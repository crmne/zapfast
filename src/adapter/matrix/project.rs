//! Projection of matrix-sdk values into the shared chat and message models.
//!
//! Everything here is pure so fixture tests can exercise it without a
//! homeserver or an account.

use matrix_sdk::ruma::UInt;
use matrix_sdk::ruma::events::room::MediaSource;
use matrix_sdk::ruma::events::room::message::{
    AudioMessageEventContent, FileMessageEventContent, ImageMessageEventContent,
    LocationMessageEventContent, MessageType, RoomMessageEventContent, VideoMessageEventContent,
};
use matrix_sdk::ruma::{EventId, OwnedRoomId, RoomId, UserId};

use crate::account::{
    AccountId, CallAccess, Capabilities, GifSource, ReactionStyle, StickerAccess,
};
use crate::model::{ChatId, Content, Delivery, Media, MediaState, Message, Quoted, Reaction};

/// What a Matrix account can do in the window today. GIPHY embeds, sticker
/// packs, image-pack emoji, and threads are not wired yet, so they stay off
/// rather than being advertised.
pub fn capabilities() -> Capabilities {
    Capabilities {
        text: true,
        files: true,
        voice_notes: true,
        video_notes: false,
        custom_emoji: false,
        reactions: ReactionStyle::Counts,
        gif: GifSource::None,
        stickers: StickerAccess::None,
        threads: false,
        spaces: true,
        typing: true,
        read_receipts: true,
        edit: true,
        delete_for_everyone: true,
        polls: false,
        calls: CallAccess::PlaceVoice,
    }
}

/// The chat id for a room: the local account plus the room id.
pub fn chat_id(account: AccountId, room: &RoomId) -> ChatId {
    ChatId::new(account, room.as_str())
}

/// The room id behind a chat, when the peer is one.
pub fn parse_room(chat: &ChatId) -> Option<OwnedRoomId> {
    OwnedRoomId::try_from(chat.peer()).ok()
}

/// The media a message carries, before any download.
fn media(mime: Option<&str>, size: u64, width: Option<u32>, height: Option<u32>) -> Media {
    Media {
        mime: mime.unwrap_or("application/octet-stream").to_owned(),
        size,
        width,
        height,
        path: None,
        state: MediaState::Idle,
    }
}

/// The caption of a media message, when the body is not just the file name.
fn caption(body: &str, filename: Option<&str>) -> Option<String> {
    match filename {
        Some(name) if name == body => None,
        Some(_) => Some(body.to_owned()),
        None => None,
    }
}

fn size_of(info_size: Option<UInt>) -> u64 {
    info_size.map(u64::from).unwrap_or(0)
}

/// Pixel sizes travel as `UInt`; the window keeps `u32` and saturates.
fn small(info_size: Option<UInt>) -> Option<u32> {
    info_size.map(|size| u64::from(size).min(u64::from(u32::MAX)) as u32)
}

fn image_content(image: &ImageMessageEventContent) -> Content {
    let info = image.info.as_deref();
    Content::Image {
        caption: caption(&image.body, image.filename.as_deref()),
        media: media(
            info.and_then(|info| info.mimetype.as_deref()),
            size_of(info.and_then(|info| info.size)),
            small(info.and_then(|info| info.width)),
            small(info.and_then(|info| info.height)),
        ),
    }
}

fn file_content(file: &FileMessageEventContent) -> Content {
    let info = file.info.as_deref();
    Content::Document {
        media: media(
            info.and_then(|info| info.mimetype.as_deref()),
            size_of(info.and_then(|info| info.size)),
            None,
            None,
        ),
        file_name: file.filename().to_owned(),
        caption: caption(&file.body, file.filename.as_deref()),
        pages: None,
    }
}

fn audio_content(audio: &AudioMessageEventContent) -> Content {
    let info = audio.info.as_deref();
    let mime = info
        .and_then(|info| info.mimetype.as_deref())
        .unwrap_or("audio/ogg")
        .to_owned();
    // Element sends voice messages as Ogg Opus; the stable event carries no
    // voice flag in this ruma release, so the codec decides.
    let voice_note = mime.contains("audio/ogg");
    Content::Audio {
        media: media(
            Some(&mime),
            size_of(info.and_then(|info| info.size)),
            None,
            None,
        ),
        seconds: info
            .and_then(|info| info.duration)
            .map(|duration| duration.as_secs().min(u64::from(u32::MAX)) as u32),
        voice_note,
        waveform: Vec::new(),
    }
}

fn video_content(video: &VideoMessageEventContent) -> Content {
    let info = video.info.as_deref();
    Content::Video {
        caption: caption(&video.body, video.filename.as_deref()),
        media: media(
            info.and_then(|info| info.mimetype.as_deref()),
            size_of(info.and_then(|info| info.size)),
            small(info.and_then(|info| info.width)),
            small(info.and_then(|info| info.height)),
        ),
        seconds: info
            .and_then(|info| info.duration)
            .map(|duration| duration.as_secs().min(u64::from(u32::MAX)) as u32),
        gif: false,
        note: false,
    }
}

fn location_content(location: &LocationMessageEventContent) -> Content {
    let Some(rest) = location.geo_uri.strip_prefix("geo:") else {
        return Content::Unsupported {
            what: "Location".to_owned(),
        };
    };
    let mut parts = rest.split(',');
    let latitude = parts.next().and_then(|value| value.parse::<f64>().ok());
    let longitude = parts.next().and_then(|value| value.parse::<f64>().ok());
    match (latitude, longitude) {
        (Some(latitude), Some(longitude)) => Content::Location {
            latitude,
            longitude,
            name: None,
            address: None,
        },
        _ => Content::Unsupported {
            what: "Location".to_owned(),
        },
    }
}

/// The window content of a room message.
pub fn content(content: &RoomMessageEventContent) -> Content {
    match &content.msgtype {
        MessageType::Text(text) => Content::Text {
            text: text.body.clone(),
            preview: None,
        },
        MessageType::Notice(notice) => Content::Text {
            text: notice.body.clone(),
            preview: None,
        },
        MessageType::Emote(emote) => Content::Text {
            text: emote.body.clone(),
            preview: None,
        },
        MessageType::Image(image) => image_content(image),
        MessageType::File(file) => file_content(file),
        MessageType::Audio(audio) => audio_content(audio),
        MessageType::Video(video) => video_content(video),
        MessageType::Location(location) => location_content(location),
        _ => Content::Unsupported {
            what: "Message".to_owned(),
        },
    }
}

/// The window content of a sticker event, as downloaded media.
pub fn sticker_content(
    source: &MediaSource,
    info: Option<&matrix_sdk::ruma::events::room::ImageInfo>,
) -> Content {
    let _ = source;
    Content::Sticker {
        media: media(
            info.and_then(|info| info.mimetype.as_deref()),
            size_of(info.and_then(|info| info.size)),
            small(info.and_then(|info| info.width)),
            small(info.and_then(|info| info.height)),
        ),
        animated: false,
    }
}

/// Builds one projected message. The caller resolves names, replies, and
/// reactions; this only assembles the shared shape.
#[allow(clippy::too_many_arguments)]
pub fn message(
    account: AccountId,
    room: &RoomId,
    event_id: &EventId,
    sender: &UserId,
    sender_name: Option<String>,
    timestamp_ms: u64,
    content: Content,
    from_me: bool,
    quoted: Option<Quoted>,
    reactions: Vec<Reaction>,
) -> Message {
    Message {
        id: event_id.to_string(),
        chat: chat_id(account, room),
        sender: sender.to_string(),
        sender_name,
        from_me,
        timestamp: (timestamp_ms / 1000) as i64,
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
    }
}

/// The reactions of one message, one row per emoji, counting senders.
/// `rows` holds `(emoji, sender)` pairs; one sender reacts once per emoji.
pub fn reactions(rows: &[(String, String)], me: &UserId) -> Vec<Reaction> {
    let mut grouped: Vec<(String, u32, bool)> = Vec::new();
    for (emoji, sender) in rows {
        match grouped.iter_mut().find(|(key, _, _)| key == emoji) {
            Some((_, count, mine)) => {
                *count += 1;
                *mine |= sender == me.as_str();
            }
            None => grouped.push((emoji.clone(), 1, sender == me.as_str())),
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
    use matrix_sdk::ruma::events::room::message::OriginalSyncRoomMessageEvent;
    use matrix_sdk::ruma::owned_room_id;
    use serde_json::json;

    use super::*;

    fn event(content: serde_json::Value) -> OriginalSyncRoomMessageEvent {
        serde_json::from_value(json!({
            "type": "m.room.message",
            "event_id": "$event",
            "sender": "@ada:example.org",
            "origin_server_ts": 1_700_000_000_000u64,
            "content": content,
        }))
        .expect("fixture event")
    }

    #[test]
    fn a_room_round_trips_through_its_chat() {
        let account = AccountId(7);
        let room = owned_room_id!("!ops:example.org");
        let chat = chat_id(account, &room);
        assert_eq!(chat.peer(), "!ops:example.org");
        assert_eq!(parse_room(&chat).as_ref(), Some(&room));
        let other = ChatId::new(account, "not a room");
        assert!(parse_room(&other).is_none());
    }

    #[test]
    fn text_notice_and_emote_project_as_text() {
        for (msgtype, body) in [
            ("m.text", "hello"),
            ("m.notice", "heads up"),
            ("m.emote", "waves"),
        ] {
            let content = content(&event(json!({ "msgtype": msgtype, "body": body })).content);
            assert_eq!(
                content,
                Content::Text {
                    text: body.to_owned(),
                    preview: None,
                }
            );
        }
    }

    #[test]
    fn an_image_keeps_its_caption_and_metadata() {
        let image = event(json!({
            "msgtype": "m.image",
            "body": "sunset",
            "filename": "sunset.jpg",
            "url": "mxc://example.org/abc",
            "info": { "mimetype": "image/jpeg", "w": 640, "h": 480, "size": 2048 }
        }));
        match content(&image.content) {
            Content::Image { caption, media } => {
                assert_eq!(caption.as_deref(), Some("sunset"));
                assert_eq!(media.mime, "image/jpeg");
                assert_eq!(media.size, 2048);
                assert_eq!(media.width, Some(640));
                assert_eq!(media.height, Some(480));
                assert!(media.path.is_none());
            }
            other => panic!("expected an image, got {other:?}"),
        }

        let bare = event(json!({
            "msgtype": "m.image",
            "body": "sunset.jpg",
            "url": "mxc://example.org/abc"
        }));
        match content(&bare.content) {
            Content::Image { caption, .. } => assert!(caption.is_none()),
            other => panic!("expected an image, got {other:?}"),
        }
    }

    #[test]
    fn a_file_keeps_its_name_and_caption() {
        let file = event(json!({
            "msgtype": "m.file",
            "body": "the report",
            "filename": "report.pdf",
            "url": "mxc://example.org/doc",
            "info": { "mimetype": "application/pdf", "size": 4096 }
        }));
        match content(&file.content) {
            Content::Document {
                media,
                file_name,
                caption,
                ..
            } => {
                assert_eq!(file_name, "report.pdf");
                assert_eq!(caption.as_deref(), Some("the report"));
                assert_eq!(media.mime, "application/pdf");
                assert_eq!(media.size, 4096);
            }
            other => panic!("expected a document, got {other:?}"),
        }
    }

    #[test]
    fn ogg_audio_projects_as_a_voice_note() {
        let audio = event(json!({
            "msgtype": "m.audio",
            "body": "voice.ogg",
            "url": "mxc://example.org/voice",
            "info": { "mimetype": "audio/ogg", "duration": 7000 }
        }));
        match content(&audio.content) {
            Content::Audio {
                seconds,
                voice_note,
                media,
                ..
            } => {
                assert!(voice_note);
                assert_eq!(seconds, Some(7));
                assert_eq!(media.mime, "audio/ogg");
            }
            other => panic!("expected audio, got {other:?}"),
        }
    }

    #[test]
    fn a_location_uri_becomes_a_location() {
        let location = event(json!({
            "msgtype": "m.location",
            "body": "Ada's office",
            "geo_uri": "geo:51.5008,-0.1247"
        }));
        match content(&location.content) {
            Content::Location {
                latitude,
                longitude,
                ..
            } => {
                assert!((latitude - 51.5008).abs() < f64::EPSILON);
                assert!((longitude + 0.1247).abs() < f64::EPSILON);
            }
            other => panic!("expected a location, got {other:?}"),
        }
        let broken = event(json!({
            "msgtype": "m.location",
            "body": "nowhere",
            "geo_uri": "not-a-geo-uri"
        }));
        assert!(matches!(
            content(&broken.content),
            Content::Unsupported { .. }
        ));
    }

    #[test]
    fn a_message_carries_its_event_and_sender() {
        let room = owned_room_id!("!ops:example.org");
        let event = event(json!({ "msgtype": "m.text", "body": "hello" }));
        let message = message(
            AccountId(7),
            &room,
            &event.event_id,
            &event.sender,
            Some("Ada".to_owned()),
            u64::from(event.origin_server_ts.get()),
            content(&event.content),
            false,
            None,
            Vec::new(),
        );
        assert_eq!(message.id, "$event");
        assert_eq!(message.chat, chat_id(AccountId(7), &room));
        assert_eq!(message.sender, "@ada:example.org");
        assert_eq!(message.sender_name.as_deref(), Some("Ada"));
        assert_eq!(message.timestamp, 1_700_000_000);
        assert_eq!(message.status, Delivery::None);
        assert!(!message.from_me);
    }

    #[test]
    fn reactions_count_senders_and_remember_ours() {
        let me = UserId::parse("@me:example.org").unwrap();
        let rows = vec![
            ("👍".to_owned(), "@ada:example.org".to_owned()),
            ("👍".to_owned(), "@me:example.org".to_owned()),
            ("🎉".to_owned(), "@ada:example.org".to_owned()),
        ];
        let projected = reactions(&rows, &me);
        assert_eq!(projected.len(), 2);
        assert_eq!(projected[0].emoji, "👍");
        assert_eq!(projected[0].count, 2);
        assert!(projected[0].from_me);
        assert_eq!(projected[1].emoji, "🎉");
        assert_eq!(projected[1].count, 1);
        assert!(!projected[1].from_me);
    }

    #[test]
    fn matrix_capabilities_stay_honest() {
        let caps = capabilities();
        assert!(caps.text && caps.files && caps.voice_notes);
        assert!(caps.reactions == ReactionStyle::Counts);
        assert!(caps.edit && caps.delete_for_everyone && caps.typing && caps.read_receipts);
        assert!(caps.spaces);
        assert!(!caps.custom_emoji && !caps.threads && !caps.polls && !caps.video_notes);
        assert!(caps.gif == GifSource::None && caps.stickers == StickerAccess::None);
        assert!(caps.calls == CallAccess::PlaceVoice);
    }
}
