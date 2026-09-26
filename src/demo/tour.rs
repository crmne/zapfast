//! A repeatable tour driven through the real pointer and keyboard handlers.

mod capture;
pub(super) mod media;
mod session;
mod whats_new;

pub use capture::Capture;

use crate::{
    app::App,
    model::{Content, Page, StickerShelf},
    settings::ThemeChoice,
    ui::focus::Stop,
};
use egui::{Event, Key, Modifiers, PointerButton, Pos2, Rect, pos2, vec2};
use serde::Serialize;
use std::{
    collections::HashMap,
    path::PathBuf,
    time::{Duration, Instant},
};

const PHOTO_CAPTION: &str = "A little poster for launch day ⚡";
/// Length of the input-driven launch tour, excluding its optional start delay.
pub const DURATION: Duration = Duration::from_secs(41);

/// Which scripted tour to play.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Script {
    /// The 41-second launch tour: search, replies, GIFs, stickers, themes.
    #[default]
    Launch,
    /// What ZapFast 0.16 added, about 90 seconds.
    WhatsNew,
}

impl Script {
    /// Names accepted by `--demo-tour-script`.
    pub const NAMES: [&str; 2] = ["launch", "whats-new"];

    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "launch" => Some(Self::Launch),
            "whats-new" => Some(Self::WhatsNew),
            _ => None,
        }
    }

    /// Length of the tour, excluding its optional start delay.
    pub fn duration(self) -> Duration {
        match self {
            Self::Launch => DURATION,
            Self::WhatsNew => whats_new::DURATION,
        }
    }

    /// Sets up the opening shot. Call only on an app populated with demo data.
    pub fn prepare(self, app: &mut App) {
        match self {
            Self::Launch => prepare(app),
            Self::WhatsNew => whats_new::prepare(app),
        }
    }

    fn cues(self) -> Vec<Cue> {
        match self {
            Self::Launch => script(),
            Self::WhatsNew => whats_new::script(),
        }
    }

    /// Wheel scrolls over the transcript: from, until, and points a second.
    fn scrolls(self) -> &'static [(f32, f32, f32)] {
        match self {
            Self::Launch => &[(3.6, 4.6, 340.0)],
            Self::WhatsNew => &[],
        }
    }
}

