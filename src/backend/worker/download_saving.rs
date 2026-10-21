//! Shared destination rules for manual saves and optional automatic copies.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};

use super::{Command, Content, Event, Worker, download_organization};
use crate::settings::{AutomaticSaving, DownloadNaming, DownloadPreferences};

pub(super) struct State {
    pub preferences: Option<DownloadPreferences>,
    automatic: AutomaticSaving,
    pub manual: HashSet<(String, String, Option<usize>)>,
    saved: HashSet<(String, String, Option<usize>)>,
    pool: Option<Arc<tokio::sync::Semaphore>>,
    generation: Arc<AtomicU64>,
    publication: Arc<Mutex<()>>,
    allowed: bool,
}

impl Drop for State {
    /// Revokes queued exports before this account releases its saving state.
    fn drop(&mut self) {
        self.invalidate();
    }
}

impl Default for State {
    /// Starts with automatic saving disabled and no remembered exports.
    fn default() -> Self {
        Self {
            preferences: None,
            automatic: AutomaticSaving::default(),
            manual: HashSet::new(),
            saved: HashSet::new(),
            pool: None,
            generation: Arc::new(AtomicU64::new(0)),
            publication: Arc::new(Mutex::new(())),
            allowed: true,
        }
    }
}

impl State {
    /// Revokes queued publication and forgets completed attachment deduplication.
    pub fn invalidate(&mut self) {
        let _publication = self
            .publication
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.saved.clear();
    }
    /// Revokes pending copies when the app lock changes export permission.
    pub fn set_allowed(&mut self, allowed: bool) {
        if self.allowed != allowed {
            self.invalidate();
        }
        self.allowed = allowed;
    }
    /// Accepts callbacks only for the current settings and allows failed copies to retry.
    pub fn completed(
        &mut self,
        generation: u64,
        attachment: &(String, String, Option<usize>),
        success: bool,
    ) -> bool {
        if self.generation.load(Ordering::SeqCst) != generation {
            return false;
        }
        if !success {
            self.saved.remove(attachment);
        }
        true
    }
    /// Applies saving preferences, revoking work started under older settings.
    pub fn configure(
        &mut self,
        preferences: Option<DownloadPreferences>,
        automatic: AutomaticSaving,
    ) {
        if self.preferences != preferences || self.automatic != automatic {
            self.invalidate();
        }
        self.preferences = preferences;
        self.automatic = automatic;
    }
}

impl Worker {
    /// Considers cached message and carousel attachments using the usual saving policy.
    pub(super) fn save_cached_attachments(&mut self, chat: &str, message: &crate::model::Message) {
        if let Some(path) = message
            .content
            .media()
            .and_then(|media| media.path.as_ref())
        {
            self.save_download_copy(chat, &message.id, None, path, false);
        }
        if let Content::Interactive {
            card: Some(card), ..
        } = &message.content
        {
            for (index, card) in card.carousel.iter().enumerate() {
                if let Some(path) = card.image.as_ref().and_then(|media| media.path.as_ref()) {
                    self.save_download_copy(chat, &message.id, Some(index), path, false);
                }
            }
        }
    }

    /// Builds a confined account-aware destination only for readable, retained messages.
    fn copy_destination(
        &self,
        root: &Path,
        chat: &str,
        id: &str,
        card: Option<usize>,
    ) -> Option<PathBuf> {
        let preferences = self.download_saving.preferences.as_ref()?;
        let mut message = self.archive.message(chat, id).ok().flatten()?;
        let conversation = self.archive.chat(chat).ok().flatten()?;
        // Exporting to a user folder must not expose a locked chat during privacy recovery.
        if !self.privacy_ready
            || !self.download_saving.allowed
            || conversation.locked
            || self.message_was_removed(chat, id)
        {
            return None;
        }
        let original_name = match &message.content {
            Content::Document { file_name, .. } => Some(file_name.clone()),
            _ => None,
        };
        let media = message.content.media_at_mut(card)?.clone();
        let cache_id = card.map_or_else(|| id.to_owned(), |index| format!("{id}-card-{index}"));
        let mut target = download_organization::path_with_original_names(
            root,
            preferences.organization,
            false,
            true,
            download_organization::Metadata {
                account: &self.dirs.id.0,
                chat,
                chat_name: &conversation.name,
                message: &cache_id,
                timestamp: message.timestamp,
                mime: &media.mime,
                original_name: original_name.as_deref(),
            },
        );
        if preferences.naming == DownloadNaming::DatePrefixed {
            let date = jiff::Timestamp::from_second(message.timestamp)
                .ok()
                .map(|time| {
                    time.to_zoned(jiff::tz::TimeZone::UTC)
                        .strftime("%Y-%m-%d_%H-%M-%S_UTC")
                        .to_string()
                })
                .unwrap_or_else(|| "Unknown-date".into());
            let name = target.file_name()?.to_string_lossy().into_owned();
            target.set_file_name(format!("{date}-{name}"));
        }
        Some(target)
    }

