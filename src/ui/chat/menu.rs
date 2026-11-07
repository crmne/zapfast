//! Message context menu, quick reactions, revoke checks, and shared contact parsing.

use super::*;

/// Emoji to send for a reaction choice: empty string removes our current one.
pub(crate) fn reaction_choice(current: Option<&str>, emoji: &str) -> String {
    if current == Some(emoji) {
        String::new()
    } else {
        emoji.to_owned()
    }
}

pub(crate) fn menu_width(
    ui: &egui::Ui,
    locale: crate::i18n::Locale,
    reaction_emoji: &[(String, u32)],
    message: &Message,
) -> f32 {
    let quick = quick_reactions(message, reaction_emoji).len() as f32 + 1.0;
    let labels = message_menu_labels(locale);
    let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
    widgets::menu_width(ui, &refs, true).max(quick * 36.0 + 12.0)
}

pub(crate) fn message_menu_labels(locale: crate::i18n::Locale) -> Vec<String> {
    use crate::i18n::gettext;
    [
        gettext(locale, "Reply"),
        gettext(locale, "Reply privately"),
        gettext(locale, "Star"),
        gettext(locale, "Unstar"),
        gettext(locale, "Pin message"),
        gettext(locale, "Unpin message"),
        gettext(locale, "Forward"),
        gettext(locale, "Open in the viewer"),
        gettext(locale, "Select"),
        gettext(locale, "Copy text"),
        gettext(locale, "Edit"),
        gettext(locale, "Delete for everyone"),
        gettext(locale, "Delete for me"),
        gettext(locale, "Add to favorites"),
        gettext(locale, "Open in system player"),
        gettext(locale, "Open file"),
        gettext(locale, "Copy image"),
        gettext(locale, "Save as…"),
        gettext(locale, "Show in folder"),
        gettext(locale, "Downloading…"),
        gettext(locale, "Download"),
        gettext(locale, "Message info"),
        gettext(locale, "Reload earlier messages"),
        gettext(locale, "Copy message ID"),
    ]
    .into_iter()
    .map(|label| label.into_owned())
    .collect()
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum MenuOrigin {
    Chat,
    List,
}

/// How a left-list row's message is marked, for the menu's Star label.
#[derive(Clone, Copy)]
pub(crate) struct Marks {
    pub starred: bool,
}

pub(crate) fn list_menu(
    app: &App,
    ui: &mut egui::Ui,
    chat: &Chat,
    message: &Message,
    marks: Marks,
    actions: &mut Vec<Action>,
) {
    let stars: HashSet<String> = marks
        .starred
        .then(|| message.id.clone())
        .into_iter()
        .collect();
    let names_or = |id: &str, hint: Option<&str>| app.display_name_or(id, hint);
    let mention_names = |id: &str| app.mention_name(id);
    let avatars = HashMap::new();
    let arrivals = HashMap::new();
    let keyboard_navigation = std::cell::Cell::new(false);
    let view = View {
        palette: app.palette,
        locale: app.locale,
        chat,
        me: app.me.as_deref(),
        auto_download: app.settings.auto_download,
        connected: app.link.is_connected(),
        poll_voting: &app.poll_voting,
        interactive_pending: &app.interactive_sending,
        // Nothing scrolls, highlights, or selects in a list row.
        anchor: None,
        open_menu: None,
        reaction: None,
        reaction_menu: false,
        reaction_emoji: &app.settings.reaction_emoji,
        keyboard_navigation: &keyboard_navigation,
        names_or: &names_or,
        mention_names: &mention_names,
        avatars: &avatars,
        contacts: &app.contacts,
        now: crate::util::now(),
        // A row drawn outside the conversation holds still.
        animate: false,
        player: &app.player,
        voice_speed: app.voice_speed_in(Some(&chat.id)),
        video: &app.video,
        account_receipts_off: app.account_receipts_off,
        copy_rows: Some(app.copy_rows.as_ref()),
        download_progress: app.download_progress.get(&chat.id),
        selecting: false,
        motion_playing: None,
        starred: marks.starred.then_some(&stars),
        pins: app.pins.get(&message.chat),
        pin_notices: &[],
        arrivals: &arrivals,
    };
    context_menu(ui, &view, message, MenuOrigin::List, actions);
}

/// Quick reactions plus our current reaction when needed.
pub(crate) fn quick_reactions<'a>(message: &'a Message, preferred: &'a [(String, u32)]) -> Vec<&'a str> {
    let mut list = Vec::new();
    for emoji in preferred
        .iter()
        .map(|(emoji, _)| emoji.as_str())
        .chain(QUICK_REACTIONS.iter().copied())
    {
        if emojis::get(emoji).is_some() && !list.contains(&emoji) {
            list.push(emoji);
        }
        if list.len() == QUICK_REACTIONS.len() {
            break;
        }
    }
    if let Some(mine) = own_reaction(message)
        && !list.contains(&mine)
    {
        list.push(mine);
    }
    list
}

pub(crate) fn viewer_file(message: &Message) -> Option<&Path> {
    message.content.gallery_kind()?;
    message
        .content
        .media()
        .and_then(|media| media.path.as_deref())
        .filter(|path| crate::image_preview::can_view(path) && path.is_file())
}

/// Checks every selected message's revoke eligibility with one scan of loaded history.
pub(crate) fn selection_can_revoke(messages: &[Message], selected: &[String], now: i64) -> bool {
    if selected.is_empty() {
        return false;
    }
    let mut remaining: std::collections::HashSet<&str> =
        selected.iter().map(String::as_str).collect();
    for message in messages {
        if remaining.remove(message.id.as_str()) {
            if !message.from_me
                || matches!(
                    message.content,
                    Content::Revoked | Content::DisappearingTimer { .. }
                )
                || now - message.timestamp > crate::app::REVOKE_WINDOW.as_secs() as i64
            {
                return false;
            }
            if remaining.is_empty() {
                return true;
            }
        }
    }
    false
}

/// Draws batch actions for the selected messages, including separate deletion choices.
pub(crate) fn selection_menu(
    ui: &mut egui::Ui,
    view: &View<'_>,
    messages: &[Message],
    selected: &[String],
    actions: &mut Vec<Action>,
) {
    selection_delete_items(
        ui,
        &view.palette,
        view.locale,
        &view.chat.id,
        selection_can_revoke(messages, selected, view.now),
        selected,
        actions,
    );
}

pub(crate) fn selection_delete_items(
    ui: &mut egui::Ui,
    palette: &Palette,
    locale: crate::i18n::Locale,
    chat: &str,
    can_revoke: bool,
    selected: &[String],
    actions: &mut Vec<Action>,
) {
    for (label, for_everyone, enabled) in [
        (
            crate::i18n::gettext(locale, "Delete for everyone"),
            true,
            can_revoke,
        ),
        (
            crate::i18n::gettext(locale, "Delete for me"),
            false,
            !selected.is_empty(),
        ),
    ] {
        if widgets::menu_item_enabled(ui, palette, Some(Icon::Trash), &label, enabled) {
            actions.push(Action::ShowDialog(Dialog::ConfirmDeleteSelection {
                chat: chat.to_owned(),
                messages: selected.to_vec(),
                for_everyone,
            }));
        }
    }
}

pub(crate) fn context_menu(
    ui: &mut egui::Ui,
    view: &View<'_>,
    message: &Message,
    origin: MenuOrigin,
    actions: &mut Vec<Action>,
) {
    let palette = view.palette;
    let chat = &view.chat.id;
    if let Content::Call { group, .. } = message.content {
        call_menu(ui, view, &message.id, group, actions);
        return;
    }
    let mine = own_reaction(message);
    ui.allocate_ui_with_layout(
        vec2(ui.available_width(), 34.0),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for emoji in quick_reactions(message, view.reaction_emoji) {
                let chosen = mine == Some(emoji);
                let line = widgets::line(ui, emoji, theme::regular(20.0), palette.text, 40.0, 1);
                let (rect, response) = ui.allocate_exact_size(Vec2::splat(34.0), Sense::click());
                // Screen readers speak the emoji's name.
                response.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), emoji)
                });
                theme::focus_outline(ui, response.id, rect, 17.0);
                if chosen {
                    ui.painter()
                        .circle_filled(rect.center(), 17.0, palette.surface_active);
                    ui.painter().circle_stroke(
                        rect.center(),
                        16.0,
                        Stroke::new(1.5, palette.accent),
                    );
                } else if response.hovered() {
                    ui.painter()
                        .circle_filled(rect.center(), 17.0, palette.surface_hover);
                }
                line.paint(ui, rect.center() - line.size() / 2.0, palette.text);
                let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
                let response = if chosen {
                    response.on_hover_text(
                        crate::i18n::gettext(view.locale, "Remove your reaction").into_owned(),
                    )
                } else {
                    response
                };
                if response.clicked() {
                    // Selecting our current reaction removes it.
                    actions.push(Action::React {
                        chat: chat.clone(),
                        message: message.id.clone(),
                        emoji: reaction_choice(mine, emoji),
                    });
                    ui.close();
                }
            }
            let (rect, response) = ui.allocate_exact_size(Vec2::splat(34.0), Sense::click());
            // The tooltip doubles as the screen reader's label.
            let more = crate::i18n::gettext(view.locale, "React with any emoji").into_owned();
            response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), &more)
            });
            theme::focus_outline(ui, response.id, rect, 17.0);
            if ui.is_rect_visible(rect) {
                let hovered = response.hovered();
                ui.painter().circle_filled(
                    rect.center(),
                    17.0,
                    if hovered {
                        palette.surface_hover
                    } else {
                        palette.surface
                    },
                );
                ui.painter()
                    .circle_stroke(rect.center(), 16.0, Stroke::new(1.0, palette.outline));
                theme::paint_icon(ui, Icon::Plus, rect, 16.0, palette.secondary);
            }
            if response
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text(more)
                .clicked()
            {
                actions.push(Action::OpenReactionPicker {
                    chat: chat.clone(),
                    message: message.id.clone(),
                    beside_menu: true,
                    from_list: false,
                });
            }
        },
    );
    widgets::menu_separator(ui, &palette);
    let mut first = None;
    let mut item = |ui: &mut egui::Ui, icon: Icon, label: &str| {
        let response = widgets::menu_item_response(ui, &palette, Some(icon), label, true);
        first.get_or_insert(response.id);
        response.clicked()
    };
    if can_reply(&message.content)
        && item(ui, Icon::Reply, &crate::i18n::gettext(view.locale, "Reply"))
    {
        if matches!(origin, MenuOrigin::List) {
            actions.push(Action::OpenMessage {
                chat: chat.clone(),
                message: message.id.clone(),
            });
        }
        actions.push(Action::Reply(message.id.clone()));
    }
    if !matches!(origin, MenuOrigin::List)
        && message.private_reply_recipient().is_some()
        && item(
            ui,
            Icon::Reply,
            &crate::i18n::gettext(view.locale, "Reply privately"),
        )
    {
        actions.push(Action::ReplyPrivately {
            chat: chat.clone(),
            message: message.id.clone(),
        });
    }
    let starred = view.starred.is_some_and(|ids| ids.contains(&message.id));
    let star_label = if starred {
        crate::i18n::gettext(view.locale, "Unstar")
    } else {
        crate::i18n::gettext(view.locale, "Star")
    };
    if !matches!(message.content, Content::Revoked) && item(ui, Icon::Star, &star_label) {
        actions.push(Action::SetStar {
            chat: chat.clone(),
            message: message.id.clone(),
            starred: !starred,
        });
        ui.close();
    }
    let pinned = view.pins.is_some_and(|ids| ids.contains(&message.id));
    let pin_label = if pinned {
        crate::i18n::gettext(view.locale, "Unpin message")
    } else {
        crate::i18n::gettext(view.locale, "Pin message")
    };
    if !matches!(message.content, Content::Revoked) && item(ui, Icon::Pin, &pin_label) {
        actions.push(Action::SetMessagePinned {
            chat: chat.clone(),
            message: message.id.clone(),
            pinned: !pinned,
        });
    }
    if crate::app::can_select(&message.content)
        && item(
            ui,
            Icon::Forward,
            &crate::i18n::gettext(view.locale, "Forward"),
        )
    {
        actions.push(forward_action(chat, &message.id));
    }
    if let Some(path) = viewer_file(message)
        && item(
            ui,
            Icon::Maximize,
            &crate::i18n::gettext(view.locale, "Open in the viewer"),
        )
    {
        actions.push(Action::PreviewImage {
            path: path.to_owned(),
            chat: chat.clone(),
            message: message.id.clone(),
            card: None,
        });
    }
    if crate::app::can_select(&message.content)
        && item(
            ui,
            Icon::Check,
            &crate::i18n::gettext(view.locale, "Select"),
        )
    {
        if matches!(origin, MenuOrigin::List) {
            actions.push(Action::OpenMessage {
                chat: chat.clone(),
                message: message.id.clone(),
            });
        }
        actions.push(Action::SelectMessage(message.id.clone()));
    }
    let keys_menu = ui
        .ctx()
        .data(|data| data.get_temp::<bool>(keys_menu_id(ui.layer_id().id)))
        .unwrap_or(false);
    let entries = if keys_menu {
        inner_entries(view, message)
    } else {
        Vec::new()
    };
    if !entries.is_empty() {
        widgets::menu_separator(ui, &palette);
        for entry in entries {
            let accessible = match entry.hint {
                Some(hint) if !entry.enabled => crate::a11y::fill(
                    &crate::i18n::gettext(view.locale, "{option}, unavailable: {reason}"),
                    &[
                        ("option", &entry.label),
                        ("reason", &hint.said(view.locale)),
                    ],
                ),
                _ => entry.label.clone(),
            };
            let response = widgets::menu_item_focusable(
                ui,
                &palette,
                Some(entry.icon),
                &entry.label,
                &accessible,
                entry.enabled,
                entry.enabled,
            );
            if entry.enabled {
                first.get_or_insert(response.id);
            }
            let clicked = response.clicked();
            if let Some(hint) = entry.hint {
                response.on_hover_text(hint.shown());
            }
            if clicked && let Some(action) = entry.action {
                actions.push(action);
            }
        }
        widgets::menu_separator(ui, &palette);
    }
    if let Some(text) = copyable_text(&message.content)
        && widgets::menu_item(
            ui,
            &palette,
            Some(Icon::Copy),
            &crate::i18n::gettext(view.locale, "Copy text"),
        )
    {
        let mentions = mentions_of(view, message);
        actions.push(Action::CopyText(markup::plain(&text, &mentions)));
    }
    let age = view.now - message.timestamp;
    let can_edit = message.from_me
        && matches!(message.content, Content::Text { .. })
        && age <= crate::app::EDIT_WINDOW.as_secs() as i64;
    let can_revoke = revoke_blocker(message, view.now).is_none();
    if can_edit
        && widgets::menu_item(
            ui,
            &palette,
            Some(Icon::Pencil),
            &crate::i18n::gettext(view.locale, "Edit"),
        )
    {
        if matches!(origin, MenuOrigin::List) {
            actions.push(Action::OpenMessage {
                chat: chat.clone(),
                message: message.id.clone(),
            });
        }
        actions.push(Action::Edit(message.id.clone()));
    }
    if can_revoke
        && widgets::menu_item(
            ui,
            &palette,
            Some(Icon::Trash),
            &crate::i18n::gettext(view.locale, "Delete for everyone"),
        )
    {
        actions.push(delete_for_everyone_action(chat, &message.id));
    }
    let delete_for_me = widgets::menu_item_response(
        ui,
        &palette,
        Some(Icon::EyeOff),
        &crate::i18n::gettext(view.locale, "Delete for me"),
        true,
    );
    first.get_or_insert(delete_for_me.id);
    let first = (!ui.is_sizing_pass()).then_some(first).flatten();
    ui.ctx().data_mut(|data| {
        data.insert_temp(menu_first_item_id(chat, &message.id), first);
    });
    if delete_for_me.clicked() {
        actions.push(delete_for_me_action(chat, &message.id));
    }
    if let Content::Sticker { media, .. } = &message.content
        && let Some(path) = &media.path
        && path.extension().is_none_or(|extension| extension != "was")
        && widgets::menu_item(
            ui,
            &palette,
            Some(Icon::Star),
            &crate::i18n::gettext(view.locale, "Add to favorites"),
        )
    {
        actions.push(Action::SaveSticker(path.clone()));
    }
    if let Some(media) = message.content.media() {
        match &media.path {
            Some(path) => {
                let open = if matches!(message.content, Content::Video { gif: false, .. }) {
                    crate::i18n::gettext(view.locale, "Open in system player")
                } else {
                    crate::i18n::gettext(view.locale, "Open file")
                };
                if widgets::menu_item(ui, &palette, Some(Icon::ExternalLink), &open) {
                    actions.push(Action::OpenFile(path.clone()));
                }
                if matches!(message.content, Content::Image { .. })
                    && widgets::menu_item(
                        ui,
                        &palette,
                        Some(Icon::Copy),
                        &crate::i18n::gettext(view.locale, "Copy image"),
                    )
                {
                    actions.push(Action::CopyImage(path.clone()));
                }
                if widgets::menu_item(
                    ui,
                    &palette,
                    Some(Icon::Download),
                    &crate::i18n::gettext(view.locale, "Save as…"),
                ) {
                    actions.push(Action::SaveAttachmentAs {
                        path: path.clone(),
                        name: attachment_name(&message.content, path),
                    });
                }
                if let Some(folder) = path.parent()
                    && widgets::menu_item(
                        ui,
                        &palette,
                        Some(Icon::FileText),
                        &crate::i18n::gettext(view.locale, "Show in folder"),
                    )
                {
                    actions.push(Action::OpenFolder(folder.to_path_buf()));
                }
            }
            None => {
                let downloading = matches!(media.state, MediaState::Downloading);
                if widgets::menu_item_enabled(
                    ui,
                    &palette,
                    Some(Icon::Download),
                    &if downloading {
                        crate::i18n::gettext(view.locale, "Downloading…")
                    } else {
                        crate::i18n::gettext(view.locale, "Download")
                    },
                    !downloading,
                ) {
                    actions.push(Action::Download {
                        manual: true,
                        card: None,
                        chat: chat.clone(),
                        message: message.id.clone(),
                    });
                }
            }
        }
    }
    if let Content::Audio { media, .. } = &message.content
        && media.path.is_some()
    {
        speed_menu_row(ui, view, message, actions);
    }
    widgets::menu_separator(ui, &palette);
    if has_message_info(message)
        && widgets::menu_item(
            ui,
            &palette,
            Some(Icon::Info),
            &crate::i18n::gettext(view.locale, "Message info"),
        )
    {
        actions.push(Action::ShowDialog(Dialog::MessageInfo {
            chat: chat.clone(),
            message: message.id.clone(),
        }));
    }
    if widgets::menu_item_enabled(
        ui,
        &palette,
        Some(Icon::Refresh),
        &crate::i18n::gettext(view.locale, "Reload earlier messages"),
        view.connected && !matches!(message.status, Delivery::Pending | Delivery::Failed),
    ) {
        actions.push(Action::ReloadHistory {
            chat: chat.clone(),
            message: message.id.clone(),
        });
    }
    // The id helps when looking a message up for a bug report.
    if widgets::menu_item(
        ui,
        &palette,
        Some(Icon::Copy),
        &crate::i18n::gettext(view.locale, "Copy message ID"),
    ) {
        actions.push(Action::CopyText(message.id.clone()));
    }
}

