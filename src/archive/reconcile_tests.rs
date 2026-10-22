//! Synthetic regressions for message metadata when a privacy id is learned.

use std::collections::HashSet;

use super::tests::message;
use super::*;

const LID: &str = "9@lid";
const PHONE: &str = "1@s.whatsapp.net";

#[test]
fn privacy_mapping_keeps_stars_and_the_latest_unstar() {
    for existing in [false, true] {
        let archive = Archive::in_memory().unwrap();
        archive.ensure_chat(LID, "Fixture").unwrap();
        if existing {
            archive.ensure_chat(PHONE, "Fixture").unwrap();
        }
        archive
            .insert_message(&message(LID, "kept", 100, false), None)
            .unwrap();
        archive.star(LID, "kept", 200).unwrap();
        archive.star(LID, "unstarred-on-lid", 100).unwrap();
        archive.unstar(LID, "unstarred-on-lid", 300).unwrap();
        archive.star(PHONE, "unstarred-on-lid", 200).unwrap();
        archive.star(LID, "unstarred-on-phone", 100).unwrap();
        archive.unstar(PHONE, "unstarred-on-phone", 300).unwrap();
        archive.star(LID, "same-second", 300).unwrap();
        archive.unstar(PHONE, "same-second", 300).unwrap();

        archive.put_lid("9", "1").unwrap();

        assert_eq!(
            archive.starred_ids(PHONE).unwrap(),
            HashSet::from(["kept".into()])
        );
        assert_eq!(archive.starred(50).unwrap()[0].message.id, "kept");
        assert!(archive.starred_ids(LID).unwrap().is_empty());
        for id in ["unstarred-on-lid", "unstarred-on-phone", "same-second"] {
            assert!(!archive.set_star(PHONE, id, true, 300, true).unwrap());
        }
        archive.put_lid("9", "1").unwrap();
        assert_eq!(
            archive.starred_ids(PHONE).unwrap(),
            HashSet::from(["kept".into()])
        );
    }
}

#[test]
fn privacy_mapping_keeps_pins_and_the_latest_unpin() {
    let archive = Archive::in_memory().unwrap();
    archive.ensure_chat(LID, "Fixture").unwrap();
    let pinner = Pinner {
        by_me: false,
        sender: "2@s.whatsapp.net".into(),
    };
    archive
        .insert_message(&message(LID, "kept", 100, false), None)
        .unwrap();
    archive.pin_from(LID, "kept", 200, 1000, &pinner).unwrap();
    archive.unpin(LID, "unpin-on-lid", 300).unwrap();
    archive.pin(PHONE, "unpin-on-lid", 100, 1000).unwrap();
    archive.pin(LID, "unpin-on-phone", 100, 1000).unwrap();
    archive.unpin(PHONE, "unpin-on-phone", 300).unwrap();
    archive.pin(LID, "same-second", 300, 1000).unwrap();
    archive.unpin(PHONE, "same-second", 300).unwrap();

    archive.put_lid("9", "1").unwrap();

    assert_eq!(
        archive.pinned_ids(PHONE, 500).unwrap(),
        HashSet::from(["kept".into()])
    );
    let pins = archive.chat_pins(PHONE, 500).unwrap();
    assert_eq!(pins.len(), 1);
    assert_eq!(pins[0].pinner, pinner);
    assert_eq!(pins[0].pinned_at, 200);
    assert_eq!(pins[0].expires_at, 1000);
    assert!(archive.pinned_ids(LID, 500).unwrap().is_empty());
    for id in ["unpin-on-lid", "unpin-on-phone", "same-second"] {
        assert!(!archive.pin(PHONE, id, 200, 1000).unwrap());
    }
    assert!(archive.pinned_ids(PHONE, 1000).unwrap().is_empty());
}

fn photo(chat: &str, id: &str) -> Message {
    let mut photo = message(chat, id, 100, false);
    photo.content = Content::Image {
        motion: None,
        caption: None,
        media: crate::model::Media {
            mime: "image/jpeg".into(),
            size: 1,
            width: None,
            height: None,
            album: None,
            path: None,
            state: crate::model::MediaState::Idle,
        },
    };
    photo
}

