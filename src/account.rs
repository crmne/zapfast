//! One linked WhatsApp number inside the process.
//!
//! The window's [`crate::app::App`] owns a list of these. Views draw the
//! active account through `App`'s `Deref`. Each account has its own backend
//! thread, archive, and folders.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::Instant;

use crate::app::{AttachmentDraft, ComposerMention, Conversation};
use crate::backend::{Backend, LinkStatus, Waker};
use crate::model::{
    AccountId, Chat, ChatFilter, ChatId, Contact, Label, Message, PollDraft, StickerPack,
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
    pub scroll_chat_into_view: Option<ChatId>,
    pub drafts: HashMap<ChatId, String>,
    pub(crate) attachment_drafts: HashMap<ChatId, AttachmentDraft>,
    pub(crate) draft_mentions: HashMap<ChatId, Vec<ComposerMention>>,
    pub search: String,
    pub search_selected: Option<ChatId>,
    pub search_hits: Vec<Message>,
    pub typing: HashMap<ChatId, Vec<(String, Instant)>>,
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
    pub poll_creating: bool,
    pub poll_voting: HashSet<(ChatId, String)>,
    pub contact_edit: Option<(String, String)>,
    pub new_contact_phone: String,
    pub new_contact_name: String,
    pub new_contact_last: String,
    pub new_contact_pending: bool,
    pub pair_phone: String,
    pub show_archived: bool,
    pub chat_filter: ChatFilter,
    pub labels: Vec<Label>,
    pub account_privacy: crate::privacy::Snapshot,
    pub interactive_sending: HashSet<(ChatId, String)>,
    pub group_saving: HashSet<ChatId>,
    pub(crate) reported_online: Option<bool>,
    /// Chats this account may pin; WhatsApp Plus raises it once known.
    pub pin_limit: usize,
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
            scroll_chat_into_view: None,
            drafts: HashMap::new(),
            draft_mentions: HashMap::new(),
            attachment_drafts: HashMap::new(),
            search: String::new(),
            search_selected: None,
            search_hits: Vec::new(),
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
            poll_creating: false,
            poll_voting: HashSet::new(),
            contact_edit: None,
            new_contact_phone: String::new(),
            new_contact_name: String::new(),
            new_contact_last: String::new(),
            new_contact_pending: false,
            pair_phone: String::new(),
            show_archived: false,
            chat_filter: ChatFilter::All,
            labels: Vec::new(),
            account_privacy: crate::privacy::Snapshot::default(),
            interactive_sending: HashSet::new(),
            group_saving: HashSet::new(),
            reported_online: None,
            pin_limit: crate::backend::PINNED_CHATS,
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

    pub fn save_settings(&mut self) {
        self.settings_dirty = false;
        if let Err(error) = self.settings.save(&self.dirs.settings_file()) {
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
