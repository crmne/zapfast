//! The camera window opened from the composer's plus menu: a live preview,
//! a Mirror switch, and buttons to take the photo or cancel. The preview
//! shows the photo exactly as it will be staged.

use egui::{Align, CornerRadius, Frame, Layout, Margin, Stroke, vec2};

use crate::app::App;
use crate::i18n::gettext;
use crate::model::Action;
use crate::theme;

pub fn show(app: &mut App, ctx: &egui::Context) {
    let palette = app.palette;
    let locale = app.locale;
    let mirror = app.photo_mirror;
    let Some(capture) = app.photo_capture.as_mut() else {
        return;
    };
    let frame = capture.frame();
    let error = capture.error();
    if let Some(frame) = &frame
        && capture.texture.as_ref().map(|(serial, _)| *serial) != Some(frame.serial)
    {
        let image =
            egui::ColorImage::from_rgba_unmultiplied([frame.width, frame.height], &frame.rgba);
        match &mut capture.texture {
            Some((serial, texture)) => {
                texture.set(image, egui::TextureOptions::LINEAR);
                *serial = frame.serial;
            }
            None => {
                let texture =
                    ctx.load_texture("photo-capture", image, egui::TextureOptions::LINEAR);
                capture.texture = Some((frame.serial, texture));
            }
        }
    }
    let texture = capture.texture.as_ref().map(|(_, texture)| texture.clone());
    let ready = frame.is_some() && error.is_none();

    let mut take = false;
    let mut cancel = false;
    let mut flip = false;
    let modal_frame = Frame::new()
        .fill(palette.overlay)
        .stroke(Stroke::new(1.0, palette.outline))
        .corner_radius(CornerRadius::same(theme::RADIUS + 4))
        .inner_margin(Margin::same(16))
        .shadow(palette.modal_shadow());
    let response = egui::Modal::new(modal_id())
        .frame(modal_frame)
        .backdrop_color(palette.shadow)
        .show(ctx, |ui| {
            let screen = ui.ctx().content_rect().size();
            let width = (screen.x - 64.0).clamp(240.0, 720.0);
            let max_height = (screen.y - 160.0).max(160.0);
            ui.set_width(width);
            ui.label(
                egui::RichText::new(gettext(locale, "Take photo").as_ref())
                    .strong()
                    .size(16.0)
                    .color(palette.text),
            );
            ui.add_space(6.0);
            match (&texture, &error) {
                (_, Some(error)) => {
                    ui.label(
                        gettext(locale, "The camera could not be opened: {error}")
                            .replace("{error}", error),
                    );
                }
                (Some(texture), None) => {
                    let [w, h] = texture.size();
                    let mut size = vec2(width, width * h as f32 / w as f32);
                    if size.y > max_height {
                        size = vec2(max_height * w as f32 / h as f32, max_height);
                    }
                    let uv = if mirror {
                        egui::Rect::from_min_max(egui::pos2(1.0, 0.0), egui::pos2(0.0, 1.0))
                    } else {
                        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0))
                    };
                    let image = egui::Image::new((texture.id(), size))
                        .uv(uv)
                        .corner_radius(CornerRadius::same(theme::RADIUS));
                    ui.vertical_centered(|ui| ui.add(image));
                }
                (None, None) => {
                    ui.add_sized(vec2(width, width * 9.0 / 16.0), egui::Spinner::new());
                }
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let mut mirrored = mirror;
                if ui
                    .checkbox(&mut mirrored, gettext(locale, "Mirror").as_ref())
                    .on_hover_text(gettext(locale, "Flip the photo left to right (M)"))
                    .changed()
                {
                    flip = true;
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui
                        .add_enabled(
                            ready,
                            egui::Button::new(gettext(locale, "Take photo").as_ref()),
                        )
                        .clicked()
                    {
                        take = true;
                    }
                    if ui.button(gettext(locale, "Cancel").as_ref()).clicked() {
                        cancel = true;
                    }
                });
            });
            ui.input(|input| {
                flip |= input.key_pressed(egui::Key::M);
                take |= ready && input.key_pressed(egui::Key::Enter);
                cancel |= input.key_pressed(egui::Key::Escape);
            });
        });
    if response.should_close() {
        cancel = true;
    }
    if flip {
        app.actions.push(Action::SetPhotoMirror(!mirror));
    }
    if take && let Some(frame) = frame {
        app.actions.push(Action::ClosePhotoCapture);
        app.actions.push(Action::PasteImage {
            width: frame.width,
            height: frame.height,
            rgba: crate::photo_capture::taken(frame, mirror),
        });
    } else if cancel {
        app.actions.push(Action::ClosePhotoCapture);
    }
}

/// The camera window's `Modal`, the layer it is drawn on.
pub(crate) fn modal_id() -> egui::Id {
    egui::Id::new("photo-capture")
}
