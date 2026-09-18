//! Tokio worker for WhatsApp, the archive, attachments, and profile pictures.
//!
//! Messages are archived before reaching the UI. Privacy ids (`@lid`) are
//! canonicalized to phone-number ids as soon as their mapping is known.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::mpsc;
use whatsapp_rust::download::MediaType;
use whatsapp_rust::features::message_edit::{
    SecretEncKind, decrypt_secret_encrypted_with_fallback, extract_secret_encrypted,
};
use whatsapp_rust::media::{
    AudioOptions, DocumentOptions, ImageOptions, VideoOptions, audio_message, document_message,
    image_message, video_message,
};
use whatsapp_rust::pair_code::PairCodeOptions;
use whatsapp_rust::prelude::{
    Bot, BotHandle, Client, Jid, MessageBuilderExt, MessageExt, MessageField, SendOptions,
    SqliteStore, wa,
};
use whatsapp_rust::send::RevokeType;
use whatsapp_rust::types::events as wa_events;
use whatsapp_rust::types::message::{MessageInfo, MessageSource};
use whatsapp_rust::types::presence::{ChatPresence, ReceiptType};
use whatsapp_rust::upload::UploadOptions;
use whatsapp_rust::wacore::download::Downloadable;
use whatsapp_rust::wacore::history_sync::{HistorySyncStream, MAX_DECOMPRESSED};
use whatsapp_rust::wacore::store::DevicePropsOverride;
use whatsapp_rust::wacore_binary::jid::JidExt;
use whatsapp_rust::waproto::buffa::Message as _;
use whatsapp_rust::{MediaRetryResult, MediaReuploadRequest};

mod poll_history;
mod polls;

use super::{Command, Event, LinkStatus, Waker, read_sync::ReadSync};
use crate::app::PAGE;
use crate::archive::Archive;
use crate::model::{
    Chat, ChatId, ChatKind, Contact, Content, Delivery, Gif, GifError, LinkPreview, Media,
    MentionRef, Message, Quoted, Reaction,
};
use crate::paths::AppDirs;

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
/// Sticker download batch size for the picker.
const STICKER_FETCH_LIMIT: usize = 40;

fn account_allows_receipts(
    settings: &whatsapp_rust::wacore::iq::privacy::PrivacySettingsResponse,
) -> bool {
    use whatsapp_rust::wacore::iq::privacy::{PrivacyCategory, PrivacyValue};
    matches!(
        settings.get_value(&PrivacyCategory::ReadReceipts),
        Some(PrivacyValue::All)
    )
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

/// App version in WhatsApp device-property format.
fn app_version() -> wa::device_props::AppVersion {
    let mut parts = env!("CARGO_PKG_VERSION")
        .split('.')
        .map(|part| part.parse::<u32>().ok());
    wa::device_props::AppVersion {
        primary: parts.next().flatten(),
        secondary: parts.next().flatten(),
        tertiary: parts.next().flatten(),
        ..Default::default()
    }
}

/// Stable sticker hash across messages and the phone's recent list.
fn sticker_hash(sha256: Option<&[u8]>, enc_sha256: Option<&[u8]>) -> Option<String> {
    let bytes = sha256
        .filter(|bytes| !bytes.is_empty())
        .or(enc_sha256.filter(|bytes| !bytes.is_empty()))?;
    Some(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

pub async fn run(
    dirs: AppDirs,
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
                        Some(Command::Shutdown) | None => return,
                        _ => {}
                    }
                }
            }
        }
    };
    let (wa_sender, wa_events) = mpsc::unbounded_channel();
    let mut worker = Worker {
        dirs,
        events,
        commands,
        waker,
        archive,
        client: None,
        handle: None,
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
        group_info_queue: std::collections::VecDeque::new(),
        group_info_tries: HashMap::new(),
        group_info_retry: Vec::new(),
        presence_subscribed: HashSet::new(),
        pending_older: HashMap::new(),
        older_warned: HashSet::new(),
        pending_avatars: HashMap::new(),
        sticker_fetches: HashSet::new(),
        sticker_downloads: HashSet::new(),
        read_sync: ReadSync::default(),
        poll_decrypting: 0,
        poll_history: Default::default(),
        poll_sending: HashSet::new(),
    };
    worker.load_state();
    worker.backfill();
    worker.relocate_media();
    worker.start_bot().await;
    let mut wa_events = wa_events;
    let mut tick = tokio::time::interval(Duration::from_secs(5));
    loop {
        let deadline = worker.sync_deadline;
        tokio::select! {
            command = inbox.recv() => {
                match command {
                    Some(Command::Shutdown) | None => break,
                    Some(command) => worker.handle_command(command).await,
                }
            }
            Some(event) = wa_events.recv() => worker.handle_wa_event(event).await,
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
                worker.expire_older_requests();
                worker.retry_avatars();
                worker.pump_group_info();
                worker.pump_read_sync();
                worker.pump_poll_votes();
                worker.pump_poll_history();
            }
        }
    }
    worker.stop_bot().await;
}

struct Worker {
    read_sync: ReadSync,
    poll_decrypting: usize,
    poll_history: poll_history::Requests,
    poll_sending: HashSet<(ChatId, String)>,
    dirs: AppDirs,
    events: std::sync::mpsc::Sender<Event>,
    commands: mpsc::UnboundedSender<Command>,
    waker: Waker,
    archive: Archive,
    client: Option<Arc<Client>>,
    handle: Option<BotHandle>,
    wa_sender: mpsc::UnboundedSender<Arc<wa_events::Event>>,
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
    group_info_tries: HashMap<String, u32>,
    /// Next retry time for failed group metadata requests.
    group_info_retry: Vec<(Instant, String)>,
    presence_subscribed: HashSet<String>,
    /// Pending phone-history request time and boundary by chat.
    pending_older: HashMap<ChatId, (Instant, super::PageKey)>,
    /// Chats already notified about a phone-history timeout.
    older_warned: HashSet<ChatId>,
    /// Deferred profile-picture requests and retry counts.
    pending_avatars: HashMap<(String, bool), u32>,
    /// Active recent-sticker downloads by hash.
    sticker_fetches: HashSet<String>,
    /// Active chat-sticker downloads by chat and message id.
    sticker_downloads: HashSet<(ChatId, String)>,
}

/// Decoded history chunk waiting to be canonicalized and archived.
struct ParsedHistory {
    chats: Vec<ParsedChat>,
    push_names: Vec<(String, String)>,
    lids: Vec<(String, String)>,
    /// Recent phone stickers included with history sync.
    stickers: Vec<wa::StickerMetadata>,
}

struct ParsedChat {
    id: String,
    name: Option<String>,
    unread: Option<u32>,
    archived: bool,
    pinned_at: Option<i64>,
    /// Outer None means the history chunk omitted mute metadata.
    muted_until: Option<Option<i64>>,
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
}

struct HistoryPollUpdate {
    id: String,
    sender: Option<String>,
    from_me: bool,
    timestamp: i64,
    update: wa::message::PollUpdateMessage,
}

/// Standalone reaction from history, applied after the parent row is stored.
struct HistoryReaction {
    target: String,
    sender: Option<String>,
    from_me: bool,
    body: HistoryReactionBody,
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

    fn emit(&self, event: Event) {
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
        let chat = chat.to_owned();
        tokio::spawn(async move {
            if let Err(error) = call(client, jid).await {
                log::warn!("the phone was not told about {chat}: {error}");
            }
        });
    }

    fn emit_chats(&self) {
        match self.archive.chats() {
            Ok(mut chats) => {
                // Early preference sync can create an empty privacy-id row.
                // Once mapped, its preferences live on the canonical chat.
                chats.retain(|chat| chat.last.is_some() || self.canonical_str(&chat.id) == chat.id);
                for chat in &mut chats {
                    self.polish_chat(chat);
                }
                self.emit(Event::Chats(chats));
            }
            Err(error) => log::warn!("could not list chats: {error}"),
        }
    }

    fn emit_chat(&self, id: &str) {
        if let Ok(Some(mut chat)) = self.archive.chat(id) {
            self.polish_chat(&mut chat);
            self.emit(Event::ChatUpdated(Box::new(chat)));
        }
    }

    /// Resolves phone numbers in chat-row previews.
    fn polish_chat(&self, chat: &mut Chat) {
        if let Some(last) = chat.last.as_mut() {
            last.summary = self.pn_tokens(&last.summary);
        }
    }

    fn emit_message(&self, chat: &str, id: &str) {
        if let Ok(Some(mut message)) = self.archive.message(chat, id) {
            self.polish(&mut message);
            self.emit(Event::MessageUpdated(Box::new(message)));
        }
    }

