//! Taking a photo with the camera: a live viewfinder and a shutter. The shot is staged in the
//! composer like a pasted picture, for the same review, caption and editor before it goes.

use egui::{Align, Color32, CornerRadius, Layout, Rect, Sense, pos2, vec2};

use crate::app::App;
use crate::model::Action;
use crate::theme::{self, Icon};
use crate::viewfinder::{State, Viewfinder};

/// The viewfinder's shape while the camera has not said its own.
const WAITING_ASPECT: f32 = 3.0 / 4.0;

pub fn show(app: &mut App, ui: &mut egui::Ui, chat: &str) {
    let palette = app.palette;
    ui.horizontal(|ui| {
        theme::icon(ui, Icon::Camera, 20.0, palette.accent);
        theme::text(
            ui,
            crate::i18n::gettext(app.locale, "Take photo"),
            theme::bold(18.0),
            palette.text,
        );
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
    ui.add_space(8.0);

    // The camera starts with the dialog and stops when it closes (`dialogs::show`).
    let viewfinder = app
        .viewfinder
        .get_or_insert_with(|| Viewfinder::start(ui.ctx().clone()));
    let state = viewfinder.state();
    let width = ui.available_width();
    // Room for the header and the buttons, however short the window.
    let tallest = (ui.ctx().content_rect().height() - 200.0).max(120.0);
    match state {
        State::Live(_) => {
            if let Some(texture) = viewfinder.texture(ui.ctx()) {
                let [picture_w, picture_h] = texture.size().map(|side| side as f32);
                let scale = (width / picture_w).min(tallest / picture_h);
                let size = vec2(picture_w * scale, picture_h * scale);
                let (rect, _) = ui.allocate_exact_size(vec2(width, size.y), Sense::hover());
                let rect = Rect::from_center_size(rect.center(), size);
                // Mirrored, as a mirror shows a face; the photo itself is not.
                egui::Image::from_texture((texture.id(), size))
                    .uv(Rect::from_min_max(pos2(1.0, 0.0), pos2(0.0, 1.0)))
                    .corner_radius(CornerRadius::same(theme::RADIUS))
                    .paint_at(ui, rect);
            }
        }
        State::Opening | State::Failed => {
            let height = (width * WAITING_ASPECT).min(tallest);
            let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
            ui.painter().rect_filled(
                rect,
                CornerRadius::same(theme::RADIUS),
                Color32::from_black_alpha(40),
            );
            let message = if matches!(state, State::Opening) {
                crate::i18n::gettext(app.locale, "Opening the camera…")
            } else {
                crate::i18n::gettext(
                    app.locale,
                    "The camera could not be opened. Check that one is connected and that no other app or call is using it.",
                )
            };
            let inner = rect.shrink(24.0);
            ui.scope_builder(
                egui::UiBuilder::new()
                    .max_rect(inner)
                    .layout(Layout::top_down(Align::Center).with_main_align(Align::Center)),
                |ui| {
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(message.as_ref())
                                .font(theme::regular(13.0))
                                .color(palette.dim),
                        )
                        .wrap(),
                    );
                },
            );
        }
    }
    ui.add_space(8.0);

    let can_shoot = matches!(state, State::Live(_)) && app.open_chat.as_deref() == Some(chat);
    ui.horizontal(|ui| {
        let cancel = crate::i18n::gettext(app.locale, "Cancel");
        if theme::soft_button(ui, &palette, None, cancel.as_ref(), false).clicked() {
            app.actions.push(Action::CloseDialog);
        }
        ui.add_enabled_ui(can_shoot, |ui| {
            let shoot = crate::i18n::gettext(app.locale, "Take photo");
            if theme::pill_button(ui, &palette, shoot.as_ref(), true).clicked()
                && let Some((width, height, rgba)) =
                    app.viewfinder.as_ref().and_then(Viewfinder::shot)
            {
                app.actions.push(Action::CloseDialog);
                app.actions.push(Action::PasteImage {
                    width,
                    height,
                    rgba,
                });
            }
        });
    });
}
