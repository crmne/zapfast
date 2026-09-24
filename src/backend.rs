//! Channel bridge between the UI and asynchronous runtime.
//!
//! A dedicated tokio runtime owns the WhatsApp connection, archive, and media
//! work. Commands and events cross channels, and events wake the UI.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;

use crate::model::{Chat, ChatId, Contact, Gif, GifError, Message, PollDraft, StickerPack};
use crate::paths::AppDirs;

// Re-exported so the picker can detect pasted Signal pack links.
mod read_sync;
pub(crate) mod sticker_import;
mod sticker_maker;
pub(crate) mod sticker_store;
mod worker;
pub use worker::{PINNED_CHATS, PLUS_PINNED_CHATS};

/// Phone-link state.
#[derive(Clone, Debug, PartialEq)]
pub enum LinkStatus {
    Starting,
    /// Waiting for QR scanning or pairing-code acceptance.
    Unlinked {
        qr: Option<String>,
        pair_code: Option<String>,
        pairing_phone: Option<String>,
    },
    Connecting,
    Connected,
    /// Connection dropped and automatic reconnection is active.
    Disconnected {
        reason: String,
    },
    /// Device unlinked by the phone.
    LoggedOut,
    Failed(String),
}

impl LinkStatus {
    pub fn is_connected(&self) -> bool {
        matches!(self, Self::Connected)
    }

