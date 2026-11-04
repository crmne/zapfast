//! Draws on, crops and turns a staged picture over the whole window, before
//! it is sent.

use egui::{Align, Color32, Layout, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2, pos2, vec2};

use super::widgets;
use crate::app::{App, Pending};
use crate::model::{
    Action, Brush, CropEdge, Ink, Mark, PictureCrop, PictureEdit, PictureSource, PictureTool,
    Point, Thickness,
};
use crate::theme::{self, Icon};

/// Room left around the cropper, in points.
const MARGIN: f32 = 28.0;
/// The room left in a narrow window, where every point counts.
const NARROW_MARGIN: f32 = 12.0;
/// Height of the row of controls above and below the picture.
const BAR: f32 = 44.0;
/// The narrowest room that still takes the inks and the bottom row's
/// buttons on one row; narrower, they stack.
const ONE_ROW: f32 = 600.0;
/// Side of a corner or edge handle, in points.
const HANDLE: f32 = 13.0;
/// How far around a handle a drag still counts as gripping it.
const GRIP: f32 = 22.0;
/// The crop's border, in points.
const BORDER: f32 = 2.0;
/// The round button that keeps the edit, like the composer's send button.
const KEEP: f32 = 36.0;
/// Side of a tool, ink or thickness choice.
const CHOICE: f32 = 30.0;
/// How far the pointer moves, in points, before a pen takes another point.
const PEN_STEP: f32 = 1.5;

/// A change asked for by the controls under the picture. It is recorded while
/// the window is drawn and applied afterwards, so the edit is not consumed
/// from inside the closure that borrows it.
#[derive(Clone, Copy)]
enum Change {
    Reset,
    Turn { clockwise: bool },
    Undo,
    Redo,
}

/// The cropper's layer.
pub(crate) fn area_id() -> egui::Id {
    egui::Id::new("picture-editor")
}

/// Where the editor notes its "Keep changes" button, which focus enters on
/// when the keys or a screen reader opened it.
pub(crate) fn keep_note_id() -> egui::Id {
    egui::Id::new("picture-editor-keep")
}

/// Where the picture is drawn, to carry points between the screen and the
/// source's own pixels.
#[derive(Clone, Copy, Debug)]
struct Placement {
    /// The turned picture on screen.
    picture: Rect,
    /// Points per pixel.
    scale: f32,
    turns: u8,
    /// The source's own size, before any turn.
    width: u32,
    height: u32,
}

impl Placement {
    fn new(picture: Rect, edit: &PictureEdit) -> Self {
        let (wide, _) = edit.displayed_size();
        Self {
            picture,
            scale: picture.width() / wide.max(1) as f32,
            turns: edit.turns,
            width: edit.width,
            height: edit.height,
        }
    }

    /// The source's pixel under a point on screen.
    fn to_source(self, at: Pos2) -> Point {
        let shown = (at - self.picture.min) / self.scale;
        crate::model::unturned_point([shown.x, shown.y], self.turns, self.width, self.height)
    }

    /// Where a pixel of the source is on screen.
    fn to_screen(self, at: Point) -> Pos2 {
        let [x, y] = crate::model::turned_point(at, self.turns, self.width, self.height);
        self.picture.min + vec2(x, y) * self.scale
    }
}

