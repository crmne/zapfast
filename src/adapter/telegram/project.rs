//! Projection from Telegram protocol values into the window's models.
//!
//! The runtime hands protocol values in; everything here is a pure function of
//! those values, so the window never sees a `tl` type and the projection stays
//! testable with recorded fixtures.

use grammers_client::media::Media as TelegramMedia;
use grammers_client::message::Message as TelegramMessage;
use grammers_client::peer::{Dialog, Peer};
use grammers_client::tl;
use grammers_session::types::{ChannelKind, PeerId, PeerKind};

use crate::account::{
    AccountId, CallAccess, Capabilities, GifSource, ReactionStyle, StickerAccess,
};
use crate::model::{
    AlbumItem, Chat, ChatId, ChatKind, Content, Delivery, LastMessage, Media, MediaState, Message,
    PollState, Quoted, Reaction,
};

/// The peer part of a chat id: the Bot API dialog id, which embeds the kind
/// of peer (user, small group, or channel).
pub fn peer_string(peer: PeerId) -> Option<String> {
    peer.bot_api_dialog_id().map(|id| id.to_string())
}

/// Reads back a peer from its stored chat id.
pub fn parse_peer(text: &str) -> Option<PeerId> {
    text.parse::<i64>()
        .ok()
        .and_then(PeerId::from_bot_api_dialog_id)
}

/// The chat id for a Telegram peer on one account.
pub fn chat_id(account: AccountId, peer: PeerId) -> Option<ChatId> {
    chat_id_for(account, peer, None)
}

/// The chat key of a forum topic: the group's peer, then the topic id.
pub fn topic_string(peer: PeerId, topic: i64) -> Option<String> {
    Some(format!("{}:{topic}", peer_string(peer)?))
}

/// Splits a chat key into its peer and its forum topic id, when it has one.
pub fn parse_chat(text: &str) -> Option<(PeerId, Option<i64>)> {
    match text.split_once(':') {
        Some((peer, topic)) => {
            let topic: i64 = topic.parse().ok()?;
            Some((parse_peer(peer)?, (topic != 0).then_some(topic)))
        }
        None => Some((parse_peer(text)?, None)),
    }
}

/// The chat id for a peer, narrowed to a forum topic when there is one.
pub fn chat_id_for(account: AccountId, peer: PeerId, topic: Option<i64>) -> Option<ChatId> {
    let text = match topic {
        Some(topic) => topic_string(peer, topic)?,
        None => peer_string(peer)?,
    };
    Some(ChatId::new(account, text))
}

/// What kind of row a Telegram peer becomes in the chat list.
pub fn peer_chat_kind(kind: PeerKind) -> ChatKind {
    match kind {
        PeerKind::User => ChatKind::Direct,
        PeerKind::Chat => ChatKind::Group,
        PeerKind::Channel => ChatKind::Group,
    }
}

/// What kind of row a channel becomes. Only broadcast channels are read-only;
/// megagroups, gigagroups, and communities behave like groups.
pub fn channel_chat_kind(kind: Option<ChannelKind>) -> ChatKind {
    match kind {
        Some(ChannelKind::Broadcast) => ChatKind::Broadcast,
        _ => ChatKind::Group,
    }
}

/// What a Telegram account can do in an open chat.
pub fn capabilities() -> Capabilities {
    Capabilities {
        text: true,
        files: true,
        voice_notes: true,
        video_notes: true,
        custom_emoji: true,
        threads: false,
        spaces: false,
        typing: true,
        // Telegram does not report the other side's read state here yet, and
        // GIF search and sticker shelves wait for their follow-up work.
        read_receipts: false,
        edit: true,
        delete_for_everyone: true,
        polls: false,
        reactions: ReactionStyle::Counts,
        gif: GifSource::None,
        stickers: StickerAccess::None,
        calls: CallAccess::LogOnly,
    }
}

/// A call entry from the chat's timeline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CallLog {
    pub video: bool,
    pub missed: bool,
    pub duration: Option<i32>,
}