pub(crate) struct InnerEntry {
    pub(crate) label: String,
    pub(crate) icon: Icon,
    /// What choosing it does. `None` for an entry that has nothing to do.
    pub(crate) action: Option<Action>,
    pub(crate) enabled: bool,
    /// Why the entry is unavailable, shown on hover as in the bubble.
    pub(crate) hint: Option<ButtonBlocker>,
}

pub(crate) fn button_entries(
    view: &View<'_>,
    message: &Message,
    card: &crate::model::InteractiveCard,
    card_number: Option<usize>,
    entries: &mut Vec<InnerEntry>,
) {
    use crate::model::InteractiveAction;
    for (index, button) in card.buttons.iter().enumerate() {
        let enabled = interactive_enabled(view, message, button);
        let icon = if button.url.is_some() {
            Icon::ExternalLink
        } else {
            match button.action {
                InteractiveAction::Reply => Icon::Reply,
                InteractiveAction::Select(_) => Icon::ListChecks,
                InteractiveAction::Copy(_) => Icon::Copy,
                InteractiveAction::Unavailable => Icon::Smartphone,
            }
        };
        let label = match card_number {
            Some(number) => crate::a11y::fill(
                &crate::i18n::gettext(view.locale, "Card {number}: {button}"),
                &[
                    ("number", &(number + 1).to_string()),
                    ("button", &button.label),
                ],
            ),
            None => button.label.clone(),
        };
        entries.push(InnerEntry {
            label,
            icon,
            action: enabled
                .then(|| interactive_action(message, index, button))
                .flatten(),
            enabled,
            hint: (!enabled).then(|| interactive_reason(view, message, button)),
        });
    }
}

