//! UI models for chats, messages, and view actions.
//!
//! The backend translates protocol types into these models, keeping protobufs
//! out of views and giving the archive a stable shape.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use serde::{Deserialize, Serialize};

/// Chat JID string: `<phone>@s.whatsapp.net`, `<id>@g.us`, or `<id>@lid`.
pub type ChatId = String;

/// Live activity another participant is performing in a chat.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChatActivity {
    Typing,
    RecordingAudio,
}

/// Stable folder name for a linked WhatsApp account on this computer.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AccountId(pub String);

impl AccountId {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn first() -> Self {
        Self("1".into())
    }

    /// Folder name allocated by the roster: one or more digits, no leading
    /// zero, so it cannot be an absolute path or climb out of `accounts/`.
    pub fn is_safe(value: &str) -> bool {
        let mut chars = value.chars();
        let Some(first) = chars.next() else {
            return false;
        };
        first.is_ascii_digit() && first != '0' && chars.all(|character| character.is_ascii_digit())
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::is_safe(value).then(|| Self(value.to_owned()))
    }
}

impl std::fmt::Display for AccountId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for AccountId {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

/// A composer quote, optionally from a different chat (a private group reply).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplyTarget {
    pub id: String,
    pub chat: Option<ChatId>,
}

impl ReplyTarget {
    /// Returns the explicit group source, or the destination for an ordinary same-chat reply.
    pub fn source_chat<'a>(&'a self, destination: &'a str) -> &'a str {
        self.chat.as_deref().unwrap_or(destination)
    }
}

impl From<String> for ReplyTarget {
    /// Wraps a same-chat message id without inventing a cross-chat source.
    fn from(id: String) -> Self {
        Self { id, chat: None }
    }
}

impl From<&str> for ReplyTarget {
    /// Wraps a same-chat message id without inventing a cross-chat source.
    fn from(id: &str) -> Self {
        id.to_owned().into()
    }
}

impl std::ops::Deref for ReplyTarget {
    type Target = str;

    /// Exposes the message id for existing transcript lookup helpers.
    fn deref(&self) -> &str {
        &self.id
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChatKind {
    Direct,
    Group,
    /// Read-only newsletter or broadcast list.
    Broadcast,
}

impl ChatKind {
    pub fn from_id(id: &str) -> Self {
        match id.rsplit('@').next() {
            Some("g.us") => Self::Group,
            Some("newsletter") | Some("broadcast") => Self::Broadcast,
            _ => Self::Direct,
        }
    }
}

/// A local chat label: a name, a colour, and nothing that leaves this computer.
/// Not a WhatsApp Business label; ZapFast neither reads nor syncs those.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Label {
    pub id: String,
    pub name: String,
    /// `#rrggbb`, lower case.
    pub color_hex: String,
    pub created_at: i64,
}

/// Chat-list filter chosen from the chips under the search field.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ChatFilter {
    #[default]
    All,
    Unread,
    /// One-to-one chats: neither groups nor broadcasts.
    Private,
    /// Chats marked as a favorite, here or on the phone.
    Favorites,
    Groups,
    /// Followed channels (newsletters), kept out of the other filters as in
    /// the official apps.
    Channels,
}

/// The chat list's filter as settings keep it: a chip or a label. `None` is
/// the All chip.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ListFilter {
    Chip(ChatFilter),
    Label(String),
}

impl ListFilter {
    const LABEL: &'static str = "label:";

    /// The text stored for this filter; `None` for showing every chat.
    pub fn key(chip: ChatFilter, label: Option<&str>) -> Option<String> {
        if let Some(label) = label {
            return Some(format!("{}{label}", Self::LABEL));
        }
        let key = match chip {
            ChatFilter::All => return None,
            ChatFilter::Unread => "unread",
            ChatFilter::Private => "private",
            ChatFilter::Favorites => "favorites",
            ChatFilter::Groups => "groups",
            ChatFilter::Channels => "channels",
        };
        Some(key.to_owned())
    }

    /// Reads a stored filter. Anything unknown, as a later version may
    /// write, shows every chat.
    pub fn parse(key: &str) -> Option<Self> {
        if let Some(label) = key.strip_prefix(Self::LABEL) {
            return (!label.is_empty()).then(|| Self::Label(label.to_owned()));
        }
        let chip = match key {
            "unread" => ChatFilter::Unread,
            "private" => ChatFilter::Private,
            "favorites" => ChatFilter::Favorites,
            "groups" => ChatFilter::Groups,
            "channels" => ChatFilter::Channels,
            _ => return None,
        };
        Some(Self::Chip(chip))
    }
}

impl ChatFilter {
    pub const EVERY: [Self; 6] = [
        Self::All,
        Self::Unread,
        Self::Private,
        Self::Favorites,
        Self::Groups,
        Self::Channels,
    ];

    pub fn label(self, locale: crate::i18n::Locale) -> std::borrow::Cow<'static, str> {
        use crate::i18n::gettext;
        match self {
            Self::All => gettext(locale, "All"),
            Self::Unread => gettext(locale, "Unread"),
            Self::Private => gettext(locale, "Private"),
            Self::Favorites => gettext(locale, "Favorites"),
            Self::Groups => gettext(locale, "Groups"),
            Self::Channels => gettext(locale, "Channels"),
        }
    }

    pub fn matches(self, chat: &Chat) -> bool {
        match self {
            Self::All => !chat.is_channel(),
            Self::Unread => chat.looks_unread() && !chat.is_channel(),
            Self::Private => chat.kind == ChatKind::Direct,
            Self::Favorites => chat.favorite && !chat.is_channel(),
            Self::Groups => chat.kind == ChatKind::Group,
            Self::Channels => chat.is_channel(),
        }
    }
}

/// Call-history filter chosen from the chips under the Calls page's search
/// field.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CallFilter {
    #[default]
    All,
    /// Calls to us that nobody took, group calls included.
    Missed,
}

impl CallFilter {
    pub const EVERY: [Self; 2] = [Self::All, Self::Missed];

    pub fn label(self, locale: crate::i18n::Locale) -> std::borrow::Cow<'static, str> {
        use crate::i18n::gettext;
        match self {
            Self::All => gettext(locale, "All"),
            Self::Missed => gettext(locale, "Missed"),
        }
    }

    /// The text settings keep for this filter; `None` for every call.
    pub fn key(self) -> Option<String> {
        match self {
            Self::All => None,
            Self::Missed => Some("missed".to_owned()),
        }
    }

    /// Reads a stored filter. Anything unknown, as a later version may
    /// write, shows every call.
    pub fn parse(key: Option<&str>) -> Self {
        match key {
            Some("missed") => Self::Missed,
            _ => Self::All,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Chat {
    pub id: ChatId,
    /// Best known address-book, push, or phone-number name.
    pub name: String,
    /// Distinguishes an actual subject "Group" from older cached placeholders.
    pub group_subject_known: bool,
    pub kind: ChatKind,
    /// Latest-message Unix timestamp used for ordering.
    pub last_activity: i64,
    pub unread: u32,
    /// Marked unread here or on another device: the empty unread dot, with no
    /// pending count. Synced with the phone.
    pub marked_unread: bool,
    pub archived: bool,
    pub pinned: bool,
    /// Pin time in Unix milliseconds; zero for older archives with no ordering.
    pub pinned_at: i64,
    /// Mute end as Unix seconds; `Some(0)` means indefinite.
    pub muted_until: Option<i64>,
    /// Latest message shown in the chat list.
    pub last: Option<LastMessage>,
    /// Whether the chat is one of the favorites, which sync with the phone.
    /// It is not a WhatsApp pin.
    pub favorite: bool,
    /// Place in the phone's favorites list, which orders the Favorites chip.
    pub favorite_position: u32,
    /// Canonical group-member ids, empty until loaded.
    pub participants: Vec<String>,
    /// Group description: `None` until metadata is fetched, `Some("")`
    /// when fetched without a description, otherwise the original text.
    pub group_description: Option<String>,
    /// Whether this is an announcement group where we cannot post.
    pub read_only: bool,
    /// Whether we confirmed leaving this group or channel. Kept apart from
    /// `read_only`, which an announcement group also carries and which a later
    /// metadata refresh rewrites.
    pub left: bool,
    /// Whether only admins may change the group's name and photo (WhatsApp's
    /// "Edit group settings"); `None` until the group's metadata has said.
    pub info_locked: Option<bool>,
    /// Whether we are an admin of this group, as its metadata last said.
    pub admin: bool,
    /// Who may do what in the group, and who its admins are; `None` until
    /// the group's metadata has said.
    pub group_settings: Option<GroupSettings>,
    /// Hidden while WhatsApp chat lock is enabled on the phone.
    pub locked: bool,
    /// Disappearing-message duration in seconds. `Some(0)` is the timer
    /// WhatsApp turned off, which is not the same as never having heard about
    /// it: `None` means nobody has said.
    pub ephemeral_expiration: Option<u32>,
    /// Labels worn by this chat, in creation order. Local to this computer.
    pub labels: Vec<String>,
    /// This chat's own notification sound; `None` follows Settings.
    pub notification_sound: Option<crate::settings::NotificationSound>,
    /// The oldest unread message that mentions us (or everyone), which the
    /// chat list marks with "@" and opening the chat shows.
    pub unread_mention: Option<String>,
    /// This chat's own voice message speed; `None` follows Settings.
    pub voice_speed: Option<f32>,
}

/// A group's permissions and admins, as its metadata says. Kept in the
/// archive with the chat; a change made here lands once WhatsApp accepts it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GroupSettings {
    /// Only admins may send messages (WhatsApp's announcement mode).
    pub announce: bool,
    /// Every member may add people, not only admins.
    pub members_add: bool,
    /// Admins approve people who join through the link.
    pub approval: bool,
    /// Every member may see and share the invite link, not only admins.
    pub members_share_link: bool,
    /// The admins under canonical chat ids, the creator included.
    pub admins: Vec<String>,
    /// The creator (WhatsApp's super admin), under canonical chat ids.
    pub super_admins: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LastMessage {
    pub from_me: bool,
    pub sender: String,
    /// Group-message sender.
    pub sender_name: Option<String>,
    pub summary: String,
    /// The whole message behind `summary`, every line of it: the chat row
    /// shows it in a tooltip when the one-line preview cannot.
    pub full: String,
    /// Whether `summary` and `full` start with a content label such as
    /// "Photo" rather than the message's own text.
    pub labelled: bool,
    pub status: Delivery,
}

impl LastMessage {
    /// `summary` with its content label in `locale`.
    pub fn summary_in(&self, locale: crate::i18n::Locale) -> std::borrow::Cow<'_, str> {
        self.localized(locale, &self.summary)
    }

    /// `full` with its content label in `locale`.
    pub fn full_in(&self, locale: crate::i18n::Locale) -> std::borrow::Cow<'_, str> {
        self.localized(locale, &self.full)
    }

    fn localized<'a>(
        &self,
        locale: crate::i18n::Locale,
        text: &'a str,
    ) -> std::borrow::Cow<'a, str> {
        if self.labelled {
            localized_summary(locale, text)
        } else {
            std::borrow::Cow::Borrowed(text)
        }
    }
}

impl Chat {
    pub fn new(id: ChatId, name: String) -> Self {
        let kind = ChatKind::from_id(&id);
        Self {
            id,
            name,
            group_subject_known: false,
            kind,
            last_activity: 0,
            unread: 0,
            marked_unread: false,
            archived: false,
            pinned: false,
            pinned_at: 0,
            muted_until: None,
            last: None,
            favorite: false,
            favorite_position: 0,
            participants: Vec::new(),
            group_description: None,
            read_only: false,
            left: false,
            info_locked: None,
            admin: false,
            group_settings: None,
            locked: false,
            ephemeral_expiration: None,
            labels: Vec::new(),
            notification_sound: None,
            unread_mention: None,
            voice_speed: None,
        }
    }

    /// Newsletter publishing permissions are not supported by this client.
    pub fn can_send(&self) -> bool {
        !self.read_only && !self.left && self.kind != ChatKind::Broadcast
    }

    /// A followed WhatsApp channel (newsletter).
    pub fn is_channel(&self) -> bool {
        self.id.ends_with("@newsletter")
    }

    pub fn is_group(&self) -> bool {
        self.kind == ChatKind::Group
    }

    /// Never held a message: history sync files such a conversation for
    /// many contacts, and the phone lists none of them.
    pub fn never_messaged(&self) -> bool {
        self.last.is_none() && self.last_activity == 0
    }

    /// Counted unread, or marked unread with nothing pending.
    pub fn looks_unread(&self) -> bool {
        self.unread > 0 || self.marked_unread
    }

    /// Whether Leave is offered. `ours` holds every id we may be listed
    /// under (phone number and privacy id). An empty group member list, or no
    /// known id of ours, means the membership is not known yet, so the group
    /// still offers it. A channel stays leaveable until leaving marks it.
    pub fn can_leave(&self, ours: &[&str]) -> bool {
        // A chat we already left has nothing to leave, even when the phone
        // never told us who was in it. `read_only` cannot say this on its own:
        // an announcement group we are still in carries it too.
        if self.left {
            return false;
        }
        if self.is_channel() {
            return !self.read_only;
        }
        if !self.is_group() {
            return false;
        }
        ours.is_empty() || self.participants.is_empty() || self.lists_any(ours)
    }

    /// Whether we may change the group's name and photo: any member while the
    /// group's info is open to everyone, only admins once it is locked. Until
    /// the metadata says which, nothing is offered, and a group we left is
    /// not ours to edit.
    pub fn can_edit_info(&self) -> bool {
        self.is_group() && !self.left && (self.admin || self.info_locked == Some(false))
    }

    /// Whether we may add people to the group: admins always, other members
    /// only while the group lets every member add (WhatsApp's "Add other
    /// members"). Nothing is offered until the metadata says.
    pub fn can_add_members(&self) -> bool {
        self.is_group()
            && !self.left
            && (self.admin
                || self
                    .group_settings
                    .as_ref()
                    .is_some_and(|settings| settings.members_add))
    }

    /// Whether the group's invite link is shown: to admins, and to every
    /// member while the group lets members share it.
    pub fn can_share_invite_link(&self) -> bool {
        self.is_group()
            && !self.left
            && (self.admin
                || self
                    .group_settings
                    .as_ref()
                    .is_some_and(|settings| settings.members_share_link))
    }

    /// Whether we run the group: its permissions, resetting its link, and
    /// what its members may be made. Only admins do.
    pub fn can_manage_group(&self) -> bool {
        self.is_group() && !self.left && self.admin
    }

    /// Whether `member` is one of the group's admins, as its metadata said.
    pub fn is_group_admin(&self, member: &str) -> bool {
        self.group_settings
            .as_ref()
            .is_some_and(|settings| settings.admins.iter().any(|id| id == member))
    }

    /// Whether `member` created the group, whom nobody can dismiss or remove.
    pub fn is_group_creator(&self, member: &str) -> bool {
        self.group_settings
            .as_ref()
            .is_some_and(|settings| settings.super_admins.iter().any(|id| id == member))
    }

    /// Whether we may mention everyone. A group of at most
    /// [`crate::mentions::OPEN_LIMIT`] people allows any member who can post.
    /// A larger group allows only admins. An empty member list means the
    /// phone has not said who is in, so nothing is offered yet.
    pub fn can_mention_everyone(&self) -> bool {
        self.is_group()
            && self.can_send()
            && !self.participants.is_empty()
            && (self.participants.len() <= crate::mentions::OPEN_LIMIT || self.admin)
    }

    /// Whether the member list names any of `ours`.
    pub fn lists_any(&self, ours: &[&str]) -> bool {
        self.participants
            .iter()
            .any(|id| ours.contains(&id.as_str()))
    }

    pub fn muted(&self, now: i64) -> bool {
        matches!(self.muted_until, Some(0)) || self.muted_until.is_some_and(|until| until > now)
    }

    /// The chat's disappearing-message duration, and only while the timer is
    /// on. A timer WhatsApp turned off is a fact about the chat, not a timer,
    /// so it reads as none here.
    pub fn disappearing_timer(&self) -> Option<u32> {
        self.ephemeral_expiration.filter(|seconds| *seconds > 0)
    }

    /// Direct-chat phone number as digits.
    pub fn phone(&self) -> Option<&str> {
        phone_of(&self.id)
    }
}

