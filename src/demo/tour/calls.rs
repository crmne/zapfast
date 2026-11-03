//! Calls: a one-to-one video call placed from the chat header, a voice call
//! answered, and a group's video call, ending on the call history.
//!
//! Every button is pressed with the pointer. The demo backend places no
//! call, so what the other side does (ringing, answering, joining) arrives
//! as the worker's updates at set times after the press that started it.
//! With `ZAPFAST_DEMO_CALL_FEEDS` set, the video tiles play those clips (see
//! `feeds`); without it they show stills.
//!
//! | Seconds     | What happens                                                     |
//! |-------------|------------------------------------------------------------------|
//! | 0.0 - 1.2   | Ada's chat; the pointer goes to the header's Video call button   |
//! | 1.2 - 2.0   | Clicked: the call screen says Connecting…                        |
//! | 2.0 - 2.8   | Calling…, with our own camera in the corner                      |
//! | 2.8 - 4.3   | Ringing…                                                         |
//! | 4.3 - 4.7   | Ada answered: Connecting…                                        |
//! | 4.7 - 6.4   | Active: Ada's video fills the window, the timer runs             |
//! | 6.4 - 8.0   | Mute, then Unmute                                                |
//! | 9.2 - 10.8  | The device pickers, open and closed                              |
//! | 12.0        | Hang up: straight back to the chat                               |
//! | 12.1 - 14.0 | The call's entry at the end of Ada's chat                        |
//! | 14.0 - 16.4 | Grace Hopper's voice call rings: Decline and Accept              |
//! | 16.4 - 21.4 | Accepted: her picture and the timer; then Hang up                |
//! | 22.6        | Rust Berlin opens                                                |
//! | 23.8        | Its header's Video call button: a group call, Connecting…        |
//! | 24.4 - 25.4 | Calling the group: everyone on it reads Invited                  |
//! | 25.4 - 26.9 | Jonas, Mira, Ada and Tom (camera off) join, with our own tile   |
//! | 26.9 - 33.0 | The group's video call runs                                      |
//! | 33.0        | Hang up                                                          |
//! | 34.2 - 39.0 | The Calls page: today's three calls on top of the history        |

use super::{Cue, Gesture, Target, session};
use crate::{
    app::App,
    calls::{CallPhase, CallUpdate, GroupCall, Participant},
    model::{CallDirection, CallStatus, Content, Delivery},
    ui::focus::Stop as Control,
};
use egui::PointerButton;
use std::time::{Duration, Instant};

/// Length of the tour, excluding its optional start delay.
pub const DURATION: Duration = Duration::from_secs(39);

/// The start of the id of every call entry the tour files.
pub(super) const ENTRY: &str = "zapfast-call-tour-";

/// When each call came up and when it was hung up, for the entries' lengths.
const VIDEO_UP: f32 = 4.7;
const VIDEO_DOWN: f32 = 12.0;
const VOICE_UP: f32 = 16.4;
const VOICE_DOWN: f32 = 21.4;
const GROUP_UP: f32 = 25.4;
const GROUP_DOWN: f32 = 33.0;

/// Who calls in the middle of the tour.
fn caller() -> &'static str {
    super::super::SAMPLES[2].id
}

/// Rust Berlin's call: the members, in roster order.
const ROSTER: [&str; 4] = [
    "491701111111@s.whatsapp.net",
    "491702222222@s.whatsapp.net",
    super::super::SAMPLES[0].id,
    "491703333333@s.whatsapp.net",
];

/// Sets up the opening shot: Ada's chat in the dark theme, in English, with
/// devices for the pickers and no desktop notification for the ringing call.
pub fn prepare(app: &mut App) {
    super::common_setup(app);
    super::show_photos(app, super::super::stock::LAUNCH, super::PHOTO_CAPTION);
    for chat in &mut app.chats {
        chat.unread_mention = None;
    }
    app.settings.interface_language = Some(crate::i18n::Locale::English);
    app.locale = crate::i18n::Locale::English;
    // The call rings in the window only: a demo never notifies the desktop.
    app.account_mut().settings.notifications = false;
    app.call = None;
    app.call_account = None;
    app.call_log.clear();
    app.call_log_loaded = false;
    app.call_devices_open = false;
    app.call_devices = crate::calls::DeviceList {
        microphones: vec![crate::calls::AudioDevice {
            id: "studio-mic".into(),
            label: "Studio Microphone".into(),
        }],
        speakers: vec![crate::calls::AudioDevice {
            id: "desk-speakers".into(),
            label: "Desk Speakers".into(),
        }],
        cameras: vec![crate::calls::CameraDevice {
            id: "0".into(),
            label: "Integrated Camera".into(),
        }],
    };
    // The tour makes no sound.
    app.video.silence();
}

/// Our offer went out: the screen says Calling, with our camera running.
fn dial(app: &mut App) {
    session::update_call(app, |call| {
        call.phase = CallPhase::Dialing;
        call.camera_on = call.video;
    });
}

/// The peer's phone rings.
fn ring(app: &mut App) {
    session::update_call(app, |call| call.phase = CallPhase::Ringing);
}

/// The peer picked up and the media is coming up.
fn accepted(app: &mut App) {
    session::update_call(app, |call| call.phase = CallPhase::Connecting);
}

/// The media is up, with the peer's picture on a video call.
fn up(app: &mut App) {
    session::update_call(app, |call| {
        call.phase = CallPhase::Active;
        call.started = Some(Instant::now());
        call.remote_video = call.video;
    });
}

