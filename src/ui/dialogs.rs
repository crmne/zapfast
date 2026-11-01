//! Shortcuts, account, linking, contact, and chat dialogs.

use egui::{Align, CornerRadius, Frame, Layout, Margin, Rect, Sense, Stroke, pos2, vec2};

use crate::app::App;
use crate::model::{Action, Dialog};
use crate::theme::{self, Icon};

/// The dialogs' `Modal`, the layer they are drawn on.
pub(crate) fn modal_id() -> egui::Id {
    egui::Id::new("dialog")
}

pub fn show(app: &mut App, ctx: &egui::Context) {
    let Some(dialog) = app.dialog.clone() else {
        return;
    };
    let palette = app.palette;
    let frame = Frame::new()
        .fill(palette.overlay)
        .stroke(Stroke::new(1.0, palette.outline))
        .corner_radius(CornerRadius::same(theme::RADIUS + 4))
        .inner_margin(Margin::same(if matches!(dialog, Dialog::Forward { .. }) {
            12
        } else {
            22
        }))
        .shadow(palette.modal_shadow());
    let response = egui::Modal::new(modal_id())
        .frame(frame)
        .backdrop_color(palette.shadow)
        .show(ctx, |ui| {
            ui.set_width(match dialog {
                Dialog::Shortcuts => shortcuts_width(ui.ctx().content_rect().width()),
                Dialog::About => 380.0,
                Dialog::ConfirmUnlink => 380.0,
                Dialog::ConfirmRemoveAccount(_) => 380.0,
                Dialog::ConfirmClearMediaCache(_) => 380.0,
                Dialog::ConfirmLeaveGroup(_) => 380.0,
                Dialog::PairWithPhone => 380.0,
                Dialog::NewContact => 380.0,
                Dialog::NewChat => 420.0,
                Dialog::UnlockLockedChats | Dialog::ConfirmLockChat(_) => 380.0,
                Dialog::ConfirmDeleteChat(_) | Dialog::JoinGroup | Dialog::ConfirmStartOver => {
                    380.0
                }
                Dialog::ConfirmClearChat(_) => 380.0,
                Dialog::ConfirmDeleteMessage { .. } | Dialog::ConfirmDeleteSelection { .. } => {
                    380.0
                }
                Dialog::ConfirmRemoveStickerPack { .. } => 380.0,
                Dialog::StickerPack => 420.0,
                Dialog::StickerMaker => 400.0,
                Dialog::Forward { .. } => {
                    let width = ui.ctx().content_rect().width();
                    (width * 0.6)
                        .clamp(420.0, 760.0)
                        .min((width - 48.0).max(1.0))
                }
                Dialog::CreatePoll(_) => 420.0,
                Dialog::SendLocation(_) => 420.0,
                Dialog::PollResults { .. }
                | Dialog::InteractiveList { .. }
                | Dialog::MessageInfo { .. } => {
                    420.0_f32.min((ui.ctx().content_rect().width() - 64.0).max(180.0))
                }
                Dialog::Labels => 460.0,
                Dialog::AddGroupMembers(_) => 420.0,
                Dialog::ConfirmResetInviteLink(_) | Dialog::ConfirmRemoveMember { .. } => 380.0,
            });
            ui.spacing_mut().item_spacing.y = 8.0;
            match dialog {
                Dialog::CreatePoll(chat) => super::polls::create(app, ui, &chat),
                Dialog::SendLocation(chat) => super::locations::send(app, ui, &chat),
                Dialog::PollResults { chat, message } => {
                    super::polls::results(app, ui, &chat, &message)
                }
                Dialog::InteractiveList {
                    chat,
                    message,
                    button,
                } => interactive_list(app, ui, &chat, &message, button),
                Dialog::Labels => super::labels::manager(app, ui, &palette),
                Dialog::Shortcuts => shortcuts(app, ui),
                Dialog::About => about(app, ui),
                Dialog::ConfirmUnlink => confirm_unlink(app, ui),
                Dialog::ConfirmRemoveAccount(id) => confirm_remove(app, ui, id),
                Dialog::ConfirmClearMediaCache(id) => confirm_clear_media(app, ui, id),
                Dialog::ConfirmLeaveGroup(id) => confirm_leave_group(app, ui, &id),
                Dialog::PairWithPhone => pair_with_phone(app, ui),
                Dialog::NewContact => new_contact(app, ui),
                Dialog::NewChat => new_chat(app, ui),
                Dialog::UnlockLockedChats => unlock_locked_chats(app, ui),
                Dialog::ConfirmLockChat(id) => confirm_lock_chat(app, ui, &id),
                Dialog::ConfirmDeleteChat(id) => confirm_delete_chat(app, ui, &id),
                Dialog::ConfirmClearChat(id) => confirm_clear_chat(app, ui, &id),
                Dialog::ConfirmDeleteMessage {
                    chat,
                    message,
                    for_everyone,
                } => confirm_delete_message(app, ui, &chat, &message, for_everyone),
                Dialog::ConfirmRemoveStickerPack { name, dir } => {
                    confirm_remove_sticker_pack(app, ui, &name, dir)
                }
                Dialog::ConfirmDeleteSelection {
                    chat,
                    messages,
                    for_everyone,
                } => {
                    confirm_delete_messages(app, ui, &chat, &messages, for_everyone, true);
                }
                Dialog::Forward { messages, .. } => forward(app, ui, &messages),
                Dialog::JoinGroup => join_group(app, ui),
                Dialog::ConfirmStartOver => confirm_start_over(app, ui),
                Dialog::StickerPack => sticker_pack(app, ui),
                Dialog::StickerMaker => sticker_maker(app, ui),
                Dialog::MessageInfo { chat, message } => {
                    super::message_info::show(app, ui, &chat, &message)
                }
                Dialog::AddGroupMembers(_) => {
                    if app.new_group.is_some() {
                        super::new_group::show(app, ui);
                    } else {
                        app.actions.push(Action::CloseDialog);
                    }
                }
                Dialog::ConfirmResetInviteLink(chat) => confirm_reset_invite_link(app, ui, &chat),
                Dialog::ConfirmRemoveMember { chat, member } => {
                    confirm_remove_member(app, ui, &chat, &member)
                }
            }
        });
    #[cfg(any(test, feature = "demo"))]
    if matches!(app.dialog, Some(Dialog::Forward { .. })) {
        ctx.data_mut(|data| {
            data.insert_temp(
                egui::Id::new(("forward-layout", "modal")),
                response.response.rect,
            )
        });
    }
    if response.should_close() {
        app.actions.push(Action::CloseDialog);
    }
}

fn interactive_list(app: &mut App, ui: &mut egui::Ui, chat: &str, message: &str, button: usize) {
    use super::widgets;
    use crate::model::{Content, InteractiveAction};

    let palette = app.palette;
    let row = app
        .conversations
        .get(chat)
        .and_then(|c| c.message(message))
        .cloned();
    let selected = row.as_ref().and_then(|row| match &row.content {
        Content::Interactive {
            card: Some(card), ..
        } => card.buttons.get(button),
        _ => None,
    });
    ui.horizontal(|ui| {
        let default_label = crate::i18n::gettext(app.locale, "Choose an option");
        let label = selected.map_or(default_label.as_ref(), |button| button.label.as_str());
        let heading = widgets::line(
            ui,
            label,
            theme::semibold(18.0),
            palette.text,
            (ui.available_width() - 36.0).max(1.0),
            2,
        );
        let (rect, _) = ui.allocate_exact_size(heading.size(), Sense::hover());
        heading.paint(ui, rect.min, palette.text);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if theme::icon_button(
                ui,
                Icon::X,
                16.0,
                palette.secondary,
                palette.text,
                &crate::i18n::gettext(app.locale, "Close"),
            )
            .clicked()
            {
                app.actions.push(Action::CloseDialog);
            }
        });
    });
    let Some(InteractiveAction::Select(options)) = selected.map(|button| &button.action) else {
        widgets::rich_text(
            ui,
            &crate::i18n::gettext(app.locale, "This list is no longer available."),
            theme::regular(14.0),
            palette.secondary,
        );
        return;
    };
    let available = row.as_ref().is_some_and(|row| !row.from_me && !row.edited)
        && app.chat(chat).is_some_and(|chat| chat.can_send());
    let pending = app
        .interactive_sending
        .contains(&(chat.to_owned(), message.to_owned()));
    let enabled = available && app.link.is_connected() && !pending;
    if !enabled {
        let reason = if !available {
            crate::i18n::gettext(app.locale, "This list can no longer receive replies.")
        } else if pending {
            crate::i18n::gettext(app.locale, "Sending reply…")
        } else {
            crate::i18n::gettext(app.locale, "Connect to WhatsApp to reply")
        };
        widgets::rich_text(ui, &reason, theme::regular(13.0), palette.secondary);
    }
    ui.add_space(8.0);
    let height = (ui.ctx().content_rect().height() - 200.0).clamp(100.0, 420.0);
    egui::ScrollArea::vertical()
        .id_salt(("interactive-list", chat, message, button))
        .max_height(height)
        .min_scrolled_height(height)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.add_enabled_ui(enabled, |ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                let mut section = "";
                for (choice, option) in options.iter().enumerate() {
                    if section != option.section {
                        section = &option.section;
                        if !section.is_empty() {
                            ui.add_space(8.0);
                            widgets::rich_text(
                                ui,
                                section,
                                theme::semibold(12.5),
                                palette.secondary,
                            );
                            ui.add_space(4.0);
                        }
                    }
                    let id = super::conversation::bubble_id(chat, message).with((
                        "interactive-option",
                        button,
                        choice,
                    ));
                    if interactive_option(ui, &palette, option, id).clicked() {
                        app.actions.push(Action::ReplyInteractive {
                            chat: chat.to_owned(),
                            message: message.to_owned(),
                            button,
                            choice: Some(choice),
                        });
                        app.actions.push(Action::CloseDialog);
                    }
                }
            });
        });
}

