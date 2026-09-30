//! Pure projections from Discord's models into the window's models.
//!
//! A guild is a chat row marked as a space; its text channels and threads are
//! rows beneath it, and direct messages are rows of their own. Every function
//! here takes Discord values and returns window values, so the runtime stays
//! about networking.

use twilight_model::channel::message::sticker::MessageSticker;
use twilight_model::channel::message::{
    EmojiReactionType, Message as DiscordMessage, Reaction as DiscordReaction,
};
use twilight_model::channel::{Attachment, Channel, ChannelType};
use twilight_model::user::User;

use crate::account::{
    AccountId, CallAccess, Capabilities, GifSource, ReactionStyle, StickerAccess,
};
use crate::model::{
    Chat, ChatId, ChatKind, Content, Delivery, Media, MediaState, Message, Quoted, Reaction,
};

/// Which Discord thing a chat id points at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// A guild, shown as a space in the rail.
    Guild(u64),
    /// A guild channel, a thread, or a direct message.
    Channel(u64),
}

/// The chat row of a guild.
pub fn guild_id(account: AccountId, guild: u64) -> ChatId {
    ChatId::new(account, format!("g{guild}"))
}

/// The chat row of a channel, thread, or direct message.
pub fn channel_id(account: AccountId, channel: u64) -> ChatId {
    ChatId::new(account, format!("c{channel}"))
}

/// Reads a chat id back into the Discord thing it points at.
pub fn parse(chat: &ChatId) -> Option<Target> {
    let mut chars = chat.peer().chars();
    let prefix = chars.next()?;
    let id = chars.as_str().parse().ok()?;
    match prefix {
        'g' => Some(Target::Guild(id)),
        'c' => Some(Target::Channel(id)),
        _ => None,
    }
}

/// What a Discord bot account can do in a chat.
pub fn capabilities() -> Capabilities {
    Capabilities {
        text: true,
        files: true,
        voice_notes: false,
        video_notes: false,
        custom_emoji: false,
        reactions: ReactionStyle::Counts,
        gif: GifSource::None,
        stickers: StickerAccess::None,
        threads: true,
        spaces: true,
        // Bots cannot send typing notices to Discord.
        typing: false,
        read_receipts: false,
        edit: true,
        delete_for_everyone: true,
        polls: false,
        calls: CallAccess::None,
    }
}

/// The row of a guild. Guilds are spaces; the rail shows them.
pub fn guild_chat(account: AccountId, guild: u64, name: &str) -> Chat {
    let mut chat = Chat::new(guild_id(account, guild), name.to_owned());
    chat.space = true;
    chat
}

/// The row of a channel, thread, or direct message. Threads hang under their
/// parent channel, guild channels hang under their guild, and the name of a
/// direct message comes from the people in it.
pub fn channel_chat(
    account: AccountId,
    channel: &Channel,
    guild: Option<(u64, &str)>,
    me: u64,
) -> Chat {
    let id = channel.id.get();
    let thread = channel.kind.is_thread();
    let direct = channel.kind == ChannelType::Private || channel.kind == ChannelType::Group;
    let name = channel.name.clone().unwrap_or_else(|| {
        let people = channel
            .recipients
            .as_deref()
            .unwrap_or_default()
            .iter()
            .filter(|user| user.id.get() != me)
            .map(user_name)
            .collect::<Vec<_>>();
        match people.len() {
            0 => "Direct message".to_owned(),
            1 => people[0].clone(),
            count => format!("{} and {} more", people[0], count - 1),
        }
    });
    let mut chat = Chat::new(channel_id(account, id), name);
    chat.kind = if direct {
        ChatKind::Direct
    } else {
        ChatKind::Group
    };
    chat.parent = if thread {
        channel
            .parent_id
            .map(|parent| channel_id(account, parent.get()))
    } else {
        guild.map(|(guild, _)| guild_id(account, guild))
    };
    chat
}

/// The display name of a user.
pub fn user_name(user: &User) -> String {
    user.global_name
        .clone()
        .unwrap_or_else(|| user.name.clone())
}

