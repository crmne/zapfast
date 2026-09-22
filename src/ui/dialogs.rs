//! Shortcuts, account, linking, contact, and chat dialogs.

use egui::{Align, CornerRadius, Frame, Layout, Margin, Sense, Stroke, pos2, vec2};

use crate::app::App;
use crate::model::{Action, Dialog};
use crate::theme::{self, Icon};

pub fn show(app: &mut App, ctx: &egui::Context) {
    let Some(dialog) = app.dialog.clone() else {
        return;
    };
    let palette = app.palette;
    let frame = Frame::new()
        .fill(palette.overlay)
        .stroke(Stroke::new(1.0, palette.outline))
        .corner_radius(CornerRadius::same(theme::RADIUS + 4))
        .inner_margin(Margin::same(22))
        .shadow(egui::epaint::Shadow {
            offset: [0, 12],
            blur: 40,
            spread: 0,
            color: palette.shadow,
        });
    let response = egui::Modal::new(egui::Id::new("dialog"))
        .frame(frame)
        .backdrop_color(palette.shadow)
        .show(ctx, |ui| {
            ui.set_width(match dialog {
                Dialog::Shortcuts => 540.0,
                Dialog::About => 380.0,
                Dialog::ConfirmUnlink => 380.0,
                Dialog::PairWithPhone => 380.0,
                Dialog::NewContact => 380.0,
                Dialog::NewChat => 420.0,
                Dialog::UnlockLockedChats | Dialog::ConfirmLockChat(_) => 380.0,
                Dialog::ChatInfo(_) => 360.0,
                Dialog::ConfirmDeleteChat(_) => 380.0,
                Dialog::Forward { .. } => 420.0,
                Dialog::CreatePoll(_) => 420.0,
            });
            ui.spacing_mut().item_spacing.y = 8.0;
            match dialog {
                Dialog::CreatePoll(chat) => super::polls::create(app, ui, &chat),
                Dialog::Shortcuts => shortcuts(app, ui),
                Dialog::About => about(app, ui),
                Dialog::ConfirmUnlink => confirm_unlink(app, ui),
                Dialog::PairWithPhone => pair_with_phone(app, ui),
                Dialog::NewContact => new_contact(app, ui),
                Dialog::NewChat => new_chat(app, ui),
                Dialog::UnlockLockedChats => unlock_locked_chats(app, ui),
                Dialog::ConfirmLockChat(id) => confirm_lock_chat(app, ui, &id),
                Dialog::ChatInfo(id) => chat_info(app, ui, &id),
                Dialog::ConfirmDeleteChat(id) => confirm_delete_chat(app, ui, &id),
                Dialog::Forward { chat, message } => forward(app, ui, &chat, &message),
            }
        });
    if response.should_close() {
        app.actions.push(Action::CloseDialog);
    }
}