/// List choices preserve emoji and wrap their descriptions without clipping.
fn interactive_option(
    ui: &mut egui::Ui,
    palette: &crate::theme::Palette,
    option: &crate::model::InteractiveOption,
    id: egui::Id,
) -> egui::Response {
    let width = ui.available_width().max(1.0);
    let title = super::widgets::line(
        ui,
        &option.title,
        theme::medium(14.0),
        palette.text,
        (width - 52.0).max(1.0),
        2,
    );
    let description = (!option.description.is_empty()).then(|| {
        super::widgets::line(
            ui,
            &option.description,
            theme::regular(12.5),
            palette.secondary,
            (width - 52.0).max(1.0),
            3,
        )
    });
    let height =
        (title.size().y + description.as_ref().map_or(0.0, |line| line.size().y + 4.0) + 20.0)
            .max(60.0);
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    let sense = if ui.is_enabled() {
        Sense::click()
    } else {
        Sense::hover()
    };
    let response = ui.interact(rect, id, sense);
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), &option.title)
    });
    ui.ctx().data_mut(|data| data.insert_temp(id, rect));
    if ui.is_enabled() && (response.hovered() || response.has_focus()) {
        ui.painter().rect_filled(
            rect,
            8.0,
            palette
                .text
                .gamma_multiply(if response.is_pointer_button_down_on() {
                    0.10
                } else {
                    0.06
                }),
        );
    }
    ui.painter().circle_stroke(
        egui::pos2(rect.right() - 18.0, rect.center().y),
        7.0,
        Stroke::new(1.5, palette.secondary),
    );
    theme::reveal_focus(&response);
    let pos = rect.min + vec2(10.0, 10.0);
    title.paint(ui, pos, palette.text);
    if let Some(description) = description {
        description.paint(ui, pos + vec2(0.0, title.size().y + 4.0), palette.secondary);
    }
    if ui.is_enabled() {
        response.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        response
    }
}

fn unlock_locked_chats(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    let setup = app.settings.chat_lock_code_hash.is_none();
    let title_text = if setup {
        crate::i18n::gettext(app.locale, "Set up locked chats")
    } else {
        crate::i18n::gettext(app.locale, "Locked chats")
    };
    title(ui, app, &title_text);
    let desc_text = if setup {
        crate::i18n::gettext(
            app.locale,
            "Choose a local code to open the Locked tab. It is separate from your phone's code. This hides chats in ZapFast, it does not add another encryption layer.",
        )
    } else {
        crate::i18n::gettext(
            app.locale,
            "Enter your local ZapFast code. Leaving the Locked tab or closing the window locks it again.",
        )
    };
    theme::paragraph(ui, desc_text, theme::regular(13.0), palette.secondary);
    let entry_id = ui.make_persistent_id("locked-code");
    let confirm_id = ui.make_persistent_id("locked-code-confirm");
    // TextEdit consumes Enter while surrendering focus. Capture and consume
    // it before drawing either editor instead of looking for it afterwards.
    let mut submit = ui
        .memory(|memory| memory.has_focus(entry_id) || (setup && memory.has_focus(confirm_id)))
        && ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
    let local_code_hint = crate::i18n::gettext(app.locale, "Local code");
    let entry = ui.add(
        egui::TextEdit::singleline(&mut app.chat_lock_entry)
            .id(entry_id)
            .password(true)
            .hint_text(local_code_hint)
            .desired_width(f32::INFINITY),
    );
    if ui.memory(|memory| memory.focused().is_none()) {
        entry.request_focus();
    }
    if setup {
        let confirm_code_hint = crate::i18n::gettext(app.locale, "Confirm code");
        ui.add(
            egui::TextEdit::singleline(&mut app.chat_lock_confirm)
                .id(confirm_id)
                .password(true)
                .hint_text(confirm_code_hint)
                .desired_width(f32::INFINITY),
        );
    }
    if app.chat_lock_error {
        theme::paragraph(
            ui,
            crate::i18n::gettext(app.locale, "That code did not match. Try again."),
            theme::regular(13.0),
            palette.danger,
        );
    }
    let ready = !app.chat_lock_entry.trim().is_empty()
        && (!setup || app.chat_lock_entry.trim() == app.chat_lock_confirm.trim());
    ui.horizontal(|ui| {
        let button_label = if setup {
            crate::i18n::gettext(app.locale, "Set code and open")
        } else {
            crate::i18n::gettext(app.locale, "Open locked chats")
        };
        submit |= ui
            .add_enabled_ui(ready, |ui| {
                theme::pill_button(ui, &palette, &button_label, true)
            })
            .inner
            .clicked();
        if theme::pill_button(
            ui,
            &palette,
            &crate::i18n::gettext(app.locale, "Cancel"),
            false,
        )
        .clicked()
        {
            app.actions.push(Action::CloseDialog);
        }
    });
    if submit && ready {
        let code = std::mem::take(&mut app.chat_lock_entry);
        app.actions.push(if setup {
            Action::CreateChatLockCode(code)
        } else {
            Action::UnlockLockedFolder(code)
        });
    }
}

fn confirm_lock_chat(app: &mut App, ui: &mut egui::Ui, id: &str) {
    let palette = app.palette;
    title(
        ui,
        app,
        &crate::i18n::gettext(app.locale, "Lock this chat?"),
    );
    theme::paragraph(
        ui,
        crate::i18n::gettext(
            app.locale,
            "This moves the chat to the Locked tab, hiding it from the normal chat list, search, unread badges, and notifications. The lock status also syncs to your phone. Open Locked with a separate local ZapFast code to read it.",
        ),
        theme::regular(13.0),
        palette.secondary,
    );
    ui.horizontal(|ui| {
        if theme::pill_button(
            ui,
            &palette,
            &crate::i18n::gettext(app.locale, "Lock chat"),
            true,
        )
        .clicked()
        {
            app.actions.push(Action::SetLocked(id.to_owned(), true));
            app.actions.push(Action::CloseDialog);
            if app.settings.chat_lock_code_hash.is_none() {
                app.actions.push(Action::OpenLockedFolder);
            }
        }
        if theme::pill_button(
            ui,
            &palette,
            &crate::i18n::gettext(app.locale, "Cancel"),
            false,
        )
        .clicked()
        {
            app.actions.push(Action::CloseDialog);
        }
    });
}

fn new_chat(app: &mut App, ui: &mut egui::Ui) {
    // Making a group turns the same dialog into its steps.
    if app.new_group.is_some() {
        super::new_group::show(app, ui);
        return;
    }
    let palette = app.palette;
    title(ui, app, &crate::i18n::gettext(app.locale, "New chat"));
    let width = ui.available_width();
    let search_hint = crate::i18n::gettext(app.locale, "Search contacts");
    let search = super::widgets::search_field(
        ui,
        &palette,
        egui::Id::new("new-chat-search"),
        &mut app.new_chat_search,
        &search_hint,
        width,
    );
    if ui.memory(|memory| memory.focused().is_none()) {
        search.request_focus();
    }
    let needle = crate::util::search_key(app.new_chat_search.trim());
    let contacts = contact_choices(app, &needle);
    egui::ScrollArea::vertical()
        .id_salt("new-chat-contacts")
        .max_height((ui.ctx().content_rect().height() - 280.0).clamp(100.0, 420.0))
        .auto_shrink([false, true])
        .show(ui, |ui| {
            // New group and Add contact head the list, then "Message
            // yourself", as on the phone.
            let new_group = crate::i18n::gettext(app.locale, "New group");
            if super::chats::action_row(app, ui, Icon::Users, &new_group).clicked() {
                app.actions.push(Action::StartNewGroup);
            }
            let add_contact = crate::i18n::gettext(app.locale, "Add contact");
            if super::chats::action_row(app, ui, Icon::Plus, &add_contact).clicked() {
                app.actions.push(Action::ShowDialog(Dialog::NewContact));
            }
            let offer_self = app.offers_self(&needle);
            if offer_self {
                super::chats::self_row(app, ui);
            }
            if contacts.is_empty() && !offer_self {
                theme::text(
                    ui,
                    crate::i18n::gettext(app.locale, "No matching contacts"),
                    theme::regular(13.0),
                    palette.secondary,
                );
            }
            for contact in contacts {
                ui.push_id(&contact.id, |ui| {
                    super::chats::contact_row(app, ui, &contact)
                });
            }
        });
}

