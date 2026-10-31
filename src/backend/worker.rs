//! Tokio worker for WhatsApp, the archive, attachments, and profile pictures.
//!
//! Messages are archived before reaching the UI. Privacy ids (`@lid`) are
//! canonicalized to phone-number ids as soon as their mapping is known.

use std::collections::{HashMap, HashSet, VecDeque};
use std::io::{self, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::mpsc;
use whatsapp_rust::download::MediaType;
use whatsapp_rust::features::PresencePolicy;
use whatsapp_rust::features::message_edit::{
    SecretEncKind, decrypt_secret_encrypted_with_fallback, extract_envelope,
    extract_secret_encrypted,
};
use whatsapp_rust::media::{
    AudioOptions, DocumentOptions, ImageOptions, VideoOptions, audio_message, document_message,
    image_message, video_message,
};
use whatsapp_rust::pair::NATIVE_CAMERA_DEEP_LINK_PREFIX;
use whatsapp_rust::pair_code::PairCodeOptions;
use whatsapp_rust::prelude::{
    Bot, BotHandle, Client, Jid, MessageBuilderExt, MessageExt, MessageField, SendOptions, wa,
};
use whatsapp_rust::send::{PinDuration, RevokeType};
use whatsapp_rust::types::events as wa_events;
use whatsapp_rust::types::message::{EncMediaType, MessageInfo, MessageSource};
use whatsapp_rust::types::presence::{ChatActivity, PresenceStatus, ReceiptType};
use whatsapp_rust::upload::UploadOptions;
use whatsapp_rust::wacore::download::{DownloadWriter, Downloadable};
use whatsapp_rust::wacore::history_sync::{HistorySyncStream, MAX_DECOMPRESSED};
use whatsapp_rust::wacore::iq::abprops;
use whatsapp_rust::wacore::store::DevicePropsOverride;
use whatsapp_rust::wacore_binary::jid::JidExt;
use whatsapp_rust::waproto::buffa::Message as _;
use whatsapp_rust::{MediaRetryResult, MediaReuploadRequest};

mod bot_replies;
mod calls;
mod channel_pictures;
mod contact_names;
mod device_store;
mod download_folders;
mod early_events;
use early_events::WaitingReaction;
mod favorite_chats;
mod group_admin;
mod interactive;
mod link_watch;
mod new_group;
mod poll_history;
mod polls;
mod prefetch;
mod star_recovery;
mod sticker_pace;
mod stickers;

use super::{
    Command, Event, GroupEdit, LinkStatus, MessageRemovalOutcome, Refusal, Unsent, Waker,
    read_sync::ReadSync,
};
use crate::app::PAGE;
use crate::archive::Archive;
use crate::model::{
    ATTACHMENT_DOWNLOAD_LIMIT, Chat, ChatId, ChatKind, Contact, Content, Delivery, Gif, GifError,
    LIVE_LOCATION_LIMIT, LinkPreview, Media, MentionRef, Message, Quoted, Reaction,
};
use crate::paths::AccountDirs;
use crate::privacy::{self, PrivacyChoice, PrivacyKind};

/// Delay after the last history chunk before sync is complete.
const SYNC_QUIET: Duration = Duration::from_secs(20);
/// Profile-picture cache lifetime.
const AVATAR_FRESH: Duration = Duration::from_secs(24 * 60 * 60);
/// Phone history-request timeout.
const PHONE_PATIENCE: Duration = Duration::from_secs(30);
/// Phone history-request batch size.
const PHONE_BATCH: i32 = 50;
/// `HistorySync.sync_type` for on-demand history responses.
const ON_DEMAND: i32 = 6;
/// Maximum attachment-preview dimension.
const THUMBNAIL_SIDE: u32 = 96;
/// WhatsApp's profile pictures are 640 pixels square.
const PROFILE_PICTURE_SIDE: u32 = 640;
/// Sticker download batch size for the picker.
const STICKER_FETCH_LIMIT: usize = 40;
/// Picker stickers that arrive between two refreshes of its shelves while
/// more are still on their way. The backlog is paced, two at a time and
/// slower after a rate limit, so waiting for all of it left the shelves
/// stale for minutes; refreshing on every arrival reshuffled Recent under
/// the reader (#165).
const SHELF_REFRESH_EVERY: usize = 4;
/// How many starred messages the list shows.
const STARRED_LIMIT: usize = 200;
const ATTACHMENT_LIMIT_ERROR: &str =
    "This attachment is larger than the 64 MiB automatic download limit";

/// The deadline of a download of unknown or small size.
const ATTACHMENT_TIMEOUT: Duration = Duration::from_secs(120);
/// The slowest transfer the deadline allows for beyond that: 128 KiB/s, so a
/// 2 GiB video still has over four hours.
const ATTACHMENT_SLOWEST_RATE: u64 = 128 * 1024;

/// Publishing bytes is not success until their archive reference has been accepted.
fn confirmed_download(
    path: PathBuf,
    saved: crate::archive::Result<bool>,
) -> Result<PathBuf, String> {
    match saved {
        Ok(true) => Ok(path),
        Ok(false) => Err("This attachment is no longer in the archive".to_owned()),
        Err(_) => {
            log::warn!("could not store a downloaded attachment's path");
            Err(
                "Could not record the downloaded file in the archive. The file has been kept"
                    .to_owned(),
            )
        }
    }
}

/// How long a download of `size` bytes may take, re-upload request included.
fn attachment_deadline(size: Option<u64>) -> Duration {
    ATTACHMENT_TIMEOUT + Duration::from_secs(size.unwrap_or(0) / ATTACHMENT_SLOWEST_RATE)
}

async fn with_attachment_deadline<T>(
    duration: Duration,
    operation: impl std::future::Future<Output = Result<T, String>>,
) -> Result<T, String> {
    tokio::time::timeout(duration, operation)
        .await
        .unwrap_or_else(|_| Err("Download timed out".to_owned()))
}

/// A streaming download sink that refuses to grow beyond the attachment limit.
///
/// WhatsApp's declared file length is useful to reject an oversized attachment
/// before connecting, but it is not trusted as the enforcement point.
struct LimitedWriter<W> {
    inner: W,
    limit: u64,
}

impl<W> LimitedWriter<W> {
    fn new(inner: W, limit: u64) -> Self {
        Self { inner, limit }
    }
}

impl<W: Write + Seek> Write for LimitedWriter<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let position = self.inner.stream_position()?;
        let remaining = self.limit.saturating_sub(position);
        if bytes.len() as u64 > remaining {
            return Err(io::Error::other(ATTACHMENT_LIMIT_ERROR));
        }
        self.inner.write(bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

impl<W: Seek> Seek for LimitedWriter<W> {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.inner.seek(position)
    }
}

impl<W: DownloadWriter> DownloadWriter for LimitedWriter<W> {
    fn truncate(&mut self, len: u64) -> io::Result<()> {
        if len > self.limit {
            return Err(io::Error::other(ATTACHMENT_LIMIT_ERROR));
        }
        self.inner.truncate(len)
    }
}

fn attachment_is_too_large(size: Option<u64>) -> bool {
    size.is_some_and(|size| size > ATTACHMENT_DOWNLOAD_LIMIT)
}

/// Streams a verified attachment to disk without accepting more than `limit`
/// bytes: 64 MiB for automatic downloads, unlimited for one the reader asked
/// for.
async fn download_attachment(
    client: &Client,
    downloadable: &dyn Downloadable,
    dir: &Path,
    path: &Path,
    limit: u64,
) -> Result<PathBuf, String> {
    fetch_attachment(client, downloadable, dir, path, limit, false, None).await
}

/// [`download_attachment`], and with `keep_others` a file already at `path`
/// stays: the download takes the first free `name (n).ext` instead, as in
/// the reader's own download folder. Returns where the file landed.
async fn fetch_attachment(
    client: &Client,
    downloadable: &dyn Downloadable,
    dir: &Path,
    path: &Path,
    limit: u64,
    keep_others: bool,
    cache: Option<&Path>,
) -> Result<PathBuf, String> {
    if downloadable.file_length().is_some_and(|size| size > limit) {
        return Err(ATTACHMENT_LIMIT_ERROR.to_owned());
    }
    debug_assert_eq!(path.parent(), Some(dir));
    let (target, temporary, file) = stage_attachment(path, cache).await?;
    let _staged = Staged(temporary.clone());
    let result = client
        .download_to_writer(
            downloadable,
            LimitedWriter::new(file, limit),
            Default::default(),
        )
        .await;
    match result {
        Ok(writer) => {
            // Close the verified file before publishing it, including on Windows.
            drop(writer);
            // The staging file is removed on the way out if it was not
            // published.
            publish_download(&temporary, &target, keep_others && target == path, cache).await
        }
        Err(error) => {
            let error = error.to_string();
            if error.contains(ATTACHMENT_LIMIT_ERROR) {
                Err(ATTACHMENT_LIMIT_ERROR.to_owned())
            } else {
                Err(error)
            }
        }
    }
}

/// Opens the actual staging file before downloading, trying the cache on local failure.
async fn stage_attachment(
    path: &Path,
    cache: Option<&Path>,
) -> Result<(PathBuf, PathBuf, std::fs::File), String> {
    let mut failure = String::new();
    for target in std::iter::once(path).chain(cache.filter(|cache| *cache != path)) {
        let staged = async {
            let dir = target.parent().ok_or("attachment has no parent folder")?;
            tokio::fs::create_dir_all(dir)
                .await
                .map_err(|error| error.to_string())?;
            temporary_attachment_file(target)
        }
        .await;
        match staged {
            Ok((temporary, file)) => return Ok((target.to_owned(), temporary, file)),
            Err(error) => failure = error,
        }
    }
    Err(failure)
}

/// A publication failure saves the verified bytes in the cache without another transfer.
async fn publish_download(
    temporary: &Path,
    path: &Path,
    keep_others: bool,
    cache: Option<&Path>,
) -> Result<PathBuf, String> {
    let published = if keep_others {
        publish_beside(temporary, path)
            .await
            .map_err(|error| error.to_string())
    } else {
        publish_attachment(temporary, path)
            .await
            .map(|()| path.to_owned())
    };
    match published {
        Ok(path) => Ok(path),
        Err(error) => {
            let Some(cache) = cache.filter(|cache| *cache != path) else {
                return Err(error);
            };
            let (_, staging, file) = stage_attachment(cache, None).await?;
            let _staged = Staged(staging.clone());
            drop(file);
            tokio::fs::copy(temporary, &staging)
                .await
                .map_err(|error| error.to_string())?;
            publish_attachment(&staging, cache).await?;
            Ok(cache.to_owned())
        }
    }
}

/// Publishes a complete attachment only after its download has been verified.
async fn publish_attachment(temporary: &Path, path: &Path) -> Result<(), String> {
    // Windows does not replace an existing destination during rename. A stale
    // cache file has no archive reference, and active downloads are deduplicated.
    #[cfg(windows)]
    if path.exists() {
        tokio::fs::remove_file(path)
            .await
            .map_err(|error| error.to_string())?;
    }
    tokio::fs::rename(temporary, path)
        .await
        .map_err(|error| error.to_string())
}

/// Publishes a complete attachment at `path`, or at the first free
/// `name (n).ext` beside it, never replacing a file already there: the
/// reader's download folder holds their own files too. A hard link claims
/// the name atomically; on a file system without them (FAT, some network
/// shares) an empty file reserves it first.
async fn publish_beside(temporary: &Path, path: &Path) -> io::Result<PathBuf> {
    let (temporary, path) = (temporary.to_path_buf(), path.to_path_buf());
    tokio::task::spawn_blocking(move || {
        for candidate in (0..1000).map(|n| numbered_path(&path, n)) {
            match std::fs::hard_link(&temporary, &candidate) {
                Ok(()) => {
                    let _ = std::fs::remove_file(&temporary);
                    return Ok(candidate);
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(_) => {}
            }
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&candidate)
            {
                Ok(reserved) => {
                    drop(reserved);
                    // Windows renames only onto a free name; the reserved
                    // file is ours to remove.
                    #[cfg(windows)]
                    std::fs::remove_file(&candidate)?;
                    return match std::fs::rename(&temporary, &candidate) {
                        Ok(()) => Ok(candidate),
                        Err(error) => {
                            let _ = std::fs::remove_file(&candidate);
                            Err(error)
                        }
                    };
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::other("no free file name for the download"))
    })
    .await
    .map_err(io::Error::other)?
}

/// `path` itself for 0, then `name (n).ext`, as file managers number copies.
fn numbered_path(path: &Path, n: u32) -> PathBuf {
    if n == 0 {
        return path.to_path_buf();
    }
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("download");
    let numbered = match name.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() => format!("{stem} ({n}).{extension}"),
        _ => format!("{name} ({n})"),
    };
    path.with_file_name(numbered)
}

/// A hidden, per-attempt path in the destination directory, so a verified
/// download can replace the cache file atomically.
fn temporary_attachment_path(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("media");
    path.with_file_name(format!(".{name}.{:016x}.part", rand::random::<u64>()))
}

/// Creates an exclusive temporary file, retrying a vanishingly unlikely name
/// collision without ever opening another download's staging file.
fn temporary_attachment_file(path: &Path) -> Result<(PathBuf, std::fs::File), String> {
    for _ in 0..8 {
        let temporary = temporary_attachment_path(path);
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
        {
            Ok(file) => return Ok((temporary, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.to_string()),
        }
    }
    Err("Could not create a unique attachment staging file".to_owned())
}

/// Removes a download's staging file when it ends, whichever way that is:
/// published away (the removal then finds nothing), failed, or cancelled by
/// its deadline, which drops the download without running its error path.
/// The download's writer is dropped first, closing the file, so the removal
/// also works on Windows.
struct Staged(PathBuf);

impl Drop for Staged {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Adds up the files under `dir`, the account's media cache. Downloads still
/// being written (hidden `.part` files) are left out.
pub(crate) fn measure_media(dir: &Path) -> io::Result<crate::model::StorageStats> {
    let mut stats = crate::model::StorageStats::default();
    walk_media(dir, &mut |path, metadata| {
        stats.add(path, metadata.len());
        Ok(())
    })?;
    Ok(stats)
}

/// Deletes the files under `dir`, the account's media cache, leaving the
/// folders and any download still being written.
pub(crate) fn clear_media(dir: &Path) -> io::Result<()> {
    let mut failed = None;
    walk_media(dir, &mut |path, _| {
        if let Err(error) = std::fs::remove_file(path) {
            failed = Some(error);
        }
        Ok(())
    })?;
    failed.map_or(Ok(()), Err)
}

/// Visits every finished file under `dir`, an absent folder being empty.
fn walk_media(
    dir: &Path,
    visit: &mut dyn FnMut(&Path, &std::fs::Metadata) -> io::Result<()>,
) -> io::Result<()> {
    let mut folders = vec![dir.to_path_buf()];
    while let Some(folder) = folders.pop() {
        let entries = match std::fs::read_dir(&folder) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        for entry in entries {
            let entry = entry?;
            let metadata = entry.metadata()?;
            let path = entry.path();
            if metadata.is_dir() {
                folders.push(path);
            } else if metadata.is_file() && !is_staging(&path) {
                visit(&path, &metadata)?;
            }
        }
    }
    Ok(())
}

/// Whether `path` is a download still being written.
fn is_staging(path: &Path) -> bool {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy())
        .unwrap_or_default();
    name.starts_with('.') && name.ends_with(".part")
}

/// Removes incomplete, unreferenced downloads left by an interrupted process.
fn discard_attachment_staging(dir: &Path) {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return,
        Err(_) => {
            log::warn!("could not list attachment staging files");
            return;
        }
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.')
            && name.ends_with(".part")
            && entry.path().is_file()
            && let Err(_) = std::fs::remove_file(entry.path())
        {
            log::warn!("could not remove incomplete attachment");
        }
    }
}

/// WhatsApp keeps at most three pinned chats without WhatsApp Plus, and
/// replaces an existing pin on the phone when a linked device adds a fourth.
pub const PINNED_CHATS: usize = 3;
/// WhatsApp Plus raises the limit to twenty.
pub const PLUS_PINNED_CHATS: usize = 20;

/// Seven days: the pin length WhatsApp offers for a message pinned in a chat.
const PIN_SECS: i64 = 7 * 24 * 60 * 60;
/// Says in the log whether WhatsApp lets this device start calls, and group
/// calls in particular. WhatsApp Web shows group calling only while the server
/// turns on `enable_web_group_calling` for the account, and a group offer from
/// a device it is off for is refused with 427 ("group call not enabled"). Two
/// flags, no personal data; asked like the pin limit, for want of an event.
fn spawn_calling_gate_log(client: Arc<Client>) {
    tokio::spawn(async move {
        for _ in 0..30 {
            let calling = client
                .ab_prop_enabled(abprops::web::ENABLE_WEB_CALLING)
                .await;
            let group = client
                .ab_prop_enabled(abprops::web::ENABLE_WEB_GROUP_CALLING)
                .await;
            if let (Some(calling), Some(group)) = (calling, group) {
                log::info!("[CALL] web calling enabled={calling} group calling enabled={group}");
                return;
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
        log::info!("[CALL] web calling flags never arrived");
    });
}

/// Reports how many chats this account may pin. The AB props arrive shortly
/// after connecting and there is no event for them, so this polls briefly.
fn spawn_pin_limit_check(
    client: Arc<Client>,
    events: std::sync::mpsc::Sender<Event>,
    waker: Waker,
) {
    tokio::spawn(async move {
        for _ in 0..30 {
            if let Some(plus) = client
                .ab_prop_enabled(abprops::web::AURA_PINNED_CHATS_BENEFIT_ACTIVE)
                .await
            {
                let limit = if plus {
                    PLUS_PINNED_CHATS
                } else {
                    PINNED_CHATS
                };
                let _ = events.send(Event::PinLimit(limit));
                waker.wake();
                return;
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    });
}

fn account_allows_receipts(
    settings: &whatsapp_rust::wacore::iq::privacy::PrivacySettingsResponse,
) -> bool {
    use whatsapp_rust::wacore::iq::privacy::{PrivacyCategory, PrivacyValue};
    matches!(
        settings.get_value(&PrivacyCategory::ReadReceipts),
        Some(PrivacyValue::All)
    )
}

/// Fetches the account privacy snapshot and hands it to the interface. A
/// failed fetch is reported as such: the rows stay disabled rather than
/// showing an invented value.
async fn publish_account_privacy(client: &Client, commands: &mpsc::UnboundedSender<Command>) {
    match client.fetch_privacy_settings().await {
        Ok(settings) => {
            let disabled = !account_allows_receipts(&settings);
            let values = privacy::values_from_response(&settings);
            let _ = commands.send(Command::AccountPrivacy {
                values,
                failed: false,
            });
            let _ = commands.send(Command::ReceiptsPrivacy { disabled });
        }
        Err(error) => {
            log::debug!("privacy settings not fetched: {error}");
            let _ = commands.send(Command::AccountPrivacy {
                values: Vec::new(),
                failed: true,
            });
        }
    }
}

/// The library's persisted privacy value is refreshed during connection setup,
/// which can finish after messages arrive and does not track later phone edits.
/// Check the account before disclosing a read/play; an unavailable setting is
/// not permission to send a receipt. Chat-state sync does not use this gate.
async fn receipts_allowed(
    client: &Client,
    jid: &Jid,
    commands: &mpsc::UnboundedSender<Command>,
) -> bool {
    if jid.is_group() {
        return true;
    }
    match client.fetch_privacy_settings().await {
        Ok(settings) => {
            let allowed = account_allows_receipts(&settings);
            let _ = commands.send(Command::ReceiptsPrivacy { disabled: !allowed });
            allowed
        }
        Err(error) => {
            log::debug!("receipt withheld: account privacy unavailable: {error}");
            false
        }
    }
}

/// Whether a played receipt for a direct message goes out. With read
/// receipts on it goes to the sender. With them off WhatsApp still tells our
/// own devices, as `played-self`, so the phone stops marking the voice message
/// unheard (#501); whatsapp-rust sends that form when it knows they are off
/// (`library_hides`). While it does not know yet, nothing goes, so the sender
/// never learns of a play the account hides.
fn played_receipt_goes(account_allows: bool, library_hides: bool) -> bool {
    account_allows || library_hides
}

/// Like `receipts_allowed`, for a played receipt: see `played_receipt_goes`.
async fn played_receipt_allowed(
    client: &Client,
    jid: &Jid,
    commands: &mpsc::UnboundedSender<Command>,
) -> bool {
    if jid.is_group() {
        return true;
    }
    match client.fetch_privacy_settings().await {
        Ok(settings) => {
            let allowed = account_allows_receipts(&settings);
            let _ = commands.send(Command::ReceiptsPrivacy { disabled: !allowed });
            let hides = client
                .persistence_manager()
                .get_device_snapshot()
                .read_receipts_disabled;
            played_receipt_goes(allowed, hides)
        }
        Err(error) => {
            log::debug!("played receipt withheld: account privacy unavailable: {error}");
            false
        }
    }
}

/// A sticker file's size and modification time, with the emojis read from it.
type EmojiStamp = ((u64, Option<std::time::SystemTime>), Vec<String>);

/// Downloadable recent sticker from the phone.
struct PhoneSticker(wa::StickerMetadata);

impl Downloadable for PhoneSticker {
    fn direct_path(&self) -> Option<&str> {
        self.0.direct_path.as_deref()
    }

    fn media_key(&self) -> Option<&[u8]> {
        self.0.media_key.as_deref()
    }

    fn file_enc_sha256(&self) -> Option<&[u8]> {
        self.0.file_enc_sha256.as_deref()
    }

    fn file_sha256(&self) -> Option<&[u8]> {
        self.0.file_sha256.as_deref()
    }

    fn file_length(&self) -> Option<u64> {
        self.0.file_length
    }

    fn app_info(&self) -> MediaType {
        MediaType::Sticker
    }
}

/// The device name shown in WhatsApp's Linked Devices list, with the
/// hostname appended so multiple machines can be told apart (#210).
fn device_name() -> String {
    let host = crate::util::hostname();
    if host.is_empty() {
        "ZapFast".to_owned()
    } else {
        format!("ZapFast ({host})")
    }
}

/// App version in WhatsApp device-property format.
fn app_version() -> wa::device_props::AppVersion {
    let mut parts = env!("CARGO_PKG_VERSION")
        .split('.')
        .map(|part| part.parse::<u32>().ok());
    {
        let mut proto = wa::device_props::AppVersion::default();
        proto.primary = parts.next().flatten();
        proto.secondary = parts.next().flatten();
        proto.tertiary = parts.next().flatten();
        proto
    }
}

/// Stable sticker hash across messages and the phone's recent list.
fn sticker_hash(sha256: Option<&[u8]>, enc_sha256: Option<&[u8]>) -> Option<String> {
    let bytes = sha256
        .filter(|bytes| !bytes.is_empty())
        .or(enc_sha256.filter(|bytes| !bytes.is_empty()))?;
    Some(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

/// Runs the protocol worker, receiving commands and waking the interface for emitted events.
pub async fn run(
    dirs: AccountDirs,
    events: std::sync::mpsc::Sender<Event>,
    commands: mpsc::UnboundedSender<Command>,
    mut inbox: mpsc::UnboundedReceiver<Command>,
    waker: Waker,
) {
    let archive = loop {
        let path = dirs.archive_db();
        let opened = tokio::task::spawn_blocking(move || Archive::open(&path)).await;
        match opened {
            Ok(Ok(archive)) => break archive,
            result => {
                let error = match result {
                    Ok(Err(error)) => format!("{error:#}"),
                    Err(_) => "Archive unlock worker failed".to_owned(),
                    Ok(Ok(_)) => unreachable!(),
                };
                log::error!("could not unlock the message archive: {error}");
                let _ = events.send(Event::Link(LinkStatus::Failed(error)));
                waker.wake();
                // Do not connect with a disposable archive: history is replayed
                // only once and would be lost if the keyring were locked.
                loop {
                    match inbox.recv().await {
                        Some(Command::Reconnect) => break,
                        Some(Command::StartOverArchive) => {
                            match set_aside_unreadable_archive(&dirs) {
                                Ok(kept) => log::warn!(
                                    "set aside an unreadable archive as {}",
                                    kept.file_name().unwrap_or_default().to_string_lossy()
                                ),
                                Err(error) => {
                                    let _ = events.send(Event::Link(LinkStatus::Failed(format!(
                                        "Could not set the old archive aside: {error}"
                                    ))));
                                    waker.wake();
                                    continue;
                                }
                            }
                            break;
                        }
                        Some(Command::Shutdown) | None => return,
                        _ => {}
                    }
                }
            }
        }
    };
    let (wa_sender, wa_events) = mpsc::unbounded_channel();
    let privacy_confirmed = archive
        .meta("chat_privacy_ready_v1")
        .ok()
        .flatten()
        .as_deref()
        == Some("complete");
    // Only an archive filled before lock state was mirrored needs its lock
    // state rebuilt from a snapshot; a new link receives it with the first sync.
    let privacy_snapshot =
        !privacy_confirmed && archive.chats().is_ok_and(|chats| !chats.is_empty());
    // A new link receives the phone's favorites and its "Keep chats archived"
    // with its first sync; only an archive filled before ZapFast followed
    // them needs them replayed.
    let replayed = [
        stickers::FAVORITES_RECOVERED,
        stickers::UNARCHIVE_SETTING_RECOVERED,
    ];
    let favorites_recovered = replayed
        .iter()
        .all(|key| archive.meta(key).ok().flatten().as_deref() == Some("complete"))
        || (archive.chats().is_ok_and(|chats| chats.is_empty()) && {
            // Every key is written, so one failure does not leave the other
            // unrecorded.
            let mut recorded = true;
            for key in replayed {
                if let Err(error) = archive.set_meta(key, "complete") {
                    log::warn!("could not record the chat settings replay: {error}");
                    recorded = false;
                }
            }
            recorded
        });
    // The phone's own setting, read before the archive moves into the worker.
    let (phone_unarchive_chats, phone_unarchive_unread) = match archive.unarchive_chats() {
        Ok(setting) => (setting, false),
        Err(error) => {
            log::warn!("could not read the phone's Keep chats archived: {error}");
            (None, true)
        }
    };
    // Likewise only contacts synced before first names were kept lack them.
    let first_names_recovered = archive
        .meta(contact_names::FIRST_NAMES_RECOVERED)
        .ok()
        .flatten()
        .as_deref()
        == Some("complete")
        || (archive.chats().is_ok_and(|chats| chats.is_empty())
            && archive
                .set_meta(contact_names::FIRST_NAMES_RECOVERED, "complete")
                .is_ok());
    // Only stars synced before ZapFast followed them were consumed unseen.
    let stars_recovered = archive
        .meta(star_recovery::STARS_RECOVERED)
        .ok()
        .flatten()
        .as_deref()
        == Some("complete")
        || (archive.chats().is_ok_and(|chats| chats.is_empty())
            && archive
                .set_meta(star_recovery::STARS_RECOVERED, "complete")
                .is_ok());
    let mut worker = Worker {
        privacy_ready: privacy_confirmed,
        privacy_confirmed,
        privacy_snapshot,
        privacy_reveal_at: (!privacy_confirmed).then(|| Instant::now() + PRIVACY_GRACE),
        privacy_attempts: 0,
        privacy_warned: false,
        privacy_recovering: false,
        privacy_generation: 0,
        message_removals_in_flight: HashSet::new(),
        contact_generation: 0,
        privacy_retry: Instant::now(),
        star_generation: HashMap::new(),
        star_session: 0,
        pin_generation: HashMap::new(),
        withheld_pages: Vec::new(),
        dirs,
        events,
        commands,
        waker,
        archive,
        client: None,
        handle: None,
        call: None,
        phone_only_calls: Vec::new(),
        call_devices: crate::calls::DeviceList::default(),
        wa_sender,
        me_pn: None,
        me_lid: None,
        me_name: None,
        me_about: None,
        lid_to_pn: HashMap::new(),
        contacts: HashMap::new(),
        status: LinkStatus::Starting,
        pairing_phone: None,
        pair_code: None,
        qr: None,
        syncing: false,
        sync_deadline: None,
        group_info_requested: HashSet::new(),
        leave_generation: HashMap::new(),
        subject_generation: HashMap::new(),
        settings_generation: HashMap::new(),
        group_pictures_sent: HashMap::new(),
        description_generation: HashMap::new(),
        group_info_generation: HashMap::new(),
        edits: HashMap::new(),
        edit_sequence: 0,
        group_info_queue: std::collections::VecDeque::new(),
        group_info_tries: HashMap::new(),
        group_info_retry: Vec::new(),
        presence_subscribed: HashSet::new(),
        download_folder: None,
        cleaned_download_folders: HashSet::new(),
        keep_chats_archived: true,
        phone_unarchive_chats,
        phone_unarchive_unread,
        online_wanted: false,
        online_changed: Instant::now(),
        online_sent: None,
        presence_in_flight: false,
        presence_refresh_pending: false,
        presence_generation: 0,
        channel_list_generation: 0,
        pending_older: HashMap::new(),
        older_warned: HashSet::new(),
        pending_avatars: HashMap::new(),
        channel_pictures: Default::default(),
        sticker_fetches: HashSet::new(),
        sticker_downloads: HashSet::new(),
        recent_hashes: HashMap::new(),
        emoji_cache: HashMap::new(),
        favorite_fetches: HashSet::new(),
        sticker_pace: Default::default(),
        sticker_failed: HashSet::new(),
        sticker_download_failed: HashSet::new(),
        sticker_resume: false,
        sticker_clicked: HashSet::new(),
        sticker_retried: HashSet::new(),
        sticker_arrivals: 0,
        favorites_pushing: false,
        favorites_again: false,
        favorites_recovered,
        favorites_recovering: false,
        first_names_recovered,
        first_names_recovering: false,
        stars_recovered,
        stars_recovering: false,
        downloads: HashSet::new(),
        read_sync: ReadSync::default(),
        favorite_chats: Default::default(),
        poll_decrypting: 0,
        poll_history: Default::default(),
        poll_sending: HashSet::new(),
        prefetch: prefetch::State::default(),
        older_serial: 0,
        late_older: VecDeque::new(),
        older_ids_match: false,
        interactive_sending: HashMap::new(),
        awaiting_ack: HashMap::new(),
        receipts_watch: None,
        receipts_pruned: Instant::now(),
        early: Default::default(),
        link_watch: Default::default(),
        forward_queue: None,
    };
    worker.load_state();
    worker.backfill();
    worker.backfill_media_albums();
    worker.backfill_video_notes();
    worker.backfill_view_once();
    worker.backfill_motion_photos();
    worker.backfill_hd_copies();
    worker.backfill_group_status_mentions();
    worker.backfill_interactive();
    worker.relocate_media();
    discard_attachment_staging(&worker.dirs.media_cache_dir());
    discard_attachment_staging(&worker.dirs.sticker_cache_dir());
    // Saved stickers are user data, but an interrupted download's staging
    // file never is: a favourite fetched when the process died would
    // otherwise stay there forever, this folder having no other sweep.
    discard_attachment_staging(&worker.dirs.saved_sticker_dir());
    worker.start_bot().await;
    let mut wa_events = wa_events;
    let mut tick = tokio::time::interval(link_watch::CHECK_INTERVAL);
    loop {
        let deadline = worker.sync_deadline;
        // Cloned before the select so its branches never borrow the worker: a live call is the only
        // thing that gives either of these a receiver.
        let mut call_events = worker.call.as_ref().map(|runtime| runtime.events.clone());
        let mut call_frames = worker
            .call
            .as_ref()
            .and_then(|runtime| runtime.frames.clone());
        tokio::select! {
            command = inbox.recv() => {
                match command {
                    Some(Command::Shutdown) | None => break,
                    Some(command) => worker.handle_command(command).await,
                }
            }
            Some(event) = wa_events.recv() => match event {
                RuntimeEvent::WhatsApp(event) => worker.handle_wa_event(event).await,
                RuntimeEvent::MessageRemoval { generation, event } => {
                    worker.handle_session_deletion(generation, event).await;
                }
                RuntimeEvent::PreferencesRecovered {
                    generation,
                    locks,
                    complete,
                } => {
                    worker.preferences_recovered(generation, locks, complete);
                }
                RuntimeEvent::FavoriteChatsRead { generation, complete } => {
                    worker.favorite_chats_read(generation, complete);
                }
                RuntimeEvent::PresenceSent {
                    generation,
                    online,
                    succeeded,
                } => worker.presence_sent(generation, online, succeeded),
            },
            Some(event) = async {
                match call_events.as_mut() {
                    Some(events) => events.recv().await.ok(),
                    None => std::future::pending().await,
                }
            } => worker.call_runtime(event),
            Some(tick) = async {
                match call_frames.as_mut() {
                    Some(frames) => frames.recv().await.ok(),
                    None => std::future::pending().await,
                }
            } => worker.call_frame(tick),
            _ = async {
                match deadline {
                    Some(deadline) => tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)).await,
                    None => std::future::pending::<()>().await,
                }
            } => {
                worker.sync_deadline = None;
                worker.set_syncing(false);
                worker.emit_chats();
            }
            _ = tick.tick() => {
                worker.watch_link();
                worker.reveal_unconfirmed_after_grace();
                worker.settle_presence();
                worker.refresh_legacy_preferences();
                worker.expire_older_requests();
                worker.retry_avatars();
                worker.pump_group_info();
                worker.pump_favorite_stickers();
                worker.resume_sticker_fetches();
                worker.pump_read_sync();
                worker.pump_favorite_chats();
                worker.pump_poll_votes();
                worker.pump_poll_history();
                worker.pump_prefetch();
                worker.prune_waiting_receipts();
                worker.reconcile_call().await;
            }
        }
    }
    worker.shutdown_call().await;
    worker.stop_bot().await;
}

enum RuntimeEvent {
    WhatsApp(Arc<wa_events::Event>),
    MessageRemoval {
        generation: u64,
        event: Arc<wa_events::Event>,
    },
    PresenceSent {
        generation: u64,
        online: bool,
        succeeded: bool,
    },
    PreferencesRecovered {
        generation: u64,
        locks: bool,
        complete: bool,
    },
    /// The one-time read of the phone's favorite chats finished.
    FavoriteChatsRead {
        generation: u64,
        complete: bool,
    },
}

/// Renames an archive whose key is gone, with its SQLite side files, and
/// removes the linked session so the next link replays history into a new
/// archive. Nothing is deleted from the archive: restoring the original
/// keyring and renaming the file back recovers it.
fn set_aside_unreadable_archive(dirs: &AccountDirs) -> std::io::Result<PathBuf> {
    let archive = dirs.archive_db();
    let stamp = jiff::Zoned::now().strftime("%Y%m%d-%H%M%S").to_string();
    let kept = archive.with_file_name(format!("archive-unreadable-{stamp}.db"));
    std::fs::rename(&archive, &kept)?;
    for suffix in ["-wal", "-shm", "-journal"] {
        let mut from = archive.clone().into_os_string();
        from.push(suffix);
        let mut to = kept.clone().into_os_string();
        to.push(suffix);
        match std::fs::rename(&from, &to) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error),
            _ => {}
        }
    }
    let session = dirs.session_db();
    for suffix in ["", "-wal", "-shm", "-journal"] {
        let mut path = session.clone().into_os_string();
        path.push(suffix);
        match std::fs::remove_file(path) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error),
            _ => {}
        }
    }
    Ok(kept)
}

/// A short explanation for a failed invite lookup or join. Protocol errors
/// can carry identifiers, so only the kind of failure is shown.
fn invite_error(error: &str) -> String {
    let error = error.to_ascii_lowercase();
    if error.contains("401") || error.contains("not-authorized") {
        "This link was reset or you are not allowed to join.".to_owned()
    } else if error.contains("404") || error.contains("item-not-found") || error.contains("invalid")
    {
        "This invite link is invalid or has been reset.".to_owned()
    } else if error.contains("410") || error.contains("gone") {
        "This invite link has expired.".to_owned()
    } else if error.contains("409") || error.contains("conflict") {
        "You are already in this group.".to_owned()
    } else if error.contains("406") || error.contains("full") {
        "This group is full.".to_owned()
    } else {
        "WhatsApp could not open this invite link. Try again later.".to_owned()
    }
}

/// How long private content waits for phone lock state before it is shown
/// unconfirmed. A healthy sync answers well within this.
const PRIVACY_GRACE: Duration = Duration::from_secs(10);

/// How long ZapFast stays "available" after the window loses focus.
const PRESENCE_LINGER: Duration = Duration::from_secs(10);

/// The window owns every presence transition (#461). Left at the default
/// `Automatic` policy, the client announces "available" whenever it connects
/// or the push name syncs, so an account whose window sits in the tray still
/// shows online and the phone holds back its own notifications. ZapFast
/// announces through `Command::SetOnline` instead.
///
/// Public so the integration test in `tests/presence.rs` drives the same
/// builder step `start_bot` uses.
pub fn host_owned_presence<B, T, H, R>(
    builder: whatsapp_rust::bot::BotBuilder<B, T, H, R>,
) -> whatsapp_rust::bot::BotBuilder<B, T, H, R> {
    builder.with_presence_policy(PresencePolicy::Manual)
}

/// Waits longer after each failed lock-state recovery, so a collection the
/// server keeps refusing is not rebuilt every few seconds.
fn privacy_backoff(attempts: u32) -> Duration {
    Duration::from_secs(30)
        .saturating_mul(1 << attempts.saturating_sub(1).min(5))
        .min(Duration::from_secs(15 * 60))
}

struct UiEvents {
    sender: mpsc::UnboundedSender<RuntimeEvent>,
    generation: u64,
}

impl wa_events::EventHandler for UiEvents {
    fn handle_event(&self, event: Arc<wa_events::Event>) {
        let event = if matches!(
            event.as_ref(),
            wa_events::Event::DeleteMessageForMeUpdate(_)
        ) {
            RuntimeEvent::MessageRemoval {
                generation: self.generation,
                event,
            }
        } else {
            RuntimeEvent::WhatsApp(event)
        };
        let _ = self.sender.send(event);
    }
}

/// A transcript read whose answer was withheld with the rest of the private
/// content, to be read again once content is shown.
#[derive(Clone, Debug, PartialEq)]
enum WithheldPage {
    /// `Command::LoadChat`.
    Page(ChatId, Option<super::PageKey>),
    /// `Command::LoadUntil`.
    Until(ChatId, String, super::PageKey),
    /// `Command::ListChatShared`, with its request.
    Media(ChatId, u64),
}

/// A failed attempt cannot settle any earlier attempt whose reply was lost.
fn message_removal_outcome(
    result: Result<(), whatsapp_rust::AppStateError>,
) -> MessageRemovalOutcome {
    match result {
        Ok(()) => MessageRemovalOutcome::Accepted,
        Err(_) => MessageRemovalOutcome::Uncertain,
    }
}

/// See `Worker::edits`.
#[derive(Default)]
struct EditGenerations {
    sent: u64,
    applied: u64,
}

#[derive(Clone, Copy)]
struct FileSendOptions {
    documents: bool,
    view_once: bool,
}

struct Worker {
    /// Private content may reach the UI.
    privacy_ready: bool,
    /// Phone lock state is known to be mirrored in the archive.
    privacy_confirmed: bool,
    /// Lock state must be rebuilt from a snapshot rather than caught up.
    privacy_snapshot: bool,
    /// When content is shown even though lock state is still unconfirmed.
    privacy_reveal_at: Option<Instant>,
    privacy_attempts: u32,
    privacy_warned: bool,
    privacy_recovering: bool,
    privacy_generation: u64,
    /// Serializes requests for one message, including across privacy-id mapping.
    message_removals_in_flight: HashSet<(ChatId, String)>,
    contact_generation: u64,
    privacy_retry: Instant,
    /// Latest star or unstar attempt per message. A finished request whose
    /// generation is older than this is dropped.
    star_generation: HashMap<(String, String), u64>,
    /// The account the star requests in flight belong to. A task that keeps
    /// the client of the account that left can finish after the next one has
    /// linked, and the per-message generations start over there, so the
    /// session has to be checked as well.
    star_session: u64,
    /// Latest pin or unpin attempt per message. A finished request whose
    /// generation is older than this is dropped.
    pin_generation: HashMap<(String, String), u64>,
    /// Transcript pages asked for while private content was withheld. Their
    /// answers never reached the interface, which still waits for them, so
    /// they are read again once content is shown (#180).
    withheld_pages: Vec<WithheldPage>,
    read_sync: ReadSync,
    /// Sending favorite chats to the phone, and reading its list once.
    favorite_chats: favorite_chats::FavoriteChats,
    poll_decrypting: usize,
    poll_history: poll_history::Requests,
    poll_sending: HashSet<(ChatId, String)>,
    interactive_sending: HashMap<(ChatId, String), String>,
    /// Messages we sent that WhatsApp has not acknowledged yet, by id, with
    /// their chat: the first ack's server time replaces their local stamp.
    awaiting_ack: HashMap<String, ChatId>,
    /// The group message whose "Message info" is open.
    receipts_watch: Option<(ChatId, String)>,
    /// When receipts that never found their message were last dropped.
    receipts_pruned: Instant,
    /// The phone's reads and reactions waiting for their message.
    early: early_events::EarlyEvents,
    /// Notices a link that stays open after a sleep but carries nothing.
    link_watch: link_watch::LinkWatch,
    /// Serial background phone-history prefetch.
    prefetch: prefetch::State,
    /// Numbers phone-history requests, so a send result or a late answer is
    /// matched to the request it belongs to rather than to whichever request
    /// the chat has now.
    older_serial: u64,
    /// Requests that timed out but whose answer may still arrive, oldest
    /// first and bounded. A late answer is matched to one of these instead of
    /// being taken for a newer request of the same chat.
    late_older: VecDeque<LateOlder>,
    /// Whether an answer has carried the id of the request it answers. Until
    /// one has, an answer with an unknown id is still matched by its chat.
    older_ids_match: bool,
    dirs: AccountDirs,
    events: std::sync::mpsc::Sender<Event>,
    commands: mpsc::UnboundedSender<Command>,
    waker: Waker,
    archive: Archive,
    client: Option<Arc<Client>>,
    handle: Option<BotHandle>,
    wa_sender: mpsc::UnboundedSender<RuntimeEvent>,
    me_pn: Option<String>,
    me_lid: Option<String>,
    me_name: Option<String>,
    me_about: Option<String>,
    /// Privacy-id user part to phone-number user part.
    lid_to_pn: HashMap<String, String>,
    contacts: HashMap<String, Contact>,
    status: LinkStatus,
    pairing_phone: Option<String>,
    pair_code: Option<String>,
    qr: Option<String>,
    syncing: bool,
    sync_deadline: Option<Instant>,
    /// Groups queued or already requested. Queries are rate-limited.
    group_info_requested: HashSet<String>,
    /// Pending group metadata queue.
    group_info_queue: std::collections::VecDeque<String>,
    /// Group metadata attempt counts.
    /// Bumped whenever a leave is confirmed. `query_group_info` runs in a
    /// spawned task, so metadata asked for before the leave can land after it;
    /// the snapshot carries the generation it was issued in and a stale one
    /// cannot resurrect the chat.
    leave_generation: HashMap<String, u64>,
    /// Bumped whenever a rename made here is confirmed, for the same reason:
    /// metadata asked for before it must not bring the old subject back.
    subject_generation: HashMap<String, u64>,
    /// Bumped whenever a permission or member change made here is accepted:
    /// metadata asked for before it (the library joins a query already in
    /// flight) must not bring the old permissions or members back.
    settings_generation: HashMap<String, u64>,
    /// The photo each group was last sent, kept until WhatsApp answers so an
    /// accepted one shows at once rather than after a lookup the server may
    /// still answer with the old picture.
    group_pictures_sent: HashMap<ChatId, Vec<u8>>,
    /// Description notices must outlive metadata requested before they arrived.
    description_generation: HashMap<String, u64>,
    /// Superseded metadata operations cannot update the archive or retry queue.
    group_info_generation: HashMap<String, u64>,
    /// The newest edit sent from here and the newest one applied, per
    /// message. The archive changes only when WhatsApp accepts an edit.
    edits: HashMap<(ChatId, String), EditGenerations>,
    edit_sequence: u64,
    group_info_tries: HashMap<String, u32>,
    /// Next retry time for failed group metadata requests.
    group_info_retry: Vec<(Instant, String)>,
    presence_subscribed: HashSet<String>,
    /// Chosen folder for new downloads, when not the cache.
    download_folder: Option<PathBuf>,
    /// Clean each chosen root once, before this worker can start downloads there.
    cleaned_download_folders: HashSet<PathBuf>,
    /// Settings' "Keep chats archived", for an account whose phone has not
    /// sent its own. Off, a new message unarchives its chat here, as the
    /// phone does; WhatsApp sends no mutation for it.
    keep_chats_archived: bool,
    /// The phone's own "Keep chats archived", inverted as WhatsApp sends it
    /// (`Some(true)`: a new message unarchives). It wins over Settings'.
    phone_unarchive_chats: Option<bool>,
    /// The stored phone setting could not be read at startup; it is read
    /// again when needed, and chats stay archived until it is.
    phone_unarchive_unread: bool,
    /// Whether the window is focused and visible.
    online_wanted: bool,
    /// When `online_wanted` last changed.
    online_changed: Instant,
    /// The presence last confirmed on this connection.
    online_sent: Option<bool>,
    /// A presence request currently awaiting its result.
    presence_in_flight: bool,
    /// Push-name sync can trigger a library-owned availability announcement.
    presence_refresh_pending: bool,
    /// Invalidates completions from an older connection.
    presence_generation: u64,
    /// Bumped by each connection's followed-channel query, so the answer of
    /// an earlier one cannot rename or drop channels after a newer one.
    channel_list_generation: u64,
    /// Pending phone-history requests by chat.
    pending_older: HashMap<ChatId, OlderRequest>,
    /// Chats already told that the phone did not answer; cleared when the
    /// phone sends that chat's history or the link reconnects.
    older_warned: HashSet<ChatId>,
    /// Deferred profile-picture requests and retry counts.
    pending_avatars: HashMap<(String, bool), u32>,
    /// Followed channels' pictures, which no profile-picture lookup finds.
    channel_pictures: channel_pictures::ChannelPictures,
    /// Active recent-sticker downloads by hash.
    sticker_fetches: HashSet<String>,
    /// Active chat-sticker downloads by chat and message id.
    sticker_downloads: HashSet<(ChatId, String)>,
    /// Content hashes of the stickers last listed in Recent, by file.
    recent_hashes: HashMap<PathBuf, String>,
    /// Sticker emojis by file, with the size and time they were read at.
    emoji_cache: HashMap<PathBuf, EmojiStamp>,
    /// Favorite stickers being fetched from the phone's list, by hash.
    favorite_fetches: HashSet<String>,
    /// Favorites waiting for their turn, and the pause the server asked for.
    sticker_pace: sticker_pace::Pace,
    /// Recent stickers whose download failed this session, not asked again.
    sticker_failed: HashSet<String>,
    /// Chat stickers whose picker download failed this session, not asked
    /// again.
    sticker_download_failed: HashSet<(ChatId, String)>,
    /// The server paused picker downloads with some left to fetch; the tick
    /// fetches them once the pause is over.
    sticker_resume: bool,
    /// Chat stickers clicked while a picker download of theirs was in
    /// flight, downloaded again, re-upload and all, if that one fails.
    sticker_clicked: HashSet<(ChatId, String)>,
    /// Those clicked stickers downloading again; one that arrives refreshes
    /// the picker's shelves, as a picker download would.
    sticker_retried: HashSet<(ChatId, String)>,
    /// Picker stickers that arrived since the shelves were last sent.
    sticker_arrivals: usize,
    /// Whether favorite changes are on their way to the phone.
    favorites_pushing: bool,
    /// More favorite changes arrived while a push was running.
    favorites_again: bool,
    /// The phone's favorites from before ZapFast followed them were replayed.
    favorites_recovered: bool,
    /// That replay is running.
    favorites_recovering: bool,
    /// The phone's contacts from before first names were kept were replayed.
    first_names_recovered: bool,
    /// That replay is running.
    first_names_recovering: bool,
    /// The phone's stars from before ZapFast followed them were replayed.
    stars_recovered: bool,
    /// That replay is running.
    stars_recovering: bool,
    /// Active attachment downloads by chat, message id, and carousel card.
    downloads: HashSet<(ChatId, String, Option<usize>)>,
    /// Serial forward in flight. The next send waits for the running one.
    forward_queue: Option<ForwardQueue<ForwardJob>>,
    call: Option<calls::CallRuntime>,
    /// Group calls ringing on the phone that this device does not join while group calls are off
    /// (`calls::GROUP_CALLS_ENABLED`): kept only so each is filed in its group as it ends, as a
    /// call that rang here and was answered elsewhere or missed is. Oldest first, a few at most.
    phone_only_calls: Vec<crate::calls::Call>,
    /// The devices the call screen was last handed, kept so a device that goes away can be named
    /// by the description the user saw in the picker rather than by its node name.
    call_devices: crate::calls::DeviceList,
}

/// A queued forward: where it goes, the protobuf, and its disappearing timer.
type ForwardJob = (ChatId, Jid, wa::Message, Option<u32>);

/// Pure queue behind a serial forward. `T` is one job's payload.
struct ForwardQueue<T> {
    remaining: VecDeque<(String, T)>,
    current: Option<String>,
}

enum ForwardStep<T> {
    /// The ack belongs to something else.
    Ignore,
    Next {
        id: String,
        payload: T,
    },
    Finished,
}

impl<T> ForwardQueue<T> {
    fn new() -> Self {
        Self {
            remaining: VecDeque::new(),
            current: None,
        }
    }

    /// Queues a batch and returns the job to start now, when the queue is idle.
    /// A batch that arrives while one runs waits behind it, in order.
    fn push(&mut self, jobs: Vec<(String, T)>) -> Option<(String, T)> {
        let mut jobs = jobs.into_iter();
        if self.current.is_some() {
            self.remaining.extend(jobs);
            return None;
        }
        let (id, payload) = jobs.next()?;
        self.current = Some(id.clone());
        self.remaining.extend(jobs);
        Some((id, payload))
    }

    fn ack(&mut self, id: &str) -> ForwardStep<T> {
        if self.current.as_deref() != Some(id) {
            return ForwardStep::Ignore;
        }
        match self.remaining.pop_front() {
            Some((next, payload)) => {
                self.current = Some(next.clone());
                ForwardStep::Next { id: next, payload }
            }
            None => {
                self.current = None;
                ForwardStep::Finished
            }
        }
    }
}

/// Decoded history chunk waiting to be canonicalized and archived.
struct ParsedHistory {
    chats: Vec<ParsedChat>,
    push_names: Vec<(String, String)>,
    lids: Vec<(String, String)>,
    /// Recent phone stickers included with history sync.
    stickers: Vec<wa::StickerMetadata>,
}

/// A phone-history request waiting for its chunk.
struct OlderRequest {
    asked: Instant,
    /// The oldest archived message when it was asked.
    before: super::PageKey,
    /// Whether the reader asked by scrolling to the top. Only these report a
    /// phone that does not answer.
    explicit: bool,
    /// Asked by the background prefetch: its answer fills the archive without
    /// moving the view or counting against the phone. A reader who asks for
    /// the same chat while it waits takes it over.
    background: bool,
    /// Tells this request's send result from a later request's.
    serial: u64,
    /// The id the request went out under, once the send returns. The phone's
    /// answer names it, which is how the answer is matched to the request.
    request: Option<String>,
}

/// A request that timed out, kept in case its answer still arrives.
struct LateOlder {
    chat: ChatId,
    serial: u64,
    background: bool,
    request: Option<String>,
}

/// How many timed-out requests are remembered for a late answer.
const LATE_OLDER_LIMIT: usize = 32;

/// The request an on-demand history answer belongs to.
#[derive(Debug, PartialEq)]
enum AnswerOwner {
    /// The chat's request still waiting.
    Pending,
    /// A request that timed out, by its place in `late_older`.
    Late(usize),
    /// No request here asked for it.
    Unclaimed,
}

struct ParsedChat {
    diagnostics: HistoryDiagnostics,
    id: String,
    name: Option<String>,
    unread: Option<u32>,
    /// The phone's unread mark; `None` when the chunk omits it.
    marked_unread: Option<bool>,
    /// `None` means the history chunk omitted archive metadata.
    archived: Option<bool>,
    pinned_at: Option<i64>,
    /// Outer None means the history chunk omitted mute metadata.
    muted_until: Option<Option<i64>>,
    /// `None` means the history chunk omitted lock metadata.
    locked: Option<bool>,
    ephemeral_expiration: Option<u32>,
    ephemeral_setting_timestamp: Option<i64>,
    last_activity: i64,
    pn_jid: Option<String>,
    lid_jid: Option<String>,
    /// Whether the phone reports more available history.
    more_on_phone: Option<bool>,
    messages: Vec<ParsedMessage>,
    revoked: Vec<String>,
    poll_updates: Vec<HistoryPollUpdate>,
    reactions: Vec<HistoryReaction>,
    clips: Vec<HistoryClip>,
    pins: Vec<HistoryPin>,
}

/// A motion photo's clip from history, filed for its photo.
struct HistoryClip {
    parent: String,
    sender: Option<String>,
    from_me: bool,
    raw: Vec<u8>,
}

/// Counts only. Never put message keys, bodies, names, or raw protobufs here:
/// this summary is safe to request from a user's ordinary application log.
#[derive(Debug, Default)]
struct HistoryDiagnostics {
    entries: usize,
    missing_envelope: usize,
    missing_key: usize,
    missing_body: usize,
    ciphertext_stubs: usize,
    futureproof_body: usize,
    edited_wrappers: usize,
    edit_snapshots: usize,
    rekeyed_edit_snapshots: usize,
    invalid_edits: usize,
    edit_without_key: usize,
    edit_without_body: usize,
    encrypted_edits: usize,
    other_protocol: usize,
    unclassified: usize,
}

struct HistoryPollUpdate {
    id: String,
    sender: Option<String>,
    from_me: bool,
    timestamp: i64,
    update: wa::message::PollUpdateMessage,
}

/// A pin or unpin the phone's history carries for one message.
struct HistoryPin {
    target: String,
    pinned: bool,
    duration_secs: u32,
    at: i64,
    /// Who pinned it, for the notice the chat shows.
    pinner: crate::archive::Pinner,
}

/// Standalone reaction from history, applied after the parent row is stored.
struct HistoryReaction {
    target: String,
    sender: Option<String>,
    from_me: bool,
    body: HistoryReactionBody,
    /// When it was sent, in milliseconds.
    sent_at: i64,
}

enum HistoryReactionBody {
    Plain(String),
    Encrypted { payload: Vec<u8>, iv: Vec<u8> },
}

struct ParsedMessage {
    id: String,
    sender: Option<String>,
    from_me: bool,
    push_name: Option<String>,
    timestamp: i64,
    history_order: Option<i64>,
    edited: bool,
    content: Content,
    status: Delivery,
    quoted: Option<Quoted>,
    reactions: Vec<(Option<String>, bool, String)>,
    mentions: Vec<String>,
    forwarded: bool,
    thumbnail: Option<Vec<u8>>,
    raw: Vec<u8>,
    poll_secret: Option<Vec<u8>>,
    poll_votes: Vec<wa::PollUpdate>,
    /// The phone's per-recipient receipts for one of our messages. A group's
    /// list may name only some of its members.
    receipts: Vec<wa::UserReceipt>,
    /// An edit made before linking that history carries encrypted with the
    /// original's secret (#582). The worker opens it before filing, and the
    /// row's body comes from it; one it cannot open is not filed.
    sealed_edit: Option<SealedEdit>,
}

/// An encrypted MESSAGE_EDIT from history, still to be opened. Its message
/// is filed under the edited message's id, as a full snapshot.
struct SealedEdit {
    payload: Vec<u8>,
    iv: Vec<u8>,
    /// The history entry's own message secret, which may be the original's.
    entry_secret: Option<Vec<u8>>,
}

/// How many of a chat's newest messages are searched for one the phone has.
const PHONE_RANGE_LOOKBACK: usize = 20;

/// The range WhatsApp checks before it archives or unarchives a chat on the
/// phone: the chat's newest message. A phone left the chat in place when
/// the change came without one. Local call entries were never sent, so they
/// give their time but name no message, and a message without a time is
/// treated as none.
fn archive_range(jid: &Jid, last: Option<&Message>) -> whatsapp_rust::SyncActionMessageRange {
    let Some(last) = last.filter(|last| last.timestamp > 0) else {
        return whatsapp_rust::message_range(crate::util::now(), None, Vec::new());
    };
    let participant = (jid.is_group() && !last.from_me)
        .then(|| Worker::jid_of(&last.sender))
        .flatten();
    let key = (!last.id.starts_with(crate::model::CALL_ENTRY_PREFIX))
        .then(|| whatsapp_rust::MessageId::new(&last.id).ok())
        .flatten()
        .and_then(|id| {
            whatsapp_rust::MessageRef::new(jid, id, participant.as_ref(), last.from_me).ok()
        });
    whatsapp_rust::message_range(last.timestamp, None, key.map(|key| (key, last.timestamp)))
}

impl Worker {
    fn ephemeral_expiration(&self, chat: &str) -> Option<u32> {
        self.archive
            .ephemeral_expiration(chat)
            .ok()
            .flatten()
            .filter(|expiration| *expiration > 0)
    }

    fn default_ephemeral_expiration(&self) -> Option<u32> {
        self.archive
            .meta("default_ephemeral_expiration")
            .ok()
            .flatten()
            .and_then(|value| value.parse().ok())
    }

    fn apply_ephemeral(&self, chat: &str, message: &mut wa::Message) -> Option<u32> {
        apply_ephemeral_expiration(message, self.ephemeral_expiration(chat))
    }

    fn emit(&self, mut event: Event) {
        if let Event::Syncing(syncing) = &mut event {
            *syncing |= !self.privacy_ready;
        }
        // An upgraded archive has no reliable lock state until the library's
        // authenticated replay has completed. Keep private content off the UI
        // and out of notifications during recovery, including failed retries.
        // A call is withheld too: its chat may be locked, and until the lock collection is loaded
        // the interface cannot know it, so an incoming call would take the window and name a
        // hidden caller. It is re-sent from `reveal_private_content` once the locks are known, so
        // a call that arrived mid-recovery still rings rather than being lost.
        if !self.privacy_ready
            && matches!(
                event,
                Event::Chats(_)
                    | Event::Drafts(_)
                    | Event::ChatHits { .. }
                    | Event::ChatUpdated(_)
                    | Event::Messages { .. }
                    | Event::MessageUpdated(_)
                    | Event::Incoming { .. }
                    | Event::Contacts(_)
                    | Event::SearchHits { .. }
                    | Event::Labels(_)
                    | Event::Typing { .. }
                    | Event::Call(_)
                    | Event::PhoneOnlyCall(_)
                    | Event::CallDevices(_)
                    | Event::StarredList(_)
                    | Event::Pins { .. }
                    | Event::ChatShared { .. }
                    | Event::ChatSharedChanged(_)
                    | Event::ChatMedia { .. }
            )
        {
            return;
        }
        let _ = self.events.send(event);
        self.waker.wake();
    }

    /// Syncs a chat setting to the phone without blocking the worker.
    fn tell_phone<F, Fut>(&self, chat: &str, call: F)
    where
        F: FnOnce(Arc<Client>, Jid) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Result<(), String>> + Send + 'static,
    {
        let (Some(client), Some(jid)) = (self.client.clone(), Self::jid_of(chat)) else {
            return;
        };
        tokio::spawn(async move {
            if call(client, jid).await.is_err() {
                log::warn!("could not synchronize a chat preference");
            }
        });
    }

    /// The chat's newest message the phone has: a send still pending or
    /// failed here may never have reached it.
    fn newest_on_phone(&self, chat: &str) -> Result<Option<Message>, rusqlite::Error> {
        Ok(self
            .archive
            .messages(chat, None, PHONE_RANGE_LOOKBACK)?
            .into_iter()
            .rev()
            .find(|message| {
                !(message.from_me && matches!(message.status, Delivery::Pending | Delivery::Failed))
            }))
    }

    /// Archives or unarchives a chat on the phone, naming its newest message.
    /// A chat whose messages cannot be read is left alone there: a range made
    /// up without them could move it past messages the phone has.
    fn archive_on_phone(&self, chat: &str, archived: bool) {
        let last = match self.newest_on_phone(chat) {
            Ok(last) => last,
            Err(error) => {
                log::warn!("could not read a chat to archive it on the phone: {error}");
                return;
            }
        };
        self.tell_phone(chat, move |client, jid| async move {
            let range = Some(archive_range(&jid, last.as_ref()));
            if archived {
                client.chat_actions().archive_chat(&jid, range).await
            } else {
                client.chat_actions().unarchive_chat(&jid, range).await
            }
            .map_err(|error| error.to_string())
        });
    }

    /// Stars or unstars one message for every linked device.
    fn next_star_generation(&mut self, chat: &str, id: &str) -> u64 {
        let slot = self
            .star_generation
            .entry((chat.to_owned(), id.to_owned()))
            .or_insert(0);
        *slot = slot.wrapping_add(1);
        *slot
    }

    fn set_star(&mut self, chat: ChatId, id: String, starred: bool) {
        let generation = self.next_star_generation(&chat, &id);
        let session = self.star_session;
        let commands = self.commands.clone();
        let (Some(client), Some(jid)) = (self.client.clone(), Self::jid_of(&chat)) else {
            // The batch counts every attempt, so a refusal it never heard
            // about would leave its toast open. Report it like any other.
            let _ = commands.send(Command::Starred {
                chat,
                message: id,
                starred,
                generation,
                session,
                result: Err("Not connected to WhatsApp".to_owned()),
            });
            return;
        };
        let Ok(Some(target)) = self.archive.message(&chat, &id) else {
            let _ = commands.send(Command::Starred {
                chat,
                message: id,
                starred,
                generation,
                session,
                result: Err("This message is not on this computer".to_owned()),
            });
            return;
        };
        let from_me = target.from_me;
        let participant = (jid.is_group() && !from_me)
            .then(|| target.sender.clone())
            .and_then(|sender| Self::jid_of(&sender));
        tokio::spawn(async move {
            let actions = client.chat_actions();
            let target = whatsapp_rust::MessageId::new(&id).and_then(|message_id| {
                whatsapp_rust::MessageRef::new(&jid, message_id, participant.as_ref(), from_me)
            });
            let result = match target {
                Ok(target) if starred => actions
                    .star_message(&target)
                    .await
                    .map_err(|error| error.to_string()),
                Ok(target) => actions
                    .unstar_message(&target)
                    .await
                    .map_err(|error| error.to_string()),
                // Only an empty id or a group message without its author fails here.
                Err(error) => Err(error.to_string()),
            };
            let _ = commands.send(Command::Starred {
                chat,
                message: id,
                starred,
                generation,
                session,
                result,
            });
        });
    }

    /// A star or unstar made on the phone, or on another linked device.
    fn star_update(&mut self, update: &wa_events::StarUpdate) {
        let chat = self.canonical(&update.chat_jid);
        let id = update.message_id.clone();
        let starred = update.action.starred.unwrap_or(false);
        let at = update.timestamp.timestamp();
        // An older event loses to the stored time and must not cancel a
        // request that is still in flight. Only a row that actually changed
        // retires that request.
        match self
            .archive
            .set_star(&chat, &id, starred, at, update.from_full_sync)
        {
            Ok(true) => {
                self.next_star_generation(&chat, &id);
                self.emit(Event::StarChanged {
                    chat,
                    message: id,
                    starred,
                    local: false,
                });
            }
            Ok(false) => {}
            Err(error) => log::warn!("could not store a star from the phone: {error}"),
        }
    }

    /// Deletes attachment files that belonged to a removed chat. Only the
    /// app's own media cache is touched; anything the user saved elsewhere
    /// stays where it is.
    fn drop_cached_media(&self, paths: &[std::path::PathBuf]) {
        let Ok(cache) = self.dirs.media_cache_dir().canonicalize() else {
            return;
        };
        for path in paths {
            let Ok(path) = path.canonicalize() else {
                continue;
            };
            if !path.starts_with(&cache) {
                continue;
            }
            if let Err(error) = std::fs::remove_file(path)
                && error.kind() != std::io::ErrorKind::NotFound
            {
                log::warn!("could not remove a cached attachment");
            }
        }
    }

    /// Deletes a chat and stops everything that could still bring it back.
    fn remove_chat(&mut self, chat: &str, through: i64, delete_media: bool) {
        // A group we left would otherwise keep being asked for metadata and
        // log a 403 or 404 for every attempt.
        self.group_info_queue.retain(|id| id != chat);
        self.group_info_retry.retain(|(_, id)| id != chat);
        self.group_info_requested.remove(chat);
        self.group_info_tries.remove(chat);
        match self.archive.remove_chat_through(chat, through, true) {
            Ok(removed) => {
                self.pending_older.remove(chat);
                self.late_older.retain(|late| late.chat != chat);
                self.prefetch.forget_chat(chat);
                if delete_media {
                    self.drop_cached_media(&removed.media);
                }
                if removed.existed {
                    if self.archive.chat(chat).ok().flatten().is_none() {
                        log::info!("chat removal: deleted cached chat");
                        self.emit(Event::ChatRemoved {
                            chat: chat.to_owned(),
                        });
                    } else {
                        log::info!("chat removal: retained messages newer than deletion boundary");
                        self.emit(Event::ChatCleared {
                            chat: chat.to_owned(),
                            through,
                        });
                        self.emit_chat(chat);
                        // The older pictures, documents and links went with
                        // the deletion; a full removal closes the panel
                        // through `ChatRemoved` instead.
                        self.emit(Event::ChatSharedChanged(chat.to_owned()));
                    }
                } else {
                    log::info!("chat removal: no matching cached chat");
                }
            }
            Err(_error) => log::warn!("could not delete a chat"),
        }
    }

    /// Empties a chat while keeping it listed.
    /// Empties a chat through `through`; false when the archive could not.
    fn empty_chat(&mut self, chat: &str, through: i64, delete_media: bool) -> bool {
        match self.archive.remove_chat_through(chat, through, false) {
            Ok(removed) => {
                self.pending_older.remove(chat);
                self.late_older.retain(|late| late.chat != chat);
                self.prefetch.forget_chat(chat);
                if delete_media {
                    self.drop_cached_media(&removed.media);
                }
                if removed.existed {
                    self.emit(Event::ChatCleared {
                        chat: chat.to_owned(),
                        through,
                    });
                    self.emit_chat(chat);
                    self.emit(Event::ChatSharedChanged(chat.to_owned()));
                }
                true
            }
            Err(_error) => {
                log::warn!("could not clear a chat");
                false
            }
        }
    }

    /// Whether a message predates the deletion or clear of its chat.
    fn predates_removal(&self, chat: &str, timestamp: i64) -> bool {
        self.archive
            .removal_point(chat)
            .ok()
            .flatten()
            .is_some_and(|through| timestamp <= through)
    }

    /// Blocks a deleted message's replay, failing closed if the barrier cannot be read.
    fn message_was_removed(&self, chat: &str, id: &str) -> bool {
        match self.archive.message_removed(chat, id) {
            Ok(removed) => removed,
            Err(_) => {
                log::warn!("could not check a message's deletion state");
                true
            }
        }
    }

    fn emit_chats(&mut self) {
        match self.archive.chats() {
            Ok(mut chats) => {
                // Early preference sync can create an empty privacy-id row.
                // Once mapped, its preferences live on the canonical chat.
                chats.retain(|chat| chat.last.is_some() || self.canonical_str(&chat.id) == chat.id);
                for chat in &mut chats {
                    self.polish_chat(chat);
                }
                self.prefetch.set_chats(&chats);
                self.emit(Event::Chats(chats));
                self.emit_labels();
                self.emit(Event::Drafts(self.archive.drafts().unwrap_or_default()));
            }
            Err(error) => log::warn!("could not list chats: {error}"),
        }
    }

    /// Every label in creation order, so the UI can draw tabs and menus.
    fn emit_labels(&self) {
        match self.archive.labels() {
            Ok(labels) => self.emit(Event::Labels(labels)),
            Err(error) => log::warn!("could not list labels: {error}"),
        }
    }

    /// Creates a label. The app refuses a full set or a taken name first,
    /// in the user's language; the archive checks again and says nothing.
    fn create_label(&mut self, name: String, color_hex: String) {
        match self
            .archive
            .create_label(&name, &color_hex, crate::util::now())
        {
            Ok(Some(_)) => self.emit_labels(),
            Ok(None) => log::info!("label not created: full, empty, or taken"),
            Err(error) => log::warn!("could not create label: {error}"),
        }
    }

    /// Hands the starred list to the interface.
    fn emit_starred(&mut self) {
        match self.archive.starred(STARRED_LIMIT) {
            Ok(list) => self.emit(Event::StarredList(list)),
            Err(error) => self.emit(Event::Error(error.to_string())),
        }
    }

    fn emit_chat(&self, id: &str) {
        if let Ok(Some(mut chat)) = self.archive.chat(id) {
            self.polish_chat(&mut chat);
            self.emit(Event::ChatUpdated(Box::new(chat)));
        }
    }

    /// Hands the chat's viewer album to the interface, centred on `around`
    /// when the viewer opened on a message rather than on the newest page.
    fn emit_chat_media(&mut self, chat: ChatId, around: Option<String>) {
        let page = match around.as_deref() {
            Some(message) => {
                self.archive
                    .gallery_around(&chat, message, crate::archive::GALLERY_PAGE)
            }
            None => self
                .archive
                .gallery_media(&chat, crate::archive::GALLERY_PAGE),
        };
        match page {
            Ok(items) => self.emit(Event::ChatMedia {
                chat,
                around,
                items,
            }),
            Err(error) => self.emit(Event::Error(error.to_string())),
        }
    }

    /// Resolves phone numbers in chat-row previews.
    fn polish_chat(&self, chat: &mut Chat) {
        if let Some(last) = chat.last.as_mut() {
            last.summary = self.pn_tokens(&last.summary);
            last.full = self.pn_tokens(&last.full);
        }
        chat.labels = self.archive.chat_labels(&chat.id).unwrap_or_default();
        if chat.is_group() && chat.unread > 0 {
            let ours: Vec<&str> = [self.me_pn.as_deref(), self.me_lid.as_deref()]
                .into_iter()
                .flatten()
                .collect();
            chat.unread_mention = self
                .archive
                .unread_mention(&chat.id, chat.unread, &ours)
                .unwrap_or_default();
        }
    }

    fn emit_message(&self, chat: &str, id: &str) {
        if let Ok(Some(mut message)) = self.archive.message(chat, id) {
            self.polish(&mut message);
            self.emit(Event::MessageUpdated(Box::new(message)));
        }
    }

    /// Moves a message we sent to the time WhatsApp's server acknowledged it.
    /// Only the first ack with a time counts: a resend after a retry receipt
    /// is acked again, later, and must not move the message down the chat.
    /// An ack without a usable time leaves the message waiting for one.
    fn server_ack(&mut self, ack: &wa_events::ServerAck) {
        if ack.class.as_deref() != Some("message") {
            return;
        }
        let Some(at) = ack.timestamp.filter(|_| ack.error.is_none()) else {
            return;
        };
        let Some(chat) = self.awaiting_ack.get(&ack.id).cloned() else {
            return;
        };
        match self.archive.restamp_sent(&chat, &ack.id, at.timestamp()) {
            Ok(changed) => {
                self.awaiting_ack.remove(&ack.id);
                if changed {
                    self.emit_message(&chat, &ack.id);
                    self.emit_chat(&chat);
                }
            }
            // Still awaited, so a later ack can place it.
            Err(error) => log::warn!("could not move a sent message to the server's time: {error}"),
        }
    }

    /// Tells the interface that what the info panel lists for `chat` may
    /// have changed, when a message went from or to a picture, a video, a
    /// document or a linked text.
    fn media_changed(&self, chat: &str, old: Option<&Content>, new: Option<&Content>) {
        if old.is_some_and(Content::listable) || new.is_some_and(Content::listable) {
            self.emit(Event::ChatSharedChanged(chat.to_owned()));
        }
    }

    fn set_status(&mut self, status: LinkStatus) {
        if self.status != status {
            log::info!("link: {}", status.log_label());
            if matches!(status, LinkStatus::Connected) {
                // The phone may have history for us again.
                self.prefetch.on_connected();
            }
            self.status = status.clone();
            self.emit(Event::Link(status));
        }
    }

    /// Reconnects a connected link after the machine woke up, as the system
    /// reports it (fastframe-power's `Resumed`, on Linux and Windows). See
    /// `link_watch`.
    fn resumed(&mut self) {
        let Some(client) = self
            .client
            .clone()
            .filter(|_| matches!(self.status, LinkStatus::Connected))
        else {
            return;
        };
        log::info!("link: the machine woke up, reconnecting");
        self.set_status(LinkStatus::Connecting);
        tokio::spawn(async move { client.reconnect_immediately().await });
    }

    /// Reconnects a link that the machine slept under (macOS, where the
    /// system does not report the wake yet), or that has received nothing
    /// for longer than a working one can, or whose socket was lost without a
    /// matching disconnect event. See `link_watch`.
    fn watch_link(&mut self) {
        let client = self
            .client
            .clone()
            .filter(|_| matches!(self.status, LinkStatus::Connected));
        let frames = client.as_ref().map(|client| client.stats().frames_received);
        let verdict = self.link_watch.check(
            Instant::now(),
            std::time::SystemTime::now(),
            frames,
            client
                .as_ref()
                .is_some_and(|client| client.is_socket_connected()),
        );
        let Some(client) = client else {
            return;
        };
        match verdict {
            link_watch::Verdict::Healthy => return,
            link_watch::Verdict::Slept(asleep) => {
                log::info!(
                    "link: resumed after {} s asleep, reconnecting",
                    asleep.as_secs()
                );
            }
            link_watch::Verdict::Silent(quiet) => {
                log::warn!(
                    "link: nothing received for {} s, reconnecting",
                    quiet.as_secs()
                );
            }
            link_watch::Verdict::SocketLost => {
                log::warn!("link: socket disconnected while reported connected, reconnecting");
            }
        }
        self.set_status(LinkStatus::Connecting);
        tokio::spawn(async move { client.reconnect_immediately().await });
    }

    fn set_syncing(&mut self, syncing: bool) {
        if self.syncing != syncing {
            self.syncing = syncing;
            self.emit(Event::Syncing(syncing));
        }
    }

    fn unlinked(&self) -> LinkStatus {
        LinkStatus::Unlinked {
            qr: self.qr.clone(),
            pair_code: self.pair_code.clone(),
            pairing_phone: self.pairing_phone.clone(),
        }
    }

    /// Canonical id used for our account.
    fn me(&self) -> String {
        self.me_pn
            .clone()
            .or_else(|| self.me_lid.clone())
            .unwrap_or_else(|| "me".to_owned())
    }

    /// Our identity for the interface, with the privacy id that mentions
    /// of us may carry instead of the phone number.
    fn me_event(&self) -> Event {
        Event::Me {
            id: self.me(),
            lid: self.me_lid.clone(),
            name: self.me_name.clone(),
            about: self.me_about.clone(),
        }
    }

    fn is_me(&self, id: &str) -> bool {
        self.me_pn.as_deref() == Some(id) || self.me_lid.as_deref() == Some(id)
    }

    fn load_state(&mut self) {
        self.me_pn = self.archive.meta("me_pn").ok().flatten();
        self.me_lid = self.archive.meta("me_lid").ok().flatten();
        self.me_name = self.archive.meta("me_name").ok().flatten();
        // An empty value records that the account has no About text.
        self.me_about = self
            .archive
            .meta("me_about")
            .ok()
            .flatten()
            .filter(|about| !about.is_empty());
        if let Ok(lids) = self.archive.lids() {
            self.lid_to_pn = lids.into_iter().collect();
        }
        if let Ok(contacts) = self.archive.contacts() {
            self.contacts = contacts
                .into_iter()
                .map(|contact| (contact.id.clone(), contact))
                .collect();
        }
        self.retry_confirmed_message_removals();
        if self.me_pn.is_some() || self.me_lid.is_some() {
            self.emit(self.me_event());
        }
        self.emit(Event::Contacts(self.contacts.values().cloned().collect()));
        self.emit_chats();
    }

    /// Measures the media cache off the runtime, emptying it first when
    /// `clear`, and hands the result back as `Command::StorageMeasured`.
    fn measure_storage(&mut self, token: u64, clear: bool) {
        let dir = self.dirs.media_cache_dir();
        let commands = self.commands.clone();
        tokio::task::spawn_blocking(move || {
            if clear && clear_media(&dir).is_err() {
                log::warn!("could not delete every cached attachment");
            }
            let stats = match measure_media(&dir) {
                Ok(stats) => Some(stats),
                Err(_) => {
                    log::warn!("could not measure the attachment cache");
                    None
                }
            };
            let _ = commands.send(Command::StorageMeasured {
                stats,
                token,
                cleared: clear,
            });
        });
    }

    /// Re-derives archived rows from raw protobufs after parser changes. Also
    /// repairs moved attachment paths or clears missing files for redownload.
    fn relocate_media(&mut self) {
        let dir = self.dirs.media_cache_dir();
        let rows = match self.archive.media_paths().and_then(|rows| {
            let mut all: Vec<_> = rows
                .into_iter()
                .map(|(chat, id, path)| (chat, id, None, path))
                .collect();
            all.extend(
                self.archive
                    .carousel_media_paths()?
                    .into_iter()
                    .map(|(chat, id, card, path)| (chat, id, Some(card), path)),
            );
            Ok(all)
        }) {
            Ok(rows) => rows,
            Err(error) => {
                log::warn!("could not list attachments: {error}");
                return;
            }
        };
        // A clip lives only in the cache: one that is gone downloads again.
        for (chat, id, path) in self.archive.motion_paths().unwrap_or_default() {
            if !path.exists() {
                let _ = self.archive.put_motion_path(&chat, &id, None);
            }
        }
        let (mut moved, mut forgotten) = (0, 0);
        for (chat, id, card, path) in rows {
            if path.exists() {
                continue;
            }
            let candidate = path.file_name().map(|name| dir.join(name));
            match candidate.filter(|candidate| candidate.exists()) {
                Some(candidate) => {
                    if self
                        .archive
                        .put_media_path_at(&chat, &id, card, Some(&candidate))
                        .is_ok()
                    {
                        moved += 1;
                    }
                }
                None => {
                    if self
                        .archive
                        .put_media_path_at(&chat, &id, card, None)
                        .is_ok()
                    {
                        forgotten += 1;
                    }
                }
            }
        }
        if moved + forgotten > 0 {
            log::info!(
                "attachments: {moved} re-pointed to {}, {forgotten} to fetch again",
                dir.display()
            );
        }
    }

    fn backfill(&mut self) {
        const VERSION: &str = "4";
        if self.archive.meta("derived").ok().flatten().as_deref() == Some(VERSION) {
            return;
        }
        let rows = match self.archive.rows_with_raw() {
            Ok(rows) => rows,
            Err(error) => {
                log::warn!("could not read the archive for re-deriving: {error}");
                return;
            }
        };
        let started = Instant::now();
        let mut updated = 0;
        for (chat, id, raw) in rows {
            let Ok(message) = wa::Message::decode_from_slice(&raw) else {
                continue;
            };
            let base = message.get_base_message();
            let Some(mut content) = classify(&message) else {
                continue;
            };
            let Ok(Some(existing)) = self.archive.message(&chat, &id) else {
                continue;
            };
            if matches!(existing.content, Content::Revoked) {
                continue;
            }
            // Live edits do not replace the original raw protobuf. Its body and
            // mention context are stale, regardless of the content kind.
            if existing.edited {
                continue;
            }
            content.keep_local_paths(&existing.content);
            let mentions = self.mentions_of(&recorded_mention_ids(base));
            let thumbnail = thumbnail_of(base);
            if self
                .archive
                .set_derived(
                    &chat,
                    &id,
                    &content,
                    &mentions,
                    thumbnail.as_deref(),
                    forwarded_of(base),
                )
                .is_ok()
            {
                updated += 1;
            }
        }
        let _ = self.archive.set_meta("derived", VERSION);
        if updated > 0 {
            log::info!(
                "re-derived {updated} archived messages in {:.1?}",
                started.elapsed()
            );
            self.emit_chats();
        }
    }

    /// Turns archived view-once media, filed as attachments that could never
    /// download, into the view-once placeholder. Only rows without a local
    /// file change, and only their content: an edit flag stays as it was.
    fn backfill_view_once(&mut self) {
        const KEY: &str = "view_once_media";
        if self.archive.meta(KEY).ok().flatten().as_deref() == Some("1") {
            return;
        }
        let rows = match self.archive.media_with_raw() {
            Ok(rows) => rows,
            Err(error) => {
                log::warn!("could not read archived media: {error}");
                return;
            }
        };
        let mut updated = 0;
        for (chat, id, raw) in rows {
            let Ok(message) = wa::Message::decode_from_slice(&raw) else {
                continue;
            };
            let Some(content @ Content::PhoneOnly { .. }) = classify(&message) else {
                continue;
            };
            let Ok(Some(existing)) = self.archive.message(&chat, &id) else {
                continue;
            };
            if existing
                .content
                .media()
                .is_none_or(|media| media.path.is_some())
            {
                continue;
            }
            if self
                .archive
                .set_content(&chat, &id, &content, existing.edited)
                .is_ok()
            {
                updated += 1;
            }
        }
        let _ = self.archive.set_meta(KEY, "1");
        if updated > 0 {
            log::info!("marked {updated} archived messages as view once");
            self.emit_chats();
        }
    }

    /// Adds album parent ids to archived pictures and videos without
    /// re-deriving edited text or any other locally preserved state.
    fn backfill_media_albums(&mut self) {
        const KEY: &str = "media_albums";
        if self.archive.meta(KEY).ok().flatten().as_deref() == Some("1") {
            return;
        }
        let rows = match self.archive.rows_with_raw() {
            Ok(rows) => rows,
            Err(error) => {
                log::warn!("could not read archived media albums: {error}");
                return;
            }
        };
        let mut updated = 0;
        let mut failed = false;
        for (chat, id, raw) in rows {
            let Ok(message) = wa::Message::decode_from_slice(&raw) else {
                continue;
            };
            let album =
                album_media(message.get_base_message(), media(None, None, None, None)).album;
            let Some(album) = album else { continue };
            let existing = match self.archive.message(&chat, &id) {
                Ok(Some(existing)) => existing,
                Ok(None) => continue,
                Err(_) => {
                    failed = true;
                    continue;
                }
            };
            let mut content = existing.content;
            let media = match &mut content {
                Content::Image { media, .. }
                | Content::Video {
                    media, note: false, ..
                } => media,
                _ => continue,
            };
            if media.album.as_deref() == Some(&album) {
                continue;
            }
            media.album = Some(album);
            if self
                .archive
                .set_derived(
                    &chat,
                    &id,
                    &content,
                    &existing.mentions,
                    existing.thumbnail.as_deref(),
                    existing.forwarded,
                )
                .is_ok()
            {
                updated += 1;
            } else {
                failed = true;
            }
        }
        if !failed {
            let _ = self.archive.set_meta(KEY, "1");
        } else {
            log::warn!("could not update some archived media-album items; will retry on startup");
        }
        if updated > 0 {
            log::info!("grouped {updated} archived media-album items");
            self.emit_chats();
        }
    }

    /// Marks archived round video messages, filed before videos told them
    /// apart, so they draw as circles. Only that flag changes: deriving the
    /// whole row again would drop edits and local paths.
    fn backfill_video_notes(&mut self) {
        const KEY: &str = "video_notes";
        if self.archive.meta(KEY).ok().flatten().as_deref() == Some("1") {
            return;
        }
        let rows = match self.archive.videos_with_raw() {
            Ok(rows) => rows,
            Err(error) => {
                log::warn!("could not read archived videos: {error}");
                return;
            }
        };
        let mut updated = 0;
        for (chat, id, raw) in rows {
            let Ok(message) = wa::Message::decode_from_slice(&raw) else {
                continue;
            };
            let base = message.get_base_message();
            if base.video_message.as_option().is_some() || base.ptv_message.as_option().is_none() {
                continue;
            }
            let Ok(Some(existing)) = self.archive.message(&chat, &id) else {
                continue;
            };
            let mut content = existing.content;
            let Content::Video { note, .. } = &mut content else {
                continue;
            };
            if *note {
                continue;
            }
            *note = true;
            if self
                .archive
                .set_content(&chat, &id, &content, existing.edited)
                .is_ok()
            {
                updated += 1;
            }
        }
        let _ = self.archive.set_meta(KEY, "1");
        if updated > 0 {
            log::info!("marked {updated} archived video messages as round");
            self.emit_chats();
        }
    }

    /// Moves the clips of motion photos, filed as unsupported messages before
    /// they were recognised, to their photos.
    fn backfill_motion_photos(&mut self) {
        const KEY: &str = "motion_photo_children";
        if self.archive.meta(KEY).ok().flatten().as_deref() == Some("1") {
            return;
        }
        let rows = match self.archive.unsupported_with_raw() {
            Ok(rows) => rows,
            Err(error) => {
                log::warn!("could not read archived unsupported messages: {error}");
                return;
            }
        };
        let mut removed = 0;
        let mut failed = false;
        for (chat, id, raw) in rows {
            let Ok(message) = wa::Message::decode_from_slice(&raw) else {
                continue;
            };
            let base = message.get_base_message();
            if !motion_photo_child(base) {
                continue;
            }
            if let Some((parent, clip)) = motion_clip(base) {
                let filed = self.archive.message(&chat, &id).and_then(|row| match row {
                    Some(row) => self
                        .archive
                        .put_motion_clip(&chat, &parent, &row.sender, &clip)
                        .map(drop),
                    None => Ok(()),
                });
                if let Err(error) = filed {
                    log::warn!("could not move an archived motion clip: {error}");
                    failed = true;
                    continue;
                }
            }
            match self.archive.delete_message(&chat, &id) {
                Ok(_) => removed += 1,
                Err(error) => {
                    log::warn!("could not remove an archived motion clip: {error}");
                    failed = true;
                }
            }
        }
        if !failed {
            let _ = self.archive.set_meta(KEY, "1");
        }
        if removed > 0 {
            log::info!("removed {removed} archived motion photo clips");
            self.emit_chats();
        }
    }

    /// Removes the HD copies of photos and videos, filed as unsupported
    /// messages before they were recognised. Their standard copies stay.
    fn backfill_hd_copies(&mut self) {
        self.remove_archived_unsupported("hd_copies", hd_copy, "HD copies");
    }

    /// Removes the notices that someone mentioned a group in their status,
    /// filed as unsupported messages before they were recognised (#595).
    fn backfill_group_status_mentions(&mut self) {
        self.remove_archived_unsupported(
            "group_status_mentions",
            group_status_mention,
            "group status mentions",
        );
    }

    /// Removes, once (`key` records it), the archived unsupported messages
    /// `hidden` recognises as nothing to show.
    fn remove_archived_unsupported(
        &mut self,
        key: &str,
        hidden: fn(&wa::Message) -> bool,
        what: &str,
    ) {
        if self.archive.meta(key).ok().flatten().as_deref() == Some("1") {
            return;
        }
        let rows = match self.archive.unsupported_with_raw() {
            Ok(rows) => rows,
            Err(error) => {
                log::warn!("could not read archived unsupported messages: {error}");
                return;
            }
        };
        let mut removed = 0;
        let mut failed = false;
        for (chat, id, raw) in rows {
            let Ok(message) = wa::Message::decode_from_slice(&raw) else {
                continue;
            };
            if !hidden(message.get_base_message()) {
                continue;
            }
            match self.archive.delete_message(&chat, &id) {
                Ok(_) => removed += 1,
                Err(error) => {
                    log::warn!("could not remove an archived message ({what}): {error}");
                    failed = true;
                }
            }
        }
        if !failed {
            let _ = self.archive.set_meta(key, "1");
        }
        if removed > 0 {
            log::info!("removed {removed} archived {what}");
            self.emit_chats();
        }
    }

    async fn start_bot(&mut self) {
        let path = self.dirs.session_db();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let store = match device_store::open(&path).await {
            Ok(store) => store,
            Err(error) => {
                self.set_status(LinkStatus::Failed(format!(
                    "Could not open the device store: {error}"
                )));
                return;
            }
        };
        let sender = self.wa_sender.clone();
        let builder = host_owned_presence(Bot::builder().with_backend(store))
            .with_watched_ab_props([
                abprops::web::AURA_PINNED_CHATS_BENEFIT_ACTIVE,
                abprops::web::ENABLE_WEB_CALLING,
                abprops::web::ENABLE_WEB_GROUP_CALLING,
            ]);
        let builder = match crate::proxy::for_whatsapp() {
            Some(proxy) => {
                log::info!("connecting through the proxy {}", proxy.redacted());
                builder
                    .with_transport_factory(crate::proxy::ProxyTransportFactory::new(proxy))
                    .with_http_client(whatsapp_rust::http::UreqHttpClient::with_agent(
                        crate::proxy::agent(),
                    ))
            }
            // Every address the host resolves to is dialed, not only the first
            // one, so a network whose IPv6 does not answer still links over
            // IPv4 (#212).
            None => builder
                .with_transport_factory(crate::transport::HappyEyeballsTransportFactory::new()),
        };
        let bot = builder
            // WhatsApp reads the linked-device name, version, and icon at pairing.
            .with_device_props(
                DevicePropsOverride::new()
                    .with_os(device_name())
                    .with_version(app_version())
                    .with_platform_type(wa::device_props::PlatformType::DESKTOP),
            )
            .with_event_handler(UiEvents {
                sender,
                generation: self.privacy_generation,
            })
            .build()
            .await;
        match bot {
            Ok(bot) => {
                let handle = bot.spawn();
                self.client = Some(handle.client());
                self.handle = Some(handle);
                self.set_status(LinkStatus::Connecting);
            }
            Err(error) => self.set_status(LinkStatus::Failed(format!(
                "Could not start WhatsApp: {error}"
            ))),
        }
    }

    /// New downloads use account/chat folders inside the chosen destination.
    /// An unavailable destination falls back to the cache, so an unplugged
    /// drive does not stop downloads.
    fn download_dir(&self, chat: &str) -> PathBuf {
        if let Some(root) = &self.download_folder {
            let chat = self.canonical_str(chat);
            let name = self
                .contact_name(&chat)
                .or_else(|| {
                    self.archive
                        .chat(&chat)
                        .ok()
                        .flatten()
                        .map(|chat| chat.name)
                })
                .unwrap_or_else(|| self.chat_name(&chat, None));
            let account =
                download_folders::account_folder(root, &self.dirs.id, self.me_pn.as_deref());
            let folder = download_folders::chat_folder(&self.archive, &account, &chat, &name);
            if download_folders::create_folder(root, &folder).is_ok() {
                return folder;
            }
            log::warn!("the download folder is unavailable; using the cache");
        }
        self.dirs.media_cache_dir()
    }

    fn set_online(&mut self, online: bool) {
        if self.online_wanted != online {
            self.online_wanted = online;
            self.online_changed = Instant::now();
        }
        // Coming back is announced at once; leaving waits for the tick, so a
        // quick switch to another window does not flap.
        if online || self.unavailable_due() {
            self.announce_presence(online);
        }
    }

    /// Announces the current state, once the offline linger has elapsed.
    fn settle_presence(&mut self) {
        if self.online_wanted || self.unavailable_due() {
            self.announce_presence(self.online_wanted);
        }
    }

    fn unavailable_due(&self) -> bool {
        !self.online_wanted
            && self.online_sent != Some(false)
            && self.online_changed.elapsed() >= PRESENCE_LINGER
    }

    /// WhatsApp holds back push notifications on the phone while a linked
    /// device is available, as WhatsApp Web does while its tab has focus.
    fn announce_presence(&mut self, online: bool) {
        if self.online_sent == Some(online)
            || self.presence_in_flight
            || !matches!(self.status, LinkStatus::Connected)
        {
            return;
        }
        let Some(client) = self.client.clone() else {
            return;
        };
        self.presence_in_flight = true;
        let generation = self.presence_generation;
        let sender = self.wa_sender.clone();
        tokio::spawn(async move {
            let presence = client.presence();
            let result = if online {
                presence.set_available().await
            } else {
                presence.set_unavailable().await
            };
            if let Err(error) = &result {
                log::debug!("presence not announced: {error}");
            }
            let _ = sender.send(RuntimeEvent::PresenceSent {
                generation,
                online,
                succeeded: result.is_ok(),
            });
        });
    }

    fn presence_sent(&mut self, generation: u64, online: bool, succeeded: bool) {
        if generation != self.presence_generation {
            return;
        }
        self.presence_in_flight = false;
        if self.presence_refresh_pending {
            self.presence_refresh_pending = false;
            self.online_sent = None;
            self.announce_presence(self.online_wanted);
        } else if succeeded {
            self.online_sent = Some(online);
            self.settle_presence();
        }
    }

    fn push_name_synced(&mut self) {
        self.online_sent = None;
        if self.presence_in_flight {
            self.presence_refresh_pending = true;
        } else {
            self.announce_presence(self.online_wanted);
        }
    }

    fn refresh_legacy_preferences(&mut self) {
        if self.privacy_confirmed
            || self.privacy_recovering
            || Instant::now() < self.privacy_retry
            || !matches!(self.status, LinkStatus::Connected)
        {
            return;
        }
        let Some(client) = self.client.clone() else {
            return;
        };
        if !self.privacy_ready && self.privacy_reveal_at.is_none() {
            self.privacy_reveal_at = Some(Instant::now() + PRIVACY_GRACE);
        }
        self.privacy_recovering = true;
        let sender = self.wa_sender.clone();
        let generation = self.privacy_generation;
        // Chat locks live in RegularLow. A snapshot discards and rebuilds the
        // collection, which only an archive that predates lock mirroring needs;
        // an incremental sync waits for the library's own first sync instead of
        // racing it.
        let mode = if self.privacy_snapshot {
            whatsapp_rust::AppStateResyncMode::Snapshot
        } else {
            whatsapp_rust::AppStateResyncMode::Incremental
        };
        if !self.privacy_ready {
            self.emit(Event::Syncing(true));
        }
        // An upgraded archive also recovers mute settings and pin order from
        // RegularHigh once, but only the lock collection holds content back.
        let mut collections = vec![whatsapp_rust::WAPatchName::RegularLow];
        if self.privacy_snapshot {
            collections.push(whatsapp_rust::WAPatchName::RegularHigh);
        }
        tokio::spawn(async move {
            use whatsapp_rust::WAPatchName;
            let (locks, complete) = match client.resync_app_state(collections, mode).await {
                Ok(report) => {
                    if !report.all_synced() {
                        log::warn!(
                            "chat settings recovery incomplete ({mode:?}): fatal {:?}, retryable {:?}, skipped {:?}",
                            report.fatal,
                            report.retryable,
                            report.skipped
                        );
                    }
                    (
                        report.synced.contains(&WAPatchName::RegularLow),
                        report.all_synced(),
                    )
                }
                Err(error) => {
                    log::warn!("chat settings recovery failed ({mode:?}): {error}");
                    (false, false)
                }
            };
            // Use the same queue as the replayed mutations, so lock updates
            // are applied before the completion marker can expose chat rows.
            let _ = sender.send(RuntimeEvent::PreferencesRecovered {
                generation,
                locks,
                complete,
            });
        });
    }

    /// `locks` says the lock collection synced; `complete` that everything
    /// requested did, so the one-time recovery need not run again.
    fn preferences_recovered(&mut self, generation: u64, locks: bool, complete: bool) {
        // A completed task from an unlinked device cannot authorize showing
        // chats belonging to the next linked account.
        if generation != self.privacy_generation {
            return;
        }
        self.privacy_recovering = false;
        if locks
            && (!complete
                || self
                    .archive
                    .set_meta("chat_privacy_ready_v1", "complete")
                    .is_ok())
        {
            // Without `complete` the marker stays unset, so the next start
            // retries the settings this run could not recover.
            self.privacy_confirmed = true;
            self.privacy_snapshot = false;
            self.privacy_reveal_at = None;
            self.reveal_private_content();
        } else {
            self.privacy_attempts = self.privacy_attempts.saturating_add(1);
            self.privacy_retry = Instant::now() + privacy_backoff(self.privacy_attempts);
            log::warn!(
                "chat lock state recovery failed (attempt {}); retrying later",
                self.privacy_attempts
            );
            // Waiting longer does not help a failed sync: show what is known.
            self.reveal_unconfirmed();
        }
    }

    fn reveal_unconfirmed_after_grace(&mut self) {
        if self
            .privacy_reveal_at
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            self.reveal_unconfirmed();
        }
    }

    /// Shows chats whose lock state could not be confirmed yet. Chats already
    /// known to be locked stay hidden, and later lock updates still apply.
    fn reveal_unconfirmed(&mut self) {
        self.privacy_reveal_at = None;
        if self.privacy_ready {
            return;
        }
        self.reveal_private_content();
        if !self.privacy_warned {
            self.privacy_warned = true;
            self.emit(Event::Info(
                "Couldn't confirm which chats are locked on your phone yet. Chats locked there may appear until they sync."
                    .to_owned(),
            ));
        }
    }

    fn reveal_private_content(&mut self) {
        if self.privacy_ready {
            return;
        }
        self.privacy_ready = true;
        self.load_state();
        self.emit(Event::Syncing(self.syncing));
        // A call that was withheld while the locks were unknown is re-sent now that they are: the
        // chat it belongs to can finally be judged private or not, so it rings against the right
        // lock state instead of being dropped for the whole recovery.
        self.replay_call();
        // The picker may have been sent an empty Received shelf meanwhile.
        self.emit_stickers();
        self.emit_starred();
        // Answer the reads made while content was withheld, now that the
        // chat list they belong to has been sent.
        for page in std::mem::take(&mut self.withheld_pages) {
            match page {
                WithheldPage::Page(chat, before) => self.send_page(&chat, before),
                WithheldPage::Until(chat, id, before) => self.load_until(chat, id, before),
                WithheldPage::Media(chat, request) => self.load_chat_media(chat, request),
            }
        }
    }

    async fn stop_bot(&mut self) {
        self.client = None;
        // A batch still going belongs to the session that was sending it, and
        // every send is its own task: one can report its tick after this
        // returns, up to the shutdown timeout. With the queue dropped the ack
        // finds nothing to advance, instead of resuming the batch through the
        // session that comes next. Message ids are fresh per send, so an ack
        // can never match a job queued after the stop.
        self.abandon_forwards();
        // Acks belong to the session that sent; the next one never answers.
        self.awaiting_ack.clear();
        if let Some(handle) = self.handle.take()
            && tokio::time::timeout(Duration::from_secs(5), handle.shutdown())
                .await
                .is_err()
        {
            log::warn!("the WhatsApp connection did not stop in time");
        }
    }

    // --- ids -------------------------------------------------------------

    fn learn_lid(&mut self, lid: &str, pn: &str) {
        if lid.is_empty() || pn.is_empty() {
            return;
        }
        if self.lid_to_pn.get(lid).is_some_and(|known| known == pn) {
            return;
        }
        let changed = match self.archive.put_lid(lid, pn) {
            Ok(changed) => changed,
            Err(error) => {
                log::warn!("could not remember an id mapping: {error}");
                return;
            }
        };
        self.lid_to_pn.insert(lid.to_owned(), pn.to_owned());
        if changed {
            self.emit_chats();
        }
        let chat = format!("{pn}@s.whatsapp.net");
        let lid_chat = format!("{lid}@lid");
        match self.archive.removed_message_ids(&chat) {
            Ok(ids) => {
                for id in ids {
                    for alias in [&chat, &lid_chat] {
                        self.emit(Event::MessageDeleted {
                            chat: alias.clone(),
                            id: id.clone(),
                        });
                    }
                }
            }
            Err(_) => self.emit(Event::Error("Could not refresh deleted messages".into())),
        }
        // Receipts filed under the privacy id may name messages archived
        // under the phone number.
        for id in self.archive.waiting_receipts(&chat).unwrap_or_default() {
            self.settle_early_receipts(&chat, &id);
        }
        let mapped = self.canonical_str(&lid_chat);
        for id in self.early.rekey(&lid_chat, &mapped) {
            if matches!(self.archive.message(&mapped, &id), Ok(Some(_))) {
                self.settle_early_events(&mapped, &id);
                self.emit_message(&mapped, &id);
                self.emit_chat(&mapped);
            }
        }
        if self.receipts_watch.is_some() {
            self.emit_receipts();
        }
    }

    fn learn_pair(&mut self, a: &Jid, b: &Jid) {
        if a.is_lid() && b.is_pn() {
            self.learn_lid(a.user_base(), b.user_base());
        } else if a.is_pn() && b.is_lid() {
            self.learn_lid(b.user_base(), a.user_base());
        }
    }

    fn learn_source(&mut self, source: &MessageSource) {
        if let Some(alt) = &source.sender_alt {
            let sender = source.sender.clone();
            self.learn_pair(&sender, alt);
        }
        if let Some(alt) = &source.recipient_alt {
            let chat = source
                .recipient
                .clone()
                .unwrap_or_else(|| source.chat.clone());
            self.learn_pair(&chat, alt);
        }
    }

    /// Returns the archive id for a JID, resolving known privacy ids.
    fn canonical(&self, jid: &Jid) -> String {
        if jid.is_lid()
            && let Some(pn) = self.lid_to_pn.get(jid.user_base())
        {
            let pn = format!("{pn}@s.whatsapp.net");
            return if self.is_me(&pn) { self.me() } else { pn };
        }
        let id = jid.to_non_ad_string();
        if self.is_me(&id) {
            return self.me();
        }
        id
    }

    fn canonical_str(&self, id: &str) -> String {
        match id.parse::<Jid>() {
            Ok(jid) => self.canonical(&jid),
            Err(_) => id.to_owned(),
        }
    }

    /// App-state mutations may use a privacy id before a message teaches the UI
    /// its mapping. Consult the protocol library's persisted mapping as well.
    async fn canonical_sync_chat(&mut self, jid: &Jid) -> String {
        if jid.is_lid()
            && !self.lid_to_pn.contains_key(jid.user_base())
            && let Some(client) = self.client.clone()
        {
            match client.get_lid_pn_entry(jid).await {
                Ok(Some(entry)) => self.learn_lid(&entry.lid, &entry.phone_number),
                Ok(None) => log::info!("chat removal: privacy mapping not yet available"),
                Err(_) => log::warn!("chat removal: could not resolve privacy mapping"),
            }
        }
        self.canonical(jid)
    }

    fn jid_of(id: &str) -> Option<Jid> {
        id.parse().ok()
    }

    fn set_account_privacy(&self, kind: PrivacyKind, choice: PrivacyChoice) {
        let Some((category, value)) = privacy::wire_set(kind, choice) else {
            let _ = self.commands.send(Command::AccountPrivacyFailed { kind });
            return;
        };
        let Some(client) = self.client.clone() else {
            self.emit(Event::Error("Not connected to WhatsApp".into()));
            let _ = self.commands.send(Command::AccountPrivacyFailed { kind });
            return;
        };
        let commands = self.commands.clone();
        tokio::spawn(async move {
            match client.set_privacy_setting(category, value).await {
                Ok(_) => {
                    let _ = commands.send(Command::AccountPrivacySaved { kind });
                }
                Err(error) => {
                    log::debug!("privacy setting not written: {error}");
                    let _ = commands.send(Command::AccountPrivacyFailed { kind });
                }
            }
        });
    }

    // --- names -----------------------------------------------------------

    fn contact_name(&self, id: &str) -> Option<String> {
        self.contacts.get(id).and_then(Contact::label)
    }

    /// Resolves a name for a quote or mention.
    fn name_for(&self, id: &str) -> Option<String> {
        if self.is_me(id) || id == self.me() {
            return Some("You".to_owned());
        }
        if let Some(name) = self.contact_name(id) {
            return Some(name);
        }
        crate::model::phone_of(id).map(crate::util::phone)
    }

    /// Returns the best current chat name.
    fn chat_name(&self, id: &str, push_name: Option<&str>) -> String {
        if id == self.me() {
            return "You".to_owned();
        }
        if let Some(name) = self
            .contacts
            .get(id)
            .and_then(|contact| contact.full_name.clone())
            .filter(|name| !name.is_empty())
        {
            return name;
        }
        if let Some(digits) = crate::model::phone_of(id) {
            return crate::util::phone(digits);
        }
        if let Some(name) = push_name
            .filter(|name| !name.is_empty())
            .or_else(|| self.contacts.get(id)?.push_name.as_deref())
        {
            return format!("~{name}");
        }
        fallback_name(id)
    }

    fn remember_push_name(&mut self, id: &str, push_name: &str) {
        if push_name.is_empty() || id == self.me() {
            return;
        }
        let contact = self
            .contacts
            .entry(id.to_owned())
            .or_insert_with(|| Contact {
                id: id.to_owned(),
                full_name: None,
                first_name: None,
                push_name: None,
            });
        if contact.push_name.as_deref() == Some(push_name) {
            return;
        }
        contact.push_name = Some(push_name.to_owned());
        let contact = contact.clone();
        if let Err(error) = self.archive.upsert_contact(&contact) {
            log::warn!("could not save a contact: {error}");
        }
        self.emit(Event::Contacts(vec![contact]));
        self.refresh_chat_name(id);
    }

    /// Mutes synced for channels we no longer follow used to file them as
    /// empty conversations, which the list could only name "Unknown". Once the
    /// server has listed the followed ones, drop the others that never held a
    /// message, as the phone does not show them either. No removal barrier
    /// is kept, so following one again later brings its history back.
    fn drop_unfollowed_empty_channels(&mut self, followed: &[(String, String)]) {
        let Ok(chats) = self.archive.chats() else {
            return;
        };
        for chat in chats {
            if !chat.is_channel()
                || chat.last.is_some()
                || chat.last_activity != 0
                || followed.iter().any(|(id, _)| *id == chat.id)
            {
                continue;
            }
            if self
                .archive
                .delete_chat(&chat.id)
                .is_ok_and(|removed| removed.existed)
            {
                self.emit(Event::ChatRemoved { chat: chat.id });
            }
        }
    }

    /// Replaces a fallback chat name when a better one is known.
    fn refresh_chat_name(&mut self, id: &str) {
        let Ok(Some(chat)) = self.archive.chat(id) else {
            return;
        };
        if keeps_its_name(&chat) {
            return;
        }
        let name = self.chat_name(id, None);
        if name != chat.name {
            let _ = self.archive.rename_chat(id, &name);
            self.emit_chat(id);
        }
    }

    fn ensure_chat(&mut self, id: &str, push_name: Option<&str>) {
        match self.archive.chat(id) {
            Ok(Some(chat)) => {
                if !keeps_its_name(&chat) {
                    let name = self.chat_name(id, push_name);
                    if name != chat.name {
                        let _ = self.archive.rename_chat(id, &name);
                    }
                }
            }
            Ok(None) => {
                let name = self.chat_name(id, push_name);
                if self.archive.ensure_chat(id, &name).is_err() {
                    log::warn!("could not create a chat");
                }
            }
            Err(_error) => log::warn!("could not read a chat"),
        }
        if ChatKind::from_id(id) == ChatKind::Group {
            self.request_group_info(id, false);
        }
    }

    /// Queues a group metadata request, at the front when `force` is true.
    fn request_group_info(&mut self, id: &str, force: bool) {
        if force {
            // Invalidate in-flight replies as soon as a replacement is queued.
            *self.group_info_generation.entry(id.to_owned()).or_default() += 1;
            self.group_info_requested.remove(id);
            self.group_info_retry.retain(|(_, chat)| chat != id);
            self.group_info_queue.retain(|chat| chat != id);
            self.group_info_tries.remove(id);
        } else {
            if self.group_info_retry.iter().any(|(_, chat)| chat == id) {
                return;
            }
            let known = self.archive.chat(id).ok().flatten().is_some_and(|chat| {
                // Older archives used "Group" as an unknown placeholder.
                // Fetch it once to distinguish that from a real subject.
                !chat.name.trim().is_empty()
                    && (chat.group_subject_known || chat.name != "Group")
                    && !chat.participants.is_empty()
                    // Archives from before group editing do not know who may
                    // edit a group's info; a group we left has nothing to ask.
                    && (chat.info_locked.is_some() || chat.left)
                    && (chat.group_description.is_some() || chat.left)
                    // Nor its permissions and admins, before group management.
                    && (chat.group_settings.is_some() || chat.left)
            });
            if known {
                return;
            }
        }
        if !self.group_info_requested.insert(id.to_owned()) {
            return;
        }
        if force {
            self.group_info_queue.push_front(id.to_owned());
        } else {
            self.group_info_queue.push_back(id.to_owned());
        }
    }

    /// Group metadata retry delay.
    fn group_retry_delay(tries: u32) -> Duration {
        Duration::from_secs(30 * 2u64.pow(tries.saturating_sub(1).min(5)))
            .min(Duration::from_secs(600))
    }

    /// Sends a limited number of group metadata requests per tick.
    fn pump_group_info(&mut self) {
        let now = Instant::now();
        let due: Vec<String> = {
            let (due, later): (Vec<_>, Vec<_>) = std::mem::take(&mut self.group_info_retry)
                .into_iter()
                .partition(|(at, _)| *at <= now);
            self.group_info_retry = later;
            due.into_iter().map(|(_, id)| id).collect()
        };
        for id in due {
            if self.group_info_requested.insert(id.clone()) {
                self.group_info_queue.push_back(id);
            }
        }
        for _ in 0..2 {
            let Some(id) = self.group_info_queue.pop_front() else {
                return;
            };
            // A late failure can requeue a group deleted in the meantime.
            if self.archive.removal_point(&id).ok().flatten().is_some()
                && self.archive.chat(&id).ok().flatten().is_none()
            {
                self.group_info_requested.remove(&id);
                continue;
            }
            self.query_group_info(&id);
        }
    }

    /// Schedules metadata retry with backoff, or stops on permanent failure.
    fn handle_failed_group(&mut self, chat: String, permanent: bool) {
        self.group_info_retry.retain(|(_, id)| id != &chat);
        self.group_info_requested.remove(&chat);
        if permanent {
            self.group_info_tries.remove(&chat);
            self.group_info_requested.insert(chat);
        } else {
            let tries = self.group_info_tries.entry(chat.clone()).or_insert(0);
            *tries += 1;
            if *tries <= 7 {
                self.group_info_retry
                    .push((Instant::now() + Self::group_retry_delay(*tries), chat));
            } else {
                self.group_info_requested.insert(chat);
            }
        }
    }

    /// Requests metadata for one group.
    fn query_group_info(&mut self, id: &str) {
        let (Some(client), Some(jid)) = (self.client.clone(), Self::jid_of(id)) else {
            // Requeue until the link is available.
            self.group_info_requested.remove(id);
            let tries = self.group_info_tries.entry(id.to_owned()).or_insert(0);
            *tries += 1;
            self.group_info_retry.push((
                Instant::now() + Self::group_retry_delay(*tries),
                id.to_owned(),
            ));
            return;
        };
        let commands = self.commands.clone();
        let chat = id.to_owned();
        // Retries and forced refreshes each own their completion.
        let request_generation = self.group_info_generation.entry(chat.clone()).or_default();
        *request_generation += 1;
        let request_generation = *request_generation;
        // The answer can land after a leave confirmed while it was in flight.
        let leave_generation = self.leave_generation.get(id).copied().unwrap_or(0);
        // Likewise after a rename made here, which the answer may predate.
        let subject_generation = self.subject_generation.get(id).copied().unwrap_or(0);
        let description_generation = self.description_generation.get(id).copied().unwrap_or(0);
        let settings_generation = self.settings_generation.get(id).copied().unwrap_or(0);
        let me: Vec<String> = [self.me_pn.clone(), self.me_lid.clone()]
            .into_iter()
            .flatten()
            .collect();
        let lids = self.lid_to_pn.clone();
        tokio::spawn(async move {
            match client.groups().fetch_metadata(&jid).await {
                Ok(metadata) => {
                    let (participants, admin) =
                        new_group::group_members(&metadata.participants, &me, &lids);
                    let settings = group_admin::settings_of(&metadata, &lids);
                    let _ = commands.send(Command::GroupInfo {
                        leave_generation,
                        chat,
                        // Empty subjects leave cached titles intact and retry.
                        name: Some(metadata.subject.clone().unwrap_or_default()),
                        description: metadata
                            .description
                            .filter(|text| !text.trim().is_empty())
                            .unwrap_or_default(),
                        description_generation,
                        request_generation,
                        participants,
                        read_only: metadata.is_announcement && !admin,
                        // GroupEphemeralSettings carries a trigger mode, not a
                        // timestamp; a zero setting timestamp keeps later
                        // authoritative updates (protocol messages) able to
                        // override the value fetched here.
                        ephemeral_expiration: metadata
                            .ephemeral
                            .as_ref()
                            .and_then(|value| value.expiration),
                        ephemeral_setting_timestamp: None,
                        info_locked: metadata.is_locked,
                        admin,
                        subject_generation,
                        settings,
                        settings_generation,
                    });
                }
                Err(error) => {
                    let text = error.to_string();
                    // Missing, forbidden, and unauthorized groups do not retry.
                    let permanent = ["item-not-found", "forbidden", "not-authorized"]
                        .iter()
                        .any(|word| text.contains(word));
                    log::warn!("could not fetch group metadata");
                    let _ = commands.send(Command::GroupInfoFailed {
                        chat,
                        permanent,
                        request_generation,
                    });
                }
            }
        });
    }

    // --- WhatsApp events -------------------------------------------------

    /// The callback owns its original generation, even if it runs after logout.
    async fn handle_session_deletion(&mut self, generation: u64, event: Arc<wa_events::Event>) {
        if generation == self.privacy_generation {
            self.handle_wa_event(event).await;
        }
    }

    async fn handle_wa_event(&mut self, event: Arc<wa_events::Event>) {
        use wa_events::Event as E;
        match &*event {
            // Call signaling. An `<offer>` that should ring, and the `accept`/`reject`/`terminate`
            // that decide the call this worker already owns: the peer's answer is the only thing
            // that moves a call out of dialing, never the fact that dialing started.
            E::IncomingCall(call) => self.call_signaling(call).await,
            E::MissedCall(call) => {
                // A call that rang here is filed as it ends, under its own chat (a group call's
                // under its group); the notice is filed only for a call this device never rang.
                if !self.call_resolved(&call.call_id, None) {
                    self.missed_call_entry(call);
                }
            }
            E::CallLogSync(sync) => self.call_log_entry(
                &sync.call_id,
                Some(&sync.call_creator_jid),
                Some(sync.from_me),
                &sync.record,
            ),
            E::CallLogHistory(history) => {
                if let Some(call_id) = history.record.call_id.clone() {
                    self.call_log_entry(
                        &call_id,
                        history.call_creator_jid.as_ref(),
                        history.from_me,
                        &history.record,
                    );
                }
            }
            E::CallEndedElsewhere(call) => {
                self.call_resolved(
                    &call.call_id,
                    Some(match call.outcome {
                        whatsapp_rust::types::call::ElsewhereOutcome::Rejected => {
                            crate::calls::CallOutcome::DeclinedElsewhere
                        }
                        _ => crate::calls::CallOutcome::AnsweredElsewhere,
                    }),
                );
            }
            E::PairingQrCode(qr) => {
                // The prefix lets the phone camera open WhatsApp at Linked devices.
                self.qr = Some(format!("{NATIVE_CAMERA_DEEP_LINK_PREFIX}{}", qr.code));
                let status = self.unlinked();
                self.set_status(status);
            }
            E::PairingCode(code) => {
                self.pair_code = Some(code.code.clone());
                let status = self.unlinked();
                self.set_status(status);
            }
            E::PairingCodeError(error) => {
                self.pair_code = None;
                self.pairing_phone = None;
                self.emit(Event::Error(format!(
                    "Could not link by phone number: {}",
                    error.error
                )));
                let status = self.unlinked();
                self.set_status(status);
            }
            E::PairingQrCodesExhausted(exhausted) => {
                self.qr = None;
                let status = self.unlinked();
                self.set_status(status);
                if exhausted.disconnected
                    && let Some(client) = self.client.clone()
                {
                    tokio::spawn(async move { client.reconnect_immediately().await });
                }
            }
            E::PairSuccess(pair) => {
                self.qr = None;
                self.pair_code = None;
                self.pairing_phone = None;
                self.remember_identity(Some(pair.id.clone()), Some(pair.lid.clone()), None);
                self.set_status(LinkStatus::Connecting);
            }
            E::Connected(_) => {
                let (pn, lid, name) = match &self.client {
                    Some(client) => (client.pn(), client.lid(), Some(client.push_name())),
                    None => (None, None, None),
                };
                self.remember_identity(pn, lid, name);
                self.set_status(LinkStatus::Connected);
                self.retry_message_removals();
                // A new connection may reach a phone that was away before.
                self.older_warned.clear();
                self.refresh_legacy_preferences();
                self.retry_avatars();
                self.pump_read_sync();
                self.pump_favorite_chats();
                self.poll_history.reconnect(Instant::now());
                self.push_favorites();
                self.fetch_missing_favorites();
                self.recover_favorites();
                self.recover_first_names();
                self.recover_stars();
                let _ = self.archive.retry_poll_votes();
                self.pump_poll_votes();
                if let Some(client) = self.client.clone() {
                    let me = self.me_pn.clone().and_then(|pn| Self::jid_of(&pn));
                    let commands = self.commands.clone();
                    self.presence_generation = self.presence_generation.wrapping_add(1);
                    self.online_sent = None;
                    self.presence_in_flight = false;
                    self.presence_refresh_pending = false;
                    self.announce_presence(self.online_wanted);
                    spawn_pin_limit_check(client.clone(), self.events.clone(), self.waker.clone());
                    spawn_calling_gate_log(client.clone());
                    let channels = self.commands.clone();
                    let followed = client.clone();
                    self.channel_list_generation = self.channel_list_generation.wrapping_add(1);
                    let generation = self.channel_list_generation;
                    tokio::spawn(async move {
                        // A channel's Mute lives on the channel, not in the
                        // chat's app state, so read it from the server.
                        match followed.newsletter().list_subscribed().await {
                            Ok(list) => {
                                let pictures = list
                                    .iter()
                                    .map(|channel| {
                                        (
                                            channel.jid.to_string(),
                                            super::ChannelPicture::of(channel),
                                        )
                                    })
                                    .collect();
                                // History sync often carries no channel name,
                                // which leaves only the bare id to show. Every
                                // id goes, named or not: the list is also what
                                // tells a followed channel from a stale one.
                                let names = list
                                    .iter()
                                    .map(|channel| (channel.jid.to_string(), channel.name.clone()))
                                    .collect();
                                let mutes = list
                                    .into_iter()
                                    .filter_map(|channel| {
                                        Some((channel.jid.to_string(), channel.muted?))
                                    })
                                    .collect();
                                let _ = channels.send(Command::ChannelNames { generation, names });
                                let _ = channels.send(Command::ChannelMutes(mutes));
                                let _ = channels.send(Command::ChannelPictures(Some(pictures)));
                            }
                            Err(error) => {
                                log::debug!("followed channels not listed: {error}");
                                let _ = channels.send(Command::ChannelPictures(None));
                            }
                        }
                    });
                    tokio::spawn(async move {
                        publish_account_privacy(&client, &commands).await;
                        if let Some(me) = me {
                            match client
                                .contacts()
                                .get_user_info(std::slice::from_ref(&me))
                                .await
                            {
                                Ok(info) => {
                                    let about = info
                                        .get(&me)
                                        .and_then(|info| info.status.clone())
                                        .filter(|about| !about.is_empty());
                                    let _ = commands.send(Command::MeInfo { about });
                                }
                                Err(error) => log::debug!("own info not fetched: {error}"),
                            }
                        }
                    });
                }
            }
            E::Disconnected(disconnected) => {
                if matches!(self.status, LinkStatus::Connected | LinkStatus::Connecting) {
                    self.set_status(LinkStatus::Disconnected {
                        reason: disconnected.reason.to_string(),
                    });
                }
            }
            E::LoggedOut(_) => self.on_logged_out().await,
            E::ConnectFailure(failure) => {
                if !failure.reason.is_logged_out() {
                    let detail = failure
                        .message
                        .as_ref()
                        .map(|message| format!(": {message}"))
                        .unwrap_or_default();
                    self.emit(Event::Error(format!(
                        "WhatsApp connection failed ({:?}){detail}",
                        failure.reason
                    )));
                }
            }
            E::StreamReplaced(_) => {
                self.emit(Event::Error(
                    "Another WhatsApp Web session replaced this one".to_owned(),
                ));
            }
            E::TemporaryBan(ban) => {
                self.set_status(LinkStatus::Failed(format!(
                    "WhatsApp has temporarily blocked this account ({:?})",
                    ban.code
                )));
            }
            E::ClientOutdated(_) => {
                self.set_status(LinkStatus::Failed(
                    "WhatsApp rejected this version of ZapFast. Update the app".to_owned(),
                ));
            }
            E::Messages(batch) => {
                for inbound in batch.messages.iter() {
                    self.ingest(&inbound.message, &inbound.info);
                }
            }
            E::UndecryptableMessage(undecryptable) => {
                self.ingest_undecryptable(
                    &undecryptable.info,
                    undecryptable.unavailable_type,
                    undecryptable.decrypt_fail_mode,
                );
            }
            E::Receipt(receipt) => self.on_receipt(receipt),
            E::ChatPresence(presence) => {
                self.learn_source(&presence.source);
                // Match WhatsApp: only other participants appear as typing,
                // including when our presence arrives from a linked device.
                if self.is_me(&self.canonical(&presence.source.sender)) {
                    return;
                }
                self.emit(Event::Typing {
                    chat: self.canonical(&presence.source.chat),
                    sender: self.canonical(&presence.source.sender),
                    // Recording a voice note also shows as typing, as before.
                    composing: matches!(
                        presence.state,
                        ChatActivity::Typing | ChatActivity::RecordingAudio
                    ),
                });
            }
            E::Presence(presence) => {
                self.emit(Event::Presence {
                    id: self.canonical(&presence.from),
                    online: presence.status != PresenceStatus::Unavailable,
                    last_seen: presence.last_seen.map(|when| when.timestamp()),
                });
            }
            E::ContactUpdate(update) => self.on_contact_update(update),
            E::GroupUpdate(update) => {
                let chat = self.canonical(&update.group_jid);
                use whatsapp_rust::wacore::stanza::groups::GroupNotificationAction;
                // A group made on the phone can announce itself before any
                // message arrives; without a row the metadata has nowhere to land.
                // The row is dated by the notice, or it would sort last.
                if matches!(&*update.action, GroupNotificationAction::Create { .. })
                    && !self.predates_removal(&chat, update.timestamp.timestamp())
                {
                    self.ensure_chat(&chat, None);
                    let _ = self
                        .archive
                        .touch_activity(&chat, update.timestamp.timestamp());
                    self.emit_chat(&chat);
                }
                if let GroupNotificationAction::Description { description, .. } = &*update.action {
                    *self.description_generation.entry(chat.clone()).or_default() += 1;
                    self.ensure_chat(&chat, None);
                    if self
                        .archive
                        .set_group_description(&chat, description.as_deref().unwrap_or_default())
                        .is_ok()
                    {
                        self.emit_chat(&chat);
                    }
                }
                if let GroupNotificationAction::Ephemeral { expiration, .. } = &*update.action {
                    self.ensure_chat(&chat, None);
                    let timestamp = update.timestamp.timestamp();
                    let accepted = self
                        .archive
                        .set_ephemeral(&chat, *expiration, timestamp)
                        .unwrap_or(false);
                    log::debug!(
                        target: "zapfast::disappearing",
                        "group timer update: duration={expiration}s timestamp={timestamp} accepted={accepted}"
                    );
                    if accepted {
                        self.emit_chat(&chat);
                    }
                }
                // Who may edit the group's info changes at once; the refresh
                // below confirms it along with everything else.
                let locked = match &*update.action {
                    GroupNotificationAction::Locked { .. } => Some(true),
                    GroupNotificationAction::Unlocked => Some(false),
                    _ => None,
                };
                if let Some(locked) = locked
                    && self.archive.set_info_locked(&chat, locked).is_ok()
                {
                    self.emit_chat(&chat);
                }
                self.request_group_info(&chat, true);
            }
            E::ArchiveUpdate(update) => {
                let chat = self.canonical(&update.jid);
                // The update can arrive before history creates the chat.
                self.ensure_chat(&chat, None);
                let _ = self.archive.set_archived_at(
                    &chat,
                    update.action.archived.unwrap_or(false),
                    update.timestamp.timestamp_millis(),
                );
                self.emit_chat(&chat);
            }
            E::PinUpdate(update) => {
                let chat = self.canonical(&update.jid);
                self.ensure_chat(&chat, None);
                let _ = self.archive.set_pinned_at(
                    &chat,
                    update.action.pinned.unwrap_or(false),
                    update.timestamp.timestamp_millis(),
                );
                self.emit_chat(&chat);
            }
            E::MuteUpdate(update) => {
                let chat = self.canonical(&update.jid);
                // App state keeps mutes of channels long unfollowed, and a
                // channel's own Mute comes from the server's list instead.
                // Such a mute must not file an empty "Unknown" channel.
                if chat.ends_with("@newsletter") && matches!(self.archive.chat(&chat), Ok(None)) {
                    return;
                }
                self.ensure_chat(&chat, None);
                let until = if update.action.muted.unwrap_or(false) {
                    Some(seconds(update.action.mute_end_timestamp.unwrap_or(0)))
                } else {
                    None
                };
                let _ =
                    self.archive
                        .set_muted_at(&chat, until, update.timestamp.timestamp_millis());
                self.emit_chat(&chat);
            }
            E::RemoveRecentStickerUpdate(update) => self.recent_sticker_removed(update),
            E::FavoriteStickerUpdate(update) => self.favorite_sticker_update(update),
            E::FavoritesUpdate(update) => self.favorite_chats_update(update),
            E::StarUpdate(update) => self.star_update(update),
            E::LockChatUpdate(update) => {
                let chat = self.canonical(&update.jid);
                self.ensure_chat(&chat, None);
                let locked = update.action.locked.unwrap_or(false);
                let _ =
                    self.archive
                        .set_locked_at(&chat, locked, update.timestamp.timestamp_millis());
                self.emit_chat(&chat);
            }
            E::DeleteChatUpdate(update) => {
                log::info!("chat removal: received delete update");
                let chat = self.canonical_sync_chat(&update.jid).await;
                let through = removal_point(
                    update
                        .action
                        .message_range
                        .as_option()
                        .and_then(|range| range.last_message_timestamp),
                    update.timestamp.timestamp(),
                );
                self.remove_chat(&chat, through, update.delete_media);
            }
            E::ClearChatUpdate(update) => {
                log::info!("chat removal: received clear update");
                let chat = self.canonical_sync_chat(&update.jid).await;
                let through = removal_point(
                    update
                        .action
                        .message_range
                        .as_option()
                        .and_then(|range| range.last_message_timestamp),
                    update.timestamp.timestamp(),
                );
                let _ = self.empty_chat(&chat, through, update.delete_media);
            }
            E::DeleteMessageForMeUpdate(update) => {
                self.canonical_sync_chat(&update.chat_jid).await;
                self.delete_message_here(&update.chat_jid.to_string(), &update.message_id);
            }
            E::MarkChatAsReadUpdate(update) => {
                let chat = self.canonical(&update.jid);
                self.ensure_chat(&chat, None);
                if update.action.read.unwrap_or(true) {
                    let through = update
                        .action
                        .message_range
                        .as_option()
                        .and_then(|range| range.last_message_timestamp);
                    if let Some(through) = through {
                        let _ = self.archive.mark_read_through(&chat, seconds(through));
                    } else {
                        let _ = self.archive.mark_read(&chat);
                    }
                    let _ = self.archive.set_marked_unread(&chat, false);
                } else {
                    // Marked unread on another device: the empty dot, as the
                    // phone shows it, with any real count kept.
                    let _ = self.archive.finish_read_sync(&chat, i64::MAX);
                    let _ = self.archive.set_marked_unread(&chat, true);
                }
                self.emit_chat(&chat);
            }
            E::HistorySync(lazy) => self.on_history_sync(lazy).await,
            E::DisappearingModeChanged(update) => {
                // This is a contact's default for new conversations, not a
                // timer change in an existing chat. Per-chat changes arrive
                // as EPHEMERAL_SETTING or a typed group Ephemeral action.
                let id = self.canonical(&update.from);
                let timestamp = update.setting_timestamp.timestamp();
                if self.is_me(&id) {
                    let stored = self
                        .archive
                        .meta("default_ephemeral_setting_timestamp")
                        .ok()
                        .flatten()
                        .and_then(|value| value.parse::<i64>().ok())
                        .unwrap_or_default();
                    if timestamp >= stored {
                        let _ = self
                            .archive
                            .set_meta("default_ephemeral_expiration", &update.duration.to_string());
                        let _ = self.archive.set_meta(
                            "default_ephemeral_setting_timestamp",
                            &timestamp.to_string(),
                        );
                    }
                }
            }
            E::PictureUpdate(update) => {
                let id = self.canonical(&update.jid);
                self.refresh_avatar(id, update.removed);
            }
            E::UserAboutUpdate(update) if self.is_me(&self.canonical(&update.jid)) => {
                let _ = self.archive.set_meta("me_about", &update.status);
                self.me_about = Some(update.status.clone()).filter(|about| !about.is_empty());
                self.emit(self.me_event());
            }
            E::UnarchiveChatsSettingUpdate(update) => {
                match self.archive.set_unarchive_chats_at(
                    update.unarchive_chats,
                    update.timestamp.timestamp_millis(),
                ) {
                    Ok(unarchive) => {
                        self.phone_unarchive_chats = Some(unarchive);
                        self.phone_unarchive_unread = false;
                    }
                    Err(error) => {
                        log::warn!("could not store the phone's archive setting: {error}")
                    }
                }
            }
            E::SelfPushNameUpdated(update) => {
                self.me_name = Some(update.new_name.clone());
                let _ = self.archive.set_meta("me_name", &update.new_name);
                self.emit(self.me_event());
                self.push_name_synced();
            }
            E::OfflineSyncCompleted(_) => self.emit_chats(),
            E::ServerAck(ack) => self.server_ack(ack),
            _ => {}
        }
    }

    /// Sends a new display name and About text; each `None` stays as it is.
    fn set_profile(&mut self, name: Option<String>, about: Option<String>) {
        let Some(client) = self.client.clone() else {
            self.emit(Event::Error(
                "Connect to WhatsApp to change your profile.".to_owned(),
            ));
            return;
        };
        let commands = self.commands.clone();
        let events = self.events.clone();
        let waker = self.waker.clone();
        tokio::spawn(async move {
            let profile = client.profile();
            let mut saved_name = None;
            if let Some(name) = name {
                match profile.set_push_name(&name).await {
                    Ok(outcome) => {
                        if let whatsapp_rust::PushNameOutcome::SyncPending { source } = outcome {
                            log::warn!("name changed, but not yet synced to the phone: {source}");
                        }
                        saved_name = Some(name);
                    }
                    Err(error) => {
                        let _ = events
                            .send(Event::Error(format!("Could not change your name: {error}")));
                    }
                }
            }
            let mut saved_about = None;
            if let Some(about) = about {
                match profile.set_status_text(&about).await {
                    Ok(()) => saved_about = Some(about),
                    Err(error) => {
                        let _ = events.send(Event::Error(format!(
                            "Could not change your About: {error}"
                        )));
                    }
                }
            }
            if saved_name.is_some() || saved_about.is_some() {
                let _ = commands.send(Command::ProfileSaved {
                    name: saved_name,
                    about: saved_about,
                    picture: false,
                });
            }
            waker.wake();
        });
    }

    fn remember_identity(&mut self, pn: Option<Jid>, lid: Option<Jid>, name: Option<String>) {
        if let Some(pn) = pn {
            let pn = pn.to_non_ad_string();
            let _ = self.archive.set_meta("me_pn", &pn);
            self.me_pn = Some(pn);
        }
        if let Some(lid) = lid {
            let lid = lid.to_non_ad_string();
            let _ = self.archive.set_meta("me_lid", &lid);
            self.me_lid = Some(lid);
        }
        if let (Some(pn), Some(lid)) = (self.me_pn.clone(), self.me_lid.clone())
            && let (Some(pn), Some(lid)) = (Self::jid_of(&pn), Self::jid_of(&lid))
        {
            self.learn_pair(&lid, &pn);
        }
        if let Some(name) = name.filter(|name| !name.is_empty()) {
            let _ = self.archive.set_meta("me_name", &name);
            self.me_name = Some(name);
        }
        self.emit(self.me_event());
    }

    async fn on_logged_out(&mut self) {
        self.contact_generation = self.contact_generation.wrapping_add(1);
        self.privacy_generation = self.privacy_generation.wrapping_add(1);
        self.message_removals_in_flight.clear();
        // End any call the old account was on before its data is cleared. A runtime left standing
        // would keep running for an account that is gone, hold the next account's calls off, and
        // later write its record into the archive this logout is about to clear.
        self.shutdown_call().await;
        self.edits.clear();
        self.sticker_download_failed.clear();
        self.sticker_clicked.clear();
        self.sticker_retried.clear();
        self.sticker_arrivals = 0;
        self.stop_bot().await;
        if let Err(error) = self.archive.clear() {
            log::warn!("could not clear the archive: {error}");
        }
        self.lid_to_pn.clear();
        self.contacts.clear();
        self.group_info_requested.clear();
        self.group_info_queue.clear();
        self.group_info_tries.clear();
        self.group_info_retry.clear();
        self.presence_subscribed.clear();
        self.read_sync = ReadSync::default();
        self.favorite_chats = Default::default();
        self.poll_sending.clear();
        self.interactive_sending.clear();
        self.poll_history = Default::default();
        self.prefetch.reset_session();
        self.late_older.clear();
        self.older_ids_match = false;
        self.forward_queue = None;
        self.pending_older.clear();
        self.pending_avatars.clear();
        // A star request belongs to the account that just left. A task that
        // finishes after logout must not write the next account, whose
        // generations start over.
        self.star_generation.clear();
        self.pin_generation.clear();
        // Pin answers carry the same session as star answers.
        self.star_session = self.star_session.wrapping_add(1);
        self.me_pn = None;
        self.me_lid = None;
        self.me_name = None;
        self.me_about = None;
        self.qr = None;
        self.pair_code = None;
        self.pairing_phone = None;
        self.set_syncing(false);
        let session = self.dirs.session_db();
        for suffix in ["", "-wal", "-shm", "-journal"] {
            let mut path = session.clone().into_os_string();
            path.push(suffix);
            let _ = std::fs::remove_file(path);
        }
        let _ = std::fs::remove_dir_all(self.dirs.avatar_cache_dir());
        let _ = std::fs::remove_dir_all(self.dirs.media_cache_dir());
        self.emit(Event::Chats(Vec::new()));
        self.emit_labels();
        self.emit(Event::Drafts(Vec::new()));
        self.privacy_ready = false;
        self.privacy_confirmed = false;
        self.privacy_snapshot = false;
        self.phone_unarchive_chats = None;
        self.phone_unarchive_unread = false;
        // Reads for the unlinked account must not be answered for the next.
        self.withheld_pages.clear();
        self.early.clear();
        self.privacy_reveal_at = None;
        self.privacy_attempts = 0;
        self.privacy_warned = false;
        self.privacy_recovering = false;
        self.privacy_retry = Instant::now();
        self.set_status(LinkStatus::LoggedOut);
        // Recreate the store so the next connection starts linking.
        self.start_bot().await;
    }

    fn on_contact_update(&mut self, update: &wa_events::ContactUpdate) {
        if let (Some(lid), Some(pn)) = (&update.action.lid_jid, &update.action.pn_jid)
            && let (Some(lid), Some(pn)) = (Self::jid_of(lid), Self::jid_of(pn))
        {
            self.learn_pair(&lid, &pn);
        }
        let id = self.canonical(&update.jid);
        let name = update
            .action
            .full_name
            .clone()
            .or_else(|| update.action.first_name.clone())
            .filter(|name| !name.is_empty());
        let first_name = update
            .action
            .first_name
            .clone()
            .filter(|first| name.is_some() && !first.is_empty());
        let contact = self.contacts.entry(id.clone()).or_insert_with(|| Contact {
            id: id.clone(),
            ..Contact::default()
        });
        if contact.full_name == name && contact.first_name == first_name {
            return;
        }
        contact.full_name = name;
        contact.first_name = first_name;
        let contact = contact.clone();
        if let Err(error) = self.archive.upsert_contact(&contact) {
            log::warn!("could not save a contact: {error}");
        }
        self.emit(Event::Contacts(vec![contact]));
        self.refresh_chat_name(&id);
    }

    fn on_receipt(&mut self, receipt: &wa_events::Receipt) {
        self.learn_source(&receipt.source);
        let chat = self.canonical(&receipt.source.chat);
        log::debug!(
            "receipt {:?} from {} (chat {chat}, from me: {}, offline: {}) for {:?}",
            receipt.r#type,
            receipt.source.sender,
            receipt.source.is_from_me,
            receipt.offline,
            receipt.message_ids
        );
        let status = match receipt.r#type {
            ReceiptType::Delivered => Delivery::Delivered,
            // An inactive-device receipt still means delivered.
            ReceiptType::Inactive => Delivery::Delivered,
            ReceiptType::Read => Delivery::Read,
            ReceiptType::Played => Delivery::Played,
            ReceiptType::ReadSelf | ReceiptType::PlayedSelf => {
                let played = receipt.r#type == ReceiptType::PlayedSelf;
                // The receipt time is when the phone read, not the position
                // it read through. A delayed receipt must leave newer messages.
                for id in &receipt.message_ids {
                    match self.archive.message(&chat, id) {
                        Ok(Some(message)) if !message.from_me => {
                            let _ = self.archive.mark_read_to(&chat, id);
                            if played {
                                self.store_played(&chat, id);
                            }
                        }
                        // Offline, messages are filed in batches after their
                        // receipts: the read applies once the message is.
                        Ok(None) if !receipt.source.chat.is_status_broadcast() => {
                            self.early.wait_read(&chat, id, played);
                        }
                        _ => {}
                    }
                }
                self.emit_chat(&chat);
                return;
            }
            // Own-device delivery counts as read only in the self chat.
            ReceiptType::Sender if chat == self.me() => Delivery::Read,
            _ => return,
        };
        let at = receipt.timestamp.timestamp();
        if ChatKind::from_id(&chat) == ChatKind::Group {
            let recipient = self.canonical(&receipt.source.sender);
            if self.is_me(&recipient) {
                return;
            }
            for id in &receipt.message_ids {
                // A receipt for a message we have not archived yet is kept
                // until the message arrives; see `settle_early_receipts`.
                if self
                    .archive
                    .message(&chat, id)
                    .ok()
                    .flatten()
                    .is_some_and(|row| !row.from_me)
                {
                    continue;
                }
                match self
                    .archive
                    .group_receipt(&chat, id, &recipient, status, at)
                {
                    Ok(true) => self.emit_message(&chat, id),
                    Ok(false) => {}
                    Err(error) => log::warn!("could not file a group receipt: {error}"),
                }
                if self.watching_receipts(&chat, id) {
                    self.emit_receipts();
                }
            }
            self.emit_chat(&chat);
            return;
        }
        let mut newest = 0;
        let mut changed = 0;
        for id in &receipt.message_ids {
            if !receipt.source.chat.is_status_broadcast()
                && matches!(self.archive.message(&chat, id), Ok(None))
            {
                let recipient = chat.clone();
                if let Err(error) = self.archive.file_receipt(&chat, id, &recipient, status, at) {
                    log::warn!("could not keep an early receipt: {error}");
                }
                continue;
            }
            match self.archive.set_status(&chat, id, status, at) {
                Ok(true) => {
                    changed += 1;
                    self.emit_message(&chat, id);
                }
                Ok(false) => {}
                Err(_error) => log::warn!("could not file a receipt"),
            }
            if let Ok(Some(message)) = self.archive.message(&chat, id) {
                newest = newest.max(message.timestamp);
            }
        }
        log::debug!(
            "receipt moved {changed} of {} messages in {chat} to {status:?}",
            receipt.message_ids.len()
        );
        // Read receipts advance all earlier messages.
        if status >= Delivery::Read
            && newest > 0
            && let Ok(ids) = self.archive.advance_statuses(&chat, newest, status, at)
        {
            for id in ids {
                self.emit_message(&chat, &id);
            }
        }
        self.emit_chat(&chat);
    }

    /// Hourly, as opening the archive only does it at startup.
    fn prune_waiting_receipts(&mut self) {
        if self.receipts_pruned.elapsed() < Duration::from_secs(60 * 60) {
            return;
        }
        self.receipts_pruned = Instant::now();
        self.early.prune(Instant::now());
        if let Err(error) = self.archive.prune_waiting_receipts() {
            log::warn!("could not drop stale receipts: {error}");
        }
    }

    fn watching_receipts(&self, chat: &str, id: &str) -> bool {
        self.receipts_watch
            .as_ref()
            .is_some_and(|(watched, message)| watched == chat && message == id)
    }

    /// Sends the followed message's receipts, with privacy ids resolved as far
    /// as they are known.
    fn emit_receipts(&self) {
        let Some((chat, message)) = self.receipts_watch.clone() else {
            return;
        };
        let recipients = match self.archive.receipts(&chat, &message) {
            Ok(recipients) => recipients,
            Err(error) => {
                log::warn!("could not load a message's receipts: {error}");
                return;
            }
        };
        let recipients = recipients
            .into_iter()
            .map(|recipient| crate::model::Recipient {
                id: self.canonical_str(&recipient.id),
                ..recipient
            })
            .filter(|recipient| !self.is_me(&recipient.id))
            .collect();
        self.emit(Event::Receipts(crate::model::MessageReceipts {
            chat,
            message,
            recipients,
        }));
    }

    /// Applies receipts that arrived before one of our messages. Messages we
    /// send from another device reach us after their recipients' receipts
    /// often enough: receipts are handled while messages are still decrypted.
    ///
    /// Such a group message has no saved audience. The group's current members
    /// are the best record of who it went to, so they become its audience;
    /// without one, its ticks could never pass "sent".
    fn settle_early_receipts(&mut self, chat: &str, id: &str) {
        if !matches!(self.archive.message(chat, id), Ok(Some(_))) {
            return;
        }
        if ChatKind::from_id(chat) == ChatKind::Group {
            if !self.archive.has_group_audience(chat, id).unwrap_or(true) {
                let members = self
                    .archive
                    .chat(chat)
                    .ok()
                    .flatten()
                    .map(|chat| chat.participants)
                    .unwrap_or_default();
                if !members.is_empty() {
                    self.save_group_recipients(chat, id, &members);
                }
            }
            match self.archive.settle_group(chat, id) {
                Ok(true) => self.emit_message(chat, id),
                Ok(false) => {}
                Err(error) => log::warn!("could not apply early group receipts: {error}"),
            }
            if self.watching_receipts(chat, id) {
                self.emit_receipts();
            }
            return;
        }
        match self.archive.settle_direct(chat, id) {
            Ok(Some((status, at))) => {
                self.emit_message(chat, id);
                // As with a live read receipt, earlier messages were read too.
                if status >= Delivery::Read
                    && let Ok(Some(message)) = self.archive.message(chat, id)
                    && let Ok(ids) =
                        self.archive
                            .advance_statuses(chat, message.timestamp, status, at)
                {
                    for id in ids {
                        self.emit_message(chat, &id);
                    }
                }
                self.emit_chat(chat);
            }
            Ok(None) => {}
            Err(error) => log::warn!("could not apply early receipts: {error}"),
        }
    }

    /// Keeps the members' receipts the phone reported for one of our older
    /// group messages. They may be only some of the members.
    fn file_history_receipts(&mut self, chat: &str, id: &str, receipts: &[wa::UserReceipt]) {
        let mut rows = Vec::new();
        for receipt in receipts {
            let recipient = self.canonical_str(&receipt.user_jid);
            if self.is_me(&recipient) {
                continue;
            }
            for (status, at) in [
                (Delivery::Delivered, receipt.receipt_timestamp),
                (Delivery::Read, receipt.read_timestamp),
                (Delivery::Played, receipt.played_timestamp),
            ] {
                if let Some(at) = at {
                    rows.push((recipient.clone(), status, at));
                }
            }
        }
        if rows.is_empty() {
            return;
        }
        match self.archive.file_receipts(chat, id, &rows) {
            Ok(true) => self.emit_message(chat, id),
            Ok(false) => {}
            Err(error) => log::warn!("could not keep history receipts: {error}"),
        }
    }

    /// Returns raw mention tokens and canonical ids.
    fn mentions_of(&self, raw: &[String]) -> Vec<MentionRef> {
        raw.iter()
            .filter_map(|jid| {
                if jid == crate::mentions::ALL_ID {
                    return Some(crate::mentions::everyone_ref());
                }
                let user = jid.split('@').next()?.to_owned();
                if user.is_empty() {
                    return None;
                }
                Some(MentionRef {
                    user,
                    id: self.canonical_str(jid),
                })
            })
            .collect()
    }

    /// Whether this chat may mention everyone and `text` carries `@all`.
    fn wants_mention_all(&self, chat: &str, text: &str) -> bool {
        crate::mentions::has_everyone_token(text)
            && self
                .archive
                .chat(chat)
                .ok()
                .flatten()
                .is_some_and(|chat| chat.can_mention_everyone())
    }

    fn ingest(&mut self, message: &Arc<wa::Message>, info: &MessageInfo) {
        self.learn_source(&info.source);
        if info.source.chat.is_status_broadcast() {
            return;
        }
        let chat = self.canonical(&info.source.chat);
        let from_me = info.source.is_from_me;
        let sender = if from_me {
            self.me()
        } else {
            self.canonical(&info.source.sender)
        };
        let push_name = (!info.push_name.is_empty()).then(|| info.push_name.clone());
        let base = message.get_base_message();
        if let Some(expiration) = base.get_ephemeral_expiration()
            && self
                .archive
                .ephemeral_expiration(&chat)
                .ok()
                .flatten()
                .is_none()
        {
            self.ensure_chat(&chat, push_name.as_deref());
            let _ = self.archive.set_ephemeral(&chat, expiration, 0);
        }

        if let Some(protocol) = base.protocol_message.as_option() {
            use wa::message::protocol_message::Type;
            if protocol.r#type == Some(Type::EPHEMERAL_SETTING) {
                if let Some(expiration) = protocol.ephemeral_expiration {
                    let timestamp = protocol
                        .ephemeral_setting_timestamp
                        .unwrap_or_else(|| info.timestamp.timestamp());
                    let used_fallback = protocol.ephemeral_setting_timestamp.is_none();
                    self.ensure_chat(&chat, push_name.as_deref());
                    let accepted = self
                        .archive
                        .set_ephemeral(&chat, expiration, timestamp)
                        .unwrap_or(false);
                    log::debug!(
                        target: "zapfast::disappearing",
                        "protocol timer update: duration={expiration}s timestamp={timestamp} fallback_timestamp={used_fallback} accepted={accepted}"
                    );
                    if accepted {
                        self.emit_chat(&chat);
                    }
                } else {
                    log::debug!(
                        target: "zapfast::disappearing",
                        "protocol timer update missing expiration"
                    );
                }
                return;
            }
            let Some(target) = protocol.key.as_option().and_then(|key| key.id.clone()) else {
                return;
            };
            match protocol.r#type {
                Some(Type::REVOKE) => {
                    let old = self.archive.message(&chat, &target).ok().flatten();
                    // A late answer to an edit sent from here must not
                    // replace what the phone did since.
                    self.edits.remove(&(chat.clone(), target.clone()));
                    if let Ok(true) =
                        self.archive
                            .set_content(&chat, &target, &Content::Revoked, false)
                    {
                        self.emit_message(&chat, &target);
                        self.emit_chat(&chat);
                        self.media_changed(&chat, old.as_ref().map(|old| &old.content), None);
                    }
                }
                Some(Type::MESSAGE_EDIT) => {
                    self.edits.remove(&(chat.clone(), target.clone()));
                    if let Some(edited) = protocol.edited_message.as_option()
                        && let Some(mut content) = classify(edited)
                    {
                        let mentions =
                            self.mentions_of(&recorded_mention_ids(edited.get_base_message()));
                        // Preserve downloaded media when updating a caption.
                        let existing = self.archive.message(&chat, &target).ok().flatten();
                        if let Some(existing) = &existing {
                            content.keep_local_paths(&existing.content);
                        }
                        if let Ok(true) = self
                            .archive
                            .set_edited_content(&chat, &target, &content, &mentions)
                        {
                            self.emit_message(&chat, &target);
                            self.emit_chat(&chat);
                            self.media_changed(
                                &chat,
                                existing.as_ref().map(|existing| &existing.content),
                                Some(&content),
                            );
                        }
                    }
                }
                _ => {}
            }
            return;
        }
        if let Some(reaction) = base.reaction_message.as_option() {
            self.store_plain_reaction(
                &chat,
                &sender,
                from_me,
                reaction,
                info.timestamp.timestamp_millis(),
            );
            return;
        }
        if base.enc_reaction_message.is_set() {
            self.store_enc_reaction(
                &chat,
                &sender,
                from_me,
                base,
                info.timestamp.timestamp_millis(),
            );
            return;
        }
        if let Some(update) = base.poll_update_message.as_option() {
            self.ingest_poll_vote(
                &chat,
                &info.id,
                &info.source.sender.to_non_ad_string(),
                from_me,
                info.timestamp.timestamp(),
                update,
            );
            return;
        }
        if let Some(pin) = base.pin_in_chat_message.as_option() {
            self.ingest_pin(
                &chat,
                pin,
                base,
                info.timestamp.timestamp(),
                &sender,
                from_me,
            );
            return;
        }
        if self.update_live_location(&chat, &sender, base, info) {
            return;
        }
        if let Some((parent, clip)) = motion_clip(base) {
            self.file_motion_clip(&chat, &parent, &sender, &clip);
            return;
        }
        let Some(mut content) = classify(message) else {
            return;
        };
        if let Content::PhoneOnly { live_location, .. } = &mut content
            && info.media_type == Some(EncMediaType::LiveLocation)
        {
            *live_location = true;
            if self.absorb_masked_live_location(&chat, &sender, &info.id, info) {
                return;
            }
        }
        let quoted = self.quoted_of(base);
        let mentions = self.mentions_of(&recorded_mention_ids(base));
        let row = Message {
            id: info.id.to_string(),
            chat: chat.clone(),
            sender,
            sender_name: if from_me {
                None
            } else {
                push_name.as_ref().map(ToString::to_string)
            },
            from_me,
            timestamp: info.timestamp.timestamp(),
            content,
            status: if from_me {
                Delivery::Sent
            } else {
                Delivery::None
            },
            delivered_at: None,
            read_at: None,
            quoted,
            reactions: Vec::new(),
            history_order: None,
            edited: false,
            mentions,
            forwarded: forwarded_of(base),
            thumbnail: thumbnail_of(base),
        };
        let is_poll = matches!(row.content, Content::Poll { .. });
        self.remember_poll(&row, message, &info.source.sender.to_non_ad_string(), None);
        // A new, normally delivered creation has no earlier votes to recover.
        // Offline delivery, PDO recovery and replay of an archived poll do not
        // establish that baseline: they may already have votes on the phone.
        let poll_baseline = is_poll
            && !info.is_offline
            && info.unavailable_request_id.is_none()
            && matches!(self.archive.message(&chat, &row.id), Ok(None));
        // A placeholder stored while the message could not be opened does
        // not count: this is the first time its content arrives.
        let sent_elsewhere = from_me
            && match self.archive.message(&chat, &row.id) {
                Ok(None) => true,
                Ok(Some(stored)) => stored.content.is_placeholder(),
                Err(_) => false,
            };
        let id = row.id.clone();
        self.archive_message(
            row,
            Some(message.encode_to_vec()),
            push_name.as_deref(),
            poll_baseline,
        );
        if sent_elsewhere {
            self.settle_early_receipts(&chat, &id);
        }
        if is_poll {
            self.pump_poll_votes();
        }
    }

    /// Applies a live location position to the share it belongs to, so a
    /// moving sender keeps one bubble and one archive row. Returns false when
    /// the message starts a share, which is then stored like any other.
    fn update_live_location(
        &mut self,
        chat: &str,
        sender: &str,
        base: &wa::Message,
        info: &MessageInfo,
    ) -> bool {
        let Some((mut content, reference)) = live_location_of(base) else {
            return false;
        };
        let now = info.timestamp.timestamp();
        if let Content::LiveLocation { updated, .. } = &mut content {
            *updated = now;
        }
        let Some((mut share, named)) = self.live_share(chat, sender, &info.id, reference, now)
        else {
            return false;
        };
        if !live_location_newer(&share, &content) {
            // A position the named share already has, or an older one,
            // changes nothing. One that only looks like it continues the
            // sender's latest share starts a new share instead.
            return named;
        }
        share.content = content;
        if let Some(thumbnail) = thumbnail_of(base) {
            share.thumbnail = Some(thumbnail);
        }
        // The row keeps its start time, so a moving share does not reorder
        // the chat list or the conversation.
        if let Err(error) = self.archive.insert_message(&share, None) {
            log::warn!("could not store a live location update: {error}");
            return true;
        }
        self.emit_message(chat, &share.id);
        true
    }

    /// Whether a masked live location only continues the share `sender`
    /// last posted in this chat. Linked devices cannot follow the position,
    /// so a share keeps the one bubble it started with: the card it already
    /// shows keeps its last position and says the newer ones are on the
    /// phone, and a share this device was never able to read keeps its
    /// single placeholder.
    fn absorb_masked_live_location(
        &mut self,
        chat: &str,
        sender: &str,
        id: &str,
        info: &MessageInfo,
    ) -> bool {
        let now = info.timestamp.timestamp();
        if self.mark_share_on_the_phone(chat, sender, now) {
            return true;
        }
        let Ok(Some(latest)) = self.archive.latest_id_from(chat, sender) else {
            return false;
        };
        if latest == id {
            return false;
        }
        self.archive
            .message(chat, &latest)
            .ok()
            .flatten()
            .is_some_and(|message| {
                matches!(
                    message.content,
                    Content::PhoneOnly {
                        live_location: true,
                        ..
                    }
                ) && info.timestamp.timestamp() - message.timestamp <= LIVE_LOCATION_LIMIT
            })
    }

    /// Whether the sender's share in this chat is a card, marking it as the
    /// one that cannot follow the phone's newer positions, and whether the
    /// position that arrived belongs to it. That card is the share's whole
    /// row, so it owns the fact instead of the chat growing a second bubble.
    fn mark_share_on_the_phone(&mut self, chat: &str, sender: &str, now: i64) -> bool {
        let since = now - LIVE_LOCATION_LIMIT;
        let Ok(Some(latest)) = self.archive.unique_live_location(chat, sender, since) else {
            return false;
        };
        let Ok(Some(mut share)) = self.archive.message(chat, &latest) else {
            return false;
        };
        // The share's newest position may be newer than this event: a masked
        // event that arrives out of order says nothing about positions the
        // card already shows, so it must not claim the phone holds them.
        if live_location_time(&share) > now {
            return false;
        }
        if share.content.live_location_over(share.timestamp, now) {
            return false;
        }
        let Content::LiveLocation { newer_on_phone, .. } = &mut share.content else {
            return false;
        };
        if *newer_on_phone {
            return true;
        }
        *newer_on_phone = true;
        if let Err(error) = self.archive.insert_message(&share, None) {
            log::warn!("could not store a live location position notice: {error}");
        }
        self.emit_message(chat, &latest);
        true
    }

    /// The stored live location that a position from `sender` updates: the
    /// message it names, the same message again, or the sender's share in
    /// this chat that last moved within [`LIVE_LOCATION_GAP`]. The flag tells
    /// whether the position named its share.
    fn live_share(
        &self,
        chat: &str,
        sender: &str,
        id: &str,
        reference: Option<String>,
        now: i64,
    ) -> Option<(Message, bool)> {
        let ours = |message: &Message| {
            message.sender == sender && matches!(message.content, Content::LiveLocation { .. })
        };
        for candidate in reference.iter().map(String::as_str).chain([id]) {
            if let Ok(Some(message)) = self.archive.message(chat, candidate)
                && ours(&message)
            {
                return Some((message, true));
            }
        }
        let latest = self
            .archive
            .latest_live_location(chat, sender, now - LIVE_LOCATION_LIMIT)
            .ok()??;
        let message = self.archive.message(chat, &latest).ok()??;
        let moving = !message.content.live_location_over(message.timestamp, now)
            && now - live_location_time(&message) <= LIVE_LOCATION_GAP;
        moving.then_some((message, false))
    }

    fn store_plain_reaction(
        &mut self,
        chat: &str,
        sender: &str,
        from_me: bool,
        reaction: &wa::message::ReactionMessage,
        sent_at: i64,
    ) {
        let Some(target) = reaction
            .key
            .as_option()
            .and_then(|key| key.id.clone())
            .filter(|id| !id.is_empty())
        else {
            return;
        };
        let emoji = reaction_emoji(reaction.text.as_deref(), reaction.grouping_key.as_deref())
            .unwrap_or_default();
        let sent_at = reaction.sender_timestamp_ms.unwrap_or(sent_at);
        let body = HistoryReactionBody::Plain(emoji);
        let reaction = WaitingReaction::new(chat, &target, sender, from_me, body, sent_at);
        self.file_reaction(reaction, None);
    }

    fn store_enc_reaction(
        &mut self,
        chat: &str,
        sender: &str,
        from_me: bool,
        message: &wa::Message,
        sent_at: i64,
    ) {
        let Some(env) = extract_secret_encrypted(message) else {
            return;
        };
        if env.kind != SecretEncKind::EncReaction {
            return;
        }
        let Some(target) = env.target_id().filter(|id| !id.is_empty()) else {
            return;
        };
        let body = HistoryReactionBody::Encrypted {
            payload: env.enc_payload.to_vec(),
            iv: env.enc_iv.to_vec(),
        };
        let reaction = WaitingReaction::new(chat, target, sender, from_me, body, sent_at);
        self.file_reaction(reaction, None);
    }

    /// Applies a reaction, or its removal. One whose target is not filed yet
    /// waits for it in `early`: offline, messages are filed in batches, and a
    /// reaction's target may only come with history.
    fn file_reaction(
        &mut self,
        reaction: WaitingReaction,
        secrets: Option<&HashMap<String, Vec<u8>>>,
    ) {
        if matches!(
            self.archive.message(&reaction.chat, &reaction.target),
            Ok(None)
        ) {
            self.early.wait_reaction(reaction);
            return;
        }
        let WaitingReaction {
            chat,
            target,
            sender,
            from_me,
            body,
            sent_at,
            ..
        } = reaction;
        if let Some(updated) = self.apply_reaction(&chat, &target, &sender, from_me, body, secrets)
        {
            // As on WhatsApp, someone reacting to one of our messages brings
            // the chat up the list; history (which passes its secrets) and a
            // removed reaction do not.
            let raised = secrets.is_none()
                && !from_me
                && updated.from_me
                && updated
                    .reactions
                    .iter()
                    .any(|reaction| reaction.sender == sender && !reaction.emoji.is_empty());
            self.emit(Event::MessageUpdated(Box::new(updated)));
            if raised {
                let at = (sent_at / 1000).min(crate::util::now());
                if let Err(error) = self.archive.touch_activity(&chat, at) {
                    log::warn!("could not date a chat by its reaction: {error}");
                }
                self.emit_chat(&chat);
            }
        }
    }

    fn apply_reaction(
        &self,
        chat: &str,
        target: &str,
        sender: &str,
        from_me: bool,
        body: HistoryReactionBody,
        secrets: Option<&HashMap<String, Vec<u8>>>,
    ) -> Option<Message> {
        let emoji = match body {
            HistoryReactionBody::Plain(emoji) => emoji,
            HistoryReactionBody::Encrypted { payload, iv } => {
                self.decrypt_enc_reaction(chat, target, sender, &payload, &iv, secrets)?
            }
        };
        match self
            .archive
            .set_reaction(chat, target, sender, from_me, &emoji)
        {
            Ok(updated) => updated,
            Err(error) => {
                log::warn!("could not store a reaction: {error}");
                None
            }
        }
    }

    /// Applies the phone's read and the reactions that arrived before this
    /// message was filed. Returns whether the phone had read it.
    fn settle_early_events(&mut self, chat: &str, id: &str) -> bool {
        let Ok(Some(message)) = self.archive.message(chat, id) else {
            return false;
        };
        let read = self.early.take_read(chat, id).filter(|_| !message.from_me);
        if read.is_some() {
            let _ = self.archive.mark_read_to(chat, id);
        }
        if read == Some(true) {
            self.store_played(chat, id);
        }
        for waiting in self.early.take_reactions(chat, id) {
            // The sender's privacy id may have been mapped since.
            let sender = self.canonical_str(&waiting.sender);
            self.apply_reaction(chat, id, &sender, waiting.from_me, waiting.body, None);
        }
        read.is_some()
    }

    /// Files an incoming voice message as played, here or on another device,
    /// and tells the interface when that is news.
    fn store_played(&self, chat: &str, id: &str) {
        match self.archive.mark_played(chat, id) {
            Ok(true) => self.emit_message(chat, id),
            Ok(false) => {}
            Err(error) => log::warn!("could not store a played voice message: {error}"),
        }
    }

    /// Opens an edit history carries encrypted (#582) with the secret of the
    /// message it edits: from this chunk, the archive, or the history entry
    /// itself. Only the author edits a message, so the author is both the
    /// original's sender and the editor, by phone number or privacy id.
    /// Returns the edited body and the secret that opened it.
    fn open_history_edit(
        &self,
        chat: &str,
        target: &str,
        sender: Option<&str>,
        from_me: bool,
        edit: &SealedEdit,
        secrets: &HashMap<String, Vec<u8>>,
    ) -> Option<(wa::Message, Vec<u8>)> {
        let (author, fallback) = if from_me {
            let pn = self.me_pn.as_deref().and_then(Self::jid_of);
            let lid = self.me_lid.as_deref().and_then(Self::jid_of);
            match (pn, lid) {
                (Some(pn), lid) => (pn, lid),
                (None, Some(lid)) => (lid, None),
                (None, None) => return None,
            }
        } else {
            let author = Self::jid_of(sender.unwrap_or(chat))?;
            let fallback = self.alt_jid(&author);
            (author, fallback)
        };
        let archived = self
            .archive
            .raw(chat, target)
            .ok()
            .flatten()
            .as_deref()
            .and_then(message_secret_from_raw);
        let mut tried: Vec<&[u8]> = Vec::new();
        for secret in [
            secrets.get(target).map(Vec::as_slice),
            archived.as_deref(),
            edit.entry_secret.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            if secret.len() != 32 || tried.contains(&secret) {
                continue;
            }
            tried.push(secret);
            let Ok(inner) = decrypt_secret_encrypted_with_fallback(
                &edit.payload,
                &edit.iv,
                secret,
                SecretEncKind::MessageEdit,
                target,
                &author,
                &author,
                fallback.as_ref(),
                fallback.as_ref(),
            ) else {
                continue;
            };
            let body = inner
                .protocol_message
                .into_option()?
                .edited_message
                .into_option()?;
            return Some((body, secret.to_vec()));
        }
        None
    }

    fn apply_history_reaction(
        &mut self,
        chat: &str,
        reaction: HistoryReaction,
        secrets: &HashMap<String, Vec<u8>>,
    ) {
        let sender = if reaction.from_me {
            self.me()
        } else {
            reaction
                .sender
                .as_deref()
                .map(|sender| self.canonical_str(sender))
                .unwrap_or_else(|| chat.to_owned())
        };
        let reaction = WaitingReaction::new(
            chat,
            &reaction.target,
            &sender,
            reaction.from_me,
            reaction.body,
            reaction.sent_at,
        );
        self.file_reaction(reaction, Some(secrets));
    }

    fn decrypt_enc_reaction(
        &self,
        chat: &str,
        target: &str,
        reactor: &str,
        payload: &[u8],
        iv: &[u8],
        secrets: Option<&HashMap<String, Vec<u8>>>,
    ) -> Option<String> {
        let parent = self.archive.message(chat, target).ok().flatten()?;
        let secret = secrets
            .and_then(|secrets| secrets.get(target).cloned())
            .or_else(|| {
                self.archive
                    .poll_key(chat, target)
                    .ok()
                    .flatten()
                    .map(|(_, secret)| secret)
            })
            .or_else(|| {
                self.archive
                    .raw(chat, target)
                    .ok()
                    .flatten()
                    .as_deref()
                    .and_then(message_secret_from_raw)
            })?;
        let parent_jid = Self::jid_of(&parent.sender)?;
        let reactor_jid = Self::jid_of(reactor)?;
        let fallback_parent = self.alt_jid(&parent_jid);
        let fallback_reactor = self.alt_jid(&reactor_jid);
        let inner = decrypt_secret_encrypted_with_fallback(
            payload,
            iv,
            &secret,
            SecretEncKind::EncReaction,
            target,
            &parent_jid,
            &reactor_jid,
            fallback_parent.as_ref(),
            fallback_reactor.as_ref(),
        )
        .ok()?;
        let reaction = inner.reaction_message.as_option()?;
        Some(
            reaction_emoji(reaction.text.as_deref(), reaction.grouping_key.as_deref())
                .unwrap_or_default(),
        )
    }

    fn alt_jid(&self, jid: &Jid) -> Option<Jid> {
        if jid.is_lid() {
            let pn = self.lid_to_pn.get(jid.user_base())?;
            format!("{pn}@s.whatsapp.net").parse().ok()
        } else {
            self.lid_to_pn.iter().find_map(|(lid, pn)| {
                (*pn == jid.user_base())
                    .then(|| format!("{lid}@lid").parse().ok())
                    .flatten()
            })
        }
    }

    /// Stores a placeholder for a message this device could not open, so
    /// it never vanishes without a trace. That includes our own messages
    /// sent from the phone: WhatsApp keeps some of them, such as live
    /// locations, off linked devices, and they belong in the chat all the same.
    fn ingest_undecryptable(
        &mut self,
        info: &MessageInfo,
        unavailable: wa_events::UnavailableType,
        mode: wa_events::DecryptFailMode,
    ) {
        self.learn_source(&info.source);
        // The sender marks internal traffic, such as reactions and protocol
        // messages, as not worth a placeholder.
        if info.source.chat.is_status_broadcast() || mode == wa_events::DecryptFailMode::Hide {
            return;
        }
        let from_me = info.source.is_from_me;
        let chat = self.canonical(&info.source.chat);
        if self
            .archive
            .message(&chat, &info.id)
            .ok()
            .flatten()
            .is_some()
        {
            return;
        }
        let push_name = (!info.push_name.is_empty()).then(|| info.push_name.clone());
        let sender = if from_me {
            self.me()
        } else {
            self.canonical(&info.source.sender)
        };
        let live_location = info.media_type == Some(EncMediaType::LiveLocation);
        if live_location && self.absorb_masked_live_location(&chat, &sender, &info.id, info) {
            return;
        }
        let row = Message {
            id: info.id.to_string(),
            chat,
            sender,
            sender_name: if from_me {
                None
            } else {
                push_name.as_ref().map(ToString::to_string)
            },
            from_me,
            timestamp: info.timestamp.timestamp(),
            content: match unavailable {
                // A live location keeps moving on the phone only, so say
                // where to follow it rather than that it is on its way.
                _ if live_location => Content::PhoneOnly {
                    view_once: false,
                    live_location: true,
                    once: None,
                },
                // The phone never shares these with linked devices, so do not
                // suggest that the message is still on its way.
                wa_events::UnavailableType::ViewOnce => Content::PhoneOnly {
                    view_once: true,
                    live_location: false,
                    once: None,
                },
                wa_events::UnavailableType::Hosted | wa_events::UnavailableType::Bot => {
                    Content::PhoneOnly {
                        view_once: false,
                        live_location: false,
                        once: None,
                    }
                }
                _ => Content::Unsupported {
                    what: "Waiting for this message. Open WhatsApp on your phone".to_owned(),
                },
            },
            status: if from_me {
                Delivery::Sent
            } else {
                Delivery::None
            },
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
        let id = row.id.clone();
        let chat = row.chat.clone();
        self.store_message(row, None, push_name.as_deref());
        if from_me {
            self.settle_early_receipts(&chat, &id);
        }
    }

    /// Archives a message and emits chat and row updates.
    fn store_message(&mut self, message: Message, raw: Option<Vec<u8>>, push_name: Option<&str>) {
        self.archive_message(message, raw, push_name, false);
    }

    /// The phone's "Keep chats archived" once it has said, else Settings'.
    /// A stored setting that could not be read is read again here, and
    /// keeps chats archived while it stays unreadable.
    fn keeps_chats_archived(&mut self) -> bool {
        if self.phone_unarchive_unread {
            match self.archive.unarchive_chats() {
                Ok(setting) => {
                    self.phone_unarchive_chats = setting;
                    self.phone_unarchive_unread = false;
                }
                Err(_) => return true,
            }
        }
        self.phone_unarchive_chats
            .map_or(self.keep_chats_archived, |unarchive| !unarchive)
    }

    /// Stores `message`; `poll_baseline` records a live poll creation's
    /// zero-vote baseline once the row is archived, before the interface
    /// hears of it. A skipped or failed insert leaves no baseline behind.
    fn archive_message(
        &mut self,
        message: Message,
        raw: Option<Vec<u8>>,
        push_name: Option<&str>,
        poll_baseline: bool,
    ) {
        if self.predates_removal(&message.chat, message.timestamp)
            || self.message_was_removed(&message.chat, &message.id)
        {
            return;
        }
        let chat = message.chat.clone();
        self.ensure_chat(&chat, if message.from_me { None } else { push_name });
        if let Some(push_name) = push_name
            && !message.from_me
        {
            let sender = message.sender.clone();
            self.remember_push_name(&sender, push_name);
        }
        let existing = self.archive.message(&chat, &message.id).ok().flatten();
        let is_new = existing.is_none();
        let mut message = message;
        // A duplicate delivery or a history replay reclassifies the same
        // message. Carry what the row already knew about its files over, or
        // the insert below replaces the content with a fresh classification
        // that has no downloaded path.
        if let Some(existing) = &existing {
            message.content.keep_local_paths(&existing.content);
            keep_live_location_notice(&mut message.content, existing);
        }
        if let Err(error) = self.archive.insert_message(&message, raw.as_deref()) {
            log::warn!("could not store a message: {error}");
            return;
        }
        // Only a stored message can be moved to the server's time.
        if message.from_me && message.status == Delivery::Pending {
            self.awaiting_ack
                .insert(message.id.clone(), message.chat.clone());
        }
        self.retire_local_call_twins(&message);
        if existing
            .as_ref()
            .is_none_or(|existing| existing.content != message.content)
        {
            self.media_changed(
                &chat,
                existing.as_ref().map(|existing| &existing.content),
                Some(&message.content),
            );
        }
        if poll_baseline && let Err(error) = self.archive.mark_poll_history(&chat, &message.id) {
            log::warn!("could not store a live poll baseline: {error}");
        }
        // History is filed elsewhere, so this is a live message, ours or
        // theirs. A duplicate delivery or one older than the archiving does
        // not bring the chat back.
        if is_new
            && !self.keeps_chats_archived()
            && let Err(error) = self
                .archive
                .unarchive_for_message(&chat, message.timestamp.saturating_mul(1000))
        {
            log::warn!("could not unarchive a chat: {error}");
        }
        // The phone may have read it, and reacted, before it reached us.
        let read_on_phone = self.settle_early_events(&chat, &message.id);
        let unread = is_new
            && !message.from_me
            && !read_on_phone
            && self
                .archive
                .read_through(&chat)
                .ok()
                .flatten()
                // A new live message may share the read message's second. Its
                // distinct id already passed the duplicate check above.
                .is_none_or(|through| message.timestamp >= through);
        if unread {
            let _ = self.archive.bump_unread(&chat);
        } else if message.from_me
            && matches!(
                message.status,
                Delivery::Sent | Delivery::Delivered | Delivery::Read | Delivery::Played
            )
        {
            // A reply sent from the phone/another companion reads the preceding
            // conversation there. Replayed replies cannot clear newer arrivals.
            let _ = self.archive.mark_read_to(&chat, &message.id);
        }
        let mut stored = self
            .archive
            .message(&chat, &message.id)
            .ok()
            .flatten()
            .unwrap_or(message);
        self.polish(&mut stored);
        // Notify only for live incoming messages, not history replay.
        let incoming = (unread && !self.syncing).then(|| stored.clone());
        self.emit(Event::Messages {
            chat: chat.clone(),
            messages: vec![stored],
            older: false,
            complete: false,
        });
        // The queue is a snapshot of the chat list. A chat outside it that has
        // just become active has to be able to join, or the ten most recently
        // active chats stay the ones that were active when it was read.
        if !self.prefetch.holds(&chat) {
            self.prefetch.touch();
        }
        self.emit_chat(&chat);
        if let Some(message) = incoming {
            self.emit(Event::Incoming {
                chat,
                message: Box::new(message),
            });
        }
    }

    /// Parses a protocol quote, preserving its source chat separately from the containing conversation.
    fn quoted_of(&self, base: &wa::Message) -> Option<Quoted> {
        let context = context_of(base)?;
        let id = context.stanza_id.clone().filter(|id| !id.is_empty())?;
        let sender = context
            .participant
            .as_deref()
            .map(|participant| self.canonical_str(participant))
            .unwrap_or_default();
        let (summary, labelled, listed) = context
            .quoted_message
            .as_option()
            .map(|quoted| {
                let base = quoted.get_base_message();
                let content = classify(quoted);
                (
                    content.as_ref().map(Content::summary).unwrap_or_default(),
                    content.as_ref().map(Content::labelled),
                    self.mentions_of(&recorded_mention_ids(base)),
                )
            })
            .unwrap_or_default();
        let summary = self.pn_tokens(&summary);
        let mentions = self.quote_mentions(&summary, listed);
        Some(Quoted {
            labelled,
            chat: context
                .remote_jid
                .as_deref()
                .map(|chat| self.canonical_str(chat)),
            sender_name: self.name_for(&sender),
            id,
            sender,
            summary,
            mentions,
        })
    }

    /// Matches a quote's mentions to its summary, whose privacy-id tokens
    /// `pn_tokens` turned into phone numbers: a mention listed under its
    /// privacy id would otherwise no longer find its token, and the quote
    /// would show the bare number. Without metadata (WhatsApp often strips
    /// the quoted message's context), mentions come from the `@user` tokens.
    fn quote_mentions(&self, summary: &str, listed: Vec<MentionRef>) -> Vec<MentionRef> {
        if listed.is_empty() {
            return self.mention_tokens(summary);
        }
        listed
            .into_iter()
            .map(|mention| MentionRef {
                user: self
                    .lid_to_pn
                    .get(&mention.user)
                    .cloned()
                    .unwrap_or(mention.user),
                id: self.canonical_str(&mention.id),
            })
            .collect()
    }

    /// Replaces known privacy ids in `@user` tokens with phone-number ids.
    fn pn_tokens(&self, text: &str) -> String {
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
            match self.lid_to_pn.get(&after[..digits]) {
                Some(pn) if digits > 0 => {
                    out.push_str(pn);
                    rest = &after[digits..];
                }
                _ => rest = after,
            }
        }
        out.push_str(rest);
        out
    }

    /// Infers canonical mention ids from `@user` tokens.
    fn mention_tokens(&self, text: &str) -> Vec<MentionRef> {
        let mut found = Vec::new();
        let mut rest = text;
        while let Some(at) = rest.find('@') {
            let after = &rest[at + 1..];
            let digits = after
                .char_indices()
                .find(|(_, c)| !c.is_ascii_digit())
                .map_or(after.len(), |(index, _)| index);
            let user = &after[..digits];
            if digits >= 5 {
                let id = match self.lid_to_pn.get(user) {
                    Some(pn) => format!("{pn}@s.whatsapp.net"),
                    None => format!("{user}@s.whatsapp.net"),
                };
                let id = self.canonical_str(&id);
                if !found.iter().any(|known: &MentionRef| known.user == user) {
                    found.push(MentionRef {
                        user: user.to_owned(),
                        id,
                    });
                }
            }
            rest = after;
        }
        found
    }

    // --- history ---------------------------------------------------------

    async fn on_history_sync(&mut self, lazy: &wa_events::LazyHistorySync) {
        // The id of the request an on-demand chunk answers.
        let session = lazy.peer_data_request_session_id().map(str::to_owned);
        let on_demand =
            lazy.sync_type() == ON_DEMAND || lazy.peer_data_request_session_id().is_some();
        if !on_demand {
            self.sync_deadline = Some(Instant::now() + SYNC_QUIET);
            self.set_syncing(true);
            if let Some(progress) = lazy.progress() {
                self.emit(Event::SyncProgress(progress.min(100)));
            }
        }
        let compressed = lazy.compressed_bytes().clone();
        let parsed = tokio::task::spawn_blocking(move || parse_history(&compressed)).await;
        match parsed {
            Ok(Ok(parsed)) => {
                if on_demand {
                    log::info!(
                        "poll recovery: on-demand history received; chats={}, messages={}, standalone_votes={}",
                        parsed.chats.len(),
                        parsed
                            .chats
                            .iter()
                            .map(|chat| chat.messages.len())
                            .sum::<usize>(),
                        parsed
                            .chats
                            .iter()
                            .map(|chat| chat.poll_updates.len())
                            .sum::<usize>()
                    );
                    for chat in &parsed.chats {
                        log::info!(
                            "history reload: parsed entries={}, visible={}, reactions={}, votes={}, counts={:?}",
                            chat.diagnostics.entries,
                            chat.messages.len(),
                            chat.reactions.len(),
                            chat.poll_updates.len(),
                            chat.diagnostics,
                        );
                    }
                }
                let filed = self.apply_history(parsed, !on_demand);
                if on_demand {
                    self.answer_older(filed, session.as_deref());
                }
            }
            Ok(Err(error)) => {
                log::warn!("a history chunk could not be read");
                self.emit(Event::Error(format!(
                    "Could not read part of the chat history: {error}"
                )));
            }
            Err(_error) => log::warn!("history parsing worker failed"),
        }
        if !on_demand && lazy.progress().is_some_and(|progress| progress >= 100) {
            self.sync_deadline = Some(Instant::now() + Duration::from_secs(3));
        }
        self.emit_chats();
    }

    /// Archives a history chunk. `metadata` controls chat-state updates.
    /// Returns each chat's message count and whether the phone has more.
    fn apply_history(
        &mut self,
        parsed: ParsedHistory,
        metadata: bool,
    ) -> Vec<(ChatId, usize, Option<bool>)> {
        // Chats whose listed pictures, documents or links changed, told
        // once each rather than once per imported message.
        let mut media_changed_in: HashSet<ChatId> = HashSet::new();
        for (lid, pn) in &parsed.lids {
            if let (Some(lid), Some(pn)) = (Self::jid_of(lid), Self::jid_of(pn)) {
                self.learn_pair(&lid, &pn);
            }
        }
        if !parsed.stickers.is_empty() {
            log::info!(
                "the phone listed {} recently used stickers",
                parsed.stickers.len()
            );
        }
        for sticker in &parsed.stickers {
            let Some(hash) = sticker_hash(
                sticker.file_sha256.as_deref(),
                sticker.file_enc_sha256.as_deref(),
            ) else {
                continue;
            };
            if let Err(_error) = self.archive.upsert_phone_sticker(
                &hash,
                &sticker.encode_to_vec(),
                seconds(sticker.last_sticker_sent_ts.unwrap_or(0)),
                sticker.weight.unwrap_or(0.0),
            ) {
                log::warn!("could not store a sticker");
            }
        }
        for chat in &parsed.chats {
            if let (Some(lid), Some(pn)) = (&chat.lid_jid, &chat.pn_jid)
                && let (Some(lid), Some(pn)) = (Self::jid_of(lid), Self::jid_of(pn))
            {
                self.learn_pair(&lid, &pn);
            }
        }
        let mut filed = Vec::new();
        for (id, name) in &parsed.push_names {
            let id = self.canonical_str(id);
            self.remember_push_name(&id, name);
        }
        for mut chat in parsed.chats {
            let id = self.canonical_str(&chat.id);
            if id.ends_with("@broadcast") {
                continue;
            }
            let existing = self.archive.chat(&id).ok().flatten();
            if let Some(through) = self.archive.removal_point(&id).ok().flatten() {
                chat.messages.retain(|message| message.timestamp > through);
                // Nothing newer than the deletion: leave the chat deleted.
                if existing.is_none() && chat.messages.is_empty() {
                    continue;
                }
            }
            if metadata || existing.is_none() {
                let subject_known = chat.name.is_some()
                    || existing
                        .as_ref()
                        .is_some_and(|chat| chat.group_subject_known);
                let named_channel = existing
                    .as_ref()
                    .filter(|known| known.is_channel() && keeps_its_name(known));
                let name = match chat.name {
                    // The server's list names a channel; history may not.
                    _ if named_channel.is_some() => named_channel
                        .map(|known| known.name.clone())
                        .unwrap_or_default(),
                    Some(name) if ChatKind::from_id(&id) == ChatKind::Group => name,
                    Some(name) if !name.is_empty() => {
                        // Prefer the phone's address-book name for direct chats.
                        let contact = self.contacts.entry(id.clone()).or_insert_with(|| Contact {
                            id: id.clone(),
                            full_name: None,
                            first_name: None,
                            push_name: None,
                        });
                        if contact.full_name.is_none()
                            && !name
                                .chars()
                                .all(|c| c.is_ascii_digit() || c == '+' || c == ' ')
                        {
                            contact.full_name = Some(name.clone());
                            let contact = contact.clone();
                            let _ = self.archive.upsert_contact(&contact);
                            self.emit(Event::Contacts(vec![contact]));
                        }
                        self.chat_name(&id, None)
                    }
                    _ => existing
                        .as_ref()
                        .map_or_else(|| self.chat_name(&id, None), |chat| chat.name.clone()),
                };
                let mut row = Chat::new(id.clone(), name);
                row.group_subject_known = subject_known;
                row.last_activity = chat.last_activity;
                row.unread = existing.as_ref().map_or(0, |existing| existing.unread);
                row.archived = chat
                    .archived
                    .unwrap_or_else(|| existing.as_ref().is_some_and(|row| row.archived));
                row.pinned_at = chat
                    .pinned_at
                    .unwrap_or_else(|| existing.as_ref().map_or(0, |row| row.pinned_at));
                row.pinned = chat.pinned_at.map_or_else(
                    || existing.as_ref().is_some_and(|row| row.pinned),
                    |when| when > 0,
                );
                row.muted_until = chat
                    .muted_until
                    .unwrap_or_else(|| existing.as_ref().and_then(|row| row.muted_until));
                row.locked = chat
                    .locked
                    .unwrap_or_else(|| existing.as_ref().is_some_and(|row| row.locked));
                if self.archive.upsert_chat(&row).is_err() {
                    log::warn!("could not store a chat");
                    continue;
                }
                // History has no lock timestamp, so it must not supersede
                // an app-state update already received from the phone.
                if let Some(locked) = chat.locked {
                    let _ = self.archive.set_locked_snapshot(&id, locked);
                }
            }
            if let Some(expiration) = chat.ephemeral_expiration {
                let _ = self.archive.set_ephemeral(
                    &id,
                    expiration,
                    chat.ephemeral_setting_timestamp.unwrap_or_default(),
                );
            }
            if ChatKind::from_id(&id) == ChatKind::Group {
                self.request_group_info(&id, false);
            }
            let count = chat.messages.len();
            let mut added = 0;
            let mut repeated = 0;
            let mut removed = 0;
            let mut failed = 0;
            let mut secrets = HashMap::new();
            let mut sealed = 0;
            let mut unopened = 0;
            // The chunk's own secrets first: an encrypted edit may come
            // before the original it was made with.
            for message in &chat.messages {
                if let Some(secret) = message
                    .poll_secret
                    .as_deref()
                    .filter(|secret| secret.len() == 32)
                {
                    secrets.insert(message.id.clone(), secret.to_vec());
                }
            }
            for mut message in chat.messages {
                if self.message_was_removed(&id, &message.id) {
                    removed += 1;
                    continue;
                }
                if let Some(edit) = message.sealed_edit.take() {
                    sealed += 1;
                    let Some((body, secret)) = self.open_history_edit(
                        &id,
                        &message.id,
                        message.sender.as_deref(),
                        message.from_me,
                        &edit,
                        &secrets,
                    ) else {
                        // Without the original's secret there is nothing
                        // to show; the original, if filed, stays as it was.
                        unopened += 1;
                        continue;
                    };
                    if !fill_from_edit(&mut message, &body, secret) {
                        unopened += 1;
                        continue;
                    }
                }
                if let Some(secret) = message
                    .poll_secret
                    .as_deref()
                    .filter(|secret| secret.len() == 32)
                {
                    secrets.insert(message.id.clone(), secret.to_vec());
                }
                let poll_creator = if message.from_me {
                    self.me()
                } else {
                    message.sender.clone().unwrap_or_else(|| chat.id.clone())
                };
                let sender = if message.from_me {
                    self.me()
                } else {
                    message
                        .sender
                        .as_deref()
                        .map(|sender| self.canonical_str(sender))
                        .unwrap_or_else(|| id.clone())
                };
                if let Some(push_name) = message.push_name.as_deref()
                    && !message.from_me
                {
                    self.remember_push_name(&sender, push_name);
                }
                let reactions = message
                    .reactions
                    .into_iter()
                    .map(|(who, from_me, emoji)| Reaction {
                        sender: if from_me {
                            self.me()
                        } else {
                            who.as_deref()
                                .map(|who| self.canonical_str(who))
                                .unwrap_or_else(|| id.clone())
                        },
                        from_me,
                        emoji,
                    })
                    .collect();
                let quoted = message.quoted.map(|quoted| {
                    let sender = self.canonical_str(&quoted.sender);
                    Quoted {
                        chat: quoted.chat.as_deref().map(|chat| self.canonical_str(chat)),
                        sender_name: self.name_for(&sender),
                        sender,
                        ..quoted
                    }
                });
                let mentions = self.mentions_of(&message.mentions);
                // A direct chat's receipt times date its ticks. A group's may
                // be partial, so they only fill in "Message info".
                let group = ChatKind::from_id(&id) == ChatKind::Group;
                // A zero timestamp means the time is unknown: kept, it would
                // win `min()` and date the tick to the Unix epoch.
                let first = |at: fn(&wa::UserReceipt) -> Option<i64>| {
                    message
                        .receipts
                        .iter()
                        .filter_map(at)
                        .filter(|&at| at > 0)
                        .min()
                };
                let read = matches!(message.status, Delivery::Read | Delivery::Played);
                let delivered_at = first(|receipt| receipt.receipt_timestamp)
                    .filter(|_| !group && (read || message.status == Delivery::Delivered));
                let read_at = first(|receipt| receipt.read_timestamp).filter(|_| !group && read);
                let mut row = Message {
                    id: message.id,
                    chat: id.clone(),
                    sender,
                    sender_name: if message.from_me {
                        None
                    } else {
                        message.push_name
                    },
                    from_me: message.from_me,
                    timestamp: message.timestamp,
                    content: message.content,
                    status: message.status,
                    delivered_at,
                    read_at,
                    quoted,
                    reactions,
                    history_order: message.history_order,
                    edited: message.edited,
                    mentions,
                    forwarded: message.forwarded,
                    thumbnail: message.thumbnail,
                };
                let mut poll_history_received = false;
                let raw =
                    ensure_message_secret(message.raw, secrets.get(&row.id).map(Vec::as_slice));
                if matches!(row.content, Content::Poll { .. }) {
                    if let Ok(raw) = wa::Message::decode_from_slice(&raw) {
                        self.remember_poll(
                            &row,
                            &raw,
                            &poll_creator,
                            message.poll_secret.as_deref(),
                        );
                    }
                    poll_history_received = self.history_poll_votes(&row, &message.poll_votes);
                }
                // History replays and on-demand chunks can repeat a message the
                // archive already holds; keep the files it already downloaded.
                let mut keep_raw = false;
                let mut existed = false;
                let mut live_location_changed = false;
                let existing = self.archive.message(&id, &row.id).ok().flatten();
                // A transcript already on screen is not paged again, so a
                // status history moves forward, such as a voice message the
                // phone played, is sent to it.
                let status_advanced = existing
                    .as_ref()
                    .is_some_and(|existing| existing.status < row.status);
                // What the info panel listed for this row before it lands,
                // so a change to its pictures, documents or links is told.
                let before = existing.as_ref().map(|existing| existing.content.clone());
                if let Some(existing) = existing {
                    existed = true;
                    row.content.keep_local_paths(&existing.content);
                    keep_live_location_notice(&mut row.content, &existing);
                    // A repeated original must not undo a later edit or revoke.
                    if (existing.edited && !row.edited)
                        || matches!(existing.content, Content::Revoked)
                    {
                        row.content = existing.content.clone();
                        row.edited = existing.edited;
                        row.quoted = existing.quoted;
                        row.mentions = existing.mentions;
                        row.thumbnail = existing.thumbnail.clone();
                        keep_raw = true;
                    }
                    live_location_changed = matches!(
                        (&existing.content, &row.content),
                        (Content::LiveLocation { .. }, Content::LiveLocation { .. })
                    ) && existing.content != row.content;
                }
                if let Err(error) = self
                    .archive
                    .insert_message(&row, (!keep_raw).then_some(raw.as_slice()))
                {
                    failed += 1;
                    log::warn!("could not store a history message: {error}");
                } else {
                    if live_location_changed || status_advanced {
                        self.emit_message(&id, &row.id);
                    }
                    self.retire_local_call_twins(&row);
                    if existed {
                        repeated += 1;
                    } else {
                        added += 1;
                    }
                    if before.as_ref() != Some(&row.content)
                        && (before.as_ref().is_some_and(Content::listable)
                            || row.content.listable())
                    {
                        media_changed_in.insert(id.clone());
                    }
                }
                self.settle_early_events(&id, &row.id);
                if group {
                    self.file_history_receipts(&id, &row.id, &message.receipts);
                }
                if matches!(row.content, Content::Poll { .. }) {
                    if poll_history_received {
                        let _ = self.archive.mark_poll_history(&id, &row.id);
                        self.poll_history.finish(&id, &row.id);
                    }
                    self.emit_message(&id, &row.id);
                }
            }
            if !metadata && self.pending_older.contains_key(&id) {
                log::info!(
                    "history reload: archived added={added}, repeated={repeated}, removed={removed}, failed={failed}"
                );
            }
            if sealed > 0 {
                log::info!("history: encrypted edits={sealed}, not opened={unopened}");
            }
            for reaction in chat.reactions {
                self.apply_history_reaction(&id, reaction, &secrets);
            }
            let had_pins = !chat.pins.is_empty();
            for pin in chat.pins {
                let written = if pin.pinned {
                    let secs = if pin.duration_secs == 0 {
                        PIN_SECS
                    } else {
                        i64::from(pin.duration_secs)
                    };
                    self.archive.pin_from(
                        &id,
                        &pin.target,
                        pin.at,
                        pin.at.saturating_add(secs),
                        &pin.pinner,
                    )
                } else {
                    self.archive
                        .set_pin(&id, &pin.target, false, pin.at, 0, &pin.pinner)
                };
                if let Err(error) = written {
                    log::warn!("could not store a history pin: {error}");
                }
            }
            if had_pins {
                self.emit_pins(&id);
            }
            for update in chat.poll_updates {
                let sender = if update.from_me {
                    self.me()
                } else {
                    update.sender.unwrap_or_else(|| chat.id.clone())
                };
                self.ingest_poll_vote(
                    &id,
                    &update.id,
                    &sender,
                    update.from_me,
                    update.timestamp,
                    &update.update,
                );
            }
            for revoked in chat.revoked {
                if self
                    .archive
                    .message(&id, &revoked)
                    .ok()
                    .flatten()
                    .is_some_and(|old| old.content.listable())
                {
                    media_changed_in.insert(id.clone());
                }
                let _ = self
                    .archive
                    .set_content(&id, &revoked, &Content::Revoked, false);
            }
            for clip in chat.clips {
                let sender = if clip.from_me {
                    self.me()
                } else {
                    clip.sender
                        .as_deref()
                        .map(|sender| self.canonical_str(sender))
                        .unwrap_or_else(|| id.clone())
                };
                self.file_motion_clip(&id, &clip.parent, &sender, &clip.raw);
            }
            if (metadata || existing.is_none())
                && let Some(snapshot_unread) = chat.unread
            {
                if snapshot_unread == 0 {
                    let _ = self.archive.mark_read_through(&id, chat.last_activity);
                } else {
                    let unread = self
                        .archive
                        .history_unread(&id, snapshot_unread)
                        .unwrap_or(0);
                    let unread = existing
                        .as_ref()
                        .map_or(unread, |existing| existing.unread.max(unread));
                    let _ = self.archive.set_unread(&id, unread);
                }
            }
            if (metadata || existing.is_none())
                && let Some(marked) = chat.marked_unread
            {
                let _ = self.archive.history_marked_unread(&id, marked);
            }
            if chat.more_on_phone == Some(false) {
                // The phone holds nothing older (or will not share it): later
                // sessions skip asking instead of waiting out a silent phone.
                let _ = self.archive.set_history_start(&id);
            }
            filed.push((id, count, chat.more_on_phone));
        }
        self.pump_poll_votes();
        self.pump_poll_history();
        for (id, _, _) in &filed {
            self.emit_chat(id);
        }
        for chat in media_changed_in {
            self.emit(Event::ChatSharedChanged(chat));
        }
        filed
    }

    /// Completes the requests an on-demand history chunk answers. `session`
    /// is the id of the request the phone says it is answering.
    fn answer_older(&mut self, filed: Vec<(ChatId, usize, Option<bool>)>, session: Option<&str>) {
        for (chat, count, more_on_phone) in filed {
            let more = count > 0 && more_on_phone != Some(false);
            if count > 0 {
                // The phone answers for this chat again; a later silence is news.
                self.older_warned.remove(&chat);
            }
            match self.owner_of_answer(&chat, session) {
                AnswerOwner::Pending => {
                    let Some(request) = self.pending_older.remove(&chat) else {
                        continue;
                    };
                    if request.background {
                        // A background page fills the archive without moving
                        // the view. The app only learns that the archive holds
                        // more than it has paged.
                        self.prefetch.finish_history(&chat, more, Instant::now());
                        self.emit(Event::OlderFetched {
                            chat,
                            more,
                            silent: true,
                        });
                        continue;
                    }
                    let (before_time, before_id) = request.before;
                    match self
                        .archive
                        .messages(&chat, Some((before_time, &before_id)), 500)
                    {
                        Ok(mut messages) => {
                            log::info!("history reload: displaying rows={}", messages.len());
                            for message in &mut messages {
                                self.polish(message);
                            }
                            self.emit(Event::Messages {
                                chat: chat.clone(),
                                messages,
                                older: true,
                                complete: false,
                            })
                        }
                        Err(error) => log::warn!("could not read older messages: {error}"),
                    }
                    self.emit(Event::OlderFetched {
                        chat,
                        more,
                        silent: false,
                    });
                }
                AnswerOwner::Late(index) => {
                    // The answer to a request that already timed out is
                    // archived; tell the app to page again. A reader's
                    // request shows it, a background one stays quiet.
                    let late = self.late_older.remove(index);
                    let silent = late.as_ref().is_none_or(|late| late.background);
                    if late.is_some_and(|late| late.background) {
                        self.prefetch.answered_late(&chat);
                    }
                    self.emit(Event::OlderFetched { chat, more, silent });
                }
                AnswerOwner::Unclaimed => {
                    // Nobody here asked for this page (a poll's history
                    // request, or one from before a restart). It is archived
                    // all the same, so the app pages the archive again, but
                    // it is nobody's answer.
                    self.emit(Event::OlderFetched {
                        chat,
                        more,
                        silent: true,
                    });
                }
            }
        }
    }

    /// Which request an answer for `chat` belongs to. The phone names the
    /// request it answers, so an answer is matched by that id first: a late
    /// answer to a request that timed out is then never taken for a newer one
    /// of the same chat. Until an answer has been seen to carry a known id,
    /// an answer is matched by its chat, which is what the phone's ordering
    /// allows and what this worker did before ids were kept.
    fn owner_of_answer(&mut self, chat: &str, session: Option<&str>) -> AnswerOwner {
        let pending = self.pending_older.get(chat);
        if let Some(id) = session {
            if pending.is_some_and(|request| request.request.as_deref() == Some(id)) {
                self.older_ids_match = true;
                return AnswerOwner::Pending;
            }
            if let Some(index) = self
                .late_older
                .iter()
                .position(|late| late.chat == chat && late.request.as_deref() == Some(id))
            {
                self.older_ids_match = true;
                return AnswerOwner::Late(index);
            }
            if self.older_ids_match {
                // The id is none of ours. Only a request whose send has not
                // returned yet has no id to compare, and may be this one.
                return if pending.is_some_and(|request| request.request.is_none()) {
                    AnswerOwner::Pending
                } else {
                    AnswerOwner::Unclaimed
                };
            }
        }
        if pending.is_some() {
            return AnswerOwner::Pending;
        }
        match self.late_older.iter().position(|late| late.chat == chat) {
            Some(index) => AnswerOwner::Late(index),
            None => AnswerOwner::Unclaimed,
        }
    }

    /// Remembers a request that timed out, in case its answer still comes.
    fn remember_late(&mut self, chat: ChatId, request: OlderRequest) {
        if self.late_older.len() >= LATE_OLDER_LIMIT {
            self.late_older.pop_front();
        }
        self.late_older.push_back(LateOlder {
            chat,
            serial: request.serial,
            background: request.background,
            request: request.request,
        });
    }

    /// Times out unanswered phone-history requests.
    fn expire_older_requests(&mut self) {
        let expired: Vec<ChatId> = self
            .pending_older
            .iter()
            .filter(|(_, request)| request.asked.elapsed() > PHONE_PATIENCE)
            .map(|(chat, _)| chat.clone())
            .collect();
        for chat in expired {
            let Some(request) = self.pending_older.remove(&chat) else {
                continue;
            };
            let (explicit, background) = (request.explicit, request.background);
            self.remember_late(chat.clone(), request);
            if background {
                // Only this chat backs off; the others keep the normal
                // background pace. Nothing is shown for the failed request.
                self.prefetch.timeout_history(&chat, Instant::now());
                self.emit(Event::OlderFetched {
                    chat,
                    more: true,
                    silent: true,
                });
                continue;
            }
            self.emit(Event::OlderFetched {
                chat: chat.clone(),
                more: true,
                silent: false,
            });
            // A phone often leaves an automatic request for a short or empty
            // chat unanswered when it has nothing to add, so only a request
            // the reader made reports the silence, once per chat; later
            // retries back off quietly.
            if explicit && self.older_warned.insert(chat) {
                self.emit(Event::Error(
                    "Your phone did not send older messages. Check that it is online".to_owned(),
                ));
            }
        }
    }

    /// Asks the phone for messages before the archive's earliest one.
    /// `background` marks a prefetch request: it never moves the view and its
    /// failures stay quiet. Returns whether a request was sent.
    fn fetch_older(&mut self, chat: ChatId, explicit: bool, background: bool) -> bool {
        if let Some(request) = self.pending_older.get_mut(&chat) {
            // The reader scrolled up while an automatic request was waiting.
            request.explicit |= explicit;
            if !background && request.background {
                // The reader takes the background request over: its answer
                // is the page they are waiting for, and it no longer counts
                // as the background's.
                request.background = false;
                self.prefetch.promote(&chat);
            }
            return false;
        }
        if self.archive.history_start(&chat).unwrap_or(false) {
            // The phone already said it has nothing older.
            if !background {
                self.emit(Event::OlderFetched {
                    chat,
                    more: false,
                    silent: false,
                });
            }
            return false;
        }
        // Chats without messages request history from the current time.
        let (id, from_me, timestamp) = match self.archive.oldest(&chat) {
            Ok(Some(oldest)) => (oldest.id, oldest.from_me, oldest.timestamp),
            _ => (String::new(), false, crate::util::now()),
        };
        self.request_history(chat, id, from_me, timestamp, explicit, background)
    }

    fn reload_history(&mut self, chat: ChatId, message: String) {
        if let Some(request) = self.pending_older.get(&chat) {
            if !request.background {
                return;
            }
            // A background request is in the way of one the reader made.
            // It steps aside and its answer, if it comes, stays quiet.
            if let Some(request) = self.pending_older.remove(&chat) {
                self.prefetch.promote(&chat);
                self.remember_late(chat.clone(), request);
            }
        }
        let Ok(Some(anchor)) = self.archive.message(&chat, &message) else {
            self.emit(Event::OlderFetched {
                chat,
                more: true,
                silent: false,
            });
            self.emit(Event::Error(
                "This message is no longer in the archive".into(),
            ));
            return;
        };
        // This repairs gaps inside existing history, even when the phone has
        // already told us there is nothing before the start of the chat.
        self.older_warned.remove(&chat);
        self.request_history(
            chat,
            anchor.id,
            anchor.from_me,
            anchor.timestamp,
            true,
            false,
        );
    }

    fn request_history(
        &mut self,
        chat: ChatId,
        id: String,
        from_me: bool,
        timestamp: i64,
        explicit: bool,
        background: bool,
    ) -> bool {
        let (Some(client), Some(jid)) = (self.client.clone(), Self::jid_of(&chat)) else {
            // Offline requests retry after reconnection; the banner shows state.
            if !background {
                self.emit(Event::OlderFetched {
                    chat,
                    more: true,
                    silent: false,
                });
            }
            return false;
        };
        self.older_serial += 1;
        let serial = self.older_serial;
        self.pending_older.insert(
            chat.clone(),
            OlderRequest {
                asked: Instant::now(),
                before: (timestamp, id.clone()),
                explicit,
                background,
                serial,
                request: None,
            },
        );
        let commands = self.commands.clone();
        tokio::spawn(async move {
            match client
                // Despite its `Ms` name, the protocol field takes Unix seconds.
                // https://github.com/tulir/whatsmeow/commit/54650307d891f89ab346a57953d316106caee371
                .fetch_message_history(&jid, &id, from_me, timestamp, PHONE_BATCH)
                .await
            {
                Ok(request) => {
                    let _ = commands.send(Command::OlderSent {
                        chat,
                        serial,
                        request,
                    });
                }
                Err(error) => {
                    log::warn!("could not request older messages");
                    let _ = commands.send(Command::OlderFailed {
                        chat,
                        serial,
                        error: format!("Could not request older messages from your phone: {error}"),
                    });
                }
            }
        });
        true
    }

    /// Records the id a history request went out under, so its answer can be
    /// matched to it.
    fn older_sent(&mut self, chat: ChatId, serial: u64, request: String) {
        if let Some(pending) = self
            .pending_older
            .get_mut(&chat)
            .filter(|pending| pending.serial == serial)
        {
            pending.request = Some(request);
        } else if let Some(late) = self
            .late_older
            .iter_mut()
            .find(|late| late.serial == serial)
        {
            late.request = Some(request);
        }
    }

    /// A history request that could not be sent. Only the request it belongs
    /// to is finished: a newer request for the same chat keeps waiting.
    fn older_failed(&mut self, chat: ChatId, serial: u64, error: String) {
        if !self
            .pending_older
            .get(&chat)
            .is_some_and(|pending| pending.serial == serial)
        {
            // Its timeout has already been reported.
            self.late_older.retain(|late| late.serial != serial);
            return;
        }
        let Some(request) = self.pending_older.remove(&chat) else {
            return;
        };
        if request.background {
            self.prefetch.fail_history(&chat, Instant::now());
            self.emit(Event::OlderFetched {
                chat,
                more: true,
                silent: true,
            });
            return;
        }
        self.emit(Event::OlderFetched {
            chat: chat.clone(),
            more: true,
            silent: false,
        });
        // Automatic requests retry quietly, as their timeouts do.
        if request.explicit && self.older_warned.insert(chat) {
            self.emit(Event::Error(error));
        }
    }

    /// Fills the archive slowly: one phone-history request every twenty
    /// seconds at most, and only while no other history request is out, so a
    /// reader scrolling up does not run into the phone's rate limit. The
    /// chats to ask come from the queue `prefetch::State` keeps in memory.
    fn pump_prefetch(&mut self) {
        if !self.status.is_connected() || !self.prefetch.on {
            return;
        }
        if self.prefetch.needs_chats() {
            self.refresh_prefetch_queue();
        }
        let busy = !self.pending_older.is_empty() || self.poll_history.in_flight();
        let Some(chat) = self.prefetch.next_history(Instant::now(), busy) else {
            return;
        };
        if self.archive.history_start(&chat).unwrap_or(false) {
            // The phone has said this chat has nothing older.
            self.prefetch.exhaust(&chat);
            return;
        }
        if self.fetch_older(chat.clone(), false, true) {
            self.prefetch.start_history(chat);
        } else {
            self.prefetch.defer(&chat);
        }
    }

    /// Builds the prefetch queue from the archive's chat list, without
    /// sending that list to the interface again.
    fn refresh_prefetch_queue(&mut self) {
        match self.archive.chats() {
            Ok(mut chats) => {
                chats.retain(|chat| chat.last.is_some() || self.canonical_str(&chat.id) == chat.id);
                self.prefetch.set_chats(&chats);
            }
            Err(error) => log::warn!("could not list chats: {error}"),
        }
    }

    // --- commands --------------------------------------------------------

    /// Saves a validated contact through the current account and tags its asynchronous completion.
    fn save_contact(
        &mut self,
        id: String,
        full_name: String,
        first_name: Option<String>,
        to_phone: bool,
    ) {
        let (Some(client), Some(jid)) = (self.client.clone(), Self::jid_of(&id)) else {
            self.emit(Event::Error("Not connected to WhatsApp".to_owned()));
            return;
        };
        let commands = self.commands.clone();
        let generation = self.contact_generation;
        tokio::spawn(async move {
            let error = client
                .chat_actions()
                .save_contact(&jid, Some(full_name.clone()), first_name.clone(), to_phone)
                .await
                .err()
                .map(|error| error.to_string());
            let _ = commands.send(Command::ContactSaved {
                generation,
                id,
                name: full_name,
                first_name,
                error,
            });
        });
    }

    /// Applies a UI or task command after checking current send permissions.
    async fn handle_command(&mut self, command: Command) {
        let destination = match &command {
            Command::SendText { chat, .. }
            | Command::ReplyInteractive { chat, .. }
            | Command::SendVoice { chat, .. }
            | Command::SendFiles { chat, .. }
            | Command::SendImage { chat, .. }
            | Command::SendSticker { chat, .. }
            | Command::SendGif { chat, .. }
            | Command::SendLocation { chat, .. }
            | Command::CreatePoll { chat, .. } => Some(chat),
            // A batch goes to every recipient or to none of them.
            Command::Forward { to_chats, .. } => to_chats
                .iter()
                .find(|chat| !self.forward_writable(chat))
                .or_else(|| to_chats.first()),
            _ => None,
        };
        let writable = |chat: &str| match &command {
            Command::Forward { .. } => self.forward_writable(chat),
            _ => self.destination_writable(chat),
        };
        if let Some(chat) = destination
            && !writable(chat)
        {
            let error = "This conversation is read-only in ZapFast".to_owned();
            if matches!(&command, Command::CreatePoll { .. }) {
                self.emit(Event::PollCreated {
                    chat: chat.clone(),
                    error: Some(error),
                });
            } else {
                self.emit(Event::Error(error));
            }
            return;
        }
        match command {
            Command::StartCall {
                chat,
                video,
                generation,
                cancel,
            } => self.start_call(chat, video, generation, cancel).await,
            Command::JoinCallLink {
                token,
                video,
                generation,
                cancel,
            } => self.join_call_link(token, video, generation, cancel).await,
            Command::AnswerCall => self.answer_call().await,
            Command::DeclineCall => self.decline_call().await,
            Command::HangupCall => self.hangup_call().await,
            Command::SetCallMuted(muted) => self.set_call_muted(muted).await,
            Command::SetCallCamera(on) => self.set_call_camera(on).await,
            Command::SetCallMicrophone(device) => self.set_call_microphone(device),
            Command::SetCallSpeaker(device) => self.set_call_speaker(device),
            Command::SetCallCameraDevice(device) => self.set_call_camera_device(device).await,
            Command::RefreshCallDevices => {
                self.emit_call_devices().await;
            }
            Command::LoadCallLog => self.emit_call_log(),
            Command::RefreshPoll { chat, message } => self.refresh_poll(chat, message),
            Command::PollHistoryFailed {
                chat,
                message,
                requested,
            } => {
                self.poll_history
                    .fail(&chat, &message, requested, Instant::now());
                self.emit_message(&chat, &message);
                self.pump_poll_history();
            }
            Command::CreatePoll { chat, draft } => self.create_poll(chat, draft),
            Command::PollCreated {
                chat,
                draft,
                result,
            } => self.poll_created(chat, draft, result),
            Command::VotePoll {
                chat,
                message,
                choices,
            } => self.vote_poll(chat, message, choices),
            Command::PollVoted {
                chat,
                message,
                choices,
                at,
                result,
            } => self.poll_voted(chat, message, choices, at, result),
            Command::PollDecoded { vote, choices } => self.poll_decoded(vote, choices),
            Command::SendText {
                chat,
                text,
                quoting,
                mentions,
            } => self.send_text(chat, text, quoting, mentions),
            Command::SendLocation {
                chat,
                latitude,
                longitude,
                quoting,
            } => self.send_location(chat, latitude, longitude, quoting),
            Command::ReplyInteractive {
                chat,
                message,
                button,
                choice,
            } => {
                self.reply_interactive(chat, message, button, choice);
            }
            Command::Forward {
                from_chat,
                messages,
                to_chats,
            } => self.forward_to_chats(from_chat, messages, to_chats),
            Command::ForwardLimit {
                from_chat,
                messages,
            } => {
                let limit = self.forward_limit(&from_chat, &messages);
                self.emit(Event::ForwardLimit {
                    from_chat,
                    messages,
                    limit,
                });
            }
            // Stores the open chat's unsent text, or clears it when empty.
            Command::SaveDraft { chat, text } => {
                let at = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|elapsed| elapsed.as_secs() as i64)
                    .unwrap_or_default();
                if let Err(error) = self.archive.set_draft(&chat, &text, at) {
                    eprintln!("draft not stored: {error}");
                }
            }
            Command::Composing { chat, composing } => {
                let (Some(client), Some(jid)) = (self.client.clone(), Self::jid_of(&chat)) else {
                    return;
                };
                tokio::spawn(async move {
                    let result = if composing {
                        client.chatstate().send_composing(&jid).await
                    } else {
                        client.chatstate().send_paused(&jid).await
                    };
                    if let Err(error) = result {
                        log::debug!("chat state not sent: {error}");
                    }
                });
            }
            Command::MarkRead { chat, receipts } => self.mark_read(chat, receipts),
            Command::MarkUnread(chat) => match self.archive.mark_unread(&chat) {
                Ok(marked) => {
                    self.emit_chat(&chat);
                    if marked {
                        self.pump_read_sync();
                    }
                }
                Err(error) => self.emit(Event::Error(error.to_string())),
            },
            Command::WatchReceipts(watch) => {
                self.receipts_watch = watch;
                self.emit_receipts();
            }
            Command::ReadSyncFinished {
                chat,
                through,
                success,
            } => {
                if !self
                    .read_sync
                    .finish(&chat, through, success, Instant::now())
                {
                    return;
                }
                if success {
                    let _ = self.archive.finish_read_sync(&chat, through);
                    self.pump_read_sync();
                }
            }
            Command::UnreadSyncFinished {
                chat,
                marked_at,
                success,
            } => {
                if !self
                    .read_sync
                    .finish_unread(&chat, marked_at, success, Instant::now())
                {
                    return;
                }
                if success {
                    let _ = self.archive.finish_unread_sync(&chat, marked_at);
                    self.pump_read_sync();
                }
            }
            Command::LoadChat { chat, before } => self.load_chat(chat, before),
            Command::LoadPins(chat) => self.emit_pins(&chat),
            Command::FetchOlder { chat, explicit } => {
                let _ = self.fetch_older(chat, explicit, false);
            }
            Command::ReloadHistory { chat, message } => self.reload_history(chat, message),
            Command::SetHistoryPrefetch { on, focused } => {
                self.prefetch.configure(on, focused);
            }
            Command::LoadUntil { chat, id, before } => self.load_until(chat, id, before),
            Command::SearchMessages { query } => self.search_messages(query),
            Command::SearchChatMessages {
                chat,
                query,
                from,
                until,
            } => self.search_chat_messages(chat, query, from, until),
            Command::LoadStarred => self.emit_starred(),
            Command::SetStar {
                chat,
                message,
                starred,
            } => self.set_star(chat, message, starred),
            Command::Starred {
                chat,
                message,
                starred,
                generation,
                session,
                result,
            } => {
                if self.star_session != session
                    || self
                        .star_generation
                        .get(&(chat.clone(), message.clone()))
                        .copied()
                        != Some(generation)
                {
                    return;
                }
                if let Err(error) = &result {
                    self.emit(Event::Error(error.clone()));
                } else {
                    // Only a confirmed star reaches the archive, so the list
                    // never claims something WhatsApp refused. The time is the
                    // moment of this answer: an older phone event cannot undo it.
                    let written =
                        self.archive
                            .set_star(&chat, &message, starred, crate::util::now(), false);
                    match written {
                        Ok(true) => self.emit(Event::StarChanged {
                            chat,
                            message,
                            starred,
                            local: true,
                        }),
                        Ok(false) => {}
                        Err(error) => self.emit(Event::Error(error.to_string())),
                    }
                }
            }
            Command::ListChatShared { chat, request } => self.load_chat_media(chat, request),
            Command::EnsureChat { chat, name } => {
                let is_new = self.archive.chat(&chat).ok().flatten().is_none();
                if let Err(error) = self.archive.ensure_chat(&chat, &name) {
                    log::warn!("could not create the chat: {error}");
                } else if is_new
                    && ChatKind::from_id(&chat) == ChatKind::Direct
                    && let Some(expiration) = self.default_ephemeral_expiration()
                {
                    let timestamp = self
                        .archive
                        .meta("default_ephemeral_setting_timestamp")
                        .ok()
                        .flatten()
                        .and_then(|value| value.parse().ok())
                        .unwrap_or_default();
                    let _ = self.archive.set_ephemeral(&chat, expiration, timestamp);
                }
            }
            Command::Download {
                manual,
                card,
                chat,
                message,
            } => self.download_media(chat, message, card, manual),
            Command::FetchAvatar { id, full } => self.fetch_avatar(id, full),
            Command::LoadChatMedia { chat, around } => self.emit_chat_media(chat, around),
            Command::EditText {
                chat,
                id,
                text,
                mentions,
            } => self.edit_text(chat, id, text, mentions),
            Command::EditSettled {
                chat,
                id,
                generation,
                accepted,
            } => self.edit_settled(chat, id, generation, accepted),
            Command::Revoke { chat, id } => self.revoke(chat, id),
            Command::DeleteLocal { chat, id } => {
                self.delete_message_for_me(chat, id);
            }
            Command::MessageDeletedForMe {
                generation,
                chat,
                id,
                outcome,
            } => {
                if generation != self.privacy_generation {
                    return;
                }
                self.message_removals_in_flight
                    .remove(&(chat.clone(), id.clone()));
                match outcome {
                    MessageRemovalOutcome::Accepted => self.delete_message_here(&chat, &id),
                    MessageRemovalOutcome::Uncertain => self.emit(Event::Error(
                        "Could not confirm the deletion. It will be retried after reconnecting"
                            .into(),
                    )),
                }
            }
            Command::PickFiles { chat, documents } => {
                let commands = self.commands.clone();
                tokio::task::spawn_blocking(move || {
                    let paths = rfd::FileDialog::new()
                        .set_title("Send to WhatsApp")
                        .pick_files()
                        .unwrap_or_default();
                    let _ = commands.send(Command::Picked {
                        chat,
                        paths,
                        documents,
                    });
                });
            }
            Command::LoadStorageStats { token } => self.measure_storage(token, false),
            Command::ClearMediaCache { token } => self.measure_storage(token, true),
            Command::StorageMeasured {
                stats,
                token,
                cleared,
            } => {
                if cleared {
                    // The archive forgets the files that are gone, so they
                    // download again instead of drawing as broken.
                    self.relocate_media();
                }
                self.emit(Event::StorageStats {
                    stats,
                    token,
                    cleared,
                });
            }
            Command::Picked {
                chat,
                paths,
                documents,
            } => self.emit(Event::Picked {
                chat,
                paths,
                documents,
            }),
            Command::SendFiles {
                chat,
                paths,
                documents,
                view_once,
                caption,
                mentions,
                quoting,
            } => {
                self.send_files(
                    chat,
                    paths,
                    FileSendOptions {
                        documents,
                        view_once,
                    },
                    caption,
                    mentions,
                    quoting,
                );
            }
            Command::SendImage {
                chat,
                width,
                height,
                rgba,
                view_once,
                caption,
                mentions,
                quoting,
            } => self.send_pasted_image(
                chat, width, height, rgba, view_once, caption, mentions, quoting,
            ),
            Command::Outbound { chat, row, raw } => self.outbound(chat, *row, raw),
            Command::SendSticker {
                chat,
                path,
                quoting,
            } => self.send_sticker(chat, path, quoting),
            Command::SaveSticker { path } => self.favorite_sticker(&path),
            Command::RemoveRecentSticker { path } => self.remove_recent_sticker(&path),
            Command::ForgetSticker { path } => self.unfavorite_sticker(&path),
            Command::FavoritePushed {
                hash,
                updated_at,
                result,
            } => self.favorite_pushed(&hash, updated_at, result),
            Command::FavoritesPushed => self.favorites_pushed(),
            Command::FavoriteFetched { hash, result } => self.favorite_fetched(&hash, result),
            Command::FavoritesRecovered { complete } => self.favorites_recovered(complete),
            Command::FirstNamesRecovered { complete } => self.first_names_recovered(complete),
            Command::StarsRecovered { complete } => self.stars_recovered(complete),
            Command::ImportStickerUrl { url } => {
                let commands = self.commands.clone();
                let packs = self.packs_dir();
                tokio::task::spawn_blocking(move || {
                    let result = super::sticker_import::import_signal_pack(&url, &packs);
                    let _ = commands.send(Command::StickerPackImported { result });
                });
            }
            Command::SetProxy(setting) => {
                crate::proxy::configure(&setting);
                // Reconnect so the WhatsApp connection uses the new route.
                if self.handle.is_some() {
                    self.stop_bot().await;
                    self.start_bot().await;
                }
            }
            Command::SetDownloadFolder(folder) => {
                // Interrupted downloads leave hidden staging files behind.
                if let Some(folder) = &folder
                    && self.cleaned_download_folders.insert(folder.clone())
                {
                    discard_attachment_staging(folder);
                    download_folders::discard_staging(folder, &self.dirs.id, self.me_pn.as_deref());
                }
                self.download_folder = folder;
            }
            Command::SetKeepChatsArchived(keep) => self.keep_chats_archived = keep,
            Command::Resumed => self.resumed(),
            Command::SetChatSound { chat, sound } => {
                let _ = self.archive.set_notification_sound(&chat, sound.as_ref());
                self.emit_chat(&chat);
            }
            Command::SetChatVoiceSpeed { chat, speed } => {
                if let Err(error) = self.archive.set_voice_speed(&chat, speed) {
                    log::warn!("could not store the chat's voice speed: {error}");
                }
                self.emit_chat(&chat);
            }
            Command::PickChatSound(chat) => {
                let events = self.events.clone();
                let waker = self.waker.clone();
                tokio::task::spawn_blocking(move || {
                    if let Some(path) = rfd::FileDialog::new()
                        .set_title("Choose a notification sound")
                        .add_filter("Audio", &["wav", "mp3", "ogg", "oga"])
                        .pick_file()
                    {
                        let _ = events.send(Event::ChatSoundPicked { chat, path });
                        waker.wake();
                    }
                });
            }
            Command::PickDownloadFolder => {
                let events = self.events.clone();
                let waker = self.waker.clone();
                tokio::task::spawn_blocking(move || {
                    if let Some(path) = rfd::FileDialog::new()
                        .set_title("Choose a folder for downloads")
                        .pick_folder()
                    {
                        let _ = events.send(Event::DownloadFolderPicked(path));
                        waker.wake();
                    }
                });
            }
            Command::PickWallpaperImage => {
                let dirs = self.dirs.clone();
                let events = self.events.clone();
                let waker = self.waker.clone();
                tokio::task::spawn_blocking(move || {
                    let Some(path) = rfd::FileDialog::new()
                        .set_title("Choose a wallpaper image")
                        .add_filter("Images", &["jpg", "jpeg", "png", "webp", "gif"])
                        .pick_file()
                    else {
                        return;
                    };
                    let result = crate::wallpaper::import(&path, &dirs.state);
                    let _ = events.send(Event::WallpaperImagePicked(result));
                    waker.wake();
                });
            }
            Command::RemoveWallpaperImage => crate::wallpaper::remove(&self.dirs.state),
            Command::SetProfile { name, about } => self.set_profile(name, about),
            Command::PickProfilePicture => {
                let commands = self.commands.clone();
                let events = self.events.clone();
                let waker = self.waker.clone();
                tokio::task::spawn_blocking(move || {
                    let Some(path) = rfd::FileDialog::new()
                        .set_title("Choose a profile picture")
                        .add_filter("Images", &["jpg", "jpeg", "png", "webp", "gif"])
                        .pick_file()
                    else {
                        return;
                    };
                    match profile_picture_jpeg(&path) {
                        Ok(bytes) => {
                            let _ = commands.send(Command::SetProfilePicture(bytes));
                        }
                        Err(error) => {
                            let _ = events
                                .send(Event::Error(format!("Could not use this picture: {error}")));
                        }
                    }
                    waker.wake();
                });
            }
            Command::SetProfilePicture(bytes) => {
                let Some(client) = self.client.clone() else {
                    self.emit(Event::Error(
                        "Connect to WhatsApp to change your profile picture.".to_owned(),
                    ));
                    return;
                };
                let commands = self.commands.clone();
                let events = self.events.clone();
                let waker = self.waker.clone();
                tokio::spawn(async move {
                    match client.profile().set_profile_picture(bytes).await {
                        Ok(_) => {
                            let _ = commands.send(Command::ProfileSaved {
                                name: None,
                                about: None,
                                picture: true,
                            });
                        }
                        Err(error) => {
                            let _ = events.send(Event::Error(format!(
                                "Could not change your profile picture: {error}"
                            )));
                        }
                    }
                    waker.wake();
                });
            }
            Command::SetGroupName { chat, name } => self.set_group_name(chat, name),
            Command::GroupInviteLink { chat, reset } => self.group_invite_link(chat, reset),
            Command::AddGroupMembers { chat, members } => self.add_group_members(chat, members),
            Command::SetGroupPermission {
                chat,
                permission,
                members,
            } => self.set_group_permission(chat, permission, members),
            Command::ChangeGroupMember {
                chat,
                member,
                change,
            } => self.change_group_member(chat, member, change),
            Command::GroupManaged { chat, answer } => self.group_managed(chat, answer),
            Command::PickGroupPicture(chat) => {
                if !self.may_edit_group(&chat) {
                    return;
                }
                let commands = self.commands.clone();
                let events = self.events.clone();
                let waker = self.waker.clone();
                tokio::task::spawn_blocking(move || {
                    let Some(path) = rfd::FileDialog::new()
                        .set_title("Choose a group photo")
                        .add_filter("Images", &["jpg", "jpeg", "png", "webp", "gif"])
                        .pick_file()
                    else {
                        return;
                    };
                    match profile_picture_jpeg(&path) {
                        Ok(jpeg) => {
                            let _ = commands.send(Command::SetGroupPicture {
                                chat,
                                jpeg: Some(jpeg),
                            });
                        }
                        Err(error) => {
                            let _ = events
                                .send(Event::Error(format!("Could not use this picture: {error}")));
                        }
                    }
                    waker.wake();
                });
            }
            Command::SetGroupPicture { chat, jpeg } => self.set_group_picture(chat, jpeg),
            Command::PickNewGroupPicture => self.pick_new_group_picture(),
            Command::CreateGroup {
                request,
                subject,
                members,
                picture,
            } => self.create_group(request, subject, members, picture),
            Command::GroupCreated {
                request,
                result,
                picture,
            } => self.group_created(request, result, picture),
            Command::GroupEdited { chat, edit, result } => self.group_edited(chat, edit, result),
            Command::ProfileSaved {
                name,
                about,
                picture,
            } => {
                if let Some(name) = name {
                    let _ = self.archive.set_meta("me_name", &name);
                    self.me_name = Some(name);
                }
                if let Some(about) = about {
                    let _ = self.archive.set_meta("me_about", &about);
                    self.me_about = Some(about).filter(|about| !about.is_empty());
                }
                if picture {
                    let me = self.me();
                    let _ = std::fs::remove_file(self.avatar_file(&me, false));
                    let _ = std::fs::remove_file(self.avatar_file(&me, true));
                    self.fetch_avatar(me.clone(), false);
                    self.fetch_avatar(me, true);
                }
                self.emit(self.me_event());
            }
            Command::PickNotificationSound { mention } => {
                let events = self.events.clone();
                let waker = self.waker.clone();
                tokio::task::spawn_blocking(move || {
                    if let Some(path) = rfd::FileDialog::new()
                        .set_title("Choose a notification sound")
                        .add_filter("Audio", &["wav", "mp3", "ogg", "oga"])
                        .pick_file()
                    {
                        let _ = events.send(Event::NotificationSoundPicked { mention, path });
                        waker.wake();
                    }
                });
            }
            Command::SaveAttachmentAs { source, name } => {
                let events = self.events.clone();
                let waker = self.waker.clone();
                tokio::task::spawn_blocking(move || {
                    let mut dialog = rfd::FileDialog::new()
                        .set_title("Save attachment")
                        .set_file_name(&name);
                    if let Some(downloads) = directories::UserDirs::new()
                        .and_then(|dirs| dirs.download_dir().map(Path::to_path_buf))
                    {
                        dialog = dialog.set_directory(downloads);
                    }
                    // Cancelling the dialog saves nothing and says nothing.
                    let Some(target) = dialog.save_file() else {
                        return;
                    };
                    let event = match std::fs::copy(&source, &target) {
                        Ok(_) => Event::Info(format!(
                            "Saved {}",
                            target.file_name().map_or_else(
                                || name.clone(),
                                |name| name.to_string_lossy().into_owned()
                            )
                        )),
                        Err(error) => {
                            Event::Error(format!("Could not save the attachment: {error}"))
                        }
                    };
                    let _ = events.send(event);
                    waker.wake();
                });
            }
            Command::OpenLog(path) => {
                let events = self.events.clone();
                let waker = self.waker.clone();
                tokio::task::spawn_blocking(move || {
                    if let Err(error) = crate::opener::open_or_reveal(&path) {
                        let _ = events.send(Event::Error(error));
                        waker.wake();
                    }
                });
            }
            Command::SaveAttachments { files } => {
                let events = self.events.clone();
                let waker = self.waker.clone();
                tokio::task::spawn_blocking(move || {
                    let mut dialog = rfd::FileDialog::new().set_title("Save album");
                    if let Some(downloads) = directories::UserDirs::new()
                        .and_then(|dirs| dirs.download_dir().map(Path::to_path_buf))
                    {
                        dialog = dialog.set_directory(downloads);
                    }
                    let Some(folder) = dialog.pick_folder() else {
                        return;
                    };
                    let mut saved = 0usize;
                    let mut failed = Vec::new();
                    for (source, name) in files {
                        match save_album_copy(&source, &folder, &name) {
                            Ok(_) => saved += 1,
                            Err(error) => failed.push(format!("{name}: {error}")),
                        }
                    }
                    let event = if failed.is_empty() {
                        Event::Info(format!("Saved {saved} album attachments"))
                    } else {
                        Event::Error(format!(
                            "Saved {saved} album attachments; {} could not be saved: {}",
                            failed.len(),
                            failed.join("; ")
                        ))
                    };
                    let _ = events.send(event);
                    waker.wake();
                });
            }
            Command::PrepareClipboardImage(path) => {
                let events = self.events.clone();
                let waker = self.waker.clone();
                tokio::task::spawn_blocking(move || {
                    let result = image::open(&path)
                        .map_err(|error| error.to_string())
                        .map(|img| {
                            let rgba = img.to_rgba8();
                            let (width, height) = rgba.dimensions();
                            crate::model::DecodedImage {
                                width: width as usize,
                                height: height as usize,
                                bytes: rgba.into_raw(),
                            }
                        });
                    let _ = events.send(Event::ClipboardImage(result));
                    waker.wake();
                });
            }
            Command::PickStickerPicture => {
                let commands = self.commands.clone();
                tokio::task::spawn_blocking(move || {
                    let result = match rfd::FileDialog::new()
                        .set_title("Make a sticker")
                        .add_filter("Pictures", &["png", "jpg", "jpeg", "webp", "gif"])
                        .pick_file()
                    {
                        Some(path) => std::fs::read(&path)
                            .map_err(|error| error.to_string())
                            .and_then(|bytes| super::sticker_maker::inspect(&bytes))
                            .map(|(width, height, transparent)| (path, width, height, transparent)),
                        // Ignore file-picker cancellation.
                        None => Err(String::new()),
                    };
                    let _ = commands.send(Command::StickerPicturePicked { result });
                });
            }
            Command::StickerPicturePicked { result } => match result {
                Ok((path, width, height, transparent)) => self.emit(Event::StickerPicture {
                    path,
                    width,
                    height,
                    transparent,
                }),
                Err(error) if error.is_empty() => {}
                Err(error) => self.emit(Event::Error(error)),
            },
            Command::MakeSticker {
                source,
                crop,
                transparent,
                emojis,
                chat,
                quoting,
            } => {
                let commands = self.commands.clone();
                let dir = self.dirs.sticker_cache_dir().join("made");
                tokio::task::spawn_blocking(move || {
                    let result = std::fs::read(&source)
                        .map_err(|error| error.to_string())
                        .and_then(|bytes| {
                            super::sticker_maker::make(&bytes, crop, transparent, &emojis)
                        })
                        .and_then(|sticker| {
                            std::fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
                            let hash = super::sticker_store::content_hash(&sticker);
                            let path = dir.join(format!("{hash}.webp"));
                            std::fs::write(&path, sticker).map_err(|error| error.to_string())?;
                            Ok(path)
                        });
                    let _ = commands.send(Command::StickerMade {
                        result,
                        chat,
                        quoting,
                    });
                });
            }
            Command::StickerMade {
                result,
                chat,
                quoting,
            } => match (result, chat) {
                (Ok(path), Some(chat)) => self.send_sticker(chat, path, quoting),
                (Ok(path), None) => {
                    self.favorite_sticker(&path);
                    self.emit(Event::Info("Added to favorites".to_owned()));
                }
                (Err(error), _) => {
                    self.emit(Event::Error(format!("Could not make the sticker: {error}")))
                }
            },
            Command::InspectPicture { target, path } => {
                let commands = self.commands.clone();
                tokio::task::spawn_blocking(move || {
                    let result = std::fs::read(&path)
                        .map_err(|error| error.to_string())
                        .and_then(|bytes| super::picture_edit::inspect(&bytes));
                    let _ = commands.send(Command::PictureInspected { target, result });
                });
            }
            Command::PictureInspected { target, result } => {
                self.emit(Event::PictureInspected { target, result });
            }
            Command::ApplyPictureEdit {
                target,
                source,
                width,
                height,
                crop,
                turns,
            } => {
                let commands = self.commands.clone();
                let dir = self.dirs.media_cache_dir().join("edited");
                tokio::task::spawn_blocking(move || {
                    let result =
                        super::picture_edit::write(&source, width, height, crop, turns, &dir);
                    let _ = commands.send(Command::PictureEdited { target, result });
                });
            }
            Command::PictureEdited { target, result } => {
                self.emit(Event::PictureEdited { target, result });
            }
            Command::PickStickerArchive => {
                let commands = self.commands.clone();
                let packs = self.packs_dir();
                tokio::task::spawn_blocking(move || {
                    let result = match rfd::FileDialog::new()
                        .set_title("Add a sticker pack")
                        .add_filter("Sticker packs", &["wastickers", "zip"])
                        .pick_file()
                    {
                        Some(path) => super::sticker_import::import_archive(&path, &packs),
                        // Ignore file-picker cancellation.
                        None => Err(String::new()),
                    };
                    let _ = commands.send(Command::StickerPackImported { result });
                });
            }
            Command::SaveContact {
                id,
                full_name,
                first_name,
                to_phone,
            } => {
                self.save_contact(id, full_name, first_name, to_phone);
            }
            Command::ContactSaved {
                generation,
                id,
                name,
                first_name,
                error,
            } => {
                if generation != self.contact_generation {
                    return;
                }
                if let Some(error) = error {
                    self.emit(Event::Error(format!("Could not save contact: {error}")));
                    return;
                }
                let contact = Contact {
                    id: id.clone(),
                    full_name: Some(name.clone()),
                    first_name: first_name.filter(|first| !first.is_empty()),
                    push_name: None,
                };
                if let Err(error) = self.archive.upsert_contact(&contact) {
                    log::warn!("could not store the contact: {error}");
                }
                // Preserve the stored push name during contact updates.
                let stored = self.archive.contact(&id).ok().flatten().unwrap_or(contact);
                self.emit(Event::Contacts(vec![stored]));
                self.emit(Event::Info(format!("Added {name} to contacts")));
                self.emit_chat(&id);
            }
            Command::NewContact {
                request,
                phone,
                full_name,
                first_name,
                to_phone,
            } => {
                let Some(phone) = crate::util::international_phone(&phone) else {
                    self.emit(Event::ContactFailed {
                        request,
                        error: "Enter a valid international phone number".to_owned(),
                    });
                    return;
                };
                let Some(client) = self.client.clone() else {
                    self.emit(Event::ContactFailed {
                        request,
                        error: "Not connected to WhatsApp".to_owned(),
                    });
                    return;
                };
                let commands = self.commands.clone();
                let jid = Jid::pn(&phone);
                let generation = self.contact_generation;
                tokio::spawn(async move {
                    // Use WhatsApp's registration check before opening the chat.
                    let registered = client
                        .contacts()
                        .is_on_whatsapp(&[jid])
                        .await
                        .map(|results| results.iter().any(|result| result.is_registered))
                        .map_err(|_| "Could not check the number. Please try again.".to_owned());
                    let _ = commands.send(Command::ContactChecked {
                        generation,
                        request,
                        phone,
                        full_name,
                        first_name,
                        to_phone,
                        registered,
                    });
                });
            }
            Command::ContactChecked {
                generation,
                request,
                phone,
                full_name,
                first_name,
                to_phone,
                registered,
            } => {
                if generation != self.contact_generation {
                    return;
                }
                match registered {
                    Ok(true) => {}
                    Ok(false) => {
                        self.emit(Event::ContactFailed {
                            request,
                            error: "This number is not on WhatsApp".to_owned(),
                        });
                        return;
                    }
                    Err(error) => {
                        self.emit(Event::ContactFailed { request, error });
                        return;
                    }
                }
                let id = format!("{phone}@s.whatsapp.net");
                if let Some(full_name) = full_name.clone() {
                    self.save_contact(id.clone(), full_name, first_name, to_phone);
                }
                self.emit(Event::ContactReady {
                    request,
                    id,
                    name: full_name,
                });
            }
            Command::StickerPackImported { result } => match result {
                Ok(name) => {
                    self.emit_stickers();
                    self.emit(Event::Info(format!("Added sticker pack \"{name}\"")));
                }
                Err(error) if error.is_empty() => self.emit_stickers(),
                Err(error) => {
                    self.emit(Event::Error(format!("Could not add sticker pack: {error}")))
                }
            },
            Command::DeleteStickerPack { dir } => {
                let root = self.packs_dir();
                if dir.starts_with(&root) && dir != root && std::fs::remove_dir_all(&dir).is_ok() {
                    self.emit_stickers();
                }
            }
            Command::SendVoice {
                chat,
                samples,
                quoting,
            } => self.send_voice(chat, samples, quoting),
            Command::MarkPlayed {
                chat,
                message,
                sender,
                receipts,
            } => {
                self.store_played(&chat, &message);
                if receipts {
                    self.mark_played(chat, message, sender);
                }
            }
            Command::SendGif { chat, gif, quoting } => self.send_gif(chat, gif, quoting),
            Command::SearchGifs { query, key } => {
                let commands = self.commands.clone();
                let dir = self.dirs.cache.join("gifs");
                tokio::task::spawn_blocking(move || {
                    let results = search_gifs(&query, &key, &dir);
                    let _ = commands.send(Command::GifResults { query, results });
                });
            }
            Command::ReceiptsPrivacy { disabled } => {
                self.emit(Event::ReceiptsPrivacy { disabled });
            }
            Command::AccountPrivacy { values, failed } => {
                self.emit(Event::AccountPrivacy { values, failed });
            }
            Command::FetchAccountPrivacy => {
                if let Some(client) = self.client.clone() {
                    let commands = self.commands.clone();
                    tokio::spawn(async move {
                        publish_account_privacy(&client, &commands).await;
                    });
                }
            }
            Command::SetAccountPrivacy { kind, choice } => self.set_account_privacy(kind, choice),
            Command::AccountPrivacySaved { kind } => {
                self.emit(Event::AccountPrivacySaved { kind });
            }
            Command::AccountPrivacyFailed { kind } => {
                self.emit(Event::AccountPrivacyFailed { kind });
                self.emit(Event::Error("Could not update privacy settings.".into()));
            }
            Command::SetOnline(online) => self.set_online(online),
            // Only meaningful while the archive cannot be opened.
            Command::StartOverArchive => {}
            Command::PreviewInvite(code) => {
                let Some(client) = self.client.clone() else {
                    self.emit(Event::InvitePreview {
                        code,
                        result: Err("ZapFast is not connected to WhatsApp".to_owned()),
                    });
                    return;
                };
                let events = self.events.clone();
                let waker = self.waker.clone();
                tokio::spawn(async move {
                    let result = client
                        .groups()
                        .get_invite_info(&code)
                        .await
                        .map(|group| crate::model::InviteInfo {
                            id: group.id.to_string(),
                            subject: group.subject.unwrap_or_default(),
                            description: group.description.filter(|text| !text.trim().is_empty()),
                            members: group
                                .participant_count
                                .map_or(group.participants.len(), |size| size as usize),
                            approval: group.membership_approval,
                        })
                        .map_err(|error| invite_error(&error.to_string()));
                    let _ = events.send(Event::InvitePreview { code, result });
                    waker.wake();
                });
            }
            Command::JoinInvite(code) => {
                let Some(client) = self.client.clone() else {
                    self.emit(Event::InviteJoined {
                        code,
                        result: Err("ZapFast is not connected to WhatsApp".to_owned()),
                    });
                    return;
                };
                let commands = self.commands.clone();
                tokio::spawn(async move {
                    use whatsapp_rust::JoinGroupResult;
                    let result = match client.groups().join_with_invite_code(&code).await {
                        Ok(JoinGroupResult::Joined(jid)) => Ok((jid.to_string(), false)),
                        Ok(JoinGroupResult::PendingApproval(jid)) => Ok((jid.to_string(), true)),
                        Err(error) => Err(invite_error(&error.to_string())),
                    };
                    let _ = commands.send(Command::InviteJoined { code, result });
                });
            }
            Command::InviteJoined { code, result } => {
                let result = result.map(|(id, pending)| {
                    if !pending {
                        self.ensure_chat(&id, None);
                        self.request_group_info(&id, true);
                        self.emit_chat(&id);
                    }
                    (id, pending)
                });
                self.emit(Event::InviteJoined { code, result });
            }
            Command::InspectUpdate => {
                let events = self.events.clone();
                let waker = self.waker.clone();
                tokio::task::spawn_blocking(move || {
                    let result = crate::updates::updater()
                        .map_err(|error| format!("{error:#}"))
                        .and_then(|updater| {
                            updater.installation().map_err(|reason| reason.to_string())
                        });
                    let _ = events.send(Event::UpdateSupport(result));
                    waker.wake();
                });
            }
            Command::DownloadUpdate { release, source } => {
                let events = self.events.clone();
                let waker = self.waker.clone();
                tokio::task::spawn_blocking(move || {
                    let result = crate::updates::updater()
                        .and_then(|updater| {
                            updater
                                .with_source(source)
                                .download(&release, |received, total| {
                                    let _ = events.send(Event::UpdateProgress { received, total });
                                    waker.wake();
                                })
                        })
                        .map(Box::new)
                        .map_err(|error| format!("{error:#}"));
                    let _ = events.send(Event::UpdateDownloaded(result));
                    waker.wake();
                });
            }
            Command::InstallUpdate {
                prepared,
                arguments,
            } => {
                let events = self.events.clone();
                let waker = self.waker.clone();
                tokio::task::spawn_blocking(move || {
                    let result = crate::updates::updater()
                        .and_then(|updater| updater.handoff(*prepared, arguments))
                        .map_err(|error| format!("{error:#}"));
                    let _ = events.send(Event::UpdateInstalling(result));
                    waker.wake();
                });
            }
            Command::CheckForUpdates => {
                let events = self.events.clone();
                let waker = self.waker.clone();
                tokio::task::spawn_blocking(move || {
                    match crate::updates::updater().and_then(|updater| updater.check()) {
                        Ok(Some(release)) => {
                            let _ = events.send(Event::UpdateAvailable {
                                version: release.version,
                                url: release.url,
                            });
                            waker.wake();
                        }
                        Ok(None) => log::debug!("this is the newest release"),
                        Err(error) => {
                            log::debug!("could not check for a newer release: {error:#}")
                        }
                    }
                });
            }
            Command::GifResults { query, results } => {
                self.emit(Event::Gifs { query, results });
            }
            Command::ViewStickerPack { chat, message } => self.view_sticker_pack(&chat, &message),
            Command::StickerPackViewed { result } => {
                self.emit(Event::StickerPackPreview(result));
            }
            Command::AddStickerPack { dir, name } => self.add_sticker_pack(&dir, &name),
            Command::SendStickerPack { chat, dir } => self.send_sticker_pack(chat, dir),
            Command::CreateStickerPack { name } => {
                match super::sticker_store::create_local_pack(
                    &self.packs_dir(),
                    &name,
                    crate::util::now(),
                ) {
                    Ok(_) => self.emit_stickers(),
                    Err(error) => {
                        self.emit(Event::Error(format!("Could not create the pack: {error}")))
                    }
                }
            }
            Command::SetStickerPack {
                pack,
                sticker,
                member,
            } => {
                // Restrict changes to folders in the pack directory.
                let root = self.packs_dir();
                if pack.starts_with(&root) && pack != root {
                    if let Err(error) = super::sticker_store::set_member(&pack, &sticker, member) {
                        log::warn!("could not file a sticker in its pack: {error}");
                    }
                    self.emit_stickers();
                }
            }
            Command::RecentStickers => {
                self.fetch_missing_stickers();
                self.emit_stickers();
            }
            Command::StickerFetched { hash, result } => {
                self.sticker_fetches.remove(&hash);
                match &result {
                    Ok(path) => {
                        if self.archive.set_sticker_path(&hash, path).is_err() {
                            log::warn!("could not file a sticker");
                        }
                    }
                    Err(error) if sticker_pace::rate_limited(error) => {
                        log::warn!("sticker downloads paused: the server asked to slow down");
                        self.sticker_pace.limited(Instant::now());
                        self.sticker_resume = true;
                    }
                    Err(_error) => {
                        log::warn!("could not fetch a sticker");
                        self.sticker_failed.insert(hash);
                    }
                }
                let arrived = result.is_ok();
                // The next missing ones take the freed places.
                self.fetch_missing_stickers();
                self.picker_sticker_settled(arrived);
            }
            Command::MeInfo { about } => {
                self.me_about = about;
                match &self.me_about {
                    Some(about) => {
                        let _ = self.archive.set_meta("me_about", about);
                    }
                    None => {
                        let _ = self.archive.set_meta("me_about", "");
                    }
                }
                self.emit(self.me_event());
            }
            Command::React {
                chat,
                message,
                emoji,
            } => self.react(chat, message, emoji),
            Command::LeaveGroup { chat, archive } => {
                self.leave_group(chat, archive).await;
            }
            Command::SetArchived(chat, archived) => {
                let _ = self.archive.set_archived(&chat, archived);
                self.emit_chat(&chat);
                self.archive_on_phone(&chat, archived);
            }
            Command::DeleteChat(chat) => {
                // The phone deletes first. Deleting here while offline would
                // leave the chat on the phone, and the next sync would bring
                // it back despite the dialog saying it was deleted there too.
                let (Some(client), Some(jid)) = (self.client.clone(), Self::jid_of(&chat)) else {
                    self.emit(Event::Error(
                        "Connect to WhatsApp to delete this chat".to_owned(),
                    ));
                    return;
                };
                let through = self
                    .archive
                    .messages(&chat, None, 1)
                    .ok()
                    .and_then(|page| page.last().map(|message| message.timestamp))
                    .unwrap_or_else(crate::util::now);
                let commands = self.commands.clone();
                tokio::spawn(async move {
                    let deleted = client
                        .chat_actions()
                        .delete_chat(
                            &jid,
                            true,
                            Some(whatsapp_rust::message_range(through, None, Vec::new())),
                        )
                        .await
                        .is_ok();
                    let _ = commands.send(Command::ChatDeleted {
                        chat,
                        deleted,
                        through,
                    });
                });
            }
            Command::ChatDeleted {
                chat,
                deleted,
                through,
            } => {
                if deleted {
                    self.remove_chat(&chat, through, true);
                } else {
                    log::warn!("the phone did not delete a chat");
                    self.emit(Event::Error(
                        "The phone did not delete this chat. Try again when connected".to_owned(),
                    ));
                }
            }
            Command::ClearChat(chat) => {
                // The phone clears first, for the same reason it deletes
                // first: clearing here while offline would leave the messages
                // on the phone, and the next sync would bring them back
                // despite the dialog saying they were cleared there too.
                let (Some(client), Some(jid)) = (self.client.clone(), Self::jid_of(&chat)) else {
                    self.emit(Event::Error(
                        "Connect to WhatsApp to clear this chat".to_owned(),
                    ));
                    return;
                };
                let Some(through) = clear_boundary(self.archive.messages(&chat, None, 1)) else {
                    // Without the boundary this archive would be cleared through
                    // a time it never agreed to, and the two sides would drift
                    // while the dialog said they matched. Nothing is cleared
                    // anywhere.
                    log::warn!("could not read the boundary of a chat to clear");
                    self.emit(Event::Error(
                        "Could not read this chat's messages. Try again".to_owned(),
                    ));
                    return;
                };
                let commands = self.commands.clone();
                tokio::spawn(async move {
                    let cleared = client
                        .chat_actions()
                        .clear_chat(
                            &jid,
                            true,
                            true,
                            Some(whatsapp_rust::message_range(through, None, Vec::new())),
                        )
                        .await
                        .is_ok();
                    let _ = commands.send(Command::ChatCleared {
                        chat,
                        cleared,
                        through,
                    });
                });
            }
            Command::ChatCleared {
                chat,
                cleared,
                through,
            } => {
                if cleared {
                    if !self.empty_chat(&chat, through, true) {
                        // The phone has cleared it; say so rather than leave
                        // the messages here looking as if nothing happened.
                        self.emit(Event::Error(
                            "The phone cleared this chat, but ZapFast could not clear it here"
                                .to_owned(),
                        ));
                    }
                } else {
                    log::warn!("the phone did not clear a chat");
                    self.emit(Event::Error(
                        "The phone did not clear this chat. Try again when connected".to_owned(),
                    ));
                }
            }
            Command::SetPinned(chat, pinned) => {
                let _ = self.archive.set_pinned(&chat, pinned);
                self.emit_chat(&chat);
                self.tell_phone(&chat, move |client, jid| async move {
                    if pinned {
                        client.chat_actions().pin_chat(&jid).await
                    } else {
                        client.chat_actions().unpin_chat(&jid).await
                    }
                    .map_err(|error| error.to_string())
                });
            }
            Command::SetMessagePinned {
                chat,
                message,
                pinned,
            } => self.set_pin(chat, message, pinned),
            Command::MessagePinned {
                chat,
                message,
                pinned,
                at,
                generation,
                session,
                expires_at,
                result,
            } => {
                if self.star_session != session
                    || self
                        .pin_generation
                        .get(&(chat.clone(), message.clone()))
                        .copied()
                        != Some(generation)
                {
                    return;
                }
                if let Err(error) = &result {
                    self.emit(Event::Error(error.clone()));
                } else {
                    let written = if pinned {
                        self.archive.pin(&chat, &message, at, expires_at)
                    } else {
                        self.archive.unpin(&chat, &message, at)
                    };
                    match written {
                        Ok(true) => {
                            self.emit(Event::PinChanged {
                                chat: chat.clone(),
                                message,
                                pinned,
                            });
                            self.emit_pins(&chat);
                        }
                        Ok(false) => {}
                        Err(error) => self.emit(Event::Error(error.to_string())),
                    }
                }
            }
            Command::ChannelMutes(mutes) => {
                for (chat, muted) in mutes {
                    let Ok(Some(known)) = self.archive.chat(&chat) else {
                        continue;
                    };
                    // Mirror the phone's channel Mute without echoing it back.
                    if muted != known.muted(crate::util::now()) {
                        let _ = self.archive.set_muted(&chat, muted.then_some(0));
                        self.emit_chat(&chat);
                    }
                }
            }
            Command::ChannelNames { generation, names } => {
                if generation != self.channel_list_generation {
                    return;
                }
                self.drop_unfollowed_empty_channels(&names);
                for (chat, name) in names {
                    if name.trim().is_empty() {
                        continue;
                    }
                    let Ok(Some(known)) = self.archive.chat(&chat) else {
                        continue;
                    };
                    if name != known.name {
                        let _ = self.archive.rename_chat(&chat, &name);
                        self.emit_chat(&chat);
                    }
                }
            }
            Command::ChannelPictures(list) => self.channel_pictures_listed(list),
            Command::SetFavorite(chat, favorite) => self.set_favorite_chat(&chat, favorite),
            Command::FavoritesSent {
                through,
                at,
                success,
            } => self.favorites_sent(through, at, success),
            Command::SetMuted(chat, until) => {
                let _ = self.archive.set_muted(&chat, until);
                self.emit_chat(&chat);
                let channel = chat.ends_with("@newsletter");
                self.tell_phone(&chat, move |client, jid| async move {
                    // A channel also keeps its own mute on the server, which
                    // WhatsApp Web toggles as the channel's Mute.
                    if channel
                        && let Err(error) = client
                            .newsletter()
                            .set_admin_mute(&jid, until.is_some())
                            .await
                    {
                        log::debug!("channel mute not sent: {error}");
                    }
                    match until {
                        None => client.chat_actions().unmute_chat(&jid).await,
                        Some(0) => client.chat_actions().mute_chat(&jid).await,
                        Some(seconds) => {
                            client
                                .chat_actions()
                                .mute_chat_until(&jid, seconds * 1000)
                                .await
                        }
                    }
                    .map_err(|error| error.to_string())
                });
            }
            Command::CreateLabel { name, color_hex } => self.create_label(name, color_hex),
            Command::UpdateLabel {
                id,
                name,
                color_hex,
            } => match self.archive.update_label(&id, &name, &color_hex) {
                Ok(true) => self.emit_labels(),
                Ok(false) => log::info!("label not updated: gone, empty, or taken"),
                Err(error) => log::warn!("could not update label: {error}"),
            },
            Command::DeleteLabel(id) => match self.archive.delete_label(&id) {
                Ok(true) => self.emit_labels(),
                Ok(false) => {}
                Err(error) => log::warn!("could not delete label: {error}"),
            },
            Command::SetChatLabels { chat, labels } => {
                if let Err(error) = self.archive.set_chat_labels(&chat, &labels) {
                    log::warn!("could not assign labels: {error}");
                }
                self.emit_chat(&chat);
            }
            Command::SetLocked(chat, locked) => {
                let _ = self.archive.set_locked(&chat, locked);
                self.emit_chat(&chat);
                self.tell_phone(&chat, move |client, jid| async move {
                    if locked {
                        client.chat_actions().lock_chat(&jid).await
                    } else {
                        client.chat_actions().unlock_chat(&jid).await
                    }
                    .map_err(|error| error.to_string())
                });
            }
            Command::PairWithPhone(phone) => {
                let Some(client) = self.client.clone() else {
                    self.emit(Event::Error("Not connected to WhatsApp yet".to_owned()));
                    return;
                };
                self.pairing_phone = Some(phone.clone());
                self.pair_code = None;
                let status = self.unlinked();
                self.set_status(status);
                let commands = self.commands.clone();
                tokio::spawn(async move {
                    let result = client
                        .pair_with_code(PairCodeOptions {
                            phone_number: phone,
                            ..Default::default()
                        })
                        .await
                        .map_err(|error| error.to_string());
                    let _ = commands.send(Command::PairCode { result });
                });
            }
            Command::PairCode { result } => match result {
                Ok(code) => {
                    self.pair_code = Some(code);
                    let status = self.unlinked();
                    self.set_status(status);
                }
                Err(error) => {
                    self.pairing_phone = None;
                    self.emit(Event::Error(format!(
                        "Could not link by phone number: {error}"
                    )));
                    let status = self.unlinked();
                    self.set_status(status);
                }
            },
            Command::Unlink => {
                if let Some(client) = self.client.clone() {
                    client.logout().await;
                } else {
                    self.on_logged_out().await;
                }
            }
            Command::RemoveAccount => {
                // A link that does not answer must not keep the account:
                // the folders go once the backend stops either way.
                if let Some(client) = self.client.clone()
                    && tokio::time::timeout(Duration::from_secs(15), client.logout())
                        .await
                        .is_err()
                {
                    log::warn!("unlinking a removed account timed out");
                }
                self.stop_bot().await;
                self.emit(Event::AccountRemoved);
            }
            Command::Reconnect => {
                if let Some(client) = self.client.clone() {
                    tokio::spawn(async move { client.reconnect_immediately().await });
                } else {
                    self.start_bot().await;
                }
            }
            Command::Shutdown => {}
            Command::OlderSent {
                chat,
                serial,
                request,
            } => self.older_sent(chat, serial, request),
            Command::OlderFailed {
                chat,
                serial,
                error,
            } => self.older_failed(chat, serial, error),
            Command::GroupInfoFailed {
                chat,
                permanent,
                request_generation,
            } => {
                if request_generation == self.group_info_generation.get(&chat).copied().unwrap_or(0)
                {
                    self.handle_failed_group(chat, permanent);
                }
            }
            Command::Sent { chat, id, error } => {
                let completed = self.interactive_sending.iter().find_map(
                    |((pending_chat, source), pending_id)| {
                        (pending_chat == &chat && pending_id == &id).then(|| source.clone())
                    },
                );
                if let Some(message) = completed {
                    self.interactive_sending
                        .remove(&(chat.clone(), message.clone()));
                    self.emit(Event::InteractiveReplyState {
                        chat: chat.clone(),
                        message,
                        pending: false,
                    });
                }
                if id.is_empty() {
                    // This is a command failure, not a failed message send.
                    if let Some(error) = error {
                        self.emit(Event::Error(error));
                    }
                    return;
                }
                let status = match &error {
                    Some(_) => Delivery::Failed,
                    None => Delivery::Sent,
                };
                if error.is_some() {
                    // A failed send is never acked; a retry stores the row again.
                    self.awaiting_ack.remove(&id);
                }
                let _ = self
                    .archive
                    .set_status(&chat, &id, status, crate::util::now());
                self.emit_message(&chat, &id);
                self.emit_chat(&chat);
                self.advance_serial_forward(&id);
                if let Some(error) = error {
                    self.emit(Event::Error(format!("Message not sent: {error}")));
                }
            }
            Command::Downloaded {
                card,
                chat,
                id,
                result,
            } => self.downloaded(chat, id, card, result),
            Command::DownloadMotion { chat, message } => self.download_motion(chat, message),
            Command::MotionDownloaded { chat, id, result } => {
                let result = result.and_then(|path| {
                    let saved = self.archive.put_motion_path(&chat, &id, Some(&path));
                    confirmed_download(path, saved)
                });
                self.emit(Event::Motion {
                    chat,
                    message: id,
                    result,
                });
            }
            Command::AvatarFetched { id, full, path } => {
                self.emit(Event::Avatar { id, full, path })
            }
            Command::AvatarFailed { id, full } => {
                *self.pending_avatars.entry((id, full)).or_insert(0) += 1;
            }
            Command::GroupRecipients {
                chat,
                id,
                recipients,
                lids,
                stored,
            } => {
                for (lid, pn) in lids {
                    self.learn_lid(&lid, &pn);
                }
                let saved = self.save_group_recipients(&chat, &id, &recipients);
                let _ = stored.send(saved);
            }
            Command::GroupInfo {
                chat,
                name,
                description,
                description_generation,
                request_generation,
                participants,
                read_only,
                ephemeral_expiration,
                ephemeral_setting_timestamp,
                leave_generation,
                info_locked,
                admin,
                subject_generation,
                settings,
                settings_generation,
            } => {
                if request_generation != self.group_info_generation.get(&chat).copied().unwrap_or(0)
                {
                    return;
                }
                // A snapshot asked for before a rename made here was confirmed
                // may still carry the old subject: keep ours.
                let name = name.filter(|_| {
                    subject_generation >= self.subject_generation.get(&chat).copied().unwrap_or(0)
                });
                if name.as_deref().is_none_or(|name| name.trim().is_empty()) {
                    self.handle_failed_group(chat.clone(), false);
                } else {
                    self.group_info_tries.remove(&chat);
                    self.group_info_retry.retain(|(_, id)| id != &chat);
                }
                // A snapshot asked for before a permission or member change
                // made here was accepted may predate it: keep what we filed.
                let current = settings_generation
                    >= self.settings_generation.get(&chat).copied().unwrap_or(0);
                let participants = if current {
                    participants
                } else {
                    self.archive
                        .chat(&chat)
                        .ok()
                        .flatten()
                        .map(|row| row.participants)
                        .unwrap_or(participants)
                };
                let _ =
                    self.archive
                        .set_group_info(&chat, name.as_deref(), &participants, read_only);
                if current {
                    let _ = self.archive.set_group_rights(&chat, info_locked, admin);
                    let _ = self.archive.set_group_settings(&chat, &settings);
                }
                if description_generation
                    >= self.description_generation.get(&chat).copied().unwrap_or(0)
                {
                    let _ = self.archive.set_group_description(&chat, &description);
                }
                // Metadata that lists us again means we are back in, so a
                // remembered leave no longer holds. Only a snapshot asked for
                // after the leave counts: one already in flight when it was
                // confirmed still lists us and would undo it.
                if leave_generation >= self.leave_generation.get(&chat).copied().unwrap_or(0)
                    && participants.iter().any(|id| self.is_me(id))
                {
                    let _ = self.archive.set_left(&chat, false);
                }
                if let Some(expiration) = ephemeral_expiration {
                    let _ = self.archive.set_ephemeral(
                        &chat,
                        expiration,
                        ephemeral_setting_timestamp.unwrap_or_default(),
                    );
                }
                self.emit_chat(&chat);
            }
        }
    }

    /// Whether the archive says we may change this group's name and photo,
    /// telling the user when not. The dialog only offers the change when we
    /// may; this guards a change asked for just before the rights changed.
    fn may_edit_group(&mut self, chat: &str) -> bool {
        let allowed = self
            .archive
            .chat(chat)
            .ok()
            .flatten()
            .is_some_and(|row| row.can_edit_info());
        if !allowed {
            self.emit(Event::Error(GROUP_EDIT_REFUSED.to_owned()));
        }
        allowed
    }

    /// Renames a group on WhatsApp. An empty or unchanged name does nothing.
    ///
    /// The name changes here only once WhatsApp accepts it
    /// (`group_edited`), not optimistically: it shows in the chat list,
    /// notifications and the header, and a refused rename (a locked group, a
    /// revoked admin) would otherwise flash the new name everywhere and then
    /// take it back. The answer comes quickly, and the dialog says it is saving
    /// meanwhile.
    fn set_group_name(&mut self, chat: ChatId, name: String) {
        let name = name.trim().to_owned();
        let Ok(Some(row)) = self.archive.chat(&chat) else {
            return;
        };
        if name.is_empty() || name == row.name {
            return;
        }
        if !self.may_edit_group(&chat) {
            return;
        }
        let Ok(subject) = whatsapp_rust::GroupSubject::new(name.clone()) else {
            self.emit(Event::Error(format!(
                "A group name can have at most {} characters.",
                crate::model::GROUP_NAME_LIMIT
            )));
            return;
        };
        let (Some(client), Some(jid)) = (self.client.clone(), Self::jid_of(&chat)) else {
            self.emit(Event::Error(
                "Connect to WhatsApp to change the group's name.".to_owned(),
            ));
            return;
        };
        self.emit(Event::GroupSaving {
            chat: chat.clone(),
            saving: true,
        });
        let commands = self.commands.clone();
        let waker = self.waker.clone();
        tokio::spawn(async move {
            let result = client
                .groups()
                .set_subject(jid, subject)
                .await
                .map_err(|error| error.to_string());
            let _ = commands.send(Command::GroupEdited {
                chat,
                edit: GroupEdit::Name(name),
                result,
            });
            waker.wake();
        });
    }

    /// Sets the group's photo to a prepared JPEG, or removes it.
    fn set_group_picture(&mut self, chat: ChatId, jpeg: Option<Vec<u8>>) {
        if !self.may_edit_group(&chat) {
            return;
        }
        let (Some(client), Some(jid)) = (self.client.clone(), Self::jid_of(&chat)) else {
            self.emit(Event::Error(
                "Connect to WhatsApp to change the group's photo.".to_owned(),
            ));
            return;
        };
        self.emit(Event::GroupSaving {
            chat: chat.clone(),
            saving: true,
        });
        match &jpeg {
            Some(jpeg) => self.group_pictures_sent.insert(chat.clone(), jpeg.clone()),
            None => self.group_pictures_sent.remove(&chat),
        };
        let commands = self.commands.clone();
        let waker = self.waker.clone();
        tokio::spawn(async move {
            let removed = jpeg.is_none();
            let result = match jpeg {
                Some(jpeg) => client.groups().set_profile_picture(jid, jpeg).await,
                None => client.groups().remove_profile_picture(jid).await,
            }
            .map(|_| ())
            .map_err(|error| error.to_string());
            let _ = commands.send(Command::GroupEdited {
                chat,
                edit: GroupEdit::Picture { removed },
                result,
            });
            waker.wake();
        });
    }

    /// Applies WhatsApp's answer to a group name or photo change.
    fn group_edited(&mut self, chat: ChatId, edit: GroupEdit, result: Result<(), String>) {
        self.emit(Event::GroupSaving {
            chat: chat.clone(),
            saving: false,
        });
        let sent_picture = match edit {
            GroupEdit::Picture { .. } => self.group_pictures_sent.remove(&chat),
            GroupEdit::Name(_) => None,
        };
        match (result, edit) {
            (Ok(()), GroupEdit::Name(name)) => {
                // Metadata asked for before now may still name the old subject.
                *self.subject_generation.entry(chat.clone()).or_default() += 1;
                let _ = self.archive.rename_chat(&chat, &name);
                self.emit_chat(&chat);
            }
            (Ok(()), GroupEdit::Picture { removed: false }) if sent_picture.is_some() => {
                self.show_sent_picture(chat, sent_picture.unwrap_or_default());
            }
            (Ok(()), GroupEdit::Picture { removed }) => self.refresh_avatar(chat, removed),
            (Err(error), edit) => {
                let refused = group_edit_refused(&error);
                log::warn!(
                    "could not change group info ({})",
                    if refused { "refused" } else { "failed" }
                );
                self.emit(Event::Error(if refused {
                    GROUP_EDIT_REFUSED.to_owned()
                } else {
                    match edit {
                        GroupEdit::Name(_) => "Could not rename the group.",
                        GroupEdit::Picture { removed: false } => {
                            "Could not change the group's photo."
                        }
                        GroupEdit::Picture { removed: true } => {
                            "Could not remove the group's photo."
                        }
                    }
                    .to_owned()
                }));
                if refused {
                    // Our rights changed without our knowing: learn them, so
                    // the dialog stops offering what WhatsApp refuses.
                    self.request_group_info(&chat, true);
                }
            }
        }
    }

    /// Tells the phone we are leaving, then keeps the local history.
    ///
    /// A group goes through the group API and a channel through the newsletter
    /// API. Leaving does not delete anything here: the chat stays with its
    /// messages, and only stops accepting new ones.
    async fn leave_group(&mut self, chat: ChatId, archive: bool) {
        let channel = chat.ends_with("@newsletter");
        if !channel && ChatKind::from_id(&chat) != ChatKind::Group {
            return;
        }
        // Leaving is a phone action. Without a connection nothing is changed
        // here, so a later reconnect does not leave a chat that still counts
        // us as a member.
        let (Some(client), Some(jid)) = (self.client.clone(), Self::jid_of(&chat)) else {
            self.emit(Event::Error(if channel {
                "Could not leave the channel.".into()
            } else {
                "Could not leave the group.".into()
            }));
            self.emit_chat(&chat);
            return;
        };
        let result = if channel {
            client
                .newsletter()
                .leave(&jid)
                .await
                .map_err(|error| error.to_string())
        } else {
            client
                .groups()
                .leave(jid)
                .await
                .map_err(|error| error.to_string())
        };
        if let Err(error) = result {
            if channel {
                log::warn!("could not leave channel: {error}");
            } else {
                log::warn!("could not leave group: {error}");
            }
            self.emit(Event::Error(if channel {
                "Could not leave the channel.".into()
            } else {
                "Could not leave the group.".into()
            }));
            // The chat goes back to what the archive says, which rolls back
            // the optimistic mark the interface made when the menu was used.
            self.emit_chat(&chat);
            return;
        }
        self.finish_leave(&chat, archive);
    }

    /// Marks the chat as one we can no longer post in, once the phone agreed.
    fn finish_leave(&mut self, chat: &str, archive: bool) {
        // Any metadata already in flight belongs to the state before this.
        *self.leave_generation.entry(chat.to_owned()).or_default() += 1;
        let Ok(Some(row)) = self.archive.chat(chat) else {
            return;
        };
        let participants: Vec<_> = row
            .participants
            .into_iter()
            .filter(|id| !self.is_me(id))
            .collect();
        let _ = self.archive.set_group_info(chat, None, &participants, true);
        // A mark of its own, so a later metadata refresh cannot make the chat
        // writable again or bring Leave back.
        let _ = self.archive.set_left(chat, true);
        if archive {
            let _ = self.archive.set_archived(chat, true);
            self.archive_on_phone(chat, true);
        }
        self.emit_chat(chat);
    }

    /// Save the same audience the protocol library uses to encrypt the send.
    fn save_group_recipients(&self, chat: &str, id: &str, recipients: &[String]) -> bool {
        let recipients: Vec<_> = recipients
            .iter()
            .map(|id| self.canonical_str(id))
            .filter(|id| !self.is_me(id))
            .collect();
        match self
            .archive
            .snapshot_group_recipients(chat, id, &recipients)
        {
            Ok(()) => true,
            Err(error) => {
                log::warn!("could not save the group message audience: {error}");
                false
            }
        }
    }

    /// Resolves the message a send replies to. A reply whose original cannot
    /// be quoted is refused rather than sent as an unrelated message: the
    /// quote needs the original's archived row and its raw protobuf.
    fn quote_target(
        &self,
        chat: &str,
        target: Option<&crate::model::ReplyTarget>,
    ) -> Result<Option<(wa::ContextInfo, Quoted)>, Refusal> {
        let Some(target) = target else {
            return Ok(None);
        };
        let id = target.id.as_str();
        let source = target.source_chat(chat);
        let unavailable = Refusal::QuoteUnavailable;
        if id.is_empty() {
            return Err(unavailable);
        }
        let row = self
            .archive
            .message(source, id)
            .map_err(|_| unavailable)?
            .ok_or(unavailable)?;
        if matches!(row.content, Content::Revoked) {
            return Err(unavailable);
        }
        let raw = self
            .archive
            .raw(source, id)
            .map_err(|_| unavailable)?
            .ok_or(unavailable)?;
        let original = wa::Message::decode_from_slice(&raw).map_err(|_| unavailable)?;
        let jid = Self::jid_of(chat).ok_or(unavailable)?;
        let source_jid = Self::jid_of(source).ok_or(unavailable)?;
        let sender = Self::jid_of(&row.sender).ok_or(unavailable)?;
        if source != chat
            && (row.private_reply_recipient().is_none()
                || self.canonical_str(&row.sender) != self.canonical_str(chat))
        {
            return Err(unavailable);
        }
        let mut context = whatsapp_rust::wacore::proto_helpers::build_quote_context_with_info(
            row.id.clone(),
            &sender,
            &source_jid,
            &jid,
            &original,
        );
        // The quoted copy can still carry the everyone bit. Clear it so a
        // reply does not notify the group again. The quote row keeps the
        // mention for display from the archived message.
        silence_everyone(&mut context);
        let shown = Quoted {
            labelled: Some(row.content.labelled()),
            chat: (source != chat).then(|| source.to_owned()),
            mentions: row.mentions.clone(),
            id: row.id,
            sender_name: if row.from_me {
                Some("You".to_owned())
            } else {
                row.sender_name
                    .clone()
                    .or_else(|| self.name_for(&row.sender))
            },
            sender: row.sender,
            summary: row.content.summary(),
        };
        Ok(Some((context, shown)))
    }

    /// The client, only while its connection is up. The handle outlives a
    /// dropped connection (it stays through every reconnect), so a send that
    /// only checked for it went out on a closed socket and failed with
    /// "client is not connected" instead of going back to the composer.
    fn connected_client(&self) -> Option<Arc<Client>> {
        self.client
            .clone()
            .filter(|client| client.is_socket_connected())
    }

    /// Resolves an explicit quote source rather than assuming it belongs to the destination chat.
    #[cfg(test)]
    fn quote(
        &self,
        chat: &str,
        id: Option<&str>,
    ) -> Result<Option<(wa::ContextInfo, Quoted)>, Refusal> {
        self.quote_target(chat, id.map(crate::model::ReplyTarget::from).as_ref())
    }

    /// Hands a send that cannot go out back to the app.
    fn refuse(
        &self,
        chat: ChatId,
        quoting: Option<crate::model::ReplyTarget>,
        unsent: Unsent,
        reason: Refusal,
    ) {
        self.emit(Event::SendRefused {
            chat,
            quoting,
            unsent,
            reason,
        });
    }

    /// Dispatches composed text with its explicit quote target and restores it on refusal.
    fn send_text(
        &mut self,
        chat: ChatId,
        text: String,
        quoting: Option<crate::model::ReplyTarget>,
        mentions: Vec<String>,
    ) {
        let (context, shown) = match self.quote_target(&chat, quoting.as_ref()) {
            Ok(Some((context, shown))) => (Some(context), Some(shown)),
            Ok(None) => (None, None),
            Err(reason) => {
                self.refuse(chat, quoting, Unsent::Text(text), reason);
                return;
            }
        };
        let (Some(client), Some(jid)) = (self.connected_client(), Self::jid_of(&chat)) else {
            self.refuse(chat, quoting, Unsent::Text(text), Refusal::Offline);
            return;
        };
        let mut mentions = mentions;
        if self.wants_mention_all(&chat, &text) {
            crate::mentions::push_everyone_id(&mut mentions);
        }
        let mut message = outgoing_text(text.clone(), context, &mentions);
        let expiration = self.apply_ephemeral(&chat, &mut message);
        let mentions = self.mentions_of(&mentions);
        let id = client.generate_message_id().into_string();
        let row = Message {
            id: id.clone(),
            chat: chat.clone(),
            sender: self.me(),
            sender_name: None,
            from_me: true,
            timestamp: crate::util::now(),
            content: Content::text(text),
            status: Delivery::Pending,
            delivered_at: None,
            read_at: None,
            quoted: shown,
            reactions: Vec::new(),
            history_order: None,
            edited: false,
            mentions,
            forwarded: false,
            thumbnail: None,
        };
        self.store_message(row, Some(message.encode_to_vec()), None);
        tokio::spawn(send_outgoing(
            client,
            self.commands.clone(),
            chat,
            jid,
            id,
            message,
            expiration,
        ));
    }

    /// Sends a spot as a location, keeping the row locally the way a phone
    /// does, so the bubble shows before the phone acknowledges it.
    fn send_location(
        &mut self,
        chat: ChatId,
        latitude: f64,
        longitude: f64,
        quoting: Option<crate::model::ReplyTarget>,
    ) {
        let unsent = || Unsent::Location {
            latitude,
            longitude,
        };
        let (context, shown) = match self.quote_target(&chat, quoting.as_ref()) {
            Ok(Some((context, shown))) => (Some(context), Some(shown)),
            Ok(None) => (None, None),
            Err(reason) => {
                self.refuse(chat, quoting, unsent(), reason);
                return;
            }
        };
        let (Some(client), Some(jid)) = (self.connected_client(), Self::jid_of(&chat)) else {
            self.refuse(chat, quoting, unsent(), Refusal::Offline);
            return;
        };
        let mut message = outgoing_location(latitude, longitude, context);
        let expiration = self.apply_ephemeral(&chat, &mut message);
        let id = client.generate_message_id().into_string();
        let row = Message {
            id: id.clone(),
            chat: chat.clone(),
            sender: self.me(),
            sender_name: None,
            from_me: true,
            timestamp: crate::util::now(),
            content: Content::Location {
                latitude,
                longitude,
                name: None,
                address: None,
            },
            status: Delivery::Pending,
            delivered_at: None,
            read_at: None,
            quoted: shown,
            reactions: Vec::new(),
            history_order: None,
            edited: false,
            mentions: Vec::new(),
            forwarded: false,
            thumbnail: None,
        };
        self.store_message(row, Some(message.encode_to_vec()), None);
        tokio::spawn(send_outgoing(
            client,
            self.commands.clone(),
            chat,
            jid,
            id,
            message,
            expiration,
        ));
    }

    /// How many chats these archived messages may go to at once. The
    /// forwarding score lives in the raw protobuf, so messages archived before
    /// the limit existed are judged the same way.
    fn forward_limit(&self, from_chat: &str, messages: &[String]) -> usize {
        crate::model::forward_limit(messages.iter().any(|id| {
            self.archive
                .raw(from_chat, id)
                .ok()
                .flatten()
                .and_then(|raw| wa::Message::decode_from_slice(&raw).ok())
                .is_some_and(|message| frequently_forwarded(message.get_base_message()))
        }))
    }

    /// Forwards a batch to each recipient in turn, or refuses all of it when
    /// it names more chats than WhatsApp allows for these messages.
    fn forward_to_chats(
        &mut self,
        from_chat: ChatId,
        messages: Vec<String>,
        to_chats: Vec<ChatId>,
    ) {
        let limit = self.forward_limit(&from_chat, &messages);
        if to_chats.len() > limit {
            self.emit(Event::Error(if limit == 1 {
                "Messages forwarded many times can be forwarded to only one chat".to_owned()
            } else {
                format!("Messages can be forwarded to up to {limit} chats at once")
            }));
            return;
        }
        for to_chat in to_chats {
            self.forward_messages(from_chat.clone(), messages.clone(), to_chat);
        }
    }

    /// Prepares archived copies and queues each destination behind active sends.
    fn forward_messages(&mut self, from_chat: ChatId, messages: Vec<String>, to_chat: ChatId) {
        let Some(client) = self.connected_client() else {
            self.emit(Event::Error("Not connected to WhatsApp".to_owned()));
            return;
        };
        let Some(jid) = Self::jid_of(&to_chat) else {
            self.emit(Event::Error("Not connected to WhatsApp".to_owned()));
            return;
        };
        let jobs: Vec<_> = messages
            .iter()
            .filter_map(|message| {
                self.forward_job(&from_chat, message, &to_chat)
                    .map(|(id, message, expiration)| {
                        (id, (to_chat.clone(), jid.clone(), message, expiration))
                    })
            })
            .collect();
        if jobs.is_empty() {
            return;
        }
        let first = match self.forward_queue.as_mut() {
            Some(queue) => queue.push(jobs),
            None => {
                let mut queue = ForwardQueue::new();
                let first = queue.push(jobs);
                self.forward_queue = Some(queue);
                first
            }
        };
        let Some((id, (to_chat, jid, message, expiration))) = first else {
            // A batch is already going: these follow it, in order.
            return;
        };
        tokio::spawn(send_outgoing(
            client,
            self.commands.clone(),
            to_chat,
            jid,
            id,
            message,
            expiration,
        ));
    }

    /// Starts the next queued forward once `id` reports its first tick, or its
    /// failure. An ack that is not the running job's is ignored.
    fn advance_serial_forward(&mut self, id: &str) {
        let Some((id, job)) = self.next_writable_forward(id) else {
            return;
        };
        let Some(client) = self.client.clone() else {
            // The link went away; the rest of the batch cannot be sent.
            let mut failed = vec![(job.0, id)];
            failed.extend(self.take_queued_forwards());
            self.fail_forwards(failed);
            return;
        };
        let (to_chat, jid, message, expiration) = job;
        tokio::spawn(send_outgoing(
            client,
            self.commands.clone(),
            to_chat,
            jid,
            id,
            message,
            expiration,
        ));
    }

    /// Rechecks the current privacy and chat state at the worker's send boundary.
    fn destination_writable(&self, chat: &str) -> bool {
        self.privacy_ready
            && match self.archive.chat(chat) {
                Ok(Some(chat)) => chat.can_send(),
                Ok(None) => ChatKind::from_id(chat) != ChatKind::Broadcast,
                Err(_) => false,
            }
    }

    /// A locked chat accepts messages written in it, but never a forward:
    /// the picker does not list it, so one aimed there was locked meanwhile.
    fn forward_writable(&self, chat: &str) -> bool {
        self.destination_writable(chat)
            && !matches!(self.archive.chat(chat), Ok(Some(chat)) if chat.locked)
    }

    /// Skips destinations that became unwritable while waiting, marking their
    /// pending rows failed and continuing with the next authorized destination.
    fn next_writable_forward(&mut self, completed: &str) -> Option<(String, ForwardJob)> {
        let mut completed = completed.to_owned();
        let mut reported_read_only = false;
        loop {
            let next = self.forward_queue.as_mut()?.ack(&completed);
            let (id, job) = match next {
                ForwardStep::Ignore => return None,
                ForwardStep::Next { id, payload } => (id, payload),
                ForwardStep::Finished => {
                    self.forward_queue = None;
                    return None;
                }
            };
            if self.forward_writable(&job.0) {
                return Some((id, job));
            }
            // Never sent, so never acked.
            self.awaiting_ack.remove(&id);
            let _ = self
                .archive
                .set_status(&job.0, &id, Delivery::Failed, crate::util::now());
            self.emit_message(&job.0, &id);
            self.emit_chat(&job.0);
            if !reported_read_only {
                self.emit(Event::Error(
                    "This conversation is read-only in ZapFast".to_owned(),
                ));
                reported_read_only = true;
            }
            completed = id;
        }
    }

    /// Drops a batch whose session ended. Its queued messages are already in
    /// the archive as pending, and nothing resends pending messages, so they
    /// are marked failed rather than left waiting forever. The running send
    /// still reports for itself.
    fn abandon_forwards(&mut self) {
        let queued = self.take_queued_forwards();
        self.fail_forwards(queued);
    }

    /// Empties the forward queue, returning the chat and id of each job that
    /// had not started.
    fn take_queued_forwards(&mut self) -> Vec<(ChatId, String)> {
        self.forward_queue
            .take()
            .map(|queue| {
                queue
                    .remaining
                    .into_iter()
                    .map(|(id, (chat, ..))| (chat, id))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn fail_forwards(&mut self, messages: Vec<(ChatId, String)>) {
        if messages.is_empty() {
            return;
        }
        let at = crate::util::now();
        let mut chats = HashSet::new();
        for (chat, id) in &messages {
            let _ = self.archive.set_status(chat, id, Delivery::Failed, at);
            self.emit_message(chat, id);
            chats.insert(chat.clone());
        }
        for chat in &chats {
            self.emit_chat(chat);
        }
        self.emit(Event::Error(
            "Not connected to WhatsApp: the rest of the forwarded messages were not sent"
                .to_owned(),
        ));
    }

    /// Prepares one forwarded message: its stored row and the outgoing
    /// protobuf. `None` when it cannot be forwarded, reported to the user.
    fn forward_job(
        &mut self,
        from_chat: &ChatId,
        message_id: &str,
        to_chat: &ChatId,
    ) -> Option<(String, wa::Message, Option<u32>)> {
        let Ok(Some(source)) = self.archive.message(from_chat, message_id) else {
            self.emit(Event::Error(
                "This message is not stored on this computer".to_owned(),
            ));
            return None;
        };
        if matches!(
            source.content,
            Content::Revoked
                | Content::Unsupported { .. }
                | Content::PhoneOnly { .. }
                | Content::Poll { .. }
                | Content::Interactive { .. }
        ) {
            self.emit(Event::Error("This message cannot be forwarded".to_owned()));
            return None;
        }
        let Ok(Some(raw)) = self.archive.raw(from_chat, message_id) else {
            self.emit(Event::Error(
                "The original message data is not available to forward".to_owned(),
            ));
            return None;
        };
        let Ok(original) = wa::Message::decode_from_slice(&raw) else {
            self.emit(Event::Error(
                "The original message data could not be read".to_owned(),
            ));
            return None;
        };
        // whatsapp-rust owns the forwarding rules: unwrap transient wrappers,
        // strip quote chains and secrets, and retain reusable media metadata.
        let (message, expiration) = outgoing_forward(&original, self.ephemeral_expiration(to_chat));
        let client = self.client.clone()?;
        let id = client.generate_message_id().into_string();
        let mentions = self.mentions_of(&recorded_mention_ids(&message));
        let thumbnail = thumbnail_of(&message).or_else(|| source.thumbnail.clone());
        let row = forwarded_row(
            source,
            to_chat.clone(),
            self.me(),
            id.clone(),
            crate::util::now(),
            mentions,
            thumbnail,
        );
        self.store_message(row, Some(message.encode_to_vec()), None);
        Some((id, message, expiration))
    }

    fn mark_read(&mut self, chat: ChatId, receipts: bool) {
        let Ok(Some(row)) = self.archive.chat(&chat) else {
            return;
        };
        // Collect before advancing the archive's read position.
        let ids = if receipts {
            self.archive
                .unread_incoming(&chat, row.unread)
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let _ = self.archive.mark_read(&chat);
        self.emit_chat(&chat);
        // A chat marked unread with nothing pending still tells the phone it
        // was read, which is what takes the phone's mark off.
        if row.unread == 0 && !row.marked_unread {
            return;
        }
        let _ = self.archive.queue_read_sync(&chat);
        self.pump_read_sync();
        self.send_read_receipts(chat, ids);
    }

    fn pump_read_sync(&mut self) {
        if !matches!(self.status, LinkStatus::Connected) || !self.read_sync.ready(Instant::now()) {
            return;
        }
        let Some(client) = self.client.clone() else {
            return;
        };
        for (chat, through) in self.archive.pending_reads().unwrap_or_default() {
            let Some(jid) = Self::jid_of(&chat) else {
                continue;
            };
            if !self.read_sync.start(&chat, through, Instant::now()) {
                break;
            }
            let client = client.clone();
            let commands = self.commands.clone();
            tokio::spawn(async move {
                // This update is private to our devices, even with blue ticks
                // disabled. Keep the original position when retrying offline
                // reads, not the latest message received since the local read.
                let range = whatsapp_rust::message_range(through, None, Vec::new());
                let result = client
                    .chat_actions()
                    .mark_chat_as_read(&jid, true, Some(range))
                    .await;
                if let Err(error) = &result {
                    log::debug!("chat read state not synced: {error}");
                }
                let _ = commands.send(Command::ReadSyncFinished {
                    chat,
                    through,
                    success: result.is_ok(),
                });
            });
            // Every read-state write uses regular_low. A queue of spawned tasks
            // would each retry the same broken collection before we can back off.
            return;
        }
        // Reads go first: a chat read here and then marked unread again ends
        // unread on the phone too.
        for (chat, marked_at, through) in self.archive.pending_unreads().unwrap_or_default() {
            let Some(jid) = Self::jid_of(&chat) else {
                continue;
            };
            if !self
                .read_sync
                .start_unread(&chat, marked_at, Instant::now())
            {
                break;
            }
            let client = client.clone();
            let commands = self.commands.clone();
            tokio::spawn(async move {
                let range = whatsapp_rust::message_range(through, None, Vec::new());
                let result = client
                    .chat_actions()
                    .mark_chat_as_read(&jid, false, Some(range))
                    .await;
                if let Err(error) = &result {
                    log::debug!("chat unread mark not synced: {error}");
                }
                let _ = commands.send(Command::UnreadSyncFinished {
                    chat,
                    marked_at,
                    success: result.is_ok(),
                });
            });
            break;
        }
    }

    fn send_read_receipts(&self, chat: ChatId, ids: Vec<(String, String)>) {
        if ids.is_empty() {
            return;
        }
        let (Some(client), Some(jid)) = (self.client.clone(), Self::jid_of(&chat)) else {
            return;
        };
        let is_group = jid.is_group();
        let mut by_sender: HashMap<Option<String>, Vec<String>> = HashMap::new();
        for (id, sender) in ids {
            by_sender
                .entry(is_group.then_some(sender))
                .or_default()
                .push(id);
        }
        let commands = self.commands.clone();
        tokio::spawn(async move {
            if !receipts_allowed(&client, &jid, &commands).await {
                return;
            }
            for (sender, ids) in by_sender {
                let sender = sender.and_then(|sender| sender.parse::<Jid>().ok());
                let ids: Vec<&str> = ids.iter().map(String::as_str).collect();
                if let Err(error) = client.mark_as_read(&jid, sender.as_ref(), &ids).await {
                    log::debug!("read receipt not sent: {error}");
                }
            }
        });
    }

    /// Refreshes stored quote ids and names with current mappings.
    fn polish(&self, message: &mut Message) {
        self.polish_poll(message);
        if let Some(quoted) = message.quoted.as_mut() {
            if let Some(chat) = quoted.chat.as_mut() {
                *chat = self.canonical_str(chat);
            }
            let sender = self.canonical_str(&quoted.sender);
            // A currently known name replaces a stale label even when the
            // canonical id did not change; an unresolvable one keeps what
            // the archive already has.
            if let Some(name) = self.name_for(&sender) {
                quoted.sender_name = Some(name);
            }
            quoted.sender = sender;
            quoted.summary = self.pn_tokens(&quoted.summary);
            quoted.mentions =
                self.quote_mentions(&quoted.summary, std::mem::take(&mut quoted.mentions));
        }
        for mention in &mut message.mentions {
            mention.id = self.canonical_str(&mention.id);
        }
    }

    fn load_chat(&mut self, chat: ChatId, before: Option<super::PageKey>) {
        self.send_page(&chat, before.clone());
        if before.is_none()
            && let Ok(ids) = self.archive.starred_ids(&chat)
        {
            self.emit(Event::Stars {
                chat: chat.clone(),
                ids: ids.into_iter().collect(),
            });
        }
        if before.is_none() {
            self.emit_pins(&chat);
        }
        if before.is_none() && ChatKind::from_id(&chat) == ChatKind::Group {
            // Force group metadata when opening a group.
            self.request_group_info(&chat, false);
        }
        if before.is_none()
            && ChatKind::from_id(&chat) == ChatKind::Direct
            && chat != self.me()
            && self.presence_subscribed.insert(chat.clone())
            && let (Some(client), Some(jid)) = (self.client.clone(), Self::jid_of(&chat))
        {
            tokio::spawn(async move {
                if let Err(error) = client.presence().subscribe(jid).await {
                    log::debug!("presence not subscribed: {error}");
                }
            });
        }
    }

    fn download(&mut self, chat: ChatId, id: String) {
        self.download_media(chat, id, None, false);
    }

    /// Downloads an attachment. A `manual` one, asked for by the reader, has
    /// no size limit; every download's deadline grows with its size.
    fn download_media(&mut self, chat: ChatId, id: String, card: Option<usize>, manual: bool) {
        if !self.downloads.insert((chat.clone(), id.clone(), card)) {
            // A click while only the picker is fetching it: the picker's
            // download skips the re-upload, so remember to ask if it fails.
            let key = (chat, id);
            if card.is_none() && self.sticker_downloads.contains(&key) {
                self.sticker_clicked.insert(key);
            }
            return;
        }
        let Some(client) = self.client.clone() else {
            self.downloaded(chat, id, card, Err("Not connected to WhatsApp".to_owned()));
            return;
        };
        let raw = self.archive.raw(&chat, &id).ok().flatten();
        let Some(message) = raw.and_then(|raw| wa::Message::decode_from_slice(&raw).ok()) else {
            self.downloaded(
                chat,
                id,
                card,
                Err("Attachment download keys are missing".to_owned()),
            );
            return;
        };
        let original = message.get_base_message();
        let base = match interactive::image_at(original, card) {
            Some(image) => {
                let mut proto = wa::Message::default();
                proto.image_message = MessageField::some(image.clone());
                proto
            }
            None if card.is_none() => original.clone(),
            None => {
                self.downloaded(
                    chat,
                    id,
                    card,
                    Err("This card has no downloadable image".to_owned()),
                );
                return;
            }
        };
        let (downloadable, mime, file_name): (Box<dyn Downloadable>, String, Option<String>) =
            if let Some(image) = base.image_message.as_option() {
                (
                    Box::new(image.clone()),
                    image.mimetype.clone().unwrap_or_default(),
                    None,
                )
            } else if let Some(video) = base
                .video_message
                .as_option()
                .or(base.ptv_message.as_option())
            {
                (
                    Box::new(video.clone()),
                    video.mimetype.clone().unwrap_or_default(),
                    None,
                )
            } else if let Some(audio) = base.audio_message.as_option() {
                (
                    Box::new(audio.clone()),
                    audio.mimetype.clone().unwrap_or_default(),
                    None,
                )
            } else if let Some(document) = base.document_message.as_option() {
                (
                    Box::new(document.clone()),
                    document.mimetype.clone().unwrap_or_default(),
                    document.file_name.clone(),
                )
            } else if let Some(sticker) = base.sticker_message.as_option() {
                (
                    Box::new(sticker.clone()),
                    sticker.mimetype.clone().unwrap_or_default(),
                    None,
                )
            } else {
                self.downloaded(
                    chat,
                    id,
                    card,
                    Err("This message has no downloadable file".to_owned()),
                );
                return;
            };
        if !manual && attachment_is_too_large(downloadable.file_length()) {
            self.downloaded(chat, id, card, Err(ATTACHMENT_LIMIT_ERROR.to_owned()));
            return;
        }
        let limit = if manual {
            u64::MAX
        } else {
            ATTACHMENT_DOWNLOAD_LIMIT
        };
        let deadline = attachment_deadline(downloadable.file_length());
        // Keep metadata needed for one media re-upload request and retry.
        let media_key = base
            .image_message
            .as_option()
            .and_then(|media| media.media_key.clone())
            .or_else(|| {
                base.video_message
                    .as_option()
                    .or(base.ptv_message.as_option())
                    .and_then(|media| media.media_key.clone())
            })
            .or_else(|| {
                base.audio_message
                    .as_option()
                    .and_then(|media| media.media_key.clone())
            })
            .or_else(|| {
                base.document_message
                    .as_option()
                    .and_then(|media| media.media_key.clone())
            })
            .or_else(|| {
                base.sticker_message
                    .as_option()
                    .and_then(|media| media.media_key.clone())
            })
            .unwrap_or_default();
        // A sticker only the picker wanted is not worth a re-upload request:
        // the picker fetches in the background, and each request counts
        // toward the server's rate limit (#405). A click on its bubble asks.
        let reupload = !self.sticker_downloads.contains(&(chat.clone(), id.clone()));
        let jid = Self::jid_of(&chat);
        let row = self.archive.message(&chat, &id).ok().flatten();
        let is_from_me = row.as_ref().is_some_and(|row| row.from_me);
        let participant = match (&jid, &row) {
            (Some(jid), Some(row)) if jid.is_group() => Self::jid_of(&row.sender),
            _ => None,
        };
        let mut fresh_base = base;
        let mut refreshed = move |direct: String| -> Option<Box<dyn Downloadable>> {
            if let Some(media) = fresh_base.image_message.as_option_mut() {
                media.direct_path = Some(direct);
                media.url = None;
                return Some(Box::new(media.clone()));
            }
            if let Some(media) = fresh_base
                .video_message
                .as_option_mut()
                .or(fresh_base.ptv_message.as_option_mut())
            {
                media.direct_path = Some(direct);
                media.url = None;
                return Some(Box::new(media.clone()));
            }
            if let Some(media) = fresh_base.audio_message.as_option_mut() {
                media.direct_path = Some(direct);
                media.url = None;
                return Some(Box::new(media.clone()));
            }
            if let Some(media) = fresh_base.document_message.as_option_mut() {
                media.direct_path = Some(direct);
                media.url = None;
                return Some(Box::new(media.clone()));
            }
            if let Some(media) = fresh_base.sticker_message.as_option_mut() {
                media.direct_path = Some(direct);
                media.url = None;
                return Some(Box::new(media.clone()));
            }
            None
        };
        let dir = self.download_dir(&chat);
        let cache = self.dirs.media_cache_dir();
        // In the reader's own folder a document keeps the name it was sent
        // with (#363); the cache keeps names unique by message.
        let own_folder = dir != self.dirs.media_cache_dir();
        let commands = self.commands.clone();
        tokio::spawn(async move {
            let cache_id = card.map_or_else(|| id.clone(), |index| format!("{id}-card-{index}"));
            let cache_path = media_path(&cache, &chat, &cache_id, &mime, file_name.as_deref());
            let named = file_name
                .as_deref()
                .filter(|_| own_folder)
                .and_then(|name| named_media_path(&dir, &mime, name));
            let keep_others = named.is_some();
            let path = named
                .unwrap_or_else(|| media_path(&dir, &chat, &cache_id, &mime, file_name.as_deref()));
            let result = with_attachment_deadline(deadline, async {
                match fetch_attachment(
                    &client,
                    &*downloadable,
                    &dir,
                    &path,
                    limit,
                    keep_others,
                    Some(&cache_path),
                )
                .await
                {
                    Ok(path) => Ok(path),
                    Err(error) => {
                        let text = error.to_string();
                        let expired = ["403", "404", "410"].iter().any(|code| text.contains(code));
                        match (&jid, expired && reupload && !media_key.is_empty()) {
                            (Some(jid), true) => {
                                // Ask the phone to re-upload expired media, then retry once.
                                let target = whatsapp_rust::MessageId::new(&id).and_then(|id| {
                                    whatsapp_rust::MessageRef::new(
                                        jid,
                                        id,
                                        participant.as_ref(),
                                        is_from_me,
                                    )
                                });
                                let (Ok(target), Ok(media_key)) =
                                    (target, <&[u8; 32]>::try_from(media_key.as_slice()))
                                else {
                                    return Err(text);
                                };
                                let request = MediaReuploadRequest { target, media_key };
                                match client.media_reupload().request(&request).await {
                                    Ok(MediaRetryResult::Success { direct_path }) => {
                                        match refreshed(direct_path) {
                                            Some(again) => {
                                                fetch_attachment(
                                                    &client,
                                                    &*again,
                                                    &dir,
                                                    &path,
                                                    limit,
                                                    keep_others,
                                                    Some(&cache_path),
                                                )
                                                .await
                                            }
                                            None => Err(text),
                                        }
                                    }
                                    Ok(_) => {
                                        Err("No longer available on WhatsApp's servers".to_owned())
                                    }
                                    Err(_error) => {
                                        log::info!("media re-upload was not granted");
                                        Err("No longer available on WhatsApp's servers".to_owned())
                                    }
                                }
                            }
                            _ => Err(text),
                        }
                    }
                }
            })
            .await;
            let _ = commands.send(Command::Downloaded {
                card,
                chat,
                id,
                result,
            });
        });
    }

    /// Keeps a motion photo's clip for its photo, which may not be here yet.
    fn file_motion_clip(&mut self, chat: &str, parent: &str, sender: &str, clip: &[u8]) {
        match self.archive.put_motion_clip(chat, parent, sender, clip) {
            Ok(true) => self.emit_message(chat, parent),
            Ok(false) => {}
            Err(error) => log::warn!("could not store a motion clip: {error}"),
        }
    }

    /// Downloads a motion photo's clip into the cache. It is fetched like any
    /// attachment: verified, under the size limit, and within the deadline.
    fn download_motion(&mut self, chat: ChatId, id: String) {
        let clip = self.archive.motion_clip(&chat, &id).ok().flatten();
        let video = clip
            .and_then(|raw| wa::Message::decode_from_slice(&raw).ok())
            .and_then(|message| message.video_message.into_option());
        let (client, video) = match (self.client.clone(), video) {
            (Some(client), Some(video)) if !attachment_is_too_large(video.file_length) => {
                (client, video)
            }
            (client, video) => {
                let error = match (client, video) {
                    (None, _) => "Not connected to WhatsApp",
                    (_, None) => "The clip has not arrived",
                    _ => ATTACHMENT_LIMIT_ERROR,
                };
                self.emit(Event::Motion {
                    chat,
                    message: id,
                    result: Err(error.to_owned()),
                });
                return;
            }
        };
        let dir = self.dirs.media_cache_dir();
        let commands = self.commands.clone();
        tokio::spawn(async move {
            let path = media_path(&dir, &chat, &format!("{id}-motion"), "video/mp4", None);
            let result = with_attachment_deadline(
                attachment_deadline(video.file_length),
                download_attachment(&client, &video, &dir, &path, ATTACHMENT_DOWNLOAD_LIMIT),
            )
            .await;
            let _ = commands.send(Command::MotionDownloaded { chat, id, result });
        });
    }

    /// Files the result and releases any picker request that started it.
    fn downloaded(
        &mut self,
        chat: ChatId,
        id: String,
        card: Option<usize>,
        result: Result<PathBuf, String>,
    ) {
        self.downloads.remove(&(chat.clone(), id.clone(), card));
        let key = (chat.clone(), id.clone());
        let for_picker = self.sticker_downloads.remove(&key);
        let clicked = self.sticker_clicked.remove(&key);
        let retried = self.sticker_retried.remove(&key);
        if for_picker && let Err(error) = &result {
            if sticker_pace::rate_limited(error) {
                log::warn!("sticker downloads paused: the server asked to slow down");
                self.sticker_pace.limited(Instant::now());
                self.sticker_resume = true;
            } else if clicked {
                // Asked for in its chat meanwhile: try again as that click,
                // which may ask the phone to re-upload it.
                self.sticker_retried.insert(key);
                self.download(chat, id);
                self.fetch_missing_stickers();
                return;
            } else {
                self.sticker_download_failed.insert(key);
            }
        }
        let transferred = result.is_ok();
        let result = result.and_then(|path| {
            let saved = self
                .archive
                .put_media_path_at(&chat, &id, card, Some(&path))
                .map(|message| message.is_some());
            confirmed_download(path, saved)
        });
        if for_picker && transferred && result.is_err() {
            // A local write failure must not schedule a phone retry or another
            // automatic transfer of a file which has already arrived.
            self.sticker_download_failed
                .insert((chat.clone(), id.clone()));
        }
        let result_ok = result.is_ok();
        self.emit(Event::Media {
            card,
            chat,
            message: id,
            result,
        });
        if retried && !for_picker && result_ok {
            self.emit_stickers();
        }
        if for_picker {
            // The next missing ones take the freed places.
            self.fetch_missing_stickers();
            self.picker_sticker_settled(result_ok);
        }
    }

    /// Sends the picker its shelves after a picker download settles: once
    /// every [`SHELF_REFRESH_EVERY`] arrivals, and when none is left in
    /// flight. Listing the shelves scans the archive, and Recent is sorted
    /// by use, so each arrival lands mid-grid and shifts the tiles after it:
    /// a refresh per arrival would keep a send queued behind the scans and
    /// reshuffle the open picker under the reader (#165).
    fn picker_sticker_settled(&mut self, arrived: bool) {
        if arrived {
            self.sticker_arrivals += 1;
        }
        if self.picker_downloads() == 0 || self.sticker_arrivals >= SHELF_REFRESH_EVERY {
            self.sticker_arrivals = 0;
            self.emit_stickers();
        }
    }

    /// Picker downloads in flight, recent and chat stickers together, which
    /// share one limit.
    fn picker_downloads(&self) -> usize {
        self.sticker_fetches.len() + self.sticker_downloads.len()
    }

    /// Fetches the stickers a rate limit left behind, once its pause is over.
    fn resume_sticker_fetches(&mut self) {
        if self.sticker_resume && self.sticker_pace.open(Instant::now()) {
            self.sticker_resume = false;
            self.fetch_missing_stickers();
        }
    }

    /// Downloads missing recent and archived stickers for the picker.
    fn fetch_missing_stickers(&mut self) {
        let Some(client) = self.client.clone() else {
            return;
        };
        // A few at a time, and none while the server asked to wait: each
        // arrival starts the next (#298).
        if !self.sticker_pace.open(Instant::now()) {
            return;
        }
        let phone = match self.archive.phone_stickers() {
            Ok(list) => list,
            Err(error) => {
                log::warn!("could not list the phone's stickers: {error}");
                Vec::new()
            }
        };
        let dir = self.dirs.sticker_cache_dir();
        for sticker in phone.into_iter().filter(|sticker| sticker.path.is_none()) {
            if self.picker_downloads() >= sticker_pace::IN_FLIGHT {
                break;
            }
            if self.sticker_failed.contains(&sticker.hash)
                || !self.sticker_fetches.insert(sticker.hash.clone())
            {
                continue;
            }
            let Ok(meta) = wa::StickerMetadata::decode_from_slice(&sticker.raw) else {
                self.sticker_fetches.remove(&sticker.hash);
                continue;
            };
            if attachment_is_too_large(meta.file_length) {
                self.sticker_fetches.remove(&sticker.hash);
                log::info!("recent sticker exceeds the attachment download limit");
                continue;
            }
            let client = client.clone();
            let commands = self.commands.clone();
            let dir = dir.clone();
            let hash = sticker.hash;
            tokio::spawn(async move {
                // The shelf waits for the whole batch, so none may hang.
                let deadline = attachment_deadline(meta.file_length);
                let result = with_attachment_deadline(deadline, async {
                    let path = dir.join(format!("{hash}.webp"));
                    let sticker = PhoneSticker(meta);
                    download_attachment(&client, &sticker, &dir, &path, ATTACHMENT_DOWNLOAD_LIMIT)
                        .await
                })
                .await;
                let _ = commands.send(Command::StickerFetched { hash, result });
            });
        }
        // Rows that failed this session are skipped below; asking for that
        // many more keeps them from filling the window and hiding the rest.
        let limit = STICKER_FETCH_LIMIT + self.sticker_download_failed.len();
        match self.archive.stickers_without_file(limit) {
            Ok(list) => {
                for key in list {
                    if self.picker_downloads() >= sticker_pace::IN_FLIGHT {
                        break;
                    }
                    // One already downloading for its chat is not the picker's.
                    let downloading =
                        self.downloads
                            .contains(&(key.0.clone(), key.1.clone(), None));
                    if downloading
                        || self.sticker_download_failed.contains(&key)
                        || !self.sticker_downloads.insert(key.clone())
                    {
                        continue;
                    }
                    let (chat, id) = key;
                    self.download(chat, id);
                }
            }
            Err(error) => log::warn!("could not list unfetched stickers: {error}"),
        }
    }

    /// Root directory for sticker packs.
    fn packs_dir(&self) -> PathBuf {
        self.dirs.saved_sticker_dir().join("packs")
    }

    /// Returns sticker packs, newest first.
    fn sticker_packs(&self) -> Vec<crate::model::StickerPack> {
        super::sticker_store::packs(&self.packs_dir())
    }

    /// Returns saved sticker files, newest first.
    fn saved_stickers(&self) -> Vec<PathBuf> {
        super::sticker_store::saved(&self.dirs.saved_sticker_dir())
    }

    fn avatar_file(&self, id: &str, full: bool) -> PathBuf {
        self.dirs.avatar_file(id, full)
    }

    /// Drops the cached pictures of a chat whose picture changed, and fetches
    /// the new one unless it was removed.
    fn refresh_avatar(&mut self, id: String, removed: bool) {
        let _ = std::fs::remove_file(self.avatar_file(&id, false));
        let _ = std::fs::remove_file(self.avatar_file(&id, true));
        if removed {
            self.emit(Event::Avatar {
                id: id.clone(),
                full: false,
                path: None,
            });
            self.emit(Event::Avatar {
                id,
                full: true,
                path: None,
            });
        } else {
            self.fetch_avatar(id.clone(), false);
            self.fetch_avatar(id, true);
        }
    }

    /// Shows a picture WhatsApp just accepted from us as the chat's own, in
    /// both sizes, without asking the server for it back: a lookup right after
    /// the change can still answer with the old one. The cached copies stay
    /// fresh for [`AVATAR_FRESH`] like any fetched picture.
    fn show_sent_picture(&mut self, id: String, jpeg: Vec<u8>) {
        for full in [false, true] {
            let path = self.avatar_file(&id, full);
            let written = path
                .parent()
                .map_or(Ok(()), std::fs::create_dir_all)
                .and_then(|()| std::fs::write(&path, &jpeg));
            if written.is_err() {
                // The server has it either way.
                let _ = std::fs::remove_file(&path);
                self.fetch_avatar(id.clone(), full);
                continue;
            }
            self.emit(Event::Avatar {
                id: id.clone(),
                full,
                path: Some(path),
            });
        }
    }

    fn fetch_avatar(&mut self, id: String, full: bool) {
        let path = self.avatar_file(&id, full);
        if let Ok(metadata) = std::fs::metadata(&path)
            && metadata
                .modified()
                .ok()
                .and_then(|modified| modified.elapsed().ok())
                .is_some_and(|age| age < AVATAR_FRESH)
        {
            let path = (metadata.len() > 0).then_some(path);
            self.emit(Event::Avatar { id, full, path });
            return;
        }
        if id.ends_with("@newsletter") {
            self.fetch_channel_avatar(id, full);
            return;
        }
        // Try both of our ids for our profile picture.
        let candidates: Vec<Jid> = if self.is_me(&id) || id == self.me() {
            [self.me_pn.clone(), self.me_lid.clone()]
                .into_iter()
                .flatten()
                .filter_map(|id| Self::jid_of(&id))
                .collect()
        } else {
            Self::jid_of(&id).into_iter().collect()
        };
        if candidates.is_empty() {
            self.emit(Event::Avatar {
                id,
                full,
                path: None,
            });
            return;
        }
        let connected = self
            .client
            .as_ref()
            .is_some_and(|client| client.is_socket_connected());
        let Some(client) = self.client.clone().filter(|_| connected) else {
            // Defer profile-picture lookup until connected.
            self.pending_avatars.entry((id, full)).or_insert(0);
            return;
        };
        let commands = self.commands.clone();
        tokio::spawn(async move {
            let fetched = async {
                let mut picture = None;
                let mut failed = false;
                'lookup: for jid in &candidates {
                    for preview in [!full, false] {
                        let target = if jid.is_group() {
                            whatsapp_rust::ProfilePictureTarget::Group(jid)
                        } else {
                            whatsapp_rust::ProfilePictureTarget::Contact(jid)
                        };
                        let size = if preview {
                            whatsapp_rust::ProfilePictureType::Preview
                        } else {
                            whatsapp_rust::ProfilePictureType::Full
                        };
                        let request = whatsapp_rust::ProfilePictureRequest::new(target, size);
                        match client
                            .pictures()
                            .lookup(request)
                            .await
                            .map(|lookup| lookup.into_found())
                        {
                            Ok(Some(found)) => {
                                picture = Some(found);
                                break 'lookup;
                            }
                            Ok(None) => {}
                            Err(error) => {
                                log::debug!("picture lookup failed: {error}");
                                failed = true;
                            }
                        }
                    }
                }
                let Some(picture) = picture else {
                    return if failed {
                        Err("lookup failed".to_owned())
                    } else {
                        Ok(None)
                    };
                };
                download_avatar(picture.url, path).await.map(Some)
            }
            .await;
            match fetched {
                Ok(path) => {
                    let _ = commands.send(Command::AvatarFetched { id, full, path });
                }
                Err(error) => {
                    log::debug!("no picture for {id} yet: {error}");
                    let _ = commands.send(Command::AvatarFailed { id, full });
                }
            }
        });
    }

    /// Retries deferred or failed profile-picture requests.
    fn retry_avatars(&mut self) {
        if !self
            .client
            .as_ref()
            .is_some_and(|client| client.is_socket_connected())
        {
            return;
        }
        let due: Vec<(String, bool)> = self.pending_avatars.keys().cloned().collect();
        for (id, full) in due {
            let attempts = self
                .pending_avatars
                .remove(&(id.clone(), full))
                .unwrap_or(0);
            if attempts >= 3 {
                self.emit(Event::Avatar {
                    id,
                    full,
                    path: None,
                });
                continue;
            }
            self.fetch_avatar(id, full);
        }
    }

    /// How many in-chat matches the pane lists. One more is asked for, so a
    /// full page can be told apart from a truncated one.
    const CHAT_SEARCH_LIMIT: usize = 80;

    /// Answers the info panel with a chat's media, documents and links, or
    /// with why they could not be listed. A listing asked for while private
    /// content is withheld waits, as a transcript page does, so the panel is
    /// answered once content is shown instead of waiting forever.
    fn load_chat_media(&mut self, chat: ChatId, request: u64) {
        use crate::model::{ChatMedia, MEDIA_LIST_LIMIT};
        if self.withhold(WithheldPage::Media(chat.clone(), request)) {
            return;
        }
        let result = match self.archive.media_docs_links(&chat, MEDIA_LIST_LIMIT + 1) {
            Ok(mut rows) => {
                for row in &mut rows {
                    self.polish(row);
                }
                Ok(ChatMedia::collect(chat, rows, MEDIA_LIST_LIMIT))
            }
            Err(error) => {
                log::warn!("could not list a chat's media: {error}");
                Err(error.to_string())
            }
        };
        self.emit(Event::ChatShared { request, result });
    }

    /// Answers the in-chat search with its matches.
    fn search_chat_messages(
        &mut self,
        chat: ChatId,
        query: String,
        from: Option<i64>,
        until: Option<i64>,
    ) {
        match self.archive.search_chat_messages(
            &chat,
            &query,
            from,
            until,
            Self::CHAT_SEARCH_LIMIT + 1,
        ) {
            Ok(mut messages) => {
                // The extra row is not shown: it is how the pane learns the
                // archive held more, so it can say the list was cut.
                let truncated = messages.len() > Self::CHAT_SEARCH_LIMIT;
                messages.truncate(Self::CHAT_SEARCH_LIMIT);
                for message in &mut messages {
                    self.polish(message);
                }
                self.emit(Event::ChatHits {
                    chat,
                    query,
                    from,
                    until,
                    messages,
                    truncated,
                });
            }
            Err(error) => self.emit(Event::Error(format!("Could not search: {error}"))),
        }
    }

    /// Loads archived messages needed to scroll to a quote.
    fn search_messages(&mut self, query: String) {
        match self.archive.search_messages(&query, 50) {
            Ok(mut messages) => {
                for message in &mut messages {
                    self.polish(message);
                }
                self.emit(Event::SearchHits { query, messages });
            }
            Err(error) => self.emit(Event::Error(format!("Could not search: {error}"))),
        }
    }

    /// Sends a page of the archive, the newest one or the one before `before`.
    fn send_page(&mut self, chat: &ChatId, before: Option<super::PageKey>) {
        if self.withhold(WithheldPage::Page(chat.clone(), before.clone())) {
            return;
        }
        match self.archive.messages(
            chat,
            before.as_ref().map(|(time, id)| (*time, id.as_str())),
            PAGE + 1,
        ) {
            Ok(mut messages) => {
                let complete = messages.len() <= PAGE;
                if !complete {
                    messages.remove(0);
                }
                for message in &mut messages {
                    self.polish(message);
                }
                self.emit(Event::Messages {
                    chat: chat.clone(),
                    messages,
                    older: before.is_some(),
                    complete,
                });
            }
            Err(error) => self.emit(Event::Error(format!("Could not read the chat: {error}"))),
        }
    }

    /// Keeps a transcript read for later while private content is withheld.
    /// Its answer would be dropped, and the interface, having asked once,
    /// would wait for it forever: a chat opened then stayed empty until a new
    /// message arrived (#180).
    fn withhold(&mut self, page: WithheldPage) -> bool {
        if self.privacy_ready {
            return false;
        }
        if !self.withheld_pages.contains(&page) {
            self.withheld_pages.push(page);
        }
        true
    }

    fn load_until(&mut self, chat: ChatId, id: String, before: super::PageKey) {
        if self.withhold(WithheldPage::Until(
            chat.clone(),
            id.clone(),
            before.clone(),
        )) {
            return;
        }
        let Ok(Some(target)) = self.archive.message(&chat, &id) else {
            self.emit(Event::HistoryUnavailable {
                chat: chat.clone(),
                error: "This message is not stored on this computer".to_owned(),
            });
            return;
        };
        match self
            .archive
            .messages_range(&chat, target.timestamp, (before.0, &before.1), 2000)
        {
            Ok(mut messages) => {
                for message in &mut messages {
                    self.polish(message);
                }
                self.emit(Event::Messages {
                    chat,
                    messages,
                    older: true,
                    complete: false,
                });
            }
            // As for a message not stored: the chat stops waiting for older
            // history, and the replied-message jump (G) waiting on this
            // request hears why.
            Err(error) => self.emit(Event::HistoryUnavailable {
                chat,
                error: format!("Could not read the chat: {error}"),
            }),
        }
    }

    /// Sends an edited body without notifying everyone for an existing mention.
    /// The interface already shows the new text; the archive takes it only
    /// once WhatsApp accepts the edit, and a refusal puts the original back.
    fn edit_text(&mut self, chat: ChatId, id: String, text: String, mentions: Vec<String>) {
        let (Some(client), Some(jid)) = (self.client.clone(), Self::jid_of(&chat)) else {
            self.emit(Event::Error("Not connected to WhatsApp".to_owned()));
            self.emit_message(&chat, &id);
            return;
        };
        let (message, content, mention_rows) =
            match self.prepare_text_edit(&chat, &id, text, mentions) {
                Ok(prepared) => prepared,
                Err(_) => {
                    self.emit(Event::Error("Could not prepare the message edit".into()));
                    self.emit_message(&chat, &id);
                    return;
                }
            };
        let generation = self.begin_edit(&chat, &id);
        let commands = self.commands.clone();
        tokio::spawn(async move {
            let accepted = client
                .edit_message_raw(
                    jid,
                    id.clone(),
                    message,
                    whatsapp_rust::EditOptions::default(),
                )
                .await
                .map(|_| (content, mention_rows))
                .map_err(|error| format!("Could not send the edit: {error}"));
            let _ = commands.send(Command::EditSettled {
                chat,
                id,
                generation,
                accepted,
            });
        });
    }

    /// Numbers an edit about to be sent, the newest for its message.
    fn begin_edit(&mut self, chat: &str, id: &str) -> u64 {
        self.edit_sequence += 1;
        self.edits
            .entry((chat.to_owned(), id.to_owned()))
            .or_default()
            .sent = self.edit_sequence;
        self.edit_sequence
    }

    /// Builds the outgoing edit, and the content and mentions the archive
    /// takes once WhatsApp accepts it. A repeated everyone mention is
    /// suppressed only in the outbound payload.
    fn prepare_text_edit(
        &mut self,
        chat: &ChatId,
        id: &str,
        text: String,
        mut mentions: Vec<String>,
    ) -> crate::archive::Result<(wa::Message, Content, Vec<MentionRef>)> {
        let already_mentioned_everyone = self
            .archive
            .message(chat, id)?
            .is_some_and(|message| message.mentions.iter().any(crate::mentions::is_everyone));
        if self.wants_mention_all(chat, &text) {
            crate::mentions::push_everyone_id(&mut mentions);
        }
        let content = Content::text(text.clone());
        let mention_rows = self.mentions_of(&mentions);
        if already_mentioned_everyone {
            mentions.retain(|id| id != crate::mentions::ALL_ID);
        }
        let mut message = outgoing_text(text, None, &mentions);
        self.apply_ephemeral(chat, &mut message);
        Ok((message, content, mention_rows))
    }

    /// Applies an accepted edit unless a newer one was applied already or the
    /// phone changed the message since. A refusal of the newest edit puts the
    /// archived text, which is what WhatsApp has, back in the bubble.
    fn edit_settled(
        &mut self,
        chat: ChatId,
        id: String,
        generation: u64,
        accepted: Result<(Content, Vec<MentionRef>), String>,
    ) {
        let edits = self.edits.get_mut(&(chat.clone(), id.clone()));
        match (accepted, edits) {
            (Ok((content, mentions)), Some(edits)) if generation > edits.applied => {
                edits.applied = generation;
                let old = self.archive.message(&chat, &id).ok().flatten();
                if let Ok(true) = self
                    .archive
                    .set_edited_content(&chat, &id, &content, &mentions)
                {
                    self.emit_message(&chat, &id);
                    self.emit_chat(&chat);
                    self.media_changed(&chat, old.as_ref().map(|old| &old.content), Some(&content));
                }
            }
            // Superseded, unlinked, or the phone edited or revoked it since.
            (Ok(_), _) => {}
            (Err(error), edits) => {
                // A newer edit still on its way settles the bubble itself.
                let newest = edits.is_some_and(|edits| edits.sent == generation);
                self.emit(Event::Error(error));
                if newest {
                    self.emit_message(&chat, &id);
                }
            }
        }
    }

    /// Sends the account deletion through whatsapp-rust before removing our copy.
    fn delete_message_for_me(&mut self, chat: ChatId, id: String) {
        if self.message_removal_in_flight(&chat, &id) {
            return;
        }
        match self.archive.message_removed(&chat, &id) {
            Ok(true) => {
                self.delete_message_here(&chat, &id);
                return;
            }
            Ok(false) => {}
            Err(_) => {
                self.emit(Event::Error(
                    "Could not read the message's deletion state".into(),
                ));
                return;
            }
        }
        let (Some(client), Some(jid)) = (self.client.clone(), Self::jid_of(&chat)) else {
            self.emit(Event::Error(
                "Connect to WhatsApp to delete this message".to_owned(),
            ));
            return;
        };
        let message = match self.archive.message(&chat, &id) {
            Ok(Some(message)) => message,
            Ok(None) => {
                if self.archive.cancel_message_removal(&chat, &id).is_err() {
                    self.emit(Event::Error("Could not clear the pending deletion".into()));
                }
                return;
            }
            Err(_) => {
                self.emit(Event::Error(
                    "Could not read the message to delete".to_owned(),
                ));
                return;
            }
        };
        let participant = if jid.is_group() && !message.from_me {
            let Some(sender) = Self::jid_of(&message.sender) else {
                self.emit(Event::Error(
                    "Could not identify the message to delete".to_owned(),
                ));
                return;
            };
            Some(sender)
        } else {
            None
        };
        let Some(account) = self.me_pn.as_deref() else {
            self.emit(Event::Error(
                "Could not identify the account for this deletion".into(),
            ));
            return;
        };
        if self
            .archive
            .queue_message_removal(account, &chat, &id)
            .is_err()
        {
            self.emit(Event::Error("Could not save the pending deletion".into()));
            return;
        }
        let generation = self.privacy_generation;
        self.message_removals_in_flight
            .insert((chat.clone(), id.clone()));
        let commands = self.commands.clone();
        tokio::spawn(async move {
            let target = whatsapp_rust::MessageId::new(&id).and_then(|message_id| {
                whatsapp_rust::MessageRef::new(
                    &jid,
                    message_id,
                    participant.as_ref(),
                    message.from_me,
                )
            });
            let outcome = match target {
                Ok(target) => message_removal_outcome(
                    client
                        .chat_actions()
                        .delete_message_for_me(&target, true, Some(message.timestamp))
                        .await,
                ),
                // Only an empty id or a group message without its author
                // fails here; it stays pending like an unconfirmed request.
                Err(_) => MessageRemovalOutcome::Uncertain,
            };
            let _ = commands.send(Command::MessageDeletedForMe {
                generation,
                chat,
                id,
                outcome,
            });
        });
    }

    /// Retries interrupted deletions after reconnect without relying on redelivery.
    fn retry_message_removals(&mut self) {
        self.retry_confirmed_message_removals();
        let Some(account) = self.me_pn.as_deref() else {
            return;
        };
        match self.archive.pending_message_removals(account) {
            Ok(pending) => {
                for (chat, id) in pending {
                    self.delete_message_for_me(chat, id);
                }
            }
            Err(_) => self.emit(Event::Error("Could not recover pending deletions".into())),
        }
    }

    /// Replays accepted work locally even offline, without relying on redelivery.
    fn retry_confirmed_message_removals(&mut self) {
        let Some(account) = self.me_pn.as_deref() else {
            return;
        };
        match self.archive.confirmed_message_removals(account) {
            Ok(pending) => {
                for (chat, id) in pending {
                    self.delete_message_here(&chat, &id);
                }
            }
            Err(_) => self.emit(Event::Error("Could not recover confirmed deletions".into())),
        }
    }

    /// Mapping can change while an attempt is running, but it is still one message.
    fn message_removal_in_flight(&self, chat: &str, id: &str) -> bool {
        let canonical = self.canonical_str(chat);
        self.message_removals_in_flight
            .iter()
            .any(|(active, message)| message == id && self.canonical_str(active) == canonical)
    }

    /// Persists an accepted deletion and wakes the UI, including for unseen messages.
    fn delete_message_here(&mut self, chat: &str, id: &str) {
        let canonical = self.canonical_str(chat);
        let mut aliases = vec![chat.to_owned(), canonical.clone()];
        if let Some(pn) = canonical.strip_suffix("@s.whatsapp.net") {
            aliases.extend(
                self.lid_to_pn
                    .iter()
                    .filter(|(_, mapped)| mapped.as_str() == pn)
                    .map(|(lid, _)| format!("{lid}@lid")),
            );
        }
        aliases.sort_unstable();
        aliases.dedup();
        let account = self.me_pn.clone().or_else(|| {
            self.client
                .as_ref()
                .and_then(|client| client.pn())
                .map(|jid| jid.to_non_ad_string())
        });
        let Some(account) = account else {
            self.emit(Event::Error(
                "Could not identify the account for the confirmed deletion".into(),
            ));
            return;
        };
        if self
            .archive
            .confirm_message_removal(&account, &aliases, id)
            .is_err()
        {
            self.emit(Event::Error(
                "Could not save the confirmed deletion for local recovery".into(),
            ));
            return;
        }
        // Whether the info panel listed it, read before the row goes.
        let listed = aliases.iter().any(|alias| {
            self.archive
                .message(alias, id)
                .ok()
                .flatten()
                .is_some_and(|old| old.content.listable())
        });
        match self.archive.delete_message_aliases(&aliases, id) {
            Ok(_) => {
                for chat in aliases {
                    self.emit(Event::MessageDeleted {
                        chat: chat.clone(),
                        id: id.to_owned(),
                    });
                    self.emit_chat(&chat);
                    if listed {
                        self.emit(Event::ChatSharedChanged(chat));
                    }
                }
            }
            Err(_) => self.emit(Event::Error(
                "WhatsApp accepted the deletion, but ZapFast could not remove its copy. Local cleanup is saved for restart or reconnect".to_owned(),
            )),
        }
    }

    fn revoke(&mut self, chat: ChatId, id: String) {
        let (Some(client), Some(jid)) = (self.client.clone(), Self::jid_of(&chat)) else {
            self.emit(Event::Error("Not connected to WhatsApp".to_owned()));
            return;
        };
        let old = self.archive.message(&chat, &id).ok().flatten();
        if let Ok(true) = self
            .archive
            .set_content(&chat, &id, &Content::Revoked, false)
        {
            self.emit_message(&chat, &id);
            self.emit_chat(&chat);
            self.media_changed(&chat, old.as_ref().map(|old| &old.content), None);
        }
        let commands = self.commands.clone();
        tokio::spawn(async move {
            if let Err(error) = client.revoke_message_raw(jid, id, RevokeType::Sender).await {
                let _ = commands.send(Command::Sent {
                    chat,
                    id: String::new(),
                    error: Some(format!(
                        "Could not delete the message for everyone: {error}"
                    )),
                });
            }
        });
    }

    /// Dispatches selected files with the first attachment carrying the caption and quote.
    fn send_files(
        &mut self,
        chat: ChatId,
        paths: Vec<PathBuf>,
        options: FileSendOptions,
        caption: Option<String>,
        mentions: Vec<String>,
        quoting: Option<crate::model::ReplyTarget>,
    ) {
        let FileSendOptions {
            documents,
            view_once,
        } = options;
        let unsent = |paths, caption| Unsent::Files {
            paths,
            documents,
            view_once,
            caption,
        };
        if view_once
            && (documents
                || paths.len() != 1
                || !paths
                    .first()
                    .is_some_and(|path| crate::app::Pending::is_view_once_photo_file(path)))
        {
            self.refuse(
                chat,
                quoting,
                unsent(paths, caption),
                Refusal::UnsupportedViewOnce,
            );
            return;
        }
        let mut quote = match self.quote_target(&chat, quoting.as_ref()) {
            Ok(quote) => quote,
            Err(reason) => {
                self.refuse(chat, quoting, unsent(paths, caption), reason);
                return;
            }
        };
        let Some(client) = self.connected_client() else {
            self.refuse(chat, quoting, unsent(paths, caption), Refusal::Offline);
            return;
        };
        let mut mentions = mentions;
        if caption
            .as_deref()
            .is_some_and(|text| self.wants_mention_all(&chat, text))
        {
            crate::mentions::push_everyone_id(&mut mentions);
        }
        for (index, path) in paths.into_iter().enumerate() {
            let client = client.clone();
            // Like the caption, the reply belongs to the first file.
            let quote = quote.take();
            let commands = self.commands.clone();
            let chat = chat.clone();
            let dir = self.dirs.media_cache_dir();
            let me = self.me();
            // Attach the caption to the first file.
            let caption = if index == 0 { caption.clone() } else { None };
            let mentions = if index == 0 {
                mentions.clone()
            } else {
                Vec::new()
            };
            tokio::spawn(async move {
                let outcome = async {
                    let bytes = tokio::fs::read(&path)
                        .await
                        .map_err(|error| format!("{}: {error}", path.display()))?;
                    let mime = mime_guess2::from_path(&path)
                        .first_or_octet_stream()
                        .to_string();
                    let file_name = path
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned());
                    let mut prepared = if documents {
                        prepare_document(&client, bytes, &mime, file_name.as_deref()).await?
                    } else {
                        prepare_media(&client, bytes, &mime, file_name.as_deref(), false).await?
                    };
                    if view_once {
                        mark_view_once_photo(&mut prepared)?;
                    }
                    quoted_outbound(
                        &client, &chat, &me, &dir, prepared, caption, mentions, quote,
                    )
                    .await
                }
                .await;
                match outcome {
                    Ok((row, raw)) => {
                        let _ = commands.send(Command::Outbound {
                            chat,
                            row: Box::new(row),
                            raw,
                        });
                    }
                    Err(error) => {
                        let _ = commands.send(Command::Sent {
                            chat,
                            id: String::new(),
                            error: Some(format!("Could not send the file: {error}")),
                        });
                    }
                }
            });
        }
    }

    /// Encodes and dispatches pasted image pixels while preserving the quote for a refused send.
    #[allow(clippy::too_many_arguments)]
    fn send_pasted_image(
        &mut self,
        chat: ChatId,
        width: u32,
        height: u32,
        rgba: Vec<u8>,
        view_once: bool,
        caption: Option<String>,
        mentions: Vec<String>,
        quoting: Option<crate::model::ReplyTarget>,
    ) {
        let unsent = |rgba, caption| Unsent::Image {
            width,
            height,
            rgba,
            view_once,
            caption,
        };
        let quote = match self.quote_target(&chat, quoting.as_ref()) {
            Ok(quote) => quote,
            Err(reason) => {
                self.refuse(chat, quoting, unsent(rgba, caption), reason);
                return;
            }
        };
        let Some(client) = self.connected_client() else {
            self.refuse(chat, quoting, unsent(rgba, caption), Refusal::Offline);
            return;
        };
        let mut mentions = mentions;
        if caption
            .as_deref()
            .is_some_and(|text| self.wants_mention_all(&chat, text))
        {
            crate::mentions::push_everyone_id(&mut mentions);
        }
        let commands = self.commands.clone();
        let dir = self.dirs.media_cache_dir();
        let me = self.me();
        tokio::spawn(async move {
            let outcome = async {
                let encoded = tokio::task::spawn_blocking(move || {
                    let image = image::RgbaImage::from_raw(width, height, rgba)
                        .ok_or_else(|| "Clipboard image data is invalid".to_owned())?;
                    encode_jpeg(&image::DynamicImage::ImageRgba8(image), 88)
                })
                .await
                .map_err(|error| error.to_string())??;
                let mut prepared =
                    prepare_media(&client, encoded, "image/jpeg", None, false).await?;
                if view_once {
                    mark_view_once_photo(&mut prepared)?;
                }
                quoted_outbound(
                    &client, &chat, &me, &dir, prepared, caption, mentions, quote,
                )
                .await
            }
            .await;
            match outcome {
                Ok((row, raw)) => {
                    let _ = commands.send(Command::Outbound {
                        chat,
                        row: Box::new(row),
                        raw,
                    });
                }
                Err(error) => {
                    let _ = commands.send(Command::Sent {
                        chat,
                        id: String::new(),
                        error: Some(format!("Could not send the picture: {error}")),
                    });
                }
            }
        });
    }

    /// Encodes and sends an OGG/Opus voice message with optional quote.
    fn send_voice(
        &mut self,
        chat: ChatId,
        samples: Vec<f32>,
        quoting: Option<crate::model::ReplyTarget>,
    ) {
        let (context, shown) = match self.quote_target(&chat, quoting.as_ref()) {
            Ok(Some((context, shown))) => (Some(Box::new(context)), Some(shown)),
            Ok(None) => (None, None),
            Err(reason) => {
                self.refuse(chat, quoting, Unsent::Voice(samples), reason);
                return;
            }
        };
        let Some(client) = self.connected_client() else {
            self.refuse(chat, quoting, Unsent::Voice(samples), Refusal::Offline);
            return;
        };
        let commands = self.commands.clone();
        let dir = self.dirs.media_cache_dir();
        let me = self.me();
        tokio::spawn(async move {
            let outcome = async {
                let (bytes, seconds, waveform) = tokio::task::spawn_blocking(move || {
                    let mut samples = samples;
                    crate::voice::normalize(&mut samples);
                    let seconds = (samples.len() as f64 / f64::from(crate::voice::RATE))
                        .round()
                        .max(1.0) as u32;
                    let waveform = crate::voice::waveform(&samples);
                    crate::voice::encode(&samples).map(|bytes| (bytes, seconds, waveform))
                })
                .await
                .map_err(|error| error.to_string())??;
                let prepared = prepare_voice(&client, bytes, seconds, waveform, context).await?;
                file_outbound(&client, &chat, &me, &dir, prepared, None, Vec::new()).await
            }
            .await;
            match outcome {
                Ok((mut row, raw)) => {
                    row.quoted = shown;
                    let _ = commands.send(Command::Outbound {
                        chat,
                        row: Box::new(row),
                        raw,
                    });
                }
                Err(error) => {
                    let _ = commands.send(Command::Sent {
                        chat,
                        id: String::new(),
                        error: Some(format!("Could not send the voice message: {error}")),
                    });
                }
            }
        });
    }

    /// Sends a played receipt for an incoming voice message.
    fn mark_played(&mut self, chat: ChatId, message: String, sender: String) {
        let (Some(client), Some(jid)) = (self.client.clone(), Self::jid_of(&chat)) else {
            return;
        };
        let sender = if jid.is_group() {
            sender.parse::<Jid>().ok()
        } else {
            None
        };
        let commands = self.commands.clone();
        tokio::spawn(async move {
            if !played_receipt_allowed(&client, &jid, &commands).await {
                return;
            }
            if let Err(error) = client
                .mark_as_played(&jid, sender.as_ref(), &[message.as_str()])
                .await
            {
                log::debug!("played receipt not sent: {error}");
            }
        });
    }

    /// Dispatches a sticker with its explicit quote and returns both on refusal.
    fn send_sticker(
        &mut self,
        chat: ChatId,
        path: PathBuf,
        quoting: Option<crate::model::ReplyTarget>,
    ) {
        let quote = match self.quote_target(&chat, quoting.as_ref()) {
            Ok(quote) => quote,
            Err(reason) => {
                self.refuse(chat, quoting, Unsent::Sticker, reason);
                return;
            }
        };
        let Some(client) = self.connected_client() else {
            self.refuse(chat, quoting, Unsent::Sticker, Refusal::Offline);
            return;
        };
        let commands = self.commands.clone();
        let dir = self.dirs.media_cache_dir();
        let me = self.me();
        tokio::spawn(async move {
            let outcome = async {
                let bytes = tokio::fs::read(&path)
                    .await
                    .map_err(|error| error.to_string())?;
                let prepared = prepare_sticker(&client, bytes).await?;
                quoted_outbound(&client, &chat, &me, &dir, prepared, None, Vec::new(), quote).await
            }
            .await;
            match outcome {
                Ok((row, raw)) => {
                    let _ = commands.send(Command::Outbound {
                        chat,
                        row: Box::new(row),
                        raw,
                    });
                }
                Err(error) => {
                    let _ = commands.send(Command::Sent {
                        chat,
                        id: String::new(),
                        error: Some(format!("Could not send the sticker: {error}")),
                    });
                }
            }
        });
    }

    /// Dispatches a selected GIF with its explicit quote and returns both on refusal.
    fn send_gif(&mut self, chat: ChatId, gif: Gif, quoting: Option<crate::model::ReplyTarget>) {
        let quote = match self.quote_target(&chat, quoting.as_ref()) {
            Ok(quote) => quote,
            Err(reason) => {
                self.refuse(chat, quoting, Unsent::Gif, reason);
                return;
            }
        };
        let Some(client) = self.connected_client() else {
            self.refuse(chat, quoting, Unsent::Gif, Refusal::Offline);
            return;
        };
        let commands = self.commands.clone();
        let dir = self.dirs.media_cache_dir();
        let me = self.me();
        tokio::spawn(async move {
            let outcome = async {
                let url = gif.mp4.clone();
                let bytes = tokio::task::spawn_blocking(move || {
                    crate::proxy::agent()
                        .get(&url)
                        .call()
                        .and_then(|mut response| response.body_mut().read_to_vec())
                        .map_err(|error| error.to_string())
                })
                .await
                .map_err(|error| error.to_string())??;
                let mut prepared = prepare_media(&client, bytes, "video/mp4", None, true).await?;
                if let Content::Video { media, .. } = &mut prepared.content {
                    media.width = Some(gif.width);
                    media.height = Some(gif.height);
                }
                if let Some(video) = prepared.message.video_message.as_option_mut() {
                    video.width = Some(gif.width);
                    video.height = Some(gif.height);
                }
                quoted_outbound(&client, &chat, &me, &dir, prepared, None, Vec::new(), quote).await
            }
            .await;
            match outcome {
                Ok((row, raw)) => {
                    let _ = commands.send(Command::Outbound {
                        chat,
                        row: Box::new(row),
                        raw,
                    });
                }
                Err(error) => {
                    let _ = commands.send(Command::Sent {
                        chat,
                        id: String::new(),
                        error: Some(format!("Could not send the GIF: {error}")),
                    });
                }
            }
        });
    }

    /// Archives and sends an uploaded attachment message.
    fn outbound(&mut self, chat: ChatId, row: Message, raw: Vec<u8>) {
        let (Some(client), Some(jid)) = (self.client.clone(), Self::jid_of(&chat)) else {
            self.emit(Event::Error("Not connected to WhatsApp".to_owned()));
            return;
        };
        let Ok(mut message) = wa::Message::decode_from_slice(&raw) else {
            self.emit(Event::Error("Could not encode the attachment".to_owned()));
            return;
        };
        let expiration = self.apply_ephemeral(&chat, &mut message);
        let raw = attachment_archive_raw(&message);
        let id = row.id.clone();
        self.store_message(row, Some(raw), None);
        tokio::spawn(send_outgoing(
            client,
            self.commands.clone(),
            chat,
            jid,
            id,
            message,
            expiration,
        ));
    }

    fn react(&mut self, chat: ChatId, id: String, emoji: String) {
        let (Some(client), Some(jid)) = (self.client.clone(), Self::jid_of(&chat)) else {
            self.emit(Event::Error("Not connected to WhatsApp".to_owned()));
            return;
        };
        let Ok(Some(target)) = self.archive.message(&chat, &id) else {
            return;
        };
        let me = self.me();
        if let Ok(Some(updated)) = self.archive.set_reaction(&chat, &id, &me, true, &emoji) {
            self.emit(Event::MessageUpdated(Box::new(updated)));
        }
        let key = {
            let mut proto = wa::MessageKey::default();
            proto.remote_jid = Some(chat.clone());
            proto.from_me = Some(target.from_me);
            proto.id = Some(id);
            proto.participant = (jid.is_group() && !target.from_me).then(|| target.sender.clone());
            proto
        };
        tokio::spawn(async move {
            if client.send_reaction_raw(jid, key, &emoji).await.is_err() {
                log::warn!("could not send a reaction");
            }
        });
    }

    /// Pins or unpins one message for everyone in the chat, for seven days.
    /// The archive is written only once WhatsApp answers, so a request that
    /// never reaches the phone leaves nothing behind.
    fn next_pin_generation(&mut self, chat: &str, id: &str) -> u64 {
        let slot = self
            .pin_generation
            .entry((chat.to_owned(), id.to_owned()))
            .or_insert(0);
        *slot = slot.wrapping_add(1);
        *slot
    }

    fn set_pin(&mut self, chat: ChatId, id: String, pinned: bool) {
        let generation = self.next_pin_generation(&chat, &id);
        let session = self.star_session;
        let commands = self.commands.clone();
        let now = crate::util::now();
        if pinned && self.archive.pin_full(&chat, &id, now).unwrap_or(true) {
            self.emit(Event::Error(
                "This chat already has three pinned messages".to_owned(),
            ));
            return;
        }
        let (Some(client), Some(jid)) = (self.client.clone(), Self::jid_of(&chat)) else {
            let _ = commands.send(Command::MessagePinned {
                chat,
                message: id,
                pinned,
                at: now,
                generation,
                session,
                expires_at: now + PIN_SECS,
                result: Err("Not connected to WhatsApp".to_owned()),
            });
            return;
        };
        let Ok(Some(target)) = self.archive.message(&chat, &id) else {
            let _ = commands.send(Command::MessagePinned {
                chat,
                message: id,
                pinned,
                at: now,
                generation,
                session,
                expires_at: now + PIN_SECS,
                result: Err("This message is not stored on this computer".to_owned()),
            });
            return;
        };
        let from_me = target.from_me;
        let participant = (jid.is_group() && !from_me)
            .then(|| target.sender.clone())
            .and_then(|sender| Self::jid_of(&sender));
        let expires_at = now + PIN_SECS;
        tokio::spawn(async move {
            let target = whatsapp_rust::MessageId::new(&id).and_then(|message_id| {
                whatsapp_rust::MessageRef::new(&jid, message_id, participant.as_ref(), from_me)
            });
            let result = match target {
                Ok(target) if pinned => client
                    .pin_message(&target, PinDuration::Days7)
                    .await
                    .map(|_| ())
                    .map_err(|error| error.to_string()),
                Ok(target) => client
                    .unpin_message(&target)
                    .await
                    .map(|_| ())
                    .map_err(|error| error.to_string()),
                // Only an empty id or a group message without its author fails here.
                Err(error) => Err(error.to_string()),
            };
            let _ = commands.send(Command::MessagePinned {
                chat,
                message: id,
                pinned,
                at: now,
                generation,
                session,
                expires_at,
                result,
            });
        });
    }

    /// Stores a pin or an unpin that arrived as a message of its own, from
    /// this account's other device or from anyone in the chat.
    fn ingest_pin(
        &mut self,
        chat: &str,
        pin: &wa::message::PinInChatMessage,
        base: &wa::Message,
        at: i64,
        sender: &str,
        from_me: bool,
    ) {
        let duration = base
            .message_context_info
            .as_option()
            .and_then(|context| context.message_add_on_duration_in_secs)
            .unwrap_or(0);
        let Some((id, pinned, duration)) = pin_action(pin, duration) else {
            return;
        };
        self.next_pin_generation(chat, &id);
        let by = crate::archive::Pinner {
            by_me: from_me,
            sender: sender.to_owned(),
        };
        let written = if pinned {
            let secs = if duration == 0 {
                PIN_SECS
            } else {
                i64::from(duration)
            };
            self.archive
                .pin_from(chat, &id, at, at.saturating_add(secs), &by)
        } else {
            // The notice names whoever made it, so an unpin keeps it too.
            self.archive.set_pin(chat, &id, false, at, 0, &by)
        };
        match written {
            Ok(true) => self.emit_pins(chat),
            Ok(false) => {}
            Err(error) => self.emit(Event::Error(error.to_string())),
        }
    }

    /// Hands one chat's active pins to the interface.
    fn emit_pins(&self, chat: &str) {
        match self.archive.chat_pins(chat, crate::util::now()) {
            Ok(items) => self.emit(Event::Pins {
                chat: chat.to_owned(),
                items,
            }),
            Err(error) => self.emit(Event::Error(error.to_string())),
        }
    }
}

// --- free helpers ----------------------------------------------------------

fn outgoing_forward(original: &wa::Message, expiration: Option<u32>) -> (wa::Message, Option<u32>) {
    let mut message = original.get_base_message().prepare_for_forward();
    if let Some(mut context) = context_of(&message).cloned() {
        // A forward belongs to the destination chat. The library retains the
        // source timer, including when the destination has no timer at all.
        context.expiration = None;
        context.ephemeral_setting_timestamp = None;
        context.ephemeral_shared_secret = None;
        // A forward keeps the visible `@all` and drops the notify bit, so the
        // destination chat is not mentioned. Other bits stay.
        silence_everyone(&mut context);
        message.set_context_info(context);
    }
    let expiration = apply_ephemeral_expiration(&mut message, expiration);
    (message, expiration)
}

fn apply_ephemeral_expiration(message: &mut wa::Message, expiration: Option<u32>) -> Option<u32> {
    let expiration = expiration.filter(|expiration| *expiration > 0)?;
    message
        .set_ephemeral_expiration(expiration)
        .then_some(expiration)
}

async fn send_outgoing(
    client: Arc<Client>,
    commands: mpsc::UnboundedSender<Command>,
    chat: ChatId,
    jid: Jid,
    id: String,
    message: wa::Message,
    ephemeral_expiration: Option<u32>,
) {
    let result = async {
        if jid.is_group() {
            // Uses whatsapp-rust's send cache; only a miss queries the server,
            // exactly as encryption would. No separate burst of metadata queries.
            let group = client
                .groups()
                .routing_info(&jid)
                .await
                .map_err(|error| error.to_string())?;
            let lids = group
                .participants
                .iter()
                .filter(|jid| jid.is_lid())
                .filter_map(|lid| {
                    group
                        .phone_jid_for_lid_user(lid.user_base())
                        .map(|pn| (lid.user_base().to_owned(), pn.user_base().to_owned()))
                })
                .collect();
            let recipients = group
                .participants
                .iter()
                .map(Jid::to_non_ad_string)
                .collect();
            let (stored, mut saved) = mpsc::unbounded_channel();
            commands
                .send(Command::GroupRecipients {
                    chat: chat.clone(),
                    id: id.clone(),
                    recipients,
                    lids,
                    stored,
                })
                .map_err(|_| "The application is shutting down".to_owned())?;
            if saved.recv().await != Some(true) {
                return Err("Could not save the group message recipients".to_owned());
            }
        }
        let message_id = whatsapp_rust::MessageId::new(&id).map_err(|error| error.to_string())?;
        let mut options = SendOptions::default().with_message_id(message_id);
        if let Some(expiration) = ephemeral_expiration {
            options = options.with_ephemeral_expiration(expiration);
        }
        client
            .send(whatsapp_rust::SendRequest::new(&jid, message).with_options(options))
            .await
            .map_err(|error| error.to_string())?;
        Ok(())
    }
    .await;
    let _ = commands.send(Command::Sent {
        chat,
        id,
        error: result.err(),
    });
}

fn forwarded_row(
    mut source: Message,
    chat: ChatId,
    sender: String,
    id: String,
    timestamp: i64,
    mentions: Vec<MentionRef>,
    thumbnail: Option<Vec<u8>>,
) -> Message {
    source.id = id;
    source.chat = chat;
    source.sender = sender;
    source.sender_name = None;
    source.from_me = true;
    source.timestamp = timestamp;
    source.status = Delivery::Pending;
    source.delivered_at = None;
    source.read_at = None;
    source.quoted = None;
    source.reactions.clear();
    source.edited = false;
    source.mentions = mentions;
    source.forwarded = true;
    source.thumbnail = thumbnail;
    source
}

/// Fallback chat name from a phone number or bare id.
fn fallback_name(id: &str) -> String {
    match crate::model::phone_of(id) {
        Some(digits) => crate::util::phone(digits),
        None if ChatKind::from_id(id) == ChatKind::Group => String::new(),
        None => id.split('@').next().unwrap_or(id).to_owned(),
    }
}

/// Whether a chat's stored name outranks one rebuilt from contacts: a group's
/// subject, or a channel's name from the server. A channel still showing its
/// bare id may be renamed.
fn keeps_its_name(chat: &Chat) -> bool {
    chat.kind == ChatKind::Group || (chat.is_channel() && chat.name != fallback_name(&chat.id))
}

/// Normalizes WhatsApp timestamps to seconds.
/// Where a deleted or cleared chat ends: the last message the deleting device
/// knew about, or the moment of the action when it sent no message range.
fn removal_point(last_message: Option<i64>, action: i64) -> i64 {
    last_message
        .filter(|timestamp| *timestamp > 0)
        .map_or(action, seconds)
}

fn seconds(timestamp: i64) -> i64 {
    if timestamp > 100_000_000_000 {
        timestamp / 1000
    } else {
        timestamp.max(0)
    }
}

fn sanitize(id: &str) -> String {
    id.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

/// Reject the whole extension instead of repairing path syntax or truncating it.
/// This alphabet is safe on both Unix and Windows, regardless of the host OS.
fn safe_extension(extension: &str) -> String {
    if extension.is_empty()
        || extension.len() > 16
        || !extension
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return "bin".to_owned();
    }
    extension.to_ascii_lowercase()
}

fn extension_for(mime: &str, file_name: Option<&str>) -> String {
    // Inspect the complete suffix, without host-specific Path parsing that could
    // discard separators within it. The rest of the name is sanitized separately.
    if let Some(extension) = file_name
        .and_then(|name| name.rsplit_once('.'))
        .map(|(_, extension)| extension)
    {
        return safe_extension(extension);
    }
    let mime = mime.split(';').next().unwrap_or(mime);
    safe_extension(match mime {
        "image/jpeg" => "jpg",
        "image/png" => "png",
        "image/webp" => "webp",
        "image/gif" => "gif",
        "video/mp4" => "mp4",
        "video/3gpp" => "3gp",
        "audio/ogg" => "ogg",
        "audio/mpeg" => "mp3",
        "audio/mp4" => "m4a",
        "audio/aac" => "aac",
        "audio/wav" => "wav",
        "application/pdf" => "pdf",
        "text/plain" => "txt",
        // Split at the first slash so extra slashes remain and are rejected.
        _ => mime.split_once('/').map_or("bin", |(_, subtype)| subtype),
    })
}

/// A document's own name in the reader's download folder, made safe for
/// every platform's file system: no path, no characters Windows refuses, no
/// device names, no leading dot, at most 120 characters before a validated
/// extension. `None` when nothing of the name is left.
fn named_media_path(dir: &Path, mime: &str, file_name: &str) -> Option<PathBuf> {
    let extension = extension_for(mime, Some(file_name));
    // Only the last component of whatever path the sender's name holds.
    let name = file_name.rsplit(['/', '\\']).next().unwrap_or_default();
    let stem = name.rsplit_once('.').map_or(name, |(stem, _)| stem);
    let stem: String = stem
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') {
                '_'
            } else {
                c
            }
        })
        .take(120)
        .collect();
    let stem = stem
        .trim_start_matches(['.', ' '])
        .trim_end_matches(['.', ' ']);
    if stem.is_empty() {
        return None;
    }
    let device = stem.split('.').next().unwrap_or(stem).to_ascii_uppercase();
    let reserved = matches!(device.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((device.starts_with("COM") || device.starts_with("LPT"))
            && device.len() == 4
            && device[3..].chars().all(|c| c.is_ascii_digit()));
    let stem = if reserved {
        format!("_{stem}")
    } else {
        stem.to_owned()
    };
    Some(dir.join(format!("{stem}.{extension}")))
}

fn media_path(dir: &Path, chat: &str, id: &str, mime: &str, file_name: Option<&str>) -> PathBuf {
    let extension = extension_for(mime, file_name);
    let stem = match file_name.and_then(|name| Path::new(name).file_stem()?.to_str()) {
        Some(name) => format!("{}-{}", sanitize(id), sanitize(name)),
        None => format!("{}-{}", sanitize(chat), sanitize(id)),
    };
    // The stem contains only ASCII alphanumerics, '_' and '-', and the validated
    // extension only alphanumerics and '-'. The single literal dot cannot create
    // a path component, drive prefix or alternate data stream on either OS.
    dir.join(format!("{stem}.{extension}"))
}

/// Reserve each destination atomically, including when another save is racing
/// us. Checking existence before fs::copy could still overwrite a user's file.
fn save_album_copy(source: &Path, folder: &Path, name: &str) -> std::io::Result<PathBuf> {
    use std::io::ErrorKind;
    let name = Path::new(name)
        .file_name()
        .ok_or_else(|| std::io::Error::new(ErrorKind::InvalidInput, "Invalid attachment name"))?;
    let path = Path::new(name);
    let stem = path.file_stem().unwrap_or(name).to_string_lossy();
    let extension = path
        .extension()
        .map(|value| format!(".{}", value.to_string_lossy()))
        .unwrap_or_default();
    let mut input = std::fs::File::open(source)?;
    let mut number = 1usize;
    loop {
        let target = if number == 1 {
            folder.join(name)
        } else {
            folder.join(format!("{stem} ({number}){extension}"))
        };
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)
        {
            Ok(mut output) => {
                if let Err(error) = std::io::copy(&mut input, &mut output) {
                    drop(output);
                    let _ = std::fs::remove_file(&target);
                    return Err(error);
                }
                return Ok(target);
            }
            Err(error) if error.kind() == ErrorKind::AlreadyExists => number += 1,
            Err(error) => return Err(error),
        }
    }
}

/// Converts optional protocol dimensions and file size into local attachment metadata.
fn media(
    mime: Option<&String>,
    size: Option<u64>,
    width: Option<u32>,
    height: Option<u32>,
) -> Media {
    Media {
        mime: mime.cloned().unwrap_or_default(),
        size: size.unwrap_or(0),
        width,
        height,
        album: None,
        path: None,
        state: Default::default(),
    }
}

fn non_empty(text: &Option<String>) -> Option<String> {
    text.clone().filter(|text| !text.trim().is_empty())
}

/// Builds a text body with optional quote and mention context.
///
/// [`crate::mentions::ALL_ID`] in `mentions` sets the everyone bit and is not
/// written as a participant JID.
fn outgoing_text(
    text: String,
    mut context: Option<wa::ContextInfo>,
    mentions: &[String],
) -> wa::Message {
    let (jids, mention_all) = participant_mentions(mentions);
    if !jids.is_empty() || mention_all {
        let ctx = context.get_or_insert_default();
        if !jids.is_empty() {
            ctx.mentioned_jid = jids;
        }
        if mention_all {
            ctx.non_jid_mentions = Some(crate::mentions::with_flag(ctx.non_jid_mentions));
        }
    }
    match context {
        Some(context) => wa::Message::text_with_context(text, context),
        None => wa::Message::text(text),
    }
}

/// JIDs to put in `mentioned_jid`, and whether everyone should be notified.
fn participant_mentions(mentions: &[String]) -> (Vec<String>, bool) {
    let mention_all = mentions.iter().any(|id| id == crate::mentions::ALL_ID);
    let jids = mentions
        .iter()
        .filter(|id| id.as_str() != crate::mentions::ALL_ID)
        .cloned()
        .collect();
    (jids, mention_all)
}

/// Drops the everyone notify bit from a context and from the message it quotes.
fn silence_everyone(context: &mut wa::ContextInfo) {
    context.non_jid_mentions = crate::mentions::without_flag(context.non_jid_mentions);
    let Some(quoted) = context.quoted_message.as_option_mut() else {
        return;
    };
    // Quotes may retain protocol wrappers, while set_context_info operates on
    // the core body. Keep that body when replacing the quoted copy.
    let mut base = quoted.get_base_message().clone();
    let Some(mut inner) = context_of(&base).cloned() else {
        return;
    };
    let silenced = crate::mentions::without_flag(inner.non_jid_mentions);
    if silenced == inner.non_jid_mentions {
        return;
    }
    inner.non_jid_mentions = silenced;
    if base.set_context_info(inner) {
        *quoted = base;
    }
}

/// A location message: the coordinates and nothing else, because a computer
/// has no place name to send. A quote rides inside the location, where a
/// phone puts it.
fn outgoing_location(
    latitude: f64,
    longitude: f64,
    context: Option<wa::ContextInfo>,
) -> wa::Message {
    let mut message = wa::Message::default();
    let mut location = wa::message::LocationMessage::default();
    location.degrees_latitude = Some(latitude);
    location.degrees_longitude = Some(longitude);
    message.location_message = MessageField::some(location);
    if let Some(context) = context {
        let _ = message.set_context_info(context);
    }
    message
}

/// Extracts quote and mention context from a message.
fn context_of(base: &wa::Message) -> Option<&wa::ContextInfo> {
    if let Some(inner) = bot_replies::invoked(base) {
        return context_of(inner);
    }
    if let Some(reply) = base.rich_response_message.as_option() {
        return reply.context_info.as_option();
    }
    if let Some(text) = base.extended_text_message.as_option() {
        return text.context_info.as_option();
    }
    if let Some(image) = base.image_message.as_option() {
        return image.context_info.as_option();
    }
    if let Some(video) = base
        .video_message
        .as_option()
        .or(base.ptv_message.as_option())
    {
        return video.context_info.as_option();
    }
    if let Some(audio) = base.audio_message.as_option() {
        return audio.context_info.as_option();
    }
    if let Some(document) = base.document_message.as_option() {
        return document.context_info.as_option();
    }
    if let Some(sticker) = base.sticker_message.as_option() {
        return sticker.context_info.as_option();
    }
    if let Some(location) = base.location_message.as_option() {
        return location.context_info.as_option();
    }
    if let Some(location) = base.live_location_message.as_option() {
        return location.context_info.as_option();
    }
    if let Some(contact) = base.contact_message.as_option() {
        return contact.context_info.as_option();
    }
    if let Some(contacts) = base.contacts_array_message.as_option() {
        return contacts.context_info.as_option();
    }
    if let Some(poll) = base
        .poll_creation_message
        .as_option()
        .or(base.poll_creation_message_v2.as_option())
        .or(base.poll_creation_message_v3.as_option())
    {
        return poll.context_info.as_option();
    }
    interactive::context(base)
}

/// Returns raw JIDs mentioned by a message.
fn mentioned_of(base: &wa::Message) -> Vec<String> {
    context_of(base)
        .map(|context| context.mentioned_jid.clone())
        .unwrap_or_default()
}

/// Participant JIDs plus [`crate::mentions::ALL_ID`] when everyone was notified.
fn recorded_mention_ids(base: &wa::Message) -> Vec<String> {
    let mut ids = mentioned_of(base);
    if crate::mentions::flagged(context_of(base).and_then(|context| context.non_jid_mentions)) {
        crate::mentions::push_everyone_id(&mut ids);
    }
    ids
}

/// Whether WhatsApp shows this message as forwarded many times. whatsapp-rust
/// raises the score of a forward to its frequently-forwarded sentinel (127)
/// once it reaches 5, the threshold WhatsApp Web uses.
fn frequently_forwarded(base: &wa::Message) -> bool {
    const FREQUENTLY_FORWARDED_SCORE: u32 = 5;
    context_of(base)
        .and_then(|context| context.forwarding_score)
        .is_some_and(|score| score >= FREQUENTLY_FORWARDED_SCORE)
}

fn forwarded_of(base: &wa::Message) -> bool {
    context_of(base).is_some_and(|context| {
        context.is_forwarded.unwrap_or(false) || context.forwarding_score.unwrap_or(0) > 0
    })
}

/// Finds the first web address when preview metadata omits its URL.
fn first_link(text: &str) -> Option<String> {
    text.split_whitespace()
        .find(|token| token.starts_with("http://") || token.starts_with("https://"))
        .map(|token| token.trim_end_matches(['.', ',', ')', ']']).to_owned())
}

/// Extracts the attachment or link-preview thumbnail.
fn thumbnail_of(base: &wa::Message) -> Option<Vec<u8>> {
    let bytes = if let Some(image) = base.image_message.as_option() {
        image.jpeg_thumbnail.clone()
    } else if let Some(video) = base
        .video_message
        .as_option()
        .or(base.ptv_message.as_option())
    {
        video.jpeg_thumbnail.clone()
    } else if let Some(document) = base.document_message.as_option() {
        document.jpeg_thumbnail.clone()
    } else if let Some(location) = base.location_message.as_option() {
        location.jpeg_thumbnail.clone()
    } else if let Some(live) = base.live_location_message.as_option() {
        live.jpeg_thumbnail.clone()
    } else if let Some(text) = base.extended_text_message.as_option() {
        text.jpeg_thumbnail.clone()
    } else {
        interactive::image(base).and_then(|image| image.jpeg_thumbnail.clone())
    };
    bytes.filter(|bytes| !bytes.is_empty())
}

/// Builds the live location content for a message, plus the id of the
/// message it quotes, which may be the start of the share it continues.
fn live_location_of(base: &wa::Message) -> Option<(Content, Option<String>)> {
    if let Some(live) = base.live_location_message.as_option() {
        let reference = live
            .context_info
            .as_option()
            .and_then(|context| context.stanza_id.clone())
            .filter(|id| !id.is_empty());
        return Some((live_location_content(live, false), reference));
    }
    if let Some(location) = base.location_message.as_option()
        && location.is_live == Some(true)
    {
        return Some((
            Content::LiveLocation {
                latitude: location.degrees_latitude.unwrap_or(0.0),
                longitude: location.degrees_longitude.unwrap_or(0.0),
                accuracy_m: location.accuracy_in_meters,
                speed_mps: location.speed_in_mps,
                heading_deg: location.degrees_clockwise_from_magnetic_north,
                sequence: 0,
                ended: false,
                updated: 0,
                newer_on_phone: false,
            },
            None,
        ));
    }
    None
}

fn live_location_content(live: &wa::message::LiveLocationMessage, ended: bool) -> Content {
    Content::LiveLocation {
        latitude: live.degrees_latitude.unwrap_or(0.0),
        longitude: live.degrees_longitude.unwrap_or(0.0),
        accuracy_m: live.accuracy_in_meters,
        speed_mps: live.speed_in_mps,
        heading_deg: live.degrees_clockwise_from_magnetic_north,
        sequence: live.sequence_number.unwrap_or(0),
        ended,
        updated: 0,
        newer_on_phone: false,
    }
}

/// Carries a stored live location's notice about the positions the phone
/// keeps to itself into a replayed classification of the same message. A
/// replay brings no newer readable position, so the notice stands; only an
/// advancing position or a share the phone reports as finished takes it back.
fn keep_live_location_notice(incoming: &mut Content, existing: &Message) {
    if !matches!(incoming, Content::LiveLocation { .. }) {
        return;
    }
    let stored = matches!(
        existing.content,
        Content::LiveLocation {
            newer_on_phone: true,
            ..
        }
    );
    // `live_location_newer` needs the share, so decide before borrowing the
    // incoming content's own fields.
    let advances = live_location_newer(existing, incoming);
    let ended = matches!(incoming, Content::LiveLocation { ended: true, .. });
    let Content::LiveLocation { newer_on_phone, .. } = incoming else {
        return;
    };
    *newer_on_phone = stored && !advances && !ended;
}

/// The last position of a share that history reports as finished.
fn finished_live_location(last: &wa::message::LiveLocationMessage, sent: i64) -> Content {
    let mut content = live_location_content(last, true);
    if let Content::LiveLocation { updated, .. } = &mut content {
        *updated = sent + i64::from(last.time_offset.unwrap_or(0));
    }
    content
}

/// How long a sender's live location may go without a position before a
/// position that names no message starts a new share instead of moving it.
const LIVE_LOCATION_GAP: i64 = 15 * 60;

/// Unix seconds of a stored live location's latest position.
fn live_location_time(message: &Message) -> i64 {
    match message.content {
        Content::LiveLocation { updated, .. } if updated > 0 => updated,
        _ => message.timestamp,
    }
}

/// Whether `incoming` moves the stored live location `share` forward. An
/// ended share takes no more positions. Sequence numbers order positions
/// when both carry one, and arrival time orders the rest.
fn live_location_newer(share: &Message, incoming: &Content) -> bool {
    let (
        Content::LiveLocation {
            sequence: old,
            ended,
            ..
        },
        Content::LiveLocation {
            sequence: new,
            updated,
            ..
        },
    ) = (&share.content, incoming)
    else {
        return false;
    };
    if *ended {
        false
    } else if *old > 0 && *new > 0 {
        new > old
    } else {
        *updated > live_location_time(share)
    }
}

/// Converts a protocol message to visible content, or `None` for internal
/// traffic. Takes the whole message, wrappers included: view-once media
/// arrives wrapped, and WhatsApp keeps it for the phone, so a linked device
/// shows a placeholder instead of an attachment that cannot be fetched.
fn classify(message: &wa::Message) -> Option<Content> {
    let content = classify_base(message.get_base_message())?;
    // Interactive messages travel in the same wrapper, so only media turns
    // into a placeholder.
    if message.is_view_once()
        && let Some(kind) = crate::model::OnceMedia::of(&content)
    {
        return Some(Content::PhoneOnly {
            view_once: true,
            live_location: false,
            once: Some(kind),
        });
    }
    Some(content)
}

/// [`classify`] for a message with its wrappers already removed.
fn classify_base(base: &wa::Message) -> Option<Content> {
    if let Some(inner) = bot_replies::invoked(base) {
        return classify_base(inner);
    }
    if let Some(text) = base.text_content() {
        let preview = base.extended_text_message.as_option().and_then(|extended| {
            let title = non_empty(&extended.title);
            let description = non_empty(&extended.description);
            let has_picture = extended
                .jpeg_thumbnail
                .as_ref()
                .is_some_and(|bytes| !bytes.is_empty());
            if title.is_none() && description.is_none() && !has_picture {
                return None;
            }
            let url = non_empty(&extended.matched_text)
                .and_then(|url| crate::safety::preview_url(&url))
                .or_else(|| first_link(text).and_then(|url| crate::safety::preview_url(&url)))?;
            Some(LinkPreview {
                url,
                title,
                description,
            })
        });
        return Some(Content::Text {
            text: text.to_owned(),
            preview,
        });
    }
    if let Some(image) = base.image_message.as_option() {
        let parent = image
            .context_info
            .as_option()
            .and_then(|context| context.paired_media_type)
            == Some(wa::context_info::PairedMediaType::MOTION_PHOTO_PARENT);
        return Some(Content::Image {
            motion: parent.then(Default::default),
            caption: non_empty(&image.caption),
            media: album_media(
                base,
                media(
                    image.mimetype.as_ref(),
                    image.file_length,
                    image.width,
                    image.height,
                ),
            ),
        });
    }
    if let Some(video) = base
        .video_message
        .as_option()
        .or(base.ptv_message.as_option())
    {
        return Some(Content::Video {
            caption: non_empty(&video.caption),
            media: album_media(
                base,
                media(
                    video.mimetype.as_ref(),
                    video.file_length,
                    video.width,
                    video.height,
                ),
            ),
            seconds: video.seconds,
            gif: video.gif_playback.unwrap_or(false),
            note: base.video_message.as_option().is_none(),
        });
    }
    if let Some(audio) = base.audio_message.as_option() {
        return Some(Content::Audio {
            media: media(audio.mimetype.as_ref(), audio.file_length, None, None),
            seconds: audio.seconds,
            voice_note: audio.ptt.unwrap_or(false),
            waveform: audio.waveform.clone().unwrap_or_default(),
        });
    }
    if let Some(document) = base.document_message.as_option() {
        let file_name = non_empty(&document.file_name)
            .or_else(|| non_empty(&document.title))
            .unwrap_or_else(|| "Document".to_owned());
        return Some(Content::Document {
            media: media(document.mimetype.as_ref(), document.file_length, None, None),
            file_name,
            caption: non_empty(&document.caption),
            pages: document.page_count,
        });
    }
    if let Some(sticker) = base.sticker_message.as_option() {
        return Some(Content::Sticker {
            media: media(
                sticker.mimetype.as_ref(),
                sticker.file_length,
                sticker.width,
                sticker.height,
            ),
            animated: sticker.is_animated.unwrap_or(false),
        });
    }
    if let Some(location) = base.location_message.as_option() {
        if location.is_live == Some(true) {
            return live_location_of(base).map(|(content, _)| content);
        }
        return Some(Content::Location {
            latitude: location.degrees_latitude.unwrap_or(0.0),
            longitude: location.degrees_longitude.unwrap_or(0.0),
            name: non_empty(&location.name),
            address: non_empty(&location.address),
        });
    }
    if base.live_location_message.is_set() {
        return live_location_of(base).map(|(content, _)| content);
    }
    if let Some(contact) = base.contact_message.as_option() {
        return Some(Content::Contact {
            display_name: non_empty(&contact.display_name).unwrap_or_else(|| "Contact".to_owned()),
            vcard: contact.vcard.clone().unwrap_or_default(),
        });
    }
    if let Some(contacts) = base.contacts_array_message.as_option() {
        let count = contacts.contacts.len();
        return Some(Content::Contact {
            display_name: non_empty(&contacts.display_name)
                .unwrap_or_else(|| format!("{count} contacts")),
            vcard: contacts
                .contacts
                .iter()
                .filter_map(|contact| contact.vcard.clone())
                .collect::<Vec<_>>()
                .join("\n"),
        });
    }
    if let Some(poll) = base
        .poll_creation_message
        .as_option()
        .or(base.poll_creation_message_v2.as_option())
        .or(base.poll_creation_message_v3.as_option())
    {
        return Some(Content::Poll {
            question: non_empty(&poll.name).unwrap_or_else(|| "Poll".to_owned()),
            state: crate::model::PollState {
                selectable: poll.selectable_options_count.unwrap_or(0) as usize,
                ..Default::default()
            },
            options: poll
                .options
                .iter()
                .map(|option| option.option_name.clone().unwrap_or_default())
                .collect(),
        });
    }
    if base.placeholder_message.is_set() {
        // The phone masks some messages from linked devices, live locations
        // among them, and sends this stand-in so the chat still shows them.
        return Some(Content::PhoneOnly {
            view_once: false,
            live_location: false,
            once: None,
        });
    }
    if let Some(reply) = base.rich_response_message.as_option() {
        return Some(bot_replies::content(reply));
    }
    let unsupported = |what: &str| {
        Some(Content::Unsupported {
            what: what.to_owned(),
        })
    };
    if base.album_message.is_set()
        || motion_photo_child(base)
        || hd_copy(base)
        || group_status_mention(base)
    {
        return None;
    }
    if base.group_invite_message.is_set() {
        return unsupported("group invite");
    }
    if base.event_message.is_set() {
        return unsupported("event");
    }
    if let Some(pack) = base.sticker_pack_message.as_option() {
        return Some(stickers::sticker_pack_content(pack));
    }
    if let Some(content) = interactive::classify(base) {
        return Some(content);
    }
    if base.product_message.is_set() || base.order_message.is_set() {
        return unsupported("product");
    }
    if base.send_payment_message.is_set()
        || base.request_payment_message.is_set()
        || base.payment_invite_message.is_set()
        || base.invoice_message.is_set()
    {
        return unsupported("payment");
    }
    if let Some(log) = base.call_log_messsage.as_option() {
        return Some(calls::call_log_content(log));
    }
    if base.scheduled_call_creation_message.is_set() {
        return unsupported("call");
    }
    if base.lottie_sticker_message.is_set() {
        return unsupported("animated sticker");
    }
    if base.poll_update_message.is_set()
        || base.enc_reaction_message.is_set()
        || base.enc_comment_message.is_set()
        || base.enc_event_response_message.is_set()
        || base.keep_in_chat_message.is_set()
        || base.pin_in_chat_message.is_set()
        || base.sender_key_distribution_message.is_set()
        || base
            .fast_ratchet_key_sender_key_distribution_message
            .is_set()
        || base.sticker_sync_rmr_message.is_set()
        || base.device_sent_message.is_set()
        || base.secret_encrypted_message.is_set()
        || base.message_history_bundle.is_set()
        || base.message_history_notice.is_set()
        || base.bot_invoke_message.is_set()
        || base.group_root_key_share.is_set()
        || base.root_secret_distribute_message.is_set()
        || base.poll_add_option_message.is_set()
        || base.bot_task_message.is_set()
        || base.status_notification_message.is_set()
    {
        return None;
    }
    // Most messages carry `message_context_info` (secrets, bot metadata), so
    // it marks nothing on its own: content beside it that is not recognised
    // above is shown as unsupported rather than dropped.
    let without_metadata = {
        let mut proto = base.clone();
        proto.message_context_info = MessageField::none();
        proto
    };
    if without_metadata == wa::Message::default() {
        return None;
    }
    unsupported("message")
}

/// Whether `base` carries the video half of a motion photo. The photo is a
/// message of its own, so the clip beside it is not a second message.
fn motion_photo_child(base: &wa::Message) -> bool {
    use wa::context_info::PairedMediaType;
    use wa::message_association::AssociationType;
    let Some(child) = base
        .associated_child_message
        .as_option()
        .and_then(|wrapper| wrapper.message.as_option())
    else {
        return false;
    };
    let associated = |message: &wa::Message| {
        message
            .message_context_info
            .as_option()
            .and_then(|context| context.message_association.as_option())
            .and_then(|association| association.association_type)
            == Some(AssociationType::MOTION_PHOTO)
    };
    associated(base)
        || associated(child)
        || child
            .video_message
            .as_option()
            .and_then(|video| video.context_info.as_option())
            .and_then(|context| context.paired_media_type)
            == Some(PairedMediaType::MOTION_PHOTO_CHILD)
}

/// Whether `base` is the notice that someone mentioned this group in their
/// status. The phone and WhatsApp for Mac show no message for it in the
/// group, and its status is not something a linked device can open (#595).
fn group_status_mention(base: &wa::Message) -> bool {
    base.group_status_mention_message.is_set()
}

/// Whether `base` carries the HD copy of a photo or video that was also sent
/// in standard quality. The standard copy is the message, so the HD copy
/// beside it is not a second one.
fn hd_copy(base: &wa::Message) -> bool {
    use wa::context_info::PairedMediaType;
    use wa::message_association::AssociationType;
    let Some(child) = base
        .associated_child_message
        .as_option()
        .and_then(|wrapper| wrapper.message.as_option())
    else {
        return false;
    };
    let dual_upload = |message: &wa::Message| {
        matches!(
            message
                .message_context_info
                .as_option()
                .and_then(|context| context.message_association.as_option())
                .and_then(|association| association.association_type),
            Some(
                AssociationType::HD_IMAGE_DUAL_UPLOAD
                    | AssociationType::HD_VIDEO_DUAL_UPLOAD
                    | AssociationType::HEVC_VIDEO_DUAL_UPLOAD
            )
        )
    };
    let paired = child
        .image_message
        .as_option()
        .and_then(|image| image.context_info.as_option())
        .or_else(|| {
            child
                .video_message
                .as_option()
                .and_then(|video| video.context_info.as_option())
        })
        .and_then(|context| context.paired_media_type);
    dual_upload(base)
        || dual_upload(child)
        || matches!(
            paired,
            Some(
                PairedMediaType::HD_IMAGE_CHILD
                    | PairedMediaType::HD_VIDEO_CHILD
                    | PairedMediaType::HEVC_VIDEO_CHILD
            )
        )
}

/// The id of the photo a motion photo's clip belongs to, and the clip as a
/// bare video message: what a later download needs and nothing else.
fn motion_clip(base: &wa::Message) -> Option<(String, Vec<u8>)> {
    if !motion_photo_child(base) {
        return None;
    }
    let child = base
        .associated_child_message
        .as_option()?
        .message
        .as_option()?;
    let video = child.video_message.as_option()?;
    let parent = [base, child].into_iter().find_map(|message| {
        message
            .message_context_info
            .as_option()?
            .message_association
            .as_option()?
            .parent_message_key
            .as_option()?
            .id
            .clone()
            .filter(|id| !id.is_empty())
    })?;
    let clip = {
        let mut proto = wa::Message::default();
        proto.video_message = MessageField::some(video.clone());
        proto
    };
    Some((parent, clip.encode_to_vec()))
}

/// Builds synthetic album attachments with explicit protocol parent ids.
fn album_media(base: &wa::Message, mut media: Media) -> Media {
    media.album = base
        .message_context_info
        .as_option()
        .and_then(|context| context.message_association.as_option())
        .filter(|association| {
            association.association_type
                == Some(wa::message_association::AssociationType::MEDIA_ALBUM)
        })
        .and_then(|association| association.parent_message_key.as_option())
        .and_then(|key| key.id.clone());
    media
}

/// Uploaded attachment protobuf and archive content.
pub(super) struct Prepared {
    message: wa::Message,
    content: Content,
    thumbnail: Option<Vec<u8>>,
    bytes: Vec<u8>,
    mime: String,
    file_name: Option<String>,
}

fn encode_jpeg(image: &image::DynamicImage, quality: u8) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, quality);
    encoder
        .encode_image(&image.to_rgb8())
        .map_err(|error| error.to_string())?;
    Ok(bytes)
}

/// Downloads a picture through the proxy settings into the avatar cache.
async fn download_avatar(url: String, path: PathBuf) -> Result<PathBuf, String> {
    let bytes = tokio::task::spawn_blocking(move || {
        crate::proxy::agent()
            .get(&url)
            .call()
            .and_then(|mut response| response.body_mut().read_to_vec())
            .map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())??;
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|error| error.to_string())?;
    }
    tokio::fs::write(&path, &bytes)
        .await
        .map_err(|error| error.to_string())?;
    Ok(path)
}

/// Crops a picture file to a centred square and encodes it at the size
/// WhatsApp uses for profile and group pictures.
fn profile_picture_jpeg(path: &std::path::Path) -> Result<Vec<u8>, String> {
    square_picture_jpeg(&image::open(path).map_err(|error| error.to_string())?)
}

/// Crops a picture to a centred square, scales it down to 640 pixels (a
/// smaller one keeps its size), and encodes it as JPEG.
fn square_picture_jpeg(image: &image::DynamicImage) -> Result<Vec<u8>, String> {
    let side = image.width().min(image.height());
    let square = image.crop_imm(
        (image.width() - side) / 2,
        (image.height() - side) / 2,
        side,
        side,
    );
    let size = side.min(PROFILE_PICTURE_SIDE);
    let resized = square.resize_exact(size, size, image::imageops::FilterType::Lanczos3);
    encode_jpeg(&resized, 85)
}

/// What a refused change to a group's info says.
const GROUP_EDIT_REFUSED: &str = "Only admins can change this group's name and photo.";

/// Whether WhatsApp refused a group change because we may not make it,
/// rather than failing to carry it out.
fn group_edit_refused(error: &str) -> bool {
    ["forbidden", "not-authorized", "code=401", "code=403"]
        .iter()
        .any(|word| error.contains(word))
}

/// Builds the pre-download attachment thumbnail.
fn thumbnail_jpeg(image: &image::DynamicImage) -> Option<Vec<u8>> {
    let small = image.thumbnail(THUMBNAIL_SIDE, THUMBNAIL_SIDE);
    encode_jpeg(&small, 60).ok()
}

/// Uploads a recording and builds a push-to-talk message with waveform.
async fn prepare_voice(
    client: &Client,
    bytes: Vec<u8>,
    seconds: u32,
    waveform: Vec<u8>,
    context: Option<Box<wa::ContextInfo>>,
) -> Result<Prepared, String> {
    let mime = "audio/ogg; codecs=opus".to_owned();
    let size = bytes.len() as u64;
    let upload = client
        .upload(bytes.clone(), MediaType::Audio, UploadOptions::default())
        .await
        .map_err(|error| error.to_string())?;
    let message = audio_message(upload, {
        let mut options = AudioOptions::default()
            .with_mimetype(mime.clone())
            .with_duration_seconds(seconds)
            .with_ptt(true)
            .with_waveform(waveform.clone());
        options.context_info = context;
        options
    });
    Ok(Prepared {
        message,
        content: Content::Audio {
            media: media(Some(&mime), Some(size), None, None),
            seconds: Some(seconds),
            voice_note: true,
            waveform,
        },
        thumbnail: None,
        bytes,
        mime,
        file_name: None,
    })
}

/// The mimetype an attached audio file is sent under as an audio message,
/// or `None` when phones cannot play it inline (WAV, FLAC, AIFF, WMA and the
/// like), so it goes as a document and arrives as the original file (#162).
/// WhatsApp's audio messages are MP3, AAC, M4A, AMR and OGG.
fn whatsapp_audio_mime(mime: &str) -> Option<&'static str> {
    match mime {
        "audio/mpeg" | "audio/mp3" => Some("audio/mpeg"),
        "audio/mp4" | "audio/m4a" | "audio/x-m4a" => Some("audio/mp4"),
        "audio/aac" => Some("audio/aac"),
        "audio/amr" => Some("audio/amr"),
        "audio/ogg" => Some("audio/ogg"),
        _ => None,
    }
}

/// Whether an attached video goes as a video message. Phones play only MP4
/// inline, and WhatsApp drops a video message in another container (a
/// QuickTime `.mov`, WebM, MKV, AVI) without telling either side, so the
/// recipient gets nothing (#553). Those go as documents and arrive as the
/// original file; ZapFast does not transcode.
fn whatsapp_video_mime(mime: &str) -> bool {
    mime == "video/mp4"
}

/// Uploads a file and builds its message. Images are encoded as JPEG.
async fn prepare_media(
    client: &Client,
    bytes: Vec<u8>,
    mime: &str,
    file_name: Option<&str>,
    gif: bool,
) -> Result<Prepared, String> {
    let is_picture = matches!(
        mime,
        "image/jpeg" | "image/png" | "image/webp" | "image/bmp" | "image/tiff"
    );
    if is_picture {
        let decoded = tokio::task::spawn_blocking({
            let bytes = bytes.clone();
            move || image::load_from_memory(&bytes).map_err(|error| error.to_string())
        })
        .await
        .map_err(|error| error.to_string())??;
        let (width, height) = (decoded.width(), decoded.height());
        let jpeg = if mime == "image/jpeg" {
            bytes
        } else {
            encode_jpeg(&decoded, 88)?
        };
        let thumbnail = thumbnail_jpeg(&decoded);
        let upload = client
            .upload(jpeg.clone(), MediaType::Image, UploadOptions::default())
            .await
            .map_err(|error| error.to_string())?;
        let mut message = image_message(upload, {
            let mut options = ImageOptions::default().with_mimetype("image/jpeg");
            options.jpeg_thumbnail = thumbnail.clone();
            options
        });
        if let Some(image) = message.image_message.as_option_mut() {
            image.width = Some(width);
            image.height = Some(height);
        }
        return Ok(Prepared {
            message,
            content: Content::Image {
                motion: None,
                caption: None,
                media: media(
                    Some(&"image/jpeg".to_owned()),
                    Some(jpeg.len() as u64),
                    Some(width),
                    Some(height),
                ),
            },
            thumbnail,
            bytes: jpeg,
            mime: "image/jpeg".to_owned(),
            file_name: None,
        });
    }
    let size = bytes.len() as u64;
    let mime_owned = mime.to_owned();
    if whatsapp_video_mime(mime) {
        // The picture, size, and length phones show before downloading it.
        // Reading a second of frames takes a moment, so off the runtime.
        let poster = {
            let bytes = bytes.clone();
            tokio::task::spawn_blocking(move || crate::animation::poster(&bytes))
                .await
                .ok()
                .flatten()
        };
        let thumbnail = poster
            .as_ref()
            .and_then(|poster| poster.picture.clone())
            .and_then(|picture| thumbnail_jpeg(&image::DynamicImage::ImageRgb8(picture)));
        let size_in_pixels = poster.as_ref().map(|poster| (poster.width, poster.height));
        let seconds = poster.as_ref().map(|poster| poster.seconds);
        let upload = client
            .upload(bytes.clone(), MediaType::Video, UploadOptions::default())
            .await
            .map_err(|error| error.to_string())?;
        let mut message = video_message(upload, {
            let mut options = VideoOptions::default()
                .with_mimetype(mime_owned.clone())
                .with_gif_playback(gif);
            options.jpeg_thumbnail = thumbnail.clone();
            options.duration_seconds = seconds;
            options
        });
        if let (Some(video), Some((width, height))) =
            (message.video_message.as_option_mut(), size_in_pixels)
        {
            video.width = Some(width);
            video.height = Some(height);
        }
        return Ok(Prepared {
            message,
            content: Content::Video {
                caption: None,
                media: media(
                    Some(&mime_owned),
                    Some(size),
                    size_in_pixels.map(|(width, _)| width),
                    size_in_pixels.map(|(_, height)| height),
                ),
                seconds,
                gif,
                note: false,
            },
            thumbnail,
            bytes,
            mime: mime_owned,
            file_name: file_name.map(str::to_owned),
        });
    }
    if let Some(audio_mime) = whatsapp_audio_mime(mime) {
        let mime_owned = audio_mime.to_owned();
        let upload = client
            .upload(bytes.clone(), MediaType::Audio, UploadOptions::default())
            .await
            .map_err(|error| error.to_string())?;
        let message = audio_message(
            upload,
            AudioOptions::default()
                .with_mimetype(mime_owned.clone())
                .with_ptt(false),
        );
        return Ok(Prepared {
            message,
            content: Content::Audio {
                media: media(Some(&mime_owned), Some(size), None, None),
                seconds: None,
                voice_note: false,
                waveform: Vec::new(),
            },
            thumbnail: None,
            bytes,
            mime: mime_owned,
            file_name: file_name.map(str::to_owned),
        });
    }
    prepare_document(client, bytes, mime, file_name).await
}

/// Uploads a file as a document: its own bytes, name and type, whatever it
/// holds, so a picture keeps its quality, transparency and metadata.
async fn prepare_document(
    client: &Client,
    bytes: Vec<u8>,
    mime: &str,
    file_name: Option<&str>,
) -> Result<Prepared, String> {
    let size = bytes.len() as u64;
    let mime_owned = mime.to_owned();
    let upload = client
        .upload(bytes.clone(), MediaType::Document, UploadOptions::default())
        .await
        .map_err(|error| error.to_string())?;
    let name = file_name.unwrap_or("file").to_owned();
    let message = document_message(
        upload,
        DocumentOptions::default()
            .with_mimetype(mime_owned.clone())
            .with_file_name(name.clone())
            .with_title(name.clone()),
    );
    Ok(Prepared {
        message,
        content: Content::Document {
            media: media(Some(&mime_owned), Some(size), None, None),
            file_name: name.clone(),
            caption: None,
            pages: None,
        },
        thumbnail: None,
        bytes,
        mime: mime_owned,
        file_name: Some(name),
    })
}

/// Uploads a WebP sticker and builds its message without a library builder.
async fn prepare_sticker(client: &Client, bytes: Vec<u8>) -> Result<Prepared, String> {
    let (animated, width, height) = tokio::task::spawn_blocking({
        let bytes = bytes.clone();
        move || {
            let decoder = image::codecs::webp::WebPDecoder::new(std::io::Cursor::new(&bytes))
                .map_err(|error| error.to_string())?;
            let animated = decoder.has_animation();
            let (width, height) = image::ImageDecoder::dimensions(&decoder);
            Ok::<_, String>((animated, width, height))
        }
    })
    .await
    .map_err(|error| error.to_string())??;
    let upload = client
        .upload(bytes.clone(), MediaType::Sticker, UploadOptions::default())
        .await
        .map_err(|error| error.to_string())?;
    let message = {
        let mut proto = wa::Message::default();
        proto.sticker_message = MessageField::some({
            let mut proto = wa::message::StickerMessage::default();
            proto.url = Some(upload.url);
            proto.direct_path = Some(upload.direct_path);
            proto.media_key = Some(upload.media_key.to_vec());
            proto.file_enc_sha256 = Some(upload.file_enc_sha256.to_vec());
            proto.file_sha256 = Some(upload.file_sha256.to_vec());
            proto.file_length = Some(upload.file_length);
            proto.mimetype = Some("image/webp".to_owned());
            proto.media_key_timestamp = Some(upload.media_key_timestamp);
            proto.is_animated = Some(animated);
            proto.width = Some(width);
            proto.height = Some(height);
            proto
        });
        proto
    };
    Ok(Prepared {
        message,
        content: Content::Sticker {
            media: media(
                Some(&"image/webp".to_owned()),
                Some(bytes.len() as u64),
                Some(width),
                Some(height),
            ),
            animated,
        },
        thumbnail: None,
        bytes,
        mime: "image/webp".to_owned(),
        file_name: None,
    })
}

fn percent_encode(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// Searches GIPHY and downloads result stills. Empty queries list trending GIFs.
fn search_gifs(query: &str, key: &str, dir: &Path) -> Result<Vec<Gif>, GifError> {
    let plain = |message: String| GifError {
        message,
        bad_key: false,
    };
    if key.is_empty() {
        return Err(GifError {
            message: "GIF search needs a GIPHY API key.".to_owned(),
            bad_key: true,
        });
    }
    let url = if query.trim().is_empty() {
        format!("https://api.giphy.com/v1/gifs/trending?api_key={key}&limit=24&rating=pg-13")
    } else {
        format!(
            "https://api.giphy.com/v1/gifs/search?api_key={key}&q={}&limit=24&rating=pg-13",
            percent_encode(query.trim())
        )
    };
    let body = match crate::proxy::agent().get(&url).call() {
        Ok(mut response) => response
            .body_mut()
            .read_to_string()
            .map_err(|error| plain(format!("GIPHY request failed: {error}")))?,
        // Treat 401 and 403 as API-key failures for the picker.
        Err(ureq::Error::StatusCode(code @ (401 | 403))) => {
            return Err(GifError {
                message: format!("GIPHY rejected the API key (error {code})."),
                bad_key: true,
            });
        }
        Err(error) => return Err(plain(format!("GIPHY request failed: {error}"))),
    };
    let json: serde_json::Value = serde_json::from_str(&body)
        .map_err(|error| plain(format!("Invalid GIPHY response: {error}")))?;
    if let Some(message) = json["meta"]["msg"].as_str()
        && let Some(status) = json["meta"]["status"]
            .as_u64()
            .filter(|status| *status >= 400)
    {
        return Err(GifError {
            message: format!("GIPHY: {message}"),
            bad_key: status == 401 || status == 403,
        });
    }
    let data = json["data"]
        .as_array()
        .ok_or_else(|| plain("GIPHY response contained no results".to_owned()))?;
    std::fs::create_dir_all(dir).map_err(|error| plain(error.to_string()))?;
    let mut gifs: Vec<(Gif, Option<String>)> = data
        .iter()
        .filter_map(|item| {
            let id = item["id"].as_str()?.to_owned();
            let images = &item["images"];
            let pick = |names: &[&str], field: &str| {
                names
                    .iter()
                    .find_map(|name| images[*name][field].as_str().map(str::to_owned))
            };
            let mp4 = pick(&["fixed_width", "downsized_small", "original"], "mp4")?;
            let still = pick(
                &[
                    "fixed_width_small_still",
                    "fixed_width_still",
                    "original_still",
                ],
                "url",
            );
            let number = |name: &str| {
                images["fixed_width"][name]
                    .as_str()
                    .and_then(|value| value.parse::<u32>().ok())
                    .unwrap_or(200)
            };
            Some((
                Gif {
                    id,
                    still: None,
                    mp4,
                    width: number("width"),
                    height: number("height"),
                },
                still,
            ))
        })
        .collect();
    std::thread::scope(|scope| {
        for (gif, still) in &mut gifs {
            let Some(url) = still.clone() else {
                continue;
            };
            let path = dir.join(format!("{}.jpg", sanitize(&gif.id)));
            if path.exists() {
                gif.still = Some(path);
                continue;
            }
            let slot = &mut gif.still;
            scope.spawn(move || {
                let fetched = ureq::get(&url)
                    .call()
                    .and_then(|mut response| response.body_mut().read_to_vec());
                if let Ok(bytes) = fetched
                    && std::fs::write(&path, bytes).is_ok()
                {
                    *slot = Some(path);
                }
            });
        }
    });
    Ok(gifs.into_iter().map(|(gif, _)| gif).collect())
}

/// Copies a sent attachment to media storage and builds its archive row.
/// Attaches a reply's quote to a prepared attachment, then builds its
/// outgoing row like [`file_outbound`]. An attachment that cannot carry the
/// quote is not sent at all.
#[allow(clippy::too_many_arguments)]
async fn quoted_outbound(
    client: &Client,
    chat: &str,
    me: &str,
    dir: &Path,
    mut prepared: Prepared,
    caption: Option<String>,
    mentions: Vec<String>,
    quote: Option<(wa::ContextInfo, Quoted)>,
) -> Result<(Message, Vec<u8>), String> {
    let shown = attach_quote(&mut prepared.message, quote)?;
    let (mut row, raw) = file_outbound(client, chat, me, dir, prepared, caption, mentions).await?;
    row.quoted = shown;
    Ok((row, raw))
}

/// Puts a reply's quote on an outgoing message, returning what the reply's
/// row shows. A message kind that cannot carry a quote is an error.
fn attach_quote(
    message: &mut wa::Message,
    quote: Option<(wa::ContextInfo, Quoted)>,
) -> Result<Option<Quoted>, String> {
    let Some((context, shown)) = quote else {
        return Ok(None);
    };
    if !message.set_context_info(context) {
        return Err("Could not attach the reply context".to_owned());
    }
    Ok(Some(shown))
}

/// Mentions people in an attachment, keeping a reply context it carries.
///
/// [`crate::mentions::ALL_ID`] sets the everyone bit and is left out of
/// `mentioned_jid`.
fn add_mentions(message: &mut wa::Message, mentions: &[String]) {
    let (jids, mention_all) = participant_mentions(mentions);
    if jids.is_empty() && !mention_all {
        return;
    }
    let mut context = context_of(message).cloned().unwrap_or_default();
    if !jids.is_empty() {
        context.mentioned_jid = jids;
    }
    if mention_all {
        context.non_jid_mentions = Some(crate::mentions::with_flag(context.non_jid_mentions));
    }
    message.set_context_info(context);
}

/// Marks a prepared photo for WhatsApp's view-once send path. The protocol
/// library recognizes this field when it builds the outgoing stanza metadata.
fn mark_view_once_photo(prepared: &mut Prepared) -> Result<(), String> {
    if !matches!(&prepared.content, Content::Image { .. }) {
        return Err("View once is only available for photos".to_owned());
    }
    let Some(image) = prepared.message.image_message.as_option_mut() else {
        return Err("View once is only available for photos".to_owned());
    };
    image.view_once = Some(true);
    Ok(())
}

/// Keeps the outgoing photo's thumbnail on the wire, but out of its local archive.
/// Prepared attachments carry their photo directly in `image_message`.
fn attachment_archive_raw(message: &wa::Message) -> Vec<u8> {
    if message.is_view_once() {
        let mut archived = message.clone();
        if let Some(image) = archived.image_message.as_option_mut() {
            image.jpeg_thumbnail = None;
        }
        archived.encode_to_vec()
    } else {
        message.encode_to_vec()
    }
}

pub(super) async fn file_outbound(
    client: &Client,
    chat: &str,
    me: &str,
    dir: &Path,
    mut prepared: Prepared,
    caption: Option<String>,
    mentions: Vec<String>,
) -> Result<(Message, Vec<u8>), String> {
    if let Some(caption) = caption.filter(|caption| !caption.trim().is_empty()) {
        match &mut prepared.content {
            Content::Image { caption: slot, .. }
            | Content::Video { caption: slot, .. }
            | Content::Document { caption: slot, .. } => *slot = Some(caption.clone()),
            _ => {}
        }
        if let Some(image) = prepared.message.image_message.as_option_mut() {
            image.caption = Some(caption.clone());
        }
        if let Some(video) = prepared.message.video_message.as_option_mut() {
            video.caption = Some(caption.clone());
        }
        if let Some(document) = prepared.message.document_message.as_option_mut() {
            document.caption = Some(caption);
        }
    }
    add_mentions(&mut prepared.message, &mentions);
    let id = client.generate_message_id().into_string();
    let mut content = prepared.content;
    let thumbnail = if prepared.message.is_view_once() {
        let once = crate::model::OnceMedia::of(&content)
            .filter(|kind| *kind == crate::model::OnceMedia::Photo)
            .ok_or_else(|| "View once is only available for photos".to_owned())?;
        content = Content::PhoneOnly {
            view_once: true,
            live_location: false,
            once: Some(once),
        };
        None
    } else {
        let path = media_path(
            dir,
            chat,
            &id,
            &prepared.mime,
            prepared.file_name.as_deref(),
        );
        tokio::fs::create_dir_all(dir)
            .await
            .map_err(|error| error.to_string())?;
        tokio::fs::write(&path, &prepared.bytes)
            .await
            .map_err(|error| error.to_string())?;
        if let Some(media) = content.media_mut() {
            media.path = Some(path);
        }
        prepared.thumbnail
    };
    let row = Message {
        id,
        chat: chat.to_owned(),
        sender: me.to_owned(),
        sender_name: None,
        from_me: true,
        timestamp: crate::util::now(),
        content,
        status: Delivery::Pending,
        delivered_at: None,
        read_at: None,
        quoted: None,
        reactions: Vec::new(),
        history_order: None,
        edited: false,
        mentions: mentions
            .into_iter()
            .filter_map(|id| {
                if id == crate::mentions::ALL_ID {
                    return Some(crate::mentions::everyone_ref());
                }
                let user = id.split('@').next()?.to_owned();
                (!user.is_empty()).then_some(MentionRef { user, id })
            })
            .collect(),
        forwarded: false,
        thumbnail,
    };
    Ok((row, prepared.message.encode_to_vec()))
}

/// Decodes a history chunk off the worker thread.
fn parse_history(compressed: &[u8]) -> Result<ParsedHistory, String> {
    let mut stream = HistorySyncStream::new(compressed, MAX_DECOMPRESSED);
    let mut chats = Vec::new();
    loop {
        let conversation = match stream.next_conversation() {
            Ok(Some(conversation)) => conversation,
            Ok(None) => break,
            Err(error) => return Err(error.to_string()),
        };
        chats.push(parse_conversation(conversation));
    }
    let remainder = stream.remainder().map_err(|error| error.to_string())?;
    let push_names = remainder
        .pushnames
        .iter()
        .filter_map(|entry| Some((entry.id.clone()?, entry.pushname.clone()?)))
        .collect();
    let lids = remainder
        .phone_number_to_lid_mappings
        .iter()
        .filter_map(|entry| Some((entry.lid_jid.clone()?, entry.pn_jid.clone()?)))
        .collect();
    Ok(ParsedHistory {
        chats,
        push_names,
        lids,
        stickers: remainder.recent_stickers,
    })
}

/// Gives a history row opened from an encrypted edit the edited body, as a
/// full snapshot: its content, reply, mentions and raw message, keeping the
/// history entry's time and order. False when the body shows nothing.
fn fill_from_edit(message: &mut ParsedMessage, body: &wa::Message, secret: Vec<u8>) -> bool {
    let Some(content) = classify(body) else {
        return false;
    };
    let base = body.get_base_message();
    message.content = content;
    message.quoted = history_quote(base);
    message.mentions = recorded_mention_ids(base);
    message.forwarded = forwarded_of(base);
    message.thumbnail = thumbnail_of(base);
    message.raw = body.encode_to_vec();
    message.poll_secret = Some(secret);
    message.edited = true;
    true
}

/// The message a history message replies to, as its context names it.
fn history_quote(base: &wa::Message) -> Option<Quoted> {
    let context = context_of(base)?;
    let id = context.stanza_id.clone().filter(|id| !id.is_empty())?;
    let content = context.quoted_message.as_option().and_then(classify);
    Some(Quoted {
        labelled: content.as_ref().map(Content::labelled),
        chat: context.remote_jid.clone(),
        mentions: Vec::new(),
        id,
        sender: context.participant.clone().unwrap_or_default(),
        sender_name: None,
        summary: content.as_ref().map(Content::summary).unwrap_or_default(),
    })
}

/// Converts supported protocol message content into local display types, retaining quoted source metadata.
fn parse_conversation(conversation: wa::Conversation) -> ParsedChat {
    let mut diagnostics = HistoryDiagnostics {
        entries: conversation.messages.len(),
        ..Default::default()
    };
    let mut messages = Vec::new();
    let mut revoked = Vec::new();
    let mut poll_updates = Vec::new();
    let mut reactions = Vec::new();
    let mut clips = Vec::new();
    let mut pins = Vec::new();
    let mut newest = 0;
    for entry in &conversation.messages {
        let Some(info) = entry.message.as_option() else {
            diagnostics.missing_envelope += 1;
            continue;
        };
        let Some(key) = info.key.as_option() else {
            diagnostics.missing_key += 1;
            continue;
        };
        let Some(id) = key.id.clone().filter(|id| !id.is_empty()) else {
            diagnostics.missing_key += 1;
            continue;
        };
        let Some(original) = info.message.as_option() else {
            // A missed call the phone filed as a bare notice.
            if let Some(content) = info.message_stub_type.and_then(calls::missed_call_stub) {
                let from_me = key.from_me.unwrap_or(false);
                let timestamp = info.message_timestamp.unwrap_or(0) as i64;
                newest = newest.max(timestamp);
                messages.push(ParsedMessage {
                    id,
                    sender: key.participant.clone().or_else(|| key.remote_jid.clone()),
                    from_me,
                    push_name: None,
                    timestamp,
                    history_order: entry
                        .msg_order_id
                        .and_then(|order| i64::try_from(order).ok()),
                    edited: false,
                    content,
                    status: Delivery::None,
                    quoted: None,
                    reactions: Vec::new(),
                    mentions: Vec::new(),
                    forwarded: false,
                    thumbnail: None,
                    raw: Vec::new(),
                    poll_secret: None,
                    poll_votes: Vec::new(),
                    receipts: Vec::new(),
                    sealed_edit: None,
                });
                continue;
            }
            diagnostics.missing_body += 1;
            diagnostics.ciphertext_stubs += usize::from(
                info.message_stub_type == Some(wa::web_message_info::StubType::CIPHERTEXT),
            );
            diagnostics.futureproof_body += usize::from(info.futureproof_data.is_some());
            continue;
        };
        let from_me = key.from_me.unwrap_or(false);
        let timestamp = info.message_timestamp.unwrap_or(0) as i64;
        newest = newest.max(timestamp);
        let base = original.get_base_message();
        if let Some(protocol) = base.protocol_message.as_option()
            && protocol.r#type == Some(wa::message::protocol_message::Type::MESSAGE_EDIT)
        {
            diagnostics.edit_without_key += usize::from(
                protocol
                    .key
                    .as_option()
                    .and_then(|key| key.id.as_ref())
                    .is_none_or(String::is_empty),
            );
            diagnostics.edit_without_body += usize::from(protocol.edited_message.is_unset());
        }
        diagnostics.edited_wrappers += usize::from(original.edited_message.is_set());
        diagnostics.encrypted_edits += usize::from(
            base.secret_encrypted_message
                .as_option()
                .is_some_and(|envelope| {
                    envelope.secret_enc_type
                        == Some(wa::message::secret_encrypted_message::SecretEncType::MESSAGE_EDIT)
                }),
        );
        // In history MESSAGE_EDIT is a complete snapshot: the outer key is
        // the latest edit's key, while protocol.key names the original bubble.
        // Keep the history envelope's timestamp/order and use the target id.
        // WA Web does this in WAWebParseHistorySyncProtocolMessageEditProto.
        let snapshot = base.protocol_message.as_option().and_then(|protocol| {
            (protocol.r#type == Some(wa::message::protocol_message::Type::MESSAGE_EDIT))
                .then_some(protocol)
                .and_then(|protocol| {
                    let target = protocol
                        .key
                        .as_option()?
                        .id
                        .as_deref()
                        .filter(|id| !id.is_empty())?;
                    Some((target, protocol.edited_message.as_option()?))
                })
        });
        diagnostics.rekeyed_edit_snapshots +=
            usize::from(snapshot.is_some_and(|(target, _)| target != id));
        let (id, message) = snapshot.map_or((id, original), |(target, edited)| {
            (target.to_owned(), edited)
        });
        let edited = snapshot.is_some() || original.edited_message.is_set();
        diagnostics.edit_snapshots += usize::from(snapshot.is_some());
        let base = message.get_base_message();
        if let Some(protocol) = base.protocol_message.as_option() {
            if protocol.r#type == Some(wa::message::protocol_message::Type::MESSAGE_EDIT) {
                diagnostics.invalid_edits += 1;
            } else {
                diagnostics.other_protocol += 1;
            }
            if protocol.r#type == Some(wa::message::protocol_message::Type::REVOKE)
                && let Some(target) = protocol.key.as_option().and_then(|key| key.id.clone())
            {
                revoked.push(target);
            }
            continue;
        }
        let sender = info
            .participant
            .clone()
            .or_else(|| key.participant.clone())
            .filter(|sender| !sender.is_empty())
            .or_else(|| key.remote_jid.clone());
        if let Some(reaction) = base.reaction_message.as_option() {
            if let Some(target) = reaction
                .key
                .as_option()
                .and_then(|key| key.id.clone())
                .filter(|id| !id.is_empty())
            {
                reactions.push(HistoryReaction {
                    target,
                    sender,
                    from_me,
                    body: HistoryReactionBody::Plain(
                        reaction_emoji(reaction.text.as_deref(), reaction.grouping_key.as_deref())
                            .unwrap_or_default(),
                    ),
                    sent_at: reaction
                        .sender_timestamp_ms
                        .unwrap_or(timestamp.saturating_mul(1000)),
                });
            }
            continue;
        }
        if base.enc_reaction_message.is_set() {
            if let Some(enc) = base.enc_reaction_message.as_option()
                && let Some(target) = enc
                    .target_message_key
                    .as_option()
                    .and_then(|key| key.id.clone())
                    .filter(|id| !id.is_empty())
                && let (Some(payload), Some(iv)) = (enc.enc_payload.clone(), enc.enc_iv.clone())
            {
                reactions.push(HistoryReaction {
                    target,
                    sender,
                    from_me,
                    body: HistoryReactionBody::Encrypted { payload, iv },
                    sent_at: timestamp.saturating_mul(1000),
                });
            }
            continue;
        }
        if let Some(pin) = base.pin_in_chat_message.as_option() {
            let duration = base
                .message_context_info
                .as_option()
                .and_then(|context| context.message_add_on_duration_in_secs)
                .unwrap_or(0);
            if let Some((target, pinned, duration_secs)) = pin_action(pin, duration) {
                pins.push(HistoryPin {
                    target,
                    pinned,
                    duration_secs,
                    at: timestamp,
                    pinner: crate::archive::Pinner {
                        by_me: from_me,
                        sender: sender.clone().unwrap_or_default(),
                    },
                });
            }
            continue;
        }
        if let Some(update) = base.poll_update_message.as_option() {
            poll_updates.push(HistoryPollUpdate {
                id,
                sender,
                from_me,
                timestamp,
                update: update.clone(),
            });
            continue;
        }
        if let Some((parent, raw)) = motion_clip(base) {
            clips.push(HistoryClip {
                parent,
                sender,
                from_me,
                raw,
            });
            continue;
        }
        // An edit WhatsApp encrypted with the original's secret: the worker
        // opens it, and it is filed under the original's id like the plain
        // snapshots above (#582).
        let sealed = snapshot
            .is_none()
            .then(|| extract_envelope(original.get_base_message()))
            .flatten()
            .and_then(|envelope| {
                let target = envelope.target_id().filter(|id| !id.is_empty())?;
                Some((
                    target.to_owned(),
                    SealedEdit {
                        payload: envelope.enc_payload.to_vec(),
                        iv: envelope.enc_iv.to_vec(),
                        entry_secret: info.message_secret.clone(),
                    },
                ))
            });
        let (id, edited, sealed) = match sealed {
            Some((target, sealed)) => (target, true, Some(sealed)),
            None => (id, edited, None),
        };
        let content = match classify(message) {
            Some(content) => Some(content),
            // A placeholder the opened edit replaces.
            None if sealed.is_some() => Some(Content::Unsupported {
                what: "edited message".into(),
            }),
            None => None,
        };
        let Some(mut content) = content else {
            diagnostics.unclassified += 1;
            continue;
        };
        // Normalizing an edit must not remove an outer view-once restriction.
        if original.is_view_once()
            && let Some(kind) = crate::model::OnceMedia::of(&content)
        {
            content = Content::PhoneOnly {
                view_once: true,
                live_location: false,
                once: Some(kind),
            };
        }
        if matches!(content, Content::LiveLocation { .. })
            && let Some(last) = info.final_live_location.as_option()
        {
            content = finished_live_location(last, timestamp);
        }
        use wa::web_message_info::Status;
        let mut status = if from_me {
            match info.status {
                Some(Status::READ) => Delivery::Read,
                Some(Status::PLAYED) => Delivery::Played,
                Some(Status::DELIVERY_ACK) => Delivery::Delivered,
                Some(Status::SERVER_ACK) => Delivery::Sent,
                Some(Status::PENDING) => Delivery::Pending,
                Some(Status::ERROR) => Delivery::Failed,
                _ => Delivery::Sent,
            }
        } else if info.status == Some(Status::PLAYED) {
            // We played their voice message on another device.
            Delivery::Played
        } else {
            Delivery::None
        };
        // A group's individual receipts may be only a partial list. Only the
        // phone's aggregate status proves delivery/read for historical groups.
        // A zero receipt time proves nothing: WhatsApp sends one when a
        // contact does not share read receipts.
        if from_me
            && ChatKind::from_id(&conversation.id) != ChatKind::Group
            && status < Delivery::Read
        {
            if info
                .user_receipt
                .iter()
                .any(|receipt| receipt.read_timestamp.is_some_and(|at| at > 0))
            {
                status = Delivery::Read;
            } else if status < Delivery::Delivered
                && info
                    .user_receipt
                    .iter()
                    .any(|receipt| receipt.receipt_timestamp.is_some_and(|at| at > 0))
            {
                status = Delivery::Delivered;
            }
        }
        let quoted = history_quote(base);
        let reactions = info
            .reactions
            .iter()
            .filter_map(|reaction| {
                let text =
                    reaction_emoji(reaction.text.as_deref(), reaction.grouping_key.as_deref())?;
                let key = reaction.key.as_option();
                let from_me = key.and_then(|key| key.from_me).unwrap_or(false);
                let who = key.and_then(|key| key.participant.clone());
                Some((who, from_me, text))
            })
            .collect();
        messages.push(ParsedMessage {
            id,
            sender,
            from_me,
            push_name: non_empty(&info.push_name),
            timestamp,
            history_order: entry
                .msg_order_id
                .and_then(|order| i64::try_from(order).ok()),
            edited,
            content,
            status,
            quoted,
            reactions,
            mentions: recorded_mention_ids(base),
            forwarded: forwarded_of(base),
            thumbnail: thumbnail_of(base),
            raw: if original.is_view_once() {
                original
            } else {
                message
            }
            .encode_to_vec(),
            // An opened edit's secret is the one that opened it.
            poll_secret: if sealed.is_some() {
                None
            } else {
                info.message_secret.clone().or_else(|| {
                    original
                        .message_context_info
                        .as_option()
                        .and_then(|context| context.message_secret.clone())
                        .or_else(|| {
                            original
                                .get_base_message()
                                .message_context_info
                                .as_option()
                                .and_then(|context| context.message_secret.clone())
                        })
                })
            },
            poll_votes: info.poll_updates.clone(),
            receipts: if from_me {
                info.user_receipt.clone()
            } else {
                Vec::new()
            },
            sealed_edit: sealed,
        });
    }
    let last_activity = conversation
        .conversation_timestamp
        .or(conversation.last_msg_timestamp)
        .map(|timestamp| timestamp as i64)
        .unwrap_or(0)
        .max(newest);
    use wa::conversation::EndOfHistoryTransferType as End;
    let more_on_phone = conversation
        .end_of_history_transfer_type
        .and_then(|end| match end {
            End::COMPLETE_BUT_MORE_MESSAGES_REMAIN_ON_PRIMARY
            | End::COMPLETE_ON_DEMAND_SYNC_BUT_MORE_MSG_REMAIN_ON_PRIMARY => Some(true),
            End::COMPLETE_AND_NO_MORE_MESSAGE_REMAIN_ON_PRIMARY
            | End::COMPLETE_ON_DEMAND_SYNC_WITH_MORE_MSG_ON_PRIMARY_BUT_NO_ACCESS => Some(false),
            // A transfer type added after this was written says nothing either way.
            _ => None,
        });
    ParsedChat {
        diagnostics,
        id: conversation.id.clone(),
        name: non_empty(&conversation.display_name).or_else(|| non_empty(&conversation.name)),
        unread: conversation.unread_count,
        marked_unread: conversation.marked_as_unread,
        archived: conversation.archived,
        pinned_at: conversation.pinned.map(|when| i64::from(when) * 1000),
        muted_until: conversation.mute_end_time.map(|end| {
            // Zero explicitly clears a history mute; a wrapped -1 means
            // indefinite. Absence of the field must preserve existing state.
            (end != 0).then(|| seconds(end as i64))
        }),
        ephemeral_expiration: conversation.ephemeral_expiration,
        ephemeral_setting_timestamp: conversation.ephemeral_setting_timestamp,
        locked: conversation.locked,
        last_activity,
        pn_jid: conversation.pn_jid.clone(),
        lid_jid: conversation.lid_jid.clone(),
        more_on_phone,
        messages,
        revoked,
        clips,
        poll_updates,
        reactions,
        pins,
    }
}

/// The message a pin names, whether it pins or unpins it, and for how long.
fn pin_action(pin: &wa::message::PinInChatMessage, duration: u32) -> Option<(String, bool, u32)> {
    let id = pin
        .key
        .as_option()
        .and_then(|key| key.id.clone())
        .filter(|id| !id.is_empty())?;
    let pinned = !matches!(
        pin.r#type,
        Some(wa::message::pin_in_chat_message::Type::UnpinForAll)
    );
    Some((id, pinned, duration))
}

/// Displayed emoji for a reaction: `text`, else `groupingKey` when text is empty.
fn reaction_emoji(text: Option<&str>, grouping_key: Option<&str>) -> Option<String> {
    [text, grouping_key]
        .into_iter()
        .flatten()
        .map(str::trim)
        .find(|emoji| !emoji.is_empty())
        .map(str::to_owned)
}

fn message_secret_from_raw(raw: &[u8]) -> Option<Vec<u8>> {
    let message = wa::Message::decode_from_slice(raw).ok()?;
    let base = message.get_base_message();
    message
        .message_context_info
        .as_option()
        .or(base.message_context_info.as_option())
        .and_then(|context| context.message_secret.clone())
        .filter(|secret| secret.len() == 32)
}

fn ensure_message_secret(raw: Vec<u8>, secret: Option<&[u8]>) -> Vec<u8> {
    let Some(secret) = secret.filter(|secret| secret.len() == 32) else {
        return raw;
    };
    if message_secret_from_raw(&raw).is_some() {
        return raw;
    }
    let Ok(mut message) = wa::Message::decode_from_slice(&raw) else {
        return raw;
    };
    let mut context = message
        .message_context_info
        .into_option()
        .unwrap_or_default();
    context.message_secret = Some(secret.to_vec());
    message.message_context_info = MessageField::some(context);
    message.encode_to_vec()
}

/// The newest message the archive holds for a chat: the boundary the phone is
/// asked to clear through. An archive that cannot be read yields no boundary at
/// all, because a guessed one would clear the phone past messages this device
/// never saw, and the dialog would say both sides matched.
fn clear_boundary(read: crate::archive::Result<Vec<Message>>) -> Option<i64> {
    read.ok().map(|page| {
        page.last()
            .map_or_else(crate::util::now, |message| message.timestamp)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[tokio::test]
    async fn a_lost_socket_without_a_disconnect_event_starts_reconnecting() {
        let (mut worker, events, _commands, _wa) = super::receipt_tests::worker();
        let directory = tempfile::tempdir().unwrap();
        let store = whatsapp_rust::store::SqliteStore::open(
            directory.path().join("session.db").to_str().unwrap(),
        )
        .await
        .unwrap();
        let bot = Bot::builder().with_backend(store).build().await.unwrap();
        let client = bot.client();
        assert!(!client.is_socket_connected());
        worker.client = Some(client);
        worker.status = LinkStatus::Connected;

        worker.watch_link();
        assert!(matches!(worker.status, LinkStatus::Connecting));
        assert!(matches!(
            events.try_recv().unwrap(),
            Event::Link(LinkStatus::Connecting)
        ));
        worker.watch_link();
        assert!(events.try_recv().is_err(), "only one recovery is requested");
    }

    /// A failed number check leaves contacts and conversation state unchanged.
    #[tokio::test]
    async fn failed_number_check_never_saves_a_contact_or_opens_a_chat() {
        let (mut worker, events, mut commands, _) = super::receipt_tests::worker();
        worker
            .handle_command(Command::ContactChecked {
                generation: 0,
                request: 42,
                phone: "15550000001".into(),
                full_name: Some("Fixture".into()),
                first_name: None,
                to_phone: true,
                registered: Err("Synthetic network failure".into()),
            })
            .await;
        assert!(commands.try_recv().is_err());
        let failures: Vec<_> = events.try_iter().collect();
        assert!(matches!(
            failures.as_slice(),
            [Event::ContactFailed { request: 42, .. }]
        ));
        assert!(
            worker
                .archive
                .contact("15550000001@s.whatsapp.net")
                .unwrap()
                .is_none()
        );
    }

    /// Malformed recipients are rejected before any number lookup is dispatched.
    #[tokio::test]
    async fn malformed_numbers_are_refused_before_contact_lookup() {
        let (mut worker, events, _, _) = super::receipt_tests::worker();
        worker
            .handle_command(Command::NewContact {
                request: 42,
                phone: "+1 (555) 123-4567 ext. 89".into(),
                full_name: None,
                first_name: None,
                to_phone: false,
            })
            .await;
        assert!(events.try_iter().any(|event| matches!(event, Event::ContactFailed { request: 42, error } if error == "Enter a valid international phone number")));
    }

    /// A lookup completion from an earlier account session is discarded.
    #[tokio::test]
    async fn contact_completions_cannot_cross_account_sessions() {
        for full_name in [None, Some("Fixture".to_owned())] {
            let (mut worker, events, _, _) = super::receipt_tests::worker();
            let generation = worker.contact_generation;
            worker.on_logged_out().await;
            events.try_iter().for_each(drop);
            worker
                .handle_command(Command::ContactChecked {
                    generation,
                    request: 42,
                    phone: "15550000001".into(),
                    full_name,
                    first_name: None,
                    to_phone: true,
                    registered: Ok(true),
                })
                .await;
            worker
                .handle_command(Command::ContactSaved {
                    generation,
                    id: "15550000001@s.whatsapp.net".into(),
                    name: "Fixture".into(),
                    first_name: None,
                    error: None,
                })
                .await;
            assert!(events.try_iter().next().is_none());
            assert!(
                worker
                    .archive
                    .contact("15550000001@s.whatsapp.net")
                    .unwrap()
                    .is_none()
            );
        }
    }

    /// A reconnect preserves valid lookups belonging to the same account.
    #[tokio::test]
    async fn reconnecting_the_same_account_keeps_contact_checks_current() {
        let (mut worker, _, _, _) = super::receipt_tests::worker();
        let generation = worker.contact_generation;
        worker.stop_bot().await;
        assert_eq!(worker.contact_generation, generation);
    }

    /// An album member's bare protobuf: only the association naming its parent.
    fn album_member(parent: &str) -> wa::Message {
        let mut message = wa::Message::default();
        message.message_context_info = MessageField::some({
            let mut context = wa::MessageContextInfo::default();
            context.message_association = MessageField::some({
                let mut association = wa::MessageAssociation::default();
                association.association_type =
                    Some(wa::message_association::AssociationType::MEDIA_ALBUM);
                association.parent_message_key = MessageField::some({
                    let mut key = wa::MessageKey::default();
                    key.id = Some(parent.into());
                    key
                });
                association
            });
            context
        });
        message
    }

    /// Checks that album export never overwrites existing files or colliding attachment names.
    #[test]
    fn saving_an_album_preserves_existing_files_and_repeated_names() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source.png");
        let folder = root.path().join("saved");
        std::fs::create_dir(&folder).unwrap();
        std::fs::write(&source, b"synthetic picture").unwrap();
        std::fs::write(folder.join("photo.png"), b"keep this file").unwrap();
        let first = save_album_copy(&source, &folder, "photo.png").unwrap();
        let second = save_album_copy(&source, &folder, "photo.png").unwrap();
        assert_eq!(first.file_name().unwrap(), "photo (2).png");
        assert_eq!(second.file_name().unwrap(), "photo (3).png");
        assert_eq!(
            std::fs::read(folder.join("photo.png")).unwrap(),
            b"keep this file"
        );
        assert_eq!(std::fs::read(first).unwrap(), b"synthetic picture");
        assert_eq!(std::fs::read(second).unwrap(), b"synthetic picture");
    }

    /// A failed derived-row write keeps the album backfill retryable and preserves local paths.
    #[test]
    fn failed_album_backfill_retries_without_losing_downloaded_media() {
        let (mut worker, _events, _inbox, _wa) = super::receipt_tests::worker();
        let chat = "album-fixture@g.us";
        worker.archive.ensure_chat(chat, "Fixture").unwrap();
        let mut row = crate::archive::tests::message(chat, "photo", 100, false);
        let path = PathBuf::from("synthetic-photo.png");
        let mut attachment = media(None, None, None, None);
        attachment.path = Some(path.clone());
        row.content = Content::Image {
            motion: None,
            caption: Some("Synthetic caption".into()),
            media: attachment,
        };
        let raw = album_member("album-parent");
        worker
            .archive
            .insert_message(&row, Some(&raw.encode_to_vec()))
            .unwrap();
        crate::archive::tests::set_derived_update_failure(&worker.archive, true);
        worker.backfill_media_albums();
        assert!(worker.archive.meta("media_albums").unwrap().is_none());
        crate::archive::tests::set_derived_update_failure(&worker.archive, false);
        worker.backfill_media_albums();
        assert_eq!(
            worker.archive.meta("media_albums").unwrap().as_deref(),
            Some("1")
        );
        let stored = worker.archive.message(chat, "photo").unwrap().unwrap();
        assert_eq!(
            stored.content.media().unwrap().album.as_deref(),
            Some("album-parent")
        );
        assert_eq!(stored.content.media().unwrap().path.as_ref(), Some(&path));
    }

    /// Checks that derived media retains the protocol album parent used for grouping.
    #[test]
    fn album_parent_is_kept_on_derived_media() {
        let parent = "album-parent";
        let base = album_member(parent);
        assert_eq!(
            album_media(&base, media(None, None, None, None))
                .album
                .as_deref(),
            Some(parent)
        );
        assert_eq!(
            album_media(&wa::Message::default(), media(None, None, None, None)).album,
            None
        );
    }

    /// A motion photo sent inside an album is one album member with its clip,
    /// and the clip, which reads the same association field, never becomes a
    /// member or a message of its own.
    #[test]
    fn a_motion_photo_in_an_album_is_one_member_and_its_clip_is_not() {
        use wa::message_association::AssociationType;
        let mut photo = album_member("album-parent");
        photo.image_message = MessageField::some({
            let mut image = wa::message::ImageMessage::default();
            image.mimetype = Some("image/jpeg".into());
            image.context_info = MessageField::some({
                let mut context = wa::ContextInfo::default();
                context.paired_media_type =
                    Some(wa::context_info::PairedMediaType::MOTION_PHOTO_PARENT);
                context
            });
            image
        });
        let Some(Content::Image { media, motion, .. }) = classify(&photo) else {
            panic!("the photo is a picture");
        };
        assert_eq!(media.album.as_deref(), Some("album-parent"));
        assert!(motion.is_some(), "the photo keeps its motion clip");

        // The clip names the photo, not the album, as its parent.
        let mut clip = album_member("photo");
        clip.message_context_info
            .as_option_mut()
            .and_then(|context| context.message_association.as_option_mut())
            .unwrap()
            .association_type = Some(AssociationType::MOTION_PHOTO);
        clip.associated_child_message = MessageField::some({
            let mut wrapper = wa::message::FutureProofMessage::default();
            wrapper.message = MessageField::some({
                let mut child = wa::Message::default();
                child.video_message = MessageField::some({
                    let mut video = wa::message::VideoMessage::default();
                    video.mimetype = Some("video/mp4".into());
                    video
                });
                child
            });
            wrapper
        });
        assert_eq!(classify(&clip), None, "the clip is not a message");
        assert_eq!(
            motion_clip(&clip).map(|(parent, _)| parent).as_deref(),
            Some("photo")
        );
        // Even with the album's association on its wrapper, the paired clip
        // stays the photo's clip.
        let mut wrapped = clip.clone();
        wrapped
            .message_context_info
            .as_option_mut()
            .and_then(|context| context.message_association.as_option_mut())
            .unwrap()
            .association_type = Some(AssociationType::MEDIA_ALBUM);
        wrapped
            .associated_child_message
            .as_option_mut()
            .and_then(|wrapper| wrapper.message.as_option_mut())
            .and_then(|child| child.video_message.as_option_mut())
            .unwrap()
            .context_info = MessageField::some({
            let mut context = wa::ContextInfo::default();
            context.paired_media_type = Some(wa::context_info::PairedMediaType::MOTION_PHOTO_CHILD);
            context
        });
        assert_eq!(classify(&wrapped), None);
    }

    #[test]
    fn only_phone_playable_audio_is_sent_as_an_audio_message() {
        let sent_as = |name: &str| {
            let mime = mime_guess2::from_path(name)
                .first_or_octet_stream()
                .to_string();
            whatsapp_audio_mime(&mime)
        };
        assert_eq!(sent_as("song.mp3"), Some("audio/mpeg"));
        assert_eq!(sent_as("memo.m4a"), Some("audio/mp4"));
        assert_eq!(sent_as("clip.aac"), Some("audio/aac"));
        assert_eq!(sent_as("note.ogg"), Some("audio/ogg"));
        assert_eq!(sent_as("note.opus"), Some("audio/ogg"));
        // These would arrive as an unplayable audio message converted on the
        // phone, not as the file that was attached (#162).
        for document in ["take.wav", "album.flac", "loop.aiff", "old.wma", "x.weba"] {
            assert_eq!(sent_as(document), None, "{document}");
        }
    }

    #[test]
    fn a_played_receipt_reaches_our_own_devices_with_read_receipts_off() {
        // Read receipts on: the sender is told.
        assert!(played_receipt_goes(true, false));
        // Off, and whatsapp-rust knows: it sends played-self, to our own
        // devices only, so the phone marks the message heard (#501).
        assert!(played_receipt_goes(false, true));
        // Off, but the library does not know yet: it would tell the sender.
        assert!(!played_receipt_goes(false, false));
    }

    #[test]
    fn only_mp4_video_is_sent_as_a_video_message() {
        let video = |name: &str| {
            let mime = mime_guess2::from_path(name)
                .first_or_octet_stream()
                .to_string();
            whatsapp_video_mime(&mime)
        };
        assert!(video("clip.mp4"));
        // WhatsApp drops these as video messages, so the recipient got
        // nothing (#553); they go as documents instead.
        for document in ["clip.mov", "clip.webm", "clip.mkv", "clip.avi", "clip.wmv"] {
            assert!(!video(document), "{document}");
        }
    }

    #[tokio::test]
    async fn stalled_attachments_finish_with_a_retryable_error() {
        let result = with_attachment_deadline(
            Duration::from_millis(1),
            std::future::pending::<Result<(), String>>(),
        )
        .await;
        assert_eq!(result.unwrap_err(), "Download timed out");
        assert_eq!(
            with_attachment_deadline(Duration::from_secs(1), async { Ok(42) }).await,
            Ok(42)
        );
    }

    #[test]
    fn a_staging_file_is_removed_when_its_download_ends_unpublished() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("photo.jpg");
        let (staging, mut file) = temporary_attachment_file(&path).unwrap();
        // A download dropped between writing and publishing leaves nothing,
        // however it ends; the guard removes the file even then.
        file.write_all(b"half").unwrap();
        drop(file);
        assert!(staging.is_file());
        drop(Staged(staging.clone()));
        assert!(!staging.exists());
    }

    #[tokio::test]
    async fn a_deadline_that_cancels_a_download_removes_its_staging_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("photo.jpg");
        let (staging, file) = temporary_attachment_file(&path).unwrap();
        drop(file);
        let held = staging.clone();
        let result: Result<(), String> =
            with_attachment_deadline(Duration::from_millis(1), async move {
                // As in `fetch_attachment`: the guard outlives the writer, and
                // the deadline cancels the future while it is in flight.
                let _staged = Staged(held);
                std::future::pending::<()>().await;
                unreachable!()
            })
            .await;
        assert_eq!(result.unwrap_err(), "Download timed out");
        assert!(!staging.exists());
    }

    #[tokio::test]
    async fn a_published_file_survives_its_staging_removal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("photo.jpg");
        let (staging, file) = temporary_attachment_file(&path).unwrap();
        drop(file);
        publish_attachment(&staging, &path).await.unwrap();
        drop(Staged(staging));
        assert!(path.is_file());
    }

    /// Builds a synthetic message carrying protocol quote context for parser regression tests.
    fn message_quoting(sender: &str, sender_name: Option<&str>) -> Message {
        Message {
            id: "message".into(),
            chat: "15550001111@s.whatsapp.net".into(),
            sender: "15550001111@s.whatsapp.net".into(),
            sender_name: None,
            from_me: true,
            timestamp: 1,
            content: Content::text("reply"),
            status: Delivery::Sent,
            delivered_at: None,
            read_at: None,
            quoted: Some(Quoted {
                labelled: None,
                chat: None,
                id: "quoted".into(),
                sender: sender.into(),
                sender_name: sender_name.map(str::to_owned),
                summary: "quoted message".into(),
                mentions: Vec::new(),
            }),
            reactions: Vec::new(),
            history_order: None,
            edited: false,
            mentions: Vec::new(),
            forwarded: false,
            thumbnail: None,
        }
    }

    #[test]
    fn a_contact_update_keeps_its_whole_first_name() {
        const ID: &str = "15551234568@s.whatsapp.net";
        let (mut worker, _events, _inbox, _wa) = receipt_tests::worker();
        let update = |full: &str, first: &str| {
            wa_events::ContactUpdate::builder()
                .jid(Jid::pn("15551234568"))
                .timestamp(whatsapp_rust::wacore::time::from_millis_or_now(1))
                .action(Box::new({
                    let mut proto = wa::sync_action_value::ContactAction::default();
                    proto.full_name = Some(full.into());
                    proto.first_name = Some(first.into());
                    proto
                }))
                .from_full_sync(false)
                .build()
        };
        worker.on_contact_update(&update("My Dih", "My Dih"));
        let stored = worker.archive.contact(ID).expect("reads").expect("stored");
        assert_eq!(stored.first_name.as_deref(), Some("My Dih"));
        assert_eq!(worker.contacts[ID].first_name.as_deref(), Some("My Dih"));

        worker.on_contact_update(&update("My Dih", ""));
        assert_eq!(
            worker.contacts[ID].first_name, None,
            "an empty first name is none"
        );
        let stored = worker.archive.contact(ID).expect("reads").expect("stored");
        assert_eq!(stored.first_name, None);
    }

    #[test]
    fn the_contact_replay_brings_first_names_saved_before_they_were_kept() {
        const ID: &str = "15551234568@s.whatsapp.net";
        let (mut worker, _events, _inbox, _wa) = receipt_tests::worker();
        worker.first_names_recovered = false;
        // Synced before first names were kept: the full name alone.
        let contact = Contact {
            id: ID.into(),
            full_name: Some("Mary Ann Evans".into()),
            first_name: None,
            push_name: None,
        };
        worker.archive.upsert_contact(&contact).expect("stores");
        worker.contacts.insert(ID.into(), contact);
        // The snapshot replays the contact with its first name.
        worker.on_contact_update(
            &wa_events::ContactUpdate::builder()
                .jid(Jid::pn("15551234568"))
                .timestamp(whatsapp_rust::wacore::time::from_millis_or_now(1))
                .action(Box::new({
                    let mut proto = wa::sync_action_value::ContactAction::default();
                    proto.full_name = Some("Mary Ann Evans".into());
                    proto.first_name = Some("Mary Ann".into());
                    proto
                }))
                .from_full_sync(true)
                .build(),
        );
        let stored = worker.archive.contact(ID).expect("reads").expect("stored");
        assert_eq!(stored.first_name.as_deref(), Some("Mary Ann"));

        // An unfinished replay is asked again on the next connection.
        worker.first_names_recovering = true;
        worker.first_names_recovered(false);
        assert!(!worker.first_names_recovered && !worker.first_names_recovering);
        assert_eq!(
            worker
                .archive
                .meta(contact_names::FIRST_NAMES_RECOVERED)
                .unwrap(),
            None
        );
        // A finished one is never asked again.
        worker.first_names_recovered(true);
        assert!(worker.first_names_recovered);
        assert_eq!(
            worker
                .archive
                .meta(contact_names::FIRST_NAMES_RECOVERED)
                .unwrap()
                .as_deref(),
            Some("complete")
        );
    }

    #[test]
    fn polish_refreshes_a_stale_quote_label_when_the_sender_id_is_unchanged() {
        const SENDER: &str = "15551234567@s.whatsapp.net";
        let (mut worker, _events, _inbox, _wa) = receipt_tests::worker();
        worker.contacts.insert(
            SENDER.into(),
            Contact {
                id: SENDER.into(),
                full_name: Some("Current Contact".into()),
                first_name: None,
                push_name: None,
            },
        );
        let mut message = message_quoting(SENDER, Some("+1 555 123 456 7"));

        worker.polish(&mut message);

        let quoted = message.quoted.expect("quote");
        assert_eq!(quoted.sender, SENDER);
        assert_eq!(quoted.sender_name.as_deref(), Some("Current Contact"));
    }

    #[test]
    fn polish_preserves_an_archived_quote_label_for_an_unmapped_lid() {
        const SENDER: &str = "424242@lid";
        let (worker, _events, _inbox, _wa) = receipt_tests::worker();
        let mut message = message_quoting(SENDER, Some("~Archived Sender"));

        worker.polish(&mut message);

        let quoted = message.quoted.expect("quote");
        assert_eq!(quoted.sender, SENDER);
        assert_eq!(quoted.sender_name.as_deref(), Some("~Archived Sender"));
    }

    /// A group reply quoting a photo whose caption mentions a member by
    /// privacy id, as WhatsApp sends it in a group that uses them.
    fn reply_quoting_a_lid_mention(listed: bool) -> wa::Message {
        use whatsapp_rust::prelude::MessageField;
        let quoted = {
            let mut proto = wa::Message::default();
            proto.image_message = MessageField::some({
                let mut proto = wa::message::ImageMessage::default();
                proto.caption = Some("@987654321012345 looks sharp".into());
                proto.context_info = if listed {
                    MessageField::some({
                        let mut proto = wa::ContextInfo::default();
                        proto.mentioned_jid = vec!["987654321012345@lid".into()];
                        proto
                    })
                } else {
                    MessageField::none()
                };
                proto
            });
            proto
        };
        outgoing_text(
            "agreed".into(),
            Some({
                let mut proto = wa::ContextInfo::default();
                proto.stanza_id = Some("photo".into());
                proto.participant = Some("15550002222@s.whatsapp.net".into());
                proto.quoted_message = MessageField::some(quoted);
                proto
            }),
            &[],
        )
    }

    #[test]
    fn quoted_privacy_id_mentions_follow_the_summary_to_the_phone_number() {
        const LID: &str = "987654321012345";
        const PN: &str = "15550003333";
        for listed in [true, false] {
            let (mut worker, _events, _inbox, _wa) = receipt_tests::worker();
            worker.lid_to_pn.insert(LID.into(), PN.into());
            let quoted = worker
                .quoted_of(&reply_quoting_a_lid_mention(listed))
                .expect("quote");
            assert_eq!(quoted.summary, format!("Photo: @{PN} looks sharp"));
            assert_eq!(
                quoted.mentions,
                vec![MentionRef {
                    user: PN.into(),
                    id: format!("{PN}@s.whatsapp.net"),
                }],
                "listed: {listed}"
            );
            let named = crate::markup::plain(
                &quoted.summary,
                &[crate::markup::Mention {
                    user: quoted.mentions[0].user.clone(),
                    name: "Mira".into(),
                }],
            );
            assert_eq!(named, "Photo: @Mira looks sharp");
        }
    }

    #[test]
    fn polish_matches_archived_privacy_id_quote_mentions_to_the_summary() {
        const LID: &str = "987654321012345";
        const PN: &str = "15550003333";
        let (mut worker, _events, _inbox, _wa) = receipt_tests::worker();
        let mut message = message_quoting("15550002222@s.whatsapp.net", None);
        if let Some(quoted) = message.quoted.as_mut() {
            quoted.summary = format!("Photo: @{LID} looks sharp");
            quoted.mentions = vec![MentionRef {
                user: LID.into(),
                id: format!("{LID}@lid"),
            }];
        }

        // Before the mapping is known the privacy id still matches itself.
        worker.polish(&mut message);
        let quoted = message.quoted.as_ref().expect("quote");
        assert_eq!(quoted.mentions[0].user, LID);
        assert!(quoted.summary.contains(&format!("@{LID}")));

        worker.lid_to_pn.insert(LID.into(), PN.into());
        worker.polish(&mut message);
        let quoted = message.quoted.expect("quote");
        assert_eq!(quoted.summary, format!("Photo: @{PN} looks sharp"));
        assert_eq!(
            quoted.mentions,
            vec![MentionRef {
                user: PN.into(),
                id: format!("{PN}@s.whatsapp.net"),
            }]
        );
    }

    #[test]
    fn fallback_names_read_as_phones_or_ids() {
        assert_eq!(
            fallback_name("393331234567@s.whatsapp.net"),
            "+39 333 123 4567"
        );
        assert_eq!(fallback_name("1-2@g.us"), "");
        assert_eq!(fallback_name("42@lid"), "42");
    }

    #[test]
    fn limited_writer_never_grows_past_its_cap() {
        let mut writer = LimitedWriter::new(Cursor::new(Vec::new()), 3);
        writer.write_all(b"abc").expect("writes through the cap");
        let error = writer
            .write_all(b"d")
            .expect_err("rejects bytes past the cap");
        assert_eq!(error.to_string(), ATTACHMENT_LIMIT_ERROR);
        writer.truncate(0).expect("clears a failed attempt");
        writer
            .seek(SeekFrom::Start(0))
            .expect("rewinds after clearing");
        writer.write_all(b"xyz").expect("can retry after clearing");
    }

    #[test]
    fn the_download_deadline_grows_with_the_file() {
        assert_eq!(attachment_deadline(None), ATTACHMENT_TIMEOUT);
        assert_eq!(attachment_deadline(Some(1024)), ATTACHMENT_TIMEOUT);
        // 64 MiB at the slowest allowed rate takes 512 s more.
        assert_eq!(
            attachment_deadline(Some(ATTACHMENT_DOWNLOAD_LIMIT)),
            ATTACHMENT_TIMEOUT + Duration::from_secs(512)
        );
        let video = attachment_deadline(Some(2 * 1024 * 1024 * 1024));
        assert!(video > Duration::from_secs(4 * 60 * 60), "{video:?}");
    }

    #[test]
    fn attachment_limit_rejects_only_oversized_metadata() {
        assert!(!attachment_is_too_large(None));
        assert!(!attachment_is_too_large(Some(ATTACHMENT_DOWNLOAD_LIMIT)));
        assert!(attachment_is_too_large(Some(ATTACHMENT_DOWNLOAD_LIMIT + 1)));
    }

    #[test]
    fn attachment_staging_files_are_hidden_and_exclusive() {
        let directory = std::env::temp_dir().join(format!(
            "zapfast-attachment-staging-{}",
            rand::random::<u64>()
        ));
        std::fs::create_dir_all(&directory).expect("creates staging directory");
        let destination = directory.join("photo.jpg");
        let (first_path, first) = temporary_attachment_file(&destination).expect("first file");
        let (second_path, second) = temporary_attachment_file(&destination).expect("second file");
        assert_ne!(first_path, second_path);
        assert!(
            first_path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with('.')
        );
        assert!(!destination.exists());
        drop((first, second));
        std::fs::write(&destination, b"complete attachment").expect("writes completed file");
        discard_attachment_staging(&directory);
        assert!(!first_path.exists());
        assert!(!second_path.exists());
        assert_eq!(
            std::fs::read(&destination).expect("reads completed file"),
            b"complete attachment"
        );
        std::fs::remove_dir_all(&directory).expect("removes staging directory");
    }

    #[test]
    fn media_paths_keep_document_names_and_map_mimes() {
        let dir = Path::new("/cache");
        assert_eq!(
            media_path(dir, "1@s.whatsapp.net", "ABC", "image/jpeg", None),
            PathBuf::from("/cache/1_s_whatsapp_net-ABC.jpg")
        );
        assert_eq!(
            media_path(
                dir,
                "1@s.whatsapp.net",
                "ABC",
                "application/pdf",
                Some("tax return.pdf")
            ),
            PathBuf::from("/cache/ABC-tax_return.pdf")
        );
        assert_eq!(extension_for("audio/ogg; codecs=opus", None), "ogg");
        assert_eq!(extension_for("application/x-unknown", None), "x-unknown");
    }

    #[test]
    fn documents_keep_their_names_in_the_download_folder() {
        let dir = Path::new("/downloads");
        let named = |name| named_media_path(dir, "application/pdf", name);
        assert_eq!(
            named("Tax return 2026.pdf"),
            Some(PathBuf::from("/downloads/Tax return 2026.pdf"))
        );
        assert_eq!(
            named("Résumé – Ada.PDF"),
            Some(PathBuf::from("/downloads/Résumé – Ada.pdf"))
        );
        assert_eq!(named("notes"), Some(PathBuf::from("/downloads/notes.pdf")));
        assert_eq!(
            named(r"..\..\evil:name?.pdf"),
            Some(PathBuf::from("/downloads/evil_name_.pdf"))
        );
        assert_eq!(
            named("../../.bashrc.pdf"),
            Some(PathBuf::from("/downloads/bashrc.pdf"))
        );
        assert_eq!(named("con.pdf"), Some(PathBuf::from("/downloads/_con.pdf")));
        assert_eq!(
            named("COM1.txt.pdf"),
            Some(PathBuf::from("/downloads/_COM1.txt.pdf"))
        );
        assert_eq!(named("..."), None);
        assert_eq!(named(" .pdf"), None);
        let long = "a".repeat(300) + ".pdf";
        let path = named(&long).unwrap();
        assert_eq!(path.file_name().unwrap().len(), 124);
        for hostile in [
            "../x.pdf",
            r"..\x.pdf",
            "/etc/x.pdf",
            r"C:\x.pdf",
            "a/b/c.pdf",
        ] {
            let path = named(hostile).unwrap();
            assert_eq!(path.parent(), Some(dir), "{hostile}");
        }
    }

    #[test]
    fn numbered_copies_follow_the_file_managers() {
        let path = Path::new("/d/report.pdf");
        assert_eq!(numbered_path(path, 0), PathBuf::from("/d/report.pdf"));
        assert_eq!(numbered_path(path, 2), PathBuf::from("/d/report (2).pdf"));
        assert_eq!(
            numbered_path(Path::new("/d/notes"), 1),
            PathBuf::from("/d/notes (1)")
        );
    }

    #[tokio::test]
    async fn a_download_never_replaces_a_file_in_the_folder() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("report.pdf");
        std::fs::write(&path, b"mine").unwrap();
        let mut landed = Vec::new();
        for body in [b"first".as_slice(), b"second"] {
            let (temporary, _) = temporary_attachment_file(&path).unwrap();
            std::fs::write(&temporary, body).unwrap();
            landed.push(publish_beside(&temporary, &path).await.unwrap());
            assert!(!temporary.exists());
        }
        assert_eq!(std::fs::read(&path).unwrap(), b"mine");
        assert_eq!(landed[0], dir.path().join("report (1).pdf"));
        assert_eq!(landed[1], dir.path().join("report (2).pdf"));
        assert_eq!(std::fs::read(&landed[1]).unwrap(), b"second");
    }

    #[test]
    fn media_extensions_reject_path_syntax_and_invalid_values() {
        for extension in [
            "",
            "../",
            "../outside",
            r"..\",
            r"..\outside",
            "/tmp/outside",
            r"\outside",
            "C:outside",
            "C:/outside",
            r"C:\outside",
            r"\\server\share\outside",
            "//server/share/outside",
            "jpg:stream",
            "tar.gz",
            "..jpg",
            ".",
            "..",
            " jpg",
            "jpg ",
            "j pg",
            "jpg\t",
            "jpg\n",
            "jp\0g",
            "pñg",
            "ｐｎｇ",
            "jpg_",
            "12345678901234567",
        ] {
            assert_eq!(safe_extension(extension), "bin", "{extension:?}");
            let mime = format!("application/{extension}");
            assert_eq!(extension_for(&mime, None), "bin", "{mime:?}");
        }
        for mime in ["", "png", "application", "application/"] {
            assert_eq!(extension_for(mime, None), "bin", "{mime:?}");
        }
        for name in [
            "file.",
            "file./outside",
            r"file.\outside",
            r"file.\..\outside",
            "file.C:outside",
            "file.C:/outside",
            r"file.C:\outside",
            r"file.\\host\share",
            "file.jpg:stream",
            "file.jpg ",
            "file.jp g",
            "file.jp\tg",
            "file.jp\ng",
            "file.jp\0g",
            "file.pñg",
            "file.jpg_",
            "file.12345678901234567",
            "file..",
        ] {
            // Invalid filename suffixes must not fall through to a valid MIME.
            assert_eq!(extension_for("image/jpeg", Some(name)), "bin", "{name:?}");
        }
    }

    #[test]
    fn media_extensions_preserve_common_formats_and_length_boundary() {
        for extension in [
            "jpg",
            "jpeg",
            "png",
            "webp",
            "gif",
            "mp4",
            "3gp",
            "ogg",
            "opus",
            "mp3",
            "m4a",
            "aac",
            "wav",
            "pdf",
            "txt",
            "docx",
            "xlsx",
            "zip",
            "x-unknown",
            "1234567890123456",
        ] {
            let uppercase = extension.to_ascii_uppercase();
            assert_eq!(safe_extension(&uppercase), extension);
            assert_eq!(
                extension_for(&format!("application/{uppercase}"), None),
                extension
            );
            assert_eq!(
                extension_for(
                    "application/octet-stream",
                    Some(&format!("file.{uppercase}"))
                ),
                extension
            );
        }
        // Multiple dots in a document's name are fine; only its last suffix is used.
        assert_eq!(
            extension_for("application/gzip", Some("archive.tar.gz")),
            "gz"
        );
        assert_eq!(extension_for("image/jpeg", Some("no-extension")), "jpg");
    }

    #[test]
    fn media_paths_and_staging_stay_direct_children_for_hostile_metadata() {
        let dir = Path::new("cache").join("media");
        for input in [
            "../",
            "../outside",
            r"..\",
            r"..\outside",
            "/tmp/outside",
            r"C:\outside",
            "C:outside",
            r"\\server\share\outside",
            "jpg:stream",
            "tar.gz",
            "",
            "12345678901234567",
            "pñg",
            "jpg ",
            r"photo.x\..\..\outside",
            "../photo.jpg",
            r"..\photo.jpg",
        ] {
            for name in [None, Some(input)] {
                let path = media_path(&dir, input, input, &format!("image/{input}"), name);
                for candidate in [path.clone(), temporary_attachment_path(&path)] {
                    assert_eq!(candidate.parent(), Some(dir.as_path()), "{candidate:?}");
                    let relative = candidate.strip_prefix(&dir).unwrap();
                    assert_eq!(relative.components().count(), 1);
                    // Enforce Windows safety even when these tests run on Unix.
                    let filename = relative.to_str().unwrap();
                    assert!(
                        filename.bytes().all(|byte| byte.is_ascii_alphanumeric()
                            || matches!(byte, b'_' | b'-' | b'.')),
                        "{filename:?}"
                    );
                    assert!(!filename.contains(".."));
                    assert!(!filename.ends_with('.'));
                }
                assert_eq!(
                    path.file_name()
                        .unwrap()
                        .to_str()
                        .unwrap()
                        .matches('.')
                        .count(),
                    1
                );
            }
        }
    }

    #[test]
    fn classification_covers_text_and_media() {
        let text = wa::Message::text("hello");
        assert_eq!(classify(&text), Some(Content::text("hello")));
        let image = {
            let mut proto = wa::Message::default();
            proto.image_message = whatsapp_rust::prelude::MessageField::some({
                let mut proto = wa::message::ImageMessage::default();
                proto.caption = Some("look".into());
                proto.mimetype = Some("image/jpeg".into());
                proto.file_length = Some(10);
                proto.width = Some(4);
                proto.height = Some(3);
                proto.jpeg_thumbnail = Some(vec![0xff, 0xd8]);
                proto
            });
            proto
        };
        match classify(&image) {
            Some(Content::Image { caption, media, .. }) => {
                assert_eq!(caption.as_deref(), Some("look"));
                assert_eq!(media.mime, "image/jpeg");
                assert_eq!((media.width, media.height), (Some(4), Some(3)));
            }
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(thumbnail_of(&image), Some(vec![0xff, 0xd8]));
        assert_eq!(classify(&wa::Message::default()), None);
    }

    #[test]
    fn live_location_is_classified_separately_from_static_location() {
        let live = {
            let mut proto = wa::Message::default();
            proto.live_location_message = MessageField::some({
                let mut proto = wa::message::LiveLocationMessage::default();
                proto.degrees_latitude = Some(51.5074);
                proto.degrees_longitude = Some(-0.1278);
                proto.accuracy_in_meters = Some(24);
                proto.speed_in_mps = Some(1.4);
                proto.degrees_clockwise_from_magnetic_north = Some(90);
                proto.sequence_number = Some(7);
                proto.jpeg_thumbnail = Some(vec![0xff, 0xd8, 0xff]);
                proto
            });
            proto
        };
        match classify(&live) {
            Some(Content::LiveLocation {
                latitude,
                longitude,
                accuracy_m,
                speed_mps,
                heading_deg,
                sequence,
                ended,
                ..
            }) => {
                assert_eq!(latitude, 51.5074);
                assert_eq!(longitude, -0.1278);
                assert_eq!(accuracy_m, Some(24));
                assert_eq!(speed_mps, Some(1.4));
                assert_eq!(heading_deg, Some(90));
                assert_eq!(sequence, 7);
                assert!(!ended);
            }
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(thumbnail_of(&live), Some(vec![0xff, 0xd8, 0xff]));

        let start = {
            let mut proto = wa::Message::default();
            proto.location_message = MessageField::some({
                let mut proto = wa::message::LocationMessage::default();
                proto.degrees_latitude = Some(51.5);
                proto.degrees_longitude = Some(-0.12);
                proto.is_live = Some(true);
                proto
            });
            proto
        };
        assert!(matches!(
            classify(&start),
            Some(Content::LiveLocation {
                sequence: 0,
                ended: false,
                ..
            })
        ));

        let pinned = {
            let mut proto = wa::Message::default();
            proto.location_message = MessageField::some({
                let mut proto = wa::message::LocationMessage::default();
                proto.degrees_latitude = Some(51.5);
                proto.degrees_longitude = Some(-0.12);
                proto.name = Some("Ada's place".into());
                proto
            });
            proto
        };
        assert!(matches!(classify(&pinned), Some(Content::Location { .. })));
    }

    fn live_position(
        id: &str,
        at: i64,
        sequence: i64,
        latitude: f64,
        quoting: Option<&str>,
    ) -> (Arc<wa::Message>, MessageInfo) {
        const PEER: &str = super::receipt_tests::PEER;
        let message = {
            let mut proto = wa::Message::default();
            proto.live_location_message = MessageField::some({
                let mut proto = wa::message::LiveLocationMessage::default();
                proto.degrees_latitude = Some(latitude);
                proto.degrees_longitude = Some(-0.12);
                proto.sequence_number = Some(sequence);
                proto.jpeg_thumbnail = Some(vec![sequence as u8]);
                proto.context_info = quoting
                    .map(|id| {
                        MessageField::some({
                            let mut proto = wa::ContextInfo::default();
                            proto.stanza_id = Some(id.into());
                            proto
                        })
                    })
                    .unwrap_or_default();
                proto
            });
            proto
        };
        let info = MessageInfo {
            id: id.into(),
            source: MessageSource {
                chat: PEER.parse().unwrap(),
                sender: PEER.parse().unwrap(),
                ..Default::default()
            },
            timestamp: whatsapp_rust::wacore::time::from_secs(at).unwrap(),
            ..Default::default()
        };
        (Arc::new(message), info)
    }

    #[test]
    fn live_location_positions_move_one_row_per_share() {
        const PEER: &str = super::receipt_tests::PEER;
        let (mut worker, _events, _inbox, _wa) = super::receipt_tests::worker();
        let start = crate::util::now() - 3_600;
        let ingest = |worker: &mut Worker, position: (Arc<wa::Message>, MessageInfo)| {
            worker.ingest(&position.0, &position.1);
        };
        let rows = |worker: &Worker| worker.archive.messages(PEER, None, 100).unwrap();
        let share = |worker: &Worker| worker.archive.message(PEER, "start").unwrap().unwrap();

        ingest(&mut worker, live_position("start", start, 1, 51.0, None));
        // Positions with ids of their own, named or not, move the share.
        ingest(&mut worker, live_position("p2", start + 60, 2, 51.1, None));
        ingest(
            &mut worker,
            live_position("p3", start + 120, 3, 51.2, Some("start")),
        );
        // A late, older position changes nothing.
        ingest(
            &mut worker,
            live_position("p2-late", start + 130, 2, 51.1, Some("start")),
        );
        assert_eq!(rows(&worker).len(), 1);
        let moved = share(&worker);
        assert_eq!(moved.timestamp, start, "the share keeps its place");
        assert_eq!(moved.thumbnail, Some(vec![3]));
        assert!(matches!(
            moved.content,
            Content::LiveLocation {
                latitude: 51.2,
                sequence: 3,
                updated,
                ..
            } if updated == start + 120
        ));

        // After a long silence, an unnamed position starts a new share.
        ingest(
            &mut worker,
            live_position("again", start + 120 + LIVE_LOCATION_GAP + 1, 1, 52.0, None),
        );
        assert_eq!(rows(&worker).len(), 2);
        assert!(matches!(
            share(&worker).content,
            Content::LiveLocation { latitude: 51.2, .. }
        ));
    }

    #[test]
    fn a_live_location_that_quotes_a_message_does_not_replace_it() {
        const PEER: &str = super::receipt_tests::PEER;
        let (mut worker, _events, _inbox, _wa) = super::receipt_tests::worker();
        let now = crate::util::now();
        let text = {
            let mut proto = wa::Message::default();
            proto.conversation = Some("where are you?".into());
            proto
        };
        let (_, mut info) = live_position("question", now - 60, 0, 0.0, None);
        info.timestamp = whatsapp_rust::wacore::time::from_secs(now - 60).unwrap();
        worker.ingest(&Arc::new(text), &info);
        let (message, info) = live_position("answer", now, 1, 51.0, Some("question"));
        worker.ingest(&message, &info);
        assert!(matches!(
            worker
                .archive
                .message(PEER, "question")
                .unwrap()
                .unwrap()
                .content,
            Content::Text { .. }
        ));
        assert!(matches!(
            worker
                .archive
                .message(PEER, "answer")
                .unwrap()
                .unwrap()
                .content,
            Content::LiveLocation { .. }
        ));
    }

    /// What the phone sends linked devices in place of a live location: the
    /// content is masked, and only the stanza's `mediatype` says what it is.
    fn masked_live_location() -> Arc<wa::Message> {
        Arc::new({
            let mut proto = wa::Message::default();
            proto.placeholder_message = MessageField::some({
                let mut proto = wa::message::PlaceholderMessage::default();
                proto.r#type =
                    Some(wa::message::placeholder_message::PlaceholderType::MASK_LINKED_DEVICES);
                proto
            });
            proto.message_context_info = MessageField::some({
                let mut proto = wa::MessageContextInfo::default();
                proto.message_secret = Some(vec![7; 32]);
                proto
            });
            proto
        })
    }

    fn live_location_info(id: &str, at: i64, source: MessageSource) -> MessageInfo {
        MessageInfo {
            id: id.into(),
            source,
            timestamp: whatsapp_rust::wacore::time::from_secs(at).unwrap(),
            media_type: Some(EncMediaType::LiveLocation),
            ..Default::default()
        }
    }

    const GROUP: &str = "120363025246125888@g.us";

    /// A contact in a chat, our phone in our own chat, and our phone and a
    /// member in a group.
    fn live_location_sources(worker: &Worker) -> Vec<(&'static str, MessageSource, String)> {
        const PEER: &str = super::receipt_tests::PEER;
        let me = worker.me();
        let phone: Jid = me.parse().unwrap();
        vec![
            (
                "contact",
                MessageSource {
                    chat: PEER.parse().unwrap(),
                    sender: PEER.parse().unwrap(),
                    ..Default::default()
                },
                PEER.to_owned(),
            ),
            (
                "self",
                MessageSource {
                    chat: phone.clone(),
                    sender: phone.clone(),
                    is_from_me: true,
                    recipient: Some(phone.clone()),
                    ..Default::default()
                },
                me.clone(),
            ),
            (
                "group-mine",
                MessageSource {
                    chat: GROUP.parse().unwrap(),
                    sender: phone.clone(),
                    is_from_me: true,
                    is_group: true,
                    ..Default::default()
                },
                GROUP.to_owned(),
            ),
            (
                "group-member",
                MessageSource {
                    chat: GROUP.parse().unwrap(),
                    sender: PEER.parse().unwrap(),
                    is_group: true,
                    ..Default::default()
                },
                GROUP.to_owned(),
            ),
        ]
    }

    #[test]
    fn a_masked_live_location_shows_a_bubble_that_points_to_the_phone() {
        let (mut worker, _events, _inbox, _wa) = super::receipt_tests::worker();
        let now = crate::util::now();
        for (id, source, chat) in live_location_sources(&worker) {
            let from_me = source.is_from_me;
            worker.ingest(
                &masked_live_location(),
                &live_location_info(id, now - 600, source),
            );
            let stored = worker
                .archive
                .message(&chat, id)
                .unwrap()
                .unwrap_or_else(|| panic!("{id}: no bubble"));
            assert_eq!(
                stored.content,
                Content::PhoneOnly {
                    view_once: false,
                    live_location: true,
                    once: None,
                },
                "{id}"
            );
            assert_eq!(stored.from_me, from_me, "{id}");
            assert!(
                worker
                    .archive
                    .chats()
                    .unwrap()
                    .iter()
                    .any(|listed| listed.id == chat),
                "{id}: the chat is listed"
            );
        }
    }

    #[test]
    fn a_masked_live_location_keeps_one_bubble_per_share() {
        const PEER: &str = super::receipt_tests::PEER;
        let (mut worker, _events, _inbox, _wa) = super::receipt_tests::worker();
        let now = crate::util::now();
        let source = || MessageSource {
            chat: PEER.parse().unwrap(),
            sender: PEER.parse().unwrap(),
            ..Default::default()
        };
        worker.ingest(
            &masked_live_location(),
            &live_location_info("start", now - 600, source()),
        );
        worker.ingest(
            &masked_live_location(),
            &live_location_info("next", now - 540, source()),
        );
        assert_eq!(worker.archive.messages(PEER, None, 10).unwrap().len(), 1);
        // Something said in between ends the share's run of bubbles.
        let mut info = live_location_info("chat", now - 300, source());
        info.media_type = None;
        worker.ingest(
            &Arc::new({
                let mut proto = wa::Message::default();
                proto.conversation = Some("on my way".into());
                proto
            }),
            &info,
        );
        worker.ingest(
            &masked_live_location(),
            &live_location_info("again", now, source()),
        );
        assert_eq!(worker.archive.messages(PEER, None, 10).unwrap().len(), 3);
    }

    #[test]
    fn a_masked_position_marks_the_share_it_cannot_move() {
        const PEER: &str = super::receipt_tests::PEER;
        let (mut worker, _events, _inbox, _wa) = super::receipt_tests::worker();
        let start = crate::util::now() - 600;
        let source = || MessageSource {
            chat: PEER.parse().unwrap(),
            sender: PEER.parse().unwrap(),
            ..Default::default()
        };
        let ingest = |worker: &mut Worker, position: (Arc<wa::Message>, MessageInfo)| {
            worker.ingest(&position.0, &position.1);
        };
        // Whether the share's card says the phone holds the newer positions.
        let marked = |worker: &Worker| {
            let share = worker.archive.message(PEER, "start").unwrap().unwrap();
            match share.content {
                Content::LiveLocation { newer_on_phone, .. } => newer_on_phone,
                other => panic!("unexpected {other:?}"),
            }
        };
        // Ada shares where she is, and this device reads the position.
        ingest(&mut worker, live_position("start", start, 1, 51.0, None));
        assert!(!marked(&worker));
        // She moves, and the phone keeps the positions to itself.
        worker.ingest(
            &masked_live_location(),
            &live_location_info("masked", start + 60, source()),
        );
        assert_eq!(worker.archive.messages(PEER, None, 10).unwrap().len(), 1);
        assert!(
            marked(&worker),
            "the card says where the newer positions are"
        );
        // A position this device can read takes the notice back.
        ingest(
            &mut worker,
            live_position("p2", start + 120, 2, 51.1, Some("start")),
        );
        let rows = worker.archive.messages(PEER, None, 10).unwrap();
        assert_eq!(rows.len(), 1, "the share keeps its one row");
        assert!(!marked(&worker));
    }

    /// A history replay of the position a share started with reclassifies
    /// the row the archive already holds, and that fresh classification
    /// carries no newer readable position. It must not take back the notice
    /// that the phone keeps the newer ones; a share the phone reports as
    /// finished does.
    #[test]
    fn a_history_replay_keeps_the_live_location_notice() {
        const PEER: &str = super::receipt_tests::PEER;
        let (mut worker, _events, _inbox, _wa) = super::receipt_tests::worker();
        let start = 1_700_000_000;
        let source = || MessageSource {
            chat: PEER.parse().unwrap(),
            sender: PEER.parse().unwrap(),
            ..Default::default()
        };
        let (message, info) = live_position("start", start, 1, 51.0, None);
        worker.ingest(&message, &info);
        // The phone keeps the positions that follow to itself.
        worker.ingest(
            &masked_live_location(),
            &live_location_info("masked", start + 60, source()),
        );
        let marked = |worker: &Worker| {
            let share = worker.archive.message(PEER, "start").unwrap().unwrap();
            match share.content {
                Content::LiveLocation { newer_on_phone, .. } => newer_on_phone,
                other => panic!("unexpected {other:?}"),
            }
        };
        assert!(
            marked(&worker),
            "the card says where the newer positions are"
        );
        let replay = |worker: &mut Worker, finished: bool| {
            worker.apply_history(
                ParsedHistory {
                    chats: vec![parse_conversation({
                        let mut proto = wa::Conversation::default();
                        proto.id = PEER.into();
                        proto.messages = vec![{
                            let mut proto = wa::HistorySyncMsg::default();
                            proto.message = MessageField::some({
                                let mut proto = wa::WebMessageInfo::default();
                                proto.key = MessageField::some({
                                    let mut proto = wa::MessageKey::default();
                                    proto.id = Some("start".into());
                                    proto.from_me = Some(false);
                                    proto.remote_jid = Some(PEER.into());
                                    proto
                                });
                                proto.message = MessageField::some({
                                    let mut proto = wa::Message::default();
                                    proto.live_location_message = MessageField::some({
                                        let mut proto = wa::message::LiveLocationMessage::default();
                                        proto.degrees_latitude = Some(51.0);
                                        proto.degrees_longitude = Some(-0.12);
                                        proto.sequence_number = Some(1);
                                        proto.time_offset = finished.then_some(600);
                                        proto
                                    });
                                    proto
                                });
                                proto.message_timestamp = Some(start as u64);
                                proto.final_live_location = if finished {
                                    MessageField::some({
                                        let mut proto = wa::message::LiveLocationMessage::default();
                                        proto.degrees_latitude = Some(51.0);
                                        proto.degrees_longitude = Some(-0.12);
                                        proto.sequence_number = Some(1);
                                        proto.time_offset = Some(600);
                                        proto
                                    })
                                } else {
                                    MessageField::default()
                                };
                                proto
                            });
                            proto
                        }];
                        proto
                    })],
                    push_names: Vec::new(),
                    lids: Vec::new(),
                    stickers: Vec::new(),
                },
                true,
            );
        };
        // The same unfinished position, replayed.
        replay(&mut worker, false);
        assert!(
            marked(&worker),
            "a replay with no newer readable position keeps the notice"
        );
        // The same position, this time as the share's last one.
        replay(&mut worker, true);
        assert!(
            !marked(&worker),
            "a share the phone reports as finished takes the notice back"
        );
    }

    /// A masked event that arrives out of order, older than the position the
    /// card already shows, says nothing about the positions the phone keeps.
    #[test]
    fn an_out_of_order_masked_event_does_not_mark_the_card() {
        const PEER: &str = super::receipt_tests::PEER;
        let (mut worker, _events, _inbox, _wa) = super::receipt_tests::worker();
        let start = crate::util::now() - 600;
        let source = || MessageSource {
            chat: PEER.parse().unwrap(),
            sender: PEER.parse().unwrap(),
            ..Default::default()
        };
        let ingest = |worker: &mut Worker, position: (Arc<wa::Message>, MessageInfo)| {
            worker.ingest(&position.0, &position.1);
        };
        // Ada shares where she is, and this device reads both positions.
        ingest(&mut worker, live_position("start", start, 1, 51.0, None));
        ingest(
            &mut worker,
            live_position("p2", start + 300, 2, 51.1, Some("start")),
        );
        // A masked event from before that position arrives late.
        worker.ingest(
            &masked_live_location(),
            &live_location_info("masked", start + 60, source()),
        );
        let share = worker.archive.message(PEER, "start").unwrap().unwrap();
        assert!(
            matches!(
                share.content,
                Content::LiveLocation {
                    newer_on_phone: false,
                    updated,
                    ..
                } if updated == start + 300
            ),
            "the card keeps the newer readable position: {:?}",
            share.content
        );
    }

    #[test]
    fn other_masked_messages_say_they_are_on_the_phone() {
        const PEER: &str = super::receipt_tests::PEER;
        let (mut worker, _events, _inbox, _wa) = super::receipt_tests::worker();
        let mut info = live_location_info(
            "masked",
            crate::util::now(),
            MessageSource {
                chat: PEER.parse().unwrap(),
                sender: PEER.parse().unwrap(),
                ..Default::default()
            },
        );
        info.media_type = None;
        worker.ingest(&masked_live_location(), &info);
        assert_eq!(
            worker
                .archive
                .message(PEER, "masked")
                .unwrap()
                .unwrap()
                .content,
            Content::PhoneOnly {
                view_once: false,
                live_location: false,
                once: None,
            }
        );
    }

    #[test]
    fn a_live_location_from_our_phone_shows_its_card() {
        let (mut worker, _events, _inbox, _wa) = super::receipt_tests::worker();
        let now = crate::util::now();
        let live = {
            let mut proto = wa::Message::default();
            proto.live_location_message = MessageField::some({
                let mut proto = wa::message::LiveLocationMessage::default();
                proto.degrees_latitude = Some(41.9028);
                proto.degrees_longitude = Some(12.4964);
                proto.accuracy_in_meters = Some(12);
                proto.sequence_number = Some(1);
                proto.jpeg_thumbnail = Some(vec![0xff, 0xd8]);
                proto
            });
            proto
        };
        for (id, source, chat) in live_location_sources(&worker) {
            // Our phone wraps what it sent for its linked devices.
            let message = if source.is_from_me {
                {
                    let mut proto = wa::Message::default();
                    proto.device_sent_message = MessageField::some({
                        let mut proto = wa::message::DeviceSentMessage::default();
                        proto.destination_jid = Some(chat.clone());
                        proto.message = MessageField::some({
                            let mut proto = wa::Message::default();
                            proto.ephemeral_message = MessageField::some({
                                let mut proto = wa::message::FutureProofMessage::default();
                                proto.message = MessageField::some(live.clone());
                                proto
                            });
                            proto
                        });
                        proto
                    });
                    proto
                }
            } else {
                live.clone()
            };
            let mut info = live_location_info(id, now - 60, source);
            info.media_type = None;
            worker.ingest(&Arc::new(message), &info);
            let stored = worker.archive.message(&chat, id).unwrap().unwrap();
            assert!(
                matches!(
                    stored.content,
                    Content::LiveLocation {
                        latitude: 41.9028,
                        ..
                    }
                ),
                "{id}: {:?}",
                stored.content
            );
        }
    }

    #[test]
    fn our_phones_unreadable_messages_leave_a_placeholder() {
        let (mut worker, _events, _inbox, _wa) = super::receipt_tests::worker();
        let now = crate::util::now();
        for (id, source, chat) in live_location_sources(&worker) {
            let from_me = source.is_from_me;
            let mut info = live_location_info(id, now - 60, source);
            info.media_type = None;
            worker.ingest_undecryptable(
                &info,
                wa_events::UnavailableType::Unknown,
                wa_events::DecryptFailMode::Show,
            );
            let stored = worker.archive.message(&chat, id).unwrap().unwrap();
            assert!(
                matches!(stored.content, Content::Unsupported { .. }),
                "{id}"
            );
            assert_eq!(stored.from_me, from_me, "{id}");
        }
        // A copy that names its live location says so, and the real content
        // replaces the placeholder once the phone resends it.
        let me = worker.me();
        let (_, self_chat, _) = live_location_sources(&worker).swap_remove(1);
        worker.ingest_undecryptable(
            &live_location_info("unreadable", now, self_chat.clone()),
            wa_events::UnavailableType::Unknown,
            wa_events::DecryptFailMode::Show,
        );
        assert_eq!(
            worker
                .archive
                .message(&me, "unreadable")
                .unwrap()
                .unwrap()
                .content,
            Content::PhoneOnly {
                view_once: false,
                live_location: true,
                once: None,
            }
        );
        let (message, _) = live_position("unreadable", now, 1, 45.0, None);
        let mut info = live_location_info("unreadable", now, self_chat.clone());
        info.unavailable_request_id = Some("pdo".into());
        worker.ingest(&message, &info);
        assert!(matches!(
            worker
                .archive
                .message(&me, "unreadable")
                .unwrap()
                .unwrap()
                .content,
            Content::LiveLocation { latitude: 45.0, .. }
        ));
        // Internal traffic the sender asked not to show stays hidden.
        worker.ingest_undecryptable(
            &live_location_info("hidden", now, self_chat),
            wa_events::UnavailableType::Unknown,
            wa_events::DecryptFailMode::Hide,
        );
        assert!(worker.archive.message(&me, "hidden").unwrap().is_none());
    }

    #[test]
    fn history_reports_a_finished_live_location_as_ended() {
        let last = {
            let mut proto = wa::message::LiveLocationMessage::default();
            proto.degrees_latitude = Some(48.1);
            proto.degrees_longitude = Some(11.6);
            proto.sequence_number = Some(9);
            proto.time_offset = Some(600);
            proto
        };
        let content = finished_live_location(&last, 1_000);
        assert!(matches!(
            content,
            Content::LiveLocation {
                latitude: 48.1,
                sequence: 9,
                ended: true,
                updated: 1_600,
                ..
            }
        ));
    }

    #[test]
    fn round_video_messages_are_marked_as_notes() {
        let clip = {
            let mut proto = wa::message::VideoMessage::default();
            proto.mimetype = Some("video/mp4".into());
            proto.seconds = Some(12);
            proto
        };
        let note = |content: Option<Content>| match content {
            Some(Content::Video { note, seconds, .. }) => {
                assert_eq!(seconds, Some(12));
                note
            }
            other => panic!("unexpected {other:?}"),
        };
        let round = {
            let mut proto = wa::Message::default();
            proto.ptv_message = MessageField::some(clip.clone());
            proto
        };
        assert!(note(classify(&round)));
        let plain = {
            let mut proto = wa::Message::default();
            proto.video_message = MessageField::some(clip);
            proto
        };
        assert!(!note(classify(&plain)));
    }

    #[test]
    fn unsafe_preview_metadata_cannot_launch_a_desktop_handler() {
        let message = {
            let mut proto = wa::Message::default();
            proto.extended_text_message = MessageField::some({
                let mut proto = wa::message::ExtendedTextMessage::default();
                proto.text = Some("Read this".into());
                proto.matched_text = Some("file:///fixture.exe".into());
                proto.title = Some("An ordinary title".into());
                proto
            });
            proto
        };
        assert!(matches!(
            classify(&message),
            Some(Content::Text { preview: None, .. })
        ));
    }

    #[test]
    fn the_video_half_of_a_motion_photo_is_not_a_message() {
        let child = |association_type| {
            let mut proto = wa::Message::default();
            proto.associated_child_message = MessageField::some({
                let mut proto = wa::message::FutureProofMessage::default();
                proto.message = MessageField::some({
                    let mut proto = wa::Message::default();
                    proto.video_message = MessageField::some({
                        let mut proto = wa::message::VideoMessage::default();
                        proto.mimetype = Some("video/mp4".into());
                        proto
                    });
                    proto
                });
                proto
            });
            proto.message_context_info = MessageField::some({
                let mut proto = wa::MessageContextInfo::default();
                proto.message_association = MessageField::some({
                    let mut proto = wa::MessageAssociation::default();
                    proto.association_type = Some(association_type);
                    proto
                });
                proto
            });
            proto
        };
        use wa::message_association::AssociationType;
        assert_eq!(classify(&child(AssociationType::MOTION_PHOTO)), None);
        // Without the photo's id there is nothing to attach the clip to.
        assert_eq!(motion_clip(&child(AssociationType::MOTION_PHOTO)), None);
        let mut clip = child(AssociationType::MOTION_PHOTO);
        clip.message_context_info
            .as_option_mut()
            .and_then(|context| context.message_association.as_option_mut())
            .unwrap()
            .parent_message_key = MessageField::some({
            let mut proto = wa::MessageKey::default();
            proto.id = Some("photo".into());
            proto
        });
        assert_eq!(
            motion_clip(&clip).map(|(parent, _)| parent).as_deref(),
            Some("photo")
        );
        assert!(matches!(
            classify(&child(AssociationType::MEDIA_ALBUM)),
            Some(Content::Unsupported { .. })
        ));
    }

    /// Someone mentioning the group in their status sends the group a notice
    /// the phone shows no message for, so it is not one here either (#595).
    #[test]
    fn a_group_status_mention_is_not_a_message() {
        let mut proto = wa::Message::default();
        proto.group_status_mention_message = MessageField::some({
            let mut proto = wa::message::FutureProofMessage::default();
            proto.message = MessageField::some(wa::Message::default());
            proto
        });
        assert_eq!(classify(&proto), None);
    }

    #[test]
    fn the_hd_copy_of_a_photo_or_video_is_not_a_message() {
        use wa::context_info::PairedMediaType;
        use wa::message_association::AssociationType;
        let copy = |association_type: Option<AssociationType>, video: bool| {
            let mut proto = wa::Message::default();
            proto.associated_child_message = MessageField::some({
                let mut proto = wa::message::FutureProofMessage::default();
                proto.message = MessageField::some({
                    let mut proto = wa::Message::default();
                    if video {
                        proto.video_message = MessageField::some({
                            let mut proto = wa::message::VideoMessage::default();
                            proto.mimetype = Some("video/mp4".into());
                            proto
                        });
                    } else {
                        proto.image_message = MessageField::some({
                            let mut proto = wa::message::ImageMessage::default();
                            proto.mimetype = Some("image/jpeg".into());
                            proto
                        });
                    }
                    proto
                });
                proto
            });
            proto.message_context_info = MessageField::some({
                let mut proto = wa::MessageContextInfo::default();
                proto.message_association = MessageField::some({
                    let mut proto = wa::MessageAssociation::default();
                    proto.association_type = association_type;
                    proto
                });
                proto
            });
            proto
        };
        assert_eq!(
            classify(&copy(Some(AssociationType::HD_IMAGE_DUAL_UPLOAD), false)),
            None
        );
        assert_eq!(
            classify(&copy(Some(AssociationType::HD_VIDEO_DUAL_UPLOAD), true)),
            None
        );
        assert_eq!(
            classify(&copy(Some(AssociationType::HEVC_VIDEO_DUAL_UPLOAD), true)),
            None
        );
        // A copy that names no association is still known by its media.
        let mut paired = copy(None, false);
        paired
            .associated_child_message
            .as_option_mut()
            .and_then(|wrapper| wrapper.message.as_option_mut())
            .and_then(|child| child.image_message.as_option_mut())
            .unwrap()
            .context_info = MessageField::some({
            let mut context = wa::ContextInfo::default();
            context.paired_media_type = Some(PairedMediaType::HD_IMAGE_CHILD);
            context
        });
        assert_eq!(classify(&paired), None);
        assert!(matches!(
            classify(&copy(None, false)),
            Some(Content::Unsupported { .. })
        ));
    }

    #[tokio::test]
    async fn newsletter_sends_are_rejected_before_reaching_the_client() {
        let (mut worker, events, _, _) = receipt_tests::worker();
        worker
            .handle_command(Command::SendText {
                chat: "fixture@newsletter".into(),
                text: "Fixture".into(),
                quoting: None,
                mentions: Vec::new(),
            })
            .await;
        assert!(matches!(events.try_recv().unwrap(), Event::Error(_)));
    }

    #[tokio::test]
    async fn channel_mutes_from_the_server_mirror_into_the_archive() {
        let (mut worker, _events, _, _) = receipt_tests::worker();
        const MUTED: &str = "1@newsletter";
        const UNMUTED: &str = "2@newsletter";
        worker.archive.ensure_chat(MUTED, "Muted").unwrap();
        worker.archive.ensure_chat(UNMUTED, "Unmuted").unwrap();
        worker.archive.set_muted(UNMUTED, Some(0)).unwrap();
        worker
            .handle_command(Command::ChannelMutes(vec![
                (MUTED.into(), true),
                (UNMUTED.into(), false),
                ("unknown@newsletter".into(), true),
            ]))
            .await;
        let now = crate::util::now();
        assert!(worker.archive.chat(MUTED).unwrap().unwrap().muted(now));
        assert!(!worker.archive.chat(UNMUTED).unwrap().unwrap().muted(now));
        assert!(worker.archive.chat("unknown@newsletter").unwrap().is_none());
    }

    #[tokio::test]
    async fn channel_names_from_the_server_replace_the_bare_id_and_stay() {
        let (mut worker, _events, _, _) = receipt_tests::worker();
        const CHANNEL: &str = "120363000000000001@newsletter";
        worker
            .archive
            .ensure_chat(CHANNEL, &fallback_name(CHANNEL))
            .unwrap();
        worker
            .handle_command(Command::ChannelNames {
                generation: worker.channel_list_generation,
                names: vec![
                    (CHANNEL.into(), "Fixture news".into()),
                    ("unknown@newsletter".into(), "Elsewhere".into()),
                ],
            })
            .await;
        let name = |worker: &Worker| worker.archive.chat(CHANNEL).unwrap().unwrap().name;
        assert_eq!(name(&worker), "Fixture news");
        assert!(worker.archive.chat("unknown@newsletter").unwrap().is_none());
        // A later message or contact update must not put the id back.
        worker.ensure_chat(CHANNEL, None);
        worker.refresh_chat_name(CHANNEL);
        assert_eq!(name(&worker), "Fixture news");
    }

    #[tokio::test]
    async fn empty_channels_we_do_not_follow_are_dropped() {
        let (mut worker, _events, _, _) = receipt_tests::worker();
        const FOLLOWED: &str = "1@newsletter";
        const EMPTY: &str = "2@newsletter";
        const ACTIVE: &str = "3@newsletter";
        const GROUP: &str = "1-2@g.us";
        for id in [FOLLOWED, EMPTY, ACTIVE, GROUP] {
            worker.archive.ensure_chat(id, &fallback_name(id)).unwrap();
        }
        worker
            .archive
            .touch_activity(ACTIVE, 1_700_000_000)
            .unwrap();
        const UNNAMED: &str = "4@newsletter";
        worker
            .archive
            .ensure_chat(UNNAMED, &fallback_name(UNNAMED))
            .unwrap();
        worker
            .handle_command(Command::ChannelNames {
                generation: worker.channel_list_generation,
                names: vec![
                    (FOLLOWED.into(), "Fixture news".into()),
                    (UNNAMED.into(), " ".into()),
                ],
            })
            .await;
        let kept = |id| worker.archive.chat(id).unwrap().is_some();
        assert!(kept(FOLLOWED));
        assert!(kept(UNNAMED), "a followed channel without a name stays");
        assert_eq!(
            worker.archive.chat(UNNAMED).unwrap().unwrap().name,
            fallback_name(UNNAMED),
            "a blank name renames nothing"
        );
        assert!(!kept(EMPTY), "an empty channel we do not follow goes");
        assert!(kept(ACTIVE), "a channel that held messages stays");
        assert!(kept(GROUP), "only channels are affected");
    }

    #[tokio::test]
    async fn an_earlier_connection_channel_list_changes_nothing() {
        let (mut worker, _events, _, _) = receipt_tests::worker();
        const CHANNEL: &str = "1@newsletter";
        const EMPTY: &str = "2@newsletter";
        worker
            .archive
            .ensure_chat(CHANNEL, &fallback_name(CHANNEL))
            .unwrap();
        worker
            .archive
            .ensure_chat(EMPTY, &fallback_name(EMPTY))
            .unwrap();
        let stale = worker.channel_list_generation;
        worker.channel_list_generation = stale.wrapping_add(1);
        worker
            .handle_command(Command::ChannelNames {
                generation: stale,
                names: vec![(CHANNEL.into(), "Old name".into())],
            })
            .await;
        assert_eq!(
            worker.archive.chat(CHANNEL).unwrap().unwrap().name,
            fallback_name(CHANNEL)
        );
        assert!(worker.archive.chat(EMPTY).unwrap().is_some());
    }

    fn unconfirmed(worker: &mut Worker) {
        worker.privacy_ready = false;
        worker.privacy_confirmed = false;
        worker.privacy_reveal_at = Some(Instant::now() + PRIVACY_GRACE);
    }

    #[test]
    fn privacy_recovery_hides_content_until_a_successful_replay() {
        let (mut worker, events, _, _) = receipt_tests::worker();
        const PEER: &str = "fixture@s.whatsapp.net";
        unconfirmed(&mut worker);
        worker.archive.ensure_chat(PEER, "Fixture").unwrap();
        worker.emit_chats();
        assert!(events.try_recv().is_err());
        worker.archive.set_locked_at(PEER, true, 100).unwrap();
        worker.preferences_recovered(0, true, true);
        assert!(worker.privacy_ready);
        assert!(worker.privacy_confirmed);
        assert_eq!(
            worker
                .archive
                .meta("chat_privacy_ready_v1")
                .unwrap()
                .as_deref(),
            Some("complete")
        );
        let chats = events
            .try_iter()
            .find_map(|event| match event {
                Event::Chats(chats) => Some(chats),
                _ => None,
            })
            .unwrap();
        assert!(chats[0].locked);
    }

    /// A call that arrives while the lock state is unknown waits, then rings once it is known.
    ///
    /// Emitting it during recovery would take the window with a chat the interface cannot yet
    /// judge private, naming a hidden caller. It is withheld instead and re-sent by
    /// `reveal_private_content`, so the call still rings rather than being dropped for the whole
    /// recovery.
    #[test]
    fn a_call_withheld_during_privacy_recovery_rings_once_the_locks_are_known() {
        let (mut worker, events, _, _) = receipt_tests::worker();
        const PEER: &str = "fixture@s.whatsapp.net";
        worker.archive.ensure_chat(PEER, "Fixture").unwrap();
        let call = crate::calls::Call::test_snapshot(PEER);
        let update = call.update();
        worker.call = Some(calls::CallRuntime::new(call, None));
        unconfirmed(&mut worker);
        // The update is withheld: the chat's lock state is not known yet.
        worker.emit(Event::Call(Box::new(update)));
        assert!(events.try_recv().is_err(), "the call waits for the locks");
        // Recovery completes and re-sends it, so a call that arrived mid-recovery still rings.
        worker.preferences_recovered(0, true, true);
        assert!(worker.privacy_ready);
        assert!(
            events
                .try_iter()
                .any(|event| matches!(event, Event::Call(_))),
            "the withheld call is re-sent once the locks are known"
        );
    }

    /// An album carries private thumbnails and paths, so it waits for the
    /// same authorized replay the rest of the archive content waits for.
    #[test]
    fn an_album_is_withheld_until_lock_state_is_confirmed() {
        let (mut worker, events, _, _) = receipt_tests::worker();
        const PEER: &str = "fixture@s.whatsapp.net";
        worker.archive.ensure_chat(PEER, "Fixture").unwrap();
        let row = Message {
            chat: PEER.into(),
            ..receipt_tests::own_message("photo", 100)
        };
        worker.archive.insert_message(&row, None).unwrap();
        unconfirmed(&mut worker);
        worker.emit_chat_media(PEER.into(), None);
        assert!(
            !events
                .try_iter()
                .any(|event| matches!(event, Event::ChatMedia { .. })),
            "no album is sent while lock state is unknown"
        );
        worker.preferences_recovered(0, false, false);
        worker.emit_chat_media(PEER.into(), None);
        assert!(
            events
                .try_iter()
                .any(|event| matches!(event, Event::ChatMedia { .. })),
            "the album arrives once lock state is confirmed"
        );
    }
    /// A chat opened while lock state was still being recovered asked for its
    /// messages once; the answer was withheld, and the interface never asked
    /// again, so the chat stayed empty until a new message came in (#180).
    #[test]
    fn transcript_reads_withheld_during_privacy_recovery_are_answered_once_shown() {
        let (mut worker, events, _, _) = receipt_tests::worker();
        const GROUP: &str = "120363000000000001@g.us";
        worker.archive.ensure_chat(GROUP, "Fixture group").unwrap();
        for (id, timestamp) in [("first", 100), ("second", 200), ("third", 300)] {
            let row = Message {
                chat: GROUP.into(),
                ..receipt_tests::own_message(id, timestamp)
            };
            worker.archive.insert_message(&row, None).unwrap();
        }
        unconfirmed(&mut worker);
        worker.load_chat(GROUP.into(), None);
        worker.load_chat(GROUP.into(), Some((300, "third".into())));
        worker.load_until(GROUP.into(), "first".into(), (200, "second".into()));
        // Asking twice keeps one read.
        worker.load_chat(GROUP.into(), None);
        assert!(
            !events
                .try_iter()
                .any(|event| matches!(event, Event::Messages { .. })),
            "nothing private is sent while lock state is unknown"
        );
        worker.preferences_recovered(0, false, false);
        let pages: Vec<(Vec<String>, bool)> = events
            .try_iter()
            .filter_map(|event| match event {
                Event::Messages {
                    chat,
                    messages,
                    older,
                    ..
                } if chat == GROUP => Some((
                    messages.into_iter().map(|message| message.id).collect(),
                    older,
                )),
                _ => None,
            })
            .collect();
        assert_eq!(
            pages,
            vec![
                (vec!["first".into(), "second".into(), "third".into()], false),
                (vec!["first".into(), "second".into()], true),
                (vec!["first".into()], true),
            ]
        );
        assert!(worker.withheld_pages.is_empty());
    }

    /// The info panel's listing is private content too: asked for while
    /// lock state is unknown, it is answered once content is shown, so the
    /// panel does not wait forever (as the transcript did in #180).
    #[test]
    fn media_listings_withheld_during_privacy_recovery_are_answered_once_shown() {
        let (mut worker, events, _, _) = receipt_tests::worker();
        const PEER: &str = "fixture@s.whatsapp.net";
        worker.archive.ensure_chat(PEER, "Fixture").unwrap();
        unconfirmed(&mut worker);
        worker.load_chat_media(PEER.into(), 7);
        worker.load_chat_media(PEER.into(), 7);
        assert!(
            !events
                .try_iter()
                .any(|event| matches!(event, Event::ChatShared { .. })),
            "nothing private is sent while lock state is unknown"
        );
        worker.preferences_recovered(0, false, false);
        let answered: Vec<u64> = events
            .try_iter()
            .filter_map(|event| match event {
                Event::ChatShared { request, result } => {
                    assert_eq!(result.expect("listed").chat, PEER);
                    Some(request)
                }
                _ => None,
            })
            .collect();
        assert_eq!(answered, [7], "asked twice, answered once");
        assert!(worker.withheld_pages.is_empty());
    }

    /// History toward a replied message that cannot be read answers with
    /// one event that carries the error: the chat stops waiting for older
    /// history, and the jump that asked hears that error and nothing else,
    /// however the window's frames split the events.
    #[test]
    fn a_failed_read_toward_a_message_answers_with_its_error_in_one_event() {
        let (mut worker, events, _, _) = receipt_tests::worker();
        const GROUP: &str = "120363000000000001@g.us";
        worker.archive.ensure_chat(GROUP, "Fixture group").unwrap();
        for (id, timestamp) in [("first", 100), ("second", 200), ("third", 300)] {
            let row = Message {
                chat: GROUP.into(),
                ..receipt_tests::own_message(id, timestamp)
            };
            worker.archive.insert_message(&row, None).unwrap();
        }
        // The target reads; a message between it and the page does not.
        worker
            .archive
            .execute_for_tests("UPDATE messages SET content = x'00' WHERE id = 'second'");
        let _: Vec<_> = events.try_iter().collect();
        for (id, unread) in [
            ("first", "Could not read the chat"),
            ("gone", "This message is not stored on this computer"),
        ] {
            worker.load_until(GROUP.into(), id.into(), (300, "third".into()));
            let answer: Vec<String> = events
                .try_iter()
                .filter_map(|event| match event {
                    Event::Messages { chat, .. } if chat == GROUP => Some("page".to_owned()),
                    Event::HistoryUnavailable { chat, error } if chat == GROUP => Some(error),
                    Event::Error(error) => Some(format!("separate error: {error}")),
                    _ => None,
                })
                .collect();
            assert_eq!(answer.len(), 1, "{id}: {answer:?}");
            assert!(answer[0].starts_with(unread), "{id}: {answer:?}");
        }
    }

    #[test]
    fn failed_privacy_recovery_shows_known_state_and_keeps_retrying() {
        let (mut worker, events, _, _) = receipt_tests::worker();
        const PEER: &str = "fixture@s.whatsapp.net";
        unconfirmed(&mut worker);
        worker.archive.ensure_chat(PEER, "Fixture").unwrap();
        worker.archive.set_locked_at(PEER, true, 100).unwrap();
        worker.preferences_recovered(0, false, false);
        assert!(worker.privacy_ready);
        assert!(!worker.privacy_confirmed);
        assert!(worker.privacy_retry > Instant::now());
        assert!(
            worker
                .archive
                .meta("chat_privacy_ready_v1")
                .unwrap()
                .is_none()
        );
        let events: Vec<_> = events.try_iter().collect();
        let chats = events
            .iter()
            .find_map(|event| match event {
                Event::Chats(chats) => Some(chats),
                _ => None,
            })
            .unwrap();
        assert!(chats[0].locked);
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, Event::Info(_)))
                .count(),
            1
        );
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, Event::Syncing(true)))
        );
        // A second failure warns no further.
        worker.preferences_recovered(0, false, false);
        assert_eq!(worker.privacy_attempts, 2);
    }

    #[test]
    fn partial_settings_recovery_confirms_locks_but_retries_next_start() {
        let (mut worker, _events, _, _) = receipt_tests::worker();
        unconfirmed(&mut worker);
        worker.preferences_recovered(0, true, false);
        assert!(worker.privacy_ready);
        assert!(worker.privacy_confirmed);
        assert!(
            worker
                .archive
                .meta("chat_privacy_ready_v1")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn unconfirmed_privacy_shows_content_after_the_grace_period() {
        let (mut worker, events, _, _) = receipt_tests::worker();
        unconfirmed(&mut worker);
        worker.reveal_unconfirmed_after_grace();
        assert!(!worker.privacy_ready);
        assert!(events.try_recv().is_err());
        worker.privacy_reveal_at = Some(Instant::now());
        worker.reveal_unconfirmed_after_grace();
        assert!(worker.privacy_ready);
        assert!(worker.privacy_reveal_at.is_none());
        assert!(
            events
                .try_iter()
                .any(|event| matches!(event, Event::Info(_)))
        );
    }

    #[tokio::test]
    async fn failed_presence_send_retries_when_the_window_state_repeats() {
        let (mut worker, _events, _, mut wa_events) = receipt_tests::worker();
        let directory = tempfile::tempdir().unwrap();
        let store = whatsapp_rust::store::SqliteStore::open(
            directory.path().join("session.db").to_str().unwrap(),
        )
        .await
        .unwrap();
        let bot = host_owned_presence(Bot::builder().with_backend(store))
            .build()
            .await
            .unwrap();
        let client = bot.client();
        client
            .persistence_manager()
            .process_command(whatsapp_rust::store::commands::DeviceCommand::SetPushName(
                "ZapFast".to_owned(),
            ))
            .await;
        worker.client = Some(client);
        worker.online_wanted = true;
        worker.online_sent = Some(true);
        worker.push_name_synced();
        assert!(
            worker.presence_in_flight,
            "push-name sync reannounces even an already confirmed state"
        );

        let RuntimeEvent::PresenceSent {
            generation,
            online,
            succeeded,
        } = tokio::time::timeout(Duration::from_secs(1), wa_events.recv())
            .await
            .unwrap()
            .unwrap()
        else {
            panic!("presence attempt should report its result");
        };
        assert!(online);
        assert!(!succeeded, "an unconnected client cannot send presence");
        worker.presence_sent(generation, online, succeeded);
        assert_eq!(worker.online_sent, None, "failed sends are not recorded");

        worker.set_online(true);
        assert!(
            worker.presence_in_flight,
            "repeating the wanted state retries an unsuccessful announcement"
        );

        worker.push_name_synced();
        assert!(worker.presence_refresh_pending);
        let RuntimeEvent::PresenceSent {
            generation,
            online,
            succeeded,
        } = tokio::time::timeout(Duration::from_secs(1), wa_events.recv())
            .await
            .unwrap()
            .unwrap()
        else {
            panic!("presence retry should report its result");
        };
        worker.presence_sent(generation, online, succeeded);
        assert!(
            worker.presence_in_flight,
            "push-name sync reannounces after an overlapping presence request"
        );
        assert!(!worker.presence_refresh_pending);
    }

    #[test]
    fn leaving_the_window_waits_before_going_unavailable() {
        let (mut worker, _events, _, _) = receipt_tests::worker();
        worker.set_online(true);
        worker.set_online(false);
        assert!(!worker.unavailable_due());
        worker.online_changed = Instant::now() - PRESENCE_LINGER;
        worker.set_online(false);
        assert!(
            worker.unavailable_due(),
            "repeating the same state does not restart the wait"
        );
        worker.online_sent = Some(false);
        assert!(!worker.unavailable_due(), "announced once per connection");
        worker.set_online(true);
        assert!(!worker.unavailable_due());
    }

    #[test]
    fn an_account_without_about_text_loads_none() {
        let (mut worker, _events, _, _) = receipt_tests::worker();
        worker.archive.set_meta("me_about", "").unwrap();
        worker.load_state();
        assert_eq!(worker.me_about, None);
        worker.archive.set_meta("me_about", "Busy").unwrap();
        worker.load_state();
        assert_eq!(worker.me_about.as_deref(), Some("Busy"));
    }

    #[tokio::test]
    async fn a_saved_profile_updates_our_name_and_about() {
        let (mut worker, events, _, _) = receipt_tests::worker();
        worker
            .handle_command(Command::ProfileSaved {
                name: Some("Carmine".into()),
                about: Some("Busy".into()),
                picture: false,
            })
            .await;
        assert_eq!(worker.me_name.as_deref(), Some("Carmine"));
        assert_eq!(worker.me_about.as_deref(), Some("Busy"));
        assert!(events.try_iter().any(|event| matches!(
            event,
            Event::Me { name: Some(name), about: Some(about), .. }
                if name == "Carmine" && about == "Busy"
        )));
        // Clearing the About keeps the name.
        worker
            .handle_command(Command::ProfileSaved {
                name: None,
                about: Some(String::new()),
                picture: false,
            })
            .await;
        assert_eq!(worker.me_name.as_deref(), Some("Carmine"));
        assert_eq!(worker.me_about, None);
        assert_eq!(
            worker.archive.meta("me_about").unwrap().as_deref(),
            Some("")
        );
    }

    #[tokio::test]
    async fn downloads_use_the_chosen_folder_and_fall_back_to_the_cache() {
        let (mut worker, _events, _, _) = receipt_tests::worker();
        let root = std::env::temp_dir().join(format!("zapfast-downloads-{}", std::process::id()));
        let chosen = root.join("Downloads/WhatsApp");
        worker
            .handle_command(Command::SetDownloadFolder(Some(chosen.clone())))
            .await;
        let chat = "200@s.whatsapp.net";
        worker.archive.ensure_chat(chat, "Alice").unwrap();
        let expected =
            download_folders::account_folder(&chosen, &worker.dirs.id, worker.me_pn.as_deref())
                .join("Alice");
        assert_eq!(worker.download_dir(chat), expected);
        assert!(
            expected.is_dir(),
            "account and chat folders are created when needed"
        );
        assert!(chosen.is_dir(), "the folder is created when needed");
        // A folder that cannot exist, such as one below a file, falls back.
        std::fs::write(root.join("file"), b"").unwrap();
        worker.download_folder = Some(root.join("file/inside"));
        assert_eq!(worker.download_dir(chat), worker.dirs.media_cache_dir());
        worker.download_folder = None;
        assert_eq!(worker.download_dir(chat), worker.dirs.media_cache_dir());
        let _ = std::fs::remove_dir_all(root);
    }

    /// Canonical phone and privacy ids name the same destination, including groups.
    #[test]
    fn download_folders_use_canonical_chat_names() {
        let (mut worker, _events, _, _) = receipt_tests::worker();
        let root = tempfile::tempdir().unwrap();
        worker.download_folder = Some(root.path().to_owned());
        let chat = "200@s.whatsapp.net";
        worker.archive.ensure_chat(chat, "Alice").unwrap();
        worker.learn_lid("9", "200");
        let path = worker.download_dir(chat);
        assert_eq!(path, worker.download_dir("9@lid"));
        assert_eq!(path.file_name().unwrap(), "Alice");

        let group = "100-200@g.us";
        worker.archive.ensure_chat(group, "Family").unwrap();
        assert_eq!(worker.download_dir(group).file_name().unwrap(), "Family");

        // Another chat with the same name gets its own folder beside it.
        let namesake = "300@s.whatsapp.net";
        worker.archive.ensure_chat(namesake, "Alice").unwrap();
        assert_eq!(
            worker.download_dir(namesake).file_name().unwrap(),
            "Alice (2)"
        );
        assert_eq!(worker.download_dir(chat), path);

        // An inaccessible nested chat folder also uses the ordinary cache.
        std::fs::remove_dir(&path).unwrap();
        std::fs::write(&path, b"blocked").unwrap();
        assert_eq!(worker.download_dir(chat), worker.dirs.media_cache_dir());
    }

    /// The account's folder is its phone number, whichever form of our own
    /// identity WhatsApp reports, and the privacy id alone never moves it.
    #[test]
    fn the_account_download_folder_stays_put_across_identity_forms() {
        let (mut worker, _events, _, _) = receipt_tests::worker();
        let root = tempfile::tempdir().unwrap();
        worker.download_folder = Some(root.path().to_owned());
        let chat = "200@s.whatsapp.net";
        worker.archive.ensure_chat(chat, "Alice").unwrap();
        worker.me_pn = Some("15550001111@s.whatsapp.net".into());
        worker.me_lid = None;
        let first = worker.download_dir(chat);
        assert_eq!(first, root.path().join("+15550001111").join("Alice"));
        worker.me_lid = Some("1001@lid".into());
        assert_eq!(worker.download_dir(chat), first);
        worker.me_pn = Some("15550001111:3@s.whatsapp.net".into());
        assert_eq!(worker.download_dir(chat), first);
    }

    /// Publication stays beside the destination, preserving names without a cache export.
    #[tokio::test]
    async fn organized_downloads_publish_once_without_overwriting() {
        let (mut worker, _events, _, _) = receipt_tests::worker();
        let root = tempfile::tempdir().unwrap();
        worker.download_folder = Some(root.path().to_owned());
        let chat = "200@s.whatsapp.net";
        worker.archive.ensure_chat(chat, "Alice").unwrap();
        let dir = worker.download_dir(chat);
        let path = named_media_path(&dir, "application/pdf", "report.pdf").unwrap();
        std::fs::write(&path, b"existing").unwrap();
        let (temporary, file) = temporary_attachment_file(&path).unwrap();
        drop(file);
        std::fs::write(&temporary, b"download").unwrap();
        let published = publish_beside(&temporary, &path).await.unwrap();
        assert_eq!(published, dir.join("report (1).pdf"));
        assert_eq!(std::fs::read(&path).unwrap(), b"existing");
        assert_eq!(std::fs::read(&published).unwrap(), b"download");
        assert!(!temporary.exists());
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2);
        assert!(!worker.dirs.media_cache_dir().join("report.pdf").exists());
        assert!(
            !worker
                .dirs
                .media_cache_dir()
                .join("report (1).pdf")
                .exists()
        );
    }

    /// Existing account/chat links must fall back without creating anything outside the root.
    #[cfg(unix)]
    #[test]
    fn linked_download_directories_use_the_cache() {
        let (mut worker, _events, _, _) = receipt_tests::worker();
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        worker.download_folder = Some(root.path().to_owned());
        let chat = "200@s.whatsapp.net";
        worker.archive.ensure_chat(chat, "Alice").unwrap();
        let folder =
            download_folders::account_folder(root.path(), &worker.dirs.id, worker.me_pn.as_deref())
                .join("Alice");
        let account = folder.parent().unwrap();
        std::os::unix::fs::symlink(outside.path(), account).unwrap();
        assert_eq!(worker.download_dir(chat), worker.dirs.media_cache_dir());
        assert_eq!(std::fs::read_dir(outside.path()).unwrap().count(), 0);
        std::fs::remove_file(account).unwrap();
        std::fs::create_dir(account).unwrap();
        std::os::unix::fs::symlink(outside.path(), &folder).unwrap();
        assert_eq!(worker.download_dir(chat), worker.dirs.media_cache_dir());
        assert_eq!(std::fs::read_dir(outside.path()).unwrap().count(), 0);
    }

    /// Selecting an already-used root must not sweep a download started in this session.
    #[tokio::test]
    async fn chosen_folder_cleanup_runs_once_per_account_and_root() {
        let (mut worker, _events, _, _) = receipt_tests::worker();
        let root = tempfile::tempdir().unwrap();
        let dir =
            download_folders::account_folder(root.path(), &worker.dirs.id, worker.me_pn.as_deref())
                .join("Alice");
        std::fs::create_dir_all(&dir).unwrap();
        let partial = dir.join(".photo.0123456789abcdef.part");
        std::fs::write(&partial, b"old").unwrap();
        worker
            .handle_command(Command::SetDownloadFolder(Some(root.path().to_owned())))
            .await;
        assert!(
            !partial.exists(),
            "a restarted worker clears crash leftovers"
        );
        std::fs::write(&partial, b"active").unwrap();
        worker
            .handle_command(Command::SetDownloadFolder(None))
            .await;
        worker
            .handle_command(Command::SetDownloadFolder(Some(root.path().to_owned())))
            .await;
        assert_eq!(std::fs::read(&partial).unwrap(), b"active");
    }

    /// A local staging failure switches to a writable cache before any network transfer.
    #[tokio::test]
    async fn unavailable_staging_uses_the_cache() {
        let root = tempfile::tempdir().unwrap();
        let blocked = root.path().join("blocked");
        std::fs::write(&blocked, b"not a directory").unwrap();
        let path = blocked.join("report.pdf");
        let cache = root.path().join("cache/message.pdf");
        assert!(stage_attachment(&path, None).await.is_err());
        let (actual, temporary, file) = stage_attachment(&path, Some(&cache)).await.unwrap();
        let staged = Staged(temporary.clone());
        assert_eq!(actual, cache);
        assert_eq!(temporary.parent(), cache.parent());
        drop(file);
        drop(staged);
        assert!(!temporary.exists());
        std::fs::write(root.path().join("cache-blocked"), b"blocked").unwrap();
        assert!(
            stage_attachment(&path, Some(&root.path().join("cache-blocked/file")))
                .await
                .is_err()
        );
    }

    /// A late publication failure preserves the verified bytes in one finished cache file.
    #[tokio::test]
    async fn failed_publication_uses_verified_bytes_in_the_cache() {
        let root = tempfile::tempdir().unwrap();
        let temporary = root.path().join(".verified.0123456789abcdef.part");
        let staged = Staged(temporary.clone());
        std::fs::write(&temporary, b"verified fixture").unwrap();
        let blocked = root.path().join("blocked");
        std::fs::write(&blocked, b"keep").unwrap();
        let path = blocked.join("report.pdf");
        let cache = root.path().join("cache/message.pdf");
        assert!(
            publish_download(&temporary, &path, true, None)
                .await
                .is_err()
        );
        let saved = publish_download(&temporary, &path, true, Some(&cache))
            .await
            .unwrap();
        assert_eq!(saved, cache);
        assert_eq!(std::fs::read(&saved).unwrap(), b"verified fixture");
        assert_eq!(std::fs::read(&blocked).unwrap(), b"keep");
        drop(staged);
        assert!(!temporary.exists());
        assert_eq!(
            std::fs::read_dir(cache.parent().unwrap()).unwrap().count(),
            1
        );
    }

    #[test]
    fn view_once_placeholders_say_they_open_only_on_the_phone() {
        let (mut worker, _events, _, _) = receipt_tests::worker();
        for (id, unavailable, expected) in [
            (
                "once",
                wa_events::UnavailableType::ViewOnce,
                Some(Content::PhoneOnly {
                    view_once: true,
                    live_location: false,
                    once: None,
                }),
            ),
            (
                "bot",
                wa_events::UnavailableType::Bot,
                Some(Content::PhoneOnly {
                    view_once: false,
                    live_location: false,
                    once: None,
                }),
            ),
            ("later", wa_events::UnavailableType::Unknown, None),
        ] {
            let info = MessageInfo {
                id: id.into(),
                source: MessageSource {
                    chat: "200@s.whatsapp.net".parse().unwrap(),
                    sender: "200@s.whatsapp.net".parse().unwrap(),
                    ..Default::default()
                },
                timestamp: whatsapp_rust::wacore::time::from_secs(100).unwrap(),
                ..Default::default()
            };
            worker.ingest_undecryptable(&info, unavailable, wa_events::DecryptFailMode::Show);
            let stored = worker
                .archive
                .message("200@s.whatsapp.net", id)
                .unwrap()
                .unwrap()
                .content;
            match expected {
                Some(content) => assert_eq!(stored, content),
                None => assert!(matches!(stored, Content::Unsupported { .. })),
            }
        }
    }

    /// View-once media that arrives whole, in any of its wrappers or with
    /// only the inline flag, is filed as the view-once placeholder for its
    /// kind, never as an attachment to download. The same wrapper around an
    /// interactive message, or around plain media, changes nothing else.
    #[test]
    fn view_once_media_becomes_a_placeholder_for_its_kind() {
        use crate::model::OnceMedia;
        let photo = || {
            let mut proto = wa::message::ImageMessage::default();
            proto.mimetype = Some("image/jpeg".into());
            proto
        };
        let wrap = |inner: wa::Message| {
            let mut proto = wa::message::FutureProofMessage::default();
            proto.message = MessageField::some(inner);
            proto
        };
        let image = |view_once| {
            let mut proto = wa::Message::default();
            proto.image_message = MessageField::some({
                let mut proto = photo();
                proto.view_once = view_once;
                proto
            });
            proto
        };
        let placeholder = |kind| {
            Some(Content::PhoneOnly {
                view_once: true,
                live_location: false,
                once: Some(kind),
            })
        };
        let wrapped = [
            {
                let mut proto = wa::Message::default();
                proto.view_once_message = MessageField::some(wrap(image(None)));
                proto
            },
            {
                let mut proto = wa::Message::default();
                proto.view_once_message_v2 = MessageField::some(wrap(image(Some(true))));
                proto
            },
            {
                let mut proto = wa::Message::default();
                proto.view_once_message_v2_extension = MessageField::some(wrap(image(None)));
                proto
            },
            image(Some(true)),
        ];
        for message in &wrapped {
            assert_eq!(classify(message), placeholder(OnceMedia::Photo));
        }
        let video = {
            let mut proto = wa::Message::default();
            proto.view_once_message_v2 = MessageField::some(wrap({
                let mut proto = wa::Message::default();
                proto.video_message = MessageField::some({
                    let mut proto = wa::message::VideoMessage::default();
                    proto.view_once = Some(true);
                    proto
                });
                proto
            }));
            proto
        };
        assert_eq!(classify(&video), placeholder(OnceMedia::Video));
        let audio = |ptt| {
            let mut proto = wa::Message::default();
            proto.view_once_message_v2 = MessageField::some(wrap({
                let mut proto = wa::Message::default();
                proto.audio_message = MessageField::some({
                    let mut proto = wa::message::AudioMessage::default();
                    proto.ptt = Some(ptt);
                    proto.view_once = Some(true);
                    proto
                });
                proto
            }));
            proto
        };
        assert_eq!(classify(&audio(true)), placeholder(OnceMedia::Voice));
        assert_eq!(classify(&audio(false)), placeholder(OnceMedia::Audio));

        assert!(matches!(
            classify(&image(None)),
            Some(Content::Image { .. })
        ));
        assert!(matches!(
            classify(&image(Some(false))),
            Some(Content::Image { .. })
        ));
        let buttons = {
            let mut proto = wa::Message::default();
            proto.view_once_message = MessageField::some(wrap({
                let mut proto = wa::Message::default();
                proto.interactive_message = MessageField::some({
                    let mut proto = wa::message::InteractiveMessage::default();
                    proto.body = MessageField::some({
                        let mut proto = wa::message::interactive_message::Body::default();
                        proto.text = Some("Pick one".into());
                        proto
                    });
                    proto
                });
                proto
            }));
            proto
        };
        assert!(
            !matches!(classify(&buttons), Some(Content::PhoneOnly { .. })),
            "interactive messages share the wrapper and stay interactive"
        );
    }

    #[tokio::test]
    async fn outgoing_view_once_photos_preserve_wire_metadata_without_caching_the_photo() {
        let directory = tempfile::tempdir().unwrap();
        let store = whatsapp_rust::store::SqliteStore::open(
            directory.path().join("session.db").to_str().unwrap(),
        )
        .await
        .unwrap();
        let bot = Bot::builder().with_backend(store).build().await.unwrap();
        let client = bot.client();
        let (mut worker, _events, _commands, _wa) = super::receipt_tests::worker();
        worker.client = Some(client.clone());
        let quote = Quoted {
            chat: None,
            id: "original".into(),
            sender: "15551234567@s.whatsapp.net".into(),
            sender_name: Some("Fixture".into()),
            summary: "Original".into(),
            labelled: Some(false),
            mentions: vec![],
        };
        let mut context = wa::ContextInfo::default();
        context.stanza_id = Some(quote.id.clone());
        context.participant = Some(quote.sender.clone());
        context.quoted_message = MessageField::some(wa::Message::text("Original"));
        for view_once in [false, true] {
            let cache = directory
                .path()
                .join(if view_once { "once" } else { "ordinary" });
            let mut prepared = Prepared {
                message: {
                    let mut message = wa::Message::default();
                    message.image_message = MessageField::some({
                        let mut image = wa::message::ImageMessage::default();
                        image.jpeg_thumbnail = Some(vec![1, 2, 3]);
                        image
                    });
                    message
                },
                content: Content::Image {
                    motion: None,
                    caption: None,
                    media: media(Some(&"image/jpeg".to_owned()), Some(4), Some(1), Some(1)),
                },
                thumbnail: Some(vec![1, 2, 3]),
                bytes: vec![1, 2, 3, 4],
                mime: "image/jpeg".into(),
                file_name: None,
            };
            if view_once {
                mark_view_once_photo(&mut prepared).unwrap();
            }
            let (row, raw) = quoted_outbound(
                &client,
                "15551234567@s.whatsapp.net",
                "15550001111@s.whatsapp.net",
                &cache,
                prepared,
                Some("Caption".into()),
                vec!["15559876543@s.whatsapp.net".into()],
                Some((context.clone(), quote.clone())),
            )
            .await
            .unwrap();
            assert_eq!(row.quoted, Some(quote.clone()));
            if view_once {
                assert_eq!(
                    row.content,
                    Content::PhoneOnly {
                        view_once: true,
                        live_location: false,
                        once: Some(crate::model::OnceMedia::Photo),
                    }
                );
                assert!(row.thumbnail.is_none());
                assert!(
                    !cache.exists(),
                    "view-once bytes must not be cached locally"
                );
            } else {
                let path = row.content.media().unwrap().path.as_ref().unwrap();
                assert_eq!(std::fs::read(path).unwrap(), [1, 2, 3, 4]);
                assert_eq!(row.thumbnail, Some(vec![1, 2, 3]));
            }
            let raw = wa::Message::decode_from_slice(&raw).unwrap();
            assert_eq!(raw.is_view_once(), view_once);
            let image = raw.image_message.as_option().unwrap();
            assert_eq!(image.view_once, view_once.then_some(true));
            assert_eq!(image.caption.as_deref(), Some("Caption"));
            let context = image.context_info.as_option().unwrap();
            assert_eq!(
                context.mentioned_jid,
                ["15559876543@s.whatsapp.net".to_owned()]
            );
            assert_eq!(context.stanza_id.as_deref(), Some("original"));
            assert_eq!(
                context.quoted_message.as_option().unwrap().text_content(),
                Some("Original")
            );
            let id = row.id.clone();
            worker.outbound(row.chat.clone(), row, raw.encode_to_vec());
            let archived = wa::Message::decode_from_slice(
                &worker
                    .archive
                    .raw("15551234567@s.whatsapp.net", &id)
                    .unwrap()
                    .unwrap(),
            )
            .unwrap();
            let mut expected = raw.clone();
            if view_once {
                expected
                    .image_message
                    .as_option_mut()
                    .unwrap()
                    .jpeg_thumbnail = None;
            }
            assert_eq!(archived, expected);
            assert_eq!(
                raw.image_message.as_option().unwrap().jpeg_thumbnail,
                Some(vec![1, 2, 3]),
                "the message sent to WhatsApp must keep its thumbnail"
            );
        }
    }

    #[test]
    fn starting_over_keeps_the_old_archive_and_forgets_the_link() {
        let root = std::env::temp_dir().join(format!("zapfast-start-over-{}", std::process::id()));
        let dirs = crate::paths::AppDirs::under(&root);
        dirs.ensure().unwrap();
        std::fs::write(dirs.archive_db(), b"encrypted").unwrap();
        let mut wal = dirs.archive_db().into_os_string();
        wal.push("-wal");
        std::fs::write(&wal, b"log").unwrap();
        std::fs::write(dirs.session_db(), b"keys").unwrap();
        let kept = set_aside_unreadable_archive(&dirs.as_account()).unwrap();
        assert_eq!(std::fs::read(&kept).unwrap(), b"encrypted");
        let mut kept_wal = kept.clone().into_os_string();
        kept_wal.push("-wal");
        assert_eq!(std::fs::read(kept_wal).unwrap(), b"log");
        assert!(!dirs.archive_db().exists());
        assert!(!dirs.session_db().exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn privacy_recovery_backs_off() {
        assert_eq!(privacy_backoff(1), Duration::from_secs(30));
        assert_eq!(privacy_backoff(2), Duration::from_secs(60));
        assert_eq!(privacy_backoff(4), Duration::from_secs(240));
        assert_eq!(privacy_backoff(40), Duration::from_secs(15 * 60));
    }

    #[test]
    fn stale_privacy_recovery_cannot_expose_a_different_linked_account() {
        let (mut worker, events, _, _) = receipt_tests::worker();
        unconfirmed(&mut worker);
        worker.privacy_recovering = true;
        worker.privacy_generation = 1;
        worker.preferences_recovered(0, true, true);
        assert!(!worker.privacy_ready);
        assert!(worker.privacy_recovering);
        assert!(events.try_recv().is_err());
        assert!(
            worker
                .archive
                .meta("chat_privacy_ready_v1")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn link_previews_and_mentions_come_from_extended_text() {
        let message = {
            let mut proto = wa::Message::default();
            proto.extended_text_message = whatsapp_rust::prelude::MessageField::some({
                let mut proto = wa::message::ExtendedTextMessage::default();
                proto.text = Some("see spotifast.rocks @123456@lid".into());
                proto.matched_text = Some("https://spotifast.rocks/".into());
                proto.title = Some("spotifast.rocks".into());
                proto.description = Some("Spotify, native and fast".into());
                proto.context_info = whatsapp_rust::prelude::MessageField::some({
                    let mut proto = wa::ContextInfo::default();
                    proto.mentioned_jid = vec!["123456@lid".into()];
                    proto
                });
                proto
            });
            proto
        };
        match classify(&message) {
            Some(Content::Text { preview, .. }) => {
                let preview = preview.expect("preview");
                assert_eq!(preview.url, "https://spotifast.rocks/");
                assert_eq!(preview.title.as_deref(), Some("spotifast.rocks"));
            }
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(mentioned_of(&message), vec!["123456@lid".to_owned()]);
    }

    #[test]
    fn outgoing_mentions_share_context_with_a_quote() {
        let mentions = vec!["491702222222@s.whatsapp.net".to_owned()];
        let message = outgoing_text(
            "hello @491702222222".to_owned(),
            Some({
                let mut proto = wa::ContextInfo::default();
                proto.stanza_id = Some("quoted".to_owned());
                proto
            }),
            &mentions,
        );

        assert_eq!(message.text_content(), Some("hello @491702222222"));
        let context = context_of(&message).expect("text context");
        assert_eq!(context.stanza_id.as_deref(), Some("quoted"));
        assert_eq!(context.mentioned_jid, mentions);
    }

    #[test]
    fn a_spot_goes_out_as_a_location_and_comes_back_as_one() {
        let message = outgoing_location(-23.5505, -46.6333, None);
        let location = message.location_message.as_option().expect("a location");
        assert_eq!(location.degrees_latitude, Some(-23.5505));
        assert_eq!(location.degrees_longitude, Some(-46.6333));
        assert_eq!(location.is_live, None, "a spot is not a share that moves");
        assert_eq!(location.name, None);
        assert_eq!(location.address, None);

        // The reader on the other side reads the fields we wrote, so the spot
        // arrives as the spot.
        assert_eq!(
            classify(&message),
            Some(Content::Location {
                latitude: -23.5505,
                longitude: -46.6333,
                name: None,
                address: None,
            })
        );
    }

    #[test]
    fn a_quoted_location_carries_the_quote_inside_it() {
        let message = outgoing_location(
            -23.5505,
            -46.6333,
            Some({
                let mut context = wa::ContextInfo::default();
                context.stanza_id = Some("quoted".to_owned());
                context
            }),
        );
        let context = context_of(&message).expect("location context");
        assert_eq!(context.stanza_id.as_deref(), Some("quoted"));
    }

    #[tokio::test]
    async fn a_spot_sent_while_offline_goes_back_to_the_dialog() {
        let (mut worker, events, _, _) = receipt_tests::worker();
        worker
            .handle_command(Command::SendLocation {
                chat: receipt_tests::PEER.into(),
                latitude: -23.5505,
                longitude: -46.6333,
                quoting: None,
            })
            .await;
        if let Event::SendRefused {
            unsent:
                Unsent::Location {
                    latitude,
                    longitude,
                },
            reason,
            ..
        } = events.try_recv().unwrap()
        {
            assert_eq!(latitude, -23.5505);
            assert_eq!(longitude, -46.6333);
            assert_eq!(reason, Refusal::Offline);
        } else {
            panic!("a refusal carries the spot back to the dialog");
        }
    }

    #[test]
    fn mentioning_everyone_sets_its_bit_and_keeps_other_mentions() {
        let member = "491702222222@s.whatsapp.net".to_owned();
        let mentions = vec![crate::mentions::ALL_ID.to_owned(), member.clone()];
        let message = outgoing_text(
            "hello @all @491702222222".to_owned(),
            Some({
                let mut proto = wa::ContextInfo::default();
                proto.stanza_id = Some("quoted".to_owned());
                proto.non_jid_mentions = Some(2);
                proto
            }),
            &mentions,
        );
        let context = context_of(&message).expect("text context");
        assert_eq!(context.non_jid_mentions, Some(3));
        assert_eq!(context.mentioned_jid, vec![member.clone()]);
        assert_eq!(context.stanza_id.as_deref(), Some("quoted"));

        let plain = outgoing_text(
            "hello @all".to_owned(),
            Some({
                let mut proto = wa::ContextInfo::default();
                proto.non_jid_mentions = Some(2);
                proto
            }),
            &[],
        );
        assert_eq!(
            context_of(&plain).and_then(|context| context.non_jid_mentions),
            Some(2),
            "the word alone does not notify"
        );

        let (worker, _, _, _) = receipt_tests::worker();
        let recorded = worker.mentions_of(&recorded_mention_ids(&message));
        assert_eq!(
            recorded,
            vec![
                MentionRef {
                    user: "491702222222".into(),
                    id: member,
                },
                crate::mentions::everyone_ref(),
            ]
        );
    }

    #[test]
    fn text_edits_notify_everyone_only_when_adding_the_mention() {
        let (mut worker, _events, _inbox, _wa) = receipt_tests::worker();
        let chat = "123@g.us".to_owned();
        let member = "12025550123@s.whatsapp.net".to_owned();
        worker.archive.ensure_chat(&chat, "Group").unwrap();
        worker
            .archive
            .set_group_info(&chat, None, std::slice::from_ref(&member), false)
            .unwrap();
        for previously_mentioned in [false, true] {
            let mut original = receipt_tests::own_message("edit-target", 1);
            original.chat = chat.clone();
            // The visible word alone must not suppress a newly added mention.
            original.content = Content::text("Original @all");
            if previously_mentioned {
                original.mentions.push(crate::mentions::everyone_ref());
            }
            worker.archive.insert_message(&original, None).unwrap();
            let (message, content, mentions) = worker
                .prepare_text_edit(
                    &chat,
                    &original.id,
                    "Edited @all @12025550123".into(),
                    vec![member.clone()],
                )
                .unwrap();
            assert_eq!(message.text_content(), Some("Edited @all @12025550123"));
            let context = context_of(&message).unwrap();
            assert_eq!(
                crate::mentions::flagged(context.non_jid_mentions),
                !previously_mentioned,
                "only a newly added everyone mention should notify"
            );
            assert_eq!(context.mentioned_jid, vec![member.clone()]);
            let unsent = worker
                .archive
                .message(&chat, &original.id)
                .unwrap()
                .unwrap();
            assert_eq!(
                unsent.content, original.content,
                "nothing is stored before WhatsApp accepts the edit"
            );
            let generation = worker.begin_edit(&chat, &original.id);
            worker.edit_settled(
                chat.clone(),
                original.id.clone(),
                generation,
                Ok((content, mentions)),
            );
            let stored = worker
                .archive
                .message(&chat, &original.id)
                .unwrap()
                .unwrap();
            assert!(stored.edited);
            assert_eq!(stored.content, Content::text("Edited @all @12025550123"));
            assert_eq!(
                stored.mentions,
                worker.mentions_of(&[member.clone(), crate::mentions::ALL_ID.into()])
            );

            let (removed, content, mentions) = worker
                .prepare_text_edit(&chat, &original.id, "No mentions".into(), vec![])
                .unwrap();
            assert!(recorded_mention_ids(&removed).is_empty());
            let generation = worker.begin_edit(&chat, &original.id);
            worker.edit_settled(
                chat.clone(),
                original.id.clone(),
                generation,
                Ok((content, mentions)),
            );
            let stored = worker
                .archive
                .message(&chat, &original.id)
                .unwrap()
                .unwrap();
            assert!(stored.mentions.is_empty());
        }
    }

    /// A refused edit leaves the archive alone and puts the original back in
    /// the bubble; a late answer to an older edit cannot replace a newer one,
    /// and an edit from the phone wins over one still on its way from here.
    #[test]
    fn edits_reach_the_archive_only_once_whatsapp_accepts_them() {
        let (mut worker, events, _inbox, _wa) = receipt_tests::worker();
        let original = receipt_tests::own_message("edit-target", 1);
        let chat = original.chat.clone();
        let id = original.id.clone();
        worker.archive.insert_message(&original, None).unwrap();
        let stored = |worker: &Worker| worker.archive.message(&chat, &id).unwrap().unwrap();
        let text = |body: &str| (Content::text(body), Vec::new());
        let updates = |events: &std::sync::mpsc::Receiver<Event>| {
            std::iter::from_fn(|| events.try_recv().ok())
                .filter_map(|event| match event {
                    Event::MessageUpdated(message) => Some(message.content),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };

        let refused = worker.begin_edit(&chat, &id);
        worker.edit_settled(chat.clone(), id.clone(), refused, Err("refused".into()));
        assert_eq!(stored(&worker).content, original.content);
        assert!(!stored(&worker).edited);
        assert_eq!(
            updates(&events),
            vec![original.content.clone()],
            "the bubble gets the original back"
        );

        let older = worker.begin_edit(&chat, &id);
        let newer = worker.begin_edit(&chat, &id);
        worker.edit_settled(chat.clone(), id.clone(), older, Err("refused".into()));
        assert!(
            updates(&events).is_empty(),
            "a newer edit is still on its way"
        );
        worker.edit_settled(chat.clone(), id.clone(), newer, Ok(text("Newer")));
        let older = worker.begin_edit(&chat, &id);
        let newest = worker.begin_edit(&chat, &id);
        worker.edit_settled(chat.clone(), id.clone(), newest, Ok(text("Newest")));
        worker.edit_settled(chat.clone(), id.clone(), older, Ok(text("Late")));
        assert_eq!(stored(&worker).content, Content::text("Newest"));
        assert!(stored(&worker).edited);

        let pending = worker.begin_edit(&chat, &id);
        worker.edits.remove(&(chat.clone(), id.clone()));
        worker.edit_settled(chat.clone(), id.clone(), pending, Ok(text("Stale")));
        assert_eq!(
            stored(&worker).content,
            Content::text("Newest"),
            "the phone's edit since stays"
        );
    }

    #[test]
    fn mention_everyone_follows_the_group_gate() {
        let (worker, _, _, _) = receipt_tests::worker();
        let chat = "123@g.us";
        worker.archive.ensure_chat(chat, "Group").unwrap();
        assert!(!worker.wants_mention_all(chat, "hi @all"));
        worker
            .archive
            .set_group_info(chat, None, &["1@s.whatsapp.net".into()], false)
            .unwrap();
        assert!(worker.wants_mention_all(chat, "hi @all"));
        assert!(!worker.wants_mention_all(chat, "hi @alligator"));
        let many: Vec<String> = (0..33)
            .map(|index| format!("{index}@s.whatsapp.net"))
            .collect();
        worker
            .archive
            .set_group_info(chat, None, &many, false)
            .unwrap();
        assert!(!worker.wants_mention_all(chat, "hi @all"));
        worker.archive.set_group_rights(chat, true, true).unwrap();
        assert!(worker.wants_mention_all(chat, "hi @all"));
    }

    #[test]
    fn forwarding_clears_only_the_everyone_bit() {
        let original = wa::Message::text_with_context("hello @all", {
            let mut proto = wa::ContextInfo::default();
            proto.non_jid_mentions = Some(3);
            proto
        });
        let (forward, _) = outgoing_forward(&original, None);
        let context = context_of(&forward).expect("forward context");
        assert_eq!(context.non_jid_mentions, Some(2));
        assert_eq!(context.is_forwarded, Some(true));
        assert!(recorded_mention_ids(&forward).is_empty());
    }

    #[test]
    fn a_reply_does_not_keep_the_quoted_everyone_bit() {
        let quoted = wa::Message::text_with_context("hello @all", {
            let mut proto = wa::ContextInfo::default();
            proto.non_jid_mentions = Some(1);
            proto
        });
        let mut context = {
            let mut proto = wa::ContextInfo::default();
            proto.non_jid_mentions = Some(3);
            proto.quoted_message = MessageField::some(quoted);
            proto
        };
        silence_everyone(&mut context);
        assert_eq!(context.non_jid_mentions, Some(2));
        let quoted = context.quoted_message.as_option().expect("quoted");
        assert_eq!(
            context_of(quoted).and_then(|inner| inner.non_jid_mentions),
            None
        );

        let wrapped = {
            let mut proto = wa::Message::default();
            proto.ephemeral_message = MessageField::some({
                let mut proto = wa::message::FutureProofMessage::default();
                proto.message =
                    MessageField::some(wa::Message::text_with_context("wrapped @all", {
                        let mut proto = wa::ContextInfo::default();
                        proto.non_jid_mentions = Some(3);
                        proto
                    }));
                proto
            });
            proto
        };
        let mut context = {
            let mut proto = wa::ContextInfo::default();
            proto.quoted_message = MessageField::some(wrapped);
            proto
        };
        silence_everyone(&mut context);
        let quoted = context.quoted_message.as_option().unwrap();
        assert_eq!(quoted.text_content(), Some("wrapped @all"));
        assert_eq!(
            context_of(quoted.get_base_message()).and_then(|inner| inner.non_jid_mentions),
            Some(2)
        );
    }

    #[test]
    fn sticker_messages_accept_quote_context() {
        let mut message = {
            let mut proto = wa::Message::default();
            proto.sticker_message = MessageField::some(wa::message::StickerMessage::default());
            proto
        };
        assert!(message.set_context_info({
            let mut proto = wa::ContextInfo::default();
            proto.stanza_id = Some("quoted".to_owned());
            proto
        }));

        let context = message
            .sticker_message
            .as_option()
            .and_then(|sticker| sticker.context_info.as_option())
            .expect("sticker context");
        assert_eq!(context.stanza_id.as_deref(), Some("quoted"));
    }

    #[test]
    fn missing_or_disabled_expiration_leaves_message_normal() {
        for expiration in [None, Some(0)] {
            let mut message = wa::Message::text("hello");
            assert_eq!(apply_ephemeral_expiration(&mut message, expiration), None);
            assert_eq!(message.get_ephemeral_expiration(), None);
        }
    }

    #[test]
    fn configured_expiration_is_added_to_text() {
        for expiration in [86_400, 604_800, 7_776_000] {
            let mut message = wa::Message::text("hello");
            assert_eq!(
                apply_ephemeral_expiration(&mut message, Some(expiration)),
                Some(expiration)
            );
            assert_eq!(message.get_ephemeral_expiration(), Some(expiration));
        }
    }

    #[test]
    fn ephemeral_reply_preserves_quote_context() {
        let mut message = outgoing_text(
            "reply".to_owned(),
            Some({
                let mut proto = wa::ContextInfo::default();
                proto.stanza_id = Some("quoted".to_owned());
                proto
            }),
            &[],
        );

        apply_ephemeral_expiration(&mut message, Some(604_800));

        let context = context_of(&message).expect("context");
        assert_eq!(context.stanza_id.as_deref(), Some("quoted"));
        assert_eq!(context.expiration, Some(604_800));
    }

    #[test]
    fn ephemeral_media_preserves_caption() {
        let mut message = {
            let mut proto = wa::Message::default();
            proto.image_message = MessageField::some({
                let mut proto = wa::message::ImageMessage::default();
                proto.caption = Some("look".to_owned());
                proto
            });
            proto
        };

        apply_ephemeral_expiration(&mut message, Some(7_776_000));

        let image = message.image_message.as_option().expect("image");
        assert_eq!(image.caption.as_deref(), Some("look"));
        assert_eq!(
            image
                .context_info
                .as_option()
                .and_then(|info| info.expiration),
            Some(7_776_000)
        );
    }

    /// Audio attachments must not acquire a forwarding label from their type,
    /// and forwarding metadata must not change whether a clip is a voice note.
    #[test]
    fn audio_type_and_forwarding_metadata_are_independent() {
        for ptt in [None, Some(false), Some(true)] {
            let voice_note = ptt.unwrap_or(false);
            let forwarded_flag = {
                let mut proto = wa::ContextInfo::default();
                proto.is_forwarded = Some(true);
                proto
            };
            let forwarding_score = {
                let mut proto = wa::ContextInfo::default();
                proto.forwarding_score = Some(1);
                proto
            };
            for (context, forwarded) in [
                (None, false),
                (Some(wa::ContextInfo::default()), false),
                (Some(forwarded_flag), true),
                (Some(forwarding_score), true),
            ] {
                let mut audio = wa::message::AudioMessage::default();
                audio.ptt = ptt;
                audio.seconds = Some(52);
                audio.context_info = context.map(MessageField::some).unwrap_or_default();
                let mut message = wa::Message::default();
                message.audio_message = MessageField::some(audio);
                assert!(
                    matches!(classify(&message), Some(Content::Audio { voice_note: note, .. }) if note == voice_note)
                );
                assert_eq!(forwarded_of(message.get_base_message()), forwarded);
            }
        }
    }

    #[test]
    fn forwards_use_only_the_destination_timer() {
        let context = {
            let mut proto = wa::ContextInfo::default();
            proto.expiration = Some(7_776_000);
            proto.ephemeral_setting_timestamp = Some(123);
            proto.ephemeral_shared_secret = Some(vec![1, 2, 3]);
            proto.is_forwarded = Some(true);
            proto.forwarding_score = Some(2);
            proto
        };
        let text = wa::Message::text_with_context("forward me", context.clone());
        let image = {
            let mut proto = wa::Message::default();
            proto.image_message = MessageField::some({
                let mut proto = wa::message::ImageMessage::default();
                proto.caption = Some("caption".into());
                proto.direct_path = Some("/media/path".into());
                proto.context_info = MessageField::some(context.clone());
                proto
            });
            proto
        };
        let contacts = {
            let mut proto = wa::Message::default();
            proto.contacts_array_message = MessageField::some({
                let mut proto = wa::message::ContactsArrayMessage::default();
                proto.context_info = MessageField::some(context);
                proto
            });
            proto
        };
        for original in [text, image, contacts] {
            for timer in [None, Some(0), Some(86_400)] {
                let expected = timer.filter(|value| *value > 0);
                let (forward, expiration) = outgoing_forward(&original, timer);
                assert_eq!(expiration, expected);
                assert_eq!(forward.get_ephemeral_expiration(), expected);
                let context = context_of(&forward).unwrap();
                assert_eq!(context.expiration, expected);
                assert_eq!(context.ephemeral_setting_timestamp, None);
                assert_eq!(context.ephemeral_shared_secret, None);
                assert_eq!(context.is_forwarded, Some(true));
                assert_eq!(context.forwarding_score, Some(3));
                if let Some(image) = forward.image_message.as_option() {
                    assert_eq!(image.caption.as_deref(), Some("caption"));
                    assert_eq!(image.direct_path.as_deref(), Some("/media/path"));
                }
                assert_eq!(original.get_ephemeral_expiration(), Some(7_776_000));
            }
        }
    }

    /// Checks that forwarding retains content while removing source conversation delivery and quote state.
    #[test]
    fn forwarded_rows_keep_content_but_reset_conversation_state() {
        let source = Message {
            id: "source".into(),
            chat: "one@s.whatsapp.net".into(),
            sender: "one@s.whatsapp.net".into(),
            sender_name: Some("Ada".into()),
            from_me: false,
            timestamp: 10,
            content: Content::text("hello"),
            status: Delivery::Read,
            delivered_at: Some(11),
            read_at: Some(12),
            quoted: Some(Quoted {
                labelled: None,
                chat: None,
                id: "quoted".into(),
                sender: "two@s.whatsapp.net".into(),
                sender_name: Some("Bob".into()),
                summary: "earlier".into(),
                mentions: Vec::new(),
            }),
            reactions: vec![Reaction {
                sender: "two@s.whatsapp.net".into(),
                from_me: false,
                emoji: "👍".into(),
            }],
            history_order: None,
            edited: true,
            mentions: Vec::new(),
            forwarded: false,
            thumbnail: Some(vec![1]),
        };
        let mention = MentionRef {
            user: "3".into(),
            id: "3@s.whatsapp.net".into(),
        };

        let forwarded = forwarded_row(
            source,
            "target@g.us".into(),
            "me@s.whatsapp.net".into(),
            "new".into(),
            20,
            vec![mention.clone()],
            Some(vec![2]),
        );

        assert_eq!(forwarded.id, "new");
        assert_eq!(forwarded.chat, "target@g.us");
        assert_eq!(forwarded.sender, "me@s.whatsapp.net");
        assert!(forwarded.from_me && forwarded.forwarded);
        assert_eq!(forwarded.timestamp, 20);
        assert_eq!(forwarded.status, Delivery::Pending);
        assert!(forwarded.delivered_at.is_none() && forwarded.read_at.is_none());
        assert!(forwarded.quoted.is_none() && forwarded.reactions.is_empty());
        assert!(!forwarded.edited);
        assert_eq!(forwarded.mentions, vec![mention]);
        assert_eq!(forwarded.thumbnail, Some(vec![2]));
        assert_eq!(forwarded.content, Content::text("hello"));
    }

    #[test]
    fn pictures_get_a_thumbnail_and_a_jpeg_body() {
        let image = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            300,
            200,
            image::Rgba([200, 30, 30, 255]),
        ));
        let jpeg = encode_jpeg(&image, 80).expect("encodes");
        assert_eq!(&jpeg[..2], &[0xff, 0xd8]);
        let thumbnail = thumbnail_jpeg(&image).expect("thumbnail");
        let small = image::load_from_memory(&thumbnail).expect("decodes");
        assert!(small.width() <= THUMBNAIL_SIDE && small.height() <= THUMBNAIL_SIDE);
    }

    #[test]
    fn millisecond_timestamps_are_normalised() {
        assert_eq!(seconds(1_700_000_000), 1_700_000_000);
        assert_eq!(seconds(1_700_000_000_000), 1_700_000_000);
        assert_eq!(seconds(-1), 0);
    }
}

#[cfg(test)]
mod receipt_tests {
    use super::*;
    use crate::model::{Content, Delivery, Message};

    const ME: &str = "15550001111@s.whatsapp.net";
    pub(super) const PEER: &str = "4917663430455@s.whatsapp.net";
    const PEER_LID: &str = "167650256810092@lid";

    #[tokio::test]
    async fn empty_group_metadata_preserves_titles_and_retries_with_backoff() {
        let (mut worker, _events, _inbox, _wa) = worker();
        let chat = "fixture@g.us";
        worker.archive.ensure_chat(chat, "Weekend plans").unwrap();
        worker
            .handle_command(Command::GroupInfo {
                chat: chat.into(),
                name: Some(String::new()),
                description: String::new(),
                description_generation: 0,
                request_generation: 0,
                participants: vec![PEER.into()],
                read_only: false,
                ephemeral_expiration: None,
                ephemeral_setting_timestamp: None,
                leave_generation: 0,
                info_locked: false,
                admin: false,
                subject_generation: 0,
                settings: Default::default(),
                settings_generation: 0,
            })
            .await;
        assert_eq!(
            worker.archive.chat(chat).unwrap().unwrap().name,
            "Weekend plans"
        );
        assert_eq!(worker.group_info_retry.len(), 1);
        worker.request_group_info(chat, false);
        assert!(
            worker.group_info_queue.is_empty(),
            "incoming traffic must respect backoff"
        );
        worker
            .handle_command(Command::GroupInfo {
                chat: chat.into(),
                name: Some("Current title".into()),
                description: String::new(),
                description_generation: 0,
                request_generation: 0,
                participants: vec![PEER.into()],
                read_only: false,
                ephemeral_expiration: None,
                ephemeral_setting_timestamp: None,
                leave_generation: 0,
                info_locked: false,
                admin: false,
                subject_generation: 0,
                settings: Default::default(),
                settings_generation: 0,
            })
            .await;
        assert_eq!(
            worker.archive.chat(chat).unwrap().unwrap().name,
            "Current title"
        );
        assert!(worker.group_info_retry.is_empty());
    }

    #[tokio::test]
    async fn a_stale_metadata_snapshot_cannot_undo_a_leave() {
        let (mut worker, _events, _inbox, _wa) = worker();
        let chat = "fixture@g.us";
        worker.archive.ensure_chat(chat, "Weekend plans").unwrap();
        worker.me_pn = Some(ME.into());
        worker.archive.set_left(chat, true).unwrap();
        // The leave was confirmed after this request went out, so the answer
        // still lists us and must not resurrect the chat.
        worker.leave_generation.insert(chat.into(), 1);
        worker
            .handle_command(Command::GroupInfo {
                chat: chat.into(),
                name: Some("Weekend plans".into()),
                description: String::new(),
                description_generation: 0,
                request_generation: 0,
                participants: vec![PEER.into(), ME.into()],
                read_only: false,
                ephemeral_expiration: None,
                ephemeral_setting_timestamp: None,
                leave_generation: 0,
                info_locked: false,
                admin: false,
                subject_generation: 0,
                settings: Default::default(),
                settings_generation: 0,
            })
            .await;
        assert!(
            worker.archive.chat(chat).unwrap().unwrap().left,
            "the stale snapshot is ignored"
        );
        // A snapshot asked for after the leave is the phone's current word, so
        // it clears the leave when it lists us again.
        worker
            .handle_command(Command::GroupInfo {
                chat: chat.into(),
                name: Some("Weekend plans".into()),
                description: String::new(),
                description_generation: 0,
                request_generation: 0,
                participants: vec![PEER.into(), ME.into()],
                read_only: false,
                ephemeral_expiration: None,
                ephemeral_setting_timestamp: None,
                leave_generation: 1,
                info_locked: false,
                admin: false,
                subject_generation: 0,
                settings: Default::default(),
                settings_generation: 0,
            })
            .await;
        assert!(
            !worker.archive.chat(chat).unwrap().unwrap().left,
            "being listed again means we are back in"
        );
    }

    #[test]
    fn cached_empty_subjects_are_recovered_and_permanent_failures_stop() {
        let (mut worker, _events, _inbox, _wa) = worker();
        let chat = "fixture@g.us";
        worker.archive.ensure_chat(chat, "Group").unwrap();
        worker
            .archive
            .set_group_info(chat, Some("old"), &[PEER.into()], false)
            .unwrap();
        worker.archive.rename_chat(chat, "").unwrap();
        worker.request_group_info(chat, false);
        assert_eq!(worker.group_info_queue.pop_front().as_deref(), Some(chat));
        worker.handle_failed_group(chat.into(), true);
        worker.request_group_info(chat, false);
        assert!(worker.group_info_queue.is_empty());
    }

    /// The metadata says who may edit the group's info, and a group whose
    /// archive predates that is asked once more even when its name and
    /// members are known.
    #[tokio::test]
    async fn group_metadata_records_who_may_edit_its_info() {
        let (mut worker, _events, _inbox, _wa) = worker();
        let chat = "fixture@g.us";
        worker.archive.ensure_chat(chat, "Weekend plans").unwrap();
        worker
            .archive
            .set_group_info(chat, Some("Weekend plans"), &[PEER.into()], false)
            .unwrap();
        worker.request_group_info(chat, false);
        assert_eq!(
            worker.group_info_queue.pop_front().as_deref(),
            Some(chat),
            "the edit rights are not known yet"
        );
        worker.group_info_requested.clear();
        worker
            .handle_command(Command::GroupInfo {
                chat: chat.into(),
                name: Some("Weekend plans".into()),
                description: String::new(),
                description_generation: 0,
                request_generation: 0,
                participants: vec![PEER.into(), ME.into()],
                read_only: false,
                ephemeral_expiration: None,
                ephemeral_setting_timestamp: None,
                leave_generation: 0,
                info_locked: true,
                admin: true,
                subject_generation: 0,
                settings: Default::default(),
                settings_generation: 0,
            })
            .await;
        let row = worker.archive.chat(chat).unwrap().unwrap();
        assert_eq!(row.info_locked, Some(true));
        assert!(row.admin);
        assert!(row.can_edit_info());
        worker.request_group_info(chat, false);
        assert!(worker.group_info_queue.is_empty(), "nothing left to ask");
    }

    /// WhatsApp's lock and unlock notices change who may edit at once.
    #[tokio::test]
    async fn group_lock_notices_apply_before_the_refresh() {
        use whatsapp_rust::wacore::stanza::groups::GroupNotificationAction;
        let (mut worker, _events, _inbox, _wa) = worker();
        let group = "123-456@g.us";
        worker.archive.ensure_chat(group, "Weekend plans").unwrap();
        worker
            .archive
            .set_group_rights(group, false, false)
            .unwrap();
        for (action, expected) in [
            (
                GroupNotificationAction::Locked { threshold: None },
                Some(true),
            ),
            (GroupNotificationAction::Unlocked, Some(false)),
        ] {
            let update = wa_events::GroupUpdate::builder()
                .group_jid(group.parse().unwrap())
                .timestamp(whatsapp_rust::wacore::time::from_secs(100).unwrap())
                .is_lid_addressing_mode(false)
                .action(Box::new(action))
                .build();
            worker
                .handle_wa_event(Arc::new(wa_events::Event::GroupUpdate(update)))
                .await;
            assert_eq!(
                worker.archive.chat(group).unwrap().unwrap().info_locked,
                expected
            );
            assert_eq!(
                worker.group_info_queue.front().map(String::as_str),
                Some(group),
                "the metadata is refreshed too"
            );
        }
    }

    /// A group created on the phone gets a chat row before its first message.
    #[tokio::test]
    async fn created_group_notice_files_the_chat() {
        use whatsapp_rust::wacore::stanza::groups::GroupNotificationAction;
        use whatsapp_rust::wacore_binary::Node;
        let (mut worker, _events, _inbox, _wa) = worker();
        let group = "123-789@g.us";
        assert!(worker.archive.chat(group).unwrap().is_none());
        let update = wa_events::GroupUpdate::builder()
            .group_jid(group.parse().unwrap())
            .timestamp(whatsapp_rust::wacore::time::from_secs(100).unwrap())
            .is_lid_addressing_mode(false)
            .action(Box::new(GroupNotificationAction::Create {
                raw: Node::default(),
            }))
            .build();
        worker
            .handle_wa_event(Arc::new(wa_events::Event::GroupUpdate(update)))
            .await;
        let chat = worker.archive.chat(group).unwrap().unwrap();
        assert_eq!(chat.last_activity, 100, "dated by the notice, not last");
        assert_eq!(
            worker.group_info_queue.front().map(String::as_str),
            Some(group)
        );
    }

    /// A create notice never brings back a group deleted after it, nor
    /// moves an existing chat's activity back in time.
    #[tokio::test]
    async fn created_group_notice_respects_removal_and_newer_activity() {
        use whatsapp_rust::wacore::stanza::groups::GroupNotificationAction;
        use whatsapp_rust::wacore_binary::Node;
        let notice = |group: &str, at: i64| {
            wa_events::Event::GroupUpdate(
                wa_events::GroupUpdate::builder()
                    .group_jid(group.parse().unwrap())
                    .timestamp(whatsapp_rust::wacore::time::from_secs(at).unwrap())
                    .is_lid_addressing_mode(false)
                    .action(Box::new(GroupNotificationAction::Create {
                        raw: Node::default(),
                    }))
                    .build(),
            )
        };
        let (mut worker, _events, _inbox, _wa) = worker();

        let deleted = "123-111@g.us";
        worker.archive.ensure_chat(deleted, "Deleted").unwrap();
        worker
            .archive
            .remove_chat_through(deleted, 200, true)
            .unwrap();
        assert!(worker.archive.chat(deleted).unwrap().is_none());
        worker.handle_wa_event(Arc::new(notice(deleted, 150))).await;
        assert!(
            worker.archive.chat(deleted).unwrap().is_none(),
            "a notice older than the deletion does not refile the chat"
        );

        let active = "123-222@g.us";
        worker.archive.ensure_chat(active, "Active").unwrap();
        worker.archive.touch_activity(active, 500).unwrap();
        worker.handle_wa_event(Arc::new(notice(active, 300))).await;
        assert_eq!(
            worker.archive.chat(active).unwrap().unwrap().last_activity,
            500,
            "an older notice never moves activity back"
        );
    }

    #[test]
    fn old_groups_without_a_description_refresh_metadata() {
        let (mut worker, _events, _inbox, _wa) = worker();
        let group = "fixture@g.us";
        worker.archive.ensure_chat(group, "Fixture").unwrap();
        worker
            .archive
            .set_group_info(group, Some("Fixture"), &[PEER.into()], false)
            .unwrap();
        worker
            .archive
            .set_group_rights(group, false, false)
            .unwrap();
        worker.request_group_info(group, false);
        assert_eq!(
            worker.group_info_queue.front().map(String::as_str),
            Some(group)
        );
    }

    #[tokio::test]
    async fn group_description_metadata_reaches_ui_without_repeated_queries() {
        let (mut worker, events, _inbox, _wa) = worker();
        let group = "fixture@g.us";
        worker.archive.ensure_chat(group, "Fixture").unwrap();
        for (description, expected) in [
            ("First line 🦀\nSecond line", "First line 🦀\nSecond line"),
            (" \n\t", ""),
        ] {
            worker
                .handle_command(Command::GroupInfo {
                    chat: group.into(),
                    name: Some("Fixture".into()),
                    description: description.into(),
                    description_generation: 0,
                    request_generation: 0,
                    participants: vec![PEER.into()],
                    read_only: false,
                    ephemeral_expiration: None,
                    ephemeral_setting_timestamp: None,
                    leave_generation: 0,
                    info_locked: false,
                    admin: false,
                    subject_generation: 0,
                    settings: Default::default(),
                    settings_generation: 0,
                })
                .await;
            assert!(events.try_iter().any(|event| matches!(event,
                Event::ChatUpdated(row) if row.id == group && row.group_description.as_deref() == Some(expected)
            )));
            // A fresh worker's request set is empty too: the archive alone
            // must distinguish an absent description from an unknown one.
            worker.group_info_requested.clear();
            worker.request_group_info(group, false);
            assert!(worker.group_info_queue.is_empty());
        }
    }

    #[tokio::test]
    async fn group_description_notices_change_and_remove_before_the_refresh() {
        use whatsapp_rust::wacore::stanza::groups::GroupNotificationAction;
        let (mut worker, events, _inbox, _wa) = worker();
        let group = "fixture@g.us";
        for description in [
            Some("First description"),
            Some("Updated 🦀\nSecond line"),
            None,
        ] {
            let update = wa_events::GroupUpdate::builder()
                .group_jid(group.parse().unwrap())
                .timestamp(whatsapp_rust::wacore::time::from_secs(100).unwrap())
                .is_lid_addressing_mode(false)
                .action(Box::new(GroupNotificationAction::Description {
                    id: "fixture-description".into(),
                    description: description.map(str::to_owned),
                }))
                .build();
            worker
                .handle_wa_event(Arc::new(wa_events::Event::GroupUpdate(update)))
                .await;
            let expected = description.unwrap_or_default();
            assert_eq!(
                worker
                    .archive
                    .chat(group)
                    .unwrap()
                    .unwrap()
                    .group_description
                    .as_deref(),
                Some(expected)
            );
            assert!(events.try_iter().any(|event| matches!(event,
                Event::ChatUpdated(row) if row.group_description.as_deref() == Some(expected)
            )));
            assert_eq!(
                worker.group_info_queue.front().map(String::as_str),
                Some(group)
            );
        }
    }

    #[tokio::test]
    async fn group_description_notice_survives_an_earlier_metadata_answer() {
        use whatsapp_rust::wacore::stanza::groups::GroupNotificationAction;
        let (mut worker, _events, _inbox, _wa) = worker();
        let group = "fixture@g.us";
        worker.archive.ensure_chat(group, "Fixture").unwrap();
        let update = wa_events::GroupUpdate::builder()
            .group_jid(group.parse().unwrap())
            .timestamp(whatsapp_rust::wacore::time::from_secs(100).unwrap())
            .is_lid_addressing_mode(false)
            .action(Box::new(GroupNotificationAction::Description {
                id: "fixture-description".into(),
                description: Some("New description".into()),
            }))
            .build();
        worker
            .handle_wa_event(Arc::new(wa_events::Event::GroupUpdate(update)))
            .await;
        worker
            .handle_command(Command::GroupInfo {
                chat: group.into(),
                name: Some("Fixture".into()),
                description: "Old description".into(),
                description_generation: 0,
                request_generation: worker
                    .group_info_generation
                    .get(group)
                    .copied()
                    .unwrap_or(0),
                participants: vec![PEER.into()],
                read_only: false,
                ephemeral_expiration: None,
                ephemeral_setting_timestamp: None,
                leave_generation: 0,
                info_locked: false,
                admin: false,
                subject_generation: 0,
                settings: Default::default(),
                settings_generation: 0,
            })
            .await;
        assert_eq!(
            worker
                .archive
                .chat(group)
                .unwrap()
                .unwrap()
                .group_description
                .as_deref(),
            Some("New description")
        );
    }

    #[tokio::test]
    async fn group_description_cache_survives_failed_metadata_refreshes() {
        let (mut worker, _events, _inbox, _wa) = worker();
        let group = "fixture@g.us";
        worker.archive.ensure_chat(group, "Fixture").unwrap();
        worker
            .archive
            .set_group_description(group, "Cached description")
            .unwrap();
        worker
            .handle_command(Command::GroupInfoFailed {
                chat: group.into(),
                permanent: false,
                request_generation: 0,
            })
            .await;
        assert_eq!(
            worker
                .archive
                .chat(group)
                .unwrap()
                .unwrap()
                .group_description
                .as_deref(),
            Some("Cached description")
        );
        assert_eq!(worker.group_info_retry.len(), 1);
    }

    #[tokio::test]
    async fn superseded_group_metadata_replies_cannot_overwrite_description() {
        for newer_first in [true, false] {
            let (mut worker, events, _inbox, _wa) = worker();
            let group = "fixture@g.us";
            worker.archive.ensure_chat(group, "Fixture").unwrap();
            worker
                .archive
                .set_group_description(group, "Cached description")
                .unwrap();
            let snapshot = |request_generation, description: &str| Command::GroupInfo {
                chat: group.into(),
                request_generation,
                name: Some("Fixture".into()),
                description: description.into(),
                description_generation: 0,
                participants: vec![PEER.into()],
                read_only: false,
                ephemeral_expiration: None,
                ephemeral_setting_timestamp: None,
                leave_generation: 0,
                info_locked: false,
                admin: false,
                subject_generation: 0,
                settings: Default::default(),
                settings_generation: 0,
            };
            worker.request_group_info(group, true);
            let older = snapshot(
                worker
                    .group_info_generation
                    .get(group)
                    .copied()
                    .unwrap_or(0),
                "Old description",
            );
            worker.request_group_info(group, true);
            let newer = snapshot(
                worker
                    .group_info_generation
                    .get(group)
                    .copied()
                    .unwrap_or(0),
                "New description",
            );
            if newer_first {
                worker.handle_command(newer).await;
                events.try_iter().for_each(drop);
                worker.handle_command(older).await;
            } else {
                worker.handle_command(older).await;
                assert_eq!(
                    worker
                        .archive
                        .chat(group)
                        .unwrap()
                        .unwrap()
                        .group_description
                        .as_deref(),
                    Some("Cached description")
                );
                assert!(
                    events.try_iter().next().is_none(),
                    "a superseded reply must not reach the UI"
                );
                worker.handle_command(newer).await;
            }
            assert_eq!(
                worker
                    .archive
                    .chat(group)
                    .unwrap()
                    .unwrap()
                    .group_description
                    .as_deref(),
                Some("New description")
            );
            if newer_first {
                assert!(
                    events.try_iter().next().is_none(),
                    "a superseded reply must not reach the UI"
                );
            } else {
                assert!(events.try_iter().any(|event| matches!(event,
                    Event::ChatUpdated(row) if row.group_description.as_deref() == Some("New description")
                )));
            }
        }
    }

    #[tokio::test]
    async fn superseded_group_metadata_failures_cannot_reset_the_new_request() {
        for permanent in [false, true] {
            let (mut worker, _events, _inbox, _wa) = worker();
            let group = "fixture@g.us";
            worker.request_group_info(group, true);
            let older = worker
                .group_info_generation
                .get(group)
                .copied()
                .unwrap_or(0);
            worker.request_group_info(group, true);
            let newer = worker
                .group_info_generation
                .get(group)
                .copied()
                .unwrap_or(0);
            worker
                .handle_command(Command::GroupInfoFailed {
                    chat: group.into(),
                    request_generation: older,
                    permanent,
                })
                .await;
            assert!(worker.group_info_requested.contains(group));
            assert!(
                worker.group_info_retry.is_empty(),
                "only the current operation may schedule a retry"
            );
            assert!(!worker.group_info_tries.contains_key(group));
            worker
                .handle_command(Command::GroupInfoFailed {
                    chat: group.into(),
                    request_generation: newer,
                    permanent: false,
                })
                .await;
            assert_eq!(worker.group_info_retry.len(), 1);
        }
    }

    fn errors(events: &std::sync::mpsc::Receiver<Event>) -> Vec<String> {
        events
            .try_iter()
            .filter_map(|event| match event {
                Event::Error(message) => Some(message),
                Event::GroupSaving { .. } => Some("saving".into()),
                _ => None,
            })
            .collect()
    }

    /// A rename or photo change goes out only when we may make it, with a
    /// name WhatsApp accepts; empty or unchanged names do nothing at all.
    #[tokio::test]
    async fn group_edits_check_the_rights_and_the_name_first() {
        let (mut worker, events, _inbox, _wa) = worker();
        let chat = "fixture@g.us";
        worker.archive.ensure_chat(chat, "Weekend plans").unwrap();
        worker.archive.set_group_rights(chat, true, false).unwrap();
        for name in ["", "   ", "Weekend plans", " Weekend plans "] {
            worker
                .handle_command(Command::SetGroupName {
                    chat: chat.into(),
                    name: name.into(),
                })
                .await;
        }
        assert!(errors(&events).is_empty(), "nothing to change");

        let rename = |name: &str| Command::SetGroupName {
            chat: chat.into(),
            name: name.into(),
        };
        worker.handle_command(rename("Trip")).await;
        worker
            .handle_command(Command::SetGroupPicture {
                chat: chat.into(),
                jpeg: None,
            })
            .await;
        assert_eq!(
            errors(&events),
            vec![GROUP_EDIT_REFUSED.to_owned(), GROUP_EDIT_REFUSED.to_owned()],
            "a locked group is for admins"
        );

        worker.archive.set_group_rights(chat, true, true).unwrap();
        worker
            .handle_command(rename(&"x".repeat(crate::model::GROUP_NAME_LIMIT + 1)))
            .await;
        worker
            .handle_command(rename(&"é".repeat(crate::model::GROUP_NAME_LIMIT)))
            .await;
        let said = errors(&events);
        assert!(said[0].contains("at most 100 characters"), "{said:?}");
        // The longest name passes and only the missing link stops it.
        assert!(said[1].starts_with("Connect to WhatsApp"), "{said:?}");
        assert_eq!(said.len(), 2, "nothing was sent: {said:?}");
        assert_eq!(
            worker.archive.chat(chat).unwrap().unwrap().name,
            "Weekend plans"
        );
    }

    /// The new name lands once WhatsApp accepts it, and metadata asked for
    /// before that cannot bring the old name back; a later snapshot still
    /// carries renames made on the phone.
    #[tokio::test]
    async fn an_accepted_rename_outlives_a_stale_metadata_snapshot() {
        let (mut worker, events, _inbox, _wa) = worker();
        let chat = "fixture@g.us";
        worker.archive.ensure_chat(chat, "Weekend plans").unwrap();
        worker.archive.set_group_rights(chat, false, false).unwrap();
        let snapshot = |name: &str, subject_generation| Command::GroupInfo {
            chat: chat.into(),
            name: Some(name.into()),
            description: String::new(),
            description_generation: 0,
            request_generation: 0,
            participants: vec![PEER.into(), ME.into()],
            read_only: false,
            ephemeral_expiration: None,
            ephemeral_setting_timestamp: None,
            leave_generation: 0,
            info_locked: false,
            admin: false,
            subject_generation,
            settings: Default::default(),
            settings_generation: 0,
        };
        worker
            .handle_command(Command::GroupEdited {
                chat: chat.into(),
                edit: GroupEdit::Name("Trip".into()),
                result: Ok(()),
            })
            .await;
        let said: Vec<_> = events.try_iter().collect();
        assert!(said.iter().any(|event| matches!(event,
            Event::ChatUpdated(row) if row.name == "Trip")));
        assert!(
            said.iter()
                .any(|event| matches!(event, Event::GroupSaving { saving: false, .. }))
        );
        worker.handle_command(snapshot("Weekend plans", 0)).await;
        assert_eq!(worker.archive.chat(chat).unwrap().unwrap().name, "Trip");
        worker
            .handle_command(snapshot("Renamed on the phone", 1))
            .await;
        assert_eq!(
            worker.archive.chat(chat).unwrap().unwrap().name,
            "Renamed on the phone"
        );
    }

    /// A refused change keeps the old name, says why, and asks WhatsApp who
    /// may edit now; any other failure says what did not happen.
    #[tokio::test]
    async fn a_refused_group_edit_keeps_the_old_value_and_relearns_the_rights() {
        let (mut worker, events, _inbox, _wa) = worker();
        let chat = "fixture@g.us";
        worker.archive.ensure_chat(chat, "Weekend plans").unwrap();
        worker.archive.set_group_rights(chat, false, false).unwrap();
        worker
            .handle_command(Command::GroupEdited {
                chat: chat.into(),
                edit: GroupEdit::Name("Trip".into()),
                result: Err("received a server error response: code=403, text='forbidden'".into()),
            })
            .await;
        assert_eq!(
            errors(&events),
            vec!["saving".to_owned(), GROUP_EDIT_REFUSED.to_owned()]
        );
        assert_eq!(
            worker.archive.chat(chat).unwrap().unwrap().name,
            "Weekend plans"
        );
        assert_eq!(
            worker.group_info_queue.front().map(String::as_str),
            Some(chat)
        );

        worker.group_info_queue.clear();
        worker.group_info_requested.clear();
        for (edit, said) in [
            (
                GroupEdit::Name("Trip".into()),
                "Could not rename the group.",
            ),
            (
                GroupEdit::Picture { removed: false },
                "Could not change the group's photo.",
            ),
            (
                GroupEdit::Picture { removed: true },
                "Could not remove the group's photo.",
            ),
        ] {
            worker
                .handle_command(Command::GroupEdited {
                    chat: chat.into(),
                    edit,
                    result: Err("IQ request timed out".into()),
                })
                .await;
            assert_eq!(errors(&events), vec!["saving".to_owned(), said.to_owned()]);
        }
        assert!(
            worker.group_info_queue.is_empty(),
            "the rights were not the problem"
        );
    }

    /// A removed group photo clears both cached sizes at once.
    #[tokio::test]
    async fn a_removed_group_photo_clears_the_pictures() {
        let (mut worker, events, _inbox, _wa) = worker();
        let chat = "fixture@g.us";
        worker
            .handle_command(Command::GroupEdited {
                chat: chat.into(),
                edit: GroupEdit::Picture { removed: true },
                result: Ok(()),
            })
            .await;
        let cleared: Vec<bool> = events
            .try_iter()
            .filter_map(|event| match event {
                Event::Avatar {
                    id,
                    full,
                    path: None,
                } if id == chat => Some(full),
                _ => None,
            })
            .collect();
        assert_eq!(cleared, vec![false, true]);
    }

    /// A photo WhatsApp accepted from us shows in both sizes from the bytes
    /// we sent, at once, without a lookup the server may still answer with
    /// the old picture. One it refused is forgotten.
    #[tokio::test]
    async fn an_accepted_group_photo_shows_what_was_sent() {
        let (mut worker, events, _inbox, _wa) = worker();
        let chat = "fixture@g.us";
        let jpeg = b"\xff\xd8 new photo".to_vec();
        worker.group_pictures_sent.insert(chat.into(), jpeg.clone());
        worker
            .handle_command(Command::GroupEdited {
                chat: chat.into(),
                edit: GroupEdit::Picture { removed: false },
                result: Ok(()),
            })
            .await;
        let shown: Vec<(bool, PathBuf)> = events
            .try_iter()
            .filter_map(|event| match event {
                Event::Avatar {
                    id,
                    full,
                    path: Some(path),
                } if id == chat => Some((full, path)),
                _ => None,
            })
            .collect();
        assert_eq!(
            shown.iter().map(|(full, _)| *full).collect::<Vec<_>>(),
            vec![false, true]
        );
        for (_, path) in &shown {
            assert_eq!(std::fs::read(path).unwrap(), jpeg);
        }
        assert!(worker.group_pictures_sent.is_empty());

        worker.group_pictures_sent.insert(chat.into(), jpeg.clone());
        worker
            .handle_command(Command::GroupEdited {
                chat: chat.into(),
                edit: GroupEdit::Picture { removed: false },
                result: Err("IQ request timed out".into()),
            })
            .await;
        assert!(worker.group_pictures_sent.is_empty());
    }

    #[test]
    fn refused_group_edits_are_told_from_failures() {
        assert!(group_edit_refused(
            "received a server error response: code=403, text='forbidden'"
        ));
        assert!(group_edit_refused(
            "received a server error response: code=401, text='not-authorized'"
        ));
        assert!(!group_edit_refused("IQ request timed out"));
        assert!(!group_edit_refused(
            "received a server error response: code=500, text='internal-server-error'"
        ));
    }

    /// Pictures are cropped to their centred square and scaled down to the
    /// size WhatsApp uses; smaller ones keep their size.
    #[test]
    fn group_photos_are_centre_cropped_squares_of_at_most_640_pixels() {
        let wide = image::RgbImage::from_fn(1600, 1000, |x, _| {
            // The centre is green, the sides that the crop drops are red.
            if (300..1300).contains(&x) {
                image::Rgb([0, 200, 0])
            } else {
                image::Rgb([200, 0, 0])
            }
        });
        let jpeg = square_picture_jpeg(&image::DynamicImage::ImageRgb8(wide)).unwrap();
        let decoded = image::load_from_memory_with_format(&jpeg, image::ImageFormat::Jpeg)
            .unwrap()
            .to_rgb8();
        assert_eq!(decoded.dimensions(), (640, 640));
        for x in [2, 320, 637] {
            let pixel = decoded.get_pixel(x, 320);
            assert!(
                pixel[1] > 150 && pixel[0] < 60,
                "only the centre is kept: {pixel:?}"
            );
        }
        let small = image::DynamicImage::ImageRgb8(image::RgbImage::new(300, 200));
        let jpeg = square_picture_jpeg(&small).unwrap();
        let decoded = image::load_from_memory(&jpeg).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (200, 200));
    }

    /// Creates a test worker with an in-memory archive and open channels.
    #[test]
    fn group_questions_wait_in_line() {
        let (mut worker, _events, _inbox, _wa) = worker();
        worker
            .archive
            .ensure_chat("1-1@g.us", "Group")
            .expect("chat");
        worker
            .archive
            .ensure_chat("2-2@g.us", "Group")
            .expect("chat");
        worker.request_group_info("1-1@g.us", false);
        worker.request_group_info("2-2@g.us", false);
        worker.request_group_info("1-1@g.us", false);
        assert_eq!(worker.group_info_queue.len(), 2, "asked once each");
        // Forced requests go to the front.
        worker.request_group_info("1-1@g.us", true);
        assert_eq!(
            worker.group_info_queue.front().map(String::as_str),
            Some("1-1@g.us")
        );
        // Without a client, processing schedules a retry.
        worker.pump_group_info();
        assert!(worker.group_info_queue.is_empty() || worker.group_info_retry.len() >= 2);
        // Permanent failures are not requeued.
        worker.group_info_retry.clear();
        worker.handle_failed_group("gone@g.us".to_owned(), true);
        assert!(worker.group_info_retry.is_empty());
        // Retry transient failures after their delay.
        worker.handle_failed_group("busy@g.us".to_owned(), false);
        assert_eq!(worker.group_info_retry.len(), 1);
        assert_eq!(worker.group_info_tries.get("busy@g.us"), Some(&1));
    }

    #[test]
    fn a_serial_forward_starts_the_next_job_on_the_running_one_ack() {
        let mut queue = ForwardQueue::new();
        assert!(queue.push(Vec::new()).is_none());

        let first = queue.push(vec![("only".into(), 7)]).expect("first job");
        assert_eq!(first, ("only".to_owned(), 7));
        assert!(matches!(queue.ack("only"), ForwardStep::Finished));
        assert!(queue.current.is_none());

        let first = queue
            .push(vec![("a".into(), 1), ("b".into(), 2), ("c".into(), 3)])
            .expect("first job");
        assert_eq!(first, ("a".to_owned(), 1));
        // A second batch waits behind the running one, in order.
        assert!(queue.push(vec![("d".into(), 4)]).is_none());
        assert!(matches!(queue.ack("other"), ForwardStep::Ignore));
        assert_eq!(queue.current.as_deref(), Some("a"));

        let ForwardStep::Next { id, payload } = queue.ack("a") else {
            panic!("the running job's ack starts the next one");
        };
        assert_eq!((id.as_str(), payload), ("b", 2));
        // A failed send reports the same ack; a stall would leave current as b.
        let ForwardStep::Next { id, payload } = queue.ack("b") else {
            panic!("a failed send still starts the next job");
        };
        assert_eq!((id.as_str(), payload), ("c", 3));
        let ForwardStep::Next { id, payload } = queue.ack("c") else {
            panic!("the batch queued behind it follows");
        };
        assert_eq!((id.as_str(), payload), ("d", 4));
        assert!(matches!(queue.ack("d"), ForwardStep::Finished));
        assert!(queue.current.is_none());
        assert!(queue.remaining.is_empty());
    }

    /// A locked chat takes what is written in it, but never a forward.
    #[test]
    fn locked_chats_accept_sends_but_not_forwards() {
        let (mut worker, _events, _inbox, _wa) = worker();
        worker.privacy_ready = true;
        worker
            .archive
            .upsert_chat(&Chat::new(PEER.to_owned(), "Fixture".to_owned()))
            .unwrap();
        worker.archive.set_locked(PEER, true).unwrap();
        assert!(worker.destination_writable(PEER));
        assert!(!worker.forward_writable(PEER));
    }

    /// A later lock, departure, or privacy reset prevents a queued send and
    /// leaves an explicit failure, without blocking other authorized recipients.
    #[test]
    fn queued_forwards_recheck_destination_authority_before_starting() {
        for unavailable in ["locked", "left", "read-only", "privacy"] {
            let (mut worker, _events, _inbox, _wa) = worker();
            worker.privacy_ready = true;
            let chat = Chat::new(PEER.to_owned(), "Fixture".to_owned());
            worker.archive.upsert_chat(&chat).unwrap();
            let mut queue = ForwardQueue::new();
            let payload = || (PEER.to_owned(), Jid::pn(PEER), wa::Message::default(), None);
            queue
                .push(vec![
                    ("running".to_owned(), payload()),
                    ("queued".to_owned(), payload()),
                ])
                .unwrap();
            worker.forward_queue = Some(queue);
            worker.store_message(
                Message {
                    status: Delivery::Pending,
                    ..own_message("queued", 1)
                },
                None,
                None,
            );
            match unavailable {
                "locked" => worker.archive.set_locked(PEER, true).unwrap(),
                "left" => worker.archive.set_left(PEER, true).unwrap(),
                "read-only" => worker
                    .archive
                    .set_group_info(PEER, None, &[], true)
                    .unwrap(),
                _ => worker.privacy_ready = false,
            }
            assert!(
                worker.next_writable_forward("running").is_none(),
                "{unavailable}"
            );
            assert!(worker.forward_queue.is_none());
            assert_eq!(
                worker
                    .archive
                    .message(PEER, "queued")
                    .unwrap()
                    .unwrap()
                    .status,
                Delivery::Failed
            );
        }
        let (mut worker, events, _inbox, _wa) = worker();
        worker.privacy_ready = true;
        let mut queue = ForwardQueue::new();
        let blocked = "blocked@s.whatsapp.net".to_owned();
        let chat = Chat::new(blocked.clone(), "Fixture".to_owned());
        worker.archive.upsert_chat(&chat).unwrap();
        worker.archive.set_locked(&blocked, true).unwrap();
        queue
            .push(vec![
                (
                    "running".to_owned(),
                    (PEER.to_owned(), Jid::pn(PEER), wa::Message::default(), None),
                ),
                (
                    "blocked".to_owned(),
                    (
                        blocked.clone(),
                        Jid::pn(&blocked),
                        wa::Message::default(),
                        None,
                    ),
                ),
                (
                    "blocked-second".to_owned(),
                    (
                        blocked.clone(),
                        Jid::pn(&blocked),
                        wa::Message::default(),
                        None,
                    ),
                ),
                (
                    "allowed".to_owned(),
                    (PEER.to_owned(), Jid::pn(PEER), wa::Message::default(), None),
                ),
            ])
            .unwrap();
        worker.forward_queue = Some(queue);
        let (id, job) = worker
            .next_writable_forward("running")
            .expect("the next authorized recipient");
        assert_eq!(id, "allowed");
        assert_eq!(job.0, PEER);
        let errors = std::iter::from_fn(|| events.try_recv().ok())
            .filter(|event| matches!(event, Event::Error(reason) if reason == "This conversation is read-only in ZapFast"))
            .count();
        assert_eq!(errors, 1, "skipped jobs report one error per advance");
        assert_eq!(
            worker.forward_queue.as_ref().unwrap().current.as_deref(),
            Some("allowed")
        );
    }

    /// Stores a text whose raw protobuf carries `score`, or none at all.
    fn store_forwarded(worker: &Worker, id: &str, score: Option<u32>) {
        let raw = score.map(|score| {
            let mut context = wa::ContextInfo::default();
            context.is_forwarded = Some(true);
            context.forwarding_score = Some(score);
            wa::Message::text_with_context("Forwarded fixture", context).encode_to_vec()
        });
        worker
            .archive
            .insert_message(&own_message(id, 1), raw.as_deref())
            .unwrap();
    }

    /// WhatsApp lets a message forwarded many times go to one chat at a time,
    /// judged from the score in the archived protobuf.
    #[test]
    fn the_forwarding_limit_follows_the_forwarding_score() {
        let (worker, _events, _inbox, _wa) = worker();
        store_forwarded(&worker, "plain", None);
        store_forwarded(&worker, "twice", Some(4));
        store_forwarded(&worker, "many", Some(5));
        store_forwarded(&worker, "sentinel", Some(127));
        let limit = |ids: &[&str]| {
            worker.forward_limit(
                PEER,
                &ids.iter().map(|id| (*id).to_owned()).collect::<Vec<_>>(),
            )
        };
        assert_eq!(limit(&["plain"]), 5);
        assert_eq!(limit(&["twice"]), 5);
        assert_eq!(limit(&["missing"]), 5);
        assert_eq!(limit(&["many"]), 1);
        assert_eq!(limit(&["sentinel"]), 1);
        assert_eq!(
            limit(&["plain", "sentinel"]),
            1,
            "one such message is enough"
        );
    }

    #[tokio::test]
    async fn a_batch_over_the_forwarding_limit_is_refused_whole() {
        for (score, recipients) in [(Some(127), 2), (None, 6)] {
            let (mut worker, events, _inbox, _wa) = worker();
            worker.privacy_ready = true;
            store_forwarded(&worker, "source", score);
            let to_chats: Vec<ChatId> = (0..recipients)
                .map(|index| format!("1555000{index:04}@s.whatsapp.net"))
                .collect();
            worker
                .handle_command(Command::Forward {
                    from_chat: PEER.into(),
                    messages: vec!["source".into()],
                    to_chats: to_chats.clone(),
                })
                .await;
            let errors: Vec<String> = events
                .try_iter()
                .filter_map(|event| match event {
                    Event::Error(error) => Some(error),
                    _ => None,
                })
                .collect();
            let expected = if score.is_some() {
                "Messages forwarded many times can be forwarded to only one chat"
            } else {
                "Messages can be forwarded to up to 5 chats at once"
            };
            assert_eq!(errors, [expected]);
            for chat in &to_chats {
                assert!(worker.archive.messages(chat, None, 10).unwrap().is_empty());
            }
        }
    }

    #[tokio::test]
    async fn the_forwarding_limit_is_answered_for_the_dialog() {
        let (mut worker, events, _inbox, _wa) = worker();
        store_forwarded(&worker, "many", Some(127));
        worker
            .handle_command(Command::ForwardLimit {
                from_chat: PEER.into(),
                messages: vec!["many".into()],
            })
            .await;
        assert!(events.try_iter().any(|event| matches!(
            event,
            Event::ForwardLimit { from_chat, messages, limit: 1 }
                if from_chat == PEER && messages == ["many"]
        )));
    }

    /// A batch belongs to the session that was sending it. The proxy-change
    /// reconnect stops the bot and starts another, and every send is its own
    /// task, so one can report its tick after the stop, inside the window the
    /// connection teardown waits for. Dropping the queue with the session is
    /// what keeps that tick from resuming the batch through the one after it.
    #[tokio::test]
    async fn stopping_the_bot_drops_a_running_forward_batch() {
        let (mut worker, _events, _inbox, _wa) = worker();
        let to_chat = PEER.to_owned();
        let jid = Jid::pn(PEER);
        let mut queue = ForwardQueue::new();
        let running = queue
            .push(vec![
                (
                    "a".to_owned(),
                    (to_chat.clone(), jid.clone(), wa::Message::default(), None),
                ),
                (
                    "b".to_owned(),
                    (to_chat.clone(), jid.clone(), wa::Message::default(), None),
                ),
            ])
            .expect("the first job of the batch");
        assert_eq!(running.0, "a");
        worker.forward_queue = Some(queue);
        for id in ["a", "b"] {
            let pending = Message {
                status: Delivery::Pending,
                ..own_message(id, 1)
            };
            worker.store_message(pending, None, None);
        }

        worker.stop_bot().await;

        // Nothing resends a pending message, so the one that never started is
        // failed, visibly; the running one still reports for itself.
        let status = |worker: &Worker, id| {
            worker
                .archive
                .message(PEER, id)
                .expect("read")
                .expect("stored")
                .status
        };
        assert_eq!(status(&worker, "b"), Delivery::Failed);
        assert_eq!(status(&worker, "a"), Delivery::Pending);

        assert!(
            worker.forward_queue.is_none(),
            "the batch goes with the session that was sending it"
        );
        // The stale tick therefore has nothing to advance, whichever job it
        // names: the queue it belonged to is gone.
        worker.advance_serial_forward("a");
        assert!(worker.forward_queue.is_none());
    }

    pub(super) fn worker() -> (
        Worker,
        std::sync::mpsc::Receiver<Event>,
        mpsc::UnboundedReceiver<Command>,
        mpsc::UnboundedReceiver<RuntimeEvent>,
    ) {
        let (events, events_rx) = std::sync::mpsc::channel();
        let (commands, inbox) = mpsc::unbounded_channel();
        let (wa_sender, wa_events) = mpsc::unbounded_channel();
        // Each worker gets its own directory: tests run in parallel, and one
        // test's cached avatar or download must not answer another's lookup.
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("zapfast-worker-test-{}-{n}", std::process::id()));
        let worker = Worker {
            privacy_ready: true,
            privacy_confirmed: true,
            privacy_snapshot: false,
            privacy_reveal_at: None,
            privacy_attempts: 0,
            privacy_warned: false,
            privacy_recovering: false,
            privacy_generation: 0,
            message_removals_in_flight: HashSet::new(),
            contact_generation: 0,
            privacy_retry: Instant::now(),
            star_generation: HashMap::new(),
            star_session: 0,
            pin_generation: HashMap::new(),
            withheld_pages: Vec::new(),
            dirs: crate::paths::AppDirs::under(&root).as_account(),
            events,
            commands,
            waker: Waker::default(),
            archive: Archive::in_memory().expect("archive"),
            client: None,
            handle: None,
            call: None,
            phone_only_calls: Vec::new(),
            call_devices: crate::calls::DeviceList::default(),
            wa_sender,
            me_pn: Some(ME.to_owned()),
            me_lid: None,
            me_name: None,
            me_about: None,
            lid_to_pn: HashMap::new(),
            contacts: HashMap::new(),
            status: LinkStatus::Connected,
            pairing_phone: None,
            pair_code: None,
            qr: None,
            syncing: false,
            sync_deadline: None,
            group_info_requested: HashSet::new(),
            leave_generation: HashMap::new(),
            subject_generation: HashMap::new(),
            settings_generation: HashMap::new(),
            group_pictures_sent: HashMap::new(),
            description_generation: HashMap::new(),
            group_info_generation: HashMap::new(),
            edits: HashMap::new(),
            edit_sequence: 0,
            group_info_queue: std::collections::VecDeque::new(),
            group_info_tries: HashMap::new(),
            group_info_retry: Vec::new(),
            presence_subscribed: HashSet::new(),
            download_folder: None,
            cleaned_download_folders: HashSet::new(),
            keep_chats_archived: true,
            phone_unarchive_chats: None,
            phone_unarchive_unread: false,
            online_wanted: false,
            online_changed: Instant::now(),
            online_sent: None,
            presence_in_flight: false,
            presence_refresh_pending: false,
            presence_generation: 0,
            channel_list_generation: 0,
            pending_older: HashMap::new(),
            older_warned: HashSet::new(),
            pending_avatars: HashMap::new(),
            channel_pictures: Default::default(),
            sticker_fetches: HashSet::new(),
            sticker_downloads: HashSet::new(),
            recent_hashes: HashMap::new(),
            emoji_cache: HashMap::new(),
            favorite_fetches: HashSet::new(),
            sticker_pace: Default::default(),
            sticker_failed: HashSet::new(),
            sticker_download_failed: HashSet::new(),
            sticker_resume: false,
            sticker_clicked: HashSet::new(),
            sticker_retried: HashSet::new(),
            sticker_arrivals: 0,
            favorites_pushing: false,
            favorites_again: false,
            favorites_recovered: true,
            favorites_recovering: false,
            first_names_recovered: true,
            first_names_recovering: false,
            stars_recovered: true,
            stars_recovering: false,
            downloads: HashSet::new(),
            read_sync: ReadSync::default(),
            favorite_chats: Default::default(),
            poll_decrypting: 0,
            poll_history: Default::default(),
            poll_sending: HashSet::new(),
            prefetch: prefetch::State::default(),
            older_serial: 0,
            late_older: VecDeque::new(),
            older_ids_match: false,
            interactive_sending: HashMap::new(),
            awaiting_ack: HashMap::new(),
            receipts_watch: None,
            receipts_pruned: Instant::now(),
            early: Default::default(),
            link_watch: Default::default(),
            forward_queue: None,
        };
        worker.archive.set_meta("me_pn", ME).unwrap();
        (worker, events_rx, inbox, wa_events)
    }

    /// View-once photos filed as attachments before they were recognised
    /// become the placeholder; ordinary photos and any file already on disk
    /// stay as they are.
    #[test]
    fn archived_view_once_media_becomes_the_placeholder() {
        let (mut worker, _events, _commands, _wa) = worker();
        worker.archive.ensure_chat(PEER, "Demo").unwrap();
        let image = |view_once| {
            let mut proto = wa::Message::default();
            proto.image_message = MessageField::some({
                let mut proto = wa::message::ImageMessage::default();
                proto.mimetype = Some("image/jpeg".into());
                proto.view_once = view_once;
                proto
            });
            proto
        };
        let once = image(Some(true));
        let plain = image(None);
        for (id, raw, path) in [
            ("once", &once, None),
            ("once-on-disk", &once, Some("/fixture/once.jpg")),
            ("plain", &plain, None),
        ] {
            let mut message = own_message(id, 1);
            let Some(Content::Image {
                mut media, caption, ..
            }) = classify_base(raw.get_base_message())
            else {
                panic!("not an image");
            };
            media.path = path.map(std::path::PathBuf::from);
            // Filed as an ordinary photo, before view once was recognised.
            message.content = Content::Image {
                caption,
                media,
                motion: None,
            };
            worker
                .archive
                .insert_message(&message, Some(&raw.encode_to_vec()))
                .unwrap();
        }
        worker.backfill_view_once();
        let stored = |id: &str| worker.archive.message(PEER, id).unwrap().unwrap().content;
        assert_eq!(
            stored("once"),
            Content::PhoneOnly {
                view_once: true,
                live_location: false,
                once: Some(crate::model::OnceMedia::Photo),
            }
        );
        assert!(matches!(stored("once-on-disk"), Content::Image { .. }));
        assert!(matches!(stored("plain"), Content::Image { .. }));
        assert_eq!(
            worker.archive.meta("view_once_media").unwrap().as_deref(),
            Some("1")
        );
    }

    #[test]
    fn archived_round_videos_become_notes_and_keep_their_rows() {
        let (mut worker, _events, _commands, _wa) = worker();
        worker.archive.ensure_chat(PEER, "Demo").unwrap();
        let clip = {
            let mut proto = wa::message::VideoMessage::default();
            proto.mimetype = Some("video/mp4".into());
            proto
        };
        let round = {
            let mut proto = wa::Message::default();
            proto.ptv_message = MessageField::some(clip.clone());
            proto
        };
        let plain = {
            let mut proto = wa::Message::default();
            proto.video_message = MessageField::some(clip);
            proto
        };
        for (id, raw) in [("round", &round), ("plain", &plain)] {
            let mut message = own_message(id, 1);
            let Some(Content::Video {
                mut media,
                seconds,
                gif,
                ..
            }) = classify(raw)
            else {
                panic!("not a video");
            };
            media.path = Some(std::path::PathBuf::from("/fixture/clip.mp4"));
            // Filed before round videos were told apart.
            message.content = Content::Video {
                caption: None,
                media,
                seconds,
                gif,
                note: false,
            };
            worker
                .archive
                .insert_message(&message, Some(&raw.encode_to_vec()))
                .unwrap();
        }
        worker.backfill_video_notes();
        let stored = |id: &str| worker.archive.message(PEER, id).unwrap().unwrap().content;
        match stored("round") {
            Content::Video { note, media, .. } => {
                assert!(note);
                assert_eq!(
                    media.path,
                    Some(std::path::PathBuf::from("/fixture/clip.mp4"))
                );
            }
            other => panic!("unexpected {other:?}"),
        }
        assert!(matches!(
            stored("plain"),
            Content::Video { note: false, .. }
        ));
        assert_eq!(
            worker.archive.meta("video_notes").unwrap().as_deref(),
            Some("1")
        );
    }

    pub(super) fn own_message(id: &str, timestamp: i64) -> Message {
        Message {
            id: id.into(),
            chat: PEER.into(),
            sender: ME.into(),
            sender_name: None,
            from_me: true,
            timestamp,
            content: Content::text("hi"),
            status: Delivery::Sent,
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

    fn receipt(chat: &str, ids: &[&str], kind: ReceiptType) -> wa_events::Receipt {
        let chat: Jid = chat.parse().expect("jid");
        wa_events::Receipt::builder()
            .message_ids(ids.iter().map(|id| (*id).into()).collect())
            .source(MessageSource {
                chat: chat.clone(),
                sender: chat,
                ..Default::default()
            })
            .timestamp(whatsapp_rust::wacore::time::now_utc())
            .r#type(kind)
            .offline(false)
            .build()
    }

    #[test]
    fn group_checks_wait_for_every_recipient_and_do_not_read_earlier_messages() {
        let (mut worker, _events, _inbox, _wa) = worker();
        let group = "123-456@g.us";
        let other = "12025550123@s.whatsapp.net";
        worker.archive.ensure_chat(group, "Group").unwrap();
        worker
            .archive
            .set_group_info(
                group,
                None,
                &[ME.into(), PEER_LID.into(), other.into()],
                false,
            )
            .unwrap();
        for (id, timestamp) in [("old", 100), ("new", 200)] {
            worker.store_message(
                Message {
                    chat: group.into(),
                    ..own_message(id, timestamp)
                },
                None,
                None,
            );
            assert!(worker.save_group_recipients(
                group,
                id,
                &[ME.into(), PEER_LID.into(), other.into()]
            ));
        }
        let send = |worker: &mut Worker, sender: &str, kind| {
            let mut receipt = receipt(group, &["new"], kind);
            receipt.source.sender = sender.parse().unwrap();
            receipt.source.is_group = true;
            worker.on_receipt(&receipt);
        };
        let status =
            |worker: &Worker, id| worker.archive.message(group, id).unwrap().unwrap().status;
        send(&mut worker, PEER_LID, ReceiptType::Read);
        send(&mut worker, ME, ReceiptType::Read);
        send(&mut worker, "12025550999@s.whatsapp.net", ReceiptType::Read);
        assert_eq!(status(&worker, "new"), Delivery::Sent);
        // A new alias or device is not another reader. Learning a mapping after
        // the first receipt must also merge its saved audience entry.
        worker.learn_lid("167650256810092", "4917663430455");
        send(&mut worker, PEER, ReceiptType::Read);
        send(
            &mut worker,
            "4917663430455:2@s.whatsapp.net",
            ReceiptType::Read,
        );
        assert_eq!(status(&worker, "new"), Delivery::Sent);
        send(&mut worker, other, ReceiptType::Delivered);
        assert_eq!(status(&worker, "new"), Delivery::Delivered);
        // Departures and joins do not rewrite the message's original audience.
        worker
            .archive
            .set_group_info(group, None, &[ME.into(), PEER.into()], false)
            .unwrap();
        send(&mut worker, PEER, ReceiptType::Read);
        assert_eq!(status(&worker, "new"), Delivery::Delivered);
        send(&mut worker, other, ReceiptType::Read);
        assert_eq!(status(&worker, "new"), Delivery::Read);
        assert_eq!(status(&worker, "old"), Delivery::Sent);
        send(&mut worker, PEER, ReceiptType::Delivered);
        assert_eq!(status(&worker, "new"), Delivery::Read);
    }

    fn sent_elsewhere(chat: &str, id: &str, timestamp: i64) -> (Arc<wa::Message>, MessageInfo) {
        let chat: Jid = chat.parse().unwrap();
        let info = MessageInfo {
            id: id.into(),
            source: MessageSource {
                chat: chat.clone(),
                sender: ME.parse().unwrap(),
                is_from_me: true,
                is_group: chat.is_group(),
                ..Default::default()
            },
            timestamp: whatsapp_rust::wacore::time::from_secs(timestamp).unwrap(),
            ..Default::default()
        };
        let message = {
            let mut proto = wa::Message::default();
            proto.conversation = Some("sent from the phone".into());
            proto
        };
        (Arc::new(message), info)
    }

    #[test]
    fn receipts_that_outrun_a_message_sent_elsewhere_still_move_its_ticks() {
        let (mut worker, _events, _inbox, _wa) = worker();
        let now = crate::util::now();
        worker.store_message(own_message("before", now - 60), None, None);
        worker
            .archive
            .set_status(PEER, "before", Delivery::Delivered, now - 50)
            .unwrap();
        worker.on_receipt(&receipt(PEER, &["phone"], ReceiptType::Delivered));
        worker.on_receipt(&receipt(PEER, &["phone"], ReceiptType::Read));
        let (message, info) = sent_elsewhere(PEER, "phone", now);
        worker.ingest(&message, &info);
        let stored = worker.archive.message(PEER, "phone").unwrap().unwrap();
        assert_eq!(stored.status, Delivery::Read);
        assert!(stored.read_at.is_some() && stored.delivered_at.is_some());
        // As with a live read receipt, the earlier message was read too.
        assert_eq!(
            worker
                .archive
                .message(PEER, "before")
                .unwrap()
                .unwrap()
                .status,
            Delivery::Read
        );
    }

    #[tokio::test]
    async fn a_group_message_sent_elsewhere_takes_the_current_members_as_its_audience() {
        let (mut worker, events, _inbox, _wa) = worker();
        let group = "123-456@g.us";
        let other = "12025550123@s.whatsapp.net";
        let now = crate::util::now();
        worker.archive.ensure_chat(group, "Group").unwrap();
        worker
            .archive
            .set_group_info(group, None, &[ME.into(), PEER.into(), other.into()], false)
            .unwrap();
        let send = |worker: &mut Worker, sender: &str, alt: Option<&str>, kind| {
            let mut receipt = receipt(group, &["phone"], kind);
            receipt.source.sender = sender.parse().unwrap();
            receipt.source.sender_alt = alt.map(|alt| alt.parse().unwrap());
            receipt.source.is_group = true;
            worker.on_receipt(&receipt);
        };
        // A privacy-id reader, before the message itself arrives.
        send(&mut worker, PEER_LID, Some(PEER), ReceiptType::Read);
        let (message, info) = sent_elsewhere(group, "phone", now);
        worker.ingest(&message, &info);
        let status = |worker: &Worker| {
            worker
                .archive
                .message(group, "phone")
                .unwrap()
                .unwrap()
                .status
        };
        assert_eq!(status(&worker), Delivery::Sent);
        worker
            .handle_command(Command::WatchReceipts(Some((group.into(), "phone".into()))))
            .await;
        let latest = |events: &std::sync::mpsc::Receiver<Event>| {
            events
                .try_iter()
                .filter_map(|event| match event {
                    Event::Receipts(receipts) => Some(receipts),
                    _ => None,
                })
                .last()
                .expect("receipts")
        };
        let receipts = latest(&events);
        assert!(receipts.audience_known());
        assert_eq!(receipts.read().len(), 1);
        assert_eq!(
            receipts.read()[0].id,
            PEER,
            "the privacy id maps to the number"
        );
        assert_eq!(receipts.remaining(), 1);
        send(&mut worker, other, None, ReceiptType::Delivered);
        let receipts = latest(&events);
        assert_eq!(receipts.delivered()[0].id, other);
        assert_eq!(receipts.remaining(), 0);
        assert_eq!(status(&worker), Delivery::Delivered);
        send(&mut worker, other, None, ReceiptType::Read);
        assert_eq!(status(&worker), Delivery::Read);
        assert_eq!(latest(&events).read().len(), 2);
        worker.handle_command(Command::WatchReceipts(None)).await;
        send(&mut worker, other, None, ReceiptType::Played);
        assert!(
            !events
                .try_iter()
                .any(|event| matches!(event, Event::Receipts(_))),
            "a closed dialog is not followed"
        );
    }

    #[test]
    fn history_keeps_ephemeral_metadata() {
        let parsed = parse_conversation({
            let mut proto = wa::Conversation::default();
            proto.id = PEER.into();
            proto.ephemeral_expiration = Some(7_776_000);
            proto.ephemeral_setting_timestamp = Some(1_700_000_000);
            proto
        });

        assert_eq!(parsed.ephemeral_expiration, Some(7_776_000));
        assert_eq!(parsed.ephemeral_setting_timestamp, Some(1_700_000_000));
    }

    fn history_entry(
        chat: &str,
        id: &str,
        from_me: bool,
        participant: Option<&str>,
        message: wa::Message,
        reactions: Vec<wa::Reaction>,
        secret: Option<Vec<u8>>,
    ) -> wa::HistorySyncMsg {
        {
            let mut proto = wa::HistorySyncMsg::default();
            proto.message = MessageField::some({
                let mut proto = wa::WebMessageInfo::default();
                proto.key = MessageField::some({
                    let mut proto = wa::MessageKey::default();
                    proto.remote_jid = Some(chat.into());
                    proto.from_me = Some(from_me);
                    proto.id = Some(id.into());
                    proto.participant = participant.map(str::to_owned);
                    proto
                });
                proto.message = MessageField::some(message);
                proto.message_timestamp = Some(100);
                proto.reactions = reactions;
                proto.message_secret = secret;
                proto
            });
            proto
        }
    }

    /// A pin for everyone of the message "target".
    fn target_pin() -> wa::message::PinInChatMessage {
        let mut pin = wa::message::PinInChatMessage::default();
        pin.key = MessageField::some({
            let mut key = wa::MessageKey::default();
            key.id = Some("target".into());
            key
        });
        pin.r#type = Some(wa::message::pin_in_chat_message::Type::PinForAll);
        pin
    }

    #[test]
    fn pin_action_reads_the_target_and_unpin_type() {
        let pin = target_pin();
        assert_eq!(
            pin_action(&pin, 604_800),
            Some(("target".into(), true, 604_800))
        );
        let mut unpin = pin;
        unpin.r#type = Some(wa::message::pin_in_chat_message::Type::UnpinForAll);
        assert_eq!(pin_action(&unpin, 0), Some(("target".into(), false, 0)));
    }

    #[test]
    fn history_reads_a_pin_without_showing_it_as_a_message() {
        let message = {
            let mut proto = wa::Message::default();
            proto.pin_in_chat_message = MessageField::some(target_pin());
            proto.message_context_info = MessageField::some({
                let mut proto = wa::MessageContextInfo::default();
                proto.message_add_on_duration_in_secs = Some(604_800);
                proto
            });
            proto
        };
        let parsed = parse_conversation({
            let mut proto = wa::Conversation::default();
            proto.id = PEER.into();
            proto.messages = vec![history_entry(
                PEER,
                "pin",
                true,
                None,
                message,
                Vec::new(),
                None,
            )];
            proto
        });
        assert!(
            parsed.messages.is_empty(),
            "a pin is not a message of its own"
        );
        assert_eq!(parsed.pins.len(), 1);
        assert_eq!(parsed.pins[0].target, "target");
        assert!(parsed.pins[0].pinned);
        assert_eq!(parsed.pins[0].duration_secs, 604_800);
        assert!(
            parsed.pins[0].pinner.by_me,
            "the notice says who pinned it, and this one is ours"
        );
    }

    #[test]
    fn reaction_emoji_prefers_text_then_grouping_key() {
        assert_eq!(
            reaction_emoji(Some("🏆"), Some("👍")).as_deref(),
            Some("🏆")
        );
        assert_eq!(reaction_emoji(Some(""), Some("🏆")).as_deref(), Some("🏆"));
        assert_eq!(reaction_emoji(None, Some("🏆")).as_deref(), Some("🏆"));
        assert_eq!(reaction_emoji(Some("  "), None), None);
    }

    #[test]
    fn history_applies_a_standalone_custom_reaction_from_another_sender() {
        let group = "123-456@g.us";
        let reactor = "12025550999@s.whatsapp.net";
        let parsed = parse_conversation({
            let mut proto = wa::Conversation::default();
            proto.id = group.into();
            proto.messages = vec![
                history_entry(
                    group,
                    "photo",
                    false,
                    Some(PEER),
                    {
                        let mut proto = wa::Message::default();
                        proto.conversation = Some("caption".into());
                        proto
                    },
                    Vec::new(),
                    None,
                ),
                history_entry(
                    group,
                    "react",
                    false,
                    Some(reactor),
                    {
                        let mut proto = wa::Message::default();
                        proto.reaction_message = MessageField::some({
                            let mut proto = wa::message::ReactionMessage::default();
                            proto.key = MessageField::some({
                                let mut proto = wa::MessageKey::default();
                                proto.remote_jid = Some(group.into());
                                proto.from_me = Some(false);
                                proto.id = Some("photo".into());
                                proto.participant = Some(PEER.into());
                                proto
                            });
                            proto.text = Some("🏆".into());
                            proto
                        });
                        proto
                    },
                    Vec::new(),
                    None,
                ),
            ];
            proto
        });
        assert!(parsed.messages.iter().all(|message| message.id != "react"));
        assert_eq!(parsed.reactions.len(), 1);
        let (mut worker, _events, _inbox, _wa) = worker();
        worker.apply_history(
            ParsedHistory {
                chats: vec![parsed],
                push_names: Vec::new(),
                lids: Vec::new(),
                stickers: Vec::new(),
            },
            true,
        );
        let stored = worker
            .archive
            .message(group, "photo")
            .unwrap()
            .expect("parent");
        assert_eq!(stored.reactions.len(), 1);
        assert_eq!(stored.reactions[0].emoji, "🏆");
        assert!(!stored.reactions[0].from_me);
        assert_eq!(stored.reactions[0].sender, reactor);
    }

    fn file_history_entries(worker: &mut Worker, messages: Vec<wa::HistorySyncMsg>) {
        file_history_entries_for(worker, PEER, messages);
    }

    fn file_history_entries_for(
        worker: &mut Worker,
        chat: &str,
        messages: Vec<wa::HistorySyncMsg>,
    ) {
        worker.apply_history(
            ParsedHistory {
                chats: vec![parse_conversation({
                    let mut proto = wa::Conversation::default();
                    proto.id = chat.into();
                    proto.messages = messages;
                    proto
                })],
                push_names: Vec::new(),
                lids: Vec::new(),
                stickers: Vec::new(),
            },
            false,
        );
    }

    #[test]
    fn history_diagnostics_describe_edit_snapshots_without_personal_data() {
        let edit = {
            let mut proto = wa::Message::default();
            proto.protocol_message = MessageField::some({
                let mut proto = wa::message::ProtocolMessage::default();
                proto.r#type = Some(wa::message::protocol_message::Type::MESSAGE_EDIT);
                proto.key = MessageField::some({
                    let mut proto = wa::MessageKey::default();
                    proto.id = Some("private-parent-id".into());
                    proto
                });
                proto.edited_message =
                    MessageField::some(wa::Message::text("Private fixture body"));
                proto
            });
            proto
        };
        let mut absent = history_entry(
            PEER,
            "private-missing-id",
            false,
            None,
            wa::Message::default(),
            Vec::new(),
            None,
        );
        let absent_info = absent.message.as_option_mut().unwrap();
        absent_info.message = MessageField::none();
        absent_info.message_stub_type = Some(wa::web_message_info::StubType::CIPHERTEXT);
        let parsed = parse_conversation({
            let mut proto = wa::Conversation::default();
            proto.id = PEER.into();
            proto.messages = vec![
                history_entry(PEER, "private-edit-id", false, None, edit, Vec::new(), None),
                history_entry(
                    PEER,
                    "private-visible-id",
                    false,
                    None,
                    wa::Message::text("Private fixture body"),
                    Vec::new(),
                    None,
                ),
                absent,
            ];
            proto
        });
        assert_eq!(parsed.diagnostics.entries, 3);
        assert_eq!(parsed.diagnostics.rekeyed_edit_snapshots, 1);
        assert_eq!(parsed.diagnostics.edit_snapshots, 1);
        assert_eq!(parsed.diagnostics.missing_body, 1);
        assert_eq!(parsed.diagnostics.ciphertext_stubs, 1);
        assert_eq!(parsed.messages.len(), 2);
        let report = format!("{:?}", parsed.diagnostics);
        assert!(!report.contains("private-"));
        assert!(!report.contains("Private fixture body"));
        assert!(!report.contains(PEER));
    }

    #[test]
    fn history_fidelity_keeps_the_phones_order_within_one_second() {
        let (mut worker, _events, _inbox, _wa) = worker();
        let entries = [("third", 3), ("first", 1), ("second", 2)]
            .into_iter()
            .map(|(id, order)| {
                let mut entry = history_entry(
                    PEER,
                    id,
                    false,
                    None,
                    wa::Message::text(id),
                    Vec::new(),
                    None,
                );
                entry.msg_order_id = Some(order);
                entry
            })
            .collect();
        file_history_entries(&mut worker, entries);
        let messages = worker.archive.messages(PEER, None, 10).unwrap();
        assert_eq!(
            messages
                .iter()
                .map(|message| message.id.as_str())
                .collect::<Vec<_>>(),
            ["first", "second", "third"],
        );
    }

    #[test]
    fn history_fidelity_keeps_an_edited_reply_snapshot() {
        let (mut worker, _events, _inbox, _wa) = worker();
        let edited = wa::Message::text_with_context("Updated fixture reply", {
            let mut proto = wa::ContextInfo::default();
            proto.stanza_id = Some("quoted-fixture".into());
            proto.participant = Some(PEER.into());
            proto.quoted_message = MessageField::some(wa::Message::text("Original fixture"));
            proto
        });
        let raw = {
            let mut proto = wa::Message::default();
            proto.message_context_info = MessageField::some({
                let mut proto = wa::MessageContextInfo::default();
                proto.message_secret = Some(vec![7; 32]);
                proto
            });
            proto.protocol_message = MessageField::some({
                let mut proto = wa::message::ProtocolMessage::default();
                proto.r#type = Some(wa::message::protocol_message::Type::MESSAGE_EDIT);
                proto.key = MessageField::some({
                    let mut proto = wa::MessageKey::default();
                    proto.id = Some("edited-reply".into());
                    proto.remote_jid = Some(PEER.into());
                    proto.from_me = Some(false);
                    proto
                });
                proto.edited_message = MessageField::some(edited);
                proto.timestamp_ms = Some(110_000);
                proto
            });
            proto
        };
        file_history_entries(
            &mut worker,
            vec![history_entry(
                PEER,
                "edited-reply",
                false,
                None,
                raw,
                Vec::new(),
                None,
            )],
        );
        let message = worker
            .archive
            .message(PEER, "edited-reply")
            .unwrap()
            .expect("an edited history row is a message, not disposable protocol traffic");
        assert_eq!(message.content, Content::text("Updated fixture reply"));
        assert_eq!(message.timestamp, 100, "keep the original send time");
        assert!(message.edited);
        let raw = worker.archive.raw(PEER, "edited-reply").unwrap().unwrap();
        let raw = wa::Message::decode_from_slice(&raw).unwrap();
        assert_eq!(classify(&raw), Some(Content::text("Updated fixture reply")));
        assert_eq!(
            raw.message_context_info.as_option().unwrap().message_secret,
            Some(vec![7; 32])
        );
        assert_eq!(message.quoted.unwrap().id, "quoted-fixture");
    }

    #[test]
    fn history_fidelity_keeps_edit_wrappers_and_view_once_restrictions() {
        let edited = {
            let mut proto = wa::Message::default();
            proto.edited_message = MessageField::some({
                let mut proto = wa::message::FutureProofMessage::default();
                proto.message = MessageField::some(wa::Message::text("Updated fixture"));
                proto
            });
            proto
        };
        let protected = {
            let mut proto = wa::Message::default();
            proto.view_once_message_v2 = MessageField::some({
                let mut proto = wa::message::FutureProofMessage::default();
                proto.message = MessageField::some({
                    let mut proto = wa::Message::default();
                    proto.protocol_message = MessageField::some({
                        let mut proto = wa::message::ProtocolMessage::default();
                        proto.r#type = Some(wa::message::protocol_message::Type::MESSAGE_EDIT);
                        proto.key = MessageField::some({
                            let mut proto = wa::MessageKey::default();
                            proto.id = Some("protected".into());
                            proto
                        });
                        proto.edited_message = MessageField::some({
                            let mut proto = wa::Message::default();
                            proto.image_message = MessageField::some({
                                let mut proto = wa::message::ImageMessage::default();
                                proto.caption = Some("Updated fixture caption".into());
                                proto
                            });
                            proto
                        });
                        proto
                    });
                    proto
                });
                proto
            });
            proto
        };
        let (mut worker, _events, _inbox, _wa) = worker();
        file_history_entries(
            &mut worker,
            vec![
                history_entry(PEER, "wrapped", false, None, edited, Vec::new(), None),
                history_entry(PEER, "protected", false, None, protected, Vec::new(), None),
            ],
        );
        let wrapped = worker.archive.message(PEER, "wrapped").unwrap().unwrap();
        assert!(wrapped.edited);
        assert_eq!(wrapped.content, Content::text("Updated fixture"));
        let protected = worker.archive.message(PEER, "protected").unwrap().unwrap();
        assert!(matches!(
            protected.content,
            Content::PhoneOnly {
                view_once: true,
                ..
            }
        ));
        let raw = worker.archive.raw(PEER, "protected").unwrap().unwrap();
        assert!(wa::Message::decode_from_slice(&raw).unwrap().is_view_once());
    }

    #[test]
    fn history_fidelity_replay_keeps_existing_edits_and_revokes() {
        let (mut worker, _events, _inbox, _wa) = worker();
        for (id, content, edited) in [
            ("edited", Content::text("Updated fixture"), true),
            ("revoked", Content::Revoked, false),
        ] {
            let entry = || {
                history_entry(
                    PEER,
                    id,
                    false,
                    None,
                    wa::Message::text("Original fixture"),
                    Vec::new(),
                    None,
                )
            };
            file_history_entries(&mut worker, vec![entry()]);
            worker
                .archive
                .set_content(PEER, id, &content, edited)
                .unwrap();
            let raw = worker.archive.raw(PEER, id).unwrap();
            let mut replay = entry();
            replay.msg_order_id = Some(42);
            file_history_entries(&mut worker, vec![replay]);
            let stored = worker.archive.message(PEER, id).unwrap().unwrap();
            assert_eq!(stored.content, content);
            assert_eq!(stored.edited, edited);
            assert_eq!(stored.history_order, Some(42));
            assert_eq!(worker.archive.raw(PEER, id).unwrap(), raw);
        }
    }

    #[test]
    fn mention_backfill_keeps_edited_bodies_mentions_and_media_paths() {
        let (mut worker, _events, _inbox, _wa) = worker();
        let group = "123-456@g.us";
        let member = "12025550123@s.whatsapp.net";
        let original = || {
            wa::Message::text_with_context("Original @all", {
                let mut proto = wa::ContextInfo::default();
                proto.non_jid_mentions = Some(1);
                proto
            })
        };
        let image = {
            let mut proto = wa::Message::default();
            proto.image_message = MessageField::some({
                let mut proto = wa::message::ImageMessage::default();
                proto.caption = Some("Original @all".into());
                proto.mimetype = Some("image/jpeg".into());
                proto.context_info = MessageField::some({
                    let mut proto = wa::ContextInfo::default();
                    proto.non_jid_mentions = Some(1);
                    proto
                });
                proto
            });
            proto
        };
        file_history_entries_for(
            &mut worker,
            group,
            vec![
                history_entry(
                    group,
                    "plain",
                    false,
                    Some(member),
                    original(),
                    vec![],
                    None,
                ),
                history_entry(
                    group,
                    "edited-text",
                    false,
                    Some(member),
                    original(),
                    vec![],
                    None,
                ),
                history_entry(
                    group,
                    "edited-image",
                    false,
                    Some(member),
                    image,
                    vec![],
                    None,
                ),
            ],
        );
        let plain = worker.archive.message(group, "plain").unwrap().unwrap();
        worker
            .archive
            .set_derived(group, "plain", &plain.content, &[], None, false)
            .unwrap();
        let current_mentions = [MentionRef {
            user: "12025550123".into(),
            id: member.into(),
        }];
        worker
            .archive
            .set_edited_content(
                group,
                "edited-text",
                &Content::text("Corrected @12025550123"),
                &current_mentions,
            )
            .unwrap();
        let path = std::path::Path::new("/synthetic/edited-image.jpg");
        worker
            .archive
            .set_media_path(group, "edited-image", path)
            .unwrap();
        let mut edited_image = worker
            .archive
            .message(group, "edited-image")
            .unwrap()
            .unwrap()
            .content;
        if let Content::Image { caption, .. } = &mut edited_image {
            *caption = Some("Corrected caption".into());
        } else {
            panic!("image fixture was not classified");
        }
        worker
            .archive
            .set_edited_content(group, "edited-image", &edited_image, &current_mentions)
            .unwrap();
        worker.archive.set_meta("derived", "3").unwrap();

        worker.backfill();

        let plain = worker.archive.message(group, "plain").unwrap().unwrap();
        assert_eq!(plain.mentions, vec![crate::mentions::everyone_ref()]);
        let text = worker
            .archive
            .message(group, "edited-text")
            .unwrap()
            .unwrap();
        assert_eq!(text.content, Content::text("Corrected @12025550123"));
        assert_eq!(text.mentions, current_mentions);
        assert!(text.edited);
        let image = worker
            .archive
            .message(group, "edited-image")
            .unwrap()
            .unwrap();
        assert_eq!(image.content, edited_image);
        assert_eq!(image.mentions, current_mentions);
        assert!(image.edited);
        assert_eq!(
            worker.archive.meta("derived").unwrap().as_deref(),
            Some("4")
        );
    }

    #[test]
    fn live_edits_replace_everyone_and_participant_mentions() {
        let (mut worker, _events, _inbox, _wa) = worker();
        let group = "123-456@g.us";
        let sender = "12025550123@s.whatsapp.net";
        let member = "12025550999@s.whatsapp.net";
        file_history_entries_for(
            &mut worker,
            group,
            vec![history_entry(
                group,
                "target",
                false,
                Some(sender),
                wa::Message::text("Original"),
                vec![],
                None,
            )],
        );
        let ingest_edit = |worker: &mut Worker, body: wa::Message| {
            let edit = Arc::new({
                let mut proto = wa::Message::default();
                proto.protocol_message = MessageField::some({
                    let mut proto = wa::message::ProtocolMessage::default();
                    proto.r#type = Some(wa::message::protocol_message::Type::MESSAGE_EDIT);
                    proto.key = MessageField::some({
                        let mut proto = wa::MessageKey::default();
                        proto.id = Some("target".into());
                        proto
                    });
                    proto.edited_message = MessageField::some(body);
                    proto
                });
                proto
            });
            let info = MessageInfo {
                id: "edit-event".into(),
                source: MessageSource {
                    chat: group.parse().unwrap(),
                    sender: sender.parse().unwrap(),
                    is_group: true,
                    ..Default::default()
                },
                timestamp: whatsapp_rust::wacore::time::from_secs(120).unwrap(),
                ..Default::default()
            };
            worker.ingest(&edit, &info);
        };
        ingest_edit(&mut worker, {
            let mut proto = wa::Message::default();
            proto.ephemeral_message = MessageField::some({
                let mut proto = wa::message::FutureProofMessage::default();
                proto.message = MessageField::some(wa::Message::text_with_context(
                    "Edited @all @12025550999",
                    {
                        let mut proto = wa::ContextInfo::default();
                        proto.non_jid_mentions = Some(1);
                        proto.mentioned_jid = vec![member.into()];
                        proto
                    },
                ));
                proto
            });
            proto
        });
        let updated = worker.archive.message(group, "target").unwrap().unwrap();
        assert_eq!(updated.content, Content::text("Edited @all @12025550999"));
        assert_eq!(
            updated.mentions,
            vec![
                MentionRef {
                    user: "12025550999".into(),
                    id: member.into(),
                },
                crate::mentions::everyone_ref(),
            ]
        );
        ingest_edit(&mut worker, wa::Message::text("No mentions now"));
        let updated = worker.archive.message(group, "target").unwrap().unwrap();
        assert_eq!(updated.content, Content::text("No mentions now"));
        assert!(updated.mentions.is_empty());
        assert!(updated.edited);

        let original_image = {
            let mut proto = wa::Message::default();
            proto.image_message = MessageField::some({
                let mut proto = wa::message::ImageMessage::default();
                proto.caption = Some("Original caption".into());
                proto.mimetype = Some("image/jpeg".into());
                proto
            });
            proto
        };
        file_history_entries_for(
            &mut worker,
            group,
            vec![history_entry(
                group,
                "image-target",
                false,
                Some(sender),
                original_image,
                vec![],
                None,
            )],
        );
        let path = std::path::Path::new("/synthetic/live-edit.jpg");
        worker
            .archive
            .set_media_path(group, "image-target", path)
            .unwrap();
        let edited_image = Arc::new({
            let mut proto = wa::Message::default();
            proto.protocol_message = MessageField::some({
                let mut proto = wa::message::ProtocolMessage::default();
                proto.r#type = Some(wa::message::protocol_message::Type::MESSAGE_EDIT);
                proto.key = MessageField::some({
                    let mut proto = wa::MessageKey::default();
                    proto.id = Some("image-target".into());
                    proto
                });
                proto.edited_message = MessageField::some({
                    let mut proto = wa::Message::default();
                    proto.image_message = MessageField::some({
                        let mut proto = wa::message::ImageMessage::default();
                        proto.caption = Some("Caption @all".into());
                        proto.mimetype = Some("image/jpeg".into());
                        proto.context_info = MessageField::some({
                            let mut proto = wa::ContextInfo::default();
                            proto.non_jid_mentions = Some(1);
                            proto
                        });
                        proto
                    });
                    proto
                });
                proto
            });
            proto
        });
        let info = MessageInfo {
            id: "image-edit-event".into(),
            source: MessageSource {
                chat: group.parse().unwrap(),
                sender: sender.parse().unwrap(),
                is_group: true,
                ..Default::default()
            },
            timestamp: whatsapp_rust::wacore::time::from_secs(121).unwrap(),
            ..Default::default()
        };
        worker.ingest(&edited_image, &info);
        let updated = worker
            .archive
            .message(group, "image-target")
            .unwrap()
            .unwrap();
        assert_eq!(updated.mentions, vec![crate::mentions::everyone_ref()]);
        match updated.content {
            Content::Image { caption, media, .. } => {
                assert_eq!(caption.as_deref(), Some("Caption @all"));
                assert_eq!(media.path.as_deref(), Some(path));
            }
            other => panic!("unexpected edited image: {other:?}"),
        }
    }

    #[test]
    fn history_fidelity_rekeys_an_edited_history_snapshot_to_its_original() {
        let raw = {
            let mut proto = wa::Message::default();
            proto.protocol_message = MessageField::some({
                let mut proto = wa::message::ProtocolMessage::default();
                proto.r#type = Some(wa::message::protocol_message::Type::MESSAGE_EDIT);
                proto.key = MessageField::some({
                    let mut proto = wa::MessageKey::default();
                    proto.id = Some("original-id".into());
                    proto
                });
                proto.edited_message = MessageField::some(wa::Message::text("Updated fixture"));
                proto
            });
            proto
        };
        let parsed = parse_conversation({
            let mut proto = wa::Conversation::default();
            proto.id = PEER.into();
            proto.messages = vec![history_entry(
                PEER,
                "separate-edit-id",
                false,
                None,
                raw,
                Vec::new(),
                None,
            )];
            proto
        });
        assert_eq!(parsed.messages.len(), 1);
        assert_eq!(parsed.messages[0].id, "original-id");
        assert_eq!(parsed.messages[0].content, Content::text("Updated fixture"));
        assert!(parsed.messages[0].edited);
        assert_eq!(parsed.messages[0].timestamp, 100);
    }

    /// An edit WhatsApp sealed with the original's secret, as history carries
    /// one made before linking (#582). The author edits, so the author is
    /// both the original's sender and the editor.
    fn sealed_edit(target: &str, author: &str, secret: &[u8], text: &str) -> wa::Message {
        use whatsapp_rust::wacore::message_edit::{MessageEditContext, encrypt_message_edit};
        let inner = {
            let mut proto = wa::Message::default();
            proto.protocol_message = MessageField::some({
                let mut proto = wa::message::ProtocolMessage::default();
                proto.r#type = Some(wa::message::protocol_message::Type::MESSAGE_EDIT);
                proto.key = MessageField::some({
                    let mut proto = wa::MessageKey::default();
                    proto.id = Some(target.into());
                    proto
                });
                proto.edited_message = MessageField::some(wa::Message::text(text));
                proto
            });
            proto
        };
        let (payload, iv) = encrypt_message_edit(
            &inner,
            secret,
            &MessageEditContext {
                original_msg_id: target,
                original_sender_jid: author,
                editor_jid: author,
            },
        )
        .unwrap();
        let mut proto = wa::Message::default();
        proto.secret_encrypted_message = MessageField::some({
            let mut proto = wa::message::SecretEncryptedMessage::default();
            proto.secret_enc_type =
                Some(wa::message::secret_encrypted_message::SecretEncType::MESSAGE_EDIT);
            proto.target_message_key = MessageField::some({
                let mut proto = wa::MessageKey::default();
                proto.id = Some(target.into());
                proto.remote_jid = Some(author.into());
                proto
            });
            proto.enc_payload = Some(payload);
            proto.enc_iv = Some(iv.to_vec());
            proto
        });
        proto
    }

    #[test]
    fn history_parses_a_sealed_edit_under_its_original() {
        let secret = [7u8; 32];
        let parsed = parse_conversation({
            let mut proto = wa::Conversation::default();
            proto.id = PEER.into();
            proto.messages = vec![history_entry(
                PEER,
                "edit-envelope",
                false,
                None,
                sealed_edit("original-id", PEER, &secret, "Edited fixture"),
                Vec::new(),
                None,
            )];
            proto
        });
        assert_eq!(parsed.diagnostics.unclassified, 0);
        assert_eq!(parsed.diagnostics.encrypted_edits, 1);
        assert_eq!(parsed.messages.len(), 1);
        assert_eq!(parsed.messages[0].id, "original-id");
        assert!(parsed.messages[0].edited);
        assert!(parsed.messages[0].sealed_edit.is_some());
        assert!(parsed.messages[0].poll_secret.is_none());
    }

    #[test]
    fn history_files_a_sealed_edit_without_its_original() {
        let (mut worker, _events, _inbox, _wa) = worker();
        let secret = vec![9u8; 32];
        // The phone's entry for the edited message names its secret.
        file_history_entries(
            &mut worker,
            vec![history_entry(
                PEER,
                "edit-envelope",
                false,
                None,
                sealed_edit("original-id", PEER, &secret, "Edited fixture"),
                Vec::new(),
                Some(secret.clone()),
            )],
        );
        let row = worker
            .archive
            .message(PEER, "original-id")
            .unwrap()
            .expect("the edited message is filed under its own id");
        assert_eq!(row.content, Content::text("Edited fixture"));
        assert!(row.edited);
        assert_eq!(row.timestamp, 100);
        assert!(
            worker
                .archive
                .message(PEER, "edit-envelope")
                .unwrap()
                .is_none()
        );
        // Its secret stays with it, so later reactions to it can be opened.
        let raw = worker.archive.raw(PEER, "original-id").unwrap().unwrap();
        assert_eq!(message_secret_from_raw(&raw), Some(secret));
    }

    #[test]
    fn history_applies_a_sealed_edit_with_the_archived_originals_secret() {
        let (mut worker, _events, _inbox, _wa) = worker();
        let secret = vec![5u8; 32];
        file_history_entries(
            &mut worker,
            vec![history_entry(
                PEER,
                "original-id",
                false,
                None,
                wa::Message::text("Before editing"),
                Vec::new(),
                Some(secret.clone()),
            )],
        );
        // A later chunk carries the edit alone, without a secret of its own.
        file_history_entries(
            &mut worker,
            vec![history_entry(
                PEER,
                "edit-envelope",
                false,
                None,
                sealed_edit("original-id", PEER, &secret, "Edited fixture"),
                Vec::new(),
                None,
            )],
        );
        let row = worker
            .archive
            .message(PEER, "original-id")
            .unwrap()
            .unwrap();
        assert_eq!(row.content, Content::text("Edited fixture"));
        assert!(row.edited);
        // A replay of the original does not undo it.
        file_history_entries(
            &mut worker,
            vec![history_entry(
                PEER,
                "original-id",
                false,
                None,
                wa::Message::text("Before editing"),
                Vec::new(),
                Some(secret),
            )],
        );
        let row = worker
            .archive
            .message(PEER, "original-id")
            .unwrap()
            .unwrap();
        assert_eq!(row.content, Content::text("Edited fixture"));
    }

    #[test]
    fn history_leaves_a_sealed_edit_it_cannot_open() {
        let (mut worker, _events, _inbox, _wa) = worker();
        file_history_entries(
            &mut worker,
            vec![history_entry(
                PEER,
                "original-id",
                false,
                None,
                wa::Message::text("Before editing"),
                Vec::new(),
                None,
            )],
        );
        // Sealed with a secret nobody here knows.
        file_history_entries(
            &mut worker,
            vec![history_entry(
                PEER,
                "edit-envelope",
                false,
                None,
                sealed_edit("original-id", PEER, &[3u8; 32], "Edited fixture"),
                Vec::new(),
                Some(vec![4u8; 32]),
            )],
        );
        let row = worker
            .archive
            .message(PEER, "original-id")
            .unwrap()
            .unwrap();
        assert_eq!(row.content, Content::text("Before editing"));
        assert!(!row.edited);
    }

    // WA Web 2.3000.1045368834, whatspec 1a441f0329c941fcdb238490a6c604550d8a9939:
    // WAWebParseHistorySyncProtocolMessageEditProto keeps base metadata, parses
    // editedMessage, then replaces id with protocol.key. The outer key identifies
    // the latest edit; it need not identify a separate event or an archived row.
    #[test]
    fn history_fidelity_restores_rekeyed_reply_without_an_original() {
        for original_present in [false, true] {
            let (mut worker, _events, _inbox, _wa) = worker();
            if original_present {
                file_history_entries(
                    &mut worker,
                    vec![history_entry(
                        PEER,
                        "original-reply",
                        false,
                        None,
                        wa::Message::text("Before editing"),
                        Vec::new(),
                        None,
                    )],
                );
            }
            let snapshot = || {
                let body = wa::Message::text_with_context("Updated fixture reply", {
                    let mut proto = wa::ContextInfo::default();
                    proto.stanza_id = Some("quoted-fixture".into());
                    proto.participant = Some(ME.into());
                    proto.quoted_message = MessageField::some(wa::Message::text("Earlier fixture"));
                    proto
                });
                let mut entry = history_entry(
                    PEER,
                    "latest-edit-id",
                    false,
                    None,
                    {
                        let mut proto = wa::Message::default();
                        proto.protocol_message = MessageField::some({
                            let mut proto = wa::message::ProtocolMessage::default();
                            proto.r#type = Some(wa::message::protocol_message::Type::MESSAGE_EDIT);
                            proto.key = MessageField::some({
                                let mut proto = wa::MessageKey::default();
                                proto.id = Some("original-reply".into());
                                // Inner keys may use the sender's perspective.
                                proto.from_me = Some(true);
                                proto
                            });
                            proto.edited_message = MessageField::some(body);
                            proto.timestamp_ms = Some(115_000);
                            proto
                        });
                        proto
                    },
                    Vec::new(),
                    Some(vec![7; 32]),
                );
                entry.msg_order_id = Some(5);
                entry
            };
            // Replaying the same snapshot must also keep a single original row.
            for _ in 0..2 {
                file_history_entries(&mut worker, vec![snapshot()]);
                let messages = worker.archive.messages(PEER, None, 10).unwrap();
                assert_eq!(messages.len(), 1);
                let row = &messages[0];
                assert_eq!(row.id, "original-reply");
                assert_eq!(
                    row.timestamp, 100,
                    "history envelope supplies the send time"
                );
                assert_eq!(row.history_order, Some(5));
                assert_eq!(row.sender, PEER);
                assert!(!row.from_me);
                assert!(row.edited);
                assert_eq!(row.content, Content::text("Updated fixture reply"));
                assert_eq!(row.quoted.as_ref().unwrap().id, "quoted-fixture");
                let raw = worker.archive.raw(PEER, &row.id).unwrap().unwrap();
                assert_eq!(
                    classify(&wa::Message::decode_from_slice(&raw).unwrap()),
                    Some(row.content.clone())
                );
                assert_eq!(message_secret_from_raw(&raw), Some(vec![7; 32]));
                assert!(
                    worker
                        .archive
                        .message(PEER, "latest-edit-id")
                        .unwrap()
                        .is_none()
                );
            }
            worker
                .archive
                .delete_message(PEER, "original-reply")
                .unwrap();
            file_history_entries(&mut worker, vec![snapshot()]);
            assert!(
                worker.archive.messages(PEER, None, 10).unwrap().is_empty(),
                "a rekeyed snapshot must respect the original message's deletion"
            );
        }
    }

    #[test]
    fn history_fidelity_rejects_edit_snapshots_without_a_target_or_body() {
        for (target, body) in [
            (None, Some(wa::Message::text("Fixture"))),
            (Some(""), Some(wa::Message::text("Fixture"))),
            (Some("original"), None),
        ] {
            let parsed = parse_conversation({
                let mut proto = wa::Conversation::default();
                proto.id = PEER.into();
                proto.messages = vec![history_entry(
                    PEER,
                    "edit-id",
                    false,
                    None,
                    {
                        let mut proto = wa::Message::default();
                        proto.protocol_message = MessageField::some({
                            let mut proto = wa::message::ProtocolMessage::default();
                            proto.r#type = Some(wa::message::protocol_message::Type::MESSAGE_EDIT);
                            proto.key = MessageField::some({
                                let mut proto = wa::MessageKey::default();
                                proto.id = target.map(str::to_owned);
                                proto
                            });
                            proto.edited_message = body.into();
                            proto
                        });
                        proto
                    },
                    Vec::new(),
                    None,
                )];
                proto
            });
            assert!(parsed.messages.is_empty());
            assert_eq!(parsed.diagnostics.invalid_edits, 1);
        }
    }

    /// The info panel's listing follows the archive: a picture, a document
    /// or a linked text filed, imported, deleted or cleared says so, once per
    /// change; a page, a receipt, a reaction, plain text and a duplicate
    /// delivery do not.
    #[test]
    fn listable_archive_changes_are_reported_once_and_others_are_not() {
        use crate::model::{Content, Media, Message};
        let (mut worker, events, _, _) = worker();
        worker.archive.ensure_chat(PEER, "Fixture").unwrap();
        let changes = |events: &std::sync::mpsc::Receiver<Event>| -> Vec<String> {
            events
                .try_iter()
                .filter_map(|event| match event {
                    Event::ChatSharedChanged(chat) => Some(chat),
                    _ => None,
                })
                .collect()
        };
        let photo = Message {
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
                    state: Default::default(),
                },
            },
            ..own_message("photo", 100)
        };
        worker.store_message(photo.clone(), None, None);
        assert_eq!(
            changes(&events),
            [PEER],
            "the first picture in an empty chat"
        );
        worker.store_message(photo, None, None);
        assert!(
            changes(&events).is_empty(),
            "the same picture delivered again"
        );
        worker.store_message(own_message("plain", 101), None, None);
        assert!(changes(&events).is_empty(), "text without a link");
        worker.send_page(&PEER.to_owned(), None);
        worker.emit_message(PEER, "photo");
        worker.store_plain_reaction(
            PEER,
            PEER,
            false,
            &{
                let mut proto = wa::message::ReactionMessage::default();
                proto.key = MessageField::some({
                    let mut proto = wa::MessageKey::default();
                    proto.id = Some("photo".into());
                    proto
                });
                proto.text = Some("❤️".into());
                proto
            },
            102,
        );
        assert!(changes(&events).is_empty(), "a page, a receipt, a reaction");
        // A document imported from the phone's history.
        let history = parse_conversation({
            let mut proto = wa::Conversation::default();
            proto.id = PEER.into();
            proto.messages = vec![history_entry(
                PEER,
                "notes",
                false,
                None,
                {
                    let mut proto = wa::Message::default();
                    proto.document_message = MessageField::some({
                        let mut proto = wa::message::DocumentMessage::default();
                        proto.mimetype = Some("application/pdf".into());
                        proto.file_name = Some("Notes.pdf".into());
                        proto.file_length = Some(10);
                        proto
                    });
                    proto
                },
                Vec::new(),
                None,
            )];
            proto
        });
        worker.apply_history(
            ParsedHistory {
                chats: vec![history],
                push_names: Vec::new(),
                lids: Vec::new(),
                stickers: Vec::new(),
            },
            false,
        );
        assert_eq!(changes(&events), [PEER], "an imported document");
        // A deletion WhatsApp accepted, for this account.
        worker.me_pn = Some("15550001111".into());
        worker.delete_message_here(PEER, "notes");
        assert_eq!(changes(&events), [PEER], "a deleted document");
        worker.delete_message_here(PEER, "plain");
        assert!(changes(&events).is_empty(), "a deleted plain text");
        assert!(worker.empty_chat(PEER, i64::MAX, false));
        assert_eq!(changes(&events), [PEER], "a cleared chat");
    }

    #[test]
    fn history_reads_aggregated_reactions_from_grouping_key() {
        let parsed = parse_conversation({
            let mut proto = wa::Conversation::default();
            proto.id = PEER.into();
            proto.messages = vec![history_entry(
                PEER,
                "photo",
                false,
                None,
                {
                    let mut proto = wa::Message::default();
                    proto.conversation = Some("caption".into());
                    proto
                },
                vec![{
                    let mut proto = wa::Reaction::default();
                    proto.key = MessageField::some({
                        let mut proto = wa::MessageKey::default();
                        proto.from_me = Some(false);
                        proto.participant = Some(PEER.into());
                        proto
                    });
                    proto.grouping_key = Some("🏆".into());
                    proto
                }],
                None,
            )];
            proto
        });
        assert_eq!(parsed.messages[0].reactions.len(), 1);
        assert_eq!(parsed.messages[0].reactions[0].2, "🏆");
        assert!(!parsed.messages[0].reactions[0].1);
    }

    #[test]
    fn live_grouping_key_reaction_from_another_sender_is_stored() {
        let (mut worker, _events, _inbox, _wa) = worker();
        worker.archive.ensure_chat(PEER, "Ada").unwrap();
        worker
            .archive
            .insert_message(&incoming("photo", 10), None)
            .unwrap();
        let raw = {
            let mut proto = wa::Message::default();
            proto.reaction_message = MessageField::some({
                let mut proto = wa::message::ReactionMessage::default();
                proto.key = MessageField::some({
                    let mut proto = wa::MessageKey::default();
                    proto.remote_jid = Some(PEER.into());
                    proto.from_me = Some(false);
                    proto.id = Some("photo".into());
                    proto
                });
                proto.grouping_key = Some("🏆".into());
                proto
            });
            proto
        };
        let info = MessageInfo {
            source: MessageSource {
                chat: PEER.parse().unwrap(),
                sender: PEER.parse().unwrap(),
                ..Default::default()
            },
            timestamp: whatsapp_rust::wacore::time::from_secs(20).unwrap(),
            ..Default::default()
        };
        worker.ingest(&Arc::new(raw), &info);
        let stored = worker
            .archive
            .message(PEER, "photo")
            .unwrap()
            .expect("parent");
        assert_eq!(stored.reactions.len(), 1);
        assert_eq!(stored.reactions[0].emoji, "🏆");
        assert!(!stored.reactions[0].from_me);
        assert_eq!(stored.reactions[0].sender, PEER);
    }

    #[test]
    fn a_reaction_to_our_message_raises_the_chat() {
        let (mut worker, _events, _inbox, _wa) = worker();
        worker.archive.ensure_chat(PEER, "Ada").unwrap();
        worker
            .archive
            .insert_message(&own_message("ours", 10), None)
            .unwrap();
        worker
            .archive
            .insert_message(&incoming("theirs", 11), None)
            .unwrap();
        let activity = |worker: &Worker| worker.archive.chat(PEER).unwrap().unwrap().last_activity;
        let before = activity(&worker);
        let react = |worker: &mut Worker, target: &str, emoji: &str, at: i64| {
            let reaction = {
                let mut proto = wa::message::ReactionMessage::default();
                proto.key = MessageField::some({
                    let mut proto = wa::MessageKey::default();
                    proto.remote_jid = Some(PEER.into());
                    proto.id = Some(target.into());
                    proto
                });
                proto.text = Some(emoji.into());
                proto.sender_timestamp_ms = Some(at * 1000);
                proto
            };
            worker.store_plain_reaction(PEER, PEER, false, &reaction, at * 1000);
        };

        react(&mut worker, "theirs", "👍", 500);
        assert_eq!(activity(&worker), before, "only reactions to our messages");
        react(&mut worker, "ours", "", 600);
        assert_eq!(activity(&worker), before, "a removal raises nothing");
        react(&mut worker, "ours", "❤️", 700);
        assert_eq!(activity(&worker), 700);
        assert_eq!(
            worker
                .archive
                .message(PEER, "ours")
                .unwrap()
                .unwrap()
                .reactions[0]
                .emoji,
            "❤️"
        );
    }

    #[test]
    fn live_encrypted_custom_reaction_from_another_sender_is_stored() {
        let secret = [0x42u8; 32];
        let reactor = "12025550999@s.whatsapp.net";
        let (payload, iv) = whatsapp_rust::wacore::reaction::encrypt_reaction_with_secret(
            "🏆",
            1_700_000_000_123,
            &secret,
            "photo",
            PEER,
            reactor,
        )
        .expect("encrypt");
        let parent_raw = {
            let mut proto = wa::Message::default();
            proto.conversation = Some("caption".into());
            proto.message_context_info = MessageField::some({
                let mut proto = wa::MessageContextInfo::default();
                proto.message_secret = Some(secret.to_vec());
                proto
            });
            proto
        };
        let (mut worker, _events, _inbox, _wa) = worker();
        worker.archive.ensure_chat(PEER, "Ada").unwrap();
        worker
            .archive
            .insert_message(&incoming("photo", 10), Some(&parent_raw.encode_to_vec()))
            .unwrap();
        let raw = {
            let mut proto = wa::Message::default();
            proto.enc_reaction_message = MessageField::some({
                let mut proto = wa::message::EncReactionMessage::default();
                proto.target_message_key = MessageField::some({
                    let mut proto = wa::MessageKey::default();
                    proto.remote_jid = Some(PEER.into());
                    proto.from_me = Some(false);
                    proto.id = Some("photo".into());
                    proto.participant = Some(PEER.into());
                    proto
                });
                proto.enc_payload = Some(payload);
                proto.enc_iv = Some(iv.to_vec());
                proto
            });
            proto
        };
        let info = MessageInfo {
            source: MessageSource {
                chat: PEER.parse().unwrap(),
                sender: reactor.parse().unwrap(),
                ..Default::default()
            },
            timestamp: whatsapp_rust::wacore::time::from_secs(20).unwrap(),
            ..Default::default()
        };
        worker.ingest(&Arc::new(raw), &info);
        let stored = worker
            .archive
            .message(PEER, "photo")
            .unwrap()
            .expect("parent");
        assert_eq!(stored.reactions.len(), 1);
        assert_eq!(stored.reactions[0].emoji, "🏆");
        assert_eq!(stored.reactions[0].sender, reactor);
        assert!(!stored.reactions[0].from_me);
    }

    #[test]
    fn protocol_timer_badge_follows_enable_disable_and_ignores_stale_updates() {
        let (mut worker, events, _inbox, _wa) = worker();
        // What the archive holds, and what the badge shows: a timer the phone
        // turned off is stored as a zero and read as no timer at all.
        for (expiration, setting_time, envelope_time, stored, badge) in [
            (86_400, None, 200, Some(86_400), Some(86_400)),
            (0, None, 300, Some(0), None),
            (604_800, Some(250), 400, Some(0), None),
        ] {
            let raw = {
                let mut proto = wa::Message::default();
                proto.protocol_message = MessageField::some({
                    let mut proto = wa::message::ProtocolMessage::default();
                    proto.r#type = Some(wa::message::protocol_message::Type::EPHEMERAL_SETTING);
                    proto.ephemeral_expiration = Some(expiration);
                    proto.ephemeral_setting_timestamp = setting_time;
                    proto
                });
                proto
            };
            let info = MessageInfo {
                source: MessageSource {
                    chat: PEER.parse().unwrap(),
                    sender: PEER.parse().unwrap(),
                    ..Default::default()
                },
                timestamp: whatsapp_rust::wacore::time::from_secs(envelope_time).unwrap(),
                ..Default::default()
            };
            worker.ingest(&Arc::new(raw), &info);
            let chat = worker.archive.chat(PEER).unwrap().unwrap();
            assert_eq!(chat.ephemeral_expiration, stored);
            assert_eq!(chat.disappearing_timer(), badge);
            // What an outgoing message carries: a zero timer adds nothing.
            assert_eq!(worker.ephemeral_expiration(PEER), badge);
        }
        let badges: Vec<_> = events
            .try_iter()
            .filter_map(|event| match event {
                Event::ChatUpdated(chat) if chat.id == PEER => Some(chat.disappearing_timer()),
                _ => None,
            })
            .collect();
        assert!(badges.contains(&Some(86_400)));
        assert_eq!(badges.last(), Some(&None));
    }

    #[tokio::test]
    async fn group_timer_updates_work_before_history_and_keep_disable_versions() {
        let (mut worker, _events, _inbox, _wa) = worker();
        let group = "123-456@g.us";
        for (expiration, timestamp, stored, badge) in [
            (86_400, 200, Some(86_400), Some(86_400)),
            (0, 300, Some(0), None),
            (604_800, 250, Some(0), None),
        ] {
            let update = wa_events::GroupUpdate::builder()
                .group_jid(group.parse().unwrap())
                .timestamp(whatsapp_rust::wacore::time::from_secs(timestamp).unwrap())
                .is_lid_addressing_mode(false)
                .action(Box::new(
                    whatsapp_rust::wacore::stanza::groups::GroupNotificationAction::Ephemeral {
                        expiration,
                        trigger: None,
                    },
                ))
                .build();
            worker
                .handle_wa_event(Arc::new(wa_events::Event::GroupUpdate(update)))
                .await;
            let chat = worker.archive.chat(group).unwrap().unwrap();
            assert_eq!(chat.ephemeral_expiration, stored, "stored at {timestamp}");
            assert_eq!(chat.disappearing_timer(), badge, "badge at {timestamp}");
        }
    }

    #[tokio::test]
    async fn default_timer_notifications_never_rewrite_existing_chat_timers() {
        let (mut worker, _events, _inbox, _wa) = worker();
        worker.ensure_chat(PEER, None);
        worker.archive.set_ephemeral(PEER, 604_800, 100).unwrap();
        for (from, duration, timestamp) in [
            (PEER, 86_400, 200),
            (ME, 86_400, 200),
            (ME, 0, 300),
            (ME, 604_800, 250),
        ] {
            let update = wa_events::DisappearingModeChanged::builder()
                .from(from.parse().unwrap())
                .duration(duration)
                .setting_timestamp(whatsapp_rust::wacore::time::from_secs(timestamp).unwrap())
                .build();
            worker
                .handle_wa_event(Arc::new(wa_events::Event::DisappearingModeChanged(update)))
                .await;
        }
        assert_eq!(worker.ephemeral_expiration(PEER), Some(604_800));
        assert_eq!(worker.default_ephemeral_expiration(), Some(0));
        assert!(worker.archive.chat(ME).unwrap().is_none());
    }

    #[tokio::test]
    async fn own_typing_is_hidden_in_self_direct_and_group_chats() {
        let (mut worker, events, _inbox, _wa) = worker();
        let device = ME.replacen('@', ":2@", 1);
        let own_lid = "9000001@lid";
        worker.me_lid = Some(own_lid.into());
        for (chat, sender) in [ME, PEER, "123-456@g.us"]
            .into_iter()
            .flat_map(|chat| [ME, device.as_str(), own_lid, PEER].map(|sender| (chat, sender)))
        {
            let presence = wa_events::ChatPresenceUpdate::builder()
                .source(MessageSource {
                    chat: chat.parse().unwrap(),
                    sender: sender.parse().unwrap(),
                    is_group: chat.ends_with("@g.us"),
                    ..Default::default()
                })
                .state(ChatActivity::Typing)
                .build();
            worker
                .handle_wa_event(Arc::new(wa_events::Event::ChatPresence(presence)))
                .await;
        }
        let senders: Vec<_> = events
            .try_iter()
            .filter_map(|event| match event {
                Event::Typing { sender, .. } => Some(sender),
                _ => None,
            })
            .collect();
        assert_eq!(senders, [PEER, PEER, PEER]);
    }

    #[test]
    fn partial_group_history_receipts_do_not_override_the_phone_aggregate() {
        use wa::web_message_info::Status;
        let parsed = |chat: &str, status| {
            parse_conversation({
                let mut proto = wa::Conversation::default();
                proto.id = chat.into();
                proto.messages = vec![{
                    let mut proto = wa::HistorySyncMsg::default();
                    proto.message = MessageField::some({
                        let mut proto = wa::WebMessageInfo::default();
                        proto.key = MessageField::some({
                            let mut proto = wa::MessageKey::default();
                            proto.id = Some("history".into());
                            proto.from_me = Some(true);
                            proto
                        });
                        proto.message = MessageField::some({
                            let mut proto = wa::Message::default();
                            proto.conversation = Some("hello".into());
                            proto
                        });
                        proto.status = Some(status);
                        proto.user_receipt = vec![{
                            let mut proto = wa::UserReceipt::default();
                            proto.user_jid = PEER.into();
                            proto.read_timestamp = Some(123);
                            proto
                        }];
                        proto
                    });
                    proto
                }];
                proto
            })
        };
        assert_eq!(
            parsed("123-456@g.us", Status::SERVER_ACK).messages[0].status,
            Delivery::Sent
        );
        assert_eq!(
            parsed("123-456@g.us", Status::DELIVERY_ACK).messages[0].status,
            Delivery::Delivered
        );
        assert_eq!(
            parsed("123-456@g.us", Status::READ).messages[0].status,
            Delivery::Read
        );
        assert_eq!(
            parsed(PEER, Status::SERVER_ACK).messages[0].status,
            Delivery::Read
        );
    }

    #[test]
    fn history_keeps_their_voice_message_played_on_the_phone() {
        use wa::web_message_info::Status;
        let parsed = |status| {
            parse_conversation({
                let mut proto = wa::Conversation::default();
                proto.id = PEER.into();
                proto.messages = vec![{
                    let mut proto = wa::HistorySyncMsg::default();
                    proto.message = MessageField::some({
                        let mut proto = wa::WebMessageInfo::default();
                        proto.key = MessageField::some({
                            let mut proto = wa::MessageKey::default();
                            proto.id = Some("voice".into());
                            proto.from_me = Some(false);
                            proto
                        });
                        proto.message = MessageField::some({
                            let mut proto = wa::Message::default();
                            proto.audio_message = MessageField::some({
                                let mut proto = wa::message::AudioMessage::default();
                                proto.ptt = Some(true);
                                proto
                            });
                            proto
                        });
                        proto.status = Some(status);
                        proto
                    });
                    proto
                }];
                proto
            })
            .messages[0]
                .status
        };
        assert_eq!(parsed(Status::PLAYED), Delivery::Played);
        assert_eq!(parsed(Status::READ), Delivery::None);
        assert_eq!(parsed(Status::DELIVERY_ACK), Delivery::None);
    }

    #[test]
    fn history_tells_the_transcript_of_a_voice_message_played_since() {
        use wa::web_message_info::Status;
        let (mut worker, events, _inbox, _wa) = worker();
        let entry = |status| {
            let mut entry = history_entry(
                PEER,
                "voice",
                false,
                None,
                {
                    let mut proto = wa::Message::default();
                    proto.audio_message = MessageField::some({
                        let mut proto = wa::message::AudioMessage::default();
                        proto.ptt = Some(true);
                        proto
                    });
                    proto
                },
                Vec::new(),
                None,
            );
            entry.message.as_option_mut().unwrap().status = Some(status);
            entry
        };
        file_history_entries(&mut worker, vec![entry(Status::DELIVERY_ACK)]);
        while events.try_recv().is_ok() {}
        // A replay that says the phone played it updates the row on screen.
        file_history_entries(&mut worker, vec![entry(Status::PLAYED)]);
        let told = std::iter::from_fn(|| events.try_recv().ok()).any(|event| {
            matches!(event, Event::MessageUpdated(message)
                if message.id == "voice" && message.status == Delivery::Played)
        });
        assert!(
            told,
            "the played voice message was not sent to the transcript"
        );
    }

    #[test]
    fn history_receipts_date_direct_ticks_and_fill_group_message_info() {
        use wa::web_message_info::Status;
        let conversation = |chat: &str, status| {
            parse_conversation({
                let mut proto = wa::Conversation::default();
                proto.id = chat.into();
                proto.messages = vec![{
                    let mut proto = wa::HistorySyncMsg::default();
                    proto.message = MessageField::some({
                        let mut proto = wa::WebMessageInfo::default();
                        proto.key = MessageField::some({
                            let mut proto = wa::MessageKey::default();
                            proto.id = Some("history".into());
                            proto.from_me = Some(true);
                            proto
                        });
                        proto.message = MessageField::some({
                            let mut proto = wa::Message::default();
                            proto.conversation = Some("hello".into());
                            proto
                        });
                        proto.message_timestamp = Some(90);
                        proto.status = Some(status);
                        proto.user_receipt = vec![{
                            let mut proto = wa::UserReceipt::default();
                            proto.user_jid = PEER.into();
                            proto.receipt_timestamp = Some(100);
                            proto.read_timestamp = Some(123);
                            proto
                        }];
                        proto
                    });
                    proto
                }];
                proto
            })
        };
        let group = "123-456@g.us";
        let (mut worker, _events, _inbox, _wa) = worker();
        worker.apply_history(
            ParsedHistory {
                chats: vec![
                    conversation(PEER, Status::DELIVERY_ACK),
                    conversation(group, Status::DELIVERY_ACK),
                ],
                push_names: Vec::new(),
                lids: Vec::new(),
                stickers: Vec::new(),
            },
            true,
        );
        let direct = worker.archive.message(PEER, "history").unwrap().unwrap();
        assert_eq!(direct.delivered_at, Some(100));
        let grouped = worker.archive.message(group, "history").unwrap().unwrap();
        assert_eq!(grouped.status, Delivery::Delivered, "the phone's aggregate");
        assert_eq!(grouped.read_at, None);
        let receipts = worker.archive.receipts(group, "history").unwrap();
        assert_eq!(receipts.len(), 1);
        assert_eq!(receipts[0].id, PEER);
        assert!(!receipts[0].expected, "a partial list is not the audience");
        assert_eq!(receipts[0].read_at, Some(123));
    }

    #[test]
    fn a_zero_history_receipt_time_neither_reads_nor_dates_a_message() {
        use wa::web_message_info::Status;
        let conversation = |id: &str, status| {
            parse_conversation({
                let mut proto = wa::Conversation::default();
                proto.id = PEER.into();
                proto.messages = vec![{
                    let mut proto = wa::HistorySyncMsg::default();
                    proto.message = MessageField::some({
                        let mut proto = wa::WebMessageInfo::default();
                        proto.key = MessageField::some({
                            let mut proto = wa::MessageKey::default();
                            proto.id = Some(id.into());
                            proto.from_me = Some(true);
                            proto
                        });
                        proto.message = MessageField::some({
                            let mut proto = wa::Message::default();
                            proto.conversation = Some("hello".into());
                            proto
                        });
                        proto.message_timestamp = Some(90);
                        proto.status = Some(status);
                        proto.user_receipt = vec![{
                            let mut proto = wa::UserReceipt::default();
                            proto.user_jid = PEER.into();
                            proto.receipt_timestamp = Some(100);
                            proto.read_timestamp = Some(0);
                            proto
                        }];
                        proto
                    });
                    proto
                }];
                proto
            })
        };
        // A contact who hides read receipts: the phone says delivered.
        let hidden = conversation("hidden", Status::DELIVERY_ACK);
        assert_eq!(hidden.messages[0].status, Delivery::Delivered);
        let (mut worker, _events, _inbox, _wa) = worker();
        let mut chats = vec![hidden];
        // The phone says read, but the receipt carries no read time.
        chats[0]
            .messages
            .extend(conversation("read", Status::READ).messages);
        worker.apply_history(
            ParsedHistory {
                chats,
                push_names: Vec::new(),
                lids: Vec::new(),
                stickers: Vec::new(),
            },
            true,
        );
        let hidden = worker.archive.message(PEER, "hidden").unwrap().unwrap();
        assert_eq!(hidden.status, Delivery::Delivered);
        assert_eq!(hidden.read_at, None);
        assert_eq!(hidden.delivered_at, Some(100));
        let read = worker.archive.message(PEER, "read").unwrap().unwrap();
        assert_eq!(read.status, Delivery::Read);
        assert_eq!(
            read.read_at, None,
            "\"Message info\" says the time was not recorded"
        );
    }

    /// The cache is measured by kind and emptied without touching a download
    /// still being written.
    #[test]
    fn the_media_cache_is_measured_and_emptied_but_not_mid_download() {
        let root = tempfile::tempdir().expect("temp");
        let dir = root.path().join("media");
        assert_eq!(
            measure_media(&dir).expect("an absent cache is empty"),
            crate::model::StorageStats::default()
        );
        std::fs::create_dir_all(dir.join("cards")).expect("dirs");
        for (name, bytes) in [
            ("a.jpg", 10),
            ("b.mp4", 20),
            ("c.webp", 3),
            ("cards/d.pdf", 4),
            (".e.part", 99),
        ] {
            std::fs::write(dir.join(name), vec![0u8; bytes]).expect("write");
        }
        let stats = measure_media(&dir).expect("measured");
        assert_eq!(
            stats.files_total(),
            4,
            "the download in progress is left out"
        );
        assert_eq!(stats.bytes_total(), 37);
        assert_eq!((stats.other, stats.other_bytes), (1, 4));
        clear_media(&dir).expect("cleared");
        assert_eq!(measure_media(&dir).expect("measured").files_total(), 0);
        assert!(
            dir.join(".e.part").exists(),
            "a download still being written stays"
        );
        assert!(dir.join("cards").is_dir(), "the folders stay");
    }

    /// After the cache is emptied, the archive forgets the files that went,
    /// so they download again, and keeps a file in a chosen download folder.
    #[tokio::test]
    async fn a_cleared_cache_is_forgotten_by_the_archive_only_where_it_was() {
        use crate::model::{Media, MediaState};
        let (mut worker, events, _inbox, _wa) = receipt_tests::worker();
        let elsewhere = tempfile::tempdir().expect("temp");
        let cache = worker.dirs.media_cache_dir();
        std::fs::create_dir_all(&cache).expect("cache");
        let cached = cache.join("photo.jpg");
        let kept = elsewhere.path().join("kept.jpg");
        for (id, path) in [("photo", &cached), ("kept", &kept)] {
            std::fs::write(path, b"jpeg").expect("file");
            let mut picture = incoming(id, 100);
            picture.content = Content::Image {
                motion: None,
                media: Media {
                    mime: "image/jpeg".into(),
                    size: 4,
                    width: None,
                    height: None,
                    album: None,
                    path: None,
                    state: MediaState::Idle,
                },
                caption: None,
            };
            worker.store_message(picture, None, None);
            worker
                .archive
                .set_media_path(PEER, id, path)
                .expect("path")
                .expect("row");
        }
        clear_media(&cache).expect("cleared");
        worker
            .handle_command(Command::StorageMeasured {
                stats: Some(measure_media(&cache).expect("measured")),
                token: 7,
                cleared: true,
            })
            .await;
        let path = |id: &str| {
            worker
                .archive
                .message(PEER, id)
                .expect("read")
                .expect("row")
                .content
                .media()
                .and_then(|media| media.path.clone())
        };
        assert_eq!(path("photo"), None, "the cached photo downloads again");
        assert_eq!(
            path("kept"),
            Some(kept.clone()),
            "a chosen folder's file stays"
        );
        assert!(kept.exists());
        assert!(events.try_iter().any(|event| matches!(
            event,
            Event::StorageStats { token: 7, cleared: true, stats: Some(stats) }
                if stats.files_total() == 0
        )));
    }

    /// A duplicate delivery or a history replay reclassifies the same message,
    /// and a fresh classification carries no local path. Replacing the row with
    /// it dropped the file that is already on the computer, so the bubble went
    /// back to offering the download.
    #[tokio::test]
    async fn a_duplicate_delivery_keeps_the_downloaded_file() {
        use crate::model::{Media, MediaState};
        let (mut worker, _events, _inbox, _wa) = receipt_tests::worker();
        let mut picture = incoming("photo", 100);
        picture.content = Content::Image {
            motion: None,
            media: Media {
                mime: "image/jpeg".into(),
                size: 10,
                width: None,
                height: None,
                album: None,
                path: None,
                state: MediaState::Idle,
            },
            caption: None,
        };
        worker.store_message(picture.clone(), None, None);
        let chat = PEER.to_owned();
        let downloaded = std::path::PathBuf::from("/tmp/zapfast-photo.jpg");
        worker
            .archive
            .set_media_path(&chat, "photo", &downloaded)
            .expect("path")
            .expect("row");

        // The same message arrives again, as history replay or a redelivery.
        worker.store_message(picture, None, None);

        let stored = worker
            .archive
            .message(&chat, "photo")
            .expect("read")
            .expect("row");
        let Some(media) = stored.content.media() else {
            panic!("the picture is still a picture");
        };
        assert_eq!(
            media.path.as_deref(),
            Some(downloaded.as_path()),
            "the file on the computer survives the replay"
        );

        // And again through history sync, which files messages on its own path.
        let history = parse_conversation({
            let mut proto = wa::Conversation::default();
            proto.id = PEER.into();
            proto.messages = vec![{
                let mut proto = wa::HistorySyncMsg::default();
                proto.message = MessageField::some({
                    let mut proto = wa::WebMessageInfo::default();
                    proto.key = MessageField::some({
                        let mut proto = wa::MessageKey::default();
                        proto.id = Some("photo".into());
                        proto.from_me = Some(false);
                        proto
                    });
                    proto.message = MessageField::some({
                        let mut proto = wa::Message::default();
                        proto.image_message = MessageField::some({
                            let mut proto = wa::message::ImageMessage::default();
                            proto.mimetype = Some("image/jpeg".into());
                            proto.file_length = Some(10);
                            proto
                        });
                        proto
                    });
                    proto.message_timestamp = Some(100);
                    proto
                });
                proto
            }];
            proto
        });
        worker.apply_history(
            ParsedHistory {
                chats: vec![history],
                push_names: Vec::new(),
                lids: Vec::new(),
                stickers: Vec::new(),
            },
            false,
        );
        let stored = worker
            .archive
            .message(&chat, "photo")
            .expect("read")
            .expect("row");
        assert_eq!(
            stored
                .content
                .media()
                .and_then(|media| media.path.as_deref()),
            Some(downloaded.as_path()),
            "the file on the computer survives history sync"
        );
    }

    fn incoming(id: &str, timestamp: i64) -> Message {
        Message {
            from_me: false,
            sender: PEER.into(),
            status: Delivery::None,
            ..own_message(id, timestamp)
        }
    }

    #[test]
    fn unknown_or_disabled_account_privacy_never_permits_receipts() {
        use whatsapp_rust::wacore::iq::privacy::{
            PrivacyCategory, PrivacySetting, PrivacySettingsResponse, PrivacyValue,
        };
        let mut settings = PrivacySettingsResponse {
            settings: Vec::new(),
        };
        assert!(!account_allows_receipts(&settings));
        settings.settings.push(PrivacySetting {
            category: PrivacyCategory::ReadReceipts,
            value: PrivacyValue::None,
        });
        assert!(!account_allows_receipts(&settings));
        settings.settings[0].value = PrivacyValue::All;
        assert!(account_allows_receipts(&settings));
        settings.settings[0].value = PrivacyValue::None;
        assert!(
            !account_allows_receipts(&settings),
            "a phone privacy change takes effect without reconnecting"
        );
    }

    fn unread(worker: &Worker) -> u32 {
        worker.archive.chat(PEER).unwrap().unwrap().unread
    }

    fn history(unread: u32) -> ParsedHistory {
        ParsedHistory {
            chats: vec![parse_conversation({
                let mut proto = wa::Conversation::default();
                proto.id = PEER.into();
                proto.unread_count = Some(unread);
                proto.conversation_timestamp = Some(200);
                proto
            })],
            push_names: Vec::new(),
            lids: Vec::new(),
            stickers: Vec::new(),
        }
    }

    #[test]
    fn history_preserves_pin_time_and_distinguishes_missing_mute_metadata() {
        let chat = parse_conversation({
            let mut proto = wa::Conversation::default();
            proto.id = PEER.into();
            proto.pinned = Some(1_700_000_000);
            proto.mute_end_time = Some(1_800_000_000);
            proto
        });
        assert_eq!(chat.pinned_at, Some(1_700_000_000_000));
        assert_eq!(chat.muted_until, Some(Some(1_800_000_000)));
        assert_eq!(chat.locked, None, "absence must preserve existing state");
        let chat = parse_conversation({
            let mut proto = wa::Conversation::default();
            proto.id = PEER.into();
            proto.locked = Some(true);
            proto
        });
        assert_eq!(chat.locked, Some(true));
        for (end, expected) in [
            (None, None),
            (Some(0), Some(None)),
            (Some(u64::MAX), Some(Some(0))),
        ] {
            let chat = parse_conversation({
                let mut proto = wa::Conversation::default();
                proto.id = PEER.into();
                proto.mute_end_time = end;
                proto
            });
            assert_eq!(chat.muted_until, expected);
        }
    }

    #[tokio::test]
    async fn history_keeps_a_channel_name_from_the_server() {
        let (mut worker, _events, _inbox, _wa) = worker();
        const CHANNEL: &str = "1@newsletter";
        worker
            .archive
            .ensure_chat(CHANNEL, &fallback_name(CHANNEL))
            .unwrap();
        worker
            .handle_command(Command::ChannelNames {
                generation: worker.channel_list_generation,
                names: vec![(CHANNEL.into(), "Fixture news".into())],
            })
            .await;
        let replay = ParsedHistory {
            chats: vec![parse_conversation({
                let mut proto = wa::Conversation::default();
                proto.id = CHANNEL.into();
                proto.name = Some("Older name".into());
                proto.conversation_timestamp = Some(200);
                proto
            })],
            push_names: Vec::new(),
            lids: Vec::new(),
            stickers: Vec::new(),
        };
        worker.apply_history(replay, true);
        let chat = worker.archive.chat(CHANNEL).unwrap().unwrap();
        assert_eq!(chat.name, "Fixture news");
    }

    #[tokio::test]
    async fn a_synced_mute_does_not_file_a_channel_we_do_not_have() {
        let (mut worker, _events, _inbox, _wa) = worker();
        const GONE: &str = "1@newsletter";
        const KEPT: &str = "2@newsletter";
        worker.archive.ensure_chat(KEPT, "Fixture news").unwrap();
        for channel in [GONE, KEPT] {
            let mute = wa_events::MuteUpdate::builder()
                .jid(channel.parse().unwrap())
                .timestamp(whatsapp_rust::wacore::time::now_utc())
                .from_full_sync(true)
                .action(Box::new({
                    let mut proto = wa::sync_action_value::MuteAction::default();
                    proto.muted = Some(true);
                    proto.mute_end_timestamp = Some(-1);
                    proto
                }))
                .build();
            worker
                .handle_wa_event(Arc::new(wa_events::Event::MuteUpdate(mute)))
                .await;
        }
        assert!(worker.archive.chat(GONE).unwrap().is_none());
        let kept = worker.archive.chat(KEPT).unwrap().unwrap();
        assert_eq!(kept.muted_until, Some(0), "a channel we have still mutes");
    }

    #[tokio::test]
    async fn mute_and_pin_sync_before_history_survive_replays_and_unsetting() {
        let (mut worker, _events, _inbox, _wa) = worker();
        let time = whatsapp_rust::wacore::time::now_utc();
        for enabled in [true, false] {
            let mute = wa_events::MuteUpdate::builder()
                .jid(PEER.parse().unwrap())
                .timestamp(time)
                .from_full_sync(true)
                .action(Box::new({
                    let mut proto = wa::sync_action_value::MuteAction::default();
                    proto.muted = Some(enabled);
                    proto.mute_end_timestamp = Some(-1);
                    proto
                }))
                .build();
            let pin = wa_events::PinUpdate::builder()
                .jid(PEER.parse().unwrap())
                .timestamp(time)
                .from_full_sync(true)
                .action(Box::new({
                    let mut proto = wa::sync_action_value::PinAction::default();
                    proto.pinned = Some(enabled);
                    proto
                }))
                .build();
            worker
                .handle_wa_event(Arc::new(wa_events::Event::MuteUpdate(mute)))
                .await;
            worker
                .handle_wa_event(Arc::new(wa_events::Event::PinUpdate(pin)))
                .await;
            let before = worker
                .archive
                .chat(PEER)
                .unwrap()
                .expect("sync creates the chat");
            assert_eq!(before.muted_until, enabled.then_some(0));
            assert_eq!(before.pinned, enabled);
            assert_eq!(
                before.pinned_at,
                if enabled { time.timestamp_millis() } else { 0 }
            );

            let mut stale = history(0);
            stale.chats[0].pinned_at = Some(if enabled { 0 } else { 123_000 });
            stale.chats[0].muted_until = Some(if enabled { None } else { Some(0) });
            worker.apply_history(stale, true);
            let after = worker.archive.chat(PEER).unwrap().unwrap();
            assert_eq!(after.muted_until, before.muted_until);
            assert_eq!(after.pinned, before.pinned);
            assert_eq!(after.pinned_at, before.pinned_at);
        }
    }

    #[tokio::test]
    async fn lock_sync_survives_stale_history_replay() {
        let (mut worker, _events, _inbox, _wa) = worker();
        let time = whatsapp_rust::wacore::time::now_utc();
        let lock = wa_events::LockChatUpdate::builder()
            .jid(PEER.parse().unwrap())
            .timestamp(time)
            .from_full_sync(true)
            .action(Box::new({
                let mut proto = wa::sync_action_value::LockChatAction::default();
                proto.locked = Some(true);
                proto
            }))
            .build();
        worker
            .handle_wa_event(Arc::new(wa_events::Event::LockChatUpdate(lock)))
            .await;
        let before = worker
            .archive
            .chat(PEER)
            .unwrap()
            .expect("sync creates the chat");
        assert!(before.locked);

        // A history chunk cannot supersede a timestamped app-state update.
        worker.apply_history(history(0), true);
        assert!(worker.archive.chat(PEER).unwrap().unwrap().locked);
        let mut locked_history = history(0);
        locked_history.chats[0].locked = Some(false);
        worker.apply_history(locked_history, true);
        assert!(worker.archive.chat(PEER).unwrap().unwrap().locked);
    }

    #[tokio::test]
    async fn archive_sync_before_history_survives_replays() {
        let (mut worker, _events, _inbox, _wa) = worker();
        let time = whatsapp_rust::wacore::time::now_utc();
        let archive = wa_events::ArchiveUpdate::builder()
            .jid(PEER.parse().unwrap())
            .timestamp(time)
            .from_full_sync(true)
            .action(Box::new({
                let mut proto = wa::sync_action_value::ArchiveChatAction::default();
                proto.archived = Some(true);
                proto
            }))
            .build();
        worker
            .handle_wa_event(Arc::new(wa_events::Event::ArchiveUpdate(archive)))
            .await;
        assert!(
            worker
                .archive
                .chat(PEER)
                .unwrap()
                .expect("sync creates the chat")
                .archived
        );
        // Neither missing nor stale history metadata unarchives the chat.
        worker.apply_history(history(0), true);
        assert!(worker.archive.chat(PEER).unwrap().unwrap().archived);
        let mut stale = history(0);
        stale.chats[0].archived = Some(false);
        worker.apply_history(stale, true);
        assert!(worker.archive.chat(PEER).unwrap().unwrap().archived);
        // Without an app-state version, history still decides.
        let other = history(0);
        let id = other.chats[0].id.clone();
        worker
            .archive
            .set_archived_at(&id, false, time.timestamp_millis() + 1)
            .unwrap();
        assert!(!worker.archive.chat(PEER).unwrap().unwrap().archived);
    }

    #[test]
    fn archive_range_names_the_newest_message() {
        let jid: Jid = PEER.parse().unwrap();
        let last = incoming("newest", 300);
        let range = archive_range(&jid, Some(&last));
        assert_eq!(range.last_message_timestamp, Some(300));
        assert_eq!(range.messages.len(), 1);
        assert_eq!(range.messages[0].timestamp, Some(300));
        assert_eq!(range.messages[0].key.id.as_deref(), Some("newest"));
    }

    #[test]
    fn archive_range_leaves_local_call_entries_out() {
        let jid: Jid = PEER.parse().unwrap();
        let last = incoming(&format!("{}1", crate::model::CALL_ENTRY_PREFIX), 400);
        let range = archive_range(&jid, Some(&last));
        assert_eq!(range.last_message_timestamp, Some(400));
        assert!(range.messages.is_empty());
        let empty = archive_range(&jid, None);
        assert!(empty.messages.is_empty());
        assert!(empty.last_message_timestamp.is_some());
    }

    #[test]
    fn the_archive_range_skips_sends_the_phone_never_got() {
        let (mut worker, _events, _inbox, _wa) = worker();
        worker.store_message(incoming("theirs", 100), None, None);
        worker.store_message(
            Message {
                status: Delivery::Failed,
                ..own_message("failed", 200)
            },
            None,
            None,
        );
        worker.store_message(
            Message {
                status: Delivery::Pending,
                ..own_message("pending", 300)
            },
            None,
            None,
        );
        let newest = worker.newest_on_phone(PEER).unwrap().expect("a message");
        assert_eq!(newest.id, "theirs");
    }

    #[test]
    fn the_archive_range_names_the_sender_in_a_group() {
        let group: Jid = "123-456@g.us".parse().unwrap();
        let last = Message {
            chat: "123-456@g.us".into(),
            sender: PEER.into(),
            ..incoming("theirs", 100)
        };
        let range = archive_range(&group, Some(&last));
        assert_eq!(range.messages.len(), 1);
        assert_eq!(range.messages[0].key.participant.as_deref(), Some(PEER));
        let untimed = archive_range(
            &group,
            Some(&Message {
                timestamp: 0,
                ..last
            }),
        );
        assert!(untimed.messages.is_empty());
        assert!(untimed.last_message_timestamp.is_some_and(|at| at > 0));
    }

    #[tokio::test]
    async fn pairing_qr_opens_linked_devices_from_a_phone_camera() {
        let (mut worker, _events, _inbox, _wa) = worker();
        let qr = wa_events::PairingQrCode::builder()
            .code("ref,noise,identity,adv,0".into())
            .timeout(std::time::Duration::from_secs(20))
            .build();
        worker
            .handle_wa_event(Arc::new(wa_events::Event::PairingQrCode(qr)))
            .await;
        assert_eq!(
            worker.qr.as_deref(),
            Some("https://wa.me/settings/linked_devices#ref,noise,identity,adv,0")
        );
    }

    #[test]
    fn history_archive_state_applies_until_the_phone_sends_its_own() {
        let (mut worker, _events, _inbox, _wa) = worker();
        let mut archived = history(0);
        archived.chats[0].archived = Some(true);
        worker.apply_history(archived, true);
        assert!(worker.archive.chat(PEER).unwrap().unwrap().archived);
        worker.apply_history(history(0), true);
        assert!(
            worker.archive.chat(PEER).unwrap().unwrap().archived,
            "a chunk without archive metadata keeps the state"
        );
        let mut unarchived = history(0);
        unarchived.chats[0].archived = Some(false);
        worker.apply_history(unarchived, true);
        assert!(!worker.archive.chat(PEER).unwrap().unwrap().archived);
    }

    #[tokio::test]
    async fn new_messages_unarchive_only_when_chats_are_not_kept_archived() {
        let (mut worker, _events, _inbox, _wa) = worker();
        let archived = |worker: &Worker| worker.archive.chat(PEER).unwrap().unwrap().archived;
        worker.store_message(incoming("before", 100), None, None);
        worker.archive.set_archived_at(PEER, true, 200_000).unwrap();
        // Kept archived, the default, as ZapFast always behaved.
        worker.store_message(incoming("kept", 300), None, None);
        assert!(archived(&worker));

        worker
            .handle_command(Command::SetKeepChatsArchived(false))
            .await;
        // A late delivery from before the archiving leaves it there, and so
        // does a duplicate of a message filed while it was kept.
        worker.store_message(incoming("late", 150), None, None);
        worker.store_message(incoming("kept", 300), None, None);
        assert!(archived(&worker));
        worker.store_message(incoming("new", 400), None, None);
        assert!(!archived(&worker));

        // An app-state archive after that message still applies, and our own
        // message sent later brings the chat back again.
        worker.archive.set_archived_at(PEER, true, 500_000).unwrap();
        assert!(archived(&worker));
        worker.store_message(own_message("reply", 600), None, None);
        assert!(!archived(&worker));
        let mut stale = history(0);
        stale.chats[0].archived = Some(true);
        worker.apply_history(stale, true);
        assert!(!archived(&worker), "history cannot undo it");
    }

    #[tokio::test]
    async fn the_phones_keep_chats_archived_wins_over_settings() {
        let (mut worker, _events, _inbox, _wa) = worker();
        let archived = |worker: &Worker| worker.archive.chat(PEER).unwrap().unwrap().archived;
        let setting = |unarchive: bool| {
            let mut action = wa::sync_action_value::UnarchiveChatsSetting::default();
            action.unarchive_chats = Some(unarchive);
            Arc::new(wa_events::Event::UnarchiveChatsSettingUpdate(
                wa_events::UnarchiveChatsSettingUpdate::builder()
                    .unarchive_chats(unarchive)
                    .timestamp(whatsapp_rust::wacore::time::now_utc())
                    .action(Box::new(action))
                    .from_full_sync(true)
                    .build(),
            ))
        };
        worker.store_message(incoming("before", 100), None, None);
        worker.archive.set_archived_at(PEER, true, 200_000).unwrap();
        // Settings keep chats archived, but the phone does not.
        worker.handle_wa_event(setting(true)).await;
        worker.store_message(incoming("new", 300), None, None);
        assert!(!archived(&worker));

        // Now the phone keeps them, though Settings would not.
        worker.archive.set_archived_at(PEER, true, 400_000).unwrap();
        worker
            .handle_command(Command::SetKeepChatsArchived(false))
            .await;
        worker.handle_wa_event(setting(false)).await;
        worker.store_message(incoming("kept", 500), None, None);
        assert!(archived(&worker));
    }

    #[test]
    fn an_unread_phone_setting_is_read_again_before_settings_decide() {
        let (mut worker, _events, _inbox, _wa) = worker();
        worker.keep_chats_archived = true;
        // Stored by an earlier session, but unreadable at this one's start.
        worker.archive.set_unarchive_chats_at(true, 100).unwrap();
        worker.phone_unarchive_unread = true;
        assert!(
            !worker.keeps_chats_archived(),
            "the phone's stored setting decides"
        );
        assert!(!worker.phone_unarchive_unread);
    }

    #[test]
    fn early_privacy_id_mute_reaches_the_canonical_chat_without_a_duplicate() {
        let (mut worker, events, _inbox, _wa) = worker();
        worker.ensure_chat(PEER_LID, None);
        worker.archive.set_muted_at(PEER_LID, Some(0), 200).unwrap();
        worker.learn_lid("167650256810092", "4917663430455");
        let mut snapshot = history(0);
        snapshot.chats[0].pinned_at = Some(123_000);
        worker.apply_history(snapshot, true);
        let chat = worker.archive.chat(PEER).unwrap().unwrap();
        assert_eq!(chat.muted_until, Some(0));
        assert!(chat.pinned, "missing pin sync must not block history's pin");
        worker.emit_chats();
        let chats = events
            .try_iter()
            .filter_map(|event| match event {
                Event::Chats(chats) => Some(chats),
                _ => None,
            })
            .last()
            .unwrap();
        assert!(chats.iter().any(|chat| chat.id == PEER));
        assert!(!chats.iter().any(|chat| chat.id == PEER_LID));
        worker.archive.set_muted_at(PEER, None, 300).unwrap();
        worker
            .archive
            .put_lid("167650256810092", "4917663430455")
            .unwrap();
        assert_eq!(
            worker.archive.chat(PEER).unwrap().unwrap().muted_until,
            None
        );
    }

    #[test]
    fn history_without_mute_metadata_preserves_the_existing_history_value() {
        let (mut worker, _events, _inbox, _wa) = worker();
        let mut first = history(0);
        first.chats[0].muted_until = Some(Some(0));
        worker.apply_history(first, true);
        worker.apply_history(history(0), true);
        assert_eq!(
            worker.archive.chat(PEER).unwrap().unwrap().muted_until,
            Some(0)
        );
        let mut unmuted = history(0);
        unmuted.chats[0].muted_until = Some(None);
        worker.apply_history(unmuted, true);
        assert_eq!(
            worker.archive.chat(PEER).unwrap().unwrap().muted_until,
            None
        );
    }

    #[test]
    fn reading_without_blue_ticks_still_queues_private_sync_and_survives_history() {
        let (mut worker, _events, _inbox, _wa) = worker();
        worker.store_message(incoming("a", 100), None, None);
        worker.store_message(incoming("b", 200), None, None);
        assert_eq!(unread(&worker), 2);
        worker.mark_read(PEER.into(), false);
        assert_eq!(unread(&worker), 0);
        assert_eq!(
            worker.archive.pending_reads().unwrap(),
            vec![(PEER.into(), 200)]
        );
        worker.apply_history(history(2), true);
        assert_eq!(
            unread(&worker),
            0,
            "stale history must not resurrect badges"
        );
        worker.store_message(incoming("late", 150), None, None);
        assert_eq!(unread(&worker), 0, "a delayed read message stays read");
        worker.store_message(incoming("new", 300), None, None);
        worker.apply_history(history(2), false);
        assert_eq!(
            unread(&worker),
            1,
            "paging old history preserves a new unread message"
        );
    }

    #[tokio::test]
    async fn a_failed_read_sync_stays_queued_until_it_succeeds() {
        let (mut worker, _events, _inbox, _wa) = worker();
        worker.store_message(incoming("a", 100), None, None);
        worker.mark_read(PEER.into(), false);
        let now = Instant::now();
        assert!(worker.read_sync.start(PEER, 100, now));
        worker
            .handle_command(Command::ReadSyncFinished {
                chat: PEER.into(),
                through: 100,
                success: false,
            })
            .await;
        assert_eq!(
            worker.archive.pending_reads().unwrap(),
            vec![(PEER.into(), 100)]
        );
        assert!(!worker.read_sync.ready(Instant::now()));
        assert!(!worker.read_sync.start("another-chat", 200, Instant::now()));
        // A new local read stays queued while the shared collection backs off.
        worker.store_message(incoming("b", 200), None, None);
        worker.mark_read(PEER.into(), false);
        assert!(
            worker
                .read_sync
                .start(PEER, 100, now + Duration::from_secs(31))
        );
        worker
            .handle_command(Command::ReadSyncFinished {
                chat: PEER.into(),
                through: 100,
                success: true,
            })
            .await;
        assert_eq!(
            worker.archive.pending_reads().unwrap(),
            vec![(PEER.into(), 200)]
        );
        assert!(worker.read_sync.start(PEER, 200, Instant::now()));
        worker
            .handle_command(Command::ReadSyncFinished {
                chat: PEER.into(),
                through: 200,
                success: true,
            })
            .await;
        assert!(worker.archive.pending_reads().unwrap().is_empty());
        assert!(worker.read_sync.ready(Instant::now()));
    }

    fn marked(worker: &Worker) -> bool {
        worker.archive.chat(PEER).unwrap().unwrap().marked_unread
    }

    async fn phone_marks(worker: &mut Worker, read: bool) {
        let event = wa_events::MarkChatAsReadUpdate::builder()
            .jid(PEER.parse().unwrap())
            .timestamp(whatsapp_rust::wacore::time::now_utc())
            .from_full_sync(false)
            .action(Box::new({
                let mut proto = wa::sync_action_value::MarkChatAsReadAction::default();
                proto.read = Some(read);
                proto.message_range = MessageField::none();
                proto
            }))
            .build();
        worker
            .handle_wa_event(Arc::new(wa_events::Event::MarkChatAsReadUpdate(event)))
            .await;
    }

    #[tokio::test]
    async fn a_chat_marked_unread_on_the_phone_shows_the_empty_dot() {
        let (mut worker, _events, _inbox, _wa) = worker();
        worker.store_message(incoming("a", 100), None, None);
        worker.mark_read(PEER.into(), false);
        phone_marks(&mut worker, false).await;
        assert!(marked(&worker));
        assert_eq!(unread(&worker), 0, "the phone's mark invents no count");
        assert!(
            worker.archive.pending_reads().unwrap().is_empty(),
            "the phone's newer mark replaces a read still waiting to go out"
        );
        phone_marks(&mut worker, true).await;
        assert!(!marked(&worker), "a read on the phone takes the mark off");
    }

    #[tokio::test]
    async fn a_chat_marked_unread_here_reaches_the_phone_and_opening_it_reads_it() {
        let (mut worker, _events, _inbox, _wa) = worker();
        worker.store_message(incoming("a", 100), None, None);
        worker.mark_read(PEER.into(), false);
        worker.archive.finish_read_sync(PEER, i64::MAX).unwrap();
        worker
            .handle_command(Command::MarkUnread(PEER.into()))
            .await;
        assert!(marked(&worker));
        let pending = worker.archive.pending_unreads().unwrap();
        assert_eq!(pending.len(), 1, "the mark waits for the phone");
        let (_, marked_at, through) = pending[0].clone();
        assert_eq!(through, 100, "the mark covers the latest message");
        assert!(
            worker
                .read_sync
                .start_unread(PEER, marked_at, Instant::now())
        );
        worker
            .handle_command(Command::UnreadSyncFinished {
                chat: PEER.into(),
                marked_at,
                success: true,
            })
            .await;
        assert!(worker.archive.pending_unreads().unwrap().is_empty());
        // Opening the chat reads it, and the read goes to the phone even
        // though no message was pending, since that clears the phone's mark.
        worker.mark_read(PEER.into(), false);
        assert!(!marked(&worker));
        assert_eq!(worker.archive.pending_reads().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn a_chat_with_unread_messages_is_not_marked_again() {
        let (mut worker, _events, _inbox, _wa) = worker();
        worker.store_message(incoming("a", 100), None, None);
        assert_eq!(unread(&worker), 1);
        worker
            .handle_command(Command::MarkUnread(PEER.into()))
            .await;
        assert!(!marked(&worker), "a counted chat already reads as unread");
        assert!(worker.archive.pending_unreads().unwrap().is_empty());
    }

    #[test]
    fn a_history_snapshot_carries_the_phones_unread_mark() {
        let (mut worker, _events, _inbox, _wa) = worker();
        let mut chunk = history(0);
        chunk.chats[0].marked_unread = Some(true);
        worker.apply_history(chunk, true);
        assert!(marked(&worker));
        assert_eq!(unread(&worker), 0);
    }

    #[test]
    fn the_first_server_ack_restamps_our_message() {
        let (mut worker, _events, _inbox, _wa) = worker();
        // Our clock is ahead: the message was stamped 200 before the server answered.
        worker.store_message(
            Message {
                status: Delivery::Pending,
                ..own_message("mine", 200)
            },
            None,
            None,
        );
        let ack = |id: &str, class: &str, at: i64| {
            wa_events::ServerAck::builder()
                .id(id.to_owned())
                .class(class.to_owned())
                .timestamp(whatsapp_rust::wacore::time::from_secs(at).unwrap())
                .build()
        };
        let stamp = |worker: &Worker, id: &str| {
            worker.archive.message(PEER, id).unwrap().unwrap().timestamp
        };
        worker.server_ack(&ack("mine", "message", 140));
        assert_eq!(stamp(&worker, "mine"), 140);
        // A resend is acked again, later; the first ack already placed it.
        worker.server_ack(&ack("mine", "message", 500));
        assert_eq!(stamp(&worker, "mine"), 140);
        // An ack for another kind of stanza never moves a message.
        worker.store_message(
            Message {
                status: Delivery::Pending,
                ..own_message("other", 300)
            },
            None,
            None,
        );
        worker.server_ack(&ack("other", "receipt", 90));
        assert_eq!(stamp(&worker, "other"), 300);
    }

    #[test]
    fn an_ack_without_a_time_leaves_our_message_waiting_for_one() {
        let (mut worker, _events, _inbox, _wa) = worker();
        worker.store_message(
            Message {
                status: Delivery::Pending,
                ..own_message("mine", 200)
            },
            None,
            None,
        );
        let untimed = wa_events::ServerAck::builder()
            .id("mine".to_owned())
            .class("message".to_owned())
            .build();
        worker.server_ack(&untimed);
        let timed = wa_events::ServerAck::builder()
            .id("mine".to_owned())
            .class("message".to_owned())
            .timestamp(whatsapp_rust::wacore::time::from_secs(140).unwrap())
            .build();
        worker.server_ack(&timed);
        assert_eq!(
            worker
                .archive
                .message(PEER, "mine")
                .unwrap()
                .unwrap()
                .timestamp,
            140
        );
    }

    #[test]
    fn an_error_ack_leaves_our_message_at_its_time() {
        let (mut worker, _events, _inbox, _wa) = worker();
        worker.store_message(
            Message {
                status: Delivery::Pending,
                ..own_message("mine", 200)
            },
            None,
            None,
        );
        let refused = wa_events::ServerAck::builder()
            .id("mine".to_owned())
            .class("message".to_owned())
            .timestamp(whatsapp_rust::wacore::time::from_secs(140).unwrap())
            .error("479".to_owned())
            .build();
        worker.server_ack(&refused);
        assert_eq!(
            worker
                .archive
                .message(PEER, "mine")
                .unwrap()
                .unwrap()
                .timestamp,
            200
        );
        // The refused ack did not drop the entry: a plain ack still places it.
        let timed = wa_events::ServerAck::builder()
            .id("mine".to_owned())
            .class("message".to_owned())
            .timestamp(whatsapp_rust::wacore::time::from_secs(140).unwrap())
            .build();
        worker.server_ack(&timed);
        assert_eq!(
            worker
                .archive
                .message(PEER, "mine")
                .unwrap()
                .unwrap()
                .timestamp,
            140
        );
    }

    #[test]
    fn replying_on_the_phone_reads_only_preceding_messages() {
        let (mut worker, events, _inbox, _wa) = worker();
        worker.store_message(incoming("old", 100), None, None);
        worker.store_message(incoming("new", 300), None, None);
        worker.store_message(
            Message {
                status: Delivery::Failed,
                ..own_message("failed", 400)
            },
            None,
            None,
        );
        assert_eq!(unread(&worker), 2, "a failed send does not read the chat");
        worker.store_message(own_message("reply", 200), None, None);
        assert_eq!(unread(&worker), 1);
        worker.store_message(own_message("reply2", 400), None, None);
        assert_eq!(unread(&worker), 0);
        worker.store_message(own_message("reply", 200), None, None);
        assert_eq!(worker.archive.read_through(PEER).unwrap(), Some(400));
        while events.try_recv().is_ok() {}
        worker.store_message(incoming("late", 150), None, None);
        assert_eq!(unread(&worker), 0);
        assert!(
            !events
                .try_iter()
                .any(|event| matches!(event, Event::Incoming { .. }))
        );
    }

    #[test]
    fn delayed_phone_receipts_preserve_newer_unread_messages() {
        let (mut worker, _events, _inbox, _wa) = worker();
        worker.learn_lid("167650256810092", "4917663430455");
        worker.store_message(incoming("old", 100), None, None);
        worker.store_message(incoming("new", 300), None, None);
        worker.on_receipt(&receipt(PEER_LID, &["old"], ReceiptType::ReadSelf));
        assert_eq!(unread(&worker), 1);
        worker.on_receipt(&receipt(PEER_LID, &["unknown"], ReceiptType::ReadSelf));
        assert_eq!(
            unread(&worker),
            1,
            "an unknown receipt has no known read position"
        );
        worker.on_receipt(&receipt(PEER_LID, &["new"], ReceiptType::ReadSelf));
        assert_eq!(unread(&worker), 0);
    }

    #[tokio::test]
    async fn a_voice_message_played_here_or_on_the_phone_is_kept_as_heard() {
        let (mut worker, events, _inbox, _wa) = worker();
        let status =
            |worker: &Worker, id: &str| worker.archive.message(PEER, id).unwrap().unwrap().status;
        worker.store_message(incoming("phone", 100), None, None);
        worker.store_message(incoming("here", 200), None, None);
        worker.store_message(own_message("ours", 300), None, None);
        while events.try_recv().is_ok() {}

        worker.on_receipt(&receipt(PEER, &["phone"], ReceiptType::PlayedSelf));
        assert_eq!(status(&worker, "phone"), Delivery::Played);
        assert!(events.try_iter().any(|event| matches!(
            event,
            Event::MessageUpdated(message) if message.id == "phone"
        )));

        worker
            .handle_command(Command::MarkPlayed {
                chat: PEER.into(),
                message: "here".into(),
                sender: PEER.into(),
                receipts: false,
            })
            .await;
        assert_eq!(
            status(&worker, "here"),
            Delivery::Played,
            "heard here even without sending receipts"
        );

        worker.on_receipt(&receipt(PEER, &["ours"], ReceiptType::PlayedSelf));
        assert_eq!(
            status(&worker, "ours"),
            Delivery::Sent,
            "our own keeps its ticks"
        );

        // Offline, the phone's play can come before its message.
        worker.on_receipt(&receipt(PEER, &["early"], ReceiptType::PlayedSelf));
        worker.store_message(incoming("early", 400), None, None);
        assert_eq!(status(&worker, "early"), Delivery::Played);

        // A replay of the message does not undo it.
        worker.store_message(incoming("phone", 100), None, None);
        assert_eq!(status(&worker, "phone"), Delivery::Played);
    }

    #[test]
    fn rapid_messages_keep_distinct_read_positions_within_the_same_second() {
        let (mut worker, _events, _inbox, _wa) = worker();
        worker.store_message(incoming("first", 100), None, None);
        worker.mark_read(PEER.into(), false);
        worker.store_message(incoming("second", 100), None, None);
        worker.store_message(incoming("third", 100), None, None);
        assert_eq!(unread(&worker), 2);
        worker.on_receipt(&receipt(PEER, &["first"], ReceiptType::ReadSelf));
        assert_eq!(unread(&worker), 2);
        worker.on_receipt(&receipt(PEER, &["second"], ReceiptType::ReadSelf));
        assert_eq!(unread(&worker), 1);
        assert_eq!(
            worker.archive.unread_incoming(PEER, 1).unwrap(),
            vec![("third".into(), PEER.into())]
        );
        worker.on_receipt(&receipt(PEER, &["third"], ReceiptType::ReadSelf));
        assert_eq!(unread(&worker), 0);
    }

    #[test]
    fn a_phone_history_snapshot_can_clear_stale_unread_counts() {
        let (mut worker, _events, _inbox, _wa) = worker();
        worker.store_message(incoming("a", 100), None, None);
        worker.store_message(incoming("b", 200), None, None);
        worker.store_message(incoming("new", 300), None, None);
        worker.apply_history(history(0), true);
        assert_eq!(
            unread(&worker),
            1,
            "a read snapshot preserves later arrivals"
        );
        worker.apply_history(history(2), true);
        assert_eq!(
            unread(&worker),
            1,
            "older unread history cannot undo a read snapshot"
        );
    }

    #[tokio::test]
    async fn phone_read_updates_cover_their_range_even_before_history_arrives() {
        let (mut worker, _events, _inbox, _wa) = worker();
        let event = wa_events::MarkChatAsReadUpdate::builder()
            .jid(PEER.parse().unwrap())
            .timestamp(whatsapp_rust::wacore::time::now_utc())
            .from_full_sync(false)
            .action(Box::new({
                let mut proto = wa::sync_action_value::MarkChatAsReadAction::default();
                proto.read = Some(true);
                proto.message_range =
                    MessageField::some(whatsapp_rust::message_range(200, None, Vec::new()));
                proto
            }))
            .build();
        worker
            .handle_wa_event(Arc::new(wa_events::Event::MarkChatAsReadUpdate(event)))
            .await;
        worker.apply_history(history(2), true);
        worker.store_message(incoming("late", 100), None, None);
        worker.store_message(incoming("new", 300), None, None);
        assert_eq!(unread(&worker), 1);
    }

    #[test]
    fn a_read_receipt_from_the_peers_privacy_id_moves_our_messages() {
        let (mut worker, _events, _inbox, _wa) = worker();
        worker.archive.ensure_chat(PEER, "R").expect("chat");
        for (id, when) in [("A1", 100), ("A2", 200), ("A3", 300)] {
            worker
                .archive
                .insert_message(&own_message(id, when), None)
                .expect("stored");
        }
        worker.learn_lid("167650256810092", "4917663430455");
        worker.on_receipt(&receipt(PEER_LID, &["A2"], ReceiptType::Read));
        let status = |id: &str| {
            worker
                .archive
                .message(PEER, id)
                .expect("read")
                .expect("row")
                .status
        };
        assert_eq!(status("A2"), Delivery::Read, "the named message");
        assert_eq!(status("A1"), Delivery::Read, "and everything before it");
        assert_eq!(status("A3"), Delivery::Sent, "not what came after");
    }

    #[test]
    fn inactive_counts_as_delivered_and_sender_only_in_the_chat_with_ourselves() {
        let (mut worker, _events, _inbox, _wa) = worker();
        worker.archive.ensure_chat(PEER, "R").expect("chat");
        worker.archive.ensure_chat(ME, "Me").expect("chat");
        worker
            .archive
            .insert_message(&own_message("C1", 100), None)
            .expect("stored");
        let mut to_self = own_message("S1", 100);
        to_self.chat = ME.into();
        worker
            .archive
            .insert_message(&to_self, None)
            .expect("stored");
        worker.on_receipt(&receipt(PEER, &["C1"], ReceiptType::Inactive));
        assert_eq!(
            worker
                .archive
                .message(PEER, "C1")
                .expect("read")
                .expect("row")
                .status,
            Delivery::Delivered,
            "an inactive device still received it"
        );
        worker.on_receipt(&receipt(PEER, &["C1"], ReceiptType::Sender));
        assert_eq!(
            worker
                .archive
                .message(PEER, "C1")
                .expect("read")
                .expect("row")
                .status,
            Delivery::Delivered,
            "our own other device says nothing about the peer"
        );
        worker.on_receipt(&receipt(ME, &["S1"], ReceiptType::Sender));
        assert_eq!(
            worker
                .archive
                .message(ME, "S1")
                .expect("read")
                .expect("row")
                .status,
            Delivery::Read,
            "a message to ourselves is read once the phone has it"
        );
    }

    #[test]
    fn a_delivery_receipt_from_the_phone_number_moves_only_the_named_message() {
        let (mut worker, _events, _inbox, _wa) = worker();
        worker.archive.ensure_chat(PEER, "R").expect("chat");
        for (id, when) in [("B1", 100), ("B2", 200)] {
            worker
                .archive
                .insert_message(&own_message(id, when), None)
                .expect("stored");
        }
        worker.on_receipt(&receipt(PEER, &["B2"], ReceiptType::Delivered));
        let status = |id: &str| {
            worker
                .archive
                .message(PEER, id)
                .expect("read")
                .expect("row")
                .status
        };
        assert_eq!(status("B2"), Delivery::Delivered);
        assert_eq!(status("B1"), Delivery::Sent);
    }
    #[test]
    fn replies_keep_the_original_reference_on_the_wire_and_in_the_archive() {
        for chat in [PEER, "123-456@g.us"] {
            for sender in [ME, PEER, "987654321@lid"] {
                let (worker, _, _, _) = worker();
                worker.archive.ensure_chat(chat, "Fixture").unwrap();
                let source = Message {
                    chat: chat.into(),
                    sender: sender.into(),
                    from_me: sender == ME,
                    content: Content::text("Original fixture"),
                    ..own_message("original", 100)
                };
                let original = wa::Message::text("Original fixture");
                worker
                    .archive
                    .insert_message(&source, Some(&original.encode_to_vec()))
                    .unwrap();
                let (context, shown) = worker.quote(chat, Some("original")).unwrap().unwrap();
                let mut reply = outgoing_text("Reply fixture".into(), Some(context), &[]);
                apply_ephemeral_expiration(&mut reply, Some(86400));
                let raw = reply.encode_to_vec();
                let decoded = wa::Message::decode_from_slice(&raw).unwrap();
                let context = context_of(&decoded).unwrap();
                assert_eq!(context.stanza_id.as_deref(), Some("original"));
                assert_eq!(context.participant.as_deref(), Some(sender));
                assert_eq!(context.remote_jid, None);
                assert_eq!(
                    context.quoted_message.as_option().unwrap().text_content(),
                    Some("Original fixture")
                );
                assert_eq!(decoded.text_content(), Some("Reply fixture"));
                assert_eq!(worker.quoted_of(&decoded).unwrap().id, "original");
                let row = Message {
                    chat: chat.into(),
                    quoted: Some(shown),
                    content: Content::text("Reply fixture"),
                    ..own_message("reply", 101)
                };
                worker.archive.insert_message(&row, Some(&raw)).unwrap();
                let stored = worker.archive.message(chat, "reply").unwrap().unwrap();
                assert_eq!(stored.quoted.unwrap().id, "original");
                let restored = wa::Message::decode_from_slice(
                    &worker.archive.raw(chat, "reply").unwrap().unwrap(),
                )
                .unwrap();
                assert_eq!(
                    context_of(&restored).unwrap().stanza_id.as_deref(),
                    Some("original")
                );
            }
        }
    }

    #[test]
    fn explicit_replies_require_a_valid_original() {
        let (worker, _, _, _) = worker();
        worker.archive.ensure_chat(PEER, "Fixture").unwrap();
        let unavailable = Err(Refusal::QuoteUnavailable);
        assert!(worker.quote(PEER, None).unwrap().is_none());
        assert_eq!(worker.quote(PEER, Some("")).map(|_| ()), unavailable);
        assert_eq!(worker.quote(PEER, Some("missing")).map(|_| ()), unavailable);
        // A row without its raw protobuf cannot be quoted.
        let mut row = own_message("original", 100);
        worker.archive.insert_message(&row, None).unwrap();
        assert_eq!(
            worker.quote(PEER, Some("original")).map(|_| ()),
            unavailable
        );
        // Nor can an unreadable protobuf.
        worker.archive.insert_message(&row, Some(&[0xff])).unwrap();
        assert_eq!(
            worker.quote(PEER, Some("original")).map(|_| ()),
            unavailable
        );
        // A readable original is quoted.
        let original = wa::Message::text("old").encode_to_vec();
        worker
            .archive
            .insert_message(&row, Some(&original))
            .unwrap();
        assert!(worker.quote(PEER, Some("original")).unwrap().is_some());
        // A deleted original is not.
        row.content = Content::Revoked;
        worker
            .archive
            .insert_message(&row, Some(&original))
            .unwrap();
        assert_eq!(
            worker.quote(PEER, Some("original")).map(|_| ()),
            unavailable
        );
    }

    /// Every command that can send a reply, quoting `quoting` in `PEER`.
    #[test]
    fn private_reply_quotes_the_group_even_with_a_matching_dm_message_id() {
        let (worker, _, _, _) = worker();
        let group = "12345@g.us";
        worker.archive.ensure_chat(group, "Group fixture").unwrap();
        worker.archive.ensure_chat(PEER, "Sender fixture").unwrap();
        let mut original = own_message("original", 100);
        original.chat = group.into();
        original.sender = PEER.into();
        original.from_me = false;
        original.content = Content::text("Group original");
        worker
            .archive
            .insert_message(
                &original,
                Some(&wa::Message::text("Group original").encode_to_vec()),
            )
            .unwrap();
        worker
            .archive
            .insert_message(
                &own_message("original", 100),
                Some(&wa::Message::text("Unrelated DM").encode_to_vec()),
            )
            .unwrap();
        let target = crate::model::ReplyTarget {
            id: "original".into(),
            chat: Some(group.into()),
        };
        let (context, shown) = worker.quote_target(PEER, Some(&target)).unwrap().unwrap();
        assert_eq!(context.remote_jid.as_deref(), Some(group));
        assert_eq!(context.participant.as_deref(), Some(PEER));
        assert_eq!(context.stanza_id.as_deref(), Some("original"));
        assert_eq!(
            context.quoted_message.as_option().unwrap().text_content(),
            Some("Group original")
        );
        assert_eq!(shown.chat.as_deref(), Some(group));
        let mut reply = wa::Message::text("Private reply");
        attach_quote(&mut reply, Some((context, shown.clone()))).unwrap();
        assert_eq!(
            worker.quoted_of(&reply).unwrap().chat.as_deref(),
            Some(group)
        );
        let mut sent = own_message("private-reply", 101);
        sent.quoted = Some(shown.clone());
        worker
            .archive
            .insert_message(&sent, Some(&reply.encode_to_vec()))
            .unwrap();
        assert_eq!(
            worker
                .archive
                .message(PEER, "private-reply")
                .unwrap()
                .unwrap()
                .quoted,
            Some(shown)
        );
        assert_eq!(
            worker
                .quote_target("someone-else@s.whatsapp.net", Some(&target))
                .map(|_| ()),
            Err(Refusal::QuoteUnavailable)
        );
        original.content = Content::Revoked;
        worker.archive.insert_message(&original, None).unwrap();
        assert_eq!(
            worker.quote_target(PEER, Some(&target)).map(|_| ()),
            Err(Refusal::QuoteUnavailable)
        );
    }

    /// Checks that each send command returns the original group source with its refused private reply.
    #[tokio::test]
    async fn every_send_path_preserves_the_private_reply_source_when_refused() {
        let (mut worker, events, _commands, _wa) = worker();
        let group = "12345@g.us";
        worker.archive.ensure_chat(group, "Group fixture").unwrap();
        let mut original = own_message("original", 100);
        original.chat = group.into();
        original.sender = PEER.into();
        original.from_me = false;
        worker
            .archive
            .insert_message(
                &original,
                Some(&wa::Message::text("Original").encode_to_vec()),
            )
            .unwrap();
        for (id, reason) in [
            ("original", Refusal::Offline),
            ("missing", Refusal::QuoteUnavailable),
        ] {
            let target = crate::model::ReplyTarget {
                id: id.into(),
                chat: Some(group.into()),
            };
            for (command, unsent) in reply_sends(Some(target.clone())) {
                worker.handle_command(command).await;
                let refused: Vec<_> = events
                    .try_iter()
                    .filter(|event| matches!(event, Event::SendRefused { .. }))
                    .collect();
                assert!(
                    matches!(refused.as_slice(), [Event::SendRefused { chat, quoting: Some(quote), unsent: returned, reason: refusal }] if chat == PEER && quote == &target && returned == &unsent && refusal == &reason)
                );
            }
        }
    }

    #[tokio::test]
    async fn view_once_file_refusals_preserve_the_choice_without_falling_back_to_normal_media() {
        let (mut worker, events, _commands, _wa) = worker();
        for (paths, documents, reason) in [
            (vec![PathBuf::from("photo.png")], false, Refusal::Offline),
            (
                vec![PathBuf::from("photo.png")],
                true,
                Refusal::UnsupportedViewOnce,
            ),
            (
                vec![PathBuf::from("photo.png"), PathBuf::from("second.jpg")],
                false,
                Refusal::UnsupportedViewOnce,
            ),
            (
                vec![PathBuf::from("clip.mp4")],
                false,
                Refusal::UnsupportedViewOnce,
            ),
            (
                vec![PathBuf::from("photo.bmp")],
                false,
                Refusal::UnsupportedViewOnce,
            ),
            (
                vec![PathBuf::from("photo.tiff")],
                false,
                Refusal::UnsupportedViewOnce,
            ),
            (Vec::new(), false, Refusal::UnsupportedViewOnce),
        ] {
            let caption = Some("Caption fixture".to_owned());
            worker
                .handle_command(Command::SendFiles {
                    chat: PEER.into(),
                    paths: paths.clone(),
                    documents,
                    view_once: true,
                    caption: caption.clone(),
                    mentions: vec![],
                    quoting: None,
                })
                .await;
            let refused: Vec<_> = events
                .try_iter()
                .filter(|event| matches!(event, Event::SendRefused { .. }))
                .collect();
            let expected = Unsent::Files {
                paths,
                documents,
                view_once: true,
                caption,
            };
            assert!(matches!(refused.as_slice(),
                [Event::SendRefused { unsent, reason: actual, .. }]
                    if unsent == &expected && actual == &reason));
        }
        worker
            .handle_command(Command::SendImage {
                chat: PEER.into(),
                width: 1,
                height: 1,
                rgba: vec![255; 4],
                view_once: true,
                caption: Some("Pasted fixture".into()),
                mentions: vec![],
                quoting: None,
            })
            .await;
        assert!(events.try_iter().any(|event| matches!(event,
            Event::SendRefused { unsent: Unsent::Image { view_once: true, caption: Some(caption), .. }, reason: Refusal::Offline, .. }
                if caption == "Pasted fixture")));
    }

    /// Enumerates synthetic send commands and matching unsent payloads with a shared quote target.
    fn reply_sends(quoting: Option<crate::model::ReplyTarget>) -> Vec<(Command, Unsent)> {
        let gif = Gif {
            id: "fixture-gif".into(),
            still: None,
            mp4: "https://example.invalid/fixture.mp4".into(),
            width: 2,
            height: 2,
        };
        vec![
            (
                Command::SendText {
                    chat: PEER.into(),
                    text: "Reply fixture".into(),
                    quoting: quoting.clone(),
                    mentions: Vec::new(),
                },
                Unsent::Text("Reply fixture".into()),
            ),
            (
                Command::SendVoice {
                    chat: PEER.into(),
                    samples: vec![0.25; 8],
                    quoting: quoting.clone(),
                },
                Unsent::Voice(vec![0.25; 8]),
            ),
            (
                Command::SendFiles {
                    chat: PEER.into(),
                    paths: vec!["/fixture/a.pdf".into(), "/fixture/b.png".into()],
                    documents: false,
                    view_once: false,
                    caption: Some("Caption fixture".into()),
                    mentions: Vec::new(),
                    quoting: quoting.clone(),
                },
                Unsent::Files {
                    paths: vec!["/fixture/a.pdf".into(), "/fixture/b.png".into()],
                    documents: false,
                    view_once: false,
                    caption: Some("Caption fixture".into()),
                },
            ),
            (
                Command::SendFiles {
                    chat: PEER.into(),
                    paths: vec!["/fixture/c.png".into()],
                    documents: true,
                    view_once: false,
                    caption: None,
                    mentions: Vec::new(),
                    quoting: quoting.clone(),
                },
                Unsent::Files {
                    paths: vec!["/fixture/c.png".into()],
                    documents: true,
                    view_once: false,
                    caption: None,
                },
            ),
            (
                Command::SendImage {
                    chat: PEER.into(),
                    width: 1,
                    height: 1,
                    rgba: vec![1, 2, 3, 4],
                    view_once: false,
                    caption: Some("Picture fixture".into()),
                    mentions: Vec::new(),
                    quoting: quoting.clone(),
                },
                Unsent::Image {
                    width: 1,
                    height: 1,
                    rgba: vec![1, 2, 3, 4],
                    view_once: false,
                    caption: Some("Picture fixture".into()),
                },
            ),
            (
                Command::SendSticker {
                    chat: PEER.into(),
                    path: "/fixture/sticker.webp".into(),
                    quoting: quoting.clone(),
                },
                Unsent::Sticker,
            ),
            (
                Command::StickerMade {
                    result: Ok("/fixture/made-sticker.webp".into()),
                    chat: Some(PEER.into()),
                    quoting: quoting.clone(),
                },
                Unsent::Sticker,
            ),
            (
                Command::SendGif {
                    chat: PEER.into(),
                    gif,
                    quoting,
                },
                Unsent::Gif,
            ),
        ]
    }

    /// Checks that every send path refuses missing originals instead of sending an unquoted reply.
    #[tokio::test]
    async fn every_send_path_refuses_a_reply_it_cannot_quote() {
        let (mut worker, events, _commands, _wa) = worker();
        worker.archive.ensure_chat(PEER, "Fixture").unwrap();
        // Stored without its raw protobuf, so it cannot be quoted.
        worker
            .archive
            .insert_message(&own_message("bare", 100), None)
            .unwrap();
        let original = wa::Message::text("hi").encode_to_vec();
        worker
            .archive
            .insert_message(&own_message("original", 101), Some(&original))
            .unwrap();
        let before = worker.archive.messages(PEER, None, 100).unwrap().len();
        for (quoting, reason) in [
            (Some("missing"), Refusal::QuoteUnavailable),
            (Some("bare"), Refusal::QuoteUnavailable),
            // A quotable reply gets past the check and then meets the missing
            // connection: the worker has no client.
            (Some("original"), Refusal::Offline),
            (None, Refusal::Offline),
        ] {
            for (command, unsent) in reply_sends(quoting.map(crate::model::ReplyTarget::from)) {
                let label = format!("{command:?}");
                worker.handle_command(command).await;
                let refused: Vec<_> = events
                    .try_iter()
                    .filter(|event| matches!(event, Event::SendRefused { .. }))
                    .collect();
                assert!(
                    matches!(
                        refused.as_slice(),
                        [Event::SendRefused { chat, quoting: q, unsent: u, reason: r }]
                            if chat == PEER && q.as_deref() == quoting && *u == unsent && *r == reason
                    ),
                    "{label} with {quoting:?}: {refused:?}"
                );
            }
        }
        // Nothing reached the archive, so nothing went out unquoted.
        assert_eq!(
            worker.archive.messages(PEER, None, 100).unwrap().len(),
            before
        );
    }

    #[tokio::test]
    async fn every_send_path_refuses_while_the_client_is_not_connected() {
        let (mut worker, events, _commands, _wa) = worker();
        let directory = tempfile::tempdir().unwrap();
        let store = whatsapp_rust::store::SqliteStore::open(
            directory.path().join("session.db").to_str().unwrap(),
        )
        .await
        .unwrap();
        // Build only, never run: the handle exists, as it does between a
        // dropped connection and the reconnect, but nothing is connected.
        let bot = Bot::builder().with_backend(store).build().await.unwrap();
        worker.client = Some(bot.client());
        assert!(!worker.client.as_ref().unwrap().is_socket_connected());
        worker.archive.ensure_chat(PEER, "Fixture").unwrap();
        let original = wa::Message::text("hi").encode_to_vec();
        worker
            .archive
            .insert_message(&own_message("original", 100), Some(&original))
            .unwrap();
        let before = worker.archive.messages(PEER, None, 100).unwrap().len();
        for quoting in [None, Some("original")] {
            for (command, unsent) in reply_sends(quoting.map(Into::into)) {
                let label = format!("{command:?}");
                worker.handle_command(command).await;
                let refused: Vec<_> = events
                    .try_iter()
                    .filter(|event| matches!(event, Event::SendRefused { .. }))
                    .collect();
                assert!(
                    matches!(
                        refused.as_slice(),
                        [Event::SendRefused { chat, quoting: q, unsent: u, reason: Refusal::Offline }]
                            if chat == PEER && q.as_deref() == quoting && *u == unsent
                    ),
                    "{label} with {quoting:?}: {refused:?}"
                );
            }
        }
        worker
            .handle_command(Command::Forward {
                from_chat: PEER.into(),
                messages: vec!["original".into()],
                to_chats: vec![PEER.into()],
            })
            .await;
        assert!(
            events
                .try_iter()
                .any(|event| matches!(event, Event::Error(_)))
        );
        // Nothing was filed, so no bubble turned into a failed send.
        assert_eq!(
            worker.archive.messages(PEER, None, 100).unwrap().len(),
            before
        );
    }

    #[test]
    fn attachments_carry_the_quote_alongside_their_mentions() {
        let (worker, _, _, _) = worker();
        worker.archive.ensure_chat(PEER, "Fixture").unwrap();
        let original = wa::Message::text("Original fixture").encode_to_vec();
        worker
            .archive
            .insert_message(&own_message("original", 100), Some(&original))
            .unwrap();
        let attachments = [
            {
                let mut proto = wa::Message::default();
                proto.image_message = MessageField::some(Default::default());
                proto
            },
            {
                let mut proto = wa::Message::default();
                proto.video_message = MessageField::some(Default::default());
                proto
            },
            {
                let mut proto = wa::Message::default();
                proto.document_message = MessageField::some(Default::default());
                proto
            },
            {
                let mut proto = wa::Message::default();
                proto.audio_message = MessageField::some(Default::default());
                proto
            },
            {
                let mut proto = wa::Message::default();
                proto.sticker_message = MessageField::some(Default::default());
                proto
            },
        ];
        for mut message in attachments {
            let quote = worker.quote(PEER, Some("original")).unwrap();
            let shown = attach_quote(&mut message, quote).unwrap().unwrap();
            assert_eq!(shown.id, "original");
            add_mentions(&mut message, &[PEER.to_owned()]);
            let decoded = wa::Message::decode_from_slice(&message.encode_to_vec()).unwrap();
            let context = context_of(&decoded).unwrap();
            assert_eq!(
                context.stanza_id.as_deref(),
                Some("original"),
                "{decoded:?}"
            );
            assert_eq!(context.mentioned_jid, vec![PEER.to_owned()]);
            assert_eq!(worker.quoted_of(&decoded).unwrap().id, "original");
        }
        // Without a reply, mentions alone still attach, and nothing is quoted.
        let mut plain = {
            let mut proto = wa::Message::default();
            proto.image_message = MessageField::some(Default::default());
            proto
        };
        assert!(attach_quote(&mut plain, None).unwrap().is_none());
        add_mentions(&mut plain, &[PEER.to_owned()]);
        let context = context_of(&plain).unwrap();
        assert_eq!(context.stanza_id, None);
        assert_eq!(context.mentioned_jid, vec![PEER.to_owned()]);
    }

    /// A reaction from `sender` in the direct chat with [`PEER`], sent at
    /// `sent_at` milliseconds; an empty emoji removes it.
    fn react_to(worker: &mut Worker, target: &str, sender: &str, emoji: &str, sent_at: i64) {
        let raw = {
            let mut proto = wa::Message::default();
            proto.reaction_message = MessageField::some({
                let mut proto = wa::message::ReactionMessage::default();
                proto.key = MessageField::some({
                    let mut proto = wa::MessageKey::default();
                    proto.remote_jid = Some(PEER.into());
                    proto.from_me = Some(false);
                    proto.id = Some(target.into());
                    proto
                });
                proto.text = Some(emoji.into());
                proto.sender_timestamp_ms = Some(sent_at);
                proto
            });
            proto
        };
        let info = MessageInfo {
            source: MessageSource {
                chat: PEER.parse().unwrap(),
                sender: sender.parse().unwrap(),
                is_from_me: sender == ME,
                ..Default::default()
            },
            timestamp: whatsapp_rust::wacore::time::from_secs(sent_at / 1000).unwrap(),
            ..Default::default()
        };
        worker.ingest(&Arc::new(raw), &info);
    }

    fn reactions(worker: &Worker, id: &str) -> Vec<(String, String)> {
        worker
            .archive
            .message(PEER, id)
            .unwrap()
            .expect("message")
            .reactions
            .into_iter()
            .map(|reaction| (reaction.sender, reaction.emoji))
            .collect()
    }

    #[test]
    fn the_phones_read_waits_for_a_message_filed_after_it() {
        let (mut worker, _events, _inbox, _wa) = worker();
        worker.on_receipt(&receipt(PEER, &["early"], ReceiptType::ReadSelf));
        worker.store_message(incoming("early", 100), None, None);
        assert_eq!(unread(&worker), 0, "the phone had read it");
        worker.store_message(incoming("later", 200), None, None);
        assert_eq!(unread(&worker), 1, "the read covers only its message");
        // The ordinary order still works.
        worker.on_receipt(&receipt(PEER, &["later"], ReceiptType::ReadSelf));
        assert_eq!(unread(&worker), 0);
    }

    #[test]
    fn the_phones_read_waits_for_a_message_from_history() {
        let (mut worker, _events, _inbox, _wa) = worker();
        worker.store_message(incoming("new", 300), None, None);
        worker.on_receipt(&receipt(PEER, &["old"], ReceiptType::ReadSelf));
        let parsed = parse_conversation({
            let mut proto = wa::Conversation::default();
            proto.id = PEER.into();
            proto.messages = vec![history_entry(
                PEER,
                "old",
                false,
                None,
                {
                    let mut proto = wa::Message::default();
                    proto.conversation = Some("hi".into());
                    proto
                },
                Vec::new(),
                None,
            )];
            proto
        });
        worker.apply_history(
            ParsedHistory {
                chats: vec![parsed],
                push_names: Vec::new(),
                lids: Vec::new(),
                stickers: Vec::new(),
            },
            true,
        );
        assert_eq!(worker.archive.read_through(PEER).unwrap(), Some(100));
        assert_eq!(unread(&worker), 1, "the newer message stays unread");
    }

    #[test]
    fn an_early_read_under_a_privacy_id_follows_its_phone_number() {
        let (mut worker, _events, _inbox, _wa) = worker();
        // Filed under the phone number, read under the unmapped privacy id.
        worker.store_message(incoming("filed", 100), None, None);
        worker.on_receipt(&receipt(
            PEER_LID,
            &["filed", "later"],
            ReceiptType::ReadSelf,
        ));
        assert_eq!(unread(&worker), 1);
        worker.learn_lid("167650256810092", "4917663430455");
        assert_eq!(unread(&worker), 0, "the mapping applies the waiting read");
        worker.store_message(incoming("later", 200), None, None);
        assert_eq!(unread(&worker), 0, "and keeps the other one waiting");
    }

    /// #276: offline, the phone read a message, reacted to it and marked the
    /// chat unread. The read and the mark reach us before the message.
    #[tokio::test]
    async fn an_early_read_does_not_undo_the_phones_later_unread_mark() {
        let (mut worker, _events, _inbox, _wa) = worker();
        worker.on_receipt(&receipt(PEER, &["m"], ReceiptType::ReadSelf));
        react_to(&mut worker, "m", ME, "👍", 150_000);
        phone_marks(&mut worker, false).await;
        worker.store_message(incoming("m", 100), None, None);
        assert_eq!(unread(&worker), 0);
        assert!(marked(&worker), "the phone's mark stays, as on the phone");
        assert_eq!(reactions(&worker, "m"), [(ME.into(), "👍".into())]);

        // Filed first, with the read arriving after the mark.
        let (mut worker, _events, _inbox, _wa) = self::worker();
        worker.store_message(incoming("m", 100), None, None);
        phone_marks(&mut worker, false).await;
        worker.on_receipt(&receipt(PEER, &["m"], ReceiptType::ReadSelf));
        assert_eq!(unread(&worker), 0);
        assert!(marked(&worker), "a late read leaves the mark");
        // And a read on the phone after the mark takes it off.
        phone_marks(&mut worker, true).await;
        assert!(!marked(&worker));
    }

    #[test]
    fn reactions_wait_for_their_message_and_the_newest_wins() {
        let (mut worker, events, _inbox, _wa) = worker();
        let other = "12025550999@s.whatsapp.net";
        react_to(&mut worker, "photo", PEER, "👍", 2_000);
        react_to(&mut worker, "photo", PEER, "", 3_000);
        react_to(&mut worker, "photo", PEER, "❤️", 1_000);
        react_to(&mut worker, "photo", ME, "😂", 2_000);
        react_to(&mut worker, "photo", other, "🔥", 2_000);
        react_to(&mut worker, "photo", other, "🎉", 4_000);
        while events.try_recv().is_ok() {}
        worker.store_message(incoming("photo", 1), None, None);
        let mut got = reactions(&worker, "photo");
        got.sort();
        let mut want = vec![
            (ME.to_owned(), "😂".to_owned()),
            (other.into(), "🎉".into()),
        ];
        want.sort();
        assert_eq!(got, want, "the removal was newer than both of the peer's");
        let shown = events.try_iter().any(|event| {
            matches!(event, Event::Messages { messages, .. }
                if messages.iter().any(|message| message.reactions.len() == 2))
        });
        assert!(shown, "the filed message carries its reactions");
        // After the message, a reaction applies at once, and so does removal.
        react_to(&mut worker, "photo", PEER, "🙏", 5_000);
        react_to(&mut worker, "photo", ME, "", 5_000);
        let mut got = reactions(&worker, "photo");
        got.sort();
        let mut want = vec![
            (PEER.to_owned(), "🙏".to_owned()),
            (other.into(), "🎉".into()),
        ];
        want.sort();
        assert_eq!(got, want);
    }

    #[test]
    fn an_early_reaction_under_a_privacy_id_reaches_the_phone_number_chat() {
        let (mut worker, _events, _inbox, _wa) = worker();
        let raw = {
            let mut proto = wa::Message::default();
            proto.reaction_message = MessageField::some({
                let mut proto = wa::message::ReactionMessage::default();
                proto.key = MessageField::some({
                    let mut proto = wa::MessageKey::default();
                    proto.remote_jid = Some(PEER_LID.into());
                    proto.from_me = Some(false);
                    proto.id = Some("m".into());
                    proto
                });
                proto.text = Some("👍".into());
                proto.sender_timestamp_ms = Some(1_000);
                proto
            });
            proto
        };
        let info = MessageInfo {
            source: MessageSource {
                chat: PEER_LID.parse().unwrap(),
                sender: PEER_LID.parse().unwrap(),
                ..Default::default()
            },
            timestamp: whatsapp_rust::wacore::time::from_secs(1).unwrap(),
            ..Default::default()
        };
        worker.ingest(&Arc::new(raw), &info);
        worker.learn_lid("167650256810092", "4917663430455");
        worker.store_message(incoming("m", 1), None, None);
        assert_eq!(reactions(&worker, "m"), [(PEER.into(), "👍".into())]);
    }

    #[test]
    fn an_encrypted_reaction_waits_for_the_secret_of_its_message() {
        let secret = [0x42u8; 32];
        let reactor = "12025550999@s.whatsapp.net";
        let (payload, iv) = whatsapp_rust::wacore::reaction::encrypt_reaction_with_secret(
            "🏆",
            1_700_000_000_123,
            &secret,
            "photo",
            PEER,
            reactor,
        )
        .expect("encrypt");
        let (mut worker, _events, _inbox, _wa) = worker();
        let raw = {
            let mut proto = wa::Message::default();
            proto.enc_reaction_message = MessageField::some({
                let mut proto = wa::message::EncReactionMessage::default();
                proto.target_message_key = MessageField::some({
                    let mut proto = wa::MessageKey::default();
                    proto.remote_jid = Some(PEER.into());
                    proto.from_me = Some(false);
                    proto.id = Some("photo".into());
                    proto.participant = Some(PEER.into());
                    proto
                });
                proto.enc_payload = Some(payload);
                proto.enc_iv = Some(iv.to_vec());
                proto
            });
            proto
        };
        let info = MessageInfo {
            source: MessageSource {
                chat: PEER.parse().unwrap(),
                sender: reactor.parse().unwrap(),
                ..Default::default()
            },
            timestamp: whatsapp_rust::wacore::time::from_secs(20).unwrap(),
            ..Default::default()
        };
        worker.ingest(&Arc::new(raw), &info);
        let parent = {
            let mut proto = wa::Message::default();
            proto.conversation = Some("caption".into());
            proto.message_context_info = MessageField::some({
                let mut proto = wa::MessageContextInfo::default();
                proto.message_secret = Some(secret.to_vec());
                proto
            });
            proto
        };
        worker.store_message(incoming("photo", 10), Some(parent.encode_to_vec()), None);
        assert_eq!(reactions(&worker, "photo"), [(reactor.into(), "🏆".into())]);
    }
}

#[cfg(test)]
mod chat_removal_tests {
    use super::*;
    use crate::model::{Content, Delivery};

    const CHAT: &str = "4915700000001@s.whatsapp.net";

    /// An incoming confirmation survives failed cleanup and recovers offline.
    #[tokio::test]
    async fn incoming_confirmation_is_recoverable_after_storage_failure() {
        let (mut worker, events, _, _) = receipt_tests::worker();
        worker.apply_history(history(CHAT, &[100, 200]), true);
        crate::archive::tests::set_message_deletion_failure(&worker.archive, true);
        worker
            .handle_wa_event(Arc::new(wa_events::Event::DeleteMessageForMeUpdate(
                wa_events::DeleteMessageForMeUpdate::builder()
                    .chat_jid(CHAT.parse().unwrap())
                    .message_id("m100".to_owned())
                    .from_me(true)
                    .timestamp(std::time::SystemTime::now().into())
                    .action(Box::default())
                    .from_full_sync(false)
                    .build(),
            )))
            .await;
        assert!(
            events
                .try_iter()
                .any(|event| matches!(event, Event::Error(_)))
        );
        assert_eq!(
            worker
                .archive
                .confirmed_message_removals(&worker.me())
                .unwrap()
                .len(),
            1
        );
        assert!(
            worker
                .archive
                .pending_message_removals(&worker.me())
                .unwrap()
                .is_empty()
        );
        assert!(worker.archive.message_removed(CHAT, "m100").unwrap());
        worker.apply_history(history(CHAT, &[100, 200]), false);
        crate::archive::tests::set_message_deletion_failure(&worker.archive, false);
        assert!(worker.client.is_none());
        worker.retry_confirmed_message_removals();
        assert_eq!(stored(&worker, CHAT), ["m200"]);
        assert!(
            worker
                .archive
                .confirmed_message_removals(&worker.me())
                .unwrap()
                .is_empty()
        );
        assert!(worker.archive.message_removed(CHAT, "m100").unwrap());
    }

    /// A callback from the old bot stays fenced even when delivered after unlink.
    #[tokio::test]
    async fn stale_incoming_deletion_does_not_repopulate_an_unlinked_archive() {
        let (mut worker, events, _, _) = receipt_tests::worker();
        let (sender, mut inbox) = mpsc::unbounded_channel();
        let old = UiEvents {
            sender: sender.clone(),
            generation: 0,
        };
        worker.privacy_generation = 1;
        worker.archive.clear().unwrap();
        let deletion = |id: &str| {
            Arc::new(wa_events::Event::DeleteMessageForMeUpdate(
                wa_events::DeleteMessageForMeUpdate::builder()
                    .chat_jid(CHAT.parse().unwrap())
                    .message_id(id.to_owned())
                    .from_me(true)
                    .timestamp(std::time::SystemTime::now().into())
                    .action(Box::default())
                    .from_full_sync(false)
                    .build(),
            ))
        };
        wa_events::EventHandler::handle_event(&old, deletion("m100"));
        let RuntimeEvent::MessageRemoval { generation, event } = inbox.try_recv().unwrap() else {
            panic!("deletion must be session tagged");
        };
        worker.handle_session_deletion(generation, event).await;
        assert!(!worker.archive.message_removed(CHAT, "m100").unwrap());
        assert!(
            events
                .try_iter()
                .all(|event| !matches!(event, Event::MessageDeleted { .. }))
        );
        worker.apply_history(history(CHAT, &[100, 200]), true);
        assert_eq!(stored(&worker, CHAT), ["m100", "m200"]);
        let current = UiEvents {
            sender,
            generation: 1,
        };
        wa_events::EventHandler::handle_event(&current, deletion("m200"));
        let RuntimeEvent::MessageRemoval { generation, event } = inbox.try_recv().unwrap() else {
            panic!("deletion must be session tagged");
        };
        worker.handle_session_deletion(generation, event).await;
        assert_eq!(stored(&worker, CHAT), ["m100"]);
        assert!(worker.archive.message_removed(CHAT, "m200").unwrap());
    }

    /// Mapping discovered after canonical acceptance also removes the old copy.
    #[tokio::test]
    async fn deletion_before_mapping_reconciles_the_original_copy() {
        let (mut worker, events, _, _) = receipt_tests::worker();
        worker.apply_history(history(CHAT, &[100, 200]), true);
        worker
            .archive
            .insert_message(
                &crate::archive::tests::message("9@lid", "m100", 100, true),
                None,
            )
            .unwrap();
        worker
            .archive
            .queue_message_removal(&worker.me(), "9@lid", "m100")
            .unwrap();
        worker
            .handle_wa_event(Arc::new(wa_events::Event::DeleteMessageForMeUpdate(
                wa_events::DeleteMessageForMeUpdate::builder()
                    .chat_jid(CHAT.parse().unwrap())
                    .message_id("m100".to_owned())
                    .from_me(true)
                    .timestamp(std::time::SystemTime::now().into())
                    .action(Box::default())
                    .from_full_sync(false)
                    .build(),
            )))
            .await;
        events.try_iter().for_each(drop);
        worker.learn_lid("9", CHAT.split('@').next().unwrap());
        assert!(worker.archive.message("9@lid", "m100").unwrap().is_none());
        assert!(worker.archive.message_removed("9@lid", "m100").unwrap());
        assert!(
            worker
                .archive
                .pending_message_removals(&worker.me())
                .unwrap()
                .is_empty()
        );
        assert_eq!(stored(&worker, CHAT), ["m200"]);
        assert!(events.try_iter().any(|event| matches!(event, Event::MessageDeleted { chat, id } if chat == "9@lid" && id == "m100")));
    }

    /// Incoming acceptance cleans every alias even after the local reply is lost.
    #[tokio::test]
    async fn incoming_deletion_reconciles_all_known_aliases() {
        for source in [CHAT, "9@lid"] {
            let (mut worker, events, _, _) = receipt_tests::worker();
            worker.apply_history(history(CHAT, &[100, 200]), true);
            for lid in ["9@lid", "10@lid"] {
                worker
                    .archive
                    .insert_message(
                        &crate::archive::tests::message(lid, "m100", 100, true),
                        None,
                    )
                    .unwrap();
                worker
                    .archive
                    .queue_message_removal(&worker.me(), lid, "m100")
                    .unwrap();
                worker
                    .handle_command(Command::MessageDeletedForMe {
                        generation: 0,
                        chat: lid.into(),
                        id: "m100".into(),
                        outcome: MessageRemovalOutcome::Uncertain,
                    })
                    .await;
                worker.learn_lid(
                    lid.split('@').next().unwrap(),
                    CHAT.split('@').next().unwrap(),
                );
            }
            events.try_iter().for_each(drop);
            worker
                .handle_wa_event(Arc::new(wa_events::Event::DeleteMessageForMeUpdate(
                    wa_events::DeleteMessageForMeUpdate::builder()
                        .chat_jid(source.parse().unwrap())
                        .message_id("m100".to_owned())
                        .from_me(true)
                        .timestamp(std::time::SystemTime::now().into())
                        .action(Box::default())
                        .from_full_sync(false)
                        .build(),
                )))
                .await;
            assert!(
                worker
                    .archive
                    .pending_message_removals(&worker.me())
                    .unwrap()
                    .is_empty()
            );
            let notified: HashSet<_> = events
                .try_iter()
                .filter_map(|event| match event {
                    Event::MessageDeleted { chat, id } if id == "m100" => Some(chat),
                    _ => None,
                })
                .collect();
            for chat in [CHAT, "9@lid", "10@lid"] {
                assert!(worker.archive.message(chat, "m100").unwrap().is_none());
                assert!(worker.archive.message_removed(chat, "m100").unwrap());
                assert!(notified.contains(chat));
                worker
                    .archive
                    .insert_message(
                        &crate::archive::tests::message(chat, "m100", 100, true),
                        None,
                    )
                    .unwrap();
                assert!(worker.archive.message(chat, "m100").unwrap().is_none());
            }
            assert_eq!(stored(&worker, CHAT), ["m200"]);
        }
    }

    /// Lost acknowledgements retain intent and rows until acceptance is known.
    #[tokio::test]
    async fn uncertain_deletion_remains_recoverable() {
        use whatsapp_rust::AppStateError;
        let (mut worker, events, _, _) = receipt_tests::worker();
        worker.apply_history(history(CHAT, &[100]), true);
        worker
            .archive
            .queue_message_removal(&worker.me(), CHAT, "m100")
            .unwrap();
        for error in [
            AppStateError::NotConnected,
            AppStateError::Internal(anyhow::anyhow!("fixture lost acknowledgement")),
            AppStateError::InvalidRequest("no app state sync key available".into()),
        ] {
            worker
                .message_removals_in_flight
                .insert((CHAT.into(), "m100".into()));
            let outcome = message_removal_outcome(Err(error));
            assert_eq!(outcome, MessageRemovalOutcome::Uncertain);
            worker
                .handle_command(Command::MessageDeletedForMe {
                    generation: 0,
                    chat: CHAT.into(),
                    id: "m100".into(),
                    outcome,
                })
                .await;
            assert!(!worker.message_removal_in_flight(CHAT, "m100"));
            assert_eq!(stored(&worker, CHAT), ["m100"]);
            assert!(!worker.archive.message_removed(CHAT, "m100").unwrap());
            worker.retry_message_removals();
            assert_eq!(
                worker
                    .archive
                    .pending_message_removals(&worker.me())
                    .unwrap()
                    .len(),
                1
            );
        }
        assert!(
            events
                .try_iter()
                .all(|event| !matches!(event, Event::MessageDeleted { .. }))
        );
        let outcome = message_removal_outcome(Ok(()));
        assert_eq!(outcome, MessageRemovalOutcome::Accepted);
        worker
            .handle_command(Command::MessageDeletedForMe {
                generation: 0,
                chat: CHAT.into(),
                id: "m100".into(),
                outcome,
            })
            .await;
        assert!(
            worker
                .archive
                .pending_message_removals(&worker.me())
                .unwrap()
                .is_empty()
        );
        assert!(stored(&worker, CHAT).is_empty());
        assert!(worker.archive.message_removed(CHAT, "m100").unwrap());
    }

    #[tokio::test]
    async fn deletion_resolves_a_mapping_known_only_to_the_protocol_library() {
        let directory = tempfile::tempdir().unwrap();
        let store = whatsapp_rust::store::SqliteStore::open(
            &directory.path().join("fixture.db").to_string_lossy(),
        )
        .await
        .unwrap();
        // Build only: never run or spawn this bot, so the fixture stays offline.
        let bot = Bot::builder().with_backend(store).build().await.unwrap();
        let client = bot.client();
        client
            .add_lid_pn_mapping(
                "100000000001",
                "4915700000001",
                whatsapp_rust::wacore::types::lid_pn::LearningSource::Usync,
            )
            .await
            .unwrap();
        let (mut worker, _, _, _) = receipt_tests::worker();
        worker.client = Some(client);
        assert!(worker.lid_to_pn.is_empty());
        assert_eq!(
            worker
                .canonical_sync_chat(&"100000000001@lid".parse().unwrap())
                .await,
            CHAT
        );
        assert_eq!(
            worker.lid_to_pn.get("100000000001").map(String::as_str),
            Some("4915700000001")
        );
    }

    #[tokio::test]
    async fn live_deletion_with_an_empty_range_removes_the_cached_chat() {
        for timestamp in [None, Some(0), Some(200), Some(200_000_000_000)] {
            let (mut worker, events, _, _) = receipt_tests::worker();
            worker.apply_history(history(CHAT, &[100, 200]), true);
            while events.try_recv().is_ok() {}
            worker
                .handle_wa_event(Arc::new(wa_events::Event::DeleteChatUpdate(
                    wa_events::DeleteChatUpdate::builder()
                        .jid(CHAT.parse().unwrap())
                        .delete_media(false)
                        .timestamp((std::time::UNIX_EPOCH + Duration::from_secs(300)).into())
                        .action(Box::new({
                            let mut proto = wa::sync_action_value::DeleteChatAction::default();
                            proto.message_range = Some({
                                let mut proto =
                                    wa::sync_action_value::SyncActionMessageRange::default();
                                proto.last_message_timestamp = timestamp;
                                proto
                            })
                            .into();
                            proto
                        }))
                        .from_full_sync(false)
                        .build(),
                )))
                .await;
            assert!(worker.archive.chat(CHAT).unwrap().is_none());
            assert!(
                std::iter::from_fn(|| events.try_recv().ok())
                    .any(|event| matches!(event, Event::ChatRemoved { chat } if chat == CHAT))
            );
        }
    }

    /// A history chunk holding one chat with a message at each timestamp.
    pub(super) fn history(chat: &str, timestamps: &[i64]) -> ParsedHistory {
        ParsedHistory {
            chats: vec![ParsedChat {
                diagnostics: HistoryDiagnostics::default(),
                id: chat.to_owned(),
                name: Some("Somebody".into()),
                unread: None,
                marked_unread: None,
                archived: None,
                pinned_at: None,
                muted_until: None,
                locked: None,
                ephemeral_expiration: None,
                ephemeral_setting_timestamp: None,
                last_activity: timestamps.iter().copied().max().unwrap_or(0),
                pn_jid: None,
                lid_jid: None,
                more_on_phone: None,
                messages: timestamps
                    .iter()
                    .map(|&at| ParsedMessage {
                        id: format!("m{at}"),
                        sender: Some(chat.to_owned()),
                        from_me: false,
                        push_name: None,
                        timestamp: at,
                        history_order: None,
                        edited: false,
                        content: Content::text(format!("sent at {at}")),
                        status: Delivery::None,
                        quoted: None,
                        reactions: Vec::new(),
                        mentions: Vec::new(),
                        forwarded: false,
                        thumbnail: None,
                        raw: Vec::new(),
                        poll_secret: None,
                        poll_votes: Vec::new(),
                        receipts: Vec::new(),
                        sealed_edit: None,
                    })
                    .collect(),
                revoked: Vec::new(),
                clips: Vec::new(),
                poll_updates: Vec::new(),
                reactions: Vec::new(),
                pins: Vec::new(),
            }],
            push_names: Vec::new(),
            lids: Vec::new(),
            stickers: Vec::new(),
        }
    }

    fn stored(worker: &Worker, chat: &str) -> Vec<String> {
        worker
            .archive
            .messages(chat, None, 50)
            .expect("messages")
            .into_iter()
            .map(|message| message.id)
            .collect()
    }

    #[test]
    fn history_that_arrives_after_a_deletion_does_not_bring_the_chat_back() {
        let (mut worker, _events, _, _) = receipt_tests::worker();
        worker.apply_history(history(CHAT, &[100, 200]), true);
        assert!(worker.archive.chat(CHAT).expect("chat").is_some());

        worker.remove_chat(CHAT, 200, false);
        // A phone-history page requested before the deletion lands afterwards.
        worker.apply_history(history(CHAT, &[50, 150, 200]), false);
        assert!(worker.archive.chat(CHAT).expect("chat").is_none());

        // A message sent after the deletion reopens the chat, as on the phone.
        worker.apply_history(history(CHAT, &[300]), false);
        assert_eq!(stored(&worker, CHAT), ["m300"]);
    }

    #[test]
    fn history_that_arrives_after_a_clear_does_not_refill_the_chat() {
        let (mut worker, _events, _, _) = receipt_tests::worker();
        worker.apply_history(history(CHAT, &[100, 200]), true);

        assert!(worker.empty_chat(CHAT, 200, false));
        worker.apply_history(history(CHAT, &[150]), false);
        assert!(worker.archive.chat(CHAT).expect("chat").is_some());
        assert!(stored(&worker, CHAT).is_empty());

        worker.apply_history(history(CHAT, &[300]), false);
        assert_eq!(stored(&worker, CHAT), ["m300"]);
    }

    #[tokio::test]
    /// Offline and refused account writes keep the row; acceptance blocks replay.
    async fn individual_deletion_waits_for_sync_and_blocks_replay() {
        let (mut worker, events, _, _) = receipt_tests::worker();
        worker.apply_history(history(CHAT, &[100, 200]), true);
        worker
            .handle_command(Command::DeleteLocal {
                chat: CHAT.into(),
                id: "m100".into(),
            })
            .await;
        assert_eq!(stored(&worker, CHAT), ["m100", "m200"]);
        assert!(
            events
                .try_iter()
                .any(|event| matches!(event, Event::Error(_)))
        );
        worker
            .handle_command(Command::MessageDeletedForMe {
                generation: 0,
                chat: CHAT.into(),
                id: "m100".into(),
                outcome: MessageRemovalOutcome::Uncertain,
            })
            .await;
        assert_eq!(stored(&worker, CHAT), ["m100", "m200"]);
        worker
            .handle_command(Command::MessageDeletedForMe {
                generation: 0,
                chat: CHAT.into(),
                id: "m100".into(),
                outcome: MessageRemovalOutcome::Accepted,
            })
            .await;
        assert_eq!(stored(&worker, CHAT), ["m200"]);
        assert!(
            events
                .try_iter()
                .any(|event| matches!(event, Event::MessageDeleted { id, .. } if id == "m100"))
        );
        worker.apply_history(history(CHAT, &[100, 200]), false);
        worker.store_message(
            crate::archive::tests::message(CHAT, "m100", 100, false),
            None,
            None,
        );
        assert_eq!(stored(&worker, CHAT), ["m200"]);
    }

    #[tokio::test]
    /// An incoming deletion may precede the phone's first history page.
    async fn phone_deletion_before_history_prevents_message_import() {
        let (mut worker, _events, _, _) = receipt_tests::worker();
        worker
            .handle_wa_event(Arc::new(
                whatsapp_rust::types::events::Event::DeleteMessageForMeUpdate(
                    whatsapp_rust::types::events::DeleteMessageForMeUpdate::builder()
                        .chat_jid(CHAT.parse().unwrap())
                        .message_id("m100".to_owned())
                        .from_me(false)
                        .timestamp(std::time::SystemTime::now().into())
                        .action(Box::default())
                        .from_full_sync(true)
                        .build(),
                ),
            ))
            .await;
        worker.apply_history(history(CHAT, &[100, 200]), true);
        assert_eq!(stored(&worker, CHAT), ["m200"]);
    }

    /// Late mappings remove an already visible canonical copy and notify the UI.
    #[test]
    fn privacy_mapping_updates_visible_deletions() {
        let (mut worker, events, _, _) = receipt_tests::worker();
        worker.apply_history(history(CHAT, &[100, 200]), true);
        worker.archive.delete_message("9@lid", "m100").unwrap();
        worker.learn_lid("9", CHAT.split('@').next().unwrap());
        assert_eq!(stored(&worker, CHAT), ["m200"]);
        assert!(events.try_iter().any(|event|
            matches!(event, Event::MessageDeleted { chat, id } if chat == CHAT && id == "m100")));
    }

    /// A completion from an unlinked account cannot delete the new account's row.
    #[tokio::test]
    async fn stale_deletion_completion_preserves_the_current_account() {
        let (mut worker, _events, _, _) = receipt_tests::worker();
        worker.apply_history(history(CHAT, &[100]), true);
        worker
            .archive
            .queue_message_removal(&worker.me(), CHAT, "m100")
            .unwrap();
        worker.privacy_generation = 1;
        worker
            .handle_command(Command::MessageDeletedForMe {
                generation: 0,
                chat: CHAT.into(),
                id: "m100".into(),
                outcome: MessageRemovalOutcome::Accepted,
            })
            .await;
        assert_eq!(stored(&worker, CHAT), ["m100"]);
        assert_eq!(
            worker
                .archive
                .pending_message_removals(&worker.me())
                .unwrap()
                .len(),
            1
        );
        worker.retry_message_removals();
        assert_eq!(
            worker
                .archive
                .pending_message_removals(&worker.me())
                .unwrap()
                .len(),
            1,
            "offline recovery keeps the intent"
        );
        worker
            .handle_command(Command::MessageDeletedForMe {
                generation: 1,
                chat: CHAT.into(),
                id: "m100".into(),
                outcome: MessageRemovalOutcome::Accepted,
            })
            .await;
        assert!(stored(&worker, CHAT).is_empty());
        assert!(
            worker
                .archive
                .pending_message_removals(&worker.me())
                .unwrap()
                .is_empty()
        );
    }

    /// A mapping learned during a request updates both identities on acceptance.
    #[tokio::test]
    async fn in_flight_deletion_follows_a_new_privacy_mapping() {
        let (mut worker, events, _, _) = receipt_tests::worker();
        let lid = "9@lid";
        worker.apply_history(history(CHAT, &[100, 200]), true);
        worker
            .archive
            .insert_message(
                &crate::archive::tests::message(lid, "m100", 100, true),
                None,
            )
            .unwrap();
        worker
            .archive
            .queue_message_removal(&worker.me(), lid, "m100")
            .unwrap();
        worker
            .message_removals_in_flight
            .insert((lid.into(), "m100".into()));
        worker.learn_lid("9", CHAT.split('@').next().unwrap());
        assert!(worker.message_removal_in_flight(CHAT, "m100"));
        events.try_iter().for_each(drop);
        worker
            .handle_command(Command::DeleteLocal {
                chat: CHAT.into(),
                id: "m100".into(),
            })
            .await;
        worker.retry_message_removals();
        assert!(
            events.try_iter().next().is_none(),
            "overlapping attempts never reach the client"
        );
        worker
            .handle_command(Command::MessageDeletedForMe {
                generation: 0,
                chat: lid.into(),
                id: "m100".into(),
                outcome: MessageRemovalOutcome::Accepted,
            })
            .await;
        assert!(!worker.message_removal_in_flight(CHAT, "m100"));
        assert!(worker.archive.message_removed(CHAT, "m100").unwrap());
        assert!(worker.archive.message_removed(lid, "m100").unwrap());
        assert_eq!(stored(&worker, CHAT), ["m200"]);
        assert!(
            worker
                .archive
                .pending_message_removals(&worker.me())
                .unwrap()
                .is_empty()
        );
        assert!(events.try_iter().any(|event|
            matches!(event, Event::MessageDeleted { chat, id } if chat == CHAT && id == "m100")));
        worker.apply_history(history(CHAT, &[100, 200]), false);
        assert_eq!(stored(&worker, CHAT), ["m200"]);
    }

    /// Recovery never submits a surviving request belonging to another account.
    #[test]
    fn deletion_recovery_skips_other_accounts() {
        let (mut worker, events, _, _) = receipt_tests::worker();
        worker.apply_history(history(CHAT, &[100]), true);
        worker
            .archive
            .queue_message_removal("old-account", CHAT, "m100")
            .unwrap();
        events.try_iter().for_each(drop);
        worker.retry_message_removals();
        assert!(events.try_iter().next().is_none());
        assert_eq!(
            worker
                .archive
                .pending_message_removals("old-account")
                .unwrap()
                .len(),
            1
        );
        assert_eq!(stored(&worker, CHAT), ["m100"]);
    }

    /// Own group messages use the account deletion event without a participant key.
    #[tokio::test]
    async fn own_group_deletion_keeps_other_messages_and_other_chats() {
        let (mut worker, events, _, _) = receipt_tests::worker();
        let group = "1-1@g.us";
        let other = "2-2@g.us";
        for chat in [group, other] {
            worker.archive.ensure_chat(chat, "Synthetic group").unwrap();
            for id in ["own", "kept"] {
                worker
                    .archive
                    .insert_message(&crate::archive::tests::message(chat, id, 100, true), None)
                    .unwrap();
            }
        }
        worker
            .handle_wa_event(Arc::new(wa_events::Event::DeleteMessageForMeUpdate(
                wa_events::DeleteMessageForMeUpdate::builder()
                    .chat_jid(group.parse().unwrap())
                    .message_id("own".to_owned())
                    .from_me(true)
                    .timestamp(std::time::SystemTime::now().into())
                    .action(Box::default())
                    .from_full_sync(false)
                    .build(),
            )))
            .await;
        assert_eq!(stored(&worker, group), ["kept"]);
        assert_eq!(stored(&worker, other).len(), 2);
        assert!(events.try_iter().any(|event|
            matches!(event, Event::MessageDeleted { chat, id } if chat == group && id == "own")));
        worker.store_message(
            crate::archive::tests::message(group, "own", 100, true),
            None,
            None,
        );
        assert_eq!(stored(&worker, group), ["kept"]);
    }

    /// A deletion that keeps the newer messages takes older pictures,
    /// documents and links with it, so the info panel is told; a deletion
    /// that removes the chat closes the panel through `ChatRemoved`.
    #[test]
    fn a_partial_deletion_reports_a_media_change() {
        let (mut worker, events, _, _) = receipt_tests::worker();
        worker.archive.ensure_chat(CHAT, "Somebody").expect("chat");
        let mut photo = crate::archive::tests::message(CHAT, "photo", 100, false);
        photo.content = crate::model::Content::Image {
            motion: None,
            caption: None,
            media: crate::model::Media {
                album: None,
                mime: "image/jpeg".into(),
                size: 1,
                width: None,
                height: None,
                path: None,
                state: Default::default(),
            },
        };
        worker.store_message(photo, None, None);
        let text = crate::archive::tests::message(CHAT, "text", 300, false);
        worker.store_message(text, None, None);
        let changes = |events: &std::sync::mpsc::Receiver<Event>| -> Vec<String> {
            events
                .try_iter()
                .filter_map(|event| match event {
                    Event::ChatSharedChanged(chat) => Some(chat),
                    _ => None,
                })
                .collect()
        };
        changes(&events);
        worker.remove_chat(CHAT, 200, false);
        assert_eq!(stored(&worker, CHAT), ["text"], "the newer text survives");
        assert_eq!(changes(&events), [CHAT], "the deleted picture is reported");
        worker.remove_chat(CHAT, i64::MAX, false);
        assert!(worker.archive.chat(CHAT).expect("chat").is_none());
        assert!(
            events
                .try_iter()
                .any(|event| matches!(event, Event::ChatRemoved { chat } if chat == CHAT)),
            "a full removal closes the panel through ChatRemoved"
        );
    }

    #[test]
    fn a_live_message_older_than_the_deletion_is_dropped() {
        let (mut worker, _events, _, _) = receipt_tests::worker();
        worker.archive.ensure_chat(CHAT, "Somebody").expect("chat");
        worker.remove_chat(CHAT, 200, false);

        let late = crate::archive::tests::message(CHAT, "late", 150, false);
        worker.store_message(late, None, None);
        assert!(worker.archive.chat(CHAT).expect("chat").is_none());

        let fresh = crate::archive::tests::message(CHAT, "fresh", 250, false);
        worker.store_message(fresh, None, None);
        assert_eq!(stored(&worker, CHAT), ["fresh"]);
    }

    #[test]
    fn a_deleted_group_is_no_longer_asked_for_metadata() {
        let (mut worker, _events, _, _) = receipt_tests::worker();
        let group = "1-1@g.us";
        worker.archive.ensure_chat(group, "Group").expect("chat");
        worker.request_group_info(group, false);
        assert_eq!(worker.group_info_queue.len(), 1);

        worker.remove_chat(group, 100, false);
        assert!(worker.group_info_queue.is_empty());
        assert!(!worker.group_info_requested.contains(group));

        // A request already out fails afterwards and schedules a retry; once
        // due, the group is gone and nothing is asked again.
        worker.handle_failed_group(group.to_owned(), false);
        for (due, _) in &mut worker.group_info_retry {
            *due = Instant::now();
        }
        worker.pump_group_info();
        assert!(worker.group_info_queue.is_empty());
        assert!(worker.group_info_retry.is_empty());
    }

    #[tokio::test]
    async fn a_chat_is_deleted_here_only_after_the_phone_deleted_it() {
        let (mut worker, events, _, _) = receipt_tests::worker();
        worker.archive.ensure_chat(CHAT, "Somebody").expect("chat");

        // Without a phone connection nothing is deleted anywhere.
        worker
            .handle_command(Command::DeleteChat(CHAT.into()))
            .await;
        assert!(worker.archive.chat(CHAT).expect("chat").is_some());
        assert!(
            events
                .try_iter()
                .any(|event| matches!(event, Event::Error(_)))
        );

        worker
            .handle_command(Command::ChatDeleted {
                chat: CHAT.into(),
                deleted: false,
                through: 200,
            })
            .await;
        assert!(worker.archive.chat(CHAT).expect("chat").is_some());
        assert!(
            events
                .try_iter()
                .any(|event| matches!(event, Event::Error(_)))
        );

        worker
            .handle_command(Command::ChatDeleted {
                chat: CHAT.into(),
                deleted: true,
                through: 200,
            })
            .await;
        assert!(worker.archive.chat(CHAT).expect("chat").is_none());
        assert!(
            events
                .try_iter()
                .any(|event| matches!(event, Event::ChatRemoved { chat } if chat == CHAT))
        );
    }

    #[tokio::test]
    async fn a_chat_is_cleared_here_only_after_the_phone_cleared_it() {
        let (mut worker, events, _, _) = receipt_tests::worker();
        worker.apply_history(history(CHAT, &[100, 200]), true);

        // Without a phone connection nothing is cleared anywhere.
        worker.handle_command(Command::ClearChat(CHAT.into())).await;
        assert_eq!(stored(&worker, CHAT), ["m100", "m200"]);
        assert!(
            events
                .try_iter()
                .any(|event| matches!(event, Event::Error(_)))
        );

        worker
            .handle_command(Command::ChatCleared {
                chat: CHAT.into(),
                cleared: false,
                through: 200,
            })
            .await;
        assert_eq!(stored(&worker, CHAT), ["m100", "m200"]);
        assert!(
            events
                .try_iter()
                .any(|event| matches!(event, Event::Error(_)))
        );

        worker
            .handle_command(Command::ChatCleared {
                chat: CHAT.into(),
                cleared: true,
                through: 200,
            })
            .await;
        // The chat stays listed; only its messages go.
        assert!(worker.archive.chat(CHAT).expect("chat").is_some());
        assert!(stored(&worker, CHAT).is_empty());
        assert!(
            events
                .try_iter()
                .any(|event| matches!(event, Event::ChatCleared { chat, .. } if chat == CHAT))
        );
    }

    /// A boundary that cannot be read is not `now`: clearing the phone through
    /// a guessed time would leave the two sides apart while the dialog said
    /// they matched. An archive with no messages still has one.
    #[test]
    fn a_boundary_that_cannot_be_read_is_not_guessed() {
        assert_eq!(
            clear_boundary(Err(rusqlite::Error::QueryReturnedNoRows)),
            None,
            "no boundary means nothing is cleared anywhere"
        );
        let empty: Vec<Message> = Vec::new();
        assert!(
            clear_boundary(Ok(empty)).is_some(),
            "an empty archive clears through now"
        );
    }

    #[tokio::test]
    async fn a_late_star_event_does_not_cancel_the_inflight_request() {
        let (mut worker, _events, _, _) = receipt_tests::worker();
        const CHAT: &str = "1@s.whatsapp.net";
        worker.archive.ensure_chat(CHAT, "Ada").unwrap();
        let now = whatsapp_rust::wacore::time::now_utc();
        worker
            .archive
            .star(CHAT, "m1", now.timestamp() + 10_000)
            .unwrap();
        worker.star_generation.insert((CHAT.into(), "m1".into()), 3);
        let stale = wa_events::StarUpdate::builder()
            .chat_jid(CHAT.parse().unwrap())
            .message_id("m1".into())
            .from_me(false)
            .timestamp(now)
            .from_full_sync(false)
            .action(Box::new({
                let mut action = wa::sync_action_value::StarAction::default();
                action.starred = Some(false);
                action
            }))
            .build();
        worker.star_update(&stale);
        assert_eq!(
            worker
                .star_generation
                .get(&(CHAT.into(), "m1".into()))
                .copied(),
            Some(3)
        );
        assert!(worker.archive.starred_ids(CHAT).unwrap().contains("m1"));
    }

    #[tokio::test]
    async fn a_star_answer_from_the_previous_account_does_not_write() {
        let (mut worker, _events, _, _) = receipt_tests::worker();
        const CHAT: &str = "1@s.whatsapp.net";
        worker.archive.ensure_chat(CHAT, "Ada").unwrap();
        // The request that is about to answer belongs to the account that
        // left. The next account starts its own generations at one, so the
        // generation alone cannot tell the two apart.
        worker.star_session = 0;
        worker.star_generation.insert((CHAT.into(), "m1".into()), 1);
        worker.star_session = 1;
        worker
            .handle_command(Command::Starred {
                chat: CHAT.into(),
                message: "m1".into(),
                starred: true,
                generation: 1,
                session: 0,
                result: Ok(()),
            })
            .await;
        assert!(worker.archive.starred_ids(CHAT).unwrap().is_empty());
    }

    /// The same for a pin: an answer that outlives a logout belongs to the
    /// account that left.
    #[tokio::test]
    async fn a_pin_answer_from_the_previous_account_does_not_write() {
        let (mut worker, _events, _, _) = receipt_tests::worker();
        const CHAT: &str = "1@s.whatsapp.net";
        worker.archive.ensure_chat(CHAT, "Ada").unwrap();
        worker
            .archive
            .insert_message(
                &crate::archive::tests::message(CHAT, "m1", 100, false),
                None,
            )
            .unwrap();
        worker.star_session = 1;
        worker.pin_generation.insert((CHAT.into(), "m1".into()), 1);
        worker
            .handle_command(Command::MessagePinned {
                chat: CHAT.into(),
                message: "m1".into(),
                pinned: true,
                at: 100,
                generation: 1,
                session: 0,
                expires_at: i64::MAX,
                result: Ok(()),
            })
            .await;
        assert!(worker.archive.pinned_ids(CHAT, 200).unwrap().is_empty());
    }
}

#[cfg(test)]
mod saved_download_tests {
    use super::*;
    use crate::model::{InteractiveCard, MediaState, Motion};

    #[derive(Clone, Copy, Debug, PartialEq)]
    enum Target {
        Document,
        Carousel,
        Motion,
    }

    /// Synthetic completion targets exercise the shared callback and the separate clip callback.
    fn fixture(target: Target) -> Message {
        let mut message = receipt_tests::own_message("saved-file", 100);
        message.edited = true;
        let media = Media {
            mime: "application/octet-stream".into(),
            size: 7,
            width: None,
            height: None,
            album: None,
            path: None,
            state: MediaState::Idle,
        };
        message.content = match target {
            Target::Document => Content::Document {
                media,
                file_name: "fixture.bin".into(),
                caption: None,
                pages: None,
            },
            Target::Carousel => Content::Interactive {
                text: "Fixture".into(),
                card: Some(Box::new(InteractiveCard {
                    carousel: vec![InteractiveCard {
                        image: Some(media),
                        ..Default::default()
                    }],
                    ..Default::default()
                })),
            },
            Target::Motion => Content::Image {
                caption: None,
                media,
                motion: Some(Motion::default()),
            },
        };
        message
    }

    /// Completes through production routing, without any connection to WhatsApp.
    async fn finish(worker: &mut Worker, target: Target, result: Result<PathBuf, String>) {
        let message = fixture(target);
        if target == Target::Motion {
            worker
                .handle_command(Command::MotionDownloaded {
                    chat: message.chat,
                    id: message.id,
                    result,
                })
                .await;
        } else {
            worker.downloaded(
                message.chat,
                message.id,
                (target == Target::Carousel).then_some(0),
                result,
            );
        }
    }

    /// Returns the durable path of the exact attachment slot under test.
    fn stored_path(message: &Message, target: Target) -> Option<&Path> {
        match (&message.content, target) {
            (
                Content::Image {
                    motion: Some(motion),
                    ..
                },
                Target::Motion,
            ) => motion.path.as_deref(),
            (
                Content::Interactive {
                    card: Some(card), ..
                },
                Target::Carousel,
            ) => card.carousel.first()?.image.as_ref()?.path.as_deref(),
            (_, Target::Document) => message.content.media()?.path.as_deref(),
            _ => None,
        }
    }

    /// Extracts the existing interface completion event, not a test-only success signal.
    fn outcome(events: &std::sync::mpsc::Receiver<Event>) -> Result<PathBuf, String> {
        events
            .try_iter()
            .find_map(|event| match event {
                Event::Media { result, .. } | Event::Motion { result, .. } => Some(result),
                _ => None,
            })
            .expect("a completion event")
    }

    /// A write refusal is a failure; the verified bytes, edit and original keys survive.
    #[tokio::test]
    async fn completion_waits_for_the_archive_and_preserves_the_downloaded_file() {
        for target in [Target::Document, Target::Carousel, Target::Motion] {
            let (mut worker, events, _, _) = receipt_tests::worker();
            let file = tempfile::NamedTempFile::new().unwrap();
            std::fs::write(file.path(), b"fixture").unwrap();
            let message = fixture(target);
            worker
                .archive
                .insert_message(&message, Some(b"synthetic download keys"))
                .unwrap();
            crate::archive::tests::set_derived_update_failure(&worker.archive, true);
            finish(&mut worker, target, Err("fixture transfer failure".into())).await;
            assert_eq!(outcome(&events).unwrap_err(), "fixture transfer failure");
            if target != Target::Motion {
                worker.downloads.insert((
                    message.chat.clone(),
                    message.id.clone(),
                    (target == Target::Carousel).then_some(0),
                ));
            }
            finish(&mut worker, target, Ok(file.path().to_owned())).await;
            assert!(outcome(&events).unwrap_err().contains("file has been kept"));
            assert!(
                worker.downloads.is_empty(),
                "failed persistence releases the active transfer"
            );
            assert!(
                stored_path(
                    &worker
                        .archive
                        .message(&message.chat, &message.id)
                        .unwrap()
                        .unwrap(),
                    target
                )
                .is_none()
            );
            assert_eq!(std::fs::read(file.path()).unwrap(), b"fixture");
            crate::archive::tests::set_derived_update_failure(&worker.archive, false);
            finish(&mut worker, target, Ok(file.path().to_owned())).await;
            assert_eq!(outcome(&events).unwrap(), file.path());
            let stored = worker
                .archive
                .message(&message.chat, &message.id)
                .unwrap()
                .unwrap();
            assert_eq!(stored_path(&stored, target), Some(file.path()));
            assert!(stored.edited);
            assert_eq!(
                worker
                    .archive
                    .raw(&message.chat, &message.id)
                    .unwrap()
                    .as_deref(),
                Some(b"synthetic download keys".as_slice())
            );
        }
    }

    /// A late completion cannot claim success for a deleted or replaced attachment.
    #[tokio::test]
    async fn missing_and_replaced_rows_cannot_report_a_saved_download() {
        for target in [Target::Document, Target::Carousel, Target::Motion] {
            let (mut worker, events, _, _) = receipt_tests::worker();
            let file = tempfile::NamedTempFile::new().unwrap();
            finish(&mut worker, target, Ok(file.path().to_owned())).await;
            assert!(outcome(&events).is_err());
            let mut message = fixture(target);
            assert!(
                worker
                    .archive
                    .message(&message.chat, &message.id)
                    .unwrap()
                    .is_none()
            );
            message.content = Content::text("Replacement fixture");
            worker.archive.insert_message(&message, None).unwrap();
            finish(&mut worker, target, Ok(file.path().to_owned())).await;
            assert!(outcome(&events).is_err());
            assert_eq!(
                worker
                    .archive
                    .message(&message.chat, &message.id)
                    .unwrap()
                    .unwrap()
                    .content,
                message.content
            );
            assert!(file.path().is_file());
        }
    }

    /// A storage error does not enter the picker's retry/re-upload path or refill the same file.
    #[tokio::test]
    async fn picker_storage_failure_does_not_retry_the_network_download() {
        let (mut worker, events, mut commands, _) = receipt_tests::worker();
        let file = tempfile::NamedTempFile::new().unwrap();
        let mut message = fixture(Target::Document);
        message.content = Content::Sticker {
            media: message.content.media().unwrap().clone(),
            animated: false,
        };
        worker.archive.insert_message(&message, None).unwrap();
        let key = (message.chat.clone(), message.id.clone());
        worker.sticker_downloads.insert(key.clone());
        worker.sticker_clicked.insert(key.clone());
        worker
            .downloads
            .insert((message.chat.clone(), message.id.clone(), None));
        crate::archive::tests::set_derived_update_failure(&worker.archive, true);
        finish(&mut worker, Target::Document, Ok(file.path().to_owned())).await;
        assert!(outcome(&events).is_err());
        assert!(worker.sticker_download_failed.contains(&key));
        assert!(!worker.sticker_retried.contains(&key));
        assert!(worker.downloads.is_empty() && worker.sticker_downloads.is_empty());
        assert!(commands.try_recv().is_err());
        assert!(file.path().is_file());
    }
}

#[cfg(test)]
mod older_history_tests {
    use super::*;

    const CHAT: &str = "4915700000002@s.whatsapp.net";

    /// A phone-history request that has waited past the patience.
    fn waiting(worker: &mut Worker, explicit: bool) {
        asked(worker, explicit, false, 0, None);
        worker
            .pending_older
            .get_mut(CHAT)
            .expect("the request")
            .asked = Instant::now()
            .checked_sub(PHONE_PATIENCE * 2)
            .expect("the clock runs past the patience");
    }

    /// A phone-history request still within the patience, with the serial
    /// and the id it went out under.
    fn asked(
        worker: &mut Worker,
        explicit: bool,
        background: bool,
        serial: u64,
        request: Option<&str>,
    ) {
        worker.pending_older.insert(
            CHAT.to_owned(),
            OlderRequest {
                asked: Instant::now(),
                before: (100, "m100".into()),
                explicit,
                background,
                serial,
                request: request.map(str::to_owned),
            },
        );
    }

    /// The silent flags of the `OlderFetched` events, in order.
    fn silences(events: &[Event]) -> Vec<bool> {
        events
            .iter()
            .filter_map(|event| match event {
                Event::OlderFetched { chat, silent, .. } if chat == CHAT => Some(*silent),
                _ => None,
            })
            .collect()
    }

    fn pages(events: &[Event]) -> usize {
        events
            .iter()
            .filter(|event| matches!(event, Event::Messages { older: true, .. }))
            .count()
    }

    /// The prefetch is waiting on this chat, as `pump_prefetch` leaves it.
    fn prefetching(worker: &mut Worker) {
        worker.prefetch.configure(true, None);
        worker.prefetch.start_history(CHAT.to_owned());
    }

    /// A background page fills the archive and nothing else: no page for the
    /// view, no answer counted against the phone, and the prefetch's turn
    /// ends.
    #[test]
    fn a_background_page_stays_quiet() {
        let (mut worker, events, _, _) = receipt_tests::worker();
        prefetching(&mut worker);
        asked(&mut worker, false, true, 1, Some("bg"));
        let filed = worker.apply_history(chat_removal_tests::history(CHAT, &[50]), false);
        worker.answer_older(filed, Some("bg"));
        let events = drain(&events);
        assert_eq!(silences(&events), [true]);
        assert_eq!(pages(&events), 0, "the view does not move");
        assert!(worker.pending_older.is_empty());
        assert!(
            worker
                .prefetch
                .next_history(Instant::now(), false)
                .is_none(),
            "the turn ended and the gap before the next one began"
        );
    }

    /// The worker distinguishes a silent response from a send error: the next
    /// chat is ready at the ordinary gap, not an account-wide retry delay.
    #[test]
    fn a_background_timeout_does_not_back_off_other_chats() {
        let (mut worker, events, _, _) = receipt_tests::worker();
        prefetching(&mut worker);
        worker.prefetch.set_chats(&[
            Chat::new(CHAT.into(), "First".into()),
            Chat::new("other@s.whatsapp.net".into(), "Other".into()),
        ]);
        asked(&mut worker, false, true, 1, Some("bg"));
        worker.pending_older.get_mut(CHAT).unwrap().asked =
            Instant::now().checked_sub(PHONE_PATIENCE * 2).unwrap();
        worker.expire_older_requests();
        assert_eq!(silences(&drain(&events)), [true]);
        assert_eq!(
            worker
                .prefetch
                .next_history(Instant::now() + prefetch::HISTORY_GAP, false)
                .as_deref(),
            Some("other@s.whatsapp.net")
        );
    }

    /// A reader who scrolls up while a background request for the chat is
    /// out takes it over: its answer is their page.
    #[test]
    fn the_reader_takes_over_a_background_request() {
        let (mut worker, events, _, _) = receipt_tests::worker();
        prefetching(&mut worker);
        asked(&mut worker, false, true, 1, Some("bg"));
        assert!(!worker.fetch_older(CHAT.to_owned(), true, false));
        let request = &worker.pending_older[CHAT];
        assert!(!request.background && request.explicit);
        let filed = worker.apply_history(chat_removal_tests::history(CHAT, &[50]), false);
        worker.answer_older(filed, Some("bg"));
        let events = drain(&events);
        assert_eq!(silences(&events), [false]);
        assert_eq!(pages(&events), 1, "the reader's page shows");
    }

    /// The bug the ids fix: a background request times out, the reader asks
    /// for the same chat, and the background's late answer arrives first. It
    /// names the background request, so it stays quiet and the reader keeps
    /// waiting for their own answer, which then shows.
    #[test]
    fn a_late_background_answer_does_not_take_the_readers_request() {
        let (mut worker, events, _, _) = receipt_tests::worker();
        prefetching(&mut worker);
        asked(&mut worker, false, true, 1, Some("bg"));
        worker.pending_older.get_mut(CHAT).unwrap().asked =
            Instant::now().checked_sub(PHONE_PATIENCE * 2).unwrap();
        worker.expire_older_requests();
        let timeout = drain(&events);
        assert_eq!(silences(&timeout), [true], "a quiet timeout");
        assert_eq!(warnings(&timeout), 0);

        asked(&mut worker, true, false, 2, Some("reader"));
        let filed = worker.apply_history(chat_removal_tests::history(CHAT, &[50]), false);
        worker.answer_older(filed, Some("bg"));
        let early = drain(&events);
        assert_eq!(silences(&early), [true]);
        assert_eq!(pages(&early), 0);
        assert_eq!(
            worker.pending_older[CHAT].serial, 2,
            "the reader's request still waits"
        );

        let filed = worker.apply_history(chat_removal_tests::history(CHAT, &[40]), false);
        worker.answer_older(filed, Some("reader"));
        let events = drain(&events);
        assert_eq!(silences(&events), [false]);
        assert_eq!(pages(&events), 1);
        assert!(worker.pending_older.is_empty());
    }

    /// A matched late page breaks the timeout streak without completing or
    /// changing the newer background request for the same chat.
    #[test]
    fn a_late_background_page_resets_the_timeout_streak() {
        let (mut worker, events, _, _) = receipt_tests::worker();
        prefetching(&mut worker);
        worker
            .prefetch
            .set_chats(&[Chat::new(CHAT.into(), "First".into())]);
        asked(&mut worker, false, true, 1, Some("old"));
        worker.pending_older.get_mut(CHAT).unwrap().asked =
            Instant::now().checked_sub(PHONE_PATIENCE * 2).unwrap();
        worker.expire_older_requests();
        drain(&events);

        worker.prefetch.start_history(CHAT.into());
        asked(&mut worker, false, true, 2, Some("new"));
        let asked_at = worker.pending_older[CHAT].asked;
        let before = worker.pending_older[CHAT].before.clone();
        let filed = worker.apply_history(chat_removal_tests::history(CHAT, &[50]), false);
        worker.answer_older(filed, Some("old"));
        let late = drain(&events);
        assert_eq!(silences(&late), [true]);
        assert_eq!(pages(&late), 0);
        let pending = &worker.pending_older[CHAT];
        assert_eq!(pending.serial, 2);
        assert_eq!(pending.request.as_deref(), Some("new"));
        assert_eq!(pending.asked, asked_at);
        assert_eq!(pending.before, before);
        assert!(pending.background);
        assert!(
            worker
                .prefetch
                .next_history(Instant::now() + Duration::from_secs(900), false)
                .is_none()
        );

        for serial in 2..=3 {
            if serial == 3 {
                worker.prefetch.start_history(CHAT.into());
                asked(&mut worker, false, true, serial, Some("third"));
            }
            worker.pending_older.get_mut(CHAT).unwrap().asked =
                Instant::now().checked_sub(PHONE_PATIENCE * 2).unwrap();
            worker.expire_older_requests();
            drain(&events);
        }
        assert_eq!(
            worker
                .prefetch
                .next_history(Instant::now() + Duration::from_secs(900), false)
                .as_deref(),
            Some(CHAT),
            "only the two timeouts since the late page count"
        );
    }

    /// Without ids, answers are matched by chat. The late background answer
    /// then pages the reader's view (it holds what they asked for), and the
    /// reader's own answer afterwards is matched to the request that timed
    /// out, so nothing is reported for it.
    #[test]
    fn without_ids_answers_are_matched_by_chat() {
        let (mut worker, events, _, _) = receipt_tests::worker();
        prefetching(&mut worker);
        asked(&mut worker, false, true, 1, None);
        worker.pending_older.get_mut(CHAT).unwrap().asked =
            Instant::now().checked_sub(PHONE_PATIENCE * 2).unwrap();
        worker.expire_older_requests();
        drain(&events);
        asked(&mut worker, true, false, 2, None);

        let filed = worker.apply_history(chat_removal_tests::history(CHAT, &[50]), false);
        worker.answer_older(filed, None);
        let events_now = drain(&events);
        assert_eq!(silences(&events_now), [false]);
        assert_eq!(pages(&events_now), 1);

        let filed = worker.apply_history(chat_removal_tests::history(CHAT, &[40]), false);
        worker.answer_older(filed, None);
        assert_eq!(silences(&drain(&events)), [true]);
        assert!(worker.late_older.is_empty());
    }

    /// Once answers are known to carry the request's id, an answer with an
    /// id that is none of ours (a poll's history request) is nobody's page:
    /// the reader's request keeps waiting for its own.
    #[test]
    fn an_answer_to_someone_elses_request_leaves_the_reader_waiting() {
        let (mut worker, events, _, _) = receipt_tests::worker();
        worker.older_ids_match = true;
        asked(&mut worker, true, false, 1, Some("reader"));
        let filed = worker.apply_history(chat_removal_tests::history(CHAT, &[50]), false);
        worker.answer_older(filed, Some("poll"));
        let events = drain(&events);
        assert_eq!(silences(&events), [true]);
        assert_eq!(pages(&events), 0);
        assert!(worker.pending_older.contains_key(CHAT));
    }

    /// A request whose send has not returned has no id yet. Its answer can
    /// still overtake the send result, and is then still its answer.
    #[test]
    fn an_answer_that_overtakes_its_send_result_is_still_matched() {
        let (mut worker, events, _, _) = receipt_tests::worker();
        worker.older_ids_match = true;
        asked(&mut worker, true, false, 1, None);
        let filed = worker.apply_history(chat_removal_tests::history(CHAT, &[50]), false);
        worker.answer_older(filed, Some("reader"));
        assert_eq!(silences(&drain(&events)), [false]);
        assert!(worker.pending_older.is_empty());
    }

    /// The send result names the request by its serial, so it lands on the
    /// request it belongs to, waiting or timed out.
    #[tokio::test]
    async fn the_send_result_records_the_requests_id() {
        let (mut worker, _events, _, _) = receipt_tests::worker();
        asked(&mut worker, false, true, 1, None);
        worker.pending_older.get_mut(CHAT).unwrap().asked =
            Instant::now().checked_sub(PHONE_PATIENCE * 2).unwrap();
        worker.expire_older_requests();
        asked(&mut worker, true, false, 2, None);
        for (serial, id) in [(1, "bg"), (2, "reader")] {
            worker
                .handle_command(Command::OlderSent {
                    chat: CHAT.to_owned(),
                    serial,
                    request: id.to_owned(),
                })
                .await;
        }
        assert_eq!(worker.late_older[0].request.as_deref(), Some("bg"));
        assert_eq!(
            worker.pending_older[CHAT].request.as_deref(),
            Some("reader")
        );
    }

    /// A send failure belongs to the request it came from. One that comes
    /// after a newer request for the chat leaves that request alone.
    #[tokio::test]
    async fn a_late_send_failure_leaves_the_newer_request() {
        let (mut worker, events, _, _) = receipt_tests::worker();
        asked(&mut worker, false, true, 1, None);
        worker.pending_older.get_mut(CHAT).unwrap().asked =
            Instant::now().checked_sub(PHONE_PATIENCE * 2).unwrap();
        worker.expire_older_requests();
        drain(&events);
        asked(&mut worker, true, false, 2, None);
        worker
            .handle_command(Command::OlderFailed {
                chat: CHAT.to_owned(),
                serial: 1,
                error: "fixture failure".into(),
            })
            .await;
        let events = drain(&events);
        assert!(silences(&events).is_empty(), "{events:?}");
        assert_eq!(warnings(&events), 0);
        assert_eq!(worker.pending_older[CHAT].serial, 2);
        assert!(worker.late_older.is_empty());
    }

    /// A background request that fails is its own failure: nothing is shown
    /// for a request nobody made.
    #[tokio::test]
    async fn a_background_failure_stays_quiet() {
        let (mut worker, events, _, _) = receipt_tests::worker();
        prefetching(&mut worker);
        asked(&mut worker, false, true, 1, None);
        worker
            .handle_command(Command::OlderFailed {
                chat: CHAT.to_owned(),
                serial: 1,
                error: "fixture failure".into(),
            })
            .await;
        let events = drain(&events);
        assert_eq!(silences(&events), [true]);
        assert_eq!(warnings(&events), 0);
        assert!(worker.pending_older.is_empty());
    }

    /// Timed-out requests are remembered for their late answers, but only so
    /// many.
    #[test]
    fn late_requests_are_bounded() {
        let (mut worker, _events, _, _) = receipt_tests::worker();
        for serial in 0..(LATE_OLDER_LIMIT as u64 + 5) {
            waiting(&mut worker, false);
            worker.pending_older.get_mut(CHAT).unwrap().serial = serial;
            worker.expire_older_requests();
        }
        assert_eq!(worker.late_older.len(), LATE_OLDER_LIMIT);
        assert_eq!(worker.late_older[0].serial, 5, "the oldest go first");
    }

    fn drain(events: &std::sync::mpsc::Receiver<Event>) -> Vec<Event> {
        std::iter::from_fn(|| events.try_recv().ok()).collect()
    }

    fn warnings(events: &[Event]) -> usize {
        events
            .iter()
            .filter(|event| matches!(event, Event::Error(_)))
            .count()
    }

    fn fetched(events: &[Event]) -> Option<bool> {
        events.iter().find_map(|event| match event {
            Event::OlderFetched { chat, more, .. } if chat == CHAT => Some(*more),
            _ => None,
        })
    }

    /// Opening a short or empty chat asks the phone on its own; a phone with
    /// nothing to add leaves that unanswered, which is no news (#325).
    #[test]
    fn an_unanswered_automatic_request_stays_silent() {
        let (mut worker, events, _, _) = receipt_tests::worker();
        waiting(&mut worker, false);
        worker.expire_older_requests();
        let events = drain(&events);
        assert_eq!(warnings(&events), 0);
        assert_eq!(
            fetched(&events),
            Some(true),
            "the app backs off and may retry"
        );
        assert!(worker.pending_older.is_empty());
    }

    #[test]
    fn a_reader_request_warns_once_until_the_phone_answers() {
        let (mut worker, events, _, _) = receipt_tests::worker();
        waiting(&mut worker, true);
        worker.expire_older_requests();
        assert_eq!(warnings(&drain(&events)), 1);

        waiting(&mut worker, true);
        worker.expire_older_requests();
        assert_eq!(warnings(&drain(&events)), 0, "once per chat");

        // The phone sends this chat's history: a later silence is news again.
        let filed = worker.apply_history(chat_removal_tests::history(CHAT, &[50]), false);
        worker.answer_older(filed, None);
        drain(&events);
        waiting(&mut worker, true);
        worker.expire_older_requests();
        assert_eq!(warnings(&drain(&events)), 1);
    }

    #[test]
    fn scrolling_up_during_an_automatic_request_makes_it_the_readers() {
        let (mut worker, events, _, _) = receipt_tests::worker();
        waiting(&mut worker, false);
        worker.fetch_older(CHAT.to_owned(), true, false);
        assert!(worker.pending_older[CHAT].explicit);
        worker.expire_older_requests();
        assert_eq!(warnings(&drain(&events)), 1);
    }

    #[tokio::test]
    async fn a_failed_automatic_request_stays_silent() {
        let (mut worker, events, _, _) = receipt_tests::worker();
        for (explicit, expected) in [(false, 0), (true, 1)] {
            waiting(&mut worker, explicit);
            worker
                .handle_command(Command::OlderFailed {
                    chat: CHAT.to_owned(),
                    serial: 0,
                    error: "fixture failure".into(),
                })
                .await;
            let events = drain(&events);
            assert_eq!(warnings(&events), expected, "explicit: {explicit}");
            assert_eq!(fetched(&events), Some(true));
        }
    }

    /// A chat the phone said it holds nothing older for is never asked again,
    /// even in a later session.
    #[test]
    fn a_chat_at_its_start_is_not_asked_again() {
        let (mut worker, events, _, _) = receipt_tests::worker();
        let mut history = chat_removal_tests::history(CHAT, &[100]);
        history.chats[0].more_on_phone = Some(false);
        worker.apply_history(history, true);
        assert!(worker.archive.history_start(CHAT).unwrap());
        drain(&events);

        worker.fetch_older(CHAT.to_owned(), true, false);
        assert_eq!(
            fetched(&drain(&events)),
            Some(false),
            "the app stops asking"
        );
        assert!(worker.pending_older.is_empty());
    }

    #[test]
    fn history_fidelity_reload_returns_overlapping_rows_before_its_anchor() {
        let (mut worker, events, _, _) = receipt_tests::worker();
        worker.apply_history(chat_removal_tests::history(CHAT, &[50, 100]), false);
        worker.archive.set_history_start(CHAT).unwrap();
        drain(&events);
        // The reader chose m100, not the archive's oldest message m50.
        waiting(&mut worker, true);
        let filed = worker.apply_history(chat_removal_tests::history(CHAT, &[50, 75]), false);
        worker.answer_older(filed, None);
        let messages = drain(&events)
            .into_iter()
            .find_map(|event| match event {
                Event::Messages {
                    messages,
                    older: true,
                    ..
                } => Some(messages),
                _ => None,
            })
            .unwrap();
        assert_eq!(
            messages.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            ["m50", "m75"]
        );
        assert!(worker.pending_older.is_empty());

        // A known start prevents fetching older history, but not repairing it.
        worker.reload_history(CHAT.into(), "m100".into());
        assert_eq!(
            fetched(&drain(&events)),
            Some(true),
            "offline, rather than refused at the chat start"
        );
    }

    #[test]
    fn more_on_the_phone_keeps_the_chat_open_to_asking() {
        let (mut worker, _events, _, _) = receipt_tests::worker();
        let mut history = chat_removal_tests::history(CHAT, &[100]);
        history.chats[0].more_on_phone = Some(true);
        worker.apply_history(history, true);
        worker.apply_history(chat_removal_tests::history(CHAT, &[90]), true);
        assert!(!worker.archive.history_start(CHAT).unwrap());
    }
}