    /// Stable, non-sensitive description suitable for the desktop log.
    pub(crate) fn log_label(&self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Unlinked { .. } => "unlinked",
            Self::Connecting => "connecting",
            Self::Connected => "connected",
            Self::Disconnected { .. } => "disconnected",
            Self::LoggedOut => "logged out",
            Self::Failed(_) => "failed",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::LinkStatus;

    #[test]
    fn backend_waits_for_window_acknowledgement_before_touching_storage() {
        let directory = tempfile::tempdir().unwrap();
        let dirs = crate::paths::AppDirs::under(directory.path());
        let mut backend = super::Backend::spawn(dirs.clone(), super::Waker::default());
        assert!(!dirs.session_db().exists());
        assert!(!dirs.archive_db().exists());
        // Closing before a first frame must cancel startup without connecting
        // or hanging while joining the waiting worker.
        backend.shutdown();
        assert!(!dirs.session_db().exists());
        assert!(!dirs.archive_db().exists());
    }

    #[test]
    fn link_logs_redact_pairing_credentials() {
        let qr = "qr-payload-that-links-an-account";
        let code = "12345678";
        let phone = "573001234567";
        let status = LinkStatus::Unlinked {
            qr: Some(qr.into()),
            pair_code: Some(code.into()),
            pairing_phone: Some(phone.into()),
        };

        // The previous Debug formatting leaked every field into zapfast.log.
        let previous = format!("link: {status:?}");
        assert!(previous.contains(qr));
        assert!(previous.contains(code));
        assert!(previous.contains(phone));

        let current = format!("link: {}", status.log_label());
        assert_eq!(current, "link: unlinked");
        assert!(!current.contains(qr));
        assert!(!current.contains(code));
        assert!(!current.contains(phone));
    }
}

/// Oldest loaded message timestamp and id used as a page boundary.
pub type PageKey = (i64, String);

#[derive(Clone, Debug)]
pub struct CreatedPoll {
    pub id: String,
    pub secret: Vec<u8>,
    pub creator: String,
    pub recipients: Vec<String>,
}

#[derive(Clone, Debug)]
pub enum Command {
    RefreshPoll {
        chat: ChatId,
        message: String,
    },
    PollHistoryFailed {
        chat: ChatId,
        message: String,
        requested: std::time::Instant,
    },
    CreatePoll {
        chat: ChatId,
        draft: PollDraft,
    },
    PollCreated {
        chat: ChatId,
        draft: PollDraft,
        result: Result<CreatedPoll, String>,
    },
    VotePoll {
        chat: ChatId,
        message: String,
        choices: Vec<usize>,
    },
    PollVoted {
        chat: ChatId,
        message: String,
        choices: Vec<usize>,
        at: i64,
        result: Result<String, String>,
    },
    PollDecoded {
        vote: crate::archive::PollVote,
        choices: Option<Vec<usize>>,
    },
    SendText {
        chat: ChatId,
        text: String,
        quoting: Option<String>,
        mentions: Vec<String>,
    },
    ReplyInteractive {
        chat: ChatId,
        message: String,
        button: usize,
        choice: Option<usize>,
    },
    /// Forwards an archived message to another chat.
    Forward {
        from_chat: ChatId,
        message: String,
        to_chat: ChatId,
    },
    /// Updates our typing state in a chat.
    Composing {
        chat: ChatId,
        composing: bool,
    },
    /// Stores the open chat's unsent text, so it survives a restart.
    SaveDraft {
        chat: ChatId,
        text: String,
    },
    /// Marks a visible chat read and optionally sends receipts.
    MarkRead {
        chat: ChatId,
        receipts: bool,
    },
    /// Marks a chat with nothing pending as unread, here and on the phone.
    MarkUnread(ChatId),
    /// Follows one of our group messages' receipts while "Message info" is
    /// open, or stops following with `None`.
    WatchReceipts(Option<(ChatId, String)>),
    /// Result of a private read-state update to the other linked devices.
    ReadSyncFinished {
        chat: ChatId,
        through: i64,
        success: bool,
    },
    /// Result of an unread mark sent to the other linked devices, keyed by
    /// when the mark was made.
    UnreadSyncFinished {
        chat: ChatId,
        marked_at: i64,
        success: bool,
    },
    /// Loads archived chat messages before an optional boundary.
    LoadChat {
        chat: ChatId,
        before: Option<PageKey>,
    },
    /// Requests messages before the archive's earliest message.
    FetchOlder(ChatId),
    Download {
        card: Option<usize>,
        chat: ChatId,
        message: String,
    },
    /// Requests a profile picture; `full` selects the info-dialog size.
    FetchAvatar {
        id: String,
        full: bool,
    },
    /// Loads archived messages from `id` through the current page.
    LoadUntil {
        chat: ChatId,
        id: String,
        before: PageKey,
    },
    /// Searches visible archived message text.
    SearchMessages {
        query: String,
    },
    /// Searches one chat, optionally inside a Unix-second day range.
    SearchChatMessages {
        chat: ChatId,
        query: String,
        from: Option<i64>,
        until: Option<i64>,
    },
    /// Creates an archive chat before its first message is sent.
    EnsureChat {
        chat: ChatId,
        name: String,
    },
    /// Internal result for a failed phone-history request.
    OlderFailed {
        chat: ChatId,
        error: String,
    },
    /// Internal group-metadata failure.
    GroupInfoFailed {
        chat: ChatId,
        /// Whether the server refusal is permanent.
        permanent: bool,
    },
    EditText {
        chat: ChatId,
        id: String,
        text: String,
        mentions: Vec<String>,
    },
    Revoke {
        chat: ChatId,
        id: String,
    },
    DeleteLocal {
        chat: ChatId,
        id: String,
    },
    /// Selects and sends files with the desktop picker.
    PickFiles(ChatId),
    /// Sends files with the caption on the first.
    SendFiles {
        chat: ChatId,
        paths: Vec<PathBuf>,
        caption: Option<String>,
        mentions: Vec<String>,
        /// The message the first file replies to.
        quoting: Option<String>,
    },
    /// Sends a clipboard image as straight-alpha RGBA.
    SendImage {
        chat: ChatId,
        width: u32,
        height: u32,
        rgba: Vec<u8>,
        caption: Option<String>,
        mentions: Vec<String>,
        quoting: Option<String>,
    },
    /// Syncs chat mute state. `Some(0)` is indefinite and `None` unmutes.
    SetMuted(ChatId, Option<i64>),
    /// Locks or unlocks a chat (the locked folder).
    SetLocked(ChatId, bool),
    /// Creates a label; the worker owns the clock for its id.
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
    /// Replaces the labels of one chat.
    SetChatLabels {
        chat: ChatId,
        labels: Vec<String>,
    },
    /// Normalizes, encodes, and sends mono 48 kHz push-to-talk audio.
    SendVoice {
        chat: ChatId,
        samples: Vec<f32>,
        quoting: Option<String>,
    },
    /// Sends a played receipt for a voice message.
    MarkPlayed {
        chat: ChatId,
        message: String,
        sender: String,
        receipts: bool,
    },
    /// Sends a WebP sticker.
    SendSticker {
        chat: ChatId,
        path: PathBuf,
        quoting: Option<String>,
    },
    /// Saves a sticker file.
    SaveSticker {
        path: PathBuf,
    },
    /// Internal: the phone received one favorite change, or refused it.
    FavoritePushed {
        hash: String,
        updated_at: i64,
        result: Result<Vec<u8>, String>,
    },
    /// Internal: every queued favorite change was sent.
    FavoritesPushed,
    /// Internal: the one-time replay of the phone's favorites finished.
    FavoritesRecovered {
        complete: bool,
    },
    /// Internal: a favorite from the phone finished downloading.
    FavoriteFetched {
        hash: String,
        result: Result<PathBuf, String>,
    },
    /// Takes a sticker out of Recent here and on the phone.
    RemoveRecentSticker {
        path: PathBuf,
    },
    /// Removes a saved sticker.
    ForgetSticker {
        path: PathBuf,
    },
    /// Imports a pack from a signal.art link.
    ImportStickerUrl {
        url: String,
    },
    /// Selects and imports a .wastickers or zip archive.
    PickStickerArchive,
    /// Asks for an audio file to use as a notification sound.
    PickNotificationSound {
        mention: bool,
    },
    /// Stores a chat's own notification sound.
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
    /// Internal: a picked picture, cropped and encoded as JPEG.
    SetProfilePicture(Vec<u8>),
    /// Internal: the server accepted a profile change.
    ProfileSaved {
        name: Option<String>,
        about: Option<String>,
        picture: bool,
    },
    /// Where new downloads go; `None` is the cache.
    SetDownloadFolder(Option<std::path::PathBuf>),
    /// Asks where to save a copy of an attachment, then copies it there.
    SaveAttachmentAs {
        source: std::path::PathBuf,
        name: String,
    },
    /// Deletes an imported pack directory.
    DeleteStickerPack {
        dir: PathBuf,
    },
    /// Internal pack-import result. An empty error means the picker was canceled.
    StickerPackImported {
        result: Result<String, String>,
    },
    /// Downloads a sticker pack shared in a chat so it can be viewed.
    ViewStickerPack {
        chat: ChatId,
        message: String,
    },
    /// Internal: a shared sticker pack finished downloading.
    StickerPackViewed {
        result: Result<(StickerPack, String), String>,
    },
    /// Copies a viewed pack into the packs here.
    AddStickerPack {
        dir: PathBuf,
        name: String,
    },
    /// Sends a pack as a WhatsApp sticker pack message.
    SendStickerPack {
        chat: ChatId,
        dir: PathBuf,
    },
    /// Chooses a picture to make a sticker from.
    PickStickerPicture,
    /// Internal: the chosen picture, its size, and whether it has see-through
    /// pixels; or an empty error when the choice was cancelled.
    StickerPicturePicked {
        result: Result<(PathBuf, u32, u32, bool), String>,
    },
    /// Makes a sticker from a picture, then adds it to favorites or, with a
    /// chat, sends it there.
    MakeSticker {
        source: PathBuf,
        crop: crate::model::StickerCrop,
        transparent: bool,
        emojis: Vec<String>,
        chat: Option<ChatId>,
    },
    /// Internal: a made sticker, and where it goes.
    StickerMade {
        result: Result<PathBuf, String>,
        chat: Option<ChatId>,
    },
    /// Creates an empty local sticker pack under the given name.
    CreateStickerPack {
        name: String,
    },
    /// Files a sticker into a local pack by its content, or takes it out.
    /// The sticker's own file stays where it is.
    SetStickerPack {
        pack: PathBuf,
        sticker: PathBuf,
        member: bool,
    },
    /// Saves a name through contact sync. `first_name` is the short display
    /// name; `to_phone` also adds it to the phone's address book.
    SaveContact {
        id: String,
        full_name: String,
        first_name: Option<String>,
        to_phone: bool,
    },
    /// Internal contact-save result.
    ContactSaved {
        id: String,
        name: String,
        error: Option<String>,
    },
    /// Checks a number, optionally saves it, and opens its chat.
    NewContact {
        phone: String,
        full_name: Option<String>,
        first_name: Option<String>,
        to_phone: bool,
    },
    /// Internal number-lookup result.
    ContactChecked {
        phone: String,
        full_name: Option<String>,
        first_name: Option<String>,
        to_phone: bool,
        registered: bool,
    },
    /// Downloads and sends a GIF as a short looping video.
    SendGif {
        chat: ChatId,
        gif: Gif,
        quoting: Option<String>,
    },
    /// Searches GIPHY or lists trending results for an empty query.
    SearchGifs {
        query: String,
        key: String,
    },
    /// Loads recent and saved stickers for the picker.
    RecentStickers,
    React {
        chat: ChatId,
        message: String,
        emoji: String,
    },
    SetArchived(ChatId, bool),
    /// Leaves a group or channel. `archive` also hides the chat in Archived.
    LeaveGroup {
        chat: ChatId,
        archive: bool,
    },
    /// Deletes a chat on the phone, then here once the phone agreed.
    DeleteChat(ChatId),
    /// Whether the phone deleted a chat requested through `DeleteChat`.
    ChatDeleted {
        chat: ChatId,
        deleted: bool,
        through: i64,
    },
    SetPinned(ChatId, bool),
    /// Marks a chat as a favorite, or removes the mark, here and on the phone.
    SetFavorite(ChatId, bool),
    /// The phone answered a favorites list sent at `at` holding the queued
    /// changes up to `through`.
    FavoritesSent {
        through: i64,
        at: i64,
        success: bool,
    },
    PairWithPhone(String),
    /// Unlinks the device remotely and locally.
    Unlink,
    Reconnect,
    /// Use this proxy setting and reconnect. Empty follows the environment.
    SetProxy(String),
    /// Sets aside an unreadable archive and the linked session, then starts
    /// over with a new archive and a new link.
    StartOverArchive,
    /// Whether the person is looking at ZapFast. While they are not, the
    /// linked phone keeps receiving push notifications.
    SetOnline(bool),
    Shutdown,
    /// Internal send result.
    Sent {
        chat: ChatId,
        id: String,
        error: Option<String>,
    },
    /// Internal attachment-download result.
    Downloaded {
        card: Option<usize>,
        chat: ChatId,
        id: String,
        result: Result<PathBuf, String>,
    },
    /// Internal recent-sticker download result.
    StickerFetched {
        hash: String,
        result: Result<PathBuf, String>,
    },
    /// Internal profile-picture result.
    AvatarFetched {
        id: String,
        full: bool,
        path: Option<PathBuf>,
    },
    /// Internal retryable profile-picture failure.
    AvatarFailed {
        id: String,
        full: bool,
    },
    /// Internal account about-text result.
    MeInfo {
        about: Option<String>,
    },
    /// Internal GIPHY result.
    GifResults {
        query: String,
        results: Result<Vec<Gif>, GifError>,
    },
    /// Internal file-picker result.
    Picked {
        chat: ChatId,
        paths: Vec<PathBuf>,
    },
    /// Internal uploaded attachment ready for archiving and sending.
    Outbound {
        chat: ChatId,
        row: Box<Message>,
        raw: Vec<u8>,
    },
    /// Internal send audience. The sender waits for it to be archived.
    GroupRecipients {
        chat: ChatId,
        id: String,
        recipients: Vec<String>,
        lids: Vec<(String, String)>,
        stored: tokio::sync::mpsc::UnboundedSender<bool>,
    },
    /// Internal group metadata result.
    GroupInfo {
        chat: ChatId,
        name: Option<String>,
        participants: Vec<String>,
        read_only: bool,
        ephemeral_expiration: Option<u32>,
        ephemeral_setting_timestamp: Option<i64>,
        /// The chat's leave generation when this metadata was asked for. A
        /// snapshot older than a confirmed leave cannot undo it.
        leave_generation: u64,
    },
    /// Internal pairing-code result.
    PairCode {
        result: Result<String, String>,
    },
    /// Internal account read-receipt setting.
    ReceiptsPrivacy {
        disabled: bool,
    },
    /// Full account privacy snapshot, or a failed fetch.
    AccountPrivacy {
        values: Vec<(crate::privacy::PrivacyKind, crate::privacy::PrivacyChoice)>,
        failed: bool,
    },
    /// Asks the phone for the account privacy snapshot again.
    FetchAccountPrivacy,
    /// Writes one account privacy category on the phone.
    SetAccountPrivacy {
        kind: crate::privacy::PrivacyKind,
        choice: crate::privacy::PrivacyChoice,
    },
    /// A confirmed SET for one category.
    AccountPrivacySaved {
        kind: crate::privacy::PrivacyKind,
    },
    /// A failed SET; the interface restores the last snapshot.
    AccountPrivacyFailed {
        kind: crate::privacy::PrivacyKind,
    },
    /// Internal: followed channels and whether each is muted on the server.
    ChannelMutes(Vec<(String, bool)>),
    /// Looks up the group behind an invite code without joining.
    PreviewInvite(String),
    /// Joins the group behind an invite code.
    JoinInvite(String),
    /// Internal result of joining through an invite.
    InviteJoined {
        code: String,
        result: Result<(ChatId, bool), String>,
    },
    /// Ask GitHub whether a newer release exists.
    CheckForUpdates,
    InspectUpdate,
    DownloadUpdate {
        release: crate::updates::Release,
        source: crate::updates::Source,
    },
    InstallUpdate {
        prepared: Box<crate::updates::install::Prepared>,
        arguments: Vec<String>,
    },
}

#[derive(Debug)]
pub enum Event {
    InteractiveReplyState {
        chat: ChatId,
        message: String,
        pending: bool,
    },
    PollCreated {
        chat: ChatId,
        error: Option<String>,
    },
    PollVoted {
        chat: ChatId,
        message: String,
        error: Option<String>,
    },
    Link(LinkStatus),
    /// Linked account identity.
    Me {
        id: String,
        /// Our privacy id (`@lid`), when known.
        lid: Option<String>,
        name: Option<String>,
        about: Option<String>,
    },
    /// Full chat list, newest first.
    Chats(Vec<Chat>),
    /// Every label in creation order. Chats carry the labels they wear.
    Labels(Vec<crate::model::Label>),
    /// Unsent text stored for each chat, sent once at startup.
    Drafts(Vec<(ChatId, String)>),
    /// Messages in one chat matching a search, newest first, echoing the
    /// query and range asked for so a stale answer can be told apart.
    ChatHits {
        chat: ChatId,
        query: String,
        from: Option<i64>,
        until: Option<i64>,
        messages: Vec<Message>,
        /// Whether the archive held more matches than `messages` carries, so
        /// the pane can say so instead of dropping them silently.
        truncated: bool,
    },
    ChatUpdated(Box<Chat>),
    /// Chat messages in ascending order. `older` prepends them; `complete`
    /// means the archive has no earlier rows.
    Messages {
        chat: ChatId,
        messages: Vec<Message>,
        older: bool,
        complete: bool,
    },
    MessageUpdated(Box<Message>),
    /// Files selected for the composer.
    Picked {
        chat: ChatId,
        paths: Vec<PathBuf>,
    },
    /// Live incoming message for desktop notification.
    Incoming {
        chat: ChatId,
        message: Box<Message>,
    },
    Contacts(Vec<Contact>),
    /// Message search results with their query, newest first.
    SearchHits {
        query: String,
        messages: Vec<Message>,
    },
    Typing {
        chat: ChatId,
        sender: String,
        composing: bool,
    },
    Presence {
        id: String,
        online: bool,
        last_seen: Option<i64>,
    },
    Avatar {
        id: String,
        full: bool,
        path: Option<PathBuf>,
    },
    MessageDeleted {
        chat: ChatId,
        id: String,
    },
    /// A chat was deleted here or on a linked device.
    ChatRemoved {
        chat: ChatId,
    },
    /// A chat's messages were cleared while the chat itself stays.
    ChatCleared {
        chat: ChatId,
        through: i64,
    },
    /// GIF search results or failure.
    Gifs {
        query: String,
        results: Result<Vec<Gif>, GifError>,
    },
    /// A picture chosen for the sticker maker: its file, size, and whether it
    /// has see-through pixels.
    StickerPicture {
        path: PathBuf,
        width: u32,
        height: u32,
        transparent: bool,
    },
    /// A shared sticker pack, ready to view, with its publisher; or why it
    /// could not be opened.
    StickerPackPreview(Result<(StickerPack, String), String>),
    /// Favorite stickers, packs, and recent stickers for the picker, with
    /// the emojis each sticker is tagged with.
    Stickers {
        favorites: Vec<PathBuf>,
        packs: Vec<StickerPack>,
        recent: Vec<PathBuf>,
        emojis: std::collections::HashMap<PathBuf, Vec<String>>,
    },
    Media {
        card: Option<usize>,
        chat: ChatId,
        message: String,
        result: Result<PathBuf, String>,
    },
    /// Link-time history sync state.
    Syncing(bool),
    /// Reported history-sync percentage.
    SyncProgress(u32),
    /// Phone-history result. `more` indicates whether another request may help.
    OlderFetched {
        chat: ChatId,
        more: bool,
    },
    /// Whether account privacy disables direct-chat read receipts.
    ReceiptsPrivacy {
        disabled: bool,
    },
    /// Account privacy snapshot from the phone, or a failed fetch.
    AccountPrivacy {
        values: Vec<(crate::privacy::PrivacyKind, crate::privacy::PrivacyChoice)>,
        failed: bool,
    },
    /// A confirmed SET for one category.
    AccountPrivacySaved {
        kind: crate::privacy::PrivacyKind,
    },
    /// A failed SET.
    AccountPrivacyFailed {
        kind: crate::privacy::PrivacyKind,
    },
    /// How many chats this account may pin: more with WhatsApp Plus.
    PinLimit(usize),
    /// The followed message's receipts, sent when following starts and
    /// whenever one arrives.
    Receipts(crate::model::MessageReceipts),
    /// An audio file chosen for one chat's notifications.
    ChatSoundPicked {
        chat: ChatId,
        path: std::path::PathBuf,
    },
    /// A folder chosen for new downloads.
    DownloadFolderPicked(std::path::PathBuf),
    /// An audio file chosen as a notification sound.
    NotificationSoundPicked {
        mention: bool,
        path: std::path::PathBuf,
    },
    /// The group behind an invite link.
    InvitePreview {
        code: String,
        result: Result<crate::model::InviteInfo, String>,
    },
    /// Joining through an invite finished; `pending` means admins must
    /// approve first.
    InviteJoined {
        code: String,
        result: Result<(ChatId, bool), String>,
    },
    /// Number lookup succeeded and its chat can open.
    ContactReady {
        id: String,
        name: Option<String>,
    },
    /// Informational toast message.
    Info(String),
    /// A newer release than this build exists.
    UpdateAvailable {
        version: String,
        url: String,
    },
    UpdateSupport(Result<crate::updates::install::Installation, String>),
    UpdateProgress {
        received: u64,
        total: u64,
    },
    UpdateDownloaded(Result<Box<crate::updates::install::Prepared>, String>),
    UpdateInstalling(Result<(), String>),
    /// A send was refused before anything left this computer. It returns
    /// what was being sent so the user loses neither text nor a recording.
    SendRefused {
        chat: ChatId,
        quoting: Option<String>,
        unsent: Unsent,
        reason: Refusal,
    },
    Error(String),
}

/// Why the worker refused a send.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// There is no WhatsApp connection.
    Offline,
    /// The message being replied to cannot be quoted, because its row or its
    /// original protobuf is missing, unreadable, or deleted. Sending anyway
    /// would deliver the reply without its quote.
    QuoteUnavailable,
}

