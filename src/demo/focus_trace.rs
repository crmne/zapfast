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
    /// How many nodes the frame's update carries.
    nodes: usize,
    /// egui's focused widget after the frame.
    egui: Option<egui::Id>,
    /// The layer that widget is on, by its id.
    layer: Option<egui::Id>,
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
            nodes: tree.nodes.len(),
            step: step.to_owned(),
            action: self.action,
            focus: tree.focus,
            effective,
            ancestors: ancestors(&tree, effective),
            name: describe(&tree, effective),
            spoken: spoken(&tree, effective),
            egui: self.ctx.memory(|memory| memory.focused()),
            layer: self
                .ctx
                .memory(|memory| memory.focused())
                .and_then(|id| self.ctx.read_response(id))
                .map(|response| response.layer_id.id),
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

/// A key pressed in one frame and released in the next, then a few quiet
/// frames. Returns the press frame's index.
fn tap(trace: &mut Trace, step: &str, key: egui::Key, modifiers: egui::Modifiers) -> usize {
    trace.action += 1;
    let press = trace.mark();
    trace.frame(step, key_press(key, modifiers));
    trace.frame(
        step,
        vec![egui::Event::Key {
            key,
            physical_key: Some(key),
            pressed: false,
            repeat: false,
            modifiers,
        }],
    );
    trace.settle(step, 4);
    press
}

/// The demo's open chat with its newest message a photo whose file is
/// there, which Space on it opens in the preview.
fn photo_chat() -> App {
    let mut app = app();
    let (photo, _) = super::sample_files(&app);
    let chat = app.open_chat.clone().expect("the demo opens a chat");
    let conversation = app.conversations.get_mut(&chat).expect("its messages");
    let last = conversation.messages.last_mut().expect("a message");
    let mut media = super::media("image/jpeg", 120_000, Some(800), Some(600));
    media.path = Some(photo);
    last.content = crate::model::Content::Image {
        motion: None,
        caption: None,
        media,
    };
    app
}

/// Escape from an overlay opened from the message list: it closes, and
/// the reader is back on the message it was opened from, heard once.
fn escape_back_to_the_list(trace: &mut Trace, row: &Frame, scenario: &str) {
    let chat = trace.app.open_chat.clone().expect("the demo opens a chat");
    let from = tap(trace, "Escape", egui::Key::Escape, egui::Modifiers::NONE);
    trace.assert_clean(from, &format!("{scenario}, closed"));
    assert_eq!(
        trace.last().egui,
        Some(crate::ui::conversation::messages_list_id(&chat)),
        "{scenario}: back in the message list\n{}",
        trace.dump(from)
    );
    assert_eq!(
        trace.last().effective,
        row.effective,
        "{scenario}: on the message it was opened from"
    );
}

/// Dialogs opened from the message list's keys take focus in the first
/// pass that shows them, never by way of the window, and Escape puts the
/// reader back on the message: Delete's confirmation, Forward, New chat and
/// the shortcuts.
#[test]
fn dialogs_opened_from_the_message_list_give_focus_back() {
    for (step, key, modifiers) in [
        ("Delete", egui::Key::Delete, egui::Modifiers::NONE),
        ("F", egui::Key::F, egui::Modifiers::NONE),
        ("Ctrl+N", egui::Key::N, egui::Modifiers::COMMAND),
        ("Ctrl+/", egui::Key::Slash, egui::Modifiers::COMMAND),
    ] {
        let mut trace = Trace::new(app());
        focus_stop(&mut trace, Stop::Messages);
        let row = trace.last().clone();
        let press = tap(&mut trace, step, key, modifiers);
        let scenario = format!("{step} from the message list");
        assert!(trace.app.dialog.is_some(), "{scenario} opens a dialog");
        trace.assert_clean(press, &scenario);
        assert_eq!(
            trace.frames[press + 1].layer,
            Some(crate::ui::dialogs::modal_id()),
            "{scenario}: focus is in the dialog\n{}",
            trace.dump(press)
        );
        escape_back_to_the_list(&mut trace, &row, &scenario);
        assert!(trace.app.dialog.is_none(), "{scenario}");
    }
}

