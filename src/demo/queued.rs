//! Offline send-queue demo: the real worker and queue over a synthetic link
//! (`backend::SyntheticLink`), so every state on screen comes from the code
//! that runs for real sends. No network, keyring, or user data: the archive
//! is in memory and the WhatsApp client is never run.
//!
//! The synthetic transport, also described in the intro message:
//! - a send takes 400 ms, then succeeds;
//! - when five sends started in the last 10 seconds, the next one is
//!   refused with a typed rate limit and a 15-second cooldown;
//! - the text "fail" is refused for good, so it shows as not sent;
//! - during a cooldown, the contact writes every 4 seconds.
//!
//! The window wakes only for the next due event: no polling.

use super::{App, Content, Delivery, SAMPLES, message};
use crate::backend::{Backend, Command, Event, SendFailure, SyntheticLink};
use crate::model::Action;
use std::{
    collections::VecDeque,
    sync::mpsc::Sender,
    time::{Duration, Instant},
};

const SEND_TIME: Duration = Duration::from_millis(400);
/// The first send of the opening scene, so a screenshot still sees it.
const SLOW_SEND_TIME: Duration = Duration::from_secs(3);
const BURST: usize = 5;
const BURST_WINDOW: Duration = Duration::from_secs(10);
const COOLDOWN: u32 = 15;
const REPLY_EVERY: Duration = Duration::from_secs(4);
/// Messages the stress page sends by itself when it opens.
const STRESS_SENDS: usize = 25;

const INTRO: &str = "Synthetic offline demo of the send queue. Nothing leaves this computer. \
Each send takes 0.4 seconds. Send more than five messages within 10 seconds and the next one \
meets a 15-second limit: it waits at the bottom with Cancel and sends by itself when the limit \
ends. During a limit, this contact writes every 4 seconds. A message that says fail is not sent. \
Below, a limit has just ended: its first message takes 3 seconds to send.";

pub(crate) struct QueueDemo {
    link: SyntheticLink,
    /// The interface's event channel, as the real backend's.
    events: Sender<Event>,
    /// Start times of accepted sends, for the burst limit.
    accepted: VecDeque<Instant>,
    on_wire: Option<OnWire>,
    /// A started send that takes `SLOW_SEND_TIME`.
    slow: Option<String>,
    next_reply: Option<Instant>,
    replies: usize,
    /// Tests: moves the demo's clock ahead of the real one.
    #[cfg(test)]
    pub(super) skew: Duration,
}

/// The one send the synthetic transport is working on.
struct OnWire {
    chat: String,
    id: String,
    /// `None` until the demo first sees it, at its first frame.
    due: Option<Instant>,
    result: Result<(), SendFailure>,
}

/// Opens the demo on its first chat. `stress` sends a burst by itself.
pub(super) fn setup(app: &mut App, stress: bool) {
    let (chat, other) = (SAMPLES[0].id, SAMPLES[2].id);
    app.chats.retain(|row| row.id == chat || row.id == other);
    // Rows come from the worker's archive, through the real events. Demo
    // mode has no phone history to ask for.
    app.conversations.clear();
    for id in [chat, other] {
        app.conversations.insert(
            id.into(),
            super::Conversation {
                requested: true,
                phone_exhausted: true,
                ..Default::default()
            },
        );
    }
    app.open_chat = Some(chat.into());
    app.page = crate::model::Page::Chats;
    app.composer.clear();
    app.reply_to = None;
    app.focus_composer = true;
    let (backend, events) = Backend::detached();
    app.backend = backend;
    app.backend.record_demo_commands();
    let mut link = SyntheticLink::new(&[(chat, "Queue demo (offline)"), (other, SAMPLES[2].name)]);
    let now = crate::util::now();
    link.seed(message(
        chat,
        "queue-demo-intro",
        false,
        now - 600,
        Content::text(INTRO),
    ));
    for (n, (from_me, text)) in SAMPLES[2].lines.iter().take(2).enumerate() {
        link.seed(message(
            other,
            &format!("queue-demo-other-{n}"),
            *from_me,
            now - 900 + n as i64,
            Content::text(*text),
        ));
    }
    let mut demo = QueueDemo {
        link,
        events,
        accepted: VecDeque::new(),
        on_wire: None,
        slow: None,
        next_reply: None,
        replies: 0,
        #[cfg(test)]
        skew: Duration::ZERO,
    };
    if stress {
        // Every fourth goes to the other chat, so the list order moves too.
        for n in 1..=STRESS_SENDS {
            app.actions.push(Action::SendText {
                chat: if n % 4 == 0 { other } else { chat }.into(),
                text: format!("Stress message {n} of {STRESS_SENDS}"),
                quoting: None,
            });
        }
    } else {
        demo.scene(chat, now);
    }
    // The interface loads each chat as one page, as from a real archive, so
    // it knows there is no older history to ask the phone for.
    for event in demo.link.events() {
        if matches!(event, Event::ChatUpdated(_)) {
            let _ = demo.events.send(event);
        }
    }
    for id in [chat, other] {
        demo.link.command(Command::LoadChat {
            chat: id.into(),
            before: None,
        });
    }
    demo.forward();
    app.queued_demo = Some(demo);
}