fn unlock_locked_chats(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    let setup = app.settings.chat_lock_code_hash.is_none();
    title(
        ui,
        app,
        if setup {
            "Set up locked chats"
        } else {
            "Locked chats"
        },
    );
    theme::paragraph(
        ui,
        if setup {
            "Choose a local code to open the Locked tab. It is separate from your phone's code. This hides chats in ZapFast, it does not add another encryption layer."
        } else {
            "Enter your local ZapFast code. Leaving the Locked tab or closing the window locks it again."
        },
        theme::regular(13.0),
        palette.secondary,
    );
    let entry_id = ui.make_persistent_id("locked-code");
    let confirm_id = ui.make_persistent_id("locked-code-confirm");
    // TextEdit consumes Enter while surrendering focus. Capture and consume
    // it before drawing either editor instead of looking for it afterwards.
    let mut submit = ui
        .memory(|memory| memory.has_focus(entry_id) || (setup && memory.has_focus(confirm_id)))
        && ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
    let entry = ui.add(
        egui::TextEdit::singleline(&mut app.chat_lock_entry)
            .id(entry_id)
            .password(true)
            .hint_text("Local code")
            .desired_width(f32::INFINITY),
    );
    if ui.memory(|memory| memory.focused().is_none()) {
        entry.request_focus();
    }
    if setup {
        ui.add(
            egui::TextEdit::singleline(&mut app.chat_lock_confirm)
                .id(confirm_id)
                .password(true)
                .hint_text("Confirm code")
                .desired_width(f32::INFINITY),
        );
    }
    if app.chat_lock_error {
        theme::paragraph(
            ui,
            "That code did not match. Try again.",
            theme::regular(13.0),
            palette.danger,
        );
    }
    let ready = !app.chat_lock_entry.trim().is_empty()
        && (!setup || app.chat_lock_entry.trim() == app.chat_lock_confirm.trim());
    ui.horizontal(|ui| {
        submit |= ui
            .add_enabled_ui(ready, |ui| {
                theme::pill_button(
                    ui,
                    &palette,
                    if setup {
                        "Set code and open"
                    } else {
                        "Open locked chats"
                    },
                    true,
                )
            })
            .inner
            .clicked();
        if theme::pill_button(ui, &palette, "Cancel", false).clicked() {
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
    title(ui, app, "Lock this chat?");
    theme::paragraph(
        ui,
        "This moves the chat to the Locked tab, hiding it from the normal chat list, search, unread badges, and notifications. The lock status also syncs to your phone. Open Locked with a separate local ZapFast code to read it. Sending from locked chats is not supported yet.",
        theme::regular(13.0),
        palette.secondary,
    );
    ui.horizontal(|ui| {
        if theme::pill_button(ui, &palette, "Lock chat", true).clicked() {
            app.actions.push(Action::SetLocked(id.to_owned(), true));
            app.actions.push(Action::CloseDialog);
            if app.settings.chat_lock_code_hash.is_none() {
                app.actions.push(Action::OpenLockedFolder);
            }
        }
        if theme::pill_button(ui, &palette, "Cancel", false).clicked() {
            app.actions.push(Action::CloseDialog);
        }
    });
}

fn new_chat(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    title(ui, app, "New chat");
    ui.horizontal_wrapped(|ui| {
        if ui
            .add_enabled_ui(app.me.is_some(), |ui| {
                theme::soft_button(
                    ui,
                    &palette,
                    Some(Icon::MessageCircle),
                    "Message yourself",
                    false,
                )
            })
            .inner
            .clicked()
        {
            app.actions.push(Action::MessageYourself);
        }
        if theme::soft_button(ui, &palette, Some(Icon::Plus), "Add contact", false).clicked() {
            app.actions.push(Action::ShowDialog(Dialog::NewContact));
        }
    });
    let width = ui.available_width();
    let search = super::widgets::search_field(
        ui,
        &palette,
        egui::Id::new("new-chat-search"),
        &mut app.new_chat_search,
        "Search contacts",
        width,
    );
    if ui.memory(|memory| memory.focused().is_none()) {
        search.request_focus();
    }
    let needle = crate::util::search_key(app.new_chat_search.trim());
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
                    || crate::util::search_key(&app.display_name(&contact.id)).contains(&needle)
                    || crate::model::phone_of(&contact.id)
                        .is_some_and(|phone| phone.contains(&needle)))
        })
        .collect();
    contacts.sort_by_key(|contact| crate::util::search_key(&app.display_name(&contact.id)));
    egui::ScrollArea::vertical()
        .id_salt("new-chat-contacts")
        .max_height((ui.ctx().content_rect().height() - 280.0).clamp(100.0, 420.0))
        .auto_shrink([false, true])
        .show(ui, |ui| {
            if contacts.is_empty() {
                theme::text(
                    ui,
                    "No matching contacts",
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

fn forward(app: &mut App, ui: &mut egui::Ui, from_chat: &str, message: &str) {
    let palette = app.palette;
    title(ui, app, "Forward message");
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
    let max_height = (ui.ctx().content_rect().height() - 220.0).clamp(row_height * 3.0, 420.0);
    let mut destination = None;
    egui::ScrollArea::vertical()
        .id_salt("forward-chats")
        .max_height(max_height)
        .auto_shrink([false, true])
        .show_rows(ui, row_height, chats.len(), |ui, range| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for chat in &chats[range] {
                let title = app.chat_title(chat);
                let (rect, response) =
                    ui.allocate_exact_size(vec2(ui.available_width(), row_height), Sense::click());
                if ui.is_rect_visible(rect) {
                    if response.hovered() {
                        ui.painter().rect_filled(rect, 8.0, palette.surface_hover);
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
                        rect.width() - 62.0,
                        1,
                    );
                    line.paint(
                        ui,
                        pos2(rect.left() + 50.0, rect.center().y - line.size().y / 2.0),
                        palette.text,
                    );
                    ui.painter().hline(
                        (rect.left() + 50.0)..=rect.right(),
                        rect.bottom() - 0.5,
                        Stroke::new(1.0, palette.outline),
                    );
                }
                if response
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .clicked()
                {
                    destination = Some(chat.id.clone());
                }
            }
        });
    if chats.is_empty() {
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            theme::text(
                ui,
                "No writable chats found",
                theme::regular(13.5),
                palette.secondary,
            );
        });
        ui.add_space(12.0);
    }
    if let Some(to_chat) = destination {
        app.actions.push(Action::Forward {
            from_chat: from_chat.to_owned(),
            message: message.to_owned(),
            to_chat,
        });
    }
}

fn forwardable(chat: &crate::model::Chat) -> bool {
    chat.kind != crate::model::ChatKind::Broadcast && !chat.read_only && !chat.locked
}

fn title(ui: &mut egui::Ui, app: &mut App, label: &str) {
    let palette = app.palette;
    ui.horizontal(|ui| {
        theme::text(ui, label, theme::bold(18.0), palette.text);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if theme::icon_button(ui, Icon::X, 16.0, palette.secondary, palette.text, "Close")
                .clicked()
            {
                app.actions.push(Action::CloseDialog);
            }
        });
    });
    ui.add_space(4.0);
}