/// Sets up the launch tour's opening shot. Call only on an app populated
/// with demo data.
pub fn prepare(app: &mut App) {
    common_setup(app);
    super::apply_flags(app, Some("voice"));
    show_photos(app, PHOTO_CAPTION);
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

/// The state every tour opens from: offline, dark, the first chat open, and
/// local stickers and GIFs in place of downloads.
fn common_setup(app: &mut App) {
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
}

/// Shows fully loaded photos, with `caption` on Ada's, instead of the
/// deliberately blurry download previews used by the screenshot fixtures.
fn show_photos(app: &mut App, caption: &str) {
    let (photo, _) = super::sample_files(app);
    for (chat, id, caption) in [
        (super::SAMPLES[0].id, "ada-photo", caption),
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
}

#[derive(Clone, Copy)]
enum Target {
    Label(&'static str),
    Widget(&'static str),
    Bubble(&'static str),
    /// Inside a bubble's lower corner, in its margin, clear of the text and
    /// media that take clicks for themselves.
    BubbleCorner(&'static str),
    Picker,
    Gif,
    Sticker,
    /// An icon control without text, by its place in the Tab order.
    Stop(Stop),
    /// The chat search calendar's day of the newest result.
    NewestHitDay,
    /// The oldest result in the chat search pane.
    OldestHit,
    /// A sticker shelf tab: 0 recent, 1 favorites, then the packs.
    Shelf(usize),
}

enum Gesture {
    Key(Key, Modifiers, &'static str),
    /// A key without a caption, for editing text.
    Press(Key, Modifiers),
    Text(char),
    Move(Target),
    Click(PointerButton),
    /// A click with modifiers held, captioned.
    ClickWith(PointerButton, Modifiers, &'static str),
    /// Turns the wheel where the pointer is, by points; negative scrolls down.
    Wheel(f32),
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
    script: Script,
    delay: Option<Duration>,
    start: Option<Instant>,
    cues: Vec<Cue>,
    next: usize,
    previous: f32,
    pointer: Pos2,
    motion: Option<(f32, Pos2, Pos2)>,
    labels: HashMap<String, Pos2>,
    /// Every painted text and where, including repeats that `labels` folds.
    spots: Vec<(String, Pos2)>,
    trace: Vec<Trace>,
    trace_path: Option<PathBuf>,
    saved: bool,
    failed: bool,
    capture: Option<capture::Frames>,
    /// The capture's tour time for the frame in progress.
    capture_at: Option<f32>,
    /// Modifier keys held for a click, with its button until released.
    holding: Option<(Option<PointerButton>, Modifiers)>,
}

impl Tour {
    /// The launch tour.
    pub fn new(delay: Option<Duration>, trace_path: Option<PathBuf>) -> Self {
        Self::scripted(Script::Launch, delay, trace_path, None)
    }

    /// `script`, started by Space or after `delay`, or with `capture`, played
    /// on a virtual clock and saved frame by frame.
    pub fn scripted(
        script: Script,
        delay: Option<Duration>,
        trace_path: Option<PathBuf>,
        capture: Option<Capture>,
    ) -> Self {
        let capture = capture.and_then(|capture| match capture::Frames::new(capture) {
            Ok(frames) => Some(frames),
            Err(error) => {
                log::error!("could not start the frame capture: {error:#}");
                None
            }
        });
        Self {
            script,
            delay,
            start: None,
            cues: script.cues(),
            next: 0,
            previous: 0.0,
            pointer: pos2(680.0, 440.0),
            motion: None,
            labels: HashMap::new(),
            spots: Vec::new(),
            trace: Vec::new(),
            trace_path,
            saved: false,
            failed: false,
            capture,
            capture_at: None,
            holding: None,
        }
    }

    pub fn input(&mut self, app: &mut App, ctx: &egui::Context, input: &mut egui::RawInput) {
        if let Some(frames) = self.capture.as_mut() {
            self.capture_at = frames.begin(input);
            if let Some(at) = self.capture_at {
                self.input_at(app, ctx, input, at);
            }
            return;
        }
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
            self.script.prepare(app);
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
            Target::Bubble(message) => Self::bubble(app, ctx, message).map(|rect| rect.center()),
            Target::BubbleCorner(message) => {
                let rect = Self::bubble(app, ctx, message)?;
                Some(rect.right_bottom() - vec2(5.0, 2.0))
            }
            Target::Picker => app.picker_anchor.map(|rect| rect.center()),
            Target::Gif => ctx
                .read_response(egui::Id::new("gif-search"))
                .map(|r| r.rect.left_bottom() + vec2(60.0, 55.0)),
            Target::Sticker => ctx
                .data(|d| d.get_temp::<Rect>(crate::ui::picker::first_tile_id()))
                .map(|rect| rect.center()),
            Target::Stop(stop) => ctx
                .read_response(crate::ui::focus::control(ctx, stop)?)
                .map(|r| r.rect.center()),
            Target::NewestHitDay => {
                let day = crate::util::day_key(app.chat_search_hits.first()?.timestamp)?;
                let id = egui::Id::new(("chat-search-day", day.year(), day.month(), day.day()));
                ctx.read_response(id).map(|r| r.rect.center())
            }
            Target::OldestHit => {
                // The hit's line is painted in the pane, and may be in its
                // bubble too; the pane is the one outside the transcript.
                let query = app.chat_search.trim().to_lowercase();
                let view = (*app.selection_view.lock().unwrap_or_else(|p| p.into_inner()))?;
                self.spots
                    .iter()
                    .filter(|(text, pos)| {
                        let text = text.to_lowercase();
                        text != query && text.contains(&query) && !view.contains(*pos)
                    })
                    .map(|(_, pos)| *pos)
                    .max_by(|a, b| a.y.total_cmp(&b.y))
            }
            Target::Shelf(index) => {
                let shelf = match index {
                    0 => StickerShelf::Recent,
                    1 => StickerShelf::Favorites,
                    pack => StickerShelf::Pack(app.sticker_packs.get(pack - 2)?.dir.clone()),
                };
                ctx.data(|d| d.get_temp::<Rect>(crate::ui::picker::shelf_tab_id(&shelf)))
                    .map(|rect| rect.center())
            }
        }
    }

    /// The part of a bubble in the open chat that the transcript shows.
    fn bubble(app: &App, ctx: &egui::Context, message: &str) -> Option<Rect> {
        let id =
            crate::ui::conversation::bubble_id(app.open_chat.as_deref()?, message).with("rect");
        let rect = ctx.data(|d| d.get_temp::<Rect>(id))?;
        let view = (*app.selection_view.lock().unwrap_or_else(|p| p.into_inner()))?;
        let rect = rect.intersect(view);
        rect.is_positive().then_some(rect)
    }

    fn input_at(&mut self, app: &App, ctx: &egui::Context, input: &mut egui::RawInput, at: f32) {
        if self.failed {
            return;
        }
        // The button comes up a frame after it went down, and the keys a
        // frame after that, so the click reads them where it lands.
        match self.holding.take() {
            Some((Some(button), modifiers)) => {
                input.events.push(Event::PointerButton {
                    pos: self.pointer,
                    button,
                    pressed: false,
                    modifiers,
                });
                self.holding = Some((None, modifiers));
            }
            Some((None, _)) => input.events.push(Event::ModifiersChanged(Modifiers::NONE)),
            None => {}
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
                Gesture::Click(button) => self.click(input, at, button),
                Gesture::ClickWith(button, modifiers, label) => {
                    // The click reads the held keys from the input state,
                    // which keeps them until the next change. Pressed and
                    // released in one frame, it would not count as a click
                    // on a message.
                    input.events.push(Event::ModifiersChanged(modifiers));
                    input.events.push(Event::PointerButton {
                        pos: self.pointer,
                        button,
                        pressed: true,
                        modifiers,
                    });
                    self.holding = Some((Some(button), modifiers));
                    self.trace_click(at, button);
                    self.caption(at, label);
                }
                Gesture::Press(key, modifiers) => press(input, key, modifiers),
                Gesture::Key(key, modifiers, label) => {
                    press(input, key, modifiers);
                    self.caption(at, label);
                }
                Gesture::Text(character) => input.events.push(Event::Text(character.to_string())),
                Gesture::Wheel(points) => input.events.push(Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: vec2(0.0, points),
                    modifiers: Modifiers::NONE,
                    phase: egui::TouchPhase::Move,
                }),
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
        for &(from, until, speed) in self.script.scrolls() {
            let scroll = (at.min(until) - self.previous.max(from)).max(0.0) * speed;
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

    fn click(&mut self, input: &mut egui::RawInput, at: f32, button: PointerButton) {
        for pressed in [true, false] {
            input.events.push(Event::PointerButton {
                pos: self.pointer,
                button,
                pressed,
                modifiers: Modifiers::NONE,
            });
        }
        self.trace_click(at, button);
    }

    fn trace_click(&mut self, at: f32, button: PointerButton) {
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

    fn caption(&mut self, at: f32, label: &str) {
        self.trace.push(Trace {
            at,
            event: TraceEvent::Keys {
                label: crate::ui::keys::label(label),
            },
        });
    }

    pub fn drive(&mut self, _app: &mut App, ctx: &egui::Context) {
        if let Some(frames) = self.capture.as_mut() {
            let duration = self.script.duration().as_secs_f32();
            if frames.end(ctx, self.capture_at, duration, self.failed) {
                self.save_trace(ctx);
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            return;
        }
        let now = Instant::now();
        if let Some(delay) = self.delay.take() {
            self.start = Some(now + delay);
        }
        let Some(start) = self.start else {
            return;
        };
        if now < start {
            ctx.request_repaint_after(start - now);
        } else if start.elapsed() < self.script.duration() && !self.failed {
            ctx.request_repaint_after(Duration::from_millis(16));
        } else {
            self.save_trace(ctx);
        }
    }

    fn save_trace(&mut self, ctx: &egui::Context) {
        if std::mem::replace(&mut self.saved, true) {
            return;
        }
        if let Some(path) = &self.trace_path {
            let data = serde_json::json!({ "width": ctx.content_rect().width(),
                "height": ctx.content_rect().height(),
                "duration": self.script.duration().as_secs(),
                "complete": !self.failed, "events": self.trace });
            if let Err(error) = std::fs::write(path, data.to_string()) {
                log::error!("could not write tour input trace: {error}");
            }
        }
    }

    /// Finds click targets in the actual painted UI; no view-specific hooks or
    /// hard-coded menu coordinates are needed. Called after frame_ui.
    pub fn observe(&mut self, app: &mut App, ctx: &egui::Context) {
        session::respond(app);
        self.labels.clear();
        self.spots.clear();
        let layers: Vec<_> = ctx.memory(|memory| memory.layer_ids().collect());
        for layer in layers {
            let transform = ctx.layer_transform_to_global(layer).unwrap_or_default();
            ctx.graphics(|graphics| {
                if let Some(list) = graphics.get(layer) {
                    for clipped in list.all_entries() {
                        if let egui::Shape::Text(text) = &clipped.shape {
                            let rect = Rect::from_min_size(text.pos, text.galley.size());
                            if rect.intersects(clipped.clip_rect) {
                                let spot = transform * rect.center();
                                self.labels.insert(text.galley.text().to_owned(), spot);
                                self.spots.push((text.galley.text().to_owned(), spot));
                            }
                        }
                    }
                }
            });
        }
    }
}

/// Presses and releases `key`.
fn press(input: &mut egui::RawInput, key: Key, modifiers: Modifiers) {
    for pressed in [true, false] {
        input.events.push(Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers,
        });
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

    /// The starred panel lists the message itself, and the open chat knows
    /// which messages are starred, which is what its footer paints from.
    #[test]
    fn the_starred_list_shows_the_message_and_the_mark() {
        let mut app = super::super::tests::app();
        prepare(&mut app);
        let ctx = egui::Context::default();
        app.attach(&ctx);
        let mut tour = Tour::new(None, None);
        frame(&mut app, &mut tour, &ctx, Vec::new());
        let chat = app.open_chat.clone().expect("a chat is open");
        let source = app.conversations[&chat]
            .messages
            .first()
            .expect("the chat has messages")
            .clone();
        let body = source.content.full_summary();
        // The star reaches the open chat: this is what the footer paints from.
        app.stars
            .insert(chat.clone(), [source.id.clone()].into_iter().collect());
        app.show_starred = true;
        app.starred = vec![crate::archive::Starred {
            message: source.clone(),
            starred_at: crate::util::now(),
        }];
        frame(&mut app, &mut tour, &ctx, Vec::new());
        assert!(
            tour.labels.contains_key("Starred"),
            "the header names the panel"
        );
        assert!(
            tour.labels.contains_key(&body),
            "the row shows the starred message itself"
        );
        assert!(
            app.stars[&chat].contains(&source.id),
            "the open chat knows which messages are starred"
        );
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
        app.custom_themes =
            crate::theme::Catalog::preview(crate::theme::presets().collect(), false);
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
    fn the_whats_new_tour_reaches_every_feature_through_real_input() {
        let mut app = super::super::tests::app();
        Script::WhatsNew.prepare(&mut app);
        let ctx = egui::Context::default();
        app.attach(&ctx);
        let mut tour = Tour::scripted(Script::WhatsNew, None, None, None);
        let features = [
            "plus menu",
            "poll dialog",
            "chat search results",
            "day calendar",
            "day filter",
            "jump to a result",
            "photo preview",
            "zoomed photo",
            "video in its bubble",
            "round video message",
            "favorite stickers",
            "sticker pack",
            "sticker search by emoji",
            "sticker search by pack",
            "message info with receipts",
            "favorites chip",
            "label chip",
            "chat menu with labels",
            "voice recorder",
            "sent voice message",
            "playback speed",
            "avatar rail",
            "hover reply control",
            "multi-select",
            "forward dialog",
            "language list",
            "settings search",
            "light theme",
        ];
        let mut seen = [false; 28];
        let duration = Script::WhatsNew.duration().as_secs_f32();
        let group = super::super::SAMPLES[1].id;
        for frame in 0..=((duration + 1.0) * 60.0) as usize {
            let at = frame as f32 / 60.0;
            // The recorder hears in real time; give it three real seconds.
            if (55.9..59.1).contains(&at) {
                std::thread::sleep(Duration::from_millis(16));
            }
            let mut input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 800.0))),
                time: Some(at as f64),
                ..Default::default()
            };
            if frame > 0 {
                tour.input_at(&app, &ctx, &mut input, at);
            }
            let mut output = ctx.run_ui(input, |ui| {
                app.background_frame(&ctx);
                app.frame_ui(ui);
                tour.observe(&mut app, &ctx);
                let hovered = Tour::bubble(&app, &ctx, "group-reply")
                    .is_some_and(|rect| rect.contains(tour.pointer));
                let checks = [
                    app.composer_tools_open,
                    matches!(app.dialog, Some(Dialog::CreatePoll(_))),
                    app.chat_search_open && app.chat_search_hits.len() >= 3,
                    app.chat_search_calendar,
                    app.chat_search_day.is_some() && !app.chat_search_hits.is_empty(),
                    app.jump_highlight.is_some(),
                    app.image_preview.is_some(),
                    app.image_preview
                        .as_ref()
                        .is_some_and(|preview| !preview.is_fit()),
                    app.video.status(whats_new::VIDEO).is_some(),
                    app.video.status(whats_new::NOTE).is_some(),
                    app.picker == Some(PickerTab::Stickers)
                        && app.sticker_shelf == StickerShelf::Favorites,
                    app.picker == Some(PickerTab::Stickers)
                        && matches!(app.sticker_shelf, StickerShelf::Pack(_)),
                    app.sticker_search == "🐸",
                    app.sticker_search == "bom dia",
                    matches!(app.dialog, Some(Dialog::MessageInfo { .. }))
                        && app.message_receipts.is_some(),
                    app.chat_filter == crate::model::ChatFilter::Favorites,
                    app.label_filter.as_deref() == Some("label-work"),
                    tour.labels.contains_key("Mark as unread")
                        && tour.labels.contains_key("Follow up")
                        && egui::Popup::is_any_open(&ctx),
                    app.recording.is_some(),
                    app.conversations[group].message("tour-voice").is_some(),
                    app.player.speed() == 1.75,
                    app.sidebar_mode() == crate::model::SidebarDisplayMode::CollapsedIconsOnly,
                    hovered && app.selection.is_none(),
                    app.selection
                        .as_ref()
                        .is_some_and(|(_, selected)| selected.len() >= 2),
                    matches!(app.dialog, Some(Dialog::Forward { .. })),
                    app.page == Page::Settings && tour.labels.contains_key("Русский"),
                    app.settings_search == "privacy" && tour.labels.contains_key("Last seen"),
                    app.settings.theme == ThemeChoice::Light,
                ];
                for (seen, check) in seen.iter_mut().zip(checks) {
                    *seen |= check;
                }
            });
            output.textures_delta.clear();
            assert!(
                !tour.failed,
                "missing target at {at:.2}s, cue {}",
                tour.next
            );
        }
        let missing: Vec<_> = features
            .iter()
            .zip(seen)
            .filter(|(_, seen)| !seen)
            .map(|(feature, _)| *feature)
            .collect();
        assert!(missing.is_empty(), "never shown: {missing:?}");
        // It ends in the light theme, back in the chats, in English, with
        // nothing left open.
        assert_eq!(app.page, Page::Chats);
        assert_eq!(app.open_chat.as_deref(), Some(group));
        assert_eq!(app.settings.theme, ThemeChoice::Light);
        assert_eq!(
            app.settings.interface_language,
            Some(crate::i18n::Locale::English)
        );
        assert!(app.dialog.is_none());
        assert!(app.selection.is_none());
        assert!(app.recording.is_none());
        assert!(app.image_preview.is_none());
        assert!(app.backend.is_offline());
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