/// The content of a refused send.
#[derive(Clone, Debug, PartialEq)]
pub enum Unsent {
    /// Composer text in wire form, with `@user` mention tokens.
    Text(String),
    Voice(Vec<f32>),
    Files {
        paths: Vec<PathBuf>,
        caption: Option<String>,
    },
    Image {
        width: u32,
        height: u32,
        rgba: Vec<u8>,
        caption: Option<String>,
    },
    Sticker,
    Gif,
}

/// Cross-thread window wake handle.
#[derive(Clone, Default)]
pub struct Waker(Arc<std::sync::Mutex<Option<egui::Context>>>);

impl Waker {
    pub fn attach(&self, ctx: &egui::Context) {
        *self.0.lock().unwrap_or_else(|p| p.into_inner()) = Some(ctx.clone());
    }

    pub fn detach(&self) {
        *self.0.lock().unwrap_or_else(|p| p.into_inner()) = None;
    }

    pub fn wake(&self) {
        if let Some(ctx) = self.0.lock().unwrap_or_else(|p| p.into_inner()).as_ref() {
            ctx.request_repaint();
        }
    }

    /// Schedules a delayed repaint.
    pub fn wake_after(&self, delay: std::time::Duration) {
        if let Some(ctx) = self.0.lock().unwrap_or_else(|p| p.into_inner()).as_ref() {
            ctx.request_repaint_after(delay);
        }
    }
}