fn shortcuts(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    title(ui, app, "Keyboard shortcuts");
    // Reserve enough width for the longest shortcut before laying out the grid.
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
        .fold(0.0, f32::max);
    egui::Grid::new("shortcuts")
        .num_columns(2)
        .min_col_width(keys_width)
        .spacing([18.0, 8.0])
        .show(ui, |ui| {
            for (keys, what) in super::keys::SHORTCUTS {
                theme::text(
                    ui,
                    super::keys::label(keys),
                    theme::semibold(13.0),
                    palette.text,
                );
                theme::text(ui, *what, theme::regular(13.0), palette.secondary);
                ui.end_row();
            }
        });
}

fn about(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    title(ui, app, "About");
    ui.horizontal(|ui| {
        let (logo, _) = ui.allocate_exact_size(egui::Vec2::splat(44.0), egui::Sense::hover());
        theme::logo(
            ui,
            logo.center(),
            44.0,
            palette.accent,
            egui::Color32::WHITE,
        );
        ui.vertical(|ui| {
            theme::text(ui, "ZapFast", theme::bold(17.0), palette.text);
            theme::text(
                ui,
                format!("Version {}", env!("CARGO_PKG_VERSION")),
                theme::regular(13.0),
                palette.secondary,
            );
        });
    });
    ui.add_space(6.0);
    theme::paragraph(
        ui,
        "A native WhatsApp client written in Rust with egui. It connects through whatsapp-rust. Messages are end-to-end encrypted on this device.",
        theme::regular(13.0),
        palette.text,
    );
    theme::paragraph(
        ui,
        "This is an unofficial client. Using it may be against WhatsApp's terms of service and could get an account suspended.",
        theme::regular(12.5),
        palette.secondary,
    );
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        if theme::link(ui, "Source code", theme::medium(13.0), palette.link).clicked() {
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
}

fn confirm_delete_chat(app: &mut App, ui: &mut egui::Ui, id: &str) {
    let palette = app.palette;
    let name = app
        .chat(id)
        .map_or_else(|| "this chat".to_owned(), |chat| chat.name.clone());
    title(ui, app, "Delete chat?");
    theme::paragraph(
        ui,
        format!(
            "This deletes your chat with {name}, including all messages, on this computer and on your phone. It cannot be undone."
        ),
        theme::regular(13.5),
        palette.text,
    );
    ui.add_space(10.0);
    ui.horizontal(|ui| {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if danger_button(ui, app, "Delete") {
                app.actions.push(Action::DeleteChat(id.to_owned()));
                app.actions.push(Action::CloseDialog);
            }
            if theme::pill_button(ui, &palette, "Cancel", false).clicked() {
                app.actions.push(Action::CloseDialog);
            }
        });
    });
}

