//! Running a group from its info panel: the invite link, adding people,
//! the group's permissions, and what its members may be made.
//!
//! The panel draws from the chat's `GroupSettings` and pushes `Action`s;
//! `App` sends the matching `Command`s, and the worker files each change in
//! the archive only once WhatsApp accepts it.

use std::fmt;

use crate::i18n::{Locale, gettext};
use crate::model::{Chat, ChatId};

/// One of WhatsApp's "Group permissions".
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Permission {
    /// Edit group settings: the name, photo, description and timer.
    EditInfo,
    /// Send new messages.
    SendMessages,
    /// Add other members.
    AddMembers,
    /// Approve new members before they join.
    ApproveMembers,
}

impl Permission {
    /// The switches in the order WhatsApp lists them.
    pub const ALL: [Self; 4] = [
        Self::EditInfo,
        Self::SendMessages,
        Self::AddMembers,
        Self::ApproveMembers,
    ];

    /// A short name for the log.
    pub fn kind(self) -> &'static str {
        match self {
            Self::EditInfo => "edit info",
            Self::SendMessages => "send messages",
            Self::AddMembers => "add members",
            Self::ApproveMembers => "approve members",
        }
    }

    /// Whether the switch is on for `chat`: members may (or, for approval,
    /// admins approve). `None` until the metadata has said.
    pub fn value(self, chat: &Chat) -> Option<bool> {
        let settings = chat.group_settings.as_ref();
        match self {
            Self::EditInfo => chat.info_locked.map(|locked| !locked),
            Self::SendMessages => settings.map(|settings| !settings.announce),
            Self::AddMembers => settings.map(|settings| settings.members_add),
            Self::ApproveMembers => settings.map(|settings| settings.approval),
        }
    }

    /// The switch's label. Literal calls, so xgettext extracts each.
    pub fn label(self, locale: Locale) -> String {
        match self {
            Self::EditInfo => gettext(locale, "Edit group settings"),
            Self::SendMessages => gettext(locale, "Send new messages"),
            Self::AddMembers => gettext(locale, "Add other members"),
            Self::ApproveMembers => gettext(locale, "Approve new members"),
        }
        .into_owned()
    }

    /// The line under the label.
    pub fn description(self, locale: Locale) -> String {
        match self {
            Self::EditInfo => gettext(
                locale,
                "Change the group's name, photo, description and disappearing messages.",
            ),
            Self::SendMessages => gettext(locale, "Write in the group."),
            Self::AddMembers => gettext(locale, "Add people to the group."),
            Self::ApproveMembers => gettext(
                locale,
                "When turned on, admins must approve anyone who wants to join.",
            ),
        }
        .into_owned()
    }
}

/// What a member may be made.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MemberChange {
    Promote,
    Demote,
    Remove,
}

impl MemberChange {
    pub fn kind(self) -> &'static str {
        match self {
            Self::Promote => "promote",
            Self::Demote => "demote",
            Self::Remove => "remove",
        }
    }
}

/// The changes the member menu offers for `member`: none for ourselves, for
/// the creator, or unless we run the group. `ours` holds every id we may be
/// listed under.
pub fn member_changes(chat: &Chat, member: &str, ours: &[&str]) -> Vec<MemberChange> {
    Roles::of(chat).changes(member, ours)
}

/// What the member list needs of a group to offer its menu, taken once per
/// frame rather than borrowed from the chat.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Roles {
    pub manage: bool,
    pub admins: Vec<String>,
    pub creators: Vec<String>,
}

impl Roles {
    pub fn of(chat: &Chat) -> Self {
        let settings = chat.group_settings.as_ref();
        Self {
            manage: chat.can_manage_group(),
            admins: settings.map(|s| s.admins.clone()).unwrap_or_default(),
            creators: settings.map(|s| s.super_admins.clone()).unwrap_or_default(),
        }
    }

    pub fn is_admin(&self, member: &str) -> bool {
        self.admins.iter().any(|id| id == member)
    }

    pub fn changes(&self, member: &str, ours: &[&str]) -> Vec<MemberChange> {
        if !self.manage || ours.contains(&member) || self.creators.iter().any(|id| id == member) {
            return Vec::new();
        }
        let role = if self.is_admin(member) {
            MemberChange::Demote
        } else {
            MemberChange::Promote
        };
        vec![role, MemberChange::Remove]
    }
}

/// A group's invite link, `https://chat.whatsapp.com/<code>`. Anyone holding
/// it can join, so it is never written to the log: its `Debug` hides it.
#[derive(Clone, PartialEq, Eq)]
pub struct InviteLink(String);