    fn set_status(&mut self, status: LinkStatus) {
        if self.status != status {
            log::info!("link: {}", status.log_label());
            self.status = status.clone();
            self.emit(Event::Link(status));
        }
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

    fn is_me(&self, id: &str) -> bool {
        self.me_pn.as_deref() == Some(id) || self.me_lid.as_deref() == Some(id)
    }

    fn load_state(&mut self) {
        self.me_pn = self.archive.meta("me_pn").ok().flatten();
        self.me_lid = self.archive.meta("me_lid").ok().flatten();
        self.me_name = self.archive.meta("me_name").ok().flatten();
        self.me_about = self.archive.meta("me_about").ok().flatten();
        if let Ok(lids) = self.archive.lids() {
            self.lid_to_pn = lids.into_iter().collect();
        }
        if let Ok(contacts) = self.archive.contacts() {
            self.contacts = contacts
                .into_iter()
                .map(|contact| (contact.id.clone(), contact))
                .collect();
        }
        if let Some(id) = self.me_pn.clone().or_else(|| self.me_lid.clone()) {
            self.emit(Event::Me {
                id,
                name: self.me_name.clone(),
                about: self.me_about.clone(),
            });
        }
        self.emit(Event::Contacts(self.contacts.values().cloned().collect()));
        self.emit_chats();
    }

    /// Re-derives archived rows from raw protobufs after parser changes. Also
    /// repairs moved attachment paths or clears missing files for redownload.
    fn relocate_media(&mut self) {
        let dir = self.dirs.media_dir();
        let rows = match self.archive.media_paths() {
            Ok(rows) => rows,
            Err(error) => {
                log::warn!("could not list attachments: {error}");
                return;
            }
        };
        let (mut moved, mut forgotten) = (0, 0);
        for (chat, id, path) in rows {
            if path.exists() {
                continue;
            }
            let candidate = path.file_name().map(|name| dir.join(name));
            match candidate.filter(|candidate| candidate.exists()) {
                Some(candidate) => {
                    if self.archive.set_media_path(&chat, &id, &candidate).is_ok() {
                        moved += 1;
                    }
                }
                None => {
                    if self.archive.clear_media_path(&chat, &id).is_ok() {
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
        const VERSION: &str = "2";
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
            let Some(mut content) = classify(base) else {
                continue;
            };
            let Ok(Some(existing)) = self.archive.message(&chat, &id) else {
                continue;
            };
            if matches!(existing.content, Content::Revoked) {
                continue;
            }
            if let (Some(new), Some(old)) = (content.media_mut(), existing.content.media()) {
                new.path = old.path.clone();
            }
            let mentions = self.mentions_of(&mentioned_of(base));
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

    async fn start_bot(&mut self) {
        let path = self.dirs.session_db();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let store = match SqliteStore::new(&path.to_string_lossy()).await {
            Ok(store) => store,
            Err(error) => {
                self.set_status(LinkStatus::Failed(format!(
                    "Could not open the device store: {error}"
                )));
                return;
            }
        };
        let sender = self.wa_sender.clone();
        let bot = Bot::builder()
            .with_backend(store)
            // WhatsApp reads the linked-device name, version, and icon at pairing.
            .with_device_props(
                DevicePropsOverride::new()
                    .with_os("ZapFast")
                    .with_version(app_version())
                    .with_platform_type(wa::device_props::PlatformType::DESKTOP),
            )
            .on_event(move |event, _client| {
                let sender = sender.clone();
                async move {
                    let _ = sender.send(event);
                }
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

    fn refresh_legacy_preferences(&self) {
        let Some(client) = self.client.clone() else {
            return;
        };
        match self.archive.take_preferences_refresh() {
            Ok(true) => {
                tokio::spawn(async move {
                    use whatsapp_rust::{WAPatchName, sync_task::MajorSyncTask};
                    // The protocol library owns collection locking, full
                    // snapshots, and bounded retries across reconnects. Do
                    // not reset its store or add an application retry loop.
                    for name in [WAPatchName::RegularLow, WAPatchName::RegularHigh] {
                        client
                            .process_sync_task(MajorSyncTask::AppStateSync {
                                name,
                                full_sync: true,
                            })
                            .await;
                    }
                });
            }
            Ok(false) => {}
            Err(error) => log::warn!("could not schedule chat preference recovery: {error}"),
        }
    }

    async fn stop_bot(&mut self) {
        self.client = None;
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
        self.lid_to_pn.insert(lid.to_owned(), pn.to_owned());
        match self.archive.put_lid(lid, pn) {
            Ok(true) => self.emit_chats(),
            Ok(false) => {}
            Err(error) => log::warn!("could not remember an id mapping: {error}"),
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

    fn jid_of(id: &str) -> Option<Jid> {
        id.parse().ok()
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

    /// Replaces a fallback chat name when a better one is known.
    fn refresh_chat_name(&mut self, id: &str) {
        let Ok(Some(chat)) = self.archive.chat(id) else {
            return;
        };
        if chat.kind == ChatKind::Group {
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
                if chat.kind != ChatKind::Group {
                    let name = self.chat_name(id, push_name);
                    if name != chat.name {
                        let _ = self.archive.rename_chat(id, &name);
                    }
                }
            }
            Ok(None) => {
                let name = self.chat_name(id, push_name);
                if let Err(error) = self.archive.ensure_chat(id, &name) {
                    log::warn!("could not create chat {id}: {error}");
                }
            }
            Err(error) => log::warn!("could not read chat {id}: {error}"),
        }
        if ChatKind::from_id(id) == ChatKind::Group {
            self.request_group_info(id, false);
        }
    }

    /// Queues a group metadata request, at the front when `force` is true.
    fn request_group_info(&mut self, id: &str, force: bool) {
        if force {
            self.group_info_requested.remove(id);
        } else {
            let known = self.archive.chat(id).ok().flatten().is_some_and(|chat| {
                chat.name != fallback_name(id) && !chat.participants.is_empty()
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
            self.query_group_info(&id);
        }
    }

    /// Schedules metadata retry with backoff, or stops on permanent failure.
    fn handle_failed_group(&mut self, chat: String, permanent: bool) {
        self.group_info_requested.remove(&chat);
        if permanent {
            self.group_info_tries.remove(&chat);
        } else {
            let tries = self.group_info_tries.entry(chat.clone()).or_insert(0);
            *tries += 1;
            if *tries <= 7 {
                self.group_info_retry
                    .push((Instant::now() + Self::group_retry_delay(*tries), chat));
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
        let me: Vec<String> = [self.me_pn.clone(), self.me_lid.clone()]
            .into_iter()
            .flatten()
            .collect();
        let lids = self.lid_to_pn.clone();
        tokio::spawn(async move {
            match client.groups().get_metadata(&jid).await {
                Ok(metadata) => {
                    let canonical = |jid: &Jid| -> String {
                        if jid.is_lid()
                            && let Some(pn) = lids.get(jid.user_base())
                        {
                            return format!("{pn}@s.whatsapp.net");
                        }
                        jid.to_non_ad_string()
                    };
                    let mut participants = Vec::new();
                    let mut admin = false;
                    for participant in &metadata.participants {
                        let id = participant
                            .phone_number
                            .as_ref()
                            .map(canonical)
                            .unwrap_or_else(|| canonical(&participant.jid));
                        let mine = me.contains(&id)
                            || participant
                                .lid
                                .as_ref()
                                .is_some_and(|lid| me.contains(&lid.to_non_ad_string()))
                            || me.contains(&participant.jid.to_non_ad_string());
                        if mine && participant.is_admin() {
                            admin = true;
                        }
                        participants.push(id);
                    }
                    let _ = commands.send(Command::GroupInfo {
                        chat,
                        name: (!metadata.subject.is_empty()).then(|| metadata.subject.clone()),
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
                    });
                }
                Err(error) => {
                    let text = error.to_string();
                    // Missing, forbidden, and unauthorized groups do not retry.
                    let permanent = ["item-not-found", "forbidden", "not-authorized"]
                        .iter()
                        .any(|word| text.contains(word));
                    log::warn!("no metadata for {chat}: {text}");
                    let _ = commands.send(Command::GroupInfoFailed { chat, permanent });
                }
            }
        });
    }

    // --- WhatsApp events -------------------------------------------------

    async fn handle_wa_event(&mut self, event: Arc<wa_events::Event>) {
        use wa_events::Event as E;
        match &*event {
            E::PairingQrCode(qr) => {
                self.qr = Some(qr.code.clone());
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
                self.refresh_legacy_preferences();
                self.retry_avatars();
                self.pump_read_sync();
                self.poll_history.reconnect(Instant::now());
                let _ = self.archive.retry_poll_votes();
                self.pump_poll_votes();
                if let Some(client) = self.client.clone() {
                    let me = self.me_pn.clone().and_then(|pn| Self::jid_of(&pn));
                    let commands = self.commands.clone();
                    tokio::spawn(async move {
                        if let Err(error) = client.presence().set_available().await {
                            log::debug!("presence not announced: {error}");
                        }
                        // whatsapp-rust also enforces the account privacy setting.
                        match client.fetch_privacy_settings().await {
                            Ok(settings) => {
                                let disabled = !account_allows_receipts(&settings);
                                let _ = commands.send(Command::ReceiptsPrivacy { disabled });
                            }
                            Err(error) => log::debug!("privacy settings not fetched: {error}"),
                        }
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
                self.ingest_undecryptable(&undecryptable.info);
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
                    composing: matches!(presence.state, ChatPresence::Composing),
                });
            }
            E::Presence(presence) => {
                self.emit(Event::Presence {
                    id: self.canonical(&presence.from),
                    online: !presence.unavailable,
                    last_seen: presence.last_seen.map(|when| when.timestamp()),
                });
            }
            E::ContactUpdate(update) => self.on_contact_update(update),
            E::GroupUpdate(update) => {
                let chat = self.canonical(&update.group_jid);
                if let whatsapp_rust::wacore::stanza::groups::GroupNotificationAction::Ephemeral {
                    expiration,
                    ..
                } = &update.action
                {
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
                self.request_group_info(&chat, true);
            }
            E::ArchiveUpdate(update) => {
                let chat = self.canonical(&update.jid);
                let _ = self
                    .archive
                    .set_archived(&chat, update.action.archived.unwrap_or(false));
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
                } else {
                    let _ = self.archive.finish_read_sync(&chat, i64::MAX);
                    let unread = self
                        .archive
                        .chat(&chat)
                        .ok()
                        .flatten()
                        .map_or(1, |row| row.unread.max(1));
                    let _ = self.archive.set_unread(&chat, unread);
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
                let _ = std::fs::remove_file(self.avatar_file(&id, false));
                let _ = std::fs::remove_file(self.avatar_file(&id, true));
                if update.removed {
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
            E::SelfPushNameUpdated(update) => {
                self.me_name = Some(update.new_name.clone());
                let _ = self.archive.set_meta("me_name", &update.new_name);
                self.emit(Event::Me {
                    id: self.me(),
                    name: self.me_name.clone(),
                    about: self.me_about.clone(),
                });
            }
            E::OfflineSyncCompleted(_) => self.emit_chats(),
            _ => {}
        }
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
        self.emit(Event::Me {
            id: self.me(),
            name: self.me_name.clone(),
            about: self.me_about.clone(),
        });
    }

    async fn on_logged_out(&mut self) {
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
        self.poll_sending.clear();
        self.poll_history = Default::default();
        self.pending_older.clear();
        self.pending_avatars.clear();
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
        self.clean_media_cache();
        self.emit(Event::Chats(Vec::new()));
        self.set_status(LinkStatus::LoggedOut);
        // Recreate the store so the next connection starts linking.
        self.start_bot().await;
    }

    /// Cleans the media cache directory on logout without removing any configured custom media subtree.
    fn clean_media_cache(&self) {
        let media_cache = self.dirs.media_cache_dir();
        if let Some(custom) = &self.dirs.custom_media
            && AppDirs::is_subpath(custom, &media_cache)
        {
            if let Ok(entries) = std::fs::read_dir(&media_cache) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if AppDirs::is_subpath(custom, &path) {
                        continue;
                    }
                    if path.is_dir() {
                        let _ = std::fs::remove_dir_all(&path);
                    } else {
                        let _ = std::fs::remove_file(&path);
                    }
                }
            }
            return;
        }
        let _ = std::fs::remove_dir_all(media_cache);
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
        let contact = self.contacts.entry(id.clone()).or_insert_with(|| Contact {
            id: id.clone(),
            full_name: None,
            push_name: None,
        });
        if contact.full_name == name {
            return;
        }
        contact.full_name = name;
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
                // The receipt time is when the phone read, not the position
                // it read through. A delayed receipt must leave newer messages.
                for id in &receipt.message_ids {
                    if self
                        .archive
                        .message(&chat, id)
                        .ok()
                        .flatten()
                        .is_some_and(|message| !message.from_me)
                    {
                        let _ = self.archive.mark_read_to(&chat, id);
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
                if !self
                    .archive
                    .message(&chat, id)
                    .ok()
                    .flatten()
                    .is_some_and(|row| row.from_me)
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
            }
            self.emit_chat(&chat);
            return;
        }
        let mut newest = 0;
        let mut changed = 0;
        for id in &receipt.message_ids {
            match self.archive.set_status(&chat, id, status, at) {
                Ok(true) => {
                    changed += 1;
                    self.emit_message(&chat, id);
                }
                Ok(false) => {}
                Err(error) => log::warn!("could not file a receipt for {id}: {error}"),
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

    /// Returns raw mention tokens and canonical ids.
    fn mentions_of(&self, raw: &[String]) -> Vec<MentionRef> {
        raw.iter()
            .filter_map(|jid| {
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
        if let Some(expiration) = info.ephemeral_expiration
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
                    if let Ok(true) =
                        self.archive
                            .set_content(&chat, &target, &Content::Revoked, false)
                    {
                        self.emit_message(&chat, &target);
                        self.emit_chat(&chat);
                    }
                }
                Some(Type::MESSAGE_EDIT) => {
                    if let Some(edited) = protocol.edited_message.as_option()
                        && let Some(mut content) = classify(edited.get_base_message())
                    {
                        // Preserve downloaded media when updating a caption.
                        if let Ok(Some(existing)) = self.archive.message(&chat, &target)
                            && let (Some(new), Some(old)) =
                                (content.media_mut(), existing.content.media())
                        {
                            new.path = old.path.clone();
                        }
                        if let Ok(true) = self.archive.set_content(&chat, &target, &content, true) {
                            self.emit_message(&chat, &target);
                            self.emit_chat(&chat);
                        }
                    }
                }
                _ => {}
            }
            return;
        }
        if let Some(reaction) = base.reaction_message.as_option() {
            self.store_plain_reaction(&chat, &sender, from_me, reaction);
            return;
        }
        if base.enc_reaction_message.is_set() {
            self.store_enc_reaction(&chat, &sender, from_me, base);
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
        let Some(content) = classify(base) else {
            return;
        };
        let quoted = self.quoted_of(base);
        let mentions = self.mentions_of(&mentioned_of(base));
        let row = Message {
            id: info.id.clone(),
            chat: chat.clone(),
            sender,
            sender_name: if from_me { None } else { push_name.clone() },
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
            edited: false,
            mentions,
            forwarded: forwarded_of(base),
            thumbnail: thumbnail_of(base),
        };
        let is_poll = matches!(row.content, Content::Poll { .. });
        self.remember_poll(&row, message, &info.source.sender.to_non_ad_string(), None);
        self.store_message(row, Some(message.encode_to_vec()), push_name.as_deref());
        if is_poll {
            self.pump_poll_votes();
        }
    }

    fn store_plain_reaction(
        &mut self,
        chat: &str,
        sender: &str,
        from_me: bool,
        reaction: &wa::message::ReactionMessage,
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
        self.store_reaction(chat, &target, sender, from_me, &emoji);
    }

    fn store_enc_reaction(
        &mut self,
        chat: &str,
        sender: &str,
        from_me: bool,
        message: &wa::Message,
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
        let Some(emoji) =
            self.decrypt_enc_reaction(chat, target, sender, env.enc_payload, env.enc_iv, None)
        else {
            return;
        };
        self.store_reaction(chat, target, sender, from_me, &emoji);
    }

    fn store_reaction(
        &mut self,
        chat: &str,
        target: &str,
        sender: &str,
        from_me: bool,
        emoji: &str,
    ) {
        if let Ok(Some(updated)) = self
            .archive
            .set_reaction(chat, target, sender, from_me, emoji)
        {
            self.emit(Event::MessageUpdated(Box::new(updated)));
        }
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
        let emoji = match reaction.body {
            HistoryReactionBody::Plain(emoji) => emoji,
            HistoryReactionBody::Encrypted { payload, iv } => {
                let Some(emoji) = self.decrypt_enc_reaction(
                    chat,
                    &reaction.target,
                    &sender,
                    &payload,
                    &iv,
                    Some(secrets),
                ) else {
                    return;
                };
                emoji
            }
        };
        self.store_reaction(chat, &reaction.target, &sender, reaction.from_me, &emoji);
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

    fn ingest_undecryptable(&mut self, info: &MessageInfo) {
        self.learn_source(&info.source);
        if info.source.chat.is_status_broadcast() || info.source.is_from_me {
            return;
        }
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
        let row = Message {
            id: info.id.clone(),
            chat,
            sender: self.canonical(&info.source.sender),
            sender_name: push_name.clone(),
            from_me: false,
            timestamp: info.timestamp.timestamp(),
            content: Content::Unsupported {
                what: "Waiting for this message. Open WhatsApp on your phone".to_owned(),
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
        self.store_message(row, None, push_name.as_deref());
    }

    /// Archives a message and emits chat and row updates.
    fn store_message(&mut self, message: Message, raw: Option<Vec<u8>>, push_name: Option<&str>) {
        let chat = message.chat.clone();
        self.ensure_chat(&chat, if message.from_me { None } else { push_name });
        if let Some(push_name) = push_name
            && !message.from_me
        {
            let sender = message.sender.clone();
            self.remember_push_name(&sender, push_name);
        }
        let is_new = self
            .archive
            .message(&chat, &message.id)
            .ok()
            .flatten()
            .is_none();
        if let Err(error) = self.archive.insert_message(&message, raw.as_deref()) {
            log::warn!("could not store a message: {error}");
            return;
        }
        let unread = is_new
            && !message.from_me
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
        self.emit_chat(&chat);
        if let Some(message) = incoming {
            self.emit(Event::Incoming {
                chat,
                message: Box::new(message),
            });
        }
    }

    fn quoted_of(&self, base: &wa::Message) -> Option<Quoted> {
        let context = context_of(base)?;
        let id = context.stanza_id.clone().filter(|id| !id.is_empty())?;
        let sender = context
            .participant
            .as_deref()
            .map(|participant| self.canonical_str(participant))
            .unwrap_or_default();
        let (summary, listed) = context
            .quoted_message
            .as_option()
            .map(|quoted| {
                let base = quoted.get_base_message();
                (
                    classify(base)
                        .map(|content| content.summary())
                        .unwrap_or_default(),
                    self.mentions_of(&mentioned_of(base)),
                )
            })
            .unwrap_or_default();
        // Recover quote mentions from `@user` tokens when metadata is missing.
        let summary = self.pn_tokens(&summary);
        let mentions = if listed.is_empty() {
            self.mention_tokens(&summary)
        } else {
            listed
        };
        Some(Quoted {
            sender_name: self.name_for(&sender),
            id,
            sender,
            summary,
            mentions,
        })
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
                }
                let filed = self.apply_history(parsed, !on_demand);
                if on_demand {
                    self.answer_older(filed);
                }
            }
            Ok(Err(error)) => {
                log::warn!("a history chunk could not be read: {error}");
                self.emit(Event::Error(format!(
                    "Could not read part of the chat history: {error}"
                )));
            }
            Err(error) => log::warn!("history parsing panicked: {error}"),
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
            if let Err(error) = self.archive.upsert_phone_sticker(
                &hash,
                &sticker.encode_to_vec(),
                seconds(sticker.last_sticker_sent_ts.unwrap_or(0)),
                sticker.weight.unwrap_or(0.0),
            ) {
                log::warn!("could not store sticker {hash}: {error}");
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
        for chat in parsed.chats {
            let id = self.canonical_str(&chat.id);
            if id.ends_with("@broadcast") {
                continue;
            }
            let existing = self.archive.chat(&id).ok().flatten();
            if metadata || existing.is_none() {
                let name = match chat.name.filter(|name| !name.is_empty()) {
                    Some(name) if ChatKind::from_id(&id) == ChatKind::Group => name,
                    Some(name) => {
                        // Prefer the phone's address-book name for direct chats.
                        let contact = self.contacts.entry(id.clone()).or_insert_with(|| Contact {
                            id: id.clone(),
                            full_name: None,
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
                    None => self.chat_name(&id, None),
                };
                let mut row = Chat::new(id.clone(), name);
                row.last_activity = chat.last_activity;
                row.unread = existing.as_ref().map_or(0, |existing| existing.unread);
                row.archived = chat.archived;
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
                if let Err(error) = self.archive.upsert_chat(&row) {
                    log::warn!("could not store chat {id}: {error}");
                    continue;
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
            let mut secrets = HashMap::new();
            for message in chat.messages {
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
                        sender_name: self.name_for(&sender),
                        sender,
                        ..quoted
                    }
                });
                let mentions = self.mentions_of(&message.mentions);
                let row = Message {
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
                    delivered_at: None,
                    read_at: None,
                    quoted,
                    reactions,
                    edited: false,
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
                if let Err(error) = self.archive.insert_message(&row, Some(&raw)) {
                    log::warn!("could not store a history message: {error}");
                }
                if matches!(row.content, Content::Poll { .. }) {
                    if poll_history_received {
                        let _ = self.archive.mark_poll_history(&id, &row.id);
                        self.poll_history.finish(&id, &row.id);
                    }
                    self.emit_message(&id, &row.id);
                }
            }
            for reaction in chat.reactions {
                self.apply_history_reaction(&id, reaction, &secrets);
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
                let _ = self
                    .archive
                    .set_content(&id, &revoked, &Content::Revoked, false);
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
            filed.push((id, count, chat.more_on_phone));
        }
        self.pump_poll_votes();
        self.pump_poll_history();
        for (id, _, _) in &filed {
            self.emit_chat(id);
        }
        filed
    }

    /// Completes pending requests covered by an on-demand history chunk.
    fn answer_older(&mut self, filed: Vec<(ChatId, usize, Option<bool>)>) {
        for (chat, count, more_on_phone) in filed {
            let more = count > 0 && more_on_phone != Some(false);
            let Some((_, (before_time, before_id))) = self.pending_older.remove(&chat) else {
                // Late responses are already archived; tell the app to page again.
                self.emit(Event::OlderFetched { chat, more });
                continue;
            };
            match self
                .archive
                .messages(&chat, Some((before_time, &before_id)), 500)
            {
                Ok(mut messages) => {
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
            self.emit(Event::OlderFetched { chat, more });
        }
    }

    /// Times out unanswered phone-history requests.
    fn expire_older_requests(&mut self) {
        let expired: Vec<ChatId> = self
            .pending_older
            .iter()
            .filter(|(_, (asked, _))| asked.elapsed() > PHONE_PATIENCE)
            .map(|(chat, _)| chat.clone())
            .collect();
        for chat in expired {
            self.pending_older.remove(&chat);
            self.emit(Event::OlderFetched {
                chat: chat.clone(),
                more: true,
            });
            // Report the timeout once per chat; later retries back off silently.
            if self.older_warned.insert(chat) {
                self.emit(Event::Error(
                    "Your phone did not send older messages. Check that it is online".to_owned(),
                ));
            }
        }
    }

    fn fetch_older(&mut self, chat: ChatId) {
        if self.pending_older.contains_key(&chat) {
            return;
        }
        let (Some(client), Some(jid)) = (self.client.clone(), Self::jid_of(&chat)) else {
            // Offline requests retry after reconnection; the banner shows state.
            self.emit(Event::OlderFetched { chat, more: true });
            return;
        };
        // Chats without messages request history from the current time.
        let (id, from_me, timestamp) = match self.archive.oldest(&chat) {
            Ok(Some(oldest)) => (oldest.id, oldest.from_me, oldest.timestamp),
            _ => (String::new(), false, crate::util::now()),
        };
        self.pending_older
            .insert(chat.clone(), (Instant::now(), (timestamp, id.clone())));
        let commands = self.commands.clone();
        tokio::spawn(async move {
            if let Err(error) = client
                // Despite its `Ms` name, the protocol field takes Unix seconds.
                // https://github.com/tulir/whatsmeow/commit/54650307d891f89ab346a57953d316106caee371
                .fetch_message_history(&jid, &id, from_me, timestamp, PHONE_BATCH)
                .await
            {
                log::warn!("older messages not requested: {error}");
                let _ = commands.send(Command::OlderFailed {
                    chat: chat.clone(),
                    error: format!("Could not request older messages from your phone: {error}"),
                });
            }
        });
    }

    // --- commands --------------------------------------------------------

    async fn handle_command(&mut self, command: Command) {
        match command {
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
            Command::Forward {
                from_chat,
                message,
                to_chat,
            } => self.forward_message(from_chat, message, to_chat),
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
            Command::LoadChat { chat, before } => self.load_chat(chat, before),
            Command::FetchOlder(chat) => self.fetch_older(chat),
            Command::LoadUntil { chat, id, before } => self.load_until(chat, id, before),
            Command::SearchMessages { query } => self.search_messages(query),
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
            Command::Download { chat, message } => self.download(chat, message),
            Command::FetchAvatar { id, full } => self.fetch_avatar(id, full),
            Command::EditText {
                chat,
                id,
                text,
                mentions,
            } => self.edit_text(chat, id, text, mentions),
            Command::Revoke { chat, id } => self.revoke(chat, id),
            Command::DeleteLocal { chat, id } => {
                if let Ok(true) = self.archive.delete_message(&chat, &id) {
                    self.emit(Event::MessageDeleted {
                        chat: chat.clone(),
                        id,
                    });
                    self.emit_chat(&chat);
                }
            }
            Command::PickFiles(chat) => {
                let commands = self.commands.clone();
                tokio::task::spawn_blocking(move || {
                    let paths = rfd::FileDialog::new()
                        .set_title("Send to WhatsApp")
                        .pick_files()
                        .unwrap_or_default();
                    let _ = commands.send(Command::Picked { chat, paths });
                });
            }
            Command::Picked { chat, paths } => self.emit(Event::Picked { chat, paths }),
            Command::SendFiles {
                chat,
                paths,
                caption,
                mentions,
            } => {
                self.send_files(chat, paths, caption, mentions);
            }
            Command::SendImage {
                chat,
                width,
                height,
                rgba,
                caption,
                mentions,
            } => self.send_pasted_image(chat, width, height, rgba, caption, mentions),
            Command::Outbound { chat, row, raw } => self.outbound(chat, *row, raw),
            Command::SendSticker { chat, path } => self.send_sticker(chat, path),
            Command::SaveSticker { path } => match self.save_sticker(&path) {
                Ok(()) => self.emit_stickers(),
                Err(error) => self.emit(Event::Error(format!("Could not save sticker: {error}"))),
            },
            Command::ForgetSticker { path } => {
                // Restrict deletion to files in the saved-sticker directory.
                if path.starts_with(self.dirs.saved_sticker_dir())
                    && std::fs::remove_file(&path).is_ok()
                {
                    self.emit_stickers();
                }
            }
            Command::ImportStickerUrl { url } => {
                let commands = self.commands.clone();
                let packs = self.packs_dir();
                tokio::task::spawn_blocking(move || {
                    let result = super::sticker_import::import_signal_pack(&url, &packs);
                    let _ = commands.send(Command::StickerPackImported { result });
                });
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
            Command::PickMediaDir => {
                let commands = self.commands.clone();
                tokio::task::spawn_blocking(move || {
                    if let Some(folder) = rfd::FileDialog::new()
                        .set_title("Select Download Directory")
                        .pick_folder()
                    {
                        let _ = commands.send(Command::SetMediaDir(folder));
                    }
                });
            }
            Command::SetMediaDir(dir) => {
                if self.dirs.is_default_media_dir(&dir) {
                    self.dirs.custom_media = None;
                    self.relocate_media();
                    self.emit(Event::MediaDirChanged(None));
                    return;
                }
                if self.dirs.is_cache_path(&dir) {
                    self.emit(Event::Error(
                        "Custom attachment folder cannot be inside the cache directory".to_owned(),
                    ));
                    return;
                }
                let _ = std::fs::create_dir_all(&dir);
                self.dirs.custom_media = Some(dir.clone());
                self.relocate_media();
                self.emit(Event::MediaDirChanged(Some(dir)));
            }
            Command::ResetMediaDir => {
                self.dirs.custom_media = None;
                self.relocate_media();
                self.emit(Event::MediaDirChanged(None));
            }
            Command::SaveContact {
                id,
                full_name,
                first_name,
                to_phone,
            } => {
                let (Some(client), Some(jid)) = (self.client.clone(), Self::jid_of(&id)) else {
                    self.emit(Event::Error("Not connected to WhatsApp".to_owned()));
                    return;
                };
                let commands = self.commands.clone();
                tokio::spawn(async move {
                    let error = client
                        .chat_actions()
                        .save_contact(&jid, Some(full_name.clone()), first_name, to_phone)
                        .await
                        .err()
                        .map(|error| error.to_string());
                    let _ = commands.send(Command::ContactSaved {
                        id,
                        name: full_name,
                        error,
                    });
                });
            }
            Command::ContactSaved { id, name, error } => {
                if let Some(error) = error {
                    self.emit(Event::Error(format!("Could not save contact: {error}")));
                    return;
                }
                let contact = Contact {
                    id: id.clone(),
                    full_name: Some(name.clone()),
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
                phone,
                full_name,
                first_name,
                to_phone,
            } => {
                let Some(client) = self.client.clone() else {
                    self.emit(Event::Error("Not connected to WhatsApp".to_owned()));
                    return;
                };
                let commands = self.commands.clone();
                let jid = Jid::pn(&phone);
                tokio::spawn(async move {
                    // Use WhatsApp's registration check before opening the chat.
                    let registered = match client.contacts().is_on_whatsapp(&[jid]).await {
                        Ok(results) => results.iter().any(|result| result.is_registered),
                        Err(error) => {
                            log::debug!("number check failed, trusting the number: {error}");
                            true
                        }
                    };
                    let _ = commands.send(Command::ContactChecked {
                        phone,
                        full_name,
                        first_name,
                        to_phone,
                        registered,
                    });
                });
            }
            Command::ContactChecked {
                phone,
                full_name,
                first_name,
                to_phone,
                registered,
            } => {
                if !registered {
                    self.emit(Event::Error(format!(
                        "{} is not on WhatsApp",
                        crate::util::phone(&phone)
                    )));
                    return;
                }
                let id = format!("{phone}@s.whatsapp.net");
                if let Some(full_name) = full_name.clone() {
                    let _ = self.commands.send(Command::SaveContact {
                        id: id.clone(),
                        full_name,
                        first_name,
                        to_phone,
                    });
                }
                self.emit(Event::ContactReady {
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
                if receipts {
                    self.mark_played(chat, message, sender);
                }
            }
            Command::SendGif { chat, gif } => self.send_gif(chat, gif),
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
            Command::InspectUpdate => {
                let events = self.events.clone();
                let waker = self.waker.clone();
                tokio::task::spawn_blocking(move || {
                    let result =
                        crate::updates::install::detect().map_err(|error| format!("{error:#}"));
                    let _ = events.send(Event::UpdateSupport(result));
                    waker.wake();
                });
            }
            Command::DownloadUpdate { release, source } => {
                let events = self.events.clone();
                let waker = self.waker.clone();
                tokio::task::spawn_blocking(move || {
                    let result = crate::updates::download(&release, &source, |received, total| {
                        let _ = events.send(Event::UpdateProgress { received, total });
                        waker.wake();
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
                    let result = crate::updates::install::handoff(&prepared, arguments)
                        .map_err(|error| format!("{error:#}"));
                    let _ = events.send(Event::UpdateInstalling(result));
                    waker.wake();
                });
            }
            Command::CheckForUpdates => {
                let events = self.events.clone();
                let waker = self.waker.clone();
                tokio::task::spawn_blocking(move || match crate::updates::newer_release() {
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
                });
            }
            Command::GifResults { query, results } => {
                self.emit(Event::Gifs { query, results });
            }
            Command::RecentStickers => {
                self.fetch_missing_stickers();
                self.emit_stickers();
            }
            Command::StickerFetched { hash, result } => {
                self.sticker_fetches.remove(&hash);
                match result {
                    Ok(path) => {
                        if let Err(error) = self.archive.set_sticker_path(&hash, &path) {
                            log::warn!("could not file sticker {hash}: {error}");
                        }
                    }
                    Err(error) => log::warn!("sticker {hash} could not be fetched: {error}"),
                }
                self.emit_stickers();
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
                self.emit(Event::Me {
                    id: self.me(),
                    name: self.me_name.clone(),
                    about: self.me_about.clone(),
                });
            }
            Command::React {
                chat,
                message,
                emoji,
            } => self.react(chat, message, emoji),
            Command::SetArchived(chat, archived) => {
                let _ = self.archive.set_archived(&chat, archived);
                self.emit_chat(&chat);
                self.tell_phone(&chat, move |client, jid| async move {
                    if archived {
                        client.chat_actions().archive_chat(&jid, None).await
                    } else {
                        client.chat_actions().unarchive_chat(&jid, None).await
                    }
                    .map_err(|error| error.to_string())
                });
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
            Command::SetMuted(chat, until) => {
                let _ = self.archive.set_muted(&chat, until);
                self.emit_chat(&chat);
                self.tell_phone(&chat, move |client, jid| async move {
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
            Command::Reconnect => {
                if let Some(client) = self.client.clone() {
                    tokio::spawn(async move { client.reconnect_immediately().await });
                } else {
                    self.start_bot().await;
                }
            }
            Command::Shutdown => {}
            Command::OlderFailed { chat, error } => {
                self.pending_older.remove(&chat);
                self.emit(Event::OlderFetched { chat, more: true });
                self.emit(Event::Error(error));
            }
            Command::GroupInfoFailed { chat, permanent } => {
                self.handle_failed_group(chat, permanent);
            }
            Command::Sent { chat, id, error } => {
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
                let _ = self
                    .archive
                    .set_status(&chat, &id, status, crate::util::now());
                self.emit_message(&chat, &id);
                self.emit_chat(&chat);
                if let Some(error) = error {
                    self.emit(Event::Error(format!("Message not sent: {error}")));
                }
            }
            Command::Downloaded { chat, id, result } => {
                if let Ok(path) = &result {
                    let _ = self.archive.set_media_path(&chat, &id, path);
                }
                let for_picker = self.sticker_downloads.remove(&(chat.clone(), id.clone()));
                self.emit(Event::Media {
                    chat,
                    message: id,
                    result,
                });
                if for_picker {
                    self.emit_stickers();
                }
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
                participants,
                read_only,
                ephemeral_expiration,
                ephemeral_setting_timestamp,
            } => {
                self.group_info_tries.remove(&chat);
                let _ =
                    self.archive
                        .set_group_info(&chat, name.as_deref(), &participants, read_only);
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

    fn send_text(
        &mut self,
        chat: ChatId,
        text: String,
        quoting: Option<String>,
        mentions: Vec<String>,
    ) {
        let (Some(client), Some(jid)) = (self.client.clone(), Self::jid_of(&chat)) else {
            self.emit(Event::Error("Not connected to WhatsApp".to_owned()));
            return;
        };
        let mut quoted_row = None;
        let context = quoting.as_deref().and_then(|id| {
            let raw = self.archive.raw(&chat, id).ok().flatten()?;
            let quoted = wa::Message::decode_from_slice(&raw).ok()?;
            let row = self.archive.message(&chat, id).ok().flatten()?;
            let sender = Self::jid_of(&row.sender).unwrap_or_else(|| jid.clone());
            let context = whatsapp_rust::wacore::proto_helpers::build_quote_context_with_info(
                row.id.clone(),
                &sender,
                &jid,
                &jid,
                &quoted,
            );
            quoted_row = Some(row);
            Some(context)
        });
        let mut message = outgoing_text(text.clone(), context, &mentions);
        let expiration = self.apply_ephemeral(&chat, &mut message);
        let mentions = self.mentions_of(&mentions);
        let id = client.generate_message_id();
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
            quoted: quoted_row.map(|row| Quoted {
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
            }),
            reactions: Vec::new(),
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

    fn forward_message(&mut self, from_chat: ChatId, message_id: String, to_chat: ChatId) {
        let (Some(client), Some(jid)) = (self.client.clone(), Self::jid_of(&to_chat)) else {
            self.emit(Event::Error("Not connected to WhatsApp".to_owned()));
            return;
        };
        let Ok(Some(source)) = self.archive.message(&from_chat, &message_id) else {
            self.emit(Event::Error(
                "This message is not stored on this computer".to_owned(),
            ));
            return;
        };
        if matches!(
            source.content,
            Content::Revoked | Content::Unsupported { .. } | Content::Poll { .. }
        ) {
            self.emit(Event::Error("This message cannot be forwarded".to_owned()));
            return;
        }
        let Ok(Some(raw)) = self.archive.raw(&from_chat, &message_id) else {
            self.emit(Event::Error(
                "The original message data is not available to forward".to_owned(),
            ));
            return;
        };
        let Ok(original) = wa::Message::decode_from_slice(&raw) else {
            self.emit(Event::Error(
                "The original message data could not be read".to_owned(),
            ));
            return;
        };
        // whatsapp-rust owns the forwarding rules: unwrap transient wrappers,
        // strip quote chains and secrets, and retain reusable media metadata.
        let (message, expiration) =
            outgoing_forward(&original, self.ephemeral_expiration(&to_chat));
        let id = client.generate_message_id();
        let mentions = self.mentions_of(&mentioned_of(&message));
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
        if row.unread == 0 {
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
            let sender = self.canonical_str(&quoted.sender);
            if sender != quoted.sender || quoted.sender_name.is_none() {
                quoted.sender_name = self.name_for(&sender);
                quoted.sender = sender;
            }
            quoted.summary = self.pn_tokens(&quoted.summary);
            for mention in &mut quoted.mentions {
                mention.id = self.canonical_str(&mention.id);
            }
            if quoted.mentions.is_empty() {
                quoted.mentions = self.mention_tokens(&quoted.summary);
            }
        }
        for mention in &mut message.mentions {
            mention.id = self.canonical_str(&mention.id);
        }
    }

    fn load_chat(&mut self, chat: ChatId, before: Option<super::PageKey>) {
        match self.archive.messages(
            &chat,
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
        let Some(client) = self.client.clone() else {
            self.emit(Event::Media {
                chat,
                message: id,
                result: Err("Not connected to WhatsApp".to_owned()),
            });
            return;
        };
        let raw = self.archive.raw(&chat, &id).ok().flatten();
        let Some(message) = raw.and_then(|raw| wa::Message::decode_from_slice(&raw).ok()) else {
            self.emit(Event::Media {
                chat,
                message: id,
                result: Err("Attachment download keys are missing".to_owned()),
            });
            return;
        };
        let base = message.get_base_message().clone();
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
                self.emit(Event::Media {
                    chat,
                    message: id,
                    result: Err("This message has no downloadable file".to_owned()),
                });
                return;
            };
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
        let dir = self.dirs.media_dir();
        let commands = self.commands.clone();
        tokio::spawn(async move {
            let keep = |bytes: Vec<u8>| {
                let dir = dir.clone();
                let path = media_path(&dir, &chat, &id, &mime, file_name.as_deref());
                async move {
                    tokio::fs::create_dir_all(&dir)
                        .await
                        .map_err(|error| error.to_string())?;
                    tokio::fs::write(&path, &bytes)
                        .await
                        .map_err(|error| error.to_string())?;
                    Ok(path)
                }
            };
            let result = match client.download(&*downloadable).await {
                Ok(bytes) => keep(bytes).await,
                Err(error) => {
                    let text = error.to_string();
                    let expired = ["403", "404", "410"].iter().any(|code| text.contains(code));
                    match (&jid, expired && !media_key.is_empty()) {
                        (Some(jid), true) => {
                            // Ask the phone to re-upload expired media, then retry once.
                            let request = MediaReuploadRequest {
                                msg_id: &id,
                                chat_jid: jid,
                                media_key: &media_key,
                                is_from_me,
                                participant: participant.as_ref(),
                            };
                            match client.media_reupload().request(&request).await {
                                Ok(MediaRetryResult::Success { direct_path }) => {
                                    match refreshed(direct_path) {
                                        Some(again) => match client.download(&*again).await {
                                            Ok(bytes) => keep(bytes).await,
                                            Err(error) => Err(error.to_string()),
                                        },
                                        None => Err(text),
                                    }
                                }
                                Ok(_) => {
                                    Err("No longer available on WhatsApp's servers".to_owned())
                                }
                                Err(error) => {
                                    log::info!("media re-upload was not granted: {error}");
                                    Err("No longer available on WhatsApp's servers".to_owned())
                                }
                            }
                        }
                        _ => Err(text),
                    }
                }
            };
            let _ = commands.send(Command::Downloaded { chat, id, result });
        });
    }

    /// Downloads missing recent and archived stickers for the picker.
    fn fetch_missing_stickers(&mut self) {
        let Some(client) = self.client.clone() else {
            return;
        };
        let phone = match self.archive.phone_stickers() {
            Ok(list) => list,
            Err(error) => {
                log::warn!("could not list the phone's stickers: {error}");
                Vec::new()
            }
        };
        let dir = self.dirs.sticker_cache_dir();
        for sticker in phone.into_iter().filter(|sticker| sticker.path.is_none()) {
            if !self.sticker_fetches.insert(sticker.hash.clone()) {
                continue;
            }
            let Ok(meta) = wa::StickerMetadata::decode_from_slice(&sticker.raw) else {
                self.sticker_fetches.remove(&sticker.hash);
                continue;
            };
            let client = client.clone();
            let commands = self.commands.clone();
            let dir = dir.clone();
            let hash = sticker.hash;
            tokio::spawn(async move {
                let result = async {
                    let bytes = client
                        .download(&PhoneSticker(meta))
                        .await
                        .map_err(|error| error.to_string())?;
                    tokio::fs::create_dir_all(&dir)
                        .await
                        .map_err(|error| error.to_string())?;
                    let path = dir.join(format!("{hash}.webp"));
                    tokio::fs::write(&path, &bytes)
                        .await
                        .map_err(|error| error.to_string())?;
                    Ok(path)
                }
                .await;
                let _ = commands.send(Command::StickerFetched { hash, result });
            });
        }
        match self.archive.stickers_without_file(STICKER_FETCH_LIMIT) {
            Ok(list) => {
                for (chat, id) in list {
                    if self.sticker_downloads.insert((chat.clone(), id.clone())) {
                        self.download(chat, id);
                    }
                }
            }
            Err(error) => log::warn!("could not list unfetched stickers: {error}"),
        }
    }

    /// Returns distinct downloaded stickers by most recent use.
    fn emit_stickers(&mut self) {
        let mut seen = HashSet::new();
        let mut list: Vec<(i64, PathBuf)> = Vec::new();
        if let Ok(phone) = self.archive.phone_stickers() {
            for sticker in phone {
                if let Some(path) = sticker.path
                    && path.exists()
                    && seen.insert(sticker.hash)
                {
                    list.push((sticker.last_used, path));
                }
            }
        }
        match self.archive.recent_stickers(80) {
            Ok(rows) => {
                for sticker in rows {
                    let hash = sticker
                        .raw
                        .as_deref()
                        .and_then(|raw| wa::Message::decode_from_slice(raw).ok())
                        .and_then(|message| {
                            let base = message.get_base_message();
                            let sticker = base.sticker_message.as_option()?;
                            sticker_hash(
                                sticker.file_sha256.as_deref(),
                                sticker.file_enc_sha256.as_deref(),
                            )
                        })
                        .unwrap_or_else(|| sticker.path.display().to_string());
                    if seen.insert(hash) {
                        list.push((sticker.last_used, sticker.path));
                    }
                }
            }
            Err(error) => log::warn!("could not list stickers: {error}"),
        }
        list.sort_by_key(|(when, _)| std::cmp::Reverse(*when));
        self.emit(Event::Stickers {
            saved: self.saved_stickers(),
            packs: self.sticker_packs(),
            recent: list.into_iter().map(|(_, path)| path).collect(),
        });
    }

    /// Root directory for imported sticker packs.
    fn packs_dir(&self) -> PathBuf {
        self.dirs.saved_sticker_dir().join("packs")
    }

    /// Returns imported packs, newest first, with files in name order.
    fn sticker_packs(&self) -> Vec<crate::model::StickerPack> {
        let Ok(entries) = std::fs::read_dir(self.packs_dir()) else {
            return Vec::new();
        };
        let mut packs: Vec<(std::time::SystemTime, crate::model::StickerPack)> = entries
            .flatten()
            .filter_map(|entry| {
                let dir = entry.path();
                if !dir.is_dir() {
                    return None;
                }
                let mut stickers: Vec<PathBuf> = std::fs::read_dir(&dir)
                    .ok()?
                    .flatten()
                    .map(|file| file.path())
                    .filter(|path| {
                        path.extension()
                            .is_some_and(|extension| extension == "webp")
                    })
                    .collect();
                if stickers.is_empty() {
                    return None;
                }
                stickers.sort();
                let when = entry
                    .metadata()
                    .and_then(|metadata| metadata.modified())
                    .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                Some((
                    when,
                    crate::model::StickerPack {
                        name: entry.file_name().to_string_lossy().into_owned(),
                        dir,
                        stickers,
                    },
                ))
            })
            .collect();
        packs.sort_by_key(|(when, _)| std::cmp::Reverse(*when));
        packs.into_iter().map(|(_, pack)| pack).collect()
    }

    /// Returns saved sticker files, newest first.
    fn saved_stickers(&self) -> Vec<PathBuf> {
        let Ok(entries) = std::fs::read_dir(self.dirs.saved_sticker_dir()) else {
            return Vec::new();
        };
        let mut saved: Vec<(std::time::SystemTime, PathBuf)> = entries
            .flatten()
            .filter_map(|entry| {
                let path = entry.path();
                path.extension()
                    .is_some_and(|extension| extension == "webp")
                    .then(|| {
                        let when = entry
                            .metadata()
                            .and_then(|metadata| metadata.modified())
                            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                        (when, path)
                    })
            })
            .collect();
        saved.sort_by_key(|(when, _)| std::cmp::Reverse(*when));
        saved.into_iter().map(|(_, path)| path).collect()
    }

    /// Saves a sticker under its content hash to deduplicate copies.
    fn save_sticker(&self, path: &Path) -> Result<(), String> {
        use sha2::{Digest, Sha256};
        let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
        let hash: String = Sha256::digest(&bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        let dir = self.dirs.saved_sticker_dir();
        std::fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
        let target = dir.join(format!("{hash}.webp"));
        if !target.exists() {
            std::fs::write(&target, &bytes).map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    fn avatar_file(&self, id: &str, full: bool) -> PathBuf {
        self.dirs.avatar_file(id, full)
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
            .is_some_and(|client| client.is_connected());
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
                        match client.contacts().get_profile_picture(jid, preview).await {
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
                let url = picture.url;
                let bytes = tokio::task::spawn_blocking(move || {
                    ureq::get(&url)
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
                Ok::<Option<PathBuf>, String>(Some(path.clone()))
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
            .is_some_and(|client| client.is_connected())
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

    fn load_until(&mut self, chat: ChatId, id: String, before: super::PageKey) {
        let Ok(Some(target)) = self.archive.message(&chat, &id) else {
            self.emit(Event::Messages {
                chat: chat.clone(),
                messages: Vec::new(),
                older: true,
                complete: false,
            });
            self.emit(Event::Error(
                "This message is not stored on this computer".to_owned(),
            ));
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
            Err(error) => self.emit(Event::Error(format!("Could not read the chat: {error}"))),
        }
    }

    fn edit_text(&mut self, chat: ChatId, id: String, text: String, mentions: Vec<String>) {
        let (Some(client), Some(jid)) = (self.client.clone(), Self::jid_of(&chat)) else {
            self.emit(Event::Error("Not connected to WhatsApp".to_owned()));
            return;
        };
        let content = Content::text(text.clone());
        let mention_rows = self.mentions_of(&mentions);
        if let Ok(true) = self
            .archive
            .set_edited_text(&chat, &id, &content, &mention_rows)
        {
            self.emit_message(&chat, &id);
            self.emit_chat(&chat);
        }
        let mut message = outgoing_text(text, None, &mentions);
        self.apply_ephemeral(&chat, &mut message);
        let commands = self.commands.clone();
        tokio::spawn(async move {
            if let Err(error) = client.edit_message(jid, id.clone(), message).await {
                let _ = commands.send(Command::Sent {
                    chat,
                    id: String::new(),
                    error: Some(format!("Could not send the edit: {error}")),
                });
            }
        });
    }

    fn revoke(&mut self, chat: ChatId, id: String) {
        let (Some(client), Some(jid)) = (self.client.clone(), Self::jid_of(&chat)) else {
            self.emit(Event::Error("Not connected to WhatsApp".to_owned()));
            return;
        };
        if let Ok(true) = self
            .archive
            .set_content(&chat, &id, &Content::Revoked, false)
        {
            self.emit_message(&chat, &id);
            self.emit_chat(&chat);
        }
        let commands = self.commands.clone();
        tokio::spawn(async move {
            if let Err(error) = client.revoke_message(jid, id, RevokeType::Sender).await {
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

    fn send_files(
        &mut self,
        chat: ChatId,
        paths: Vec<PathBuf>,
        caption: Option<String>,
        mentions: Vec<String>,
    ) {
        for (index, path) in paths.into_iter().enumerate() {
            let Some(client) = self.client.clone() else {
                self.emit(Event::Error("Not connected to WhatsApp".to_owned()));
                return;
            };
            let commands = self.commands.clone();
            let chat = chat.clone();
            let dir = self.dirs.media_dir();
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
                    let prepared =
                        prepare_media(&client, bytes, &mime, file_name.as_deref(), false).await?;
                    file_outbound(&client, &chat, &me, &dir, prepared, caption, mentions).await
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

    fn send_pasted_image(
        &mut self,
        chat: ChatId,
        width: u32,
        height: u32,
        rgba: Vec<u8>,
        caption: Option<String>,
        mentions: Vec<String>,
    ) {
        let Some(client) = self.client.clone() else {
            self.emit(Event::Error("Not connected to WhatsApp".to_owned()));
            return;
        };
        let commands = self.commands.clone();
        let dir = self.dirs.media_dir();
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
                let prepared = prepare_media(&client, encoded, "image/jpeg", None, false).await?;
                file_outbound(&client, &chat, &me, &dir, prepared, caption, mentions).await
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
    fn send_voice(&mut self, chat: ChatId, samples: Vec<f32>, quoting: Option<String>) {
        let Some(client) = self.client.clone() else {
            self.emit(Event::Error("Not connected to WhatsApp".to_owned()));
            return;
        };
        let quote = quoting.as_deref().and_then(|id| {
            let raw = self.archive.raw(&chat, id).ok().flatten()?;
            let quoted = wa::Message::decode_from_slice(&raw).ok()?;
            let row = self.archive.message(&chat, id).ok().flatten()?;
            let jid = Self::jid_of(&chat)?;
            let sender = Self::jid_of(&row.sender).unwrap_or_else(|| jid.clone());
            let context = whatsapp_rust::wacore::proto_helpers::build_quote_context_with_info(
                row.id.clone(),
                &sender,
                &jid,
                &jid,
                &quoted,
            );
            let shown = Quoted {
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
            Some((context, shown))
        });
        let (context, shown) = match quote {
            Some((context, shown)) => (Some(Box::new(context)), Some(shown)),
            None => (None, None),
        };
        let commands = self.commands.clone();
        let dir = self.dirs.media_dir();
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
            if !receipts_allowed(&client, &jid, &commands).await {
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

    fn send_sticker(&mut self, chat: ChatId, path: PathBuf) {
        let Some(client) = self.client.clone() else {
            self.emit(Event::Error("Not connected to WhatsApp".to_owned()));
            return;
        };
        let commands = self.commands.clone();
        let dir = self.dirs.media_dir();
        let me = self.me();
        tokio::spawn(async move {
            let outcome = async {
                let bytes = tokio::fs::read(&path)
                    .await
                    .map_err(|error| error.to_string())?;
                let prepared = prepare_sticker(&client, bytes).await?;
                file_outbound(&client, &chat, &me, &dir, prepared, None, Vec::new()).await
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

    fn send_gif(&mut self, chat: ChatId, gif: Gif) {
        let Some(client) = self.client.clone() else {
            self.emit(Event::Error("Not connected to WhatsApp".to_owned()));
            return;
        };
        let commands = self.commands.clone();
        let dir = self.dirs.media_dir();
        let me = self.me();
        tokio::spawn(async move {
            let outcome = async {
                let url = gif.mp4.clone();
                let bytes = tokio::task::spawn_blocking(move || {
                    ureq::get(&url)
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
                file_outbound(&client, &chat, &me, &dir, prepared, None, Vec::new()).await
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
        let raw = message.encode_to_vec();
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
        let key = wa::MessageKey {
            remote_jid: Some(chat.clone()),
            from_me: Some(target.from_me),
            id: Some(id),
            participant: (jid.is_group() && !target.from_me).then(|| target.sender.clone()),
        };
        tokio::spawn(async move {
            if let Err(error) = client.send_reaction(jid, key, &emoji).await {
                log::warn!("reaction not sent: {error}");
            }
        });
    }
}

// --- free helpers ----------------------------------------------------------

fn outgoing_forward(original: &wa::Message, expiration: Option<u32>) -> (wa::Message, Option<u32>) {
    let mut message = *original.get_base_message().prepare_for_forward();
    if let Some(mut context) = context_of(&message).cloned() {
        // A forward belongs to the destination chat. The library retains the
        // source timer, including when the destination has no timer at all.
        context.expiration = None;
        context.ephemeral_setting_timestamp = None;
        context.ephemeral_shared_secret = None;
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
                .query_info(&jid)
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
        let mut options = SendOptions::default().with_message_id(id.clone());
        if let Some(expiration) = ephemeral_expiration {
            options = options.with_ephemeral_expiration(expiration);
        }
        client
            .send_message_with_options(jid, message, options)
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
        None if ChatKind::from_id(id) == ChatKind::Group => "Group".to_owned(),
        None => id.split('@').next().unwrap_or(id).to_owned(),
    }
}

/// Normalizes WhatsApp timestamps to seconds.
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

fn extension_for(mime: &str, file_name: Option<&str>) -> String {
    if let Some(extension) = file_name
        .and_then(|name| Path::new(name).extension())
        .and_then(|extension| extension.to_str())
        .filter(|extension| !extension.is_empty() && extension.len() <= 8)
    {
        return extension.to_ascii_lowercase();
    }
    let mime = mime.split(';').next().unwrap_or(mime).trim();
    match mime {
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
        _ => mime.rsplit('/').next().unwrap_or("bin"),
    }
    .to_owned()
}

fn media_path(dir: &Path, chat: &str, id: &str, mime: &str, file_name: Option<&str>) -> PathBuf {
    let extension = extension_for(mime, file_name);
    let stem = match file_name.and_then(|name| Path::new(name).file_stem()?.to_str()) {
        Some(name) => format!("{}-{}", sanitize(id), sanitize(name)),
        None => format!("{}-{}", sanitize(chat), sanitize(id)),
    };
    dir.join(format!("{stem}.{extension}"))
}

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
        path: None,
        state: Default::default(),
    }
}

fn non_empty(text: &Option<String>) -> Option<String> {
    text.clone().filter(|text| !text.trim().is_empty())
}

/// Builds a text body with optional quote and mention context.
fn outgoing_text(
    text: String,
    mut context: Option<wa::ContextInfo>,
    mentions: &[String],
) -> wa::Message {
    if !mentions.is_empty() {
        context.get_or_insert_default().mentioned_jid = mentions.to_vec();
    }
    match context {
        Some(context) => wa::Message::text_with_context(text, context),
        None => wa::Message::text(text),
    }
}

/// Extracts quote and mention context from a message.
fn context_of(base: &wa::Message) -> Option<&wa::ContextInfo> {
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
    None
}

/// Returns raw JIDs mentioned by a message.
fn mentioned_of(base: &wa::Message) -> Vec<String> {
    context_of(base)
        .map(|context| context.mentioned_jid.clone())
        .unwrap_or_default()
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
    } else if let Some(text) = base.extended_text_message.as_option() {
        text.jpeg_thumbnail.clone()
    } else {
        None
    };
    bytes.filter(|bytes| !bytes.is_empty())
}

/// Converts a protocol message to visible content, or `None` for internal traffic.
fn classify(base: &wa::Message) -> Option<Content> {
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
            let url = non_empty(&extended.matched_text).or_else(|| first_link(text))?;
            let url = if url.contains("://") {
                url
            } else {
                format!("https://{url}")
            };
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
        return Some(Content::Image {
            caption: non_empty(&image.caption),
            media: media(
                image.mimetype.as_ref(),
                image.file_length,
                image.width,
                image.height,
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
            media: media(
                video.mimetype.as_ref(),
                video.file_length,
                video.width,
                video.height,
            ),
            seconds: video.seconds,
            gif: video.gif_playback.unwrap_or(false),
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
        return Some(Content::Location {
            latitude: location.degrees_latitude.unwrap_or(0.0),
            longitude: location.degrees_longitude.unwrap_or(0.0),
            name: non_empty(&location.name),
            address: non_empty(&location.address),
        });
    }
    if let Some(live) = base.live_location_message.as_option() {
        return Some(Content::Location {
            latitude: live.degrees_latitude.unwrap_or(0.0),
            longitude: live.degrees_longitude.unwrap_or(0.0),
            name: Some("Live location".to_owned()),
            address: None,
        });
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
    let unsupported = |what: &str| {
        Some(Content::Unsupported {
            what: what.to_owned(),
        })
    };
    if base.album_message.is_set() {
        return None;
    }
    if base.group_invite_message.is_set() {
        return unsupported("group invite");
    }
    if base.event_message.is_set() {
        return unsupported("event");
    }
    if base.sticker_pack_message.is_set() {
        return unsupported("sticker pack");
    }
    if base.interactive_message.is_set()
        || base.buttons_message.is_set()
        || base.list_message.is_set()
        || base.template_message.is_set()
        || base.buttons_response_message.is_set()
        || base.list_response_message.is_set()
        || base.interactive_response_message.is_set()
        || base.template_button_reply_message.is_set()
    {
        return unsupported("interactive message");
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
    if base.call_log_messsage.is_set() || base.scheduled_call_creation_message.is_set() {
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
        || base.message_context_info.is_set()
        || base.device_sent_message.is_set()
        || base.placeholder_message.is_set()
        || base.secret_encrypted_message.is_set()
        || base.message_history_bundle.is_set()
        || base.message_history_notice.is_set()
        || base.bot_invoke_message.is_set()
    {
        return None;
    }
    if *base == wa::Message::default() {
        return None;
    }
    unsupported("message")
}

/// Uploaded attachment protobuf and archive content.
struct Prepared {
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
    let message = audio_message(
        upload,
        AudioOptions {
            mimetype: Some(mime.clone()),
            duration_seconds: Some(seconds),
            ptt: Some(true),
            waveform: Some(waveform.clone()),
            context_info: context,
        },
    );
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

/// Uploads a file and builds its message. Images are encoded as JPEG.
async fn prepare_media(
    client: &Client,
    bytes: Vec<u8>,
    mime: &str,
    file_name: Option<&str>,
    gif: bool,
) -> Result<Prepared, String> {
    let kind = mime.split('/').next().unwrap_or_default();
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
        let mut message = image_message(
            upload,
            ImageOptions {
                caption: None,
                mimetype: Some("image/jpeg".to_owned()),
                jpeg_thumbnail: thumbnail.clone(),
                context_info: None,
            },
        );
        if let Some(image) = message.image_message.as_option_mut() {
            image.width = Some(width);
            image.height = Some(height);
        }
        return Ok(Prepared {
            message,
            content: Content::Image {
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
    if kind == "video" {
        let upload = client
            .upload(bytes.clone(), MediaType::Video, UploadOptions::default())
            .await
            .map_err(|error| error.to_string())?;
        let message = video_message(
            upload,
            VideoOptions {
                mimetype: Some(mime_owned.clone()),
                gif_playback: Some(gif),
                ..Default::default()
            },
        );
        return Ok(Prepared {
            message,
            content: Content::Video {
                caption: None,
                media: media(Some(&mime_owned), Some(size), None, None),
                seconds: None,
                gif,
            },
            thumbnail: None,
            bytes,
            mime: mime_owned,
            file_name: file_name.map(str::to_owned),
        });
    }
    if kind == "audio" {
        let upload = client
            .upload(bytes.clone(), MediaType::Audio, UploadOptions::default())
            .await
            .map_err(|error| error.to_string())?;
        let message = audio_message(
            upload,
            AudioOptions {
                mimetype: Some(mime_owned.clone()),
                ptt: Some(false),
                ..Default::default()
            },
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
    let upload = client
        .upload(bytes.clone(), MediaType::Document, UploadOptions::default())
        .await
        .map_err(|error| error.to_string())?;
    let name = file_name.unwrap_or("file").to_owned();
    let message = document_message(
        upload,
        DocumentOptions {
            mimetype: Some(mime_owned.clone()),
            file_name: Some(name.clone()),
            title: Some(name.clone()),
            ..Default::default()
        },
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
    let message = wa::Message {
        sticker_message: MessageField::some(wa::message::StickerMessage {
            url: Some(upload.url),
            direct_path: Some(upload.direct_path),
            media_key: Some(upload.media_key.to_vec()),
            file_enc_sha256: Some(upload.file_enc_sha256.to_vec()),
            file_sha256: Some(upload.file_sha256.to_vec()),
            file_length: Some(upload.file_length),
            mimetype: Some("image/webp".to_owned()),
            media_key_timestamp: Some(upload.media_key_timestamp),
            is_animated: Some(animated),
            width: Some(width),
            height: Some(height),
            ..Default::default()
        }),
        ..Default::default()
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
    let body = match ureq::get(&url).call() {
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
async fn file_outbound(
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
    if !mentions.is_empty() {
        prepared.message.set_context_info(wa::ContextInfo {
            mentioned_jid: mentions.clone(),
            ..Default::default()
        });
    }
    let id = client.generate_message_id();
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
    let mut content = prepared.content;
    if let Some(media) = content.media_mut() {
        media.path = Some(path);
    }
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
        edited: false,
        mentions: mentions
            .into_iter()
            .filter_map(|id| {
                let user = id.split('@').next()?.to_owned();
                (!user.is_empty()).then_some(MentionRef { user, id })
            })
            .collect(),
        forwarded: false,
        thumbnail: prepared.thumbnail,
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

fn parse_conversation(conversation: wa::Conversation) -> ParsedChat {
    let mut messages = Vec::new();
    let mut revoked = Vec::new();
    let mut poll_updates = Vec::new();
    let mut reactions = Vec::new();
    let mut newest = 0;
    for entry in &conversation.messages {
        let Some(info) = entry.message.as_option() else {
            continue;
        };
        let Some(key) = info.key.as_option() else {
            continue;
        };
        let Some(id) = key.id.clone().filter(|id| !id.is_empty()) else {
            continue;
        };
        let Some(message) = info.message.as_option() else {
            continue;
        };
        let from_me = key.from_me.unwrap_or(false);
        let timestamp = info.message_timestamp.unwrap_or(0) as i64;
        newest = newest.max(timestamp);
        let base = message.get_base_message();
        if let Some(protocol) = base.protocol_message.as_option() {
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
        let Some(content) = classify(base) else {
            continue;
        };
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
        } else {
            Delivery::None
        };
        // A group's individual receipts may be only a partial list. Only the
        // phone's aggregate status proves delivery/read for historical groups.
        if from_me
            && ChatKind::from_id(&conversation.id) != ChatKind::Group
            && status < Delivery::Read
        {
            if info
                .user_receipt
                .iter()
                .any(|receipt| receipt.read_timestamp.is_some())
            {
                status = Delivery::Read;
            } else if status < Delivery::Delivered
                && info
                    .user_receipt
                    .iter()
                    .any(|receipt| receipt.receipt_timestamp.is_some())
            {
                status = Delivery::Delivered;
            }
        }
        let quoted = context_of(base).and_then(|context| {
            let id = context.stanza_id.clone().filter(|id| !id.is_empty())?;
            Some(Quoted {
                mentions: Vec::new(),
                id,
                sender: context.participant.clone().unwrap_or_default(),
                sender_name: None,
                summary: context
                    .quoted_message
                    .as_option()
                    .and_then(|quoted| classify(quoted.get_base_message()))
                    .map(|content| content.summary())
                    .unwrap_or_default(),
            })
        });
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
            content,
            status,
            quoted,
            reactions,
            mentions: mentioned_of(base),
            forwarded: forwarded_of(base),
            thumbnail: thumbnail_of(base),
            raw: message.encode_to_vec(),
            poll_secret: info.message_secret.clone(),
            poll_votes: info.poll_updates.clone(),
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
        .map(|end| match end {
            End::COMPLETE_BUT_MORE_MESSAGES_REMAIN_ON_PRIMARY
            | End::COMPLETE_ON_DEMAND_SYNC_BUT_MORE_MSG_REMAIN_ON_PRIMARY => true,
            End::COMPLETE_AND_NO_MORE_MESSAGE_REMAIN_ON_PRIMARY
            | End::COMPLETE_ON_DEMAND_SYNC_WITH_MORE_MSG_ON_PRIMARY_BUT_NO_ACCESS => false,
        });
    ParsedChat {
        id: conversation.id.clone(),
        name: non_empty(&conversation.display_name).or_else(|| non_empty(&conversation.name)),
        unread: conversation.unread_count,
        archived: conversation.archived.unwrap_or(false),
        pinned_at: conversation.pinned.map(|when| i64::from(when) * 1000),
        muted_until: conversation.mute_end_time.map(|end| {
            // Zero explicitly clears a history mute; a wrapped -1 means
            // indefinite. Absence of the field must preserve existing state.
            (end != 0).then(|| seconds(end as i64))
        }),
        ephemeral_expiration: conversation.ephemeral_expiration,
        ephemeral_setting_timestamp: conversation.ephemeral_setting_timestamp,
        last_activity,
        pn_jid: conversation.pn_jid.clone(),
        lid_jid: conversation.lid_jid.clone(),
        more_on_phone,
        messages,
        revoked,
        poll_updates,
        reactions,
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_names_read_as_phones_or_ids() {
        assert_eq!(
            fallback_name("393331234567@s.whatsapp.net"),
            "+39 333 123 456 7"
        );
        assert_eq!(fallback_name("1-2@g.us"), "Group");
        assert_eq!(fallback_name("42@lid"), "42");
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
    fn classification_covers_text_and_media() {
        let text = wa::Message::text("hello");
        assert_eq!(classify(&text), Some(Content::text("hello")));
        let image = wa::Message {
            image_message: whatsapp_rust::prelude::MessageField::some(wa::message::ImageMessage {
                caption: Some("look".into()),
                mimetype: Some("image/jpeg".into()),
                file_length: Some(10),
                width: Some(4),
                height: Some(3),
                jpeg_thumbnail: Some(vec![0xff, 0xd8]),
                ..Default::default()
            }),
            ..Default::default()
        };
        match classify(&image) {
            Some(Content::Image { caption, media }) => {
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
    fn link_previews_and_mentions_come_from_extended_text() {
        let message = wa::Message {
            extended_text_message: whatsapp_rust::prelude::MessageField::some(
                wa::message::ExtendedTextMessage {
                    text: Some("see spotifast.rocks @123456@lid".into()),
                    matched_text: Some("https://spotifast.rocks/".into()),
                    title: Some("spotifast.rocks".into()),
                    description: Some("Spotify, native and fast".into()),
                    context_info: whatsapp_rust::prelude::MessageField::some(wa::ContextInfo {
                        mentioned_jid: vec!["123456@lid".into()],
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            ),
            ..Default::default()
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
            Some(wa::ContextInfo {
                stanza_id: Some("quoted".to_owned()),
                ..Default::default()
            }),
            &mentions,
        );

        assert_eq!(message.text_content(), Some("hello @491702222222"));
        let context = context_of(&message).expect("text context");
        assert_eq!(context.stanza_id.as_deref(), Some("quoted"));
        assert_eq!(context.mentioned_jid, mentions);
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
            Some(wa::ContextInfo {
                stanza_id: Some("quoted".to_owned()),
                ..Default::default()
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
        let mut message = wa::Message {
            image_message: MessageField::some(wa::message::ImageMessage {
                caption: Some("look".to_owned()),
                ..Default::default()
            }),
            ..Default::default()
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

    #[test]
    fn forwards_use_only_the_destination_timer() {
        let context = wa::ContextInfo {
            expiration: Some(7_776_000),
            ephemeral_setting_timestamp: Some(123),
            ephemeral_shared_secret: Some(vec![1, 2, 3]),
            is_forwarded: Some(true),
            forwarding_score: Some(2),
            ..Default::default()
        };
        let text = wa::Message::text_with_context("forward me", context.clone());
        let image = wa::Message {
            image_message: MessageField::some(wa::message::ImageMessage {
                caption: Some("caption".into()),
                direct_path: Some("/media/path".into()),
                context_info: MessageField::some(context.clone()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let contacts = wa::Message {
            contacts_array_message: MessageField::some(wa::message::ContactsArrayMessage {
                context_info: MessageField::some(context),
                ..Default::default()
            }),
            ..Default::default()
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
    const PEER: &str = "4917663430455@s.whatsapp.net";
    const PEER_LID: &str = "167650256810092@lid";

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

    pub(super) fn worker() -> (
        Worker,
        std::sync::mpsc::Receiver<Event>,
        mpsc::UnboundedReceiver<Command>,
        mpsc::UnboundedReceiver<Arc<wa_events::Event>>,
    ) {
        let (events, events_rx) = std::sync::mpsc::channel();
        let (commands, inbox) = mpsc::unbounded_channel();
        let (wa_sender, wa_events) = mpsc::unbounded_channel();
        let root = std::env::temp_dir().join(format!("zapfast-worker-test-{}", std::process::id()));
        let worker = Worker {
            dirs: AppDirs::under(&root),
            events,
            commands,
            waker: Waker(Arc::new(std::sync::Mutex::new(None))),
            archive: Archive::in_memory().expect("archive"),
            client: None,
            handle: None,
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
            group_info_queue: std::collections::VecDeque::new(),
            group_info_tries: HashMap::new(),
            group_info_retry: Vec::new(),
            presence_subscribed: HashSet::new(),
            pending_older: HashMap::new(),
            older_warned: HashSet::new(),
            pending_avatars: HashMap::new(),
            sticker_fetches: HashSet::new(),
            sticker_downloads: HashSet::new(),
            read_sync: ReadSync::default(),
            poll_decrypting: 0,
            poll_history: Default::default(),
            poll_sending: HashSet::new(),
        };
        (worker, events_rx, inbox, wa_events)
    }

    fn own_message(id: &str, timestamp: i64) -> Message {
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
            edited: false,
            mentions: Vec::new(),
            forwarded: false,
            thumbnail: None,
        }
    }

    fn receipt(chat: &str, ids: &[&str], kind: ReceiptType) -> wa_events::Receipt {
        let chat: Jid = chat.parse().expect("jid");
        wa_events::Receipt::builder()
            .message_ids(ids.iter().map(|id| (*id).to_owned()).collect())
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

    #[test]
    fn history_keeps_ephemeral_metadata() {
        let parsed = parse_conversation(wa::Conversation {
            id: PEER.into(),
            ephemeral_expiration: Some(7_776_000),
            ephemeral_setting_timestamp: Some(1_700_000_000),
            ..Default::default()
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
        wa::HistorySyncMsg {
            message: MessageField::some(wa::WebMessageInfo {
                key: MessageField::some(wa::MessageKey {
                    remote_jid: Some(chat.into()),
                    from_me: Some(from_me),
                    id: Some(id.into()),
                    participant: participant.map(str::to_owned),
                }),
                message: MessageField::some(message),
                message_timestamp: Some(100),
                reactions,
                message_secret: secret,
                ..Default::default()
            }),
            ..Default::default()
        }
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
        let parsed = parse_conversation(wa::Conversation {
            id: group.into(),
            messages: vec![
                history_entry(
                    group,
                    "photo",
                    false,
                    Some(PEER),
                    wa::Message {
                        conversation: Some("caption".into()),
                        ..Default::default()
                    },
                    Vec::new(),
                    None,
                ),
                history_entry(
                    group,
                    "react",
                    false,
                    Some(reactor),
                    wa::Message {
                        reaction_message: MessageField::some(wa::message::ReactionMessage {
                            key: MessageField::some(wa::MessageKey {
                                remote_jid: Some(group.into()),
                                from_me: Some(false),
                                id: Some("photo".into()),
                                participant: Some(PEER.into()),
                            }),
                            text: Some("🏆".into()),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                    Vec::new(),
                    None,
                ),
            ],
            ..Default::default()
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

    #[test]
    fn history_reads_aggregated_reactions_from_grouping_key() {
        let parsed = parse_conversation(wa::Conversation {
            id: PEER.into(),
            messages: vec![history_entry(
                PEER,
                "photo",
                false,
                None,
                wa::Message {
                    conversation: Some("caption".into()),
                    ..Default::default()
                },
                vec![wa::Reaction {
                    key: MessageField::some(wa::MessageKey {
                        from_me: Some(false),
                        participant: Some(PEER.into()),
                        ..Default::default()
                    }),
                    grouping_key: Some("🏆".into()),
                    ..Default::default()
                }],
                None,
            )],
            ..Default::default()
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
        let raw = wa::Message {
            reaction_message: MessageField::some(wa::message::ReactionMessage {
                key: MessageField::some(wa::MessageKey {
                    remote_jid: Some(PEER.into()),
                    from_me: Some(false),
                    id: Some("photo".into()),
                    ..Default::default()
                }),
                grouping_key: Some("🏆".into()),
                ..Default::default()
            }),
            ..Default::default()
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
        let parent_raw = wa::Message {
            conversation: Some("caption".into()),
            message_context_info: MessageField::some(wa::MessageContextInfo {
                message_secret: Some(secret.to_vec()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let (mut worker, _events, _inbox, _wa) = worker();
        worker.archive.ensure_chat(PEER, "Ada").unwrap();
        worker
            .archive
            .insert_message(&incoming("photo", 10), Some(&parent_raw.encode_to_vec()))
            .unwrap();
        let raw = wa::Message {
            enc_reaction_message: MessageField::some(wa::message::EncReactionMessage {
                target_message_key: MessageField::some(wa::MessageKey {
                    remote_jid: Some(PEER.into()),
                    from_me: Some(false),
                    id: Some("photo".into()),
                    participant: Some(PEER.into()),
                }),
                enc_payload: Some(payload),
                enc_iv: Some(iv.to_vec()),
            }),
            ..Default::default()
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
        for (expiration, setting_time, envelope_time, expected) in [
            (86_400, None, 200, Some(86_400)),
            (0, None, 300, None),
            (604_800, Some(250), 400, None),
        ] {
            let raw = wa::Message {
                protocol_message: MessageField::some(wa::message::ProtocolMessage {
                    r#type: Some(wa::message::protocol_message::Type::EPHEMERAL_SETTING),
                    ephemeral_expiration: Some(expiration),
                    ephemeral_setting_timestamp: setting_time,
                    ..Default::default()
                }),
                ..Default::default()
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
            assert_eq!(
                worker
                    .archive
                    .chat(PEER)
                    .unwrap()
                    .unwrap()
                    .ephemeral_expiration,
                expected
            );
            assert_eq!(worker.ephemeral_expiration(PEER), expected);
        }
        let badges: Vec<_> = events
            .try_iter()
            .filter_map(|event| match event {
                Event::ChatUpdated(chat) if chat.id == PEER => Some(chat.ephemeral_expiration),
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
        for (expiration, timestamp, expected) in [
            (86_400, 200, Some(86_400)),
            (0, 300, None),
            (604_800, 250, None),
        ] {
            let update = wa_events::GroupUpdate::builder()
                .group_jid(group.parse().unwrap())
                .timestamp(whatsapp_rust::wacore::time::from_secs(timestamp).unwrap())
                .is_lid_addressing_mode(false)
                .action(
                    whatsapp_rust::wacore::stanza::groups::GroupNotificationAction::Ephemeral {
                        expiration,
                        trigger: None,
                    },
                )
                .build();
            worker
                .handle_wa_event(Arc::new(wa_events::Event::GroupUpdate(update)))
                .await;
            assert_eq!(
                worker
                    .archive
                    .chat(group)
                    .unwrap()
                    .unwrap()
                    .ephemeral_expiration,
                expected
            );
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
                .state(ChatPresence::Composing)
                .media(whatsapp_rust::types::presence::ChatPresenceMedia::Text)
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
            parse_conversation(wa::Conversation {
                id: chat.into(),
                messages: vec![wa::HistorySyncMsg {
                    message: MessageField::some(wa::WebMessageInfo {
                        key: MessageField::some(wa::MessageKey {
                            id: Some("history".into()),
                            from_me: Some(true),
                            ..Default::default()
                        }),
                        message: MessageField::some(wa::Message {
                            conversation: Some("hello".into()),
                            ..Default::default()
                        }),
                        status: Some(status),
                        user_receipt: vec![wa::UserReceipt {
                            user_jid: PEER.into(),
                            read_timestamp: Some(123),
                            ..Default::default()
                        }],
                        ..Default::default()
                    }),
                    ..Default::default()
                }],
                ..Default::default()
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
            chats: vec![parse_conversation(wa::Conversation {
                id: PEER.into(),
                unread_count: Some(unread),
                conversation_timestamp: Some(200),
                ..Default::default()
            })],
            push_names: Vec::new(),
            lids: Vec::new(),
            stickers: Vec::new(),
        }
    }

    #[test]
    fn history_preserves_pin_time_and_distinguishes_missing_mute_metadata() {
        let chat = parse_conversation(wa::Conversation {
            id: PEER.into(),
            pinned: Some(1_700_000_000),
            mute_end_time: Some(1_800_000_000),
            ..Default::default()
        });
        assert_eq!(chat.pinned_at, Some(1_700_000_000_000));
        assert_eq!(chat.muted_until, Some(Some(1_800_000_000)));
        for (end, expected) in [
            (None, None),
            (Some(0), Some(None)),
            (Some(u64::MAX), Some(Some(0))),
        ] {
            let chat = parse_conversation(wa::Conversation {
                id: PEER.into(),
                mute_end_time: end,
                ..Default::default()
            });
            assert_eq!(chat.muted_until, expected);
        }
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
                .action(Box::new(wa::sync_action_value::MuteAction {
                    muted: Some(enabled),
                    mute_end_timestamp: Some(-1),
                    ..Default::default()
                }))
                .build();
            let pin = wa_events::PinUpdate::builder()
                .jid(PEER.parse().unwrap())
                .timestamp(time)
                .from_full_sync(true)
                .action(Box::new(wa::sync_action_value::PinAction {
                    pinned: Some(enabled),
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
            .action(Box::new(wa::sync_action_value::MarkChatAsReadAction {
                read: Some(true),
                message_range: MessageField::some(whatsapp_rust::message_range(
                    200,
                    None,
                    Vec::new(),
                )),
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
}
