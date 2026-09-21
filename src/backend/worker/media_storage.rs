//! Changing attachment folders copies files before committing their archive paths.

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use super::{Event, Worker};
use crate::paths::AppDirs;

/// Only known cache files may be forgotten automatically. An absent external
/// file may belong to an unmounted drive, even if its mount point is readable.
pub(super) fn is_disposable_source(dirs: &AppDirs, path: &Path) -> bool {
    AppDirs::is_subpath(path, &dirs.media_cache_dir())
        && !dirs
            .custom_media
            .as_ref()
            .is_some_and(|custom| AppDirs::is_subpath(path, custom))
}

/// Removes newly published copies on failure, but never touches the source files.
#[derive(Default)]
struct Copies(Vec<PathBuf>);

impl Drop for Copies {
    fn drop(&mut self) {
        for path in &self.0 {
            let _ = std::fs::remove_file(path);
        }
    }
}

impl Copies {
    fn keep(mut self) {
        self.0.clear();
    }

    fn copy(&mut self, source: &Path, dir: &Path) -> std::io::Result<PathBuf> {
        let name = source.file_name().ok_or(std::io::ErrorKind::InvalidInput)?;
        // Validate even when the source is already in the destination folder.
        let mut input = std::fs::File::open(source)?;
        if !input.metadata()?.is_file() {
            return Err(std::io::ErrorKind::InvalidInput.into());
        }
        if source
            .parent()
            .and_then(|parent| parent.canonicalize().ok())
            == Some(dir.canonicalize()?)
        {
            return Ok(source.to_owned());
        }
        let mut staged = tempfile::NamedTempFile::new_in(dir)?;
        std::io::copy(&mut input, &mut staged)?;
        staged.as_file().sync_all()?;
        let path = publish(staged, &dir.join(name))?;
        self.0.push(path.clone());
        Ok(path)
    }
}

fn publish(mut staged: tempfile::NamedTempFile, preferred: &Path) -> std::io::Result<PathBuf> {
    let name = preferred
        .file_name()
        .ok_or(std::io::ErrorKind::InvalidInput)?;
    let mut target = preferred.to_owned();
    loop {
        match staged.persist_noclobber(&target) {
            Ok(_) => return Ok(target),
            Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
                staged = error.file;
                // Never overwrite a user's file or mistake it for this attachment.
                let mut unique =
                    std::ffi::OsString::from(format!("{:016x}-", rand::random::<u64>()));
                unique.push(name);
                target = preferred.with_file_name(unique);
            }
            Err(error) => return Err(error.error),
        }
    }
}

/// Save new downloads and uploaded attachments with the same no-clobber policy.
pub(super) async fn save(path: PathBuf, bytes: Vec<u8>) -> Result<PathBuf, String> {
    tokio::task::spawn_blocking(move || {
        let dir = path.parent().ok_or(std::io::ErrorKind::InvalidInput)?;
        std::fs::create_dir_all(dir)?;
        let mut staged = tempfile::NamedTempFile::new_in(dir)?;
        staged.write_all(&bytes)?;
        staged.as_file().sync_all()?;
        publish(staged, &path)
    })
    .await
    .map_err(|_| "Attachment save task failed".to_owned())?
    .map_err(|error| error.to_string())
}

impl Worker {
    pub(super) async fn change_media_dir(&mut self, custom: Option<PathBuf>) {
        if let Err(error) = self.try_change_media_dir(custom).await {
            self.emit(Event::Error(format!(
                "Could not change attachment folder: {error}"
            )));
        }
    }

