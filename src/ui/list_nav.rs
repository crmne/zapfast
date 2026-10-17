//! Keyboard and screen-reader navigation for long lists such as the messages.
//!
//! A list is one focusable container: Tab reaches it once, and the arrows
//! move a cursor among its items. The current item is exposed to AccessKit as
//! the container's active descendant rather than by moving egui focus, so
//! focus never names a row that a later frame skips laying out.

use egui::accesskit::{Rect as AkRect, Role};

/// A move of the cursor in a list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Previous,
    Next,
    First,
    Last,
    /// Up by about a page of rows.
    PageUp(usize),
    /// Down by about a page of rows.
    PageDown(usize),
}

/// New index for `step` in a list of `len` items from `current`; None for an
/// empty list. Without a cursor the list starts on its last (newest) item.
pub fn step(current: Option<usize>, len: usize, step: Step) -> Option<usize> {
    if len == 0 {
        return None;
    }
    let last = len - 1;
    // A cursor left past the end by removed items counts from the end.
    let Some(current) = current.map(|index| index.min(last)) else {
        return Some(last);
    };
    Some(match step {
        Step::Previous => current.saturating_sub(1),
        Step::Next => (current + 1).min(last),
        Step::First => 0,
        Step::Last => last,
        Step::PageUp(rows) => current.saturating_sub(rows.max(1)),
        Step::PageDown(rows) => (current + rows.max(1)).min(last),
    })
}

/// A rect in AccessKit's physical pixels.
fn bounds(rect: egui::Rect, ppp: f32) -> AkRect {
    AkRect {
        x0: f64::from(rect.min.x * ppp),
        y0: f64::from(rect.min.y * ppp),
        x1: f64::from(rect.max.x * ppp),
        y1: f64::from(rect.max.y * ppp),
    }
}

/// The focusable list widget: one Tab stop whose node is a labelled ListBox.
///
/// It senses focus only, never clicks, hovers or drags, so every widget in
/// the list keeps its pointer behaviour. Its node is created here, before the
/// rows, so rows that name it as their accessibility parent attach to it.
/// `label` is asked only while a screen reader is attached and the list is
/// drawn visibly, after `interact` has given the node its place in the tree:
/// a node made before that would hang from the window, and one made in a
/// hidden pass would have no role.
pub fn container(
    ui: &mut egui::Ui,
    id: egui::Id,
    rect: egui::Rect,
    label: impl FnOnce() -> String,
    len: usize,
) -> egui::Response {
    let response = ui.interact(rect, id, egui::Sense::focusable_noninteractive());
    outline_only(ui.ctx(), id);
    if ui.ctx().accesskit_node_builder(id, |_| ()).is_some() {
        let label = label();
        let ppp = ui.ctx().pixels_per_point();
        ui.ctx().accesskit_node_builder(id, |node| {
            node.set_role(Role::ListBox);
            node.set_label(label);
            node.set_size_of_set(len);
            node.set_bounds(bounds(rect, ppp));
        });
    }
    if response.has_focus() {
        // The arrows move the cursor, not egui's focus. Tab and Escape keep
        // their usual meaning.
        ui.memory_mut(|memory| {
            memory.set_focus_lock_filter(
                id,
                egui::EventFilter {
                    tab: false,
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    escape: false,
                },
            );
        });
    }
    response
}

/// The builder of a row's `Ui` in the list `list`: the row's node is an item
/// of the list's node rather than of the containers in between, and the
/// row's ids come from `salt` alone, as with `id_salt`. Unlike `id_salt`,
/// which leaves the row's own node id counting the rows drawn before it, the
/// node id is fixed too: a list that lays out only the rows near the view
/// would otherwise rename its current row whenever that count changes, and
/// a screen reader would hear the same item again.
pub fn row_builder(
    ui: &egui::Ui,
    salt: impl std::hash::Hash + std::fmt::Debug,
    list: egui::Id,
) -> egui::UiBuilder {
    // `id_salt` hashes its salt into an `IdSalt` before combining it with
    // the `Ui`'s id: the same here keeps the row's `Ui` id, and every id
    // made from it, as it was. Automatic ids inside the row now come from
    // the fixed id too, so they no longer shift either.
    egui::UiBuilder::new()
        .id(ui.make_persistent_id(egui::IdSalt::new(salt)))
        .accessibility_parent(list)
}