/// The people New chat and New call offer, by name: the contacts and the
/// one-to-one chats, without ourselves or a locked chat, matching `needle`
/// (a `search_key`) by name or number.
pub(super) fn contact_choices(app: &App, needle: &str) -> Vec<crate::model::Contact> {
    let mut contacts = app.contacts.clone();
    for chat in &app.chats {
        if chat.kind == crate::model::ChatKind::Direct {
            contacts
                .entry(chat.id.clone())
                .or_insert_with(|| crate::model::Contact {
                    id: chat.id.clone(),
                    full_name: Some(app.chat_title(chat)),
                    ..Default::default()
                });
        }
    }
    let mut contacts: Vec<_> = contacts
        .into_values()
        .filter(|contact| {
            crate::model::phone_of(&contact.id).is_some()
                && app.me.as_deref() != Some(contact.id.as_str())
                && !app.chat(&contact.id).is_some_and(|chat| chat.locked)
                && (needle.is_empty()
                    || crate::util::search_matches(
                        &crate::util::search_key(&app.display_name(&contact.id)),
                        needle,
                    )
                    || crate::model::phone_of(&contact.id)
                        .is_some_and(|phone| phone.contains(needle)))
        })
        .collect();
    contacts.sort_by_key(|contact| crate::util::search_key(&app.display_name(&contact.id)));
    contacts
}

/// Draws recipient selection or, for two or more recipients, their
/// confirmation. A single recipient is forwarded to without one.
fn forward(app: &mut App, ui: &mut egui::Ui, messages: &[String]) {
    let palette = app.palette;
    ui.spacing_mut().item_spacing.y = 4.0;
    let height = ui.ctx().content_rect().height();
    ui.set_height(
        (height * 0.8)
            .clamp(300.0, 800.0)
            .min((height - 48.0).max(1.0)),
    );
    let heading = if messages.len() == 1 {
        crate::i18n::gettext(app.locale, "Forward message").into_owned()
    } else {
        crate::i18n::ngettext(
            app.locale,
            "Forward {count} message",
            "Forward {count} messages",
            messages.len() as u32,
        )
        .replace("{count}", &messages.len().to_string())
    };
    title(ui, app, &heading);
    // Lay out the real footer first, upwards from the bottom. Wrapped notes
    // and translated buttons then leave exactly their measured room for the
    // list, rather than relying on a guessed header/footer height.
    let mut footer = ui.new_child(
        egui::UiBuilder::new()
            .id_salt("forward-footer")
            .max_rect(ui.available_rect_before_wrap())
            .layout(Layout::bottom_up(Align::Min)),
    );
    forward_footer(app, &mut footer);
    let footer_rect = footer.min_rect();
    #[cfg(any(test, feature = "demo"))]
    ui.ctx().data_mut(|data| {
        data.insert_temp(egui::Id::new(("forward-layout", "footer")), footer_rect)
    });
    if app.forward_reviewing {
        theme::text(
            ui,
            crate::i18n::gettext(app.locale, "Forward to these chats?"),
            theme::medium(14.5),
            palette.text,
        );
        ui.add_space(8.0);
        let list = egui::ScrollArea::vertical()
            .id_salt("forward-review")
            .max_height((footer_rect.top() - ui.cursor().top() - 8.0).max(1.0))
            .min_scrolled_height(0.0)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for id in &app.forward_recipients {
                    if let Some(chat) = app.chat(id) {
                        theme::text(ui, app.chat_title(chat), theme::regular(14.0), palette.text);
                    }
                }
            });
        #[cfg(any(test, feature = "demo"))]
        ui.ctx().data_mut(|data| {
            data.insert_temp(egui::Id::new(("forward-layout", "list")), list.inner_rect)
        });
        #[cfg(not(any(test, feature = "demo")))]
        let _ = list;
        return;
    }
    let width = ui.available_width();
    let search = super::widgets::search_field(
        ui,
        &palette,
        egui::Id::new("forward-search"),
        &mut app.forward_search,
        "Search chats",
        width,
    );
    if ui.memory(|memory| memory.focused().is_none()) {
        search.request_focus();
    }
    ui.add_space(4.0);

    let needle = app.forward_search.trim().to_lowercase();
    let mut chats: Vec<_> = app
        .chats
        .iter()
        .filter(|chat| forwardable(chat))
        .filter(|chat| {
            needle.is_empty()
                || app.chat_title(chat).to_lowercase().contains(&needle)
                || chat.phone().is_some_and(|phone| phone.contains(&needle))
        })
        .cloned()
        .collect();
    chats.sort_by_key(|chat| std::cmp::Reverse(chat.last_activity));

    let row_height = 52.0;
    let max_height = (footer_rect.top() - ui.cursor().top() - 8.0).max(1.0);
    // With room for one recipient a click swaps it; otherwise a full list
    // takes no more.
    let full = app.forward_limit > 1 && app.forward_recipients.len() >= app.forward_limit;

    let list = egui::ScrollArea::vertical()
        .id_salt("forward-chats")
        .max_height(max_height)
        .min_scrolled_height(0.0)
        .auto_shrink([false, false])
        .show_rows(ui, row_height, chats.len(), |ui, range| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for chat in &chats[range] {
                let title = app.chat_title(chat);
                let (rect, response) =
                    ui.allocate_exact_size(vec2(ui.available_width(), row_height), Sense::click());
                theme::reveal_focus(&response);
                response.widget_info(|| {
                    egui::WidgetInfo::selected(
                        egui::WidgetType::SelectableLabel,
                        ui.is_enabled(),
                        app.forward_recipients.contains(&chat.id),
                        &title,
                    )
                });
                #[cfg(any(test, feature = "demo"))]
                ui.ctx().data_mut(|data| {
                    data.insert_temp(egui::Id::new(("forward-recipient", &chat.id)), rect)
                });
                let selected = app.forward_recipients.contains(&chat.id);
                let blocked = full && !selected;
                if ui.is_rect_visible(rect) {
                    if (response.hovered() && !blocked) || selected {
                        super::widgets::dialog_row_highlight(ui, rect, palette.surface_hover);
                    }
                    let check = egui::Rect::from_center_size(
                        pos2(rect.right() - 18.0, rect.center().y),
                        egui::Vec2::splat(20.0),
                    );
                    if super::widgets::round_check(
                        ui, &palette, check, &chat.id, selected, !blocked, &title,
                    ) {
                        app.actions
                            .push(Action::ToggleForwardRecipient(chat.id.clone()));
                    }
                    let avatar = egui::Rect::from_center_size(
                        pos2(rect.left() + 23.0, rect.center().y),
                        egui::Vec2::splat(38.0),
                    );
                    let picture = app.avatar(&chat.id);
                    super::widgets::paint_avatar(
                        ui,
                        &palette,
                        avatar,
                        &title,
                        &chat.id,
                        picture.as_deref(),
                    );
                    let line = super::widgets::line(
                        ui,
                        &title,
                        theme::medium(14.5),
                        palette.text,
                        rect.width() - 90.0,
                        1,
                    );
                    let colour = if blocked {
                        palette.secondary
                    } else {
                        palette.text
                    };
                    line.paint(
                        ui,
                        pos2(rect.left() + 50.0, rect.center().y - line.size().y / 2.0),
                        colour,
                    );
                }
                let response = if blocked {
                    response
                } else {
                    response.on_hover_cursor(egui::CursorIcon::PointingHand)
                };
                if response.clicked() && !blocked {
                    app.actions
                        .push(Action::ToggleForwardRecipient(chat.id.clone()));
                }
            }
            if chats.is_empty() {
                theme::text(
                    ui,
                    "No writable chats found",
                    theme::regular(13.5),
                    palette.secondary,
                );
            }
        });
    #[cfg(any(test, feature = "demo"))]
    ui.ctx().data_mut(|data| {
        data.insert_temp(egui::Id::new(("forward-layout", "list")), list.inner_rect)
    });
    #[cfg(not(any(test, feature = "demo")))]
    let _ = list;
}

fn forward_footer(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    if app.forward_reviewing {
        ui.horizontal(|ui| {
            if theme::pill_button(
                ui,
                &palette,
                &crate::i18n::gettext(app.locale, "Back"),
                false,
            )
            .clicked()
            {
                app.actions.push(Action::BackToForwardSelection);
            }
            if theme::pill_button(
                ui,
                &palette,
                &crate::i18n::gettext(app.locale, "Forward"),
                true,
            )
            .clicked()
            {
                app.actions.push(Action::ConfirmForward);
            }
        });
        return;
    }
    let count = app.forward_recipients.len();
    // Keep the action visible even when the translated count needs truncation.
    let (label, action) = if count > 1 {
        (
            crate::i18n::gettext(app.locale, "Review recipients"),
            Action::ReviewForward,
        )
    } else {
        (
            crate::i18n::gettext(app.locale, "Forward"),
            Action::ConfirmForward,
        )
    };
    ui.horizontal(|ui| {
        let button_width = ui
            .painter()
            .layout_no_wrap(label.to_string(), theme::semibold(13.0), palette.on_accent)
            .size()
            .x
            + 36.0;
        let count_width =
            (ui.available_width() - button_width - ui.spacing().item_spacing.x).max(1.0);
        ui.allocate_ui(vec2(count_width, 20.0), |ui| {
            theme::text(
                ui,
                crate::i18n::ngettext(
                    app.locale,
                    "{count} recipient selected",
                    "{count} recipients selected",
                    count as u32,
                )
                .replace("{count}", &count.to_string()),
                theme::regular(13.5),
                palette.secondary,
            );
        });
        let button = ui
            .add_enabled_ui(count > 0, |ui| {
                theme::pill_button(ui, &palette, &label, true)
            })
            .inner;
        if button.clicked() {
            app.actions.push(action);
        }
    });
    // This UI lays out bottom-up: the note sits above the action row.
    let full = app.forward_limit > 1 && app.forward_recipients.len() >= app.forward_limit;
    let limit = if app.forward_limit == 1 {
        Some(
            crate::i18n::gettext(
                app.locale,
                "Messages forwarded many times can be sent to one chat at a time",
            )
            .into_owned(),
        )
    } else if full {
        Some(
            crate::i18n::ngettext(
                app.locale,
                "You can forward to up to {count} chat",
                "You can forward to up to {count} chats",
                app.forward_limit as u32,
            )
            .replace("{count}", &app.forward_limit.to_string()),
        )
    } else {
        None
    };
    if let Some(limit) = limit {
        ui.add(
            egui::Label::new(
                egui::RichText::new(limit)
                    .font(theme::regular(13.0))
                    .color(palette.secondary),
            )
            .wrap(),
        );
    }
}