fn confirm_unlink(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    title(ui, app, "Unlink this computer?");
    theme::paragraph(
        ui,
        "This removes the device from WhatsApp and deletes the chats stored here. You can link again with a new code.",
        theme::regular(13.5),
        palette.text,
    );
    ui.add_space(10.0);
    ui.horizontal(|ui| {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if danger_button(ui, app, "Unlink") {
                app.actions.push(Action::Unlink);
            }
            if theme::pill_button(ui, &palette, "Cancel", false).clicked() {
                app.actions.push(Action::CloseDialog);
            }
        });
    });
}

/// Why the number dialogs cannot act yet.
const NUMBER_TOO_SHORT: &str = "Enter the whole number, starting with the country code";

fn pair_with_phone(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    title(ui, app, "Link with a phone number");
    theme::paragraph(
        ui,
        "Enter the WhatsApp phone number with its country code. Do not include a plus sign or leading zero. You will get a code to enter on the phone.",
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
            let get_code = ui
                .add_enabled_ui(ready, |ui| {
                    theme::pill_button(ui, &palette, "Get a code", true)
                        .on_disabled_hover_text(NUMBER_TOO_SHORT)
                })
                .inner;
            if (get_code.clicked() || submit) && ready {
                app.actions
                    .push(Action::PairWithPhone(app.pair_phone.clone()));
            }
            if theme::pill_button(ui, &palette, "Cancel", false).clicked() {
                app.actions.push(Action::CloseDialog);
            }
        });
    });
}

