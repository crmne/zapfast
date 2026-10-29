//! Creating a WhatsApp group from the New chat dialog: the dialog's own
//! state (who is picked, the name, the photo) and what can go wrong.
//!
//! The dialog draws from [`Draft`] and pushes `Action`s; `App` applies them
//! here, sends `Command::CreateGroup`, and opens the group once the worker
//! answers with `Event::GroupCreated`.
//!
//! The same draft, made with [`Draft::adding`], is the group info panel's
//! Add members dialog: only the choosing step, without the group's members,
//! sent as `Command::AddGroupMembers`.

use std::path::PathBuf;

use crate::i18n::Locale;
use crate::model::{ChatId, GROUP_NAME_LIMIT};

/// Where the New chat dialog is while a group is being made.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Stage {
    /// Contacts have check boxes; the picked ones sit above as chips.
    #[default]
    Choosing,
    /// The group's name and, optionally, its photo.
    Naming,
}

/// A group being put together in the New chat dialog.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Draft {
    pub stage: Stage,
    /// The people picked, in the order they were picked.
    pub members: Vec<ChatId>,
    /// The name being typed, the view's own field.
    pub name: String,
    /// The photo picked for the group, already cropped and encoded.
    pub picture: Option<PathBuf>,
    /// Whether the group was asked for and WhatsApp has not answered yet.
    pub creating: bool,
    /// The group people are being added to, in the Add members dialog.
    pub adding: Option<ChatId>,
    /// Who is already in that group, and so not offered.
    pub existing: Vec<ChatId>,
}

/// What a step of the dialog leads to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// The draft goes on.
    Stay,
    /// The dialog is the plain contact list again.
    Leave,
}

impl Draft {
    /// The Add members dialog for `chat`, whose `existing` members are not
    /// offered.
    pub fn adding(chat: ChatId, existing: Vec<ChatId>) -> Self {
        Self {
            adding: Some(chat),
            existing,
            ..Self::default()
        }
    }

    /// Whether `id` may be offered: anyone not already in the group.
    pub fn offers(&self, id: &str) -> bool {
        !self.existing.iter().any(|member| member == id)
    }

    /// Picks someone, or lets them go. Letting the last one go while
    /// choosing returns to the plain list, as unchecking everyone should;
    /// the Add members dialog has no plain list and stays.
    pub fn toggle(&mut self, id: &str) -> Step {
        if self.creating || !self.offers(id) {
            return Step::Stay;
        }
        if let Some(index) = self.members.iter().position(|member| member == id) {
            self.members.remove(index);
            if self.members.is_empty() && self.adding.is_none() {
                return Step::Leave;
            }
        } else {
            self.members.push(id.to_owned());
        }
        Step::Stay
    }

    pub fn picked(&self, id: &str) -> bool {
        self.members.iter().any(|member| member == id)
    }

    /// Next is offered once someone is picked. Adding has no next step.
    pub fn can_advance(&self) -> bool {
        self.adding.is_none() && self.stage == Stage::Choosing && !self.members.is_empty()
    }

    /// Whether Add is offered in the Add members dialog.
    pub fn can_add(&self) -> bool {
        self.adding.is_some() && !self.creating && !self.members.is_empty()
    }

    pub fn advance(&mut self) {
        if self.can_advance() {
            self.stage = Stage::Naming;
        }
    }

    /// Back from naming returns to the choice, and from the choice to the
    /// plain list. Nothing moves while WhatsApp is creating the group.
    pub fn back(&mut self) -> Step {
        if self.creating {
            return Step::Stay;
        }
        if self.adding.is_some() {
            return Step::Leave;
        }
        match self.stage {
            Stage::Naming => {
                self.stage = Stage::Choosing;
                Step::Stay
            }
            Stage::Choosing => Step::Leave,
        }
    }

    /// The name as it is sent: trimmed.
    pub fn subject(&self) -> &str {
        self.name.trim()
    }

    /// Why the draft cannot be created yet, if anything stops it.
    pub fn problem(&self) -> Option<CreateFailure> {
        if self.members.is_empty() {
            Some(CreateFailure::NoMembers)
        } else if self.subject().is_empty() {
            Some(CreateFailure::NoName)
        } else if self.subject().chars().count() > GROUP_NAME_LIMIT {
            Some(CreateFailure::NameTooLong)
        } else {
            None
        }
    }

    /// Whether Create is offered.
    pub fn can_create(&self) -> bool {
        self.stage == Stage::Naming && !self.creating && self.problem().is_none()
    }
}