/// The call behind a service message, when it is one.
pub fn call_log(action: &tl::enums::MessageAction) -> Option<CallLog> {
    let tl::enums::MessageAction::PhoneCall(call) = action else {
        return None;
    };
    Some(CallLog {
        video: call.video,
        missed: matches!(
            call.reason.as_ref(),
            Some(tl::enums::PhoneCallDiscardReason::Missed)
        ),
        duration: call.duration,
    })
}

/// The reactions with their counts, one row per emoji.
pub fn reactions(reactions: Option<&tl::enums::MessageReactions>) -> Vec<Reaction> {
    let Some(tl::enums::MessageReactions::Reactions(reactions)) = reactions else {
        return Vec::new();
    };
    reactions
        .results
        .iter()
        .filter_map(|row| {
            let tl::enums::ReactionCount::Count(row) = row;
            let tl::enums::Reaction::Emoji(emoji) = &row.reaction else {
                return None;
            };
            Some(Reaction {
                sender: String::new(),
                from_me: row.chosen_order.is_some(),
                emoji: emoji.emoticon.clone(),
                count: row.count.max(0) as u32,
            })
        })
        .collect()
}

/// The text with Telegram's formatting entities turned into the markup the
/// painter already understands. Offsets count UTF-16 code units, the way
/// Telegram reports them.
pub fn entities_text(text: &str, entities: &[tl::enums::MessageEntity]) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut marked: Vec<Marked> = Vec::new();
    for entity in entities {
        let Some((open, close)) = markers(entity) else {
            continue;
        };
        let length = entity.length();
        if length <= 0 {
            continue;
        }
        let Some(start) = char_boundary(&chars, entity.offset()) else {
            continue;
        };
        // An entity that runs past the end covers the rest of the text.
        let end =
            char_boundary(&chars, entity.offset().saturating_add(length)).unwrap_or(chars.len());
        marked.push(Marked {
            start,
            end,
            open,
            close,
        });
    }
    if marked.is_empty() {
        return text.to_owned();
    }
    let mut opens: Vec<&Marked> = marked.iter().collect();
    opens.sort_by_key(|marked| (marked.start, std::cmp::Reverse(marked.end)));
    let mut closes: Vec<&Marked> = marked.iter().collect();
    closes.sort_by_key(|marked| (marked.end, std::cmp::Reverse(marked.start)));
    let mut out = String::new();
    let mut next_open = 0;
    let mut next_close = 0;
    for index in 0..=chars.len() {
        while let Some(marked) = closes.get(next_close) {
            if marked.end != index {
                break;
            }
            out.push_str(marked.close);
            next_close += 1;
        }
        while let Some(marked) = opens.get(next_open) {
            if marked.start != index {
                break;
            }
            out.push_str(marked.open);
            next_open += 1;
        }
        if let Some(character) = chars.get(index) {
            out.push(*character);
        }
    }
    out
}

struct Marked {
    start: usize,
    end: usize,
    open: &'static str,
    close: &'static str,
}

/// The markup pair for the entity kinds the painter styles. Kinds without a
/// pair (links, mentions, hashes) arrive with the text and need no markers.
fn markers(entity: &tl::enums::MessageEntity) -> Option<(&'static str, &'static str)> {
    Some(match entity {
        tl::enums::MessageEntity::Bold(_) => ("*", "*"),
        tl::enums::MessageEntity::Italic(_) => ("_", "_"),
        tl::enums::MessageEntity::Underline(_) => ("_", "_"),
        tl::enums::MessageEntity::Strike(_) => ("~", "~"),
        tl::enums::MessageEntity::Code(_) => ("`", "`"),
        tl::enums::MessageEntity::Pre(_) => ("```", "```"),
        _ => return None,
    })
}

