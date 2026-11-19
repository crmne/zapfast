//! Right-to-left layout helpers.
//!
//! egui has no process-wide direction: every horizontal row states its own,
//! and a row's children are laid out in the order they are added. These
//! helpers name the two directions a row can take, so the interface mirrors
//! when the active locale is written right to left without reversing any
//! text: bidirectional text still lays itself out.

use egui::{Align, Context, InnerResponse, Layout, Panel, Rect, Ui, vec2};

/// The per-frame direction, set once from the active locale.
fn direction_id() -> egui::Id {
    egui::Id::new("right-to-left")
}

/// Applies the reader's direction to the whole frame.
pub fn set(ctx: &Context, rtl: bool) {
    ctx.data_mut(|data| data.insert_temp(direction_id(), rtl));
}

/// Whether this frame reads right to left.
pub fn is(ctx: &Context) -> bool {
    ctx.data(|data| data.get_temp::<bool>(direction_id()).unwrap_or(false))
}

/// Whether `ui` reads right to left.
pub fn enabled(ui: &Ui) -> bool {
    is(ui.ctx())
}

/// Orders a pair the way the reader sees it: the reading start first.
pub fn pair<T>(ui: &Ui, start: T, end: T) -> (T, T) {
    if enabled(ui) {
        (end, start)
    } else {
        (start, end)
    }
}

/// `ltr` in a left-to-right interface and `rtl` in a right-to-left one, for a
/// choice that is not one coordinate: a shadow's side, a pair of alignments.
pub fn pick<T>(ui: &Ui, ltr: T, rtl: T) -> T {
    if enabled(ui) { rtl } else { ltr }
}

/// The x of `rect`'s reading start: its right edge in RTL, its left otherwise.
pub fn start_edge(ui: &Ui, rect: Rect) -> f32 {
    if enabled(ui) {
        rect.right()
    } else {
        rect.left()
    }
}

/// The x of `rect`'s reading end: its left edge in RTL, its right otherwise.
pub fn end_edge(ui: &Ui, rect: Rect) -> f32 {
    if enabled(ui) {
        rect.left()
    } else {
        rect.right()
    }
}

/// Advances `x` by `width` in the reading direction: positive for LTR,
/// negative for RTL.
pub fn ahead(ui: &Ui, x: f32, width: f32) -> f32 {
    if enabled(ui) { x - width } else { x + width }
}

/// Moves `x` by `width` against the reading direction: positive for RTL,
/// negative for LTR. It takes a reading end back towards the text, or a row's
/// badges back towards its start.
pub fn behind(ui: &Ui, x: f32, width: f32) -> f32 {
    if enabled(ui) { x + width } else { x - width }
}

/// Available space from `x` to `end` in the reading direction.
pub fn room(ui: &Ui, x: f32, end: f32) -> f32 {
    if enabled(ui) {
        (x - end).max(0.0)
    } else {
        (end - x).max(0.0)
    }
}

/// The x where a block of `width` whose reading start sits at `start` is
/// painted: its left edge in LTR, and `width` points left of `start` in RTL.
pub fn text_left(ui: &Ui, start: f32, width: f32) -> f32 {
    if enabled(ui) { start - width } else { start }
}

/// The x where a block of `width` whose reading end sits at `end` is painted.
pub fn text_at_end(ui: &Ui, end: f32, width: f32) -> f32 {
    if enabled(ui) { end } else { end - width }
}

/// A horizontal layout whose first child sits at the reading start.
pub fn start(ui: &Ui, valign: Align) -> Layout {
    if enabled(ui) {
        Layout::right_to_left(valign)
    } else {
        Layout::left_to_right(valign)
    }
}

/// A horizontal layout whose contents end at the reading end.
pub fn end(ui: &Ui) -> Layout {
    if enabled(ui) {
        Layout::left_to_right(Align::Center)
    } else {
        Layout::right_to_left(Align::Center)
    }
}

/// The alignment that puts text at the reading start of a vertical column.
pub fn align(ui: &Ui) -> Align {
    if enabled(ui) { Align::Max } else { Align::Min }
}

/// A vertical layout whose text starts at the reading side.
pub fn top(ui: &Ui) -> Layout {
    Layout::top_down(align(ui))
}

