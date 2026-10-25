//! Middle-click scrolling belongs to the list where it starts.
//!
//! Views show their lists through [`show`] and [`show_rows`], which note
//! where each one is. The application resolves the press after drawing and
//! changes only the owning egui scroll state. Moving over another pane never
//! transfers the gesture. Speed, dead zone, cursor and the hold-or-click
//! choice follow Chromium's middle-click autoscroll, as Spotifast's does.
//! As there, the click or Escape that stops a gesture does nothing else.

use egui::{
    Context, Id, LayerId, Pos2, Rect, Response, ScrollArea, Ui, Vec2, ViewportId,
    scroll_area::ScrollAreaOutput,
};

const FRAME_ID: &str = "zapfast-autoscroll-frame";
/// Points the pointer may move on each axis before that axis scrolls.
const DEAD_ZONE: f32 = 15.0;
/// Chromium's speed is `distance ^ 2.2 * 0.000008` per fling unit, and its
/// fixed-velocity fling advances 5000 units a second.
const EXPONENT: f32 = 2.2;
const MULTIPLIER: f32 = 0.000_008 * 5000.0;
/// The longest step one frame may take, so a stalled frame cannot jump.
const MAX_STEP: f64 = 0.1;

#[derive(Clone, Copy, Default)]
struct Input {
    viewport: ViewportId,
    focused: bool,
    pointer: Option<Pos2>,
    middle: bool,
    released: bool,
    cancel: bool,
    time: f64,
}

impl Input {
    fn read(ctx: &Context) -> Self {
        let viewport = ctx.viewport_id();
        ctx.input(|input| Self {
            viewport,
            focused: input.focused,
            pointer: input.pointer.hover_pos(),
            middle: input.pointer.button_pressed(egui::PointerButton::Middle),
            released: input.pointer.any_released(),
            cancel: input.pointer.any_pressed()
                || input.key_pressed(egui::Key::Escape)
                || input
                    .events
                    .iter()
                    .any(|event| matches!(event, egui::Event::MouseWheel { .. })),
            time: input.time,
        })
    }
}

#[derive(Clone, Copy)]
struct Area {
    id: Id,
    layer: LayerId,
    rect: Rect,
    max: Vec2,
    input: Input,
    state: egui::scroll_area::State,
}

#[derive(Clone, Default)]
struct Observations {
    enabled: bool,
    root: Input,
    areas: Vec<Area>,
    /// Widgets that drag only with the primary button, pressed this frame.
    primary_drags: Vec<Id>,
}

impl Observations {
    fn released(&self) -> bool {
        self.root.released || self.areas.iter().any(|area| area.input.released)
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    /// The middle button is down and the pointer is still in the dead zone.
    Pressed,
    /// The pointer left the dead zone with the button down: releasing stops.
    Held,
    /// The button came up inside the dead zone: the next click stops.
    Toggled,
}

#[derive(Clone, Copy)]
struct Active {
    id: Id,
    viewport: ViewportId,
    anchor: Pos2,
    /// The previous frame's time. egui's `stable_dt` is only a frame-rate
    /// prediction when nothing asked for an immediate repaint, so the
    /// elapsed time is measured instead.
    time: f64,
    /// Kept while the pointer is outside the window, as in Chromium.
    velocity: Vec2,
    mode: Mode,
}

#[derive(Default)]
pub struct Autoscroll {
    active: Option<Active>,
    cancelled: bool,
}

/// What an [`egui::Plugin`] input hook kept from the views: a press or an
/// Escape that stops a gesture, which would otherwise also open what is
/// under the pointer or close what is behind the list.
#[derive(Default)]
struct Gate {
    /// The viewport whose gesture the next press or Escape stops.
    armed: Option<ViewportId>,
    /// Whether a press or Escape stopped the gesture since the last frame.
    stopped: bool,
    /// Buttons and keys pressed to stop it, whose repeats and releases are
    /// kept from the views as well.
    held: Vec<(ViewportId, Held)>,
}

#[derive(Clone, Copy, PartialEq)]
enum Held {
    Button(egui::PointerButton),
    Escape,
}

impl egui::Plugin for Gate {
    fn debug_name(&self) -> &'static str {
        "zapfast::autoscroll"
    }