/// UI handle to the backend runtime.
pub struct Backend {
    startup: Option<tokio::sync::oneshot::Sender<()>>,
    commands: mpsc::UnboundedSender<Command>,
    events: std::sync::mpsc::Receiver<Event>,
    thread: Option<std::thread::JoinHandle<()>>,
    offline: bool,
    #[cfg(any(test, feature = "demo"))]
    demo_commands: Option<std::sync::Mutex<Vec<Command>>>,
}

impl Backend {
    pub fn spawn(dirs: AppDirs, waker: Waker) -> Self {
        let (command_tx, command_rx) = mpsc::unbounded_channel();
        let (event_tx, event_rx) = std::sync::mpsc::channel();
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("zapfast-runtime")
            .enable_all()
            .build()
            .expect("unable to start the async runtime");
        let worker_commands = command_tx.clone();
        let (startup, started) = tokio::sync::oneshot::channel();
        let thread = std::thread::Builder::new()
            .name("zapfast-backend".to_string())
            .spawn(move || {
                runtime.block_on(async move {
                    if started.await.is_ok() {
                        worker::run(dirs, event_tx, worker_commands, command_rx, waker).await;
                    }
                });
                runtime.shutdown_timeout(Duration::from_secs(3));
            })
            .expect("unable to start the backend thread");

        Self {
            startup: Some(startup),
            commands: command_tx,
            events: event_rx,
            thread: Some(thread),
            offline: false,
            #[cfg(any(test, feature = "demo"))]
            demo_commands: None,
        }
    }

