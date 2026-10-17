//! Frame-by-frame focus traces: what AccessKit is told has focus in every
//! frame of a keyboard scenario. A screen reader speaks each change of that
//! focus, so a frame that reports the window (the tree's root), or a key
//! that moves focus twice (to a node and on, or away and straight back), is
//! heard as "ZapFast window" or as an extra item, however briefly it lasts.

use egui::accesskit::{NodeId, TreeUpdate};

use super::tests::{app, press_key, render};
use crate::app::App;
use crate::model::Action;
use crate::ui::focus::{Stop, control};

/// One frame as a screen reader sees it.
#[derive(Clone, Debug)]
struct Frame {
    step: String,
    /// The user action this frame belongs to: its key frame and the quiet
    /// frames after it.
    action: usize,
    /// The tree's focus.
    focus: NodeId,
    /// Where the reader is: the focused node's active descendant, if it has
    /// one in this update, else the focused node.
    effective: NodeId,
    /// The effective node's ancestors, nearest first. A reader announces
    /// the ancestors that changed, so a node moved under another parent is
    /// heard again.
    ancestors: Vec<NodeId>,
    /// The effective node's role and label, for the failure dump.
    name: String,
    /// What a reader says of the effective node: a change while it keeps
    /// focus is read out again.
    spoken: String,
    /// egui's focused widget after the frame.
    egui: Option<egui::Id>,
    /// egui's focused widget at the end of each pass's UI, before egui's own
    /// end of pass.
    passes: Vec<Option<egui::Id>>,
}

impl Frame {
    fn heard(&self) -> (NodeId, &[NodeId], &str) {
        (self.effective, &self.ancestors, &self.spoken)
    }
}

struct Trace {
    app: App,
    ctx: egui::Context,
    frames: Vec<Frame>,
    window_focused: bool,
    action: usize,
    time: f64,
}

fn root() -> NodeId {
    egui::accesskit_root_id().accesskit_id()
}

fn describe(tree: &TreeUpdate, id: NodeId) -> String {
    if id == root() {
        return "WINDOW".to_owned();
    }
    tree.nodes
        .iter()
        .find(|(found, _)| *found == id)
        .map_or_else(
            || "missing node".to_owned(),
            |(_, node)| {
                let label: String = node.label().unwrap_or_default().chars().take(24).collect();
                format!("{:?} {label:?}", node.role())
            },
        )
}

fn spoken(tree: &TreeUpdate, id: NodeId) -> String {
    tree.nodes
        .iter()
        .find(|(found, _)| *found == id)
        .map(|(_, node)| {
            format!(
                "{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}",
                node.role(),
                node.label(),
                node.value(),
                node.description(),
                node.is_selected(),
                node.toggled(),
                node.position_in_set(),
                node.size_of_set()
            )
        })
        .unwrap_or_default()
}

fn effective(tree: &TreeUpdate) -> NodeId {
    let node = |id: NodeId| tree.nodes.iter().find(|(found, _)| *found == id);
    node(tree.focus)
        .and_then(|(_, focused)| focused.active_descendant())
        .filter(|active| node(*active).is_some())
        .unwrap_or(tree.focus)
}

fn ancestors(tree: &TreeUpdate, mut id: NodeId) -> Vec<NodeId> {
    let mut chain = Vec::new();
    while let Some((parent, _)) = tree
        .nodes
        .iter()
        .find(|(_, node)| node.children().contains(&id))
    {
        chain.push(*parent);
        id = *parent;
        if chain.len() > 64 {
            break;
        }
    }
    chain
}

impl Trace {
    fn new(app: App) -> Self {
        let ctx = egui::Context::default();
        let mut app = app;
        app.attach(&ctx);
        ctx.enable_accesskit();
        let mut trace = Self {
            app,
            ctx,
            frames: Vec::new(),
            window_focused: true,
            action: 0,
            time: 0.0,
        };
        trace.settle("start", 4);
        trace
    }

