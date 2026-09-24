//! Deliberate Tab navigation for the main chat view. Message content and chat
//! rows remain pointer/accessibility targets, not stops in the primary cycle.

use egui::{Context, FocusDirection, Id, Response};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Stop {
    Composer,
    Send,
    Attach,
    Emoji,
    /// The chat header's Search, then the search pane's controls in reading
    /// order. The arrows walk its results from the field.
    ChatSearch,
    ChatSearchClose,
    ChatSearchDate,
    ChatSearchField,
    ChatSearchDay,
    Back,
    Profile,
    Sidebar,
    NewChat,
    Settings,
    Search,
    /// The Labels menu chip, when labels are not shown one chip each.
    Labels,
    All,
    Unread,
    Private,
    Favorites,
    Groups,
    Channels,
    Archived,
    Locked,
    /// One chip in the label row, by its position among the labels.
    Label(u8),
    ManageLabels,
}

#[derive(Clone, Copy, Debug)]
struct Move {
    from: Option<Stop>,
    steps: i32,
}

#[derive(Clone, Default)]
struct Order {
    frame: Option<u64>,
    controls: Vec<(Stop, Id)>,
    movement: Option<Move>,
}

fn order_id() -> Id {
    Id::new("main-chat-tab-order")
}

/// Attach a semantic position to an enabled main-view control. Using the real
/// response id keeps pointer clicks and keyboard activation on the same widget.
pub trait TabStop: Sized {
    fn tab_stop(self, stop: Stop) -> Self;
}

impl TabStop for Response {
    fn tab_stop(self, stop: Stop) -> Self {
        if self.enabled() {
            // `reveal_focus` has queued the scroll while this control's
            // scroll area is open. Re-layout so its ring is visible now.
            if self.gained_focus() && self.interact_rect != self.rect {
                self.ctx.request_discard("reveal primary Tab control");
            }
            self.ctx.data_mut(|data| {
                let order = data.get_temp_mut_or_default::<Order>(order_id());
                order.controls.retain(|(known, _)| *known != stop);
                order.controls.push((stop, self.id));
            });
        }
        self
    }
}

/// Intercept Tab before any widgets are registered. Merely consuming the key
/// is insufficient: egui already chose a focus direction during begin_pass.
pub fn begin(ctx: &Context, active: bool) {
    let mut pressed = false;
    let mut steps = 0;
    if active {
        ctx.input_mut(|input| {
            input.events.retain(|event| {
                if let egui::Event::Key {
                    key: egui::Key::Tab,
                    pressed: true,
                    modifiers,
                    ..
                } = event
                    && (!modifiers.any() || modifiers.shift_only())
                {
                    pressed = true;
                    steps += if modifiers.shift { -1 } else { 1 };
                    false
                } else {
                    true
                }
            });
        });
    }
    let focused = ctx.memory(|memory| memory.focused());
    let frame = ctx.cumulative_frame_nr();
    let target = ctx.data_mut(|data| {
        let order = data.get_temp_mut_or_default::<Order>(order_id());
        // Re-layout passes replay the same key. Always navigate from the
        // original source, not from the target chosen in the previous pass.
        if order.frame != Some(frame) {
            order.frame = Some(frame);
            order.movement = (active && pressed && steps != 0).then(|| Move {
                from: order
                    .controls
                    .iter()
                    .find(|(_, id)| Some(*id) == focused)
                    .map(|(stop, _)| *stop),
                steps,
            });
        }
        let target = active
            .then(|| {
                order
                    .movement
                    .and_then(|movement| next(&order.controls, movement))
            })
            .flatten();
        order.controls.clear();
        target
    });
    if active && (pressed || target.is_some()) {
        ctx.memory_mut(|memory| {
            memory.move_focus(FocusDirection::None);
            if let Some(target) = target {
                memory.request_focus(target);
            }
        });
    }
}

/// Reconcile against controls actually drawn this pass (a hidden sidebar or a
/// read-only chat can remove stops). Menus and dialogs keep egui's own order.
pub fn finish(ctx: &Context, active: bool) {
    let target = ctx.data_mut(|data| {
        let order = data.get_temp_mut_or_default::<Order>(order_id());
        order.controls.sort_by_key(|(stop, _)| *stop);
        order
            .movement
            .and_then(|movement| next(&order.controls, movement))
    });
    if active
        && !egui::Popup::is_any_open(ctx)
        && let Some(target) = target
    {
        ctx.memory_mut(|memory| {
            memory.move_focus(FocusDirection::None);
            memory.request_focus(target);
        });
    }
}

fn next(controls: &[(Stop, Id)], movement: Move) -> Option<Id> {
    if controls.is_empty() {
        return None;
    }
    let index = if let Some(index) = controls
        .iter()
        .position(|(stop, _)| Some(*stop) == movement.from)
    {
        index as i32 + movement.steps
    } else if movement.steps < 0 {
        let before = movement.from.map_or(controls.len(), |from| {
            controls.partition_point(|(stop, _)| *stop < from)
        });
        before as i32 + movement.steps
    } else {
        let after = movement.from.map_or(0, |from| {
            controls.partition_point(|(stop, _)| *stop <= from)
        });
        after as i32 + movement.steps.saturating_sub(1)
    };
    Some(controls[index.rem_euclid(controls.len() as i32) as usize].1)
}

#[cfg(test)]
pub fn stops(ctx: &Context) -> Vec<(Stop, Id)> {
    ctx.data(|data| {
        data.get_temp::<Order>(order_id())
            .unwrap_or_default()
            .controls
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn navigation_wraps_and_skips_missing_controls_in_both_directions() {
        let controls = [Stop::Composer, Stop::Send, Stop::Search]
            .map(|stop| (stop, Id::new(format!("{stop:?}"))));
        for (from, steps, expected) in [
            (Some(Stop::Composer), 1, 1),
            (Some(Stop::Search), 1, 0),
            (Some(Stop::Composer), -1, 2),
            (Some(Stop::Attach), 1, 2),
            (Some(Stop::Attach), -1, 1),
            (Some(Stop::Locked), 1, 0),
            (None, 1, 0),
            (None, -1, 2),
            (Some(Stop::Composer), 5, 2),
            (Some(Stop::Composer), -5, 1),
        ] {
            assert_eq!(
                next(&controls, Move { from, steps }),
                Some(controls[expected].1)
            );
        }
        assert_eq!(
            next(
                &[],
                Move {
                    from: None,
                    steps: 1
                }
            ),
            None
        );
    }

    #[test]
    fn a_relayout_does_not_advance_tab_twice() {
        let ctx = Context::default();
        let draw = |ui: &mut egui::Ui| {
            begin(ui.ctx(), true);
            for stop in [Stop::Composer, Stop::Send, Stop::Attach] {
                ui.button(format!("{stop:?}")).tab_stop(stop);
            }
            finish(ui.ctx(), true);
        };
        let mut output = ctx.run_ui(egui::RawInput::default(), draw);
        output.textures_delta.clear();
        let controls = stops(&ctx);
        ctx.memory_mut(|memory| memory.request_focus(controls[0].1));
        let mut passes = 0;
        let mut output = ctx.run_ui(
            egui::RawInput {
                events: vec![egui::Event::Key {
                    key: egui::Key::Tab,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }],
                ..Default::default()
            },
            |ui| {
                draw(ui);
                passes += 1;
                if passes == 1 {
                    ui.ctx().request_discard("test re-layout");
                }
            },
        );
        output.textures_delta.clear();
        assert_eq!(passes, 2);
        assert_eq!(ctx.memory(|memory| memory.focused()), Some(controls[1].1));
    }
}
