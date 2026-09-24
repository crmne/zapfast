//! WhatsApp account privacy: last seen, online, profile photo, About, groups,
//! read receipts, and calls. The values live on the phone, not in
//! `settings.json`; ZapFast reads them on connect and writes one category at a
//! time.
//!
//! "My contacts except…" is shown when the account holds it, but not offered:
//! the people it excludes are edited on the phone.

use std::borrow::Cow;
use std::collections::HashMap;

use whatsapp_rust::wacore::iq::privacy::{PrivacyCategory, PrivacySettingsResponse, PrivacyValue};

use crate::i18n::{Locale, gettext, pgettext};

/// Account privacy category shown in Settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PrivacyKind {
    LastSeen,
    Online,
    Profile,
    About,
    GroupAdd,
    ReadReceipts,
    CallAdd,
}

impl PrivacyKind {
    /// Every category, in the phone's order.
    pub const ALL: [Self; 7] = [
        Self::LastSeen,
        Self::Online,
        Self::Profile,
        Self::About,
        Self::GroupAdd,
        Self::ReadReceipts,
        Self::CallAdd,
    ];

    pub fn label(self, locale: Locale) -> Cow<'static, str> {
        match self {
            Self::LastSeen => pgettext(locale, "privacy", "Last seen"),
            Self::Online => pgettext(locale, "privacy", "Online"),
            Self::Profile => pgettext(locale, "privacy", "Profile photo"),
            Self::About => pgettext(locale, "privacy", "About"),
            Self::GroupAdd => pgettext(locale, "privacy", "Groups"),
            Self::ReadReceipts => pgettext(locale, "privacy", "Read receipts"),
            Self::CallAdd => pgettext(locale, "privacy", "Calls"),
        }
    }

    pub fn hint(self, locale: Locale) -> Cow<'static, str> {
        match self {
            Self::LastSeen => gettext(locale, "Who can see when you were last on WhatsApp."),
            Self::Online => gettext(locale, "Who can see when you are online."),
            Self::Profile => gettext(locale, "Who can see your profile photo."),
            Self::About => gettext(locale, "Who can see your About text."),
            Self::GroupAdd => gettext(
                locale,
                "Who can add you to groups. Anyone else has to send you an invite.",
            ),
            Self::ReadReceipts => gettext(
                locale,
                "For your whole WhatsApp account. When off, you neither send nor see read receipts in one-to-one chats. Groups always send them.",
            ),
            Self::CallAdd => gettext(
                locale,
                "Silencing unknown callers keeps calls from people not in your contacts from ringing.",
            ),
        }
    }

    /// The values offered in the picker, which are the ones the phone offers.
    /// The account can hold others (an Except list, or an older "Nobody" for
    /// groups); those are shown but not offered.
    pub fn choices(self) -> &'static [PrivacyChoice] {
        match self {
            Self::LastSeen | Self::Profile | Self::About => &[
                PrivacyChoice::Everyone,
                PrivacyChoice::Contacts,
                PrivacyChoice::Nobody,
            ],
            Self::GroupAdd => &[PrivacyChoice::Everyone, PrivacyChoice::Contacts],
            Self::Online => &[PrivacyChoice::Everyone, PrivacyChoice::SameAsLastSeen],
            Self::ReadReceipts => &[PrivacyChoice::Everyone, PrivacyChoice::Nobody],
            Self::CallAdd => &[PrivacyChoice::Everyone, PrivacyChoice::Known],
        }
    }

    /// A stable name for widget ids.
    pub fn wire_name(self) -> &'static str {
        match self {
            Self::LastSeen => "last",
            Self::Online => "online",
            Self::Profile => "profile",
            Self::About => "status",
            Self::GroupAdd => "groupadd",
            Self::ReadReceipts => "readreceipts",
            Self::CallAdd => "calladd",
        }
    }

    pub fn from_wire(category: &PrivacyCategory) -> Option<Self> {
        match category {
            PrivacyCategory::Last => Some(Self::LastSeen),
            PrivacyCategory::Online => Some(Self::Online),
            PrivacyCategory::Profile => Some(Self::Profile),
            PrivacyCategory::Status => Some(Self::About),
            PrivacyCategory::GroupAdd => Some(Self::GroupAdd),
            PrivacyCategory::ReadReceipts => Some(Self::ReadReceipts),
            PrivacyCategory::CallAdd => Some(Self::CallAdd),
            PrivacyCategory::Messages
            | PrivacyCategory::DefenseMode
            | PrivacyCategory::Other(_) => None,
        }
    }

    pub fn to_wire(self) -> PrivacyCategory {
        match self {
            Self::LastSeen => PrivacyCategory::Last,
            Self::Online => PrivacyCategory::Online,
            Self::Profile => PrivacyCategory::Profile,
            Self::About => PrivacyCategory::Status,
            Self::GroupAdd => PrivacyCategory::GroupAdd,
            Self::ReadReceipts => PrivacyCategory::ReadReceipts,
            Self::CallAdd => PrivacyCategory::CallAdd,
        }
    }
}

