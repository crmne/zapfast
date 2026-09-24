//! UI models for chats, messages, and view actions.
//!
//! The backend translates protocol types into these models, keeping protobufs
//! out of views and giving the archive a stable shape.

use std::path::PathBuf;
use std::time::Instant;

use serde::{Deserialize, Serialize};

/// Chat JID string: `<phone>@s.whatsapp.net`, `<id>@g.us`, or `<id>@lid`.
pub type ChatId = String;

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
    /// Whether this is an announcement group where we cannot post.
    pub read_only: bool,
    /// Whether we confirmed leaving this group or channel. Kept apart from
    /// `read_only`, which an announcement group also carries and which a later
    /// metadata refresh rewrites.
    pub left: bool,
    /// Hidden while WhatsApp chat lock is enabled on the phone.
    pub locked: bool,
    /// Disappearing-message duration in seconds, if enabled.
    pub ephemeral_expiration: Option<u32>,
    /// Labels worn by this chat, in creation order. Local to this computer.
    pub labels: Vec<String>,
    /// This chat's own notification sound; `None` follows Settings.
    pub notification_sound: Option<crate::settings::NotificationSound>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LastMessage {
    pub from_me: bool,
    pub sender: String,
    /// Group-message sender.
    pub sender_name: Option<String>,
    pub summary: String,
    pub status: Delivery,
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
            read_only: false,
            left: false,
            locked: false,
            ephemeral_expiration: None,
            labels: Vec::new(),
            notification_sound: None,
        }
    }

    /// Newsletter publishing permissions are not supported by this client.
    pub fn can_send(&self) -> bool {
        !self.locked && !self.read_only && !self.left && self.kind != ChatKind::Broadcast
    }

    /// A followed WhatsApp channel (newsletter).
    pub fn is_channel(&self) -> bool {
        self.id.ends_with("@newsletter")
    }

    pub fn is_group(&self) -> bool {
        self.kind == ChatKind::Group
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

    /// Whether the member list names any of `ours`.
    pub fn lists_any(&self, ours: &[&str]) -> bool {
        self.participants
            .iter()
            .any(|id| ours.contains(&id.as_str()))
    }

    pub fn muted(&self, now: i64) -> bool {
        matches!(self.muted_until, Some(0)) || self.muted_until.is_some_and(|until| until > now)
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
    /// One-line summary used in chat rows and quotes.
    pub fn summary(&self) -> String {
        self.content.summary()
    }

    /// The line of this message that contains `query`, for a search result's
    /// preview. The archive matches the whole text, so a hit on a later line
    /// would otherwise show a first line the query is nowhere in.
    pub fn text_matching(&self, query: &str) -> Option<String> {
        self.content.text_matching(query)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Quoted {
    pub id: String,
    pub sender: String,
    pub sender_name: Option<String>,
    pub summary: String,
    /// Mentions in quoted text.
    #[serde(default)]
    pub mentions: Vec<MentionRef>,
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
    },
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

/// WhatsApp's longest live location share, in seconds.
pub const LIVE_LOCATION_LIMIT: i64 = 8 * 60 * 60;

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

    pub fn summary(&self) -> String {
        match self {
            Self::Text { text, .. } | Self::Interactive { text, .. } => {
                text.lines().next().unwrap_or_default().to_owned()
            }
            Self::Image { caption, .. } => with_caption("Photo", caption),
            Self::Video {
                caption, gif, note, ..
            } => with_caption(
                if *gif {
                    "GIF"
                } else if *note {
                    "Video message"
                } else {
                    "Video"
                },
                caption,
            ),
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
                view_once: true, ..
            } => "View once message".to_owned(),
            Self::PhoneOnly { .. } => "Message on your phone".to_owned(),
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

    /// Carries downloaded file paths over from `old` when rederiving content
    /// from the raw protobuf: the main attachment and each carousel card's image.
    pub fn keep_local_paths(&mut self, old: &Content) {
        if let (Some(new), Some(old)) = (self.media_mut(), old.media()) {
            new.path = old.path.clone();
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

fn with_caption(label: &str, caption: &Option<String>) -> String {
    match caption
        .as_deref()
        .and_then(|caption| caption.lines().next())
    {
        Some(caption) if !caption.is_empty() => format!("{label}: {caption}"),
        _ => label.to_owned(),
    }
}

/// Maximum size accepted for a downloaded attachment.
pub(crate) const ATTACHMENT_DOWNLOAD_LIMIT: u64 = 64 * 1024 * 1024;

/// Attachment metadata, download state, and optional local file. Download keys
/// remain in the archive's raw message.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Media {
    pub mime: String,
    pub size: u64,
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// Decrypted downloaded file.
    #[serde(default)]
    pub path: Option<PathBuf>,
    /// Non-persisted download state.
    #[serde(skip)]
    pub state: MediaState,
}

impl Media {
    /// Whether the attachment's declared size can be downloaded locally.
    ///
    /// A missing size is represented as zero and is allowed here. The worker
    /// still enforces the limit while streaming it from WhatsApp.
    pub fn is_within_download_limit(&self) -> bool {
        self.size <= ATTACHMENT_DOWNLOAD_LIMIT
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub enum MediaState {
    #[default]
    Idle,
    Downloading,
    Failed(String),
}

/// Contact names from app-state sync and message push names.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Contact {
    pub id: String,
    pub full_name: Option<String>,
    pub push_name: Option<String>,
}

impl Contact {
    pub fn display_name(&self) -> Option<&str> {
        self.full_name
            .as_deref()
            .filter(|name| !name.is_empty())
            .or(self.push_name.as_deref().filter(|name| !name.is_empty()))
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
    Settings,
    Wallpaper,
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
    /// Nothing at all.
    Hidden,
}

/// Which list the sticker tab shows.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum StickerShelf {
    /// Stickers we sent, and the phone's recent list.
    #[default]
    Recent,
    Favorites,
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

#[derive(Clone, Debug, PartialEq)]
pub enum Dialog {
    Shortcuts,
    About,
    ConfirmUnlink,
    /// Phone number used for pairing-code linking.
    PairWithPhone,
    /// Contacts and the self-chat shortcut.
    NewChat,
    /// Manually entered number for messaging or saving a contact.
    NewContact,
    UnlockLockedChats,
    ConfirmLockChat(ChatId),
    ChatInfo(ChatId),
    /// Manages the local labels.
    Labels,
    /// Confirms deleting a chat, which cannot be undone.
    ConfirmDeleteChat(ChatId),
    /// Leaves a group or channel, optionally archiving the chat.
    ConfirmLeaveGroup(ChatId),
    /// Chooses a destination for an archived message.
    Forward {
        chat: ChatId,
        /// Message ids, in the order they appear in the chat.
        messages: Vec<String>,
    },
    CreatePoll(ChatId),
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

/// Actions queued by views and applied after drawing.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Open(Page),
    OpenChat(ChatId),
    /// Creates and opens a chat for a contact without one.
    StartChat {
        id: ChatId,
        name: String,
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
    /// Replaces the query of the open chat's search.
    ChatSearch(String),
    /// Restricts the in-chat search to a local calendar day.
    SetChatSearchDay(Option<jiff::civil::Date>),
    CloseChat,
    SendText {
        chat: ChatId,
        text: String,
        /// Quoted message id.
        quoting: Option<String>,
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
    LoadOlder(ChatId),
    /// Requests messages older than the local archive.
    FetchOlder(ChatId),
    Download {
        card: Option<usize>,
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
    PreviewImage(PathBuf),
    ZoomImageIn,
    /// Shows the previewed image at its original size.
    ImageActualSize,
    ZoomImageOut,
    FitImage,
    CloseImagePreview,
    OpenFile(PathBuf),
    OpenFolder(PathBuf),
    /// Saves a copy of a downloaded attachment where the person chooses.
    SaveAttachmentAs {
        path: PathBuf,
        name: String,
    },
    OpenUrl(String),
    CopyText(String),
    /// Closes the toast at this index. Only errors wait to be dismissed.
    DismissToast(usize),
    /// Starts a reply to a message in the open chat.
    Reply(String),
    CancelReply,
    /// Forwards an archived message to another chat.
    Forward {
        from_chat: ChatId,
        messages: Vec<String>,
        to_chat: ChatId,
    },
    /// Starts selecting messages in the open chat, beginning with this one.
    SelectMessage(String),
    /// Adds a message to the selection or removes it.
    ToggleSelected(String),
    /// Selects every message from the last one clicked to this one.
    SelectRange(String),
    /// Leaves selection mode.
    CancelSelection,
    /// Loads an outgoing message into the composer for editing.
    Edit(String),
    CancelEdit,
    /// Revokes an outgoing message for everyone.
    DeleteForEveryone(String),
    /// Deletes a message locally.
    DeleteForMe(String),
    /// Opens the attachment picker for the current chat.
    Attach,
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
    OpenReactionPicker {
        chat: ChatId,
        message: String,
        beside_menu: bool,
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
    /// Opens the prefilled contact-name editor.
    EditContact(String),
    /// Saves a contact through WhatsApp contact sync. `first` is the short
    /// display name and `last` completes the full name.
    SaveContact {
        id: String,
        first: String,
        last: String,
    },
    /// Checks a number, optionally saves it, and opens its chat.
    NewContact {
        phone: String,
        first: String,
        last: String,
    },
    /// Searches GIFs or lists trending results for an empty query.
    SearchGifs(String),
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
    SetPinned(ChatId, bool),
    /// Marks a chat as a favorite, or removes the mark, here and on the phone.
    SetFavorite(ChatId, bool),
    ShowDialog(Dialog),
    CloseDialog,
    ToggleSidebar,
    SetChatFilter(ChatFilter),
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
    /// Mutes (`true`) or unmutes every followed channel.
    MuteAllChannels(bool),
    /// Joins the group of the invite being previewed.
    JoinGroup,
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
    HideShortcutHints,
    DismissChatLockHint,
    OpenLockedFolder,
    UnlockLockedFolder(String),
    CreateChatLockCode(String),
    MessageYourself,
    CloseLockedFolder,
    SetChatLockCode(Option<String>),
    ScrollToBottom,
    /// Scrolls the open chat to a message.
    ScrollTo(String),
    /// Updates chat-list search text.
    Search(String),
    ShowUpdate,
    CloseUpdate,
    DownloadUpdate,
    InstallUpdate,
    SetTheme(crate::settings::ThemeChoice),
    SetInterfaceLanguage(Option<crate::i18n::Locale>),
    SetCustomTheme(String),
    SetWallpaperColor(crate::settings::WallpaperColor),
    SetWallpaperDoodles(bool),
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
    /// Asks for a folder for new downloads.
    PickDownloadFolder,
    /// Changes our display name and About text; `None` keeps the current one.
    SetProfile {
        name: Option<String>,
        about: Option<String>,
    },
    /// Asks for a picture and makes it our profile picture.
    PickProfilePicture,
    /// Sets or resets (`None`) the folder for new downloads.
    SetDownloadFolder(Option<PathBuf>),
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
    /// Mutes until Unix time, indefinitely with `Some(0)`, or unmutes with `None`.
    SetMuted(ChatId, Option<i64>),
    /// Moves a chat into or out of the locked folder.
    SetLocked(ChatId, bool),
    /// Sends pending attachments with the composer text as caption.
    SendPending {
        chat: ChatId,
        caption: String,
    },
    /// Removes one pending attachment.
    RemovePending(usize),
    /// Removes all pending attachments.
    ClearPending,
}

#[cfg(test)]
mod tests {
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

    use super::*;

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

    #[test]
    fn rederived_interactive_content_keeps_every_downloaded_image() {
        let image = |path: Option<&str>| Media {
            mime: "image/jpeg".into(),
            size: 1,
            width: None,
            height: None,
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

    fn media() -> Media {
        Media {
            mime: "image/jpeg".into(),
            size: 1,
            width: None,
            height: None,
            path: None,
            state: MediaState::Idle,
        }
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
                caption: Some("look".into()),
                media: media()
            }
            .summary(),
            "Photo: look"
        );
        assert_eq!(
            Content::Image {
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
            push_name: Some("ada l".into()),
        };
        assert_eq!(saved.label().as_deref(), Some("Ada"));
        let stranger = Contact {
            id: "2".into(),
            full_name: None,
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
        };
        assert!(!live(false).live_location_over(1_000, 1_000 + LIVE_LOCATION_LIMIT));
        assert!(live(false).live_location_over(1_000, 1_001 + LIVE_LOCATION_LIMIT));
        assert!(live(true).live_location_over(1_000, 1_000));
        assert!(!Content::text("hi").live_location_over(0, i64::MAX));
    }
}
