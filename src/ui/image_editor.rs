//! Crops and turns a staged picture over the whole window, before it is sent.

use egui::{Align, Color32, Layout, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2, pos2, vec2};

use super::widgets;
use crate::app::{App, Pending};
use crate::model::{Action, CropEdge, PictureCrop, PictureSource};
use crate::theme::{self, Icon};

/// Room left around the cropper, in points.
const MARGIN: f32 = 28.0;
/// Height of the row of controls above and below the picture.
const BAR: f32 = 44.0;
/// Side of a corner or edge handle, in points.
const HANDLE: f32 = 13.0;
/// How far around a handle a drag still counts as gripping it.
const GRIP: f32 = 22.0;
/// The crop's border, in points.
const BORDER: f32 = 2.0;

/// A change asked for by the controls under the picture. It is recorded while
/// the window is drawn and applied afterwards, so the edit is not consumed
/// from inside the closure that borrows it.
#[derive(Clone, Copy)]
enum Change {
    Reset,
    Turn { clockwise: bool },
}

/// Draws the cropper over the whole window and records whatever its controls
/// asked for. The staged picture is left alone until the crop is kept.
pub fn show(app: &mut App, ctx: &egui::Context) {
    let Some(mut edit) = app.picture_edit.clone() else {
        return;
    };
    let palette = app.palette;
    let locale = app.locale;
    // Captured before the edit is borrowed mutably, so the closure does not
    // need the whole app.
    let source = edit.source.clone();
    let pasted = pasted_texture(app, ctx, &edit);
    let screen = ctx.content_rect();
    let mut actions = Vec::new();
    let mut change = None;
    egui::Area::new(egui::Id::new("picture-editor"))
        .order(egui::Order::Foreground)
        .fixed_pos(screen.min)
        .show(ctx, |ui| {
            let (rect, _) = ui.allocate_exact_size(screen.size(), Sense::hover());
            ui.painter()
                .rect_filled(rect, 0.0, Color32::from_black_alpha(238));
            let room = rect.shrink(MARGIN);
            let top = Rect::from_min_size(room.min, vec2(room.width(), BAR));
            let bottom =
                Rect::from_min_size(pos2(room.min.x, room.max.y - BAR), vec2(room.width(), BAR));
            let (wide, tall) = edit.displayed_size();
            let inner =
                Rect::from_min_max(pos2(room.min.x, top.max.y), pos2(room.max.x, bottom.min.y));
            let picture = super::video_preview::fitted(vec2(wide as f32, tall as f32), inner);
            let scale = picture.width() / wide.max(1) as f32;
            if let Some(id) = texture(ui, &source, pasted.as_ref(), picture.size()) {
                paint_turned(ui, id, picture, edit.turns);
            }
            // The region is kept in the picture's pixels, so a drag is turned
            // into pixels before it is applied.
            let moving = ui.interact(
                to_screen(edit.crop, picture, scale),
                ui.id().with("crop-inside"),
                Sense::drag(),
            );
            if moving.dragged() {
                let delta = moving.drag_delta() / scale;
                edit.crop = edit.crop.drag(
                    CropEdge::Inside,
                    delta.x.round() as i64,
                    delta.y.round() as i64,
                    wide,
                    tall,
                );
            }
            for (edge, at) in handles(to_screen(edit.crop, picture, scale)) {
                let grip = Rect::from_center_size(at, Vec2::splat(GRIP));
                let response = ui
                    .interact(grip, ui.id().with(("crop-edge", edge as u8)), Sense::drag())
                    .on_hover_cursor(cursor(edge));
                if response.dragged() {
                    let delta = response.drag_delta() / scale;
                    edit.crop = edit.crop.drag(
                        edge,
                        delta.x.round() as i64,
                        delta.y.round() as i64,
                        wide,
                        tall,
                    );
                }
            }
            shade_outside(ui, palette, picture, to_screen(edit.crop, picture, scale));
            ui.scope_builder(
                egui::UiBuilder::new()
                    .max_rect(top)
                    .layout(Layout::right_to_left(Align::Center)),
                |ui| {
                    let close = crate::i18n::gettext(locale, "Cancel (Esc)");
                    if theme::icon_button(
                        ui,
                        Icon::X,
                        18.0,
                        palette.secondary,
                        palette.text,
                        &close,
                    )
                    .clicked()
                    {
                        actions.push(Action::CancelPictureEdit);
                    }
                    let keep = crate::i18n::gettext(locale, "Keep this crop");
                    if theme::icon_button(
                        ui,
                        Icon::Check,
                        18.0,
                        palette.secondary,
                        palette.text,
                        &keep,
                    )
                    .clicked()
                    {
                        actions.push(Action::ApplyPictureEdit);
                    }
                },
            );
            ui.scope_builder(
                egui::UiBuilder::new()
                    .max_rect(bottom)
                    .layout(Layout::right_to_left(Align::Center)),
                |ui| {
                    let reset = crate::i18n::gettext(locale, "Reset");
                    if theme::pill_button(ui, &palette, &reset, false).clicked() {
                        change = Some(Change::Reset);
                    }
                    let right = crate::i18n::gettext(locale, "Turn right");
                    if theme::icon_button(
                        ui,
                        Icon::RotateRight,
                        18.0,
                        palette.secondary,
                        palette.text,
                        &right,
                    )
                    .clicked()
                    {
                        change = Some(Change::Turn { clockwise: true });
                    }
                    let left = crate::i18n::gettext(locale, "Turn left");
                    if theme::icon_button(
                        ui,
                        Icon::RotateLeft,
                        18.0,
                        palette.secondary,
                        palette.text,
                        &left,
                    )
                    .clicked()
                    {
                        change = Some(Change::Turn { clockwise: false });
                    }
                },
            );
        });
    match change {
        Some(Change::Reset) => edit = edit.reset(),
        Some(Change::Turn { clockwise }) => edit = edit.turned(clockwise),
        None => {}
    }
    app.picture_edit = Some(edit);
    app.actions.extend(actions);
}