/// A shared sticker pack's dialog, a video across the window (in the media
/// viewer) and the sticker view, opened while the reader is in the message
/// list, take focus and give it back as well.
#[test]
fn the_sticker_pack_and_the_expanded_video_give_focus_back() {
    for overlay in ["sticker pack", "video", "sticker view"] {
        let mut app = app();
        if overlay == "video" {
            super::video_sample(&mut app, None);
        } else if overlay == "sticker view" {
            super::sample_files(&app);
        } else {
            super::shared_pack_sample(&mut app, false);
        }
        let mut trace = Trace::new(app);
        focus_stop(&mut trace, Stop::Messages);
        let row = trace.last().clone();
        let from = trace.mark();
        trace.action += 1;
        if overlay == "video" {
            let path = trace.app.dirs.media_cache_dir().join("demo-video.mp4");
            trace.app.actions.push(Action::ExpandVideo {
                message: "demo-video".into(),
                path,
            });
        } else if overlay == "sticker view" {
            let (_, sticker) = super::sample_files(&trace.app);
            trace.app.actions.push(Action::PreviewSticker(sticker));
        } else {
            super::sticker_sample(&mut trace.app, crate::model::StickerShelf::Recent, "");
            trace.app.picker = None;
            let ducks = trace.app.sticker_packs.first().cloned();
            trace.app.sticker_preview = ducks.map(|pack| (pack, "Ada Lovelace".to_owned()));
            trace
                .app
                .actions
                .push(Action::ShowDialog(crate::model::Dialog::StickerPack));
        }
        trace.settle(overlay, 4);
        trace.assert_clean(from, overlay);
        let layer = match overlay {
            "video" => crate::ui::image_preview::modal_id(),
            "sticker view" => crate::ui::image_preview::sticker_modal_id(),
            _ => crate::ui::dialogs::modal_id(),
        };
        // Opened at the end of the first frame, entered in the next.
        assert_eq!(
            trace.frames[from + 1].layer,
            Some(layer),
            "{overlay}: focus is in it\n{}",
            trace.dump(from)
        );
        escape_back_to_the_list(&mut trace, &row, overlay);
    }
}

/// With the pointer, no keyboard navigation and no screen reader, the
/// preview opens and closes as it always did: focus let go as it opens,
/// and the message field's once it closes.
#[test]
fn the_image_preview_with_the_pointer_is_as_before() {
    let mut app = photo_chat();
    let ctx = egui::Context::default();
    app.attach(&ctx);
    render(&mut app, &ctx);
    let chat = app.open_chat.clone().expect("the demo opens a chat");
    let list = crate::ui::conversation::messages_list_id(&chat);
    ctx.memory_mut(|memory| memory.request_focus(list));
    render(&mut app, &ctx);
    assert!(!crate::ui::keys::keyboard_or_reader(&ctx));
    let (photo, _) = super::sample_files(&app);
    let message = app.conversations[&chat]
        .messages
        .last()
        .expect("the photo")
        .id
        .clone();
    app.actions.push(Action::PreviewImage {
        path: photo,
        chat,
        message,
        card: None,
    });
    render(&mut app, &ctx);
    render(&mut app, &ctx);
    assert!(app.image_preview.is_some());
    assert_eq!(ctx.memory(|memory| memory.focused()), None, "let go");
    app.actions.push(Action::CloseImagePreview);
    render(&mut app, &ctx);
    render(&mut app, &ctx);
    assert_eq!(
        ctx.memory(|memory| memory.focused()),
        Some(egui::Id::new("composer-text"))
    );
}

/// A dialog the pointer opened leaves focus alone; a viewer the keys then
/// open over it takes focus, as one the keys open over the chat does.
#[test]
fn the_keys_viewer_over_a_pointers_dialog_takes_focus() {
    let mut app = photo_chat();
    let ctx = egui::Context::default();
    app.attach(&ctx);
    render(&mut app, &ctx);
    assert!(!crate::ui::keys::keyboard_or_reader(&ctx));
    app.actions
        .push(Action::ShowDialog(crate::model::Dialog::Shortcuts));
    render(&mut app, &ctx);
    render(&mut app, &ctx);
    assert!(app.dialog.is_some());
    assert_eq!(ctx.memory(|memory| memory.focused()), None, "left alone");
    ctx.data_mut(|data| data.insert_temp(crate::theme::keyboard_focus_id(), true));
    let chat = app.open_chat.clone().expect("the demo opens a chat");
    let (photo, _) = super::sample_files(&app);
    let message = app.conversations[&chat]
        .messages
        .last()
        .expect("the photo")
        .id
        .clone();
    app.actions.push(Action::PreviewImage {
        path: photo,
        chat,
        message,
        card: None,
    });
    for _ in 0..3 {
        render(&mut app, &ctx);
    }
    assert!(app.image_preview.is_some());
    let layer = ctx
        .memory(|memory| memory.focused())
        .and_then(|id| ctx.read_response(id))
        .map(|response| response.layer_id.id);
    assert_eq!(layer, Some(crate::ui::image_preview::modal_id()));
}