/// Extracts digits from a `<phone>@s.whatsapp.net` id.
pub fn phone_of(id: &str) -> Option<&str> {
    let (user, server) = id.split_once('@')?;
    (server == "s.whatsapp.net" && user.chars().all(|c| c.is_ascii_digit())).then_some(user)
}

/// Outgoing-message delivery state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Delivery {
    /// Incoming message without outgoing receipts.
    #[default]
    None,
    /// Sent to the backend but not acknowledged by the server.
    Pending,
    Sent,
    Delivered,
    Read,
    Played,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Message {
    /// WhatsApp message id, unique within a chat.
    pub id: String,
    pub chat: ChatId,
    /// Sender JID, including our own for outgoing messages.
    pub sender: String,
    /// Group sender's push name at receipt time.
    pub sender_name: Option<String>,
    pub from_me: bool,
    /// Unix seconds.
    pub timestamp: i64,
    /// The phone's order within a history conversation, used to break timestamp
    /// ties. Live messages and older archives may not have one.
    #[serde(default)]
    pub history_order: Option<i64>,
    pub content: Content,
    pub status: Delivery,
    /// First delivered-receipt Unix timestamp for outgoing messages.
    #[serde(default)]
    pub delivered_at: Option<i64>,
    /// First read or played receipt Unix timestamp.
    #[serde(default)]
    pub read_at: Option<i64>,
    pub quoted: Option<Quoted>,
    pub reactions: Vec<Reaction>,
    pub edited: bool,
    /// Mentions in the text or caption.
    #[serde(default)]
    pub mentions: Vec<MentionRef>,
    /// Forwarded from another chat.
    #[serde(default)]
    pub forwarded: bool,
    /// JPEG preview sent with an attachment or link.
    #[serde(default)]
    pub thumbnail: Option<Vec<u8>>,
}

/// Raw WhatsApp mention token and its canonical id.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MentionRef {
    pub user: String,
    pub id: String,
}

/// Link metadata attached by WhatsApp.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LinkPreview {
    pub url: String,
    pub title: Option<String>,
    pub description: Option<String>,
}

impl Message {
    /// Returns a valid individual sender only for an incoming, non-revoked group message.
    pub fn private_reply_recipient(&self) -> Option<&str> {
        (!self.from_me
            && ChatKind::from_id(&self.chat) == ChatKind::Group
            && !matches!(self.content, Content::Revoked)
            && self.sender.split_once('@').is_some_and(|(user, server)| {
                !user.is_empty() && matches!(server, "s.whatsapp.net" | "lid")
            }))
        .then_some(self.sender.as_str())
    }
    /// One-line summary used in chat rows and quotes.
    pub fn summary(&self) -> String {
        self.content.summary()
    }

    /// [`Self::summary`] with its content label in `locale`.
    pub fn summary_in(&self, locale: crate::i18n::Locale) -> String {
        self.content.summary_in(locale)
    }

    /// The line of this message that contains `query`, for a search result's
    /// preview. The archive matches the whole text, so a hit on a later line
    /// would otherwise show a first line the query is nowhere in.
    pub fn text_matching(&self, query: &str) -> Option<String> {
        self.content.text_matching(query)
    }
}

/// A web link in a chat's messages, for the info panel's Links tab.
#[derive(Clone, Debug, PartialEq)]
pub struct ChatLink {
    /// The message it was written in.
    pub message: String,
    pub timestamp: i64,
    pub url: String,
    /// The preview's title, when WhatsApp attached one for this address.
    pub title: Option<String>,
}

/// How many pictures and videos, documents, and links the info panel lists
/// for a chat, each. The worker asks the archive for one more, so a full
/// list can be told apart from a cut one.
pub const MEDIA_LIST_LIMIT: usize = 500;

/// What the info panel lists for one chat: its pictures and videos, its
/// documents, and the web links in its text, each newest first.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ChatMedia {
    pub chat: ChatId,
    pub media: Vec<Message>,
    pub docs: Vec<Message>,
    pub links: Vec<ChatLink>,
    /// Whether the archive held more than each list carries.
    pub media_truncated: bool,
    pub docs_truncated: bool,
    pub links_truncated: bool,
}

impl ChatMedia {
    /// Files `rows` under their tabs, newest first, each list cut at
    /// `limit`. Stickers, audio, deleted and phone-only messages, and text
    /// without a web link are left out.
    pub fn collect(chat: ChatId, mut rows: Vec<Message>, limit: usize) -> Self {
        rows.sort_by_key(|row| std::cmp::Reverse(row.timestamp));
        let mut media = Self {
            chat,
            ..Self::default()
        };
        for row in rows {
            match &row.content {
                Content::Image { .. } | Content::Video { .. } => media.media.push(row),
                Content::Document { .. } => media.docs.push(row),
                Content::Text { .. } => {
                    for (url, title) in row.content.web_links() {
                        media.links.push(ChatLink {
                            message: row.id.clone(),
                            timestamp: row.timestamp,
                            url,
                            title,
                        });
                    }
                }
                _ => {}
            }
        }
        media.media_truncated = media.media.len() > limit;
        media.docs_truncated = media.docs.len() > limit;
        media.links_truncated = media.links.len() > limit;
        media.media.truncate(limit);
        media.docs.truncate(limit);
        media.links.truncate(limit);
        media
    }

