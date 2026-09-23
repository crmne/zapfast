//! Native preview for downloaded image attachments.

use egui::{Align, CornerRadius, Frame, Layout, Margin, Stroke, Vec2, vec2};

use crate::app::App;
use crate::model::Action;
use crate::theme::{self, Icon};

pub fn show(app: &mut App, ctx: &egui::Context) {
    let Some(preview) = app.image_preview.clone() else {
        return;
    };
    let palette = app.palette;
    let frame = Frame::new()
        .fill(palette.overlay)
        .stroke(Stroke::new(1.0, palette.outline))
        .corner_radius(CornerRadius::same(theme::RADIUS + 4))
        .inner_margin(Margin::same(14));
    let viewport = ctx.content_rect().size();
    let response = egui::Modal::new(egui::Id::new("image-preview"))
        .frame(frame)
        .backdrop_color(palette.shadow)
        .show(ctx, |ui| {
            ui.set_width((viewport.x * 0.9).clamp(viewport.x.min(320.0), 1200.0));
            ui.set_height((viewport.y * 0.88).clamp(viewport.y.min(260.0), 900.0));
            ui.horizontal(|ui| {
                let name = preview
                    .path()
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("Image");
                crate::ui::widgets::rich_text(ui, name, theme::semibold(14.0), palette.text);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if theme::icon_button(
                        ui,
                        Icon::X,
                        18.0,
                        palette.secondary,
                        palette.text,
                        "Close preview (Esc)",
                    )
                    .clicked()
                    {
                        app.actions.push(Action::CloseImagePreview);
                    }
                    if theme::icon_button(
                        ui,
                        Icon::ExternalLink,
                        18.0,
                        palette.secondary,
                        palette.text,
                        "Open in another app",
                    )
                    .clicked()
                    {
                        app.actions
                            .push(Action::OpenFile(preview.path().to_owned()));
                    }
                    ui.add_space(8.0);
                    // Right to left: zoom in, the current scale, zoom out.
                    if theme::icon_button(
                        ui,
                        Icon::Plus,
                        18.0,
                        palette.secondary,
                        palette.text,
                        "Zoom in",
                    )
                    .clicked()
                    {
                        app.actions.push(Action::ZoomImageIn);
                    }
                    // One control shows the scale and switches between fitting
                    // the window and the original size.
                    let (label, hint, action) = if preview.is_fit() {
                        (
                            "Fit".to_owned(),
                            "Show at original size",
                            Action::ImageActualSize,
                        )
                    } else {
                        (
                            format!("{:.0}%", preview.zoom() * 100.0),
                            "Fit to the window (0)",
                            Action::FitImage,
                        )
                    };
                    if theme::soft_button(ui, &palette, None, &label, false)
                        .on_hover_text(hint)
                        .clicked()
                    {
                        app.actions.push(action);
                    }
                    if theme::icon_button(
                        ui,
                        Icon::Minus,
                        18.0,
                        palette.secondary,
                        palette.text,
                        "Zoom out",
                    )
                    .clicked()
                    {
                        app.actions.push(Action::ZoomImageOut);
                    }
                });
            });
            ui.separator();

            let canvas = vec2(
                ui.available_width().max(0.0),
                ui.available_height().max(0.0),
            );
            // Registered with the image cache like every other draw site, so a
            // sweep never releases the picture while it is on screen.
            let image = crate::ui::widgets::file_image(ui, preview.path());
            match image.load_for_size(ctx, canvas) {
                Ok(egui::load::TexturePoll::Ready { texture }) => {
                    let size = display_size(texture.size, canvas, preview.is_fit(), preview.zoom());
                    if preview.is_fit()
                        && texture.size.x > 0.0
                        && let Some(state) = &mut app.image_preview
                    {
                        state.set_fit_scale(size.x / texture.size.x);
                    }
                    egui::ScrollArea::both()
                        .id_salt("image-preview-scroll")
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.allocate_ui_with_layout(
                                canvas.max(size),
                                Layout::centered_and_justified(egui::Direction::TopDown),
                                |ui| {
                                    ui.add(image.fit_to_exact_size(size));
                                },
                            );
                        });
                }
                Ok(egui::load::TexturePoll::Pending { .. }) => {
                    let (rect, _) = ui.allocate_exact_size(canvas, egui::Sense::hover());
                    theme::paint_spinner(ui, rect, 28.0, palette.accent);
                }
                Err(_) => {
                    ui.allocate_ui_with_layout(
                        canvas,
                        Layout::centered_and_justified(egui::Direction::TopDown),
                        |ui| {
                            ui.label("This image could not be displayed in ZapFast.");
                            if ui.button("Open externally").clicked() {
                                app.actions
                                    .push(Action::OpenFile(preview.path().to_owned()));
                            }
                        },
                    );
                }
            }
        });
    if response.should_close() {
        app.actions.push(Action::CloseImagePreview);
    }
}

/// Size the image is drawn at from the texture's intrinsic pixel dimensions:
/// fitted into the canvas, or scaled by the preview's zoom factor. Zoom is
/// applied here only. The size hint passed when loading does not change the
/// texture: egui decodes raster formats (all the preview accepts) once at full
/// resolution and reports the source size, whatever size is asked for.
fn display_size(original: Vec2, canvas: Vec2, fit: bool, zoom: f32) -> Vec2 {
    let (width, height) = if fit {
        crate::image_preview::fit_size(original.x, original.y, canvas.x, canvas.y)
    } else {
        crate::image_preview::zoomed_size(original.x, original.y, zoom)
    };
    vec2(width, height)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fitted_images_keep_aspect_ratio_inside_the_canvas() {
        assert_eq!(
            display_size(vec2(1600.0, 1200.0), vec2(800.0, 700.0), true, 1.0),
            vec2(800.0, 600.0)
        );
        assert_eq!(
            display_size(vec2(320.0, 240.0), vec2(800.0, 700.0), true, 1.0),
            vec2(320.0, 240.0)
        );
        assert_eq!(
            display_size(vec2(320.0, 240.0), vec2(800.0, 700.0), false, 2.0),
            vec2(640.0, 480.0)
        );
    }
}