impl InviteLink {
    /// The link WhatsApp answered with. Only its own links are taken.
    pub fn new(link: String) -> Option<Self> {
        let code = link.strip_prefix("https://chat.whatsapp.com/")?;
        (!code.is_empty() && code.chars().all(|c| c.is_ascii_alphanumeric())).then_some(Self(link))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for InviteLink {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("InviteLink(..)")
    }
}

/// The info panel's link row: asked for, known, or refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkState {
    Loading,
    Known(InviteLink),
    Failed(Failure),
}

/// Why a change to a group did not happen, in the window's terms.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    /// There is no link to WhatsApp.
    Offline,
    /// WhatsApp did not answer in time.
    Timeout,
    /// We may not make it (no longer an admin, a locked setting).
    Refused,
    RateLimited,
    /// Anything else.
    Failed,
}

impl Failure {
    pub fn kind(self) -> &'static str {
        match self {
            Self::Offline => "offline",
            Self::Timeout => "timeout",
            Self::Refused => "refused",
            Self::RateLimited => "rate limited",
            Self::Failed => "failed",
        }
    }

    /// The failure an IQ error code stands for.
    pub fn from_code(code: u16) -> Self {
        match code {
            401 | 403 => Self::Refused,
            429 => Self::RateLimited,
            408 | 504 => Self::Timeout,
            _ => Self::Failed,
        }
    }

    /// What the toast says about a change that did not happen.
    pub fn message(self, locale: Locale) -> String {
        match self {
            Self::Offline => gettext(locale, "Connect to WhatsApp to change the group."),
            Self::Timeout => gettext(locale, "WhatsApp did not answer in time. Try again."),
            Self::Refused => gettext(locale, "Only group admins can do that."),
            Self::RateLimited => gettext(locale, "Too many changes at once. Try again later."),
            Self::Failed => gettext(locale, "Could not change the group. Try again."),
        }
        .into_owned()
    }

    /// What the link row says when the link could not be had.
    pub fn link_message(self, locale: Locale) -> String {
        match self {
            Self::Offline => gettext(locale, "Connect to WhatsApp to see the invite link."),
            Self::Refused => gettext(locale, "Only group admins can see the invite link."),
            _ => gettext(locale, "Could not get the invite link."),
        }
        .into_owned()
    }
}

/// Why WhatsApp would not add one person, from its per-person error code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// Their privacy settings let only some people add them (403).
    Privacy,
    /// They left the group recently and cannot be added back yet (408).
    RecentlyLeft,
    /// The group has no more room (500).
    Full,
    /// Anything else, such as a number not on WhatsApp.
    Other,
}

impl Refusal {
    pub fn from_code(code: &str) -> Self {
        match code {
            "403" => Self::Privacy,
            "408" => Self::RecentlyLeft,
            "500" => Self::Full,
            _ => Self::Other,
        }
    }
}

/// What WhatsApp did with an Add members request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Added {
    pub chat: ChatId,
    /// The people now in the group, by canonical chat id.
    pub added: Vec<ChatId>,
    /// The people WhatsApp would not add, with why.
    pub refused: Vec<(ChatId, Refusal)>,
}

/// The toasts that tell who could not be added, one per reason, given each
/// person's name.
pub fn refused_messages(locale: Locale, refused: &[(String, Refusal)]) -> Vec<String> {
    let mut said = Vec::new();
    for reason in [
        Refusal::Privacy,
        Refusal::RecentlyLeft,
        Refusal::Full,
        Refusal::Other,
    ] {
        let names: Vec<&str> = refused
            .iter()
            .filter(|(_, why)| *why == reason)
            .map(|(name, _)| name.as_str())
            .collect();
        if names.is_empty() {
            continue;
        }
        let template = match reason {
            Refusal::Privacy => gettext(
                locale,
                "{names} could not be added to the group. Their privacy settings may not allow it. You can send them the invite link.",
            ),
            Refusal::RecentlyLeft => gettext(
                locale,
                "{names} left the group recently and cannot be added back yet.",
            ),
            Refusal::Full => gettext(locale, "The group is full. {names} could not be added."),
            Refusal::Other => gettext(locale, "{names} could not be added to the group."),
        };
        said.push(template.replace("{names}", &names.join(", ")));
    }
    said
}