/// Uses the model's send permissions to reject unavailable forwarding
/// destinations. Locked chats stay out of the picker, which would otherwise
/// list them outside the locked folder.
pub(crate) fn forwardable(chat: &crate::model::Chat) -> bool {
    chat.can_send() && !chat.locked
}

fn title(ui: &mut egui::Ui, app: &mut App, label: &str) {
    let palette = app.palette;
    ui.horizontal(|ui| {
        theme::text(ui, label, theme::bold(18.0), palette.text);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let close = theme::icon_button(
                ui,
                Icon::X,
                16.0,
                palette.secondary,
                palette.text,
                &crate::i18n::gettext(app.locale, "Close"),
            );
            #[cfg(any(test, feature = "demo"))]
            if matches!(app.dialog, Some(Dialog::Forward { .. })) {
                ui.ctx().data_mut(|data| {
                    data.insert_temp(egui::Id::new(("forward-layout", "close")), close.rect)
                });
            }
            if close.clicked() {
                app.actions.push(Action::CloseDialog);
            }
        });
    });
    ui.add_space(4.0);
}

/// The shortcuts dialog's width with its two columns, and with one.
const SHORTCUTS_WIDE: f32 = 920.0;
const SHORTCUTS_NARROW: f32 = 540.0;
/// The space between the two columns.
const SHORTCUTS_GUTTER: f32 = 32.0;

/// How wide the shortcuts dialog is in a window `window` wide: two columns
/// where they fit, one otherwise, and never wider than the window.
fn shortcuts_width(window: f32) -> f32 {
    let room = window - 64.0;
    if room >= SHORTCUTS_WIDE {
        SHORTCUTS_WIDE
    } else {
        SHORTCUTS_NARROW.min(room).max(180.0)
    }
}

/// One shortcut: its keys in a column `keys_width` wide, then what it does,
/// wrapped in the room that is left.
fn shortcut_row(
    ui: &mut egui::Ui,
    palette: &theme::Palette,
    keys_width: f32,
    keys: &str,
    what: &str,
) {
    ui.horizontal_top(|ui| {
        let rest = (ui.available_width() - keys_width - ui.spacing().item_spacing.x).max(60.0);
        ui.allocate_ui_with_layout(vec2(keys_width, 0.0), Layout::top_down(Align::Min), |ui| {
            ui.set_width(keys_width);
            theme::text(
                ui,
                super::keys::label(keys),
                theme::semibold(13.0),
                palette.text,
            );
        });
        ui.allocate_ui_with_layout(vec2(rest, 0.0), Layout::top_down(Align::Min), |ui| {
            ui.set_width(rest);
            theme::paragraph(ui, what, theme::regular(13.0), palette.secondary);
        });
    });
}

fn shortcuts(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    title(
        ui,
        app,
        &crate::i18n::gettext(app.locale, "Keyboard shortcuts"),
    );
    // Reserve enough width for the longest shortcut before laying out the rows.
    let keys_width = super::keys::SHORTCUTS
        .iter()
        .map(|(keys, _)| {
            ui.painter()
                .layout_no_wrap(
                    super::keys::label(keys),
                    theme::semibold(13.0),
                    egui::Color32::WHITE,
                )
                .size()
                .x
        })
        .fold(0.0, f32::max)
        .ceil();
    // Two columns in a window wide enough for them; whatever the window
    // still cannot show scrolls, so the title and the close button stay.
    let width = ui.available_width();
    let columns = if width >= SHORTCUTS_WIDE { 2 } else { 1 };
    let column = (width - SHORTCUTS_GUTTER * (columns - 1) as f32) / columns as f32;
    let per_column = super::keys::SHORTCUTS.len().div_ceil(columns);
    let height = (ui.ctx().content_rect().height() - 190.0).max(120.0);
    egui::ScrollArea::vertical()
        .id_salt("shortcuts")
        .max_height(height)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = SHORTCUTS_GUTTER;
                for rows in super::keys::SHORTCUTS.chunks(per_column) {
                    ui.allocate_ui_with_layout(
                        vec2(column, 0.0),
                        Layout::top_down(Align::Min),
                        |ui| {
                            ui.set_width(column);
                            ui.spacing_mut().item_spacing = vec2(18.0, 8.0);
                            for (keys, what) in rows {
                                shortcut_row(ui, &palette, keys_width, keys, what);
                            }
                        },
                    );
                }
            });
        });
    ui.add_space(12.0);
    // The hint bar's × hides it; this brings it back.
    let mut hints = app.settings.show_shortcut_hints;
    if ui
        .checkbox(
            &mut hints,
            crate::i18n::gettext(app.locale, "Show shortcut hints under the message box"),
        )
        .changed()
    {
        app.actions.push(Action::SetShortcutHints(hints));
    }
}

fn about(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    title(ui, app, &crate::i18n::gettext(app.locale, "About"));
    ui.horizontal(|ui| {
        let (logo, _) = ui.allocate_exact_size(egui::Vec2::splat(44.0), egui::Sense::hover());
        theme::mark(ui, logo.center(), 44.0);
        ui.vertical(|ui| {
            theme::text(ui, "ZapFast", theme::bold(17.0), palette.text);
            let version_label = crate::i18n::gettext(app.locale, "Version {version}")
                .replace("{version}", env!("CARGO_PKG_VERSION"));
            theme::text(ui, version_label, theme::regular(13.0), palette.secondary);
        });
    });
    ui.add_space(6.0);
    theme::paragraph(
        ui,
        crate::i18n::gettext(
            app.locale,
            "A native WhatsApp client written in Rust with egui. It connects through whatsapp-rust. Messages are end-to-end encrypted on this device.",
        ),
        theme::regular(13.0),
        palette.text,
    );
    theme::paragraph(
        ui,
        crate::i18n::gettext(
            app.locale,
            "This is an unofficial client. Using it may be against WhatsApp's terms of service and could get an account suspended.",
        ),
        theme::regular(12.5),
        palette.secondary,
    );
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        if theme::link(
            ui,
            crate::i18n::gettext(app.locale, "Source code"),
            theme::medium(13.0),
            palette.link,
        )
        .clicked()
        {
            app.actions
                .push(Action::OpenUrl(env!("CARGO_PKG_REPOSITORY").to_owned()));
        }
        theme::text(ui, "·", theme::regular(13.0), palette.dim);
        if theme::link(ui, "whatsapp-rust", theme::medium(13.0), palette.link).clicked() {
            app.actions.push(Action::OpenUrl(
                "https://github.com/oxidezap/whatsapp-rust".to_owned(),
            ));
        }
    });
    ui.add_space(4.0);
    if super::widgets::credit(ui, &palette, app.locale) {
        app.actions
            .push(Action::OpenUrl(super::widgets::AUTHOR_URL.to_owned()));
    }
}

/// Formats the localized singular or plural selected-message count.
fn selected_message_count(locale: crate::i18n::Locale, count: usize) -> String {
    crate::i18n::ngettext(
        locale,
        "{count} selected message",
        "{count} selected messages",
        count as u32,
    )
    .replace("{count}", &count.to_string())
}

fn confirm_delete_chat(app: &mut App, ui: &mut egui::Ui, id: &str) {
    let locale = app.locale;
    let name = app.chat(id).map_or_else(
        || crate::i18n::gettext(locale, "this chat").into_owned(),
        |chat| chat.name.clone(),
    );
    let body = crate::i18n::gettext(
        locale,
        "This deletes your chat with {name}, including all messages, on this computer and on your phone. It cannot be undone.",
    )
    .replace("{name}", &name);
    confirm_destructive(
        app,
        ui,
        &crate::i18n::gettext(locale, "Delete chat?"),
        &[&body],
        &crate::i18n::gettext(locale, "Delete"),
        [Action::DeleteChat(id.to_owned())],
    );
}