/// The character position for a UTF-16 offset, when it lands on a boundary.
fn char_boundary(chars: &[char], offset: i32) -> Option<usize> {
    if offset < 0 {
        return None;
    }
    let mut units = 0i32;
    for (index, character) in chars.iter().enumerate() {
        if units == offset {
            return Some(index);
        }
        units += character.len_utf16() as i32;
        if units > offset {
            return None;
        }
    }
    (units == offset).then_some(chars.len())
}

/// The forum topic a raw message belongs to, from its reply header.
pub fn topic_of(raw: &tl::enums::Message) -> Option<i64> {
    let tl::enums::Message::Message(message) = raw else {
        return None;
    };
    message.reply_to.as_ref().and_then(topic_from_reply)
}

/// The forum topic a reply header names, when it marks one.
fn topic_from_reply(reply: &tl::enums::MessageReplyHeader) -> Option<i64> {
    let tl::enums::MessageReplyHeader::Header(reply) = reply else {
        return None;
    };
    if !reply.forum_topic {
        return None;
    }
    reply
        .reply_to_top_id
        .or(reply.reply_to_msg_id)
        .map(i64::from)
}

/// A chat-list row for one dialog.
pub fn chat(account: AccountId, dialog: &Dialog, me: Option<PeerId>) -> Option<Chat> {
    let peer = &dialog.peer;
    let id = chat_id(account, peer.id())?;
    let name = peer
        .name()
        .map(str::to_owned)
        .unwrap_or_else(|| id.to_string());
    let mut chat = Chat::new(id, name);
    chat.kind = match peer {
        Peer::User(_) => peer_chat_kind(PeerKind::User),
        Peer::Group(_) => peer_chat_kind(PeerKind::Chat),
        Peer::Channel(channel) => channel_chat_kind(channel.kind()),
        Peer::Community(_) => ChatKind::Group,
    };
    if let tl::enums::Dialog::Dialog(raw) = &dialog.raw {
        chat.unread = raw.unread_count.max(0) as u32;
        chat.pinned = raw.pinned;
    }
    if let Some(last) = dialog.last_message.as_ref()
        && let Some(message) = message(account, last, me, None, None)
    {
        chat.last_activity = message.timestamp;
        let summary = message.summary();
        chat.last = Some(LastMessage {
            from_me: message.from_me,
            sender: message.sender.clone(),
            sender_name: message.sender_name.clone(),
            summary,
            full: message.summary(),
            status: message.status,
        });
    }
    Some(chat)
}

/// A chat-list row for one forum topic of a group.
pub fn topic_chat(
    account: AccountId,
    peer: PeerId,
    parent: &ChatId,
    topic: &tl::types::ForumTopic,
) -> Option<Chat> {
    let id = chat_id_for(account, peer, Some(i64::from(topic.id)))?;
    let mut chat = Chat::new(id, topic.title.clone());
    chat.kind = ChatKind::Group;
    chat.parent = Some(parent.clone());
    chat.unread = topic.unread_count.max(0) as u32;
    chat.pinned = topic.pinned;
    Some(chat)
}

/// One message projected for the window. `quoted` is the reply target, which
/// the runtime looks up before projecting; `topic` is the forum topic the
/// caller asked for, when the message itself does not carry one.
pub fn message(
    account: AccountId,
    message: &TelegramMessage,
    me: Option<PeerId>,
    quoted: Option<Quoted>,
    topic: Option<i64>,
) -> Option<Message> {
    let topic = topic_of(&message.raw).or(topic);
    let chat = chat_id_for(account, message.peer_id(), topic)?;
    let from_me = message.outgoing()
        || (me.is_some() && message.sender_id().is_some() && message.sender_id() == me);
    let sender = message
        .sender_id()
        .and_then(peer_string)
        .unwrap_or_default();
    let sender_name = message
        .sender()
        .and_then(|peer| peer.name())
        .map(str::to_owned);
    let reactions = match &message.raw {
        tl::enums::Message::Message(raw) => reactions(raw.reactions.as_ref()),
        tl::enums::Message::Service(raw) => reactions(raw.reactions.as_ref()),
        _ => Vec::new(),
    };
    Some(Message {
        id: message.id().to_string(),
        chat,
        sender,
        sender_name,
        from_me,
        timestamp: message.date().as_second(),
        content: content(message)?,
        status: if from_me {
            Delivery::Sent
        } else {
            Delivery::None
        },
        delivered_at: None,
        read_at: None,
        quoted,
        reactions,
        edited: message.edit_date().is_some(),
        mentions: Vec::new(),
        forwarded: message.forward_header().is_some(),
        thumbnail: None,
    })
}

