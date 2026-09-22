//! Where ZapFast keeps its files.
//!
//! Configuration, session state, and caches use separate standard platform
//! directories. Clearing a cache does not remove device keys.

use std::path::{Path, PathBuf};

use directories::ProjectDirs;

#[derive(Clone, Debug)]
pub struct AppDirs {
    pub config: PathBuf,
    pub state: PathBuf,
    pub cache: PathBuf,
    pub custom_media: Option<PathBuf>,
}

impl AppDirs {
    pub fn discover() -> Self {
        match Self::of("zapfast") {
            Some(dirs) => dirs,
            None => {
                let fallback = std::env::current_dir().unwrap_or_default();
                Self {
                    config: fallback.join("zapfast-config"),
                    state: fallback.join("zapfast-state"),
                    cache: fallback.join("zapfast-cache"),
                    custom_media: None,
                }
            }
        }
    }

    /// Standard platform directories for the app.
    fn of(name: &str) -> Option<Self> {
        let project = ProjectDirs::from("me", "paolino", name)?;
        Some(Self {
            config: project.config_dir().to_path_buf(),
            state: project
                .state_dir()
                .map(|path| path.to_path_buf())
                .unwrap_or_else(|| project.data_local_dir().to_path_buf()),
            cache: project.cache_dir().to_path_buf(),
            custom_media: None,
        })
    }

    /// Adopts earlier names, newest first, without replacing existing data.
    /// Call only after acquiring the instance guard, and never for demo runs.
    pub fn adopt_previous_names(&self) -> std::io::Result<()> {
        for name in ["fastsapp", "fastwhatsapp"] {
            if let Some(old) = Self::of(name) {
                self.adopt(&old)?;
            }
            if let (Some(from), Some(to)) =
                (eframe::storage_dir(name), eframe::storage_dir("zapfast"))
            {
                adopt_directory(&from, &to)?;
            }
        }
        Ok(())
    }

    fn adopt(&self, old: &Self) -> std::io::Result<()> {
        for (from, to) in [
            (&old.config, &self.config),
            (&old.state, &self.state),
            (&old.cache, &self.cache),
        ] {
            adopt_directory(from, to)?;
        }
        Ok(())
    }

    /// Places all data under one directory for tests and temporary runs.
    pub fn under(root: &std::path::Path) -> Self {
        Self {
            config: root.join("config"),
            state: root.join("state"),
            cache: root.join("cache"),
            custom_media: None,
        }
    }

    pub fn settings_file(&self) -> PathBuf {
        self.config.join("settings.json")
    }

    /// whatsapp-rust device identity, Signal sessions, and state keys.
    /// Deleting this database unlinks the computer.
    pub fn session_db(&self) -> PathBuf {
        self.state.join("session.db")
    }

    /// Local message archive.
    pub fn archive_db(&self) -> PathBuf {
        self.state.join("archive.db")
    }

    /// Current-run log, replaced at startup.
    pub fn log_file(&self) -> PathBuf {
        self.state.join("zapfast.log")
    }

    /// Panic log written before process exit.
    pub fn panic_log(&self) -> PathBuf {
        self.state.join("panic.log")
    }

    /// Downloaded attachments keyed by message id.
    pub fn media_cache_dir(&self) -> PathBuf {
        self.cache.join("media")
    }

    /// Effective attachment directory, using a custom path when configured.
    pub fn media_dir(&self) -> PathBuf {
        self.custom_media
            .clone()
            .unwrap_or_else(|| self.media_cache_dir())
    }

    /// Creates the default attachment folder before use. A configured custom
    /// folder must already exist: recreating a disconnected mount would save
    /// into it locally and hide the file when the drive is mounted again.
    pub fn ensure_media_dir(&self) -> std::io::Result<PathBuf> {
        let dir = self.media_dir();
        if self.custom_media.is_some() {
            return match std::fs::metadata(&dir) {
                Ok(metadata) if metadata.is_dir() => Ok(dir),
                Ok(_) => Err(std::io::Error::new(
                    std::io::ErrorKind::NotADirectory,
                    "Attachment folder is not a directory",
                )),
                Err(error) => Err(error),
            };
        }
        std::fs::create_dir_all(&dir)?;
        Ok(dir)
    }