    /// Whether any list was cut.
    pub fn truncated(&self) -> bool {
        self.media_truncated || self.docs_truncated || self.links_truncated
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Quoted {
    /// Source chat for a cross-chat quote, absent for ordinary replies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat: Option<ChatId>,
    pub id: String,
    pub sender: String,
    pub sender_name: Option<String>,
    pub summary: String,
    /// Whether `summary` starts with a content label such as "Photo" rather
    /// than quoting text. `None` for quotes archived before it was kept.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub labelled: Option<bool>,
    /// Mentions in quoted text.
    #[serde(default)]
    pub mentions: Vec<MentionRef>,
}

impl Quoted {
    /// `summary` with its content label in `locale`. A quote archived before
    /// `labelled` was kept translates a label it starts with.
    pub fn summary_in(&self, locale: crate::i18n::Locale) -> std::borrow::Cow<'_, str> {
        if self.labelled == Some(false) {
            std::borrow::Cow::Borrowed(&self.summary)
        } else {
            localized_summary(locale, &self.summary)
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Reaction {
    pub sender: String,
    pub from_me: bool,
    pub emoji: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Content {
    Text {
        text: String,
        #[serde(default)]
        preview: Option<LinkPreview>,
    },
    /// Readable text plus presentation metadata. Raw protocol data stays in the worker.
    Interactive {
        text: String,
        #[serde(default)]
        card: Option<Box<InteractiveCard>>,
    },
    Image {
        caption: Option<String>,
        media: Media,
        /// The short clip of a motion photo.
        #[serde(default)]
        motion: Option<Motion>,
    },
    Video {
        caption: Option<String>,
        media: Media,
        seconds: Option<u32>,
        gif: bool,
        /// A round video message, which WhatsApp calls PTV.
        #[serde(default)]
        note: bool,
    },
    Audio {
        media: Media,
        seconds: Option<u32>,
        voice_note: bool,
        /// Sender-provided 64-bar voice waveform.
        #[serde(default)]
        waveform: Vec<u8>,
    },
    Document {
        media: Media,
        file_name: String,
        caption: Option<String>,
        pages: Option<u32>,
    },
    Sticker {
        media: Media,
        animated: bool,
    },
    /// A WhatsApp sticker pack shared in a chat. Its stickers download when
    /// someone opens it.
    #[serde(rename = "sticker_pack")]
    StickerPack {
        name: String,
        publisher: String,
        count: u32,
        caption: Option<String>,
    },
    Location {
        latitude: f64,
        longitude: f64,
        name: Option<String>,
        address: Option<String>,
    },
    /// A live location that updates in place as the sender moves. The map
    /// preview rides in `Message.thumbnail`; the shared `(chat, id)` upsert
    /// keeps one bubble per session.
    LiveLocation {
        latitude: f64,
        longitude: f64,
        /// Position accuracy reported by the sender, in metres.
        #[serde(default)]
        accuracy_m: Option<u32>,
        /// Speed in metres per second.
        #[serde(default)]
        speed_mps: Option<f32>,
        /// Heading, degrees clockwise from magnetic north.
        #[serde(default)]
        heading_deg: Option<u32>,
        /// Monotonic ordering guard against out-of-order updates.
        #[serde(default)]
        sequence: i64,
        /// Whether the sender has stopped sharing.
        #[serde(default)]
        ended: bool,
        /// Unix seconds of the latest position, or 0 before any update. The
        /// message keeps its start time so updates do not reorder the chat.
        #[serde(default)]
        updated: i64,
        /// Whether the sender's phone has posted positions this device cannot
        /// read. WhatsApp keeps live locations off linked devices, so the
        /// card holds the last position it was given and says where the
        /// newer ones are.
        #[serde(default)]
        newer_on_phone: bool,
    },
    Contact {
        display_name: String,
        vcard: String,
    },
    Poll {
        question: String,
        options: Vec<String>,
        #[serde(default)]
        state: PollState,
    },
    /// "This message was deleted."
    Revoked,
    /// Unsupported content with a user-facing description.
    Unsupported {
        what: String,
    },
    /// A message WhatsApp only delivers to the phone, such as view-once
    /// media. Linked devices receive a placeholder that never fills in.
    PhoneOnly {
        view_once: bool,
        /// A live location, which WhatsApp shows only on the phone.
        #[serde(default)]
        live_location: bool,
        /// What a view-once message holds, when it arrived as media this
        /// device may not open rather than as a bare placeholder.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        once: Option<OnceMedia>,
    },
    /// A call in this chat, where it falls among the messages, as WhatsApp
    /// shows it. `from_me` on the message says who placed it. Entries ZapFast
    /// writes itself carry an id that starts with [`CALL_ENTRY_PREFIX`] and are
    /// never sent anywhere.
    Call {
        video: bool,
        status: CallStatus,
        /// Seconds the call was up; zero when it never was.
        #[serde(default)]
        duration: u64,
        /// A group call.
        #[serde(default)]
        group: bool,
        /// How many people were on the call, this account counted once. `None`
        /// for a one-to-one call, and for a group call whose record does not
        /// name who took part (one filed before this was kept, or a missed-call
        /// notice that carries no roster). Only the number is kept, never who:
        /// no name, number, or id of anyone on the call is stored.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        participants: Option<u32>,
    },
}

/// The id prefix of the call entries ZapFast files from call records: the
/// calls it handled and the phone's call log. They are local, so no receipt,
/// reaction or deletion is ever sent for them.
pub const CALL_ENTRY_PREFIX: &str = "zapfast-call-";

/// The chat id the entries of calls joined through a call link are filed
/// under. No chat has it: such a call belongs to none, so its entries show on
/// the Calls page alone, never in the chat list.
pub const CALL_LINK_CHAT: &str = "zapfast-call-link";

/// How far apart, in seconds, two entries for one call may be: the phone's
/// call-log message and the call's record are stamped by different clocks.
pub const CALL_ENTRY_WINDOW: i64 = 120;

/// What became of a call.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CallStatus {
    /// The two sides talked, so the call's length is real.
    Answered,
    /// Another of this account's devices took it.
    AnsweredElsewhere,
    /// An incoming call nobody picked up.
    Missed,
    /// Refused, by either side.
    Declined,
    /// Another of this account's devices refused it.
    DeclinedElsewhere,
    /// The other phone was on another call.
    Busy,
    /// It never came up, for no reason above.
    Failed,
    /// Outgoing, and nobody answered.
    NoAnswer,
    /// The line went while the call was up.
    ConnectionLost,
}

impl CallStatus {
    /// Whether the call is one to notice: missed, refused, or never up.
    pub fn is_bad(self) -> bool {
        !matches!(self, Self::Answered | Self::AnsweredElsewhere)
    }
}

/// The kind of media a view-once message holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OnceMedia {
    Photo,
    Video,
    Voice,
    Audio,
}

impl OnceMedia {
    /// The kind of view-once media `content` would be, if it is media that
    /// can be sent to be viewed once.
    pub fn of(content: &Content) -> Option<Self> {
        match content {
            Content::Image { .. } => Some(Self::Photo),
            Content::Video { .. } => Some(Self::Video),
            Content::Audio {
                voice_note: true, ..
            } => Some(Self::Voice),
            Content::Audio { .. } => Some(Self::Audio),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct InteractiveCard {
    /// Message text without the action labels, which have their own rows.
    pub body: String,
    pub buttons: Vec<InteractiveButton>,
    pub image: Option<Media>,
    /// Unrenderable attachments, forms, or missing message text.
    pub needs_phone: bool,
    /// Independent carousel cards, in wire order. No protocol ids or keys.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub carousel: Vec<InteractiveCard>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thumbnail: Option<Vec<u8>>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InteractiveButton {
    pub label: String,
    /// Validated HTTP(S) target, also retained for older archived cards.
    pub url: Option<String>,
    /// Non-link action. Protocol option ids stay in the worker.
    #[serde(default)]
    pub action: InteractiveAction,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum InteractiveAction {
    #[default]
    Unavailable,
    Reply,
    Copy(String),
    Select(Vec<InteractiveOption>),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InteractiveOption {
    pub section: String,
    pub title: String,
    pub description: String,
}

/// Poll information safe to send to the interface; encryption keys stay in the worker.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PollState {
    pub selectable: usize,
    pub counts: Vec<usize>,
    pub selected: Vec<usize>,
    pub voters: usize,
    pub can_vote: bool,
    pub history_complete: bool,
    pub refresh_needed: bool,
    pub refreshing: bool,
    pub refresh_failed: bool,
    /// Latest decrypted votes only. Derived on load, never stored in content JSON.
    #[serde(skip)]
    pub votes: Vec<PollVoter>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PollVoter {
    pub id: String,
    pub name: String,
    pub from_me: bool,
    pub timestamp: i64,
    pub choices: Vec<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PollDraft {
    pub question: String,
    pub options: Vec<String>,
    pub multiple: bool,
}

impl Default for PollDraft {
    fn default() -> Self {
        Self {
            question: String::new(),
            options: vec![String::new(); 2],
            multiple: true,
        }
    }
}

impl PollDraft {
    pub fn validated(&self) -> Result<Self, &'static str> {
        let question = self.question.trim().to_owned();
        let options: Vec<String> = self
            .options
            .iter()
            .map(|option| option.trim().to_owned())
            .collect();
        if question.is_empty() || question.chars().count() > 255 {
            return Err("Enter a question of up to 255 characters.");
        }
        if !(2..=12).contains(&options.len())
            || options
                .iter()
                .any(|option| option.is_empty() || option.chars().count() > 100)
        {
            return Err("Add 2–12 answers, each with 1–100 characters.");
        }
        let mut unique = std::collections::HashSet::new();
        if options.iter().any(|option| !unique.insert(option)) {
            return Err("Each answer must be different.");
        }
        Ok(Self {
            question,
            options,
            multiple: self.multiple,
        })
    }

    pub fn selectable(&self) -> usize {
        if self.multiple { self.options.len() } else { 1 }
    }
}

/// The longest group name WhatsApp accepts, in characters.
pub const GROUP_NAME_LIMIT: usize = whatsapp_rust::wacore::iq::groups::GROUP_SUBJECT_MAX_LENGTH;

/// WhatsApp's longest live location share, in seconds.
pub const LIVE_LOCATION_LIMIT: i64 = 8 * 60 * 60;

/// One photo or playable clip of a chat, as the archive lists them for the
/// full-window viewer's album (`Command::LoadChatMedia`): what it is, when it
/// arrived, and what a strip or grid needs to draw it before the file itself
/// is loaded. Any view that lists a chat's photos and clips reads these.
#[derive(Clone, Debug)]
pub struct MediaItem {
    pub id: String,
    /// For an interactive message's picture, which one: `None` for the
    /// message's own attachment or a card's header image, and the carousel
    /// card's index otherwise, as `Action::Download` counts them.
    pub card: Option<usize>,
    pub timestamp: i64,
    pub video: bool,
    /// `None` until the attachment is downloaded.
    pub path: Option<std::path::PathBuf>,
    /// The stored thumbnail, drawn instead of decoding the file.
    pub thumbnail: Option<Vec<u8>>,
}

impl MediaItem {
    /// The album items one message holds, in the order it shows them: a photo
    /// or a playable clip, or an interactive message's card images.
    ///
    /// `exists` is asked once per recorded path: an attachment that was deleted
    /// or moved is reported as missing, so the item offers Download again
    /// instead of drawing a dead frame. A downloaded file the viewer cannot
    /// decode (a bmp, a tif) is not an item: the bubble sends it to the system
    /// viewer. A card image is an item only once it is downloaded, as the card
    /// downloads its own pictures.
    pub fn of_content(
        id: &str,
        timestamp: i64,
        content: &Content,
        thumbnail: Option<&[u8]>,
        exists: impl Fn(&Path) -> bool,
    ) -> Vec<MediaItem> {
        let item = |card, video, path, thumbnail: Option<&[u8]>| MediaItem {
            id: id.to_owned(),
            card,
            timestamp,
            video,
            path,
            thumbnail: thumbnail.map(<[u8]>::to_vec),
        };
        if let Some(kind) = content.gallery_kind() {
            let path = content
                .media()
                .and_then(|media| media.path.clone())
                .filter(|path| exists(path));
            if path
                .as_ref()
                .is_some_and(|path| gallery_kind_for_path(path).is_none())
            {
                return Vec::new();
            }
            return vec![item(None, kind == GalleryKind::Video, path, thumbnail)];
        }
        let Content::Interactive {
            card: Some(card), ..
        } = content
        else {
            return Vec::new();
        };
        std::iter::once((None, &**card))
            .chain(
                card.carousel
                    .iter()
                    .enumerate()
                    .map(|(index, entry)| (Some(index), entry)),
            )
            .filter_map(|(index, entry)| {
                let path = entry.image.as_ref()?.path.clone().filter(|path| {
                    gallery_kind_for_path(path) == Some(GalleryKind::Photo) && exists(path)
                })?;
                // Each card keeps its own preview; the header image may only
                // have the message's.
                let preview = entry
                    .thumbnail
                    .as_deref()
                    .or(thumbnail.filter(|_| index.is_none()));
                Some(item(index, false, Some(path), preview))
            })
            .collect()
    }

    /// The album items of a message already in memory, checking its files.
    pub fn of_message(message: &Message) -> Vec<MediaItem> {
        Self::of_content(
            &message.id,
            message.timestamp,
            &message.content,
            message.thumbnail.as_deref(),
            Path::is_file,
        )
    }
}

/// A photo or a playable video, as the in-app viewer album counts them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GalleryKind {
    Photo,
    Video,
}

impl Content {
    /// Whether a live location sent at `sent` has stopped by `now`: its
    /// sender ended it, or it has outlived the longest share.
    pub fn live_location_over(&self, sent: i64, now: i64) -> bool {
        match self {
            Self::LiveLocation { ended, .. } => *ended || now - sent > LIVE_LOCATION_LIMIT,
            _ => false,
        }
    }

    pub fn text(text: impl Into<String>) -> Self {
        Self::Text {
            text: text.into(),
            preview: None,
        }
    }

    /// The first line of the text the archive search looks at that contains
    /// `query`, trimmed, or `None` when no line has it.
    pub fn text_matching(&self, query: &str) -> Option<String> {
        let needle = query.trim().to_lowercase();
        if needle.is_empty() {
            return None;
        }
        self.searchable_fields()
            .into_iter()
            .flat_map(str::lines)
            .find(|line| line.to_lowercase().contains(&needle))
            .map(|line| line.trim().to_owned())
    }

    /// The text fields the archive search matches (its `SEARCHED_TEXT`), so
    /// a preview is built from the same set.
    fn searchable_fields(&self) -> Vec<&str> {
        match self {
            Self::Text { text, .. } | Self::Interactive { text, .. } => vec![text],
            Self::Image { caption, .. } | Self::Video { caption, .. } => {
                caption.as_deref().into_iter().collect()
            }
            Self::Document {
                file_name, caption, ..
            } => std::iter::once(file_name.as_str())
                .chain(caption.as_deref())
                .collect(),
            Self::StickerPack { name, caption, .. } => std::iter::once(name.as_str())
                .chain(caption.as_deref())
                .collect(),
            Self::Poll { question, .. } => vec![question],
            Self::Contact { display_name, .. } => vec![display_name],
            Self::Location { name, .. } => name.as_deref().into_iter().collect(),
            _ => Vec::new(),
        }
    }

    /// The whole message as [`Self::summary`] would label it: every line of
    /// a text or a photo or video caption. Other content has nothing more
    /// to say than its summary.
    pub fn full_summary(&self) -> String {
        let captioned = |label: &str, caption: &Option<String>| match caption.as_deref() {
            Some(caption) if !caption.trim().is_empty() => format!("{label}: {caption}"),
            _ => label.to_owned(),
        };
        match self {
            Self::Text { text, .. } | Self::Interactive { text, .. } => text.clone(),
            Self::Image { caption, .. } => captioned("Photo", caption),
            Self::Video {
                caption, gif, note, ..
            } => captioned(video_label(*gif, *note), caption),
            _ => self.summary(),
        }
    }

    /// The web addresses in a text message: the preview's, with its title,
    /// then the others written in the text, each once.
    pub fn web_links(&self) -> Vec<(String, Option<String>)> {
        let Self::Text { text, preview } = self else {
            return Vec::new();
        };
        // The preview's address was normalised when it was archived; the
        // ones found in the text are normalised the same way, so an address
        // typed without its scheme is not listed beside its preview.
        let normalise =
            |url: &str| crate::safety::preview_url(url).unwrap_or_else(|| url.to_owned());
        let mut links: Vec<(String, Option<String>)> = Vec::new();
        if let Some(preview) = preview {
            let title = preview
                .title
                .clone()
                .filter(|title| !title.trim().is_empty());
            links.push((normalise(&preview.url), title));
        }
        for url in crate::markup::links(text) {
            let url = normalise(&url);
            if !links.iter().any(|(known, _)| *known == url) {
                links.push((url, None));
            }
        }
        links
    }

    /// Whether the info panel lists this message: a picture, a video, a
    /// document, or text with a web link.
    pub fn listable(&self) -> bool {
        match self {
            Self::Image { .. } | Self::Video { .. } | Self::Document { .. } => true,
            Self::Text { .. } => !self.web_links().is_empty(),
            _ => false,
        }
    }

    /// Whether [`Self::summary`] starts with a content label such as "Photo"
    /// rather than the message's own text.
    pub fn labelled(&self) -> bool {
        !matches!(self, Self::Text { .. } | Self::Interactive { .. })
    }

    /// [`Self::summary`] with its content label in `locale`.
    pub fn summary_in(&self, locale: crate::i18n::Locale) -> String {
        let summary = self.summary();
        if self.labelled() {
            localized_summary(locale, &summary).into_owned()
        } else {
            summary
        }
    }

    /// [`Self::full_summary`] with its content label in `locale`.
    pub fn full_summary_in(&self, locale: crate::i18n::Locale) -> String {
        let summary = self.full_summary();
        if self.labelled() {
            localized_summary(locale, &summary).into_owned()
        } else {
            summary
        }
    }

    pub fn summary(&self) -> String {
        match self {
            Self::Text { text, .. } | Self::Interactive { text, .. } => {
                text.lines().next().unwrap_or_default().to_owned()
            }
            Self::Image { caption, .. } => with_caption("Photo", caption),
            Self::Video {
                caption, gif, note, ..
            } => with_caption(video_label(*gif, *note), caption),
            Self::Audio {
                voice_note,
                seconds,
                ..
            } => {
                let label = if *voice_note {
                    "Voice message"
                } else {
                    "Audio"
                };
                match seconds {
                    Some(seconds) => format!("{label} ({})", crate::util::duration(*seconds)),
                    None => label.to_owned(),
                }
            }
            Self::Document { file_name, .. } => format!("Document: {file_name}"),
            Self::Sticker { .. } => "Sticker".to_owned(),
            Self::StickerPack { name, .. } => format!("Sticker pack: {name}"),
            Self::Location { name, .. } => match name {
                Some(name) => format!("Location: {name}"),
                None => "Location".to_owned(),
            },
            Self::LiveLocation { ended, .. } => {
                if *ended {
                    "Live location ended".to_owned()
                } else {
                    "Live location".to_owned()
                }
            }
            Self::Contact { display_name, .. } => format!("Contact: {display_name}"),
            Self::Poll { question, .. } => format!("Poll: {question}"),
            Self::Revoked => "This message was deleted".to_owned(),
            Self::Unsupported { what } => format!("Unsupported message ({what})"),
            Self::PhoneOnly {
                live_location: true,
                ..
            } => "Live location".to_owned(),
            Self::PhoneOnly {
                once: Some(kind), ..
            } => match kind {
                OnceMedia::Photo => "View once photo",
                OnceMedia::Video => "View once video",
                OnceMedia::Voice => "View once voice message",
                OnceMedia::Audio => "View once audio",
            }
            .to_owned(),
            Self::PhoneOnly {
                view_once: true, ..
            } => "View once message".to_owned(),
            Self::PhoneOnly { .. } => "Message on your phone".to_owned(),
            Self::Call {
                video,
                status,
                duration,
                ..
            } => {
                let label = match (status, video) {
                    (CallStatus::Missed, false) => "Missed voice call",
                    (CallStatus::Missed, true) => "Missed video call",
                    (_, false) => "Voice call",
                    (_, true) => "Video call",
                };
                if *status == CallStatus::Answered && *duration > 0 {
                    format!(
                        "{label} ({})",
                        // Summaries are English, as every other one is.
                        crate::util::call_length(crate::i18n::Locale::English, *duration)
                    )
                } else {
                    label.to_owned()
                }
            }
        }
    }

    /// Whether this stands in for a message this device could not open.
    pub fn is_placeholder(&self) -> bool {
        matches!(self, Self::Unsupported { .. } | Self::PhoneOnly { .. })
    }

    pub fn media(&self) -> Option<&Media> {
        match self {
            Self::Image { media, .. }
            | Self::Video { media, .. }
            | Self::Audio { media, .. }
            | Self::Document { media, .. }
            | Self::Sticker { media, .. } => Some(media),
            Self::Interactive {
                card: Some(card), ..
            } => card.image.as_ref(),
            _ => None,
        }
    }

    pub fn media_at_mut(&mut self, card_index: Option<usize>) -> Option<&mut Media> {
        match card_index {
            None => self.media_mut(),
            Some(index) => match self {
                Self::Interactive {
                    card: Some(card), ..
                } => card.carousel.get_mut(index)?.image.as_mut(),
                _ => None,
            },
        }
    }

    /// Whether this message belongs in the viewer album, and as what.
    ///
    /// A GIF is not a video here: it plays inline as an animation, so it is
    /// left out.
    pub fn gallery_kind(&self) -> Option<GalleryKind> {
        match self {
            Self::Image { .. } => Some(GalleryKind::Photo),
            Self::Video { gif: false, .. } => Some(GalleryKind::Video),
            _ => None,
        }
    }

    /// Carries downloaded file paths over from `old` when rederiving content
    /// from the raw protobuf: the main attachment and each carousel card's image.
    pub fn keep_local_paths(&mut self, old: &Content) {
        if let (Some(new), Some(old)) = (self.media_mut(), old.media()) {
            new.path = old.path.clone();
        }
        if let (
            Self::Image { motion: new, .. },
            Self::Image {
                motion: Some(old), ..
            },
        ) = (&mut *self, old)
        {
            *new = Some(old.clone());
        }
        if let (
            Self::Interactive {
                card: Some(new), ..
            },
            Self::Interactive {
                card: Some(old), ..
            },
        ) = (self, old)
        {
            for (new, old) in new.carousel.iter_mut().zip(&old.carousel) {
                if let (Some(new), Some(old)) = (&mut new.image, &old.image) {
                    new.path = old.path.clone();
                }
            }
        }
    }

    /// Forgets every downloaded file of this message kept under `dir`, so
    /// it downloads again: the attachment, a motion photo's clip, and each
    /// carousel card's picture. Files elsewhere, such as a chosen download
    /// folder, are kept.
    pub fn forget_files_in(&mut self, dir: &std::path::Path) {
        let forget = |path: &mut Option<PathBuf>, state: &mut MediaState| {
            if path.as_deref().is_some_and(|path| path.starts_with(dir)) {
                *path = None;
                *state = MediaState::Idle;
            }
        };
        if let Some(media) = self.media_mut() {
            forget(&mut media.path, &mut media.state);
        }
        match self {
            Self::Image {
                motion: Some(motion),
                ..
            } => forget(&mut motion.path, &mut motion.state),
            Self::Interactive {
                card: Some(card), ..
            } => {
                for item in &mut card.carousel {
                    if let Some(media) = item.image.as_mut() {
                        forget(&mut media.path, &mut media.state);
                    }
                }
            }
            _ => {}
        }
    }

    pub fn media_mut(&mut self) -> Option<&mut Media> {
        match self {
            Self::Image { media, .. }
            | Self::Video { media, .. }
            | Self::Audio { media, .. }
            | Self::Document { media, .. }
            | Self::Sticker { media, .. } => Some(media),
            Self::Interactive {
                card: Some(card), ..
            } => card.image.as_mut(),
            _ => None,
        }
    }
}

/// The viewer kind of a file on disk, from its name alone.
pub(crate) fn gallery_kind_for_path(path: &Path) -> Option<GalleryKind> {
    gallery_file(&path.file_name()?.to_string_lossy())
}

/// The viewer kind of a file, from its name. A GIF is an inline animation, not
/// a clip, so it is left out, and a photo is only a format the image crate is
/// built to draw (`jpeg`, `png`, `webp`): a `bmp` or a `tif` opens the viewer
/// and fails to decode, where the system viewer draws it.
fn gallery_file(file_name: &str) -> Option<GalleryKind> {
    let ext = Path::new(file_name)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())?;
    match ext.as_str() {
        "jpg" | "jpeg" | "png" | "webp" => Some(GalleryKind::Photo),
        "mp4" | "m4v" | "mov" | "webm" | "mkv" | "3gp" | "3gpp" => Some(GalleryKind::Video),
        _ => None,
    }
}

/// What a video is called in previews.
fn video_label(gif: bool, note: bool) -> &'static str {
    if gif {
        "GIF"
    } else if note {
        "Video message"
    } else {
        "Video"
    }
}

/// The labels a summary of content other than text starts with. Summaries
/// are made in English where the archive and the worker build them, and
/// [`localized_summary`] translates the label when one is shown.
const SUMMARY_LABELS: [&str; 26] = [
    "Photo",
    "Video",
    "GIF",
    "Video message",
    "Voice message",
    "Audio",
    "Document",
    "Sticker",
    "Sticker pack",
    "Location",
    "Live location",
    "Live location ended",
    "Contact",
    "Poll",
    "This message was deleted",
    "Unsupported message",
    "View once photo",
    "View once video",
    "View once voice message",
    "View once audio",
    "View once message",
    "Message on your phone",
    "Missed voice call",
    "Missed video call",
    "Voice call",
    "Video call",
];

/// A summary label in `locale`. Literal calls, so xgettext extracts each.
fn summary_label(
    locale: crate::i18n::Locale,
    label: &'static str,
) -> std::borrow::Cow<'static, str> {
    use crate::i18n::gettext;
    match label {
        "Photo" => gettext(locale, "Photo"),
        "Video" => gettext(locale, "Video"),
        "GIF" => gettext(locale, "GIF"),
        "Video message" => gettext(locale, "Video message"),
        "Voice message" => gettext(locale, "Voice message"),
        "Audio" => gettext(locale, "Audio"),
        "Document" => gettext(locale, "Document"),
        "Sticker" => gettext(locale, "Sticker"),
        "Sticker pack" => gettext(locale, "Sticker pack"),
        "Location" => gettext(locale, "Location"),
        "Live location" => gettext(locale, "Live location"),
        "Live location ended" => gettext(locale, "Live location ended"),
        "Contact" => gettext(locale, "Contact"),
        "Poll" => gettext(locale, "Poll"),
        "This message was deleted" => gettext(locale, "This message was deleted"),
        "Unsupported message" => gettext(locale, "Unsupported message"),
        "View once photo" => gettext(locale, "View once photo"),
        "View once video" => gettext(locale, "View once video"),
        "View once voice message" => gettext(locale, "View once voice message"),
        "View once audio" => gettext(locale, "View once audio"),
        "View once message" => gettext(locale, "View once message"),
        "Message on your phone" => gettext(locale, "Message on your phone"),
        "Missed voice call" => gettext(locale, "Missed voice call"),
        "Missed video call" => gettext(locale, "Missed video call"),
        "Voice call" => gettext(locale, "Voice call"),
        "Video call" => gettext(locale, "Video call"),
        _ => std::borrow::Cow::Borrowed(label),
    }
}

/// `summary` with the content label it starts with, such as "Photo" or
/// "Voice message", in `locale`. What follows the label (a caption, a file
/// name, a length) stays as it is. A summary of text must not come here: a
/// message that reads "Location: home" is the sender's words.
pub fn localized_summary(locale: crate::i18n::Locale, summary: &str) -> std::borrow::Cow<'_, str> {
    // The longest label that fits, so "Video message" is not read as "Video".
    let label = SUMMARY_LABELS
        .into_iter()
        .filter(|label| {
            summary.strip_prefix(label).is_some_and(|rest| {
                rest.is_empty() || rest.starts_with(": ") || rest.starts_with(" (")
            })
        })
        .max_by_key(|label| label.len());
    let Some(label) = label else {
        return std::borrow::Cow::Borrowed(summary);
    };
    let translated = summary_label(locale, label);
    if translated == label {
        return std::borrow::Cow::Borrowed(summary);
    }
    std::borrow::Cow::Owned(format!("{translated}{}", &summary[label.len()..]))
}

/// `label: caption` with the caption whole, for a row that wraps it.
fn with_caption(label: &str, caption: &Option<String>) -> String {
    match caption
        .as_deref()
        .and_then(|caption| caption.lines().next())
    {
        Some(caption) if !caption.is_empty() => format!("{label}: {caption}"),
        _ => label.to_owned(),
    }
}

/// Maximum size of an attachment downloaded without the reader asking.
pub(crate) const ATTACHMENT_DOWNLOAD_LIMIT: u64 = 64 * 1024 * 1024;

/// Attachment metadata, download state, and optional local file. Download keys
/// remain in the archive's raw message.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Media {
    pub mime: String,
    pub size: u64,
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// Parent message id when WhatsApp sent this picture or video as an album.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub album: Option<String>,
    /// Decrypted downloaded file.
    #[serde(default)]
    pub path: Option<PathBuf>,
    /// Non-persisted download state.
    #[serde(skip)]
    pub state: MediaState,
}

impl Media {
    /// Whether the attachment's declared size may download automatically.
    ///
    /// A missing size is represented as zero and is allowed here. The worker
    /// still enforces the limit while streaming an automatic download.
    pub fn is_within_download_limit(&self) -> bool {
        self.size <= ATTACHMENT_DOWNLOAD_LIMIT
    }
}

/// A motion photo's clip. Its download keys stay in the archive.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Motion {
    /// Decrypted downloaded file.
    #[serde(default)]
    pub path: Option<PathBuf>,
    /// Non-persisted download state.
    #[serde(skip)]
    pub state: MediaState,
}

/// Counts and sizes of the files in an account's media cache, for the
/// storage rows in Settings. The kind comes from each file's extension.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StorageStats {
    pub images: u64,
    pub image_bytes: u64,
    pub videos: u64,
    pub video_bytes: u64,
    pub stickers: u64,
    pub sticker_bytes: u64,
    pub other: u64,
    pub other_bytes: u64,
}

