//! Where ZapFast keeps its files.
//!
//! Configuration, session state, and caches use separate standard platform
//! directories. Clearing a cache does not remove device keys.

use std::path::{Path, PathBuf};

use directories::ProjectDirs;

use crate::model::AccountId;

#[derive(Clone, Debug)]
pub struct AppDirs {
    pub config: PathBuf,
    pub state: PathBuf,
    pub cache: PathBuf,
    /// Per-user directory for the single-instance lock and control socket.
    pub runtime: PathBuf,
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
                    runtime: fallback.join("zapfast-run"),
                }
            }
        }
    }

    /// Standard platform directories for the app.
    fn of(name: &str) -> Option<Self> {
        let project = ProjectDirs::from("me", "paolino", name)?;
        let state = project
            .state_dir()
            .map(|path| path.to_path_buf())
            .unwrap_or_else(|| project.data_local_dir().to_path_buf());
        Some(Self {
            config: project.config_dir().to_path_buf(),
            runtime: runtime_dir(&project, &state),
            state,
            cache: project.cache_dir().to_path_buf(),
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
            runtime: root.join("run"),
        }
    }

    pub fn settings_file(&self) -> PathBuf {
        self.config.join("settings.json")
    }

    /// Ordered list of account ids and the active one.
    pub fn accounts_file(&self) -> PathBuf {
        self.config.join("accounts.json")
    }

    pub fn account(&self, id: &AccountId) -> AccountDirs {
        assert!(
            AccountId::is_safe(id.as_str()),
            "account id is not a safe folder name"
        );
        AccountDirs {
            id: id.clone(),
            state: self.state.join("accounts").join(id.as_str()),
            cache: self.cache.join("accounts").join(id.as_str()),
        }
    }

    /// Session files at the app root, used by tests that have not moved into
    /// `accounts/<id>/`.
    pub fn as_account(&self) -> AccountDirs {
        AccountDirs {
            id: AccountId::first(),
            state: self.state.clone(),
            cache: self.cache.clone(),
        }
    }

    /// Moves a pre-0.18 single-account layout into `accounts/1/`.
    /// Copies the archive keyring entry before renaming files.
    pub fn adopt_single_account(&self) -> std::io::Result<()> {
        let legacy_session = self.state.join("session.db");
        let legacy_archive = self.state.join("archive.db");
        if !legacy_session.exists() && !legacy_archive.exists() {
            return Ok(());
        }
        let dest = self.account(&AccountId::first());
        dest.ensure()?;
        if legacy_archive.exists() {
            crate::archive::copy_archive_key(&legacy_archive, &dest.archive_db()).map_err(
                |error| std::io::Error::other(format!("could not copy the archive key: {error:#}")),
            )?;
        }
        for name in [
            "session.db",
            "session.db-wal",
            "session.db-shm",
            "session.db-journal",
            "archive.db",
            "archive.db-wal",
            "archive.db-shm",
            "archive.db-journal",
        ] {
            move_file(&self.state.join(name), &dest.state.join(name))?;
        }
        adopt_directory(&self.state.join("stickers"), &dest.saved_sticker_dir())?;
        adopt_directory(&self.cache.join("media"), &dest.media_cache_dir())?;
        adopt_directory(&self.cache.join("avatars"), &dest.avatar_cache_dir())?;
        adopt_directory(&self.cache.join("stickers"), &dest.sticker_cache_dir())?;
        for extension in ["jpg", "png", "webp", "gif"] {
            move_file(
                &self.state.join(format!("wallpaper.{extension}")),
                &dest.wallpaper_file(extension),
            )?;
        }
        Ok(())
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

    /// ZapFast's copy of the chosen chat wallpaper image. User data, so it
    /// sits beside saved stickers rather than in the cache.
    pub fn wallpaper_file(&self, extension: &str) -> PathBuf {
        self.state.join(format!("wallpaper.{extension}"))
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

/// Per-account session, archive, stickers, and caches.
#[derive(Clone, Debug)]
pub struct AccountDirs {
    pub id: AccountId,
    pub state: PathBuf,
    pub cache: PathBuf,
}

impl AccountDirs {
    pub fn ensure(&self) -> std::io::Result<()> {
        for dir in [&self.state, &self.cache] {
            let mut builder = std::fs::DirBuilder::new();
            builder.recursive(true);
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

    /// whatsapp-rust device identity. Deleting this database unlinks the account.
    pub fn session_db(&self) -> PathBuf {
        self.state.join("session.db")
    }

    pub fn archive_db(&self) -> PathBuf {
        self.state.join("archive.db")
    }

    pub fn settings_file(&self) -> PathBuf {
        self.state.join("settings.json")
    }

    pub fn media_cache_dir(&self) -> PathBuf {
        self.cache.join("media")
    }

    pub fn avatar_cache_dir(&self) -> PathBuf {
        self.cache.join("avatars")
    }

    pub fn sticker_cache_dir(&self) -> PathBuf {
        self.cache.join("stickers")
    }

    pub fn saved_sticker_dir(&self) -> PathBuf {
        self.state.join("stickers")
    }

    /// ZapFast's copy of the chosen chat wallpaper image for this account.
    pub fn wallpaper_file(&self, extension: &str) -> PathBuf {
        self.state.join(format!("wallpaper.{extension}"))
    }

    pub fn avatar_file(&self, id: &str, full: bool) -> PathBuf {
        let stem: String = id
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect();
        self.avatar_cache_dir()
            .join(format!("{stem}{}.jpg", if full { "-full" } else { "" }))
    }
}

fn move_file(from: &Path, to: &Path) -> std::io::Result<()> {
    if from.exists() && !to.try_exists()? {
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::rename(from, to)?;
    }
    Ok(())
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

/// Private directory for the instance lock and control channel. Uses
/// `$XDG_RUNTIME_DIR` where there is one. Elsewhere the directory sits
/// beside the state directory rather than inside it, because the state
/// directory must not exist before an earlier name's data is adopted.
fn runtime_dir(project: &ProjectDirs, state: &Path) -> PathBuf {
    // Flatpak gives each sandbox a private runtime directory and shares only
    // this one between instances of the app.
    #[cfg(target_os = "linux")]
    if let (Some(runtime), Some(id)) = (
        project.runtime_dir().and_then(Path::parent),
        std::env::var_os("FLATPAK_ID"),
    ) {
        return runtime.join("app").join(id);
    }
    if let Some(runtime) = project.runtime_dir() {
        return runtime.to_path_buf();
    }
    let mut name = state.file_name().unwrap_or_default().to_os_string();
    name.push(".run");
    state.with_file_name(name)
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
            runtime: root.join("old/run"),
        };
        let new = AppDirs {
            config: root.join("new/data"),
            state: root.join("new/data"),
            cache: root.join("new/cache"),
            runtime: root.join("new/run"),
        };
        old.ensure().unwrap();
        std::fs::write(old.session_db(), b"session").unwrap();
        new.adopt(&old).unwrap();
        assert_eq!(std::fs::read(new.session_db()).unwrap(), b"session");
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

    #[test]
    fn a_legacy_session_moves_into_the_first_account_folder() {
        let root = root("single-account");
        let dirs = AppDirs::under(&root);
        dirs.ensure().unwrap();
        std::fs::write(dirs.session_db(), b"session").unwrap();
        std::fs::write(dirs.archive_db(), b"SQLite format 3\0").unwrap();
        std::fs::create_dir_all(dirs.media_cache_dir()).unwrap();
        std::fs::write(dirs.media_cache_dir().join("photo.jpg"), b"photo").unwrap();
        std::fs::write(dirs.state.join("wallpaper.png"), b"wall").unwrap();
        dirs.adopt_single_account().unwrap();
        let account = dirs.account(&AccountId::first());
        assert_eq!(std::fs::read(account.session_db()).unwrap(), b"session");
        assert_eq!(
            std::fs::read(account.archive_db()).unwrap(),
            b"SQLite format 3\0"
        );
        assert_eq!(
            std::fs::read(account.media_cache_dir().join("photo.jpg")).unwrap(),
            b"photo"
        );
        assert!(!dirs.session_db().exists());
        assert!(!dirs.archive_db().exists());
        assert_eq!(
            std::fs::read(account.wallpaper_file("png")).unwrap(),
            b"wall"
        );
        assert!(!dirs.state.join("wallpaper.png").exists());
        std::fs::remove_dir_all(root).unwrap();
    }
}