/// Sets `active_descendant` on the container to `row`. Callers pass only a
/// row whose node was created in this pass: AccessKit must never be told
/// about a node that is not in the tree it receives.
pub fn set_active(ctx: &egui::Context, container: egui::Id, row: Option<egui::Id>) {
    let Some(row) = row else {
        return;
    };
    ctx.accesskit_node_builder(container, |node| {
        node.set_active_descendant(row.accesskit_id());
    });
}

/// Turns a row Ui's node into a ListBoxOption at `index` of `len`. `selected`
/// is for lists with a selection; messages pass `None`.
pub fn mark_row(
    ctx: &egui::Context,
    row: egui::Id,
    label: String,
    index: usize,
    len: usize,
    rect: egui::Rect,
    selected: Option<bool>,
) {
    let ppp = ctx.pixels_per_point();
    ctx.accesskit_node_builder(row, |node| {
        node.set_role(Role::ListBoxOption);
        node.set_label(label);
        node.set_position_in_set(index);
        node.set_size_of_set(len);
        node.set_bounds(bounds(rect, ppp));
        if let Some(selected) = selected {
            node.set_selected(selected);
        }
    });
}

fn outline_only_id(id: egui::Id) -> egui::Id {
    id.with("outline-only")
}

/// Marks `id` as a widget whose focus ring is drawn only through
/// `theme::focus_outline`, around its current item, and never around the
/// widget's own rect. Registered each frame the widget is drawn.
pub fn outline_only(ctx: &egui::Context, id: egui::Id) {
    let frame = ctx.cumulative_frame_nr();
    ctx.data_mut(|data| data.insert_temp(outline_only_id(id), frame));
}

/// Whether `id` was registered with [`outline_only`] this frame.
pub fn is_outline_only(ctx: &egui::Context, id: egui::Id) -> bool {
    ctx.data(|data| data.get_temp::<u64>(outline_only_id(id))) == Some(ctx.cumulative_frame_nr())
}

/// Whether `id` was registered with [`outline_only`] this frame or the one
/// before: what a check at the start of a frame, before the widget is drawn
/// again, can see.
pub fn was_outline_only(ctx: &egui::Context, id: egui::Id) -> bool {
    let frame = ctx.cumulative_frame_nr();
    ctx.data(|data| data.get_temp::<u64>(outline_only_id(id)))
        .is_some_and(|registered| registered + 1 >= frame)
}