pub(crate) fn inner_entries(view: &View<'_>, message: &Message) -> Vec<InnerEntry> {
    let mut entries = Vec::new();
    if let Content::Interactive {
        card: Some(card), ..
    } = &message.content
    {
        button_entries(view, message, card, None, &mut entries);
        for (index, child) in card.carousel.iter().enumerate() {
            let row = carousel_row(message, index, child);
            button_entries(view, &row, child, Some(index), &mut entries);
        }
    }
    if let Some(quoted) = &message.quoted {
        entries.push(InnerEntry {
            label: crate::i18n::gettext(view.locale, "Go to replied message").into_owned(),
            icon: Icon::Reply,
            action: Some(quote_key_action(quoted)),
            enabled: true,
            hint: None,
        });
    }
    if shows_sender_pictures(view.chat) && !message.from_me {
        let name = (view.names_or)(&message.sender, message.sender_name.as_deref());
        entries.push(InnerEntry {
            label: crate::a11y::fill(
                &crate::i18n::gettext(view.locale, "Info about {name}"),
                &[("name", name.trim_start_matches('~'))],
            ),
            icon: Icon::Info,
            action: Some(sender_info_action(message)),
            enabled: true,
            hint: None,
        });
    }
    entries
}

/// Where a message menu's popup records whether the keys opened it.
pub(crate) fn keys_menu_id(popup: egui::Id) -> egui::Id {
    popup.with("opened-by-keys")
}