fn confirm_clear_chat(app: &mut App, ui: &mut egui::Ui, id: &str) {
    let locale = app.locale;
    let name = app.chat(id).map_or_else(
        || crate::i18n::gettext(locale, "this chat").into_owned(),
        |chat| chat.name.clone(),
    );
    let body = crate::i18n::gettext(
        locale,
        "This clears every message in your chat with {name}, including downloaded media, on this computer and on your phone. The chat itself stays. It cannot be undone.",
    )
    .replace("{name}", &name);
    confirm_destructive(
        app,
        ui,
        &crate::i18n::gettext(locale, "Clear chat?"),
        &[&body],
        &crate::i18n::gettext(locale, "Clear chat"),
        [Action::ClearChat(id.to_owned())],
    );
}

fn join_group(app: &mut App, ui: &mut egui::Ui) {
    use crate::model::InviteState;
    let palette = app.palette;
    let Some(invite) = app.invite.clone() else {
        app.actions.push(Action::CloseDialog);
        return;
    };
    let (info, joining) = match &invite.state {
        InviteState::Loading => {
            title(ui, app, &crate::i18n::gettext(app.locale, "Group invite"));
            ui.horizontal(|ui| {
                crate::motion::egui_spinner(ui);
                ui.label(
                    egui::RichText::new(crate::i18n::gettext(app.locale, "Looking up the group…"))
                        .color(palette.secondary),
                );
            });
            cancel_row(app, ui);
            return;
        }
        InviteState::Failed(error) => {
            title(ui, app, &crate::i18n::gettext(app.locale, "Group invite"));
            theme::paragraph(ui, error, theme::regular(13.5), palette.text);
            cancel_row(app, ui);
            return;
        }
        InviteState::Ready(info) => (info, false),
        InviteState::Joining(info) => (info, true),
    };
    super::widgets::rich_text(ui, &info.subject, theme::semibold(17.0), palette.text);
    let members = crate::i18n::ngettext(app.locale, "{} member", "{} members", info.members as u32)
        .replace("{}", &info.members.to_string());
    ui.label(egui::RichText::new(members).color(palette.secondary));
    if let Some(description) = &info.description {
        ui.add_space(4.0);
        egui::ScrollArea::vertical()
            .max_height(140.0)
            .show(ui, |ui| {
                super::widgets::rich_text(ui, description, theme::regular(13.5), palette.text);
            });
    }
    let member = app.chats.iter().any(|chat| chat.id == info.id);
    if info.approval && !member {
        ui.add_space(4.0);
        theme::paragraph(
            ui,
            crate::i18n::gettext(
                app.locale,
                "An admin must approve your request before you join.",
            ),
            theme::regular(13.0),
            palette.secondary,
        );
    }
    ui.add_space(10.0);
    ui.horizontal(|ui| {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let label = match (member, info.approval) {
                (true, _) => crate::i18n::gettext(app.locale, "Open chat"),
                (false, true) => crate::i18n::gettext(app.locale, "Request to join"),
                (false, false) => crate::i18n::gettext(app.locale, "Join group"),
            };
            let button_text = if joining {
                crate::i18n::gettext(app.locale, "Joining…")
            } else {
                label
            };
            let join = ui.add_enabled_ui(!joining, |ui| {
                theme::pill_button(ui, &palette, &button_text, true)
            });
            if join.inner.clicked() {
                app.actions.push(Action::JoinGroup);
            }
            if theme::pill_button(
                ui,
                &palette,
                &crate::i18n::gettext(app.locale, "Cancel"),
                false,
            )
            .clicked()
            {
                app.actions.push(Action::CloseDialog);
            }
        });
    });
}

fn cancel_row(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    ui.add_space(10.0);
    ui.horizontal(|ui| {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if theme::pill_button(
                ui,
                &palette,
                &crate::i18n::gettext(app.locale, "Close"),
                false,
            )
            .clicked()
            {
                app.actions.push(Action::CloseDialog);
            }
        });
    });
}

/// A sticker pack shared in a chat: its stickers, and a button to add it.
fn sticker_pack(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    let locale = app.locale;
    let Some((pack, publisher)) = app.sticker_preview.clone() else {
        title(ui, app, &crate::i18n::gettext(locale, "Sticker pack"));
        ui.horizontal(|ui| {
            theme::spinner(ui, 16.0, palette.accent);
            theme::text(
                ui,
                crate::i18n::gettext(locale, "Downloading the pack…"),
                theme::regular(13.0),
                palette.secondary,
            );
        });
        cancel_row(app, ui);
        return;
    };
    title(ui, app, &pack.name);
    if !publisher.trim().is_empty() {
        theme::text(ui, &publisher, theme::regular(13.0), palette.secondary);
    }
    egui::ScrollArea::vertical()
        .max_height(320.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            super::picker::sticker_preview_grid(ui, &palette, &pack.stickers, app.window_focused);
        });
    ui.add_space(10.0);
    ui.horizontal(|ui| {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if theme::pill_button(
                ui,
                &palette,
                &crate::i18n::gettext(locale, "Add to my stickers"),
                true,
            )
            .clicked()
            {
                app.actions.push(Action::AddStickerPack);
            }
            if theme::pill_button(ui, &palette, &crate::i18n::gettext(locale, "Close"), false)
                .clicked()
            {
                app.actions.push(Action::CloseDialog);
            }
        });
    });
}

/// Height of the sticker maker's picture area.
const MAKER_VIEW: f32 = 300.0;

/// Crops a chosen picture into a sticker: drag the square to move it, use the
/// slider to size it, and tag it with emojis before adding or sending it.
fn sticker_maker(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    let locale = app.locale;
    title(ui, app, &crate::i18n::gettext(locale, "Make a sticker"));
    let Some(mut draft) = app.sticker_draft.clone() else {
        cancel_row(app, ui);
        return;
    };
    let (area, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), MAKER_VIEW), Sense::drag());
    let scale = (area.width() / draft.width as f32).min(area.height() / draft.height as f32);
    let picture = egui::Rect::from_center_size(
        area.center(),
        vec2(draft.width as f32 * scale, draft.height as f32 * scale),
    );
    if response.dragged() && scale > 0.0 {
        let delta = response.drag_delta() / scale;
        draft.crop = draft.crop.moved(
            delta.x.round() as i64,
            delta.y.round() as i64,
            draft.width,
            draft.height,
        );
    }
    if ui.is_rect_visible(area) {
        ui.painter()
            .rect_filled(area, CornerRadius::same(theme::RADIUS), palette.surface);
        super::widgets::file_image(ui, &draft.source)
            .fit_to_exact_size(picture.size())
            .paint_at(ui, picture);
        let crop = egui::Rect::from_min_size(
            picture.min + vec2(draft.crop.x as f32, draft.crop.y as f32) * scale,
            egui::Vec2::splat(draft.crop.side as f32 * scale),
        );
        // Dim what the sticker leaves out.
        let shade = palette.shadow.gamma_multiply(1.6);
        for outside in [
            egui::Rect::from_min_max(picture.min, pos2(picture.max.x, crop.min.y)),
            egui::Rect::from_min_max(pos2(picture.min.x, crop.max.y), picture.max),
            egui::Rect::from_min_max(
                pos2(picture.min.x, crop.min.y),
                pos2(crop.min.x, crop.max.y),
            ),
            egui::Rect::from_min_max(
                pos2(crop.max.x, crop.min.y),
                pos2(picture.max.x, crop.max.y),
            ),
        ] {
            if outside.is_positive() {
                ui.painter().rect_filled(outside, 0.0, shade);
            }
        }
        ui.painter().rect_stroke(
            crop,
            0.0,
            Stroke::new(2.0, egui::Color32::WHITE),
            egui::StrokeKind::Inside,
        );
    }
    let response = response.on_hover_cursor(egui::CursorIcon::Grab);
    let _ = response;
    let largest = draft.width.min(draft.height).max(1);
    let smallest = (largest / 8).max(1);
    let mut side = draft.crop.side;
    ui.horizontal(|ui| {
        theme::text(
            ui,
            crate::i18n::gettext(locale, "Size"),
            theme::regular(13.0),
            palette.secondary,
        );
        ui.spacing_mut().slider_width = ui.available_width() - 8.0;
        ui.add(egui::Slider::new(&mut side, smallest..=largest).show_value(false));
    });
    if side != draft.crop.side {
        draft.crop = draft.crop.resized(side, draft.width, draft.height);
    }
    if draft.transparent {
        ui.horizontal(|ui| {
            super::widgets::switch(ui, &palette, &mut draft.keep_transparent);
            theme::text(
                ui,
                crate::i18n::gettext(locale, "Keep the transparent background"),
                theme::regular(13.0),
                palette.text,
            );
        });
    }
    Frame::new()
        .fill(palette.surface)
        .corner_radius(CornerRadius::same(theme::RADIUS))
        .inner_margin(Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.add(
                egui::TextEdit::singleline(&mut draft.emojis)
                    .id(egui::Id::new("sticker-maker-emojis"))
                    .hint_text(
                        egui::RichText::new(crate::i18n::gettext(
                            locale,
                            "Emojis that describe it, like 😂 or ❤️",
                        ))
                        .color(palette.dim)
                        .font(theme::regular(13.0)),
                    )
                    .font(theme::regular(13.0))
                    .text_color(palette.text)
                    .frame(Frame::NONE)
                    .desired_width(f32::INFINITY),
            );
        });
    app.sticker_draft = Some(draft);
    ui.add_space(10.0);
    let can_send = app.open_chat.is_some();
    ui.horizontal(|ui| {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if can_send
                && theme::pill_button(ui, &palette, &crate::i18n::gettext(locale, "Send"), true)
                    .clicked()
            {
                app.actions.push(Action::MakeSticker { send: true });
            }
            if theme::pill_button(
                ui,
                &palette,
                &crate::i18n::gettext(locale, "Add to favorites"),
                !can_send,
            )
            .clicked()
            {
                app.actions.push(Action::MakeSticker { send: false });
            }
            if theme::pill_button(ui, &palette, &crate::i18n::gettext(locale, "Cancel"), false)
                .clicked()
            {
                app.actions.push(Action::CloseDialog);
            }
        });
    });
}