/// Arrows Down in an open menu until the reader is on the item `label`.
fn walk_to(trace: &mut Trace, label: &str) {
    for _ in 0..20 {
        if trace.last().name.contains(label) {
            return;
        }
        trace.press("Down", egui::Key::ArrowDown, egui::Modifiers::NONE);
    }
    panic!("no menu item {label}\n{}", trace.dump(trace.mark() - 1));
}

/// A message menu item that opens a dialog moves the reader once, from
/// the item into the dialog, not by way of the message list; Escape puts
/// them back on the message.
#[test]
fn message_menu_items_open_dialogs_with_one_focus_change() {
    for item in ["Forward", "Delete for me", "Message info"] {
        let mut trace = Trace::new(app());
        focus_stop(&mut trace, Stop::Messages);
        let row = trace.last().clone();
        trace.press("Shift+F10", egui::Key::F10, egui::Modifiers::SHIFT);
        walk_to(&mut trace, item);
        let press = tap(&mut trace, "Enter", egui::Key::Enter, egui::Modifiers::NONE);
        let scenario = format!("the message menu's {item}");
        assert!(trace.app.dialog.is_some(), "{scenario} opens a dialog");
        trace.assert_clean(press, &scenario);
        assert_eq!(
            trace.frames[press + 1].layer,
            Some(crate::ui::dialogs::modal_id()),
            "{scenario}\n{}",
            trace.dump(press)
        );
        escape_back_to_the_list(&mut trace, &row, &scenario);
    }
}

/// A primary click at `pos`, pressed in one frame and released in the next,
/// then a few quiet frames.
fn pointer_click(trace: &mut Trace, step: &str, pos: egui::Pos2) {
    let button = |pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    trace.action += 1;
    trace.frame(step, vec![egui::Event::PointerMoved(pos)]);
    trace.frame(step, vec![button(true), button(false)]);
    trace.settle(step, 4);
}

/// Where the first control on the layer `layer` was drawn last.
fn first_control(trace: &Trace, layer: egui::Id) -> egui::Pos2 {
    trace
        .ctx
        .viewport(|viewport| {
            viewport
                .prev_pass
                .widgets
                .layers()
                .filter(|(layer_id, _)| layer_id.id == layer)
                .flat_map(|(_, widgets)| widgets.iter())
                .find(|widget| widget.enabled && widget.visible && widget.sense.is_focusable())
                .map(|widget| widget.rect.center())
        })
        .expect("a control on the layer")
}

/// With a screen reader, or any other UIA client, attached, a mouse user
/// opening and closing the preview with clicks gets upstream's
/// behaviour: no focus pushed into the overlay, and the caret back in the
/// message field once it closes.
#[test]
fn mouse_clicks_keep_upstream_overlay_focus_with_accesskit_on() {
    let mut trace = Trace::new(photo_chat());
    focus_stop(&mut trace, Stop::Composer);
    // The photo takes clicks once it is decoded and drawn.
    for _ in 0..300 {
        if !trace.ctx.has_pending_images() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
        trace.settle("loading", 1);
    }
    trace.settle("loaded", 2);
    let chat = trace.app.open_chat.clone().expect("the demo opens a chat");
    let newest = trace.app.conversations[&chat]
        .messages
        .last()
        .expect("a message")
        .id
        .clone();
    let picture = trace
        .ctx
        .data(|data| {
            data.get_temp::<egui::Rect>(
                crate::ui::conversation::bubble_id(&chat, &newest).with("picture"),
            )
        })
        .expect("the photo is drawn");
    pointer_click(&mut trace, "click the photo", picture.center());
    assert!(trace.app.image_preview.is_some(), "the click opens it");
    assert_eq!(trace.last().egui, None, "focus let go, as upstream");
    let close = first_control(&trace, crate::ui::image_preview::modal_id());
    pointer_click(&mut trace, "click Close", close);
    assert!(trace.app.image_preview.is_none(), "the click closes it");
    assert_eq!(
        trace.last().egui,
        Some(crate::ui::conversation::composer_id()),
        "the caret is back in the message field"
    );
}

