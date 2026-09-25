//! Application state and the frame loop.
//!
//! Views queue [`Action`]s while drawing. The app applies them after the frame
//! and processes backend events.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::audio::{Player, Recorder};
use crate::backend::{Backend, Command, Event, LinkStatus, Refusal, Unsent, Waker};
use crate::i18n::Locale;
use crate::image_preview::PreviewState;
use crate::model::{
    Action, Chat, ChatFilter, ChatId, Contact, Content, Delivery, Dialog, Gif, GifError, Label,
    Media, MediaState, Message, Page, PickerTab, Scroll, SidebarDisplayMode, StickerPack,
    StickerShelf, Toast, ToastKind,
};
use crate::paths::AppDirs;
use crate::settings::{NotificationSound, Settings, ThemeChoice};
use crate::single_instance::{ControlCommand, Guard};
use crate::theme::{self, Palette};

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
/// How long an info toast stays, including its fade.
pub const INFO_TOAST_LIFETIME: Duration = Duration::from_millis(3200);
/// Error toasts kept on screen at once; older ones give way to newer ones.
const MAX_ERROR_TOASTS: usize = 3;
/// Typing-state timeout when no stop event arrives.
const TYPING_TIMEOUT: Duration = Duration::from_secs(12);
/// How long other apps' media stays paused while the next voice message of a
/// run downloads. A stalled download must not keep music paused for good.
const VOICE_FETCH_HOLD: Duration = Duration::from_secs(10);

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
    /// The height each row last took, keyed by message id, so the transcript
    /// can skip rows far from the viewport instead of laying them out.
    pub(crate) rows: HashMap<String, RowHeight>,
    /// The keyboard page, Home, or End scroll under way in the message list.
    pub(crate) key_scroll: Option<KeyScroll>,
}

/// A keyboard scroll the message list eases through, one instant step per
/// frame.
#[derive(Clone, Copy, Debug)]
pub(crate) struct KeyScroll {
    pub kind: Scroll,
    /// For PgUp/PgDn, the signed `scroll_with_delta` distance still to cover.
    pub remaining: f32,
    /// `ui.input().time` when it started.
    start: f64,
    duration: f32,
    /// The eased progress already applied, from 0 to 1.
    progress: f32,
}

impl KeyScroll {
    pub fn new(kind: Scroll, remaining: f32, start: f64, duration: f32) -> Self {
        Self {
            kind,
            remaining,
            start,
            duration,
            progress: 0.0,
        }
    }

    /// Advances the easing to `now` and returns the share of the distance
    /// still to cover that this frame moves: all of it once time is up. A
    /// pass redone in the same frame gets 0.
    pub fn advance(&mut self, now: f64) -> f32 {
        let t = ((now - self.start) / f64::from(self.duration)).clamp(0.0, 1.0) as f32;
        if t >= 1.0 || self.progress >= 1.0 {
            self.progress = 1.0;
            return 1.0;
        }
        // Ease out: a scroll a held key restarts still moves at once.
        let eased = egui::emath::easing::cubic_out(t);
        let fraction = (eased - self.progress) / (1.0 - self.progress);
        self.progress = eased;
        fraction
    }

    pub fn done(&self) -> bool {
        self.progress >= 1.0
    }
}

/// A transcript row's height as last laid out or estimated.
#[derive(Clone, Copy, Debug)]
pub(crate) struct RowHeight {
    pub height: f32,
    /// The egui pass the row was last laid out in; `None` for an estimate.
    pub pass: Option<u64>,
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
                    Some(existing) => {
                        // A reload or scroll delivers a freshly classified copy
                        // of an already-loaded message whose Media has no local
                        // path and a default state. Replacing it would throw
                        // away an in-flight download and re-fetch media already
                        // on disk, so keep the runtime-only fields (as
                        // `MessageUpdated` already does for the state).
                        let media = existing
                            .content
                            .media()
                            .map(|media| (media.state.clone(), media.path.clone()));
                        *existing = message;
                        // A copy that carries its own path is newer, for
                        // example after the archive relocated the file.
                        if let (Some((state, path)), Some(media)) =
                            (media, existing.content.media_mut())
                            && media.path.is_none()
                        {
                            media.state = state;
                            media.path = path;
                        }
                    }
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

/// The "unread messages" divider of the open chat. It stays until another
/// chat opens, like on the phone.
#[derive(Clone, Debug, PartialEq)]
pub struct UnreadDivider {
    pub chat: ChatId,
    /// Unread incoming messages when the chat was opened.
    pub count: u32,
    /// The transcript has scrolled to it once.
    pub placed: bool,
}

pub struct App {
    pub dirs: AppDirs,
    pub settings: Settings,
    /// Resolved interface language, from the setting or the system locale.
    pub locale: Locale,
    settings_dirty: bool,
    last_settings_save: Instant,
    pub backend: Backend,
    pub palette: Palette,
    pub custom_themes: theme::Catalog,
    applied_dark: Option<bool>,
    zoom_applied: bool,

    pub link: LinkStatus,
    /// Whether link-time history sync is active.
    pub syncing: bool,
    pub sync_percent: Option<u32>,
    pub me: Option<String>,
    /// Our privacy id (`@lid`), which mentions of us may carry instead.
    pub me_lid: Option<String>,
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
    /// The search pane beside the open chat: its query, day filter and the
    /// matches it lists.
    pub chat_search_open: bool,
    pub chat_search: String,
    /// Matches in the open chat, newest first.
    pub chat_search_hits: Vec<Message>,
    /// Whether the archive held more matches than the pane lists.
    pub chat_search_truncated: bool,
    /// Whether an answer for the current query and day is still due.
    pub chat_search_pending: bool,
    /// The result the arrow keys have reached, if any.
    pub chat_search_selected: Option<usize>,
    /// Local day the in-chat search is limited to, if any.
    pub chat_search_day: Option<jiff::civil::Date>,
    /// Month the day filter shows.
    pub chat_search_month: jiff::civil::Date,
    pub chat_search_calendar: bool,
    /// Whether the pane's field should take focus.
    pub focus_chat_search: bool,
    /// Whether the locked-chats folder is open.
    pub locked_folder: bool,
    /// The verifier authenticated for this window session, never the code.
    chat_lock_session: Option<String>,
    pub chat_lock_entry: String,
    pub chat_lock_confirm: String,
    pub chat_lock_error: bool,
    pub new_chat_search: String,
    /// Last answer from the slow code verifier, keyed by what was checked.
    chat_lock_check: std::cell::RefCell<Option<(String, Option<String>, bool)>>,
    /// Active typers and their latest event time by chat.
    pub typing: HashMap<ChatId, Vec<(String, Instant)>>,
    pub presence: HashMap<String, Presence>,
    /// Whether account privacy disables direct-chat read receipts.
    pub account_receipts_off: bool,
    /// Last account privacy snapshot from the phone.
    pub account_privacy: crate::privacy::Snapshot,
    /// Receipts of the message whose "Message info" is open.
    pub message_receipts: Option<crate::model::MessageReceipts>,
    /// The group message the backend is following receipts for.
    pub(crate) receipts_watch: Option<(ChatId, String)>,
    /// The group invite link being previewed or joined.
    pub invite: Option<crate::model::GroupInvite>,
    /// Where the unread messages began when the open chat was opened.
    pub unread_divider: Option<UnreadDivider>,
    /// Messages selected in a chat, in the chat's order.
    pub selection: Option<(ChatId, Vec<String>)>,
    /// The message a Shift-click range starts from.
    selection_anchor: Option<String>,
    avatars: HashMap<String, Option<PathBuf>>,
    avatar_requests: HashSet<String>,
    /// Full-size profile pictures for info dialogs.
    avatars_full: HashMap<String, Option<PathBuf>>,
    avatar_full_requests: HashSet<String>,
    /// Whether files are being dragged over the window.
    pub dropping: bool,
    /// A text paste already handled the clipboard before the shortcut release.
    paste_before_release: bool,
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
    /// Chats this account may pin; WhatsApp Plus raises it once known.
    pub pin_limit: usize,
    /// The reaction picker came from a message's context menu, which stays
    /// open beside it.
    pub reaction_beside_menu: bool,
    /// Demo/test: keep this message's context menu open.
    pub open_message_menu: Option<String>,
    /// Demo/test: keep this chat row's context menu open.
    #[cfg(any(test, feature = "demo"))]
    pub open_chat_menu: Option<ChatId>,
    /// Emoji-grid header to scroll into view.
    pub emoji_jump: Option<&'static str>,
    /// Attachments pending in the composer.
    pub pending: Vec<Pending>,
    /// Whether the composer plus menu is open or closing.
    pub composer_tools_open: bool,
    /// In-chat audio player.
    pub player: Player,
    /// In-chat video player.
    pub video: crate::video::Player,
    /// Chat of the loaded video; leaving it stops the video.
    video_chat: Option<ChatId>,
    /// Video to play once its download finishes.
    video_wanted: Option<(ChatId, String)>,
    /// Chat whose voice messages carry on into the next unheard one when a
    /// clip ends. Leaving the chat, or playing a video, ends the run.
    voice_chat: Option<ChatId>,
    /// Next voice message of a run, playing once its download finishes, and
    /// when the download began. Media stays paused for a short while meanwhile.
    voice_wanted: Option<(ChatId, String, Instant)>,
    /// Active voice recorder.
    pub recording: Option<Recorder>,
    /// A voice message the worker refused, kept with its chat so it can be
    /// sent again from that chat or discarded.
    pub(crate) unsent_voice: Option<(ChatId, Vec<f32>)>,
    /// Keeps other apps' music paused while recording or playing audio.
    media_hold: Option<crate::media_pause::Hold>,
    /// Only the real app pauses other apps' media, never tests or demos.
    pauses_media: bool,
    /// Image currently shown in the native preview.
    pub image_preview: Option<PreviewState>,
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
    /// Favorite stickers, newest first.
    pub stickers_saved: Vec<PathBuf>,
    /// Imported sticker packs, newest first.
    pub sticker_packs: Vec<StickerPack>,
    /// Whether the sticker list is loading.
    pub stickers_pending: bool,
    /// Whether a sticker pack import is active.
    pub sticker_import_pending: bool,
    /// signal.art link in the sticker tab.
    pub sticker_link: String,
    /// The list the sticker tab shows.
    pub sticker_shelf: StickerShelf,
    /// Text in the sticker search field.
    pub sticker_search: String,
    /// Emojis each listed sticker is tagged with.
    pub sticker_emojis: std::collections::HashMap<PathBuf, Vec<String>>,
    /// Text in the "New pack" field.
    pub sticker_pack_name: String,
    /// The picture being made into a sticker.
    pub sticker_draft: Option<crate::model::StickerDraft>,
    /// A pack shared in a chat, being viewed: the pack and its publisher.
    pub sticker_preview: Option<(StickerPack, String)>,
    /// Whether the viewed pack is still downloading.
    pub sticker_preview_pending: bool,
    /// A pack just created here, selected once the backend lists it.
    sticker_pack_created: Option<String>,
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
    pub interactive_sending: HashSet<(ChatId, String)>,
    /// Contact-name editor buffers.
    pub contact_edit: Option<(String, String)>,
    /// New-contact buffers and lookup state.
    pub new_contact_phone: String,
    pub new_contact_name: String,
    pub new_contact_last: String,
    pub new_contact_pending: bool,
    /// The new-contact dialog's "Save to phone" box.
    pub new_contact_to_phone: bool,
    /// Phone number entered for pairing.
    pub pair_phone: String,
    pub sidebar_visible: bool,
    pub show_archived: bool,
    /// Chat-list filter; applies to the main list, not to search or the archive.
    pub chat_filter: ChatFilter,
    /// Labels known here, in creation order. Local to this computer.
    pub labels: Vec<Label>,
    /// Name typed in the label manager.
    pub label_name: String,
    /// Colour the manager will use for the next label.
    pub label_color: String,
    /// Label being renamed, with the name being typed.
    pub label_editing: Option<(String, String)>,
    /// Label the chat list shows; `None` shows every chat.
    pub label_filter: Option<String>,
    /// Chats opened from the Unread list, kept there until the filter changes.
    unread_kept: HashSet<ChatId>,
    pub toasts: Vec<Toast>,
    pub actions: Vec<Action>,
    /// A newer release than this build, once GitHub has said so.
    pub update: Option<crate::updates::Release>,
    last_update_check: Option<Instant>,
    pub show_update: bool,
    pub update_download: crate::updates::DownloadState,
    pub update_support: Option<Result<crate::updates::Installation, String>>,
    update_inspecting: bool,
    pub update_arguments: Vec<String>,
    /// Whether to scroll the conversation to its newest message.
    pub scroll_to_bottom: bool,
    /// One-shot: an explicit jump (Ctrl+End, or the return-to-bottom button)
    /// must reach the bottom even while a message bubble retains keyboard
    /// focus, unlike `scroll_to_bottom` set for other reasons (opening a
    /// chat, sending a message), which still defers to that focus so it is
    /// not pulled out from under a keyboard-navigating reader.
    pub scroll_to_bottom_forced: bool,
    /// A pending page-relative scroll for the open chat, consumed by its
    /// message list on the next frame.
    pub scroll_page: Option<Scroll>,
    /// Whether the conversation was at the bottom last frame.
    pub at_bottom: bool,
    /// Message id to scroll into view.
    pub scroll_anchor: Option<String>,
    /// A message reached from a quote or a search result, which flashes
    /// once it is in view so the eye finds it.
    pub jump_highlight: Option<JumpHighlight>,
    pub focus_composer: bool,
    pub focus_search: bool,
    /// What the Settings page is filtered by.
    pub settings_search: String,
    pub focus_settings_search: bool,
    pub quit_requested: bool,
    pub window_focused: bool,
    /// Presence last reported to the backend.
    reported_online: Option<bool>,
    /// Whether ZapFast starts at login, when this installation supports it.
    pub start_with_system: Option<bool>,
    /// Cross-thread window repaint handle.
    waker: Waker,
    tray: Option<fastframe_tray::Tray>,
    /// Whether the app is running without a window.
    pub window_hidden: bool,
    /// Whether window close should keep the process running.
    pub hide_intent: bool,
    /// Whether a headless app should create a window.
    pub wants_show: bool,
    /// Requests received from later launches.
    control_commands: Option<std::sync::Arc<std::sync::Mutex<Vec<ControlCommand>>>>,
    /// Chats and messages from clicked notifications.
    notification_opens: std::sync::Arc<std::sync::Mutex<Vec<crate::notify::NotificationTarget>>>,
    notifications: crate::notify::Notifications,
    /// Unread count on the taskbar icon, where the desktop reads it. `None`
    /// for demo and test runs, which must not touch the real taskbar.
    badge: Option<crate::notify::Badge>,
}

/// A message that flashes after a jump to it, as WhatsApp does.
#[derive(Clone, Debug, PartialEq)]
pub struct JumpHighlight {
    pub chat: ChatId,
    pub message: String,
    /// When the message came into view, in egui's input time; `None` until
    /// then.
    pub since: Option<f64>,
}

impl JumpHighlight {
    /// How long the flash lasts, in seconds.
    pub const DURATION: f64 = 2.0;

    pub fn new(chat: ChatId, message: String) -> Self {
        Self {
            chat,
            message,
            since: None,
        }
    }

    /// The flash's strength `elapsed` seconds after the message came into
    /// view: a quick rise, a hold, then a fade to nothing.
    pub fn strength(elapsed: f64) -> f32 {
        const RISE: f64 = 0.15;
        const HOLD: f64 = 0.9;
        let strength = if elapsed < 0.0 {
            0.0
        } else if elapsed < RISE {
            elapsed / RISE
        } else if elapsed < HOLD {
            1.0
        } else {
            1.0 - (elapsed - HOLD) / (Self::DURATION - HOLD)
        };
        strength.clamp(0.0, 1.0) as f32
    }
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

/// The app outlives its window: closing it with "keep running" on hides
/// ZapFast, and the tray, a notification or another launch brings it back.
impl fastframe_shell::Resident for App {
    fn closed(&self) -> fastframe_shell::Closed {
        if !self.quit_requested && self.hide_intent {
            fastframe_shell::Closed::Hide
        } else {
            fastframe_shell::Closed::Quit
        }
    }

    fn window_gone(&mut self) {
        App::window_gone(self);
    }

    fn headless_frame(&mut self, ctx: &egui::Context) -> fastframe_shell::Headless {
        self.background_frame(ctx);
        if self.quit_requested {
            fastframe_shell::Headless::Quit
        } else if self.wants_show {
            fastframe_shell::Headless::Show
        } else {
            fastframe_shell::Headless::Wait
        }
    }

    /// Without a tray there is no way back to a hidden window, so show it.
    fn start_hidden(&mut self) -> bool {
        if !self.hides_to_tray() {
            return false;
        }
        App::start_hidden(self);
        true
    }

    fn shutdown(&mut self) {
        App::shutdown(self);
    }
}

const TRAY_SHOW: &str = "show";
const TRAY_QUIT: &str = "quit";

/// What a tray click asks for: a left click on Linux and macOS, or the menu's
/// first entry, toggles the window; a left click on Windows and a Dock click
/// on macOS show it.
fn tray_action(event: fastframe_tray::Event, window_hidden: bool) -> Option<Action> {
    use fastframe_tray::Event;
    Some(match event {
        Event::Show => Action::ShowWindow,
        Event::Toggle | Event::Menu(TRAY_SHOW) if window_hidden => Action::ShowWindow,
        Event::Toggle | Event::Menu(TRAY_SHOW) => Action::HideWindow,
        Event::Menu(TRAY_QUIT) => Action::Quit,
        Event::Menu(_) => return None,
    })
}

/// The tray item: ZapFast's icon, and a menu to show or hide the window and
/// to quit.
fn tray_config() -> fastframe_tray::Config {
    use fastframe_tray::MenuItem;
    fastframe_tray::Config {
        id: "zapfast",
        title: "ZapFast".into(),
        icon: crate::util::app_icon_rgba,
        template_icon: Some(crate::util::tray_template_rgba),
        menu: vec![
            MenuItem::action(TRAY_SHOW, "Show or hide ZapFast"),
            MenuItem::Separator,
            MenuItem::action(TRAY_QUIT, "Quit"),
        ],
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
    pub fn new(waker: &Waker, dirs: AppDirs, settings: Settings, options: AppOptions) -> Self {
        crate::proxy::configure(&settings.proxy);
        let backend = Backend::spawn(dirs.clone(), waker.clone());
        let mut app = Self::with_backend(dirs, settings, backend, waker.clone());
        app.pauses_media = true;
        app.badge = Some(Default::default());
        app.custom_themes
            .enable_desktop_themes(crate::theme::DESKTOP_THEMES);
        app.load_custom_themes();
        // Reading the desktop's font settings may wait on D-Bus; keep it off
        // the first frame.
        let text_waker = waker.clone();
        std::thread::Builder::new()
            .name("text-rendering".into())
            .spawn(move || crate::theme::follow_text_rendering(move || text_waker.wake()))
            .ok();
        if options.tray {
            let waker = waker.clone();
            app.tray = fastframe_tray::Tray::spawn(tray_config(), move || waker.wake());
        }
        // The clock preference may run a helper on Linux; keep it off the
        // first frame.
        std::thread::Builder::new()
            .name("clock-format".into())
            .spawn(|| {
                crate::util::twelve_hour_clock();
            })
            .ok();
        app.backend.send(Command::SetDownloadFolder(
            app.settings.download_folder.clone(),
        ));
        if crate::autostart::supported() {
            app.start_with_system = Some(crate::autostart::enabled());
        }
        app
    }

    /// Single-instance guard used by later launches.
    pub fn set_remote_control(&mut self, guard: &Guard) {
        self.control_commands = Some(guard.commands());
    }

    /// Creates a disconnected app and event sender for demos and tests.
    pub fn headless(dirs: AppDirs, settings: Settings) -> (Self, std::sync::mpsc::Sender<Event>) {
        let (backend, events) = Backend::detached();
        (
            Self::with_backend(dirs, settings, backend, Waker::default()),
            events,
        )
    }

    /// Starts without a window (`--start-hidden`). The link and the archive
    /// start now: they otherwise wait for a first frame, which a hidden start
    /// only draws once the tray or another launch shows the window.
    pub fn start_hidden(&mut self) {
        self.hide_intent = true;
        if let Some(startup) = self.backend.take_startup() {
            let _ = startup.send(());
        }
    }

    fn with_backend(dirs: AppDirs, settings: Settings, backend: Backend, waker: Waker) -> Self {
        let palette = settings
            .cached_palette()
            .unwrap_or_else(|| match settings.theme {
                ThemeChoice::Light => Palette::light(),
                _ => Palette::dark(),
            });
        let open_chat = settings.last_chat.clone();
        let locale = crate::i18n::resolve(settings.interface_language);
        let mut app = Self {
            dirs,
            settings,
            locale,
            settings_dirty: false,
            last_settings_save: Instant::now(),
            backend,
            palette,
            custom_themes: theme::Catalog::default(),
            applied_dark: None,
            zoom_applied: false,
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
            composer: String::new(),
            composer_mentions: Vec::new(),
            emoji_start: None,
            emoji_selected: 0,
            mention_start: None,
            mention_selected: 0,
            reply_to: None,
            editing: None,
            unsent_voice: None,
            composing: false,
            last_keystroke: None,
            search: String::new(),
            search_hits: Vec::new(),
            chat_search_open: false,
            chat_search: String::new(),
            chat_search_hits: Vec::new(),
            chat_search_truncated: false,
            chat_search_pending: false,
            chat_search_selected: None,
            chat_search_day: None,
            chat_search_month: crate::util::today(),
            chat_search_calendar: false,
            focus_chat_search: false,
            locked_folder: false,
            chat_lock_session: None,
            chat_lock_entry: String::new(),
            chat_lock_confirm: String::new(),
            chat_lock_error: false,
            new_chat_search: String::new(),
            chat_lock_check: Default::default(),
            typing: HashMap::new(),
            presence: HashMap::new(),
            account_receipts_off: false,
            account_privacy: crate::privacy::Snapshot::default(),
            message_receipts: None,
            receipts_watch: None,
            invite: None,
            unread_divider: None,
            selection: None,
            selection_anchor: None,
            avatars: HashMap::new(),
            avatar_requests: HashSet::new(),
            avatars_full: HashMap::new(),
            avatar_full_requests: HashSet::new(),
            dropping: false,
            paste_before_release: false,
            picker: None,
            picker_anchor: None,
            picker_search: String::new(),
            picker_focus: false,
            reaction_target: None,
            reaction_anchor: None,
            pin_limit: crate::backend::PINNED_CHATS,
            reaction_beside_menu: false,
            open_message_menu: None,
            #[cfg(any(test, feature = "demo"))]
            open_chat_menu: None,
            emoji_jump: None,
            pending: Vec::new(),
            composer_tools_open: false,
            player: Player::new(waker.clone()),
            video: crate::video::Player::new(waker.clone()),
            video_chat: None,
            video_wanted: None,
            voice_chat: None,
            voice_wanted: None,
            recording: None,
            media_hold: None,
            pauses_media: false,
            image_preview: None,
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
            sticker_shelf: StickerShelf::default(),
            sticker_search: String::new(),
            sticker_emojis: std::collections::HashMap::new(),
            sticker_pack_name: String::new(),
            sticker_pack_created: None,
            sticker_preview: None,
            sticker_preview_pending: false,
            sticker_draft: None,
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
            interactive_sending: HashSet::new(),
            contact_edit: None,
            new_contact_phone: String::new(),
            new_contact_name: String::new(),
            new_contact_last: String::new(),
            new_contact_pending: false,
            new_contact_to_phone: true,
            pair_phone: String::new(),
            sidebar_visible: true,
            show_archived: false,
            chat_filter: ChatFilter::All,
            labels: Vec::new(),
            label_name: String::new(),
            label_color: crate::archive::DEFAULT_COLOR.to_owned(),
            label_editing: None,
            label_filter: None,
            unread_kept: HashSet::new(),
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
            scroll_to_bottom_forced: false,
            scroll_page: None,
            at_bottom: true,
            scroll_anchor: None,
            jump_highlight: None,
            focus_composer: false,
            focus_search: false,
            settings_search: String::new(),
            focus_settings_search: false,
            quit_requested: false,
            window_focused: false,
            reported_online: None,
            start_with_system: None,
            waker,
            tray: None,
            window_hidden: false,
            hide_intent: false,
            wants_show: false,
            control_commands: None,
            notification_opens: Default::default(),
            notifications: Default::default(),
            badge: None,
        };
        // A hand-edited speed snaps to a supported one, so a speed control
        // always shows the speed that plays.
        app.settings.voice_speed = app.player.set_speed(app.settings.voice_speed);
        app
    }

    /// Updates the linked app while no window exists.
    pub fn window_gone(&mut self) {
        self.flush_open_draft();
        self.clear_chat_lock_entry();
        if self.dialog == Some(Dialog::UnlockLockedChats) {
            self.dialog = None;
        }
        if self.locked_folder || self.secret_code_matched() {
            self.close_locked_folder();
            self.search.clear();
            self.search_hits.clear();
        }
        self.window_hidden = true;
        self.window_focused = false;
        self.hide_intent = false;
        self.wants_show = false;
    }

    /// Whether window close keeps the app in the tray.
    pub fn hides_to_tray(&self) -> bool {
        self.tray.is_some() && self.settings.keep_running_in_background
    }

    fn handle_tray(&mut self) {
        let Some(events) = self.tray.as_ref().map(fastframe_tray::Tray::events) else {
            return;
        };
        let hidden = self.window_hidden;
        self.actions.extend(
            events
                .into_iter()
                .filter_map(|event| tray_action(event, hidden)),
        );
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
                ControlCommand::Ping => {}
            }
        }
    }