/// What clicking a group sender's name or picture does: their info.
pub(crate) fn sender_info_action(message: &Message) -> Action {
    Action::OpenInfo(message.sender.clone())
}

/// Where the message menu records its first item for this pass.
pub(crate) fn menu_first_item_id(chat: &str, message: &str) -> egui::Id {
    bubble_id(chat, message).with("menu-first-item")
}

pub(crate) fn can_reply(content: &Content) -> bool {
    !matches!(content, Content::Revoked | Content::Call { .. })
}

/// The message menu's Forward: the forward dialog for this one message.
pub(crate) fn forward_action(chat: &str, message: &str) -> Action {
    Action::ShowDialog(Dialog::Forward {
        chat: chat.to_owned(),
        messages: vec![message.to_owned()],
    })
}

/// The message menu's "Delete for me": its confirmation, never the deletion.
pub(crate) fn delete_for_me_action(chat: &str, message: &str) -> Action {
    Action::ShowDialog(Dialog::ConfirmDeleteMessage {
        chat: chat.to_owned(),
        message: message.to_owned(),
        for_everyone: false,
    })
}

pub(crate) fn delete_for_everyone_action(chat: &str, message: &str) -> Action {
    Action::ShowDialog(Dialog::ConfirmDeleteMessage {
        chat: chat.to_owned(),
        message: message.to_owned(),
        for_everyone: true,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RevokeBlocker {
    NotOurs,
    Deleted,
    TooOld,
}

impl RevokeBlocker {
    #[cfg(test)]
    pub(crate) const ALL: [Self; 3] = [Self::NotOurs, Self::Deleted, Self::TooOld];

    /// The reason, translated, as Shift+Delete says it.
    pub(crate) fn said(self, locale: crate::i18n::Locale) -> String {
        use crate::i18n::gettext;
        match self {
            Self::NotOurs => gettext(locale, "Only messages you sent"),
            Self::Deleted => gettext(locale, "This message was deleted"),
            Self::TooOld => gettext(locale, "The time limit has passed"),
        }
        .into_owned()
    }
}

pub(crate) fn revoke_blocker(message: &Message, now: i64) -> Option<RevokeBlocker> {
    if !message.from_me {
        Some(RevokeBlocker::NotOurs)
    } else if matches!(message.content, Content::Revoked) {
        Some(RevokeBlocker::Deleted)
    } else if now - message.timestamp > crate::app::REVOKE_WINDOW.as_secs() as i64 {
        Some(RevokeBlocker::TooOld)
    } else {
        None
    }
}

pub(crate) fn revoke_refusal_said(locale: crate::i18n::Locale, blocker: RevokeBlocker) -> String {
    crate::a11y::fill(
        &crate::i18n::gettext(locale, "{option}, unavailable: {reason}"),
        &[
            (
                "option",
                &crate::i18n::gettext(locale, "Delete for everyone"),
            ),
            ("reason", &blocker.said(locale)),
        ],
    )
}

pub(crate) fn copyable_text(content: &Content) -> Option<String> {
    match content {
        Content::Text { text, .. } | Content::Interactive { text, .. } => Some(text.clone()),
        Content::Image { caption, .. }
        | Content::Video { caption, .. }
        | Content::Document { caption, .. } => caption.clone(),
        Content::Location {
            latitude,
            longitude,
            ..
        }
        | Content::LiveLocation {
            latitude,
            longitude,
            ..
        } => Some(format!("{latitude},{longitude}")),
        Content::Contact { vcard, .. } => Some(vcard.clone()),
        _ => None,
    }
}

pub(crate) fn copy_key_action(
    message: &Message,
    mention_names: &dyn Fn(&str) -> String,
) -> Option<Action> {
    if let Some(text) = copyable_text(&message.content) {
        let mentions = mentions_named(message, mention_names);
        return Some(Action::CopyText(markup::plain(&text, &mentions)));
    }
    match &message.content {
        Content::Image { media, .. } => media.path.clone().map(Action::CopyImage),
        _ => None,
    }
}

pub(crate) fn quote_jump_action(quoted: &crate::model::Quoted) -> Action {
    match &quoted.chat {
        Some(chat) => Action::OpenMessage {
            chat: chat.clone(),
            message: quoted.id.clone(),
        },
        None => Action::ScrollTo(quoted.id.clone()),
    }
}

pub(crate) fn quote_key_action(quoted: &crate::model::Quoted) -> Action {
    match &quoted.chat {
        Some(_) => quote_jump_action(quoted),
        None => Action::GoToReplied(quoted.id.clone()),
    }
}

pub(crate) fn mentions_of(view: &View<'_>, message: &Message) -> Vec<markup::Mention> {
    mentions_named(message, view.mention_names)
}

pub(crate) fn mentions_named(
    message: &Message,
    mention_names: &dyn Fn(&str) -> String,
) -> Vec<markup::Mention> {
    message
        .mentions
        .iter()
        .map(|mention| markup::Mention {
            user: mention.user.clone(),
            name: mention_names(&mention.id),
        })
        .collect()
}

pub(crate) fn quote_mentions(view: &View<'_>, quoted: &crate::model::Quoted) -> Vec<markup::Mention> {
    quoted
        .mentions
        .iter()
        .map(|mention| markup::Mention {
            user: mention.user.clone(),
            name: (view.mention_names)(&mention.id),
        })
        .collect()
}

pub(crate) fn vcard_tel_preference(property: &str) -> Option<u8> {
    let mut best = None;
    for parameter in property.split(';').skip(1) {
        let (name, value) = parameter.split_once('=').unwrap_or(("TYPE", parameter));
        let value = value.trim_matches('"');
        let rank = if name.eq_ignore_ascii_case("PREF") {
            value
                .parse::<u8>()
                .ok()
                .filter(|rank| (1..=100).contains(rank))
        } else if name.eq_ignore_ascii_case("TYPE")
            && value
                .split(',')
                .any(|value| value.eq_ignore_ascii_case("PREF"))
        {
            Some(1)
        } else {
            None
        };
        if let Some(rank) = rank {
            best = Some(best.map_or(rank, |current: u8| current.min(rank)));
        }
    }
    best
}

#[derive(Debug, PartialEq)]
pub(crate) struct SharedContact {
    pub(crate) name: String,
    pub(crate) number: String,
    pub(crate) account: Option<String>,
    pub(crate) first_name: Option<String>,
}

pub(crate) fn vcard_first_name(value: &str) -> Option<String> {
    let mut parts = vec![String::new()];
    let mut chars = value.chars();
    while let Some(character) = chars.next() {
        match character {
            '\\' => match chars.next() {
                Some('n' | 'N') => parts.last_mut()?.push(' '),
                Some(escaped) => parts.last_mut()?.push(escaped),
                None => {}
            },
            ';' => parts.push(String::new()),
            _ => parts.last_mut()?.push(character),
        }
    }
    let part = |index: usize| parts.get(index).map_or("", |part| part.trim());
    let first = [part(1), part(2)]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    (!first.is_empty()).then_some(first)
}

pub(crate) fn vcard_tel_account(property: &str, value: &str) -> Option<String> {
    let digits = |text: &str| {
        text.chars()
            .filter(char::is_ascii_digit)
            .collect::<String>()
    };
    let waid = property.split(';').skip(1).find_map(|parameter| {
        let (name, value) = parameter.split_once('=')?;
        name.eq_ignore_ascii_case("WAID")
            .then(|| digits(value.trim_matches('"')))
    });
    waid.or_else(|| value.trim().starts_with('+').then(|| digits(value)))
        .filter(|account| account.len() >= 7)
}

pub(crate) fn shared_contact_details(vcard: &str, fallback_name: &str) -> Option<SharedContact> {
    let mut name = None;
    let mut first_name = None;
    let mut first_phone = None;
    let mut preferred_phone: Option<(u8, (String, Option<String>))> = None;
    let mut first_card: Vec<String> = Vec::new();
    for line in vcard.lines() {
        let line = line.trim_end_matches('\r');
        if line.starts_with([' ', '\t']) {
            if let Some(previous) = first_card.last_mut() {
                previous.push_str(line.trim_start());
            }
            continue;
        }
        if line.eq_ignore_ascii_case("END:VCARD") && !first_card.is_empty() {
            break;
        }
        if line.eq_ignore_ascii_case("BEGIN:VCARD") {
            if first_card.is_empty() {
                first_card.push(line.to_owned());
            }
            continue;
        }
        if !first_card.is_empty() {
            first_card.push(line.to_owned());
        }
    }
    if first_card.is_empty() {
        first_card.extend(vcard.lines().map(str::to_owned));
    }
    for line in first_card {
        let Some((property, value)) = line.split_once(':') else {
            continue;
        };
        let raw_name = property.split(';').next().unwrap_or(property);
        let property_name = raw_name.rsplit('.').next().unwrap_or(raw_name);
        if property_name.eq_ignore_ascii_case("FN") {
            name = Some(value.trim().to_owned());
        } else if property_name.eq_ignore_ascii_case("N") {
            first_name = vcard_first_name(value);
        } else if property_name.eq_ignore_ascii_case("TEL") {
            let number = value.trim().to_owned();
            if number.chars().filter(char::is_ascii_digit).count() < 7 {
                continue;
            }
            let phone = (number, vcard_tel_account(property, value));
            if let Some(rank) = vcard_tel_preference(property) {
                let replace = preferred_phone
                    .as_ref()
                    .is_none_or(|(best_rank, _)| rank < *best_rank);
                if replace {
                    preferred_phone = Some((rank, phone));
                }
            } else if first_phone.is_none() {
                first_phone = Some(phone);
            }
        }
    }
    let (number, account) = preferred_phone.map(|(_, phone)| phone).or(first_phone)?;
    let name = name
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| fallback_name.to_owned());
    Some(SharedContact {
        name,
        number,
        account,
        first_name,
    })
}