    fn input_hook(&mut self, _ctx: &Context, input: &mut egui::RawInput) {
        let viewport = input.viewport_id;
        input.events.retain(|event| {
            let (held, pressed, repeat) = match *event {
                egui::Event::PointerButton {
                    button, pressed, ..
                } => (Held::Button(button), pressed, false),
                egui::Event::Key {
                    key: egui::Key::Escape,
                    pressed,
                    repeat,
                    ..
                } => (Held::Escape, pressed, repeat),
                _ => return true,
            };
            if let Some(index) = self.held.iter().position(|&kept| kept == (viewport, held)) {
                // A fresh press means the release never came, as when the
                // window lost focus first: it is not the stopping press's.
                if !pressed || !repeat {
                    self.held.swap_remove(index);
                }
                if !pressed || repeat {
                    return false;
                }
            }
            if pressed && self.armed == Some(viewport) {
                self.armed = None;
                self.stopped = true;
                self.held.push((viewport, held));
                return false;
            }
            true
        });
    }
}

fn gate<R>(ctx: &Context, change: impl FnOnce(&mut Gate) -> R) -> R {
    change(&mut ctx.plugin_or_default::<Gate>().lock())
}

impl Autoscroll {
    pub fn active(&self) -> bool {
        self.active.is_some()
    }

    /// Whether the gesture under way scrolls the list with this id.
    pub fn scrolls(&self, id: Id) -> bool {
        self.active.is_some_and(|active| active.id == id)
    }

    /// Focus events still arrive when a minimized/background window skips drawing.
    pub fn cancel_if_unfocused(&mut self, ctx: &Context) -> bool {
        let input = Input::read(ctx);
        if self
            .active
            .is_some_and(|active| active.viewport == input.viewport && !input.focused)
        {
            self.active = None;
            gate(ctx, |gate| gate.armed = None);
            true
        } else {
            false
        }
    }

    pub fn begin(&mut self, ctx: &Context, enabled: bool) {
        let root = Input::read(ctx);
        let stopped = gate(ctx, |gate| std::mem::take(&mut gate.stopped));
        self.cancelled = self.active.is_some_and(|active| {
            !enabled
                || stopped
                || root.cancel
                || (active.viewport == root.viewport && !root.focused)
        });
        if self.cancelled {
            self.active = None;
        }
        ctx.data_mut(|data| {
            data.insert_temp(
                Id::new(FRAME_ID),
                Observations {
                    enabled,
                    root,
                    ..Default::default()
                },
            )
        });
    }

    /// Returns whether the gesture owns this frame's scrolling, including its
    /// cancellation frame, so a previous trackpad glide cannot resume behind it.
    pub fn finish(&mut self, ctx: &Context, enabled: bool) -> bool {
        let owned = self.step(ctx, enabled);
        let armed = self.active.map(|active| active.viewport);
        gate(ctx, |gate| gate.armed = armed);
        owned
    }

