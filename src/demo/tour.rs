//! A repeatable tour driven through the real pointer and keyboard handlers.

pub(super) mod media;
mod session;

use crate::{
    app::App,
    model::{Content, Page},
    settings::ThemeChoice,
};
use egui::{Event, Key, Modifiers, PointerButton, Pos2, Rect, pos2, vec2};
use serde::Serialize;
use std::{
    collections::HashMap,
    path::PathBuf,
    time::{Duration, Instant},
};

const PHOTO_CAPTION: &str = "A little poster for launch day ⚡";
/// Length of the input-driven tour, excluding its optional start delay.
pub const DURATION: Duration = Duration::from_secs(41);
/// Sets up the opening shot. Call only on an app populated with demo data.
pub fn prepare(app: &mut App) {
    assert!(app.backend.is_offline(), "a tour requires an offline app");
    app.settings.theme = ThemeChoice::Dark;
    app.settings.keep_running_in_background = false;
    app.page = Page::Chats;
    app.dialog = None;
    app.picker = None;
    app.search.clear();
    app.search_hits.clear();
    app.composer.clear();
    app.drafts.clear();
    app.reply_to = None;
    app.typing.clear();
    app.actions.clear();
    app.open_chat = Some(super::SAMPLES[0].id.to_owned());
    app.scroll_to_bottom = true;
    app.scroll_anchor = None;
    app.focus_composer = false;
    app.sidebar_visible = true;
    app.show_archived = false;
    app.backend.record_demo_commands();
    media::populate(app).expect("bundled demo media");
    if let Some(row) = app
        .conversations
        .get_mut(super::SAMPLES[0].id)
        .and_then(|chat| chat.message_mut("ada-sticker"))
        && let Content::Sticker { media, animated } = &mut row.content
    {
        media.path = app.stickers_saved.first().cloned();
        *animated = false;
    }
    super::apply_flags(app, Some("voice"));
    // Show fully loaded media instead of the deliberately blurry download
    // previews used by the general screenshot fixtures.
    let (photo, _) = super::sample_files(app);
    for (chat, id, caption) in [
        (super::SAMPLES[0].id, "ada-photo", PHOTO_CAPTION),
        (
            super::SAMPLES[1].id,
            "group-photo",
            "Tonight's meetup, doors at 18:30",
        ),
    ] {
        if let Some(row) = app
            .conversations
            .get_mut(chat)
            .and_then(|chat| chat.message_mut(id))
            && let Content::Image {
                media,
                caption: text,
            } = &mut row.content
        {
            media.path = Some(photo.clone());
            media.width = Some(900);
            media.height = Some(1200);
            *text = Some(caption.to_owned());
        }
    }
    if let Some(quote) = app
        .conversations
        .get_mut(super::SAMPLES[0].id)
        .and_then(|chat| chat.message_mut("ada-reply"))
        .and_then(|row| row.quoted.as_mut())
    {
        quote.summary = "Voice message (0:06)".into();
    }
    // Keep the launch footage focused on this app.
    if let Some(row) = app
        .conversations
        .get_mut(super::SAMPLES[0].id)
        .and_then(|chat| chat.message_mut("ada-link"))
    {
        row.content = Content::text("The desktop app is ready! https://zapfast.rocks");
        row.thumbnail = None;
        let summary = row.summary();
        if let Some(last) = app.chats.first_mut().and_then(|chat| chat.last.as_mut()) {
            last.summary = summary;
        }
    }
}

