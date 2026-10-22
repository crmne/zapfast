//! Message metadata follows a chat's privacy id into its phone-number chat.

use rusqlite::{Transaction, params};

use super::{Archive, Result};

impl Archive {
    /// Called after messages have moved, within the mapping's transaction.
    /// Newer marks win; equal timestamps keep the canonical chat's state.
    /// A missing target is allowed only when its source message is also absent
    /// (metadata may arrive first), never when a removal excluded the source.
    pub(super) fn move_message_metadata(
        transaction: &Transaction<'_>,
        source: &str,
        target: &str,
    ) -> Result<()> {
        transaction.execute(
            "INSERT INTO stars (chat, id, starred, starred_at)
             SELECT ?2, s.id, s.starred, s.starred_at FROM stars s
             LEFT JOIN messages original ON original.chat = ?1 AND original.id = s.id
             LEFT JOIN messages current ON current.chat = ?2 AND current.id = s.id
             WHERE s.chat = ?1 AND (s.starred = 0 OR (
                (original.id IS NULL OR current.id IS NOT NULL)
                AND NOT EXISTS (SELECT 1 FROM message_removals r WHERE r.chat = ?2 AND r.id = s.id
                    AND r.account = COALESCE((SELECT value FROM meta WHERE key = 'me_pn'), ''))
                AND (current.id IS NULL OR json_extract(current.content, '$.kind') <> 'revoked')))
             ON CONFLICT(chat, id) DO UPDATE SET
                starred = excluded.starred, starred_at = excluded.starred_at
             WHERE excluded.starred_at > stars.starred_at",
            params![source, target],
        )?;
        transaction.execute(
            "INSERT INTO message_pins (chat, id, pinned, pinned_at, expires_at, by_me, sender)
             SELECT ?2, p.id, p.pinned, p.pinned_at, p.expires_at, p.by_me, p.sender FROM message_pins p
             LEFT JOIN messages original ON original.chat = ?1 AND original.id = p.id
             LEFT JOIN messages current ON current.chat = ?2 AND current.id = p.id
             WHERE p.chat = ?1 AND (p.pinned = 0 OR (
                (original.id IS NULL OR current.id IS NOT NULL)
                AND NOT EXISTS (SELECT 1 FROM message_removals r WHERE r.chat = ?2 AND r.id = p.id
                    AND r.account = COALESCE((SELECT value FROM meta WHERE key = 'me_pn'), ''))
                AND (current.id IS NULL OR json_extract(current.content, '$.kind') <> 'revoked')))
             ON CONFLICT(chat, id) DO UPDATE SET
                pinned = excluded.pinned, pinned_at = excluded.pinned_at,
                expires_at = excluded.expires_at, by_me = excluded.by_me, sender = excluded.sender
             WHERE excluded.pinned_at > message_pins.pinned_at",
            params![source, target],
        )?;
        // The old chat's normal purge does not include message_pins.
        transaction.execute("DELETE FROM message_pins WHERE chat = ?1", [source])?;
        transaction.execute(
            "INSERT OR IGNORE INTO motion_clips (chat, parent, sender, raw)
             SELECT ?2, c.parent,
                CASE WHEN c.sender IN (?1, ?2) AND current.sender IN (?1, ?2)
                    THEN current.sender
                    WHEN c.sender = ?1 AND current.id IS NULL THEN ?2
                    ELSE c.sender END, c.raw
             FROM motion_clips c
             LEFT JOIN messages original ON original.chat = ?1 AND original.id = c.parent
             LEFT JOIN messages current ON current.chat = ?2 AND current.id = c.parent
             WHERE c.chat = ?1 AND (original.id IS NULL OR current.id IS NOT NULL)
                AND NOT EXISTS (SELECT 1 FROM message_removals r WHERE r.chat = ?2 AND r.id = c.parent
                    AND r.account = COALESCE((SELECT value FROM meta WHERE key = 'me_pn'), ''))
                AND (current.id IS NULL OR json_extract(current.content, '$.kind') = 'image')",
            params![source, target],
        )?;
        // A canonical copy may predate the clip. Mark it only when the clip's
        // sender is the photo's sender (the two mapped ids are equivalent).
        transaction.execute(
            "UPDATE messages SET content = json_set(content, '$.motion', json('{}'))
             WHERE chat = ?1 AND json_valid(content)
                AND json_extract(content, '$.kind') = 'image'
                AND json_extract(content, '$.motion') IS NULL
                AND EXISTS (SELECT 1 FROM motion_clips c WHERE c.chat = messages.chat
                    AND c.parent = messages.id AND c.sender = messages.sender)",
            [target],
        )?;
        Ok(())
    }
}
