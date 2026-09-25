//! Native preview for downloaded image attachments.

use egui::{Align, CornerRadius, Frame, Layout, Margin, Rect, Stroke, Vec2, vec2};

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
                    let copy_hint = format!(
                        "{} ({})",
                        crate::i18n::gettext(app.locale, "Copy image"),
                        super::keys::label("Ctrl+C"),
                    );
                    if theme::icon_button(
                        ui,
                        Icon::Copy,
                        18.0,
                        palette.secondary,
                        palette.text,
                        &copy_hint,
                    )
                    .clicked()
                    {
                        app.actions
                            .push(Action::CopyImage(preview.path().to_owned()));
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

            // The scroll area below takes this rect as its viewport.
            let area = ui.available_rect_before_wrap();
            let canvas = area.size().max(Vec2::ZERO);
            // Registered with the image cache like every other draw site, so a
            // sweep never releases the picture while it is on screen.
            let image = crate::ui::widgets::file_image(ui, preview.path());
            match image.load_for_size(ctx, canvas) {
                Ok(egui::load::TexturePoll::Ready { texture }) => {
                    let mut size =
                        display_size(texture.size, canvas, preview.is_fit(), preview.zoom());
                    if preview.is_fit()
                        && texture.size.x > 0.0
                        && let Some(state) = &mut app.image_preview
                    {
                        state.set_fit_scale(size.x / texture.size.x);
                    }
                    let scroll_id =
                        ui.make_persistent_id(egui::IdSalt::new("image-preview-scroll"));
                    let trackpad = app.scroll_from_trackpad();
                    // Zoomed before the scroll area lays out, and its stored offset
                    // moved to keep the pointed-at pixel still: egui loads that
                    // state when the area begins, so both land in this frame.
                    if let Some((factor, pointer)) = zoom_input(ui, area, trackpad)
                        && let Some(state) = &mut app.image_preview
                    {
                        state.zoom_by(factor);
                        let zoomed =
                            display_size(texture.size, canvas, state.is_fit(), state.zoom());
                        let mut scroll =
                            egui::scroll_area::State::load(ctx, scroll_id).unwrap_or_default();
                        scroll.offset = crate::image_preview::anchored_offset(
                            canvas,
                            size,
                            zoomed,
                            scroll.offset,
                            pointer,
                            pointer,
                        );
                        scroll.store(ctx, scroll_id);
                        size = zoomed;
                    }
                    let output = egui::ScrollArea::both()
                        .id_salt("image-preview-scroll")
                        .auto_shrink([false, false])
                        // egui drags only on touch screens by default.
                        .scroll_source(egui::scroll_area::ScrollSource {
                            drag: egui::scroll_area::DragScroll::Always,
                            ..Default::default()
                        })
                        .on_hover_cursor(egui::CursorIcon::Grab)
                        .on_drag_cursor(egui::CursorIcon::Grabbing)
                        .show(ui, |ui| {
                            ui.allocate_ui_with_layout(
                                canvas.max(size),
                                Layout::centered_and_justified(egui::Direction::TopDown),
                                |ui| {
                                    let image_response = ui.add(
                                        image.fit_to_exact_size(size).sense(egui::Sense::click()),
                                    );
                                    let copy_label = crate::i18n::gettext(app.locale, "Copy image");
                                    let save_label = crate::i18n::gettext(app.locale, "Save as…");
                                    let open_label =
                                        crate::i18n::gettext(app.locale, "Open in another app");
                                    let menu_width = crate::ui::widgets::menu_width(
                                        ui,
                                        &[&copy_label, &save_label, &open_label],
                                        true,
                                    )
                                    .max(180.0);
                                    egui::Popup::context_menu(&image_response)
                                        .width(menu_width)
                                        .frame(crate::ui::widgets::menu_frame(&palette))
                                        .show(|ui| {
                                            if crate::ui::widgets::menu_item(
                                                ui,
                                                &palette,
                                                Some(Icon::Copy),
                                                &copy_label,
                                            ) {
                                                app.actions.push(Action::CopyImage(
                                                    preview.path().to_owned(),
                                                ));
                                            }
                                            if crate::ui::widgets::menu_item(
                                                ui,
                                                &palette,
                                                Some(Icon::Download),
                                                &save_label,
                                            ) {
                                                let name = preview
                                                    .path()
                                                    .file_name()
                                                    .and_then(|name| name.to_str())
                                                    .unwrap_or("image.png")
                                                    .to_owned();
                                                app.actions.push(Action::SaveAttachmentAs {
                                                    path: preview.path().to_owned(),
                                                    name,
                                                });
                                            }
                                            if crate::ui::widgets::menu_item(
                                                ui,
                                                &palette,
                                                Some(Icon::ExternalLink),
                                                &open_label,
                                            ) {
                                                app.actions.push(Action::OpenFile(
                                                    preview.path().to_owned(),
                                                ));
                                            }
                                        });
                                    image_response
                                        .interact_pointer_pos()
                                        .filter(|_| image_response.double_clicked())
                                },
                            )
                            .inner
                        });
                    // The header's Fit/% toggle. The original size opens with the
                    // double-clicked point in the middle: the offset is stored for
                    // the next frame, which lays out the new size.
                    if let Some(pos) = output.inner {
                        if app
                            .image_preview
                            .as_ref()
                            .is_some_and(crate::image_preview::PreviewState::is_fit)
                        {
                            let mut scroll = output.state;
                            scroll.offset = crate::image_preview::anchored_offset(
                                canvas,
                                size,
                                texture.size,
                                output.state.offset,
                                pos - area.min,
                                canvas / 2.0,
                            );
                            scroll.store(ctx, scroll_id);
                            app.actions.push(Action::ImageActualSize);
                        } else {
                            app.actions.push(Action::FitImage);
                        }
                    }
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

/// Zoom factor the wheel or a pinch asks for over the picture, with the
/// anchor point relative to the picture area. Each wheel notch is one header
/// zoom step. A plain mouse wheel zooms instead of scrolling, so its delta is
/// taken from the scroll area. Windows and X11 report touchpads as wheel
/// lines, so two fingers zoom there; only scrolling reported in points
/// (macOS, Wayland) keeps moving the picture.
fn zoom_input(ui: &mut egui::Ui, area: Rect, trackpad: bool) -> Option<(f32, Vec2)> {
    let (touch, hover) = ui.input(|input| {
        (
            input.multi_touch().map(|touch| touch.center_pos),
            input.pointer.hover_pos(),
        )
    });
    let anchor = touch.or(hover)?;
    if !area.contains(anchor) || touch.is_none() && !ui.rect_contains_pointer(area) {
        return None;
    }
    let (zoom_speed, line_speed) = ui.ctx().options(|options| {
        let input = &options.input_options;
        (input.scroll_zoom_speed, input.line_scroll_speed)
    });
    let step = crate::image_preview::PreviewState::ZOOM_STEP;
    let factor = ui.input_mut(|input| {
        let pinch = input.multi_touch().is_some()
            || input
                .events
                .iter()
                .any(|event| matches!(event, egui::Event::Zoom(_)));
        let mut factor = input.zoom_delta();
        if !pinch {
            // Without a pinch the zoom came from Ctrl/Cmd+wheel (Windows
            // touchpad pinches included), to which egui applies its own curve,
            // exp(scroll_zoom_speed * points). It keeps the modifiers from the
            // start of the gesture, so this checks the source, not the keys.
            factor = factor.powf(step.ln() / (zoom_speed * line_speed));
        }
        // Shift and Alt turn the wheel sideways; Ctrl pressed during a plain
        // notch must not hand the rest of it to the scroll area.
        if !trackpad
            && !input.modifiers.shift
            && !input.modifiers.alt
            && input.smooth_scroll_delta.y != 0.0
        {
            factor *= step.powf(input.smooth_scroll_delta.y / line_speed);
            input.smooth_scroll_delta.y = 0.0;
        }
        factor
    });
    (factor != 1.0).then_some((factor, anchor - area.min))
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