    /// Creates a disconnected backend and event sender for demos and tests.
    pub fn detached() -> (Self, std::sync::mpsc::Sender<Event>) {
        let (command_tx, _command_rx) = mpsc::unbounded_channel();
        let (event_tx, event_rx) = std::sync::mpsc::channel();
        (
            Self {
                startup: None,
                commands: command_tx,
                events: event_rx,
                thread: None,
                offline: true,
                #[cfg(any(test, feature = "demo"))]
                demo_commands: None,
            },
            event_tx,
        )
    }

    /// Records commands without a runtime or network connection.
    #[cfg(test)]
    pub(crate) fn recording() -> (Self, mpsc::UnboundedReceiver<Command>) {
        let (backend, inbox, _) = Self::recording_with_events();
        (backend, inbox)
    }

    /// Records commands and lets a test deliver events.
    #[cfg(test)]
    pub(crate) fn recording_with_events() -> (
        Self,
        mpsc::UnboundedReceiver<Command>,
        std::sync::mpsc::Sender<Event>,
    ) {
        let (mut backend, events) = Self::detached();
        let (commands, inbox) = mpsc::unbounded_channel();
        backend.commands = commands;
        backend.offline = false;
        (backend, inbox, events)
    }

    /// Disables commands except shutdown.
    pub fn set_offline(&mut self, offline: bool) {
        self.offline = offline;
    }

