//! SQLCipher archive storage and automatic OS-keyring unlock.

use std::{fs, io::Read, path::Path};

use anyhow::{Context, Result, ensure};
use keyring_core::api::CredentialStoreApi;
use rusqlite::{Connection, OpenFlags};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

const HEADER: &[u8; 16] = b"SQLite format 3\0";

fn plaintext(path: &Path) -> Result<bool> {
    let mut file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(true),
        Err(error) => return Err(error.into()),
    };
    if file.metadata()?.len() == 0 {
        return Ok(true);
    }
    let mut header = [0; 16];
    let read = file.read(&mut header)?;
    Ok(read < 16 || &header == HEADER)
}

#[cfg(target_os = "linux")]
type PlatformStore = zbus_secret_service_keyring_store::Store;
#[cfg(target_os = "macos")]
type PlatformStore = apple_native_keyring_store::keychain::Store;
#[cfg(windows)]
type PlatformStore = windows_native_keyring_store::Store;

/// The keyring credential label for an archive: a digest of its directory,
/// never a user path, phone number, or message data. Separate profiles must
/// not overwrite each other's keys.
fn identity(path: &Path) -> Result<String> {
    let parent = path.parent().context("Archive has no parent directory")?;
    let digest = Sha256::digest(parent.canonicalize()?.as_os_str().as_encoded_bytes());
    Ok(format!(
        "archive-{}",
        digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    ))
}

fn platform_store() -> Result<std::sync::Arc<PlatformStore>> {
    #[cfg(target_os = "linux")]
    let store = zbus_secret_service_keyring_store::Store::new();
    #[cfg(target_os = "macos")]
    let store = apple_native_keyring_store::keychain::Store::new();
    #[cfg(windows)]
    let store = windows_native_keyring_store::Store::new();
    store
        .map_err(keyring_error)
        .context("Unlock your OS keyring and restart ZapFast")
}

fn entry_for(store: &impl CredentialStoreApi, path: &Path) -> Result<keyring_core::Entry> {
    store
        .build("rocks.zapfast.ZapFast", &identity(path)?, None)
        .map_err(keyring_error)
        .context("The OS keyring could not open ZapFast's archive key")
}

pub(super) fn key_for(path: &Path) -> Result<Zeroizing<[u8; 32]>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let store = platform_store()?;
    let entry = entry_for(&*store, path)?;
    key_from_entry(path, &entry)
}

/// Moves an encrypted archive's keyring credential to the new path before its
/// file moves there. A plaintext or empty archive has no key: opening it at
/// the new path creates one there, and this returns without touching the
/// keyring.
pub(super) fn move_key(from: &Path, to: &Path) -> Result<()> {
    if plaintext(from)? {
        return Ok(());
    }
    let store = platform_store()?;
    move_key_in(&*store, from, to)
}

/// The credential move itself. The old entry is removed only after the new
/// one reads back, so an interruption leaves the key reachable from both
/// paths.
fn move_key_in(store: &impl CredentialStoreApi, from: &Path, to: &Path) -> Result<()> {
    let old = entry_for(store, from)?;
    let secret = match old.get_secret() {
        Ok(secret) => Zeroizing::new(secret),
        Err(keyring_core::Error::NoEntry) => {
            anyhow::bail!(
                "The archive is encrypted but its OS keyring key is missing. Restore the original keyring; the archive has not been changed"
            )
        }
        Err(error) => {
            return Err(keyring_error(error)).context("Unlock your OS keyring and restart ZapFast");
        }
    };
    ensure!(
        secret.len() == 32,
        "The archive key in the OS keyring is invalid"
    );
    let new = entry_for(store, to)?;
    new.set_secret(&secret)
        .map_err(keyring_error)
        .context("Could not save the archive key in the OS keyring")?;
    // Read back before removing the only other copy.
    let saved = Zeroizing::new(
        new.get_secret()
            .map_err(keyring_error)
            .context("Could not verify the saved archive key")?,
    );
    ensure!(
        saved.as_slice() == secret.as_slice(),
        "The OS keyring did not retain the archive key"
    );
    old.delete_credential()
        .map_err(keyring_error)
        .context("Could not remove the old archive key from the OS keyring")?;
    Ok(())
}

