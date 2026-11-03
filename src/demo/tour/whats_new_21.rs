//! "What's new in 0.21": link previews, reviewing attachments at full size,
//! upload progress with cancel, compact reactions, the composer's format
//! menu, more GIFs as you scroll, disappearing messages, blocking, chat
//! export, and muting an account.
//!
//! Every step is real pointer and keyboard input. What the network does (an
//! upload going out) arrives as the worker's events at set times.
//!
//! | Seconds     | What happens                                                   |
//! |-------------|----------------------------------------------------------------|
//! | 0.0 - 4.4   | Ada's chat: a message with a link, its card above the field    |
//! | 4.4 - 5.4   | Sent: the bubble carries the card                              |
//! | 5.8 - 10.4  | Two photos dropped: full-size review, the tile strip, the      |
//! |             | second photo picked, a caption typed                           |
//! | 10.4 - 14.8 | Sent: a progress strip for each, one cancelled, the other      |
//! |             | arrives                                                        |
//! | 15.2 - 18.8 | Rust Berlin: the photo's compact reactions, then who reacted   |
//! | 19.2 - 25.0 | The field's right-click menu makes a word bold; sent           |
//! | 25.4 - 31.0 | The GIF picker: animated results, scrolled, Load more          |
//! | 31.8 - 36.2 | Katherine Johnson's info: disappearing messages set to 7 days  |
//! | 36.8 - 40.0 | The chat menu: Block contact, confirmed                        |
//! | 40.6 - 44.0 | The chat menu: Export chat, with attachments, then Cancel      |
//! | 45.0 - 48.0 | The account switcher: the second account's notifications muted |

use super::{Cue, Gesture, Target, session};
use crate::{
    app::App,
    backend::Event,
    model::{Content, UploadGate},
    ui::focus::Stop as Control,
};
use egui::{Key as K, Modifiers, PointerButton};
use std::time::Duration;

/// Length of the tour, excluding its optional start delay.
pub const DURATION: Duration = Duration::from_secs(49);

/// The photos dropped on the chat, and the caption sent with them.
const PHOTOS: [super::super::stock::Photo; 2] =
    [super::super::stock::LAUNCH, super::super::stock::VENUE];
const DROPPED: &[&str] = &["launch.jpg", "venue.jpg"];
pub(super) const CAPTION: &str = "From launch night";
/// The uploads' ids, one per photo.
const UPLOADS: [u64; 2] = [9001, 9002];

/// Sets up the opening shot: Ada's chat in the dark theme, in English, with a
/// second account, photos to drop, a GIF search with more to load, and the
/// block list read.
pub fn prepare(app: &mut App) {
    super::common_setup(app);
    super::show_photos(app, super::super::stock::LAUNCH, super::PHOTO_CAPTION);
    for chat in &mut app.chats {
        chat.unread_mention = None;
    }
    app.settings.interface_language = Some(crate::i18n::Locale::English);
    app.locale = crate::i18n::Locale::English;
    let dir = super::media::dir(app);
    for (photo, name) in PHOTOS.into_iter().zip(DROPPED) {
        super::super::stock::save(&dir, name, photo.bytes);
    }
    // Each GIF result moves on its own picture.
    super::media::animate_gifs(app);
    // A work account beside this one, for the switcher.
    super::super::apply_flags(app, Some("accounts-closed"));
    app.blocklist.ids = Some(Default::default());
    if let Some(chat) = app.chat_mut(super::super::SAMPLES[3].id) {
        chat.ephemeral_expiration = None;
    }
    app.gif_next_offset = Some(app.gif_results.len());
    app.uploads.clear();
    app.pending.clear();
    // The tour makes no sound.
    app.video.silence();
}

/// The two photos start going out.
fn start_uploads(app: &mut App) {
    for (id, name) in UPLOADS.into_iter().zip(DROPPED) {
        upload(app, id, name, 0, None);
    }
}

fn upload(app: &App, id: u64, name: &str, percent: u8, seconds_left: Option<u32>) {
    app.backend.deliver_demo_event(Event::UploadProgress {
        id,
        gate: UploadGate::default(),
        chat: super::super::SAMPLES[0].id.to_owned(),
        name: name.to_owned(),
        percent,
        seconds_left,
    });
}

fn progress_early(app: &mut App) {
    upload(app, UPLOADS[0], DROPPED[0], 34, Some(6));
    upload(app, UPLOADS[1], DROPPED[1], 22, Some(11));
}

fn progress_late(app: &mut App) {
    upload(app, UPLOADS[0], DROPPED[0], 71, Some(2));
    upload(app, UPLOADS[1], DROPPED[1], 46, Some(7));
}

fn progress_done(app: &mut App) {
    upload(app, UPLOADS[0], DROPPED[0], 100, Some(0));
}