/// A dialog open when ZapFast locks is there again after the unlock, as
/// upstream keeps it, and takes focus again: the field behind it cannot,
/// and the lock screen's field is gone.
#[test]
fn a_dialog_open_at_the_lock_takes_focus_again_after_the_unlock() {
    let mut app = app();
    app.settings.app_lock_hash = Some(crate::app_lock::verifier("fixture"));
    let mut trace = Trace::new(app);
    focus_stop(&mut trace, Stop::Messages);
    tap(
        &mut trace,
        "Ctrl+/",
        egui::Key::Slash,
        egui::Modifiers::COMMAND,
    );
    assert!(trace.app.dialog.is_some());
    trace.action += 1;
    trace.app.lock_app();
    trace.settle("locked", 3);
    let from = trace.mark();
    trace.action += 1;
    trace.app.app_lock.unlocked(true);
    trace.settle("unlocked", 3);
    assert!(trace.app.dialog.is_some(), "the dialog stays, as upstream");
    assert_eq!(
        trace.last().layer,
        Some(crate::ui::dialogs::modal_id()),
        "{}",
        trace.dump(from - 1)
    );
    trace.assert_clean(from, "unlocked");
}

/// The cropper over a staged photo, as the keys or a screen reader open it:
/// the demo's "crop" page, set up after the message list has focus.
fn open_cropper(trace: &mut Trace, target: crate::app::AttachmentId, photo: &std::path::Path) {
    let (width, height) = image::image_dimensions(photo).unwrap_or((4, 3));
    trace.app.picture_edit = Some(crate::model::PictureEdit::new(
        target,
        crate::model::PictureSource::File(photo.to_owned()),
        width,
        height,
    ));
    trace.action += 1;
    trace.settle("open the cropper", 4);
}

/// The cropper, opened from the keys, takes focus on "Keep this crop" and
/// keeps Tab among its own controls, never the chat under it. Enter presses
/// the focused button: Cancel leaves the picture as it was and gives focus
/// back to the attachment review, Turn left turns it, and Keep keeps the crop.
/// Before, the cropper took no focus and Enter kept the crop whatever was
/// focused.
#[test]
fn the_cropper_takes_focus_and_enter_presses_the_focused_button() {
    let mut app = app();
    let (photo, _) = super::sample_files(&app);
    let target = app.stage(crate::app::Pending::File(photo.clone()));
    let mut trace = Trace::new(app);
    focus_stop(&mut trace, Stop::ReviewEdit);
    let row = trace.last().clone();
    let editor = crate::ui::image_editor::area_id();
    let from = trace.mark();
    open_cropper(&mut trace, target, &photo);
    trace.assert_clean(from, "the cropper opened");
    assert_eq!(
        trace.last().name,
        "Button \"Keep this crop\"",
        "{}",
        trace.dump(from)
    );
    for _ in 0..7 {
        trace.press("Tab", egui::Key::Tab, egui::Modifiers::NONE);
        assert_eq!(
            trace.last().layer,
            Some(editor),
            "Tab stays in the cropper\n{}",
            trace.dump(from)
        );
        // The drag areas take the pointer only: no nameless stop.
        assert!(
            !trace.last().name.ends_with("\"\""),
            "a named control\n{}",
            trace.dump(from)
        );
    }
    // Enter on Cancel cancels: nothing is kept, and the reader is back on
    // the attachment review.
    for _ in 0..8 {
        if trace.last().name.contains("Cancel") {
            break;
        }
        trace.press("Tab", egui::Key::Tab, egui::Modifiers::NONE);
    }
    let from = tap(&mut trace, "Enter", egui::Key::Enter, egui::Modifiers::NONE);
    assert!(
        trace.app.picture_edit.is_none(),
        "Cancel closed the cropper"
    );
    assert!(
        trace.app.picture_applying.is_empty(),
        "Enter on Cancel keeps nothing"
    );
    trace.assert_clean(from, "the cropper cancelled");
    assert_eq!(
        trace.last().effective,
        row.effective,
        "back on the attachment review"
    );
    // Enter on Turn left turns the picture and stays.
    open_cropper(&mut trace, target, &photo);
    for _ in 0..8 {
        if trace.last().name.contains("Turn left") {
            break;
        }
        trace.press("Tab", egui::Key::Tab, egui::Modifiers::NONE);
    }
    tap(&mut trace, "Enter", egui::Key::Enter, egui::Modifiers::NONE);
    assert_eq!(
        trace.app.picture_edit.as_ref().map(|edit| edit.turns),
        Some(3),
        "turned a quarter to the left, still open"
    );
    // Enter on Keep, where focus enters, keeps the crop.
    trace.app.picture_edit = None;
    trace.settle("closed", 2);
    open_cropper(&mut trace, target, &photo);
    tap(&mut trace, "Enter", egui::Key::Enter, egui::Modifiers::NONE);
    assert!(trace.app.picture_edit.is_none());
    assert!(
        trace.app.picture_applying.contains_key(&target),
        "Enter on Keep keeps the crop"
    );
}