fn new_contact(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    title(ui, app, "New contact");
    theme::paragraph(
        ui,
        "Enter a phone number with its country code, without a plus sign or leading zero. Add a name to save the contact, or leave it blank to open the chat. WhatsApp uses the first name as the display name.",
        theme::regular(13.0),
        palette.secondary,
    );
    ui.add_space(4.0);
    let boxed =
        |ui: &mut egui::Ui, plus: bool, inner: &mut dyn FnMut(&mut egui::Ui) -> egui::Response| {
            let field = Frame::new()
                .fill(palette.surface)
                .corner_radius(CornerRadius::same(theme::RADIUS))
                .inner_margin(Margin::symmetric(12, 8))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        // Use the field height to align the plus sign.
                        ui.set_min_height(
                            ui.ctx()
                                .fonts_mut(|fonts| fonts.row_height(&theme::regular(16.0)))
                                + 4.0,
                        );
                        if plus {
                            theme::text(ui, "+", theme::semibold(16.0), palette.secondary);
                        }
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
        ($buffer:expr, $salt:literal, $hint:literal, $width:expr) => {
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
    let phone_field = boxed(ui, true, &mut |ui| {
        let response = ui.add(edit!(
            &mut app.new_contact_phone,
            "new-contact-phone",
            "15551234567",
            f32::INFINITY
        ));
        if !response.has_focus() && phone_empty {
            response.request_focus();
        }
        response
    });
    ui.add_space(4.0);
    let (first_field, last_field) = ui
        .horizontal(|ui| {
            let half = (ui.available_width() - ui.spacing().item_spacing.x) / 2.0 - 24.0;
            let first = boxed(ui, false, &mut |ui| {
                ui.add(edit!(
                    &mut app.new_contact_name,
                    "new-contact-first",
                    "First name",
                    half
                ))
            });
            let last = boxed(ui, false, &mut |ui| {
                ui.add(edit!(
                    &mut app.new_contact_last,
                    "new-contact-last",
                    "Surname",
                    half
                ))
            });
            (first, last)
        })
        .inner;
    let digits: String = app
        .new_contact_phone
        .chars()
        .filter(char::is_ascii_digit)
        .collect();
    let ready = digits.len() >= 7 && !app.new_contact_pending;
    let named = !app.new_contact_name.trim().is_empty() || !app.new_contact_last.trim().is_empty();
    let submitted =
        (phone_field.lost_focus() || first_field.lost_focus() || last_field.lost_focus())
            && ui.input(|input| input.key_pressed(egui::Key::Enter));
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        if app.new_contact_pending {
            theme::spinner(ui, 16.0, palette.accent);
            theme::text(
                ui,
                "Checking the number…",
                theme::regular(12.5),
                palette.secondary,
            );
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let (save, message) = ui
                .add_enabled_ui(ready, |ui| {
                    let hint = if app.new_contact_pending {
                        "Checking the number…"
                    } else {
                        NUMBER_TOO_SHORT
                    };
                    let save = theme::pill_button(ui, &palette, "Save contact", named)
                        .on_disabled_hover_text(hint)
                        .clicked();
                    let message = theme::pill_button(ui, &palette, "Message", !named)
                        .on_disabled_hover_text(hint)
                        .clicked();
                    (save, message)
                })
                .inner;
            if theme::pill_button(ui, &palette, "Cancel", false).clicked() {
                app.actions.push(Action::CloseDialog);
            }
            // Enter saves a named contact or opens an unnamed chat.
            if ready && (save || (submitted && named)) {
                app.actions.push(Action::NewContact {
                    phone: digits.clone(),
                    first: app.new_contact_name.trim().to_owned(),
                    last: app.new_contact_last.trim().to_owned(),
                });
            } else if ready && (message || submitted) {
                app.actions.push(Action::NewContact {
                    phone: digits.clone(),
                    first: String::new(),
                    last: String::new(),
                });
            }
        });
    });
}

