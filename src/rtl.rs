//! Right-to-left layout helpers.
//!
//! egui has no process-wide direction: every horizontal row states its own,
//! and a row's children are laid out in the order they are added. These
//! helpers name the two directions a row can take, so the interface mirrors
//! when the active locale is written right to left without reversing any
//! text: bidirectional text still lays itself out.

use egui::{Align, Context, InnerResponse, Layout, Panel, Ui, vec2};

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

#[cfg(test)]
mod tests {
    #[test]
    fn the_frame_direction_is_read_back() {
        let ctx = egui::Context::default();
        assert!(!super::is(&ctx));
        super::set(&ctx, true);
        assert!(super::is(&ctx));
        super::set(&ctx, false);
        assert!(!super::is(&ctx));
    }
}
