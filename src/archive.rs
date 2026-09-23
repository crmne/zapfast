//! SQLite archive of chats, messages, contacts, and stickers.
//!
//! Each message keeps its raw protobuf because attachment download keys may be
//! needed long after history sync.

use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension, params};

use crate::model::{Chat, ChatKind, Contact, Content, Delivery, LastMessage, Message};

mod drafts;
mod encryption;
mod labels;
pub use labels::{DEFAULT_COLOR, LABEL_LIMIT, NAME_LIMIT};
mod polls;
mod receipts;
mod stickers;
pub use polls::PollVote;
pub use stickers::FavoriteSticker;

/// Outcome of deleting or clearing a chat.
#[derive(Clone, Debug, Default)]
pub struct Removed {
    /// Whether a chat row was present before the change.
    pub existed: bool,
    /// Attachment paths the removed messages pointed at.
    pub media: Vec<PathBuf>,
}

/// Recent phone sticker metadata, last-used time, and optional local file.
#[derive(Clone, Debug)]
pub struct PhoneSticker {
    pub hash: String,
    pub raw: Vec<u8>,
    pub last_used: i64,
    pub path: Option<std::path::PathBuf>,
}

/// Downloaded chat sticker with its last-seen time and source message.
#[derive(Clone, Debug)]
pub struct ArchivedSticker {
    pub last_used: i64,
    pub path: std::path::PathBuf,
    pub raw: Option<Vec<u8>>,
}

pub struct Archive {
    connection: Connection,
}

