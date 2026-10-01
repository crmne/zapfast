//! Sending a location: the spot the reader pastes, checked before it goes out.

use egui::{Align, Layout};

use crate::app::App;
use crate::geo;
use crate::model::Action;
use crate::theme::{self, Icon};

pub fn send(app: &mut App, ui: &mut egui::Ui, chat: &str) {
    let palette = app.palette;
    ui.horizontal(|ui| {
        theme::icon(ui, Icon::MapPin, 20.0, palette.accent);
        theme::text(
            ui,
            crate::i18n::gettext(app.locale, "Send location"),
            theme::bold(18.0),
            palette.text,
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if theme::icon_button(ui, Icon::X, 16.0, palette.secondary, palette.text, "Close")
                .clicked()
            {
                app.actions.push(Action::CloseDialog);
            }
        });
    });
    ui.add_space(8.0);
    ui.add(
        egui::TextEdit::singleline(&mut app.location_draft)
            .id_salt("send-location")
            .hint_text(crate::i18n::gettext(app.locale, "Paste a map link or coordinates").as_ref())
            .char_limit(2048)
            .font(theme::regular(14.0))
            .desired_width(f32::INFINITY),
    );
    ui.add_space(6.0);
    // The spot is read again here and shown before it is sent: a pair read the
    // wrong way round is visible in this dialog, not in someone else's chat.
    let here = geo::spot(&app.location_draft);
    match here {
        Some(here) => {
            theme::text(ui, here.text(), theme::regular(13.0), palette.text);
            let open = crate::i18n::gettext(app.locale, "Open in a map");
            if theme::link(ui, open.as_ref(), theme::regular(12.5), palette.link).clicked() {
                app.actions.push(Action::OpenUrl(geo::map_url(here)));
            }
        }
        None if app.location_draft.trim().is_empty() => {}
        None => {
            theme::text(
                ui,
                crate::i18n::gettext(app.locale, "That is not a location."),
                theme::regular(12.0),
                palette.dim,
            );
        }
    }
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        let cancel = crate::i18n::gettext(app.locale, "Cancel");
        if theme::soft_button(ui, &palette, None, cancel.as_ref(), false).clicked() {
            app.actions.push(Action::CloseDialog);
        }
        ui.add_enabled_ui(here.is_some() && app.link.is_connected(), |ui| {
            let send = crate::i18n::gettext(app.locale, "Send");
            if theme::pill_button(ui, &palette, send.as_ref(), true).clicked()
                && let Some(here) = here
            {
                app.actions.push(Action::SendLocation {
                    chat: chat.to_owned(),
                    latitude: here.latitude,
                    longitude: here.longitude,
                    quoting: app.reply_to.clone(),
                });
            }
        });
    });
}