fn key_from_entry(path: &Path, entry: &keyring_core::Entry) -> Result<Zeroizing<[u8; 32]>> {
    match entry.get_secret() {
        Ok(secret) => {
            let secret = Zeroizing::new(secret);
            // Start over moves the unreadable archive aside but leaves its
            // credential behind. Replace an invalid credential only when no
            // archive remains at this path, never while protecting an archive.
            if secret.len() != 32 && !path.try_exists()? {
                return create_key(entry);
            }
            ensure!(
                secret.len() == 32,
                "The archive key in the OS keyring is invalid"
            );
            let mut key = Zeroizing::new([0; 32]);
            key.copy_from_slice(&secret);
            Ok(key)
        }
        Err(keyring_core::Error::NoEntry) => {
            ensure!(
                plaintext(path)?,
                "The archive is encrypted but its OS keyring key is missing. Restore the original keyring; the archive has not been changed"
            );
            create_key(entry)
        }
        Err(error) => {
            Err(keyring_error(error)).context("Unlock your OS keyring and restart ZapFast")
        }
    }
}

/// How the Windows store names `ERROR_NOT_ENOUGH_MEMORY`, which `CredWriteW`
/// returns when Credential Manager holds as many credentials as it can take.
/// The store keeps its error type private, so its text is what can be matched.
const WINDOWS_CREDENTIALS_FULL: &str = "Windows error code 8";

/// A keyring error as one message.
///
/// keyring-core's text already includes the error underneath and also reports
/// it as the source, so a chain printed with `{:#}` repeated it ("Platform
/// failure: Windows error code 8: Windows error code 8"). A full Credential
/// Manager is also named for what it is, with what to do about it.
fn keyring_error(error: keyring_core::Error) -> anyhow::Error {
    match &error {
        keyring_core::Error::PlatformFailure(inner)
            if inner.to_string() == WINDOWS_CREDENTIALS_FULL =>
        {
            anyhow::anyhow!(
                "Windows Credential Manager is full. Remove entries you no longer need in Credential Manager and try again"
            )
        }
        _ => anyhow::anyhow!("{error}"),
    }
}

fn create_key(entry: &keyring_core::Entry) -> Result<Zeroizing<[u8; 32]>> {
    let mut key = Zeroizing::new([0; 32]);
    getrandom::fill(key.as_mut()).context("Could not generate an archive key")?;
    entry
        .set_secret(key.as_ref())
        .map_err(keyring_error)
        .context("Could not save the archive key in the OS keyring")?;
    // Read back before touching the only copy of the message history.
    let saved = Zeroizing::new(
        entry
            .get_secret()
            .map_err(keyring_error)
            .context("Could not verify the saved archive key")?,
    );
    ensure!(
        saved.as_slice() == key.as_ref(),
        "The OS keyring did not retain the archive key"
    );
    Ok(key)
}

fn key_literal(key: &[u8; 32]) -> Zeroizing<String> {
    use std::fmt::Write;
    let mut literal = Zeroizing::new(String::with_capacity(67));
    literal.push_str("x'");
    for byte in key {
        write!(&mut *literal, "{byte:02x}").expect("writing to a String");
    }
    literal.push('\'');
    literal
}

fn keyed(path: &Path, key: &[u8; 32]) -> Result<Connection> {
    let connection = Connection::open(path)?;
    connection.pragma_update(None, "key", &*key_literal(key))?;
    let version: String = connection
        .query_row("PRAGMA cipher_version", [], |row| row.get(0))
        .context("This build does not support encrypted archives")?;
    ensure!(
        !version.is_empty(),
        "This build does not support encrypted archives"
    );
    // PRAGMA key alone does not verify a key. Read a page before any migration.
    connection
        .query_row("SELECT count(*) FROM sqlite_master", [], |row| {
            row.get::<_, i64>(0)
        })
        .context("The archive could not be unlocked with its OS keyring key")?;
    connection.pragma_update(None, "temp_store", "MEMORY")?;
    Ok(connection)
}

fn private_file(path: &Path) -> Result<()> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)?.sync_all()?;
    Ok(())
}