    fn step(&mut self, ctx: &Context, enabled: bool) -> bool {
        let Some(frame) = ctx.data_mut(|data| data.remove_temp::<Observations>(Id::new(FRAME_ID)))
        else {
            self.active = None;
            return false;
        };
        let owned = self.active() || self.cancelled;
        if !enabled || self.cancelled {
            self.active = None;
            return owned;
        }
        if let Some(mut active) = self.active {
            let owner = frame
                .areas
                .iter()
                .find(|area| area.id == active.id && area.input.viewport == active.viewport);
            let Some(owner) = owner.filter(|area| {
                area.input.focused && area.rect.is_positive() && area.max != Vec2::ZERO
            }) else {
                self.active = None;
                return true;
            };
            if let Some(pointer) = owner.input.pointer {
                active.velocity = scroll_velocity(pointer - active.anchor);
            }
            if active.mode == Mode::Pressed && active.velocity != Vec2::ZERO {
                active.mode = Mode::Held;
            }
            if frame.released() {
                match active.mode {
                    Mode::Pressed => active.mode = Mode::Toggled,
                    Mode::Held => {
                        self.active = None;
                        return true;
                    }
                    Mode::Toggled => {}
                }
            }
            if frame.root.cancel || frame.areas.iter().any(|area| area.input.cancel) {
                self.active = None;
                return true;
            }
            let dt = (owner.input.time - active.time).clamp(0.0, MAX_STEP) as f32;
            active.time = owner.input.time;
            let mut velocity = active.velocity;
            if owner.max.x <= 0.0 {
                velocity.x = 0.0;
            }
            if owner.max.y <= 0.0 {
                velocity.y = 0.0;
            }
            // A list may move its own offset after drawing, as the message
            // list does to keep the rows on screen still while rows above
            // them change height, so each step starts from the stored offset.
            let mut state = egui::scroll_area::State::load(ctx, owner.id).unwrap_or(owner.state);
            let offset = (state.offset + velocity * dt).clamp(Vec2::ZERO, owner.max);
            if offset != state.offset {
                state.offset = offset;
                state.store(ctx, owner.id);
                // Every frame while the list moves, as the message list's
                // own edge and keyboard scrolling do.
                ctx.request_repaint();
            }
            if owner.input.viewport == ctx.viewport_id() {
                ctx.set_cursor_icon(cursor(velocity, owner.max));
            }
            self.active = Some(active);
            return true;
        }
        let Some(position) = frame.root.pointer.filter(|_| frame.root.middle) else {
            return false;
        };
        // Only the top layer under the press may own it, so a menu or dialog
        // keeps the list beneath it still.
        let layer = ctx
            .layer_id_at(position)
            .unwrap_or_else(LayerId::background);
        if claimed(ctx, &frame, layer, position) {
            return false;
        }
        // Nested lists finish first. An inner list owns the press if it can
        // scroll; otherwise the list around it may.
        let Some(owner) = frame.areas.iter().find(|area| {
            area.input.viewport == frame.root.viewport
                && area.layer == layer
                && area.input.focused
                && area.max != Vec2::ZERO
                && area.rect.contains(position)
        }) else {
            return false;
        };
        self.active = Some(Active {
            id: owner.id,
            viewport: frame.root.viewport,
            anchor: position,
            time: owner.input.time,
            velocity: Vec2::ZERO,
            mode: if frame.released() {
                Mode::Toggled
            } else {
                Mode::Pressed
            },
        });
        // The press is the gesture's now: the widget under it must not start
        // a drag with the middle button, such as a sweep over messages, and
        // egui's text selection, which follows any held button until it is
        // released, must not select what scrolls past.
        ctx.stop_dragging();
        if let Some(selection) = ctx.plugin_opt::<egui::text_selection::LabelSelectionState>() {
            selection.lock().clear_selection();
        }
        ctx.set_cursor_icon(cursor(Vec2::ZERO, owner.max));
        true
    }
}

/// Whether a widget under the press keeps it: a text field, for middle-click
/// paste, or anything that drags with any button, such as a slider, a seek
/// bar, a scroll bar or a panel edge. Text and message rows drag only with
/// the primary button.
fn claimed(ctx: &Context, frame: &Observations, layer: LayerId, position: Pos2) -> bool {
    let hits = ctx.interaction_snapshot(|snapshot| {
        snapshot
            .contains_pointer
            .iter()
            .copied()
            .collect::<Vec<_>>()
    });
    hits.into_iter()
        .filter_map(|id| ctx.read_response(id))
        .any(|hit| {
            hit.enabled()
                && hit.layer_id == layer
                && hit.interact_rect.contains(position)
                && (egui::text_edit::TextEditState::load(ctx, hit.id).is_some()
                    || (hit.sense.senses_drag()
                        && !frame.primary_drags.contains(&hit.id)
                        // A touch screen lets the area itself drag to scroll.
                        && !frame.areas.iter().any(|area| hit.id == area.id.with("area"))))
        })
}

