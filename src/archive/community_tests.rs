use super::*;
use crate::model::GroupCommunity;

#[test]
fn community_metadata_survives_history_and_can_be_unlinked() {
    let archive = Archive::in_memory().unwrap();
    let id = "group@g.us";
    archive.ensure_chat(id, "General").unwrap();
    assert!(archive.chat(id).unwrap().unwrap().community.is_none());
    let community = GroupCommunity {
        is_parent: false,
        parent: Some("parent@g.us".into()),
        is_announcement: true,
    };
    archive.set_group_community(id, &community).unwrap();
    archive
        .upsert_chat(&Chat::new(id.into(), "General".into()))
        .unwrap();
    assert_eq!(
        archive.chat(id).unwrap().unwrap().community,
        Some(community)
    );
    archive
        .set_group_community(id, &GroupCommunity::default())
        .unwrap();
    assert_eq!(
        archive.chat(id).unwrap().unwrap().community.unwrap().parent,
        None
    );
}