/// Why a group was not created, in the terms the window explains it in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CreateFailure {
    NoName,
    NameTooLong,
    NoMembers,
    /// There is no link to WhatsApp.
    Offline,
    /// WhatsApp did not answer in time; the group may still have been made.
    Timeout,
    /// WhatsApp refused this account (not authorized, forbidden).
    Refused,
    /// Too many groups made recently.
    RateLimited,
    /// WhatsApp rejected the request itself: the name or the members.
    NotAccepted,
    /// Anything else.
    Failed,
}

impl CreateFailure {
    /// A short name for the log, never anything about the group.
    pub fn kind(self) -> &'static str {
        match self {
            Self::NoName => "no name",
            Self::NameTooLong => "name too long",
            Self::NoMembers => "no members",
            Self::Offline => "offline",
            Self::Timeout => "timeout",
            Self::Refused => "refused",
            Self::RateLimited => "rate limited",
            Self::NotAccepted => "not accepted",
            Self::Failed => "failed",
        }
    }

    /// What the toast says. Literal calls, so xgettext extracts each.
    pub fn message(self, locale: Locale) -> String {
        use crate::i18n::gettext;
        match self {
            Self::NoName => gettext(locale, "Enter a name for the group.").into_owned(),
            Self::NameTooLong => {
                gettext(locale, "A group name can have at most {count} characters.")
                    .replace("{count}", &GROUP_NAME_LIMIT.to_string())
            }
            Self::NoMembers => {
                gettext(locale, "Choose at least one person for the group.").into_owned()
            }
            Self::Offline => gettext(locale, "Connect to WhatsApp to create a group.").into_owned(),
            Self::Timeout => gettext(
                locale,
                "WhatsApp did not answer in time. If the group does not appear shortly, try again.",
            )
            .into_owned(),
            Self::Refused => gettext(
                locale,
                "WhatsApp does not allow this account to create groups right now.",
            )
            .into_owned(),
            Self::RateLimited => gettext(
                locale,
                "You have created too many groups recently. Try again later.",
            )
            .into_owned(),
            Self::NotAccepted => gettext(
                locale,
                "WhatsApp did not accept this group's name or members.",
            )
            .into_owned(),
            Self::Failed => gettext(locale, "Could not create the group. Try again.").into_owned(),
        }
    }

    /// The failure an IQ error code stands for.
    pub fn from_code(code: u16) -> Self {
        match code {
            401 | 403 => Self::Refused,
            429 => Self::RateLimited,
            400 | 406 | 409 => Self::NotAccepted,
            408 | 504 => Self::Timeout,
            _ => Self::Failed,
        }
    }
}

/// A group WhatsApp created, as the window hears of it.
#[derive(Clone, Debug, PartialEq)]
pub struct Created {
    pub chat: ChatId,
    /// The people picked whom WhatsApp did not add (their privacy settings
    /// allow only some to add them), by canonical chat id.
    pub unadded: Vec<ChatId>,
}

/// The people picked who are not among the group's members. `members` are
/// canonical ids, as the archive files them.
pub fn unadded(requested: &[ChatId], members: &[String]) -> Vec<ChatId> {
    requested
        .iter()
        .filter(|id| !members.contains(id))
        .cloned()
        .collect()
}