/// A file's cropper opens frames after it was asked for, once the
/// picture's size is read. Asked for with the pointer, it leaves focus
/// where it was even with a screen reader attached; asked for from the
/// keys, it takes it on "Keep this crop".
#[test]
fn the_cropper_follows_focus_as_it_was_asked_for() {
    for pointer in [true, false] {
        let mut app = app();
        let (photo, _) = super::sample_files(&app);
        let target = app.stage(crate::app::Pending::File(photo.clone()));
        let mut trace = Trace::new(app);
        focus_stop(&mut trace, Stop::ReviewEdit);
        trace.app.actions.push(Action::EditPicture(target));
        trace.action += 1;
        if pointer {
            // The click is applied before this frame's UI, which would note
            // the pointer: noted already, as the frame of a real click does
            // before its UI asks for the cropper.
            trace
                .ctx
                .data_mut(|data| data.insert_temp(crate::theme::keyboard_focus_id(), false));
            let at = egui::pos2(600.0, 400.0);
            let button = |pressed| egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            trace.frame("ask with the pointer", vec![button(true), button(false)]);
        } else {
            trace.frame("ask from the keys", Vec::new());
        }
        assert!(
            trace.app.picture_edit.is_none(),
            "a file waits for its size"
        );
        let before = trace.last().egui;
        // The size arrives: the cropper opens, as `Event::PictureInspected`
        // opens it.
        open_cropper(&mut trace, target, &photo);
        if pointer {
            assert_eq!(trace.last().egui, before, "focus left alone");
        } else {
            assert_eq!(
                trace.last().name,
                "Button \"Keep this crop\"",
                "{}",
                trace.dump(1)
            );
        }
    }
}

/// The pickers take focus in the first pass that shows them, never by way
/// of the window, and keep their own way back: the reaction picker to its
/// message, the emoji picker to the message field its emoji go to.
#[test]
fn the_pickers_open_without_a_window_frame() {
    let mut trace = Trace::new(app());
    focus_stop(&mut trace, Stop::Messages);
    let row = trace.last().clone();
    let press = tap(&mut trace, "E", egui::Key::E, egui::Modifiers::NONE);
    assert!(trace.app.reaction_target.is_some());
    trace.assert_clean(press, "the reaction picker");
    assert_eq!(trace.last().name, "TextInput \"Search emoji\"");
    escape_back_to_the_list(&mut trace, &row, "the reaction picker");

    let mut trace = Trace::new(app());
    focus_stop(&mut trace, Stop::Emoji);
    assert!(trace.last().name.contains("Emoji"), "{}", trace.last().name);
    let press = tap(&mut trace, "Enter", egui::Key::Enter, egui::Modifiers::NONE);
    assert!(trace.app.picker.is_some());
    trace.assert_clean(press, "the emoji picker");
    assert_eq!(trace.last().name, "TextInput \"Search emoji\"");
    let from = tap(
        &mut trace,
        "Escape",
        egui::Key::Escape,
        egui::Modifiers::NONE,
    );
    trace.assert_clean(from, "the emoji picker, closed");
    assert_eq!(trace.last().egui, Some(egui::Id::new("composer-text")));
}