    /// Opens the messages announced by clicked notifications, creating a
    /// window when needed. The click carries the message, so the reader lands
    /// on what the notification showed, not on the end of the chat.
    fn handle_notification_opens(&mut self) {
        let opened: Vec<crate::notify::NotificationTarget> = std::mem::take(
            &mut *self
                .notification_opens
                .lock()
                .unwrap_or_else(|p| p.into_inner()),
        );
        for target in opened {
            self.actions.push(Action::OpenMessage {
                chat: target.chat,
                message: target.message,
            });
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
        if !notification_eligible(chat, now, message.timestamp) {
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
        let chat_sound = chat.notification_sound.clone();
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
        let for_us = is_group && self.addresses_us(message);
        let sound = notification_sound(&self.settings, chat_sound, is_group, for_us);
        self.notifications.show(
            title,
            body,
            picture,
            sound,
            crate::notify::NotificationTarget {
                chat: chat_id.to_owned(),
                message: message.id.clone(),
            },
            std::sync::Arc::clone(&self.notification_opens),
            move || waker.wake(),
        );
    }

    /// Every id a member list may name us by: the phone number and, before
    /// the worker knows the pair, the privacy id.
    pub fn our_ids(&self) -> Vec<&str> {
        [self.me.as_deref(), self.me_lid.as_deref()]
            .into_iter()
            .flatten()
            .collect()
    }

    /// Whether a message mentions us or replies to one of our messages. A
    /// mention may name us by phone number or by privacy id; the worker files
    /// both under the phone number once it knows the pair, and the raw token
    /// keeps the privacy id recognisable before then.
    fn addresses_us(&self, message: &Message) -> bool {
        let ours: Vec<&str> = [self.me.as_deref(), self.me_lid.as_deref()]
            .into_iter()
            .flatten()
            .collect();
        let is_us = |id: &str| ours.contains(&id);
        let mentioned = message.mentions.iter().any(|mention| {
            is_us(&mention.id)
                || ours
                    .iter()
                    .any(|id| id.split('@').next() == Some(mention.user.as_str()))
        });
        mentioned
            || message
                .quoted
                .as_ref()
                .is_some_and(|quoted| is_us(&quoted.sender))
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
        // A keystroke that wraps the draft is applied in one pass, and the
        // bottom panel holding the composer only takes the new height in
        // the next: three passes keep it from showing a frame out of place.
        ctx.options_mut(|options| options.max_passes = std::num::NonZeroUsize::new(3).unwrap());
        // Load and index the color emoji font outside the frame loop.
        std::thread::Builder::new()
            .name("emoji-font".into())
            .spawn(crate::emoji::warm_up)
            .ok();
        self.applied_dark = None;
        self.zoom_applied = false;
        self.window_hidden = false;
        self.paste_before_release = false;
        self.hide_intent = false;
        self.wants_show = false;
        self.refocus_composer(ctx);
        // Before the tray: muda keeps the first menu handler it is given, and
        // the tray installs one when it makes its item on the first window.
        // Ours hands the tray its own events (#215).
        #[cfg(target_os = "macos")]
        crate::macos::attach(ctx);
        if let Some(tray) = &mut self.tray {
            tray.attach();
        }
    }

    pub fn is_connected(&self) -> bool {
        self.link.is_connected()
    }

    /// How the chat list is drawn right now. Hiding it leaves a column of
    /// avatars with unread badges.
    pub fn sidebar_mode(&self) -> SidebarDisplayMode {
        if self.sidebar_visible {
            SidebarDisplayMode::Expanded
        } else {
            SidebarDisplayMode::CollapsedIconsOnly
        }
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

    /// Drops every trace of a chat that no longer exists. Unlike hiding a
    /// locked chat this discards the draft, because there is nothing left to
    /// send it to, and it clears `last_chat` so a restart does not reopen it.
    fn forget_chat(&mut self, id: &str) {
        self.leave_chat(id);
        self.chats.retain(|chat| chat.id != id);
        self.conversations.remove(id);
        self.drafts.remove(id);
        self.draft_mentions.remove(id);
        self.typing.remove(id);
        self.unread_kept.remove(id);
        if self.scroll_chat_into_view.as_deref() == Some(id) {
            self.scroll_chat_into_view = None;
        }
        if self.settings.last_chat.as_deref() == Some(id) {
            self.settings.last_chat = None;
        }
    }

    /// Takes everything on screen away from a chat the user can no longer
    /// reach, whether it was locked or deleted: its notifications, search
    /// hits and dialogs, and, when it is open, the conversation with its
    /// composer, recording and playback. Keeping a draft is up to the caller.
    fn leave_chat(&mut self, id: &str) {
        self.notifications.clear(id);
        self.search_hits.retain(|message| message.chat != id);
        if matches!(
            &self.dialog,
            Some(
                Dialog::ChatInfo(chat) | Dialog::CreatePoll(chat) | Dialog::ConfirmDeleteChat(chat)
            ) if chat == id
        ) || matches!(&self.dialog, Some(Dialog::Forward { chat, .. }) if chat == id)
        {
            self.dialog = None;
            self.poll_creating = false;
        }
        if self.open_chat.as_deref() == Some(id) {
            self.stop_composing(id);
            self.open_chat = None;
            self.composer.clear();
            self.composer_mentions.clear();
            self.pending.clear();
            self.reply_to = None;
            self.editing = None;
            self.picker = None;
            self.reaction_target = None;
            self.reaction_anchor = None;
            self.emoji_start = None;
            self.mention_start = None;
            self.emoji_jump = None;
            self.dialog = None;
            self.recording = None;
            self.player.stop();
        }
    }

    pub fn current_chat(&self) -> Option<&Chat> {
        self.open_chat
            .as_deref()
            .and_then(|id| self.chat(id))
            .filter(|chat| !chat.locked || self.locked_folder_open())
    }

    /// The label with this id, when it still exists.
    pub fn label(&self, id: &str) -> Option<&Label> {
        self.labels.iter().find(|label| label.id == id)
    }

    /// Whether a chat wears a label id.
    pub fn chat_wears(&self, chat: &Chat, label: &str) -> bool {
        chat.labels.iter().any(|worn| worn == label)
    }

    /// Unread chats wearing a label, counted the way the other chips count.
    ///
    /// Archived and locked chats are left out, because the label's chip does
    /// not list them.
    pub fn label_unread(&self, id: &str) -> usize {
        self.chats
            .iter()
            .filter(|chat| {
                !chat.archived && !chat.locked && chat.unread > 0 && self.chat_wears(chat, id)
            })
            .count()
    }

    /// Drops filters, edits and chat labels that point at labels which are
    /// gone, so a deleted label leaves the list before the chats reload.
    fn prune_labels(&mut self) {
        let known: Vec<String> = self.labels.iter().map(|label| label.id.clone()).collect();
        for chat in &mut self.chats {
            chat.labels.retain(|id| known.contains(id));
        }
        if self
            .label_filter
            .as_ref()
            .is_some_and(|id| !known.contains(id))
        {
            self.label_filter = None;
        }
        if self
            .label_editing
            .as_ref()
            .is_some_and(|(id, _)| !known.contains(id))
        {
            self.label_editing = None;
        }
    }

    /// Picks the label the chat list shows. A label is one more chip in the
    /// filter row, so it replaces the chip filter, and leaves the archive and
    /// the locked folder the way the other chips do.
    fn select_label(&mut self, label: Option<String>) {
        if self.locked_folder {
            self.close_locked_folder();
            self.search.clear();
            self.search_hits.clear();
        }
        self.label_filter = label;
        self.chat_filter = ChatFilter::All;
        self.show_archived = false;
        self.unread_kept.clear();
    }

    /// Why a label name cannot be used, if it cannot. `except` is the label
    /// being renamed, which may keep its own name.
    fn label_name_refusal(&self, name: &str, except: Option<&str>) -> Option<String> {
        let taken = self.labels.iter().any(|label| {
            Some(label.id.as_str()) != except && label.name.to_lowercase() == name.to_lowercase()
        });
        taken.then(|| {
            crate::i18n::gettext(self.locale, "A label named “{name}” already exists.")
                .replace("{name}", name)
        })
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
        if chat.is_group()
            && (chat.name.trim().is_empty() || (chat.name == "Group" && !chat.group_subject_known))
        {
            let participants = self.participant_names(chat);
            return if participants.is_empty() {
                "Group".to_owned()
            } else {
                participants
            };
        }
        if self.me.as_deref() == Some(chat.id.as_str()) {
            return self.self_title();
        }
        if chat.is_group() {
            return chat.name.clone();
        }
        self.person_name(&chat.id, None)
    }

    /// Our own chat's title, "Name (You)" as on the phone, so searching for
    /// our own name finds it.
    pub fn self_title(&self) -> String {
        match self
            .me_name
            .as_deref()
            .filter(|name| !name.trim().is_empty())
        {
            Some(name) => crate::i18n::gettext(self.locale, "{name} (You)").replace("{name}", name),
            None => crate::i18n::gettext(self.locale, "You").into_owned(),
        }
    }

    /// Whether a contact search for `needle`, already a search key, should
    /// offer the chat with ourselves: by our name, our number, "You" or
    /// "Message yourself", in English or the interface language.
    pub fn offers_self(&self, needle: &str) -> bool {
        let Some(me) = self.me.as_deref() else {
            return false;
        };
        // A locked chat stays out of every list but the locked one.
        if self.chat(me).is_some_and(|chat| chat.locked) {
            return false;
        }
        if needle.is_empty() {
            return true;
        }
        let yourself = crate::i18n::gettext(self.locale, "Message yourself");
        let you = crate::i18n::gettext(self.locale, "You");
        [
            self.self_title().as_str(),
            yourself.as_ref(),
            you.as_ref(),
            "Message yourself",
        ]
        .iter()
        .any(|text| crate::util::search_key(text).contains(needle))
            || crate::model::phone_of(me).is_some_and(|phone| phone.contains(needle))
    }

    fn person_name(&self, id: &str, hint: Option<&str>) -> String {
        let contact = self.contacts.get(id);
        let present = |name: Option<&str>| name.filter(|name| !name.is_empty()).map(str::to_owned);
        let saved = present(contact.and_then(|contact| contact.full_name.as_deref()));
        let called = present(contact.and_then(|contact| contact.push_name.as_deref()))
            .or_else(|| present(hint));
        // Saved names first, as WhatsApp does; a profile name wears a tilde.
        if let Some(name) = saved.or_else(|| called.map(|name| format!("~{name}"))) {
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
            Content::Text { text, .. } | Content::Interactive { text, .. } => {
                crate::markup::plain(text, &self.mention_list(message))
            }
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
        let mut seen = HashSet::new();
        for id in chat
            .participants
            .iter()
            .filter(|id| Some(id.as_str()) != me && seen.insert(id.as_str()))
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
        let mut counted = Vec::new();
        let mut iter = names.into_iter().peekable();
        while let Some(name) = iter.next() {
            let mut count = 1;
            while iter
                .peek()
                .is_some_and(|next| next.to_lowercase() == name.to_lowercase())
            {
                iter.next();
                count += 1;
            }
            counted.push(if count == 1 {
                name
            } else {
                format!("{name} x{count}")
            });
        }
        numbers.sort();
        counted.extend(numbers);
        if chat.participants.iter().any(|id| Some(id.as_str()) == me) {
            counted.push("You".to_owned());
        }
        counted.join(", ")
    }

    /// Whether the typed search text is the secret code that reveals the
    /// locked-chats folder.
    pub fn secret_code_matched(&self) -> bool {
        // Verifying runs a slow KDF, and this is read every frame, so the
        // answer is kept until the typed text or the stored verifier changes.
        let code = self.search.trim();
        let stored = &self.settings.chat_lock_code_hash;
        let mut cached = self.chat_lock_check.borrow_mut();
        if let Some((checked, against, matched)) = cached.as_ref()
            && checked == code
            && against == stored
        {
            return *matched;
        }
        let matched = self.settings.verifies_chat_lock_code(code);
        *cached = Some((code.to_owned(), stored.clone(), matched));
        matched
    }

    fn chat_lock_authenticated(&self) -> bool {
        self.chat_lock_session.is_some()
            && self.chat_lock_session == self.settings.chat_lock_code_hash
    }

    /// Search-code entry is retained for compatibility; the Locked tab uses
    /// a window-scoped session so the search field remains useful.
    pub fn locked_folder_open(&self) -> bool {
        self.locked_folder && (self.chat_lock_authenticated() || self.secret_code_matched())
    }

    pub fn locked_count(&self) -> usize {
        self.chats.iter().filter(|chat| chat.locked).count()
    }

    pub fn should_show_chat_lock_hint(&self) -> bool {
        self.locked_count() > 0
            && self.settings.chat_lock_code_hash.is_none()
            && !self.settings.chat_lock_hint_dismissed
    }

    /// Visible chats filtered by search, archive state, and the chat filter,
    /// with pinned first.
    /// Locked chats only appear inside the locked folder.
    pub fn visible_chats(&self) -> Vec<&Chat> {
        let needle = crate::util::search_key(self.search.trim());
        let locked = self.locked_folder_open();
        let filtering = !locked && needle.is_empty() && !self.show_archived;
        let mut chats: Vec<&Chat> = self
            .chats
            .iter()
            .filter(|chat| chat.locked == locked)
            .filter(|chat| locked || chat.archived == self.show_archived || !needle.is_empty())
            .filter(|chat| match &self.label_filter {
                // A label lists every chat wearing it, channels included,
                // because someone put each of them there.
                _ if !filtering => true,
                Some(label) => self.chat_wears(chat, label),
                None => {
                    self.chat_filter.matches(chat)
                        || (self.chat_filter == ChatFilter::Unread
                            && self.unread_kept.contains(&chat.id))
                }
            })
            .filter(|chat| {
                // Inside the locked folder the typed text is the secret code,
                // not a query to match.
                (self.locked_folder && !self.chat_lock_authenticated())
                    || needle.is_empty()
                    || crate::util::search_key(&self.chat_title(chat)).contains(&needle)
                    || chat.phone().is_some_and(|phone| phone.contains(&needle))
                    || chat.last.as_ref().is_some_and(|last| {
                        crate::util::search_key(&last.summary).contains(&needle)
                    })
            })
            .collect();
        // The Favorites chip keeps the phone's order below the pinned chats.
        let favorites_order =
            filtering && self.label_filter.is_none() && self.chat_filter == ChatFilter::Favorites;
        chats.sort_by(|a, b| {
            b.pinned.cmp(&a.pinned).then_with(|| {
                if a.pinned && b.pinned {
                    b.pinned_at.cmp(&a.pinned_at).then(a.id.cmp(&b.id))
                } else if favorites_order {
                    a.favorite_position
                        .cmp(&b.favorite_position)
                        .then(a.id.cmp(&b.id))
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

    /// Archived chats with unread messages, for the Archived chip.
    pub fn archived_unread(&self) -> usize {
        self.chats
            .iter()
            .filter(|chat| chat.archived && !chat.locked && chat.unread > 0)
            .count()
    }

    pub fn archived_count(&self) -> usize {
        self.chats
            .iter()
            .filter(|chat| chat.archived && !chat.locked)
            .count()
    }

    /// Unarchived chats with unread messages that a filter would list.
    pub fn unread_chats(&self, filter: ChatFilter) -> usize {
        self.chats
            .iter()
            .filter(|chat| {
                !chat.archived && !chat.locked && chat.looks_unread() && filter.matches(chat)
            })
            .count()
    }

    pub fn unread_total(&self) -> u32 {
        self.chats
            .iter()
            .filter(|chat| !chat.archived && !chat.locked && !chat.muted(crate::util::now()))
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
                Event::Me {
                    id,
                    lid,
                    name,
                    about,
                } => {
                    self.me = Some(id);
                    self.me_lid = lid;
                    self.me_name = name;
                    self.me_about = about;
                }
                Event::Labels(labels) => {
                    self.labels = labels;
                    self.prune_labels();
                }
                Event::Drafts(drafts) => {
                    // Unsent text stored by an earlier session. Text typed in
                    // this session wins over the stored copy.
                    for (chat, text) in drafts {
                        // The chat reopened at startup shows its draft at once.
                        if self.open_chat.as_deref() == Some(chat.as_str())
                            && self.editing.is_none()
                            && self.composer.is_empty()
                        {
                            self.composer = text;
                        } else {
                            self.drafts.entry(chat).or_insert(text);
                        }
                    }
                }
                Event::Chats(chats) => {
                    for chat in &chats {
                        if chat.unread == 0 {
                            self.notifications.clear(&chat.id);
                        }
                    }
                    self.chats = chats;
                    if let Some(open) = self.open_chat.clone() {
                        if self.chat(&open).is_none_or(|chat| chat.locked) {
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
                Event::ChatHits {
                    chat,
                    query,
                    from,
                    until,
                    messages,
                    truncated,
                } => {
                    // Hits for another chat, for a query the user has already
                    // replaced, or for a day they have already moved off,
                    // arrive too late to matter.
                    let (want_from, want_until) = self.chat_search_range();
                    if self.open_chat.as_deref() == Some(chat.as_str())
                        && query == self.chat_search.trim()
                        && from == want_from
                        && until == want_until
                    {
                        self.chat_search_hits = messages;
                        self.chat_search_truncated = truncated;
                        self.chat_search_pending = false;
                        self.chat_search_selected = None;
                    }
                }
                Event::SearchHits { query, messages } => {
                    if query == self.search.trim() {
                        // Locked chats' messages stay out of plain search.
                        self.search_hits = messages
                            .into_iter()
                            .filter(|message| {
                                self.chat(&message.chat).is_none_or(|chat| !chat.locked)
                            })
                            .collect();
                    }
                }
                Event::Incoming { chat, message } => self.maybe_notify(&chat, &message),
                Event::Picked { chat, paths } => {
                    if self.open_chat.as_deref() == Some(chat.as_str()) {
                        self.stage_files(paths);
                    }
                }
                Event::InteractiveReplyState {
                    chat,
                    message,
                    pending,
                } => {
                    if pending {
                        self.interactive_sending.insert((chat, message));
                    } else {
                        self.interactive_sending.remove(&(chat, message));
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
                        let carousel_states = match &existing.content {
                            Content::Interactive {
                                card: Some(card), ..
                            } => card
                                .carousel
                                .iter()
                                .map(|card| card.image.as_ref().map(|media| media.state.clone()))
                                .collect::<Vec<_>>(),
                            _ => Vec::new(),
                        };
                        *existing = message;
                        for (index, state) in carousel_states.into_iter().enumerate() {
                            if let (Some(state), Some(media)) =
                                (state, existing.content.media_at_mut(Some(index)))
                            {
                                media.state = state;
                            }
                        }
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
                    favorites,
                    packs,
                    recent,
                    emojis,
                } => {
                    self.stickers_saved = favorites;
                    self.sticker_packs = packs;
                    self.stickers = recent;
                    self.sticker_emojis = emojis;
                    // Show a pack made here as soon as it exists. Packs list
                    // newest first, so the first match is the new one.
                    if let Some(name) = self.sticker_pack_created.take() {
                        match self
                            .sticker_packs
                            .iter()
                            .find(|pack| pack.local && pack.name == name)
                        {
                            Some(pack) => self.sticker_shelf = StickerShelf::Pack(pack.dir.clone()),
                            None => self.sticker_pack_created = Some(name),
                        }
                    }
                    // A pack deleted on another surface cannot stay selected.
                    if matches!(self.sticker_shelf, StickerShelf::Pack(_))
                        && self.selected_pack().is_none()
                    {
                        self.sticker_shelf = StickerShelf::Recent;
                    }
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
                Event::ChatRemoved { chat } => self.forget_chat(&chat),
                Event::ChatCleared { chat, through } => self.handle_chat_cleared(&chat, through),
                Event::Media {
                    card,
                    chat,
                    message,
                    result,
                } => self.handle_media(&chat, &message, card, result),
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
                Event::AccountPrivacy { values, failed } => {
                    self.account_privacy.apply_fetch(values, failed);
                    // The account value wins over the local switch: it is what
                    // the phone and the other linked devices enforce.
                    if let Some(choice) = self
                        .account_privacy
                        .get(crate::privacy::PrivacyKind::ReadReceipts)
                    {
                        self.account_receipts_off =
                            choice != crate::privacy::PrivacyChoice::Everyone;
                    }
                }
                Event::AccountPrivacySaved { kind } => {
                    // A confirmation nothing waits for belongs to an account
                    // that has since been unlinked.
                    if self.account_privacy.finish_set(kind)
                        && kind == crate::privacy::PrivacyKind::ReadReceipts
                    {
                        self.account_receipts_off = self.account_privacy.get(kind)
                            != Some(crate::privacy::PrivacyChoice::Everyone);
                    }
                }
                Event::AccountPrivacyFailed { kind } => self.account_privacy.fail_set(kind),
                Event::PinLimit(limit) => self.pin_limit = limit,
                Event::Receipts(receipts) => {
                    // A late answer for a dialog that has since closed is stale.
                    if self.receipts_watch.as_ref().is_some_and(|(chat, message)| {
                        *chat == receipts.chat && *message == receipts.message
                    }) {
                        self.message_receipts = Some(receipts);
                    }
                }
                Event::ChatSoundPicked { chat, path } => {
                    crate::notify::play_sound(crate::settings::NotificationSound::Custom(
                        path.clone(),
                    ));
                    self.actions.push(Action::SetChatSound {
                        chat,
                        sound: Some(crate::settings::NotificationSound::Custom(path)),
                    });
                }
                Event::DownloadFolderPicked(path) => {
                    self.actions.push(Action::SetDownloadFolder(Some(path)));
                }
                Event::NotificationSoundPicked { mention, path } => {
                    crate::notify::play_sound(crate::settings::NotificationSound::Custom(
                        path.clone(),
                    ));
                    self.actions.push(Action::SetNotificationSound {
                        mention,
                        sound: crate::settings::NotificationSound::Custom(path),
                    });
                }
                Event::InvitePreview { code, result } => {
                    use crate::model::InviteState;
                    if let Some(invite) = self.invite.as_mut().filter(|invite| invite.code == code)
                    {
                        invite.state = match result {
                            Ok(info) => InviteState::Ready(info),
                            Err(error) => InviteState::Failed(error),
                        };
                    }
                }
                Event::InviteJoined { code, result } => {
                    if self
                        .invite
                        .as_ref()
                        .is_some_and(|invite| invite.code == code)
                    {
                        match result {
                            Ok((id, pending)) => {
                                self.invite = None;
                                if self.dialog == Some(Dialog::JoinGroup) {
                                    self.dialog = None;
                                }
                                if pending {
                                    self.toast(
                                        "Request sent. An admin must approve it before you join.",
                                    );
                                } else {
                                    self.actions.push(Action::OpenChat(id));
                                }
                            }
                            Err(error) => {
                                if let Some(invite) = self.invite.as_mut() {
                                    invite.state = crate::model::InviteState::Failed(error);
                                }
                            }
                        }
                    }
                }
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
                Event::ClipboardImage(result) => {
                    self.handle_clipboard_image(result, write_clipboard_image);
                }
                Event::StickerPicture {
                    path,
                    width,
                    height,
                    transparent,
                } => {
                    self.sticker_draft = Some(crate::model::StickerDraft {
                        source: path,
                        width,
                        height,
                        transparent,
                        crop: crate::model::StickerCrop::centered(width, height),
                        keep_transparent: transparent,
                        emojis: String::new(),
                    });
                    self.picker = None;
                    self.dialog = Some(Dialog::StickerMaker);
                }
                Event::StickerPackPreview(result) => {
                    self.sticker_preview_pending = false;
                    match result {
                        Ok(preview) => self.sticker_preview = Some(preview),
                        Err(error) => {
                            if self.dialog == Some(Dialog::StickerPack) {
                                self.dialog = None;
                            }
                            self.toast_error(error);
                        }
                    }
                }
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
                Event::SendRefused {
                    chat,
                    quoting,
                    unsent,
                    reason,
                } => self.send_refused(chat, quoting, unsent, reason),
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
                self.interactive_sending.clear();
                self.poll_creating = false;
                self.poll_draft = Default::default();
                self.notifications.clear_all();
                self.chats.clear();
                self.conversations.clear();
                self.contacts.clear();
                self.avatars.clear();
                self.account_privacy = crate::privacy::Snapshot::default();
                self.account_receipts_off = false;
                self.open_chat = None;
                // Unsent text belongs to the account that was unlinked.
                self.drafts.clear();
                self.draft_mentions.clear();
                self.composer.clear();
                self.composer_mentions.clear();
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
        if is_open
            && (!chat.locked || self.locked_folder_open())
            && chat.unread > 0
            && self.window_focused
            && !self.window_hidden
        {
            chat.unread = 0;
            self.mark_read(&chat.id);
        }
        // Inside the authenticated folder the chat stays open; otherwise a
        // lock closes and clears everything it left behind.
        if chat.locked && !self.locked_folder_open() {
            self.hide_locked_chat(&chat.id);
        }
        // Leaving with "and archive" archives the chat once the phone agreed,
        // so this is where the open conversation closes, not before. Only a
        // chat that just became archived: a message arriving in one that was
        // already archived does not close it.
        if is_open && chat.archived && self.chat(&chat.id).is_none_or(|known| !known.archived) {
            self.actions.push(Action::CloseChat);
        }
        match self.chats.iter_mut().find(|known| known.id == chat.id) {
            Some(existing) => *existing = chat,
            None => self.chats.push(chat),
        }
        self.chats
            .sort_by_key(|chat| std::cmp::Reverse(chat.last_activity));
    }

    fn close_locked_folder(&mut self) {
        self.locked_folder = false;
        self.chat_lock_session = None;
        self.clear_chat_lock_entry();
        self.chat_lock_check.borrow_mut().take();
        if let Some(id) = self.open_chat.clone()
            && self.chat(&id).is_some_and(|chat| chat.locked)
        {
            self.hide_locked_chat(&id);
        }
    }

    fn clear_chat_lock_entry(&mut self) {
        self.chat_lock_entry.clear();
        self.chat_lock_confirm.clear();
        self.chat_lock_error = false;
    }

    fn enter_locked_folder(&mut self) {
        self.chat_lock_session = self.settings.chat_lock_code_hash.clone();
        self.locked_folder = true;
        self.search.clear();
        self.search_hits.clear();
        self.chat_lock_check.borrow_mut().take();
        self.clear_chat_lock_entry();
        self.dialog = None;
        self.chat_filter = ChatFilter::All;
        self.show_archived = false;
        self.page = Page::Chats;
        self.sidebar_visible = true;
    }

    /// Empties a chat that stays listed. Search hits and anything pointing at
    /// one of its messages would otherwise refer to rows that are gone, and a
    /// pending edit would send `EditText` for a message that no longer exists.
    fn handle_chat_cleared(&mut self, id: &str, through: i64) {
        self.notifications.clear(id);
        // Clearing a chat also removes its stored draft.
        self.drafts.remove(id);
        self.draft_mentions.remove(id);
        self.search_hits
            .retain(|message| message.chat != id || message.timestamp > through);
        // Nothing earlier is left here, and the phone no longer has it either.
        let conversation = self.conversations.entry(id.to_owned()).or_default();
        conversation
            .messages
            .retain(|message| message.timestamp > through);
        conversation.requested = true;
        conversation.complete = true;
        conversation.phone_exhausted = true;
        conversation.loading_older = false;
        if self.open_chat.as_deref() == Some(id) {
            if self
                .editing
                .as_ref()
                .is_some_and(|id| conversation.message(id).is_none())
            {
                self.editing = None;
                self.composer.clear();
                self.composer_mentions.clear();
            }
            if self
                .reply_to
                .as_ref()
                .is_some_and(|id| conversation.message(id).is_none())
            {
                self.reply_to = None;
            }
            if self
                .reaction_target
                .as_ref()
                .is_some_and(|(_, id)| conversation.message(id).is_none())
            {
                self.reaction_target = None;
                self.reaction_anchor = None;
                self.picker = None;
            }
        }
    }

    /// The pack the sticker tab shows, when it still exists.
    pub fn selected_pack(&self) -> Option<&StickerPack> {
        let StickerShelf::Pack(dir) = &self.sticker_shelf else {
            return None;
        };
        self.sticker_packs.iter().find(|pack| pack.dir == *dir)
    }

    /// Whether the search pane is on screen: it belongs to the open chat and
    /// only the chat view shows it.
    pub fn chat_search_visible(&self) -> bool {
        self.chat_search_open && self.page == Page::Chats && self.open_chat.is_some()
    }

    /// Opens the search pane beside the open chat, or focuses its field when
    /// it is already open.
    fn open_chat_search(&mut self) {
        if self.open_chat.is_none() || self.page != Page::Chats {
            return;
        }
        if !self.chat_search_open {
            self.chat_search_open = true;
            self.chat_search_month = crate::util::today();
        }
        self.focus_chat_search = true;
        self.focus_search = false;
        self.focus_composer = false;
    }

    fn close_chat_search(&mut self) {
        self.chat_search_open = false;
        self.chat_search.clear();
        self.chat_search_hits.clear();
        self.chat_search_truncated = false;
        self.chat_search_pending = false;
        self.chat_search_selected = None;
        self.chat_search_day = None;
        self.chat_search_calendar = false;
        self.focus_chat_search = false;
    }

    /// The Unix-second range the day filter stands for, if a day is picked.
    fn chat_search_range(&self) -> (Option<i64>, Option<i64>) {
        match self.chat_search_day.and_then(crate::util::day_bounds) {
            Some((from, until)) => (Some(from), Some(until)),
            None => (None, None),
        }
    }

    /// Asks for the open chat's matches, for the query and day in force.
    fn request_chat_search(&mut self) {
        self.chat_search_selected = None;
        let query = self.chat_search.trim().to_owned();
        let (from, until) = self.chat_search_range();
        let Some(chat) = self
            .open_chat
            .clone()
            .filter(|_| !query.is_empty() || from.is_some())
        else {
            self.chat_search_hits.clear();
            self.chat_search_truncated = false;
            self.chat_search_pending = false;
            return;
        };
        // The earlier matches stay listed until the answer replaces them, so
        // the list does not blink empty on every keystroke.
        self.chat_search_pending = true;
        self.backend.send(Command::SearchChatMessages {
            chat,
            query,
            from,
            until,
        });
    }

    fn hide_locked_chat(&mut self, id: &str) {
        // A locked chat still exists, so its unsent text waits as a draft.
        // Text emptied in the composer clears the stored copy too.
        if self.open_chat.as_deref() == Some(id)
            && self.editing.is_none()
            && self.composer.is_empty()
        {
            self.store_draft(id, "");
        }
        if self.open_chat.as_deref() == Some(id)
            && self.editing.is_none()
            && !self.composer.is_empty()
        {
            self.drafts
                .insert(id.to_owned(), std::mem::take(&mut self.composer));
            self.draft_mentions
                .insert(id.to_owned(), std::mem::take(&mut self.composer_mentions));
            self.store_draft(
                id,
                self.drafts.get(id).map(String::as_str).unwrap_or_default(),
            );
        }
        self.leave_chat(id);
    }

    /// Takes back a send the worker refused. Nothing typed or recorded is
    /// lost, and a refused reply is never sent again without its quote unless
    /// the user cancels the reply first.
    fn send_refused(
        &mut self,
        chat: ChatId,
        quoting: Option<String>,
        unsent: Unsent,
        reason: Refusal,
    ) {
        let open = self.open_chat.as_deref() == Some(chat.as_str());
        // Re-arm the reply banner, unless the user has moved on to another
        // reply or an edit since.
        if open && quoting.is_some() && self.reply_to.is_none() && self.editing.is_none() {
            self.reply_to = quoting;
            self.focus_composer = true;
        }
        match unsent {
            Unsent::Text(text) => self.restore_text(&chat, text),
            Unsent::Voice(samples) => {
                self.unsent_voice = Some((chat, samples));
                self.focus_composer = open;
            }
            Unsent::Files { paths, caption } => {
                if open {
                    self.pending.extend(paths.into_iter().map(Pending::File));
                }
                self.restore_text(&chat, caption.unwrap_or_default());
            }
            Unsent::Image {
                width,
                height,
                rgba,
                caption,
            } => {
                if open {
                    self.pending.push(Pending::Picture {
                        width: width as usize,
                        height: height as usize,
                        rgba: std::sync::Arc::new(rgba),
                        texture: None,
                    });
                }
                self.restore_text(&chat, caption.unwrap_or_default());
            }
            Unsent::Sticker | Unsent::Gif => {}
        }
        let message = match reason {
            Refusal::Offline => crate::i18n::gettext(
                self.locale,
                "Not sent: ZapFast is not connected to WhatsApp.",
            ),
            Refusal::QuoteUnavailable => crate::i18n::gettext(
                self.locale,
                "Not sent: the message you’re replying to isn’t available on this computer. Cancel the reply to send without a quote.",
            ),
        };
        self.toast_error(message.into_owned());
    }

    /// Returns refused text to its chat's composer, or to its stored draft
    /// when another chat is open. Text typed since the send is kept.
    fn restore_text(&mut self, chat: &str, text: String) {
        if text.trim().is_empty() {
            return;
        }
        if self.open_chat.as_deref() == Some(chat) {
            if self.composer.trim().is_empty() && self.editing.is_none() {
                self.composer = text;
                self.composer_mentions.clear();
                self.emoji_start = None;
                self.mention_start = None;
                self.focus_composer = true;
                self.store_draft(chat, &self.composer);
            }
        } else if self
            .drafts
            .get(chat)
            .is_none_or(|draft| draft.trim().is_empty())
        {
            self.store_draft(chat, &text);
            self.drafts.insert(chat.to_owned(), text);
        }
    }

    /// Mirrors a chat's draft into the encrypted archive, so unsent text
    /// survives a restart. An empty text clears the stored row.
    fn store_draft(&self, chat: &str, text: &str) {
        self.backend.send(Command::SaveDraft {
            chat: chat.to_owned(),
            text: text.to_owned(),
        });
    }

    fn handle_media(
        &mut self,
        chat: &str,
        id: &str,
        card: Option<usize>,
        result: Result<PathBuf, String>,
    ) {
        let Some(message) = self
            .conversations
            .get_mut(chat)
            .and_then(|conversation| conversation.message_mut(id))
        else {
            return;
        };
        let Some(media) = message.content.media_at_mut(card) else {
            return;
        };
        match result {
            Ok(path) => {
                media.path = Some(path.clone());
                media.state = MediaState::Idle;
                if self
                    .video_wanted
                    .as_ref()
                    .is_some_and(|(wanted_chat, wanted)| wanted_chat == chat && wanted == id)
                {
                    self.video_wanted = None;
                    self.actions.push(Action::PlayVideo {
                        message: id.to_owned(),
                        path,
                    });
                } else if self
                    .voice_wanted
                    .as_ref()
                    .is_some_and(|(wanted_chat, wanted, _)| wanted_chat == chat && wanted == id)
                {
                    self.voice_wanted = None;
                    // Leaving the chat ended the run, so a clip that lands
                    // after that is not started.
                    if self.open_chat.as_deref() == Some(chat) {
                        self.actions.push(Action::PlayVoice {
                            message: id.to_owned(),
                            path,
                        });
                    }
                }
            }
            Err(error) => {
                if self
                    .voice_wanted
                    .as_ref()
                    .is_some_and(|(wanted_chat, wanted, _)| wanted_chat == chat && wanted == id)
                {
                    self.voice_wanted = None;
                }
                // Show expired-file failures in the bubble, not as a toast.
                let notice = if error.contains("403") || error.contains("404") {
                    "No longer available on WhatsApp's servers".to_owned()
                } else {
                    error
                };
                log::warn!("attachment download failed; details are shown in the bubble");
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
            known.marked_unread = false;
        }
        // Clear local unread state regardless of receipt settings.
        self.backend.send(Command::MarkRead {
            chat: chat.to_owned(),
            receipts: self.settings.send_read_receipts,
        });
    }

    /// Reminds the reader about a chat with nothing pending. A chat that
    /// already counts unread messages keeps its number instead.
    fn mark_unread(&mut self, chat: &str) {
        let Some(known) = self.chat_mut(chat) else {
            return;
        };
        if known.unread > 0 {
            return;
        }
        known.marked_unread = true;
        self.backend.send(Command::MarkUnread(chat.to_owned()));
    }

    fn open_chat(&mut self, id: ChatId) {
        // Notifications and stale actions must not open a locked chat from
        // outside the authenticated folder.
        if self.chat(&id).is_some_and(|chat| chat.locked) && !self.locked_folder_open() {
            return;
        }
        if self.locked_folder && self.chat(&id).is_some_and(|chat| !chat.locked) {
            self.close_locked_folder();
            self.search.clear();
            self.search_hits.clear();
        }
        if self.open_chat.as_deref() != Some(id.as_str()) {
            self.reaction_target = None;
            self.reaction_anchor = None;
            self.composer_tools_open = false;
            self.emoji_jump = None;
            self.jump_highlight = None;
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
                let draft = self.drafts.get(&previous).cloned().unwrap_or_default();
                self.store_draft(&previous, &draft);
            }
            self.selection = None;
            self.unread_divider =
                self.chat(&id)
                    .filter(|chat| chat.unread > 0)
                    .map(|chat| UnreadDivider {
                        chat: id.clone(),
                        count: chat.unread,
                        placed: false,
                    });
            self.composer = self.drafts.remove(&id).unwrap_or_default();
            self.composer_mentions = self.draft_mentions.remove(&id).unwrap_or_default();
            // A search belongs to the chat it was typed in.
            self.close_chat_search();
            self.reply_to = None;
            self.editing = None;
            // A run of voice messages belongs to the chat it started in.
            self.voice_chat = None;
            self.voice_wanted = None;
            // A page request belongs to the chat it was pressed in.
            self.scroll_page = None;
            if let Some(conversation) = self.conversations.get_mut(&id) {
                conversation.key_scroll = None;
            }
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
        if self
            .chat(&id)
            .is_some_and(|chat| chat.unread > 0 || chat.marked_unread)
        {
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
            && self.reaction_target.is_none()
            && self.recording.is_none()
            && self.open_chat.is_some()
            && self.search.trim().is_empty()
            && !self.focus_search
            && !search_focused
        {
            self.focus_composer = true;
        }
    }

    /// The most recent own text message in the open chat, for Arrow-Up
    /// editing. Non-text and revoked messages cannot be edited and are
    /// skipped.
    pub(crate) fn previous_own_editable(&self) -> Option<String> {
        let conversation = self.conversations.get(self.open_chat.as_deref()?)?;
        conversation
            .messages
            .iter()
            .rev()
            .find(|message| self.can_edit(message))
            .map(|message| message.id.clone())
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
        // The text is on its way, so there is nothing left to restore.
        self.store_draft(&chat, "");
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
        // The reply travels with the first attachment, like the caption.
        let mut quoting = self.reply_to.take();
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
                        quoting: quoting.take(),
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
                quoting: quoting.take(),
            });
        }
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
            quoting: None,
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
        self.toasts.retain(|toast| {
            toast.kind == ToastKind::Error || toast.created.elapsed() < INFO_TOAST_LIFETIME
        });
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
                source: crate::updates::Source::github(),
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
        let waker = self.waker.clone();
        self.custom_themes.start(
            self.dirs.config.join("themes"),
            self.settings.custom_theme.clone(),
            &fastframe_theme::Waker::new(move || waker.wake()),
        );
    }

    fn poll_custom_themes(&mut self) {
        if self.custom_themes.needs_reload() {
            self.load_custom_themes();
        }
        if self.custom_themes.poll() {
            self.cache_custom_themes();
        }
    }

    /// Keeps the selected and the desktop's palettes in settings, so the
    /// last usable appearance survives a missing file or a slow scan.
    fn cache_custom_themes(&mut self) {
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
        if crate::theme::apply_text_rendering_change(ctx) {
            self.applied_dark = None;
        }
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
                // Privacy can change on the phone at any time, and nothing
                // announces it: read it again whenever Settings opens.
                if page == Page::Settings && self.page != Page::Settings && self.is_connected() {
                    self.backend.send(Command::FetchAccountPrivacy);
                }
                self.page = page;
                self.dialog = None;
                self.emoji_start = None;
                self.mention_start = None;
                if opens_chats {
                    // Settings open unfiltered next time.
                    self.settings_search.clear();
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
                self.dialog = None;
            }
            Action::MessageYourself => {
                if let Some(id) = self.me.clone() {
                    if self.chat(&id).is_some_and(|chat| chat.locked) && !self.locked_folder_open()
                    {
                        self.apply(Action::OpenLockedFolder, ctx);
                    } else {
                        self.apply(
                            Action::StartChat {
                                id,
                                name: "You".to_owned(),
                            },
                            ctx,
                        );
                    }
                }
            }
            Action::OpenMessage { chat, message } => {
                // A result picked in the search pane keeps the keyboard in
                // the pane, so the arrows can walk on to the next one.
                let from_pane =
                    self.chat_search_visible() && self.open_chat.as_deref() == Some(chat.as_str());
                self.open_chat(chat.clone());
                // A locked chat stays shut outside its folder, even for a
                // notification clicked before the chat was locked.
                if self.open_chat.as_deref() != Some(chat.as_str()) {
                    return;
                }
                if from_pane {
                    self.focus_composer = false;
                }
                // Keep the search result, not the chat end, in view.
                self.scroll_to_bottom = false;
                self.at_bottom = false;
                self.scroll_anchor = Some(message.clone());
                self.jump_highlight = Some(JumpHighlight::new(chat.clone(), message.clone()));
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
            Action::ReplyInteractive {
                chat,
                message,
                button,
                choice,
            } => {
                if self.link.is_connected() && self.chat(&chat).is_some_and(|chat| chat.can_send())
                {
                    self.backend.send(Command::ReplyInteractive {
                        chat,
                        message,
                        button,
                        choice,
                    });
                    self.scroll_to_bottom = true;
                    self.at_bottom = true;
                }
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
            Action::MarkUnread(chat) => self.mark_unread(&chat),
            Action::LoadOlder(chat) => self.load_older(&chat),
            Action::FetchOlder(chat) => self.fetch_older(&chat),
            Action::Download {
                card,
                chat,
                message,
            } => {
                let Some(media) = self
                    .conversations
                    .get_mut(&chat)
                    .and_then(|conversation| conversation.message_mut(&message))
                    .and_then(|message| message.content.media_at_mut(card))
                else {
                    return;
                };
                if !media.is_within_download_limit() {
                    media.state = MediaState::Failed(
                        "This attachment is larger than the 64 MiB download limit".into(),
                    );
                    return;
                }
                if matches!(media.state, MediaState::Downloading) {
                    return;
                }
                media.state = MediaState::Downloading;
                self.backend.send(Command::Download {
                    card,
                    chat,
                    message,
                });
            }
            Action::PreviewImage(path) => {
                if crate::safety::can_preview_image(&path) && path.is_file() {
                    self.image_preview = Some(PreviewState::new(path));
                    self.dialog = None;
                    self.picker = None;
                    // egui drops the focus of widgets behind a modal only from
                    // the frame after it first shows; until then a focused
                    // composer would still take Enter and send the draft.
                    ctx.memory_mut(|memory| {
                        if let Some(focused) = memory.focused() {
                            memory.surrender_focus(focused);
                        }
                    });
                } else {
                    self.actions.push(Action::OpenFile(path));
                }
            }
            Action::ZoomImageIn => {
                if let Some(preview) = &mut self.image_preview {
                    preview.zoom_in();
                }
            }
            Action::ZoomImageOut => {
                if let Some(preview) = &mut self.image_preview {
                    preview.zoom_out();
                }
            }
            Action::FitImage => {
                if let Some(preview) = &mut self.image_preview {
                    preview.fit();
                }
            }
            Action::ImageActualSize => {
                if let Some(preview) = &mut self.image_preview {
                    preview.actual_size();
                }
            }
            Action::CloseImagePreview => {
                self.image_preview = None;
                self.refocus_composer(ctx);
            }
            Action::OpenFile(path) => {
                if crate::safety::can_open_attachment(&path) && path.is_file() {
                    if let Err(error) = open::that_detached(&path) {
                        self.toast_error(format!("Could not open the attachment: {error}"));
                    }
                } else {
                    self.toast("For safety, open this file yourself from its folder");
                    if let Some(folder) = path.parent() {
                        self.actions.push(Action::OpenFolder(folder.to_owned()));
                    }
                }
            }
            Action::SaveAttachmentAs { path, name } => {
                self.backend
                    .send(Command::SaveAttachmentAs { source: path, name });
            }
            Action::OpenFolder(path) => {
                if path.is_dir() {
                    if let Err(error) = open::that_detached(&path) {
                        self.toast_error(format!("Could not open the folder: {error}"));
                    }
                } else {
                    self.toast_error("The folder is unavailable");
                }
            }
            Action::OpenUrl(url) => {
                if let Some(code) = crate::safety::group_invite_code(&url) {
                    self.invite = Some(crate::model::GroupInvite {
                        code: code.clone(),
                        state: crate::model::InviteState::Loading,
                    });
                    self.dialog = Some(Dialog::JoinGroup);
                    self.backend.send(Command::PreviewInvite(code));
                } else if let Some(url) = crate::safety::external_url(&url) {
                    ctx.open_url(egui::OpenUrl::new_tab(url));
                } else {
                    self.toast_error("This link type cannot be opened from ZapFast");
                }
            }
            Action::CopyText(text) => {
                ctx.copy_text(text);
                self.toast("Copied");
            }
            Action::CopyImage(path) => {
                self.backend.send(Command::PrepareClipboardImage(path));
            }
            Action::DismissToast(index) => {
                if index < self.toasts.len() {
                    self.toasts.remove(index);
                }
            }
            Action::Reply(id) => {
                // Replying while editing starts a new message: the edited
                // text must not go out as the reply.
                if self.editing.take().is_some() {
                    self.composer.clear();
                    self.composer_mentions.clear();
                    self.emoji_start = None;
                    self.mention_start = None;
                }
                self.reply_to = Some(id);
                self.focus_composer = true;
            }
            Action::CancelReply => self.reply_to = None,
            Action::Forward {
                from_chat,
                messages,
                to_chat,
            } => {
                self.backend.send(Command::Forward {
                    from_chat,
                    messages,
                    to_chat,
                });
                self.dialog = None;
                self.forward_search.clear();
                self.selection = None;
            }
            Action::SelectMessage(id) => {
                if let Some(chat) = self.open_chat.clone() {
                    self.selection = Some((chat, vec![id.clone()]));
                    self.selection_anchor = Some(id);
                }
            }
            Action::SelectRange(id) => {
                let Some((chat, ids)) = self.selection.as_mut() else {
                    return;
                };
                let Some(conversation) = self.conversations.get(chat.as_str()) else {
                    return;
                };
                let position = |id: &str| {
                    conversation
                        .messages
                        .iter()
                        .position(|message| message.id == id)
                };
                let anchor = self.selection_anchor.clone().unwrap_or_else(|| id.clone());
                if let (Some(from), Some(to)) = (position(&anchor), position(&id)) {
                    let (from, to) = (from.min(to), from.max(to));
                    for message in &conversation.messages[from..=to] {
                        // Deleted and placeholder messages cannot be forwarded.
                        if !matches!(
                            message.content,
                            Content::Revoked
                                | Content::PhoneOnly { .. }
                                | Content::Unsupported { .. }
                        ) && !ids.contains(&message.id)
                        {
                            ids.push(message.id.clone());
                        }
                    }
                    ids.sort_by_key(|id| position(id).unwrap_or(usize::MAX));
                }
                self.selection_anchor = Some(id);
            }
            Action::ToggleSelected(id) => {
                self.selection_anchor = Some(id.clone());
                if let Some((chat, ids)) = self.selection.as_mut() {
                    if let Some(index) = ids.iter().position(|selected| *selected == id) {
                        ids.remove(index);
                    } else {
                        ids.push(id);
                        // Keep the chat's order, so forwards arrive as they were sent.
                        if let Some(conversation) = self.conversations.get(chat.as_str()) {
                            let position = |id: &String| {
                                conversation
                                    .messages
                                    .iter()
                                    .position(|message| message.id == *id)
                                    .unwrap_or(usize::MAX)
                            };
                            ids.sort_by_key(position);
                        }
                    }
                    if ids.is_empty() {
                        self.selection = None;
                    }
                }
            }
            Action::CancelSelection => self.selection = None,
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
                    self.composer_tools_open = false;
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
            Action::SetComposerTools(open) => {
                self.composer_tools_open = open;
                if open {
                    self.picker = None;
                    self.reaction_target = None;
                    self.reaction_anchor = None;
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
            Action::PlayVideo { message, path } => self.play_video(message, path),
            Action::PlayVideoWhenDownloaded(message) => {
                self.voice_chat = None;
                self.voice_wanted = None;
                self.video_wanted = self.open_chat.clone().map(|chat| (chat, message));
            }
            Action::SeekVideo { message, fraction } => self.video.seek(&message, fraction),
            Action::ToggleVideoSound => self.video.toggle_mute(),
            Action::SeekVoice {
                message,
                path,
                fraction,
            } => {
                // One sound at a time, and the clip the reader picked wins
                // over one a run is still fetching.
                self.video.stop();
                self.voice_wanted = None;
                self.voice_chat = self.open_chat.clone();
                if let Err(error) = self.player.seek(&message, &path, fraction) {
                    self.toast_error(error);
                }
            }
            Action::SetVoiceSpeed(speed) => {
                self.settings.voice_speed = self.player.set_speed(speed);
                self.mark_settings_dirty();
            }
            Action::StartRecording => {
                if self.open_chat.is_some() && self.recording.is_none() {
                    self.picker = None;
                    self.composer_tools_open = false;
                    // An offline demo never opens the microphone.
                    #[cfg(any(test, feature = "demo"))]
                    let recorder = if self.backend.is_offline() {
                        Recorder::simulated(self.waker.clone())
                    } else {
                        Recorder::start(self.waker.clone())
                    };
                    #[cfg(not(any(test, feature = "demo")))]
                    let recorder = Recorder::start(self.waker.clone());
                    self.recording = Some(recorder);
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
            Action::DiscardUnsentVoice => self.unsent_voice = None,
            Action::SetMuted(chat, until) => {
                if let Some(known) = self.chat_mut(&chat) {
                    known.muted_until = until;
                }
                self.backend.send(Command::SetMuted(chat, until));
            }
            Action::SetLocked(chat, locked) => {
                if let Some(known) = self.chat_mut(&chat) {
                    known.locked = locked;
                }
                // Locking the open chat closes it, as the phone does.
                if locked && self.open_chat.as_deref() == Some(chat.as_str()) {
                    self.hide_locked_chat(&chat);
                }
                self.backend.send(Command::SetLocked(chat, locked));
            }
            Action::TogglePicker(tab) => {
                self.composer_tools_open = false;
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
                if let Some((chat, message)) = &self.reaction_target {
                    egui::Popup::close_id(
                        ctx,
                        crate::ui::conversation::bubble_id(chat, message).with("popup"),
                    );
                }
                self.picker = None;
                self.reaction_target = None;
                self.reaction_anchor = None;
                self.emoji_jump = None;
                if !was_reaction {
                    self.refocus_composer(ctx);
                }
            }
            Action::OpenReactionPicker {
                chat,
                message,
                beside_menu,
            } => {
                self.focus_composer = false;
                self.emoji_start = None;
                self.mention_start = None;
                self.picker = None;
                let id = crate::ui::conversation::bubble_id(&chat, &message);
                // Anchor to the menu when it stays open, else to the hover
                // button that opened the picker.
                let anchor = if beside_menu {
                    "menu-rect"
                } else {
                    "react-rect"
                };
                self.reaction_anchor =
                    ctx.data(|data| data.get_temp::<egui::Rect>(id.with(anchor)));
                self.reaction_beside_menu = beside_menu;
                self.reaction_target = Some((chat, message));
                self.picker_search.clear();
                self.picker_focus = true;
                self.emoji_selected = 0;
                self.emoji_jump = None;
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
                self.toast(crate::i18n::gettext(self.locale, "Added to favorites"));
            }
            Action::RemoveRecentSticker(path) => {
                self.stickers.retain(|recent| *recent != path);
                self.backend.send(Command::RemoveRecentSticker { path });
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
                if self.sticker_shelf == StickerShelf::Pack(dir.clone()) {
                    self.sticker_shelf = StickerShelf::Recent;
                }
                self.backend.send(Command::DeleteStickerPack { dir });
            }
            Action::CreateStickerPack(name) => {
                let name = name.trim().to_owned();
                if !name.is_empty() {
                    // The backend picks the folder; select the pack once the
                    // next Stickers event lists it.
                    self.sticker_pack_created = Some(name.clone());
                    self.backend.send(Command::CreateStickerPack { name });
                }
            }
            Action::SelectStickerShelf(shelf) => {
                self.sticker_shelf = shelf;
            }
            Action::ViewStickerPack(message) => {
                if let Some(chat) = self.open_chat.clone() {
                    self.sticker_preview = None;
                    self.sticker_preview_pending = true;
                    self.dialog = Some(Dialog::StickerPack);
                    self.backend
                        .send(Command::ViewStickerPack { chat, message });
                }
            }
            Action::PickStickerPicture => {
                self.backend.send(Command::PickStickerPicture);
            }
            Action::MakeSticker { send } => {
                if let Some(draft) = self.sticker_draft.take() {
                    let chat = if send { self.open_chat.clone() } else { None };
                    self.backend.send(Command::MakeSticker {
                        source: draft.source,
                        crop: draft.crop,
                        transparent: draft.transparent && draft.keep_transparent,
                        emojis: crate::sticker_meta::clean_emojis(&draft.emojis),
                        chat,
                    });
                }
                self.dialog = None;
            }
            Action::AddStickerPack => {
                if let Some((pack, _)) = &self.sticker_preview {
                    self.backend.send(Command::AddStickerPack {
                        dir: pack.dir.clone(),
                        name: pack.name.clone(),
                    });
                }
                self.dialog = None;
            }
            Action::ShareStickerPack(dir) => {
                if let Some(chat) = self.open_chat.clone() {
                    self.picker = None;
                    self.toast(crate::i18n::gettext(
                        self.locale,
                        "Sending the sticker pack…",
                    ));
                    self.backend.send(Command::SendStickerPack { chat, dir });
                }
            }
            Action::SetStickerPack {
                pack,
                sticker,
                member,
            } => {
                self.backend.send(Command::SetStickerPack {
                    pack,
                    sticker,
                    member,
                });
            }
            Action::SendSticker(path) => {
                if let Some(chat) = self.open_chat.clone() {
                    let quoting = self.reply_to.take();
                    self.backend.send(Command::SendSticker {
                        chat,
                        path,
                        quoting,
                    });
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
                    let quoting = self.reply_to.take();
                    self.backend.send(Command::SendGif { chat, gif, quoting });
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
                    let count = self
                        .settings
                        .reaction_emoji
                        .iter()
                        .find(|(known, _)| known == &emoji)
                        .map_or(1, |(_, count)| count.saturating_add(1));
                    self.settings
                        .reaction_emoji
                        .retain(|(known, _)| known != &emoji);
                    self.settings
                        .reaction_emoji
                        .insert(0, (emoji.clone(), count));
                    self.settings
                        .reaction_emoji
                        .sort_by_key(|(_, count)| std::cmp::Reverse(*count));
                    self.settings.reaction_emoji.truncate(36);
                    self.remember_emoji(&emoji);
                }
                egui::Popup::close_id(
                    ctx,
                    crate::ui::conversation::bubble_id(&chat, &message).with("popup"),
                );
                self.reaction_target = None;
                self.reaction_anchor = None;
                self.backend.send(Command::React {
                    chat,
                    message,
                    emoji,
                });
            }
            Action::LeaveGroup { chat, archive } => {
                self.dialog = None;
                let ours: Vec<String> = self.our_ids().into_iter().map(str::to_owned).collect();
                if let Some(known) = self.chat_mut(&chat) {
                    known.read_only = true;
                    known.left = true;
                    known.participants.retain(|id| !ours.contains(id));
                }
                // Archiving and closing the open conversation both wait for the
                // phone: a refused leave rolls the mark back, and a chat that
                // archived and closed itself would not come back. The
                // confirmed update does both.
                self.backend.send(Command::LeaveGroup { chat, archive });
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
            // The chat leaves the list once the phone confirmed, through
            // `Event::ChatRemoved`.
            Action::DeleteChat(chat) => self.backend.send(Command::DeleteChat(chat)),
            Action::SetPinned(chat, pinned) => {
                if pinned && self.pinned_count() >= self.pin_limit {
                    self.toast(format!("You can only pin {} chats", self.pin_limit));
                    return;
                }
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
            Action::SetFavorite(chat, favorite) => {
                if let Some(known) = self.chat_mut(&chat) {
                    known.favorite = favorite;
                    // A new favorite joins the end until the archive numbers it.
                    known.favorite_position = u32::MAX;
                }
                self.backend.send(Command::SetFavorite(chat, favorite));
            }
            Action::ShowDialog(dialog) => {
                self.clear_chat_lock_entry();
                if dialog == Dialog::NewChat {
                    self.new_chat_search.clear();
                }
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
                    self.new_contact_to_phone = self.settings.save_contacts_to_phone;
                    self.new_contact_phone.clear();
                    self.new_contact_name.clear();
                    self.new_contact_last.clear();
                    self.new_contact_pending = false;
                }
                self.contact_edit = None;
                self.dialog = Some(dialog);
            }
            Action::CloseDialog => {
                self.clear_chat_lock_entry();
                self.dialog = None;
                self.invite = None;
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
            Action::NewContact {
                phone,
                first,
                last,
                to_phone,
            } => {
                self.new_contact_pending = true;
                let (full_name, first_name) = compose_name(&first, &last);
                // The dialog's choice starts the next one.
                if let Some(to_phone) = to_phone
                    && full_name.is_some()
                    && to_phone != self.settings.save_contacts_to_phone
                {
                    self.settings.save_contacts_to_phone = to_phone;
                    self.mark_settings_dirty();
                }
                self.backend.send(Command::NewContact {
                    phone,
                    full_name,
                    first_name,
                    to_phone: to_phone.unwrap_or(self.settings.save_contacts_to_phone),
                });
            }
            Action::ToggleSidebar => self.sidebar_visible = !self.sidebar_visible,
            Action::SetChatFilter(filter) => {
                if self.locked_folder {
                    self.close_locked_folder();
                    self.search.clear();
                    self.search_hits.clear();
                }
                self.chat_filter = filter;
                self.label_filter = None;
                self.show_archived = false;
                self.unread_kept.clear();
            }
            Action::JoinGroup => {
                use crate::model::InviteState;
                if let Some(invite) = self.invite.as_mut()
                    && let InviteState::Ready(info) = &invite.state
                {
                    if self.chats.iter().any(|chat| chat.id == info.id) {
                        let id = info.id.clone();
                        self.invite = None;
                        self.dialog = None;
                        self.actions.push(Action::OpenChat(id));
                    } else {
                        invite.state = InviteState::Joining(info.clone());
                        self.backend.send(Command::JoinInvite(invite.code.clone()));
                    }
                }
            }
            Action::MuteAllChannels(mute) => {
                let channels: Vec<ChatId> = self
                    .chats
                    .iter()
                    .filter(|chat| chat.is_channel())
                    .map(|chat| chat.id.clone())
                    .collect();
                for chat in channels {
                    self.actions.push(Action::SetMuted(chat, mute.then_some(0)));
                }
            }
            Action::ShowArchived(show) => {
                if self.locked_folder {
                    self.close_locked_folder();
                    self.search.clear();
                    self.search_hits.clear();
                }
                self.show_archived = show;
                self.unread_kept.clear();
            }
            Action::SelectLabel(label) => self.select_label(label),
            Action::SetChatLabels { chat, labels } => {
                self.backend.send(Command::SetChatLabels { chat, labels });
            }
            Action::CreateLabel { name, color_hex } => {
                let name = name.trim().to_owned();
                if name.is_empty() {
                    return;
                }
                if self.labels.len() >= crate::archive::LABEL_LIMIT {
                    self.toast_error(
                        crate::i18n::gettext(
                            self.locale,
                            "You have {limit} labels, the most ZapFast keeps.",
                        )
                        .replace("{limit}", &crate::archive::LABEL_LIMIT.to_string()),
                    );
                    return;
                }
                if let Some(refusal) = self.label_name_refusal(&name, None) {
                    self.toast_error(refusal);
                    return;
                }
                self.backend.send(Command::CreateLabel { name, color_hex });
                self.label_name.clear();
                self.label_color = crate::archive::DEFAULT_COLOR.to_owned();
            }
            Action::UpdateLabel {
                id,
                name,
                color_hex,
            } => {
                let name = name.trim().to_owned();
                if name.is_empty() {
                    return;
                }
                if let Some(refusal) = self.label_name_refusal(&name, Some(&id)) {
                    self.toast_error(refusal);
                    return;
                }
                self.backend.send(Command::UpdateLabel {
                    id,
                    name,
                    color_hex,
                });
            }
            Action::DeleteLabel(id) => {
                self.backend.send(Command::DeleteLabel(id));
                self.label_editing = None;
            }
            // Reading a chat must not pull its row out from under the pointer.
            // Only the filtered list sends this: search results and
            // notifications open chats without keeping them.
            Action::KeepUnread(id) => {
                if self.chat_filter == ChatFilter::Unread {
                    self.unread_kept.insert(id);
                }
            }
            // Ctrl+K and Ctrl+Shift+F search the chat list everywhere; Ctrl+F
            // searches the open chat when there is one.
            Action::FocusSearch => self.actions.push(Action::FocusChatList),
            Action::FocusChatList => {
                // The list search takes over Escape and Enter from the chat's.
                self.close_chat_search();
                self.sidebar_visible = true;
                self.page = Page::Chats;
                self.focus_composer = false;
                self.focus_search = true;
                self.emoji_start = None;
                self.mention_start = None;
            }
            Action::FocusSettingsSearch => {
                self.page = Page::Settings;
                self.focus_settings_search = true;
            }
            Action::SearchSettings(text) => self.settings_search = text,
            Action::FocusComposer => {
                self.focus_search = false;
                self.focus_composer = true;
            }
            Action::ScrollToBottom => {
                self.scroll_to_bottom = true;
                // Ctrl+End and the return-to-bottom button are explicit: they
                // must win over a message bubble's retained keyboard focus.
                self.scroll_to_bottom_forced = true;
            }
            Action::ScrollPage(scroll) => {
                // Reaching the top releases stick-to-bottom, as the wheel and
                // the edge-scroll drag do; reaching the bottom (paging down or
                // End) lets it take over again, so it is left alone here.
                if matches!(scroll, Scroll::PageUp | Scroll::Top) {
                    self.scroll_to_bottom = false;
                }
                self.scroll_page = Some(scroll);
            }
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
                        chat: chat.clone(),
                        id: id.clone(),
                        before: (oldest.timestamp, oldest.id.clone()),
                    });
                }
                self.jump_highlight = Some(JumpHighlight::new(chat, id.clone()));
                self.scroll_anchor = Some(id);
            }
            Action::Search(text) => {
                self.search = text;
                let query = self.search.trim().to_owned();
                // Editing the search away from the secret code hides the
                // locked folder again, like leaving the phone's home screen.
                if !self.chat_lock_authenticated() && !self.secret_code_matched() {
                    self.close_locked_folder();
                }
                if query.is_empty() || self.locked_folder_open() || self.secret_code_matched() {
                    self.search_hits.clear();
                } else {
                    self.backend.send(Command::SearchMessages { query });
                }
            }
            Action::OpenChatSearch => self.open_chat_search(),
            Action::CloseChatSearch => {
                self.close_chat_search();
                self.refocus_composer(ctx);
            }
            Action::ChatSearch(query) => {
                self.chat_search = query;
                self.request_chat_search();
            }
            Action::SetChatSearchDay(day) => {
                self.chat_search_day = day;
                self.chat_search_calendar = false;
                if let Some(day) = day {
                    self.chat_search_month = day;
                }
                self.request_chat_search();
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
            Action::SetInterfaceLanguage(choice) => {
                self.settings.interface_language = choice;
                self.locale = crate::i18n::resolve(choice);
                self.mark_settings_dirty();
            }
            Action::SetCustomTheme(filename) => {
                if let Some(theme) = self.custom_themes.find(&filename) {
                    self.settings.custom_theme_cache = Some(theme.clone());
                    self.settings.custom_theme = Some(filename);
                    self.mark_settings_dirty();
                    self.apply_theme(ctx);
                }
            }
            Action::SetWallpaperColor(color) => {
                if self.palette.dark {
                    self.settings.dark_wallpaper_color = color;
                } else {
                    self.settings.wallpaper_color = color;
                }
                self.mark_settings_dirty();
            }
            Action::SetWallpaperDoodles(show) => {
                self.settings.show_wallpaper = show;
                self.mark_settings_dirty();
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
            Action::SetShortcutHints(show) => {
                self.settings.show_shortcut_hints = show;
                self.mark_settings_dirty();
            }
            Action::DismissChatLockHint => {
                self.settings.chat_lock_hint_dismissed = true;
                self.mark_settings_dirty();
            }
            Action::OpenLockedFolder => {
                if self.locked_folder_open() || self.secret_code_matched() {
                    self.enter_locked_folder();
                } else {
                    self.clear_chat_lock_entry();
                    self.dialog = Some(Dialog::UnlockLockedChats);
                }
            }
            Action::UnlockLockedFolder(code) => {
                if self.settings.verifies_chat_lock_code(code.trim()) {
                    self.enter_locked_folder();
                } else {
                    self.chat_lock_entry.clear();
                    self.chat_lock_error = true;
                }
            }
            Action::CreateChatLockCode(code) => {
                if self.settings.chat_lock_code_hash.is_none() && !code.trim().is_empty() {
                    self.settings.set_chat_lock_code(Some(code.trim()));
                    self.mark_settings_dirty();
                    self.enter_locked_folder();
                }
            }
            Action::CloseLockedFolder => {
                self.close_locked_folder();
                self.search.clear();
                self.search_hits.clear();
            }
            Action::SetChatLockCode(code) => {
                self.settings.set_chat_lock_code(code.as_deref());
                self.close_locked_folder();
                self.search.clear();
                self.search_hits.clear();
                self.mark_settings_dirty();
            }
            Action::SettingsChanged => self.mark_settings_dirty(),
            Action::SetAccountPrivacy { kind, choice } => {
                // The value lives on the phone: nothing is written without a
                // connection and a snapshot to write against.
                if self.is_connected()
                    && self.account_privacy.editable()
                    && self.account_privacy.begin_set(kind, choice)
                {
                    self.backend
                        .send(Command::SetAccountPrivacy { kind, choice });
                }
            }
            Action::SetNotificationSound { mention, sound } => {
                if mention {
                    self.settings.mention_sound = sound;
                } else {
                    self.settings.message_sound = sound;
                }
                self.mark_settings_dirty();
            }
            Action::PickNotificationSound { mention } => {
                self.backend
                    .send(Command::PickNotificationSound { mention });
            }
            Action::PreviewSound(sound) => crate::notify::play_sound(sound),
            Action::PickDownloadFolder => self.backend.send(Command::PickDownloadFolder),
            Action::SetProfile { name, about } => {
                self.backend.send(Command::SetProfile { name, about });
            }
            Action::PickProfilePicture => self.backend.send(Command::PickProfilePicture),
            Action::SetChatSound { chat, sound } => {
                if let Some(known) = self.chat_mut(&chat) {
                    known.notification_sound = sound.clone();
                }
                self.backend.send(Command::SetChatSound { chat, sound });
            }
            Action::PickChatSound(chat) => self.backend.send(Command::PickChatSound(chat)),
            Action::SetDownloadFolder(folder) => {
                self.settings.download_folder = folder.clone();
                self.mark_settings_dirty();
                self.backend.send(Command::SetDownloadFolder(folder));
            }
            Action::SetProxy(value) => {
                let value = value.trim().to_owned();
                if value == self.settings.proxy {
                    return;
                }
                if !value.is_empty()
                    && let Err(error) = crate::proxy::Proxy::parse(&value)
                {
                    self.toast_error(error);
                    return;
                }
                self.settings.proxy = value.clone();
                self.mark_settings_dirty();
                crate::proxy::configure(&value);
                self.backend.send(Command::SetProxy(value));
            }
            Action::SetStartWithSystem(enabled) => match crate::autostart::set(enabled) {
                Ok(()) => self.start_with_system = Some(crate::autostart::enabled()),
                Err(error) => self.toast_error(format!("Could not change the login item: {error}")),
            },
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
            Action::StartOverArchive => {
                self.dialog = None;
                self.backend.send(Command::StartOverArchive);
            }
            Action::Quit => {
                self.quit_requested = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            Action::ShowWindow => {
                if self.window_hidden {
                    // The headless loop in `main` will create the window.
                    self.wants_show = true;
                } else {
                    // Focus alone leaves a minimized window where it is on
                    // Windows, so restore it first.
                    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
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
        while self
            .toasts
            .iter()
            .filter(|toast| toast.kind == ToastKind::Info)
            .count()
            > 4
        {
            if let Some(oldest) = self
                .toasts
                .iter()
                .position(|toast| toast.kind == ToastKind::Info)
            {
                self.toasts.remove(oldest);
            }
        }
    }

    pub fn toast_error(&mut self, message: impl Into<String>) {
        let message = message.into();
        log::warn!("an operation failed; details are shown in the window");
        // Errors stay until dismissed: a repeat moves to the end instead of
        // stacking, and only the newest few are kept.
        self.toasts
            .retain(|toast| toast.kind != ToastKind::Error || toast.message != message);
        while self
            .toasts
            .iter()
            .filter(|toast| toast.kind == ToastKind::Error)
            .count()
            >= MAX_ERROR_TOASTS
        {
            let oldest = self
                .toasts
                .iter()
                .position(|toast| toast.kind == ToastKind::Error)
                .expect("counted above");
            self.toasts.remove(oldest);
        }
        self.toasts.push(Toast {
            message,
            kind: ToastKind::Error,
            created: Instant::now(),
        });
    }

    /// Chats pinned to the top, counted the way WhatsApp limits them.
    fn pinned_count(&self) -> usize {
        self.chats
            .iter()
            .filter(|chat| chat.pinned && !chat.archived)
            .count()
    }

    /// Tells the backend whether the person is looking at the app, so the
    /// phone keeps its notifications while they are not.
    fn report_presence(&mut self) {
        let online = self.window_focused && !self.window_hidden;
        if self.reported_online != Some(online) {
            self.reported_online = Some(online);
            self.backend.send(Command::SetOnline(online));
        }
    }

    /// Processes app state shared by windowed and headless modes.
    pub fn background_frame(&mut self, ctx: &egui::Context) {
        // Events are drained before frame_ui observes focus. Losing focus in
        // this frame must take effect before an incoming chat update can read it.
        if self.window_hidden || ctx.input(|input| input.viewport().focused) == Some(false) {
            self.window_focused = false;
        }
        self.report_presence();
        self.handle_tray();
        #[cfg(target_os = "macos")]
        self.actions.extend(crate::macos::drain(self.window_hidden));
        self.handle_control_commands();
        self.poll_custom_themes();
        self.handle_notification_opens();
        self.handle_events();
        self.tick(ctx);
        self.tick_audio();
        self.tick_video(ctx);
        self.apply_actions(ctx);
        self.hold_media();
        self.follow_receipts();
        self.sync_badge();
    }

    /// Mirrors the unread total onto the taskbar icon, where the desktop
    /// reads it. The badge ignores repeats, so calling this each frame is cheap.
    fn sync_badge(&mut self) {
        let count = self.unread_total();
        if let Some(badge) = &mut self.badge {
            badge.set(count);
        }
    }

    /// Pauses other apps' music while recording or playing audio, as the
    /// settings allow, and resumes it once neither needs quiet.
    fn hold_media(&mut self) {
        let wanted = self.pauses_media && self.wants_quiet();
        if wanted != self.media_hold.is_some() {
            self.media_hold = wanted.then(crate::media_pause::hold);
        }
    }

    /// Whether recording or playback needs other apps' media paused now. A
    /// run of voice messages keeps it paused while the next clip downloads, so
    /// music does not resume and pause again between clips, but only for a
    /// while.
    fn wants_quiet(&self) -> bool {
        let fetching_next = self
            .voice_wanted
            .as_ref()
            .is_some_and(|(_, _, since)| since.elapsed() < VOICE_FETCH_HOLD);
        self.settings.pause_other_media
            && (self.recording.is_some()
                || self.player.is_playing()
                || fetching_next
                || self.video.is_active() && !self.video.muted())
    }

    /// Keeps the backend following receipts for exactly the group message
    /// whose "Message info" is open. A direct message's times are on its row.
    fn follow_receipts(&mut self) {
        let wanted = match &self.dialog {
            Some(Dialog::MessageInfo { chat, message })
                if crate::model::ChatKind::from_id(chat) == crate::model::ChatKind::Group =>
            {
                Some((chat.clone(), message.clone()))
            }
            _ => None,
        };
        if wanted == self.receipts_watch {
            return;
        }
        self.message_receipts = None;
        self.receipts_watch = wanted.clone();
        self.backend.send(Command::WatchReceipts(wanted));
    }

    /// Polls audio state and schedules repaints while it changes.
    fn tick_audio(&mut self) {
        if let Err(error) = self.player.poll() {
            self.toast_error(error);
        }
        if let Some(finished) = self.player.take_finished() {
            self.continue_voice(&finished);
        }
        if let Some(error) = self.recording.as_ref().and_then(Recorder::failure) {
            self.recording = None;
            self.toast_error(format!("Could not record: {error}"));
        }
        if self.player.is_playing() || self.recording.is_some() {
            self.waker.wake_after(Duration::from_millis(40));
        }
        // Let the media hold lapse on time if the next clip never lands.
        if let Some((_, _, since)) = &self.voice_wanted {
            let left = VOICE_FETCH_HOLD.saturating_sub(since.elapsed());
            if !left.is_zero() {
                self.waker.wake_after(left);
            }
        }
    }

    /// Shows the playing video's frames, stops it once its chat is left, and
    /// hands a video it cannot decode to the system player.
    fn tick_video(&mut self, ctx: &egui::Context) {
        if self.video.message().is_some() && self.video_chat != self.open_chat {
            self.video.stop();
        }
        if let Some(crate::video::Notice::Unsupported(path)) = self.video.poll(ctx) {
            self.toast(crate::i18n::gettext(
                self.locale,
                "This video opens in your system player",
            ));
            self.actions.push(Action::OpenFile(path));
        }
    }

    /// Plays or pauses a video in its message. A video message, the round
    /// kind, sends its played receipt like a voice message.
    fn play_video(&mut self, message: String, path: PathBuf) {
        let Some(chat) = self.open_chat.clone() else {
            return;
        };
        // One sound at a time; a video also ends a run of voice messages.
        self.player.stop();
        self.voice_chat = None;
        self.voice_wanted = None;
        let starting = self.video.message() != Some(message.as_str());
        self.video.toggle(&message, &path);
        self.video_chat = Some(chat.clone());
        let note = self
            .conversations
            .get(&chat)
            .and_then(|conversation| conversation.message(&message))
            .is_some_and(|row| matches!(row.content, Content::Video { note: true, .. }));
        if starting && note {
            self.tell_played(message);
        }
    }

    /// Plays or pauses audio and sends the first played receipt when needed.
    fn play_voice(&mut self, message: String, path: PathBuf) {
        self.video.stop();
        self.voice_wanted = None;
        self.voice_chat = self.open_chat.clone();
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

    /// Starts the next unheard voice message after one plays to its end, as
    /// the phone does. Only a clip that finished in the chat the reader is
    /// still in carries on. A clip that is not downloaded yet is fetched first
    /// and plays when it lands.
    fn continue_voice(&mut self, finished: &str) {
        let Some(chat) = self.open_chat.clone() else {
            return;
        };
        if self.voice_chat.as_ref() != Some(&chat) {
            return;
        }
        let Some(next) = self.next_voice_after(&chat, finished) else {
            return;
        };
        match self
            .conversations
            .get(&chat)
            .and_then(|conversation| conversation.message(&next))
            .and_then(|message| message.content.media())
            .and_then(|media| media.path.clone())
        {
            Some(path) => self.actions.push(Action::PlayVoice {
                message: next,
                path,
            }),
            None => {
                self.voice_wanted = Some((chat.clone(), next.clone(), Instant::now()));
                self.actions.push(Action::Download {
                    card: None,
                    chat,
                    message: next,
                });
            }
        }
    }

    /// The next voice message after `finished` that the reader has not heard
    /// yet. As on the phone, the run goes through consecutive voice messages:
    /// the reader's own and those already played here are passed over, and
    /// anything else, an audio file included, ends it.
    fn next_voice_after(&self, chat: &str, finished: &str) -> Option<String> {
        let conversation = self.conversations.get(chat)?;
        let position = conversation
            .messages
            .iter()
            .position(|message| message.id == finished)?;
        let voice_note = |message: &Message| {
            matches!(
                message.content,
                Content::Audio {
                    voice_note: true,
                    ..
                }
            )
        };
        if !voice_note(&conversation.messages[position]) {
            return None;
        }
        conversation.messages[position + 1..]
            .iter()
            .take_while(|message| voice_note(message))
            .find(|message| !message.from_me && !self.played_told.contains(&message.id))
            .map(|message| message.id.clone())
    }

    /// Stops and sends a recording unless it is under one second. With no
    /// recorder running, sends the open chat's refused voice message again;
    /// it quotes whatever the reply banner shows now, so a reply the worker
    /// refused for its missing original is only sent unquoted after the
    /// user cancels the reply.
    fn send_recording(&mut self) {
        let Some(recorder) = self.recording.take() else {
            if let Some(chat) = self.open_chat.clone()
                && self
                    .unsent_voice
                    .as_ref()
                    .is_some_and(|(unsent, _)| *unsent == chat)
                && let Some((_, samples)) = self.unsent_voice.take()
            {
                let quoting = self.reply_to.take();
                self.backend.send(Command::SendVoice {
                    chat,
                    samples,
                    quoting,
                });
                self.scroll_to_bottom = true;
                self.at_bottom = true;
            }
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
        self.report_presence();
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
        // Release the image caches of everything that scrolled away.
        crate::image_cache::sweep(ctx);
        // Only fading info toasts animate. Errors wait for the reader.
        if self
            .toasts
            .iter()
            .any(|toast| toast.kind == ToastKind::Info)
        {
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
        let (dropped, hovering) = ctx.input(|input| {
            let dropped: Vec<PathBuf> = input
                .raw
                .dropped_files
                .iter()
                .map(|file| file.path().to_path_buf())
                .collect();
            let hovering = !input.raw.hovered_files.is_empty();
            (dropped, hovering)
        });
        self.dropping = hovering && self.open_chat.is_some();
        if !dropped.is_empty() {
            self.actions.push(Action::SendFiles(dropped));
        }
        self.take_image_paste(ctx, clipboard_image);
    }

    fn take_image_paste(
        &mut self,
        ctx: &egui::Context,
        read_image: impl FnOnce() -> Option<(usize, usize, Vec<u8>)>,
    ) {
        let (paste, text, released, focused, command) = ctx.input(|input| {
            (
                wants_paste(input),
                input
                    .events
                    .iter()
                    .any(|event| matches!(event, egui::Event::Paste(_))),
                input.events.iter().any(|event| {
                    matches!(
                        event,
                        egui::Event::Key {
                            key: egui::Key::V,
                            pressed: false,
                            ..
                        }
                    )
                }),
                input.focused,
                input.modifiers.command,
            )
        });
        let requested = paste && (text || !self.paste_before_release);
        if released || !focused {
            self.paste_before_release = false;
        } else if text {
            // A menu paste has no key release to wait for.
            self.paste_before_release = command;
        }
        // Handle image paste only when the composer or no field has focus.
        let composing = ctx.memory(|memory| {
            memory.has_focus(egui::Id::new("composer-text")) || memory.focused().is_none()
        });
        if requested
            && focused
            && composing
            && self.page == Page::Chats
            && self.dialog.is_none()
            && self
                .open_chat
                .as_deref()
                .and_then(|id| self.chat(id))
                .is_some_and(Chat::can_send)
            && let Some((width, height, rgba)) = read_image()
        {
            // A browser can offer both pixels and its source URL. Consume the
            // text before the composer sees it, keeping any existing caption.
            ctx.input_mut(|input| {
                input
                    .events
                    .retain(|event| !matches!(event, egui::Event::Paste(_)))
            });
            self.actions.push(Action::PasteImage {
                width,
                height,
                rgba,
            });
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
            // Precision wheels can report points too. If egui has remapped
            // their vertical input to horizontal (Shift, including the rest
            // of an active gesture), keep that direction and native smoothing.
            let remapped = sum.y != 0.0
                && input.smooth_scroll_delta.x != 0.0
                && input.smooth_scroll_delta.y == 0.0;
            (sum, pointish && !remapped, ended)
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
        // egui already maps Shift + mouse wheel to the horizontal axis. The
        // raw wheel event still has a vertical delta, so applying the trackpad
        // axis lock to it would discard the remapped input (including its
        // smoothing tail). Discrete mouse-wheel input needs no gesture lock.
        if !self.scroll_from_trackpad {
            self.scroll_lock = None;
            return;
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
        self.flush_open_draft();
        self.recording = None;
        // A background resume would die with the process.
        if self.media_hold.take().is_some() {
            crate::media_pause::settle(Duration::from_secs(2));
        }
        self.backend.shutdown();
    }

    /// Stores the open chat's unsent text, which otherwise only moves into
    /// the archive when another chat opens.
    fn flush_open_draft(&self) {
        if let Some(chat) = self.open_chat.as_deref()
            && self.editing.is_none()
        {
            self.store_draft(chat, &self.composer);
        }
    }

    /// Returns attachment state for a loaded message.
    pub fn media_of(&self, chat: &str, id: &str) -> Option<&Media> {
        self.conversations.get(chat)?.message(id)?.content.media()
    }

    fn handle_clipboard_image(
        &mut self,
        result: Result<crate::model::DecodedImage, String>,
        writer: impl FnOnce(&crate::model::DecodedImage) -> Result<(), String>,
    ) {
        match result {
            Ok(image) => match writer(&image) {
                Ok(()) => self.toast(crate::i18n::gettext(self.locale, "Copied image")),
                Err(error) => {
                    log::warn!("failed to write image to clipboard: {error}");
                    self.toast_error(crate::i18n::gettext(self.locale, "Failed to copy image"));
                }
            },
            Err(error) => {
                log::warn!("failed to decode image for clipboard: {error}");
                self.toast_error(crate::i18n::gettext(self.locale, "Failed to copy image"));
            }
        }
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
        matches!(event, egui::Event::Paste(_))
            || matches!(
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

/// Writes decoded straight-alpha RGBA image bytes to the system clipboard.
///
/// The handle stays open: on X11 the copying process serves the data, and
/// dropping arboard's last handle leaves the image only to a clipboard
/// manager, if there is one.
fn write_clipboard_image(image: &crate::model::DecodedImage) -> Result<(), String> {
    thread_local! {
        static CLIPBOARD: std::cell::RefCell<Option<arboard::Clipboard>> =
            const { std::cell::RefCell::new(None) };
    }
    CLIPBOARD.with_borrow_mut(|slot| {
        let clipboard = match slot {
            Some(clipboard) => clipboard,
            None => slot.insert(arboard::Clipboard::new().map_err(|error| error.to_string())?),
        };
        clipboard
            .set_image(arboard::ImageData {
                width: image.width,
                height: image.height,
                bytes: std::borrow::Cow::Borrowed(&image.bytes),
            })
            .map_err(|error| error.to_string())
    })
}

impl Delivery {
    /// Whether an outgoing message is still pending.
    pub fn in_flight(self) -> bool {
        matches!(self, Delivery::Pending)
    }
}

/// Whether an incoming message in this chat warrants a desktop notification.
///
/// Archived chats stay silent, direct and group alike, and speak up again once
/// they are unarchived. Muted and locked chats give no signal that one arrived,
/// and delayed reconnect backlogs are not news.
/// The sound a notification plays. A chat's own sound wins, even for
/// mentions, so a chat set to no sound stays silent. Otherwise a group message
/// that mentions or answers us plays the mention sound, and other group
/// messages stay silent while group sounds are off.
fn notification_sound(
    settings: &Settings,
    chat_sound: Option<NotificationSound>,
    is_group: bool,
    for_us: bool,
) -> NotificationSound {
    match chat_sound {
        Some(sound) => sound,
        None if for_us => settings.mention_sound.clone(),
        None if is_group && !settings.group_sounds => NotificationSound::None,
        None => settings.message_sound.clone(),
    }
}

fn notification_eligible(chat: &Chat, now: i64, message_at: i64) -> bool {
    if chat.archived || chat.unread == 0 || chat.muted(now) || chat.locked {
        return false;
    }
    now - message_at <= 60
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ChatKind, Content, Media, MediaState, ToastKind};

    fn app() -> App {
        let root = std::env::temp_dir().join(format!("zapfast-app-{}", std::process::id()));
        App::headless(AppDirs::under(&root), Settings::default()).0
    }

    /// Demo and test runs share the machine with a linked ZapFast, whose real
    /// taskbar badge they must not overwrite.
    #[test]
    fn demo_and_test_runs_do_not_publish_a_taskbar_badge() {
        assert!(app().badge.is_none());
    }

    #[test]
    fn the_new_contact_box_remembers_whether_to_save_to_the_phone() {
        let mut app = app();
        let (backend, mut commands) = Backend::recording();
        app.backend = backend;
        let ctx = egui::Context::default();
        let to_phone = |commands: &mut tokio::sync::mpsc::UnboundedReceiver<Command>| {
            std::iter::from_fn(|| commands.try_recv().ok())
                .find_map(|command| match command {
                    Command::NewContact { to_phone, .. } => Some(to_phone),
                    _ => None,
                })
                .expect("the number is checked")
        };
        app.apply(Action::ShowDialog(Dialog::NewContact), &ctx);
        assert!(app.new_contact_to_phone, "on until turned off");
        let add = |to_phone, first: &str| Action::NewContact {
            phone: "15550002222".into(),
            first: first.into(),
            last: String::new(),
            to_phone,
        };
        app.apply(add(Some(false), "Ada"), &ctx);
        assert!(!to_phone(&mut commands));
        assert!(!app.settings.save_contacts_to_phone);
        assert!(app.settings_dirty);
        app.apply(Action::ShowDialog(Dialog::NewContact), &ctx);
        assert!(!app.new_contact_to_phone, "the next dialog starts from it");
        // Opening a chat without a name saves nothing, so it keeps the choice.
        app.apply(add(Some(true), ""), &ctx);
        assert!(to_phone(&mut commands));
        assert!(!app.settings.save_contacts_to_phone);
        // A shared contact's Add follows the last choice.
        app.apply(add(None, "Bob"), &ctx);
        assert!(!to_phone(&mut commands));
    }

    #[test]
    fn hiding_the_chat_list_always_collapses_it_to_avatars() {
        let mut app = app();
        let ctx = egui::Context::default();
        assert_eq!(app.sidebar_mode(), SidebarDisplayMode::Expanded);
        app.apply(Action::ToggleSidebar, &ctx);
        assert_eq!(
            app.sidebar_mode(),
            SidebarDisplayMode::CollapsedIconsOnly,
            "hiding the list leaves the avatar column"
        );
        app.apply(Action::ToggleSidebar, &ctx);
        assert_eq!(
            app.sidebar_mode(),
            SidebarDisplayMode::Expanded,
            "and brings the full list back"
        );
    }

    #[test]
    fn wallpaper_colors_remain_independent_between_themes() {
        let mut app = app();
        let ctx = egui::Context::default();
        let light_color = crate::settings::WallpaperColor::Cruise;
        let dark_color = crate::settings::WallpaperColor::Nordic;
        let original_dark_color = app.settings.dark_wallpaper_color;

        app.palette.dark = false;
        app.apply(Action::SetWallpaperColor(light_color), &ctx);
        assert_eq!(app.settings.wallpaper_color, light_color);
        assert_eq!(app.settings.dark_wallpaper_color, original_dark_color);
        assert!(app.settings_dirty);

        app.settings_dirty = false;
        app.palette.dark = true;
        app.apply(Action::SetWallpaperColor(dark_color), &ctx);
        assert_eq!(app.settings.dark_wallpaper_color, dark_color);
        assert_eq!(app.settings.wallpaper_color, light_color);
        assert!(app.settings_dirty);
    }

    fn local_pack(name: &str, dir: &str) -> StickerPack {
        StickerPack {
            name: name.into(),
            dir: PathBuf::from(dir),
            stickers: Vec::new(),
            local: true,
        }
    }

    fn stickers_event(packs: Vec<StickerPack>) -> Event {
        Event::Stickers {
            favorites: Vec::new(),
            packs,
            recent: Vec::new(),
            emojis: std::collections::HashMap::new(),
        }
    }

    #[test]
    fn a_new_pack_is_selected_once_the_backend_lists_it() {
        let root = tempfile::tempdir().unwrap();
        let (mut app, events) = App::headless(AppDirs::under(root.path()), Settings::default());
        let ctx = egui::Context::default();
        app.apply(Action::CreateStickerPack("   ".into()), &ctx);
        app.apply(Action::CreateStickerPack("  Bom dia  ".into()), &ctx);
        assert_eq!(app.sticker_shelf, StickerShelf::Recent, "no folder yet");
        events.send(stickers_event(Vec::new())).unwrap();
        app.handle_events();
        assert_eq!(app.sticker_shelf, StickerShelf::Recent, "an update waits");
        events
            .send(stickers_event(vec![
                local_pack("Bom dia", "/packs/Bom dia 2"),
                local_pack("Bom dia", "/packs/Bom dia"),
            ]))
            .unwrap();
        app.handle_events();
        assert_eq!(
            app.sticker_shelf,
            StickerShelf::Pack(PathBuf::from("/packs/Bom dia 2")),
            "the trimmed name picks the newest pack of that name"
        );
        app.apply(Action::SelectStickerShelf(StickerShelf::Favorites), &ctx);
        assert_eq!(app.sticker_shelf, StickerShelf::Favorites);
    }

    #[test]
    fn a_pack_that_vanished_cannot_stay_selected() {
        let root = tempfile::tempdir().unwrap();
        let (mut app, events) = App::headless(AppDirs::under(root.path()), Settings::default());
        events
            .send(stickers_event(vec![local_pack(
                "Bom dia",
                "/packs/Bom dia",
            )]))
            .unwrap();
        app.handle_events();
        let ctx = egui::Context::default();
        app.apply(
            Action::SelectStickerShelf(StickerShelf::Pack(PathBuf::from("/packs/Bom dia"))),
            &ctx,
        );
        assert!(app.selected_pack().is_some());
        events.send(stickers_event(Vec::new())).unwrap();
        app.handle_events();
        assert_eq!(
            app.sticker_shelf,
            StickerShelf::Recent,
            "a pack deleted elsewhere cannot stay open"
        );
    }

    fn paste_release() -> egui::Event {
        egui::Event::Key {
            key: egui::Key::V,
            physical_key: None,
            pressed: false,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        }
    }

    fn clipboard_frame(
        app: &mut App,
        ctx: &egui::Context,
        mut events: Vec<egui::Event>,
        image: bool,
    ) -> usize {
        let mut reads = 0;
        events.insert(0, egui::Event::ModifiersChanged(egui::Modifiers::COMMAND));
        let mut output = ctx.run_ui(
            egui::RawInput {
                events,
                ..Default::default()
            },
            |ui| {
                app.take_image_paste(ui.ctx(), || {
                    reads += 1;
                    image.then(|| (2, 2, vec![200; 16]))
                });
                ui.add(
                    egui::TextEdit::singleline(&mut app.composer)
                        .id(egui::Id::new("composer-text")),
                );
                app.apply_actions(ui.ctx());
            },
        );
        output.textures_delta.clear();
        reads
    }

    fn clipboard_app() -> (App, egui::Context) {
        let mut app = app();
        app.open_chat = Some("fixture".into());
        app.chats
            .push(Chat::new("fixture".into(), "Fixture".into()));
        app.composer = "caption".into();
        let ctx = egui::Context::default();
        ctx.memory_mut(|memory| memory.request_focus(egui::Id::new("composer-text")));
        clipboard_frame(&mut app, &ctx, vec![], false);
        (app, ctx)
    }

    #[test]
    fn image_paste_consumes_source_text_and_stages_once_across_frames() {
        let (mut app, ctx) = clipboard_app();
        let reads = clipboard_frame(
            &mut app,
            &ctx,
            vec![egui::Event::Paste("https://example.org/picture.png".into())],
            true,
        );
        assert_eq!(reads, 1);
        assert_eq!(app.pending.len(), 1);
        assert_eq!(app.composer, "caption");
        assert_eq!(
            clipboard_frame(&mut app, &ctx, vec![paste_release()], true),
            0
        );
        assert_eq!(
            app.pending.len(),
            1,
            "release must not duplicate the picture"
        );
        assert_eq!(
            clipboard_frame(&mut app, &ctx, vec![paste_release()], true),
            1
        );
        assert_eq!(app.pending.len(), 2, "a later image-only paste still works");
    }

    #[test]
    fn a_menu_paste_does_not_suppress_a_later_image_only_shortcut() {
        let (mut app, ctx) = clipboard_app();
        let mut output = ctx.run_ui(
            egui::RawInput {
                events: vec![
                    egui::Event::ModifiersChanged(egui::Modifiers::NONE),
                    egui::Event::Paste("fixture URL".into()),
                ],
                ..Default::default()
            },
            |ui| {
                app.take_image_paste(ui.ctx(), || Some((2, 2, vec![200; 16])));
                app.apply_actions(ui.ctx());
            },
        );
        output.textures_delta.clear();
        assert_eq!(app.pending.len(), 1);
        clipboard_frame(&mut app, &ctx, vec![paste_release()], true);
        assert_eq!(app.pending.len(), 2);
    }

    #[test]
    fn image_paste_handles_press_and_release_in_one_frame() {
        let (mut app, ctx) = clipboard_app();
        clipboard_frame(
            &mut app,
            &ctx,
            vec![
                egui::Event::Paste("<img src='fixture'>".into()),
                paste_release(),
            ],
            true,
        );
        assert_eq!(app.pending.len(), 1);
        assert_eq!(app.composer, "caption");
        clipboard_frame(&mut app, &ctx, vec![paste_release()], true);
        assert_eq!(app.pending.len(), 2);
    }

    #[test]
    fn text_paste_is_preserved_when_the_clipboard_has_no_image() {
        let (mut app, ctx) = clipboard_app();
        clipboard_frame(
            &mut app,
            &ctx,
            vec![egui::Event::Paste("plain text".into())],
            false,
        );
        assert!(app.composer.contains("plain text"));
        assert!(app.pending.is_empty());
        assert_eq!(
            clipboard_frame(&mut app, &ctx, vec![paste_release()], true),
            0,
            "a clipboard change before release must not stage an unrelated image"
        );
        assert!(app.pending.is_empty());
    }

    #[test]
    fn image_paste_only_reads_the_clipboard_for_a_writable_composer() {
        for state in ["search", "dialog", "settings", "read-only", "closed"] {
            let (mut app, ctx) = clipboard_app();
            match state {
                "search" => ctx.memory_mut(|memory| memory.request_focus(egui::Id::new("search"))),
                "dialog" => app.dialog = Some(Dialog::NewContact),
                "settings" => app.page = Page::Settings,
                "read-only" => app.chats[0].read_only = true,
                "closed" => app.open_chat = None,
                _ => unreachable!(),
            }
            let reads = clipboard_frame(
                &mut app,
                &ctx,
                vec![egui::Event::Paste("fixture".into())],
                true,
            );
            assert_eq!(reads, 0, "{state}");
            assert!(app.pending.is_empty(), "{state}");
            assert!(
                ctx.input(|input| input
                    .events
                    .iter()
                    .any(|event| matches!(event, egui::Event::Paste(_)))),
                "{state}"
            );
        }
    }

    #[test]
    fn interactive_send_events_release_only_the_matching_message() {
        let directory = tempfile::tempdir().unwrap();
        let (mut app, events) =
            App::headless(AppDirs::under(directory.path()), Settings::default());
        let ctx = egui::Context::default();
        for id in ["first", "second"] {
            events
                .send(Event::InteractiveReplyState {
                    chat: "chat".into(),
                    message: id.into(),
                    pending: true,
                })
                .unwrap();
        }
        app.background_frame(&ctx);
        assert_eq!(app.interactive_sending.len(), 2);
        events
            .send(Event::InteractiveReplyState {
                chat: "chat".into(),
                message: "first".into(),
                pending: false,
            })
            .unwrap();
        app.background_frame(&ctx);
        assert!(
            !app.interactive_sending
                .contains(&("chat".into(), "first".into()))
        );
        assert!(
            app.interactive_sending
                .contains(&("chat".into(), "second".into()))
        );
        events.send(Event::Link(LinkStatus::LoggedOut)).unwrap();
        app.background_frame(&ctx);
        assert!(app.interactive_sending.is_empty());
    }

    #[test]
    fn archived_chats_do_not_qualify_for_notifications_until_unarchived() {
        let now = crate::util::now();
        for (id, kind) in [
            ("1@s.whatsapp.net", ChatKind::Direct),
            ("2@g.us", ChatKind::Group),
        ] {
            let mut chat = Chat::new(id.into(), "Fixture".into());
            assert_eq!(chat.kind, kind, "fixture id picks the chat kind");
            chat.unread = 1;

            chat.archived = true;
            assert!(!notification_eligible(&chat, now, now), "{id} archived");

            chat.archived = false;
            assert!(notification_eligible(&chat, now, now), "{id} unarchived");
        }
    }

    #[test]
    fn image_preview_opens_zooms_fits_and_closes() {
        let ctx = egui::Context::default();
        let mut app = app();
        let file = tempfile::NamedTempFile::with_suffix(".png").unwrap();
        std::fs::write(file.path(), b"not a real image").unwrap();

        app.apply(Action::PreviewImage(file.path().to_owned()), &ctx);
        let preview = app.image_preview.as_ref().expect("preview opens");
        assert_eq!(preview.path(), file.path());
        assert!(preview.is_fit());

        app.apply(Action::ZoomImageIn, &ctx);
        assert_eq!(app.image_preview.as_ref().unwrap().zoom(), 1.25);
        app.apply(Action::FitImage, &ctx);
        assert!(app.image_preview.as_ref().unwrap().is_fit());

        app.image_preview.as_mut().unwrap().zoom_in();
        app.apply(Action::CloseImagePreview, &ctx);
        assert!(app.image_preview.is_none());
        assert!(app.dialog.is_none());
    }

    #[test]
    fn copying_an_image_dispatches_background_decode() {
        let mut app = app();
        let (backend, mut commands) = Backend::recording();
        app.backend = backend;
        let ctx = egui::Context::default();
        let path = std::path::PathBuf::from("sample-photo.png");

        app.apply(Action::CopyImage(path.clone()), &ctx);
        assert!(matches!(
            commands.try_recv(),
            Ok(Command::PrepareClipboardImage(p)) if p == path
        ));
    }

    #[test]
    fn clipboard_image_event_preserves_pixels_and_toasts() {
        let mut app = app();
        let pixels = vec![
            255, 0, 0, 255, // red
            0, 255, 0, 255, // green
            0, 0, 255, 255, // blue
            255, 255, 0, 255, // yellow
        ];
        let decoded = crate::model::DecodedImage {
            width: 2,
            height: 2,
            bytes: pixels.clone(),
        };

        let mut written = None;
        app.handle_clipboard_image(Ok(decoded), |image| {
            written = Some((image.width, image.height, image.bytes.clone()));
            Ok(())
        });

        assert_eq!(written, Some((2, 2, pixels)));
        assert_eq!(app.toasts.len(), 1);
        assert_eq!(app.toasts[0].message, "Copied image");
        assert_eq!(app.toasts[0].kind, ToastKind::Info);

        // Failure during clipboard write surfaces an error toast.
        app.toasts.clear();
        let decoded_err = crate::model::DecodedImage {
            width: 1,
            height: 1,
            bytes: vec![0; 4],
        };
        app.handle_clipboard_image(Ok(decoded_err), |_| Err("OS clipboard locked".into()));
        assert_eq!(app.toasts.len(), 1);
        assert_eq!(app.toasts[0].message, "Failed to copy image");
        assert_eq!(app.toasts[0].kind, ToastKind::Error);

        // Failure during background decoding also surfaces an error toast.
        app.toasts.clear();
        app.handle_clipboard_image(Err("Corrupt image data".into()), |_| unreachable!());
        assert_eq!(app.toasts.len(), 1);
        assert_eq!(app.toasts[0].message, "Failed to copy image");
        assert_eq!(app.toasts[0].kind, ToastKind::Error);
    }

    #[test]
    fn a_hidden_start_starts_the_backend_without_a_frame() {
        let root = tempfile::tempdir().unwrap();
        let (backend, mut started) = Backend::detached_with_startup();
        let mut app = App::with_backend(
            AppDirs::under(root.path()),
            Settings::default(),
            backend,
            Waker::default(),
        );
        assert!(started.try_recv().is_err(), "nothing starts before asked");
        app.start_hidden();
        assert!(app.hide_intent);
        assert_eq!(started.try_recv(), Ok(()));
    }

    /// The shell asks the app what a closed window means and what each
    /// headless tick wants; without a tray a hidden start opens the window.
    #[test]
    fn the_shell_hides_shows_and_quits_as_the_app_asks() {
        use fastframe_shell::{Closed, Headless, Resident};
        let mut app = app();
        assert_eq!(app.closed(), Closed::Quit);
        app.hide_intent = true;
        assert_eq!(app.closed(), Closed::Hide);
        app.quit_requested = true;
        assert_eq!(app.closed(), Closed::Quit);
        app.quit_requested = false;
        Resident::window_gone(&mut app);
        assert!(!app.hide_intent && !app.wants_show);
        let ctx = egui::Context::default();
        assert_eq!(app.headless_frame(&ctx), Headless::Wait);
        app.wants_show = true;
        assert_eq!(app.headless_frame(&ctx), Headless::Show);
        app.quit_requested = true;
        assert_eq!(app.headless_frame(&ctx), Headless::Quit);

        let mut app = self::app();
        assert!(app.tray.is_none());
        assert!(!Resident::start_hidden(&mut app), "no tray, no way back");
        assert!(!app.hide_intent);
    }

    #[test]
    fn unsupported_media_falls_back_to_the_external_opener() {
        let ctx = egui::Context::default();
        let mut app = app();
        let file = tempfile::NamedTempFile::with_suffix(".heic").unwrap();
        std::fs::write(file.path(), b"not a real image").unwrap();

        app.apply(Action::PreviewImage(file.path().to_owned()), &ctx);

        assert!(
            app.image_preview.is_none(),
            "no preview for unsupported media"
        );
        assert!(
            app.actions
                .iter()
                .any(|action| matches!(action, Action::OpenFile(path) if path == file.path())),
            "the external opener is queued instead"
        );
    }

    #[test]
    fn marking_unread_uses_the_empty_dot_until_the_chat_opens() {
        let mut app = app();
        let id = "1@s.whatsapp.net".to_owned();
        app.chats.push(Chat::new(id.clone(), "Ada".to_owned()));
        app.mark_unread(&id);
        let chat = app.chat(&id).expect("chat");
        assert!(chat.marked_unread);
        assert_eq!(chat.unread, 0);
        assert!(chat.looks_unread());
        app.open_chat(id.clone());
        let chat = app.chat(&id).expect("chat");
        assert!(!chat.marked_unread);
        assert!(!chat.looks_unread());
    }

    #[test]
    fn marking_unread_leaves_a_real_count_alone() {
        let mut app = app();
        let id = "1@s.whatsapp.net".to_owned();
        let mut chat = Chat::new(id.clone(), "Ada".to_owned());
        chat.unread = 3;
        app.chats.push(chat);
        app.mark_unread(&id);
        let chat = app.chat(&id).expect("chat");
        assert_eq!(chat.unread, 3);
        assert!(!chat.marked_unread);
    }

    #[test]
    fn leaving_a_group_marks_it_read_only_and_can_archive() {
        let mut app = app();
        let me = "me@s.whatsapp.net";
        app.me = Some(me.into());
        let id = "1-2@g.us".to_owned();
        let mut chat = Chat::new(id.clone(), "Rust".into());
        chat.participants = vec![me.into(), "other@s.whatsapp.net".into()];
        app.chats.push(chat);
        app.open_chat = Some(id.clone());
        app.dialog = Some(Dialog::ConfirmLeaveGroup(id.clone()));
        let ctx = egui::Context::default();
        app.apply(
            Action::LeaveGroup {
                chat: id.clone(),
                archive: false,
            },
            &ctx,
        );
        let chat = app.chat(&id).expect("chat");
        assert!(chat.read_only);
        assert!(!chat.participants.iter().any(|id| id == me));
        assert!(!chat.archived);
        assert_eq!(app.open_chat.as_deref(), Some(id.as_str()));
        assert!(app.dialog.is_none());
        // The chat no longer offers leave once we are out of it.
        assert!(!chat.can_leave(&app.our_ids()));
        app.apply(
            Action::LeaveGroup {
                chat: id.clone(),
                archive: true,
            },
            &ctx,
        );
        let chat = app.chat(&id).expect("chat");
        assert!(!chat.archived, "the archive waits for the phone");
        assert_eq!(
            app.open_chat.as_deref(),
            Some(id.as_str()),
            "and the conversation stays open until then"
        );
        // The phone agreed, so the archive lands and the open chat closes.
        let mut confirmed = chat.clone();
        confirmed.archived = true;
        app.handle_chat_updated(confirmed);
        app.apply_actions(&ctx);
        assert!(app.chat(&id).expect("chat").archived);
        assert!(app.open_chat.is_none(), "the confirmed archive closes it");
    }

    #[test]
    fn a_refused_leave_rolls_back_the_local_mark() {
        let mut app = app();
        let me = "me@s.whatsapp.net";
        app.me = Some(me.into());
        let id = "1-2@g.us".to_owned();
        let mut chat = Chat::new(id.clone(), "Rust".into());
        chat.participants = vec![me.into(), "other@s.whatsapp.net".into()];
        app.chats.push(chat.clone());
        let ctx = egui::Context::default();
        app.apply(
            Action::LeaveGroup {
                chat: id.clone(),
                archive: false,
            },
            &ctx,
        );
        assert!(
            app.chat(&id).expect("chat").read_only,
            "the menu marks the chat at once"
        );
        // The worker could not reach the phone, so it sends the archive row
        // back untouched. The mark has to go with it.
        app.handle_chat_updated(chat);
        let chat = app.chat(&id).expect("chat");
        assert!(!chat.read_only, "the refused leave is rolled back");
        assert!(!chat.left, "and so is the leave itself");
        assert!(chat.participants.iter().any(|id| id == me));
        assert!(chat.can_leave(&app.our_ids()), "and it can be tried again");
    }

    #[test]
    fn leaving_a_channel_marks_it_read_only_and_can_archive() {
        let mut app = app();
        let id = "1@newsletter".to_owned();
        let chat = Chat::new(id.clone(), "News".into());
        app.chats.push(chat);
        app.open_chat = Some(id.clone());
        app.dialog = Some(Dialog::ConfirmLeaveGroup(id.clone()));
        let ctx = egui::Context::default();
        app.apply(
            Action::LeaveGroup {
                chat: id.clone(),
                archive: false,
            },
            &ctx,
        );
        let chat = app.chat(&id).expect("chat");
        assert!(chat.read_only);
        assert!(!chat.archived);
        assert_eq!(app.open_chat.as_deref(), Some(id.as_str()));
        assert!(app.dialog.is_none());
        assert!(!chat.can_leave(&app.our_ids()));
        app.apply(
            Action::LeaveGroup {
                chat: id.clone(),
                archive: true,
            },
            &ctx,
        );
        let chat = app.chat(&id).expect("chat");
        assert!(!chat.archived, "the archive waits for the phone");
        assert_eq!(
            app.open_chat.as_deref(),
            Some(id.as_str()),
            "and the conversation stays open until then"
        );
        // The phone agreed, so the archive lands and the open chat closes.
        let mut confirmed = chat.clone();
        confirmed.archived = true;
        app.handle_chat_updated(confirmed);
        app.apply_actions(&ctx);
        assert!(app.chat(&id).expect("chat").archived);
        assert!(app.open_chat.is_none(), "the confirmed archive closes it");
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

    fn label(id: &str, name: &str) -> Label {
        Label {
            id: id.to_owned(),
            name: name.to_owned(),
            color_hex: "#3b82f6".to_owned(),
            created_at: 1,
        }
    }

    fn labeled(id: &str, unread: u32, labels: &[&str]) -> Chat {
        let mut chat = Chat::new(id.to_owned(), id.to_owned());
        chat.unread = unread;
        chat.labels = labels.iter().map(|label| (*label).to_owned()).collect();
        chat.last_activity = i64::from(unread) + 1;
        chat
    }

    #[test]
    fn picking_a_chip_lets_go_of_the_label() {
        let ctx = egui::Context::default();
        let mut app = app();
        app.labels = vec![label("label-1", "Work")];
        app.apply(Action::SelectLabel(Some("label-1".into())), &ctx);
        app.apply(Action::SetChatFilter(ChatFilter::Groups), &ctx);
        assert!(app.label_filter.is_none());
        assert_eq!(app.chat_filter, ChatFilter::Groups);
    }

    #[test]
    fn a_label_lists_its_channels_and_leaves_the_archive_alone() {
        let ctx = egui::Context::default();
        let mut app = app();
        app.labels = vec![label("label-1", "Work")];
        let channel = labeled("1@newsletter", 0, &["label-1"]);
        let mut archived = labeled("2@s.whatsapp.net", 0, &["label-1"]);
        archived.archived = true;
        app.chats = vec![channel, archived, labeled("3@s.whatsapp.net", 0, &[])];
        app.apply(Action::ShowArchived(true), &ctx);
        app.apply(Action::SelectLabel(Some("label-1".into())), &ctx);
        assert!(!app.show_archived, "a label chip leaves the archive");
        let listed: Vec<String> = app
            .visible_chats()
            .into_iter()
            .map(|chat| chat.id.clone())
            .collect();
        assert_eq!(listed, ["1@newsletter".to_owned()]);
        app.apply(Action::ShowArchived(true), &ctx);
        assert_eq!(
            app.visible_chats().len(),
            1,
            "the archive lists every archived chat, labelled or not"
        );
    }

    #[test]
    fn a_taken_label_name_is_refused_in_the_app() {
        let ctx = egui::Context::default();
        let mut app = app();
        app.labels = vec![label("label-1", "Work")];
        app.label_name = "work".into();
        app.apply(
            Action::CreateLabel {
                name: " work ".into(),
                color_hex: "#3b82f6".into(),
            },
            &ctx,
        );
        assert_eq!(app.label_name, "work", "the typed name stays to be fixed");
        assert!(
            app.toasts
                .iter()
                .any(|toast| toast.kind == ToastKind::Error)
        );
    }

    #[test]
    fn a_label_chip_replaces_the_chip_filter() {
        let ctx = egui::Context::default();
        let mut app = app();
        app.labels = vec![label("label-1", "Work")];
        app.chat_filter = ChatFilter::Unread;
        app.apply(Action::SelectLabel(Some("label-1".into())), &ctx);
        assert_eq!(app.label_filter.as_deref(), Some("label-1"));
        assert_eq!(
            app.chat_filter,
            ChatFilter::All,
            "a label replaces what the chips were filtering"
        );
    }

    #[test]
    fn a_label_counts_its_unread_chats() {
        let mut app = app();
        app.labels = vec![label("label-1", "Work")];
        let mut archived = labeled("3@s.whatsapp.net", 5, &["label-1"]);
        archived.archived = true;
        let mut locked = labeled("5@s.whatsapp.net", 4, &["label-1"]);
        locked.locked = true;
        app.chats = vec![
            labeled("1@s.whatsapp.net", 3, &["label-1"]),
            labeled("2@s.whatsapp.net", 2, &["label-1", "label-2"]),
            labeled("4@s.whatsapp.net", 7, &[]),
            labeled("6@s.whatsapp.net", 0, &["label-1"]),
            archived,
            locked,
        ];
        assert_eq!(
            app.label_unread("label-1"),
            2,
            "unread chats, as the other chips count; archived and locked ones stay out"
        );
        assert!(app.chat_wears(&app.chats[1], "label-2"));
        assert!(!app.chat_wears(&app.chats[2], "label-1"));
    }

    #[test]
    fn the_list_shows_only_the_chosen_label() {
        let mut app = app();
        app.labels = vec![label("label-1", "Work")];
        app.chats = vec![
            labeled("1@s.whatsapp.net", 0, &["label-1"]),
            labeled("2@s.whatsapp.net", 0, &[]),
        ];
        app.label_filter = Some("label-1".into());
        let listed: Vec<String> = app
            .visible_chats()
            .into_iter()
            .map(|chat| chat.id.clone())
            .collect();
        assert_eq!(listed, vec!["1@s.whatsapp.net".to_owned()]);
        app.label_filter = None;
        assert_eq!(
            app.visible_chats().len(),
            2,
            "without a label every chat shows"
        );
    }

    #[test]
    fn a_gone_label_leaves_the_list_showing_all_chats() {
        let root = std::env::temp_dir().join(format!("zapfast-labels-{}", std::process::id()));
        let (mut app, events) = App::headless(AppDirs::under(&root), Settings::default());
        let ctx = egui::Context::default();
        app.labels = vec![label("label-1", "Work"), label("label-2", "Home")];
        app.label_filter = Some("label-2".into());
        app.label_editing = Some(("label-2".into(), "Hous".into()));
        events
            .send(Event::Labels(vec![label("label-1", "Work")]))
            .unwrap();
        app.background_frame(&ctx);
        assert!(
            app.label_filter.is_none(),
            "the list falls back to showing every chat"
        );
        assert!(
            app.label_editing.is_none(),
            "the editor let go of the label"
        );
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
    fn tray_clicks_show_hide_and_quit() {
        use fastframe_tray::Event;
        assert!(matches!(
            super::tray_action(Event::Show, false),
            Some(Action::ShowWindow)
        ));
        for event in [Event::Toggle, Event::Menu(super::TRAY_SHOW)] {
            assert!(matches!(
                super::tray_action(event, true),
                Some(Action::ShowWindow)
            ));
            assert!(matches!(
                super::tray_action(event, false),
                Some(Action::HideWindow)
            ));
        }
        assert!(matches!(
            super::tray_action(Event::Menu(super::TRAY_QUIT), false),
            Some(Action::Quit)
        ));
        assert!(super::tray_action(Event::Menu("other"), false).is_none());
        let menu = super::tray_config().menu;
        assert_eq!(
            menu,
            [
                fastframe_tray::MenuItem::action(super::TRAY_SHOW, "Show or hide ZapFast"),
                fastframe_tray::MenuItem::Separator,
                fastframe_tray::MenuItem::action(super::TRAY_QUIT, "Quit"),
            ]
        );
    }

    #[test]
    fn custom_theme_cache_survives_a_missing_file_and_follows_system_updates() {
        use crate::theme::{Catalog, CustomTheme};
        let mut app = app();
        let ctx = egui::Context::default();
        let mut first = CustomTheme {
            filename: "mine.json".into(),
            palette: Palette::dark(),
        };
        first.palette.accent = egui::Color32::RED;
        app.custom_themes = Catalog::preview(vec![first.clone()], false);
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
        app.custom_themes = Catalog::preview(vec![system.clone()], true);
        app.cache_custom_themes();
        assert_eq!(app.settings.system_theme_cache.as_ref(), Some(&system));
        app.apply_theme(&ctx);
        assert_eq!(app.palette.accent, egui::Color32::GREEN);
        app.apply(Action::SetTheme(ThemeChoice::Light), &ctx);
        assert_eq!(app.palette, Palette::light());
    }

    #[test]
    fn automatic_updates_require_opt_in_and_explicit_restart() {
        use crate::updates::{DownloadState, Installation, Kind, Prepared};
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
        app.update_download =
            DownloadState::Ready(Box::new(Prepared::sample(installation, "99.0.0")));
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
    fn message_info_follows_a_group_messages_receipts_only_while_open() {
        let mut app = app();
        let (backend, mut commands, events) = Backend::recording_with_events();
        app.backend = backend;
        let ctx = egui::Context::default();
        let group = "123-456@g.us";
        let watches = |commands: &mut tokio::sync::mpsc::UnboundedReceiver<Command>| {
            std::iter::from_fn(|| commands.try_recv().ok())
                .filter_map(|command| match command {
                    Command::WatchReceipts(watch) => Some(watch),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        let receipts = |message: &str| crate::model::MessageReceipts {
            chat: group.into(),
            message: message.into(),
            recipients: Vec::new(),
        };
        app.apply(
            Action::ShowDialog(Dialog::MessageInfo {
                chat: group.into(),
                message: "m".into(),
            }),
            &ctx,
        );
        app.follow_receipts();
        app.follow_receipts();
        assert_eq!(
            watches(&mut commands),
            [Some((group.to_owned(), "m".to_owned()))]
        );
        // Receipts for another message, from a dialog opened earlier, are stale.
        events.send(Event::Receipts(receipts("other"))).unwrap();
        app.handle_events();
        assert!(app.message_receipts.is_none());
        events.send(Event::Receipts(receipts("m"))).unwrap();
        app.handle_events();
        assert_eq!(app.message_receipts, Some(receipts("m")));
        app.apply(Action::CloseDialog, &ctx);
        app.follow_receipts();
        assert_eq!(watches(&mut commands), [None]);
        assert!(app.message_receipts.is_none());
        // A direct message's times are on its row: nothing to follow.
        app.apply(
            Action::ShowDialog(Dialog::MessageInfo {
                chat: "1@s.whatsapp.net".into(),
                message: "m".into(),
            }),
            &ctx,
        );
        app.follow_receipts();
        assert!(watches(&mut commands).is_empty());
    }

    #[test]
    fn drafts_come_back_after_a_restart_and_leave_with_the_account() {
        let mut app = app();
        let (backend, mut commands, events) = Backend::recording_with_events();
        app.backend = backend;
        let (open, other) = ("1@s.whatsapp.net", "2@s.whatsapp.net");
        app.open_chat = Some(open.into());
        events
            .send(Event::Drafts(vec![
                (open.into(), "half a reply".into()),
                (other.into(), "later".into()),
            ]))
            .unwrap();
        app.handle_events();
        assert_eq!(app.composer, "half a reply", "the reopened chat shows it");
        assert_eq!(app.drafts.get(other).map(String::as_str), Some("later"));
        // Quitting stores what is in the composer.
        app.composer = "half a reply, finished".into();
        app.shutdown();
        let saved: Vec<(String, String)> = std::iter::from_fn(|| commands.try_recv().ok())
            .filter_map(|command| match command {
                Command::SaveDraft { chat, text } => Some((chat, text)),
                _ => None,
            })
            .collect();
        assert_eq!(
            saved,
            [(open.to_owned(), "half a reply, finished".to_owned())]
        );
        // Unlinking forgets every draft.
        events.send(Event::Link(LinkStatus::LoggedOut)).unwrap();
        app.handle_events();
        assert!(app.drafts.is_empty());
        assert!(app.composer.is_empty());
    }

    #[test]
    fn selected_messages_forward_together_in_chat_order() {
        let mut app = app();
        let (backend, mut commands) = Backend::recording();
        app.backend = backend;
        let ctx = egui::Context::default();
        let chat = "1@s.whatsapp.net";
        app.chats = vec![Chat::new(chat.into(), "Ada".into())];
        app.open_chat = Some(chat.into());
        app.conversations.entry(chat.into()).or_default().merge(
            vec![
                message(chat, "first", 1),
                message(chat, "second", 2),
                message(chat, "third", 3),
            ],
            false,
        );
        app.apply(Action::SelectMessage("third".into()), &ctx);
        app.apply(Action::ToggleSelected("first".into()), &ctx);
        assert_eq!(
            app.selection,
            Some((chat.into(), vec!["first".into(), "third".into()]))
        );
        app.apply(
            Action::Forward {
                from_chat: chat.into(),
                messages: vec!["first".into(), "third".into()],
                to_chat: "2@s.whatsapp.net".into(),
            },
            &ctx,
        );
        let forwarded: Vec<String> = std::iter::from_fn(|| commands.try_recv().ok())
            .flat_map(|command| match command {
                Command::Forward { messages, .. } => messages,
                _ => Vec::new(),
            })
            .collect();
        assert_eq!(forwarded, ["first", "third"]);
        assert!(app.selection.is_none());
        // Shift-click selects everything between the last click and this one,
        // skipping what cannot be forwarded.
        let mut deleted = message(chat, "gone", 4);
        deleted.content = Content::Revoked;
        app.conversations
            .get_mut(chat)
            .unwrap()
            .merge(vec![deleted, message(chat, "fifth", 5)], false);
        app.apply(Action::SelectMessage("second".into()), &ctx);
        app.apply(Action::SelectRange("fifth".into()), &ctx);
        assert_eq!(
            app.selection.as_ref().map(|(_, ids)| ids.clone()),
            Some(vec!["second".into(), "third".into(), "fifth".into()])
        );
        app.apply(Action::CancelSelection, &ctx);
        // Unselecting the last message leaves selection mode.
        app.apply(Action::SelectMessage("second".into()), &ctx);
        app.apply(Action::ToggleSelected("second".into()), &ctx);
        assert!(app.selection.is_none());
    }

    #[test]
    fn mentions_and_other_messages_keep_their_own_notification_sound() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.apply(
            Action::SetNotificationSound {
                mention: true,
                sound: NotificationSound::None,
            },
            &ctx,
        );
        assert_eq!(app.settings.mention_sound, NotificationSound::None);
        assert_eq!(app.settings.message_sound, NotificationSound::Receive);
    }

    #[test]
    fn a_group_message_addresses_us_by_phone_number_privacy_id_or_reply() {
        let mut app = app();
        app.me = Some("15550001111@s.whatsapp.net".into());
        app.me_lid = Some("98765@lid".into());
        let group = "fixture@g.us";
        let mention = |user: &str, id: &str| crate::model::MentionRef {
            user: user.into(),
            id: id.into(),
        };
        let mut plain = message(group, "plain", 1);
        plain.sender = "15550002222@s.whatsapp.net".into();
        assert!(!app.addresses_us(&plain));

        let mut by_number = plain.clone();
        by_number.mentions = vec![mention("15550001111", "15550001111@s.whatsapp.net")];
        assert!(app.addresses_us(&by_number));

        // Once the worker knows our pair, a privacy-id mention arrives under
        // the phone number; before that, as the privacy id itself.
        let mut by_privacy_id = plain.clone();
        by_privacy_id.mentions = vec![mention("98765", "98765@lid")];
        assert!(app.addresses_us(&by_privacy_id));
        app.me_lid = None;
        assert!(
            !app.addresses_us(&by_privacy_id),
            "an unknown privacy id is someone else"
        );
        app.me_lid = Some("98765@lid".into());

        let mut someone_else = plain.clone();
        someone_else.mentions = vec![mention("15550002222", "15550002222@s.whatsapp.net")];
        assert!(!app.addresses_us(&someone_else));

        let quote = |sender: &str| crate::model::Quoted {
            id: "earlier".into(),
            sender: sender.into(),
            sender_name: None,
            summary: "earlier".into(),
            mentions: Vec::new(),
        };
        let mut reply = plain.clone();
        reply.quoted = Some(quote("15550001111@s.whatsapp.net"));
        assert!(app.addresses_us(&reply));
        reply.quoted = Some(quote("98765@lid"));
        assert!(app.addresses_us(&reply));
        reply.quoted = Some(quote("15550002222@s.whatsapp.net"));
        assert!(!app.addresses_us(&reply));
    }

    #[test]
    fn mentions_sound_even_in_quiet_groups_and_chat_sounds_win() {
        let mut settings = Settings::default();
        let sound = |settings: &Settings, chat, group, for_us| {
            notification_sound(settings, chat, group, for_us)
        };
        // Pidgin's model: the message sound everywhere, the alert when
        // someone addresses us in a group.
        assert_eq!(
            sound(&settings, None, false, false),
            NotificationSound::Receive
        );
        assert_eq!(
            sound(&settings, None, true, false),
            NotificationSound::Receive
        );
        assert_eq!(sound(&settings, None, true, true), NotificationSound::Alert);

        settings.group_sounds = false;
        assert_eq!(sound(&settings, None, true, false), NotificationSound::None);
        assert_eq!(sound(&settings, None, true, true), NotificationSound::Alert);
        assert_eq!(
            sound(&settings, None, false, false),
            NotificationSound::Receive,
            "one-to-one chats are not groups"
        );

        // A chat's own sound covers every message in it, mentions included.
        let custom = NotificationSound::Custom("/sounds/ding.wav".into());
        assert_eq!(sound(&settings, Some(custom.clone()), true, false), custom);
        assert_eq!(sound(&settings, Some(custom.clone()), true, true), custom);
        assert_eq!(
            sound(&settings, Some(NotificationSound::None), true, true),
            NotificationSound::None
        );
    }

    #[test]
    fn opening_an_unread_chat_remembers_where_its_unread_messages_begin() {
        let mut app = app();
        let mut busy = Chat::new("1@s.whatsapp.net".into(), "Ada".into());
        busy.unread = 4;
        let quiet = Chat::new("2@s.whatsapp.net".into(), "Bob".into());
        app.chats = vec![busy, quiet];
        app.open_chat("1@s.whatsapp.net".into());
        assert_eq!(
            app.unread_divider.as_ref().map(|divider| divider.count),
            Some(4)
        );
        assert_eq!(app.chat("1@s.whatsapp.net").unwrap().unread, 0);
        // Reopening the same chat keeps it; another chat without unread clears it.
        app.open_chat("1@s.whatsapp.net".into());
        assert!(app.unread_divider.is_some());
        app.open_chat("2@s.whatsapp.net".into());
        assert!(app.unread_divider.is_none());
    }

    #[test]
    fn an_invite_link_is_previewed_and_joined_inside_the_app() {
        use crate::model::{InviteInfo, InviteState};
        let mut app = app();
        let (backend, mut commands) = Backend::recording();
        app.backend = backend;
        let ctx = egui::Context::default();
        app.apply(
            Action::OpenUrl("https://chat.whatsapp.com/AbCdEf1234567890XyZ".into()),
            &ctx,
        );
        assert_eq!(app.dialog, Some(Dialog::JoinGroup));
        assert!(matches!(
            commands.try_recv(),
            Ok(Command::PreviewInvite(code)) if code == "AbCdEf1234567890XyZ"
        ));
        let info = InviteInfo {
            id: "1@g.us".into(),
            subject: "Club".into(),
            description: None,
            members: 3,
            approval: false,
        };
        app.invite.as_mut().unwrap().state = InviteState::Ready(info);
        app.apply(Action::JoinGroup, &ctx);
        assert!(matches!(commands.try_recv(), Ok(Command::JoinInvite(_))));
        assert!(matches!(
            app.invite.as_ref().unwrap().state,
            InviteState::Joining(_)
        ));
        // A second click while joining sends nothing more.
        app.apply(Action::JoinGroup, &ctx);
        assert!(commands.try_recv().is_err());
    }

    #[test]
    fn a_fourth_pin_is_refused_like_on_the_phone() {
        let mut app = app();
        let (backend, mut commands) = Backend::recording();
        app.backend = backend;
        for index in 0..5 {
            let mut chat = Chat::new(format!("{index}@s.whatsapp.net"), format!("Chat {index}"));
            chat.pinned = index < 3;
            chat.archived = index == 4;
            app.chats.push(chat);
        }
        app.apply(
            Action::SetPinned("3@s.whatsapp.net".into(), true),
            &egui::Context::default(),
        );
        assert!(!app.chat("3@s.whatsapp.net").unwrap().pinned);
        assert!(commands.try_recv().is_err());
        app.apply(
            Action::SetPinned("0@s.whatsapp.net".into(), false),
            &egui::Context::default(),
        );
        app.apply(
            Action::SetPinned("3@s.whatsapp.net".into(), true),
            &egui::Context::default(),
        );
        assert!(app.chat("3@s.whatsapp.net").unwrap().pinned);
    }

    #[test]
    fn whatsapp_plus_raises_the_pin_limit() {
        let mut app = app();
        let (backend, _commands) = Backend::recording();
        app.backend = backend;
        for index in 0..4 {
            let mut chat = Chat::new(format!("{index}@s.whatsapp.net"), format!("Chat {index}"));
            chat.pinned = index < 3;
            app.chats.push(chat);
        }
        app.pin_limit = crate::backend::PLUS_PINNED_CHATS;
        app.apply(
            Action::SetPinned("3@s.whatsapp.net".into(), true),
            &egui::Context::default(),
        );
        assert!(app.chat("3@s.whatsapp.net").unwrap().pinned);
    }

    #[test]
    fn presence_follows_focus_and_the_hidden_window() {
        let mut app = app();
        let (backend, mut commands) = Backend::recording();
        app.backend = backend;
        let mut reported = || {
            std::iter::from_fn(|| commands.try_recv().ok())
                .filter_map(|command| match command {
                    Command::SetOnline(online) => Some(online),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        app.window_focused = true;
        app.report_presence();
        app.report_presence();
        assert_eq!(reported(), [true]);
        app.window_gone();
        app.report_presence();
        assert_eq!(reported(), [false]);
        // Focus left over from a window callback does not count while hidden.
        app.window_focused = true;
        app.report_presence();
        assert!(reported().is_empty());
    }

    #[test]
    fn a_deleted_chat_leaves_only_after_the_phone_confirmed_it() {
        let mut app = app();
        let (backend, mut commands) = Backend::recording();
        app.backend = backend;
        let chat = "peer@s.whatsapp.net";
        let other = "friend@s.whatsapp.net";
        app.chats.push(Chat::new(chat.into(), "Peer".into()));
        app.chats.push(Chat::new(other.into(), "Friend".into()));
        app.conversations
            .entry(chat.into())
            .or_default()
            .merge(vec![message(chat, "m1", 100)], false);
        app.drafts.insert(chat.into(), "half-written".into());
        app.open_chat = Some(chat.into());
        app.settings.last_chat = Some(chat.into());
        app.unread_kept.insert(chat.into());
        app.scroll_chat_into_view = Some(chat.into());
        app.search_hits.push(message(chat, "m1", 100));
        app.search_hits.push(message(other, "m2", 100));
        app.dialog = Some(Dialog::ChatInfo(chat.into()));

        let ctx = egui::Context::default();
        app.apply(Action::DeleteChat(chat.into()), &ctx);

        // Nothing changes here until the phone has deleted the chat too.
        assert!(app.chat(chat).is_some());
        assert!(app.drafts.contains_key(chat));
        assert!(
            std::iter::from_fn(|| commands.try_recv().ok())
                .any(|command| matches!(command, Command::DeleteChat(id) if id == chat))
        );

        let (backend, events) = Backend::detached();
        app.backend = backend;
        events
            .send(Event::ChatRemoved { chat: chat.into() })
            .unwrap();
        app.handle_events();

        assert!(app.chat(chat).is_none());
        assert!(!app.conversations.contains_key(chat));
        // The draft goes with the chat: closing would have kept it, but there
        // is nothing left to send it to.
        assert!(!app.drafts.contains_key(chat));
        assert_eq!(app.open_chat, None);
        // A restart must not try to reopen a chat that is gone.
        assert_eq!(app.settings.last_chat, None);
        // Nothing may keep pointing at a chat that is gone.
        assert!(!app.unread_kept.contains(chat));
        assert_eq!(app.scroll_chat_into_view, None);
        assert!(app.search_hits.iter().all(|hit| hit.chat != chat));
        assert!(app.dialog.is_none());
        // Neighbouring chats and their search hits stay.
        assert!(app.chat(other).is_some());
        assert_eq!(app.search_hits.len(), 1);
    }

    #[test]
    fn a_chat_deleted_on_the_phone_disappears_here_too() {
        let directory = tempfile::tempdir().unwrap();
        let (mut app, events) =
            App::headless(AppDirs::under(directory.path()), Settings::default());
        let chat = "peer@s.whatsapp.net";
        app.chats.push(Chat::new(chat.into(), "Peer".into()));
        app.open_chat = Some(chat.into());

        events
            .send(Event::ChatRemoved { chat: chat.into() })
            .unwrap();
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            app.background_frame(ui.ctx())
        });
        output.textures_delta.clear();

        assert!(app.chat(chat).is_none());
        assert_eq!(app.open_chat, None);
    }

    #[test]
    fn a_chat_cleared_on_the_phone_keeps_the_chat_but_drops_its_messages() {
        let directory = tempfile::tempdir().unwrap();
        let (mut app, events) =
            App::headless(AppDirs::under(directory.path()), Settings::default());
        let chat = "peer@s.whatsapp.net";
        app.chats.push(Chat::new(chat.into(), "Peer".into()));
        app.conversations
            .entry(chat.into())
            .or_default()
            .merge(vec![message(chat, "m1", 100)], false);
        app.open_chat = Some(chat.into());
        app.editing = Some("m1".into());
        app.composer = "edited text".into();
        app.reply_to = Some("m1".into());
        app.reaction_target = Some((chat.into(), "m1".into()));
        app.search_hits.push(message(chat, "m1", 100));

        events
            .send(Event::ChatCleared {
                chat: chat.into(),
                through: 100,
            })
            .unwrap();
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            app.background_frame(ui.ctx())
        });
        output.textures_delta.clear();

        assert!(app.chat(chat).is_some());
        let conversation = &app.conversations[chat];
        assert!(conversation.messages.is_empty());
        // Nothing older remains, locally or on the phone, so neither is asked.
        assert!(conversation.complete && conversation.phone_exhausted);
        // Nothing may point at a message that was just removed.
        assert_eq!(app.editing, None);
        assert!(app.composer.is_empty());
        assert_eq!(app.reply_to, None);
        assert_eq!(app.reaction_target, None);
        assert!(app.search_hits.is_empty());
        // The chat itself stays open.
        assert_eq!(app.open_chat.as_deref(), Some(chat));
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

    #[test]
    fn a_clicked_notification_leaves_a_locked_chat_shut() {
        let mut app = app();
        let ctx = egui::Context::default();
        let chat = "1@s.whatsapp.net";
        let mut locked = Chat::new(chat.into(), "Ada".into());
        locked.locked = true;
        app.chats = vec![locked];
        app.conversations
            .entry(chat.into())
            .or_default()
            .merge(vec![message(chat, "secret", 1)], false);
        app.notification_opens
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(crate::notify::NotificationTarget {
                chat: chat.into(),
                message: "secret".into(),
            });

        app.handle_notification_opens();
        app.apply_actions(&ctx);

        assert_eq!(app.open_chat, None);
        assert_eq!(app.scroll_anchor, None);
    }

    #[test]
    fn a_clicked_notification_opens_the_message_it_announced() {
        let mut app = app();
        let ctx = egui::Context::default();
        let chat = "1@s.whatsapp.net";
        app.chats = vec![Chat::new(chat.into(), "Ada".into())];
        app.conversations.entry(chat.into()).or_default().merge(
            vec![message(chat, "first", 1), message(chat, "second", 2)],
            false,
        );
        app.notification_opens
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(crate::notify::NotificationTarget {
                chat: chat.into(),
                message: "second".into(),
            });

        app.handle_notification_opens();
        app.apply_actions(&ctx);

        assert_eq!(
            app.open_chat.as_deref(),
            Some(chat),
            "the click opens its chat"
        );
        assert_eq!(
            app.scroll_anchor.as_deref(),
            Some("second"),
            "and brings the announced message into view"
        );
    }

    fn voice(chat: &str, id: &str, timestamp: i64, path: Option<&str>) -> Message {
        let mut row = message(chat, id, timestamp);
        row.content = Content::Audio {
            media: Media {
                mime: "audio/ogg".into(),
                size: 1,
                width: None,
                height: None,
                path: path.map(PathBuf::from),
                state: MediaState::Idle,
            },
            seconds: Some(3),
            voice_note: true,
            waveform: Vec::new(),
        };
        row
    }

    #[test]
    fn a_finished_voice_message_carries_on_with_the_next_unplayed_one() {
        let mut app = app();
        let (backend, mut commands) = Backend::recording();
        app.backend = backend;
        let ctx = egui::Context::default();
        let chat = "1@s.whatsapp.net";
        app.chats = vec![Chat::new(chat.into(), "Ada".into())];
        app.open_chat = Some(chat.into());
        app.settings.pause_other_media = true;
        app.conversations.entry(chat.into()).or_default().merge(
            vec![
                voice(chat, "first", 1, Some("first.ogg")),
                voice(chat, "second", 2, None),
            ],
            false,
        );
        app.played_told.insert("first".into());
        app.voice_chat = Some(chat.into());

        app.continue_voice("first");
        assert_eq!(
            app.voice_wanted
                .as_ref()
                .map(|(chat, id, _)| (chat.as_str(), id.as_str())),
            Some((chat, "second")),
            "a clip that is not downloaded yet is fetched first"
        );
        assert!(
            app.wants_quiet(),
            "music stays paused while the next clip downloads"
        );
        app.apply_actions(&ctx);
        assert!(matches!(
            commands.try_recv().unwrap(),
            Command::Download { message, .. } if message == "second"
        ));

        app.handle_media(chat, "second", None, Ok(PathBuf::from("second.ogg")));
        assert!(
            app.actions.iter().any(|action| matches!(
                action,
                Action::PlayVoice { message, .. } if message == "second"
            )),
            "the fetched clip plays when it lands"
        );
        assert!(app.voice_wanted.is_none());
    }

    #[test]
    fn a_voice_run_goes_through_consecutive_unheard_voice_messages() {
        let mut app = app();
        let chat = "1@s.whatsapp.net";
        app.open_chat = Some(chat.into());
        let mut own = voice(chat, "own", 2, None);
        own.from_me = true;
        let mut song = voice(chat, "song", 6, None);
        if let Content::Audio { voice_note, .. } = &mut song.content {
            *voice_note = false;
        }
        app.conversations.entry(chat.into()).or_default().merge(
            vec![
                voice(chat, "played", 1, None),
                own,
                voice(chat, "heard", 3, None),
                voice(chat, "waiting", 4, None),
                message(chat, "text", 5),
                voice(chat, "after text", 6, None),
                song,
                voice(chat, "after song", 7, None),
            ],
            false,
        );
        app.played_told.insert("played".into());
        app.played_told.insert("heard".into());

        assert_eq!(
            app.next_voice_after(chat, "played").as_deref(),
            Some("waiting"),
            "own voice messages and those already heard are passed over"
        );
        assert_eq!(
            app.next_voice_after(chat, "waiting"),
            None,
            "a message in between ends the run"
        );
        assert_eq!(
            app.next_voice_after(chat, "after text"),
            None,
            "an audio file ends the run"
        );
        assert_eq!(
            app.next_voice_after(chat, "song"),
            None,
            "an audio file does not start one"
        );
    }

    #[test]
    fn leaving_the_chat_or_playing_a_video_ends_a_voice_run() {
        let mut app = app();
        let ctx = egui::Context::default();
        let chat = "1@s.whatsapp.net";
        let other = "2@s.whatsapp.net";
        app.chats = vec![
            Chat::new(chat.into(), "Ada".into()),
            Chat::new(other.into(), "Bob".into()),
        ];
        app.conversations.entry(chat.into()).or_default().merge(
            vec![
                voice(chat, "first", 1, Some("first.ogg")),
                voice(chat, "second", 2, Some("second.ogg")),
                voice(chat, "third", 3, None),
            ],
            false,
        );
        app.open_chat = Some(chat.into());
        app.voice_chat = Some(chat.into());
        app.voice_wanted = Some((chat.into(), "third".into(), Instant::now()));

        app.open_chat(other.into());
        app.open_chat(chat.into());
        assert!(app.voice_wanted.is_none(), "a pending clip is dropped");
        app.continue_voice("first");
        assert!(
            app.actions.is_empty(),
            "a clip from before does not carry on"
        );
        app.handle_media(chat, "third", None, Ok(PathBuf::from("third.ogg")));
        assert!(app.actions.is_empty(), "nor does one that lands later");

        app.voice_chat = Some(chat.into());
        app.voice_wanted = Some((chat.into(), "third".into(), Instant::now()));
        app.actions
            .push(Action::PlayVideoWhenDownloaded("video".into()));
        app.apply_actions(&ctx);
        assert!(app.voice_wanted.is_none() && app.voice_chat.is_none());
    }

    #[test]
    fn a_failed_download_ends_the_wait_for_the_next_voice_message() {
        let mut app = app();
        let chat = "1@s.whatsapp.net";
        app.open_chat = Some(chat.into());
        app.settings.pause_other_media = true;
        app.conversations
            .entry(chat.into())
            .or_default()
            .merge(vec![voice(chat, "next", 1, None)], false);
        app.voice_wanted = Some((chat.into(), "next".into(), Instant::now()));
        app.handle_media(chat, "next", None, Err("404".into()));
        assert!(app.voice_wanted.is_none());
        assert!(!app.wants_quiet(), "music resumes");

        app.voice_wanted = Some((
            chat.into(),
            "next".into(),
            Instant::now() - VOICE_FETCH_HOLD,
        ));
        assert!(
            !app.wants_quiet(),
            "a download that takes too long does not keep music paused"
        );
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
    fn merge_keeps_a_downloaded_medias_path_and_state() {
        let mut conversation = Conversation::default();
        let chat = "fixture@s.whatsapp.net";
        let image = |path: Option<PathBuf>, state: MediaState| Message {
            content: Content::Image {
                caption: None,
                media: Media {
                    mime: "image/jpeg".into(),
                    size: 100,
                    width: None,
                    height: None,
                    path,
                    state,
                },
            },
            ..message(chat, "picture", 1)
        };
        conversation.merge(vec![image(None, MediaState::Idle)], false);
        // A download lands, then is marked failed after the fact.
        let downloaded = PathBuf::from("/tmp/picture.jpg");
        if let Some(media) = conversation
            .message_mut("picture")
            .expect("loaded")
            .content
            .media_mut()
        {
            media.path = Some(downloaded.clone());
            media.state = MediaState::Failed("gone".into());
        }
        // A reload delivers the same message freshly classified, without the
        // local path or the runtime state.
        conversation.merge(vec![image(None, MediaState::Idle)], false);
        let media = conversation
            .message("picture")
            .and_then(|message| message.content.media().cloned())
            .expect("still present");
        assert_eq!(media.path, Some(downloaded));
        assert_eq!(media.state, MediaState::Failed("gone".into()));
        // A copy with its own path replaces the in-memory one.
        let relocated = PathBuf::from("/elsewhere/picture.jpg");
        conversation.merge(
            vec![image(Some(relocated.clone()), MediaState::Idle)],
            false,
        );
        let media = conversation
            .message("picture")
            .and_then(|message| message.content.media().cloned())
            .expect("still present");
        assert_eq!(media.path, Some(relocated));
        assert_eq!(media.state, MediaState::Idle);
    }

    #[test]
    fn a_video_note_clicked_before_download_plays_once_it_arrives() {
        let mut app = app();
        app.video.silence();
        let chat = "fixture@s.whatsapp.net";
        let mut clip = message(chat, "clip", 1);
        clip.content = Content::Video {
            caption: None,
            media: Media {
                mime: "video/mp4".into(),
                size: 100,
                width: None,
                height: None,
                path: None,
                state: MediaState::Idle,
            },
            seconds: Some(3),
            gif: false,
            note: true,
        };
        app.conversations
            .entry(chat.into())
            .or_default()
            .merge(vec![clip], false);
        app.open_chat = Some(chat.into());
        let ctx = egui::Context::default();
        app.apply(Action::PlayVideoWhenDownloaded("clip".into()), &ctx);
        let (backend, events) = Backend::detached();
        app.backend = backend;
        let path = PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/video/sample.mp4"
        ));
        events
            .send(Event::Media {
                card: None,
                chat: chat.into(),
                message: "clip".into(),
                result: Ok(path.clone()),
            })
            .unwrap();
        app.handle_events();
        let (backend, mut commands) = Backend::recording();
        app.backend = backend;
        app.apply_actions(&ctx);
        assert_eq!(app.video.message(), Some("clip"));
        // A round video message is played like a voice message.
        assert!(matches!(
            commands.try_recv(),
            Ok(Command::MarkPlayed { message, .. }) if message == "clip"
        ));
        // Leaving the chat stops it.
        app.open_chat = None;
        app.tick_video(&ctx);
        assert!(app.video.message().is_none());
    }

    #[test]
    fn repeated_download_clicks_do_not_queue_more_requests() {
        let mut app = app();
        let (backend, mut commands) = Backend::recording();
        app.backend = backend;
        let chat = "fixture@s.whatsapp.net";
        let mut attachment = message(chat, "picture", 1);
        attachment.content = Content::Image {
            caption: None,
            media: Media {
                mime: "image/jpeg".into(),
                size: 100,
                width: None,
                height: None,
                path: None,
                state: MediaState::Idle,
            },
        };
        app.conversations
            .entry(chat.into())
            .or_default()
            .merge(vec![attachment], false);
        let ctx = egui::Context::default();
        for _ in 0..2 {
            app.apply(
                Action::Download {
                    card: None,
                    chat: chat.into(),
                    message: "picture".into(),
                },
                &ctx,
            );
        }
        assert!(matches!(commands.try_recv(), Ok(Command::Download { .. })));
        assert!(commands.try_recv().is_err());
        let (backend, events) = Backend::detached();
        app.backend = backend;
        events
            .send(Event::Media {
                card: None,
                chat: chat.into(),
                message: "picture".into(),
                result: Err("Download timed out".into()),
            })
            .unwrap();
        app.handle_events();
        assert!(matches!(
            app.media_of(chat, "picture").map(|media| &media.state),
            Some(MediaState::Failed(_))
        ));
        assert!(app.toasts.is_empty());
        let (backend, mut commands) = Backend::recording();
        app.backend = backend;
        app.apply(
            Action::Download {
                card: None,
                chat: chat.into(),
                message: "picture".into(),
            },
            &ctx,
        );
        assert!(matches!(commands.try_recv(), Ok(Command::Download { .. })));
    }

    #[test]
    fn carousel_downloads_track_each_card_separately() {
        let mut app = app();
        let (backend, mut commands) = Backend::recording();
        app.backend = backend;
        let chat = "fixture@s.whatsapp.net";
        let card = || crate::model::InteractiveCard {
            image: Some(Media {
                mime: "image/jpeg".into(),
                size: 100,
                width: None,
                height: None,
                path: None,
                state: MediaState::Idle,
            }),
            ..Default::default()
        };
        let mut carousel = message(chat, "carousel", 1);
        carousel.content = Content::Interactive {
            text: String::new(),
            card: Some(Box::new(crate::model::InteractiveCard {
                carousel: vec![card(), card()],
                ..Default::default()
            })),
        };
        app.conversations
            .entry(chat.into())
            .or_default()
            .merge(vec![carousel.clone()], false);
        let images = |app: &App| -> Vec<Media> {
            match &app.conversations[chat].message("carousel").unwrap().content {
                Content::Interactive {
                    card: Some(card), ..
                } => card
                    .carousel
                    .iter()
                    .map(|card| card.image.clone().unwrap())
                    .collect(),
                _ => Vec::new(),
            }
        };
        let ctx = egui::Context::default();
        app.apply(
            Action::Download {
                card: Some(1),
                chat: chat.into(),
                message: "carousel".into(),
            },
            &ctx,
        );
        assert!(matches!(
            commands.try_recv(),
            Ok(Command::Download { card: Some(1), .. })
        ));
        let states = |app: &App| {
            images(app)
                .into_iter()
                .map(|media| media.state)
                .collect::<Vec<_>>()
        };
        assert_eq!(states(&app), [MediaState::Idle, MediaState::Downloading]);
        // A worker update carries no download state; the card keeps its own.
        let (backend, events) = Backend::detached();
        app.backend = backend;
        events
            .send(Event::MessageUpdated(Box::new(carousel)))
            .unwrap();
        app.handle_events();
        assert_eq!(states(&app), [MediaState::Idle, MediaState::Downloading]);
        let path = PathBuf::from("/cache/zapfast/media/carousel-card-1.jpg");
        events
            .send(Event::Media {
                card: Some(1),
                chat: chat.into(),
                message: "carousel".into(),
                result: Ok(path.clone()),
            })
            .unwrap();
        app.handle_events();
        let images = images(&app);
        assert_eq!(images[0].path, None);
        assert_eq!(images[1].path, Some(path));
        assert_eq!(images[1].state, MediaState::Idle);
    }

    fn refused(chat: &str, quoting: Option<&str>, unsent: Unsent, reason: Refusal) -> Event {
        Event::SendRefused {
            chat: chat.into(),
            quoting: quoting.map(str::to_owned),
            unsent,
            reason,
        }
    }

    fn error_toasts(app: &App) -> Vec<String> {
        app.toasts
            .iter()
            .filter(|toast| toast.kind == ToastKind::Error)
            .map(|toast| toast.message.clone())
            .collect()
    }

    #[test]
    fn a_refused_text_reply_returns_to_the_composer_with_its_reply() {
        let root = tempfile::tempdir().unwrap();
        let (mut app, _) = App::headless(AppDirs::under(root.path()), Settings::default());
        let (backend, mut commands, events) = Backend::recording_with_events();
        app.backend = backend;
        let chat = "fixture@s.whatsapp.net";
        app.open_chat = Some(chat.into());
        app.reply_to = Some("original".into());
        app.apply(
            Action::SendText {
                chat: chat.into(),
                text: "Reply fixture".into(),
                quoting: app.reply_to.clone(),
            },
            &egui::Context::default(),
        );
        assert!(app.reply_to.is_none());
        let sent: Vec<_> = std::iter::from_fn(|| commands.try_recv().ok()).collect();
        assert!(sent.iter().any(|command| matches!(command,
            Command::SendText { quoting: Some(id), .. } if id == "original")));
        events
            .send(refused(
                chat,
                Some("original"),
                Unsent::Text("Reply fixture".into()),
                Refusal::QuoteUnavailable,
            ))
            .unwrap();
        app.handle_events();
        assert_eq!(app.composer, "Reply fixture");
        assert_eq!(
            app.reply_to.as_deref(),
            Some("original"),
            "the reply is re-armed"
        );
        assert!(app.focus_composer);
        assert!(
            std::iter::from_fn(|| commands.try_recv().ok()).any(|command| matches!(command,
                Command::SaveDraft { chat: saved, text } if saved == chat && text == "Reply fixture")),
            "the returned text is a draft again"
        );
        assert!(error_toasts(&app)[0].contains("replying to"));
        // Text typed since is not overwritten by a later refusal.
        app.composer = "Newer draft".into();
        events
            .send(refused(
                chat,
                None,
                Unsent::Text("Older text".into()),
                Refusal::Offline,
            ))
            .unwrap();
        app.handle_events();
        assert_eq!(app.composer, "Newer draft");
        assert!(error_toasts(&app)[1].contains("not connected"));
    }

    #[test]
    fn a_refused_text_for_another_chat_becomes_its_draft() {
        let root = tempfile::tempdir().unwrap();
        let (mut app, _) = App::headless(AppDirs::under(root.path()), Settings::default());
        let (backend, mut commands, events) = Backend::recording_with_events();
        app.backend = backend;
        app.open_chat = Some("other@s.whatsapp.net".into());
        app.composer = "Other chat's text".into();
        let chat = "fixture@s.whatsapp.net";
        events
            .send(refused(
                chat,
                Some("original"),
                Unsent::Text("Reply fixture".into()),
                Refusal::QuoteUnavailable,
            ))
            .unwrap();
        app.handle_events();
        assert_eq!(app.composer, "Other chat's text");
        assert!(
            app.reply_to.is_none(),
            "no reply is armed in the wrong chat"
        );
        assert_eq!(
            app.drafts.get(chat).map(String::as_str),
            Some("Reply fixture")
        );
        assert!(
            std::iter::from_fn(|| commands.try_recv().ok()).any(|command| matches!(command,
                Command::SaveDraft { chat: saved, text } if saved == chat && text == "Reply fixture"))
        );
        // A draft the chat already has is kept.
        events
            .send(refused(
                chat,
                None,
                Unsent::Text("Later text".into()),
                Refusal::Offline,
            ))
            .unwrap();
        app.handle_events();
        assert_eq!(
            app.drafts.get(chat).map(String::as_str),
            Some("Reply fixture")
        );
    }

    #[test]
    fn attachments_and_gifs_carry_the_reply_and_come_back_when_refused() {
        let root = tempfile::tempdir().unwrap();
        let (mut app, _) = App::headless(AppDirs::under(root.path()), Settings::default());
        let (backend, mut commands, events) = Backend::recording_with_events();
        app.backend = backend;
        let ctx = egui::Context::default();
        let chat = "fixture@s.whatsapp.net";
        app.open_chat = Some(chat.into());
        app.reply_to = Some("original".into());
        app.pending.push(Pending::Picture {
            width: 1,
            height: 1,
            rgba: std::sync::Arc::new(vec![1, 2, 3, 4]),
            texture: None,
        });
        app.pending.push(Pending::File("/fixture/a.pdf".into()));
        app.apply(
            Action::SendPending {
                chat: chat.into(),
                caption: "Caption fixture".into(),
            },
            &ctx,
        );
        let sent: Vec<_> = std::iter::from_fn(|| commands.try_recv().ok()).collect();
        // Only the first attachment, which carries the caption, is the reply.
        assert!(matches!(
            sent.as_slice(),
            [
                Command::SendImage { quoting: Some(id), caption: Some(_), .. },
                Command::SendFiles { quoting: None, caption: None, .. },
            ] if id == "original"
        ));
        assert!(app.reply_to.is_none());
        assert!(app.pending.is_empty());
        app.reply_to = Some("original".into());
        app.apply(
            Action::SendGif(Gif {
                id: "fixture".into(),
                still: None,
                mp4: "https://example.invalid/fixture.mp4".into(),
                width: 2,
                height: 2,
            }),
            &ctx,
        );
        assert!(app.reply_to.is_none());
        assert!(
            std::iter::from_fn(|| commands.try_recv().ok()).any(|command| matches!(command,
                Command::SendGif { quoting: Some(id), .. } if id == "original"))
        );
        // Refused attachments return to the composer with their caption.
        for unsent in [
            Unsent::Image {
                width: 1,
                height: 1,
                rgba: vec![1, 2, 3, 4],
                caption: Some("Caption fixture".into()),
            },
            Unsent::Files {
                paths: vec!["/fixture/a.pdf".into()],
                caption: None,
            },
            Unsent::Gif,
        ] {
            events
                .send(refused(
                    chat,
                    Some("original"),
                    unsent,
                    Refusal::QuoteUnavailable,
                ))
                .unwrap();
        }
        app.handle_events();
        assert!(matches!(
            app.pending.as_slice(),
            [Pending::Picture { width: 1, height: 1, .. }, Pending::File(path)]
                if path == std::path::Path::new("/fixture/a.pdf")
        ));
        assert_eq!(app.composer, "Caption fixture");
        assert_eq!(app.reply_to.as_deref(), Some("original"));
        // Repeats of one error share a toast.
        assert_eq!(error_toasts(&app).len(), 1);
    }

    #[test]
    fn a_refused_voice_message_is_sent_again_only_from_its_chat_or_discarded() {
        let root = tempfile::tempdir().unwrap();
        let (mut app, _) = App::headless(AppDirs::under(root.path()), Settings::default());
        let (backend, mut commands, events) = Backend::recording_with_events();
        app.backend = backend;
        let ctx = egui::Context::default();
        let chat = "fixture@s.whatsapp.net";
        let other = "other@s.whatsapp.net";
        let clip = vec![0.25; crate::voice::RATE as usize * 2];
        app.open_chat = Some(chat.into());
        events
            .send(refused(
                chat,
                Some("original"),
                Unsent::Voice(clip.clone()),
                Refusal::QuoteUnavailable,
            ))
            .unwrap();
        app.handle_events();
        assert_eq!(app.unsent_voice, Some((chat.to_owned(), clip.clone())));
        assert_eq!(app.reply_to.as_deref(), Some("original"));
        assert!(app.composer.is_empty(), "the composer stays empty");
        // Another chat's Send neither sends nor drops the clip.
        app.open_chat = Some(other.into());
        app.reply_to = None;
        app.apply(Action::SendRecording, &ctx);
        assert!(commands.try_recv().is_err());
        assert!(app.unsent_voice.is_some());
        // Back in its chat, with the reply cancelled, it goes out unquoted
        // because the user chose so.
        app.open_chat = Some(chat.into());
        app.apply(Action::SendRecording, &ctx);
        assert!(matches!(
            commands.try_recv(),
            Ok(Command::SendVoice { chat: sent, samples, quoting: None })
                if sent == chat && samples == clip
        ));
        assert!(app.unsent_voice.is_none());
        // Refused again: sending with the reply armed quotes it, and an active
        // recording is never preempted by the retained clip.
        events
            .send(refused(
                chat,
                Some("original"),
                Unsent::Voice(clip.clone()),
                Refusal::Offline,
            ))
            .unwrap();
        app.handle_events();
        app.recording = Some(crate::audio::Recorder::rehearsal());
        app.apply(Action::SendRecording, &ctx);
        assert!(app.recording.is_none());
        assert!(app.unsent_voice.is_some(), "the retained clip still waits");
        let _ = std::iter::from_fn(|| commands.try_recv().ok()).count();
        app.reply_to = Some("original".into());
        app.apply(Action::SendRecording, &ctx);
        assert!(matches!(
            commands.try_recv(),
            Ok(Command::SendVoice { quoting: Some(id), .. }) if id == "original"
        ));
        // Discarding drops it for good.
        events
            .send(refused(chat, None, Unsent::Voice(clip), Refusal::Offline))
            .unwrap();
        app.handle_events();
        app.apply(Action::DiscardUnsentVoice, &ctx);
        assert!(app.unsent_voice.is_none());
        app.apply(Action::SendRecording, &ctx);
        assert!(commands.try_recv().is_err());
    }

    #[test]
    fn replying_while_editing_starts_a_new_message() {
        let mut app = app();
        app.open_chat = Some("fixture@s.whatsapp.net".into());
        app.editing = Some("edited".into());
        app.composer = "Text being edited".into();
        app.apply(Action::Reply("original".into()), &egui::Context::default());
        assert!(app.editing.is_none());
        assert!(
            app.composer.is_empty(),
            "the edit's text is not sent as a reply"
        );
        assert_eq!(app.reply_to.as_deref(), Some("original"));
        assert!(app.focus_composer);
    }

    #[test]
    fn sending_a_sticker_consumes_the_pending_reply() {
        let mut app = app();
        let (backend, mut commands) = Backend::recording();
        app.backend = backend;
        app.open_chat = Some("fixture@s.whatsapp.net".into());
        app.reply_to = Some("quoted-message".into());

        app.apply(
            Action::SendSticker(std::path::PathBuf::from("sticker.webp")),
            &egui::Context::default(),
        );

        assert!(app.reply_to.is_none());
        assert!(matches!(
            commands.try_recv(),
            Ok(Command::SendSticker {
                chat,
                quoting: Some(id),
                ..
            }) if chat == "fixture@s.whatsapp.net" && id == "quoted-message"
        ));
    }

    #[test]
    fn clicking_an_oversized_attachment_does_not_start_a_download() {
        let mut app = app();
        let (backend, mut commands) = Backend::recording();
        app.backend = backend;
        let chat = "peer@s.whatsapp.net";
        let mut attachment = message(chat, "picture", 1);
        attachment.content = Content::Image {
            caption: None,
            media: Media {
                mime: "image/jpeg".into(),
                size: crate::model::ATTACHMENT_DOWNLOAD_LIMIT + 1,
                width: None,
                height: None,
                path: None,
                state: MediaState::Idle,
            },
        };
        app.conversations
            .entry(chat.into())
            .or_default()
            .merge(vec![attachment], false);

        app.apply(
            Action::Download {
                card: None,
                chat: chat.into(),
                message: "picture".into(),
            },
            &egui::Context::default(),
        );

        assert!(commands.try_recv().is_err());
        assert!(matches!(
            app.media_of(chat, "picture").map(|media| &media.state),
            Some(MediaState::Failed(_))
        ));
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
    fn a_jump_flash_rises_holds_and_fades_out() {
        assert_eq!(JumpHighlight::strength(-1.0), 0.0);
        assert_eq!(JumpHighlight::strength(0.0), 0.0);
        assert_eq!(JumpHighlight::strength(0.5), 1.0);
        let fading = JumpHighlight::strength(1.5);
        assert!(fading > 0.0 && fading < 1.0, "{fading}");
        assert_eq!(JumpHighlight::strength(JumpHighlight::DURATION), 0.0);
        assert_eq!(JumpHighlight::strength(10.0), 0.0);
    }

    #[test]
    fn quotes_and_search_hits_flash_their_message_until_the_chat_changes() {
        let mut app = app();
        let ctx = egui::Context::default();
        let (ada, bob) = ("1@s.whatsapp.net", "2@s.whatsapp.net");
        for chat in [ada, bob] {
            app.chats.push(Chat::new(chat.into(), "Chat".into()));
            app.conversations.insert(
                chat.into(),
                Conversation {
                    requested: true,
                    complete: true,
                    messages: vec![message(chat, "old", 10)],
                    ..Default::default()
                },
            );
        }
        app.apply(
            Action::OpenMessage {
                chat: ada.into(),
                message: "old".into(),
            },
            &ctx,
        );
        assert_eq!(
            app.jump_highlight,
            Some(JumpHighlight::new(ada.into(), "old".into()))
        );
        app.jump_highlight = None;
        app.apply(Action::ScrollTo("old".into()), &ctx);
        assert_eq!(
            app.jump_highlight,
            Some(JumpHighlight::new(ada.into(), "old".into()))
        );
        app.open_chat(bob.into());
        assert_eq!(app.jump_highlight, None);
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
    fn ctrl_f_searches_the_open_chat_in_the_pane_and_ctrl_k_the_list() {
        let mut app = app();
        let ctx = egui::Context::default();
        // Without an open chat there is nothing to search inside.
        app.apply(Action::OpenChatSearch, &ctx);
        assert!(!app.chat_search_open);
        app.open_chat = Some("1@s.whatsapp.net".into());
        app.apply(Action::OpenChatSearch, &ctx);
        assert!(app.chat_search_visible());
        assert!(app.focus_chat_search);
        assert!(!app.focus_composer, "the pane's field takes the keyboard");
        // Ctrl+K searches the chat list, and closes the pane.
        app.apply(Action::FocusSearch, &ctx);
        app.apply_actions(&ctx);
        assert!(app.focus_search);
        assert!(!app.chat_search_open);
    }

    #[test]
    fn the_pane_keeps_the_chat_list_search_and_gives_the_composer_back() {
        let mut app = app();
        let ctx = egui::Context::default();
        let chat = "1@s.whatsapp.net";
        app.chats.push(Chat::new(chat.into(), "Ada".into()));
        app.open_chat = Some(chat.into());
        app.search = "list".into();
        app.apply(Action::OpenChatSearch, &ctx);
        assert!(app.chat_search_open);
        assert_eq!(app.search, "list", "the two searches are independent");
        app.search.clear();
        app.apply(Action::ChatSearch("engine".into()), &ctx);
        app.apply(Action::CloseChatSearch, &ctx);
        assert!(!app.chat_search_open);
        assert!(app.chat_search.is_empty());
        assert!(app.focus_composer, "closing hands the keyboard back");
    }

    #[test]
    fn the_pane_lists_only_the_answer_to_the_query_and_day_in_force() {
        let mut app = app();
        let (backend, mut commands, events) = Backend::recording_with_events();
        app.backend = backend;
        let ctx = egui::Context::default();
        let chat = "1@s.whatsapp.net";
        app.open_chat = Some(chat.into());
        app.apply(Action::OpenChatSearch, &ctx);
        let searches = |commands: &mut tokio::sync::mpsc::UnboundedReceiver<Command>| {
            std::iter::from_fn(|| commands.try_recv().ok())
                .filter_map(|command| match command {
                    Command::SearchChatMessages {
                        query, from, until, ..
                    } => Some((query, from.is_some() && until.is_some())),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        assert!(searches(&mut commands).is_empty(), "nothing to ask yet");
        app.apply(Action::ChatSearch("eng".into()), &ctx);
        app.apply(Action::ChatSearch(" engine ".into()), &ctx);
        assert_eq!(
            searches(&mut commands),
            [("eng".to_owned(), false), ("engine".to_owned(), false)]
        );
        assert!(app.chat_search_pending);
        let hits =
            |query: &str, from: Option<i64>, until: Option<i64>, ids: &[&str]| Event::ChatHits {
                chat: chat.into(),
                query: query.into(),
                from,
                until,
                messages: ids.iter().map(|id| message(chat, id, 1)).collect(),
                truncated: false,
            };
        // The answer to the query already replaced arrives too late.
        events.send(hits("eng", None, None, &["old"])).unwrap();
        app.handle_events();
        assert!(app.chat_search_hits.is_empty());
        assert!(app.chat_search_pending);
        events
            .send(hits("engine", None, None, &["m1", "m2"]))
            .unwrap();
        app.handle_events();
        assert_eq!(app.chat_search_hits.len(), 2);
        assert!(!app.chat_search_pending);
        // A day narrows the same query, and an answer without it is stale.
        let day = jiff::civil::Date::new(2026, 9, 23).expect("a date");
        app.apply(Action::SetChatSearchDay(Some(day)), &ctx);
        assert_eq!(searches(&mut commands), [("engine".to_owned(), true)]);
        assert_eq!(app.chat_search_day, Some(day));
        assert_eq!(app.chat_search_month, day, "the calendar follows the pick");
        assert!(!app.chat_search_calendar, "picking a day closes it");
        events.send(hits("engine", None, None, &["m3"])).unwrap();
        app.handle_events();
        assert_eq!(app.chat_search_hits.len(), 2, "still the earlier list");
        let (from, until) = app.chat_search_range();
        events.send(hits("engine", from, until, &["m1"])).unwrap();
        app.handle_events();
        assert_eq!(app.chat_search_hits.len(), 1);
        // A day on its own lists that day; with neither, nothing is asked.
        app.apply(Action::ChatSearch(String::new()), &ctx);
        assert_eq!(searches(&mut commands), [(String::new(), true)]);
        app.apply(Action::SetChatSearchDay(None), &ctx);
        assert!(searches(&mut commands).is_empty());
        assert!(app.chat_search_hits.is_empty());
        assert!(!app.chat_search_pending);
        assert_eq!(app.chat_search_range(), (None, None));
    }

    #[test]
    fn a_result_opened_from_the_pane_keeps_the_keyboard_there() {
        let mut app = app();
        let ctx = egui::Context::default();
        let chat = "1@s.whatsapp.net";
        app.chats.push(Chat::new(chat.into(), "Ada".into()));
        let conversation = Conversation {
            requested: true,
            complete: true,
            messages: vec![message(chat, "hit", 10)],
            ..Default::default()
        };
        app.conversations.insert(chat.into(), conversation);
        app.apply(Action::OpenChat(chat.into()), &ctx);
        app.apply(Action::OpenChatSearch, &ctx);
        app.apply(Action::ChatSearch("engine".into()), &ctx);
        app.apply(
            Action::OpenMessage {
                chat: chat.into(),
                message: "hit".into(),
            },
            &ctx,
        );
        assert!(app.chat_search_open, "the pane stays open");
        assert_eq!(app.chat_search, "engine", "with its query");
        assert!(!app.focus_composer);
        assert_eq!(
            app.jump_highlight
                .as_ref()
                .map(|jump| jump.message.as_str()),
            Some("hit"),
            "and the result flashes"
        );
        // Another chat's search does not follow the reader there.
        let other = "2@s.whatsapp.net";
        app.chats.push(Chat::new(other.into(), "Bo".into()));
        app.apply(Action::OpenChat(other.into()), &ctx);
        assert!(!app.chat_search_open);
        assert!(app.chat_search.is_empty());
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
    fn muting_all_channels_leaves_other_chats_alone() {
        let mut app = app();
        let (backend, mut commands) = Backend::recording();
        app.backend = backend;
        let ctx = egui::Context::default();
        app.chats = vec![
            Chat::new("1@newsletter".into(), "News".into()),
            Chat::new("2@newsletter".into(), "More news".into()),
            Chat::new("3@s.whatsapp.net".into(), "Ada".into()),
        ];
        app.apply(Action::MuteAllChannels(true), &ctx);
        app.apply_actions(&ctx);
        let muted: Vec<String> = std::iter::from_fn(|| commands.try_recv().ok())
            .filter_map(|command| match command {
                Command::SetMuted(chat, Some(0)) => Some(chat),
                _ => None,
            })
            .collect();
        assert_eq!(muted, ["1@newsletter", "2@newsletter"]);
    }

    #[test]
    fn the_unread_chip_finds_a_chat_marked_unread_by_hand() {
        let mut app = app();
        let ctx = egui::Context::default();
        let mut marked = Chat::new("1@s.whatsapp.net".into(), "Ada".into());
        marked.marked_unread = true;
        let read = Chat::new("2@s.whatsapp.net".into(), "Grace".into());
        app.chats = vec![marked, read];
        assert_eq!(app.unread_chats(ChatFilter::Unread), 1);
        app.apply(Action::SetChatFilter(ChatFilter::Unread), &ctx);
        let shown: Vec<&str> = app
            .visible_chats()
            .iter()
            .map(|chat| chat.name.as_str())
            .collect();
        assert_eq!(shown, ["Ada"], "the chip finds the chat marked by hand");
        // Nothing is pending, so the app badge stays at zero.
        assert_eq!(app.unread_total(), 0);
    }

    #[test]
    fn channels_have_their_own_chip_and_archived_chats_theirs() {
        let mut app = app();
        let ctx = egui::Context::default();
        let mut friend = Chat::new("1@s.whatsapp.net".into(), "Ada".into());
        friend.unread = 1;
        let mut channel = Chat::new("2@newsletter".into(), "News".into());
        channel.unread = 3;
        let mut archived = Chat::new("3@s.whatsapp.net".into(), "Old".into());
        archived.archived = true;
        archived.unread = 2;
        app.chats = vec![friend, channel, archived];
        let names = |app: &App| {
            app.visible_chats()
                .iter()
                .map(|chat| chat.name.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(names(&app), ["Ada"], "All leaves channels out");
        assert_eq!(app.unread_chats(ChatFilter::Unread), 1);
        assert_eq!(app.unread_chats(ChatFilter::Channels), 1);
        app.apply(Action::SetChatFilter(ChatFilter::Channels), &ctx);
        assert_eq!(names(&app), ["News"]);
        app.apply(Action::ShowArchived(true), &ctx);
        assert_eq!(names(&app), ["Old"]);
        assert_eq!(app.archived_unread(), 1);
        app.apply(Action::SetChatFilter(ChatFilter::All), &ctx);
        assert!(!app.show_archived, "choosing a filter leaves the archive");
        assert_eq!(names(&app), ["Ada"]);
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
    fn the_favorites_chip_keeps_the_phone_order_below_pins() {
        let mut app = app();
        let favorite = |id: &str, name: &str, position: u32, activity: i64| {
            let mut chat = Chat::new(id.into(), name.into());
            chat.favorite = true;
            chat.favorite_position = position;
            chat.last_activity = activity;
            chat
        };
        let mut pinned = favorite("3@s.whatsapp.net", "Cy", 2, 1);
        pinned.pinned = true;
        app.chats = vec![
            favorite("1@s.whatsapp.net", "Ada", 1, 30),
            favorite("2@s.whatsapp.net", "Bob", 0, 10),
            pinned,
            Chat::new("4@s.whatsapp.net".into(), "Dee".into()),
        ];
        app.chat_filter = ChatFilter::Favorites;
        let names: Vec<&str> = app
            .visible_chats()
            .iter()
            .map(|chat| chat.name.as_str())
            .collect();
        assert_eq!(names, ["Cy", "Bob", "Ada"]);
        app.chat_filter = ChatFilter::All;
        let names: Vec<&str> = app
            .visible_chats()
            .iter()
            .map(|chat| chat.name.as_str())
            .collect();
        assert_eq!(
            names,
            ["Cy", "Ada", "Bob", "Dee"],
            "other chips keep recency"
        );
    }

    #[test]
    fn the_chat_filter_narrows_the_main_list_only() {
        let mut app = app();
        let mut ada = Chat::new("1@s.whatsapp.net".into(), "Ada".into());
        ada.last_activity = 40;
        ada.unread = 2;
        let mut bob = Chat::new("2@s.whatsapp.net".into(), "Bob".into());
        bob.last_activity = 30;
        let mut club = Chat::new("3@g.us".into(), "Club".into());
        club.last_activity = 20;
        club.unread = 1;
        let mut news = Chat::new("4@newsletter".into(), "News".into());
        news.last_activity = 10;
        let mut old = Chat::new("5@g.us".into(), "Old group".into());
        old.archived = true;
        old.unread = 3;
        app.chats = vec![ada, bob, club, news, old];
        let names = |app: &App| -> Vec<String> {
            app.visible_chats()
                .iter()
                .map(|chat| chat.name.clone())
                .collect()
        };
        assert_eq!(
            names(&app),
            ["Ada", "Bob", "Club"],
            "channels have their own chip"
        );
        app.chat_filter = ChatFilter::Channels;
        assert_eq!(names(&app), ["News"]);
        app.chat_filter = ChatFilter::Unread;
        assert_eq!(names(&app), ["Ada", "Club"]);
        app.chat_filter = ChatFilter::Private;
        assert_eq!(names(&app), ["Ada", "Bob"], "no groups or broadcasts");
        app.chat_filter = ChatFilter::Groups;
        assert_eq!(names(&app), ["Club"], "archived groups stay in the archive");
        // Unread chats per chip, archived ones left out.
        assert_eq!(app.unread_chats(ChatFilter::Unread), 2);
        assert_eq!(app.unread_chats(ChatFilter::Private), 1);
        assert_eq!(app.unread_chats(ChatFilter::Groups), 1);
        // Search and the archive ignore the filter.
        app.search = "bob".into();
        assert_eq!(names(&app), ["Bob"]);
        app.search = String::new();
        app.show_archived = true;
        assert_eq!(names(&app), ["Old group"]);
    }

    #[test]
    fn the_unread_filter_keeps_the_open_chat_after_it_is_read() {
        let mut app = app();
        let mut ada = Chat::new("1@s.whatsapp.net".into(), "Ada".into());
        ada.unread = 1;
        let bob = Chat::new("2@s.whatsapp.net".into(), "Bob".into());
        app.chats = vec![ada, bob];
        let mut cy = Chat::new("3@s.whatsapp.net".into(), "Cy".into());
        cy.unread = 1;
        app.chats.push(cy);
        app.chat_filter = ChatFilter::Unread;
        let ctx = egui::Context::default();
        // Every chat opened from the list stays, not only the latest.
        for index in [0, 2] {
            let id = app.chats[index].id.clone();
            app.apply(Action::KeepUnread(id.clone()), &ctx);
            app.open_chat(id);
            app.chats[index].unread = 0;
        }
        assert_eq!(app.visible_chats().len(), 2, "both still listed once read");
        // Choosing a filter again forgets the kept chats.
        app.apply(Action::SetChatFilter(ChatFilter::Unread), &ctx);
        assert!(app.visible_chats().is_empty());
        // A chat opened from search or a notification is not kept.
        app.chats[0].unread = 1;
        app.open_chat("1@s.whatsapp.net".into());
        app.chats[0].unread = 0;
        assert!(app.visible_chats().is_empty());
        // Nothing is kept under another filter.
        app.apply(Action::SetChatFilter(ChatFilter::Private), &ctx);
        app.apply(Action::KeepUnread("2@s.whatsapp.net".into()), &ctx);
        app.apply(Action::SetChatFilter(ChatFilter::Unread), &ctx);
        assert!(app.visible_chats().is_empty());
    }

    #[test]
    fn leaving_the_locked_folder_closes_its_open_conversation() {
        let mut app = app();
        let ctx = egui::Context::default();
        let mut chat = Chat::new("fixture".into(), "Fixture".into());
        chat.locked = true;
        app.chats.push(chat);
        app.settings.set_chat_lock_code(Some("fixture-code"));
        app.search = "fixture-code".into();
        app.locked_folder = true;
        app.open_chat("fixture".into());
        assert!(app.current_chat().is_some());
        app.composer = "fixture draft".into();
        app.reply_to = Some("fixture-message".into());
        app.apply(Action::Search(String::new()), &ctx);
        assert!(app.current_chat().is_none());
        assert!(app.open_chat.is_none());
        assert!(app.composer.is_empty());
        assert!(app.reply_to.is_none());
        assert_eq!(app.drafts["fixture"], "fixture draft");
    }

    #[test]
    fn locked_tab_authenticates_without_using_search_and_relocks_on_filter_change() {
        let mut app = app();
        let ctx = egui::Context::default();
        let mut chat = Chat::new("fixture@g.us".into(), "Secret fixture".into());
        chat.locked = true;
        chat.archived = true;
        app.chats.push(chat);
        app.settings.set_chat_lock_code(Some("test-code"));
        app.apply(Action::OpenLockedFolder, &ctx);
        assert_eq!(app.dialog, Some(Dialog::UnlockLockedChats));
        assert!(app.visible_chats().is_empty());
        app.apply(Action::UnlockLockedFolder("wrong".into()), &ctx);
        assert!(app.chat_lock_error);
        assert!(!app.locked_folder_open());
        app.apply(Action::UnlockLockedFolder("test-code".into()), &ctx);
        assert!(app.locked_folder_open());
        assert!(app.search.is_empty());
        assert_eq!(
            app.visible_chats().len(),
            1,
            "includes archived locked chats"
        );
        app.apply(Action::Search("secret".into()), &ctx);
        assert_eq!(app.visible_chats().len(), 1);
        app.apply(Action::Search("unmatched".into()), &ctx);
        assert!(app.visible_chats().is_empty());
        assert!(app.locked_folder_open());
        app.open_chat("fixture@g.us".into());
        app.apply(Action::SetChatFilter(ChatFilter::All), &ctx);
        assert!(!app.locked_folder_open());
        assert!(app.current_chat().is_none());
        assert!(app.chat_lock_session.is_none());
        app.apply(Action::OpenLockedFolder, &ctx);
        assert_eq!(app.dialog, Some(Dialog::UnlockLockedChats));
    }

    #[test]
    fn code_setup_cannot_replace_an_existing_verifier_and_clears_prompt_state() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.apply(Action::CreateChatLockCode("fixture-code".into()), &ctx);
        assert!(app.locked_folder_open());
        app.apply(Action::CreateChatLockCode("replacement".into()), &ctx);
        assert!(app.settings.verifies_chat_lock_code("fixture-code"));
        app.apply(Action::CloseLockedFolder, &ctx);
        app.apply(Action::OpenLockedFolder, &ctx);
        app.chat_lock_entry = "partial".into();
        app.chat_lock_confirm = "partial".into();
        app.window_gone();
        assert!(app.chat_lock_entry.is_empty());
        assert!(app.chat_lock_confirm.is_empty());
        assert!(app.dialog.is_none());
    }

    #[test]
    fn our_own_chat_is_titled_and_found_by_our_name() {
        let mut app = app();
        let me = "15550000000@s.whatsapp.net";
        app.me = Some(me.into());
        let mut own = Chat::new(me.into(), "You".into());
        own.last_activity = 10;
        let mut ada = Chat::new("1@s.whatsapp.net".into(), "Ada".into());
        ada.last_activity = 20;
        app.chats = vec![own, ada];
        assert_eq!(app.chat_title(&app.chats[0].clone()), "You");
        app.me_name = Some("Carmine Paolino".into());
        assert_eq!(
            app.chat_title(&app.chats[0].clone()),
            "Carmine Paolino (You)"
        );
        for search in ["carmine", "you", "5550000"] {
            app.search = search.into();
            let ids: Vec<&str> = app
                .visible_chats()
                .iter()
                .map(|chat| chat.id.as_str())
                .collect();
            assert_eq!(ids, vec![me], "{search}");
        }
        app.locale = crate::i18n::Locale::German;
        assert_eq!(
            app.chat_title(&app.chats[0].clone()),
            "Carmine Paolino (Du)"
        );
    }

    #[test]
    fn contact_searches_offer_to_message_ourselves() {
        let mut app = app();
        assert!(!app.offers_self(""), "not before we know who we are");
        app.me = Some("15550000000@s.whatsapp.net".into());
        app.me_name = Some("Carmine".into());
        for needle in ["", "carm", "you", "message your", "555000"] {
            assert!(app.offers_self(needle), "{needle}");
        }
        assert!(!app.offers_self("ada"));
        app.locale = crate::i18n::Locale::Italian;
        assert!(app.offers_self("te stesso"), "the interface language");
        assert!(app.offers_self("yourself"), "English still works");
        let mut own = Chat::new(app.me.clone().unwrap(), "You".into());
        own.locked = true;
        app.chats = vec![own];
        assert!(
            !app.offers_self(""),
            "a locked chat stays in the locked list"
        );
    }

    #[test]
    fn message_yourself_creates_one_chat_and_respects_its_lock() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.me = Some("15550000000@s.whatsapp.net".into());
        app.dialog = Some(Dialog::NewChat);
        app.apply(Action::MessageYourself, &ctx);
        assert_eq!(app.open_chat, app.me);
        assert!(app.dialog.is_none());
        app.apply(Action::MessageYourself, &ctx);
        assert_eq!(app.chats.len(), 1);
        app.apply(Action::SetLocked(app.me.clone().unwrap(), true), &ctx);
        app.apply(Action::MessageYourself, &ctx);
        assert!(app.open_chat.is_none());
        assert_eq!(app.dialog, Some(Dialog::UnlockLockedChats));
    }

    #[test]
    fn quick_reaction_preferences_count_use_without_counting_removal_or_insertions() {
        let mut app = app();
        let ctx = egui::Context::default();
        for emoji in ["🦀", "🎉", "🦀", ""] {
            app.apply(
                Action::React {
                    chat: "fixture".into(),
                    message: "message".into(),
                    emoji: emoji.into(),
                },
                &ctx,
            );
        }
        app.remember_emoji("🔥");
        assert_eq!(
            app.settings.reaction_emoji,
            vec![("🦀".into(), 2), ("🎉".into(), 1)]
        );
        let saved = serde_json::to_string(&app.settings).unwrap();
        let loaded: Settings = serde_json::from_str(&saved).unwrap();
        assert_eq!(loaded.reaction_emoji, app.settings.reaction_emoji);
    }

    #[test]
    fn locked_conversations_close_on_back_code_changes_and_window_close() {
        for exit in ["back", "change-code", "clear-code", "window"] {
            let mut app = app();
            let ctx = egui::Context::default();
            let mut chat = Chat::new("fixture".into(), "Fixture".into());
            chat.locked = true;
            app.chats.push(chat);
            app.settings.set_chat_lock_code(Some("fixture-code"));
            app.search = "wrong-code".into();
            app.apply(Action::OpenLockedFolder, &ctx);
            assert!(!app.locked_folder);
            app.search = "fixture-code".into();
            app.apply(Action::OpenLockedFolder, &ctx);
            app.open_chat("fixture".into());
            assert!(app.current_chat().is_some());
            match exit {
                "back" => app.apply(Action::CloseLockedFolder, &ctx),
                "change-code" => app.apply(Action::SetChatLockCode(Some("new-code".into())), &ctx),
                "clear-code" => app.apply(Action::SetChatLockCode(None), &ctx),
                "window" => app.window_gone(),
                _ => unreachable!(),
            }
            assert!(!app.locked_folder, "{exit}");
            assert!(app.current_chat().is_none(), "{exit}");
            assert!(app.open_chat.is_none(), "{exit}");
            assert!(app.search.is_empty(), "{exit}");
        }
    }

    #[test]
    fn locked_chats_hide_everywhere_until_the_code_opens_the_folder() {
        let mut app = app();
        let mut a = Chat::new("1@s.whatsapp.net".into(), "Ada".into());
        a.last_activity = 10;
        a.unread = 3;
        let mut b = Chat::new("2@s.whatsapp.net".into(), "Bob".into());
        b.last_activity = 20;
        b.locked = true;
        b.unread = 5;
        app.chats = vec![b, a];
        app.settings.set_chat_lock_code(Some("1234"));

        // Hidden from the list, search, and the unread badge.
        let names: Vec<&str> = app
            .visible_chats()
            .iter()
            .map(|chat| chat.name.as_str())
            .collect();
        assert_eq!(names, vec!["Ada"]);
        app.search = "bob".into();
        assert!(app.visible_chats().is_empty());
        assert_eq!(app.unread_total(), 3);
        assert_eq!(app.unread_chats(ChatFilter::All), 1);

        // Typing the code reveals the entry; opening the folder shows only
        // the locked chats; editing the search away hides them again.
        assert!(!app.secret_code_matched());
        app.search = "1234".into();
        assert!(app.secret_code_matched());
        app.locked_folder = true;
        let names: Vec<&str> = app
            .visible_chats()
            .iter()
            .map(|chat| chat.name.as_str())
            .collect();
        assert_eq!(names, vec!["Bob"]);
        assert_eq!(app.locked_count(), 1);
        app.search = "123".into();
        assert_eq!(
            app.visible_chats()
                .iter()
                .map(|chat| chat.name.as_str())
                .collect::<Vec<_>>(),
            vec!["Ada"]
        );
        app.apply(Action::Search("123".into()), &egui::Context::default());
        assert!(!app.locked_folder);
        app.apply(Action::Search(String::new()), &egui::Context::default());
        let names: Vec<&str> = app
            .visible_chats()
            .iter()
            .map(|chat| chat.name.as_str())
            .collect();
        assert_eq!(names, vec!["Ada"]);
    }

    #[test]
    fn locked_chat_code_hint_is_shown_once() {
        let mut app = app();
        let mut chat = Chat::new("1@s.whatsapp.net".into(), "Ada".into());
        chat.locked = true;
        app.chats.push(chat);

        assert!(app.should_show_chat_lock_hint());
        app.apply(Action::DismissChatLockHint, &egui::Context::default());
        assert!(!app.should_show_chat_lock_hint());
    }

    #[test]
    fn locking_the_open_chat_closes_it() {
        let mut app = app();
        let id: ChatId = "2@s.whatsapp.net".into();
        app.chats = vec![Chat::new(id.clone(), "Bob".into())];
        app.open_chat = Some(id.clone());
        app.locked_folder = true;
        app.apply(Action::SetLocked(id, true), &egui::Context::default());
        assert!(app.open_chat.is_none());
        assert!(app.chats[0].locked);
    }

    #[test]
    fn a_remote_lock_closes_the_chat_and_hides_search_hits() {
        let mut app = app();
        let id: ChatId = "2@s.whatsapp.net".into();
        app.chats = vec![Chat::new(id.clone(), "Bob".into())];
        app.open_chat = Some(id.clone());
        app.search_hits.push(message(&id, "m", 1));
        app.composer = "Synthetic draft".into();
        let mut chat = app.chats[0].clone();
        chat.archived = true;
        chat.locked = true;
        app.handle_chat_updated(chat);
        assert!(app.open_chat.is_none());
        assert!(app.search_hits.is_empty());
        assert!(app.composer.is_empty());
        assert_eq!(
            app.drafts.get(&id).map(String::as_str),
            Some("Synthetic draft")
        );
        // A locked chat never contributes an archived row either.
        assert_eq!(app.archived_count(), 0);

        // Reopening it needs the folder open with the code typed.
        app.open_chat(id.clone());
        assert!(app.open_chat.is_none());
        app.settings.set_chat_lock_code(Some("1234"));
        app.search = "1234".into();
        app.locked_folder = true;
        app.open_chat(id.clone());
        assert_eq!(app.open_chat.as_deref(), Some(id.as_str()));
    }

    #[test]
    fn desktop_handlers_are_validated_even_for_archived_urls() {
        let mut app = app();
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(Default::default(), |_| {
            app.apply(Action::OpenUrl("file:///fixture.exe".into()), &ctx);
            app.apply(
                Action::OpenFile(PathBuf::from("/fixture/program.exe")),
                &ctx,
            );
        });
        output.textures_delta.clear();
        assert!(
            output
                .platform_output
                .commands
                .iter()
                .all(|command| !matches!(command, egui::OutputCommand::OpenUrl(_)))
        );
        assert!(
            app.actions
                .iter()
                .any(|action| matches!(action, Action::OpenFolder(_)))
        );
    }

    #[test]
    fn locked_last_chat_is_not_restored_from_a_snapshot() {
        let root = tempfile::tempdir().unwrap();
        let settings = Settings {
            last_chat: Some("locked".into()),
            ..Default::default()
        };
        let (mut app, events) = App::headless(AppDirs::under(root.path()), settings);
        let mut chat = Chat::new("locked".into(), "Fixture".into());
        chat.locked = true;
        events.send(Event::Chats(vec![chat])).unwrap();
        app.handle_events();
        assert!(app.open_chat.is_none());
        assert!(app.conversations.is_empty());
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
    fn a_saved_speed_between_choices_snaps_to_one() {
        let root = std::env::temp_dir().join(format!("zapfast-speed-{}", std::process::id()));
        let settings = Settings {
            voice_speed: 1.3,
            ..Settings::default()
        };
        let app = App::headless(AppDirs::under(&root), settings).0;
        assert_eq!(app.player.speed(), 1.25);
        assert_eq!(app.settings.voice_speed, 1.25);
    }

    #[test]
    fn direct_speed_selection_reaches_player_and_settings() {
        let mut app = app();
        let ctx = egui::Context::default();

        for speed in crate::audio::SPEEDS {
            app.apply(Action::SetVoiceSpeed(speed), &ctx);
            assert_eq!(app.player.speed(), speed);
            assert_eq!(app.settings.voice_speed, speed);
        }

        app.apply(Action::SetVoiceSpeed(4.0), &ctx);
        assert_eq!(app.settings.voice_speed, crate::audio::SPEEDS[4]);
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
        app.apply(Action::SetShortcutHints(false), &ctx);
        assert!(!app.settings.show_shortcut_hints);
        assert!(app.settings_dirty);
        app.apply(Action::SetShortcutHints(true), &ctx);
        assert!(
            app.settings.show_shortcut_hints,
            "the shortcuts dialog brings them back"
        );
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
    fn unnamed_and_cached_group_titles_share_counted_participant_names() {
        let mut app = app();
        let mut chat = Chat::new("fixture@g.us".into(), "Group".into());
        for (index, name) in [
            "Andrea North",
            "Andrea South",
            "Andrea West",
            "Giacomo East",
        ]
        .iter()
        .enumerate()
        {
            let id = format!("1555000000{index}@s.whatsapp.net");
            app.contacts.insert(
                id.clone(),
                Contact {
                    id: id.clone(),
                    full_name: Some((*name).into()),
                    push_name: Some((*name).into()),
                },
            );
            chat.participants.push(id);
        }
        // Duplicate entries for the same identity must not inflate the count.
        chat.participants.push(chat.participants[0].clone());
        chat.participants.push(app.me.clone().unwrap());
        assert_eq!(app.participant_names(&chat), "Andrea x3, Giacomo, You");
        assert_eq!(app.chat_title(&chat), app.participant_names(&chat));
        chat.name.clear();
        assert_eq!(app.chat_title(&chat), app.participant_names(&chat));
        chat.name = "Group".into();
        chat.group_subject_known = true;
        assert_eq!(
            app.chat_title(&chat),
            "Group",
            "an authoritative title is not a placeholder"
        );
        chat.name = "Weekend plans".into();
        assert_eq!(app.chat_title(&chat), "Weekend plans");
        chat.name.clear();
        chat.participants.clear();
        assert_eq!(
            app.chat_title(&chat),
            "Group",
            "no invented members while metadata is missing"
        );
    }

    #[test]
    fn saved_names_come_first_and_profile_names_fill_in() {
        let app = app();
        assert_eq!(app.display_name("1@s.whatsapp.net"), "Ada Lovelace");
        assert_eq!(app.display_name("2@s.whatsapp.net"), "~Bob");
        assert_eq!(
            app.display_name_or("3@s.whatsapp.net", Some("Cy")),
            "~Cy",
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
}