/// What the window says after creating a group some could not join.
pub fn unadded_message(locale: Locale, names: &[String]) -> String {
    crate::i18n::gettext(
        locale,
        "{names} could not be added to the group. Their privacy settings may not allow it.",
    )
    .replace("{names}", &names.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft(members: &[&str]) -> Draft {
        Draft {
            members: members.iter().map(|id| (*id).to_owned()).collect(),
            ..Draft::default()
        }
    }

    #[test]
    fn checking_and_unchecking_everyone_returns_to_the_plain_list() {
        let mut draft = Draft::default();
        assert!(!draft.can_advance(), "Next waits for someone");
        assert_eq!(draft.toggle("1@s.whatsapp.net"), Step::Stay);
        assert_eq!(draft.toggle("2@s.whatsapp.net"), Step::Stay);
        assert!(draft.picked("2@s.whatsapp.net"));
        assert!(draft.can_advance());
        assert_eq!(draft.toggle("1@s.whatsapp.net"), Step::Stay);
        assert_eq!(draft.members, ["2@s.whatsapp.net"]);
        assert_eq!(draft.toggle("2@s.whatsapp.net"), Step::Leave);
    }

    #[test]
    fn next_names_the_group_and_back_steps_out_one_stage_at_a_time() {
        let mut draft = draft(&["1@s.whatsapp.net"]);
        draft.advance();
        assert_eq!(draft.stage, Stage::Naming);
        assert!(!draft.can_create(), "a name is required");
        assert_eq!(draft.problem(), Some(CreateFailure::NoName));
        draft.name = "   ".into();
        assert!(!draft.can_create(), "a blank name is no name");
        draft.name = " Book club ".into();
        assert!(draft.can_create());
        assert_eq!(draft.subject(), "Book club");
        assert_eq!(draft.back(), Step::Stay);
        assert_eq!(draft.stage, Stage::Choosing);
        assert_eq!(draft.name, " Book club ", "the name is kept for Next");
        assert_eq!(draft.back(), Step::Leave);
    }

    #[test]
    fn the_name_follows_whatsapps_subject_limit_in_characters() {
        let mut draft = draft(&["1@s.whatsapp.net"]);
        draft.advance();
        draft.name = "é".repeat(GROUP_NAME_LIMIT);
        assert!(draft.can_create(), "counted in characters, not bytes");
        draft.name.push('é');
        assert_eq!(draft.problem(), Some(CreateFailure::NameTooLong));
        assert!(!draft.can_create());
    }

    #[test]
    fn nothing_moves_while_whatsapp_is_creating_the_group() {
        let mut draft = draft(&["1@s.whatsapp.net"]);
        draft.advance();
        draft.name = "Trip".into();
        draft.creating = true;
        assert!(!draft.can_create(), "Create is not offered twice");
        assert_eq!(draft.back(), Step::Stay);
        assert_eq!(draft.stage, Stage::Naming);
        assert_eq!(draft.toggle("1@s.whatsapp.net"), Step::Stay);
        assert_eq!(draft.members.len(), 1);
    }

    #[test]
    fn adding_offers_only_people_outside_the_group_and_has_one_step() {
        let mut draft = Draft::adding(
            "120363000000000001@g.us".into(),
            vec!["1@s.whatsapp.net".into()],
        );
        assert!(!draft.offers("1@s.whatsapp.net"), "already a member");
        assert_eq!(draft.toggle("1@s.whatsapp.net"), Step::Stay);
        assert!(draft.members.is_empty(), "and cannot be picked");
        assert!(!draft.can_add(), "Add waits for someone");
        assert_eq!(draft.toggle("2@s.whatsapp.net"), Step::Stay);
        assert!(draft.can_add());
        assert!(!draft.can_advance(), "there is no naming step");
        draft.advance();
        assert_eq!(draft.stage, Stage::Choosing);
        assert_eq!(
            draft.toggle("2@s.whatsapp.net"),
            Step::Stay,
            "unchecking everyone keeps the dialog"
        );
        assert_eq!(draft.back(), Step::Leave, "Back closes it");
        draft.creating = true;
        assert!(!draft.can_add());
    }

    #[test]
    fn the_people_left_out_are_those_picked_and_not_listed() {
        let requested = vec!["1@s.whatsapp.net".to_owned(), "2@s.whatsapp.net".to_owned()];
        let members = vec![
            "me@s.whatsapp.net".to_owned(),
            "1@s.whatsapp.net".to_owned(),
        ];
        assert_eq!(unadded(&requested, &members), ["2@s.whatsapp.net"]);
        assert!(unadded(&requested[..1], &members).is_empty());
    }

    #[test]
    fn error_codes_read_as_plain_reasons() {
        assert_eq!(CreateFailure::from_code(403), CreateFailure::Refused);
        assert_eq!(CreateFailure::from_code(401), CreateFailure::Refused);
        assert_eq!(CreateFailure::from_code(429), CreateFailure::RateLimited);
        assert_eq!(CreateFailure::from_code(406), CreateFailure::NotAccepted);
        assert_eq!(CreateFailure::from_code(408), CreateFailure::Timeout);
        assert_eq!(CreateFailure::from_code(500), CreateFailure::Failed);
        let limit = CreateFailure::NameTooLong.message(Locale::English);
        assert!(limit.contains(&GROUP_NAME_LIMIT.to_string()), "{limit}");
        assert_eq!(
            unadded_message(Locale::English, &["Ada".into(), "Grace".into()]),
            "Ada, Grace could not be added to the group. Their privacy settings may not allow it."
        );
    }
}