/// One account privacy value. Not every value is valid for every category.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PrivacyChoice {
    Everyone,
    Contacts,
    /// My contacts except a list the phone edits.
    Except,
    Nobody,
    SameAsLastSeen,
    /// Calls: only known callers ring.
    Known,
}

impl PrivacyChoice {
    pub fn label(self, locale: Locale) -> Cow<'static, str> {
        match self {
            Self::Everyone => pgettext(locale, "privacy", "Everyone"),
            Self::Contacts => pgettext(locale, "privacy", "My contacts"),
            Self::Except => pgettext(locale, "privacy", "My contacts except…"),
            Self::Nobody => pgettext(locale, "privacy", "Nobody"),
            Self::SameAsLastSeen => pgettext(locale, "privacy", "Same as last seen"),
            Self::Known => pgettext(locale, "privacy", "Silence unknown callers"),
        }
    }

    pub fn from_wire(value: &PrivacyValue) -> Option<Self> {
        match value {
            PrivacyValue::All => Some(Self::Everyone),
            PrivacyValue::Contacts => Some(Self::Contacts),
            PrivacyValue::ContactBlacklist => Some(Self::Except),
            PrivacyValue::None => Some(Self::Nobody),
            PrivacyValue::MatchLastSeen => Some(Self::SameAsLastSeen),
            PrivacyValue::Known => Some(Self::Known),
            PrivacyValue::Off | PrivacyValue::OnStandard | PrivacyValue::Other(_) => None,
        }
    }

    pub fn to_wire(self) -> PrivacyValue {
        match self {
            Self::Everyone => PrivacyValue::All,
            Self::Contacts => PrivacyValue::Contacts,
            Self::Except => PrivacyValue::ContactBlacklist,
            Self::Nobody => PrivacyValue::None,
            Self::SameAsLastSeen => PrivacyValue::MatchLastSeen,
            Self::Known => PrivacyValue::Known,
        }
    }
}

/// The account's privacy as the phone last reported it, with the writes still
/// in flight applied on top.
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    values: HashMap<PrivacyKind, PrivacyChoice>,
    /// The confirmed value behind each write in flight, restored if it fails.
    revert: HashMap<PrivacyKind, PrivacyChoice>,
    /// The last fetch failed; the values shown may be stale.
    pub fetch_failed: bool,
    /// A fetch has succeeded since linking.
    pub loaded: bool,
}

impl Snapshot {
    pub fn get(&self, kind: PrivacyKind) -> Option<PrivacyChoice> {
        self.values.get(&kind).copied()
    }

    pub fn pending(&self, kind: PrivacyKind) -> bool {
        self.revert.contains_key(&kind)
    }

    /// Whether the rows can be edited: the phone's values are loaded and the
    /// last fetch did not fail.
    pub fn editable(&self) -> bool {
        self.loaded && !self.fetch_failed
    }

    /// Takes a fetched snapshot. A failed fetch keeps what was shown and
    /// marks it stale. A write still in flight keeps its value, and the
    /// fetched one becomes what a failure restores.
    pub fn apply_fetch(&mut self, values: Vec<(PrivacyKind, PrivacyChoice)>, failed: bool) {
        self.fetch_failed = failed;
        if failed {
            return;
        }
        let mut fresh: HashMap<_, _> = values.into_iter().collect();
        for (kind, confirmed) in &mut self.revert {
            if let Some(fetched) = fresh.get(kind) {
                *confirmed = *fetched;
            }
            if let Some(optimistic) = self.values.get(kind) {
                fresh.insert(*kind, *optimistic);
            }
        }
        self.values = fresh;
        self.loaded = true;
    }

