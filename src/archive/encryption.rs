//! SQLCipher archive storage and automatic OS-keyring unlock.

use std::{fs, io::Read, path::Path};

use anyhow::{Context, Result, ensure};
use hkdf::Hkdf;
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
    file.read_exact(&mut header)?;
    Ok(&header == HEADER)
}

pub(super) fn key_for(path: &Path) -> Result<Zeroizing<[u8; 32]>> {
    let parent = path.parent().context("Archive has no parent directory")?;
    fs::create_dir_all(parent)?;
    let store = platform_store()?;
    let entry = store
        .build("rocks.zapfast.ZapFast", &identity_for(parent)?, None)
        .map_err(keyring_error)
        .context("The OS keyring could not open ZapFast's archive key")?;
    key_from_entry(path, &entry)
}

/// Copies the keyring entry when an archive moves to another folder.
pub fn copy_archive_key(old_archive: &Path, new_archive: &Path) -> Result<()> {
    if plaintext(old_archive)? {
        return Ok(());
    }
    copy_archive_key_in(old_archive, new_archive, &*platform_store()?)
}

fn copy_archive_key_in(
    old_archive: &Path,
    new_archive: &Path,
    store: &dyn CredentialStoreApi,
) -> Result<()> {
    let old_parent = old_archive
        .parent()
        .context("Archive has no parent directory")?;
    let new_parent = new_archive
        .parent()
        .context("Archive has no parent directory")?;
    if !old_parent.exists() {
        return Ok(());
    }
    fs::create_dir_all(new_parent)?;
    if old_parent.canonicalize()? == new_parent.canonicalize()? {
        return Ok(());
    }
    let old_entry = store
        .build("rocks.zapfast.ZapFast", &identity_for(old_parent)?, None)
        .map_err(keyring_error)
        .context("The OS keyring could not open ZapFast's archive key")?;
    let secret = match old_entry.get_secret() {
        Ok(secret) => {
            ensure!(
                !secret.is_empty(),
                "The archive key in the OS keyring is invalid"
            );
            secret
        }
        Err(keyring_core::Error::NoEntry) => {
            ensure!(
                plaintext(old_archive)?,
                "The archive is encrypted but its OS keyring key is missing. Restore the original keyring; the archive has not been changed"
            );
            return Ok(());
        }
        Err(error) => {
            return Err(keyring_error(error)).context("Unlock your OS keyring and restart ZapFast");
        }
    };
    let new_entry = store
        .build("rocks.zapfast.ZapFast", &identity_for(new_parent)?, None)
        .map_err(keyring_error)
        .context("The OS keyring could not open ZapFast's archive key")?;
    new_entry
        .set_secret(&secret)
        .map_err(keyring_error)
        .context("Could not save the archive key in the OS keyring")?;
    // Read back before the only copy of the history moves under this key.
    let saved = Zeroizing::new(
        new_entry
            .get_secret()
            .map_err(keyring_error)
            .context("Could not verify the copied archive key")?,
    );
    ensure!(
        saved.as_slice() == secret.as_slice(),
        "The OS keyring did not retain the copied archive key"
    );
    Ok(())
}

/// Deletes the keyring entry of an archive folder that is gone for good,
/// such as a removed account's. Never call it while the archive exists.
pub fn forget_archive_key(identity: &str) -> Result<()> {
    let entry = platform_store()?
        .build("rocks.zapfast.ZapFast", identity, None)
        .map_err(keyring_error)?;
    match entry.delete_credential() {
        Ok(()) | Err(keyring_core::Error::NoEntry) => Ok(()),
        Err(error) => Err(keyring_error(error)),
    }
}

/// The keyring identity of the archive at `path`, while its folder exists.
pub fn archive_key_identity(path: &Path) -> Result<String> {
    identity_for(path.parent().context("Archive has no parent directory")?)
}

fn identity_for(parent: &Path) -> Result<String> {
    // Separate profiles must not overwrite each other's keys. The credential
    // label contains a digest, never a user path, phone number or message data.
    let digest = Sha256::digest(parent.canonicalize()?.as_os_str().as_encoded_bytes());
    Ok(format!(
        "archive-{}",
        digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    ))
}