/// The first photo arrived: its strip goes and its message shows.
fn uploaded(app: &mut App) {
    app.backend
        .deliver_demo_event(Event::UploadEnded { id: UPLOADS[0] });
    let photo = PHOTOS[0];
    let mut media = super::super::media(
        "image/jpeg",
        photo.bytes.len() as u64,
        Some(photo.width),
        Some(photo.height),
    );
    media.path = Some(super::media::dir(app).join(DROPPED[0]));
    let chat = super::super::SAMPLES[0].id;
    let mut row = super::super::message(
        chat,
        "tour-photo",
        true,
        crate::util::now(),
        Content::Image {
            motion: None,
            caption: Some(CAPTION.to_owned()),
            media,
        },
    );
    row.thumbnail = Some(super::super::stock::thumbnail(photo));
    session::append(app, row);
}

pub(super) fn script() -> Vec<Cue> {
    use Gesture::*;
    use Target::*;
    let mut cues = Vec::new();
    let mut add = |at, gesture| cues.push(Cue { at, gesture });
    let left = PointerButton::Primary;
    let right = PointerButton::Secondary;
    let enter = |label| Key(K::Enter, Modifiers::NONE, label);
    let esc = |label| Key(K::Escape, Modifiers::NONE, label);

    // A link's card, above the field and on the sent bubble.
    add(0.5, Move(Stop(Control::Composer)));
    add(0.9, Click(left));
    add(4.4, enter("Enter · Send"));

    // Two photos dropped: reviewed at full size, captioned, sent.
    add(5.2, Move(Stop(Control::Messages)));
    add(5.8, Drop(DROPPED, "Drop two photos"));
    add(6.8, Move(Stop(Control::ReviewFile(1))));
    add(7.2, Click(left));
    add(8.2, Move(Stop(Control::Composer)));
    add(8.6, Click(left));
    add(10.4, enter("Enter · Send"));
    add(10.5, Remote(start_uploads));

    // Each upload shows how far it got; one is cancelled.
    add(11.0, Remote(progress_early));
    add(11.8, Remote(progress_late));
    add(12.0, Move(Button("Cancel sending venue.jpg")));
    add(12.6, Click(left));
    add(13.2, Remote(progress_done));
    add(13.6, Remote(uploaded));

    // Reactions take little room; a click says who reacted.
    add(14.8, Move(Label("Rust Berlin")));
    add(15.2, Click(left));
    add(16.0, Move(Reactions("group-photo")));
    add(16.4, Click(left));
    add(18.4, Move(Stop(Control::Composer)));
    add(18.8, Click(left));

    // The field's right-click menu formats the selected word.
    // A word is Option on a Mac, where Command goes to the line's start.
    add(
        21.0,
        if cfg!(target_os = "macos") {
            Key(
                K::ArrowLeft,
                Modifiers::ALT | Modifiers::SHIFT,
                "Option + Shift + ← · Select a word",
            )
        } else {
            Key(
                K::ArrowLeft,
                Modifiers::CTRL | Modifiers::SHIFT,
                "Ctrl + Shift + ← · Select a word",
            )
        },
    );
    add(21.8, Click(right));
    add(22.6, Move(Label("Bold")));
    add(23.0, Click(left));
    add(23.8, enter("Enter · Send"));

    // GIF results animate, and more load at the end.
    add(25.0, Move(Picker));
    add(25.4, Click(left));
    add(26.0, Move(Label("GIF")));
    add(26.4, Click(left));
    add(27.4, Move(Gif));
    add(27.8, Wheel(-800.0));
    add(28.8, Move(Label("Load more")));
    add(29.2, Click(left));
    add(29.8, Move(Gif));
    add(30.0, Wheel(-800.0));
    add(30.8, esc("Esc · Close GIFs"));

    // Disappearing messages from the info panel.
    add(31.4, Move(Label("Katherine Johnson")));
    add(31.8, Click(left));
    add(32.6, Move(Label("Katherine Johnson")));
    add(33.0, Click(left));
    add(34.0, Move(Label("Disappearing messages")));
    add(34.4, Click(left));
    add(35.0, Move(Label("7 days")));
    add(35.4, Click(left));
    add(36.2, esc("Esc · Close info"));

    // Block the contact.
    add(36.8, Move(Button("More")));
    add(37.2, Click(left));
    add(37.9, Move(Label("Block contact")));
    add(38.3, Click(left));
    add(39.0, Move(Label("Block")));
    add(39.4, Click(left));

    // Export the chat: the dialog, with attachments, then Cancel.
    add(40.2, Move(Button("More")));
    add(40.6, Click(left));
    add(41.2, Move(Label("Export chat…")));
    add(41.6, Click(left));
    add(42.4, Move(Label("Include downloaded attachments (ZIP)")));
    add(42.8, Click(left));
    add(43.6, Move(Label("Cancel")));
    add(44.0, Click(left));

    // Mute one account from the switcher.
    add(44.6, Move(Button("Switch account")));
    add(45.0, Click(left));
    add(45.8, Move(Button("Mute notifications")));
    add(46.2, Click(left));
    add(47.6, esc("Esc · Close"));

    for (start, text) in [
        (1.2, "Have you seen https://spotifast.rocks yet?"),
        (8.9, CAPTION),
        (19.2, "See you at the meetup tonight"),
    ] {
        for (index, character) in text.chars().enumerate() {
            cues.push(Cue {
                at: start + index as f32 * 0.035,
                gesture: Text(character),
            });
        }
    }
    cues.sort_by(|a, b| a.at.total_cmp(&b.at));
    cues
}