/// Draws the editor over the whole window and records whatever its controls
/// asked for. The staged picture is left alone until the edit is kept.
pub fn show(app: &mut App, ctx: &egui::Context) {
    let Some(mut edit) = app.picture_edit.clone() else {
        return;
    };
    let palette = app.palette;
    let locale = app.locale;
    let mut brush = app.picture_brush;
    // Captured before the edit is borrowed mutably, so the closure does not
    // need the whole app.
    let source = edit.source.clone();
    let pasted = pasted_texture(app, ctx, &edit);
    let screen = ctx.content_rect();
    let mut actions = Vec::new();
    let mut change = None;
    // It covers the window, so it keeps the keyboard as the viewer does:
    // Tab walks its own controls, never the chat hidden under it. The
    // pointer already met only the cropper.
    ctx.memory_mut(|memory| {
        memory.set_modal_layer(egui::LayerId::new(egui::Order::Foreground, area_id()));
    });
    egui::Area::new(area_id())
        .order(egui::Order::Foreground)
        .fixed_pos(screen.min)
        .show(ctx, |ui| {
            let (rect, _) = ui.allocate_exact_size(screen.size(), Sense::hover());
            // The window takes the theme's own colour, so the controls on it
            // read in a light theme as in a dark one.
            ui.painter().rect_filled(rect, 0.0, palette.window);
            let narrow = rect.width() - 2.0 * MARGIN < ONE_ROW;
            let room = rect.shrink(if narrow { NARROW_MARGIN } else { MARGIN });
            let rows = if narrow { 2.0 } else { 1.0 };
            let top = Rect::from_min_size(room.min, vec2(room.width(), BAR));
            let bottom = Rect::from_min_max(pos2(room.min.x, room.max.y - BAR * rows), room.max);
            let (wide, tall) = edit.displayed_size();
            let inner =
                Rect::from_min_max(pos2(room.min.x, top.max.y), pos2(room.max.x, bottom.min.y));
            let picture = fitted(vec2(wide as f32, tall as f32), inner);
            let place = Placement::new(picture, &edit);
            let scale = place.scale;
            let texture = texture(ui, &source, pasted.as_ref(), picture.size());
            if let Some(id) = texture {
                paint_turned(ui, id, picture, edit.turns);
            }
            let painter = ui.painter().with_clip_rect(picture);
            for mark in edit.marks.iter().chain(&edit.drawing) {
                paint_mark(&painter, mark, place, texture, palette);
            }
            if brush.tool == PictureTool::Crop {
                crop_handles(ui, &mut edit, picture, scale);
            } else {
                draw(ui, &mut edit, brush, place);
            }
            shade_outside(
                ui,
                palette,
                picture,
                to_screen(edit.crop, picture, scale),
                brush.tool == PictureTool::Crop,
            );
            ui.scope_builder(
                egui::UiBuilder::new()
                    .max_rect(top)
                    .layout(Layout::left_to_right(Align::Center)),
                |ui| tools(ui, palette, locale, &mut brush),
            );
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
                    let keep = crate::i18n::gettext(locale, "Keep changes");
                    let kept = theme::circle_button(
                        ui,
                        Icon::Check,
                        KEEP,
                        palette.accent,
                        palette.accent_hover,
                        palette.on_accent,
                        &keep,
                    );
                    ui.ctx()
                        .data_mut(|data| data.insert_temp(keep_note_id(), kept.id));
                    if kept.clicked() {
                        actions.push(Action::ApplyPictureEdit);
                    }
                },
            );
            let (inks_row, buttons_row) = if narrow {
                bottom.split_top_bottom_at_y(bottom.min.y + BAR)
            } else {
                (bottom, bottom)
            };
            if brush.tool.inked() {
                ui.scope_builder(
                    egui::UiBuilder::new()
                        .max_rect(inks_row)
                        .layout(Layout::left_to_right(Align::Center)),
                    |ui| inks(ui, palette, locale, &mut brush),
                );
            }
            ui.scope_builder(
                egui::UiBuilder::new()
                    .max_rect(buttons_row)
                    .layout(Layout::right_to_left(Align::Center)),
                |ui| {
                    let reset = crate::i18n::gettext(locale, "Reset");
                    if theme::pill_button(ui, &palette, &reset, false).clicked() {
                        change = Some(Change::Reset);
                    }
                    for (icon, label, step) in [
                        (
                            Icon::RotateRight,
                            crate::i18n::gettext(locale, "Turn right"),
                            Change::Turn { clockwise: true },
                        ),
                        (
                            Icon::RotateLeft,
                            crate::i18n::gettext(locale, "Turn left"),
                            Change::Turn { clockwise: false },
                        ),
                    ] {
                        if theme::icon_button(
                            ui,
                            icon,
                            18.0,
                            palette.secondary,
                            palette.text,
                            &label,
                        )
                        .clicked()
                        {
                            change = Some(step);
                        }
                    }
                    ui.add_space(8.0);
                    for (icon, label, step, possible) in [
                        (
                            Icon::Redo,
                            crate::i18n::gettext(locale, "Redo"),
                            Change::Redo,
                            !edit.undone.is_empty(),
                        ),
                        (
                            Icon::Undo,
                            crate::i18n::gettext(locale, "Undo"),
                            Change::Undo,
                            !edit.marks.is_empty(),
                        ),
                    ] {
                        let pressed = ui
                            .add_enabled_ui(possible, |ui| {
                                theme::icon_button(
                                    ui,
                                    icon,
                                    18.0,
                                    if possible {
                                        palette.secondary
                                    } else {
                                        palette.dim
                                    },
                                    palette.text,
                                    &label,
                                )
                            })
                            .inner
                            .clicked();
                        if pressed {
                            change = Some(step);
                        }
                    }
                },
            );
        });
    match change {
        Some(Change::Reset) => edit = edit.reset(),
        Some(Change::Turn { clockwise }) => edit = edit.turned(clockwise),
        Some(Change::Undo) => edit.undo(),
        Some(Change::Redo) => edit.redo(),
        None => {}
    }
    app.picture_edit = Some(edit);
    app.picture_brush = brush;
    app.actions.extend(actions);
}