/// The keys a focused list consumes: the arrows, Home, End, PgUp and PgDn
/// without modifiers. Returns this frame's first one as a step, and cancels
/// the focus move egui chose for an arrow at the start of the frame. `page`
/// gives the rows a page key moves; it is asked only when PgUp or PgDn was
/// taken, so the frames without one do not work it out.
pub fn take_step(ctx: &egui::Context, page: impl FnOnce() -> usize) -> Option<Step> {
    let mut found = None;
    ctx.input_mut(|input| {
        for (key, step) in [
            (egui::Key::ArrowUp, Step::Previous),
            (egui::Key::ArrowDown, Step::Next),
            (egui::Key::Home, Step::First),
            (egui::Key::End, Step::Last),
            (egui::Key::PageUp, Step::PageUp(0)),
            (egui::Key::PageDown, Step::PageDown(0)),
        ] {
            if found.is_none() && crate::ui::keys::take_plain(input, key) {
                found = Some(step);
            }
        }
    });
    if found.is_some() {
        ctx.memory_mut(|memory| memory.move_focus(egui::FocusDirection::None));
    }
    // Outside the input lock: `page` may read the context.
    found.map(|step| match step {
        Step::PageUp(_) => Step::PageUp(page()),
        Step::PageDown(_) => Step::PageDown(page()),
        other => other,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The list's node hangs from the node of the Ui around it, and its
    /// label is worked out then; a hidden list has no node and no label.
    #[test]
    fn the_container_node_has_its_place_and_none_while_hidden() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let id = egui::Id::new("list");
        let list = id.accesskit_id();
        let run = |hidden: bool| {
            let mut asked = false;
            let mut outer = None;
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                let builder = egui::UiBuilder::new().id_salt("outer");
                let builder = if hidden { builder.invisible() } else { builder };
                ui.scope_builder(builder, |ui| {
                    // A node of its own, as the conversation around the
                    // message list has.
                    outer = Some(ui.unique_id().accesskit_id());
                    ui.ctx().accesskit_node_builder(ui.unique_id(), |node| {
                        node.set_role(Role::GenericContainer);
                    });
                    let rect = ui.max_rect();
                    container(
                        ui,
                        id,
                        rect,
                        || {
                            asked = true;
                            "Messages".to_owned()
                        },
                        3,
                    );
                });
            });
            output.textures_delta.clear();
            let tree = output
                .platform_output
                .accesskit_update
                .expect("accessibility tree");
            (tree, asked, outer.expect("drawn"))
        };
        let (tree, asked, outer) = run(false);
        assert!(asked);
        let parent = tree
            .nodes
            .iter()
            .find(|(_, node)| node.children().contains(&list))
            .map(|(id, _)| *id);
        assert_eq!(parent, Some(outer), "under the Ui around it");
        let (_, node) = tree.nodes.iter().find(|(node, _)| *node == list).unwrap();
        assert_eq!(node.role(), Role::ListBox);
        assert_eq!(node.label(), Some("Messages"));
        let (tree, asked, _) = run(true);
        assert!(!asked, "no label for a hidden list");
        assert!(
            tree.nodes.iter().all(|(node, _)| *node != list),
            "no node for a hidden list"
        );
    }

    /// The page size is worked out only for a page key.
    #[test]
    fn only_a_page_key_asks_for_the_page_size() {
        let ctx = egui::Context::default();
        let press = |key| {
            let mut asked = 0;
            let mut step = None;
            let mut output = ctx.run_ui(
                egui::RawInput {
                    events: vec![egui::Event::Key {
                        key,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    }],
                    ..Default::default()
                },
                |ui| {
                    step = take_step(ui.ctx(), || {
                        asked += 1;
                        7
                    });
                },
            );
            output.textures_delta.clear();
            (step, asked)
        };
        assert_eq!(press(egui::Key::ArrowDown), (Some(Step::Next), 0));
        assert_eq!(press(egui::Key::End), (Some(Step::Last), 0));
        assert_eq!(press(egui::Key::PageDown), (Some(Step::PageDown(7)), 1));
        assert_eq!(press(egui::Key::PageUp), (Some(Step::PageUp(7)), 1));
        assert_eq!(press(egui::Key::A), (None, 0));
    }

    #[test]
    fn steps_clamp_and_handle_empty_lists() {
        assert_eq!(step(None, 0, Step::Next), None);
        assert_eq!(
            step(None, 5, Step::Next),
            Some(4),
            "no cursor starts at the newest"
        );
        assert_eq!(step(Some(0), 5, Step::Previous), Some(0));
        assert_eq!(step(Some(4), 5, Step::Next), Some(4));
        assert_eq!(step(Some(2), 5, Step::First), Some(0));
        assert_eq!(step(Some(2), 5, Step::Last), Some(4));
        assert_eq!(step(Some(4), 5, Step::PageUp(3)), Some(1));
        assert_eq!(step(Some(1), 5, Step::PageDown(10)), Some(4));
        assert_eq!(
            step(Some(9), 5, Step::Previous),
            Some(3),
            "a stale index is clamped first"
        );
    }
}