impl StorageStats {
    /// Counts one cached file of `bytes` under the kind its name says.
    /// Animated GIFs arrive as MP4 videos, so they count with the videos.
    pub fn add(&mut self, path: &std::path::Path, bytes: u64) {
        let extension = path
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase)
            .unwrap_or_default();
        let (count, total) = match extension.as_str() {
            "jpg" | "jpeg" | "png" | "heic" | "heif" | "bmp" | "tif" | "tiff" => {
                (&mut self.images, &mut self.image_bytes)
            }
            "mp4" | "m4v" | "mov" | "3gp" | "3gpp" | "webm" | "mkv" | "gif" => {
                (&mut self.videos, &mut self.video_bytes)
            }
            "webp" => (&mut self.stickers, &mut self.sticker_bytes),
            _ => (&mut self.other, &mut self.other_bytes),
        };
        *count += 1;
        *total = total.saturating_add(bytes);
    }

    /// Every cached byte counted.
    pub fn bytes_total(self) -> u64 {
        self.image_bytes
            .saturating_add(self.video_bytes)
            .saturating_add(self.sticker_bytes)
            .saturating_add(self.other_bytes)
    }

    /// Every cached file counted.
    pub fn files_total(self) -> u64 {
        self.images + self.videos + self.stickers + self.other
    }

    /// Filled width for `bytes` out of `total` on a track of `track` pixels.
    /// Empty when there is nothing to measure, so an empty cache draws no
    /// bar instead of dividing by zero.
    pub fn bar_width(bytes: u64, total: u64, track: f32) -> f32 {
        if total == 0 || track <= 0.0 {
            0.0
        } else {
            track * (bytes as f32 / total as f32)
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub enum MediaState {
    #[default]
    Idle,
    Downloading,
    Failed(String),
}

/// Decoded straight-alpha RGBA image bytes ready for the clipboard.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodedImage {
    pub width: usize,
    pub height: usize,
    pub bytes: Vec<u8>,
}

/// Contact names from app-state sync and message push names.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Contact {
    pub id: String,
    pub full_name: Option<String>,
    /// The first name saved with `full_name`, which WhatsApp shows where
    /// space is short, as in a group's member line. It may hold several
    /// words; only a contact saved with a separate first name has one.
    pub first_name: Option<String>,
    pub push_name: Option<String>,
}

impl Contact {
    pub fn display_name(&self) -> Option<&str> {
        self.full_name
            .as_deref()
            .filter(|name| !name.is_empty())
            .or(self.push_name.as_deref().filter(|name| !name.is_empty()))
    }

    /// The saved first name, when the address-book entry has one.
    pub fn first_name(&self) -> Option<&str> {
        self.full_name.as_deref().filter(|name| !name.is_empty())?;
        self.first_name
            .as_deref()
            .map(str::trim)
            .filter(|name| !name.is_empty())
    }

    /// WhatsApp display name: address-book name or `~`-prefixed push name.
    pub fn label(&self) -> Option<String> {
        if let Some(name) = self.full_name.as_deref().filter(|name| !name.is_empty()) {
            return Some(name.to_owned());
        }
        self.push_name
            .as_deref()
            .filter(|name| !name.is_empty())
            .map(|name| format!("~{name}"))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Page {
    Chats,
    /// Every call of the account, newest first.
    Calls,
    Settings,
    Wallpaper,
}

/// Which side of a call this account was on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallDirection {
    Incoming,
    Outgoing,
}

/// The tabs of the picker above the composer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickerTab {
    Emoji,
    Gifs,
    Stickers,
}

/// How the chat list is drawn. Hiding it can also just collapse it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SidebarDisplayMode {
    /// The full list: names, previews, timestamps.
    #[default]
    Expanded,
    /// Avatars and unread badges only, in a narrow column.
    CollapsedIconsOnly,
}

/// Which list the sticker tab shows.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum StickerShelf {
    /// Stickers we sent, and the phone's recent list.
    #[default]
    Recent,
    Favorites,
    /// Stickers others sent us, newest first.
    Received,
    /// One pack, by its folder.
    Pack(PathBuf),
    /// Importing packs, starting one, or making a sticker.
    Add,
}

/// A square region of a picture, in its pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StickerCrop {
    pub x: u32,
    pub y: u32,
    pub side: u32,
}

impl StickerCrop {
    /// The largest square in the middle of the picture.
    pub fn centered(width: u32, height: u32) -> Self {
        let side = width.min(height).max(1);
        Self {
            x: width.saturating_sub(side) / 2,
            y: height.saturating_sub(side) / 2,
            side,
        }
    }

    /// The same square kept inside a picture of this size.
    pub fn clamped(self, width: u32, height: u32) -> Self {
        let side = self.side.clamp(1, width.min(height).max(1));
        Self {
            x: self.x.min(width.saturating_sub(side)),
            y: self.y.min(height.saturating_sub(side)),
            side,
        }
    }

    /// The square moved by whole pixels, staying inside the picture.
    pub fn moved(self, dx: i64, dy: i64, width: u32, height: u32) -> Self {
        let shift = |at: u32, by: i64| (i64::from(at) + by).max(0) as u32;
        Self {
            x: shift(self.x, dx),
            y: shift(self.y, dy),
            ..self
        }
        .clamped(width, height)
    }

    /// The square resized around its center, staying inside the picture.
    pub fn resized(self, side: u32, width: u32, height: u32) -> Self {
        let center = |at: u32| i64::from(at) + i64::from(self.side) / 2;
        let half = i64::from(side) / 2;
        Self {
            x: (center(self.x) - half).max(0) as u32,
            y: (center(self.y) - half).max(0) as u32,
            side,
        }
        .clamped(width, height)
    }
}

/// Which part of a crop is being dragged. The edges move one side, a corner
/// moves two, and `Inside` moves the whole region.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CropEdge {
    Inside,
    Left,
    Right,
    Top,
    Bottom,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl CropEdge {
    /// Whether this part drags the region's left side.
    fn moves_left(self) -> bool {
        matches!(self, Self::Left | Self::TopLeft | Self::BottomLeft)
    }

    /// Whether this part drags the region's right side.
    fn moves_right(self) -> bool {
        matches!(self, Self::Right | Self::TopRight | Self::BottomRight)
    }

    /// Whether this part drags the region's top side.
    fn moves_top(self) -> bool {
        matches!(self, Self::Top | Self::TopLeft | Self::TopRight)
    }

    /// Whether this part drags the region's bottom side.
    fn moves_bottom(self) -> bool {
        matches!(self, Self::Bottom | Self::BottomLeft | Self::BottomRight)
    }
}

/// The smallest crop side, so a drag cannot close the region to nothing.
pub const CROP_MIN_SIDE: u32 = 16;

/// A region of a picture to keep, in its pixels, as the picture is shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PictureCrop {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl PictureCrop {
    /// The whole picture.
    pub fn full(width: u32, height: u32) -> Self {
        Self {
            x: 0,
            y: 0,
            width: width.max(1),
            height: height.max(1),
        }
    }

    /// The same region kept inside a picture of this size.
    pub fn clamped(self, width: u32, height: u32) -> Self {
        let (width, height) = (width.max(1), height.max(1));
        let crop_width = self.width.clamp(1, width);
        let crop_height = self.height.clamp(1, height);
        Self {
            x: self.x.min(width - crop_width),
            y: self.y.min(height - crop_height),
            width: crop_width,
            height: crop_height,
        }
    }

    /// The region after one edge or corner is dragged by whole pixels.
    pub fn drag(self, edge: CropEdge, dx: i64, dy: i64, width: u32, height: u32) -> Self {
        let (width, height) = (i64::from(width.max(1)), i64::from(height.max(1)));
        let (mut left, mut top) = (i64::from(self.x), i64::from(self.y));
        let (mut right, mut bottom) = (left + i64::from(self.width), top + i64::from(self.height));
        if edge == CropEdge::Inside {
            return self.moved(dx, dy, width as u32, height as u32);
        }
        if edge.moves_left() {
            left += dx;
        }
        if edge.moves_right() {
            right += dx;
        }
        if edge.moves_top() {
            top += dy;
        }
        if edge.moves_bottom() {
            bottom += dy;
        }
        left = left.clamp(0, width);
        right = right.clamp(0, width);
        top = top.clamp(0, height);
        bottom = bottom.clamp(0, height);
        // A moving edge gives way to the fixed one, so the region keeps a
        // usable size without the picture's own edge being pushed out.
        let least = i64::from(CROP_MIN_SIDE).min(width).min(height);
        if edge.moves_left() {
            left = left.min(right - least).max(0);
        }
        if edge.moves_right() {
            right = right.max(left + least).min(width);
        }
        if edge.moves_top() {
            top = top.min(bottom - least).max(0);
        }
        if edge.moves_bottom() {
            bottom = bottom.max(top + least).min(height);
        }
        Self {
            x: left as u32,
            y: top as u32,
            width: (right - left).max(1) as u32,
            height: (bottom - top).max(1) as u32,
        }
    }

    /// The region moved by whole pixels, staying inside the picture.
    pub fn moved(self, dx: i64, dy: i64, width: u32, height: u32) -> Self {
        let shift = |at: u32, by: i64| (i64::from(at) + by).max(0) as u32;
        Self {
            x: shift(self.x, dx),
            y: shift(self.y, dy),
            ..self
        }
        .clamped(width, height)
    }

    /// The same content after the picture is turned a quarter clockwise. The
    /// picture's former height becomes its width.
    pub fn turned_clockwise(self, old_height: u32) -> Self {
        Self {
            x: old_height.saturating_sub(self.y + self.height),
            y: self.x,
            width: self.height,
            height: self.width,
        }
    }

    /// The same content after the picture is turned a quarter anticlockwise.
    pub fn turned_counter_clockwise(self, old_width: u32) -> Self {
        Self {
            x: self.y,
            y: old_width.saturating_sub(self.x + self.width),
            width: self.height,
            height: self.width,
        }
    }
}

/// Where a picture being edited comes from: a staged file, or a picture that
/// was pasted and only lives in memory.
#[derive(Clone, Debug, PartialEq)]
pub enum PictureSource {
    File(PathBuf),
    /// Straight RGBA rows, as the clipboard gives them.
    Pasted(Arc<Vec<u8>>),
}

/// A staged picture on its way to being cropped and sent.
#[derive(Clone, Debug, PartialEq)]
pub struct PictureEdit {
    /// The staged attachment it replaces, named rather than numbered, so that
    /// removing another attachment cannot point it at a different picture.
    pub target: crate::app::AttachmentId,
    pub source: PictureSource,
    /// The source picture's own size, before any turn.
    pub width: u32,
    pub height: u32,
    pub crop: PictureCrop,
    /// Quarter turns clockwise already applied to the source.
    pub turns: u8,
}

impl PictureEdit {
    /// A whole, unturned picture from a staged attachment.
    pub fn new(
        target: crate::app::AttachmentId,
        source: PictureSource,
        width: u32,
        height: u32,
    ) -> Self {
        Self {
            target,
            source,
            width: width.max(1),
            height: height.max(1),
            crop: PictureCrop::full(width, height),
            turns: 0,
        }
    }

    /// The source turned to the side the crop is measured in.
    pub fn displayed_size(&self) -> (u32, u32) {
        if self.turns.is_multiple_of(2) {
            (self.width, self.height)
        } else {
            (self.height, self.width)
        }
    }

    /// Turns the picture, carrying the selection round with it.
    pub fn turned(self, clockwise: bool) -> Self {
        let (width, height) = self.displayed_size();
        let crop = if clockwise {
            self.crop.turned_clockwise(height)
        } else {
            self.crop.turned_counter_clockwise(width)
        };
        Self {
            turns: if clockwise {
                (self.turns + 1) % 4
            } else {
                (self.turns + 3) % 4
            },
            crop,
            ..self
        }
    }

    /// The whole picture again, the way it started: no turn and nothing
    /// cropped away. This is what the editor's Reset does, so it has to undo
    /// the turn as well as the region.
    pub fn reset(self) -> Self {
        Self {
            crop: PictureCrop::full(self.width, self.height),
            turns: 0,
            ..self
        }
    }
}

/// Where a staged picture came from before it was cropped. Keeping it means
/// cropping the same picture again opens on the original with its crop, so a
/// second pass does not stack another lossy generation on the first.
#[derive(Clone, Debug, PartialEq)]
pub struct PictureOrigin {
    pub source: PictureSource,
    pub width: u32,
    pub height: u32,
    pub crop: PictureCrop,
    pub turns: u8,
}

/// A picture on its way to becoming a sticker.
#[derive(Clone, Debug, PartialEq)]
pub struct StickerDraft {
    pub source: PathBuf,
    pub width: u32,
    pub height: u32,
    /// The picture has see-through pixels.
    pub transparent: bool,
    pub crop: StickerCrop,
    /// Keep see-through pixels instead of filling them with white.
    pub keep_transparent: bool,
    /// Emojis typed for search, WhatsApp's suggestions, or both.
    pub emojis: String,
}