impl QueueDemo {
    /// One row of each state, made by the real queue: a limit has just
    /// ended, its first message is sending, and two more wait their turn.
    fn scene(&mut self, chat: &str, now: i64) {
        let mut sent = message(
            chat,
            "queue-demo-sent",
            true,
            now - 300,
            Content::text("This message was sent normally."),
        );
        sent.status = Delivery::Sent;
        self.link.seed(sent);
        let mut interrupted = message(
            chat,
            "queue-demo-unconfirmed",
            true,
            now - 240,
            Content::text(
                "Synthetic interrupted send. This may have been sent; it will not retry.",
            ),
        );
        interrupted.status = Delivery::Unconfirmed;
        self.link.seed(interrupted);
        for text in [
            "This one is sending now.",
            "This one waits for its turn.",
            "So does this one.",
        ] {
            self.link.command(Command::SendText {
                chat: chat.into(),
                text: text.into(),
                quoting: None,
                mentions: Vec::new(),
            });
        }
        // WhatsApp refused the first: all three wait out the cooldown.
        for (chat, id) in self.link.started() {
            self.slow = Some(id.clone());
            self.link.finish(
                chat,
                id,
                Err(SendFailure::RateLimited {
                    retry_after: COOLDOWN,
                }),
            );
        }
        self.link.seed(message(
            chat,
            "queue-demo-reply",
            false,
            now - 1,
            Content::text("A reply that arrived during the limit."),
        ));
        // The cooldown ends: the first goes out as a new message.
        if let Some(end) = self.link.deadline() {
            self.link.pump(end);
        }
        self.take_started(Instant::now());
    }

    /// Sends the worker's events to the interface. Desktop notifications
    /// are not part of the demo.
    fn forward(&mut self) -> bool {
        let mut any = false;
        for event in self.link.events() {
            if !matches!(event, Event::Incoming { .. }) {
                let _ = self.events.send(event);
                any = true;
            }
        }
        any
    }

    /// Ends the send on the wire when due, runs the queue's timer, and
    /// starts whatever the queue sent next.
    fn advance(&mut self, now: Instant) {
        if let Some(wire) = self
            .on_wire
            .take_if(|wire| wire.due.is_some_and(|due| due <= now))
        {
            self.link.finish(wire.chat, wire.id, wire.result);
        }
        if self.link.deadline().is_some_and(|due| due <= now) {
            self.link.pump(now);
        }
        self.take_started(now);
        if let Some(wire) = self.on_wire.as_mut()
            && wire.due.is_none()
        {
            let time = if self.slow.as_ref() == Some(&wire.id) {
                self.slow = None;
                SLOW_SEND_TIME
            } else {
                SEND_TIME
            };
            wire.due = Some(now + time);
        }
        self.reply(now);
    }

