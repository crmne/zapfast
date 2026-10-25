//! No input interception or frame state on macOS.

use egui::{Context, Id, Response, ScrollArea, Ui, scroll_area::ScrollAreaOutput};

#[derive(Default)]
pub struct Autoscroll {
    _private: (),
}

impl Autoscroll {
    pub fn active(&self) -> bool {
        false
    }
    pub fn scrolls(&self, _id: Id) -> bool {
        false
    }
    pub fn cancel_if_unfocused(&mut self, _ctx: &Context) -> bool {
        false
    }
    pub fn begin(&mut self, _ctx: &Context, _enabled: bool) {}
    pub fn finish(&mut self, _ctx: &Context, _enabled: bool) -> bool {
        false
    }
}

pub fn primary_drag(_ui: &Ui, _response: &Response) {}

#[expect(clippy::disallowed_methods, reason = "the wrapper the rule points to")]
pub fn show<R>(
    ui: &mut Ui,
    area: ScrollArea,
    contents: impl FnOnce(&mut Ui) -> R,
) -> ScrollAreaOutput<R> {
    area.show(ui, contents)
}

#[expect(clippy::disallowed_methods, reason = "the wrapper the rule points to")]
pub fn show_rows<R>(
    ui: &mut Ui,
    area: ScrollArea,
    row_height: f32,
    rows: usize,
    contents: impl FnOnce(&mut Ui, std::ops::Range<usize>) -> R,
) -> ScrollAreaOutput<R> {
    area.show_rows(ui, row_height, rows, contents)
}
