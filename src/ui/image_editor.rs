//! Crops and turns a staged picture over the whole window, before it is sent.

use egui::{Align, Color32, Layout, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2, pos2, vec2};

use super::widgets;
use crate::app::{App, Pending};
use crate::model::{
    Action, CropEdge, PictureCrop, PictureMark, PictureShape, PictureSource, PictureStamp,
};
use crate::theme::{self, Icon};

/// Room left around the picture, in points.
const MARGIN: f32 = 18.0;
/// Height of the row of controls above and below the picture.
const BAR: f32 = 58.0;
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
    Undo,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tool {
    Crop,
    Pen,
    Shapes,
    Text,
    Blur,
    Emoji,
}

#[derive(Clone, Copy)]
struct EditorTool {
    kind: Tool,
    color: usize,
    width: f32,
    width_open: bool,
    shape: PictureShape,
    filled: bool,
    edit_shapes: bool,
    background: bool,
}

#[derive(Clone, Copy)]
struct ShapeDrag {
    index: usize,
    start: (f32, f32),
    from: (f32, f32),
    to: (f32, f32),
    move_whole: bool,
}

#[derive(Clone, Copy)]
struct TextDrag {
    index: usize,
    start: (f32, f32),
    at: (f32, f32),
}

#[derive(Clone, Copy)]
struct EmojiDrag {
    index: usize,
    start: (f32, f32),
    at: (f32, f32),
    size: f32,
    resize: bool,
}

#[derive(Clone)]
struct EditorHistory {
    target: crate::app::AttachmentId,
    stack: Vec<crate::model::PictureEdit>,
}

fn remember(history: &mut EditorHistory, edit: &crate::model::PictureEdit) {
    if history.stack.len() == 32 {
        history.stack.remove(0);
    }
    history.stack.push(edit.clone());
}

const COLORS: [[u8; 3]; 8] = [
    [56, 58, 61],
    [155, 160, 168],
    [255, 255, 255],
    [40, 198, 238],
    [68, 218, 70],
    [178, 97, 230],
    [255, 151, 34],
    [246, 70, 80],
];

/// Keep the pen's apparent diameter in editor points at any source resolution.
fn stroke_radius(width_points: f32, image_scale: f32) -> f32 {
    width_points / (2.0 * image_scale.max(0.001))
}

/// The cropper's layer.
pub(crate) fn area_id() -> egui::Id {
    egui::Id::new("picture-editor")
}

/// Where the cropper notes its "Keep this crop" button, which focus enters
/// on when the keys or a screen reader opened it.
pub(crate) fn keep_note_id() -> egui::Id {
    egui::Id::new("picture-editor-keep")
}

/// Draws the cropper over the whole window and records whatever its controls
/// asked for. The staged picture is left alone until the crop is kept.
pub fn show(app: &mut App, ctx: &egui::Context) {
    if ctx.data_mut(|data| {
        let id = area_id().with("embedded-this-frame");
        let embedded = data.get_temp::<bool>(id).unwrap_or(false);
        data.remove::<bool>(id);
        embedded
    }) {
        return;
    }
    ctx.data_mut(|data| data.remove::<egui::LayerId>(area_id().with("embedded-layer")));
    show_impl(app, ctx, None);
}

/// Draw the editor in the attachment review's own layout.
pub fn show_in_ui(app: &mut App, ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    ctx.data_mut(|data| {
        data.insert_temp(area_id().with("embedded-this-frame"), true);
        data.insert_temp(area_id().with("embedded-layer"), ui.layer_id());
    });
    show_impl(app, &ctx, Some(ui));
}

