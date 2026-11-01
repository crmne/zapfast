//! One linked WhatsApp number inside the process.
//!
//! The window's [`crate::app::App`] owns a list of these. Views draw the
//! active account through `App`'s `Deref`. Each account has its own backend
//! thread, archive, and folders.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::Instant;

use crate::app::{AttachmentDraft, ComposerMention, Conversation};
use crate::backend::{Backend, LinkStatus, Waker};
use crate::model::{
    AccountId, Chat, ChatActivity, ChatFilter, ChatId, Contact, Label, Message, PollDraft,
    StickerPack,
};
use crate::paths::{AccountDirs, AppDirs};
use crate::settings::AccountSettings;

/// One WhatsApp account's runtime state.
pub struct Account {
    pub id: AccountId,
    pub dirs: AccountDirs,
    pub settings: AccountSettings,
    pub(crate) settings_dirty: bool,
    pub backend: Backend,
    pub link: LinkStatus,
    pub syncing: bool,
    pub sync_percent: Option<u32>,
    pub me: Option<String>,
    pub me_lid: Option<String>,
    pub me_name: Option<String>,
    pub me_about: Option<String>,
    pub chats: Vec<Chat>,
    pub contacts: HashMap<String, Contact>,
    pub conversations: HashMap<ChatId, Conversation>,
    pub open_chat: Option<ChatId>,
    /// The background-history state last sent to this account's worker: the
    /// switch and the chat on screen.
    pub(crate) prefetch_sent: Option<(bool, Option<ChatId>)>,
    pub scroll_chat_into_view: Option<ChatId>,
    pub drafts: HashMap<ChatId, String>,
    /// Whether the open chat had a draft when opened. Keep its list priority
    /// stable while the reader types, clears, or sends that draft.
    pub(crate) open_draft_priority: bool,
    /// This account's media cache as last measured, for Settings. `None`
    /// until a measurement succeeds, so a failed one shows no numbers
    /// rather than zeroes.
    pub storage_stats: Option<crate::model::StorageStats>,
    /// The measurement being waited for; an older answer is dropped.
    pub storage_token: u64,
    /// Whether the media cache is being emptied.
    pub storage_clearing: bool,
    pub(crate) attachment_drafts: HashMap<ChatId, AttachmentDraft>,
    pub(crate) draft_mentions: HashMap<ChatId, Vec<ComposerMention>>,
    pub search: String,
    pub search_selected: Option<ChatId>,
    pub search_hits: Vec<Message>,
    /// When the text last typed into the chat list's search is scanned for.
    /// Waiting out the typing makes a word cost one scan, not one per letter.
    pub search_due: Option<Instant>,
    /// Normalized forms of the names and previews the chat list's search
    /// compares, keyed by the text they came from. Normalizing ICU text costs
    /// microseconds, and repeating it for every chat on every frame made the
    /// whole window slow for as long as the search was open.
    pub search_keys: RefCell<HashMap<String, String>>,
    pub typing: HashMap<ChatId, Vec<(String, ChatActivity, Instant)>>,
    pub presence: HashMap<String, crate::app::Presence>,
    pub account_receipts_off: bool,
    pub(crate) avatars: HashMap<String, Option<PathBuf>>,
    pub(crate) avatar_requests: HashSet<String>,
    pub(crate) avatars_full: HashMap<String, Option<PathBuf>>,
    pub(crate) avatar_full_requests: HashSet<String>,
    pub(crate) played_told: HashSet<String>,
    pub stickers: Vec<PathBuf>,
    pub stickers_received: Vec<PathBuf>,
    pub stickers_saved: Vec<PathBuf>,
    pub sticker_packs: Vec<StickerPack>,
    pub stickers_pending: bool,
    pub sticker_import_pending: bool,
    pub sticker_link: String,
    pub poll_draft: PollDraft,
    /// What the location dialog has to read, cleared when it opens.
    pub location_draft: String,
    pub poll_creating: bool,
    pub poll_voting: HashSet<(ChatId, String)>,
    pub contact_edit: Option<(String, String)>,
    pub new_contact_phone: String,
    pub new_contact_name: String,
    pub new_contact_last: String,
    pub new_contact_pending: bool,
    pub(crate) new_contact_sequence: u64,
    pub(crate) new_contact_request: Option<u64>,
    pub(crate) new_contact_dialog_request: Option<u64>,
    /// The group being put together in the New chat dialog, if any.
    pub new_group: Option<crate::new_group::Draft>,
    pub(crate) new_group_sequence: u64,
    /// The `CreateGroup` the window waits for; an earlier one's answer files
    /// its group and opens nothing.
    pub(crate) new_group_request: Option<u64>,
    pub pair_phone: String,
    pub show_archived: bool,
    pub chat_filter: ChatFilter,
    /// The chip the Calls page's history shows.
    pub call_filter: crate::model::CallFilter,
    pub labels: Vec<Label>,
    /// Starred messages, newest star first, and whether the left panel lists
    /// them instead of the chats.
    pub starred: Vec<crate::archive::Starred>,
    pub show_starred: bool,
    /// Ids of the starred messages of each chat, for the mark in the
    /// conversation. Filled when a chat opens and on every confirmed star.
    pub stars: HashMap<ChatId, HashSet<String>>,
    /// Ids of each chat's pinned messages, for its menu and its bubble mark.
    pub pins: HashMap<ChatId, HashSet<String>>,
    /// Active pins of each chat, for the line under its header.
    pub chat_pins: HashMap<ChatId, Vec<crate::archive::Pinned>>,
    /// A starred row asked to reply: the quote starts once the message it
    /// points at is loaded, because the composer drops a quote whose message
    /// has not arrived yet.
    pub reply_when_loaded: Option<(ChatId, String)>,
    pub account_privacy: crate::privacy::Snapshot,
    pub interactive_sending: HashSet<(ChatId, String)>,
    pub group_saving: HashSet<ChatId>,
    pub(crate) reported_online: Option<bool>,
    /// Chats this account may pin; WhatsApp Plus raises it once known.
    pub pin_limit: usize,
    /// The call entries the Calls page lists, newest first, once read.
    pub call_log: Vec<Message>,
    pub call_log_loaded: bool,
    /// The call history row last picked on the Calls page, whose chat shows
    /// beside the list: its chat and its first call.
    pub picked_call: Option<(ChatId, String)>,
    /// Whether the new-call list covers the call history.
    pub new_call: bool,
    /// What the Calls list's search field holds.
    pub calls_search: String,
    /// What the new-call list's search field holds.
    pub new_call_search: String,
}