#[test]
fn privacy_mapping_keeps_motion_clips_with_their_photos() {
    for existing in 0..3 {
        let archive = Archive::in_memory().unwrap();
        archive.ensure_chat(LID, "Fixture").unwrap();
        archive.insert_message(&photo(LID, "photo"), None).unwrap();
        archive.put_motion_clip(LID, "photo", LID, b"clip").unwrap();
        archive
            .put_motion_clip(LID, "later", LID, b"early")
            .unwrap();
        archive
            .put_motion_clip(LID, "wrong-sender", "other@lid", b"wrong")
            .unwrap();
        if existing > 0 {
            archive.ensure_chat(PHONE, "Fixture").unwrap();
            archive
                .insert_message(&photo(PHONE, "photo"), None)
                .unwrap();
            if existing == 2 {
                archive
                    .put_motion_clip(PHONE, "photo", PHONE, b"canonical")
                    .unwrap();
            }
        }

        archive.put_lid("9", "1").unwrap();

        assert_eq!(
            archive.motion_clip(PHONE, "photo").unwrap().as_deref(),
            Some(if existing == 2 {
                &b"canonical"[..]
            } else {
                &b"clip"[..]
            })
        );
        assert!(matches!(
            archive.message(PHONE, "photo").unwrap().unwrap().content,
            Content::Image {
                motion: Some(_),
                ..
            }
        ));
        // A clip may arrive before its photo. Keep the same sender check.
        archive
            .insert_message(&photo(PHONE, "later"), None)
            .unwrap();
        assert_eq!(
            archive.motion_clip(PHONE, "later").unwrap().as_deref(),
            Some(&b"early"[..])
        );
        archive
            .insert_message(&photo(PHONE, "wrong-sender"), None)
            .unwrap();
        assert!(
            archive
                .motion_clip(PHONE, "wrong-sender")
                .unwrap()
                .is_none()
        );
        assert!(archive.motion_clip(LID, "photo").unwrap().is_none());
    }
}

#[test]
fn privacy_mapping_does_not_restore_metadata_of_removed_photos() {
    let archive = Archive::in_memory().unwrap();
    archive.ensure_chat(LID, "Fixture").unwrap();
    archive.ensure_chat(PHONE, "Fixture").unwrap();
    archive.remove_chat_through(PHONE, 100, false).unwrap();
    for (id, at) in [("cleared", 100), ("deleted", 200), ("revoked", 200)] {
        let mut row = photo(LID, id);
        row.timestamp = at;
        archive.insert_message(&row, None).unwrap();
        archive.pin(LID, id, 300, 1000).unwrap();
        archive.put_motion_clip(LID, id, LID, b"clip").unwrap();
    }
    archive.delete_message(PHONE, "deleted").unwrap();
    let mut revoked = photo(PHONE, "revoked");
    revoked.timestamp = 200;
    archive.insert_message(&revoked, None).unwrap();
    archive
        .set_content(PHONE, "revoked", &Content::Revoked, false)
        .unwrap();

    archive.put_lid("9", "1").unwrap();

    assert!(archive.pinned_ids(PHONE, 500).unwrap().is_empty());
    for id in ["cleared", "deleted", "revoked"] {
        assert!(archive.motion_clip(PHONE, id).unwrap().is_none());
    }
}

#[test]
fn privacy_mapping_moves_metadata_atomically_and_survives_restart() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("fixture.db");
    let key = [21; 32];
    {
        let archive = Archive::open_with_key(&path, &key).unwrap();
        archive.ensure_chat(LID, "Fixture").unwrap();
        archive.insert_message(&photo(LID, "photo"), None).unwrap();
        archive.star(LID, "photo", 200).unwrap();
        archive.pin(LID, "photo", 200, 1000).unwrap();
        archive.put_motion_clip(LID, "photo", LID, b"clip").unwrap();
        archive
            .connection
            .execute_batch(
                "CREATE TRIGGER reject_merge BEFORE DELETE ON messages
             BEGIN SELECT RAISE(ABORT, 'fixture failure'); END;",
            )
            .unwrap();

        assert!(archive.put_lid("9", "1").is_err());
        assert!(archive.lids().unwrap().is_empty());
        assert!(archive.chat(PHONE).unwrap().is_none());
        assert!(archive.starred_ids(LID).unwrap().contains("photo"));
        assert!(archive.pinned_ids(LID, 500).unwrap().contains("photo"));
        assert_eq!(
            archive.motion_clip(LID, "photo").unwrap().as_deref(),
            Some(&b"clip"[..])
        );
        for table in ["stars", "message_pins", "motion_clips"] {
            let count: i64 = archive
                .connection
                .query_row(
                    &format!("SELECT COUNT(*) FROM {table} WHERE chat = ?1"),
                    [PHONE],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(count, 0, "{table} rolled back");
        }
        archive
            .connection
            .execute_batch("DROP TRIGGER reject_merge")
            .unwrap();
        archive.put_lid("9", "1").unwrap();
    }
    let archive = Archive::open_with_key(&path, &key).unwrap();
    assert!(archive.starred_ids(PHONE).unwrap().contains("photo"));
    assert!(archive.pinned_ids(PHONE, 500).unwrap().contains("photo"));
    assert_eq!(
        archive.motion_clip(PHONE, "photo").unwrap().as_deref(),
        Some(&b"clip"[..])
    );
}