    /// Returns true if `path` resolves to the default media cache directory.
    pub fn is_default_media_dir(&self, path: &Path) -> bool {
        Self::resolved_for_compare(path) == Self::resolved_for_compare(&self.media_cache_dir())
    }

    /// Returns true if `path` is equal to or contained within `self.cache`.
    pub fn is_cache_path(&self, path: &Path) -> bool {
        Self::is_subpath(path, &self.cache)
    }

    /// Checks whether `child` is equal to or located within `parent`.
    pub fn is_subpath(child: &Path, parent: &Path) -> bool {
        Self::resolved_for_compare(child).starts_with(Self::resolved_for_compare(parent))
    }

    /// Resolves a path as far as it currently exists, following symlinks in
    /// existing ancestors and collapsing `.` and `..` in the missing tail, so
    /// a not-yet-created path is compared by where it will actually live once
    /// the missing components appear.
    fn resolved_for_compare(path: &Path) -> PathBuf {
        if let Ok(resolved) = path.canonicalize() {
            return simplify_verbatim(resolved);
        }
        let components: Vec<_> = path.components().collect();
        let mut existing = 0;
        for len in (1..=components.len()).rev() {
            let prefix = PathBuf::from_iter(&components[..len]);
            if prefix.try_exists().unwrap_or(false) {
                existing = len;
                break;
            }
        }
        let base = PathBuf::from_iter(&components[..existing])
            .canonicalize()
            .map(simplify_verbatim)
            .unwrap_or_else(|_| PathBuf::from_iter(&components[..existing]));
        let mut out = base;
        for component in &components[existing..] {
            use std::path::Component;
            match component {
                Component::ParentDir => {
                    out.pop();
                }
                Component::CurDir => {}
                _ => out.push(component.as_os_str()),
            }
        }
        out
    }

    /// Validates a persisted custom folder before wiring it into the runtime.
    /// A hand-edited settings file, or a path whose symlink was swapped since
    /// it was saved, must not make downloads write into the cache, session,
    /// archive, or settings directories. A folder that no longer exists is
    /// kept: it may be a disconnected mount that returns later.
    pub fn validated_custom_media(&self, custom: Option<PathBuf>) -> Option<PathBuf> {
        let custom = custom?;
        // The app only ever persists absolute canonical paths (a folder
        // picker plus `try_change_media_dir`), so a relative entry means a
        // hand-edited settings file. Keeping it would make every media path
        // resolve against whatever the process working directory happens to
        // be, outside every boundary check below.
        if custom.is_relative() {
            log::warn!(
                "ignoring persisted attachment folder that is not an absolute path: {}",
                custom.display()
            );
            return None;
        }
        // The cache boundary check runs first and works lexically, so a
        // hand-edited path equal to the default media folder is rejected even
        // before that folder has ever been created on disk.
        if self.is_default_media_dir(&custom) || self.is_cache_path(&custom) {
            log::warn!(
                "ignoring persisted attachment folder inside the cache: {}",
                custom.display()
            );
            return None;
        }
        match std::fs::metadata(&custom) {
            Ok(metadata) if metadata.is_dir() => {}
            Ok(_) => {
                log::warn!(
                    "ignoring persisted attachment folder that is not a directory: {}",
                    custom.display()
                );
                return None;
            }
            // A missing folder may be an unmounted drive; retain the path so
            // the archive rows keep pointing at it, as `ensure_media_dir` does.
            // The app-data boundaries still apply to it: a not-yet-created
            // folder inside state or config must not start receiving
            // downloads if it appears later. `is_subpath` compares such a path
            // by where it will resolve once it appears, `..` segments and
            // symlinked ancestors included.
            Err(_) => {
                if AppDirs::is_subpath(&custom, &self.state)
                    || AppDirs::is_subpath(&custom, &self.config)
                {
                    log::warn!(
                        "ignoring persisted attachment folder inside the app data folders: {}",
                        custom.display()
                    );
                    return None;
                }
                return Some(custom);
            }
        }
        if AppDirs::is_subpath(&custom, &self.state) || AppDirs::is_subpath(&custom, &self.config) {
            log::warn!(
                "ignoring persisted attachment folder inside the app data folders: {}",
                custom.display()
            );
            return None;
        }
        Some(custom.canonicalize().unwrap_or(custom))
    }