impl Account {
    /// Creates isolated chat, draft and backend state for one linked account.
    pub fn new(
        id: AccountId,
        dirs: AccountDirs,
        settings: AccountSettings,
        backend: Backend,
    ) -> Self {
        let open_chat = settings.last_chat.clone();
        let chat_filter = match settings
            .list_filter
            .as_deref()
            .and_then(crate::model::ListFilter::parse)
        {
            Some(crate::model::ListFilter::Chip(chip)) => chip,
            _ => ChatFilter::All,
        };
        let call_filter = crate::model::CallFilter::parse(settings.call_filter.as_deref());
        Self {
            id,
            dirs,
            settings,
            settings_dirty: false,
            backend,
            link: LinkStatus::Starting,
            syncing: false,
            sync_percent: None,
            me: None,
            me_lid: None,
            me_name: None,
            me_about: None,
            chats: Vec::new(),
            contacts: HashMap::new(),
            conversations: HashMap::new(),
            open_chat,
            prefetch_sent: None,
            scroll_chat_into_view: None,
            drafts: HashMap::new(),
            open_draft_priority: false,
            storage_stats: None,
            storage_token: 0,
            storage_clearing: false,
            draft_mentions: HashMap::new(),
            attachment_drafts: HashMap::new(),
            search: String::new(),
            search_selected: None,
            search_hits: Vec::new(),
            search_due: None,
            search_keys: RefCell::new(HashMap::new()),
            typing: HashMap::new(),
            presence: HashMap::new(),
            account_receipts_off: false,
            avatars: HashMap::new(),
            avatar_requests: HashSet::new(),
            avatars_full: HashMap::new(),
            avatar_full_requests: HashSet::new(),
            played_told: HashSet::new(),
            stickers: Vec::new(),
            stickers_received: Vec::new(),
            stickers_saved: Vec::new(),
            sticker_packs: Vec::new(),
            stickers_pending: false,
            sticker_import_pending: false,
            sticker_link: String::new(),
            poll_draft: Default::default(),
            location_draft: String::new(),
            poll_creating: false,
            poll_voting: HashSet::new(),
            contact_edit: None,
            new_contact_phone: String::new(),
            new_contact_name: String::new(),
            new_contact_last: String::new(),
            new_contact_pending: false,
            new_contact_sequence: 0,
            new_contact_request: None,
            new_contact_dialog_request: None,
            new_group: None,
            new_group_sequence: 0,
            new_group_request: None,
            pair_phone: String::new(),
            show_archived: false,
            chat_filter,
            call_filter,
            labels: Vec::new(),
            starred: Vec::new(),
            show_starred: false,
            stars: HashMap::new(),
            pins: HashMap::new(),
            chat_pins: HashMap::new(),
            reply_when_loaded: None,
            account_privacy: crate::privacy::Snapshot::default(),
            interactive_sending: HashSet::new(),
            group_saving: HashSet::new(),
            reported_online: None,
            pin_limit: crate::backend::PINNED_CHATS,
            call_log: Vec::new(),
            call_log_loaded: false,
            picked_call: None,
            new_call: false,
            calls_search: String::new(),
            new_call_search: String::new(),
        }
    }

