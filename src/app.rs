//! Application state and the frame loop.
//!
//! Views queue [`Action`]s while drawing. The app applies them after the frame
//! and processes backend events.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::audio::{Player, Recorder};
use crate::backend::{Backend, Command, Event, LinkStatus, Waker};
use crate::model::{
    Action, Chat, ChatId, Contact, Content, Delivery, Dialog, Gif, GifError, Media, MediaState,
    Message, Page, PickerTab, StickerPack, Toast, ToastKind,
};
use crate::paths::AppDirs;
use crate::settings::{Settings, ThemeChoice};
use crate::single_instance::{ControlCommand, Guard};
use crate::theme::{self, Palette};
use crate::tray::{TrayCommand, TrayService};

/// Initial and incremental message-page size.
pub const PAGE: usize = 60;
/// Minimum delay between phone history requests.
const PHONE_COOLDOWN: Duration = Duration::from_secs(6);
/// WhatsApp message-edit window.
pub const EDIT_WINDOW: Duration = Duration::from_secs(15 * 60);
/// WhatsApp revoke-for-everyone window.
pub const REVOKE_WINDOW: Duration = Duration::from_secs(2 * 24 * 60 * 60);

/// Pause after which a trackpad gesture selects a new axis.
const SCROLL_GESTURE_GAP: Duration = Duration::from_millis(150);
/// Linux trackpad scroll multiplier.
const TRACKPAD_SCALE: f32 = 1.8;
/// Trackpad glide decay, minimum start speed, and stop speed.
const GLIDE_DECAY: f32 = 0.35;
const GLIDE_START: f32 = 120.0;
const GLIDE_STOP: f32 = 40.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ScrollAxis {
    Horizontal,
    Vertical,
}
/// Delay after the last keystroke before clearing typing state.
const COMPOSING_TIMEOUT: Duration = Duration::from_secs(4);
/// Typing-state timeout when no stop event arrives.
const TYPING_TIMEOUT: Duration = Duration::from_secs(12);

/// Loaded chat history and paging state.
#[derive(Default)]
pub struct Conversation {
    pub messages: Vec<Message>,
    /// Whether the local archive has no earlier messages.
    pub complete: bool,
    pub loading_older: bool,
    /// Whether the initial page was requested.
    pub requested: bool,
    /// Whether a phone history request is active.
    pub fetching_phone: bool,
    /// Whether phone history is exhausted or unavailable.
    pub phone_exhausted: bool,
    /// Last phone response time for request throttling.
    pub phone_answered: Option<Instant>,
    /// Consecutive empty phone responses used for backoff.
    pub phone_misses: u32,
    /// Whether messages arrived after the latest phone request.
    pub phone_delivered: bool,
}

impl Conversation {
    fn merge(&mut self, incoming: Vec<Message>, older: bool) {
        if older {
            let known: HashSet<String> = self.messages.iter().map(|m| m.id.clone()).collect();
            let mut fresh: Vec<Message> = incoming
                .into_iter()
                .filter(|message| !known.contains(&message.id))
                .collect();
            fresh.append(&mut self.messages);
            self.messages = fresh;
        } else {
            for message in incoming {
                match self.messages.iter_mut().find(|m| m.id == message.id) {
                    Some(existing) => *existing = message,
                    None => self.messages.push(message),
                }
            }
        }
        self.messages.sort_by_key(|message| message.timestamp);
    }

    pub fn message_mut(&mut self, id: &str) -> Option<&mut Message> {
        self.messages.iter_mut().find(|message| message.id == id)
    }