/// Grace Hopper calls: a voice call rings.
fn ring_voice(app: &mut App) {
    let mut call = CallUpdate::placing(crate::calls::next_generation(), caller().to_owned(), false);
    call.direction = CallDirection::Incoming;
    call.phase = CallPhase::Incoming;
    app.backend
        .deliver_demo_event(crate::backend::Event::Call(Box::new(call)));
}

/// The group rings: everyone on it is asked and nobody has joined.
fn ring_group(app: &mut App) {
    session::update_call(app, |call| {
        call.phase = CallPhase::Dialing;
        call.camera_on = call.video;
        call.group = Some(GroupCall {
            caller: None,
            link: false,
            participants: ROSTER
                .iter()
                .map(|id| Participant {
                    id: (*id).to_owned(),
                    joined: false,
                    video: false,
                })
                .collect(),
        });
    });
}

/// `index` on the roster joins with their camera on; the first to join
/// starts the call.
fn join(app: &mut App, index: usize) {
    session::update_call(app, |call| {
        if let Some(person) = call
            .group
            .as_mut()
            .and_then(|group| group.participants.get_mut(index))
        {
            person.joined = true;
            // Tom keeps his camera off: his tile shows his picture.
            person.video = call.video && index != TOM;
        }
        if call.phase != CallPhase::Active {
            call.phase = CallPhase::Active;
            call.started = Some(Instant::now());
        }
    });
}

/// Tom's place on the roster.
const TOM: usize = 3;

fn join_0(app: &mut App) {
    join(app, 0);
}

fn join_1(app: &mut App) {
    join(app, 1);
}

fn join_2(app: &mut App) {
    join(app, 2);
}

fn join_3(app: &mut App) {
    join(app, 3);
}

/// Files a call in its chat, as the worker does once a call is over.
fn file(app: &mut App, chat: &str, name: &str, from_me: bool, content: Content) {
    let mut row = super::super::message(
        chat,
        &format!("{ENTRY}{name}"),
        from_me,
        crate::util::now(),
        content,
    );
    row.status = Delivery::None;
    session::append(app, row);
}

fn seconds(up: f32, down: f32) -> u64 {
    (down - up).round() as u64
}

fn file_video_call(app: &mut App) {
    let content = Content::Call {
        video: true,
        status: CallStatus::Answered,
        duration: seconds(VIDEO_UP, VIDEO_DOWN),
        group: false,
        participants: None,
    };
    file(app, super::super::SAMPLES[0].id, "ada", true, content);
}

fn file_voice_call(app: &mut App) {
    let content = Content::Call {
        video: false,
        status: CallStatus::Answered,
        duration: seconds(VOICE_UP, VOICE_DOWN),
        group: false,
        participants: None,
    };
    file(app, caller(), "grace", false, content);
}

fn file_group_call(app: &mut App) {
    let content = Content::Call {
        video: true,
        status: CallStatus::Answered,
        duration: seconds(GROUP_UP, GROUP_DOWN),
        group: true,
        participants: Some(ROSTER.len() as u32 + 1),
    };
    file(app, super::super::SAMPLES[1].id, "group", true, content);
}

pub(super) fn script() -> Vec<Cue> {
    use Gesture::*;
    use Target::*;
    let mut cues = Vec::new();
    let mut add = |at, gesture| cues.push(Cue { at, gesture });
    let left = PointerButton::Primary;

    // A video call from the chat header: placed, ringing, answered.
    add(0.8, Move(Button("Video call")));
    add(1.2, Click(left));
    add(2.0, Remote(dial));
    add(2.8, Remote(ring));
    add(4.3, Remote(accepted));
    add(VIDEO_UP, Remote(up));

    // Mute and unmute, then the device pickers.
    add(5.9, Move(Button("Mute")));
    add(6.4, Click(left));
    add(7.6, Move(Button("Unmute")));
    add(8.0, Click(left));
    add(8.8, Move(Button("Devices")));
    add(9.2, Click(left));
    add(10.8, Click(left));

    // Hang up: the chat again, with the call filed in it.
    add(11.6, Move(Button("Hang up")));
    add(VIDEO_DOWN, Click(left));
    add(VIDEO_DOWN + 0.1, Remote(file_video_call));

    // A voice call rings and is answered.
    add(14.0, Remote(ring_voice));
    add(15.4, Move(Button("Accept")));
    add(VOICE_UP, Click(left));
    add(21.0, Move(Button("Hang up")));
    add(VOICE_DOWN, Click(left));
    add(VOICE_DOWN + 0.1, Remote(file_voice_call));

    // A group's video call: everyone asked, then four join.
    add(22.2, Move(Label("Rust Berlin")));
    add(22.6, Click(left));
    add(23.4, Move(Button("Video call")));
    add(23.8, Click(left));
    add(24.4, Remote(ring_group));
    add(GROUP_UP, Remote(join_0));
    add(25.9, Remote(join_1));
    add(26.4, Remote(join_2));
    add(26.9, Remote(join_3));
    add(32.6, Move(Button("Hang up")));
    add(GROUP_DOWN, Click(left));
    add(GROUP_DOWN + 0.1, Remote(file_group_call));

    // The call history.
    add(33.8, Move(Stop(Control::Calls)));
    add(34.2, Click(left));
    cues.sort_by(|a, b| a.at.total_cmp(&b.at));
    cues
}