fn platform_store() -> Result<std::sync::Arc<dyn CredentialStoreApi>> {
    #[cfg(target_os = "linux")]
    let store = zbus_secret_service_keyring_store::Store::new();
    #[cfg(target_os = "macos")]
    let store = apple_native_keyring_store::keychain::Store::new();
    #[cfg(windows)]
    let store = windows_native_keyring_store::Store::new();
    Ok(store
        .map_err(keyring_error)
        .context("Unlock your OS keyring and restart ZapFast")?
        as std::sync::Arc<dyn CredentialStoreApi>)
}

fn key_from_entry(path: &Path, entry: &keyring_core::Entry) -> Result<Zeroizing<[u8; 32]>> {
    let (derived, raw) = resolve_entry_keys(path, entry)?;

    if path.exists() && fs::metadata(path)?.len() > 0 && !plaintext(path)? {
        ensure_migrated_to_machine_bound(path, &derived, &raw)?;
    }

    Ok(derived)
}

fn resolve_entry_keys(
    path: &Path,
    entry: &keyring_core::Entry,
) -> Result<(Zeroizing<[u8; 32]>, Zeroizing<[u8; 32]>)> {
    match entry.get_secret() {
        Ok(secret) => {
            if secret.is_empty() && !path.try_exists()? {
                return create_key(entry);
            }
            let fp = crate::platform::machine_id::hardware_fingerprint();
            let raw_key = if secret.len() == 32 {
                let mut raw = Zeroizing::new([0u8; 32]);
                raw.copy_from_slice(&secret);
                #[cfg(windows)]
                if let Ok(protected) = dpapi::protect(&secret, &fp) {
                    let _ = entry.set_secret(&protected);
                }
                raw
            } else {
                #[cfg(windows)]
                {
                    dpapi::unprotect(&secret, &fp).context("The archive key in the OS keyring is invalid")?
                }
                #[cfg(not(windows))]
                {
                    ensure!(
                        secret.len() == 32,
                        "The archive key in the OS keyring is invalid"
                    );
                    let mut raw = Zeroizing::new([0u8; 32]);
                    raw.copy_from_slice(&secret);
                    raw
                }
            };
            let derived = derive_machine_bound_key(&raw_key);
            Ok((derived, raw_key))
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

pub(super) fn derive_machine_bound_key(raw_key: &[u8; 32]) -> Zeroizing<[u8; 32]> {
    let fp = crate::platform::machine_id::hardware_fingerprint();
    let hk = Hkdf::<Sha256>::new(Some(&fp), raw_key);
    let mut derived = Zeroizing::new([0u8; 32]);
    hk.expand(b"zapfast-machine-bound-key-v1", derived.as_mut())
        .expect("32 bytes is valid length for HKDF-SHA256");
    derived
}

fn ensure_migrated_to_machine_bound(
    path: &Path,
    derived: &[u8; 32],
    raw: &[u8; 32],
) -> Result<()> {
    if keyed(path, derived).is_ok() {
        return Ok(());
    }

    if let Ok(connection) = keyed(path, raw) {
        connection.pragma_update(None, "rekey", &*key_literal(derived))?;
        connection.query_row("SELECT count(*) FROM sqlite_master", [], |row| {
            row.get::<_, i64>(0)
        })?;
        return Ok(());
    }

    anyhow::bail!("The archive could not be unlocked with its machine-bound key");
}

/// How the Windows store names `ERROR_NOT_ENOUGH_MEMORY`, which `CredWriteW`
/// returns when Credential Manager holds as many credentials as it can take.
/// The store keeps its error type private, so its text is what can be matched.
const WINDOWS_CREDENTIALS_FULL: &str = "Windows error code 8";

/// A keyring error as one message.
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

fn create_key(entry: &keyring_core::Entry) -> Result<(Zeroizing<[u8; 32]>, Zeroizing<[u8; 32]>)> {
    let mut raw_key = Zeroizing::new([0; 32]);
    getrandom::fill(raw_key.as_mut()).context("Could not generate an archive key")?;

    let fp = crate::platform::machine_id::hardware_fingerprint();
    #[cfg(windows)]
    let stored_bytes = dpapi::protect(raw_key.as_ref(), &fp).unwrap_or_else(|_| raw_key.as_ref().to_vec());
    #[cfg(not(windows))]
    let stored_bytes = raw_key.as_ref().to_vec();

    entry
        .set_secret(&stored_bytes)
        .map_err(keyring_error)
        .context("Could not save the archive key in the OS keyring")?;

    let saved = entry
        .get_secret()
        .map_err(keyring_error)
        .context("Could not verify the saved archive key")?;
    ensure!(
        saved == stored_bytes,
        "The OS keyring did not retain the archive key"
    );

    let derived = derive_machine_bound_key(&raw_key);
    Ok((derived, raw_key))
}

#[cfg(windows)]
mod dpapi {
    use super::*;

    #[repr(C)]
    struct DATA_BLOB {
        cbData: u32,
        pbData: *mut u8,
    }

    const CRYPTPROTECT_UI_FORBIDDEN: u32 = 0x1;
    const CRYPTPROTECT_LOCAL_MACHINE: u32 = 0x4;

    #[link(name = "crypt32")]
    unsafe extern "system" {
        fn CryptProtectData(
            pDataIn: *const DATA_BLOB,
            szDataDescr: *const u16,
            pOptionalEntropy: *const DATA_BLOB,
            pvReserved: *mut std::ffi::c_void,
            pPromptStruct: *mut std::ffi::c_void,
            dwFlags: u32,
            pDataOut: *mut DATA_BLOB,
        ) -> i32;

        fn CryptUnprotectData(
            pDataIn: *const DATA_BLOB,
            ppszDataDescr: *mut *mut u16,
            pOptionalEntropy: *const DATA_BLOB,
            pvReserved: *mut std::ffi::c_void,
            pPromptStruct: *mut std::ffi::c_void,
            dwFlags: u32,
            pDataOut: *mut DATA_BLOB,
        ) -> i32;
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn LocalFree(hMem: *mut std::ffi::c_void) -> *mut std::ffi::c_void;
    }

    pub fn protect(data: &[u8], entropy: &[u8]) -> Result<Vec<u8>> {
        let in_blob = DATA_BLOB {
            cbData: data.len() as u32,
            pbData: data.as_ptr() as *mut u8,
        };
        let entropy_blob = DATA_BLOB {
            cbData: entropy.len() as u32,
            pbData: entropy.as_ptr() as *mut u8,
        };
        let mut out_blob = DATA_BLOB {
            cbData: 0,
            pbData: std::ptr::null_mut(),
        };

        let status = unsafe {
            CryptProtectData(
                &in_blob,
                std::ptr::null(),
                &entropy_blob,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                CRYPTPROTECT_LOCAL_MACHINE | CRYPTPROTECT_UI_FORBIDDEN,
                &mut out_blob,
            )
        };

        if status == 0 || out_blob.pbData.is_null() {
            anyhow::bail!("Windows DPAPI CryptProtectData failed");
        }

        let slice = unsafe {
            std::slice::from_raw_parts(out_blob.pbData, out_blob.cbData as usize)
        };
        let result = slice.to_vec();
        unsafe { LocalFree(out_blob.pbData.cast()) };
        Ok(result)
    }

    pub fn unprotect(data: &[u8], entropy: &[u8]) -> Result<Zeroizing<[u8; 32]>> {
        let in_blob = DATA_BLOB {
            cbData: data.len() as u32,
            pbData: data.as_ptr() as *mut u8,
        };
        let entropy_blob = DATA_BLOB {
            cbData: entropy.len() as u32,
            pbData: entropy.as_ptr() as *mut u8,
        };
        let mut out_blob = DATA_BLOB {
            cbData: 0,
            pbData: std::ptr::null_mut(),
        };

        let status = unsafe {
            CryptUnprotectData(
                &in_blob,
                std::ptr::null_mut(),
                &entropy_blob,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out_blob,
            )
        };

        if status == 0 || out_blob.pbData.is_null() {
            anyhow::bail!("Windows DPAPI unprotect failed: archive is locked to another machine or user");
        }

        let slice = unsafe {
            std::slice::from_raw_parts(out_blob.pbData, out_blob.cbData as usize)
        };
        ensure!(slice.len() == 32, "The archive key in the OS keyring is invalid");

        let mut key = Zeroizing::new([0u8; 32]);
        key.copy_from_slice(slice);
        unsafe { LocalFree(out_blob.pbData.cast()) };
        Ok(key)
    }
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
    unlock(Connection::open(path)?, key)
}

/// Opens an existing archive for an export without migrating or creating any data.
pub(super) fn read_only(path: &Path, key: &[u8; 32]) -> Result<Connection> {
    unlock(
        Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?,
        key,
    )
}

/// Applies the supplied SQLCipher key and verifies a page before allowing archive access.
fn unlock(connection: Connection, key: &[u8; 32]) -> Result<Connection> {
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
}