/// Sticker pack stored as a folder of WebP files, imported or made here.
#[derive(Clone, Debug, PartialEq)]
pub struct StickerPack {
    pub name: String,
    pub dir: PathBuf,
    pub stickers: Vec<PathBuf>,
    /// Put together in ZapFast, so stickers can be filed into it.
    pub local: bool,
}

/// GIF search failure.
#[derive(Clone, Debug, PartialEq)]
pub struct GifError {
    pub message: String,
    /// GIPHY rejected the API key.
    pub bad_key: bool,
}

/// One page from GIPHY, with an offset based on all returned API entries.
#[derive(Clone, Debug, PartialEq)]
pub struct GifPage {
    pub gifs: Vec<Gif>,
    pub next_offset: Option<usize>,
}

/// A GIF found through GIPHY.
#[derive(Clone, Debug, PartialEq)]
pub struct Gif {
    pub id: String,
    /// Downloaded still-frame path.
    pub still: Option<PathBuf>,
    pub mp4: String,
    pub width: u32,
    pub height: u32,
}

/// The tab open in the info panel's media view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaTab {
    Media,
    Docs,
    Links,
}

/// What the info panel shows: the chat's overview, or its media view on one
/// of the tabs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InfoView {
    Overview,
    Media(MediaTab),
    /// A group's permissions, for its admins.
    Permissions,
}

/// The info panel beside the conversation. It can show a member of the open
/// group, so its chat is not always the open one.
#[derive(Clone, Debug, PartialEq)]
pub struct InfoPanel {
    pub chat: ChatId,
    pub view: InfoView,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Dialog {
    Shortcuts,
    About,
    ConfirmUnlink,
    ConfirmRemoveAccount(AccountId),
    /// Confirms emptying one account's media cache.
    ConfirmClearMediaCache(AccountId),
    /// Phone number used for pairing-code linking.
    PairWithPhone,
    /// Contacts and the self-chat shortcut.
    NewChat,
    /// Manually entered number for messaging or saving a contact.
    NewContact,
    UnlockLockedChats,
    ConfirmLockChat(ChatId),
    /// Manages the local labels.
    Labels,
    /// Confirms deleting a chat, which cannot be undone.
    ConfirmDeleteChat(ChatId),
    /// Confirms clearing a chat's messages, which cannot be undone.
    ConfirmClearChat(ChatId),
    /// Leaves a group or channel, optionally archiving the chat.
    ConfirmLeaveGroup(ChatId),
    /// Confirms deleting one message. The archive is the only copy, so a
    /// local delete cannot be undone either.
    ConfirmDeleteMessage {
        chat: ChatId,
        message: String,
        /// Revokes for everyone instead of deleting only this copy.
        for_everyone: bool,
    },
    /// Confirms removing an imported sticker pack from disk.
    ConfirmRemoveStickerPack {
        name: String,
        dir: PathBuf,
    },
    /// Confirms deleting the complete selection, captured when the menu opens.
    ConfirmDeleteSelection {
        chat: ChatId,
        messages: Vec<String>,
        for_everyone: bool,
    },
    /// Chooses a destination for an archived message.
    Forward {
        chat: ChatId,
        /// Message ids, in the order they appear in the chat.
        messages: Vec<String>,
    },
    CreatePoll(ChatId),
    /// Picks the spot to send, from coordinates or a map link.
    SendLocation(ChatId),
    PollResults {
        chat: ChatId,
        message: String,
    },
    InteractiveList {
        chat: ChatId,
        message: String,
        button: usize,
    },
    /// Previews a group invite link before joining.
    JoinGroup,
    /// Confirms setting aside an archive whose key is gone.
    ConfirmStartOver,
    /// The stickers of a pack shared in a chat, with a button to add it.
    StickerPack,
    /// Crops a picture into a sticker.
    StickerMaker,
    /// Who has received and read one of our messages.
    MessageInfo {
        chat: ChatId,
        message: String,
    },
    /// Picks people to add to a group, with New group's check boxes.
    AddGroupMembers(ChatId),
    /// Confirms revoking a group's invite link for a new one.
    ConfirmResetInviteLink(ChatId),
    /// Confirms removing someone from a group.
    ConfirmRemoveMember {
        chat: ChatId,
        member: ChatId,
    },
}

/// One recipient's receipts for one of our group messages.
#[derive(Clone, Debug, PartialEq)]
pub struct Recipient {
    pub id: String,
    /// Named in the audience saved when the message was sent.
    pub expected: bool,
    pub delivered_at: Option<i64>,
    pub read_at: Option<i64>,
    pub played_at: Option<i64>,
}

/// Per-recipient receipts for one of our group messages, as far as they are
/// known. Receipts are only kept from when ZapFast began recording them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MessageReceipts {
    pub chat: ChatId,
    pub message: String,
    pub recipients: Vec<Recipient>,
}

impl MessageReceipts {
    /// Whether the message's audience was saved, so that members without a
    /// receipt are known to be waiting rather than simply unrecorded.
    pub fn audience_known(&self) -> bool {
        self.recipients.iter().any(|recipient| recipient.expected)
    }

    /// Recipients who played a voice or video note, most recent first.
    pub fn played(&self) -> Vec<&Recipient> {
        self.newest_first(|recipient| recipient.played_at)
    }

    /// Recipients who read the message without playing it, most recent first.
    pub fn read(&self) -> Vec<&Recipient> {
        self.newest_first(|recipient| recipient.read_at.filter(|_| recipient.played_at.is_none()))
    }

    /// Recipients whose device has the message but who have not read it yet.
    pub fn delivered(&self) -> Vec<&Recipient> {
        self.newest_first(|recipient| {
            recipient
                .delivered_at
                .filter(|_| recipient.read_at.is_none() && recipient.played_at.is_none())
        })
    }

    /// Audience members with no receipt at all.
    pub fn remaining(&self) -> usize {
        self.recipients
            .iter()
            .filter(|recipient| {
                recipient.expected
                    && recipient.delivered_at.is_none()
                    && recipient.read_at.is_none()
                    && recipient.played_at.is_none()
            })
            .count()
    }

    fn newest_first(&self, at: impl Fn(&Recipient) -> Option<i64>) -> Vec<&Recipient> {
        let mut rows: Vec<_> = self
            .recipients
            .iter()
            .filter_map(|recipient| Some((at(recipient)?, recipient)))
            .collect();
        rows.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.id.cmp(&b.1.id)));
        rows.into_iter().map(|(_, recipient)| recipient).collect()
    }
}

/// A group invite link being previewed or joined.
#[derive(Clone, Debug, PartialEq)]
pub struct GroupInvite {
    pub code: String,
    pub state: InviteState,
}

#[derive(Clone, Debug, PartialEq)]
pub enum InviteState {
    Loading,
    Ready(InviteInfo),
    Joining(InviteInfo),
    Failed(String),
}

/// What an invite link says about its group, without joining it.
#[derive(Clone, Debug, PartialEq)]
pub struct InviteInfo {
    pub id: ChatId,
    pub subject: String,
    pub description: Option<String>,
    pub members: usize,
    /// Admins approve new members before they join.
    pub approval: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToastKind {
    Info,
    Error,
}

/// Info toasts fade after a few seconds; errors stay until dismissed so they
/// can be read to the end and copied.
#[derive(Clone, Debug)]
pub struct Toast {
    pub message: String,
    pub kind: ToastKind,
    pub created: Instant,
}

/// A scroll request for the open chat's message list, from the keyboard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scroll {
    /// About one screen toward older messages.
    PageUp,
    /// About one screen toward newer messages.
    PageDown,
    /// The top of the loaded history.
    Top,
    /// The newest message, eased. `Action::ScrollToBottom` (Ctrl+End) jumps
    /// there at once.
    Bottom,
}

/// How many chats a message may be forwarded to at once, as on WhatsApp.
pub const FORWARD_LIMIT: usize = 5;

/// The forwarding limit for messages: one chat when any of them was
/// forwarded many times, [`FORWARD_LIMIT`] otherwise.
pub fn forward_limit(frequently_forwarded: bool) -> usize {
    if frequently_forwarded {
        1
    } else {
        FORWARD_LIMIT
    }
}