/// The texture to draw a pasted picture with, built from its buffer when the
/// strip no longer has it. Cropping a pasted picture leaves a file on disk and
/// takes the strip's copy of the pixels with it, so on the second visit the
/// pixels have to come from the buffer the crop was taken from.
fn pasted_texture(
    app: &mut App,
    ctx: &egui::Context,
    edit: &crate::model::PictureEdit,
) -> Option<egui::TextureHandle> {
    if let Some(at) = app.staged_at(edit.target)
        && let Some(Pending::Picture {
            texture: Some(handle),
            ..
        }) = app.pending.get(at).map(|staged| &staged.item)
    {
        return Some(handle.clone());
    }
    let PictureSource::Pasted(rgba) = &edit.source else {
        return None;
    };
    let key = std::sync::Arc::as_ptr(rgba) as usize;
    if let Some((cached, handle)) = &app.picture_texture
        && *cached == key
    {
        return Some(handle.clone());
    }
    let image = pasted_image(rgba, edit.width, edit.height)?;
    let handle = ctx.load_texture(
        format!("edited-picture-{key}"),
        image,
        egui::TextureOptions::LINEAR,
    );
    app.picture_texture = Some((key, handle.clone()));
    Some(handle)
}

/// A pasted buffer as an image to upload, capped the way the strip caps its own
/// copy so a large paste does not meet the GPU's texture limit here.
fn pasted_image(rgba: &[u8], width: u32, height: u32) -> Option<egui::ColorImage> {
    let full = image::RgbaImage::from_raw(width.max(1), height.max(1), rgba.to_vec())?;
    let small = if width > 1024 || height > 1024 {
        let scale = 1024.0 / width.max(height) as f32;
        let (wide, tall) = (
            ((width as f32 * scale) as u32).max(1),
            ((height as f32 * scale) as u32).max(1),
        );
        image::imageops::resize(&full, wide, tall, image::imageops::FilterType::Triangle)
    } else {
        full
    };
    Some(egui::ColorImage::from_rgba_unmultiplied(
        [small.width() as usize, small.height() as usize],
        &small,
    ))
}

/// The picture's texture, once it is ready to draw.
fn texture(
    ui: &mut egui::Ui,
    source: &PictureSource,
    pasted: Option<&egui::TextureHandle>,
    size: Vec2,
) -> Option<egui::TextureId> {
    match source {
        PictureSource::File(path) => {
            let image = widgets::file_image(ui, path);
            match image.load_for_size(ui.ctx(), size) {
                Ok(egui::load::TexturePoll::Ready { texture }) => Some(texture.id),
                _ => None,
            }
        }
        PictureSource::Pasted(_) => pasted.map(egui::TextureHandle::id),
    }
}

/// Which corner of the texture each corner of the picture takes, turned by
/// whole quarter turns clockwise. Read as the picture's own corners, going
/// clockwise from the top left.
fn turned_corners(turns: u8) -> [Pos2; 4] {
    let quarter = usize::from(turns % 4);
    let corners = [
        Pos2::new(0.0, 0.0),
        Pos2::new(1.0, 0.0),
        Pos2::new(1.0, 1.0),
        Pos2::new(0.0, 1.0),
    ];
    [
        corners[(4 - quarter) % 4],
        corners[(5 - quarter) % 4],
        corners[(6 - quarter) % 4],
        corners[(7 - quarter) % 4],
    ]
}

