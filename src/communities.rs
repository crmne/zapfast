//! Community hierarchy for the main Chats list. Filtered views keep matching
//! message destinations flat, and globally pinned chats stay above the groups.

use crate::model::{Chat, ChatId};
use std::collections::{HashMap, HashSet};

pub fn visible<'a>(
    chats: Vec<&'a Chat>,
    all: &'a [Chat],
    collapsed: &HashSet<ChatId>,
    grouped: bool,
    selected: Option<&str>,
) -> Vec<&'a Chat> {
    if !grouped {
        return chats
            .into_iter()
            .filter(|chat| !chat.is_community())
            .collect();
    }
    let parents: HashMap<&str, &Chat> = all
        .iter()
        .filter(|chat| chat.is_community() && !chat.locked && !chat.left)
        .map(|chat| (chat.id.as_str(), chat))
        .collect();
    let mut groups: HashMap<&str, Vec<&Chat>> = HashMap::new();
    for chat in &chats {
        if !chat.pinned
            && !chat.left
            && let Some(parent) = chat
                .community
                .as_ref()
                .and_then(|info| info.parent.as_deref())
            && parents.contains_key(parent)
        {
            groups.entry(parent).or_default().push(chat);
        }
    }
    for groups in groups.values_mut() {
        // Stable sorting retains activity and draft ordering after Announcements.
        groups.sort_by_key(|chat| {
            !chat
                .community
                .as_ref()
                .is_some_and(|info| info.is_announcement)
        });
    }
    let has_known_groups: HashSet<&str> = all
        .iter()
        .filter_map(|chat| chat.community.as_ref()?.parent.as_deref())
        .collect();
    let mut result: Vec<&Chat> = chats
        .iter()
        .copied()
        .filter(|chat| chat.pinned && !chat.is_community())
        .collect();
    let mut shown = HashSet::new();
    for chat in chats {
        if chat.pinned && !chat.is_community() {
            continue;
        }
        if chat.left && !chat.is_community() {
            result.push(chat);
            continue;
        }
        let parent = if chat.is_community() {
            parents.get(chat.id.as_str()).copied()
        } else {
            chat.community
                .as_ref()
                .and_then(|info| info.parent.as_deref())
                .and_then(|id| parents.get(id))
                .copied()
        };
        let Some(parent) = parent else {
            if !chat.is_community() {
                result.push(chat);
            }
            continue;
        };
        if !shown.insert(parent.id.as_str()) {
            continue;
        }
        let members = groups.remove(parent.id.as_str()).unwrap_or_default();
        // A community whose groups are all archived, locked, or pinned adds
        // no empty header. A newly discovered container still shows while syncing.
        if members.is_empty() && has_known_groups.contains(parent.id.as_str()) {
            continue;
        }
        result.push(parent);
        let expanded = !collapsed.contains(&parent.id);
        result.extend(members.into_iter().filter(|member| {
            expanded || member.looks_unread() || selected == Some(member.id.as_str())
        }));
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::GroupCommunity;
    fn chat(id: &str, parent: Option<&str>, is_parent: bool) -> Chat {
        let mut chat = Chat::new(id.into(), id.into());
        chat.community = Some(GroupCommunity {
            is_parent,
            parent: parent.map(str::to_owned),
            is_announcement: false,
        });
        chat
    }
    #[test]
    fn grouping_collapse_and_pins_share_one_visible_order() {
        let mut chats = vec![
            chat("general@g.us", Some("parent@g.us"), false),
            chat("parent@g.us", None, true),
            chat("news@g.us", Some("parent@g.us"), false),
            Chat::new("direct@s.whatsapp.net".into(), "Direct".into()),
        ];
        chats[2].community.as_mut().unwrap().is_announcement = true;
        let ids = |collapsed: HashSet<ChatId>| {
            visible(chats.iter().collect(), &chats, &collapsed, true, None)
                .iter()
                .map(|chat| chat.id.as_str())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            ids(HashSet::new()),
            [
                "parent@g.us",
                "news@g.us",
                "general@g.us",
                "direct@s.whatsapp.net"
            ]
        );
        assert_eq!(
            ids(["parent@g.us".into()].into()),
            ["parent@g.us", "direct@s.whatsapp.net"]
        );
        chats[0].pinned = true;
        assert_eq!(
            visible(
                chats.iter().collect(),
                &chats,
                &Default::default(),
                true,
                None
            )[0]
            .id,
            "general@g.us"
        );
    }
    #[test]
    fn collapsed_communities_reveal_only_unread_or_selected_members() {
        let mut chats = vec![
            chat("parent@g.us", None, true),
            chat("active@g.us", Some("parent@g.us"), false),
            chat("quiet@g.us", Some("parent@g.us"), false),
        ];
        let collapsed = ["parent@g.us".into()].into();
        chats[1].unread = 1;
        let ids = |selected| {
            visible(chats.iter().collect(), &chats, &collapsed, true, selected)
                .into_iter()
                .map(|chat| chat.id.as_str())
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(None), ["parent@g.us", "active@g.us"]);
        assert_eq!(
            ids(Some("quiet@g.us")),
            ["parent@g.us", "active@g.us", "quiet@g.us"]
        );
        assert!(collapsed.contains("parent@g.us"));
    }
    #[test]
    fn filtered_or_unknown_or_private_parents_never_hide_member_chats() {
        let mut chats = vec![
            chat("group@g.us", Some("parent@g.us"), false),
            chat("parent@g.us", None, true),
        ];
        assert_eq!(
            visible(
                chats.iter().collect(),
                &chats,
                &Default::default(),
                false,
                None
            )
            .len(),
            1
        );
        chats[1].locked = true;
        assert_eq!(
            visible(vec![&chats[0]], &chats, &Default::default(), true, None).len(),
            1
        );
        chats[1].left = true;
        assert_eq!(
            visible(vec![&chats[0]], &chats, &Default::default(), true, None)[0].id,
            "group@g.us"
        );
    }
}
