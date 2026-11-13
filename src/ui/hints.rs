//! Keyboard hints, as Vimium has them in a browser: with the setting on, F
//! labels everything on screen that a click reaches, and typing a label
//! clicks it, so the window needs no pointer. Escape leaves a text field
//! first (see `keys::handle`), which frees the letters for this.
//!
//! The targets are the widgets the last pass registered as clickable, and
//! the click is a real pointer press and release at a point egui's own hit
//! test gives to that widget, injected by [`Clicker`] before the next pass:
//! whatever the pointer can click, the labels can, with no list of
//! controls to keep in step with the views.

use egui::text::{LayoutJob, TextFormat};
use egui::{Align2, Color32, Context, Id, Key, LayerId, Order, Pos2, Rect, Stroke, vec2};

use crate::theme::{self, Palette};

/// The letters labels are made of: the home row first, then those nearest
/// it, as Vimium's are.
const ALPHABET: &str = "sadfjklewcmpgh";

/// A widget covering more of the window than this is a backdrop that
/// dismisses what is above it, not a control: labelling it says nothing.
const BACKDROP_SHARE: f32 = 0.5;

/// How far inside a widget's edge the points tried for its click lie.
const INSET: f32 = 3.0;

/// One labelled widget.
#[derive(Clone, Debug, PartialEq)]
struct Target {
    id: Id,
    label: String,
    /// Where the label is drawn, and how it hangs from there.
    anchor: Pos2,
    align: Align2,
}

/// The labels on screen and what has been typed of one.
#[derive(Clone, Debug, Default)]
struct Session {
    targets: Vec<Target>,
    typed: String,
}

fn session_id() -> Id {
    Id::new("keyboard-hints")
}

fn click_id() -> Id {
    Id::new("keyboard-hint-click")
}

/// Whether the labels are on screen.
pub fn active(ctx: &Context) -> bool {
    ctx.data(|data| data.get_temp::<Session>(session_id()).is_some())
}

/// Takes the labels away without clicking anything.
pub fn cancel(ctx: &Context) {
    ctx.data_mut(|data| data.remove_temp::<Session>(session_id()));
    ctx.request_repaint();
}

/// Labels what the last pass drew as clickable. Returns false, showing
/// nothing, when there is nothing to click.
pub fn start(ctx: &Context) -> bool {
    let found = targets(ctx);
    if found.is_empty() {
        return false;
    }
    let labels = labels(found.len());
    let targets = found
        .into_iter()
        .zip(labels)
        .map(|(found, label)| Target {
            id: found.id,
            label,
            anchor: found.anchor,
            align: found.align,
        })
        .collect();
    ctx.data_mut(|data| {
        data.insert_temp(
            session_id(),
            Session {
                targets,
                typed: String::new(),
            },
        );
    });
    ctx.request_repaint();
    true
}

/// While the labels are up, takes this pass's keys: letters narrow them
/// down, Backspace takes the last letter back, and Escape or any other
/// key takes them away. A full label clicks its widget. A pointer press or
/// a scroll takes the labels away too, and goes on to do what it does.
///
/// Returns true when the keys were taken, and no other shortcut should see
/// them.
pub fn handle(ctx: &Context) -> bool {
    let Some(mut session) = ctx.data(|data| data.get_temp::<Session>(session_id())) else {
        return false;
    };
    let moved_on = ctx.input(|input| {
        input.pointer.any_pressed()
            || input.events.iter().any(|event| {
                matches!(
                    event,
                    egui::Event::MouseWheel { .. } | egui::Event::WindowFocused(false)
                )
            })
    });
    if moved_on || egui::Popup::is_any_open(ctx) {
        cancel(ctx);
        return false;
    }
    let typed = session.typed.clone();
    let mut done = false;
    let mut left = false;
    ctx.input_mut(|input| {
        input.events.retain(|event| match event {
            egui::Event::Key {
                key,
                pressed,
                modifiers,
                ..
            } => {
                // What follows the key that took the labels away is the
                // shortcuts', and so are releases.
                if left || !pressed {
                    return true;
                }
                if done {
                    return false;
                }
                if *key == Key::Escape {
                    done = true;
                } else if *key == Key::Backspace {
                    session.typed.pop();
                } else if let Some(letter) = letter(*key)
                    && !modifiers.command
                    && !modifiers.ctrl
                    && !modifiers.alt
                {
                    if ALPHABET.contains(letter) {
                        session.typed.push(letter);
                        if !session
                            .targets
                            .iter()
                            .any(|target| target.label.starts_with(&session.typed))
                        {
                            // A letter no label goes on with is a slip.
                            session.typed.pop();
                        }
                    }
                } else {
                    // Another key ends the labels and does what it does.
                    left = true;
                    return true;
                }
                false
            }
            // The letters typed for a label are not text.
            egui::Event::Text(_) | egui::Event::Ime(_) => left,
            _ => true,
        });
    });
    if done || left {
        cancel(ctx);
        return !left;
    }
    if let Some(target) = session
        .targets
        .iter()
        .find(|target| target.label == session.typed)
    {
        // The widget may have moved since it was labelled. One that is gone
        // is not clicked: whatever took its place was never labelled.
        let found = targets(ctx).into_iter().find(|found| found.id == target.id);
        let Some(found) = found else {
            cancel(ctx);
            return true;
        };
        ctx.data_mut(|data| {
            data.remove_temp::<Session>(session_id());
            data.insert_temp(click_id(), found.point);
        });
        ctx.request_repaint();
        return true;
    }
    // A pass that typed nothing changes nothing, and asks for no other.
    if session.typed != typed {
        ctx.data_mut(|data| data.insert_temp(session_id(), session));
        ctx.request_repaint();
    }
    true
}