    /// Shows `choice` at once and remembers what to restore. Returns false
    /// when nothing should be written: the category is unknown, already
    /// holds the value, or has a write in flight.
    pub fn begin_set(&mut self, kind: PrivacyKind, choice: PrivacyChoice) -> bool {
        let Some(current) = self.get(kind) else {
            return false;
        };
        if current == choice || self.pending(kind) || !kind.choices().contains(&choice) {
            return false;
        }
        self.revert.insert(kind, current);
        self.values.insert(kind, choice);
        true
    }

    /// The phone confirmed a write. Returns false for a confirmation nothing
    /// here was waiting for, such as one from before an unlink.
    pub fn finish_set(&mut self, kind: PrivacyKind) -> bool {
        self.revert.remove(&kind).is_some()
    }

    /// The phone refused a write: the confirmed value comes back.
    pub fn fail_set(&mut self, kind: PrivacyKind) {
        if let Some(confirmed) = self.revert.remove(&kind) {
            self.values.insert(kind, confirmed);
        }
    }

    /// Sample values for the demo Settings page.
    pub fn demo() -> Self {
        let mut snapshot = Self::default();
        snapshot.apply_fetch(
            vec![
                (PrivacyKind::LastSeen, PrivacyChoice::Except),
                (PrivacyKind::Online, PrivacyChoice::SameAsLastSeen),
                (PrivacyKind::Profile, PrivacyChoice::Contacts),
                (PrivacyKind::About, PrivacyChoice::Everyone),
                (PrivacyKind::GroupAdd, PrivacyChoice::Contacts),
                (PrivacyKind::ReadReceipts, PrivacyChoice::Everyone),
                (PrivacyKind::CallAdd, PrivacyChoice::Everyone),
            ],
            false,
        );
        snapshot
    }
}

/// The categories and values the phone reported that ZapFast shows. A value
/// the library does not consider valid for its category is left out.
pub fn values_from_response(
    settings: &PrivacySettingsResponse,
) -> Vec<(PrivacyKind, PrivacyChoice)> {
    settings
        .settings
        .iter()
        .filter(|setting| setting.category.is_valid_value(&setting.value))
        .filter_map(|setting| {
            Some((
                PrivacyKind::from_wire(&setting.category)?,
                PrivacyChoice::from_wire(&setting.value)?,
            ))
        })
        .collect()
}

