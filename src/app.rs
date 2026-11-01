//! Application state and the frame loop.
//!
//! Views queue [`Action`]s while drawing. The app applies them after the frame
//! and processes backend events.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::account::Account;
use crate::audio::{Player, Recorder};
use crate::backend::{Backend, Command, Event, LinkStatus, Refusal, Unsent, Waker};
use crate::i18n::Locale;
use crate::image_preview::PreviewState;
use crate::model::{
    AccountId, Action, Chat, ChatFilter, ChatId, ChatMedia, Contact, Content, Delivery, Dialog,
    Gif, GifError, InfoPanel, InfoView, Label, Media, MediaState, Message, Page, PickerTab, Scroll,
    SidebarDisplayMode, StickerPack, StickerShelf, Toast, ToastKind,
};
use crate::nav::{History, Location};
use crate::paths::AppDirs;
use crate::settings::{AccountRoster, NotificationSound, Settings, ThemeChoice};
use crate::single_instance::{ControlCommand, Guard};
use crate::theme::{self, Palette};

/// The notice a failed attachment download shows, in the chat bubble and in
/// the viewer alike: an expired file has a name of its own.
fn download_notice(error: String) -> String {
    if error.contains("403") || error.contains("404") {
        "No longer available on WhatsApp's servers".to_owned()
    } else {
        error
    }
}

/// Initial and incremental message-page size.
pub const PAGE: usize = 60;
/// Minimum delay between phone history requests.
const PHONE_COOLDOWN: Duration = Duration::from_secs(6);
/// WhatsApp message-edit window.
pub const EDIT_WINDOW: Duration = Duration::from_secs(15 * 60);
/// WhatsApp revoke-for-everyone window.
pub const REVOKE_WINDOW: Duration = Duration::from_secs(2 * 24 * 60 * 60);

/// Pause, in seconds, after which a trackpad gesture ends and a new one
/// picks its own pane, as fastframe-scroll ends one. Measured on the frame's
/// input clock (`InputState::time`), not on the wall clock, so a slow frame
/// is not taken for a pause and tests can run it at their pace.
const SCROLL_GESTURE_GAP: f64 = 0.15;
/// How long a finished call's outcome stays on screen before the surface goes away, about what
/// WhatsApp's own clients show "Call ended" for. A call the reader hung up or declined here goes at
/// once instead: they know how it ended.
const CALL_FAREWELL: Duration = Duration::from_millis(1500);

/// A video call's picture on the GPU, and the frame it was last filled from.
pub struct CallTexture {
    pub texture: egui::TextureHandle,
    pub frame: std::sync::Arc<egui::ColorImage>,
}

/// A pane that scrolls on its own, which a scroll gesture stays with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollPane {
    /// The chat list, or the rail's avatars while the list is hidden.
    Chats,
    /// The open chat's messages.
    Messages,
}

/// Keeps a scroll gesture, glide included, with the pane it began over
/// (#274). egui scrolls whichever scroll area is under the pointer, so a
/// gesture that drifted over the other pane moved that one instead, and so
/// did the rest of its glide. The panes say where they are each frame; while
/// a gesture lasts, vertical scrolling with the pointer away from its pane is
/// taken from egui and handed to that pane, which applies it itself.
#[derive(Default)]
pub struct ScrollRoute {
    /// Where each pane was drawn in the last frame.
    placed: Vec<(ScrollPane, egui::Rect)>,
    /// Where each pane is drawn in this frame. Behind a lock so a view can
    /// record it while other parts of the app are borrowed.
    placing: std::sync::Mutex<Vec<(ScrollPane, egui::Rect)>>,
    /// The pane the gesture under way began over.
    owner: Option<ScrollPane>,
    /// When the gesture last had input, in egui input time.
    last_input: Option<f64>,
    /// Whether the gesture's fingers lifted: new input starts another.
    lifted: bool,
    /// Scrolling taken for the owner this frame.
    carry: Option<(ScrollPane, f32)>,
}

impl ScrollRoute {
    /// Records where `pane` is drawn this frame.
    pub fn place(&self, pane: ScrollPane, rect: egui::Rect) {
        self.placing
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push((pane, rect));
    }

    /// The vertical scrolling taken for `pane` from over another place this
    /// frame, for the pane to apply with `Ui::scroll_with_delta`.
    pub fn take(&mut self, pane: ScrollPane) -> f32 {
        match self.carry {
            Some((to, delta)) if to == pane => {
                self.carry = None;
                delta
            }
            _ => 0.0,
        }
    }

    /// Picks the gesture's pane and takes its scrolling from elsewhere.
    /// `moved` is whether wheel input arrived this frame, `lifted` whether
    /// the fingers left the trackpad, and `gliding` whether fastframe-scroll's
    /// glide is still adding to the scroll.
    fn route(&mut self, ctx: &egui::Context, moved: bool, lifted: bool, gliding: bool) {
        self.placed = std::mem::take(self.placing.get_mut().unwrap_or_else(|p| p.into_inner()));
        self.carry = None;
        let now = ctx.input(|input| input.time);
        // Floating layers, such as menus and dialogs, are no pane.
        let under = ctx
            .input(|input| input.pointer.hover_pos())
            .filter(|pos| {
                ctx.layer_id_at(*pos)
                    .is_none_or(|layer| layer.order == egui::Order::Background)
            })
            .and_then(|pos| {
                self.placed
                    .iter()
                    .find(|(_, rect)| rect.contains(pos))
                    .map(|(pane, _)| *pane)
            });
        let recent = self
            .last_input
            .is_some_and(|at| now - at < SCROLL_GESTURE_GAP);
        if moved {
            if self.lifted || !recent {
                self.owner = under;
            }
            self.last_input = Some(now);
            self.lifted = false;
        }
        self.lifted |= lifted;
        let settling = ctx.input(|input| input.smooth_scroll_delta != egui::Vec2::ZERO);
        if !moved && !gliding && !settling && !recent {
            self.owner = None;
        }
        let Some(owner) = self.owner else {
            return;
        };
        if under != Some(owner) && self.placed.iter().any(|(pane, _)| *pane == owner) {
            let delta = ctx.input_mut(|input| std::mem::take(&mut input.smooth_scroll_delta.y));
            if delta != 0.0 {
                self.carry = Some((owner, delta));
            }
        }
    }
}
/// Delay after the last keystroke before clearing typing state.
const COMPOSING_TIMEOUT: Duration = Duration::from_secs(4);
/// How long an info toast stays, including its fade.
pub const INFO_TOAST_LIFETIME: Duration = Duration::from_millis(3200);
/// Error toasts kept on screen at once; older ones give way to newer ones.
const MAX_ERROR_TOASTS: usize = 3;
/// Typing-state timeout when no stop event arrives.
const TYPING_TIMEOUT: Duration = Duration::from_secs(12);
/// How long the chat list's search waits after the last keystroke before it
/// scans the archive, which takes seconds on a long history.
const SEARCH_DELAY: Duration = Duration::from_millis(250);
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
    /// Whether the active phone request is the reader's own (see
    /// [`Command::FetchOlder`]).
    pub phone_explicit: bool,
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
        // Prepend new older rows, but refresh duplicates too: on-demand
        // history can supply edits and phone ordering absent from our archive.
        let old_len = self.messages.len();
        let mut positions: HashMap<String, usize> = self
            .messages
            .iter()
            .enumerate()
            .map(|(index, message)| (message.id.clone(), index))
            .collect();
        for message in incoming {
            match positions.get(&message.id) {
                Some(&index) => {
                    let existing = &mut self.messages[index];
                    let media = existing
                        .content
                        .media()
                        .map(|media| (media.state.clone(), media.path.clone()));
                    *existing = message;
                    // Keep downloads in flight and files absent from a freshly
                    // classified copy. An incoming path takes precedence.
                    if let (Some((state, path)), Some(media)) =
                        (media, existing.content.media_mut())
                        && media.path.is_none()
                    {
                        media.state = state;
                        media.path = path;
                    }
                }
                None => {
                    positions.insert(message.id.clone(), self.messages.len());
                    self.messages.push(message);
                }
            }
        }
        if older {
            let added = self.messages.len() - old_len;
            self.messages.rotate_right(added);
        }
        self.sort();
    }

    fn sort(&mut self) {
        self.messages
            .sort_by_key(|message| (message.timestamp, message.history_order.unwrap_or(i64::MAX)));
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

/// Messages swept by dragging over them (#246): everything from the row
/// the drag began on to the row under the pointer joins what was selected
/// before, by the chat's order, so rows the list skipped count too.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Sweep {
    pub chat: ChatId,
    pub anchor: String,
    /// The row under the pointer when the sweep last moved.
    pub to: String,
    base: Vec<String>,
}

/// Whether a message can join a selection, matching `Worker::forward_job`.
pub(crate) fn can_select(content: &Content) -> bool {
    !matches!(
        content,
        Content::Revoked
            | Content::PhoneOnly { .. }
            | Content::Unsupported { .. }
            | Content::Poll { .. }
            | Content::Interactive { .. }
            | Content::Call { .. }
    )
}

/// Adds the messages from `anchor` to `to` to a selection, in either
/// direction, keeping the chat's order and skipping content that cannot
/// be forwarded.
fn add_range(messages: &[Message], ids: &mut Vec<String>, anchor: &str, to: &str) {
    let position = |id: &str| messages.iter().position(|message| message.id == id);
    let (Some(from), Some(to)) = (position(anchor), position(to)) else {
        return;
    };
    let (from, to) = (from.min(to), from.max(to));
    for message in &messages[from..=to] {
        if can_select(&message.content) && !ids.contains(&message.id) {
            ids.push(message.id.clone());
        }
    }
    ids.sort_by_key(|id| position(id).unwrap_or(usize::MAX));
}

/// The info panel control the keyboard moves to after a view change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum InfoFocus {
    /// The Back button, after the media row opened the media view.
    Back,
    /// The media row, after Back returned to the overview.
    MediaRow,
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

/// A replied message the keys asked to go to while it was not loaded. It
/// ends quietly as soon as the reader moves on, as Alt+3's jump does.
#[derive(Clone)]
struct PendingFollow {
    chat: ChatId,
    message: String,
    /// The widget focused when it was asked: focus moved elsewhere ends it.
    focus: Option<egui::Id>,
    /// The frame it was asked in. A key or text in a later one ends it.
    frame: u64,
    /// Whether its own request for history toward the message is on its
    /// way. Only that request's answer can say the message is not there.
    asked: bool,
    /// That answer came empty: "Replied message not found" follows. A
    /// request that failed answers with its error instead
    /// (`Event::HistoryUnavailable`), and ends the wait.
    answered: bool,
}

pub struct App {
    pub dirs: AppDirs,
    pub settings: Settings,
    /// Writes settings and the account roster on a thread of its own, so
    /// a save never stalls the window; flushed at exit and before a suspend.
    pub files: fastframe_file::Writer,
    /// Resolved interface language, from the setting or the system locale.
    pub locale: Locale,
    settings_dirty: bool,
    last_settings_save: Instant,
    pub roster: AccountRoster,
    pub accounts: Vec<Account>,
    pub active: usize,
    pub adding_account: bool,
    /// The account on screen when "Add account" was chosen, for Cancel.
    account_before_adding: Option<AccountId>,
    /// Whether the account switcher under our own avatar is open.
    pub account_menu: bool,
    pub palette: Palette,
    pub custom_themes: theme::Catalog,
    applied_dark: Option<bool>,
    /// Reveals a new palette from the middle of the window outwards.
    theme_transition: fastframe_theme::Transition,
    /// Whether palette changes are revealed. Off in tests, whose frames send
    /// no screenshots and would hold the old palette.
    pub reveal_theme_changes: bool,
    zoom_applied: bool,
    /// The wallpaper image named in the settings, decoded off this thread.
    pub wallpaper_image: crate::wallpaper::CustomImage,

    /// A sent message moves its chat up, so the chat list goes to the top,
    /// if it is near the top already (the list decides, from its offset).
    pub scroll_chats_to_top: bool,
    /// Image URIs whose file changed under them, for egui to read afresh on
    /// the next frame: it caches a picture by its URI for as long as it is
    /// on screen, and a chat's picture keeps one file name across changes.
    stale_images: Vec<String>,
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
    /// Private quotes kept with their direct chat while switching conversations.
    /// Deletion tombstones prevent refused private sends from reviving a replacement chat.
    /// Account baseline, advanced on unlink without reusing an old destination lifetime.
    /// Reply target in the open chat.
    pub reply_to: Option<crate::model::ReplyTarget>,
    /// Outgoing message being edited.
    pub editing: Option<String>,
    composing: bool,
    last_keystroke: Option<Instant>,
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
    /// The info panel beside the conversation, when open.
    pub info: Option<InfoPanel>,
    /// The listing of the panel's chat, once the worker has answered.
    pub info_media: Option<ChatMedia>,
    /// Whether a listing is on its way from the worker.
    pub info_media_pending: bool,
    /// Whether the chat changed while a listing was on its way, so it is
    /// asked for again once that one is in.
    info_media_stale: bool,
    /// The thumbnails the panel has registered with egui, freed when it
    /// closes or a listing drops them, rather than when they fall out of
    /// the resident window.
    pub(crate) info_textures: HashSet<String>,
    /// The texture name of each listed picture, in the listing's order.
    pub(crate) info_uris: Vec<String>,
    /// Texture names a new listing or another chat's left out, freed on
    /// the panel's next frame though the panel stays open.
    pub(crate) info_released: Vec<String>,
    /// The request the panel is waiting for; an earlier one's answer is
    /// ignored.
    info_media_request: u64,
    /// Whether the worker could not make the listing asked for last.
    pub info_media_failed: bool,
    /// The shown group's members in display order, sorted once per change
    /// rather than every frame.
    pub(crate) info_members: Option<(ChatId, Vec<(String, String)>)>,
    /// The shown group's invite link, asked for when its row is drawn and
    /// dropped with the panel: it is a secret, never kept longer.
    pub info_invite: Option<(ChatId, crate::group_admin::LinkState)>,
    /// The panel control that takes the keyboard on the next frame.
    pub(crate) info_focus: Option<InfoFocus>,
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
    /// Messages being swept with the mouse held down, if any.
    pub(crate) sweep: Option<Sweep>,
    /// Whether files are being dragged over the window.
    pub dropping: bool,
    /// Where the Ctrl/Cmd+V shortcut stands across frames.
    paste_keys: PasteKeys,
    /// Open emoji, GIF, or sticker picker tab.
    pub picker: Option<PickerTab>,
    /// Picker anchor at the composer button.
    pub picker_anchor: Option<egui::Rect>,
    pub picker_search: String,
    /// Recent emoji as they stood when the emoji picker opened. The row keeps
    /// that order while the picker is open, so the same emoji can be sent
    /// again from where it was; it catches up the next time it opens (#294).
    pub picker_recent: Option<Vec<String>>,
    /// Whether the newly opened picker should focus search.
    pub picker_focus: bool,
    /// Message the full emoji reaction picker is targeting.
    pub reaction_target: Option<(ChatId, String)>,
    /// Control that opened the reaction picker.
    pub reaction_anchor: Option<egui::Rect>,
    /// The reaction picker came from a message's context menu, which stays
    /// open beside it.
    pub reaction_beside_menu: bool,
    /// One-shot: open this message's context menu, by chat and message id,
    /// when its bubble is next drawn. Set by the message list's keys that
    /// open it, and by demo pages; the menu then stays open until it is
    /// closed. A chat or page change drops a request not yet drawn.
    pub open_message_menu: Option<(ChatId, String)>,
    /// The reaction picker came from the message list's E: closing it
    /// returns focus to the list.
    pub reaction_from_list: bool,
    /// Demo/test: keep this chat row's context menu open.
    #[cfg(any(test, feature = "demo"))]
    pub open_chat_menu: Option<ChatId>,
    /// Demo/test: keep the open chat's header menu open.
    #[cfg(any(test, feature = "demo"))]
    pub open_header_menu: Option<ChatId>,
    /// Demo/test: keep this group member's menu open in the info panel.
    #[cfg(any(test, feature = "demo"))]
    pub open_member_menu: Option<ChatId>,
    /// Demo/test: keep the selection bar's delete menu open.
    #[cfg(any(test, feature = "demo"))]
    pub open_selection_delete_menu: bool,
    /// Emoji-grid header to scroll into view.
    pub emoji_jump: Option<&'static str>,
    /// Attachments pending in the composer.
    pub pending: Vec<Staged>,
    /// Whether the single staged photo will be sent as view-once media.
    pub(crate) pending_view_once: bool,
    /// The next identity to give a staged attachment.
    next_pending_id: AttachmentId,
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
    /// The replied message the keys asked to go to while it was not loaded:
    /// the list moves to it once history brings it, and stays put until then.
    pending_follow: Option<PendingFollow>,
    /// Chat whose voice messages carry on into the next unheard one when a
    /// clip ends. Leaving the chat, or playing a video, ends the run.
    voice_chat: Option<ChatId>,
    /// Chat of the clip in the player, whose speed it plays at. Unlike
    /// `voice_chat` it outlives a chat switch, as the clip keeps playing.
    voice_clip_chat: Option<ChatId>,
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
    /// The motion photo whose clip plays over it: its chat and message.
    pub motion_playing: Option<(ChatId, String)>,
    /// Sticker currently shown large.
    pub sticker_view: Option<crate::image_preview::StickerPreview>,
    /// Photos and playable videos of the open chat, oldest first, as the viewer
    /// album lists them. Loaded when the viewer opens.
    pub viewer_media: Vec<crate::model::MediaItem>,
    /// Chat whose album `viewer_media` belongs to, so a late answer for another
    /// chat is dropped instead of shown.
    pub viewer_media_chat: Option<ChatId>,
    /// What each viewer item's download is doing, keyed by message id. The
    /// album reaches messages the transcript has not paged in, so their
    /// progress and their failure have nowhere else to live.
    pub viewer_media_state: HashMap<String, MediaState>,
    /// Message bodies registered for transcript copy formatting.
    pub copy_rows: std::sync::Arc<std::sync::Mutex<Vec<crate::transcript::Row>>>,
    /// Previous message-list rect used by the selection hook.
    pub selection_view: std::sync::Arc<std::sync::Mutex<Option<egui::Rect>>>,
    pub gif_query: String,
    pub gif_results: Vec<Gif>,
    pub gif_next_offset: Option<usize>,
    gif_request: u64,
    /// Whether a GIF search is active.
    pub gif_pending: bool,
    pub gif_error: Option<GifError>,
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
    /// The staged picture being cropped, while the cropper is open.
    pub picture_edit: Option<crate::model::PictureEdit>,
    /// The staged picture whose size was asked for, and whose cropper that
    /// answer may still open. A size that arrives for anything else has been
    /// overtaken by a cancellation or by another picture, and opening the
    /// cropper on it would take the window off whoever has it now.
    pub picture_inspecting: Option<AttachmentId>,
    /// Whether the cropper asked for follows focus: asked for from the keys
    /// or with a screen reader, as `ui::overlay::followed` says at the time
    /// of asking. A file opens the cropper only once its size is read, in a
    /// later frame that no longer knows how it was asked for.
    pub picture_edit_followed: Option<bool>,
    /// Where the picture being written came from, by the attachment it will
    /// replace, so two edits in flight cannot swap their origins.
    pub picture_applying: std::collections::HashMap<AttachmentId, crate::model::PictureOrigin>,
    /// Where each cropped staged picture came from, by the attachment it
    /// replaced. Cropping it again starts from that original, and two pictures
    /// that happen to be written to the same file keep their own answer.
    pub picture_origins: std::collections::HashMap<AttachmentId, crate::model::PictureOrigin>,
    /// The pixels of a pasted picture being cropped again, keyed by the buffer
    /// they came from. Cropping a pasted picture leaves a file where the strip
    /// had the pixels, so the cropper has to draw from the buffer itself.
    pub picture_texture: Option<(usize, egui::TextureHandle)>,
    /// A pack shared in a chat, being viewed: the pack and its publisher.
    pub sticker_preview: Option<(StickerPack, String)>,
    /// Whether the viewed pack is still downloading.
    pub sticker_preview_pending: bool,
    /// A pack just created here, selected once the backend lists it.
    sticker_pack_created: Option<String>,
    /// The wheel step, and on Linux the touchpad's scale, glide and axis
    /// lock.
    pub(crate) scrolling: fastframe_scroll::Scrolling,
    /// Keeps a scroll gesture with the pane it began over.
    pub scroll_route: ScrollRoute,

    pub page: Page,
    pub dialog: Option<Dialog>,
    /// Places visited in this window, for the mouse's back and forward
    /// buttons. Cleared when the account on screen changes.
    nav_history: History,
    /// Chat filter in the forwarding destination dialog.
    pub forward_search: String,
    /// Recipients chosen in the forwarding dialog, all chats of the account
    /// on screen, in the order they were picked.
    pub forward_recipients: Vec<ChatId>,
    /// Whether the forwarding dialog shows its recipients for confirmation.
    pub forward_reviewing: bool,
    /// How many recipients the forwarding dialog's messages may have.
    pub forward_limit: usize,
    /// The group name being typed in the group info dialog.
    pub group_name_edit: Option<String>,
    /// The new-contact dialog's "Save to phone" box.
    pub new_contact_to_phone: bool,
    pub sidebar_visible: bool,
    /// Demo/test: keep this starred row's context menu open.
    #[cfg(any(test, feature = "demo"))]
    pub open_list_menu: Option<(ChatId, String)>,
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
    /// Invalidates editor history when a composer closes and reopens before
    /// another frame, including a new session reusing the same account id.
    composer_undo_epoch: u64,
    /// Text a link put in the composer, with the chat it belongs to. Text the
    /// reader never changed is not their draft, so a chat they leave does not
    /// store it in place of the one they wrote.
    prefilled: Option<(ChatId, String)>,
    /// Actions queued by a backend event, applied on that account after the frame.
    deferred_account_actions: Vec<(AccountId, Action)>,
    /// Set while the events of an account that is not on screen are being
    /// applied: they update that account only, never the window's composer,
    /// dialogs, playback, or read state.
    events_hidden: bool,
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
    /// The current message of each chat's message list, by id: where the
    /// arrows move from and what a screen reader reads. A chat without one
    /// starts on its newest message.
    pub message_cursor: HashMap<ChatId, String>,
    /// The newest message the keys took the list's cursor to, with its chat,
    /// at the true bottom. While the cursor stays on it, the list holds the
    /// view at the bottom as the return-to-bottom button does, so messages
    /// arriving after it are followed.
    pub cursor_followed: Option<(ChatId, String)>,
    /// One-shot: scroll the open chat's current message into view.
    pub reveal_cursor: bool,
    /// Whether the open chat's message list had keyboard focus in the last
    /// frame drawn. Actions cannot see egui focus, so the list records it.
    pub messages_focused: bool,
    /// One-shot: give the open chat's message list keyboard focus when it is
    /// next drawn.
    pub focus_messages: bool,
    /// A read-only chat's message list asked for focus in a pass that drew
    /// it hidden: the chat it was for, which a later visible pass of that
    /// chat honours, and no other chat.
    pub(crate) list_focus_deferred: Option<ChatId>,
    pub focus_composer: bool,
    pub focus_search: bool,
    /// What the Settings page is filtered by.
    pub settings_search: String,
    pub focus_settings_search: bool,
    pub quit_requested: bool,
    pub window_focused: bool,
    /// A notification went out this frame: highlight the taskbar entry of a
    /// window that is open but not focused.
    wants_attention: bool,
    /// The live call, or the outcome of the one that just ended. One call at
    /// a time across every account: there is one microphone.
    pub call: Option<crate::calls::CallUpdate>,
    /// The account the call is on, which need not be the one on screen.
    pub call_account: Option<AccountId>,
    /// When a finished call's surface should disappear.
    call_surface_until: Option<Instant>,
    /// The microphones and speakers the call screen offers.
    pub call_devices: crate::calls::DeviceList,
    /// Whether the call screen shows its device pickers.
    pub call_devices_open: bool,
    /// Whether the full call screen is put aside so a chat can be read while
    /// the call runs. The call itself is untouched; a bar offers the way back.
    pub call_surface_hidden: bool,
    /// Whether the call surface has taken the whole screen. A window state:
    /// leaving it never touches the call, and a call that ends puts the
    /// window back itself.
    pub call_fullscreen: bool,
    /// The incoming call the desktop was told about, so its notification can
    /// be taken back once it is answered or given up.
    call_notified: Option<u64>,
    /// The incoming call (by generation) whose notification may sound, so
    /// its ringtone plays: notifications on, a sound chosen, and a chat that
    /// is not muted, archived or locked.
    call_rings: Option<u64>,
    /// The ringback and ringtone, played on the default output in the real
    /// app and only tracked in tests and demos.
    call_tones: crate::call_tones::Tones,
    /// The call the reader hung up or declined here: it left the screen at
    /// once, and the worker's last updates about it are not drawn again.
    call_dismissed: Option<u64>,
    /// Set when the reader hangs up a call still being placed, so the worker
    /// does not send its offer.
    call_cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    /// When the call on screen was placed here, while the worker has not
    /// said anything about it yet.
    call_placed: Option<Instant>,
    /// The newest camera preview and peer picture of a video call.
    pub call_local_frame: Option<std::sync::Arc<egui::ColorImage>>,
    pub call_remote_frame: Option<std::sync::Arc<egui::ColorImage>>,
    /// The textures those pictures are drawn through, updated in place rather
    /// than made afresh for every frame.
    pub call_local_texture: Option<CallTexture>,
    pub call_remote_texture: Option<CallTexture>,
    /// The newest picture of each group member whose video is arriving, by chat id, and the
    /// texture each is drawn through.
    pub call_member_frames: HashMap<String, std::sync::Arc<egui::ColorImage>>,
    pub call_member_textures: HashMap<String, CallTexture>,
    /// Whether ZapFast starts at login, when this installation supports it.
    pub start_with_system: Option<bool>,
    /// The read or change of the login item running on its thread.
    login_item: crate::autostart::LoginItem,
    /// Cross-thread window repaint handle.
    waker: Waker,
    tray: Option<fastframe_tray::Tray>,
    /// Sleep and wake as the system reports them. Tests and demo runs get a
    /// fixed one that reads nothing from the machine.
    power: fastframe_power::Power,
    /// The desktop portal's light or dark preference, which winit does not
    /// report on Linux. Tests and demo runs read none.
    #[cfg(target_os = "linux")]
    system_appearance: Option<crate::appearance::SystemAppearance>,
    /// Stands in for whether a panel shows the tray item, which tests cannot
    /// spawn.
    #[cfg(test)]
    test_tray_shown: Option<bool>,
    /// Stands in for the compositor raising the window, so tests never run
    /// its command line tool.
    #[cfg(test)]
    test_raised: bool,
    /// Whether the tray menu offers "Lock ZapFast": whether an app lock
    /// password was set when it last changed.
    tray_lockable: bool,
    /// Whether the app is running without a window.
    pub window_hidden: bool,
    /// Whether window close should keep the process running.
    pub hide_intent: bool,
    /// Whether a headless app should create a window.
    pub wants_show: bool,
    /// A hidden start whose tray item no panel shows yet waits until then
    /// for one to appear, and shows the window if none has (#429).
    tray_wait: Option<Instant>,
    /// Whether this session draws through Wayland, where a compositor ignores
    /// a programmatic focus or unminimize request and the window is closed and
    /// reopened instead.
    wayland: bool,
    /// Whether the current window close should reopen a fresh one at once
    /// (`Closed::Reopen`) rather than stop drawing.
    reopen: bool,
    /// Requests received from later launches.
    control_commands: Option<std::sync::Arc<std::sync::Mutex<Vec<ControlCommand>>>>,
    /// Chats and messages from clicked notifications.
    notification_opens: std::sync::Arc<std::sync::Mutex<Vec<crate::notify::NotificationTarget>>>,
    notifications: crate::notify::Notifications,
    /// Unread count on the taskbar icon, where the desktop reads it. `None`
    /// for demo and test runs, which must not touch the real taskbar.
    badge: Option<crate::notify::Badge>,
    /// The app lock: whether the window shows only the lock screen, the
    /// inactivity count, and the Settings password form.
    pub app_lock: crate::app_lock::AppLock,
    /// Spoken announcements for screen readers, emitted every frame.
    pub announcer: crate::announcer::Announcer,
    /// Whether "Connection lost" was announced, so the way back is announced
    /// too while the startup connection stays silent.
    announced_offline: bool,
    /// "Connecting" is spoken once per offline episode, not on every retry.
    announced_connecting: bool,
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

/// Attachments and their quote belong to one chat in one account.
#[derive(Default)]
pub(crate) struct AttachmentDraft {
    /// Kept with their names, so what is remembered about a picture survives
    /// a switch and cropping it again still starts from its original rather
    /// than from the crop that came out of it.
    pub(crate) pending: Vec<Staged>,
    pub(crate) view_once: bool,
    quoting: Option<crate::model::ReplyTarget>,
}

/// Localizes known number-lookup failures while preserving unknown failure details.
fn contact_failure_message(locale: crate::i18n::Locale, error: &str) -> String {
    match error {
        "Could not check the number. Please try again." => {
            crate::i18n::gettext(locale, "Could not check the number. Please try again.")
                .into_owned()
        }
        "This number is not on WhatsApp" => {
            crate::i18n::gettext(locale, "This number is not on WhatsApp").into_owned()
        }
        "Not connected to WhatsApp" => {
            crate::i18n::gettext(locale, "Not connected to WhatsApp").into_owned()
        }
        "Enter a valid international phone number" => {
            crate::i18n::gettext(locale, "Enter a valid international phone number").into_owned()
        }
        _ => error.to_owned(),
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
    /// A file to send as a document with its own bytes, even a picture.
    Document(PathBuf),
}

/// What names a staged attachment. It is handed out when the attachment is
/// staged and never changes afterwards, so work that finishes later can say
/// which attachment it belongs to instead of which place in the strip it sat
/// in when the work started.
pub type AttachmentId = u64;

/// One attachment in the strip, with the identity it keeps while it is staged.
pub struct Staged {
    pub id: AttachmentId,
    pub item: Pending,
}

impl App {
    /// Stages an attachment and answers with the identity the backend names
    /// when it is done with it.
    pub fn stage(&mut self, item: Pending) -> AttachmentId {
        let id = self.next_attachment_id();
        self.pending.push(Staged { id, item });
        id
    }

    /// Whether exactly one staged photo can be sent with WhatsApp's view-once
    /// restriction. Documents and batches cannot carry this option.
    pub(crate) fn pending_supports_view_once(&self) -> bool {
        matches!(
            self.pending.as_slice(),
            [Staged {
                item: Pending::Picture { .. },
                ..
            }]
        ) || matches!(
            self.pending.as_slice(),
            [Staged { item: Pending::File(path), .. }]
                if Pending::is_view_once_photo_file(path)
        )
    }

    fn set_pending_view_once(&mut self, view_once: bool) {
        self.pending_view_once = view_once && self.pending_supports_view_once();
    }

    /// The identity for the next attachment, whether it lands in the composer
    /// or in a chat's draft.
    fn next_attachment_id(&mut self) -> AttachmentId {
        let id = self.next_pending_id;
        self.next_pending_id += 1;
        id
    }

    /// Where a staged attachment sits now, if it is still staged at all. Where
    /// it sits moves when another attachment is removed, so nothing holds on to
    /// the answer.
    pub fn staged_at(&self, id: AttachmentId) -> Option<usize> {
        self.pending.iter().position(|staged| staged.id == id)
    }

    /// Forgets what was remembered about attachments that are no longer staged,
    /// so neither map keeps an entry for every picture ever cropped, and a
    /// removed picture's original does not outlive it. Attachments waiting in
    /// a chat's draft are still kept, so what is remembered about them stays
    /// with them until that draft goes. What is remembered is shared by every
    /// account, while drafts are not, so all of them are looked at rather than
    /// the one on screen: a picture waiting in another account is still a
    /// picture, and coming back to it must still know where it came from.
    pub fn forget_unstaged_pictures(&mut self) {
        let mut live: Vec<AttachmentId> = self.pending.iter().map(|staged| staged.id).collect();
        for account in &self.accounts {
            for draft in account.attachment_drafts.values() {
                live.extend(draft.pending.iter().map(|staged| staged.id));
            }
        }
        self.picture_origins.retain(|id, _| live.contains(id));
        self.picture_applying.retain(|id, _| live.contains(id));
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// A display-name mention selected in the composer and captured with a send.
pub struct ComposerMention {
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

    /// Whether this file can be decoded and sent as a WhatsApp view-once photo.
    pub fn is_view_once_photo_file(path: &std::path::Path) -> bool {
        mime_guess2::from_path(path)
            .first_raw()
            .is_some_and(|mime| matches!(mime, "image/jpeg" | "image/png" | "image/webp"))
    }
}

/// Whether this session draws through Wayland, where a compositor ignores an
/// app's request to focus or unminimize one of its own windows: winit's
/// Wayland `focus_window` is a no-op and `set_minimized(false)` only warns.
/// Mirrors winit's own choice: Wayland when either variable is non-empty.
#[cfg(target_os = "linux")]
fn wayland_session() -> bool {
    ["WAYLAND_DISPLAY", "WAYLAND_SOCKET"]
        .into_iter()
        .any(|name| std::env::var_os(name).is_some_and(|value| !value.is_empty()))
}

/// Wayland is Linux-only; elsewhere a window focuses and restores normally.
#[cfg(not(target_os = "linux"))]
fn wayland_session() -> bool {
    false
}

/// The app outlives its window: closing it with "keep running" on hides
/// ZapFast, and the tray, a notification or another launch brings it back.
impl fastframe_shell::Resident for App {
    fn closed(&self) -> fastframe_shell::Closed {
        if self.quit_requested {
            fastframe_shell::Closed::Quit
        } else if self.reopen {
            // The window was closed only to make a fresh one, which is how a
            // Wayland session brings a minimized or covered window forward.
            fastframe_shell::Closed::Reopen
        } else if self.hide_intent {
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
        self.check_tray_wait(Instant::now());
        if self.quit_requested {
            fastframe_shell::Headless::Quit
        } else if self.wants_show {
            fastframe_shell::Headless::Show
        } else {
            fastframe_shell::Headless::Wait
        }
    }

    /// Without a tray there is no way back to a hidden window, so show it.
    /// A tray item no panel shows yet may only be early: a start at login
    /// can beat the desktop's tray watcher (GNOME's AppIndicator extension
    /// loads after the session's autostart apps), so the window waits
    /// `TRAY_WAIT` for a panel before it shows.
    fn start_hidden(&mut self) -> bool {
        if !self.has_tray() || !self.settings.keep_running_in_background {
            return false;
        }
        if !self.tray_shown() {
            self.tray_wait = Some(Instant::now() + TRAY_WAIT);
        }
        App::start_hidden(self);
        true
    }

    fn shutdown(&mut self) {
        App::shutdown(self);
    }
}

/// How long a hidden start waits for a panel to show the tray item.
const TRAY_WAIT: Duration = Duration::from_secs(30);

const TRAY_SHOW: &str = "show";
const TRAY_LOCK: &str = "lock";
const TRAY_QUIT: &str = "quit";

/// What a tray click asks for: a left click on Linux, or the menu's first
/// entry, toggles the window; a left click on Windows and a Dock click on
/// macOS show it. Clicking the macOS menu-bar icon opens its menu.
fn tray_action(event: fastframe_tray::Event, window_hidden: bool) -> Option<Action> {
    use fastframe_tray::Event;
    Some(match event {
        Event::Show => Action::ShowWindow,
        Event::Toggle | Event::Menu(TRAY_SHOW) if window_hidden => Action::ShowWindow,
        Event::Toggle | Event::Menu(TRAY_SHOW) => Action::HideWindow,
        Event::Menu(TRAY_LOCK) => Action::LockApp,
        Event::Menu(TRAY_QUIT) => Action::Quit,
        Event::Menu(_) => return None,
    })
}

/// The tray item: ZapFast's icon, and a menu to show or hide the window,
/// to lock it while an app lock password is set, and to quit. The title and
/// the menu never name a chat, so they are safe while locked.
///
/// "Lock ZapFast" is always in the menu, hidden without a password;
/// `App::sync_tray` shows or hides it as the password is set or removed.
fn tray_config(lockable: bool) -> fastframe_tray::Config {
    use fastframe_tray::MenuItem;
    fastframe_tray::Config {
        id: "zapfast",
        title: "ZapFast".into(),
        icon: crate::util::app_icon_rgba,
        template_icon: Some(crate::util::tray_template_rgba),
        // The tray icon is the app icon, so hosts that draw only named icons
        // may use the installed one.
        themed_icon: true,
        menu: vec![
            MenuItem::action(TRAY_SHOW, "Show or hide ZapFast"),
            MenuItem::action(TRAY_LOCK, "Lock ZapFast").visible(lockable),
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

impl std::ops::Deref for App {
    type Target = Account;

    fn deref(&self) -> &Account {
        &self.accounts[self.active]
    }
}

impl std::ops::DerefMut for App {
    fn deref_mut(&mut self) -> &mut Account {
        &mut self.accounts[self.active]
    }
}

impl App {
    pub fn new(
        waker: &Waker,
        dirs: AppDirs,
        settings: Settings,
        options: AppOptions,
    ) -> std::io::Result<Self> {
        crate::proxy::configure(&settings.proxy);
        let mut roster = AccountRoster::load(&dirs.accounts_file())?;
        if roster.order.is_empty() {
            roster = AccountRoster::default();
        }
        let mut accounts = Vec::new();
        for id in &roster.order {
            let Some(id) = AccountId::parse(id) else {
                continue;
            };
            accounts.push(Account::spawn(&dirs, id, &settings, waker)?);
        }
        if accounts.is_empty() {
            accounts.push(Account::spawn(&dirs, AccountId::first(), &settings, waker)?);
            roster = AccountRoster::default();
        }
        let active = roster
            .order
            .iter()
            .position(|id| id == &roster.active)
            .unwrap_or(0)
            .min(accounts.len() - 1);
        let mut app = Self::with_accounts(dirs, settings, roster, accounts, active, waker.clone());
        app.pauses_media = true;
        app.call_tones = crate::call_tones::Tones::new(true);
        app.badge = Some(crate::notify::Badge::new(fastframe_desktop::desktop_entry(
            "zapfast",
        )));
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
        let power_waker = waker.clone();
        app.power = fastframe_power::Power::spawn(move || power_waker.wake());
        #[cfg(target_os = "linux")]
        {
            let appearance_waker = waker.clone();
            app.system_appearance = Some(crate::appearance::SystemAppearance::spawn(move || {
                appearance_waker.wake()
            }));
        }
        if options.tray {
            let waker = waker.clone();
            app.tray =
                fastframe_tray::Tray::spawn(tray_config(app.tray_lockable), move || waker.wake());
        }
        // The clock preference may wait on the desktop portal on Linux; keep
        // it off the first frame.
        std::thread::Builder::new()
            .name("clock-format".into())
            .spawn(|| {
                crate::util::twelve_hour_clock();
            })
            .ok();
        let folder = app.settings.download_folder.clone();
        for account in &app.accounts {
            account
                .backend
                .send(Command::SetDownloadFolder(folder.clone()));
            account.backend.send(Command::SetKeepChatsArchived(
                app.settings.keep_chats_archived,
            ));
        }
        let login_waker = waker.clone();
        app.login_item.read(move || login_waker.wake());
        let _ = app.roster.save_to(&app.files, &app.dirs.accounts_file());
        Ok(app)
    }

    /// Single-instance guard used by later launches.
    pub fn set_remote_control(&mut self, guard: &Guard) {
        self.control_commands = Some(guard.commands());
    }

    /// Creates a disconnected app and event sender for demos and tests.
    pub fn headless(dirs: AppDirs, settings: Settings) -> (Self, std::sync::mpsc::Sender<Event>) {
        let (account, events) = Account::detached(
            &dirs,
            AccountId::first(),
            crate::settings::AccountSettings::from_legacy(&settings),
        )
        .expect("test account folders");
        let roster = AccountRoster {
            active: account.id.0.clone(),
            ..AccountRoster::default()
        };
        let app = Self::with_accounts(dirs, settings, roster, vec![account], 0, Waker::default());
        (app, events)
    }

    /// Starts without a window (`--start-hidden`). The link and the archive
    /// start now: they otherwise wait for a first frame, which a hidden start
    /// only draws once the tray or another launch shows the window.
    pub fn start_hidden(&mut self) {
        self.hide_intent = true;
        // No window will attach the tray, which makes the macOS item; make
        // it now without bringing ZapFast forward.
        if let Some(tray) = &mut self.tray {
            tray.create_item();
        }
        for account in &mut self.accounts {
            if let Some(startup) = account.backend.take_startup() {
                let _ = startup.send(());
            }
        }
    }

    #[cfg_attr(not(test), expect(dead_code))]
    fn with_backend(dirs: AppDirs, settings: Settings, backend: Backend, waker: Waker) -> Self {
        let (account, _) = {
            let id = AccountId::first();
            let account_dirs = dirs.account(&id);
            let _ = account_dirs.ensure();
            (
                Account::new(
                    id,
                    account_dirs,
                    crate::settings::AccountSettings::from_legacy(&settings),
                    backend,
                ),
                (),
            )
        };
        let roster = AccountRoster {
            active: account.id.0.clone(),
            ..AccountRoster::default()
        };
        Self::with_accounts(dirs, settings, roster, vec![account], 0, waker)
    }

    fn with_accounts(
        dirs: AppDirs,
        settings: Settings,
        roster: AccountRoster,
        accounts: Vec<Account>,
        active: usize,
        waker: Waker,
    ) -> Self {
        let palette = settings
            .cached_palette()
            .unwrap_or_else(|| match settings.theme {
                ThemeChoice::Light => Palette::light(),
                ThemeChoice::Messages => Palette::messages(),
                _ => Palette::dark(),
            });
        let locale = crate::i18n::resolve(settings.interface_language);
        // With a password set, ZapFast starts locked.
        let app_lock = crate::app_lock::AppLock::new(settings.app_lock_hash.is_some());
        let tray_lockable = settings.app_lock_hash.is_some();
        let mut app = Self {
            dirs,
            settings,
            locale,
            settings_dirty: false,
            last_settings_save: Instant::now(),
            files: fastframe_file::Writer::new(),
            roster,
            accounts,
            active,
            adding_account: false,
            account_before_adding: None,
            account_menu: false,
            palette,
            custom_themes: theme::Catalog::default(),
            applied_dark: None,
            theme_transition: fastframe_theme::Transition::default(),
            reveal_theme_changes: !cfg!(test),
            zoom_applied: false,
            wallpaper_image: crate::wallpaper::CustomImage::default(),
            scroll_chats_to_top: false,
            stale_images: Vec::new(),
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
            chat_search_open: false,
            info: None,
            info_media: None,
            info_media_pending: false,
            info_media_stale: false,
            info_textures: HashSet::new(),
            info_uris: Vec::new(),
            info_released: Vec::new(),
            info_media_request: 0,
            info_media_failed: false,
            info_members: None,
            info_invite: None,
            info_focus: None,
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
            message_receipts: None,
            receipts_watch: None,
            invite: None,
            unread_divider: None,
            selection: None,
            selection_anchor: None,
            sweep: None,
            dropping: false,
            paste_keys: PasteKeys::default(),
            picker: None,
            picker_anchor: None,
            picker_search: String::new(),
            picker_recent: None,
            picker_focus: false,
            reaction_target: None,
            reaction_anchor: None,
            reaction_beside_menu: false,
            open_message_menu: None,
            reaction_from_list: false,
            #[cfg(any(test, feature = "demo"))]
            open_chat_menu: None,
            #[cfg(any(test, feature = "demo"))]
            open_header_menu: None,
            #[cfg(any(test, feature = "demo"))]
            open_member_menu: None,
            #[cfg(any(test, feature = "demo"))]
            open_selection_delete_menu: false,
            emoji_jump: None,
            pending: Vec::new(),
            pending_view_once: false,
            next_pending_id: 0,
            composer_tools_open: false,
            player: Player::new(waker.clone()),
            video: crate::video::Player::new(waker.clone()),
            video_chat: None,
            video_wanted: None,
            pending_follow: None,
            voice_chat: None,
            voice_clip_chat: None,
            voice_wanted: None,
            recording: None,
            media_hold: None,
            pauses_media: false,
            image_preview: None,
            motion_playing: None,
            sticker_view: None,
            viewer_media: Vec::new(),
            viewer_media_chat: None,
            viewer_media_state: HashMap::new(),
            copy_rows: Default::default(),
            selection_view: Default::default(),
            gif_query: String::new(),
            gif_results: Vec::new(),
            gif_next_offset: None,
            gif_request: 0,
            gif_pending: false,
            gif_error: None,
            sticker_shelf: StickerShelf::default(),
            sticker_search: String::new(),
            sticker_emojis: std::collections::HashMap::new(),
            sticker_pack_name: String::new(),
            sticker_pack_created: None,
            sticker_preview: None,
            sticker_preview_pending: false,
            sticker_draft: None,
            picture_edit: None,
            picture_inspecting: None,
            picture_edit_followed: None,
            picture_applying: Default::default(),
            picture_origins: Default::default(),
            picture_texture: None,
            scrolling: fastframe_scroll::Scrolling::default(),
            scroll_route: ScrollRoute::default(),
            page: Page::Chats,
            dialog: None,
            nav_history: History::default(),
            forward_search: String::new(),
            forward_recipients: Vec::new(),
            forward_reviewing: false,
            forward_limit: crate::model::FORWARD_LIMIT,
            group_name_edit: None,
            new_contact_to_phone: true,
            sidebar_visible: true,
            #[cfg(any(test, feature = "demo"))]
            open_list_menu: None,
            label_name: String::new(),
            label_color: crate::archive::DEFAULT_COLOR.to_owned(),
            label_editing: None,
            label_filter: None,
            unread_kept: HashSet::new(),
            toasts: Vec::new(),
            actions: Vec::new(),
            composer_undo_epoch: 0,
            prefilled: None,
            deferred_account_actions: Vec::new(),
            events_hidden: false,
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
            message_cursor: HashMap::new(),
            cursor_followed: None,
            reveal_cursor: false,
            messages_focused: false,
            focus_messages: false,
            list_focus_deferred: None,
            focus_composer: false,
            focus_search: false,
            settings_search: String::new(),
            focus_settings_search: false,
            quit_requested: false,
            window_focused: false,
            wants_attention: false,
            call: None,
            call_account: None,
            call_surface_until: None,
            call_devices: crate::calls::DeviceList::default(),
            call_devices_open: false,
            call_surface_hidden: false,
            call_fullscreen: false,
            call_notified: None,
            call_rings: None,
            call_tones: Default::default(),
            call_dismissed: None,
            call_cancel: None,
            call_placed: None,
            call_local_frame: None,
            call_remote_frame: None,
            call_local_texture: None,
            call_remote_texture: None,
            call_member_frames: HashMap::new(),
            call_member_textures: HashMap::new(),
            start_with_system: None,
            login_item: Default::default(),
            waker,
            tray: None,
            power: fastframe_power::Power::fixed(Default::default()),
            #[cfg(target_os = "linux")]
            system_appearance: None,
            #[cfg(test)]
            test_tray_shown: None,
            #[cfg(test)]
            test_raised: false,
            tray_lockable,
            window_hidden: false,
            hide_intent: false,
            wants_show: false,
            tray_wait: None,
            wayland: wayland_session(),
            reopen: false,
            control_commands: None,
            notification_opens: Default::default(),
            notifications: Default::default(),
            badge: None,
            app_lock,
            announcer: Default::default(),
            announced_offline: false,
            announced_connecting: false,
        };
        // A hand-edited speed snaps to a supported one, so a speed control
        // always shows the speed that plays.
        app.settings.voice_speed = app.player.set_speed(app.settings.voice_speed);
        // The chip comes back with its account; a label is the window's.
        app.label_filter = app.remembered_label();
        app
    }

    pub fn account(&self) -> &Account {
        &self.accounts[self.active]
    }

    pub fn account_mut(&mut self) -> &mut Account {
        &mut self.accounts[self.active]
    }

    pub fn any_linked(&self) -> bool {
        self.accounts.iter().any(Account::is_linked)
    }

    /// Whether more than one number is (or is being) linked here, so the
    /// window names which one it shows.
    pub fn has_several_accounts(&self) -> bool {
        self.accounts.len() >= 2
    }

    /// Unread chats across every account, for the taskbar and the tray. A
    /// muted account is left out, as a muted chat is.
    pub fn unread_chat_count_everywhere(&self) -> u32 {
        self.accounts
            .iter()
            .filter(|account| account.settings.notifications)
            .map(Account::unread_chat_count)
            .fold(0, u32::saturating_add)
    }

    /// Unread chats in the accounts that are not on screen and not muted, for
    /// the mark on the account switcher.
    pub fn unread_chat_count_elsewhere(&self) -> u32 {
        self.accounts
            .iter()
            .enumerate()
            .filter(|(index, account)| *index != self.active && account.settings.notifications)
            .map(|(_, account)| account.unread_chat_count())
            .fold(0, u32::saturating_add)
    }

    fn save_roster(&mut self) {
        self.roster.active = self.account().id.0.clone();
        self.roster.order = self
            .accounts
            .iter()
            .map(|account| account.id.0.clone())
            .collect();
        if let Err(error) = self.roster.save_to(&self.files, &self.dirs.accounts_file()) {
            log::warn!("could not save accounts.json: {error}");
        }
    }

    /// Text a link put in the composer, and the reader never changed. It is
    /// not their unsent text, so no path that saves the composer may keep it:
    /// the draft they wrote stays theirs.
    fn is_untouched_prefill(&self, chat: &str, text: &str) -> bool {
        self.prefilled
            .as_ref()
            .is_some_and(|(prefilled_chat, prefilled)| prefilled_chat == chat && prefilled == text)
    }

    /// Parks the current account's text, mentions and attachments before switching accounts.
    fn park_composer(&mut self) {
        if let Some(previous) = self.open_chat.clone() {
            self.save_attachment_draft(&previous);
            let draft = std::mem::take(&mut self.composer);
            if self.is_untouched_prefill(&previous, &draft) {
                // A link's text is not the reader's, so the draft they wrote
                // stays exactly as it was.
                self.composer_mentions.clear();
            } else if self.editing.take().is_some() || draft.trim().is_empty() {
                self.drafts.remove(&previous);
                self.draft_mentions.remove(&previous);
                self.composer_mentions.clear();
            } else {
                let mentions = std::mem::take(&mut self.composer_mentions);
                self.drafts.insert(previous.clone(), draft);
                self.draft_mentions.insert(previous.clone(), mentions);
            }
            self.stop_composing(&previous);
            let text = self.drafts.get(&previous).map_or("", String::as_str);
            self.store_draft(&previous, text);
        }
        self.picker = None;
        self.pending.clear();
        self.pending_view_once = false;
        self.forget_unstaged_pictures();
        self.reply_to = None;
        self.editing = None;
        self.recording = None;
        self.emoji_start = None;
        self.mention_start = None;
    }

    /// Restores the active account's remembered chat draft into the window composer.
    fn restore_composer(&mut self) {
        // Whatever the composer held is not a link's text any more.
        self.prefilled = None;
        if let Some(id) = self.open_chat.clone() {
            self.open_draft_priority = self.parked_draft(&id);
            self.composer = self.drafts.remove(&id).unwrap_or_default();
            self.composer_mentions = self.draft_mentions.remove(&id).unwrap_or_default();
            let attachments = self.attachment_drafts.remove(&id).unwrap_or_default();
            self.pending = attachments.pending;
            self.pending_view_once = attachments.view_once;
            self.reply_to = attachments.quoting;
        } else {
            self.composer.clear();
            self.composer_mentions.clear();
            self.pending_view_once = false;
        }
        self.focus_composer = self.open_chat.is_some();
    }

    fn switch_account(&mut self, id: &AccountId) {
        let Some(index) = self.accounts.iter().position(|account| &account.id == id) else {
            return;
        };
        if index == self.active {
            return;
        }
        // An account left before it was linked was never really added.
        let abandoned =
            (self.adding_account && !self.is_linked()).then(|| self.account().id.clone());
        self.park_composer();
        self.clear_account_ui();
        self.active = index;
        self.adding_account = false;
        if let Some(abandoned) = abandoned {
            self.remove_account(abandoned);
        }
        self.restore_composer();
        // Labels belong to their account: show the one this account showed.
        self.label_filter = self.remembered_label();
        // The Calls tab stays open on the other account's calls.
        if self.page == Page::Calls {
            self.backend.send(Command::LoadCallLog);
        } else {
            self.page = Page::Chats;
        }
        self.save_roster();
        self.wallpaper_image.reload();
        self.report_presence();
        // Chats belong to the account that owns them; another account's chat
        // cannot be restored into this one.
        self.nav_history.clear();
    }

    /// Drops App-owned pointers into the previous account's chats and media.
    fn clear_account_ui(&mut self) {
        self.invalidate_composer_undo();
        self.gif_request = self.gif_request.wrapping_add(1);
        self.gif_query.clear();
        self.gif_results.clear();
        self.gif_next_offset = None;
        self.gif_pending = false;
        self.gif_error = None;
        self.forward_recipients.clear();
        self.forward_reviewing = false;
        // The locked folder and the picker show the previous account's chats
        // and stickers.
        if self.locked_folder {
            self.close_locked_folder();
        }
        self.sticker_shelf = StickerShelf::default();
        self.sticker_emojis.clear();
        self.sticker_pack_created = None;
        self.account_menu = false;
        // The info panel shows the previous account's chat and its listing.
        self.close_info();
        self.chat_search_open = false;
        self.chat_search.clear();
        self.chat_search_hits.clear();
        self.chat_search_selected = None;
        self.chat_search_pending = false;
        self.selection = None;
        self.selection_anchor = None;
        self.sweep = None;
        self.reply_to = None;
        self.editing = None;
        self.reaction_target = None;
        self.reaction_anchor = None;
        self.picker = None;
        self.pending.clear();
        self.pending_view_once = false;
        self.forget_unstaged_pictures();
        self.invite = None;
        self.message_receipts = None;
        self.receipts_watch = None;
        self.unread_divider = None;
        self.scroll_anchor = None;
        self.dialog = None;
        self.player.stop();
        self.video.stop();
        self.video_chat = None;
        self.video_wanted = None;
        self.motion_playing = None;
        self.voice_chat = None;
        self.voice_clip_chat = None;
        self.voice_wanted = None;
        self.recording = None;
        self.image_preview = None;
        self.viewer_media.clear();
        self.viewer_media_chat = None;
        self.viewer_media_state.clear();
        // The offline episode announced was the previous account's link.
        self.announced_offline = false;
        self.announced_connecting = false;
        // Messages and typing spoken for the previous account are not
        // left readable in the tree.
        self.announcer.forget_private();
        // A request for the message list to take focus was for the
        // previous account's chat.
        self.focus_messages = false;
        // So was a list focus waiting for a visible pass.
        self.list_focus_deferred = None;
        // Each chat's current message was the previous account's: accounts
        // can share a chat, and a hidden account's deletions do not move it.
        // The newest message the keys followed goes with it.
        self.message_cursor.clear();
        self.cursor_followed = None;
        // So was a message menu asked for and not drawn yet.
        self.open_message_menu = None;
        // A reaction picker the message list opened belongs to the chat
        // left behind.
        self.reaction_from_list = false;
        // So does a replied message still loading, as on a chat change.
        self.pending_follow = None;
    }

    fn add_account(&mut self) {
        let id = self.roster.allocate();
        let account = if self
            .accounts
            .iter()
            .all(|account| account.backend.is_offline())
        {
            match Account::detached(&self.dirs, id, crate::settings::AccountSettings::default()) {
                Ok((account, _)) => account,
                Err(error) => {
                    self.toast_error(format!("Could not add an account: {error}"));
                    return;
                }
            }
        } else {
            match Account::spawn(&self.dirs, id, &self.settings, &self.waker) {
                Ok(account) => account,
                Err(error) => {
                    self.toast_error(format!("Could not add an account: {error}"));
                    return;
                }
            }
        };
        account.backend.send(Command::SetDownloadFolder(
            self.settings.download_folder.clone(),
        ));
        account.backend.send(Command::SetKeepChatsArchived(
            self.settings.keep_chats_archived,
        ));
        self.account_before_adding = Some(self.account().id.clone());
        self.park_composer();
        self.clear_account_ui();
        self.accounts.push(account);
        self.active = self.accounts.len() - 1;
        self.adding_account = true;
        self.page = Page::Chats;
        self.restore_composer();
        self.save_roster();
        self.report_presence();
        self.nav_history.clear();
    }

    /// Unlinks an account and, once its backend has stopped, deletes its
    /// folders. The last account is only unlinked: the window always shows
    /// one, even if it is waiting to be linked.
    fn remove_account(&mut self, id: AccountId) {
        let Some(index) = self.accounts.iter().position(|account| account.id == id) else {
            return;
        };
        if self.accounts.len() <= 1 {
            self.accounts[index].backend.send(Command::Unlink);
            return;
        }
        self.accounts[index].backend.send(Command::RemoveAccount);
    }

    fn finish_removed(&mut self, index: usize) {
        if self.accounts.len() <= 1 {
            // Another removal got there first. Keep this one listed and
            // linkable instead of leaving the window on a stopped backend.
            self.accounts[index].backend.send(Command::Unlink);
            return;
        }
        let removing_active = index == self.active;
        if removing_active {
            self.park_composer();
            self.clear_account_ui();
        }
        self.accounts[index].backend.shutdown();
        // The backend ends its call on the way out; the surface goes with it.
        let removed = self.accounts[index].id.clone();
        self.forget_call_of(&removed);
        let dirs = self.accounts[index].dirs.clone();
        // The key outlives the archive only by mistake: forget it once the
        // archive is gone, never before.
        let key = crate::archive::archive_key_identity(&dirs.archive_db()).ok();
        let state_error = std::fs::remove_dir_all(&dirs.state).err();
        let cache_error = std::fs::remove_dir_all(&dirs.cache)
            .err()
            .filter(|error| error.kind() != std::io::ErrorKind::NotFound);
        if let Some(identity) = key
            && !dirs.archive_db().exists()
            && cfg!(not(test))
            && let Err(error) = crate::archive::forget_archive_key(&identity)
        {
            log::warn!("could not delete a removed account's archive key: {error:#}");
        }
        self.accounts.remove(index);
        if index < self.active {
            self.active -= 1;
        } else if self.active >= self.accounts.len() {
            self.active = self.accounts.len() - 1;
        }
        if removing_active {
            self.adding_account = false;
            self.restore_composer();
            self.page = Page::Chats;
            self.wallpaper_image.reload();
            self.nav_history.clear();
        }
        self.save_roster();
        self.report_presence();
        if let Some(error) = state_error.or(cache_error) {
            let message =
                crate::i18n::gettext(self.locale, "Could not delete the removed account's files");
            self.toast_error(format!("{message}: {error}"));
        }
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
        // The window is gone with the full screen it was in; a new one starts
        // at its normal size.
        self.call_fullscreen = false;
    }

    /// What the conversation shows behind its bubbles, for the chat and the
    /// wallpaper preview alike.
    pub fn wallpaper(&self) -> crate::wallpaper::Look {
        let mut color = self.settings.wallpaper_background(&self.palette);
        if self.palette.vibrant() {
            color = self.palette.vibrant_chat(color);
        }
        crate::wallpaper::Look {
            color,
            // Messages draws its conversations on a plain background.
            doodles: self.settings.show_wallpaper
                && self.palette.bubbles != crate::theme::BubbleStyle::Messages,
            image: self
                .account()
                .settings
                .wallpaper_image
                .is_some()
                .then(|| self.wallpaper_image.ready())
                .flatten(),
        }
    }

    /// Whether window close keeps the app in the tray.
    pub fn hides_to_tray(&self) -> bool {
        self.tray_shown() && self.settings.keep_running_in_background
    }

    /// Asks the compositor to bring the existing window forward, which niri
    /// can do on Wayland; see [`fastframe_shell::window::raise_on_compositor`].
    fn raise_on_compositor(&self) -> bool {
        #[cfg(test)]
        return self.test_raised;
        #[cfg(not(test))]
        fastframe_shell::window::raise_on_compositor()
    }

    /// Whether a panel shows the tray item now, so a hidden window can be
    /// brought back from it. On Linux the item exists before a panel shows
    /// it (ZapFast started at login before the panel), and on a desktop
    /// without one it never is.
    fn tray_shown(&self) -> bool {
        #[cfg(test)]
        if let Some(shown) = self.test_tray_shown {
            return shown;
        }
        self.tray
            .as_ref()
            .is_some_and(fastframe_tray::Tray::is_shown)
    }

    /// Whether there is a tray item at all, shown by a panel or not.
    fn has_tray(&self) -> bool {
        #[cfg(test)]
        if self.test_tray_shown.is_some() {
            return true;
        }
        self.tray.is_some()
    }

    /// Ends a hidden start's wait for a panel: a panel showing the tray item
    /// keeps the window hidden, and none by the deadline shows it, since
    /// nothing else could bring it back.
    fn check_tray_wait(&mut self, now: Instant) {
        let Some(deadline) = self.tray_wait else {
            return;
        };
        if self.tray_shown() {
            self.tray_wait = None;
        } else if now >= deadline {
            log::info!("no panel showed the tray item; showing the window");
            self.tray_wait = None;
            self.wants_show = true;
        }
    }

    fn handle_tray(&mut self) {
        self.sync_tray();
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

    /// Offers "Lock ZapFast" in the tray exactly while an app lock password
    /// is set, so setting or removing one changes the menu at once.
    fn sync_tray(&mut self) {
        let lockable = self.settings.app_lock_hash.is_some();
        if lockable == self.tray_lockable {
            return;
        }
        self.tray_lockable = lockable;
        if let Some(tray) = &mut self.tray {
            tray.set_visible(TRAY_LOCK, lockable);
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
                ControlCommand::Quit => self.actions.push(Action::Quit),
                ControlCommand::Ping => {}
                ControlCommand::OpenInvite(code) => {
                    self.actions.push(Action::ShowWindow);
                    self.actions
                        .push(Action::OpenUrl(format!("https://chat.whatsapp.com/{code}")));
                }
                ControlCommand::OpenChat(request) => {
                    // The chat is only visible in a window, so a run
                    // started into the tray shows one first.
                    self.actions.push(Action::ShowWindow);
                    self.actions.push(Action::StartChat {
                        dismiss_dialog: true,
                        name: request.display_name(),
                        id: request.chat.clone(),
                    });
                    if let Some(text) = request.text {
                        self.actions.push(Action::PrefillComposer {
                            chat: request.chat.clone(),
                            text,
                        });
                    }
                }
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
            // Behind the app lock nothing changes yet: the message, with its
            // account, opens once unlocked.
            if self.app_lock.is_locked() {
                self.app_lock
                    .defer(crate::app_lock::Deferred::Message(target));
                self.actions.push(Action::ShowWindow);
                continue;
            }
            self.switch_account(&target.account);
            match target.message {
                Some(message) => self.actions.push(Action::OpenMessage {
                    chat: target.chat,
                    message,
                }),
                // A call's notification brings the window up, where the call
                // is waiting.
                None => self.actions.push(Action::ReturnToCall),
            }
            self.actions.push(Action::ShowWindow);
        }
    }

    fn clear_chat_notifications(&mut self, chat: &str) {
        let account = self.account().id.clone();
        self.notifications.clear(&account, chat);
    }

    /// The chat was looked at, so its next message may notify at once.
    fn reset_notification_throttle(&mut self, chat: &str) {
        let account = self.account().id.clone();
        self.notifications.reset_throttle(&account, chat);
    }

    /// Sends a desktop notification for an unseen incoming message.
    fn maybe_notify(&mut self, chat_id: &str, message: &Message) {
        if !self.account().settings.notifications || self.power.conditions().display_off {
            return;
        }
        let Some(chat) = self.chat(chat_id) else {
            return;
        };
        let now = crate::util::now();
        if !notification_eligible(chat, now, message.timestamp) {
            return;
        }
        let reading = !self.events_hidden
            && !self.window_hidden
            && self.window_focused
            && self.chat_view()
            && self.open_chat.as_deref() == Some(chat_id);
        if reading {
            return;
        }
        // A limited chat notifies once, then waits; opening it ends the wait.
        // A mention of us or a reply to us always notifies and leaves the wait alone.
        let for_us = chat.is_group() && self.addresses_us(message);
        let limited = !for_us
            && self
                .account()
                .settings
                .limit_notifications
                .applies(chat.is_group());
        let account = self.account().id.clone();
        if !self
            .notifications
            .allow(&account, chat_id, std::time::Instant::now(), limited)
        {
            return;
        }
        let Some(chat) = self.chat(chat_id) else {
            return;
        };
        if self.app_lock.is_locked() || !self.settings.notification_content {
            self.notify_without_content(chat_id, chat.is_group(), &message.id, limited);
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
        let sound = notification_sound(&self.settings, chat_sound, is_group, for_us);
        self.notifications.show(
            title,
            body,
            picture,
            sound,
            crate::notify::NotificationTarget {
                account: self.account().id.clone(),
                chat: chat_id.to_owned(),
                message: Some(message.id.clone()),
            },
            std::sync::Arc::clone(&self.notification_opens),
            move || waker.wake(),
            limited,
        );
        self.wants_attention = true;
    }

    /// Announces a message while the app lock is on, or with notification
    /// content turned off: "New message" from ZapFast, without the chat, the
    /// sender, the text or a picture, so the desktop shows nothing the lock
    /// screen hides. A chat's own sound and the mention sound would tell who
    /// wrote, so only the message sound plays, still silent for groups when
    /// group sounds are off. The click target stays inside ZapFast: it opens
    /// the message, once unlocked behind the lock.
    fn notify_without_content(
        &mut self,
        chat_id: &str,
        is_group: bool,
        message: &str,
        limited: bool,
    ) {
        self.wants_attention = true;
        let (title, body) = crate::notify::locked_lines(self.locale);
        let sound = notification_sound(&self.settings, None, is_group, false);
        let waker = self.waker.clone();
        self.notifications.show(
            title,
            body,
            None,
            sound,
            crate::notify::NotificationTarget {
                account: self.account().id.clone(),
                chat: chat_id.to_owned(),
                message: Some(message.to_owned()),
            },
            std::sync::Arc::clone(&self.notification_opens),
            move || waker.wake(),
            limited,
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
        let mentioned = message
            .mentions
            .iter()
            .any(|mention| crate::mentions::names_us(mention, &ours));
        mentioned
            || message
                .quoted
                .as_ref()
                .is_some_and(|quoted| is_us(&quoted.sender))
    }

    /// Initializes a newly created window.
    pub fn attach(&mut self, ctx: &egui::Context) {
        // A window is here, so a hidden start's wait for a panel is over.
        self.tray_wait = None;
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
        // Colour emoji in labels, menus, tooltips and text fields; message
        // bodies paint their own over placeholders, which it leaves alone.
        ctx.add_plugin(crate::emoji::plugin());
        crate::theme::set_font(ctx, self.settings.font);
        crate::theme::install(ctx);
        // Zoom stays in the settings, so egui must not change it behind the
        // app's back: the shortcuts below go through `Action::ZoomBy`.
        ctx.options_mut(|options| options.zoom_with_keyboard = false);
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
        self.theme_transition = fastframe_theme::Transition::default();
        self.zoom_applied = false;
        self.window_hidden = false;
        self.paste_keys = PasteKeys::default();
        self.hide_intent = false;
        self.wants_show = false;
        self.reopen = false;
        // A new window starts with nothing focused: a message list that had
        // focus in the last one does not hold it back from the composer.
        self.messages_focused = false;
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

    /// The speed voice messages play at in `chat`: its own, else the one in
    /// Settings.
    pub fn voice_speed_in(&self, chat: Option<&str>) -> f32 {
        chat.and_then(|chat| self.chat(chat))
            .and_then(|chat| chat.voice_speed)
            .unwrap_or(self.settings.voice_speed)
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
        // The recorded places belong to the account on screen, and chat ids
        // repeat across accounts.
        if !self.events_hidden {
            self.nav_history.forget(id);
        }
        self.drafts.remove(id);
        self.draft_mentions.remove(id);
        self.attachment_drafts.remove(id);
        self.typing.remove(id);
        self.unread_kept.remove(id);
        if self.scroll_chat_into_view.as_deref() == Some(id) {
            self.scroll_chat_into_view = None;
        }
        if self.settings.last_chat.as_deref() == Some(id) {
            self.settings.last_chat = None;
        }
        if self.account().settings.last_chat.as_deref() == Some(id) {
            self.account_mut().settings.last_chat = None;
            self.account_mut().mark_settings_dirty();
        }
    }

    /// Takes everything on screen away from a chat the user can no longer
    /// reach, whether it was locked or deleted: its notifications, search
    /// hits and dialogs, and, when it is open, the conversation with its
    /// composer, recording and playback. Keeping a draft is up to the caller.
    fn leave_chat(&mut self, id: &str) {
        if !self.events_hidden && self.open_chat.as_deref() == Some(id) {
            self.invalidate_composer_undo();
        }
        self.clear_chat_notifications(id);
        self.search_hits.retain(|message| message.chat != id);
        // A locked or deleted chat keeps none of its pinned words on screen.
        self.pins.remove(id);
        self.chat_pins.remove(id);
        if self.events_hidden {
            if self.open_chat.as_deref() == Some(id) {
                self.open_chat = None;
            }
            return;
        }
        if self.info.as_ref().is_some_and(|info| info.chat == id) {
            self.close_info();
        }
        if self.image_preview.as_ref().map(PreviewState::chat) == Some(id) {
            self.image_preview = None;
        }
        if matches!(
            &self.dialog,
            Some(
                Dialog::CreatePoll(chat)
                    | Dialog::SendLocation(chat)
                    | Dialog::ConfirmDeleteChat(chat)
                    | Dialog::ConfirmClearChat(chat)
            ) if chat == id
        ) || matches!(
            &self.dialog,
            Some(Dialog::Forward { chat, .. } | Dialog::ConfirmDeleteMessage { chat, .. } | Dialog::ConfirmDeleteSelection { chat, .. })
                if chat == id
        ) {
            self.dialog = None;
            self.poll_creating = false;
        }
        if self.open_chat.as_deref() == Some(id) {
            self.stop_composing(id);
            self.set_open_chat(None);
            self.composer.clear();
            self.composer_mentions.clear();
            self.pending.clear();
            self.pending_view_once = false;
            self.forget_unstaged_pictures();
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

    /// Whether the main pane can show the open chat: on the chat list's
    /// page, and on the Calls tab beside its history.
    pub fn chat_view(&self) -> bool {
        matches!(self.page, Page::Chats | Page::Calls)
    }

    /// Whether the open chat is the one picked from the Calls tab's history.
    pub fn call_picked(&self) -> bool {
        self.picked_call
            .as_ref()
            .is_some_and(|(chat, _)| self.open_chat.as_deref() == Some(chat.as_str()))
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
            // A deleted label is not shown at the next start either.
            self.remember_list_filter();
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
        self.remember_list_filter();
    }

    /// Keeps the chip or label the chat list shows for the next start.
    fn remember_list_filter(&mut self) {
        let key = crate::model::ListFilter::key(self.chat_filter, self.label_filter.as_deref());
        if self.account().settings.list_filter != key {
            self.account_mut().settings.list_filter = key;
            self.account_mut().mark_settings_dirty();
        }
    }

    /// The label the active account's chat list showed when it was last
    /// used, which waits for the account's labels to arrive.
    fn remembered_label(&self) -> Option<String> {
        match self
            .account()
            .settings
            .list_filter
            .as_deref()
            .and_then(crate::model::ListFilter::parse)
        {
            Some(crate::model::ListFilter::Label(label)) => Some(label),
            _ => None,
        }
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
    /// The short name WhatsApp shows where space is short (a group's member
    /// line, the sender before a group's last message) for `id`, whose full
    /// display name is `name`: the first name saved with the contact, whole,
    /// as it can hold several words. Without one (a profile name, or a
    /// contact the phone has not sent since first names were kept) the first
    /// word of the name; a phone number and our own "You" stay whole.
    pub fn short_name<'a>(&'a self, id: &str, name: &'a str) -> &'a str {
        if self.me.as_deref() == Some(id) || name.starts_with('+') {
            return name;
        }
        if let Some(first) = self.contacts.get(id).and_then(Contact::first_name) {
            return first;
        }
        name.split_whitespace().next().unwrap_or(name)
    }

    pub fn display_name_or(&self, id: &str, hint: Option<&str>) -> String {
        if self.me.as_deref() == Some(id) {
            return "You".to_owned();
        }
        self.person_name(id, hint)
    }

    /// Resolves a mention name without replacing our own name with "You".
    pub fn mention_name(&self, id: &str) -> String {
        if id == crate::mentions::ALL_ID {
            return crate::mentions::ALL_USER.to_owned();
        }
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
        self.person_name_of(&chat.id, None, Some(chat))
    }

    /// The title of the chat `id`, as the chat list writes it, for places that
    /// hold only the id (a starred message). A chat not loaded falls back to
    /// the person's name.
    pub fn chat_title_of(&self, id: &str) -> String {
        match self.chat(id) {
            Some(chat) => self.chat_title(chat),
            None => self.display_name(id),
        }
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
        .any(|text| crate::util::search_matches(&crate::util::search_key(text), needle))
            || crate::model::phone_of(me).is_some_and(|phone| phone.contains(needle))
    }

    fn person_name(&self, id: &str, hint: Option<&str>) -> String {
        self.person_name_of(id, hint, None)
    }

    /// [`Self::person_name`] for a chat already at hand, which spares the
    /// lookup by id: the chat list asks for every chat's title on every frame
    /// while it searches, and that lookup walks the whole list.
    fn person_name_of(&self, id: &str, hint: Option<&str>, chat: Option<&Chat>) -> String {
        let contact = self.contacts.get(id);
        let present = |name: Option<&str>| name.filter(|name| !name.is_empty()).map(str::to_owned);
        let saved = present(contact.and_then(|contact| contact.full_name.as_deref()));
        let called = present(contact.and_then(|contact| contact.push_name.as_deref()))
            .or_else(|| present(hint));
        // Saved names first, as WhatsApp does; a profile name wears a tilde.
        if let Some(name) = saved.or_else(|| called.map(|name| format!("~{name}"))) {
            return name;
        }
        // A name of digits alone is no name. When the id carries the number,
        // neither is one as `util::phone` grouped it when the chat was filed:
        // that is formatted afresh from the id.
        let phone = crate::model::phone_of(id);
        let number = |c: char| {
            c.is_ascii_digit() || (phone.is_some() && matches!(c, '+' | ' ' | '(' | ')' | '-'))
        };
        if let Some(chat) = chat.or_else(|| self.chat(id))
            && !chat.name.is_empty()
            && !chat.name.chars().all(number)
        {
            return chat.name.clone();
        }
        match phone {
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
            _ => self.preview_line(&message.summary_in(self.locale), message),
        }
    }

    /// Plain text taken from `message` (a caption summary, a matched line)
    /// with its mentions named as its body names them, including privacy-id
    /// mentions the contact list cannot resolve on its own; any other
    /// `@number` token of a known person is named too.
    pub fn preview_line(&self, text: &str, message: &Message) -> String {
        self.resolve_mention_tokens(&crate::markup::plain(text, &self.mention_list(message)))
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
    ///
    /// Everyone is offered first when this group allows it and the query is a
    /// prefix of `@all` or of the language's own word.
    pub fn mention_candidates(&self, chat: &Chat, query: &str) -> Vec<(String, String)> {
        if !chat.is_group() {
            return Vec::new();
        }
        let needle = query.trim().to_lowercase();
        let digits: String = query.chars().filter(char::is_ascii_digit).collect();
        let mut found: Vec<(String, String)> = self
            .participant_list(chat)
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
            .collect();
        let alias = crate::mentions::everyone_alias(self.locale);
        if chat.can_mention_everyone() && crate::mentions::everyone_query_matches(query, &alias) {
            found.insert(
                0,
                (
                    crate::mentions::ALL_ID.to_owned(),
                    crate::mentions::ALL_USER.to_owned(),
                ),
            );
        }
        found
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
                names.push(self.short_name(id, name.trim_start_matches('~')).to_owned());
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

    /// A person never written to, left out of the list as on the phone. One
    /// the reader kept (pinned, a favorite, labeled, unread, locked, or with
    /// an unsent draft) stays listed.
    fn unlisted_stranger(&self, chat: &Chat) -> bool {
        chat.kind == crate::model::ChatKind::Direct
            && chat.never_messaged()
            && !chat.pinned
            && !chat.favorite
            && !chat.looks_unread()
            && chat.labels.is_empty()
            && !chat.locked
            && !self.has_draft(&chat.id)
    }

    /// Visible chats filtered by search, archive state, and the chat filter,
    /// with pins first, then parked drafts. The Favorites chip retains the
    /// phone's order below pins instead of promoting drafts.
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
            // Searching still finds people never written to, and one being
            // opened stays.
            .filter(|chat| {
                !needle.is_empty()
                    || self.open_chat.as_deref() == Some(chat.id.as_str())
                    || !self.unlisted_stranger(chat)
            })
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
                    || self.text_matches(&self.chat_title(chat), &needle)
                    || chat.phone().is_some_and(|phone| phone.contains(&needle))
                    || chat
                        .last
                        .as_ref()
                        .is_some_and(|last| self.text_matches(&last.summary, &needle))
            })
            .collect();
        // The Favorites chip keeps the phone's order below the pinned chats.
        let favorites_order = self.favorites_order();
        chats.sort_by(|a, b| {
            b.pinned.cmp(&a.pinned).then_with(|| {
                if a.pinned && b.pinned {
                    b.pinned_at.cmp(&a.pinned_at).then(a.id.cmp(&b.id))
                } else if favorites_order {
                    a.favorite_position
                        .cmp(&b.favorite_position)
                        .then(a.id.cmp(&b.id))
                } else {
                    self.draft_priority(&b.id)
                        .cmp(&self.draft_priority(&a.id))
                        .then_with(|| b.last_activity.cmp(&a.last_activity))
                        .then(a.id.cmp(&b.id))
                }
            })
        });
        chats
    }

    /// Whether the chat list keeps the phone's favorites order instead of the
    /// latest activity, so a sent message does not move its chat up.
    fn favorites_order(&self) -> bool {
        self.label_filter.is_none()
            && self.chat_filter == ChatFilter::Favorites
            && !self.show_archived
            && crate::util::search_key(self.search.trim()).is_empty()
            && !self.locked_folder_open()
    }

    /// Matching individual contacts without an existing chat, sorted by name.
    pub fn matching_contacts(&self) -> Vec<&Contact> {
        let needle = crate::util::search_key(self.search.trim());
        if needle.is_empty() {
            return Vec::new();
        }
        let talked_to: std::collections::HashSet<&str> =
            self.chats.iter().map(|chat| chat.id.as_str()).collect();
        let mut contacts: Vec<&Contact> = self
            .contacts
            .values()
            .filter(|contact| crate::model::phone_of(&contact.id).is_some())
            .filter(|contact| self.me.as_deref() != Some(contact.id.as_str()))
            // Looking each contact up among the chats one by one made every
            // keystroke cost contacts × chats.
            .filter(|contact| !talked_to.contains(contact.id.as_str()))
            .filter(|contact| {
                contact
                    .display_name()
                    .is_some_and(|name| self.text_matches(name, &needle))
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
            .filter(|chat| chat.archived && !chat.locked && !self.unlisted_stranger(chat))
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

    /// The taskbar count: unarchived, unmuted, unlocked chats that look
    /// unread. WhatsApp counts chats here, not the messages inside them.
    pub fn unread_chat_count(&self) -> u32 {
        self.account().unread_chat_count()
    }

    /// Returns or requests a cached profile picture.
    fn cached_avatar(&self, id: &str) -> Option<PathBuf> {
        let path = self.account().dirs.avatar_file(id, false);
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
        self.account_mut().avatar(id)
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

    /// Whether a closed chat has unsent text or attachments.
    fn parked_draft(&self, chat: &str) -> bool {
        self.drafts
            .get(chat)
            .is_some_and(|text| !text.trim().is_empty())
            || self
                .attachment_drafts
                .get(chat)
                .is_some_and(|draft| !draft.pending.is_empty())
    }

    /// Opening a draft keeps its priority until the reader leaves. New
    /// typing is promoted only after leaving, so editing cannot move a row.
    fn draft_priority(&self, chat: &str) -> bool {
        if self.open_chat.as_deref() == Some(chat) {
            self.open_draft_priority
        } else {
            self.parked_draft(chat)
        }
    }

    /// Whether a chat holds unsent text or attachments. Editing a sent
    /// message is not a draft.
    fn has_draft(&self, chat: &str) -> bool {
        if self.open_chat.as_deref() == Some(chat) {
            (self.editing.is_none() && !self.composer.trim().is_empty()) || !self.pending.is_empty()
        } else {
            self.parked_draft(chat)
        }
    }

    /// A chat's unsent text on one line, or its first staged attachment, for
    /// its row in the chat list. The open chat's draft is in the composer,
    /// where the reader sees it.
    pub fn draft_preview(&self, chat: &str) -> Option<String> {
        if self.open_chat.as_deref() == Some(chat) {
            return None;
        }
        let line = self
            .drafts
            .get(chat)
            .map(|draft| draft.split_whitespace().collect::<Vec<_>>().join(" "))
            .unwrap_or_default();
        if !line.is_empty() {
            return Some(line);
        }
        let attachment = &self.attachment_drafts.get(chat)?.pending.first()?.item;
        Some(match attachment {
            Pending::File(path) | Pending::Document(path) => path.file_name().map_or_else(
                || "📎".to_owned(),
                |name| {
                    name.to_string_lossy()
                        .split_whitespace()
                        .collect::<Vec<_>>()
                        .join(" ")
                },
            ),
            Pending::Picture { .. } => crate::i18n::gettext(self.locale, "Image").into_owned(),
        })
    }

    /// Active typers and voice recorders in a chat, with their display names.
    pub fn typing_in(&self, chat: &str) -> Vec<(String, String, crate::model::ChatActivity)> {
        self.typing
            .get(chat)
            .map(|typers| {
                typers
                    .iter()
                    .map(|(sender, activity, _)| {
                        (sender.clone(), self.display_name(sender), *activity)
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn handle_events(&mut self) {
        let mut removed = Vec::new();
        let mut deferred = Vec::new();
        let real_active = self.active;
        for index in 0..self.accounts.len() {
            let events = self.accounts[index].backend.poll();
            if events.is_empty() {
                continue;
            }
            let origin = self.accounts[index].id.clone();
            let live = index == real_active;
            let pending_before = self.actions.len();
            self.active = index;
            for event in events {
                if matches!(event, Event::AccountRemoved) {
                    removed.push(index);
                    continue;
                }
                self.events_hidden = !live;
                self.apply_backend_event(event, live);
                self.events_hidden = false;
            }
            let queued: Vec<_> = self.actions.drain(pending_before..).collect();
            deferred.extend(queued.into_iter().map(|action| (origin.clone(), action)));
            self.active = real_active;
        }
        self.deferred_account_actions.extend(deferred);
        for index in removed.into_iter().rev() {
            self.finish_removed(index);
        }
    }

    fn apply_backend_event(&mut self, event: Event, live: bool) {
        match event {
            Event::StorageStats {
                stats,
                token,
                cleared,
            } => {
                if cleared {
                    // The files are gone whichever measurement this answers:
                    // what is on screen downloads again rather than drawing
                    // as broken.
                    let dir = self.account().dirs.media_cache_dir();
                    for conversation in self.conversations.values_mut() {
                        for message in &mut conversation.messages {
                            message.content.forget_files_in(&dir);
                        }
                    }
                    if live {
                        let done = crate::i18n::gettext(self.locale, "Media cache cleared");
                        self.toast(done.into_owned());
                    }
                }
                if token == self.storage_token {
                    if cleared {
                        self.storage_clearing = false;
                    }
                    // A failed measurement leaves the last numbers in place.
                    if stats.is_some() {
                        self.storage_stats = stats;
                    }
                }
            }
            Event::Link(status) => self.handle_link(status, live),
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
            Event::Call(update) => self.handle_call_update(*update),
            Event::PhoneOnlyCall(update) => self.handle_phone_only_call(*update),
            Event::CallDevices(devices) => self.call_devices = *devices,
            Event::CallVideo {
                local,
                remote,
                from,
            } => self.handle_call_video(local, remote, from),
            Event::CallLog(entries) => {
                self.call_log = entries;
                self.call_log_loaded = true;
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
                    if live
                        && self.open_chat.as_deref() == Some(chat.as_str())
                        && self.editing.is_none()
                        && self.composer.is_empty()
                    {
                        // Startup restoration may establish priority, but a
                        // late answer must not move a chat where the reader
                        // has already started composing with attachments.
                        if self.pending.is_empty() {
                            self.open_draft_priority = !text.trim().is_empty();
                        }
                        self.composer = text;
                    } else {
                        self.drafts.entry(chat).or_insert(text);
                    }
                }
            }
            Event::Stars { chat, ids } => {
                self.stars.insert(chat, ids.into_iter().collect());
            }
            Event::StarChanged {
                chat,
                message,
                starred,
                local,
            } => {
                let ids = self.stars.entry(chat).or_default();
                if starred {
                    ids.insert(message);
                } else {
                    ids.remove(&message);
                }
                // Only a click here that WhatsApp confirmed says so. Stars
                // synced from the phone arrive in bursts at link time and
                // would stack a toast each.
                if local && live {
                    if starred {
                        self.toast(crate::i18n::gettext(self.locale, "Starred"));
                    } else {
                        self.toast(crate::i18n::gettext(self.locale, "Star removed"));
                    }
                }
                // The open list would otherwise show the old state.
                if self.show_starred {
                    self.backend.send(Command::LoadStarred);
                }
            }
            Event::StarredList(list) => self.starred = list,
            Event::Pins { chat, items } => {
                // A pin carries the message's own words, so a locked chat
                // keeps them off the window until its folder is open, the
                // way its messages do.
                if self.locked_folder_open() || !self.chat(&chat).is_some_and(|known| known.locked)
                {
                    self.pins.insert(
                        chat.clone(),
                        items.iter().map(|item| item.id.clone()).collect(),
                    );
                    self.chat_pins.insert(chat, items);
                }
            }
            Event::PinChanged {
                chat,
                message,
                pinned,
            } => {
                let ids = self.pins.entry(chat.clone()).or_default();
                if pinned {
                    ids.insert(message);
                } else {
                    ids.remove(&message);
                }
                if live {
                    // One literal per branch, so both reach the catalogs.
                    let label = if pinned {
                        crate::i18n::gettext(self.locale, "Pinned for 7 days")
                    } else {
                        crate::i18n::gettext(self.locale, "Pin removed")
                    };
                    self.toast(label);
                }
            }
            Event::Chats(chats) => {
                for chat in &chats {
                    if chat.unread == 0 {
                        self.clear_chat_notifications(&chat.id);
                    }
                }
                self.chats = chats;
                if live {
                    self.info_members = None;
                    self.drop_locked_panel();
                    // A chat the phone unlocked can be listed now.
                    self.request_info_media_if_unloaded();
                }
                if !self.events_hidden
                    && self
                        .image_preview
                        .as_ref()
                        .map(PreviewState::chat)
                        .is_some_and(|id| {
                            self.chat(id)
                                .is_none_or(|chat| chat.locked && !self.locked_folder_open())
                        })
                {
                    self.image_preview = None;
                }
                if let Some(open) = self.open_chat.clone() {
                    if self.chat(&open).is_none_or(|chat| chat.locked) {
                        if live {
                            self.invalidate_composer_undo();
                        }
                        self.open_chat = None;
                    } else {
                        // Show archived messages immediately, including offline.
                        self.ensure_loaded(&open);
                    }
                }
            }
            Event::ChatMedia { chat, items, .. } => {
                // An album for a chat the viewer has already left, or from an
                // account that is not on screen, arrives too late to matter.
                if live && self.viewer_media_chat.as_deref() == Some(chat.as_str()) {
                    self.viewer_media = items;
                }
            }
            Event::ChatUpdated(chat) => {
                self.handle_chat_updated(*chat, live);
                if live {
                    self.info_members = None;
                    // A chat unlocked alone opens the way for its waiting panel.
                    self.request_info_media_if_unloaded();
                }
            }
            Event::Messages {
                chat,
                messages,
                older,
                complete,
            } => {
                // A call filed while the Calls page is open joins its list.
                if live
                    && self.page == Page::Calls
                    && messages
                        .iter()
                        .any(|message| matches!(message.content, Content::Call { .. }))
                {
                    self.backend.send(Command::LoadCallLog);
                }
                let conversation = self.conversations.entry(chat.clone()).or_default();
                let was_empty = conversation.messages.is_empty();
                let messages_empty = messages.is_empty();
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
                // A message that arrives while the viewer is open joins its album.
                if !older {
                    self.refresh_viewer_album(&chat);
                }
                // A replied message the keys wait for is in the chat on
                // screen; another account's chat of the same id is not it.
                if live && older {
                    self.settle_pending_follow(&chat, messages_empty, complete);
                }
                if live && self.open_chat.as_deref() == Some(chat.as_str()) {
                    if !older && (self.at_bottom || was_empty) {
                        self.scroll_to_bottom = true;
                    }
                    if bare {
                        self.fetch_older(&chat, false);
                    }
                    // After the first page, load toward a pending search anchor once.
                    if !older && let Some(anchor) = self.scroll_anchor.clone() {
                        let load = self.conversations.get_mut(&chat).and_then(|conversation| {
                            if conversation.message(&anchor).is_none()
                                && !conversation.loading_older
                            {
                                conversation.messages.first().map(|oldest| {
                                    conversation.loading_older = true;
                                    (oldest.timestamp, oldest.id.clone())
                                })
                            } else {
                                None
                            }
                        });
                        if let Some(before) = load {
                            self.backend.send(Command::LoadUntil {
                                chat,
                                id: anchor,
                                before,
                            });
                        }
                    }
                }
            }
            Event::ChatShared { request, result } => {
                // Only the answer to the request being waited for counts:
                // an earlier request's, or one for a chat the panel no
                // longer shows, arrives too late to matter.
                let current = live
                    && request == self.info_media_request
                    && self.info_media_allowed()
                    && self.info.as_ref().is_some_and(|info| match &result {
                        Ok(media) => media.chat == info.chat,
                        Err(_) => true,
                    });
                if current {
                    self.info_media_pending = false;
                    match result {
                        Ok(media) => {
                            self.info_media_failed = false;
                            self.set_info_media(media);
                            if std::mem::take(&mut self.info_media_stale) {
                                self.request_info_media();
                            }
                        }
                        Err(_) => {
                            self.info_media_failed = true;
                            self.info_media_stale = false;
                        }
                    }
                }
            }
            // The worker's word that what the panel lists changed; the
            // transcript's own events are not read for it.
            Event::ChatSharedChanged(chat) => {
                if live {
                    self.refresh_info_media(&chat);
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
                if live
                    && self.open_chat.as_deref() == Some(chat.as_str())
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
                        .filter(|message| self.chat(&message.chat).is_none_or(|chat| !chat.locked))
                        .collect();
                }
            }
            Event::Incoming { chat, message } => self.maybe_notify(&chat, &message),
            Event::Picked {
                chat,
                paths,
                documents,
            } => {
                if live && self.open_chat.as_deref() == Some(chat.as_str()) {
                    if documents {
                        self.stage_documents(paths);
                    } else {
                        self.stage_files(paths);
                    }
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
            Event::ForwardLimit {
                from_chat,
                messages,
                limit,
            } => {
                // The answer for the dialog on screen, not one asked earlier
                // or by an account that is not on screen.
                if live
                    && self.dialog
                        == Some(Dialog::Forward {
                            chat: from_chat,
                            messages,
                        })
                {
                    self.forward_limit = limit;
                    self.forward_recipients.truncate(limit);
                    if self.forward_recipients.len() < 2 {
                        self.forward_reviewing = false;
                    }
                }
            }
            Event::PollCreated { chat, error } => {
                self.poll_creating = false;
                if let Some(error) = error {
                    self.toast_error(error);
                } else if live && self.dialog == Some(Dialog::CreatePoll(chat)) {
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
                // The starred panel holds its own copy. An edit has to
                // replace that row, or the open list keeps the old text.
                let in_starred = self.starred.iter().any(|entry| {
                    entry.message.chat == message.chat && entry.message.id == message.id
                });
                if matches!(message.content, Content::Revoked) {
                    // A revoke for everyone arrives as an update, not as a
                    // deletion. The star it still holds would keep the row
                    // in the open list with nothing left to unstar.
                    self.forget_star(&message.chat, &message.id);
                    // Its pin goes with it, from the line and the mark.
                    if let Some(ids) = self.pins.get_mut(&message.chat) {
                        ids.remove(&message.id);
                    }
                    if let Some(rows) = self.chat_pins.get_mut(&message.chat) {
                        rows.retain(|row| row.id != message.id);
                    }
                } else if in_starred {
                    for entry in &mut self.starred {
                        if entry.message.chat == message.chat && entry.message.id == message.id {
                            entry.message = message.clone();
                        }
                    }
                    self.reload_lists();
                }
                let mut restamped = None;
                if let Some(conversation) = self.conversations.get_mut(&message.chat)
                    && let Some(existing) = conversation.message_mut(&message.id)
                {
                    let state = existing.content.media().map(|media| media.state.clone());
                    let motion_state = match &existing.content {
                        Content::Image {
                            motion: Some(motion),
                            ..
                        } => Some(motion.state.clone()),
                        _ => None,
                    };
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
                    let moved = existing.timestamp != message.timestamp;
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
                    if let (
                        Some(state),
                        Content::Image {
                            motion: Some(motion),
                            ..
                        },
                    ) = (motion_state, &mut existing.content)
                    {
                        motion.state = state;
                    }
                    if moved {
                        // Our own message, restamped with the server's time.
                        restamped = Some(existing.chat.clone());
                        conversation.sort();
                    }
                }
                // A hidden account's events never touch the window's selection.
                if let Some(chat) = restamped.filter(|_| !self.events_hidden) {
                    self.order_selection(&chat);
                }
            }
            Event::Contacts(contacts) => {
                if live {
                    self.info_members = None;
                }
                for contact in contacts {
                    self.contacts.insert(contact.id.clone(), contact);
                }
            }
            Event::Typing {
                chat,
                sender,
                activity,
            } => {
                let typers = self.typing.entry(chat).or_default();
                typers.retain(|(who, _, _)| *who != sender);
                if let Some(activity) = activity {
                    typers.push((sender, activity, Instant::now()));
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
                // A changed picture arrives under the file name of the one it
                // replaces, which egui would otherwise keep showing.
                self.stale_images
                    .extend(path.iter().map(|path| crate::util::image_uri(path)));
                let previous = if full {
                    self.avatar_full_requests.remove(&id);
                    self.avatars_full.insert(id, path)
                } else {
                    self.avatar_requests.remove(&id);
                    self.avatars.insert(id, path)
                };
                self.stale_images.extend(
                    previous
                        .flatten()
                        .map(|previous| crate::util::image_uri(&previous)),
                );
            }
            Event::Gifs {
                query,
                offset,
                request,
                results,
            } => {
                if query == self.gif_query && request == self.gif_request && self.gif_pending {
                    self.gif_pending = false;
                    match results {
                        Ok(results) => {
                            self.gif_next_offset = results.next_offset;
                            if offset == 0 {
                                self.gif_results = results.gifs;
                            } else {
                                let mut ids: std::collections::HashSet<_> =
                                    self.gif_results.iter().map(|gif| gif.id.clone()).collect();
                                self.gif_results.extend(
                                    results
                                        .gifs
                                        .into_iter()
                                        .filter(|gif| ids.insert(gif.id.clone())),
                                );
                            }
                            self.gif_error = None;
                        }
                        Err(error) => {
                            if offset == 0 {
                                self.gif_results.clear();
                            }
                            self.gif_error = Some(error);
                        }
                    }
                }
            }
            Event::Stickers {
                favorites,
                packs,
                recent,
                received,
                emojis,
            } => {
                self.stickers_saved = favorites;
                self.sticker_packs = packs;
                self.stickers = recent;
                self.stickers_received = received;
                self.stickers_pending = false;
                self.sticker_import_pending = false;
                // The picker belongs to the account on screen.
                if !live {
                    return;
                }
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
            }
            Event::MessageDeleted { chat, id } => {
                // The current message deleted, the list's cursor goes to the
                // one after it, or before it when it was the newest, so the
                // reader stays where they were rather than at the newest.
                let neighbour = self.conversations.get(&chat).and_then(|conversation| {
                    let at = conversation
                        .messages
                        .iter()
                        .position(|message| message.id == id)?;
                    conversation
                        .messages
                        .get(at + 1)
                        .or_else(|| {
                            at.checked_sub(1)
                                .and_then(|at| conversation.messages.get(at))
                        })
                        .map(|message| message.id.clone())
                });
                if live
                    && self.message_cursor.get(&chat) == Some(&id)
                    && let Some(neighbour) = neighbour
                {
                    // The followed message deleted, the cursor's new message
                    // is followed in its place, so new messages still are.
                    if self
                        .cursor_followed
                        .as_ref()
                        .is_some_and(|(followed_chat, followed)| {
                            *followed_chat == chat && *followed == id
                        })
                    {
                        self.cursor_followed = Some((chat.clone(), neighbour.clone()));
                    }
                    self.message_cursor.insert(chat.clone(), neighbour);
                }
                if let Some(conversation) = self.conversations.get_mut(&chat) {
                    conversation.messages.retain(|message| message.id != id);
                }
                if live && self.editing.as_deref() == Some(id.as_str()) {
                    self.editing = None;
                    self.composer.clear();
                }
                self.forget_viewer_item(&chat, &id);
                // A photo that left the chat leaves the album on screen.
                self.refresh_viewer_album(&chat);
                // A deleted message leaves the line under the header and
                // the mark on its bubble behind.
                if let Some(pins) = self.chat_pins.get_mut(&chat) {
                    pins.retain(|pin| pin.id != id);
                }
                if let Some(ids) = self.pins.get_mut(&chat) {
                    ids.remove(&id);
                }
                // A deleted message leaves the starred list.
                self.forget_star(&chat, &id);
            }
            Event::ChatRemoved { chat } => {
                self.forget_chat(&chat);
                self.stars.remove(&chat);
                self.reload_lists();
            }
            Event::ChatCleared { chat, through } => {
                self.handle_chat_cleared(&chat, through, live);
                self.stars.remove(&chat);
                // Clearing the chat clears its pins in the archive too.
                self.pins.remove(&chat);
                self.chat_pins.remove(&chat);
                self.reload_lists();
            }
            Event::Media {
                card,
                chat,
                message,
                result,
            } => self.handle_media(&chat, &message, card, result),
            Event::Motion {
                chat,
                message,
                result,
            } => {
                // The viewer, on this photo, asked for the clip to play it.
                let previewed = !self.events_hidden
                    && self.open_chat.as_deref() == Some(chat.as_str())
                    && self.image_preview.as_ref().is_some_and(|preview| {
                        preview.chat() == chat && preview.message() == message
                    });
                if let Some(motion) = self.motion_mut(&chat, &message) {
                    match result {
                        Ok(path) => {
                            motion.path = Some(path.clone());
                            motion.state = MediaState::Idle;
                            if previewed {
                                self.actions.push(Action::ExpandVideo { message, path });
                            }
                        }
                        Err(error) => {
                            motion.state = MediaState::Failed(error);
                            if self
                                .motion_playing
                                .as_ref()
                                .is_some_and(|(c, m)| *c == chat && *m == message)
                            {
                                self.motion_playing = None;
                            }
                        }
                    }
                }
            }
            Event::Syncing(syncing) => {
                if self.syncing && !syncing {
                    self.toast("History loaded");
                }
                if live && !self.syncing && syncing {
                    self.announcer.say(
                        crate::announcer::Category::Status,
                        crate::i18n::gettext(self.locale, "Loading chat history"),
                    );
                }
                self.syncing = syncing;
                if !syncing {
                    self.sync_percent = None;
                }
            }
            Event::SyncProgress(percent) => self.sync_percent = Some(percent),
            Event::OlderFetched { chat, more, silent } => {
                let conversation = self.conversations.entry(chat).or_default();
                if silent {
                    // A background page is not an answer to a request the
                    // reader made: it must not clear the spinner of a
                    // request still in flight, and it must not count
                    // against the phone, or every successful prefetch would
                    // look like a miss and back off the next scroll.
                    //
                    // It does fill the archive with older messages, so a
                    // chat that had reached its start has not any more:
                    // without this, scrolling up asks the phone instead of
                    // reading what the page just filed, and the page stays
                    // out of sight until the chat is opened again.
                    conversation.complete = false;
                    return;
                }
                conversation.fetching_phone = false;
                conversation.phone_explicit = false;
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
                    self.account_receipts_off = choice != crate::privacy::PrivacyChoice::Everyone;
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
                if live
                    && self.receipts_watch.as_ref().is_some_and(|(chat, message)| {
                        *chat == receipts.chat && *message == receipts.message
                    })
                {
                    self.message_receipts = Some(receipts);
                }
            }
            Event::ChatSoundPicked { chat, path } => {
                crate::notify::play_sound(crate::settings::NotificationSound::Custom(path.clone()));
                self.actions.push(Action::SetChatSound {
                    chat,
                    sound: Some(crate::settings::NotificationSound::Custom(path)),
                });
            }
            Event::DownloadFolderPicked(path) => {
                if live {
                    self.actions.push(Action::SetDownloadFolder(Some(path)));
                }
            }
            Event::WallpaperImagePicked(Ok(path)) => {
                // The copy may keep the earlier one's name: decode it anew.
                self.account_mut().settings.wallpaper_image = Some(path);
                self.account_mut().mark_settings_dirty();
                if live {
                    self.wallpaper_image.reload();
                }
            }
            Event::WallpaperImagePicked(Err(error)) => {
                let message = crate::i18n::gettext(self.locale, "Could not use this image");
                self.toast_error(format!("{message}: {error}"));
            }
            Event::NotificationSoundPicked { mention, path } => {
                if live {
                    crate::notify::play_sound(crate::settings::NotificationSound::Custom(
                        path.clone(),
                    ));
                    self.actions.push(Action::SetNotificationSound {
                        mention,
                        sound: crate::settings::NotificationSound::Custom(path),
                    });
                }
            }
            Event::InvitePreview { code, result } => {
                use crate::model::InviteState;
                if let Some(invite) = self.invite.as_mut().filter(|invite| invite.code == code) {
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
                            if live && self.dialog == Some(Dialog::JoinGroup) {
                                self.dialog = None;
                            }
                            if pending {
                                self.toast(
                                    "Request sent. An admin must approve it before you join.",
                                );
                            } else if live {
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
            Event::NewGroupPicture(result) => match result {
                Ok(path) => {
                    if let Some(draft) = self.new_group.as_mut() {
                        draft.picture = Some(path);
                    }
                }
                Err(_) if live => {
                    let said = crate::i18n::gettext(self.locale, "Could not use this picture.");
                    self.toast_error(said.into_owned());
                }
                Err(_) => {}
            },
            Event::GroupCreated { request, result } => self.group_created(request, result, live),
            Event::GroupInviteLink {
                chat,
                reset,
                result,
            } => self.group_invite_link(chat, reset, result, live),
            Event::GroupMembersAdded { chat, result } => {
                self.group_members_added(chat, result, live)
            }
            Event::GroupPermissionSaved { result, .. } => {
                if let Err(failure) = result
                    && live
                {
                    let said = failure.message(self.locale);
                    self.toast_error(said);
                }
            }
            Event::GroupMemberChanged {
                member,
                change,
                result,
                ..
            } => {
                if live {
                    match result {
                        Ok(()) => {
                            let name = self.display_name(&member);
                            let said =
                                crate::group_admin::changed_message(self.locale, change, &name);
                            self.toast(said);
                        }
                        Err(failure) => {
                            let said = failure.message(self.locale);
                            self.toast_error(said);
                        }
                    }
                }
            }
            Event::ContactFailed { request, error } => {
                if self.new_contact_request == Some(request) {
                    self.new_contact_request = None;
                    self.new_contact_dialog_request = None;
                    self.new_contact_pending = false;
                    if live {
                        self.toast_error(contact_failure_message(self.locale, &error));
                    }
                }
            }
            Event::ContactReady { request, id, name } => {
                if self.new_contact_request != Some(request) {
                    return;
                }
                let owned_by_dialog = self.new_contact_dialog_request.take() == Some(request);
                self.new_contact_request = None;
                self.new_contact_pending = false;
                if live {
                    if owned_by_dialog && matches!(self.dialog, Some(Dialog::NewContact)) {
                        self.dialog = None;
                    }
                    let name = name
                        .filter(|name| !name.is_empty())
                        .unwrap_or_else(|| crate::util::phone(&id));
                    self.actions.push(Action::StartChat {
                        id,
                        name,
                        dismiss_dialog: false,
                    });
                }
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
                if live {
                    self.picker = None;
                    self.dialog = Some(Dialog::StickerMaker);
                }
            }
            Event::PictureInspected { target, result } => {
                // Only the picture still being waited for may open the cropper.
                // Anything else was cancelled or overtaken while its size was
                // being read, and opening on it would take the window off
                // whoever has it now.
                if self.picture_inspecting == Some(target) {
                    self.picture_inspecting = None;
                    match result {
                        Ok((width, height)) => {
                            // The attachment may have been removed while its
                            // size was being read, so it is found by name
                            // rather than by place.
                            let source = self.staged_at(target).and_then(|at| {
                                match &self.pending[at].item {
                                    Pending::File(path) => Some(path.clone()),
                                    Pending::Picture { .. } | Pending::Document(_) => None,
                                }
                            });
                            if let Some(path) = source {
                                self.picture_edit = Some(crate::model::PictureEdit::new(
                                    target,
                                    crate::model::PictureSource::File(path),
                                    width,
                                    height,
                                ));
                            }
                        }
                        Err(error) => self.toast_error(error),
                    }
                }
            }
            Event::PictureEdited { target, result } => {
                // The window is left alone: applying a crop takes the editor
                // with it, so an editor open here belongs to another picture
                // and closing it would throw away a crop nobody finished.
                match result {
                    // Applied only while that attachment is still staged. A
                    // result that arrives after its attachment was removed, or
                    // after another account took over the composer, is dropped
                    // rather than landing on whatever sits in its place now.
                    Ok(path) => match self.staged_at(target) {
                        Some(at) => {
                            self.pending[at].item = Pending::File(path);
                            if let Some(origin) = self.picture_applying.remove(&target) {
                                self.picture_origins.insert(target, origin);
                            }
                        }
                        None => {
                            self.picture_applying.remove(&target);
                        }
                    },
                    Err(error) => {
                        self.picture_applying.remove(&target);
                        self.toast_error(error);
                    }
                }
            }
            Event::StickerPackPreview(result) => {
                self.sticker_preview_pending = false;
                match result {
                    Ok(preview) => self.sticker_preview = Some(preview),
                    Err(error) => {
                        if live && self.dialog == Some(Dialog::StickerPack) {
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
                Err(error) => self.update_download = crate::updates::DownloadState::Failed(error),
            },
            Event::SendRefused {
                chat,
                quoting,
                unsent,
                reason,
            } => self.send_refused(chat, quoting, unsent, reason),
            Event::GroupSaving { chat, saving } => {
                if saving {
                    self.group_saving.insert(chat);
                } else {
                    self.group_saving.remove(&chat);
                }
            }
            Event::Error(message) => {
                self.sticker_import_pending = false;
                // The worker sends the English catalog key. `gettext` only
                // accepts a static key, so unknown errors stay as the worker
                // wrote them.
                let shown = match message.as_str() {
                    "This chat already has three pinned messages" => crate::i18n::gettext(
                        self.locale,
                        "This chat already has three pinned messages",
                    )
                    .into_owned(),
                    crate::calls::GROUP_CALLS_NOT_ENABLED => crate::i18n::gettext(
                        self.locale,
                        "WhatsApp hasn't turned on group calls from a computer for your account yet. Ask someone in the group to call it, and answer here.",
                    )
                    .into_owned(),
                    crate::calls::CALL_LINKS_OFF => crate::i18n::gettext(
                        self.locale,
                        "ZapFast can't join group calls yet: open the link on your phone.",
                    )
                    .into_owned(),
                    _ => message,
                };
                self.toast_error(shown);
            }
            Event::HistoryUnavailable { chat, error } => {
                // The replied message's own request, asked before this
                // answer came, failed: its wait ends, and the error alone is
                // said. Another request's answer leaves the wait as an empty
                // page does.
                let answer = live
                    && self
                        .pending_follow
                        .as_ref()
                        .is_some_and(|pending| pending.chat == chat && pending.asked);
                self.apply_backend_event(
                    Event::Messages {
                        chat,
                        messages: Vec::new(),
                        older: true,
                        complete: false,
                    },
                    live,
                );
                if answer {
                    self.pending_follow = None;
                }
                self.apply_backend_event(Event::Error(error), live);
            }
            Event::AccountRemoved => {}
            Event::ArchiveFailed { archived, failure } => {
                use crate::backend::ArchiveFailure;
                let message = match failure {
                    ArchiveFailure::Offline if archived => crate::i18n::gettext(
                        self.locale,
                        "Connect to WhatsApp to archive this chat",
                    ),
                    ArchiveFailure::Offline => crate::i18n::gettext(
                        self.locale,
                        "Connect to WhatsApp to unarchive this chat",
                    ),
                    ArchiveFailure::Read => {
                        crate::i18n::gettext(self.locale, "Could not read this chat. Try again")
                    }
                    ArchiveFailure::Sync if archived => crate::i18n::gettext(
                        self.locale,
                        "Could not confirm archiving on WhatsApp. Try again",
                    ),
                    ArchiveFailure::Sync => crate::i18n::gettext(
                        self.locale,
                        "Could not confirm unarchiving on WhatsApp. Try again",
                    ),
                    ArchiveFailure::Write => crate::i18n::gettext(
                        self.locale,
                        "WhatsApp updated this chat, but ZapFast could not save the change",
                    ),
                };
                self.toast_error(message);
            }
        }
        // A hidden account's events never touch the window's selection.
        if !self.events_hidden {
            self.prune_selection();
        }
    }

    fn prune_selection(&mut self) {
        let Some((chat, ids)) = self.selection.clone() else {
            return;
        };
        let conversation = self.conversations.get(chat.as_str());
        let selectable = |id: &str| {
            conversation
                .and_then(|conversation| conversation.message(id))
                .is_some_and(|message| can_select(&message.content))
        };
        let ids: Vec<String> = ids.into_iter().filter(|id| selectable(id)).collect();
        let anchor_gone = self
            .selection_anchor
            .as_deref()
            .is_some_and(|id| !selectable(id));
        let base = self
            .sweep
            .as_ref()
            .filter(|sweep| sweep.chat == chat)
            .map(|sweep| {
                sweep
                    .base
                    .iter()
                    .filter(|id| selectable(id))
                    .cloned()
                    .collect::<Vec<_>>()
            });
        if anchor_gone {
            self.selection_anchor = ids.last().cloned();
        }
        if let (Some(base), Some(sweep)) = (base, self.sweep.as_mut()) {
            sweep.base = base;
        }
        self.selection = Some((chat, ids));
    }

    /// Puts the selected messages of `chat` back in chat order after one of
    /// them moved, so selection actions keep following the chat.
    fn order_selection(&mut self, chat: &str) {
        let Some((selected, mut ids)) = self.selection.clone() else {
            return;
        };
        let Some(conversation) = self.conversations.get(chat).filter(|_| selected == chat) else {
            return;
        };
        ids.sort_by_key(|id| {
            conversation
                .messages
                .iter()
                .position(|message| message.id == *id)
        });
        self.selection = Some((selected, ids));
    }

    /// Updates one account's link state, discarding its drafts when it is unlinked.
    fn handle_link(&mut self, status: LinkStatus, live: bool) {
        // Only a drop after being connected is news; the startup sequence
        // (Starting, Connecting, Connected) stays silent. An account that is
        // not on screen is never announced.
        if live
            && matches!(status, LinkStatus::Disconnected { .. })
            && matches!(self.link, LinkStatus::Connected)
        {
            self.announced_offline = true;
            self.announcer.say(
                crate::announcer::Category::Status,
                crate::i18n::gettext(self.locale, "Connection lost"),
            );
        } else if live
            && matches!(status, LinkStatus::Connecting | LinkStatus::Starting)
            && self.announced_offline
            && !self.announced_connecting
        {
            self.announced_connecting = true;
            self.announcer.say(
                crate::announcer::Category::Status,
                crate::i18n::gettext(self.locale, "Connecting"),
            );
        }
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
                // The toast announces itself, which covers "Connected".
                // A retry passes through Connecting, so the episode flag counts too.
                if (live && self.announced_offline)
                    || matches!(self.link, LinkStatus::Disconnected { .. })
                {
                    self.toast("Back online");
                }
                if live {
                    self.announced_offline = false;
                    self.announced_connecting = false;
                    self.dialog = match self.dialog.take() {
                        Some(Dialog::PairWithPhone) => None,
                        other => other,
                    };
                }
                if let Some(open) = self.open_chat.clone() {
                    self.ensure_loaded(&open);
                }
            }
            LinkStatus::LoggedOut => {
                if live {
                    self.invalidate_composer_undo();
                }
                // Account-scoped lookup results are discarded by the worker.
                // End their UI requests too, so a later account can retry.
                self.new_contact_request = None;
                self.new_contact_dialog_request = None;
                self.new_contact_pending = false;
                let account = self.account().id.clone();
                self.poll_voting.clear();
                self.interactive_sending.clear();
                self.poll_creating = false;
                self.poll_draft = Default::default();
                self.notifications.clear_account(&account);
                self.chats.clear();
                self.conversations.clear();
                self.contacts.clear();
                self.avatars.clear();
                self.starred.clear();
                self.show_starred = false;
                self.stars.clear();
                self.pins.clear();
                self.chat_pins.clear();
                self.account_privacy = crate::privacy::Snapshot::default();
                self.account_receipts_off = false;
                self.open_chat = None;
                // Every recorded place named a chat of the account that just
                // went away, but only when that account is the one on screen.
                if !self.events_hidden {
                    self.nav_history.clear();
                    // So did the info panel's chat and its listing.
                    self.close_info();
                }
                // Unsent text belongs to the account that was unlinked.
                self.drafts.clear();
                self.draft_mentions.clear();
                self.attachment_drafts.clear();
                if live {
                    self.pending.clear();
                    self.pending_view_once = false;
                    self.reply_to = None;
                    self.composer.clear();
                    self.composer_mentions.clear();
                    // The viewer, its album and the thumbnails it cached belong
                    // to the account that left: close it and release the clip.
                    self.video.stop();
                    self.image_preview = None;
                    self.viewer_media.clear();
                    self.viewer_media_chat = None;
                    self.viewer_media_state.clear();
                }
                self.link = status;
                // The password guarded chats that are gone now; a forgotten
                // one is recovered exactly this way, once no number remains.
                if !self.any_linked() {
                    self.forget_app_lock();
                }
                self.toast_error("This device was unlinked from your phone");
                return;
            }
            LinkStatus::Failed(message) => self.toast_error(message.clone()),
            _ => {}
        }
        self.link = status;
    }

    fn handle_chat_updated(&mut self, chat: Chat, live: bool) {
        // A hidden account's remembered chat is not being read: marking it
        // read would send receipts for messages nobody has seen.
        let remembered = self.open_chat.as_deref() == Some(chat.id.as_str());
        let is_open = live && remembered && self.chat_view();
        let mut chat = chat;
        if chat.unread == 0 {
            self.clear_chat_notifications(&chat.id);
            // Read on another device: the chat's next message may notify at once.
            if self.chat(&chat.id).is_some_and(|known| known.unread > 0) {
                self.reset_notification_throttle(&chat.id);
            }
        }
        if is_open
            && (!chat.locked || self.locked_folder_open())
            && chat.unread > 0
            && self.window_focused
            && !self.window_hidden
        {
            chat.unread = 0;
            chat.unread_mention = None;
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
        if (is_open || (!live && remembered))
            && chat.archived
            && self.chat(&chat.id).is_none_or(|known| !known.archived)
        {
            if live {
                self.actions.push(Action::CloseChat);
            } else {
                self.open_chat = None;
            }
        }
        let newly_locked = chat.locked && self.chat(&chat.id).is_none_or(|known| !known.locked);
        let locked_id = chat.id.clone();
        match self.chats.iter_mut().find(|known| known.id == chat.id) {
            Some(existing) => *existing = chat,
            None => self.chats.push(chat),
        }
        self.chats
            .sort_by_key(|chat| std::cmp::Reverse(chat.last_activity));
        // The starred query hides locked chats. Drop the row now, before the
        // reload comes back, so the open panel does not keep painting it.
        if newly_locked {
            self.starred.retain(|entry| entry.message.chat != locked_id);
            self.reload_lists();
        }
    }

    /// Rechecks source-chat authorization for every retained private quote.
    fn reply_source_visible(&self, target: &crate::model::ReplyTarget, destination: &str) -> bool {
        target.chat.is_none()
            || self
                .chat(target.source_chat(destination))
                .is_some_and(|chat| !chat.locked || self.locked_folder_open())
    }

    /// Resolves a quote preview only while its source chat is visible to the user.
    pub(crate) fn reply_preview(
        &self,
        target: &crate::model::ReplyTarget,
        destination: &str,
    ) -> Option<&Message> {
        let source = target.source_chat(destination);
        if !self.reply_source_visible(target, destination) {
            return None;
        }
        self.conversations.get(source)?.message(target)
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
        // A live call in a locked chat goes back behind the bar.
        self.hide_private_call();
        self.drop_locked_panel();
    }

    /// Takes a locked chat's info panel away once it may no longer be shown:
    /// lock authentication ended, or the chat was locked under its panel.
    /// The listing, the pictures' textures and the answer being waited for
    /// go with it.
    fn drop_locked_panel(&mut self) {
        if self.info.is_some() && !self.info_media_allowed() {
            self.close_info();
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
        // A locked chat's panel opened before this can be listed now.
        self.request_info_media_if_unloaded();
    }

    /// Empties a chat that stays listed. Search hits and anything pointing at
    /// one of its messages would otherwise refer to rows that are gone, and a
    /// pending edit would send `EditText` for a message that no longer exists.
    fn handle_chat_cleared(&mut self, id: &str, through: i64, live: bool) {
        // A confirmation that is open for this chat is about messages that are
        // already gone: clearing again would take what arrived since.
        if matches!(&self.dialog, Some(Dialog::ConfirmClearChat(chat)) if chat == id) {
            self.dialog = None;
        }
        self.clear_chat_notifications(id);
        // Clearing a chat also removes its stored draft.
        self.drafts.remove(id);
        self.draft_mentions.remove(id);
        self.attachment_drafts.remove(id);
        self.search_hits
            .retain(|message| message.chat != id || message.timestamp > through);
        // Nothing earlier is left here, and the phone no longer has it either.
        let open = self.open_chat.as_deref() == Some(id);
        let editing = self.editing.clone();
        let reply = self.reply_to.clone();
        let reaction = self.reaction_target.clone();
        let (editing_gone, reply_gone, reaction_gone) = {
            let conversation = self.conversations.entry(id.to_owned()).or_default();
            conversation
                .messages
                .retain(|message| message.timestamp > through);
            conversation.requested = true;
            conversation.complete = true;
            conversation.phone_exhausted = true;
            conversation.loading_older = false;
            (
                editing
                    .as_ref()
                    .is_some_and(|edit| conversation.message(edit).is_none()),
                reply.as_ref().is_some_and(|reply| {
                    reply.source_chat(id) == id && conversation.message(&reply.id).is_none()
                }),
                reaction
                    .as_ref()
                    .is_some_and(|(_, target)| conversation.message(target).is_none()),
            )
        };
        if open && live {
            if editing_gone {
                self.editing = None;
                self.composer.clear();
                self.composer_mentions.clear();
            }
            if reply_gone {
                self.reply_to = None;
            }
            if reaction_gone {
                self.reaction_target = None;
                self.reaction_anchor = None;
                self.picker = None;
            }
        }
        // The album the viewer is browsing was built from the rows that just
        // went: ask for it again, so it offers what the chat still has.
        self.refresh_viewer_album(id);
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
        self.chat_search_open && self.chat_view() && self.open_chat.is_some()
    }

    /// Opens the search pane beside the open chat, or focuses its field when
    /// it is already open.
    fn open_chat_search(&mut self) {
        if self.open_chat.is_none() || !self.chat_view() {
            return;
        }
        // The pane takes the info panel's place beside the chat.
        self.close_info();
        if !self.chat_search_open {
            self.chat_search_open = true;
            self.chat_search_month = crate::util::today();
        }
        self.focus_chat_search = true;
        self.focus_search = false;
        self.focus_composer = false;
    }

    /// Whether the info panel is on screen: it sits beside the open chat and
    /// only the chat view shows it.
    pub fn info_visible(&self) -> bool {
        self.info.is_some() && self.chat_view() && self.open_chat.is_some()
    }

    /// Opens the info panel on a chat's overview, in the search pane's
    /// place. The listing is asked for once per chat shown; opening the
    /// same chat again only returns to the overview.
    fn open_info(&mut self, chat: ChatId) {
        if self.open_chat.is_none() || !self.chat_view() {
            return;
        }
        self.close_chat_search();
        self.contact_edit = None;
        self.group_name_edit = None;
        self.emoji_start = None;
        self.mention_start = None;
        if self.info.as_ref().is_none_or(|info| info.chat != chat) {
            // The listing asked for the chat shown before is not this
            // chat's: its answer is refused and nothing stays due, even
            // when this chat cannot be asked for yet.
            self.reset_info_media();
            self.info_members = None;
            self.info_invite = None;
            self.info = Some(InfoPanel {
                chat,
                view: InfoView::Overview,
            });
            self.request_info_media();
        } else if let Some(info) = self.info.as_mut() {
            info.view = InfoView::Overview;
            self.request_info_media_if_unloaded();
        }
    }

    /// Asks for the panel's listing when it may be listed and nothing is
    /// loaded, on its way or failed: the same panel opened again, or lock
    /// authentication opening the way for a locked chat's panel.
    fn request_info_media_if_unloaded(&mut self) {
        if self.info.is_some()
            && self.info_media_allowed()
            && self.info_media.is_none()
            && !self.info_media_pending
            && !self.info_media_failed
        {
            self.request_info_media();
        }
    }

    fn close_info(&mut self) {
        self.info = None;
        self.reset_info_media();
        self.info_members = None;
        self.info_invite = None;
        self.info_focus = None;
        self.contact_edit = None;
        self.group_name_edit = None;
    }

    /// Drops the listing and invalidates the request still out for it: an
    /// answer on its way is for a chat the panel no longer shows.
    fn reset_info_media(&mut self) {
        self.info_media = None;
        self.info_released.append(&mut self.info_uris);
        self.info_media_request += 1;
        self.info_media_pending = false;
        self.info_media_stale = false;
        self.info_media_failed = false;
    }

    /// The one place the open chat changes: the info panel belongs beside
    /// the chat it was opened over, and goes with it.
    fn set_open_chat(&mut self, id: Option<ChatId>) {
        if self.open_chat != id {
            self.close_info();
        }
        self.open_chat = id;
    }

    /// Whether the panel may list its chat's media: a locked chat's stays
    /// in the locked folder, so a member's panel opened from an unlocked
    /// group shows who they are and nothing shared with them.
    pub fn info_media_allowed(&self) -> bool {
        self.info.as_ref().is_some_and(|info| {
            !self.chat(&info.chat).is_some_and(|chat| chat.locked) || self.locked_folder_open()
        })
    }

    fn request_info_media(&mut self) {
        if !self.info_media_allowed() {
            return;
        }
        if let Some(info) = &self.info {
            self.info_media_request += 1;
            self.info_media_pending = true;
            self.info_media_failed = false;
            self.backend.send(Command::ListChatShared {
                chat: info.chat.clone(),
                request: self.info_media_request,
            });
        }
    }

    /// Keeps the panel's listing current after `chat` gained, changed or
    /// lost a message: one request at a time, and one more once it is in.
    fn refresh_info_media(&mut self, chat: &str) {
        if self.info.as_ref().is_none_or(|info| info.chat != chat) || !self.info_media_allowed() {
            return;
        }
        if self.info_media_pending {
            self.info_media_stale = true;
        } else {
            self.request_info_media();
        }
    }

    /// Keeps a listing, with the texture name of each of its pictures, so
    /// the panel draws them under fixed names and frees them together when
    /// it closes. The pictures the last listing had and this one has not
    /// are freed on the panel's next frame.
    pub(crate) fn set_info_media(&mut self, media: ChatMedia) {
        let uris: Vec<String> = media
            .media
            .iter()
            .map(|row| crate::ui::info::thumbnail_uri(&row.chat, &row.id))
            .collect();
        let listed: HashSet<&str> = uris.iter().map(String::as_str).collect();
        // One dropped earlier and listed again before a frame stays.
        self.info_released
            .retain(|uri| !listed.contains(uri.as_str()));
        let dropped = self
            .info_uris
            .drain(..)
            .filter(|uri| !listed.contains(uri.as_str()));
        self.info_released.extend(dropped);
        self.info_textures.extend(uris.iter().cloned());
        self.info_uris = uris;
        self.info_media = Some(media);
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

    /// Whether `text` holds `needle` once both are normalized, remembering
    /// the normalized form so the next frame does not pay for it again.
    fn text_matches(&self, text: &str, needle: &str) -> bool {
        if let Some(key) = self.search_keys.borrow().get(text) {
            return key.contains(needle);
        }
        let key = crate::util::search_key(text);
        let hit = key.contains(needle);
        self.search_keys.borrow_mut().insert(text.to_owned(), key);
        hit
    }

    /// Asks the archive to be scanned for the chat list's search text.
    fn request_search(&mut self) {
        self.search_due = None;
        let query = self.search.trim().to_owned();
        if query.is_empty() || self.locked_folder_open() || self.secret_code_matched() {
            return;
        }
        self.backend.send(Command::SearchMessages { query });
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

    /// Parks the open locked chat's draft before hiding its conversation and controls.
    fn hide_locked_chat(&mut self, id: &str) {
        // A locked chat still exists, so its unsent text waits as a draft.
        // Text emptied in the composer clears the stored copy too. A hidden
        // account's composer text is already parked among its drafts.
        if self.events_hidden {
            self.leave_chat(id);
            return;
        }
        if self.open_chat.as_deref() == Some(id) {
            self.save_attachment_draft(id);
        }
        if self.open_chat.as_deref() == Some(id)
            && self.editing.is_none()
            && self.composer.is_empty()
        {
            self.store_draft(id, "");
        }
        if self.open_chat.as_deref() == Some(id)
            && self.editing.is_none()
            && !self.composer.is_empty()
            && !self.is_untouched_prefill(id, &self.composer)
        {
            let draft = std::mem::take(&mut self.composer);
            let mentions = std::mem::take(&mut self.composer_mentions);
            self.drafts.insert(id.to_owned(), draft);
            self.draft_mentions.insert(id.to_owned(), mentions);
            self.store_draft(
                id,
                self.drafts.get(id).map(String::as_str).unwrap_or_default(),
            );
        }
        self.prefilled = None;
        self.leave_chat(id);
    }

    /// Takes back a send the worker refused. Nothing typed or recorded is
    /// lost, and a refused reply is never sent again without its quote unless
    /// the user cancels the reply first.
    fn send_refused(
        &mut self,
        chat: ChatId,
        quoting: Option<crate::model::ReplyTarget>,
        unsent: Unsent,
        reason: Refusal,
    ) {
        // A private reply quotes another chat, so its recovery needs the
        // destination chat to still exist.
        if quoting.as_ref().is_some_and(|target| target.chat.is_some())
            && self.chat(&chat).is_none()
        {
            return;
        }
        let open = !self.events_hidden && self.open_chat.as_deref() == Some(chat.as_str());
        // Re-arm the reply banner, unless the user has moved on to another
        // reply or an edit since.
        if let Some(target) = quoting.clone() {
            if open && self.reply_to.is_none() && self.editing.is_none() {
                self.reply_to = Some(target);
                self.focus_composer = true;
            } else if !open && target.chat.is_some() {
                // The private quote waits in the chat's draft, like a
                // refused attachment's reply.
                self.attachment_drafts
                    .entry(chat.clone())
                    .or_default()
                    .quoting
                    .get_or_insert(target);
            }
        }
        match unsent {
            Unsent::Text(text) => self.restore_text(&chat, text),
            // The dialog takes the spot back while its chat is on screen and
            // nothing has opened since. Otherwise the coordinates go to that
            // chat's composer or draft as text, like a refused message, so a
            // late refusal never replaces a newer dialog and is never lost.
            Unsent::Location {
                latitude,
                longitude,
            } => {
                let spot = crate::geo::Spot {
                    latitude,
                    longitude,
                }
                .text();
                if open && self.dialog.is_none() {
                    self.location_draft = spot;
                    self.dialog = Some(Dialog::SendLocation(chat));
                } else {
                    self.restore_text(&chat, spot);
                }
            }
            Unsent::Voice(samples) => {
                self.unsent_voice = Some((chat, samples));
                self.focus_composer = open;
            }
            Unsent::Files {
                paths,
                documents,
                view_once,
                caption,
            } => {
                let pending = if documents {
                    Pending::Document
                } else {
                    Pending::File
                };
                self.restore_attachments(&chat, paths.into_iter().map(pending), quoting, view_once);
                self.restore_text(&chat, caption.unwrap_or_default());
            }
            Unsent::Image {
                width,
                height,
                rgba,
                view_once,
                caption,
            } => {
                self.restore_attachments(
                    &chat,
                    [Pending::Picture {
                        width: width as usize,
                        height: height as usize,
                        rgba: std::sync::Arc::new(rgba),
                        texture: None,
                    }],
                    quoting,
                    view_once,
                );
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
            Refusal::UnsupportedViewOnce => crate::i18n::gettext(
                self.locale,
                "Not sent: view-once mode is available for one photo at a time.",
            ),
        };
        self.toast_error(message.into_owned());
    }

    /// Returns refused attachments to the live composer or their owning account's chat draft.
    fn restore_attachments(
        &mut self,
        chat: &str,
        pending: impl IntoIterator<Item = Pending>,
        quoting: Option<crate::model::ReplyTarget>,
        view_once: bool,
    ) {
        if !self.events_hidden && self.open_chat.as_deref() == Some(chat) {
            let preserve_view_once = self.pending_view_once || view_once;
            for item in pending {
                self.stage(item);
            }
            // Recovery can merge with a newer draft. Keep the privacy choice;
            // send_pending refuses an unsupported combination until the user
            // removes the extra attachments or explicitly turns the option off.
            self.pending_view_once = preserve_view_once;
        } else if self.chat(chat).is_some() {
            // Named here as well, so the draft carries an identity back with
            // the attachment rather than handing out a new one later.
            let staged: Vec<Staged> = pending
                .into_iter()
                .map(|item| {
                    let id = self.next_attachment_id();
                    Staged { id, item }
                })
                .collect();
            let draft = self.attachment_drafts.entry(chat.to_owned()).or_default();
            draft.pending.extend(staged);
            draft.view_once |= view_once;
            if draft.quoting.is_none() {
                draft.quoting = quoting;
            }
        }
    }

    /// Returns refused text to its chat's composer, or to its stored draft
    /// when another chat is open. Text typed since the send is kept.
    fn restore_text(&mut self, chat: &str, text: String) {
        if text.trim().is_empty() {
            return;
        }
        if !self.events_hidden && self.open_chat.as_deref() == Some(chat) {
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

    fn motion_mut(&mut self, chat: &str, id: &str) -> Option<&mut crate::model::Motion> {
        match &mut self.conversations.get_mut(chat)?.message_mut(id)?.content {
            Content::Image { motion, .. } => motion.as_mut(),
            _ => None,
        }
    }

    /// Asks for the viewer's album again, anchored on what is on screen, so
    /// a message that arrives or is deleted while the viewer is open shows
    /// up there instead of waiting for the next open. Only for the account on
    /// screen: the viewer belongs to the window.
    fn refresh_viewer_album(&mut self, chat: &str) {
        if self.events_hidden || self.viewer_media_chat.as_deref() != Some(chat) {
            return;
        }
        let around = self
            .image_preview
            .as_ref()
            .map(|preview| preview.message().to_owned());
        self.backend.send(Command::LoadChatMedia {
            chat: chat.to_owned(),
            around,
        });
    }

    /// Drops a message from the viewer's album and from its download
    /// state, for a message deleted while the viewer is open.
    fn forget_viewer_item(&mut self, chat: &str, id: &str) {
        if self.events_hidden || self.viewer_media_chat.as_deref() != Some(chat) {
            return;
        }
        self.viewer_media.retain(|item| item.id != id);
        self.viewer_media_state.remove(id);
    }

    fn handle_media(
        &mut self,
        chat: &str,
        id: &str,
        card: Option<usize>,
        result: Result<PathBuf, String>,
    ) {
        // The info panel's listing keeps its own copy of the message, so a
        // picture without a preview stops being blank once its file is there.
        if let Ok(path) = &result
            && let Some(listing) = self
                .info_media
                .as_mut()
                .filter(|listing| listing.chat == chat)
            && let Some(row) = listing
                .media
                .iter_mut()
                .chain(listing.docs.iter_mut())
                .find(|row| row.id == id)
            && let Some(media) = row.content.media_at_mut(card)
        {
            media.path = Some(path.clone());
            media.state = MediaState::Idle;
        }
        // The album can hold a message the transcript has not paged in, so its
        // item, and the viewer's own download state, are updated from here,
        // before the transcript lookup below, which only the chat's own bubble
        // needs: a failure of an archived item has nowhere else to show. The
        // viewer belongs to the window, so only the account on screen reaches it.
        if card.is_none() && !self.events_hidden && self.viewer_media_chat.as_deref() == Some(chat)
        {
            match &result {
                Ok(path) => {
                    if let Some(item) = self
                        .viewer_media
                        .iter_mut()
                        .find(|item| item.id == id && item.card.is_none())
                    {
                        item.path = Some(path.clone());
                    }
                    // The viewer may be showing that item, waiting for its
                    // file: give it the file too, or it keeps offering the
                    // download it just did.
                    if let Some(preview) = &mut self.image_preview
                        && preview.message() == id
                        && preview.card().is_none()
                    {
                        preview.show_item(Some(path.clone()), id.to_owned());
                    }
                    self.viewer_media_state.remove(id);
                }
                Err(error) => {
                    self.viewer_media_state.insert(
                        id.to_owned(),
                        MediaState::Failed(download_notice(error.clone())),
                    );
                }
            }
        }
        // A carousel card's picture joins the album once it is downloaded.
        if card.is_some()
            && result.is_ok()
            && !self.events_hidden
            && self.viewer_media_chat.as_deref() == Some(chat)
            && self.image_preview.is_some()
        {
            self.refresh_viewer_album(chat);
        }
        let want_video = self
            .video_wanted
            .as_ref()
            .is_some_and(|(wanted_chat, wanted)| wanted_chat == chat && wanted == id);
        let want_voice = self
            .voice_wanted
            .as_ref()
            .is_some_and(|(wanted_chat, wanted, _)| wanted_chat == chat && wanted == id);
        let open = self.open_chat.as_deref() == Some(chat);
        // Playback waits belong to the account on screen.
        let want_video = want_video && !self.events_hidden;
        let want_voice = want_voice && !self.events_hidden;
        let applied = {
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
                    Ok(path)
                }
                Err(error) => {
                    log::warn!("attachment download failed; details are shown in the bubble");
                    media.state = MediaState::Failed(download_notice(error));
                    Err(())
                }
            }
        };
        match applied {
            Ok(path) => {
                if want_video {
                    self.video_wanted = None;
                    self.actions.push(Action::PlayVideo {
                        message: id.to_owned(),
                        path,
                    });
                } else if want_voice {
                    self.voice_wanted = None;
                    if open {
                        self.actions.push(Action::PlayVoice {
                            message: id.to_owned(),
                            path,
                        });
                    }
                }
            }
            Err(()) => {
                if want_voice {
                    self.voice_wanted = None;
                }
            }
        }
    }

    fn ensure_loaded(&mut self, chat: &str) {
        let request = {
            let conversation = self.conversations.entry(chat.to_owned()).or_default();
            if conversation.requested {
                false
            } else {
                conversation.requested = true;
                true
            }
        };
        if request {
            self.backend.send(Command::LoadChat {
                chat: chat.to_owned(),
                before: None,
            });
            self.backend.send(Command::LoadPins(chat.to_owned()));
        }
    }

    pub fn load_older(&mut self, chat: &str, explicit: bool) {
        // The account's fields directly, so the window's scroll anchor can
        // change while its conversation is borrowed.
        let account = &mut self.accounts[self.active];
        let Some(conversation) = account.conversations.get_mut(chat) else {
            return;
        };
        if conversation.loading_older {
            return;
        }
        let Some(oldest) = conversation.messages.first() else {
            return;
        };
        if conversation.complete {
            self.fetch_older(chat, explicit);
            return;
        }
        conversation.loading_older = true;
        let before = (oldest.timestamp, oldest.id.clone());
        self.scroll_anchor = Some(oldest.id.clone());
        account.backend.send(Command::LoadChat {
            chat: chat.to_owned(),
            before: Some(before),
        });
    }

    /// Requests older phone history when available and outside the cooldown.
    /// `explicit` when the reader asked by scrolling to the top; automatic
    /// requests never report a silent phone.
    pub fn fetch_older(&mut self, chat: &str, explicit: bool) {
        let account = &mut self.accounts[self.active];
        let Some(conversation) = account.conversations.get_mut(chat) else {
            return;
        };
        if conversation.phone_exhausted {
            return;
        }
        if conversation.fetching_phone {
            if explicit && !conversation.phone_explicit {
                // The reader scrolled up while an automatic request waits:
                // the worker makes that request theirs instead of asking twice.
                conversation.phone_explicit = true;
                account.backend.send(Command::FetchOlder {
                    chat: chat.to_owned(),
                    explicit,
                });
            }
            return;
        }
        // Back off after empty responses. Only a connected phone can answer.
        if !matches!(account.link, LinkStatus::Connected) {
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
        conversation.phone_explicit = explicit;
        self.scroll_anchor = conversation
            .messages
            .first()
            .map(|oldest| oldest.id.clone());
        account.backend.send(Command::FetchOlder {
            chat: chat.to_owned(),
            explicit,
        });
    }

    fn mark_read(&mut self, chat: &str) {
        self.clear_chat_notifications(chat);
        self.reset_notification_throttle(chat);
        if let Some(known) = self.chat_mut(chat) {
            known.unread = 0;
            known.marked_unread = false;
            known.unread_mention = None;
        }
        // Clear local unread state regardless of receipt settings.
        self.backend.send(Command::MarkRead {
            chat: chat.to_owned(),
            receipts: self.account().settings.send_read_receipts,
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

    /// Records where the window now is, for the back and forward buttons. The
    /// place is built only when it changed, so an idle frame allocates
    /// nothing.
    fn record_location(&mut self) {
        if self
            .nav_history
            .is_current(&self.page, self.open_chat.as_deref())
        {
            return;
        }
        self.nav_history.visit(Location {
            page: self.page.clone(),
            chat: self.open_chat.clone(),
        });
    }

    /// Steps to the place visited before this one, as the mouse's back button
    /// does.
    fn navigate_back(&mut self, ctx: &egui::Context) {
        // A place the window cannot show is forgotten by `show_location`, and
        // the step moves on to the one before it, so one press never lands on
        // a place that shows nothing. Each call takes a place off the stack,
        // so this ends at the oldest.
        while let Some(location) = self.nav_history.back() {
            if self.show_location(location, ctx) {
                break;
            }
        }
    }

    /// Steps forward again after a back step, as the mouse's forward button
    /// does.
    fn navigate_forward(&mut self, ctx: &egui::Context) {
        while let Some(location) = self.nav_history.forward() {
            if self.show_location(location, ctx) {
                break;
            }
        }
    }

    /// Restores a place through the same actions the interface uses, so a
    /// draft is saved and a locked chat stays shut. Reports whether the window
    /// could show it; a place naming a chat it cannot open is forgotten.
    fn show_location(&mut self, location: Location, ctx: &egui::Context) -> bool {
        let wanted = location.chat.clone();
        // The chat first: opening one lands on the chat list, and the page
        // below puts the reader back on the page the place recorded.
        if self.open_chat != location.chat {
            // A place whose chat is gone closes the current chat instead.
            match location.chat {
                Some(chat) if self.chat(&chat).is_some() => self.apply(Action::OpenChat(chat), ctx),
                _ => self.apply(Action::CloseChat, ctx),
            }
        }
        if self.page != location.page {
            self.apply(Action::Open(location.page), ctx);
        }
        // A chat that is gone, or locked outside its folder, cannot be opened.
        // Keeping its place would make every back step land on it again and
        // replace the forward history.
        if let Some(chat) = wanted
            && self.open_chat.as_deref() != Some(chat.as_str())
        {
            self.nav_history.forget(&chat);
            return false;
        }
        true
    }

    /// Cancels dialog-owned lookups while preserving shared-contact requests.
    fn cancel_contact_dialog_request(&mut self) {
        if self.new_contact_dialog_request.take().is_some() {
            self.new_contact_request = None;
            self.new_contact_pending = false;
        }
    }

    /// Opens an eligible chat, parking the previous draft and restoring this chat's composer.
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
        self.reset_notification_throttle(&id);
        if self.open_chat.as_deref() != Some(id.as_str()) {
            self.invalidate_composer_undo();
            self.reaction_target = None;
            self.reaction_anchor = None;
            self.reaction_from_list = false;
            // A menu asked for in the chat being left is not opened later.
            self.open_message_menu = None;
            self.composer_tools_open = false;
            self.emoji_jump = None;
            self.jump_highlight = None;
            if let Some(previous) = self.open_chat.take() {
                self.save_attachment_draft(&previous);
                let draft = std::mem::take(&mut self.composer);
                // Text a link brought, and the reader never changed, is not
                // their draft: the one they wrote stays theirs. An edit of it
                // is theirs, and is kept as any other text is.
                let untouched_prefill = self.is_untouched_prefill(&previous, &draft);
                self.prefilled = None;
                // Discard an unfinished edit instead of keeping it as a draft.
                if self.editing.take().is_some() || draft.trim().is_empty() {
                    self.drafts.remove(&previous);
                    self.draft_mentions.remove(&previous);
                    self.composer_mentions.clear();
                } else if !untouched_prefill {
                    self.drafts.insert(previous.clone(), draft);
                    let mentions = std::mem::take(&mut self.composer_mentions);
                    self.draft_mentions.insert(previous.clone(), mentions);
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
            self.open_draft_priority = self.parked_draft(&id);
            self.composer = self.drafts.remove(&id).unwrap_or_default();
            self.prefilled = None;
            self.composer_mentions = self.draft_mentions.remove(&id).unwrap_or_default();
            let attachments = self.attachment_drafts.remove(&id).unwrap_or_default();
            self.pending = attachments.pending;
            self.pending_view_once = attachments.view_once;
            // A search belongs to the chat it was typed in.
            self.close_chat_search();
            self.close_info();
            self.reply_to = attachments.quoting;
            self.editing = None;
            // A starred row's pending quote belongs to its own chat too.
            if self
                .reply_when_loaded
                .as_ref()
                .is_some_and(|(chat, _)| chat != &id)
            {
                self.reply_when_loaded = None;
            }
            // A run of voice messages belongs to the chat it started in.
            self.voice_chat = None;
            self.voice_wanted = None;
            // So does a replied message still loading.
            self.pending_follow = None;
            // A page request belongs to the chat it was pressed in.
            self.scroll_page = None;
            if let Some(conversation) = self.conversations.get_mut(&id) {
                conversation.key_scroll = None;
            }
        }
        self.emoji_start = None;
        self.mention_start = None;
        self.set_open_chat(Some(id.clone()));
        // The Calls tab keeps its history beside the chat; every other page
        // gives way to the chat list.
        if self.page != Page::Calls {
            self.page = Page::Chats;
        }
        self.scroll_to_bottom = true;
        self.at_bottom = true;
        self.focus_composer = true;
        self.ensure_loaded(&id);
        // Read again on every open: a pin that expired while the chat was
        // closed leaves the line under the header.
        self.backend.send(Command::LoadPins(id.clone()));
        if self
            .conversations
            .get(&id)
            .is_some_and(|conversation| conversation.complete && conversation.messages.is_empty())
        {
            self.fetch_older(&id, false);
        }
        if self
            .chat(&id)
            .is_some_and(|chat| chat.unread > 0 || chat.marked_unread)
        {
            self.mark_read(&id);
        }
        if self.settings.last_chat.as_deref() != Some(id.as_str()) {
            self.settings.last_chat = Some(id.clone());
            self.mark_settings_dirty();
        }
        if self.account().settings.last_chat.as_deref() != Some(id.as_str()) {
            self.account_mut().settings.last_chat = Some(id);
            self.account_mut().mark_settings_dirty();
        }
    }

    /// Returns keyboard focus to the open conversation when no search or
    /// overlay is active.
    fn refocus_composer(&mut self, ctx: &egui::Context) {
        let search_focused = ctx.memory(|memory| memory.has_focus(egui::Id::new("chat-search")));
        if self.chat_view()
            && self.dialog.is_none()
            && self.picker.is_none()
            && self.reaction_target.is_none()
            && self.recording.is_none()
            && self.open_chat.is_some()
            && self.search.trim().is_empty()
            && !self.focus_search
            && !search_focused
            // A reader in the message list stays there: coming back to the
            // window (Alt+Tab, a screen reader dialog) must not lose their
            // place. Still last frame's value here.
            && !self.messages_focused
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
            && self.account().settings.send_typing
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

    /// Scrolls the open chat to its true bottom, below the newest message
    /// and anything drawn under it, such as the typing bubble, exactly as
    /// the return-to-bottom button's click does (`Action::ScrollToBottom`):
    /// the button is out of reach of the keys and of a screen reader, so the
    /// list's cursor landing on the newest message does it instead. Short of
    /// it, the button would stay drawn over that message, and new messages
    /// would not be followed.
    fn scroll_to_newest(&mut self) {
        self.scroll_to_bottom = true;
        self.scroll_to_bottom_forced = true;
        self.cursor_followed = self
            .open_chat
            .clone()
            .and_then(|chat| Some((chat.clone(), self.message_cursor.get(&chat)?.clone())));
    }

    /// Whether the open chat's message list stands on its newest message,
    /// as the list resolves its cursor: one naming no loaded message stands
    /// there too.
    fn cursor_on_newest(&self) -> bool {
        let Some(chat) = self.open_chat.as_ref() else {
            return false;
        };
        let Some(conversation) = self.conversations.get(chat) else {
            return false;
        };
        let messages = &conversation.messages;
        // The newest row, which may be an album of several messages.
        let starts = crate::ui::conversation::row_starts(messages, self.selecting_in(chat));
        let Some(&newest) = starts.last() else {
            return false;
        };
        self.message_cursor.get(chat).is_none_or(|id| {
            messages
                .iter()
                .position(|message| &message.id == id)
                .is_none_or(|index| index >= newest)
        })
    }

    /// Whether messages are being selected in `chat`, which draws an album's
    /// items each in a row of its own.
    pub fn selecting_in(&self, chat: &str) -> bool {
        self.selection
            .as_ref()
            .is_some_and(|(selected, _)| selected == chat)
    }

    /// Keeps following outgoing messages only when the reader was already at
    /// the newest edge. Sending from older history must not lose their place.
    fn follow_outgoing(&mut self) {
        self.follow_sent_chat();
        if self.at_bottom {
            self.scroll_to_bottom = true;
        }
    }

    /// Asks the chat list to follow a sent message's chat to the top. The list
    /// goes only if it was near the top already: a reader working down through
    /// older chats keeps their place.
    fn follow_sent_chat(&mut self) {
        if !self.favorites_order() {
            self.scroll_chats_to_top = true;
        }
    }

    /// Sends the spot the dialog read. A refusal brings the dialog back with
    /// the spot in it, since the composer cannot hold one.
    fn send_location(
        &mut self,
        chat: ChatId,
        latitude: f64,
        longitude: f64,
        quoting: Option<crate::model::ReplyTarget>,
    ) {
        self.backend.send(Command::SendLocation {
            chat,
            latitude,
            longitude,
            quoting,
        });
        self.follow_outgoing();
    }

    /// Dispatches composed text with its explicit quote target and restores it on refusal.
    fn send_text(
        &mut self,
        chat: ChatId,
        text: String,
        quoting: Option<crate::model::ReplyTarget>,
    ) {
        let text = text.trim().to_owned();
        if text.is_empty() {
            return;
        }
        if self.picker == Some(PickerTab::Emoji) {
            self.picker = None;
        }
        let (text, mentions) = self.encode_composer_mentions(&chat, text);
        let mention_everyone = self.chat(&chat).is_some_and(Chat::can_mention_everyone)
            && crate::mentions::has_everyone_token(&text);
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
                if mention_everyone {
                    crate::mentions::push_everyone(&mut message.mentions);
                }
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
        self.follow_outgoing();
    }

    /// Snapshots selected mentions before the view queues an attachment send.
    pub(crate) fn composer_mentions(&self) -> Vec<ComposerMention> {
        self.composer_mentions.clone()
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
        // A localized word is only a way to type the mention. The body that
        // goes out, and the bubble, keep the visible `@all` token. When this
        // chat cannot mention everyone, the typed word stays ordinary text.
        if self.chat(chat).is_some_and(Chat::can_mention_everyone) {
            let alias = crate::mentions::everyone_alias(self.locale);
            crate::mentions::rewrite_alias(&mut text, &alias);
        }
        (text, mentions)
    }

    /// Parks staged attachments, and the reply they carry, before leaving a
    /// chat or account. A reply without attachments is not kept, as before,
    /// except a private reply: its quote comes from a group and cannot be
    /// picked again in this chat.
    fn save_attachment_draft(&mut self, chat: &str) {
        let private = self
            .reply_to
            .as_ref()
            .is_some_and(|target| target.chat.is_some());
        if self.pending.is_empty() && !private {
            self.attachment_drafts.remove(chat);
        } else {
            let draft = AttachmentDraft {
                pending: std::mem::take(&mut self.pending),
                view_once: self.pending_view_once,
                quoting: self.reply_to.take(),
            };
            self.attachment_drafts.insert(chat.to_owned(), draft);
        }
        self.pending_view_once = false;
    }

    /// Adds files to the open chat's composer.
    fn stage_files(&mut self, paths: Vec<PathBuf>) {
        self.stage_all(paths.into_iter().map(Pending::File));
    }

    /// Adds files to the open chat's composer to go as documents.
    fn stage_documents(&mut self, paths: Vec<PathBuf>) {
        self.stage_all(paths.into_iter().map(Pending::Document));
    }

    fn stage_all(&mut self, items: impl IntoIterator<Item = Pending>) {
        if self.open_chat.is_none() {
            self.toast_error("Open a chat first");
            return;
        }
        for item in items {
            self.stage(item);
        }
        self.focus_composer = true;
    }

    /// Sends pending files, attaching the caption to the first.
    fn send_pending(
        &mut self,
        account: AccountId,
        chat: ChatId,
        caption: String,
        selected: Vec<ComposerMention>,
    ) {
        // A queued action cannot consume another chat's or account's draft.
        let active = self.account().id.clone();
        let Some(target) = self.accounts.iter_mut().find(|known| known.id == account) else {
            return;
        };
        if account != active || target.open_chat.as_deref() != Some(chat.as_str()) {
            if target
                .attachment_drafts
                .get(&chat)
                .is_some_and(|draft| !draft.pending.is_empty())
                && !caption.trim().is_empty()
                && target
                    .drafts
                    .get(&chat)
                    .is_none_or(|text| text.trim().is_empty())
            {
                target.backend.send(Command::SaveDraft {
                    chat: chat.clone(),
                    text: caption.clone(),
                });
                target.drafts.insert(chat.clone(), caption);
                target.draft_mentions.insert(chat, selected);
            }
            return;
        }
        if self.pending.is_empty() {
            return;
        }
        if self.pending_view_once && !self.pending_supports_view_once() {
            let restore_mentions = self.composer.trim().is_empty() && self.editing.is_none();
            self.restore_text(&chat, caption);
            if restore_mentions {
                self.composer_mentions = selected;
            }
            self.toast_error(
                crate::i18n::gettext(
                    self.locale,
                    "Not sent: view-once mode is available for one photo at a time.",
                )
                .into_owned(),
            );
            return;
        }
        let view_once = self.pending_view_once;
        self.pending_view_once = false;
        self.drafts.remove(&chat);
        self.draft_mentions.remove(&chat);
        self.store_draft(&chat, "");
        self.composer_mentions = selected;
        // The reply travels with the first attachment, like the caption.
        let mut quoting = self.reply_to.take();
        let caption = caption.trim().to_owned();
        let (caption, mentions) = self.encode_composer_mentions(&chat, caption);
        let caption = Some(caption).filter(|text| !text.is_empty());
        let mut caption = caption;
        let mut mentions = mentions;
        self.emoji_start = None;
        self.mention_start = None;
        // Everything goes in the order it was staged, files in runs of one
        // kind; a picture between files splits the run around it.
        enum Send {
            Picture {
                width: u32,
                height: u32,
                rgba: Vec<u8>,
            },
            Files {
                documents: bool,
                paths: Vec<PathBuf>,
            },
        }
        let mut sends: Vec<Send> = Vec::new();
        for staged in std::mem::take(&mut self.pending) {
            let (documents, path) = match staged.item {
                Pending::Picture {
                    width,
                    height,
                    rgba,
                    ..
                } => {
                    sends.push(Send::Picture {
                        width: width as u32,
                        height: height as u32,
                        rgba: std::sync::Arc::try_unwrap(rgba).unwrap_or_else(|arc| (*arc).clone()),
                    });
                    continue;
                }
                Pending::File(path) => (false, path),
                Pending::Document(path) => (true, path),
            };
            match sends.last_mut() {
                Some(Send::Files {
                    documents: kind,
                    paths,
                }) if *kind == documents => paths.push(path),
                _ => sends.push(Send::Files {
                    documents,
                    paths: vec![path],
                }),
            }
        }
        for send in sends {
            let chat = chat.clone();
            let caption = caption.take();
            let mentions = std::mem::take(&mut mentions);
            let quoting = quoting.take();
            self.backend.send(match send {
                Send::Picture {
                    width,
                    height,
                    rgba,
                } => Command::SendImage {
                    chat,
                    width,
                    height,
                    rgba,
                    view_once,
                    caption,
                    mentions,
                    quoting,
                },
                Send::Files { documents, paths } => Command::SendFiles {
                    chat,
                    paths,
                    documents,
                    view_once,
                    caption,
                    mentions,
                    quoting,
                },
            });
        }
        // Everything staged has gone, so nothing is remembered about it.
        self.forget_unstaged_pictures();
        self.follow_outgoing();
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
            documents: false,
            view_once: false,
            caption: None,
            mentions: Vec::new(),
            quoting: None,
        });
        self.follow_outgoing();
    }

    fn tick(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        self.tick_call(ctx);
        if self.composing
            && let Some(last) = self.last_keystroke
            && now.duration_since(last) > COMPOSING_TIMEOUT
            && let Some(chat) = self.open_chat.clone()
        {
            self.stop_composing(&chat);
        }
        for typers in self.typing.values_mut() {
            typers.retain(|(_, _, since)| now.duration_since(*since) < TYPING_TIMEOUT);
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
        // The chat list's search scans the archive once the reader stops
        // typing: a whole-history scan takes seconds, so one per word rather
        // than one per letter.
        if let Some(due) = self.search_due {
            let left = due.saturating_duration_since(now);
            if left.is_zero() {
                self.request_search();
            } else {
                ctx.request_repaint_after(left);
            }
        }
        for account in &mut self.accounts {
            if account.settings_dirty {
                account.save_settings(&self.files);
            }
        }
        if !self.typing.is_empty() || self.composing {
            ctx.request_repaint_after(Duration::from_secs(1));
        }
        // A pin that has run out its seven days is no longer on the message,
        // and the worker only speaks when a pin changes. Drop it from the
        // cache the footer and the message menu read, so an expired pin stops
        // painting its mark and stops offering Unpin.
        let stamp = crate::util::now();
        let account = &mut **self;
        for (chat, rows) in &account.chat_pins {
            let Some(ids) = account.pins.get_mut(chat) else {
                continue;
            };
            ids.retain(|id| {
                rows.iter()
                    .any(|row| row.id == *id && row.expires_at > stamp)
            });
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

    /// Tells each account's worker whether it may fetch older history in the
    /// background, and which chat is on screen so it goes first. Only the
    /// account on screen has one: a hidden account's remembered chat is not
    /// being read. Sent only when it changes, so it can run every frame and
    /// follow every way a chat opens, closes or disappears.
    fn sync_prefetch(&mut self) {
        let active = self.active;
        for (index, account) in self.accounts.iter_mut().enumerate() {
            let wanted = (
                account.settings.history_prefetch,
                (index == active)
                    .then(|| account.open_chat.clone())
                    .flatten(),
            );
            if account.prefetch_sent.as_ref() != Some(&wanted) {
                account.backend.send(Command::SetHistoryPrefetch {
                    on: wanted.0,
                    focused: wanted.1.clone(),
                });
                account.prefetch_sent = Some(wanted);
            }
        }
    }

    fn save_settings(&mut self) {
        self.settings_dirty = false;
        self.last_settings_save = Instant::now();
        if let Err(error) = self
            .settings
            .save_to(&self.files, &self.dirs.settings_file())
        {
            log::warn!("could not save settings: {error}");
        }
    }

    /// Remembers the window's size, position and maximized state in the
    /// settings, in logical pixels, so a window saved while zoomed opens at
    /// the same size; see [`fastframe_shell::window::Geometry`].
    fn sync_window_state(&mut self, ctx: &egui::Context) {
        let mut geometry = self.settings.window_geometry();
        if geometry.remember(ctx) {
            self.settings.set_window_geometry(geometry);
            self.mark_settings_dirty();
        }
    }

    /// What must happen in every pass of an open window, including the
    /// passes eframe runs without `ui` for a minimized or covered one. Call it
    /// from `eframe::App::logic`, after [`Self::background_frame`].
    pub fn window_logic(&mut self, ctx: &egui::Context) {
        self.sync_window_state(ctx);
        // Close the window and continue headless when background mode is
        // enabled. Decided here rather than in `frame_ui`, which eframe skips
        // for a minimized window: closing one from the taskbar quit instead.
        if ctx.input(|input| input.viewport().close_requested())
            && !self.quit_requested
            && self.hides_to_tray()
        {
            self.hide_intent = true;
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
                // Messages follows the system's appearance, as Messages does.
                ThemeChoice::System | ThemeChoice::Messages => egui::ThemePreference::System,
            },
            |palette| {
                if palette.dark {
                    egui::ThemePreference::Dark
                } else {
                    egui::ThemePreference::Light
                }
            },
        );
        if !self.zoom_applied {
            ctx.set_zoom_factor(self.settings.zoom);
            self.zoom_applied = true;
        }
        // winit reports no system theme on Linux, so "Follow system" falls
        // back to what the desktop portal says, for our palette and egui's
        // own controls alike.
        #[cfg(target_os = "linux")]
        if let Some(dark) = self
            .system_appearance
            .as_ref()
            .and_then(crate::appearance::SystemAppearance::dark)
        {
            let theme = if dark {
                egui::Theme::Dark
            } else {
                egui::Theme::Light
            };
            if ctx.options(|options| options.fallback_theme) != theme {
                ctx.options_mut(|options| options.fallback_theme = theme);
            }
        }
        // Resolved here rather than after `set_theme`, so a change can keep the
        // old colours, egui's own controls included, while it is revealed.
        let dark = match preference {
            egui::ThemePreference::Dark => true,
            egui::ThemePreference::Light => false,
            egui::ThemePreference::System => {
                ctx.system_theme()
                    .unwrap_or_else(|| ctx.options(|options| options.fallback_theme))
                    == egui::Theme::Dark
            }
        };
        let palette = self.settings.cached_palette().unwrap_or_else(|| {
            if self.settings.theme == ThemeChoice::Messages {
                if dark {
                    Palette::messages()
                } else {
                    Palette::messages_light()
                }
            } else if dark {
                Palette::dark()
            } else {
                Palette::light()
            }
        });
        if crate::theme::apply_text_rendering_change(ctx) {
            self.applied_dark = None;
        }
        // A change of colours after the window's first is revealed from the
        // middle outwards, as Omarchy does; the old palette stays until the
        // window's picture of it arrives.
        // A vibrant window's picture misses the desktop behind it, so changes
        // to or from one switch at once.
        if self.applied_dark.is_some()
            && self.palette != palette
            && self.reveal_theme_changes
            && !self.palette.vibrant()
            && !palette.vibrant()
        {
            self.theme_transition.begin(ctx);
            if self.theme_transition.holding(ctx) {
                return;
            }
        }
        // Use the same preference for our palette and egui's native controls.
        ctx.set_theme(preference);
        if self.applied_dark.is_none() || self.palette != palette {
            self.palette = palette;
            crate::theme::apply(ctx, &self.palette);
            self.applied_dark = Some(dark);
        }
    }

    fn apply_actions(&mut self, ctx: &egui::Context) {
        let deferred = std::mem::take(&mut self.deferred_account_actions);
        for (id, action) in deferred {
            let Some(index) = self.accounts.iter().position(|account| account.id == id) else {
                continue;
            };
            let previous = self.active;
            self.active = index;
            self.apply(action, ctx);
            self.active = previous;
        }
        let mut actions = std::mem::take(&mut self.actions);
        while !actions.is_empty() {
            for action in actions.drain(..) {
                self.apply(action, ctx);
                self.record_location();
            }
            actions = std::mem::take(&mut self.actions);
        }
        // A frame with no queued action still fixes the opening place, so the
        // first navigation has somewhere to return to.
        self.record_location();
    }

    /// Applies one model action after drawing the views, respecting the app lock and account state.
    fn apply(&mut self, action: Action, ctx: &egui::Context) {
        if self.app_lock.is_locked() && !allowed_while_locked(&action) {
            // A clicked notification opens its message once unlocked, and a
            // link opens the chat it named; the rest would show or change
            // what the lock hides.
            match action {
                Action::OpenMessage { chat, message } => {
                    let account = self.account().id.clone();
                    self.app_lock.defer(crate::app_lock::Deferred::Message(
                        crate::notify::NotificationTarget {
                            account,
                            chat,
                            message: Some(message),
                        },
                    ));
                }
                Action::StartChat { id, .. } => {
                    // The name is the number, which the chat carries, so
                    // nothing extra has to be kept for the replay.
                    self.app_lock
                        .defer(crate::app_lock::Deferred::Chat(crate::target::Request {
                            chat: id,
                            text: None,
                        }));
                }
                Action::OpenUrl(url) => {
                    // Only an invite link is held: it is what a launch
                    // argument may carry. A link from a message stays
                    // unreachable while locked.
                    if let Some(code) = crate::safety::group_invite_code(&url) {
                        self.app_lock.defer(crate::app_lock::Deferred::Invite(code));
                    }
                }
                Action::PrefillComposer { chat, text } => {
                    // The chat it belongs to arrives in the request the
                    // StartChat above is holding; a template without its
                    // chat has nowhere to go.
                    self.app_lock.defer_text(&chat, text);
                }
                _ => {}
            }
            return;
        }
        let Some(action) = self.apply_call_action(action, ctx) else {
            return;
        };
        let reply_send = match &action {
            Action::SendText { chat, quoting, .. } | Action::SendLocation { chat, quoting, .. } => {
                Some((chat.as_str(), quoting.as_ref()))
            }
            Action::SendPending { chat, .. } => Some((chat.as_str(), self.reply_to.as_ref())),
            Action::SendGif(_) | Action::SendSticker(_) | Action::MakeSticker { send: true } => {
                self.open_chat
                    .as_deref()
                    .map(|chat| (chat, self.reply_to.as_ref()))
            }
            _ => None,
        };
        if let Some((chat, Some(target))) = reply_send
            && !self.reply_source_visible(target, chat)
        {
            let chat = chat.to_owned();
            let target = target.clone();
            let unsent = match &action {
                Action::SendText { text, .. } => Unsent::Text(text.clone()),
                Action::SendPending { caption, .. } => Unsent::Text(caption.clone()),
                // The dialog comes back with the spot, as for any refusal.
                Action::SendLocation {
                    latitude,
                    longitude,
                    ..
                } => Unsent::Location {
                    latitude: *latitude,
                    longitude: *longitude,
                },
                _ => Unsent::Text(String::new()),
            };
            self.send_refused(chat, Some(target), unsent, Refusal::QuoteUnavailable);
            return;
        }
        match action {
            Action::Open(page) => {
                let opens_chats = page == Page::Chats;
                // Privacy can change on the phone at any time, and nothing
                // announces it: read it again whenever Settings opens.
                if page == Page::Settings && self.page != Page::Settings && self.is_connected() {
                    self.backend.send(Command::FetchAccountPrivacy);
                }
                // The cache grows with every download: measure it again
                // whenever Settings opens.
                if page == Page::Settings && self.page != Page::Settings {
                    self.storage_token += 1;
                    let token = self.storage_token;
                    self.backend.send(Command::LoadStorageStats { token });
                }
                // The Calls page reads the account's calls as it opens, and
                // opens on the call history rather than the new-call list.
                if page == Page::Calls {
                    self.backend.send(Command::LoadCallLog);
                    if self.page != Page::Calls {
                        self.new_call = false;
                        self.picked_call = None;
                    }
                    // The rail's destinations take turns: Calls replaces the
                    // starred messages as it replaces the chats.
                    self.show_starred = false;
                }
                // Typed passwords do not wait in a form nobody sees.
                if page != Page::Settings && !self.app_lock.checking() {
                    self.app_lock.form = None;
                }
                if matches!(self.dialog, Some(Dialog::NewContact)) {
                    self.cancel_contact_dialog_request();
                }
                if self.page != page {
                    // A menu asked for on the page being left is not opened
                    // later.
                    self.open_message_menu = None;
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
            Action::ToggleSettings => {
                // The button that opened settings closes them again, and
                // closing lands on what was showing, exactly as Escape does.
                let page = if self.page == Page::Settings {
                    Page::Chats
                } else {
                    Page::Settings
                };
                self.apply(Action::Open(page), ctx);
            }
            Action::OpenChat(id) => {
                // A chat where someone mentioned us opens at the mention, as
                // on WhatsApp, rather than at the first unread message.
                let mention = self
                    .chat(&id)
                    .filter(|_| self.open_chat.as_deref() != Some(id.as_str()))
                    .and_then(|chat| chat.unread_mention.clone());
                match mention {
                    Some(message) => self.apply(Action::OpenMessage { chat: id, message }, ctx),
                    None => self.open_chat(id),
                }
            }
            // Applied by `apply_call_action` before anything else.
            Action::StartCall(_)
            | Action::AnswerCall
            | Action::DeclineCall
            | Action::HangupCall
            | Action::StartVideoCall(_)
            | Action::SetCallMuted(_)
            | Action::SetCallCamera(_)
            | Action::SetCallMicrophone(_)
            | Action::SetCallSpeaker(_)
            | Action::SetCallCameraDevice(_)
            | Action::LeaveCallSurface
            | Action::ReturnToCall
            | Action::ToggleCallFullscreen => {}
            Action::PickCall { chat, message } => {
                self.new_call = false;
                self.picked_call = Some((chat.clone(), message.clone()));
                if self.chat(&chat).is_some() {
                    self.apply(Action::OpenMessage { chat, message }, ctx);
                } else {
                    // A call from the phone's log with nobody we chat with.
                    let name = self.display_name(&chat);
                    self.apply(
                        Action::StartChat {
                            id: chat,
                            name,
                            dismiss_dialog: false,
                        },
                        ctx,
                    );
                }
            }
            Action::ShowNewCall(open) => {
                self.new_call = open;
                self.new_call_search.clear();
            }
            Action::NavigateBack => self.navigate_back(ctx),
            Action::NavigateForward => self.navigate_forward(ctx),
            Action::StartChat {
                id,
                name,
                dismiss_dialog,
            } => {
                if self.chat(&id).is_none() {
                    self.chats.push(Chat::new(id.clone(), name.clone()));
                    self.backend.send(Command::EnsureChat {
                        chat: id.clone(),
                        name,
                    });
                }
                self.open_chat(id);
                if dismiss_dialog {
                    self.dialog = None;
                }
            }
            Action::PrefillComposer { chat, text } => {
                // The composer belongs to whoever is reading: a template is
                // only a start, and only where nothing is already written.
                if self.open_chat.as_deref() != Some(chat.as_str()) {
                    // The chat was not opened. `open_chat` refuses a locked
                    // chat outside its folder, and the reader is still in
                    // another one: its text must not go there.
                    return;
                }
                if self.editing.is_none()
                    && self.composer.trim().is_empty()
                    && self.recording.is_none()
                    && self.reply_to.is_none()
                    && !self.drafts.contains_key(&chat)
                {
                    self.composer = text.clone();
                    self.prefilled = Some((chat, text));
                    self.composer_mentions.clear();
                    self.emoji_start = None;
                    self.mention_start = None;
                    self.focus_composer = true;
                }
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
                                dismiss_dialog: true,
                            },
                            ctx,
                        );
                    }
                }
            }
            Action::OpenMessage { chat, message } => {
                // A stored cross-chat quote may outlive its locally deleted source.
                if self.chat(&chat).is_none() {
                    return;
                }
                // A result picked in the search pane keeps the keyboard in
                // the pane, so the arrows can walk on to the next one.
                let from_pane =
                    self.chat_search_visible() && self.open_chat.as_deref() == Some(chat.as_str());
                // So does a reader in this chat's message list: the list
                // follows to the message instead (`follow_message` below).
                let from_list =
                    self.messages_focused && self.open_chat.as_deref() == Some(chat.as_str());
                self.open_chat(chat.clone());
                // A locked chat stays shut outside its folder, even for a
                // notification clicked before the chat was locked.
                if self.open_chat.as_deref() != Some(chat.as_str()) {
                    return;
                }
                if from_pane || from_list {
                    self.focus_composer = false;
                }
                // Keep the search result, not the chat end, in view.
                self.scroll_to_bottom = false;
                self.at_bottom = false;
                self.scroll_anchor = Some(message.clone());
                self.jump_highlight = Some(JumpHighlight::new(chat.clone(), message.clone()));
                self.follow_message(&chat, &message);
                let load = {
                    let conversation = self.conversations.entry(chat.clone()).or_default();
                    if conversation.message(&message).is_none() && !conversation.loading_older {
                        conversation.messages.first().map(|oldest| {
                            conversation.loading_older = true;
                            (oldest.timestamp, oldest.id.clone())
                        })
                    } else {
                        None
                    }
                };
                if let Some(before) = load {
                    self.backend.send(Command::LoadUntil {
                        chat,
                        id: message,
                        before,
                    });
                }
            }
            Action::CloseChat => {
                self.close_info();
                // A narrow window's Back from a picked call returns to the
                // call history.
                self.picked_call = None;
                self.open_message_menu = None;
                if let Some(chat) = self.open_chat.take() {
                    self.invalidate_composer_undo();
                    self.save_attachment_draft(&chat);
                    self.stop_composing(&chat);
                    let draft = std::mem::take(&mut self.composer);
                    if self.is_untouched_prefill(&chat, &draft) {
                        // A link's text is not the reader's; theirs stays.
                        self.composer_mentions.clear();
                    } else if self.editing.take().is_none() && !draft.trim().is_empty() {
                        self.drafts.insert(chat.clone(), draft);
                        let mentions = std::mem::take(&mut self.composer_mentions);
                        self.draft_mentions.insert(chat.clone(), mentions);
                    } else {
                        self.composer_mentions.clear();
                    }
                    self.store_draft(&chat, self.drafts.get(&chat).map_or("", String::as_str));
                }
                self.reply_to = None;
                // A pending quote belongs to the chat being left, like the
                // quote it would start.
                self.reply_when_loaded = None;
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
            Action::SendLocation {
                chat,
                latitude,
                longitude,
                quoting,
            } => {
                self.send_location(chat, latitude, longitude, quoting);
                self.reply_to = None;
                self.location_draft.clear();
                self.dialog = None;
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
                    self.follow_outgoing();
                }
            }
            Action::CreatePoll { chat, draft } => {
                if !self.poll_creating {
                    match draft.validated() {
                        Ok(draft) => {
                            self.poll_creating = true;
                            self.backend.send(Command::CreatePoll { chat, draft });
                            self.follow_sent_chat();
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
            Action::LoadOlder { chat, explicit } => self.load_older(&chat, explicit),
            Action::FetchOlder(chat) => self.fetch_older(&chat, true),
            Action::ReloadHistory { chat, message } => {
                if !self.is_connected() {
                    return;
                }
                let Some(conversation) = self.conversations.get_mut(&chat) else {
                    return;
                };
                if conversation.fetching_phone {
                    self.toast("History is already being requested for this chat");
                    return;
                }
                if conversation.message(&message).is_none() {
                    return;
                }
                conversation.fetching_phone = true;
                conversation.phone_explicit = true;
                self.backend.send(Command::ReloadHistory { chat, message });
                self.toast("Requesting earlier messages from your phone");
            }
            Action::ToggleMotion { chat, message } => {
                let playing = (chat, message);
                if self.motion_playing.as_ref() == Some(&playing) {
                    self.motion_playing = None;
                    return;
                }
                let (chat, message) = playing.clone();
                self.motion_playing = Some(playing);
                if self
                    .motion_mut(&chat, &message)
                    .is_some_and(|motion| motion.path.is_none())
                {
                    self.actions.push(Action::DownloadMotion { chat, message });
                }
            }
            Action::DownloadMotion { chat, message } => {
                let Some(motion) = self.motion_mut(&chat, &message) else {
                    return;
                };
                if matches!(motion.state, MediaState::Downloading) {
                    return;
                }
                motion.state = MediaState::Downloading;
                self.backend.send(Command::DownloadMotion { chat, message });
            }
            Action::Download {
                manual,
                card,
                chat,
                message,
            } => {
                // The viewer browses the album the archive holds, which reaches
                // older messages than the loaded transcript: the message may be
                // absent from `conversations` and still be in the archive, where
                // the worker reads the download keys. Only the state drawn in
                // the chat needs the message here, so it is updated when it is
                // present, and the command goes out either way.
                if let Some(media) = self
                    .conversations
                    .get_mut(&chat)
                    .and_then(|conversation| conversation.message_mut(&message))
                    .and_then(|message| message.content.media_at_mut(card))
                {
                    // The limit spares the disk and the link from what nobody
                    // asked for; a file the reader clicked downloads whole.
                    if !manual && !media.is_within_download_limit() {
                        let notice =
                            "This attachment is larger than the 64 MiB automatic download limit";
                        media.state = MediaState::Failed(notice.into());
                        self.viewer_media_state
                            .insert(message.clone(), MediaState::Failed(notice.into()));
                        return;
                    }
                    if matches!(media.state, MediaState::Downloading) {
                        self.viewer_media_state
                            .insert(message.clone(), MediaState::Downloading);
                        return;
                    }
                    media.state = MediaState::Downloading;
                }
                self.viewer_media_state
                    .insert(message.clone(), MediaState::Downloading);
                self.backend.send(Command::Download {
                    manual,
                    card,
                    chat,
                    message,
                });
            }
            Action::UnreadableMedia {
                chat,
                message,
                path,
            } => {
                // The file may come back under the same name, which must be
                // decoded afresh rather than answered with this failure.
                crate::animation::forget(ctx, &path);
                crate::image_cache::forget(ctx, &crate::util::image_uri(&path));
                let Some(media) = self
                    .conversations
                    .get_mut(&chat)
                    .and_then(|conversation| conversation.message_mut(&message))
                    .and_then(|message| message.content.media_at_mut(None))
                    .filter(|media| media.path.as_ref() == Some(&path))
                else {
                    return;
                };
                media.path = None;
                if path.is_file() {
                    // A file that does not decode is not fetched again on its
                    // own, or one that never decodes would download forever:
                    // the bubble offers the retry, as a failed download does.
                    log::warn!("a downloaded attachment could not be displayed");
                    media.state = MediaState::Failed("ZapFast could not display this file".into());
                } else if !matches!(media.state, MediaState::Downloading) {
                    // Gone from the disk: it downloads again like any
                    // attachment that was never downloaded.
                    media.state = MediaState::Idle;
                }
            }
            Action::PreviewImage {
                path,
                chat,
                message,
                card,
            } => {
                if crate::image_preview::can_view(&path) && path.is_file() {
                    self.open_viewer(
                        PreviewState::new(Some(path), chat, message).with_card(card),
                        ctx,
                    );
                } else {
                    self.actions.push(Action::OpenFile(path));
                }
            }
            Action::ZoomImageBy(factor) => {
                if let Some(preview) = &mut self.image_preview {
                    preview.zoom_by(factor);
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
                // The viewer owned the clip on screen, so leaving it releases
                // the player: a clip that kept playing had its sound over the
                // chat, and over the next picture opened from the strip.
                self.video.stop();
                self.video.set_expanded(false);
                self.image_preview = None;
                self.viewer_media.clear();
                self.viewer_media_chat = None;
                self.viewer_media_state.clear();
                self.refocus_composer(ctx);
            }
            Action::PreviewSticker(path) => {
                if path.is_file() {
                    self.sticker_view = Some(crate::image_preview::StickerPreview::new(path));
                    self.dialog = None;
                    self.picker = None;
                    // As for the image preview: the composer must not take
                    // Enter in the frame before the modal is known, and
                    // anything else keeps focus until the card takes it when
                    // the keys or a screen reader opened it.
                    let composer = crate::ui::conversation::composer_id();
                    let followed = crate::ui::overlay::followed(ctx);
                    ctx.memory_mut(|memory| {
                        if let Some(focused) = memory.focused()
                            && (focused == composer || !followed)
                        {
                            memory.surrender_focus(focused);
                        }
                    });
                }
            }
            Action::CloseStickerPreview => {
                self.sticker_view = None;
                self.refocus_composer(ctx);
            }
            Action::ViewImage { message, card } => {
                if self.image_preview.is_none() {
                    return;
                }
                let Some(item) = self
                    .viewer_media
                    .iter()
                    .find(|item| item.id == message && item.card == card)
                    .cloned()
                else {
                    // The photo on screen, back from its motion clip, even
                    // when the album does not hold it.
                    if let Some(preview) = &mut self.image_preview
                        && preview.message() == message
                        && preview.motion().is_some()
                    {
                        self.video.stop();
                        let path = preview.path().map(std::path::Path::to_path_buf);
                        preview.show_item(path, message);
                    }
                    return;
                };
                // Stepping away from a clip that was playing stops it, and so
                // does going back from a motion clip to its photo.
                if self.video.message() != Some(item.id.as_str())
                    || self
                        .image_preview
                        .as_ref()
                        .is_some_and(|preview| preview.motion().is_some())
                {
                    self.video.stop();
                }
                // An item whose file is not here yet is shown for what it is
                // and offered for download. It is not shown as the picture
                // that happens to be on screen: that would be another file
                // under this item's name.
                let path = item.path.clone();
                if let Some(preview) = &mut self.image_preview {
                    preview.show_album_item(path, item.id.clone(), item.card);
                }
            }
            Action::ViewerStep(step) => {
                let Some(preview) = self.image_preview.as_ref() else {
                    return;
                };
                // A chat that is gone, or locked outside the open folder,
                // shows nothing more of its album: the viewer closes instead.
                if self
                    .chat(preview.chat())
                    .is_none_or(|chat| chat.locked && !self.locked_folder_open())
                {
                    self.apply(Action::CloseImagePreview, ctx);
                    return;
                }
                let Some(index) = preview.position(&self.viewer_media) else {
                    return;
                };
                // Stepping stops at either end of the album, as it always has.
                let Some(next) = index
                    .checked_add_signed(isize::from(step))
                    .filter(|next| *next < self.viewer_media.len())
                else {
                    return;
                };
                let item = self.viewer_media[next].clone();
                // Stepping away from a clip that was playing stops it, so its
                // sound does not carry over a photo.
                if self.video.message() != Some(item.id.as_str()) || preview.motion().is_some() {
                    self.video.stop();
                }
                // As above: a missing file keeps its own state.
                let path = item.path.clone();
                if let Some(preview) = &mut self.image_preview {
                    preview.show_album_item(path, item.id.clone(), item.card);
                }
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
            Action::OpenLog(path) => self.backend.send(Command::OpenLog(path)),
            Action::SaveAttachmentAs { path, name } => {
                self.backend
                    .send(Command::SaveAttachmentAs { source: path, name });
            }
            Action::SaveAttachments { files } => {
                self.backend.send(Command::SaveAttachments { files });
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
                if let Some((token, video)) = crate::safety::call_link(&url) {
                    if crate::calls::group_calls_enabled() {
                        self.join_call_link(token, video);
                    } else {
                        self.toast(crate::i18n::gettext(
                            self.locale,
                            "ZapFast can't join group calls yet: open the link on your phone.",
                        ));
                    }
                } else if let Some(code) = crate::safety::group_invite_code(&url) {
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
                    self.set_composer_cursor(ctx, 0);
                }
                self.reply_to = Some(id.clone().into());
                self.focus_composer = true;
                let loaded = self.open_chat.as_deref().is_some_and(|chat| {
                    self.conversations
                        .get(chat)
                        .is_some_and(|conversation| conversation.message(&id).is_some())
                });
                if !loaded && let Some(chat) = self.open_chat.clone() {
                    // A starred row can reply before its chat has paged to
                    // the message. The quote starts again once that page arrives.
                    self.reply_when_loaded = Some((chat, id));
                } else {
                    self.reply_when_loaded = None;
                }
            }
            Action::ReplyPrivately { chat, message } => {
                if self.open_chat.as_deref() != Some(chat.as_str())
                    || self
                        .chat(&chat)
                        .is_some_and(|source| source.locked && !self.locked_folder_open())
                {
                    return;
                }
                let Some(original) = self
                    .conversations
                    .get(&chat)
                    .and_then(|conversation| conversation.message(&message))
                    .cloned()
                else {
                    return;
                };
                let Some(recipient) = original.private_reply_recipient().map(str::to_owned) else {
                    return;
                };
                if self.chat(&recipient).is_some_and(|chat| {
                    chat.read_only || (chat.locked && !self.locked_folder_open())
                }) {
                    return;
                }
                let name = self.display_name_or(&recipient, original.sender_name.as_deref());
                self.apply(
                    Action::StartChat {
                        id: recipient.clone(),
                        name,
                        dismiss_dialog: true,
                    },
                    ctx,
                );
                if self.open_chat.as_deref() == Some(recipient.as_str()) {
                    self.reply_to = Some(crate::model::ReplyTarget {
                        id: message,
                        chat: Some(chat),
                    });
                    self.focus_composer = true;
                }
            }
            Action::CancelReply => {
                self.reply_to = None;
                self.reply_when_loaded = None;
            }
            Action::SetStar {
                chat,
                message,
                starred,
            } => {
                self.backend.send(Command::SetStar {
                    chat,
                    message,
                    starred,
                });
            }
            Action::ToggleForwardRecipient(id) => {
                if !matches!(self.dialog, Some(Dialog::Forward { .. }))
                    || self.forward_reviewing
                    || !self.chat(&id).is_some_and(Chat::can_send)
                {
                    return;
                }
                if self.forward_recipients.contains(&id) {
                    self.forward_recipients.retain(|recipient| recipient != &id);
                } else if self.forward_limit == 1 {
                    // A message that may go to one chat only: the new choice
                    // replaces the old one, as a radio button would.
                    self.forward_recipients = vec![id];
                } else if self.forward_recipients.len() < self.forward_limit {
                    self.forward_recipients.push(id);
                }
            }
            Action::ReviewForward => {
                if matches!(self.dialog, Some(Dialog::Forward { .. }))
                    && self.forward_recipients.len() > 1
                {
                    self.forward_reviewing = true;
                }
            }
            Action::BackToForwardSelection => self.forward_reviewing = false,
            Action::ConfirmForward => {
                let Some(Dialog::Forward { chat, messages }) = self.dialog.clone() else {
                    return;
                };
                // One recipient goes at once; several only from their
                // confirmation.
                let confirmed = match self.forward_recipients.len() {
                    0 => false,
                    1 => true,
                    _ => self.forward_reviewing,
                };
                if !confirmed
                    || messages.is_empty()
                    || self.forward_recipients.len() > self.forward_limit
                {
                    return;
                }
                // Recheck the complete selection before dispatch. A destination may
                // have become locked or read-only while the review was open.
                let writable: std::collections::HashSet<_> = self
                    .chats
                    .iter()
                    .filter(|chat| crate::ui::dialogs::forwardable(chat))
                    .map(|chat| chat.id.clone())
                    .collect();
                let count = self.forward_recipients.len();
                self.forward_recipients.retain(|id| writable.contains(id));
                if self.forward_recipients.len() != count {
                    self.forward_reviewing = false;
                    self.toast_error(
                        crate::i18n::gettext(self.locale, "A selected chat is no longer writable")
                            .into_owned(),
                    );
                    return;
                }
                let to_chats = std::mem::take(&mut self.forward_recipients);
                self.backend.send(Command::Forward {
                    from_chat: chat,
                    messages,
                    to_chats,
                });
                self.follow_sent_chat();
                self.dialog = None;
                self.forward_search.clear();
                self.forward_reviewing = false;
                self.selection = None;
            }
            Action::StartSelection => {
                // Choosing it again keeps what is already ticked.
                if let Some(chat) = self.open_chat.clone()
                    && self
                        .selection
                        .as_ref()
                        .is_none_or(|(selected, _)| *selected != chat)
                {
                    self.selection = Some((chat, Vec::new()));
                    self.selection_anchor = None;
                }
            }
            Action::SelectMessage(id) => {
                if let Some(chat) = self.open_chat.clone()
                    && self
                        .conversations
                        .get(&chat)
                        .and_then(|conversation| conversation.message(&id))
                        .is_some_and(|message| can_select(&message.content))
                {
                    self.selection = Some((chat, vec![id.clone()]));
                    self.selection_anchor = Some(id);
                }
            }
            Action::SelectRange(id) => {
                let Some((chat, ids)) = self.selection.clone() else {
                    return;
                };
                let Some(conversation) = self.conversations.get(chat.as_str()) else {
                    return;
                };
                let anchor = self.selection_anchor.clone().unwrap_or_else(|| id.clone());
                let mut ids = ids;
                add_range(&conversation.messages, &mut ids, &anchor, &id);
                self.selection = Some((chat, ids));
                self.selection_anchor = Some(id);
            }
            Action::SweepMessages { anchor, to } => {
                let Some(chat) = self.open_chat.clone() else {
                    return;
                };
                // A sweep adds to what was selected when it began, so
                // dragging back leaves out the rows it passes again.
                if self
                    .sweep
                    .as_ref()
                    .is_none_or(|sweep| sweep.chat != chat || sweep.anchor != anchor)
                {
                    let base = self
                        .selection
                        .as_ref()
                        .filter(|(selected, _)| *selected == chat)
                        .map(|(_, ids)| ids.clone())
                        .unwrap_or_default();
                    self.sweep = Some(Sweep {
                        chat: chat.clone(),
                        anchor: anchor.clone(),
                        to: to.clone(),
                        base,
                    });
                }
                {
                    let Some(sweep) = self.sweep.as_mut() else {
                        return;
                    };
                    sweep.to.clone_from(&to);
                }
                let base = self
                    .sweep
                    .as_ref()
                    .map(|sweep| sweep.base.clone())
                    .unwrap_or_default();
                let range = self.conversations.get(chat.as_str()).map(|conversation| {
                    let mut ids = base;
                    add_range(&conversation.messages, &mut ids, &anchor, &to);
                    ids
                });
                let Some(ids) = range else {
                    return;
                };
                // An open selection stays open, as its check boxes do, even
                // with nothing in it.
                let open = self
                    .selection
                    .as_ref()
                    .is_some_and(|(selected, _)| *selected == chat);
                self.selection = (open || !ids.is_empty()).then_some((chat, ids));
                self.selection_anchor = Some(to);
            }
            Action::EndSweep => self.sweep = None,
            Action::ToggleSelected(id) => {
                let mut next = self.selection.clone();
                if let Some((chat, ids)) = next.as_mut() {
                    let selectable = self
                        .conversations
                        .get(chat.as_str())
                        .and_then(|conversation| conversation.message(&id))
                        .is_some_and(|message| can_select(&message.content));
                    if !selectable {
                        return;
                    }
                    self.selection_anchor = Some(id.clone());
                    if let Some(index) = ids.iter().position(|selected| *selected == id) {
                        ids.remove(index);
                    } else {
                        ids.push(id);
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
                }
                self.selection = next;
            }
            Action::CancelSelection => {
                self.selection = None;
                self.sweep = None;
            }
            Action::Edit(id) => {
                let open = self.open_chat.clone();
                let text =
                    open.as_deref()
                        .and_then(|chat| self.conversations.get(chat))
                        .and_then(|conversation| conversation.message(&id))
                        .and_then(|message| match &message.content {
                            Content::Text { text, .. } => Some(text.clone()),
                            _ => None,
                        })
                        .or_else(|| {
                            let chat = open.as_deref()?;
                            let entry = self.starred.iter().find(|entry| {
                                entry.message.chat == chat && entry.message.id == id
                            })?;
                            match &entry.message.content {
                                Content::Text { text, .. } => Some(text.clone()),
                                _ => None,
                            }
                        });
                if let Some(text) = text {
                    self.editing = Some(id);
                    self.composer_tools_open = false;
                    self.reply_to = None;
                    let cursor = text.chars().count();
                    self.composer = text;
                    self.composer_mentions.clear();
                    self.emoji_start = None;
                    self.mention_start = None;
                    self.set_composer_cursor(ctx, cursor);
                    self.focus_composer = true;
                }
            }
            Action::CancelEdit => {
                if self.editing.take().is_some() {
                    self.composer.clear();
                    self.composer_mentions.clear();
                    self.emoji_start = None;
                    self.mention_start = None;
                    self.set_composer_cursor(ctx, 0);
                }
            }
            Action::DeleteForEveryone { chat, id } => {
                if let Some(message) = self
                    .conversations
                    .get_mut(&chat)
                    .and_then(|conversation| conversation.message_mut(&id))
                {
                    message.content = Content::Revoked;
                }
                self.prune_selection();
                self.forget_star(&chat, &id);
                self.backend.send(Command::Revoke { chat, id });
            }
            Action::DeleteForMe { chat, id } => {
                // The message stays until WhatsApp accepts the deletion.
                // `Event::MessageDeleted` removes it and prunes the selection.
                self.backend.send(Command::DeleteLocal { chat, id });
            }
            Action::Attach { documents } => {
                if let Some(chat) = self.open_chat.clone() {
                    self.backend.send(Command::PickFiles { chat, documents });
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
            Action::SendFiles(paths) => {
                self.stage_files(paths);
                if !self.pending.is_empty() {
                    ctx.memory_mut(|memory| memory.request_focus(egui::Id::new("composer-text")));
                }
            }
            Action::SendPending {
                account,
                chat,
                caption,
                mentions,
            } => self.send_pending(account, chat, caption, mentions),
            Action::RemovePending(index) => {
                if index < self.pending.len() {
                    self.pending.remove(index);
                    self.pending_view_once &= !self.pending.is_empty();
                    self.forget_unstaged_pictures();
                }
            }
            Action::SetPendingViewOnce(view_once) => {
                self.set_pending_view_once(view_once);
            }
            Action::EditPicture(target) => {
                // Whatever was being waited for is not what is wanted now.
                self.picture_inspecting = None;
                self.picture_edit_followed = Some(crate::ui::overlay::followed(ctx));
                // Read out before anything is written back, so the borrow of
                // the strip ends here.
                let pasted = match self.staged_at(target) {
                    Some(at) => match &self.pending[at].item {
                        Pending::Picture {
                            width,
                            height,
                            rgba,
                            ..
                        } => Some((*width as u32, *height as u32, rgba.clone())),
                        Pending::File(_) | Pending::Document(_) => None,
                    },
                    None => None,
                };
                let file = match self.staged_at(target) {
                    Some(at) => match &self.pending[at].item {
                        Pending::File(path) => Some(path.clone()),
                        Pending::Picture { .. } | Pending::Document(_) => None,
                    },
                    None => None,
                };
                if let Some((width, height, rgba)) = pasted {
                    self.picture_edit = Some(crate::model::PictureEdit::new(
                        target,
                        crate::model::PictureSource::Pasted(rgba),
                        width,
                        height,
                    ));
                } else if let Some(path) = file {
                    // A picture that was cropped before opens on the original
                    // with the crop it had, so cropping it again does not
                    // stack another lossy pass on the one before it.
                    if let Some(origin) = self.picture_origins.get(&target).cloned() {
                        self.picture_edit = Some(crate::model::PictureEdit {
                            target,
                            source: origin.source,
                            width: origin.width,
                            height: origin.height,
                            crop: origin.crop,
                            turns: origin.turns,
                        });
                    } else if Pending::is_picture_file(&path) {
                        // Its size has to be read before the cropper can open
                        // on it, and only this answer may open it.
                        self.picture_inspecting = Some(target);
                        self.backend.send(Command::InspectPicture { target, path });
                    }
                }
            }
            Action::ApplyPictureEdit => {
                if let Some(edit) = self.picture_edit.take() {
                    // Held against the attachment until the file lands, so a
                    // second edit started meanwhile cannot take its origin.
                    self.picture_applying.insert(
                        edit.target,
                        crate::model::PictureOrigin {
                            source: edit.source.clone(),
                            width: edit.width,
                            height: edit.height,
                            crop: edit.crop,
                            turns: edit.turns,
                        },
                    );
                    self.backend.send(Command::ApplyPictureEdit {
                        target: edit.target,
                        source: edit.source,
                        width: edit.width,
                        height: edit.height,
                        crop: edit.crop,
                        turns: edit.turns,
                    });
                }
            }
            Action::CancelPictureEdit => {
                self.picture_edit = None;
                self.picture_inspecting = None;
            }
            Action::ClearPending => {
                self.pending.clear();
                self.pending_view_once = false;
                self.forget_unstaged_pictures();
            }
            Action::PlayVoice { message, path } => self.play_voice(message, path),
            Action::PlayVideo { message, path } => self.play_video(message, path),
            Action::PlayVideoWhenDownloaded(message) => {
                self.voice_chat = None;
                self.voice_wanted = None;
                self.video_wanted = self.open_chat.clone().map(|chat| (chat, message));
            }
            Action::SeekVideo { message, fraction } => self.video.seek(&message, fraction),
            Action::ToggleVideoSound => self.video.toggle_mute(),
            Action::CycleVideoSpeed => self.video.cycle_speed(),
            Action::ExpandVideo { message, path } => self.expand_video(message, path, ctx),
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
                self.voice_clip_chat = self.open_chat.clone();
                self.sync_voice_speed();
                if let Err(error) = self.player.seek(&message, &path, fraction) {
                    self.toast_error(error);
                }
            }
            Action::SetVoiceSpeed(speed) => {
                self.settings.voice_speed = crate::audio::supported_speed(speed);
                self.sync_voice_speed();
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
                    if tab == PickerTab::Emoji {
                        crate::emoji::prewarm(
                            ctx,
                            crate::ui::picker::first_page(&self.settings.recent_emoji),
                        );
                    }
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
                self.return_from_reaction_picker(was_reaction);
                if !was_reaction {
                    self.refocus_composer(ctx);
                }
            }
            Action::OpenReactionPicker {
                chat,
                message,
                beside_menu,
                from_list,
            } => {
                self.reaction_from_list = from_list;
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
                let frequent: Vec<String> = self
                    .settings
                    .reaction_emoji
                    .iter()
                    .map(|(emoji, _)| emoji.clone())
                    .collect();
                crate::emoji::prewarm(ctx, crate::ui::picker::first_page(&frequent));
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
                let chat = self.current_chat();
                let everyone =
                    id == crate::mentions::ALL_ID && chat.is_some_and(Chat::can_mention_everyone);
                let member = chat.is_some_and(|chat| {
                    chat.is_group() && chat.participants.iter().any(|known| known == &id)
                });
                let mention_at = start
                    .checked_add(1)
                    .is_some_and(|after| self.composer.get(start..after) == Some("@"));
                if (member || everyone)
                    && start <= end
                    && self.composer.is_char_boundary(start)
                    && self.composer.is_char_boundary(end)
                    && mention_at
                {
                    let mention = if everyone {
                        format!("@{}", crate::mentions::ALL_USER)
                    } else {
                        format!("@{name}")
                    };
                    let inserted = format!("{mention} ");
                    self.composer.replace_range(start..end, &inserted);
                    if !everyone {
                        self.composer_mentions.push(ComposerMention { id, name });
                    }
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
                    let quoting = chat.as_ref().and(self.reply_to.clone());
                    self.backend.send(Command::MakeSticker {
                        source: draft.source,
                        crop: draft.crop,
                        transparent: draft.transparent && draft.keep_transparent,
                        emojis: crate::sticker_meta::clean_emojis(&draft.emojis),
                        chat,
                        quoting,
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
                    self.follow_sent_chat();
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
                    self.follow_outgoing();
                    self.refocus_composer(ctx);
                }
            }
            Action::SearchGifs(query) => {
                self.gif_query = query.trim().to_owned();
                self.gif_results.clear();
                self.gif_next_offset = None;
                self.request_gif_page(0);
            }
            Action::MoreGifs => {
                if !self.gif_pending
                    && let Some(offset) = self.gif_next_offset
                {
                    self.request_gif_page(offset);
                }
            }
            Action::SendGif(gif) => {
                if let Some(chat) = self.open_chat.clone() {
                    self.toast("Sending GIF…");
                    let quoting = self.reply_to.take();
                    self.backend.send(Command::SendGif { chat, gif, quoting });
                    self.picker = None;
                    self.follow_outgoing();
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
                    self.stage(Pending::Picture {
                        width,
                        height,
                        rgba: std::sync::Arc::new(rgba),
                        texture: None,
                    });
                    self.focus_composer = true;
                    ctx.memory_mut(|memory| {
                        memory.request_focus(egui::Id::new("composer-text"));
                    });
                }
            }
            Action::React {
                chat,
                message,
                emoji,
            } => {
                // Only a reaction chosen in the open picker hands focus back
                // to the list that opened it.
                let picker_open = self.reaction_target.is_some();
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
                self.return_from_reaction_picker(picker_open);
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
                self.backend.send(Command::SetArchived(chat, archived));
            }
            // The chat leaves the list once the phone confirmed, through
            // `Event::ChatRemoved`.
            Action::DeleteChat(chat) => self.backend.send(Command::DeleteChat(chat)),
            // The messages go once the phone confirmed, through
            // `Event::ChatCleared`; the chat stays either way.
            Action::ClearChat(chat) => self.backend.send(Command::ClearChat(chat)),
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
            Action::SetMessagePinned {
                chat,
                message,
                pinned,
            } => {
                // WhatsApp keeps three active pins per chat, and the phone
                // replaces an existing one when a fourth arrives. Refuse the
                // fourth here, where the count is already known.
                let now = crate::util::now();
                let active = self.chat_pins.get(&chat).map_or(0, |rows| {
                    rows.iter()
                        .filter(|row| row.expires_at > now && row.id != message)
                        .count()
                });
                if pinned && active >= 3 {
                    self.toast(crate::i18n::gettext(
                        self.locale,
                        "This chat already has three pinned messages",
                    ));
                } else {
                    self.backend.send(Command::SetMessagePinned {
                        chat,
                        message,
                        pinned,
                    });
                }
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
                self.cancel_contact_dialog_request();
                self.clear_chat_lock_entry();
                if dialog == Dialog::NewChat {
                    self.new_chat_search.clear();
                    self.new_group = None;
                }
                if let Dialog::AddGroupMembers(chat) = &dialog {
                    let Some(existing) = self
                        .chat(chat)
                        .filter(|chat| chat.can_add_members())
                        .map(|chat| chat.participants.clone())
                    else {
                        return;
                    };
                    self.new_chat_search.clear();
                    self.new_group = Some(crate::new_group::Draft::adding(chat.clone(), existing));
                }
                self.emoji_start = None;
                self.mention_start = None;
                if matches!(&dialog, Dialog::CreatePoll(_)) && !self.poll_creating {
                    self.poll_draft = Default::default();
                }
                if matches!(&dialog, Dialog::SendLocation(_)) {
                    self.location_draft.clear();
                }
                if let Dialog::Forward { chat, messages } = &dialog {
                    self.forward_search.clear();
                    self.forward_recipients.clear();
                    self.forward_reviewing = false;
                    // Until the worker has read the messages' forwarding
                    // scores; it refuses a batch over the real limit anyway.
                    self.forward_limit = crate::model::FORWARD_LIMIT;
                    self.backend.send(Command::ForwardLimit {
                        from_chat: chat.clone(),
                        messages: messages.clone(),
                    });
                }
                if dialog == Dialog::PairWithPhone {
                    self.pair_phone.clear();
                }
                if dialog == Dialog::NewContact {
                    self.new_contact_to_phone = self.account().settings.save_contacts_to_phone;
                    self.new_contact_phone.clear();
                    self.new_contact_name.clear();
                    self.new_contact_last.clear();
                    self.new_contact_pending = false;
                }
                self.contact_edit = None;
                self.group_name_edit = None;
                self.dialog = Some(dialog);
            }
            Action::CloseDialog => {
                self.cancel_contact_dialog_request();
                self.clear_chat_lock_entry();
                self.dialog = None;
                self.invite = None;
                self.forward_search.clear();
                self.forward_recipients.clear();
                self.forward_reviewing = false;
                self.contact_edit = None;
                self.group_name_edit = None;
                // A group on its way still opens once WhatsApp makes it.
                self.new_group = None;
                self.refocus_composer(ctx);
            }
            Action::EditContact { id, name } => {
                let first = self.contacts.get(&id).and_then(Contact::first_name);
                self.contact_edit = Some(crate::util::editor_names(&name, first));
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
                    to_phone: self.account().settings.save_contacts_to_phone,
                });
            }
            Action::NewContact {
                phone,
                first,
                last,
                to_phone,
            } => {
                let Some(phone) = crate::util::international_phone(&phone) else {
                    self.toast_error(
                        crate::i18n::gettext(
                            self.locale,
                            "Enter a valid international phone number",
                        )
                        .into_owned(),
                    );
                    return;
                };
                self.new_contact_pending = true;
                self.new_contact_sequence = self.new_contact_sequence.wrapping_add(1);
                let request = self.new_contact_sequence;
                self.new_contact_request = Some(request);
                self.new_contact_dialog_request =
                    matches!(self.dialog, Some(Dialog::NewContact)).then_some(request);
                let (full_name, first_name) = compose_name(&first, &last);
                // The dialog's choice starts the next one.
                if let Some(to_phone) = to_phone
                    && full_name.is_some()
                    && to_phone != self.account().settings.save_contacts_to_phone
                {
                    self.account_mut().settings.save_contacts_to_phone = to_phone;
                    self.account_mut().mark_settings_dirty();
                }
                self.backend.send(Command::NewContact {
                    request,
                    phone,
                    full_name,
                    first_name,
                    to_phone: to_phone.unwrap_or(self.account().settings.save_contacts_to_phone),
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
                self.remember_list_filter();
            }
            Action::SetCallFilter(filter) => {
                self.call_filter = filter;
                // Kept for the next start, as the chat list's chip is.
                let key = filter.key();
                if self.account().settings.call_filter != key {
                    self.account_mut().settings.call_filter = key;
                    self.account_mut().mark_settings_dirty();
                }
            }
            Action::JoinGroup => {
                use crate::model::InviteState;
                let ready = self.invite.as_ref().and_then(|invite| match &invite.state {
                    InviteState::Ready(info) => {
                        Some((info.id.clone(), invite.code.clone(), info.clone()))
                    }
                    _ => None,
                });
                if let Some((id, code, info)) = ready {
                    if self.chats.iter().any(|chat| chat.id == id) {
                        self.invite = None;
                        self.dialog = None;
                        self.actions.push(Action::OpenChat(id));
                    } else if let Some(invite) = self.invite.as_mut() {
                        invite.state = InviteState::Joining(info);
                        self.backend.send(Command::JoinInvite(code));
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
            Action::ToggleStarred => {
                // The rail, the list header's Back and Escape all land here. The
                // sidebar has to be open for the list to be visible.
                self.sidebar_visible = true;
                self.show_starred = !self.show_starred;
                if self.show_starred {
                    self.show_archived = false;
                    // The starred list takes the panel the folder held, and
                    // the folder is a locked one: leave it on the way in.
                    if self.locked_folder {
                        self.close_locked_folder();
                        self.search.clear();
                        self.search_hits.clear();
                    }
                    self.backend.send(Command::LoadStarred);
                }
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
                // From the focused message list standing on the newest
                // message, it then follows new messages, as End does.
                if self.messages_focused && self.cursor_on_newest() {
                    self.scroll_to_newest();
                }
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
            Action::MessageCursor(step) => {
                use crate::ui::list_nav::Step;
                let Some(chat) = self.open_chat.clone() else {
                    return;
                };
                // The account's field directly, so the window's cursor can
                // change while the messages are borrowed.
                let Some(conversation) = self.accounts[self.active].conversations.get(&chat) else {
                    return;
                };
                let messages = &conversation.messages;
                // The cursor steps through the list's rows, an album being
                // one, as the list resolves it: a cursor naming no loaded
                // message stands on the newest.
                let starts =
                    crate::ui::conversation::row_starts(messages, self.selecting_in(&chat));
                let current = self
                    .message_cursor
                    .get(&chat)
                    .and_then(|id| messages.iter().position(|message| &message.id == id))
                    .map(|index| crate::ui::conversation::row_of(&starts, index))
                    .or(starts.len().checked_sub(1));
                let Some(row) = crate::ui::list_nav::step(current, starts.len(), step) else {
                    return;
                };
                let id = messages[starts[row]].id.clone();
                let newest = row + 1 == starts.len();
                // The reader moved: a replied message still loading does not
                // take the list back later.
                self.pending_follow = None;
                self.message_cursor.insert(chat.clone(), id);
                self.reveal_cursor = true;
                // Home on the oldest loaded message asks for older ones, as
                // scrolling to the top does.
                if step == Step::First && current == Some(0) {
                    self.load_older(&chat, true);
                }
                // However the keys reached the newest message (End, Down,
                // PgDn), the view goes on to the true bottom below it.
                if newest {
                    self.scroll_to_newest();
                } else {
                    self.cursor_followed = None;
                }
            }
            Action::OpenMessageMenu(id) => {
                // The bubble opens its menu when it is next drawn; the
                // current message is brought into view so that it is.
                if let Some(chat) = self.open_chat.clone() {
                    self.open_message_menu = Some((chat, id));
                    self.reveal_cursor = true;
                }
            }
            Action::MessageMenuOpened => self.open_message_menu = None,
            Action::FocusMessages => {
                if self.current_chat().is_some() {
                    // The Calls tab keeps its history beside the chat, as
                    // when a chat opens there, unless a narrow window shows
                    // only the history; every other page gives way to the
                    // chats.
                    if self.page != Page::Calls || (crate::ui::narrow(ctx) && !self.call_picked()) {
                        self.page = Page::Chats;
                    }
                    self.focus_search = false;
                    self.focus_composer = false;
                    // The list takes focus when it is next drawn.
                    self.focus_messages = true;
                    // On the newest message, it shows the true bottom below
                    // it, as End does.
                    if self.cursor_on_newest() {
                        self.scroll_to_newest();
                    }
                    // Shown as after Tab: the ring goes around the current
                    // message.
                    ctx.data_mut(|data| {
                        data.insert_temp(crate::theme::keyboard_focus_id(), true);
                    });
                } else {
                    self.announcer.say(
                        crate::announcer::Category::Status,
                        crate::i18n::gettext(self.locale, "No chat open"),
                    );
                }
            }
            Action::ScrollTo(id) => {
                self.scroll_to_message(id, true);
            }
            Action::GoToReplied(id) => {
                let Some(chat) = self.open_chat.clone() else {
                    return;
                };
                let loaded = self
                    .conversations
                    .get(&chat)
                    .is_some_and(|conversation| conversation.message(&id).is_some());
                // Not loaded yet: the list keeps its place, rather than fall
                // back to the newest message, and moves once the replied
                // message arrives.
                let asked = self.scroll_to_message(id.clone(), loaded);
                if !loaded {
                    self.pending_follow = Some(PendingFollow {
                        chat,
                        message: id,
                        focus: ctx.memory(|memory| memory.focused()),
                        frame: ctx.cumulative_frame_nr(),
                        asked,
                        answered: false,
                    });
                    let text = crate::i18n::gettext(self.locale, "Loading the replied message");
                    self.announcer.say(crate::announcer::Category::Status, text);
                }
            }
            Action::Search(text) => {
                self.search = text;
                self.search_selected = None;
                // The names and previews compared for the last text are not
                // worth remembering for the next one.
                self.search_keys.borrow_mut().clear();
                let query = self.search.trim().to_owned();
                // Editing the search away from the secret code hides the
                // locked folder again, like leaving the phone's home screen.
                if !self.chat_lock_authenticated() && !self.secret_code_matched() {
                    self.close_locked_folder();
                }
                if query.is_empty() || self.locked_folder_open() || self.secret_code_matched() {
                    self.search_hits.clear();
                    self.search_due = None;
                } else {
                    self.search_due = Some(Instant::now() + SEARCH_DELAY);
                }
            }
            Action::OpenChatSearch => self.open_chat_search(),
            Action::OpenInfo(chat) => self.open_info(chat),
            Action::CloseInfo => {
                self.close_info();
                self.refocus_composer(ctx);
            }
            Action::ShowInfo(view) => {
                if let Some(info) = self.info.as_mut() {
                    // The keyboard follows the view: Back after the media row
                    // opened the media view, the row after Back closed it.
                    self.info_focus = match (info.view, view) {
                        (InfoView::Overview, InfoView::Media(_) | InfoView::Permissions) => {
                            Some(InfoFocus::Back)
                        }
                        (InfoView::Media(_), InfoView::Overview) => Some(InfoFocus::MediaRow),
                        _ => None,
                    };
                    info.view = view;
                }
            }
            Action::RetryInfoMedia => self.request_info_media(),
            Action::CloseContactEdit => self.contact_edit = None,
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
            Action::SetFont(choice) => {
                self.settings.font = choice;
                self.mark_settings_dirty();
                crate::theme::set_font(ctx, choice);
                ctx.request_repaint();
            }
            Action::SetCustomTitlebar(custom) => {
                if self.settings.custom_titlebar != custom {
                    self.settings.custom_titlebar = custom;
                    self.mark_settings_dirty();
                    // The choice is process-wide; tests share one process,
                    // and a window drawn with it would change the layout
                    // of every other test running beside this one.
                    #[cfg(not(test))]
                    fastframe_titlebar::set_enabled(custom);
                    // Decorations are fixed when a window is made. Replace
                    // the window, keeping the chat, the link and any call:
                    // the shell opens the next one at once.
                    if cfg!(any(windows, target_os = "linux")) && !self.window_hidden {
                        self.reopen = true;
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                }
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
            Action::PickWallpaperImage => self.backend.send(Command::PickWallpaperImage),
            Action::RemoveWallpaperImage => {
                if self.account_mut().settings.wallpaper_image.take().is_some() {
                    self.account_mut().mark_settings_dirty();
                }
                crate::wallpaper::forget_image(ctx);
                self.backend.send(Command::RemoveWallpaperImage);
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
            Action::EditGroupName(name) => self.group_name_edit = Some(name),
            Action::CloseGroupName => self.group_name_edit = None,
            Action::SetGroupName { chat, name } => {
                self.group_name_edit = None;
                self.backend.send(Command::SetGroupName { chat, name });
            }
            Action::StartNewGroup => {
                self.new_chat_search.clear();
                self.new_group = Some(Default::default());
            }
            Action::ToggleNewGroupMember(id) => {
                let Some(draft) = self.new_group.as_mut() else {
                    return;
                };
                let adding = !draft.picked(&id);
                if draft.toggle(&id) == crate::new_group::Step::Leave {
                    self.new_group = None;
                } else if adding {
                    // Ready for the next name, as on the phone.
                    self.new_chat_search.clear();
                }
            }
            Action::NewGroupNext => {
                if let Some(draft) = self.new_group.as_mut() {
                    draft.advance();
                }
            }
            Action::NewGroupBack => {
                let adding = self
                    .new_group
                    .as_ref()
                    .is_some_and(|draft| draft.adding.is_some());
                if self
                    .new_group
                    .as_mut()
                    .is_some_and(|draft| draft.back() == crate::new_group::Step::Leave)
                {
                    self.new_group = None;
                    // Add members has no plain list to step back to.
                    if adding {
                        self.actions.push(Action::CloseDialog);
                    }
                }
            }
            Action::PickNewGroupPicture => {
                if self.new_group.is_some() {
                    self.backend.send(Command::PickNewGroupPicture);
                }
            }
            Action::RemoveNewGroupPicture => {
                if let Some(draft) = self.new_group.as_mut() {
                    draft.picture = None;
                }
            }
            Action::CreateGroup => self.create_group(),
            Action::FetchInviteLink(chat) => {
                let asked = self.info_invite.as_ref().is_some_and(|(shown, state)| {
                    shown == &chat && !matches!(state, crate::group_admin::LinkState::Failed(_))
                });
                if !asked && self.chat(&chat).is_some_and(Chat::can_share_invite_link) {
                    self.info_invite = Some((chat.clone(), crate::group_admin::LinkState::Loading));
                    self.backend
                        .send(Command::GroupInviteLink { chat, reset: false });
                }
            }
            Action::CopyInviteLink(chat) => {
                if let Some((_, crate::group_admin::LinkState::Known(link))) = self
                    .info_invite
                    .as_ref()
                    .filter(|(shown, _)| shown == &chat)
                {
                    ctx.copy_text(link.as_str().to_owned());
                    let said = crate::i18n::gettext(self.locale, "Invite link copied");
                    self.toast(said.into_owned());
                }
            }
            Action::ResetInviteLink(chat) => {
                if self.dialog == Some(Dialog::ConfirmResetInviteLink(chat.clone())) {
                    self.dialog = None;
                }
                if self.chat(&chat).is_some_and(Chat::can_manage_group) {
                    self.info_invite = Some((chat.clone(), crate::group_admin::LinkState::Loading));
                    self.backend
                        .send(Command::GroupInviteLink { chat, reset: true });
                }
            }
            Action::AddGroupMembers => {
                let Some(draft) = self.new_group.as_mut().filter(|draft| draft.can_add()) else {
                    return;
                };
                let Some(chat) = draft.adding.clone() else {
                    return;
                };
                draft.creating = true;
                let members = draft.members.clone();
                self.backend
                    .send(Command::AddGroupMembers { chat, members });
            }
            Action::SetGroupPermission {
                chat,
                permission,
                members,
            } => {
                if self.chat(&chat).is_some_and(Chat::can_manage_group) {
                    self.backend.send(Command::SetGroupPermission {
                        chat,
                        permission,
                        members,
                    });
                }
            }
            Action::ChangeGroupMember {
                chat,
                member,
                change,
            } => {
                if matches!(&self.dialog, Some(Dialog::ConfirmRemoveMember { .. })) {
                    self.dialog = None;
                }
                if self.chat(&chat).is_some_and(Chat::can_manage_group) {
                    self.backend.send(Command::ChangeGroupMember {
                        chat,
                        member,
                        change,
                    });
                }
            }
            Action::PickGroupPicture(chat) => self.backend.send(Command::PickGroupPicture(chat)),
            Action::RemoveGroupPicture(chat) => {
                self.backend
                    .send(Command::SetGroupPicture { chat, jpeg: None });
            }
            Action::SetChatSound { chat, sound } => {
                if let Some(known) = self.chat_mut(&chat) {
                    known.notification_sound = sound.clone();
                }
                self.backend.send(Command::SetChatSound { chat, sound });
            }
            Action::PickChatSound(chat) => self.backend.send(Command::PickChatSound(chat)),
            Action::SetChatVoiceSpeed { chat, speed } => self.set_chat_voice_speed(chat, speed),
            Action::SetDownloadFolder(folder) => {
                self.settings.download_folder = folder.clone();
                self.mark_settings_dirty();
                for account in &self.accounts {
                    account
                        .backend
                        .send(Command::SetDownloadFolder(folder.clone()));
                }
            }
            Action::SetKeepChatsArchived(keep) => {
                self.settings.keep_chats_archived = keep;
                self.mark_settings_dirty();
                for account in &self.accounts {
                    account.backend.send(Command::SetKeepChatsArchived(keep));
                }
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
                for account in &self.accounts {
                    account.backend.send(Command::SetProxy(value.clone()));
                }
            }
            Action::SetStartWithSystem(enabled) => {
                let waker = self.waker.clone();
                self.login_item.set(enabled, move || waker.wake());
            }
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
            Action::LockApp => self.lock_app(),
            Action::UnlockApp => {
                if let Some(stored) = self.settings.app_lock_hash.clone() {
                    let waker = self.waker.clone();
                    self.app_lock.try_unlock(&stored, move || waker.wake());
                }
            }
            Action::ForgotAppPassword(open) => {
                use crate::app_lock::Forgetting;
                if self.app_lock.forgetting != Forgetting::Unlinking {
                    self.app_lock.forgetting = if open {
                        Forgetting::Confirming
                    } else {
                        Forgetting::No
                    };
                }
            }
            Action::UnlinkLockedApp => {
                // The lock lifts only once WhatsApp has unlinked and the
                // chats are gone (`LinkStatus::LoggedOut`), never before.
                // Every number linked here goes: the lock guards them all.
                if self.app_lock.is_locked() {
                    self.app_lock.forgetting = crate::app_lock::Forgetting::Unlinking;
                    for account in &self.accounts {
                        account.backend.send(Command::Unlink);
                    }
                }
            }
            Action::AppLockForm(mode) => {
                use crate::app_lock::{Form, FormMode};
                // Setting a password must not replace one without it, and
                // changing or removing one needs one to exist.
                let fits = |mode: FormMode| {
                    (mode == FormMode::Set) == self.settings.app_lock_hash.is_none()
                };
                if !self.app_lock.checking() {
                    self.app_lock.form = mode.filter(|mode| fits(*mode)).map(Form::new);
                }
            }
            Action::SubmitAppLockForm => {
                use crate::app_lock::FormMode;
                let stored = self.settings.app_lock_hash.clone();
                let fits = self
                    .app_lock
                    .form
                    .as_ref()
                    .is_some_and(|form| (form.mode == FormMode::Set) == stored.is_none());
                if fits {
                    let waker = self.waker.clone();
                    self.app_lock
                        .submit_form(stored.as_deref(), move || waker.wake());
                } else {
                    self.app_lock.form = None;
                }
            }
            Action::SetAutoLock(after) => {
                self.settings.app_lock_after = after;
                self.app_lock.note_input();
                self.mark_settings_dirty();
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
                } else if self.wayland && self.raise_on_compositor() {
                    // The compositor (niri) focused the existing window and
                    // switched to its workspace, so nothing is reopened.
                } else if self.wayland {
                    // Wayland drops a programmatic focus or unminimize
                    // request, so a minimized or covered window cannot come
                    // forward that way. Close it and let the shell open a
                    // fresh one at once: the compositor raises a new toplevel,
                    // and a notification click lands on a visible window.
                    self.reopen = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                } else {
                    // Focus alone leaves a minimized window where it is on
                    // Windows, so restore it first.
                    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                }
            }
            Action::HideWindow => {
                if self.tray_shown() {
                    self.hide_intent = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
            // Route through the configured window-close behavior.
            Action::CloseWindow => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            Action::SwitchAccount(id) => self.switch_account(&id),
            Action::AddAccount => self.add_account(),
            Action::CancelAddAccount => {
                // Back to the account the window showed before; leaving an
                // unlinked new account removes it.
                if self.adding_account
                    && let Some(back) = self.account_before_adding.clone().or_else(|| {
                        self.accounts
                            .iter()
                            .find(|account| account.id != self.account().id)
                            .map(|account| account.id.clone())
                    })
                {
                    self.switch_account(&back);
                }
            }
            Action::RemoveAccount(id) => self.remove_account(id),
            Action::ClearMediaCache(id) => {
                if let Some(account) = self.accounts.iter_mut().find(|known| known.id == id) {
                    account.storage_token += 1;
                    account.storage_clearing = true;
                    let token = account.storage_token;
                    account.backend.send(Command::ClearMediaCache { token });
                }
            }
            Action::SetAccountNotifications(id, on) => {
                if let Some(account) = self.accounts.iter_mut().find(|known| known.id == id)
                    && account.settings.notifications != on
                {
                    account.settings.notifications = on;
                    account.mark_settings_dirty();
                }
            }
        }
    }

    /// What a screen reader hears for a toast. One raised by an account that
    /// is not on screen shows over another account's chats, so it is said
    /// with its account's name first: "Studio: History loaded".
    fn toast_said(&self, message: &str) -> String {
        if !self.events_hidden {
            return message.to_owned();
        }
        let account = self.account().display_label(self.locale);
        // Translators: an announcement from an account that is not on screen:
        // the account's name, then what it announced.
        let template = crate::i18n::gettext(self.locale, "{}: {}");
        let mut parts = template.splitn(3, "{}");
        format!(
            "{}{}{}{}{}",
            parts.next().unwrap_or_default(),
            account,
            parts.next().unwrap_or_default(),
            message,
            parts.next().unwrap_or_default(),
        )
    }

    /// Sends the group the New chat dialog put together to WhatsApp.
    fn create_group(&mut self) {
        let Some(draft) = self.new_group.as_mut() else {
            return;
        };
        if draft.stage != crate::new_group::Stage::Naming || draft.creating {
            return;
        }
        if let Some(problem) = draft.problem() {
            let said = problem.message(self.locale);
            self.toast_error(said);
            return;
        }
        draft.creating = true;
        let subject = draft.subject().to_owned();
        let members = draft.members.clone();
        let picture = draft.picture.clone();
        self.new_group_sequence = self.new_group_sequence.wrapping_add(1);
        let request = self.new_group_sequence;
        self.new_group_request = Some(request);
        self.backend.send(Command::CreateGroup {
            request,
            subject,
            members,
            picture,
        });
    }

    /// WhatsApp answered `CreateGroup`: the new group opens, or the dialog
    /// says why not and stays for another try. An answer the window no
    /// longer waits for opens nothing; its group is filed all the same.
    fn group_created(
        &mut self,
        request: u64,
        result: Result<crate::new_group::Created, crate::new_group::CreateFailure>,
        live: bool,
    ) {
        if self.new_group_request != Some(request) {
            return;
        }
        self.new_group_request = None;
        match result {
            Ok(created) => {
                self.new_group = None;
                if !live {
                    return;
                }
                if self.dialog == Some(Dialog::NewChat) {
                    self.dialog = None;
                }
                self.actions.push(Action::OpenChat(created.chat));
                if !created.unadded.is_empty() {
                    let names: Vec<String> = created
                        .unadded
                        .iter()
                        .map(|id| self.display_name(id))
                        .collect();
                    let said = crate::new_group::unadded_message(self.locale, &names);
                    self.toast(said);
                }
            }
            Err(failure) => {
                if let Some(draft) = self.new_group.as_mut() {
                    draft.creating = false;
                }
                if live {
                    let said = failure.message(self.locale);
                    self.toast_error(said);
                }
            }
        }
    }

    /// WhatsApp answered for a group's invite link: the panel's row shows
    /// it, and a reset says so.
    fn group_invite_link(
        &mut self,
        chat: ChatId,
        reset: bool,
        result: Result<crate::group_admin::InviteLink, crate::group_admin::Failure>,
        live: bool,
    ) {
        use crate::group_admin::LinkState;
        // The panel is the window's: another account's answer is not for it.
        if !live
            || self
                .info_invite
                .as_ref()
                .is_none_or(|(shown, _)| shown != &chat)
        {
            return;
        }
        match result {
            Ok(link) => {
                self.info_invite = Some((chat, LinkState::Known(link)));
                if reset {
                    let said = crate::i18n::gettext(
                        self.locale,
                        "The old link no longer works. Here is a new one.",
                    );
                    self.toast(said.into_owned());
                }
            }
            Err(failure) => {
                self.info_invite = Some((chat, LinkState::Failed(failure)));
                if reset {
                    let said = failure.message(self.locale);
                    self.toast_error(said);
                }
            }
        }
    }

    /// WhatsApp answered Add members: the dialog closes, and the toasts say
    /// who joined and who could not be added, by name.
    fn group_members_added(
        &mut self,
        chat: ChatId,
        result: Result<crate::group_admin::Added, crate::group_admin::Failure>,
        live: bool,
    ) {
        let waiting = self
            .new_group
            .as_ref()
            .is_some_and(|draft| draft.adding.as_deref() == Some(chat.as_str()));
        match result {
            Ok(added) => {
                if waiting {
                    self.new_group = None;
                    if self.dialog == Some(Dialog::AddGroupMembers(chat.clone())) {
                        self.dialog = None;
                    }
                }
                if !live {
                    return;
                }
                if !added.added.is_empty() {
                    let count = added.added.len();
                    let said = crate::i18n::ngettext(
                        self.locale,
                        "{count} person added to the group",
                        "{count} people added to the group",
                        count as u32,
                    )
                    .replace("{count}", &count.to_string());
                    self.toast(said);
                }
                let refused: Vec<(String, crate::group_admin::Refusal)> = added
                    .refused
                    .iter()
                    .map(|(id, why)| (self.display_name(id), *why))
                    .collect();
                for said in crate::group_admin::refused_messages(self.locale, &refused) {
                    self.toast(said);
                }
            }
            Err(failure) => {
                if waiting && let Some(draft) = self.new_group.as_mut() {
                    draft.creating = false;
                }
                if live {
                    let said = failure.message(self.locale);
                    self.toast_error(said);
                }
            }
        }
    }

    pub fn toast(&mut self, message: impl Into<String>) {
        let message = message.into();
        let said = self.toast_said(&message);
        self.announcer.say(crate::announcer::Category::Toast, said);
        self.toasts.push(Toast {
            message,
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
        let said = self.toast_said(&message);
        self.announcer.say(crate::announcer::Category::Error, said);
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

    /// Starts the quote a starred row asked for, once the message it points at
    /// is in the open chat. The composer drops a quote whose message has not
    /// loaded, so waiting for it here keeps Reply working from the list.
    fn settle_pending_reply(&mut self) {
        let Some((chat, id)) = self.reply_when_loaded.clone() else {
            return;
        };
        let loaded = self.open_chat.as_deref() == Some(chat.as_str())
            && self
                .conversations
                .get(&chat)
                .is_some_and(|conversation| conversation.message(&id).is_some());
        if !loaded {
            return;
        }
        self.reply_when_loaded = None;
        self.reply_to = Some(id.into());
        self.focus_composer = true;
    }

    /// Asks the worker for the starred list again, for the one on screen. A
    /// message that changed in the archive is a stale row otherwise.
    fn reload_lists(&mut self) {
        if self.show_starred {
            self.backend.send(Command::LoadStarred);
        }
    }

    /// Drops the star of a message that no longer exists. A message deleted
    /// here, and one revoked for everyone, keep no star: the mark would stay
    /// in the conversation and the open list would hold a row with nothing
    /// left to unstar.
    fn forget_star(&mut self, chat: &str, id: &str) {
        if let Some(ids) = self.stars.get_mut(chat) {
            ids.remove(id);
        }
        self.reload_lists();
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
        let looking =
            self.window_focused && !self.window_hidden && !self.power.conditions().display_off;
        let active = self.active;
        for (index, account) in self.accounts.iter_mut().enumerate() {
            let online = looking && index == active;
            if account.reported_online != Some(online) {
                account.reported_online = Some(online);
                account.backend.send(Command::SetOnline(online));
            }
        }
    }

    /// Processes app state shared by windowed and headless modes.
    pub fn background_frame(&mut self, ctx: &egui::Context) {
        // Events are drained before frame_ui observes focus. Losing focus in
        // this frame must take effect before an incoming chat update can read it.
        if self.window_hidden || ctx.input(|input| input.viewport().focused) == Some(false) {
            self.window_focused = false;
        }
        self.tick_app_lock(ctx);
        self.tick_login_item();
        // Nobody reads behind the lock screen: messages stay unread, the
        // phone keeps notifying, and notifications here say nothing.
        if self.app_lock.is_locked() {
            self.window_focused = false;
        }
        self.report_presence();
        self.handle_power();
        self.handle_tray();
        #[cfg(target_os = "macos")]
        self.actions.extend(crate::macos::drain(self.window_hidden));
        self.handle_control_commands();
        self.poll_custom_themes();
        let wallpaper = self.account().settings.wallpaper_image.clone();
        self.wallpaper_image.sync(wallpaper.as_deref(), &self.waker);
        self.handle_notification_opens();
        // Before this frame's pages: a reader who moved on ends the wait.
        self.end_pending_follow(ctx);
        self.handle_events();
        for uri in std::mem::take(&mut self.stale_images) {
            crate::image_cache::forget(ctx, &uri);
        }
        self.tick(ctx);
        self.tick_audio(ctx);
        self.tick_video(ctx);
        self.apply_actions(ctx);
        self.sync_prefetch();
        self.hold_media();
        self.follow_receipts();
        self.sync_badge();
        self.request_attention(ctx);
    }

    /// Answers the system's sleep and wake. Every account reconnects its link
    /// after a wake (Linux and Windows report it; macOS does not yet, so
    /// there the worker's link watch notices the sleep on its own). Settings
    /// waiting for their debounced save are saved before the machine sleeps
    /// or the session ends.
    fn handle_power(&mut self) {
        for event in self.power.events() {
            match event {
                fastframe_power::Event::Suspending(ack)
                | fastframe_power::Event::EndingSession(ack) => {
                    self.save_state();
                    // Within the time the system waits for the app.
                    self.files.flush(Duration::from_millis(1500));
                    ack.done();
                }
                fastframe_power::Event::Resumed => {
                    for account in &self.accounts {
                        account.backend.send(Command::Resumed);
                    }
                }
            }
        }
    }

    /// Highlights the taskbar entry for a message that notified while the
    /// window is open behind others, as KDE Plasma's task manager shows for
    /// chat apps. The desktop clears it once the window is focused. Linux
    /// only: elsewhere it would bounce the Dock or flash the taskbar. Asks the
    /// viewport too, since `window_focused` stays false behind the app lock
    /// even while the lock screen has the focus.
    fn request_attention(&mut self, ctx: &egui::Context) {
        let skip = fastframe_desktop::is_gnome()
            || hyprland(
                std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some(),
                std::env::var("XDG_CURRENT_DESKTOP").ok().as_deref(),
            );
        self.request_attention_on(ctx, skip);
    }

    /// [`Self::request_attention`] on a desktop that marks no taskbar entry
    /// for the request (`skip`) or one that does. GNOME shows its own "is
    /// ready" notice instead (#466), and Hyprland focuses the window when
    /// `misc:focus_on_activate` is on (#530).
    fn request_attention_on(&mut self, ctx: &egui::Context, skip: bool) {
        if std::mem::take(&mut self.wants_attention)
            && cfg!(target_os = "linux")
            && !skip
            && !self.window_hidden
            && !self.window_focused
            && ctx.input(|input| input.viewport().focused) != Some(true)
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::RequestUserAttention(
                egui::UserAttentionType::Informational,
            ));
        }
    }

    /// Collects a finished read or change of the login item.
    fn tick_login_item(&mut self) {
        let Some(outcome) = self.login_item.poll() else {
            return;
        };
        match outcome.state {
            Ok(state) => {
                self.start_with_system = crate::autostart::switch(state);
                if outcome.changed && matches!(state, fastframe_autostart::State::NeedsApproval(_))
                {
                    self.toast("Allow ZapFast in System Settings, under Login Items.");
                }
            }
            Err(error) => {
                if outcome.changed {
                    self.toast_error(format!("Could not change the login item: {error}"));
                }
            }
        }
    }

    /// Collects a finished password check, and locks once ZapFast has gone
    /// unused for the chosen time. Inactivity counts while the window is
    /// hidden too: only input in the window restarts it.
    fn tick_app_lock(&mut self, ctx: &egui::Context) {
        if let Some(outcome) = self.app_lock.poll() {
            self.app_lock_outcome(outcome);
        }
        if self.settings.app_lock_hash.is_none() {
            // Nothing to unlock with; never leave the window stuck.
            if self.app_lock.is_locked() {
                self.app_lock.release();
            }
            return;
        }
        if self.app_lock.is_locked() {
            return;
        }
        // Recording a voice message is using ZapFast, keys or not.
        if self.recording.is_some() {
            self.app_lock.note_input();
        }
        let left = self
            .app_lock
            .idle_left(self.settings.app_lock_after.duration());
        if left.is_zero() {
            self.lock_app();
        } else {
            ctx.request_repaint_after(left);
        }
    }

    /// Hides everything behind the lock screen. What was open stays open
    /// for after the unlock, except what plays, records, or shows beyond the
    /// window: media stops, a recording is discarded, the locked-chats folder
    /// closes, and notifications already on the desktop are withdrawn.
    pub fn lock_app(&mut self) {
        if self.settings.app_lock_hash.is_none() || self.app_lock.is_locked() {
            return;
        }
        self.app_lock.lock();
        // Nothing private stays readable in the tree behind the lock screen.
        self.announcer.forget_private();
        self.window_focused = false;
        self.flush_open_draft();
        self.recording = None;
        self.player.stop();
        self.video.stop();
        self.voice_chat = None;
        self.voice_wanted = None;
        self.video_wanted = None;
        // A replied message is never found over the lock screen.
        self.pending_follow = None;
        self.dropping = false;
        self.notifications.clear_all();
        self.clear_chat_lock_entry();
        if self.dialog == Some(Dialog::UnlockLockedChats) {
            self.dialog = None;
        }
        if self.locked_folder || self.secret_code_matched() {
            self.close_locked_folder();
            self.search.clear();
            self.search_hits.clear();
        }
        self.copy_rows
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clear();
    }

    /// Queues what a request held by the app lock asked for, in its own
    /// account: a notification's, or `here` for a link. The switch is queued
    /// too, so each request lands in its account after the ones before it.
    fn replay_deferred(&mut self, request: crate::app_lock::Deferred, here: &AccountId) {
        use crate::app_lock::Deferred;
        let account = match &request {
            Deferred::Message(target) => target.account.clone(),
            Deferred::Chat(_) | Deferred::Invite(_) => here.clone(),
        };
        self.actions.push(Action::SwitchAccount(account));
        match request {
            Deferred::Message(target) => {
                // A call's notification has no message: switching to its
                // account is all it asked for.
                if let Some(message) = target.message {
                    self.actions.push(Action::OpenMessage {
                        chat: target.chat,
                        message,
                    });
                }
            }
            Deferred::Chat(request) => {
                self.actions.push(Action::ShowWindow);
                self.actions.push(Action::StartChat {
                    dismiss_dialog: true,
                    name: request.display_name(),
                    id: request.chat.clone(),
                });
                if let Some(text) = request.text {
                    self.actions.push(Action::PrefillComposer {
                        chat: request.chat,
                        text,
                    });
                }
            }
            Deferred::Invite(code) => {
                self.actions
                    .push(Action::OpenUrl(format!("https://chat.whatsapp.com/{code}")));
            }
        }
    }

    /// Applies what the password thread worked out.
    fn app_lock_outcome(&mut self, outcome: crate::app_lock::Outcome) {
        use crate::app_lock::{FormError, Outcome};
        match outcome {
            Outcome::Unlock(matched) => {
                if !self.app_lock.is_locked() {
                    return;
                }
                self.app_lock.unlocked(matched);
                if matched {
                    // In the order they were asked for, so the last one is
                    // what the reader finds on screen. Nothing switches
                    // accounts while locked, so a link was meant for the
                    // account on screen now.
                    let here = self.account().id.clone();
                    for request in std::mem::take(&mut self.app_lock.deferred) {
                        self.replay_deferred(request, &here);
                    }
                }
            }
            Outcome::WrongCurrent => {
                if let Some(form) = &mut self.app_lock.form {
                    form.busy = false;
                    form.error = Some(FormError::WrongCurrent);
                }
            }
            Outcome::Set(verifier) => {
                if self.app_lock.form.take().is_none() {
                    return;
                }
                self.settings.app_lock_hash = Some(verifier);
                self.app_lock.note_input();
                self.save_settings();
                self.toast(crate::i18n::gettext(self.locale, "App lock password saved"));
            }
            Outcome::TurnOff => {
                if self.app_lock.form.take().is_none() {
                    return;
                }
                self.settings.app_lock_hash = None;
                self.save_settings();
                self.toast(crate::i18n::gettext(self.locale, "App lock turned off"));
            }
        }
    }

    /// Forgets the app lock password and lifts the lock: after unlinking,
    /// when nothing is left behind it.
    fn forget_app_lock(&mut self) {
        self.app_lock.form = None;
        self.app_lock.deferred.clear();
        self.app_lock.release();
        if self.settings.app_lock_hash.take().is_some() {
            self.save_settings();
        }
    }

    /// Mirrors the unread chat count onto the taskbar icon, where the desktop
    /// reads it. The badge ignores repeats, so calling this each frame is cheap.
    fn sync_badge(&mut self) {
        let count = self.unread_chat_count_everywhere();
        if let Some(badge) = &mut self.badge {
            badge.set(count);
        }
    }

    /// The unread chat count for the Windows taskbar overlay, which the window
    /// applies itself; `None` in demo and test runs.
    #[cfg(target_os = "windows")]
    pub fn taskbar_badge_count(&self) -> Option<u32> {
        self.badge.as_ref()?.count()
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
                || self.call_live()
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
    fn tick_audio(&mut self, ctx: &egui::Context) {
        if let Err(error) = self.player.poll() {
            self.toast_error(error);
        }
        if let Some(finished) = self.player.take_finished() {
            self.continue_voice(&finished);
        }
        if let Some(error) = self.recording.as_ref().and_then(Recorder::failure) {
            self.recording = None;
            self.recording_failed(&error);
        }
        // The playback bar, the recording light and the waveform only show
        // progress, so they step while a screen reader is attached.
        let stepped = crate::motion::screen_reader_attached(ctx);
        if let Some(recorder) = &self.recording {
            recorder.set_stepped(stepped);
        }
        if self.player.is_playing() || self.recording.is_some() {
            self.waker.wake_after(if stepped {
                crate::motion::STEP
            } else {
                Duration::from_millis(40)
            });
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
        // The viewer shows the open chat's media, and leaves with it.
        if self
            .image_preview
            .as_ref()
            .is_some_and(|preview| self.open_chat.as_deref() != Some(preview.chat()))
        {
            self.image_preview = None;
            self.viewer_media.clear();
            self.viewer_media_chat = None;
            self.viewer_media_state.clear();
        }
        // Larger frames while the viewer shows the clip, the bubble's size
        // otherwise. Only a clip on the viewer plays while it is open.
        self.video.set_expanded(self.image_preview.is_some());
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

    /// Opens the full-window viewer on `preview`, and asks for the album
    /// around it, so stepping through the chat's photos and clips works from
    /// the first frame.
    fn open_viewer(&mut self, preview: PreviewState, ctx: &egui::Context) {
        // The viewer takes the screen: a clip playing in the chat behind it
        // would keep its sound over whatever opens here.
        if self.video.message() != Some(preview.message()) {
            self.video.stop();
        }
        self.viewer_media.clear();
        self.viewer_media_state.clear();
        self.viewer_media_chat = Some(preview.chat().to_owned());
        self.backend.send(Command::LoadChatMedia {
            chat: preview.chat().to_owned(),
            around: Some(preview.message().to_owned()),
        });
        self.image_preview = Some(preview);
        self.dialog = None;
        self.picker = None;
        // egui drops the focus of widgets behind a modal only from the frame
        // after it first shows; until then a focused composer would still
        // take Enter and send the draft. Opened from the keys or for a screen
        // reader, anything else keeps focus until the viewer takes it, in the
        // pass that shows it (`ui::overlay`): let go now, focus would be on
        // the window meanwhile.
        let composer = crate::ui::conversation::composer_id();
        let followed = crate::ui::overlay::followed(ctx);
        ctx.memory_mut(|memory| {
            if let Some(focused) = memory.focused()
                && (focused == composer || !followed)
            {
                memory.surrender_focus(focused);
            }
        });
    }

    /// Plays a video, or a motion photo's clip, across the window: in the
    /// viewer, which opens on it unless it is already showing that message.
    fn expand_video(&mut self, message: String, path: PathBuf, ctx: &egui::Context) {
        let Some(chat) = self.open_chat.clone() else {
            return;
        };
        // A motion photo's clip plays over its photo, which stays the item.
        let photo = self
            .conversations
            .get(&chat)
            .and_then(|conversation| conversation.message(&message))
            .and_then(|row| match &row.content {
                Content::Image {
                    media,
                    motion: Some(motion),
                    ..
                } if motion.path.as_deref() == Some(path.as_path()) => media.path.clone(),
                _ => None,
            });
        let showing = self
            .image_preview
            .as_ref()
            .is_some_and(|preview| preview.chat() == chat && preview.message() == message);
        if !showing {
            let preview = match &photo {
                Some(photo) => PreviewState::new(Some(photo.clone()), chat, message.clone()),
                None => PreviewState::new_clip(path.clone(), chat, message.clone()),
            };
            self.open_viewer(preview, ctx);
        }
        if photo.is_some()
            && let Some(preview) = &mut self.image_preview
        {
            preview.play_motion(path.clone());
        }
        self.video.set_expanded(true);
        if self.video.message() != Some(message.as_str())
            || self.video.path() != Some(path.as_path())
        {
            self.play_video(message, path);
        }
        if self.video.message().is_some() {
            self.video.resume();
        }
    }

    /// Plays or pauses audio and sends the first played receipt when needed.
    fn play_voice(&mut self, message: String, path: PathBuf) {
        self.video.stop();
        self.voice_wanted = None;
        self.voice_chat = self.open_chat.clone();
        self.voice_clip_chat = self.open_chat.clone();
        self.sync_voice_speed();
        if let Err(error) = self.player.toggle(&message, &path) {
            self.toast_error(error);
            return;
        }
        self.tell_played(message);
    }

    /// Gives `chat` its own voice message speed, or `None` to follow
    /// Settings.
    fn set_chat_voice_speed(&mut self, chat: ChatId, speed: Option<f32>) {
        let speed = speed.map(crate::audio::supported_speed);
        if let Some(known) = self.chat_mut(&chat) {
            known.voice_speed = speed;
        }
        self.sync_voice_speed();
        self.backend
            .send(Command::SetChatVoiceSpeed { chat, speed });
    }

    /// Plays the clip in the player at the speed of its own chat, which may
    /// not be the one on screen.
    fn sync_voice_speed(&mut self) {
        let chat = self.voice_clip_chat.clone().or(self.open_chat.clone());
        self.player.set_speed(self.voice_speed_in(chat.as_deref()));
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
        // Played on another device: the phone already told the sender.
        if row.from_me || row.status == Delivery::Played {
            return;
        }
        let sender = row.sender.clone();
        self.played_told.insert(message.clone());
        self.backend.send(Command::MarkPlayed {
            chat,
            message,
            sender,
            receipts: self.account().settings.send_read_receipts,
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
        self.follow_message(&chat, &next);
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
                    manual: false,
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
            .find(|message| {
                !message.from_me
                    && message.status != Delivery::Played
                    && !self.played_told.contains(&message.id)
            })
            .map(|message| message.id.clone())
    }

    /// A reaction picker opened by the message list's E hands focus back to
    /// the list as it closes, by Escape or a chosen emoji: egui's Escape has
    /// already cleared focus, and nothing else would take it. `picker_open`
    /// says the picker was still open: one closed another way (the emoji tab,
    /// the composer tools, a cleared chat) leaves the flag behind, and a
    /// later reaction from a chip or the menu's row is not the picker's.
    /// The flag is dropped either way.
    fn return_from_reaction_picker(&mut self, picker_open: bool) {
        if std::mem::take(&mut self.reaction_from_list) && picker_open {
            self.focus_composer = false;
            self.focus_messages = true;
        }
    }

    /// Scrolls the open chat to message `id`, loading older history toward it
    /// when it is not loaded; `follow` also makes it the list's current one.
    /// Returns whether it asked for that history.
    fn scroll_to_message(&mut self, id: String, follow: bool) -> bool {
        self.scroll_to_bottom = false;
        let Some(chat) = self.open_chat.clone() else {
            return false;
        };
        let asked = self.load_until(&chat, &id);
        if follow {
            self.follow_message(&chat, &id);
        }
        self.jump_highlight = Some(JumpHighlight::new(chat, id.clone()));
        self.scroll_anchor = Some(id);
        asked
    }

    /// Asks for older archive pages down to message `id` when it is not
    /// loaded and no older page is already on its way. Returns whether it
    /// asked.
    fn load_until(&mut self, chat: &str, id: &str) -> bool {
        // Load older archive pages toward the target.
        let load = {
            let conversation = self.conversations.entry(chat.to_owned()).or_default();
            if conversation.message(id).is_none() && !conversation.loading_older {
                conversation.messages.first().map(|oldest| {
                    conversation.loading_older = true;
                    (oldest.timestamp, oldest.id.clone())
                })
            } else {
                None
            }
        };
        let Some(before) = load else {
            return false;
        };
        self.backend.send(Command::LoadUntil {
            chat: chat.to_owned(),
            id: id.to_owned(),
            before,
        });
        true
    }

    /// An older page arrived in `chat`: the replied message the keys wait for
    /// is followed once it is there. Another page (one already on its way when
    /// G was pressed, or the phone's) only lets the wait ask for its own; the
    /// answer to its own request is the one that can say the message is not
    /// there, as can a `complete` page, after which there is no older history
    /// to bring it.
    fn settle_pending_follow(&mut self, chat: &str, empty: bool, complete: bool) {
        let Some(mut pending) = self.pending_follow.clone() else {
            return;
        };
        if pending.chat != chat {
            return;
        }
        let found = self
            .conversations
            .get(chat)
            .is_some_and(|conversation| conversation.message(&pending.message).is_some());
        if found {
            self.pending_follow = None;
            self.follow_message(chat, &pending.message);
        } else if complete {
            // The chat's history is all here, and the message is not in it.
            self.pending_follow = None;
            let text = crate::i18n::gettext(self.locale, "Replied message not found");
            self.announcer.say(crate::announcer::Category::Status, text);
        } else if !pending.asked {
            pending.asked = self.load_until(chat, &pending.message);
            self.pending_follow = Some(pending);
        } else if empty {
            // Its own request found nothing more toward it: once this
            // frame's events are in, `end_pending_follow` says the message
            // is not found. A request that failed says why instead
            // (`Event::HistoryUnavailable`). Nothing else may come to draw
            // that frame before the next key, which would end the wait
            // unsaid, so it is asked for.
            pending.answered = true;
            self.pending_follow = Some(pending);
            self.waker.wake();
        }
    }

    /// Ends the replied message's wait quietly once the reader moved on: to
    /// another chat, to another widget, behind the lock, or with a key or
    /// text after the one that asked. A key held down is not a new one: its
    /// repeats, and the text they type, leave the wait alone. An answer that
    /// came empty says the message is not there.
    fn end_pending_follow(&mut self, ctx: &egui::Context) {
        let Some(pending) = self.pending_follow.as_ref() else {
            return;
        };
        let typed = ctx.cumulative_frame_nr() > pending.frame
            && ctx.input(|input| {
                let events = &input.raw.events;
                let held = events.iter().any(|event| {
                    matches!(
                        event,
                        egui::Event::Key {
                            pressed: true,
                            repeat: true,
                            ..
                        }
                    )
                });
                events.iter().any(|event| match event {
                    egui::Event::Key {
                        pressed: true,
                        repeat,
                        ..
                    } => !repeat,
                    egui::Event::Text(_) => !held,
                    egui::Event::Paste(_) => true,
                    _ => false,
                })
            });
        let moved_on = self.open_chat.as_deref() != Some(pending.chat.as_str())
            || ctx.memory(|memory| memory.focused()) != pending.focus
            || self.app_lock.is_locked()
            || typed;
        let answered = pending.answered;
        if moved_on {
            self.pending_follow = None;
        } else if answered {
            self.pending_follow = None;
            let text = crate::i18n::gettext(self.locale, "Replied message not found");
            self.announcer.say(crate::announcer::Category::Status, text);
        }
    }

    /// Makes `message` the current one in `chat`'s message list, where the
    /// app just took the reader on its own (the next voice message, a quote,
    /// a search result), so the list and the view agree. A focused list also
    /// scrolls it into view.
    pub fn follow_message(&mut self, chat: &str, message: &str) {
        // The reader went somewhere: a replied message still loading does
        // not take the list back later.
        self.pending_follow = None;
        self.message_cursor
            .insert(chat.to_owned(), message.to_owned());
        if self.messages_focused && self.open_chat.as_deref() == Some(chat) {
            self.reveal_cursor = true;
        }
    }

    /// About how many messages fill the open chat's view: its height over
    /// the median height of the rows laid out so far, at least one. PgUp and
    /// PgDn move the message list's cursor by this many; 8 before the list
    /// has been measured.
    pub fn message_page_rows(&self, ctx: &egui::Context, chat: &str) -> usize {
        let viewport = ctx
            .data(|data| {
                data.get_temp::<(f32, f32)>(crate::ui::conversation::scroll_metrics_id(
                    &chat.to_owned(),
                ))
            })
            .map(|(_, height)| height);
        let mut heights: Vec<f32> = self
            .conversations
            .get(chat)
            .map(|conversation| {
                conversation
                    .rows
                    .values()
                    // An album's later items take no height of their own.
                    .filter(|row| row.pass.is_some() && row.height > 0.0)
                    .map(|row| row.height)
                    .collect()
            })
            .unwrap_or_default();
        heights.sort_by(f32::total_cmp);
        match (viewport, heights.get(heights.len() / 2)) {
            (Some(viewport), Some(&median)) if median > 0.0 => {
                ((viewport / median).floor() as usize).max(1)
            }
            _ => 8,
        }
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
            {
                if self
                    .reply_to
                    .as_ref()
                    .is_some_and(|target| !self.reply_source_visible(target, &chat))
                {
                    self.send_refused(
                        chat,
                        self.reply_to.clone(),
                        Unsent::Text(String::new()),
                        Refusal::QuoteUnavailable,
                    );
                    return;
                }
                let Some((_, samples)) = self.unsent_voice.take() else {
                    return;
                };
                let quoting = self.reply_to.take();
                self.backend.send(Command::SendVoice {
                    chat,
                    samples,
                    quoting,
                });
                self.follow_outgoing();
            }
            return;
        };
        let Some(chat) = self.open_chat.clone() else {
            return;
        };
        match recorder.finish() {
            Ok(samples) if samples.len() < crate::voice::RATE as usize / 2 => {}
            Ok(samples) => {
                if self
                    .reply_to
                    .as_ref()
                    .is_some_and(|target| !self.reply_source_visible(target, &chat))
                {
                    self.send_refused(
                        chat,
                        self.reply_to.clone(),
                        Unsent::Voice(samples),
                        Refusal::QuoteUnavailable,
                    );
                    return;
                }
                let quoting = self.reply_to.take();
                self.backend.send(Command::SendVoice {
                    chat,
                    samples,
                    quoting,
                });
                self.follow_sent_chat();
            }
            Err(error) => self.recording_failed(&error),
        }
    }

    fn recording_failed(&mut self, error: &str) {
        // Microphone errors name only the device and its stream, never what
        // was said, so the log keeps them for bug reports (#382).
        log::warn!("recording failed: {error}");
        self.toast_error(format!("Could not record: {error}"));
    }

    pub fn frame_ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let ctx = &ctx;
        self.settle_pending_reply();
        self.copy_rows
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clear();
        *self
            .selection_view
            .lock()
            .unwrap_or_else(|p| p.into_inner()) = None;
        self.apply_theme(ctx);
        if ctx.input(|input| input.events.iter().any(is_user_input)) {
            self.app_lock.note_input();
        }
        let locked = self.app_lock.is_locked();
        // Behind the lock screen nobody is reading; unlocking counts as
        // coming back to the window.
        let focused = ctx.input(|input| input.viewport().focused.unwrap_or(true)) && !locked;
        let regained_focus = focused && !self.window_focused;
        // Mark messages received while hidden as read on window return.
        if regained_focus
            && self.chat_view()
            && let Some(open) = self.open_chat.clone()
            && self.chat(&open).is_some_and(|chat| chat.unread > 0)
        {
            self.mark_read(&open);
        }
        // Coming back to the window (Alt+Tab, a screen reader dialog) leaves
        // the reader on whatever had focus, in any section: egui keeps it
        // while the window is away. Only with nothing focused does the
        // message field take it. Unlocking counts as coming back, and the
        // lock screen's field, gone now, is nothing to return to.
        if regained_focus
            && ctx
                .memory(|memory| memory.focused())
                .is_none_or(|id| id == egui::Id::new(crate::ui::lock::PASSWORD_ID))
        {
            self.refocus_composer(ctx);
        }
        self.window_focused = focused;
        self.report_presence();
        if locked {
            // Files dropped or pasted on the lock screen go nowhere.
            self.dropping = false;
        } else {
            self.scrolling.apply(ctx);
            self.route_scroll(ctx);
            Self::paste_shift_insert(ctx);
            self.take_drops_and_pastes(ctx);
        }
        crate::ui::show(self, ui);
        self.apply_actions(ctx);
        crate::ui::overlay::settle(self, ctx);
        // The old colours, if a change is being revealed, go over everything.
        self.theme_transition.paint(ctx);
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

    fn request_gif_page(&mut self, offset: usize) {
        self.gif_request = self.gif_request.wrapping_add(1);
        self.gif_pending = true;
        self.gif_error = None;
        self.backend.send(Command::SearchGifs {
            query: self.gif_query.clone(),
            key: self.settings.effective_giphy_key().unwrap_or_default(),
            offset,
            request: self.gif_request,
        });
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

    fn invalidate_composer_undo(&mut self) {
        self.composer_undo_epoch = self.composer_undo_epoch.wrapping_add(1);
    }

    /// Keep the fixed editor id for focus/cursor handling, but discard undo
    /// snapshots before keyboard input reaches a different composer owner.
    pub(crate) fn prepare_composer_undo(&self, ctx: &egui::Context) {
        let id = egui::Id::new("composer-text");
        let owner_id = id.with("undo-owner");
        let owner = (
            self.account().id.clone(),
            self.open_chat.clone(),
            self.editing.clone(),
            self.composer_undo_epoch,
        );
        let previous = ctx.data(|data| {
            data.get_temp::<(AccountId, Option<ChatId>, Option<String>, u64)>(owner_id)
        });
        if previous.as_ref() != Some(&owner) {
            if let Some(mut state) = egui::TextEdit::load_state(ctx, id) {
                state.clear_undoer();
                egui::TextEdit::store_state(ctx, id, state);
            }
            ctx.data_mut(|data| data.insert_temp(owner_id, owner));
        }
    }

    fn set_composer_cursor(&self, ctx: &egui::Context, at: usize) {
        let id = egui::Id::new("composer-text");
        let mut state = egui::TextEdit::load_state(ctx, id).unwrap_or_default();
        state
            .cursor
            .set_char_range(Some(egui::text::CCursorRange::one(
                egui::text::CCursor::new(at),
            )));
        egui::TextEdit::store_state(ctx, id, state);
    }

    fn remember_emoji(&mut self, emoji: &str) {
        self.settings.recent_emoji.retain(|known| known != emoji);
        self.settings.recent_emoji.insert(0, emoji.to_owned());
        self.settings.recent_emoji.truncate(36);
        self.mark_settings_dirty();
    }

    /// Pastes the clipboard's text on Shift+Insert, as Linux applications do.
    ///
    /// egui-winit turns only Ctrl/Cmd+V into a `Paste` event, but on Wayland
    /// espanso pastes clipboard-based expansions (non-ASCII text, long
    /// replacements) with Shift+Insert, so without this their trigger is
    /// deleted and nothing appears.
    fn paste_shift_insert(ctx: &egui::Context) {
        if !ctx.text_edit_focused() {
            return;
        }
        let is_shift_insert = |event: &egui::Event| {
            matches!(
                event,
                egui::Event::Key {
                    key: egui::Key::Insert,
                    pressed: true,
                    modifiers,
                    ..
                } if modifiers.shift && !modifiers.command && !modifiers.alt
            )
        };
        if !ctx.input(|input| input.events.iter().any(&is_shift_insert)) {
            return;
        }
        // Tests inject fixtures and must never read the system clipboard.
        let text = if cfg!(test) {
            ctx.data_mut(|data| data.remove_temp::<String>(egui::Id::new("fixture-clipboard-text")))
        } else {
            clipboard_text()
        };
        let text = text.filter(|text| !text.is_empty());
        // Replacing the key event where it is keeps the paste's place in the
        // frame's event order, ahead of anything typed after it.
        ctx.input_mut(|input| {
            input.events.retain_mut(|event| {
                if !is_shift_insert(event) {
                    return true;
                }
                match &text {
                    Some(text) => {
                        *event = egui::Event::Paste(text.clone());
                        true
                    }
                    None => false,
                }
            });
        });
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
        let frame = ctx.cumulative_frame_nr();
        let drop_id = egui::Id::new("dropped-files-input-frame");
        let already_staged = ctx.data(|data| data.get_temp::<u64>(drop_id) == Some(frame));
        if !dropped.is_empty() && !already_staged {
            ctx.data_mut(|data| data.insert_temp(drop_id, frame));
            self.stage_clipboard(ctx, ClipboardPaste::Files(dropped));
        }
        // A text paste reads the clipboard too, and finding no file or
        // picture there is then nothing to report.
        let text = ctx.input(|input| {
            input
                .events
                .iter()
                .any(|event| matches!(event, egui::Event::Paste(_)))
        });
        // Tests inject fixtures and must never read the system clipboard.
        self.take_clipboard_paste(ctx, || {
            let contents = if cfg!(test) {
                clipboard_contents(
                    || {
                        ctx.data_mut(|data| {
                            data.remove_temp::<Vec<PathBuf>>(egui::Id::new(
                                "fixture-clipboard-files",
                            ))
                        })
                        .ok_or_else(|| "no fixture files".to_owned())
                    },
                    || {
                        ctx.data_mut(|data| {
                            data.remove_temp::<(usize, usize, Vec<u8>)>(egui::Id::new(
                                "fixture-clipboard-image",
                            ))
                        })
                        .ok_or_else(|| "no fixture picture".to_owned())
                    },
                )
            } else {
                clipboard_contents(clipboard_files, clipboard_image)
            };
            contents
                .inspect_err(|why| {
                    if !text {
                        // Only the reason: never the paths or the pixels.
                        log::info!("Ctrl+V found nothing to attach: {why}");
                    }
                })
                .ok()
        });
    }

    /// Stages only detected attachments before drawing; queued view actions
    /// remain deferred until the frame ends.
    fn stage_clipboard(&mut self, ctx: &egui::Context, contents: ClipboardPaste) {
        match contents {
            ClipboardPaste::Files(paths) => self.stage_files(paths),
            ClipboardPaste::Image {
                width,
                height,
                rgba,
            } => {
                if self.open_chat.is_some() {
                    self.stage(Pending::Picture {
                        width,
                        height,
                        rgba: std::sync::Arc::new(rgba),
                        texture: None,
                    });
                }
            }
        }
        if !self.pending.is_empty() {
            self.focus_composer = true;
            ctx.memory_mut(|memory| memory.request_focus(egui::Id::new("composer-text")));
        }
    }

    /// Stages pasted files or a pasted picture for the open chat.
    fn take_clipboard_paste(
        &mut self,
        ctx: &egui::Context,
        read_clipboard: impl FnOnce() -> Option<ClipboardPaste>,
    ) {
        let frame = ctx.cumulative_frame_nr();
        let frame_id = egui::Id::new("clipboard-input-frame");
        if let Some((last, staged)) = ctx.data(|data| data.get_temp::<(u64, bool)>(frame_id))
            && last == frame
        {
            if staged {
                ctx.input_mut(|input| {
                    input
                        .events
                        .retain(|event| !matches!(event, egui::Event::Paste(_)))
                });
            }
            return;
        }
        ctx.data_mut(|data| data.insert_temp(frame_id, (frame, false)));
        let requested = ctx.input(|input| self.paste_keys.read(input));
        // A message or button may retain keyboard focus after clicking it.
        // Other text fields still own their clipboard input.
        let editing_text = ctx.text_edit_focused();
        let composing = ctx.memory(|memory| {
            memory.has_focus(egui::Id::new("composer-text"))
                || (!editing_text
                    && !memory.has_focus(egui::Id::new("search"))
                    && !memory.has_focus(egui::Id::new("chat-search")))
        });
        if requested
            && composing
            && self.chat_view()
            && self.dialog.is_none()
            && self.image_preview.is_none()
            && self.picker.is_none()
            && self.reaction_target.is_none()
            && self.recording.is_none()
            && !egui::Popup::is_any_open(ctx)
            && !self.show_update
            && self
                .open_chat
                .as_deref()
                .and_then(|id| self.chat(id))
                .is_some_and(Chat::can_send)
            && let Some(contents) = read_clipboard()
        {
            ctx.data_mut(|data| data.insert_temp(frame_id, (frame, true)));
            // A browser can offer both pixels and its source URL, and a file
            // manager both paths and their text. Consume the text before the
            // composer sees it, keeping any existing caption.
            ctx.input_mut(|input| {
                input
                    .events
                    .retain(|event| !matches!(event, egui::Event::Paste(_)))
            });
            self.stage_clipboard(ctx, contents);
        }
    }

    /// Keeps this frame's scrolling with the pane its gesture began over,
    /// after fastframe-scroll's axis lock and glide have had their say.
    fn route_scroll(&mut self, ctx: &egui::Context) {
        let (moved, lifted) = ctx.input(|input| {
            input
                .events
                .iter()
                .fold((false, false), |(moved, lifted), event| match event {
                    egui::Event::MouseWheel { delta, phase, .. } => (
                        moved || *delta != egui::Vec2::ZERO,
                        lifted || matches!(phase, egui::TouchPhase::End | egui::TouchPhase::Cancel),
                    ),
                    _ => (moved, lifted),
                })
        });
        let gliding = self.scrolling.gliding();
        self.scroll_route.route(ctx, moved, lifted, gliding);
    }

    pub fn save_state(&mut self) {
        if self.settings_dirty {
            self.save_settings();
        }
        for account in &mut self.accounts {
            if account.settings_dirty {
                account.save_settings(&self.files);
            }
        }
    }

    pub fn shutdown(&mut self) {
        self.call_tones.set(None);
        self.save_state();
        self.flush_open_draft();
        self.recording = None;
        // A background resume would die with the process.
        if self.media_hold.take().is_some() {
            crate::media_pause::settle(Duration::from_secs(2));
        }
        for account in &mut self.accounts {
            account.backend.shutdown();
        }
        self.files.flush(fastframe_file::FLUSH_TIMEOUT);
    }

    /// Stores the open chat's unsent text, which otherwise only moves into
    /// the archive when another chat opens.
    fn flush_open_draft(&self) {
        if let Some(chat) = self.open_chat.as_deref()
            && self.editing.is_none()
            && !self.is_untouched_prefill(chat, &self.composer)
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

/// How long after Ctrl/Cmd goes up a V release still completes the paste
/// shortcut: a quick tap often lets go of the modifier first (#494).
const PASTE_RELEASE_GRACE: f64 = 0.5;

/// Follows the Ctrl/Cmd+V shortcut across frames.
///
/// egui-winit swallows the shortcut's key press and sends a `Paste` event
/// only when the clipboard holds text, so on Linux and Windows a picture or
/// file paste shows up only as the V release. That release carries the
/// modifiers of its own moment, which no longer include Ctrl when a quick tap
/// lets go of Ctrl first. So a V release is the shortcut when no V press of
/// its own reached us (a delivered press is typing) and Ctrl/Cmd was down at
/// most [`PASTE_RELEASE_GRACE`] before. macOS's Edit menu sends `Paste` for
/// Cmd+V whatever the clipboard holds, and its release is then ignored.
#[derive(Debug, Default)]
struct PasteKeys {
    /// Ctrl/Cmd is down, as of the last event that said.
    command: bool,
    /// When Ctrl/Cmd was last seen down.
    command_at: Option<f64>,
    /// Shift is down, as of the last modifier event.
    shift: bool,
    /// When Ctrl/Cmd+Shift was last held, for an image-only clipboard's
    /// swallowed press whose modifiers may be released before V.
    plain_at: Option<f64>,
    /// A V press reached us since the last V release: typing, not a paste.
    typed_v: bool,
    /// This shortcut's `Paste` event was handled, so its release must not
    /// read the clipboard again, including after a plain-text paste.
    handled: bool,
}

impl PasteKeys {
    /// Reads one frame's events, in order, and says whether they ask for an
    /// attachment paste: an unshifted `Paste` event, or the release of a
    /// swallowed Ctrl/Cmd+V. Ctrl/Cmd+Shift+V leaves text for its field,
    /// including the native macOS Option+Shift+Cmd+V combination.
    fn read(&mut self, input: &egui::InputState) -> bool {
        let now = input.time;
        if !input.focused {
            *self = Self::default();
            return false;
        }
        if self.command {
            // Still held when this frame's events arrived.
            self.command_at = Some(now);
            if self.shift {
                self.plain_at = Some(now);
            }
        }
        let recent = |at: Option<f64>| at.is_some_and(|at| now - at <= PASTE_RELEASE_GRACE);
        let mut requested = false;
        for event in &input.events {
            match event {
                egui::Event::WindowFocused(_) => *self = Self::default(),
                egui::Event::ModifiersChanged(modifiers) => self.set_modifiers(*modifiers, now),
                egui::Event::Paste(_) => {
                    // egui-winit supplies text for Ctrl/Cmd+Shift+V too.
                    // Leave it for the text field instead of staging pixels.
                    requested |= !(self.command && self.shift);
                    // A menu paste has no key release to wait for. The Edit
                    // menu's Cmd+V may arrive after Cmd is already up.
                    self.handled = recent(self.command_at);
                }
                egui::Event::Key {
                    key,
                    pressed,
                    modifiers,
                    ..
                } => {
                    self.set_modifiers(*modifiers, now);
                    if *key != egui::Key::V {
                        continue;
                    }
                    if *pressed {
                        self.typed_v = true;
                        continue;
                    }
                    requested |= !self.typed_v
                        && !self.handled
                        && recent(self.command_at)
                        && !recent(self.plain_at);
                    self.typed_v = false;
                    self.handled = false;
                    self.plain_at = None;
                    if !self.command {
                        // The next release needs a Ctrl/Cmd of its own.
                        self.command_at = None;
                    }
                }
                _ => {}
            }
        }
        requested
    }

    fn set_modifiers(&mut self, modifiers: egui::Modifiers, now: f64) {
        let down = modifiers.command;
        if down {
            if !self.command {
                // A new press starts another shortcut, including after a
                // menu paste that had no V release.
                self.handled = false;
                self.plain_at = None;
            }
            self.command_at = Some(now);
            if modifiers.shift {
                self.plain_at = Some(now);
            }
        }
        self.command = down;
        self.shift = modifiers.shift;
    }
}

/// What a paste into the composer stages.
#[derive(Debug)]
enum ClipboardPaste {
    /// Files copied in a file manager, staged like dropped files.
    Files(Vec<PathBuf>),
    /// Picture data as width, height, and straight-alpha RGBA.
    Image {
        width: usize,
        height: usize,
        rgba: Vec<u8>,
    },
}

/// Prefers the clipboard's file list over its picture: Finder, Explorer and
/// Linux file managers offer the file's icon as an image alongside the path,
/// so a copied PDF or ZIP would otherwise arrive as its icon (#285).
///
/// Says why when there is neither, without naming any file, so a paste that
/// stages nothing leaves its reason in the log (#404).
fn clipboard_contents(
    read_files: impl FnOnce() -> Result<Vec<PathBuf>, String>,
    read_image: impl FnOnce() -> Result<(usize, usize, Vec<u8>), String>,
) -> Result<ClipboardPaste, String> {
    let listed = read_files();
    let files: Vec<PathBuf> = listed
        .as_ref()
        .map(|paths| {
            paths
                .iter()
                .filter_map(|path| {
                    // text/uri-list lines end in CRLF, and arboard splits only on LF.
                    let path = match path.to_str() {
                        Some(text) if text.ends_with('\r') => {
                            PathBuf::from(text.trim_end_matches('\r'))
                        }
                        _ => path.clone(),
                    };
                    path.exists().then_some(path)
                })
                .collect()
        })
        .unwrap_or_default();
    if !files.is_empty() {
        return Ok(ClipboardPaste::Files(files));
    }
    let files = match &listed {
        Ok(paths) => format!("{} copied files, none found on disk", paths.len()),
        Err(why) => format!("no copied files ({why})"),
    };
    match read_image() {
        Ok((width, height, rgba)) => Ok(ClipboardPaste::Image {
            width,
            height,
            rgba,
        }),
        Err(why) => Err(format!("{files}, no picture ({why})")),
    }
}

/// The clipboard's text, if it holds any.
fn clipboard_text() -> Option<String> {
    arboard::Clipboard::new().ok()?.get_text().ok()
}

/// Files copied to the clipboard by a file manager.
fn clipboard_files() -> Result<Vec<PathBuf>, String> {
    let mut clipboard = arboard::Clipboard::new().map_err(|error| error.to_string())?;
    clipboard
        .get()
        .file_list()
        .map_err(|error| error.to_string())
}

/// Clipboard image as width, height, and straight-alpha RGBA.
fn clipboard_image() -> Result<(usize, usize, Vec<u8>), String> {
    let mut clipboard = arboard::Clipboard::new().map_err(|error| error.to_string())?;
    let image = clipboard.get_image().map_err(|error| error.to_string())?;
    if image.width == 0 || image.height == 0 {
        return Err("the picture is empty".to_owned());
    }
    Ok((image.width, image.height, image.bytes.into_owned()))
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

/// What may still happen while the app lock is on: the lock screen's own
/// actions, the window and the tray, and settings results that show
/// nothing. Everything else would show or change what the lock hides.
fn allowed_while_locked(action: &Action) -> bool {
    matches!(
        action,
        Action::UnlockApp
            | Action::LockApp
            | Action::ForgotAppPassword(_)
            | Action::UnlinkLockedApp
            | Action::ShowWindow
            | Action::HideWindow
            | Action::CloseWindow
            | Action::Quit
            | Action::ReloadThemes
            | Action::ZoomBy(_)
            | Action::ResetZoom
            | Action::SettingsChanged
            | Action::SetChatSound { .. }
            | Action::SetNotificationSound { .. }
            | Action::SetDownloadFolder(_)
    )
}

/// Whether an input event is someone using the window, which restarts the
/// app lock's inactivity count. Focus changes and the pointer leaving are
/// the desktop's doing, not the person's.
fn is_user_input(event: &egui::Event) -> bool {
    matches!(
        event,
        egui::Event::Key { .. }
            | egui::Event::Text(_)
            | egui::Event::Copy
            | egui::Event::Cut
            | egui::Event::Paste(_)
            | egui::Event::PointerMoved(_)
            | egui::Event::MouseMoved(_)
            | egui::Event::PointerButton { .. }
            | egui::Event::MouseWheel { .. }
            | egui::Event::Zoom(_)
            | egui::Event::Touch { .. }
            | egui::Event::Ime(_)
    )
}

/// Whether the session is Hyprland, when both its instance signature and the
/// desktop name say so, since either alone can be left over from another
/// session (the rule `backdrop` uses). On Wayland an attention request is an
/// xdg-activation request, and Hyprland focuses the window for it when
/// `misc:focus_on_activate` is on, as Omarchy sets it: a window in a special
/// workspace then opens that workspace over what the user is doing (#530).
fn hyprland(signature: bool, desktop: Option<&str>) -> bool {
    signature
        && desktop.is_some_and(|desktop| {
            desktop
                .split(':')
                .any(|name| name.trim().eq_ignore_ascii_case("Hyprland"))
        })
}

fn notification_eligible(chat: &Chat, now: i64, message_at: i64) -> bool {
    if chat.archived || chat.unread == 0 || chat.muted(now) || chat.locked {
        return false;
    }
    now - message_at <= 60
}

mod calls;
pub use calls::CallPerson;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ChatKind, Content, Dialog, Media, MediaState, ReplyTarget, ToastKind};

    fn composer_undo_boundary_frame(
        app: &mut App,
        ctx: &egui::Context,
        time: f64,
        events: Vec<egui::Event>,
    ) {
        let id = egui::Id::new("composer-text");
        ctx.memory_mut(|memory| memory.request_focus(id));
        if time == 0.0 {
            app.attach(ctx);
        }
        app.link = crate::backend::LinkStatus::Connected;
        let mut output = ctx.run_ui(
            egui::RawInput {
                time: Some(time),
                events,
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1180.0, 780.0),
                )),
                ..Default::default()
            },
            |ui| app.frame_ui(ui),
        );
        output.textures_delta.clear();
    }

    fn composer_undo_key() -> egui::Event {
        egui::Event::Key {
            key: egui::Key::Z,
            physical_key: Some(egui::Key::Z),
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        }
    }

    /// Real keyboard undo cannot restore a previous chat/account's text
    /// through the production composer, even with a stored checkpoint.
    #[test]
    fn editor_undo_cannot_restore_text_from_another_chat_or_account() {
        for switch_account in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let (mut app, _) = two_accounts(directory.path());
            for account in &mut app.accounts {
                account
                    .chats
                    .push(Chat::new("source@s.whatsapp.net".into(), "Source".into()));
                account
                    .chats
                    .push(Chat::new("target@s.whatsapp.net".into(), "Target".into()));
            }
            let ctx = egui::Context::default();
            let marker = "Synthetic source account draft";
            app.open_chat("source@s.whatsapp.net".into());
            composer_undo_boundary_frame(&mut app, &ctx, 0.0, vec![]);
            composer_undo_boundary_frame(
                &mut app,
                &ctx,
                0.1,
                vec![egui::Event::Text(marker.into())],
            );
            // egui's default stable_time is 1 second. No sleeping is needed.
            composer_undo_boundary_frame(&mut app, &ctx, 1.2, vec![]);
            assert_eq!(app.composer, marker);
            if switch_account {
                let second = app.accounts[1].id.clone();
                app.switch_account(&second);
                app.open_chat("source@s.whatsapp.net".into());
            } else {
                app.open_chat("target@s.whatsapp.net".into());
            }
            assert!(app.composer.is_empty());
            composer_undo_boundary_frame(&mut app, &ctx, 1.3, vec![composer_undo_key()]);
            assert!(
                !app.composer.contains(marker),
                "Ctrl/Cmd+Z crossed the composer ownership boundary"
            );
        }
    }

    /// Normal edits within one ownership lifetime keep ordinary undo/redo.
    #[test]
    fn composer_owner_isolation_preserves_same_chat_undo_and_redo() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.chats
            .push(Chat::new("source@s.whatsapp.net".into(), "Source".into()));
        app.open_chat("source@s.whatsapp.net".into());
        composer_undo_boundary_frame(&mut app, &ctx, 0.0, vec![]);
        composer_undo_boundary_frame(&mut app, &ctx, 0.1, vec![egui::Event::Text("alpha".into())]);
        composer_undo_boundary_frame(&mut app, &ctx, 1.2, vec![]);
        composer_undo_boundary_frame(&mut app, &ctx, 1.3, vec![egui::Event::Text(" beta".into())]);
        composer_undo_boundary_frame(&mut app, &ctx, 1.4, vec![composer_undo_key()]);
        assert_eq!(app.composer, "alpha");
        let mut redo = composer_undo_key();
        if let egui::Event::Key { modifiers, .. } = &mut redo {
            modifiers.shift = true;
        }
        composer_undo_boundary_frame(&mut app, &ctx, 1.5, vec![redo]);
        assert_eq!(app.composer, "alpha beta");
    }

    /// Reusing exactly the same ids cannot revive history from a composer
    /// closed or unlinked between frames.
    #[test]
    fn composer_undo_resets_on_close_reopen_and_logout_with_reused_ids() {
        for logout in [false, true] {
            let mut app = app();
            let ctx = egui::Context::default();
            let chat = "source@s.whatsapp.net";
            let marker = "Synthetic previous lifetime";
            app.chats.push(Chat::new(chat.into(), "Source".into()));
            app.open_chat(chat.into());
            composer_undo_boundary_frame(&mut app, &ctx, 0.0, vec![]);
            composer_undo_boundary_frame(
                &mut app,
                &ctx,
                0.1,
                vec![egui::Event::Text(marker.into())],
            );
            composer_undo_boundary_frame(&mut app, &ctx, 1.2, vec![]);
            if logout {
                app.handle_link(LinkStatus::LoggedOut, true);
                app.handle_link(LinkStatus::Connected, true);
                app.chats.push(Chat::new(chat.into(), "New session".into()));
                app.open_chat(chat.into());
                composer_undo_boundary_frame(
                    &mut app,
                    &ctx,
                    1.3,
                    vec![egui::Event::Text("Fresh draft".into())],
                );
                composer_undo_boundary_frame(&mut app, &ctx, 1.4, vec![composer_undo_key()]);
                assert!(
                    !app.composer.contains(marker),
                    "a new session restored old editor history"
                );
            } else {
                app.apply(Action::CloseChat, &ctx);
                app.open_chat(chat.into());
                assert_eq!(app.composer, marker);
                composer_undo_boundary_frame(&mut app, &ctx, 1.3, vec![composer_undo_key()]);
                assert_eq!(
                    app.composer, marker,
                    "closing/reopening starts fresh editor history"
                );
            }
        }
    }

    /// Editing owns a distinct history from another message and the draft.
    #[test]
    fn composer_undo_cannot_cross_edit_targets_or_return_to_draft() {
        let mut app = app();
        let ctx = egui::Context::default();
        let chat = "source@s.whatsapp.net";
        app.chats.push(Chat::new(chat.into(), "Source".into()));
        let mut first = message(chat, "first", crate::util::now());
        first.from_me = true;
        first.content = Content::text("First edit target");
        let mut second = message(chat, "second", crate::util::now());
        second.from_me = true;
        second.content = Content::text("Second edit target");
        app.conversations.insert(
            chat.into(),
            Conversation {
                messages: vec![first, second],
                requested: true,
                ..Default::default()
            },
        );
        app.open_chat(chat.into());
        composer_undo_boundary_frame(&mut app, &ctx, 0.0, vec![]);
        app.apply(Action::Edit("first".into()), &ctx);
        composer_undo_boundary_frame(&mut app, &ctx, 0.1, vec![]);
        composer_undo_boundary_frame(&mut app, &ctx, 1.2, vec![]);
        assert_eq!(app.composer, "First edit target");
        app.apply(Action::Edit("second".into()), &ctx);
        composer_undo_boundary_frame(&mut app, &ctx, 1.3, vec![composer_undo_key()]);
        assert_eq!(app.composer, "Second edit target");
        app.apply(Action::CancelEdit, &ctx);
        composer_undo_boundary_frame(&mut app, &ctx, 1.4, vec![composer_undo_key()]);
        assert!(
            app.composer.is_empty(),
            "draft undo cannot restore either edited message"
        );
    }

    /// Events for an account off screen cannot discard the active editor's
    /// normal undo checkpoints, including removal that changes the roster.
    #[test]
    fn hidden_account_logout_and_removal_preserve_active_composer_undo() {
        for remove in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let (mut app, _) = two_accounts(directory.path());
            let ctx = egui::Context::default();
            let chat = "source@s.whatsapp.net";
            app.chats.push(Chat::new(chat.into(), "Source".into()));
            app.open_chat(chat.into());
            let (backend, events) = Backend::detached();
            app.accounts[1].backend = backend;
            composer_undo_boundary_frame(&mut app, &ctx, 0.0, vec![]);
            composer_undo_boundary_frame(
                &mut app,
                &ctx,
                0.1,
                vec![egui::Event::Text("alpha".into())],
            );
            composer_undo_boundary_frame(&mut app, &ctx, 1.2, vec![]);
            composer_undo_boundary_frame(
                &mut app,
                &ctx,
                1.3,
                vec![egui::Event::Text(" beta".into())],
            );
            events
                .send(if remove {
                    Event::AccountRemoved
                } else {
                    Event::Link(LinkStatus::LoggedOut)
                })
                .unwrap();
            app.handle_events();
            composer_undo_boundary_frame(&mut app, &ctx, 1.4, vec![composer_undo_key()]);
            assert_eq!(
                app.composer, "alpha",
                "hidden account event erased active undo"
            );
        }
    }

    #[test]
    fn toasts_errors_and_connection_changes_are_announced() {
        let root = tempfile::tempdir().unwrap();
        let (mut app, _events) = App::headless(AppDirs::under(root.path()), Settings::default());
        app.toast("Copied");
        app.toast_error("Could not send");
        let ctx = egui::Context::default();
        app.announcer.show(&ctx, true);
        // Errors are read first.
        assert_eq!(app.announcer.spoken()[0], "Could not send. Copied");

        app.link = LinkStatus::Connected;
        app.handle_link(
            LinkStatus::Disconnected {
                reason: "fixture".into(),
            },
            true,
        );
        app.announcer.show(&ctx, true);
        assert!(app.announcer.spoken().contains(&"Connection lost"));
        app.handle_link(LinkStatus::Connected, true);
        app.announcer.show(&ctx, true);
        assert!(
            app.announcer
                .spoken()
                .iter()
                .any(|text| text.starts_with("Back online"))
        );
    }

    #[test]
    fn connecting_is_announced_once_per_offline_episode() {
        let root = tempfile::tempdir().unwrap();
        let (mut app, _events) = App::headless(AppDirs::under(root.path()), Settings::default());
        let ctx = egui::Context::default();
        let mut spoken = Vec::new();
        app.link = LinkStatus::Connected;
        for status in [
            LinkStatus::Disconnected {
                reason: "fixture".into(),
            },
            LinkStatus::Connecting,
            LinkStatus::Disconnected {
                reason: "fixture".into(),
            },
            LinkStatus::Connecting,
            LinkStatus::Connected,
        ] {
            let before: Vec<String> = app.announcer.spoken().map(String::from).into();
            app.handle_link(status, true);
            app.announcer.show(&ctx, true);
            // A slot that changed this frame holds what was just spoken.
            for (old, new) in before.iter().zip(app.announcer.spoken()) {
                if old != new {
                    spoken.push(new.trim_end_matches('\u{200B}').to_string());
                }
            }
        }
        assert_eq!(spoken.iter().filter(|t| *t == "Connection lost").count(), 1);
        assert_eq!(spoken.iter().filter(|t| *t == "Connecting").count(), 1);
        assert!(spoken.last().unwrap().starts_with("Back online"));
    }

    #[test]
    fn startup_connection_is_not_announced() {
        let root = tempfile::tempdir().unwrap();
        let (mut app, _events) = App::headless(AppDirs::under(root.path()), Settings::default());
        app.handle_link(LinkStatus::Connecting, true);
        app.handle_link(LinkStatus::Connected, true);
        app.announcer.show(&egui::Context::default(), true);
        assert_eq!(app.announcer.spoken(), ["", ""]);
    }

    fn app() -> App {
        let root = std::env::temp_dir().join(format!("zapfast-app-{}", std::process::id()));
        App::headless(AppDirs::under(&root), Settings::default()).0
    }

    #[test]
    fn gif_pages_append_retry_and_ignore_overtaken_searches() {
        let mut app = app();
        let ctx = egui::Context::default();
        let gif = |id: &str| Gif {
            id: id.into(),
            still: None,
            mp4: "https://example.test/gif.mp4".into(),
            width: 100,
            height: 100,
        };
        let page = |request, offset, ids: &[&str], next_offset| Event::Gifs {
            query: "cats".into(),
            request,
            offset,
            results: Ok(crate::model::GifPage {
                gifs: ids.iter().map(|id| gif(id)).collect(),
                next_offset,
            }),
        };
        app.apply(Action::SearchGifs("cats".into()), &ctx);
        let first = app.gif_request;
        app.apply(Action::SearchGifs("cats".into()), &ctx);
        let current = app.gif_request;
        app.apply_backend_event(page(first, 0, &["stale"], None), false);
        assert!(app.gif_pending);
        assert!(app.gif_results.is_empty());
        app.apply_backend_event(page(current, 0, &["one", "two"], Some(24)), false);
        app.apply(Action::MoreGifs, &ctx);
        let more = app.gif_request;
        app.apply(Action::MoreGifs, &ctx);
        assert_eq!(app.gif_request, more, "one page request at a time");
        app.apply_backend_event(
            Event::Gifs {
                query: "cats".into(),
                offset: 24,
                request: more,
                results: Err(GifError {
                    message: "offline".into(),
                    bad_key: false,
                }),
            },
            false,
        );
        assert_eq!(app.gif_results.len(), 2);
        assert_eq!(app.gif_next_offset, Some(24));
        app.apply(Action::MoreGifs, &ctx);
        app.apply_backend_event(page(app.gif_request, 24, &["two", "three"], None), false);
        assert_eq!(
            app.gif_results
                .iter()
                .map(|gif| gif.id.as_str())
                .collect::<Vec<_>>(),
            ["one", "two", "three"]
        );
        assert!(!app.gif_pending);
        assert!(app.gif_error.is_none());
        assert!(app.gif_next_offset.is_none());
        app.apply(Action::SearchGifs("cats".into()), &ctx);
        let old_account = app.gif_request;
        app.apply_backend_event(page(old_account, 0, &["old-account"], Some(24)), false);
        app.apply(Action::MoreGifs, &ctx);
        let late = app.gif_request;
        app.clear_account_ui();
        assert!(app.gif_results.is_empty());
        assert!(app.gif_query.is_empty());
        assert!(app.gif_next_offset.is_none());
        assert!(!app.gif_pending);
        app.apply(Action::SearchGifs("cats".into()), &ctx);
        app.apply_backend_event(page(late, 24, &["late-old-account"], None), false);
        assert!(app.gif_pending);
        assert!(app.gif_results.is_empty());
        app.apply_backend_event(page(app.gif_request, 0, &["new-account"], None), false);
        assert_eq!(app.gif_results[0].id, "new-account");
    }

    /// While recording, the light, the timer and the waveform ask for a
    /// frame every 40 ms, or every step while a screen reader is attached,
    /// and the recorder then stops asking for frames of its own.
    #[test]
    fn recording_progress_steps_while_a_screen_reader_is_attached() {
        let run = |reader: bool| {
            let directory = tempfile::tempdir().unwrap();
            let mut app = App::headless(AppDirs::under(directory.path()), Settings::default()).0;
            let ctx = crate::motion::tests::reading(reader);
            app.waker.attach(&ctx);
            app.recording = Some(Recorder::rehearsal());
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| app.tick_audio(ui.ctx()));
            output.textures_delta.clear();
            let stepped = app.recording.as_ref().is_some_and(Recorder::is_stepped);
            (
                output.viewport_output[&egui::ViewportId::ROOT].repaint_delay,
                stepped,
            )
        };
        let predicted = Duration::from_secs_f32(1.0 / 60.0);
        assert_eq!(run(false), (Duration::from_millis(40) - predicted, false));
        assert_eq!(run(true), (crate::motion::STEP - predicted, true));
    }

    #[test]
    fn back_and_forward_walk_between_chats_and_settings() {
        let ctx = egui::Context::default();
        let mut app = app();
        let chat = "15550008888@s.whatsapp.net";
        app.chats.push(Chat::new(chat.into(), "Ada".into()));
        // The first frame fixes the place the window opened on.
        app.apply_actions(&ctx);

        let open = |app: &mut App, action: Action| {
            app.actions.push(action);
            app.apply_actions(&ctx);
        };
        open(&mut app, Action::OpenChat(chat.into()));
        assert_eq!(app.open_chat.as_deref(), Some(chat));
        open(&mut app, Action::Open(Page::Settings));
        assert_eq!(app.page, Page::Settings);
        assert_eq!(app.open_chat.as_deref(), Some(chat));

        app.apply(Action::NavigateBack, &ctx);
        assert_eq!(app.page, Page::Chats, "back leaves Settings");
        assert_eq!(app.open_chat.as_deref(), Some(chat));
        app.apply(Action::NavigateBack, &ctx);
        assert_eq!(app.open_chat, None, "back leaves the chat");

        app.apply(Action::NavigateForward, &ctx);
        assert_eq!(app.open_chat.as_deref(), Some(chat), "forward reopens it");
        app.apply(Action::NavigateForward, &ctx);
        assert_eq!(app.page, Page::Settings);
        assert_eq!(app.open_chat.as_deref(), Some(chat));
    }

    #[test]
    fn a_dialog_is_not_a_place_in_the_history() {
        let ctx = egui::Context::default();
        let mut app = app();
        let chat = "15550007777@s.whatsapp.net";
        app.chats.push(Chat::new(chat.into(), "Ada".into()));
        app.apply_actions(&ctx);

        app.actions.push(Action::OpenChat(chat.into()));
        app.apply_actions(&ctx);
        app.actions.push(Action::ShowDialog(Dialog::NewChat));
        app.apply_actions(&ctx);
        app.actions.push(Action::CloseDialog);
        app.apply_actions(&ctx);

        // The dialog never became a place, so one back step reaches the list.
        app.apply(Action::NavigateBack, &ctx);
        assert_eq!(app.open_chat, None);
        assert_eq!(app.page, Page::Chats);
        assert_eq!(app.dialog, None, "the dialog is not restored");
    }

    #[test]
    fn a_deleted_chat_is_not_reopened_from_the_history() {
        let ctx = egui::Context::default();
        let mut app = app();
        let chat = "15550007777@s.whatsapp.net";
        app.chats.push(Chat::new(chat.into(), "Ada".into()));
        app.apply_actions(&ctx);

        app.actions.push(Action::OpenChat(chat.into()));
        app.apply_actions(&ctx);
        app.actions.push(Action::Open(Page::Settings));
        app.apply_actions(&ctx);

        // The phone deletes the chat while the window is elsewhere.
        app.forget_chat(chat);
        app.apply_actions(&ctx);
        assert_eq!(app.open_chat, None);

        // The step skips the removed chat instead of reopening it.
        app.apply(Action::NavigateBack, &ctx);
        assert_eq!(app.page, Page::Chats);
        assert_eq!(app.open_chat, None);
    }

    #[test]
    fn a_gone_chat_is_never_reopened_from_a_stale_place() {
        let ctx = egui::Context::default();
        let mut app = app();
        let chat = "15550007777@s.whatsapp.net";
        app.chats.push(Chat::new(chat.into(), "Ada".into()));
        app.apply_actions(&ctx);
        app.actions.push(Action::OpenChat(chat.into()));
        app.apply_actions(&ctx);
        app.actions.push(Action::Open(Page::Settings));
        app.apply_actions(&ctx);

        // Defence in depth: even when a path clears chats without pruning the
        // history, a place naming a gone chat is not opened.
        app.chats.clear();
        app.open_chat = None;
        app.apply(Action::NavigateBack, &ctx);
        app.apply(Action::NavigateBack, &ctx);
        assert_eq!(app.open_chat, None);
        assert_eq!(app.page, Page::Chats);
    }

    /// Two accounts sharing `chat`: the first on screen, with `chat` opened
    /// and Settings over it, and the second's event channel.
    fn two_accounts_on_settings(
        directory: &std::path::Path,
        chat: &str,
    ) -> (App, std::sync::mpsc::Sender<crate::backend::Event>) {
        let ctx = egui::Context::default();
        let dirs = AppDirs::under(directory);
        let mut app = App::headless(dirs.clone(), Settings::default()).0;
        let (second, events) = Account::detached(
            &dirs,
            AccountId::parse("2").unwrap(),
            crate::settings::AccountSettings::default(),
        )
        .unwrap();
        app.accounts.push(second);
        app.active = 0;
        app.chats.push(Chat::new(chat.into(), "Ada".into()));
        app.accounts[1]
            .chats
            .push(Chat::new(chat.into(), "Ada".into()));
        app.apply_actions(&ctx);
        app.actions.push(Action::OpenChat(chat.into()));
        app.apply_actions(&ctx);
        app.actions.push(Action::Open(Page::Settings));
        app.apply_actions(&ctx);
        (app, events)
    }

    #[test]
    fn a_hidden_accounts_chat_removal_leaves_the_history_alone() {
        let directory = tempfile::tempdir().unwrap();
        let chat = "15550007777@s.whatsapp.net";
        let (mut app, events) = two_accounts_on_settings(directory.path(), chat);
        let ctx = egui::Context::default();

        events
            .send(crate::backend::Event::ChatRemoved { chat: chat.into() })
            .unwrap();
        app.handle_events();

        // The chat is gone from the hidden account only, and chat ids repeat
        // across accounts, so the window's own place still names it.
        app.apply(Action::NavigateBack, &ctx);
        assert_eq!(app.page, Page::Chats);
        assert_eq!(app.open_chat.as_deref(), Some(chat));
    }

    #[test]
    fn a_hidden_accounts_logout_leaves_the_history_alone() {
        let directory = tempfile::tempdir().unwrap();
        let chat = "15550007777@s.whatsapp.net";
        let (mut app, events) = two_accounts_on_settings(directory.path(), chat);
        let ctx = egui::Context::default();

        events
            .send(crate::backend::Event::Link(LinkStatus::LoggedOut))
            .unwrap();
        app.handle_events();

        app.apply(Action::NavigateBack, &ctx);
        assert_eq!(
            app.page,
            Page::Chats,
            "back still leaves the window's own Settings"
        );
        assert_eq!(app.open_chat.as_deref(), Some(chat));
    }

    #[test]
    fn a_gone_chat_keeps_the_forward_history() {
        let ctx = egui::Context::default();
        let mut app = app();
        let gone = "15550007771@s.whatsapp.net";
        let kept = "15550007772@s.whatsapp.net";
        app.chats.push(Chat::new(gone.into(), "Ada".into()));
        app.chats.push(Chat::new(kept.into(), "Grace".into()));
        app.apply_actions(&ctx);
        app.actions.push(Action::OpenChat(gone.into()));
        app.apply_actions(&ctx);
        app.actions.push(Action::OpenChat(kept.into()));
        app.apply_actions(&ctx);
        app.actions.push(Action::Open(Page::Settings));
        app.apply_actions(&ctx);

        // A path that drops a chat without pruning the history, which the
        // guard in `show_location` exists for.
        app.chats.retain(|chat| chat.id != gone);

        app.actions.push(Action::NavigateBack);
        app.apply_actions(&ctx);
        assert_eq!(app.open_chat.as_deref(), Some(kept));
        app.actions.push(Action::NavigateBack);
        app.apply_actions(&ctx);
        assert_eq!(app.open_chat, None, "the gone chat is not reopened");

        // The step that skipped the gone chat must not drop where a forward
        // step would go.
        app.actions.push(Action::NavigateForward);
        app.apply_actions(&ctx);
        assert_eq!(app.open_chat.as_deref(), Some(kept));
    }

    #[test]
    fn a_locked_chat_outside_its_folder_does_not_strand_the_history() {
        let ctx = egui::Context::default();
        let mut app = app();
        app.settings.set_chat_lock_code(Some("123456"));
        let locked = "15550007774@s.whatsapp.net";
        let mut chat = Chat::new(locked.into(), "Ada".into());
        chat.locked = true;
        app.chats.push(chat);
        app.apply_actions(&ctx);

        // The reader opens the locked chat from the locked folder, then opens
        // Settings over it.
        app.apply(Action::UnlockLockedFolder("123456".into()), &ctx);
        assert!(app.locked_folder_open());
        app.actions.push(Action::OpenChat(locked.into()));
        app.apply_actions(&ctx);
        app.actions.push(Action::Open(Page::Settings));
        app.apply_actions(&ctx);
        assert_eq!(app.open_chat.as_deref(), Some(locked));

        // Hiding the window closes the folder and drops the code, so the chat
        // can no longer be opened from the history.
        app.window_gone();
        app.apply_actions(&ctx);
        assert_eq!(app.open_chat, None, "the locked chat is hidden again");

        // Back must step past the place it cannot show in one press, and the
        // way forward must survive the step.
        app.actions.push(Action::NavigateBack);
        app.apply_actions(&ctx);
        assert_eq!(app.page, Page::Chats, "one press past the locked place");
        assert_eq!(app.open_chat, None);

        app.actions.push(Action::NavigateForward);
        app.apply_actions(&ctx);
        assert_eq!(app.page, Page::Settings, "forward still reaches Settings");
    }

    #[test]
    fn a_new_place_drops_the_forward_history() {
        let ctx = egui::Context::default();
        let mut app = app();
        let first = "15550009991@s.whatsapp.net";
        let second = "15550009992@s.whatsapp.net";
        app.chats.push(Chat::new(first.into(), "Ada".into()));
        app.chats.push(Chat::new(second.into(), "Grace".into()));
        app.apply_actions(&ctx);

        app.actions.push(Action::OpenChat(first.into()));
        app.apply_actions(&ctx);
        app.apply(Action::NavigateBack, &ctx);
        assert_eq!(app.open_chat, None);

        app.actions.push(Action::OpenChat(second.into()));
        app.apply_actions(&ctx);
        app.apply(Action::NavigateForward, &ctx);
        assert_eq!(
            app.open_chat.as_deref(),
            Some(second),
            "forward no longer reaches the chat the back step left"
        );
    }

    #[test]
    fn switching_accounts_forgets_the_history() {
        let directory = tempfile::tempdir().unwrap();
        let (mut app, _commands) = two_accounts(directory.path());
        let ctx = egui::Context::default();
        let chat = "15550003333@s.whatsapp.net";
        app.chats.push(Chat::new(chat.into(), "Ada".into()));
        app.apply_actions(&ctx);
        app.actions.push(Action::OpenChat(chat.into()));
        app.apply_actions(&ctx);
        app.apply(Action::SwitchAccount(AccountId::parse("2").unwrap()), &ctx);
        assert_eq!(app.open_chat, None, "the other account has no chat open");
        app.apply(Action::NavigateBack, &ctx);
        assert_eq!(
            app.open_chat, None,
            "the first account's chat is not restored"
        );
    }

    /// The Calls tab stays open across a switch of account and shows that
    /// account's calls; what each account had chosen there stays its own.
    #[test]
    fn the_calls_tab_follows_a_switch_of_account() {
        let directory = tempfile::tempdir().unwrap();
        let (mut app, mut commands) = two_accounts(directory.path());
        let ctx = egui::Context::default();
        app.apply(Action::Open(Page::Calls), &ctx);
        app.apply(
            Action::PickCall {
                chat: "15550003333@s.whatsapp.net".into(),
                message: "call-1".into(),
            },
            &ctx,
        );
        app.apply(Action::ShowNewCall(true), &ctx);
        assert!(app.new_call);
        app.apply(Action::SwitchAccount(AccountId::parse("2").unwrap()), &ctx);
        assert_eq!(app.page, Page::Calls);
        assert!(
            std::iter::from_fn(|| commands.try_recv().ok())
                .any(|command| matches!(command, Command::LoadCallLog)),
            "the second account reads its own calls"
        );
        assert!(!app.new_call && app.picked_call.is_none());
        assert!(app.accounts[0].new_call && app.accounts[0].picked_call.is_some());
        // Coming back to the tab from elsewhere starts at the history.
        app.apply(Action::SwitchAccount(AccountId::parse("1").unwrap()), &ctx);
        app.apply(Action::Open(Page::Chats), &ctx);
        app.apply(Action::Open(Page::Calls), &ctx);
        assert!(!app.new_call);
        assert!(
            app.picked_call.is_none(),
            "a narrow window opens the tab on its history, not the last picked chat"
        );
        assert_eq!(app.page, Page::Calls);
    }

    #[test]
    fn a_back_side_button_press_steps_back_through_a_frame() {
        let ctx = egui::Context::default();
        let mut app = app();
        app.attach(&ctx);
        let chat = "15550007777@s.whatsapp.net";
        app.chats.push(Chat::new(chat.into(), "Ada".into()));
        app.apply_actions(&ctx);
        app.actions.push(Action::OpenChat(chat.into()));
        app.apply_actions(&ctx);
        assert_eq!(app.open_chat.as_deref(), Some(chat));

        let press = |app: &mut App, button: egui::PointerButton| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    events: vec![egui::Event::PointerButton {
                        pos: egui::pos2(20.0, 20.0),
                        button,
                        pressed: true,
                        modifiers: egui::Modifiers::NONE,
                    }],
                    ..Default::default()
                },
                |ui| app.frame_ui(ui),
            );
            output.textures_delta.clear();
        };
        press(&mut app, egui::PointerButton::Extra1);
        assert_eq!(app.open_chat, None, "the back button closed the chat");
        press(&mut app, egui::PointerButton::Extra2);
        assert_eq!(
            app.open_chat.as_deref(),
            Some(chat),
            "the forward button reopened it"
        );
    }

    /// A cropped picture keeps its original while it waits in another account's draft.
    #[test]
    fn an_original_survives_a_picture_waiting_in_another_account() {
        let directory = tempfile::tempdir().unwrap();
        let dirs = AppDirs::under(directory.path());
        let mut app = App::headless(dirs.clone(), Settings::default()).0;
        let second_id = AccountId::parse("2").unwrap();
        let (second, _) = Account::detached(
            &dirs,
            second_id.clone(),
            crate::settings::AccountSettings::default(),
        )
        .unwrap();
        app.accounts.push(second);
        let chat = "fixture@g.us";
        for account in &mut app.accounts {
            account
                .chats
                .push(Chat::new(chat.into(), "Synthetic group".into()));
        }
        app.open_chat(chat.into());
        let source = std::path::PathBuf::from("/fixture/photo.png");
        let target = app.stage(Pending::File(source.clone()));
        app.picture_origins.insert(
            target,
            crate::model::PictureOrigin {
                source: crate::model::PictureSource::File(source),
                width: 8,
                height: 6,
                crop: crate::model::PictureCrop::full(8, 6),
                turns: 0,
            },
        );
        let first_id = app.account().id.clone();
        // Away to the other account, where this picture waits in a draft.
        app.switch_account(&second_id);
        assert!(
            app.picture_origins.contains_key(&target),
            "parked in a draft, so not forgotten yet"
        );
        // And back, where it must still know the original it was cropped from.
        app.switch_account(&first_id);
        assert!(
            app.picture_origins.contains_key(&target),
            "coming back kept the original its picture was cropped from"
        );
        assert!(
            app.pending.iter().any(|staged| staged.id == target),
            "and the picture itself came back with it"
        );
    }

    /// Identical chat IDs across accounts retain separate files, captions, mentions and reply quotes.
    #[test]
    fn attachment_drafts_keep_their_account_caption_mentions_and_quote() {
        let directory = tempfile::tempdir().unwrap();
        let dirs = AppDirs::under(directory.path());
        let mut app = App::headless(dirs.clone(), Settings::default()).0;
        let second_id = AccountId::parse("2").unwrap();
        let (second, _) = Account::detached(
            &dirs,
            second_id.clone(),
            crate::settings::AccountSettings::default(),
        )
        .unwrap();
        app.accounts.push(second);
        let chat = "fixture@g.us";
        let member = "fixture-member@s.whatsapp.net";
        for account in &mut app.accounts {
            let mut group = Chat::new(chat.into(), "Synthetic group".into());
            group.participants.push(member.into());
            account.chats.push(group);
        }
        app.open_chat(chat.into());
        app.composer = "Hello @Mira Example".into();
        app.composer_mentions.push(ComposerMention {
            id: member.into(),
            name: "Mira Example".into(),
        });
        app.reply_to = Some("synthetic-quote".into());
        app.stage_files(vec!["First account.pdf".into()]);
        let first_id = app.account().id.clone();
        let queued_caption = std::mem::take(&mut app.composer);
        let queued_mentions = app.composer_mentions.clone();
        app.switch_account(&second_id);
        app.open_chat(chat.into());
        assert!(
            !app.has_draft(chat),
            "identical chat IDs in another account must not expose a draft"
        );
        assert!(app.pending.is_empty());
        app.stage_files(vec!["Second account.pdf".into()]);
        app.send_pending(
            first_id.clone(),
            chat.into(),
            queued_caption,
            queued_mentions,
        );
        assert_eq!(
            app.pending.len(),
            1,
            "a queued send from another account cannot consume these files"
        );
        app.switch_account(&first_id);
        assert_eq!(app.composer, "Hello @Mira Example");
        assert_eq!(app.composer_mentions.len(), 1);
        assert_eq!(app.reply_to.as_deref(), Some("synthetic-quote"));
        assert!(
            matches!(app.pending.as_slice(), [Staged { item: Pending::File(path), .. }] if path == std::path::Path::new("First account.pdf"))
        );
        app.apply(Action::CloseChat, &egui::Context::default());
        assert!(app.pending.is_empty());
        assert!(app.has_draft(chat));
        app.open_chat(chat.into());
        let (backend, mut commands) = Backend::recording();
        app.backend = backend;
        let caption = std::mem::take(&mut app.composer);
        app.send_pending(
            first_id.clone(),
            chat.into(),
            caption,
            app.composer_mentions.clone(),
        );
        assert!(!app.has_draft(chat));
        let sent: Vec<_> = std::iter::from_fn(|| commands.try_recv().ok()).collect();
        assert!(sent.iter().any(|command| matches!(command, Command::SaveDraft { chat: id, text } if id == chat && text.is_empty())));
        assert!(sent.iter().any(|command| matches!(command,
            Command::SendFiles { chat: id, caption: Some(caption), mentions, quoting: Some(quote), .. }
            if id == chat && caption == "Hello @fixture-member" && mentions == &[member] && quote.id == "synthetic-quote")));
        app.switch_account(&second_id);
        assert!(
            matches!(app.pending.as_slice(), [Staged { item: Pending::File(path), .. }] if path == std::path::Path::new("Second account.pdf"))
        );
    }

    /// Attachment-only drafts remain recognizable after leaving their conversation.
    #[test]
    fn attachment_only_drafts_have_list_previews() {
        let mut app = app();
        let chat = "preview-fixture@s.whatsapp.net";
        app.chats.push(Chat::new(chat.into(), "Fixture".into()));
        app.open_chat(chat.into());
        app.stage_files(vec!["Synthetic document.pdf".into()]);
        app.apply(Action::CloseChat, &egui::Context::default());
        assert_eq!(
            app.draft_preview(chat).as_deref(),
            Some("Synthetic document.pdf")
        );
        app.attachment_drafts.get_mut(chat).unwrap().pending = vec![Staged {
            id: 1,
            item: Pending::Picture {
                width: 1,
                height: 1,
                rgba: std::sync::Arc::new(vec![0; 4]),
                texture: None,
            },
        }];
        assert_eq!(app.draft_preview(chat).as_deref(), Some("Image"));
        app.open_chat(chat.into());
        app.pending.clear();
        app.composer = "Active text stays in the composer".into();
        assert!(app.draft_preview(chat).is_none());
    }

    /// A reply without attachments is not parked, as before attachment
    /// drafts: only staged files carry their reply with them.
    #[test]
    fn a_reply_without_attachments_is_not_kept_when_switching_chats() {
        let mut app = app();
        app.open_chat("synthetic-chat".into());
        app.reply_to = Some("first-quote".into());
        app.open_chat("other-chat".into());
        assert!(app.reply_to.is_none());
        app.open_chat("synthetic-chat".into());
        assert!(app.reply_to.is_none());
        assert!(app.attachment_drafts.is_empty());
    }

    /// Late file and image refusals recover only into their originating account's draft.
    #[test]
    fn hidden_account_attachment_refusals_cannot_enter_the_visible_draft() {
        for image in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let (mut app, _) = two_accounts(directory.path());
            let chat = "fixture@s.whatsapp.net";
            for account in &mut app.accounts {
                account
                    .chats
                    .push(Chat::new(chat.into(), "Synthetic chat".into()));
                account.open_chat = Some(chat.into());
            }
            let (backend, events) = Backend::detached();
            app.accounts[1].backend = backend;
            let caption = "Recovered hidden caption";
            let unsent = if image {
                Unsent::Image {
                    width: 1,
                    height: 1,
                    rgba: vec![1, 2, 3, 4],
                    view_once: true,
                    caption: Some(caption.into()),
                }
            } else {
                Unsent::Files {
                    paths: vec!["Hidden account.pdf".into()],
                    documents: false,
                    view_once: false,
                    caption: Some(caption.into()),
                }
            };
            events
                .send(refused(
                    chat,
                    Some("hidden-quote"),
                    unsent,
                    Refusal::Offline,
                ))
                .unwrap();
            app.handle_events();
            assert!(
                app.pending.is_empty(),
                "inactive account files cannot enter the visible composer"
            );
            assert!(app.composer.is_empty());
            assert!(app.reply_to.is_none());
            assert!(
                !app.pending_view_once,
                "the hidden privacy choice stays in its account"
            );
            app.open_chat("other@s.whatsapp.net".into());
            assert!(!app.accounts[0].attachment_drafts.contains_key(chat));
            let second = app.accounts[1].id.clone();
            app.switch_account(&second);
            assert_eq!(app.composer, caption);
            assert_eq!(app.pending_view_once, image);
            assert_eq!(app.reply_to.as_deref(), Some("hidden-quote"));
            assert_eq!(app.pending.len(), 1);
            if image {
                assert!(
                    matches!(&app.pending[0].item, Pending::Picture { rgba, .. } if rgba.as_slice() == [1, 2, 3, 4])
                );
            } else {
                assert!(
                    matches!(&app.pending[0].item, Pending::File(path) if path == std::path::Path::new("Hidden account.pdf"))
                );
            }
        }
    }

    /// A send queued before switching chats preserves both drafts instead of sending the next chat's files.
    #[test]
    fn stale_attachment_sends_preserve_both_chats_and_the_original_caption() {
        let mut app = app();
        app.open_chat("first".into());
        app.stage_files(vec!["First.pdf".into()]);
        app.reply_to = Some("first-quote".into());
        app.composer = "First caption".into();
        let caption = std::mem::take(&mut app.composer);
        app.open_chat("second".into());
        app.stage_files(vec!["Second.pdf".into()]);
        app.reply_to = Some("second-quote".into());
        app.send_pending(
            app.account().id.clone(),
            "first".into(),
            caption,
            Vec::new(),
        );
        assert!(
            matches!(app.pending.as_slice(), [Staged { item: Pending::File(path), .. }] if path == std::path::Path::new("Second.pdf"))
        );
        assert_eq!(app.reply_to.as_deref(), Some("second-quote"));
        app.open_chat("first".into());
        assert_eq!(app.composer, "First caption");
        assert_eq!(app.reply_to.as_deref(), Some("first-quote"));
        assert!(
            matches!(app.pending.as_slice(), [Staged { item: Pending::File(path), .. }] if path == std::path::Path::new("First.pdf"))
        );
    }

    /// Clearing, deleting or unlinking removes the affected stored and active attachment drafts.
    #[test]
    fn clearing_deleting_and_unlinking_discard_attachment_drafts() {
        let mut app = app();
        app.open_chat("cleared".into());
        app.composer = "Caption".into();
        app.stage_files(vec!["Cleared.pdf".into()]);
        app.open_chat("other".into());
        assert!(app.has_draft("cleared"));
        app.handle_chat_cleared("cleared", 1, true);
        assert!(!app.has_draft("cleared"));
        app.open_chat("deleted".into());
        app.stage_files(vec!["Deleted.pdf".into()]);
        app.open_chat("other".into());
        assert!(app.has_draft("deleted"));
        app.forget_chat("deleted");
        assert!(!app.has_draft("deleted"));
        app.stage_files(vec!["Unlinked.pdf".into()]);
        app.open_chat("last".into());
        app.stage_files(vec!["Active.pdf".into()]);
        app.handle_link(LinkStatus::LoggedOut, true);
        assert!(app.attachment_drafts.is_empty());
        assert!(app.pending.is_empty());
    }

    /// Hiding a locked conversation retains its attachment draft without exposing it in the regular list.
    #[test]
    fn hiding_a_locked_chat_keeps_its_attachments_out_of_the_regular_list() {
        let mut app = app();
        let mut chat = Chat::new("locked".into(), "Synthetic locked chat".into());
        chat.locked = true;
        app.chats.push(chat);
        app.open_chat = Some("locked".into());
        app.stage_files(vec!["Private synthetic document.pdf".into()]);
        app.reply_to = Some("private-quote".into());
        app.hide_locked_chat("locked");
        assert!(app.open_chat.is_none());
        assert!(app.pending.is_empty());
        assert!(app.has_draft("locked"));
        assert!(app.visible_chats().iter().all(|chat| chat.id != "locked"));
        assert_eq!(
            app.attachment_drafts["locked"].quoting.as_deref(),
            Some("private-quote")
        );
    }

    /// The recipients of each forward the app asked for, skipping the
    /// forwarding-limit questions that opening the dialog sends.
    fn sent_forwards(
        commands: &mut tokio::sync::mpsc::UnboundedReceiver<Command>,
    ) -> Vec<(String, Vec<String>, Vec<String>)> {
        std::iter::from_fn(|| commands.try_recv().ok())
            .filter_map(|command| match command {
                Command::Forward {
                    from_chat,
                    messages,
                    to_chats,
                } => Some((from_chat, messages, to_chats)),
                _ => None,
            })
            .collect()
    }

    fn forward_dialog(messages: &[&str]) -> Dialog {
        Dialog::Forward {
            chat: "source".into(),
            messages: messages.iter().map(|id| (*id).to_owned()).collect(),
        }
    }

    #[test]
    /// Several recipients are confirmed first, and going back keeps them.
    fn forwarding_to_several_chats_asks_for_confirmation() {
        let mut app = app();
        let (backend, mut commands) = Backend::recording();
        app.backend = backend;
        let ctx = egui::Context::default();
        for id in ["first", "second"] {
            app.chats.push(Chat::new(id.into(), id.into()));
        }
        let dialog = forward_dialog(&["m1", "m2"]);
        app.apply(Action::ShowDialog(dialog.clone()), &ctx);
        assert!(
            matches!(commands.try_recv(), Ok(Command::ForwardLimit { from_chat, messages })
                if from_chat == "source" && messages == ["m1", "m2"]),
            "opening the dialog asks for the limit"
        );
        app.apply(Action::ConfirmForward, &ctx);
        app.apply(Action::ToggleForwardRecipient("first".into()), &ctx);
        app.forward_search = "second".into();
        app.apply(Action::ToggleForwardRecipient("second".into()), &ctx);
        app.apply(Action::ConfirmForward, &ctx);
        assert!(
            sent_forwards(&mut commands).is_empty(),
            "two recipients are not sent without their confirmation"
        );
        app.apply(Action::ReviewForward, &ctx);
        app.apply(Action::BackToForwardSelection, &ctx);
        assert_eq!(app.forward_recipients, ["first", "second"]);
        assert!(
            sent_forwards(&mut commands).is_empty(),
            "review never sends"
        );
        app.apply(Action::ReviewForward, &ctx);
        app.apply(Action::ConfirmForward, &ctx);
        assert_eq!(
            sent_forwards(&mut commands),
            [(
                "source".to_owned(),
                vec!["m1".to_owned(), "m2".to_owned()],
                vec!["first".to_owned(), "second".to_owned()]
            )],
            "one batch for every recipient, in the order they were picked"
        );
        app.apply(Action::ConfirmForward, &ctx);
        assert!(
            sent_forwards(&mut commands).is_empty(),
            "double confirmation never duplicates"
        );
        assert!(app.dialog.is_none());
        app.apply(Action::ShowDialog(dialog), &ctx);
        app.apply(Action::ToggleForwardRecipient("first".into()), &ctx);
        app.apply(Action::CloseDialog, &ctx);
        assert!(app.forward_recipients.is_empty());
        assert!(!app.forward_reviewing);
        assert!(
            sent_forwards(&mut commands).is_empty(),
            "cancel never sends"
        );
    }

    #[test]
    fn forwarding_to_one_chat_needs_no_confirmation() {
        let mut app = app();
        let (backend, mut commands) = Backend::recording();
        app.backend = backend;
        let ctx = egui::Context::default();
        app.chats.push(Chat::new("first".into(), "First".into()));
        app.apply(Action::ShowDialog(forward_dialog(&["m"])), &ctx);
        app.apply(Action::ToggleForwardRecipient("first".into()), &ctx);
        app.apply(Action::ReviewForward, &ctx);
        assert!(
            !app.forward_reviewing,
            "one recipient has nothing to review"
        );
        app.apply(Action::ConfirmForward, &ctx);
        assert_eq!(
            sent_forwards(&mut commands),
            [(
                "source".to_owned(),
                vec!["m".to_owned()],
                vec!["first".to_owned()]
            )]
        );
        assert!(app.dialog.is_none());
    }

    #[test]
    fn forwarding_takes_at_most_five_chats() {
        let mut app = app();
        let (backend, mut commands) = Backend::recording();
        app.backend = backend;
        let ctx = egui::Context::default();
        let ids: Vec<String> = (0..7).map(|index| format!("chat-{index}")).collect();
        for id in &ids {
            app.chats.push(Chat::new(id.clone(), id.clone()));
        }
        app.apply(Action::ShowDialog(forward_dialog(&["m"])), &ctx);
        for id in &ids {
            app.apply(Action::ToggleForwardRecipient(id.clone()), &ctx);
        }
        assert_eq!(app.forward_recipients, ids[..crate::model::FORWARD_LIMIT]);
        // Dropping one makes room for another.
        app.apply(Action::ToggleForwardRecipient(ids[0].clone()), &ctx);
        app.apply(Action::ToggleForwardRecipient(ids[6].clone()), &ctx);
        assert_eq!(app.forward_recipients.len(), crate::model::FORWARD_LIMIT);
        assert!(app.forward_recipients.contains(&ids[6]));
        app.apply(Action::ReviewForward, &ctx);
        app.apply(Action::ConfirmForward, &ctx);
        let sent = sent_forwards(&mut commands);
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].2.len(), crate::model::FORWARD_LIMIT);
    }

    #[test]
    fn a_message_forwarded_many_times_goes_to_one_chat() {
        let mut app = app();
        let (backend, mut commands) = Backend::recording();
        app.backend = backend;
        let ctx = egui::Context::default();
        for id in ["first", "second", "third"] {
            app.chats.push(Chat::new(id.into(), id.into()));
        }
        app.apply(Action::ShowDialog(forward_dialog(&["m"])), &ctx);
        // Picked before the worker answered.
        app.apply(Action::ToggleForwardRecipient("first".into()), &ctx);
        app.apply(Action::ToggleForwardRecipient("second".into()), &ctx);
        app.apply(Action::ReviewForward, &ctx);
        assert!(app.forward_reviewing);
        // An answer for other messages changes nothing.
        app.apply_backend_event(
            Event::ForwardLimit {
                from_chat: "source".into(),
                messages: vec!["other".into()],
                limit: 1,
            },
            true,
        );
        assert_eq!(app.forward_limit, crate::model::FORWARD_LIMIT);
        app.apply_backend_event(
            Event::ForwardLimit {
                from_chat: "source".into(),
                messages: vec!["m".into()],
                limit: 1,
            },
            true,
        );
        assert_eq!(app.forward_limit, 1);
        assert_eq!(app.forward_recipients, ["first"], "the first pick stays");
        assert!(!app.forward_reviewing);
        // Another pick replaces it.
        app.apply(Action::ToggleForwardRecipient("third".into()), &ctx);
        assert_eq!(app.forward_recipients, ["third"]);
        app.apply(Action::ConfirmForward, &ctx);
        let sent = sent_forwards(&mut commands);
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].2, ["third"]);
    }

    #[test]
    /// Checks confirmation rejects destinations that became locked or left after selection.
    fn forwarding_rechecks_destinations_before_sending() {
        for left in [false, true] {
            let mut app = app();
            app.locale = crate::i18n::Locale::PortugueseBrazil;
            let (backend, mut commands) = Backend::recording();
            app.backend = backend;
            let ctx = egui::Context::default();
            app.chats.push(Chat::new("target".into(), "Target".into()));
            app.chats.push(Chat::new("valid".into(), "Valid".into()));
            app.apply(Action::ShowDialog(forward_dialog(&["m"])), &ctx);
            app.apply(Action::ToggleForwardRecipient("target".into()), &ctx);
            app.apply(Action::ToggleForwardRecipient("valid".into()), &ctx);
            app.apply(Action::ReviewForward, &ctx);
            let target = app
                .chats
                .iter_mut()
                .find(|chat| chat.id == "target")
                .unwrap();
            target.left = left;
            target.locked = !left;
            app.apply(Action::ConfirmForward, &ctx);
            assert!(sent_forwards(&mut commands).is_empty());
            assert!(app.dialog.is_some());
            assert!(!app.forward_reviewing);
            assert_eq!(app.forward_recipients, ["valid"]);
            // The one recipient left goes without another confirmation.
            app.apply(Action::ConfirmForward, &ctx);
            let sent = sent_forwards(&mut commands);
            assert_eq!(sent.len(), 1);
            assert_eq!(sent[0].2, ["valid"]);
            assert!(app.toasts.iter().any(|toast| toast.message
                == "Não é mais possível enviar mensagens para uma das conversas selecionadas"));
        }
    }

    #[test]
    /// Checks a chat the user has left cannot be selected as a forwarding destination.
    fn forwarding_cannot_select_a_left_chat() {
        let mut app = app();
        let ctx = egui::Context::default();
        let mut target = Chat::new("left".into(), "Left".into());
        target.left = true;
        app.chats.push(target);
        app.apply(
            Action::ShowDialog(Dialog::Forward {
                chat: "source".into(),
                messages: vec!["m".into()],
            }),
            &ctx,
        );
        app.apply(Action::ToggleForwardRecipient("left".into()), &ctx);
        assert!(app.forward_recipients.is_empty());
    }

    /// Unrelated dialogs do not cancel a shared-contact lookup.
    #[test]
    fn shared_contact_lookup_survives_opening_and_closing_unrelated_dialogs() {
        for success in [false, true] {
            let mut app = app();
            app.locale = crate::i18n::Locale::PortugueseBrazil;
            let (backend, _commands, events) = Backend::recording_with_events();
            app.backend = backend;
            let ctx = egui::Context::default();
            app.apply(
                Action::NewContact {
                    phone: "15550000001".into(),
                    first: String::new(),
                    last: String::new(),
                    to_phone: None,
                },
                &ctx,
            );
            let request = app.new_contact_request.unwrap();
            app.apply(Action::ShowDialog(Dialog::Shortcuts), &ctx);
            app.apply(Action::CloseDialog, &ctx);
            app.apply(Action::ShowDialog(Dialog::NewContact), &ctx);
            assert_eq!(app.new_contact_request, Some(request));
            events
                .send(if success {
                    Event::ContactReady {
                        request,
                        id: "15550000001@s.whatsapp.net".into(),
                        name: None,
                    }
                } else {
                    Event::ContactFailed {
                        request,
                        error: "Could not check the number. Please try again.".into(),
                    }
                })
                .unwrap();
            app.handle_events();
            app.apply_actions(&ctx);
            assert_eq!(
                app.dialog,
                Some(Dialog::NewContact),
                "independent completion does not dismiss the new dialog"
            );
            assert!(app.new_contact_request.is_none());
            if success {
                assert_eq!(app.open_chat.as_deref(), Some("15550000001@s.whatsapp.net"));
            } else {
                assert!(app.toasts.iter().any(|toast| toast.message
                    == "Não foi possível verificar o número. Tente novamente."));
            }
        }
    }

    /// Verifies that a later sticker dialog survives dispatching a completed number lookup.
    #[test]
    fn completed_number_lookup_preserves_a_later_sticker_dialog() {
        let mut app = app();
        let (backend, _commands, events) = Backend::recording_with_events();
        app.backend = backend;
        let ctx = egui::Context::default();
        app.apply(Action::ShowDialog(Dialog::NewContact), &ctx);
        app.apply(
            Action::NewContact {
                phone: "15550000001".into(),
                first: String::new(),
                last: String::new(),
                to_phone: None,
            },
            &ctx,
        );
        let request = app.new_contact_request.unwrap();
        events
            .send(Event::ContactReady {
                request,
                id: "15550000001@s.whatsapp.net".into(),
                name: None,
            })
            .unwrap();
        events
            .send(Event::StickerPicture {
                path: PathBuf::from("synthetic-sticker.png"),
                width: 32,
                height: 32,
                transparent: false,
            })
            .unwrap();
        app.handle_events();
        assert_eq!(app.dialog, Some(Dialog::StickerMaker));
        app.apply_actions(&ctx);
        assert_eq!(app.dialog, Some(Dialog::StickerMaker));
        assert_eq!(app.open_chat.as_deref(), Some("15550000001@s.whatsapp.net"));
    }

    /// Settings cancel number-entry lookups while shared-contact requests remain active.
    #[test]
    fn opening_settings_invalidates_dialog_lookups_but_keeps_shared_contacts() {
        for dialog in [Some(Dialog::NewContact), Some(Dialog::NewContact), None] {
            let mut app = app();
            let (backend, _commands, events) = Backend::recording_with_events();
            app.backend = backend;
            let ctx = egui::Context::default();
            app.dialog = dialog.clone();
            app.apply(
                Action::NewContact {
                    phone: "15550000001".into(),
                    first: String::new(),
                    last: String::new(),
                    to_phone: None,
                },
                &ctx,
            );
            let request = app.new_contact_request.unwrap();
            app.apply(Action::ToggleSettings, &ctx);
            events
                .send(Event::ContactReady {
                    request,
                    id: "15550000001@s.whatsapp.net".into(),
                    name: None,
                })
                .unwrap();
            app.handle_events();
            assert_eq!(app.page, Page::Settings);
            let opened = app
                .deferred_account_actions
                .iter()
                .any(|(account, action)| {
                    account == &app.account().id && matches!(action, Action::StartChat { .. })
                });
            assert_eq!(
                opened,
                dialog.is_none(),
                "shared-contact requests are independent of dialogs"
            );
        }
    }

    /// Invalid international numbers cannot reach the backend lookup.
    #[test]
    fn invalid_number_actions_never_start_a_lookup() {
        let mut app = app();
        let (backend, mut commands) = Backend::recording();
        app.backend = backend;
        app.apply(
            Action::NewContact {
                phone: "+1 555 1234567 ext. 89".into(),
                first: String::new(),
                last: String::new(),
                to_phone: None,
            },
            &egui::Context::default(),
        );
        assert!(commands.try_recv().is_err());
        assert!(!app.new_contact_pending);
    }

    /// Cancelled lookup completions cannot disturb a newer dialog.
    #[test]
    fn cancelled_number_lookups_cannot_open_or_close_a_new_dialog() {
        let mut app = app();
        let (backend, mut commands, events) = Backend::recording_with_events();
        app.backend = backend;
        let ctx = egui::Context::default();
        let lookup = |phone: &str| Action::NewContact {
            phone: phone.into(),
            first: String::new(),
            last: String::new(),
            to_phone: None,
        };
        app.apply(Action::ShowDialog(Dialog::NewContact), &ctx);
        app.apply(lookup("15550000001"), &ctx);
        let first = app.new_contact_request.unwrap();
        app.apply(Action::CloseDialog, &ctx);
        app.apply(Action::ShowDialog(Dialog::NewContact), &ctx);
        app.apply(lookup("15550000002"), &ctx);
        let second = app.new_contact_request.unwrap();
        events
            .send(Event::ContactReady {
                request: first,
                id: "15550000001@s.whatsapp.net".into(),
                name: None,
            })
            .unwrap();
        events
            .send(Event::ContactFailed {
                request: first,
                error: "Stale failure".into(),
            })
            .unwrap();
        app.handle_events();
        assert_eq!(app.dialog, Some(Dialog::NewContact));
        assert_eq!(app.new_contact_request, Some(second));
        assert!(app.new_contact_pending);
        assert!(
            !app.actions
                .iter()
                .any(|action| matches!(action, Action::StartChat { .. }))
        );
        assert!(app.toasts.is_empty());
        events
            .send(Event::ContactFailed {
                request: second,
                error: "Try again".into(),
            })
            .unwrap();
        app.handle_events();
        assert_eq!(app.dialog, Some(Dialog::NewContact));
        assert!(!app.new_contact_pending);
        assert!(app.new_contact_request.is_none());
        app.apply(lookup("15550000002"), &ctx);
        let retry = app.new_contact_request.unwrap();
        assert_ne!(retry, second);
        events
            .send(Event::ContactReady {
                request: retry,
                id: "15550000002@s.whatsapp.net".into(),
                name: None,
            })
            .unwrap();
        app.handle_events();
        assert!(app.dialog.is_none());
        assert!(app.deferred_account_actions.iter().any(|(account, action)| account == &app.account().id && matches!(action, Action::StartChat { id, .. } if id == "15550000002@s.whatsapp.net")));
        assert!(commands.try_recv().is_ok());
    }

    /// A hidden account's late lookup result cannot alter the visible account's dialog.
    #[test]
    fn hidden_account_number_lookups_leave_the_visible_request_alone() {
        for failed in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let dirs = AppDirs::under(directory.path());
            let mut app = App::headless(dirs.clone(), Settings::default()).0;
            let (backend, _commands, events) = Backend::recording_with_events();
            app.backend = backend;
            let second_id = AccountId::parse("2").unwrap();
            let (second, _) = Account::detached(
                &dirs,
                second_id.clone(),
                crate::settings::AccountSettings::default(),
            )
            .unwrap();
            app.accounts.push(second);
            let ctx = egui::Context::default();
            let lookup = || Action::NewContact {
                phone: "15550000001".into(),
                first: String::new(),
                last: String::new(),
                to_phone: None,
            };
            app.apply(Action::ShowDialog(Dialog::NewContact), &ctx);
            app.apply(lookup(), &ctx);
            let first_request = app.new_contact_request.unwrap();
            app.switch_account(&second_id);
            app.apply(Action::ShowDialog(Dialog::NewContact), &ctx);
            app.apply(lookup(), &ctx);
            let visible_request = app.new_contact_request.unwrap();
            events
                .send(if failed {
                    Event::ContactFailed {
                        request: first_request,
                        error: "Synthetic lookup failure".into(),
                    }
                } else {
                    Event::ContactReady {
                        request: first_request,
                        id: "15550000001@s.whatsapp.net".into(),
                        name: None,
                    }
                })
                .unwrap();
            app.handle_events();
            assert_eq!(app.account().id, second_id);
            assert_eq!(app.dialog, Some(Dialog::NewContact));
            assert_eq!(app.new_contact_request, Some(visible_request));
            assert!(app.new_contact_pending);
            assert!(app.toasts.is_empty());
            assert!(
                !app.actions
                    .iter()
                    .any(|action| matches!(action, Action::StartChat { .. }))
            );
            assert!(!app.accounts[0].new_contact_pending);
            assert!(
                !app.deferred_account_actions
                    .iter()
                    .any(|(_, action)| matches!(action, Action::StartChat { .. }))
            );
            assert!(app.accounts[0].new_contact_request.is_none());
        }
    }

    /// Logout invalidates pending lookups and reconnect permits a fresh request.
    #[test]
    fn logout_ends_number_lookups_and_allows_retry_after_reconnect() {
        let mut app = app();
        let (backend, _commands, events) = Backend::recording_with_events();
        app.backend = backend;
        let ctx = egui::Context::default();
        let lookup = || Action::NewContact {
            phone: "15550000001".into(),
            first: String::new(),
            last: String::new(),
            to_phone: None,
        };
        app.apply(Action::ShowDialog(Dialog::NewContact), &ctx);
        app.apply(lookup(), &ctx);
        let old_request = app.new_contact_request.unwrap();
        app.handle_link(LinkStatus::LoggedOut, true);
        assert!(!app.new_contact_pending);
        assert!(app.new_contact_request.is_none());
        assert!(app.new_contact_dialog_request.is_none());
        app.handle_link(LinkStatus::Connected, true);
        app.apply(lookup(), &ctx);
        let retry = app.new_contact_request.unwrap();
        assert_ne!(retry, old_request);
        events
            .send(Event::ContactReady {
                request: old_request,
                id: "15550000001@s.whatsapp.net".into(),
                name: None,
            })
            .unwrap();
        app.handle_events();
        assert_eq!(app.new_contact_request, Some(retry));
        assert!(app.new_contact_pending);
        assert_eq!(app.dialog, Some(Dialog::NewContact));
        assert!(
            !app.actions
                .iter()
                .any(|action| matches!(action, Action::StartChat { .. }))
        );
        events
            .send(Event::ContactFailed {
                request: retry,
                error: "Fixture retry failure".into(),
            })
            .unwrap();
        app.handle_events();
        assert!(!app.new_contact_pending);
    }

    /// Private-reply refusals preserve the owning account's quote and text.
    #[test]
    fn hidden_account_private_reply_recovery_keeps_the_visible_composer_unchanged() {
        let directory = tempfile::tempdir().unwrap();
        let (mut app, _) = two_accounts(directory.path());
        let chat = "recipient@s.whatsapp.net";
        for account in &mut app.accounts {
            account
                .chats
                .push(Chat::new(chat.into(), "Synthetic recipient".into()));
            account.open_chat = Some(chat.into());
        }
        let (backend, events) = Backend::detached();
        app.accounts[1].backend = backend;
        let quote = crate::model::ReplyTarget {
            id: "source-message".into(),
            chat: Some("source@g.us".into()),
        };
        events
            .send(Event::SendRefused {
                chat: chat.into(),
                quoting: Some(quote.clone()),
                unsent: Unsent::Text("Hidden correction".into()),
                reason: Refusal::QuoteUnavailable,
            })
            .unwrap();
        app.handle_events();
        assert!(app.composer.is_empty());
        assert!(app.reply_to.is_none());
        let second = app.accounts[1].id.clone();
        app.switch_account(&second);
        assert_eq!(app.composer, "Hidden correction");
        assert_eq!(app.reply_to, Some(quote));
    }

    #[test]
    fn a_second_account_is_remembered_with_the_one_on_screen() {
        let directory = tempfile::tempdir().unwrap();
        let mut app = App::headless(AppDirs::under(directory.path()), Settings::default()).0;
        assert!(!app.has_several_accounts());
        let ctx = egui::Context::default();
        app.apply(Action::AddAccount, &ctx);
        assert!(app.has_several_accounts());
        assert_eq!(app.account().id.as_str(), "2");
        assert!(app.adding_account);
        app.link = LinkStatus::Connected;
        let first = AccountId::first();
        app.apply(Action::SwitchAccount(first.clone()), &ctx);
        assert_eq!(app.account().id, first);
        assert!(app.files.flush(Duration::from_secs(10)));
        let roster = AccountRoster::load(&app.dirs.accounts_file()).unwrap();
        assert_eq!(roster.order, ["1".to_string(), "2".to_string()]);
        assert_eq!(roster.active, "1");
    }

    /// Two accounts, the second with a recording backend; the first on screen.
    fn two_accounts(
        directory: &std::path::Path,
    ) -> (App, tokio::sync::mpsc::UnboundedReceiver<Command>) {
        let dirs = AppDirs::under(directory);
        let mut app = App::headless(dirs.clone(), Settings::default()).0;
        let mut second = Account::detached(
            &dirs,
            AccountId::parse("2").unwrap(),
            crate::settings::AccountSettings::default(),
        )
        .unwrap()
        .0;
        let (backend, commands) = Backend::recording();
        second.backend = backend;
        second.link = LinkStatus::Connected;
        app.accounts.push(second);
        app.active = 0;
        (app, commands)
    }

    /// Background history is each account's own switch. Only the account on
    /// screen names a chat to ask first, and the worker hears only changes.
    #[test]
    fn background_history_follows_each_accounts_switch_and_the_screen() {
        let directory = tempfile::tempdir().unwrap();
        let (mut app, mut commands) = two_accounts(directory.path());
        let mut sent = || -> Vec<(bool, Option<ChatId>)> {
            std::iter::from_fn(|| commands.try_recv().ok())
                .filter_map(|command| match command {
                    Command::SetHistoryPrefetch { on, focused } => Some((on, focused)),
                    _ => None,
                })
                .collect()
        };
        app.accounts[1].open_chat = Some("hidden@s.whatsapp.net".into());
        app.sync_prefetch();
        assert_eq!(sent(), [(false, None)], "off by default, and hidden");
        app.sync_prefetch();
        assert!(sent().is_empty(), "nothing changed, nothing sent");

        app.accounts[1].settings.history_prefetch = true;
        app.sync_prefetch();
        assert_eq!(sent(), [(true, None)]);

        app.active = 1;
        app.sync_prefetch();
        assert_eq!(
            sent(),
            [(true, Some("hidden@s.whatsapp.net".into()))],
            "on screen, its open chat goes first"
        );
        assert!(
            !app.accounts[0].settings.history_prefetch,
            "the other account keeps its own switch"
        );
    }

    /// A pending request for the message list's focus ends with its account.
    #[test]
    fn switching_accounts_drops_a_message_list_focus_request() {
        let directory = tempfile::tempdir().unwrap();
        let (mut app, _commands) = two_accounts(directory.path());
        let ctx = egui::Context::default();
        app.focus_messages = true;
        app.apply(Action::SwitchAccount(AccountId::parse("2").unwrap()), &ctx);
        assert!(!app.focus_messages);
    }

    /// A reaction picker the message list opened is not handed back to the
    /// list of another account's chat.
    #[test]
    fn switching_accounts_forgets_the_lists_reaction_picker() {
        let directory = tempfile::tempdir().unwrap();
        let (mut app, _commands) = two_accounts(directory.path());
        let ctx = egui::Context::default();
        app.reaction_from_list = true;
        app.apply(Action::SwitchAccount(AccountId::parse("2").unwrap()), &ctx);
        assert!(!app.reaction_from_list);
    }

    /// A reaction picker the list opened and something else closed (the
    /// emoji tab, the composer tools, the chat cleared on the phone) has no
    /// claim on focus: a later reaction from a chip leaves focus alone.
    #[test]
    fn a_reaction_after_the_lists_picker_closed_elsewhere_leaves_focus_alone() {
        let chat = "peer@s.whatsapp.net";
        for close in ["emoji tab", "composer tools", "chat cleared"] {
            let directory = tempfile::tempdir().unwrap();
            let (mut app, _events) =
                App::headless(AppDirs::under(directory.path()), Settings::default());
            app.chats.push(Chat::new(chat.into(), "Peer".into()));
            app.conversations.entry(chat.into()).or_default().merge(
                vec![message(chat, "m1", 100), message(chat, "m2", 200)],
                false,
            );
            app.open_chat = Some(chat.into());
            let ctx = egui::Context::default();
            app.apply(
                crate::ui::conversation::open_reaction_picker_action(chat, "m1", true),
                &ctx,
            );
            assert!(app.reaction_target.is_some() && app.reaction_from_list);
            match close {
                "emoji tab" => app.apply(Action::TogglePicker(PickerTab::Emoji), &ctx),
                "composer tools" => app.apply(Action::SetComposerTools(true), &ctx),
                _ => app.handle_chat_cleared(chat, 100, true),
            }
            assert_eq!(app.reaction_target, None, "{close} closes the picker");
            app.focus_messages = false;
            app.apply(
                Action::React {
                    chat: chat.into(),
                    message: "m2".into(),
                    emoji: "👍".into(),
                },
                &ctx,
            );
            assert!(
                !app.focus_messages,
                "after the {close}, a chip's reaction does not move focus to the list"
            );
            assert!(!app.reaction_from_list, "{close}: the flag is gone");
        }
    }

    /// A message menu asked for and not drawn yet is not opened on another
    /// account's chat.
    #[test]
    fn switching_accounts_drops_a_pending_message_menu() {
        let directory = tempfile::tempdir().unwrap();
        let (mut app, _commands) = two_accounts(directory.path());
        let ctx = egui::Context::default();
        app.open_message_menu = Some(("15550007777@s.whatsapp.net".into(), "m".into()));
        app.apply(Action::SwitchAccount(AccountId::parse("2").unwrap()), &ctx);
        assert!(app.open_message_menu.is_none());
    }

    /// A replied message still loading ends with its account, as with its
    /// chat.
    #[test]
    fn switching_accounts_ends_the_wait_for_a_replied_message() {
        let directory = tempfile::tempdir().unwrap();
        let (mut app, _commands) = two_accounts(directory.path());
        let ctx = egui::Context::default();
        app.pending_follow = Some(PendingFollow {
            chat: "15550007777@s.whatsapp.net".into(),
            message: "quoted".into(),
            focus: None,
            frame: 0,
            asked: true,
            answered: false,
        });
        app.apply(Action::SwitchAccount(AccountId::parse("2").unwrap()), &ctx);
        assert!(app.pending_follow.is_none());
    }

    /// An empty answer to G's request is said on the next frame, so it asks
    /// for that frame: nothing else may come to draw one before the next key,
    /// which would end the wait unsaid.
    #[test]
    fn an_empty_answer_for_a_replied_message_asks_for_the_frame_that_says_it() {
        let directory = tempfile::tempdir().unwrap();
        let (mut app, events) =
            App::headless(AppDirs::under(directory.path()), Settings::default());
        let chat = "15550007777@s.whatsapp.net";
        app.pending_follow = Some(PendingFollow {
            chat: chat.into(),
            message: "quoted".into(),
            focus: None,
            frame: 0,
            asked: true,
            answered: false,
        });
        let ctx = egui::Context::default();
        app.waker.attach(&ctx);
        // Quiet passes use up egui's own follow-up frames.
        for _ in 0..10 {
            if !ctx.has_requested_repaint() {
                break;
            }
            ctx.run_ui(egui::RawInput::default(), |_| {})
                .textures_delta
                .clear();
        }
        assert!(!ctx.has_requested_repaint(), "the window is idle");
        events
            .send(Event::Messages {
                chat: chat.into(),
                messages: Vec::new(),
                older: true,
                complete: false,
            })
            .unwrap();
        app.handle_events();
        assert!(
            app.pending_follow
                .as_ref()
                .is_some_and(|pending| pending.answered)
        );
        assert!(ctx.has_requested_repaint(), "the next frame says it");
    }

    #[test]
    fn leaving_an_account_before_it_is_linked_removes_it() {
        let directory = tempfile::tempdir().unwrap();
        let mut app = App::headless(AppDirs::under(directory.path()), Settings::default()).0;
        let ctx = egui::Context::default();
        app.apply(Action::AddAccount, &ctx);
        let (backend, mut commands) = Backend::recording();
        app.backend = backend;
        app.link = LinkStatus::Unlinked {
            qr: None,
            pair_code: None,
            pairing_phone: None,
        };
        app.apply(Action::CancelAddAccount, &ctx);
        assert_eq!(app.account().id, AccountId::first());
        assert!(!app.adding_account);
        assert!(
            std::iter::from_fn(|| commands.try_recv().ok())
                .any(|command| matches!(command, Command::RemoveAccount)),
            "the unlinked account is removed"
        );
    }

    #[test]
    fn a_hidden_account_never_marks_its_remembered_chat_read() {
        let directory = tempfile::tempdir().unwrap();
        let (mut app, mut commands) = two_accounts(directory.path());
        app.window_focused = true;
        app.window_hidden = false;
        let chat = "15550003333@s.whatsapp.net";
        app.accounts[1].open_chat = Some(chat.into());
        app.accounts[1].settings.notifications = true;
        let mut update = Chat::new(chat.into(), "Grace".into());
        update.unread = 1;
        app.active = 1;
        app.events_hidden = true;
        app.handle_chat_updated(update, false);
        app.events_hidden = false;
        app.active = 0;
        let kept = app.accounts[1].chats.iter().find(|known| known.id == chat);
        assert_eq!(kept.unwrap().unread, 1);
        assert!(
            !std::iter::from_fn(|| commands.try_recv().ok())
                .any(|command| matches!(command, Command::MarkRead { .. })),
            "no read receipt for a chat nobody is looking at"
        );
    }

    /// A hidden account's toasts are spoken with its name first, since they
    /// show over the chats of the account on screen: a message of its own,
    /// and its way back online after a drop that was never announced.
    #[test]
    fn a_hidden_accounts_toasts_name_the_account() {
        let directory = tempfile::tempdir().unwrap();
        let (mut app, _commands) = two_accounts(directory.path());
        app.accounts[1].me_name = Some("Studio".into());
        app.accounts[1].link = LinkStatus::Disconnected {
            reason: "fixture".into(),
        };
        app.active = 1;
        app.events_hidden = true;
        app.apply_backend_event(Event::Info("Contact saved".into()), false);
        app.apply_backend_event(Event::Link(LinkStatus::Connected), false);
        app.events_hidden = false;
        app.active = 0;
        app.announcer.show(&egui::Context::default(), true);
        let spoken = app.announcer.spoken().join(" ");
        assert!(spoken.contains("Studio: Contact saved"), "{spoken}");
        assert!(spoken.contains("Studio: Back online"), "{spoken}");
        assert!(!spoken.contains("Connection lost"), "{spoken}");
        // The toasts show as upstream shows them.
        assert!(
            app.toasts
                .iter()
                .any(|toast| toast.message == "Contact saved")
        );
    }

    /// eframe draws no frame for a minimized, covered or hidden window, so
    /// nothing is spoken then. On return, what happened meanwhile is not read
    /// out in one burst: only what is still on screen, an unclosed error.
    #[test]
    fn announcements_go_stale_with_their_toasts_while_nothing_is_drawn() {
        use fastframe_shell::Resident;
        let root = tempfile::tempdir().unwrap();
        let (mut app, _events) = App::headless(AppDirs::under(root.path()), Settings::default());
        let ctx = egui::Context::default();
        app.window_hidden = true;
        app.toast("Copied");
        app.toast_error("Could not send");
        app.headless_frame(&ctx);
        app.announcer.age(INFO_TOAST_LIFETIME);
        app.window_hidden = false;
        app.announcer.show(&ctx, true);
        assert_eq!(app.announcer.spoken()[0], "Could not send");
        assert!(
            app.announcer
                .spoken()
                .iter()
                .all(|text| !text.contains("Copied")),
            "{:?}",
            app.announcer.spoken()
        );
    }

    /// The lock screen draws no toasts, so a toast, which can name a hidden
    /// account or a contact, is not spoken behind it either.
    #[test]
    fn a_toast_is_not_spoken_while_the_app_is_locked() {
        let directory = tempfile::tempdir().unwrap();
        let (mut app, _commands) = two_accounts(directory.path());
        app.accounts[1].me_name = Some("Studio".into());
        app.active = 1;
        app.events_hidden = true;
        app.apply_backend_event(Event::Info("Contact saved".into()), false);
        app.events_hidden = false;
        app.active = 0;
        // What ui::announce passes on a locked frame.
        app.announcer.show(&egui::Context::default(), false);
        let spoken = app.announcer.spoken().join(" ");
        assert!(!spoken.contains("Studio"), "{spoken}");
        assert!(!spoken.contains("Contact saved"), "{spoken}");
    }

    /// Another account on screen has its own link: the previous account's
    /// offline episode is over for the announcements.
    #[test]
    fn switching_accounts_ends_the_announced_offline_episode() {
        let directory = tempfile::tempdir().unwrap();
        let (mut app, _commands) = two_accounts(directory.path());
        let ctx = egui::Context::default();
        app.announced_offline = true;
        app.announced_connecting = true;
        app.apply(Action::SwitchAccount(AccountId::parse("2").unwrap()), &ctx);
        assert_eq!(app.account().id.as_str(), "2");
        assert!(!app.announced_offline);
        assert!(!app.announced_connecting);
    }

    /// The current message of a chat is the account on screen's: another
    /// account with the same chat starts from its own newest message.
    #[test]
    fn switching_accounts_forgets_the_current_messages() {
        let directory = tempfile::tempdir().unwrap();
        let (mut app, _commands) = two_accounts(directory.path());
        let ctx = egui::Context::default();
        app.message_cursor
            .insert("1@s.whatsapp.net".into(), "m1".into());
        app.cursor_followed = Some(("1@s.whatsapp.net".into(), "m1".into()));
        app.apply(Action::SwitchAccount(AccountId::parse("2").unwrap()), &ctx);
        assert_eq!(app.account().id.as_str(), "2");
        assert!(app.message_cursor.is_empty());
        assert_eq!(app.cursor_followed, None);
    }

    /// A message spoken for one account is not left readable in the tree
    /// once another account is on screen.
    #[test]
    fn switching_accounts_empties_the_spoken_messages() {
        let directory = tempfile::tempdir().unwrap();
        let (mut app, _commands) = two_accounts(directory.path());
        let ctx = egui::Context::default();
        app.announcer
            .say(crate::announcer::Category::Message, "Ada: Hello");
        app.announcer.show(&ctx, true);
        assert!(app.announcer.spoken().contains(&"Ada: Hello"));
        app.apply(Action::SwitchAccount(AccountId::parse("2").unwrap()), &ctx);
        app.announcer.show(&ctx, true);
        assert!(
            !app.announcer
                .spoken()
                .iter()
                .any(|slot| slot.contains("Hello"))
        );
    }

    #[test]
    fn the_taskbar_counts_every_account_and_the_switcher_the_others() {
        let directory = tempfile::tempdir().unwrap();
        let (mut app, _commands) = two_accounts(directory.path());
        let mut mine = Chat::new("15550004444@s.whatsapp.net".into(), "Ada".into());
        mine.unread = 1;
        app.accounts[0].chats.push(mine);
        for id in ["15550005555@s.whatsapp.net", "15550006666@s.whatsapp.net"] {
            let mut theirs = Chat::new(id.into(), "Grace".into());
            theirs.unread = 3;
            app.accounts[1].chats.push(theirs);
        }
        assert_eq!(app.unread_chat_count(), 1);
        assert_eq!(app.unread_chat_count_everywhere(), 3);
        assert_eq!(app.unread_chat_count_elsewhere(), 2);

        // A muted account counts toward neither, as a muted chat does not.
        app.accounts[1].settings.notifications = false;
        assert_eq!(app.unread_chat_count_everywhere(), 1);
        assert_eq!(app.unread_chat_count_elsewhere(), 0);
        app.accounts[0].settings.notifications = false;
        assert_eq!(app.unread_chat_count_everywhere(), 0);
    }

    /// The switcher's bell mutes one account without showing it: that
    /// account stops notifying, stays linked, and the others keep notifying.
    #[test]
    fn the_switcher_mutes_an_account_that_is_not_on_screen() {
        let directory = tempfile::tempdir().unwrap();
        let (mut app, _commands) = two_accounts(directory.path());
        let ctx = egui::Context::default();
        let chat = "15550008888@s.whatsapp.net";
        let mut theirs = Chat::new(chat.into(), "Grace".into());
        theirs.unread = 1;
        app.accounts[1].chats.push(theirs);
        let message = |id: &str| Message {
            id: id.into(),
            chat: chat.into(),
            sender: chat.into(),
            sender_name: None,
            from_me: false,
            timestamp: crate::util::now(),
            content: crate::model::Content::text("are you there?"),
            status: crate::model::Delivery::None,
            delivered_at: None,
            read_at: None,
            quoted: None,
            reactions: Vec::new(),
            history_order: None,
            edited: false,
            mentions: Vec::new(),
            forwarded: false,
            thumbnail: None,
        };
        let arrives = |app: &mut App, id: &str| {
            app.active = 1;
            app.events_hidden = true;
            app.maybe_notify(chat, &message(id));
            app.events_hidden = false;
            app.active = 0;
        };
        arrives(&mut app, "m1");
        assert_eq!(app.notifications.shown.len(), 1);

        let second = AccountId::parse("2").unwrap();
        app.apply(Action::SetAccountNotifications(second.clone(), false), &ctx);
        assert_eq!(app.active, 0, "muting does not switch accounts");
        assert!(!app.accounts[1].settings.notifications);
        assert!(app.accounts[1].settings_dirty, "the choice is saved");
        assert!(app.accounts[0].settings.notifications);
        assert_eq!(app.accounts[1].link, LinkStatus::Connected);
        arrives(&mut app, "m2");
        assert_eq!(app.notifications.shown.len(), 1, "a muted account is quiet");

        app.apply(Action::SetAccountNotifications(second, true), &ctx);
        assert!(app.accounts[1].settings.notifications);
        arrives(&mut app, "m3");
        assert_eq!(app.notifications.shown.len(), 2);
    }

    #[test]
    fn global_settings_reach_every_account() {
        let directory = tempfile::tempdir().unwrap();
        let (mut app, mut commands) = two_accounts(directory.path());
        let ctx = egui::Context::default();
        let folder = directory.path().join("Downloads");
        app.apply(Action::SetDownloadFolder(Some(folder.clone())), &ctx);
        assert!(
            std::iter::from_fn(|| commands.try_recv().ok()).any(|command| matches!(
                command,
                Command::SetDownloadFolder(Some(ref chosen)) if *chosen == folder
            )),
            "the hidden account downloads there too"
        );
        app.apply(Action::SetKeepChatsArchived(false), &ctx);
        assert!(
            std::iter::from_fn(|| commands.try_recv().ok())
                .any(|command| matches!(command, Command::SetKeepChatsArchived(false))),
            "the hidden account unarchives on new messages too"
        );
    }

    /// `zapfast quit` takes the tray's Quit path, and does even behind the
    /// app lock, since quitting reveals nothing.
    #[test]
    fn a_quit_request_quits_even_behind_the_lock() {
        let ctx = egui::Context::default();
        let mut app = app();
        app.app_lock = crate::app_lock::AppLock::new(true);
        assert!(app.app_lock.is_locked());
        let queue = std::sync::Arc::new(std::sync::Mutex::new(vec![ControlCommand::Quit]));
        app.control_commands = Some(queue);
        app.handle_control_commands();
        app.apply_actions(&ctx);
        assert!(app.quit_requested);
        assert_eq!(
            fastframe_shell::Resident::closed(&app),
            fastframe_shell::Closed::Quit
        );
    }

    #[test]
    fn a_notification_behind_the_lock_keeps_its_account() {
        let directory = tempfile::tempdir().unwrap();
        let (mut app, _commands) = two_accounts(directory.path());
        app.app_lock = crate::app_lock::AppLock::new(true);
        app.notification_opens
            .lock()
            .unwrap()
            .push(crate::notify::NotificationTarget {
                account: AccountId::parse("2").unwrap(),
                chat: "15550003333@s.whatsapp.net".into(),
                message: Some("m1".into()),
            });
        app.handle_notification_opens();
        assert_eq!(
            app.account().id,
            AccountId::first(),
            "nothing changes while locked"
        );
        assert!(matches!(
            app.app_lock.deferred.as_slice(),
            [crate::app_lock::Deferred::Message(target)] if target.account.as_str() == "2"
        ));
    }

    #[test]
    fn removing_an_earlier_account_keeps_the_open_one() {
        let directory = tempfile::tempdir().unwrap();
        let dirs = AppDirs::under(directory.path());
        let mut app = App::headless(dirs.clone(), Settings::default()).0;
        let second = Account::detached(
            &dirs,
            AccountId::parse("2").unwrap(),
            crate::settings::AccountSettings::default(),
        )
        .unwrap()
        .0;
        let third = Account::detached(
            &dirs,
            AccountId::parse("3").unwrap(),
            crate::settings::AccountSettings::default(),
        )
        .unwrap()
        .0;
        app.accounts.push(second);
        app.accounts.push(third);
        app.active = 1;
        app.composer = "keep typing".into();
        let kept = app.account().id.clone();
        app.finish_removed(0);
        assert_eq!(app.account().id, kept);
        assert_eq!(app.accounts.len(), 2);
        assert_eq!(app.composer, "keep typing");
    }

    #[test]
    fn a_hidden_account_draft_does_not_fill_the_open_composer() {
        let directory = tempfile::tempdir().unwrap();
        let dirs = AppDirs::under(directory.path());
        let mut app = App::headless(dirs.clone(), Settings::default()).0;
        let (second, events) = Account::detached(
            &dirs,
            AccountId::parse("2").unwrap(),
            crate::settings::AccountSettings::default(),
        )
        .unwrap();
        app.accounts.push(second);
        app.active = 0;
        app.composer = "visible".into();
        app.accounts[1].open_chat = Some("1@s.whatsapp.net".into());
        events
            .send(crate::backend::Event::Drafts(vec![(
                "1@s.whatsapp.net".into(),
                "from work".into(),
            )]))
            .unwrap();
        app.handle_events();
        assert_eq!(app.composer, "visible");
        assert_eq!(
            app.accounts[1]
                .drafts
                .get("1@s.whatsapp.net")
                .map(String::as_str),
            Some("from work")
        );
    }

    #[test]
    fn a_notification_action_keeps_its_account_when_another_emits() {
        let directory = tempfile::tempdir().unwrap();
        let dirs = AppDirs::under(directory.path());
        let mut app = App::headless(dirs.clone(), Settings::default()).0;
        let (second, events) = Account::detached(
            &dirs,
            AccountId::parse("2").unwrap(),
            crate::settings::AccountSettings::default(),
        )
        .unwrap();
        app.accounts.push(second);
        app.active = 0;
        app.actions.push(Action::ShowWindow);
        events
            .send(crate::backend::Event::ChatSoundPicked {
                chat: "1@s.whatsapp.net".into(),
                path: directory.path().join("ding.ogg"),
            })
            .unwrap();
        app.handle_events();
        assert!(
            app.actions
                .iter()
                .any(|action| matches!(action, Action::ShowWindow)),
            "a click waiting before the batch stays untagged"
        );
        assert_eq!(app.deferred_account_actions.len(), 1);
        assert_eq!(app.deferred_account_actions[0].0.as_str(), "2");
    }

    /// A trackpad gesture keeps its pane while fastframe-scroll says it goes
    /// on: a frame that is slow to draw is no pause, and the gesture's glide
    /// stays with the pane too. A pause in the input itself ends the gesture.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_trackpad_gesture_keeps_its_pane_through_a_slow_frame_and_its_glide() {
        let mut app = app();
        let ctx = egui::Context::default();
        let run = |app: &mut App, time: f64, delta: egui::Vec2| {
            let events = if delta == egui::Vec2::ZERO {
                Vec::new()
            } else {
                vec![egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta,
                    modifiers: egui::Modifiers::NONE,
                    phase: egui::TouchPhase::Move,
                }]
            };
            let mut output = ctx.run_ui(
                egui::RawInput {
                    time: Some(time),
                    events,
                    ..Default::default()
                },
                |ui| {
                    let ctx = ui.ctx().clone();
                    app.scrolling.apply(&ctx);
                    app.route_scroll(&ctx);
                },
            );
            output.textures_delta.clear();
        };
        for frame in 0..6 {
            run(&mut app, f64::from(frame) / 60.0, egui::vec2(0.0, 30.0));
        }
        assert!(app.scrolling.from_trackpad());
        app.scroll_route.owner = Some(ScrollPane::Messages);
        // A frame that takes a quarter of a second to draw, 16 ms of input
        // time after the last: nothing about the gesture has changed.
        std::thread::sleep(Duration::from_millis(250));
        run(&mut app, 6.0 / 60.0, egui::Vec2::ZERO);
        assert!(
            !app.scrolling.gliding(),
            "a slow frame lets the gesture glide"
        );
        assert_eq!(
            app.scroll_route.owner,
            Some(ScrollPane::Messages),
            "a slow frame lets go of the gesture's pane"
        );
        // A pause in the input itself ends the gesture, which glides, still
        // with its pane.
        run(&mut app, 6.0 / 60.0 + 0.2, egui::Vec2::ZERO);
        assert!(app.scrolling.gliding(), "the lifted gesture glides");
        run(&mut app, 6.0 / 60.0 + 0.25, egui::Vec2::ZERO);
        assert_eq!(
            app.scroll_route.owner,
            Some(ScrollPane::Messages),
            "the glide lets go of the gesture's pane"
        );
        // Once the glide is over, the pane is free for the next gesture.
        let mut time = 6.0 / 60.0 + 0.25;
        while app.scrolling.gliding() && time < 10.0 {
            time += 1.0 / 60.0;
            run(&mut app, time, egui::Vec2::ZERO);
        }
        assert!(!app.scrolling.gliding(), "the glide stops");
        run(&mut app, time + 1.0, egui::Vec2::ZERO);
        assert_eq!(app.scroll_route.owner, None, "the next gesture picks");
    }

    /// The chat a link asked for can refuse to open: a locked chat outside
    /// its folder stays shut, leaving the reader where they were. Its
    /// template must not follow them there.
    #[test]
    fn a_template_does_not_land_in_another_chat() {
        let ctx = egui::Context::default();
        let mut app = app();
        let here = "here@s.whatsapp.net";
        let locked = "locked@s.whatsapp.net";
        app.chats.push(Chat::new(here.into(), "Here".into()));
        let mut shut = Chat::new(locked.into(), "Shut".into());
        shut.locked = true;
        app.chats.push(shut);
        app.open_chat = Some(here.into());

        app.apply(
            Action::PrefillComposer {
                chat: locked.into(),
                text: "This is not here".into(),
            },
            &ctx,
        );

        assert_eq!(app.open_chat.as_deref(), Some(here));
        assert!(
            app.composer.is_empty(),
            "the template stayed out of the open chat: {:?}",
            app.composer
        );
    }

    /// A link's template is a start, not something the reader left behind:
    /// leaving the chat it filled must not store it as their draft.
    #[test]
    fn a_template_is_not_stored_as_a_draft() {
        let ctx = egui::Context::default();
        let mut app = app();
        let chat = "peer@s.whatsapp.net";
        let other = "other@s.whatsapp.net";
        app.chats.push(Chat::new(chat.into(), "Peer".into()));
        app.chats.push(Chat::new(other.into(), "Other".into()));

        app.apply(
            Action::StartChat {
                dismiss_dialog: true,
                id: chat.into(),
                name: "Peer".into(),
            },
            &ctx,
        );
        app.apply(
            Action::PrefillComposer {
                chat: chat.into(),
                text: "Invoice 123 is attached".into(),
            },
            &ctx,
        );
        assert_eq!(app.composer, "Invoice 123 is attached");

        app.apply(Action::OpenChat(other.into()), &ctx);

        assert_eq!(
            app.drafts.get(chat),
            None,
            "the template became a draft: {:?}",
            app.drafts.get(chat)
        );
    }

    /// Every path that saves the composer has to know a template is not the
    /// reader's text, not only the one that switches chats.
    #[test]
    fn a_template_is_not_saved_by_any_other_path_either() {
        for save in [
            // Closing the chat.
            "close", // Switching account parks the composer.
            "park",  // Locking or shutting down flushes it.
            "flush", // Hiding a locked chat stores it.
            "hide",
        ] {
            let ctx = egui::Context::default();
            let mut app = app();
            let chat = "peer@s.whatsapp.net";
            app.chats.push(Chat::new(chat.into(), "Peer".into()));
            app.apply(
                Action::StartChat {
                    dismiss_dialog: true,
                    id: chat.into(),
                    name: "Peer".into(),
                },
                &ctx,
            );
            app.apply(
                Action::PrefillComposer {
                    chat: chat.into(),
                    text: "Invoice 123".into(),
                },
                &ctx,
            );

            match save {
                "close" => app.apply(Action::CloseChat, &ctx),
                "park" => app.park_composer(),
                "flush" => app.flush_open_draft(),
                "hide" => app.hide_locked_chat(chat),
                _ => unreachable!(),
            }

            assert_eq!(
                app.drafts.get(chat),
                None,
                "the {save} path stored the template as a draft: {:?}",
                app.drafts.get(chat)
            );
        }
    }

    /// The same paths must not delete a draft the reader did write, which is
    /// the other half of the same rule.
    #[test]
    fn a_template_never_deletes_the_readers_draft() {
        let ctx = egui::Context::default();
        let mut app = app();
        let chat = "peer@s.whatsapp.net";
        app.chats.push(Chat::new(chat.into(), "Peer".into()));
        app.apply(
            Action::StartChat {
                dismiss_dialog: true,
                id: chat.into(),
                name: "Peer".into(),
            },
            &ctx,
        );
        // The reader's draft arrives while the chat is open, as it does when
        // the archive answers after the window is up.
        app.drafts.insert(chat.into(), "Half written".into());
        app.composer = "Half written".into();

        app.apply(Action::CloseChat, &ctx);

        assert_eq!(
            app.drafts.get(chat).map(String::as_str),
            Some("Half written"),
            "the reader's own draft survived"
        );
    }

    /// A reply the reader armed is theirs to send: a template must not take
    /// the composer and drop the quote with it.
    #[test]
    fn a_template_leaves_an_armed_reply_alone() {
        let ctx = egui::Context::default();
        let mut app = app();
        let chat = "peer@s.whatsapp.net";
        app.chats.push(Chat::new(chat.into(), "Peer".into()));
        app.apply(
            Action::StartChat {
                dismiss_dialog: true,
                id: chat.into(),
                name: "Peer".into(),
            },
            &ctx,
        );
        app.reply_to = Some("m1".into());

        app.apply(
            Action::PrefillComposer {
                chat: chat.into(),
                text: "Invoice 123".into(),
            },
            &ctx,
        );

        assert_eq!(app.reply_to.as_deref(), Some("m1"), "the reply is kept");
        assert!(app.composer.is_empty(), "and the composer is untouched");
    }

    /// An edit of a template is the reader's own text, and is kept the way
    /// any other unsent text is.
    #[test]
    fn an_edited_template_is_kept_as_a_draft() {
        let ctx = egui::Context::default();
        let mut app = app();
        let chat = "peer@s.whatsapp.net";
        let other = "other@s.whatsapp.net";
        app.chats.push(Chat::new(chat.into(), "Peer".into()));
        app.chats.push(Chat::new(other.into(), "Other".into()));

        app.apply(
            Action::StartChat {
                dismiss_dialog: true,
                id: chat.into(),
                name: "Peer".into(),
            },
            &ctx,
        );
        app.apply(
            Action::PrefillComposer {
                chat: chat.into(),
                text: "Invoice 123".into(),
            },
            &ctx,
        );
        app.composer = "Invoice 123, paid".into();

        app.apply(Action::OpenChat(other.into()), &ctx);

        assert_eq!(
            app.drafts.get(chat).map(String::as_str),
            Some("Invoice 123, paid")
        );
    }

    /// Unsent text the reader wrote wins over a template: a link does not
    /// replace what they were already going to send.
    #[test]
    fn a_template_leaves_the_readers_own_draft_alone() {
        let ctx = egui::Context::default();
        let mut app = app();
        let chat = "peer@s.whatsapp.net";
        let other = "other@s.whatsapp.net";
        app.chats.push(Chat::new(chat.into(), "Peer".into()));
        app.chats.push(Chat::new(other.into(), "Other".into()));
        app.drafts.insert(chat.into(), "Half written".into());

        app.apply(
            Action::StartChat {
                dismiss_dialog: true,
                id: chat.into(),
                name: "Peer".into(),
            },
            &ctx,
        );
        app.apply(
            Action::PrefillComposer {
                chat: chat.into(),
                text: "Invoice 123".into(),
            },
            &ctx,
        );

        assert_eq!(app.composer, "Half written", "the draft is what is shown");

        // Opening the chat took the draft into the composer, as it does for
        // any chat. Leaving stores it back, and it is still the reader's.
        app.apply(Action::OpenChat(other.into()), &ctx);
        assert_eq!(
            app.drafts.get(chat).map(String::as_str),
            Some("Half written"),
            "the template did not become the stored draft"
        );
        app.apply(Action::OpenChat(chat.into()), &ctx);
        assert_eq!(app.composer, "Half written", "and it comes back");
    }

    /// A chat that is gone or emptied takes its confirmation with it: a modal
    /// left behind for a chat that no longer exists still dispatches its
    /// action, and after a remote clear that action would take what arrived
    /// since.
    #[test]
    fn a_remote_removal_or_clear_closes_the_confirmation() {
        let mut app = app();
        let (backend, events) = Backend::detached();
        app.backend = backend;
        let chat = "peer@s.whatsapp.net";
        app.chats.push(Chat::new(chat.into(), "Peer".into()));

        app.dialog = Some(Dialog::ConfirmClearChat(chat.into()));
        events
            .send(Event::ChatCleared {
                chat: chat.into(),
                through: 100,
            })
            .unwrap();
        app.handle_events();
        assert!(
            app.dialog.is_none(),
            "a cleared chat closes its confirmation"
        );

        app.dialog = Some(Dialog::ConfirmClearChat(chat.into()));
        events
            .send(Event::ChatRemoved { chat: chat.into() })
            .unwrap();
        app.handle_events();
        assert!(
            app.dialog.is_none(),
            "a removed chat closes its confirmation"
        );
    }

    /// Demo and test runs share the machine with a linked ZapFast, whose real
    /// taskbar badge they must not overwrite.
    /// A background page is not an answer to a request the reader made.
    /// Counting it as one would look like the phone missing, and back off the
    /// next scroll by the miss cooldown.
    #[test]
    fn a_silent_page_does_not_count_against_the_phone() {
        let mut app = app();
        let (backend, events) = Backend::detached();
        app.backend = backend;
        let chat = "peer@s.whatsapp.net";
        {
            let conversation = app.conversations.entry(chat.into()).or_default();
            conversation.fetching_phone = true;
            conversation.phone_misses = 2;
            conversation.complete = true;
        }

        events
            .send(Event::OlderFetched {
                chat: chat.into(),
                more: true,
                silent: true,
            })
            .unwrap();
        app.handle_events();

        let conversation = app.conversations.get(chat).expect("the chat");
        assert_eq!(
            conversation.phone_misses, 2,
            "a background page is not a miss"
        );
        assert!(
            !conversation.complete,
            "and it sends the view back to the archive, which holds the older messages the page just filed"
        );
        assert!(
            conversation.fetching_phone,
            "a background page leaves the reader's request in flight"
        );

        // A page the reader asked for still counts, and ends the wait.
        events
            .send(Event::OlderFetched {
                chat: chat.into(),
                more: false,
                silent: false,
            })
            .unwrap();
        app.handle_events();
        let conversation = app.conversations.get(chat).expect("the chat");
        assert_eq!(conversation.phone_misses, 3);
        assert!(conversation.phone_exhausted);
        assert!(!conversation.complete);
        assert!(!conversation.fetching_phone);
    }

    #[test]
    fn demo_and_test_runs_do_not_publish_a_taskbar_badge() {
        let app = app();
        assert!(app.badge.is_none());
        #[cfg(target_os = "windows")]
        assert!(app.taskbar_badge_count().is_none());
    }

    /// Every account reconnects when the system reports a wake, and a sleep
    /// is answered at once, with the settings saved.
    #[test]
    fn a_wake_reconnects_every_account_and_a_sleep_saves_first() {
        let mut app = app();
        let (backend, mut commands) = Backend::recording();
        app.backend = backend;
        app.power.send(fastframe_power::Event::Resumed);
        app.handle_power();
        assert!(matches!(commands.try_recv(), Ok(Command::Resumed)));
        assert!(commands.try_recv().is_err(), "once per wake");

        app.settings_dirty = true;
        let ack = fastframe_power::Ack::default();
        app.power.send(fastframe_power::Event::Suspending(ack));
        app.handle_power();
        assert!(!app.settings_dirty, "saved before sleeping");
        assert!(commands.try_recv().is_err());
    }

    /// A settings save hands its bytes to the file writer and returns while
    /// the disk is still busy, so the window never waits for it.
    #[test]
    fn a_settings_save_does_not_wait_for_the_disk() {
        let root = tempfile::tempdir().unwrap();
        let (mut app, _events) = App::headless(AppDirs::under(root.path()), Settings::default());
        let (release, gate) = std::sync::mpsc::channel::<()>();
        let gate = std::sync::Mutex::new(gate);
        let (written, writes) = std::sync::mpsc::channel();
        app.files = fastframe_file::Writer::with_write(move |path, contents, access| {
            gate.lock().unwrap().recv().unwrap();
            written
                .send((path.to_owned(), contents.to_vec(), access))
                .unwrap();
            Ok(())
        });
        app.settings.check_for_updates = false;
        app.save_settings();
        assert!(writes.try_recv().is_err(), "nothing written yet");
        release.send(()).unwrap();
        assert!(app.files.flush(Duration::from_secs(10)));
        let (path, contents, access) = writes.try_recv().expect("written after the flush");
        assert_eq!(path, app.dirs.settings_file());
        assert_eq!(access, fastframe_file::Access::Default);
        let saved: serde_json::Value = serde_json::from_slice(&contents).unwrap();
        assert_eq!(saved["check_for_updates"], false);
    }

    /// Settings and account settings waiting for their save are on the disk
    /// by the time a sleep is acknowledged, and again when the app quits.
    #[test]
    fn sleeping_and_quitting_wait_for_the_settings_to_reach_the_disk() {
        let root = tempfile::tempdir().unwrap();
        let (mut app, _events) = App::headless(AppDirs::under(root.path()), Settings::default());
        app.settings.check_for_updates = false;
        app.mark_settings_dirty();
        app.account_mut().settings.history_prefetch = true;
        app.account_mut().mark_settings_dirty();
        app.power.send(fastframe_power::Event::Suspending(
            fastframe_power::Ack::default(),
        ));
        app.handle_power();
        assert!(!Settings::load(&app.dirs.settings_file()).check_for_updates);
        let account_file = app.account().dirs.settings_file();
        assert!(crate::settings::AccountSettings::load(&account_file).history_prefetch);

        app.settings.check_for_updates = true;
        app.mark_settings_dirty();
        app.shutdown();
        assert!(Settings::load(&app.dirs.settings_file()).check_for_updates);
    }

    /// The window's geometry is remembered in the logic pass, which eframe
    /// runs for a minimized window too, and only a change marks the settings
    /// for saving.
    #[test]
    fn the_logic_pass_remembers_the_window_geometry() {
        let mut app = app();
        let ctx = egui::Context::default();
        let run = |app: &mut App, rect: egui::Rect| {
            let mut input = egui::RawInput::default();
            let info = input.viewports.entry(egui::ViewportId::ROOT).or_default();
            info.inner_rect = Some(rect);
            info.outer_rect = Some(rect.expand(4.0));
            info.maximized = Some(false);
            let mut output = ctx.run_ui(input, |ui| app.window_logic(ui.ctx()));
            output.textures_delta.clear();
        };
        let rect = egui::Rect::from_min_size(egui::pos2(100.0, 50.0), egui::vec2(1280.0, 800.0));
        run(&mut app, rect);
        assert!(app.settings_dirty);
        assert_eq!(
            (app.settings.window_width, app.settings.window_height),
            (Some(1280.0), Some(800.0))
        );

        app.settings_dirty = false;
        run(&mut app, rect);
        assert!(!app.settings_dirty, "nothing changed, nothing to save");
    }

    /// A short chat asks the phone by itself; only the reader scrolling to
    /// the top makes a request theirs, so only then may a silent phone be
    /// reported (#325). Scrolling up during an automatic request claims it
    /// once rather than asking twice.
    #[test]
    fn only_the_reader_scrolling_up_asks_the_phone_explicitly() {
        let mut app = app();
        let (backend, mut commands, events) = Backend::recording_with_events();
        app.backend = backend;
        app.link = LinkStatus::Connected;
        let chat = "4915700000003@s.whatsapp.net";
        let conversation = app.conversations.entry(chat.into()).or_default();
        conversation.merge(vec![message(chat, "m1", 100)], false);
        conversation.complete = true;
        let mut asked = || {
            std::iter::from_fn(|| commands.try_recv().ok())
                .filter_map(|command| match command {
                    Command::FetchOlder { explicit, .. } => Some(explicit),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };

        app.load_older(chat, false);
        assert_eq!(asked(), [false]);
        app.load_older(chat, false);
        assert_eq!(asked(), [false; 0], "one request at a time");
        app.load_older(chat, true);
        assert_eq!(asked(), [true], "the reader claims the waiting request");
        app.load_older(chat, true);
        assert_eq!(asked(), [false; 0], "and only once");

        events
            .send(Event::OlderFetched {
                chat: chat.into(),
                more: false,
                silent: false,
            })
            .unwrap();
        app.handle_events();
        assert!(!app.conversations[chat].phone_explicit);
        app.load_older(chat, true);
        assert_eq!(
            asked(),
            [false; 0],
            "a chat at its start is not asked again"
        );
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
        assert!(!app.account().settings.save_contacts_to_phone);
        assert!(app.account().settings_dirty);
        app.apply(Action::ShowDialog(Dialog::NewContact), &ctx);
        assert!(!app.new_contact_to_phone, "the next dialog starts from it");
        // Opening a chat without a name saves nothing, so it keeps the choice.
        app.apply(add(Some(true), ""), &ctx);
        assert!(to_phone(&mut commands));
        assert!(!app.account().settings.save_contacts_to_phone);
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
            received: Vec::new(),
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
        events: Vec<egui::Event>,
        image: bool,
    ) -> usize {
        clipboard_frame_with_files(app, ctx, events, None, image).0
    }

    /// Runs a frame whose clipboard holds `files` and, if `image`, a picture.
    /// Returns how often the clipboard and its picture were read.
    fn clipboard_frame_with_files(
        app: &mut App,
        ctx: &egui::Context,
        mut events: Vec<egui::Event>,
        files: Option<Vec<PathBuf>>,
        image: bool,
    ) -> (usize, usize) {
        let mut reads = 0;
        let mut image_reads = 0;
        if !events
            .iter()
            .any(|event| matches!(event, egui::Event::ModifiersChanged(_)))
        {
            events.insert(0, egui::Event::ModifiersChanged(egui::Modifiers::COMMAND));
        }
        let mut output = ctx.run_ui(
            egui::RawInput {
                events,
                ..Default::default()
            },
            |ui| {
                app.take_clipboard_paste(ui.ctx(), || {
                    reads += 1;
                    clipboard_contents(
                        || files.clone().ok_or_else(|| "no fixture files".to_owned()),
                        || {
                            image_reads += 1;
                            image
                                .then(|| (2, 2, vec![200; 16]))
                                .ok_or_else(|| "no fixture picture".to_owned())
                        },
                    )
                    .ok()
                });
                ui.add(
                    egui::TextEdit::singleline(&mut app.composer)
                        .id(egui::Id::new("composer-text")),
                );
                app.apply_actions(ui.ctx());
            },
        );
        output.textures_delta.clear();
        (reads, image_reads)
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
    fn plain_text_paste_does_not_read_attachments() {
        for modifiers in [
            egui::Modifiers {
                ctrl: true,
                command: true,
                shift: true,
                ..egui::Modifiers::NONE
            },
            egui::Modifiers {
                mac_cmd: true,
                command: true,
                shift: true,
                ..egui::Modifiers::NONE
            },
            egui::Modifiers {
                mac_cmd: true,
                command: true,
                shift: true,
                alt: true,
                ..egui::Modifiers::NONE
            },
        ] {
            let (mut app, ctx) = clipboard_app();
            assert_eq!(
                clipboard_frame(
                    &mut app,
                    &ctx,
                    vec![
                        egui::Event::ModifiersChanged(modifiers),
                        egui::Event::Paste("plain ✓".into()),
                    ],
                    true
                ),
                0
            );
            assert_eq!(app.composer, "captionplain ✓");
            assert!(app.pending.is_empty());
            let mut release = paste_release();
            if let egui::Event::Key { modifiers, .. } = &mut release {
                *modifiers = egui::Modifiers::NONE;
            }
            assert_eq!(
                clipboard_frame(
                    &mut app,
                    &ctx,
                    vec![
                        egui::Event::ModifiersChanged(egui::Modifiers::NONE),
                        release,
                    ],
                    true
                ),
                0
            );
            assert!(app.pending.is_empty());
            assert_eq!(
                clipboard_frame(
                    &mut app,
                    &ctx,
                    vec![egui::Event::Paste("image URL".into()),],
                    true
                ),
                1
            );
            assert_eq!(
                app.pending.len(),
                1,
                "a subsequent ordinary paste still stages images"
            );
        }
    }

    #[test]
    fn plain_text_paste_without_text_does_not_attach_on_release() {
        let (mut app, ctx) = clipboard_app();
        let modifiers = egui::Modifiers::COMMAND | egui::Modifiers::SHIFT;
        assert_eq!(
            clipboard_frame(
                &mut app,
                &ctx,
                vec![egui::Event::ModifiersChanged(modifiers),],
                true
            ),
            0
        );
        let mut release = paste_release();
        if let egui::Event::Key { modifiers, .. } = &mut release {
            *modifiers = egui::Modifiers::NONE;
        }
        assert_eq!(
            clipboard_frame(
                &mut app,
                &ctx,
                vec![
                    egui::Event::ModifiersChanged(egui::Modifiers::NONE),
                    release,
                ],
                true
            ),
            0
        );
        assert_eq!(app.composer, "caption");
        assert!(app.pending.is_empty());
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
                app.take_clipboard_paste(ui.ctx(), || {
                    Some(ClipboardPaste::Image {
                        width: 2,
                        height: 2,
                        rgba: vec![200; 16],
                    })
                });
                app.apply_actions(ui.ctx());
            },
        );
        output.textures_delta.clear();
        assert_eq!(app.pending.len(), 1);
        clipboard_frame(&mut app, &ctx, vec![paste_release()], true);
        assert_eq!(app.pending.len(), 2);
    }

    /// A keyboard paste remains owned until V-up even if Control went up first.
    #[test]
    fn a_paste_after_modifier_up_is_not_repeated_on_v_up() {
        let (mut app, ctx) = clipboard_app();
        assert_eq!(clipboard_frame(&mut app, &ctx, vec![], true), 0);
        assert_eq!(
            clipboard_frame(
                &mut app,
                &ctx,
                vec![
                    egui::Event::ModifiersChanged(egui::Modifiers::NONE),
                    egui::Event::Paste("fixture image URL".into()),
                ],
                true,
            ),
            1,
        );
        let mut release = paste_release();
        if let egui::Event::Key { modifiers, .. } = &mut release {
            *modifiers = egui::Modifiers::NONE;
        }
        assert_eq!(
            clipboard_frame(
                &mut app,
                &ctx,
                vec![
                    egui::Event::ModifiersChanged(egui::Modifiers::NONE),
                    release
                ],
                true,
            ),
            0,
        );
        assert_eq!(app.pending.len(), 1);
    }

    /// Replayed drop input must stage once across discarded layout passes.
    #[test]
    fn a_layout_retry_does_not_stage_the_same_dropped_file_twice() {
        #[derive(Debug)]
        struct FixtureDrop(PathBuf);
        impl egui::DroppedFile for FixtureDrop {
            /// Supplies a synthetic path without consulting the user's files.
            fn path(&self) -> &std::path::Path {
                &self.0
            }
            /// Drop staging needs only the path, so fixture bytes are empty.
            fn bytes(&self) -> Result<Vec<u8>, String> {
                Ok(Vec::new())
            }
        }
        let (mut app, ctx) = clipboard_app();
        let mut passes = 0;
        let mut output = ctx.run_ui(
            egui::RawInput {
                dropped_files: vec![std::sync::Arc::new(FixtureDrop("fixture.png".into()))],
                ..Default::default()
            },
            |ui| {
                passes += 1;
                app.take_drops_and_pastes(ui.ctx());
                if passes == 1 {
                    ui.ctx().request_discard("fixture drop relayout");
                }
            },
        );
        output.textures_delta.clear();
        assert_eq!(passes, 2);
        assert_eq!(app.pending.len(), 1);
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
    fn screenshot_paste_survives_releasing_control_before_v() {
        let (mut app, ctx) = clipboard_app();
        let mut release = paste_release();
        if let egui::Event::Key { modifiers, .. } = &mut release {
            *modifiers = egui::Modifiers::NONE;
        }
        assert_eq!(clipboard_frame(&mut app, &ctx, vec![], true), 0);
        assert_eq!(
            clipboard_frame(
                &mut app,
                &ctx,
                vec![egui::Event::ModifiersChanged(egui::Modifiers::NONE)],
                true
            ),
            0
        );
        clipboard_frame(
            &mut app,
            &ctx,
            vec![
                egui::Event::ModifiersChanged(egui::Modifiers::NONE),
                release.clone(),
            ],
            true,
        );
        assert_eq!(app.pending.len(), 1);
        clipboard_frame(&mut app, &ctx, vec![release], true);
        assert_eq!(app.pending.len(), 2, "each shortcut stages one image");
    }

    /// Runs clipboard frames at the given input times, returning how often
    /// the clipboard was read.
    fn timed_clipboard_frames(
        app: &mut App,
        ctx: &egui::Context,
        frames: Vec<(f64, Vec<egui::Event>)>,
    ) -> usize {
        let mut reads = 0;
        for (time, events) in frames {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    time: Some(time),
                    events,
                    ..Default::default()
                },
                |ui| {
                    app.take_clipboard_paste(ui.ctx(), || {
                        reads += 1;
                        Some(ClipboardPaste::Image {
                            width: 2,
                            height: 2,
                            rgba: vec![200; 16],
                        })
                    });
                },
            );
            output.textures_delta.clear();
        }
        reads
    }

    fn bare_v_release() -> egui::Event {
        let mut release = paste_release();
        if let egui::Event::Key { modifiers, .. } = &mut release {
            *modifiers = egui::Modifiers::NONE;
        }
        release
    }

    /// A quick tap whose events land in one frame (#494): by the V release,
    /// neither the event nor the frame says Ctrl any more.
    #[test]
    fn a_quick_tap_releasing_control_first_pastes_within_one_frame() {
        let (mut app, ctx) = clipboard_app();
        let up = egui::Event::ModifiersChanged(egui::Modifiers::NONE);
        let down = egui::Event::ModifiersChanged(egui::Modifiers::COMMAND);
        let reads = timed_clipboard_frames(
            &mut app,
            &ctx,
            vec![
                (10.0, vec![up.clone()]),
                (20.0, vec![down, up, bare_v_release()]),
            ],
        );
        assert_eq!(reads, 1);
        assert_eq!(app.pending.len(), 1);
    }

    /// A V release without a press of its own long after Ctrl went up (an
    /// input method that kept the press, say) is not the shortcut.
    #[test]
    fn a_v_release_long_after_control_is_not_a_paste() {
        let (mut app, ctx) = clipboard_app();
        let reads = timed_clipboard_frames(
            &mut app,
            &ctx,
            vec![
                (
                    10.0,
                    vec![egui::Event::ModifiersChanged(egui::Modifiers::COMMAND)],
                ),
                (
                    10.1,
                    vec![egui::Event::ModifiersChanged(egui::Modifiers::NONE)],
                ),
                (10.2 + PASTE_RELEASE_GRACE, vec![bare_v_release()]),
            ],
        );
        assert_eq!(reads, 0);
        assert!(app.pending.is_empty());
    }

    #[test]
    fn a_bare_v_release_after_refocusing_never_reads_the_clipboard() {
        let (mut app, ctx) = clipboard_app();
        let mut reads = 0;
        let mut release = paste_release();
        if let egui::Event::Key { modifiers, .. } = &mut release {
            *modifiers = egui::Modifiers::NONE;
        }
        for (focused, events) in [
            (
                true,
                vec![egui::Event::ModifiersChanged(egui::Modifiers::COMMAND)],
            ),
            (false, vec![]),
            (true, vec![release]),
        ] {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    focused,
                    events,
                    ..Default::default()
                },
                |ui| {
                    app.take_clipboard_paste(ui.ctx(), || {
                        reads += 1;
                        Some(ClipboardPaste::Image {
                            width: 2,
                            height: 2,
                            rgba: vec![200; 16],
                        })
                    });
                },
            );
            output.textures_delta.clear();
        }
        assert_eq!(reads, 0);
        assert!(app.pending.is_empty());
    }

    #[test]
    fn typing_v_does_not_paste_a_screenshot_on_release() {
        let (mut app, ctx) = clipboard_app();
        let mut press = paste_release();
        if let egui::Event::Key {
            pressed, modifiers, ..
        } = &mut press
        {
            *pressed = true;
            *modifiers = egui::Modifiers::NONE;
        }
        let mut release = paste_release();
        if let egui::Event::Key { modifiers, .. } = &mut release {
            *modifiers = egui::Modifiers::NONE;
        }
        assert_eq!(clipboard_frame(&mut app, &ctx, vec![press], true), 0);
        assert_eq!(clipboard_frame(&mut app, &ctx, vec![release], true), 0);
        assert!(app.pending.is_empty());
    }

    /// Adding Control before an ordinary V release never reads images or copied files.
    #[test]
    fn typing_v_then_control_before_release_never_stages_attachments() {
        for same_frame in [true, false] {
            for files in [None, Some(vec![PathBuf::from("unrelated-fixture.png")])] {
                let (mut app, ctx) = clipboard_app();
                let mut press = paste_release();
                if let egui::Event::Key {
                    pressed, modifiers, ..
                } = &mut press
                {
                    *pressed = true;
                    *modifiers = egui::Modifiers::NONE;
                }
                let events = if same_frame {
                    vec![
                        egui::Event::ModifiersChanged(egui::Modifiers::NONE),
                        press,
                        egui::Event::ModifiersChanged(egui::Modifiers::COMMAND),
                        paste_release(),
                    ]
                } else {
                    assert_eq!(
                        clipboard_frame_with_files(
                            &mut app,
                            &ctx,
                            vec![egui::Event::ModifiersChanged(egui::Modifiers::NONE), press],
                            files.clone(),
                            true
                        ),
                        (0, 0)
                    );
                    vec![
                        egui::Event::ModifiersChanged(egui::Modifiers::COMMAND),
                        paste_release(),
                    ]
                };
                assert_eq!(
                    clipboard_frame_with_files(&mut app, &ctx, events, files, true),
                    (0, 0)
                );
                assert!(app.pending.is_empty());
            }
        }
    }

    #[test]
    fn screenshot_paste_takes_focus_back_from_a_message() {
        let (mut app, ctx) = clipboard_app();
        ctx.memory_mut(|memory| memory.request_focus(egui::Id::new("fixture-message")));
        clipboard_frame(&mut app, &ctx, vec![paste_release()], true);
        assert_eq!(app.pending.len(), 1);
        assert!(ctx.memory(|memory| memory.has_focus(egui::Id::new("composer-text"))));
    }

    #[test]
    fn a_layout_retry_does_not_stage_the_same_screenshot_twice() {
        let (mut app, ctx) = clipboard_app();
        let mut passes = 0;
        let mut reads = 0;
        let mut output = ctx.run_ui(
            egui::RawInput {
                events: vec![paste_release()],
                ..Default::default()
            },
            |ui| {
                passes += 1;
                app.take_clipboard_paste(ui.ctx(), || {
                    reads += 1;
                    Some(ClipboardPaste::Image {
                        width: 2,
                        height: 2,
                        rgba: vec![200; 16],
                    })
                });
                app.apply_actions(ui.ctx());
                if passes == 1 {
                    ui.ctx().request_discard("fixture relayout");
                }
            },
        );
        output.textures_delta.clear();
        assert_eq!(passes, 2);
        assert_eq!(reads, 1);
        assert_eq!(app.pending.len(), 1);
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
    fn a_copied_file_stages_the_file_and_not_its_icon() {
        let directory = tempfile::tempdir().unwrap();
        let pdf = directory.path().join("fixture.pdf");
        let zip = directory.path().join("fixture archive.zip");
        std::fs::write(&pdf, b"%PDF-fixture").unwrap();
        std::fs::write(&zip, b"PK-fixture").unwrap();
        let (mut app, ctx) = clipboard_app();
        // Finder offers the path, the file name as text, and an icon picture.
        let (reads, image_reads) = clipboard_frame_with_files(
            &mut app,
            &ctx,
            vec![egui::Event::Paste("fixture.pdf".into()), paste_release()],
            Some(vec![pdf.clone(), zip.clone()]),
            true,
        );
        assert_eq!(reads, 1);
        assert_eq!(image_reads, 0, "the icon picture is never read");
        assert!(matches!(
            app.pending.as_slice(),
            [
                Staged { item: Pending::File(first), .. },
                Staged { item: Pending::File(second), .. }
            ] if *first == pdf && *second == zip
        ));
        assert_eq!(app.composer, "caption", "the file name is not pasted");
    }

    #[test]
    fn a_uri_list_line_ending_does_not_hide_the_file() {
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("fixture.zip");
        std::fs::write(&file, b"PK-fixture").unwrap();
        let mut listed = file.clone().into_os_string();
        listed.push("\r");
        let contents = clipboard_contents(
            || Ok(vec![PathBuf::from(listed)]),
            || Err("no fixture picture".to_owned()),
        );
        assert!(matches!(contents, Ok(ClipboardPaste::Files(paths)) if paths == [file]));
    }

    /// A file copied in Explorer and pasted with a quick Ctrl+V right after
    /// switching back (#404): egui-winit swallows the V press while it looks
    /// for text that is not there, so only the V release arrives, after Ctrl
    /// already went up, and a chat row clicked to get here may still hold the
    /// focus. Explorer offers the file's icon too, and the file must win.
    #[test]
    fn a_file_copied_in_explorer_pastes_on_a_quick_tap_after_switching_back() {
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("fixture report.pdf");
        std::fs::write(&file, b"%PDF-fixture").unwrap();
        let (mut app, ctx) = clipboard_app();
        ctx.memory_mut(|memory| memory.request_focus(egui::Id::new("fixture-chat-row")));
        let up = egui::Event::ModifiersChanged(egui::Modifiers::NONE);
        let down = egui::Event::ModifiersChanged(egui::Modifiers::COMMAND);
        let mut reads = 0;
        for (time, events) in [
            (10.0, vec![egui::Event::WindowFocused(true), up.clone()]),
            (10.5, vec![down]),
            (10.6, vec![up, bare_v_release()]),
        ] {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    time: Some(time),
                    events,
                    ..Default::default()
                },
                |ui| {
                    app.take_clipboard_paste(ui.ctx(), || {
                        reads += 1;
                        clipboard_contents(|| Ok(vec![file.clone()]), || Ok((2, 2, vec![200; 16])))
                            .ok()
                    });
                    app.apply_actions(ui.ctx());
                },
            );
            output.textures_delta.clear();
        }
        assert_eq!(reads, 1);
        assert_eq!(app.pending.len(), 1);
        assert!(matches!(&app.pending[0].item, Pending::File(path) if *path == file));
    }

    /// A paste that finds nothing says why for the log, naming no file.
    #[test]
    fn an_empty_paste_explains_itself_without_naming_files() {
        let missing = PathBuf::from("/fixture/secret-name.pdf");
        let why = clipboard_contents(
            || Ok(vec![missing.clone()]),
            || Err("no picture here".to_owned()),
        )
        .unwrap_err();
        assert!(why.contains("1 copied files, none found on disk"), "{why}");
        assert!(why.contains("no picture here"), "{why}");
        assert!(!why.contains("secret-name"), "{why}");
        let why = clipboard_contents(
            || Err("format unavailable".to_owned()),
            || Err("empty".to_owned()),
        )
        .unwrap_err();
        assert_eq!(
            why,
            "no copied files (format unavailable), no picture (empty)"
        );
    }

    #[test]
    fn a_missing_copied_file_falls_back_to_the_picture() {
        let directory = tempfile::tempdir().unwrap();
        let (mut app, ctx) = clipboard_app();
        let (_, image_reads) = clipboard_frame_with_files(
            &mut app,
            &ctx,
            vec![paste_release()],
            Some(vec![directory.path().join("gone.pdf")]),
            true,
        );
        assert_eq!(image_reads, 1);
        assert!(matches!(
            app.pending.as_slice(),
            [Staged {
                item: Pending::Picture { .. },
                ..
            }]
        ));
    }

    #[test]
    fn clipboard_fallback_leaves_overlay_and_recording_focus_alone() {
        for state in ["reaction", "popup", "recording"] {
            let (mut app, ctx) = clipboard_app();
            let control = egui::Id::new("fixture-active-control");
            match state {
                "reaction" => {
                    app.reaction_target = Some(("fixture".into(), "fixture-message".into()))
                }
                "popup" => egui::Popup::open_id(&ctx, egui::Id::new("fixture-menu")),
                "recording" => app.recording = Some(crate::audio::Recorder::rehearsal()),
                _ => unreachable!(),
            }
            let mut reads = 0;
            let _ = ctx.run_ui(
                egui::RawInput {
                    events: vec![paste_release()],
                    ..Default::default()
                },
                |ui| {
                    let response = ui.interact(
                        egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(50.0, 20.0)),
                        control,
                        egui::Sense::click(),
                    );
                    response.request_focus();
                    app.take_clipboard_paste(ui.ctx(), || {
                        reads += 1;
                        Some(ClipboardPaste::Image {
                            width: 2,
                            height: 2,
                            rgba: vec![200; 16],
                        })
                    });
                    assert!(ui.memory(|memory| memory.has_focus(control)), "{state}");
                },
            );
            assert_eq!(reads, 0, "{state}");
            assert!(app.pending.is_empty(), "{state}");
        }
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

    /// A chat whose album holds `count` pictures.
    fn app_with_album(count: usize) -> App {
        let mut app = app();
        let chat = "1@s.whatsapp.net".to_owned();
        app.chats.push(Chat::new(chat.clone(), "Ada".into()));
        app.viewer_media = (0..count)
            .map(|index| crate::model::MediaItem {
                card: None,
                id: format!("m{index}"),
                timestamp: index as i64,
                video: false,
                path: Some(PathBuf::from(format!("/fixture/{index}.png"))),
                thumbnail: None,
            })
            .collect();
        app.viewer_media_chat = Some(chat);
        app
    }

    /// Opens the viewer on one item of that album.
    fn open_viewer(app: &mut App, index: usize) {
        app.image_preview = Some(PreviewState::new(
            Some(PathBuf::from(format!("/fixture/{index}.png"))),
            "1@s.whatsapp.net".into(),
            format!("m{index}"),
        ));
    }

    #[test]
    fn stepping_the_viewer_walks_the_album_and_stops_at_its_ends() {
        let mut app = app_with_album(3);
        let ctx = egui::Context::default();
        open_viewer(&mut app, 0);
        app.apply(Action::ViewerStep(-1), &ctx);
        assert_eq!(
            app.image_preview.as_ref().unwrap().message(),
            "m0",
            "nothing comes before the first"
        );
        app.apply(Action::ZoomImageIn, &ctx);
        app.apply(Action::ViewerStep(1), &ctx);
        let preview = app.image_preview.as_ref().unwrap();
        assert_eq!(preview.message(), "m1");
        assert!(preview.is_fit(), "the next picture starts fitted");
        app.apply(Action::ViewerStep(1), &ctx);
        assert_eq!(app.image_preview.as_ref().unwrap().message(), "m2");
        app.apply(Action::ViewerStep(1), &ctx);
        assert_eq!(
            app.image_preview.as_ref().unwrap().message(),
            "m2",
            "nothing comes after the last"
        );
        app.apply(Action::ViewerStep(-1), &ctx);
        assert_eq!(app.image_preview.as_ref().unwrap().message(), "m1");
    }

    /// An album item whose attachment is not here yet keeps that state. It is
    /// not shown as the picture that happens to be on screen: that would be
    /// another file under this item's name.
    #[test]
    fn an_item_without_a_file_is_not_shown_as_another_one() {
        let mut app = app_with_album(3);
        let ctx = egui::Context::default();
        app.viewer_media[2].path = None;
        open_viewer(&mut app, 0);
        app.apply(
            Action::ViewImage {
                card: None,
                message: "m2".into(),
            },
            &ctx,
        );
        let preview = app.image_preview.as_ref().unwrap();
        assert_eq!(preview.message(), "m2", "the viewer moved to the item");
        assert_eq!(preview.path(), None, "the previous picture is not reused");
        // Stepping onto it says the same.
        app.apply(Action::ViewerStep(-1), &ctx);
        assert_eq!(app.image_preview.as_ref().unwrap().message(), "m1");
        app.apply(Action::ViewerStep(1), &ctx);
        let preview = app.image_preview.as_ref().unwrap();
        assert_eq!(preview.message(), "m2");
        assert_eq!(preview.path(), None, "and stepping does not either");
    }

    /// An album item with no file is shown for what it is, and the viewer is
    /// waiting on the download it offered. When the file lands, the viewer has
    /// to be given it too, or it keeps offering the download it just did.
    #[test]
    fn a_download_gives_the_viewer_the_file_it_was_waiting_for() {
        let mut app = app_with_album(3);
        let ctx = egui::Context::default();
        app.viewer_media[2].path = None;
        open_viewer(&mut app, 0);
        app.apply(
            Action::ViewImage {
                card: None,
                message: "m2".into(),
            },
            &ctx,
        );
        assert_eq!(app.image_preview.as_ref().unwrap().path(), None);

        let (backend, events) = Backend::detached();
        app.backend = backend;
        events
            .send(Event::Media {
                card: None,
                chat: "1@s.whatsapp.net".into(),
                message: "m2".into(),
                result: Ok(PathBuf::from("/fixture/2.png")),
            })
            .unwrap();
        app.handle_events();

        let preview = app.image_preview.as_ref().unwrap();
        assert_eq!(preview.message(), "m2");
        assert_eq!(
            preview.path(),
            Some(std::path::Path::new("/fixture/2.png")),
            "the viewer takes the file that just arrived"
        );
        assert_eq!(
            app.viewer_media[2].path.as_deref(),
            Some(std::path::Path::new("/fixture/2.png")),
            "and so does the album"
        );
    }

    /// The album is centred on the message the viewer opened on, so a photo
    /// older than the newest page still has neighbours to step through.
    #[test]
    fn opening_a_photo_asks_for_the_album_around_it() {
        let ctx = egui::Context::default();
        let mut app = app_with_album(3);
        let (backend, mut commands) = Backend::recording();
        app.backend = backend;
        let file = tempfile::NamedTempFile::with_suffix(".png").unwrap();
        std::fs::write(file.path(), b"not a real image").unwrap();

        app.apply(
            Action::PreviewImage {
                card: None,
                path: file.path().to_owned(),
                chat: "1@s.whatsapp.net".into(),
                message: "m1".into(),
            },
            &ctx,
        );

        assert!(
            matches!(
                commands.try_recv(),
                Ok(Command::LoadChatMedia { ref around, .. }) if around.as_deref() == Some("m1")
            ),
            "the album is asked for around the opened message"
        );
    }

    /// The viewer tracks its own download: progress while it runs, and the
    /// worker's own message when it fails, instead of a button that looks dead.
    #[test]
    fn a_download_from_the_viewer_shows_progress_and_a_failure() {
        let ctx = egui::Context::default();
        let mut app = app_with_album(3);
        app.viewer_media[2].path = None;
        open_viewer(&mut app, 0);
        let (backend, events) = Backend::detached();
        app.backend = backend;

        app.apply(
            Action::Download {
                manual: true,
                card: None,
                chat: "1@s.whatsapp.net".into(),
                message: "m2".into(),
            },
            &ctx,
        );
        assert_eq!(
            app.viewer_media_state.get("m2"),
            Some(&MediaState::Downloading),
            "the viewer shows the download it started"
        );

        events
            .send(Event::Media {
                card: None,
                chat: "1@s.whatsapp.net".into(),
                message: "m2".into(),
                result: Err("403 Forbidden".into()),
            })
            .unwrap();
        app.handle_events();
        assert_eq!(
            app.viewer_media_state.get("m2"),
            Some(&MediaState::Failed(
                "No longer available on WhatsApp's servers".into()
            )),
            "and why it failed"
        );

        events
            .send(Event::Media {
                card: None,
                chat: "1@s.whatsapp.net".into(),
                message: "m2".into(),
                result: Ok(PathBuf::from("/fixture/2.png")),
            })
            .unwrap();
        app.handle_events();
        assert_eq!(
            app.viewer_media_state.get("m2"),
            None,
            "the file landing ends the progress"
        );
    }

    /// The album on screen follows the chat: a message deleted while the viewer
    /// is open leaves the strip, and the page is asked for again so a photo
    /// that arrives does not wait for the next open.
    #[test]
    fn the_album_follows_a_chat_while_the_viewer_is_open() {
        let ctx = egui::Context::default();
        let mut app = app_with_album(3);
        let (backend, mut commands, events) = Backend::recording_with_events();
        app.backend = backend;
        open_viewer(&mut app, 1);
        app.viewer_media_state
            .insert("m1".into(), MediaState::Downloading);

        events
            .send(Event::MessageDeleted {
                chat: "1@s.whatsapp.net".into(),
                id: "m1".into(),
            })
            .unwrap();
        app.handle_events();
        let ids: Vec<&str> = app
            .viewer_media
            .iter()
            .map(|item| item.id.as_str())
            .collect();
        assert_eq!(ids, ["m0", "m2"], "the deleted message left the album");
        assert_eq!(
            app.viewer_media_state.get("m1"),
            None,
            "and its download state"
        );
        assert!(
            matches!(
                commands.try_recv(),
                Ok(Command::LoadChatMedia { ref around, .. }) if around.as_deref() == Some("m1")
            ),
            "the page is asked for again, anchored on what is on screen"
        );

        // A message arriving in the same chat asks for it again too.
        events
            .send(Event::Messages {
                chat: "1@s.whatsapp.net".into(),
                messages: Vec::new(),
                older: false,
                complete: true,
            })
            .unwrap();
        app.handle_events();
        assert!(
            matches!(commands.try_recv(), Ok(Command::LoadChatMedia { .. })),
            "a message that arrives refreshes the album"
        );

        // And a chat the viewer is not showing is left alone.
        let ctx = ctx;
        app.apply(Action::CloseImagePreview, &ctx);
        events
            .send(Event::Messages {
                chat: "1@s.whatsapp.net".into(),
                messages: Vec::new(),
                older: false,
                complete: true,
            })
            .unwrap();
        app.handle_events();
        assert!(
            commands.try_recv().is_err(),
            "no album is asked for once the viewer is closed"
        );
    }

    #[test]
    fn stepping_does_nothing_without_an_album() {
        let mut app = app();
        app.chats
            .push(Chat::new("1@s.whatsapp.net".into(), "Ada".into()));
        let ctx = egui::Context::default();
        open_viewer(&mut app, 0);
        app.apply(Action::ViewerStep(1), &ctx);
        assert_eq!(
            app.image_preview.as_ref().unwrap().message(),
            "m0",
            "a picture opened on its own has nowhere to step"
        );
    }

    #[test]
    fn a_thumbnail_click_shows_that_item() {
        let mut app = app_with_album(3);
        let ctx = egui::Context::default();
        open_viewer(&mut app, 0);
        app.apply(
            Action::ViewImage {
                card: None,
                message: "m2".into(),
            },
            &ctx,
        );
        let preview = app.image_preview.as_ref().unwrap();
        assert_eq!(preview.message(), "m2");
        assert_eq!(preview.path(), Some(std::path::Path::new("/fixture/2.png")));
        assert!(preview.is_fit(), "the picture opens fitted");
        // An id the album does not hold leaves the viewer where it was.
        app.apply(
            Action::ViewImage {
                card: None,
                message: "nope".into(),
            },
            &ctx,
        );
        assert_eq!(app.image_preview.as_ref().unwrap().message(), "m2");
    }

    #[test]
    fn closing_the_viewer_forgets_the_album() {
        let mut app = app_with_album(3);
        let ctx = egui::Context::default();
        open_viewer(&mut app, 0);
        app.apply(Action::CloseImagePreview, &ctx);
        assert!(app.image_preview.is_none());
        assert!(app.viewer_media.is_empty());
        assert!(app.viewer_media_chat.is_none());
    }

    #[test]
    fn an_album_answer_for_another_chat_is_dropped() {
        let root = std::env::temp_dir().join(format!("zapfast-album-{}", std::process::id()));
        let (mut app, events) = App::headless(AppDirs::under(&root), Settings::default());
        app.viewer_media_chat = Some("1@s.whatsapp.net".into());
        // An album for a chat the viewer has already left arrives too late.
        events
            .send(Event::ChatMedia {
                chat: "2@s.whatsapp.net".into(),
                around: None,
                items: Vec::new(),
            })
            .unwrap();
        app.handle_events();
        assert!(app.viewer_media_chat.as_deref() == Some("1@s.whatsapp.net"));
        // Its own album lands.
        events
            .send(Event::ChatMedia {
                chat: "1@s.whatsapp.net".into(),
                around: None,
                items: Vec::new(),
            })
            .unwrap();
        app.handle_events();
        assert!(app.viewer_media.is_empty());
    }

    /// The viewer belongs to the window. Another linked account may use the
    /// same chat and message ids, and nothing it sends while it is not on
    /// screen reaches the viewer: not its album, not a download, not a
    /// deletion, and it asks for no album either.
    #[test]
    fn a_hidden_accounts_album_never_reaches_the_viewer() {
        let directory = tempfile::tempdir().unwrap();
        let dirs = AppDirs::under(directory.path());
        let (mut app, _) = App::headless(dirs.clone(), Settings::default());
        let (visible, mut visible_commands) = Backend::recording();
        app.backend = visible;
        let mut second = Account::detached(
            &dirs,
            AccountId::parse("2").unwrap(),
            crate::settings::AccountSettings::default(),
        )
        .unwrap()
        .0;
        let (hidden, mut hidden_commands, hidden_events) = Backend::recording_with_events();
        second.backend = hidden;
        second.link = LinkStatus::Connected;
        app.accounts.push(second);
        app.active = 0;

        let chat = "1@s.whatsapp.net";
        let item = |id: &str| crate::model::MediaItem {
            card: None,
            id: id.into(),
            timestamp: 0,
            video: false,
            path: Some(PathBuf::from(format!("/fixture/{id}.png"))),
            thumbnail: None,
        };
        app.viewer_media = vec![item("m0"), item("m1")];
        app.viewer_media_chat = Some(chat.into());
        app.image_preview = Some(PreviewState::new(
            Some(PathBuf::from("/fixture/m0.png")),
            chat.into(),
            "m0".into(),
        ));
        for event in [
            Event::ChatMedia {
                chat: chat.into(),
                around: None,
                items: vec![item("other")],
            },
            Event::Media {
                card: None,
                chat: chat.into(),
                message: "m1".into(),
                result: Err("404".into()),
            },
            Event::MessageDeleted {
                chat: chat.into(),
                id: "m1".into(),
            },
        ] {
            hidden_events.send(event).unwrap();
        }
        app.handle_events();

        assert_eq!(app.active, 0, "the first account stays on screen");
        let ids: Vec<_> = app
            .viewer_media
            .iter()
            .map(|item| item.id.as_str())
            .collect();
        assert_eq!(ids, ["m0", "m1"], "the album on screen is untouched");
        assert!(
            app.viewer_media_state.is_empty(),
            "another account's failed download is not the viewer's"
        );
        assert_eq!(
            app.image_preview.as_ref().map(PreviewState::message),
            Some("m0")
        );
        assert!(
            visible_commands.try_recv().is_err() && hidden_commands.try_recv().is_err(),
            "no album is asked for on either account"
        );
    }

    #[test]
    fn a_video_in_the_viewer_keeps_clipboard_and_control_focus() {
        let (mut app, ctx) = clipboard_app();
        app.image_preview = Some(PreviewState::new_clip(
            PathBuf::from("/cache/clip.mp4"),
            "fixture@s.whatsapp.net".into(),
            "m1".into(),
        ));
        let control = egui::Id::new("fixture-video-control");
        ctx.memory_mut(|memory| memory.request_focus(control));
        let mut reads = 0;
        let mut output = ctx.run_ui(
            egui::RawInput {
                events: vec![paste_release()],
                ..Default::default()
            },
            |ui| {
                ui.interact(
                    egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(40.0, 40.0)),
                    control,
                    egui::Sense::click(),
                );
                app.take_clipboard_paste(ui.ctx(), || {
                    reads += 1;
                    Some(ClipboardPaste::Image {
                        width: 2,
                        height: 2,
                        rgba: vec![200; 16],
                    })
                });
            },
        );
        output.textures_delta.clear();
        assert_eq!(reads, 0);
        assert!(app.pending.is_empty());
        assert!(ctx.memory(|memory| memory.has_focus(control)));
    }

    #[test]
    fn image_preview_opens_zooms_fits_and_closes() {
        let ctx = egui::Context::default();
        let mut app = app();
        let file = tempfile::NamedTempFile::with_suffix(".png").unwrap();
        std::fs::write(file.path(), b"not a real image").unwrap();

        app.apply(
            Action::PreviewImage {
                card: None,
                path: file.path().to_owned(),
                chat: "1@s.whatsapp.net".into(),
                message: "m1".into(),
            },
            &ctx,
        );
        let preview = app.image_preview.as_ref().expect("preview opens");
        assert_eq!(preview.path(), Some(file.path()));
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
    fn the_album_holds_photos_clips_and_card_pictures_in_chat_order() {
        let ctx = egui::Context::default();
        let mut app = app();
        let dir = tempfile::tempdir().unwrap();
        let paths: Vec<_> = [
            "first.png",
            "middle.jpg",
            "last.webp",
            "sticker.png",
            "other.png",
        ]
        .iter()
        .map(|name| dir.path().join(name))
        .collect();
        for path in &paths {
            std::fs::write(path, b"fixture").unwrap();
        }
        let media = |path: Option<PathBuf>| Media {
            album: None,
            mime: "image/png".into(),
            size: 7,
            width: None,
            height: None,
            path,
            state: MediaState::Idle,
        };
        let mut rows = Vec::new();
        for (index, path) in paths[..3].iter().enumerate() {
            if index == 2 {
                rows.push(message("chat", "text-between-photos", 3));
            }
            let mut row = message("chat", &format!("photo-{index}"), index as i64 * 2);
            row.content = Content::Image {
                motion: None,
                caption: None,
                media: media(Some(path.clone())),
            };
            rows.push(row);
        }
        let mut sticker = message("chat", "sticker", 6);
        sticker.content = Content::Sticker {
            media: media(Some(paths[3].clone())),
            animated: false,
        };
        rows.push(sticker);
        let mut pending = message("chat", "pending", 7);
        pending.content = Content::Image {
            motion: None,
            caption: None,
            media: media(None),
        };
        rows.push(pending);
        let mut video = message("chat", "video", 8);
        video.content = Content::Video {
            caption: None,
            media: media(Some(paths[3].clone())),
            seconds: Some(1),
            gif: false,
            note: false,
        };
        rows.push(video);
        let mut card = message("chat", "card", 9);
        card.content = Content::Interactive {
            text: String::new(),
            card: Some(Box::new(crate::model::InteractiveCard {
                carousel: vec![crate::model::InteractiveCard {
                    image: Some(media(Some(paths[0].clone()))),
                    ..Default::default()
                }],
                ..Default::default()
            })),
        };
        rows.push(card);
        app.conversations.insert(
            "chat".into(),
            Conversation {
                messages: rows,
                ..Default::default()
            },
        );
        let mut other = message("other", "photo", 0);
        other.content = Content::Image {
            motion: None,
            caption: None,
            media: media(Some(paths[4].clone())),
        };
        app.conversations.insert(
            "other".into(),
            Conversation {
                messages: vec![other],
                ..Default::default()
            },
        );
        app.open_chat = Some("chat".into());
        app.chats.push(Chat::new("chat".into(), "Fixture".into()));
        // The album the archive would list for this chat, from the same rows.
        app.viewer_media = app.conversations["chat"]
            .messages
            .iter()
            .flat_map(crate::model::MediaItem::of_message)
            .collect();
        app.viewer_media_chat = Some("chat".into());
        let items: Vec<_> = app
            .viewer_media
            .iter()
            .map(|item| (item.id.as_str(), item.card))
            .collect();
        assert_eq!(
            items,
            [
                ("photo-0", None),
                ("photo-1", None),
                ("photo-2", None),
                ("pending", None),
                ("video", None),
                ("card", Some(0)),
            ],
            "photos, a clip and a card's picture, in chat order; no sticker, no text"
        );
        let album = app.viewer_media.clone();
        app.apply(
            Action::PreviewImage {
                path: paths[1].clone(),
                chat: "chat".into(),
                message: "photo-1".into(),
                card: None,
            },
            &ctx,
        );
        // As the worker's answer would bring it.
        app.viewer_media = album;
        assert_eq!(
            app.image_preview.as_ref().unwrap().path(),
            Some(paths[1].as_path())
        );
        app.apply(Action::ZoomImageIn, &ctx);
        app.apply(Action::ViewerStep(1), &ctx);
        let preview = app.image_preview.as_ref().unwrap();
        assert_eq!(preview.path(), Some(paths[2].as_path()));
        assert!(preview.is_fit());
        assert_eq!(preview.zoom(), 1.0);
        // The photo not downloaded yet is shown for what it is.
        app.apply(Action::ViewerStep(1), &ctx);
        let preview = app.image_preview.as_ref().unwrap();
        assert_eq!(preview.message(), "pending");
        assert_eq!(preview.path(), None);
        app.apply(Action::ViewerStep(1), &ctx);
        app.apply(Action::ViewerStep(1), &ctx);
        let preview = app.image_preview.as_ref().unwrap();
        assert_eq!((preview.message(), preview.card()), ("card", Some(0)));
        assert_eq!(preview.path(), Some(paths[0].as_path()));
        app.apply(Action::ViewerStep(1), &ctx);
        assert_eq!(
            app.image_preview.as_ref().unwrap().card(),
            Some(0),
            "the album ends there"
        );
        // A photo of another chat opens over that chat's album instead.
        app.apply(
            Action::PreviewImage {
                path: paths[4].clone(),
                chat: "other".into(),
                message: "photo".into(),
                card: None,
            },
            &ctx,
        );
        assert_eq!(
            app.image_preview.as_ref().unwrap().path(),
            Some(paths[4].as_path())
        );
    }

    #[test]
    fn image_navigation_revokes_access_when_a_conversation_is_locked() {
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir().unwrap();
        let paths = [dir.path().join("first.png"), dir.path().join("second.png")];
        for path in &paths {
            std::fs::write(path, b"fixture").unwrap();
        }
        for remote_lock in [false, true] {
            let mut app = app();
            let mut chat = Chat::new("fixture".into(), "Fixture".into());
            chat.locked = !remote_lock;
            app.chats.push(chat.clone());
            if !remote_lock {
                app.settings.set_chat_lock_code(Some("fixture-code"));
                app.enter_locked_folder();
            }
            app.open_chat = Some(chat.id.clone());
            let rows = paths
                .iter()
                .enumerate()
                .map(|(index, path)| {
                    let mut row = message(&chat.id, &format!("photo-{index}"), index as i64);
                    row.content = Content::Image {
                        motion: None,
                        caption: None,
                        media: Media {
                            album: None,
                            mime: "image/png".into(),
                            size: 7,
                            width: None,
                            height: None,
                            path: Some(path.clone()),
                            state: MediaState::Idle,
                        },
                    };
                    row
                })
                .collect();
            app.conversations.insert(
                chat.id.clone(),
                Conversation {
                    messages: rows,
                    ..Default::default()
                },
            );
            app.viewer_media = app.conversations[&chat.id]
                .messages
                .iter()
                .flat_map(crate::model::MediaItem::of_message)
                .collect();
            app.viewer_media_chat = Some(chat.id.clone());
            let album = app.viewer_media.clone();
            app.apply(
                Action::PreviewImage {
                    path: paths[0].clone(),
                    chat: chat.id.clone(),
                    message: "photo-0".into(),
                    card: None,
                },
                &ctx,
            );
            app.viewer_media = album;
            app.apply(Action::ViewerStep(1), &ctx);
            assert_eq!(
                app.image_preview.as_ref().unwrap().path(),
                Some(paths[1].as_path())
            );
            app.apply(Action::ViewerStep(-1), &ctx);
            assert_eq!(
                app.image_preview.as_ref().unwrap().path(),
                Some(paths[0].as_path())
            );
            let retained_preview = app.image_preview.clone();
            if remote_lock {
                chat.locked = true;
                app.apply_backend_event(Event::ChatUpdated(Box::new(chat)), true);
            } else {
                app.apply(Action::CloseLockedFolder, &ctx);
            }
            assert!(app.image_preview.is_none());
            assert!(app.open_chat.is_none());
            // Even a retained viewer cannot step through the chat's album.
            for step in [-1, 1] {
                app.image_preview = retained_preview.clone();
                app.apply(Action::ViewerStep(step), &ctx);
                assert!(app.image_preview.is_none());
            }
        }
    }

    #[test]
    fn chat_snapshots_reconcile_image_preview_access_only_for_the_visible_account() {
        let ctx = egui::Context::default();
        for present in [false, true] {
            for locked in [false, true] {
                for authorized in [false, true] {
                    for hidden in [false, true] {
                        let mut app = app();
                        if authorized {
                            app.settings.set_chat_lock_code(Some("fixture-code"));
                            app.enter_locked_folder();
                        }
                        let preview = PreviewState::new(
                            Some("fixture.png".into()),
                            "fixture".into(),
                            "photo".into(),
                        );
                        app.image_preview = Some(preview.clone());
                        // Preview access depends on its captured chat, even if no chat is open.
                        app.open_chat = None;
                        let mut chat = Chat::new("fixture".into(), "Fixture".into());
                        chat.locked = locked;
                        let chats = if present { vec![chat] } else { Vec::new() };
                        app.events_hidden = hidden;
                        app.apply_backend_event(Event::Chats(chats), true);
                        let accessible = present && (!locked || authorized);
                        assert_eq!(app.image_preview.is_some(), hidden || accessible);
                        if !hidden {
                            for step in [-1, 1] {
                                app.image_preview = Some(preview.clone());
                                app.apply(Action::ViewerStep(step), &ctx);
                                assert_eq!(app.image_preview.is_some(), accessible);
                            }
                        }
                    }
                }
            }
        }
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

    /// On Linux the tray item exists before a panel shows it (ZapFast started
    /// at login before the panel, or a desktop without one). Until a panel
    /// shows it, closing quits and the window is not hidden, since nothing
    /// could bring it back. A hidden start waits for a panel instead; see
    /// `a_hidden_start_waits_for_a_panel_to_show_the_tray`.
    #[test]
    fn a_tray_no_panel_shows_does_not_keep_the_app_running() {
        use fastframe_shell::Resident;
        let ctx = egui::Context::default();
        let mut app = app();
        assert!(app.settings.keep_running_in_background);
        app.test_tray_shown = Some(false);
        assert!(!app.hides_to_tray());
        app.apply(Action::HideWindow, &ctx);
        assert!(!app.hide_intent);

        app.test_tray_shown = Some(true);
        assert!(app.hides_to_tray());
        app.apply(Action::HideWindow, &ctx);
        assert!(app.hide_intent);
        app.hide_intent = false;
        assert!(Resident::start_hidden(&mut app));
        assert!(app.hide_intent);

        app.settings.keep_running_in_background = false;
        assert!(!app.hides_to_tray(), "the setting still decides");
        assert!(!Resident::start_hidden(&mut app));
    }

    /// At login on GNOME the tray watcher comes up after ZapFast (#429): a
    /// hidden start keeps the window hidden while it waits for a panel, stays
    /// hidden once one shows the item, and shows the window if none has
    /// within `TRAY_WAIT`.
    #[test]
    fn a_hidden_start_waits_for_a_panel_to_show_the_tray() {
        use fastframe_shell::{Headless, Resident};
        let ctx = egui::Context::default();
        let mut app = app();
        app.test_tray_shown = Some(false);
        assert!(Resident::start_hidden(&mut app), "waits for a panel");
        assert!(app.hide_intent);
        let started = Instant::now();
        app.check_tray_wait(started + Duration::from_secs(5));
        assert!(!app.wants_show, "still waiting");
        assert_eq!(app.headless_frame(&ctx), Headless::Wait);
        app.test_tray_shown = Some(true);
        app.check_tray_wait(started + Duration::from_secs(10));
        assert!(app.tray_wait.is_none(), "a panel showed it");
        app.test_tray_shown = Some(false);
        app.check_tray_wait(started + TRAY_WAIT * 2);
        assert!(!app.wants_show, "the panel came; nothing more to wait for");

        let mut app = self::app();
        app.test_tray_shown = Some(false);
        assert!(Resident::start_hidden(&mut app));
        app.check_tray_wait(Instant::now() + TRAY_WAIT + Duration::from_secs(1));
        assert!(app.wants_show, "no panel came: the window shows");
        assert_eq!(app.headless_frame(&ctx), Headless::Show);
    }

    /// eframe runs only the logic pass for a minimized window, so closing one
    /// (from the taskbar, say) must still hide it to the tray rather than
    /// quit, and quit when nothing could bring it back.
    #[test]
    fn closing_a_minimized_window_hides_it_to_the_tray() {
        use fastframe_shell::{Closed, Resident};
        let ctx = egui::Context::default();
        let close_minimized = |app: &mut App| {
            let mut input = egui::RawInput::default();
            let info = input.viewports.entry(egui::ViewportId::ROOT).or_default();
            info.minimized = Some(true);
            info.events.push(egui::ViewportEvent::Close);
            let mut output = ctx.run_ui(input, |ui| app.window_logic(ui.ctx()));
            output.textures_delta.clear();
        };

        let mut app = app();
        app.test_tray_shown = Some(true);
        close_minimized(&mut app);
        assert!(app.hide_intent);
        assert_eq!(app.closed(), Closed::Hide);

        let mut app = self::app();
        app.test_tray_shown = Some(false);
        close_minimized(&mut app);
        assert_eq!(app.closed(), Closed::Quit, "no tray, no way back");

        let mut app = self::app();
        app.test_tray_shown = Some(true);
        app.quit_requested = true;
        close_minimized(&mut app);
        assert_eq!(app.closed(), Closed::Quit);
    }

    /// A Wayland compositor ignores an app's focus and unminimize requests, so
    /// a notification click or a second launch closes the window and asks the
    /// shell for a fresh one, which the compositor raises, unless the
    /// compositor (niri) can raise the existing one. Elsewhere the window is
    /// only told to restore and focus.
    #[test]
    fn a_wayland_show_reopens_the_window_instead_of_focusing_it() {
        use fastframe_shell::{Closed, Resident};
        let ctx = egui::Context::default();

        let mut app = app();
        app.wayland = true;
        app.apply(Action::ShowWindow, &ctx);
        assert!(app.reopen, "Wayland closes the window to make a fresh one");
        assert_eq!(app.closed(), Closed::Reopen);

        // niri brings the existing window forward, on its own workspace.
        let mut app = self::app();
        app.wayland = true;
        app.test_raised = true;
        app.apply(Action::ShowWindow, &ctx);
        assert!(!app.reopen, "a raised window is not reopened");
        assert_ne!(app.closed(), Closed::Reopen);

        let mut app = self::app();
        app.wayland = false;
        app.apply(Action::ShowWindow, &ctx);
        assert!(!app.reopen, "elsewhere the window is only focused");
        assert_ne!(app.closed(), Closed::Reopen);
    }

    /// The title bar choice is saved and, because decorations are fixed when
    /// a window is made, replaces the window on Windows and Linux. macOS has
    /// no such choice: its traffic lights stay.
    #[test]
    fn choosing_the_custom_title_bar_replaces_the_window() {
        use fastframe_shell::{Closed, Resident};
        let ctx = egui::Context::default();
        let mut app = app();
        assert!(!app.settings.custom_titlebar, "off by default");
        app.apply(Action::SetCustomTitlebar(true), &ctx);
        assert!(app.settings.custom_titlebar);
        let replaced = cfg!(any(windows, target_os = "linux"));
        assert_eq!(app.reopen, replaced);
        assert_eq!(app.closed() == Closed::Reopen, replaced);

        app.reopen = false;
        app.apply(Action::SetCustomTitlebar(true), &ctx);
        assert!(!app.reopen, "an unchanged choice keeps the window");

        // A hidden app has no window to replace; the next one follows.
        app.window_hidden = true;
        app.apply(Action::SetCustomTitlebar(false), &ctx);
        assert!(!app.settings.custom_titlebar);
        assert!(!app.reopen);
    }

    #[test]
    fn unsupported_media_falls_back_to_the_external_opener() {
        let ctx = egui::Context::default();
        let mut app = app();
        let file = tempfile::NamedTempFile::with_suffix(".heic").unwrap();
        std::fs::write(file.path(), b"not a real image").unwrap();

        app.apply(
            Action::PreviewImage {
                card: None,
                path: file.path().to_owned(),
                chat: "1@s.whatsapp.net".into(),
                message: "m1".into(),
            },
            &ctx,
        );

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
    fn archive_request_keeps_the_confirmed_state_until_whatsapp_answers() {
        for archived in [false, true] {
            let mut app = app();
            let (backend, mut commands, events) = Backend::recording_with_events();
            app.backend = backend;
            let id = "15550002222@s.whatsapp.net".to_owned();
            let mut chat = Chat::new(id.clone(), "Fixture".into());
            chat.archived = !archived;
            app.chats.push(chat.clone());
            app.open_chat = Some(id.clone());
            let ctx = egui::Context::default();

            app.apply(Action::SetArchived(id.clone(), archived), &ctx);
            app.apply_actions(&ctx);

            assert_eq!(app.chat(&id).unwrap().archived, !archived);
            assert_eq!(app.open_chat.as_deref(), Some(id.as_str()));
            assert!(
                matches!(commands.try_recv(), Ok(Command::SetArchived(chat, state)) if chat == id && state == archived)
            );
            events
                .send(Event::ArchiveFailed {
                    archived,
                    failure: crate::backend::ArchiveFailure::Sync,
                })
                .unwrap();
            app.handle_events();
            assert_eq!(app.chat(&id).unwrap().archived, !archived);
            assert_eq!(app.open_chat.as_deref(), Some(id.as_str()));
            assert!(
                app.toasts
                    .iter()
                    .any(|toast| toast.message.contains("Try again"))
            );

            chat.archived = archived;
            events.send(Event::ChatUpdated(Box::new(chat))).unwrap();
            app.handle_events();
            app.apply_actions(&ctx);
            assert_eq!(app.chat(&id).unwrap().archived, archived);
            assert_eq!(app.open_chat.is_none(), archived);
        }
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
        app.handle_chat_updated(confirmed, true);
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
        app.handle_chat_updated(chat, true);
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
        app.handle_chat_updated(confirmed, true);
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

    /// The chip or label the list showed comes back at the next start, with
    /// its account (#380).
    #[test]
    fn the_list_filter_survives_a_restart() {
        let ctx = egui::Context::default();
        let restart = |settings: crate::settings::AccountSettings| {
            let root = tempfile::tempdir().unwrap();
            let dirs = AppDirs::under(root.path());
            let (account, _events) =
                Account::detached(&dirs, AccountId::first(), settings).unwrap();
            let roster = AccountRoster {
                active: account.id.0.clone(),
                ..AccountRoster::default()
            };
            let app = App::with_accounts(
                dirs,
                Settings::default(),
                roster,
                vec![account],
                0,
                Waker::default(),
            );
            (app, root)
        };
        let mut app = app();
        app.apply(Action::SetChatFilter(ChatFilter::Groups), &ctx);
        assert_eq!(
            app.account().settings.list_filter.as_deref(),
            Some("groups")
        );
        let (again, _root) = restart(app.account().settings.clone());
        assert_eq!(again.chat_filter, ChatFilter::Groups);
        assert!(again.label_filter.is_none());

        app.labels = vec![label("label-1", "Work")];
        app.apply(Action::SelectLabel(Some("label-1".into())), &ctx);
        let (mut again, _root) = restart(app.account().settings.clone());
        assert_eq!(again.chat_filter, ChatFilter::All);
        assert_eq!(again.label_filter.as_deref(), Some("label-1"));

        // The label is deleted elsewhere: the list shows every chat, now
        // and at the next start.
        again.apply_backend_event(Event::Labels(Vec::new()), true);
        assert!(again.label_filter.is_none());
        assert_eq!(again.account().settings.list_filter, None);

        app.apply(Action::SetChatFilter(ChatFilter::All), &ctx);
        assert_eq!(app.account().settings.list_filter, None);
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
        assert!(matches!(
            super::tray_action(Event::Menu(super::TRAY_LOCK), false),
            Some(Action::LockApp)
        ));
        for lockable in [false, true] {
            assert_eq!(
                super::tray_config(lockable).menu,
                [
                    fastframe_tray::MenuItem::action(super::TRAY_SHOW, "Show or hide ZapFast"),
                    fastframe_tray::MenuItem::action(super::TRAY_LOCK, "Lock ZapFast")
                        .visible(lockable),
                    fastframe_tray::MenuItem::Separator,
                    fastframe_tray::MenuItem::action(super::TRAY_QUIT, "Quit"),
                ]
            );
        }
    }

    /// The tray offers "Lock ZapFast" as soon as a password is set and stops
    /// as soon as it is removed, not at the next start.
    #[test]
    fn the_tray_offers_the_lock_while_a_password_is_set() {
        let mut app = app();
        assert!(!app.tray_lockable);
        app.settings.app_lock_hash = Some(crate::app_lock::verifier("secret"));
        app.handle_tray();
        assert!(app.tray_lockable);
        app.settings.app_lock_hash = None;
        app.handle_tray();
        assert!(!app.tray_lockable);
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

    /// winit reports no system theme on Linux, so "Follow system" takes the
    /// desktop portal's preference, and a fixed choice ignores it.
    #[cfg(target_os = "linux")]
    #[test]
    fn follow_system_uses_the_desktops_preference_on_linux() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.settings.theme = ThemeChoice::System;
        assert!(app.settings.cached_palette().is_none());
        app.system_appearance = Some(crate::appearance::SystemAppearance::fixed(false));
        app.apply_theme(&ctx);
        assert_eq!(ctx.theme(), egui::Theme::Light);
        assert_eq!(app.palette, Palette::light());

        app.system_appearance = Some(crate::appearance::SystemAppearance::fixed(true));
        app.apply_theme(&ctx);
        assert_eq!(ctx.theme(), egui::Theme::Dark);
        assert_eq!(app.palette, Palette::dark());

        app.settings.theme = ThemeChoice::Light;
        app.apply_theme(&ctx);
        assert_eq!(ctx.theme(), egui::Theme::Light, "a chosen theme wins");
        assert_eq!(app.palette, Palette::light());
    }

    /// A change of colours keeps the old palette while the window's picture
    /// of it is on its way, and applies the new one once it arrives or after
    /// a short wait without it. The window's first palette is not revealed.
    #[test]
    fn a_theme_change_holds_the_old_colours_until_its_reveal_can_start() {
        let mut app = app();
        app.reveal_theme_changes = true;
        app.settings.theme = ThemeChoice::Dark;
        app.settings.custom_theme = None;
        let ctx = egui::Context::default();
        let at = |app: &mut App, time: f64| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    time: Some(time),
                    ..Default::default()
                },
                |ui| app.apply_theme(ui.ctx()),
            );
            output.textures_delta.clear();
        };
        at(&mut app, 0.0);
        assert_eq!(
            app.palette,
            Palette::dark(),
            "the first palette applies at once"
        );

        app.settings.theme = ThemeChoice::Light;
        at(&mut app, 1.0);
        assert_eq!(
            app.palette,
            Palette::dark(),
            "held for the window's picture"
        );
        at(&mut app, 1.1);
        assert_eq!(app.palette, Palette::dark());
        at(&mut app, 1.5);
        assert_eq!(
            app.palette,
            Palette::light(),
            "no picture came: applied anyway"
        );
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
        app.handle_chat_updated(chat.clone(), true);
        assert_eq!(app.chat(&chat.id).unwrap().unread, 2);
        // Focus left over from a window callback is insufficient while hidden.
        app.window_focused = true;
        app.handle_chat_updated(chat.clone(), true);
        assert_eq!(app.chat(&chat.id).unwrap().unread, 2);
        app.window_hidden = false;
        app.handle_chat_updated(chat.clone(), true);
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
    /// Checks a selected message batch is forwarded in its original conversation order.
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
        app.chats
            .push(Chat::new("2@s.whatsapp.net".into(), "Grace".into()));
        app.apply(
            Action::ShowDialog(Dialog::Forward {
                chat: chat.into(),
                messages: vec!["first".into(), "third".into()],
            }),
            &ctx,
        );
        app.apply(
            Action::ToggleForwardRecipient("2@s.whatsapp.net".into()),
            &ctx,
        );
        app.apply(Action::ReviewForward, &ctx);
        app.apply(Action::ConfirmForward, &ctx);
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
        // Unselecting the last message keeps selecting, as WhatsApp Web
        // does, and the chat menu opens a selection with nothing in it.
        app.apply(Action::SelectMessage("second".into()), &ctx);
        app.apply(Action::ToggleSelected("second".into()), &ctx);
        assert_eq!(app.selection, Some((chat.into(), Vec::new())));
        app.apply(Action::CancelSelection, &ctx);
        app.apply(Action::StartSelection, &ctx);
        assert_eq!(app.selection, Some((chat.into(), Vec::new())));
        app.apply(Action::ToggleSelected("fifth".into()), &ctx);
        assert_eq!(app.selection, Some((chat.into(), vec!["fifth".into()])));
        // Choosing it again keeps what is ticked, and a deleted message
        // cannot be ticked.
        app.apply(Action::StartSelection, &ctx);
        app.apply(Action::ToggleSelected("gone".into()), &ctx);
        assert_eq!(app.selection, Some((chat.into(), vec!["fifth".into()])));
    }

    #[test]
    fn selection_drops_messages_that_become_ineligible() {
        let mut app = app();
        let (backend, events) = Backend::detached();
        app.backend = backend;
        let ctx = egui::Context::default();
        let chat = "1@s.whatsapp.net";
        app.open_chat = Some(chat.into());
        app.conversations.entry(chat.into()).or_default().merge(
            vec![message(chat, "first", 1), message(chat, "second", 2)],
            false,
        );
        app.apply(Action::SelectMessage("first".into()), &ctx);
        app.apply(
            Action::SweepMessages {
                anchor: "first".into(),
                to: "second".into(),
            },
            &ctx,
        );
        let mut revoked = message(chat, "first", 1);
        revoked.content = Content::Revoked;
        events
            .send(Event::MessageUpdated(Box::new(revoked)))
            .unwrap();
        app.handle_events();
        assert_eq!(app.selection, Some((chat.into(), vec!["second".into()])));
        assert_eq!(app.selection_anchor.as_deref(), Some("second"));
        assert!(app.sweep.as_ref().unwrap().base.is_empty());
        app.apply(
            Action::SweepMessages {
                anchor: "first".into(),
                to: "second".into(),
            },
            &ctx,
        );
        assert_eq!(app.selection, Some((chat.into(), vec!["second".into()])));

        app.apply(
            Action::DeleteForEveryone {
                chat: chat.into(),
                id: "second".into(),
            },
            &ctx,
        );
        assert_eq!(app.selection, Some((chat.into(), Vec::new())));

        // Delete for me keeps the row until the deletion is accepted, then
        // drops it from the selection along with the message.
        app.conversations
            .get_mut(chat)
            .unwrap()
            .merge(vec![message(chat, "kept", 3)], false);
        app.apply(Action::SelectMessage("kept".into()), &ctx);
        events
            .send(Event::MessageDeleted {
                chat: chat.to_owned(),
                id: "kept".into(),
            })
            .unwrap();
        app.handle_events();
        assert_eq!(app.selection, Some((chat.into(), Vec::new())));
        assert!(app.conversations[chat].message("kept").is_none());
    }

    #[test]
    fn selection_rejects_messages_that_cannot_be_forwarded() {
        let mut app = app();
        let ctx = egui::Context::default();
        let chat = "1@s.whatsapp.net";
        app.open_chat = Some(chat.into());
        let blocked = [
            Content::Revoked,
            Content::PhoneOnly {
                view_once: true,
                live_location: false,
                once: None,
            },
            Content::Unsupported {
                what: "Test".into(),
            },
            Content::Poll {
                question: "Lunch?".into(),
                options: vec!["Yes".into(), "No".into()],
                state: Default::default(),
            },
            Content::Interactive {
                text: "Choose an option".into(),
                card: None,
            },
        ];
        let mut messages = vec![message(chat, "first", 1)];
        for (index, content) in blocked.into_iter().enumerate() {
            let mut row = message(chat, &format!("blocked-{index}"), index as i64 + 2);
            row.content = content;
            messages.push(row);
        }
        messages.push(message(chat, "last", 10));
        app.conversations
            .entry(chat.into())
            .or_default()
            .merge(messages, false);
        for id in (0..5)
            .map(|index| format!("blocked-{index}"))
            .chain(std::iter::once("missing".into()))
        {
            app.apply(Action::SelectMessage(id.clone()), &ctx);
            assert!(app.selection.is_none(), "{id} cannot start a selection");
            assert!(app.selection_anchor.is_none());
            app.apply(Action::SelectMessage("first".into()), &ctx);
            app.apply(Action::SelectMessage(id.clone()), &ctx);
            app.apply(Action::ToggleSelected(id.clone()), &ctx);
            assert_eq!(
                app.selection,
                Some((chat.into(), vec!["first".into()])),
                "{id} cannot replace or join a selection"
            );
            assert_eq!(app.selection_anchor.as_deref(), Some("first"));
            app.apply(Action::CancelSelection, &ctx);
            app.selection_anchor = None;
        }
        app.apply(Action::SelectMessage("first".into()), &ctx);
        app.apply(Action::SelectRange("last".into()), &ctx);
        assert_eq!(
            app.selection,
            Some((chat.into(), vec!["first".into(), "last".into()]))
        );
        app.apply(Action::CancelSelection, &ctx);
        app.apply(
            Action::SweepMessages {
                anchor: "first".into(),
                to: "last".into(),
            },
            &ctx,
        );
        assert_eq!(
            app.selection,
            Some((chat.into(), vec!["first".into(), "last".into()]))
        );
    }

    /// #246: a sweep adds its range to what was selected when it began, in
    /// the chat's order, and shrinks again when dragged back.
    #[test]
    fn a_sweep_adds_its_range_to_the_selection_it_began_from() {
        let mut app = app();
        let ctx = egui::Context::default();
        let chat = "1@s.whatsapp.net";
        app.chats = vec![Chat::new(chat.into(), "Ada".into())];
        app.open_chat = Some(chat.into());
        let mut gone = message(chat, "gone", 3);
        gone.content = Content::Revoked;
        app.conversations.entry(chat.into()).or_default().merge(
            vec![
                message(chat, "first", 1),
                message(chat, "second", 2),
                gone,
                message(chat, "fourth", 4),
                message(chat, "fifth", 5),
            ],
            false,
        );
        let selected = |app: &App| app.selection.as_ref().map(|(_, ids)| ids.clone());
        let sweep = |app: &mut App, to: &str| {
            app.apply(
                Action::SweepMessages {
                    anchor: "fifth".into(),
                    to: to.into(),
                },
                &ctx,
            );
        };
        // Outside a selection, a sweep starts one.
        sweep(&mut app, "fifth");
        assert_eq!(selected(&app), Some(vec!["fifth".into()]));
        sweep(&mut app, "second");
        assert_eq!(
            selected(&app),
            Some(vec!["second".into(), "fourth".into(), "fifth".into()]),
            "what cannot be forwarded stays out"
        );
        sweep(&mut app, "fourth");
        assert_eq!(selected(&app), Some(vec!["fourth".into(), "fifth".into()]));
        app.apply(Action::EndSweep, &ctx);
        assert!(app.sweep.is_none());
        // A new sweep keeps what was selected before it.
        app.apply(Action::ToggleSelected("first".into()), &ctx);
        app.apply(
            Action::SweepMessages {
                anchor: "second".into(),
                to: "second".into(),
            },
            &ctx,
        );
        assert_eq!(
            selected(&app),
            Some(vec![
                "first".into(),
                "second".into(),
                "fourth".into(),
                "fifth".into()
            ])
        );
        // Escape ends both.
        app.apply(Action::CancelSelection, &ctx);
        assert!(app.selection.is_none() && app.sweep.is_none());
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

    /// Checks that incoming group mentions and quotes identify us across phone and privacy ids.
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
            labelled: None,
            chat: None,
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

        let mut everyone = plain.clone();
        everyone.mentions = vec![crate::mentions::everyone_ref()];
        assert!(
            app.addresses_us(&everyone),
            "mentioning everyone includes us"
        );
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

    /// Create sends the draft once; WhatsApp's answer opens the group and
    /// names who could not be added, a failure keeps the dialog for another
    /// try, and an answer nobody waits for opens nothing.
    #[test]
    fn a_created_group_opens_and_names_who_was_left_out() {
        use crate::new_group::{CreateFailure, Created, Draft, Stage};
        let mut app = app();
        let (backend, mut commands) = Backend::recording();
        app.backend = backend;
        let ctx = egui::Context::default();
        let ada = "15550000001@s.whatsapp.net";
        let grace = "15550000002@s.whatsapp.net";
        app.contacts.insert(
            grace.into(),
            Contact {
                id: grace.into(),
                full_name: Some("Grace Hopper".into()),
                ..Default::default()
            },
        );
        app.apply(Action::ShowDialog(Dialog::NewChat), &ctx);
        app.apply(Action::StartNewGroup, &ctx);
        app.apply(Action::ToggleNewGroupMember(ada.into()), &ctx);
        app.apply(Action::ToggleNewGroupMember(grace.into()), &ctx);
        app.apply(Action::CreateGroup, &ctx);
        assert!(commands.try_recv().is_err(), "not before the name");
        app.apply(Action::NewGroupNext, &ctx);
        app.new_group.as_mut().unwrap().name = " Book club ".into();
        app.apply(Action::CreateGroup, &ctx);
        app.apply(Action::CreateGroup, &ctx);
        let request = match commands.try_recv() {
            Ok(Command::CreateGroup {
                request,
                subject,
                members,
                picture: None,
            }) => {
                assert_eq!(subject, "Book club");
                assert_eq!(members, [ada, grace]);
                request
            }
            other => panic!("{other:?}"),
        };
        assert!(commands.try_recv().is_err(), "sent once");

        // A failure says why in plain words and leaves the name for a retry.
        app.apply_backend_event(
            Event::GroupCreated {
                request,
                result: Err(CreateFailure::RateLimited),
            },
            true,
        );
        let draft = app.new_group.as_ref().unwrap();
        assert!(!draft.creating);
        assert_eq!(draft.stage, Stage::Naming);
        assert!(
            app.toasts
                .iter()
                .any(|toast| toast.message.contains("too many groups")),
        );

        app.apply(Action::CreateGroup, &ctx);
        let Ok(Command::CreateGroup { request, .. }) = commands.try_recv() else {
            panic!("retried");
        };
        let chat = "120363000000000001@g.us";
        app.chats.push(Chat::new(chat.into(), "Book club".into()));
        // An earlier request's answer is not the one the window waits for.
        app.apply_backend_event(
            Event::GroupCreated {
                request: request.wrapping_sub(1),
                result: Ok(Created {
                    chat: chat.into(),
                    unadded: Vec::new(),
                }),
            },
            true,
        );
        assert!(app.new_group.is_some());
        app.apply_backend_event(
            Event::GroupCreated {
                request,
                result: Ok(Created {
                    chat: chat.into(),
                    unadded: vec![grace.into()],
                }),
            },
            true,
        );
        assert_eq!(app.dialog, None);
        assert!(app.new_group.is_none());
        assert!(
            app.actions
                .iter()
                .any(|action| matches!(action, Action::OpenChat(id) if id == chat))
        );
        assert!(app.toasts.iter().any(|toast| toast.message
            == "Grace Hopper could not be added to the group. Their privacy settings may not allow it."));

        // Back out of naming and choosing; letting the last one go leaves too.
        app.apply(Action::ShowDialog(Dialog::NewChat), &ctx);
        assert!(app.new_group.is_none(), "a new dialog starts fresh");
        app.new_group = Some(Draft::default());
        app.apply(Action::ToggleNewGroupMember(ada.into()), &ctx);
        app.apply(Action::ToggleNewGroupMember(ada.into()), &ctx);
        assert!(app.new_group.is_none());
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
    fn a_fourth_pinned_message_is_refused_here() {
        let mut app = app();
        let (backend, mut commands) = Backend::recording();
        app.backend = backend;
        let ctx = egui::Context::default();
        let chat = "1@s.whatsapp.net".to_owned();
        app.pins.insert(
            chat.clone(),
            ["a".to_owned(), "b".to_owned(), "c".to_owned()]
                .into_iter()
                .collect(),
        );
        app.chat_pins.insert(
            chat.clone(),
            vec![
                pinned_row(&chat, "a"),
                pinned_row(&chat, "b"),
                pinned_row(&chat, "c"),
            ],
        );
        app.apply(
            Action::SetMessagePinned {
                chat: chat.clone(),
                message: "d".into(),
                pinned: true,
            },
            &ctx,
        );
        assert!(
            !matches!(commands.try_recv(), Ok(Command::SetMessagePinned { .. })),
            "a fourth pin never reaches the phone"
        );
        assert!(
            app.toasts
                .iter()
                .any(|toast| toast.message.contains("three pinned messages")),
            "and the window says why"
        );
        app.apply(
            Action::SetMessagePinned {
                chat,
                message: "a".into(),
                pinned: false,
            },
            &ctx,
        );
        assert!(
            matches!(
                commands.try_recv(),
                Ok(Command::SetMessagePinned { pinned: false, .. })
            ),
            "unpinning always goes through"
        );
    }

    /// The banner reads a cache, so a delete, a clear or a lock has to take
    /// the pinned words off it: the archive join would no longer return them.
    #[test]
    fn a_deleted_message_takes_its_pin_off_the_banner() {
        let mut app = app();
        let (backend, events) = Backend::detached();
        app.backend = backend;
        let chat = "1@s.whatsapp.net";
        app.chats.push(Chat::new(chat.into(), "Fixture".into()));
        app.chat_pins
            .insert(chat.into(), vec![pinned_row(chat, "m1")]);
        app.pins
            .insert(chat.into(), ["m1".to_owned()].into_iter().collect());

        events
            .send(Event::MessageDeleted {
                chat: chat.into(),
                id: "m1".into(),
            })
            .unwrap();
        app.handle_events();

        assert!(app.chat_pins.get(chat).is_some_and(Vec::is_empty));
        assert!(app.pins.get(chat).is_some_and(HashSet::is_empty));
    }

    #[test]
    fn clearing_a_chat_drops_its_pins() {
        let mut app = app();
        let (backend, events) = Backend::detached();
        app.backend = backend;
        let chat = "1@s.whatsapp.net";
        app.chats.push(Chat::new(chat.into(), "Fixture".into()));
        app.chat_pins
            .insert(chat.into(), vec![pinned_row(chat, "m1")]);

        events
            .send(Event::ChatCleared {
                chat: chat.into(),
                through: 100,
            })
            .unwrap();
        app.handle_events();

        assert!(!app.chat_pins.contains_key(chat), "the pins went with it");
    }

    #[test]
    fn a_locked_chat_keeps_none_of_its_pins_on_screen() {
        let mut app = app();
        let (backend, events) = Backend::detached();
        app.backend = backend;
        let chat = "1@s.whatsapp.net";
        app.chats.push(Chat::new(chat.into(), "Fixture".into()));
        app.chat_pins
            .insert(chat.into(), vec![pinned_row(chat, "m1")]);
        app.pins
            .insert(chat.into(), ["m1".to_owned()].into_iter().collect());

        let mut locked = Chat::new(chat.into(), "Fixture".into());
        locked.locked = true;
        events.send(Event::ChatUpdated(Box::new(locked))).unwrap();
        app.handle_events();

        assert!(!app.chat_pins.contains_key(chat));
        assert!(!app.pins.contains_key(chat));
    }

    #[test]
    fn an_expired_pin_does_not_count_toward_the_cap() {
        let mut app = app();
        let (backend, mut commands) = Backend::recording();
        app.backend = backend;
        let chat = "1@s.whatsapp.net".to_owned();
        let mut rows = vec![
            pinned_row(&chat, "a"),
            pinned_row(&chat, "b"),
            pinned_row(&chat, "c"),
        ];
        for row in &mut rows {
            row.expires_at = 1;
        }
        app.pins.insert(
            chat.clone(),
            ["a".to_owned(), "b".to_owned(), "c".to_owned()]
                .into_iter()
                .collect(),
        );
        app.chat_pins.insert(chat.clone(), rows);
        app.apply(
            Action::SetMessagePinned {
                chat,
                message: "d".into(),
                pinned: true,
            },
            &egui::Context::default(),
        );
        assert!(
            matches!(
                commands.try_recv(),
                Ok(Command::SetMessagePinned { pinned: true, .. })
            ),
            "expired rows leave a free slot"
        );
    }

    #[test]
    fn a_revoked_message_leaves_the_pin_banner() {
        let mut app = app();
        let (backend, events) = Backend::detached();
        app.backend = backend;
        let chat = "1@s.whatsapp.net";
        app.chats.push(Chat::new(chat.into(), "Fixture".into()));
        app.chat_pins
            .insert(chat.into(), vec![pinned_row(chat, "m1")]);
        app.pins
            .insert(chat.into(), ["m1".to_owned()].into_iter().collect());
        let mut message = message(chat, "m1", 1);
        message.content = Content::Revoked;
        events
            .send(Event::MessageUpdated(Box::new(message)))
            .unwrap();
        app.handle_events();
        assert!(app.chat_pins.get(chat).is_some_and(Vec::is_empty));
        assert!(app.pins.get(chat).is_some_and(HashSet::is_empty));
    }

    #[test]
    fn a_worker_pin_refusal_uses_the_reader_language() {
        let mut app = app();
        let (backend, events) = Backend::detached();
        app.backend = backend;
        app.locale = crate::i18n::Locale::Spanish;
        events
            .send(Event::Error(
                "This chat already has three pinned messages".into(),
            ))
            .unwrap();
        app.handle_events();
        assert!(
            app.toasts
                .iter()
                .any(|toast| toast.message.contains("fijados")),
            "the toast is translated"
        );
    }

    /// A pin that expires while the chat is closed leaves the line when it is
    /// opened again, so the read is asked for on every open.
    #[test]
    fn opening_a_chat_reads_its_pins_again() {
        let mut app = app();
        let (backend, mut commands) = Backend::recording();
        app.backend = backend;
        let chat = "1@s.whatsapp.net";
        app.chats.push(Chat::new(chat.into(), "Fixture".into()));

        app.open_chat(chat.into());

        let mut asked = false;
        while let Ok(command) = commands.try_recv() {
            if matches!(command, Command::LoadPins(id) if id == chat) {
                asked = true;
            }
        }
        assert!(asked, "the pins are read when the chat opens");
    }

    /// One pinned row, as the archive hands it over.
    fn pinned_row(chat: &str, id: &str) -> crate::archive::Pinned {
        crate::archive::Pinned {
            chat: chat.to_owned(),
            id: id.to_owned(),
            pinned_at: 1,
            expires_at: i64::MAX,
            text: format!("message {id}"),
            from_me: false,
            sent_at: 1,
            pinner: crate::archive::Pinner {
                by_me: true,
                sender: String::new(),
            },
        }
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
    fn presence_goes_offline_while_the_display_is_off() {
        let mut app = app();
        let (backend, mut commands) = Backend::recording();
        app.backend = backend;
        app.window_focused = true;
        app.report_presence();
        assert!(matches!(commands.try_recv(), Ok(Command::SetOnline(true))));

        app.power.set_conditions(fastframe_power::Conditions {
            display_off: true,
            ..Default::default()
        });
        app.report_presence();
        assert!(matches!(commands.try_recv(), Ok(Command::SetOnline(false))));

        app.power.set_conditions(Default::default());
        app.report_presence();
        assert!(matches!(commands.try_recv(), Ok(Command::SetOnline(true))));
        assert!(commands.try_recv().is_err());
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
        app.info = Some(InfoPanel {
            chat: chat.into(),
            view: InfoView::Overview,
        });
        app.dialog = Some(Dialog::ConfirmClearChat(chat.into()));

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
        assert!(app.info.is_none(), "the panel goes with the chat");
        // Neighbouring chats and their search hits stay.
        assert!(app.chat(other).is_some());
        assert_eq!(app.search_hits.len(), 1);
    }

    /// A pending delete-message question belongs to one chat. When that chat
    /// goes away the message is gone with it, so the question must not stay
    /// open over another chat.
    #[test]
    fn a_removed_chat_closes_its_pending_message_deletion() {
        let directory = tempfile::tempdir().unwrap();
        let (mut app, events) =
            App::headless(AppDirs::under(directory.path()), Settings::default());
        let chat = "peer@s.whatsapp.net";
        let other = "other@s.whatsapp.net";
        for id in [chat, other] {
            app.chats.push(Chat::new(id.into(), "Peer".into()));
        }
        app.dialog = Some(Dialog::ConfirmDeleteMessage {
            chat: other.into(),
            message: "m1".into(),
            for_everyone: true,
        });

        events
            .send(Event::ChatRemoved { chat: chat.into() })
            .unwrap();
        app.handle_events();
        // Another chat's question stays open.
        assert!(app.dialog.is_some());

        events
            .send(Event::ChatRemoved { chat: other.into() })
            .unwrap();
        app.handle_events();
        assert_eq!(app.dialog, None);
    }

    #[test]
    fn a_cleared_chat_keeps_its_row_until_the_phone_confirmed_it() {
        let mut app = app();
        let (backend, mut commands) = Backend::recording();
        app.backend = backend;
        let chat = "peer@s.whatsapp.net";
        let other = "friend@s.whatsapp.net";
        app.chats.push(Chat::new(chat.into(), "Peer".into()));
        app.conversations
            .entry(chat.into())
            .or_default()
            .merge(vec![message(chat, "m1", 100)], false);
        app.drafts.insert(chat.into(), "half-written".into());
        app.open_chat = Some(chat.into());
        app.search_hits.push(message(chat, "m1", 100));
        app.search_hits.push(message(other, "m2", 100));

        let ctx = egui::Context::default();
        app.apply(Action::ClearChat(chat.into()), &ctx);

        // Nothing changes here until the phone has cleared the chat too.
        assert!(app.chat(chat).is_some());
        assert_eq!(
            app.conversations.get(chat).map(|open| open.messages.len()),
            Some(1)
        );
        assert!(app.drafts.contains_key(chat));
        assert!(
            std::iter::from_fn(|| commands.try_recv().ok())
                .any(|command| matches!(command, Command::ClearChat(id) if id == chat))
        );

        let (backend, events) = Backend::detached();
        app.backend = backend;
        events
            .send(Event::ChatCleared {
                chat: chat.into(),
                through: 100,
            })
            .unwrap();
        app.handle_events();

        // The chat stays open with nothing left in it, and its draft goes.
        assert!(app.chat(chat).is_some());
        assert_eq!(
            app.conversations.get(chat).map(|open| open.messages.len()),
            Some(0)
        );
        assert!(!app.drafts.contains_key(chat));
        assert_eq!(app.open_chat, Some(chat.into()));
        // Only the cleared chat loses its search hits.
        assert!(app.search_hits.iter().all(|hit| hit.chat != chat));
        assert_eq!(app.search_hits.len(), 1);
    }

    #[test]
    fn clearing_a_chat_preserves_a_surviving_edit_and_other_account_composer() {
        let chat = "peer@s.whatsapp.net";
        for live in [true, false] {
            let mut app = app();
            app.open_chat = Some(chat.into());
            app.composer = "Synthetic edited text".into();
            app.editing = Some("newer".into());
            app.conversations
                .entry(chat.into())
                .or_default()
                .merge(vec![message(chat, "newer", 200)], false);
            app.handle_chat_cleared(chat, 100, live);
            assert_eq!(app.composer, "Synthetic edited text");
            assert_eq!(app.editing.as_deref(), Some("newer"));
        }
        let mut app = app();
        app.open_chat = Some(chat.into());
        app.composer = "Other account draft".into();
        app.handle_chat_cleared(chat, 100, false);
        assert_eq!(app.composer, "Other account draft");
        app.handle_chat_cleared("another@s.whatsapp.net", 100, true);
        assert_eq!(app.composer, "Other account draft");
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
        app.account_mut().settings.send_read_receipts = false;
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
        app.account_mut().settings.send_read_receipts = true;
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
                account: crate::model::AccountId::first(),
                chat: chat.into(),
                message: Some("secret".into()),
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
                account: crate::model::AccountId::first(),
                chat: chat.into(),
                message: Some("second".into()),
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

    /// Builds a synthetic voice message fixture without reading the user archive.
    fn voice(chat: &str, id: &str, timestamp: i64, path: Option<&str>) -> Message {
        let mut row = message(chat, id, timestamp);
        row.content = Content::Audio {
            media: Media {
                mime: "audio/ogg".into(),
                size: 1,
                width: None,
                height: None,
                album: None,
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
        // Played on the phone.
        let mut heard = voice(chat, "heard", 3, None);
        heard.status = Delivery::Played;
        let mut song = voice(chat, "song", 6, None);
        if let Content::Audio { voice_note, .. } = &mut song.content {
            *voice_note = false;
        }
        app.conversations.entry(chat.into()).or_default().merge(
            vec![
                voice(chat, "played", 1, None),
                own,
                heard,
                voice(chat, "waiting", 4, None),
                message(chat, "text", 5),
                voice(chat, "after text", 6, None),
                song,
                voice(chat, "after song", 7, None),
            ],
            false,
        );
        app.played_told.insert("played".into());

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
            history_order: None,
            edited: false,
            mentions: Vec::new(),
            forwarded: false,
            thumbnail: None,
        }
    }

    /// Settings measures the account's media cache as it opens; Clear empties
    /// that account's cache, and the files on screen that lived there are
    /// forgotten so they download again. An older measurement is dropped.
    #[test]
    fn the_media_cache_is_measured_in_settings_and_cleared_for_its_account() {
        let root = tempfile::tempdir().unwrap();
        let (mut app, _) = App::headless(AppDirs::under(root.path()), Settings::default());
        let (backend, mut commands, events) = Backend::recording_with_events();
        app.backend = backend;
        let ctx = egui::Context::default();
        app.apply(Action::Open(Page::Settings), &ctx);
        let sent: Vec<_> = std::iter::from_fn(|| commands.try_recv().ok()).collect();
        let Some(asked) = sent.iter().find_map(|command| match command {
            Command::LoadStorageStats { token } => Some(*token),
            _ => None,
        }) else {
            panic!("opening Settings measures the cache: {sent:?}");
        };
        let cache = app.account().dirs.media_cache_dir();
        let chat = "fixture@s.whatsapp.net";
        let mut photo = message(chat, "photo", 1);
        photo.content = Content::Image {
            caption: None,
            motion: None,
            media: Media {
                mime: "image/jpeg".into(),
                size: 1,
                width: None,
                height: None,
                album: None,
                path: Some(cache.join("photo.jpg")),
                state: MediaState::Idle,
            },
        };
        app.conversations
            .entry(chat.into())
            .or_default()
            .messages
            .push(photo);
        let id = app.account().id.clone();
        app.apply(Action::ClearMediaCache(id), &ctx);
        assert!(app.storage_clearing);
        let token = match commands.try_recv() {
            Ok(Command::ClearMediaCache { token }) => token,
            other => panic!("the account's backend empties its cache: {other:?}"),
        };
        assert!(token > asked);
        // The measurement asked for before the clear arrives late and is dropped.
        let stats = |images| crate::model::StorageStats {
            images,
            image_bytes: images,
            ..Default::default()
        };
        for (stats, token, cleared) in [(stats(5), asked, false), (stats(0), token, true)] {
            events
                .send(Event::StorageStats {
                    stats: Some(stats),
                    token,
                    cleared,
                })
                .unwrap();
        }
        app.handle_events();
        assert_eq!(app.storage_stats, Some(stats(0)));
        assert!(!app.storage_clearing);
        let shown = &app.conversations[chat].messages[0];
        assert_eq!(shown.content.media().unwrap().path, None);
    }

    /// Home moves to the oldest loaded message first; Home again there asks
    /// for older history.
    #[test]
    fn home_on_the_oldest_message_asks_for_older_history() {
        use crate::ui::list_nav::Step;
        let mut app = app();
        let ctx = egui::Context::default();
        let chat = "fixture@s.whatsapp.net";
        app.open_chat = Some(chat.into());
        let conversation = Conversation {
            messages: (0..3)
                .map(|n| message(chat, &format!("m{n}"), 100 + n))
                .collect(),
            ..Default::default()
        };
        app.conversations.insert(chat.into(), conversation);
        app.message_cursor.insert(chat.into(), "m1".into());
        app.apply(Action::MessageCursor(Step::First), &ctx);
        assert_eq!(app.message_cursor[chat], "m0");
        assert!(app.reveal_cursor);
        assert!(
            !app.conversations[chat].loading_older,
            "the first Home only moves"
        );
        app.apply(Action::MessageCursor(Step::First), &ctx);
        assert_eq!(app.message_cursor[chat], "m0");
        assert!(
            app.conversations[chat].loading_older,
            "Home on the oldest asks for more"
        );
    }

    #[test]
    fn following_a_message_reveals_it_only_in_a_focused_list() {
        let mut app = app();
        let chat = "fixture@s.whatsapp.net";
        app.open_chat = Some(chat.into());
        app.follow_message(chat, "m1");
        assert_eq!(app.message_cursor[chat], "m1");
        assert!(
            !app.reveal_cursor,
            "an unfocused list only moves its cursor"
        );
        app.messages_focused = true;
        app.follow_message(chat, "m2");
        assert_eq!(app.message_cursor[chat], "m2");
        assert!(app.reveal_cursor);
    }

    /// Checks that incoming replacements retain local downloaded media paths and transfer state.
    #[test]
    fn merge_keeps_a_downloaded_medias_path_and_state() {
        let mut conversation = Conversation::default();
        let chat = "fixture@s.whatsapp.net";
        let image = |path: Option<PathBuf>, state: MediaState| Message {
            content: Content::Image {
                motion: None,
                caption: None,
                media: Media {
                    mime: "image/jpeg".into(),
                    size: 100,
                    width: None,
                    height: None,
                    album: None,
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
        conversation.merge(vec![image(None, MediaState::Idle)], true);
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

    /// Checks that clicking a pending video note resumes playback only after its download arrives.
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
                album: None,
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

    /// Choosing a font saves the choice and installs it at once.
    #[test]
    fn choosing_a_font_saves_and_applies_it() {
        use crate::settings::FontChoice;
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        assert_eq!(app.settings.font, FontChoice::System);
        app.apply(Action::SetFont(FontChoice::Inter), &ctx);
        assert_eq!(app.settings.font, FontChoice::Inter);
        assert!(crate::theme::inter_chosen());
        app.apply(Action::SetFont(FontChoice::System), &ctx);
        assert_eq!(app.settings.font, FontChoice::System);
        assert!(!crate::theme::inter_chosen());
    }

    /// A video opens in the viewer at a size worth the room, goes back to
    /// its message at the bubble's, and does not outlive the chat.
    #[test]
    fn a_video_covers_the_window_and_goes_back_to_its_message() {
        let mut app = app();
        app.video.silence();
        let chat = "fixture@s.whatsapp.net";
        app.open_chat = Some(chat.into());
        let ctx = egui::Context::default();
        let path = PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/video/sample.mp4"
        ));
        let media = Media {
            album: None,
            mime: "video/mp4".into(),
            size: 1,
            width: None,
            height: None,
            path: Some(path.clone()),
            state: MediaState::Idle,
        };
        app.conversations.entry(chat.into()).or_default().messages = vec![Message {
            content: Content::Video {
                caption: None,
                media,
                seconds: None,
                gif: false,
                note: false,
            },
            ..message(chat, "clip", 1)
        }];
        let bubble = app.video.side();
        let expand = || Action::ExpandVideo {
            message: "clip".into(),
            path: path.clone(),
        };
        let viewing = |app: &App| {
            app.image_preview
                .as_ref()
                .is_some_and(|preview| preview.message() == "clip")
        };

        // Not loaded yet: it starts in the viewer, already at the larger size.
        app.apply(expand(), &ctx);
        assert_eq!(app.video.message(), Some("clip"));
        assert!(viewing(&app) && app.video.is_active());
        assert!(
            app.image_preview
                .as_ref()
                .unwrap()
                .shows_video(&app.viewer_media),
            "a video from its bubble is a clip, whatever the album says"
        );
        assert!(app.video.side() > bubble);

        app.apply(Action::CloseImagePreview, &ctx);
        assert!(app.image_preview.is_none());
        assert_eq!(app.video.side(), bubble);
        assert!(
            app.video.message().is_none(),
            "closing the viewer stops the clip, even one its bubble could show"
        );

        // Playing in its bubble, expanding it keeps it playing in the viewer.
        app.apply(
            Action::PlayVideo {
                message: "clip".into(),
                path: path.clone(),
            },
            &ctx,
        );
        assert!(app.video.is_active());
        app.apply(expand(), &ctx);
        assert!(viewing(&app) && app.video.is_active());

        // Leaving the chat stops the video and takes the viewer down with it.
        app.open_chat = None;
        app.tick_video(&ctx);
        assert!(app.video.message().is_none());
        assert!(app.image_preview.is_none());
        assert_eq!(app.video.side(), bubble);
    }

    /// A motion photo's clip plays over its photo in the viewer; going back
    /// to the photo, or closing the viewer, stops it, since the photo's
    /// bubble has no player to take it back.
    #[test]
    fn a_motion_clip_plays_over_its_photo_and_stops_with_it() {
        let mut app = app();
        app.video.silence();
        let chat = "fixture@s.whatsapp.net";
        app.open_chat = Some(chat.into());
        let ctx = egui::Context::default();
        let clip = PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/video/sample.mp4"
        ));
        let photo = PathBuf::from("/fixture/photo.jpg");
        let media = Media {
            album: None,
            mime: "image/jpeg".into(),
            size: 1,
            width: None,
            height: None,
            path: Some(photo.clone()),
            state: MediaState::Idle,
        };
        app.conversations.entry(chat.into()).or_default().messages = vec![Message {
            content: Content::Image {
                caption: None,
                media,
                motion: Some(crate::model::Motion {
                    path: Some(clip.clone()),
                    ..Default::default()
                }),
            },
            ..message(chat, "photo", 1)
        }];
        let play = || Action::ExpandVideo {
            message: "photo".into(),
            path: clip.clone(),
        };

        app.apply(play(), &ctx);
        let preview = app.image_preview.as_ref().expect("the viewer opens");
        assert_eq!(preview.message(), "photo", "the photo stays the item");
        assert_eq!(preview.path(), Some(photo.as_path()));
        assert_eq!(preview.motion(), Some(clip.as_path()));
        assert_eq!(app.video.message(), Some("photo"));

        // Back to the photo, which the album does not hold yet.
        app.apply(
            Action::ViewImage {
                card: None,
                message: "photo".into(),
            },
            &ctx,
        );
        let preview = app.image_preview.as_ref().unwrap();
        assert_eq!(preview.motion(), None);
        assert_eq!(preview.path(), Some(photo.as_path()));
        assert!(app.video.message().is_none(), "the clip stops");

        app.apply(play(), &ctx);
        assert_eq!(app.video.message(), Some("photo"));
        app.apply(Action::CloseImagePreview, &ctx);
        assert!(app.video.message().is_none(), "nothing takes the clip back");
    }

    /// Checks that repeated media clicks do not duplicate an active download command.
    #[test]
    fn repeated_download_clicks_do_not_queue_more_requests() {
        let mut app = app();
        let (backend, mut commands) = Backend::recording();
        app.backend = backend;
        let chat = "fixture@s.whatsapp.net";
        let mut attachment = message(chat, "picture", 1);
        attachment.content = Content::Image {
            motion: None,
            caption: None,
            media: Media {
                mime: "image/jpeg".into(),
                size: 100,
                width: None,
                height: None,
                album: None,
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
                    manual: false,
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
                manual: true,
                card: None,
                chat: chat.into(),
                message: "picture".into(),
            },
            &ctx,
        );
        assert!(matches!(commands.try_recv(), Ok(Command::Download { .. })));
    }

    /// The viewer browses the album the archive holds, which reaches older
    /// messages than the loaded transcript. Downloading one of those has to
    /// reach the worker all the same: it reads the download keys from the
    /// archive, so a message that is not in `conversations` is not a reason to
    /// drop the request.
    #[test]
    fn a_download_for_a_message_outside_the_loaded_transcript_reaches_the_worker() {
        let mut app = app();
        let (backend, mut commands) = Backend::recording();
        app.backend = backend;
        let chat = "fixture@s.whatsapp.net";
        let ctx = egui::Context::default();
        assert!(!app.conversations.contains_key(chat));
        app.apply(
            Action::Download {
                manual: true,
                card: None,
                chat: chat.into(),
                message: "older-picture".into(),
            },
            &ctx,
        );
        assert!(matches!(commands.try_recv(), Ok(Command::Download { .. })));
    }

    /// Checks that carousel download results update only the requested card.
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
                album: None,
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
                manual: true,
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

    /// Builds a synthetic send refusal without accessing the user archive.
    fn refused(chat: &str, quoting: Option<&str>, unsent: Unsent, reason: Refusal) -> Event {
        Event::SendRefused {
            chat: chat.into(),
            quoting: quoting.map(crate::model::ReplyTarget::from),
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

    /// Checks that refusing a text send restores both draft text and the original quote target.
    #[test]
    fn sending_a_message_scrolls_the_chat_list_to_the_top() {
        let root = tempfile::tempdir().unwrap();
        let (mut app, _) = App::headless(AppDirs::under(root.path()), Settings::default());
        let (backend, _commands) = Backend::recording();
        app.backend = backend;
        let ctx = egui::Context::default();
        let chat = "fixture@s.whatsapp.net";
        app.open_chat = Some(chat.into());
        app.chats
            .push(Chat::new("other@s.whatsapp.net".into(), "Other".into()));
        app.dialog = Some(Dialog::Forward {
            chat: chat.into(),
            messages: vec!["fixture-message".into()],
        });
        app.forward_recipients = vec!["other@s.whatsapp.net".into()];
        let sends = [
            Action::SendText {
                chat: chat.into(),
                text: "Text fixture".into(),
                quoting: None,
            },
            Action::SendSticker(PathBuf::from("sticker.webp")),
            Action::ShareStickerPack(PathBuf::from("sticker-pack")),
            Action::ConfirmForward,
            Action::CreatePoll {
                chat: chat.into(),
                draft: crate::model::PollDraft {
                    question: "Question fixture".into(),
                    options: vec!["One".into(), "Two".into()],
                    multiple: false,
                },
            },
        ];
        for send in sends {
            let name = format!("{send:?}");
            app.scroll_chats_to_top = false;
            app.apply(send, &ctx);
            assert!(app.scroll_chats_to_top, "{name} scrolls the list up");
        }
    }

    #[test]
    fn sending_under_the_favorites_chip_keeps_the_list_in_place() {
        let root = tempfile::tempdir().unwrap();
        let (mut app, _) = App::headless(AppDirs::under(root.path()), Settings::default());
        let (backend, _commands) = Backend::recording();
        app.backend = backend;
        let chat = "fixture@s.whatsapp.net";
        app.open_chat = Some(chat.into());
        // Favorites keep the phone's order, so the chat does not move up.
        app.chat_filter = ChatFilter::Favorites;
        app.apply(
            Action::SendText {
                chat: chat.into(),
                text: "Text fixture".into(),
                quoting: None,
            },
            &egui::Context::default(),
        );
        assert!(!app.scroll_chats_to_top);
    }

    #[test]
    fn sending_a_spot_closes_the_dialog_and_a_refusal_brings_it_back() {
        let root = tempfile::tempdir().unwrap();
        let (mut app, _) = App::headless(AppDirs::under(root.path()), Settings::default());
        let (backend, mut commands, events) = Backend::recording_with_events();
        app.backend = backend;
        let chat = "fixture@s.whatsapp.net";
        app.open_chat = Some(chat.into());
        app.dialog = Some(Dialog::SendLocation(chat.into()));
        app.location_draft = "-23.5505, -46.6333".into();
        app.apply(
            Action::SendLocation {
                chat: chat.into(),
                latitude: -23.5505,
                longitude: -46.6333,
                quoting: None,
            },
            &egui::Context::default(),
        );
        let sent: Vec<_> = std::iter::from_fn(|| commands.try_recv().ok()).collect();
        assert!(sent.iter().any(|command| matches!(command,
            Command::SendLocation { latitude, longitude, .. }
                if *latitude == -23.5505 && *longitude == -46.6333)));
        assert_eq!(app.dialog, None, "the dialog closes over the spot");
        assert!(app.location_draft.is_empty(), "and forgets it");

        // A refusal sends the spot back to the dialog, which is the only place
        // it can be held: the composer takes text.
        events
            .send(refused(
                chat,
                None,
                Unsent::Location {
                    latitude: -23.5505,
                    longitude: -46.6333,
                },
                Refusal::Offline,
            ))
            .unwrap();
        app.handle_events();
        assert_eq!(app.dialog, Some(Dialog::SendLocation(chat.into())));
        assert_eq!(app.location_draft, "-23.55050, -46.63330");
    }

    /// A refusal that arrives late never replaces a dialog opened since, and
    /// never drops the spot: it goes to the chat's composer or draft as text.
    #[test]
    fn a_late_location_refusal_keeps_newer_state_and_the_spot() {
        let root = tempfile::tempdir().unwrap();
        let (mut app, _) = App::headless(AppDirs::under(root.path()), Settings::default());
        let (backend, _commands, events) = Backend::recording_with_events();
        app.backend = backend;
        let chat = "fixture@s.whatsapp.net";
        let other = "other@s.whatsapp.net";
        let spot = Unsent::Location {
            latitude: -23.5505,
            longitude: -46.6333,
        };

        // The reader opened a second location dialog and typed another spot.
        app.open_chat = Some(chat.into());
        app.dialog = Some(Dialog::SendLocation(chat.into()));
        app.location_draft = "51.5074, -0.1278".into();
        events
            .send(refused(chat, None, spot.clone(), Refusal::Offline))
            .unwrap();
        app.handle_events();
        assert_eq!(app.dialog, Some(Dialog::SendLocation(chat.into())));
        assert_eq!(
            app.location_draft, "51.5074, -0.1278",
            "the newer spot stays"
        );
        assert_eq!(
            app.composer, "-23.55050, -46.63330",
            "the refused one waits"
        );

        // The reader moved to another chat: the spot waits in its own draft.
        app.dialog = None;
        app.composer.clear();
        app.open_chat = Some(other.into());
        events
            .send(refused(chat, None, spot, Refusal::Offline))
            .unwrap();
        app.handle_events();
        assert_eq!(app.dialog, None);
        assert_eq!(
            app.drafts.get(chat).map(String::as_str),
            Some("-23.55050, -46.63330")
        );
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
            Command::SendText { quoting: Some(id), .. } if id.id == "original")));
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

    /// Checks that sending a quoted reply preserves the loaded transcript rather than dropping older rows.
    #[test]
    fn sending_a_reply_keeps_older_messages_in_view() {
        let mut app = app();
        let (backend, mut commands) = Backend::recording();
        app.backend = backend;
        let chat = "fixture@s.whatsapp.net";
        app.open_chat = Some(chat.into());
        app.at_bottom = false;
        app.scroll_to_bottom = false;

        app.apply(
            Action::SendText {
                chat: chat.into(),
                text: "Reply fixture".into(),
                quoting: Some("older-message".into()),
            },
            &egui::Context::default(),
        );

        assert!(!app.scroll_to_bottom, "the older position stays selected");
        assert!(!app.at_bottom, "sending does not pretend the view moved");
        assert!(
            std::iter::from_fn(|| commands.try_recv().ok()).any(|command| matches!(
                command,
                Command::SendText { quoting: Some(id), .. } if id.id == "older-message"
            ))
        );

        app.at_bottom = true;
        app.scroll_to_bottom = false;
        app.apply(
            Action::SendText {
                chat: chat.into(),
                text: "Latest fixture".into(),
                quoting: None,
            },
            &egui::Context::default(),
        );

        assert!(
            app.scroll_to_bottom,
            "a reader at the newest edge keeps following outgoing messages"
        );
        assert!(app.at_bottom);
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

    /// Checks that attachment and GIF refusals restore the same quote alongside their unsent media.
    #[test]
    /// Attachment and GIF sends preserve replies and restore their drafts when refused.
    fn attachments_and_gifs_carry_the_reply_and_come_back_when_refused() {
        let root = tempfile::tempdir().unwrap();
        let (mut app, _) = App::headless(AppDirs::under(root.path()), Settings::default());
        let (backend, mut commands, events) = Backend::recording_with_events();
        app.backend = backend;
        let ctx = egui::Context::default();
        let chat = "fixture@s.whatsapp.net";
        app.open_chat = Some(chat.into());
        app.reply_to = Some("original".into());
        app.stage(Pending::Picture {
            width: 1,
            height: 1,
            rgba: std::sync::Arc::new(vec![1, 2, 3, 4]),
            texture: None,
        });
        app.stage(Pending::File("/fixture/a.pdf".into()));
        app.apply(
            Action::SendPending {
                account: app.account().id.clone(),
                mentions: app.composer_mentions.clone(),
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
                Command::SaveDraft { text, .. },
                Command::SendImage { quoting: Some(id), caption: Some(_), .. },
                Command::SendFiles { quoting: None, caption: None, .. },
            ] if id.id == "original" && text.is_empty()
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
                Command::SendGif { quoting: Some(id), .. } if id.id == "original"))
        );
        // Refused attachments return to the composer with their caption.
        for unsent in [
            Unsent::Image {
                width: 1,
                height: 1,
                rgba: vec![1, 2, 3, 4],
                view_once: false,
                caption: Some("Caption fixture".into()),
            },
            Unsent::Files {
                paths: vec!["/fixture/a.pdf".into()],
                documents: false,
                view_once: false,
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
            [
                Staged { item: Pending::Picture { width: 1, height: 1, .. }, .. },
                Staged { item: Pending::File(path), .. }
            ] if path == std::path::Path::new("/fixture/a.pdf")
        ));
        assert_eq!(app.composer, "Caption fixture");
        assert_eq!(app.reply_to.as_deref(), Some("original"));
        // Repeats of one error share a toast.
        assert_eq!(error_toasts(&app).len(), 1);
    }

    #[test]
    fn a_pasted_photo_can_be_sent_view_once_and_keeps_the_choice_in_its_draft() {
        let directory = tempfile::tempdir().unwrap();
        let mut app = App::headless(AppDirs::under(directory.path()), Settings::default()).0;
        let chat = "fixture@s.whatsapp.net";
        app.chats.push(Chat::new(chat.into(), "Fixture".into()));
        app.open_chat(chat.into());
        app.stage(Pending::Picture {
            width: 1,
            height: 1,
            rgba: std::sync::Arc::new(vec![1, 2, 3, 4]),
            texture: None,
        });
        assert!(app.pending_supports_view_once());
        let ctx = egui::Context::default();
        app.apply(Action::SetPendingViewOnce(true), &ctx);
        assert!(app.pending_view_once);

        app.park_composer();
        assert!(app.attachment_drafts[chat].view_once);
        app.restore_composer();
        assert!(app.pending_view_once);

        // Extra attachments cannot silently switch the photo back to normal
        // media. The choice stays until the draft is valid again or disabled.
        app.stage(Pending::File("notes.pdf".into()));
        assert!(app.pending_view_once);
        app.apply(Action::RemovePending(1), &ctx);
        assert!(app.pending_view_once);

        let (backend, mut commands) = Backend::recording();
        app.backend = backend;
        let account = app.account().id.clone();
        app.send_pending(account, chat.into(), "Caption".into(), Vec::new());
        let sent: Vec<_> = std::iter::from_fn(|| commands.try_recv().ok()).collect();
        assert!(sent.iter().any(|command| matches!(command,
            Command::SendImage { chat: sent_chat, view_once: true, caption: Some(caption), .. }
                if sent_chat == chat && caption == "Caption")));
        assert!(app.pending.is_empty());
        assert!(!app.pending_view_once);
    }

    #[test]
    fn one_supported_photo_file_can_be_sent_view_once() {
        let directory = tempfile::tempdir().unwrap();
        let mut app = App::headless(AppDirs::under(directory.path()), Settings::default()).0;
        let chat = "fixture@s.whatsapp.net";
        app.chats.push(Chat::new(chat.into(), "Fixture".into()));
        app.open_chat(chat.into());
        app.stage(Pending::File("photo.png".into()));
        assert!(app.pending_supports_view_once());
        let ctx = egui::Context::default();
        app.apply(Action::SetPendingViewOnce(true), &ctx);

        let (backend, mut commands) = Backend::recording();
        app.backend = backend;
        let account = app.account().id.clone();
        app.send_pending(account, chat.into(), String::new(), Vec::new());
        assert!(
            std::iter::from_fn(|| commands.try_recv().ok()).any(|command| matches!(command,
            Command::SendFiles { paths, documents: false, view_once: true, .. }
                if paths == [PathBuf::from("photo.png")]))
        );
    }

    #[test]
    fn a_refused_view_once_photo_returns_to_its_chat_with_caption_and_quote() {
        for away in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let mut app = App::headless(AppDirs::under(directory.path()), Settings::default()).0;
            let chat = "fixture@s.whatsapp.net";
            let other = "other@s.whatsapp.net";
            app.chats.push(Chat::new(chat.into(), "Fixture".into()));
            app.chats.push(Chat::new(other.into(), "Other".into()));
            app.open_chat(if away { other } else { chat }.into());
            app.send_refused(
                chat.into(),
                Some("original".into()),
                Unsent::Files {
                    paths: vec!["photo.png".into()],
                    documents: false,
                    view_once: true,
                    caption: Some("Caption fixture".into()),
                },
                Refusal::Offline,
            );
            if away {
                assert!(app.pending.is_empty());
                assert!(!app.pending_view_once);
                assert!(app.attachment_drafts[chat].view_once);
                app.open_chat(chat.into());
            }
            assert!(app.pending_view_once);
            assert_eq!(app.pending.len(), 1);
            assert_eq!(app.composer, "Caption fixture");
            assert_eq!(app.reply_to.as_deref(), Some("original"));
        }
    }

    #[test]
    fn a_refused_view_once_photo_cannot_be_downgraded_when_new_attachments_exist() {
        for away in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let mut app = App::headless(AppDirs::under(directory.path()), Settings::default()).0;
            let chat = "fixture@s.whatsapp.net";
            let other = "other@s.whatsapp.net";
            app.chats.push(Chat::new(chat.into(), "Fixture".into()));
            app.chats.push(Chat::new(other.into(), "Other".into()));
            app.open_chat(chat.into());
            app.stage(Pending::Document("notes.pdf".into()));
            app.stage(Pending::Document("second.pdf".into()));
            if away {
                app.open_chat(other.into());
            }
            app.send_refused(
                chat.into(),
                None,
                Unsent::Files {
                    paths: vec!["photo.png".into()],
                    documents: false,
                    view_once: true,
                    caption: Some("Caption fixture".into()),
                },
                Refusal::Offline,
            );
            if away {
                app.open_chat(chat.into());
            }
            let (backend, mut commands) = Backend::recording();
            app.backend = backend;
            // The view takes the caption before submitting SendPending.
            let caption = std::mem::take(&mut app.composer);
            app.send_pending(app.account().id.clone(), chat.into(), caption, vec![]);
            assert!(
                !std::iter::from_fn(|| commands.try_recv().ok()).any(|command| matches!(
                    command,
                    Command::SendFiles { .. } | Command::SendImage { .. }
                ))
            );
            assert_eq!(app.pending.len(), 3);
            assert!(app.pending_view_once);
            assert_eq!(app.composer, "Caption fixture");
            assert!(!error_toasts(&app).is_empty());

            // Removing only one extra does not silently clear the choice.
            app.apply(Action::RemovePending(0), &egui::Context::default());
            assert!(app.pending_view_once);
            let caption = std::mem::take(&mut app.composer);
            app.send_pending(app.account().id.clone(), chat.into(), caption, vec![]);
            assert!(
                !std::iter::from_fn(|| commands.try_recv().ok()).any(|command| matches!(
                    command,
                    Command::SendFiles { .. } | Command::SendImage { .. }
                ))
            );
            assert_eq!(app.composer, "Caption fixture");
            // Once only the original photo remains, it keeps the option.
            app.apply(Action::RemovePending(0), &egui::Context::default());
            let caption = std::mem::take(&mut app.composer);
            app.send_pending(app.account().id.clone(), chat.into(), caption, vec![]);
            assert!(
                std::iter::from_fn(|| commands.try_recv().ok()).any(|command| matches!(
                    command,
                    Command::SendFiles {
                        view_once: true,
                        ..
                    }
                ))
            );
        }
    }

    #[test]
    fn view_once_file_types_match_the_enabled_photo_decoders() {
        for extension in ["jpg", "jpeg", "png", "webp"] {
            let path = PathBuf::from(format!("photo.{extension}"));
            assert!(
                Pending::is_view_once_photo_file(&path),
                "{extension} should be eligible"
            );
        }
        for extension in ["bmp", "tif", "tiff", "gif", "avif", "pdf"] {
            let path = PathBuf::from(format!("attachment.{extension}"));
            assert!(
                !Pending::is_view_once_photo_file(&path),
                "{extension} should not be eligible"
            );
        }
    }

    /// Checks that an unsent recording remains scoped to its destination and retains its quote.
    #[test]
    fn a_picture_between_files_is_sent_in_its_place() {
        let root = tempfile::tempdir().unwrap();
        let (mut app, _) = App::headless(AppDirs::under(root.path()), Settings::default());
        let (backend, mut commands) = Backend::recording();
        app.backend = backend;
        let ctx = egui::Context::default();
        let chat = "fixture@s.whatsapp.net";
        app.open_chat = Some(chat.into());
        app.stage(Pending::File("/fixture/a.png".into()));
        app.stage(Pending::Picture {
            width: 1,
            height: 1,
            rgba: std::sync::Arc::new(vec![0; 4]),
            texture: None,
        });
        app.stage(Pending::File("/fixture/b.png".into()));
        app.apply(
            Action::SendPending {
                account: app.account().id.clone(),
                mentions: Vec::new(),
                chat: chat.into(),
                caption: "Caption fixture".into(),
            },
            &ctx,
        );
        let sent: Vec<_> = std::iter::from_fn(|| commands.try_recv().ok())
            .filter_map(|command| match command {
                Command::SendFiles { paths, caption, .. } => Some((Some(paths), caption)),
                Command::SendImage { caption, .. } => Some((None, caption)),
                _ => None,
            })
            .collect();
        // The caption goes with the first thing staged, a file here.
        assert_eq!(
            sent,
            vec![
                (
                    Some(vec![PathBuf::from("/fixture/a.png")]),
                    Some("Caption fixture".to_owned())
                ),
                (None, None),
                (Some(vec![PathBuf::from("/fixture/b.png")]), None),
            ]
        );
    }

    #[test]
    fn picked_documents_are_sent_as_documents_in_the_order_they_were_staged() {
        let root = tempfile::tempdir().unwrap();
        let (mut app, _) = App::headless(AppDirs::under(root.path()), Settings::default());
        let (backend, mut commands, events) = Backend::recording_with_events();
        app.backend = backend;
        let ctx = egui::Context::default();
        let chat = "fixture@s.whatsapp.net";
        app.open_chat = Some(chat.into());
        app.apply(Action::Attach { documents: true }, &ctx);
        assert!(matches!(
            commands.try_recv(),
            Ok(Command::PickFiles { chat: id, documents: true }) if id == chat
        ));
        app.stage(Pending::File("/fixture/a.png".into()));
        events
            .send(Event::Picked {
                chat: chat.into(),
                paths: vec!["/fixture/b.png".into(), "/fixture/c.pdf".into()],
                documents: true,
            })
            .unwrap();
        app.handle_events();
        app.stage(Pending::File("/fixture/d.png".into()));
        let staged: Vec<_> = app.pending.iter().map(|staged| &staged.item).collect();
        assert!(matches!(
            staged.as_slice(),
            [Pending::File(_), Pending::Document(b), Pending::Document(c), Pending::File(_)]
                if b == std::path::Path::new("/fixture/b.png")
                    && c == std::path::Path::new("/fixture/c.pdf")
        ));
        app.apply(
            Action::SendPending {
                account: app.account().id.clone(),
                mentions: Vec::new(),
                chat: chat.into(),
                caption: "Caption fixture".into(),
            },
            &ctx,
        );
        let sent: Vec<_> = std::iter::from_fn(|| commands.try_recv().ok())
            .filter_map(|command| match command {
                Command::SendFiles {
                    paths,
                    documents,
                    caption,
                    ..
                } => Some((paths, documents, caption)),
                _ => None,
            })
            .collect();
        // The caption goes with the first file, and each run keeps its kind.
        assert_eq!(
            sent,
            vec![
                (
                    vec![PathBuf::from("/fixture/a.png")],
                    false,
                    Some("Caption fixture".to_owned())
                ),
                (
                    vec![
                        PathBuf::from("/fixture/b.png"),
                        PathBuf::from("/fixture/c.pdf")
                    ],
                    true,
                    None
                ),
                (vec![PathBuf::from("/fixture/d.png")], false, None),
            ]
        );
        // A refused document comes back as a document.
        events
            .send(refused(
                chat,
                None,
                Unsent::Files {
                    paths: vec!["/fixture/b.png".into()],
                    documents: true,
                    view_once: false,
                    caption: None,
                },
                Refusal::Offline,
            ))
            .unwrap();
        app.handle_events();
        assert!(matches!(
            app.pending.as_slice(),
            [Staged { item: Pending::Document(path), .. }]
                if path == std::path::Path::new("/fixture/b.png")
        ));
    }

    #[test]
    fn a_crop_that_lands_after_its_attachment_was_removed_is_dropped() {
        let root = tempfile::tempdir().unwrap();
        let (mut app, _) = App::headless(AppDirs::under(root.path()), Settings::default());
        let source = std::path::PathBuf::from("/fixture/photo.png");
        let target = app.stage(Pending::File(source.clone()));
        app.picture_applying.insert(
            target,
            crate::model::PictureOrigin {
                source: crate::model::PictureSource::File(source),
                width: 4,
                height: 4,
                crop: crate::model::PictureCrop::full(4, 4),
                turns: 0,
            },
        );
        // Unstaged while its crop is still being written.
        app.pending.clear();
        app.apply_backend_event(
            Event::PictureEdited {
                target,
                result: Ok(std::path::PathBuf::from("/fixture/edited.jpg")),
            },
            true,
        );
        assert!(app.pending.is_empty(), "nothing was staged back");
        assert!(
            !app.picture_origins.contains_key(&target),
            "and nothing was remembered about it"
        );
    }

    #[test]
    fn a_crop_that_lands_closes_only_the_editor_it_opened() {
        let root = tempfile::tempdir().unwrap();
        let (mut app, _) = App::headless(AppDirs::under(root.path()), Settings::default());
        let first = app.stage(Pending::File("/fixture/one.png".into()));
        let second = app.stage(Pending::File("/fixture/two.png".into()));
        // The first crop has been applied, so its editor is already gone, and
        // the second picture has been opened while its file is written.
        let open = crate::model::PictureEdit::new(
            second,
            crate::model::PictureSource::File("/fixture/two.png".into()),
            12,
            6,
        );
        app.picture_edit = Some(open.clone());
        app.apply_backend_event(
            Event::PictureEdited {
                target: first,
                result: Ok(std::path::PathBuf::from("/fixture/edited.jpg")),
            },
            true,
        );
        assert_eq!(
            app.picture_edit,
            Some(open),
            "the crop that landed threw away the editor the user was in"
        );
        let staged = app.pending.first().expect("the first is still staged");
        assert!(
            matches!(
                &staged.item,
                Pending::File(path) if path == std::path::Path::new("/fixture/edited.jpg")
            ),
            "and the first picture was still replaced by its own crop"
        );
    }

    #[test]
    fn a_size_only_opens_the_cropper_while_it_is_still_being_waited_for() {
        let root = tempfile::tempdir().unwrap();
        let (mut app, _) = App::headless(AppDirs::under(root.path()), Settings::default());
        let target = app.stage(Pending::File("/fixture/photo.png".into()));
        // Waited for, so the answer opens the cropper.
        app.picture_inspecting = Some(target);
        app.apply_backend_event(
            Event::PictureInspected {
                target,
                result: Ok((8, 6)),
            },
            true,
        );
        assert!(
            app.picture_edit.is_some(),
            "the size that was asked for opened the cropper"
        );
        // Cancelled, so a later answer for anything else opens nothing.
        app.picture_edit = None;
        app.picture_inspecting = None;
        app.apply_backend_event(
            Event::PictureInspected {
                target,
                result: Ok((8, 6)),
            },
            true,
        );
        assert!(
            app.picture_edit.is_none(),
            "a size that was no longer wanted reopened the cropper"
        );
    }

    #[test]
    fn a_crop_cannot_land_on_the_attachment_that_took_its_place() {
        let root = tempfile::tempdir().unwrap();
        let (mut app, _) = App::headless(AppDirs::under(root.path()), Settings::default());
        let going = app.stage(Pending::File("/fixture/one.png".into()));
        let staying = std::path::PathBuf::from("/fixture/two.png");
        app.stage(Pending::File(staying.clone()));
        // The first goes, so the second moves up into its place.
        app.pending.remove(0);
        app.apply_backend_event(
            Event::PictureEdited {
                target: going,
                result: Ok(std::path::PathBuf::from("/fixture/edited.jpg")),
            },
            true,
        );
        let staged = app.pending.first().expect("the other one is still staged");
        assert!(
            matches!(&staged.item, Pending::File(path) if *path == staying),
            "the late result did not replace the picture that moved into its place"
        );
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
            Ok(Command::SendVoice { quoting: Some(id), .. }) if id.id == "original"
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

    /// Private quotes keep their source identity through chat switches and send refusals.
    #[test]
    fn private_reply_opens_the_sender_and_preserves_both_drafts() {
        let mut app = app();
        let (backend, mut commands) = Backend::recording();
        app.backend = backend;
        let ctx = egui::Context::default();
        let group = "12345@g.us";
        let sender = "15550002222@s.whatsapp.net";
        app.chats
            .push(Chat::new(group.into(), "Group fixture".into()));
        let mut original = message(group, "original", 100);
        original.sender = sender.into();
        original.sender_name = Some("Sender fixture".into());
        app.conversations.insert(
            group.into(),
            Conversation {
                messages: vec![original],
                requested: true,
                ..Default::default()
            },
        );
        app.open_chat = Some(group.into());
        app.composer = "Group draft".into();
        app.drafts.insert(sender.into(), "Private draft".into());
        app.apply(
            Action::ReplyPrivately {
                chat: group.into(),
                message: "original".into(),
            },
            &ctx,
        );
        assert_eq!(app.open_chat.as_deref(), Some(sender));
        assert_eq!(app.composer, "Private draft");
        assert_eq!(
            app.drafts.get(group).map(String::as_str),
            Some("Group draft")
        );
        let target = app.reply_to.clone().unwrap();
        assert_eq!(target.id, "original");
        assert_eq!(target.chat.as_deref(), Some(group));
        assert!(app.focus_composer);
        assert!(
            !std::iter::from_fn(|| commands.try_recv().ok()).any(|command| matches!(
                command,
                Command::SendText { .. } | Command::SendFiles { .. } | Command::SendVoice { .. }
            ))
        );
        app.apply(
            Action::SendText {
                chat: sender.into(),
                text: "Private reply".into(),
                quoting: Some(target.clone()),
            },
            &ctx,
        );
        assert!(std::iter::from_fn(|| commands.try_recv().ok()).any(|command| matches!(command,
            Command::SendText { chat, quoting: Some(quote), .. } if chat == sender && quote == target
        )));
        app.send_refused(
            sender.into(),
            Some(target.clone()),
            Unsent::Text("Private reply".into()),
            Refusal::QuoteUnavailable,
        );
        assert_eq!(app.reply_to, Some(target.clone()));
        app.open_chat(group.into());
        app.send_refused(
            sender.into(),
            Some(target.clone()),
            Unsent::Text("Returned private reply".into()),
            Refusal::Offline,
        );
        assert!(app.reply_to.is_none());
        app.open_chat(sender.into());
        assert_eq!(app.reply_to, Some(target));
    }

    /// Late refusals cannot restore a deleted recipient's private quote, text or voice.
    #[test]
    fn private_reply_refusals_do_not_recreate_deleted_destination_drafts() {
        for voice in [false, true] {
            let mut app = app();
            let (backend, mut commands) = Backend::recording();
            app.backend = backend;
            let ctx = egui::Context::default();
            let group = "12345@g.us";
            let recipient = "15550002222@s.whatsapp.net";
            app.chats
                .push(Chat::new(group.into(), "Group fixture".into()));
            let mut original = message(group, "original", 100);
            original.sender = recipient.into();
            app.conversations
                .entry(group.into())
                .or_default()
                .merge(vec![original], false);
            app.open_chat = Some(group.into());
            app.apply(
                Action::ReplyPrivately {
                    chat: group.into(),
                    message: "original".into(),
                },
                &ctx,
            );
            let target = app.reply_to.clone().unwrap();
            let unsent = if voice {
                let samples = vec![0.0; 10];
                app.unsent_voice = Some((recipient.into(), samples.clone()));
                app.apply(Action::SendRecording, &ctx);
                Unsent::Voice(samples)
            } else {
                app.apply(
                    Action::SendText {
                        chat: recipient.into(),
                        text: "Private draft".into(),
                        quoting: Some(target.clone()),
                    },
                    &ctx,
                );
                Unsent::Text("Private draft".into())
            };
            assert!(std::iter::from_fn(|| commands.try_recv().ok()).any(|command| matches!(command,
                Command::SendText { quoting: Some(quote), .. } | Command::SendVoice { quoting: Some(quote), .. } if quote == target
            )));
            app.forget_chat(recipient);
            app.send_refused(recipient.into(), Some(target), unsent, Refusal::Offline);
            assert!(!app.attachment_drafts.contains_key(recipient));
            assert!(!app.drafts.contains_key(recipient));
            assert!(app.reply_to.is_none());
            assert!(app.unsent_voice.is_none());
            app.chats.push(Chat::new(
                recipient.into(),
                "Recreated recipient fixture".into(),
            ));
            app.open_chat(recipient.into());
            assert!(app.reply_to.is_none());
            assert!(app.composer.is_empty());
            // A new private send in the replacement lifetime still recovers normally.
            app.open_chat(group.into());
            app.apply(
                Action::ReplyPrivately {
                    chat: group.into(),
                    message: "original".into(),
                },
                &ctx,
            );
            let replacement = app.reply_to.take().unwrap();
            app.send_refused(
                recipient.into(),
                Some(replacement.clone()),
                Unsent::Text("Replacement draft".into()),
                Refusal::Offline,
            );
            assert_eq!(app.reply_to, Some(replacement));
            assert_eq!(app.composer, "Replacement draft");
        }
    }

    /// A private quote with nothing typed waits in the chat's attachment
    /// draft, without making the chat list show a draft.
    #[test]
    fn a_bare_private_quote_is_kept_without_a_list_draft() {
        let mut app = app();
        let ctx = egui::Context::default();
        let recipient = "15550003333@s.whatsapp.net";
        let other = "15550004444@s.whatsapp.net";
        app.chats
            .push(Chat::new(recipient.into(), "Recipient fixture".into()));
        app.chats
            .push(Chat::new(other.into(), "Other fixture".into()));
        app.open_chat(recipient.into());
        let target = ReplyTarget {
            id: "original".into(),
            chat: Some("12345@g.us".into()),
        };
        app.reply_to = Some(target.clone());
        app.open_chat(other.into());
        assert!(app.reply_to.is_none());
        assert!(!app.has_draft(recipient));
        assert!(app.draft_preview(recipient).is_none());
        app.apply(Action::CloseChat, &ctx);
        app.open_chat(recipient.into());
        assert_eq!(app.reply_to, Some(target));
        assert!(app.pending.is_empty());
    }

    /// Closing and reopening a recipient restores its private quote, while cancel removes it.
    #[test]
    fn closing_a_private_reply_preserves_the_quote_until_cancelled() {
        let mut app = app();
        let ctx = egui::Context::default();
        let recipient = "15550001111@s.whatsapp.net";
        app.chats
            .push(Chat::new(recipient.into(), "Recipient fixture".into()));
        app.open_chat(recipient.into());
        let target = ReplyTarget {
            id: "original".into(),
            chat: Some("12345@g.us".into()),
        };
        app.reply_to = Some(target.clone());
        app.composer = "Private draft".into();
        app.apply(Action::CloseChat, &ctx);
        assert!(app.open_chat.is_none());
        assert!(app.reply_to.is_none());
        assert_eq!(
            app.attachment_drafts
                .get(recipient)
                .and_then(|draft| draft.quoting.as_ref()),
            Some(&target)
        );
        app.open_chat(recipient.into());
        assert_eq!(app.reply_to, Some(target));
        assert_eq!(app.composer, "Private draft");
        app.reply_to = None;
        app.apply(Action::CloseChat, &ctx);
        assert!(!app.attachment_drafts.contains_key(recipient));
        app.open_chat(recipient.into());
        assert!(app.reply_to.is_none());
    }

    /// Leaving or deleting one recipient must not erase another recipient's quote draft.
    #[test]
    fn leaving_a_chat_preserves_other_private_reply_drafts() {
        for delete in [false, true] {
            let mut app = app();
            let left = "15550001111@s.whatsapp.net";
            let other = "15550002222@s.whatsapp.net";
            let quote = ReplyTarget {
                id: "original".into(),
                chat: Some("12345@g.us".into()),
            };
            app.chats
                .push(Chat::new(left.into(), "Left fixture".into()));
            app.chats
                .push(Chat::new(other.into(), "Other fixture".into()));
            app.open_chat = Some(left.into());
            app.reply_to = Some(quote.clone());
            app.attachment_drafts.insert(
                other.into(),
                AttachmentDraft {
                    pending: Vec::new(),
                    view_once: false,
                    quoting: Some(quote.clone()),
                },
            );
            if delete {
                app.forget_chat(left);
            } else {
                app.leave_chat(left);
            }
            assert!(app.open_chat.is_none());
            assert!(app.reply_to.is_none());
            assert!(!app.attachment_drafts.contains_key(left));
            assert!(app.attachment_drafts.contains_key(other));
            app.open_chat(other.into());
            assert_eq!(app.reply_to, Some(quote));
        }
    }

    /// Only a visible incoming group message and an accessible sender can start a reply.
    #[test]
    fn private_reply_ignores_own_deleted_invalid_and_locked_targets() {
        for case in [
            "own",
            "deleted",
            "invalid",
            "locked",
            "locked-source",
            "stale",
        ] {
            let mut app = app();
            let group = "12345@g.us";
            let sender = "15550002222@s.whatsapp.net";
            let mut original = message(group, "original", 100);
            original.sender = sender.into();
            match case {
                "own" => original.from_me = true,
                "deleted" => original.content = Content::Revoked,
                "invalid" => original.sender = group.into(),
                "locked" => {
                    let mut chat = Chat::new(sender.into(), "Locked fixture".into());
                    chat.locked = true;
                    app.chats.push(chat);
                }
                "locked-source" => {
                    let mut chat = Chat::new(group.into(), "Locked group fixture".into());
                    chat.locked = true;
                    app.chats.push(chat);
                }
                _ => {}
            }
            app.open_chat = Some(if case == "stale" { sender } else { group }.into());
            let previous = app.open_chat.clone();
            app.conversations.insert(
                group.into(),
                Conversation {
                    messages: vec![original],
                    ..Default::default()
                },
            );
            app.apply(
                Action::ReplyPrivately {
                    chat: group.into(),
                    message: "original".into(),
                },
                &egui::Context::default(),
            );
            assert_eq!(app.open_chat, previous, "{case}");
            assert!(app.reply_to.is_none(), "{case}");
        }
    }

    /// Cached source content stays hidden after switching out of an authenticated folder.
    #[test]
    fn private_quote_previews_follow_source_lock_authorization() {
        let mut app = app();
        let group = "12345@g.us";
        let sender = "15550002222@s.whatsapp.net";
        let other = "15550003333@s.whatsapp.net";
        let mut source = Chat::new(group.into(), "Locked group fixture".into());
        source.locked = true;
        app.chats.extend([
            source,
            Chat::new(sender.into(), "Sender fixture".into()),
            Chat::new(other.into(), "Other fixture".into()),
        ]);
        let mut original = message(group, "original", 100);
        original.sender = sender.into();
        app.conversations.insert(
            group.into(),
            Conversation {
                messages: vec![original],
                requested: true,
                ..Default::default()
            },
        );
        app.settings.chat_lock_code_hash = Some("synthetic authenticated verifier".into());
        app.enter_locked_folder();
        app.open_chat = Some(group.into());
        app.apply(
            Action::ReplyPrivately {
                chat: group.into(),
                message: "original".into(),
            },
            &egui::Context::default(),
        );
        assert_eq!(app.open_chat.as_deref(), Some(sender));
        assert!(!app.locked_folder_open());
        let target = app.reply_to.clone().unwrap();
        assert!(app.conversations[group].message(&target).is_some());
        assert!(app.reply_preview(&target, sender).is_none());
        app.open_chat(other.into());
        app.open_chat(sender.into());
        assert_eq!(app.reply_to, Some(target.clone()));
        assert!(app.reply_preview(&target, sender).is_none());
        app.enter_locked_folder();
        assert!(app.reply_preview(&target, sender).is_some());
        app.close_locked_folder();
        assert!(app.reply_preview(&target, sender).is_none());
        let mut unlocked = app.chat(group).unwrap().clone();
        unlocked.locked = false;
        app.handle_chat_updated(unlocked, true);
        assert!(app.reply_preview(&target, sender).is_some());
    }

    /// Every send preserves hidden private quotes and drafts until their source is authorized.
    #[test]
    fn hidden_private_quotes_cannot_be_sent_by_any_composer_action() {
        for authorized in [false, true] {
            for kind in 0..7 {
                let mut app = app();
                let (backend, mut commands) = Backend::recording();
                app.backend = backend;
                let group = "12345@g.us";
                let recipient = "15550002222@s.whatsapp.net";
                let mut source = Chat::new(group.into(), "Locked source fixture".into());
                source.locked = true;
                app.chats.extend([
                    source,
                    Chat::new(recipient.into(), "Recipient fixture".into()),
                ]);
                app.open_chat = Some(recipient.into());
                let target = ReplyTarget {
                    id: "original".into(),
                    chat: Some(group.into()),
                };
                app.reply_to = Some(target.clone());
                app.settings.chat_lock_code_hash = Some("synthetic authenticated verifier".into());
                if authorized {
                    app.enter_locked_folder();
                }
                let action = match kind {
                    0 => Action::SendText {
                        chat: recipient.into(),
                        text: "Private draft".into(),
                        quoting: Some(target.clone()),
                    },
                    1 => {
                        app.stage(Pending::File(PathBuf::from("synthetic-attachment.png")));
                        Action::SendPending {
                            account: app.account().id.clone(),
                            chat: recipient.into(),
                            caption: "Private draft".into(),
                            mentions: Vec::new(),
                        }
                    }
                    2 => Action::SendSticker(PathBuf::from("synthetic-sticker.webp")),
                    3 => Action::SendGif(Gif {
                        id: "fixture".into(),
                        still: None,
                        mp4: "https://example.invalid/fixture.mp4".into(),
                        width: 2,
                        height: 2,
                    }),
                    4 => {
                        app.unsent_voice = Some((recipient.into(), vec![0.0; 10]));
                        Action::SendRecording
                    }
                    5 => {
                        app.recording = Some(crate::audio::Recorder::recorded_fixture(vec![
                                0.0;
                                crate::voice::RATE
                                    as usize
                            ]));
                        Action::SendRecording
                    }
                    _ => {
                        app.sticker_draft = Some(crate::model::StickerDraft {
                            source: PathBuf::from("synthetic-sticker.png"),
                            width: 1,
                            height: 1,
                            transparent: false,
                            crop: crate::model::StickerCrop::centered(1, 1),
                            keep_transparent: false,
                            emojis: String::new(),
                        });
                        Action::MakeSticker { send: true }
                    }
                };
                app.apply(action, &egui::Context::default());
                let sent: Vec<_> = std::iter::from_fn(|| commands.try_recv().ok()).collect();
                if authorized {
                    assert!(
                        sent.iter().any(|command| matches!(command,
                            Command::SendText { quoting: Some(quote), .. } |
                            Command::SendFiles { quoting: Some(quote), .. } |
                            Command::SendSticker { quoting: Some(quote), .. } |
                            Command::SendGif { quoting: Some(quote), .. } |
                            Command::MakeSticker { quoting: Some(quote), .. } |
                            Command::SendVoice { quoting: Some(quote), .. } if quote == &target
                        )),
                        "kind {kind}"
                    );
                    if kind == 6 {
                        assert_eq!(app.reply_to, Some(target));
                    } else {
                        assert!(app.reply_to.is_none());
                    }
                } else {
                    assert!(
                        !sent.iter().any(|command| matches!(
                            command,
                            Command::SendText { .. }
                                | Command::SendFiles { .. }
                                | Command::SendImage { .. }
                                | Command::SendSticker { .. }
                                | Command::SendGif { .. }
                                | Command::SendVoice { .. }
                                | Command::MakeSticker { .. }
                        )),
                        "kind {kind}"
                    );
                    assert_eq!(app.reply_to, Some(target));
                    if kind <= 1 {
                        assert_eq!(app.composer, "Private draft");
                    }
                    if kind == 1 {
                        assert_eq!(app.pending.len(), 1);
                    }
                    if (4..=5).contains(&kind) {
                        assert!(app.unsent_voice.is_some());
                        assert!(app.recording.is_none());
                    }
                }
            }
        }
    }

    /// Checks that sticker dispatch consumes and forwards the pending quote exactly once.
    #[test]
    fn a_loaded_reply_drops_a_pending_one_and_cancel_drops_both() {
        let mut app = app();
        let chat = "fixture@s.whatsapp.net";
        app.open_chat = Some(chat.into());
        app.conversations.insert(
            chat.into(),
            Conversation {
                messages: vec![message(chat, "here", 10)],
                ..Default::default()
            },
        );
        app.reply_when_loaded = Some((chat.into(), "gone".into()));
        app.apply(Action::Reply("here".into()), &egui::Context::default());
        assert_eq!(app.reply_to.as_deref(), Some("here"));
        assert!(app.reply_when_loaded.is_none());
        app.apply(Action::CancelReply, &egui::Context::default());
        assert!(app.reply_to.is_none());
        assert!(app.reply_when_loaded.is_none());
    }

    #[test]
    fn locking_a_chat_drops_it_from_the_open_starred_list() {
        let mut app = app();
        let chat = "fixture@s.whatsapp.net";
        app.chats.push(Chat::new(chat.into(), "Ada".into()));
        app.show_starred = true;
        app.starred.push(crate::archive::Starred {
            message: message(chat, "m1", 10),
            starred_at: 10,
        });
        let mut locked = Chat::new(chat.into(), "Ada".into());
        locked.locked = true;
        app.handle_chat_updated(locked, true);
        assert!(app.starred.is_empty());
    }

    /// Stars synced from the phone arrive in a burst at link time; only a
    /// click here that WhatsApp confirmed says so with a toast.
    #[test]
    fn only_a_confirmed_local_star_toasts() {
        let mut app = app();
        let chat = "fixture@s.whatsapp.net";
        for id in ["m1", "m2", "m3"] {
            app.apply_backend_event(
                Event::StarChanged {
                    chat: chat.into(),
                    message: id.into(),
                    starred: true,
                    local: false,
                },
                true,
            );
        }
        assert!(app.toasts.is_empty(), "phone stars stay quiet");
        assert_eq!(app.stars[chat].len(), 3, "but still mark the messages");
        app.apply_backend_event(
            Event::StarChanged {
                chat: chat.into(),
                message: "m4".into(),
                starred: true,
                local: true,
            },
            true,
        );
        assert_eq!(app.toasts.len(), 1);
        assert_eq!(app.toasts[0].message, "Starred");
    }

    #[test]
    fn an_edited_message_replaces_the_open_starred_row() {
        let mut app = app();
        let (backend, mut commands, events) = Backend::recording_with_events();
        app.backend = backend;
        app.show_starred = true;
        let chat = "fixture@s.whatsapp.net";
        app.starred.push(crate::archive::Starred {
            message: message(chat, "m1", 10),
            starred_at: 1,
        });
        let mut edited = message(chat, "m1", 11);
        edited.content = Content::text("edited");
        events
            .send(Event::MessageUpdated(Box::new(edited.clone())))
            .unwrap();
        app.handle_events();
        assert_eq!(app.starred[0].message.content, edited.content);
        assert!(
            std::iter::from_fn(|| commands.try_recv().ok())
                .any(|command| { matches!(command, Command::LoadStarred) })
        );
    }

    #[test]
    fn toggling_starred_opens_the_sidebar() {
        let mut app = app();
        app.sidebar_visible = false;
        app.apply(Action::ToggleStarred, &egui::Context::default());
        assert!(app.show_starred);
        assert!(app.sidebar_visible);
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
            }) if chat == "fixture@s.whatsapp.net" && id.id == "quoted-message"
        ));
    }

    /// Checks that oversized attachment clicks are refused before a download command is queued.
    #[test]
    fn only_a_click_downloads_an_oversized_attachment() {
        let mut app = app();
        let (backend, mut commands) = Backend::recording();
        app.backend = backend;
        let chat = "peer@s.whatsapp.net";
        let mut attachment = message(chat, "picture", 1);
        attachment.content = Content::Image {
            motion: None,
            caption: None,
            media: Media {
                mime: "image/jpeg".into(),
                size: crate::model::ATTACHMENT_DOWNLOAD_LIMIT + 1,
                width: None,
                height: None,
                album: None,
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
                manual: false,
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

        app.apply(
            Action::Download {
                manual: true,
                card: None,
                chat: chat.into(),
                message: "picture".into(),
            },
            &egui::Context::default(),
        );
        assert!(matches!(
            commands.try_recv(),
            Ok(Command::Download { manual: true, .. })
        ));
        assert!(matches!(
            app.media_of(chat, "picture").map(|media| &media.state),
            Some(MediaState::Downloading)
        ));
    }

    #[test]
    fn history_fidelity_reload_repairs_loaded_order_and_content() {
        let mut conversation = Conversation::default();
        conversation.merge(
            vec![message("c", "third", 100), message("c", "first", 100)],
            false,
        );
        let refreshed = [("second", 2), ("first", 1), ("third", 3)]
            .into_iter()
            .map(|(id, order)| Message {
                history_order: Some(order),
                content: Content::text(format!("updated {id}")),
                edited: true,
                ..message("c", id, 100)
            })
            .collect::<Vec<_>>();
        conversation.merge(refreshed.clone(), true);
        conversation.merge(refreshed, true);
        assert_eq!(
            conversation
                .messages
                .iter()
                .map(|m| m.id.as_str())
                .collect::<Vec<_>>(),
            ["first", "second", "third"]
        );
        assert!(conversation.messages.iter().all(|m| m.edited));
        assert_eq!(
            conversation.messages[0].content,
            Content::text("updated first")
        );
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
    fn a_restamped_message_moves_to_its_new_place() {
        let mut app = app();
        let (backend, events) = Backend::detached();
        app.backend = backend;
        let chat = "1@s.whatsapp.net";
        let mut mine = message(chat, "mine", 200);
        mine.from_me = true;
        let restamped = Message {
            timestamp: 120,
            ..mine.clone()
        };
        app.conversations.entry(chat.into()).or_default().merge(
            vec![message(chat, "a", 100), mine, message(chat, "reply", 150)],
            false,
        );
        let ids = |app: &App| {
            app.conversations[chat]
                .messages
                .iter()
                .map(|m| m.id.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(&app), ["a", "reply", "mine"]);
        app.selection = Some((chat.into(), vec!["reply".into(), "mine".into()]));
        // The server's time for our message arrives as an update.
        events
            .send(Event::MessageUpdated(Box::new(restamped)))
            .unwrap();
        app.handle_events();
        assert_eq!(ids(&app), ["a", "mine", "reply"]);
        assert_eq!(
            app.selection.as_ref().map(|(_, ids)| ids.as_slice()),
            Some(["mine".to_owned(), "reply".to_owned()].as_slice()),
            "the selection follows the chat's order"
        );
    }

    /// Events of a hidden account are checked against that account's chats,
    /// so they must not prune the selection the window holds.
    #[test]
    fn a_hidden_accounts_event_keeps_the_window_selection() {
        let directory = tempfile::tempdir().unwrap();
        let (mut app, _commands) = two_accounts(directory.path());
        let chat = "15550003333@s.whatsapp.net";
        app.conversations
            .entry(chat.into())
            .or_default()
            .merge(vec![message(chat, "reply", 150)], false);
        app.selection = Some((chat.into(), vec!["reply".into()]));
        app.active = 1;
        app.events_hidden = true;
        app.apply_backend_event(Event::Info("Contact saved".into()), false);
        app.events_hidden = false;
        app.active = 0;
        assert_eq!(
            app.selection,
            Some((chat.into(), vec!["reply".into()])),
            "the window's selection survives another account's event"
        );
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
    fn the_chat_search_scans_once_after_the_typing_stops() {
        let directory = tempfile::tempdir().unwrap();
        let mut app = App::headless(AppDirs::under(directory.path()), Settings::default()).0;
        let (backend, mut commands) = Backend::recording();
        app.backend = backend;
        let ctx = egui::Context::default();
        for text in ["e", "en", "eng"] {
            app.apply(Action::Search(text.into()), &ctx);
            assert!(app.search_due.is_some(), "the scan waits for {text}");
        }
        assert!(
            commands.try_recv().is_err(),
            "no scan of the archive while the reader is still typing"
        );
        // The wait ends: the text in force is scanned for once.
        app.search_due = Some(Instant::now());
        app.tick(&ctx);
        let queries: Vec<_> = std::iter::from_fn(|| commands.try_recv().ok())
            .filter_map(|command| match command {
                Command::SearchMessages { query } => Some(query),
                _ => None,
            })
            .collect();
        assert_eq!(queries, ["eng"]);
        assert!(app.search_due.is_none());
    }

    /// An answer to an earlier scan cannot end the wait a later keystroke
    /// started: typing "a", "ab" and back to "a" keeps its scan.
    #[test]
    fn an_older_hit_list_keeps_the_scan_a_later_keystroke_asked_for() {
        let directory = tempfile::tempdir().unwrap();
        let mut app = App::headless(AppDirs::under(directory.path()), Settings::default()).0;
        let (backend, mut commands) = Backend::recording();
        app.backend = backend;
        let ctx = egui::Context::default();
        for text in ["a", "ab", "a"] {
            app.apply(Action::Search(text.into()), &ctx);
        }
        let due = app
            .search_due
            .expect("the last keystroke waits its turn before scanning");
        events_for(
            &mut app,
            Event::SearchHits {
                query: "a".into(),
                messages: Vec::new(),
            },
        );
        assert_eq!(
            app.search_due,
            Some(due),
            "the answer to the first scan keeps the waiting one"
        );
        app.search_due = Some(Instant::now());
        app.tick(&ctx);
        let queries: Vec<_> = std::iter::from_fn(|| commands.try_recv().ok())
            .filter_map(|command| match command {
                Command::SearchMessages { query } => Some(query),
                _ => None,
            })
            .collect();
        assert_eq!(queries, ["a"], "the waiting scan still runs");
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

    /// The listings the info panel asked the worker for: each chat and the
    /// request it carries.
    fn media_requests(
        commands: &mut tokio::sync::mpsc::UnboundedReceiver<Command>,
    ) -> Vec<(String, u64)> {
        std::iter::from_fn(|| commands.try_recv().ok())
            .filter_map(|command| match command {
                Command::ListChatShared { chat, request } => Some((chat, request)),
                _ => None,
            })
            .collect()
    }

    /// The worker's answer to `request`, listing `ids` as the pictures of
    /// `chat`.
    fn media_listing(request: u64, chat: &str, ids: &[&str]) -> Event {
        Event::ChatShared {
            request,
            result: Ok(crate::model::ChatMedia {
                chat: chat.into(),
                media: ids.iter().map(|id| message(chat, id, 1)).collect(),
                ..Default::default()
            }),
        }
    }

    #[test]
    fn the_info_panel_opens_beside_the_chat_and_asks_for_its_media_once() {
        use crate::model::{InfoPanel, InfoView, MediaTab};
        let mut app = app();
        let (backend, mut commands) = Backend::recording();
        app.backend = backend;
        let ctx = egui::Context::default();
        let chat = "1@s.whatsapp.net";
        app.chats.push(Chat::new(chat.into(), "Ada".into()));
        app.apply(Action::OpenChat(chat.into()), &ctx);
        app.contact_edit = Some(("Ada".into(), String::new()));
        app.apply(Action::OpenInfo(chat.into()), &ctx);
        assert_eq!(
            app.info,
            Some(InfoPanel {
                chat: chat.into(),
                view: InfoView::Overview,
            })
        );
        assert!(app.info_visible());
        assert!(app.dialog.is_none(), "a panel, not a modal");
        assert!(app.contact_edit.is_none(), "opening starts without an edit");
        assert_eq!(media_requests(&mut commands).len(), 1);
        assert!(app.info_media_pending);
        // Opened again for the same chat, from the header, it asks nothing
        // more and returns to the overview.
        app.apply(Action::ShowInfo(InfoView::Media(MediaTab::Docs)), &ctx);
        assert_eq!(
            app.info.as_ref().map(|info| info.view),
            Some(InfoView::Media(MediaTab::Docs))
        );
        app.apply(Action::OpenInfo(chat.into()), &ctx);
        assert!(media_requests(&mut commands).is_empty());
        assert_eq!(
            app.info.as_ref().map(|info| info.view),
            Some(InfoView::Overview)
        );
    }

    #[test]
    fn the_info_panel_and_the_search_pane_never_show_together() {
        let mut app = app();
        let ctx = egui::Context::default();
        let chat = "1@s.whatsapp.net";
        app.chats.push(Chat::new(chat.into(), "Ada".into()));
        app.apply(Action::OpenChat(chat.into()), &ctx);
        app.apply(Action::OpenInfo(chat.into()), &ctx);
        app.apply(Action::OpenChatSearch, &ctx);
        assert!(app.chat_search_visible());
        assert!(app.info.is_none(), "the pane takes the panel's place");
        app.apply(Action::OpenInfo(chat.into()), &ctx);
        assert!(app.info_visible());
        assert!(!app.chat_search_open, "and the panel the pane's");
    }

    #[test]
    fn the_info_panel_closes_with_a_chat_switch_a_close_or_a_leave() {
        let mut app = app();
        let ctx = egui::Context::default();
        let (ada, bob) = ("1@s.whatsapp.net", "2@s.whatsapp.net");
        app.chats.push(Chat::new(ada.into(), "Ada".into()));
        app.chats.push(Chat::new(bob.into(), "Bob".into()));
        app.apply(Action::OpenChat(ada.into()), &ctx);
        app.apply(Action::OpenInfo(ada.into()), &ctx);
        app.apply(Action::OpenChat(ada.into()), &ctx);
        assert!(app.info.is_some(), "the same chat again keeps it");
        app.apply(Action::OpenChat(bob.into()), &ctx);
        assert!(app.info.is_none(), "another chat closes it");
        // A member's panel over a group goes when that member's chat is
        // deleted; every panel goes when the chat under it is closed.
        app.apply(Action::OpenInfo(ada.into()), &ctx);
        app.leave_chat(ada);
        assert!(app.info.is_none());
        assert!(app.open_chat.is_some(), "the chat under it stays open");
        app.apply(Action::OpenInfo(bob.into()), &ctx);
        app.apply(Action::CloseChat, &ctx);
        assert!(app.info.is_none());
    }

    /// The chat under the panel can go without a close or a switch: the
    /// account is unlinked, or the open group is left while a member's panel
    /// is showing. The panel goes with it either way.
    #[test]
    fn the_info_panel_goes_when_the_chat_under_it_goes() {
        let mut app = app();
        let (backend, events) = Backend::detached();
        app.backend = backend;
        let ctx = egui::Context::default();
        let group = "1-2@g.us";
        let mira = "2@s.whatsapp.net";
        let mut chat = Chat::new(group.into(), "Rust".into());
        chat.participants = vec![mira.into()];
        app.chats.push(chat);
        app.apply(Action::OpenChat(group.into()), &ctx);
        app.apply(Action::OpenInfo(mira.into()), &ctx);
        app.leave_chat(group);
        assert!(app.open_chat.is_none());
        assert!(
            app.info.is_none(),
            "leaving the open group closes a member's panel"
        );
        app.chats.push(Chat::new(group.into(), "Rust".into()));
        app.apply(Action::OpenChat(group.into()), &ctx);
        app.apply(Action::OpenInfo(group.into()), &ctx);
        events.send(Event::Link(LinkStatus::LoggedOut)).unwrap();
        app.handle_events();
        assert!(app.open_chat.is_none());
        assert!(app.info.is_none(), "unlinking closes the panel");
    }

    /// A locked chat's media stays private: a member's panel opened from an
    /// unlocked group shows who they are, not what was shared with them.
    #[test]
    fn a_locked_members_media_stays_out_of_the_panel() {
        let mut app = app();
        let (backend, mut commands) = Backend::recording();
        app.backend = backend;
        let ctx = egui::Context::default();
        let group = "1-2@g.us";
        let mira = "2@s.whatsapp.net";
        let mut chat = Chat::new(group.into(), "Rust".into());
        chat.participants = vec![mira.into()];
        app.chats.push(chat);
        let mut private = Chat::new(mira.into(), "Mira".into());
        private.locked = true;
        app.chats.push(private);
        app.apply(Action::OpenChat(group.into()), &ctx);
        app.apply(Action::OpenInfo(mira.into()), &ctx);
        assert!(app.info.is_some(), "the identity still shows");
        assert!(!app.info_media_allowed(), "the media section does not");
        assert!(media_requests(&mut commands).is_empty(), "nothing is asked");
        assert!(!app.info_media_pending);
        // A change in that chat does not ask either.
        events_for(&mut app, Event::ChatSharedChanged(mira.into()));
        assert!(media_requests(&mut commands).is_empty());
    }

    /// Delivers one event through the app's channel.
    fn events_for(app: &mut App, event: Event) {
        let (backend, events) = Backend::detached();
        let recording = std::mem::replace(&mut app.backend, backend);
        events.send(event).unwrap();
        app.handle_events();
        app.backend = recording;
    }

    #[test]
    fn media_arrives_for_the_shown_chat_only_and_leaves_with_the_panel() {
        let mut app = app();
        let (backend, mut commands, events) = Backend::recording_with_events();
        app.backend = backend;
        let ctx = egui::Context::default();
        let (ada, bob) = ("1@s.whatsapp.net", "2@s.whatsapp.net");
        app.chats.push(Chat::new(ada.into(), "Ada".into()));
        app.apply(Action::OpenChat(ada.into()), &ctx);
        app.apply(Action::OpenInfo(ada.into()), &ctx);
        let request = media_requests(&mut commands)[0].1;
        events
            .send(media_listing(request, bob, &["theirs"]))
            .unwrap();
        app.handle_events();
        assert!(
            app.info_media.is_none(),
            "another chat's listing is ignored"
        );
        assert!(app.info_media_pending);
        events
            .send(media_listing(request, ada, &["photo"]))
            .unwrap();
        app.handle_events();
        assert_eq!(
            app.info_media.as_ref().map(|media| media.media.len()),
            Some(1)
        );
        assert!(!app.info_media_pending);
        app.apply(Action::CloseInfo, &ctx);
        assert!(app.info.is_none());
        assert!(app.info_media.is_none(), "closing drops the listing");
        assert!(app.focus_composer, "and hands the keyboard back");
    }

    /// A refreshed listing frees the pictures it no longer lists, and
    /// another chat's panel frees the rest, on the panel's next frame and
    /// not only once the panel closes.
    #[test]
    fn pictures_a_listing_drops_are_freed_while_the_panel_stays() {
        let mut app = app();
        let (backend, mut commands, events) = Backend::recording_with_events();
        app.backend = backend;
        let ctx = egui::Context::default();
        app.attach(&ctx);
        let (ada, bob) = ("1@s.whatsapp.net", "2@s.whatsapp.net");
        app.chats.push(Chat::new(ada.into(), "Ada".into()));
        app.chats.push(Chat::new(bob.into(), "Bob".into()));
        app.apply(Action::OpenChat(ada.into()), &ctx);
        app.apply(Action::OpenInfo(ada.into()), &ctx);
        let uri = |id| crate::ui::info::thumbnail_uri(ada, id);
        let registered = |uri: &str| ctx.try_load_bytes(uri).is_ok();
        let frame = |app: &mut App| {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                crate::ui::info::release_thumbnails(app, ui.ctx());
                crate::ui::info::show(app, ui);
            });
            output.textures_delta.clear();
        };
        let request = media_requests(&mut commands)[0].1;
        events
            .send(media_listing(request, ada, &["old", "kept"]))
            .unwrap();
        app.handle_events();
        for id in ["old", "kept"] {
            crate::image_cache::include(&ctx, &uri(id), b"picture");
        }
        events.send(Event::ChatSharedChanged(ada.into())).unwrap();
        app.handle_events();
        let request = media_requests(&mut commands)[0].1;
        events
            .send(media_listing(request, ada, &["kept", "new"]))
            .unwrap();
        app.handle_events();
        frame(&mut app);
        assert!(app.info_visible());
        assert!(!registered(&uri("old")), "the dropped picture is freed");
        assert!(registered(&uri("kept")), "a picture still listed stays");
        // A member's panel opened over the same chat lists another chat.
        app.apply(Action::OpenInfo(bob.into()), &ctx);
        frame(&mut app);
        assert!(app.info_visible());
        assert!(!registered(&uri("kept")), "the first chat's go too");
    }

    /// A chat's new picture arrives under the file name of its old one, and
    /// the window reads it afresh instead of keeping the old one it loaded.
    #[test]
    fn a_changed_chat_picture_is_read_again() {
        let mut app = app();
        let (backend, _commands, events) = Backend::recording_with_events();
        app.backend = backend;
        let ctx = egui::Context::default();
        app.attach(&ctx);
        let group = "fixture@g.us";
        let path = std::path::PathBuf::from("/cache/avatars/fixture.jpg");
        let uri = crate::util::image_uri(&path);
        let picture = |events: &std::sync::mpsc::Sender<Event>| {
            events
                .send(Event::Avatar {
                    id: group.into(),
                    full: false,
                    path: Some(path.clone()),
                })
                .unwrap();
        };
        picture(&events);
        app.background_frame(&ctx);
        crate::image_cache::include(&ctx, &uri, b"old picture");
        assert!(
            ctx.try_load_bytes(&uri).is_ok(),
            "the old picture is loaded"
        );
        picture(&events);
        app.background_frame(&ctx);
        let old = matches!(
            ctx.try_load_bytes(&uri),
            Ok(egui::load::BytesPoll::Ready { bytes, .. }) if bytes.as_ref() == b"old picture"
        );
        assert!(!old, "the old picture is gone, so the new file is read");
        assert_eq!(app.avatars.get(group), Some(&Some(path)));
    }

    /// A picture a listing drops and the next one lists again, before the
    /// panel draws a frame, stays: it is not freed while it is shown.
    #[test]
    fn a_picture_listed_again_before_a_frame_is_not_freed() {
        let mut app = app();
        let (backend, mut commands, events) = Backend::recording_with_events();
        app.backend = backend;
        let ctx = egui::Context::default();
        app.attach(&ctx);
        let ada = "1@s.whatsapp.net";
        app.chats.push(Chat::new(ada.into(), "Ada".into()));
        app.apply(Action::OpenChat(ada.into()), &ctx);
        app.apply(Action::OpenInfo(ada.into()), &ctx);
        let uri = crate::ui::info::thumbnail_uri(ada, "photo");
        for listed in [&["photo"][..], &[], &["photo"]] {
            events.send(Event::ChatSharedChanged(ada.into())).unwrap();
            app.handle_events();
            let request = media_requests(&mut commands)
                .last()
                .map(|(_, request)| *request)
                .unwrap_or(app.info_media_request);
            events.send(media_listing(request, ada, listed)).unwrap();
            app.handle_events();
        }
        crate::image_cache::include(&ctx, &uri, b"picture");
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            crate::ui::info::release_thumbnails(&mut app, ui.ctx());
            crate::ui::info::show(&mut app, ui);
        });
        output.textures_delta.clear();
        assert!(ctx.try_load_bytes(&uri).is_ok(), "the listed picture stays");
    }

    /// The pictures of a panel that closed are freed on the next frame even
    /// when that frame only draws the login or lock screen.
    #[test]
    fn a_closed_panels_pictures_are_freed_on_the_login_screen() {
        let mut app = app();
        let (backend, mut commands, events) = Backend::recording_with_events();
        app.backend = backend;
        let ctx = egui::Context::default();
        app.attach(&ctx);
        let ada = "1@s.whatsapp.net";
        app.chats.push(Chat::new(ada.into(), "Ada".into()));
        app.apply(Action::OpenChat(ada.into()), &ctx);
        app.apply(Action::OpenInfo(ada.into()), &ctx);
        let request = media_requests(&mut commands)[0].1;
        events
            .send(media_listing(request, ada, &["photo"]))
            .unwrap();
        app.handle_events();
        let uri = crate::ui::info::thumbnail_uri(ada, "photo");
        crate::image_cache::include(&ctx, &uri, b"picture");
        app.apply(Action::CloseInfo, &ctx);
        app.link = crate::backend::LinkStatus::LoggedOut;
        assert!(!app.is_linked(), "the frame draws the login screen");
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            crate::ui::show(&mut app, ui);
        });
        output.textures_delta.clear();
        assert!(ctx.try_load_bytes(&uri).is_err(), "the picture is freed");
    }

    /// Closing and reopening the panel asks again; the first request's
    /// answer, arriving late, must not clear the wait for the second.
    #[test]
    fn only_the_newest_requests_answer_is_accepted() {
        let mut app = app();
        let (backend, mut commands, events) = Backend::recording_with_events();
        app.backend = backend;
        let ctx = egui::Context::default();
        let ada = "1@s.whatsapp.net";
        app.chats.push(Chat::new(ada.into(), "Ada".into()));
        app.apply(Action::OpenChat(ada.into()), &ctx);
        app.apply(Action::OpenInfo(ada.into()), &ctx);
        app.apply(Action::CloseInfo, &ctx);
        app.apply(Action::OpenInfo(ada.into()), &ctx);
        let requests = media_requests(&mut commands);
        assert_eq!(requests.len(), 2);
        assert_ne!(requests[0].1, requests[1].1, "each request is its own");
        events
            .send(media_listing(requests[0].1, ada, &["old"]))
            .unwrap();
        app.handle_events();
        assert!(app.info_media.is_none(), "the first answer is stale");
        assert!(app.info_media_pending, "and the second is still due");
        events
            .send(media_listing(requests[1].1, ada, &["new"]))
            .unwrap();
        app.handle_events();
        assert_eq!(
            app.info_media
                .as_ref()
                .map(|media| media.media[0].id.as_str()),
            Some("new")
        );
        assert!(!app.info_media_pending);
    }

    /// A stale listing is one event of a batch; the ones after it in the
    /// same batch still count.
    #[test]
    fn a_stale_listing_does_not_drop_the_rest_of_its_batch() {
        let mut app = app();
        let (backend, events) = Backend::detached();
        app.backend = backend;
        let ctx = egui::Context::default();
        let ada = "1@s.whatsapp.net";
        app.chats.push(Chat::new(ada.into(), "Ada".into()));
        app.apply(Action::OpenChat(ada.into()), &ctx);
        app.apply(Action::OpenInfo(ada.into()), &ctx);
        events.send(media_listing(999, ada, &["stale"])).unwrap();
        events
            .send(Event::Presence {
                id: ada.into(),
                online: true,
                last_seen: None,
            })
            .unwrap();
        app.handle_events();
        assert!(app.info_media.is_none());
        assert!(
            app.presence
                .get(ada)
                .is_some_and(|presence| presence.online),
            "the event after the stale listing was applied"
        );
    }

    /// A listing the worker could not make is reported, not waited for, and
    /// can be asked for again.
    #[test]
    fn a_failed_listing_is_reported_and_can_be_asked_for_again() {
        let mut app = app();
        let (backend, mut commands, events) = Backend::recording_with_events();
        app.backend = backend;
        let ctx = egui::Context::default();
        let ada = "1@s.whatsapp.net";
        app.chats.push(Chat::new(ada.into(), "Ada".into()));
        app.apply(Action::OpenChat(ada.into()), &ctx);
        app.apply(Action::OpenInfo(ada.into()), &ctx);
        let first = media_requests(&mut commands)[0].1;
        events
            .send(Event::ChatShared {
                request: first,
                result: Err("disk".into()),
            })
            .unwrap();
        app.handle_events();
        assert!(!app.info_media_pending, "nothing is waited for");
        assert!(app.info_media_failed, "and the panel says so");
        app.apply(Action::RetryInfoMedia, &ctx);
        let again = media_requests(&mut commands);
        assert_eq!(again.len(), 1);
        assert_ne!(again[0].1, first);
        assert!(app.info_media_pending);
        events
            .send(media_listing(again[0].1, ada, &["photo"]))
            .unwrap();
        app.handle_events();
        assert!(!app.info_media_failed);
        assert_eq!(
            app.info_media.as_ref().map(|media| media.media.len()),
            Some(1)
        );
    }

    /// The worker says when what the panel lists changed in the archive;
    /// the app asks again on that word alone, one request at a time.
    /// Receipts, reactions, pages and even new messages in the transcript
    /// are not that word.
    #[test]
    fn the_workers_change_signal_refreshes_the_listing_one_request_at_a_time() {
        let mut app = app();
        let (backend, mut commands, events) = Backend::recording_with_events();
        app.backend = backend;
        let ctx = egui::Context::default();
        let (ada, bob) = ("1@s.whatsapp.net", "2@s.whatsapp.net");
        app.chats.push(Chat::new(ada.into(), "Ada".into()));
        let picture = |id: &str, timestamp: i64| Message {
            content: Content::Image {
                motion: None,
                caption: None,
                media: Media {
                    album: None,
                    mime: "image/jpeg".into(),
                    size: 1,
                    width: None,
                    height: None,
                    path: None,
                    state: MediaState::Idle,
                },
            },
            ..message(ada, id, timestamp)
        };
        app.conversations.insert(
            ada.into(),
            Conversation {
                requested: true,
                complete: true,
                messages: vec![picture("photo", 10)],
                ..Default::default()
            },
        );
        app.apply(Action::OpenChat(ada.into()), &ctx);
        app.apply(Action::OpenInfo(ada.into()), &ctx);
        let request = media_requests(&mut commands)[0].1;
        events
            .send(media_listing(request, ada, &["photo"]))
            .unwrap();
        app.handle_events();
        let mut read = picture("photo", 10);
        read.status = Delivery::Read;
        events.send(Event::MessageUpdated(Box::new(read))).unwrap();
        let mut liked = picture("photo", 10);
        liked.reactions.push(crate::model::Reaction {
            sender: ada.into(),
            from_me: false,
            emoji: "❤️".into(),
        });
        events.send(Event::MessageUpdated(Box::new(liked))).unwrap();
        events
            .send(Event::Messages {
                chat: ada.into(),
                messages: vec![picture("older", 1)],
                older: true,
                complete: true,
            })
            .unwrap();
        events
            .send(Event::Messages {
                chat: ada.into(),
                messages: vec![picture("fresh", 12)],
                older: false,
                complete: true,
            })
            .unwrap();
        events.send(Event::ChatSharedChanged(bob.into())).unwrap();
        app.handle_events();
        assert!(
            media_requests(&mut commands).is_empty(),
            "the transcript's events are not the archive's word"
        );
        events.send(Event::ChatSharedChanged(ada.into())).unwrap();
        events.send(Event::ChatSharedChanged(ada.into())).unwrap();
        app.handle_events();
        let asked = media_requests(&mut commands);
        assert_eq!(asked.len(), 1, "one request at a time");
        events
            .send(media_listing(asked[0].1, ada, &["fresh", "photo"]))
            .unwrap();
        app.handle_events();
        assert_eq!(
            media_requests(&mut commands).len(),
            1,
            "the change during the wait is asked for once it is over"
        );
    }

    /// A locked member's panel opened while the locked folder was open goes
    /// when lock authentication ends: the folder closes, the window goes, or
    /// the chat is locked under it. Its listing and the answer it was
    /// waiting for go with it.
    #[test]
    fn the_end_of_lock_authentication_takes_a_locked_panel_with_it() {
        use crate::model::{InfoView, MediaTab};
        let mut app = app();
        let (backend, mut commands, events) = Backend::recording_with_events();
        app.backend = backend;
        let ctx = egui::Context::default();
        let group = "1-2@g.us";
        let mira = "2@s.whatsapp.net";
        let mut chat = Chat::new(group.into(), "Rust".into());
        chat.participants = vec![mira.into()];
        app.chats.push(chat);
        let mut private = Chat::new(mira.into(), "Mira".into());
        private.locked = true;
        app.chats.push(private);
        app.settings.set_chat_lock_code(Some("fixture-code"));
        app.apply(Action::OpenChat(group.into()), &ctx);
        app.enter_locked_folder();
        assert!(app.locked_folder_open());
        app.apply(Action::OpenInfo(mira.into()), &ctx);
        let first = media_requests(&mut commands);
        assert_eq!(first.len(), 1, "listed while the folder is open");
        events
            .send(media_listing(first[0].1, mira, &["theirs"]))
            .unwrap();
        app.handle_events();
        app.apply(Action::ShowInfo(InfoView::Media(MediaTab::Media)), &ctx);
        assert!(app.info_media.is_some());
        app.close_locked_folder();
        assert!(app.info.is_none(), "the folder closing takes the panel");
        assert!(app.info_media.is_none(), "and its listing");
        assert!(!app.info_media_pending);
        assert_eq!(
            app.open_chat.as_deref(),
            Some(group),
            "the group stays open"
        );
        // A late answer to the request made while authenticated is refused.
        app.enter_locked_folder();
        app.apply(Action::OpenInfo(mira.into()), &ctx);
        let second = media_requests(&mut commands);
        assert_eq!(second.len(), 1);
        app.close_locked_folder();
        events
            .send(media_listing(second[0].1, mira, &["theirs"]))
            .unwrap();
        app.handle_events();
        assert!(app.info.is_none() && app.info_media.is_none());
        // The window going away ends the session the same way.
        app.enter_locked_folder();
        app.apply(Action::OpenInfo(mira.into()), &ctx);
        let third = media_requests(&mut commands);
        events
            .send(media_listing(third[0].1, mira, &["theirs"]))
            .unwrap();
        app.handle_events();
        assert!(app.info_media.is_some());
        app.window_gone();
        assert!(app.info.is_none(), "the window going takes the panel");
        assert!(app.info_media.is_none());
        // And a chat locked under its panel, with the folder closed, loses it.
        app.chat_mut(mira).expect("chat").locked = false;
        app.apply(Action::OpenInfo(mira.into()), &ctx);
        let fourth = media_requests(&mut commands);
        events
            .send(media_listing(fourth[0].1, mira, &["theirs"]))
            .unwrap();
        app.handle_events();
        assert!(app.info_media.is_some());
        let mut locked_now = app.chats.clone();
        locked_now
            .iter_mut()
            .find(|chat| chat.id == mira)
            .expect("chat")
            .locked = true;
        events.send(Event::Chats(locked_now)).unwrap();
        app.handle_events();
        assert!(app.info.is_none(), "locking the shown chat takes the panel");
        assert!(app.info_media.is_none());
    }

    /// A locked member's panel opened before authentication asks for its
    /// listing once the locked folder opens, or once the chat is unlocked;
    /// opening the same panel again asks only while nothing is loaded.
    #[test]
    fn a_locked_members_listing_loads_once_the_way_is_open() {
        let mut app = app();
        let (backend, mut commands, events) = Backend::recording_with_events();
        app.backend = backend;
        let ctx = egui::Context::default();
        let group = "1-2@g.us";
        let mira = "2@s.whatsapp.net";
        let mut chat = Chat::new(group.into(), "Rust".into());
        chat.participants = vec![mira.into()];
        app.chats.push(chat);
        let mut private = Chat::new(mira.into(), "Mira".into());
        private.locked = true;
        app.chats.push(private);
        app.settings.set_chat_lock_code(Some("fixture-code"));
        app.apply(Action::OpenChat(group.into()), &ctx);
        app.apply(Action::OpenInfo(mira.into()), &ctx);
        assert!(
            media_requests(&mut commands).is_empty(),
            "locked: not asked"
        );
        app.enter_locked_folder();
        let asked = media_requests(&mut commands);
        assert_eq!(asked.len(), 1, "authentication asks once");
        assert!(app.info_media_pending);
        events
            .send(media_listing(asked[0].1, mira, &["theirs"]))
            .unwrap();
        app.handle_events();
        app.apply(Action::OpenInfo(mira.into()), &ctx);
        assert!(
            media_requests(&mut commands).is_empty(),
            "reopening with a listing loaded asks nothing"
        );
        // Opened again with nothing loaded: the chat was unlocked meanwhile.
        app.close_locked_folder();
        assert!(app.info.is_none());
        app.chat_mut(mira).expect("chat").locked = true;
        app.apply(Action::OpenInfo(mira.into()), &ctx);
        assert!(media_requests(&mut commands).is_empty());
        app.chat_mut(mira).expect("chat").locked = false;
        app.apply(Action::OpenInfo(mira.into()), &ctx);
        assert_eq!(
            media_requests(&mut commands).len(),
            1,
            "reopening with nothing loaded asks once"
        );
        // The phone unlocking the chat opens the way too.
        app.apply(Action::CloseInfo, &ctx);
        app.chat_mut(mira).expect("chat").locked = true;
        app.apply(Action::OpenInfo(mira.into()), &ctx);
        media_requests(&mut commands);
        let mut unlocked = app.chats.clone();
        unlocked
            .iter_mut()
            .find(|chat| chat.id == mira)
            .expect("chat")
            .locked = false;
        events.send(Event::Chats(unlocked)).unwrap();
        app.handle_events();
        assert_eq!(
            media_requests(&mut commands).len(),
            1,
            "an unlocked chat is listed"
        );
    }

    /// The phone unlocking one chat arrives as that chat's update, not as
    /// a new chat list; a panel waiting for it asks once and shows the answer.
    #[test]
    fn a_chat_the_phone_unlocks_alone_is_listed_in_its_waiting_panel() {
        let mut app = app();
        let (backend, mut commands, events) = Backend::recording_with_events();
        app.backend = backend;
        let ctx = egui::Context::default();
        let group = "1-2@g.us";
        let mira = "2@s.whatsapp.net";
        let mut chat = Chat::new(group.into(), "Rust".into());
        chat.participants = vec![mira.into()];
        app.chats.push(chat);
        let mut private = Chat::new(mira.into(), "Mira".into());
        private.locked = true;
        app.chats.push(private);
        app.settings.set_chat_lock_code(Some("fixture-code"));
        app.apply(Action::OpenChat(group.into()), &ctx);
        app.apply(Action::OpenInfo(mira.into()), &ctx);
        assert!(
            media_requests(&mut commands).is_empty(),
            "locked: not asked"
        );
        let mut unlocked = app.chat(mira).expect("chat").clone();
        unlocked.locked = false;
        events.send(Event::ChatUpdated(Box::new(unlocked))).unwrap();
        app.handle_events();
        let asked = media_requests(&mut commands);
        assert_eq!(asked.len(), 1, "the unlocked chat is asked for once");
        events
            .send(media_listing(asked[0].1, mira, &["theirs"]))
            .unwrap();
        app.handle_events();
        assert_eq!(
            app.info_media.as_ref().map(|media| media.chat.as_str()),
            Some(mira),
            "and its answer is shown"
        );
    }

    /// Switching the panel to another chat invalidates the request still
    /// out for the first one, even when the new chat cannot be asked for:
    /// the old answer is refused, nothing stays pending, and once the new
    /// chat may be listed it is asked for exactly once.
    #[test]
    fn switching_the_panel_while_a_listing_is_due_invalidates_that_request() {
        let mut app = app();
        let (backend, mut commands, events) = Backend::recording_with_events();
        app.backend = backend;
        let ctx = egui::Context::default();
        let group = "1-2@g.us";
        let mira = "2@s.whatsapp.net";
        let mut chat = Chat::new(group.into(), "Rust".into());
        chat.participants = vec![mira.into()];
        app.chats.push(chat);
        let mut private = Chat::new(mira.into(), "Mira".into());
        private.locked = true;
        app.chats.push(private);
        app.settings.set_chat_lock_code(Some("fixture-code"));
        app.apply(Action::OpenChat(group.into()), &ctx);
        app.apply(Action::OpenInfo(group.into()), &ctx);
        let groups = media_requests(&mut commands);
        assert_eq!(groups.len(), 1);
        app.apply(Action::OpenInfo(mira.into()), &ctx);
        assert!(!app.info_media_pending, "nothing is due for the member");
        events
            .send(media_listing(groups[0].1, group, &["group-photo"]))
            .unwrap();
        app.handle_events();
        assert!(
            app.info_media.is_none(),
            "the group's late answer is refused"
        );
        assert!(!app.info_media_pending);
        app.enter_locked_folder();
        let members = media_requests(&mut commands);
        assert_eq!(members.len(), 1, "authentication asks for the member once");
        events
            .send(media_listing(members[0].1, mira, &["theirs"]))
            .unwrap();
        app.handle_events();
        assert_eq!(
            app.info_media.as_ref().map(|media| media.chat.as_str()),
            Some(mira),
            "and that answer is accepted"
        );
        assert!(!app.info_media_pending);
    }

    /// A download that finishes lands in the listing too, so a picture
    /// without a preview stops being blank once its file is there.
    #[test]
    fn a_finished_download_lands_in_the_listing() {
        let mut app = app();
        let (backend, mut commands, events) = Backend::recording_with_events();
        app.backend = backend;
        let ctx = egui::Context::default();
        let ada = "1@s.whatsapp.net";
        app.chats.push(Chat::new(ada.into(), "Ada".into()));
        let picture = Message {
            content: Content::Image {
                motion: None,
                caption: None,
                media: Media {
                    album: None,
                    mime: "image/jpeg".into(),
                    size: 1,
                    width: None,
                    height: None,
                    path: None,
                    state: MediaState::Idle,
                },
            },
            ..message(ada, "photo", 10)
        };
        app.conversations.insert(
            ada.into(),
            Conversation {
                requested: true,
                complete: true,
                messages: vec![picture.clone()],
                ..Default::default()
            },
        );
        app.apply(Action::OpenChat(ada.into()), &ctx);
        app.apply(Action::OpenInfo(ada.into()), &ctx);
        let request = media_requests(&mut commands)[0].1;
        events
            .send(Event::ChatShared {
                request,
                result: Ok(crate::model::ChatMedia {
                    chat: ada.into(),
                    media: vec![picture],
                    ..Default::default()
                }),
            })
            .unwrap();
        events
            .send(Event::Media {
                card: None,
                chat: ada.into(),
                message: "photo".into(),
                result: Ok(PathBuf::from("photo.jpg")),
            })
            .unwrap();
        app.handle_events();
        let listed = &app.info_media.as_ref().expect("listed").media[0];
        assert_eq!(
            listed
                .content
                .media()
                .and_then(|media| media.path.as_deref()),
            Some(std::path::Path::new("photo.jpg"))
        );
        assert!(media_requests(&mut commands).is_empty(), "no query for it");
    }

    #[test]
    fn matching_contacts_are_people_not_yet_talked_to() {
        let mut app = app();
        app.me = Some("490000000000@s.whatsapp.net".into());
        let contact = |id: &str, name: &str| crate::model::Contact {
            id: id.into(),
            full_name: Some(name.into()),
            first_name: None,
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
        // The app badge counts it too, as WhatsApp does.
        assert_eq!(app.unread_chat_count(), 1);
    }

    /// Opening a chat where someone mentioned us shows the mention, and
    /// reading it takes the "@" away (#487).
    #[test]
    fn opening_a_chat_goes_to_its_unread_mention() {
        let mut app = app();
        let ctx = egui::Context::default();
        let id = "1-2@g.us";
        let mut club = Chat::new(id.into(), "Club".into());
        club.unread = 3;
        club.unread_mention = Some("m2".into());
        app.chats = vec![club, Chat::new("3@s.whatsapp.net".into(), "Ada".into())];
        app.conversations.entry(id.into()).or_default().merge(
            (1..=4).map(|n| message(id, &format!("m{n}"), n)).collect(),
            false,
        );
        app.apply(Action::OpenChat(id.into()), &ctx);
        assert_eq!(app.open_chat.as_deref(), Some(id));
        assert_eq!(app.scroll_anchor.as_deref(), Some("m2"));
        let chat = app.chat(id).unwrap();
        assert_eq!((chat.unread, chat.unread_mention.as_ref()), (0, None));

        // Without a mention, a chat opens as before.
        app.apply(Action::OpenChat("3@s.whatsapp.net".into()), &ctx);
        app.scroll_anchor = None;
        app.apply(Action::OpenChat(id.into()), &ctx);
        assert_eq!(app.scroll_anchor, None);
    }

    #[test]
    fn the_app_badge_counts_unread_chats_not_messages() {
        let mut app = app();
        let mut busy = Chat::new("1@s.whatsapp.net".into(), "Ada".into());
        busy.unread = 7;
        let mut quiet = Chat::new("2@s.whatsapp.net".into(), "Grace".into());
        quiet.unread = 1;
        let mut archived = Chat::new("3@s.whatsapp.net".into(), "Old".into());
        archived.archived = true;
        archived.unread = 4;
        let mut muted = Chat::new("4@s.whatsapp.net".into(), "Loud".into());
        muted.muted_until = Some(i64::MAX);
        muted.unread = 9;
        let read = Chat::new("5@s.whatsapp.net".into(), "Done".into());
        app.chats = vec![busy, quiet, archived, muted, read];
        assert_eq!(app.unread_chat_count(), 2);
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
            Chat {
                last_activity: 1,
                ..Chat::new("4@s.whatsapp.net".into(), "Dee".into())
            },
        ];
        app.chat_filter = ChatFilter::Favorites;
        app.drafts
            .insert("1@s.whatsapp.net".into(), "Unsent favorite".into());
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

    /// Pins retain their order above text and attachment drafts; filtered
    /// chats do not reappear merely because they contain a draft.
    #[test]
    fn pins_stay_above_eligible_text_and_attachment_drafts() {
        let mut app = app();
        let mut chat = |id: &str, activity| {
            let mut chat = Chat::new(id.into(), id.into());
            chat.last_activity = activity;
            app.chats.push(chat);
        };
        chat("pin-old", 1);
        chat("pin-new", 2);
        chat("recent", 50);
        chat("text", 20);
        chat("file", 10);
        chat("locked", 100);
        chat("archived", 100);
        app.chats[0].pinned = true;
        app.chats[0].pinned_at = 10;
        app.chats[1].pinned = true;
        app.chats[1].pinned_at = 20;
        app.chats[5].locked = true;
        app.chats[6].archived = true;
        for id in ["pin-old", "text", "locked", "archived"] {
            app.drafts.insert(id.into(), "Unsent text".into());
        }
        app.open_chat("file".into());
        app.stage_files(vec!["Synthetic document.pdf".into()]);
        app.apply(Action::CloseChat, &egui::Context::default());
        let ids: Vec<_> = app
            .visible_chats()
            .iter()
            .map(|chat| chat.id.clone())
            .collect();
        assert_eq!(ids, ["pin-new", "pin-old", "text", "file", "recent"]);
    }

    /// A new draft moves only on leaving; reopening and clearing a saved
    /// draft cannot move its row while the reader is composing.
    #[test]
    fn draft_priority_changes_on_leaving_instead_of_typing_or_opening() {
        let mut app = app();
        let mut recent = Chat::new("recent".into(), "Recent".into());
        recent.last_activity = 20;
        // An older chat, not a stranger never written to: those leave the
        // list once their draft is gone.
        let mut draft = Chat::new("draft".into(), "Draft".into());
        draft.last_activity = 10;
        app.chats = vec![recent, draft];
        let ids = |app: &App| -> Vec<String> {
            app.visible_chats()
                .iter()
                .map(|chat| chat.id.clone())
                .collect()
        };
        app.open_chat("draft".into());
        app.composer = "Typing a new draft".into();
        assert_eq!(ids(&app), ["recent", "draft"]);
        app.open_chat("recent".into());
        assert_eq!(ids(&app), ["draft", "recent"]);
        app.open_chat("draft".into());
        assert_eq!(ids(&app), ["draft", "recent"]);
        app.composer.clear();
        assert_eq!(ids(&app), ["draft", "recent"]);
        app.open_chat("recent".into());
        assert_eq!(ids(&app), ["recent", "draft"]);
    }

    /// Archive drafts can establish the initial startup priority, but a
    /// delayed answer cannot promote an active attachment composer.
    #[test]
    fn late_archived_draft_keeps_an_attachment_composers_priority() {
        for staged_attachment in [false, true] {
            let mut app = app();
            let mut recent = Chat::new("recent".into(), "Recent".into());
            recent.last_activity = 20;
            app.chats = vec![recent, Chat::new("draft".into(), "Draft".into())];
            app.open_chat("draft".into());
            if staged_attachment {
                app.stage_files(vec!["Synthetic document.pdf".into()]);
            }
            assert_eq!(app.visible_chats()[0].id, "recent");
            events_for(
                &mut app,
                Event::Drafts(vec![("draft".into(), "Saved archive text".into())]),
            );
            assert_eq!(app.composer, "Saved archive text");
            assert_eq!(app.pending.len(), usize::from(staged_attachment));
            assert_eq!(
                app.visible_chats()[0].id,
                if staged_attachment { "recent" } else { "draft" },
                "only the untouched startup composer gains draft priority"
            );
            app.open_chat("recent".into());
            assert_eq!(app.visible_chats()[0].id, "draft");
        }
    }

    /// Identical chat ids in two accounts never share draft priority, and
    /// restoring an account keeps its parked draft in the same priority group.
    #[test]
    fn draft_priority_stays_with_its_account() {
        let directory = tempfile::tempdir().unwrap();
        let (mut app, _) = two_accounts(directory.path());
        for account in &mut app.accounts {
            let mut recent = Chat::new("recent".into(), "Recent".into());
            recent.last_activity = 20;
            account.chats = vec![recent, Chat::new("draft".into(), "Draft".into())];
        }
        let first = app.account().id.clone();
        let second = app.accounts[1].id.clone();
        app.drafts
            .insert("draft".into(), "First account only".into());
        app.open_chat("draft".into());
        assert_eq!(app.visible_chats()[0].id, "draft");
        app.switch_account(&second);
        app.open_chat("draft".into());
        assert_eq!(app.visible_chats()[0].id, "recent");
        app.switch_account(&first);
        assert_eq!(app.visible_chats()[0].id, "draft");
        assert_eq!(app.composer, "First account only");
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
        ada.last_activity = 2;
        let bob = Chat::new("2@s.whatsapp.net".into(), "Bob".into());
        app.chats = vec![ada, bob];
        let mut cy = Chat::new("3@s.whatsapp.net".into(), "Cy".into());
        cy.unread = 1;
        cy.last_activity = 1;
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
    fn people_never_written_to_stay_out_of_the_list_until_searched() {
        let mut app = app();
        let mut ada = Chat::new("1@s.whatsapp.net".into(), "Ada".into());
        ada.last_activity = 10;
        let stranger = Chat::new("2@s.whatsapp.net".into(), "Stranger".into());
        let group = Chat::new("3@g.us".into(), "Group".into());
        app.chats = vec![ada, stranger, group];
        let names = |app: &App| {
            app.visible_chats()
                .iter()
                .map(|chat| chat.name.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(names(&app), ["Ada", "Group"], "groups stay listed");
        app.search = "Stran".into();
        assert_eq!(names(&app), ["Stranger"]);
        app.search.clear();
        app.open_chat("2@s.whatsapp.net".into());
        assert_eq!(names(&app), ["Ada", "Stranger", "Group"]);
        // A draft left unsent keeps the chat listed once it is closed, and
        // parked drafts come before other chats.
        app.open_chat = None;
        app.drafts.insert("2@s.whatsapp.net".into(), "hello".into());
        assert_eq!(names(&app), ["Stranger", "Ada", "Group"]);
    }

    #[test]
    fn strangers_the_reader_kept_stay_listed_and_counted() {
        let mut app = app();
        let stranger = |id: &str| Chat::new(id.into(), id.into());
        let mut pinned = stranger("1@s.whatsapp.net");
        pinned.pinned = true;
        let mut favorite = stranger("2@s.whatsapp.net");
        favorite.favorite = true;
        let mut unread = stranger("3@s.whatsapp.net");
        unread.unread = 1;
        let mut locked = stranger("4@s.whatsapp.net");
        locked.locked = true;
        let mut labeled = stranger("6@s.whatsapp.net");
        labeled.labels = vec!["label".into()];
        let mut archived = stranger("5@s.whatsapp.net");
        archived.archived = true;
        app.chats = vec![pinned, favorite, unread, locked, labeled, archived];
        let unlisted: Vec<bool> = app
            .chats
            .iter()
            .map(|chat| app.unlisted_stranger(chat))
            .collect();
        assert_eq!(unlisted, [false, false, false, false, false, true]);
        assert_eq!(
            app.archived_count(),
            0,
            "Archived does not count a hidden stranger"
        );
        assert_eq!(
            app.locked_count(),
            1,
            "the locked folder keeps a locked stranger"
        );
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
        assert_eq!(app.unread_chat_count(), 1);
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
        app.handle_chat_updated(chat, true);
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
            first_name: None,
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
    fn a_chat_with_its_own_speed_keeps_it_and_leaves_settings_alone() {
        let mut app = app();
        let ctx = egui::Context::default();
        let (own, other) = ("1@s.whatsapp.net", "2@s.whatsapp.net");
        app.chats.push(Chat::new(own.into(), "Ada".into()));
        app.chats.push(Chat::new(other.into(), "Bob".into()));
        app.apply(Action::SetVoiceSpeed(2.0), &ctx);

        app.open_chat(own.into());
        app.apply(
            Action::SetChatVoiceSpeed {
                chat: own.into(),
                speed: Some(1.0),
            },
            &ctx,
        );
        assert_eq!(app.voice_speed_in(Some(own)), 1.0);
        assert_eq!(app.player.speed(), 1.0);
        assert_eq!(app.voice_speed_in(Some(other)), 2.0);

        // Settings leave a chat with its own speed alone.
        app.apply(
            Action::SetChatVoiceSpeed {
                chat: own.into(),
                speed: Some(1.5),
            },
            &ctx,
        );
        app.open_chat(other.into());
        app.apply(Action::SetVoiceSpeed(1.25), &ctx);
        assert_eq!(app.settings.voice_speed, 1.25);
        assert_eq!(app.chat(own).unwrap().voice_speed, Some(1.5));

        // A clip from that chat still playing in another chat keeps the
        // chat's speed when Settings change, and follows the chat's own.
        app.voice_clip_chat = Some(own.into());
        app.apply(Action::SetVoiceSpeed(2.0), &ctx);
        assert_eq!(app.settings.voice_speed, 2.0);
        assert_eq!(app.player.speed(), 1.5);
        app.apply(
            Action::SetChatVoiceSpeed {
                chat: own.into(),
                speed: Some(1.0),
            },
            &ctx,
        );
        assert_eq!(app.player.speed(), 1.0);
        app.voice_clip_chat = None;
        app.apply(Action::SetVoiceSpeed(1.25), &ctx);

        // Back to Default, the chat follows Settings again.
        app.apply(
            Action::SetChatVoiceSpeed {
                chat: own.into(),
                speed: None,
            },
            &ctx,
        );
        assert_eq!(app.voice_speed_in(Some(own)), 1.25);
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

    fn group_chat(members: usize) -> Chat {
        let mut chat = Chat::new("123@g.us".into(), "Group".into());
        chat.participants = (0..members)
            .map(|index| format!("{index}@s.whatsapp.net"))
            .collect();
        chat
    }

    #[test]
    fn a_localized_everyone_word_becomes_the_visible_token() {
        let mut app = app();
        app.locale = Locale::PortugueseBrazil;
        app.chats.push(group_chat(3));

        let (text, mentions) =
            app.encode_composer_mentions("123@g.us", "oi @todos, mail@todos.com @todosX".into());

        assert_eq!(text, "oi @all, mail@todos.com @todosX");
        assert!(mentions.is_empty());
    }

    #[test]
    fn english_leaves_a_portuguese_everyone_word_alone() {
        let mut app = app();
        app.chats.push(group_chat(3));

        let (text, _) = app.encode_composer_mentions("123@g.us", "hi @todos and @all".into());

        assert_eq!(text, "hi @todos and @all");
    }

    #[test]
    fn everyone_is_not_rewritten_when_the_group_disallows_it() {
        let mut app = app();
        app.locale = Locale::PortugueseBrazil;
        let chat = group_chat(33);
        assert!(!chat.can_mention_everyone());
        app.chats.push(chat);

        let (text, _) = app.encode_composer_mentions("123@g.us", "oi @todos @all".into());

        assert_eq!(text, "oi @todos @all");
    }

    #[test]
    fn everyone_is_offered_only_when_the_group_allows_it() {
        let mut app = app();
        app.locale = Locale::PortugueseBrazil;
        let small = group_chat(3);
        let offered = app.mention_candidates(&small, "to");
        assert_eq!(offered[0].0, crate::mentions::ALL_ID);
        assert!(
            app.mention_candidates(&small, "mira")
                .iter()
                .all(|(id, _)| id != crate::mentions::ALL_ID)
        );

        let mut large = group_chat(33);
        assert!(
            app.mention_candidates(&large, "")
                .iter()
                .all(|(id, _)| id != crate::mentions::ALL_ID)
        );
        large.admin = true;
        assert_eq!(
            app.mention_candidates(&large, "all")[0].0,
            crate::mentions::ALL_ID
        );

        let direct = Chat::new("1@s.whatsapp.net".into(), "Ada".into());
        assert!(app.mention_candidates(&direct, "").is_empty());
    }

    #[test]
    fn choosing_everyone_inserts_the_visible_token() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.chats.push(group_chat(3));
        app.open_chat("123@g.us".into());
        app.composer = "@tod".into();

        app.apply(
            Action::InsertMention {
                id: crate::mentions::ALL_ID.into(),
                name: "all".into(),
                start: 0,
                end: app.composer.len(),
            },
            &ctx,
        );

        assert_eq!(app.composer, "@all ");
        assert!(app.composer_mentions.is_empty());

        app.composer = "@all".into();
        app.chats[0].participants = (0..33)
            .map(|index| format!("{index}@s.whatsapp.net"))
            .collect();
        app.apply(
            Action::InsertMention {
                id: crate::mentions::ALL_ID.into(),
                name: "all".into(),
                start: 0,
                end: app.composer.len(),
            },
            &ctx,
        );
        assert_eq!(
            app.composer, "@all",
            "a large group hides it from non-admins"
        );
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

    /// Plain egui text (a label, a tooltip) gets its emoji in colour from
    /// the plugin, while a line laid out with placeholders gets exactly one
    /// picture, its own, and no second one from the plugin.
    #[test]
    fn plain_text_emoji_are_coloured_without_painting_placeholders_twice() {
        use egui::epaint::{Shape, TextureId};
        fn pictures(shape: &Shape) -> usize {
            match shape {
                Shape::Vec(shapes) => shapes.iter().map(pictures).sum(),
                Shape::Mesh(mesh) if mesh.texture_id != TextureId::default() => 1,
                _ => 0,
            }
        }
        assert!(
            crate::emoji::available(),
            "the bundled font is always there"
        );
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        let count = |draw: &dyn Fn(&mut egui::Ui)| {
            let mut total = 0;
            // A new area is measured in its first frame and drawn after.
            for _ in 0..2 {
                let mut output = ctx.run_ui(egui::RawInput::default(), |ui| draw(ui));
                output.textures_delta.clear();
                total = output
                    .shapes
                    .iter()
                    .map(|clipped| pictures(&clipped.shape))
                    .sum::<usize>();
            }
            total
        };
        assert_eq!(count(&|ui| drop(ui.label("plain 😀"))), 1, "a label");
        assert_eq!(
            count(&|ui| {
                egui::Area::new(egui::Id::new("tip"))
                    .order(egui::Order::Tooltip)
                    .show(ui.ctx(), |ui| ui.label("tip 👍🏽"));
            }),
            1,
            "a tooltip"
        );
        assert_eq!(
            count(&|ui| {
                let line = crate::ui::widgets::line(
                    ui,
                    "placeholder 😀",
                    egui::FontId::proportional(14.0),
                    egui::Color32::WHITE,
                    300.0,
                    1,
                );
                line.paint(ui, egui::pos2(0.0, 0.0), egui::Color32::WHITE);
            }),
            1,
            "a placeholder line"
        );
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
                first_name: None,
                push_name: None,
            },
        );
        assert_eq!(app.display_name("1@s.whatsapp.net"), "Ada");
        assert_eq!(
            app.display_name("393331234567@s.whatsapp.net"),
            "+39 333 123 4567"
        );
        assert_eq!(app.display_name("42@lid"), "Unknown");
        app.contacts.insert(
            "42@lid".into(),
            Contact {
                id: "42@lid".into(),
                full_name: None,
                first_name: None,
                push_name: Some("Bob".into()),
            },
        );
        assert_eq!(app.display_name("42@lid"), "~Bob");
        app.me = Some("42@lid".into());
        assert_eq!(app.display_name("42@lid"), "You");
    }

    #[test]
    fn an_archived_phone_name_is_grouped_again() {
        // Unnamed chats keep the number as it was formatted when they were
        // filed, which used to split every country code after two digits.
        let mut app = app();
        app.chats.push(Chat::new(
            "15551234567@s.whatsapp.net".into(),
            "+15 551 234 567".into(),
        ));
        app.chats.push(Chat::new(
            "5511999999999@s.whatsapp.net".into(),
            "+55 (11) 99999-9999".into(),
        ));
        assert_eq!(
            app.display_name("15551234567@s.whatsapp.net"),
            "+1 555 123 4567"
        );
        assert_eq!(
            app.display_name("5511999999999@s.whatsapp.net"),
            "+55 (11) 99999-9999"
        );
        // Without a number in the id there is nothing to format afresh.
        app.chats
            .push(Chat::new("42@lid".into(), "+1 555 123 4567".into()));
        assert_eq!(app.display_name("42@lid"), "+1 555 123 4567");
        app.chats.push(Chat::new("43@lid".into(), "43".into()));
        assert_eq!(app.display_name("43@lid"), "Unknown");
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
                first_name: None,
                push_name: Some("Ada".into()),
            },
        );
        app.contacts.insert(
            "2@s.whatsapp.net".into(),
            Contact {
                id: "2@s.whatsapp.net".into(),
                full_name: None,
                first_name: None,
                push_name: Some("Bob".into()),
            },
        );
        app
    }

    #[test]
    fn group_members_go_by_their_whole_saved_first_name() {
        let mut app = app();
        let mut chat = Chat::new("fixture@g.us".into(), "Group".into());
        for (index, full_name, first_name) in [
            (0, "My Dih", Some("My Dih")),
            (1, "Grace Hopper", Some("Grace")),
            (2, "Mary Ann Evans", None),
            (3, "Stray Name", Some("")),
        ] {
            let id = format!("1555000001{index}@s.whatsapp.net");
            app.contacts.insert(
                id.clone(),
                Contact {
                    id: id.clone(),
                    full_name: Some(full_name.into()),
                    first_name: first_name.map(Into::into),
                    push_name: None,
                },
            );
            chat.participants.push(id);
        }
        // Without a saved first name (a profile name, or a contact synced
        // before first names were kept) the line keeps the first word.
        chat.participants.push("2@s.whatsapp.net".into());
        app.contacts.get_mut("2@s.whatsapp.net").unwrap().push_name = Some("Bob Builder".into());
        assert_eq!(
            app.participant_names(&chat),
            "Bob, Grace, Mary, My Dih, Stray"
        );
        // The sender before a group's last message goes by the same name.
        assert_eq!(
            app.short_name("15550000010@s.whatsapp.net", "My Dih"),
            "My Dih"
        );
        assert_eq!(
            app.short_name("15550000011@s.whatsapp.net", "Grace Hopper"),
            "Grace"
        );
        assert_eq!(app.short_name("2@s.whatsapp.net", "~Bob Builder"), "~Bob");
        assert_eq!(
            app.short_name("3@s.whatsapp.net", "+1 555 0100"),
            "+1 555 0100"
        );
        let me = app.me.clone().unwrap();
        assert_eq!(app.short_name(&me, "You"), "You");
    }

    #[test]
    fn the_contact_editor_opens_with_the_saved_first_name_whole() {
        let mut app = app();
        let ctx = egui::Context::default();
        let saved = |app: &mut App, id: &str, full: &str, first: Option<&str>| {
            app.contacts.insert(
                id.into(),
                Contact {
                    id: id.into(),
                    full_name: Some(full.into()),
                    first_name: first.map(Into::into),
                    push_name: None,
                },
            );
        };
        let edit = |app: &mut App, id: &str, name: &str| {
            app.apply(
                Action::EditContact {
                    id: id.into(),
                    name: name.into(),
                },
                &ctx,
            );
            app.contact_edit.take().expect("the editor opens")
        };
        let pair = |first: &str, last: &str| (first.to_owned(), last.to_owned());
        // #314: a first name of two words, without and with a last name.
        saved(
            &mut app,
            "15550000020@s.whatsapp.net",
            "first second",
            Some("first second"),
        );
        assert_eq!(
            edit(&mut app, "15550000020@s.whatsapp.net", "first second"),
            pair("first second", "")
        );
        saved(
            &mut app,
            "15550000021@s.whatsapp.net",
            "first second third",
            Some("first second"),
        );
        assert_eq!(
            edit(&mut app, "15550000021@s.whatsapp.net", "first second third"),
            pair("first second", "third")
        );
        // Without a saved first name the whole name stays first, so saving
        // it unchanged cannot shorten the first name to one word.
        saved(&mut app, "15550000022@s.whatsapp.net", "My Dih", None);
        assert_eq!(
            edit(&mut app, "15550000022@s.whatsapp.net", "My Dih"),
            pair("My Dih", "")
        );
        assert_eq!(
            compose_name("My Dih", ""),
            (Some("My Dih".into()), Some("My Dih".into()))
        );
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
                    first_name: name.split(' ').next().map(Into::into),
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

    /// A starred message names its chat by the chat's title: a group without
    /// a subject is named after its members, as in the chat list, not
    /// "Unknown" as a person lookup of a group id would have it.
    #[test]
    fn a_chat_is_titled_by_id_as_the_chat_list_titles_it() {
        let mut app = app();
        let mut chat = Chat::new("fixture@g.us".into(), String::new());
        let id = "15550000001@s.whatsapp.net".to_owned();
        app.contacts.insert(
            id.clone(),
            Contact {
                id: id.clone(),
                full_name: Some("Ada Lovelace".into()),
                first_name: Some("Ada".into()),
                push_name: None,
            },
        );
        chat.participants.push(id);
        chat.participants.push(app.me.clone().unwrap());
        app.chats.push(chat.clone());
        assert_eq!(app.display_name(&chat.id), "Unknown");
        assert_eq!(app.chat_title_of(&chat.id), app.participant_names(&chat));
        assert_eq!(app.chat_title_of(&chat.id), "Ada, You");
        assert_eq!(app.chat_title_of("gone@g.us"), "Unknown");
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

    /// Checks that mention names and preview tokens resolve consistently in synthetic messages.
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
            history_order: None,
            edited: false,
            mentions: vec![MentionRef {
                user: "15550001111".into(),
                id: "15550001111@s.whatsapp.net".into(),
            }],
            forwarded: false,
            thumbnail: None,
        };
        assert_eq!(app.message_text(&message), "ciao @Carmine");

        // A caption mentioning a privacy id is named through the message's
        // mentions, which carry the canonical id, in notifications and
        // search results alike.
        let photo = Message {
            content: Content::Image {
                motion: None,
                caption: Some("@987654321012345 looks sharp".into()),
                media: Media {
                    mime: "image/jpeg".into(),
                    size: 100,
                    width: None,
                    height: None,
                    path: None,
                    state: MediaState::Idle,
                    album: None,
                },
            },
            mentions: vec![MentionRef {
                user: "987654321012345".into(),
                id: "15550001111@s.whatsapp.net".into(),
            }],
            ..message
        };
        assert_eq!(app.message_text(&photo), "Photo: @Carmine looks sharp");
        assert_eq!(
            app.preview_line("@987654321012345 looks sharp", &photo),
            "@Carmine looks sharp"
        );
    }
}

#[cfg(test)]
mod app_lock_tests {
    use super::*;
    use crate::app_lock::{Clock, Form, FormError, FormMode};
    use std::sync::{Arc, Mutex};

    const PASSWORD: &str = "open-sesame";
    const CHAT: &str = "1@s.whatsapp.net";

    fn settings(password: Option<&str>) -> Settings {
        Settings {
            app_lock_hash: password.map(crate::app_lock::verifier),
            ..Settings::default()
        }
    }

    fn app_with(settings: Settings) -> App {
        let root = std::env::temp_dir().join(format!(
            "zapfast-app-lock-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        App::headless(AppDirs::under(&root), settings).0
    }

    /// An app with the lock set but not locked, as after unlocking.
    fn unlocked_app() -> App {
        let mut app = app_with(settings(Some(PASSWORD)));
        app.app_lock.release();
        app
    }

    fn manual_clock(app: &mut App) -> Arc<Mutex<Instant>> {
        let now = Arc::new(Mutex::new(Instant::now()));
        let read = Arc::clone(&now);
        let clock: Clock = Arc::new(move || *read.lock().unwrap());
        app.app_lock.set_clock(clock);
        now
    }

    fn advance(now: &Mutex<Instant>, by: Duration) {
        *now.lock().unwrap() += by;
    }

    /// Waits for the password thread and applies its answer.
    fn finish(app: &mut App, ctx: &egui::Context) {
        let outcome = app.app_lock.wait().expect("a finished check");
        app.app_lock_outcome(outcome);
        app.apply_actions(ctx);
    }

    fn try_password(app: &mut App, ctx: &egui::Context, password: &str) {
        app.app_lock.entry = password.into();
        app.apply(Action::UnlockApp, ctx);
        finish(app, ctx);
    }

    /// A link names a chat the lock hides, and the reader asked for it from
    /// outside, so it waits for the unlock rather than being dropped.
    #[test]
    fn a_link_waits_for_the_unlock() {
        let ctx = egui::Context::default();
        let mut app = app_with(settings(Some(PASSWORD)));
        assert!(app.app_lock.is_locked());
        app.chats = vec![Chat::new(CHAT.into(), "Ada Lovelace".into())];

        app.apply(
            Action::StartChat {
                dismiss_dialog: true,
                id: CHAT.into(),
                name: "+1 555 000 0000".into(),
            },
            &ctx,
        );
        app.apply(
            Action::PrefillComposer {
                chat: CHAT.into(),
                text: "Invoice 123 is attached".into(),
            },
            &ctx,
        );

        assert_eq!(app.open_chat, None, "the lock hides the chat");
        assert!(
            app.composer.is_empty(),
            "and hides the template: {:?}",
            app.composer
        );
        assert_eq!(
            app.app_lock.deferred,
            vec![crate::app_lock::Deferred::Chat(crate::target::Request {
                chat: CHAT.into(),
                text: Some("Invoice 123 is attached".into()),
            })],
            "the request is kept whole"
        );

        try_password(&mut app, &ctx, PASSWORD);

        assert_eq!(
            app.open_chat.as_deref(),
            Some(CHAT),
            "the unlock opens what the link asked for"
        );
        assert_eq!(app.composer, "Invoice 123 is attached");
        assert!(app.app_lock.deferred.is_empty(), "and forgets it");
    }

    /// A group invite from the command line waits for the unlock too, while
    /// any other link stays shut out.
    #[test]
    fn a_group_invite_waits_for_the_unlock() {
        let ctx = egui::Context::default();
        let mut app = app_with(settings(Some(PASSWORD)));
        assert!(app.app_lock.is_locked());

        app.apply(Action::OpenUrl("https://example.com/".into()), &ctx);
        assert!(app.app_lock.deferred.is_empty(), "other links are dropped");

        app.apply(
            Action::OpenUrl("https://chat.whatsapp.com/AbCdEf1234567890XyZ".into()),
            &ctx,
        );
        assert!(app.invite.is_none(), "the lock hides the invite");
        assert_eq!(
            app.app_lock.deferred,
            vec![crate::app_lock::Deferred::Invite(
                "AbCdEf1234567890XyZ".into()
            )]
        );

        try_password(&mut app, &ctx, PASSWORD);

        assert_eq!(
            app.invite.as_ref().map(|invite| invite.code.as_str()),
            Some("AbCdEf1234567890XyZ"),
            "the unlock shows the invite"
        );
        assert!(app.app_lock.deferred.is_empty(), "and forgets it");
    }

    /// An invite and a chat asked for while locked open in the order they
    /// arrived, so the later one is what the unlock leaves on screen.
    #[test]
    fn requests_held_by_the_lock_replay_in_the_order_they_arrived() {
        const INVITE: &str = "https://chat.whatsapp.com/AbCdEf1234567890XyZ";
        let ctx = egui::Context::default();

        // The invite first, then the chat: the chat ends up open.
        let mut app = app_with(settings(Some(PASSWORD)));
        app.chats = vec![Chat::new(CHAT.into(), "Ada Lovelace".into())];
        app.apply(Action::OpenUrl(INVITE.into()), &ctx);
        app.apply(
            Action::StartChat {
                id: CHAT.into(),
                name: "+1 555 000 0000".into(),
                dismiss_dialog: true,
            },
            &ctx,
        );
        try_password(&mut app, &ctx, PASSWORD);
        assert_eq!(app.open_chat.as_deref(), Some(CHAT));
        assert!(app.dialog.is_none(), "the chat came after the invite");

        // The chat first, then the invite: the invite stays up over it.
        let mut app = app_with(settings(Some(PASSWORD)));
        app.chats = vec![Chat::new(CHAT.into(), "Ada Lovelace".into())];
        app.apply(
            Action::StartChat {
                id: CHAT.into(),
                name: "+1 555 000 0000".into(),
                dismiss_dialog: true,
            },
            &ctx,
        );
        app.apply(Action::OpenUrl(INVITE.into()), &ctx);
        try_password(&mut app, &ctx, PASSWORD);
        assert_eq!(app.open_chat.as_deref(), Some(CHAT));
        assert_eq!(
            app.invite.as_ref().map(|invite| invite.code.as_str()),
            Some("AbCdEf1234567890XyZ")
        );
        assert!(
            app.dialog == Some(Dialog::JoinGroup),
            "the invite came after the chat"
        );

        // A second request of a kind replaces the first, keeping its turn
        // after everything asked for in between.
        let mut app = app_with(settings(Some(PASSWORD)));
        app.apply(Action::OpenUrl(INVITE.into()), &ctx);
        app.apply(
            Action::StartChat {
                id: CHAT.into(),
                name: "+1 555 000 0000".into(),
                dismiss_dialog: true,
            },
            &ctx,
        );
        app.apply(Action::OpenUrl(INVITE.into()), &ctx);
        assert_eq!(
            app.app_lock.deferred,
            vec![
                crate::app_lock::Deferred::Chat(crate::target::Request {
                    chat: CHAT.into(),
                    text: None,
                }),
                crate::app_lock::Deferred::Invite("AbCdEf1234567890XyZ".into()),
            ]
        );
    }

    /// Without a password there is nothing to wait for: the link opens.
    #[test]
    fn a_link_opens_at_once_when_no_lock_is_set() {
        let ctx = egui::Context::default();
        let mut app = unlocked_app();
        app.chats = vec![Chat::new(CHAT.into(), "Ada Lovelace".into())];

        app.apply(
            Action::StartChat {
                dismiss_dialog: true,
                id: CHAT.into(),
                name: "+1 555 000 0000".into(),
            },
            &ctx,
        );
        app.apply(
            Action::PrefillComposer {
                chat: CHAT.into(),
                text: "Invoice 123 is attached".into(),
            },
            &ctx,
        );

        assert_eq!(app.open_chat.as_deref(), Some(CHAT));
        assert_eq!(app.composer, "Invoice 123 is attached");
    }

    /// Forgetting the lock forgets what was waiting on it: after unlinking
    /// there is no chat left to open.
    #[test]
    fn forgetting_the_lock_forgets_a_waiting_link() {
        let ctx = egui::Context::default();
        let mut app = app_with(settings(Some(PASSWORD)));
        app.apply(
            Action::StartChat {
                dismiss_dialog: true,
                id: CHAT.into(),
                name: "+1 555 0000".into(),
            },
            &ctx,
        );
        assert!(!app.app_lock.deferred.is_empty());

        app.forget_app_lock();

        assert!(app.app_lock.deferred.is_empty());
    }

    fn incoming(app: &mut App) -> Message {
        let mut chat = Chat::new(CHAT.into(), "Ada Lovelace".into());
        chat.unread = 1;
        app.chats = vec![chat];
        Message {
            id: "m1".into(),
            chat: CHAT.into(),
            sender: CHAT.into(),
            sender_name: None,
            from_me: false,
            timestamp: crate::util::now(),
            content: Content::text("the engine is ready"),
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
    fn it_starts_locked_only_with_a_password() {
        assert!(app_with(settings(Some(PASSWORD))).app_lock.is_locked());
        assert!(!app_with(settings(None)).app_lock.is_locked());
    }

    #[test]
    fn inactivity_locks_after_the_chosen_time_even_while_hidden() {
        let ctx = egui::Context::default();
        let mut app = unlocked_app();
        app.settings.app_lock_after = crate::settings::AutoLock::OneMinute;
        let now = manual_clock(&mut app);
        advance(&now, Duration::from_secs(59));
        app.background_frame(&ctx);
        assert!(!app.app_lock.is_locked());
        // Input restarts the count.
        app.app_lock.note_input();
        advance(&now, Duration::from_secs(59));
        app.background_frame(&ctx);
        assert!(!app.app_lock.is_locked());
        // Hidden in the tray, only the headless loop runs, and it locks.
        app.window_gone();
        advance(&now, Duration::from_secs(2));
        app.background_frame(&ctx);
        assert!(app.app_lock.is_locked());
    }

    #[test]
    fn a_longer_choice_waits_longer_and_no_password_never_locks() {
        let ctx = egui::Context::default();
        let mut app = unlocked_app();
        app.apply(
            Action::SetAutoLock(crate::settings::AutoLock::OneHour),
            &ctx,
        );
        let now = manual_clock(&mut app);
        advance(&now, Duration::from_secs(59 * 60));
        app.background_frame(&ctx);
        assert!(!app.app_lock.is_locked());
        advance(&now, Duration::from_secs(60));
        app.background_frame(&ctx);
        assert!(app.app_lock.is_locked());

        let mut app = app_with(settings(None));
        let now = manual_clock(&mut app);
        advance(&now, Duration::from_secs(24 * 60 * 60));
        app.background_frame(&ctx);
        app.apply(Action::LockApp, &ctx);
        assert!(!app.app_lock.is_locked());
    }

    #[test]
    fn window_input_counts_as_use() {
        let ctx = egui::Context::default();
        let mut app = unlocked_app();
        app.settings.app_lock_after = crate::settings::AutoLock::OneMinute;
        app.attach(&ctx);
        let now = manual_clock(&mut app);
        advance(&now, Duration::from_secs(50));
        let mut output = ctx.run_ui(
            egui::RawInput {
                events: vec![egui::Event::PointerMoved(egui::pos2(10.0, 10.0))],
                ..Default::default()
            },
            |ui| app.frame_ui(ui),
        );
        output.textures_delta.clear();
        advance(&now, Duration::from_secs(50));
        app.background_frame(&ctx);
        assert!(!app.app_lock.is_locked());
    }

    #[test]
    fn lock_now_stops_media_and_closes_the_locked_folder() {
        let ctx = egui::Context::default();
        let mut app = unlocked_app();
        app.locked_folder = true;
        app.chat_lock_entry = "half typed".into();
        app.window_focused = true;
        app.apply(Action::LockApp, &ctx);
        assert!(app.app_lock.is_locked());
        assert!(!app.locked_folder);
        assert!(app.chat_lock_entry.is_empty());
        assert!(!app.window_focused);
    }

    #[test]
    fn a_right_password_unlocks_and_wrong_ones_back_off() {
        let ctx = egui::Context::default();
        let mut app = app_with(settings(Some(PASSWORD)));
        let now = manual_clock(&mut app);
        for _ in 0..3 {
            try_password(&mut app, &ctx, "not the password");
            assert!(app.app_lock.is_locked());
            assert!(app.app_lock.wrong);
        }
        assert_eq!(app.app_lock.wait_left(), Some(Duration::from_secs(1)));
        // A try during the wait is not made at all.
        app.app_lock.entry = PASSWORD.into();
        app.apply(Action::UnlockApp, &ctx);
        assert!(!app.app_lock.checking());
        advance(&now, Duration::from_secs(1));
        try_password(&mut app, &ctx, PASSWORD);
        assert!(!app.app_lock.is_locked());
        assert!(app.app_lock.entry.is_empty());
    }

    #[test]
    fn while_locked_actions_do_nothing_and_a_clicked_notification_waits() {
        let ctx = egui::Context::default();
        let mut app = app_with(settings(Some(PASSWORD)));
        app.chats = vec![Chat::new(CHAT.into(), "Ada".into())];
        app.open_chat = None;
        app.apply(Action::OpenChat(CHAT.into()), &ctx);
        app.apply(Action::Open(Page::Settings), &ctx);
        app.apply(Action::ShowDialog(Dialog::NewChat), &ctx);
        app.apply(Action::Unlink, &ctx);
        assert_eq!(app.open_chat, None);
        assert_eq!(app.page, Page::Chats);
        assert_eq!(app.dialog, None);

        app.notification_opens
            .lock()
            .unwrap()
            .push(crate::notify::NotificationTarget {
                account: crate::model::AccountId::first(),
                chat: CHAT.into(),
                message: Some("m1".into()),
            });
        app.handle_notification_opens();
        app.apply_actions(&ctx);
        assert_eq!(app.open_chat, None, "not while locked");
        try_password(&mut app, &ctx, PASSWORD);
        assert_eq!(app.open_chat.as_deref(), Some(CHAT), "opened once unlocked");
    }

    fn attention_requested(ctx: &egui::Context) -> bool {
        ctx.viewport(|viewport| {
            viewport
                .commands
                .iter()
                .any(|command| matches!(command, egui::ViewportCommand::RequestUserAttention(_)))
        })
    }

    /// A notification highlights the taskbar entry of an open, unfocused
    /// window, once; a hidden window has no entry to highlight.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_notification_highlights_an_unfocused_window_in_the_taskbar() {
        let ctx = egui::Context::default();
        let mut app = unlocked_app();
        let message = incoming(&mut app);
        app.window_hidden = false;
        app.window_focused = false;
        app.maybe_notify(CHAT, &message);
        app.request_attention_on(&ctx, false);
        assert!(attention_requested(&ctx));

        let ctx = egui::Context::default();
        app.request_attention_on(&ctx, false);
        assert!(!attention_requested(&ctx), "only once per notification");

        app.window_hidden = true;
        app.maybe_notify(CHAT, &message);
        app.request_attention_on(&ctx, false);
        assert!(!attention_requested(&ctx));
    }

    /// GNOME shows its own "is ready" notice for a window that asks for
    /// attention, on top of the message's notification (#466).
    #[cfg(target_os = "linux")]
    #[test]
    fn gnome_gets_no_attention_request() {
        let ctx = egui::Context::default();
        let mut app = unlocked_app();
        let message = incoming(&mut app);
        app.window_hidden = false;
        app.window_focused = false;
        app.maybe_notify(CHAT, &message);
        app.request_attention_on(&ctx, true);
        assert!(!attention_requested(&ctx));
        // The request is spent, not kept for another desktop later.
        let ctx = egui::Context::default();
        app.request_attention_on(&ctx, false);
        assert!(!attention_requested(&ctx));
    }

    /// Hyprland is known by its instance signature and the desktop name
    /// together, in any entry of the desktop list.
    #[test]
    fn hyprland_needs_its_signature_and_its_desktop_name() {
        assert!(hyprland(true, Some("Hyprland")));
        assert!(hyprland(true, Some("Hyprland:wlroots")));
        assert!(hyprland(true, Some("hyprland")));
        // A signature left over, or a desktop name alone, is not enough.
        assert!(!hyprland(true, Some("KDE")));
        assert!(!hyprland(true, None));
        assert!(!hyprland(false, Some("Hyprland")));
    }

    /// Behind the app lock `window_focused` stays false, but a lock screen
    /// that has the focus is no window to point at.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_focused_lock_screen_is_not_highlighted() {
        let ctx = egui::Context::default();
        let mut input = egui::RawInput::default();
        input.viewports.insert(
            egui::ViewportId::ROOT,
            egui::ViewportInfo {
                focused: Some(true),
                ..Default::default()
            },
        );
        ctx.run_ui(input, |_| {}).textures_delta.clear();
        let mut app = unlocked_app();
        let message = incoming(&mut app);
        app.window_hidden = false;
        app.lock_app();
        assert!(!app.window_focused);
        app.maybe_notify(CHAT, &message);
        app.request_attention_on(&ctx, false);
        assert!(!attention_requested(&ctx));
    }

    #[test]
    fn reading_the_chat_asks_for_no_attention() {
        let ctx = egui::Context::default();
        let mut app = unlocked_app();
        let message = incoming(&mut app);
        app.window_hidden = false;
        app.window_focused = true;
        app.page = Page::Chats;
        app.open_chat = Some(CHAT.into());
        app.maybe_notify(CHAT, &message);
        app.request_attention_on(&ctx, false);
        assert!(!attention_requested(&ctx));
    }

    #[test]
    fn notifications_while_locked_carry_no_chat_sender_text_or_picture() {
        let mut app = unlocked_app();
        let message = incoming(&mut app);
        app.chats[0].notification_sound = Some(crate::settings::NotificationSound::Alert);
        app.maybe_notify(CHAT, &message);
        let shown = app.notifications.shown.last().unwrap().clone();
        assert_eq!(shown.title, "Ada Lovelace");
        assert_eq!(shown.body, "the engine is ready");

        app.lock_app();
        app.maybe_notify(CHAT, &message);
        let shown = app.notifications.shown.last().unwrap();
        assert_eq!(shown.title, "ZapFast");
        assert_eq!(shown.body, "New message");
        assert_eq!(shown.picture, None);
        assert_eq!(
            shown.sound, app.settings.message_sound,
            "the chat's own sound would name it"
        );
    }

    #[test]
    fn messages_do_not_notify_while_the_display_is_off() {
        let mut app = unlocked_app();
        let message = incoming(&mut app);
        app.power.set_conditions(fastframe_power::Conditions {
            display_off: true,
            ..Default::default()
        });

        app.maybe_notify(CHAT, &message);
        assert!(app.notifications.shown.is_empty());
        assert!(!app.wants_attention);

        app.power.set_conditions(Default::default());
        app.maybe_notify(CHAT, &message);
        assert_eq!(app.notifications.shown.len(), 1);
    }

    #[test]
    fn notifications_without_content_say_only_new_message() {
        let mut app = unlocked_app();
        let message = incoming(&mut app);
        app.chats[0].notification_sound = Some(crate::settings::NotificationSound::Alert);
        app.settings.notification_content = false;
        app.maybe_notify(CHAT, &message);
        let shown = app.notifications.shown.last().unwrap().clone();
        assert_eq!(shown.title, "ZapFast");
        assert_eq!(shown.body, "New message");
        assert_eq!(shown.picture, None);
        assert_eq!(shown.sound, app.settings.message_sound);

        app.settings.notification_content = true;
        app.maybe_notify(CHAT, &message);
        let shown = app.notifications.shown.last().unwrap();
        assert_eq!(shown.title, "Ada Lovelace");
        assert_eq!(shown.body, "the engine is ready");
    }

    #[test]
    fn limited_direct_chats_notify_once_until_the_chat_is_opened() {
        let mut app = unlocked_app();
        let message = incoming(&mut app);
        app.maybe_notify(CHAT, &message);
        app.maybe_notify(CHAT, &message);
        assert_eq!(app.notifications.shown.len(), 2, "off by default");

        app.account_mut().settings.limit_notifications = crate::settings::NotificationLimit::Direct;
        app.notifications.shown.clear();
        app.maybe_notify(CHAT, &message);
        app.maybe_notify(CHAT, &message);
        assert_eq!(app.notifications.shown.len(), 1);

        app.open_chat(CHAT.into());
        app.chats[0].unread = 1;
        app.maybe_notify(CHAT, &message);
        assert_eq!(app.notifications.shown.len(), 2, "opening reset the wait");
        app.maybe_notify(CHAT, &message);
        assert_eq!(app.notifications.shown.len(), 2);
    }

    #[test]
    fn a_chat_read_on_another_device_notifies_again() {
        let mut app = unlocked_app();
        let message = incoming(&mut app);
        app.account_mut().settings.limit_notifications = crate::settings::NotificationLimit::Direct;
        app.maybe_notify(CHAT, &message);
        app.maybe_notify(CHAT, &message);
        assert_eq!(app.notifications.shown.len(), 1);

        // An update that leaves the chat unread keeps the wait.
        let still_unread = app.chats[0].clone();
        app.handle_chat_updated(still_unread, true);
        app.maybe_notify(CHAT, &message);
        assert_eq!(app.notifications.shown.len(), 1);

        // The phone reads the chat, and the sync drops its unread count to 0.
        let mut read = app.chats[0].clone();
        read.unread = 0;
        app.handle_chat_updated(read, true);
        app.chats[0].unread = 1;
        app.maybe_notify(CHAT, &message);
        assert_eq!(
            app.notifications.shown.len(),
            2,
            "reading elsewhere reset the wait"
        );
        app.maybe_notify(CHAT, &message);
        assert_eq!(app.notifications.shown.len(), 2);
    }

    #[test]
    fn the_direct_and_group_limits_are_independent() {
        const GROUP: &str = "2@g.us";
        let mut app = unlocked_app();
        let direct = incoming(&mut app);
        let mut group_chat = Chat::new(GROUP.into(), "Engineers".into());
        group_chat.unread = 1;
        app.chats.push(group_chat);
        let group = Message {
            id: "m2".into(),
            chat: GROUP.into(),
            sender: CHAT.into(),
            ..direct.clone()
        };

        app.account_mut().settings.limit_notifications = crate::settings::NotificationLimit::Group;
        app.maybe_notify(CHAT, &direct);
        app.maybe_notify(CHAT, &direct);
        app.maybe_notify(GROUP, &group);
        app.maybe_notify(GROUP, &group);
        assert_eq!(
            app.notifications.shown.len(),
            3,
            "direct chats are unlimited, the group notified once"
        );

        app.account_mut().settings.limit_notifications = crate::settings::NotificationLimit::Direct;
        app.notifications.shown.clear();
        app.maybe_notify(GROUP, &group);
        app.maybe_notify(GROUP, &group);
        app.maybe_notify(CHAT, &direct);
        app.maybe_notify(CHAT, &direct);
        assert_eq!(app.notifications.shown.len(), 3);
    }

    #[test]
    fn mentions_and_replies_to_us_bypass_the_group_limit() {
        const GROUP: &str = "2@g.us";
        let mut app = unlocked_app();
        app.me = Some("15550001111@s.whatsapp.net".into());
        let direct = incoming(&mut app);
        let mut group_chat = Chat::new(GROUP.into(), "Engineers".into());
        group_chat.unread = 1;
        app.chats.push(group_chat);
        let plain = Message {
            id: "m2".into(),
            chat: GROUP.into(),
            sender: CHAT.into(),
            ..direct
        };
        let mut mention = plain.clone();
        mention.mentions = vec![crate::model::MentionRef {
            user: "15550001111".into(),
            id: "15550001111@s.whatsapp.net".into(),
        }];
        let mut reply = plain.clone();
        reply.quoted = Some(crate::model::Quoted {
            labelled: None,
            chat: None,
            id: "earlier".into(),
            sender: "15550001111@s.whatsapp.net".into(),
            sender_name: None,
            summary: "earlier".into(),
            mentions: Vec::new(),
        });

        app.account_mut().settings.limit_notifications = crate::settings::NotificationLimit::Group;
        app.maybe_notify(GROUP, &mention);
        app.maybe_notify(GROUP, &mention);
        app.maybe_notify(GROUP, &reply);
        assert_eq!(app.notifications.shown.len(), 3, "addressed to us");

        // They neither start nor use the wait: the first plain message
        // still notifies, and the second does not.
        app.maybe_notify(GROUP, &plain);
        app.maybe_notify(GROUP, &plain);
        assert_eq!(app.notifications.shown.len(), 4);
        app.maybe_notify(GROUP, &mention);
        assert_eq!(app.notifications.shown.len(), 5, "still addressed to us");
    }

    #[test]
    fn a_limited_chat_is_limited_while_the_app_is_locked_too() {
        let mut app = unlocked_app();
        let message = incoming(&mut app);
        app.account_mut().settings.limit_notifications = crate::settings::NotificationLimit::Direct;
        app.maybe_notify(CHAT, &message);
        app.lock_app();
        app.maybe_notify(CHAT, &message);
        assert_eq!(app.notifications.shown.len(), 1);
    }

    #[test]
    fn messages_arriving_while_locked_stay_unread() {
        let ctx = egui::Context::default();
        let mut app = app_with(settings(Some(PASSWORD)));
        app.open_chat = Some(CHAT.into());
        app.window_focused = true;
        app.background_frame(&ctx);
        let mut chat = Chat::new(CHAT.into(), "Ada".into());
        chat.unread = 1;
        app.handle_chat_updated(chat, true);
        assert_eq!(app.chat(CHAT).unwrap().unread, 1);
    }

    #[test]
    fn setting_changing_and_turning_off_the_password() {
        let ctx = egui::Context::default();
        let mut app = app_with(settings(None));
        // Changing needs a password to change.
        app.apply(Action::AppLockForm(Some(FormMode::Change)), &ctx);
        assert_eq!(app.app_lock.form, None);
        app.apply(Action::AppLockForm(Some(FormMode::Set)), &ctx);
        let form = app.app_lock.form.as_mut().unwrap();
        form.new = "first-password".into();
        form.confirm = "first-password".into();
        app.apply(Action::SubmitAppLockForm, &ctx);
        finish(&mut app, &ctx);
        assert_eq!(app.app_lock.form, None);
        let first = app.settings.app_lock_hash.clone().unwrap();
        assert!(crate::app_lock::verifies(&first, "first-password"));
        assert!(
            !app.app_lock.is_locked(),
            "setting a password does not lock"
        );

        // A second Set cannot replace it without the current password.
        app.apply(Action::AppLockForm(Some(FormMode::Set)), &ctx);
        assert_eq!(app.app_lock.form, None);

        let mut change = Form::new(FormMode::Change);
        change.current = "wrong".into();
        change.new = "second-password".into();
        change.confirm = "second-password".into();
        app.app_lock.form = Some(change.clone());
        app.apply(Action::SubmitAppLockForm, &ctx);
        finish(&mut app, &ctx);
        let form = app.app_lock.form.as_ref().unwrap();
        assert_eq!(form.error, Some(FormError::WrongCurrent));
        assert!(!form.busy);
        assert_eq!(app.settings.app_lock_hash.as_deref(), Some(first.as_str()));

        change.current = "first-password".into();
        app.app_lock.form = Some(change);
        app.apply(Action::SubmitAppLockForm, &ctx);
        finish(&mut app, &ctx);
        let second = app.settings.app_lock_hash.clone().unwrap();
        assert!(crate::app_lock::verifies(&second, "second-password"));

        // Turning off requires the password too.
        app.apply(Action::AppLockForm(Some(FormMode::TurnOff)), &ctx);
        app.app_lock.form.as_mut().unwrap().current = "first-password".into();
        app.apply(Action::SubmitAppLockForm, &ctx);
        finish(&mut app, &ctx);
        assert!(app.settings.app_lock_hash.is_some());
        app.app_lock.form.as_mut().unwrap().current = "second-password".into();
        app.apply(Action::SubmitAppLockForm, &ctx);
        finish(&mut app, &ctx);
        assert_eq!(app.settings.app_lock_hash, None);
        assert_eq!(app.app_lock.form, None);
    }

    #[test]
    fn forgetting_the_password_unlocks_only_after_unlinking() {
        let ctx = egui::Context::default();
        let mut app = app_with(settings(Some(PASSWORD)));
        app.apply(Action::ForgotAppPassword(true), &ctx);
        assert_eq!(
            app.app_lock.forgetting,
            crate::app_lock::Forgetting::Confirming
        );
        app.apply(Action::UnlinkLockedApp, &ctx);
        assert_eq!(
            app.app_lock.forgetting,
            crate::app_lock::Forgetting::Unlinking
        );
        assert!(app.app_lock.is_locked(), "still locked until unlinked");
        app.handle_link(LinkStatus::LoggedOut, true);
        assert!(!app.app_lock.is_locked());
        assert_eq!(app.settings.app_lock_hash, None);
    }

    #[test]
    fn the_lock_shortcut_needs_a_password() {
        let ctx = egui::Context::default();
        let press = |app: &mut App| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    events: vec![egui::Event::Key {
                        key: egui::Key::L,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                    }],
                    ..Default::default()
                },
                |ui| crate::ui::keys::handle(app, ui.ctx()),
            );
            output.textures_delta.clear();
            std::mem::take(&mut app.actions)
        };
        let mut app = unlocked_app();
        let actions = press(&mut app);
        assert!(actions.contains(&Action::LockApp), "{actions:?}");
        assert!(!actions.contains(&Action::FocusComposer));
        let mut app = app_with(settings(None));
        assert!(!press(&mut app).contains(&Action::LockApp));
    }
}