fn confirm_start_over(app: &mut App, ui: &mut egui::Ui) {
    let locale = app.locale;
    confirm_destructive(
        app,
        ui,
        &crate::i18n::gettext(locale, "Start over?"),
        &[
            &crate::i18n::gettext(
                locale,
                "ZapFast keeps your unreadable archive as a separate file, creates a new one, and asks you to link again. Linking again brings back recent history from your phone. Afterwards, remove the old ZapFast entry under Linked devices on your phone.",
            ),
            &crate::i18n::gettext(
                locale,
                "If you can restore the original keyring instead, choose Cancel and Try again: nothing is lost that way.",
            ),
        ],
        &crate::i18n::gettext(locale, "Start over"),
        [Action::StartOverArchive],
    );
}

/// Formats localized deletion guidance for direct or selected messages and the chosen scope.
fn deletion_body(locale: crate::i18n::Locale, for_everyone: bool, count: Option<usize>) -> String {
    match (for_everyone, count) {
        (false, None) => crate::i18n::gettext(locale, "This removes the message from your phone and linked devices. Other people keep their copy. Connect to WhatsApp to delete it. This cannot be undone.").into_owned(),
        (true, None) => crate::i18n::gettext(locale, "Everyone in this chat will see \"This message was deleted\" instead. It cannot be undone.").into_owned(),
        (false, Some(_)) => crate::i18n::gettext(locale, "This removes these messages from your phone and linked devices. Other people keep their copy. Connect to WhatsApp to delete them. This cannot be undone.").into_owned(),
        (true, Some(count)) => crate::i18n::ngettext(locale, "Everyone in this chat will see \"This message was deleted\" in place of the selected message. This cannot be undone.", "Everyone in this chat will see \"This message was deleted\" in place of the selected messages. This cannot be undone.", count as u32).into_owned(),
    }
}

/// Confirms deleting one message. Enter is deliberately not bound here: a
/// stray keypress must not destroy a message.
fn confirm_delete_message(
    app: &mut App,
    ui: &mut egui::Ui,
    chat: &str,
    id: &str,
    for_everyone: bool,
) {
    confirm_delete_messages(app, ui, chat, &[id.to_owned()], for_everyone, false);
}

/// Confirms deleting messages, one or the whole selection, and queues one
/// deletion per message in the chosen scope. A confirmed selection ends.
fn confirm_delete_messages(
    app: &mut App,
    ui: &mut egui::Ui,
    chat: &str,
    ids: &[String],
    for_everyone: bool,
    selection: bool,
) {
    let locale = app.locale;
    let heading = if for_everyone {
        crate::i18n::gettext(locale, "Delete for everyone?")
    } else {
        crate::i18n::gettext(locale, "Delete for me?")
    };
    let body = deletion_body(locale, for_everyone, selection.then_some(ids.len()));
    let count = selected_message_count(locale, ids.len());
    let paragraphs: Vec<&str> = if selection && !ids.is_empty() {
        vec![&count, &body]
    } else {
        vec![&body]
    };
    let mut actions: Vec<Action> = ids
        .iter()
        .map(|id| {
            let (chat, id) = (chat.to_owned(), id.clone());
            if for_everyone {
                Action::DeleteForEveryone { chat, id }
            } else {
                Action::DeleteForMe { chat, id }
            }
        })
        .collect();
    if selection {
        actions.push(Action::CancelSelection);
    }
    confirm_destructive(
        app,
        ui,
        &heading,
        &paragraphs,
        &crate::i18n::gettext(locale, "Delete"),
        actions,
    );
}

fn confirm_remove_sticker_pack(
    app: &mut App,
    ui: &mut egui::Ui,
    name: &str,
    dir: std::path::PathBuf,
) {
    let locale = app.locale;
    let body = crate::i18n::gettext(
        locale,
        "This deletes \"{name}\" and its stickers from this computer. To use it again, import it again.",
    )
    .replace("{name}", name);
    confirm_destructive(
        app,
        ui,
        &crate::i18n::gettext(locale, "Remove sticker pack?"),
        &[&body],
        &crate::i18n::gettext(locale, "Remove"),
        [Action::DeleteStickerPack(dir)],
    );
}

fn confirm_unlink(app: &mut App, ui: &mut egui::Ui) {
    let locale = app.locale;
    confirm_destructive(
        app,
        ui,
        &crate::i18n::gettext(locale, "Unlink this computer?"),
        &[&crate::i18n::gettext(
            locale,
            "This removes the device from WhatsApp and deletes the chats stored here. You can link again with a new code.",
        )],
        &crate::i18n::gettext(locale, "Unlink"),
        [Action::Unlink],
    );
}

fn confirm_clear_media(app: &mut App, ui: &mut egui::Ui, id: crate::model::AccountId) {
    let locale = app.locale;
    let account = app
        .accounts
        .iter()
        .find(|account| account.id == id)
        .map(|account| account.display_label(locale))
        .unwrap_or_default();
    confirm_destructive(
        app,
        ui,
        &crate::i18n::gettext(locale, "Clear the media cache?"),
        &[
            &crate::i18n::gettext(
                locale,
                "This deletes the photos, videos, stickers and files {account} downloaded to this computer. Messages stay, and attachments download again when you open them, while WhatsApp still has them.",
            )
            .replace("{account}", &account),
            &crate::i18n::gettext(
                locale,
                "Files in a download folder you chose are kept.",
            ),
        ],
        &crate::i18n::gettext(locale, "Clear"),
        [Action::ClearMediaCache(id)],
    );
}

fn confirm_remove(app: &mut App, ui: &mut egui::Ui, id: crate::model::AccountId) {
    let locale = app.locale;
    confirm_destructive(
        app,
        ui,
        &crate::i18n::gettext(locale, "Remove this account?"),
        &[&crate::i18n::gettext(
            locale,
            "This unlinks the number on this computer and deletes its local chats. Other accounts stay.",
        )],
        &crate::i18n::gettext(locale, "Remove"),
        [Action::RemoveAccount(id)],
    );
}

/// A confirmation for something that cannot be undone: the question, what it
/// destroys (the first paragraph in the text colour, any after it as a smaller
/// note), then Cancel and a red button that queues `action` and closes the
/// dialog. Callers pass translated text. Enter is deliberately not bound: a
/// stray keypress must not destroy anything.
fn confirm_destructive(
    app: &mut App,
    ui: &mut egui::Ui,
    heading: &str,
    body: &[&str],
    button: &str,
    actions: impl IntoIterator<Item = Action>,
) {
    let palette = app.palette;
    title(ui, app, heading);
    for (index, paragraph) in body.iter().enumerate() {
        let (font, colour) = if index == 0 {
            (theme::regular(13.5), palette.text)
        } else {
            (theme::regular(13.0), palette.secondary)
        };
        theme::paragraph(ui, *paragraph, font, colour);
    }
    ui.add_space(10.0);
    let cancel = crate::i18n::gettext(app.locale, "Cancel");
    ui.horizontal(|ui| {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if danger_button(ui, app, button).clicked() {
                app.actions.extend(actions);
                app.actions.push(Action::CloseDialog);
            }
            if theme::pill_button(ui, &palette, &cancel, false).clicked() {
                app.actions.push(Action::CloseDialog);
            }
        });
    });
}

/// Asks before revoking a group's invite link: everyone holding the old
/// one loses the way in.
fn confirm_reset_invite_link(app: &mut App, ui: &mut egui::Ui, chat: &str) {
    let palette = app.palette;
    let locale = app.locale;
    title(
        ui,
        app,
        crate::i18n::gettext(locale, "Reset the invite link?").as_ref(),
    );
    theme::paragraph(
        ui,
        crate::i18n::gettext(
            locale,
            "The current link will stop working, and nobody can join with it any more. A new link replaces it.",
        ),
        theme::regular(13.5),
        palette.text,
    );
    ui.add_space(10.0);
    confirm_buttons(
        app,
        ui,
        crate::i18n::gettext(locale, "Reset link").as_ref(),
        Action::ResetInviteLink(chat.to_owned()),
    );
}