/// What the toast says once a member change is accepted.
pub fn changed_message(locale: Locale, change: MemberChange, name: &str) -> String {
    match change {
        MemberChange::Promote => gettext(locale, "{name} is now a group admin."),
        MemberChange::Demote => gettext(locale, "{name} is no longer a group admin."),
        MemberChange::Remove => gettext(locale, "{name} was removed from the group."),
    }
    .replace("{name}", name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::GroupSettings;

    fn group(admin: bool) -> Chat {
        let mut chat = Chat::new("120363000000000001@g.us".into(), "Club".into());
        chat.admin = admin;
        chat.info_locked = Some(true);
        chat.participants = vec!["1@s.whatsapp.net".into(), "2@s.whatsapp.net".into()];
        chat
    }

    #[test]
    fn nothing_is_offered_before_the_metadata_says() {
        let chat = group(false);
        assert!(!chat.can_add_members());
        assert!(!chat.can_share_invite_link());
        assert!(!chat.can_manage_group());
        assert_eq!(Permission::SendMessages.value(&chat), None);
        assert_eq!(Permission::EditInfo.value(&chat), Some(false));
    }

    #[test]
    fn members_add_and_share_only_when_the_group_lets_them() {
        let mut chat = group(false);
        chat.group_settings = Some(GroupSettings::default());
        assert!(!chat.can_add_members(), "admins only, by default");
        assert!(!chat.can_share_invite_link());
        assert!(!chat.can_manage_group());
        chat.group_settings = Some(GroupSettings {
            members_add: true,
            members_share_link: true,
            ..GroupSettings::default()
        });
        assert!(chat.can_add_members());
        assert!(chat.can_share_invite_link());
        assert!(!chat.can_manage_group(), "permissions stay with admins");
        chat.left = true;
        assert!(!chat.can_add_members(), "a group we left is not ours");
        assert!(!chat.can_share_invite_link());
    }

    #[test]
    fn admins_run_the_group_whatever_it_allows_members() {
        let mut chat = group(true);
        chat.group_settings = Some(GroupSettings::default());
        assert!(chat.can_add_members());
        assert!(chat.can_share_invite_link());
        assert!(chat.can_manage_group());
        let direct = Chat::new("1@s.whatsapp.net".into(), "Ada".into());
        assert!(!direct.can_manage_group(), "only groups have admins");
    }

    #[test]
    fn the_member_menu_spares_us_and_the_creator() {
        let mut chat = group(true);
        chat.group_settings = Some(GroupSettings {
            admins: vec!["1@s.whatsapp.net".into(), "me@s.whatsapp.net".into()],
            super_admins: vec!["9@s.whatsapp.net".into()],
            ..GroupSettings::default()
        });
        let ours = ["me@s.whatsapp.net"];
        assert_eq!(
            member_changes(&chat, "1@s.whatsapp.net", &ours),
            [MemberChange::Demote, MemberChange::Remove]
        );
        assert_eq!(
            member_changes(&chat, "2@s.whatsapp.net", &ours),
            [MemberChange::Promote, MemberChange::Remove]
        );
        assert!(member_changes(&chat, "me@s.whatsapp.net", &ours).is_empty());
        assert!(member_changes(&chat, "9@s.whatsapp.net", &ours).is_empty());
        chat.admin = false;
        assert!(member_changes(&chat, "2@s.whatsapp.net", &ours).is_empty());
    }

    #[test]
    fn the_permission_switches_read_what_members_may_do() {
        let mut chat = group(true);
        chat.info_locked = Some(false);
        chat.group_settings = Some(GroupSettings {
            announce: true,
            members_add: true,
            approval: true,
            ..GroupSettings::default()
        });
        assert_eq!(Permission::EditInfo.value(&chat), Some(true));
        assert_eq!(Permission::SendMessages.value(&chat), Some(false));
        assert_eq!(Permission::AddMembers.value(&chat), Some(true));
        assert_eq!(Permission::ApproveMembers.value(&chat), Some(true));
    }

    #[test]
    fn an_invite_link_is_whatsapps_own_and_never_shows_in_the_log() {
        let link = InviteLink::new("https://chat.whatsapp.com/AbC123".into()).unwrap();
        assert_eq!(link.as_str(), "https://chat.whatsapp.com/AbC123");
        assert_eq!(format!("{link:?}"), "InviteLink(..)");
        let state = LinkState::Known(link);
        assert!(!format!("{state:?}").contains("AbC123"));
        assert!(InviteLink::new("https://example.com/AbC123".into()).is_none());
        assert!(InviteLink::new("https://chat.whatsapp.com/".into()).is_none());
        assert!(InviteLink::new("https://chat.whatsapp.com/a/../b".into()).is_none());
    }

    #[test]
    fn refusals_are_told_by_name_one_toast_per_reason() {
        let said = refused_messages(
            Locale::English,
            &[
                ("Ada".into(), Refusal::Privacy),
                ("Grace".into(), Refusal::Privacy),
                ("Tom".into(), Refusal::RecentlyLeft),
            ],
        );
        assert_eq!(
            said,
            [
                "Ada, Grace could not be added to the group. Their privacy settings may not allow it. You can send them the invite link.",
                "Tom left the group recently and cannot be added back yet.",
            ]
        );
        assert!(refused_messages(Locale::English, &[]).is_empty());
        assert_eq!(Refusal::from_code("403"), Refusal::Privacy);
        assert_eq!(Refusal::from_code("409"), Refusal::Other);
        assert_eq!(Failure::from_code(403), Failure::Refused);
        assert_eq!(Failure::from_code(429), Failure::RateLimited);
        assert_eq!(Failure::from_code(500), Failure::Failed);
    }
}