#[derive(Clone, Copy)]
enum Target {
    Label(&'static str),
    Widget(&'static str),
    Bubble(&'static str),
    Picker,
    Gif,
    Sticker,
}

enum Gesture {
    Key(Key, Modifiers, &'static str),
    Text(char),
    Move(Target),
    Click(PointerButton),
}

struct Cue {
    at: f32,
    gesture: Gesture,
}

fn command() -> Modifiers {
    Modifiers {
        command: true,
        ctrl: !cfg!(target_os = "macos"),
        mac_cmd: cfg!(target_os = "macos"),
        ..Modifiers::NONE
    }
}

fn script() -> Vec<Cue> {
    use Gesture::*;
    use Target::*;
    let mut cues = Vec::new();
    let mut add = |at, gesture| cues.push(Cue { at, gesture });
    let left = PointerButton::Primary;
    add(0.0, Key(egui::Key::K, command(), "Ctrl + K · Search chats"));
    add(0.8, Move(Label("Rust Berlin")));
    add(1.15, Click(left));
    add(
        1.7,
        Key(
            egui::Key::Escape,
            Modifiers::NONE,
            "Esc · Return to the chat",
        ),
    );
    add(
        2.1,
        Key(
            egui::Key::ArrowUp,
            Modifiers::ALT,
            "Alt + ↑ · Previous chat",
        ),
    );
    add(
        2.6,
        Key(egui::Key::ArrowDown, Modifiers::ALT, "Alt + ↓ · Next chat"),
    );
    add(
        3.1,
        Key(
            egui::Key::ArrowUp,
            Modifiers::ALT,
            "Alt + ↑ · Previous chat",
        ),
    );
    add(
        4.8,
        Key(egui::Key::End, command(), "Ctrl + End · Latest messages"),
    );
    add(5.4, Move(Bubble("ada-voice")));
    add(5.8, Click(PointerButton::Secondary));
    add(6.25, Move(Label("Reply")));
    add(6.7, Click(left));
    add(
        8.1,
        Key(egui::Key::Enter, Modifiers::NONE, "Enter · Complete emoji"),
    );
    add(
        8.7,
        Key(egui::Key::Enter, Modifiers::NONE, "Enter · Send reply"),
    );
    add(9.4, Move(Picker));
    add(9.8, Click(left));
    add(10.5, Move(Label("GIF")));
    add(10.9, Click(left));
    add(11.5, Move(Widget("gif-search")));
    add(11.9, Click(left));
    add(
        12.6,
        Key(egui::Key::Enter, Modifiers::NONE, "Enter · Search GIFs"),
    );
    add(13.3, Move(Gif));
    add(
        13.8,
        Key(egui::Key::Escape, Modifiers::NONE, "Esc · Close GIF search"),
    );
    add(15.9, Move(Picker));
    add(16.3, Click(left));
    add(17.0, Move(Label("Stickers")));
    add(17.4, Click(left));
    add(18.0, Move(Sticker));
    add(18.5, Click(left));
    add(
        20.2,
        Key(egui::Key::ArrowDown, Modifiers::ALT, "Alt + ↓ · Next chat"),
    );
    add(
        21.6,
        Key(
            egui::Key::Enter,
            Modifiers::NONE,
            "Enter · Complete mention",
        ),
    );
    add(
        23.0,
        Key(egui::Key::Enter, Modifiers::NONE, "Enter · Send message"),
    );
    add(
        24.0,
        Key(egui::Key::B, command(), "Ctrl + B · Hide chat list"),
    );
    add(
        25.0,
        Key(egui::Key::B, command(), "Ctrl + B · Show chat list"),
    );
    add(26.0, Move(Label("Rust Berlin")));
    add(26.5, Click(left));
    add(
        28.0,
        Key(egui::Key::Escape, Modifiers::NONE, "Esc · Close group info"),
    );
    add(
        28.6,
        Key(egui::Key::Slash, command(), "Ctrl + / · Keyboard shortcuts"),
    );
    add(
        31.8,
        Key(egui::Key::Escape, Modifiers::NONE, "Esc · Close shortcuts"),
    );
    add(
        32.5,
        Key(egui::Key::Comma, command(), "Ctrl + , · Settings"),
    );
    add(33.0, Move(Label("Dark")));
    add(33.35, Click(left));
    add(33.5, Move(Label("Light")));
    add(33.85, Click(left));
    add(
        34.5,
        Key(egui::Key::Escape, Modifiers::NONE, "Esc · Back to chats"),
    );
    add(
        36.0,
        Key(
            egui::Key::ArrowUp,
            Modifiers::ALT,
            "Alt + ↑ · Previous chat",
        ),
    );
    add(
        37.5,
        Key(egui::Key::Comma, command(), "Ctrl + , · Settings"),
    );
    add(38.0, Move(Label("Light")));
    add(38.35, Click(left));
    add(38.5, Move(Label("Dark")));
    add(38.85, Click(left));
    add(
        39.2,
        Key(egui::Key::Escape, Modifiers::NONE, "Esc · Back to chats"),
    );
    for (start, text) in [
        (0.2, "Rust"),
        (7.1, "See you tonight! :smile"),
        (12.1, "party"),
        (20.8, "@mi"),
        (21.9, " see you in the front row!"),
    ] {
        for (index, character) in text.chars().enumerate() {
            cues.push(Cue {
                at: start + index as f32 * 0.025,
                gesture: Text(character),
            });
        }
    }
    cues.sort_by(|a, b| a.at.total_cmp(&b.at));
    cues
}

#[derive(Serialize)]
struct Trace {
    at: f32,
    #[serde(flatten)]
    event: TraceEvent,
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum TraceEvent {
    Pointer {
        x: f32,
        y: f32,
    },
    Click {
        x: f32,
        y: f32,
        button: &'static str,
    },
    Keys {
        label: String,
    },
}

/// Supplies ordinary egui input. The optional trace is rendered onto video later.
pub struct Tour {
    delay: Option<Duration>,
    start: Option<Instant>,
    cues: Vec<Cue>,
    next: usize,
    previous: f32,
    pointer: Pos2,
    motion: Option<(f32, Pos2, Pos2)>,
    labels: HashMap<String, Pos2>,
    trace: Vec<Trace>,
    trace_path: Option<PathBuf>,
    saved: bool,
    failed: bool,
}

impl Tour {
    pub fn new(delay: Option<Duration>, trace_path: Option<PathBuf>) -> Self {
        Self {
            delay,
            start: None,
            cues: script(),
            next: 0,
            previous: 0.0,
            pointer: pos2(680.0, 440.0),
            motion: None,
            labels: HashMap::new(),
            trace: Vec::new(),
            trace_path,
            saved: false,
            failed: false,
        }
    }

    pub fn input(&mut self, app: &mut App, ctx: &egui::Context, input: &mut egui::RawInput) {
        let replay = input.events.iter().any(|event| {
            matches!(event,
            Event::Key { key: Key::Space, pressed: true, repeat: false, modifiers, .. }
                if modifiers.is_none())
        });
        if replay {
            input.events.retain(|event| {
                !matches!(
                    event,
                    Event::Key {
                        key: Key::Space,
                        ..
                    } | Event::Text(_)
                )
            });
            super::populate(app);
            prepare(app);
            self.start = Some(Instant::now());
            self.delay = None;
            self.next = 0;
            self.previous = 0.0;
            self.motion = None;
            self.trace.clear();
            self.saved = false;
            self.failed = false;
        }
        if let Some(start) = self.start
            && Instant::now() >= start
        {
            self.input_at(app, ctx, input, start.elapsed().as_secs_f32());
        }
    }

    fn target(&self, target: Target, app: &App, ctx: &egui::Context) -> Option<Pos2> {
        match target {
            Target::Label(label) => self.labels.get(label).copied(),
            Target::Widget(id) => ctx
                .read_response(egui::Id::new(id))
                .map(|r| r.rect.center()),
            Target::Bubble(message) => {
                let id = crate::ui::conversation::bubble_id(app.open_chat.as_deref()?, message)
                    .with("rect");
                let rect = ctx.data(|d| d.get_temp::<Rect>(id))?;
                let view = (*app.selection_view.lock().unwrap_or_else(|p| p.into_inner()))?;
                let rect = rect.intersect(view);
                rect.is_positive().then_some(rect.center())
            }
            Target::Picker => app.picker_anchor.map(|rect| rect.center()),
            Target::Gif => ctx
                .read_response(egui::Id::new("gif-search"))
                .map(|r| r.rect.left_bottom() + vec2(60.0, 55.0)),
            Target::Sticker => ctx
                .data(|d| d.get_temp::<Rect>(crate::ui::picker::first_tile_id()))
                .map(|rect| rect.center()),
        }
    }

    fn input_at(&mut self, app: &App, ctx: &egui::Context, input: &mut egui::RawInput, at: f32) {
        if self.failed {
            return;
        }
        while self.next < self.cues.len() && at >= self.cues[self.next].at {
            match self.cues[self.next].gesture {
                Gesture::Move(target) => {
                    let Some(end) = self.target(target, app, ctx) else {
                        self.failed = true;
                        log::error!("tour stopped: missing UI target at step {}", self.next);
                        return;
                    };
                    self.motion = Some((at, self.pointer, end));
                }
                Gesture::Click(button) => {
                    for pressed in [true, false] {
                        input.events.push(Event::PointerButton {
                            pos: self.pointer,
                            button,
                            pressed,
                            modifiers: Modifiers::NONE,
                        });
                    }
                    self.trace.push(Trace {
                        at,
                        event: TraceEvent::Click {
                            x: self.pointer.x,
                            y: self.pointer.y,
                            button: if button == PointerButton::Secondary {
                                "right"
                            } else {
                                "left"
                            },
                        },
                    });
                }
                Gesture::Key(key, modifiers, label) => {
                    for pressed in [true, false] {
                        input.events.push(Event::Key {
                            key,
                            physical_key: None,
                            pressed,
                            repeat: false,
                            modifiers,
                        });
                    }
                    self.trace.push(Trace {
                        at,
                        event: TraceEvent::Keys {
                            label: crate::ui::keys::label(label),
                        },
                    });
                }
                Gesture::Text(character) => input.events.push(Event::Text(character.to_string())),
            }
            self.next += 1;
        }
        if let Some((began, from, to)) = self.motion {
            let t = ((at - began) / 0.28).clamp(0.0, 1.0);
            self.pointer = from.lerp(to, t * t * (3.0 - 2.0 * t));
            if t == 1.0 {
                self.motion = None;
            }
        }
        let scroll = (at.min(4.6) - self.previous.max(3.6)).max(0.0) * 340.0;
        if scroll > 0.0
            && let Some(view) = *app.selection_view.lock().unwrap_or_else(|p| p.into_inner())
        {
            self.pointer = view.center();
            input.events.push(Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: vec2(0.0, scroll),
                modifiers: Modifiers::NONE,
                phase: egui::TouchPhase::Move,
            });
        }
        input.events.insert(0, Event::PointerMoved(self.pointer));
        if self.trace.last().is_none_or(|event| {
            !matches!(event.event,
            TraceEvent::Pointer { x, y } if x == self.pointer.x && y == self.pointer.y)
        }) {
            self.trace.push(Trace {
                at,
                event: TraceEvent::Pointer {
                    x: self.pointer.x,
                    y: self.pointer.y,
                },
            });
        }
        self.previous = at;
    }

    pub fn drive(&mut self, _app: &mut App, ctx: &egui::Context) {
        let now = Instant::now();
        if let Some(delay) = self.delay.take() {
            self.start = Some(now + delay);
        }
        let Some(start) = self.start else {
            return;
        };
        if now < start {
            ctx.request_repaint_after(start - now);
        } else if start.elapsed() < DURATION && !self.failed {
            ctx.request_repaint_after(Duration::from_millis(16));
        } else if !self.saved {
            self.saved = true;
            if let Some(path) = &self.trace_path {
                let data = serde_json::json!({ "width": ctx.content_rect().width(),
                    "height": ctx.content_rect().height(), "duration": DURATION.as_secs(),
                    "complete": !self.failed, "events": self.trace });
                if let Err(error) = std::fs::write(path, data.to_string()) {
                    log::error!("could not write tour input trace: {error}");
                }
            }
        }
    }

    /// Finds click targets in the actual painted UI; no view-specific hooks or
    /// hard-coded menu coordinates are needed. Called after frame_ui.
    pub fn observe(&mut self, app: &mut App, ctx: &egui::Context) {
        session::respond(app);
        self.labels.clear();
        let layers: Vec<_> = ctx.memory(|memory| memory.layer_ids().collect());
        for layer in layers {
            let transform = ctx.layer_transform_to_global(layer).unwrap_or_default();
            ctx.graphics(|graphics| {
                if let Some(list) = graphics.get(layer) {
                    for clipped in list.all_entries() {
                        if let egui::Shape::Text(text) = &clipped.shape {
                            let rect = Rect::from_min_size(text.pos, text.galley.size());
                            if rect.intersects(clipped.clip_rect) {
                                self.labels.insert(
                                    text.galley.text().to_owned(),
                                    transform * rect.center(),
                                );
                            }
                        }
                    }
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Dialog, Page, PickerTab};

    fn frame(app: &mut App, tour: &mut Tour, ctx: &egui::Context, events: Vec<Event>) {
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1180.0, 780.0))),
            events,
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            app.background_frame(ctx);
            app.frame_ui(ui);
            tour.observe(app, ctx);
        });
        output.textures_delta.clear();
    }
    fn click(app: &mut App, tour: &mut Tour, ctx: &egui::Context, label: &str) {
        let pos = *tour
            .labels
            .get(label)
            .unwrap_or_else(|| panic!("missing {label}"));
        for pressed in [true, false] {
            frame(
                app,
                tour,
                ctx,
                vec![
                    Event::PointerMoved(pos),
                    Event::PointerButton {
                        pos,
                        button: PointerButton::Primary,
                        pressed,
                        modifiers: Modifiers::NONE,
                    },
                ],
            );
        }
        frame(app, tour, ctx, Vec::new());
    }

    #[test]
    fn the_chat_search_pane_lists_hits_and_opens_one() {
        let mut app = super::super::tests::app();
        prepare(&mut app);
        let ctx = egui::Context::default();
        app.attach(&ctx);
        let mut tour = Tour::new(None, None);
        let open = super::super::sample_ids()[0].to_owned();
        let other = super::super::sample_ids()[1].to_owned();
        app.actions.push(crate::model::Action::OpenChat(open));
        frame(&mut app, &mut tour, &ctx, Vec::new());
        app.actions.push(crate::model::Action::OpenChatSearch);
        frame(&mut app, &mut tour, &ctx, Vec::new());
        app.chat_search = "engine".into();
        // Hits from another chat, so their text is painted only in the pane
        // and the click lands on the row rather than on a bubble.
        app.chat_search_hits = app
            .conversations
            .get(&other)
            .map(|conversation| conversation.messages.clone())
            .unwrap_or_default();
        for _ in 0..3 {
            frame(&mut app, &mut tour, &ctx, Vec::new());
        }
        // The pane's copy is translated, and the interface language follows
        // the system when Settings carries no choice of its own.
        let title = crate::i18n::gettext(app.locale, "Search messages").to_string();
        assert!(tour.labels.contains_key(&title), "the pane names itself");
        let hit = app.chat_search_hits.first().cloned().expect("a hit");
        // The row previews the line the query matched, not the message's first
        // line, so that is the text the click lands on.
        let preview = hit.text_matching("engine").unwrap_or_else(|| hit.summary());
        click(&mut app, &mut tour, &ctx, &preview);
        assert_eq!(
            app.open_chat.as_deref(),
            Some(hit.chat.as_str()),
            "clicking a hit opens the chat it belongs to"
        );
        // The jump is consumed by the frame that scrolls to it, so the flash
        // it leaves behind is what says the message was the one asked for.
        assert_eq!(
            app.jump_highlight
                .as_ref()
                .map(|jump| (jump.chat.as_str(), jump.message.as_str())),
            Some((hit.chat.as_str(), hit.id.as_str())),
            "and brings that message into view"
        );
    }

    #[test]
    fn the_keyboard_walks_the_chat_search_results_and_escape_closes_the_pane() {
        let mut app = super::super::tests::app();
        prepare(&mut app);
        let ctx = egui::Context::default();
        app.attach(&ctx);
        let mut tour = Tour::new(None, None);
        let open = super::super::sample_ids()[0].to_owned();
        app.actions
            .push(crate::model::Action::OpenChat(open.clone()));
        frame(&mut app, &mut tour, &ctx, Vec::new());
        app.actions.push(crate::model::Action::OpenChatSearch);
        frame(&mut app, &mut tour, &ctx, Vec::new());
        app.chat_search = "e".into();
        // Newest first, as the archive answers.
        app.chat_search_hits = app.conversations[&open]
            .messages
            .iter()
            .rev()
            .take(3)
            .cloned()
            .collect();
        assert_eq!(app.chat_search_hits.len(), 3);
        frame(&mut app, &mut tour, &ctx, Vec::new());
        let field = egui::Id::new("chat-message-search");
        assert!(ctx.memory(|memory| memory.has_focus(field)));
        let key = |key: egui::Key| Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        };
        frame(&mut app, &mut tour, &ctx, vec![key(egui::Key::ArrowDown)]);
        frame(&mut app, &mut tour, &ctx, vec![key(egui::Key::ArrowDown)]);
        frame(&mut app, &mut tour, &ctx, vec![key(egui::Key::ArrowDown)]);
        frame(&mut app, &mut tour, &ctx, vec![key(egui::Key::ArrowDown)]);
        assert_eq!(app.chat_search_selected, Some(2), "stops at the last");
        frame(&mut app, &mut tour, &ctx, vec![key(egui::Key::ArrowUp)]);
        frame(&mut app, &mut tour, &ctx, vec![key(egui::Key::Enter)]);
        frame(&mut app, &mut tour, &ctx, Vec::new());
        let second = app.chat_search_hits[1].id.clone();
        assert_eq!(
            app.jump_highlight
                .as_ref()
                .map(|jump| jump.message.as_str()),
            Some(second.as_str()),
            "Enter jumps to the result reached"
        );
        assert!(
            ctx.memory(|memory| memory.has_focus(field)),
            "and the field keeps the keyboard for the next one"
        );
        assert!(app.chat_search_open);
        frame(&mut app, &mut tour, &ctx, vec![key(egui::Key::Escape)]);
        frame(&mut app, &mut tour, &ctx, Vec::new());
        assert!(!app.chat_search_open, "Escape closes the pane");
        assert_eq!(
            app.open_chat.as_deref(),
            Some(open.as_str()),
            "not the chat"
        );
        assert!(app.chat_search.is_empty());
    }

    #[test]
    fn a_narrow_window_lays_the_search_pane_over_the_chat_and_folds_it_on_a_pick() {
        let mut app = super::super::tests::app();
        prepare(&mut app);
        let ctx = egui::Context::default();
        app.attach(&ctx);
        let mut tour = Tour::new(None, None);
        let narrow = |app: &mut App, tour: &mut Tour, events: Vec<Event>| {
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(760.0, 600.0))),
                events,
                ..Default::default()
            };
            let mut output = ctx.run_ui(input, |ui| {
                app.background_frame(&ctx);
                app.frame_ui(ui);
                tour.observe(app, &ctx);
            });
            output.textures_delta.clear();
        };
        let open = super::super::sample_ids()[0].to_owned();
        app.actions
            .push(crate::model::Action::OpenChat(open.clone()));
        narrow(&mut app, &mut tour, Vec::new());
        app.actions.push(crate::model::Action::OpenChatSearch);
        narrow(&mut app, &mut tour, Vec::new());
        app.chat_search = "e".into();
        app.chat_search_hits = app.conversations[&open]
            .messages
            .iter()
            .rev()
            .take(2)
            .cloned()
            .collect();
        narrow(&mut app, &mut tour, Vec::new());
        // The chat list leaves too little room to share, so the pane lies
        // over the conversation rather than squeezing it.
        let overlay = ctx.memory(|memory| {
            memory
                .layer_ids()
                .any(|layer| layer.id == egui::Id::new("chat-search-overlay"))
        });
        assert!(overlay, "the pane is laid over the chat");
        let enter = Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        };
        narrow(&mut app, &mut tour, vec![enter]);
        narrow(&mut app, &mut tour, Vec::new());
        let newest = app.chat_search_hits[0].id.clone();
        assert_eq!(
            app.jump_highlight
                .as_ref()
                .map(|jump| jump.message.as_str()),
            Some(newest.as_str())
        );
        assert!(!app.chat_search_open, "folded so the message shows");
        assert_eq!(app.chat_search, "e", "with the search kept for later");
        app.actions.push(crate::model::Action::OpenChatSearch);
        narrow(&mut app, &mut tour, Vec::new());
        assert!(app.chat_search_open);
        assert_eq!(app.chat_search_hits.len(), 2, "as it was");
    }

    #[test]
    fn leaving_is_offered_for_a_group_and_for_a_channel() {
        for (index, leave, title, archive) in [
            (
                1usize,
                "Leave group",
                "Leave this group?",
                "Leave group and archive",
            ),
            (
                9usize,
                "Leave channel",
                "Leave this channel?",
                "Leave channel and archive",
            ),
        ] {
            let mut app = super::super::tests::app();
            prepare(&mut app);
            let ctx = egui::Context::default();
            app.attach(&ctx);
            let mut tour = Tour::new(None, None);
            let id = super::super::sample_ids()[index].to_owned();
            app.open_chat = Some(id.clone());
            app.dialog = Some(crate::model::Dialog::ChatInfo(id.clone()));
            for _ in 0..3 {
                frame(&mut app, &mut tour, &ctx, Vec::new());
            }
            // The controls are announced in the interface language, which
            // follows the system when Settings carries no choice of its own.
            let locale = app.locale;
            let leave = crate::i18n::gettext(locale, leave).to_string();
            let title = crate::i18n::gettext(locale, title).to_string();
            let archive = crate::i18n::gettext(locale, archive).to_string();
            assert!(tour.labels.contains_key(&leave), "chat info offers {leave}");
            click(&mut app, &mut tour, &ctx, &leave);
            assert!(
                matches!(
                    app.dialog,
                    Some(crate::model::Dialog::ConfirmLeaveGroup(ref open)) if *open == id
                ),
                "the button opens the confirm dialog for {id}"
            );
            for _ in 0..3 {
                frame(&mut app, &mut tour, &ctx, Vec::new());
            }
            assert!(tour.labels.contains_key(&title), "the dialog asks {title}");
            assert!(
                tour.labels.contains_key(&leave),
                "the dialog offers {leave}"
            );
            assert!(
                tour.labels.contains_key(&archive),
                "the dialog offers archiving in the same step"
            );
            // Cancelling leaves the chat alone.
            let cancel = crate::i18n::gettext(locale, "Cancel").to_string();
            click(&mut app, &mut tour, &ctx, &cancel);
            assert!(app.dialog.is_none(), "cancel closes the dialog");
            let chat = app.chat(&id).expect("chat");
            assert!(!chat.read_only, "cancelling does not leave the chat");
            assert!(chat.can_leave(&app.our_ids()), "and it stays leaveable");
        }
    }

    #[test]
    fn polls_are_created_and_voted_through_real_controls() {
        let mut app = super::super::tests::app();
        app.backend.record_demo_commands();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        let mut tour = Tour::new(None, None);
        let chat = app.open_chat.clone().unwrap();
        for multiple in [false, true] {
            app.actions
                .push(crate::model::Action::ShowDialog(Dialog::CreatePoll(
                    chat.clone(),
                )));
            frame(&mut app, &mut tour, &ctx, Vec::new());
            app.poll_draft = crate::model::PollDraft {
                question: "Lunch?".into(),
                options: vec!["Pizza".into(), "Pasta".into()],
                multiple,
            };
            for _ in 0..3 {
                frame(&mut app, &mut tour, &ctx, Vec::new());
            }
            click(&mut app, &mut tour, &ctx, "Send poll");
            assert!(app.dialog.is_none());
            assert!(!app.poll_creating);
            for _ in 0..3 {
                frame(&mut app, &mut tour, &ctx, Vec::new());
            }
            click(&mut app, &mut tour, &ctx, "Pizza");
            click(&mut app, &mut tour, &ctx, "Pasta");
            let Content::Poll { state, .. } =
                &app.conversations[&chat].messages.last().unwrap().content
            else {
                panic!("poll")
            };
            assert_eq!(state.selected, if multiple { vec![0, 1] } else { vec![1] });
            assert_eq!(state.voters, 1);
            click(&mut app, &mut tour, &ctx, "Pasta");
            if multiple {
                click(&mut app, &mut tour, &ctx, "Pizza");
            }
            let Content::Poll { state, .. } =
                &app.conversations[&chat].messages.last().unwrap().content
            else {
                panic!("poll")
            };
            assert!(state.selected.is_empty());
            assert_eq!(state.counts, vec![0, 0]);
            assert_eq!(state.voters, 0);
            assert!(app.poll_voting.is_empty());
        }
    }

    #[test]
    fn the_settings_search_finds_account_privacy_rows() {
        let mut app = super::super::tests::app();
        app.page = Page::Settings;
        app.settings_search = "profile photo".into();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        let mut tour = Tour::new(None, None);
        for _ in 0..3 {
            frame(&mut app, &mut tour, &ctx, Vec::new());
        }
        assert!(tour.labels.contains_key("Profile photo"));
        assert!(!tour.labels.contains_key("Last seen"));
        assert!(!tour.labels.contains_key("Enter sends"));
        // The section title keeps every row in it.
        app.settings_search = "privacy".into();
        for _ in 0..3 {
            frame(&mut app, &mut tour, &ctx, Vec::new());
        }
        assert!(tour.labels.contains_key("Last seen"));
        assert!(tour.labels.contains_key("Send read receipts"));
    }

    #[test]
    fn receipts_and_typing_sit_in_settings_privacy() {
        let mut app = super::super::tests::app();
        app.page = Page::Settings;
        // The section headings are translated, so pin the interface language
        // rather than reading whatever this machine is set to.
        app.settings.interface_language = Some(crate::i18n::Locale::English);
        app.locale = crate::i18n::Locale::English;
        let ctx = egui::Context::default();
        app.attach(&ctx);
        let mut tour = Tour::new(None, None);
        for _ in 0..3 {
            frame(&mut app, &mut tour, &ctx, Vec::new());
        }
        // Privacy sits below the fold. How far below depends on the platform's
        // own rows, so scroll until the section is on screen instead of by a
        // fixed distance: a few points too far and the heading leaves the top
        // of the view again, and `labels` only holds what a frame painted.
        let wheel = |delta: f32| {
            vec![
                Event::PointerMoved(pos2(590.0, 400.0)),
                Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: vec2(0.0, delta),
                    modifiers: Modifiers::NONE,
                    phase: egui::TouchPhase::Move,
                },
            ]
        };
        // The heading and the first category it writes, with both switches
        // between them, have to share one frame for the order to mean
        // anything.
        let on_screen = |tour: &Tour| {
            [
                "Privacy",
                "Send read receipts",
                "Show when you are typing",
                "Last seen",
            ]
            .iter()
            .all(|label| tour.labels.contains_key(*label))
        };
        let mut frames = 0;
        while !on_screen(&tour) && frames < 40 {
            frame(&mut app, &mut tour, &ctx, wheel(-160.0));
            frames += 1;
        }
        assert!(
            on_screen(&tour),
            "the Privacy section and its switches are on screen"
        );
        let privacy = *tour.labels.get("Privacy").expect("Privacy section");
        let last_seen = *tour.labels.get("Last seen").expect("account last seen");
        let receipts = *tour
            .labels
            .get("Send read receipts")
            .expect("read receipts");
        let typing = *tour.labels.get("Show when you are typing").expect("typing");
        // Both switches belong to the account, so they sit inside the Privacy
        // section, between its heading and the first category it writes.
        assert!(
            privacy.y < receipts.y && receipts.y < last_seen.y,
            "receipts at {receipts:?} should sit between Privacy {privacy:?} and Last seen {last_seen:?}"
        );
        assert!(
            privacy.y < typing.y && typing.y < last_seen.y,
            "typing at {typing:?} should sit between Privacy {privacy:?} and Last seen {last_seen:?}"
        );
    }

    #[test]
    fn the_theme_dropdown_selects_spotifast_palettes_and_returns_to_follow_system() {
        let mut app = super::super::tests::app();
        app.page = Page::Settings;
        app.custom_themes = crate::theme::custom::Catalog::preview(
            crate::theme::presets::themes().collect(),
            false,
        );
        let ctx = egui::Context::default();
        app.attach(&ctx);
        let mut tour = Tour::new(None, None);
        for _ in 0..3 {
            frame(&mut app, &mut tour, &ctx, Vec::new());
        }
        click(&mut app, &mut tour, &ctx, "Dark");
        let menu_pos = tour.labels["Nord.json"];
        for name in [
            "Follow system",
            "Light",
            "Dark",
            "Catppuccin Latte.json",
            "Catppuccin.json",
            "Nord.json",
            "Ristretto.json",
            "Tokyo Night.json",
            "Rose Pine.json",
            "Rose Pine Moon.json",
            "Rose Pine Dawn.json",
        ] {
            // The bundled choices now exceed the popup's visible height.
            // Scroll over the menu, as a user would, to reveal later entries.
            for _ in 0..20 {
                if tour.labels.contains_key(name) {
                    break;
                }
                frame(
                    &mut app,
                    &mut tour,
                    &ctx,
                    vec![
                        Event::PointerMoved(menu_pos),
                        Event::MouseWheel {
                            unit: egui::MouseWheelUnit::Point,
                            delta: vec2(0.0, -30.0),
                            modifiers: Modifiers::NONE,
                            phase: egui::TouchPhase::Move,
                        },
                    ],
                );
            }
            assert!(
                tour.labels.contains_key(name),
                "missing theme choice {name}"
            );
        }
        for _ in 0..20 {
            if tour.labels.contains_key("Follow system") {
                break;
            }
            frame(
                &mut app,
                &mut tour,
                &ctx,
                vec![
                    Event::PointerMoved(menu_pos),
                    Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        delta: vec2(0.0, 30.0),
                        modifiers: Modifiers::NONE,
                        phase: egui::TouchPhase::Move,
                    },
                ],
            );
        }
        click(&mut app, &mut tour, &ctx, "Nord.json");
        assert_eq!(app.settings.custom_theme.as_deref(), Some("Nord.json"));
        assert_eq!(
            app.palette.window,
            egui::Color32::from_rgb(0x2e, 0x34, 0x40)
        );
        click(&mut app, &mut tour, &ctx, "Nord.json");
        click(&mut app, &mut tour, &ctx, "Follow system");
        assert!(app.settings.custom_theme.is_none());
        assert_eq!(app.settings.theme, ThemeChoice::System);
    }

    #[test]
    fn real_input_opens_menus_completes_text_and_sends_offline_media() {
        let mut app = super::super::tests::app();
        prepare(&mut app);
        let ctx = egui::Context::default();
        app.attach(&ctx);
        let mut tour = Tour::new(None, None);
        let mut seen = [false; 7];
        for frame in 0..=42 * 60 {
            let at = frame as f32 / 60.0;
            let mut input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 800.0))),
                time: Some(at as f64),
                ..Default::default()
            };
            // Give the first screen a frame to establish its hit targets.
            if frame > 0 {
                tour.input_at(&app, &ctx, &mut input, at);
            }
            let mut output = ctx.run_ui(input, |ui| {
                app.background_frame(&ctx);
                app.frame_ui(ui);
                tour.observe(&mut app, &ctx);
                seen[0] |= egui::Popup::is_any_open(&ctx);
                seen[1] |= app.reply_to.is_some();
                seen[2] |= app.picker == Some(PickerTab::Gifs);
                seen[3] |= app.picker == Some(PickerTab::Stickers);
                seen[4] |= matches!(app.dialog, Some(Dialog::ChatInfo(_)));
                seen[5] |= matches!(app.dialog, Some(Dialog::Shortcuts));
                seen[6] |= app.settings.theme == ThemeChoice::Light;
            });
            output.textures_delta.clear();
            assert!(
                !tour.failed,
                "missing target at {at:.2}s, cue {}",
                tour.next
            );
        }
        assert_eq!(
            seen, [true; 7],
            "every advertised interaction must be visible"
        );
        let ada = &app.conversations[super::super::SAMPLES[0].id];
        let sent: Vec<_> = ada
            .messages
            .iter()
            .filter(|row| row.id.starts_with("tour-"))
            .collect();
        assert_eq!(
            sent.len(),
            2,
            "reply and still sticker through real send commands"
        );
        assert!(sent[0].quoted.is_some());
        assert!(matches!(&sent[0].content, Content::Text { text, .. }
            if text.starts_with("See you tonight! ") && !text.contains(':')));
        assert!(
            matches!(&sent[1].content, Content::Sticker { animated: false, media }
            if media.path.as_ref().unwrap().is_file())
        );
        for row in &ada.messages {
            assert!(!matches!(
                row.content,
                Content::Sticker { animated: true, .. }
            ));
        }
        let group = &app.conversations[super::super::SAMPLES[1].id];
        assert_eq!(group.messages.last().unwrap().mentions.len(), 1);
        assert_eq!(app.settings.theme, ThemeChoice::Dark);
        assert!(app.backend.is_offline());
        assert!(app.composer.is_empty());
        assert!(app.dialog.is_none());

        let mut input = egui::RawInput::default();
        input.events.push(Event::Key {
            key: Key::Space,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        });
        input.events.push(Event::Text(" ".into()));
        tour.input(&mut app, &ctx, &mut input);
        assert_eq!(tour.next, 1, "replay immediately runs the first shortcut");
        assert!(
            app.conversations[super::super::SAMPLES[0].id]
                .messages
                .iter()
                .all(|row| !row.id.starts_with("tour-"))
        );
    }
}
