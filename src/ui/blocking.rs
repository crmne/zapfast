//! Blocking controls shared by the contact panel, menus and privacy dialog.

use crate::app::App;
use crate::i18n::gettext;
use crate::model::{Action, Dialog};
use crate::theme::{self, Icon};

use super::focus::{Stop, TabStop};
use super::widgets;

pub(super) fn label(app: &App, chat: &str) -> std::borrow::Cow<'static, str> {
    if app.blocklist.contains(chat) {
        gettext(app.locale, "Unblock contact")
    } else {
        gettext(app.locale, "Block contact")
    }
}

fn confirm(chat: &str, blocked: bool, from_list: bool) -> Action {
    Action::ShowDialog(Dialog::ConfirmBlock {
        chat: chat.into(),
        blocked,
        from_list,
    })
}

pub(super) fn menu(app: &mut App, ui: &mut egui::Ui, chat: &str) {
    if !crate::blocking::eligible(chat, app.me.as_deref(), app.me_lid.as_deref()) {
        return;
    }
    if widgets::menu_item_enabled(
        ui,
        &app.palette,
        Some(Icon::Ban),
        &label(app, chat),
        app.is_connected() && app.blocklist.editable(),
    ) {
        app.actions
            .push(confirm(chat, !app.blocklist.contains(chat), false));
    }
}

pub(super) fn button(app: &mut App, ui: &mut egui::Ui, chat: &str, stop: Stop) {
    if !crate::blocking::eligible(chat, app.me.as_deref(), app.me_lid.as_deref()) {
        return;
    }
    let text = label(app, chat);
    let enabled = app.is_connected() && app.blocklist.editable();
    if ui
        .add_enabled_ui(enabled, |ui| {
            theme::soft_button(ui, &app.palette, Some(Icon::Ban), &text, false)
        })
        .inner
        .tab_stop(stop)
        .clicked()
    {
        app.actions
            .push(confirm(chat, !app.blocklist.contains(chat), false));
    }
}

fn hidden(app: &App, chat: &str) -> bool {
    app.chat(chat).is_some_and(|chat| chat.locked) && !app.locked_folder_open()
}

fn name(app: &App, chat: &str) -> String {
    if hidden(app, chat) {
        gettext(app.locale, "Locked chat").into_owned()
    } else {
        app.chat_title_of(chat)
    }
}

pub(super) fn confirmation(app: &mut App, ui: &mut egui::Ui, chat: &str, blocked: bool) {
    let title = if blocked {
        gettext(app.locale, "Block contact?")
    } else {
        gettext(app.locale, "Unblock contact?")
    };
    let body = if blocked {
        gettext(app.locale, "Block {name}? You will no longer receive their direct messages or calls. This keeps your chat history.")
    } else {
        gettext(app.locale, "Unblock {name}? You will be able to exchange messages and calls again.")
    }.replace("{name}", &name(app, chat));
    let button = if blocked {
        gettext(app.locale, "Block")
    } else {
        gettext(app.locale, "Unblock")
    };
    super::dialogs::confirm_destructive(
        app,
        ui,
        &title,
        &[&body],
        &button,
        [Action::SetBlocked {
            chat: chat.into(),
            blocked,
        }],
    );
}

pub(super) fn list(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    theme::text(
        ui,
        gettext(app.locale, "Blocked contacts"),
        theme::semibold(20.0),
        palette.text,
    );
    if app.blocklist.loading || app.blocklist.pending {
        ui.spinner();
    }
    if !app.is_connected() {
        theme::paragraph(
            ui,
            gettext(
                app.locale,
                "Connect to WhatsApp to block or unblock contacts.",
            ),
            theme::regular(13.5),
            palette.secondary,
        );
    }
    if app.blocklist.failed {
        theme::paragraph(
            ui,
            gettext(app.locale, "Could not load blocked contacts."),
            theme::regular(13.5),
            palette.secondary,
        );
        if ui
            .add_enabled_ui(app.is_connected(), |ui| {
                theme::pill_button(ui, &palette, &gettext(app.locale, "Try again"), false)
            })
            .inner
            .clicked()
        {
            app.actions.push(Action::FetchBlocklist);
        }
    }
    let mut ids: Vec<_> = app.blocklist.ids.iter().flatten().cloned().collect();
    ids.sort_by_cached_key(|id| crate::util::search_key(&name(app, id)));
    if app.blocklist.ids.is_some() && ids.is_empty() {
        theme::paragraph(
            ui,
            gettext(app.locale, "No blocked contacts"),
            theme::regular(13.5),
            palette.secondary,
        );
    }
    let unblock = gettext(app.locale, "Unblock");
    let button_width = ui
        .painter()
        .layout_no_wrap(unblock.to_string(), theme::semibold(13.0), palette.text)
        .size()
        .x
        + 36.0
        + ui.spacing().item_spacing.x;
    egui::ScrollArea::vertical()
        .max_height((ui.ctx().content_rect().height() - 220.0).max(80.0))
        .show_rows(ui, 36.0, ids.len(), |ui, range| {
            for id in &ids[range] {
                ui.push_id(id, |ui| {
                    ui.horizontal(|ui| {
                        let private = hidden(app, id);
                        let name = name(app, id);
                        let picture = (!private).then(|| app.avatar(id)).flatten();
                        widgets::avatar(
                            ui,
                            &palette,
                            &name,
                            if private { "locked" } else { id },
                            32.0,
                            picture.as_deref(),
                        );
                        ui.allocate_ui_with_layout(
                            egui::vec2((ui.available_width() - button_width).max(20.0), 36.0),
                            egui::Layout::left_to_right(egui::Align::Center),
                            |ui| {
                                ui.set_min_width(ui.available_width());
                                theme::text(ui, &name, theme::regular(14.0), palette.text);
                            },
                        );
                        if ui
                            .add_enabled_ui(app.is_connected() && app.blocklist.editable(), |ui| {
                                theme::pill_button(ui, &palette, &unblock, false)
                            })
                            .inner
                            .clicked()
                        {
                            app.actions.push(confirm(id, false, true));
                        }
                    });
                });
            }
        });
    if theme::pill_button(ui, &palette, &gettext(app.locale, "Close"), false).clicked() {
        app.actions.push(Action::CloseDialog);
    }
}