/// A long chat whose newest message replies to message `target`.
fn replying_to(target: usize) -> App {
    let mut app = app();
    long_chat(&mut app, 400);
    let chat = app.open_chat.clone().expect("the demo opens a chat");
    let conversation = app.conversations.get_mut(&chat).expect("its messages");
    let quoted = conversation.messages[target].clone();
    conversation.messages[399].quoted = Some(crate::model::Quoted {
        labelled: None,
        chat: None,
        id: quoted.id.clone(),
        sender: quoted.sender.clone(),
        sender_name: None,
        summary: quoted.summary(),
        mentions: Vec::new(),
    });
    app
}

/// Fixes the clock at `now`, in UTC on a 24-hour clock, until the guard
/// drops. The demo dates its messages from now, so a message gains its day
/// in its name ("Today, ...") when a local midnight falls just before it.
fn clock_at(now: i64) -> crate::util::fixed_clock::Guard {
    crate::util::fixed_clock::set(crate::util::fixed_clock::Clock {
        now,
        zone: jiff::tz::TimeZone::UTC,
        twelve_hour: false,
    })
}

/// G on a reply lands on the replied message in the frame after the key,
/// which egui runs at once after a frame with input. A replied message far
/// up a long chat costs that frame no more than a near one: the rows laid
/// out only to place it are not handed to the screen reader. The clock is
/// fixed, and message 50 is replied to both inside a day and as the day's
/// first message, which the reader hears with its day.
#[test]
fn g_lands_on_the_replied_message_in_the_next_frame() {
    // 14:00 UTC on Wednesday 11 March 2026: the 400 messages, a minute
    // apart, all fall on that day.
    const AFTERNOON: i64 = 1_773_237_600;
    // A later time that places message 50 just after midnight.
    let day_start = {
        let _clock = clock_at(AFTERNOON);
        let app = replying_to(50);
        let chat = app.open_chat.as_ref().expect("the demo opens a chat");
        let sent = app.conversations[chat].messages[50].timestamp;
        let (_, midnight) = crate::util::day_key(sent)
            .and_then(crate::util::day_bounds)
            .expect("the message has a day");
        AFTERNOON + midnight + 30 - sent
    };
    for (target, now, day) in [
        (398, AFTERNOON, ""),
        (50, AFTERNOON, ""),
        (50, day_start, "Today, "),
    ] {
        let _clock = clock_at(now);
        let mut trace = Trace::new(replying_to(target));
        focus_stop(&mut trace, Stop::Messages);
        let before = trace.last().clone();
        let chat = trace.app.open_chat.clone().expect("the demo opens a chat");
        let heights = |trace: &Trace| -> Vec<f32> {
            let rows = &trace.app.conversations[&chat].rows;
            (target.saturating_sub(10)..target)
                .filter_map(|index| rows.get(&format!("long-{index}")).map(|row| row.height))
                .collect()
        };
        trace.action += 1;
        let press = trace.mark();
        trace.frame("G", key_press(egui::Key::G, egui::Modifiers::NONE));
        trace.frame(
            "G",
            vec![egui::Event::Key {
                key: egui::Key::G,
                physical_key: Some(egui::Key::G),
                pressed: false,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        let laid_out_hidden = heights(&trace);
        trace.settle("G", 4);
        let scenario = format!("G to message {target} at {now}");
        trace.assert_clean(press, &scenario);
        let landed = &trace.frames[press + 1];
        assert!(
            landed
                .spoken
                .contains(&format!("Some(\"{day}You, Fixture line {target},")),
            "{scenario}: the reader is on the replied message in the next frame, \
             not on {}\n{}",
            landed.spoken,
            trace.dump(press)
        );
        assert!(
            landed.nodes < before.nodes * 2,
            "{scenario}: the landing frame carries {} nodes, against {} before",
            landed.nodes,
            before.nodes
        );
        assert_eq!(
            laid_out_hidden,
            heights(&trace),
            "{scenario}: rows laid out hidden measure as shown"
        );
        if target == 50 {
            // Far from either end: centred in the view, as the jump asks.
            let bubble = trace
                .ctx
                .data(|data| {
                    data.get_temp::<egui::Rect>(
                        crate::ui::conversation::bubble_id(&chat, "long-50").with("rect"),
                    )
                })
                .expect("the replied message is drawn");
            let view = trace
                .app
                .selection_view
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .expect("the message view is drawn");
            assert!(
                (bubble.center().y - view.center().y).abs() <= 1.0,
                "{scenario}: {bubble:?} centred in {view:?}"
            );
        }
    }
}
