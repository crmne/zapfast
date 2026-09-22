//! Compatibility for returning to ZapFast 0.14 after the library's migration.
//!
//! The protocol library owns the device schema and all message-secret access.
//! Its new migration drops an unused column that 0.14 still writes. Retain that
//! column after migration, without reading or rewriting any stored secrets.

use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::{Connection, OpenFlags};
use whatsapp_rust::store::SqliteStore;

pub(super) async fn open(path: &Path) -> Result<SqliteStore> {
    let store = SqliteStore::new(&path.to_string_lossy()).await;
    let path = path.to_owned();
    // Also repair after a later migration fails, before returning its error.
    tokio::task::spawn_blocking(move || preserve_legacy_column(&path))
        .await
        .context("Device store compatibility task failed")??;
    store.context("Could not open the device store")
}

fn preserve_legacy_column(path: &Path) -> Result<()> {
    if !path.try_exists()? {
        return Ok(());
    }
    let mut connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    connection.busy_timeout(std::time::Duration::from_secs(5))?;
    let transaction =
        connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let columns = transaction
        .prepare("PRAGMA table_info(msg_secrets)")?
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if !columns.is_empty() && !columns.iter().any(|column| column == "created_at") {
        transaction.execute_batch(
            "ALTER TABLE msg_secrets ADD COLUMN created_at INTEGER NOT NULL DEFAULT 0",
        )?;
    }
    transaction.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn library_migration_keeps_legacy_writes_working() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("session.db");
        let store = open(&path).await.unwrap();
        let connection = Connection::open(&path).unwrap();
        // This is the column set used by the old library. All values are fixtures.
        connection.execute(
            "INSERT INTO msg_secrets (chat, sender, msg_id, secret, device_id, created_at, message_ts, expires_at)
             VALUES ('fixture', 'fixture', 'fixture', x'00', 1, 1, 1, 9223372036854775807)", [],
        ).unwrap();
        drop(connection);
        drop(store);
        let _store = open(&path).await.unwrap();
        let connection = Connection::open(&path).unwrap();
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM msg_secrets", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
    }
}