pub(super) fn open(path: &Path, key: &[u8; 32]) -> Result<Connection> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let staging = path.with_extension("db.encrypting");
    if !path.exists() {
        private_file(path)?;
    } else if fs::metadata(path)?.len() > 0 && plaintext(path)? {
        // Keep the original authoritative until the exported copy is complete.
        // Switching away from WAL folds in committed pages and removes sidecars
        // before replacing the main file. An interruption leaves the original
        // usable, with at most an encrypted staging file to discard next time.
        let source = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
        let mode: String =
            source.query_row("PRAGMA journal_mode = DELETE", [], |row| row.get(0))?;
        ensure!(
            mode == "delete",
            "Close other programs using the archive before migrating it"
        );
        source.pragma_update(None, "temp_store", "MEMORY")?;
        if staging.try_exists()? {
            fs::remove_file(&staging)?;
        }
        private_file(&staging)?;
        source.execute(
            "ATTACH DATABASE ?1 AS encrypted KEY ?2",
            rusqlite::params![
                staging.to_str().context("Archive path is not UTF-8")?,
                &*key_literal(key)
            ],
        )?;
        source.query_row("SELECT sqlcipher_export('encrypted')", [], |_| Ok(()))?;
        // sqlcipher_export does not copy SQLite's application/user version.
        for pragma in ["user_version", "application_id"] {
            let value: i64 = source.pragma_query_value(None, pragma, |row| row.get(0))?;
            source.pragma_update(Some("encrypted"), pragma, value)?;
        }
        source.execute_batch("DETACH DATABASE encrypted")?;
        let verified = keyed(&staging, key)?;
        let integrity: String =
            verified.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
        ensure!(
            integrity == "ok",
            "The encrypted archive failed its integrity check"
        );
        drop(verified);
        drop(source);
        // FlushFileBuffers on Windows requires a writable handle.
        fs::OpenOptions::new()
            .write(true)
            .open(&staging)?
            .sync_all()?;
        fs::rename(&staging, path)
            .context("Could not replace the archive with its encrypted copy")?;
        #[cfg(unix)]
        if let Some(parent) = path.parent() {
            fs::File::open(parent)?.sync_all()?;
        }
    }
    let connection = keyed(path, key)?;
    if staging.try_exists()? {
        fs::remove_file(staging)?;
    }
    Ok(connection)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn directory() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    fn read_secret(connection: &Connection) -> String {
        connection
            .query_row("SELECT value FROM secrets", [], |row| row.get(0))
            .unwrap()
    }

    #[test]
    fn keyring_unlock_reuses_keys_and_never_replaces_a_missing_key() {
        let directory = directory();
        let path = directory.path().join("archive.db");
        let store = keyring_core::mock::Store::new().unwrap();
        let entry = store.build("zapfast-test", "archive", None).unwrap();
        let key = key_from_entry(&path, &entry).unwrap();
        assert_eq!(*key, *key_from_entry(&path, &entry).unwrap());
        let connection = open(&path, &key).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE example(value TEXT); INSERT INTO example VALUES ('fixture');",
            )
            .unwrap();
        drop(connection);
        let original = fs::read(&path).unwrap();
        entry.delete_credential().unwrap();
        assert!(
            key_from_entry(&path, &entry)
                .unwrap_err()
                .to_string()
                .contains("missing")
        );
        assert!(matches!(
            entry.get_secret(),
            Err(keyring_core::Error::NoEntry)
        ));
        assert_eq!(fs::read(&path).unwrap(), original);
        entry.set_secret(&[1; 12]).unwrap();
        assert!(
            key_from_entry(&path, &entry)
                .unwrap_err()
                .to_string()
                .contains("invalid")
        );
        let mock = entry
            .as_any()
            .downcast_ref::<keyring_core::mock::Cred>()
            .unwrap();
        mock.set_error(keyring_core::Error::PlatformFailure(Box::new(
            std::io::Error::other("locked"),
        )));
        assert!(
            key_from_entry(&path, &entry)
                .unwrap_err()
                .to_string()
                .contains("Unlock")
        );
        assert_eq!(fs::read(&path).unwrap(), original);
    }

    /// Stands in for the Windows store's private error type, whose text for
    /// code 8 is "Windows error code 8".
    #[derive(Debug)]
    struct WindowsCode(u32);

    impl std::fmt::Display for WindowsCode {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "Windows error code {}", self.0)
        }
    }

    impl std::error::Error for WindowsCode {}

    fn saving(error: keyring_core::Error) -> String {
        let error: Result<()> =
            Err(keyring_error(error)).context("Could not save the archive key in the OS keyring");
        format!("{:#}", error.unwrap_err())
    }

    #[test]
    fn a_full_credential_manager_says_what_to_do() {
        assert_eq!(
            saving(keyring_core::Error::PlatformFailure(Box::new(WindowsCode(
                8
            )))),
            "Could not save the archive key in the OS keyring: Windows Credential Manager is full. Remove entries you no longer need in Credential Manager and try again"
        );
        // Any other platform failure is reported once, as the keyring put it.
        assert_eq!(
            saving(keyring_core::Error::PlatformFailure(Box::new(WindowsCode(
                5
            )))),
            "Could not save the archive key in the OS keyring: Platform failure: Windows error code 5"
        );
        assert_eq!(
            saving(keyring_core::Error::NoStorageAccess(Box::new(WindowsCode(
                8
            )))),
            "Could not save the archive key in the OS keyring: Couldn't access platform storage: Windows error code 8"
        );
    }

    #[test]
    fn starting_over_replaces_an_invalid_key_only_after_preserving_the_archive() {
        for invalid in [vec![], vec![1; 12], vec![2; 64]] {
            let directory = directory();
            let path = directory.path().join("archive.db");
            let kept = directory.path().join("archive-unreadable.db");
            let store = keyring_core::mock::Store::new().unwrap();
            let entry = store.build("zapfast-test", "archive", None).unwrap();
            let original_key = key_from_entry(&path, &entry).unwrap();
            let connection = open(&path, &original_key).unwrap();
            connection
                .execute_batch("CREATE TABLE example(value TEXT);")
                .unwrap();
            drop(connection);
            let original = fs::read(&path).unwrap();
            entry.set_secret(&invalid).unwrap();

            assert!(key_from_entry(&path, &entry).is_err());
            assert_eq!(entry.get_secret().unwrap(), invalid);
            assert_eq!(fs::read(&path).unwrap(), original);

            // The recovery action keeps the old archive before retrying unlock.
            fs::rename(&path, &kept).unwrap();
            let key = key_from_entry(&path, &entry).unwrap();
            assert_eq!(entry.get_secret().unwrap(), key.as_slice());
            assert_eq!(*key_from_entry(&path, &entry).unwrap(), *key);
            assert!(!path.exists());
            assert_eq!(fs::read(&kept).unwrap(), original);
            assert!(open(&kept, &original_key).is_ok());
            assert!(open(&path, &key).is_ok());
        }
    }

    #[test]
    fn invalid_key_is_not_replaced_for_an_existing_plaintext_or_empty_archive() {
        let directory = directory();
        let path = directory.path().join("archive.db");
        let store = keyring_core::mock::Store::new().unwrap();
        let entry = store.build("zapfast-test", "archive", None).unwrap();
        entry.set_secret(&[]).unwrap();
        for contents in [b"".as_slice(), HEADER.as_slice()] {
            fs::write(&path, contents).unwrap();
            assert!(key_from_entry(&path, &entry).is_err());
            assert!(entry.get_secret().unwrap().is_empty());
            assert_eq!(fs::read(&path).unwrap(), contents);
        }
    }

    #[test]
    fn encrypted_database_and_wal_reject_missing_or_wrong_keys() {
        let directory = directory();
        let path = directory.path().join("archive.db");
        let connection = open(&path, &[7; 32]).unwrap();
        connection.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE secrets (value TEXT); INSERT INTO secrets VALUES ('private archive marker');").unwrap();
        for file in [&path, &path.with_extension("db-wal")] {
            let bytes = fs::read(file).unwrap();
            assert!(
                !bytes
                    .windows(22)
                    .any(|bytes| bytes == b"private archive marker")
            );
        }
        assert!(!plaintext(&path).unwrap());
        assert!(open(&path, &[8; 32]).is_err());
        assert!(
            Connection::open(&path)
                .unwrap()
                .query_row("SELECT count(*) FROM sqlite_master", [], |row| row
                    .get::<_, i64>(0))
                .is_err()
        );
        drop(connection);
        assert_eq!(
            read_secret(&open(&path, &[7; 32]).unwrap()),
            "private archive marker"
        );
    }

    #[test]
    fn migration_preserves_wal_data_schema_and_version() {
        let directory = directory();
        let path = directory.path().join("archive.db");
        let source = Connection::open(&path).unwrap();
        source.execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0; PRAGMA user_version=9; PRAGMA application_id=42; CREATE TABLE secrets (value TEXT); CREATE INDEX secret_values ON secrets(value); INSERT INTO secrets VALUES ('from the phone');").unwrap();
        // Leave the committed WAL on disk as after an interrupted old process.
        source
            .set_db_config(
                rusqlite::config::DbConfig::SQLITE_DBCONFIG_NO_CKPT_ON_CLOSE,
                true,
            )
            .unwrap();
        drop(source);
        assert!(path.with_extension("db-wal").exists());
        fs::write(path.with_extension("db.encrypting"), b"interrupted export").unwrap();
        let encrypted = open(&path, &[9; 32]).unwrap();
        assert_eq!(read_secret(&encrypted), "from the phone");
        assert_eq!(
            encrypted
                .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
                .unwrap(),
            9
        );
        assert_eq!(
            encrypted
                .pragma_query_value(None, "application_id", |row| row.get::<_, i64>(0))
                .unwrap(),
            42
        );
        assert_eq!(
            encrypted
                .query_row(
                    "SELECT count(*) FROM sqlite_master WHERE name='secret_values'",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            1
        );
        assert!(!plaintext(&path).unwrap());
        assert!(!path.with_extension("db.encrypting").exists());
        assert!(!path.with_extension("db-wal").exists());
    }

    #[test]
    fn a_failed_migration_leaves_the_original_readable() {
        let directory = directory();
        let path = directory.path().join("archive.db");
        let source = Connection::open(&path).unwrap();
        source
            .execute_batch(
                "CREATE TABLE secrets(value TEXT); INSERT INTO secrets VALUES ('keep me');",
            )
            .unwrap();
        drop(source);
        // A blocked staging path simulates a filesystem failure before replace.
        fs::create_dir(path.with_extension("db.encrypting")).unwrap();
        assert!(open(&path, &[9; 32]).is_err());
        assert!(plaintext(&path).unwrap());
        assert_eq!(read_secret(&Connection::open(&path).unwrap()), "keep me");
        fs::remove_dir(path.with_extension("db.encrypting")).unwrap();
        assert_eq!(read_secret(&open(&path, &[9; 32]).unwrap()), "keep me");
    }

    #[test]
    fn a_plaintext_archive_has_no_key_to_move() {
        let directory = directory();
        let from = directory.path().join("state/archive.db");
        let to = directory.path().join("state/accounts/1/archive.db");
        fs::create_dir_all(from.parent().unwrap()).unwrap();
        fs::create_dir_all(to.parent().unwrap()).unwrap();
        for bytes in [b"".as_slice(), b"short".as_slice(), HEADER.as_slice()] {
            fs::write(&from, bytes).unwrap();
            move_key(&from, &to).unwrap();
            assert!(!to.exists());
        }
    }

    #[test]
    fn moving_an_encrypted_archives_key_keeps_it_readable_at_the_new_path() {
        let directory = directory();
        let store = keyring_core::mock::Store::new().unwrap();
        let old_path = directory.path().join("state/archive.db");
        let new_path = directory.path().join("state/accounts/1/archive.db");
        fs::create_dir_all(old_path.parent().unwrap()).unwrap();
        fs::create_dir_all(new_path.parent().unwrap()).unwrap();
        let old_entry = entry_for(&*store, &old_path).unwrap();
        let key = key_from_entry(&old_path, &old_entry).unwrap();
        let connection = open(&old_path, &key).unwrap();
        connection
            .execute_batch("CREATE TABLE secrets(value TEXT); INSERT INTO secrets VALUES ('kept');")
            .unwrap();
        drop(connection);
        fs::rename(&old_path, &new_path).unwrap();
        move_key_in(&*store, &old_path, &new_path).unwrap();
        assert!(matches!(
            old_entry.get_secret(),
            Err(keyring_core::Error::NoEntry)
        ));
        let moved = key_from_entry(&new_path, &entry_for(&*store, &new_path).unwrap()).unwrap();
        assert_eq!(*moved, *key);
        assert_eq!(read_secret(&open(&new_path, &key).unwrap()), "kept");
    }

    #[test]
    fn moving_an_encrypted_archive_without_its_key_stops_before_touching_it() {
        let directory = directory();
        let store = keyring_core::mock::Store::new().unwrap();
        let old_path = directory.path().join("state/archive.db");
        let new_path = directory.path().join("state/accounts/1/archive.db");
        fs::create_dir_all(old_path.parent().unwrap()).unwrap();
        fs::create_dir_all(new_path.parent().unwrap()).unwrap();
        let entry = entry_for(&*store, &old_path).unwrap();
        let key = key_from_entry(&old_path, &entry).unwrap();
        let connection = open(&old_path, &key).unwrap();
        connection
            .execute_batch("CREATE TABLE secrets(value TEXT); INSERT INTO secrets VALUES ('kept');")
            .unwrap();
        drop(connection);
        entry.delete_credential().unwrap();
        let error = move_key_in(&*store, &old_path, &new_path).unwrap_err();
        assert!(error.to_string().contains("missing"));
        assert!(!new_path.exists());
        assert!(open(&old_path, &key).is_ok());
    }
}