    /// Profile pictures keyed by chat.
    pub fn avatar_cache_dir(&self) -> PathBuf {
        self.cache.join("avatars")
    }

    /// Recent phone stickers keyed by file hash.
    pub fn sticker_cache_dir(&self) -> PathBuf {
        self.cache.join("stickers")
    }

    /// Saved stickers keyed by content hash. These are user data, not cache.
    pub fn saved_sticker_dir(&self) -> PathBuf {
        self.state.join("stickers")
    }

    /// Cached profile-picture path. `full` selects the info-dialog size.
    pub fn avatar_file(&self, id: &str, full: bool) -> PathBuf {
        let stem: String = id
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect();
        self.avatar_cache_dir()
            .join(format!("{stem}{}.jpg", if full { "-full" } else { "" }))
    }

    pub fn ensure(&self) -> std::io::Result<()> {
        for dir in [&self.config, &self.state, &self.cache] {
            let mut builder = std::fs::DirBuilder::new();
            builder.recursive(true);
            // Create new directories privately, even with a permissive umask.
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            builder.create(dir)?;
            restrict_directory(dir)?;
        }
        Ok(())
    }
}

#[cfg(unix)]
fn restrict_directory(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
}

#[cfg(not(unix))]
fn restrict_directory(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

/// Rename whole directories so SQLite databases travel with their WAL files.
/// A failed move stops startup before empty replacement directories are made.
fn adopt_directory(from: &Path, to: &Path) -> std::io::Result<()> {
    if from.is_dir() && !to.try_exists()? {
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::rename(from, to)?;
    }
    Ok(())
}

/// Canonical paths on Windows carry the `\\?\` verbatim prefix, which never
/// compares equal to the plain spelling of the same path, so a resolved child
/// would not match a parent that has nothing to canonicalize yet. Strip the
/// prefix the way `dunce` does; on other platforms this is the identity.
#[cfg(windows)]
fn simplify_verbatim(path: PathBuf) -> PathBuf {
    let text = path.as_os_str().to_string_lossy();
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{rest}"))
    } else if let Some(rest) = text.strip_prefix(r"\\?\") {
        PathBuf::from(rest)
    } else {
        path
    }
}