/// Projects one message. Returns nothing for messages with no content at all,
/// which are the system entries the window does not show.
pub fn message(
    _account: AccountId,
    chat: &ChatId,
    message: &DiscordMessage,
    me: u64,
    sender_name: Option<String>,
    quoted: Option<Quoted>,
) -> Option<Message> {
    let content = content_of(message)?;
    let from_me = message.author.id.get() == me;
    Some(Message {
        id: message.id.get().to_string(),
        chat: chat.clone(),
        sender: message.author.id.get().to_string(),
        sender_name,
        from_me,
        timestamp: message.timestamp.as_secs(),
        content,
        status: if from_me {
            Delivery::Sent
        } else {
            Delivery::None
        },
        delivered_at: None,
        read_at: None,
        quoted,
        reactions: reactions(&message.reactions),
        edited: message.edited_timestamp.is_some(),
        mentions: Vec::new(),
        forwarded: false,
        thumbnail: None,
    })
}

/// The reaction rows Discord reports, with their counts.
pub fn reactions(rows: &[DiscordReaction]) -> Vec<Reaction> {
    rows.iter()
        .filter(|row| row.count > 0)
        .map(|row| Reaction {
            sender: String::new(),
            from_me: row.me,
            emoji: reaction_display(&row.emoji),
            count: row.count.min(u64::from(u32::MAX)) as u32,
        })
        .collect()
}

/// How an emoji is named when it is not a plain character.
pub fn reaction_display(emoji: &EmojiReactionType) -> String {
    match emoji {
        EmojiReactionType::Unicode { name } => name.clone(),
        EmojiReactionType::Custom { name, .. } => name
            .as_ref()
            .map(|name| format!(":{name}:"))
            .unwrap_or_else(|| "emoji".to_owned()),
    }
}

fn content_of(message: &DiscordMessage) -> Option<Content> {
    if let Some(sticker) = message.sticker_items.first() {
        return Some(sticker_content(sticker));
    }
    let caption = (!message.content.is_empty()).then(|| message.content.clone());
    if let Some(attachment) = message.attachments.first() {
        return Some(attachment_content(attachment, caption));
    }
    if message.content.is_empty() {
        return None;
    }
    Some(Content::Text {
        text: message.content.clone(),
        preview: None,
    })
}

fn sticker_content(sticker: &MessageSticker) -> Content {
    Content::Sticker {
        media: Media {
            mime: "image/png".to_owned(),
            size: 0,
            width: None,
            height: None,
            path: None,
            state: MediaState::Idle,
        },
        animated: matches!(
            sticker.format_type,
            twilight_model::channel::message::sticker::StickerFormatType::Gif
                | twilight_model::channel::message::sticker::StickerFormatType::Apng
                | twilight_model::channel::message::sticker::StickerFormatType::Lottie
        ),
    }
}