    /// Runs one frame with `events`, as eframe does: the app's logic and its
    /// UI in every pass, with time moving on at 60 frames a second.
    fn frame(&mut self, step: &str, events: Vec<egui::Event>) {
        self.time += 1.0 / 60.0;
        let mut input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1180.0, 780.0),
            )),
            time: Some(self.time),
            events,
            focused: self.window_focused,
            ..Default::default()
        };
        input
            .viewports
            .get_mut(&egui::ViewportId::ROOT)
            .expect("the root viewport")
            .focused = Some(self.window_focused);
        let mut passes = Vec::new();
        let app = &mut self.app;
        let mut output = self.ctx.run_ui(input, |ui| {
            let ctx = ui.ctx().clone();
            app.background_frame(&ctx);
            app.frame_ui(ui);
            passes.push(ctx.memory(|memory| memory.focused()));
        });
        output.textures_delta.clear();
        let tree = output
            .platform_output
            .accesskit_update
            .expect("accessibility tree");
        let effective = effective(&tree);
        self.frames.push(Frame {
            step: step.to_owned(),
            action: self.action,
            focus: tree.focus,
            effective,
            ancestors: ancestors(&tree, effective),
            name: describe(&tree, effective),
            spoken: spoken(&tree, effective),
            egui: self.ctx.memory(|memory| memory.focused()),
            passes,
        });
    }

    fn settle(&mut self, step: &str, frames: usize) {
        for _ in 0..frames {
            self.frame(step, Vec::new());
        }
    }

    /// A key pressed in one frame and released in the next, then a few
    /// quiet frames.
    fn press(&mut self, step: &str, key: egui::Key, modifiers: egui::Modifiers) {
        self.action += 1;
        self.frame(step, key_press(key, modifiers));
        self.frame(
            step,
            vec![egui::Event::Key {
                key,
                physical_key: Some(key),
                pressed: false,
                repeat: false,
                modifiers,
            }],
        );
        self.settle(step, 4);
    }

    /// A key held down: a press, then `repeats` repeats, every other frame
    /// as a keyboard repeats at about 30 a second, each its own action as
    /// the reader hears it.
    fn hold(&mut self, step: &str, key: egui::Key, repeats: usize) {
        for repeat in 0..=repeats {
            self.action += 1;
            self.frame(
                step,
                vec![egui::Event::Key {
                    key,
                    physical_key: Some(key),
                    pressed: true,
                    repeat: repeat > 0,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
            self.frame(step, Vec::new());
        }
        self.settle(step, 4);
    }

    /// The window loses or regains focus, as winit reports it.
    fn window(&mut self, step: &str, focused: bool) {
        self.action += 1;
        self.window_focused = focused;
        self.frame(step, vec![egui::Event::WindowFocused(focused)]);
        self.settle(step, 4);
    }

    fn mark(&self) -> usize {
        self.frames.len()
    }

    fn last(&self) -> &Frame {
        self.frames.last().expect("a frame ran")
    }

    fn dump(&self, from: usize) -> String {
        self.frames[from..]
            .iter()
            .enumerate()
            .map(|(index, frame)| {
                format!(
                    "  #{:<3} {:<14} reader {} {:<40} up {:?} egui {:?} passes {:?}{}",
                    from + index,
                    frame.step,
                    frame.effective.0 % 100_000,
                    frame.name,
                    frame
                        .ancestors
                        .iter()
                        .map(|id| id.0 % 10_000)
                        .collect::<Vec<_>>(),
                    frame.egui,
                    frame.passes,
                    if frame.focus == root() {
                        "  <-- WINDOW"
                    } else {
                        ""
                    }
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Failures from frame `from` on: the window reported as focus, one
    /// action heard as more than one focus change, or the item focus is on
    /// given a new node id, which a reader hears as a move to it.
    fn failures(&self, from: usize) -> Vec<String> {
        let mut failures = Vec::new();
        for (index, frame) in self.frames.iter().enumerate().skip(from) {
            if frame.focus == root() {
                failures.push(format!(
                    "frame #{index} ({}): focus is the window",
                    frame.step
                ));
            }
        }
        let mut changes = 0;
        for index in from.max(1)..self.frames.len() {
            let (before, frame) = (&self.frames[index - 1], &self.frames[index]);
            if frame.action != before.action {
                changes = 0;
            }
            if frame.effective != before.effective
                && frame.spoken == before.spoken
                && frame.ancestors == before.ancestors
            {
                failures.push(format!(
                    "frame #{index} ({}): the same item under a new node id",
                    frame.step
                ));
            }
            if frame.heard() != before.heard() {
                changes += 1;
                if changes == 2 {
                    failures.push(format!(
                        "frame #{index} ({}): a second focus change for one action",
                        frame.step
                    ));
                }
            }
        }
        failures
    }

    fn assert_clean(&self, from: usize, scenario: &str) {
        let failures = self.failures(from);
        assert!(
            failures.is_empty(),
            "{scenario}:\n{}\n{}",
            failures.join("\n"),
            self.dump(from.saturating_sub(1))
        );
    }

    /// No focus change at all from frame `from` on.
    fn assert_unchanged(&self, from: usize, scenario: &str) {
        let before = &self.frames[from - 1];
        assert!(
            self.frames[from..]
                .iter()
                .all(|frame| frame.heard() == before.heard() && frame.focus != root()),
            "{scenario}: focus moved\n{}",
            self.dump(from - 1)
        );
    }
}

/// The events of a key press as Windows delivers them: Alt with a digit
/// also types the digit as text, which a shortcut must not let through.
fn key_press(key: egui::Key, modifiers: egui::Modifiers) -> Vec<egui::Event> {
    let mut events = vec![egui::Event::Key {
        key,
        physical_key: Some(key),
        pressed: true,
        repeat: false,
        modifiers,
    }];
    let digit = key.name();
    if modifiers.alt && digit.len() == 1 && digit.chars().all(|c| c.is_ascii_digit()) {
        events.push(egui::Event::Text(digit.to_owned()));
    }
    events
}

fn alt() -> egui::Modifiers {
    if cfg!(target_os = "macos") {
        egui::Modifiers::CTRL
    } else {
        egui::Modifiers::ALT
    }
}

fn long_chat(app: &mut App, count: usize) {
    let chat = app.open_chat.clone().expect("the demo opens a chat");
    let conversation = app.conversations.get_mut(&chat).expect("its messages");
    let template = conversation.messages.last().expect("a message").clone();
    conversation.messages = (0..count)
        .map(|index| crate::model::Message {
            id: format!("long-{index}"),
            timestamp: template.timestamp - (count - index) as i64 * 60,
            content: crate::model::Content::text(format!("Fixture line {index}")),
            ..template.clone()
        })
        .collect();
}

/// Gives focus to the main view's `stop` as the keys reach it: the lists
/// by their shortcuts, the message field as the demo starts, anything else
/// as Tab would.
fn focus_stop(trace: &mut Trace, stop: Stop) {
    match stop {
        Stop::Messages => trace.press("Alt+2", egui::Key::Num2, alt()),
        Stop::Composer => {}
        _ => {
            let id = control(&trace.ctx, stop).expect("drawn");
            trace.ctx.memory_mut(|memory| memory.request_focus(id));
            trace.settle("focus", 3);
        }
    }
    let id = control(&trace.ctx, stop).expect("drawn");
    assert_eq!(trace.last().egui, Some(id), "{stop:?} took focus");
}

/// The demo with no chat open and `stop` focused.
fn no_chat_open(stop: Stop) -> Trace {
    let mut trace = Trace::new(app());
    trace.app.actions.push(Action::CloseChat);
    trace.settle("close the chat", 3);
    focus_stop(&mut trace, stop);
    trace
}

#[test]
fn arrowing_the_message_list_never_reports_the_window() {
    let mut trace = Trace::new(app());
    focus_stop(&mut trace, Stop::Messages);
    let from = trace.mark();
    for _ in 0..12 {
        trace.press("Up", egui::Key::ArrowUp, egui::Modifiers::NONE);
    }
    for _ in 0..4 {
        trace.press("Down", egui::Key::ArrowDown, egui::Modifiers::NONE);
    }
    trace.assert_clean(from, "arrows in the message list");
}

/// Home and End reach messages far from the view, which are not laid out
/// yet, and the page keys move the rows laid out above the current one: the
/// reader still hears each message once.
#[test]
fn a_long_message_list_never_reports_the_window() {
    let mut app = app();
    long_chat(&mut app, 400);
    let mut trace = Trace::new(app);
    focus_stop(&mut trace, Stop::Messages);
    let from = trace.mark();
    for (step, key) in [
        ("Home", egui::Key::Home),
        ("Down", egui::Key::ArrowDown),
        ("Down", egui::Key::ArrowDown),
        ("End", egui::Key::End),
        ("Up", egui::Key::ArrowUp),
        ("PageUp", egui::Key::PageUp),
        ("PageUp", egui::Key::PageUp),
        ("PageDown", egui::Key::PageDown),
        ("Home", egui::Key::Home),
        ("Up", egui::Key::ArrowUp),
        ("End", egui::Key::End),
    ] {
        trace.press(step, key, egui::Modifiers::NONE);
    }
    // Held, the arrows move a message a frame and scroll as they go.
    trace.hold("Up held", egui::Key::ArrowUp, 40);
    trace.hold("Down held", egui::Key::ArrowDown, 20);
    trace.assert_clean(from, "Home, End and the page keys in a long chat");
}

#[test]
fn alt_with_a_digit_also_delivers_its_text() {
    assert_eq!(
        key_press(egui::Key::Num2, egui::Modifiers::ALT)[1],
        egui::Event::Text("2".to_owned())
    );
    assert_eq!(
        key_press(egui::Key::ArrowUp, egui::Modifiers::NONE).len(),
        1
    );
}

/// Alt+2 in the message field moves to the message list and types nothing,
/// though Windows delivers its digit as text too.
#[test]
fn alt_2_in_the_message_field_types_nothing() {
    let mut trace = Trace::new(app());
    focus_stop(&mut trace, Stop::Composer);
    let draft = trace.app.composer.clone();
    trace.press("Alt+2", egui::Key::Num2, alt());
    let chat = trace.app.open_chat.clone().expect("the demo opens a chat");
    assert_eq!(
        trace.last().egui,
        Some(crate::ui::conversation::messages_list_id(&chat))
    );
    assert_eq!(trace.app.composer, draft, "nothing typed");
}

/// With the pointer, no keyboard navigation and no screen reader, Escape
/// lets go of focus as egui always did: a clicked field loses its caret.
#[test]
fn escape_with_the_pointer_lets_go_as_before() {
    let mut app = app();
    let ctx = egui::Context::default();
    app.attach(&ctx);
    app.actions.push(Action::CloseChat);
    render(&mut app, &ctx);
    let search = control(&ctx, Stop::Search).expect("drawn");
    ctx.memory_mut(|memory| memory.request_focus(search));
    render(&mut app, &ctx);
    assert_eq!(ctx.memory(|memory| memory.focused()), Some(search));
    press_key(&mut app, &ctx, egui::Key::Escape, egui::Modifiers::NONE);
    assert_eq!(ctx.memory(|memory| memory.focused()), None);
}

/// Where focus can be with no chat open: Escape and Alt+2 are tried there.
const NO_CHAT_STOPS: &[Stop] = &[Stop::Search];

/// Escape with nothing to close is not heard at all.
#[test]
fn escape_with_nothing_open_changes_nothing() {
    for &stop in NO_CHAT_STOPS {
        let mut trace = no_chat_open(stop);
        let from = trace.mark();
        trace.press("Escape", egui::Key::Escape, egui::Modifiers::NONE);
        trace.assert_unchanged(from, &format!("Escape with nothing open on {stop:?}"));
    }
}

/// Alt+2 with no chat open says so, and focus stays put without a sound.
#[test]
fn alt_2_without_a_chat_only_announces() {
    for &stop in NO_CHAT_STOPS {
        let mut trace = no_chat_open(stop);
        let from = trace.mark();
        trace.press("Alt+2", egui::Key::Num2, alt());
        trace.assert_unchanged(from, &format!("Alt+2 without a chat on {stop:?}"));
        assert!(
            trace.app.announcer.spoken().contains(&"No chat open"),
            "{:?}",
            trace.app.announcer.spoken()
        );
    }
}

/// The current message deleted (Delete for me, confirmed), the list's
/// cursor goes to the message after it, not to the newest.
#[test]
fn a_deleted_current_message_leaves_the_cursor_on_the_next() {
    let mut trace = Trace::new(app());
    let (backend, _commands, events) = crate::backend::Backend::recording_with_events();
    trace.app.backend = backend;
    focus_stop(&mut trace, Stop::Messages);
    trace.press("Up", egui::Key::ArrowUp, egui::Modifiers::NONE);
    trace.press("Up", egui::Key::ArrowUp, egui::Modifiers::NONE);
    let chat = trace.app.open_chat.clone().expect("the demo opens a chat");
    let messages = &trace.app.conversations[&chat].messages;
    let current = trace.app.message_cursor[&chat].clone();
    let at = messages
        .iter()
        .position(|message| message.id == current)
        .expect("the current message");
    assert!(at + 2 < messages.len(), "not the newest two");
    let next = messages[at + 1].id.clone();
    events
        .send(crate::backend::Event::MessageDeleted {
            chat: chat.clone(),
            id: current,
        })
        .expect("the app listens");
    trace.settle("deleted", 4);
    assert_eq!(trace.app.message_cursor.get(&chat), Some(&next));
}

/// Where focus can be when the window goes away, one stop or more in each
/// section.
const REFOCUS_STOPS: &[Stop] = &[Stop::Search, Stop::Messages, Stop::Composer];

/// Leaving the window and coming back puts the reader where they were, in
/// any section.
#[test]
fn coming_back_to_the_window_restores_focus() {
    for &stop in REFOCUS_STOPS {
        let mut trace = Trace::new(app());
        focus_stop(&mut trace, stop);
        let before = trace.last().egui;
        let from = trace.mark();
        trace.window("window lost", false);
        trace.window("window back", true);
        let scenario = format!("window refocus on {stop:?}");
        trace.assert_unchanged(from, &scenario);
        assert_eq!(trace.last().egui, before, "{scenario}");
    }
}

/// With nothing focused, coming back to the window lands in the message
/// field of the open chat.
#[test]
fn coming_back_to_the_window_with_nothing_focused_lands_in_the_field() {
    let mut trace = Trace::new(app());
    trace.ctx.memory_mut(|memory| {
        if let Some(id) = memory.focused() {
            memory.surrender_focus(id);
        }
    });
    trace.settle("nothing focused", 3);
    assert_eq!(trace.last().egui, None);
    trace.window("window lost", false);
    trace.window("window back", true);
    assert_eq!(trace.last().egui, Some(egui::Id::new("composer-text")));
}

/// Unlocking counts as coming back to the window: the lock screen's field
/// is gone, and the message field takes focus.
#[test]
fn unlocking_lands_in_the_message_field() {
    let mut app = app();
    app.settings.app_lock_hash = Some(crate::app_lock::verifier("fixture"));
    let mut trace = Trace::new(app);
    trace.app.lock_app();
    trace.settle("locked", 3);
    assert_eq!(
        trace.last().egui,
        Some(egui::Id::new(crate::ui::lock::PASSWORD_ID))
    );
    trace.app.app_lock.unlocked(true);
    trace.settle("unlocked", 3);
    assert_eq!(trace.last().egui, Some(egui::Id::new("composer-text")));
}

/// A notification for the open chat, clicked while the window is away,
/// leaves a reader in its message list there.
#[test]
fn a_notification_clicked_while_away_keeps_the_message_list() {
    let mut trace = Trace::new(app());
    focus_stop(&mut trace, Stop::Messages);
    let before = trace.last().egui;
    let chat = trace.app.open_chat.clone().expect("the demo opens a chat");
    let message = trace.app.conversations[&chat].messages[1].id.clone();
    trace.window("window lost", false);
    trace
        .app
        .actions
        .push(Action::OpenMessage { chat, message });
    trace.window("window back", true);
    assert_eq!(trace.last().egui, before);
}