pub type Result<T> = std::result::Result<T, rusqlite::Error>;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS chats (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    kind TEXT NOT NULL,
    last_activity INTEGER NOT NULL DEFAULT 0,
    unread INTEGER NOT NULL DEFAULT 0,
    archived INTEGER NOT NULL DEFAULT 0,
    pinned INTEGER NOT NULL DEFAULT 0,
    muted_until INTEGER,
    locked INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS messages (
    chat TEXT NOT NULL,
    id TEXT NOT NULL,
    sender TEXT NOT NULL,
    sender_name TEXT,
    from_me INTEGER NOT NULL,
    timestamp INTEGER NOT NULL,
    content TEXT NOT NULL,
    status INTEGER NOT NULL DEFAULT 0,
    quoted TEXT,
    reactions TEXT NOT NULL DEFAULT '[]',
    edited INTEGER NOT NULL DEFAULT 0,
    raw BLOB,
    PRIMARY KEY (chat, id)
);
CREATE INDEX IF NOT EXISTS messages_by_time ON messages (chat, timestamp);
CREATE TABLE IF NOT EXISTS contacts (
    id TEXT PRIMARY KEY,
    full_name TEXT,
    push_name TEXT
);
CREATE TABLE IF NOT EXISTS meta (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS chat_removals (
    chat TEXT PRIMARY KEY,
    through INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS lids (
    lid TEXT PRIMARY KEY,
    pn TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS stickers (
    hash TEXT PRIMARY KEY,
    raw BLOB NOT NULL,
    last_used INTEGER NOT NULL DEFAULT 0,
    weight REAL NOT NULL DEFAULT 0,
    path TEXT
);
CREATE TABLE IF NOT EXISTS group_receipts (
    chat TEXT NOT NULL,
    id TEXT NOT NULL,
    recipient TEXT NOT NULL,
    expected INTEGER NOT NULL DEFAULT 0,
    status INTEGER NOT NULL DEFAULT 0,
    delivered_at INTEGER,
    read_at INTEGER,
    played_at INTEGER,
    PRIMARY KEY (chat, id, recipient)
);
CREATE TRIGGER IF NOT EXISTS delete_group_receipts AFTER DELETE ON messages BEGIN
    DELETE FROM group_receipts WHERE chat = OLD.chat AND id = OLD.id;
END;
";

const CHAT_COLUMNS: &str =
    "c.id, c.name, c.kind, c.last_activity, c.unread, c.archived, c.pinned, c.muted_until,
                    m.from_me, m.sender_name, m.content, m.status, m.sender, c.participants, c.read_only,
                    c.pinned_at, c.ephemeral_expiration, c.locked, c.group_subject_known,
                    c.notification_sound";

/// Adds columns introduced after the initial schema when missing.
const MIGRATIONS: &[(&str, &str, &str)] = &[
    ("messages", "thumbnail", "BLOB"),
    ("messages", "mentions", "TEXT NOT NULL DEFAULT '[]'"),
    ("chats", "participants", "TEXT NOT NULL DEFAULT '[]'"),
    ("chats", "read_only", "INTEGER NOT NULL DEFAULT 0"),
    ("messages", "forwarded", "INTEGER NOT NULL DEFAULT 0"),
    ("messages", "delivered_at", "INTEGER"),
    ("messages", "read_at", "INTEGER"),
    ("chats", "read_through", "INTEGER"),
    ("chats", "pending_read", "INTEGER"),
    ("chats", "ephemeral_expiration", "INTEGER"),
    ("chats", "ephemeral_setting_timestamp", "INTEGER"),
    ("chats", "pinned_at", "INTEGER NOT NULL DEFAULT 0"),
    ("chats", "pin_updated_at", "INTEGER"),
    ("chats", "mute_updated_at", "INTEGER"),
    ("chats", "locked", "INTEGER NOT NULL DEFAULT 0"),
    ("chats", "lock_updated_at", "INTEGER"),
    ("chats", "archive_updated_at", "INTEGER"),
    ("chats", "notification_sound", "TEXT"),
    ("chats", "group_subject_known", "INTEGER NOT NULL DEFAULT 0"),
];
const CHAT_JOIN: &str = "FROM chats c
             LEFT JOIN messages m ON m.chat = c.id AND m.rowid = (
                 SELECT rowid FROM messages WHERE chat = c.id ORDER BY timestamp DESC, rowid DESC LIMIT 1
             )";

fn chat_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Chat> {
    let content: Option<String> = row.get(10)?;
    let last = match content {
        Some(content) => {
            let content: Content = serde_json::from_str(&content).unwrap_or(Content::Unsupported {
                what: "unreadable".into(),
            });
            Some(LastMessage {
                from_me: row.get(8)?,
                sender: row.get::<_, Option<String>>(12)?.unwrap_or_default(),
                sender_name: row.get(9)?,
                summary: content.summary(),
                status: status_from_rank(row.get(11)?),
            })
        }
        None => None,
    };
    let kind: String = row.get(2)?;
    let participants: String = row.get(13)?;
    Ok(Chat {
        id: row.get(0)?,
        name: row.get(1)?,
        group_subject_known: row.get(18)?,
        kind: kind_from_name(&kind),
        last_activity: row.get(3)?,
        unread: row.get(4)?,
        archived: row.get(5)?,
        pinned: row.get(6)?,
        pinned_at: row.get(15)?,
        muted_until: row.get(7)?,
        locked: row.get(17)?,
        last,
        participants: serde_json::from_str(&participants).unwrap_or_default(),
        read_only: row.get(14)?,
        labels: Vec::new(),
        ephemeral_expiration: row
            .get::<_, Option<u32>>(16)?
            .filter(|expiration| *expiration != 0),
        notification_sound: row
            .get::<_, Option<String>>(19)?
            .and_then(|sound| serde_json::from_str(&sound).ok()),
    })
}

fn status_rank(status: Delivery) -> i64 {
    match status {
        Delivery::None => 0,
        Delivery::Pending => 1,
        Delivery::Sent => 2,
        Delivery::Delivered => 3,
        Delivery::Read => 4,
        Delivery::Played => 5,
        Delivery::Failed => 6,
    }
}

/// Timestamp column for a remembered delivery stage.
fn stamp_column(status: Delivery) -> Option<&'static str> {
    match status {
        Delivery::Delivered => Some("delivered_at"),
        Delivery::Read | Delivery::Played => Some("read_at"),
        _ => None,
    }
}

fn status_from_rank(rank: i64) -> Delivery {
    match rank {
        1 => Delivery::Pending,
        2 => Delivery::Sent,
        3 => Delivery::Delivered,
        4 => Delivery::Read,
        5 => Delivery::Played,
        6 => Delivery::Failed,
        _ => Delivery::None,
    }
}

fn kind_name(kind: ChatKind) -> &'static str {
    match kind {
        ChatKind::Direct => "direct",
        ChatKind::Group => "group",
        ChatKind::Broadcast => "broadcast",
    }
}

fn kind_from_name(name: &str) -> ChatKind {
    match name {
        "group" => ChatKind::Group,
        "broadcast" => ChatKind::Broadcast,
        _ => ChatKind::Direct,
    }
}

impl Archive {
    /// Unlocks the on-disk archive with its OS keyring key, migrating plaintext
    /// archives before their first encrypted use. Never falls back to plaintext.
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        let key = encryption::key_for(path)?;
        Self::open_with_key(path, &key)
    }

    fn open_with_key(path: &Path, key: &[u8; 32]) -> anyhow::Result<Self> {
        Ok(Self::prepare(encryption::open(path, key)?)?)
    }

    pub fn in_memory() -> Result<Self> {
        Self::prepare(Connection::open_in_memory()?)
    }

    fn prepare(connection: Connection) -> Result<Self> {
        connection.execute_batch("PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL;")?;
        connection.execute_batch(SCHEMA)?;
        connection.execute_batch(labels::SCHEMA)?;
        connection.execute_batch(polls::SCHEMA)?;
        connection.execute_batch(drafts::SCHEMA)?;
        connection.execute_batch(stickers::SCHEMA)?;
        for (table, column, definition) in MIGRATIONS {
            let exists = connection
                .prepare(&format!("PRAGMA table_info({table})"))?
                .query_map([], |row| row.get::<_, String>(1))?
                .any(|name| name.as_deref() == Ok(*column));
            if !exists {
                connection.execute_batch(&format!(
                    "ALTER TABLE {table} ADD COLUMN {column} {definition}"
                ))?;
            }
        }
        Self::prune_receipts(&connection)?;
        Ok(Self { connection })
    }

    /// Creates a chat or replaces a phone-number title with a better name.
    pub fn upsert_chat(&self, chat: &Chat) -> Result<()> {
        self.connection.execute(
            "INSERT INTO chats (id, name, kind, last_activity, unread, archived, pinned, muted_until, pinned_at, group_subject_known)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                group_subject_known = excluded.group_subject_known,
                last_activity = MAX(last_activity, excluded.last_activity),
                archived = CASE WHEN archive_updated_at IS NULL THEN excluded.archived ELSE archived END,
                pinned = CASE WHEN pin_updated_at IS NULL THEN excluded.pinned ELSE pinned END,
                pinned_at = CASE WHEN pin_updated_at IS NULL THEN excluded.pinned_at ELSE pinned_at END,
                muted_until = CASE WHEN mute_updated_at IS NULL THEN excluded.muted_until ELSE muted_until END",
            params![
                chat.id,
                chat.name,
                kind_name(chat.kind),
                chat.last_activity,
                chat.unread,
                chat.archived,
                chat.pinned,
                chat.muted_until,
                chat.pinned_at,
                chat.group_subject_known,
            ],
        )?;
        Ok(())
    }

    /// Inserts a chat row only when missing.
    pub fn ensure_chat(&self, id: &str, name: &str) -> Result<()> {
        self.connection.execute(
            "INSERT OR IGNORE INTO chats (id, name, kind) VALUES (?1, ?2, ?3)",
            params![id, name, kind_name(ChatKind::from_id(id))],
        )?;
        Ok(())
    }

    /// Updates group subject, members, and posting permission.
    pub fn set_group_info(
        &self,
        id: &str,
        name: Option<&str>,
        participants: &[String],
        read_only: bool,
    ) -> Result<()> {
        // Incomplete metadata must not erase a subject learned from history.
        // Keep unresolved subjects eligible for another metadata request.
        let name = name.filter(|name| !name.trim().is_empty());
        self.connection.execute(
            "UPDATE chats SET name = COALESCE(?2, name), participants = ?3, read_only = ?4,
                group_subject_known = CASE WHEN ?2 IS NOT NULL THEN 1 ELSE group_subject_known END
             WHERE id = ?1",
            params![
                id,
                name,
                serde_json::to_string(participants).unwrap_or_else(|_| "[]".into()),
                read_only
            ],
        )?;
        Ok(())
    }

    pub fn rename_chat(&self, id: &str, name: &str) -> Result<()> {
        self.connection.execute(
            "UPDATE chats SET name = ?2, group_subject_known = 1 WHERE id = ?1",
            params![id, name],
        )?;
        Ok(())
    }

    pub fn set_archived(&self, id: &str, archived: bool) -> Result<()> {
        self.set_archived_at(id, archived, jiff::Timestamp::now().as_millisecond())
    }

    /// Apply archive state in timestamp order, like pin and mute, so history
    /// arriving later cannot undo an archive change received from the phone.
    pub fn set_archived_at(&self, id: &str, archived: bool, timestamp: i64) -> Result<()> {
        self.connection.execute(
            "UPDATE chats SET archived = ?2, archive_updated_at = ?3 WHERE id = ?1
                AND (archive_updated_at IS NULL OR archive_updated_at <= ?3)",
            params![id, archived, timestamp],
        )?;
        Ok(())
    }

    /// A chat's own notification sound; `None` follows Settings.
    pub fn set_notification_sound(
        &self,
        id: &str,
        sound: Option<&crate::settings::NotificationSound>,
    ) -> Result<()> {
        let sound = sound.map(|sound| serde_json::to_string(sound).unwrap_or_default());
        self.connection.execute(
            "UPDATE chats SET notification_sound = ?2 WHERE id = ?1",
            params![id, sound],
        )?;
        Ok(())
    }

    pub fn set_pinned(&self, id: &str, pinned: bool) -> Result<()> {
        self.set_pinned_at(id, pinned, jiff::Timestamp::now().as_millisecond())
    }

    /// Apply app-state in timestamp order. A later history chunk has no state
    /// version and must not overwrite a pin/unpin already received from sync.
    pub fn set_pinned_at(&self, id: &str, pinned: bool, timestamp: i64) -> Result<()> {
        self.connection.execute(
            "UPDATE chats SET pinned = ?2, pinned_at = CASE WHEN ?2 THEN ?3 ELSE 0 END,
                pin_updated_at = ?3 WHERE id = ?1
                AND (pin_updated_at IS NULL OR pin_updated_at <= ?3)",
            params![id, pinned, timestamp],
        )?;
        Ok(())
    }

    pub fn set_muted(&self, id: &str, until: Option<i64>) -> Result<()> {
        self.set_muted_at(id, until, jiff::Timestamp::now().as_millisecond())
    }
    /// Keep mute/unmute actions across history replay, including actions that
    /// precede the initial chat snapshot and older app-state replay.
    pub fn set_muted_at(&self, id: &str, until: Option<i64>, timestamp: i64) -> Result<()> {
        self.connection.execute(
            "UPDATE chats SET muted_until = ?2, mute_updated_at = ?3 WHERE id = ?1
                AND (mute_updated_at IS NULL OR mute_updated_at <= ?3)",
            params![id, until, timestamp],
        )?;
        Ok(())
    }

    pub fn set_locked(&self, id: &str, locked: bool) -> Result<()> {
        self.set_locked_at(id, locked, jiff::Timestamp::now().as_millisecond())
    }

    /// Records history metadata only until app-state provides its version.
    pub fn set_locked_snapshot(&self, id: &str, locked: bool) -> Result<()> {
        self.connection.execute(
            "UPDATE chats SET locked = ?2 WHERE id = ?1 AND lock_updated_at IS NULL",
            params![id, locked],
        )?;
        Ok(())
    }

    /// Apply lock state in timestamp order, like pin and mute, so an old
    /// replay cannot undo a lock change just received from the phone.
    pub fn set_locked_at(&self, id: &str, locked: bool, timestamp: i64) -> Result<()> {
        self.connection.execute(
            "UPDATE chats SET locked = ?2, lock_updated_at = ?3 WHERE id = ?1
                AND (lock_updated_at IS NULL OR lock_updated_at <= ?3)",
            params![id, locked, timestamp],
        )?;
        Ok(())
    }

    /// Applies disappearing-message metadata unless a newer setting is stored.
    pub fn set_ephemeral(&self, id: &str, expiration: u32, setting_timestamp: i64) -> Result<bool> {
        Ok(self.connection.execute(
            "UPDATE chats SET ephemeral_expiration = ?2, ephemeral_setting_timestamp = ?3
             WHERE id = ?1 AND (ephemeral_setting_timestamp IS NULL OR ephemeral_setting_timestamp <= ?3)",
            params![id, expiration, setting_timestamp],
        )? > 0)
    }

    /// Returns the chat timer, including zero for an explicitly disabled timer.
    pub fn ephemeral_expiration(&self, id: &str) -> Result<Option<u32>> {
        self.connection
            .query_row(
                "SELECT ephemeral_expiration FROM chats WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .optional()
            .map(Option::flatten)
    }

    pub fn mark_read(&self, id: &str) -> Result<()> {
        self.connection.execute(
            "UPDATE chats SET unread = 0,
             read_through = MAX(COALESCE(read_through, 0), last_activity) WHERE id = ?1",
            params![id],
        )?;
        Ok(())
    }

    /// A read on another device covers messages up to its position, not newer
    /// arrivals. Keep the position across restarts and history replays.
    pub fn mark_read_through(&self, id: &str, timestamp: i64) -> Result<()> {
        self.connection.execute(
            "UPDATE chats SET read_through = MAX(COALESCE(read_through, 0), ?2),
             unread = MIN(unread, (SELECT COUNT(*) FROM messages
                 WHERE chat = ?1 AND from_me = 0
                 AND timestamp > MAX(COALESCE(read_through, 0), ?2))) WHERE id = ?1",
            params![id, timestamp],
        )?;
        Ok(())
    }

    /// A message id disambiguates rapid messages with the same second-level
    /// timestamp. A receipt for the first must leave the later messages unread.
    pub fn mark_read_to(&self, chat: &str, message: &str) -> Result<()> {
        self.connection.execute(
            "UPDATE chats SET
             read_through = MAX(COALESCE(read_through, 0),
                 (SELECT timestamp FROM messages WHERE chat = ?1 AND id = ?2)),
             unread = MIN(unread, (SELECT COUNT(*) FROM messages m
                 JOIN messages boundary ON boundary.chat = m.chat AND boundary.id = ?2
                 WHERE m.chat = ?1 AND m.from_me = 0
                 AND (m.timestamp > boundary.timestamp
                      OR (m.timestamp = boundary.timestamp AND m.rowid > boundary.rowid))))
             WHERE id = ?1 AND EXISTS(SELECT 1 FROM messages WHERE chat = ?1 AND id = ?2)",
            params![chat, message],
        )?;
        Ok(())
    }

    pub fn read_through(&self, id: &str) -> Result<Option<i64>> {
        self.connection
            .query_row(
                "SELECT read_through FROM chats WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .optional()
            .map(Option::flatten)
    }

    pub fn queue_read_sync(&self, id: &str) -> Result<()> {
        self.connection.execute(
            "UPDATE chats SET pending_read = read_through WHERE id = ?1",
            params![id],
        )?;
        Ok(())
    }

    pub fn pending_reads(&self) -> Result<Vec<(String, i64)>> {
        self.connection
            .prepare("SELECT id, pending_read FROM chats WHERE pending_read IS NOT NULL")?
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect()
    }

    pub fn finish_read_sync(&self, id: &str, through: i64) -> Result<()> {
        self.connection.execute(
            "UPDATE chats SET pending_read = NULL WHERE id = ?1 AND pending_read <= ?2",
            params![id, through],
        )?;
        Ok(())
    }

    /// Limit a phone snapshot to messages after any more recent read here.
    pub fn history_unread(&self, id: &str, unread: u32) -> Result<u32> {
        let Some(through) = self.read_through(id)? else {
            return Ok(unread);
        };
        let remaining: u32 = self.connection.query_row(
            "SELECT COUNT(*) FROM messages WHERE chat = ?1 AND from_me = 0 AND timestamp > ?2",
            params![id, through],
            |row| row.get(0),
        )?;
        Ok(unread.min(remaining))
    }

    pub fn set_unread(&self, id: &str, unread: u32) -> Result<()> {
        self.connection.execute(
            "UPDATE chats SET unread = ?2 WHERE id = ?1",
            params![id, unread],
        )?;
        Ok(())
    }

    /// Returns all chats with their latest message, newest first.
    pub fn chats(&self) -> Result<Vec<Chat>> {
        let mut statement = self.connection.prepare(&format!(
            "SELECT {CHAT_COLUMNS} {CHAT_JOIN} ORDER BY c.last_activity DESC"
        ))?;
        let rows = statement.query_map([], chat_from_row)?;
        rows.collect()
    }

    pub fn chat(&self, id: &str) -> Result<Option<Chat>> {
        let mut statement = self.connection.prepare(&format!(
            "SELECT {CHAT_COLUMNS} {CHAT_JOIN} WHERE c.id = ?1"
        ))?;
        statement.query_row(params![id], chat_from_row).optional()
    }

    pub fn bump_unread(&self, id: &str) -> Result<()> {
        self.connection.execute(
            "UPDATE chats SET unread = unread + 1 WHERE id = ?1",
            params![id],
        )?;
        Ok(())
    }

    /// Returns recent incoming message ids and senders for read receipts.
    pub fn unread_incoming(&self, chat: &str, limit: u32) -> Result<Vec<(String, String)>> {
        let mut statement = self.connection.prepare(
            "SELECT id, sender FROM messages WHERE chat = ?1 AND from_me = 0
             AND timestamp >= COALESCE((SELECT read_through FROM chats WHERE id = ?1), -1)
             ORDER BY timestamp DESC, rowid DESC LIMIT ?2",
        )?;
        let rows = statement.query_map(params![chat, i64::from(limit)], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })?;
        rows.collect()
    }

    /// Records an attachment's local path.
    pub fn set_media_path(&self, chat: &str, id: &str, path: &Path) -> Result<Option<Message>> {
        self.put_media_path(chat, id, Some(path))
    }

    /// Clears an attachment path so it can be downloaded again.
    pub fn clear_media_path(&self, chat: &str, id: &str) -> Result<Option<Message>> {
        self.put_media_path(chat, id, None)
    }

    fn put_media_path(&self, chat: &str, id: &str, path: Option<&Path>) -> Result<Option<Message>> {
        self.put_media_path_at(chat, id, None, path)
    }

    pub fn put_media_path_at(
        &self,
        chat: &str,
        id: &str,
        card: Option<usize>,
        path: Option<&Path>,
    ) -> Result<Option<Message>> {
        let Some(mut message) = self.message(chat, id)? else {
            return Ok(None);
        };
        let Some(media) = message.content.media_at_mut(card) else {
            return Ok(None);
        };
        media.path = path.map(Path::to_path_buf);
        self.set_content(chat, id, &message.content, message.edited)?;
        Ok(Some(message))
    }

    /// Returns all recorded attachment paths.
    pub fn media_paths(&self) -> Result<Vec<(String, String, std::path::PathBuf)>> {
        let mut statement = self.connection.prepare(
            "SELECT chat, id, coalesce(json_extract(content, '$.media.path'),
                 json_extract(content, '$.card.image.path')) AS path
             FROM messages WHERE path IS NOT NULL",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                std::path::PathBuf::from(row.get::<_, String>(2)?),
            ))
        })?;
        rows.collect()
    }

    /// Includes each carousel attachment separately so moves and cache cleanup
    /// never reuse one card's image for another.
    pub fn carousel_media_paths(&self) -> Result<Vec<(String, String, usize, std::path::PathBuf)>> {
        let mut statement = self.connection.prepare(
            "SELECT m.chat, m.id, c.key, json_extract(c.value, '$.image.path') AS image_path
             FROM messages m, json_each(m.content, '$.card.carousel') c WHERE image_path IS NOT NULL",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get::<_, u32>(2)? as usize,
                std::path::PathBuf::from(row.get::<_, String>(3)?),
            ))
        })?;
        rows.collect()
    }

    /// Stores a privacy id mapping and carries early mute/pin/lock sync to the
    /// canonical chat. Returns whether that chat's preferences were touched.
    pub fn put_lid(&self, lid: &str, pn: &str) -> Result<bool> {
        self.connection.execute(
            "INSERT INTO lids (lid, pn) VALUES (?1, ?2) ON CONFLICT(lid) DO UPDATE SET pn = excluded.pn",
            params![lid, pn],
        )?;
        self.merge_group_recipient(&format!("{lid}@lid"), &format!("{pn}@s.whatsapp.net"))?;
        self.connection.execute(
            "INSERT INTO chat_removals (chat, through) SELECT ?2, through FROM chat_removals WHERE chat = ?1
             ON CONFLICT(chat) DO UPDATE SET through = MAX(through, excluded.through)",
            params![format!("{lid}@lid"), format!("{pn}@s.whatsapp.net")])?;
        let changed = self.connection.execute(
            "INSERT INTO chats (id, name, kind, pinned, pinned_at, pin_updated_at,
                muted_until, mute_updated_at, locked, lock_updated_at, archived, archive_updated_at)
             SELECT ?2, ?3, 'direct', pinned, pinned_at, pin_updated_at,
                muted_until, mute_updated_at, locked, lock_updated_at, archived, archive_updated_at
                FROM chats WHERE id = ?1
                AND (pin_updated_at IS NOT NULL OR mute_updated_at IS NOT NULL
                    OR lock_updated_at IS NOT NULL OR locked OR archive_updated_at IS NOT NULL)
             ON CONFLICT(id) DO UPDATE SET
                pinned = CASE WHEN excluded.pin_updated_at >= COALESCE(pin_updated_at, -1)
                    THEN excluded.pinned ELSE pinned END,
                pinned_at = CASE WHEN excluded.pin_updated_at >= COALESCE(pin_updated_at, -1)
                    THEN excluded.pinned_at ELSE pinned_at END,
                pin_updated_at = NULLIF(MAX(COALESCE(pin_updated_at, -1), COALESCE(excluded.pin_updated_at, -1)), -1),
                muted_until = CASE WHEN excluded.mute_updated_at >= COALESCE(mute_updated_at, -1)
                    THEN excluded.muted_until ELSE muted_until END,
                mute_updated_at = NULLIF(MAX(COALESCE(mute_updated_at, -1), COALESCE(excluded.mute_updated_at, -1)), -1),
                locked = CASE WHEN excluded.lock_updated_at >= COALESCE(lock_updated_at, -1)
                    THEN excluded.locked
                    WHEN lock_updated_at IS NULL AND excluded.lock_updated_at IS NULL
                    THEN MAX(locked, excluded.locked) ELSE locked END,
                lock_updated_at = NULLIF(MAX(COALESCE(lock_updated_at, -1), COALESCE(excluded.lock_updated_at, -1)), -1),
                archived = CASE WHEN excluded.archive_updated_at >= COALESCE(archive_updated_at, -1)
                    THEN excluded.archived ELSE archived END,
                archive_updated_at = NULLIF(MAX(COALESCE(archive_updated_at, -1), COALESCE(excluded.archive_updated_at, -1)), -1)",
            params![format!("{lid}@lid"), format!("{pn}@s.whatsapp.net"), pn],
        )?;
        Ok(changed > 0)
    }

    pub fn lids(&self) -> Result<Vec<(String, String)>> {
        let mut statement = self.connection.prepare("SELECT lid, pn FROM lids")?;
        let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        rows.collect()
    }

    /// Upserts a message, preserves the furthest delivery state, and updates
    /// chat activity. `raw` contains attachment metadata.
    ///
    /// Both preservation rules run inside the UPSERT: the stored delivery state
    /// only moves forward, and a history row that arrives without reactions
    /// keeps the reactions already stored. Inserting a message is therefore one
    /// write instead of a read followed by a write.
    pub fn insert_message(&self, message: &Message, raw: Option<&[u8]>) -> Result<()> {
        self.connection.execute(
            "INSERT INTO messages (chat, id, sender, sender_name, from_me, timestamp, content, status, quoted, reactions, edited, raw, thumbnail, mentions, forwarded, delivered_at, read_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)
             ON CONFLICT(chat, id) DO UPDATE SET
                sender_name = COALESCE(excluded.sender_name, sender_name),
                content = excluded.content,
                status = CASE
                    WHEN messages.status > excluded.status AND excluded.status <> ?18
                    THEN messages.status
                    ELSE excluded.status
                END,
                quoted = COALESCE(excluded.quoted, quoted),
                reactions = CASE
                    WHEN excluded.reactions = '[]' THEN messages.reactions
                    ELSE excluded.reactions
                END,
                edited = excluded.edited,
                raw = COALESCE(excluded.raw, raw),
                thumbnail = COALESCE(excluded.thumbnail, thumbnail),
                mentions = excluded.mentions,
                forwarded = excluded.forwarded,
                delivered_at = COALESCE(delivered_at, excluded.delivered_at),
                read_at = COALESCE(read_at, excluded.read_at)",
            params![
                message.chat,
                message.id,
                message.sender,
                message.sender_name,
                message.from_me,
                message.timestamp,
                serde_json::to_string(&message.content).unwrap_or_default(),
                status_rank(message.status),
                message
                    .quoted
                    .as_ref()
                    .map(|quoted| serde_json::to_string(quoted).unwrap_or_default()),
                serde_json::to_string(&message.reactions).unwrap_or_default(),
                message.edited,
                raw,
                message.thumbnail.as_deref(),
                serde_json::to_string(&message.mentions).unwrap_or_default(),
                message.forwarded,
                message.delivered_at,
                message.read_at,
                // An explicit failure still writes over a further state.
                status_rank(Delivery::Failed),
            ],
        )?;
        self.connection.execute(
            "UPDATE chats SET last_activity = MAX(last_activity, ?2) WHERE id = ?1",
            params![message.chat, message.timestamp],
        )?;
        Ok(())
    }

    /// Returns up to `limit` messages before an optional timestamp/id boundary,
    /// in ascending order.
    pub fn messages(
        &self,
        chat: &str,
        before: Option<(i64, &str)>,
        limit: usize,
    ) -> Result<Vec<Message>> {
        let mut statement = self.connection.prepare(
            "SELECT id, sender, sender_name, from_me, timestamp, content, status, quoted, reactions, edited, thumbnail, mentions, forwarded, delivered_at, read_at
             FROM messages
             WHERE chat = ?1 AND (timestamp < ?2 OR (timestamp = ?2 AND rowid <
                 (SELECT rowid FROM messages WHERE chat = ?1 AND id = ?3)))
             ORDER BY timestamp DESC, rowid DESC
             LIMIT ?4",
        )?;
        let (before_time, before_id) = before.unwrap_or((i64::MAX, ""));
        let rows =
            statement.query_map(params![chat, before_time, before_id, limit as i64], |row| {
                let content: String = row.get(5)?;
                let quoted: Option<String> = row.get(7)?;
                let reactions: String = row.get(8)?;
                let mentions: String = row.get(11)?;
                Ok(Message {
                    id: row.get(0)?,
                    chat: chat.to_owned(),
                    sender: row.get(1)?,
                    sender_name: row.get(2)?,
                    from_me: row.get(3)?,
                    timestamp: row.get(4)?,
                    content: serde_json::from_str(&content).unwrap_or(Content::Unsupported {
                        what: "unreadable".into(),
                    }),
                    status: status_from_rank(row.get(6)?),
                    delivered_at: row.get(13)?,
                    read_at: row.get(14)?,
                    quoted: quoted.and_then(|quoted| serde_json::from_str(&quoted).ok()),
                    reactions: serde_json::from_str(&reactions).unwrap_or_default(),
                    edited: row.get(9)?,
                    mentions: serde_json::from_str(&mentions).unwrap_or_default(),
                    forwarded: row.get(12)?,
                    thumbnail: row.get(10)?,
                })
            })?;
        let mut messages: Vec<Message> = rows.collect::<Result<_>>()?;
        messages.reverse();
        Ok(messages)
    }

    /// Message ids in one chat whose visible text matches, oldest first so
    /// next and previous walk forward in time. Same fields as the global
    /// search, scoped to a single chat.
    pub fn search_chat_messages(
        &self,
        chat: &str,
        needle: &str,
        limit: usize,
    ) -> Result<Vec<String>> {
        let pattern = format!(
            "%{}%",
            needle
                .to_lowercase()
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_")
        );
        let mut statement = self.connection.prepare(
            "SELECT id
             FROM messages
             WHERE chat = ?1 AND json_valid(content) AND lower(
                     coalesce(json_extract(content, '$.text'), '') || char(10) ||
                     coalesce(json_extract(content, '$.caption'), '') || char(10) ||
                     coalesce(json_extract(content, '$.file_name'), '') || char(10) ||
                     coalesce(json_extract(content, '$.question'), '') || char(10) ||
                     coalesce(json_extract(content, '$.display_name'), '') || char(10) ||
                     coalesce(json_extract(content, '$.name'), '')
                 ) LIKE ?2 ESCAPE '\\'
             ORDER BY timestamp ASC, rowid ASC
             LIMIT ?3",
        )?;
        let rows = statement.query_map(params![chat, pattern, limit as i64], |row| row.get(0))?;
        rows.collect()
    }

    /// Searches visible message text, filenames, polls, contacts, and places.
    /// ASCII matching is case-insensitive; other text follows SQLite behavior.
    pub fn search_messages(&self, needle: &str, limit: usize) -> Result<Vec<Message>> {
        let pattern = format!(
            "%{}%",
            needle
                .to_lowercase()
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_")
        );
        let mut statement = self.connection.prepare(
            "SELECT chat, id, sender, sender_name, from_me, timestamp, content, status, quoted, reactions, edited, thumbnail, mentions, forwarded, delivered_at, read_at
             FROM messages
             WHERE json_valid(content) AND lower(
                     coalesce(json_extract(content, '$.text'), '') || char(10) ||
                     coalesce(json_extract(content, '$.caption'), '') || char(10) ||
                     coalesce(json_extract(content, '$.file_name'), '') || char(10) ||
                     coalesce(json_extract(content, '$.question'), '') || char(10) ||
                     coalesce(json_extract(content, '$.display_name'), '') || char(10) ||
                     coalesce(json_extract(content, '$.name'), '')
                 ) LIKE ?1 ESCAPE '\\'
             ORDER BY timestamp DESC, rowid DESC
             LIMIT ?2",
        )?;
        let rows = statement.query_map(params![pattern, limit as i64], |row| {
            let chat: String = row.get(0)?;
            let content: String = row.get(6)?;
            let quoted: Option<String> = row.get(8)?;
            let reactions: String = row.get(9)?;
            let mentions: String = row.get(12)?;
            Ok(Message {
                id: row.get(1)?,
                chat,
                sender: row.get(2)?,
                sender_name: row.get(3)?,
                from_me: row.get(4)?,
                timestamp: row.get(5)?,
                content: serde_json::from_str(&content).unwrap_or(Content::Unsupported {
                    what: "unreadable".into(),
                }),
                status: status_from_rank(row.get(7)?),
                delivered_at: row.get(14)?,
                read_at: row.get(15)?,
                quoted: quoted.and_then(|quoted| serde_json::from_str(&quoted).ok()),
                reactions: serde_json::from_str(&reactions).unwrap_or_default(),
                edited: row.get(10)?,
                mentions: serde_json::from_str(&mentions).unwrap_or_default(),
                forwarded: row.get(13)?,
                thumbnail: row.get(11)?,
            })
        })?;
        let messages: Vec<Message> = rows.collect::<Result<_>>()?;
        Ok(messages)
    }

    /// Returns messages from `from` through `before`, ascending and limited.
    pub fn messages_range(
        &self,
        chat: &str,
        from: i64,
        before: (i64, &str),
        limit: usize,
    ) -> Result<Vec<Message>> {
        let mut statement = self.connection.prepare(
            "SELECT id, sender, sender_name, from_me, timestamp, content, status, quoted, reactions, edited, thumbnail, mentions, forwarded, delivered_at, read_at
             FROM messages
             WHERE chat = ?1 AND timestamp >= ?2 AND (timestamp < ?3 OR (timestamp = ?3 AND rowid <
                 (SELECT rowid FROM messages WHERE chat = ?1 AND id = ?4)))
             ORDER BY timestamp ASC, rowid ASC
             LIMIT ?5",
        )?;
        let rows = statement.query_map(
            params![chat, from, before.0, before.1, limit as i64],
            |row| {
                let content: String = row.get(5)?;
                let quoted: Option<String> = row.get(7)?;
                let reactions: String = row.get(8)?;
                let mentions: String = row.get(11)?;
                Ok(Message {
                    id: row.get(0)?,
                    chat: chat.to_owned(),
                    sender: row.get(1)?,
                    sender_name: row.get(2)?,
                    from_me: row.get(3)?,
                    timestamp: row.get(4)?,
                    content: serde_json::from_str(&content).unwrap_or(Content::Unsupported {
                        what: "unreadable".into(),
                    }),
                    status: status_from_rank(row.get(6)?),
                    delivered_at: row.get(13)?,
                    read_at: row.get(14)?,
                    quoted: quoted.and_then(|quoted| serde_json::from_str(&quoted).ok()),
                    reactions: serde_json::from_str(&reactions).unwrap_or_default(),
                    edited: row.get(9)?,
                    mentions: serde_json::from_str(&mentions).unwrap_or_default(),
                    forwarded: row.get(12)?,
                    thumbnail: row.get(10)?,
                })
            },
        )?;
        rows.collect()
    }

    /// Returns downloaded stickers we sent, newest first. Received stickers
    /// stay out of Recent, as in WhatsApp's own apps.
    pub fn recent_stickers(&self, limit: usize) -> Result<Vec<ArchivedSticker>> {
        let mut statement = self.connection.prepare(
            "SELECT json_extract(content, '$.media.path') AS path, MAX(timestamp), raw
             FROM messages
             WHERE json_extract(content, '$.kind') = 'sticker' AND path IS NOT NULL
               AND from_me = 1
             GROUP BY path
             ORDER BY 2 DESC
             LIMIT ?1",
        )?;
        let rows = statement.query_map(params![limit as i64], |row| {
            Ok(ArchivedSticker {
                last_used: row.get(1)?,
                path: std::path::PathBuf::from(row.get::<_, String>(0)?),
                raw: row.get(2)?,
            })
        })?;
        Ok(rows
            .flatten()
            .filter(|sticker| sticker.path.exists())
            .collect())
    }

    /// Returns undownloaded stickers we sent, newest first, for Recent.
    pub fn stickers_without_file(&self, limit: usize) -> Result<Vec<(String, String)>> {
        let mut statement = self.connection.prepare(
            "SELECT chat, id FROM messages
             WHERE json_extract(content, '$.kind') = 'sticker'
               AND json_extract(content, '$.media.path') IS NULL
               AND raw IS NOT NULL
               AND from_me = 1
             ORDER BY timestamp DESC
             LIMIT ?1",
        )?;
        let rows = statement.query_map(params![limit as i64], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        rows.collect()
    }

    /// Upserts a recent phone sticker, preserving the latest use time.
    pub fn upsert_phone_sticker(
        &self,
        hash: &str,
        raw: &[u8],
        last_used: i64,
        weight: f32,
    ) -> Result<()> {
        self.connection.execute(
            "INSERT INTO stickers (hash, raw, last_used, weight) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(hash) DO UPDATE SET
                 raw = excluded.raw,
                 last_used = MAX(stickers.last_used, excluded.last_used),
                 weight = excluded.weight",
            params![hash, raw, last_used, weight as f64],
        )?;
        Ok(())
    }

    /// Returns recent phone stickers by latest use.
    pub fn phone_stickers(&self) -> Result<Vec<PhoneSticker>> {
        let mut statement = self.connection.prepare(
            "SELECT hash, raw, last_used, path FROM stickers
             ORDER BY last_used DESC, weight DESC
             LIMIT 120",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(PhoneSticker {
                hash: row.get(0)?,
                raw: row.get(1)?,
                last_used: row.get(2)?,
                path: row
                    .get::<_, Option<String>>(3)?
                    .map(std::path::PathBuf::from),
            })
        })?;
        rows.collect()
    }

    pub fn set_sticker_path(&self, hash: &str, path: &Path) -> Result<()> {
        self.connection.execute(
            "UPDATE stickers SET path = ?2 WHERE hash = ?1",
            params![hash, path.to_string_lossy()],
        )?;
        Ok(())
    }

    /// Returns raw messages for re-deriving fields in newer versions.
    pub fn rows_with_raw(&self) -> Result<Vec<(String, String, Vec<u8>)>> {
        let mut statement = self
            .connection
            .prepare("SELECT chat, id, raw FROM messages WHERE raw IS NOT NULL")?;
        let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
        rows.collect()
    }

    /// Video messages with their raw protobuf.
    pub fn videos_with_raw(&self) -> Result<Vec<(String, String, Vec<u8>)>> {
        let mut statement = self.connection.prepare(
            "SELECT chat, id, raw FROM messages WHERE raw IS NOT NULL AND json_valid(content)
             AND json_extract(content, '$.kind') = 'video'",
        )?;
        let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
        rows.collect()
    }

    /// Interactive messages eligible for a derived presentation upgrade.
    /// Deleted and edited rows are left intact; callers preserve local media paths.
    pub fn interactive_placeholders(&self) -> Result<Vec<(String, String, Vec<u8>)>> {
        let mut statement = self.connection.prepare(
            "SELECT chat, id, raw FROM messages WHERE raw IS NOT NULL AND edited = 0 AND json_valid(content)
             AND ((json_extract(content, '$.kind') = 'unsupported'
                   AND json_extract(content, '$.what') = 'interactive message')
                  OR json_extract(content, '$.kind') = 'interactive')",
        )?;
        let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
        rows.collect()
    }

    /// Replaces protobuf-derived fields. Callers must retain local media paths.
    pub fn set_derived(
        &self,
        chat: &str,
        id: &str,
        content: &Content,
        mentions: &[crate::model::MentionRef],
        thumbnail: Option<&[u8]>,
        forwarded: bool,
    ) -> Result<()> {
        self.connection.execute(
            "UPDATE messages SET content = ?3, mentions = ?4, thumbnail = COALESCE(?5, thumbnail), forwarded = ?6
             WHERE chat = ?1 AND id = ?2",
            params![
                chat,
                id,
                serde_json::to_string(content).unwrap_or_default(),
                serde_json::to_string(mentions).unwrap_or_default(),
                thumbnail,
                forwarded
            ],
        )?;
        Ok(())
    }

    pub fn delete_message(&self, chat: &str, id: &str) -> Result<bool> {
        let deleted = self.connection.execute(
            "DELETE FROM messages WHERE chat = ?1 AND id = ?2",
            params![chat, id],
        )?;
        Ok(deleted > 0)
    }

    /// Removes a chat with everything stored for it.
    pub fn removal_point(&self, chat: &str) -> Result<Option<i64>> {
        self.connection
            .query_row(
                "SELECT through FROM chat_removals WHERE chat = ?1",
                params![chat],
                |row| row.get(0),
            )
            .optional()
    }

    /// Atomically removes only the range the linked device knew about, keeping
    /// newer messages and a durable barrier against replay after restart.
    pub fn remove_chat_through(&self, chat: &str, through: i64, delete: bool) -> Result<Removed> {
        let transaction = self.connection.unchecked_transaction()?;
        let through = self
            .removal_point(chat)?
            .map_or(through, |old| old.max(through));
        self.connection.execute(
            "INSERT INTO chat_removals (chat, through) VALUES (?1, ?2)
             ON CONFLICT(chat) DO UPDATE SET through = MAX(through, excluded.through)",
            params![chat, through],
        )?;
        let newer: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM messages WHERE chat = ?1 AND timestamp > ?2)",
            params![chat, through],
            |row| row.get(0),
        )?;
        let removed = if newer {
            let media =
                self.cached_media("m.chat = ?1 AND m.timestamp <= ?2", params![chat, through])?;
            self.connection.execute(
                "DELETE FROM messages WHERE chat = ?1 AND timestamp <= ?2",
                params![chat, through],
            )?;
            self.connection.execute(
                "UPDATE chats SET unread = MIN(unread, (SELECT COUNT(*) FROM messages
                    WHERE chat = ?1 AND from_me = 0 AND timestamp > COALESCE(read_through, 0))),
                    pending_read = CASE WHEN pending_read <= ?2 THEN NULL ELSE pending_read END
                 WHERE id = ?1",
                params![chat, through],
            )?;
            Removed {
                existed: true,
                media,
            }
        } else if delete {
            self.delete_chat(chat)?
        } else {
            self.clear_chat(chat)?
        };
        transaction.commit()?;
        Ok(removed)
    }

    /// Removes a chat with everything stored for it.
    ///
    /// `existed` reports whether a chat row was actually there, so a replayed
    /// sync action does not announce a removal twice.
    pub fn delete_chat(&self, chat: &str) -> Result<Removed> {
        let media = self.chat_media(chat)?;
        let existed = self
            .connection
            .execute("DELETE FROM chats WHERE id = ?1", params![chat])?
            > 0;
        self.purge_chat_rows(chat)?;
        Ok(Removed { existed, media })
    }

    /// Removes a chat's messages while keeping the chat itself, matching
    /// WhatsApp's "clear chat". The chat list preview empties through the
    /// message join; unread counters reset because clearing implies read.
    pub fn clear_chat(&self, chat: &str) -> Result<Removed> {
        let media = self.chat_media(chat)?;
        let existed = self.connection.execute(
            "UPDATE chats SET unread = 0, read_through = NULL, pending_read = NULL
                 WHERE id = ?1",
            params![chat],
        )? > 0;
        self.purge_chat_rows(chat)?;
        Ok(Removed { existed, media })
    }

    /// Drops every chat-scoped row outside the `chats` table itself.
    fn purge_chat_rows(&self, chat: &str) -> Result<()> {
        for table in [
            "messages",
            "group_receipts",
            "polls",
            "poll_history",
            "local_chat_labels",
            "drafts",
        ] {
            self.connection.execute(
                &format!("DELETE FROM {table} WHERE chat = ?1"),
                params![chat],
            )?;
        }
        self.connection
            .execute("DELETE FROM poll_votes WHERE chat = ?1", params![chat])?;
        Ok(())
    }

    /// Attachment paths recorded for one chat.
    fn chat_media(&self, chat: &str) -> Result<Vec<PathBuf>> {
        self.cached_media("m.chat = ?1", params![chat])
    }

    /// Attachment paths of the messages matching `filter` (over `messages m`),
    /// including an interactive card's image and each carousel card's image.
    fn cached_media(&self, filter: &str, params: impl rusqlite::Params) -> Result<Vec<PathBuf>> {
        let mut statement = self.connection.prepare(&format!(
            "SELECT file FROM (
                 SELECT json_extract(m.content, '$.media.path') AS file FROM messages m WHERE {filter}
                 UNION ALL
                 SELECT json_extract(m.content, '$.card.image.path') FROM messages m WHERE {filter}
                 UNION ALL
                 SELECT json_extract(c.value, '$.image.path')
                 FROM messages m, json_each(m.content, '$.card.carousel') c WHERE {filter}
             ) WHERE file IS NOT NULL"
        ))?;
        let rows =
            statement.query_map(params, |row| Ok(PathBuf::from(row.get::<_, String>(0)?)))?;
        rows.collect()
    }

    /// The id of `sender`'s newest live location in `chat` sent at or after
    /// `since`.
    pub fn latest_live_location(
        &self,
        chat: &str,
        sender: &str,
        since: i64,
    ) -> Result<Option<String>> {
        self.connection
            .query_row(
                "SELECT id FROM messages
                 WHERE chat = ?1 AND timestamp >= ?3 AND sender = ?2
                   AND json_extract(content, '$.kind') = 'livelocation'
                 ORDER BY timestamp DESC LIMIT 1",
                params![chat, sender, since],
                |row| row.get(0),
            )
            .optional()
    }

    pub fn message(&self, chat: &str, id: &str) -> Result<Option<Message>> {
        let mut statement = self.connection.prepare(
            "SELECT sender, sender_name, from_me, timestamp, content, status, quoted, reactions, edited, thumbnail, mentions, forwarded, delivered_at, read_at
             FROM messages WHERE chat = ?1 AND id = ?2",
        )?;
        statement
            .query_row(params![chat, id], |row| {
                let content: String = row.get(4)?;
                let quoted: Option<String> = row.get(6)?;
                let reactions: String = row.get(7)?;
                let mentions: String = row.get(10)?;
                Ok(Message {
                    id: id.to_owned(),
                    chat: chat.to_owned(),
                    sender: row.get(0)?,
                    sender_name: row.get(1)?,
                    from_me: row.get(2)?,
                    timestamp: row.get(3)?,
                    content: serde_json::from_str(&content).unwrap_or(Content::Unsupported {
                        what: "unreadable".into(),
                    }),
                    status: status_from_rank(row.get(5)?),
                    delivered_at: row.get(12)?,
                    read_at: row.get(13)?,
                    quoted: quoted.and_then(|quoted| serde_json::from_str(&quoted).ok()),
                    reactions: serde_json::from_str(&reactions).unwrap_or_default(),
                    edited: row.get(8)?,
                    mentions: serde_json::from_str(&mentions).unwrap_or_default(),
                    forwarded: row.get(11)?,
                    thumbnail: row.get(9)?,
                })
            })
            .optional()
    }

    /// Returns the earliest message for phone-history requests.
    pub fn oldest(&self, chat: &str) -> Result<Option<Message>> {
        let id: Option<String> = self
            .connection
            .query_row(
                "SELECT id FROM messages WHERE chat = ?1 ORDER BY timestamp ASC, rowid ASC LIMIT 1",
                params![chat],
                |row| row.get(0),
            )
            .optional()?;
        match id {
            Some(id) => self.message(chat, &id),
            None => Ok(None),
        }
    }

    /// Returns a message's raw protobuf for attachment downloads.
    pub fn raw(&self, chat: &str, id: &str) -> Result<Option<Vec<u8>>> {
        self.connection
            .query_row(
                "SELECT raw FROM messages WHERE chat = ?1 AND id = ?2",
                params![chat, id],
                |row| row.get(0),
            )
            .optional()
            .map(Option::flatten)
    }

    /// Advances delivery state, except that `Failed` may replace it. Stores the
    /// first timestamp for each delivery stage.
    pub fn set_status(&self, chat: &str, id: &str, status: Delivery, at: i64) -> Result<bool> {
        let rank = status_rank(status);
        let changed = if status == Delivery::Failed {
            self.connection.execute(
                "UPDATE messages SET status = ?3 WHERE chat = ?1 AND id = ?2",
                params![chat, id, rank],
            )?
        } else if let Some(column) = stamp_column(status) {
            self.connection.execute(
                &format!(
                    "UPDATE messages SET status = ?3, {column} = COALESCE({column}, ?4)
                     WHERE chat = ?1 AND id = ?2 AND status < ?3"
                ),
                params![chat, id, rank, at],
            )?
        } else {
            self.connection.execute(
                "UPDATE messages SET status = ?3 WHERE chat = ?1 AND id = ?2 AND status < ?3",
                params![chat, id, rank],
            )?
        };
        Ok(changed > 0)
    }

    /// Advances outgoing messages through `timestamp` to `status` and returns changed ids.
    pub fn advance_statuses(
        &self,
        chat: &str,
        up_to: i64,
        status: Delivery,
        at: i64,
    ) -> Result<Vec<String>> {
        let rank = status_rank(status);
        let mut statement = self.connection.prepare(
            "SELECT id FROM messages WHERE chat = ?1 AND from_me = 1 AND timestamp <= ?2 AND status > 0 AND status < ?3",
        )?;
        let ids: Vec<String> = statement
            .query_map(params![chat, up_to, rank], |row| row.get(0))?
            .collect::<Result<_>>()?;
        if let Some(column) = stamp_column(status) {
            self.connection.execute(
                &format!(
                    "UPDATE messages SET status = ?3, {column} = COALESCE({column}, ?4)
                     WHERE chat = ?1 AND from_me = 1 AND timestamp <= ?2 AND status > 0 AND status < ?3"
                ),
                params![chat, up_to, rank, at],
            )?;
        } else {
            self.connection.execute(
                "UPDATE messages SET status = ?3 WHERE chat = ?1 AND from_me = 1 AND timestamp <= ?2 AND status > 0 AND status < ?3",
                params![chat, up_to, rank],
            )?;
        }
        Ok(ids)
    }

    pub fn set_content(
        &self,
        chat: &str,
        id: &str,
        content: &Content,
        edited: bool,
    ) -> Result<bool> {
        let changed = self.connection.execute(
            "UPDATE messages SET content = ?3, edited = ?4 WHERE chat = ?1 AND id = ?2",
            params![
                chat,
                id,
                serde_json::to_string(content).unwrap_or_default(),
                edited
            ],
        )?;
        Ok(changed > 0)
    }

    /// Replaces an edited text body and its mention metadata.
    pub fn set_edited_text(
        &self,
        chat: &str,
        id: &str,
        content: &Content,
        mentions: &[crate::model::MentionRef],
    ) -> Result<bool> {
        let changed = self.connection.execute(
            "UPDATE messages SET content = ?3, mentions = ?4, edited = 1 WHERE chat = ?1 AND id = ?2",
            params![
                chat,
                id,
                serde_json::to_string(content).unwrap_or_default(),
                serde_json::to_string(mentions).unwrap_or_default(),
            ],
        )?;
        Ok(changed > 0)
    }

    /// Upserts a reaction, or removes it when the emoji is empty.
    pub fn set_reaction(
        &self,
        chat: &str,
        id: &str,
        sender: &str,
        from_me: bool,
        emoji: &str,
    ) -> Result<Option<Message>> {
        let Some(mut message) = self.message(chat, id)? else {
            return Ok(None);
        };
        message
            .reactions
            .retain(|reaction| reaction.sender != sender);
        if !emoji.is_empty() {
            message.reactions.push(crate::model::Reaction {
                sender: sender.to_owned(),
                from_me,
                emoji: emoji.to_owned(),
            });
        }
        self.connection.execute(
            "UPDATE messages SET reactions = ?3 WHERE chat = ?1 AND id = ?2",
            params![
                chat,
                id,
                serde_json::to_string(&message.reactions).unwrap_or_default()
            ],
        )?;
        Ok(Some(message))
    }

    pub fn upsert_contact(&self, contact: &Contact) -> Result<()> {
        self.connection.execute(
            "INSERT INTO contacts (id, full_name, push_name) VALUES (?1, ?2, ?3)
             ON CONFLICT(id) DO UPDATE SET
                full_name = COALESCE(excluded.full_name, full_name),
                push_name = COALESCE(excluded.push_name, push_name)",
            params![contact.id, contact.full_name, contact.push_name],
        )?;
        Ok(())
    }

    /// Returns a contact by id.
    pub fn contact(&self, id: &str) -> Result<Option<Contact>> {
        self.connection
            .query_row(
                "SELECT id, full_name, push_name FROM contacts WHERE id = ?1",
                params![id],
                |row| {
                    Ok(Contact {
                        id: row.get(0)?,
                        full_name: row.get(1)?,
                        push_name: row.get(2)?,
                    })
                },
            )
            .optional()
    }

    pub fn contacts(&self) -> Result<Vec<Contact>> {
        let mut statement = self
            .connection
            .prepare("SELECT id, full_name, push_name FROM contacts")?;
        let rows = statement.query_map([], |row| {
            Ok(Contact {
                id: row.get(0)?,
                full_name: row.get(1)?,
                push_name: row.get(2)?,
            })
        })?;
        rows.collect()
    }

    pub fn meta(&self, key: &str) -> Result<Option<String>> {
        self.connection
            .query_row(
                "SELECT value FROM meta WHERE key = ?1",
                params![key],
                |row| row.get(0),
            )
            .optional()
    }

    /// Older archives discarded pin times and could lose mute sync. Request
    /// one library-managed snapshot for an existing archive. Fresh links
    /// already receive snapshots; reconnecting must not add another request.
    pub fn take_preferences_refresh(&self) -> Result<bool> {
        const KEY: &str = "chat_preferences_refresh_v1";
        if self.meta(KEY)?.is_some() {
            return Ok(false);
        }
        let existing: bool =
            self.connection
                .query_row("SELECT EXISTS(SELECT 1 FROM chats)", [], |row| row.get(0))?;
        self.set_meta(KEY, "requested")?;
        Ok(existing)
    }

    pub fn set_meta(&self, key: &str, value: &str) -> Result<()> {
        self.connection.execute(
            "INSERT INTO meta (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    /// Clears all archived data during unlinking.
    pub fn clear(&self) -> Result<()> {
        self.connection.execute_batch(
            "DELETE FROM poll_history; DELETE FROM poll_votes; DELETE FROM polls; DELETE FROM group_receipts; DELETE FROM messages; DELETE FROM chats; DELETE FROM chat_removals; DELETE FROM contacts; DELETE FROM meta; DELETE FROM lids; DELETE FROM drafts; DELETE FROM local_chat_labels; DELETE FROM local_labels; DELETE FROM removed_recent_stickers; DELETE FROM favorite_stickers;",
        )
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::model::Content;

    pub(crate) fn message(chat: &str, id: &str, timestamp: i64, from_me: bool) -> Message {
        Message {
            id: id.into(),
            chat: chat.into(),
            sender: if from_me { "me@s.whatsapp.net" } else { chat }.into(),
            sender_name: None,
            from_me,
            timestamp,
            content: Content::text(format!("message {id}")),
            status: if from_me {
                Delivery::Pending
            } else {
                Delivery::None
            },
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

    #[test]
    fn a_chat_search_is_scoped_ordered_and_escaped() {
        let archive = Archive::in_memory().expect("opens");
        let chat = "1@s.whatsapp.net";
        let other = "2@s.whatsapp.net";
        archive.ensure_chat(chat, "Ada").expect("chat");
        archive.ensure_chat(other, "Grace").expect("chat");
        let text = |id: &str, at: i64, body: &str| {
            let mut message = message(chat, id, at, false);
            message.content = Content::text(body);
            message
        };
        let mut elsewhere = message(other, "x1", 15, false);
        elsewhere.content = Content::text("engine notes");
        for message in [
            text("m1", 10, "The Difference Engine"),
            text("m2", 20, "Nothing here"),
            text("m3", 30, "the engine again"),
            elsewhere,
        ] {
            archive.insert_message(&message, None).expect("insert");
        }
        // Only this chat, oldest first, so Enter walks forward in time.
        assert_eq!(
            archive
                .search_chat_messages(chat, "engine", 50)
                .expect("search"),
            vec!["m1".to_owned(), "m3".to_owned()]
        );
        // The limit keeps the oldest matches.
        assert_eq!(
            archive
                .search_chat_messages(chat, "engine", 1)
                .expect("search"),
            vec!["m1".to_owned()]
        );
        // A chat whose messages do not match has no hits.
        assert!(
            archive
                .search_chat_messages(other, "nothing", 50)
                .expect("search")
                .is_empty()
        );
        // Wildcards are text, like the cross-chat search.
        assert!(
            archive
                .search_chat_messages(chat, "%", 50)
                .expect("search")
                .is_empty()
        );
    }

    #[test]
    fn search_finds_text_captions_and_file_names() {
        let archive = Archive::in_memory().expect("opens");
        archive
            .ensure_chat("1@s.whatsapp.net", "Ada")
            .expect("chat");
        let media = || crate::model::Media {
            mime: "application/pdf".into(),
            size: 1,
            width: None,
            height: None,
            path: None,
            state: crate::model::MediaState::Idle,
        };
        let mut plain = message("1@s.whatsapp.net", "m1", 10, false);
        plain.content = Content::text("The Difference Engine assembles");
        let mut caption = message("1@s.whatsapp.net", "m2", 20, true);
        caption.content = Content::Document {
            media: media(),
            file_name: "Notes on the Engine.pdf".into(),
            caption: Some("progress at 100% now".into()),
            pages: None,
        };
        let mut other = message("1@s.whatsapp.net", "m3", 30, false);
        other.content = Content::text("Nothing of note");
        for row in [&plain, &caption, &other] {
            archive.insert_message(row, None).expect("insert");
        }
        // Match body and filename case-insensitively, newest first.
        let hits = archive.search_messages("ENGINE", 10).expect("search");
        let ids: Vec<&str> = hits.iter().map(|hit| hit.id.as_str()).collect();
        assert_eq!(ids, vec!["m2", "m1"]);
        // Escape LIKE wildcards from the search query.
        let hits = archive.search_messages("100%", 10).expect("search");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, "m2");
        assert!(
            archive
                .search_messages("100&", 10)
                .expect("search")
                .is_empty(),
            "the percent sign was matched literally"
        );
        assert!(
            archive
                .search_messages("zebra", 10)
                .expect("search")
                .is_empty()
        );
        // Apply the result limit.
        let hits = archive.search_messages("e", 1).expect("search");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, "m3", "newest first");
    }

    #[test]
    fn migrations_add_columns_to_an_older_archive() {
        let connection = Connection::open_in_memory().expect("opens");
        connection
            .execute_batch(
                "CREATE TABLE chats (id TEXT PRIMARY KEY, name TEXT NOT NULL, kind TEXT NOT NULL,
                    last_activity INTEGER NOT NULL DEFAULT 0, unread INTEGER NOT NULL DEFAULT 0,
                    archived INTEGER NOT NULL DEFAULT 0, pinned INTEGER NOT NULL DEFAULT 0, muted_until INTEGER);
                 INSERT INTO chats (id, name, kind) VALUES ('1@s.whatsapp.net', 'A', 'direct');",
            )
            .expect("old schema");
        let archive = Archive::prepare(connection).expect("migrates");
        let chats = archive.chats().expect("chats");
        assert_eq!(chats.len(), 1);
        assert!(chats[0].participants.is_empty());
        assert!(!chats[0].read_only);
        assert!(!chats[0].locked, "the lock column migrates in unset");
        assert!(
            !chats[0].group_subject_known,
            "legacy group placeholders remain identifiable"
        );
        let mut with_thumbnail = message("1@s.whatsapp.net", "m1", 1, false);
        with_thumbnail.thumbnail = Some(vec![1, 2, 3]);
        archive
            .insert_message(&with_thumbnail, None)
            .expect("insert");
        assert_eq!(
            archive
                .message("1@s.whatsapp.net", "m1")
                .expect("read")
                .expect("exists")
                .thumbnail,
            Some(vec![1, 2, 3])
        );
        assert_eq!(
            archive
                .oldest("1@s.whatsapp.net")
                .expect("oldest")
                .map(|m| m.id),
            Some("m1".into())
        );
    }

    #[test]
    fn ephemeral_setting_keeps_the_newest_timestamp() {
        let archive = Archive::in_memory().expect("opens");
        let chat = "1@s.whatsapp.net";
        archive.ensure_chat(chat, "Ada").expect("chat");

        assert!(archive.set_ephemeral(chat, 604_800, 20).expect("setting"));
        assert!(!archive.set_ephemeral(chat, 86_400, 10).expect("stale"));

        assert_eq!(
            archive.ephemeral_expiration(chat).expect("expiration"),
            Some(604_800)
        );
    }

    #[test]
    fn live_location_round_trips_and_upserts_in_place() {
        let archive = Archive::in_memory().expect("opens");
        let chat = "1@s.whatsapp.net";
        archive.ensure_chat(chat, "Ada").expect("chat");
        let live = |sequence: i64, ended: bool, thumbnail: Option<Vec<u8>>| {
            let mut row = message(chat, "live", 100, false);
            row.content = Content::LiveLocation {
                latitude: 51.5,
                longitude: -0.12,
                accuracy_m: Some(10),
                speed_mps: Some(1.1),
                heading_deg: Some(45),
                sequence,
                ended,
                updated: 0,
            };
            row.thumbnail = thumbnail;
            row
        };
        archive
            .insert_message(&live(1, false, None), None)
            .expect("insert");
        let read = archive
            .message(chat, "live")
            .expect("read")
            .expect("exists");
        assert_eq!(read.content, live(1, false, None).content);
        assert_eq!(read.thumbnail, None);

        // A newer update replaces the row in place rather than appending one.
        archive
            .insert_message(&live(2, false, Some(vec![1, 2, 3])), None)
            .expect("update");
        let updated = archive
            .message(chat, "live")
            .expect("read")
            .expect("exists");
        assert_eq!(
            updated.content,
            Content::LiveLocation {
                latitude: 51.5,
                longitude: -0.12,
                accuracy_m: Some(10),
                speed_mps: Some(1.1),
                heading_deg: Some(45),
                sequence: 2,
                ended: false,
                updated: 0,
            }
        );
        assert_eq!(updated.thumbnail, Some(vec![1, 2, 3]));
        assert_eq!(
            archive
                .latest_live_location(chat, &updated.sender, 100)
                .expect("query"),
            Some("live".to_owned())
        );
        assert_eq!(
            archive
                .latest_live_location(chat, &updated.sender, 101)
                .expect("query"),
            None
        );

        let rows: i64 = archive
            .connection
            .query_row(
                "SELECT COUNT(*) FROM messages WHERE chat = ?1 AND id = ?2",
                rusqlite::params![chat, "live"],
                |row| row.get(0),
            )
            .expect("count");
        assert_eq!(rows, 1);
    }

    #[test]
    fn ephemeral_setting_preserves_explicitly_disabled_timer() {
        let archive = Archive::in_memory().expect("opens");
        let chat = "1@s.whatsapp.net";
        archive.ensure_chat(chat, "Ada").expect("chat");

        archive.set_ephemeral(chat, 0, 20).expect("setting");

        assert_eq!(
            archive.ephemeral_expiration(chat).expect("expiration"),
            Some(0)
        );
    }

    #[test]
    fn group_info_is_kept() {
        let archive = Archive::in_memory().expect("opens");
        let chat = "1-2@g.us";
        archive.ensure_chat(chat, "Group").expect("chat");
        archive
            .set_group_info(
                chat,
                Some("Rust Berlin"),
                &["a@s.whatsapp.net".into()],
                true,
            )
            .expect("info");
        let row = archive.chat(chat).expect("chat").expect("exists");
        assert_eq!(row.name, "Rust Berlin");
        assert_eq!(row.participants, vec!["a@s.whatsapp.net"]);
        assert!(row.read_only);
        archive
            .set_group_info(chat, None, &[], false)
            .expect("info");
        assert_eq!(
            archive.chat(chat).expect("chat").expect("exists").name,
            "Rust Berlin"
        );
    }

    #[test]
    fn empty_metadata_preserves_group_titles_and_keeps_placeholders_unresolved() {
        let archive = Archive::in_memory().unwrap();
        let id = "fixture@g.us";
        archive.ensure_chat(id, "Group").unwrap();
        assert!(!archive.chat(id).unwrap().unwrap().group_subject_known);
        archive
            .set_group_info(id, Some(""), &["1@s.whatsapp.net".into()], false)
            .unwrap();
        let row = archive.chat(id).unwrap().unwrap();
        assert_eq!(row.name, "Group");
        assert!(!row.group_subject_known);
        archive.rename_chat(id, "Weekend plans").unwrap();
        archive.set_group_info(id, Some("  "), &[], false).unwrap();
        assert_eq!(archive.chat(id).unwrap().unwrap().name, "Weekend plans");
        archive.rename_chat(id, "Group").unwrap();
        let row = archive.chat(id).unwrap().unwrap();
        assert_eq!(row.name, "Group");
        assert!(row.group_subject_known);
        archive.set_group_info(id, None, &[], false).unwrap();
        assert!(archive.chat(id).unwrap().unwrap().group_subject_known);
    }

    #[test]
    fn existing_archives_request_preference_recovery_once_across_restarts() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("fixture.db");
        let key = [31; 32];
        {
            let archive = Archive::open_with_key(&path, &key).unwrap();
            archive.ensure_chat("1@s.whatsapp.net", "Fixture").unwrap();
            assert!(archive.take_preferences_refresh().unwrap());
            assert!(!archive.take_preferences_refresh().unwrap());
        }
        let archive = Archive::open_with_key(&path, &key).unwrap();
        assert!(!archive.take_preferences_refresh().unwrap());
        let fresh = Archive::in_memory().unwrap();
        assert!(!fresh.take_preferences_refresh().unwrap());
        fresh
            .ensure_chat("1@s.whatsapp.net", "Initial history")
            .unwrap();
        assert!(!fresh.take_preferences_refresh().unwrap());
    }

    #[test]
    fn mute_and_pin_versions_survive_restart_and_ignore_older_updates() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("fixture.db");
        let key = [29; 32];
        let id = "1@s.whatsapp.net";
        {
            let archive = Archive::open_with_key(&path, &key).unwrap();
            archive.ensure_chat(id, "Fixture").unwrap();
            archive.set_muted_at(id, Some(0), 200).unwrap();
            archive.set_pinned_at(id, true, 200).unwrap();
        }
        let archive = Archive::open_with_key(&path, &key).unwrap();
        archive.set_muted_at(id, None, 100).unwrap();
        archive.set_pinned_at(id, false, 100).unwrap();
        archive
            .upsert_chat(&Chat::new(id.into(), "History name".into()))
            .unwrap();
        let chat = archive.chat(id).unwrap().unwrap();
        assert_eq!(chat.name, "History name");
        assert_eq!(chat.muted_until, Some(0));
        assert!(chat.pinned);
        assert_eq!(chat.pinned_at, 200);
        archive.set_muted_at(id, None, 300).unwrap();
        archive.set_pinned_at(id, false, 300).unwrap();
        let chat = archive.chat(id).unwrap().unwrap();
        assert_eq!(chat.muted_until, None);
        assert!(!chat.pinned);
        assert_eq!(chat.pinned_at, 0);
    }

    /// Builds a chat with rows in every chat-scoped table: a text message, a
    /// downloaded image, a poll with its history and a vote, and a group
    /// receipt. The builder checks each table, so a missing row cannot let a
    /// broken purge pass.
    fn furnished_chat(archive: &Archive, chat: &str, media: &Path) -> String {
        archive.ensure_chat(chat, "Somebody").expect("chat");
        archive
            .insert_message(&message(chat, "m1", 100, false), None)
            .expect("insert");
        let mut image = message(chat, "m2", 200, false);
        image.content = Content::Image {
            caption: None,
            media: crate::model::Media {
                mime: "image/jpeg".into(),
                size: 1,
                width: None,
                height: None,
                path: None,
                state: crate::model::MediaState::Idle,
            },
        };
        archive.insert_message(&image, None).expect("insert");
        archive
            .set_media_path(chat, "m2", media)
            .expect("media path");
        archive.set_unread(chat, 3).expect("unread");
        archive
            .connection
            .execute_batch(&format!(
                "INSERT INTO polls (chat, id, creator, secret) VALUES ('{chat}', 'p1', '{chat}', x'00');
                 INSERT INTO poll_history (chat, id) VALUES ('{chat}', 'p1');
                 INSERT INTO poll_votes (chat, poll, voter, sender, update_id, at, from_me)
                     VALUES ('{chat}', 'p1', '{chat}', '{chat}', 'u1', 150, 0);
                 INSERT INTO group_receipts (chat, id, recipient) VALUES ('{chat}', 'm1', '{chat}');
                 INSERT INTO local_chat_labels (chat, label) VALUES ('{chat}', 'label-1');
                 INSERT INTO drafts (chat, text, updated_at) VALUES ('{chat}', 'unsent', 150);"
            ))
            .expect("poll and receipt rows");
        for table in CHAT_TABLES {
            assert!(
                rows(archive, table, chat) > 0,
                "{table} needs a row to remove"
            );
        }
        chat.to_owned()
    }

    /// Every table keyed by chat besides `chats` itself.
    const CHAT_TABLES: [&str; 7] = [
        "messages",
        "group_receipts",
        "polls",
        "poll_history",
        "poll_votes",
        "local_chat_labels",
        "drafts",
    ];

    fn rows(archive: &Archive, table: &str, chat: &str) -> i64 {
        archive
            .connection
            .query_row(
                &format!("SELECT COUNT(*) FROM {table} WHERE chat = ?1"),
                params![chat],
                |row| row.get(0),
            )
            .expect("count")
    }

    #[test]
    fn deleting_a_chat_removes_it_with_its_messages_and_reports_its_media() {
        let archive = Archive::in_memory().expect("opens");
        let media = PathBuf::from("/cache/zapfast/media/m2.jpg");
        let gone = furnished_chat(&archive, "1@s.whatsapp.net", &media);
        let kept = furnished_chat(&archive, "2@s.whatsapp.net", &media);

        let removed = archive.delete_chat(&gone).expect("delete");

        assert!(removed.existed);
        assert_eq!(removed.media, vec![media]);
        assert!(archive.chat(&gone).expect("chat").is_none());
        assert!(
            archive
                .messages(&gone, None, 50)
                .expect("messages")
                .is_empty()
        );
        for table in CHAT_TABLES {
            assert_eq!(
                rows(&archive, table, &gone),
                0,
                "{table} still holds the chat"
            );
        }
        // Only the named chat goes; its neighbour is untouched.
        assert!(archive.chat(&kept).expect("chat").is_some());
        assert_eq!(
            archive.messages(&kept, None, 50).expect("messages").len(),
            2
        );
        for table in CHAT_TABLES {
            assert!(
                rows(&archive, table, &kept) > 0,
                "{table} lost the other chat"
            );
        }
    }

    #[test]
    fn deleting_an_unknown_chat_reports_that_nothing_was_there() {
        let archive = Archive::in_memory().expect("opens");
        let removed = archive
            .delete_chat("nobody@s.whatsapp.net")
            .expect("delete");
        assert!(!removed.existed);
        assert!(removed.media.is_empty());
    }

    #[test]
    fn delayed_chat_removal_preserves_newer_messages_and_survives_restart() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("fixture.db");
        let key = [42; 32];
        let chat = "1@s.whatsapp.net";
        {
            let archive = Archive::open_with_key(&path, &key).unwrap();
            archive.ensure_chat(chat, "Fixture").unwrap();
            for at in [100, 200, 300] {
                archive
                    .insert_message(&message(chat, &format!("m{at}"), at, false), None)
                    .unwrap();
            }
            archive.remove_chat_through(chat, 200, true).unwrap();
            assert!(archive.chat(chat).unwrap().is_some());
            assert_eq!(
                archive
                    .messages(chat, None, 50)
                    .unwrap()
                    .iter()
                    .map(|m| m.timestamp)
                    .collect::<Vec<_>>(),
                [300]
            );
            archive.remove_chat_through(chat, 100, false).unwrap();
            assert_eq!(archive.removal_point(chat).unwrap(), Some(200));
        }
        let archive = Archive::open_with_key(&path, &key).unwrap();
        assert_eq!(archive.removal_point(chat).unwrap(), Some(200));
        assert!(archive.message(chat, "m300").unwrap().is_some());
        archive.remove_chat_through("42@lid", 150, true).unwrap();
        archive.put_lid("42", "15550000000").unwrap();
        assert_eq!(
            archive.removal_point("15550000000@s.whatsapp.net").unwrap(),
            Some(150)
        );
        archive.clear().unwrap();
        assert_eq!(archive.removal_point(chat).unwrap(), None);
    }

    #[test]
    fn chat_removal_rolls_back_the_cutoff_and_rows_on_failure() {
        let archive = Archive::in_memory().unwrap();
        let chat = "1@s.whatsapp.net";
        archive.ensure_chat(chat, "Fixture").unwrap();
        archive
            .insert_message(&message(chat, "m1", 100, false), None)
            .unwrap();
        archive.connection.execute_batch("CREATE TRIGGER refuse_delete BEFORE DELETE ON messages BEGIN SELECT RAISE(ABORT, 'fixture failure'); END;").unwrap();
        assert!(archive.remove_chat_through(chat, 100, true).is_err());
        assert!(archive.chat(chat).unwrap().is_some());
        assert!(archive.message(chat, "m1").unwrap().is_some());
        assert_eq!(archive.removal_point(chat).unwrap(), None);
    }

    #[test]
    fn removing_a_chat_reports_interactive_card_images() {
        let image = |name: &str| crate::model::Media {
            mime: "image/jpeg".into(),
            size: 1,
            width: None,
            height: None,
            path: Some(PathBuf::from(format!("/cache/zapfast/media/{name}.jpg"))),
            state: Default::default(),
        };
        let card = |image: crate::model::Media| crate::model::InteractiveCard {
            image: Some(image),
            ..Default::default()
        };
        let insert = |archive: &Archive, chat: &str| {
            archive.ensure_chat(chat, "Shop").unwrap();
            let mut single = message(chat, "card", 100, false);
            single.content = Content::Interactive {
                text: String::new(),
                card: Some(Box::new(card(image("card")))),
            };
            let mut carousel = message(chat, "carousel", 100, false);
            carousel.content = Content::Interactive {
                text: String::new(),
                card: Some(Box::new(crate::model::InteractiveCard {
                    carousel: vec![card(image("first")), card(image("second"))],
                    ..Default::default()
                })),
            };
            archive.insert_message(&single, None).unwrap();
            archive.insert_message(&carousel, None).unwrap();
        };
        let mut expected: Vec<_> = ["card", "first", "second"]
            .map(|name| image(name).path.unwrap())
            .into();
        expected.sort();
        let chat = "1@s.whatsapp.net";
        for removal in ["clear", "delete", "through"] {
            let archive = Archive::in_memory().expect("opens");
            insert(&archive, chat);
            let mut removed = match removal {
                "clear" => archive.clear_chat(chat),
                "delete" => archive.delete_chat(chat),
                _ => {
                    // A newer message makes the removal keep the chat's tail.
                    archive
                        .insert_message(&message(chat, "newer", 200, false), None)
                        .unwrap();
                    archive.remove_chat_through(chat, 100, false)
                }
            }
            .expect("removes")
            .media;
            removed.sort();
            assert_eq!(removed, expected, "{removal}");
        }
    }

    #[test]
    fn clearing_a_chat_keeps_it_but_empties_its_messages_and_unread_count() {
        let archive = Archive::in_memory().expect("opens");
        let media = PathBuf::from("/cache/zapfast/media/m2.jpg");
        let chat = furnished_chat(&archive, "1@s.whatsapp.net", &media);

        let removed = archive.clear_chat(&chat).expect("clear");

        assert!(removed.existed);
        assert_eq!(removed.media, vec![media]);
        let row = archive.chat(&chat).expect("chat").expect("still listed");
        assert_eq!(row.unread, 0);
        // The chat-list preview comes from the message join, so it empties too.
        assert!(row.last.is_none());
        for table in CHAT_TABLES {
            assert_eq!(
                rows(&archive, table, &chat),
                0,
                "{table} still holds the chat"
            );
        }
        assert!(
            archive
                .messages(&chat, None, 50)
                .expect("messages")
                .is_empty()
        );
    }

    #[test]
    fn lock_versions_survive_restart_and_ignore_older_updates() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("fixture.db");
        let key = [37; 32];
        let id = "491700000001@s.whatsapp.net";
        {
            let archive = Archive::open_with_key(&path, &key).unwrap();
            archive.ensure_chat(id, "Ada").expect("chat");
            archive.set_locked_at(id, true, 200).unwrap();
        }
        let archive = Archive::open_with_key(&path, &key).unwrap();
        // An older replayed patch must not undo the newer lock.
        archive.set_locked_at(id, false, 100).unwrap();
        assert!(archive.chat(id).unwrap().unwrap().locked);
        archive.set_locked_at(id, false, 300).unwrap();
        assert!(!archive.chat(id).unwrap().unwrap().locked);
        // Upserts from history metadata never touch the lock state.
        archive.set_locked(id, true).unwrap();
        archive
            .upsert_chat(&Chat::new(id.into(), "History name".into()))
            .unwrap();
        assert!(archive.chat(id).unwrap().unwrap().locked);
    }

    #[test]
    fn privacy_id_mapping_preserves_history_locks_but_respects_versioned_unlocks() {
        for existing in [false, true] {
            let archive = Archive::in_memory().unwrap();
            let lid = "2@lid";
            let phone = "1@s.whatsapp.net";
            archive.ensure_chat(lid, "Fixture").unwrap();
            archive.set_locked_snapshot(lid, true).unwrap();
            if existing {
                archive.ensure_chat(phone, "Fixture").unwrap();
            }
            archive.put_lid("2", "1").unwrap();
            assert!(archive.chat(phone).unwrap().unwrap().locked);

            // Conflicting unversioned history cannot expose the mapped chat.
            archive.set_locked_snapshot(lid, false).unwrap();
            archive.set_pinned_at(lid, true, 100).unwrap();
            archive.put_lid("2", "1").unwrap();
            assert!(archive.chat(phone).unwrap().unwrap().locked);

            // An authenticated unlock takes precedence over stale history.
            archive.set_locked_at(phone, false, 200).unwrap();
            archive.set_locked_snapshot(lid, true).unwrap();
            archive.put_lid("2", "1").unwrap();
            assert!(!archive.chat(phone).unwrap().unwrap().locked);
        }
    }

    #[test]
    fn a_chat_keeps_its_own_notification_sound() {
        use crate::settings::NotificationSound;
        let archive = Archive::in_memory().unwrap();
        let id = "1@s.whatsapp.net";
        archive.ensure_chat(id, "Ada").unwrap();
        assert_eq!(archive.chat(id).unwrap().unwrap().notification_sound, None);
        let sound = NotificationSound::Custom("/sounds/ada.ogg".into());
        archive.set_notification_sound(id, Some(&sound)).unwrap();
        assert_eq!(
            archive.chat(id).unwrap().unwrap().notification_sound,
            Some(sound)
        );
        archive.set_notification_sound(id, None).unwrap();
        assert_eq!(archive.chat(id).unwrap().unwrap().notification_sound, None);
    }

    #[test]
    fn privacy_id_mapping_carries_the_newest_archive_state() {
        for existing in [false, true] {
            let archive = Archive::in_memory().unwrap();
            let lid = "2@lid";
            let phone = "1@s.whatsapp.net";
            archive.ensure_chat(lid, "Fixture").unwrap();
            archive.set_archived_at(lid, true, 100).unwrap();
            if existing {
                archive.ensure_chat(phone, "Fixture").unwrap();
            }
            archive.put_lid("2", "1").unwrap();
            assert!(archive.chat(phone).unwrap().unwrap().archived);

            // An older privacy-id version cannot undo a newer one.
            archive.set_archived_at(phone, false, 200).unwrap();
            archive.put_lid("2", "1").unwrap();
            assert!(!archive.chat(phone).unwrap().unwrap().archived);

            // History cannot supersede a versioned archive state.
            let mut history = Chat::new(phone.into(), "History name".into());
            history.archived = true;
            archive.upsert_chat(&history).unwrap();
            assert!(!archive.chat(phone).unwrap().unwrap().archived);
        }
    }

    #[test]
    fn chats_order_by_activity_and_carry_their_last_message() {
        let archive = Archive::in_memory().expect("opens");
        let a = "1@s.whatsapp.net";
        let b = "2@s.whatsapp.net";
        archive.ensure_chat(a, "A").expect("chat");
        archive.ensure_chat(b, "B").expect("chat");
        archive
            .insert_message(&message(a, "m1", 100, false), None)
            .expect("insert");
        archive
            .insert_message(&message(b, "m2", 200, true), None)
            .expect("insert");
        archive
            .insert_message(&message(a, "m3", 150, false), None)
            .expect("insert");
        let chats = archive.chats().expect("chats");
        assert_eq!(chats[0].id, b);
        assert_eq!(
            chats[0].last.as_ref().map(|last| last.summary.as_str()),
            Some("message m2")
        );
        assert_eq!(
            chats[0].last.as_ref().map(|last| last.status),
            Some(Delivery::Pending)
        );
        assert_eq!(chats[1].id, a);
        assert_eq!(chats[1].last_activity, 150);
        assert_eq!(
            chats[1].last.as_ref().map(|last| last.summary.as_str()),
            Some("message m3")
        );
    }

    #[test]
    fn a_saved_name_keeps_the_push_name_beside_it() {
        let archive = Archive::in_memory().expect("opens");
        let id = "491700000001@s.whatsapp.net";
        archive
            .upsert_contact(&Contact {
                id: id.into(),
                full_name: None,
                push_name: Some("~slavic".into()),
            })
            .expect("stores");
        archive
            .upsert_contact(&Contact {
                id: id.into(),
                full_name: Some("Slavic".into()),
                push_name: None,
            })
            .expect("renames");
        let stored = archive.contact(id).expect("reads").expect("exists");
        assert_eq!(stored.full_name.as_deref(), Some("Slavic"));
        assert_eq!(stored.push_name.as_deref(), Some("~slavic"));
        assert!(
            archive
                .contact("nobody@s.whatsapp.net")
                .expect("reads")
                .is_none()
        );
    }

    #[test]
    fn statuses_only_move_forward() {
        let archive = Archive::in_memory().expect("opens");
        let chat = "1@s.whatsapp.net";
        archive.ensure_chat(chat, "A").expect("chat");
        archive
            .insert_message(&message(chat, "m1", 100, true), None)
            .expect("insert");
        assert!(
            archive
                .set_status(chat, "m1", Delivery::Read, 500)
                .expect("status")
        );
        assert!(
            !archive
                .set_status(chat, "m1", Delivery::Delivered, 600)
                .expect("status")
        );
        let stored = archive.message(chat, "m1").expect("read").expect("exists");
        assert_eq!(stored.status, Delivery::Read);
        assert_eq!(stored.read_at, Some(500));
        assert_eq!(stored.delivered_at, None);
        // History replay must not lower an existing Read state.
        archive
            .insert_message(&message(chat, "m1", 100, true), None)
            .expect("insert");
        assert_eq!(
            archive
                .message(chat, "m1")
                .expect("read")
                .expect("exists")
                .status,
            Delivery::Read
        );
        assert!(
            archive
                .set_status(chat, "m1", Delivery::Failed, 700)
                .expect("status")
        );
    }

    #[test]
    fn one_insert_keeps_the_furthest_status_and_the_stored_reactions() {
        let archive = Archive::in_memory().expect("opens");
        let chat = "1@s.whatsapp.net";
        archive.ensure_chat(chat, "A").expect("chat");
        let reaction = |sender: &str, emoji: &str| crate::model::Reaction {
            sender: sender.into(),
            from_me: false,
            emoji: emoji.into(),
        };

        let mut sent = message(chat, "m1", 100, true);
        sent.status = Delivery::Sent;
        sent.reactions = vec![reaction("2@s.whatsapp.net", "🎉")];
        archive.insert_message(&sent, None).expect("insert");
        let stored = archive.message(chat, "m1").expect("read").expect("exists");
        assert_eq!(stored.status, Delivery::Sent);
        assert_eq!(stored.reactions.len(), 1);

        // A receipt moves the state forward, and the history row that arrives
        // without reactions must not wipe the stored list.
        let mut receipt = message(chat, "m1", 100, true);
        receipt.status = Delivery::Delivered;
        archive.insert_message(&receipt, None).expect("receipt");
        let stored = archive.message(chat, "m1").expect("read").expect("exists");
        assert_eq!(stored.status, Delivery::Delivered);
        assert_eq!(stored.reactions.len(), 1, "an empty list must not wipe");
        assert_eq!(stored.reactions[0].emoji, "🎉");

        // A replay cannot lower the state, but an explicit failure must show.
        let mut replay = message(chat, "m1", 100, true);
        replay.status = Delivery::Pending;
        archive.insert_message(&replay, None).expect("replay");
        assert_eq!(
            archive
                .message(chat, "m1")
                .expect("read")
                .expect("exists")
                .status,
            Delivery::Delivered
        );
        let mut failure = message(chat, "m1", 100, true);
        failure.status = Delivery::Failed;
        archive.insert_message(&failure, None).expect("failure");
        assert_eq!(
            archive
                .message(chat, "m1")
                .expect("read")
                .expect("exists")
                .status,
            Delivery::Failed
        );

        // A history snapshot with reactions replaces the list.
        let mut snapshot = message(chat, "m1", 100, true);
        snapshot.reactions = vec![reaction("3@s.whatsapp.net", "👍")];
        archive.insert_message(&snapshot, None).expect("snapshot");
        let stored = archive.message(chat, "m1").expect("read").expect("exists");
        assert_eq!(stored.reactions.len(), 1);
        assert_eq!(stored.reactions[0].sender, "3@s.whatsapp.net");
    }

    #[test]
    fn a_read_receipt_covers_everything_before_it() {
        let archive = Archive::in_memory().expect("opens");
        let chat = "1@s.whatsapp.net";
        archive.ensure_chat(chat, "A").expect("chat");
        for (id, timestamp) in [("m1", 100), ("m2", 200), ("m3", 300)] {
            archive
                .insert_message(&message(chat, id, timestamp, true), None)
                .expect("insert");
        }
        archive
            .insert_message(&message(chat, "theirs", 250, false), None)
            .expect("insert");
        let changed = archive
            .advance_statuses(chat, 200, Delivery::Read, 400)
            .expect("advance");
        assert_eq!(changed, vec!["m1", "m2"]);
        let messages = archive.messages(chat, None, 10).expect("messages");
        let statuses: Vec<Delivery> = messages.iter().map(|message| message.status).collect();
        assert_eq!(
            statuses,
            vec![
                Delivery::Read,
                Delivery::Read,
                Delivery::None,
                Delivery::Pending
            ]
        );
        assert_eq!(messages[0].read_at, Some(400));
    }

    #[test]
    fn paging_walks_backwards_in_time() {
        let archive = Archive::in_memory().expect("opens");
        let chat = "1@s.whatsapp.net";
        archive.ensure_chat(chat, "A").expect("chat");
        for index in 0..10 {
            archive
                .insert_message(
                    &message(chat, &format!("m{index}"), 100 + index, false),
                    None,
                )
                .expect("insert");
        }
        let newest = archive.messages(chat, None, 3).expect("messages");
        assert_eq!(
            newest.iter().map(|m| m.timestamp).collect::<Vec<_>>(),
            vec![107, 108, 109]
        );
        let older = archive
            .messages(chat, Some((107, "m7")), 3)
            .expect("messages");
        assert_eq!(
            older.iter().map(|m| m.timestamp).collect::<Vec<_>>(),
            vec![104, 105, 106]
        );
    }

    #[test]
    fn paging_keeps_every_message_of_a_second() {
        // Cover messages sharing one timestamp across page boundaries.
        let archive = Archive::in_memory().expect("opens");
        let chat = "1@s.whatsapp.net";
        archive.ensure_chat(chat, "A").expect("chat");
        archive
            .insert_message(&message(chat, "before", 99, false), None)
            .expect("insert");
        for index in 0..5 {
            archive
                .insert_message(&message(chat, &format!("a{index}"), 100, false), None)
                .expect("insert");
        }
        let first = archive.messages(chat, None, 3).expect("messages");
        assert_eq!(
            first.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            ["a2", "a3", "a4"]
        );
        let oldest = &first[0];
        let second = archive
            .messages(chat, Some((oldest.timestamp, &oldest.id)), 3)
            .expect("messages");
        assert_eq!(
            second.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            ["before", "a0", "a1"],
            "the rest of the second comes next, not the message before it alone"
        );
        let range = archive
            .messages_range(chat, 100, (100, "a2"), 10)
            .expect("range");
        assert_eq!(
            range.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            ["a0", "a1"]
        );
    }

    #[test]
    fn ranges_and_deletion() {
        let archive = Archive::in_memory().expect("opens");
        let chat = "1@s.whatsapp.net";
        archive.ensure_chat(chat, "A").expect("chat");
        for index in 0..6 {
            archive
                .insert_message(
                    &message(chat, &format!("m{index}"), 100 + index, false),
                    None,
                )
                .expect("insert");
        }
        let range = archive
            .messages_range(chat, 102, (105, "m5"), 10)
            .expect("range");
        assert_eq!(
            range.iter().map(|m| m.timestamp).collect::<Vec<_>>(),
            vec![102, 103, 104]
        );
        assert!(archive.delete_message(chat, "m3").expect("delete"));
        assert!(!archive.delete_message(chat, "m3").expect("delete"));
        assert!(archive.message(chat, "m3").expect("read").is_none());
    }

    #[test]
    fn reactions_replace_per_sender() {
        let archive = Archive::in_memory().expect("opens");
        let chat = "1@s.whatsapp.net";
        archive.ensure_chat(chat, "A").expect("chat");
        archive
            .insert_message(&message(chat, "m1", 100, false), None)
            .expect("insert");
        archive
            .set_reaction(chat, "m1", chat, false, "👍")
            .expect("react");
        let updated = archive
            .set_reaction(chat, "m1", chat, false, "❤️")
            .expect("react")
            .expect("exists");
        assert_eq!(updated.reactions.len(), 1);
        assert_eq!(updated.reactions[0].emoji, "❤️");
        let removed = archive
            .set_reaction(chat, "m1", chat, false, "")
            .expect("react")
            .expect("exists");
        assert!(removed.reactions.is_empty());
    }

    #[test]
    fn a_history_replay_keeps_another_senders_custom_reaction() {
        let archive = Archive::in_memory().expect("opens");
        let chat = "1@s.whatsapp.net";
        archive.ensure_chat(chat, "A").expect("chat");
        archive
            .insert_message(&message(chat, "m1", 100, false), None)
            .expect("insert");
        archive
            .set_reaction(chat, "m1", "2@s.whatsapp.net", false, "🏆")
            .expect("react");
        archive
            .insert_message(&message(chat, "m1", 100, false), None)
            .expect("replay");
        let stored = archive.message(chat, "m1").expect("read").expect("exists");
        assert_eq!(stored.reactions.len(), 1);
        assert_eq!(stored.reactions[0].emoji, "🏆");
        assert!(!stored.reactions[0].from_me);
    }

    #[test]
    fn a_history_snapshot_replaces_the_reaction_list() {
        let archive = Archive::in_memory().expect("opens");
        let chat = "1@s.whatsapp.net";
        archive.ensure_chat(chat, "A").expect("chat");
        archive
            .insert_message(&message(chat, "m1", 100, false), None)
            .expect("insert");
        archive
            .set_reaction(chat, "m1", "2@s.whatsapp.net", false, "👍")
            .expect("react");
        archive
            .set_reaction(chat, "m1", "3@s.whatsapp.net", false, "❤️")
            .expect("react");
        let mut replay = message(chat, "m1", 100, false);
        replay.reactions = vec![crate::model::Reaction {
            sender: "3@s.whatsapp.net".into(),
            from_me: false,
            emoji: "🎉".into(),
        }];
        archive.insert_message(&replay, None).expect("replay");
        let stored = archive.message(chat, "m1").expect("read").expect("exists");
        assert_eq!(stored.reactions.len(), 1);
        assert_eq!(stored.reactions[0].sender, "3@s.whatsapp.net");
        assert_eq!(stored.reactions[0].emoji, "🎉");
    }

    #[test]
    fn unread_counts_and_incoming_ids_track_the_other_side() {
        let archive = Archive::in_memory().expect("opens");
        let chat = "1@s.whatsapp.net";
        archive.ensure_chat(chat, "A").expect("chat");
        archive
            .insert_message(&message(chat, "m1", 100, false), None)
            .expect("insert");
        archive.bump_unread(chat).expect("bump");
        archive
            .insert_message(&message(chat, "mine", 150, true), None)
            .expect("insert");
        archive
            .insert_message(&message(chat, "m2", 200, false), None)
            .expect("insert");
        archive.bump_unread(chat).expect("bump");
        assert_eq!(archive.chat(chat).expect("chat").expect("exists").unread, 2);
        let ids: Vec<String> = archive
            .unread_incoming(chat, 2)
            .expect("ids")
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(ids, vec!["m2", "m1"]);
        archive.mark_read(chat).expect("read");
        assert_eq!(archive.chat(chat).expect("chat").expect("exists").unread, 0);
    }

    #[test]
    fn read_positions_and_pending_sync_survive_reopening_the_archive() {
        let dir = std::env::temp_dir().join(format!("zapfast-read-test-{}", std::process::id()));
        let path = dir.join("archive.db");
        let _ = std::fs::remove_dir_all(&dir);
        let chat = "1@s.whatsapp.net";
        {
            let archive = Archive::open_with_key(&path, &[7; 32]).unwrap();
            archive.ensure_chat(chat, "A").unwrap();
            archive
                .insert_message(&message(chat, "a", 100, false), None)
                .unwrap();
            archive.bump_unread(chat).unwrap();
            archive.mark_read(chat).unwrap();
            archive.queue_read_sync(chat).unwrap();
        }
        {
            let archive = Archive::open_with_key(&path, &[7; 32]).unwrap();
            assert_eq!(archive.read_through(chat).unwrap(), Some(100));
            assert_eq!(archive.pending_reads().unwrap(), vec![(chat.into(), 100)]);
            archive
                .insert_message(&message(chat, "b", 200, false), None)
                .unwrap();
            archive.bump_unread(chat).unwrap();
            archive.mark_read(chat).unwrap();
            archive.queue_read_sync(chat).unwrap();
            archive.finish_read_sync(chat, 100).unwrap();
            assert_eq!(
                archive.pending_reads().unwrap(),
                vec![(chat.into(), 200)],
                "an old completion must not lose the next read"
            );
            archive.finish_read_sync(chat, 200).unwrap();
            assert!(archive.pending_reads().unwrap().is_empty());
            archive.clear().unwrap();
            assert!(archive.read_through(chat).unwrap().is_none());
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn media_paths_are_written_into_the_content() {
        let archive = Archive::in_memory().expect("opens");
        let chat = "1@s.whatsapp.net";
        archive.ensure_chat(chat, "A").expect("chat");
        let mut picture = message(chat, "p1", 100, false);
        picture.content = Content::Image {
            caption: None,
            media: crate::model::Media {
                mime: "image/jpeg".into(),
                size: 10,
                width: None,
                height: None,
                path: None,
                state: Default::default(),
            },
        };
        archive.insert_message(&picture, None).expect("insert");
        let updated = archive
            .set_media_path(chat, "p1", Path::new("/tmp/p1.jpg"))
            .expect("set")
            .expect("exists");
        assert_eq!(
            updated.content.media().and_then(|media| media.path.clone()),
            Some(std::path::PathBuf::from("/tmp/p1.jpg"))
        );
        let reread = archive.message(chat, "p1").expect("read").expect("exists");
        assert_eq!(reread.content, updated.content);
    }

    #[test]
    fn raw_bytes_survive_a_replay_without_them() {
        let archive = Archive::in_memory().expect("opens");
        let chat = "1@s.whatsapp.net";
        archive.ensure_chat(chat, "A").expect("chat");
        archive
            .insert_message(&message(chat, "m1", 100, false), Some(&[1, 2, 3]))
            .expect("insert");
        archive
            .insert_message(&message(chat, "m1", 100, false), None)
            .expect("insert");
        assert_eq!(archive.raw(chat, "m1").expect("raw"), Some(vec![1, 2, 3]));
    }
}

#[cfg(test)]
mod sticker_tests {
    use super::*;
    use crate::model::{Content, Delivery, Media, MediaState};

    fn sticker(chat: &str, id: &str, timestamp: i64, path: Option<&str>) -> Message {
        Message {
            id: id.into(),
            chat: chat.into(),
            sender: chat.into(),
            sender_name: None,
            from_me: true,
            timestamp,
            content: Content::Sticker {
                media: Media {
                    mime: "image/webp".into(),
                    size: 10,
                    width: Some(512),
                    height: Some(512),
                    path: path.map(std::path::PathBuf::from),
                    state: MediaState::Idle,
                },
                animated: false,
            },
            status: Delivery::None,
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

    #[test]
    fn phone_stickers_keep_the_latest_use_and_their_file() {
        let archive = Archive::in_memory().expect("opens");
        archive
            .upsert_phone_sticker("aa", b"one", 100, 0.5)
            .expect("stored");
        archive
            .upsert_phone_sticker("bb", b"two", 300, 0.1)
            .expect("stored");
        // Older repeated use does not lower the last-used time.
        archive
            .upsert_phone_sticker("aa", b"one", 50, 0.9)
            .expect("stored");
        let list = archive.phone_stickers().expect("lists");
        assert_eq!(
            list.iter().map(|s| s.hash.as_str()).collect::<Vec<_>>(),
            ["bb", "aa"]
        );
        assert_eq!(list[1].last_used, 100);
        assert!(list.iter().all(|s| s.path.is_none()));
        archive
            .set_sticker_path("aa", Path::new("/tmp/aa.webp"))
            .expect("filed");
        let list = archive.phone_stickers().expect("lists");
        assert_eq!(list[1].path.as_deref(), Some(Path::new("/tmp/aa.webp")));
    }

    #[test]
    fn unfetched_stickers_are_listed_for_the_picker_and_fetched_ones_are_not() {
        let archive = Archive::in_memory().expect("opens");
        archive.ensure_chat("a@s.whatsapp.net", "A").expect("chat");
        archive
            .insert_message(&sticker("a@s.whatsapp.net", "s1", 10, None), Some(b"raw"))
            .expect("inserted");
        archive
            .insert_message(
                &sticker("a@s.whatsapp.net", "s2", 20, Some("/nowhere/s2.webp")),
                Some(b"raw"),
            )
            .expect("inserted");
        let missing = archive.stickers_without_file(10).expect("lists");
        assert_eq!(
            missing,
            vec![("a@s.whatsapp.net".to_owned(), "s1".to_owned())]
        );
        // Exclude missing local files.
        assert!(archive.recent_stickers(10).expect("lists").is_empty());
    }

    #[test]
    fn only_stickers_we_sent_are_recent() {
        let dir = tempfile::tempdir().expect("temp");
        let file = |name: &str| {
            let path = dir.path().join(name);
            std::fs::write(&path, b"webp").expect("writes");
            path.display().to_string()
        };
        let (sent, received) = (file("sent.webp"), file("received.webp"));
        let archive = Archive::in_memory().expect("opens");
        archive.ensure_chat("a@s.whatsapp.net", "A").expect("chat");
        archive
            .insert_message(&sticker("a@s.whatsapp.net", "s1", 10, Some(&sent)), None)
            .expect("inserted");
        let mut theirs = sticker("a@s.whatsapp.net", "s2", 20, Some(&received));
        theirs.from_me = false;
        archive.insert_message(&theirs, None).expect("inserted");
        let mut unfetched = sticker("a@s.whatsapp.net", "s3", 30, None);
        unfetched.from_me = false;
        archive
            .insert_message(&unfetched, Some(b"raw"))
            .expect("inserted");
        let recent: Vec<_> = archive
            .recent_stickers(10)
            .expect("lists")
            .into_iter()
            .map(|sticker| sticker.path.display().to_string())
            .collect();
        assert_eq!(recent, vec![sent], "a received sticker is not recent");
        assert!(
            archive.stickers_without_file(10).expect("lists").is_empty(),
            "a received sticker is not fetched for Recent"
        );
    }
}

#[cfg(test)]
mod media_path_tests {
    use super::*;
    use crate::model::{Content, Delivery, Media, MediaState};

    fn picture(id: &str) -> Message {
        Message {
            id: id.into(),
            chat: "a@s.whatsapp.net".into(),
            sender: "a@s.whatsapp.net".into(),
            sender_name: None,
            from_me: false,
            timestamp: 1,
            content: Content::Image {
                media: Media {
                    mime: "image/jpeg".into(),
                    size: 10,
                    width: None,
                    height: None,
                    path: None,
                    state: MediaState::Idle,
                },
                caption: None,
            },
            status: Delivery::None,
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

    #[test]
    fn attachment_paths_can_be_listed_moved_and_forgotten() {
        let archive = Archive::in_memory().expect("opens");
        archive.ensure_chat("a@s.whatsapp.net", "A").expect("chat");
        archive
            .insert_message(&picture("p1"), None)
            .expect("inserted");
        assert!(archive.media_paths().expect("lists").is_empty());
        archive
            .set_media_path("a@s.whatsapp.net", "p1", Path::new("/old/media/p1.jpg"))
            .expect("filed");
        let listed = archive.media_paths().expect("lists");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].2, Path::new("/old/media/p1.jpg"));
        archive
            .clear_media_path("a@s.whatsapp.net", "p1")
            .expect("cleared");
        assert!(archive.media_paths().expect("lists").is_empty());
    }
}