    pub fn is_offline(&self) -> bool {
        self.offline
    }

    pub fn send(&self, command: Command) {
        if self.offline && !matches!(command, Command::Shutdown) {
            #[cfg(any(test, feature = "demo"))]
            if let Some(commands) = &self.demo_commands {
                commands
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .push(command);
            }
            return;
        }
        let _ = self.commands.send(command);
    }

    /// Captures real UI commands for an offline demo's local responder.
    #[cfg(any(test, feature = "demo"))]
    pub(crate) fn record_demo_commands(&mut self) {
        assert!(self.offline && self.thread.is_none());
        self.demo_commands = Some(Default::default());
    }

    #[cfg(any(test, feature = "demo"))]
    pub(crate) fn take_demo_commands(&self) -> Vec<Command> {
        self.demo_commands
            .as_ref()
            .map_or_else(Vec::new, |commands| {
                std::mem::take(&mut *commands.lock().unwrap_or_else(|p| p.into_inner()))
            })
    }

    pub fn poll(&self) -> Vec<Event> {
        self.events.try_iter().collect()
    }

    /// Start database migrations only after the first window frame has been
    /// acknowledged by the update helper. Dropping this permit cancels startup.
    pub fn take_startup(&mut self) -> Option<tokio::sync::oneshot::Sender<()>> {
        self.startup.take()
    }

    pub fn shutdown(&mut self) {
        self.startup.take();
        self.send(Command::Shutdown);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