/// The crop's handles and the region itself, which drag the crop.
fn crop_handles(ui: &mut egui::Ui, edit: &mut PictureEdit, picture: Rect, scale: f32) {
    let (wide, tall) = edit.displayed_size();
    // The region is kept in the picture's pixels, so a drag is turned
    // into pixels before it is applied.
    let moving = ui.interact(
        to_screen(edit.crop, picture, scale),
        ui.id().with("crop-inside"),
        // `Sense::DRAG` is not focusable: a drag needs the pointer,
        // and a focusable area is a nameless Tab stop.
        Sense::DRAG,
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
            .interact(grip, ui.id().with(("crop-edge", edge as u8)), Sense::DRAG)
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
}

/// A drag over the picture draws a mark with the brush.
fn draw(ui: &mut egui::Ui, edit: &mut PictureEdit, brush: Brush, place: Placement) {
    // `Sense::DRAG` alone starts the drag on the press, so a click with the
    // pen leaves a dot, and it is no Tab stop.
    let response = ui
        .interact(place.picture, ui.id().with("draw"), Sense::DRAG)
        .on_hover_cursor(egui::CursorIcon::Crosshair);
    if response.drag_started()
        && let Some(at) = ui.input(|input| input.pointer.press_origin())
    {
        edit.begin_mark(brush, place.to_source(at));
    }
    if response.dragged()
        && let Some(at) = response.interact_pointer_pos()
    {
        edit.extend_mark(place.to_source(at), PEN_STEP / place.scale);
    }
    if response.drag_stopped() {
        edit.finish_mark();
    }
}

/// The colour a mark is drawn in on screen.
fn colour(rgba: [u8; 4]) -> Color32 {
    let [r, g, b, a] = rgba;
    Color32::from_rgba_unmultiplied(r, g, b, a)
}

/// Draws a mark as it will be written. Round ends are discs, as the written
/// lines have round caps.
fn paint_mark(
    painter: &egui::Painter,
    mark: &Mark,
    place: Placement,
    texture: Option<egui::TextureId>,
    palette: theme::Palette,
) {
    let screen = |points: &[Point]| -> Vec<Pos2> {
        points.iter().map(|point| place.to_screen(*point)).collect()
    };
    let line = |width: f32| width * place.scale;
    match mark {
        Mark::Pen { points, width, .. } | Mark::Highlighter { points, width, .. } => {
            let fill = colour(mark.rgba().unwrap_or_default());
            let points = screen(points);
            let round = matches!(mark, Mark::Pen { .. });
            match points.as_slice() {
                [] => {}
                [dot] => {
                    painter.circle_filled(*dot, line(*width) / 2.0, fill);
                }
                [first, .., last] => {
                    if round {
                        painter.circle_filled(*first, line(*width) / 2.0, fill);
                        painter.circle_filled(*last, line(*width) / 2.0, fill);
                    }
                    painter.add(egui::Shape::line(
                        points.clone(),
                        Stroke::new(line(*width), fill),
                    ));
                }
            }
        }
        Mark::Arrow {
            from, to, width, ..
        } => {
            let fill = colour(mark.rgba().unwrap_or_default());
            let stroke = Stroke::new(line(*width), fill);
            let [left, right] = crate::model::arrow_barbs(*from, *to, *width);
            let [from, to, left, right] = [*from, *to, left, right].map(|at| place.to_screen(at));
            painter.line_segment([from, to], stroke);
            painter.add(egui::Shape::line(vec![left, to, right], stroke));
            for end in [from, to, left, right] {
                painter.circle_filled(end, line(*width) / 2.0, fill);
            }
        }
        Mark::Rectangle {
            from, to, width, ..
        } => {
            let fill = colour(mark.rgba().unwrap_or_default());
            painter.rect_stroke(
                Rect::from_two_pos(place.to_screen(*from), place.to_screen(*to)),
                0.0,
                Stroke::new(line(*width), fill),
                StrokeKind::Middle,
            );
        }
        Mark::Pixelate { from, to } => {
            // Each block is a quad whose corners all sample the texture at
            // the block's centre, so the GPU paints it in one colour without
            // the pixels coming back to the CPU.
            let cells = crate::model::pixel_cells(*from, *to, place.width, place.height);
            let (width, height) = (place.width as f32, place.height as f32);
            let mut mesh = egui::epaint::Mesh {
                texture_id: texture.unwrap_or_default(),
                ..Default::default()
            };
            for [left, top, right, bottom] in cells {
                let [left, top, right, bottom] = [left, top, right, bottom].map(|edge| edge as f32);
                let centre = pos2((left + right) / 2.0 / width, (top + bottom) / 2.0 / height);
                let base = mesh.vertices.len() as u32;
                for corner in [[left, top], [right, top], [right, bottom], [left, bottom]] {
                    mesh.vertices.push(egui::epaint::Vertex {
                        pos: place.to_screen(corner),
                        uv: if texture.is_some() {
                            centre
                        } else {
                            egui::epaint::WHITE_UV
                        },
                        color: if texture.is_some() {
                            Color32::WHITE
                        } else {
                            palette.surface
                        },
                    });
                }
                mesh.indices.extend_from_slice(&[
                    base,
                    base + 1,
                    base + 2,
                    base,
                    base + 2,
                    base + 3,
                ]);
            }
            painter.add(egui::Shape::mesh(mesh));
        }
    }
}

/// A tool, ink or thickness to pick: lit while it is the one in use.
fn choice(
    ui: &mut egui::Ui,
    palette: theme::Palette,
    label: &str,
    picked: bool,
    paint: impl FnOnce(&egui::Ui, Rect, Color32),
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(CHOICE), Sense::click());
    theme::reveal_focus(&response);
    theme::focus_outline(ui, response.id, rect, 8.0);
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Button, ui.is_enabled(), picked, label)
    });
    if ui.is_rect_visible(rect) {
        if picked {
            ui.painter().rect_filled(rect, 8.0, palette.surface_active);
        } else if response.hovered() {
            ui.painter().rect_filled(rect, 8.0, palette.surface_hover);
        }
        let tint = if picked || response.hovered() || response.has_focus() {
            palette.text
        } else {
            palette.secondary
        };
        paint(ui, rect, tint);
    }
    #[cfg(any(test, feature = "demo"))]
    crate::ui::focus::name_button(ui.ctx(), label, response.id);
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(label)
}

