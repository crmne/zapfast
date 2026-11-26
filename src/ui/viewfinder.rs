//! The camera over the whole window, to take a photo for the open chat. The
//! photo is staged as a pasted picture is, and the cropper opens on it.

use egui::{Align2, Color32, Rect, Sense, Vec2, pos2, vec2};

use crate::app::App;
use crate::model::Action;
use crate::theme::{self, Icon};

/// Room left around the picture, in points, as the cropper leaves.
const MARGIN: f32 = 28.0;
/// Height of the row holding the close button.
const BAR: f32 = 44.0;
/// The round button that takes the photo.
const SHUTTER: f32 = 56.0;

/// The viewfinder's layer.
pub(crate) fn area_id() -> egui::Id {
    egui::Id::new("camera-viewfinder")
}

/// Draws the camera over the whole window while it is open.
pub fn show(app: &mut App, ctx: &egui::Context) {
    let Some(camera) = &app.camera else {
        // The last frame's texture goes with the camera.
        app.camera_texture = None;
        return;
    };
    let error = camera.error();
    let frame = camera.frame();
    let palette = app.palette;
    let locale = app.locale;
    let screen = ctx.content_rect();
    // It covers the window, so the pointer meets only the viewfinder.
    ctx.memory_mut(|memory| {
        memory.set_modal_layer(egui::LayerId::new(egui::Order::Foreground, area_id()));
    });
    egui::Area::new(area_id())
        .order(egui::Order::Foreground)
        .fixed_pos(screen.min)
        .show(ctx, |ui| {
            let (rect, _) = ui.allocate_exact_size(screen.size(), Sense::hover());
            ui.painter().rect_filled(rect, 0.0, palette.window);
            let room = rect.shrink(MARGIN);
            let top = Rect::from_min_size(room.min, vec2(room.width(), BAR));
            let shutter = Rect::from_center_size(
                pos2(room.center().x, room.max.y - SHUTTER / 2.0),
                Vec2::splat(SHUTTER),
            );
            let inner = Rect::from_min_max(
                pos2(room.min.x, top.max.y),
                pos2(room.max.x, shutter.min.y - MARGIN / 2.0),
            );
            let note = |ui: &egui::Ui, text: &str| {
                ui.painter().text(
                    inner.center(),
                    Align2::CENTER_CENTER,
                    text,
                    theme::regular(15.0),
                    palette.secondary,
                );
            };
            let texture = frame.as_ref().map(|frame| {
                let id = super::call::call_texture(
                    ui,
                    &mut app.camera_texture,
                    "camera-viewfinder",
                    frame,
                );
                (id, frame.size)
            });
            match (&texture, &error) {
                (_, Some(_)) => note(
                    ui,
                    &crate::i18n::gettext(locale, "The camera could not open"),
                ),
                (Some((texture, [width, height])), None) => {
                    ui.painter().image(
                        *texture,
                        super::call::fit_inside(inner, vec2(*width as f32, *height as f32)),
                        // Mirrored, as a mirror shows you, and the photo
                        // is taken as shown.
                        Rect::from_min_max(pos2(1.0, 0.0), pos2(0.0, 1.0)),
                        Color32::WHITE,
                    );
                }
                (None, None) => note(ui, &crate::i18n::gettext(locale, "Opening the camera…")),
            }
            let close = Rect::from_min_size(pos2(top.max.x - BAR, top.min.y), Vec2::splat(BAR));
            let closed = ui
                .scope_builder(egui::UiBuilder::new().max_rect(close), |ui| {
                    theme::icon_button(
                        ui,
                        Icon::X,
                        18.0,
                        palette.secondary,
                        palette.text,
                        &crate::i18n::gettext(locale, "Close (Esc)"),
                    )
                })
                .inner;
            if closed.clicked() {
                app.actions.push(Action::CloseCamera);
            }
            let ready = texture.is_some() && error.is_none();
            let taken = ui
                .scope_builder(egui::UiBuilder::new().max_rect(shutter), |ui| {
                    ui.add_enabled_ui(ready, |ui| {
                        theme::circle_button(
                            ui,
                            Icon::Camera,
                            SHUTTER,
                            palette.accent,
                            palette.accent_hover,
                            palette.on_accent,
                            &crate::i18n::gettext(locale, "Take photo (Enter)"),
                        )
                    })
                    .inner
                })
                .inner;
            if taken.clicked() {
                app.actions.push(Action::TakePhoto);
            }
        });
}