/// The quote a reply message points at.
pub fn quoted(reply: &TelegramMessage) -> Quoted {
    let sender = reply.sender_id().and_then(peer_string).unwrap_or_default();
    let sender_name = reply
        .sender()
        .and_then(|peer| peer.name())
        .map(str::to_owned);
    let summary = content(reply)
        .map(|content| content.summary())
        .unwrap_or_default();
    Quoted {
        id: reply.id().to_string(),
        sender,
        sender_name,
        summary,
        mentions: Vec::new(),
    }
}

/// What a message carries. Service messages the client cannot show become
/// unsupported rows; call entries keep their history.
pub fn content(message: &TelegramMessage) -> Option<Content> {
    if let Some(action) = message.action() {
        let log = call_log(action)?;
        return Some(Content::Call {
            video: log.video,
            missed: log.missed,
            seconds: log.duration.map(|seconds| seconds.max(0) as u32),
        });
    }
    if let Some(media) = message.media() {
        return Some(media_content(&media, message));
    }
    let text = message.text();
    if text.is_empty() {
        return Some(Content::Unsupported {
            what: "Message".to_owned(),
        });
    }
    let entities = message
        .fmt_entities()
        .map(Vec::as_slice)
        .unwrap_or_default();
    Some(Content::Text {
        text: entities_text(text, entities),
        preview: None,
    })
}

/// The plain text of a Telegram text-with-entities value.
fn entities_plain(text: &tl::enums::TextWithEntities) -> String {
    match text {
        tl::enums::TextWithEntities::Entities(text) => text.text.clone(),
    }
}

/// A poll with its server-side results.
fn poll_content(poll: &grammers_client::media::Poll) -> Content {
    let options: Vec<String> = poll
        .iter_answers()
        .map(|answer| match answer {
            tl::enums::PollAnswer::Answer(answer) => entities_plain(&answer.text),
            tl::enums::PollAnswer::InputPollAnswer(_) => String::new(),
        })
        .collect();
    let mut counts = vec![0usize; options.len()];
    let mut selected = Vec::new();
    if let Some(summary) = poll.iter_voters_summary() {
        for (index, voters) in summary.enumerate() {
            let count = voters.voters.unwrap_or(0).max(0) as usize;
            if let Some(slot) = counts.get_mut(index) {
                *slot = count;
            }
            if voters.chosen {
                selected.push(index);
            }
        }
    }
    Content::Poll {
        question: entities_plain(&poll.raw.question),
        options,
        state: PollState {
            selectable: if poll.raw.multiple_choice {
                poll.raw.answers.len()
            } else {
                1
            },
            counts,
            selected,
            voters: poll.total_voters().unwrap_or(0).max(0) as usize,
            can_vote: !poll.closed(),
            history_complete: true,
            refresh_needed: false,
            refreshing: false,
            refresh_failed: false,
            votes: Vec::new(),
        },
    }
}