#[cfg(not(windows))]
fn simplify_verbatim(path: PathBuf) -> PathBuf {
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root(name: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("zapfast-paths-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[cfg(unix)]
    #[test]
    fn ensure_restricts_base_directories() {
        use std::os::unix::fs::PermissionsExt;

        let root = root("permissions");
        let dirs = AppDirs::under(&root);
        dirs.ensure().unwrap();
        for path in [&dirs.config, &dirs.state, &dirs.cache] {
            assert_eq!(
                std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o700
            );
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn ensure_repairs_existing_directory_permissions_without_changing_data() {
        use std::os::unix::fs::PermissionsExt;

        let root = root("existing-permissions");
        let dirs = AppDirs::under(&root);
        for path in [&dirs.config, &dirs.state, &dirs.cache] {
            std::fs::create_dir_all(path).unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
            std::fs::write(path.join("fixture"), b"preserved").unwrap();
        }
        dirs.ensure().unwrap();
        for path in [&dirs.config, &dirs.state, &dirs.cache] {
            assert_eq!(
                std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o700
            );
            assert_eq!(std::fs::read(path.join("fixture")).unwrap(), b"preserved");
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn ensure_stops_when_an_application_directory_cannot_be_created() {
        let root = root("blocked-directory");
        let dirs = AppDirs::under(&root);
        std::fs::write(&dirs.state, b"existing file").unwrap();
        assert!(dirs.ensure().is_err());
        assert!(!dirs.cache.exists());
        assert_eq!(std::fs::read(&dirs.state).unwrap(), b"existing file");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rename_preserves_session_archive_settings_and_cached_files() {
        for name in ["fastsapp", "fastwhatsapp"] {
            let root = root(name);
            let old = AppDirs::under(&root.join(name));
            let new = AppDirs::under(&root.join("zapfast"));
            old.ensure().unwrap();
            for path in [
                old.settings_file(),
                old.session_db(),
                old.state.join("session.db-wal"),
                old.archive_db(),
                old.state.join("archive.db-wal"),
                old.saved_sticker_dir().join("pack/sticker.webp"),
                old.media_cache_dir().join("photo.jpg"),
            ] {
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(path, b"preserved").unwrap();
            }
            new.adopt(&old).unwrap();
            new.adopt(&old).unwrap(); // A second launch is a no-op.
            for path in [
                new.settings_file(),
                new.session_db(),
                new.state.join("session.db-wal"),
                new.archive_db(),
                new.state.join("archive.db-wal"),
                new.saved_sticker_dir().join("pack/sticker.webp"),
                new.media_cache_dir().join("photo.jpg"),
            ] {
                assert_eq!(std::fs::read(path).unwrap(), b"preserved");
            }
            assert!(!old.config.exists());
            assert!(!old.state.exists());
            assert!(!old.cache.exists());
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn newest_data_wins_without_merging_archives() {
        let root = root("precedence");
        let new = AppDirs::under(&root.join("zapfast"));
        let recent = AppDirs::under(&root.join("fastsapp"));
        let oldest = AppDirs::under(&root.join("fastwhatsapp"));
        recent.ensure().unwrap();
        oldest.ensure().unwrap();
        std::fs::create_dir_all(&new.config).unwrap();
        std::fs::write(new.settings_file(), b"new settings").unwrap();
        std::fs::write(recent.settings_file(), b"old settings").unwrap();
        std::fs::write(recent.archive_db(), b"recent archive").unwrap();
        std::fs::write(oldest.archive_db(), b"oldest archive").unwrap();
        new.adopt(&recent).unwrap();
        new.adopt(&oldest).unwrap();
        assert_eq!(std::fs::read(new.settings_file()).unwrap(), b"new settings");
        assert_eq!(
            std::fs::read(recent.settings_file()).unwrap(),
            b"old settings"
        );
        assert_eq!(std::fs::read(new.archive_db()).unwrap(), b"recent archive");
        assert_eq!(
            std::fs::read(oldest.archive_db()).unwrap(),
            b"oldest archive"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn shared_config_and_state_directory_moves_once() {
        let root = root("shared");
        let old = AppDirs {
            config: root.join("old/data"),
            state: root.join("old/data"),
            cache: root.join("old/cache"),
            custom_media: None,
        };
        let new = AppDirs {
            config: root.join("new/data"),
            state: root.join("new/data"),
            cache: root.join("new/cache"),
            custom_media: None,
        };
        old.ensure().unwrap();
        std::fs::write(old.session_db(), b"session").unwrap();
        new.adopt(&old).unwrap();
        assert_eq!(std::fs::read(new.session_db()).unwrap(), b"session");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn media_dir_uses_custom_path_when_configured() {
        let root = root("custom-media");
        let mut dirs = AppDirs::under(&root);
        assert_eq!(dirs.media_dir(), dirs.media_cache_dir());
        let custom = root.join("my-custom-media");
        dirs.custom_media = Some(custom.clone());
        assert_eq!(dirs.media_dir(), custom);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cache_path_detection_identifies_subpaths() {
        let root = root("cache-detection");
        let dirs = AppDirs::under(&root);
        dirs.ensure().unwrap();

        assert!(dirs.is_default_media_dir(&dirs.media_cache_dir()));
        assert!(dirs.is_cache_path(&dirs.media_cache_dir()));
        assert!(dirs.is_cache_path(&dirs.media_cache_dir().join("subfolder")));
        assert!(!dirs.is_cache_path(&root.join("external-downloads")));

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn opening_media_folder_creates_the_cache_but_never_a_custom_folder() {
        let root = tempfile::tempdir().unwrap();
        let mut dirs = AppDirs::under(root.path());
        dirs.ensure().unwrap();
        assert!(!dirs.media_dir().exists());
        assert!(dirs.ensure_media_dir().unwrap().is_dir());
        std::fs::remove_dir_all(dirs.media_cache_dir()).unwrap();
        assert!(dirs.ensure_media_dir().unwrap().is_dir());

        // A missing custom folder may be a disconnected mount; recreating it
        // would save locally and hide the file when the drive is mounted.
        let missing = root.path().join("custom");
        dirs.custom_media = Some(missing.clone());
        assert!(dirs.ensure_media_dir().is_err());
        assert!(!missing.exists());
        std::fs::create_dir(&missing).unwrap();
        assert_eq!(dirs.ensure_media_dir().unwrap(), missing);
        let blocked = root.path().join("file");
        std::fs::write(&blocked, b"fixture").unwrap();
        dirs.custom_media = Some(blocked.clone());
        assert!(dirs.ensure_media_dir().is_err());
        assert_eq!(std::fs::read(blocked).unwrap(), b"fixture");
    }

    #[test]
    fn validated_custom_media_rejects_boundaries_but_keeps_valid_and_missing_folders() {
        let root = root("validated-custom");
        let dirs = AppDirs::under(&root);
        dirs.ensure().unwrap();
        // The default cache and its subfolders are never adopted from settings.
        assert_eq!(
            dirs.validated_custom_media(Some(dirs.media_cache_dir())),
            None
        );
        let nested = dirs.cache.join("media/nested");
        std::fs::create_dir_all(&nested).unwrap();
        assert_eq!(dirs.validated_custom_media(Some(nested)), None);
        // Neither are folders inside the app data directories.
        let in_state = root.join("state/attachments");
        std::fs::create_dir_all(&in_state).unwrap();
        assert_eq!(dirs.validated_custom_media(Some(in_state)), None);
        let in_config = root.join("config/attachments");
        std::fs::create_dir_all(&in_config).unwrap();
        assert_eq!(dirs.validated_custom_media(Some(in_config)), None);
        // A file instead of a folder is dropped, never followed.
        let file = root.join("settings-file");
        std::fs::write(&file, b"fixture").unwrap();
        assert_eq!(dirs.validated_custom_media(Some(file.clone())), None);
        assert_eq!(std::fs::read(file).unwrap(), b"fixture");
        // A missing folder is kept: it may be a disconnected mount.
        let missing = root.join("mounted");
        assert_eq!(
            dirs.validated_custom_media(Some(missing.clone())),
            Some(missing)
        );
        // Except when it lives inside the app data folders: boundaries apply
        // to not-yet-created paths too, before they can start receiving files.
        let missing_in_state = root.join("state/future-attachments");
        assert_eq!(dirs.validated_custom_media(Some(missing_in_state)), None);
        let missing_in_config = root.join("config/future-attachments");
        assert_eq!(dirs.validated_custom_media(Some(missing_in_config)), None);
        // A valid folder is kept, with symlinks and '.' segments resolved.
        let valid = root.join("downloads/./sub");
        std::fs::create_dir_all(&valid).unwrap();
        let kept = dirs.validated_custom_media(Some(valid.clone())).unwrap();
        assert_eq!(kept, valid.canonicalize().unwrap());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn is_subpath_resolves_dotdot_and_symlink_ancestors_in_missing_paths() {
        let root = root("subpath-resolution");
        let dirs = AppDirs::under(&root);
        dirs.ensure().unwrap();
        // A `..` segment in a missing path collapses against the existing
        // ancestors, so it cannot smuggle the folder into app data.
        assert!(AppDirs::is_subpath(
            &root.join("scratch/../state/attachments"),
            &dirs.state
        ));
        assert!(!AppDirs::is_subpath(
            &root.join("scratch/../downloads"),
            &dirs.state
        ));
        #[cfg(unix)]
        {
            // A symlinked ancestor resolves too: the path lands in app data
            // as soon as the missing component appears under the link.
            std::os::unix::fs::symlink(&dirs.state, root.join("linked-state")).unwrap();
            assert!(AppDirs::is_subpath(
                &root.join("linked-state/attachments"),
                &dirs.state
            ));
            assert!(!AppDirs::is_subpath(
                &root.join("linked-state/attachments"),
                &dirs.config
            ));
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn validated_custom_media_rejects_relative_paths() {
        let root = root("validated-relative");
        let dirs = AppDirs::under(&root);
        dirs.ensure().unwrap();
        // The app only persists absolute canonical paths, so a relative entry
        // means a hand-edited settings file; keeping it would resolve every
        // media path against the process working directory instead.
        assert_eq!(
            dirs.validated_custom_media(Some(PathBuf::from("attachments"))),
            None
        );
        assert_eq!(
            dirs.validated_custom_media(Some(PathBuf::from("state/attachments"))),
            None
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn default_media_dir_matches_dotladen_spellings_before_it_exists() {
        let root = root("default-media-resolution");
        let dirs = AppDirs::under(&root);
        dirs.ensure().unwrap();
        assert!(dirs.is_default_media_dir(&dirs.media_cache_dir()));
        // The default folder is created lazily; equivalent spellings of it
        // must still count as the default, not as a custom folder.
        assert!(dirs.is_default_media_dir(&dirs.cache.join("media")));
        assert!(dirs.is_default_media_dir(&dirs.cache.join("./media")));
        assert!(dirs.is_default_media_dir(&dirs.cache.join("sub/../media")));
        assert!(!dirs.is_default_media_dir(&dirs.cache.join("media2")));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn validated_custom_media_normalizes_missing_paths_before_boundary_checks() {
        let root = root("validated-missing-normalized");
        let dirs = AppDirs::under(&root);
        dirs.ensure().unwrap();
        // `..` segments must not smuggle a not-yet-created folder past the
        // app-data checks: it resolves into `state` once it appears.
        let dotdot = root.join("scratch/../state/attachments");
        assert_eq!(dirs.validated_custom_media(Some(dotdot)), None);
        // The same shape staying outside app data is kept as configured.
        let outside = root.join("scratch/../downloads");
        assert_eq!(
            dirs.validated_custom_media(Some(outside.clone())),
            Some(outside)
        );
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&dirs.state, root.join("linked-state")).unwrap();
            let via_link = root.join("linked-state/attachments");
            assert_eq!(dirs.validated_custom_media(Some(via_link)), None);
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn failed_migration_leaves_source_available_for_retry() {
        let root = root("failure");
        let old = AppDirs::under(&root.join("old"));
        let new = AppDirs::under(&root.join("blocked/new"));
        old.ensure().unwrap();
        std::fs::write(old.session_db(), b"session").unwrap();
        std::fs::write(root.join("blocked"), b"not a directory").unwrap();
        assert!(new.adopt(&old).is_err());
        assert_eq!(std::fs::read(old.session_db()).unwrap(), b"session");
        std::fs::remove_file(root.join("blocked")).unwrap();
        new.adopt(&old).unwrap();
        assert_eq!(std::fs::read(new.session_db()).unwrap(), b"session");
        std::fs::remove_dir_all(root).unwrap();
    }
}