/// The tools along the top: crop, pen, highlighter, arrow, rectangle and
/// pixelate.
fn tools(
    ui: &mut egui::Ui,
    palette: theme::Palette,
    locale: crate::i18n::Locale,
    brush: &mut Brush,
) {
    ui.spacing_mut().item_spacing.x = 2.0;
    for tool in PictureTool::ALL {
        let (icon, label) = match tool {
            PictureTool::Crop => (Icon::Crop, crate::i18n::gettext(locale, "Crop")),
            PictureTool::Pen => (Icon::Pencil, crate::i18n::gettext(locale, "Pen")),
            PictureTool::Highlighter => (
                Icon::Highlighter,
                crate::i18n::gettext(locale, "Highlighter"),
            ),
            PictureTool::Arrow => (Icon::ArrowUpRight, crate::i18n::gettext(locale, "Arrow")),
            PictureTool::Rectangle => (Icon::Square, crate::i18n::gettext(locale, "Rectangle")),
            PictureTool::Pixelate => (Icon::Grid, crate::i18n::gettext(locale, "Pixelate")),
        };
        let picked = brush.tool == tool;
        let response = choice(ui, palette, &label, picked, |ui, rect, tint| {
            let size = 18.0;
            let at = Rect::from_center_size(rect.center(), Vec2::splat(size));
            icon.image(tint, size).paint_at(ui, at);
        });
        if response.clicked() {
            brush.tool = tool;
        }
    }
}