fn show_impl(app: &mut App, ctx: &egui::Context, parent: Option<&mut egui::Ui>) {
    let Some(mut edit) = app.picture_edit.clone() else {
        return;
    };
    let palette = app.palette;
    let locale = app.locale;
    let recent_emoji = app.settings.recent_emoji.clone();
    // Captured before the edit is borrowed mutably, so the closure does not
    // need the whole app.
    let source = edit.source.clone();
    let pasted = pasted_texture(app, ctx, &edit);
    let screen = parent
        .as_ref()
        .map_or_else(|| ctx.content_rect(), |ui| ui.available_rect_before_wrap());
    let mut actions = Vec::new();
    let mut change = None;
    let tool_id = area_id().with("tool");
    let history_id = area_id().with("history");
    let mut history = ctx
        .data(|data| data.get_temp::<EditorHistory>(history_id))
        .filter(|history| history.target == edit.target)
        .unwrap_or(EditorHistory {
            target: edit.target,
            stack: Vec::new(),
        });
    let mut tool = ctx
        .data(|data| data.get_temp::<EditorTool>(tool_id))
        .unwrap_or(EditorTool {
            kind: Tool::Crop,
            color: 7,
            width: 2.5,
            width_open: false,
            shape: PictureShape::Rectangle,
            filled: false,
            edit_shapes: false,
            background: false,
        });
    // It covers the window, so it keeps the keyboard as the viewer does:
    // Tab walks its own controls, never the chat hidden under it. The
    // pointer already met only the cropper.
    if parent.is_none() {
        ctx.memory_mut(|memory| {
            memory.set_modal_layer(egui::LayerId::new(egui::Order::Foreground, area_id()));
        });
    }
    let draw = |ui: &mut egui::Ui| {
        let mut width_anchor = None;
        let (rect, _) = ui.allocate_exact_size(screen.size(), Sense::hover());
        // The window takes the theme's own colour, so the controls on it
        // read in a light theme as in a dark one.
        ui.painter().rect_filled(rect, 0.0, palette.window);
        let room = rect.shrink(MARGIN);
        let top = Rect::from_min_size(room.min, vec2(room.width(), BAR));
        let bottom =
            Rect::from_min_size(pos2(room.min.x, room.max.y - BAR), vec2(room.width(), BAR));
        ui.painter().rect_filled(
            Rect::from_min_max(rect.min, pos2(rect.max.x, top.max.y + MARGIN)),
            0.0,
            palette.surface,
        );
        ui.painter().rect_filled(
            Rect::from_min_max(pos2(rect.min.x, bottom.min.y - MARGIN), rect.max),
            0.0,
            palette.surface,
        );
        let (wide, tall) = edit.displayed_size();
        let inner = Rect::from_min_max(
            pos2(room.min.x, top.max.y + MARGIN + 8.0),
            pos2(room.max.x, bottom.min.y - MARGIN - 8.0),
        );
        let picture = fitted(vec2(wide as f32, tall as f32), inner);
        let scale = picture.width() / wide.max(1) as f32;
        ui.painter()
            .rect_filled(picture.expand(6.0), 0.0, palette.surface_hover);
        let texture_id = texture(ui, &source, pasted.as_ref(), picture.size());
        if let Some(id) = texture_id {
            paint_turned(ui, id, picture, edit.turns);
        }
        // The region is kept in the picture's pixels, so a drag is turned
        // into pixels before it is applied.
        if tool.kind == Tool::Crop {
            let moving = ui.interact(
                to_screen(edit.crop, picture, scale),
                ui.id().with("crop-inside"),
                Sense::DRAG,
            );
            if moving.drag_started() {
                remember(&mut history, &edit);
            }
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
                if response.drag_started() {
                    remember(&mut history, &edit);
                }
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
        if tool.kind != Tool::Crop {
            let canvas = ui.interact(picture, ui.id().with("markup"), Sense::click_and_drag());
            let point = |pos: Pos2| {
                (
                    ((pos.x - picture.min.x) / scale).clamp(0.0, wide as f32),
                    ((pos.y - picture.min.y) / scale).clamp(0.0, tall as f32),
                )
            };
            if canvas.drag_started()
                && let Some(pos) = canvas.interact_pointer_pos()
            {
                if !matches!(tool.kind, Tool::Text | Tool::Emoji) {
                    remember(&mut history, &edit);
                }
                ui.ctx().data_mut(|data| {
                    data.remove::<TextDrag>(area_id().with("text-drag"));
                    data.remove::<EmojiDrag>(area_id().with("emoji-drag"));
                });
                let p = point(pos);
                match tool.kind {
                    Tool::Pen => edit.marks.push(PictureMark::Stroke {
                        points: vec![p],
                        color: COLORS[tool.color],
                        radius: stroke_radius(tool.width, scale),
                    }),
                    Tool::Blur => edit.marks.push(PictureMark::Mosaic(PictureCrop {
                        x: p.0 as u32,
                        y: p.1 as u32,
                        width: 1,
                        height: 1,
                    })),
                    Tool::Shapes => {
                        let hit = edit
                            .marks
                            .iter()
                            .enumerate()
                            .rev()
                            .find_map(|(index, mark)| {
                                if let PictureMark::Shape { from, to, .. } = mark {
                                    let near_end = (to.0 - p.0).hypot(to.1 - p.1) < 14.0 / scale;
                                    let inside = p.0 >= from.0.min(to.0)
                                        && p.0 <= from.0.max(to.0)
                                        && p.1 >= from.1.min(to.1)
                                        && p.1 <= from.1.max(to.1);
                                    if near_end || inside {
                                        Some((index, *from, *to, !near_end))
                                    } else {
                                        None
                                    }
                                } else {
                                    None
                                }
                            })
                            .filter(|_| tool.edit_shapes);
                        let drag = if let Some((index, from, to, move_whole)) = hit {
                            ShapeDrag {
                                index,
                                start: p,
                                from,
                                to,
                                move_whole,
                            }
                        } else {
                            let index = edit.marks.len();
                            edit.marks.push(PictureMark::Shape {
                                kind: tool.shape,
                                from: p,
                                to: p,
                                color: COLORS[tool.color],
                                width: tool.width / scale,
                                filled: tool.filled,
                            });
                            ShapeDrag {
                                index,
                                start: p,
                                from: p,
                                to: p,
                                move_whole: false,
                            }
                        };
                        ui.ctx()
                            .data_mut(|data| data.insert_temp(area_id().with("shape-drag"), drag));
                    }
                    Tool::Text => {
                        let hit = edit
                            .marks
                            .iter()
                            .enumerate()
                            .rev()
                            .find_map(|(index, mark)| {
                                if let PictureMark::Text { at, text, size, .. } = mark {
                                    let width = text.chars().count().max(1) as f32 * size * 0.6;
                                    (p.0 >= at.0
                                        && p.0 <= at.0 + width
                                        && p.1 >= at.1
                                        && p.1 <= at.1 + size * 1.3)
                                        .then_some(TextDrag {
                                            index,
                                            start: p,
                                            at: *at,
                                        })
                                } else {
                                    None
                                }
                            });
                        if let Some(hit) = hit {
                            remember(&mut history, &edit);
                            ui.ctx().data_mut(|data| {
                                data.insert_temp(area_id().with("text-drag"), hit)
                            });
                        }
                    }
                    Tool::Emoji => {
                        let hit = edit
                            .marks
                            .iter()
                            .enumerate()
                            .rev()
                            .find_map(|(index, mark)| {
                                if let PictureMark::Emoji { at, size, .. } = mark {
                                    let inside = p.0 >= at.0
                                        && p.0 <= at.0 + size
                                        && p.1 >= at.1
                                        && p.1 <= at.1 + size;
                                    inside.then_some(EmojiDrag {
                                        index,
                                        start: p,
                                        at: *at,
                                        size: *size,
                                        resize: p.0 > at.0 + size * 0.75
                                            && p.1 > at.1 + size * 0.75,
                                    })
                                } else {
                                    None
                                }
                            });
                        if let Some(hit) = hit {
                            remember(&mut history, &edit);
                            ui.ctx().data_mut(|data| {
                                data.insert_temp(area_id().with("emoji-drag"), hit)
                            });
                        }
                    }
                    Tool::Crop => {}
                }
                ui.ctx().data_mut(|data| {
                    data.insert_temp(area_id().with("mark-start"), (p.0 as u32, p.1 as u32))
                });
            }
            if canvas.dragged()
                && let Some(pos) = canvas.interact_pointer_pos()
            {
                let p = point(pos);
                match edit.marks.last_mut() {
                    Some(PictureMark::Stroke { points, .. }) if tool.kind == Tool::Pen => {
                        if points.last() != Some(&p) {
                            points.push(p);
                        }
                    }
                    Some(PictureMark::Mosaic(rect)) if tool.kind == Tool::Blur => {
                        let start = ui
                            .ctx()
                            .data(|data| data.get_temp::<(u32, u32)>(area_id().with("mark-start")))
                            .unwrap_or((p.0 as u32, p.1 as u32));
                        *rect = PictureCrop {
                            x: start.0.min(p.0 as u32),
                            y: start.1.min(p.1 as u32),
                            width: start.0.abs_diff(p.0 as u32).max(1),
                            height: start.1.abs_diff(p.1 as u32).max(1),
                        };
                    }
                    _ => {}
                }
                if tool.kind == Tool::Shapes
                    && let Some(drag) = ui
                        .ctx()
                        .data(|data| data.get_temp::<ShapeDrag>(area_id().with("shape-drag")))
                    && let Some(PictureMark::Shape { from, to, .. }) =
                        edit.marks.get_mut(drag.index)
                {
                    let delta = (p.0 - drag.start.0, p.1 - drag.start.1);
                    if drag.move_whole {
                        *from = (
                            (drag.from.0 + delta.0).clamp(0.0, wide as f32),
                            (drag.from.1 + delta.1).clamp(0.0, tall as f32),
                        );
                        *to = (
                            (drag.to.0 + delta.0).clamp(0.0, wide as f32),
                            (drag.to.1 + delta.1).clamp(0.0, tall as f32),
                        );
                    } else {
                        *to = p;
                    }
                }
                if tool.kind == Tool::Text
                    && let Some(drag) = ui
                        .ctx()
                        .data(|data| data.get_temp::<TextDrag>(area_id().with("text-drag")))
                    && let Some(PictureMark::Text { at, .. }) = edit.marks.get_mut(drag.index)
                {
                    *at = (
                        (drag.at.0 + p.0 - drag.start.0).clamp(0.0, wide as f32),
                        (drag.at.1 + p.1 - drag.start.1).clamp(0.0, tall as f32),
                    );
                }
                if tool.kind == Tool::Emoji
                    && let Some(drag) = ui
                        .ctx()
                        .data(|data| data.get_temp::<EmojiDrag>(area_id().with("emoji-drag")))
                    && let Some(PictureMark::Emoji { at, size, .. }) =
                        edit.marks.get_mut(drag.index)
                {
                    if drag.resize {
                        *size = (drag.size + p.0 - drag.start.0).max(8.0);
                    } else {
                        *at = (
                            (drag.at.0 + p.0 - drag.start.0).clamp(0.0, wide as f32),
                            (drag.at.1 + p.1 - drag.start.1).clamp(0.0, tall as f32),
                        );
                    }
                }
            }
            if (canvas.clicked() || canvas.double_clicked())
                && let Some(pos) = canvas.interact_pointer_pos()
            {
                let p = point(pos);
                match tool.kind {
                    Tool::Text => {
                        let hit = edit
                            .marks
                            .iter()
                            .enumerate()
                            .rev()
                            .find_map(|(index, mark)| {
                                if let PictureMark::Text { at, text, size, .. } = mark {
                                    let width = text.chars().count().max(1) as f32 * size * 0.6;
                                    (p.0 >= at.0
                                        && p.0 <= at.0 + width
                                        && p.1 >= at.1
                                        && p.1 <= at.1 + size * 1.3)
                                        .then_some(index)
                                } else {
                                    None
                                }
                            });
                        remember(&mut history, &edit);
                        let index = hit.unwrap_or_else(|| {
                            edit.marks.push(PictureMark::Text {
                                at: p,
                                text: String::new(),
                                color: COLORS[tool.color],
                                background: tool.background,
                                size: 22.0 / scale,
                            });
                            edit.marks.len() - 1
                        });
                        ui.ctx().data_mut(|data| {
                            data.insert_temp(area_id().with("editing-text"), index);
                            data.insert_temp(area_id().with("focus-text"), true);
                        });
                    }
                    Tool::Emoji => {
                        let selected = ui
                            .ctx()
                            .data(|data| data.get_temp::<String>(area_id().with("selected-emoji")))
                            .unwrap_or_else(|| "😀".to_owned());
                        remember(&mut history, &edit);
                        edit.marks.push(PictureMark::Emoji {
                            at: p,
                            emoji: selected,
                            size: 36.0 / scale,
                        });
                    }
                    _ => {}
                }
            }
        }
        let editing_text = ui
            .ctx()
            .data(|data| data.get_temp::<usize>(area_id().with("editing-text")));
        for (mark_index, mark) in edit.marks.iter().enumerate() {
            match mark {
                PictureMark::Stroke {
                    points,
                    color,
                    radius,
                } => {
                    let color = Color32::from_rgb(color[0], color[1], color[2]);
                    let positions: Vec<_> = points
                        .iter()
                        .map(|&(x, y)| picture.min + vec2(x * scale, y * scale))
                        .collect();
                    if positions.len() == 1 {
                        ui.painter()
                            .circle_filled(positions[0], *radius * scale, color);
                    } else if positions.len() > 1 {
                        ui.painter().add(egui::Shape::line(
                            positions,
                            Stroke::new(*radius * 2.0 * scale, color),
                        ));
                    }
                }
                PictureMark::Redact(rect) => {
                    ui.painter().rect_filled(
                        to_screen(*rect, picture, scale),
                        0.0,
                        Color32::from_rgb(24, 24, 24),
                    );
                }
                PictureMark::Mosaic(rect) => {
                    if let Some(id) = texture_id {
                        paint_mosaic(ui, id, *rect, picture, scale, edit.turns, (wide, tall));
                    }
                    if tool.kind == Tool::Blur {
                        ui.painter().rect_stroke(
                            to_screen(*rect, picture, scale),
                            0.0,
                            Stroke::new(2.0, Color32::WHITE),
                            StrokeKind::Outside,
                        );
                    }
                }
                PictureMark::Shape {
                    kind,
                    from,
                    to,
                    color,
                    width,
                    filled,
                } => {
                    let a = picture.min + vec2(from.0 * scale, from.1 * scale);
                    let b = picture.min + vec2(to.0 * scale, to.1 * scale);
                    let color = Color32::from_rgb(color[0], color[1], color[2]);
                    let stroke = Stroke::new((width * scale).max(1.0), color);
                    match kind {
                        PictureShape::Rectangle => {
                            let rect = Rect::from_two_pos(a, b);
                            if *filled {
                                ui.painter().rect_filled(rect, 0.0, color);
                            } else {
                                ui.painter()
                                    .rect_stroke(rect, 0.0, stroke, StrokeKind::Inside);
                            }
                        }
                        PictureShape::Ellipse => {
                            let center = Pos2::new((a.x + b.x) / 2.0, (a.y + b.y) / 2.0);
                            let radius = vec2((a.x - b.x).abs() / 2.0, (a.y - b.y).abs() / 2.0);
                            let points: Vec<_> = (0..=64)
                                .map(|n| {
                                    let angle = std::f32::consts::TAU * n as f32 / 64.0;
                                    center + vec2(radius.x * angle.cos(), radius.y * angle.sin())
                                })
                                .collect();
                            if *filled {
                                ui.painter().add(egui::Shape::convex_polygon(
                                    points,
                                    color,
                                    Stroke::NONE,
                                ));
                            } else {
                                ui.painter().add(egui::Shape::line(points, stroke));
                            }
                        }
                        PictureShape::Line | PictureShape::Arrow => {
                            ui.painter().line_segment([a, b], stroke);
                            if *kind == PictureShape::Arrow {
                                let angle = (b.y - a.y).atan2(b.x - a.x);
                                let head = a.distance(b).mul_add(0.25, 0.0).clamp(8.0, 30.0);
                                for side in [-0.55_f32, 0.55] {
                                    let back = angle + std::f32::consts::PI + side;
                                    ui.painter().line_segment(
                                        [b, b + vec2(head * back.cos(), head * back.sin())],
                                        stroke,
                                    );
                                }
                            }
                        }
                    }
                }
                PictureMark::Text {
                    at,
                    text,
                    color,
                    background,
                    size,
                } => {
                    if editing_text == Some(mark_index) {
                        continue;
                    }
                    let pos = picture.min + vec2(at.0 * scale, at.1 * scale);
                    let color = Color32::from_rgb(color[0], color[1], color[2]);
                    let font = egui::FontId::proportional(size * scale);
                    let galley = ui.painter().layout_no_wrap(text.clone(), font, color);
                    if *background {
                        ui.painter().rect_filled(
                            Rect::from_min_size(
                                pos - vec2(4.0, 2.0),
                                galley.size() + vec2(8.0, 4.0),
                            ),
                            8.0,
                            Color32::from_rgb(22, 28, 32),
                        );
                    }
                    ui.painter().galley(pos, galley, color);
                }
                PictureMark::Stamp {
                    at,
                    kind,
                    color,
                    size,
                } => {
                    let pos = picture.min + vec2(at.0 * scale, at.1 * scale);
                    let glyph = match kind {
                        PictureStamp::Smile => "☺",
                        PictureStamp::Heart => "♥",
                        PictureStamp::Star => "★",
                    };
                    ui.painter().text(
                        pos,
                        egui::Align2::LEFT_TOP,
                        glyph,
                        egui::FontId::proportional(size * scale),
                        Color32::from_rgb(color[0], color[1], color[2]),
                    );
                }
                PictureMark::Emoji { at, emoji, size } => {
                    let place = picture.min + vec2(at.0 * scale, at.1 * scale);
                    let rect = Rect::from_min_size(place, Vec2::splat(size * scale));
                    crate::emoji::paint_cluster(ui, emoji, rect);
                    if tool.kind == Tool::Emoji {
                        ui.painter().rect_stroke(
                            rect,
                            0.0,
                            Stroke::new(1.0, Color32::WHITE),
                            StrokeKind::Outside,
                        );
                        ui.painter()
                            .circle_filled(rect.right_bottom(), 4.0, Color32::WHITE);
                    }
                }
            }
        }
        if tool.kind == Tool::Text
            && let Some(index) = ui
                .ctx()
                .data(|data| data.get_temp::<usize>(area_id().with("editing-text")))
            && let Some(PictureMark::Text {
                at,
                text,
                color,
                background,
                size,
            }) = edit.marks.get_mut(index)
        {
            *color = COLORS[tool.color];
            *background = tool.background;
            let at = picture.min + vec2(at.0 * scale, at.1 * scale);
            let field = Rect::from_min_size(
                at,
                vec2(
                    (text.chars().count().max(6) as f32 * *size * scale * 0.65).max(90.0),
                    *size * scale * 1.5,
                ),
            );
            let response = ui.put(
                field,
                egui::TextEdit::singleline(text)
                    .frame(egui::Frame::NONE)
                    .font(egui::FontId::proportional(*size * scale))
                    .text_color(Color32::from_rgb(color[0], color[1], color[2])),
            );
            if ui
                .ctx()
                .data(|data| data.get_temp::<bool>(area_id().with("focus-text")))
                .unwrap_or(false)
            {
                response.request_focus();
                ui.ctx()
                    .data_mut(|data| data.remove::<bool>(area_id().with("focus-text")));
            }
            if response.lost_focus() {
                ui.ctx()
                    .data_mut(|data| data.remove::<usize>(area_id().with("editing-text")));
            }
        }
        shade_outside(ui, palette, picture, to_screen(edit.crop, picture, scale));
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(Rect::from_min_size(top.min, vec2(92.0, BAR)))
                .layout(Layout::left_to_right(Align::Center)),
            |ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                if tool_button(
                    ui,
                    Icon::X,
                    &crate::i18n::gettext(locale, "Cancel (Esc)"),
                    false,
                    &palette,
                )
                .clicked()
                {
                    actions.push(Action::CancelPictureEdit);
                }
                if tool_button(
                    ui,
                    Icon::Undo,
                    &crate::i18n::gettext(locale, "Undo"),
                    false,
                    &palette,
                )
                .clicked()
                {
                    change = Some(Change::Undo);
                }
            },
        );
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(Rect::from_center_size(top.center(), vec2(346.0, BAR)))
                .layout(Layout::left_to_right(Align::Center)),
            |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                if tool_button(
                    ui,
                    Icon::RotateLeft,
                    &crate::i18n::gettext(locale, "Turn left"),
                    false,
                    &palette,
                )
                .clicked()
                {
                    change = Some(Change::Turn { clockwise: false });
                }
                if tool_button(
                    ui,
                    Icon::RotateRight,
                    &crate::i18n::gettext(locale, "Turn right"),
                    false,
                    &palette,
                )
                .clicked()
                {
                    change = Some(Change::Turn { clockwise: true });
                }
                for (kind, icon, label) in [
                    (Tool::Crop, Icon::Crop, crate::i18n::gettext(locale, "Crop")),
                    (
                        Tool::Pen,
                        Icon::Pencil,
                        crate::i18n::gettext(locale, "Draw"),
                    ),
                    (
                        Tool::Shapes,
                        Icon::SquarePen,
                        crate::i18n::gettext(locale, "Shapes"),
                    ),
                    (
                        Tool::Text,
                        Icon::FileText,
                        crate::i18n::gettext(locale, "Text"),
                    ),
                    (
                        Tool::Blur,
                        Icon::EyeOff,
                        crate::i18n::gettext(locale, "Blur"),
                    ),
                    (
                        Tool::Emoji,
                        Icon::Smile,
                        crate::i18n::gettext(locale, "Emoji"),
                    ),
                ] {
                    if tool_button(ui, icon, &label, tool.kind == kind, &palette).clicked() {
                        tool.kind = kind;
                        if kind != Tool::Pen {
                            tool.width_open = false;
                        }
                    }
                }
            },
        );
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(Rect::from_min_size(
                    pos2(top.right() - 210.0, top.top()),
                    vec2(210.0, BAR),
                ))
                .layout(Layout::right_to_left(Align::Center)),
            |ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let done = ui.add(
                    egui::Button::new(
                        egui::RichText::new(crate::i18n::gettext(locale, "Done"))
                            .color(palette.on_accent),
                    )
                    .fill(palette.accent)
                    .corner_radius(18.0),
                );
                ui.ctx()
                    .data_mut(|data| data.insert_temp(keep_note_id(), done.id));
                if done.clicked() {
                    actions.push(Action::ApplyPictureEdit);
                }
                if ui.button(crate::i18n::gettext(locale, "Reset")).clicked() {
                    change = Some(Change::Reset);
                }
                if ui
                    .selectable_label(edit.hd, "HD")
                    .on_hover_text(crate::i18n::gettext(locale, "High definition quality"))
                    .clicked()
                {
                    remember(&mut history, &edit);
                    edit.hd = !edit.hd;
                }
            },
        );
        if matches!(
            tool.kind,
            Tool::Pen | Tool::Shapes | Tool::Text | Tool::Emoji
        ) {
            let wanted: f32 = match tool.kind {
                Tool::Pen => 410.0,
                Tool::Shapes => 630.0,
                Tool::Text => 343.0,
                Tool::Emoji => 165.0,
                _ => 0.0,
            };
            let bar_width = wanted.min(bottom.width() - 8.0);
            ui.scope_builder(
                egui::UiBuilder::new()
                    .max_rect(Rect::from_center_size(
                        bottom.center(),
                        vec2(bar_width, BAR),
                    ))
                    .layout(Layout::left_to_right(Align::Center)),
                |ui| {
                    ui.spacing_mut().item_spacing.x = 5.0;
                    let color_names = [
                        "Dark gray",
                        "Gray",
                        "White",
                        "Cyan",
                        "Green",
                        "Purple",
                        "Orange",
                        "Red",
                    ];
                    if tool.kind != Tool::Emoji {
                        for (index, color) in COLORS.iter().enumerate() {
                            let label = crate::i18n::gettext(locale, color_names[index]);
                            if swatch(ui, *color, tool.color == index, &label, &palette).clicked() {
                                tool.color = index;
                            }
                        }
                    }
                    if matches!(tool.kind, Tool::Pen | Tool::Shapes) {
                        let size = tool_button(
                            ui,
                            Icon::ChevronUp,
                            &crate::i18n::gettext(locale, "Brush size"),
                            tool.width_open,
                            &palette,
                        );
                        width_anchor = Some(size.rect);
                        if size.clicked() {
                            tool.width_open = !tool.width_open;
                        }
                        for size in [2.0, 4.0, 8.0, 14.0] {
                            let label =
                                format!("{}: {size}", crate::i18n::gettext(locale, "Brush size"));
                            if size_dot(ui, size, tool.width == size, &label, &palette).clicked() {
                                tool.width = size;
                            }
                        }
                    }
                    if tool.kind == Tool::Shapes {
                        ui.separator();
                        for (kind, label) in [
                            (PictureShape::Rectangle, "□"),
                            (PictureShape::Ellipse, "○"),
                            (PictureShape::Arrow, "➜"),
                            (PictureShape::Line, "╱"),
                        ] {
                            if ui.selectable_label(tool.shape == kind, label).clicked() {
                                tool.shape = kind;
                            }
                        }
                        ui.toggle_value(&mut tool.filled, crate::i18n::gettext(locale, "Filled"));
                        ui.toggle_value(
                            &mut tool.edit_shapes,
                            crate::i18n::gettext(locale, "Select"),
                        );
                    }
                    if tool.kind == Tool::Text {
                        ui.toggle_value(
                            &mut tool.background,
                            crate::i18n::gettext(locale, "Background"),
                        );
                    }
                    if tool.kind == Tool::Emoji {
                        let current = ui
                            .ctx()
                            .data(|data| data.get_temp::<String>(area_id().with("selected-emoji")))
                            .unwrap_or_else(|| "Emoji".to_owned());
                        ui.label(crate::i18n::gettext(locale, "Choose Emoji:"));
                        if ui.button(current).clicked() {
                            let key = area_id().with("emoji-picker-open");
                            let open = ui
                                .ctx()
                                .data(|data| data.get_temp::<bool>(key))
                                .unwrap_or(false);
                            ui.ctx().data_mut(|data| data.insert_temp(key, !open));
                        }
                    }
                },
            );
        }
        if tool.kind == Tool::Emoji
            && ui
                .ctx()
                .data(|data| data.get_temp::<bool>(area_id().with("emoji-picker-open")))
                .unwrap_or(false)
        {
            let height = 430.0_f32.min((bottom.top() - top.bottom() - 12.0).max(250.0));
            let width = 420.0_f32.min(room.width() - 16.0);
            let popup = Rect::from_min_size(
                pos2(
                    (bottom.center().x - width / 2.0).max(room.left()),
                    bottom.top() - height - 8.0,
                ),
                vec2(width, height),
            );
            ui.scope_builder(egui::UiBuilder::new().max_rect(popup), |ui| {
                egui::Frame::new()
                    .fill(palette.surface)
                    .stroke(Stroke::new(1.0, palette.secondary))
                    .corner_radius(12.0)
                    .inner_margin(10.0)
                    .show(ui, |ui| {
                        ui.set_width(width - 20.0);
                        ui.horizontal(|ui| {
                            ui.label(crate::i18n::gettext(locale, "Choose Emoji"));
                            if ui.button("×").clicked() {
                                ui.ctx().data_mut(|data| {
                                    data.insert_temp(area_id().with("emoji-picker-open"), false)
                                });
                            }
                        });
                        let key = area_id().with("emoji-search");
                        let mut query = ui
                            .ctx()
                            .data(|data| data.get_temp::<String>(key))
                            .unwrap_or_default();
                        let category_key = area_id().with("emoji-category");
                        let mut category = ui
                            .ctx()
                            .data(|data| data.get_temp::<Option<emojis::Group>>(category_key))
                            .flatten();
                        let mut picked = None;
                        ui.allocate_ui_with_layout(
                            vec2(width - 20.0, height - 105.0),
                            Layout::top_down(Align::Min),
                            |ui| {
                                picked = super::picker::editor_emoji_grid(
                                    ui,
                                    &palette,
                                    locale,
                                    &mut query,
                                    &recent_emoji,
                                    &mut category,
                                );
                            },
                        );
                        super::picker::editor_emoji_categories(ui, &palette, &mut category);
                        ui.ctx().data_mut(|data| {
                            data.insert_temp(key, query);
                            data.insert_temp(category_key, category);
                            if let Some(emoji) = picked {
                                data.insert_temp(area_id().with("selected-emoji"), emoji);
                                data.insert_temp(area_id().with("emoji-picker-open"), false);
                            }
                        });
                    });
            });
        }
        if matches!(tool.kind, Tool::Pen | Tool::Shapes)
            && tool.width_open
            && let Some(anchor) = width_anchor
        {
            let popup = Rect::from_min_size(
                pos2(
                    anchor
                        .left()
                        .clamp(room.left(), (room.right() - 220.0).max(room.left())),
                    bottom.top() - 94.0,
                ),
                vec2(220.0, 84.0),
            );
            ui.scope_builder(
                egui::UiBuilder::new()
                    .max_rect(popup)
                    .layout(Layout::top_down(Align::Min)),
                |ui| {
                    egui::Frame::new()
                        .fill(palette.surface)
                        .stroke(Stroke::new(1.0, palette.secondary))
                        .inner_margin(10.0)
                        .show(ui, |ui| {
                            ui.label(crate::i18n::gettext(locale, "Brush size"));
                            ui.horizontal(|ui| {
                                let (sample, _) =
                                    ui.allocate_exact_size(Vec2::splat(28.0), Sense::hover());
                                let color = COLORS[tool.color];
                                ui.painter().circle_filled(
                                    sample.center(),
                                    tool.width / 2.0,
                                    Color32::from_rgb(color[0], color[1], color[2]),
                                );
                                ui.add_sized(
                                    [150.0, 20.0],
                                    egui::Slider::new(&mut tool.width, 1.0..=16.0)
                                        .show_value(false),
                                );
                            });
                        });
                },
            );
        }
    };
    if let Some(ui) = parent {
        draw_editor_contents(ui, draw);
    } else {
        egui::Area::new(area_id())
            .order(egui::Order::Foreground)
            .fixed_pos(screen.min)
            .show(ctx, |ui| draw_editor_contents(ui, draw));
    }
    match change {
        Some(Change::Reset) => {
            remember(&mut history, &edit);
            edit = edit.reset();
        }
        Some(Change::Turn { clockwise }) => {
            remember(&mut history, &edit);
            edit = edit.turned(clockwise);
        }
        Some(Change::Undo) => {
            if let Some(previous) = history.stack.pop() {
                edit = previous;
            }
        }
        None => {}
    }
    app.picture_edit = Some(edit);
    ctx.data_mut(|data| data.insert_temp(tool_id, tool));
    ctx.data_mut(|data| data.insert_temp(history_id, history));
    app.actions.extend(actions);
}