/// A horizontal row whose first child sits at the reading start, as
/// `ui.horizontal` lays one out.
pub fn row<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    let size = vec2(
        ui.available_size_before_wrap().x,
        ui.spacing().interact_size.y,
    );
    ui.allocate_ui_with_layout(size, start(ui, Align::Center), add)
}

/// A wrapping horizontal row in the reader's direction, as
/// `ui.horizontal_wrapped` lays one out.
pub fn wrapped_row<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    let size = vec2(
        ui.available_size_before_wrap().x,
        ui.spacing().interact_size.y,
    );
    let layout = start(ui, Align::Center).with_main_wrap(true);
    ui.allocate_ui_with_layout(size, layout, add)
}

/// A panel at the reading start, the rail and the chat list.
pub fn start_panel(ctx: &Context, id: impl Into<egui::Id>) -> Panel {
    if is(ctx) {
        Panel::right(id)
    } else {
        Panel::left(id)
    }
}

/// The arrow that points back: left, or right when reading right to left.
pub fn back_icon(ui: &Ui) -> crate::theme::Icon {
    if enabled(ui) {
        crate::theme::Icon::ArrowRight
    } else {
        crate::theme::Icon::ArrowLeft
    }
}

/// A panel at the reading end, such as search or chat info.
pub fn end_panel(ctx: &Context, id: impl Into<egui::Id>) -> Panel {
    if is(ctx) {
        Panel::left(id)
    } else {
        Panel::right(id)
    }
}

/// A `width`-wide rect at the reading end of `region`: the right edge in LTR,
/// the left in RTL. For a pane drawn in an `Area`, which has no `Ui` yet.
pub fn end_rect(ctx: &Context, region: Rect, width: f32) -> Rect {
    let left = if is(ctx) {
        region.left()
    } else {
        region.right() - width
    };
    Rect::from_min_max(
        egui::pos2(left, region.top()),
        egui::pos2(left + width, region.bottom()),
    )
}

#[cfg(test)]
mod tests {
    use egui::{Pos2, RawInput, Rect, pos2, vec2};

    #[test]
    fn the_frame_direction_is_read_back() {
        let ctx = egui::Context::default();
        assert!(!super::is(&ctx));
        super::set(&ctx, true);
        assert!(super::is(&ctx));
        super::set(&ctx, false);
        assert!(!super::is(&ctx));
    }

    /// The reading-direction helpers mirror every coordinate they touch, so a
    /// row can say where its start and end are once and keep the arithmetic
    /// out of its drawing code.
    #[test]
    fn the_direction_helpers_mirror() {
        let ctx = egui::Context::default();
        let measure = |rtl: bool| {
            super::set(&ctx, rtl);
            let values = std::cell::Cell::new(None);
            let mut output = ctx.run_ui(
                RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(200.0, 100.0))),
                    ..Default::default()
                },
                |ui| {
                    let rect = Rect::from_min_max(pos2(10.0, 0.0), pos2(110.0, 20.0));
                    values.set(Some((
                        super::start_edge(ui, rect),
                        super::end_edge(ui, rect),
                        super::ahead(ui, 50.0, 10.0),
                        super::behind(ui, 50.0, 10.0),
                        super::room(ui, super::start_edge(ui, rect), super::end_edge(ui, rect)),
                        super::text_left(ui, 100.0, 30.0),
                        super::text_at_end(ui, 100.0, 30.0),
                        super::end_rect(ui.ctx(), rect, 25.0),
                    )));
                },
            );
            output.textures_delta.clear();
            values.into_inner().expect("a laid-out frame")
        };
        assert_eq!(
            measure(false),
            (
                10.0,
                110.0,
                60.0,
                40.0,
                100.0,
                100.0,
                70.0,
                Rect::from_min_max(pos2(85.0, 0.0), pos2(110.0, 20.0)),
            )
        );
        assert_eq!(
            measure(true),
            (
                110.0,
                10.0,
                40.0,
                60.0,
                100.0,
                70.0,
                100.0,
                Rect::from_min_max(pos2(10.0, 0.0), pos2(35.0, 20.0)),
            )
        );
    }
}