/// Draws the picture into `rect`, turned by whole quarter turns clockwise.
/// The turn is taken out of the texture's own corners, so the pixels are
/// never copied and turning a picture costs nothing.
fn paint_turned(ui: &egui::Ui, texture: egui::TextureId, rect: Rect, turns: u8) {
    let places = [
        rect.left_top(),
        rect.right_top(),
        rect.right_bottom(),
        rect.left_bottom(),
    ];
    let mut mesh = egui::epaint::Mesh {
        texture_id: texture,
        ..Default::default()
    };
    for (place, uv) in places.into_iter().zip(turned_corners(turns)) {
        mesh.vertices.push(egui::epaint::Vertex {
            pos: place,
            uv,
            color: Color32::WHITE,
        });
    }
    mesh.indices.extend_from_slice(&[0, 1, 2, 0, 2, 3]);
    ui.painter().add(egui::Shape::mesh(mesh));
}

/// Where a crop's edges and corners are on screen.
fn handles(crop: Rect) -> [(CropEdge, Pos2); 8] {
    [
        (CropEdge::TopLeft, crop.left_top()),
        (CropEdge::Top, crop.center_top()),
        (CropEdge::TopRight, crop.right_top()),
        (CropEdge::Right, crop.right_center()),
        (CropEdge::BottomRight, crop.right_bottom()),
        (CropEdge::Bottom, crop.center_bottom()),
        (CropEdge::BottomLeft, crop.left_bottom()),
        (CropEdge::Left, crop.left_center()),
    ]
}

/// The crop's place on screen, given where the picture is drawn.
fn to_screen(crop: PictureCrop, picture: Rect, scale: f32) -> Rect {
    Rect::from_min_size(
        picture.min + vec2(crop.x as f32 * scale, crop.y as f32 * scale),
        vec2(crop.width as f32 * scale, crop.height as f32 * scale),
    )
}

/// The cursor that fits the edge being dragged.
fn cursor(edge: CropEdge) -> egui::CursorIcon {
    match edge {
        CropEdge::Left | CropEdge::Right => egui::CursorIcon::ResizeHorizontal,
        CropEdge::Top | CropEdge::Bottom => egui::CursorIcon::ResizeVertical,
        CropEdge::TopLeft | CropEdge::BottomRight => egui::CursorIcon::ResizeNwSe,
        CropEdge::TopRight | CropEdge::BottomLeft => egui::CursorIcon::ResizeNeSw,
        CropEdge::Inside => egui::CursorIcon::Grab,
    }
}

/// Dims what the crop leaves out, and marks its border and handles.
fn shade_outside(ui: &egui::Ui, palette: theme::Palette, picture: Rect, crop: Rect) {
    let shade = palette.shadow.gamma_multiply(1.6);
    for outside in [
        Rect::from_min_max(picture.min, pos2(picture.max.x, crop.min.y)),
        Rect::from_min_max(pos2(picture.min.x, crop.max.y), picture.max),
        Rect::from_min_max(
            pos2(picture.min.x, crop.min.y),
            pos2(crop.min.x, crop.max.y),
        ),
        Rect::from_min_max(
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
        Stroke::new(BORDER, Color32::WHITE),
        StrokeKind::Inside,
    );
    for (_, at) in handles(crop) {
        ui.painter().rect_filled(
            Rect::from_center_size(at, Vec2::splat(HANDLE)),
            2.0,
            Color32::WHITE,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_quarter_turn_clockwise_puts_the_top_left_at_the_top_right() {
        assert_eq!(
            turned_corners(1),
            [
                Pos2::new(0.0, 1.0),
                Pos2::new(0.0, 0.0),
                Pos2::new(1.0, 0.0),
                Pos2::new(1.0, 1.0),
            ]
        );
    }

    #[test]
    fn turning_takes_whole_quarters_and_comes_back_round() {
        assert_eq!(
            turned_corners(0),
            [
                Pos2::new(0.0, 0.0),
                Pos2::new(1.0, 0.0),
                Pos2::new(1.0, 1.0),
                Pos2::new(0.0, 1.0),
            ]
        );
        assert_eq!(turned_corners(4), turned_corners(0));
        assert_eq!(turned_corners(7), turned_corners(3));
        // Half a turn puts each corner where the one across from it was.
        assert_eq!(turned_corners(2)[0], Pos2::new(1.0, 1.0));
        assert_eq!(turned_corners(2)[1], Pos2::new(0.0, 1.0));
    }
}