/// Actions queued by views and applied after drawing.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Open(Page),
    /// Opens settings, or closes them when they are already showing.
    ToggleSettings,
    /// Steps back to the previously visited page, chat, or dialog, as a
    /// browser's back button does.
    NavigateBack,
    /// Steps forward again after a back step.
    NavigateForward,
    OpenChat(ChatId),
    /// Starts a voice call with the chat: the person, or everyone in a group.
    StartCall(ChatId),
    /// Starts a video call with the chat: the person, or everyone in a group.
    StartVideoCall(ChatId),
    /// Picks a call history row on the Calls page: its chat opens beside the
    /// list at the call, with the chat's own call buttons in its header.
    /// Carries the row's first call, which picks the row out.
    PickCall {
        chat: ChatId,
        message: String,
    },
    /// Opens the new-call list over the Calls list, or closes it.
    ShowNewCall(bool),
    /// Answers the ringing incoming call.
    AnswerCall,
    /// Declines the ringing incoming call.
    DeclineCall,
    /// Ends the current call.
    HangupCall,
    /// Mutes or unmutes the current call's microphone.
    SetCallMuted(bool),
    /// Turns the current call's camera on or off.
    SetCallCamera(bool),
    /// Rebinds the current call's microphone; `None` is the system default.
    SetCallMicrophone(Option<String>),
    /// Rebinds the current call's speaker; `None` is the system default.
    SetCallSpeaker(Option<String>),
    /// Switches the current call's camera node.
    SetCallCameraDevice(Option<String>),
    /// Steps away from the full call screen without ending the call.
    LeaveCallSurface,
    /// Opens the full call screen again.
    ReturnToCall,
    /// Takes the call surface full screen, or leaves full screen again. The window moves; the call
    /// does not.
    ToggleCallFullscreen,
    /// Creates and opens a chat for a contact without one.
    StartChat {
        id: ChatId,
        name: String,
        dismiss_dialog: bool,
    },
    /// Puts a template in the named chat's composer without sending it, the
    /// way a share link's `text=` asks. Carries the chat it belongs to, since
    /// a chat the app refuses to open must not put its text in another one.
    PrefillComposer {
        chat: ChatId,
        text: String,
    },
    /// Opens a chat at a message search result.
    OpenMessage {
        chat: ChatId,
        message: String,
    },
    /// Opens the search pane beside the open chat, or focuses its field.
    OpenChatSearch,
    /// Closes the pane and drops its query and day.
    CloseChatSearch,
    /// Opens the info panel beside the conversation for a chat or a group
    /// member, on its overview.
    OpenInfo(ChatId),
    CloseInfo,
    /// Moves the open info panel between its overview and its media view.
    ShowInfo(InfoView),
    /// Asks the worker again for the panel's listing after it failed.
    RetryInfoMedia,
    /// Closes the contact name editor without saving.
    CloseContactEdit,
    /// Replaces the query of the open chat's search.
    ChatSearch(String),
    /// Restricts the in-chat search to a local calendar day.
    SetChatSearchDay(Option<jiff::civil::Date>),
    CloseChat,
    SendText {
        chat: ChatId,
        text: String,
        /// Quoted message id.
        quoting: Option<crate::model::ReplyTarget>,
    },
    /// Sends a spot as a location message.
    SendLocation {
        chat: ChatId,
        latitude: f64,
        longitude: f64,
        /// Quoted message, in this chat or, for a private reply, in its group.
        quoting: Option<ReplyTarget>,
    },
    ReplyInteractive {
        chat: ChatId,
        message: String,
        button: usize,
        choice: Option<usize>,
    },
    CreatePoll {
        chat: ChatId,
        draft: PollDraft,
    },
    RefreshPoll {
        chat: ChatId,
        message: String,
    },
    VotePoll {
        chat: ChatId,
        message: String,
        choices: Vec<usize>,
    },
    /// Updates our typing state in a chat.
    Composing {
        chat: ChatId,
        composing: bool,
    },
    MarkRead(ChatId),
    /// Marks a read chat unread, here and on the phone; does not invent a
    /// pending count.
    MarkUnread(ChatId),
    /// Pages older messages from the archive, then the phone. `explicit` when
    /// the reader scrolled to the top, rather than a short chat filling its
    /// view: only the reader's own requests report a phone that is silent.
    LoadOlder {
        chat: ChatId,
        explicit: bool,
    },
    /// Requests messages older than the local archive, for the reader.
    FetchOlder(ChatId),
    ReloadHistory {
        chat: ChatId,
        message: String,
    },
    Download {
        /// The reader asked for it (a click, a menu item): exempt from the
        /// automatic download limit.
        manual: bool,
        card: Option<usize>,
        chat: ChatId,
        message: String,
    },
    /// A downloaded picture or sticker at `path` could not be drawn: its file
    /// is gone, and it downloads again like any other attachment, or it does
    /// not decode, and its bubble offers a retry.
    UnreadableMedia {
        chat: ChatId,
        message: String,
        path: PathBuf,
    },
    /// Downloads a motion photo's clip.
    DownloadMotion {
        chat: ChatId,
        message: String,
    },
    /// Plays a motion photo's clip over its photo, downloading it first, or
    /// goes back to the photo.
    ToggleMotion {
        chat: ChatId,
        message: String,
    },
    /// Plays or pauses a downloaded voice or audio message.
    PlayVoice {
        message: String,
        path: PathBuf,
    },
    /// Seeks to a fraction from 0 to 1 and starts playback.
    SeekVoice {
        message: String,
        path: PathBuf,
        fraction: f32,
    },
    /// Sets the voice playback speed to one of the supported speeds.
    SetVoiceSpeed(f32),
    /// Plays or pauses a downloaded video inside its message.
    PlayVideo {
        message: String,
        path: PathBuf,
    },
    /// Plays a video in the open chat once its download finishes.
    PlayVideoWhenDownloaded(String),
    /// Jumps to a fraction from 0 to 1 of the playing video.
    SeekVideo {
        message: String,
        fraction: f32,
    },
    /// Mutes or unmutes video playback.
    ToggleVideoSound,
    /// Plays videos at the next of 1x, 1.5x and 2x.
    CycleVideoSpeed,
    /// Plays a downloaded video, or a motion photo's clip (`message` is then
    /// the photo), in the full-window viewer, starting it if it is not the
    /// one loaded. Closing the viewer puts a video back in its message.
    ExpandVideo {
        message: String,
        path: PathBuf,
    },
    /// Starts, cancels, or sends a voice recording.
    StartRecording,
    CancelRecording,
    SendRecording,
    /// Drops a voice message the worker refused to send.
    DiscardUnsentVoice,
    /// Opens a downloaded image in ZapFast's native preview. Only the file
    /// extension and existence are checked here, and anything else opens
    /// externally; an image that then fails to decode shows a message with an
    /// Open externally button inside the preview.
    PreviewImage {
        path: PathBuf,
        /// The chat whose album the viewer browses from here.
        chat: ChatId,
        /// The message this picture came from, so the album can be positioned.
        message: String,
        /// Which of an interactive message's pictures, as `MediaItem::card`.
        card: Option<usize>,
    },
    /// Moves the viewer to the next (`1`) or previous (`-1`) item of the album:
    /// the Right and Left arrow keys, and the arrows over the picture.
    ViewerStep(i8),
    /// Shows one item of the album the viewer already has, by message id.
    ViewImage {
        message: String,
        /// Which of an interactive message's pictures, as `MediaItem::card`.
        card: Option<usize>,
    },
    ZoomImageIn,
    /// Scales the previewed image by a factor, as the wheel or a pinch asks.
    ZoomImageBy(f32),
    /// Shows the previewed image at its original size.
    ImageActualSize,
    ZoomImageOut,
    FitImage,
    CloseImagePreview,
    /// Shows a downloaded sticker large, with a favorite toggle.
    PreviewSticker(PathBuf),
    CloseStickerPreview,
    OpenFile(PathBuf),
    /// Opens ZapFast's log, or shows it in its folder when no application
    /// takes it, and says so when neither works.
    OpenLog(PathBuf),
    OpenFolder(PathBuf),
    /// Saves a copy of a downloaded attachment where the person chooses.
    SaveAttachmentAs {
        path: PathBuf,
        name: String,
    },
    /// Saves a group of downloaded album attachments into one chosen folder.
    SaveAttachments {
        files: Vec<(PathBuf, String)>,
    },
    OpenUrl(String),
    CopyText(String),
    CopyImage(PathBuf),
    /// Closes the toast at this index. Only errors wait to be dismissed.
    DismissToast(usize),
    /// Starts a reply to a message in the open chat.
    Reply(String),
    /// Opens the group sender's direct chat with this message quoted.
    ReplyPrivately {
        chat: ChatId,
        message: String,
    },
    CancelReply,
    /// Starts selecting messages in the open chat with none picked yet.
    StartSelection,
    /// Adds a chat to the forwarding dialog's recipients or removes it,
    /// within the forwarding limit. Nothing is sent.
    ToggleForwardRecipient(ChatId),
    /// Shows the chosen recipients for confirmation (two or more).
    ReviewForward,
    /// Returns from the confirmation to the recipient list, keeping it.
    BackToForwardSelection,
    /// Forwards to the chosen recipients: one at once, several after
    /// their confirmation.
    ConfirmForward,
    /// Starts selecting messages in the open chat, beginning with this one.
    SelectMessage(String),
    /// Adds a message to the selection or removes it.
    ToggleSelected(String),
    /// Selects every message from the last one clicked to this one.
    SelectRange(String),
    /// Selects the messages a mouse drag has swept, from the row it began on
    /// to the row under the pointer, starting a selection if none was open.
    SweepMessages {
        anchor: String,
        to: String,
    },
    /// The mouse button that swept messages was released.
    EndSweep,
    /// Leaves selection mode.
    CancelSelection,
    /// Loads an outgoing message into the composer for editing.
    Edit(String),
    CancelEdit,
    /// Revokes an outgoing message for everyone. The chat travels with the
    /// message because the reader may switch chats before confirming.
    DeleteForEveryone {
        chat: ChatId,
        id: String,
    },
    /// Deletes a message locally, in the chat it belongs to.
    DeleteForMe {
        chat: ChatId,
        id: String,
    },
    /// Stars or unstars one message from its bubble's menu.
    SetStar {
        chat: ChatId,
        message: String,
        starred: bool,
    },
    /// Opens the attachment picker for the current chat. With `documents`
    /// the picked files go as documents, pictures included.
    Attach {
        documents: bool,
    },
    /// Opens or closes the composer tools menu.
    SetComposerTools(bool),
    SendFiles(Vec<PathBuf>),
    /// Clipboard image as straight-alpha RGBA.
    PasteImage {
        width: usize,
        height: usize,
        rgba: Vec<u8>,
    },
    /// Toggles a picker tab.
    TogglePicker(PickerTab),
    ClosePicker,
    /// Opens the full emoji picker to react to a message. `beside_menu` keeps
    /// the message's context menu open next to it, as when the picker comes
    /// from the menu's "+"; the hover button opens the picker alone.
    /// `from_list`: opened by the message list's E, so closing the picker
    /// returns focus to the list.
    OpenReactionPicker {
        chat: ChatId,
        message: String,
        beside_menu: bool,
        from_list: bool,
    },
    /// Inserts an emoji at the composer cursor.
    InsertEmoji(String),
    /// Replaces an active `:query` with its selected emoji.
    InsertEmojiCompletion {
        emoji: String,
        start: usize,
        end: usize,
    },
    CloseEmojiSuggestions,
    /// Replaces the active `@` query with a selected group member.
    InsertMention {
        id: String,
        name: String,
        start: usize,
        end: usize,
    },
    CloseMentions,
    SendSticker(PathBuf),
    /// Saves a sticker for the picker.
    SaveSticker(PathBuf),
    /// Removes a saved sticker.
    ForgetSticker(PathBuf),
    /// Takes a sticker out of Recent.
    RemoveRecentSticker(PathBuf),
    /// Imports a sticker pack from a signal.art link.
    ImportStickerUrl(String),
    /// Selects and imports a .wastickers or zip file.
    PickStickerArchive,
    /// Deletes a pack directory.
    DeleteStickerPack(PathBuf),
    /// Creates a local sticker pack.
    CreateStickerPack(String),
    /// Shows one list in the sticker tab.
    SelectStickerShelf(StickerShelf),
    /// Opens a sticker pack shared in the open chat.
    ViewStickerPack(String),
    /// Adds the sticker pack being viewed to the packs here.
    AddStickerPack,
    /// Sends a pack to the open chat as a WhatsApp sticker pack.
    ShareStickerPack(PathBuf),
    /// Chooses a picture for the sticker maker.
    PickStickerPicture,
    /// Makes the drafted sticker, then sends it to the open chat or adds it
    /// to favorites.
    MakeSticker {
        send: bool,
    },
    /// Files a sticker into a local pack, or takes it out of it.
    SetStickerPack {
        pack: PathBuf,
        sticker: PathBuf,
        member: bool,
    },
    /// Opens the contact-name editor for `id`, prefilled with `name` and
    /// split as the contact's saved first name says.
    EditContact {
        id: String,
        name: String,
    },
    /// Saves a contact through WhatsApp contact sync. `first` is the short
    /// display name and `last` completes the full name.
    SaveContact {
        id: String,
        first: String,
        last: String,
    },
    /// Checks a number, optionally saves it, and opens its chat.
    /// `to_phone` is the dialog's "Save to phone" choice; `None` uses the
    /// last one.
    NewContact {
        phone: String,
        first: String,
        last: String,
        to_phone: Option<bool>,
    },
    /// Searches GIFs or lists trending results for an empty query.
    SearchGifs(String),
    MoreGifs,
    SendGif(Gif),
    React {
        chat: ChatId,
        message: String,
        emoji: String,
    },
    SetArchived(ChatId, bool),
    /// Leaves a group or a channel. `archive` also hides the chat in Archived.
    LeaveGroup {
        chat: ChatId,
        archive: bool,
    },
    /// Deletes a chat here and on the phone.
    DeleteChat(ChatId),
    /// Clears a chat's messages here and on the phone, keeping the chat.
    ClearChat(ChatId),
    SetPinned(ChatId, bool),
    /// Pins or unpins one message for everyone in the chat.
    SetMessagePinned {
        chat: ChatId,
        message: String,
        pinned: bool,
    },
    /// Marks a chat as a favorite, or removes the mark, here and on the phone.
    SetFavorite(ChatId, bool),
    ShowDialog(Dialog),
    CloseDialog,
    ToggleSidebar,
    SetChatFilter(ChatFilter),
    /// Picks the chip the Calls page's history shows.
    SetCallFilter(CallFilter),
    /// Picks the label the chat list shows; `None` shows every chat.
    SelectLabel(Option<String>),
    /// Replaces the labels worn by one chat.
    SetChatLabels {
        chat: ChatId,
        labels: Vec<String>,
    },
    /// Creates a label from the name and colour in the manager dialog.
    CreateLabel {
        name: String,
        color_hex: String,
    },
    /// Renames and recolours a label.
    UpdateLabel {
        id: String,
        name: String,
        color_hex: String,
    },
    /// Deletes a label and takes it off every chat.
    DeleteLabel(String),
    /// Shows or leaves the archived chats.
    ShowArchived(bool),
    /// Shows or hides the starred messages in the left panel.
    ToggleStarred,
    /// Mutes (`true`) or unmutes every followed channel.
    MuteAllChannels(bool),
    /// Joins the group of the invite being previewed.
    JoinGroup,
    /// Gives the New chat dialog's contacts check boxes, to make a group.
    StartNewGroup,
    /// Picks someone for the new group, or lets them go.
    ToggleNewGroupMember(ChatId),
    /// From choosing the new group's members to naming it.
    NewGroupNext,
    /// One step back: from naming to choosing, from choosing to the list.
    NewGroupBack,
    /// Asks for a photo for the new group.
    PickNewGroupPicture,
    RemoveNewGroupPicture,
    /// Creates the group the New chat dialog put together.
    CreateGroup,
    /// Asks WhatsApp for a group's invite link, for the info panel.
    FetchInviteLink(ChatId),
    /// Copies the group's invite link, once it is known, with a toast.
    CopyInviteLink(ChatId),
    /// Revokes the group's invite link; WhatsApp answers with a new one.
    ResetInviteLink(ChatId),
    /// Adds the people picked in the Add members dialog to its group.
    AddGroupMembers,
    /// Changes one of the group's permissions: `members` true lets every
    /// member, false only admins (for approval: true turns it on).
    SetGroupPermission {
        chat: ChatId,
        permission: crate::group_admin::Permission,
        members: bool,
    },
    /// Makes a member an admin, dismisses one, or removes them.
    ChangeGroupMember {
        chat: ChatId,
        member: ChatId,
        change: crate::group_admin::MemberChange,
    },
    /// A chat opened from the main list, kept there under the Unread filter.
    KeepUnread(ChatId),
    /// Focuses the chat-list search and leaves the open chat alone.
    FocusChatList,
    FocusSearch,
    /// Focuses the search field on the Settings page.
    FocusSettingsSearch,
    /// Filters the Settings page to the rows matching this text.
    SearchSettings(String),
    FocusComposer,
    SetShortcutHints(bool),
    DismissChatLockHint,
    OpenLockedFolder,
    UnlockLockedFolder(String),
    CreateChatLockCode(String),
    MessageYourself,
    CloseLockedFolder,
    SetChatLockCode(Option<String>),
    ScrollToBottom,
    /// Scrolls the open chat by about a page, or to the top of its history.
    ScrollPage(Scroll),
    /// Moves the open chat's current message, from its focused message list.
    MessageCursor(crate::ui::list_nav::Step),
    /// Gives the open chat's message list keyboard focus.
    FocusMessages,
    /// Opens a message's context menu in the open chat, from the keyboard.
    OpenMessageMenu(String),
    /// The menu asked for by `OpenMessageMenu` is open: stop asking, so it
    /// opens once and closes as any menu does.
    MessageMenuOpened,
    /// Scrolls the open chat to a message.
    ScrollTo(String),
    /// Goes to a replied message from the keys: as `ScrollTo`, but the
    /// message list moves only once the message is loaded.
    GoToReplied(String),
    /// Updates chat-list search text.
    Search(String),
    ShowUpdate,
    CloseUpdate,
    DownloadUpdate,
    InstallUpdate,
    SetTheme(crate::settings::ThemeChoice),
    /// Draws the interface in the platform's font or in the bundled Inter.
    SetFont(crate::settings::FontChoice),
    /// Windows and Linux: draws ZapFast's own title bar and window buttons
    /// (`true`) or the desktop's. Decorations are fixed when a window is
    /// made, so the window is replaced.
    SetCustomTitlebar(bool),
    SetInterfaceLanguage(Option<crate::i18n::Locale>),
    SetCustomTheme(String),
    SetWallpaperColor(crate::settings::WallpaperColor),
    SetWallpaperDoodles(bool),
    /// Asks for an image to use as the chat wallpaper.
    PickWallpaperImage,
    /// Goes back to the wallpaper colour and deletes the copied image.
    RemoveWallpaperImage,
    ReloadThemes,
    OpenThemesFolder,
    SettingsChanged,
    /// Writes one WhatsApp account privacy category on the phone.
    SetAccountPrivacy {
        kind: crate::privacy::PrivacyKind,
        choice: crate::privacy::PrivacyChoice,
    },
    /// Registers or removes the login entry that starts ZapFast in the tray.
    SetStartWithSystem(bool),
    /// Sets the sound for mentions and replies to us (`true`) or for
    /// other new messages.
    SetNotificationSound {
        mention: bool,
        sound: crate::settings::NotificationSound,
    },
    /// Asks for an audio file to use as a notification sound.
    PickNotificationSound {
        mention: bool,
    },
    /// Sets a chat's own notification sound; `None` follows Settings.
    SetChatSound {
        chat: ChatId,
        sound: Option<crate::settings::NotificationSound>,
    },
    /// Asks for an audio file for one chat's notifications.
    PickChatSound(ChatId),
    /// Sets a chat's own voice message speed; `None` follows Settings.
    SetChatVoiceSpeed {
        chat: ChatId,
        speed: Option<f32>,
    },
    /// Asks for a folder for new downloads.
    PickDownloadFolder,
    /// Changes our display name and About text; `None` keeps the current one.
    SetProfile {
        name: Option<String>,
        about: Option<String>,
    },
    /// Asks for a picture and makes it our profile picture.
    PickProfilePicture,
    /// Opens the group name editor in the group info dialog, starting from
    /// the current name.
    EditGroupName(String),
    /// Closes the group name editor without renaming.
    CloseGroupName,
    /// Renames a group on WhatsApp; the editor closes.
    SetGroupName {
        chat: ChatId,
        name: String,
    },
    /// Asks for a picture and makes it the group's photo.
    PickGroupPicture(ChatId),
    /// Removes the group's photo.
    RemoveGroupPicture(ChatId),
    /// Sets or resets (`None`) the folder for new downloads.
    SetDownloadFolder(Option<PathBuf>),
    /// Keeps archived chats archived when a new message comes, or not.
    SetKeepChatsArchived(bool),
    /// Saves the proxy setting and reconnects. Empty follows the environment.
    SetProxy(String),
    /// Plays a notification sound once, as a preview.
    PreviewSound(crate::settings::NotificationSound),
    ZoomBy(f32),
    ResetZoom,
    /// Requests a pairing code for a phone number.
    PairWithPhone(String),
    /// Unlinks the device remotely and locally.
    Unlink,
    /// Hides everything behind the app lock, when a password is set.
    LockApp,
    /// Tries the password typed on the lock screen.
    UnlockApp,
    /// Opens (`true`) or closes the lock screen's question about unlinking.
    ForgotAppPassword(bool),
    /// Unlinks this computer from the lock screen. The lock lifts, and its
    /// password is forgotten, once WhatsApp has unlinked it.
    UnlinkLockedApp,
    /// Opens a password form in Settings, or closes it with `None`.
    AppLockForm(Option<crate::app_lock::FormMode>),
    /// Submits the Settings password form.
    SubmitAppLockForm,
    /// How long ZapFast may go unused before it locks.
    SetAutoLock(crate::settings::AutoLock),
    Reconnect,
    /// Sets aside an archive whose key is gone and links again.
    StartOverArchive,
    Quit,
    /// Shows the window, creating it when running headless.
    ShowWindow,
    /// Closes the window while keeping the app in the tray.
    HideWindow,
    /// Applies the configured close-button behavior.
    CloseWindow,
    /// Shows another linked account in the window.
    SwitchAccount(AccountId),
    /// Starts linking another number beside the ones already here.
    AddAccount,
    /// Leaves an account being added before it was linked.
    CancelAddAccount,
    /// Unlinks an account and deletes what is stored here for it.
    RemoveAccount(AccountId),
    /// Deletes one account's cached attachments, never its archive.
    ClearMediaCache(AccountId),
    /// Turns one account's desktop notifications on or off, from the account
    /// switcher, without showing that account. It stays linked either way.
    SetAccountNotifications(AccountId, bool),
    /// Mutes until Unix time, indefinitely with `Some(0)`, or unmutes with `None`.
    SetMuted(ChatId, Option<i64>),
    /// Moves a chat into or out of the locked folder.
    SetLocked(ChatId, bool),
    /// Sends pending attachments with the composer text as caption.
    SendPending {
        /// Account and mentions captured when the composer queues the send.
        account: AccountId,
        chat: ChatId,
        caption: String,
        mentions: Vec<crate::app::ComposerMention>,
    },
    /// Removes one pending attachment.
    RemovePending(usize),
    /// Sends the one eligible staged photo as a WhatsApp view-once message.
    SetPendingViewOnce(bool),
    /// Opens the cropper on a staged picture.
    EditPicture(crate::app::AttachmentId),
    /// Keeps the crop and the turn, replacing the staged picture.
    ApplyPictureEdit,
    /// Closes the cropper, leaving the staged picture as it was.
    CancelPictureEdit,
    /// Removes all pending attachments.
    ClearPending,
}

#[cfg(test)]
mod storage_tests {
    use super::StorageStats;