/// The lowercase letter a key types, for the letter keys.
fn letter(key: Key) -> Option<char> {
    let name = key.name();
    let mut chars = name.chars();
    match (chars.next(), chars.next()) {
        (Some(letter), None) if letter.is_ascii_alphabetic() => Some(letter.to_ascii_lowercase()),
        _ => None,
    }
}

/// Draws the labels that still match what has been typed, above everything.
pub fn paint(ctx: &Context, palette: &Palette) {
    let Some(session) = ctx.data(|data| data.get_temp::<Session>(session_id())) else {
        return;
    };
    let painter = ctx.layer_painter(LayerId::new(Order::Debug, session_id()));
    let font = theme::semibold(12.0);
    let typed_colour = palette.on_accent.gamma_multiply(0.55);
    for target in session
        .targets
        .iter()
        .filter(|target| target.label.starts_with(&session.typed))
    {
        let (typed, rest) = target.label.split_at(session.typed.len());
        let mut job = LayoutJob::default();
        let format = |color: Color32| TextFormat {
            font_id: font.clone(),
            color,
            ..Default::default()
        };
        job.append(&typed.to_uppercase(), 0.0, format(typed_colour));
        job.append(&rest.to_uppercase(), 0.0, format(palette.on_accent));
        let galley = painter.layout_job(job);
        let size = galley.size() + vec2(8.0, 2.0);
        let rect = target.align.anchor_size(target.anchor, size);
        painter.rect(
            rect,
            4.0,
            palette.accent,
            Stroke::new(1.0, palette.shadow),
            egui::StrokeKind::Outside,
        );
        painter.galley(rect.min + vec2(4.0, 1.0), galley, palette.on_accent);
    }
}

/// Clicks where a finished label asked, before the next pass reads its
/// input: a pointer move, press and release in one pass, which egui takes
/// for a click, then the pointer back where it was in the pass after.
#[derive(Default)]
pub struct Clicker {
    /// Where the real pointer was before the click, once one was made.
    restore: Option<Option<Pos2>>,
}

impl egui::plugin::Plugin for Clicker {
    fn debug_name(&self) -> &'static str {
        "zapfast-keyboard-hints"
    }

    fn input_hook(&mut self, ctx: &Context, input: &mut egui::RawInput) {
        // First, so a real move in the same pass has the last word.
        if let Some(previous) = self.restore.take() {
            input.events.insert(
                0,
                match previous {
                    Some(pos) => egui::Event::PointerMoved(pos),
                    None => egui::Event::PointerGone,
                },
            );
        }
        let Some(point) = ctx.data_mut(|data| data.remove_temp::<Pos2>(click_id())) else {
            return;
        };
        let modifiers = ctx.input(|input| input.modifiers);
        let button = |pressed| egui::Event::PointerButton {
            pos: point,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers,
        };
        input.events.push(egui::Event::PointerMoved(point));
        input.events.push(button(true));
        input.events.push(button(false));
        self.restore = Some(ctx.input(|input| input.pointer.latest_pos()));
        ctx.request_repaint();
    }
}

/// A clickable widget, where a click reaches it and where its label goes.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Found {
    id: Id,
    point: Pos2,
    anchor: Pos2,
    align: Align2,
}