    /// Decides how each started send ends, by the transport rules above.
    fn take_started(&mut self, now: Instant) {
        for (chat, id) in self.link.started() {
            self.accepted
                .retain(|at| now.saturating_duration_since(*at) < BURST_WINDOW);
            let fail = self
                .link
                .message(&chat, &id)
                .is_some_and(|row| matches!(&row.content, Content::Text { text, .. } if text.trim().eq_ignore_ascii_case("fail")));
            let result = if fail {
                Err(SendFailure::Failed {
                    kind: "demo refusal",
                    code: None,
                })
            } else if self.accepted.len() >= BURST {
                Err(SendFailure::RateLimited {
                    retry_after: COOLDOWN,
                })
            } else {
                self.accepted.push_back(now);
                Ok(())
            };
            self.on_wire = Some(OnWire {
                chat,
                id,
                due: None,
                result,
            });
        }
    }

    /// During a cooldown the contact of the first waiting message writes,
    /// so the rule that waiting messages stay last is visible.
    fn reply(&mut self, now: Instant) {
        let Some(end) = self.link.cooldown_end(now) else {
            self.next_reply = None;
            return;
        };
        let due = *self.next_reply.get_or_insert(now + REPLY_EVERY);
        if due > now {
            return;
        }
        self.next_reply = Some(due + REPLY_EVERY).filter(|next| *next < end);
        if let Some(chat) = self.link.waiting_chat() {
            self.replies += 1;
            let text = format!("Synthetic reply {} during the limit.", self.replies);
            self.link.receive(message(
                &chat,
                &format!("queue-demo-reply-{}", self.replies),
                false,
                crate::util::now(),
                Content::text(text),
            ));
        }
    }

    /// The next moment something is due, for one timed repaint.
    fn next_due(&self, now: Instant) -> Option<Instant> {
        let reply = self
            .next_reply
            .filter(|_| self.link.cooldown_end(now).is_some());
        [
            self.on_wire.as_ref().and_then(|wire| wire.due),
            self.link.deadline(),
            reply,
        ]
        .into_iter()
        .flatten()
        .min()
    }
}

/// Runs after each frame's actions: hands the commands the interface sent
/// to the worker and wakes the window for the next due event.
pub(crate) fn respond(app: &mut App, ctx: &egui::Context, now: Instant) {
    let Some(mut demo) = app.queued_demo.take() else {
        return;
    };
    #[cfg(test)]
    let now = now + demo.skew;
    // A cooldown that ended applies before new sends are filed.
    demo.advance(now);
    for command in app.backend.take_demo_commands() {
        match command {
            Command::SendText { .. }
            | Command::CancelQueued { .. }
            | Command::DeleteLocal { .. }
            | Command::LoadChat { .. }
            | Command::SaveDraft { .. } => demo.link.command(command),
            Command::MarkRead { chat, .. } => demo.link.read(&chat),
            _ => {}
        }
    }
    demo.advance(now);
    if demo.forward() {
        ctx.request_repaint();
    }
    if let Some(next) = demo.next_due(now) {
        ctx.request_repaint_after(next.saturating_duration_since(now));
    }
    app.queued_demo = Some(demo);
}

#[cfg(test)]
mod tests {
    use super::super::tests::{app, render};
    use super::*;

    /// Each row of a chat as (ours, status, text), in display order.
    fn rows(app: &App, chat: &str) -> Vec<(bool, Delivery, String)> {
        app.conversations[chat]
            .messages
            .iter()
            .map(|row| {
                let text = match &row.content {
                    Content::Text { text, .. } => text.clone(),
                    _ => String::new(),
                };
                (row.from_me, row.status, text)
            })
            .collect()
    }