    #[test]
    fn cached_files_are_counted_by_kind() {
        let mut stats = StorageStats::default();
        for (name, bytes) in [
            ("a.jpg", 10),
            ("b.PNG", 5),
            ("c.mp4", 100),
            ("d.webp", 3),
            ("e.pdf", 7),
            ("f", 1),
        ] {
            stats.add(std::path::Path::new(name), bytes);
        }
        assert_eq!((stats.images, stats.image_bytes), (2, 15));
        assert_eq!((stats.videos, stats.video_bytes), (1, 100));
        assert_eq!((stats.stickers, stats.sticker_bytes), (1, 3));
        assert_eq!((stats.other, stats.other_bytes), (2, 8));
        assert_eq!(stats.bytes_total(), 126);
        assert_eq!(stats.files_total(), 6);
    }

    /// An empty cache draws no bar instead of dividing by zero.
    #[test]
    fn a_storage_bar_is_empty_when_there_is_nothing_to_measure() {
        assert_eq!(StorageStats::bar_width(0, 0, 100.0), 0.0);
        assert_eq!(StorageStats::bar_width(50, 0, 100.0), 0.0);
        assert_eq!(StorageStats::bar_width(50, 100, 200.0), 100.0);
    }

    #[test]
    fn forgetting_the_cache_keeps_files_kept_elsewhere() {
        let media = |path: &str| super::Media {
            mime: "image/jpeg".into(),
            size: 1,
            width: None,
            height: None,
            album: None,
            path: Some(path.into()),
            state: super::MediaState::Idle,
        };
        let mut cached = super::Content::Image {
            caption: None,
            media: media("/cache/media/a.jpg"),
            motion: None,
        };
        let mut kept = super::Content::Image {
            caption: None,
            media: media("/home/me/Downloads/b.jpg"),
            motion: None,
        };
        for content in [&mut cached, &mut kept] {
            content.forget_files_in(std::path::Path::new("/cache/media"));
        }
        assert_eq!(cached.media().unwrap().path, None);
        assert!(kept.media().unwrap().path.is_some());
    }
}

#[cfg(test)]
mod tests {
    /// Checks compatibility with older archived quotes that omitted a source chat.
    #[test]
    fn archived_quotes_without_a_source_chat_remain_readable() {
        let old = r#"{"id":"original","sender":"sender@lid","sender_name":null,"summary":"Fixture","mentions":[]}"#;
        let mut quote: super::Quoted = serde_json::from_str(old).unwrap();
        assert_eq!(quote.chat, None);
        quote.chat = Some("fixture@g.us".into());
        let stored = serde_json::to_string(&quote).unwrap();
        assert_eq!(
            serde_json::from_str::<super::Quoted>(&stored).unwrap(),
            quote
        );
    }

    use super::StickerCrop;

    #[test]
    fn a_preview_comes_from_the_line_the_query_matched() {
        let text = super::Content::Text {
            text: "first line\nsecond line with Zebra\nthird".into(),
            preview: None,
        };
        assert_eq!(
            text.text_matching("zebra").as_deref(),
            Some("second line with Zebra"),
            "the matching line, not the first one"
        );
        assert_eq!(
            text.text_matching("First").as_deref(),
            Some("first line"),
            "case does not matter"
        );
        assert_eq!(text.text_matching("nowhere"), None);
        assert_eq!(
            text.text_matching("  "),
            None,
            "an empty query matches nothing"
        );
        // A caption is searched too, and previewed the same way.
        let photo = super::Content::Image {
            motion: None,
            caption: Some("a photo of a Zebra".into()),
            media: media(),
        };
        assert_eq!(
            photo.text_matching("zebra").as_deref(),
            Some("a photo of a Zebra")
        );
        // So is a file name, with no text of its own to show.
        let file = super::Content::Document {
            media: media(),
            file_name: "Zebra report.pdf".into(),
            caption: None,
            pages: None,
        };
        assert_eq!(
            file.text_matching("zebra").as_deref(),
            Some("Zebra report.pdf")
        );
    }

    #[test]
    fn a_left_chat_stops_offering_leave_even_without_members() {
        let me = "me@s.whatsapp.net";
        let mut chat = super::Chat::new("1-2@g.us".into(), "Rust".into());
        // An empty member list means the phone never told us who is in, which
        // is exactly when the old check kept offering Leave after a leave.
        assert!(chat.can_leave(&[me]));
        chat.left = true;
        assert!(!chat.can_leave(&[me]), "we already left");
        assert!(!chat.can_send(), "and we cannot post in it");
        // Being a member again clears it, so a rejoin is leaveable once more.
        chat.left = false;
        chat.participants = vec![me.into()];
        assert!(chat.can_leave(&[me]));
    }

    #[test]
    fn group_info_is_editable_when_open_or_by_admins() {
        let mut chat = super::Chat::new("1-2@g.us".into(), "Rust".into());
        assert!(!chat.can_edit_info(), "unknown until the metadata says");
        chat.info_locked = Some(false);
        assert!(chat.can_edit_info(), "an open group lets every member edit");
        chat.info_locked = Some(true);
        assert!(!chat.can_edit_info(), "a locked group is for admins");
        chat.admin = true;
        assert!(chat.can_edit_info(), "which we are");
        chat.left = true;
        assert!(!chat.can_edit_info(), "a group we left is not ours to edit");

        let mut direct = super::Chat::new("1@s.whatsapp.net".into(), "Ada".into());
        direct.info_locked = Some(false);
        direct.admin = true;
        assert!(!direct.can_edit_info(), "only groups have group info");
    }

    #[test]
    fn mentioning_everyone_is_open_to_small_groups_and_admins() {
        let mut chat = super::Chat::new("1-2@g.us".into(), "Rust".into());
        assert!(
            !chat.can_mention_everyone(),
            "an unknown member list offers nothing"
        );
        chat.participants = vec!["1@s.whatsapp.net".into()];
        assert!(chat.can_mention_everyone());
        chat.participants = (0..32).map(|n| format!("{n}@s.whatsapp.net")).collect();
        assert!(
            chat.can_mention_everyone(),
            "32 people still count as small"
        );
        chat.participants.push("32@s.whatsapp.net".into());
        assert!(!chat.can_mention_everyone(), "a larger group is for admins");
        chat.admin = true;
        assert!(chat.can_mention_everyone());
        chat.read_only = true;
        assert!(!chat.can_mention_everyone(), "a chat we cannot post in");
        chat.read_only = false;
        chat.left = true;
        assert!(!chat.can_mention_everyone());

        let mut direct = super::Chat::new("1@s.whatsapp.net".into(), "Ada".into());
        direct.participants = vec!["1@s.whatsapp.net".into()];
        direct.admin = true;
        assert!(!direct.can_mention_everyone());
    }

    #[test]
    fn looks_unread_covers_counts_and_the_empty_dot() {
        let mut chat = Chat::new("1@s.whatsapp.net".into(), "A".into());
        assert!(!chat.looks_unread());
        chat.marked_unread = true;
        assert!(chat.looks_unread());
        chat.marked_unread = false;
        chat.unread = 2;
        assert!(chat.looks_unread());
    }

    #[test]
    fn a_sticker_crop_stays_square_and_inside_the_picture() {
        let crop = StickerCrop::centered(800, 600);
        assert_eq!(
            crop,
            StickerCrop {
                x: 100,
                y: 0,
                side: 600
            }
        );
        // Dragging past an edge stops at it.
        assert_eq!(
            crop.moved(-500, 40, 800, 600),
            StickerCrop {
                x: 0,
                y: 0,
                side: 600
            }
        );
        // Shrinking keeps the center; growing past the picture stops at it.
        let small = crop.resized(200, 800, 600);
        assert_eq!(
            small,
            StickerCrop {
                x: 300,
                y: 200,
                side: 200
            }
        );
        assert_eq!(
            small.moved(1000, 1000, 800, 600),
            StickerCrop {
                x: 600,
                y: 400,
                side: 200
            }
        );
        assert_eq!(small.resized(5000, 800, 600).side, 600);
        assert_eq!(StickerCrop::centered(0, 0).side, 1);
    }

    use super::{CropEdge, PictureCrop, PictureEdit, PictureSource};

    #[test]
    fn a_crop_keeps_its_size_and_stays_inside_the_picture() {
        let full = PictureCrop::full(800, 600);
        assert_eq!((full.x, full.y, full.width, full.height), (0, 0, 800, 600));
        // A region larger than the picture shrinks to it and starts at the corner.
        let huge = PictureCrop {
            x: 900,
            y: 900,
            width: 4000,
            height: 4000,
        };
        assert_eq!(huge.clamped(800, 600), PictureCrop::full(800, 600));
        // A region hanging off an edge slides back, keeping its size.
        let off = PictureCrop {
            x: 700,
            y: 500,
            width: 400,
            height: 300,
        };
        assert_eq!(
            off.clamped(800, 600),
            PictureCrop {
                x: 400,
                y: 300,
                width: 400,
                height: 300
            }
        );
    }

    #[test]
    fn dragging_an_edge_moves_that_side_only() {
        let full = PictureCrop::full(800, 600);
        assert_eq!(
            full.drag(CropEdge::Left, 100, 0, 800, 600),
            PictureCrop {
                x: 100,
                y: 0,
                width: 700,
                height: 600
            }
        );
        assert_eq!(
            full.drag(CropEdge::TopLeft, 100, 50, 800, 600),
            PictureCrop {
                x: 100,
                y: 50,
                width: 700,
                height: 550
            }
        );
        // Dragging a side past the picture's edge stops at it.
        assert_eq!(full.drag(CropEdge::Left, -100, 0, 800, 600), full);
        // Dragging it past the far side leaves the smallest region.
        assert_eq!(
            full.drag(CropEdge::Left, 1000, 0, 800, 600),
            PictureCrop {
                x: 800 - CROP_MIN_SIDE,
                y: 0,
                width: CROP_MIN_SIDE,
                height: 600
            }
        );
        assert_eq!(
            full.drag(CropEdge::BottomRight, -1000, -1000, 800, 600),
            PictureCrop {
                x: 0,
                y: 0,
                width: CROP_MIN_SIDE,
                height: CROP_MIN_SIDE
            }
        );
    }

    #[test]
    fn dragging_inside_moves_the_region_without_resizing_it() {
        let crop = PictureCrop {
            x: 100,
            y: 100,
            width: 200,
            height: 200,
        };
        assert_eq!(
            crop.drag(CropEdge::Inside, 50, 25, 800, 600),
            PictureCrop {
                x: 150,
                y: 125,
                width: 200,
                height: 200
            }
        );
        assert_eq!(
            crop.drag(CropEdge::Inside, 1000, 1000, 800, 600),
            PictureCrop {
                x: 600,
                y: 400,
                width: 200,
                height: 200
            }
        );
        // A region filling the picture cannot go anywhere.
        assert_eq!(
            PictureCrop::full(800, 600).drag(CropEdge::Inside, 40, 40, 800, 600),
            PictureCrop::full(800, 600)
        );
    }

    #[test]
    fn turning_carries_the_selection_round_with_the_picture() {
        let crop = PictureCrop {
            x: 10,
            y: 5,
            width: 20,
            height: 10,
        };
        // Clockwise, the picture's former height becomes its width.
        let clockwise = crop.turned_clockwise(50);
        assert_eq!(
            clockwise,
            PictureCrop {
                x: 35,
                y: 10,
                width: 10,
                height: 20
            }
        );
        assert!(clockwise.x + clockwise.width <= 50);
        assert!(clockwise.y + clockwise.height <= 100);
        let counter = crop.turned_counter_clockwise(100);
        assert_eq!(
            counter,
            PictureCrop {
                x: 5,
                y: 70,
                width: 10,
                height: 20
            }
        );
        assert!(counter.x + counter.width <= 50);
        assert!(counter.y + counter.height <= 100);
        // Turning back the other way restores the region. The picture is
        // 50 across once it has been turned clockwise.
        assert_eq!(clockwise.turned_counter_clockwise(50), crop);
    }

    #[test]
    fn an_edit_turns_the_picture_underneath_the_crop() {
        let edit = PictureEdit::new(0, PictureSource::File(PathBuf::from("photo.jpg")), 800, 600);
        assert_eq!(edit.displayed_size(), (800, 600));
        let turned = edit.clone().turned(true);
        assert_eq!(turned.turns, 1);
        assert_eq!(turned.displayed_size(), (600, 800));
        assert_eq!(turned.crop, PictureCrop::full(600, 800));
        // Four quarter turns come back to where it started.
        let round = turned.clone().turned(true).turned(true).turned(true);
        assert_eq!(round.turns, 0);
        assert_eq!(round.displayed_size(), (800, 600));
        assert_eq!(round.crop, PictureCrop::full(800, 600));
        // Turning anticlockwise is the same as turning clockwise three times.
        let left = edit.clone().turned(false);
        assert_eq!(left.turns, 3);
        assert_eq!(left.displayed_size(), (600, 800));
    }

    #[test]
    fn resetting_puts_the_picture_back_the_way_it_started() {
        let edit = PictureEdit::new(0, PictureSource::File(PathBuf::from("photo.jpg")), 800, 600)
            .turned(true)
            .reset();
        assert_eq!(edit.turns, 0, "the turn goes back too");
        assert_eq!(edit.displayed_size(), (800, 600));
        assert_eq!(edit.crop, PictureCrop::full(800, 600));
    }

    use super::*;

    #[test]
    fn list_filters_survive_their_stored_form() {
        for chip in ChatFilter::EVERY {
            let key = ListFilter::key(chip, None);
            let read = key.as_deref().and_then(ListFilter::parse);
            match chip {
                ChatFilter::All => assert_eq!(read, None),
                _ => assert_eq!(read, Some(ListFilter::Chip(chip))),
            }
        }
        let key = ListFilter::key(ChatFilter::All, Some("label-7")).unwrap();
        assert_eq!(
            ListFilter::parse(&key),
            Some(ListFilter::Label("label-7".into()))
        );
        assert_eq!(ListFilter::parse("something-newer"), None);
        assert_eq!(ListFilter::parse("label:"), None);
    }

    #[test]
    fn call_filters_survive_their_stored_form() {
        for chip in CallFilter::EVERY {
            assert_eq!(CallFilter::parse(chip.key().as_deref()), chip);
        }
        assert_eq!(CallFilter::All.key(), None);
        assert_eq!(CallFilter::parse(Some("something-newer")), CallFilter::All);
    }

    #[test]
    fn message_receipts_sort_each_recipient_into_one_list() {
        let recipient = |id: &str, expected, delivered, read, played| Recipient {
            id: id.into(),
            expected,
            delivered_at: delivered,
            read_at: read,
            played_at: played,
        };
        let receipts = MessageReceipts {
            chat: "g@g.us".into(),
            message: "m".into(),
            recipients: vec![
                recipient("a", true, Some(10), Some(20), None),
                recipient("b", true, Some(11), Some(30), None),
                recipient("c", true, Some(12), None, None),
                recipient("d", true, None, None, None),
                recipient("e", true, Some(13), Some(14), Some(15)),
                // Joined after the send, or answered under an unsaved alias.
                recipient("f", false, Some(16), None, None),
            ],
        };
        let ids = |rows: Vec<&Recipient>| rows.iter().map(|row| row.id.clone()).collect::<Vec<_>>();
        assert!(receipts.audience_known());
        assert_eq!(ids(receipts.played()), ["e"]);
        assert_eq!(ids(receipts.read()), ["b", "a"]);
        assert_eq!(ids(receipts.delivered()), ["f", "c"]);
        assert_eq!(receipts.remaining(), 1);
        let unknown = MessageReceipts {
            recipients: vec![recipient("a", false, Some(1), None, None)],
            ..Default::default()
        };
        assert!(!unknown.audience_known());
        assert_eq!(unknown.remaining(), 0);
    }

    /// Checks that content rederivation retains each downloaded interactive image path.
    #[test]
    fn rederived_interactive_content_keeps_every_downloaded_image() {
        let image = |path: Option<&str>| Media {
            mime: "image/jpeg".into(),
            size: 1,
            width: None,
            height: None,
            album: None,
            path: path.map(PathBuf::from),
            state: MediaState::Idle,
        };
        let content = |main: Option<&str>, cards: [Option<&str>; 2]| Content::Interactive {
            text: String::new(),
            card: Some(Box::new(InteractiveCard {
                image: Some(image(main)),
                carousel: cards
                    .map(|path| InteractiveCard {
                        image: Some(image(path)),
                        ..Default::default()
                    })
                    .into(),
                ..Default::default()
            })),
        };
        let old = content(Some("/main.jpg"), [None, Some("/second.jpg")]);
        let mut new = content(None, [None, None]);
        new.keep_local_paths(&old);
        assert_eq!(new, old);
    }