/// Draw the editor's toolbar and canvas into an already allocated UI region.
/// The caller owns the region and its sizing; this helper never creates a
/// window, area, or overlay of its own.
fn draw_editor_contents(ui: &mut egui::Ui, draw: impl FnOnce(&mut egui::Ui)) {
    draw(ui);
}

fn tool_button(
    ui: &mut egui::Ui,
    icon: Icon,
    label: &str,
    active: bool,
    palette: &theme::Palette,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(38.0), Sense::click());
    response
        .widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, active, label));
    if ui.is_rect_visible(rect) {
        if active || response.hovered() {
            ui.painter().rect_filled(
                rect,
                19.0,
                if active {
                    palette.surface_hover
                } else {
                    palette.overlay
                },
            );
        }
        theme::paint_icon(
            ui,
            icon,
            rect,
            19.0,
            if active {
                palette.text
            } else {
                palette.secondary
            },
        );
    }
    response
        .on_hover_text(label)
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn swatch(
    ui: &mut egui::Ui,
    rgb: [u8; 3],
    active: bool,
    label: &str,
    palette: &theme::Palette,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(26.0), Sense::click());
    response
        .widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, active, label));
    let color = Color32::from_rgb(rgb[0], rgb[1], rgb[2]);
    ui.painter().circle_filled(rect.center(), 9.0, color);
    if active {
        ui.painter()
            .circle_stroke(rect.center(), 12.0, Stroke::new(1.5, palette.text));
    }
    response
        .on_hover_text(label)
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn size_dot(
    ui: &mut egui::Ui,
    size: f32,
    active: bool,
    label: &str,
    palette: &theme::Palette,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(26.0), Sense::click());
    response
        .widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, active, label));
    ui.painter()
        .circle_filled(rect.center(), 2.0 + size * 0.42, palette.secondary);
    if active {
        ui.painter()
            .circle_stroke(rect.center(), 12.0, Stroke::new(1.5, palette.text));
    }
    response
        .on_hover_text(label)
        .on_hover_cursor(egui::CursorIcon::PointingHand)
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
    let limit = crate::backend::picture_edit::PREVIEW_SIDE as u32;
    let small = if width > limit || height > limit {
        let scale = limit as f32 / width.max(height) as f32;
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

/// Repeat each source sample across a small square, matching the exported
/// mosaic's block boundaries even on a turned picture.
fn paint_mosaic(
    ui: &egui::Ui,
    texture: egui::TextureId,
    rect: PictureCrop,
    picture: Rect,
    scale: f32,
    turns: u8,
    size: (u32, u32),
) {
    let (wide, tall) = size;
    let rect = rect.clamped(wide, tall);
    let block = (wide.max(tall) / 60).max(12);
    if rect.width < block / 2 || rect.height < block / 2 {
        ui.painter().rect_filled(
            to_screen(rect, picture, scale),
            0.0,
            Color32::from_rgb(24, 24, 24),
        );
        return;
    }
    let mut mesh = egui::epaint::Mesh {
        texture_id: texture,
        ..Default::default()
    };
    for y in (rect.y..rect.y + rect.height).step_by(block as usize) {
        for x in (rect.x..rect.x + rect.width).step_by(block as usize) {
            let right = (x + block).min(rect.x + rect.width);
            let bottom = (y + block).min(rect.y + rect.height);
            let u = (x as f32 + right as f32) / (2.0 * wide as f32);
            let v = (y as f32 + bottom as f32) / (2.0 * tall as f32);
            let uv = match turns % 4 {
                1 => pos2(v, 1.0 - u),
                2 => pos2(1.0 - u, 1.0 - v),
                3 => pos2(1.0 - v, u),
                _ => pos2(u, v),
            };
            let area = Rect::from_min_max(
                picture.min + vec2(x as f32 * scale, y as f32 * scale),
                picture.min + vec2(right as f32 * scale, bottom as f32 * scale),
            );
            let base = mesh.vertices.len() as u32;
            for pos in [
                area.left_top(),
                area.right_top(),
                area.right_bottom(),
                area.left_bottom(),
            ] {
                mesh.vertices.push(egui::epaint::Vertex {
                    pos,
                    uv,
                    color: Color32::WHITE,
                });
            }
            mesh.indices
                .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        }
    }
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

/// Fades what the crop leaves out toward the window, and marks the crop's
/// border and handles in the theme's accent.
fn shade_outside(ui: &egui::Ui, palette: theme::Palette, picture: Rect, crop: Rect) {
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
    fn pen_width_is_independent_of_source_resolution() {
        let on_small = stroke_radius(3.0, 8.0);
        let on_large = stroke_radius(3.0, 0.5);
        assert!((on_small * 8.0 * 2.0 - 3.0).abs() < 0.001);
        assert!((on_large * 0.5 * 2.0 - 3.0).abs() < 0.001);
    }

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
}
