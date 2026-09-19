//! A repeatable tour driven through the real pointer and keyboard handlers.

mod media;
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
            Target::Sticker => self.labels.get("Saved").map(|pos| *pos + vec2(25.0, 52.0)),
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
    use crate::model::{Dialog, PickerTab};

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
