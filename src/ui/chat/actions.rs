use crate::i18n::Locale;
use crate::model::{Action, Chat, Dialog};
use crate::theme::Icon;

pub fn pin_action(chat: &Chat, locale: Locale) -> (Icon, String, Action) {
    let icon = if chat.pinned { Icon::PinOff } else { Icon::Pin };
    let label = if chat.pinned {
        crate::i18n::gettext(locale, "Unpin").into_owned()
    } else {
        crate::i18n::gettext(locale, "Pin to top").into_owned()
    };
    (icon, label, Action::SetPinned(chat.id.clone(), !chat.pinned))
}

pub fn archive_action(chat: &Chat, locale: Locale) -> (Icon, String, Action) {
    let label = if chat.archived {
        crate::i18n::gettext(locale, "Unarchive").into_owned()
    } else {
        crate::i18n::gettext(locale, "Archive").into_owned()
    };
    (Icon::Archive, label, Action::SetArchived(chat.id.clone(), !chat.archived))
}

pub fn favorite_action(chat: &Chat, locale: Locale) -> Option<(Icon, String, Action)> {
    if chat.is_channel() {
        return None;
    }
    let label = if chat.favorite {
        crate::i18n::gettext(locale, "Remove from favorites").into_owned()
    } else {
        crate::i18n::gettext(locale, "Add to favorites").into_owned()
    };
    Some((Icon::Heart, label, Action::SetFavorite(chat.id.clone(), !chat.favorite)))
}

pub fn leave_action(chat: &Chat, locale: Locale, our_ids: &[&str]) -> Option<(Icon, String, Action)> {
    if !chat.can_leave(our_ids) {
        return None;
    }
    let label = if chat.is_channel() {
        crate::i18n::gettext(locale, "Leave channel").into_owned()
    } else {
        crate::i18n::gettext(locale, "Leave group").into_owned()
    };
    Some((Icon::LogOut, label, Action::ShowDialog(Dialog::ConfirmLeaveGroup(chat.id.clone()))))
}

pub fn clear_chat_action(chat: &Chat, locale: Locale) -> (Icon, String, Action) {
    (
        Icon::Eraser,
        crate::i18n::gettext(locale, "Clear chat").into_owned(),
        Action::ShowDialog(Dialog::ConfirmClearChat(chat.id.clone())),
    )
}

pub fn delete_chat_action(chat: &Chat, locale: Locale) -> (Icon, String, Action) {
    (
        Icon::Trash,
        crate::i18n::gettext(locale, "Delete chat").into_owned(),
        Action::ShowDialog(Dialog::ConfirmDeleteChat(chat.id.clone())),
    )
}

pub fn copy_number_action(chat: &Chat, locale: Locale) -> Option<(Icon, String, Action)> {
    chat.phone().map(|phone| {
        (
            Icon::Copy,
            crate::i18n::gettext(locale, "Copy number").into_owned(),
            Action::CopyText(format!("+{phone}")),
        )
    })
}

pub fn lock_action(chat: &Chat, locale: Locale) -> (Icon, String, Action) {
    let icon = if chat.locked { Icon::LockOpen } else { Icon::Lock };
    let label = if chat.locked {
        crate::i18n::gettext(locale, "Unlock chat").into_owned()
    } else {
        crate::i18n::gettext(locale, "Lock chat").into_owned()
    };
    let action = if chat.locked {
        Action::SetLocked(chat.id.clone(), false)
    } else {
        Action::ShowDialog(Dialog::ConfirmLockChat(chat.id.clone()))
    };
    (icon, label, action)
}

pub fn mark_read_action(chat: &Chat, locale: Locale) -> (Icon, String, Action) {
    if chat.looks_unread() {
        (
            Icon::CheckCheck,
            crate::i18n::gettext(locale, "Mark as read").into_owned(),
            Action::MarkRead(chat.id.clone()),
        )
    } else {
        (
            Icon::MessageCircle,
            crate::i18n::gettext(locale, "Mark as unread").into_owned(),
            Action::MarkUnread(chat.id.clone()),
        )
    }
}