/// A media message's content, from the shape the protocol sent.
fn media_content(media: &TelegramMedia, message: &TelegramMessage) -> Content {
    let text = message.text();
    let entities = message
        .fmt_entities()
        .map(Vec::as_slice)
        .unwrap_or_default();
    let caption = (!text.is_empty()).then(|| entities_text(text, entities));
    match media {
        TelegramMedia::Photo(photo) => Content::Image {
            caption,
            media: Media {
                mime: "image/jpeg".to_owned(),
                size: photo.size().unwrap_or(0) as u64,
                width: None,
                height: None,
                path: None,
                state: MediaState::Idle,
            },
        },
        TelegramMedia::Document(document) => {
            let mime = document
                .mime_type()
                .unwrap_or("application/octet-stream")
                .to_owned();
            let (width, height) = match document.resolution() {
                Some((width, height)) => (Some(width.max(0) as u32), Some(height.max(0) as u32)),
                None => (None, None),
            };
            let seconds = document.duration().map(|seconds| seconds.max(0.0) as u32);
            let media = Media {
                mime: mime.clone(),
                size: document.size().unwrap_or(0) as u64,
                width,
                height,
                path: None,
                state: MediaState::Idle,
            };
            if document.is_animated() {
                return Content::Video {
                    caption,
                    media,
                    seconds,
                    gif: true,
                    note: false,
                };
            }
            if mime.starts_with("audio/") {
                return Content::Audio {
                    media,
                    seconds,
                    voice_note: mime.contains("ogg"),
                    waveform: Vec::new(),
                };
            }
            if mime.starts_with("video/") {
                return Content::Video {
                    caption,
                    media,
                    seconds,
                    gif: false,
                    note: false,
                };
            }
            Content::Document {
                media,
                file_name: document.name().unwrap_or_default().to_owned(),
                caption,
                pages: None,
            }
        }
        TelegramMedia::Sticker(sticker) => {
            let (width, height) = sticker
                .document
                .resolution()
                .map_or((None, None), |(width, height)| {
                    (Some(width.max(0) as u32), Some(height.max(0) as u32))
                });
            Content::Sticker {
                media: Media {
                    // Animated stickers arrive as application/x-tgsticker and
                    // video stickers as video/webm; the download turns the
                    // animated one into the WebP the player decodes.
                    mime: sticker
                        .document
                        .mime_type()
                        .unwrap_or("image/webp")
                        .to_owned(),
                    size: sticker.document.size().unwrap_or(0) as u64,
                    width,
                    height,
                    path: None,
                    state: MediaState::Idle,
                },
                animated: sticker.is_animated(),
            }
        }
        TelegramMedia::Geo(geo) => Content::Location {
            latitude: geo.latitue(),
            longitude: geo.longitude(),
            name: None,
            address: None,
        },
        TelegramMedia::Contact(contact) => {
            let display_name = format!("{} {}", contact.raw.first_name, contact.raw.last_name)
                .trim()
                .to_owned();
            Content::Contact {
                display_name,
                vcard: contact.vcard().to_owned(),
            }
        }
        TelegramMedia::Poll(poll) => poll_content(poll),
        _ => Content::Unsupported {
            what: "Media".to_owned(),
        },
    }
}

/// One album item from a projected message, when it holds a photo or a video.
fn item_of(content: &Content) -> Option<(Option<String>, AlbumItem)> {
    match content {
        Content::Image { caption, media } => Some((
            caption.clone(),
            AlbumItem {
                media: media.clone(),
                seconds: None,
                gif: false,
            },
        )),
        Content::Video {
            caption,
            media,
            seconds,
            gif,
            note,
        } if !note => Some((
            caption.clone(),
            AlbumItem {
                media: media.clone(),
                seconds: *seconds,
                gif: *gif,
            },
        )),
        _ => None,
    }
}

/// The album behind a message's content, either already grouped or one item.
fn take_album(content: &Content) -> Option<(Option<String>, Vec<AlbumItem>)> {
    match content {
        Content::Album { caption, items } => Some((caption.clone(), items.clone())),
        _ => item_of(content).map(|(caption, item)| (caption, vec![item])),
    }
}