fn chat_info(app: &mut App, ui: &mut egui::Ui, id: &str) {
    let palette = app.palette;
    // Group members may not have an existing chat.
    let chat = app
        .chat(id)
        .cloned()
        .unwrap_or_else(|| crate::model::Chat::new(id.to_owned(), app.display_name(id)));
    let has_chat = app.chat(id).is_some();
    let name = app.chat_title(&chat);
    title(ui, app, if chat.is_group() { "Group" } else { "Contact" });
    // Scale the photo and member list to fit the window.
    let window = ui.ctx().content_rect().height();
    let photo = (window * 0.34).clamp(120.0, 240.0);
    let picture = app.avatar_full(id).or_else(|| app.avatar(id));
    let mine = app.me.as_deref() == Some(id);
    let editable = chat.phone().is_some() && !mine;
    // Saving or cancelling leaves the editor buffer checked out.
    let mut editing = app.contact_edit.take().filter(|_| editable);
    let mut saved = None;
    ui.vertical_centered(|ui| {
        super::widgets::avatar(ui, &palette, &name, id, photo, picture.as_deref());
        ui.add_space(6.0);
        if let Some((first, last)) = editing.as_mut() {
            let mut submit = false;
            ui.horizontal(|ui| {
                ui.add_space((ui.available_width() - 288.0).max(0.0) / 2.0);
                let name_field = |ui: &mut egui::Ui, buffer: &mut String, salt: &str, hint| {
                    let field = Frame::new()
                        .fill(palette.surface)
                        .corner_radius(CornerRadius::same(theme::RADIUS))
                        .inner_margin(Margin::symmetric(10, 5))
                        .show(ui, |ui| {
                            ui.add(
                                egui::TextEdit::singleline(buffer)
                                    .id(egui::Id::new(salt))
                                    .hint_text(
                                        egui::RichText::new(hint)
                                            .color(palette.dim)
                                            .font(theme::semibold(15.0)),
                                    )
                                    .font(theme::semibold(15.0))
                                    .text_color(palette.text)
                                    .frame(Frame::NONE)
                                    .desired_width(108.0),
                            )
                        });
                    theme::focus_outline(
                        ui,
                        field.inner.id,
                        field.response.rect,
                        f32::from(theme::RADIUS),
                    );
                    field.inner
                };
                let first_field = name_field(ui, first, "contact-first", "First name");
                let last_field = name_field(ui, last, "contact-last", "Surname");
                if ui.memory(|memory| memory.focused().is_none()) {
                    first_field.request_focus();
                }
                submit = (first_field.lost_focus() || last_field.lost_focus())
                    && ui.input(|input| input.key_pressed(egui::Key::Enter));
                if theme::icon_button(
                    ui,
                    Icon::Check,
                    18.0,
                    palette.secondary,
                    palette.accent,
                    "Save name (Enter)",
                )
                .clicked()
                {
                    submit = true;
                }
            });
            if submit && !(first.trim().is_empty() && last.trim().is_empty()) {
                saved = Some((first.trim().to_owned(), last.trim().to_owned()));
            }
        } else {
            super::widgets::selectable_rich_text(ui, &name, theme::bold(19.0), palette.text);
        }
        if let Some(phone) = chat.phone() {
            theme::selectable_text(
                ui,
                crate::util::phone(phone),
                theme::regular(13.5),
                palette.secondary,
            );
        }
        if chat.is_group() && !chat.participants.is_empty() {
            theme::text(
                ui,
                format!("{} members", chat.participants.len()),
                theme::regular(13.5),
                palette.secondary,
            );
        }
        if let Some(presence) = app.presence.get(id) {
            let status = if presence.online {
                "online".to_owned()
            } else if let Some(seen) = presence.last_seen {
                format!("last seen {}", crate::util::chat_stamp(seen).to_lowercase())
            } else {
                String::new()
            };
            if !status.is_empty() {
                theme::text(ui, status, theme::regular(12.5), palette.dim);
            }
        }
    });
    if let Some((first, last)) = saved {
        editing = None;
        app.actions.push(Action::SaveContact {
            id: id.to_owned(),
            first,
            last,
        });
    }
    app.contact_edit = editing;
    ui.add_space(8.0);
    if chat.is_group() && !chat.participants.is_empty() {
        let members = app.participant_list(&chat);
        theme::text(
            ui,
            format!("Members ({})", members.len()),
            theme::medium(12.5),
            palette.secondary,
        );
        ui.add_space(4.0);
        // Limit the visible rows because groups can have thousands of members.
        let row_height = 30.0;
        let rows = ((window - photo - 300.0) / row_height)
            .floor()
            .clamp(2.0, 8.0);
        let mut open = None;
        egui::ScrollArea::vertical()
            .id_salt("members")
            .max_height(row_height * rows)
            .auto_shrink([false, true])
            .show_rows(ui, row_height, members.len(), |ui, range| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for (member, name) in &members[range] {
                    let (rect, response) = ui.allocate_exact_size(
                        egui::vec2(ui.available_width(), row_height),
                        egui::Sense::click(),
                    );
                    if ui.is_rect_visible(rect) {
                        if response.hovered() {
                            ui.painter().rect_filled(rect, 6.0, palette.surface_hover);
                        }
                        let picture = app.avatar(member);
                        let avatar = egui::Rect::from_center_size(
                            egui::pos2(rect.left() + 16.0, rect.center().y),
                            egui::Vec2::splat(24.0),
                        );
                        super::widgets::paint_avatar(
                            ui,
                            &palette,
                            avatar,
                            name.trim_start_matches('~'),
                            member,
                            picture.as_deref(),
                        );
                        let line = super::widgets::line(
                            ui,
                            name,
                            theme::regular(13.0),
                            palette.text,
                            rect.width() - 40.0,
                            1,
                        );
                        line.paint(
                            ui,
                            egui::pos2(rect.left() + 34.0, rect.center().y - line.size().y / 2.0),
                            palette.text,
                        );
                    }
                    if response
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .clicked()
                    {
                        open = Some(member.clone());
                    }
                }
            });
        if let Some(member) = open
            && Some(member.as_str()) != app.me.as_deref()
        {
            app.actions
                .push(Action::ShowDialog(Dialog::ChatInfo(member)));
        }
        ui.add_space(6.0);
    }
    if let Some(until) = chat.muted_until {
        theme::text(
            ui,
            if until == 0 {
                "Muted".to_owned()
            } else {
                format!("Muted until {}", crate::util::chat_stamp(until))
            },
            theme::regular(12.5),
            palette.secondary,
        );
    }
    ui.add_space(4.0);
    // Center the available actions below the picture.
    let mut buttons: Vec<(Icon, &str, Vec<Action>)> = Vec::new();
    if !chat.is_group() && !mine {
        buttons.push((
            Icon::MessageCircle,
            "Message",
            vec![
                Action::StartChat {
                    id: chat.id.clone(),
                    name: name.clone(),
                },
                Action::CloseDialog,
            ],
        ));
    }
    if editable {
        let known = app
            .contacts
            .get(id)
            .and_then(|contact| contact.full_name.as_deref())
            .is_some_and(|full| !full.is_empty());
        buttons.push((
            Icon::User,
            if known { "Rename" } else { "Add to contacts" },
            vec![Action::EditContact(name.trim_start_matches('~').to_owned())],
        ));
    }
    if let Some(phone) = chat.phone() {
        buttons.push((
            Icon::Copy,
            "Copy number",
            vec![Action::CopyText(format!("+{phone}"))],
        ));
    }
    if has_chat {
        let muted = chat.muted(crate::util::now());
        buttons.push(if muted {
            (
                Icon::Bell,
                "Unmute",
                vec![Action::SetMuted(chat.id.clone(), None)],
            )
        } else {
            (
                Icon::BellOff,
                "Mute",
                vec![Action::SetMuted(chat.id.clone(), Some(0))],
            )
        });
        buttons.push((
            if chat.pinned { Icon::PinOff } else { Icon::Pin },
            if chat.pinned { "Unpin" } else { "Pin" },
            vec![Action::SetPinned(chat.id.clone(), !chat.pinned)],
        ));
        buttons.push((
            Icon::Archive,
            if chat.archived {
                "Unarchive"
            } else {
                "Archive"
            },
            vec![
                Action::SetArchived(chat.id.clone(), !chat.archived),
                Action::CloseDialog,
            ],
        ));
    }
    let spacing = ui.spacing().item_spacing.x;
    let available = ui.available_width();
    let mut fired: Option<Vec<Action>> = None;
    let mut start = 0;
    while start < buttons.len() {
        // Fit and center as many buttons as each row allows.
        let mut end = start;
        let mut total = 0.0;
        while end < buttons.len() {
            let width = theme::soft_button_width(ui, buttons[end].1, true);
            let grown = if end == start {
                width
            } else {
                total + spacing + width
            };
            if end > start && grown > available {
                break;
            }
            total = grown;
            end += 1;
        }
        ui.horizontal(|ui| {
            ui.add_space((available - total).max(0.0) / 2.0);
            for (icon, label, actions) in &buttons[start..end] {
                if theme::soft_button(ui, &palette, Some(*icon), label, false).clicked() {
                    fired = Some(actions.clone());
                }
            }
        });
        start = end;
    }
    if let Some(actions) = fired {
        app.actions.extend(actions);
    }
}

/// A filled button for a destructive action.
fn danger_button(ui: &mut egui::Ui, app: &mut App, label: &str) -> bool {
    let palette = app.palette;
    let galley = ui.painter().layout_no_wrap(
        label.to_owned(),
        theme::semibold(13.0),
        egui::Color32::WHITE,
    );
    let size = galley.size() + egui::vec2(36.0, 16.0);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
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
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
}

#[cfg(test)]
mod tests {
    use crate::model::{Chat, ChatKind};

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