/// The wire form of a write, for a value the picker offers.
pub fn wire_set(
    kind: PrivacyKind,
    choice: PrivacyChoice,
) -> Option<(PrivacyCategory, PrivacyValue)> {
    if !kind.choices().contains(&choice) {
        return None;
    }
    let value = choice.to_wire();
    let category = kind.to_wire();
    category.is_valid_value(&value).then_some((category, value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use whatsapp_rust::wacore::iq::privacy::PrivacySetting;

    const ALL_CHOICES: [PrivacyChoice; 6] = [
        PrivacyChoice::Everyone,
        PrivacyChoice::Contacts,
        PrivacyChoice::Except,
        PrivacyChoice::Nobody,
        PrivacyChoice::SameAsLastSeen,
        PrivacyChoice::Known,
    ];

    #[test]
    fn only_offered_values_are_written() {
        for kind in PrivacyKind::ALL {
            for choice in ALL_CHOICES {
                assert_eq!(
                    wire_set(kind, choice).is_some(),
                    kind.choices().contains(&choice),
                    "{kind:?} {choice:?}"
                );
            }
        }
        // An Except list is edited on the phone, never written from here.
        for kind in PrivacyKind::ALL {
            assert!(wire_set(kind, PrivacyChoice::Except).is_none());
        }
        assert!(wire_set(PrivacyKind::GroupAdd, PrivacyChoice::Nobody).is_none());
        assert!(wire_set(PrivacyKind::Online, PrivacyChoice::Nobody).is_none());
        assert!(wire_set(PrivacyKind::ReadReceipts, PrivacyChoice::Contacts).is_none());
    }

    #[test]
    fn every_offered_value_is_one_the_library_accepts() {
        for kind in PrivacyKind::ALL {
            for choice in kind.choices() {
                assert!(
                    kind.to_wire().is_valid_value(&choice.to_wire()),
                    "{kind:?} {choice:?}"
                );
            }
        }
    }

    #[test]
    fn fetch_maps_phone_categories_and_skips_the_rest() {
        let setting = |category, value| PrivacySetting { category, value };
        let settings = PrivacySettingsResponse {
            settings: vec![
                setting(PrivacyCategory::Last, PrivacyValue::ContactBlacklist),
                setting(PrivacyCategory::Online, PrivacyValue::MatchLastSeen),
                setting(PrivacyCategory::ReadReceipts, PrivacyValue::None),
                setting(PrivacyCategory::GroupAdd, PrivacyValue::None),
                setting(PrivacyCategory::DefenseMode, PrivacyValue::Off),
                setting(PrivacyCategory::Messages, PrivacyValue::Contacts),
                // Not a value this category takes.
                setting(PrivacyCategory::Online, PrivacyValue::None),
            ],
        };
        assert_eq!(
            values_from_response(&settings),
            vec![
                (PrivacyKind::LastSeen, PrivacyChoice::Except),
                (PrivacyKind::Online, PrivacyChoice::SameAsLastSeen),
                (PrivacyKind::ReadReceipts, PrivacyChoice::Nobody),
                (PrivacyKind::GroupAdd, PrivacyChoice::Nobody),
            ]
        );
    }

    #[test]
    fn a_failed_write_restores_the_confirmed_value() {
        let mut snapshot = Snapshot::demo();
        assert!(snapshot.begin_set(PrivacyKind::Profile, PrivacyChoice::Nobody));
        assert_eq!(
            snapshot.get(PrivacyKind::Profile),
            Some(PrivacyChoice::Nobody)
        );
        assert!(
            !snapshot.begin_set(PrivacyKind::Profile, PrivacyChoice::Everyone),
            "a second write waits for the first"
        );
        snapshot.fail_set(PrivacyKind::Profile);
        assert_eq!(
            snapshot.get(PrivacyKind::Profile),
            Some(PrivacyChoice::Contacts)
        );
        assert!(!snapshot.pending(PrivacyKind::Profile));
    }

    #[test]
    fn a_fetch_during_a_write_keeps_it_pending() {
        let mut snapshot = Snapshot::demo();
        assert!(snapshot.begin_set(PrivacyKind::Profile, PrivacyChoice::Nobody));
        snapshot.apply_fetch(
            vec![
                (PrivacyKind::Profile, PrivacyChoice::Everyone),
                (PrivacyKind::About, PrivacyChoice::Nobody),
            ],
            false,
        );
        assert!(snapshot.pending(PrivacyKind::Profile));
        assert_eq!(
            snapshot.get(PrivacyKind::Profile),
            Some(PrivacyChoice::Nobody)
        );
        assert_eq!(
            snapshot.get(PrivacyKind::About),
            Some(PrivacyChoice::Nobody)
        );
        // A refusal now restores what the fetch reported.
        snapshot.fail_set(PrivacyKind::Profile);
        assert_eq!(
            snapshot.get(PrivacyKind::Profile),
            Some(PrivacyChoice::Everyone)
        );
    }

    #[test]
    fn a_failed_fetch_keeps_the_values_but_stops_editing() {
        let mut snapshot = Snapshot::demo();
        assert!(snapshot.editable());
        snapshot.apply_fetch(Vec::new(), true);
        assert!(!snapshot.editable());
        assert_eq!(
            snapshot.get(PrivacyKind::Profile),
            Some(PrivacyChoice::Contacts)
        );
        snapshot.apply_fetch(vec![(PrivacyKind::Profile, PrivacyChoice::Nobody)], false);
        assert!(snapshot.editable());
        assert_eq!(
            snapshot.get(PrivacyKind::LastSeen),
            None,
            "the fetch is whole"
        );
    }

    #[test]
    fn nothing_is_written_for_an_unknown_or_unchanged_category() {
        let mut snapshot = Snapshot::default();
        assert!(!snapshot.begin_set(PrivacyKind::Profile, PrivacyChoice::Nobody));
        let mut snapshot = Snapshot::demo();
        assert!(!snapshot.begin_set(PrivacyKind::Profile, PrivacyChoice::Contacts));
        assert!(
            !snapshot.finish_set(PrivacyKind::Profile),
            "nothing was waiting"
        );
    }
}