    #[test]
    fn polls_validate_trimmed_questions_and_distinct_bounded_answers() {
        let mut draft = PollDraft {
            question: " Lunch? ".into(),
            options: vec![" Pizza ".into(), "Pasta".into()],
            multiple: false,
        };
        let valid = draft.validated().unwrap();
        assert_eq!(valid.question, "Lunch?");
        assert_eq!(valid.options, ["Pizza", "Pasta"]);
        assert_eq!(valid.selectable(), 1);
        draft.options[1] = "Pizza".into();
        assert!(draft.validated().is_err());
        draft.options[1].clear();
        assert!(draft.validated().is_err());
        draft.options = (0..13).map(|i| format!("Answer {i}")).collect();
        assert!(draft.validated().is_err());
        draft.options.pop();
        draft.multiple = true;
        assert_eq!(draft.validated().unwrap().selectable(), 12);
        draft.question = "🍕".repeat(256);
        assert!(draft.validated().is_err());
    }

    /// Builds synthetic attachment metadata for media-state and album regression tests.
    fn media() -> Media {
        Media {
            mime: "image/jpeg".into(),
            size: 1,
            width: None,
            height: None,
            album: None,
            path: None,
            state: MediaState::Idle,
        }
    }

    /// The album holds photos and playable clips: an image, and a video that
    /// is not a GIF, which plays inline as an animation instead. A file the
    /// sender attached is a document, not an album item, whatever its name.
    #[test]
    fn the_album_holds_photos_and_playable_clips() {
        use super::{Content, GalleryKind};
        let picture = Content::Image {
            caption: None,
            media: media(),
            motion: None,
        };
        assert_eq!(picture.gallery_kind(), Some(GalleryKind::Photo));
        let clip = |gif: bool| Content::Video {
            caption: None,
            media: media(),
            seconds: None,
            gif,
            note: false,
        };
        assert_eq!(clip(false).gallery_kind(), Some(GalleryKind::Video));
        assert_eq!(clip(true).gallery_kind(), None, "a GIF plays inline");
        let file = Content::Document {
            media: media(),
            file_name: "holiday.jpg".into(),
            caption: None,
            pages: None,
        };
        assert_eq!(file.gallery_kind(), None);
    }

    #[test]
    fn attachment_download_limit_includes_the_boundary() {
        let mut item = media();
        item.size = ATTACHMENT_DOWNLOAD_LIMIT;
        assert!(item.is_within_download_limit());
        item.size += 1;
        assert!(!item.is_within_download_limit());
    }

    #[test]
    fn kinds_come_from_the_server_part() {
        assert_eq!(ChatKind::from_id("1@s.whatsapp.net"), ChatKind::Direct);
        assert_eq!(ChatKind::from_id("1@lid"), ChatKind::Direct);
        assert_eq!(ChatKind::from_id("1-2@g.us"), ChatKind::Group);
        assert_eq!(ChatKind::from_id("1@newsletter"), ChatKind::Broadcast);
    }

    #[test]
    fn a_group_can_be_left_until_we_are_no_longer_a_member() {
        let me = "me@s.whatsapp.net";
        let mut chat = Chat::new("1-2@g.us".into(), "Rust".into());
        assert!(
            chat.can_leave(&[me]),
            "unknown membership still offers leave"
        );
        chat.participants = vec![me.into(), "other@s.whatsapp.net".into()];
        assert!(chat.can_leave(&[me]));
        chat.participants.retain(|id| id != me);
        assert!(!chat.can_leave(&[me]));
        // Before the worker knows our own pair, the list may name our privacy id.
        chat.participants.push("98765@lid".into());
        assert!(chat.can_leave(&[me, "98765@lid"]));
        assert!(!chat.can_leave(&[me]));
        assert!(!Chat::new("1@s.whatsapp.net".into(), "Ada".into()).can_leave(&[me]));
        assert!(!Chat::new("1@broadcast".into(), "List".into()).can_leave(&[me]));
    }

    #[test]
    fn a_channel_can_be_left_until_it_is_read_only() {
        let me = "me@s.whatsapp.net";
        let mut chat = Chat::new("1@newsletter".into(), "News".into());
        assert!(chat.is_channel());
        assert!(chat.can_leave(&[me]));
        chat.read_only = true;
        assert!(!chat.can_leave(&[me]));
    }

    #[test]
    fn a_broadcast_list_is_not_a_channel() {
        assert!(!Chat::new("1@broadcast".into(), "List".into()).is_channel());
        assert!(Chat::new("1@newsletter".into(), "News".into()).is_channel());
    }

    #[test]
    fn summaries_read_like_whatsapp() {
        assert_eq!(Content::text("hi\nthere").summary(), "hi");
        assert_eq!(
            Content::Image {
                motion: None,
                caption: Some("look".into()),
                media: media()
            }
            .summary(),
            "Photo: look"
        );
        assert_eq!(
            Content::Image {
                motion: None,
                caption: None,
                media: media()
            }
            .summary(),
            "Photo"
        );
        assert_eq!(
            Content::Audio {
                media: media(),
                seconds: Some(65),
                voice_note: true,
                waveform: Vec::new()
            }
            .summary(),
            "Voice message (1:05)"
        );
    }

    #[test]
    fn summaries_show_their_label_in_the_interface_language() {
        use crate::i18n::Locale;
        let photo = Content::Image {
            motion: None,
            caption: Some("look".into()),
            media: media(),
        };
        assert_eq!(photo.summary_in(Locale::Spanish), "Foto: look");
        assert_eq!(photo.summary_in(Locale::English), "Photo: look");
        let voice = Content::Audio {
            media: media(),
            seconds: Some(65),
            voice_note: true,
            waveform: Vec::new(),
        };
        assert_eq!(voice.summary_in(Locale::Spanish), "Mensaje de voz (1:05)");
        // The longest label wins: a video message is not a "Video".
        assert_eq!(
            localized_summary(Locale::Spanish, "Video message (0:12)"),
            "Videomensaje (0:12)"
        );
        // Only a whole label followed by its caption or length is one.
        assert_eq!(
            localized_summary(Locale::Spanish, "Photographs"),
            "Photographs"
        );
        // Text is the sender's words, even when it reads like a label.
        let text = Content::text("Location: home");
        assert_eq!(text.summary_in(Locale::Spanish), "Location: home");
        assert_eq!(text.full_summary_in(Locale::Spanish), "Location: home");
    }

    #[test]
    fn stored_summaries_translate_only_their_labels() {
        use crate::i18n::Locale;
        let quote = |labelled| Quoted {
            chat: None,
            id: "quoted".into(),
            sender: "ada@s.whatsapp.net".into(),
            sender_name: None,
            summary: "Photo: look".into(),
            labelled,
            mentions: Vec::new(),
        };
        assert_eq!(quote(Some(true)).summary_in(Locale::Spanish), "Foto: look");
        assert_eq!(
            quote(Some(false)).summary_in(Locale::Spanish),
            "Photo: look"
        );
        // A quote archived before `labelled` was kept still translates.
        assert_eq!(quote(None).summary_in(Locale::Spanish), "Foto: look");
        let last = |labelled| LastMessage {
            from_me: false,
            sender: String::new(),
            sender_name: None,
            summary: "Photo: look".into(),
            full: "Photo: look\nagain".into(),
            labelled,
            status: Delivery::Read,
        };
        assert_eq!(last(true).summary_in(Locale::Spanish), "Foto: look");
        assert_eq!(last(true).full_in(Locale::Spanish), "Foto: look\nagain");
        assert_eq!(last(false).summary_in(Locale::Spanish), "Photo: look");
    }

    /// Every label has its own literal `gettext` call, so xgettext extracts
    /// it and a translation reaches the screen.
    #[test]
    fn every_summary_label_is_translated() {
        use crate::i18n::Locale;
        // Russian spells every label but GIF in its own script.
        for label in SUMMARY_LABELS.into_iter().filter(|label| *label != "GIF") {
            assert_ne!(summary_label(Locale::Russian, label), label);
        }
    }

    #[test]
    fn full_summaries_keep_every_line_behind_the_summary_label() {
        assert_eq!(Content::text("hi\nthere").full_summary(), "hi\nthere");
        assert_eq!(
            Content::Image {
                motion: None,
                caption: Some("look\nat this".into()),
                media: media()
            }
            .full_summary(),
            "Photo: look\nat this"
        );
        let voice = Content::Audio {
            media: media(),
            seconds: Some(65),
            voice_note: true,
            waveform: Vec::new(),
        };
        assert_eq!(voice.full_summary(), voice.summary());
    }

    #[test]
    fn phones_only_come_from_phone_ids() {
        assert_eq!(
            phone_of("393331234567@s.whatsapp.net"),
            Some("393331234567")
        );
        assert_eq!(phone_of("12345@lid"), None);
        assert_eq!(phone_of("1-2@g.us"), None);
    }

    #[test]
    fn labels_mark_names_people_chose_themselves() {
        let saved = Contact {
            id: "1".into(),
            full_name: Some("Ada".into()),
            first_name: None,
            push_name: Some("ada l".into()),
        };
        assert_eq!(saved.label().as_deref(), Some("Ada"));
        let stranger = Contact {
            id: "2".into(),
            full_name: None,
            first_name: None,
            push_name: Some("Bob".into()),
        };
        assert_eq!(stranger.label().as_deref(), Some("~Bob"));
        assert_eq!(Contact::default().label(), None);
    }

    #[test]
    fn old_text_content_still_parses() {
        let old: Content = serde_json::from_str(r#"{"kind":"text","text":"hi"}"#).expect("parses");
        assert_eq!(old, Content::text("hi"));
    }

    #[test]
    fn content_survives_json() {
        let content = Content::Document {
            media: media(),
            file_name: "a.pdf".into(),
            caption: None,
            pages: Some(3),
        };
        let json = serde_json::to_string(&content).expect("serializes");
        let back: Content = serde_json::from_str(&json).expect("parses");
        assert_eq!(back, content);
    }

    #[test]
    fn live_location_content_survives_json() {
        let content = Content::LiveLocation {
            latitude: 51.5,
            longitude: -0.12,
            accuracy_m: Some(10),
            speed_mps: Some(1.1),
            heading_deg: Some(45),
            sequence: 7,
            ended: true,
            updated: 1_700_000_000,
            newer_on_phone: true,
        };
        let json = serde_json::to_string(&content).expect("serializes");
        let back: Content = serde_json::from_str(&json).expect("parses");
        assert_eq!(back, content);
        // Optional fields default when absent, so a sparse payload still parses.
        let sparse: Content =
            serde_json::from_str(r#"{"kind":"livelocation","latitude":1.0,"longitude":2.0}"#)
                .expect("parses sparse");
        assert_eq!(
            sparse,
            Content::LiveLocation {
                latitude: 1.0,
                longitude: 2.0,
                accuracy_m: None,
                speed_mps: None,
                heading_deg: None,
                sequence: 0,
                ended: false,
                updated: 0,
                newer_on_phone: false,
            }
        );
    }

    #[test]
    fn live_location_is_over_when_ended_or_older_than_the_longest_share() {
        let live = |ended| Content::LiveLocation {
            latitude: 0.0,
            longitude: 0.0,
            accuracy_m: None,
            speed_mps: None,
            heading_deg: None,
            sequence: 1,
            ended,
            updated: 0,
            newer_on_phone: false,
        };
        assert!(!live(false).live_location_over(1_000, 1_000 + LIVE_LOCATION_LIMIT));
        assert!(live(false).live_location_over(1_000, 1_001 + LIVE_LOCATION_LIMIT));
        assert!(live(true).live_location_over(1_000, 1_000));
        assert!(!Content::text("hi").live_location_over(0, i64::MAX));
    }

    fn row(id: &str, timestamp: i64, content: Content) -> Message {
        Message {
            id: id.into(),
            chat: "1@s.whatsapp.net".into(),
            sender: "1@s.whatsapp.net".into(),
            sender_name: None,
            from_me: false,
            timestamp,
            content,
            status: Delivery::None,
            delivered_at: None,
            read_at: None,
            quoted: None,
            reactions: Vec::new(),
            history_order: None,
            edited: false,
            mentions: Vec::new(),
            forwarded: false,
            thumbnail: None,
        }
    }

    #[test]
    fn chat_media_files_pictures_documents_and_links_newest_first() {
        let video = |seconds| Content::Video {
            caption: None,
            media: media(),
            seconds,
            gif: false,
            note: false,
        };
        let document = |name: &str| Content::Document {
            media: media(),
            file_name: name.into(),
            caption: None,
            pages: None,
        };
        let rows = vec![
            row(
                "old-photo",
                10,
                Content::Image {
                    motion: None,
                    caption: None,
                    media: media(),
                },
            ),
            row(
                "sticker",
                11,
                Content::Sticker {
                    media: media(),
                    animated: false,
                },
            ),
            row(
                "voice",
                12,
                Content::Audio {
                    media: media(),
                    seconds: None,
                    voice_note: true,
                    waveform: Vec::new(),
                },
            ),
            row("deleted", 13, Content::Revoked),
            row(
                "view-once",
                14,
                Content::PhoneOnly {
                    view_once: true,
                    live_location: false,
                    once: None,
                },
            ),
            row("plain", 15, Content::text("no link here")),
            row("notes", 16, document("Notes.pdf")),
            row(
                "bare",
                17,
                Content::text("read engine.rocks and https://a.b/c"),
            ),
            row("clip", 18, video(Some(5))),
            row(
                "preview",
                19,
                Content::Text {
                    // Typed without the scheme, previewed with it: one link.
                    text: "see spotifast.rocks".into(),
                    preview: Some(LinkPreview {
                        url: "https://spotifast.rocks/".into(),
                        title: Some("Spotifast".into()),
                        description: None,
                    }),
                },
            ),
            row(
                "new-photo",
                20,
                Content::Image {
                    motion: None,
                    caption: None,
                    media: media(),
                },
            ),
        ];
        let listed = ChatMedia::collect("1@s.whatsapp.net".into(), rows, 10);
        let ids = |rows: &[Message]| rows.iter().map(|row| row.id.clone()).collect::<Vec<_>>();
        assert_eq!(ids(&listed.media), ["new-photo", "clip", "old-photo"]);
        assert_eq!(ids(&listed.docs), ["notes"]);
        assert_eq!(
            listed
                .links
                .iter()
                .map(|link| (
                    link.message.as_str(),
                    link.url.as_str(),
                    link.title.as_deref()
                ))
                .collect::<Vec<_>>(),
            [
                ("preview", "https://spotifast.rocks/", Some("Spotifast")),
                ("bare", "https://engine.rocks/", None),
                ("bare", "https://a.b/c", None),
            ]
        );
        assert!(!listed.media_truncated && !listed.docs_truncated && !listed.links_truncated);
        // Each list is cut at the limit, and the panel is told.
        let rows = (0..4)
            .map(|index| {
                row(
                    &format!("photo-{index}"),
                    index,
                    Content::Image {
                        motion: None,
                        caption: None,
                        media: media(),
                    },
                )
            })
            .collect();
        let cut = ChatMedia::collect("1@s.whatsapp.net".into(), rows, 3);
        assert_eq!(ids(&cut.media), ["photo-3", "photo-2", "photo-1"]);
        assert!(cut.media_truncated, "the pictures were cut");
        assert!(
            !cut.docs_truncated && !cut.links_truncated,
            "the others were not"
        );
    }

    #[test]
    fn links_are_capped_per_address_and_each_list_reports_its_own_cut() {
        let rows = (0..300)
            .map(|index| {
                row(
                    &format!("linked-{index}"),
                    index,
                    Content::text(format!("https://a.b/{index} and https://c.d/{index}")),
                )
            })
            .collect();
        let listed = ChatMedia::collect("1@s.whatsapp.net".into(), rows, MEDIA_LIST_LIMIT);
        assert_eq!(listed.links.len(), MEDIA_LIST_LIMIT, "cut to what is shown");
        assert!(listed.links_truncated, "600 addresses do not fit");
        assert!(!listed.media_truncated && !listed.docs_truncated);
    }
}
