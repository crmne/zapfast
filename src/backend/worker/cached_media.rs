//! Repair a stale cache reference only after the reader tries to open its file.
use super::{Event, Worker};
use std::io;
use std::path::{Component, Path};

impl Worker {
    /// Checks an explicitly opened file on the worker; missing files trigger guarded repair.
    /// Filesystem and privacy errors are returned without allowing the open to proceed.
    pub(super) fn check_cached_attachment(
        &self,
        chat: &str,
        id: &str,
        path: &Path,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(self.privacy_ready, "The attachment is not available");
        match std::fs::metadata(path) {
            Ok(metadata) => {
                anyhow::ensure!(metadata.is_file(), "The attachment is not a regular file")
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                self.recover_cached_attachment(chat, id, path)?;
                anyhow::bail!("The cached attachment is missing. Download it again.");
            }
            Err(error) => return Err(error.into()),
        }
        Ok(())
    }

    /// Clears only a confirmed missing cache path, preserving replacement files and edits.
    /// Notifies the interface only after the changed content has been saved successfully.
    pub(super) fn recover_cached_attachment(
        &self,
        chat: &str,
        id: &str,
        path: &Path,
    ) -> crate::archive::Result<()> {
        if !self.privacy_ready
            || !confirmed_missing(&self.dirs.media_cache_dir(), path, |path| {
                std::fs::symlink_metadata(path)
            })
        {
            return Ok(());
        }
        let chat = self.canonical_str(chat);
        let Some(mut message) = self.archive.message(&chat, id)? else {
            return Ok(());
        };
        let before = message.content.clone();
        message.content.forget_file(path);
        if message.content != before
            && self
                .archive
                .set_content(&chat, id, &message.content, message.edited)?
        {
            self.emit(Event::CachedMediaMissing {
                chat,
                message: id.into(),
                paths: vec![path.to_owned()],
            });
        }
        Ok(())
    }
}

/// A missing file under an accessible cache is recoverable; errors, links and external paths stay.
fn confirmed_missing(
    root: &Path,
    path: &Path,
    metadata: impl Fn(&Path) -> io::Result<std::fs::Metadata>,
) -> bool {
    let Ok(relative) = path.strip_prefix(root) else {
        return false;
    };
    if relative.as_os_str().is_empty()
        || relative
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return false;
    }
    let Ok(root_metadata) = metadata(root) else {
        return false;
    };
    if !root_metadata.is_dir() || is_link(&root_metadata) {
        return false;
    }
    let mut current = root.to_path_buf();
    for part in relative.components() {
        current.push(part.as_os_str());
        match metadata(&current) {
            Err(error) => return error.kind() == io::ErrorKind::NotFound,
            Ok(metadata) => {
                if is_link(&metadata) || (current != path && !metadata.is_dir()) {
                    return false;
                }
            }
        }
    }
    false
}

/// Windows junctions and other reparse points must not redirect the cache probe.
fn is_link(metadata: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

#[cfg(test)]
mod tests {
    use super::super::receipt_tests;
    use super::*;
    use crate::model::{Content, Media, MediaState};

    #[test]
    fn an_explicit_failed_open_repairs_only_the_same_cached_file() {
        let (mut worker, events, _, _) = receipt_tests::worker();
        let root = worker.dirs.media_cache_dir();
        std::fs::create_dir_all(&root).unwrap();
        let missing = root.join("missing.pdf");
        let replacement = root.join("replacement.pdf");
        let mut row = receipt_tests::own_message("file", 100);
        row.edited = true;
        row.content = Content::Document {
            media: Media {
                mime: "application/pdf".into(),
                size: 4,
                width: None,
                height: None,
                album: None,
                path: Some(missing.clone()),
                state: MediaState::Idle,
            },
            file_name: "fixture.pdf".into(),
            caption: Some("Synthetic edit".into()),
            pages: None,
        };
        worker
            .archive
            .insert_message(&row, Some(b"synthetic keys"))
            .unwrap();
        worker.send_page(&row.chat, None);
        assert!(
            worker
                .archive
                .message(&row.chat, &row.id)
                .unwrap()
                .unwrap()
                .content
                .media()
                .unwrap()
                .path
                .is_some()
        );
        assert!(
            !events
                .try_iter()
                .any(|event| matches!(event, Event::CachedMediaMissing { .. }))
        );
        std::fs::write(&missing, b"synthetic file").unwrap();
        worker
            .check_cached_attachment(&row.chat, &row.id, &missing)
            .unwrap();
        std::fs::remove_file(&missing).unwrap();
        assert!(
            worker
                .check_cached_attachment(&row.chat, &row.id, &missing)
                .is_err()
        );
        let repaired = worker.archive.message(&row.chat, &row.id).unwrap().unwrap();
        assert!(repaired.content.media().unwrap().path.is_none());
        assert!(repaired.edited);
        assert_eq!(
            worker.archive.raw(&row.chat, &row.id).unwrap().as_deref(),
            Some(b"synthetic keys".as_slice())
        );
        assert!(matches!(
            events.try_recv().unwrap(),
            Event::CachedMediaMissing { .. }
        ));
        row.content.media_mut().unwrap().path = Some(replacement.clone());
        worker
            .archive
            .set_content(&row.chat, &row.id, &row.content, true)
            .unwrap();
        worker
            .recover_cached_attachment(&row.chat, &row.id, &missing)
            .unwrap();
        assert_eq!(
            worker
                .archive
                .message(&row.chat, &row.id)
                .unwrap()
                .unwrap()
                .content
                .media()
                .unwrap()
                .path
                .as_ref(),
            Some(&replacement)
        );
        assert!(events.try_recv().is_err());
        crate::archive::tests::set_derived_update_failure(&worker.archive, true);
        assert!(
            worker
                .recover_cached_attachment(&row.chat, &row.id, &replacement)
                .is_err()
        );
        assert!(events.try_recv().is_err());
    }

    #[test]
    fn unavailable_cache_permissions_and_external_paths_are_preserved() {
        let root = tempfile::tempdir().unwrap();
        let missing = root.path().join("missing.pdf");
        let stat = |path: &Path| std::fs::symlink_metadata(path);
        assert!(confirmed_missing(root.path(), &missing, stat));
        assert!(!confirmed_missing(
            &root.path().join("absent"),
            &root.path().join("absent/file"),
            stat
        ));
        assert!(!confirmed_missing(
            root.path(),
            &root.path().join("../outside"),
            stat
        ));
        assert!(!confirmed_missing(
            &root.path().join("other-account"),
            &missing,
            stat
        ));
        assert!(!confirmed_missing(root.path(), &missing, |path| {
            if path == missing {
                Err(io::ErrorKind::PermissionDenied.into())
            } else {
                stat(path)
            }
        }));
    }

    #[cfg(unix)]
    #[test]
    fn linked_cache_directories_are_preserved() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), root.path().join("link")).unwrap();
        assert!(!confirmed_missing(
            root.path(),
            &root.path().join("link/missing"),
            |path| std::fs::symlink_metadata(path)
        ));
    }
}
