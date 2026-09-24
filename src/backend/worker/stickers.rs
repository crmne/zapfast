//! The sticker picker's lists and their sync with the phone.
//!
//! WhatsApp names a sticker in app-state sync by its `filehash`: the base64
//! SHA-256 of the decrypted file. ZapFast names it by the same digest in hex,
//! which is also the name of every saved or packed copy, so both sides agree
//! on which sticker a change is about.

use super::*;
use whatsapp_rust::schemas;

/// The hex content hash for a WhatsApp `filehash`. WhatsApp writes standard
/// padded base64, but an unpadded or URL-safe digest names the same file.
pub(super) fn hash_of_filehash(filehash: &str) -> Option<String> {
    use base64::Engine;
    use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD};
    let filehash = filehash.trim();
    let bytes = [STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD]
        .iter()
        .find_map(|engine| engine.decode(filehash).ok())?;
    (bytes.len() == 32).then(|| bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

/// The raw SHA-256 behind a hex content hash.
fn digest_of_hash(hash: &str) -> Option<Vec<u8>> {
    if hash.len() != 64 {
        return None;
    }
    (0..32)
        .map(|index| u8::from_str_radix(hash.get(index * 2..index * 2 + 2)?, 16).ok())
        .collect()
}

/// Whether an action carries enough to fetch the sticker from the CDN.
fn fetchable(action: &wa::sync_action_value::StickerAction) -> bool {
    action
        .direct_path
        .as_deref()
        .is_some_and(|path| !path.is_empty())
        || (action.media_key.is_none() && action.url.as_deref().is_some_and(|url| !url.is_empty()))
}

/// A download error without the CDN paths and tokens it may quote.
fn redacted(error: &str) -> String {
    error
        .split_whitespace()
        .map(|word| {
            if word.contains("://") || word.contains("/v/") || word.contains("oh=") {
                "<link>"
            } else {
                word
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The WhatsApp `filehash` for a hex content hash.
pub(super) fn filehash_of_hash(hash: &str) -> Option<String> {
    use base64::Engine;
    Some(base64::engine::general_purpose::STANDARD.encode(digest_of_hash(hash)?))
}

/// How a WhatsApp sticker pack message reads in a chat.
pub(super) fn sticker_pack_content(pack: &wa::message::StickerPackMessage) -> Content {
    Content::StickerPack {
        name: pack.name.clone().unwrap_or_default(),
        publisher: pack.publisher.clone().unwrap_or_default(),
        count: pack.stickers.len() as u32,
        caption: pack
            .caption
            .clone()
            .filter(|caption| !caption.trim().is_empty()),
    }
}

/// WhatsApp accepts at most this many stickers in one pack.
const PACK_LIMIT: usize = 60;

/// Zips, uploads, and builds a sticker pack message from a pack's files.
async fn prepare_sticker_pack(
    client: &Client,
    name: String,
    publisher: String,
    files: Vec<PathBuf>,
) -> Result<Prepared, String> {
    use whatsapp_rust::sticker_pack::{
        StickerInput, StickerPackMetadata, build_sticker_pack_message, create_sticker_pack_zip,
    };
    let mut stickers = Vec::new();
    for path in files.into_iter().take(PACK_LIMIT) {
        let bytes = tokio::fs::read(&path)
            .await
            .map_err(|error| error.to_string())?;
        let emojis = crate::sticker_meta::emojis(&bytes);
        stickers.push((bytes, emojis));
    }
    let first = stickers
        .first()
        .ok_or("This pack has no stickers")?
        .0
        .clone();
    let (cover, thumbnail) =
        tokio::task::spawn_blocking(move || super::super::sticker_import::pack_art(&first))
            .await
            .map_err(|error| error.to_string())?
            .ok_or("Could not draw the pack's cover")?;
    let pack_id: String = rand::random::<[u8; 16]>()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let inputs: Vec<StickerInput<'_>> = stickers
        .iter()
        .map(|(bytes, emojis)| StickerInput::new(bytes).with_emojis(emojis.clone()))
        .collect();
    let zip =
        create_sticker_pack_zip(&pack_id, &inputs, &cover).map_err(|error| error.to_string())?;
    // The zip and its thumbnail share one media key, so upload both together.
    let key: [u8; 32] = rand::random();
    let (zip_upload, thumbnail_upload) = tokio::try_join!(
        client.upload(
            zip.zip_bytes.clone(),
            MediaType::StickerPack,
            UploadOptions::new().with_media_key(key),
        ),
        client.upload(
            thumbnail,
            MediaType::StickerPackThumbnail,
            UploadOptions::new().with_media_key(key),
        ),
    )
    .map_err(|error| error.to_string())?;
    let metadata = StickerPackMetadata::new(pack_id, name, publisher);
    let message =
        build_sticker_pack_message(&zip, &zip_upload.into(), &thumbnail_upload.into(), metadata)
            .map_err(|error| error.to_string())?;
    let content = message
        .sticker_pack_message
        .as_option()
        .map(sticker_pack_content)
        .ok_or("Could not build the pack message")?;
    Ok(Prepared {
        message,
        content,
        thumbnail: None,
        bytes: zip.zip_bytes,
        mime: "application/zip".to_owned(),
        file_name: None,
    })
}

/// Milliseconds since the epoch, as app-state actions carry them.
fn now_millis() -> i64 {
    crate::util::now().saturating_mul(1000)
}

/// How many recent sticker messages to search for a sticker's references.
const REFERENCE_SEARCH: usize = 2000;

/// Archive marker: favorites the phone synced before ZapFast followed them
/// have been replayed.
pub(super) const FAVORITES_RECOVERED: &str = "favorite_stickers_recovered_v1";

/// A favorite sticker the phone told us about, fetched by its references.
/// The filehash is the plaintext SHA-256, which an unencrypted sticker (one
/// the phone has no media key for) needs to be fetched at all.
struct FavoriteDownload {
    action: wa::sync_action_value::StickerAction,
    file_sha256: Vec<u8>,
}

impl Downloadable for FavoriteDownload {
    fn direct_path(&self) -> Option<&str> {
        self.action
            .direct_path
            .as_deref()
            .filter(|path| !path.is_empty())
    }

    fn media_key(&self) -> Option<&[u8]> {
        self.action.media_key.as_deref()
    }

    fn file_enc_sha256(&self) -> Option<&[u8]> {
        self.action.file_enc_sha256.as_deref()
    }

    fn file_sha256(&self) -> Option<&[u8]> {
        Some(&self.file_sha256)
    }

    fn file_length(&self) -> Option<u64> {
        self.action.file_length
    }

    fn app_info(&self) -> MediaType {
        MediaType::Sticker
    }

    fn static_url(&self) -> Option<&str> {
        // Only a plain CDN file without a direct path is fetched by its URL.
        if self.media_key().is_some() || self.direct_path().is_some() {
            return None;
        }
        self.action.url.as_deref().filter(|url| !url.is_empty())
    }
}

/// A favorite change on its way to the phone.
pub(super) struct FavoritePush {
    hash: String,
    favorite: bool,
    updated_at: i64,
    /// Known CDN references, or none when the file must be uploaded first.
    action: Option<wa::sync_action_value::StickerAction>,
    /// The favorite's file, uploaded when no references are known.
    file: PathBuf,
}

/// References from a sticker message, as a favorite action carries them.
fn action_of_message(
    sticker: &wa::message::StickerMessage,
) -> wa::sync_action_value::StickerAction {
    wa::sync_action_value::StickerAction {
        url: sticker.url.clone(),
        file_enc_sha256: sticker.file_enc_sha256.clone(),
        media_key: sticker.media_key.clone(),
        mimetype: sticker.mimetype.clone(),
        height: sticker.height,
        width: sticker.width,
        direct_path: sticker.direct_path.clone(),
        file_length: sticker.file_length,
        is_lottie: sticker.is_lottie,
        is_avatar_sticker: sticker.is_avatar,
        ..Default::default()
    }
}

/// References from the phone's recent-sticker list.
fn action_of_metadata(sticker: &wa::StickerMetadata) -> wa::sync_action_value::StickerAction {
    wa::sync_action_value::StickerAction {
        url: sticker.url.clone(),
        file_enc_sha256: sticker.file_enc_sha256.clone(),
        media_key: sticker.media_key.clone(),
        mimetype: sticker.mimetype.clone(),
        height: sticker.height,
        width: sticker.width,
        direct_path: sticker.direct_path.clone(),
        file_length: sticker.file_length,
        is_lottie: sticker.is_lottie,
        is_avatar_sticker: sticker.is_avatar_sticker,
        ..Default::default()
    }
}

/// Tells the phone about one favorite change, uploading the sticker first
/// when the phone could not fetch it otherwise. Returns the references sent.
async fn push_favorite(client: &Client, push: FavoritePush) -> Result<Vec<u8>, String> {
    let filehash = filehash_of_hash(&push.hash).ok_or("not a sticker hash")?;
    let mut action = push.action.unwrap_or_default();
    if push.favorite && action.direct_path.is_none() {
        let bytes = tokio::fs::read(&push.file)
            .await
            .map_err(|error| error.to_string())?;
        let size = image::ImageReader::new(std::io::Cursor::new(&bytes))
            .with_guessed_format()
            .ok()
            .and_then(|reader| reader.into_dimensions().ok());
        let upload = client
            .upload(bytes, MediaType::Sticker, UploadOptions::default())
            .await
            .map_err(|error| error.to_string())?;
        action = wa::sync_action_value::StickerAction {
            url: Some(upload.url),
            file_enc_sha256: Some(upload.file_enc_sha256.to_vec()),
            media_key: Some(upload.media_key.to_vec()),
            mimetype: Some("image/webp".to_owned()),
            width: size.map(|(width, _)| width),
            height: size.map(|(_, height)| height),
            direct_path: Some(upload.direct_path),
            file_length: Some(upload.file_length),
            ..Default::default()
        };
    }
    action.is_favorite = Some(push.favorite);
    let encoded = action.encode_to_vec();
    let value = wa::SyncActionValue {
        sticker_action: MessageField::some(action),
        timestamp: Some(push.updated_at),
        ..Default::default()
    };
    client
        .send_app_state_action(&schemas::FAVORITE_STICKER, &[&filehash], &value)
        .await
        .map_err(|error| error.to_string())?;
    Ok(encoded)
}

impl Worker {
    /// Sends the picker its lists: saved stickers, packs, and Recent, which
    /// holds the phone's recent stickers and the ones we sent, newest first,
    /// minus those removed from Recent since their last use.
    pub(super) fn emit_stickers(&mut self) {
        let removed = self.archive.removed_recent_stickers().unwrap_or_default();
        let hidden = |hash: &str, used: i64| removed.get(hash).is_some_and(|at| *at >= used);
        let mut seen = HashSet::new();
        let mut list: Vec<(i64, PathBuf, String)> = Vec::new();
        if let Ok(phone) = self.archive.phone_stickers() {
            for sticker in phone {
                if let Some(path) = sticker.path
                    && path.exists()
                    && !hidden(&sticker.hash, sticker.last_used)
                    && seen.insert(sticker.hash.clone())
                {
                    list.push((sticker.last_used, path, sticker.hash));
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
                    if !hidden(&hash, sticker.last_used) && seen.insert(hash.clone()) {
                        list.push((sticker.last_used, sticker.path, hash));
                    }
                }
            }
            Err(error) => log::warn!("could not list stickers: {error}"),
        }
        list.sort_by_key(|(when, _, _)| std::cmp::Reverse(*when));
        self.recent_hashes = list
            .iter()
            .map(|(_, path, hash)| (path.clone(), hash.clone()))
            .collect();
        let favorites = self.saved_stickers();
        let packs = self.sticker_packs();
        let recent: Vec<PathBuf> = list.into_iter().map(|(_, path, _)| path).collect();
        let listed: Vec<PathBuf> = favorites
            .iter()
            .chain(&recent)
            .chain(packs.iter().flat_map(|pack| &pack.stickers))
            .cloned()
            .collect();
        let emojis = self.sticker_emojis(&listed);
        self.emit(Event::Stickers {
            favorites,
            packs,
            recent,
            emojis,
        });
    }

    /// The emojis each sticker file is tagged with, read from its metadata
    /// once per file version.
    fn sticker_emojis(&mut self, paths: &[PathBuf]) -> HashMap<PathBuf, Vec<String>> {
        let mut found = HashMap::new();
        for path in paths {
            let Ok(metadata) = std::fs::metadata(path) else {
                continue;
            };
            let stamp = (metadata.len(), metadata.modified().ok());
            let emojis = match self.emoji_cache.get(path) {
                Some((seen, emojis)) if *seen == stamp => emojis.clone(),
                _ => {
                    let emojis = std::fs::read(path)
                        .map(|bytes| crate::sticker_meta::emojis(&bytes))
                        .unwrap_or_default();
                    self.emoji_cache
                        .insert(path.clone(), (stamp, emojis.clone()));
                    emojis
                }
            };
            if !emojis.is_empty() {
                found.insert(path.clone(), emojis);
            }
        }
        found
    }

    /// Takes a sticker out of Recent here and on the phone.
    pub(super) fn remove_recent_sticker(&mut self, path: &Path) {
        let Some(hash) = self.recent_hashes.get(path).cloned() else {
            return;
        };
        let now = crate::util::now();
        if let Err(error) = self.archive.remove_recent_sticker(&hash, now) {
            log::warn!("could not remove a recent sticker: {error}");
        }
        self.emit_stickers();
        let (Some(client), Some(filehash)) = (self.client.clone(), filehash_of_hash(&hash)) else {
            return;
        };
        tokio::spawn(async move {
            let value = wa::SyncActionValue {
                remove_recent_sticker_action: MessageField::some(
                    wa::sync_action_value::RemoveRecentStickerAction {
                        last_sticker_sent_ts: Some(now_millis()),
                    },
                ),
                timestamp: Some(now_millis()),
                ..Default::default()
            };
            if let Err(error) = client
                .send_app_state_action(&schemas::REMOVE_RECENT_STICKER, &[&filehash], &value)
                .await
            {
                log::warn!("could not remove a recent sticker on the phone: {error}");
            }
        });
    }

    /// Makes a sticker a favorite here and on the phone.
    pub(super) fn favorite_sticker(&mut self, path: &Path) {
        let hash = match super::super::sticker_store::save(&self.dirs.saved_sticker_dir(), path) {
            Ok(hash) => hash,
            Err(error) => {
                self.emit(Event::Error(format!("Could not add to favorites: {error}")));
                return;
            }
        };
        self.favorite_changed_here(hash, true);
    }

    /// Removes a favorite here and on the phone.
    pub(super) fn unfavorite_sticker(&mut self, path: &Path) {
        // Restrict deletion to files in the favorites directory.
        if !path.starts_with(self.dirs.saved_sticker_dir()) {
            return;
        }
        let hash = std::fs::read(path)
            .ok()
            .map(|bytes| super::super::sticker_store::content_hash(&bytes));
        if std::fs::remove_file(path).is_err() {
            return;
        }
        match hash {
            Some(hash) => self.favorite_changed_here(hash, false),
            None => self.emit_stickers(),
        }
    }

    fn favorite_changed_here(&mut self, hash: String, favorite: bool) {
        let now = now_millis();
        if let Err(error) = self
            .archive
            .set_favorite_sticker(&hash, favorite, now, None, false)
        {
            log::warn!("could not record a favorite sticker: {error}");
        }
        self.emit_stickers();
        self.push_favorites();
    }

    /// Sends every favorite change the phone has not seen, one at a time.
    /// Favorites saved before sync existed, or while offline, count too, so
    /// they reach the phone once after linking.
    pub(super) fn push_favorites(&mut self) {
        let Some(client) = self.client.clone() else {
            return;
        };
        if self.favorites_pushing {
            self.favorites_again = true;
            return;
        }
        let dir = self.dirs.saved_sticker_dir();
        // Saved files the sync table has never seen are favorites to send.
        for path in super::super::sticker_store::saved(&dir) {
            let Some(hash) = path
                .file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
            else {
                continue;
            };
            if filehash_of_hash(&hash).is_some()
                && matches!(self.archive.favorite_sticker(&hash), Ok(None))
            {
                let when = std::fs::metadata(&path)
                    .and_then(|metadata| metadata.modified())
                    .ok()
                    .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                    .map_or_else(now_millis, |age| age.as_millis() as i64);
                let _ = self
                    .archive
                    .set_favorite_sticker(&hash, true, when, None, false);
            }
        }
        let waiting = match self.archive.unpushed_favorite_stickers() {
            Ok(waiting) => waiting,
            Err(error) => {
                log::warn!("could not list favorite stickers to sync: {error}");
                return;
            }
        };
        if waiting.is_empty() {
            return;
        }
        let pushes: Vec<FavoritePush> = waiting
            .into_iter()
            .map(|(hash, state)| {
                let action = state
                    .action
                    .and_then(|raw| {
                        wa::sync_action_value::StickerAction::decode_from_slice(&raw).ok()
                    })
                    .or_else(|| self.sticker_references(&hash));
                FavoritePush {
                    file: dir.join(format!("{hash}.webp")),
                    hash,
                    favorite: state.favorite,
                    updated_at: state.updated_at,
                    action,
                }
            })
            .collect();
        self.favorites_pushing = true;
        let commands = self.commands.clone();
        tokio::spawn(async move {
            for push in pushes {
                let (hash, updated_at) = (push.hash.clone(), push.updated_at);
                let result = push_favorite(&client, push).await;
                let _ = commands.send(Command::FavoritePushed {
                    hash,
                    updated_at,
                    result,
                });
            }
            let _ = commands.send(Command::FavoritesPushed);
        });
    }

    /// CDN references for a sticker we have seen in a chat or on the phone's
    /// recent list, so a favorite need not be uploaded again.
    fn sticker_references(&self, hash: &str) -> Option<wa::sync_action_value::StickerAction> {
        if let Ok(phone) = self.archive.phone_stickers()
            && let Some(sticker) = phone.into_iter().find(|sticker| sticker.hash == hash)
            && let Ok(meta) = wa::StickerMetadata::decode_from_slice(&sticker.raw)
            && meta.direct_path.is_some()
        {
            return Some(action_of_metadata(&meta));
        }
        self.archive
            .sticker_message_raws(REFERENCE_SEARCH)
            .ok()?
            .into_iter()
            .filter_map(|raw| wa::Message::decode_from_slice(&raw).ok())
            .find_map(|message| {
                let sticker = message.get_base_message().sticker_message.as_option()?;
                let matches =
                    sticker_hash(sticker.file_sha256.as_deref(), None).as_deref() == Some(hash);
                (matches && sticker.direct_path.is_some()).then(|| action_of_message(sticker))
            })
    }

    /// Records the phone's copy of a favorite change.
    pub(super) fn favorite_pushed(
        &mut self,
        hash: &str,
        updated_at: i64,
        result: Result<Vec<u8>, String>,
    ) {
        match result {
            Ok(action) => {
                let _ = self
                    .archive
                    .favorite_sticker_pushed(hash, updated_at, Some(&action));
            }
            // Left unpushed, so the next connection tries again.
            Err(error) => log::warn!("could not sync a favorite sticker: {error}"),
        }
    }

    /// The push task finished; start another if changes arrived meanwhile.
    pub(super) fn favorites_pushed(&mut self) {
        self.favorites_pushing = false;
        if std::mem::take(&mut self.favorites_again) {
            self.push_favorites();
        }
    }

    /// Applies a favorite added or removed on the phone. The phone's change
    /// comes later in the sync order than anything it already had from us, so
    /// it wins unless a change made here is still on its way to the phone and
    /// is newer. A new favorite is copied from a local copy of the same sticker
    /// or fetched into the favorites folder, and fetched again on the next
    /// connection until its file arrives.
    pub(super) fn favorite_sticker_update(&mut self, update: &wa_events::FavoriteStickerUpdate) {
        let source = if update.from_full_sync {
            "phone (full sync)"
        } else {
            "phone"
        };
        let Some(hash) = hash_of_filehash(&update.filehash) else {
            log::warn!(
                "ignored a favorite sticker change from the {source}: unreadable file hash ({} characters)",
                update.filehash.len()
            );
            return;
        };
        let Some(favorite) = update.action.is_favorite else {
            log::warn!("ignored a favorite sticker change from the {source}: no favorite flag");
            return;
        };
        // Syncd timestamps are milliseconds; a missing one reads as the epoch.
        let stamped = update.timestamp.timestamp_millis();
        if let Ok(Some(known)) = self.archive.favorite_sticker(&hash)
            && !known.pushed
            && known.updated_at > stamped
        {
            log::info!("kept a newer favorite sticker change made here over one from the {source}");
            return;
        }
        let at = if stamped > 0 { stamped } else { now_millis() };
        // Removing a favorite carries no references; keep the ones we know.
        let action = fetchable(&update.action).then(|| update.action.encode_to_vec());
        if let Err(error) =
            self.archive
                .set_favorite_sticker(&hash, favorite, at, action.as_deref(), true)
        {
            log::warn!("could not record a favorite sticker: {error}");
        }
        log::info!(
            "favorite sticker {} on the {source}",
            if favorite { "added" } else { "removed" }
        );
        if !favorite {
            let path = self.dirs.saved_sticker_dir().join(format!("{hash}.webp"));
            if std::fs::remove_file(&path).is_ok() {
                self.emit_stickers();
            }
            return;
        }
        self.fetch_favorite(hash);
    }

    /// Brings a favorite's file into the favorites folder: from a copy of the
    /// same sticker already on this computer, otherwise from the CDN.
    fn fetch_favorite(&mut self, hash: String) {
        let dir = self.dirs.saved_sticker_dir();
        let path = dir.join(format!("{hash}.webp"));
        if path.exists() || self.favorite_fetches.contains(&hash) {
            return;
        }
        if let Some(source) = self.local_sticker_copy(&hash) {
            match super::super::sticker_store::save(&dir, &source) {
                Ok(_) => {
                    log::info!("favorite sticker copied from a local copy");
                    self.emit_stickers();
                    return;
                }
                Err(error) => log::warn!("could not copy a favorite sticker: {error}"),
            }
        }
        let Some(file_sha256) = digest_of_hash(&hash) else {
            return;
        };
        let stored = self
            .archive
            .favorite_sticker(&hash)
            .ok()
            .flatten()
            .and_then(|known| known.action)
            .and_then(|raw| wa::sync_action_value::StickerAction::decode_from_slice(&raw).ok());
        let candidates: Vec<FavoriteDownload> = stored
            .into_iter()
            .chain(self.sticker_references(&hash))
            .filter(fetchable)
            .map(|action| FavoriteDownload {
                action,
                file_sha256: file_sha256.clone(),
            })
            .collect();
        if candidates.is_empty() {
            log::warn!(
                "could not fetch a favorite sticker: no download references and no local copy"
            );
            return;
        }
        let Some(client) = self.client.clone() else {
            log::info!("a favorite sticker will be fetched once connected");
            return;
        };
        self.favorite_fetches.insert(hash.clone());
        let commands = self.commands.clone();
        tokio::spawn(async move {
            let mut result = Err("no download references".to_owned());
            for download in &candidates {
                result = download_attachment(&client, download, &dir, &path).await;
                if result.is_ok() {
                    break;
                }
            }
            let _ = commands.send(Command::FavoriteFetched { hash, result });
        });
    }

    /// A file on this computer holding the sticker with this content hash: in
    /// the phone's recent stickers, stickers from chats, or a pack.
    fn local_sticker_copy(&self, hash: &str) -> Option<PathBuf> {
        let mut candidates: Vec<PathBuf> = Vec::new();
        if let Ok(phone) = self.archive.phone_stickers() {
            candidates.extend(
                phone
                    .into_iter()
                    .filter(|sticker| sticker.hash == hash)
                    .filter_map(|sticker| sticker.path),
            );
        }
        if let Ok(rows) = self.archive.recent_stickers(REFERENCE_SEARCH) {
            candidates.extend(rows.into_iter().filter_map(|sticker| {
                let raw = sticker.raw.as_deref()?;
                let message = wa::Message::decode_from_slice(raw).ok()?;
                let found = message.get_base_message().sticker_message.as_option()?;
                (sticker_hash(found.file_sha256.as_deref(), None).as_deref() == Some(hash))
                    .then_some(sticker.path)
            }));
        }
        candidates.extend(
            self.sticker_packs()
                .into_iter()
                .flat_map(|pack| pack.stickers)
                .filter(|path| {
                    path.file_stem()
                        .is_some_and(|stem| stem.to_string_lossy() == hash)
                }),
        );
        candidates.into_iter().find(|path| {
            std::fs::read(path)
                .is_ok_and(|bytes| super::super::sticker_store::content_hash(&bytes) == hash)
        })
    }

    /// Fetches favorites the phone named whose files never arrived, such as
    /// one whose download failed or that came while offline.
    pub(super) fn fetch_missing_favorites(&mut self) {
        let dir = self.dirs.saved_sticker_dir();
        let missing: Vec<String> = match self.archive.favorite_stickers_from_phone() {
            Ok(hashes) => hashes
                .into_iter()
                .filter(|hash| !dir.join(format!("{hash}.webp")).exists())
                .collect(),
            Err(error) => {
                log::warn!("could not list favorite stickers to fetch: {error}");
                return;
            }
        };
        if !missing.is_empty() {
            log::info!(
                "fetching {} favorite stickers from the phone",
                missing.len()
            );
        }
        for hash in missing {
            self.fetch_favorite(hash);
        }
    }

    /// Keeps a fetched favorite only when it is the sticker the phone named.
    pub(super) fn favorite_fetched(&mut self, hash: &str, result: Result<PathBuf, String>) {
        self.favorite_fetches.remove(hash);
        match result {
            Ok(path) => {
                let matches = std::fs::read(&path)
                    .is_ok_and(|bytes| super::super::sticker_store::content_hash(&bytes) == hash);
                if matches {
                    log::info!("favorite sticker fetched from the phone");
                } else {
                    log::warn!("a favorite sticker did not match its hash");
                    let _ = std::fs::remove_file(&path);
                }
                self.emit_stickers();
            }
            // The phone's record stays, so the next connection tries again.
            Err(error) => log::warn!(
                "could not fetch a favorite sticker; retrying on the next connection: {}",
                redacted(&error)
            ),
        }
    }

    /// Favorites saved on the phone before ZapFast followed them were already
    /// consumed by earlier syncs, and incremental syncs never repeat them.
    /// Rebuilds the collection holding them once, so they replay.
    pub(super) fn recover_favorites(&mut self) {
        if self.favorites_recovered
            || self.favorites_recovering
            || !matches!(self.status, LinkStatus::Connected)
        {
            return;
        }
        let Some(client) = self.client.clone() else {
            return;
        };
        self.favorites_recovering = true;
        log::info!("reading favorite stickers from the phone");
        let commands = self.commands.clone();
        tokio::spawn(async move {
            use whatsapp_rust::WAPatchName;
            let complete = match client
                .resync_app_state(
                    [WAPatchName::RegularLow],
                    whatsapp_rust::AppStateResyncMode::Snapshot,
                )
                .await
            {
                Ok(report) => report.synced.contains(&WAPatchName::RegularLow),
                Err(error) => {
                    log::warn!("could not read favorite stickers from the phone: {error}");
                    false
                }
            };
            let _ = commands.send(Command::FavoritesRecovered { complete });
        });
    }

    /// The one-time favorites replay finished, or waits for the next
    /// connection.
    pub(super) fn favorites_recovered(&mut self, complete: bool) {
        self.favorites_recovering = false;
        if !complete {
            log::warn!(
                "favorite stickers from the phone not read yet; retrying on the next connection"
            );
            return;
        }
        self.favorites_recovered = true;
        if let Err(error) = self.archive.set_meta(FAVORITES_RECOVERED, "complete") {
            log::warn!("could not record the favorite sticker sync: {error}");
        }
    }

    /// Downloads a pack shared in a chat into the cache and shows it. A pack
    /// opened before shows again without downloading.
    pub(super) fn view_sticker_pack(&mut self, chat: &str, message: &str) {
        let pack = self
            .archive
            .raw(chat, message)
            .ok()
            .flatten()
            .and_then(|raw| wa::Message::decode_from_slice(&raw).ok())
            .and_then(|message| {
                message
                    .get_base_message()
                    .sticker_pack_message
                    .as_option()
                    .cloned()
            });
        let Some(pack) = pack else {
            self.emit(Event::StickerPackPreview(Err(
                "This sticker pack is no longer available".to_owned(),
            )));
            return;
        };
        let name = pack.name.clone().unwrap_or_default();
        let publisher = pack.publisher.clone().unwrap_or_default();
        let id = pack
            .sticker_pack_id
            .clone()
            .filter(|id| !id.is_empty())
            .unwrap_or_else(|| message.to_owned());
        let dir = self
            .dirs
            .sticker_cache_dir()
            .join("shared")
            .join(sanitize(&id));
        let cached = dir
            .parent()
            .map(super::super::sticker_store::packs)
            .unwrap_or_default()
            .into_iter()
            .find(|listed| listed.dir == dir);
        if let Some(mut cached) = cached {
            cached.name = name;
            self.emit(Event::StickerPackPreview(Ok((cached, publisher))));
            return;
        }
        let Some(client) = self.client.clone() else {
            self.emit(Event::StickerPackPreview(Err(
                "Connect to WhatsApp to open this sticker pack".to_owned(),
            )));
            return;
        };
        if attachment_is_too_large(pack.file_length) {
            self.emit(Event::StickerPackPreview(Err(
                ATTACHMENT_LIMIT_ERROR.to_owned()
            )));
            return;
        }
        let commands = self.commands.clone();
        tokio::spawn(async move {
            let result = async {
                let zip = client
                    .download(&pack)
                    .await
                    .map_err(|error| format!("Could not download the sticker pack: {error}"))?;
                let stickers: Vec<(String, Vec<String>)> = pack
                    .stickers
                    .iter()
                    .filter_map(|sticker| {
                        Some((sticker.file_name.clone()?, sticker.emojis.clone()))
                    })
                    .collect();
                let tray = pack.tray_icon_file_name.clone();
                let target = dir.clone();
                let label = name.clone();
                let stickers = tokio::task::spawn_blocking(move || {
                    super::super::sticker_import::extract_whatsapp_pack(
                        &zip,
                        &stickers,
                        tray.as_deref(),
                        &label,
                        &target,
                    )
                })
                .await
                .map_err(|error| error.to_string())??;
                Ok((
                    crate::model::StickerPack {
                        name,
                        dir,
                        stickers,
                        local: false,
                    },
                    publisher,
                ))
            }
            .await;
            let _ = commands.send(Command::StickerPackViewed { result });
        });
    }

    /// Copies a viewed pack into the packs here, under its own name.
    pub(super) fn add_sticker_pack(&mut self, dir: &Path, name: &str) {
        let root = self.dirs.sticker_cache_dir();
        if !dir.starts_with(&root) {
            return;
        }
        match super::super::sticker_import::copy_pack(dir, &self.packs_dir(), name) {
            Ok(name) => {
                self.emit_stickers();
                self.emit(Event::Info(format!("Added sticker pack \"{name}\"")));
            }
            Err(error) => self.emit(Event::Error(format!("Could not add sticker pack: {error}"))),
        }
    }

    /// Sends one of our packs to a chat as a WhatsApp sticker pack.
    pub(super) fn send_sticker_pack(&mut self, chat: ChatId, dir: PathBuf) {
        let Some(client) = self.client.clone() else {
            self.emit(Event::Error("Not connected to WhatsApp".to_owned()));
            return;
        };
        let Some(pack) = self
            .sticker_packs()
            .into_iter()
            .find(|pack| pack.dir == dir)
        else {
            return;
        };
        let publisher = self.me_name.clone().unwrap_or_default();
        let commands = self.commands.clone();
        let media = self.dirs.media_cache_dir();
        let me = self.me();
        tokio::spawn(async move {
            let outcome = async {
                let prepared =
                    prepare_sticker_pack(&client, pack.name, publisher, pack.stickers).await?;
                file_outbound(&client, &chat, &me, &media, prepared, None, Vec::new()).await
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
                        error: Some(format!("Could not send the sticker pack: {error}")),
                    });
                }
            }
        });
    }

    /// Applies a sticker the phone took out of Recent. Without a time the
    /// phone drops it unconditionally, so every earlier use goes.
    pub(super) fn recent_sticker_removed(&mut self, update: &wa_events::RemoveRecentStickerUpdate) {
        let Some(hash) = hash_of_filehash(&update.filehash) else {
            return;
        };
        let at = update
            .action
            .last_sticker_sent_ts
            .map(seconds)
            .unwrap_or_else(|| update.timestamp.timestamp());
        if let Err(error) = self.archive.remove_recent_sticker(&hash, at) {
            log::warn!("could not remove a recent sticker: {error}");
        }
        self.emit_stickers();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shared_sticker_pack_reads_as_a_pack_in_the_chat() {
        let message = wa::Message {
            sticker_pack_message: MessageField::some(wa::message::StickerPackMessage {
                name: Some("Ducks".into()),
                publisher: Some("Ada".into()),
                caption: Some("  ".into()),
                stickers: vec![Default::default(); 3],
                ..Default::default()
            }),
            ..Default::default()
        };
        assert_eq!(
            classify(&message),
            Some(Content::StickerPack {
                name: "Ducks".into(),
                publisher: "Ada".into(),
                count: 3,
                caption: None,
            })
        );
    }

    #[test]
    fn filehashes_and_content_hashes_name_the_same_sticker() {
        let hash = crate::backend::sticker_store::content_hash(b"sticker");
        let filehash = filehash_of_hash(&hash).expect("encodes");
        assert_eq!(filehash.len(), 44, "base64 of 32 bytes");
        assert_eq!(hash_of_filehash(&filehash).as_deref(), Some(hash.as_str()));
        assert!(hash_of_filehash("not base64!").is_none());
        assert!(hash_of_filehash("c2hvcnQ=").is_none(), "too short");
        assert!(filehash_of_hash("abc").is_none());
        // The same digest written without padding or URL-safe still names it.
        use base64::Engine;
        let digest = digest_of_hash(&hash).expect("digest");
        for engine in [
            base64::engine::general_purpose::STANDARD_NO_PAD,
            base64::engine::general_purpose::URL_SAFE,
            base64::engine::general_purpose::URL_SAFE_NO_PAD,
        ] {
            assert_eq!(
                hash_of_filehash(&engine.encode(&digest)).as_deref(),
                Some(hash.as_str())
            );
        }
    }

    /// Bytes standing in for a WebP sticker file, distinct per test.
    fn sticker_bytes(tag: &str) -> Vec<u8> {
        format!("RIFF....WEBPVP8L sticker {tag}").into_bytes()
    }

    /// A `favoriteSticker` change as the phone syncs it, in upstream's test
    /// format: indexed by the standard base64 SHA-256 of the sticker file,
    /// carrying the CDN references a companion fetches it by, stamped in
    /// milliseconds (`0` when the phone sent no timestamp, which upstream
    /// turns into the epoch).
    fn phone_favorite(
        bytes: &[u8],
        favorite: bool,
        at_ms: i64,
    ) -> wa_events::FavoriteStickerUpdate {
        use base64::Engine;
        use sha2::{Digest, Sha256};
        let filehash = base64::engine::general_purpose::STANDARD.encode(Sha256::digest(bytes));
        let action = wa::sync_action_value::StickerAction {
            url: Some("https://mmg.whatsapp.net/v/t62.15575-24/sticker.enc?oh=x&oe=y".into()),
            file_enc_sha256: Some(vec![9; 32]),
            media_key: Some(vec![7; 32]),
            mimetype: Some("image/webp".into()),
            height: Some(512),
            width: Some(512),
            direct_path: Some("/v/t62.15575-24/sticker.enc?oh=x&oe=y".into()),
            file_length: Some(bytes.len() as u64),
            is_favorite: Some(favorite),
            device_id_hint: Some(0),
            ..Default::default()
        };
        wa_events::FavoriteStickerUpdate::builder()
            .filehash(filehash)
            .timestamp(whatsapp_rust::wacore::time::from_millis_or_now(at_ms))
            .action(Box::new(action))
            .from_full_sync(false)
            .build()
    }

    fn sticker_worker() -> (
        Worker,
        tempfile::TempDir,
        std::sync::mpsc::Receiver<Event>,
        mpsc::UnboundedReceiver<Command>,
    ) {
        let (mut worker, events, commands, _wa) = super::super::receipt_tests::worker();
        let root = tempfile::tempdir().expect("temp");
        worker.dirs = AppDirs::under(root.path());
        (worker, root, events, commands)
    }

    fn favorites_listed(events: &std::sync::mpsc::Receiver<Event>) -> Option<Vec<PathBuf>> {
        events
            .try_iter()
            .filter_map(|event| match event {
                Event::Stickers { favorites, .. } => Some(favorites),
                _ => None,
            })
            .last()
    }

    #[tokio::test]
    async fn the_recent_shelf_updates_once_its_batch_of_fetches_is_in() {
        let (mut worker, _root, events, _commands) = sticker_worker();
        worker.sticker_fetches = ["first", "second"].map(str::to_owned).into();
        let listed = |events: &std::sync::mpsc::Receiver<Event>| {
            events
                .try_iter()
                .filter(|event| matches!(event, Event::Stickers { .. }))
                .count()
        };
        worker
            .handle_command(Command::StickerFetched {
                hash: "first".into(),
                result: Err("offline".into()),
            })
            .await;
        assert_eq!(
            listed(&events),
            0,
            "the open grid is not reshuffled mid-batch"
        );
        worker
            .handle_command(Command::StickerFetched {
                hash: "second".into(),
                result: Err("offline".into()),
            })
            .await;
        assert_eq!(listed(&events), 1);
    }

    /// Carmine's test: a sticker favorited on the phone never showed up. The
    /// phone favorites a sticker from its own tray, which ZapFast already has
    /// as one of the phone's recent stickers.
    #[test]
    fn a_sticker_favorited_on_the_phone_appears_in_favorites() {
        let (mut worker, _root, events, _commands) = sticker_worker();
        let bytes = sticker_bytes("from the phone's tray");
        let hash = crate::backend::sticker_store::content_hash(&bytes);
        let cache = worker.dirs.sticker_cache_dir();
        std::fs::create_dir_all(&cache).expect("cache");
        let cached = cache.join("recent.webp");
        std::fs::write(&cached, &bytes).expect("writes");
        worker
            .archive
            .upsert_phone_sticker(&hash, b"meta", 100, 1.0)
            .expect("stores");
        worker
            .archive
            .set_sticker_path(&hash, &cached)
            .expect("stores");

        worker.favorite_sticker_update(&phone_favorite(&bytes, true, now_millis()));

        let saved = worker.dirs.saved_sticker_dir().join(format!("{hash}.webp"));
        assert_eq!(
            std::fs::read(&saved).ok(),
            Some(bytes),
            "saved as a favorite"
        );
        assert_eq!(favorites_listed(&events), Some(vec![saved]));
        let known = worker
            .archive
            .favorite_sticker(&hash)
            .expect("reads")
            .expect("row");
        assert!(known.favorite && known.pushed, "the phone already has it");
    }

    /// A favorite the phone names but that cannot be fetched now stays owed:
    /// the next connection fetches it instead of forgetting it.
    #[test]
    fn a_favorite_that_could_not_be_fetched_is_fetched_again() {
        let (mut worker, _root, _events, _commands) = sticker_worker();
        let bytes = sticker_bytes("not on this computer yet");
        let hash = crate::backend::sticker_store::content_hash(&bytes);
        worker.favorite_sticker_update(&phone_favorite(&bytes, true, now_millis()));
        worker.favorite_fetched(&hash, Err("HTTP 410 https://mmg.example/v/x".to_owned()));
        assert_eq!(
            worker
                .archive
                .favorite_stickers_from_phone()
                .expect("lists"),
            vec![hash.clone()]
        );
        let stored = worker
            .archive
            .favorite_sticker(&hash)
            .expect("reads")
            .and_then(|known| known.action)
            .expect("the phone's references are kept for the retry");
        let action =
            wa::sync_action_value::StickerAction::decode_from_slice(&stored).expect("decodes");
        assert!(fetchable(&action));

        // By the next connection the sticker arrived in a pack.
        let pack = worker.packs_dir().join("Ducks");
        std::fs::create_dir_all(&pack).expect("pack");
        std::fs::write(pack.join(format!("{hash}.webp")), &bytes).expect("writes");
        worker.fetch_missing_favorites();
        let saved = worker.dirs.saved_sticker_dir().join(format!("{hash}.webp"));
        assert_eq!(std::fs::read(saved).ok(), Some(bytes));
    }

    /// The phone's clock need not agree with this computer's. A change the
    /// phone makes after it received ours comes later in the sync, so it
    /// wins even when its timestamp reads earlier, or is missing.
    #[test]
    fn a_phone_change_wins_over_one_the_phone_already_has() {
        for phone_at in [now_millis() - 60_000, 0] {
            let (mut worker, root, _events, _commands) = sticker_worker();
            let bytes = sticker_bytes("favorited here first");
            let file = root.path().join("picked.webp");
            std::fs::write(&file, &bytes).expect("writes");
            worker.favorite_sticker(&file);
            let hash = crate::backend::sticker_store::content_hash(&bytes);
            let saved = worker.dirs.saved_sticker_dir().join(format!("{hash}.webp"));
            assert!(saved.exists());
            let here = worker
                .archive
                .favorite_sticker(&hash)
                .expect("reads")
                .expect("row");
            worker
                .archive
                .favorite_sticker_pushed(&hash, here.updated_at, Some(b"refs"))
                .expect("pushed");

            worker.favorite_sticker_update(&phone_favorite(&bytes, false, phone_at));

            assert!(
                !saved.exists(),
                "removed on the phone (timestamp {phone_at})"
            );
            let known = worker
                .archive
                .favorite_sticker(&hash)
                .expect("reads")
                .expect("row");
            assert!(!known.favorite && known.pushed);
            assert!(known.updated_at > 0, "a missing timestamp is stored as now");
        }
    }

    /// A change made here that the phone has not received yet is newer than
    /// the phone's, so it is kept and still sent.
    #[test]
    fn a_newer_change_on_its_way_to_the_phone_is_kept() {
        let (mut worker, root, _events, _commands) = sticker_worker();
        let bytes = sticker_bytes("favorited here while offline");
        let file = root.path().join("picked.webp");
        std::fs::write(&file, &bytes).expect("writes");
        worker.favorite_sticker(&file);
        let hash = crate::backend::sticker_store::content_hash(&bytes);
        worker.favorite_sticker_update(&phone_favorite(&bytes, false, now_millis() - 60_000));
        let saved = worker.dirs.saved_sticker_dir().join(format!("{hash}.webp"));
        assert!(saved.exists());
        let known = worker
            .archive
            .favorite_sticker(&hash)
            .expect("reads")
            .expect("row");
        assert!(known.favorite && !known.pushed, "still to be sent");
    }

    /// The filehash is the plaintext SHA-256, which a sticker the phone holds
    /// no media key for needs to be fetched and checked by.
    #[test]
    fn a_favorite_download_carries_the_file_hash() {
        let bytes = sticker_bytes("plain");
        let update = phone_favorite(&bytes, true, 1);
        let hash = hash_of_filehash(&update.filehash).expect("hash");
        let mut action = (*update.action).clone();
        action.media_key = None;
        action.direct_path = None;
        action.url = Some("https://static.whatsapp.net/sticker?id=1".into());
        assert!(fetchable(&action));
        let download = FavoriteDownload {
            action,
            file_sha256: digest_of_hash(&hash).expect("digest"),
        };
        use sha2::{Digest, Sha256};
        assert_eq!(download.file_sha256(), Some(&Sha256::digest(&bytes)[..]));
        assert_eq!(
            download.static_url(),
            Some("https://static.whatsapp.net/sticker?id=1")
        );
        assert!(!download.is_encrypted());
        assert_eq!(
            redacted("HTTP 410 for https://mmg.whatsapp.net/v/t62/x?oh=1"),
            "HTTP 410 for <link>"
        );
    }
}