    pub fn message(&self, id: &str) -> Option<&Message> {
        self.messages.iter().find(|message| message.id == id)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Presence {
    pub online: bool,
    pub last_seen: Option<i64>,
}

pub struct App {
    pub dirs: AppDirs,
    pub settings: Settings,
    settings_dirty: bool,
    last_settings_save: Instant,
    pub backend: Backend,
    pub palette: Palette,
    pub custom_themes: theme::custom::Catalog,
    applied_dark: Option<bool>,
    zoom_applied: bool,

    pub link: LinkStatus,
    /// Whether link-time history sync is active.
    pub syncing: bool,
    pub sync_percent: Option<u32>,
    pub me: Option<String>,
    pub me_name: Option<String>,
    /// Account about text.
    pub me_about: Option<String>,

    /// Chats ordered by latest activity.
    pub chats: Vec<Chat>,
    pub contacts: HashMap<String, Contact>,
    pub conversations: HashMap<ChatId, Conversation>,
    pub open_chat: Option<ChatId>,
    /// Chat row to reveal after keyboard navigation.
    pub scroll_chat_into_view: Option<ChatId>,
    /// Composer drafts by chat.
    pub drafts: HashMap<ChatId, String>,
    draft_mentions: HashMap<ChatId, Vec<ComposerMention>>,
    pub composer: String,
    composer_mentions: Vec<ComposerMention>,
    /// Byte offset of the `:` starting the active emoji query.
    pub emoji_start: Option<usize>,
    /// Keyboard-highlighted emoji in suggestions or the full picker.
    pub emoji_selected: usize,
    /// Byte offset of the `@` starting the active mention query.
    pub mention_start: Option<usize>,
    /// Keyboard-highlighted member in the mention suggestions.
    pub mention_selected: usize,
    /// Reply target in the open chat.
    pub reply_to: Option<String>,
    /// Outgoing message being edited.
    pub editing: Option<String>,
    composing: bool,
    last_keystroke: Option<Instant>,
    pub search: String,
    /// Message search results, newest first.
    pub search_hits: Vec<Message>,
    /// Active typers and their latest event time by chat.
    pub typing: HashMap<ChatId, Vec<(String, Instant)>>,
    pub presence: HashMap<String, Presence>,
    /// Whether account privacy disables direct-chat read receipts.
    pub account_receipts_off: bool,
    avatars: HashMap<String, Option<PathBuf>>,
    avatar_requests: HashSet<String>,
    /// Full-size profile pictures for info dialogs.
    avatars_full: HashMap<String, Option<PathBuf>>,
    avatar_full_requests: HashSet<String>,
    /// Whether files are being dragged over the window.
    pub dropping: bool,
    /// Open emoji, GIF, or sticker picker tab.
    pub picker: Option<PickerTab>,
    /// Picker anchor at the composer button.
    pub picker_anchor: Option<egui::Rect>,
    pub picker_search: String,
    /// Whether the newly opened picker should focus search.
    pub picker_focus: bool,
    /// Message the full emoji reaction picker is targeting.
    pub reaction_target: Option<(ChatId, String)>,
    /// Control that opened the reaction picker.
    pub reaction_anchor: Option<egui::Rect>,
    /// Demo/test: keep this message's context menu open.
    pub open_message_menu: Option<String>,
    /// Emoji-grid header to scroll into view.
    pub emoji_jump: Option<&'static str>,
    /// Attachments pending in the composer.
    pub pending: Vec<Pending>,
    /// In-chat audio player.
    pub player: Player,
    /// Active voice recorder.
    pub recording: Option<Recorder>,
    /// Voice messages with a sent played receipt.
    played_told: HashSet<String>,
    /// Message bodies registered for transcript copy formatting.
    pub copy_rows: std::sync::Arc<std::sync::Mutex<Vec<crate::transcript::Row>>>,
    /// Previous message-list rect used by the selection hook.
    pub selection_view: std::sync::Arc<std::sync::Mutex<Option<egui::Rect>>>,
    pub gif_query: String,
    pub gif_results: Vec<Gif>,
    /// Whether a GIF search is active.
    pub gif_pending: bool,
    pub gif_error: Option<GifError>,
    pub stickers: Vec<PathBuf>,
    /// Saved stickers, newest first.
    pub stickers_saved: Vec<PathBuf>,
    /// Imported sticker packs, newest first.
    pub sticker_packs: Vec<StickerPack>,
    /// Whether the sticker list is loading.
    pub stickers_pending: bool,
    /// Whether a sticker pack import is active.
    pub sticker_import_pending: bool,
    /// signal.art link in the sticker tab.
    pub sticker_link: String,
    scroll_lock: Option<(ScrollAxis, Instant)>,
    scroll_from_trackpad: bool,
    scroll_history: egui::util::History<egui::Vec2>,
    scroll_accum: egui::Vec2,
    glide: Option<egui::Vec2>,
    scroll_last_event: Option<Instant>,

    pub page: Page,
    pub dialog: Option<Dialog>,
    /// Chat filter in the forwarding destination dialog.
    pub forward_search: String,
    pub poll_draft: crate::model::PollDraft,
    pub poll_creating: bool,
    pub poll_voting: HashSet<(ChatId, String)>,
    /// Contact-name editor buffers.
    pub contact_edit: Option<(String, String)>,
    /// New-contact buffers and lookup state.
    pub new_contact_phone: String,
    pub new_contact_name: String,
    pub new_contact_last: String,
    pub new_contact_pending: bool,
    /// Phone number entered for pairing.
    pub pair_phone: String,
    pub sidebar_visible: bool,
    pub show_archived: bool,
    pub toasts: Vec<Toast>,
    pub actions: Vec<Action>,
    /// A newer release than this build, once GitHub has said so.
    pub update: Option<crate::updates::Release>,
    last_update_check: Option<Instant>,
    pub show_update: bool,
    pub update_download: crate::updates::DownloadState,
    pub update_support: Option<Result<crate::updates::install::Installation, String>>,
    update_inspecting: bool,
    pub update_arguments: Vec<String>,
    /// Whether to scroll the conversation to its newest message.
    pub scroll_to_bottom: bool,
    /// Whether the conversation was at the bottom last frame.
    pub at_bottom: bool,
    /// Message id to scroll into view.
    pub scroll_anchor: Option<String>,
    pub focus_composer: bool,
    pub focus_search: bool,
    pub quit_requested: bool,
    pub window_focused: bool,
    /// Cross-thread window repaint handle.
    waker: Waker,
    tray: Option<TrayService>,
    /// Whether the app is running without a window.
    pub window_hidden: bool,
    /// Whether window close should keep the process running.
    pub hide_intent: bool,
    /// Whether a headless app should create a window.
    pub wants_show: bool,
    /// Requests received from later launches.
    control_commands: Option<std::sync::Arc<std::sync::Mutex<Vec<ControlCommand>>>>,
    /// Chat ids from clicked notifications.
    notification_opens: std::sync::Arc<std::sync::Mutex<Vec<ChatId>>>,
    notifications: crate::notify::Notifications,
}

/// Attachment pending in the composer.
pub enum Pending {
    /// Clipboard image as straight-alpha RGBA and optional preview.
    Picture {
        width: usize,
        height: usize,
        rgba: std::sync::Arc<Vec<u8>>,
        texture: Option<egui::TextureHandle>,
    },
    File(PathBuf),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ComposerMention {
    id: String,
    name: String,
}

impl Pending {
    /// Whether the composer can preview the file as an image.
    pub fn is_picture_file(path: &std::path::Path) -> bool {
        mime_guess2::from_path(path)
            .first()
            .is_some_and(|mime| mime.type_() == "image")
    }
}

/// Process-level app services.
#[derive(Clone, Copy, Debug)]
pub struct AppOptions {
    /// Registers the system-tray item.
    pub tray: bool,
}

impl Default for AppOptions {
    fn default() -> Self {
        Self { tray: true }
    }
}

impl App {
    pub fn new(waker: &Waker, mut dirs: AppDirs, settings: Settings, options: AppOptions) -> Self {
        dirs.custom_media = settings.custom_media_dir.clone();
        let backend = Backend::spawn(dirs.clone(), waker.clone());
        let mut app = Self::with_backend(dirs, settings, backend, waker.clone());
        app.custom_themes.enable_desktop_themes();
        app.load_custom_themes();
        if options.tray {
            let waker = waker.clone();
            app.tray = TrayService::spawn(move || waker.wake());
        }
        app
    }

    /// Single-instance guard used by later launches.
    pub fn set_remote_control(&mut self, guard: &Guard) {
        self.control_commands = Some(guard.commands());
    }

    /// Creates a disconnected app and event sender for demos and tests.
    pub fn headless(
        mut dirs: AppDirs,
        settings: Settings,
    ) -> (Self, std::sync::mpsc::Sender<Event>) {
        dirs.custom_media = settings.custom_media_dir.clone();
        let (backend, events) = Backend::detached();
        (
            Self::with_backend(dirs, settings, backend, Waker::default()),
            events,
        )
    }

    fn with_backend(dirs: AppDirs, settings: Settings, backend: Backend, waker: Waker) -> Self {
        let palette = settings
            .cached_palette()
            .unwrap_or_else(|| match settings.theme {
                ThemeChoice::Light => Palette::light(),
                _ => Palette::dark(),
            });
        let open_chat = settings.last_chat.clone();
        Self {
            dirs,
            settings,
            settings_dirty: false,
            last_settings_save: Instant::now(),
            backend,
            palette,
            custom_themes: theme::custom::Catalog::default(),
            applied_dark: None,
            zoom_applied: false,
            link: LinkStatus::Starting,
            syncing: false,
            sync_percent: None,
            me: None,
            me_name: None,
            me_about: None,
            chats: Vec::new(),
            contacts: HashMap::new(),
            conversations: HashMap::new(),
            open_chat,
            scroll_chat_into_view: None,
            drafts: HashMap::new(),
            draft_mentions: HashMap::new(),
            composer: String::new(),
            composer_mentions: Vec::new(),
            emoji_start: None,
            emoji_selected: 0,
            mention_start: None,
            mention_selected: 0,
            reply_to: None,
            editing: None,
            composing: false,
            last_keystroke: None,
            search: String::new(),
            search_hits: Vec::new(),
            typing: HashMap::new(),
            presence: HashMap::new(),
            account_receipts_off: false,
            avatars: HashMap::new(),
            avatar_requests: HashSet::new(),
            avatars_full: HashMap::new(),
            avatar_full_requests: HashSet::new(),
            dropping: false,
            picker: None,
            picker_anchor: None,
            picker_search: String::new(),
            picker_focus: false,
            reaction_target: None,
            reaction_anchor: None,
            open_message_menu: None,
            emoji_jump: None,
            pending: Vec::new(),
            player: Player::new(waker.clone()),
            recording: None,
            played_told: HashSet::new(),
            copy_rows: Default::default(),
            selection_view: Default::default(),
            gif_query: String::new(),
            gif_results: Vec::new(),
            gif_pending: false,
            gif_error: None,
            stickers: Vec::new(),
            stickers_saved: Vec::new(),
            sticker_packs: Vec::new(),
            stickers_pending: false,
            sticker_import_pending: false,
            sticker_link: String::new(),
            scroll_lock: None,
            scroll_from_trackpad: false,
            scroll_history: egui::util::History::new(2..16, 0.1),
            scroll_accum: egui::Vec2::ZERO,
            glide: None,
            scroll_last_event: None,
            page: Page::Chats,
            dialog: None,
            forward_search: String::new(),
            poll_draft: Default::default(),
            poll_creating: false,
            poll_voting: HashSet::new(),
            contact_edit: None,
            new_contact_phone: String::new(),
            new_contact_name: String::new(),
            new_contact_last: String::new(),
            new_contact_pending: false,
            pair_phone: String::new(),
            sidebar_visible: true,
            show_archived: false,
            toasts: Vec::new(),
            actions: Vec::new(),
            update: None,
            last_update_check: None,
            show_update: false,
            update_download: Default::default(),
            update_support: None,
            update_inspecting: false,
            update_arguments: Vec::new(),
            scroll_to_bottom: true,
            at_bottom: true,
            scroll_anchor: None,
            focus_composer: false,
            focus_search: false,
            quit_requested: false,
            window_focused: false,
            waker,
            tray: None,
            window_hidden: false,
            hide_intent: false,
            wants_show: false,
            control_commands: None,
            notification_opens: Default::default(),
            notifications: Default::default(),
        }
    }

    /// Updates the linked app while no window exists.
    pub fn window_gone(&mut self) {
        self.window_hidden = true;
        self.window_focused = false;
        self.hide_intent = false;
        self.wants_show = false;
        if let Some(tray) = &mut self.tray {
            tray.hidden();
        }
    }

    /// Whether window close keeps the app in the tray.
    pub fn hides_to_tray(&self) -> bool {
        self.tray.is_some() && self.settings.keep_running_in_background
    }

    fn handle_tray(&mut self) {
        let Some(commands) = self.tray.as_ref().map(TrayService::drain_commands) else {
            return;
        };
        for command in commands {
            match command {
                TrayCommand::Show => self.actions.push(Action::ShowWindow),
                TrayCommand::ShowHide => self.actions.push(if self.window_hidden {
                    Action::ShowWindow
                } else {
                    Action::HideWindow
                }),
                TrayCommand::Quit => self.actions.push(Action::Quit),
            }
        }
    }

    fn handle_control_commands(&mut self) {
        let Some(queue) = &self.control_commands else {
            return;
        };
        let commands: Vec<ControlCommand> =
            std::mem::take(&mut *queue.lock().unwrap_or_else(|p| p.into_inner()));
        for command in commands {
            match command {
                ControlCommand::Show => self.actions.push(Action::ShowWindow),
                ControlCommand::ReloadThemes => self.actions.push(Action::ReloadThemes),
            }
        }
    }

    /// Opens chats from clicked notifications, creating a window when needed.
    fn handle_notification_opens(&mut self) {
        let opened: Vec<ChatId> = std::mem::take(
            &mut *self
                .notification_opens
                .lock()
                .unwrap_or_else(|p| p.into_inner()),
        );
        for chat in opened {
            self.actions.push(Action::OpenChat(chat));
            self.actions.push(Action::ShowWindow);
        }
    }

    /// Sends a desktop notification for an unseen incoming message.
    fn maybe_notify(&mut self, chat_id: &str, message: &Message) {
        if !self.settings.notifications {
            return;
        }
        let Some(chat) = self.chat(chat_id) else {
            return;
        };
        let now = crate::util::now();
        // Skip muted chats and delayed reconnect backlogs.
        if chat.unread == 0 || chat.muted(now) || now - message.timestamp > 60 {
            return;
        }
        let reading = !self.window_hidden
            && self.window_focused
            && self.page == Page::Chats
            && self.open_chat.as_deref() == Some(chat_id);
        if reading {
            return;
        }
        let (name, is_group) = (self.chat_title(chat), chat.is_group());
        let sender = self.display_name_or(&message.sender, message.sender_name.as_deref());
        let (title, body) =
            crate::notify::lines(&name, is_group, &sender, &self.message_text(message));
        // Prefer the chat picture, then the sender picture. Cached files work
        // before the chat list loads; new requests help later notifications.
        let sender = message.sender.clone();
        let picture = self
            .avatar(chat_id)
            .or_else(|| self.cached_avatar(chat_id))
            .or_else(|| self.avatar(&sender))
            .or_else(|| self.cached_avatar(&sender));
        let waker = self.waker.clone();
        self.notifications.show(
            title,
            body,
            picture,
            chat_id.to_owned(),
            std::sync::Arc::clone(&self.notification_opens),
            move || waker.wake(),
        );
    }

    /// Initializes a newly created window.
    pub fn attach(&mut self, ctx: &egui::Context) {
        // Register transcript copy formatting once per egui context.
        ctx.add_plugin(crate::transcript::CopyAnnotator {
            rows: std::sync::Arc::clone(&self.copy_rows),
        });
        ctx.data_mut(|data| {
            data.insert_temp(
                egui::Id::new("copy-rows"),
                std::sync::Arc::clone(&self.copy_rows),
            );
        });
        ctx.add_plugin(crate::ui::conversation::SelectionLeash::new(
            std::sync::Arc::clone(&self.selection_view),
        ));
        crate::theme::install(ctx);
        // Use a faster wheel speed for short chat rows.
        ctx.options_mut(|options| options.input_options.line_scroll_speed = 120.0);
        // Load and index the color emoji font outside the frame loop.
        std::thread::Builder::new()
            .name("emoji-font".into())
            .spawn(crate::emoji::warm_up)
            .ok();
        self.applied_dark = None;
        self.zoom_applied = false;
        self.window_hidden = false;
        self.hide_intent = false;
        self.wants_show = false;
        self.refocus_composer(ctx);
        if let Some(tray) = &mut self.tray {
            tray.attach();
        }
        #[cfg(target_os = "macos")]
        crate::macos::attach(ctx);
    }

    pub fn is_connected(&self) -> bool {
        self.link.is_connected()
    }

    /// Whether the device has linked data, including while offline.
    pub fn is_linked(&self) -> bool {
        matches!(
            self.link,
            LinkStatus::Connected | LinkStatus::Connecting | LinkStatus::Disconnected { .. }
        ) || (!self.chats.is_empty() && !matches!(self.link, LinkStatus::LoggedOut))
    }

    pub fn chat(&self, id: &str) -> Option<&Chat> {
        self.chats.iter().find(|chat| chat.id == id)
    }

    pub fn chat_mut(&mut self, id: &str) -> Option<&mut Chat> {
        self.chats.iter_mut().find(|chat| chat.id == id)
    }

    pub fn current_chat(&self) -> Option<&Chat> {
        self.open_chat.as_deref().and_then(|id| self.chat(id))
    }

    /// Resolves an address-book, push, phone-number, or fallback name.
    pub fn display_name(&self, id: &str) -> String {
        self.display_name_or(id, None)
    }

    /// Resolves a consistent display name using settings and an optional
    /// message-provided fallback. Our own id becomes "You".
    pub fn display_name_or(&self, id: &str, hint: Option<&str>) -> String {
        if self.me.as_deref() == Some(id) {
            return "You".to_owned();
        }
        self.person_name(id, hint)
    }

    /// Resolves a mention name without replacing our own name with "You".
    pub fn mention_name(&self, id: &str) -> String {
        if self.me.as_deref() == Some(id) {
            return self
                .me_name
                .clone()
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| "You".to_owned());
        }
        self.person_name(id, None)
    }

    /// Resolves the chat-list title.
    pub fn chat_title(&self, chat: &Chat) -> String {
        if chat.is_group() || self.me.as_deref() == Some(chat.id.as_str()) {
            return chat.name.clone();
        }
        self.person_name(&chat.id, None)
    }

    fn person_name(&self, id: &str, hint: Option<&str>) -> String {
        let contact = self.contacts.get(id);
        let present = |name: Option<&str>| name.filter(|name| !name.is_empty()).map(str::to_owned);
        let saved = present(contact.and_then(|contact| contact.full_name.as_deref()));
        let called = present(contact.and_then(|contact| contact.push_name.as_deref()))
            .or_else(|| present(hint));
        let (first, second) = if self.settings.names_from_contacts {
            (saved, called.map(|name| format!("~{name}")))
        } else {
            (called, saved)
        };
        if let Some(name) = first.or(second) {
            return name;
        }
        if let Some(chat) = self.chat(id)
            && !chat.name.is_empty()
            && !chat.name.chars().all(|c| c.is_ascii_digit())
        {
            return chat.name.clone();
        }
        match crate::model::phone_of(id) {
            Some(digits) => crate::util::phone(digits),
            None => "Unknown".to_owned(),
        }
    }

    /// Resolves message mentions for markup.
    pub fn mention_list(&self, message: &Message) -> Vec<crate::markup::Mention> {
        message
            .mentions
            .iter()
            .map(|mention| crate::markup::Mention {
                user: mention.user.clone(),
                name: self.mention_name(&mention.id),
            })
            .collect()
    }

    /// Resolves `@user` tokens in previews without mention metadata.
    pub fn resolve_mention_tokens(&self, text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        let mut rest = text;
        while let Some(at) = rest.find('@') {
            out.push_str(&rest[..at]);
            out.push('@');
            let after = &rest[at + 1..];
            let digits = after
                .char_indices()
                .find(|(_, c)| !c.is_ascii_digit())
                .map_or(after.len(), |(index, _)| index);
            let id = format!("{}@s.whatsapp.net", &after[..digits]);
            let known = digits >= 5
                && (self.me.as_deref() == Some(id.as_str())
                    || self.contacts.contains_key(&id)
                    || self.chat(&id).is_some());
            if known {
                out.push_str(&self.mention_name(&id));
                rest = &after[digits..];
            } else {
                rest = after;
            }
        }
        out.push_str(rest);
        out
    }

    /// One-line plain-text message summary with resolved mentions.
    pub fn message_text(&self, message: &Message) -> String {
        match &message.content {
            Content::Text { text, .. } => crate::markup::plain(text, &self.mention_list(message)),
            _ => self.resolve_mention_tokens(&message.summary()),
        }
    }

    /// Whether a direct chat uses a saved address-book name.
    pub fn is_saved_contact(&self, id: &str) -> bool {
        self.contacts.get(id).is_some_and(|contact| {
            contact
                .full_name
                .as_deref()
                .is_some_and(|name| !name.is_empty())
        })
    }

    /// Group members sorted by name, then phone number, with our id last.
    pub fn participant_list(&self, chat: &Chat) -> Vec<(String, String)> {
        let me = self.me.as_deref();
        let mut named = Vec::new();
        let mut numbers = Vec::new();
        for id in chat
            .participants
            .iter()
            .filter(|id| Some(id.as_str()) != me)
        {
            let name = self.display_name(id);
            if name.starts_with('+') || name == "Unknown" {
                numbers.push((id.clone(), name));
            } else {
                named.push((id.clone(), name));
            }
        }
        named.sort_by_key(|(_, name)| name.trim_start_matches('~').to_lowercase());
        numbers.sort_by(|a, b| a.1.cmp(&b.1));
        named.extend(numbers);
        if let Some(me) = me
            && chat.participants.iter().any(|id| id == me)
        {
            named.push((me.to_owned(), "You".to_owned()));
        }
        named
    }

    /// Group members matching the active composer mention query.
    pub fn mention_candidates(&self, chat: &Chat, query: &str) -> Vec<(String, String)> {
        if !chat.is_group() {
            return Vec::new();
        }
        let needle = query.trim().to_lowercase();
        let digits: String = query.chars().filter(char::is_ascii_digit).collect();
        self.participant_list(chat)
            .into_iter()
            .filter(|(id, name)| {
                if self.me.as_deref() == Some(id) {
                    return false;
                }
                if needle.is_empty() {
                    return true;
                }
                name.trim_start_matches('~')
                    .to_lowercase()
                    .contains(&needle)
                    || (!digits.is_empty()
                        && id
                            .split('@')
                            .next()
                            .is_some_and(|user| user.contains(&digits)))
            })
            .collect()
    }

    pub fn participant_names(&self, chat: &Chat) -> String {
        let me = self.me.as_deref();
        let mut names = Vec::new();
        let mut numbers = Vec::new();
        for id in chat
            .participants
            .iter()
            .filter(|id| Some(id.as_str()) != me)
        {
            let name = self.display_name(id);
            if name.starts_with('+') || name == "Unknown" {
                numbers.push(name);
            } else {
                let name = name.trim_start_matches('~');
                names.push(name.split_whitespace().next().unwrap_or(name).to_owned());
            }
        }
        names.sort_by_key(|name| name.to_lowercase());
        names.dedup();
        numbers.sort();
        numbers.dedup();
        names.extend(numbers);
        if chat.participants.iter().any(|id| Some(id.as_str()) == me) {
            names.push("You".to_owned());
        }
        names.join(", ")
    }

    /// Visible chats filtered by search and archive state, with pinned first.
    pub fn visible_chats(&self) -> Vec<&Chat> {
        let needle = crate::util::search_key(self.search.trim());
        let mut chats: Vec<&Chat> = self
            .chats
            .iter()
            .filter(|chat| chat.archived == self.show_archived || !needle.is_empty())
            .filter(|chat| {
                needle.is_empty()
                    || crate::util::search_key(&chat.name).contains(&needle)
                    || chat.phone().is_some_and(|phone| phone.contains(&needle))
                    || chat.last.as_ref().is_some_and(|last| {
                        crate::util::search_key(&last.summary).contains(&needle)
                    })
            })
            .collect();
        chats.sort_by(|a, b| {
            b.pinned.cmp(&a.pinned).then_with(|| {
                if a.pinned && b.pinned {
                    b.pinned_at.cmp(&a.pinned_at).then(a.id.cmp(&b.id))
                } else {
                    b.last_activity.cmp(&a.last_activity).then(a.id.cmp(&b.id))
                }
            })
        });
        chats
    }

    /// Matching individual contacts without an existing chat, sorted by name.
    pub fn matching_contacts(&self) -> Vec<&Contact> {
        let needle = crate::util::search_key(self.search.trim());
        if needle.is_empty() {
            return Vec::new();
        }
        let mut contacts: Vec<&Contact> = self
            .contacts
            .values()
            .filter(|contact| crate::model::phone_of(&contact.id).is_some())
            .filter(|contact| self.me.as_deref() != Some(contact.id.as_str()))
            .filter(|contact| !self.chats.iter().any(|chat| chat.id == contact.id))
            .filter(|contact| {
                contact
                    .display_name()
                    .is_some_and(|name| crate::util::search_key(name).contains(&needle))
                    || contact
                        .id
                        .split('@')
                        .next()
                        .is_some_and(|phone| phone.contains(&needle))
            })
            .collect();
        contacts
            .sort_by_key(|contact| contact.display_name().unwrap_or(&contact.id).to_lowercase());
        contacts.truncate(15);
        contacts
    }

    pub fn archived_count(&self) -> usize {
        self.chats.iter().filter(|chat| chat.archived).count()
    }

    pub fn unread_total(&self) -> u32 {
        self.chats
            .iter()
            .filter(|chat| !chat.archived && !chat.muted(crate::util::now()))
            .map(|chat| chat.unread)
            .sum()
    }

    /// Returns or requests a cached profile picture.
    fn cached_avatar(&self, id: &str) -> Option<PathBuf> {
        let path = self.dirs.avatar_file(id, false);
        path.metadata()
            .ok()
            .filter(|metadata| metadata.len() > 0)
            .map(|_| path)
    }

    /// Registers an existing profile picture, used by demo data.
    pub fn adopt_avatar(&mut self, id: &str, path: PathBuf) {
        self.avatars.insert(id.to_owned(), Some(path));
    }

    pub fn avatar(&mut self, id: &str) -> Option<PathBuf> {
        if let Some(known) = self.avatars.get(id) {
            return known.clone();
        }
        if self.avatar_requests.insert(id.to_owned()) {
            self.backend.send(Command::FetchAvatar {
                id: id.to_owned(),
                full: false,
            });
        }
        None
    }

    /// Returns or requests a full-size profile picture.
    pub fn avatar_full(&mut self, id: &str) -> Option<PathBuf> {
        if let Some(known) = self.avatars_full.get(id) {
            return known.clone();
        }
        if self.avatar_full_requests.insert(id.to_owned()) {
            self.backend.send(Command::FetchAvatar {
                id: id.to_owned(),
                full: true,
            });
        }
        None
    }

    /// Whether an outgoing message is still editable.
    pub fn can_edit(&self, message: &Message) -> bool {
        message.from_me
            && matches!(message.content, Content::Text { .. })
            && crate::util::now() - message.timestamp <= EDIT_WINDOW.as_secs() as i64
    }

    /// Whether an outgoing message can still be revoked for everyone.
    pub fn can_revoke(&self, message: &Message) -> bool {
        message.from_me
            && !matches!(message.content, Content::Revoked)
            && crate::util::now() - message.timestamp <= REVOKE_WINDOW.as_secs() as i64
    }

    /// Active typers in a chat as id and display name.
    pub fn typing_in(&self, chat: &str) -> Vec<(String, String)> {
        self.typing
            .get(chat)
            .map(|typers| {
                typers
                    .iter()
                    .map(|(sender, _)| (sender.clone(), self.display_name(sender)))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn handle_events(&mut self) {
        for event in self.backend.poll() {
            match event {
                Event::Link(status) => self.handle_link(status),
                Event::Me { id, name, about } => {
                    self.me = Some(id);
                    self.me_name = name;
                    self.me_about = about;
                }
                Event::Chats(chats) => {
                    for chat in &chats {
                        if chat.unread == 0 {
                            self.notifications.clear(&chat.id);
                        }
                    }
                    self.chats = chats;
                    if let Some(open) = self.open_chat.clone() {
                        if self.chat(&open).is_none() {
                            self.open_chat = None;
                        } else {
                            // Show archived messages immediately, including offline.
                            self.ensure_loaded(&open);
                        }
                    }
                }
                Event::ChatUpdated(chat) => self.handle_chat_updated(*chat),
                Event::Messages {
                    chat,
                    messages,
                    older,
                    complete,
                } => {
                    let conversation = self.conversations.entry(chat.clone()).or_default();
                    let was_empty = conversation.messages.is_empty();
                    if older && !messages.is_empty() {
                        conversation.phone_delivered = true;
                    }
                    conversation.merge(messages, older);
                    if older {
                        conversation.loading_older = false;
                        conversation.complete = complete;
                    } else if was_empty {
                        conversation.complete = complete;
                    }
                    // Request phone history when sync created a chat without messages.
                    let bare = !older && complete && conversation.messages.is_empty();
                    if self.open_chat.as_deref() == Some(chat.as_str()) {
                        if !older && (self.at_bottom || was_empty) {
                            self.scroll_to_bottom = true;
                        }
                        if bare {
                            self.fetch_older(&chat);
                        }
                        // After the first page, load toward a pending search anchor once.
                        if !older
                            && let Some(anchor) = self.scroll_anchor.clone()
                            && let Some(conversation) = self.conversations.get_mut(&chat)
                            && conversation.message(&anchor).is_none()
                            && !conversation.loading_older
                            && let Some(oldest) = conversation.messages.first()
                        {
                            conversation.loading_older = true;
                            self.backend.send(Command::LoadUntil {
                                chat,
                                id: anchor,
                                before: (oldest.timestamp, oldest.id.clone()),
                            });
                        }
                    }
                }
                Event::SearchHits { query, messages } => {
                    if query == self.search.trim() {
                        self.search_hits = messages;
                    }
                }
                Event::Incoming { chat, message } => self.maybe_notify(&chat, &message),
                Event::Picked { chat, paths } => {
                    if self.open_chat.as_deref() == Some(chat.as_str()) {
                        self.stage_files(paths);
                    }
                }
                Event::PollCreated { chat, error } => {
                    self.poll_creating = false;
                    if let Some(error) = error {
                        self.toast_error(error);
                    } else if self.dialog == Some(Dialog::CreatePoll(chat)) {
                        self.dialog = None;
                        self.poll_draft = Default::default();
                    }
                }
                Event::PollVoted {
                    chat,
                    message,
                    error,
                } => {
                    self.poll_voting.remove(&(chat, message));
                    if let Some(error) = error {
                        self.toast_error(error);
                    }
                }
                Event::MessageUpdated(message) => {
                    let message = *message;
                    if let Some(conversation) = self.conversations.get_mut(&message.chat)
                        && let Some(existing) = conversation.message_mut(&message.id)
                    {
                        let state = existing.content.media().map(|media| media.state.clone());
                        *existing = message;
                        if let (Some(state), Some(media)) = (state, existing.content.media_mut()) {
                            media.state = state;
                        }
                    }
                }
                Event::Contacts(contacts) => {
                    for contact in contacts {
                        self.contacts.insert(contact.id.clone(), contact);
                    }
                }
                Event::Typing {
                    chat,
                    sender,
                    composing,
                } => {
                    let typers = self.typing.entry(chat).or_default();
                    typers.retain(|(who, _)| *who != sender);
                    if composing {
                        typers.push((sender, Instant::now()));
                    }
                }
                Event::Presence {
                    id,
                    online,
                    last_seen,
                } => {
                    self.presence.insert(id, Presence { online, last_seen });
                }
                Event::Avatar { id, full, path } => {
                    if full {
                        self.avatar_full_requests.remove(&id);
                        self.avatars_full.insert(id, path);
                    } else {
                        self.avatar_requests.remove(&id);
                        self.avatars.insert(id, path);
                    }
                }
                Event::Gifs { query, results } => {
                    if query == self.gif_query {
                        self.gif_pending = false;
                        match results {
                            Ok(results) => {
                                self.gif_results = results;
                                self.gif_error = None;
                            }
                            Err(error) => {
                                self.gif_results.clear();
                                self.gif_error = Some(error);
                            }
                        }
                    }
                }
                Event::Stickers {
                    saved,
                    packs,
                    recent,
                } => {
                    self.stickers_saved = saved;
                    self.sticker_packs = packs;
                    self.stickers = recent;
                    self.stickers_pending = false;
                    self.sticker_import_pending = false;
                }
                Event::MessageDeleted { chat, id } => {
                    if let Some(conversation) = self.conversations.get_mut(&chat) {
                        conversation.messages.retain(|message| message.id != id);
                    }
                    if self.editing.as_deref() == Some(id.as_str()) {
                        self.editing = None;
                        self.composer.clear();
                    }
                }
                Event::Media {
                    chat,
                    message,
                    result,
                } => self.handle_media(&chat, &message, result),
                Event::Syncing(syncing) => {
                    if self.syncing && !syncing {
                        self.toast("History loaded");
                    }
                    self.syncing = syncing;
                    if !syncing {
                        self.sync_percent = None;
                    }
                }
                Event::SyncProgress(percent) => self.sync_percent = Some(percent),
                Event::OlderFetched { chat, more } => {
                    let conversation = self.conversations.entry(chat).or_default();
                    conversation.fetching_phone = false;
                    conversation.phone_exhausted = !more;
                    conversation.phone_answered = Some(Instant::now());
                    if conversation.phone_delivered {
                        conversation.phone_misses = 0;
                    } else {
                        conversation.phone_misses = (conversation.phone_misses + 1).min(7);
                    }
                    conversation.phone_delivered = false;
                    // Page the archive again after phone history arrives.
                    conversation.complete = false;
                }
                Event::ReceiptsPrivacy { disabled } => self.account_receipts_off = disabled,
                Event::ContactReady { id, name } => {
                    self.new_contact_pending = false;
                    if self.dialog == Some(Dialog::NewContact) {
                        self.dialog = None;
                    }
                    let name = name
                        .filter(|name| !name.is_empty())
                        .unwrap_or_else(|| crate::util::phone(&id));
                    self.actions.push(Action::StartChat { id, name });
                }
                Event::Info(message) => self.toast(message),
                Event::UpdateAvailable { version, url } => {
                    let notice = crate::updates::Release { version, url };
                    if self.update.as_ref() != Some(&notice) {
                        self.toast(format!("ZapFast {} is available", notice.version));
                    }
                    self.update = Some(notice);
                }
                Event::UpdateSupport(result) => {
                    self.update_support = Some(result);
                    self.update_inspecting = false;
                    self.maybe_download_update();
                }
                Event::UpdateProgress { received, total } => {
                    self.update_download =
                        crate::updates::DownloadState::Downloading { received, total };
                }
                Event::UpdateDownloaded(result) => {
                    self.update_download = match result {
                        Ok(prepared) => crate::updates::DownloadState::Ready(prepared),
                        Err(error) => crate::updates::DownloadState::Failed(error),
                    };
                }
                Event::UpdateInstalling(result) => match result {
                    Ok(()) => self.actions.push(Action::Quit),
                    Err(error) => {
                        self.update_download = crate::updates::DownloadState::Failed(error)
                    }
                },
                Event::MediaDirChanged(custom) => {
                    self.dirs.custom_media = custom.clone();
                    self.settings.custom_media_dir = custom;
                    self.mark_settings_dirty();
                    let dir = self.dirs.media_dir();
                    for conversation in self.conversations.values_mut() {
                        for message in &mut conversation.messages {
                            let Some(media) = message.content.media_mut() else {
                                continue;
                            };
                            let Some(path) = &media.path else {
                                continue;
                            };
                            let Some(name) = path.file_name() else {
                                continue;
                            };
                            if !path.exists() {
                                let candidate = dir.join(name);
                                if candidate.exists() {
                                    media.path = Some(candidate);
                                }
                            }
                        }
                    }
                    self.toast(if self.dirs.custom_media.is_some() {
                        "Attachment folder updated"
                    } else {
                        "Attachment folder reset to default"
                    });
                }
                Event::Error(message) => {
                    self.sticker_import_pending = false;
                    self.new_contact_pending = false;
                    self.toast_error(message);
                }
            }
        }
    }

    fn handle_link(&mut self, status: LinkStatus) {
        match &status {
            LinkStatus::Connected => {
                for conversation in self.conversations.values_mut() {
                    for message in &mut conversation.messages {
                        if let Content::Poll { state, .. } = &mut message.content {
                            state.refresh_needed = true;
                            state.refreshing = false;
                        }
                    }
                }
                if matches!(self.link, LinkStatus::Disconnected { .. }) {
                    self.toast("Back online");
                }
                self.dialog = match self.dialog.take() {
                    Some(Dialog::PairWithPhone) => None,
                    other => other,
                };
                if let Some(open) = self.open_chat.clone() {
                    self.ensure_loaded(&open);
                }
            }
            LinkStatus::LoggedOut => {
                self.poll_voting.clear();
                self.poll_creating = false;
                self.poll_draft = Default::default();
                self.notifications.clear_all();
                self.chats.clear();
                self.conversations.clear();
                self.contacts.clear();
                self.avatars.clear();
                self.open_chat = None;
                self.toast_error("This device was unlinked from your phone");
            }
            LinkStatus::Failed(message) => self.toast_error(message.clone()),
            _ => {}
        }
        self.link = status;
    }

    fn handle_chat_updated(&mut self, chat: Chat) {
        let is_open =
            self.open_chat.as_deref() == Some(chat.id.as_str()) && self.page == Page::Chats;
        let mut chat = chat;
        if chat.unread == 0 {
            self.notifications.clear(&chat.id);
        }
        if is_open && chat.unread > 0 && self.window_focused && !self.window_hidden {
            chat.unread = 0;
            self.mark_read(&chat.id);
        }
        match self.chats.iter_mut().find(|known| known.id == chat.id) {
            Some(existing) => *existing = chat,
            None => self.chats.push(chat),
        }
        self.chats
            .sort_by_key(|chat| std::cmp::Reverse(chat.last_activity));
    }

    fn handle_media(&mut self, chat: &str, id: &str, result: Result<PathBuf, String>) {
        let Some(message) = self
            .conversations
            .get_mut(chat)
            .and_then(|conversation| conversation.message_mut(id))
        else {
            return;
        };
        let Some(media) = message.content.media_mut() else {
            return;
        };
        match result {
            Ok(path) => {
                media.path = Some(path);
                media.state = MediaState::Idle;
            }
            Err(error) => {
                // Show expired-file failures in the bubble, not as a toast.
                let notice = if error.contains("403") || error.contains("404") {
                    "No longer available on WhatsApp's servers".to_owned()
                } else {
                    error
                };
                log::warn!("download failed: {notice}");
                media.state = MediaState::Failed(notice);
            }
        }
    }

    fn ensure_loaded(&mut self, chat: &str) {
        let conversation = self.conversations.entry(chat.to_owned()).or_default();
        if !conversation.requested {
            conversation.requested = true;
            self.backend.send(Command::LoadChat {
                chat: chat.to_owned(),
                before: None,
            });
        }
    }

    pub fn load_older(&mut self, chat: &str) {
        let Some(conversation) = self.conversations.get_mut(chat) else {
            return;
        };
        if conversation.loading_older {
            return;
        }
        let Some(oldest) = conversation.messages.first() else {
            return;
        };
        if conversation.complete {
            self.fetch_older(chat);
            return;
        }
        conversation.loading_older = true;
        let before = (oldest.timestamp, oldest.id.clone());
        self.scroll_anchor = Some(oldest.id.clone());
        self.backend.send(Command::LoadChat {
            chat: chat.to_owned(),
            before: Some(before),
        });
    }

    /// Requests older phone history when available and outside the cooldown.
    pub fn fetch_older(&mut self, chat: &str) {
        let Some(conversation) = self.conversations.get_mut(chat) else {
            return;
        };
        if conversation.fetching_phone || conversation.phone_exhausted {
            return;
        }
        // Back off after empty responses. Only a connected phone can answer.
        if !matches!(self.link, LinkStatus::Connected) {
            return;
        }
        let cooldown =
            (PHONE_COOLDOWN * 2u32.pow(conversation.phone_misses)).min(Duration::from_secs(600));
        if conversation
            .phone_answered
            .is_some_and(|answered| answered.elapsed() < cooldown)
        {
            return;
        }
        conversation.fetching_phone = true;
        self.scroll_anchor = conversation
            .messages
            .first()
            .map(|oldest| oldest.id.clone());
        self.backend.send(Command::FetchOlder(chat.to_owned()));
    }

    fn mark_read(&mut self, chat: &str) {
        self.notifications.clear(chat);
        if let Some(known) = self.chat_mut(chat) {
            known.unread = 0;
        }
        // Clear local unread state regardless of receipt settings.
        self.backend.send(Command::MarkRead {
            chat: chat.to_owned(),
            receipts: self.settings.send_read_receipts,
        });
    }

    fn open_chat(&mut self, id: ChatId) {
        if self.open_chat.as_deref() != Some(id.as_str()) {
            self.reaction_target = None;
            self.reaction_anchor = None;
            self.emoji_jump = None;
            if let Some(previous) = self.open_chat.take() {
                let draft = std::mem::take(&mut self.composer);
                // Discard an unfinished edit instead of keeping it as a draft.
                if self.editing.take().is_some() || draft.trim().is_empty() {
                    self.drafts.remove(&previous);
                    self.draft_mentions.remove(&previous);
                    self.composer_mentions.clear();
                } else {
                    self.drafts.insert(previous.clone(), draft);
                    self.draft_mentions.insert(
                        previous.clone(),
                        std::mem::take(&mut self.composer_mentions),
                    );
                }
                self.stop_composing(&previous);
            }
            self.composer = self.drafts.remove(&id).unwrap_or_default();
            self.composer_mentions = self.draft_mentions.remove(&id).unwrap_or_default();
            self.reply_to = None;
            self.editing = None;
        }
        self.emoji_start = None;
        self.mention_start = None;
        self.open_chat = Some(id.clone());
        self.page = Page::Chats;
        self.scroll_to_bottom = true;
        self.at_bottom = true;
        self.focus_composer = true;
        self.ensure_loaded(&id);
        if self
            .conversations
            .get(&id)
            .is_some_and(|conversation| conversation.complete && conversation.messages.is_empty())
        {
            self.fetch_older(&id);
        }
        if self.chat(&id).is_some_and(|chat| chat.unread > 0) {
            self.mark_read(&id);
        }
        if self.settings.last_chat.as_deref() != Some(id.as_str()) {
            self.settings.last_chat = Some(id);
            self.mark_settings_dirty();
        }
    }

    /// Returns keyboard focus to the open conversation when no search or
    /// overlay is active.
    fn refocus_composer(&mut self, ctx: &egui::Context) {
        let search_focused = ctx.memory(|memory| memory.has_focus(egui::Id::new("chat-search")));
        if self.page == Page::Chats
            && self.dialog.is_none()
            && self.picker.is_none()
            && self.recording.is_none()
            && self.open_chat.is_some()
            && self.search.trim().is_empty()
            && !self.focus_search
            && !search_focused
        {
            self.focus_composer = true;
        }
    }

    /// Updates typing state after composer changes.
    pub fn note_keystroke(&mut self) {
        self.last_keystroke = Some(Instant::now());
        if !self.composing
            && self.settings.send_typing
            && let Some(chat) = self.open_chat.clone()
        {
            self.composing = true;
            self.backend.send(Command::Composing {
                chat,
                composing: true,
            });
        }
    }

    fn stop_composing(&mut self, chat: &str) {
        if self.composing {
            self.composing = false;
            self.backend.send(Command::Composing {
                chat: chat.to_owned(),
                composing: false,
            });
        }
        self.last_keystroke = None;
    }

    fn send_text(&mut self, chat: ChatId, text: String, quoting: Option<String>) {
        let text = text.trim().to_owned();
        if text.is_empty() {
            return;
        }
        let (text, mentions) = self.encode_composer_mentions(&chat, text);
        self.emoji_start = None;
        self.mention_start = None;
        self.stop_composing(&chat);
        if let Some(id) = self.editing.take() {
            if let Some(message) = self
                .conversations
                .get_mut(&chat)
                .and_then(|conversation| conversation.message_mut(&id))
            {
                message.content = Content::text(text.clone());
                message.edited = true;
                message.mentions = mention_refs(&mentions);
            }
            self.backend.send(Command::EditText {
                chat,
                id,
                text,
                mentions,
            });
            return;
        }
        self.backend.send(Command::SendText {
            chat,
            text,
            quoting,
            mentions,
        });
        self.scroll_to_bottom = true;
        self.at_bottom = true;
    }

    /// Replaces selected display-name mentions with WhatsApp's `@user`
    /// tokens and returns the JIDs for message context.
    fn encode_composer_mentions(&mut self, chat: &str, mut text: String) -> (String, Vec<String>) {
        let participants = self
            .chat(chat)
            .map(|chat| chat.participants.clone())
            .unwrap_or_default();
        let selected = std::mem::take(&mut self.composer_mentions);
        let mut mentions = Vec::new();
        for mention in selected {
            if !participants.iter().any(|id| id == &mention.id) {
                continue;
            }
            let Some(user) = mention.id.split('@').next().filter(|user| !user.is_empty()) else {
                continue;
            };
            let shown = format!("@{}", mention.name);
            if let Some(at) = find_named_mention(&text, &shown) {
                text.replace_range(at..at + shown.len(), &format!("@{user}"));
                if !mentions.iter().any(|id| id == &mention.id) {
                    mentions.push(mention.id);
                }
            }
        }
        // Preserve mentions in an edited draft that already contains wire
        // tokens, even when it did not originate in this composer session.
        for id in participants {
            let Some(user) = id.split('@').next().filter(|user| !user.is_empty()) else {
                continue;
            };
            if contains_mention_token(&text, user) && !mentions.iter().any(|known| known == &id) {
                mentions.push(id);
            }
        }
        (text, mentions)
    }

    /// Adds files to the open chat's composer.
    fn stage_files(&mut self, paths: Vec<PathBuf>) {
        if self.open_chat.is_none() {
            self.toast_error("Open a chat first");
            return;
        }
        for path in paths {
            self.pending.push(Pending::File(path));
        }
        self.focus_composer = true;
    }

    /// Sends pending files, attaching the caption to the first.
    fn send_pending(&mut self, chat: ChatId, caption: String) {
        let caption = caption.trim().to_owned();
        let (caption, mentions) = self.encode_composer_mentions(&chat, caption);
        let caption = Some(caption).filter(|text| !text.is_empty());
        let mut caption = caption;
        let mut mentions = mentions;
        self.emoji_start = None;
        self.mention_start = None;
        let mut files = Vec::new();
        for item in std::mem::take(&mut self.pending) {
            match item {
                Pending::Picture {
                    width,
                    height,
                    rgba,
                    ..
                } => {
                    self.backend.send(Command::SendImage {
                        chat: chat.clone(),
                        width: width as u32,
                        height: height as u32,
                        rgba: std::sync::Arc::try_unwrap(rgba).unwrap_or_else(|arc| (*arc).clone()),
                        caption: caption.take(),
                        mentions: std::mem::take(&mut mentions),
                    });
                }
                Pending::File(path) => files.push(path),
            }
        }
        if !files.is_empty() {
            self.backend.send(Command::SendFiles {
                chat,
                paths: files,
                caption: caption.take(),
                mentions,
            });
        }
        self.reply_to = None;
        self.scroll_to_bottom = true;
        self.at_bottom = true;
    }

    #[allow(dead_code)]
    fn send_files(&mut self, paths: Vec<PathBuf>) {
        let Some(chat) = self.open_chat.clone() else {
            self.toast_error("Open a chat first");
            return;
        };
        if paths.is_empty() {
            return;
        }
        self.toast(format!(
            "Sending {} file{}…",
            paths.len(),
            if paths.len() == 1 { "" } else { "s" }
        ));
        self.backend.send(Command::SendFiles {
            chat,
            paths,
            caption: None,
            mentions: Vec::new(),
        });
        self.scroll_to_bottom = true;
        self.at_bottom = true;
    }

    fn tick(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        if self.composing
            && let Some(last) = self.last_keystroke
            && now.duration_since(last) > COMPOSING_TIMEOUT
            && let Some(chat) = self.open_chat.clone()
        {
            self.stop_composing(&chat);
        }
        for typers in self.typing.values_mut() {
            typers.retain(|(_, since)| now.duration_since(*since) < TYPING_TIMEOUT);
        }
        self.typing.retain(|_, typers| !typers.is_empty());
        self.toasts
            .retain(|toast| toast.created.elapsed() < Duration::from_millis(3200));
        if self.settings.check_for_updates
            && !self.backend.is_offline()
            && self
                .last_update_check
                .is_none_or(|at| at.elapsed() >= crate::updates::CHECK_INTERVAL)
        {
            self.last_update_check = Some(now);
            self.backend.send(Command::CheckForUpdates);
        }
        self.maybe_download_update();
        if self.settings_dirty && self.last_settings_save.elapsed() > Duration::from_secs(2) {
            self.save_settings();
        }
        if !self.typing.is_empty() || self.composing {
            ctx.request_repaint_after(Duration::from_secs(1));
        }
    }

    fn inspect_update(&mut self) {
        if self.update_support.is_none() && !self.update_inspecting {
            self.update_inspecting = true;
            self.backend.send(Command::InspectUpdate);
        }
    }

    fn maybe_download_update(&mut self) {
        if !self.settings.check_for_updates
            || !self.settings.download_updates_automatically
            || self.update.is_none()
            || !matches!(self.update_download, crate::updates::DownloadState::Idle)
        {
            return;
        }
        self.inspect_update();
        if matches!(self.update_support, Some(Ok(_))) {
            self.download_update();
        }
    }

    fn download_update(&mut self) {
        if !matches!(
            self.update_download,
            crate::updates::DownloadState::Idle | crate::updates::DownloadState::Failed(_)
        ) || !matches!(self.update_support, Some(Ok(_)))
        {
            return;
        }
        if let Some(release) = self.update.clone() {
            self.update_download = crate::updates::DownloadState::Downloading {
                received: 0,
                total: 0,
            };
            self.backend.send(Command::DownloadUpdate {
                release,
                source: crate::updates::Source::GitHub,
            });
        }
    }

    pub fn mark_settings_dirty(&mut self) {
        self.settings_dirty = true;
    }

    fn save_settings(&mut self) {
        self.settings_dirty = false;
        self.last_settings_save = Instant::now();
        if let Err(error) = self.settings.save(&self.dirs.settings_file()) {
            log::warn!("could not save settings: {error}");
        }
    }

    pub fn load_custom_themes(&mut self) {
        self.custom_themes.start(
            self.dirs.config.join("themes"),
            self.settings.custom_theme.clone(),
            &self.waker,
        );
    }

    fn poll_custom_themes(&mut self) {
        if self.custom_themes.needs_reload() {
            self.load_custom_themes();
        }
        if !self.custom_themes.poll() {
            return;
        }
        let mut changed = false;
        if let Some(filename) = &self.settings.custom_theme
            && let Some(theme) = self.custom_themes.find(filename)
            && self.settings.custom_theme_cache.as_ref() != Some(theme)
        {
            self.settings.custom_theme_cache = Some(theme.clone());
            changed = true;
        }
        if self.custom_themes.follows_omarchy() {
            if let Some(theme) = self.custom_themes.system_theme()
                && self.settings.system_theme_cache.as_ref() != Some(theme)
            {
                self.settings.system_theme_cache = Some(theme.clone());
                changed = true;
            }
        } else if self.settings.system_theme_cache.take().is_some() {
            changed = true;
        }
        if changed {
            self.mark_settings_dirty();
        }
    }

    fn apply_theme(&mut self, ctx: &egui::Context) {
        let preference = self.settings.cached_palette().map_or_else(
            || match self.settings.theme {
                ThemeChoice::Dark => egui::ThemePreference::Dark,
                ThemeChoice::Light => egui::ThemePreference::Light,
                ThemeChoice::System => egui::ThemePreference::System,
            },
            |palette| {
                if palette.dark {
                    egui::ThemePreference::Dark
                } else {
                    egui::ThemePreference::Light
                }
            },
        );
        ctx.set_theme(preference);
        // Use the same preference for our palette and egui's native controls.
        let dark = ctx.theme() == egui::Theme::Dark;
        let palette = self.settings.cached_palette().unwrap_or_else(|| {
            if dark {
                Palette::dark()
            } else {
                Palette::light()
            }
        });
        if self.applied_dark.is_none() || self.palette != palette {
            self.palette = palette;
            crate::theme::apply(ctx, &self.palette);
            self.applied_dark = Some(dark);
        }
        if !self.zoom_applied {
            ctx.set_zoom_factor(self.settings.zoom);
            self.zoom_applied = true;
        }
    }

    fn apply_actions(&mut self, ctx: &egui::Context) {
        let mut actions = std::mem::take(&mut self.actions);
        while !actions.is_empty() {
            for action in actions.drain(..) {
                self.apply(action, ctx);
            }
            actions = std::mem::take(&mut self.actions);
        }
    }

    fn apply(&mut self, action: Action, ctx: &egui::Context) {
        match action {
            Action::Open(page) => {
                let opens_chats = page == Page::Chats;
                self.page = page;
                self.dialog = None;
                self.emoji_start = None;
                self.mention_start = None;
                if opens_chats {
                    self.refocus_composer(ctx);
                }
            }
            Action::OpenChat(id) => self.open_chat(id),
            Action::StartChat { id, name } => {
                if self.chat(&id).is_none() {
                    self.chats.push(Chat::new(id.clone(), name.clone()));
                    self.backend.send(Command::EnsureChat {
                        chat: id.clone(),
                        name,
                    });
                }
                self.open_chat(id);
            }
            Action::OpenMessage { chat, message } => {
                self.open_chat(chat.clone());
                // Keep the search result, not the chat end, in view.
                self.scroll_to_bottom = false;
                self.at_bottom = false;
                self.scroll_anchor = Some(message.clone());
                let conversation = self.conversations.entry(chat.clone()).or_default();
                if conversation.message(&message).is_none()
                    && !conversation.loading_older
                    && let Some(oldest) = conversation.messages.first()
                {
                    // Load older archive pages toward the search result.
                    conversation.loading_older = true;
                    self.backend.send(Command::LoadUntil {
                        chat,
                        id: message,
                        before: (oldest.timestamp, oldest.id.clone()),
                    });
                }
            }
            Action::CloseChat => {
                if let Some(chat) = self.open_chat.take() {
                    self.stop_composing(&chat);
                    let draft = std::mem::take(&mut self.composer);
                    if self.editing.take().is_none() && !draft.trim().is_empty() {
                        self.drafts.insert(chat.clone(), draft);
                        self.draft_mentions
                            .insert(chat, std::mem::take(&mut self.composer_mentions));
                    } else {
                        self.composer_mentions.clear();
                    }
                }
                self.reply_to = None;
                self.emoji_start = None;
                self.mention_start = None;
                self.reaction_target = None;
                self.reaction_anchor = None;
                self.emoji_jump = None;
            }
            Action::SendText {
                chat,
                text,
                quoting,
            } => {
                self.send_text(chat, text, quoting);
                self.reply_to = None;
            }
            Action::RefreshPoll { chat, message } => {
                if let Some(row) = self
                    .conversations
                    .get_mut(&chat)
                    .and_then(|chat| chat.message_mut(&message))
                    && let Content::Poll { state, .. } = &mut row.content
                {
                    state.refreshing = true;
                }
                self.backend.send(Command::RefreshPoll { chat, message });
            }
            Action::CreatePoll { chat, draft } => {
                if !self.poll_creating {
                    match draft.validated() {
                        Ok(draft) => {
                            self.poll_creating = true;
                            self.backend.send(Command::CreatePoll { chat, draft });
                        }
                        Err(error) => self.toast_error(error),
                    }
                }
            }
            Action::VotePoll {
                chat,
                message,
                choices,
            } => {
                if self.poll_voting.insert((chat.clone(), message.clone())) {
                    self.backend.send(Command::VotePoll {
                        chat,
                        message,
                        choices,
                    });
                }
            }
            Action::Composing { chat, composing } => {
                if composing {
                    self.note_keystroke();
                } else {
                    self.stop_composing(&chat);
                }
            }
            Action::MarkRead(chat) => self.mark_read(&chat),
            Action::LoadOlder(chat) => self.load_older(&chat),
            Action::FetchOlder(chat) => self.fetch_older(&chat),
            Action::Download { chat, message } => {
                if let Some(media) = self
                    .conversations
                    .get_mut(&chat)
                    .and_then(|conversation| conversation.message_mut(&message))
                    .and_then(|message| message.content.media_mut())
                {
                    media.state = MediaState::Downloading;
                }
                self.backend.send(Command::Download { chat, message });
            }
            Action::OpenFile(path) => {
                if let Err(error) = open::that_detached(&path) {
                    self.toast_error(format!("Could not open {}: {error}", path.display()));
                }
            }
            Action::OpenUrl(url) => ctx.open_url(egui::OpenUrl::new_tab(url)),
            Action::CopyText(text) => {
                ctx.copy_text(text);
                self.toast("Copied");
            }
            Action::Reply(id) => {
                self.reply_to = Some(id);
                self.focus_composer = true;
            }
            Action::CancelReply => self.reply_to = None,
            Action::Forward {
                from_chat,
                message,
                to_chat,
            } => {
                self.backend.send(Command::Forward {
                    from_chat,
                    message,
                    to_chat,
                });
                self.dialog = None;
                self.forward_search.clear();
            }
            Action::Edit(id) => {
                let text = self
                    .open_chat
                    .as_deref()
                    .and_then(|chat| self.conversations.get(chat))
                    .and_then(|conversation| conversation.message(&id))
                    .and_then(|message| match &message.content {
                        Content::Text { text, .. } => Some(text.clone()),
                        _ => None,
                    });
                if let Some(text) = text {
                    self.editing = Some(id);
                    self.reply_to = None;
                    self.composer = text;
                    self.composer_mentions.clear();
                    self.emoji_start = None;
                    self.mention_start = None;
                    self.focus_composer = true;
                }
            }
            Action::CancelEdit => {
                if self.editing.take().is_some() {
                    self.composer.clear();
                    self.composer_mentions.clear();
                    self.emoji_start = None;
                    self.mention_start = None;
                }
            }
            Action::DeleteForEveryone(id) => {
                if let Some(chat) = self.open_chat.clone() {
                    if let Some(message) = self
                        .conversations
                        .get_mut(&chat)
                        .and_then(|conversation| conversation.message_mut(&id))
                    {
                        message.content = Content::Revoked;
                    }
                    self.backend.send(Command::Revoke { chat, id });
                }
            }
            Action::DeleteForMe(id) => {
                if let Some(chat) = self.open_chat.clone() {
                    if let Some(conversation) = self.conversations.get_mut(&chat) {
                        conversation.messages.retain(|message| message.id != id);
                    }
                    self.backend.send(Command::DeleteLocal { chat, id });
                }
            }
            Action::Attach => {
                if let Some(chat) = self.open_chat.clone() {
                    self.backend.send(Command::PickFiles(chat));
                }
            }
            Action::SendFiles(paths) => self.stage_files(paths),
            Action::SendPending { chat, caption } => self.send_pending(chat, caption),
            Action::RemovePending(index) => {
                if index < self.pending.len() {
                    self.pending.remove(index);
                }
            }
            Action::ClearPending => self.pending.clear(),
            Action::PlayVoice { message, path } => self.play_voice(message, path),
            Action::SeekVoice {
                message,
                path,
                fraction,
            } => {
                if let Err(error) = self.player.seek(&message, &path, fraction) {
                    self.toast_error(error);
                }
            }
            Action::StartRecording => {
                if self.open_chat.is_some() && self.recording.is_none() {
                    self.recording = Some(Recorder::start(self.waker.clone()));
                }
            }
            Action::CancelRecording => {
                self.recording = None;
                self.refocus_composer(ctx);
            }
            Action::SendRecording => {
                self.send_recording();
                self.refocus_composer(ctx);
            }
            Action::SetMuted(chat, until) => {
                if let Some(known) = self.chat_mut(&chat) {
                    known.muted_until = until;
                }
                self.backend.send(Command::SetMuted(chat, until));
            }
            Action::TogglePicker(tab) => {
                self.emoji_start = None;
                self.mention_start = None;
                self.reaction_target = None;
                self.reaction_anchor = None;
                if self.picker == Some(tab) {
                    self.picker = None;
                    self.refocus_composer(ctx);
                } else {
                    self.picker = Some(tab);
                    self.picker_search.clear();
                    self.picker_focus = tab == PickerTab::Emoji;
                    self.emoji_selected = 0;
                    self.emoji_jump = None;
                    if tab == PickerTab::Stickers {
                        self.stickers_pending = self.stickers.is_empty()
                            && self.stickers_saved.is_empty()
                            && self.sticker_packs.is_empty();
                        self.backend.send(Command::RecentStickers);
                    }
                    if tab == PickerTab::Gifs && self.gif_results.is_empty() {
                        self.actions.push(Action::SearchGifs(String::new()));
                    }
                }
            }
            Action::ClosePicker => {
                let was_reaction = self.reaction_target.is_some();
                self.picker = None;
                self.reaction_target = None;
                self.reaction_anchor = None;
                self.emoji_jump = None;
                if !was_reaction {
                    self.refocus_composer(ctx);
                }
            }
            Action::OpenReactionPicker { chat, message } => {
                self.emoji_start = None;
                self.mention_start = None;
                self.picker = None;
                self.reaction_target = Some((chat, message));
                self.picker_search.clear();
                self.picker_focus = true;
                self.emoji_selected = 0;
                self.emoji_jump = None;
                self.reaction_anchor = ctx.input(|input| {
                    input
                        .pointer
                        .latest_pos()
                        .map(|pos| egui::Rect::from_center_size(pos, egui::vec2(34.0, 34.0)))
                });
            }
            Action::InsertEmoji(emoji) => {
                self.insert_in_composer(ctx, &emoji);
                self.remember_emoji(&emoji);
                self.focus_composer = true;
            }
            Action::InsertEmojiCompletion { emoji, start, end } => {
                let starts_with_colon = start
                    .checked_add(1)
                    .is_some_and(|after| self.composer.get(start..after) == Some(":"));
                if starts_with_colon
                    && start <= end
                    && self.composer.is_char_boundary(start)
                    && self.composer.is_char_boundary(end)
                {
                    self.composer.replace_range(start..end, &emoji);
                    let cursor = self.composer[..start].chars().count() + emoji.chars().count();
                    self.set_composer_cursor(ctx, cursor);
                    self.remember_emoji(&emoji);
                    self.focus_composer = true;
                }
                self.emoji_start = None;
            }
            Action::CloseEmojiSuggestions => {
                self.emoji_start = None;
                self.focus_composer = true;
            }
            Action::InsertMention {
                id,
                name,
                start,
                end,
            } => {
                let member = self.current_chat().is_some_and(|chat| {
                    chat.is_group() && chat.participants.iter().any(|known| known == &id)
                });
                let mention_at = start
                    .checked_add(1)
                    .is_some_and(|after| self.composer.get(start..after) == Some("@"));
                if member
                    && start <= end
                    && self.composer.is_char_boundary(start)
                    && self.composer.is_char_boundary(end)
                    && mention_at
                {
                    let mention = format!("@{name}");
                    let inserted = format!("{mention} ");
                    self.composer.replace_range(start..end, &inserted);
                    self.composer_mentions.push(ComposerMention { id, name });
                    let cursor = self.composer[..start + inserted.len()].chars().count();
                    self.set_composer_cursor(ctx, cursor);
                    self.focus_composer = true;
                }
                self.emoji_start = None;
                self.mention_start = None;
            }
            Action::CloseMentions => self.mention_start = None,
            Action::SaveSticker(path) => {
                self.backend.send(Command::SaveSticker { path });
                self.toast("Sticker saved");
            }
            Action::ForgetSticker(path) => {
                self.backend.send(Command::ForgetSticker { path });
            }
            Action::ImportStickerUrl(url) => {
                self.sticker_import_pending = true;
                self.sticker_link.clear();
                self.backend.send(Command::ImportStickerUrl { url });
            }
            Action::PickStickerArchive => {
                self.sticker_import_pending = true;
                self.backend.send(Command::PickStickerArchive);
            }
            Action::DeleteStickerPack(dir) => {
                self.backend.send(Command::DeleteStickerPack { dir });
            }
            Action::SendSticker(path) => {
                if let Some(chat) = self.open_chat.clone() {
                    self.backend.send(Command::SendSticker { chat, path });
                    self.picker = None;
                    self.scroll_to_bottom = true;
                    self.at_bottom = true;
                    self.refocus_composer(ctx);
                }
            }
            Action::SearchGifs(query) => {
                self.gif_query = query.clone();
                self.gif_pending = true;
                self.gif_error = None;
                self.backend.send(Command::SearchGifs {
                    query,
                    key: self.settings.effective_giphy_key().unwrap_or_default(),
                });
            }
            Action::SendGif(gif) => {
                if let Some(chat) = self.open_chat.clone() {
                    self.toast("Sending GIF…");
                    self.backend.send(Command::SendGif { chat, gif });
                    self.picker = None;
                    self.scroll_to_bottom = true;
                    self.at_bottom = true;
                    self.refocus_composer(ctx);
                }
            }
            Action::PasteImage {
                width,
                height,
                rgba,
            } => {
                // Stage the files so the user can add a caption.
                if self.open_chat.is_some() {
                    self.pending.push(Pending::Picture {
                        width,
                        height,
                        rgba: std::sync::Arc::new(rgba),
                        texture: None,
                    });
                    self.focus_composer = true;
                }
            }
            Action::React {
                chat,
                message,
                emoji,
            } => {
                if !emoji.is_empty() {
                    self.remember_emoji(&emoji);
                }
                self.reaction_target = None;
                self.reaction_anchor = None;
                self.backend.send(Command::React {
                    chat,
                    message,
                    emoji,
                });
            }
            Action::SetArchived(chat, archived) => {
                if let Some(known) = self.chat_mut(&chat) {
                    known.archived = archived;
                }
                if archived && self.open_chat.as_deref() == Some(chat.as_str()) {
                    self.actions.push(Action::CloseChat);
                }
                self.backend.send(Command::SetArchived(chat, archived));
            }
            Action::SetPinned(chat, pinned) => {
                if let Some(known) = self.chat_mut(&chat) {
                    known.pinned = pinned;
                    known.pinned_at = if pinned {
                        jiff::Timestamp::now().as_millisecond()
                    } else {
                        0
                    };
                }
                self.backend.send(Command::SetPinned(chat, pinned));
            }
            Action::ShowDialog(dialog) => {
                self.emoji_start = None;
                self.mention_start = None;
                if matches!(&dialog, Dialog::CreatePoll(_)) && !self.poll_creating {
                    self.poll_draft = Default::default();
                }
                if matches!(&dialog, Dialog::Forward { .. }) {
                    self.forward_search.clear();
                }
                if dialog == Dialog::PairWithPhone {
                    self.pair_phone.clear();
                }
                if dialog == Dialog::NewContact {
                    self.new_contact_phone.clear();
                    self.new_contact_name.clear();
                    self.new_contact_last.clear();
                    self.new_contact_pending = false;
                }
                self.contact_edit = None;
                self.dialog = Some(dialog);
            }
            Action::CloseDialog => {
                self.dialog = None;
                self.forward_search.clear();
                self.contact_edit = None;
                self.refocus_composer(ctx);
            }
            Action::EditContact(prefill) => {
                self.contact_edit = Some(crate::util::split_name(&prefill));
            }
            Action::SaveContact { id, first, last } => {
                self.contact_edit = None;
                let (full_name, first_name) = compose_name(&first, &last);
                let Some(full_name) = full_name else {
                    return;
                };
                self.backend.send(Command::SaveContact {
                    id,
                    full_name,
                    first_name,
                    to_phone: self.settings.save_contacts_to_phone,
                });
            }
            Action::NewContact { phone, first, last } => {
                self.new_contact_pending = true;
                let (full_name, first_name) = compose_name(&first, &last);
                self.backend.send(Command::NewContact {
                    phone,
                    full_name,
                    first_name,
                    to_phone: self.settings.save_contacts_to_phone,
                });
            }
            Action::ToggleSidebar => self.sidebar_visible = !self.sidebar_visible,
            Action::FocusSearch => {
                self.sidebar_visible = true;
                self.page = Page::Chats;
                self.focus_composer = false;
                self.focus_search = true;
                self.emoji_start = None;
                self.mention_start = None;
            }
            Action::FocusComposer => {
                self.focus_search = false;
                self.focus_composer = true;
            }
            Action::ScrollToBottom => self.scroll_to_bottom = true,
            Action::ScrollTo(id) => {
                self.scroll_to_bottom = false;
                let Some(chat) = self.open_chat.clone() else {
                    return;
                };
                let conversation = self.conversations.entry(chat.clone()).or_default();
                if conversation.message(&id).is_none()
                    && !conversation.loading_older
                    && let Some(oldest) = conversation.messages.first()
                {
                    // Load older archive pages toward the target.
                    conversation.loading_older = true;
                    self.backend.send(Command::LoadUntil {
                        chat,
                        id: id.clone(),
                        before: (oldest.timestamp, oldest.id.clone()),
                    });
                }
                self.scroll_anchor = Some(id);
            }
            Action::Search(text) => {
                self.search = text;
                let query = self.search.trim().to_owned();
                if query.is_empty() {
                    self.search_hits.clear();
                } else {
                    self.backend.send(Command::SearchMessages { query });
                }
            }
            Action::ShowUpdate => {
                self.show_update = self.update.is_some();
                self.inspect_update();
            }
            Action::CloseUpdate => self.show_update = false,
            Action::DownloadUpdate => self.download_update(),
            Action::InstallUpdate => {
                if matches!(
                    self.update_download,
                    crate::updates::DownloadState::Ready(_)
                ) {
                    let crate::updates::DownloadState::Ready(prepared) = std::mem::replace(
                        &mut self.update_download,
                        crate::updates::DownloadState::Installing,
                    ) else {
                        unreachable!()
                    };
                    self.backend.send(Command::InstallUpdate {
                        prepared,
                        arguments: self.update_arguments.clone(),
                    });
                }
            }
            Action::SetTheme(choice) => {
                self.settings.theme = choice;
                self.settings.custom_theme = None;
                self.settings.custom_theme_cache = None;
                self.mark_settings_dirty();
                self.apply_theme(ctx);
            }
            Action::SetCustomTheme(filename) => {
                if let Some(theme) = self.custom_themes.find(&filename) {
                    self.settings.custom_theme_cache = Some(theme.clone());
                    self.settings.custom_theme = Some(filename);
                    self.mark_settings_dirty();
                    self.apply_theme(ctx);
                }
            }
            Action::ReloadThemes => self.load_custom_themes(),
            Action::OpenThemesFolder => {
                let directory = self.dirs.config.join("themes");
                std::thread::spawn(move || {
                    if std::fs::create_dir_all(&directory).is_ok() {
                        let _ = open::that(directory);
                    }
                });
            }
            Action::PickMediaDir => self.backend.send(Command::PickMediaDir),
            Action::ResetMediaDir => self.backend.send(Command::ResetMediaDir),
            Action::HideShortcutHints => {
                self.settings.show_shortcut_hints = false;
                self.mark_settings_dirty();
            }
            Action::SettingsChanged => self.mark_settings_dirty(),
            Action::ZoomBy(delta) => {
                self.settings.zoom = (self.settings.zoom + delta).clamp(0.6, 2.0);
                self.zoom_applied = false;
                self.mark_settings_dirty();
            }
            Action::ResetZoom => {
                self.settings.zoom = 1.0;
                self.zoom_applied = false;
                self.mark_settings_dirty();
            }
            Action::PairWithPhone(phone) => {
                let digits: String = phone.chars().filter(char::is_ascii_digit).collect();
                if digits.len() < 7 {
                    self.toast_error(
                        "Enter the phone number with its country code, using digits only",
                    );
                } else {
                    self.backend.send(Command::PairWithPhone(digits));
                }
            }
            Action::Unlink => {
                self.dialog = None;
                self.backend.send(Command::Unlink);
            }
            Action::Reconnect => self.backend.send(Command::Reconnect),
            Action::Quit => {
                self.quit_requested = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            Action::ShowWindow => {
                if self.window_hidden {
                    // The headless loop in `main` will create the window.
                    self.wants_show = true;
                } else {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                }
            }
            Action::HideWindow => {
                if self.tray.is_some() {
                    self.hide_intent = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
            // Route through the configured window-close behavior.
            Action::CloseWindow => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
        }
    }

    pub fn toast(&mut self, message: impl Into<String>) {
        self.toasts.push(Toast {
            message: message.into(),
            kind: ToastKind::Info,
            created: Instant::now(),
        });
        self.toasts.truncate(4);
    }

    pub fn toast_error(&mut self, message: impl Into<String>) {
        let message = message.into();
        log::warn!("{message}");
        self.toasts.push(Toast {
            message,
            kind: ToastKind::Error,
            created: Instant::now(),
        });
    }

    /// Processes app state shared by windowed and headless modes.
    pub fn background_frame(&mut self, ctx: &egui::Context) {
        // Events are drained before frame_ui observes focus. Losing focus in
        // this frame must take effect before an incoming chat update can read it.
        if self.window_hidden || ctx.input(|input| input.viewport().focused) == Some(false) {
            self.window_focused = false;
        }
        self.handle_tray();
        #[cfg(target_os = "macos")]
        self.actions
            .extend(crate::macos::drain(ctx, self.window_hidden));
        self.handle_control_commands();
        self.poll_custom_themes();
        self.handle_notification_opens();
        self.handle_events();
        self.tick(ctx);
        self.tick_audio();
        self.apply_actions(ctx);
    }

    /// Polls audio state and schedules repaints while it changes.
    fn tick_audio(&mut self) {
        if let Err(error) = self.player.poll() {
            self.toast_error(error);
        }
        if let Some(error) = self.recording.as_ref().and_then(Recorder::failure) {
            self.recording = None;
            self.toast_error(format!("Could not record: {error}"));
        }
        if self.player.is_playing() || self.recording.is_some() {
            self.waker.wake_after(Duration::from_millis(40));
        }
    }

    /// Plays or pauses audio and sends the first played receipt when needed.
    fn play_voice(&mut self, message: String, path: PathBuf) {
        if let Err(error) = self.player.toggle(&message, &path) {
            self.toast_error(error);
            return;
        }
        self.tell_played(message);
    }

    fn tell_played(&mut self, message: String) {
        let Some(chat) = self.open_chat.clone() else {
            return;
        };
        if self.played_told.contains(&message) {
            return;
        }
        let Some(row) = self
            .conversations
            .get(&chat)
            .and_then(|conversation| conversation.message(&message))
        else {
            return;
        };
        if row.from_me {
            return;
        }
        let sender = row.sender.clone();
        self.played_told.insert(message.clone());
        self.backend.send(Command::MarkPlayed {
            chat,
            message,
            sender,
            receipts: self.settings.send_read_receipts,
        });
    }

    /// Stops and sends a recording unless it is under one second.
    fn send_recording(&mut self) {
        let Some(recorder) = self.recording.take() else {
            return;
        };
        let Some(chat) = self.open_chat.clone() else {
            return;
        };
        match recorder.finish() {
            Ok(samples) if samples.len() < crate::voice::RATE as usize / 2 => {}
            Ok(samples) => {
                let quoting = self.reply_to.take();
                self.backend.send(Command::SendVoice {
                    chat,
                    samples,
                    quoting,
                });
            }
            Err(error) => self.toast_error(format!("Could not record: {error}")),
        }
    }

    pub fn frame_ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let ctx = &ctx;
        self.copy_rows
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clear();
        *self
            .selection_view
            .lock()
            .unwrap_or_else(|p| p.into_inner()) = None;
        self.apply_theme(ctx);
        let focused = ctx.input(|input| input.viewport().focused.unwrap_or(true));
        let regained_focus = focused && !self.window_focused;
        // Mark messages received while hidden as read on window return.
        if regained_focus
            && self.page == Page::Chats
            && let Some(open) = self.open_chat.clone()
            && self.chat(&open).is_some_and(|chat| chat.unread > 0)
        {
            self.mark_read(&open);
        }
        if regained_focus {
            self.refocus_composer(ctx);
        }
        self.window_focused = focused;
        // Close the window and continue headless when background mode is enabled.
        if ctx.input(|input| input.viewport().close_requested())
            && !self.quit_requested
            && self.hides_to_tray()
        {
            self.hide_intent = true;
        }
        self.lock_scroll_axis(ctx);
        self.take_drops_and_pastes(ctx);
        crate::ui::show(self, ui);
        self.apply_actions(ctx);
        if !self.toasts.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(120));
        }
    }

    /// Inserts text at the composer cursor or end.
    fn insert_in_composer(&mut self, ctx: &egui::Context, text: &str) {
        let id = egui::Id::new("composer-text");
        let at = egui::TextEdit::load_state(ctx, id)
            .and_then(|state| state.cursor.char_range())
            .map(|range| range.primary.index.0)
            .unwrap_or_else(|| self.composer.chars().count());
        let at = at.min(self.composer.chars().count());
        let byte = self
            .composer
            .char_indices()
            .nth(at)
            .map_or(self.composer.len(), |(byte, _)| byte);
        self.composer.insert_str(byte, text);
        self.set_composer_cursor(ctx, at + text.chars().count());
    }

    fn set_composer_cursor(&self, ctx: &egui::Context, at: usize) {
        let id = egui::Id::new("composer-text");
        if let Some(mut state) = egui::TextEdit::load_state(ctx, id) {
            state
                .cursor
                .set_char_range(Some(egui::text::CCursorRange::one(
                    egui::text::CCursor::new(at),
                )));
            egui::TextEdit::store_state(ctx, id, state);
        }
    }

    fn remember_emoji(&mut self, emoji: &str) {
        self.settings.recent_emoji.retain(|known| known != emoji);
        self.settings.recent_emoji.insert(0, emoji.to_owned());
        self.settings.recent_emoji.truncate(36);
        self.mark_settings_dirty();
    }

    /// Handles dropped files and pasted images for the open chat.
    fn take_drops_and_pastes(&mut self, ctx: &egui::Context) {
        let (dropped, hovering, paste) = ctx.input(|input| {
            let dropped: Vec<PathBuf> = input
                .raw
                .dropped_files
                .iter()
                .map(|file| file.path().to_path_buf())
                .collect();
            let hovering = !input.raw.hovered_files.is_empty();
            (dropped, hovering, wants_paste(input))
        });
        self.dropping = hovering && self.open_chat.is_some();
        if !dropped.is_empty() {
            self.actions.push(Action::SendFiles(dropped));
        }
        // Handle image paste only when the composer or no field has focus.
        let composing = ctx.memory(|memory| {
            memory.has_focus(egui::Id::new("composer-text")) || memory.focused().is_none()
        });
        if paste && composing && self.open_chat.is_some() {
            // egui handles text paste; the app handles clipboard images.
            if let Some(image) = clipboard_image() {
                self.actions.push(Action::PasteImage {
                    width: image.0,
                    height: image.1,
                    rgba: image.2,
                });
            }
        }
    }

    /// Locks trackpad scrolling to one axis, scales Linux deltas, and adds glide.
    fn lock_scroll_axis(&mut self, ctx: &egui::Context) {
        let (raw, from_trackpad, ended) = ctx.input(|input| {
            let mut sum = egui::Vec2::ZERO;
            let mut pointish = false;
            let mut ended = false;
            for event in &input.events {
                if let egui::Event::MouseWheel {
                    unit, delta, phase, ..
                } = event
                {
                    sum += *delta;
                    pointish |= *unit == egui::MouseWheelUnit::Point;
                    ended |= matches!(phase, egui::TouchPhase::End | egui::TouchPhase::Cancel);
                }
            }
            (sum, pointish, ended)
        });
        let now = Instant::now();
        if raw != egui::Vec2::ZERO {
            self.scroll_from_trackpad = from_trackpad;
        }
        let trackpad_here = cfg!(target_os = "linux") && self.scroll_from_trackpad;
        if trackpad_here {
            ctx.input_mut(|input| input.smooth_scroll_delta *= TRACKPAD_SCALE);
        }
        if trackpad_here && raw != egui::Vec2::ZERO {
            self.glide = None;
            self.scroll_accum += raw * TRACKPAD_SCALE;
            self.scroll_history
                .add(ctx.input(|input| input.time), self.scroll_accum);
            self.scroll_last_event = Some(now);
            ctx.request_repaint_after(Duration::from_millis(60));
        } else if raw != egui::Vec2::ZERO || ctx.input(|input| input.pointer.any_down()) {
            self.glide = None;
            self.scroll_history.clear();
            self.scroll_last_event = None;
        }
        let quiet = self
            .scroll_last_event
            .is_some_and(|at| now.duration_since(at).as_secs_f32() > 0.15);
        if ended || quiet {
            let mut velocity = self.scroll_history.velocity().unwrap_or(egui::Vec2::ZERO);
            if let Some((axis, _)) = self.scroll_lock {
                match axis {
                    ScrollAxis::Horizontal => velocity.y = 0.0,
                    ScrollAxis::Vertical => velocity.x = 0.0,
                }
            }
            self.glide = (velocity.length() > GLIDE_START).then_some(velocity);
            self.scroll_history.clear();
            self.scroll_accum = egui::Vec2::ZERO;
            self.scroll_last_event = None;
        }
        if let Some(velocity) = self.glide {
            if raw == egui::Vec2::ZERO {
                let dt = ctx.input(|input| input.stable_dt).clamp(0.001, 0.05);
                ctx.input_mut(|input| input.smooth_scroll_delta += velocity * dt);
                let slower = velocity * (-dt / GLIDE_DECAY).exp();
                self.glide = (slower.length() > GLIDE_STOP).then_some(slower);
            }
            ctx.request_repaint_after(Duration::from_millis(8));
        }
        let held = self
            .scroll_lock
            .filter(|(_, at)| now.duration_since(*at) < SCROLL_GESTURE_GAP)
            .map(|(axis, _)| axis);
        let moved = raw != egui::Vec2::ZERO;
        let axis = match held {
            Some(axis) => axis,
            None if moved && raw.x.abs() > raw.y.abs() * 1.2 => ScrollAxis::Horizontal,
            None if moved => ScrollAxis::Vertical,
            None => {
                self.scroll_lock = None;
                return;
            }
        };
        if moved {
            self.scroll_lock = Some((axis, now));
        }
        ctx.input_mut(|input| match axis {
            ScrollAxis::Horizontal => input.smooth_scroll_delta.y = 0.0,
            ScrollAxis::Vertical => input.smooth_scroll_delta.x = 0.0,
        });
    }

    pub fn save_state(&mut self) {
        if self.settings_dirty {
            self.save_settings();
        }
    }

    pub fn shutdown(&mut self) {
        self.save_state();
        self.backend.shutdown();
    }

    /// Returns attachment state for a loaded message.
    pub fn media_of(&self, chat: &str, id: &str) -> Option<&Media> {
        self.conversations.get(chat)?.message(id)?.content.media()
    }
}

/// Detects paste from the key release. egui consumes the press and emits a
/// `Paste` event only for text, so image paste has no key-press event.
/// Builds WhatsApp's full and short contact names. A first name is required.
fn compose_name(first: &str, last: &str) -> (Option<String>, Option<String>) {
    let first = first.trim();
    let last = last.trim();
    if first.is_empty() && last.is_empty() {
        return (None, None);
    }
    let full = if last.is_empty() {
        first.to_owned()
    } else if first.is_empty() {
        last.to_owned()
    } else {
        format!("{first} {last}")
    };
    let short = (!first.is_empty()).then(|| first.to_owned());
    (Some(full), short)
}

fn contains_mention_token(text: &str, user: &str) -> bool {
    let token = format!("@{user}");
    let mut rest = text;
    while let Some(at) = rest.find(&token) {
        let after = &rest[at + token.len()..];
        if after
            .chars()
            .next()
            .is_none_or(|character| !character.is_ascii_digit())
        {
            return true;
        }
        rest = &rest[at + 1..];
    }
    false
}

fn find_named_mention(text: &str, token: &str) -> Option<usize> {
    text.match_indices(token).find_map(|(at, _)| {
        let after = &text[at + token.len()..];
        after
            .chars()
            .next()
            .is_none_or(|character| !character.is_alphanumeric())
            .then_some(at)
    })
}

fn mention_refs(ids: &[String]) -> Vec<crate::model::MentionRef> {
    ids.iter()
        .filter_map(|id| {
            let user = id.split('@').next()?.to_owned();
            (!user.is_empty()).then(|| crate::model::MentionRef {
                user,
                id: id.clone(),
            })
        })
        .collect()
}

pub fn wants_paste(input: &egui::InputState) -> bool {
    input.events.iter().any(|event| {
        matches!(
            event,
            egui::Event::Key {
                key: egui::Key::V,
                pressed: false,
                modifiers,
                ..
            } if modifiers.command
        )
    })
}

/// Clipboard image as width, height, and straight-alpha RGBA.
fn clipboard_image() -> Option<(usize, usize, Vec<u8>)> {
    let mut clipboard = arboard::Clipboard::new().ok()?;
    let image = clipboard.get_image().ok()?;
    if image.width == 0 || image.height == 0 {
        return None;
    }
    Some((image.width, image.height, image.bytes.into_owned()))
}

impl Delivery {
    /// Whether an outgoing message is still pending.
    pub fn in_flight(self) -> bool {
        matches!(self, Delivery::Pending)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Content;

    fn app() -> App {
        let root = std::env::temp_dir().join(format!("zapfast-app-{}", std::process::id()));
        App::headless(AppDirs::under(&root), Settings::default()).0
    }

    #[test]
    fn failed_poll_requests_keep_the_draft_and_clear_pending_controls() {
        let directory = tempfile::tempdir().unwrap();
        let (mut app, events) =
            App::headless(AppDirs::under(directory.path()), Settings::default());
        let ctx = egui::Context::default();
        let draft = crate::model::PollDraft {
            question: "Lunch?".into(),
            options: vec!["Pizza".into(), "Pasta".into()],
            multiple: false,
        };
        app.dialog = Some(Dialog::CreatePoll("chat".into()));
        app.poll_draft = draft.clone();
        app.apply(
            Action::CreatePoll {
                chat: "chat".into(),
                draft: draft.clone(),
            },
            &ctx,
        );
        assert!(app.poll_creating);
        events
            .send(Event::PollCreated {
                chat: "chat".into(),
                error: Some("Could not send".into()),
            })
            .unwrap();
        app.background_frame(&ctx);
        assert!(!app.poll_creating);
        assert_eq!(app.poll_draft, draft);
        assert!(app.dialog.is_some());
        app.apply(
            Action::VotePoll {
                chat: "chat".into(),
                message: "poll".into(),
                choices: vec![0],
            },
            &ctx,
        );
        assert_eq!(app.poll_voting.len(), 1);
        events
            .send(Event::PollVoted {
                chat: "chat".into(),
                message: "poll".into(),
                error: Some("Could not vote".into()),
            })
            .unwrap();
        app.background_frame(&ctx);
        assert!(app.poll_voting.is_empty());
    }

    #[test]
    fn follow_system_retains_the_os_theme_between_platform_events() {
        let mut app = app();
        app.settings.theme = ThemeChoice::System;
        let ctx = egui::Context::default();
        let mut input = egui::RawInput::default();
        for theme in [egui::Theme::Light, egui::Theme::Dark] {
            input.system_theme = Some(theme);
            // Native input preserves the OS preference when taking each frame.
            for _ in 0..2 {
                let mut output = ctx.run_ui(input.take(), |_| app.apply_theme(&ctx));
                output.textures_delta.clear();
                assert_eq!(app.palette.dark, theme == egui::Theme::Dark);
                assert_eq!(ctx.theme(), theme);
            }
        }
    }

    #[test]
    fn custom_theme_cache_survives_a_missing_file_and_follows_system_updates() {
        use crate::theme::custom::{Catalog, CustomTheme};
        let mut app = app();
        let ctx = egui::Context::default();
        let mut first = CustomTheme {
            filename: "mine.json".into(),
            palette: Palette::dark(),
        };
        first.palette.accent = egui::Color32::RED;
        app.custom_themes = Catalog::from_themes(vec![first.clone()]);
        app.apply(Action::SetCustomTheme(first.filename.clone()), &ctx);
        assert_eq!(app.palette.accent, egui::Color32::RED);
        // Cached selection remains usable while the file is temporarily missing.
        app.custom_themes = Catalog::default();
        app.settings =
            serde_json::from_str(&serde_json::to_string(&app.settings).unwrap()).unwrap();
        app.apply_theme(&ctx);
        assert_eq!(app.palette, first.palette);
        app.apply(Action::SetTheme(ThemeChoice::System), &ctx);
        assert!(app.settings.custom_theme.is_none());
        let mut system = first;
        system.filename = "omarchy.json".into();
        system.palette.accent = egui::Color32::GREEN;
        app.custom_themes
            .load_system_test(Some(system.clone()), true);
        let deadline = Instant::now() + Duration::from_secs(5);
        while app.settings.system_theme_cache.as_ref() != Some(&system) {
            app.poll_custom_themes();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        app.apply_theme(&ctx);
        assert_eq!(app.palette.accent, egui::Color32::GREEN);
        app.apply(Action::SetTheme(ThemeChoice::Light), &ctx);
        assert_eq!(app.palette, Palette::light());
    }

    #[test]
    fn automatic_updates_require_opt_in_and_explicit_restart() {
        use crate::updates::{
            DownloadState,
            install::{Installation, Kind, Prepared},
        };
        let mut app = app();
        let ctx = egui::Context::default();
        app.update = Some(crate::updates::Release {
            version: "99.0.0".into(),
            url: "https://github.com/crmne/zapfast/releases/latest".into(),
        });
        app.update_support = Some(Err("Use your package manager".into()));
        app.settings.download_updates_automatically = true;
        app.maybe_download_update();
        assert!(matches!(app.update_download, DownloadState::Idle));
        let installation = Installation {
            executable: PathBuf::from("/fixture/zapfast"),
            kind: Kind::Portable,
        };
        app.update_support = Some(Ok(installation.clone()));
        app.settings.download_updates_automatically = false;
        app.maybe_download_update();
        assert!(matches!(app.update_download, DownloadState::Idle));
        app.settings.download_updates_automatically = true;
        app.maybe_download_update();
        assert!(matches!(
            app.update_download,
            DownloadState::Downloading { .. }
        ));
        app.update_download = DownloadState::Ready(Box::new(Prepared {
            installation,
            directory: "/fixture/staging".into(),
            payload: "/fixture/staging/next".into(),
            sha256: String::new(),
            version: "99.0.0".into(),
        }));
        app.maybe_download_update();
        assert!(matches!(app.update_download, DownloadState::Ready(_)));
        assert!(!app.quit_requested);
        app.apply(Action::InstallUpdate, &ctx);
        assert!(matches!(app.update_download, DownloadState::Installing));
        assert!(!app.quit_requested, "wait for the helper before closing");
    }

    #[test]
    fn a_closed_window_does_not_read_new_messages_in_the_last_chat() {
        let mut app = app();
        let mut chat = Chat::new("peer@s.whatsapp.net".into(), "Peer".into());
        app.open_chat = Some(chat.id.clone());
        app.window_focused = true;
        app.window_gone();
        assert!(!app.window_focused);
        chat.unread = 2;
        app.handle_chat_updated(chat.clone());
        assert_eq!(app.chat(&chat.id).unwrap().unread, 2);
        // Focus left over from a window callback is insufficient while hidden.
        app.window_focused = true;
        app.handle_chat_updated(chat.clone());
        assert_eq!(app.chat(&chat.id).unwrap().unread, 2);
        app.window_hidden = false;
        app.handle_chat_updated(chat.clone());
        assert_eq!(app.chat(&chat.id).unwrap().unread, 0);
    }

    #[test]
    fn losing_focus_takes_effect_before_processing_an_incoming_chat_update() {
        let root = std::env::temp_dir().join("zapfast-focus-test");
        let (mut app, events) = App::headless(AppDirs::under(&root), Settings::default());
        let mut chat = Chat::new("peer@s.whatsapp.net".into(), "Peer".into());
        chat.unread = 1;
        app.open_chat = Some(chat.id.clone());
        app.window_focused = true;
        events
            .send(Event::ChatUpdated(Box::new(chat.clone())))
            .unwrap();
        let ctx = egui::Context::default();
        let mut input = egui::RawInput::default();
        input
            .viewports
            .get_mut(&egui::ViewportId::ROOT)
            .unwrap()
            .focused = Some(false);
        let mut output = ctx.run_ui(input, |ui| app.background_frame(ui.ctx()));
        output.textures_delta.clear();
        assert_eq!(app.chat(&chat.id).unwrap().unread, 1);
    }

    #[test]
    fn read_receipt_preference_applies_to_both_reading_and_voice_playback() {
        let mut app = app();
        let (backend, mut commands) = Backend::recording();
        app.backend = backend;
        let chat = "peer@s.whatsapp.net";
        app.open_chat = Some(chat.into());
        app.conversations
            .entry(chat.into())
            .or_default()
            .merge(vec![message(chat, "voice", 100)], false);
        app.settings.send_read_receipts = false;
        app.mark_read(chat);
        assert!(matches!(
            commands.try_recv().unwrap(),
            Command::MarkRead {
                receipts: false,
                ..
            }
        ));
        app.tell_played("voice".into());
        assert!(matches!(
            commands.try_recv().unwrap(),
            Command::MarkPlayed {
                receipts: false,
                ..
            }
        ));
        app.settings.send_read_receipts = true;
        app.played_told.clear();
        app.tell_played("voice".into());
        assert!(matches!(
            commands.try_recv().unwrap(),
            Command::MarkPlayed { receipts: true, .. }
        ));
    }

    fn message(chat: &str, id: &str, timestamp: i64) -> Message {
        Message {
            id: id.into(),
            chat: chat.into(),
            sender: chat.into(),
            sender_name: None,
            from_me: false,
            timestamp,
            content: Content::text(id),
            status: Delivery::None,
            delivered_at: None,
            read_at: None,
            quoted: None,
            reactions: Vec::new(),
            edited: false,
            mentions: Vec::new(),
            forwarded: false,
            thumbnail: None,
        }
    }

    #[test]
    fn conversations_merge_pages_without_duplicates() {
        let mut conversation = Conversation::default();
        conversation.merge(vec![message("c", "b", 2), message("c", "c", 3)], false);
        conversation.merge(vec![message("c", "a", 1), message("c", "b", 2)], true);
        let ids: Vec<&str> = conversation
            .messages
            .iter()
            .map(|m| m.id.as_str())
            .collect();
        assert_eq!(ids, vec!["a", "b", "c"]);
        conversation.merge(vec![message("c", "c", 3)], false);
        assert_eq!(conversation.messages.len(), 3);
    }

    #[test]
    fn a_search_hit_opens_its_chat_at_the_message() {
        let mut app = app();
        let ctx = egui::Context::default();
        let chat = "1@s.whatsapp.net";
        app.chats.push(Chat::new(chat.into(), "Ada".into()));
        let conversation = Conversation {
            requested: true,
            complete: true,
            messages: vec![message(chat, "old", 10)],
            ..Default::default()
        };
        app.conversations.insert(chat.into(), conversation);
        app.apply(
            Action::OpenMessage {
                chat: chat.into(),
                message: "old".into(),
            },
            &ctx,
        );
        assert_eq!(app.open_chat.as_deref(), Some(chat));
        assert_eq!(app.scroll_anchor.as_deref(), Some("old"));
        assert!(!app.scroll_to_bottom, "aims at the hit, not the end");
    }

    #[test]
    fn clearing_the_search_clears_its_hits() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.search_hits.push(message("1@s.whatsapp.net", "m", 1));
        app.apply(Action::Search(String::new()), &ctx);
        assert!(app.search_hits.is_empty());
    }

    #[test]
    fn matching_contacts_are_people_not_yet_talked_to() {
        let mut app = app();
        app.me = Some("490000000000@s.whatsapp.net".into());
        let contact = |id: &str, name: &str| crate::model::Contact {
            id: id.into(),
            full_name: Some(name.into()),
            push_name: None,
        };
        // Exclude contacts that already have chats.
        app.contacts.insert(
            "491700000001@s.whatsapp.net".into(),
            contact("491700000001@s.whatsapp.net", "Ada Lovelace"),
        );
        app.chats.push(Chat::new(
            "491700000001@s.whatsapp.net".into(),
            "Ada Lovelace".into(),
        ));
        // Include contacts without chats.
        app.contacts.insert(
            "491700000002@s.whatsapp.net".into(),
            contact("491700000002@s.whatsapp.net", "Adele Goldberg"),
        );
        // Exclude groups and our own id.
        app.contacts
            .insert("12345@g.us".into(), contact("12345@g.us", "Adventurers"));
        app.contacts.insert(
            "490000000000@s.whatsapp.net".into(),
            contact("490000000000@s.whatsapp.net", "Adah Me"),
        );
        app.search = "ad".into();
        let names: Vec<&str> = app
            .matching_contacts()
            .iter()
            .filter_map(|contact| contact.display_name())
            .collect();
        assert_eq!(names, vec!["Adele Goldberg"]);
        // Match phone-number digits.
        app.search = "491700000002".into();
        assert_eq!(app.matching_contacts().len(), 1);
        app.search = String::new();
        assert!(app.matching_contacts().is_empty());
    }

    #[test]
    fn visible_chats_pin_first_and_filter() {
        let mut app = app();
        let mut a = Chat::new("1@s.whatsapp.net".into(), "Ada".into());
        a.last_activity = 10;
        let mut b = Chat::new("2@s.whatsapp.net".into(), "Bob".into());
        b.last_activity = 20;
        let mut c = Chat::new("3@s.whatsapp.net".into(), "Cy".into());
        c.last_activity = 5;
        c.pinned = true;
        let mut d = Chat::new("4@s.whatsapp.net".into(), "Dee".into());
        d.archived = true;
        app.chats = vec![b, a, c, d];
        let names: Vec<&str> = app
            .visible_chats()
            .iter()
            .map(|chat| chat.name.as_str())
            .collect();
        assert_eq!(names, vec!["Cy", "Bob", "Ada"]);
        app.search = "ad".into();
        let names: Vec<&str> = app
            .visible_chats()
            .iter()
            .map(|chat| chat.name.as_str())
            .collect();
        assert_eq!(names, vec!["Ada"]);
    }

    #[test]
    fn pinned_order_survives_new_messages_and_legacy_pin_ties() {
        let mut app = app();
        for (id, pin, activity) in [("a", 100, 999), ("b", 200, 1), ("c", 0, 0), ("d", 0, 900)] {
            let mut chat = Chat::new(id.into(), id.into());
            chat.pinned = true;
            chat.pinned_at = pin;
            chat.last_activity = activity;
            app.chats.push(chat);
        }
        let order = |app: &App| {
            app.visible_chats()
                .iter()
                .map(|chat| chat.id.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(order(&app), ["b", "a", "c", "d"]);
        app.chats[0].last_activity = 10_000;
        app.chats[2].last_activity = 20_000;
        assert_eq!(order(&app), ["b", "a", "c", "d"]);
    }

    #[test]
    fn chat_and_contact_search_ignore_composed_and_decomposed_accents() {
        let mut app = app();
        app.chats
            .push(Chat::new("1@s.whatsapp.net".into(), "Ángel".into()));
        let contact = Contact {
            id: "2@s.whatsapp.net".into(),
            full_name: Some("A\u{301}ngel".into()),
            push_name: None,
        };
        app.contacts.insert(contact.id.clone(), contact);
        for query in ["angel", "ÁNGEL", "A\u{301}ngel"] {
            app.search = query.into();
            assert_eq!(app.visible_chats().len(), 1, "{query}");
            assert_eq!(app.matching_contacts().len(), 1, "{query}");
        }
        assert_eq!(app.chats[0].name, "Ángel");
        app.search = "bob".into();
        assert!(app.visible_chats().is_empty());
        assert!(app.matching_contacts().is_empty());
    }

    #[test]
    fn closing_a_chat_preserves_its_text_draft() {
        let mut app = app();
        let id = "1@s.whatsapp.net";
        app.chats.push(Chat::new(id.into(), "Ada".into()));
        app.open_chat(id.into());
        app.composer = "unfinished message".into();
        app.actions.push(Action::CloseChat);
        app.apply_actions(&egui::Context::default());
        assert!(app.open_chat.is_none());
        app.open_chat(id.into());
        assert_eq!(app.composer, "unfinished message");
    }

    #[test]
    fn opening_a_chat_keeps_drafts_apart() {
        let mut app = app();
        app.chats
            .push(Chat::new("1@s.whatsapp.net".into(), "Ada".into()));
        app.chats
            .push(Chat::new("2@s.whatsapp.net".into(), "Bob".into()));
        app.open_chat("1@s.whatsapp.net".into());
        app.composer = "hello ada".into();
        app.open_chat("2@s.whatsapp.net".into());
        assert_eq!(app.composer, "");
        app.open_chat("1@s.whatsapp.net".into());
        assert_eq!(app.composer, "hello ada");
        assert_eq!(app.settings.last_chat.as_deref(), Some("1@s.whatsapp.net"));
    }

    #[test]
    fn selected_mentions_become_wire_tokens_and_context_jids() {
        let mut app = app();
        let chat_id = "123@g.us";
        let member = "491702222222@s.whatsapp.net";
        let mut chat = Chat::new(chat_id.into(), "Group".into());
        chat.participants.push(member.into());
        app.chats.push(chat);
        app.composer_mentions.push(ComposerMention {
            id: member.into(),
            name: "Mira Example".into(),
        });

        let (text, mentions) = app.encode_composer_mentions(chat_id, "hello @Mira Example".into());

        assert_eq!(text, "hello @491702222222");
        assert_eq!(mentions, vec![member]);
    }

    #[test]
    fn existing_wire_mentions_survive_an_edit() {
        let mut app = app();
        let chat_id = "123@g.us";
        let member = "491702222222@s.whatsapp.net";
        let mut chat = Chat::new(chat_id.into(), "Group".into());
        chat.participants.push(member.into());
        app.chats.push(chat);

        let (text, mentions) = app.encode_composer_mentions(chat_id, "still @491702222222!".into());

        assert_eq!(text, "still @491702222222!");
        assert_eq!(mentions, vec![member]);
    }

    #[test]
    fn editing_a_selected_name_drops_its_mention() {
        let mut app = app();
        let chat_id = "123@g.us";
        let member = "491702222222@s.whatsapp.net";
        let mut chat = Chat::new(chat_id.into(), "Group".into());
        chat.participants.push(member.into());
        app.chats.push(chat);
        app.composer_mentions.push(ComposerMention {
            id: member.into(),
            name: "Mira".into(),
        });

        let (text, mentions) = app.encode_composer_mentions(chat_id, "hello @Miranda".into());

        assert_eq!(text, "hello @Miranda");
        assert!(mentions.is_empty());
    }

    #[test]
    fn dismissing_shortcut_hints_persists_and_focusing_keeps_the_draft() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.composer = "Unsent draft".into();
        app.focus_search = true;
        app.apply(Action::HideShortcutHints, &ctx);
        assert!(!app.settings.show_shortcut_hints);
        assert!(app.settings_dirty);
        app.apply(Action::FocusComposer, &ctx);
        assert!(app.focus_composer);
        assert!(!app.focus_search);
        assert_eq!(app.composer, "Unsent draft");
    }

    #[test]
    fn returning_to_a_conversation_refocuses_the_composer() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.open_chat = Some("1@s.whatsapp.net".into());
        app.page = Page::Settings;

        app.apply(Action::Open(Page::Chats), &ctx);

        assert!(app.focus_composer);
    }

    #[test]
    fn recreating_the_window_refocuses_the_composer() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.open_chat = Some("1@s.whatsapp.net".into());

        app.attach(&ctx);

        assert!(app.focus_composer);
    }

    #[test]
    fn returning_to_a_conversation_does_not_interrupt_search() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.open_chat = Some("1@s.whatsapp.net".into());
        app.page = Page::Settings;
        app.search = "ada".into();

        app.apply(Action::Open(Page::Chats), &ctx);

        assert!(!app.focus_composer);

        app.search.clear();
        ctx.memory_mut(|memory| memory.request_focus(egui::Id::new("chat-search")));
        app.dialog = Some(Dialog::About);
        app.apply(Action::CloseDialog, &ctx);
        assert!(!app.focus_composer);
    }

    #[test]
    fn names_fall_back_from_contacts_to_phones() {
        let mut app = app();
        app.contacts.insert(
            "1@s.whatsapp.net".into(),
            Contact {
                id: "1@s.whatsapp.net".into(),
                full_name: Some("Ada".into()),
                push_name: None,
            },
        );
        assert_eq!(app.display_name("1@s.whatsapp.net"), "Ada");
        assert_eq!(
            app.display_name("393331234567@s.whatsapp.net"),
            "+39 333 123 456 7"
        );
        assert_eq!(app.display_name("42@lid"), "Unknown");
        app.contacts.insert(
            "42@lid".into(),
            Contact {
                id: "42@lid".into(),
                full_name: None,
                push_name: Some("Bob".into()),
            },
        );
        assert_eq!(app.display_name("42@lid"), "~Bob");
        app.me = Some("42@lid".into());
        assert_eq!(app.display_name("42@lid"), "You");
    }
}

#[cfg(test)]
mod name_tests {
    use super::*;
    use crate::model::{Contact, Content, Delivery, MentionRef};

    fn app() -> App {
        let root = std::env::temp_dir().join(format!("zapfast-names-{}", std::process::id()));
        let (mut app, _events) = App::headless(AppDirs::under(&root), Settings::default());
        app.me = Some("15550001111@s.whatsapp.net".into());
        app.me_name = Some("Carmine".into());
        app.contacts.insert(
            "1@s.whatsapp.net".into(),
            Contact {
                id: "1@s.whatsapp.net".into(),
                full_name: Some("Ada Lovelace".into()),
                push_name: Some("Ada".into()),
            },
        );
        app.contacts.insert(
            "2@s.whatsapp.net".into(),
            Contact {
                id: "2@s.whatsapp.net".into(),
                full_name: None,
                push_name: Some("Bob".into()),
            },
        );
        app
    }

    #[test]
    fn the_setting_picks_the_source_and_the_other_fills_in() {
        let mut app = app();
        assert_eq!(app.display_name("1@s.whatsapp.net"), "Ada Lovelace");
        assert_eq!(app.display_name("2@s.whatsapp.net"), "~Bob");
        app.settings.names_from_contacts = false;
        assert_eq!(app.display_name("1@s.whatsapp.net"), "Ada");
        assert_eq!(app.display_name("2@s.whatsapp.net"), "Bob");
        assert_eq!(
            app.display_name_or("3@s.whatsapp.net", Some("Cy")),
            "Cy",
            "a name the message carried, for someone unknown"
        );
    }

    #[test]
    fn mentions_use_our_own_name_and_previews_resolve_tokens() {
        let app = app();
        assert_eq!(app.mention_name("15550001111@s.whatsapp.net"), "Carmine");
        assert_eq!(app.display_name("15550001111@s.whatsapp.net"), "You");
        assert_eq!(
            app.resolve_mention_tokens("palestra oggi? @15550001111 e @1 ?"),
            "palestra oggi? @Carmine e @1 ?",
            "a short number is not a mention"
        );
        let message = Message {
            id: "m".into(),
            chat: "1@s.whatsapp.net".into(),
            sender: "1@s.whatsapp.net".into(),
            sender_name: None,
            from_me: false,
            timestamp: 0,
            content: Content::text("ciao @15550001111"),
            status: Delivery::None,
            delivered_at: None,
            read_at: None,
            quoted: None,
            reactions: Vec::new(),
            edited: false,
            mentions: vec![MentionRef {
                user: "15550001111".into(),
                id: "15550001111@s.whatsapp.net".into(),
            }],
            forwarded: false,
            thumbnail: None,
        };
        assert_eq!(app.message_text(&message), "ciao @Carmine");
    }

    #[test]
    fn media_dir_changed_event_updates_settings_and_dirs() {
        let directory = tempfile::tempdir().unwrap();
        let (mut app, events) =
            App::headless(AppDirs::under(directory.path()), Settings::default());
        let ctx = egui::Context::default();
        assert_eq!(app.dirs.custom_media, None);
        assert_eq!(app.settings.custom_media_dir, None);

        let chat = "15550001111@s.whatsapp.net";
        let old_missing = directory.path().join("old_cache/photo.jpg");
        let message = Message {
            id: "m_img".into(),
            chat: chat.into(),
            sender: chat.into(),
            sender_name: None,
            from_me: false,
            timestamp: 0,
            content: Content::Image {
                caption: None,
                media: Media {
                    mime: "image/jpeg".into(),
                    size: 100,
                    width: None,
                    height: None,
                    path: Some(old_missing),
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
        };
        let conversation = Conversation {
            messages: vec![message],
            ..Default::default()
        };
        app.conversations.insert(chat.into(), conversation);

        let custom = directory.path().join("custom_downloads");
        std::fs::create_dir_all(&custom).unwrap();
        let replacement = custom.join("photo.jpg");
        std::fs::write(&replacement, b"image payload").unwrap();

        events
            .send(Event::MediaDirChanged(Some(custom.clone())))
            .unwrap();
        app.background_frame(&ctx);
        assert_eq!(app.dirs.custom_media, Some(custom.clone()));
        assert_eq!(app.settings.custom_media_dir, Some(custom));
        assert!(app.settings_dirty);

        let repaired = app.conversations.get(chat).unwrap();
        assert_eq!(
            repaired.messages[0]
                .content
                .media()
                .and_then(|m| m.path.as_ref()),
            Some(&replacement)
        );

        events.send(Event::MediaDirChanged(None)).unwrap();
        app.background_frame(&ctx);
        assert_eq!(app.dirs.custom_media, None);
        assert_eq!(app.settings.custom_media_dir, None);
    }
}