/// The widgets the last pass drew that a click at some point of them would
/// reach, in reading order: top to bottom, then left to right.
fn targets(ctx: &Context) -> Vec<Found> {
    let layers: Vec<(LayerId, Vec<egui::WidgetRect>)> = ctx.viewport(|viewport| {
        viewport
            .prev_pass
            .widgets
            .layers()
            .map(|(layer, widgets)| (*layer, widgets.to_vec()))
            .collect()
    });
    let screen = ctx.content_rect();
    let mut found = Vec::new();
    for (layer, widgets) in layers {
        let transform = ctx.layer_transform_to_global(layer).unwrap_or_default();
        let rects: Vec<Rect> = widgets
            .iter()
            .map(|widget| transform * widget.interact_rect)
            .collect();
        for (index, widget) in widgets.iter().enumerate() {
            let rect = rects[index].intersect(screen);
            if !widget.enabled
                || !widget.visible
                || !widget.sense.senses_click()
                || !rect.is_positive()
                || rect.area() > screen.area() * BACKDROP_SHARE
                // Text that can be swept with a drag, sliders and handles
                // are not things to click; a text field is, to type in it.
                || (widget.sense.senses_drag()
                    && egui::TextEdit::load_state(ctx, widget.id).is_none())
            {
                continue;
            }
            // Widgets registered later in a layer sit on top and take the
            // click where they overlap this one.
            let covered = |point: Pos2| {
                widgets[index + 1..]
                    .iter()
                    .zip(&rects[index + 1..])
                    .any(|(above, rect)| {
                        above.enabled
                            && (above.sense.senses_click() || above.sense.senses_drag())
                            && rect.contains(point)
                    })
            };
            let Some(point) = points(rect)
                .into_iter()
                .find(|point| !covered(*point) && ctx.layer_id_at(*point) == Some(layer))
            else {
                continue;
            };
            let (anchor, align) = if point == rect.center() {
                (rect.left_top(), Align2::LEFT_TOP)
            } else {
                (point, Align2::CENTER_CENTER)
            };
            found.push(Found {
                id: widget.id,
                point,
                anchor,
                align,
            });
        }
    }
    found.sort_by(|a, b| {
        (a.anchor.y, a.anchor.x)
            .partial_cmp(&(b.anchor.y, b.anchor.x))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    found
}

/// The points of `rect` tried for a click, the centre first, then just
/// inside its corners and the middles of its edges.
fn points(rect: Rect) -> Vec<Pos2> {
    let inner = rect.shrink2(vec2(
        INSET.min(rect.width() / 2.0),
        INSET.min(rect.height() / 2.0),
    ));
    vec![
        rect.center(),
        inner.left_top(),
        inner.right_top(),
        inner.left_bottom(),
        inner.right_bottom(),
        inner.center_top(),
        inner.center_bottom(),
        inner.left_center(),
        inner.right_center(),
    ]
}

/// `count` labels, none the start of another, as short as `count` allows:
/// a label is split into the alphabet's letters, the earliest first, until
/// there are enough.
fn labels(count: usize) -> Vec<String> {
    let alphabet: Vec<char> = ALPHABET.chars().collect();
    let mut labels = vec![String::new()];
    let mut next = 0;
    while labels.len() - next < count || labels.len() == 1 {
        let parent = labels[next].clone();
        next += 1;
        labels.extend(alphabet.iter().map(|letter| format!("{parent}{letter}")));
    }
    let mut labels = labels.split_off(next);
    labels.truncate(count);
    labels
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_are_short_and_none_starts_another() {
        assert_eq!(labels(3), ["s", "a", "d"]);
        assert!(labels(14).iter().all(|label| label.len() == 1));
        for count in [1, 13, 14, 15, 40, 200] {
            let made = labels(count);
            assert_eq!(made.len(), count);
            for (i, a) in made.iter().enumerate() {
                for (j, b) in made.iter().enumerate() {
                    assert!(i == j || !b.starts_with(a.as_str()), "{a} starts {b}");
                }
            }
        }
        // One more than a letter each splits only the last letter's place.
        let fifteen = labels(15);
        assert_eq!(fifteen.iter().filter(|label| label.len() == 1).count(), 13);
    }

    fn key(key: Key) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }
    }

    /// Up to two buttons side by side over a window, and the clicks each
    /// has received.
    struct Bench {
        ctx: Context,
        clicks: [u32; 2],
        /// How many of the buttons the next pass draws.
        shown: usize,
    }

    impl Bench {
        fn new() -> Self {
            let ctx = Context::default();
            ctx.add_plugin(Clicker::default());
            Self {
                ctx,
                clicks: [0, 0],
                shown: 2,
            }
        }

        /// A pass of `events`. Returns whether the labels took its keys, or
        /// with `labels` set, whether they came up, and the events left for
        /// the shortcuts after them.
        fn pass(&mut self, events: Vec<egui::Event>, labels: bool) -> (bool, Vec<egui::Event>) {
            let mut taken = false;
            let mut left = Vec::new();
            let (clicks, shown) = (&mut self.clicks, self.shown);
            let mut output = self.ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(400.0, 300.0))),
                    events,
                    ..Default::default()
                },
                |ui| {
                    taken = if labels {
                        start(ui.ctx())
                    } else {
                        handle(ui.ctx())
                    };
                    left = ui.input(|input| input.events.clone());
                    ui.horizontal(|ui| {
                        for (index, clicks) in clicks.iter_mut().enumerate().take(shown) {
                            if ui.button(format!("Button {index}")).clicked() {
                                *clicks += 1;
                            }
                        }
                    });
                    // Text that can be selected is not labelled.
                    ui.add(egui::Label::new("Selectable").selectable(true));
                },
            );
            output.textures_delta.clear();
            (taken, left)
        }

        fn labels(&self) -> Vec<String> {
            self.ctx
                .data(|data| data.get_temp::<Session>(session_id()))
                .map(|session| session.targets.into_iter().map(|t| t.label).collect())
                .unwrap_or_default()
        }
    }

    #[test]
    fn typing_a_label_clicks_its_widget_and_puts_the_pointer_back() {
        let mut bench = Bench::new();
        // The real pointer rests where nothing is.
        let resting = Pos2::new(350.0, 250.0);
        let (taken, _) = bench.pass(vec![egui::Event::PointerMoved(resting)], false);
        assert!(!taken, "nothing is taken before the labels are up");
        assert!(bench.pass(Vec::new(), true).0 && active(&bench.ctx));
        assert_eq!(bench.labels(), ["s", "a"], "the two buttons, left to right");
        // A letter no label has is a slip, and changes nothing.
        assert!(bench.pass(vec![key(Key::Z)], false).0 && active(&bench.ctx));
        // The second button's label: the click is made before the next pass.
        bench.pass(vec![key(Key::A)], false);
        assert!(!active(&bench.ctx), "a full label takes the labels away");
        bench.pass(Vec::new(), false);
        assert_eq!(bench.clicks, [0, 1]);
        // The pointer goes back, and nothing else is clicked.
        bench.pass(Vec::new(), false);
        assert_eq!(bench.clicks, [0, 1]);
        assert_eq!(
            bench.ctx.input(|input| input.pointer.latest_pos()),
            Some(resting)
        );
    }

    #[test]
    fn a_label_whose_widget_is_gone_clicks_nothing() {
        let mut bench = Bench::new();
        bench.pass(Vec::new(), false);
        bench.pass(Vec::new(), true);
        assert_eq!(bench.labels(), ["s", "a"]);
        // The second button goes before its label is typed.
        bench.shown = 1;
        bench.pass(Vec::new(), false);
        bench.pass(vec![key(Key::A)], false);
        assert!(!active(&bench.ctx));
        bench.shown = 2;
        bench.pass(Vec::new(), false);
        bench.pass(Vec::new(), false);
        assert_eq!(bench.clicks, [0, 0]);
    }

    #[test]
    fn escape_and_other_keys_take_the_labels_away() {
        let mut bench = Bench::new();
        bench.pass(Vec::new(), false);
        bench.pass(Vec::new(), true);
        assert!(
            bench.pass(vec![key(Key::Escape)], false).0,
            "Escape goes no further"
        );
        assert!(!active(&bench.ctx));
        bench.pass(Vec::new(), true);
        // Another key, and every key after it, is left for the shortcuts.
        let (taken, left) = bench.pass(vec![key(Key::Enter), key(Key::K)], false);
        assert!(!taken);
        assert!(!active(&bench.ctx));
        assert_eq!(left, [key(Key::Enter), key(Key::K)]);
        bench.pass(Vec::new(), false);
        assert_eq!(bench.clicks, [0, 0]);
    }

    /// While the labels wait for a key, a pass without one asks for no
    /// other: the window stays idle.
    #[test]
    fn waiting_labels_ask_for_no_repaint() {
        let mut bench = Bench::new();
        bench.pass(Vec::new(), false);
        bench.pass(Vec::new(), true);
        bench.pass(Vec::new(), false);
        let mut repaint = false;
        let mut output = bench.ctx.run_ui(egui::RawInput::default(), |ui| {
            assert!(handle(ui.ctx()));
        });
        output.textures_delta.clear();
        if let Some(viewport) = output.viewport_output.values().next() {
            repaint = viewport.repaint_delay.is_zero();
        }
        assert!(!repaint, "an idle pass asked for another");
    }
}
