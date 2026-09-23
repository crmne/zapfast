//! Sticker state synchronized with the phone.
//!
//! Recent stickers come from the phone's own list and from stickers we sent.
//! Removing one from Recent, here or on the phone, is remembered by content
//! hash with the time it happened, so a later send of the same sticker brings
//! it back while older sends stay hidden.
//!
//! Favorite sticker files live with the user's data, not here. This table
//! only tracks each favorite's sync with the phone: whether it is a favorite,
//! when that last changed on either side, the CDN references the phone needs
//! to fetch it, and whether the phone has been told.

use std::collections::HashMap;

use super::{Archive, Result, params};

pub const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS removed_recent_stickers (
    hash TEXT PRIMARY KEY,
    removed_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS favorite_stickers (
    hash TEXT PRIMARY KEY,
    favorite INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    action BLOB,
    pushed INTEGER NOT NULL DEFAULT 0
);
";

/// One favorite sticker's sync state.
#[derive(Clone, Debug, PartialEq)]
pub struct FavoriteSticker {
    pub favorite: bool,
    /// When it last changed, in Unix milliseconds.
    pub updated_at: i64,
    /// The encoded `StickerAction` with the sticker's CDN references.
    pub action: Option<Vec<u8>>,
    /// Whether the phone has this state.
    pub pushed: bool,
}

impl Archive {
    /// Hides a sticker from Recent for every use up to `at` (Unix seconds).
    pub fn remove_recent_sticker(&self, hash: &str, at: i64) -> Result<()> {
        self.connection.execute(
            "DELETE FROM stickers WHERE hash = ?1 AND last_used <= ?2",
            params![hash, at],
        )?;
        self.connection.execute(
            "INSERT INTO removed_recent_stickers (hash, removed_at) VALUES (?1, ?2)
             ON CONFLICT(hash) DO UPDATE SET
                 removed_at = MAX(removed_recent_stickers.removed_at, excluded.removed_at)",
            params![hash, at],
        )?;
        Ok(())
    }

    /// When each sticker was last removed from Recent, by content hash.
    pub fn removed_recent_stickers(&self) -> Result<HashMap<String, i64>> {
        let mut statement = self
            .connection
            .prepare("SELECT hash, removed_at FROM removed_recent_stickers")?;
        let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        rows.collect()
    }

    /// A sticker's favorite sync state, by content hash.
    pub fn favorite_sticker(&self, hash: &str) -> Result<Option<FavoriteSticker>> {
        use rusqlite::OptionalExtension;
        self.connection
            .query_row(
                "SELECT favorite, updated_at, action, pushed FROM favorite_stickers WHERE hash = ?1",
                params![hash],
                |row| {
                    Ok(FavoriteSticker {
                        favorite: row.get(0)?,
                        updated_at: row.get(1)?,
                        action: row.get(2)?,
                        pushed: row.get(3)?,
                    })
                },
            )
            .optional()
    }

    /// Records a favorite change. A missing `action` keeps the references
    /// already known, since removing a favorite does not carry them.
    pub fn set_favorite_sticker(
        &self,
        hash: &str,
        favorite: bool,
        updated_at: i64,
        action: Option<&[u8]>,
        pushed: bool,
    ) -> Result<()> {
        self.connection.execute(
            "INSERT INTO favorite_stickers (hash, favorite, updated_at, action, pushed)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(hash) DO UPDATE SET
                 favorite = excluded.favorite,
                 updated_at = excluded.updated_at,
                 action = COALESCE(excluded.action, favorite_stickers.action),
                 pushed = excluded.pushed",
            params![hash, favorite, updated_at, action, pushed],
        )?;
        Ok(())
    }

    /// Favorite changes the phone has not been told about yet.
    pub fn unpushed_favorite_stickers(&self) -> Result<Vec<(String, FavoriteSticker)>> {
        let mut statement = self.connection.prepare(
            "SELECT hash, favorite, updated_at, action, pushed FROM favorite_stickers
             WHERE pushed = 0 ORDER BY updated_at",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get(0)?,
                FavoriteSticker {
                    favorite: row.get(1)?,
                    updated_at: row.get(2)?,
                    action: row.get(3)?,
                    pushed: row.get(4)?,
                },
            ))
        })?;
        rows.collect()
    }

    /// Raw sticker messages, newest first, to find a sticker's CDN references.
    pub fn sticker_message_raws(&self, limit: usize) -> Result<Vec<Vec<u8>>> {
        let mut statement = self.connection.prepare(
            "SELECT raw FROM messages
             WHERE json_extract(content, '$.kind') = 'sticker' AND raw IS NOT NULL
             ORDER BY timestamp DESC LIMIT ?1",
        )?;
        let rows = statement.query_map(params![limit as i64], |row| row.get(0))?;
        rows.collect()
    }

    /// Marks a change as delivered to the phone, unless a newer one replaced
    /// it meanwhile, and keeps the references it was sent with.
    pub fn favorite_sticker_pushed(
        &self,
        hash: &str,
        updated_at: i64,
        action: Option<&[u8]>,
    ) -> Result<()> {
        self.connection.execute(
            "UPDATE favorite_stickers SET pushed = 1, action = COALESCE(?3, action)
             WHERE hash = ?1 AND updated_at = ?2",
            params![hash, updated_at, action],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_removed_recent_sticker_stays_hidden_until_it_is_used_again() {
        let archive = Archive::in_memory().expect("opens");
        archive
            .upsert_phone_sticker("aa", b"one", 100, 0.5)
            .expect("stores");
        archive
            .upsert_phone_sticker("bb", b"two", 300, 0.5)
            .expect("stores");
        archive.remove_recent_sticker("aa", 200).expect("removes");
        // A removal older than the latest use leaves the phone's entry.
        archive.remove_recent_sticker("bb", 200).expect("removes");
        let phone: Vec<String> = archive
            .phone_stickers()
            .expect("lists")
            .into_iter()
            .map(|sticker| sticker.hash)
            .collect();
        assert_eq!(phone, vec!["bb"]);
        archive.remove_recent_sticker("aa", 150).expect("removes");
        assert_eq!(
            archive.removed_recent_stickers().expect("lists")["aa"],
            200,
            "an older removal does not move the mark back"
        );
    }

    #[test]
    fn a_favorite_keeps_its_references_and_waits_for_the_phone() {
        let archive = Archive::in_memory().expect("opens");
        assert!(archive.favorite_sticker("aa").expect("reads").is_none());
        archive
            .set_favorite_sticker("aa", true, 10, None, false)
            .expect("stores");
        archive
            .favorite_sticker_pushed("aa", 10, Some(b"refs"))
            .expect("marks");
        let stored = archive.favorite_sticker("aa").expect("reads").expect("row");
        assert!(stored.favorite && stored.pushed);
        assert_eq!(stored.action.as_deref(), Some(&b"refs"[..]));
        // Removing it keeps the references for the next time.
        archive
            .set_favorite_sticker("aa", false, 20, None, false)
            .expect("stores");
        // A push of the older change arriving late does not claim the newer.
        archive
            .favorite_sticker_pushed("aa", 10, None)
            .expect("marks");
        let stored = archive.favorite_sticker("aa").expect("reads").expect("row");
        assert!(!stored.favorite && !stored.pushed);
        let waiting = archive.unpushed_favorite_stickers().expect("lists");
        assert_eq!(waiting.len(), 1);
        assert_eq!(waiting[0].0, "aa");
        assert_eq!(stored.action.as_deref(), Some(&b"refs"[..]));
        archive.clear().expect("clears");
        assert!(archive.favorite_sticker("aa").expect("reads").is_none());
    }
}