/// Asks before removing someone from a group.
fn confirm_remove_member(app: &mut App, ui: &mut egui::Ui, chat: &str, member: &str) {
    let palette = app.palette;
    let locale = app.locale;
    let name = app.display_name(member);
    let group = app
        .chat(chat)
        .map(|chat| app.chat_title(chat))
        .unwrap_or_default();
    title(
        ui,
        app,
        crate::i18n::gettext(locale, "Remove from the group?").as_ref(),
    );
    theme::paragraph(
        ui,
        crate::i18n::gettext(
            locale,
            "{name} will no longer receive messages from {group}. You can add them again later.",
        )
        .replace("{name}", &name)
        .replace("{group}", &group),
        theme::regular(13.5),
        palette.text,
    );
    ui.add_space(10.0);
    confirm_buttons(
        app,
        ui,
        crate::i18n::gettext(locale, "Remove").as_ref(),
        Action::ChangeGroupMember {
            chat: chat.to_owned(),
            member: member.to_owned(),
            change: crate::group_admin::MemberChange::Remove,
        },
    );
}

/// A destructive button and Cancel, at the right.
fn confirm_buttons(app: &mut App, ui: &mut egui::Ui, label: &str, action: Action) {
    let palette = app.palette;
    let cancel = crate::i18n::gettext(app.locale, "Cancel");
    ui.horizontal(|ui| {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if danger_button(ui, app, label).clicked() {
                app.actions.push(action);
            }
            if theme::pill_button(ui, &palette, cancel.as_ref(), false).clicked() {
                app.actions.push(Action::CloseDialog);
            }
        });
    });
}

fn confirm_leave_group(app: &mut App, ui: &mut egui::Ui, id: &str) {
    let palette = app.palette;
    let chat = app.chat(id);
    let archived = chat.is_some_and(|chat| chat.archived);
    let channel = chat.is_some_and(crate::model::Chat::is_channel);
    let locale = app.locale;
    let question = if channel {
        crate::i18n::gettext(locale, "Leave this channel?")
    } else {
        crate::i18n::gettext(locale, "Leave this group?")
    };
    title(ui, app, question.as_ref());
    theme::paragraph(
        ui,
        crate::i18n::gettext(
            locale,
            "You will not receive new messages. The local history stays on this computer.",
        ),
        theme::regular(13.5),
        palette.text,
    );
    ui.add_space(10.0);
    let leave_label = if channel {
        crate::i18n::gettext(locale, "Leave channel")
    } else {
        crate::i18n::gettext(locale, "Leave group")
    };
    if danger_button(ui, app, leave_label.as_ref()).clicked() {
        app.actions.push(Action::LeaveGroup {
            chat: id.to_owned(),
            archive: false,
        });
    }
    if !archived {
        ui.add_space(4.0);
        let archive_label = if channel {
            crate::i18n::gettext(locale, "Leave channel and archive")
        } else {
            crate::i18n::gettext(locale, "Leave group and archive")
        };
        if theme::pill_button(ui, &palette, archive_label.as_ref(), false).clicked() {
            app.actions.push(Action::LeaveGroup {
                chat: id.to_owned(),
                archive: true,
            });
        }
    }
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if theme::pill_button(
                ui,
                &palette,
                crate::i18n::gettext(locale, "Cancel").as_ref(),
                false,
            )
            .clicked()
            {
                app.actions.push(Action::CloseDialog);
            }
        });
    });
}

fn pair_with_phone(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    title(
        ui,
        app,
        &crate::i18n::gettext(app.locale, "Link with a phone number"),
    );
    theme::paragraph(
        ui,
        crate::i18n::gettext(
            app.locale,
            "Enter the WhatsApp phone number with its country code. Do not include a plus sign or leading zero. You will get a code to enter on the phone.",
        ),
        theme::regular(13.0),
        palette.secondary,
    );
    ui.add_space(4.0);
    let id = egui::Id::new("pair-phone");
    let submit = ui.memory(|memory| memory.has_focus(id))
        && ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
    let field = Frame::new()
        .fill(palette.surface)
        .corner_radius(CornerRadius::same(theme::RADIUS))
        .inner_margin(Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                // Set the row height from the field instead of the shorter label.
                ui.set_min_height(
                    ui.ctx()
                        .fonts_mut(|fonts| fonts.row_height(&theme::regular(16.0)))
                        + 4.0,
                );
                theme::text(ui, "+", theme::semibold(16.0), palette.secondary);
                let response = ui.add(
                    egui::TextEdit::singleline(&mut app.pair_phone)
                        .id(id)
                        .hint_text(
                            egui::RichText::new("15551234567")
                                .color(palette.dim)
                                .font(theme::regular(16.0)),
                        )
                        .font(theme::regular(16.0))
                        .text_color(palette.text)
                        .frame(egui::Frame::NONE)
                        .desired_width(f32::INFINITY),
                );
                if !response.has_focus() && app.pair_phone.is_empty() {
                    response.request_focus();
                }
            });
        });
    theme::focus_outline(ui, id, field.response.rect, f32::from(theme::RADIUS));
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let ready = app.pair_phone.chars().filter(char::is_ascii_digit).count() >= 7;
            // Stay the main action, but look and read as unavailable until
            // the number is long enough.
            let hint = crate::i18n::gettext(
                app.locale,
                "Enter the whole number, starting with the country code",
            );
            let get_code = ui
                .add_enabled_ui(ready, |ui| {
                    theme::pill_button(
                        ui,
                        &palette,
                        &crate::i18n::gettext(app.locale, "Get a code"),
                        true,
                    )
                    .on_disabled_hover_text(hint)
                })
                .inner;
            if (get_code.clicked() || submit) && ready {
                app.actions
                    .push(Action::PairWithPhone(app.pair_phone.clone()));
            }
            if theme::pill_button(
                ui,
                &palette,
                &crate::i18n::gettext(app.locale, "Cancel"),
                false,
            )
            .clicked()
            {
                app.actions.push(Action::CloseDialog);
            }
        });
    });
}

fn new_contact(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    title(ui, app, &crate::i18n::gettext(app.locale, "New contact"));
    theme::paragraph(
        ui,
        crate::i18n::gettext(
            app.locale,
            "Enter a phone number with its country code. Add a name to save the contact, or leave it blank to message the number without saving it. WhatsApp uses the first name as the display name.",
        ),
        theme::regular(13.0),
        palette.secondary,
    );
    ui.add_space(4.0);
    let boxed = |ui: &mut egui::Ui, inner: &mut dyn FnMut(&mut egui::Ui) -> egui::Response| {
        let field = Frame::new()
            .fill(palette.surface)
            .corner_radius(CornerRadius::same(theme::RADIUS))
            .inner_margin(Margin::symmetric(12, 8))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.set_min_height(
                        ui.ctx()
                            .fonts_mut(|fonts| fonts.row_height(&theme::regular(16.0)))
                            + 4.0,
                    );
                    inner(ui)
                })
                .inner
            });
        theme::focus_outline(
            ui,
            field.inner.id,
            field.response.rect,
            f32::from(theme::RADIUS),
        );
        field.inner
    };
    macro_rules! edit {
        ($buffer:expr, $salt:literal, $hint:expr, $width:expr) => {
            egui::TextEdit::singleline($buffer)
                .id(egui::Id::new($salt))
                .hint_text(
                    egui::RichText::new($hint)
                        .color(palette.dim)
                        .font(theme::regular(16.0)),
                )
                .font(theme::regular(16.0))
                .text_color(palette.text)
                .frame(egui::Frame::NONE)
                .desired_width($width)
        };
    }
    let phone_empty = app.new_contact_phone.is_empty();
    let phone_field = boxed(ui, &mut |ui| {
        let response = ui.add(edit!(
            &mut app.new_contact_phone,
            "new-contact-phone",
            "+1 555 123 4567",
            f32::INFINITY
        ));
        if !response.has_focus() && phone_empty {
            response.request_focus();
        }
        response
    });
    ui.add_space(4.0);
    let first_name_hint = crate::i18n::gettext(app.locale, "First name");
    let surname_hint = crate::i18n::gettext(app.locale, "Surname");
    let (first_field, last_field) = ui
        .horizontal(|ui| {
            let half = (ui.available_width() - ui.spacing().item_spacing.x) / 2.0 - 24.0;
            let format = egui::TextFormat::simple(theme::regular(16.0), palette.text);
            let mut layouter = |ui: &egui::Ui, text: &dyn egui::TextBuffer, wrap: f32| {
                crate::bidi::layout_field(ui, text.as_str(), &format, wrap)
            };
            let first_align = if crate::bidi::base_rtl(&app.new_contact_name) {
                Align::RIGHT
            } else {
                Align::LEFT
            };
            let last_align = if crate::bidi::base_rtl(&app.new_contact_last) {
                Align::RIGHT
            } else {
                Align::LEFT
            };
            let first = boxed(ui, &mut |ui| {
                ui.add(
                    edit!(
                        &mut app.new_contact_name,
                        "new-contact-first",
                        first_name_hint.as_ref(),
                        half
                    )
                    .horizontal_align(first_align)
                    .layouter(&mut layouter),
                )
            });
            let last = boxed(ui, &mut |ui| {
                ui.add(
                    edit!(
                        &mut app.new_contact_last,
                        "new-contact-last",
                        surname_hint.as_ref(),
                        half
                    )
                    .horizontal_align(last_align)
                    .layouter(&mut layouter),
                )
            });
            (first, last)
        })
        .inner;
    // A leading plus, spaces, hyphens and brackets are formatting; anything
    // else (an extension, a letter) would change the number, so it is refused.
    let digits = crate::util::international_phone(&app.new_contact_phone).unwrap_or_default();
    let ready = !digits.is_empty() && !app.new_contact_pending;
    let named = !app.new_contact_name.trim().is_empty() || !app.new_contact_last.trim().is_empty();
    let submitted =
        (phone_field.lost_focus() || first_field.lost_focus() || last_field.lost_focus())
            && ui.input(|input| input.key_pressed(egui::Key::Enter));
    ui.add_space(4.0);
    // As on the phone, each contact chooses; the choice starts the next one.
    ui.add_enabled(
        named,
        egui::Checkbox::new(
            &mut app.new_contact_to_phone,
            crate::i18n::gettext(app.locale, "Also save to your phone's contacts"),
        ),
    );
    let to_phone = Some(app.new_contact_to_phone);
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        if app.new_contact_pending {
            theme::spinner(ui, 16.0, palette.accent);
            theme::text(
                ui,
                crate::i18n::gettext(app.locale, "Checking the number…"),
                theme::regular(12.5),
                palette.secondary,
            );
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let (save, message) = ui
                .add_enabled_ui(ready, |ui| {
                    let hint = if app.new_contact_pending {
                        crate::i18n::gettext(app.locale, "Checking the number…")
                    } else {
                        crate::i18n::gettext(
                            app.locale,
                            "Enter a phone number with its country code, using only digits, spaces, hyphens and brackets",
                        )
                    };
                    let save = theme::pill_button(
                        ui,
                        &palette,
                        &crate::i18n::gettext(app.locale, "Save contact"),
                        named,
                    )
                    .on_disabled_hover_text(hint.as_ref())
                    .clicked();
                    let message = theme::pill_button(
                        ui,
                        &palette,
                        &crate::i18n::gettext(app.locale, "Message"),
                        !named,
                    )
                    .on_disabled_hover_text(hint.as_ref())
                    .clicked();
                    (save, message)
                })
                .inner;
            if theme::pill_button(
                ui,
                &palette,
                &crate::i18n::gettext(app.locale, "Cancel"),
                false,
            )
            .clicked()
            {
                app.actions.push(Action::CloseDialog);
            }
            // Enter saves a named contact or opens an unnamed chat.
            if ready && (save || (submitted && named)) {
                app.actions.push(Action::NewContact {
                    phone: digits.clone(),
                    first: app.new_contact_name.trim().to_owned(),
                    last: app.new_contact_last.trim().to_owned(),
                    to_phone,
                });
            } else if ready && (message || submitted) {
                app.actions.push(Action::NewContact {
                    phone: digits.clone(),
                    first: String::new(),
                    last: String::new(),
                    to_phone: None,
                });
            }
        });
    });
}

