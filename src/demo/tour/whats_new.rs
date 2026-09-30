//! "What's new in 0.16": the composer's plus menu, chat search, the photo
//! preview, videos, sticker shelves, message info, chat list chips, voice
//! recording and speeds, the avatar rail, multi-select, and Settings.

use super::{Cue, Gesture, Target, command};
use crate::{
    app::App,
    model::{Content, StickerShelf},
    ui::focus::Stop as Control,
};
use egui::{Key as K, Modifiers, PointerButton};
use std::time::Duration;

/// Length of the tour, excluding its optional start delay.
pub const DURATION: Duration = Duration::from_secs(86);

/// Messages the script points at, filed in Grace Hopper's chat.
pub(super) const VIDEO: &str = "tour-video";
pub(super) const NOTE: &str = "tour-note";

/// Sets up the opening shot: Ada's chat in the dark theme, in English, with
/// labels, stickers, and two videos in Grace Hopper's chat.
pub fn prepare(app: &mut App) {
    super::common_setup(app);
    app.settings.interface_language = Some(crate::i18n::Locale::English);
    app.locale = crate::i18n::Locale::English;
    super::show_photos(app, "The difference engine, finally assembled");
    super::super::labels_sample(app);
    app.label_filter = None;
    app.chat_filter = crate::model::ChatFilter::All;
    // Ctrl+B folds the list to avatars, with badges on chats the tour never
    // opens.
    for (name, unread) in [("Section 8 Berlin", 3), ("Family", 5)] {
        if let Some(chat) = app.chats.iter_mut().find(|chat| chat.name == name) {
            chat.unread = unread;
        }
    }
    super::super::sticker_sample(app, StickerShelf::Recent, "");
    app.picker = None;
    app.message_receipts = None;
    app.receipts_watch = None;
    app.recording = None;
    app.selection = None;
    app.chat_search_open = false;
    app.chat_search.clear();
    app.chat_search_hits.clear();
    app.chat_search_day = None;
    app.chat_search_calendar = false;
    videos(app);
    // The tour makes no sound.
    app.video.silence();
}

/// A video from Grace and a round video message of our own, both downloaded.
fn videos(app: &mut App) {
    let grace = super::super::SAMPLES[2].id;
    let path = app.dirs.media_cache_dir().join("tour-video.mp4");
    let _ = std::fs::create_dir_all(app.dirs.media_cache_dir());
    let _ = std::fs::write(&path, super::super::DEMO_VIDEO);
    let clip = |note: bool| {
        let mut media = super::super::media(
            "video/mp4",
            super::super::DEMO_VIDEO.len() as u64,
            Some(320),
            Some(180),
        );
        media.path = Some(path.clone());
        Content::Video {
            caption: (!note).then(|| "The relay, running again".to_owned()),
            media,
            seconds: Some(3),
            gif: false,
            note,
        }
    };
    // The clip's own picture as the poster, not a blurry placeholder.
    let poster = crate::animation::video_frame(&path, 12).and_then(|frame| {
        let [width, height] = frame.size;
        let rgb: Vec<u8> = frame
            .pixels
            .iter()
            .flat_map(|pixel| {
                let [r, g, b, _] = pixel.to_srgba_unmultiplied();
                [r, g, b]
            })
            .collect();
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 90)
            .encode(
                &rgb,
                width as u32,
                height as u32,
                image::ExtendedColorType::Rgb8,
            )
            .ok()?;
        Some(jpeg)
    });
    let Some(conversation) = app
        .conversations
        .get_mut(&crate::model::ChatId::whatsapp(grace))
    else {
        return;
    };
    let last = conversation
        .messages
        .last()
        .map_or_else(crate::util::now, |row| row.timestamp);
    for (index, (id, from_me, note)) in [(VIDEO, false, false), (NOTE, true, true)]
        .into_iter()
        .enumerate()
    {
        let mut row = super::super::message(
            grace,
            id,
            from_me,
            last + 60 * (index as i64 + 1),
            clip(note),
        );
        row.thumbnail = Some(
            poster
                .clone()
                .unwrap_or_else(|| super::super::sample_thumbnail(index as u32 + 5)),
        );
        conversation.messages.push(row);
    }
    if let Some(row) = conversation.messages.last().cloned()
        && let Some(chat) = app.chats.iter_mut().find(|chat| chat.id == grace)
    {
        chat.last_activity = row.timestamp;
        chat.last = Some(crate::model::LastMessage {
            from_me: row.from_me,
            sender: row.sender.clone(),
            sender_name: row.sender_name.clone(),
            summary: row.summary(),
            full: row.content.full_summary(),
            status: row.status,
        });
    }
}