fn attachment_content(attachment: &Attachment, caption: Option<String>) -> Content {
    let mime = attachment.content_type.clone().unwrap_or_default();
    let media = Media {
        mime: mime.clone(),
        size: attachment.size,
        width: attachment
            .width
            .map(|width| width.min(u64::from(u32::MAX)) as u32),
        height: attachment
            .height
            .map(|height| height.min(u64::from(u32::MAX)) as u32),
        path: None,
        state: MediaState::Idle,
    };
    let seconds = attachment
        .duration_secs
        .map(|seconds| seconds.max(0.0).round() as u32);
    if mime.starts_with("image/") {
        Content::Image { caption, media }
    } else if mime.starts_with("video/") {
        Content::Video {
            caption,
            media,
            seconds,
            gif: false,
            note: false,
        }
    } else if mime.starts_with("audio/") {
        Content::Audio {
            media,
            seconds,
            voice_note: mime.contains("ogg"),
            waveform: Vec::new(),
        }
    } else {
        Content::Document {
            media,
            file_name: attachment.filename.clone(),
            caption,
            pages: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use twilight_model::channel::Channel;

    use super::*;

    fn account() -> AccountId {
        AccountId(7)
    }

    fn channel(value: serde_json::Value) -> Channel {
        serde_json::from_value(value).expect("fixture channel")
    }

    fn discord_message(value: serde_json::Value) -> DiscordMessage {
        serde_json::from_value(value).expect("fixture message")
    }

    fn message_json() -> serde_json::Value {
        json!({
            "attachments": [],
            "author": {"id": "7", "username": "Ada", "discriminator": "0", "bot": false},
            "channel_id": "2",
            "content": "hello",
            "embeds": [],
            "id": "10",
            "type": 0,
            "mention_everyone": false,
            "mention_roles": [],
            "mentions": [],
            "message_snapshots": [],
            "pinned": false,
            "reactions": [],
            "sticker_items": [],
            "timestamp": "2024-05-01T12:00:00.000000+00:00",
            "tts": false,
            "guild_id": "1"
        })
    }

    #[test]
    fn peers_round_trip_through_their_stored_form() {
        assert_eq!(parse(&guild_id(account(), 3)), Some(Target::Guild(3)));
        assert_eq!(parse(&channel_id(account(), 4)), Some(Target::Channel(4)));
        assert_eq!(parse(&ChatId::new(account(), "nonsense")), None);
        assert_eq!(parse(&ChatId::new(account(), "x1")), None);
    }

    #[test]
    fn capabilities_stay_honest() {
        let capabilities = capabilities();
        assert!(capabilities.text && capabilities.files);
        assert!(capabilities.threads && capabilities.spaces);
        assert!(capabilities.edit && capabilities.delete_for_everyone);
        assert!(!capabilities.typing, "bots cannot send typing notices");
        assert!(!capabilities.voice_notes && !capabilities.read_receipts);
        assert_eq!(capabilities.reactions, ReactionStyle::Counts);
        assert_eq!(capabilities.calls, CallAccess::None);
    }

    #[test]
    fn guilds_are_spaces() {
        let guild = guild_chat(account(), 1, "Rust Friends");
        assert!(guild.space);
        assert_eq!(guild.id, guild_id(account(), 1));
        assert_eq!(guild.name, "Rust Friends");
    }

    #[test]
    fn text_channels_hang_under_their_guild() {
        let fixture = channel(json!({
            "id": "2",
            "type": 0,
            "guild_id": "1",
            "name": "general"
        }));
        let chat = channel_chat(account(), &fixture, Some((1, "Rust Friends")), 7);
        assert_eq!(chat.kind, ChatKind::Group);
        assert_eq!(chat.parent, Some(guild_id(account(), 1)));
        assert_eq!(chat.name, "general");
    }

    #[test]
    fn threads_hang_under_their_channel() {
        let fixture = channel(json!({
            "id": "5",
            "type": 11,
            "guild_id": "1",
            "parent_id": "2",
            "name": "release plans"
        }));
        let chat = channel_chat(account(), &fixture, Some((1, "Rust Friends")), 7);
        assert_eq!(chat.parent, Some(channel_id(account(), 2)));
        assert_eq!(chat.name, "release plans");
    }

    #[test]
    fn direct_messages_name_their_people() {
        let fixture = channel(json!({
            "id": "9",
            "type": 1,
            "recipients": [
                {"id": "7", "username": "Ada", "discriminator": "0"},
                {"id": "8", "username": "grace", "discriminator": "0", "global_name": "Grace"}
            ]
        }));
        let chat = channel_chat(account(), &fixture, None, 7);
        assert_eq!(chat.kind, ChatKind::Direct);
        assert_eq!(chat.name, "Grace");
        assert_eq!(chat.parent, None);
    }

    #[test]
    fn messages_project_their_author_and_text() {
        let raw = discord_message(message_json());
        let projected = message(
            account(),
            &channel_id(account(), 2),
            &raw,
            99,
            Some("Ada".to_owned()),
            None,
        )
        .expect("a message");
        assert_eq!(projected.id, "10");
        assert_eq!(projected.chat, channel_id(account(), 2));
        assert_eq!(projected.sender, "7");
        assert_eq!(projected.sender_name.as_deref(), Some("Ada"));
        assert!(!projected.from_me);
        assert_eq!(projected.timestamp, 1_714_564_800);
        match projected.content {
            Content::Text { text, .. } => assert_eq!(text, "hello"),
            other => panic!("unexpected content: {other:?}"),
        }
    }

    #[test]
    fn own_messages_are_marked_sent() {
        let raw = discord_message(message_json());
        let projected = message(
            account(),
            &channel_id(account(), 2),
            &raw,
            7,
            Some("Ada".to_owned()),
            None,
        )
        .expect("a message");
        assert!(projected.from_me);
        assert_eq!(projected.status, Delivery::Sent);
    }

    #[test]
    fn attachments_become_media_of_their_kind() {
        let mut value = message_json();
        value["content"] = json!("look");
        value["attachments"] = json!([{
            "id": "77",
            "filename": "cat.png",
            "content_type": "image/png",
            "size": 1234,
            "url": "https://cdn.discordapp.com/attachments/2/77/cat.png",
            "proxy_url": "https://media.discordapp.net/attachments/2/77/cat.png",
            "width": 640,
            "height": 480
        }]);
        let raw = discord_message(value);
        let projected =
            message(account(), &channel_id(account(), 2), &raw, 99, None, None).expect("a message");
        match projected.content {
            Content::Image { caption, media } => {
                assert_eq!(caption.as_deref(), Some("look"));
                assert_eq!(media.mime, "image/png");
                assert_eq!(media.width, Some(640));
                assert_eq!(media.size, 1234);
            }
            other => panic!("unexpected content: {other:?}"),
        }
    }

    #[test]
    fn a_webp_file_stays_a_document_unless_discord_calls_it_a_sticker() {
        let mut value = message_json();
        value["attachments"] = json!([{
            "id": "77",
            "filename": "party.webp",
            "content_type": "image/webp",
            "size": 10,
            "url": "https://cdn.discordapp.com/attachments/2/77/party.webp",
            "proxy_url": "https://media.discordapp.net/attachments/2/77/party.webp"
        }]);
        let raw = discord_message(value);
        let projected =
            message(account(), &channel_id(account(), 2), &raw, 99, None, None).expect("a message");
        assert!(matches!(projected.content, Content::Image { .. }));
    }

    #[test]
    fn stickers_are_their_own_content() {
        let mut value = message_json();
        value["content"] = json!("");
        value["sticker_items"] = json!([{"id": "88", "name": "party", "format_type": 4}]);
        let raw = discord_message(value);
        let projected =
            message(account(), &channel_id(account(), 2), &raw, 99, None, None).expect("a sticker");
        match projected.content {
            Content::Sticker { animated, .. } => assert!(animated, "gif stickers play"),
            other => panic!("unexpected content: {other:?}"),
        }
    }

    #[test]
    fn reactions_carry_their_counts() {
        let rows: Vec<DiscordReaction> = serde_json::from_value(json!([
            {"count": 3, "me": false, "emoji": {"name": "👍"}, "burst_colors": [],
             "count_details": {"burst": 0, "normal": 3}, "me_burst": false},
            {"count": 1, "me": true, "emoji": {"id": "555", "name": "party", "animated": false},
             "burst_colors": [], "count_details": {"burst": 0, "normal": 1}, "me_burst": false}
        ]))
        .expect("fixture reactions");
        let projected = reactions(&rows);
        assert_eq!(projected.len(), 2);
        assert_eq!(projected[0].emoji, "👍");
        assert_eq!(projected[0].count, 3);
        assert!(!projected[0].from_me);
        assert_eq!(projected[1].emoji, ":party:");
        assert!(projected[1].from_me);
    }

    #[test]
    fn the_gateway_reaction_event_shape_deserializes() {
        let reaction: twilight_model::gateway::payload::incoming::ReactionAdd =
            serde_json::from_value(json!({
                "burst": false,
                "burst_colors": [],
                "channel_id": "2",
                "emoji": {"name": "🎉"},
                "guild_id": "1",
                "message_id": "10",
                "user_id": "8",
                "message_author_id": "7"
            }))
            .expect("fixture reaction event");
        assert_eq!(reaction_display(&reaction.emoji), "🎉");
        assert_eq!(reaction.message_id.get(), 10);
    }
}