    async fn try_change_media_dir(&mut self, custom: Option<PathBuf>) -> Result<(), String> {
        let rows = self
            .archive
            .media_paths()
            .map_err(|error| error.to_string())?;
        let dirs = self.dirs.clone();
        let (custom, copies, updates, paths) = tokio::task::spawn_blocking(move || {
            // Inspect the old folder before creating any destination. Selecting
            // the same unavailable folder must not recreate it and hide an outage.
            if !rows.is_empty()
                && let Some(source_dir) = &dirs.custom_media
            {
                std::fs::read_dir(source_dir).map_err(|error| {
                    std::io::Error::new(
                        error.kind(),
                        "Current attachment folder is unavailable. Reconnect it and retry",
                    )
                })?;
            }
            let custom = custom.filter(|path| !dirs.is_default_media_dir(path));
            let dir = custom.clone().unwrap_or_else(|| dirs.media_cache_dir());
            std::fs::create_dir_all(&dir)?;
            // Resolve symlinks and '..' before checking the cache boundary.
            let dir = dir.canonicalize()?;
            if custom.is_some() && dirs.is_cache_path(&dir) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "Custom attachment folder cannot be inside the cache directory",
                ));
            }
            if custom.is_some()
                && (AppDirs::is_subpath(&dir, &dirs.state)
                    || AppDirs::is_subpath(&dir, &dirs.config))
            {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "Custom attachment folder cannot be inside the app data folders",
                ));
            }
            let custom = custom.map(|_| dir.clone());
            // Even an empty archive must not accept an unwritable directory.
            let _probe = tempfile::NamedTempFile::new_in(&dir)?;
            let mut copies = Copies::default();
            let mut updates = Vec::new();
            let mut paths: HashMap<PathBuf, Option<PathBuf>> = HashMap::new();
            for (chat, id, source) in rows {
                let replacement = if let Some(path) = paths.get(&source) {
                    path.clone()
                } else {
                    let path = match source.try_exists()? {
                        true => Some(copies.copy(&source, &dir)?),
                        false if is_disposable_source(&dirs, &source) => None,
                        false => {
                            return Err(std::io::Error::new(
                                std::io::ErrorKind::NotFound,
                                "A saved attachment is unavailable. Restore its file or reconnect its storage and retry",
                            ));
                        }
                    };
                    paths.insert(source, path.clone());
                    path
                };
                updates.push((chat, id, replacement));
            }
            Ok::<_, std::io::Error>((custom, copies, updates, paths))
        })
        .await
        .map_err(|_| "Attachment copy task failed".to_owned())?
        .map_err(|error| error.to_string())?;

        // A database failure rolls back all rows and drops only the new copies.
        self.archive
            .set_media_paths(&updates)
            .map_err(|error| error.to_string())?;
        copies.keep();
        self.dirs.custom_media = custom.clone();
        self.emit(Event::MediaDirChanged { custom, paths });
        Ok(())
    }

    /// Reconcile downloads and uploads started before the last folder change.
    pub(super) fn keep_current_media(&self, source: &Path) -> Result<PathBuf, String> {
        let dir = self
            .dirs
            .ensure_media_dir()
            .map_err(|error| error.to_string())?;
        let mut copies = Copies::default();
        let path = copies
            .copy(source, &dir)
            .map_err(|error| error.to_string())?;
        copies.keep();
        Ok(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::Command;
    use crate::model::{Content, Media};
    use crate::paths::AppDirs;

    const CHAT: &str = "1@s.whatsapp.net";

    fn attachment(worker: &Worker, id: &str, path: &Path) {
        worker.archive.ensure_chat(CHAT, "Fixture").unwrap();
        let mut message = crate::archive::tests::message(CHAT, id, 1, false);
        message.content = Content::Image {
            caption: None,
            media: Media {
                mime: "image/jpeg".into(),
                size: 7,
                width: None,
                height: None,
                path: Some(path.to_owned()),
                state: Default::default(),
            },
        };
        worker.archive.insert_message(&message, None).unwrap();
    }

    fn archived_path(worker: &Worker, id: &str) -> Option<PathBuf> {
        worker
            .archive
            .message(CHAT, id)
            .unwrap()
            .unwrap()
            .content
            .media()
            .unwrap()
            .path
            .clone()
    }

    #[tokio::test]
    async fn folder_changes_copy_existing_files_update_rows_and_survive_unlink_cleanup() {
        let root = tempfile::tempdir().unwrap();
        let (mut worker, events, _commands, _wa) = super::super::receipt_tests::worker();
        worker.dirs = AppDirs::under(root.path());
        let old = worker.dirs.ensure_media_dir().unwrap().join("photo.jpg");
        std::fs::write(&old, b"fixture").unwrap();
        attachment(&worker, "image", &old);
        // Multiple forwarded rows can reference the same file.
        attachment(&worker, "forward", &old);
        let missing = old.with_file_name("missing.jpg");
        attachment(&worker, "missing", &missing);

        let custom = root.path().join("downloads");
        worker
            .handle_command(Command::SetMediaDir(custom.clone()))
            .await;
        let path = archived_path(&worker, "image").unwrap();
        assert_eq!(path.parent().unwrap(), custom.canonicalize().unwrap());
        assert_eq!(std::fs::read(&path).unwrap(), b"fixture");
        assert_eq!(archived_path(&worker, "forward"), Some(path.clone()));
        assert_eq!(archived_path(&worker, "missing"), None);
        assert!(
            matches!(events.try_recv().unwrap(), Event::MediaDirChanged { paths, .. }
            if paths.get(&old) == Some(&Some(path.clone())) && paths.get(&missing) == Some(&None))
        );
        worker.clean_media_cache();
        assert!(!old.exists());
        assert!(path.exists());

        worker.handle_command(Command::ResetMediaDir).await;
        assert_eq!(worker.dirs.custom_media, None);
        let reset = archived_path(&worker, "image").unwrap();
        assert_eq!(
            reset.parent().unwrap(),
            worker.dirs.media_cache_dir().canonicalize().unwrap()
        );
        assert_eq!(std::fs::read(reset).unwrap(), b"fixture");
        assert!(
            path.exists(),
            "reset preserves the user's custom folder files"
        );
    }

    #[tokio::test]
    async fn collisions_preserve_unrelated_files_and_report_the_actual_new_path() {
        let root = tempfile::tempdir().unwrap();
        let (mut worker, events, _commands, _wa) = super::super::receipt_tests::worker();
        worker.dirs = AppDirs::under(root.path());
        let old = worker.dirs.ensure_media_dir().unwrap().join("photo.jpg");
        std::fs::write(&old, b"fixture").unwrap();
        attachment(&worker, "image", &old);
        let custom = root.path().join("downloads");
        std::fs::create_dir_all(&custom).unwrap();
        let existing = custom.join("photo.jpg");
        std::fs::write(&existing, b"unrelated").unwrap();

        worker.change_media_dir(Some(custom)).await;
        let path = archived_path(&worker, "image").unwrap();
        assert_eq!(std::fs::read(&existing).unwrap(), b"unrelated");
        assert_eq!(std::fs::read(&path).unwrap(), b"fixture");
        assert_eq!(path.extension().unwrap(), "jpg");
        assert!(
            matches!(events.try_recv().unwrap(), Event::MediaDirChanged { paths, .. }
            if paths.get(&old) == Some(&Some(path.clone())))
        );
        assert!(old.exists());
    }

    #[tokio::test]
    async fn failed_copy_keeps_the_previous_folder_and_archive_paths() {
        let root = tempfile::tempdir().unwrap();
        let (mut worker, events, _commands, _wa) = super::super::receipt_tests::worker();
        worker.dirs = AppDirs::under(root.path());
        let old = worker.dirs.ensure_media_dir().unwrap().join("photo.jpg");
        std::fs::write(&old, b"fixture").unwrap();
        attachment(&worker, "image", &old);
        let invalid = old.with_file_name("directory.jpg");
        std::fs::create_dir_all(&invalid).unwrap();
        attachment(&worker, "invalid", &invalid);

        let custom = root.path().join("downloads");
        worker.change_media_dir(Some(custom.clone())).await;
        assert_eq!(worker.dirs.custom_media, None);
        assert_eq!(archived_path(&worker, "image"), Some(old.clone()));
        assert_eq!(archived_path(&worker, "invalid"), Some(invalid));
        assert_eq!(std::fs::read(&old).unwrap(), b"fixture");
        assert_eq!(std::fs::read_dir(custom).unwrap().count(), 0);
        assert!(matches!(events.try_recv().unwrap(), Event::Error(_)));
        assert!(events.try_recv().is_err());
    }

    #[tokio::test]
    async fn invalid_or_cache_folders_are_rejected_and_default_selection_resets() {
        let root = tempfile::tempdir().unwrap();
        let (mut worker, events, _commands, _wa) = super::super::receipt_tests::worker();
        worker.dirs = AppDirs::under(root.path());
        worker.dirs.ensure().unwrap();
        let blocked = root.path().join("file");
        std::fs::write(&blocked, b"fixture").unwrap();
        for path in [blocked, worker.dirs.cache.join("nested/../forbidden")] {
            worker.change_media_dir(Some(path)).await;
            assert!(matches!(events.try_recv().unwrap(), Event::Error(_)));
            assert_eq!(worker.dirs.custom_media, None);
        }
        worker
            .change_media_dir(Some(root.path().join("downloads")))
            .await;
        assert!(worker.dirs.custom_media.is_some());
        worker
            .change_media_dir(Some(worker.dirs.media_cache_dir()))
            .await;
        assert_eq!(worker.dirs.custom_media, None);
    }

    #[tokio::test]
    async fn a_download_started_in_the_old_folder_finishes_in_the_current_folder() {
        let root = tempfile::tempdir().unwrap();
        let (mut worker, events, _commands, _wa) = super::super::receipt_tests::worker();
        worker.dirs = AppDirs::under(root.path());
        let old = worker.dirs.ensure_media_dir().unwrap().join("late.jpg");
        attachment(&worker, "image", &old);
        worker
            .change_media_dir(Some(root.path().join("downloads")))
            .await;
        let _ = events.try_recv().unwrap();
        std::fs::write(&old, b"fixture").unwrap();
        worker
            .handle_command(Command::Downloaded {
                chat: CHAT.into(),
                id: "image".into(),
                result: Ok(old.clone()),
            })
            .await;
        let path = archived_path(&worker, "image").unwrap();
        assert_eq!(path.parent().unwrap(), worker.dirs.media_dir());
        assert!(
            matches!(events.try_recv().unwrap(), Event::Media { result: Ok(result), .. } if result == path)
        );
        worker.clean_media_cache();
        assert_eq!(std::fs::read(path).unwrap(), b"fixture");
    }

    #[test]
    fn failed_operation_removes_only_new_copies() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source.jpg");
        std::fs::write(&source, b"fixture").unwrap();
        let dir = root.path().join("target");
        std::fs::create_dir_all(&dir).unwrap();
        let mut copies = Copies::default();
        let target = copies.copy(&source, &dir).unwrap();
        assert!(target.exists());
        drop(copies);
        assert!(!target.exists());
        assert!(source.exists());
    }

    #[tokio::test]
    async fn new_downloads_do_not_overwrite_existing_files() {
        let root = tempfile::tempdir().unwrap();
        let preferred = root.path().join("downloads/photo.jpg");
        let first = save(preferred.clone(), b"first".to_vec()).await.unwrap();
        let second = save(preferred.clone(), b"second".to_vec()).await.unwrap();
        assert_eq!(first, preferred);
        assert_ne!(first, second);
        assert_eq!(std::fs::read(first).unwrap(), b"first");
        assert_eq!(std::fs::read(second).unwrap(), b"second");
    }

    #[test]
    fn legacy_custom_cache_subtrees_survive_cleanup() {
        let root = tempfile::tempdir().unwrap();
        let (mut worker, _events, _commands, _wa) = super::super::receipt_tests::worker();
        worker.dirs = AppDirs::under(root.path());
        let cache = worker.dirs.ensure_media_dir().unwrap();
        let custom = cache.join("legacy/custom");
        std::fs::create_dir_all(&custom).unwrap();
        let saved = custom.join("saved.jpg");
        std::fs::write(&saved, b"fixture").unwrap();
        let disposable = cache.join("cached.jpg");
        std::fs::write(&disposable, b"fixture").unwrap();
        worker.dirs.custom_media = Some(custom);
        worker.clean_media_cache();
        assert!(saved.exists());
        assert!(!disposable.exists());
        worker.dirs.custom_media = Some(cache);
        worker.clean_media_cache();
        assert!(saved.exists());
    }

    #[test]
    fn startup_retains_custom_paths_until_storage_returns() {
        // A disconnected mount may disappear, leave an empty directory, or fail
        // directory inspection. Replacing it with a file simulates that last case
        // without depending on platform ACLs or whether tests run as root.
        for unavailable in ["missing", "empty", "not-directory"] {
            let root = tempfile::tempdir().unwrap();
            let (mut worker, _events, _commands, _wa) = super::super::receipt_tests::worker();
            worker.dirs = AppDirs::under(root.path());
            let custom = root.path().join("mounted");
            std::fs::create_dir_all(&custom).unwrap();
            let source = custom.join("photo.jpg");
            std::fs::write(&source, b"preserved").unwrap();
            worker.dirs.custom_media = Some(custom.clone());
            attachment(&worker, "image", &source);
            let offline = root.path().join("offline");
            std::fs::rename(&custom, &offline).unwrap();
            match unavailable {
                "empty" => std::fs::create_dir(&custom).unwrap(),
                "not-directory" => std::fs::write(&custom, b"blocked").unwrap(),
                _ => {}
            }

            worker.relocate_media();
            assert_eq!(archived_path(&worker, "image"), Some(source.clone()));
            match unavailable {
                "empty" => std::fs::remove_dir(&custom).unwrap(),
                "not-directory" => std::fs::remove_file(&custom).unwrap(),
                _ => {}
            }
            std::fs::rename(offline, custom).unwrap();
            worker.relocate_media();
            assert_eq!(archived_path(&worker, "image"), Some(source.clone()));
            assert_eq!(std::fs::read(source).unwrap(), b"preserved");
        }
    }

    #[test]
    fn startup_does_not_replace_an_external_path_with_a_same_named_cache_file() {
        let root = tempfile::tempdir().unwrap();
        let (mut worker, _events, _commands, _wa) = super::super::receipt_tests::worker();
        worker.dirs = AppDirs::under(root.path());
        // Legacy archives can retain paths outside the currently configured folder.
        let source = root.path().join("offline/photo.jpg");
        attachment(&worker, "image", &source);
        let decoy = worker.dirs.ensure_media_dir().unwrap().join("photo.jpg");
        std::fs::write(&decoy, b"unrelated").unwrap();

        worker.relocate_media();
        assert_eq!(archived_path(&worker, "image"), Some(source));
        assert_eq!(std::fs::read(decoy).unwrap(), b"unrelated");
    }

    #[test]
    fn startup_never_adopts_a_same_named_file_in_the_custom_folder() {
        let root = tempfile::tempdir().unwrap();
        let (mut worker, _events, _commands, _wa) = super::super::receipt_tests::worker();
        worker.dirs = AppDirs::under(root.path());
        let cache = worker.dirs.ensure_media_dir().unwrap();
        // A stale default-cache row that vanished after an interrupted change.
        let stale = cache.join("photo.jpg");
        attachment(&worker, "image", &stale);
        let custom = root.path().join("downloads");
        std::fs::create_dir_all(&custom).unwrap();
        // An unrelated file in the user's folder with the exact same name.
        let decoy = custom.join("photo.jpg");
        std::fs::write(&decoy, b"unrelated").unwrap();
        worker.dirs.custom_media = Some(custom.clone());

        worker.relocate_media();
        assert_eq!(archived_path(&worker, "image"), None);
        assert_eq!(std::fs::read(decoy).unwrap(), b"unrelated");
    }

    #[test]
    fn startup_still_repairs_and_clears_disposable_cache_paths() {
        let root = tempfile::tempdir().unwrap();
        let (mut worker, _events, _commands, _wa) = super::super::receipt_tests::worker();
        worker.dirs = AppDirs::under(root.path());
        let cache = worker.dirs.ensure_media_dir().unwrap();
        let existing = cache.join("existing.jpg");
        let moved = cache.join("moved.jpg");
        std::fs::write(&existing, b"existing").unwrap();
        std::fs::write(&moved, b"moved").unwrap();
        attachment(&worker, "existing", &existing);
        attachment(&worker, "moved", &cache.join("old/moved.jpg"));
        attachment(&worker, "missing", &cache.join("missing.jpg"));

        worker.relocate_media();
        assert_eq!(archived_path(&worker, "existing"), Some(existing));
        assert_eq!(archived_path(&worker, "moved"), Some(moved));
        assert_eq!(archived_path(&worker, "missing"), None);
    }

    #[tokio::test]
    async fn unavailable_custom_storage_blocks_changes_and_reset_until_it_returns() {
        for unavailable in ["missing", "empty", "not-directory"] {
            for destination in ["new", "default", "same"] {
                let root = tempfile::tempdir().unwrap();
                let (mut worker, events, _commands, _wa) = super::super::receipt_tests::worker();
                worker.dirs = AppDirs::under(root.path());
                let custom = root.path().join("mounted");
                std::fs::create_dir_all(&custom).unwrap();
                let source = custom.join("photo.jpg");
                std::fs::write(&source, b"preserved").unwrap();
                worker.dirs.custom_media = Some(custom.clone());
                attachment(&worker, "image", &source);
                let target = match destination {
                    "new" => Some(root.path().join("new")),
                    "same" => Some(custom.clone()),
                    _ => None,
                };
                let offline = root.path().join("offline");
                std::fs::rename(&custom, &offline).unwrap();
                match unavailable {
                    "empty" => std::fs::create_dir(&custom).unwrap(),
                    "not-directory" => std::fs::write(&custom, b"blocked").unwrap(),
                    _ => {}
                }

                worker.change_media_dir(target.clone()).await;
                assert!(matches!(events.try_recv().unwrap(), Event::Error(_)));
                assert!(events.try_recv().is_err());
                assert_eq!(worker.dirs.custom_media, Some(custom.clone()));
                assert_eq!(archived_path(&worker, "image"), Some(source.clone()));
                match unavailable {
                    "empty" => std::fs::remove_dir(&custom).unwrap(),
                    "not-directory" => std::fs::remove_file(&custom).unwrap(),
                    _ => assert!(!custom.exists(), "must not recreate the source folder"),
                }

                std::fs::rename(offline, custom).unwrap();
                worker.change_media_dir(target).await;
                assert!(matches!(
                    events.try_recv().unwrap(),
                    Event::MediaDirChanged { .. }
                ));
                let saved = archived_path(&worker, "image").unwrap();
                assert_eq!(std::fs::read(saved).unwrap(), b"preserved");
                assert_eq!(std::fs::read(source).unwrap(), b"preserved");
                if destination == "default" {
                    assert_eq!(worker.dirs.custom_media, None);
                }
            }
        }
    }

    #[tokio::test]
    async fn missing_external_file_rolls_back_copies_even_with_readable_folders() {
        for custom_configured in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let (mut worker, events, _commands, _wa) = super::super::receipt_tests::worker();
            worker.dirs = AppDirs::under(root.path());
            let source_dir = root.path().join("external");
            std::fs::create_dir_all(&source_dir).unwrap();
            let present = source_dir.join("present.jpg");
            let missing = source_dir.join("missing.jpg");
            std::fs::write(&present, b"preserved").unwrap();
            if custom_configured {
                worker.dirs.custom_media = Some(source_dir.clone());
            }
            let previous = worker.dirs.custom_media.clone();
            attachment(&worker, "first", &present);
            attachment(&worker, "second", &missing);
            let target = root.path().join("target");
            std::fs::create_dir_all(&target).unwrap();
            let unrelated = target.join("present.jpg");
            std::fs::write(&unrelated, b"unrelated").unwrap();

            worker.change_media_dir(Some(target.clone())).await;
            assert!(matches!(events.try_recv().unwrap(), Event::Error(_)));
            assert!(events.try_recv().is_err());
            assert_eq!(worker.dirs.custom_media, previous);
            assert_eq!(archived_path(&worker, "first"), Some(present.clone()));
            assert_eq!(archived_path(&worker, "second"), Some(missing.clone()));
            assert_eq!(std::fs::read_dir(&target).unwrap().count(), 1);
            assert_eq!(std::fs::read(&unrelated).unwrap(), b"unrelated");
            assert_eq!(std::fs::read(present).unwrap(), b"preserved");

            std::fs::write(missing, b"restored").unwrap();
            worker.change_media_dir(Some(target)).await;
            assert!(matches!(
                events.try_recv().unwrap(),
                Event::MediaDirChanged { .. }
            ));
            assert_eq!(
                std::fs::read(archived_path(&worker, "second").unwrap()).unwrap(),
                b"restored"
            );
            assert_eq!(std::fs::read(unrelated).unwrap(), b"unrelated");
        }
    }

    #[tokio::test]
    async fn unavailable_custom_folder_without_archived_files_can_be_reset() {
        let root = tempfile::tempdir().unwrap();
        let (mut worker, events, _commands, _wa) = super::super::receipt_tests::worker();
        worker.dirs = AppDirs::under(root.path());
        let unavailable = root.path().join("offline");
        worker.dirs.custom_media = Some(unavailable.clone());

        worker.change_media_dir(None).await;
        assert_eq!(worker.dirs.custom_media, None);
        assert!(!unavailable.exists());
        assert!(matches!(
            events.try_recv().unwrap(),
            Event::MediaDirChanged { .. }
        ));
    }

    #[tokio::test]
    async fn ancestor_custom_folder_preserves_copies_but_not_disposable_cache() {
        let root = tempfile::tempdir().unwrap();
        let (mut worker, _events, _commands, _wa) = super::super::receipt_tests::worker();
        worker.dirs = AppDirs::under(root.path());
        let cache = worker.dirs.ensure_media_dir().unwrap();
        let source = cache.join("photo.jpg");
        std::fs::write(&source, b"preserved").unwrap();
        attachment(&worker, "image", &source);
        let unrelated = root.path().join("user.txt");
        std::fs::write(&unrelated, b"unrelated").unwrap();

        worker.change_media_dir(Some(root.path().to_owned())).await;
        let saved = archived_path(&worker, "image").unwrap();
        assert_eq!(saved.parent().unwrap(), root.path().canonicalize().unwrap());
        worker.clean_media_cache();
        assert!(!cache.exists());
        assert_eq!(std::fs::read(saved).unwrap(), b"preserved");
        assert_eq!(std::fs::read(unrelated).unwrap(), b"unrelated");
    }

    #[test]
    fn same_folder_copy_still_checks_that_the_source_is_a_readable_file() {
        let root = tempfile::tempdir().unwrap();
        let missing = root.path().join("missing.jpg");
        let directory = root.path().join("directory.jpg");
        std::fs::create_dir(&directory).unwrap();
        let mut copies = Copies::default();
        assert!(copies.copy(&missing, root.path()).is_err());
        assert!(copies.copy(&directory, root.path()).is_err());
        assert!(directory.is_dir());
    }
}