    /// Queues a non-overwriting copy when a manual request or automatic policy permits it.
    pub(super) fn save_download_copy(
        &mut self,
        chat: &str,
        id: &str,
        card: Option<usize>,
        source: &Path,
        manual: bool,
    ) {
        let Some(root) = self.download_folder.clone() else {
            return;
        };
        if !manual && self.download_saving.automatic.since.is_none() {
            return;
        }
        let Ok(Some(message)) = self.archive.message(chat, id) else {
            return;
        };
        // Picker stickers are assets, not user attachment exports. Automatic saving
        // copies incoming attachments only and never requests an extra download.
        if (!manual
            && self
                .sticker_downloads
                .contains(&(chat.to_owned(), id.to_owned())))
            || (!manual
                && (message.from_me || !self.download_saving.automatic.includes(message.timestamp)))
        {
            return;
        }
        let Some(target) = self.copy_destination(&root, chat, id, card) else {
            return;
        };
        let attachment = (chat.to_owned(), id.to_owned(), card);
        if !self.download_saving.saved.insert(attachment.clone()) {
            return;
        }
        let source = source.to_owned();
        let epoch = self.download_saving.generation.clone();
        let publication = self.download_saving.publication.clone();
        let generation = epoch.load(Ordering::SeqCst);
        let commands = self.commands.clone();
        let pool = self
            .download_saving
            .pool
            .get_or_insert_with(|| Arc::new(tokio::sync::Semaphore::new(2)))
            .clone();
        tokio::spawn(async move {
            let Ok(_permit) = pool.acquire_owned().await else {
                return;
            };
            let result = tokio::task::spawn_blocking(move || download_organization::copy_verified_guarded(&source, &root, &target, || epoch.load(Ordering::SeqCst) == generation, &publication))
                .await
                .map_err(|_| "Could not save the attachment copy".to_owned())
                .and_then(|result| result.map_err(|_| "Could not save the attachment copy. Check the download folder and available space.".to_owned()));
            let _ = commands.send(Command::AttachmentSaved {
                generation,
                attachment,
                result,
            });
        });
    }

    /// Suggests an organized destination while preserving the native save dialog choice.
    pub(super) fn save_attachment_as(
        &self,
        source: PathBuf,
        name: String,
        attachment: Option<(String, String, Option<usize>)>,
    ) {
        let root = self.download_folder.clone().or_else(|| {
            directories::UserDirs::new().and_then(|dirs| dirs.download_dir().map(Path::to_path_buf))
        });
        let suggestion = root.as_ref().and_then(|root| {
            attachment
                .as_ref()
                .and_then(|(chat, id, card)| self.copy_destination(root, chat, id, *card))
        });
        let events = self.events.clone();
        let waker = self.waker.clone();
        tokio::task::spawn_blocking(move || {
            let mut dialog = rfd::FileDialog::new()
                .set_title("Save attachment")
                .set_file_name(&name);
            if let Some(suggestion) = &suggestion {
                if let Some(folder) = suggestion.parent() {
                    if let Err(_error) = std::fs::create_dir_all(folder) {
                        let _ = events.send(Event::Error("Could not open the download folder. Choose another folder in Settings.".into()));
                        waker.wake();
                        return;
                    }
                    dialog = dialog.set_directory(folder);
                }
                if let Some(name) = suggestion.file_name() {
                    dialog = dialog.set_file_name(name.to_string_lossy());
                }
            } else if let Some(root) = &root {
                dialog = dialog.set_directory(root);
            }
            let Some(target) = dialog.save_file() else {
                return;
            };
            let result = if suggestion.is_some() {
                target
                    .parent()
                    .ok_or_else(|| std::io::Error::other("Invalid save folder"))
                    .and_then(|root| {
                        download_organization::copy_verified_when(&source, root, &target, || true)
                    })
            } else if target.exists()
                && std::fs::canonicalize(&source).ok() == std::fs::canonicalize(&target).ok()
            {
                Ok(target.clone())
            } else {
                std::fs::copy(&source, &target).map(|_| target.clone())
            };
            let event = match result {
                Ok(target) => Event::Info(format!(
                    "Saved {}",
                    target
                        .file_name()
                        .map_or_else(|| name.clone(), |name| name.to_string_lossy().into_owned())
                )),
                Err(_error) => Event::Error(
                    "Could not save the attachment. Check the folder and available space.".into(),
                ),
            };
            let _ = events.send(event);
            waker.wake();
        });
    }