/// Chromium's autoscroll velocity, in points per second, for the pointer's
/// offset from the anchor. Each axis has its own dead zone.
fn scroll_velocity(distance: Vec2) -> Vec2 {
    let axis = |distance: f32| {
        if distance.abs() <= DEAD_ZONE {
            0.0
        } else {
            distance.signum() * distance.abs().powf(EXPONENT) * MULTIPLIER
        }
    };
    Vec2::new(axis(distance.x), axis(distance.y))
}

/// Chromium's cursors on Linux: the four-way arrows until the list moves,
/// then the direction it is moving in, along the axes it can scroll.
fn cursor(velocity: Vec2, max: Vec2) -> egui::CursorIcon {
    use egui::CursorIcon::*;
    let sign = |velocity: f32, scrolls: bool| {
        if scrolls && velocity != 0.0 {
            velocity.signum() as i8
        } else {
            0
        }
    };
    match (sign(velocity.x, max.x > 0.0), sign(velocity.y, max.y > 0.0)) {
        (1, -1) => ResizeNorthEast,
        (-1, -1) => ResizeNorthWest,
        (0, -1) => ResizeNorth,
        (1, 1) => ResizeSouthEast,
        (-1, 1) => ResizeSouthWest,
        (0, 1) => ResizeSouth,
        (1, 0) => ResizeEast,
        (-1, 0) => ResizeWest,
        _ => AllScroll,
    }
}

/// Lets a middle press on a widget that drags only with the primary button,
/// such as selectable text or a message row, start autoscroll. It senses
/// dragging for selecting; without this, the press would belong to it like
/// a slider's.
pub fn primary_drag(ui: &Ui, response: &Response) {
    if ui.input(|input| input.pointer.button_pressed(egui::PointerButton::Middle)) {
        ui.ctx().data_mut(|data| {
            data.get_temp_mut_or_default::<Observations>(Id::new(FRAME_ID))
                .primary_drags
                .push(response.id)
        });
    }
}

/// Shows a vertical list as [`ScrollArea::show`] does and notes it for the
/// controller.
#[expect(clippy::disallowed_methods, reason = "the wrapper the rule points to")]
pub fn show<R>(
    ui: &mut Ui,
    area: ScrollArea,
    contents: impl FnOnce(&mut Ui) -> R,
) -> ScrollAreaOutput<R> {
    observe(ui, |ui| area.show(ui, contents))
}

/// Shows a vertical list as [`ScrollArea::show_rows`] does and notes it for
/// the controller.
#[expect(clippy::disallowed_methods, reason = "the wrapper the rule points to")]
pub fn show_rows<R>(
    ui: &mut Ui,
    area: ScrollArea,
    row_height: f32,
    rows: usize,
    contents: impl FnOnce(&mut Ui, std::ops::Range<usize>) -> R,
) -> ScrollAreaOutput<R> {
    observe(ui, |ui| area.show_rows(ui, row_height, rows, contents))
}

/// Keeps the area's id, sizing, animation and drawing, and only adds an
/// observation for the after-draw controller; no application state changes.
fn observe<R>(
    ui: &mut Ui,
    show: impl FnOnce(&mut Ui) -> ScrollAreaOutput<R>,
) -> ScrollAreaOutput<R> {
    let enabled = ui.ctx().data_mut(|data| {
        data.get_temp_mut_or_default::<Observations>(Id::new(FRAME_ID))
            .enabled
    });
    let layer = ui.layer_id();
    let clip = ui.clip_rect();
    let output = show(ui);
    if enabled {
        let max = Vec2::new(
            0.0,
            (output.content_size.y - output.inner_rect.height()).max(0.0),
        );
        let observation = Area {
            id: output.id,
            layer,
            rect: output.inner_rect.intersect(clip),
            max,
            input: Input::read(ui.ctx()),
            state: output.state,
        };
        ui.ctx().data_mut(|data| {
            data.get_temp_mut_or_default::<Observations>(Id::new(FRAME_ID))
                .areas
                .push(observation);
        });
    }
    output
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
