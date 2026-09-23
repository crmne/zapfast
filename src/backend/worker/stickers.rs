//! The sticker picker's lists and their sync with the phone.
//!
//! WhatsApp names a sticker in app-state sync by its `filehash`: the base64
//! SHA-256 of the decrypted file. ZapFast names it by the same digest in hex,
//! which is also the name of every saved or packed copy, so both sides agree
//! on which sticker a change is about.

use super::*;
use whatsapp_rust::schemas;

/// The hex content hash for a WhatsApp `filehash`.
pub(super) fn hash_of_filehash(filehash: &str) -> Option<String> {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(filehash.trim())
        .ok()?;
    (bytes.len() == 32).then(|| bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

/// The WhatsApp `filehash` for a hex content hash.
pub(super) fn filehash_of_hash(hash: &str) -> Option<String> {
    use base64::Engine;
    if hash.len() != 64 {
        return None;
    }
    let bytes = (0..32)
        .map(|index| u8::from_str_radix(hash.get(index * 2..index * 2 + 2)?, 16).ok())
        .collect::<Option<Vec<u8>>>()?;
    Some(base64::engine::general_purpose::STANDARD.encode(bytes))
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

/// A favorite sticker the phone told us about, fetched by its references.
struct FavoriteDownload(wa::sync_action_value::StickerAction);

impl Downloadable for FavoriteDownload {
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
        None
    }

    fn file_length(&self) -> Option<u64> {
        self.0.file_length
    }

    fn app_info(&self) -> MediaType {
        MediaType::Sticker
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

    /// Applies a favorite added or removed on the phone, unless a later
    /// change here wins. A new favorite is fetched into the favorites folder.
    pub(super) fn favorite_sticker_update(&mut self, update: &wa_events::FavoriteStickerUpdate) {
        let Some(hash) = hash_of_filehash(&update.filehash) else {
            return;
        };
        let Some(favorite) = update.action.is_favorite else {
            return;
        };
        let at = update.timestamp.timestamp_millis();
        if let Ok(Some(known)) = self.archive.favorite_sticker(&hash)
            && known.updated_at > at
        {
            return;
        }
        let action = update.action.encode_to_vec();
        if let Err(error) =
            self.archive
                .set_favorite_sticker(&hash, favorite, at, Some(&action), true)
        {
            log::warn!("could not record a favorite sticker: {error}");
        }
        let dir = self.dirs.saved_sticker_dir();
        let path = dir.join(format!("{hash}.webp"));
        if !favorite {
            if std::fs::remove_file(&path).is_ok() {
                self.emit_stickers();
            }
            return;
        }
        if path.exists() || !self.favorite_fetches.insert(hash.clone()) {
            return;
        }
        let Some(client) = self.client.clone() else {
            self.favorite_fetches.remove(&hash);
            return;
        };
        let download = FavoriteDownload((*update.action).clone());
        let commands = self.commands.clone();
        tokio::spawn(async move {
            let result = download_attachment(&client, &download, &dir, &path).await;
            let _ = commands.send(Command::FavoriteFetched { hash, result });
        });
    }

    /// Keeps a fetched favorite only when it is the sticker the phone named.
    pub(super) fn favorite_fetched(&mut self, hash: &str, result: Result<PathBuf, String>) {
        self.favorite_fetches.remove(hash);
        match result {
            Ok(path) => {
                let matches = std::fs::read(&path)
                    .is_ok_and(|bytes| super::super::sticker_store::content_hash(&bytes) == hash);
                if !matches {
                    log::warn!("a favorite sticker did not match its hash");
                    let _ = std::fs::remove_file(&path);
                }
                self.emit_stickers();
            }
            Err(_error) => log::warn!("could not fetch a favorite sticker"),
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
    }
}