    /// Opens a live account from disk, migrating older global fields onto account 1.
    pub fn spawn(
        app_dirs: &AppDirs,
        id: AccountId,
        legacy: &crate::settings::Settings,
        waker: &Waker,
    ) -> std::io::Result<Self> {
        let dirs = app_dirs.account(&id);
        dirs.ensure()?;
        let path = dirs.settings_file();
        let mut settings = if path.exists() {
            AccountSettings::load(&path)
        } else if id.as_str() == "1" {
            // Only `App::new` spawns account 1 (an added account is numbered
            // 2 or higher), before the window's thread is marked, so this
            // write may wait for the disk and still fail the start.
            let migrated = AccountSettings::from_legacy(legacy);
            migrated.save(&path)?;
            migrated
        } else {
            AccountSettings::default()
        };
        resolve_wallpaper_path(&dirs, &mut settings);
        let backend = Backend::spawn(dirs.clone(), waker.clone());
        Ok(Self::new(id, dirs, settings, backend))
    }

    pub fn detached(
        app_dirs: &AppDirs,
        id: AccountId,
        settings: AccountSettings,
    ) -> std::io::Result<(Self, std::sync::mpsc::Sender<crate::backend::Event>)> {
        let dirs = app_dirs.account(&id);
        dirs.ensure()?;
        let (backend, events) = Backend::detached();
        Ok((Self::new(id, dirs, settings, backend), events))
    }

    /// Whether the device has linked data, including while offline.
    pub fn is_linked(&self) -> bool {
        matches!(
            self.link,
            LinkStatus::Connected | LinkStatus::Connecting | LinkStatus::Disconnected { .. }
        ) || (!self.chats.is_empty() && !matches!(self.link, LinkStatus::LoggedOut))
    }

    /// The taskbar count: unarchived, unmuted, unlocked chats that look
    /// unread. WhatsApp counts chats here, not the messages inside them.
    pub fn unread_chat_count(&self) -> u32 {
        let now = crate::util::now();
        let chats = self
            .chats
            .iter()
            .filter(|chat| {
                !chat.archived && !chat.locked && !chat.muted(now) && chat.looks_unread()
            })
            .count();
        u32::try_from(chats).unwrap_or(u32::MAX)
    }

    /// The name the switcher shows: the profile name, else the number,
    /// else "Account" and its number here.
    pub fn display_label(&self, locale: crate::i18n::Locale) -> String {
        if let Some(name) = self
            .me_name
            .as_deref()
            .filter(|name| !name.trim().is_empty())
        {
            return name.trim().to_owned();
        }
        if let Some(phone) = self.phone() {
            return phone;
        }
        crate::i18n::gettext(locale, "Account {id}").replace("{id}", self.id.as_str())
    }

    /// Returns or requests a cached profile picture from this account.
    pub fn avatar(&mut self, id: &str) -> Option<PathBuf> {
        if let Some(known) = self.avatars.get(id) {
            return known.clone();
        }
        if self.avatar_requests.insert(id.to_owned()) {
            self.backend.send(crate::backend::Command::FetchAvatar {
                id: id.to_owned(),
                full: false,
            });
        }
        None
    }

    /// Our phone number, formatted, once WhatsApp has told us.
    pub fn phone(&self) -> Option<String> {
        let me = self.me.as_deref()?;
        crate::model::phone_of(me).map(crate::util::phone)
    }

    pub fn mark_settings_dirty(&mut self) {
        self.settings_dirty = true;
    }

    /// Hands the account's settings to the app's file writer.
    pub fn save_settings(&mut self, files: &fastframe_file::Writer) {
        self.settings_dirty = false;
        if let Err(error) = self.settings.save_to(files, &self.dirs.settings_file()) {
            log::warn!("could not save account settings: {error}");
        }
    }
}

fn resolve_wallpaper_path(dirs: &AccountDirs, settings: &mut AccountSettings) {
    if settings
        .wallpaper_image
        .as_ref()
        .is_some_and(|path| path.exists())
    {
        return;
    }
    for extension in ["jpg", "png", "webp", "gif"] {
        let path = dirs.wallpaper_file(extension);
        if path.exists() {
            settings.wallpaper_image = Some(path);
            return;
        }
    }
    if settings
        .wallpaper_image
        .as_ref()
        .is_some_and(|path| !path.exists())
    {
        settings.wallpaper_image = None;
    }
}