    /// Suggests organized names and copies an album into the reader-selected folder.
    pub(super) fn save_album(&self, files: Vec<(PathBuf, String, String)>, chat: &str) {
        let root = self.download_folder.clone().or_else(|| {
            directories::UserDirs::new().and_then(|dirs| dirs.download_dir().map(Path::to_path_buf))
        });
        let suggestions: Vec<_> = files
            .iter()
            .map(|(_, _, id)| {
                root.as_ref()
                    .and_then(|root| self.copy_destination(root, chat, id, None))
            })
            .collect();
        let events = self.events.clone();
        let waker = self.waker.clone();
        tokio::task::spawn_blocking(move || {
            let mut dialog = rfd::FileDialog::new().set_title("Save album");
            if let Some(folder) = suggestions
                .first()
                .and_then(Option::as_ref)
                .and_then(|path| path.parent())
                .or(root.as_deref())
            {
                if std::fs::create_dir_all(folder).is_err() {
                    let _ = events.send(Event::Error(
                        "Could not open the download folder. Choose another folder in Settings."
                            .into(),
                    ));
                    waker.wake();
                    return;
                }
                dialog = dialog.set_directory(folder);
            }
            let Some(folder) = dialog.pick_folder() else {
                return;
            };
            let mut saved = 0;
            for ((source, name, _), suggestion) in files.into_iter().zip(suggestions) {
                let name = suggestion
                    .as_ref()
                    .and_then(|path| path.file_name())
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or(name);
                let result = if suggestion.is_some() {
                    download_organization::copy_verified_when(
                        &source,
                        &folder,
                        &folder.join(&name),
                        || true,
                    )
                } else {
                    super::save_album_copy(&source, &folder, &name)
                };
                if result.is_ok() {
                    saved += 1;
                } else {
                    let _ = events.send(Event::Error(format!("Saved {saved} album attachments; another attachment could not be saved. Check the folder and available space.")));
                    waker.wake();
                    return;
                }
            }
            let _ = events.send(Event::Info(format!("Saved {saved} album attachments")));
            waker.wake();
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Delivery, Media, Message};
    use crate::settings::DownloadOrganization;

    #[test]
    fn completed_revocation_cannot_be_followed_by_managed_publication() {
        use std::sync::mpsc;
        use std::time::Duration;

        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("cache.pdf");
        std::fs::write(&source, b"synthetic attachment").unwrap();
        let root = directory.path().join("exports");
        let target = root.join("invoice.pdf");
        let state = State::default();
        let epoch = state.generation.clone();
        let generation = epoch.load(Ordering::SeqCst);
        let publication = state.publication.clone();
        let (inside_tx, inside_rx) = mpsc::sync_channel(0);
        let (resume_tx, resume_rx) = mpsc::sync_channel(0);
        let copy_source = source.clone();
        let copy_target = target.clone();
        let copying = std::thread::spawn(move || {
            download_organization::copy_verified_guarded(
                &copy_source,
                &root,
                &copy_target,
                || {
                    let allowed = epoch.load(Ordering::SeqCst) == generation;
                    if allowed && publication.try_lock().is_err() {
                        // Pause after the final authorization read, while publication owns
                        // the gate. Revocation must wait until this publication finishes.
                        inside_tx.send(()).unwrap();
                        resume_rx.recv_timeout(Duration::from_secs(10)).unwrap();
                    }
                    allowed
                },
                &publication,
            )
        });
        inside_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        let (revoking_tx, revoking_rx) = mpsc::sync_channel(0);
        let (revoked_tx, revoked_rx) = mpsc::channel();
        let revoked_target = target.clone();
        let revoking = std::thread::spawn(move || {
            let mut state = state;
            revoking_tx.send(()).unwrap();
            state.set_allowed(false);
            revoked_tx.send(revoked_target.exists()).unwrap();
            state
        });
        revoking_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        assert!(
            matches!(revoked_rx.try_recv(), Err(mpsc::TryRecvError::Empty)),
            "revocation cannot finish while authorized publication is paused"
        );
        resume_tx.send(()).unwrap();
        assert!(
            revoked_rx.recv_timeout(Duration::from_secs(10)).unwrap(),
            "a winning publication is already visible when revocation finishes"
        );
        assert_eq!(
            copying.join().unwrap().unwrap(),
            std::fs::canonicalize(&target).unwrap()
        );
        let mut state = revoking.join().unwrap();
        assert!(!state.completed(generation, &("chat".into(), "message".into(), None), true));
        let revoked_generation = state.generation.clone();
        let gate = state.publication.clone();
        let late_target = target.with_file_name("late.pdf");
        let result = download_organization::copy_verified_guarded(
            &source,
            target.parent().unwrap(),
            &late_target,
            || revoked_generation.load(Ordering::SeqCst) == generation,
            &gate,
        );
        assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::Interrupted);
        assert!(
            !late_target.exists(),
            "completed revocation forbids later publication"
        );
        assert_eq!(std::fs::read(&target).unwrap(), b"synthetic attachment");
        assert_eq!(std::fs::read(source).unwrap(), b"synthetic attachment");
    }

    #[test]
    fn a_poisoned_publication_gate_does_not_prevent_revocation() {
        let mut state = State::default();
        let generation = state.generation.load(Ordering::SeqCst);
        let gate = state.publication.clone();
        assert!(
            std::thread::spawn(move || {
                let _guard = gate.lock().unwrap();
                panic!("synthetic interrupted publisher");
            })
            .join()
            .is_err()
        );
        state.set_allowed(false);
        assert_ne!(state.generation.load(Ordering::SeqCst), generation);
        assert!(!state.completed(generation, &("chat".into(), "message".into(), None), true));
        // Teardown also recovers the poisoned gate and invalidates without panicking.
        drop(state);
    }

    fn document(worker: &Worker, timestamp: i64) -> Message {
        let chat = "fixture@g.us";
        worker.archive.ensure_chat(chat, "Synthetic team").unwrap();
        Message {
            id: "same-message".into(),
            chat: chat.into(),
            sender: "fixture@s.whatsapp.net".into(),
            sender_name: None,
            from_me: false,
            timestamp,
            history_order: None,
            content: Content::Document {
                media: Media {
                    mime: "application/pdf".into(),
                    size: 20,
                    width: None,
                    height: None,
                    album: None,
                    path: None,
                    state: Default::default(),
                },
                file_name: "invoice.pdf".into(),
                caption: None,
                pages: None,
            },
            status: Delivery::Delivered,
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

    /// A history snapshot revokes queued exports only when it changes the effective lock.
    #[test]
    fn history_lock_changes_revoke_pending_attachment_copies() {
        let (mut worker, _, _, _) = super::super::receipt_tests::worker();
        let message = document(&worker, 99);
        let snapshot = |locked| {
            let mut history = super::super::chat_removal_tests::history(&message.chat, &[]);
            history.chats[0].locked = Some(locked);
            history
        };
        let epoch = worker.download_saving.generation.clone();
        worker.apply_history(snapshot(true), true);
        let revoked = epoch.load(Ordering::SeqCst);
        assert!(revoked > 0);
        assert!(worker.archive.chat(&message.chat).unwrap().unwrap().locked);
        worker.apply_history(snapshot(true), true);
        assert_eq!(epoch.load(Ordering::SeqCst), revoked);
        worker
            .archive
            .set_locked_at(&message.chat, false, 500)
            .unwrap();
        worker.apply_history(snapshot(true), true);
        assert_eq!(
            epoch.load(Ordering::SeqCst),
            revoked,
            "stale history cannot override app-state"
        );
        assert!(!worker.archive.chat(&message.chat).unwrap().unwrap().locked);
    }

    /// Quote navigation uses the same opt-in policy as opening an archive page.
    #[tokio::test]
    async fn quote_navigation_saves_older_cached_attachments_only_when_enabled() {
        let (mut worker, _, mut commands, _) = super::super::receipt_tests::worker();
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("cache.pdf");
        std::fs::write(&source, b"synthetic attachment").unwrap();
        worker.download_folder = Some(directory.path().join("exports"));
        worker.download_saving.configure(
            Some(DownloadPreferences::default()),
            AutomaticSaving {
                since: Some(100),
                include_older: false,
            },
        );
        let mut message = document(&worker, 99);
        if let Content::Document { media, .. } = &mut message.content {
            media.path = Some(source.clone());
        }
        worker.archive.insert_message(&message, None).unwrap();
        worker.load_until(
            message.chat.clone(),
            message.id.clone(),
            (200, "anchor".into()),
        );
        assert!(worker.download_saving.saved.is_empty());
        worker.download_saving.automatic.include_older = true;
        worker.load_until(
            message.chat.clone(),
            message.id.clone(),
            (200, "anchor".into()),
        );
        let finished = tokio::time::timeout(std::time::Duration::from_secs(10), commands.recv())
            .await
            .unwrap()
            .unwrap();
        let Command::AttachmentSaved { result, .. } = finished else {
            panic!("saved-copy callback")
        };
        assert_eq!(
            std::fs::read(result.unwrap()).unwrap(),
            b"synthetic attachment"
        );
    }

    /// A click during picker work keeps the explicit save through its non-picker retry.
    #[tokio::test]
    async fn a_manual_picker_retry_still_saves_with_automatic_saving_off() {
        let (mut worker, _, mut commands, _) = super::super::receipt_tests::worker();
        let directory = tempfile::tempdir().unwrap();
        let store = whatsapp_rust::store::SqliteStore::open(
            directory.path().join("session.db").to_str().unwrap(),
        )
        .await
        .unwrap();
        let bot = super::super::Bot::builder()
            .with_backend(store)
            .build()
            .await
            .unwrap();
        worker.client = Some(bot.client());
        let source = directory.path().join("cache.webp");
        std::fs::write(&source, b"synthetic sticker").unwrap();
        worker.download_folder = Some(directory.path().join("exports"));
        worker.download_saving.preferences = Some(DownloadPreferences::default());
        let mut message = document(&worker, 99);
        message.content = Content::Sticker {
            media: Media {
                mime: "image/webp".into(),
                size: 17,
                width: None,
                height: None,
                album: None,
                path: None,
                state: Default::default(),
            },
            animated: false,
        };
        let mut raw = super::super::wa::Message::default();
        raw.sticker_message =
            super::super::MessageField::some(super::super::wa::message::StickerMessage::default());
        use whatsapp_rust::waproto::buffa::Message as _;
        worker
            .archive
            .insert_message(&message, Some(&raw.encode_to_vec()))
            .unwrap();
        let key = (message.chat.clone(), message.id.clone());
        worker.sticker_downloads.insert(key.clone());
        worker
            .downloads
            .insert((key.0.clone(), key.1.clone(), None));
        worker.download_media(key.0.clone(), key.1.clone(), None, true);
        worker.downloaded(
            key.0.clone(),
            key.1.clone(),
            None,
            Err("fixture picker failure".into()),
        );
        assert!(
            worker
                .download_saving
                .manual
                .contains(&(key.0.clone(), key.1.clone(), None))
        );
        worker.downloaded(key.0, key.1, None, Ok(source));
        let saved = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                if let Some(Command::AttachmentSaved { result, .. }) = commands.recv().await {
                    break result;
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(std::fs::read(saved.unwrap()).unwrap(), b"synthetic sticker");
    }

    #[test]
    fn shared_destinations_keep_original_names_dates_and_account_identity() {
        let (mut worker, _, _, _) = super::super::receipt_tests::worker();
        worker.download_saving.configure(
            Some(DownloadPreferences::default()),
            AutomaticSaving::default(),
        );
        let mut message = document(&worker, 1_700_000_000);
        if let Content::Document { media, .. } = &mut message.content {
            media.mime = "application/pdf".into();
        }
        worker.archive.insert_message(&message, None).unwrap();
        let root = Path::new("exports");
        let first = worker
            .copy_destination(root, &message.chat, &message.id, None)
            .unwrap();
        assert_eq!(first.file_name().unwrap(), "invoice.pdf");
        worker.dirs.id = crate::model::AccountId("2".into());
        assert_ne!(
            first,
            worker
                .copy_destination(root, &message.chat, &message.id, None)
                .unwrap()
        );
        worker.download_saving.preferences.as_mut().unwrap().naming = DownloadNaming::DatePrefixed;
        let dated = worker
            .copy_destination(root, &message.chat, &message.id, None)
            .unwrap();
        assert_eq!(
            dated.file_name().unwrap(),
            "2023-11-14_22-13-20_UTC-invoice.pdf"
        );
        worker
            .download_saving
            .preferences
            .as_mut()
            .unwrap()
            .organization = DownloadOrganization::Flat;
        assert_eq!(
            worker
                .copy_destination(root, &message.chat, &message.id, None)
                .unwrap()
                .parent(),
            Some(root)
        );
    }

    #[test]
    fn recovery_locked_chats_and_app_lock_refuse_automatic_destinations() {
        let (mut worker, _, _, _) = super::super::receipt_tests::worker();
        worker.download_saving.preferences = Some(DownloadPreferences::default());
        let message = document(&worker, 100);
        worker.archive.insert_message(&message, None).unwrap();
        let destination = |worker: &Worker| {
            worker.copy_destination(Path::new("exports"), &message.chat, &message.id, None)
        };
        assert!(destination(&worker).is_some());
        worker.privacy_ready = false;
        assert!(destination(&worker).is_none());
        worker.privacy_ready = true;
        worker.archive.set_locked(&message.chat, true).unwrap();
        assert!(destination(&worker).is_none());
        worker.archive.set_locked(&message.chat, false).unwrap();
        worker.download_saving.set_allowed(false);
        assert!(destination(&worker).is_none());
    }

    /// Equal document names must not suppress another attachment or repeat an export.
    #[tokio::test]
    async fn distinct_attachments_with_the_same_name_are_each_saved_once() {
        let (mut worker, _, mut commands, _) = super::super::receipt_tests::worker();
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("exports");
        worker.download_folder = Some(root.clone());
        worker.download_saving.configure(
            Some(DownloadPreferences {
                organization: DownloadOrganization::Flat,
                ..Default::default()
            }),
            AutomaticSaving {
                since: Some(100),
                include_older: false,
            },
        );
        let mut attachments = Vec::new();
        for (id, bytes) in [
            ("first", b"first attachment"),
            ("other", b"other attachment"),
        ] {
            let mut message = document(&worker, 101);
            message.id = id.into();
            worker.archive.insert_message(&message, None).unwrap();
            let source = directory.path().join(id);
            std::fs::write(&source, bytes).unwrap();
            worker.save_download_copy(&message.chat, &message.id, None, &source, false);
            attachments.push((message, source));
        }
        let mut saved = Vec::new();
        for _ in 0..2 {
            let command = tokio::time::timeout(std::time::Duration::from_secs(10), commands.recv())
                .await
                .unwrap()
                .unwrap();
            let Command::AttachmentSaved {
                result: Ok(path), ..
            } = &command
            else {
                panic!("successful saved-copy callback")
            };
            saved.push(std::fs::read(path).unwrap());
            worker.handle_command(command).await;
        }
        saved.sort();
        assert_eq!(
            saved,
            vec![b"first attachment".to_vec(), b"other attachment".to_vec()]
        );
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 2);
        for (message, source) in attachments {
            worker.save_download_copy(&message.chat, &message.id, None, &source, false);
        }
        assert!(commands.try_recv().is_err());
        assert_eq!(worker.download_saving.saved.len(), 2);
    }

    #[tokio::test]
    async fn automatic_copy_failure_keeps_playable_media_and_can_retry() {
        let (mut worker, _, mut commands, _) = super::super::receipt_tests::worker();
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("cache.pdf");
        std::fs::write(&source, b"synthetic attachment").unwrap();
        let root = directory.path().join("exports");
        std::fs::write(&root, b"unavailable folder").unwrap();
        worker.download_folder = Some(root.clone());
        worker.download_saving.configure(
            Some(DownloadPreferences::default()),
            AutomaticSaving {
                since: Some(100),
                include_older: false,
            },
        );
        let mut message = document(&worker, 101);
        if let Content::Document { media, .. } = &mut message.content {
            media.path = Some(source.clone());
        }
        worker.archive.insert_message(&message, None).unwrap();
        worker.save_download_copy(&message.chat, &message.id, None, &source, false);
        let finished = tokio::time::timeout(std::time::Duration::from_secs(10), commands.recv())
            .await
            .unwrap()
            .unwrap();
        let Command::AttachmentSaved {
            generation,
            attachment,
            result,
        } = finished
        else {
            panic!("saved-copy callback")
        };
        assert!(result.is_err());
        worker
            .download_saving
            .completed(generation, &attachment, false);
        assert_eq!(
            worker
                .archive
                .message(&message.chat, &message.id)
                .unwrap()
                .unwrap()
                .content
                .media()
                .unwrap()
                .path,
            Some(source.clone())
        );
        std::fs::remove_file(&root).unwrap();
        worker.save_download_copy(&message.chat, &message.id, None, &source, false);
        let finished = tokio::time::timeout(std::time::Duration::from_secs(10), commands.recv())
            .await
            .unwrap()
            .unwrap();
        let Command::AttachmentSaved { result, .. } = finished else {
            panic!("saved-copy callback")
        };
        assert_eq!(
            std::fs::read(result.unwrap()).unwrap(),
            b"synthetic attachment"
        );
        assert!(source.is_file());
    }

    #[tokio::test]
    async fn picker_assets_stay_private_but_explicit_sticker_downloads_can_be_saved() {
        let (mut worker, _, mut commands, _) = super::super::receipt_tests::worker();
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("cache.webp");
        std::fs::write(&source, b"synthetic sticker bytes").unwrap();
        let root = directory.path().join("exports");
        worker.download_folder = Some(root.clone());
        worker.download_saving.configure(
            Some(DownloadPreferences::default()),
            AutomaticSaving {
                since: Some(100),
                include_older: false,
            },
        );
        let mut message = document(&worker, 101);
        message.content = Content::Sticker {
            media: Media {
                mime: "image/webp".into(),
                size: 22,
                width: None,
                height: None,
                album: None,
                path: Some(source.clone()),
                state: Default::default(),
            },
            animated: false,
        };
        worker.archive.insert_message(&message, None).unwrap();
        worker
            .sticker_downloads
            .insert((message.chat.clone(), message.id.clone()));
        worker.save_download_copy(&message.chat, &message.id, None, &source, false);
        assert!(!root.exists(), "picker work alone never exports files");
        worker.save_download_copy(&message.chat, &message.id, None, &source, true);
        let finished = tokio::time::timeout(std::time::Duration::from_secs(10), commands.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(
            finished,
            Command::AttachmentSaved { result: Ok(_), .. }
        ));
    }

    #[tokio::test]
    async fn automatic_saving_is_off_unless_enabled_and_respects_the_cutoff() {
        let (mut worker, _, mut commands, _) = super::super::receipt_tests::worker();
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("cache.pdf");
        std::fs::write(&source, b"synthetic attachment").unwrap();
        let root = directory.path().join("exports");
        worker.download_folder = Some(root.clone());
        worker.download_saving.preferences = Some(DownloadPreferences::default());
        let message = document(&worker, 99);
        worker.archive.insert_message(&message, None).unwrap();
        worker.save_download_copy(&message.chat, &message.id, None, &source, false);
        assert!(!root.exists());
        worker.download_saving.automatic.since = Some(100);
        worker.save_download_copy(&message.chat, &message.id, None, &source, false);
        assert!(!root.exists());
        worker.download_saving.automatic.include_older = true;
        worker.save_download_copy(&message.chat, &message.id, None, &source, false);
        let finished = tokio::time::timeout(std::time::Duration::from_secs(10), commands.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(
            finished,
            Command::AttachmentSaved { result: Ok(_), .. }
        ));
    }
}
