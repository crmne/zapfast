//! User-defined sticker groups: named folders the picker can filter by.
//!
//! A group stores sticker files by an opaque key the backend derives from the
//! file's location, never by content, so deleting a group or a membership
//! leaves every sticker file exactly where it was.

use rusqlite::{OptionalExtension, params};

use super::{Archive, Result};

pub const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS sticker_groups (
    name TEXT PRIMARY KEY,
    created_at INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS sticker_group_stickers (
    group_name TEXT NOT NULL,
    sticker TEXT NOT NULL,
    PRIMARY KEY (group_name, sticker)
);
CREATE TRIGGER IF NOT EXISTS delete_sticker_group_stickers
AFTER DELETE ON sticker_groups BEGIN
    DELETE FROM sticker_group_stickers WHERE group_name = OLD.name;
END;
";

impl Archive {
    /// Every group with its stickers: groups oldest first, stickers in the
    /// order they were added.
    pub fn sticker_groups(&self) -> Result<Vec<(String, Vec<String>)>> {
        let mut statement = self
            .connection
            .prepare("SELECT name FROM sticker_groups ORDER BY created_at, name")?;
        let names = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<String>>>()?;
        let mut groups = Vec::with_capacity(names.len());
        for name in names {
            let stickers = self.sticker_group_stickers(&name)?;
            groups.push((name, stickers));
        }
        Ok(groups)
    }

    /// Sticker keys of one group, in the order they were added.
    pub fn sticker_group_stickers(&self, group: &str) -> Result<Vec<String>> {
        let mut statement = self.connection.prepare(
            "SELECT sticker FROM sticker_group_stickers WHERE group_name = ?1 ORDER BY rowid",
        )?;
        statement
            .query_map(params![group], |row| row.get(0))?
            .collect()
    }

    /// Creates a group. The name is the group's identity, so creating an
    /// existing name keeps that group instead of replacing or duplicating it.
    pub fn create_sticker_group(&self, name: &str, at: i64) -> Result<()> {
        self.connection.execute(
            "INSERT INTO sticker_groups (name, created_at) VALUES (?1, ?2)
             ON CONFLICT(name) DO NOTHING",
            params![name, at],
        )?;
        Ok(())
    }

    /// Deletes a group and its memberships. The sticker files stay on disk.
    pub fn delete_sticker_group(&self, name: &str) -> Result<()> {
        self.connection
            .execute("DELETE FROM sticker_groups WHERE name = ?1", params![name])?;
        Ok(())
    }

    /// Adds or removes one sticker in one group. A group that does not exist
    /// cannot gain memberships, so a stale menu cannot create an orphan.
    pub fn set_sticker_group(&self, group: &str, sticker: &str, member: bool) -> Result<()> {
        if member {
            self.connection.execute(
                "INSERT INTO sticker_group_stickers (group_name, sticker)
                 SELECT ?1, ?2 WHERE EXISTS (SELECT 1 FROM sticker_groups WHERE name = ?1)
                 ON CONFLICT(group_name, sticker) DO NOTHING",
                params![group, sticker],
            )?;
        } else {
            self.connection.execute(
                "DELETE FROM sticker_group_stickers WHERE group_name = ?1 AND sticker = ?2",
                params![group, sticker],
            )?;
        }
        Ok(())
    }

    /// Whether one sticker belongs to one group.
    pub fn sticker_group_member(&self, group: &str, sticker: &str) -> Result<bool> {
        self.connection
            .query_row(
                "SELECT 1 FROM sticker_group_stickers WHERE group_name = ?1 AND sticker = ?2",
                params![group, sticker],
                |_| Ok(()),
            )
            .optional()
            .map(|found| found.is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn archive() -> Archive {
        Archive::in_memory().expect("opens")
    }

    #[test]
    fn groups_list_in_creation_order_with_their_stickers_in_add_order() {
        let archive = archive();
        archive.create_sticker_group("Bom dia", 20).expect("group");
        archive.create_sticker_group("Futebol", 10).expect("group");
        archive
            .set_sticker_group("Futebol", "aa.webp", true)
            .expect("add");
        archive
            .set_sticker_group("Futebol", "bb.webp", true)
            .expect("add");
        archive
            .set_sticker_group("Bom dia", "cc.webp", true)
            .expect("add");
        assert_eq!(
            archive.sticker_groups().expect("lists"),
            vec![
                (
                    "Futebol".to_owned(),
                    vec!["aa.webp".to_owned(), "bb.webp".to_owned()]
                ),
                ("Bom dia".to_owned(), vec!["cc.webp".to_owned()]),
            ]
        );
        assert!(
            archive
                .sticker_group_member("Futebol", "bb.webp")
                .expect("member")
        );
        assert!(
            !archive
                .sticker_group_member("Futebol", "cc.webp")
                .expect("member")
        );
    }

    #[test]
    fn adding_the_same_sticker_twice_keeps_one_membership() {
        let archive = archive();
        archive.create_sticker_group("Trabalho", 10).expect("group");
        for _ in 0..2 {
            archive
                .set_sticker_group("Trabalho", "aa.webp", true)
                .expect("add");
        }
        assert_eq!(
            archive.sticker_group_stickers("Trabalho").expect("list"),
            vec!["aa.webp".to_owned()]
        );
    }

    #[test]
    fn deleting_a_group_takes_its_memberships_and_leaves_the_others() {
        let archive = archive();
        archive.create_sticker_group("Bom dia", 10).expect("group");
        archive.create_sticker_group("Trabalho", 20).expect("group");
        archive
            .set_sticker_group("Bom dia", "aa.webp", true)
            .expect("add");
        archive
            .set_sticker_group("Trabalho", "aa.webp", true)
            .expect("add");
        archive.delete_sticker_group("Bom dia").expect("delete");
        assert_eq!(
            archive.sticker_groups().expect("lists"),
            vec![("Trabalho".to_owned(), vec!["aa.webp".to_owned()])]
        );
        assert!(
            !archive
                .sticker_group_member("Bom dia", "aa.webp")
                .expect("member")
        );
    }

    #[test]
    fn a_sticker_can_leave_one_group_without_leaving_the_other() {
        let archive = archive();
        archive.create_sticker_group("Bom dia", 10).expect("group");
        archive.create_sticker_group("Trabalho", 20).expect("group");
        for group in ["Bom dia", "Trabalho"] {
            archive
                .set_sticker_group(group, "aa.webp", true)
                .expect("add");
        }
        archive
            .set_sticker_group("Bom dia", "aa.webp", false)
            .expect("remove");
        assert!(
            archive
                .sticker_group_stickers("Bom dia")
                .expect("list")
                .is_empty()
        );
        assert_eq!(
            archive.sticker_group_stickers("Trabalho").expect("list"),
            vec!["aa.webp".to_owned()]
        );
    }

    #[test]
    fn the_same_name_reuses_the_group_it_already_names() {
        let archive = archive();
        archive.create_sticker_group("Futebol", 20).expect("group");
        archive
            .set_sticker_group("Futebol", "aa.webp", true)
            .expect("add");
        archive.create_sticker_group("Futebol", 10).expect("group");
        assert_eq!(
            archive.sticker_groups().expect("lists"),
            vec![("Futebol".to_owned(), vec!["aa.webp".to_owned()])]
        );
    }

    #[test]
    fn a_group_that_does_not_exist_cannot_gain_memberships() {
        let archive = archive();
        archive
            .set_sticker_group("Ghost", "aa.webp", true)
            .expect("add");
        assert!(archive.sticker_groups().expect("lists").is_empty());
    }

    #[test]
    fn clearing_the_archive_drops_every_group() {
        let archive = archive();
        archive.create_sticker_group("Bom dia", 10).expect("group");
        archive
            .set_sticker_group("Bom dia", "aa.webp", true)
            .expect("add");
        archive.clear().expect("clears");
        assert!(archive.sticker_groups().expect("lists").is_empty());
        assert!(
            archive
                .sticker_group_stickers("Bom dia")
                .expect("list")
                .is_empty()
        );
    }
}
