//! The server's blocklist, kept in memory for one account only.

use std::collections::HashSet;

use crate::model::ChatId;

#[derive(Clone, Copy, Debug)]
pub enum Failure {
    Offline,
    Refused,
}

impl Failure {
    pub fn message(self, locale: crate::i18n::Locale) -> std::borrow::Cow<'static, str> {
        match self {
            Self::Offline => {
                crate::i18n::gettext(locale, "Connect to WhatsApp to block or unblock contacts.")
            }
            Self::Refused => crate::i18n::gettext(
                locale,
                "Could not change this contact's block. Try again, or use WhatsApp on your phone.",
            ),
        }
    }
}

/// Translate known worker errors at the UI boundary; preserve library errors.
pub(crate) fn localize_error(locale: crate::i18n::Locale, error: String) -> String {
    match error.as_str() {
        "Unblock this contact to send messages." => {
            crate::i18n::gettext(locale, "Unblock this contact to send messages.").into_owned()
        }
        "Unblock this contact to call them." => {
            crate::i18n::gettext(locale, "Unblock this contact to call them.").into_owned()
        }
        _ => error,
    }
}

#[derive(Clone, Debug, Default)]
pub struct Blocklist {
    /// `None` until a successful read; an empty set is a confirmed empty list.
    pub ids: Option<HashSet<ChatId>>,
    pub loading: bool,
    pub failed: bool,
    pub pending: bool,
    pub(crate) generation: u64,
}

impl Blocklist {
    pub fn contains(&self, chat: &str) -> bool {
        self.ids.as_ref().is_some_and(|ids| ids.contains(chat))
    }

    pub fn editable(&self) -> bool {
        self.ids.is_some() && !self.loading && !self.pending
    }

    /// A click may reach the worker before the UI sees an in-flight read.
    /// Supersede that read rather than silently losing the requested change.
    pub(crate) fn begin_change(&mut self) -> Option<u64> {
        if self.ids.is_none() || self.pending {
            return None;
        }
        self.invalidate();
        self.pending = true;
        Some(self.generation)
    }

    /// Discard older tasks while retaining the last confirmed list.
    pub(crate) fn invalidate(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.loading = false;
        self.pending = false;
    }
}

/// Only a person, addressed by a phone number or privacy id, can be blocked.
pub(crate) fn eligible(chat: &str, me: Option<&str>, me_lid: Option<&str>) -> bool {
    (chat.ends_with("@s.whatsapp.net") || chat.ends_with("@lid"))
        && Some(chat) != me
        && Some(chat) != me_lid
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_other_people_can_be_blocked() {
        let me = "15550001111@s.whatsapp.net";
        let lid = "200000000000001@lid";
        for id in [
            me,
            lid,
            "120363000000001@g.us",
            "status@broadcast",
            "123@newsletter",
            "unknown",
        ] {
            assert!(!eligible(id, Some(me), Some(lid)));
        }
        assert!(eligible("15550002222@s.whatsapp.net", Some(me), Some(lid)));
        assert!(eligible("200000000000002@lid", Some(me), Some(lid)));
    }
}