/// The inks and the three thicknesses, for the tools that draw in them.
fn inks(
    ui: &mut egui::Ui,
    palette: theme::Palette,
    locale: crate::i18n::Locale,
    brush: &mut Brush,
) {
    ui.spacing_mut().item_spacing.x = 0.0;
    for ink in Ink::ALL {
        let label = match ink {
            Ink::Red => crate::i18n::gettext(locale, "Red"),
            Ink::Yellow => crate::i18n::gettext(locale, "Yellow"),
            Ink::Green => crate::i18n::gettext(locale, "Green"),
            Ink::Blue => crate::i18n::gettext(locale, "Blue"),
            Ink::Purple => crate::i18n::gettext(locale, "Purple"),
            Ink::White => crate::i18n::gettext(locale, "White"),
            Ink::Black => crate::i18n::gettext(locale, "Black"),
        };
        let [r, g, b] = ink.rgb();
        let response = choice(ui, palette, &label, brush.ink == ink, |ui, rect, _| {
            // A faint ring keeps white on a light theme, and black on a
            // dark one, from melting into the window.
            ui.painter().circle(
                rect.center(),
                8.0,
                Color32::from_rgb(r, g, b),
                Stroke::new(1.0, palette.secondary.gamma_multiply(0.6)),
            );
        });
        if response.clicked() {
            brush.ink = ink;
        }
    }
    ui.add_space(10.0);
    let [r, g, b] = brush.ink.rgb();
    for (thickness, label, radius) in [
        (
            Thickness::Thin,
            crate::i18n::gettext(locale, "Thin line"),
            2.5,
        ),
        (
            Thickness::Medium,
            crate::i18n::gettext(locale, "Medium line"),
            4.0,
        ),
        (
            Thickness::Thick,
            crate::i18n::gettext(locale, "Thick line"),
            6.0,
        ),
    ] {
        let picked = brush.thickness == thickness;
        let response = choice(ui, palette, &label, picked, |ui, rect, tint| {
            let fill = if picked {
                Color32::from_rgb(r, g, b)
            } else {
                tint
            };
            ui.painter().circle(
                rect.center(),
                radius,
                fill,
                Stroke::new(1.0, palette.outline),
            );
        });
        if response.clicked() {
            brush.thickness = thickness;
        }
    }
}
/// The largest rectangle of `shape`'s proportions centred in `room`.
fn fitted(shape: Vec2, room: Rect) -> Rect {
    let scale = (room.width() / shape.x.max(1.0)).min(room.height() / shape.y.max(1.0));
    Rect::from_center_size(room.center(), shape * scale)
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

/// Fades what the crop leaves out toward the window, and, while the crop
/// tool is in use, marks the crop's border and handles in the theme's accent.
fn shade_outside(
    ui: &egui::Ui,
    palette: theme::Palette,
    picture: Rect,
    crop: Rect,
    cropping: bool,
) {
    let shade = palette.window.gamma_multiply(0.7);
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
    if !cropping {
        return;
    }
    ui.painter().rect_stroke(
        crop,
        0.0,
        Stroke::new(BORDER, palette.accent),
        StrokeKind::Inside,
    );
    for (_, at) in handles(crop) {
        let handle = Rect::from_center_size(at, Vec2::splat(HANDLE));
        ui.painter().rect_filled(handle, 2.0, palette.accent);
        ui.painter().rect_stroke(
            handle,
            2.0,
            Stroke::new(1.0, palette.on_accent),
            StrokeKind::Inside,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_picture_fills_the_room_in_one_direction() {
        let room = Rect::from_min_size(egui::pos2(10.0, 10.0), vec2(1000.0, 500.0));
        let wide = fitted(vec2(1920.0, 1080.0), room);
        assert!((wide.height() - 500.0).abs() < 0.01);
        assert!((wide.width() - 500.0 * 16.0 / 9.0).abs() < 0.01);
        assert_eq!(wide.center(), room.center());
        let tall = fitted(vec2(100.0, 1000.0), room);
        assert!((tall.height() - 500.0).abs() < 0.01 && tall.width() < 60.0);
    }

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

    /// A picture 400 by 200, drawn at half size from (10, 20).
    fn placed(turns: u8) -> Placement {
        let mut edit = PictureEdit::new(
            0,
            PictureSource::File(std::path::PathBuf::from("photo.png")),
            400,
            200,
        );
        for _ in 0..turns {
            edit = edit.turned(true);
        }
        let (wide, tall) = edit.displayed_size();
        let picture = Rect::from_min_size(pos2(10.0, 20.0), vec2(wide as f32, tall as f32) * 0.5);
        Placement::new(picture, &edit)
    }

    #[test]
    fn a_point_on_screen_names_the_sources_pixel_under_it() {
        // Unturned: half size, from the picture's corner.
        assert_eq!(placed(0).to_source(pos2(60.0, 45.0)), [100.0, 50.0]);
        // Turned a quarter clockwise, the source's top-left corner sits at
        // the top right of the picture on screen, which is 100 wide.
        let turned = placed(1);
        assert_eq!(turned.to_screen([0.0, 0.0]), pos2(110.0, 20.0));
        assert_eq!(turned.to_source(pos2(110.0, 20.0)), [0.0, 0.0]);
        assert_eq!(turned.to_source(pos2(10.0, 220.0)), [400.0, 200.0]);
        for turns in 0..4 {
            let place = placed(turns);
            let point = [123.0, 45.0];
            let back = place.to_source(place.to_screen(point));
            assert!(
                (back[0] - point[0]).abs() < 1e-3 && (back[1] - point[1]).abs() < 1e-3,
                "{turns}: {back:?}"
            );
        }
    }
}