pub(super) fn script() -> Vec<Cue> {
    use Gesture::*;
    use Target::*;
    let mut cues = Vec::new();
    let mut add = |at, gesture| cues.push(Cue { at, gesture });
    let left = PointerButton::Primary;
    let right = PointerButton::Secondary;
    let esc = |label| Key(K::Escape, Modifiers::NONE, label);

    // The rounded composer and its plus menu.
    add(0.8, Move(Stop(Control::Attach)));
    add(1.2, Click(left));
    add(2.4, Move(Label("Create poll")));
    add(2.8, Click(left));
    add(5.0, esc("Esc · Cancel the poll"));

    // Search inside the chat, narrowed to a day.
    add(5.7, Key(K::F, command(), "Ctrl + F · Search this chat"));
    add(8.6, Move(Stop(Control::ChatSearchDate)));
    add(9.0, Click(left));
    add(9.9, Move(NewestHitDay));
    add(10.3, Click(left));
    add(12.0, Move(OldestHit));
    add(12.4, Click(left));
    add(14.8, esc("Esc · Close search"));

    // Photos open in the app.
    add(15.4, Key(K::End, command(), "Ctrl + End · Latest messages"));
    add(16.2, Move(Bubble("ada-photo")));
    add(16.6, Click(left));
    add(17.9, Move(Label("Fit")));
    add(18.3, Click(left));
    add(19.3, Key(K::Plus, Modifiers::NONE, "+ · Zoom in"));
    add(21.0, esc("Esc · Close the preview"));

    // Videos play in their bubble, round video messages in their circle.
    add(21.6, Move(Label("Grace Hopper")));
    add(22.0, Click(left));
    add(22.9, Move(Bubble(VIDEO)));
    add(23.3, Click(left));
    add(26.6, Move(Bubble(NOTE)));
    add(27.0, Click(left));

    // Sticker shelves and search.
    add(30.3, Move(Picker));
    add(30.7, Click(left));
    add(31.3, Move(Label("Stickers")));
    add(31.7, Click(left));
    add(33.0, Move(Shelf(1)));
    add(33.4, Click(left));
    add(34.6, Move(Shelf(2)));
    add(35.0, Click(left));
    add(36.2, Move(Widget("sticker-search")));
    add(36.6, Click(left));
    add(38.4, Press(K::Backspace, Modifiers::NONE));
    add(40.4, esc("Esc · Close stickers"));

    // Who read a group message.
    add(41.0, Move(Label("Rust Berlin")));
    add(41.4, Click(left));
    add(42.2, Move(Bubble("group-poll")));
    add(42.6, Click(right));
    add(43.3, Move(Label("Message info")));
    add(43.7, Click(left));
    add(46.8, esc("Esc · Close message info"));

    // Favorites and label chips, and a chat's menu.
    add(47.4, Move(Label("Favorites")));
    add(47.8, Click(left));
    add(49.0, Move(Label("Work")));
    add(49.4, Click(left));
    add(50.6, Move(Label("All")));
    add(51.0, Click(left));
    add(51.6, Move(Label("Katherine Johnson")));
    add(52.0, Click(right));
    add(52.8, Move(Label("Labels")));
    add(55.0, esc("Esc · Close the menu"));

    // Record a voice message, then pick a playback speed.
    add(55.6, Move(Stop(Control::Send)));
    add(56.0, Click(left));
    add(
        59.0,
        Key(K::Enter, Modifiers::NONE, "Enter · Send voice message"),
    );
    add(59.8, Move(Bubble("tour-voice")));
    add(60.2, Click(right));
    add(60.9, Move(Label("1.75x")));
    add(61.3, Click(left));

    // The chat list folds to avatars with unread badges.
    add(
        62.2,
        Key(K::B, command(), "Ctrl + B · Collapse the chat list"),
    );
    add(
        64.6,
        Key(K::B, command(), "Ctrl + B · Expand the chat list"),
    );

    // Hover controls, then several messages forwarded at once.
    add(65.3, Move(Bubble("group-reply")));
    add(66.7, Move(BubbleCorner("group-reply")));
    add(
        67.0,
        ClickWith(left, command(), "Ctrl + click · Select a message"),
    );
    add(67.8, Move(BubbleCorner("tour-voice")));
    add(
        68.2,
        ClickWith(
            left,
            Modifiers::SHIFT,
            "Shift + click · Select the ones between",
        ),
    );
    add(69.2, Move(Label("Forward…")));
    add(69.6, Click(left));
    add(71.8, esc("Esc · Close"));
    add(72.4, esc("Esc · Cancel the selection"));

    // Settings on cards: languages, search, and the light theme.
    add(73.2, Key(K::Comma, command(), "Ctrl + , · Settings"));
    add(74.2, Move(Label("English")));
    add(74.6, Click(left));
    add(75.3, Move(Label("Deutsch")));
    add(75.7, Wheel(-160.0));
    add(76.8, esc("Esc · Close the list"));
    add(77.2, Key(K::F, command(), "Ctrl + F · Search settings"));
    add(80.4, esc("Esc · Clear the search"));
    add(81.0, Move(Label("Dark")));
    add(81.4, Click(left));
    add(81.9, Move(Label("Light")));
    add(82.3, Click(left));
    add(83.2, esc("Esc · Back to chats"));

    for (start, text) in [
        (6.1, "engine"),
        (36.9, "🐸"),
        (38.6, "bom dia"),
        (77.6, "privacy"),
    ] {
        for (index, character) in text.chars().enumerate() {
            cues.push(Cue {
                at: start + index as f32 * 0.07,
                gesture: Text(character),
            });
        }
    }
    cues.sort_by(|a, b| a.at.total_cmp(&b.at));
    cues
}