/// Merges consecutive Telegram album items into one grouped bubble. Items
/// without a group id, or that are not photos and videos, stay on their own.
pub fn album_bubbles(items: Vec<(Option<i64>, Message)>) -> Vec<Message> {
    let mut out: Vec<(Option<i64>, Message)> = Vec::new();
    for (group, message) in items {
        if let Some(group) = group
            && let Some((last_group, last)) = out.last_mut()
            && *last_group == Some(group)
            && let Some((caption, item)) = item_of(&message.content)
            && let Some((mut album_caption, mut album)) = take_album(&last.content)
        {
            album.push(item);
            if album_caption.is_none() {
                album_caption = caption;
            }
            last.content = Content::Album {
                caption: album_caption,
                items: album,
            };
            continue;
        }
        out.push((group, message));
    }
    out.into_iter().map(|(_, message)| message).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bold(offset: i32, length: i32) -> tl::enums::MessageEntity {
        tl::enums::MessageEntity::Bold(tl::types::MessageEntityBold { offset, length })
    }

    fn italic(offset: i32, length: i32) -> tl::enums::MessageEntity {
        tl::enums::MessageEntity::Italic(tl::types::MessageEntityItalic { offset, length })
    }

    #[test]
    fn peer_ids_round_trip_through_their_stored_form() {
        for peer in [
            PeerId::user(42).unwrap(),
            PeerId::chat(77).unwrap(),
            PeerId::channel(123456).unwrap(),
        ] {
            let text = peer_string(peer).expect("a dialog id");
            assert_eq!(parse_peer(&text), Some(peer));
            assert_eq!(peer_string(peer).as_deref(), Some(text.as_str()));
        }
        assert_eq!(parse_peer("not a peer"), None);
        assert_eq!(parse_peer("0"), None);
    }

    #[test]
    fn bold_and_italic_entities_become_markup() {
        assert_eq!(entities_text("hello world", &[bold(0, 5)]), "*hello* world");
        assert_eq!(entities_text("abc", &[bold(0, 3)]), "*abc*");
        assert_eq!(entities_text("abc", &[italic(1, 1)]), "a_b_c");
        assert_eq!(
            entities_text("both", &[bold(0, 4), italic(0, 2)]),
            "*_bo_th*"
        );
    }

    #[test]
    fn emoji_offsets_count_utf16_units() {
        assert_eq!(entities_text("a😀b", &[bold(0, 3)]), "*a😀*b");
        assert_eq!(entities_text("a😀b", &[bold(1, 2)]), "a*😀*b");
    }

    #[test]
    fn stray_or_unknown_entities_stay_plain() {
        assert_eq!(entities_text("hi", &[bold(0, 99)]), "*hi*");
        assert_eq!(entities_text("hi", &[bold(-1, 2)]), "hi");
        assert_eq!(entities_text("hi", &[bold(1, 5)]), "h*i*");
        let custom = tl::enums::MessageEntity::CustomEmoji(tl::types::MessageEntityCustomEmoji {
            offset: 0,
            length: 2,
            document_id: 9,
        });
        assert_eq!(entities_text("hi", &[custom]), "hi");
    }

    #[test]
    fn reactions_carry_their_counts() {
        let raw = tl::enums::MessageReactions::Reactions(tl::types::MessageReactions {
            min: false,
            can_see_list: true,
            reactions_as_tags: false,
            results: vec![
                tl::enums::ReactionCount::Count(tl::types::ReactionCount {
                    chosen_order: None,
                    reaction: tl::enums::Reaction::Emoji(tl::types::ReactionEmoji {
                        emoticon: "👍".to_owned(),
                    }),
                    count: 3,
                }),
                tl::enums::ReactionCount::Count(tl::types::ReactionCount {
                    chosen_order: Some(1),
                    reaction: tl::enums::Reaction::Emoji(tl::types::ReactionEmoji {
                        emoticon: "❤️".to_owned(),
                    }),
                    count: 1,
                }),
            ],
            recent_reactions: None,
            top_reactors: None,
        });
        assert_eq!(
            reactions(Some(&raw)),
            vec![
                Reaction {
                    sender: String::new(),
                    from_me: false,
                    emoji: "👍".to_owned(),
                    count: 3,
                },
                Reaction {
                    sender: String::new(),
                    from_me: true,
                    emoji: "❤️".to_owned(),
                    count: 1,
                },
            ]
        );
        assert!(reactions(None).is_empty());
    }

    #[test]
    fn call_actions_become_call_entries() {
        let call = |video: bool,
                    reason: Option<tl::enums::PhoneCallDiscardReason>,
                    duration: Option<i32>| {
            tl::enums::MessageAction::PhoneCall(tl::types::MessageActionPhoneCall {
                video,
                call_id: 7,
                reason,
                duration,
            })
        };
        assert_eq!(
            call_log(&call(
                false,
                Some(tl::enums::PhoneCallDiscardReason::Missed),
                None
            )),
            Some(CallLog {
                video: false,
                missed: true,
                duration: None,
            })
        );
        assert_eq!(
            call_log(&call(
                true,
                Some(tl::enums::PhoneCallDiscardReason::Hangup),
                Some(12)
            )),
            Some(CallLog {
                video: true,
                missed: false,
                duration: Some(12),
            })
        );
        assert_eq!(call_log(&tl::enums::MessageAction::Empty), None);
    }

    #[test]
    fn telegram_capabilities_count_reactions_and_keep_calls_as_history() {
        let capabilities = capabilities();
        assert!(capabilities.text && capabilities.files && capabilities.voice_notes);
        assert!(capabilities.custom_emoji && capabilities.video_notes);
        assert!(!capabilities.threads && !capabilities.spaces && !capabilities.polls);
        assert_eq!(capabilities.reactions, ReactionStyle::Counts);
        assert_eq!(capabilities.gif, GifSource::None);
        assert_eq!(capabilities.stickers, StickerAccess::None);
        assert_eq!(capabilities.calls, CallAccess::LogOnly);
    }

    #[test]
    fn topic_keys_round_trip_through_their_stored_form() {
        let peer = PeerId::user(123).expect("a user id");
        let account = AccountId::WHATSAPP;
        assert_eq!(topic_string(peer, 456).as_deref(), Some("123:456"));
        assert_eq!(parse_chat("123:456"), Some((peer, Some(456))));
        assert_eq!(parse_chat("123"), Some((peer, None)));
        assert_eq!(parse_chat("123:0"), Some((peer, None)));
        assert_eq!(parse_chat("nonsense"), None);
        assert_eq!(
            chat_id_for(account, peer, Some(456)).map(|chat| chat.as_str().to_owned()),
            topic_string(peer, 456)
        );
        assert_eq!(
            chat_id_for(account, peer, None).map(|chat| chat.peer().to_owned()),
            Some(peer.to_string())
        );
    }

    fn reply_header(
        top: Option<i32>,
        message: Option<i32>,
        forum: bool,
    ) -> tl::enums::MessageReplyHeader {
        tl::enums::MessageReplyHeader::Header(tl::types::MessageReplyHeader {
            reply_to_scheduled: false,
            forum_topic: forum,
            quote: false,
            reply_to_ephemeral: false,
            reply_to_msg_id: message,
            reply_to_peer_id: None,
            reply_from: None,
            reply_media: None,
            reply_to_top_id: top,
            quote_text: None,
            quote_entities: None,
            quote_offset: None,
            todo_item_id: None,
            poll_option: None,
        })
    }

    #[test]
    fn forum_topic_replies_name_their_topic() {
        assert_eq!(
            topic_from_reply(&reply_header(Some(7), Some(9), true)),
            Some(7)
        );
        assert_eq!(
            topic_from_reply(&reply_header(None, Some(9), true)),
            Some(9)
        );
        assert_eq!(
            topic_from_reply(&reply_header(Some(7), Some(9), false)),
            None
        );
        assert_eq!(
            topic_from_reply(&tl::enums::MessageReplyHeader::MessageReplyStoryHeader(
                tl::types::MessageReplyStoryHeader {
                    peer: tl::enums::Peer::User(tl::types::PeerUser { user_id: 1 }),
                    story_id: 2,
                }
            )),
            None
        );
    }

    fn poll_answer(text: &str, option: u8) -> tl::enums::PollAnswer {
        tl::enums::PollAnswer::Answer(tl::types::PollAnswer {
            text: tl::enums::TextWithEntities::Entities(tl::types::TextWithEntities {
                text: text.to_owned(),
                entities: Vec::new(),
            }),
            option: vec![option],
            media: None,
            added_by: None,
            date: None,
        })
    }

    fn raw_poll() -> grammers_client::media::Poll {
        grammers_client::media::Poll::from_raw_media(tl::types::MessageMediaPoll {
            poll: tl::enums::Poll::Poll(tl::types::Poll {
                id: 9,
                closed: false,
                public_voters: true,
                multiple_choice: false,
                quiz: false,
                open_answers: false,
                revoting_disabled: false,
                shuffle_answers: false,
                hide_results_until_close: false,
                creator: false,
                subscribers_only: false,
                question: tl::enums::TextWithEntities::Entities(tl::types::TextWithEntities {
                    text: "Ship it?".to_owned(),
                    entities: Vec::new(),
                }),
                answers: vec![poll_answer("Yes", 1), poll_answer("No", 2)],
                close_period: None,
                close_date: None,
                countries_iso2: None,
                hash: 0,
            }),
            results: tl::enums::PollResults::Results(Box::new(tl::types::PollResults {
                min: false,
                has_unread_votes: false,
                can_view_stats: true,
                results: Some(vec![
                    tl::enums::PollAnswerVoters::Voters(tl::types::PollAnswerVoters {
                        chosen: true,
                        correct: false,
                        option: vec![1],
                        voters: Some(3),
                        recent_voters: None,
                    }),
                    tl::enums::PollAnswerVoters::Voters(tl::types::PollAnswerVoters {
                        chosen: false,
                        correct: false,
                        option: vec![2],
                        voters: Some(1),
                        recent_voters: None,
                    }),
                ]),
                total_voters: Some(4),
                recent_voters: None,
                solution: None,
                solution_entities: None,
                solution_media: None,
            })),
            attached_media: None,
        })
    }

    #[test]
    fn polls_project_their_questions_options_and_counts() {
        let Content::Poll {
            question,
            options,
            state,
        } = poll_content(&raw_poll())
        else {
            panic!("a poll projects as a poll");
        };
        assert_eq!(question, "Ship it?");
        assert_eq!(options, ["Yes", "No"]);
        assert_eq!(state.counts, [3, 1]);
        assert_eq!(state.selected, [0]);
        assert_eq!(state.voters, 4);
        assert!(state.can_vote);
        assert_eq!(state.selectable, 1);
        assert!(state.history_complete);
    }

    fn photo(id: &str, group: Option<i64>, caption: Option<String>) -> (Option<i64>, Message) {
        (
            group,
            Message {
                id: id.to_owned(),
                chat: ChatId::whatsapp("chat"),
                sender: "ada".to_owned(),
                sender_name: Some("Ada".to_owned()),
                from_me: false,
                timestamp: 1,
                content: Content::Image {
                    caption,
                    media: Media {
                        mime: "image/jpeg".to_owned(),
                        size: 1,
                        width: None,
                        height: None,
                        path: None,
                        state: MediaState::Idle,
                    },
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
            },
        )
    }

    #[test]
    fn albums_merge_only_consecutive_items_of_the_same_group() {
        let merged = album_bubbles(vec![
            photo("1", Some(5), Some("Trip".to_owned())),
            photo("2", Some(5), None),
            photo("3", None, None),
            photo("4", Some(6), None),
            photo("5", Some(6), None),
        ]);
        assert_eq!(merged.len(), 3);
        assert!(matches!(
            &merged[0].content,
            Content::Album { caption: Some(caption), items }
                if caption == "Trip" && items.len() == 2
        ));
        assert_eq!(merged[0].id, "1");
        assert!(matches!(merged[1].content, Content::Image { .. }));
        assert!(matches!(
            &merged[2].content,
            Content::Album { caption: None, items } if items.len() == 2
        ));
    }
}