    /// The page opens on one row of each state, all from the real worker:
    /// sent, unconfirmed, a reply that came during a limit, the first
    /// message after the limit still sending, and two waiting at the bottom.
    #[test]
    fn the_page_opens_on_every_send_state_from_the_real_queue() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        super::super::apply_flags(&mut app, Some("queued-messages"));
        render(&mut app, &ctx);
        let chat = SAMPLES[0].id;
        let statuses: Vec<_> = rows(&app, chat)
            .into_iter()
            .map(|(_, status, _)| status)
            .collect();
        assert_eq!(
            statuses,
            [
                Delivery::None,
                Delivery::Sent,
                Delivery::Unconfirmed,
                Delivery::None,
                Delivery::Pending,
                Delivery::Queued,
                Delivery::Queued,
            ]
        );
        assert_eq!(app.chats.len(), 2, "a second chat to switch to");
    }

    /// Moves the demo's clock forward and lays out a few frames.
    fn advance(app: &mut App, ctx: &egui::Context, by: Duration) {
        app.queued_demo.as_mut().expect("queue demo").skew += by;
        render(app, ctx);
    }

    fn texts_with(app: &App, chat: &str, status: Delivery) -> Vec<String> {
        rows(app, chat)
            .into_iter()
            .filter(|(ours, row, _)| *ours && *row == status)
            .map(|(_, _, text)| text)
            .collect()
    }

    /// Presses the real Cancel control of a waiting bubble.
    fn click_cancel(app: &mut App, ctx: &egui::Context, chat: &str, id: &str) {
        let rect = ctx
            .data(|data| {
                data.get_temp::<egui::Rect>(crate::ui::conversation::cancel_queued_id(chat, id))
            })
            .expect("a waiting bubble shows its Cancel control");
        for pressed in [true, false] {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1180.0, 780.0),
                    )),
                    events: vec![
                        egui::Event::PointerMoved(rect.center()),
                        egui::Event::PointerButton {
                            pos: rect.center(),
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                    ..Default::default()
                },
                |ui| {
                    app.background_frame(ui.ctx());
                    app.frame_ui(ui);
                },
            );
            output.textures_delta.clear();
        }
        render(app, ctx);
    }

    /// The real send and cancel actions drive the real queue: the scene
    /// drains, a burst meets the limit, the rest wait at the bottom below a
    /// reply, Cancel deletes one, and the others send in order afterwards.
    #[test]
    fn a_burst_meets_the_limit_waits_below_replies_and_sends_in_order() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        super::super::apply_flags(&mut app, Some("queued-messages"));
        render(&mut app, &ctx);
        let chat = SAMPLES[0].id;
        let last = app.conversations[chat].messages.last().unwrap().id.clone();
        click_cancel(&mut app, &ctx, chat, &last);
        assert!(
            app.conversations[chat].message(&last).is_none(),
            "Cancel deletes the waiting message"
        );
        advance(&mut app, &ctx, Duration::from_secs(4));
        advance(&mut app, &ctx, Duration::from_secs(1));
        assert_eq!(
            texts_with(&app, chat, Delivery::Sent).len(),
            3,
            "{:?}",
            rows(&app, chat)
        );
        let burst: Vec<String> = (1..=8).map(|n| format!("Burst {n}")).collect();
        for text in &burst {
            app.actions.push(crate::model::Action::SendText {
                chat: chat.into(),
                text: text.clone(),
                quoting: None,
            });
        }
        render(&mut app, &ctx);
        // Normal sends look as before: a clock tick, never Waiting.
        assert!(texts_with(&app, chat, Delivery::Queued).is_empty());
        for _ in 0..10 {
            advance(&mut app, &ctx, Duration::from_millis(500));
        }
        let waiting = texts_with(&app, chat, Delivery::Queued);
        assert!(!waiting.is_empty(), "{:?}", rows(&app, chat));
        assert_eq!(waiting, burst[burst.len() - waiting.len()..]);
        advance(&mut app, &ctx, Duration::from_millis(4500));
        let order = rows(&app, chat);
        let first_waiting = order
            .iter()
            .position(|(_, status, _)| *status == Delivery::Queued)
            .unwrap();
        assert!(
            order[first_waiting..]
                .iter()
                .all(|(_, status, _)| *status == Delivery::Queued),
            "waiting rows stay at the bottom: {order:?}"
        );
        assert!(
            !order[first_waiting - 1].0,
            "a reply during the limit sits above them: {order:?}"
        );
        for _ in 0..80 {
            advance(&mut app, &ctx, Duration::from_millis(500));
        }
        let sent: Vec<String> = texts_with(&app, chat, Delivery::Sent)
            .into_iter()
            .filter(|text| text.starts_with("Burst"))
            .collect();
        assert_eq!(sent, burst, "{:?}", rows(&app, chat));
        assert!(
            app.toasts.is_empty(),
            "a rate limit is not an error to report"
        );
    }

    /// The stress page sends a burst across both chats on its own; it meets
    /// the limit several times and still ends with every message sent.
    #[test]
    fn the_stress_page_meets_the_limit_and_sends_everything_in_the_end() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        super::super::apply_flags(&mut app, Some("queued-messages-stress"));
        render(&mut app, &ctx);
        let (a, b) = (SAMPLES[0].id, SAMPLES[2].id);
        let ours = |app: &App| {
            rows(app, a)
                .into_iter()
                .chain(rows(app, b))
                .filter(|(ours, _, text)| *ours && text.starts_with("Stress"))
                .count()
        };
        assert_eq!(ours(&app), STRESS_SENDS);
        let mut limited = false;
        for _ in 0..240 {
            advance(&mut app, &ctx, Duration::from_millis(500));
            limited |= !texts_with(&app, a, Delivery::Queued).is_empty();
            if texts_with(&app, a, Delivery::Sent).len() + texts_with(&app, b, Delivery::Sent).len()
                == STRESS_SENDS
            {
                break;
            }
        }
        assert!(limited, "the burst meets the limit");
        assert!(!texts_with(&app, b, Delivery::Sent).is_empty());
        assert_eq!(
            texts_with(&app, a, Delivery::Sent).len() + texts_with(&app, b, Delivery::Sent).len(),
            STRESS_SENDS
        );
    }

    /// A tiny seeded generator, so the stress run is the same every time.
    struct Lcg(u64);

    impl Lcg {
        fn below(&mut self, bound: u64) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (self.0 >> 33) % bound
        }
    }

    #[derive(Debug, PartialEq)]
    enum Log {
        Dispatched(String),
        Deleted(String),
        Finished(String, &'static str),
        /// Failed at once without a link, and never dispatched.
        Refused(String),
    }

    /// Stress: hundreds of sends across three chats through the real queue,
    /// with scripted rate limits, failures, lost links and cancels at seeded
    /// points, mirrored into a real interface.
    #[test]
    fn a_seeded_stress_run_keeps_every_queue_invariant() {
        let chats = [
            "4917600000001@s.whatsapp.net",
            "4917600000002@s.whatsapp.net",
            "4917600000003@s.whatsapp.net",
        ];
        let mut link = SyntheticLink::new(&[
            (chats[0], "Stress one"),
            (chats[1], "Stress two"),
            (chats[2], "Stress three"),
        ]);
        let mut app = app();
        let ctx = egui::Context::default();
        let (backend, interface) = crate::backend::Backend::detached();
        app.backend = backend;
        let mut random = Lcg(0x5eed_cafe);
        let mut log = Vec::new();
        // Every message we sent, by chat and id, in the order sent.
        let mut sent: Vec<(String, String)> = Vec::new();
        let mut on_wire: Option<(String, String)> = None;
        let mut connected = true;
        let sync = |link: &mut SyntheticLink,
                    app: &mut App,
                    log: &mut Vec<Log>,
                    sent: &mut Vec<(String, String)>,
                    on_wire: &mut Option<(String, String)>,
                    connected: bool| {
            for (chat, id) in link.started() {
                assert!(on_wire.is_none(), "one send at a time");
                log.push(Log::Dispatched(id.clone()));
                *on_wire = Some((chat, id));
            }
            for event in link.events() {
                match &event {
                    Event::Messages { messages, .. } => {
                        for row in messages {
                            if row.from_me && !sent.iter().any(|(_, id)| *id == row.id) {
                                sent.push((row.chat.clone(), row.id.clone()));
                            }
                        }
                    }
                    Event::MessageDeleted { id, .. } => log.push(Log::Deleted(id.clone())),
                    Event::MessageUpdated(row)
                        if row.from_me
                            && row.status == Delivery::Failed
                            && !log.contains(&Log::Finished(row.id.clone(), "failed")) =>
                    {
                        // Only a lost link fails a message that never left.
                        assert!(!connected, "{} failed while connected", row.id);
                        log.push(Log::Refused(row.id.clone()));
                    }
                    _ => {}
                }
                interface.send(event).unwrap();
            }
            app.background_frame(&ctx);
            for chat in chats {
                let Some(conversation) = app.conversations.get(chat) else {
                    continue;
                };
                assert!(
                    conversation
                        .messages
                        .iter()
                        .skip_while(|row| !row.waiting_to_send())
                        .all(|row| row.waiting_to_send()),
                    "waiting rows sort last in {chat}"
                );
                // They show in the queue's order, which is the order sent,
                // also after a send that waited is refused again.
                let ranks: Vec<usize> = conversation
                    .messages
                    .iter()
                    .filter(|row| row.waiting_to_send())
                    .map(|row| {
                        sent.iter()
                            .position(|(_, id)| *id == row.id)
                            .expect("a waiting row was sent")
                    })
                    .collect();
                assert!(
                    ranks.is_sorted(),
                    "waiting rows out of queue order in {chat}"
                );
            }
            // Idle means a future deadline or none: the worker never spins.
            assert!(
                link.deadline().is_none_or(|due| due > Instant::now()),
                "a due deadline left unserved"
            );
        };
        for step in 0..900 {
            let chat = chats[random.below(3) as usize];
            match random.below(100) {
                0..=39 => link.command(Command::SendText {
                    chat: chat.into(),
                    text: format!("Stress step {step}"),
                    quoting: None,
                    mentions: Vec::new(),
                }),
                40..=49 if !sent.is_empty() => {
                    // Mostly what a reader would cancel: a row not sent yet.
                    let unsent: Vec<&(String, String)> = sent
                        .iter()
                        .filter(|(chat, id)| {
                            app.conversations[chat].message(id).is_some_and(|row| {
                                matches!(row.status, Delivery::Queued | Delivery::Pending)
                            })
                        })
                        .collect();
                    let (chat, id) = if !unsent.is_empty() && random.below(5) > 0 {
                        unsent[random.below(unsent.len() as u64) as usize].clone()
                    } else {
                        sent[random.below(sent.len() as u64) as usize].clone()
                    };
                    if random.below(2) == 0 {
                        link.command(Command::CancelQueued { chat, id });
                    } else {
                        link.command(Command::DeleteLocal { chat, id });
                    }
                }
                50..=79 => {
                    if let Some((chat, id)) = on_wire.take() {
                        let (result, outcome) = match (connected, random.below(20)) {
                            (true, 0..=12) | (false, 0..=9) => (Ok(()), "sent"),
                            (true, 13..=16) => (
                                Err(SendFailure::RateLimited {
                                    retry_after: 30 + random.below(90) as u32,
                                }),
                                "limited",
                            ),
                            _ => (
                                Err(SendFailure::Failed {
                                    kind: "stress failure",
                                    code: None,
                                }),
                                "failed",
                            ),
                        };
                        log.push(Log::Finished(id.clone(), outcome));
                        link.finish(chat, id, result);
                    }
                }
                80..=91 => {
                    if let Some(due) = link.deadline() {
                        link.pump(due);
                    }
                }
                _ => {
                    connected = !connected;
                    link.set_connected(connected);
                }
            }
            sync(
                &mut link,
                &mut app,
                &mut log,
                &mut sent,
                &mut on_wire,
                connected,
            );
        }
        // The link returns and every message still waiting goes out.
        connected = true;
        link.set_connected(true);
        sync(
            &mut link,
            &mut app,
            &mut log,
            &mut sent,
            &mut on_wire,
            connected,
        );
        for _ in 0..10_000 {
            if let Some((chat, id)) = on_wire.take() {
                log.push(Log::Finished(id.clone(), "sent"));
                link.finish(chat, id, Ok(()));
            } else if let Some(due) = link.deadline() {
                link.pump(due);
            } else {
                break;
            }
            sync(
                &mut link,
                &mut app,
                &mut log,
                &mut sent,
                &mut on_wire,
                connected,
            );
        }
        assert!(connected && on_wire.is_none() && link.deadline().is_none());
        assert!(sent.len() > 300, "hundreds of sends: {}", sent.len());

        let deleted: Vec<&String> = log
            .iter()
            .filter_map(|entry| match entry {
                Log::Deleted(id) => Some(id),
                _ => None,
            })
            .collect();
        assert!(deleted.len() > 10, "cancels happened: {}", deleted.len());
        let refused = log
            .iter()
            .filter(|entry| matches!(entry, Log::Refused(_)))
            .count();
        assert!(refused > 10, "sends without a link failed: {refused}");
        for (chat, id) in &sent {
            let mine: Vec<&Log> = log
                .iter()
                .filter(|entry| match entry {
                    Log::Dispatched(of)
                    | Log::Deleted(of)
                    | Log::Finished(of, _)
                    | Log::Refused(of) => of == id,
                })
                .collect();
            // Each dispatch ends once, and only a pre-send rate refusal
            // dispatches the same message again.
            let count =
                |wanted: fn(&Log) -> bool| mine.iter().filter(|entry| wanted(entry)).count();
            let dispatched = count(|entry| matches!(entry, Log::Dispatched(_)));
            let limited = count(|entry| matches!(entry, Log::Finished(_, "limited")));
            let ended = count(|entry| matches!(entry, Log::Finished(_, "sent" | "failed")));
            assert_eq!(dispatched, limited + ended, "{id}: {mine:?}");
            // A deleted message, cancelled or already sent, is never
            // dispatched or finished afterwards.
            if let Some(at) = mine
                .iter()
                .position(|entry| matches!(entry, Log::Deleted(_)))
            {
                assert_eq!(at, mine.len() - 1, "{id}: {mine:?}");
                continue;
            }
            // Every other message ends sent or failed, as its last attempt
            // did, or failed without a link before it was ever dispatched.
            let status = link.message(chat, id).map(|row| row.status);
            let expected = match mine.last() {
                Some(Log::Finished(_, "sent")) => Delivery::Sent,
                _ => Delivery::Failed,
            };
            let refused = count(|entry| matches!(entry, Log::Refused(_)));
            assert_eq!(
                (ended + refused, status),
                (1, Some(expected)),
                "{id}: {mine:?}"
            );
            if refused > 0 {
                assert_eq!(dispatched, 0, "{id}: {mine:?}");
            }
            assert_eq!(
                app.conversations[chat].message(id).map(|row| row.status),
                status
            );
        }
        // One queue for the account: messages go out, and end, in the
        // order they were sent, a refused one included.
        let rank = |id: &String| sent.iter().position(|(_, sent)| sent == id).unwrap();
        let mut first = Vec::new();
        for entry in &log {
            if let Log::Dispatched(id) = entry
                && !first.contains(&rank(id))
            {
                first.push(rank(id));
            }
        }
        assert!(first.is_sorted(), "first dispatches out of order");
        let ends: Vec<usize> = log
            .iter()
            .filter_map(|entry| match entry {
                Log::Finished(id, "sent" | "failed") => Some(rank(id)),
                _ => None,
            })
            .collect();
        assert!(ends.is_sorted(), "messages ended out of order");
    }
}
