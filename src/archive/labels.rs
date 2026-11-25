//! Chat labels: local labels created on this computer, and synced WhatsApp
//! labels/lists from the phone.

use rusqlite::params;

use super::{Archive, OptionalExtension, Result};
use crate::model::Label;

pub const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS local_labels (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    color TEXT NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS local_chat_labels (
    chat TEXT NOT NULL,
    label TEXT NOT NULL,
    PRIMARY KEY (chat, label)
);
CREATE INDEX IF NOT EXISTS local_chat_labels_by_label ON local_chat_labels (label);
CREATE TABLE IF NOT EXISTS synced_labels (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    color INTEGER,
    list_type INTEGER NOT NULL DEFAULT 0,
    predefined_id INTEGER,
    order_index INTEGER NOT NULL DEFAULT 0,
    is_active INTEGER NOT NULL DEFAULT 1,
    deleted INTEGER NOT NULL DEFAULT 0,
    updated_at INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS synced_chat_labels (
    chat TEXT NOT NULL,
    label TEXT NOT NULL,
    labeled INTEGER NOT NULL DEFAULT 1,
    updated_at INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (chat, label)
);
CREATE INDEX IF NOT EXISTS synced_chat_labels_by_label ON synced_chat_labels (label);
";

/// Prefix for synced WhatsApp label ids in [`Label::id`] and [`Chat::labels`],
/// keeping them disjoint from local `label-…` ids.
pub const SYNCED_PREFIX: &str = "wa:";

/// WhatsApp `LabelEditAction.ListType` values that ZapFast already renders as
/// built-in filter chips or rail folders (`UNREAD=1`, `GROUPS=2`, `FAVORITES=3`,
/// `CHANNELS=10`, `ARCHIVED=12`, `LOCKED=13`).
const BUILTIN_LIST_TYPES: &str = "(1, 2, 3, 10, 12, 13)";

/// WhatsApp Business label colour palette (indices 0..20).
const WA_LABEL_COLORS: [&str; 20] = [
    "#ff9485", "#64c4ff", "#ffd429", "#dfaeff", "#99b6c1", "#55ccb3", "#ff9dff", "#d3a91d",
    "#6d7cce", "#d7e752", "#00d0e2", "#ffc5c7", "#93ceac", "#f74848", "#00a0f2", "#83e422",
    "#ffaf04", "#b5ebff", "#9ba6ff", "#9368cf",
];

/// Maps a WhatsApp label colour index to a `#rrggbb` hex colour.
pub fn wa_label_color(index: i32) -> String {
    if index < 0 {
        return String::new();
    }
    WA_LABEL_COLORS[(index as usize) % WA_LABEL_COLORS.len()].to_owned()
}

/// Returns the raw WhatsApp label id when `id` refers to a synced label.
pub fn synced_label_raw_id(id: &str) -> Option<&str> {
    id.strip_prefix(SYNCED_PREFIX)
}

/// Most local labels kept, so the chip row and the chat menu stay short.
pub const LABEL_LIMIT: usize = 20;
/// Longest local label name kept, in characters.
pub const NAME_LIMIT: usize = 32;
/// Colour used when the given one is not a hex colour.
pub const DEFAULT_COLOR: &str = "#3b82f6";

/// Metadata for a synced WhatsApp label/list from `LabelEditUpdate`.
#[derive(Clone, Debug, Default)]
pub struct PhoneLabel<'a> {
    pub id: &'a str,
    pub name: Option<&'a str>,
    pub color: Option<i32>,
    pub list_type: Option<i32>,
    pub predefined_id: Option<i32>,
    pub order_index: Option<i32>,
    pub is_active: Option<bool>,
    pub deleted: bool,
    pub updated_at: i64,
}

impl Archive {
    /// Visible labels: synced WhatsApp tags/lists in phone order, followed by
    /// local labels in creation order.
    pub fn labels(&self) -> Result<Vec<Label>> {
        let mut out = Vec::new();
        {
            let sql = format!(
                "SELECT id, name, color, updated_at FROM synced_labels
                 WHERE deleted = 0 AND is_active = 1 AND trim(name) <> ''
                   AND list_type NOT IN {BUILTIN_LIST_TYPES}
                 ORDER BY order_index ASC, rowid ASC"
            );
            let mut statement = self.connection.prepare(&sql)?;
            let rows = statement.query_map([], |row| {
                let raw_id: String = row.get(0)?;
                let color: Option<i32> = row.get(2)?;
                Ok(Label {
                    id: format!("{SYNCED_PREFIX}{raw_id}"),
                    name: row.get(1)?,
                    color_hex: color.map(wa_label_color).unwrap_or_default(),
                    created_at: row.get(3)?,
                    synced: true,
                })
            })?;
            for row in rows {
                out.push(row?);
            }
        }
        {
            let mut statement = self.connection.prepare(
                "SELECT id, name, color, created_at FROM local_labels ORDER BY created_at ASC, rowid ASC",
            )?;
            let rows = statement.query_map([], |row| {
                Ok(Label {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    color_hex: row.get(2)?,
                    created_at: row.get(3)?,
                    synced: false,
                })
            })?;
            for row in rows {
                out.push(row?);
            }
        }
        Ok(out)
    }

    /// Creates a local label, unless the ceiling is reached or the name is taken.
    ///
    /// `created_at` is passed in so the caller owns the clock, and the id is
    /// derived from it, which keeps labels stable across restarts without a
    /// generated id in the archive.
    pub fn create_label(
        &self,
        name: &str,
        color_hex: &str,
        created_at: i64,
    ) -> Result<Option<Label>> {
        let name = clean_name(name);
        let local_count: i64 =
            self.connection
                .query_row("SELECT COUNT(*) FROM local_labels", [], |row| row.get(0))?;
        if name.is_empty() || local_count as usize >= LABEL_LIMIT {
            return Ok(None);
        }
        let taken: i64 = self.connection.query_row(
            "SELECT COUNT(*) FROM local_labels WHERE lower(name) = lower(?1)",
            params![name],
            |row| row.get(0),
        )?;
        if taken > 0 {
            return Ok(None);
        }
        let label = Label {
            id: self.label_id(created_at)?,
            name,
            color_hex: clean_color(color_hex),
            created_at,
            synced: false,
        };
        self.connection.execute(
            "INSERT INTO local_labels (id, name, color, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![label.id, label.name, label.color_hex, label.created_at],
        )?;
        Ok(Some(label))
    }

    /// Renames and recolours a label. Returns false when the name is taken.
    pub fn update_label(&self, id: &str, name: &str, color_hex: &str) -> Result<bool> {
        let name = clean_name(name);
        if name.is_empty() {
            return Ok(false);
        }
        let taken: i64 = self.connection.query_row(
            "SELECT COUNT(*) FROM local_labels WHERE lower(name) = lower(?1) AND id <> ?2",
            params![name, id],
            |row| row.get(0),
        )?;
        if taken > 0 {
            return Ok(false);
        }
        let changed = self.connection.execute(
            "UPDATE local_labels SET name = ?2, color = ?3 WHERE id = ?1",
            params![id, name, clean_color(color_hex)],
        )?;
        Ok(changed > 0)
    }

    /// Deletes a label and takes it off every chat that wore it.
    pub fn delete_label(&self, id: &str) -> Result<bool> {
        let transaction = super::Batch::begin(&self.connection)?;
        transaction.execute(
            "DELETE FROM local_chat_labels WHERE label = ?1",
            params![id],
        )?;
        let changed = transaction.execute("DELETE FROM local_labels WHERE id = ?1", params![id])?;
        transaction.commit()?;
        Ok(changed > 0)
    }

    /// Replaces the labels of one chat; a shorter list unassigns the rest.
    ///
    /// Ids of labels that no longer exist are skipped, so a menu drawn just
    /// before a delete cannot bring a label back onto a chat.
    pub fn set_chat_labels(&self, chat: &str, labels: &[String]) -> Result<()> {
        let transaction = super::Batch::begin(&self.connection)?;
        transaction.execute(
            "DELETE FROM local_chat_labels WHERE chat = ?1",
            params![chat],
        )?;
        for label in labels {
            transaction.execute(
                "INSERT OR IGNORE INTO local_chat_labels (chat, label)
                 SELECT ?1, id FROM local_labels WHERE id = ?2",
                params![chat, label],
            )?;
        }
        transaction.commit()
    }

    /// The label ids a chat wears (synced WhatsApp labels first, then local labels).
    pub fn chat_labels(&self, chat: &str) -> Result<Vec<String>> {
        let mut out = Vec::new();
        {
            let sql = format!(
                "SELECT synced_chat_labels.label FROM synced_chat_labels
                 JOIN synced_labels ON synced_labels.id = synced_chat_labels.label
                 WHERE synced_chat_labels.chat = ?1
                   AND synced_chat_labels.labeled = 1
                   AND synced_labels.deleted = 0
                   AND synced_labels.is_active = 1
                   AND trim(synced_labels.name) <> ''
                   AND synced_labels.list_type NOT IN {BUILTIN_LIST_TYPES}
                 ORDER BY synced_labels.order_index ASC, synced_labels.rowid ASC"
            );
            let mut statement = self.connection.prepare(&sql)?;
            let rows = statement.query_map(params![chat], |row| {
                let raw_id: String = row.get(0)?;
                Ok(format!("{SYNCED_PREFIX}{raw_id}"))
            })?;
            for row in rows {
                out.push(row?);
            }
        }
        {
            let mut statement = self.connection.prepare(
                "SELECT local_chat_labels.label FROM local_chat_labels
                 JOIN local_labels ON local_labels.id = local_chat_labels.label
                 WHERE local_chat_labels.chat = ?1
                 ORDER BY local_labels.created_at ASC, local_labels.rowid ASC",
            )?;
            let rows = statement.query_map(params![chat], |row| row.get::<_, String>(0))?;
            for row in rows {
                out.push(row?);
            }
        }
        Ok(out)
    }

    /// Applies a synced label/list definition from WhatsApp (`label_edit`).
    /// Older updates than the stored `updated_at` are ignored.
    /// Returns whether the label's stored state changed.
    pub fn apply_phone_label(&self, phone: &PhoneLabel<'_>) -> Result<bool> {
        let name = phone.name.map(str::trim).unwrap_or_default();
        let list_type = phone.list_type.unwrap_or(0);
        let order_index = phone.order_index.unwrap_or(0);
        let is_active = phone.is_active.unwrap_or(true);
        let changed = self.connection.execute(
            "INSERT INTO synced_labels
                (id, name, color, list_type, predefined_id, order_index, is_active, deleted, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(id) DO UPDATE SET
                name = CASE WHEN ?10 THEN excluded.name ELSE synced_labels.name END,
                color = COALESCE(excluded.color, synced_labels.color),
                list_type = COALESCE(?11, synced_labels.list_type),
                predefined_id = COALESCE(excluded.predefined_id, synced_labels.predefined_id),
                order_index = COALESCE(?12, synced_labels.order_index),
                is_active = COALESCE(?13, synced_labels.is_active),
                deleted = excluded.deleted,
                updated_at = excluded.updated_at
             WHERE excluded.updated_at >= synced_labels.updated_at",
            params![
                phone.id,
                name,
                phone.color,
                list_type,
                phone.predefined_id,
                order_index,
                i32::from(is_active),
                i32::from(phone.deleted),
                phone.updated_at,
                phone.name.is_some(),
                phone.list_type,
                phone.order_index,
                phone.is_active.map(i32::from),
            ],
        )?;
        Ok(changed > 0)
    }

    /// Applies a chat-label association from WhatsApp (`label_jid`) or a local
    /// toggle of a synced label. Older updates than the stored `updated_at`
    /// are ignored. Returns whether the effective `labeled` state changed.
    pub fn apply_phone_chat_label(
        &self,
        label_id: &str,
        chat: &str,
        labeled: bool,
        updated_at: i64,
    ) -> Result<bool> {
        let before: Option<(i32, i64)> = self
            .connection
            .query_row(
                "SELECT labeled, updated_at FROM synced_chat_labels WHERE chat = ?1 AND label = ?2",
                params![chat, label_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        if let Some((_, prev_at)) = before
            && updated_at < prev_at
        {
            return Ok(false);
        }
        self.connection.execute(
            "INSERT INTO synced_chat_labels (chat, label, labeled, updated_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(chat, label) DO UPDATE SET
                labeled = excluded.labeled,
                updated_at = excluded.updated_at
             WHERE excluded.updated_at >= synced_chat_labels.updated_at",
            params![chat, label_id, i32::from(labeled), updated_at],
        )?;
        let was_labeled = before.is_some_and(|(v, _)| v != 0);
        Ok(was_labeled != labeled)
    }

    /// An id that is free, derived from the creation time.
    fn label_id(&self, created_at: i64) -> Result<String> {
        for suffix in 0..1000 {
            let id = if suffix == 0 {
                format!("label-{created_at}")
            } else {
                format!("label-{created_at}-{suffix}")
            };
            let taken: i64 = self.connection.query_row(
                "SELECT COUNT(*) FROM local_labels WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )?;
            if taken == 0 {
                return Ok(id);
            }
        }
        Ok(format!("label-{created_at}-{}", self.labels()?.len()))
    }
}

/// Trims a label name and caps its length.
fn clean_name(name: &str) -> String {
    name.trim().chars().take(NAME_LIMIT).collect()
}

/// Normalises `#rgb`, `#rrggbb`, `rgb` and `rrggbb` to `#rrggbb`.
///
/// Anything else falls back to [`DEFAULT_COLOR`], so a stray paste cannot make
/// a label invisible.
pub fn clean_color(color: &str) -> String {
    let raw = color.trim().trim_start_matches('#');
    let expanded = match raw.len() {
        3 => raw.chars().flat_map(|c| [c, c]).collect::<String>(),
        6 => raw.to_owned(),
        _ => return DEFAULT_COLOR.to_owned(),
    };
    if !expanded.chars().all(|c| c.is_ascii_hexdigit()) {
        return DEFAULT_COLOR.to_owned();
    }
    format!("#{}", expanded.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn archive() -> Archive {
        Archive::in_memory().expect("opens")
    }

    #[test]
    fn labels_are_created_in_order_and_names_are_unique() {
        let archive = archive();
        let first = archive
            .create_label("Novos Clientes", "#FF8800", 10)
            .expect("create")
            .expect("created");
        assert_eq!(first.name, "Novos Clientes");
        assert_eq!(first.color_hex, "#ff8800", "colours normalise");
        assert_eq!(first.created_at, 10);
        let second = archive
            .create_label("Pagamento Pendente", "3b82f6", 20)
            .expect("create")
            .expect("created");
        assert_eq!(second.color_hex, "#3b82f6", "a bare hex is accepted");
        assert!(
            archive
                .create_label("novos clientes", "#000000", 30)
                .expect("create")
                .is_none(),
            "names are unique regardless of case"
        );
        assert!(
            archive
                .create_label("   ", "#000000", 40)
                .expect("create")
                .is_none()
        );
        assert_eq!(
            archive.labels().expect("labels").len(),
            2,
            "only the two good ones are stored"
        );
    }

    #[test]
    fn the_ceiling_is_twenty_labels() {
        let archive = archive();
        for index in 0..LABEL_LIMIT {
            assert!(
                archive
                    .create_label(&format!("Label {index}"), "#101010", index as i64)
                    .expect("create")
                    .is_some(),
                "label {index} fits"
            );
        }
        assert!(
            archive
                .create_label("One too many", "#101010", 99)
                .expect("create")
                .is_none(),
            "the twenty-first label is refused"
        );
        assert_eq!(archive.labels().expect("labels").len(), LABEL_LIMIT);
    }

    #[test]
    fn a_chat_can_wear_several_labels_and_deleting_one_takes_it_off() {
        let archive = archive();
        archive
            .ensure_chat("1@s.whatsapp.net", "Ana")
            .expect("chat");
        archive
            .ensure_chat("2@s.whatsapp.net", "Bia")
            .expect("chat");
        let work = archive
            .create_label("Trabalho", "#111111", 1)
            .expect("create")
            .expect("created");
        let money = archive
            .create_label("Dinheiro", "#222222", 2)
            .expect("create")
            .expect("created");
        archive
            .set_chat_labels("1@s.whatsapp.net", &[work.id.clone(), money.id.clone()])
            .expect("assign");
        archive
            .set_chat_labels("2@s.whatsapp.net", std::slice::from_ref(&money.id))
            .expect("assign");
        assert_eq!(
            archive.chat_labels("1@s.whatsapp.net").expect("labels"),
            vec![work.id.clone(), money.id.clone()]
        );
        assert_eq!(
            archive.chat_labels("2@s.whatsapp.net").expect("labels"),
            vec![money.id.clone()],
            "the money label is on both chats"
        );

        // Unassigning is assigning a shorter list.
        archive
            .set_chat_labels("1@s.whatsapp.net", std::slice::from_ref(&work.id))
            .expect("assign");
        assert_eq!(
            archive.chat_labels("1@s.whatsapp.net").expect("labels"),
            vec![work.id.clone()]
        );
        assert!(archive.delete_label(&money.id).expect("delete"));
        assert!(
            archive
                .chat_labels("2@s.whatsapp.net")
                .expect("labels")
                .is_empty(),
            "deleting a label takes it off every chat"
        );
        assert_eq!(archive.labels().expect("labels").len(), 1);
    }

    #[test]
    fn labels_made_in_the_same_second_keep_their_order() {
        let archive = archive();
        for name in ["Zeta", "Alpha", "Mid"] {
            archive
                .create_label(name, "#111111", 7)
                .expect("create")
                .expect("created");
        }
        let names: Vec<String> = archive
            .labels()
            .expect("labels")
            .into_iter()
            .map(|label| label.name)
            .collect();
        assert_eq!(names, ["Zeta", "Alpha", "Mid"]);
    }

    #[test]
    fn a_deleted_label_cannot_be_put_back_on_a_chat() {
        let archive = archive();
        archive
            .ensure_chat("1@s.whatsapp.net", "Ana")
            .expect("chat");
        archive
            .set_chat_labels("1@s.whatsapp.net", &["label-gone".to_owned()])
            .expect("assign");
        let stored: i64 = archive
            .connection
            .query_row("SELECT COUNT(*) FROM local_chat_labels", [], |row| {
                row.get(0)
            })
            .expect("count");
        assert_eq!(stored, 0);
    }

    #[test]
    fn colours_fall_back_to_the_default() {
        assert_eq!(clean_color("#abc"), "#aabbcc");
        assert_eq!(clean_color("AABBCC"), "#aabbcc");
        assert_eq!(clean_color(""), DEFAULT_COLOR);
        assert_eq!(clean_color("not a colour"), DEFAULT_COLOR);
        assert_eq!(clean_color("#12345"), DEFAULT_COLOR);
    }

    #[test]
    fn synced_labels_follow_order_filter_builtins_and_respect_timestamps() {
        let archive = archive();
        archive
            .ensure_chat("1@s.whatsapp.net", "Ana")
            .expect("chat");
        // Custom list (type 5) with order_index 2.
        assert!(
            archive
                .apply_phone_label(&PhoneLabel {
                    id: "10",
                    name: Some("Distributors"),
                    list_type: Some(5),
                    order_index: Some(2),
                    is_active: Some(true),
                    updated_at: 100,
                    ..Default::default()
                })
                .expect("apply")
        );
        // Business label (type 0 / None) with colour and order_index 1.
        assert!(
            archive
                .apply_phone_label(&PhoneLabel {
                    id: "2",
                    name: Some("VIP"),
                    color: Some(1),
                    order_index: Some(1),
                    updated_at: 100,
                    ..Default::default()
                })
                .expect("apply")
        );
        // Built-in Unread (type 1) and Favorites (type 3) must not duplicate built-in chips.
        archive
            .apply_phone_label(&PhoneLabel {
                id: "1",
                name: Some("Unread"),
                list_type: Some(1),
                order_index: Some(0),
                is_active: Some(true),
                updated_at: 100,
                ..Default::default()
            })
            .expect("apply");
        // Inactive list must not show.
        archive
            .apply_phone_label(&PhoneLabel {
                id: "9",
                name: Some("Hidden"),
                list_type: Some(5),
                is_active: Some(false),
                updated_at: 100,
                ..Default::default()
            })
            .expect("apply");

        let labels = archive.labels().expect("labels");
        assert_eq!(labels.len(), 2);
        assert_eq!(labels[0].id, "wa:2");
        assert_eq!(labels[0].name, "VIP");
        assert_eq!(labels[0].color_hex, wa_label_color(1));
        assert!(labels[0].synced);
        assert_eq!(labels[1].id, "wa:10");
        assert_eq!(labels[1].name, "Distributors");
        assert_eq!(labels[1].color_hex, "");
        assert!(labels[1].synced);

        // Chat associations: newer wins, older replay is ignored.
        assert!(
            archive
                .apply_phone_chat_label("10", "1@s.whatsapp.net", true, 200)
                .expect("assoc")
        );
        assert!(
            !archive
                .apply_phone_chat_label("10", "1@s.whatsapp.net", false, 150)
                .expect("stale assoc ignored")
        );
        assert_eq!(
            archive
                .chat_labels("1@s.whatsapp.net")
                .expect("chat labels"),
            vec!["wa:10"]
        );

        // Deleting a synced label hides it and its chat association, and an older replay cannot resurrect it.
        assert!(
            archive
                .apply_phone_label(&PhoneLabel {
                    id: "10",
                    deleted: true,
                    updated_at: 300,
                    ..Default::default()
                })
                .expect("delete")
        );
        assert!(
            !archive
                .apply_phone_label(&PhoneLabel {
                    id: "10",
                    name: Some("Distributors"),
                    list_type: Some(5),
                    updated_at: 250,
                    ..Default::default()
                })
                .expect("stale resurrect ignored")
        );
        assert_eq!(archive.labels().expect("labels").len(), 1);
        assert!(
            archive
                .chat_labels("1@s.whatsapp.net")
                .expect("chat labels")
                .is_empty()
        );
    }
}