/// The disappearing-message timer line, centred under a chat's photo with the
/// same timer icon as the chat list badge, and what it drew.
///
/// The row is measured and centred by hand: a nested centred container claims
/// the column's whole height in egui. The text is cut to what is left beside
/// the icon, so a long translation cannot widen the dialog, and the horizontal
/// item spacing is cleared because egui adds it on top of `gap`, which would
/// otherwise make the row wider than the budget it was measured against.
pub(super) fn disappearing_timer_row(
    ui: &mut egui::Ui,
    palette: &theme::Palette,
    text: &str,
) -> TimerRow {
    const ICON: f32 = 12.0;
    const GAP: f32 = 5.0;
    let font = theme::regular(12.5);
    let line = super::widgets::line(
        ui,
        text,
        font,
        palette.secondary,
        (ui.available_width() - ICON - GAP).max(0.0),
        1,
    );
    let height = line.size().y;
    let mut drawn = TimerRow {
        text: text.to_owned(),
        rect: Rect::NOTHING,
    };
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.add_space(((ui.available_width() - ICON - GAP - line.size().x) / 2.0).max(0.0));
        let (slot, _) = ui.allocate_exact_size(vec2(ICON, height), Sense::hover());
        let icon = Rect::from_center_size(slot.center(), vec2(ICON, ICON));
        theme::paint_icon(ui, Icon::Timer, icon, ICON, palette.secondary);
        ui.add_space(GAP);
        let (slot, _) = ui.allocate_exact_size(line.size(), Sense::hover());
        line.paint(ui, slot.min, palette.secondary);
        drawn.rect = icon.union(slot);
    });
    drawn
}

/// Where the disappearing-message timer row was drawn, for tests: what it
/// said and where it landed, and nothing at all when no row was drawn.
pub fn disappearing_timer_id() -> egui::Id {
    egui::Id::new("disappearing-timer")
}

/// The timer row as drawn, for tests.
#[derive(Clone, Debug)]
pub struct TimerRow {
    pub text: String,
    pub rect: egui::Rect,
}

/// A filled button for a destructive action.
pub(super) fn danger_button(ui: &mut egui::Ui, app: &mut App, label: &str) -> egui::Response {
    let palette = app.palette;
    let galley = ui.painter().layout_no_wrap(
        label.to_owned(),
        theme::semibold(13.0),
        egui::Color32::WHITE,
    );
    let size = galley.size() + egui::vec2(36.0, 16.0);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    // Announced like every other button: AccessKit reads its label. In a
    // dialog egui's own Tab order reaches it; in the info panel the caller
    // puts it in the main cycle.
    theme::reveal_focus(&response);
    theme::focus_outline(ui, response.id, rect, rect.height() / 2.0);
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label)
    });
    if ui.is_rect_visible(rect) {
        let fill = if response.hovered() {
            palette.danger.gamma_multiply(0.85)
        } else {
            palette.danger
        };
        ui.painter().rect_filled(rect, rect.height() / 2.0, fill);
        ui.painter().galley(
            rect.center() - galley.size() / 2.0,
            galley,
            egui::Color32::WHITE,
        );
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

#[cfg(test)]
mod tests {
    #[test]
    fn chinese_delete_body_is_neutral_for_one_or_multiple_selected_messages() {
        for count in [1, 2] {
            let body =
                super::deletion_body(crate::i18n::Locale::ChineseSimplified, false, Some(count));
            assert!(body.contains("手机和关联设备"), "{body}");
            assert!(body.contains("所选消息"), "{body}");
            assert!(!body.contains("这些消息"), "{body}");
        }
    }
    use crate::model::{Chat, ChatKind};
    use crate::theme;

    /// The timer line stays inside the chat info dialog's own width whatever it
    /// has to say: a label longer than the dialog is cut, not laid out wider,
    /// and a short one is centred on it.
    #[test]
    fn the_timer_row_fits_the_dialog_however_long_its_label() {
        let width = 360.0_f32;
        for label in [
            "Disappearing messages: 24 hours",
            "Disappearing messages: 90 days",
            &"Disappearing messages: a translation far longer than any chat info dialog can be "
                .repeat(4),
        ] {
            let bounds = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 400.0));
            let ctx = egui::Context::default();
            let slot = std::cell::RefCell::new(None);
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(bounds),
                    ..Default::default()
                },
                |ui| {
                    let mut row_ui = egui::Ui::new(
                        ui.ctx().clone(),
                        egui::Id::new("timer-row-test"),
                        egui::UiBuilder::new().max_rect(bounds),
                    );
                    *slot.borrow_mut() = Some(super::disappearing_timer_row(
                        &mut row_ui,
                        &theme::Palette::dark(),
                        label,
                    ));
                },
            );
            // Headless tests must apply font-atlas updates themselves.
            output.textures_delta.clear();
            let drawn = slot.into_inner().expect("a drawn row");
            assert!(
                drawn.rect.left() >= bounds.left() - 0.5,
                "{label:?}: {} starts left of the dialog",
                drawn.rect.left()
            );
            assert!(
                drawn.rect.right() <= bounds.right() + 0.5,
                "{label:?}: {} runs past the dialog",
                drawn.rect.right()
            );
            assert!(
                (drawn.rect.center().x - bounds.center().x).abs() < 1.0,
                "{label:?}: off centre by {}",
                drawn.rect.center().x - bounds.center().x
            );
        }
    }

    #[test]
    fn locked_chats_are_not_forward_destinations() {
        let mut chat = Chat::new("1@s.whatsapp.net".into(), "Ada".into());
        chat.locked = true;

        assert!(!super::forwardable(&chat));

        chat.locked = false;
        assert!(super::forwardable(&chat));

        chat.kind = ChatKind::Broadcast;
        assert!(!super::forwardable(&chat));
    }
}

#[cfg(test)]
mod deletion_count_tests {
    #[test]
    fn confirmation_counts_include_a_single_selected_message() {
        assert_eq!(
            super::selected_message_count(crate::i18n::Locale::English, 1),
            "1 selected message"
        );
        assert_eq!(
            super::selected_message_count(crate::i18n::Locale::English, 2),
            "2 selected messages"
        );
        for (count, expected) in [
            (1, "1 сообщение выбрано"),
            (2, "2 сообщения выбрано"),
            (5, "5 сообщений выбрано"),
        ] {
            assert_eq!(
                super::selected_message_count(crate::i18n::Locale::Russian, count),
                expected
            );
        }
    }
}
