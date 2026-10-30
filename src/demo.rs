//! Offline sample data for screenshots, recorded tours, and headless UI tests.

pub mod stock;
pub mod tour;

#[cfg(test)]
mod focus_trace;

use std::collections::HashMap;

use crate::app::{App, Conversation, Presence};
use crate::backend::LinkStatus;
use crate::calls::{CallPhase, CallUpdate};
use crate::model::CallDirection;
use crate::model::{
    Chat, ChatMedia, Contact, Content, Delivery, Dialog, InfoPanel, InfoView, LinkPreview, Media,
    MediaTab, MentionRef, Message, Page, Quoted, Reaction,
};
use crate::settings::ThemeChoice;

const ME: &str = "15550001111@s.whatsapp.net";
/// Family, the sample group we run.
const FAMILY: &str = "120363098765432109@g.us";
/// A member of every sample group.
const JONAS: &str = "491701111111@s.whatsapp.net";
/// A contact in no group, offered by Add members.
const DOROTHY: &str = "12025550137@s.whatsapp.net";
/// A made-up invite link for the sample group.
const DEMO_INVITE_LINK: &str = "https://chat.whatsapp.com/DemoFamilyInvite2026";

struct Sample {
    id: &'static str,
    name: &'static str,
    minutes_ago: i64,
    unread: u32,
    pinned: bool,
    muted: bool,
    archived: bool,
    locked: bool,
    lines: &'static [(bool, &'static str)],
}

/// A small JPEG attachment preview: one of the stock pictures, by `seed`.
pub fn sample_thumbnail(seed: u32) -> Vec<u8> {
    stock::thumbnail_for(seed)
}

/// A small street-map picture standing in for a location's map preview.
fn sample_map() -> Vec<u8> {
    let (width, height) = (208u32, 120u32);
    let image = image::RgbImage::from_fn(width, height, |x, y| {
        let (x, y) = (x as i32, y as i32);
        let river = (y - (60 + (x - 104) * (x - 104) / 180)).abs() < 5;
        let park = (130..185).contains(&x) && (12..44).contains(&y);
        let road = (x - 70).abs() < 3 || (y - 88).abs() < 3 || (x + y - 190).abs() < 3;
        let street = x % 34 == 0 || y % 26 == 0;
        image::Rgb(if road {
            [250, 214, 120]
        } else if river {
            [158, 196, 230]
        } else if park {
            [190, 222, 170]
        } else if street {
            [255, 255, 255]
        } else {
            [236, 232, 222]
        })
    });
    let mut bytes = Vec::new();
    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 85);
    let _ = encoder.encode_image(&image);
    bytes
}

const SAMPLES: &[Sample] = &[
    Sample {
        id: "393331234567@s.whatsapp.net",
        name: "Ada Lovelace",
        minutes_ago: 3,
        unread: 2,
        pinned: true,
        muted: false,
        archived: false,
        locked: false,
        lines: &[
            (false, "Did the analytical engine build finish?"),
            (true, "Yes! It compiles on stable now, no nightly needed."),
            (
                false,
                "That is wonderful news. Send me the branch when you can.",
            ),
            (
                false,
                "Also: https://en.wikipedia.org/wiki/Analytical_engine for the bedtime reading 😄",
            ),
        ],
    },
    Sample {
        id: "120363012345678901@g.us",
        name: "Rust Berlin",
        minutes_ago: 25,
        unread: 14,
        pinned: false,
        muted: true,
        archived: false,
        locked: false,
        lines: &[
            (false, "Anyone at the meetup tonight?"),
            (true, "I'll be there around 19:00"),
            (false, "Same. Bringing the egui demo"),
            (false, "Save me a seat 🙏"),
        ],
    },
    Sample {
        id: "441632960123@s.whatsapp.net",
        name: "Grace Hopper",
        minutes_ago: 90,
        unread: 0,
        pinned: false,
        muted: false,
        archived: false,
        locked: false,
        lines: &[
            (true, "Found the bug. It was a moth."),
            (false, "Literally?"),
            (true, "Literally. Taped it into the logbook."),
        ],
    },
    Sample {
        id: "4915112345678@s.whatsapp.net",
        name: "Katherine Johnson",
        minutes_ago: 60 * 26,
        unread: 0,
        pinned: false,
        muted: false,
        archived: false,
        locked: false,
        lines: &[
            (false, "Talk is cheap. Show me the code."),
            (true, "Pushed 😌"),
        ],
    },
    Sample {
        id: "120363098765432109@g.us",
        name: "Family",
        minutes_ago: 60 * 50,
        unread: 0,
        pinned: false,
        muted: false,
        archived: false,
        locked: false,
        lines: &[
            (false, "Dinner on Sunday at 13:00?"),
            (true, "We'll be there"),
            (false, "Bring the good bread 🥖"),
        ],
    },
    Sample {
        id: "14155550199@s.whatsapp.net",
        name: "Margaret Hamilton",
        minutes_ago: 60 * 24 * 4,
        unread: 0,
        pinned: false,
        muted: false,
        archived: false,
        locked: false,
        lines: &[
            (false, "The landing software held up."),
            (true, "Never doubted it."),
        ],
    },
    Sample {
        id: "120363011122233344@g.us",
        name: "Section 8 Berlin",
        minutes_ago: 60 * 5,
        unread: 0,
        pinned: false,
        muted: false,
        archived: false,
        locked: false,
        lines: &[
            (
                false,
                "*ARTIST CARE Timetable*\n\n20:00 – 21:00 @491701111111 (no pronouns)\n21:00 – 23:00 Melissa (she/they)\n23:00 – 07:00 @491703333333 (she/her)",
            ),
            (
                false,
                "We'd really appreciate if you could take 3 minutes to check out our *vision & values*. It helps set the tone for a smooth and healthy collaboration 💜\nsection8berlin.com\n\nIn the next days we will drop some more information🔥\n* Guestlist\n* Dresscode\n* Coatcheck",
            ),
        ],
    },
    Sample {
        id: "972501234567@s.whatsapp.net",
        name: "Yael",
        minutes_ago: 60 * 3,
        unread: 1,
        pinned: false,
        muted: false,
        archived: false,
        locked: true,
        lines: &[(false, "הכלב הגדול קפץ"), (true, "OK הכלב end")],
    },
    Sample {
        id: "33612345678@s.whatsapp.net",
        name: "Dentist",
        minutes_ago: 60 * 24 * 12,
        unread: 0,
        pinned: false,
        muted: false,
        archived: true,
        locked: false,
        lines: &[(false, "Reminder: your appointment is on Tuesday at 9:30.")],
    },
    Sample {
        id: "120363055566677788@newsletter",
        name: "Rust Weekly",
        minutes_ago: 60 * 8,
        unread: 0,
        pinned: false,
        muted: false,
        archived: false,
        locked: false,
        lines: &[(false, "A new client build is out.")],
    },
];

/// Builds synthetic attachment metadata for media-state and album regression tests.
fn media(mime: &str, size: u64, width: Option<u32>, height: Option<u32>) -> Media {
    Media {
        mime: mime.to_owned(),
        size,
        width,
        height,
        album: None,
        path: None,
        state: Default::default(),
    }
}

/// Generates a speech-like demo waveform.
fn demo_waveform() -> Vec<u8> {
    (0..crate::voice::BARS)
        .map(|index| {
            let t = index as f32 * 0.55;
            (18.0 + 70.0 * (t.sin() * (t * 0.37).cos()).abs()) as u8
        })
        .collect()
}

/// Synthetic mixed-direction lines for the `rtl-self` page, one message each.
///
/// They cover neutrals, numbers, brackets, embedded Latin, a repeated-letter
/// word pair, bold, a link, emoji inside Hebrew and Arabic paragraphs, and
/// Arabic lam ligatures in a line long enough to wrap.
pub const RTL_SELF_CHAT: [&str; 3] = [
    "בדיקת RTL בלבד\nסער + מירון = ❤️\nשלום ❤️ עולם\nשלום (test 123) עולם!\nשלום 12:34, מחיר 50₪.\nHello שלום עולם world ❤️\nمرحبا بالعالم ❤️ (123)\nשלום 👨‍👩‍👧‍👦 עולם",
    "שלום!\nשלום 123\nHello שלום עולם end\nאב גד בא\nשלום (עולם)\nשלום *עולם* !\nשלום https://example.com עולם",
    "إلى السطر التالي\nالله أكبر، لا بأس 🌙\nهذا نص عربي طويل يختبر ترتيب الأسطر عندما تلتف الكلمات داخل فقاعة رسالة ضيقة إلى السطر التالي",
];

/// Numbers inside right-to-left text on the `rtl` page: Arabic-Indic and
/// European digits, a time, and a phone number, each reading left to right.
const RTL_NUMBERS: &str = "لدي ٤٥ رسالة، الساعة ١٢:٣٠\nعندي 45 رسالة\nاتصل على +00 (00) 00000-0000";

fn message(chat: &str, id: &str, from_me: bool, timestamp: i64, content: Content) -> Message {
    Message {
        id: id.to_owned(),
        chat: chat.to_owned(),
        sender: if from_me {
            ME.to_owned()
        } else {
            chat.to_owned()
        },
        sender_name: None,
        from_me,
        timestamp,
        content,
        status: if from_me {
            Delivery::Read
        } else {
            Delivery::None
        },
        delivered_at: None,
        read_at: None,
        quoted: None,
        reactions: Vec::new(),
        history_order: None,
        edited: false,
        mentions: Vec::new(),
        forwarded: false,
        thumbnail: None,
    }
}

/// Writes sample attachments and generated profile pictures to disk.
fn plant_avatars(app: &mut App) {
    let dir = app.dirs.avatar_cache_dir();
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let mut everyone: Vec<String> = sample_ids().iter().map(|id| (*id).to_owned()).collect();
    everyone.push(ME.to_owned());
    everyone.extend(app.contacts.keys().cloned());
    for chat in &app.chats {
        everyone.extend(chat.participants.iter().cloned());
    }
    for id in everyone {
        let kind = if id.ends_with("@g.us") {
            "group"
        } else {
            "person"
        };
        let name = format!("demo-{kind}-{}.jpg", crate::util::hue(&id) as u32);
        if let Some(picture) = stock::avatar(&id) {
            app.adopt_avatar(&id, stock::save(&dir, &name, picture));
            continue;
        }
        let path = dir.join(name);
        if !path.exists() {
            let picture = painted_avatar(&id);
            if picture.save(&path).is_err() {
                continue;
            }
        }
        app.adopt_avatar(&id, path);
    }
}

/// Generates one 128-pixel id-colored profile picture, for the chats
/// without a stock one.
fn painted_avatar(id: &str) -> image::RgbImage {
    let hue = crate::util::hue(id);
    let motif = (crate::util::hue(&format!("motif-{id}")) as u32) % 4;
    let side = 128u32;
    image::RgbImage::from_fn(side, side, |px, py| {
        let x = px as f32 / side as f32;
        let y = py as f32 / side as f32;
        match motif {
            0 => {
                // Landscape.
                let sky = tint(hue, 0.35, 0.92 - y * 0.25);
                let sun = ((x - 0.68) * (x - 0.68) + (y - 0.30) * (y - 0.30)).sqrt() < 0.13;
                let near = y > 0.62 + (x - 0.30).abs() * 0.9;
                let far = y > 0.55 + (x - 0.75).abs() * 1.1;
                if near {
                    tint(hue, 0.45, 0.35)
                } else if far {
                    tint(hue, 0.40, 0.5)
                } else if sun {
                    tint(hue + 40.0, 0.55, 0.95)
                } else {
                    sky
                }
            }
            1 => {
                // Portrait silhouette.
                let head = ((x - 0.5) * (x - 0.5) + (y - 0.40) * (y - 0.40)).sqrt() < 0.17;
                let shoulders = {
                    let dx = (x - 0.5) / 0.34;
                    let dy = (y - 1.02) / 0.42;
                    dx * dx + dy * dy < 1.0
                };
                if head || shoulders {
                    tint(hue, 0.40, 0.38)
                } else {
                    tint(hue, 0.30, 0.88 - y * 0.15)
                }
            }
            2 => {
                // Overlapping circles.
                let a = ((x - 0.35) * (x - 0.35) + (y - 0.38) * (y - 0.38)).sqrt() < 0.26;
                let b = ((x - 0.66) * (x - 0.66) + (y - 0.62) * (y - 0.62)).sqrt() < 0.30;
                match (a, b) {
                    (true, true) => tint(hue + 60.0, 0.55, 0.55),
                    (true, false) => tint(hue + 30.0, 0.50, 0.70),
                    (false, true) => tint(hue - 20.0, 0.50, 0.62),
                    _ => tint(hue, 0.28, 0.90),
                }
            }
            _ => {
                // Leaf silhouette.
                let leaf = {
                    let dx = (x - 0.5) / 0.24;
                    let dy = (y - 0.48) / 0.36;
                    let lean = dx + dy * 0.5;
                    lean * lean + dy * dy < 1.0
                };
                let stem = (x - 0.52).abs() < 0.02 && y > 0.45 && y < 0.92;
                if leaf || stem {
                    tint(hue + 90.0, 0.45, 0.45)
                } else {
                    tint(hue, 0.25, 0.90 - y * 0.10)
                }
            }
        }
    })
}

/// Converts HSV to pixel color.
fn tint(hue: f32, saturation: f32, value: f32) -> image::Rgb<u8> {
    let hue = hue.rem_euclid(360.0) / 60.0;
    let chroma = value * saturation;
    let second = chroma * (1.0 - (hue % 2.0 - 1.0).abs());
    let (r, g, b) = match hue as u32 {
        0 => (chroma, second, 0.0),
        1 => (second, chroma, 0.0),
        2 => (0.0, chroma, second),
        3 => (0.0, second, chroma),
        4 => (second, 0.0, chroma),
        _ => (chroma, 0.0, second),
    };
    let base = value - chroma;
    image::Rgb([
        ((r + base) * 255.0) as u8,
        ((g + base) * 255.0) as u8,
        ((b + base) * 255.0) as u8,
    ])
}

fn sample_files(app: &App) -> (std::path::PathBuf, std::path::PathBuf) {
    let dir = app.dirs.media_cache_dir();
    let photo = stock::save_photo(&dir, stock::SPACEWALK);
    let sticker = stock::save(&dir, "demo-sticker.webp", stock::ANIMATED_STICKER);
    (photo, sticker)
}

/// Loads the sample account and opens its first chat.
pub fn populate(app: &mut App) {
    app.backend.set_offline(true);
    // With no backend to answer them, the interface's commands are answered
    // here instead.
    app.backend.record_demo_commands();
    // Demo mode has no backend to handle downloads.
    app.account_mut().settings.auto_download = false;
    app.link = LinkStatus::Connected;
    app.me = Some(ME.to_owned());
    app.me_name = Some("Carmine".to_owned());
    app.chats.clear();
    app.conversations.clear();
    let now = crate::util::now();
    let group_members = [
        ("491701111111@s.whatsapp.net", "Jonas"),
        ("491702222222@s.whatsapp.net", "Mira"),
        ("491703333333@s.whatsapp.net", "Tom"),
    ];
    for (id, name) in group_members {
        app.contacts.insert(
            id.to_owned(),
            Contact {
                id: id.to_owned(),
                full_name: Some(name.to_owned()),
                first_name: None,
                push_name: None,
            },
        );
    }
    // Include a contact without a chat for search results.
    app.contacts.insert(
        "12025550137@s.whatsapp.net".to_owned(),
        Contact {
            id: "12025550137@s.whatsapp.net".to_owned(),
            full_name: Some("Dorothy Vaughan".to_owned()),
            first_name: None,
            push_name: None,
        },
    );
    for sample in SAMPLES {
        let mut chat = Chat::new(sample.id.to_owned(), sample.name.to_owned());
        chat.last_activity = now - sample.minutes_ago * 60;
        chat.unread = sample.unread;
        // Two favorites, in the phone's order rather than by recency.
        chat.favorite = matches!(sample.name, "Ada Lovelace" | "Margaret Hamilton");
        chat.favorite_position = u32::from(sample.name == "Ada Lovelace");
        // One chat carries the empty dot, so the sample shows both marks.
        chat.marked_unread = sample.name == "Grace Hopper";
        chat.pinned = sample.pinned;
        chat.pinned_at = if sample.pinned {
            (now - sample.minutes_ago * 60) * 1000
        } else {
            0
        };
        chat.muted_until = sample.muted.then_some(0);
        chat.archived = sample.archived;
        chat.locked = sample.locked;
        // Someone in Rust Berlin mentioned us among its unread messages.
        if sample.name == "Rust Berlin" {
            chat.unread_mention = Some(format!("{}-2", sample.id));
        }
        let mut conversation = Conversation {
            complete: true,
            requested: true,
            // Demo mode has no phone connection.
            phone_exhausted: true,
            ..Default::default()
        };
        let count = sample.lines.len() as i64;
        for (index, (from_me, text)) in sample.lines.iter().enumerate() {
            let timestamp = chat.last_activity - (count - index as i64 - 1) * 60 * 7;
            let mut row = message(
                sample.id,
                &format!("{}-{index}", sample.id),
                *from_me,
                timestamp,
                Content::text(*text),
            );
            if chat.is_group() && !from_me {
                let (sender, name) = group_members[index % group_members.len()];
                row.sender = sender.to_owned();
                row.sender_name = Some(name.to_owned());
                row.mentions = group_members
                    .iter()
                    .map(|(id, _)| MentionRef {
                        user: id.split('@').next().unwrap_or_default().to_owned(),
                        id: (*id).to_owned(),
                    })
                    .collect();
            }
            conversation.messages.push(row);
        }
        if chat.is_group() {
            chat.participants = group_members
                .iter()
                .map(|(id, _)| (*id).to_owned())
                .chain(std::iter::once(ME.to_owned()))
                .collect();
            chat.read_only = sample.name == "Section 8 Berlin";
            // Rust Berlin lets every member edit its info, Family is locked
            // but we are an admin, and Section 8 Berlin is locked for us.
            chat.info_locked = Some(sample.name != "Rust Berlin");
            chat.admin = sample.name == "Family";
            // Family is ours to run, with Mira as a second admin; Rust Berlin
            // lets its members add people and share its link; Section 8
            // Berlin is Tom's announcement group.
            chat.group_settings = Some(match sample.name {
                "Family" => crate::model::GroupSettings {
                    approval: true,
                    admins: vec![ME.to_owned(), group_members[1].0.to_owned()],
                    super_admins: vec![ME.to_owned()],
                    ..Default::default()
                },
                "Rust Berlin" => crate::model::GroupSettings {
                    members_add: true,
                    members_share_link: true,
                    admins: vec![group_members[0].0.to_owned()],
                    super_admins: vec![group_members[0].0.to_owned()],
                    ..Default::default()
                },
                _ => crate::model::GroupSettings {
                    announce: true,
                    admins: vec![group_members[2].0.to_owned()],
                    super_admins: vec![group_members[2].0.to_owned()],
                    ..Default::default()
                },
            });
            chat.group_description = Some(if sample.name == "Rust Berlin" {
                "Rust meetups in Berlin 🦀\nShare projects, ask questions, and help each other learn.\nNext meetup: Thursday at 19:00.".to_owned()
            } else {
                String::new()
            });
        }
        chat.last = conversation
            .messages
            .last()
            .map(|last| crate::model::LastMessage {
                labelled: last.content.labelled(),
                from_me: last.from_me,
                sender: last.sender.clone(),
                sender_name: last.sender_name.clone(),
                summary: last.summary(),
                full: last.content.full_summary(),
                status: last.status,
            });
        app.conversations.insert(sample.id.to_owned(), conversation);
        app.chats.push(chat);
    }

    plant_avatars(app);
    // Cover every supported bubble type in the first chat.
    let (photo, sticker) = sample_files(app);
    let ada = SAMPLES[0].id;
    let base = app.chats[0].last_activity;
    let older = base - 60 * 60 * 30;
    // Put representative messages at the end for screenshots.
    let latest = vec![
        {
            let mut row = message(
                ada,
                "ada-photo",
                false,
                base + 30,
                Content::Image {
                    motion: None,
                    caption: Some("The difference engine, finally assembled".into()),
                    media: media(
                        "image/jpeg",
                        1_843_201,
                        Some(stock::ENGINE.width),
                        Some(stock::ENGINE.height),
                    ),
                },
            );
            row.thumbnail = Some(stock::thumbnail(stock::ENGINE));
            row.reactions.push(Reaction {
                sender: ME.into(),
                from_me: true,
                emoji: "❤️".into(),
            });
            row.reactions.push(Reaction {
                sender: ada.into(),
                from_me: false,
                emoji: "😂".into(),
            });
            row
        },
        message(
            ada,
            "ada-doc",
            true,
            base + 60,
            Content::Document {
                media: media("application/pdf", 482_113, None, None),
                file_name: "Notes on the Engine.pdf".into(),
                caption: None,
                pages: Some(12),
            },
        ),
        {
            let mut row = message(
                ada,
                "ada-voice",
                false,
                base + 90,
                Content::Audio {
                    media: media("audio/ogg; codecs=opus", 71_002, None, None),
                    seconds: Some(42),
                    voice_note: true,
                    waveform: demo_waveform(),
                },
            );
            // The reply below says it was heard.
            row.status = Delivery::Played;
            row
        },
        message(
            ada,
            "you-voice",
            true,
            base + 95,
            Content::Audio {
                media: media("audio/ogg; codecs=opus", 24_113, None, None),
                seconds: Some(11),
                voice_note: true,
                waveform: demo_waveform(),
            },
        ),
        {
            let mut row = message(
                ada,
                "ada-reply",
                true,
                base + 120,
                Content::text("Listened, agreed on *all three* points."),
            );
            row.quoted = Some(Quoted {
                labelled: None,
                chat: None,
                id: "ada-voice".into(),
                sender: ada.into(),
                sender_name: Some("Ada Lovelace".into()),
                summary: "Voice message (0:42)".into(),
                mentions: Vec::new(),
            });
            row.edited = true;
            row.status = Delivery::Delivered;
            row
        },
        {
            let mut row = message(
                ada,
                "ada-link",
                true,
                base + 150,
                Content::Text {
                    text: "btw I made my own Spotify app from scratch! https://spotifast.rocks/".into(),
                    preview: Some(LinkPreview {
                        url: "https://spotifast.rocks/".into(),
                        title: Some("spotifast.rocks".into()),
                        description: Some("Spotify, native and fast. A lightweight Spotify client written in Rust with egui.".into()),
                    }),
                },
            );
            row.thumbnail = Some(stock::thumbnail(stock::SPOTIFAST));
            row
        },
    ];
    let extra = vec![
        {
            let mut row = message(
                ada,
                "ada-video",
                true,
                older + 60,
                Content::Video {
                    caption: None,
                    media: media("video/mp4", 820_000, Some(1280), Some(720)),
                    seconds: Some(5),
                    gif: false,
                    note: false,
                },
            );
            row.thumbnail = Some(stock::thumbnail(stock::LAUNCH));
            row
        },
        message(
            ada,
            "ada-format",
            false,
            older + 60 * 16,
            Content::text(
                "_Reading list_ for the weekend:\n* ~Babbage's memoirs~ done\n* `sketch.rs` from the repo\n> and the essay you sent 🙏\nMail me at ada@analytical.engine or see engine.rocks",
            ),
        ),
        message(
            ada,
            "ada-emoji",
            true,
            older + 60 * 17,
            Content::text("😂🎉"),
        ),
        {
            let mut row = message(
                ada,
                "ada-tall",
                false,
                older + 60 * 18,
                Content::Image {
                    motion: None,
                    caption: None,
                    media: media(
                        "image/jpeg",
                        stock::SPACEWALK.bytes.len() as u64,
                        Some(stock::SPACEWALK.width),
                        Some(stock::SPACEWALK.height),
                    ),
                },
            );
            row.forwarded = true;
            if let Content::Image { media, .. } = &mut row.content {
                media.path = Some(photo.clone());
            }
            row
        },
        {
            let mut row = message(
                ada,
                "ada-sticker",
                true,
                older + 60 * 19,
                Content::Sticker {
                    media: media(
                        "image/webp",
                        stock::ANIMATED_STICKER.len() as u64,
                        Some(256),
                        Some(256),
                    ),
                    animated: true,
                },
            );
            if let Content::Sticker { media, .. } = &mut row.content {
                media.path = Some(sticker.clone());
            }
            row
        },
        message(
            ada,
            "ada-location",
            false,
            older + 60 * 20,
            Content::Location {
                latitude: 51.5237,
                longitude: -0.1585,
                name: Some("Ada's place".into()),
                address: Some("12 St James's Square, London".into()),
            },
        ),
        {
            let mut row = message(
                ada,
                "ada-live",
                false,
                older + 60 * 21,
                Content::LiveLocation {
                    latitude: 51.5074,
                    longitude: -0.1278,
                    accuracy_m: Some(24),
                    speed_mps: Some(1.4),
                    heading_deg: Some(90),
                    sequence: 1,
                    ended: false,
                    updated: 0,
                    newer_on_phone: false,
                },
            );
            row.thumbnail = Some(sample_map());
            row
        },
        // A photo sent to be viewed once, which opens only on the phone.
        message(
            ada,
            "ada-view-once",
            false,
            older + 60 * 22,
            Content::PhoneOnly {
                view_once: true,
                live_location: false,
                once: Some(crate::model::OnceMedia::Photo),
            },
        ),
        message(ada, "ada-deleted", false, older + 60 * 25, Content::Revoked),
    ];
    let conversation = app.conversations.get_mut(ada).expect("sample chat");
    conversation.messages.splice(0..0, extra);
    conversation.messages.extend(latest);

    // Cover a group image, mentioned reply, and poll.
    let group = SAMPLES[1].id;
    let group_base = app.chats[1].last_activity;
    let (jonas, mira, tom) = (group_members[0], group_members[1], group_members[2]);
    let group_extra = vec![
        {
            let mut row = message(
                group,
                "group-photo",
                false,
                group_base + 60,
                Content::Image {
                    motion: None,
                    caption: Some("Tonight's venue, doors at 18:30".into()),
                    media: media(
                        "image/jpeg",
                        1_204_551,
                        Some(stock::VENUE.width),
                        Some(stock::VENUE.height),
                    ),
                },
            );
            row.sender = tom.0.to_owned();
            row.sender_name = Some(tom.1.to_owned());
            row.thumbnail = Some(stock::thumbnail(stock::VENUE));
            row.reactions.push(Reaction {
                sender: jonas.0.into(),
                from_me: false,
                emoji: "🔥".into(),
            });
            row.reactions.push(Reaction {
                sender: mira.0.into(),
                from_me: false,
                emoji: "🏆".into(),
            });
            row.reactions.push(Reaction {
                sender: ME.into(),
                from_me: true,
                emoji: "🔥".into(),
            });
            row
        },
        {
            let mut row = message(
                group,
                "group-reply",
                false,
                group_base + 120,
                Content::text(format!(
                    "@{} will do, front row",
                    jonas.0.split('@').next().unwrap_or_default()
                )),
            );
            row.sender = mira.0.to_owned();
            row.sender_name = Some(mira.1.to_owned());
            row.quoted = Some(Quoted {
                labelled: None,
                chat: None,
                id: format!("{group}-3"),
                sender: jonas.0.into(),
                sender_name: Some(jonas.1.into()),
                summary: "Save me a seat 🙏".into(),
                mentions: Vec::new(),
            });
            row.mentions = vec![MentionRef {
                user: jonas.0.split('@').next().unwrap_or_default().to_owned(),
                id: jonas.0.to_owned(),
            }];
            row
        },
        {
            let mut row = message(
                group,
                "group-everyone",
                true,
                group_base + 165,
                Content::text("Heading over @all"),
            );
            row.mentions = vec![crate::mentions::everyone_ref()];
            row
        },
        message(
            group,
            "group-poll",
            true,
            group_base + 180,
            Content::Poll {
                question: "Pizza after the talks?".into(),
                state: crate::model::PollState {
                    selectable: 1,
                    counts: vec![3, 2, 0],
                    selected: vec![0],
                    voters: 5,
                    can_vote: true,
                    history_complete: true,
                    ..Default::default()
                },
                options: vec!["Yes".into(), "Only if it's Neapolitan".into(), "No".into()],
            },
        ),
    ];
    app.conversations
        .get_mut(group)
        .expect("sample group")
        .messages
        .extend(group_extra);

    // Sync chat-row previews with each conversation's last message.
    // chats and conversations live on Account; Deref would treat a joint
    // borrow as one exclusive lock, so the previews are collected first.
    let previews: Vec<(crate::model::ChatId, i64, crate::model::LastMessage)> = app
        .chats
        .iter()
        .filter_map(|chat| {
            app.conversations
                .get(&chat.id)
                .and_then(|conversation| conversation.messages.last())
                .map(|last| {
                    (
                        chat.id.clone(),
                        last.timestamp,
                        crate::model::LastMessage {
                            labelled: last.content.labelled(),
                            from_me: last.from_me,
                            sender: last.sender.clone(),
                            sender_name: last.sender_name.clone(),
                            summary: last.summary(),
                            full: last.content.full_summary(),
                            status: last.status,
                        },
                    )
                })
        })
        .collect();
    for (id, activity, last) in previews {
        if let Some(chat) = app.chats.iter_mut().find(|chat| chat.id == id) {
            chat.last_activity = activity;
            chat.last = Some(last);
        }
    }
    app.typing.insert(
        SAMPLES[1].id.to_owned(),
        vec![(group_members[1].0.to_owned(), std::time::Instant::now())],
    );
    app.presence.insert(
        ada.to_owned(),
        Presence {
            online: true,
            last_seen: None,
        },
    );
    app.open_chat = Some(ada.to_owned());
    // Mark the open chat as read.
    if let Some(chat) = app.chats.iter_mut().find(|chat| chat.id == ada) {
        chat.unread = 0;
    }
    // The privacy rows read like a linked account, so the sample shows them
    // filled instead of disabled.
    app.account_privacy = crate::privacy::Snapshot::demo();
    // The storage rows read like a cache in use, not an empty one.
    app.storage_stats = Some(crate::model::StorageStats {
        images: 12,
        image_bytes: 8_388_608,
        videos: 3,
        video_bytes: 25_165_824,
        stickers: 18,
        sticker_bytes: 2_097_152,
        other: 4,
        other_bytes: 524_288,
    });
    app.scroll_to_bottom = true;
    app.focus_composer = false;
}

/// A WhatsApp sticker pack shared by Ada, and with `open`, its stickers in
/// the dialog that adds it.
fn shared_pack_sample(app: &mut App, open: bool) {
    let id = SAMPLES[0].id;
    let Some(conversation) = app.conversations.get_mut(id) else {
        return;
    };
    let Some(mut row) = conversation.messages.last().cloned() else {
        return;
    };
    row.id = "ada-pack".into();
    row.from_me = false;
    row.sender = id.into();
    row.quoted = None;
    row.reactions.clear();
    row.content = crate::model::Content::StickerPack {
        name: "Ducks".into(),
        publisher: "Ada Lovelace".into(),
        count: 6,
        caption: None,
    };
    conversation.messages.push(row);
    app.scroll_to_bottom = true;
    if open {
        sticker_sample(app, crate::model::StickerShelf::Recent, "");
        app.picker = None;
        let ducks = app.sticker_packs.first().cloned();
        app.sticker_preview = ducks.map(|pack| (pack, "Ada Lovelace".to_owned()));
        app.dialog = Some(Dialog::StickerPack);
    }
}

/// Opens the sticker tab on `shelf` with stock and emoji stickers tagged the
/// way WhatsApp tags them, a Signal-style pack, and a pack made here.
fn sticker_sample(app: &mut App, shelf: crate::model::StickerShelf, search: &str) {
    let dir = app.dirs.media_cache_dir().join("demo-stickers");
    let _ = std::fs::create_dir_all(&dir);
    let make = |character: char| -> Option<std::path::PathBuf> {
        let path = dir.join(format!("{:x}.webp", character as u32));
        if let Some(sticker) = stock::sticker(character) {
            // A captioned stock sticker, tagged with its emoji as WhatsApp's are.
            let info = crate::sticker_meta::StickerInfo {
                emojis: vec![character.to_string()],
                ..Default::default()
            };
            let tagged = crate::sticker_meta::write(sticker, &info)?;
            return Some(stock::save(
                &dir,
                &format!("{:x}.webp", character as u32),
                &tagged,
            ));
        }
        if !path.exists() {
            let emoji = tour::media::emoji_image(character, 150).ok()?;
            let mut tile = image::RgbaImage::new(192, 192);
            image::imageops::overlay(&mut tile, &emoji, 21, 21);
            let mut webp = Vec::new();
            image::codecs::webp::WebPEncoder::new_lossless(&mut webp)
                .encode(&tile, 192, 192, image::ExtendedColorType::Rgba8)
                .ok()?;
            let info = crate::sticker_meta::StickerInfo {
                emojis: vec![character.to_string()],
                ..Default::default()
            };
            std::fs::write(&path, crate::sticker_meta::write(&webp, &info)?).ok()?;
        }
        Some(path)
    };
    let set = |characters: &str| -> Vec<std::path::PathBuf> {
        characters.chars().filter_map(&make).collect()
    };
    app.picker = Some(crate::model::PickerTab::Stickers);
    app.stickers = set("🤣🐸🚀🙅🥱🐶🦉🎉🔥");
    app.stickers_saved = set("❤😍🤣🐱");
    app.stickers_received = set("🐶🌮🎈🦄");
    let ducks = set("🦆🐤🐣🐥🦢🪿");
    let local = set("☕🌅🌻");
    app.sticker_packs = vec![
        crate::model::StickerPack {
            name: "Ducks".to_owned(),
            dir: app.dirs.media_cache_dir().join("Ducks"),
            stickers: ducks,
            local: false,
        },
        crate::model::StickerPack {
            name: "Bom dia".to_owned(),
            dir: app.dirs.media_cache_dir().join("Bom dia"),
            stickers: local,
            local: true,
        },
    ];
    app.sticker_emojis = app
        .stickers
        .iter()
        .chain(&app.stickers_saved)
        .chain(&app.stickers_received)
        .chain(app.sticker_packs.iter().flat_map(|pack| &pack.stickers))
        .filter_map(|path| {
            let emojis = crate::sticker_meta::emojis(&std::fs::read(path).ok()?);
            Some((path.clone(), emojis))
        })
        .collect();
    app.stickers_pending = false;
    app.sticker_shelf = shelf;
    app.sticker_search = search.to_owned();
}

/// Opens the sample group with an own reply quoting another member, beside
/// the member's reply quoting a third.
fn quote_sample(app: &mut App) {
    let group = SAMPLES[1].id;
    app.open_chat = Some(group.to_owned());
    let Some(conversation) = app.conversations.get_mut(group) else {
        return;
    };
    let quoted = conversation
        .messages
        .iter()
        .position(|row| row.id == "group-reply");
    if let Some(index) = quoted {
        let original = &conversation.messages[index];
        let (sender, sender_name) = (original.sender.clone(), original.sender_name.clone());
        // The quoted reply mentions Jonas, so the quote names him too.
        let (summary, mentions) = (original.summary(), original.mentions.clone());
        let mut reply = message(
            group,
            "quote-own",
            true,
            original.timestamp + 30,
            Content::text("See you there!"),
        );
        reply.quoted = Some(Quoted {
            labelled: None,
            chat: None,
            id: "group-reply".into(),
            sender,
            sender_name,
            summary,
            mentions,
        });
        conversation.messages.insert(index + 1, reply);
    }
}

/// A chat with Meta AI: a question and a rich reply, drawn as the text the
/// worker makes of it (#253).
fn meta_ai_sample(app: &mut App) {
    let id = "15550100000@s.whatsapp.net";
    let now = crate::util::now();
    let question = message(
        id,
        "meta-ai-question",
        true,
        now - 60,
        Content::text("How do I reverse a string in Rust?"),
    );
    let mut reply = message(
        id,
        "meta-ai-reply",
        false,
        now,
        Content::text(
            "Collect its characters in reverse order:\n\n```\nlet reversed: String = text.chars().rev().collect();\n```\n\nMethod | Handles\nchars().rev() | Unicode scalar values\ngraphemes(true).rev() | Combined emoji and accents",
        ),
    );
    reply.quoted = Some(Quoted {
        labelled: None,
        chat: None,
        id: "meta-ai-question".into(),
        sender: ME.into(),
        sender_name: None,
        summary: "How do I reverse a string in Rust?".into(),
        mentions: Vec::new(),
    });
    let mut chat = Chat::new(id.into(), "Meta AI".into());
    chat.last_activity = now;
    chat.last = Some(crate::model::LastMessage {
        labelled: reply.content.labelled(),
        from_me: false,
        sender: reply.sender.clone(),
        sender_name: None,
        summary: reply.summary(),
        full: reply.content.full_summary(),
        status: reply.status,
    });
    app.chats.insert(0, chat);
    app.conversations.entry(id.into()).or_default().messages = vec![question, reply];
    app.open_chat = Some(id.into());
}

/// A Recent shelf of animated stickers, more frames than the animation cache
/// holds at once, for the picker's paused tiles (#165).
fn animated_sticker_sample(app: &mut App) {
    sticker_sample(app, crate::model::StickerShelf::Recent, "");
    let dir = app.dirs.media_cache_dir().join("demo-stickers");
    let make = |character: char| -> Option<std::path::PathBuf> {
        let path = dir.join(format!("{:x}-bounce.webp", character as u32));
        if !path.exists() {
            let emoji = tour::media::emoji_image(character, 132).ok()?;
            let mut encoder = webp_animation::Encoder::new((192, 192)).ok()?;
            let frames = 30;
            for index in 0..frames {
                let phase = index as f32 / frames as f32 * std::f32::consts::TAU;
                let lift = (phase.sin().abs() * 28.0) as i64;
                let mut tile = image::RgbaImage::new(192, 192);
                image::imageops::overlay(&mut tile, &emoji, 30, 44 - lift);
                encoder.add_frame(&tile, index * 50).ok()?;
            }
            let webp = encoder.finalize(frames * 50).ok()?;
            std::fs::write(&path, &*webp).ok()?;
        }
        Some(path)
    };
    let animated: Vec<_> = "😂🐸🎉👋😎🚀🥳🙏🔥❤😍🤣🐱🦆🐤🐣🐥🦢☕🌅🌻💃🕺🎈🎂"
        .chars()
        .filter_map(make)
        .collect();
    app.stickers = animated.into_iter().chain(app.stickers.clone()).collect();
}

/// Applies the UI state selected by `--demo-page`.
fn interactive_sample(app: &mut App, with_image: bool) {
    use crate::model::{InteractiveButton, InteractiveCard};
    let id = SAMPLES[0].id;
    let now = crate::util::now();
    let body = if with_image {
        "A little more room for your day. 🌤️\n\nExplore the new *Cedar Mobile* plans, with more data for the things you enjoy."
    } else {
        "Hi! Our *creative workshop* starts tonight at 19:00. 🎨\n\nWe saved a few free places for this session. Choose an option below to find out more."
    };
    let labels = if with_image {
        ["View plans", "Maybe later", "Stop messages"]
    } else {
        ["Tell me more", "Send the invitation", "Stop messages"]
    };
    let mut picture = with_image.then(|| media("image/jpeg", 48_000, Some(900), Some(1200)));
    if let Some(picture) = &mut picture {
        picture.path = Some(sample_files(app).0);
    }
    let text = std::iter::once(body.to_owned())
        .chain(labels.iter().map(|label| format!("• {label}")))
        .collect::<Vec<_>>()
        .join("\n\n");
    let mut messages = vec![message(
        id,
        "interactive-card",
        false,
        now - 180,
        Content::Interactive {
            text,
            card: Some(Box::new(InteractiveCard {
                body: body.into(),
                buttons: labels
                    .iter()
                    .map(|label| InteractiveButton {
                        label: (*label).into(),
                        url: None,
                        action: crate::model::InteractiveAction::Reply,
                    })
                    .collect(),
                image: picture,
                needs_phone: false,
                ..Default::default()
            })),
        },
    )];
    let mut reply = message(
        id,
        "interactive-reply",
        true,
        now - 120,
        Content::Interactive {
            text: labels[0].into(),
            card: None,
        },
    );
    reply.quoted = Some(Quoted {
        labelled: None,
        chat: None,
        id: "interactive-card".into(),
        sender: id.into(),
        sender_name: Some("Cedar Studio".into()),
        summary: body.lines().next().unwrap().into(),
        mentions: Vec::new(),
    });
    messages.push(reply);
    messages.push(message(
        id,
        "interactive-link",
        false,
        now - 60,
        Content::Interactive {
            text: "Here are all the details.\n\n• Visit our website".into(),
            card: Some(Box::new(InteractiveCard {
                body: "Here are all the details.".into(),
                buttons: vec![InteractiveButton {
                    label: "Visit our website".into(),
                    url: Some("https://example.com/".into()),
                    action: crate::model::InteractiveAction::Unavailable,
                }],
                ..Default::default()
            })),
        },
    ));
    if let Some(chat) = app.chats.iter_mut().find(|chat| chat.id == id) {
        chat.name = "Cedar Studio".into();
        let last = messages.last().unwrap();
        chat.last_activity = last.timestamp;
        chat.last = Some(crate::model::LastMessage {
            labelled: last.content.labelled(),
            from_me: last.from_me,
            sender: last.sender.clone(),
            sender_name: None,
            summary: last.summary(),
            full: last.content.full_summary(),
            status: last.status,
        });
    }
    app.conversations.get_mut(id).unwrap().messages = messages;
    app.open_chat = Some(id.into());
    app.typing.clear();
    app.scroll_to_bottom = true;
}

/// The first chat with only its last two messages: a conversation shorter
/// than the view, which sits at its bottom.
fn short_chat_sample(app: &mut App) {
    let id = SAMPLES[0].id;
    let conversation = app.conversations.get_mut(id).unwrap();
    let keep = conversation.messages.len().saturating_sub(2);
    conversation.messages.drain(..keep);
    conversation.complete = true;
    app.open_chat = Some(id.into());
    app.typing.clear();
    app.scroll_to_bottom = true;
}

fn interactive_actions_sample(app: &mut App) {
    use crate::model::{InteractiveAction, InteractiveButton, InteractiveCard, InteractiveOption};
    interactive_sample(app, false);
    let source = &mut app.conversations.get_mut(SAMPLES[0].id).unwrap().messages[0];
    let body =
        "Your *creative workshop* is ready. 🎨\n\nChoose a session or copy your invitation code.";
    let buttons = vec![
        InteractiveButton {
            label: "Tell me more".into(),
            url: None,
            action: InteractiveAction::Reply,
        },
        InteractiveButton {
            label: "Choose a session".into(),
            url: None,
            action: InteractiveAction::Select(vec![
                InteractiveOption {
                    section: "Available sessions".into(),
                    title: "Morning ☀️".into(),
                    description: "Tuesday at 10:00. Bring your sketchbook.".into(),
                },
                InteractiveOption {
                    section: "Available sessions".into(),
                    title: "Evening 🎨".into(),
                    description: "Tuesday at 19:00. Materials included.".into(),
                },
            ]),
        },
        InteractiveButton {
            label: "Copy invitation code".into(),
            url: None,
            action: InteractiveAction::Copy("CEDAR20".into()),
        },
        InteractiveButton {
            label: "Open registration form".into(),
            url: None,
            action: InteractiveAction::Unavailable,
        },
    ];
    source.content = Content::Interactive {
        text: std::iter::once(body.to_owned())
            .chain(buttons.iter().map(|b| format!("• {}", b.label)))
            .collect::<Vec<_>>()
            .join("\n\n"),
        card: Some(Box::new(InteractiveCard {
            body: body.into(),
            buttons,
            ..Default::default()
        })),
    };
    if let Some(quote) = &mut app.conversations.get_mut(SAMPLES[0].id).unwrap().messages[1].quoted {
        quote.summary = body.lines().next().unwrap().into();
    }
}

fn interactive_list_sample(app: &mut App) {
    interactive_actions_sample(app);
    let row = &mut app.conversations.get_mut(SAMPLES[0].id).unwrap().messages[0];
    let Content::Interactive {
        text,
        card: Some(card),
    } = &mut row.content
    else {
        return;
    };
    card.body = "Creative workshops\n\nFind a session that works for you.".into();
    card.buttons = vec![crate::model::InteractiveButton {
        label: "Browse sessions".into(),
        url: None,
        action: crate::model::InteractiveAction::Select(
            [
                (
                    "Drawing",
                    "Sketchbook morning ☀️",
                    "An easy start, materials included",
                ),
                (
                    "Drawing",
                    "Evening illustration",
                    "Bring your favourite ideas",
                ),
                (
                    "Photography",
                    "A walk in the city",
                    "Discover new ways to see familiar places",
                ),
                (
                    "Photography",
                    "Studio portraits",
                    "Light, composition and a little practice",
                ),
            ]
            .into_iter()
            .map(
                |(section, title, description)| crate::model::InteractiveOption {
                    section: section.into(),
                    title: title.into(),
                    description: description.into(),
                },
            )
            .collect(),
        ),
    }];
    *text = card.body.clone();
    app.conversations
        .get_mut(SAMPLES[0].id)
        .unwrap()
        .messages
        .truncate(1);
}

fn carousel_sample(app: &mut App, count: usize) {
    interactive_sample(app, true);
    let path = sample_files(app).0;
    let cards = [
        (
            "Sketchbook set 🎨",
            "Make room for your next idea.",
            "DRAW20",
        ),
        (
            "Photo workshop 📷",
            "Find a new perspective this weekend.",
            "PHOTO15",
        ),
        ("Evening studio", "Create something together.", "STUDIO10"),
    ]
    .into_iter()
    .take(count)
    .map(|(title, body, code)| {
        let mut picture = media("image/jpeg", 48_000, Some(900), Some(1200));
        picture.path = Some(path.clone());
        crate::model::InteractiveCard {
            body: format!("*{title}*\n{body}"),
            image: Some(picture),
            buttons: vec![
                crate::model::InteractiveButton {
                    label: "Copy code".into(),
                    url: None,
                    action: crate::model::InteractiveAction::Copy(code.into()),
                },
                crate::model::InteractiveButton {
                    label: "View details".into(),
                    url: Some("https://example.com/workshops".into()),
                    action: crate::model::InteractiveAction::Unavailable,
                },
                crate::model::InteractiveButton {
                    label: "Call the studio".into(),
                    url: None,
                    action: crate::model::InteractiveAction::Unavailable,
                },
            ],
            ..Default::default()
        }
    })
    .collect();
    let c = app.conversations.get_mut(SAMPLES[0].id).unwrap();
    c.messages.truncate(1);
    c.messages[0].content = Content::Interactive {
        text: "Explore our creative sessions".into(),
        card: Some(Box::new(crate::model::InteractiveCard {
            body: "Explore our creative sessions".into(),
            carousel: cards,
            ..Default::default()
        })),
    };
}

fn poll_sample(app: &mut App, voted: bool, results: bool) {
    interactive_sample(app, false);
    let now = crate::util::now();
    let c = app.conversations.get_mut(SAMPLES[0].id).unwrap();
    c.messages.truncate(1);
    c.messages[0].id = "poll-demo".into();
    c.messages[0].content = Content::Poll {
        question: "When would you like to join the workshop?".into(),
        options: vec![
            "Morning (08:00–12:00)".into(),
            "Afternoon (13:00–17:00)".into(),
            "Evening (18:00–22:00)".into(),
        ],
        state: crate::model::PollState {
            selectable: 1,
            can_vote: true,
            history_complete: true,
            voters: if voted { 3 } else { 0 },
            counts: if voted { vec![2, 1, 0] } else { vec![0; 3] },
            selected: if voted { vec![0] } else { vec![] },
            votes: if voted {
                vec![
                    crate::model::PollVoter {
                        id: ME.into(),
                        name: "You".into(),
                        from_me: true,
                        timestamp: now - 20,
                        choices: vec![0],
                    },
                    crate::model::PollVoter {
                        id: SAMPLES[2].id.into(),
                        name: "Grace Hopper".into(),
                        from_me: false,
                        timestamp: now - 90,
                        choices: vec![0],
                    },
                    crate::model::PollVoter {
                        id: SAMPLES[4].id.into(),
                        name: "Katherine Johnson".into(),
                        from_me: false,
                        timestamp: now - 120,
                        choices: vec![1],
                    },
                ]
            } else {
                vec![]
            },
            ..Default::default()
        },
    };
    if results {
        app.dialog = Some(Dialog::PollResults {
            chat: SAMPLES[0].id.into(),
            message: "poll-demo".into(),
        });
    }
}

/// A bigger group's audience for one of our messages: some members read it,
/// some only have it, and the rest have not received it yet.
fn sample_recipients(now: i64) -> Vec<crate::model::Recipient> {
    let recipient = |id: &str, delivered: Option<i64>, read: Option<i64>| crate::model::Recipient {
        id: id.into(),
        expected: true,
        delivered_at: delivered.map(|minutes| now - minutes * 60),
        read_at: read.map(|minutes| now - minutes * 60),
        played_at: None,
    };
    vec![
        recipient("491701111111@s.whatsapp.net", Some(24), Some(3)),
        recipient("491702222222@s.whatsapp.net", Some(24), Some(11)),
        recipient(SAMPLES[2].id, Some(23), Some(19)),
        recipient("491703333333@s.whatsapp.net", Some(22), None),
        recipient(SAMPLES[4].id, Some(9), None),
        recipient("12025550137@s.whatsapp.net", Some(20), None),
        recipient("491704444444@s.whatsapp.net", None, None),
        recipient("491705555555@s.whatsapp.net", None, None),
        recipient("491706666666@s.whatsapp.net", None, None),
    ]
}

/// "Message info" for our message in a bigger group: some members read it,
/// some only have it, and the rest have not received it yet. Without a saved
/// audience, the dialog explains that earlier receipts are unknown.
fn message_info_sample(app: &mut App, recorded: bool) {
    let chat = SAMPLES[1].id;
    let now = crate::util::now();
    app.open_chat = Some(chat.into());
    app.typing.clear();
    let c = app.conversations.get_mut(chat).unwrap();
    let message = c
        .messages
        .iter_mut()
        .rev()
        .find(|m| m.from_me && matches!(m.content, Content::Text { .. }))
        .unwrap();
    message.status = crate::model::Delivery::Delivered;
    let id = message.id.clone();
    app.message_receipts = Some(crate::model::MessageReceipts {
        chat: chat.into(),
        message: id.clone(),
        recipients: if recorded {
            sample_recipients(now)
        } else {
            Vec::new()
        },
    });
    app.receipts_watch = Some((chat.into(), id.clone()));
    app.dialog = Some(Dialog::MessageInfo {
        chat: chat.into(),
        message: id,
    });
}

/// Pictures with and without captions, forwarded or not, from both sides,
/// so the forwarded label and the time over a picture can be checked.
fn photos_sample(app: &mut App) {
    let id = SAMPLES[0].id;
    let now = crate::util::now();
    let dir = app.dirs.media_cache_dir();
    let _ = std::fs::create_dir_all(&dir);
    // Light towards the top; the time sits over the dark of its lower corner.
    let wide = stock::save_photo(&dir, stock::DUSK);
    let photo = |caption: Option<&str>| {
        let mut media = media(
            "image/jpeg",
            stock::DUSK.bytes.len() as u64,
            Some(stock::DUSK.width),
            Some(stock::DUSK.height),
        );
        media.path = Some(wide.clone());
        Content::Image {
            motion: None,
            caption: caption.map(str::to_owned),
            media,
        }
    };
    let mut rows = vec![
        message(
            id,
            "photos-caption",
            true,
            0,
            photo(Some("The launch, from the causeway")),
        ),
        message(id, "photos-in", false, 0, photo(None)),
        message(id, "photos-out", true, 0, photo(None)),
        message(
            id,
            "photos-forwarded-in",
            false,
            0,
            Content::text("Minutes from Tuesday, as promised"),
        ),
        message(
            id,
            "photos-forwarded-out",
            true,
            0,
            Content::text("Passing this along"),
        ),
    ];
    for (index, row) in rows.iter_mut().enumerate() {
        row.timestamp = now - 600 + index as i64 * 100;
        row.forwarded = matches!(
            row.id.as_str(),
            "photos-forwarded-in" | "photos-forwarded-out" | "photos-in"
        );
    }
    app.conversations.entry(id.into()).or_default().messages = rows;
    app.open_chat = Some(id.into());
    app.scroll_to_bottom = true;
}

/// Runs of bubbles from both sides, for the Messages style's balloons and
/// their tails (`bubbles,messages` and `bubbles,messages-light`): single
/// lines, a wrapped paragraph, a message of emoji alone, a quoted reply on
/// our own blue balloon, and a lone photo, which leaves the tail to the text
/// after it. Only each run's last balloon curls a tail. Every row fits
/// a 1280 by 800 window, so all four runs show at once.
fn bubbles_sample(app: &mut App) {
    let id = SAMPLES[0].id;
    let now = crate::util::now();
    let dir = app.dirs.media_cache_dir();
    let _ = std::fs::create_dir_all(&dir);
    let wide = stock::save_photo(&dir, stock::DUSK);
    let photo = || {
        let mut media = media(
            "image/jpeg",
            stock::DUSK.bytes.len() as u64,
            Some(stock::DUSK.width),
            Some(stock::DUSK.height),
        );
        media.path = Some(wide.clone());
        Content::Image {
            motion: None,
            caption: None,
            media,
        }
    };
    let text = Content::text;
    let mut rows = vec![
        message(
            id,
            "bubbles-in-1",
            false,
            0,
            text("Are you around this afternoon?"),
        ),
        message(
            id,
            "bubbles-in-2",
            false,
            0,
            text(
                "The ferry leaves at four, so if we meet at the pier by half past three \
                 there is time for a coffee first and nobody has to run for it.",
            ),
        ),
        message(id, "bubbles-out-1", true, 0, text("Yes!")),
        message(id, "bubbles-out-2", true, 0, text("🎉🙌")),
        message(
            id,
            "bubbles-out-3",
            true,
            0,
            text("Half past three works for me"),
        ),
        message(id, "bubbles-in-3", false, 0, photo()),
        message(
            id,
            "bubbles-in-4",
            false,
            0,
            text("The view from the pier yesterday"),
        ),
        message(id, "bubbles-out-4", true, 0, text("Then coffee is on me")),
    ];
    rows[4].quoted = Some(crate::model::Quoted {
        labelled: None,
        id: "bubbles-in-2".into(),
        sender: id.into(),
        sender_name: None,
        summary: "The ferry leaves at four".into(),
        mentions: Vec::new(),
        chat: None,
    });
    for (index, row) in rows.iter_mut().enumerate() {
        row.timestamp = now - 1200 + index as i64 * 60;
    }
    let conversation = app.conversations.entry(id.into()).or_default();
    conversation.messages = rows;
    conversation.complete = true;
    app.open_chat = Some(id.into());
    app.typing.clear();
    app.scroll_to_bottom = true;
}

/// A motion photo with its clip downloaded. `playing` plays it in the bubble.
fn motion_sample(app: &mut App, playing: bool) {
    let id = SAMPLES[0].id;
    let dir = app.dirs.media_cache_dir();
    let mut media = media(
        "image/jpeg",
        stock::LAUNCH.bytes.len() as u64,
        Some(stock::LAUNCH.width),
        Some(stock::LAUNCH.height),
    );
    media.path = Some(stock::save_photo(&dir, stock::LAUNCH));
    let photo = Content::Image {
        caption: Some("Liftoff, as it happened".into()),
        media,
        motion: Some(crate::model::Motion {
            path: Some(stock::save(&dir, "demo-video.mp4", stock::VIDEO)),
            ..Default::default()
        }),
    };
    app.conversations.entry(id.into()).or_default().messages = vec![message(
        id,
        "demo-motion",
        false,
        crate::util::now() - 60,
        photo,
    )];
    app.open_chat = Some(id.into());
    app.motion_playing = playing.then(|| (id.into(), "demo-motion".into()));
}

/// Replaces the first chat with videos: a downloaded one, a round video
/// message of our own, and one still on WhatsApp's servers. `play` starts
/// one of them.
fn video_sample(app: &mut App, play: Option<&str>) {
    let id = SAMPLES[0].id;
    let now = crate::util::now();
    let dir = app.dirs.media_cache_dir();
    let path = stock::save(&dir, "demo-video.mp4", stock::VIDEO);
    let round = stock::save(&dir, "demo-note.mp4", stock::NOTE);
    let clip = |note: bool, downloaded: bool| {
        let (bytes, side, file) = if note {
            (stock::NOTE, (360, 360), &round)
        } else {
            (stock::VIDEO, (640, 360), &path)
        };
        let mut media = media("video/mp4", bytes.len() as u64, Some(side.0), Some(side.1));
        if downloaded {
            media.path = Some(file.clone());
        }
        Content::Video {
            caption: None,
            media,
            seconds: Some(stock::CLIP_SECONDS),
            gif: false,
            note,
        }
    };
    let mut rows = vec![
        message(id, "demo-note-remote", false, 0, clip(true, false)),
        message(id, "demo-video", false, 0, clip(false, true)),
        message(id, "demo-note", true, 0, clip(true, true)),
    ];
    // The one that plays comes last, so it is on screen.
    if let Some(index) = play.and_then(|play| rows.iter().position(|row| row.id == play)) {
        let playing = rows.remove(index);
        rows.push(playing);
    }
    for (index, row) in rows.iter_mut().enumerate() {
        row.thumbnail = Some(stock::thumbnail(stock::LAUNCH));
        row.timestamp = now - 300 + index as i64 * 100;
    }
    app.conversations.entry(id.into()).or_default().messages = rows;
    app.open_chat = Some(id.into());
    // Screenshots stay quiet.
    app.video.silence();
    if let Some(message) = play {
        app.actions.push(crate::model::Action::PlayVideo {
            message: message.into(),
            path: if message == "demo-note" { round } else { path },
        });
    }
}

/// Replaces the first chat with a synthetic four-picture WhatsApp album.
fn media_album_sample(app: &mut App, grouped: bool) {
    let chat = SAMPLES[0].id;
    let now = crate::util::now();
    // Keep the ordinary album fixture inside today's local date, including at midnight.
    let start = crate::util::day_key(now)
        .and_then(crate::util::day_bounds)
        .map_or(now - 30, |(start, _)| start);
    let at = (now - 30).max(start);
    let rows = (0..4)
        .map(|index| {
            let mut attachment =
                media("image/jpeg", 180_000 + index * 10_000, Some(900), Some(700));
            attachment.album = grouped.then(|| "demo-album".into());
            let mut row = message(
                chat,
                &format!("album-{index}"),
                false,
                at + index as i64,
                Content::Image {
                    motion: None,
                    caption: (index == 0).then(|| "Weekend references, all in one place".into()),
                    media: attachment,
                },
            );
            row.thumbnail = Some(sample_thumbnail(index as u32 + 10));
            row
        })
        .collect();
    app.conversations.entry(chat.into()).or_default().messages = rows;
    app.open_chat = Some(chat.into());
    app.scroll_to_bottom = true;
}

/// The search pane over the sample chat with the most matches for
/// `query`, listing them newest first as the archive would.
fn chat_search_sample(app: &mut App, query: &str) {
    let matches = |conversation: &crate::app::Conversation| -> Vec<crate::model::Message> {
        conversation
            .messages
            .iter()
            .rev()
            .filter(|message| message.text_matching(query).is_some())
            .cloned()
            .collect()
    };
    let Some((chat, hits)) = app
        .conversations
        .iter()
        .map(|(chat, conversation)| (chat.clone(), matches(conversation)))
        .max_by_key(|(chat, hits)| (hits.len(), std::cmp::Reverse(chat.clone())))
    else {
        return;
    };
    app.open_chat = Some(chat);
    app.chat_search_open = true;
    app.chat_search = query.into();
    app.chat_search_hits = hits;
}

/// Opens the info panel on `chat`, listed from the sample's own messages:
/// the demo has no worker to answer for the archive.
fn info_sample(app: &mut App, chat: &str, view: InfoView) {
    let rows = app
        .conversations
        .get(chat)
        .map(|conversation| conversation.messages.clone())
        .unwrap_or_default();
    app.info = Some(InfoPanel {
        chat: chat.to_owned(),
        view,
    });
    app.set_info_media(ChatMedia::collect(
        chat.to_owned(),
        rows,
        crate::model::MEDIA_LIST_LIMIT,
    ));
}

/// Three local labels worn by some of the sample chats.
fn labels_sample(app: &mut App) {
    let label = |id: &str, name: &str, color_hex: &str, created_at| crate::model::Label {
        id: id.to_owned(),
        name: name.to_owned(),
        color_hex: color_hex.to_owned(),
        created_at,
    };
    app.labels = vec![
        label("label-work", "Work", "#3b82f6", 1),
        label("label-family", "Family", "#22c55e", 2),
        label("label-follow-up", "Follow up", "#f97316", 3),
    ];
    let worn: [(usize, &[&str]); 5] = [
        (0, &["label-work", "label-follow-up"]),
        (1, &["label-work"]),
        (2, &["label-follow-up"]),
        (4, &["label-family"]),
        (6, &["label-work"]),
    ];
    for (index, labels) in worn {
        if let Some(chat) = app
            .chats
            .iter_mut()
            .find(|chat| chat.id == SAMPLES[index].id)
        {
            chat.labels = labels.iter().map(|id| (*id).to_owned()).collect();
        }
    }
}

/// A synthetic dusk gradient as the wallpaper image, shown at once. The file
/// is never written: the sample hands the decoded image over directly.
/// Sets the demo password ("demo-password") and locks.
fn app_lock_sample(app: &mut App) {
    app.settings.app_lock_hash = Some(crate::app_lock::verifier("demo-password"));
    app.lock_app();
}

fn wallpaper_image_sample(app: &mut App) {
    let (width, height) = (1600usize, 1000usize);
    let pixels = (0..height)
        .flat_map(|y| {
            (0..width).map(move |x| {
                let across = x as f32 / width as f32;
                let down = y as f32 / height as f32;
                let sun =
                    (1.0 - ((across - 0.7).powi(2) + (down - 0.35).powi(2)).sqrt() * 3.0).max(0.0);
                egui::Color32::from_rgb(
                    (40.0 + 150.0 * down + 60.0 * sun) as u8,
                    (50.0 + 60.0 * down + 50.0 * sun) as u8,
                    (110.0 - 40.0 * down + 20.0 * sun) as u8,
                )
            })
        })
        .collect();
    let image = egui::ColorImage::new([width, height], pixels);
    let path = app.dirs.wallpaper_file("png");
    app.wallpaper_image.show_now(&path, image);
    app.account_mut().settings.wallpaper_image = Some(path);
}

/// Sample stars, and the left panel that lists them. `menu` keeps one row's
/// own context menu open, so a screenshot and a layout test can see it.
fn starred_sample(app: &mut App, menu: Option<&str>) {
    let chat = SAMPLES[0].id.to_owned();
    let starred = app.conversations[&chat]
        .messages
        .iter()
        .find(|row| row.id == "ada-link")
        .cloned()
        .expect("the sample link message");
    app.stars
        .insert(chat.clone(), [starred.id.clone()].into_iter().collect());
    app.starred = vec![crate::archive::Starred {
        message: starred,
        starred_at: crate::util::now(),
    }];
    app.show_starred = true;
    app.open_list_menu = menu.map(|id| (chat, id.to_owned()));
}

/// Configures synthetic offline demo states and appearance for tests and screenshots.
/// Audio fixtures vary voice-note and forwarding metadata independently without playback.
pub fn apply_flags(app: &mut App, page: Option<&str>) {
    let Some(page) = page else {
        return;
    };
    for part in page.split(',').map(str::trim) {
        match part {
            "history-date" => history_date_sample(app),
            "chat" | "" => {}
            "phone-menu" => phone_menu_sample(app),
            "short-chat" => short_chat_sample(app),
            "chat-menu" => app.open_chat_menu = Some(app.chats[0].id.clone()),
            "chat-header-menu" => app.open_header_menu = app.open_chat.clone(),
            "interactive-actions" => interactive_actions_sample(app),
            "interactive-list" => interactive_list_sample(app),
            "interactive-list-dialog" => {
                interactive_list_sample(app);
                app.dialog = Some(Dialog::InteractiveList {
                    chat: SAMPLES[0].id.into(),
                    message: "interactive-card".into(),
                    button: 0,
                });
            }
            "carousel" => carousel_sample(app, 3),
            "carousel-pair" => carousel_sample(app, 2),
            // A carousel card's picture in the viewer, among the chat's album.
            "carousel-viewer" => {
                carousel_sample(app, 3);
                let chat = SAMPLES[0].id.to_owned();
                let card = app.conversations[&chat].messages[0].clone();
                app.viewer_media = crate::model::MediaItem::of_message(&card);
                app.viewer_media_chat = Some(chat.clone());
                let path = app.viewer_media.get(1).and_then(|item| item.path.clone());
                app.image_preview = Some(
                    crate::image_preview::PreviewState::new(path, chat, card.id).with_card(Some(1)),
                );
            }
            "poll-empty" => poll_sample(app, false, false),
            "poll-voted" => poll_sample(app, true, false),
            "poll-results" => poll_sample(app, true, true),
            "photos" => photos_sample(app),
            "bubbles" => bubbles_sample(app),
            "drafts" => {
                // Unsent text in two chats besides the open one, whose
                // draft is in the composer.
                app.drafts
                    .insert(SAMPLES[3].id.into(), "Bring the spare HDMI adapter".into());
                app.drafts.insert(
                    SAMPLES[2].id.into(),
                    "Sounds good, see you at\nthe station".into(),
                );
                app.composer = "Still typing this one".into();
            }
            "motion" => motion_sample(app, false),
            "motion-playing" => motion_sample(app, true),
            "motion-preview" => {
                motion_sample(app, false);
                app.actions.push(crate::model::Action::PreviewImage {
                    path: app.dirs.media_cache_dir().join(stock::LAUNCH.name),
                    chat: SAMPLES[0].id.into(),
                    message: "demo-motion".into(),
                    card: None,
                });
            }
            // The motion photo's clip playing over the photo in the viewer.
            "motion-viewer" => {
                motion_sample(app, false);
                app.actions.push(crate::model::Action::ExpandVideo {
                    message: "demo-motion".into(),
                    path: app.dirs.media_cache_dir().join("demo-video.mp4"),
                });
            }
            "video" => video_sample(app, None),
            "video-playing" => video_sample(app, Some("demo-video")),
            "video-expanded" => {
                video_sample(app, None);
                app.actions.push(crate::model::Action::ExpandVideo {
                    message: "demo-video".into(),
                    path: app.dirs.media_cache_dir().join("demo-video.mp4"),
                });
            }
            "media-album" => media_album_sample(app, true),
            "media-album-failed" => {
                media_album_sample(app, true);
                let messages = &mut app.conversations.get_mut(SAMPLES[0].id).unwrap().messages;
                for message in &mut *messages {
                    message.from_me = true;
                    message.status = Delivery::Sent;
                }
                messages[1].status = Delivery::Failed;
            }
            "media-album-partial" => {
                media_album_sample(app, true);
                let (saved, _) = sample_files(app);
                let messages = &mut app.conversations.get_mut(SAMPLES[0].id).unwrap().messages;
                messages[3].content.media_at_mut(None).unwrap().path = Some(saved);
                messages[2].quoted = Some(crate::model::Quoted {
                    labelled: None,
                    id: "synthetic-quoted-message".into(),
                    sender: SAMPLES[0].id.into(),
                    sender_name: Some("Alex".into()),
                    summary: "A reference for the album".into(),
                    mentions: Vec::new(),
                    chat: None,
                });
            }
            "media-album-oversized" => {
                media_album_sample(app, true);
                let chat = SAMPLES[0].id;
                let messages = &mut app.conversations.get_mut(chat).unwrap().messages;
                messages[1].content.media_at_mut(None).unwrap().size =
                    crate::model::ATTACHMENT_DOWNLOAD_LIMIT + 1;
                let downloads: Vec<_> = messages
                    .iter()
                    .map(|message| crate::model::Action::Download {
                        // As automatic downloads would: the page shows the
                        // limit's failure on the oversized member.
                        manual: false,
                        card: None,
                        chat: chat.into(),
                        message: message.id.clone(),
                    })
                    .collect();
                app.actions.extend(downloads);
            }
            "media-album-midnight" => {
                media_album_sample(app, true);
                let midnight = crate::util::day_key(crate::util::now())
                    .and_then(crate::util::day_bounds)
                    .unwrap()
                    .0;
                for (message, offset) in app
                    .conversations
                    .get_mut(SAMPLES[0].id)
                    .unwrap()
                    .messages
                    .iter_mut()
                    .zip([-2, -1, 1, 2])
                {
                    message.timestamp = midnight + offset;
                }
            }
            "media-album-before" => media_album_sample(app, false),
            "shared-contact" => {
                let chat = SAMPLES[0].id;
                let now = crate::util::now();
                app.conversations
                    .entry(chat.into())
                    .or_default()
                    .messages
                    .push(message(
                        chat,
                        "shared-contact",
                        false,
                        now,
                        Content::Contact {
                            display_name: "Contact from sender".into(),
                            vcard: "BEGIN:VCARD\nVERSION:3.0\nFN:Jordan Rivera\nTEL;TYPE=CELL;waid=15550002222:+1 555-000-2222\nEND:VCARD".into(),
                        },
                    ));
                app.open_chat = Some(chat.into());
            }
            "note-playing" => video_sample(app, Some("demo-note")),
            "interactive" | "interactive-media" => {
                interactive_sample(app, part == "interactive-media")
            }
            "empty" => app.open_chat = None,
            "channel" => {
                let id = "fixture@newsletter";
                let mut chat = Chat::new(id.into(), "Demo announcements".into());
                chat.last_activity = crate::util::now();
                app.chats.insert(0, chat);
                app.conversations.entry(id.into()).or_default().messages = vec![message(
                    id,
                    "channel-fixture",
                    false,
                    crate::util::now(),
                    Content::text("A synthetic announcement from a read-only channel."),
                )];
                app.open_chat = Some(id.into());
            }
            "meta-ai" => meta_ai_sample(app),
            "locked" => {
                app.chats[0].locked = true;
                app.open_chat = None;
            }
            "locked-prompt" => {
                app.chats[0].locked = true;
                app.settings.set_chat_lock_code(Some("demo-code"));
                app.dialog = Some(crate::model::Dialog::UnlockLockedChats);
                app.open_chat = None;
            }
            "locked-setup" => app.dialog = Some(crate::model::Dialog::UnlockLockedChats),
            // The app lock's screen, over the sample chats it hides.
            "app-lock" => app_lock_sample(app),
            "app-lock-wrong" => {
                app_lock_sample(app);
                // Enough wrong tries for an eight-second wait.
                for _ in 0..6 {
                    app.app_lock.unlocked(false);
                }
            }
            "app-lock-forgot" => {
                app_lock_sample(app);
                app.app_lock.forgetting = crate::app_lock::Forgetting::Confirming;
            }
            // Settings with a password set and the form to change it open.
            "app-lock-settings" => {
                app.settings.app_lock_hash = Some(crate::app_lock::verifier("demo-password"));
                app.page = Page::Settings;
                app.settings_search = "app lock".into();
                app.app_lock.form = Some(crate::app_lock::Form::new(
                    crate::app_lock::FormMode::Change,
                ));
            }
            "app-lock-setup" => {
                app.page = Page::Settings;
                app.settings_search = "app lock".into();
                let mut form = crate::app_lock::Form::new(crate::app_lock::FormMode::Set);
                form.new = "short".into();
                form.error = Some(crate::app_lock::FormError::TooShort);
                app.app_lock.form = Some(form);
            }
            "new-chat" => app.dialog = Some(crate::model::Dialog::NewChat),
            // New group: two people picked and a third being looked for,
            // then the group being named.
            "new-group" | "new-group-name" | "new-group-creating" => {
                app.dialog = Some(crate::model::Dialog::NewChat);
                // Opened from the chat list, with no chat behind the dialog.
                app.open_chat = None;
                let mut draft = crate::new_group::Draft {
                    members: vec![SAMPLES[0].id.to_owned(), SAMPLES[3].id.to_owned()],
                    ..Default::default()
                };
                if part == "new-group" {
                    app.new_chat_search = "gr".into();
                } else {
                    draft.stage = crate::new_group::Stage::Naming;
                    draft.name = "Weekend in Lisbon".into();
                    draft.creating = part == "new-group-creating";
                }
                app.new_group = Some(draft);
            }
            "unnamed-group" => {
                app.typing.clear();
                let mut ids = Vec::new();
                for (index, name) in [
                    "Andrea North",
                    "Andrea South",
                    "Andrea West",
                    "Giacomo East",
                ]
                .iter()
                .enumerate()
                {
                    let id = format!("1555000000{index}@s.whatsapp.net");
                    app.contacts.insert(
                        id.clone(),
                        Contact {
                            id: id.clone(),
                            full_name: Some((*name).to_owned()),
                            first_name: None,
                            push_name: None,
                        },
                    );
                    ids.push(id);
                }
                ids.push(ME.to_owned());
                if let Some(chat) = app.chats.iter_mut().find(|chat| chat.is_group()) {
                    chat.name = "Group".to_owned();
                    chat.group_subject_known = false;
                    chat.participants = ids;
                    app.open_chat = Some(chat.id.clone());
                }
            }
            "locked-open" => {
                app.chats[0].locked = true;
                app.settings.set_chat_lock_code(Some("demo-code"));
                app.open_chat = None;
                app.actions
                    .push(crate::model::Action::UnlockLockedFolder("demo-code".into()));
                app.actions
                    .push(crate::model::Action::OpenChat(app.chats[0].id.clone()));
            }
            "keyring" => {
                unlink(app);
                app.link = LinkStatus::Failed("The archive is encrypted but its OS keyring key is missing. Restore the original keyring; the archive has not been changed".into());
            }
            "receipts-off" => {
                app.account_receipts_off = true;
            }
            "receipts-played" => {
                let conversation = app.conversations.get_mut(SAMPLES[0].id).unwrap();
                let message = conversation
                    .messages
                    .iter_mut()
                    .find(|message| {
                        message.from_me && matches!(message.content, Content::Audio { .. })
                    })
                    .unwrap();
                message.status = crate::model::Delivery::Played;
            }
            "message-info" => message_info_sample(app, true),
            "message-info-unknown" => message_info_sample(app, false),
            "message-info-partial" => {
                message_info_sample(app, true);
                // One reader, from receipts kept before the audience was.
                if let Some(receipts) = &mut app.message_receipts {
                    receipts.recipients.truncate(1);
                    receipts.recipients[0].expected = false;
                }
            }
            "message-info-direct" => {
                let chat = SAMPLES[0].id;
                let c = app.conversations.get_mut(chat).unwrap();
                let message = c
                    .messages
                    .iter_mut()
                    .rev()
                    .find(|m| m.from_me && matches!(m.content, Content::Text { .. }))
                    .unwrap();
                message.status = crate::model::Delivery::Read;
                message.delivered_at = Some(message.timestamp + 4);
                message.read_at = Some(message.timestamp + 3 * 60);
                app.dialog = Some(Dialog::MessageInfo {
                    chat: chat.into(),
                    message: message.id.clone(),
                });
            }
            "disappearing" => {
                let chat = app
                    .chats
                    .iter_mut()
                    .find(|chat| chat.id == SAMPLES[1].id)
                    .unwrap();
                chat.ephemeral_expiration = Some(86_400);
                app.open_chat = Some(chat.id.clone());
                app.typing.clear();
                app.scroll_to_bottom = true;
            }
            "disappearing-direct" => {
                // A one-to-one chat with its own duration, so the timer shows
                // in the contact details as well as the group's.
                let chat = app
                    .chats
                    .iter_mut()
                    .find(|chat| chat.id == SAMPLES[0].id)
                    .unwrap();
                chat.ephemeral_expiration = Some(604_800);
                app.open_chat = Some(chat.id.clone());
                app.typing.clear();
                app.scroll_to_bottom = true;
            }
            "disappearing-off" => {
                // A chat whose timer the phone turned off: the details say so
                // rather than staying quiet about it.
                let chat = app
                    .chats
                    .iter_mut()
                    .find(|chat| chat.id == SAMPLES[2].id)
                    .unwrap();
                chat.ephemeral_expiration = Some(0);
                app.open_chat = Some(chat.id.clone());
                app.typing.clear();
                app.scroll_to_bottom = true;
            }
            "arabic-reply" => {
                let chat = SAMPLES[0].id;
                let now = crate::util::now();
                let original = message(
                    chat,
                    "arabic-original",
                    false,
                    now - 120,
                    Content::text("مساء الخير"),
                );
                let mut messages = vec![original];
                for (id, own) in [("arabic-incoming", false), ("arabic-outgoing", true)] {
                    let mut reply =
                        message(chat, id, own, now - 60, Content::text("Reply preview test"));
                    reply.quoted = Some(Quoted {
                        labelled: None,
                        chat: None,
                        id: "arabic-original".into(),
                        sender: chat.into(),
                        sender_name: Some("Demo contact".into()),
                        summary: "مساء الخير".into(),
                        mentions: Vec::new(),
                    });
                    messages.push(reply);
                }
                app.conversations.get_mut(chat).unwrap().messages = messages;
                app.open_chat = Some(chat.into());
                app.reply_to = Some("arabic-original".into());
                app.typing.clear();
                app.scroll_to_bottom = true;
            }
            "rtl" => {
                let id = SAMPLES[1].id;
                let now = crate::util::now();
                let messages = vec![
                    message(
                        id,
                        "rtl-hebrew",
                        false,
                        now - 120,
                        Content::text("הכלב הגדול קפץ 🐕"),
                    ),
                    message(
                        id,
                        "rtl-arabic",
                        false,
                        now - 60,
                        Content::text("مرحبا بالعالم الجميل 🌍"),
                    ),
                    {
                        let mut reply =
                            message(id, "rtl-reply", true, now, Content::text("שלום עולם"));
                        reply.quoted = Some(Quoted {
                            labelled: None,
                            chat: None,
                            id: "rtl-hebrew".into(),
                            sender: SAMPLES[0].id.into(),
                            sender_name: Some("שלום עולם".into()),
                            summary: "הכלב הגדול קפץ 🐕".into(),
                            mentions: Vec::new(),
                        });
                        reply
                    },
                    // Numbers keep their left-to-right order inside
                    // right-to-left text, and alone (#184).
                    message(id, "rtl-numbers", false, now, Content::text(RTL_NUMBERS)),
                    message(id, "rtl-digits", true, now, Content::text("٤٥")),
                    {
                        let text = Content::text("עולה 3.14 ש״ח");
                        let mut reply = message(id, "rtl-digits-reply", false, now, text);
                        reply.quoted = Some(Quoted {
                            labelled: None,
                            chat: None,
                            id: "rtl-digits".into(),
                            sender: ME.into(),
                            sender_name: None,
                            summary: "٤٥".into(),
                            mentions: Vec::new(),
                        });
                        reply
                    },
                ];
                if let Some(chat) = app.chats.iter_mut().find(|chat| chat.id == id) {
                    chat.name = "שלום יזמות ונדל\"ן".into();
                    if let Some(last) = &mut chat.last {
                        last.summary = "٤٥".into();
                    }
                }
                app.conversations.get_mut(id).expect("demo group").messages = messages;
                app.composer = "١٢:٣٠".into();
                app.open_chat = Some(id.into());
                app.typing.clear();
                app.scroll_to_bottom = true;
            }
            "rtl-self" => {
                let now = crate::util::now();
                let count = RTL_SELF_CHAT.len() as i64;
                let messages: Vec<Message> = RTL_SELF_CHAT
                    .iter()
                    .enumerate()
                    .map(|(index, text)| {
                        let timestamp = now - (count - index as i64) * 60 * 5;
                        let id = format!("rtl-self-{index}");
                        message(ME, &id, true, timestamp, Content::text(*text))
                    })
                    .collect();
                let mut chat = Chat::new(ME.into(), "You".into());
                chat.last_activity = now;
                chat.last = messages.last().map(|last| crate::model::LastMessage {
                    labelled: last.content.labelled(),
                    from_me: true,
                    sender: last.sender.clone(),
                    sender_name: None,
                    summary: last.summary(),
                    full: last.content.full_summary(),
                    status: last.status,
                });
                app.chats.insert(0, chat);
                app.conversations.insert(
                    ME.into(),
                    Conversation {
                        messages,
                        complete: true,
                        requested: true,
                        phone_exhausted: true,
                        ..Default::default()
                    },
                );
                app.open_chat = Some(ME.into());
                app.typing.clear();
                app.scroll_to_bottom = true;
            }
            "settings" => app.page = Page::Settings,
            choice if choice.starts_with("settings-search=") => {
                app.page = Page::Settings;
                app.settings_search = choice["settings-search=".len()..].to_owned();
            }
            "wallpaper" => app.page = Page::Wallpaper,
            // The call surface itself: the incoming call waiting to be answered, and a call that
            // has been up for a few minutes.
            "call" => call_sample(app, true, false),
            "call-active" => call_sample(app, false, false),
            // The other states of a call: dialing, muted with the device
            // pickers open and a headset gone, its farewell, and the strip a
            // call keeps while the reader is in a chat.
            "call-outgoing" => {
                call_sample(app, false, false);
                if let Some(call) = app.call.as_mut() {
                    call.phase = CallPhase::Ringing;
                    call.started = None;
                }
            }
            // A call whose button was just pressed: on screen at once, before
            // its offer has gone out.
            "call-connecting" => {
                call_sample(app, false, false);
                if let Some(call) = app.call.as_mut() {
                    call.phase = CallPhase::Placing;
                    call.started = None;
                }
            }
            // A call that could not be placed, saying so before it goes.
            "call-failed" => {
                call_sample(app, false, false);
                if let Some(call) = app.call.as_mut() {
                    call.phase = CallPhase::Failed;
                    call.started = None;
                    call.outcome = Some(crate::calls::CallOutcome::Failed);
                }
            }
            "call-devices" => {
                call_sample(app, false, false);
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
                app.call_devices_open = true;
                if let Some(call) = app.call.as_mut() {
                    call.muted = true;
                    call.speaker = Some("desk-speakers".into());
                    call.lost_devices = vec![crate::calls::LostDevice {
                        kind: crate::calls::DeviceKind::Microphone,
                        name: "Wireless Headset".into(),
                    }];
                }
            }
            "call-ended" => {
                call_sample(app, false, false);
                if let Some(call) = app.call.as_mut() {
                    call.phase = CallPhase::Ended;
                    call.outcome = Some(crate::calls::CallOutcome::Answered);
                }
            }
            // A group's call: ringing with who started it and who is on it, and
            // up with a tile for each member, one of them only asked to it.
            "call-group" => group_call_sample(app, true, false),
            "call-group-active" => group_call_sample(app, false, false),
            // A group's video call: three members' cameras on, Tom's off, and
            // ours in the corner.
            "call-group-video" => group_call_sample(app, false, true),
            // A call we started in the group: nobody has joined yet, so
            // everyone on it reads Invited while it rings.
            "call-group-calling" => {
                group_call_sample(app, false, false);
                if let Some(call) = app.call.as_mut() {
                    call.direction = crate::model::CallDirection::Outgoing;
                    call.phase = CallPhase::Dialing;
                    call.started = None;
                    if let Some(group) = call.group.as_mut() {
                        group.caller = None;
                        for person in &mut group.participants {
                            person.joined = false;
                        }
                    }
                }
            }
            // While group calls are off (`calls::GROUP_CALLS_ENABLED`), a group
            // call to us does not take the screen: the window says who is
            // calling the group and to answer on the phone.
            "call-group-on-phone" => {
                group_call_sample(app, true, false);
                if let Some(update) = app.call.take() {
                    app.call_account = None;
                    app.handle_phone_only_call(update);
                }
            }
            // The same call with our microphone muted: our own tile says so.
            "call-group-muted" => {
                group_call_sample(app, false, false);
                if let Some(call) = app.call.as_mut() {
                    call.muted = true;
                }
            }
            // A call link from a message, while the call service lets us in.
            "call-link" => {
                call_sample(app, false, false);
                if let Some(call) = app.call.as_mut() {
                    call.chat = String::new();
                    call.phase = CallPhase::Connecting;
                    call.started = None;
                    call.group = Some(crate::calls::GroupCall {
                        link: true,
                        ..crate::calls::GroupCall::default()
                    });
                }
            }
            "call-strip" => {
                call_sample(app, false, false);
                app.call_surface_hidden = true;
            }
            "call-entries" => {
                // Calls among the open chat's messages, as WhatsApp files them.
                let open = app.open_chat.clone().unwrap_or_default();
                let entries: Vec<Message> = call_entries(crate::util::now())
                    .into_iter()
                    .filter(|entry| entry.chat == open)
                    .collect();
                if let Some(conversation) = app.conversations.get_mut(&open) {
                    conversation.messages.extend(entries);
                    conversation
                        .messages
                        .sort_by_key(|message| message.timestamp);
                }
                app.scroll_to_bottom = true;
            }
            // Every kind of call entry in a group: answered with a length,
            // unanswered, missed and declined, voice and video, ours and
            // members' (whose names head their bubbles).
            "call-bubbles" => call_bubbles_sample(app),
            // The Calls tab: with no chat open, its start pane; with the
            // chat that was open, that chat beside the history; with a row
            // picked, that row's chat at the call.
            // The Missed chip picked, and with no missed call to show.
            "calls" | "calls-chat" | "calls-info" | "new-call" | "calls-missed"
            | "calls-missed-none" | "calls-link" => {
                app.page = Page::Calls;
                if part != "calls-chat" {
                    app.open_chat = None;
                }
                let mut log = call_entries(crate::util::now());
                if part == "calls-link" {
                    // A video call joined through a link half an hour ago,
                    // which no chat holds.
                    log.push(message(
                        crate::model::CALL_LINK_CHAT,
                        &format!("{}link-1", crate::model::CALL_ENTRY_PREFIX),
                        true,
                        crate::util::now() - 30,
                        Content::Call {
                            video: true,
                            status: crate::model::CallStatus::Answered,
                            duration: 1_260,
                            group: true,
                        },
                    ));
                }
                if part.starts_with("calls-missed") {
                    app.call_filter = crate::model::CallFilter::Missed;
                }
                if part == "calls-missed-none" {
                    log.retain(|message| {
                        !crate::ui::calls::Entry::of(message).is_some_and(|entry| entry.missed())
                    });
                }
                log.sort_by_key(|message| std::cmp::Reverse(message.timestamp));
                if part == "calls-info" {
                    // The newest row, Ada's two missed calls, picked: her
                    // chat opens at the call, filed among its messages.
                    let (chat, id) = (log[0].chat.clone(), log[0].id.clone());
                    if let Some(conversation) = app.conversations.get_mut(&chat) {
                        conversation
                            .messages
                            .extend(log.iter().filter(|entry| entry.chat == chat).cloned());
                        conversation
                            .messages
                            .sort_by_key(|message| message.timestamp);
                    }
                    app.actions
                        .push(crate::model::Action::PickCall { chat, message: id });
                }
                app.new_call = part == "new-call";
                app.call_log = log;
                app.call_log_loaded = true;
            }
            "call-video" => call_sample(app, false, true),
            "call-video-incoming" => call_sample(app, true, true),
            "call-video-off" => {
                call_sample(app, false, true);
                if let Some(call) = app.call.as_mut() {
                    call.remote_video = false;
                    call.camera_on = false;
                }
                app.call_remote_frame = None;
                app.call_local_frame = None;
            }
            "wallpaper-image" => wallpaper_image_sample(app),
            "omarchy" | "omarchy-light" => {
                let mut themes: Vec<_> = crate::theme::presets().collect();
                let filename = if part == "omarchy-light" {
                    "Catppuccin Latte.json"
                } else {
                    "Catppuccin.json"
                };
                let mut system = themes
                    .iter()
                    .find(|t| t.filename == filename)
                    .unwrap()
                    .clone();
                system.filename = "omarchy.json".into();
                app.settings.theme = crate::settings::ThemeChoice::System;
                app.settings.custom_theme = None;
                app.settings.system_theme_cache = Some(system.clone());
                themes.push(system);
                app.custom_themes = crate::theme::Catalog::preview(themes, true);
            }
            choice if choice.starts_with("theme=") => {
                let themes: Vec<_> = crate::theme::presets().collect();
                if let Some(theme) = themes
                    .iter()
                    .find(|theme| Some(theme.filename.as_str()) == choice.strip_prefix("theme="))
                {
                    app.settings.custom_theme = Some(theme.filename.clone());
                    app.settings.custom_theme_cache = Some(theme.clone());
                }
                app.custom_themes = crate::theme::Catalog::preview(themes, false);
            }
            "themes" => {
                use crate::theme::{Catalog, CustomTheme};
                let mut palette = crate::theme::Palette::dark();
                palette.accent = egui::Color32::from_rgb(137, 180, 250);
                palette.bubble_out = egui::Color32::from_rgb(41, 57, 84);
                let theme = CustomTheme {
                    filename: "Moonlight 🌙.json".into(),
                    palette,
                };
                let mut themes: Vec<_> = crate::theme::presets().collect();
                themes.push(theme.clone());
                app.custom_themes = Catalog::preview(themes, false);
                app.settings.custom_theme = Some(theme.filename.clone());
                app.settings.custom_theme_cache = Some(theme);
                app.page = Page::Settings;
            }
            "update" | "update-downloading" | "update-ready" | "update-failed"
            | "update-managed" => {
                use crate::updates::{DownloadState, Installation, Kind, Prepared};
                app.update = Some(crate::updates::Release {
                    version: "99.0.0".to_owned(),
                    url: "https://github.com/crmne/zapfast/releases/latest".to_owned(),
                });
                app.show_update = true;
                let installation = Installation {
                    executable: "/demo/zapfast".into(),
                    kind: Kind::Portable,
                };
                app.update_support = Some(Ok(installation.clone()));
                app.update_download = match part {
                    "update-downloading" => DownloadState::Downloading {
                        received: 8_000_000,
                        total: 20_000_000,
                    },
                    "update-ready" => {
                        DownloadState::Ready(Box::new(Prepared::sample(installation, "99.0.0")))
                    }
                    "update-failed" => DownloadState::Failed(
                        "The download could not be verified. Try downloading it again.".into(),
                    ),
                    _ => DownloadState::Idle,
                };
                if part == "update-managed" {
                    app.update_support = Some(Err(
                        "Update this installation through your package manager or software center."
                            .into(),
                    ));
                }
            }
            "poll" => {
                app.open_chat = Some(SAMPLES[1].id.into());
                app.scroll_to_bottom = true;
                app.typing.clear();
            }
            "poll-create" => {
                app.dialog = app.open_chat.clone().map(Dialog::CreatePoll);
                app.poll_draft = crate::model::PollDraft {
                    question: "Pizza after the talks? 🍕".into(),
                    options: vec!["Yes".into(), "Only if it’s Neapolitan".into(), "No".into()],
                    multiple: false,
                };
            }
            // The location dialog, with a spot pasted in.
            "location" => {
                app.dialog = app.open_chat.clone().map(Dialog::SendLocation);
                app.location_draft = "-23.5505, -46.6333".into();
            }
            "shortcuts" => app.dialog = Some(Dialog::Shortcuts),
            "about" => app.dialog = Some(Dialog::About),
            "failed" => {
                // The newest outgoing message in the open chat failed to send.
                if let Some(message) = app
                    .open_chat
                    .clone()
                    .and_then(|chat| app.conversations.get_mut(&chat))
                    .and_then(|conversation| {
                        conversation
                            .messages
                            .iter_mut()
                            .rev()
                            .find(|message| message.from_me)
                    })
                {
                    message.status = crate::model::Delivery::Failed;
                }
            }
            "info" | "info-media" | "info-docs" | "info-links" => {
                let view = match part {
                    "info-media" => InfoView::Media(MediaTab::Media),
                    "info-docs" => InfoView::Media(MediaTab::Docs),
                    "info-links" => InfoView::Media(MediaTab::Links),
                    _ => InfoView::Overview,
                };
                if let Some(chat) = app.open_chat.clone() {
                    info_sample(app, &chat, view);
                }
            }
            "group-admin"
            | "group-admin-menu"
            | "group-permissions"
            | "group-add-members"
            | "group-reset-link"
            | "group-remove-member" => {
                // Family, which we run: its invite link, its permissions, a
                // member's menu, and the dialogs its actions open.
                let group = FAMILY;
                app.open_chat = Some(group.to_owned());
                let view = if part == "group-permissions" {
                    InfoView::Permissions
                } else {
                    InfoView::Overview
                };
                info_sample(app, group, view);
                app.info_invite = Some((
                    group.to_owned(),
                    crate::group_admin::LinkState::Known(
                        crate::group_admin::InviteLink::new(DEMO_INVITE_LINK.to_owned())
                            .expect("a WhatsApp link"),
                    ),
                ));
                match part {
                    "group-admin-menu" => app.open_member_menu = Some(JONAS.to_owned()),
                    "group-add-members" => {
                        let existing = app
                            .chat(group)
                            .map(|chat| chat.participants.clone())
                            .unwrap_or_default();
                        let mut draft = crate::new_group::Draft::adding(group.to_owned(), existing);
                        draft.toggle(DOROTHY);
                        app.new_group = Some(draft);
                        app.dialog = Some(Dialog::AddGroupMembers(group.to_owned()));
                    }
                    "group-reset-link" => {
                        app.dialog = Some(Dialog::ConfirmResetInviteLink(group.to_owned()))
                    }
                    "group-remove-member" => {
                        app.dialog = Some(Dialog::ConfirmRemoveMember {
                            chat: group.to_owned(),
                            member: JONAS.to_owned(),
                        })
                    }
                    _ => {}
                }
            }
            "group-info" | "group-info-rename" | "group-info-locked" | "group-info-saving" => {
                // A group whose name and photo we may change, the same with its
                // name being typed or its change on the way, and one locked for us.
                let group = if part == "group-info-locked" {
                    "120363011122233344@g.us"
                } else {
                    SAMPLES[1].id
                };
                app.open_chat = Some(group.to_owned());
                info_sample(app, group, InfoView::Overview);
                if part == "group-info-rename" {
                    app.group_name_edit = Some("Rust Berlin 🦀".to_owned());
                }
                if part == "group-info-saving" {
                    app.group_saving.insert(group.to_owned());
                }
            }
            "forward" | "forward-selected" | "forward-review" | "forward-full" | "forward-once" => {
                app.dialog = app.open_chat.clone().map(|chat| Dialog::Forward {
                    chat,
                    messages: vec!["ada-format".to_owned()],
                });
                app.forward_limit = crate::model::FORWARD_LIMIT;
                match part {
                    "forward-selected" | "forward-review" => {
                        app.forward_recipients =
                            vec![SAMPLES[0].id.to_owned(), SAMPLES[1].id.to_owned()];
                        app.forward_reviewing = part == "forward-review";
                    }
                    // As many recipients as WhatsApp allows, with the note
                    // saying so.
                    "forward-full" => {
                        let writable: Vec<_> = app
                            .chats
                            .iter()
                            .filter(|chat| crate::ui::dialogs::forwardable(chat))
                            .map(|chat| chat.id.clone())
                            .collect();
                        app.forward_recipients = writable
                            .into_iter()
                            .take(crate::model::FORWARD_LIMIT)
                            .collect();
                    }
                    // A message forwarded many times, which goes to one chat.
                    "forward-once" => {
                        app.forward_limit = 1;
                        app.forward_recipients = vec![SAMPLES[0].id.to_owned()];
                    }
                    _ => {}
                }
            }
            "unlink" => app.dialog = Some(Dialog::ConfirmUnlink),
            "clear-media" => {
                app.dialog = Some(Dialog::ConfirmClearMediaCache(app.account().id.clone()));
            }
            "leave-group" => {
                let group = SAMPLES[1].id.to_owned();
                app.open_chat = Some(group.clone());
                app.dialog = Some(Dialog::ConfirmLeaveGroup(group));
            }
            "left-group" => {
                // The chat after the phone confirmed the leave.
                let group = SAMPLES[1].id.to_owned();
                let ours: Vec<String> = app.our_ids().into_iter().map(str::to_owned).collect();
                if let Some(chat) = app.chats.iter_mut().find(|chat| chat.id == group) {
                    chat.left = true;
                    chat.read_only = true;
                    chat.participants.retain(|id| !ours.contains(id));
                }
                app.open_chat = Some(group);
            }
            "leave-channel" => {
                let channel = SAMPLES
                    .iter()
                    .find(|sample| sample.id.ends_with("@newsletter"))
                    .expect("channel sample")
                    .id
                    .to_owned();
                app.open_chat = Some(channel.clone());
                app.dialog = Some(Dialog::ConfirmLeaveGroup(channel));
            }
            "toasts" => {
                app.toast("History loaded");
                app.toast_error(
                    "Could not open the attachment: No application knows how to open \"Notes on the Engine.pdf\" (error -10814)",
                );
            }
            "delete-chat" => {
                app.dialog = app.open_chat.clone().map(Dialog::ConfirmDeleteChat);
            }
            "clear-chat" => {
                app.dialog = app.open_chat.clone().map(Dialog::ConfirmClearChat);
            }
            "select" => {
                if let Some(chat) = app.open_chat.clone() {
                    let ids: Vec<String> = app
                        .conversations
                        .get(&chat)
                        .map(|conversation| {
                            conversation
                                .messages
                                .iter()
                                .rev()
                                .take(3)
                                .step_by(2)
                                .map(|message| message.id.clone())
                                .rev()
                                .collect()
                        })
                        .unwrap_or_default();
                    app.selection = Some((chat, ids));
                }
            }
            "quotes" => {
                quote_sample(app);
                app.scroll_to_bottom = false;
                app.at_bottom = false;
                app.scroll_anchor = Some("quote-own".into());
            }
            "quote-jump" => {
                // A clicked quote has scrolled back to the message it quotes,
                // which flashes.
                quote_sample(app);
                let group = SAMPLES[1].id;
                let target = format!("{group}-3");
                app.scroll_to_bottom = false;
                app.at_bottom = false;
                app.scroll_anchor = Some(target.clone());
                app.jump_highlight = Some(crate::app::JumpHighlight::new(group.to_owned(), target));
            }
            "unread-divider" => {
                let id = SAMPLES[1].id.to_owned();
                app.open_chat = Some(id.clone());
                app.unread_divider = Some(crate::app::UnreadDivider {
                    chat: id,
                    count: 3,
                    placed: false,
                });
                app.scroll_to_bottom = true;
            }
            "invite" => {
                app.invite = Some(crate::model::GroupInvite {
                    code: "DemoInviteCode123".into(),
                    state: crate::model::InviteState::Ready(crate::model::InviteInfo {
                        id: "120363000000000000@g.us".into(),
                        subject: "Analytical Engine Club 🛠️".into(),
                        description: Some(
                            "Notes, diagrams and bad puns about difference engines.".into(),
                        ),
                        members: 42,
                        approval: true,
                    }),
                });
                app.dialog = Some(Dialog::JoinGroup);
            }
            "delete-message" => {
                app.dialog = app
                    .open_chat
                    .clone()
                    .map(|chat| Dialog::ConfirmDeleteMessage {
                        chat,
                        message: "ada-emoji".to_owned(),
                        for_everyone: true,
                    });
            }
            "remove-pack" => {
                apply_flags(app, Some("stickers"));
                app.dialog = Some(Dialog::ConfirmRemoveStickerPack {
                    name: "Happy Frogs".to_owned(),
                    dir: std::path::PathBuf::from("Happy Frogs"),
                });
            }
            "delete-message-mine" => {
                app.dialog = app
                    .open_chat
                    .clone()
                    .map(|chat| Dialog::ConfirmDeleteMessage {
                        chat,
                        message: "ada-format".to_owned(),
                        for_everyone: false,
                    });
            }
            "select-delete" => {
                if let Some(chat) = app.open_chat.clone() {
                    app.selection =
                        Some((chat, vec!["ada-format".to_owned(), "ada-reply".to_owned()]));
                    app.open_selection_delete_menu = true;
                }
            }
            "delete-selection" => {
                if let Some(chat) = app.open_chat.clone() {
                    let messages = vec!["ada-doc".to_owned(), "ada-reply".to_owned()];
                    app.selection = Some((chat.clone(), messages.clone()));
                    app.dialog = Some(Dialog::ConfirmDeleteSelection {
                        chat,
                        messages,
                        for_everyone: false,
                    });
                }
            }
            "new-contact" => app.dialog = Some(Dialog::NewContact),
            // A formatted number with no name: the dialog offers Message.
            "new-contact-number" => {
                app.dialog = Some(Dialog::NewContact);
                app.new_contact_phone = "+1 (555) 010-4477".to_owned();
            }
            // The switcher under our avatar open with the one account.
            "account-menu" => app.account_menu = true,
            // A second number linked beside the first, with unread chats of
            // its own, and the switcher under our avatar open; with
            // `accounts-muted` that number's notifications are off.
            "accounts" | "accounts-closed" | "accounts-muted" => {
                if app.accounts.len() < 2 {
                    let id = crate::model::AccountId::parse("2").expect("demo account");
                    if let Ok((mut work, _)) = crate::account::Account::detached(
                        &app.dirs,
                        id,
                        crate::settings::AccountSettings::default(),
                    ) {
                        work.me = Some("15550002222@s.whatsapp.net".into());
                        work.me_name = Some("Carmine (Studio)".into());
                        work.link = crate::backend::LinkStatus::Connected;
                        for (id, name, unread) in [
                            ("15550003333@s.whatsapp.net", "Grace", 2),
                            ("120363000000000099@g.us", "Studio team", 5),
                        ] {
                            let mut chat = Chat::new(id.into(), name.into());
                            chat.unread = unread;
                            chat.last_activity = crate::util::now();
                            work.chats.push(chat);
                        }
                        app.accounts.push(work);
                    }
                }
                if part == "accounts-muted"
                    && let Some(work) = app.accounts.get_mut(1)
                {
                    work.settings.notifications = false;
                }
                app.account_menu = part != "accounts-closed";
            }
            // One pinned message: its line under the chat header and its mark
            // on the bubble.
            "pinned" => pin_sample(app),
            "light" => {
                app.settings.theme = ThemeChoice::Light;
            }
            "messages" => {
                app.settings.theme = ThemeChoice::Messages;
            }
            // Messages follows the system; this shows its light side anywhere.
            "messages-light" => {
                app.settings.custom_theme = Some("Messages light.json".into());
                app.settings.custom_theme_cache = Some(crate::theme::CustomTheme {
                    filename: "Messages light.json".into(),
                    palette: crate::theme::Palette::messages_light(),
                });
            }
            "login" => {
                unlink(app);
                app.link = LinkStatus::Unlinked {
                    qr: Some(sample_qr()),
                    pair_code: None,
                    pairing_phone: None,
                };
            }
            "pair" => {
                unlink(app);
                app.link = LinkStatus::Unlinked {
                    qr: None,
                    pair_code: Some("FWAP1234".into()),
                    pairing_phone: Some("15550001111".into()),
                };
            }
            "phone" => {
                unlink(app);
                app.link = LinkStatus::Unlinked {
                    qr: Some(sample_qr()),
                    pair_code: None,
                    pairing_phone: None,
                };
                app.dialog = Some(Dialog::PairWithPhone);
            }
            "offline" => {
                app.link = LinkStatus::Disconnected {
                    reason: "stream ended".into(),
                };
            }
            "syncing" => app.syncing = true,
            // Ada shares where she is now.
            "live" => {
                let ada = SAMPLES[0].id;
                let now = crate::util::now();
                let mut row = message(
                    ada,
                    "ada-live-now",
                    false,
                    now - 60 * 12,
                    Content::LiveLocation {
                        latitude: 51.5226,
                        longitude: -0.1571,
                        accuracy_m: Some(12),
                        speed_mps: Some(1.4),
                        heading_deg: Some(90),
                        sequence: 14,
                        ended: false,
                        updated: now - 60,
                        // The phone has posted newer positions this device
                        // cannot read, so the card says where they are.
                        newer_on_phone: true,
                    },
                );
                row.thumbnail = Some(sample_map());
                // Then a plain location of our own, without a preview.
                let pinned = message(
                    ada,
                    "ada-location-now",
                    true,
                    now - 60 * 2,
                    Content::Location {
                        latitude: 51.5226,
                        longitude: -0.1571,
                        name: None,
                        address: None,
                    },
                );
                if let Some(conversation) = app.conversations.get_mut(ada) {
                    conversation.messages.push(row);
                    conversation.messages.push(pinned);
                }
                app.open_chat = Some(ada.to_owned());
                app.scroll_to_bottom = true;
            }
            // Our phone shares a live location in our own chat, masked from
            // linked devices as WhatsApp does.
            "live-phone" => {
                let now = crate::util::now();
                let messages = vec![
                    message(
                        ME,
                        "self-note",
                        true,
                        now - 60 * 5,
                        Content::text("Parking"),
                    ),
                    message(
                        ME,
                        "self-live",
                        true,
                        now - 60 * 2,
                        Content::PhoneOnly {
                            view_once: false,
                            live_location: true,
                            once: None,
                        },
                    ),
                ];
                let mut chat = Chat::new(ME.into(), "You".into());
                chat.last_activity = now;
                chat.last = messages.last().map(|last| crate::model::LastMessage {
                    labelled: last.content.labelled(),
                    from_me: true,
                    sender: last.sender.clone(),
                    sender_name: None,
                    summary: last.summary(),
                    full: last.content.full_summary(),
                    status: last.status,
                });
                app.chats.insert(0, chat);
                app.conversations.insert(
                    ME.into(),
                    Conversation {
                        messages,
                        complete: true,
                        requested: true,
                        phone_exhausted: true,
                        ..Default::default()
                    },
                );
                app.open_chat = Some(ME.into());
                app.typing.clear();
                app.scroll_to_bottom = true;
            }
            "typing" => {
                app.composer = (1..=9)
                    .map(|line| format!("line {line}"))
                    .collect::<Vec<_>>()
                    .join("\n");
                app.focus_composer = true;
            }
            "composer-tools" => {
                app.open_chat = Some(SAMPLES[0].id.to_owned());
                app.composer_tools_open = true;
                app.focus_composer = true;
            }
            "mention" => {
                let group = SAMPLES[1].id;
                app.open_chat = Some(group.to_owned());
                app.composer = "@mi".to_owned();
                app.mention_start = Some(0);
                app.mention_selected = 0;
                app.focus_composer = true;
            }
            "mention-everyone" => {
                let group = SAMPLES[1].id;
                app.open_chat = Some(group.to_owned());
                app.composer = "@".to_owned();
                app.mention_start = Some(0);
                app.mention_selected = 0;
                app.focus_composer = true;
            }
            "emoji-complete" => {
                app.composer = "hello :gri".to_owned();
                app.emoji_start = Some(6);
                app.emoji_selected = 0;
                app.focus_composer = true;
            }
            // The strips above the composer: a reply, an edit, and a voice
            // message WhatsApp refused.
            "reply" => {
                app.open_chat = Some(SAMPLES[0].id.to_owned());
                app.reply_to = Some("ada-photo".into());
                app.focus_composer = true;
            }
            "private-reply" | "private-reply-menu" | "private-reply-locked-source" => {
                let group = SAMPLES[1].id;
                app.open_chat = Some(group.into());
                if part == "private-reply-menu" {
                    app.open_message_menu = message_menu(app, "group-photo");
                    app.scroll_to_bottom = false;
                    app.scroll_anchor = Some("group-photo".into());
                } else if part == "private-reply-locked-source" {
                    let recipient = app.conversations[group]
                        .message("group-photo")
                        .unwrap()
                        .sender
                        .clone();
                    if !app.chats.iter().any(|chat| chat.id == recipient) {
                        app.chats.push(Chat::new(
                            recipient.clone(),
                            "Private recipient fixture".into(),
                        ));
                    }
                    app.conversations.entry(recipient.clone()).or_default();
                    app.open_chat = Some(recipient);
                    app.reply_to = Some(crate::model::ReplyTarget {
                        id: "group-photo".into(),
                        chat: Some(group.into()),
                    });
                    app.chats
                        .iter_mut()
                        .find(|chat| chat.id == group)
                        .unwrap()
                        .locked = true;
                } else {
                    app.actions.push(crate::model::Action::ReplyPrivately {
                        chat: group.into(),
                        message: "group-photo".into(),
                    });
                }
            }
            "edit" => {
                let chat = SAMPLES[0].id;
                app.open_chat = Some(chat.to_owned());
                let own =
                    app.conversations.get(chat).and_then(|conversation| {
                        conversation.messages.iter().rev().find_map(|message| {
                            match &message.content {
                                Content::Text { text, .. } if message.from_me => {
                                    Some((message.id.clone(), text.clone()))
                                }
                                _ => None,
                            }
                        })
                    });
                if let Some((id, text)) = own {
                    app.composer = text;
                    app.editing = Some(id);
                }
                app.focus_composer = true;
            }
            // Reading back through the first chat, away from its end: the
            // button back to the newest message shows.
            "scrolled" => {
                let chat = SAMPLES[0].id;
                app.open_chat = Some(chat.to_owned());
                app.scroll_to_bottom = false;
                app.at_bottom = false;
                app.scroll_anchor = app
                    .conversations
                    .get(chat)
                    .and_then(|conversation| conversation.messages.get(4))
                    .map(|message| message.id.clone());
            }
            // The group photo with its reactions, and the day above it.
            "reactions" => {
                app.open_chat = Some(SAMPLES[1].id.to_owned());
                app.scroll_to_bottom = false;
                app.at_bottom = false;
                app.scroll_anchor = Some("group-photo".into());
            }
            "unsent-voice" => {
                let chat = SAMPLES[0].id;
                app.open_chat = Some(chat.to_owned());
                app.unsent_voice =
                    Some((chat.to_owned(), vec![0.0; crate::voice::RATE as usize * 6]));
            }
            // Show two simultaneous group typers.
            "typers" => {
                let group = SAMPLES[1].id;
                app.open_chat = Some(group.to_owned());
                if let Some(chat) = app.chats.iter_mut().find(|chat| chat.id == group) {
                    chat.unread = 0;
                }
                app.typing.insert(
                    group.to_owned(),
                    vec![
                        (
                            "491702222222@s.whatsapp.net".to_owned(),
                            std::time::Instant::now(),
                        ),
                        (
                            "491703333333@s.whatsapp.net".to_owned(),
                            std::time::Instant::now(),
                        ),
                    ],
                );
            }
            // The chat list collapsed to avatars with unread badges.
            "nosidebar" | "rail" => app.sidebar_visible = false,
            // A list wide enough for the whole chip row, which scrolls out of
            // sight at the default width.
            "wide" => app.settings.sidebar_width = 560.0,
            "search" => {
                app.search = "do".into();
                let mut hits = Vec::new();
                for (chat, id) in [
                    ("120363012345678901@g.us", "group-reply"),
                    ("120363012345678901@g.us", "group-photo"),
                    ("14155550199@s.whatsapp.net", "14155550199@s.whatsapp.net-1"),
                ] {
                    if let Some(message) = app
                        .conversations
                        .get(chat)
                        .and_then(|conversation| conversation.message(id))
                    {
                        hits.push(message.clone());
                    }
                }
                hits.sort_by_key(|message| std::cmp::Reverse(message.timestamp));
                app.search_hits = hits;
            }
            "audio-types" => {
                let chat = app.open_chat.clone().unwrap_or_default();
                if let Some(conversation) = app.conversations.get_mut(&chat) {
                    let source = conversation
                        .messages
                        .iter()
                        .find(|message| message.id == "ada-voice")
                        .cloned();
                    if let Some(source) = source {
                        conversation.messages = [
                            ("audio-original", true, false, 8),
                            ("audio-file", false, false, 52),
                            ("audio-forwarded", false, true, 52),
                        ]
                        .into_iter()
                        .enumerate()
                        .map(|(index, (id, voice_note, forwarded, seconds))| {
                            let mut row = source.clone();
                            row.id = id.into();
                            row.timestamp += index as i64;
                            row.forwarded = forwarded;
                            if let Content::Audio {
                                seconds: duration,
                                voice_note: note,
                                ..
                            } = &mut row.content
                            {
                                *duration = Some(seconds);
                                *note = voice_note;
                            }
                            row
                        })
                        .collect();
                    }
                }
            }
            "voice" => {
                // Use a valid clip for playback tests.
                let tone: Vec<f32> = (0..crate::voice::RATE * 6)
                    .map(|i| {
                        let t = i as f32 / crate::voice::RATE as f32;
                        (t * 220.0 * std::f32::consts::TAU).sin() * 0.4 * (t * 1.3).sin().abs()
                    })
                    .collect();
                let path = app.dirs.media_cache_dir().join("demo-voice.ogg");
                if let Ok(bytes) = crate::voice::encode(&tone) {
                    let _ = std::fs::create_dir_all(path.parent().expect("a directory"));
                    let _ = std::fs::write(&path, bytes);
                }
                let waveform = crate::voice::waveform(&tone);
                let open = app.open_chat.clone().unwrap_or_default();
                for id in ["ada-voice", "you-voice"] {
                    if let Some(message) = app
                        .conversations
                        .get_mut(&open)
                        .and_then(|conversation| conversation.message_mut(id))
                        && let crate::model::Content::Audio {
                            media,
                            waveform: bars,
                            seconds,
                            ..
                        } = &mut message.content
                    {
                        media.path = Some(path.clone());
                        *bars = waveform.clone();
                        *seconds = Some(6);
                    }
                }
            }
            "recording" => app.recording = Some(crate::audio::Recorder::rehearsal()),
            // Shows the native image preview over the demo chat.
            "preview" => {
                // Three photos with a text between the second and the third,
                // the viewer open on the second over the chat's album.
                let chat = app.open_chat.clone().expect("demo chat");
                let photos = [stock::ENGINE, stock::LAUNCH, stock::SPACEWALK];
                let now = crate::util::now();
                let mut rows = photos
                    .iter()
                    .enumerate()
                    .map(|(index, photo)| {
                        let mut attachment = media(
                            "image/jpeg",
                            photo.bytes.len() as u64,
                            Some(photo.width),
                            Some(photo.height),
                        );
                        attachment.path =
                            Some(stock::save_photo(&app.dirs.media_cache_dir(), *photo));
                        message(
                            &chat,
                            &format!("preview-{index}"),
                            false,
                            now + index as i64 * 2,
                            Content::Image {
                                motion: None,
                                caption: None,
                                media: attachment,
                            },
                        )
                    })
                    .collect::<Vec<_>>();
                rows.insert(
                    2,
                    message(
                        &chat,
                        "preview-text",
                        false,
                        now + 3,
                        Content::text("One more photo below."),
                    ),
                );
                let selected = rows[1].content.media().unwrap().path.clone();
                app.viewer_media = rows
                    .iter()
                    .flat_map(crate::model::MediaItem::of_message)
                    .collect();
                app.conversations.get_mut(&chat).unwrap().messages = rows;
                app.viewer_media_chat = Some(chat.clone());
                app.image_preview = Some(crate::image_preview::PreviewState::new(
                    selected,
                    chat,
                    "preview-1".to_owned(),
                ));
            }
            "compose-emoji" => {
                app.composer = "Andiamo 😊 con due 👍🏽 e poi testo normale".to_owned();
            }
            "attachment-draft-away" | "attachment-draft-return" => {
                apply_flags(app, Some("staged"));
                app.actions
                    .push(crate::model::Action::OpenChat(SAMPLES[1].id.into()));
                if part == "attachment-draft-return" {
                    app.actions
                        .push(crate::model::Action::OpenChat(SAMPLES[0].id.into()));
                }
            }
            "forward-portuguese" => {
                apply_flags(app, Some("forward"));
                app.locale = crate::i18n::Locale::PortugueseBrazil;
            }
            "view-once" | "view-once-off" => {
                let (photo, _) = sample_files(app);
                app.stage(crate::app::Pending::File(photo));
                app.pending_view_once = part == "view-once";
                app.composer = "One moment, just for you.".into();
            }
            "staged" => {
                let (photo, _) = sample_files(app);
                let side = 48usize;
                let rgba: Vec<u8> = (0..side * side)
                    .flat_map(|index| {
                        let x = (index % side) as u8;
                        let y = (index / side) as u8;
                        [x * 5, 120, 255 - y * 5, 255]
                    })
                    .collect();
                app.stage(crate::app::Pending::Picture {
                    width: side,
                    height: side,
                    rgba: std::sync::Arc::new(rgba),
                    texture: None,
                });
                app.stage(crate::app::Pending::File(photo.clone()));
                // The same picture staged to go as a document shows as a file.
                app.stage(crate::app::Pending::Document(photo));
                app.stage(crate::app::Pending::File("/tmp/notes.pdf".into()));
                app.composer = "Look at these".into();
            }
            // Opens the cropper on a sample picture. The demo has no worker
            // to read a picture's size, so it is read here instead.
            "crop" => {
                let (photo, _) = sample_files(app);
                let (width, height) = image::image_dimensions(&photo).unwrap_or((4, 3));
                // The photo is named by the strip, so another page having
                // staged pictures already cannot point this at the wrong one.
                let target = app.stage(crate::app::Pending::File(photo.clone()));
                app.picture_edit = Some(crate::model::PictureEdit::new(
                    target,
                    crate::model::PictureSource::File(photo),
                    width,
                    height,
                ));
            }
            "archived" => app.show_archived = true,
            "labels" | "label-chips" => labels_sample(app),
            "label-filter" => {
                labels_sample(app);
                app.label_filter = Some("label-work".into());
            }
            "labels-dialog" => {
                labels_sample(app);
                app.dialog = Some(Dialog::Labels);
            }
            "chat-search" => chat_search_sample(app, "engine"),
            // Right-to-left previews with emoji.
            "chat-search-rtl" => chat_search_sample(app, "שלום"),
            // The same pane with the day filter open.
            "chat-search-day" => {
                chat_search_sample(app, "engine");
                app.chat_search_day = app
                    .chat_search_hits
                    .first()
                    .and_then(|hit| crate::util::day_key(hit.timestamp));
                if let Some(day) = app.chat_search_day {
                    app.chat_search_month = day;
                    app.chat_search_hits
                        .retain(|hit| crate::util::day_key(hit.timestamp) == Some(day));
                }
                app.chat_search_calendar = true;
            }
            "unread" => app.chat_filter = crate::model::ChatFilter::Unread,
            "private" => app.chat_filter = crate::model::ChatFilter::Private,
            "favorites" => app.chat_filter = crate::model::ChatFilter::Favorites,
            "groups" => app.chat_filter = crate::model::ChatFilter::Groups,
            "picker" => app.picker = Some(crate::model::PickerTab::Emoji),
            "stickers" => sticker_sample(app, crate::model::StickerShelf::Recent, ""),
            "sticker-favorites" => sticker_sample(app, crate::model::StickerShelf::Favorites, ""),
            "sticker-received" => sticker_sample(app, crate::model::StickerShelf::Received, ""),
            "sticker-pack" => {
                let pack =
                    crate::model::StickerShelf::Pack(app.dirs.media_cache_dir().join("Ducks"));
                sticker_sample(app, pack, "")
            }
            "sticker-search" => sticker_sample(app, crate::model::StickerShelf::Recent, "laugh"),
            "sticker-animated" => animated_sticker_sample(app),
            "sticker-add" => sticker_sample(app, crate::model::StickerShelf::Add, ""),
            // A received sticker shown large, not yet a favorite.
            "sticker-view" => {
                sticker_sample(app, crate::model::StickerShelf::Recent, "");
                app.picker = None;
                app.sticker_view = app
                    .stickers_received
                    .first()
                    .map(|path| crate::image_preview::StickerPreview::new(path.clone()));
            }
            "sticker-maker" => {
                let (photo, _) = sample_files(app);
                let crop = crate::model::StickerCrop::centered(900, 1200).resized(620, 900, 1200);
                app.sticker_draft = Some(crate::model::StickerDraft {
                    source: photo,
                    width: 900,
                    height: 1200,
                    transparent: false,
                    crop: crop.moved(0, 130, 900, 1200),
                    keep_transparent: false,
                    emojis: "🌅 🌊".into(),
                });
                app.dialog = Some(Dialog::StickerMaker);
            }
            "sticker-pack-message" | "sticker-pack-view" => {
                shared_pack_sample(app, part == "sticker-pack-view")
            }
            "gifs" => {
                app.picker = Some(crate::model::PickerTab::Gifs);
                app.settings.giphy_key = "demo".into();
                app.gif_results = (0..6)
                    .map(|index| crate::model::Gif {
                        id: format!("demo{index}"),
                        still: Some(sample_files(app).0),
                        mp4: String::new(),
                        width: 200,
                        height: if index % 2 == 0 { 150 } else { 200 },
                    })
                    .collect();
            }
            // Show the rejected GIPHY key state.
            "gifs-badkey" => {
                app.picker = Some(crate::model::PickerTab::Gifs);
                app.settings.giphy_key = "demo".into();
                app.gif_error = Some(crate::model::GifError {
                    message: "GIPHY rejected the API key (error 401).".into(),
                    bad_key: true,
                });
            }
            // With "voice": the menu of a playable voice message, which
            // lists every playback speed.
            "voice-menu" => app.open_message_menu = message_menu(app, "ada-voice"),
            "react-menu" => {
                app.open_message_menu = message_menu(app, "ada-link");
                if let Some(row) = app
                    .conversations
                    .get_mut(SAMPLES[0].id)
                    .and_then(|conversation| conversation.message_mut("ada-link"))
                {
                    row.delivered_at = Some(row.timestamp);
                    row.read_at = Some(row.timestamp + 60 * 60 * 7);
                }
            }
            "starred" => starred_sample(app, None),
            "star-menu" => starred_sample(app, Some("ada-link")),
            "react-picker" => {
                let chat = SAMPLES[0].id.to_owned();
                app.reaction_target = Some((chat, "ada-link".into()));
                app.reaction_beside_menu = true;
                app.scroll_to_bottom = false;
                app.scroll_anchor = Some("ada-link".into());
                app.picker_focus = true;
                app.settings.recent_emoji = vec![
                    "👍".into(),
                    "❤️".into(),
                    "😂".into(),
                    "🦀".into(),
                    "🎉".into(),
                    "🔥".into(),
                ];
                app.settings.reaction_emoji = app
                    .settings
                    .recent_emoji
                    .iter()
                    .map(|emoji| (emoji.clone(), 1))
                    .collect();
            }
            "react-picker-empty" => {
                let chat = SAMPLES[0].id.to_owned();
                app.reaction_target = Some((chat, "ada-link".into()));
                app.reaction_beside_menu = true;
                app.scroll_to_bottom = false;
                app.scroll_anchor = Some("ada-link".into());
                app.picker_focus = true;
                app.settings.recent_emoji.clear();
            }
            "react-custom" => {
                if let Some(row) = app
                    .conversations
                    .get_mut(SAMPLES[0].id)
                    .and_then(|conversation| conversation.message_mut("ada-link"))
                {
                    row.reactions.retain(|reaction| !reaction.from_me);
                    row.reactions.push(crate::model::Reaction {
                        sender: ME.into(),
                        from_me: true,
                        emoji: "🦀".into(),
                    });
                }
            }
            "react-other" => {
                let group = SAMPLES[1].id;
                app.open_chat = Some(group.to_owned());
                if let Some(chat) = app.chats.iter_mut().find(|chat| chat.id == group) {
                    chat.unread = 0;
                }
                app.scroll_to_bottom = true;
            }
            other => {
                if app.chat(other).is_some() {
                    app.open_chat = Some(other.to_owned());
                    if let Some(chat) = app.chats.iter_mut().find(|chat| chat.id == other) {
                        chat.unread = 0;
                    }
                } else {
                    log::warn!("unknown demo page {other}");
                }
            }
        }
    }
}

const PHONE_MENU_SAMPLE: &str = "Call +1 (555) 010-2040 to arrange the pickup.";

fn phone_menu_sample(app: &mut App) {
    let Some(row) = app
        .conversations
        .get_mut(SAMPLES[0].id)
        .and_then(|conversation| conversation.message_mut("ada-link"))
    else {
        return;
    };
    row.content = Content::text(PHONE_MENU_SAMPLE);
    row.from_me = false;
    app.open_chat = Some(SAMPLES[0].id.into());
    app.scroll_to_bottom = true;
}

pub fn phone_menu_popup_id() -> egui::Id {
    let start = PHONE_MENU_SAMPLE.find('+').unwrap();
    let end = start + "+1 (555) 010-2040".len();
    crate::ui::conversation::bubble_id(SAMPLES[0].id, "ada-link")
        .with(("phone-link", start, end, 0usize))
        .with("popup")
}

/// Three pinned messages in the first sample chat: their lines under the
/// header, their marks on the bubbles, and the notice each pin leaves in the
/// transcript. Plain text, a long line that gets cut, and emoji, so one
/// capture covers all three.
fn pin_sample(app: &mut App) {
    let chat = SAMPLES[0].id;
    let now = crate::util::now();
    // Each pin is made just after the message it names, and the list is handed
    // over newest first, the way `chat_pins` reads it. The notices then belong
    // one above each pinned message, rather than all at the end of the
    // transcript, which is what the ordering bug looked like.
    let sample = [
        (
            "ada-link",
            "btw I made my own Spotify app from scratch! https://spotifast.rocks/",
            2,
        ),
        ("ada-reply", "Listened, agreed on all three points.", 2),
        ("ada-emoji", "😂🎉", 2),
    ];
    let mut rows = Vec::new();
    for (id, text, after) in sample {
        let sent_at = app
            .conversations
            .get(chat)
            .and_then(|conversation| conversation.message(id))
            .map_or(now, |message| message.timestamp);
        app.pins.entry(chat.into()).or_default().insert(id.into());
        rows.push(crate::archive::Pinned {
            chat: chat.into(),
            id: id.into(),
            pinned_at: sent_at + after,
            expires_at: sent_at + 7 * 24 * 60 * 60,
            text: text.into(),
            from_me: true,
            sent_at,
            pinner: crate::archive::Pinner {
                by_me: true,
                sender: String::new(),
            },
        });
    }
    app.chat_pins.insert(chat.into(), rows);
}

/// A request to open `message`'s menu, in the chat that holds it.
fn message_menu(app: &App, message: &str) -> Option<(crate::model::ChatId, String)> {
    app.conversations
        .iter()
        .find(|(_, conversation)| conversation.message(message).is_some())
        .map(|(chat, _)| (chat.clone(), message.to_owned()))
}

fn unlink(app: &mut App) {
    app.chats.clear();
    app.conversations.clear();
    app.open_chat = None;
    app.me = None;
}

/// Builds synthetic history rows spanning multiple dates for the scrolling-date demo.
fn history_date_sample(app: &mut App) {
    let chat = SAMPLES[0].id;
    let base = crate::util::now() - 3 * 86_400;
    let conversation = app.conversations.entry(chat.into()).or_default();
    conversation.messages = (0..90)
        .map(|index| {
            message(
                chat,
                &format!("history-date-{index}"),
                index % 2 == 0,
                base + (index / 30) * 86_400 + (index % 30) * 60,
                Content::text(format!(
                    "Reference note {} for the design review",
                    index + 1
                )),
            )
        })
        .collect();
    conversation.complete = true;
    app.open_chat = Some(chat.into());
    app.scroll_to_bottom = false;
    app.scroll_anchor = Some("history-date-45".into());
}

fn sample_qr() -> String {
    "2@P0wCq0m3R7bC5w8kJgyEUvE8g4mR6qJ1u5o0dQ+K0nH1Lf6xw1GZrJH9fdQmKX3xJfN0oT2XQ5YV8W2v4u7aV1I=,\
     Q9Y8x7W6v5U4t3S2r1Q0p9O8n7M6l5K4j3I2h1G0f9E8d7C6b5A4z3Y2x1W0=,K8j7H6g5F4d3S2a1Q0w9E8r7T6y5U4i3O2p1L0k9J8h7G6f5D4s3A2z1X0c9V8=,\
     v7B6n5M4k3J2h1G0f9D8s7A6z5X4c3V2b1N0m9L8k7J6h5G4f3D2s1A0q9W8e7R6="
        .to_owned()
}

/// Names of all sample chats.
pub fn sample_ids() -> Vec<&'static str> {
    SAMPLES.iter().map(|sample| sample.id).collect()
}

/// Puts a synthetic call on screen, for the call previews and their layout test.
///
/// Demo mode has no engine, no peer, and no camera, so the surface is handed the same snapshot a
/// real call publishes, filled in by hand: an invented call id, one of the sample chats, and a
/// picture that is plainly a test pattern rather than anybody's room. `incoming` is the call
/// waiting to be accepted; otherwise the call is one that has been up for a few minutes.
/// Calls across the sample chats: answered, missed, declined and unanswered,
/// voice and video, one in a group.
fn call_entries(now: i64) -> Vec<Message> {
    use crate::model::CallStatus;
    let call = |chat: &str, id: &str, from_me: bool, minutes: i64, video, status, duration| {
        message(
            chat,
            &format!("{}{id}", crate::model::CALL_ENTRY_PREFIX),
            from_me,
            now - minutes * 60,
            Content::Call {
                video,
                status,
                duration,
                group: false,
            },
        )
    };
    let mut entries = vec![
        call(
            SAMPLES[0].id,
            "ada-1",
            false,
            1,
            false,
            CallStatus::Missed,
            0,
        ),
        // Two missed calls in a row fold into one history row.
        call(
            SAMPLES[0].id,
            "ada-1b",
            false,
            3,
            false,
            CallStatus::Missed,
            0,
        ),
        call(
            SAMPLES[0].id,
            "ada-2",
            true,
            5,
            true,
            CallStatus::Answered,
            754,
        ),
        call(
            SAMPLES[0].id,
            "ada-3",
            false,
            70,
            false,
            CallStatus::Answered,
            221,
        ),
        call(
            SAMPLES[2].id,
            "grace",
            true,
            300,
            false,
            CallStatus::NoAnswer,
            0,
        ),
        call(
            SAMPLES[3].id,
            "katherine",
            false,
            1500,
            true,
            CallStatus::Declined,
            0,
        ),
        call(
            SAMPLES[5].id,
            "margaret",
            true,
            3000,
            false,
            CallStatus::Answered,
            3725,
        ),
    ];
    let mut group = call(
        SAMPLES[1].id,
        "group",
        false,
        600,
        true,
        CallStatus::Missed,
        0,
    );
    group.sender = SAMPLES[2].id.to_owned();
    if let Content::Call { group, .. } = &mut group.content {
        *group = true;
    }
    entries.push(group);
    for entry in &mut entries {
        entry.status = Delivery::None;
    }
    entries
}

/// Opens Rust Berlin with a run of call entries at its end, one of each
/// kind a group's transcript files, for the `call-bubbles` page. They are
/// the newest messages, so they share one screen.
fn call_bubbles_sample(app: &mut App) {
    use crate::model::CallStatus;
    let chat = SAMPLES[1].id;
    let now = crate::util::now();
    let entries = [
        // (sender, minutes ago, video, status, seconds)
        (
            Some(("491701111111@s.whatsapp.net", "Jonas")),
            12,
            true,
            CallStatus::Answered,
            2_173,
        ),
        (None, 10, true, CallStatus::NoAnswer, 0),
        (
            Some(("491702222222@s.whatsapp.net", "Mira")),
            8,
            false,
            CallStatus::Answered,
            36,
        ),
        (None, 6, false, CallStatus::Answered, 754),
        (
            Some(("491703333333@s.whatsapp.net", "Tom")),
            4,
            false,
            CallStatus::Missed,
            0,
        ),
        (
            Some(("491701111111@s.whatsapp.net", "Jonas")),
            2,
            true,
            CallStatus::Declined,
            0,
        ),
    ];
    let Some(conversation) = app.conversations.get_mut(chat) else {
        return;
    };
    for (index, (sender, minutes, video, status, duration)) in entries.into_iter().enumerate() {
        let mut entry = message(
            chat,
            &format!("{}bubble-{index}", crate::model::CALL_ENTRY_PREFIX),
            sender.is_none(),
            now - minutes * 60,
            Content::Call {
                video,
                status,
                duration,
                group: true,
            },
        );
        if let Some((id, name)) = sender {
            entry.sender = id.to_owned();
            entry.sender_name = Some(name.to_owned());
        }
        entry.status = Delivery::None;
        conversation.messages.push(entry);
    }
    conversation
        .messages
        .sort_by_key(|message| message.timestamp);
    app.open_chat = Some(chat.to_owned());
    app.scroll_to_bottom = true;
}

pub fn call_sample(app: &mut App, incoming: bool, video: bool) {
    let chat = SAMPLES[0].id;
    app.call = Some(CallUpdate {
        generation: 1,
        chat: chat.to_owned(),
        direction: if incoming {
            CallDirection::Incoming
        } else {
            CallDirection::Outgoing
        },
        video,
        phase: if incoming {
            CallPhase::Incoming
        } else {
            CallPhase::Active
        },
        // A call already up for three minutes and forty-one seconds, so the previews show the
        // duration the way a real one counts it.
        started: (!incoming)
            .then(|| std::time::Instant::now() - std::time::Duration::from_secs(221)),
        muted: false,
        camera_on: video,
        remote_video: video && !incoming,
        video_requested: false,
        outcome: None,
        peer_audio: None,
        lost_devices: Vec::new(),
        microphone: None,
        speaker: None,
        camera: None,
        group: None,
    });
    app.call_account = Some(app.account().id.clone());
    if video && !incoming {
        app.call_remote_frame = Some(std::sync::Arc::new(stock_frame(stock::SPACEWALK, 720)));
        app.call_local_frame = Some(std::sync::Arc::new(stock_frame(stock::DUSK, 320)));
    }
}

/// A call in the Rust Berlin group, started by Jonas: Mira, Tom and Ada are
/// on it, and Grace was asked and has not joined.
pub fn group_call_sample(app: &mut App, incoming: bool, video: bool) {
    call_sample(app, incoming, video);
    let roster = [
        ("491701111111@s.whatsapp.net", true),
        ("491702222222@s.whatsapp.net", true),
        ("491703333333@s.whatsapp.net", true),
        (SAMPLES[0].id, true),
        (SAMPLES[2].id, false),
    ];
    // Who sends a picture on a video call, and which stand-in it is.
    let cameras = [
        (roster[0].0, stock::VENUE),
        (roster[1].0, stock::SPACEWALK),
        (roster[3].0, stock::LAUNCH),
    ];
    let sending = |id: &str| video && cameras.iter().any(|(camera, _)| *camera == id);
    if let Some(call) = app.call.as_mut() {
        call.chat = SAMPLES[1].id.to_owned();
        call.remote_video = false;
        call.group = Some(crate::calls::GroupCall {
            link: false,
            caller: Some(roster[0].0.to_owned()),
            participants: roster
                .iter()
                .map(|(id, joined)| crate::calls::Participant {
                    id: (*id).to_owned(),
                    joined: *joined,
                    video: sending(id),
                })
                .collect(),
        });
    }
    app.call_remote_frame = None;
    if video && !incoming {
        for (id, photo) in cameras {
            app.call_member_frames
                .insert(id.to_owned(), std::sync::Arc::new(stock_frame(photo, 360)));
        }
    }
}

/// A stock picture as a video frame no taller than `height`: the demo's
/// stand-in for a camera, with no person a real call could show.
fn stock_frame(photo: stock::Photo, height: u32) -> egui::ColorImage {
    let picture = image::load_from_memory(photo.bytes)
        .expect("the demo pictures decode")
        .resize(height * 2, height, image::imageops::FilterType::Triangle)
        .to_rgba8();
    egui::ColorImage::from_rgba_unmultiplied(
        [picture.width() as usize, picture.height() as usize],
        picture.as_raw(),
    )
}

#[allow(dead_code)]
fn contacts_by_id(app: &App) -> HashMap<&str, &Contact> {
    app.contacts
        .iter()
        .map(|(id, contact)| (id.as_str(), contact))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::AppDirs;
    use crate::settings::Settings;

    thread_local! {
        /// The folders of this test's apps, removed when the test's thread
        /// ends (each test runs on a thread of its own).
        static ROOTS: std::cell::RefCell<Vec<tempfile::TempDir>> = const {
            std::cell::RefCell::new(Vec::new())
        };
    }

    pub(super) fn app() -> App {
        let root = tempfile::Builder::new()
            .prefix("zapfast-demo-")
            .tempdir()
            .unwrap();
        let (mut app, _events) = App::headless(AppDirs::under(root.path()), Settings::default());
        ROOTS.with(|roots| roots.borrow_mut().push(root));
        populate(&mut app);
        app
    }

    /// Lays out several frames without a display to catch view panics.
    pub(super) fn render(app: &mut App, ctx: &egui::Context) {
        for _ in 0..3 {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1180.0, 780.0),
                )),
                ..Default::default()
            };
            let mut output = ctx.run_ui(input, |ui| {
                let ctx = ui.ctx().clone();
                app.background_frame(&ctx);
                app.frame_ui(ui);
            });
            // Headless tests must apply font-atlas updates themselves.
            output.textures_delta.clear();
        }
    }

    /// One frame with a key press, then `render`'s frames to settle.
    pub(super) fn press_key(
        app: &mut App,
        ctx: &egui::Context,
        key: egui::Key,
        modifiers: egui::Modifiers,
    ) {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1180.0, 780.0),
                )),
                events: vec![egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers,
                }],
                ..Default::default()
            },
            |ui| {
                let ctx = ui.ctx().clone();
                app.background_frame(&ctx);
                app.frame_ui(ui);
            },
        );
        output.textures_delta.clear();
        render(app, ctx);
    }

    /// One frame in which `key` is let go of.
    pub(super) fn release_key(app: &mut App, ctx: &egui::Context, key: egui::Key) {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1180.0, 780.0),
                )),
                events: vec![egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed: false,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }],
                ..Default::default()
            },
            |ui| {
                let ctx = ui.ctx().clone();
                app.background_frame(&ctx);
                app.frame_ui(ui);
            },
        );
        output.textures_delta.clear();
    }

    /// One frame with `key` still held down, as a keyboard repeats it: the
    /// repeated press and the text it types.
    pub(super) fn hold_key(app: &mut App, ctx: &egui::Context, key: egui::Key, text: &str) {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1180.0, 780.0),
                )),
                events: vec![
                    egui::Event::Key {
                        key,
                        physical_key: None,
                        pressed: true,
                        repeat: true,
                        modifiers: egui::Modifiers::NONE,
                    },
                    egui::Event::Text(text.to_owned()),
                ],
                ..Default::default()
            },
            |ui| {
                let ctx = ui.ctx().clone();
                app.background_frame(&ctx);
                app.frame_ui(ui);
            },
        );
        output.textures_delta.clear();
    }

    /// Two contacts typing in the open group keep the window drawing every
    /// refresh, but only about ten times a second while a screen reader is
    /// attached: at the display's rate its thread could not answer the
    /// reader's window messages in time.
    #[test]
    fn a_contact_typing_steps_the_window_only_with_a_screen_reader() {
        let run = |reader: bool| {
            let mut app = app();
            apply_flags(&mut app, Some("typers"));
            let ctx = egui::Context::default();
            app.attach(&ctx);
            if reader {
                ctx.enable_accesskit();
            }
            let mut delay = std::time::Duration::MAX;
            // Avatars and media load on threads that ask for a frame when
            // they finish; measure once three frames in a row had none
            // pending.
            let mut settled = 0;
            for frame in 0..400 {
                if ctx.has_pending_images() {
                    settled = 0;
                    std::thread::sleep(std::time::Duration::from_millis(10));
                } else {
                    settled += 1;
                }
                if frame >= 8 && settled > 3 {
                    break;
                }
                let input = egui::RawInput {
                    time: Some(10.0 + f64::from(frame) * 0.1),
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1180.0, 780.0),
                    )),
                    ..Default::default()
                };
                let mut output = ctx.run_ui(input, |ui| {
                    let ctx = ui.ctx().clone();
                    app.background_frame(&ctx);
                    app.frame_ui(ui);
                });
                output.textures_delta.clear();
                delay = output.viewport_output[&egui::ViewportId::ROOT].repaint_delay;
            }
            (delay, ctx.repaint_causes())
        };
        let from_the_dots = |causes: &[egui::RepaintCause]| {
            causes
                .iter()
                .any(|cause| cause.file.ends_with("conversation.rs"))
        };
        let (plain, causes) = run(false);
        assert_eq!(plain, std::time::Duration::ZERO, "{causes:?}");
        assert!(from_the_dots(&causes), "{causes:?}");
        let (stepped, causes) = run(true);
        assert!(
            stepped > std::time::Duration::from_millis(75) && stepped <= crate::motion::STEP,
            "{stepped:?}: {causes:?}"
        );
        assert!(from_the_dots(&causes), "{causes:?}");
    }

    /// A sent message moves its chat up, and a chat list within a row of the
    /// top follows it there; a list scrolled further down keeps its place.
    #[test]
    fn sending_a_message_scrolls_the_chat_list_back_to_the_top_only_from_near_it() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        let offset = || {
            ctx.data(|data| data.get_temp::<f32>(crate::ui::chats::list_offset_id()))
                .expect("the chat list was drawn")
        };
        // Short enough that the sample chats do not all fit.
        let render = |app: &mut App| {
            for _ in 0..3 {
                let input = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1180.0, 420.0),
                    )),
                    ..Default::default()
                };
                let mut output = ctx.run_ui(input, |ui| {
                    let ctx = ui.ctx().clone();
                    app.background_frame(&ctx);
                    app.frame_ui(ui);
                });
                output.textures_delta.clear();
            }
        };
        let send = |app: &mut App| {
            let last = app.visible_chats().last().map(|chat| chat.id.clone());
            app.actions.push(crate::model::Action::SendText {
                chat: last.expect("sample chats"),
                text: "Fixture".into(),
                quoting: None,
            });
            render(app);
        };
        let last = app.visible_chats().last().map(|chat| chat.id.clone());
        app.scroll_chat_into_view = last;
        render(&mut app);
        let far = offset();
        assert!(far > crate::theme::ROW_HEIGHT, "the list starts far down");
        send(&mut app);
        assert_eq!(offset(), far, "a reader far down keeps their place");

        let near = crate::theme::ROW_HEIGHT / 2.0;
        let id = ctx
            .data(|data| data.get_temp::<egui::Id>(crate::ui::chats::list_offset_id().with("id")))
            .expect("the chat list was drawn");
        let mut state = egui::scroll_area::State::load(&ctx, id).expect("scroll state");
        state.offset.y = near;
        state.store(&ctx, id);
        render(&mut app);
        assert_eq!(offset(), near, "the list is just below the top");
        send(&mut app);
        assert_eq!(offset(), 0.0, "the list is back at the top");
    }

    /// Enter opens the batch menu while retaining the selected messages and checkbox focus.
    #[test]
    fn keyboard_activation_opens_batch_delete_without_changing_selection() {
        let mut app = app();
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        app.attach(&ctx);
        let chat = SAMPLES[0].id;
        let selected = vec!["ada-doc".to_owned(), "ada-reply".to_owned()];
        app.selection = Some((chat.into(), selected.clone()));
        render(&mut app, &ctx);
        let nodes = accessible_nodes(&mut app, &ctx, Vec::new());
        assert!(
            nodes
                .iter()
                .any(|(label, _, _)| label.starts_with("Select message from ")),
            "selection rows have accessible labels"
        );
        let pick = crate::ui::conversation::bubble_id(chat, "ada-reply").with("pick");
        ctx.memory_mut(|memory| memory.request_focus(pick));
        frame_with(
            &mut app,
            &ctx,
            vec![key(egui::Key::Enter, egui::Modifiers::NONE)],
        );
        render(&mut app, &ctx);
        assert!(egui::Popup::is_id_open(&ctx, pick.with("popup")));
        assert_eq!(app.selection.as_ref().unwrap().1, selected);
        assert!(app.dialog.is_none(), "opening the menu never deletes");
        let bubble = crate::ui::conversation::bubble_id(chat, "ada-reply");
        let row_outline = ctx
            .data(|data| data.get_temp::<crate::theme::FocusOutline>(pick.with("focus-outline")))
            .unwrap();
        let bubble_outline = ctx
            .data(|data| data.get_temp::<crate::theme::FocusOutline>(bubble.with("focus-outline")))
            .unwrap();
        let check = ctx
            .data(|data| data.get_temp::<egui::Rect>(bubble.with("check")))
            .unwrap();
        assert!(row_outline.rect.contains_rect(check));
        assert_eq!(row_outline.rect, bubble_outline.rect);
    }

    /// A clicked notification lands on the message it announced and keeps it
    /// in view, even with the unread divider far above it.
    #[test]
    fn an_opened_message_stays_in_view_below_a_distant_unread_divider() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        let chat = SAMPLES[0].id;
        let conversation = app.conversations.get_mut(chat).unwrap();
        let mut template = conversation.messages.last().unwrap().clone();
        template.from_me = false;
        for n in 0..40 {
            let mut row = template.clone();
            row.id = format!("unread-{n}");
            row.timestamp = template.timestamp + 1 + n;
            row.content = crate::model::Content::text(format!("Unread line {n}"));
            conversation.messages.push(row);
        }
        let mut announced = template.clone();
        announced.id = "announced".into();
        announced.timestamp = template.timestamp + 100;
        announced.content = crate::model::Content::text("The announced message");
        conversation.messages.push(announced);
        app.chats
            .iter_mut()
            .find(|row| row.id == chat)
            .unwrap()
            .unread = 41;
        app.open_chat = None;

        app.actions.push(crate::model::Action::OpenMessage {
            chat: chat.into(),
            message: "announced".into(),
        });
        render(&mut app, &ctx);
        render(&mut app, &ctx);
        let shapes = frame_sized(&mut app, &ctx, 780.0, Vec::new());
        let visible = shapes.iter().any(|clipped| {
            matches!(&clipped.shape, egui::Shape::Text(text)
                if text.galley.text().contains("The announced message")
                    && clipped.clip_rect.contains(text.pos + egui::vec2(1.0, 1.0)))
        });
        assert!(visible, "the announced message is on screen");
    }

    /// Paints the self-chat through the real bubble path and checks what reaches
    /// the screen: every row in bidi order, brackets mirrored, the message
    /// flush right, and the time on its own row at the bottom right.
    #[test]
    fn rtl_self_chat_bubbles_render_like_whatsapp() {
        fn collect(
            shape: &egui::Shape,
            out: &mut Vec<(egui::Pos2, std::sync::Arc<egui::Galley>)>,
            images: &mut Vec<egui::Rect>,
        ) {
            match shape {
                egui::Shape::Text(text) => out.push((text.pos, text.galley.clone())),
                egui::Shape::Mesh(mesh) if mesh.texture_id != egui::TextureId::default() => {
                    images.push(mesh.calc_bounds());
                }
                egui::Shape::Vec(shapes) => {
                    for shape in shapes {
                        collect(shape, out, images);
                    }
                }
                _ => {}
            }
        }
        let mut app = app();
        apply_flags(&mut app, Some("rtl-self"));
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        let shapes = frame_sized(&mut app, &ctx, 780.0, Vec::new());
        let mut texts = Vec::new();
        let mut images = Vec::new();
        for shape in &shapes {
            collect(&shape.shape, &mut texts, &mut images);
        }
        let atlas = ctx.fonts(|fonts| fonts.image());
        let clocks: Vec<String> = app.conversations[ME]
            .messages
            .iter()
            .map(|message| crate::util::clock(message.timestamp))
            .collect();
        assert_ne!(clocks[0], clocks[1], "each bubble has its own time");
        for (first_line, clock) in [
            ("בדיקת RTL בלבד\n", &clocks[0]),
            ("שלום!\n", &clocks[1]),
            ("إلى السطر التالي\n", &clocks[2]),
        ] {
            let (pos, body) = texts
                .iter()
                .find(|(_, galley)| galley.text().starts_with(first_line))
                .unwrap_or_else(|| panic!("painted body starting {first_line:?}"));
            crate::bidi::assert_rows_follow_uba(body, &atlas);
            crate::bidi::assert_right_aligned(body);
            let body_right = body
                .rows
                .iter()
                .flat_map(|placed| {
                    placed
                        .row
                        .glyphs
                        .iter()
                        .map(move |glyph| pos.x + placed.pos.x + glyph.max_x())
                })
                .fold(f32::NEG_INFINITY, f32::max);
            let body_bottom = pos.y + body.rows.last().expect("rows").rect().max.y;
            let (time_pos, time) = texts
                .iter()
                .find(|(_, galley)| galley.text() == clock.as_str())
                .unwrap_or_else(|| panic!("painted time {clock}"));
            assert!(
                time_pos.y >= body_bottom - 0.5,
                "time at {} overlaps the last row ending at {body_bottom}",
                time_pos.y
            );
            let gap = body_right - (time_pos.x + time.size().x);
            assert!(
                (0.0..40.0).contains(&gap),
                "time should end beside the ticks at the right edge, {gap} short of it"
            );
            let body_rect = body
                .rows
                .iter()
                .map(|placed| placed.rect().translate(pos.to_vec2()))
                .reduce(|a, b| a.union(b))
                .expect("rows");
            let row_height = body.rows[0].row.size.y;
            // A picture may overhang its row: Apple's cell, transparent above
            // and below the emoji, and Segoe UI Emoji's narrower families,
            // drawn at their full height, reach up to a tenth of a row past it.
            let overhang = row_height * 0.1;
            // Only images inside the text are emoji; a wallpaper tile behind
            // the bubble can have its centre there too.
            let emoji: Vec<&egui::Rect> = images
                .iter()
                .filter(|image| {
                    body_rect
                        .expand2(egui::vec2(1.0, overhang))
                        .contains_rect(**image)
                })
                .collect();
            let placeholders = body.text().matches(crate::emoji::PLACEHOLDER).count();
            if crate::emoji::available() {
                assert_eq!(emoji.len(), placeholders, "one bitmap per emoji");
            }
            for image in &emoji {
                assert!(
                    image.width().max(image.height()) >= row_height,
                    "emoji {image:?} shrank below the row height {row_height}"
                );
            }
            for placed in &body.rows {
                for glyph in &placed.row.glyphs {
                    if glyph.uv_rect.is_nothing()
                        || glyph.chr.is_whitespace()
                        || glyph.chr == crate::emoji::PLACEHOLDER
                    {
                        continue;
                    }
                    let ink = egui::Rect::from_min_size(
                        *pos + placed.pos.to_vec2() + egui::vec2(glyph.pos.x, 0.0),
                        egui::vec2(glyph.advance_width, placed.row.size.y),
                    );
                    for image in &emoji {
                        // Only a sideways overlap in the same line counts.
                        assert!(
                            !image.shrink2(egui::vec2(0.5, overhang)).intersects(ink),
                            "emoji at {image:?} overlaps {:?} at {ink:?}",
                            glyph.chr
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn interactive_text_participates_in_selection_and_transcripts() {
        let mut app = app();
        apply_flags(&mut app, Some("interactive"));
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        let rows = app.copy_rows.lock().unwrap();
        assert!(rows.iter().any(|row| row.body.contains("Tell me more")));
        assert!(
            rows.iter()
                .all(|row| !row.body.contains("View full message"))
        );
        assert!(rows.iter().filter(|row| row.body == "Tell me more").count() == 1);
        let body =
            crate::ui::conversation::bubble_id(SAMPLES[0].id, "interactive-reply").with("body");
        let rect = ctx
            .data(|data| data.get_temp::<egui::Rect>(body))
            .expect("selectable body");
        assert!(rect.is_positive());
    }

    #[test]
    fn interactive_card_images_join_transcripts_once() {
        for (single, body) in [(true, false), (false, false), (true, true), (false, true)] {
            let mut app = app();
            carousel_sample(&mut app, 2);
            let message = &mut app.conversations.get_mut(SAMPLES[0].id).unwrap().messages[0];
            message.reactions = vec![crate::model::Reaction {
                sender: SAMPLES[0].id.into(),
                from_me: false,
                emoji: "👍".into(),
            }];
            let Content::Interactive {
                card: Some(card), ..
            } = &mut message.content
            else {
                panic!("interactive sample");
            };
            if !body {
                card.body.clear();
            }
            if single {
                card.image = card.carousel[0].image.clone();
                card.carousel.clear();
            }
            let ctx = egui::Context::default();
            app.attach(&ctx);
            render(&mut app, &ctx);
            let rows = app.copy_rows.lock().unwrap();
            let case = format!("single card: {single}, body: {body}");
            assert_eq!(
                rows.iter()
                    .filter(|row| row.marker.as_deref() == Some("[photo]"))
                    .count(),
                1,
                "{case}"
            );
            // Carousel cards are parts of one message, not replies or reactions.
            assert_eq!(
                rows.iter().filter(|row| !row.reactions.is_empty()).count(),
                1,
                "{case}"
            );
        }
    }

    #[test]
    fn interactive_links_and_replies_activate_by_click_and_keyboard() {
        let mut app = app();
        apply_flags(&mut app, Some("interactive"));
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        let run = |app: &mut App, events| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1180.0, 780.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    let ctx = ui.ctx().clone();
                    app.background_frame(&ctx);
                    app.frame_ui(ui);
                },
            );
            output.textures_delta.clear();
            output
                .platform_output
                .commands
                .into_iter()
                .filter_map(|command| match command {
                    egui::OutputCommand::OpenUrl(url) => Some(url.url),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        app.backend.record_demo_commands();
        for (id, expected) in [
            ("interactive-card", Vec::new()),
            ("interactive-link", vec!["https://example.com/".to_owned()]),
        ] {
            let button = crate::ui::conversation::bubble_id(SAMPLES[0].id, id)
                .with(("interactive-button", 0usize));
            let rect = ctx
                .data(|data| data.get_temp::<egui::Rect>(button))
                .unwrap();
            let pos = rect.center();
            let press = |pressed| egui::Event::PointerButton {
                pos,
                pressed,
                button: egui::PointerButton::Primary,
                modifiers: egui::Modifiers::NONE,
            };
            run(&mut app, vec![egui::Event::PointerMoved(pos), press(true)]);
            assert_eq!(run(&mut app, vec![press(false)]), expected);
            assert_eq!(app.conversations[SAMPLES[0].id].messages.len(), 3);
        }
        let commands = app.backend.take_demo_commands();
        assert_eq!(commands.iter().filter(|command| matches!(command, crate::backend::Command::ReplyInteractive { chat, message, button: 0, choice: None } if chat == SAMPLES[0].id && message == "interactive-card")).count(), 1);
        let reply_button = crate::ui::conversation::bubble_id(SAMPLES[0].id, "interactive-card")
            .with(("interactive-action", 0usize));
        ctx.memory_mut(|memory| memory.request_focus(reply_button));
        run(&mut app, vec![key(egui::Key::Enter, egui::Modifiers::NONE)]);
        assert!(
            app.backend
                .take_demo_commands()
                .iter()
                .any(|command| matches!(
                    command,
                    crate::backend::Command::ReplyInteractive {
                        button: 0,
                        choice: None,
                        ..
                    }
                ))
        );
        let button = crate::ui::conversation::bubble_id(SAMPLES[0].id, "interactive-link")
            .with(("interactive-action", 0usize));
        ctx.memory_mut(|memory| memory.request_focus(button));
        assert_eq!(
            run(&mut app, vec![key(egui::Key::Enter, egui::Modifiers::NONE)]),
            vec!["https://example.com/"]
        );
    }

    #[test]
    fn phone_number_link_opens_its_actions_from_the_keyboard() {
        let mut app = app();
        app.backend.record_demo_commands();
        let chat = SAMPLES[0].id;
        let row = app
            .conversations
            .get_mut(chat)
            .unwrap()
            .message_mut("ada-link")
            .unwrap();
        let body = "اتصل على +99 (999) 99999-9999";
        row.content = Content::text(body);
        row.from_me = false;

        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        let run = |app: &mut App, events| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(400.0, 780.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    let ctx = ui.ctx().clone();
                    app.background_frame(&ctx);
                    app.frame_ui(ui);
                },
            );
            output.textures_delta.clear();
            output.platform_output.commands
        };
        // Register controls at the same width used by synthetic keyboard
        // input so a wrapped link has the same hit-region id in both passes.
        run(&mut app, vec![]);
        let phone_links = crate::ui::focus::stops(&ctx)
            .into_iter()
            .filter_map(|(stop, id)| {
                matches!(stop, crate::ui::focus::Stop::PhoneLink(_)).then_some(id)
            })
            .collect::<Vec<_>>();
        assert!(
            !phone_links.is_empty(),
            "the phone link is keyboard reachable"
        );
        let phone = phone_links[0];
        let phone_last = *phone_links.last().unwrap();
        let previous = crate::ui::focus::control(&ctx, crate::ui::focus::Stop::Emoji).unwrap();
        let next = crate::ui::focus::control(&ctx, crate::ui::focus::Stop::ChatSearch).unwrap();
        let open_menu = |app: &mut App, from: egui::Id, target: egui::Id, tab: egui::Modifiers| {
            ctx.memory_mut(|memory| memory.request_focus(from));
            run(app, vec![key(egui::Key::Tab, tab)]);
            assert_eq!(ctx.memory(|memory| memory.focused()), Some(target));
            run(app, vec![key(egui::Key::Enter, egui::Modifiers::NONE)]);
            run(app, vec![]);
            assert!(egui::Popup::is_id_open(&ctx, target.with("popup")));
        };
        let click = |app: &mut App, rect: egui::Rect| {
            let pos = rect.center();
            let button = |pressed| egui::Event::PointerButton {
                pos,
                pressed,
                button: egui::PointerButton::Primary,
                modifiers: egui::Modifiers::NONE,
            };
            run(app, vec![egui::Event::PointerMoved(pos), button(true)]);
            run(app, vec![button(false)])
        };

        open_menu(&mut app, previous, phone, egui::Modifiers::NONE);
        let copy_rect = ctx
            .data(|data| data.get_temp::<egui::Rect>(phone.with("test-copy-action")))
            .unwrap();
        let copied = click(&mut app, copy_rect)
            .into_iter()
            .find_map(|command| match command {
                egui::OutputCommand::CopyText(text) => Some(text),
                _ => None,
            })
            .unwrap();
        assert_eq!(copied, "+99 (999) 99999-9999");

        open_menu(&mut app, next, phone_last, egui::Modifiers::SHIFT);
        let message_rect = ctx
            .data(|data| data.get_temp::<egui::Rect>(phone_last.with("test-message-action")))
            .unwrap();
        click(&mut app, message_rect);
        // The menu is activated through the existing NewContact command,
        // whose worker path checks whether the number is registered.
        assert!(
            app.backend
                .take_demo_commands()
                .iter()
                .any(|command| matches!(
                    command,
                    crate::backend::Command::NewContact {
                        phone,
                        full_name: None,
                        ..
                    } if phone == "99999999999999"
                ))
        );
    }

    #[test]
    fn interactive_lists_copy_codes_and_unavailable_actions_use_the_correct_paths() {
        let mut app = app();
        apply_flags(&mut app, Some("interactive-actions"));
        app.backend.record_demo_commands();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        let run = |app: &mut App, events| {
            let mut out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1180.0, 780.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    let ctx = ui.ctx().clone();
                    app.background_frame(&ctx);
                    app.frame_ui(ui);
                },
            );
            out.textures_delta.clear();
            out.platform_output.commands
        };
        let bubble = crate::ui::conversation::bubble_id(SAMPLES[0].id, "interactive-card");
        // Keyboard opens the modal; choosing a row sends only its index.
        ctx.memory_mut(|memory| memory.request_focus(bubble.with(("interactive-action", 1usize))));
        run(&mut app, vec![key(egui::Key::Enter, egui::Modifiers::NONE)]);
        render(&mut app, &ctx);
        assert!(
            !app.backend
                .take_demo_commands()
                .iter()
                .any(|c| matches!(c, crate::backend::Command::ReplyInteractive { .. }))
        );
        assert!(matches!(
            app.dialog,
            Some(Dialog::InteractiveList { button: 1, .. })
        ));
        let option = bubble.with(("interactive-option", 1usize, 1usize));
        assert!(
            ctx.data(|data| data.get_temp::<egui::Rect>(option))
                .is_some()
        );
        ctx.memory_mut(|memory| memory.request_focus(option));
        run(&mut app, vec![key(egui::Key::Enter, egui::Modifiers::NONE)]);
        assert!(app.backend.take_demo_commands().iter().any(|c| matches!(
            c,
            crate::backend::Command::ReplyInteractive {
                button: 1,
                choice: Some(1),
                ..
            }
        )));
        assert!(app.dialog.is_none(), "choosing an item closes the dialog");
        render(&mut app, &ctx);
        for index in [2usize, 3] {
            let rect = ctx
                .data(|data| {
                    data.get_temp::<egui::Rect>(bubble.with(("interactive-button", index)))
                })
                .unwrap();
            let press = |pressed| egui::Event::PointerButton {
                pos: rect.center(),
                pressed,
                button: egui::PointerButton::Primary,
                modifiers: egui::Modifiers::NONE,
            };
            run(
                &mut app,
                vec![egui::Event::PointerMoved(rect.center()), press(true)],
            );
            let output = run(&mut app, vec![press(false)]);
            let copied: Vec<_> = output
                .iter()
                .filter_map(|c| {
                    if let egui::OutputCommand::CopyText(text) = c {
                        Some(text.as_str())
                    } else {
                        None
                    }
                })
                .collect();
            assert_eq!(
                copied,
                if index == 2 {
                    vec!["CEDAR20"]
                } else {
                    Vec::new()
                }
            );
            assert!(!app.backend.take_demo_commands().iter().any(|c| matches!(
                c,
                crate::backend::Command::ReplyInteractive { .. }
                    | crate::backend::Command::SendText { .. }
            )));
        }
    }

    #[test]
    fn interactive_replies_disable_only_while_unavailable_or_sending() {
        for state in ["offline", "readonly", "own", "pending", "available"] {
            let mut app = app();
            apply_flags(&mut app, Some("interactive"));
            app.backend.record_demo_commands();
            match state {
                "offline" => app.link = crate::backend::LinkStatus::Connecting,
                "readonly" => {
                    app.chats
                        .iter_mut()
                        .find(|chat| chat.id == SAMPLES[0].id)
                        .unwrap()
                        .read_only = true
                }
                "own" => {
                    app.conversations.get_mut(SAMPLES[0].id).unwrap().messages[0].from_me = true
                }
                "pending" => {
                    app.interactive_sending
                        .insert((SAMPLES[0].id.into(), "interactive-card".into()));
                }
                _ => {}
            }
            let ctx = egui::Context::default();
            app.attach(&ctx);
            render(&mut app, &ctx);
            let id = crate::ui::conversation::bubble_id(SAMPLES[0].id, "interactive-card")
                .with(("interactive-button", 0usize));
            let rect = ctx.data(|data| data.get_temp::<egui::Rect>(id)).unwrap();
            let press = |pressed| egui::Event::PointerButton {
                pos: rect.center(),
                pressed,
                button: egui::PointerButton::Primary,
                modifiers: egui::Modifiers::NONE,
            };
            frame_with(
                &mut app,
                &ctx,
                vec![egui::Event::PointerMoved(rect.center()), press(true)],
            );
            frame_with(&mut app, &ctx, vec![press(false)]);
            let sent = app
                .backend
                .take_demo_commands()
                .iter()
                .any(|c| matches!(c, crate::backend::Command::ReplyInteractive { .. }));
            assert_eq!(sent, state == "available", "{state}");
        }
    }

    #[test]
    fn interactive_rows_stay_inside_the_bubble_at_narrow_widths() {
        for (width, light, own) in [
            (640.0, false, false),
            (640.0, true, false),
            (1180.0, false, false),
            (640.0, false, true),
            (1180.0, true, true),
        ] {
            let mut app = app();
            apply_flags(&mut app, Some("interactive-media"));
            if own {
                let message = &mut app.conversations.get_mut(SAMPLES[0].id).unwrap().messages[0];
                message.from_me = true;
                message.sender = ME.into();
            }
            if light {
                app.settings.theme = ThemeChoice::Light;
            }
            let ctx = egui::Context::default();
            app.attach(&ctx);
            for _ in 0..4 {
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(width, 1100.0),
                        )),
                        ..Default::default()
                    },
                    |ui| {
                        app.frame_ui(ui);
                    },
                );
                output.textures_delta.clear();
            }
            let bubble = crate::ui::conversation::bubble_id(SAMPLES[0].id, "interactive-card");
            let bounds = ctx
                .data(|data| data.get_temp::<egui::Rect>(bubble.with("rect")))
                .unwrap();
            assert!(bounds.right() <= width + 1.0, "{bounds:?}");
            for index in 0..3usize {
                let row = ctx
                    .data(|data| {
                        data.get_temp::<egui::Rect>(bubble.with(("interactive-button", index)))
                    })
                    .unwrap();
                assert!(row.left() >= bounds.left() && row.right() <= bounds.right());
                assert!(row.height() >= 44.0);
            }
        }
    }

    #[test]
    fn poll_results_keep_zero_vote_options_visible() {
        let mut app = app();
        poll_sample(&mut app, true, true);
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        let id = crate::ui::conversation::bubble_id(SAMPLES[0].id, "poll-demo");
        let viewport = ctx
            .data(|data| data.get_temp::<egui::Rect>(id.with("poll-results-viewport")))
            .unwrap();
        for index in 0..3usize {
            let row = ctx
                .data(|data| data.get_temp::<egui::Rect>(id.with(("poll-result-option", index))))
                .unwrap();
            assert!(
                viewport.contains_rect(row),
                "option {index}: {row:?}, viewport: {viewport:?}"
            );
        }
    }

    #[test]
    fn receipt_privacy_demo_keeps_played_voice_visible() {
        let mut app = app();
        apply_flags(&mut app, Some("chat,receipts-off,receipts-played"));
        assert!(app.account_receipts_off);
        let conversation = &app.conversations[SAMPLES[0].id];
        assert!(conversation.messages.iter().any(|message| {
            message.from_me
                && matches!(message.content, Content::Audio { .. })
                && message.status == crate::model::Delivery::Played
        }));
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
    }

    /// Message info grows with its content like poll results, so a short
    /// list, the note about missing receipts included, shows without
    /// scrolling.
    #[test]
    fn short_message_info_shows_in_full() {
        for page in [
            "message-info-partial",
            "message-info-unknown",
            "message-info-direct",
        ] {
            let mut app = app();
            apply_flags(&mut app, Some(page));
            let ctx = egui::Context::default();
            app.attach(&ctx);
            render(&mut app, &ctx);
            let Some(Dialog::MessageInfo { chat, message }) = app.dialog.clone() else {
                panic!("{page}: no message info");
            };
            let id = crate::ui::conversation::bubble_id(&chat, &message);
            let viewport = ctx
                .data(|data| data.get_temp::<egui::Rect>(id.with("message-info-viewport")))
                .unwrap();
            let content = ctx
                .data(|data| data.get_temp::<egui::Vec2>(id.with("message-info-content")))
                .unwrap();
            assert!(
                content.y <= viewport.height() + 0.5,
                "{page}: content {content:?}, viewport {viewport:?}"
            );
        }
    }

    #[test]
    fn list_dialog_dismissal_and_connection_changes_do_not_send_replies() {
        for state in ["escape", "disconnected", "pending", "edited", "removed"] {
            let mut app = app();
            interactive_list_sample(&mut app);
            app.backend.record_demo_commands();
            let ctx = egui::Context::default();
            app.attach(&ctx);
            render(&mut app, &ctx);
            app.actions
                .push(crate::model::Action::ShowDialog(Dialog::InteractiveList {
                    chat: SAMPLES[0].id.into(),
                    message: "interactive-card".into(),
                    button: 0,
                }));
            render(&mut app, &ctx);
            let option = crate::ui::conversation::bubble_id(SAMPLES[0].id, "interactive-card")
                .with(("interactive-option", 0usize, 0usize));
            match state {
                "disconnected" => {
                    app.link = LinkStatus::Disconnected {
                        reason: "Offline".into(),
                    }
                }
                "pending" => {
                    app.interactive_sending
                        .insert((SAMPLES[0].id.into(), "interactive-card".into()));
                }
                "edited" => {
                    app.conversations.get_mut(SAMPLES[0].id).unwrap().messages[0].edited = true
                }
                "removed" => app
                    .conversations
                    .get_mut(SAMPLES[0].id)
                    .unwrap()
                    .messages
                    .clear(),
                _ => {}
            }
            render(&mut app, &ctx);
            ctx.memory_mut(|memory| memory.request_focus(option));
            frame_with(
                &mut app,
                &ctx,
                vec![key(
                    if state == "escape" {
                        egui::Key::Escape
                    } else {
                        egui::Key::Enter
                    },
                    egui::Modifiers::NONE,
                )],
            );
            assert!(
                !app.backend
                    .take_demo_commands()
                    .iter()
                    .any(|c| matches!(c, crate::backend::Command::ReplyInteractive { .. })),
                "{state}"
            );
            if state == "escape" {
                assert!(app.dialog.is_none());
            }
        }
    }

    #[test]
    fn poll_results_open_only_with_votes_and_ballots_send_choices() {
        for voted in [false, true] {
            let mut app = app();
            poll_sample(&mut app, voted, false);
            app.backend.record_demo_commands();
            let ctx = egui::Context::default();
            app.attach(&ctx);
            render(&mut app, &ctx);
            let bubble = crate::ui::conversation::bubble_id(SAMPLES[0].id, "poll-demo");
            let rect = ctx
                .data(|data| data.get_temp::<egui::Rect>(bubble.with("poll-results-rect")))
                .unwrap();
            let click = |pressed| egui::Event::PointerButton {
                pos: rect.center(),
                pressed,
                button: egui::PointerButton::Primary,
                modifiers: egui::Modifiers::NONE,
            };
            frame_with(
                &mut app,
                &ctx,
                vec![egui::Event::PointerMoved(rect.center()), click(true)],
            );
            frame_with(&mut app, &ctx, vec![click(false)]);
            assert_eq!(
                matches!(app.dialog, Some(Dialog::PollResults { .. })),
                voted
            );
            assert!(
                !app.backend
                    .take_demo_commands()
                    .iter()
                    .any(|c| matches!(c, crate::backend::Command::VotePoll { .. }))
            );
            if !voted {
                let rect = ctx
                    .data(|data| data.get_temp::<egui::Rect>(bubble.with(("poll-option", 1usize))))
                    .unwrap();
                let click = |pressed| egui::Event::PointerButton {
                    pos: rect.center(),
                    pressed,
                    button: egui::PointerButton::Primary,
                    modifiers: egui::Modifiers::NONE,
                };
                frame_with(
                    &mut app,
                    &ctx,
                    vec![egui::Event::PointerMoved(rect.center()), click(true)],
                );
                frame_with(&mut app, &ctx, vec![click(false)]);
                assert!(app.backend.take_demo_commands().iter().any(|c| matches!(c, crate::backend::Command::VotePoll { choices, .. } if choices == &[1])));
            }
        }
    }

    /// A call in the chat is a bubble as wide as its words, on either side:
    /// in an own bubble the row runs right to left, and its text column once
    /// took every point the row offered, stretching across the transcript.
    #[test]
    fn a_call_bubble_is_as_wide_as_its_words() {
        let mut app = app();
        let chat = SAMPLES[0].id;
        app.open_chat = Some(chat.to_owned());
        let call = |id: &str, from_me: bool| {
            message(
                chat,
                &format!("{}{id}", crate::model::CALL_ENTRY_PREFIX),
                from_me,
                crate::util::now(),
                Content::Call {
                    video: false,
                    status: crate::model::CallStatus::Declined,
                    duration: 0,
                    group: false,
                },
            )
        };
        let conversation = app.conversations.get_mut(chat).unwrap();
        conversation.messages.push(call("width-in", false));
        conversation.messages.push(call("width-out", true));
        app.scroll_to_bottom = true;
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        render(&mut app, &ctx);
        let width = |id: &str| {
            let id = crate::ui::conversation::bubble_id(
                chat,
                &format!("{}{id}", crate::model::CALL_ENTRY_PREFIX),
            )
            .with("rect");
            ctx.data(|data| data.get_temp::<egui::Rect>(id))
                .expect("the call bubble was drawn")
                .width()
        };
        let incoming = width("width-in");
        let own = width("width-out");
        assert!(own < 240.0, "an own call bubble fits its words: {own}");
        assert!(
            (own - incoming).abs() < 24.0,
            "both sides are alike: {own} and {incoming}"
        );
    }

    /// A chat shorter than the view sits at its bottom, by the composer, as
    /// on WhatsApp (#378), and stays there when a message arrives.
    #[test]
    fn a_short_chat_sits_at_the_bottom_of_the_view() {
        let mut app = app();
        short_chat_sample(&mut app);
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        render(&mut app, &ctx);
        let chat = SAMPLES[0].id;
        let rect_of = |app: &App, ctx: &egui::Context, index: usize| {
            let message = &app.conversations[chat].messages[index];
            let id = crate::ui::conversation::bubble_id(chat, &message.id).with("rect");
            ctx.data(|data| data.get_temp::<egui::Rect>(id)).unwrap()
        };
        let first = rect_of(&app, &ctx, 0);
        let last = rect_of(&app, &ctx, 1);
        // The window is 780 points tall; the composer takes its foot.
        assert!(first.top() > 300.0, "the messages moved down: {first:?}");
        assert!(
            last.bottom() > 600.0,
            "the last one is by the composer: {last:?}"
        );

        let mut arrival = app.conversations[chat].messages[1].clone();
        arrival.id = "short-arrival".into();
        arrival.timestamp += 1;
        app.conversations
            .get_mut(chat)
            .unwrap()
            .messages
            .push(arrival);
        render(&mut app, &ctx);
        let newest = rect_of(&app, &ctx, 2);
        assert!(
            (newest.bottom() - last.bottom()).abs() < 2.0,
            "the newest message takes the bottom: {newest:?} {last:?}"
        );
        assert!(
            rect_of(&app, &ctx, 0).top() < first.top(),
            "older ones move up"
        );
    }

    #[test]
    fn carousel_cards_scroll_without_widening_the_chat_and_copy_locally() {
        for (width, cards) in [(640.0, 3), (1180.0, 3), (1180.0, 2), (1180.0, 1)] {
            let mut app = app();
            carousel_sample(&mut app, cards);
            app.backend.record_demo_commands();
            let ctx = egui::Context::default();
            app.attach(&ctx);
            let run = |app: &mut App, events| {
                let mut out = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(width, 1100.0),
                        )),
                        events,
                        ..Default::default()
                    },
                    |ui| {
                        app.background_frame(&ctx);
                        app.frame_ui(ui);
                    },
                );
                out.textures_delta.clear();
                out
            };
            for _ in 0..4 {
                run(&mut app, vec![]);
            }
            let bubble = crate::ui::conversation::bubble_id(SAMPLES[0].id, "interactive-card");
            let bounds = ctx
                .data(|data| data.get_temp::<egui::Rect>(bubble.with("rect")))
                .unwrap();
            assert!(bounds.right() <= width + 1.0, "{bounds:?}");
            let first = ctx
                .data(|data| data.get_temp::<egui::Rect>(bubble.with(("carousel-card", 0usize))))
                .unwrap();
            if cards > 1 {
                let second = ctx
                    .data(|data| {
                        data.get_temp::<egui::Rect>(bubble.with(("carousel-card", 1usize)))
                    })
                    .unwrap();
                assert!(second.left() >= first.right());
            }
            if cards < 3 {
                for direction in [-1, 1] {
                    assert!(
                        ctx.data(|data| data
                            .get_temp::<egui::Rect>(bubble.with(("carousel-arrow", direction))))
                            .is_none(),
                        "fitting cards need no navigation arrows"
                    );
                }
                let last = ctx
                    .data(|data| {
                        data.get_temp::<egui::Rect>(bubble.with(("carousel-card", cards - 1)))
                    })
                    .unwrap();
                let output = run(&mut app, vec![]);
                let stamp =
                    crate::util::clock(app.conversations[SAMPLES[0].id].messages[0].timestamp);
                let time = output
                    .shapes
                    .iter()
                    .find_map(|shape| {
                        let egui::Shape::Text(text) = &shape.shape else {
                            return None;
                        };
                        (text.galley.text() == stamp
                            && bounds.contains(text.pos)
                            && text.pos.y >= last.bottom())
                        .then(|| egui::Rect::from_min_size(text.pos, text.galley.size()))
                    })
                    .expect("timestamp below the cards");
                assert!(
                    (time.right() - last.right()).abs() < 1.0,
                    "timestamp {time:?} must follow the last card {last:?}"
                );
            }
            let id = crate::ui::conversation::bubble_id(SAMPLES[0].id, "interactive-card-card-0")
                .with(("interactive-action", 0usize));
            ctx.memory_mut(|memory| memory.request_focus(id));
            let output = run(&mut app, vec![key(egui::Key::Enter, egui::Modifiers::NONE)]);
            assert!(
                output
                    .platform_output
                    .commands
                    .iter()
                    .any(|c| matches!(c, egui::OutputCommand::CopyText(text) if text == "DRAW20"))
            );
            assert!(!app.backend.take_demo_commands().iter().any(|c| matches!(
                c,
                crate::backend::Command::ReplyInteractive { .. }
                    | crate::backend::Command::SendText { .. }
            )));
        }
    }

    #[test]
    fn carousel_arrows_navigate_without_activating_cards_and_preserve_wheel_scrolling() {
        let mut app = app();
        carousel_sample(&mut app, 3);
        // Put action rows beneath the arrows to catch clicks reaching a card.
        if let Content::Interactive {
            card: Some(card), ..
        } = &mut app.conversations.get_mut(SAMPLES[0].id).unwrap().messages[0].content
        {
            for child in &mut card.carousel {
                child.image = None;
            }
        }
        app.backend.record_demo_commands();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        let time = std::cell::Cell::new(0.0);
        let run = |app: &mut App, events| {
            time.set(time.get() + 1.0 / 60.0);
            let mut out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(640.0, 780.0),
                    )),
                    time: Some(time.get()),
                    events,
                    ..Default::default()
                },
                |ui| {
                    app.background_frame(&ctx);
                    app.frame_ui(ui);
                },
            );
            out.textures_delta.clear();
            assert!(
                out.platform_output.commands.iter().all(|command| !matches!(
                    command,
                    egui::OutputCommand::OpenUrl(_) | egui::OutputCommand::CopyText(_)
                )),
                "navigation must not activate a card action"
            );
        };
        let settle = |app: &mut App| {
            for _ in 0..40 {
                run(app, vec![]);
            }
        };
        settle(&mut app);
        let bubble = crate::ui::conversation::bubble_id(SAMPLES[0].id, "interactive-card");
        let arrow = |direction| {
            ctx.data(|data| data.get_temp::<egui::Rect>(bubble.with(("carousel-arrow", direction))))
        };
        let first_card = || {
            ctx.data(|data| data.get_temp::<egui::Rect>(bubble.with(("carousel-card", 0usize))))
                .unwrap()
        };
        let start = first_card().left();
        assert!(arrow(-1).is_none());
        assert!(arrow(1).is_some());
        let viewport = ctx
            .data(|data| data.get_temp::<egui::Rect>(bubble.with("carousel-viewport")))
            .unwrap();
        let top = first_card().top();
        // A vertical mouse wheel with Shift is mapped by egui to horizontal
        // motion; it must survive the app's trackpad axis-lock handling.
        for unit in [egui::MouseWheelUnit::Line, egui::MouseWheelUnit::Point] {
            for direction in [-1.0, 1.0] {
                let delta = direction
                    * if unit == egui::MouseWheelUnit::Line {
                        3.0
                    } else {
                        360.0
                    };
                run(
                    &mut app,
                    vec![
                        egui::Event::PointerMoved(viewport.center()),
                        egui::Event::MouseWheel {
                            unit,
                            delta: egui::vec2(0.0, delta),
                            modifiers: egui::Modifiers::SHIFT,
                            phase: egui::TouchPhase::Move,
                        },
                    ],
                );
                settle(&mut app);
                assert!(
                    (first_card().top() - top).abs() < 1.0,
                    "Shift-wheel must not move the chat vertically"
                );
                if delta < 0.0 {
                    assert!(
                        first_card().left() < start - 10.0,
                        "Shift-wheel advances cards"
                    );
                } else {
                    assert!(
                        (first_card().left() - start).abs() < 1.0,
                        "Shift-wheel goes back"
                    );
                }
            }
        }
        for end in [false, true] {
            let rect = arrow(1).expect("next card");
            let click = |pressed| egui::Event::PointerButton {
                pos: rect.center(),
                pressed,
                button: egui::PointerButton::Primary,
                modifiers: egui::Modifiers::NONE,
            };
            run(
                &mut app,
                vec![egui::Event::PointerMoved(rect.center()), click(true)],
            );
            run(&mut app, vec![click(false)]);
            settle(&mut app);
            assert!(first_card().left() < start - 10.0);
            assert!(arrow(-1).is_some());
            assert_eq!(arrow(1).is_none(), end);
        }
        let end = first_card().left();
        ctx.memory_mut(|memory| memory.request_focus(bubble.with(("carousel-arrow", -1))));
        run(&mut app, vec![key(egui::Key::Enter, egui::Modifiers::NONE)]);
        settle(&mut app);
        assert!(first_card().left() > end + 10.0, "keyboard goes back");
        assert!(arrow(1).is_some());
        run(
            &mut app,
            vec![
                egui::Event::PointerMoved(viewport.center()),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(1000.0, 0.0),
                    modifiers: egui::Modifiers::NONE,
                    phase: egui::TouchPhase::Move,
                },
            ],
        );
        settle(&mut app);
        assert!(
            (first_card().left() - start).abs() < 1.0,
            "wheel returns to the first card"
        );
        assert!(arrow(-1).is_none());
        assert!(arrow(1).is_some());
        assert!(
            app.reply_to.is_none(),
            "arrows do not trigger a message reply"
        );
        assert!(
            !app.backend
                .take_demo_commands()
                .iter()
                .any(|command| matches!(
                    command,
                    crate::backend::Command::ReplyInteractive { .. }
                        | crate::backend::Command::SendText { .. }
                ))
        );
    }

    #[test]
    fn issue_126_arabic_word_order_in_both_quotes_and_composer() {
        fn collect(shape: &egui::Shape, galleys: &mut Vec<std::sync::Arc<egui::Galley>>) {
            match shape {
                egui::Shape::Text(text) if text.galley.text() == "مساء الخير" => {
                    galleys.push(text.galley.clone());
                }
                egui::Shape::Vec(shapes) => {
                    for shape in shapes {
                        collect(shape, galleys);
                    }
                }
                _ => {}
            }
        }
        let mut app = app();
        apply_flags(&mut app, Some("arabic-reply"));
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        let shapes = frame_sized(&mut app, &ctx, 780.0, Vec::new());
        let mut galleys = Vec::new();
        for shape in shapes {
            collect(&shape.shape, &mut galleys);
        }
        assert_eq!(
            galleys.len(),
            4,
            "original, incoming quote, outgoing quote, composer preview"
        );
        let mut reversed = Vec::new();
        for (index, galley) in galleys.into_iter().enumerate() {
            let x = |letter| {
                galley
                    .rows
                    .iter()
                    .flat_map(|row| row.glyphs.iter())
                    .find(|glyph| glyph.chr == letter)
                    .expect("Arabic glyph")
                    .pos
                    .x
            };
            if x('م') <= x('خ') {
                reversed.push((index, x('م'), x('خ')));
            }
            assert_eq!(galley.text(), "مساء الخير", "copy retains logical order");
            let mut repeated = (*galley).clone();
            crate::bidi::reorder_rtl_runs(&mut repeated);
            assert_eq!(
                repeated, *galley,
                "a second layout correction must not reverse the words again"
            );
        }
        assert!(
            reversed.is_empty(),
            "مساء must be right of الخير; reversed instances (index, م x, خ x): {reversed:?}"
        );
    }

    /// Issue #184: "٤٥" drew as "٥٤". Every number on the `rtl` page, in the
    /// bubbles, the quote, the chat list preview, and the composer, must read
    /// left to right, alone or inside right-to-left text.
    #[test]
    fn issue_184_numbers_read_left_to_right_everywhere() {
        fn collect(shape: &egui::Shape, galleys: &mut Vec<std::sync::Arc<egui::Galley>>) {
            match shape {
                egui::Shape::Text(text) => galleys.push(text.galley.clone()),
                egui::Shape::Vec(shapes) => {
                    for shape in shapes {
                        collect(shape, galleys);
                    }
                }
                _ => {}
            }
        }
        let mut app = app();
        apply_flags(&mut app, Some("rtl"));
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        let shapes = frame_sized(&mut app, &ctx, 780.0, Vec::new());
        let mut galleys = Vec::new();
        for shape in shapes {
            collect(&shape.shape, &mut galleys);
        }
        let count = |text: &str| galleys.iter().filter(|g| g.text() == text).count();
        assert!(
            count("٤٥") >= 3,
            "bubble, quote, and chat list preview of \"٤٥\""
        );
        assert_eq!(count("١٢:٣٠"), 1, "composer");
        assert_eq!(
            galleys
                .iter()
                .filter(|g| g.text().replace(crate::emoji::PLACEHOLDER, "") == RTL_NUMBERS)
                .count(),
            1,
            "bubble with numbers inside Arabic"
        );
        let mut reversed = Vec::new();
        for galley in &galleys {
            for placed in &galley.rows {
                // A row that was never reordered keeps its glyphs in shaped
                // order, so compare them in logical (cluster) order.
                let mut logical: Vec<_> = placed.row.glyphs.iter().collect();
                logical.sort_by_key(|glyph| glyph.cluster);
                for pair in logical.windows(2) {
                    let [left, right] = pair else { continue };
                    if left.chr.is_numeric() && right.chr.is_numeric() && left.pos.x >= right.pos.x
                    {
                        reversed.push((galley.text().to_owned(), left.chr, right.chr));
                    }
                }
            }
        }
        assert!(reversed.is_empty(), "reversed digits: {reversed:?}");
    }

    #[test]
    fn the_sample_has_every_kind_of_row() {
        let app = app();
        assert!(app.chats.len() >= 5);
        assert!(app.chats.iter().any(|chat| chat.is_group()));
        assert!(app.chats.iter().any(|chat| chat.is_channel()));
        assert!(app.chats.iter().any(|chat| chat.archived));
        assert!(app.chats.iter().any(|chat| chat.pinned));
        let ada = app.conversations.get(sample_ids()[0]).expect("first chat");
        assert!(
            ada.messages
                .iter()
                .any(|m| matches!(m.content, Content::Image { .. }))
        );
        assert!(
            ada.messages
                .iter()
                .any(|m| matches!(m.content, Content::Revoked))
        );
        assert!(ada.messages.iter().any(|m| m.quoted.is_some()));
    }

    #[test]
    fn demo_assets_stay_in_the_demo_directories() {
        let mut app = app();
        let avatar = app.avatar(sample_ids()[0]).expect("sample avatar");
        assert!(avatar.starts_with(app.dirs.avatar_cache_dir()));
        assert!(avatar.is_file());
        apply_flags(&mut app, Some("voice"));
        assert!(app.dirs.media_cache_dir().join("demo-voice.ogg").is_file());
    }

    #[test]
    fn custom_controls_and_messages_expose_accessible_labels() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        crate::theme::install(&ctx);
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            let palette = crate::theme::Palette::dark();
            crate::theme::icon_button(
                ui,
                crate::theme::Icon::Send,
                20.0,
                palette.text,
                palette.accent,
                "Send message",
            );
            crate::ui::widgets::rich_text(
                ui,
                "Fixture hello 🙂",
                crate::theme::regular(14.0),
                palette.text,
            );
            let text = crate::markup::layout(
                ui,
                "*Fixture body* 🙂",
                &[],
                &crate::markup::Style {
                    size: 14.0,
                    color: palette.text,
                    secondary: palette.secondary,
                    link: palette.accent,
                    mention: palette.accent,
                },
                300.0,
            );
            let (rect, response) =
                ui.allocate_exact_size(text.galley.size(), egui::Sense::click_and_drag());
            crate::markup::paint_selectable(ui, &text, &response, rect.min, palette.text, true);
        });
        output.textures_delta.clear();
        let tree = output
            .platform_output
            .accesskit_update
            .expect("accessibility tree");
        let labels: Vec<_> = tree
            .nodes
            .iter()
            .filter_map(|(_, node)| node.label().or_else(|| node.value()))
            .collect();
        for expected in ["Send message", "Fixture hello 🙂", "Fixture body 🙂"] {
            assert!(
                labels.contains(&expected),
                "missing accessible label: {expected}"
            );
        }
    }

    #[test]
    fn locked_folder_chats_accept_messages() {
        let mut app = app();
        apply_flags(&mut app, Some("locked-open"));
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        assert!(
            app.search.is_empty(),
            "unlocking must not expose the code in search"
        );
        ctx.enable_accesskit();
        assert!(app.current_chat().unwrap().can_send());
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1180.0, 780.0),
                )),
                ..Default::default()
            },
            |ui| app.frame_ui(ui),
        );
        output.textures_delta.clear();
        let tree = output.platform_output.accesskit_update.unwrap();
        let labels: Vec<_> = tree
            .nodes
            .iter()
            .filter_map(|(_, node)| node.label().or_else(|| node.value()))
            .collect();
        assert!(
            !labels.contains(&"Locked chats are read-only in ZapFast"),
            "{labels:?}"
        );
        assert!(!labels.contains(&"admins"));
    }

    /// Every label and value in the accessibility tree of one frame.
    fn accessible_labels(app: &mut App, ctx: &egui::Context) -> Vec<String> {
        render(app, ctx);
        ctx.enable_accesskit();
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1180.0, 780.0),
                )),
                ..Default::default()
            },
            |ui| app.frame_ui(ui),
        );
        output.textures_delta.clear();
        output
            .platform_output
            .accesskit_update
            .expect("accessibility tree")
            .nodes
            .iter()
            .filter_map(|(_, node)| node.label().or_else(|| node.value()))
            .map(str::to_owned)
            .collect()
    }

    /// With our read receipts off, WhatsApp shows no read ticks in a
    /// one-to-one chat (#458): the label reads the ticks the bubble shows,
    /// delivered, not read.
    #[test]
    fn an_own_read_message_reads_delivered_while_receipts_are_off() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut app = app();
        app.attach(&ctx);
        let chat = app.open_chat.clone().unwrap();
        let last = app
            .conversations
            .get_mut(&chat)
            .unwrap()
            .messages
            .last_mut()
            .unwrap();
        last.from_me = true;
        last.status = crate::model::Delivery::Read;
        last.content = crate::model::Content::text("Receipts fixture");
        render(&mut app, &ctx);
        let label = |app: &mut App| {
            accessible_nodes(app, &ctx, Vec::new())
                .into_iter()
                .map(|(text, _, _)| text)
                .find(|text| text.starts_with("You, Receipts fixture"))
                .expect("the message's label")
        };
        let read = label(&mut app);
        assert!(read.contains(", read"), "{read}");
        app.account_receipts_off = true;
        let delivered = label(&mut app);
        assert!(delivered.contains(", delivered"), "{delivered}");
        assert!(!delivered.contains(", read"), "{delivered}");
    }

    #[test]
    fn every_message_row_reads_as_one_labelled_node() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        ctx.enable_accesskit();
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1180.0, 780.0),
                )),
                ..Default::default()
            },
            |ui| app.frame_ui(ui),
        );
        output.textures_delta.clear();
        let tree = output
            .platform_output
            .accesskit_update
            .expect("accessibility tree");
        let chat = app.current_chat().unwrap().clone();
        let last = app.conversations[&chat.id].messages.last().unwrap().clone();
        let time = crate::util::clock(last.timestamp);
        // The message list's items only: the chat list's row for the chat
        // reads the same message and time.
        let node = |id: egui::accesskit::NodeId| {
            tree.nodes
                .iter()
                .find(|(found, _)| *found == id)
                .map(|(_, node)| node)
        };
        let list = node(crate::ui::conversation::messages_list_id(&chat.id).accesskit_id())
            .expect("the message list");
        let labels: Vec<&str> = list
            .children()
            .iter()
            .filter_map(|id| node(*id)?.label())
            .collect();
        let row = labels
            .iter()
            .find(|label| label.contains(&time) && label.contains(&last.summary()))
            .unwrap_or_else(|| {
                panic!("no row label with the last message's time and text: {labels:?}")
            });
        assert!(!last.from_me || row.starts_with("You, "), "{row}");
    }

    /// An album is one row (#286), and so one item: its label counts the
    /// pictures, reads the caption and its last member's footer, star and
    /// pin marks included, after the notice of the pin drawn above it.
    #[test]
    fn an_album_reads_as_one_row_with_its_notices_and_marks() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut app = app();
        app.attach(&ctx);
        let chat = app.open_chat.clone().unwrap();
        let messages = &mut app.conversations.get_mut(&chat).unwrap().messages;
        let count = messages.len();
        let at = messages[count - 4].timestamp + 60;
        for (index, message) in messages[count - 3..].iter_mut().enumerate() {
            message.from_me = false;
            message.sender = "album@s.whatsapp.net".into();
            message.timestamp = at;
            message.reactions.clear();
            message.quoted = None;
            message.edited = false;
            message.forwarded = false;
            message.content = crate::model::Content::Image {
                motion: None,
                caption: (index == 0).then(|| "Album fixture caption".to_owned()),
                media: crate::model::Media {
                    mime: "image/jpeg".into(),
                    size: 1,
                    width: Some(640),
                    height: Some(480),
                    album: Some("album-fixture".into()),
                    path: None,
                    state: crate::model::MediaState::Idle,
                },
            };
        }
        let last = messages[count - 1].id.clone();
        app.stars
            .entry(chat.clone())
            .or_default()
            .insert(last.clone());
        app.pins
            .entry(chat.clone())
            .or_default()
            .insert(last.clone());
        app.chat_pins.insert(
            chat.clone(),
            vec![crate::archive::Pinned {
                chat: chat.clone(),
                id: last,
                pinned_at: at - 30,
                expires_at: at + 7 * 24 * 60 * 60,
                text: String::new(),
                from_me: false,
                sent_at: at,
                pinner: crate::archive::Pinner {
                    by_me: true,
                    sender: String::new(),
                },
            }],
        );
        render(&mut app, &ctx);
        let labels: Vec<String> = accessible_nodes(&mut app, &ctx, Vec::new())
            .into_iter()
            .map(|(text, _, _)| text)
            .filter(|text| text.contains(", 3 photos, "))
            .collect();
        let [row] = labels.as_slice() else {
            panic!("the album is not one labelled row: {labels:?}");
        };
        assert!(row.contains("You pinned a message, "), "{row}");
        assert!(row.contains("3 photos, Album fixture caption, "), "{row}");
        assert!(row.contains(", starred, pinned"), "{row}");
    }

    /// A reply reads the quoted sender's name as its quote block draws it:
    /// "You" for our own message, a saved contact name before the name the
    /// quote carries.
    #[test]
    fn a_reply_reads_the_quoted_sender_as_its_quote_draws_it() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut app = app();
        app.attach(&ctx);
        render(&mut app, &ctx);
        let chat = app.open_chat.clone().unwrap();
        let reply_of = |app: &mut App| -> String {
            accessible_nodes(app, &ctx, Vec::new())
                .into_iter()
                .map(|(text, _, _)| text)
                .find(|text| text.starts_with("You, Listened, agreed on all three points., "))
                .expect("the reply's label")
        };
        fn quoted<'a>(app: &'a mut App, chat: &str) -> &'a mut crate::model::Quoted {
            app.conversations
                .get_mut(chat)
                .unwrap()
                .messages
                .iter_mut()
                .find(|message| message.id == "ada-reply")
                .unwrap()
                .quoted
                .as_mut()
                .unwrap()
        }
        let ada = quoted(&mut app, &chat).sender.clone();
        app.contacts.insert(
            ada.clone(),
            Contact {
                id: ada,
                full_name: Some("Ada King".to_owned()),
                first_name: None,
                push_name: None,
            },
        );
        let label = reply_of(&mut app);
        assert!(label.contains(", replying to Ada King: "), "{label}");
        let quote = quoted(&mut app, &chat);
        quote.sender = ME.to_owned();
        quote.sender_name = Some("Abdulaziz".to_owned());
        let label = reply_of(&mut app);
        assert!(label.contains(", replying to You: "), "{label}");
    }

    /// A reply reads the quoted summary in the interface language, as its
    /// quote block draws it (#527): "Voice message" in a German interface
    /// reads "Sprachnachricht".
    #[test]
    fn a_reply_reads_the_quoted_summary_in_the_interface_language() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut app = app();
        app.locale = crate::i18n::Locale::German;
        app.attach(&ctx);
        render(&mut app, &ctx);
        let chat = app.open_chat.clone().unwrap();
        let quoted = app.conversations[&chat]
            .message("ada-reply")
            .and_then(|message| message.quoted.clone())
            .expect("the demo's reply");
        let english = quoted.summary.clone();
        let german = quoted.summary_in(app.locale).into_owned();
        assert_ne!(german, english, "the catalog translates the label");
        let label = accessible_nodes(&mut app, &ctx, Vec::new())
            .into_iter()
            .map(|(text, _, _)| text)
            .find(|text| {
                text.contains("Listened, agreed on all three points.")
                    && text.contains(", replying to ")
            })
            .expect("the reply's label");
        assert!(label.contains(&format!(": {german}")), "{label}");
        assert!(!label.contains(&english), "{label}");
    }

    /// The hover-only React and Reply controls are pointer conveniences: the
    /// message menu and its keys reach the same actions, so they stay out of
    /// Tab order and out of the accessibility tree.
    #[test]
    fn hover_reaction_controls_are_not_focusable_or_exposed() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        let chat = app.open_chat.clone().expect("demo opens a chat");
        let message = app.conversations[&chat].messages.last().unwrap().id.clone();
        let bubble = crate::ui::conversation::bubble_id(&chat, &message);
        let controls = [bubble.with("react"), bubble.with("reply")];
        let labels = accessible_labels(&mut app, &ctx);
        assert!(
            !labels
                .iter()
                .any(|label| label.starts_with("React to") || label == "Reply"),
            "hover controls exposed: {labels:?}"
        );
        // No node at all, labelled or not, may exist for the controls' ids.
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1180.0, 780.0),
                )),
                ..Default::default()
            },
            |ui| app.frame_ui(ui),
        );
        output.textures_delta.clear();
        let tree = output
            .platform_output
            .accesskit_update
            .expect("accessibility tree");
        for control in controls {
            assert!(
                !tree.nodes.iter().any(|(id, _)| id.0 == control.value()),
                "node exposed for hover control {control:?}"
            );
        }
        for control in controls {
            ctx.memory_mut(|memory| memory.request_focus(control));
            render(&mut app, &ctx);
            assert_ne!(ctx.memory(|memory| memory.focused()), Some(control));
        }
    }

    fn focus_messages(app: &mut App, ctx: &egui::Context) {
        let chat = app.open_chat.clone().unwrap();
        ctx.memory_mut(|memory| {
            memory.request_focus(crate::ui::conversation::messages_list_id(&chat))
        });
        render(app, ctx);
    }

    /// Alt+2, or Control+2 on macOS.
    fn messages_shortcut() -> egui::Modifiers {
        if cfg!(target_os = "macos") {
            egui::Modifiers::CTRL
        } else {
            egui::Modifiers::ALT
        }
    }

    #[test]
    fn message_list_is_a_labelled_listbox_with_an_active_item() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        focus_messages(&mut app, &ctx);
        ctx.enable_accesskit();
        let mut output = ctx.run_ui(Default::default(), |ui| app.frame_ui(ui));
        output.textures_delta.clear();
        let tree = output.platform_output.accesskit_update.unwrap();
        let chat = app.open_chat.clone().unwrap();
        let list_id = crate::ui::conversation::messages_list_id(&chat).accesskit_id();
        let list = tree
            .nodes
            .iter()
            .find(|(id, _)| *id == list_id)
            .unwrap()
            .1
            .clone();
        assert_eq!(list.role(), egui::accesskit::Role::ListBox);
        assert_eq!(list.label(), Some("Messages with Ada Lovelace"));
        let total = app.conversations[&chat].messages.len();
        assert_eq!(list.size_of_set(), Some(total));
        let active = list.active_descendant().expect("an active message");
        let item = tree
            .nodes
            .iter()
            .find(|(id, _)| *id == active)
            .unwrap()
            .1
            .clone();
        assert_eq!(item.role(), egui::accesskit::Role::ListBoxOption);
        assert_eq!(
            item.position_in_set(),
            Some(total - 1),
            "starts on the newest"
        );
        assert_eq!(item.size_of_set(), Some(total));
        assert_eq!(item.is_selected(), None, "messages never carry selected");
        assert!(
            list.children().contains(&active),
            "rows are the list's items"
        );
        assert_eq!(tree.focus, list_id);
    }

    #[test]
    fn a_group_message_list_is_named_after_the_group() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        app.open_chat = Some(SAMPLES[1].id.into());
        render(&mut app, &ctx);
        focus_messages(&mut app, &ctx);
        ctx.enable_accesskit();
        let mut output = ctx.run_ui(Default::default(), |ui| app.frame_ui(ui));
        output.textures_delta.clear();
        let tree = output.platform_output.accesskit_update.unwrap();
        let list_id = crate::ui::conversation::messages_list_id(SAMPLES[1].id).accesskit_id();
        let list = &tree.nodes.iter().find(|(id, _)| *id == list_id).unwrap().1;
        assert_eq!(list.label(), Some("Messages in Rust Berlin"));
    }

    /// A message whose file is not simply there ends its label with why: not
    /// downloaded yet, downloading, or the error the bubble shows.
    #[test]
    fn a_message_label_ends_with_its_attachment_state() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut app = app();
        app.attach(&ctx);
        render(&mut app, &ctx);
        let chat = app.open_chat.clone().unwrap();
        let limit = "This attachment is larger than the 64 MiB download limit";
        // Auto-download is off in the demo, so the document waits for a
        // click and reads as not downloaded.
        let label_of = |app: &mut App, start: &str| -> String {
            let nodes = accessible_nodes(app, &ctx, Vec::new());
            nodes
                .into_iter()
                .map(|(text, _, _)| text)
                .find(|text| text.starts_with(start))
                .unwrap_or_else(|| panic!("no message label starting {start}"))
        };
        let document = "You, Document, Notes on the Engine.pdf, ";
        assert!(
            label_of(&mut app, document).ends_with(", not downloaded"),
            "{}",
            label_of(&mut app, document)
        );
        for (state, ending) in [
            (crate::model::MediaState::Downloading, ", downloading"),
            (crate::model::MediaState::Failed(limit.into()), limit),
        ] {
            let message = app
                .conversations
                .get_mut(&chat)
                .unwrap()
                .messages
                .iter_mut()
                .find(|message| message.id == "ada-doc")
                .unwrap();
            message.content.media_mut().unwrap().state = state;
            let label = label_of(&mut app, document);
            assert!(label.ends_with(ending), "{label}");
            assert!(!label.contains("Click to retry"), "{label}");
        }
        let message = app
            .conversations
            .get_mut(&chat)
            .unwrap()
            .messages
            .iter_mut()
            .find(|message| message.id == "ada-doc")
            .unwrap();
        message.content.media_mut().unwrap().path = Some("notes.pdf".into());
        message.content.media_mut().unwrap().state = Default::default();
        let label = label_of(&mut app, document);
        assert!(!label.contains("download"), "{label}");
    }

    #[test]
    fn arrows_home_and_end_move_the_active_message() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        focus_messages(&mut app, &ctx);
        let chat = app.open_chat.clone().unwrap();
        let ids: Vec<String> = app.conversations[&chat]
            .messages
            .iter()
            .map(|m| m.id.clone())
            .collect();
        for (key, expected) in [
            (egui::Key::ArrowUp, ids[ids.len() - 2].clone()),
            (egui::Key::Home, ids[0].clone()),
            (egui::Key::ArrowDown, ids[1].clone()),
            (egui::Key::End, ids[ids.len() - 1].clone()),
        ] {
            press_key(&mut app, &ctx, key, egui::Modifiers::NONE);
            assert_eq!(app.message_cursor.get(&chat), Some(&expected), "{key:?}");
            assert_eq!(
                ctx.memory(|memory| memory.focused()),
                Some(crate::ui::conversation::messages_list_id(&chat)),
                "focus stays on the list"
            );
        }
    }

    /// An album is drawn as one row (#286), so it is one item of the
    /// list: the arrows step over its later pictures, which have no row and
    /// no node, and the list counts it once.
    #[test]
    fn the_arrows_step_over_an_album_as_one_message() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        let chat = app.open_chat.clone().unwrap();
        let messages = &mut app.conversations.get_mut(&chat).unwrap().messages;
        let count = messages.len();
        let at = messages[count - 4].timestamp + 60;
        for message in &mut messages[count - 3..] {
            message.from_me = false;
            message.sender = "album@s.whatsapp.net".into();
            message.timestamp = at;
            message.content = crate::model::Content::Image {
                motion: None,
                caption: None,
                media: crate::model::Media {
                    mime: "image/jpeg".into(),
                    size: 1,
                    width: Some(640),
                    height: Some(480),
                    album: Some("album-fixture".into()),
                    path: None,
                    state: crate::model::MediaState::Idle,
                },
            };
        }
        let ids: Vec<String> = messages.iter().map(|m| m.id.clone()).collect();
        render(&mut app, &ctx);
        focus_messages(&mut app, &ctx);
        // The newest item is the album, which its first picture draws.
        press_key(&mut app, &ctx, egui::Key::End, egui::Modifiers::NONE);
        assert_eq!(app.message_cursor.get(&chat), Some(&ids[count - 3]));
        press_key(&mut app, &ctx, egui::Key::ArrowUp, egui::Modifiers::NONE);
        assert_eq!(app.message_cursor.get(&chat), Some(&ids[count - 4]));
        press_key(&mut app, &ctx, egui::Key::ArrowDown, egui::Modifiers::NONE);
        assert_eq!(app.message_cursor.get(&chat), Some(&ids[count - 3]));
        press_key(&mut app, &ctx, egui::Key::ArrowDown, egui::Modifiers::NONE);
        assert_eq!(
            app.message_cursor.get(&chat),
            Some(&ids[count - 3]),
            "the album is the last item"
        );
        // The list counts rows, and its active item is the album's.
        ctx.enable_accesskit();
        let mut output = ctx.run_ui(Default::default(), |ui| app.frame_ui(ui));
        output.textures_delta.clear();
        let tree = output.platform_output.accesskit_update.unwrap();
        let node = |id: egui::accesskit::NodeId| {
            tree.nodes
                .iter()
                .find(|(node, _)| *node == id)
                .map(|(_, node)| node.clone())
                .unwrap()
        };
        let list = node(crate::ui::conversation::messages_list_id(&chat).accesskit_id());
        assert_eq!(list.size_of_set(), Some(count - 2));
        let active = node(list.active_descendant().expect("an active message"));
        assert_eq!(active.position_in_set(), Some(count - 3));
        assert!(
            active
                .label()
                .is_some_and(|label| label.contains("3 photos")),
            "{:?}",
            active.label()
        );
        // A cursor left on a later picture, as a jump to it leaves it,
        // stands on the album's row.
        app.message_cursor
            .insert(chat.clone(), ids[count - 1].clone());
        press_key(&mut app, &ctx, egui::Key::ArrowUp, egui::Modifiers::NONE);
        assert_eq!(app.message_cursor.get(&chat), Some(&ids[count - 4]));
    }

    fn long_chat(app: &mut App, count: usize) -> String {
        let chat = app.open_chat.clone().unwrap();
        let conversation = app.conversations.get_mut(&chat).unwrap();
        let template = conversation.messages.last().unwrap().clone();
        conversation.messages = (0..count)
            .map(|index| crate::model::Message {
                id: format!("long-{index}"),
                timestamp: template.timestamp - (count - index) as i64 * 60,
                content: crate::model::Content::text(format!("Fixture line {index}")),
                ..template.clone()
            })
            .collect();
        chat
    }

    fn tree_after(
        app: &mut App,
        ctx: &egui::Context,
        key: Option<egui::Key>,
    ) -> egui::accesskit::TreeUpdate {
        if let Some(key) = key {
            press_key(app, ctx, key, egui::Modifiers::NONE);
        }
        ctx.enable_accesskit();
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1180.0, 780.0),
                )),
                ..Default::default()
            },
            |ui| app.frame_ui(ui),
        );
        output.textures_delta.clear();
        output
            .platform_output
            .accesskit_update
            .expect("accessibility tree")
    }

    /// The message list's node hangs from the conversation around it, never
    /// from the window, and always as a list. Its name used to be worked out
    /// before the list was laid out, which made its node under the window.
    #[test]
    fn the_message_list_node_hangs_from_the_conversation() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        ctx.enable_accesskit();
        let root = egui::accesskit_root_id().accesskit_id();
        let first = app.open_chat.clone().unwrap();
        let second = app.visible_chats()[1].id.clone();
        let mut lists_seen = 0;
        // From the first frame, whose passes lay the list out hidden once,
        // then on to another chat.
        for frame in 0..8 {
            let events = if frame == 4 {
                vec![egui::Event::Key {
                    key: egui::Key::Num2,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::COMMAND,
                }]
            } else {
                Vec::new()
            };
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1180.0, 780.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    let ctx = ui.ctx().clone();
                    app.background_frame(&ctx);
                    app.frame_ui(ui);
                },
            );
            output.textures_delta.clear();
            let tree = output
                .platform_output
                .accesskit_update
                .expect("accessibility tree");
            for chat in [&first, &second] {
                let list = crate::ui::conversation::messages_list_id(chat).accesskit_id();
                let Some((_, node)) = tree.nodes.iter().find(|(id, _)| *id == list) else {
                    continue;
                };
                lists_seen += 1;
                assert_eq!(
                    node.role(),
                    egui::accesskit::Role::ListBox,
                    "frame {frame}: no list node without its role"
                );
                let parent = tree
                    .nodes
                    .iter()
                    .find(|(_, node)| node.children().contains(&list))
                    .map(|(id, node)| (*id, node.role()));
                assert!(
                    parent.is_some_and(|(id, _)| id != root),
                    "frame {frame}: the list hangs from {parent:?}, not the window"
                );
            }
        }
        assert_eq!(app.open_chat.as_ref(), Some(&second));
        assert!(lists_seen > 0, "the list was in the tree");
    }

    fn node(
        tree: &egui::accesskit::TreeUpdate,
        id: egui::accesskit::NodeId,
    ) -> Option<egui::accesskit::Node> {
        tree.nodes
            .iter()
            .find(|(found, _)| *found == id)
            .map(|(_, node)| node.clone())
    }

    #[test]
    fn cursor_far_from_view_never_names_a_missing_active_descendant() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        let chat = long_chat(&mut app, 400);
        render(&mut app, &ctx);
        focus_messages(&mut app, &ctx);
        let list_id = crate::ui::conversation::messages_list_id(&chat).accesskit_id();
        for key in [egui::Key::Home, egui::Key::End, egui::Key::Home] {
            let tree = tree_after(&mut app, &ctx, Some(key));
            let list = tree
                .nodes
                .iter()
                .find(|(id, _)| *id == list_id)
                .unwrap()
                .1
                .clone();
            if let Some(active) = list.active_descendant() {
                assert!(
                    tree.nodes.iter().any(|(id, _)| *id == active),
                    "{key:?}: active descendant {active:?} is not in this update"
                );
            }
        }
        assert_eq!(
            app.message_cursor.get(&chat).map(String::as_str),
            Some("long-0")
        );
    }

    /// Home far above the view still lands: the oldest message is laid out,
    /// scrolled to and becomes the active descendant.
    #[test]
    fn home_far_above_the_view_reaches_the_oldest_message() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        let chat = long_chat(&mut app, 400);
        app.conversations.get_mut(&chat).unwrap().complete = true;
        render(&mut app, &ctx);
        focus_messages(&mut app, &ctx);
        let tree = tree_after(&mut app, &ctx, Some(egui::Key::Home));
        let list_id = crate::ui::conversation::messages_list_id(&chat).accesskit_id();
        let list = tree
            .nodes
            .iter()
            .find(|(id, _)| *id == list_id)
            .unwrap()
            .1
            .clone();
        let active = list.active_descendant().expect("the oldest is active");
        let item = tree
            .nodes
            .iter()
            .find(|(id, _)| *id == active)
            .unwrap()
            .1
            .clone();
        assert_eq!(item.position_in_set(), Some(0));
        assert!(
            item.label()
                .is_some_and(|label| label.contains("Fixture line 0")),
            "{:?}",
            item.label()
        );
    }

    #[test]
    fn cursor_survives_deleted_message() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        let chat = long_chat(&mut app, 10);
        render(&mut app, &ctx);
        app.message_cursor.insert(chat.clone(), "long-3".into());
        app.conversations
            .get_mut(&chat)
            .unwrap()
            .messages
            .retain(|message| message.id != "long-3");
        focus_messages(&mut app, &ctx);
        let tree = tree_after(&mut app, &ctx, None);
        let list_id = crate::ui::conversation::messages_list_id(&chat).accesskit_id();
        let list = tree
            .nodes
            .iter()
            .find(|(id, _)| *id == list_id)
            .unwrap()
            .1
            .clone();
        let active = list
            .active_descendant()
            .expect("falls back to an existing message");
        let item = tree
            .nodes
            .iter()
            .find(|(id, _)| *id == active)
            .unwrap()
            .1
            .clone();
        assert_eq!(item.position_in_set(), Some(8), "newest of the 9 remaining");
    }

    fn bubble_rect(ctx: &egui::Context, chat: &str, message: &str) -> Option<egui::Rect> {
        ctx.data(|data| {
            data.get_temp::<egui::Rect>(
                crate::ui::conversation::bubble_id(chat, message).with("rect"),
            )
        })
    }

    /// Pointer users never see the list: no ring until the keyboard reaches
    /// it, and then a ring around the current message only.
    #[test]
    fn the_list_rings_only_the_current_message_and_only_after_the_keyboard() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        let chat = app.open_chat.clone().unwrap();
        let list = crate::ui::conversation::messages_list_id(&chat);
        // At rest the composer has focus, and its own ring; the list has
        // neither.
        assert_eq!(
            ctx.memory(|memory| memory.focused()),
            Some(egui::Id::new("composer-text"))
        );
        let at_rest = ring(&ctx);
        assert!(
            app.conversations[&chat]
                .messages
                .iter()
                .all(|message| bubble_rect(&ctx, &chat, &message.id) != at_rest),
            "no message is ringed at rest"
        );
        // Focus without keyboard navigation draws nothing.
        ctx.memory_mut(|memory| memory.request_focus(list));
        render(&mut app, &ctx);
        assert_eq!(ctx.memory(|memory| memory.focused()), Some(list));
        assert_eq!(ring(&ctx), None, "no ring without the keyboard");
        press_key(&mut app, &ctx, egui::Key::Num2, messages_shortcut());
        assert_eq!(
            ctx.memory(|memory| memory.focused()),
            Some(crate::ui::conversation::messages_list_id(&chat))
        );
        let newest = app.conversations[&chat].messages.last().unwrap().id.clone();
        let bubble = bubble_rect(&ctx, &chat, &newest).expect("newest bubble drawn");
        assert_eq!(ring(&ctx), Some(bubble), "the ring is the newest bubble's");
        press_key(&mut app, &ctx, egui::Key::ArrowUp, egui::Modifiers::NONE);
        let messages = &app.conversations[&chat].messages;
        let previous = messages[messages.len() - 2].id.clone();
        let bubble = bubble_rect(&ctx, &chat, &previous).expect("previous bubble drawn");
        assert_eq!(ring(&ctx), Some(bubble), "the ring follows the cursor");
    }

    /// A focused list whose current message is not laid out draws no ring
    /// at all, not one around the whole list.
    #[test]
    fn a_focused_list_without_its_current_message_draws_no_ring() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        let chat = long_chat(&mut app, 400);
        render(&mut app, &ctx);
        press_key(&mut app, &ctx, egui::Key::Num2, messages_shortcut());
        assert!(ring(&ctx).is_some(), "rings the newest message");
        // Far above the view, and not revealed.
        app.message_cursor.insert(chat.clone(), "long-0".into());
        render(&mut app, &ctx);
        assert_eq!(
            ctx.memory(|memory| memory.focused()),
            Some(crate::ui::conversation::messages_list_id(&chat))
        );
        assert_eq!(ring(&ctx), None);
    }

    #[test]
    fn the_messages_shortcut_without_an_open_chat_says_so() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        app.open_chat = None;
        render(&mut app, &ctx);
        press_key(&mut app, &ctx, egui::Key::Num2, messages_shortcut());
        assert!(
            app.announcer.spoken().contains(&"No chat open"),
            "{:?}",
            app.announcer.spoken()
        );
    }

    /// The Calls tab keeps the open chat beside its history. Alt+2 moves to
    /// that chat's list there and leaves the history on screen, the list's
    /// keys walk it as on the chats' page, and Escape returns to the chats
    /// with the list still focused. A narrow window showing only the history
    /// goes to the chat itself.
    #[test]
    fn the_message_list_works_beside_the_call_history() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        let chat = app.open_chat.clone().unwrap();
        let list = crate::ui::conversation::messages_list_id(&chat);
        app.page = crate::model::Page::Calls;
        render(&mut app, &ctx);
        press_key(&mut app, &ctx, egui::Key::Num2, messages_shortcut());
        assert_eq!(app.page, crate::model::Page::Calls, "the history stays");
        assert_eq!(ctx.memory(|memory| memory.focused()), Some(list));
        press_key(&mut app, &ctx, egui::Key::ArrowUp, egui::Modifiers::NONE);
        let messages = &app.conversations[&chat].messages;
        let previous = messages[messages.len() - 2].id.clone();
        assert_eq!(app.message_cursor.get(&chat), Some(&previous));
        press_key(&mut app, &ctx, egui::Key::Escape, egui::Modifiers::NONE);
        assert_eq!(app.page, crate::model::Page::Chats);
        assert_eq!(ctx.memory(|memory| memory.focused()), Some(list));

        // Narrow, the Calls tab shows only its history beside a chat that
        // was not picked from it.
        app.page = crate::model::Page::Calls;
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(500.0, 780.0),
                )),
                events: vec![egui::Event::Key {
                    key: egui::Key::Num2,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: messages_shortcut(),
                }],
                ..Default::default()
            },
            |ui| {
                let ctx = ui.ctx().clone();
                app.background_frame(&ctx);
                app.frame_ui(ui);
            },
        );
        output.textures_delta.clear();
        assert_eq!(app.page, crate::model::Page::Chats, "the chat itself");
        assert!(app.focus_messages || ctx.memory(|memory| memory.has_focus(list)));
    }

    /// A quote jump moves the current message with the view, and reveals it
    /// when the list has focus.
    #[test]
    fn a_jump_to_a_message_moves_the_current_message() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        focus_messages(&mut app, &ctx);
        let chat = app.open_chat.clone().unwrap();
        let target = app.conversations[&chat].messages[1].id.clone();
        app.actions
            .push(crate::model::Action::ScrollTo(target.clone()));
        render(&mut app, &ctx);
        assert_eq!(app.message_cursor.get(&chat), Some(&target));
    }

    /// One frame with the window focused or not, as the platform reports it.
    fn frame_with_window_focus(app: &mut App, ctx: &egui::Context, focused: bool) {
        let mut input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1180.0, 780.0),
            )),
            ..Default::default()
        };
        input
            .viewports
            .get_mut(&egui::ViewportId::ROOT)
            .unwrap()
            .focused = Some(focused);
        let mut output = ctx.run_ui(input, |ui| {
            let ctx = ui.ctx().clone();
            app.background_frame(&ctx);
            app.frame_ui(ui);
        });
        output.textures_delta.clear();
    }

    /// Coming back to the window (Alt+Tab, a screen reader dialog) leaves a
    /// reader in the message list where they were, not in the message field.
    #[test]
    fn returning_to_the_window_keeps_focus_in_the_message_list() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        let chat = app.open_chat.clone().unwrap();
        let list = crate::ui::conversation::messages_list_id(&chat);
        press_key(&mut app, &ctx, egui::Key::Num2, messages_shortcut());
        press_key(&mut app, &ctx, egui::Key::ArrowUp, egui::Modifiers::NONE);
        let cursor = app.message_cursor.get(&chat).cloned();
        frame_with_window_focus(&mut app, &ctx, false);
        frame_with_window_focus(&mut app, &ctx, true);
        render(&mut app, &ctx);
        assert_eq!(ctx.memory(|memory| memory.focused()), Some(list));
        assert_eq!(app.message_cursor.get(&chat).cloned(), cursor);
    }

    /// A notification for the open chat, clicked while the reader is in its
    /// message list, moves the list to that message and keeps focus there.
    #[test]
    fn opening_a_message_in_the_open_chat_keeps_focus_in_its_list() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        let chat = app.open_chat.clone().unwrap();
        let list = crate::ui::conversation::messages_list_id(&chat);
        press_key(&mut app, &ctx, egui::Key::Num2, messages_shortcut());
        let target = app.conversations[&chat].messages[2].id.clone();
        app.actions.push(crate::model::Action::OpenMessage {
            chat: chat.clone(),
            message: target.clone(),
        });
        render(&mut app, &ctx);
        render(&mut app, &ctx);
        assert_eq!(ctx.memory(|memory| memory.focused()), Some(list));
        assert_eq!(app.message_cursor.get(&chat), Some(&target));
        assert_eq!(
            ring(&ctx),
            bubble_rect(&ctx, &chat, &target),
            "the ring is on the message opened"
        );
        assert!(ring(&ctx).is_some());
    }

    fn channel(app: &App) -> String {
        let id = SAMPLES
            .iter()
            .find(|sample| sample.id.ends_with("@newsletter"))
            .expect("channel sample")
            .id
            .to_owned();
        assert!(!app.chat(&id).unwrap().can_send(), "a read-only chat");
        id
    }

    /// A long read-only chat opened from the keyboard for the first time
    /// shows its newest messages in the first frame it is drawn, never its
    /// oldest (#357), and its message list has the focus.
    #[test]
    fn a_long_read_only_chat_opened_from_the_keyboard_shows_its_end_at_once() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        press_key(&mut app, &ctx, egui::Key::Num2, messages_shortcut());
        let channel = channel(&app);
        let conversation = app.conversations.get_mut(&channel).unwrap();
        let template = conversation.messages.last().unwrap().clone();
        conversation.messages = (0..60)
            .map(|index| crate::model::Message {
                id: format!("channel-{index}"),
                timestamp: template.timestamp - (60 - index) as i64 * 60,
                content: crate::model::Content::text(format!("Announcement {index}")),
                ..template.clone()
            })
            .collect();
        app.actions
            .push(crate::model::Action::OpenChat(channel.clone()));
        // The first frame that draws any of the channel's messages.
        let mut first = None;
        for _ in 0..4 {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1180.0, 780.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    let ctx = ui.ctx().clone();
                    app.background_frame(&ctx);
                    app.frame_ui(ui);
                },
            );
            output.textures_delta.clear();
            // The text the reader sees: drawn inside its clip rect.
            let shown: Vec<String> = output
                .shapes
                .iter()
                .filter_map(|clipped| match &clipped.shape {
                    egui::Shape::Text(text)
                        if clipped.clip_rect.intersects(text.visual_bounding_rect()) =>
                    {
                        Some(text.galley.text().to_owned())
                    }
                    _ => None,
                })
                .filter(|text| text.starts_with("Announcement "))
                .collect();
            if !shown.is_empty() {
                first = Some(shown);
                break;
            }
        }
        let shown = first.expect("the channel's messages are drawn");
        assert!(
            shown.iter().any(|text| text == "Announcement 59"),
            "the newest message is in view at once: {shown:?}"
        );
        assert!(
            !shown.iter().any(|text| text == "Announcement 0"),
            "the oldest never flashes: {shown:?}"
        );
        render(&mut app, &ctx);
        assert_eq!(
            ctx.memory(|memory| memory.focused()),
            Some(crate::ui::conversation::messages_list_id(&channel))
        );
    }

    /// A chat without a message field gives its message list the focus the
    /// field would have had: switching chats from the list never lands on
    /// nothing.
    #[test]
    fn switching_to_a_read_only_chat_from_the_list_focuses_its_list() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        press_key(&mut app, &ctx, egui::Key::Num2, messages_shortcut());
        let channel = channel(&app);
        app.actions
            .push(crate::model::Action::OpenChat(channel.clone()));
        render(&mut app, &ctx);
        let list = crate::ui::conversation::messages_list_id(&channel);
        assert_eq!(ctx.memory(|memory| memory.focused()), Some(list));
        let tree = tree_after(&mut app, &ctx, None);
        assert_eq!(tree.focus, list.accesskit_id());
        assert!(
            tree.nodes.iter().any(|(id, _)| *id == tree.focus),
            "focus names a node in the tree"
        );
    }

    /// Opens the read-only demo channel, filled long enough to scroll, by
    /// clicking its row in the chat list, after the keyboard had been used
    /// in the message list, as a mixed keyboard and mouse user does.
    fn open_channel_with_the_pointer(app: &mut App, ctx: &egui::Context) -> String {
        let channel = channel(app);
        // Channels are listed under their own filter.
        app.chat_filter = crate::model::ChatFilter::Channels;
        render(app, ctx);
        press_key(app, ctx, egui::Key::Num2, messages_shortcut());
        let conversation = app.conversations.get_mut(&channel).unwrap();
        let template = conversation.messages.last().unwrap().clone();
        conversation.messages = (0..60)
            .map(|index| crate::model::Message {
                id: format!("channel-{index}"),
                timestamp: template.timestamp - (60 - index) as i64 * 60,
                content: crate::model::Content::text(format!("Announcement {index}")),
                ..template.clone()
            })
            .collect();
        let row = ctx
            .data(|data| {
                data.get_temp::<egui::Id>(crate::ui::chats::chat_row_id(&channel).with("widget"))
            })
            .and_then(|id| ctx.read_response(id))
            .expect("the channel's row is drawn");
        let at = row.rect.center();
        let pointer = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame_with(
            app,
            ctx,
            vec![egui::Event::PointerMoved(at), pointer(true), pointer(false)],
        );
        render(app, ctx);
        render(app, ctx);
        assert_eq!(app.open_chat.as_deref(), Some(channel.as_str()));
        channel
    }

    /// Opened with the pointer, a read-only chat opens as it did before the
    /// list existed: at its newest message, pinned there for new posts, with
    /// nothing focused and no ring.
    #[test]
    fn a_read_only_chat_opened_with_the_pointer_opens_at_its_newest_message() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        let channel = open_channel_with_the_pointer(&mut app, &ctx);
        assert_ne!(
            ctx.memory(|memory| memory.focused()),
            Some(crate::ui::conversation::messages_list_id(&channel)),
            "the pointer does not focus the list"
        );
        assert!(app.at_bottom, "at the newest message");
        assert!(
            app.scroll_to_bottom,
            "still pinned there, as any chat opened with the pointer"
        );
        assert_eq!(ring(&ctx), None);
        // A new post keeps the view at the newest message.
        let conversation = app.conversations.get_mut(&channel).unwrap();
        let mut post = conversation.messages.last().unwrap().clone();
        post.id = "channel-new".into();
        post.timestamp += 60;
        post.content = crate::model::Content::text("A new announcement\nover\nseveral lines");
        conversation.messages.push(post);
        render(&mut app, &ctx);
        assert!(app.at_bottom, "still at the newest message");
    }

    /// In a read-only chat opened with the pointer, PgUp still eases the
    /// view up a page, as before the list existed: no current message
    /// moves and no ring appears.
    #[test]
    fn page_keys_in_a_pointer_opened_channel_still_scroll() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        let channel = open_channel_with_the_pointer(&mut app, &ctx);
        let offset = |ctx: &egui::Context| {
            ctx.data(|data| {
                data.get_temp::<(f32, f32)>(crate::ui::conversation::scroll_metrics_id(&channel))
            })
            .unwrap()
            .0
        };
        let before = offset(&ctx);
        press_key(&mut app, &ctx, egui::Key::PageUp, egui::Modifiers::NONE);
        assert!(offset(&ctx) < before, "PgUp scrolled the view up");
        assert_eq!(app.message_cursor.get(&channel), None, "no cursor moved");
        assert!(!app.reveal_cursor);
        assert_ne!(
            ctx.memory(|memory| memory.focused()),
            Some(crate::ui::conversation::messages_list_id(&channel))
        );
        assert_eq!(ring(&ctx), None);
    }

    /// The message field's focus request, declined in a read-only chat
    /// opened with the pointer, is dropped: a later Tab moves focus as it
    /// always did and leaves the view where the reader scrolled it.
    #[test]
    fn tab_after_a_pointer_opened_channel_does_not_scroll_it() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        let channel = open_channel_with_the_pointer(&mut app, &ctx);
        assert!(!app.focus_composer, "the declined request is dropped");
        let offset = |ctx: &egui::Context| {
            ctx.data(|data| {
                data.get_temp::<(f32, f32)>(crate::ui::conversation::scroll_metrics_id(&channel))
            })
            .unwrap()
            .0
        };
        // Read further up, and let the eased scroll finish.
        press_key(&mut app, &ctx, egui::Key::PageUp, egui::Modifiers::NONE);
        for _ in 0..30 {
            render(&mut app, &ctx);
        }
        let before = offset(&ctx);
        assert!(!app.at_bottom, "scrolled up to read");
        press_key(&mut app, &ctx, egui::Key::Tab, egui::Modifiers::NONE);
        assert_eq!(offset(&ctx), before, "Tab left the view where it was");
        let list = crate::ui::conversation::messages_list_id(&channel);
        assert_ne!(ctx.memory(|memory| memory.focused()), Some(list));
        let first = crate::ui::focus::stops(&ctx)[0].1;
        assert_eq!(
            ctx.memory(|memory| memory.focused()),
            Some(first),
            "Tab reached the first stop, as before the list existed"
        );
    }

    /// Alt+Down is keyboard navigation even straight after launch, before
    /// any Tab: into a chat without a message field it lands on the chat's
    /// message list, not on nothing.
    #[test]
    fn switching_chats_by_keyboard_from_the_field_focuses_a_read_only_chats_list() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        assert_eq!(
            ctx.memory(|memory| memory.focused()),
            Some(egui::Id::new("composer-text"))
        );
        assert!(
            !ctx.data(|data| data
                .get_temp::<bool>(crate::theme::keyboard_focus_id())
                .unwrap_or(false)),
            "launch state: no keyboard navigation yet"
        );
        let channel = channel(&app);
        // The open chat is not a channel, so the first channel is next.
        app.chat_filter = crate::model::ChatFilter::Channels;
        render(&mut app, &ctx);
        press_key(&mut app, &ctx, egui::Key::ArrowDown, egui::Modifiers::ALT);
        assert_eq!(app.open_chat.as_deref(), Some(channel.as_str()));
        assert_eq!(
            ctx.memory(|memory| memory.focused()),
            Some(crate::ui::conversation::messages_list_id(&channel))
        );
    }

    /// Ctrl+2 (Command+2 on macOS), upstream's shortcut for the second chat
    /// in the list, works from the focused message list as from the message
    /// field, and lands where Alt+Down lands: the new chat's message field.
    /// A numbered chat without a message field, such as a channel, lands on
    /// its message list instead.
    #[test]
    fn a_numbered_chat_shortcut_works_from_the_message_list() {
        let digits = [
            egui::Key::Num1,
            egui::Key::Num2,
            egui::Key::Num3,
            egui::Key::Num4,
            egui::Key::Num5,
            egui::Key::Num6,
            egui::Key::Num7,
            egui::Key::Num8,
            egui::Key::Num9,
        ];
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        focus_messages(&mut app, &ctx);
        let chat = app.open_chat.clone().unwrap();
        assert_eq!(
            ctx.memory(|memory| memory.focused()),
            Some(crate::ui::conversation::messages_list_id(&chat))
        );
        let second = app.visible_chats()[1].id.clone();
        assert_ne!(second, chat);
        assert!(
            app.chat(&second).unwrap().can_send(),
            "the second chat has a message field"
        );
        press_key(&mut app, &ctx, egui::Key::Num2, egui::Modifiers::COMMAND);
        render(&mut app, &ctx);
        assert_eq!(app.open_chat.as_ref(), Some(&second));
        assert_eq!(
            ctx.memory(|memory| memory.focused()),
            Some(egui::Id::new("composer-text"))
        );
        let (index, read_only) = app
            .visible_chats()
            .iter()
            .take(digits.len())
            .enumerate()
            .find(|(_, chat)| !chat.can_send())
            .map(|(index, chat)| (index, chat.id.clone()))
            .expect("a chat without a message field among the first nine");
        focus_messages(&mut app, &ctx);
        assert_eq!(
            ctx.memory(|memory| memory.focused()),
            Some(crate::ui::conversation::messages_list_id(&second))
        );
        press_key(&mut app, &ctx, digits[index], egui::Modifiers::COMMAND);
        render(&mut app, &ctx);
        assert_eq!(app.open_chat.as_ref(), Some(&read_only));
        assert_eq!(
            ctx.memory(|memory| memory.focused()),
            Some(crate::ui::conversation::messages_list_id(&read_only))
        );
    }

    /// A list focus deferred past a hidden pass belongs to its chat: drawn
    /// for another chat, it is dropped, and that chat's field keeps focus.
    #[test]
    fn a_deferred_list_focus_is_dropped_for_another_chat() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        let chat = app.open_chat.clone().unwrap();
        assert_eq!(
            ctx.memory(|memory| memory.focused()),
            Some(egui::Id::new("composer-text"))
        );
        app.list_focus_deferred = Some("15550009999@s.whatsapp.net".into());
        render(&mut app, &ctx);
        assert!(app.list_focus_deferred.is_none(), "dropped");
        assert_ne!(
            ctx.memory(|memory| memory.focused()),
            Some(crate::ui::conversation::messages_list_id(&chat))
        );
    }

    /// In background mode a new window reuses the app. A message list that
    /// had focus in the window that closed must not keep the new window's
    /// message field from taking focus.
    #[test]
    fn a_new_window_focuses_the_message_field_after_the_list_had_focus() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        press_key(&mut app, &ctx, egui::Key::Num2, messages_shortcut());
        assert!(app.messages_focused);
        let reopened = egui::Context::default();
        app.attach(&reopened);
        render(&mut app, &reopened);
        assert_eq!(
            reopened.memory(|memory| memory.focused()),
            Some(egui::Id::new("composer-text"))
        );
    }

    #[test]
    fn letters_type_in_the_composer_not_actions() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        ctx.memory_mut(|memory| memory.request_focus(egui::Id::new("composer-text")));
        render(&mut app, &ctx);
        for key in [
            egui::Key::R,
            egui::Key::E,
            egui::Key::C,
            egui::Key::F,
            egui::Key::S,
            egui::Key::M,
            egui::Key::G,
        ] {
            app.actions.clear();
            press_key(&mut app, &ctx, key, egui::Modifiers::NONE);
            assert!(
                app.reply_to.is_none() && app.reaction_target.is_none(),
                "{key:?} acted"
            );
        }
    }

    #[test]
    fn r_replies_to_the_focused_message_and_moves_to_the_composer() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        focus_messages(&mut app, &ctx);
        let chat = app.open_chat.clone().unwrap();
        let newest = app.conversations[&chat].messages.last().unwrap().id.clone();
        press_key(&mut app, &ctx, egui::Key::R, egui::Modifiers::NONE);
        assert_eq!(app.reply_to.as_deref(), Some(newest.as_str()));
        assert!(ctx.memory(|memory| memory.has_focus(egui::Id::new("composer-text"))));
    }

    fn spoke(app: &App, text: &str) -> bool {
        app.announcer.spoken().contains(&text)
    }

    /// A demo app that records its commands and downloads nothing by itself,
    /// with the list focused on a fixture message holding `content`, and the
    /// sender of the backend's events, such as a download's result.
    struct OnAttachment {
        app: App,
        ctx: egui::Context,
        chat: String,
        events: std::sync::mpsc::Sender<crate::backend::Event>,
    }

    fn app_on_attachment(content: Content) -> OnAttachment {
        let root = tempfile::Builder::new()
            .prefix("zapfast-demo-")
            .tempdir()
            .unwrap();
        let (mut app, events) = App::headless(AppDirs::under(root.path()), Settings::default());
        ROOTS.with(|roots| roots.borrow_mut().push(root));
        populate(&mut app);
        let ctx = egui::Context::default();
        app.attach(&ctx);
        app.backend.record_demo_commands();
        app.settings.auto_download = false;
        render(&mut app, &ctx);
        let chat = focus_fixture_message(&mut app, &ctx, content);
        app.backend.take_demo_commands();
        OnAttachment {
            app,
            ctx,
            chat,
            events,
        }
    }

    impl OnAttachment {
        /// A primary click on the bubble's part stored as `part`, in its
        /// middle.
        fn click(&mut self, part: Part) {
            let bubble = crate::ui::conversation::bubble_id(&self.chat, "key-fixture");
            let Part::Named(name) = part;
            let id = bubble.with(name);
            let rect = self
                .ctx
                .data(|data| data.get_temp::<egui::Rect>(id))
                .unwrap_or_else(|| panic!("no {part:?} drawn"));
            let at = rect.center();
            let pointer = |pressed| egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            frame_with(
                &mut self.app,
                &self.ctx,
                vec![egui::Event::PointerMoved(at), pointer(true), pointer(false)],
            );
            render(&mut self.app, &self.ctx);
        }
    }

    fn sample_video() -> std::path::PathBuf {
        std::path::PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/video/sample.mp4"
        ))
    }

    fn video_to_fetch() -> Content {
        Content::Video {
            caption: None,
            media: media("video/mp4", 120_000, Some(800), Some(600)),
            seconds: Some(6),
            gif: false,
            note: false,
        }
    }

    /// Where a test clicks in the fixture's bubble.
    #[derive(Clone, Copy, Debug)]
    enum Part {
        /// The part the bubble stores under this name.
        Named(&'static str),
    }

    /// The fixture replying to the open chat's oldest loaded message, with
    /// the list focused on the fixture. Returns the replied message's id.
    fn on_a_reply() -> (OnAttachment, String) {
        let mut on = app_on_attachment(Content::text("Agreed"));
        let target = on.app.conversations[&on.chat].messages[0].clone();
        let conversation = on.app.conversations.get_mut(&on.chat).unwrap();
        conversation.message_mut("key-fixture").unwrap().quoted = Some(crate::model::Quoted {
            labelled: None,
            chat: None,
            id: target.id.clone(),
            sender: target.sender.clone(),
            sender_name: None,
            summary: target.summary(),
            mentions: Vec::new(),
        });
        render(&mut on.app, &on.ctx);
        (on, target.id)
    }

    /// The fixture replying to a message that is not loaded, with the list
    /// focused on the fixture. Returns the replied message.
    fn on_a_reply_not_loaded() -> (OnAttachment, Message) {
        let mut on = app_on_attachment(Content::text("Agreed"));
        let oldest = on.app.conversations[&on.chat].messages[0].timestamp;
        let target = message(
            &on.chat,
            "not-loaded",
            false,
            oldest - 60,
            Content::text("Long ago"),
        );
        let conversation = on.app.conversations.get_mut(&on.chat).unwrap();
        conversation.message_mut("key-fixture").unwrap().quoted = Some(crate::model::Quoted {
            labelled: None,
            chat: None,
            id: target.id.clone(),
            sender: target.sender.clone(),
            sender_name: None,
            summary: target.summary(),
            mentions: Vec::new(),
        });
        render(&mut on.app, &on.ctx);
        (on, target)
    }

    /// G on a reply whose quoted message is not loaded keeps the list on the
    /// reply, says it is loading, and moves there once history brings it.
    #[test]
    fn g_waits_for_a_replied_message_that_is_not_loaded() {
        let (mut on, target) = on_a_reply_not_loaded();
        press_key(&mut on.app, &on.ctx, egui::Key::G, egui::Modifiers::NONE);
        assert_eq!(
            current_message(&on),
            "key-fixture",
            "the list keeps its place"
        );
        assert!(
            spoke(&on.app, "Loading the replied message"),
            "{:?}",
            on.app.announcer.spoken()
        );
        assert!(
            on.app
                .backend
                .take_demo_commands()
                .iter()
                .any(|command| matches!(
                    command,
                    crate::backend::Command::LoadUntil { id, .. } if id == "not-loaded"
                )),
            "older history is asked for"
        );
        on.events
            .send(crate::backend::Event::Messages {
                chat: on.chat.clone(),
                messages: vec![target],
                older: true,
                complete: true,
            })
            .unwrap();
        render(&mut on.app, &on.ctx);
        assert_eq!(
            on.app.message_cursor.get(&on.chat).map(String::as_str),
            Some("not-loaded")
        );
    }

    /// G held down asks once and keeps waiting: its repeats, and the text
    /// they type, are not a key after it, so the replied message still moves
    /// the list once it arrives, and "Loading" is not said again.
    #[test]
    fn holding_g_keeps_waiting_for_the_replied_message() {
        let (mut on, target) = on_a_reply_not_loaded();
        press_key(&mut on.app, &on.ctx, egui::Key::G, egui::Modifiers::NONE);
        assert!(spoke(&on.app, "Loading the replied message"));
        on.app.announcer = Default::default();
        for _ in 0..3 {
            hold_key(&mut on.app, &on.ctx, egui::Key::G, "g");
        }
        render(&mut on.app, &on.ctx);
        assert!(
            !spoke(&on.app, "Loading the replied message"),
            "{:?}",
            on.app.announcer.spoken()
        );
        assert_eq!(current_message(&on), "key-fixture", "still waiting");
        deliver(&mut on, vec![target]);
        assert_eq!(current_message(&on), "not-loaded");
    }

    /// History that comes without the replied message says it is not there,
    /// and the list stays where it was.
    #[test]
    fn g_says_when_the_replied_message_is_not_found() {
        let (mut on, _) = on_a_reply_not_loaded();
        press_key(&mut on.app, &on.ctx, egui::Key::G, egui::Modifiers::NONE);
        on.app.announcer = Default::default();
        on.events
            .send(crate::backend::Event::Messages {
                chat: on.chat.clone(),
                messages: Vec::new(),
                older: true,
                complete: false,
            })
            .unwrap();
        render(&mut on.app, &on.ctx);
        assert!(
            spoke(&on.app, "Replied message not found"),
            "{:?}",
            on.app.announcer.spoken()
        );
        assert_eq!(current_message(&on), "key-fixture");
    }

    /// A page that completes the chat's history without the replied message
    /// ends the wait at once: no older history can bring it, so nothing more
    /// is asked for, it is said not to be found, and the list stays.
    #[test]
    fn the_end_of_history_without_the_replied_message_says_it_is_not_found() {
        let (mut on, target) = on_a_reply_not_loaded();
        // A page already on its way: G waits for it before asking its own.
        on.app
            .conversations
            .get_mut(&on.chat)
            .unwrap()
            .loading_older = true;
        press_key(&mut on.app, &on.ctx, egui::Key::G, egui::Modifiers::NONE);
        on.app.backend.take_demo_commands();
        on.app.announcer = Default::default();
        let other = message(
            &on.chat,
            "older-page",
            false,
            target.timestamp + 1,
            Content::text("Older"),
        );
        on.events
            .send(crate::backend::Event::Messages {
                chat: on.chat.clone(),
                messages: vec![other],
                older: true,
                complete: true,
            })
            .unwrap();
        render(&mut on.app, &on.ctx);
        assert!(
            spoke(&on.app, "Replied message not found"),
            "{:?}",
            on.app.announcer.spoken()
        );
        assert!(
            !on.app
                .backend
                .take_demo_commands()
                .iter()
                .any(|command| matches!(command, crate::backend::Command::LoadUntil { .. })),
            "nothing more to ask for"
        );
        assert_eq!(current_message(&on), "key-fixture");
        deliver(&mut on, vec![target]);
        assert_eq!(current_message(&on), "key-fixture", "the wait is over");
    }

    /// The list's current message: its cursor, or the newest without one.
    fn current_message(on: &OnAttachment) -> String {
        on.app
            .message_cursor
            .get(&on.chat)
            .cloned()
            .unwrap_or_else(|| {
                on.app.conversations[&on.chat]
                    .messages
                    .last()
                    .unwrap()
                    .id
                    .clone()
            })
    }

    /// The page that brings the replied message `target`.
    fn deliver(on: &mut OnAttachment, messages: Vec<Message>) {
        on.events
            .send(crate::backend::Event::Messages {
                chat: on.chat.clone(),
                messages,
                older: true,
                complete: false,
            })
            .unwrap();
        render(&mut on.app, &on.ctx);
    }

    /// Points the fixture's quote at the open chat's oldest loaded message
    /// and returns that message's id.
    fn quote_the_oldest(on: &mut OnAttachment) -> String {
        let target = on.app.conversations[&on.chat].messages[0].clone();
        let conversation = on.app.conversations.get_mut(&on.chat).unwrap();
        conversation.message_mut("key-fixture").unwrap().quoted = Some(crate::model::Quoted {
            labelled: None,
            chat: None,
            id: target.id.clone(),
            sender: target.sender.clone(),
            sender_name: None,
            summary: target.summary(),
            mentions: Vec::new(),
        });
        render(&mut on.app, &on.ctx);
        target.id
    }

    /// Once the reader moves on, a replied message still loading never takes
    /// the list back, and nothing more is said about it: after an arrow,
    /// after G on a reply whose quote is loaded, after a click on a quote,
    /// and after a search result.
    #[test]
    fn a_reply_still_loading_ends_when_the_reader_moves_on() {
        for way in ["arrow", "g", "click", "search"] {
            let (mut on, target) = on_a_reply_not_loaded();
            press_key(&mut on.app, &on.ctx, egui::Key::G, egui::Modifiers::NONE);
            assert!(spoke(&on.app, "Loading the replied message"), "{way}");
            match way {
                "arrow" => press_key(
                    &mut on.app,
                    &on.ctx,
                    egui::Key::ArrowUp,
                    egui::Modifiers::NONE,
                ),
                "g" => {
                    quote_the_oldest(&mut on);
                    // Let go of the first G, or egui takes the second for
                    // the key held down, which jumps only once.
                    release_key(&mut on.app, &on.ctx, egui::Key::G);
                    press_key(&mut on.app, &on.ctx, egui::Key::G, egui::Modifiers::NONE);
                }
                "click" => {
                    quote_the_oldest(&mut on);
                    on.click(Part::Named("quote"));
                }
                _ => {
                    let oldest = on.app.conversations[&on.chat].messages[0].id.clone();
                    on.app.actions.push(crate::model::Action::OpenMessage {
                        chat: on.chat.clone(),
                        message: oldest,
                    });
                    render(&mut on.app, &on.ctx);
                }
            }
            let place = current_message(&on);
            assert_ne!(place, "key-fixture", "{way}: the reader moved");
            on.app.announcer = Default::default();
            deliver(&mut on, vec![target.clone()]);
            assert_eq!(current_message(&on), place, "{way}: the list stays");
            assert!(
                on.app
                    .announcer
                    .spoken()
                    .iter()
                    .all(|spoken| spoken.is_empty()),
                "{way}: {:?}",
                on.app.announcer.spoken()
            );
        }
    }

    /// With an older page already on its way, G asks for its own once that
    /// page is in, and says nothing about a page that was not its answer.
    #[test]
    fn a_reply_still_loading_asks_for_its_own_history() {
        let (mut on, target) = on_a_reply_not_loaded();
        on.app
            .conversations
            .get_mut(&on.chat)
            .unwrap()
            .loading_older = true;
        press_key(&mut on.app, &on.ctx, egui::Key::G, egui::Modifiers::NONE);
        let asked = |on: &OnAttachment| {
            on.app.backend.take_demo_commands().iter().any(|command| {
                matches!(
                    command,
                    crate::backend::Command::LoadUntil { id, .. } if id == "not-loaded"
                )
            })
        };
        assert!(!asked(&on), "a page is already on its way");
        on.app.announcer = Default::default();
        let other = message(
            &on.chat,
            "older-page",
            false,
            target.timestamp + 1,
            Content::text("Older"),
        );
        deliver(&mut on, vec![other]);
        assert!(asked(&on), "its own request once that page is in");
        assert!(
            !on.app
                .announcer
                .spoken()
                .iter()
                .any(|spoken| spoken.contains("not found")),
            "{:?}",
            on.app.announcer.spoken()
        );
        deliver(&mut on, vec![target]);
        assert_eq!(current_message(&on), "not-loaded");
    }

    /// When the archive says the replied message is not stored, that error
    /// is the one thing said.
    /// The same for an archive that cannot be read: the worker answers with
    /// the error, the error alone is said, in the frame that drains it and
    /// in every frame after it, and the wait is over, so a later page does
    /// not move the list.
    #[test]
    fn the_archives_error_replaces_replied_message_not_found() {
        for error in [
            "This message is not stored on this computer",
            "Could not read the chat: fixture",
        ] {
            let (mut on, target) = on_a_reply_not_loaded();
            press_key(&mut on.app, &on.ctx, egui::Key::G, egui::Modifiers::NONE);
            on.app.announcer = Default::default();
            on.events
                .send(crate::backend::Event::HistoryUnavailable {
                    chat: on.chat.clone(),
                    error: error.into(),
                })
                .unwrap();
            // Frame by frame: "not found" would come in a frame after the
            // error's, from a wait the error did not end.
            let mut spoken = String::new();
            for _ in 0..3 {
                render(&mut on.app, &on.ctx);
                spoken.push_str(&on.app.announcer.spoken().concat());
            }
            assert!(spoken.contains(error), "{spoken}");
            assert!(!spoken.contains("Replied message not found"), "{spoken}");
            assert_eq!(current_message(&on), "key-fixture");
            deliver(&mut on, vec![target]);
            assert_eq!(
                current_message(&on),
                "key-fixture",
                "{error}: the wait ended"
            );
        }
    }

    /// An error that is not the answer to G's own request, such as a failed
    /// send, leaves the wait alone: the page that brings the replied message
    /// still moves the list there.
    #[test]
    fn an_unrelated_error_leaves_the_replied_message_wait() {
        let (mut on, target) = on_a_reply_not_loaded();
        press_key(&mut on.app, &on.ctx, egui::Key::G, egui::Modifiers::NONE);
        on.events
            .send(crate::backend::Event::Error("Not connected".into()))
            .unwrap();
        render(&mut on.app, &on.ctx);
        assert_eq!(current_message(&on), "key-fixture", "still waiting");
        deliver(&mut on, vec![target]);
        assert_eq!(current_message(&on), "not-loaded");
    }

    /// The quote's own click is as it was: the list's cursor goes to the
    /// replied message at once, loaded or not.
    #[test]
    fn the_quote_click_still_jumps_at_once() {
        let (mut on, _) = on_a_reply_not_loaded();
        on.click(Part::Named("quote"));
        assert_eq!(
            on.app.message_cursor.get(&on.chat).map(String::as_str),
            Some("not-loaded")
        );
    }

    /// The label of the message list's current message, as a screen reader
    /// reads it, if the list has focus.
    fn current_message_label(on: &mut OnAttachment) -> Option<String> {
        let tree = tree_after(&mut on.app, &on.ctx, None);
        let list = crate::ui::conversation::messages_list_id(&on.chat).accesskit_id();
        if tree.focus != list {
            return None;
        }
        let active = node(&tree, list)?.active_descendant()?;
        node(&tree, active).and_then(|node| node.label().map(str::to_owned))
    }

    /// G goes where clicking the quote goes: the list moves to the replied
    /// message, keeps focus, and reads that message.
    #[test]
    fn g_goes_to_the_replied_message_as_its_quote_click_does() {
        let (mut on, target) = on_a_reply();
        press_key(&mut on.app, &on.ctx, egui::Key::G, egui::Modifiers::NONE);
        assert_eq!(on.app.message_cursor.get(&on.chat), Some(&target));
        let label = current_message_label(&mut on).expect("the list keeps focus");
        let body = on.app.conversations[&on.chat].messages[0].summary();
        assert!(label.contains(body.trim()), "{label} / {body}");
        // The click on the quote lands on the same message.
        let (mut clicked, target) = on_a_reply();
        clicked.click(Part::Named("quote"));
        assert_eq!(clicked.app.message_cursor.get(&clicked.chat), Some(&target));
    }

    /// Shift, Ctrl and Alt with a message key are left alone.
    #[test]
    fn modified_message_keys_do_not_act() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        focus_messages(&mut app, &ctx);
        for modifiers in [
            egui::Modifiers::SHIFT,
            egui::Modifiers::COMMAND,
            egui::Modifiers::ALT,
        ] {
            press_key(&mut app, &ctx, egui::Key::R, modifiers);
            assert!(app.reply_to.is_none(), "R with {modifiers:?} replied");
        }
        // F10 needs Shift alone, and Shift+Enter is no menu key either.
        for (key, modifiers) in [
            (egui::Key::F10, egui::Modifiers::NONE),
            (
                egui::Key::F10,
                egui::Modifiers::SHIFT | egui::Modifiers::COMMAND,
            ),
            (egui::Key::Enter, egui::Modifiers::SHIFT),
        ] {
            press_key(&mut app, &ctx, key, modifiers);
            assert!(
                !egui::Popup::is_any_open(&ctx),
                "{key:?} with {modifiers:?}"
            );
        }
    }

    #[test]
    fn shift_f10_opens_the_message_menu_with_focus_inside() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        focus_messages(&mut app, &ctx);
        press_key(&mut app, &ctx, egui::Key::F10, egui::Modifiers::SHIFT);
        assert!(egui::Popup::is_any_open(&ctx));
        let focused = ctx.memory(|memory| memory.focused());
        let chat = app.open_chat.clone().unwrap();
        assert_ne!(
            focused,
            Some(crate::ui::conversation::messages_list_id(&chat)),
            "focus moved into the menu"
        );
        let newest = app.conversations[&chat].messages.last().unwrap().id.clone();
        let popup = crate::ui::conversation::bubble_id(&chat, &newest).with("popup");
        assert_eq!(
            focused_layer(&ctx).map(|layer| layer.id),
            Some(popup),
            "a menu item has focus"
        );
        assert!(
            app.open_message_menu.is_none(),
            "opened once, not pinned open"
        );
    }

    /// Shift+F10 lands on a row of the menu whatever the message holds: a
    /// deleted message has no Reply, Star or Select, and its menu still
    /// takes focus rather than leaving it on the list behind.
    #[test]
    fn the_keyboard_menu_focuses_a_row_on_every_kind_of_message() {
        let sample = app();
        let mut kinds: Vec<Content> = Vec::new();
        let mut chats: Vec<_> = sample.conversations.keys().collect();
        chats.sort();
        for chat in chats {
            for message in &sample.conversations[chat].messages {
                let kind = std::mem::discriminant(&message.content);
                if !kinds
                    .iter()
                    .any(|known| std::mem::discriminant(known) == kind)
                {
                    kinds.push(message.content.clone());
                }
            }
        }
        kinds.push(Content::Unsupported {
            what: "Fixture unsupported".into(),
        });
        assert!(kinds.iter().any(|kind| matches!(kind, Content::Revoked)));
        for content in kinds {
            let what = format!("{content:?}");
            let mut app = app();
            let ctx = egui::Context::default();
            app.attach(&ctx);
            render(&mut app, &ctx);
            let chat = focus_fixture_message(&mut app, &ctx, content);
            press_key(&mut app, &ctx, egui::Key::F10, egui::Modifiers::SHIFT);
            let popup = crate::ui::conversation::bubble_id(&chat, "key-fixture").with("popup");
            assert!(egui::Popup::is_id_open(&ctx, popup), "{what}: the menu");
            assert_eq!(
                focused_layer(&ctx).map(|layer| layer.id),
                Some(popup),
                "{what}: a menu row has focus"
            );
        }
    }

    /// A deleted message from someone in a group focuses the menu's first
    /// row, their info, which comes before Delete for me.
    #[test]
    fn a_deleted_group_messages_keyboard_menu_focuses_its_first_row() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut app = app();
        app.attach(&ctx);
        let mut groups: Vec<_> = app
            .conversations
            .keys()
            .filter(|chat| chat.ends_with("@g.us"))
            .cloned()
            .collect();
        groups.sort();
        let (chat, sender) = groups
            .into_iter()
            .find_map(|chat| {
                let sender = app.conversations[&chat]
                    .messages
                    .iter()
                    .find(|message| !message.from_me)?
                    .sender
                    .clone();
                Some((chat, sender))
            })
            .expect("a group with a message from someone else");
        app.open_chat = Some(chat.clone());
        render(&mut app, &ctx);
        let conversation = app.conversations.get_mut(&chat).unwrap();
        let newest = conversation.messages.last().unwrap().timestamp + 60;
        let mut deleted = message(&chat, "key-fixture", false, newest, Content::Revoked);
        deleted.sender = sender.clone();
        conversation.messages.push(deleted);
        render(&mut app, &ctx);
        focus_messages(&mut app, &ctx);
        press_key(&mut app, &ctx, egui::Key::F10, egui::Modifiers::SHIFT);
        let name = app.display_name_or(&sender, None);
        assert_eq!(
            focused_label(&mut app, &ctx),
            Some(format!("Info about {}", name.trim_start_matches('~')))
        );
    }

    /// Enter is no menu key: on a text message, where Space has nothing to
    /// do, it does nothing either, and the list keeps focus.
    #[test]
    fn enter_on_a_text_message_does_nothing() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        let chat = focus_fixture_message(&mut app, &ctx, Content::text("Fixture plain text"));
        app.actions.clear();
        press_key(&mut app, &ctx, egui::Key::Enter, egui::Modifiers::NONE);
        assert!(!egui::Popup::is_any_open(&ctx), "no menu opened");
        assert!(app.open_message_menu.is_none());
        assert!(
            app.dialog.is_none() && app.reply_to.is_none() && app.reaction_target.is_none(),
            "nothing acted on the message"
        );
        assert_eq!(
            ctx.memory(|memory| memory.focused()),
            Some(crate::ui::conversation::messages_list_id(&chat)),
            "the list keeps focus"
        );
    }

    /// Opens the focused message's menu from the keyboard and returns the
    /// menu's popup id.
    fn open_menu_by_keys(app: &mut App, ctx: &egui::Context) -> egui::Id {
        render(app, ctx);
        focus_messages(app, ctx);
        let chat = app.open_chat.clone().unwrap();
        let newest = app.conversations[&chat].messages.last().unwrap().id.clone();
        press_key(app, ctx, egui::Key::F10, egui::Modifiers::SHIFT);
        let popup = crate::ui::conversation::bubble_id(&chat, &newest).with("popup");
        assert!(egui::Popup::is_id_open(ctx, popup));
        popup
    }

    #[test]
    fn escape_closes_the_message_menu_and_returns_to_the_list() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        open_menu_by_keys(&mut app, &ctx);
        press_key(&mut app, &ctx, egui::Key::Escape, egui::Modifiers::NONE);
        assert!(!egui::Popup::is_any_open(&ctx));
        let chat = app.open_chat.clone().unwrap();
        assert_eq!(
            ctx.memory(|memory| memory.focused()),
            Some(crate::ui::conversation::messages_list_id(&chat))
        );
        assert!(app.open_chat.is_some(), "Escape closed the menu only");
    }

    /// The layer of the focused widget, if any.
    fn focused_layer(ctx: &egui::Context) -> Option<egui::LayerId> {
        ctx.memory(|memory| memory.focused())
            .and_then(|id| ctx.read_response(id))
            .map(|response| response.layer_id)
    }

    #[test]
    fn tab_and_the_arrows_stay_in_a_menu_opened_from_the_keyboard() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        let popup = open_menu_by_keys(&mut app, &ctx);
        for (key, modifiers) in [
            (egui::Key::Tab, egui::Modifiers::NONE),
            (egui::Key::Tab, egui::Modifiers::SHIFT),
            (egui::Key::ArrowDown, egui::Modifiers::NONE),
            (egui::Key::ArrowUp, egui::Modifiers::NONE),
        ] {
            let mut seen = std::collections::HashSet::new();
            for _ in 0..24 {
                press_key(&mut app, &ctx, key, modifiers);
                assert!(egui::Popup::is_id_open(&ctx, popup), "{key:?} closed it");
                let layer = focused_layer(&ctx);
                assert_eq!(
                    layer.map(|layer| layer.id),
                    Some(popup),
                    "{key:?} with {modifiers:?} left the menu"
                );
                seen.insert(ctx.memory(|memory| memory.focused()));
            }
            assert!(seen.len() > 1, "{key:?} with {modifiers:?} moved focus");
        }
    }

    /// Typing a letter in the message menu moves focus to the next item that
    /// starts with it, as in a desktop menu, and the screen reader is told
    /// of the new item as it is for the arrows. The item is not chosen, and
    /// the letter reaches neither the message list's keys (R, E) nor the
    /// message field.
    #[test]
    fn a_letter_moves_through_the_message_menu_without_acting() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        let popup = open_menu_by_keys(&mut app, &ctx);
        let typed = |app: &mut App, key: egui::Key, text: &str| {
            frame_with(
                app,
                &ctx,
                vec![
                    egui::Event::Key {
                        key,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    },
                    egui::Event::Text(text.to_owned()),
                ],
            );
            render(app, &ctx);
            assert!(egui::Popup::is_id_open(&ctx, popup), "{text:?} closed it");
            assert_eq!(
                focused_layer(&ctx).map(|layer| layer.id),
                Some(popup),
                "{text:?} left the menu"
            );
            let tree = tree_after(app, &ctx, None);
            node(&tree, tree.focus)
                .and_then(|node| node.label().map(str::to_owned))
                .expect("the focused item is named")
        };
        for (key, letter) in [
            (egui::Key::C, 'c'),
            (egui::Key::D, 'd'),
            (egui::Key::R, 'r'),
        ] {
            let label = typed(&mut app, key, &letter.to_string());
            assert!(
                label.to_lowercase().starts_with(letter),
                "{letter}: focus on {label:?}"
            );
        }
        typed(&mut app, egui::Key::E, "e");
        assert!(app.reply_to.is_none(), "R did not reply");
        assert!(app.reaction_target.is_none(), "E did not react");
        assert!(
            app.dialog.is_none() && app.toasts.is_empty(),
            "nothing chosen"
        );
        assert!(
            app.composer.is_empty(),
            "{:?} reached the field",
            app.composer
        );
    }

    /// Tab and Shift+Tab in a menu the keys opened stop on named items only:
    /// the menu's background takes clicks but is no Tab stop of its own.
    #[test]
    fn tab_in_a_menu_opened_from_the_keyboard_never_lands_on_an_unnamed_node() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        ctx.enable_accesskit();
        let popup = open_menu_by_keys(&mut app, &ctx);
        for modifiers in [egui::Modifiers::NONE, egui::Modifiers::SHIFT] {
            let mut seen = std::collections::HashSet::new();
            for _ in 0..40 {
                press_key(&mut app, &ctx, egui::Key::Tab, modifiers);
                assert!(egui::Popup::is_id_open(&ctx, popup), "Tab closed it");
                let tree = tree_after(&mut app, &ctx, None);
                let focused = tree
                    .nodes
                    .iter()
                    .find(|(id, _)| *id == tree.focus)
                    .map(|(_, node)| node.clone())
                    .expect("the focused node is in the tree");
                let named = focused
                    .label()
                    .is_some_and(|label| !label.trim().is_empty());
                assert!(
                    named,
                    "Tab with {modifiers:?} landed on an unnamed {:?}",
                    focused.role()
                );
                seen.insert(tree.focus);
            }
            assert!(seen.len() > 1, "Tab with {modifiers:?} moved focus");
        }
    }

    /// The open chat with one fixture message as its newest, the list
    /// focused on it.
    fn focus_fixture_message(app: &mut App, ctx: &egui::Context, content: Content) -> String {
        let chat = app.open_chat.clone().unwrap();
        let conversation = app.conversations.get_mut(&chat).unwrap();
        let newest = conversation.messages.last().unwrap().timestamp + 60;
        conversation
            .messages
            .push(message(&chat, "key-fixture", false, newest, content));
        render(app, ctx);
        focus_messages(app, ctx);
        chat
    }

    #[test]
    fn c_copies_the_focused_message_text() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        focus_fixture_message(&mut app, &ctx, Content::text("Fixture copy text"));
        press_key(&mut app, &ctx, egui::Key::C, egui::Modifiers::NONE);
        assert!(
            app.toasts.iter().any(|toast| toast.message == "Copied"),
            "{:?}",
            app.toasts
        );
        assert!(
            app.announcer
                .spoken()
                .iter()
                .any(|spoken| spoken.contains("Copied"))
        );
    }

    #[test]
    fn c_copies_a_downloaded_photo_and_nothing_from_one_not_downloaded() {
        for downloaded in [true, false] {
            let mut app = app();
            // A real file: a path whose file is gone downloads again instead.
            let (path, _) = sample_files(&app);
            let ctx = egui::Context::default();
            app.attach(&ctx);
            app.backend.record_demo_commands();
            render(&mut app, &ctx);
            let mut photo = media("image/jpeg", 120_000, Some(800), Some(600));
            photo.path = downloaded.then(|| path.clone());
            focus_fixture_message(
                &mut app,
                &ctx,
                Content::Image {
                    motion: None,
                    caption: None,
                    media: photo,
                },
            );
            app.backend.take_demo_commands();
            press_key(&mut app, &ctx, egui::Key::C, egui::Modifiers::NONE);
            let copied = app.backend.take_demo_commands().iter().any(|command| {
                matches!(
                    command,
                    crate::backend::Command::PrepareClipboardImage(copied) if copied == &path
                )
            });
            assert_eq!(copied, downloaded);
            assert!(!app.toasts.iter().any(|toast| toast.message == "Copied"));
        }
    }

    #[test]
    fn f_opens_the_forward_dialog_for_the_focused_message() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        let chat = focus_fixture_message(&mut app, &ctx, Content::text("Fixture forward"));
        press_key(&mut app, &ctx, egui::Key::F, egui::Modifiers::NONE);
        assert_eq!(
            app.dialog,
            Some(crate::model::Dialog::Forward {
                chat,
                messages: vec!["key-fixture".into()],
            })
        );
    }

    /// The Calls tab keeps the open chat beside its history, and the list's
    /// current message takes its keys there as on the chats' page. A narrow
    /// window that shows only the history has no list on screen: there the
    /// keys act on no message.
    #[test]
    fn the_message_keys_work_beside_the_call_history() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        app.page = crate::model::Page::Calls;
        render(&mut app, &ctx);
        let chat = focus_fixture_message(&mut app, &ctx, Content::text("Fixture forward"));
        assert_eq!(app.page, crate::model::Page::Calls, "the history stays");
        let list = crate::ui::conversation::messages_list_id(&chat);
        assert!(ctx.memory(|memory| memory.has_focus(list)));
        press_key(&mut app, &ctx, egui::Key::F, egui::Modifiers::NONE);
        assert_eq!(
            app.dialog,
            Some(crate::model::Dialog::Forward {
                chat,
                messages: vec!["key-fixture".into()],
            })
        );

        // Narrow, the Calls tab shows only its history beside a chat that
        // was not picked from it.
        app.dialog = None;
        let narrow = |app: &mut App, events: Vec<egui::Event>| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(500.0, 780.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    let ctx = ui.ctx().clone();
                    app.background_frame(&ctx);
                    app.frame_ui(ui);
                },
            );
            output.textures_delta.clear();
        };
        for _ in 0..2 {
            narrow(&mut app, Vec::new());
        }
        narrow(
            &mut app,
            vec![egui::Event::Key {
                key: egui::Key::F,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert_eq!(app.page, crate::model::Page::Calls);
        assert_eq!(app.dialog, None, "no message on screen to forward");
    }

    /// Delete asks first, and only to delete the message for me: Shift+Delete
    /// is the one for everyone.
    #[test]
    fn delete_asks_to_delete_the_focused_message_for_me() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        let chat = focus_fixture_message(&mut app, &ctx, Content::text("Fixture delete"));
        press_key(&mut app, &ctx, egui::Key::Delete, egui::Modifiers::NONE);
        assert_eq!(
            app.dialog,
            Some(crate::model::Dialog::ConfirmDeleteMessage {
                chat: chat.clone(),
                message: "key-fixture".into(),
                for_everyone: false,
            })
        );
        assert!(
            app.conversations[&chat].message("key-fixture").is_some(),
            "nothing is deleted before the confirmation"
        );
    }

    /// Shift+Delete on our own message, sent within WhatsApp's time limit,
    /// opens the dialog the menu's "Delete for everyone" opens, and deletes
    /// nothing until it is confirmed. On another's message it opens nothing
    /// and says why.
    #[test]
    fn shift_delete_asks_to_delete_the_focused_message_for_everyone() {
        let mut ours = app();
        let ctx = egui::Context::default();
        ours.attach(&ctx);
        ours.backend.record_demo_commands();
        render(&mut ours, &ctx);
        let chat = focus_fixture_message(&mut ours, &ctx, Content::text("Fixture revoke"));
        let fixture = ours
            .conversations
            .get_mut(&chat)
            .unwrap()
            .messages
            .iter_mut()
            .find(|message| message.id == "key-fixture")
            .unwrap();
        fixture.from_me = true;
        fixture.timestamp = crate::util::now();
        press_key(&mut ours, &ctx, egui::Key::Delete, egui::Modifiers::SHIFT);
        assert_eq!(
            ours.dialog,
            Some(crate::model::Dialog::ConfirmDeleteMessage {
                chat: chat.clone(),
                message: "key-fixture".into(),
                for_everyone: true,
            })
        );
        assert!(
            !matches!(
                ours.conversations[&chat]
                    .message("key-fixture")
                    .unwrap()
                    .content,
                Content::Revoked
            ),
            "nothing is deleted before the confirmation"
        );
        assert!(
            !ours
                .backend
                .take_demo_commands()
                .iter()
                .any(|command| matches!(command, crate::backend::Command::Revoke { .. })),
            "nothing is sent before the confirmation"
        );

        // Another's message: no dialog, and the reason.
        let mut theirs = app();
        let ctx = egui::Context::default();
        theirs.attach(&ctx);
        render(&mut theirs, &ctx);
        focus_fixture_message(&mut theirs, &ctx, Content::text("Fixture revoke"));
        theirs.announcer = Default::default();
        press_key(&mut theirs, &ctx, egui::Key::Delete, egui::Modifiers::SHIFT);
        assert_eq!(theirs.dialog, None);
        assert!(
            theirs.announcer.spoken().iter().any(|spoken| spoken
                .contains("Delete for everyone, unavailable: Only messages you sent")),
            "{:?}",
            theirs.announcer.spoken()
        );
    }

    #[test]
    fn e_opens_the_reaction_picker_with_focus_in_its_search() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        let chat = focus_fixture_message(&mut app, &ctx, Content::text("Fixture react"));
        press_key(&mut app, &ctx, egui::Key::E, egui::Modifiers::NONE);
        assert_eq!(app.reaction_target, Some((chat, "key-fixture".into())));
        assert_eq!(
            ctx.memory(|memory| memory.focused()),
            Some(egui::Id::new("reaction-emoji-search"))
        );
    }

    /// Turns the open chat's last `count` messages into one album from
    /// another sender, its pictures not downloaded, and returns their ids.
    fn album_fixture(app: &mut App, count: usize) -> Vec<String> {
        let chat = app.open_chat.clone().unwrap();
        let messages = &mut app.conversations.get_mut(&chat).unwrap().messages;
        let total = messages.len();
        let at = messages[total - count - 1].timestamp + 60;
        for message in &mut messages[total - count..] {
            message.from_me = false;
            message.sender = "album@s.whatsapp.net".into();
            message.timestamp = at;
            message.quoted = None;
            message.content = Content::Image {
                motion: None,
                caption: None,
                media: crate::model::Media {
                    mime: "image/jpeg".into(),
                    size: 1,
                    width: Some(640),
                    height: Some(480),
                    album: Some("album-fixture".into()),
                    path: None,
                    state: crate::model::MediaState::Idle,
                },
            };
        }
        messages[total - count..]
            .iter()
            .map(|message| message.id.clone())
            .collect()
    }

    /// An album is one row of the list, which its first picture names: the
    /// keys act on that picture even when the list's cursor was left on a
    /// later one, and its menu opens with focus inside.
    #[test]
    fn the_message_keys_act_on_an_albums_first_picture() {
        let start = || {
            let mut app = app();
            let ctx = egui::Context::default();
            app.attach(&ctx);
            let ids = album_fixture(&mut app, 3);
            let chat = app.open_chat.clone().unwrap();
            render(&mut app, &ctx);
            focus_messages(&mut app, &ctx);
            app.message_cursor.insert(chat.clone(), ids[2].clone());
            (app, ctx, chat, ids)
        };
        let (mut app, ctx, _, ids) = start();
        press_key(&mut app, &ctx, egui::Key::R, egui::Modifiers::NONE);
        assert_eq!(app.reply_to.as_deref(), Some(ids[0].as_str()));
        let (mut app, ctx, chat, ids) = start();
        press_key(&mut app, &ctx, egui::Key::F10, egui::Modifiers::SHIFT);
        let popup = crate::ui::conversation::bubble_id(&chat, &ids[0]).with("popup");
        assert!(egui::Popup::is_id_open(&ctx, popup), "the album's menu");
        assert_eq!(
            focused_layer(&ctx).map(|layer| layer.id),
            Some(popup),
            "a menu item has focus"
        );
    }

    /// A call in the chat is a record, with no replies, reactions or
    /// deletion in its menu: only Space, Enter and Shift+F10 act on it, each
    /// opening that menu with focus on calling back.
    #[test]
    fn a_call_entry_takes_only_the_menu_keys() {
        let call = || {
            let mut app = app();
            let ctx = egui::Context::default();
            app.attach(&ctx);
            render(&mut app, &ctx);
            let chat = focus_fixture_message(
                &mut app,
                &ctx,
                Content::Call {
                    video: false,
                    status: crate::model::CallStatus::Missed,
                    duration: 0,
                    group: false,
                },
            );
            (app, ctx, chat)
        };
        let (mut app, ctx, _) = call();
        for key in [
            egui::Key::R,
            egui::Key::E,
            egui::Key::C,
            egui::Key::F,
            egui::Key::Delete,
        ] {
            app.actions.clear();
            press_key(&mut app, &ctx, key, egui::Modifiers::NONE);
            assert!(
                app.reply_to.is_none() && app.reaction_target.is_none() && app.dialog.is_none(),
                "{key:?} acted on a call"
            );
            assert!(!egui::Popup::is_any_open(&ctx), "{key:?} opened a menu");
        }
        for (key, modifiers) in [
            (egui::Key::Space, egui::Modifiers::NONE),
            (egui::Key::Enter, egui::Modifiers::NONE),
            (egui::Key::F10, egui::Modifiers::SHIFT),
        ] {
            let (mut app, ctx, chat) = call();
            press_key(&mut app, &ctx, key, modifiers);
            let popup = crate::ui::conversation::bubble_id(&chat, "key-fixture").with("popup");
            assert!(
                egui::Popup::is_id_open(&ctx, popup),
                "{key:?}: the call's menu"
            );
            assert_eq!(
                focused_layer(&ctx).map(|layer| layer.id),
                Some(popup),
                "{key:?}: Voice call has focus"
            );
        }
    }

    /// A group call's menu has no choice, only why: opened from the keys,
    /// that row takes focus and says it, so the reader does not land nowhere.
    #[test]
    fn a_group_call_entrys_menu_focuses_why_it_has_no_choice() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        let chat = focus_fixture_message(
            &mut app,
            &ctx,
            Content::Call {
                video: false,
                status: crate::model::CallStatus::Missed,
                duration: 0,
                group: true,
            },
        );
        press_key(&mut app, &ctx, egui::Key::Enter, egui::Modifiers::NONE);
        let popup = crate::ui::conversation::bubble_id(&chat, "key-fixture").with("popup");
        assert!(egui::Popup::is_id_open(&ctx, popup), "the call's menu");
        assert_eq!(
            focused_layer(&ctx).map(|layer| layer.id),
            Some(popup),
            "its row has focus"
        );
        assert_eq!(
            focused_label(&mut app, &ctx).as_deref(),
            Some("Group calls happen on the phone")
        );
    }

    /// Escape or a chosen emoji closes a picker opened by E, and focus goes
    /// back to the list rather than nowhere.
    #[test]
    fn the_reaction_picker_from_e_returns_focus_to_the_list() {
        for key in [egui::Key::Escape, egui::Key::Enter] {
            let mut app = app();
            let ctx = egui::Context::default();
            app.attach(&ctx);
            render(&mut app, &ctx);
            let chat = focus_fixture_message(&mut app, &ctx, Content::text("Fixture react"));
            press_key(&mut app, &ctx, egui::Key::E, egui::Modifiers::NONE);
            assert!(app.reaction_target.is_some());
            press_key(&mut app, &ctx, key, egui::Modifiers::NONE);
            assert!(app.reaction_target.is_none(), "{key:?} closed the picker");
            assert_eq!(
                ctx.memory(|memory| memory.focused()),
                Some(crate::ui::conversation::messages_list_id(&chat)),
                "{key:?}"
            );
        }
    }

    /// A menu opened from the keyboard and then left by a chat switch, not
    /// closed, takes no focus when its chat is opened again: opening a chat
    /// focuses the message field.
    #[test]
    fn a_menu_left_by_a_chat_switch_takes_no_focus_later() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        open_menu_by_keys(&mut app, &ctx);
        let chat = app.open_chat.clone().unwrap();
        press_key(&mut app, &ctx, egui::Key::ArrowDown, egui::Modifiers::ALT);
        assert_ne!(app.open_chat.as_ref(), Some(&chat));
        press_key(&mut app, &ctx, egui::Key::ArrowUp, egui::Modifiers::ALT);
        assert_eq!(app.open_chat.as_ref(), Some(&chat));
        assert!(!egui::Popup::is_any_open(&ctx));
        assert_eq!(
            ctx.memory(|memory| memory.focused()),
            Some(egui::Id::new("composer-text"))
        );
    }

    /// A shortcut that takes focus out of a keyboard-opened menu closes it,
    /// so it does not stay open unseen holding the arrows and Escape.
    #[test]
    fn a_shortcut_out_of_the_message_menu_closes_it() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        open_menu_by_keys(&mut app, &ctx);
        press_key(&mut app, &ctx, egui::Key::L, egui::Modifiers::COMMAND);
        assert!(!egui::Popup::is_any_open(&ctx), "the menu closed");
        assert_eq!(
            ctx.memory(|memory| memory.focused()),
            Some(egui::Id::new("composer-text")),
            "focus stays where the shortcut put it"
        );
    }

    #[test]
    fn the_message_menus_reaction_buttons_have_names() {
        let mut app = app();
        app.settings.reaction_emoji = vec![("🦀".into(), 5)];
        let ctx = egui::Context::default();
        app.attach(&ctx);
        open_menu_by_keys(&mut app, &ctx);
        ctx.enable_accesskit();
        let nodes = accessible_nodes(&mut app, &ctx, Vec::new());
        for name in ["🦀", "React with any emoji"] {
            assert!(
                nodes
                    .iter()
                    .any(|(label, role, _)| label == name
                        && *role == egui::accesskit::Role::Button),
                "no button named {name}"
            );
        }
    }

    /// A menu asked for in one chat and not yet drawn when the reader moves
    /// to another chat does not open when they come back.
    #[test]
    fn a_pending_message_menu_is_dropped_by_a_chat_change() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        let chat = app.open_chat.clone().unwrap();
        let other = sample_ids()
            .into_iter()
            .find(|id| *id != chat)
            .unwrap()
            .to_owned();
        let newest = app.conversations[&chat].messages.last().unwrap().id.clone();
        app.actions
            .push(crate::model::Action::OpenMessageMenu(newest));
        app.actions.push(crate::model::Action::OpenChat(other));
        render(&mut app, &ctx);
        app.actions.push(crate::model::Action::OpenChat(chat));
        render(&mut app, &ctx);
        assert!(!egui::Popup::is_any_open(&ctx));
        assert!(app.open_message_menu.is_none());
    }

    #[test]
    fn announcer_nodes_are_in_every_frame() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        app.announcer
            .say(crate::announcer::Category::Status, "Fixture status");
        let labels = accessible_labels(&mut app, &ctx);
        assert!(
            labels.iter().any(|label| label == "Fixture status"),
            "{labels:?}"
        );
    }

    #[test]
    fn the_lock_screen_draws_no_chat_name_or_message() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        apply_flags(&mut app, Some("app-lock"));
        let labels = accessible_labels(&mut app, &ctx);
        assert!(labels.iter().any(|label| label == "Unlock"), "{labels:?}");
        let leaks = |labels: &[String]| -> Vec<String> {
            SAMPLES
                .iter()
                .flat_map(|sample| {
                    std::iter::once(sample.name).chain(sample.lines.iter().map(|(_, line)| *line))
                })
                .filter(|private| labels.iter().any(|label| label.contains(private)))
                .map(str::to_owned)
                .collect()
        };
        assert_eq!(leaks(&labels), Vec::<String>::new());
        assert!(
            app.copy_rows
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .is_empty(),
            "no message body is drawn for copying"
        );
        // The same app unlocked shows them, so the check above means something.
        app.app_lock.release();
        let labels = accessible_labels(&mut app, &ctx);
        assert!(!leaks(&labels).is_empty());
    }

    /// Lays out each synthetic demo screen and checks that its controls fit the visible surface.
    #[test]
    fn keeping_a_crop_stages_the_cropped_picture_and_remembers_its_original() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        apply_flags(&mut app, Some("crop"));
        render(&mut app, &ctx);
        let edit = app.picture_edit.clone().expect("the cropper is open");
        // A region well inside the picture, so the size proves the crop ran.
        app.picture_edit = Some(crate::model::PictureEdit {
            crop: crate::model::PictureCrop {
                x: 10,
                y: 20,
                width: 40,
                height: 30,
            },
            ..edit
        });
        app.actions.push(crate::model::Action::ApplyPictureEdit);
        render(&mut app, &ctx);
        crate::demo::tour::respond(&mut app);
        let staged = app.pending.first().expect("a picture is staged");
        let path = match &staged.item {
            crate::app::Pending::File(path) => path.clone(),
            _ => panic!("the crop should leave a file staged"),
        };
        assert_eq!(image::image_dimensions(&path).expect("reads"), (40, 30));
        assert!(
            app.picture_origins.contains_key(&staged.id),
            "the original is remembered against that attachment, so cropping again starts over from it"
        );
        assert!(app.picture_edit.is_none(), "keeping the crop closes it");
    }

    #[test]
    fn reopening_a_cropped_pasted_picture_finds_its_pixels_again() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        apply_flags(&mut app, Some("staged"));
        render(&mut app, &ctx);
        let at = app
            .pending
            .iter()
            .position(|staged| matches!(staged.item, crate::app::Pending::Picture { .. }))
            .expect("the demo stages a pasted picture");
        let target = app.pending[at].id;
        let (rgba, width, height) = match &app.pending[at].item {
            crate::app::Pending::Picture {
                width,
                height,
                rgba,
                ..
            } => (rgba.clone(), *width as u32, *height as u32),
            _ => unreachable!(),
        };
        // What keeping a crop leaves behind: a file in the strip, and the
        // pixels it came from remembered against that attachment.
        app.pending[at].item = crate::app::Pending::File("/fixture/edited.jpg".into());
        app.picture_origins.insert(
            target,
            crate::model::PictureOrigin {
                source: crate::model::PictureSource::Pasted(rgba),
                width,
                height,
                crop: crate::model::PictureCrop::full(width, height),
                turns: 0,
            },
        );
        app.actions.push(crate::model::Action::EditPicture(target));
        render(&mut app, &ctx);
        assert!(
            matches!(
                app.picture_edit.as_ref().map(|edit| &edit.source),
                Some(crate::model::PictureSource::Pasted(_))
            ),
            "the cropper opened on the pixels it came from"
        );
        assert!(
            app.picture_texture.is_some(),
            "and found something to draw them with, rather than an empty frame"
        );
    }

    #[test]
    fn the_crop_page_opens_on_the_photo_it_stages() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        // Both pages at once: staged fills the strip before crop adds its own.
        apply_flags(&mut app, Some("staged,crop"));
        render(&mut app, &ctx);
        let edit = app.picture_edit.as_ref().expect("the cropper is open");
        let at = app.staged_at(edit.target).expect("the target is staged");
        let staged = match &app.pending[at].item {
            crate::app::Pending::File(path) => path.clone(),
            _ => panic!("the cropper points at the file it staged"),
        };
        assert!(
            matches!(&edit.source, crate::model::PictureSource::File(path) if *path == staged),
            "the editor and the staged attachment are the same picture"
        );
    }

    /// A file's cropper opens as its size arrives, in the frame that
    /// drains it, while the message field it was asked for from still has
    /// focus: egui only drops it from the next frame on. Enter in that
    /// first frame keeps the crop, as in every frame after it, rather than
    /// reach the field and send the draft with the picture.
    #[test]
    fn enter_in_the_croppers_first_frame_keeps_the_crop() {
        let root = tempfile::Builder::new()
            .prefix("zapfast-demo-")
            .tempdir()
            .unwrap();
        let (mut app, events) = App::headless(AppDirs::under(root.path()), Settings::default());
        ROOTS.with(|roots| roots.borrow_mut().push(root));
        populate(&mut app);
        let ctx = egui::Context::default();
        app.attach(&ctx);
        let (photo, _) = sample_files(&app);
        let (width, height) = image::image_dimensions(&photo).unwrap();
        let target = app.stage(crate::app::Pending::File(photo));
        render(&mut app, &ctx);
        app.composer = "A draft".into();
        let composer = crate::ui::conversation::composer_id();
        ctx.memory_mut(|memory| memory.request_focus(composer));
        render(&mut app, &ctx);
        assert_eq!(ctx.memory(|memory| memory.focused()), Some(composer));
        app.backend.record_demo_commands();
        let frame = |app: &mut App, events: Vec<egui::Event>| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1180.0, 780.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    let ctx = ui.ctx().clone();
                    app.background_frame(&ctx);
                    app.frame_ui(ui);
                },
            );
            output.textures_delta.clear();
        };
        // Asked for with the pointer: focus stays in the field.
        app.actions.push(crate::model::Action::EditPicture(target));
        frame(&mut app, Vec::new());
        assert!(app.picture_edit.is_none(), "a file waits for its size");
        assert_eq!(ctx.memory(|memory| memory.focused()), Some(composer));
        events
            .send(crate::backend::Event::PictureInspected {
                target,
                result: Ok((width, height)),
            })
            .unwrap();
        frame(
            &mut app,
            vec![egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        let commands = app.backend.take_demo_commands();
        assert!(
            !commands.iter().any(|command| matches!(
                command,
                crate::backend::Command::SendText { .. }
                    | crate::backend::Command::SendImage { .. }
                    | crate::backend::Command::SendFiles { .. }
            )),
            "{commands:?}"
        );
        assert_eq!(app.composer, "A draft");
        assert!(app.picture_edit.is_none(), "Enter kept the crop");
    }

    #[test]
    fn direct_delete_controls_use_the_selected_locale() {
        for for_everyone in [false, true] {
            let mut app = app();
            app.locale = crate::i18n::Locale::PortugueseBrazil;
            app.dialog = Some(crate::model::Dialog::ConfirmDeleteMessage {
                chat: SAMPLES[0].id.to_owned(),
                message: "fixture".into(),
                for_everyone,
            });
            let ctx = egui::Context::default();
            app.attach(&ctx);
            let labels = accessible_labels(&mut app, &ctx);
            for expected in [
                if for_everyone {
                    "Apagar para todos?"
                } else {
                    "Apagar para mim?"
                },
                "Apagar",
                "Cancelar",
            ] {
                assert!(labels.iter().any(|label| label == expected), "{labels:?}");
            }
        }
    }

    #[test]
    fn deletion_dialog_distinguishes_direct_messages_from_selected_batches() {
        for for_everyone in [false, true] {
            for count in [0, 1, 2] {
                let mut app = app();
                let chat = SAMPLES[0].id.to_owned();
                app.dialog = Some(if count == 0 {
                    crate::model::Dialog::ConfirmDeleteMessage {
                        chat,
                        message: "fixture".into(),
                        for_everyone,
                    }
                } else {
                    crate::model::Dialog::ConfirmDeleteSelection {
                        chat,
                        messages: (0..count).map(|i| format!("fixture-{i}")).collect(),
                        for_everyone,
                    }
                });
                let ctx = egui::Context::default();
                app.attach(&ctx);
                let labels = accessible_labels(&mut app, &ctx);
                let text = labels.join("\n");
                if count == 0 {
                    assert!(!text.contains("selected message"), "{text}");
                } else {
                    assert!(
                        text.contains(&format!("{count} selected message")),
                        "{text}"
                    );
                    if count == 2 {
                        assert!(text.contains("selected messages"), "{text}");
                    }
                }
            }
        }
    }

    #[test]
    /// Checks all synthetic demo surfaces fit their tested window sizes and themes.
    fn every_surface_lays_out() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        for id in sample_ids() {
            apply_flags(&mut app, Some(id));
            render(&mut app, &ctx);
        }
        for page in [
            "chat-menu",
            "chat-header-menu",
            "channel",
            "meta-ai",
            "locked",
            "locked-open",
            "locked-prompt",
            "locked-setup",
            "app-lock",
            "app-lock-wrong",
            "app-lock-forgot",
            "app-lock-settings",
            "app-lock-setup",
            "new-chat",
            "new-group",
            "new-group-name",
            "new-group-creating",
            "new-contact-number",
            "carousel-viewer",
            "media-album",
            "media-album-before",
            "media-album-oversized",
            "media-album-oversized,light",
            "media-album-midnight",
            "media-album-midnight,light",
            "unnamed-group",
            "keyring",
            "interactive",
            "interactive-actions",
            "interactive-media",
            "interactive-list",
            "interactive-list-dialog",
            "carousel",
            "carousel-pair",
            "poll-empty",
            "poll-voted",
            "poll-results",
            "message-info",
            "message-info-unknown",
            "message-info-partial",
            "message-info-direct",
            "motion",
            "motion-playing",
            "motion-preview",
            "motion-viewer",
            "video",
            "video-playing",
            "video-expanded",
            "shared-contact",
            "note-playing",
            "empty",
            "rtl",
            "disappearing",
            "disappearing-direct",
            "disappearing-off",
            "disappearing,group-info",
            "disappearing-direct,info",
            "disappearing-off,info",
            "settings",
            "settings-search=Notifications",
            "settings-search=System",
            "settings-search=Storage",
            "clear-media",
            "wallpaper",
            "wallpaper,light",
            "wallpaper,wallpaper-image",
            "wallpaper,wallpaper-image,light",
            "wallpaper,theme=Nord.json",
            "wallpaper-image",
            "update",
            "update-downloading",
            "update-ready",
            "update-failed",
            "update-managed",
            "themes",
            "settings,omarchy",
            "settings,omarchy-light",
            "poll",
            "poll-create",
            "theme=Catppuccin.json",
            "theme=Catppuccin Latte.json",
            "theme=Nord.json",
            "theme=Ristretto.json",
            "theme=Rose Pine.json",
            "theme=Rose Pine Moon.json",
            "theme=Rose Pine Dawn.json",
            "theme=Tokyo Night.json",
            "shortcuts",
            "about",
            "failed",
            "info",
            "info-media",
            "info-docs",
            "info-links",
            "group-info",
            "group-info-rename",
            "group-info-locked",
            "group-info-saving",
            "group-admin",
            "group-admin-menu",
            "group-permissions",
            "group-add-members",
            "group-reset-link",
            "group-remove-member",
            "forward",
            "forward-selected",
            "forward-review",
            "forward-full",
            "forward-once",
            "unlink",
            "leave-group",
            "left-group",
            "leave-channel",
            "toasts",
            "delete-chat",
            "clear-chat",
            "invite",
            "unread-divider",
            "quotes",
            "quote-jump",
            "select",
            "delete-message",
            "delete-message-mine",
            "remove-pack",
            "delete-selection",
            "select-delete",
            "new-contact",
            "accounts",
            "accounts,light",
            "accounts-closed",
            "accounts-muted",
            "accounts-muted,light",
            "account-menu",
            "light",
            "archived",
            "chat-search",
            "chat-search-day",
            "chat-search-rtl",
            "unread",
            "private",
            "favorites",
            "groups",
            "offline",
            "syncing",
            "picker",
            "stickers",
            "sticker-favorites",
            "sticker-received",
            "sticker-pack",
            "sticker-search",
            "sticker-animated",
            "sticker-add",
            "sticker-view",
            "sticker-pack-message",
            "sticker-maker",
            "sticker-pack-view",
            "typing",
            "composer-tools",
            "mention",
            "mention-everyone",
            "emoji-complete",
            "reply",
            "private-reply",
            "private-reply,light",
            "private-reply-menu",
            "private-reply-menu,light",
            "private-reply-locked-source",
            "private-reply-locked-source,light",
            "edit",
            "unsent-voice",
            "scrolled",
            "reactions",
            "typers",
            "nosidebar",
            "wide",
            "rail",
            "search",
            "forward-portuguese",
            "staged",
            "view-once-off",
            "view-once",
            "view-once,light",
            "attachment-draft-away",
            "attachment-draft-return",
            "crop",
            "compose-emoji",
            "voice",
            "voice,voice-menu",
            "recording",
            "preview",
            "gifs",
            "gifs-badkey",
            "react-menu",
            "starred",
            "star-menu",
            "react-picker",
            "react-picker-empty",
            "react-custom",
            "react-other",
            "calls",
            "calls-chat",
            "calls-info",
            "calls-missed",
            "calls-missed-none",
            "new-call",
            "call-entries",
            "call-bubbles",
            "call-bubbles,light",
            "call",
            "call-active",
            "call-outgoing",
            "call-connecting",
            "call-failed",
            "call-devices",
            "call-ended",
            "call-strip",
            "call-group",
            "call-group-active",
            "call-group-video",
            "call-group-muted",
            "call-group-calling",
            "call-group-on-phone",
            "call-link",
            "calls-link",
            "call-video",
            "call-video,light",
            "call-video-incoming",
            "call-video-off",
            "pinned",
        ] {
            let mut app = self::app();
            apply_flags(&mut app, Some(page));
            render(&mut app, &ctx);
        }
        for page in ["login", "pair", "phone"] {
            let mut app = self::app();
            apply_flags(&mut app, Some(page));
            render(&mut app, &ctx);
            assert!(!app.is_linked());
        }
    }

    /// Where fastframe-titlebar draws the caption buttons for a footprint
    /// in a window `width` wide: GNOME's round ones from 14 to 42 points
    /// down, or Windows' square ones centred in the header's height.
    fn caption_rects(footprint: fastframe_titlebar::Footprint, width: f32) -> Vec<egui::Rect> {
        let linux = footprint.height < crate::theme::TOP_BAR_HEIGHT;
        let (top, bottom) = if linux {
            (0.0, 42.0)
        } else {
            (0.0, (footprint.height - 36.0) / 2.0 + 2.0 + 36.0)
        };
        let mut rects = Vec::new();
        if footprint.left > 0.0 {
            rects.push(egui::Rect::from_x_y_ranges(
                0.0..=footprint.left,
                top..=bottom,
            ));
        }
        if footprint.right > 0.0 {
            rects.push(egui::Rect::from_x_y_ranges(
                width - footprint.right..=width,
                top..=bottom,
            ));
        }
        rects
    }

    /// The clickable widgets of the last frame that lie under `rects`,
    /// leaving out what catches clicks across a whole screen or panel (the
    /// call screen's and the viewer's backdrops).
    fn widgets_under(ctx: &egui::Context, rects: &[egui::Rect]) -> Vec<(egui::Id, egui::Rect)> {
        let window = ctx.content_rect();
        ctx.viewport(|viewport| {
            viewport
                .prev_pass
                .widgets
                .layers()
                .flat_map(|(_, widgets)| widgets.iter())
                .filter(|widget| {
                    widget.enabled
                        && widget.visible
                        && widget.sense.senses_click()
                        && widget.interact_rect.is_positive()
                        && widget.interact_rect.height() < window.height() / 2.0
                        && widget.interact_rect.width() < window.width() * 0.9
                        && rects
                            .iter()
                            .any(|rect| rect.intersects(widget.interact_rect.shrink(0.5)))
                })
                .map(|widget| (widget.id, widget.interact_rect))
                .collect()
        })
    }

    /// Lays out a few frames of a window `size` points big.
    fn render_sized(app: &mut App, ctx: &egui::Context, size: egui::Vec2) {
        for _ in 0..4 {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                    ..Default::default()
                },
                |ui| {
                    let ctx = ui.ctx().clone();
                    app.background_frame(&ctx);
                    app.frame_ui(ui);
                },
            );
            output.textures_delta.clear();
        }
    }

    /// With the title bar ZapFast draws on Windows and Linux, no header
    /// control lies under the caption buttons, at either end of the top edge
    /// and in each layout: GNOME's buttons at the right or the left, and
    /// Windows' taller row. Headers that reach a corner leave the buttons
    /// room beside their own controls, or move them down when narrow.
    ///
    /// Not on macOS, which never draws them: its headers keep the traffic
    /// lights' layout under any preview, and drag the window from their top
    /// (fastframe-titlebar's drag area, which senses clicks) wherever the
    /// caption buttons would stand.
    #[test]
    #[cfg_attr(
        target_os = "macos",
        ignore = "macOS has the traffic lights, never the drawn caption buttons"
    )]
    fn header_controls_keep_clear_of_the_drawn_caption_buttons() {
        use fastframe_titlebar::Footprint;
        let footprints = [
            // GNOME elsewhere than GNOME: minimize, maximize and close.
            Footprint {
                left: 0.0,
                right: 3.0 * 36.0 + 14.0,
                height: 41.0,
            },
            // A button layout on the left, such as `close,minimize,maximize:`.
            Footprint {
                left: 3.0 * 36.0 + 14.0,
                right: 0.0,
                height: 41.0,
            },
            // Windows: three square buttons centred in the header's height.
            Footprint {
                left: 0.0,
                right: 3.0 * 36.0 + 5.0,
                height: crate::theme::TOP_BAR_HEIGHT,
            },
        ];
        let wide = egui::vec2(1180.0, 780.0);
        let narrow = egui::vec2(480.0, 720.0);
        let pages = [
            ("chat", wide),
            ("info", wide),
            ("chat-search", wide),
            ("empty", wide),
            ("chat", narrow),
            ("info", narrow),
            ("empty", narrow),
            ("chat", egui::vec2(700.0, 600.0)),
            ("call-active", wide),
            ("call", wide),
            ("call-active", narrow),
            ("call-group", wide),
            ("call-group-active", wide),
            ("call-group-active", narrow),
            ("call-group-video", wide),
            ("call-group-video", narrow),
            ("call-group-muted", wide),
            ("call-group-calling", wide),
            ("call-group-on-phone", wide),
            ("call-group-on-phone", narrow),
            ("calls-link", wide),
            ("settings", wide),
            ("settings", narrow),
            ("calls", wide),
            ("calls-info", wide),
            ("calls-info", narrow),
            // The rail beside the list, and alone beside a hidden list.
            ("nosidebar", wide),
            ("settings,nosidebar", wide),
            ("empty,nosidebar", wide),
            ("calls,nosidebar", wide),
            ("archived", wide),
            ("archived,nosidebar", wide),
            ("locked-open", wide),
            ("app-lock", wide),
            ("login", wide),
            ("offline", wide),
            ("offline,empty", wide),
            ("offline,settings", wide),
            ("preview", wide),
        ];
        for footprint in footprints {
            for (page, size) in pages {
                let mut app = self::app();
                apply_flags(&mut app, Some(page));
                let ctx = egui::Context::default();
                app.attach(&ctx);
                crate::theme::preview_titlebar(&ctx, footprint);
                render_sized(&mut app, &ctx, size);
                let under = widgets_under(&ctx, &caption_rects(footprint, size.x));
                assert!(
                    under.is_empty(),
                    "{page} at {size:?} with {footprint:?}: {under:?} under the caption buttons"
                );
            }
        }
        // Without the room, the chat header's own buttons stand where the
        // caption buttons go: the check above sees them.
        let mut app = self::app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render_sized(&mut app, &ctx, wide);
        assert!(!widgets_under(&ctx, &caption_rects(footprints[0], wide.x)).is_empty());
    }

    /// A header shares the caption row only at a corner it reaches: beside
    /// the buttons when its content fits, under them otherwise, and not at
    /// all below the row or away from the edge.
    #[test]
    fn a_header_makes_room_only_at_the_corner_it_reaches() {
        use crate::ui::{CaptionRoom, caption_room};
        let ctx = egui::Context::default();
        crate::theme::preview_titlebar(
            &ctx,
            fastframe_titlebar::Footprint {
                left: 50.0,
                right: 122.0,
                height: 41.0,
            },
        );
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1000.0, 700.0),
                )),
                ..Default::default()
            },
            |ui| {
                let ctx = ui.ctx();
                let rect = |x0: f32, y0: f32, x1: f32| {
                    egui::Rect::from_min_max(egui::pos2(x0, y0), egui::pos2(x1, y0 + 44.0))
                };
                // The conversation header, 14 points in from the right edge.
                assert_eq!(
                    caption_room(ctx, rect(334.0, 8.0, 986.0), 320.0),
                    CaptionRoom {
                        left: 0.0,
                        right: 108.0,
                        top: 0.0
                    }
                );
                // The chat list at the left, with the left-hand buttons.
                assert_eq!(
                    caption_room(ctx, rect(14.0, 8.0, 306.0), 200.0),
                    CaptionRoom {
                        left: 36.0,
                        right: 0.0,
                        top: 0.0
                    }
                );
                // Too narrow for its content beside both ends: under them.
                assert_eq!(
                    caption_room(ctx, rect(14.0, 8.0, 986.0), 900.0),
                    CaptionRoom {
                        left: 0.0,
                        right: 0.0,
                        top: 33.0
                    }
                );
                // Below the caption row, or between the corners: nothing.
                assert_eq!(
                    caption_room(ctx, rect(334.0, 60.0, 986.0), 320.0),
                    CaptionRoom::default()
                );
                assert_eq!(
                    caption_room(ctx, rect(334.0, 8.0, 700.0), 320.0),
                    CaptionRoom::default()
                );
            },
        );
        output.textures_delta.clear();
    }

    /// The drawn title bar's room goes in full screen, where the buttons go.
    #[test]
    fn the_drawn_caption_buttons_take_no_room_in_full_screen() {
        let ctx = egui::Context::default();
        crate::theme::preview_titlebar(
            &ctx,
            fastframe_titlebar::Footprint {
                left: 0.0,
                right: 122.0,
                height: 41.0,
            },
        );
        assert!(crate::theme::drawn_titlebar(&ctx));
        assert!(crate::theme::chrome_in_headers(&ctx));
        let mut input = egui::RawInput::default();
        input
            .viewports
            .get_mut(&egui::ViewportId::ROOT)
            .unwrap()
            .fullscreen = Some(true);
        let mut output = ctx.run_ui(input, |ui| {
            assert_eq!(
                crate::theme::caption_footprint(ui.ctx()),
                fastframe_titlebar::Footprint::default()
            );
            let room = crate::ui::caption_room(ui.ctx(), ui.max_rect(), 0.0);
            assert_eq!(room, crate::ui::CaptionRoom::default());
        });
        output.textures_delta.clear();
    }

    #[test]
    /// Checks header layout at different zoom levels with the sidebar shown or hidden.
    fn macos_headers_fit_when_zoomed_with_and_without_the_sidebar() {
        for zoom in [0.6, 1.0, 2.0] {
            for page in [
                "history-date",
                "chat",
                "nosidebar",
                "settings",
                "settings,nosidebar",
                "empty,nosidebar",
                "rail",
                "settings,rail",
                "empty,rail",
                "archived",
                "offline",
                "login",
            ] {
                let mut app = self::app();
                app.settings.zoom = zoom;
                apply_flags(&mut app, Some(page));
                let ctx = egui::Context::default();
                app.attach(&ctx);
                crate::theme::preview_macos(&ctx);
                render(&mut app, &ctx);
                assert!(
                    (crate::theme::traffic_light_inset(&ctx) * ctx.zoom_factor() - 84.0).abs()
                        < 0.01
                );
                let mut input = egui::RawInput::default();
                input
                    .viewports
                    .get_mut(&egui::ViewportId::ROOT)
                    .unwrap()
                    .fullscreen = Some(true);
                let mut output = ctx.run_ui(input, |ui| {
                    assert_eq!(crate::theme::traffic_light_inset(ui.ctx()), 0.0);
                    app.frame_ui(ui);
                });
                output.textures_delta.clear();
            }
        }
    }

    /// Runs one frame with input events.
    pub(super) fn frame_with(app: &mut App, ctx: &egui::Context, events: Vec<egui::Event>) {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1180.0, 780.0),
                )),
                events,
                ..Default::default()
            },
            |ui| {
                let ctx = ui.ctx().clone();
                app.background_frame(&ctx);
                app.frame_ui(ui);
            },
        );
        output.textures_delta.clear();
    }

    /// Runs a copy input and follow-up frame, collecting clipboard writes.
    fn copied_text(app: &mut App, ctx: &egui::Context, event: egui::Event) -> Vec<String> {
        let mut copied = Vec::new();
        for events in [vec![event], Vec::new()] {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1180.0, 780.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    let ctx = ui.ctx().clone();
                    app.background_frame(&ctx);
                    app.frame_ui(ui);
                },
            );
            output.textures_delta.clear();
            copied.extend(
                output
                    .platform_output
                    .commands
                    .into_iter()
                    .filter_map(|command| match command {
                        egui::OutputCommand::CopyText(text) => Some(text),
                        _ => None,
                    }),
            );
        }
        copied
    }

    /// Exports only selected messages in chat order, including off-screen text.
    #[test]
    fn selected_messages_copy_in_chat_order_including_offscreen_text() {
        let (mut app, ctx, chat) = sweep_chat(400);
        let conversation = app.conversations.get_mut(&chat).unwrap();
        conversation.messages[1].content = Content::text("*First* 👨‍👩‍👧‍👦\nsecond line");
        conversation.messages[399].content = Content::text("Last café 🇹🇷");
        app.selection = Some((chat.clone(), vec!["m399".into(), "m001".into()]));
        render(&mut app, &ctx);
        assert!(
            ctx.data(|data| data.get_temp::<egui::Rect>(
                crate::ui::conversation::bubble_id(&chat, "m001").with("body")
            ))
            .is_none(),
            "the first selected message is outside the virtualized viewport"
        );
        let copied = copied_text(&mut app, &ctx, egui::Event::Copy);
        assert_eq!(copied.len(), 1, "one clipboard write per copy command");
        let text = &copied[0];
        assert!(text.starts_with('['), "uses transcript sharing headers");
        assert!(text.contains("First 👨‍👩‍👧‍👦\nsecond line\n["), "{text}");
        assert!(text.ends_with("Last café 🇹🇷"), "{text}");
        assert!(
            !text.contains("message number"),
            "unselected rows are excluded"
        );
        assert_eq!(selected_ids(&app), ["m399", "m001"], "copy keeps selection");
    }

    /// Copy belongs to egui when the selected IDs or conversation are gone;
    /// available messages consume native and raw shortcuts with one write.
    #[test]
    fn selected_messages_consume_copy_only_when_loaded_text_exists() {
        for missing in [None, Some("messages"), Some("conversation")] {
            for copy in [
                vec![egui::Event::Copy],
                vec![key(egui::Key::C, egui::Modifiers::COMMAND)],
                vec![key(egui::Key::C, egui::Modifiers::CTRL)],
                vec![
                    egui::Event::Copy,
                    key(egui::Key::C, egui::Modifiers::COMMAND),
                ],
            ] {
                let (mut app, ctx, chat) = sweep_chat(2);
                let id = if missing == Some("messages") {
                    "deleted-message"
                } else {
                    "m001"
                };
                app.selection = Some((chat.clone(), vec![id.into()]));
                if missing == Some("conversation") {
                    app.conversations.remove(&chat);
                }
                let mut events = copy;
                events.push(egui::Event::Text("unrelated input".into()));
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        events: events.clone(),
                        ..Default::default()
                    },
                    |ui| {
                        crate::ui::keys::handle(&mut app, ui.ctx());
                        ui.ctx().input(|input| {
                            if missing.is_some() {
                                assert_eq!(input.events, events, "{missing:?}");
                            } else {
                                assert_eq!(input.events, events[events.len() - 1..]);
                            }
                        });
                    },
                );
                output.textures_delta.clear();
                if missing.is_some() {
                    assert!(app.actions.is_empty(), "no selected text to copy");
                } else {
                    assert!(matches!(
                        app.actions.as_slice(),
                        [crate::model::Action::CopyText(text)]
                            if text.ends_with("message number 1")
                    ));
                }
            }
        }
    }

    /// Whole-message exports resolve mentions and retain captions and media labels.
    #[test]
    fn selected_messages_copy_mentions_captions_and_media_labels() {
        let mut app = app();
        let chat = sample_ids()[0].to_owned();
        let conversation = app.conversations.get_mut(&chat).unwrap();
        let doc = conversation
            .messages
            .iter_mut()
            .find(|m| m.id == "ada-doc")
            .unwrap();
        if let Content::Document { caption, .. } = &mut doc.content {
            *caption = Some("*Notes* for @12345 🎉".into());
        }
        doc.mentions = vec![crate::model::MentionRef {
            user: "12345".into(),
            id: chat.clone(),
        }];
        app.selection = Some((chat, vec!["ada-voice".into(), "ada-doc".into()]));
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        let copied = copied_text(&mut app, &ctx, egui::Event::Copy);
        assert_eq!(copied.len(), 1);
        assert!(
            copied[0].contains("[document: Notes on the Engine.pdf] Notes for @Ada Lovelace 🎉"),
            "{}",
            copied[0]
        );
        assert!(
            copied[0].ends_with("[voice message, 0:42]"),
            "{}",
            copied[0]
        );
    }

    /// Visible text-field selections retain their normal clipboard handling.
    #[test]
    fn selected_messages_leave_copy_available_to_focused_search_text() {
        let (mut app, ctx, chat) = sweep_chat(2);
        app.selection = Some((chat, vec!["m001".into()]));
        app.search = "Ada".into();
        app.actions.push(crate::model::Action::FocusSearch);
        render(&mut app, &ctx);
        assert!(ctx.text_edit_focused());
        frame_with(
            &mut app,
            &ctx,
            vec![key(egui::Key::A, egui::Modifiers::COMMAND)],
        );
        let copied = copied_text(&mut app, &ctx, egui::Event::Copy);
        assert_eq!(copied, ["Ada"], "copy belongs to the focused text field");
    }

    /// Dialogs and another chat must not export selected messages.
    #[test]
    fn selected_messages_do_not_copy_through_dialogs_or_other_chats() {
        let (mut app, ctx, chat) = sweep_chat(2);
        app.selection = Some((chat, vec!["m001".into()]));
        app.dialog = Some(crate::model::Dialog::Shortcuts);
        render(&mut app, &ctx);
        assert!(copied_text(&mut app, &ctx, egui::Event::Copy).is_empty());
        app.dialog = None;
        app.selection = Some(("another-chat".into(), vec!["m001".into()]));
        render(&mut app, &ctx);
        assert!(copied_text(&mut app, &ctx, egui::Event::Copy).is_empty());
    }

    /// Pasted transcript text in other messages must not add duplicate headers.
    #[test]
    fn selected_messages_copy_does_not_annotate_a_transcript_twice() {
        let (mut app, ctx, chat) = sweep_chat(4);
        let name = app.display_name(&chat);
        let first = format!("[{}] {name}: first", crate::util::copy_stamp(1_700_000_000));
        let second = format!(
            "[{}] {name}: second",
            crate::util::copy_stamp(1_700_000_001)
        );
        // Unselected messages contain a previously pasted transcript. The
        // text-selection annotator must not mistake our export for their text.
        app.conversations.get_mut(&chat).unwrap().messages = [
            Content::text("first"),
            Content::text("second"),
            Content::text(&first),
            Content::text(&second),
        ]
        .into_iter()
        .enumerate()
        .map(|(i, content)| {
            message(
                &chat,
                &format!("m{i:03}"),
                false,
                1_700_000_000 + i as i64,
                content,
            )
        })
        .collect();
        app.selection = Some((chat, vec!["m000".into(), "m001".into()]));
        render(&mut app, &ctx);
        assert_eq!(
            copied_text(&mut app, &ctx, egui::Event::Copy),
            [format!("{first}\n{second}")]
        );
    }

    /// A newly hidden composer's stale focus must not delay copying the selection.
    #[test]
    fn selected_messages_copy_immediately_after_hiding_a_focused_composer() {
        let (mut app, ctx, _) = sweep_chat(2);
        app.composer = "unsent draft".into();
        app.actions.push(crate::model::Action::FocusComposer);
        render(&mut app, &ctx);
        assert!(ctx.text_edit_focused());
        app.actions
            .push(crate::model::Action::SelectMessage("m001".into()));
        let copied = copied_text(&mut app, &ctx, egui::Event::Copy);
        assert_eq!(copied.len(), 1, "a hidden composer must not swallow Copy");
        assert!(copied[0].ends_with("message number 1"));
        assert_eq!(app.composer, "unsent draft");
    }

    #[test]
    /// Checks recipient selection and review both use the localized singular forwarding heading.
    fn singular_forward_headings_use_the_forwarding_translation_in_both_steps() {
        for page in ["forward", "forward-review"] {
            let mut app = app();
            app.locale = crate::i18n::Locale::PortugueseBrazil;
            apply_flags(&mut app, Some(page));
            let ctx = egui::Context::default();
            app.attach(&ctx);
            let labels = accessible_labels(&mut app, &ctx);
            assert!(
                labels.iter().any(|label| label == "Encaminhar mensagem"),
                "{page}: {labels:?}"
            );
            if page == "forward-review" {
                assert!(labels.iter().any(|label| label == "Voltar"), "{labels:?}");
                assert!(
                    labels.iter().any(|label| label == "Encaminhar"),
                    "{labels:?}"
                );
            }
        }
    }

    #[test]
    /// Measures actual modal, list, close and footer rectangles, without sending.
    fn forward_layout_fits_small_viewports_and_preserves_profile_search() {
        for size in [
            egui::vec2(360.0, 240.0),
            egui::vec2(360.0, 600.0),
            egui::vec2(1180.0, 780.0),
        ] {
            for page in [
                "forward",
                "forward-selected",
                "forward-once",
                "forward-full",
                "forward-review",
            ] {
                let mut app = app();
                app.backend.record_demo_commands();
                let ctx = egui::Context::default();
                app.attach(&ctx);
                app.locale = crate::i18n::Locale::PortugueseBrazil;
                apply_flags(&mut app, Some(page));
                let selected = app.forward_recipients.clone();
                let limit = app.forward_limit;
                for i in 0..12 {
                    app.chats.push(Chat::new(
                        format!("{}@s.whatsapp.net", 100 + i),
                        "Fixture".into(),
                    ));
                }
                render_sized(&mut app, &ctx, size);
                let viewport = ctx.content_rect();
                for part in ["modal", "footer", "list", "close"] {
                    let rect = ctx
                        .data(|data| {
                            data.get_temp::<egui::Rect>(egui::Id::new(("forward-layout", part)))
                        })
                        .expect(part);
                    assert!(
                        rect.is_positive() && viewport.expand(1.0).contains_rect(rect),
                        "{page} {size:?} {part}: {rect:?} outside {viewport:?}"
                    );
                }
                let list = ctx
                    .data(|data| {
                        data.get_temp::<egui::Rect>(egui::Id::new(("forward-layout", "list")))
                    })
                    .unwrap();
                let footer = ctx
                    .data(|data| {
                        data.get_temp::<egui::Rect>(egui::Id::new(("forward-layout", "footer")))
                    })
                    .unwrap();
                assert!(
                    list.bottom() <= footer.top() + 1.0,
                    "list overlaps footer: {page} {size:?}"
                );
                assert!(
                    list.height() >= 16.0,
                    "recipient list collapsed: {page} {size:?}"
                );
                assert_eq!(app.forward_recipients, selected);
                assert_eq!(app.forward_limit, limit);
                assert!(
                    !app.backend
                        .take_demo_commands()
                        .iter()
                        .any(|command| matches!(command, crate::backend::Command::Forward { .. }))
                );
            }
        }
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        apply_flags(&mut app, Some("forward"));
        let id = SAMPLES[0].id;
        app.contacts.insert(
            id.into(),
            Contact {
                id: id.into(),
                push_name: Some("Profile Fixture".into()),
                ..Contact::default()
            },
        );
        app.forward_search = "profile fixture".into();
        assert_eq!(app.chat_title(app.chat(id).unwrap()), "~Profile Fixture");
        render_sized(&mut app, &ctx, egui::vec2(360.0, 240.0));
        assert!(
            ctx.data(|data| data.get_temp::<egui::Rect>(egui::Id::new(("forward-recipient", id))))
                .is_some()
        );
    }

    #[test]
    fn forward_uses_large_windows_and_keeps_room_when_search_changes() {
        for size in [egui::vec2(1180.0, 780.0), egui::vec2(1920.0, 1080.0)] {
            for page in [
                "forward",
                "forward-selected",
                "forward-full",
                "forward-review",
            ] {
                for search in ["", "Ada", "no-fixture-matches"] {
                    let mut app = app();
                    app.backend.record_demo_commands();
                    let ctx = egui::Context::default();
                    app.attach(&ctx);
                    apply_flags(&mut app, Some(page));
                    app.forward_search = search.into();
                    let selected = app.forward_recipients.clone();
                    render_sized(&mut app, &ctx, size);
                    let part = |name| {
                        ctx.data(|data| {
                            data.get_temp::<egui::Rect>(egui::Id::new(("forward-layout", name)))
                        })
                        .unwrap()
                    };
                    let modal = part("modal");
                    let list = part("list");
                    let footer = part("footer");
                    let viewport = ctx.content_rect();
                    assert!(viewport.expand(1.0).contains_rect(modal));
                    assert!(
                        modal.width() >= (size.x * 0.55).min(700.0),
                        "modal remains narrow on a large window: {size:?} {page} {search:?}: {modal:?}"
                    );
                    assert!(
                        modal.height() >= (size.y * 0.75).min(760.0),
                        "modal remains short on a large window: {size:?} {page}: {modal:?}"
                    );
                    assert!(
                        (0.0..=12.0).contains(&(footer.top() - list.bottom())),
                        "recipient list shrinks instead of filling the modal: {size:?} {page} {search:?}: {list:?} {footer:?}"
                    );
                    assert_eq!(app.forward_recipients, selected);
                    assert!(!app.backend.take_demo_commands().iter().any(|command| {
                        matches!(command, crate::backend::Command::Forward { .. })
                    }));
                }
            }
        }
    }

    #[test]
    /// Checks real row and checkbox clicks toggle recipients without dispatching a forward command.
    fn forward_row_and_checkbox_clicks_select_without_sending() {
        for light in [false, true] {
            let mut app = app();
            app.backend.record_demo_commands();
            let ctx = egui::Context::default();
            ctx.enable_accesskit();
            app.attach(&ctx);
            apply_flags(
                &mut app,
                Some(if light { "forward,light" } else { "forward" }),
            );
            for _ in 0..3 {
                render(&mut app, &ctx);
            }
            app.backend.take_demo_commands();
            let nodes = accessible_nodes(&mut app, &ctx, Vec::new());
            let title = app.chat_title(app.chat(SAMPLES[0].id).unwrap());
            assert!(
                nodes.iter().filter(|(label, _, _)| label == &title).count() >= 2,
                "both row and checkbox have accessible labels"
            );
            let id = egui::Id::new(("forward-recipient", SAMPLES[0].id));
            let rect = ctx.data(|data| data.get_temp::<egui::Rect>(id)).unwrap();
            for pos in [
                rect.center(),
                egui::pos2(rect.right() - 18.0, rect.center().y),
            ] {
                for pressed in [true, false] {
                    frame_with(
                        &mut app,
                        &ctx,
                        vec![
                            egui::Event::PointerMoved(pos),
                            egui::Event::PointerButton {
                                pos,
                                button: egui::PointerButton::Primary,
                                pressed,
                                modifiers: egui::Modifiers::NONE,
                            },
                        ],
                    );
                }
                assert!(app.dialog.is_some());
                assert!(
                    !app.backend
                        .take_demo_commands()
                        .iter()
                        .any(|command| matches!(command, crate::backend::Command::Forward { .. }))
                );
                if pos == rect.center() {
                    assert_eq!(app.forward_recipients, [SAMPLES[0].id]);
                } else {
                    assert!(
                        app.forward_recipients.is_empty(),
                        "checkbox toggles exactly once"
                    );
                }
            }
        }
    }

    /// Presses and releases the primary button at `pos`.
    fn press_at(app: &mut App, ctx: &egui::Context, pos: egui::Pos2) {
        // Twice: the tooltip of the control the pointer left goes first.
        frame_with(app, ctx, vec![egui::Event::PointerMoved(pos)]);
        frame_with(app, ctx, Vec::new());
        for pressed in [true, false] {
            frame_with(
                app,
                ctx,
                vec![egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
        }
        render(app, ctx);
    }

    /// Clicks the accessible node labelled `label` with the role `role`.
    fn press_labelled(
        app: &mut App,
        ctx: &egui::Context,
        label: &str,
        role: egui::accesskit::Role,
    ) {
        let nodes = accessible_nodes(app, ctx, Vec::new());
        let pos = nodes
            .iter()
            .find(|(name, found, _)| name == label && *found == role)
            .map(|(_, _, pos)| *pos)
            .unwrap_or_else(|| panic!("no {role:?} {label:?} in {nodes:?}"));
        press_at(app, ctx, pos);
    }

    fn new_group_dialog() -> (App, egui::Context) {
        let mut app = app();
        app.backend.record_demo_commands();
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        app.attach(&ctx);
        apply_flags(&mut app, Some("new-chat"));
        // Let the dialog's opening animation finish.
        for _ in 0..8 {
            render(&mut app, &ctx);
        }
        (app, ctx)
    }

    fn press_member(app: &mut App, ctx: &egui::Context, id: &str) {
        // Only this frame's rows: one scrolled away keeps an old rect.
        let ids: Vec<_> = SAMPLES
            .iter()
            .map(|sample| crate::ui::new_group::member_row_id(sample.id))
            .collect();
        ctx.data_mut(|data| {
            for row in &ids {
                data.remove::<egui::Rect>(*row);
            }
        });
        // The dialog re-centres over a few frames when its height changes.
        for _ in 0..4 {
            frame_with(app, ctx, Vec::new());
        }
        let rect = ctx
            .data(|data| data.get_temp::<egui::Rect>(crate::ui::new_group::member_row_id(id)))
            .unwrap_or_else(|| panic!("{id} has no row"));
        press_at(app, ctx, rect.center());
    }

    /// New chat's New group row gives the contacts check boxes; picked people
    /// become chips, Next asks for the name, and Create sends the group.
    #[test]
    fn a_group_is_chosen_named_and_created_from_new_chat() {
        use crate::new_group::Stage;
        use egui::accesskit::Role;
        let (mut app, ctx) = new_group_dialog();
        let labels = accessible_labels(&mut app, &ctx);
        for row in ["New group", "Add contact"] {
            assert!(labels.iter().any(|label| label == row), "{labels:?}");
        }
        press_labelled(&mut app, &ctx, "New group", Role::Button);
        assert_eq!(
            app.new_group.as_ref().map(|draft| draft.stage),
            Some(Stage::Choosing)
        );
        assert_eq!(app.dialog, Some(Dialog::NewChat), "the same dialog");

        press_member(&mut app, &ctx, SAMPLES[0].id);
        press_member(&mut app, &ctx, SAMPLES[3].id);
        assert_eq!(
            app.new_group.as_ref().unwrap().members,
            [SAMPLES[0].id, SAMPLES[3].id]
        );
        // The chip lets its person go again.
        let chip = format!("Remove {}", SAMPLES[3].name);
        press_labelled(&mut app, &ctx, &chip, Role::Button);
        assert_eq!(app.new_group.as_ref().unwrap().members, [SAMPLES[0].id]);

        press_labelled(&mut app, &ctx, "Next", Role::Button);
        assert_eq!(app.new_group.as_ref().unwrap().stage, Stage::Naming);
        render(&mut app, &ctx);
        assert_eq!(
            ctx.memory(|memory| memory.focused()),
            Some(crate::ui::new_group::name_field_id()),
            "the name field takes the keyboard"
        );
        frame_with(&mut app, &ctx, vec![egui::Event::Text("Book club".into())]);
        assert_eq!(app.new_group.as_ref().unwrap().name, "Book club");
        app.backend.take_demo_commands();
        press_labelled(&mut app, &ctx, "Create", Role::Button);
        let sent: Vec<_> = app
            .backend
            .take_demo_commands()
            .into_iter()
            .filter_map(|command| match command {
                crate::backend::Command::CreateGroup {
                    subject, members, ..
                } => Some((subject, members)),
                _ => None,
            })
            .collect();
        assert_eq!(
            sent,
            [("Book club".to_owned(), vec![SAMPLES[0].id.to_owned()])]
        );
        assert!(app.new_group.as_ref().unwrap().creating);
    }

    /// Back steps out one stage at a time, unchecking everyone leaves the
    /// choice, and Escape closes the dialog as before.
    #[test]
    fn back_unchecking_and_escape_leave_the_new_group() {
        use crate::new_group::Stage;
        use egui::accesskit::Role;
        let (mut app, ctx) = new_group_dialog();
        press_labelled(&mut app, &ctx, "New group", Role::Button);
        press_member(&mut app, &ctx, SAMPLES[0].id);
        press_member(&mut app, &ctx, SAMPLES[0].id);
        assert!(app.new_group.is_none(), "nobody picked: the plain list");
        assert_eq!(app.dialog, Some(Dialog::NewChat));

        press_labelled(&mut app, &ctx, "New group", Role::Button);
        press_member(&mut app, &ctx, SAMPLES[0].id);
        press_labelled(&mut app, &ctx, "Next", Role::Button);
        // The pill, not the header's arrow: both say Back.
        let nodes = accessible_nodes(&mut app, &ctx, Vec::new());
        let back = nodes
            .iter()
            .filter(|(label, role, _)| label == "Back" && *role == Role::Button)
            .map(|(_, _, pos)| *pos)
            .max_by(|a, b| a.y.total_cmp(&b.y))
            .unwrap();
        press_at(&mut app, &ctx, back);
        assert_eq!(app.new_group.as_ref().unwrap().stage, Stage::Choosing);
        assert_eq!(app.new_group.as_ref().unwrap().members, [SAMPLES[0].id]);
        // The header's arrow, beside the heading.
        let nodes = accessible_nodes(&mut app, &ctx, Vec::new());
        let heading = nodes
            .iter()
            .find(|(label, _, _)| label == "Add group members")
            .map(|(_, _, pos)| *pos)
            .unwrap();
        let arrow = nodes
            .iter()
            .filter(|(label, role, _)| label == "Back" && *role == Role::Button)
            .map(|(_, _, pos)| *pos)
            .min_by(|a, b| a.distance(heading).total_cmp(&b.distance(heading)))
            .unwrap();
        press_at(&mut app, &ctx, arrow);
        assert!(app.new_group.is_none());

        press_labelled(&mut app, &ctx, "New group", Role::Button);
        frame_with(
            &mut app,
            &ctx,
            vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        render(&mut app, &ctx);
        assert_eq!(app.dialog, None);
        assert!(app.new_group.is_none());
    }

    /// The choosing step's search narrows the list, and checking someone
    /// clears it for the next name.
    #[test]
    fn checking_someone_clears_the_search_for_the_next_name() {
        let (mut app, ctx) = new_group_dialog();
        press_labelled(&mut app, &ctx, "New group", egui::accesskit::Role::Button);
        frame_with(&mut app, &ctx, vec![egui::Event::Text("katherine".into())]);
        render(&mut app, &ctx);
        assert_eq!(app.new_chat_search, "katherine");
        let listed = |ctx: &egui::Context, id: &str| {
            ctx.data(|data| data.get_temp::<egui::Rect>(crate::ui::new_group::member_row_id(id)))
        };
        ctx.data_mut(|data| {
            data.remove::<egui::Rect>(crate::ui::new_group::member_row_id(SAMPLES[0].id))
        });
        render(&mut app, &ctx);
        assert!(listed(&ctx, SAMPLES[0].id).is_none(), "Ada is not a match");
        press_member(&mut app, &ctx, SAMPLES[3].id);
        assert!(app.new_chat_search.is_empty());
        assert_eq!(app.new_group.as_ref().unwrap().members, [SAMPLES[3].id]);
    }

    /// Lets an eased key scroll's animation settle: tests advance time by the
    /// predicted ~1/60s per frame, and the animation lasts up to 0.3s.
    fn settle_key_scroll(app: &mut App, ctx: &egui::Context) {
        for _ in 0..10 {
            render(app, ctx);
        }
    }

    /// Resting the pointer on a chat row's cut-short preview shows the whole
    /// last message, sender first in a group; a preview that already fits
    /// shows nothing more, and neither does a row showing typing or a row
    /// whose menu is open.
    #[test]
    fn hovering_a_cut_short_preview_shows_the_whole_last_message() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        let group = SAMPLES[4].id;
        let direct = SAMPLES[2].id;
        // The sample has someone typing here, which the row shows instead.
        let typing = SAMPLES[1].id;
        let long = "The venue moved to the courtyard because the hall is being painted, \
                    bring a jacket since it gets cold after sunset and the tail marker";
        let sender = "4930111222333@s.whatsapp.net";
        for chat in app.chats.iter_mut() {
            if chat.id == group || chat.id == typing {
                chat.last = Some(crate::model::LastMessage {
                    labelled: false,
                    from_me: false,
                    sender: sender.into(),
                    sender_name: Some("Linus Example".into()),
                    summary: long.into(),
                    full: long.into(),
                    status: Delivery::Read,
                });
            } else if chat.id == direct {
                chat.last = Some(crate::model::LastMessage {
                    labelled: false,
                    from_me: false,
                    sender: direct.into(),
                    sender_name: None,
                    summary: "Short one".into(),
                    full: "Short one".into(),
                    status: Delivery::Read,
                });
            }
        }
        let name = app.display_name_or(sender, Some("Linus Example"));
        let prefix = format!("{}: ", name.split_whitespace().next().unwrap());
        render(&mut app, &ctx);
        let long_area = ctx
            .read_response(crate::ui::chats::preview_id(group))
            .expect("a cut-short preview registers its hover area")
            .rect;
        assert!(
            ctx.read_response(crate::ui::chats::preview_id(direct))
                .is_none(),
            "a preview that fits registers none"
        );
        assert!(
            ctx.read_response(crate::ui::chats::preview_id(typing))
                .is_none(),
            "a row showing typing offers no tooltip"
        );
        // Rests the pointer on the preview line of `chat` past the tooltip
        // delay and returns every text painted in the last frame.
        let clock = std::cell::Cell::new(10.0);
        let rest_on = |app: &mut App, chat: &str| -> Vec<String> {
            let index = |id: &str| app.chats.iter().position(|row| row.id == id).unwrap();
            let offset = (index(chat) as f32 - index(group) as f32) * crate::theme::ROW_HEIGHT;
            let pos = long_area.left_center() + egui::vec2(20.0, offset);
            let mut shapes = Vec::new();
            for step in 0..6 {
                clock.set(clock.get() + 0.3);
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(1180.0, 780.0),
                        )),
                        time: Some(clock.get()),
                        events: if step == 0 {
                            vec![egui::Event::PointerMoved(pos)]
                        } else {
                            Vec::new()
                        },
                        ..Default::default()
                    },
                    |ui| {
                        let ctx = ui.ctx().clone();
                        app.background_frame(&ctx);
                        app.frame_ui(ui);
                    },
                );
                output.textures_delta.clear();
                shapes = output.shapes;
            }
            shapes
                .into_iter()
                .filter_map(|clipped| match clipped.shape {
                    egui::Shape::Text(text) => Some(text.galley.text().to_owned()),
                    _ => None,
                })
                .collect()
        };

        // A galley keeps its whole text however much of it shows, so the
        // row's own preview counts once and the tooltip adds a second.
        let count = |texts: &[String], needle: &str| {
            texts.iter().filter(|text| text.contains(needle)).count()
        };
        let texts = rest_on(&mut app, group);
        assert!(
            texts
                .iter()
                .any(|text| text.starts_with(&prefix) && text.contains("tail marker")),
            "the tooltip shows the whole message after the group sender: {texts:?}"
        );

        let texts = rest_on(&mut app, direct);
        assert_eq!(
            count(&texts, "Short one"),
            1,
            "a preview that fits shows no tooltip"
        );
        assert_eq!(
            count(&texts, "tail marker"),
            1,
            "only the group row shows it"
        );

        app.open_chat_menu = Some(group.into());
        let texts = rest_on(&mut app, group);
        assert_eq!(
            count(&texts, "tail marker"),
            1,
            "an open menu hides the tooltip"
        );
    }

    fn key(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }
    }

    /// The panel says when a contact was last seen as the chat header does:
    /// with the day and the time, in the reader's language.
    #[test]
    fn the_panel_says_when_a_contact_was_last_seen_as_the_header_does() {
        fn texts(shape: &egui::Shape, out: &mut Vec<String>) {
            match shape {
                egui::Shape::Text(text) => out.push(text.galley.text().to_owned()),
                egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| texts(shape, out)),
                _ => {}
            }
        }
        let mut app = app();
        apply_flags(&mut app, Some("info"));
        let chat = app.open_chat.clone().expect("a chat");
        let seen = jiff::Timestamp::now().as_second() - 86_400;
        app.presence.insert(
            chat,
            crate::app::Presence {
                online: false,
                last_seen: Some(seen),
            },
        );
        let ctx = egui::Context::default();
        app.attach(&ctx);
        let mut shapes = Vec::new();
        for _ in 0..3 {
            shapes = frame_sized(&mut app, &ctx, 780.0, Vec::new());
        }
        let mut drawn = Vec::new();
        for clipped in &shapes {
            texts(&clipped.shape, &mut drawn);
        }
        let line = crate::util::last_seen(app.locale, seen);
        assert_eq!(
            drawn.iter().filter(|text| **text == line).count(),
            2,
            "the header and the panel both say {line:?}"
        );
    }

    /// The info pages open the panel on the sample chat, listed from the
    /// sample's own messages: the overview, and each tab of the media view,
    /// with something to show on every one.
    #[test]
    fn info_pages_open_the_panel_on_each_view_with_content() {
        use crate::model::{InfoView, MediaTab};
        for (page, view) in [
            ("info", InfoView::Overview),
            ("info-media", InfoView::Media(MediaTab::Media)),
            ("info-docs", InfoView::Media(MediaTab::Docs)),
            ("info-links", InfoView::Media(MediaTab::Links)),
        ] {
            let mut app = app();
            apply_flags(&mut app, Some(page));
            let info = app
                .info
                .as_ref()
                .unwrap_or_else(|| panic!("{page} opens the panel"));
            assert_eq!(info.view, view, "{page}");
            assert_eq!(info.chat, app.open_chat.clone().expect("a chat"), "{page}");
            let listing = app.info_media.as_ref().expect("the sample is listed");
            assert!(listing.media.len() >= 3, "{page}: pictures and a video");
            assert!(
                listing
                    .media
                    .iter()
                    .any(|row| matches!(row.content, Content::Video { .. })),
                "{page}: a video"
            );
            assert!(!listing.docs.is_empty(), "{page}: a document");
            assert!(!listing.links.is_empty(), "{page}: a link");
            let ctx = egui::Context::default();
            app.attach(&ctx);
            render(&mut app, &ctx);
            assert_eq!(
                ctx.data(|data| data.get_temp::<bool>(crate::ui::info::docked_id())),
                Some(true),
                "{page}: the panel is drawn beside the chat"
            );
        }
        // The group has media of its own to show.
        let mut app = app();
        apply_flags(&mut app, Some("group-info"));
        let listing = app.info_media.as_ref().expect("the group is listed");
        assert!(!listing.media.is_empty(), "the group's photo");
    }

    /// The group dialog offers the pencil and the photo menu only when we may
    /// change the group's info, and not while a change is on its way.
    #[test]
    fn group_info_offers_editing_only_when_allowed() {
        use crate::ui::info::{group_name_button_id, group_photo_id};
        for (page, offered) in [
            ("group-info", true),
            ("group-info-locked", false),
            ("group-info-saving", false),
        ] {
            let mut app = app();
            let ctx = egui::Context::default();
            app.attach(&ctx);
            apply_flags(&mut app, Some(page));
            render(&mut app, &ctx);
            let drawn = |id| ctx.data(|data| data.get_temp::<egui::Rect>(id)).is_some();
            assert_eq!(drawn(group_name_button_id()), offered, "{page}: the pencil");
            assert_eq!(drawn(group_photo_id()), offered, "{page}: the photo menu");
        }
    }

    /// The panel offers each part of running a group to whoever may use it:
    /// everything to an admin, adding and the link to members of a group
    /// that lets them, and nothing to members of one that does not.
    #[test]
    fn group_management_is_offered_by_role() {
        use crate::ui::info::{
            add_members_row_id, invite_link_id, member_menu_id, permissions_row_id, reset_link_id,
        };
        for (page, permissions, add, link, reset, menus) in [
            ("group-admin", true, true, true, true, true),
            ("group-info", false, true, true, false, false),
            ("group-info-locked", false, false, false, false, false),
        ] {
            let mut app = app();
            let ctx = egui::Context::default();
            app.attach(&ctx);
            apply_flags(&mut app, Some(page));
            if page != "group-admin" {
                // The link the member's panel asks for, as WhatsApp answers.
                let chat = app.open_chat.clone().unwrap();
                app.info_invite = Some((
                    chat,
                    crate::group_admin::LinkState::Known(
                        crate::group_admin::InviteLink::new(DEMO_INVITE_LINK.into()).unwrap(),
                    ),
                ));
            }
            render(&mut app, &ctx);
            let drawn = |id| ctx.data(|data| data.get_temp::<egui::Rect>(id)).is_some();
            assert_eq!(
                drawn(permissions_row_id()),
                permissions,
                "{page}: permissions"
            );
            assert_eq!(drawn(add_members_row_id()), add, "{page}: add members");
            assert_eq!(drawn(invite_link_id()), link, "{page}: the link");
            assert_eq!(drawn(reset_link_id()), reset, "{page}: reset");
            assert_eq!(drawn(member_menu_id(JONAS)), menus, "{page}: member menus");
            assert!(
                !drawn(member_menu_id(ME)),
                "{page}: never a menu for ourselves"
            );
        }
    }

    /// The panel asks for the link once, and a member of a group that keeps
    /// it for admins never asks.
    #[test]
    fn the_invite_link_is_asked_for_once_and_only_when_shared() {
        use crate::backend::Command;
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        app.backend.record_demo_commands();
        apply_flags(&mut app, Some("group-admin"));
        app.info_invite = None;
        render(&mut app, &ctx);
        render(&mut app, &ctx);
        let asked = app
            .backend
            .take_demo_commands()
            .into_iter()
            .filter(|command| {
                matches!(command, Command::GroupInviteLink { chat, reset: false } if chat == FAMILY)
            })
            .count();
        assert_eq!(asked, 1);
        assert_eq!(
            app.info_invite,
            Some((FAMILY.to_owned(), crate::group_admin::LinkState::Loading))
        );

        let mut app = self::app();
        app.backend.record_demo_commands();
        apply_flags(&mut app, Some("group-info-locked"));
        app.attach(&ctx);
        let chat = app.open_chat.clone().unwrap();
        app.actions
            .push(crate::model::Action::FetchInviteLink(chat));
        render(&mut app, &ctx);
        assert!(
            !app.backend
                .take_demo_commands()
                .iter()
                .any(|command| matches!(command, Command::GroupInviteLink { .. })),
            "the group keeps its link for admins"
        );
    }

    /// Copy link puts the link on the clipboard and says so; Reset link asks
    /// first, then revokes it.
    #[test]
    fn the_invite_link_is_copied_and_reset_from_the_panel() {
        use crate::backend::Command;
        use crate::ui::info::{copy_link_id, reset_link_id};
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        app.backend.record_demo_commands();
        apply_flags(&mut app, Some("group-admin"));
        render(&mut app, &ctx);
        let mut copied = Vec::new();
        let mut click =
            |app: &mut App, id: egui::Id| {
                let pos = ctx
                    .data(|data| data.get_temp::<egui::Rect>(id))
                    .expect("the control is on screen")
                    .center();
                let press = |pressed| egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                };
                for events in [
                    vec![egui::Event::PointerMoved(pos), press(true)],
                    vec![press(false)],
                    Vec::new(),
                ] {
                    let mut output = ctx.run_ui(
                        egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(
                                egui::Pos2::ZERO,
                                egui::vec2(1180.0, 780.0),
                            )),
                            events,
                            ..Default::default()
                        },
                        |ui| {
                            let ctx = ui.ctx().clone();
                            app.background_frame(&ctx);
                            app.frame_ui(ui);
                        },
                    );
                    output.textures_delta.clear();
                    copied.extend(output.platform_output.commands.into_iter().filter_map(
                        |command| match command {
                            egui::OutputCommand::CopyText(text) => Some(text),
                            _ => None,
                        },
                    ));
                }
            };
        click(&mut app, copy_link_id());
        click(&mut app, reset_link_id());
        assert_eq!(copied, [DEMO_INVITE_LINK]);
        assert!(
            app.toasts
                .iter()
                .any(|toast| toast.message == "Invite link copied"),
            "the copy is announced"
        );
        assert_eq!(
            app.dialog,
            Some(Dialog::ConfirmResetInviteLink(FAMILY.to_owned())),
            "a reset asks first"
        );
        assert!(
            !app.backend
                .take_demo_commands()
                .iter()
                .any(|command| matches!(command, Command::GroupInviteLink { reset: true, .. })),
            "nothing is revoked before the answer"
        );
        app.actions
            .push(crate::model::Action::ResetInviteLink(FAMILY.to_owned()));
        render(&mut app, &ctx);
        assert!(app.dialog.is_none());
        assert!(
            app.backend
                .take_demo_commands()
                .iter()
                .any(|command| matches!(
                    command,
                    Command::GroupInviteLink { chat, reset: true } if chat == FAMILY
                ))
        );
    }

    /// Turning a permission's switch asks WhatsApp; the switch itself waits
    /// for the answer. Every one of WhatsApp's four is there.
    #[test]
    fn a_permission_switch_sends_the_change() {
        use crate::backend::Command;
        use crate::group_admin::Permission;
        use crate::ui::info::permission_switch_id;
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        app.backend.record_demo_commands();
        apply_flags(&mut app, Some("group-permissions"));
        render(&mut app, &ctx);
        for permission in Permission::ALL {
            assert!(
                ctx.data(|data| data.get_temp::<egui::Rect>(permission_switch_id(permission)))
                    .is_some(),
                "{permission:?} is drawn"
            );
        }
        let pos = ctx
            .data(|data| {
                data.get_temp::<egui::Rect>(permission_switch_id(Permission::SendMessages))
            })
            .unwrap()
            .center();
        let press = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame_with(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(pos), press(true)],
        );
        frame_with(&mut app, &ctx, vec![press(false)]);
        render(&mut app, &ctx);
        let sent = app.backend.take_demo_commands();
        assert!(
            sent.iter().any(|command| matches!(
                command,
                Command::SetGroupPermission {
                    chat,
                    permission: Permission::SendMessages,
                    members: false,
                } if chat == FAMILY
            )),
            "only admins may send now: {sent:?}"
        );
        assert_eq!(
            Permission::SendMessages.value(app.chat(FAMILY).unwrap()),
            Some(true),
            "nothing changes before WhatsApp accepts it"
        );
    }

    /// Add members offers only people outside the group, and Add sends the
    /// picked ones.
    #[test]
    fn add_members_offers_outsiders_and_sends_the_picked() {
        use crate::backend::Command;
        use crate::ui::new_group::member_row_id;
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        app.backend.record_demo_commands();
        apply_flags(&mut app, Some("group-add-members"));
        render(&mut app, &ctx);
        let drawn = |id: &str| {
            ctx.data(|data| data.get_temp::<egui::Rect>(member_row_id(id)))
                .is_some()
        };
        assert!(drawn(DOROTHY), "someone outside the group");
        assert!(!drawn(JONAS), "a member is not offered");
        app.actions.push(crate::model::Action::AddGroupMembers);
        render(&mut app, &ctx);
        assert!(app.backend.take_demo_commands().iter().any(|command| matches!(
            command,
            Command::AddGroupMembers { chat, members } if chat == FAMILY && members == &[DOROTHY.to_owned()]
        )));
        assert!(app.new_group.as_ref().is_some_and(|draft| draft.creating));
    }

    /// The member menu offers what the member may be made: Mira is an admin,
    /// Jonas is not. Remove asks first.
    #[test]
    fn the_member_menu_follows_each_members_role() {
        use crate::group_admin::{MemberChange, member_changes};
        fn texts(shape: &egui::Shape, out: &mut Vec<String>) {
            match shape {
                egui::Shape::Text(text) => out.push(text.galley.text().to_owned()),
                egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| texts(shape, out)),
                _ => {}
            }
        }
        let mut app = app();
        apply_flags(&mut app, Some("group-admin-menu"));
        let chat = app.chat(FAMILY).unwrap().clone();
        let mira = "491702222222@s.whatsapp.net";
        assert_eq!(
            member_changes(&chat, JONAS, &[ME]),
            [MemberChange::Promote, MemberChange::Remove]
        );
        assert_eq!(
            member_changes(&chat, mira, &[ME]),
            [MemberChange::Demote, MemberChange::Remove]
        );
        let ctx = egui::Context::default();
        app.attach(&ctx);
        let shapes = (0..3)
            .map(|_| frame_sized(&mut app, &ctx, 780.0, Vec::new()))
            .last()
            .unwrap();
        let mut drawn = Vec::new();
        for clipped in &shapes {
            texts(&clipped.shape, &mut drawn);
        }
        for label in ["Make group admin", "Remove from group", "Group admin"] {
            assert!(drawn.iter().any(|text| text == label), "{label}: {drawn:?}");
        }
    }

    /// The timer line says what WhatsApp says: a duration for a chat with one,
    /// "Off" for a chat the phone turned it off for, and nothing at all for a
    /// chat nobody has mentioned a timer for, which includes a frame where the
    /// row was drawn a moment ago and the chat changed under it.
    #[test]
    fn chat_info_says_the_timer_and_hides_what_it_does_not_know() {
        use crate::ui::dialogs::disappearing_timer_id;
        for (page, expected) in [
            (
                "disappearing,group-info",
                Some("Disappearing messages: 24 hours"),
            ),
            (
                "disappearing-direct,info",
                Some("Disappearing messages: 7 days"),
            ),
            ("disappearing-off,info", Some("Disappearing messages: Off")),
            ("group-info", None),
            ("info", None),
        ] {
            let mut app = app();
            let ctx = egui::Context::default();
            app.attach(&ctx);
            apply_flags(&mut app, Some(page));
            render(&mut app, &ctx);
            let drawn = ctx
                .data(|data| {
                    data.get_temp::<Option<crate::ui::dialogs::TimerRow>>(disappearing_timer_id())
                })
                .flatten();
            assert_eq!(
                drawn.as_ref().map(|row| row.text.as_str()),
                expected,
                "{page}: the timer row"
            );
            let Some(row) = drawn else { continue };
            // The row stays inside the dialog, whatever a translation measures.
            let width = ctx.content_rect().width();
            assert!(
                row.rect.width() <= width,
                "{page}: {} wider than the window",
                row.rect.width()
            );
            assert!(row.rect.height() > 0.0, "{page}: drawn with no height");
        }
    }

    /// A chat whose timer is not known draws no row, even in the same context
    /// that just drew one: the marker records each frame, so the last frame's
    /// row cannot be read back as this frame's.
    #[test]
    fn the_timer_row_does_not_outlive_its_chat() {
        use crate::ui::dialogs::disappearing_timer_id;
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        apply_flags(&mut app, Some("disappearing-off,info"));
        render(&mut app, &ctx);
        let drawn = |ctx: &egui::Context| {
            ctx.data(|data| {
                data.get_temp::<Option<crate::ui::dialogs::TimerRow>>(disappearing_timer_id())
            })
            .flatten()
        };
        assert_eq!(
            drawn(&ctx).map(|row| row.text),
            Some("Disappearing messages: Off".to_owned())
        );

        // The same panel, now a chat whose timer nobody has mentioned.
        app.open_chat = Some(SAMPLES[3].id.to_owned());
        info_sample(&mut app, SAMPLES[3].id, InfoView::Overview);
        render(&mut app, &ctx);

        assert!(
            drawn(&ctx).is_none(),
            "the previous chat's row is still marked"
        );
    }

    #[test]
    fn group_description_is_shown_in_group_info() {
        let mut app = app();
        apply_flags(&mut app, Some("group-info"));
        let ctx = egui::Context::default();
        app.attach(&ctx);
        let mut shapes = Vec::new();
        for _ in 0..3 {
            shapes = frame_sized(&mut app, &ctx, 800.0, Vec::new());
        }
        fn has_heading(shape: &egui::Shape) -> bool {
            match shape {
                egui::Shape::Text(text) => text.galley.text() == "Description",
                egui::Shape::Vec(shapes) => shapes.iter().any(has_heading),
                _ => false,
            }
        }
        assert!(shapes.iter().any(|shape| has_heading(&shape.shape)));
    }

    #[test]
    fn group_description_layout_handles_absent_and_long_text_in_both_themes() {
        fn description_shape(shape: &egui::Shape) -> Option<&egui::epaint::TextShape> {
            match shape {
                egui::Shape::Text(text)
                    if text.galley.text().starts_with("Description fixture") =>
                {
                    Some(text)
                }
                egui::Shape::Vec(shapes) => shapes.iter().find_map(description_shape),
                _ => None,
            }
        }
        fn has_heading(shape: &egui::Shape) -> bool {
            match shape {
                egui::Shape::Text(text) => text.galley.text() == "Description",
                egui::Shape::Vec(shapes) => shapes.iter().any(has_heading),
                _ => false,
            }
        }
        let long = (0..40)
            .map(|line| format!("Description fixture line {line} 🦀"))
            .collect::<Vec<_>>()
            .join("\n");
        for page in ["group-info", "group-info,light"] {
            for (width, height) in [(1280.0, 800.0), (900.0, 600.0)] {
                for description in [
                    None,
                    Some(""),
                    Some("Description fixture first 🦀\nSecond line\nThird line"),
                    Some(long.as_str()),
                ] {
                    let mut app = app();
                    apply_flags(&mut app, Some(page));
                    app.chats
                        .iter_mut()
                        .find(|chat| chat.id == SAMPLES[1].id)
                        .unwrap()
                        .group_description = description.map(str::to_owned);
                    let ctx = egui::Context::default();
                    app.attach(&ctx);
                    let mut shapes = Vec::new();
                    for _ in 0..3 {
                        let mut output = ctx.run_ui(
                            egui::RawInput {
                                screen_rect: Some(egui::Rect::from_min_size(
                                    egui::Pos2::ZERO,
                                    egui::vec2(width, height),
                                )),
                                ..Default::default()
                            },
                            |ui| {
                                app.background_frame(ui.ctx());
                                app.frame_ui(ui);
                            },
                        );
                        shapes = std::mem::take(&mut output.shapes);
                        output.textures_delta.clear();
                    }
                    assert_eq!(
                        shapes.iter().any(|shape| has_heading(&shape.shape)),
                        description.is_some_and(|text| !text.is_empty()),
                        "{page}, {width}x{height}"
                    );
                    if description.is_some_and(|text| !text.is_empty()) {
                        fn footer(shape: &egui::Shape) -> Option<&egui::epaint::TextShape> {
                            match shape {
                                egui::Shape::Text(text) if text.galley.text() == "Archive" => {
                                    Some(text)
                                }
                                egui::Shape::Vec(shapes) => shapes.iter().find_map(footer),
                                _ => None,
                            }
                        }
                        let footer = shapes
                            .iter()
                            .find_map(|shape| footer(&shape.shape))
                            .expect("the panel actions are drawn");
                        assert!(
                            footer.pos.y >= 0.0
                                && footer.pos.y + footer.galley.rect.max.y <= height,
                            "the panel actions stay on screen: {page}, {width}x{height}"
                        );
                        let (clipped, text) = shapes
                            .iter()
                            .find_map(|clipped| {
                                description_shape(&clipped.shape).map(|text| (clipped, text))
                            })
                            .expect("the description is drawn");
                        if description == Some(long.as_str()) {
                            assert!(
                                text.galley.text().contains("line 39"),
                                "all lines remain available to scroll"
                            );
                            assert!(text.galley.rows.len() >= 40);
                        }
                        assert!(
                            clipped.clip_rect.height() <= 121.0,
                            "description scroll area stays bounded"
                        );
                        assert!(
                            clipped.clip_rect.min.y >= 0.0 && clipped.clip_rect.max.y <= height
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn group_description_can_be_selected_and_copied_with_line_breaks_and_emoji() {
        let mut app = app();
        apply_flags(&mut app, Some("group-info"));
        let description = "Description fixture 🦀\nSecond line";
        app.chats
            .iter_mut()
            .find(|chat| chat.id == SAMPLES[1].id)
            .unwrap()
            .group_description = Some(description.into());
        let ctx = egui::Context::default();
        app.attach(&ctx);
        let draw = |app: &mut App, events| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1280.0, 800.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    app.background_frame(ui.ctx());
                    app.frame_ui(ui);
                },
            );
            output.textures_delta.clear();
            output
        };
        let mut output = draw(&mut app, Vec::new());
        for _ in 0..2 {
            output = draw(&mut app, Vec::new());
        }
        fn find(shape: &egui::Shape) -> Option<&egui::epaint::TextShape> {
            match shape {
                egui::Shape::Text(text)
                    if text.galley.text().starts_with("Description fixture") =>
                {
                    Some(text)
                }
                egui::Shape::Vec(shapes) => shapes.iter().find_map(find),
                _ => None,
            }
        }
        let text = output
            .shapes
            .iter()
            .find_map(|shape| find(&shape.shape))
            .expect("description label");
        let start = text.pos
            + text
                .galley
                .rows
                .first()
                .unwrap()
                .rect()
                .left_center()
                .to_vec2()
            + egui::vec2(1.0, 0.0);
        let end = text.pos
            + text
                .galley
                .rows
                .last()
                .unwrap()
                .rect()
                .right_center()
                .to_vec2()
            + egui::vec2(4.0, 0.0);
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        draw(
            &mut app,
            vec![egui::Event::PointerMoved(start), button(start, true)],
        );
        draw(&mut app, vec![egui::Event::PointerMoved(end)]);
        draw(&mut app, vec![button(end, false)]);
        let copied = draw(&mut app, vec![egui::Event::Copy]);
        assert!(copied.platform_output.commands.iter().any(
            |command| matches!(command, egui::OutputCommand::CopyText(text) if text == description)
        ));
    }

    /// The pencil opens the name editor; Enter renames the group on WhatsApp,
    /// an unchanged name sends nothing, and Escape cancels without closing the
    /// dialog. The photo opens its menu.
    #[test]
    fn a_group_is_renamed_from_its_info_dialog() {
        use crate::backend::Command;
        use crate::ui::info::{group_name_button_id, group_photo_id};
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        app.backend.record_demo_commands();
        apply_flags(&mut app, Some("group-info"));
        render(&mut app, &ctx);
        let group = SAMPLES[1].id;
        let click = |app: &mut App, id: egui::Id| {
            let pos = ctx
                .data(|data| data.get_temp::<egui::Rect>(id))
                .expect("the control is on screen")
                .center();
            let press = |pressed| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            frame_with(app, &ctx, vec![egui::Event::PointerMoved(pos), press(true)]);
            frame_with(app, &ctx, vec![press(false)]);
            render(app, &ctx);
        };
        let enter = |app: &mut App| {
            frame_with(
                app,
                &ctx,
                vec![key(egui::Key::Enter, egui::Modifiers::NONE)],
            );
            render(app, &ctx);
            app.backend.take_demo_commands()
        };

        click(&mut app, group_name_button_id());
        assert_eq!(app.group_name_edit.as_deref(), Some("Rust Berlin"));
        // Unchanged: the editor closes and nothing is sent.
        let sent = enter(&mut app);
        assert!(app.group_name_edit.is_none());
        assert!(
            !sent
                .iter()
                .any(|command| matches!(command, Command::SetGroupName { .. })),
            "an unchanged name is not sent"
        );

        click(&mut app, group_name_button_id());
        app.group_name_edit = Some("  Rust Berlin meetups ".into());
        render(&mut app, &ctx);
        let sent = enter(&mut app);
        assert!(
            sent.iter().any(|command| matches!(command,
                Command::SetGroupName { chat, name }
                    if chat == group && name == "Rust Berlin meetups")),
            "Enter renames the group, trimmed"
        );
        assert!(app.group_name_edit.is_none());
        assert_eq!(
            app.chat(group).map(|chat| chat.name.as_str()),
            Some("Rust Berlin"),
            "the name waits for WhatsApp to accept it"
        );

        click(&mut app, group_name_button_id());
        assert!(app.group_name_edit.is_some());
        frame_with(
            &mut app,
            &ctx,
            vec![key(egui::Key::Escape, egui::Modifiers::NONE)],
        );
        render(&mut app, &ctx);
        assert!(app.group_name_edit.is_none(), "Escape cancels the rename");
        assert!(app.info.is_some(), "and keeps the panel");

        click(&mut app, group_photo_id());
        assert!(egui::Popup::is_any_open(&ctx), "the photo opens its menu");
        app.actions
            .push(crate::model::Action::RemoveGroupPicture(group.into()));
        render(&mut app, &ctx);
        assert!(
            app.backend
                .take_demo_commands()
                .iter()
                .any(|command| matches!(
                    command,
                    Command::SetGroupPicture { chat, jpeg: None } if chat == group
                )),
            "Remove photo asks WhatsApp to remove it"
        );
    }

    #[test]
    fn the_speed_chip_cycles_and_the_menu_offers_every_speed() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        // The speed row only appears for a downloaded clip, so give the open
        // sample chat one voice message that carries a media path.
        let chat = sample_ids()[0].to_owned();
        let mut clip = media("audio/ogg; codecs=opus", 12_000, None, None);
        clip.path = Some(std::path::PathBuf::from("demo/voice.ogg"));
        app.conversations.get_mut(&chat).unwrap().messages = vec![message(
            &chat,
            "voice-speed",
            false,
            100,
            Content::Audio {
                media: clip,
                seconds: Some(5),
                voice_note: true,
                waveform: demo_waveform(),
            },
        )];
        render(&mut app, &ctx);
        let click = |app: &mut App, id: egui::Id, button: egui::PointerButton| {
            let pos = ctx
                .data(|data| data.get_temp::<egui::Rect>(id))
                .expect("the speed control is on screen")
                .center();
            let press = |pressed| egui::Event::PointerButton {
                pos,
                button,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            frame_with(app, &ctx, vec![egui::Event::PointerMoved(pos), press(true)]);
            frame_with(app, &ctx, vec![press(false)]);
            // Draw once more so a menu opened by the click is laid out.
            frame_with(app, &ctx, Vec::new());
        };
        let chip = crate::ui::conversation::speed_chip_id(&chat, "voice-speed");
        // The chip cycles 1x, 1.5x, and 2x, as on the phone.
        for expected in [1.5, 2.0, 1.0] {
            click(&mut app, chip, egui::PointerButton::Primary);
            assert_eq!(app.player.speed(), expected);
            assert_eq!(app.settings.voice_speed, expected);
        }
        // Right-clicking it opens the message menu, which lists every speed.
        for option in crate::audio::SPEEDS.into_iter().rev() {
            click(&mut app, chip, egui::PointerButton::Secondary);
            let choice = crate::ui::conversation::speed_button_id(&chat, "voice-speed", option);
            click(&mut app, choice, egui::PointerButton::Primary);
            assert_eq!(
                app.settings.voice_speed, option,
                "choosing {option}x from the menu reaches App and settings"
            );
        }
    }

    /// In a chat with its own speed, the chip and the message menu change
    /// that chat's speed and leave the one in Settings alone.
    #[test]
    fn a_chat_with_its_own_speed_changes_only_its_own() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        app.settings.voice_speed = 2.0;
        let chat = sample_ids()[0].to_owned();
        app.chat_mut(&chat).unwrap().voice_speed = Some(1.0);
        let mut clip = media("audio/ogg; codecs=opus", 12_000, None, None);
        clip.path = Some(std::path::PathBuf::from("demo/voice.ogg"));
        app.conversations.get_mut(&chat).unwrap().messages = vec![message(
            &chat,
            "voice-speed",
            false,
            100,
            Content::Audio {
                media: clip,
                seconds: Some(5),
                voice_note: true,
                waveform: demo_waveform(),
            },
        )];
        render(&mut app, &ctx);
        let click = |app: &mut App, id: egui::Id, button: egui::PointerButton| {
            let pos = ctx
                .data(|data| data.get_temp::<egui::Rect>(id))
                .expect("the speed control is on screen")
                .center();
            let press = |pressed| egui::Event::PointerButton {
                pos,
                button,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            frame_with(app, &ctx, vec![egui::Event::PointerMoved(pos), press(true)]);
            frame_with(app, &ctx, vec![press(false)]);
            frame_with(app, &ctx, Vec::new());
        };
        // The chip starts from the chat's 1x, not the 2x in Settings.
        let chip = crate::ui::conversation::speed_chip_id(&chat, "voice-speed");
        click(&mut app, chip, egui::PointerButton::Primary);
        assert_eq!(app.chat(&chat).unwrap().voice_speed, Some(1.5));
        assert_eq!(app.player.speed(), 1.5);
        assert_eq!(app.settings.voice_speed, 2.0);
        click(&mut app, chip, egui::PointerButton::Secondary);
        let choice = crate::ui::conversation::speed_button_id(&chat, "voice-speed", 1.25);
        click(&mut app, choice, egui::PointerButton::Primary);
        assert_eq!(app.chat(&chat).unwrap().voice_speed, Some(1.25));
        assert_eq!(app.settings.voice_speed, 2.0);
    }

    /// A refused voice message waits above its own chat's composer, where
    /// Enter in the empty composer sends it again and the strip's X discards
    /// it; other chats neither show nor send it.
    #[test]
    fn a_refused_voice_message_is_offered_only_in_its_chat() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        app.backend.record_demo_commands();
        let own = SAMPLES[0].id.to_owned();
        let other = SAMPLES[2].id.to_owned();
        let clip = vec![0.25; crate::voice::RATE as usize * 6];
        app.actions.push(crate::model::Action::OpenChat(other));
        render(&mut app, &ctx);
        app.unsent_voice = Some((own.clone(), clip.clone()));
        let discard = |ctx: &egui::Context| {
            ctx.data(|data| data.get_temp::<egui::Rect>(egui::Id::new("unsent-voice-discard")))
        };
        let enter = |app: &mut App| {
            app.focus_composer = true;
            render(app, &ctx);
            frame_with(
                app,
                &ctx,
                vec![key(egui::Key::Enter, egui::Modifiers::NONE)],
            );
            render(app, &ctx);
            app.backend.take_demo_commands()
        };
        let sent = enter(&mut app);
        assert!(
            !sent
                .iter()
                .any(|command| matches!(command, crate::backend::Command::SendVoice { .. })),
            "another chat does not send the clip"
        );
        assert!(discard(&ctx).is_none(), "another chat does not show it");
        assert!(app.unsent_voice.is_some());
        app.actions
            .push(crate::model::Action::OpenChat(own.clone()));
        render(&mut app, &ctx);
        assert!(
            discard(&ctx).is_some(),
            "its own chat shows the unsent clip"
        );
        let sent = enter(&mut app);
        assert!(
            sent.iter().any(|command| matches!(command,
                crate::backend::Command::SendVoice { chat, samples, .. }
                    if *chat == own && samples.len() == clip.len())),
            "Enter sends the clip again"
        );
        assert!(app.unsent_voice.is_none());
        // The strip's X discards a clip without sending it.
        app.unsent_voice = Some((own, clip));
        render(&mut app, &ctx);
        let pos = discard(&ctx).unwrap().center();
        let click = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame_with(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(pos), click(true)],
        );
        frame_with(&mut app, &ctx, vec![click(false)]);
        render(&mut app, &ctx);
        assert!(app.unsent_voice.is_none());
        assert!(
            !app.backend
                .take_demo_commands()
                .iter()
                .any(|command| matches!(command, crate::backend::Command::SendVoice { .. }))
        );
    }

    /// Actual composer input leaves the list steady; parking or discarding
    /// that draft changes its priority only after leaving the conversation.
    #[test]
    fn typing_and_clearing_a_draft_do_not_reorder_the_chat_list() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        let ids = |app: &App| -> Vec<String> {
            app.visible_chats()
                .iter()
                .map(|chat| chat.id.clone())
                .collect()
        };
        let original = ids(&app);
        let draft = SAMPLES[3].id;
        let pinned = SAMPLES[0].id;
        app.actions
            .push(crate::model::Action::OpenChat(draft.into()));
        app.focus_composer = true;
        render(&mut app, &ctx);
        frame_with(
            &mut app,
            &ctx,
            vec![egui::Event::Text("Synthetic unfinished reply".into())],
        );
        assert_eq!(app.composer, "Synthetic unfinished reply");
        assert_eq!(ids(&app), original, "typing cannot promote the open chat");
        app.actions
            .push(crate::model::Action::OpenChat(pinned.into()));
        frame_with(&mut app, &ctx, vec![]);
        let parked = ids(&app);
        assert_eq!(parked[0], pinned, "the pin stays first");
        assert_eq!(parked[1], draft, "the parked draft follows the pin");
        app.actions
            .push(crate::model::Action::OpenChat(draft.into()));
        app.focus_composer = true;
        render(&mut app, &ctx);
        frame_with(
            &mut app,
            &ctx,
            vec![key(egui::Key::A, egui::Modifiers::COMMAND)],
        );
        frame_with(
            &mut app,
            &ctx,
            vec![key(egui::Key::Backspace, egui::Modifiers::NONE)],
        );
        assert!(app.composer.is_empty());
        assert_eq!(ids(&app), parked, "clearing cannot demote the open chat");
        app.actions
            .push(crate::model::Action::OpenChat(pinned.into()));
        frame_with(&mut app, &ctx, vec![]);
        assert_eq!(ids(&app), original, "leaving drops the cleared draft");
    }

    #[test]
    fn enter_sends_and_shift_enter_breaks_the_line() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        app.focus_composer = true;
        render(&mut app, &ctx);
        frame_with(&mut app, &ctx, vec![egui::Event::Text("hello".into())]);
        assert_eq!(app.composer, "hello");
        frame_with(
            &mut app,
            &ctx,
            vec![key(egui::Key::Enter, egui::Modifiers::SHIFT)],
        );
        assert_eq!(app.composer, "hello\n", "Shift+Enter adds a line");
        frame_with(&mut app, &ctx, vec![egui::Event::Text("there".into())]);
        frame_with(
            &mut app,
            &ctx,
            vec![key(egui::Key::Enter, egui::Modifiers::NONE)],
        );
        assert_eq!(app.composer, "", "Enter sends");
    }

    #[test]
    fn shift_insert_pastes_the_clipboard_text() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        app.focus_composer = true;
        render(&mut app, &ctx);
        // The fixture keeps the test away from the system clipboard.
        ctx.data_mut(|data| {
            data.insert_temp(
                egui::Id::new("fixture-clipboard-text"),
                "ñandú ✓".to_owned(),
            );
        });
        frame_with(
            &mut app,
            &ctx,
            vec![key(egui::Key::Insert, egui::Modifiers::SHIFT)],
        );
        assert_eq!(app.composer, "ñandú ✓");
        frame_with(
            &mut app,
            &ctx,
            vec![key(egui::Key::Insert, egui::Modifiers::NONE)],
        );
        assert_eq!(app.composer, "ñandú ✓", "Insert alone inserts nothing");
    }

    #[test]
    fn shift_insert_paste_keeps_its_place_among_the_frame_events() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        app.focus_composer = true;
        render(&mut app, &ctx);
        ctx.data_mut(|data| {
            data.insert_temp(egui::Id::new("fixture-clipboard-text"), "ab".to_owned());
        });
        frame_with(
            &mut app,
            &ctx,
            vec![
                key(egui::Key::Insert, egui::Modifiers::SHIFT),
                egui::Event::Text("!".into()),
            ],
        );
        assert_eq!(app.composer, "ab!", "the paste comes before later text");
    }

    #[test]
    fn clicking_empty_conversation_space_returns_focus_to_the_composer() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        let chat = app.open_chat.clone().expect("the demo opens a chat");
        let conversation = app.conversations.get_mut(&chat).unwrap();
        conversation.messages.clear();
        conversation.complete = true;
        conversation.phone_exhausted = true;
        app.focus_composer = true;
        render(&mut app, &ctx);
        assert!(ctx.memory(|memory| memory.has_focus(egui::Id::new("composer-text"))));

        let viewport = app
            .selection_view
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .expect("the message viewport is on screen");
        let at = viewport.center();
        let pointer = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame_with(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(at), pointer(true), pointer(false)],
        );
        render(&mut app, &ctx);

        assert!(ctx.memory(|memory| memory.has_focus(egui::Id::new("composer-text"))));
    }

    /// Most empty space in a chat is the strip beside a bubble, which takes
    /// clicks before the background does; a click there refocuses too.
    #[test]
    fn clicking_beside_a_message_returns_focus_to_the_composer() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        let chat = app.open_chat.clone().expect("the demo opens a chat");
        render(&mut app, &ctx);
        render(&mut app, &ctx);
        let viewport = app
            .selection_view
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .expect("the message viewport is on screen");
        let bubble = app.conversations[&chat]
            .messages
            .iter()
            .rev()
            .find_map(|message| {
                let id = crate::ui::conversation::bubble_id(&chat, &message.id).with("rect");
                let rect = ctx.data(|data| data.get_temp::<egui::Rect>(id))?;
                (viewport.contains_rect(rect) && rect.right() < viewport.right() - 60.0)
                    .then_some(rect)
            })
            .expect("an incoming bubble with room beside it");
        let composer = egui::Id::new("composer-text");
        ctx.memory_mut(|memory| memory.surrender_focus(composer));
        render(&mut app, &ctx);
        assert!(!ctx.memory(|memory| memory.has_focus(composer)));

        let at = egui::pos2(viewport.right() - 20.0, bubble.center().y);
        let pointer = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame_with(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(at), pointer(true), pointer(false)],
        );
        render(&mut app, &ctx);
        assert!(ctx.memory(|memory| memory.has_focus(composer)));
    }

    /// Our time or ticks open "Message info"; an incoming time opens nothing.
    #[test]
    fn clicking_the_time_or_ticks_opens_message_info() {
        // A fresh app per click: a dialog just closed still covers the next frame.
        fn click_footer(from_me: bool, inset: f32) -> (String, String, Option<Dialog>) {
            let mut app = app();
            let ctx = egui::Context::default();
            app.attach(&ctx);
            let chat = app.open_chat.clone().expect("the demo opens a chat");
            render(&mut app, &ctx);
            render(&mut app, &ctx);
            let viewport = app
                .selection_view
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .expect("the message viewport is on screen");
            let (id, footer) = app.conversations[&chat]
                .messages
                .iter()
                .rev()
                .filter(|message| message.from_me == from_me)
                .filter(|message| {
                    !from_me
                        || matches!(
                            message.status,
                            Delivery::Delivered | Delivery::Read | Delivery::Played
                        )
                })
                .find_map(|message| {
                    let id = crate::ui::conversation::footer_id(&chat, &message.id);
                    let rect = ctx.data(|data| data.get_temp::<egui::Rect>(id))?;
                    viewport
                        .contains_rect(rect)
                        .then(|| (message.id.clone(), rect))
                })
                .expect("a message with its footer on screen");
            let at = egui::pos2(footer.right() - inset, footer.center().y);
            frame_with(
                &mut app,
                &ctx,
                vec![
                    egui::Event::PointerMoved(at),
                    primary(at, true),
                    primary(at, false),
                ],
            );
            render(&mut app, &ctx);
            (chat, id, app.dialog)
        }

        let (_, _, dialog) = click_footer(false, 7.5);
        assert!(dialog.is_none(), "an incoming time is not a button");

        for (part, inset) in [("ticks", 7.5), ("time", 25.0)] {
            let (chat, id, dialog) = click_footer(true, inset);
            assert!(
                matches!(
                    &dialog,
                    Some(Dialog::MessageInfo { chat: shown, message }) if *shown == chat && *message == id
                ),
                "the {part} open that message's info: {dialog:?}"
            );
        }
    }

    /// Scrolling within a day keeps its date at the top of the transcript,
    /// and the date fades away once the transcript stops moving.
    #[test]
    fn scrolling_inside_a_day_keeps_its_date_visible() {
        fn shapes_at(
            app: &mut App,
            ctx: &egui::Context,
            time: f64,
        ) -> Vec<egui::epaint::ClippedShape> {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1180.0, 780.0),
                    )),
                    time: Some(time),
                    ..Default::default()
                },
                |ui| {
                    let ctx = ui.ctx().clone();
                    app.background_frame(&ctx);
                    app.frame_ui(ui);
                },
            );
            output.textures_delta.clear();
            output.shapes
        }
        fn date_on_top(shapes: &[egui::epaint::ClippedShape], label: &str) -> bool {
            shapes.iter().any(|shape| {
                matches!(&shape.shape,
                egui::Shape::Text(text) if text.galley.text() == label
                    && text.pos.y >= shape.clip_rect.top()
                    && text.pos.y < shape.clip_rect.top() + 40.0)
            })
        }
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        let chat = app.open_chat.clone().unwrap();
        let conversation = app.conversations.get_mut(&chat).unwrap();
        let template = conversation.messages.last().unwrap().clone();
        conversation.messages = (0..120)
            .map(|index| {
                let mut row = template.clone();
                row.id = format!("date-fixture-{index}");
                row.timestamp = 1_700_000_000 + (index / 60) * 86_400 + (index % 60) * 60;
                row.content = Content::text(format!("History row {index}"));
                row
            })
            .collect();
        conversation.complete = true;
        for index in [90, 30] {
            app.actions.push(crate::model::Action::OpenMessage {
                chat: chat.clone(),
                message: format!("date-fixture-{index}"),
            });
            render(&mut app, &ctx);
            render(&mut app, &ctx);
            // Jumping to a message is not scrolling; turning the wheel is.
            let label = crate::util::day_label(
                app.locale,
                app.conversations[&chat].messages[index].timestamp,
            );
            let still = ctx.input(|input| input.time) + 2.0;
            assert!(!date_on_top(&shapes_at(&mut app, &ctx, still), &label));
            frame_with(
                &mut app,
                &ctx,
                vec![
                    egui::Event::PointerMoved(egui::pos2(800.0, 400.0)),
                    egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        delta: egui::vec2(0.0, 60.0),
                        modifiers: egui::Modifiers::NONE,
                        phase: egui::TouchPhase::Move,
                    },
                ],
            );
            render(&mut app, &ctx);
            let shapes = frame_sized(&mut app, &ctx, 780.0, Vec::new());
            assert!(
                date_on_top(&shapes, &label),
                "the date remains at the top when its separator is off screen: {label}"
            );
            let now = ctx.input(|input| input.time);
            assert!(
                date_on_top(&shapes_at(&mut app, &ctx, now + 0.5), &label),
                "the date stays a moment after scrolling stops"
            );
            // The wheel's easing may still move the view on the frames above.
            shapes_at(&mut app, &ctx, now + 1.1);
            shapes_at(&mut app, &ctx, now + 2.2);
            assert!(
                !date_on_top(&shapes_at(&mut app, &ctx, now + 3.0), &label),
                "the date fades once the transcript has been still"
            );
        }
    }

    #[test]
    fn the_send_shortcut_submits_newly_staged_images_and_files_before_message_controls() {
        for image in [false, true] {
            for enter_sends in [false, true] {
                let mut app = app();
                app.settings.enter_sends = enter_sends;
                app.backend.record_demo_commands();
                let ctx = egui::Context::default();
                app.attach(&ctx);
                render(&mut app, &ctx);
                app.backend.take_demo_commands();
                ctx.memory_mut(|memory| memory.request_focus(egui::Id::new("fixture-message")));
                if image {
                    ctx.data_mut(|data| {
                        data.insert_temp(
                            egui::Id::new("fixture-clipboard-image"),
                            (2_usize, 2_usize, vec![200_u8; 16]),
                        )
                    });
                } else {
                    let (photo, _) = sample_files(&app);
                    ctx.data_mut(|data| {
                        data.insert_temp(egui::Id::new("fixture-clipboard-files"), vec![photo])
                    });
                }
                let modifiers = if enter_sends {
                    egui::Modifiers::NONE
                } else {
                    egui::Modifiers::COMMAND
                };
                app.actions
                    .push(crate::model::Action::Open(crate::model::Page::Settings));
                // Bypass background_frame and stage through the real clipboard path.
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(1180.0, 780.0),
                        )),
                        events: vec![
                            egui::Event::ModifiersChanged(egui::Modifiers::COMMAND),
                            egui::Event::Key {
                                key: egui::Key::V,
                                physical_key: None,
                                pressed: false,
                                repeat: false,
                                modifiers: egui::Modifiers::COMMAND,
                            },
                            key(egui::Key::Enter, modifiers),
                        ],
                        ..Default::default()
                    },
                    |ui| app.frame_ui(ui),
                );
                output.textures_delta.clear();
                let commands = app.backend.take_demo_commands();
                assert!(
                    commands.iter().any(|command| matches!(
                        command,
                        crate::backend::Command::SendImage { .. }
                            | crate::backend::Command::SendFiles { .. }
                    )),
                    "image={image}, enter_sends={enter_sends}"
                );
                assert!(app.pending.is_empty());
                assert!(app.image_preview.is_none());
            }
        }
    }

    #[test]
    fn view_once_photo_toggle_supports_click_keyboard_and_send() {
        use crate::ui::focus::{Stop, control};
        let mut app = app();
        apply_flags(&mut app, Some("view-once-off"));
        app.backend.record_demo_commands();
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        app.attach(&ctx);
        render(&mut app, &ctx);

        press_labelled(&mut app, &ctx, "View once", egui::accesskit::Role::CheckBox);
        assert!(app.pending_view_once);
        ctx.memory_mut(|memory| memory.request_focus(egui::Id::new("composer-text")));
        frame_with(&mut app, &ctx, tab());
        assert_eq!(focused_stop(&ctx), Some(Stop::ViewOnce));
        frame_with(
            &mut app,
            &ctx,
            vec![key(egui::Key::Space, egui::Modifiers::NONE)],
        );
        render(&mut app, &ctx);
        assert!(
            !app.pending_view_once,
            "Space switches the selected icon off"
        );

        press_labelled(&mut app, &ctx, "View once", egui::accesskit::Role::CheckBox);
        app.backend.take_demo_commands();
        press_labelled(&mut app, &ctx, "Send", egui::accesskit::Role::Button);
        assert!(
            app.backend
                .take_demo_commands()
                .iter()
                .any(|command| matches!(command,
            crate::backend::Command::SendFiles { view_once: true, caption: Some(caption), .. }
                if caption == "One moment, just for you."))
        );
        assert!(app.pending.is_empty());
        assert!(!app.pending_view_once);
        assert!(control(&ctx, Stop::ViewOnce).is_none());
    }

    #[test]
    fn view_once_photo_control_fits_the_caption_in_both_themes_and_narrow_windows() {
        use crate::ui::focus::{Stop, control};
        for page in ["view-once", "view-once,light"] {
            for width in [380.0, 640.0, 1180.0] {
                let mut app = app();
                apply_flags(&mut app, Some(page));
                apply_flags(&mut app, Some("nosidebar"));
                let ctx = egui::Context::default();
                ctx.enable_accesskit();
                app.attach(&ctx);
                for _ in 0..4 {
                    let mut output = ctx.run_ui(
                        egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(
                                egui::Pos2::ZERO,
                                egui::vec2(width, 780.0),
                            )),
                            ..Default::default()
                        },
                        |ui| {
                            let ctx = ui.ctx().clone();
                            app.background_frame(&ctx);
                            app.frame_ui(ui);
                        },
                    );
                    let tree = output.platform_output.accesskit_update.as_ref().unwrap();
                    let toggle = tree
                        .nodes
                        .iter()
                        .map(|(_, node)| node)
                        .find(|node| node.label() == Some("View once"))
                        .expect("the icon has a readable name");
                    assert_eq!(toggle.role(), egui::accesskit::Role::CheckBox);
                    assert_eq!(toggle.toggled(), Some(egui::accesskit::Toggled::True));
                    output.textures_delta.clear();
                }
                let toggle = ctx
                    .read_response(control(&ctx, Stop::ViewOnce).unwrap())
                    .unwrap()
                    .rect;
                let send = ctx
                    .read_response(control(&ctx, Stop::Send).unwrap())
                    .unwrap()
                    .rect;
                let caption = ctx
                    .read_response(egui::Id::new("composer-text"))
                    .unwrap()
                    .rect;
                let pill = ctx
                    .data(|data| {
                        data.get_temp::<egui::Rect>(crate::ui::conversation::composer_pill_id())
                    })
                    .unwrap();
                assert!(caption.right() <= toggle.left(), "{page} {width}");
                assert!(toggle.right() <= send.left(), "{page} {width}");
                assert!(
                    pill.contains_rect(toggle) && pill.contains_rect(send),
                    "{page} {width}"
                );
                assert!((toggle.center().y - send.center().y).abs() < 1.0);
            }
        }
    }

    #[test]
    fn view_once_toggle_is_hidden_for_documents_batches_and_edits() {
        use crate::ui::focus::{Stop, control};
        for kind in ["document", "batch", "video", "edit"] {
            let mut app = app();
            apply_flags(&mut app, Some("view-once-off"));
            let path = match &app.pending[0].item {
                crate::app::Pending::File(path) => path.clone(),
                _ => unreachable!(),
            };
            match kind {
                "document" => app.pending[0].item = crate::app::Pending::Document(path),
                "video" => {
                    app.pending[0].item = crate::app::Pending::File("/fixture/clip.mp4".into())
                }
                "batch" => {
                    app.stage(crate::app::Pending::File(path));
                }
                "edit" => apply_flags(&mut app, Some("edit")),
                _ => unreachable!(),
            }
            let ctx = egui::Context::default();
            app.attach(&ctx);
            render(&mut app, &ctx);
            assert!(control(&ctx, Stop::ViewOnce).is_none(), "{kind}");
        }
    }

    /// New contact with a blank name messages a formatted number without
    /// saving it, and refuses a number that formatting cannot explain.
    #[test]
    fn new_contact_messages_a_formatted_number_left_unnamed() {
        let mut app = app();
        app.backend.record_demo_commands();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        app.actions
            .push(crate::model::Action::ShowDialog(Dialog::NewContact));
        render(&mut app, &ctx);
        app.backend.take_demo_commands();
        let submit = |app: &mut App, phone: &str| {
            app.new_contact_phone = phone.into();
            ctx.memory_mut(|memory| memory.request_focus(egui::Id::new("new-contact-phone")));
            render(app, &ctx);
            frame_with(
                app,
                &ctx,
                vec![key(egui::Key::Enter, egui::Modifiers::NONE)],
            );
            app.backend.take_demo_commands()
        };
        for refused in ["123", "+1 555 123 4567 ext. 89", "+1 (555 123-4567"] {
            assert!(
                !submit(&mut app, refused)
                    .iter()
                    .any(|command| matches!(command, crate::backend::Command::NewContact { .. })),
                "{refused}"
            );
        }
        assert!(
            submit(&mut app, "+1 (555) 123-4567")
                .iter()
                .any(|command| matches!(command,
            crate::backend::Command::NewContact { phone, full_name, first_name, .. }
                if phone == "15551234567" && full_name.is_none() && first_name.is_none()))
        );
        assert!(app.new_contact_pending);
    }

    /// Checks that navigation to an individual album member scrolls its grouped row into view.
    #[test]
    fn a_jump_to_an_album_member_reaches_the_group_and_finishes() {
        let mut app = app();
        media_album_sample(&mut app, true);
        let chat = app.open_chat.clone().unwrap();
        let conversation = app.conversations.get_mut(&chat).unwrap();
        let album = std::mem::take(&mut conversation.messages);
        let at = album[0].timestamp;
        let rows = |after: bool| {
            let chat = &chat;
            (0..80).map(move |index| {
                message(
                    chat,
                    &format!("album-context-{after}-{index}"),
                    false,
                    at + if after { 100 + index } else { -100 + index },
                    Content::text(format!("Surrounding history {index}")),
                )
            })
        };
        conversation.messages = rows(false).chain(album).chain(rows(true)).collect();
        conversation.complete = true;
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        app.actions.push(crate::model::Action::OpenMessage {
            chat,
            message: "album-2".into(),
        });
        render(&mut app, &ctx);
        render(&mut app, &ctx);
        assert!(
            app.scroll_anchor.is_none(),
            "a member jump resolves at its album row"
        );
        let shapes = frame_sized(&mut app, &ctx, 780.0, Vec::new());
        assert!(
            shapes
                .iter()
                .any(|shape| matches!(&shape.shape, egui::Shape::Text(text)
            if text.galley.text().contains("Weekend references")
                && shape.clip_rect.contains(text.pos + egui::vec2(1.0, 1.0))))
        );
    }

    /// A right-click on one album tile opens the menu for that member, so
    /// Reply quotes the picture under the pointer rather than the first.
    #[test]
    fn the_menu_of_an_album_tile_acts_on_that_member() {
        let mut app = app();
        media_album_sample(&mut app, true);
        let chat = app.open_chat.clone().unwrap();
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        app.attach(&ctx);
        render(&mut app, &ctx);
        render(&mut app, &ctx);
        let tile = ctx
            .data(|data| {
                data.get_temp::<egui::Rect>(crate::ui::conversation::album_tile_id(
                    &chat, "album-2",
                ))
            })
            .expect("the third member's tile was drawn");
        let press = |pressed| egui::Event::PointerButton {
            pos: tile.center(),
            button: egui::PointerButton::Secondary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame_with(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(tile.center()), press(true)],
        );
        frame_with(&mut app, &ctx, vec![press(false)]);
        let popup = crate::ui::conversation::bubble_id(&chat, "album-0").with("popup");
        assert!(
            egui::Popup::is_id_open(&ctx, popup),
            "the album's menu opened"
        );
        let reply = accessible_nodes(&mut app, &ctx, Vec::new())
            .into_iter()
            .find(|(label, _, _)| label == "Reply")
            .expect("the menu offers Reply")
            .2;
        for pressed in [true, false] {
            frame_with(
                &mut app,
                &ctx,
                vec![
                    egui::Event::PointerMoved(reply),
                    egui::Event::PointerButton {
                        pos: reply,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
        render(&mut app, &ctx);
        assert_eq!(app.reply_to.as_deref(), Some("album-2"));
    }

    /// Our own albums lay their tiles out in reading order even though an
    /// own bubble lays out from the right.
    #[test]
    fn our_album_tiles_keep_reading_order() {
        let mut app = app();
        media_album_sample(&mut app, true);
        let chat = app.open_chat.clone().unwrap();
        for row in &mut app.conversations.get_mut(&chat).unwrap().messages {
            row.from_me = true;
            row.status = crate::model::Delivery::Read;
        }
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        render(&mut app, &ctx);
        let tile = |id: &str| {
            ctx.data(|data| {
                data.get_temp::<egui::Rect>(crate::ui::conversation::album_tile_id(&chat, id))
            })
            .expect("the tile was drawn")
        };
        assert!(tile("album-0").left() < tile("album-1").left());
        assert!(tile("album-2").left() < tile("album-3").left());
        assert!(tile("album-1").bottom() <= tile("album-2").top());
    }

    /// A reaction to a later member shows under the album row, which
    /// otherwise carried only the first member's reactions.
    #[test]
    fn album_rows_show_every_members_reactions() {
        let mut app = app();
        media_album_sample(&mut app, true);
        let chat = app.open_chat.clone().unwrap();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        render(&mut app, &ctx);
        let bare = app.conversations[&chat].rows["album-0"].height;
        app.conversations.get_mut(&chat).unwrap().messages[2]
            .reactions
            .push(Reaction {
                sender: "fixture@s.whatsapp.net".into(),
                from_me: false,
                emoji: "👍".into(),
            });
        render(&mut app, &ctx);
        render(&mut app, &ctx);
        assert!(app.conversations[&chat].rows["album-0"].height > bare);
    }

    /// Checks that grouped album members remain independently selectable and actionable.
    #[test]
    fn selecting_messages_keeps_every_album_member_individually_accessible() {
        let mut app = app();
        media_album_sample(&mut app, true);
        let chat = app.open_chat.clone().unwrap();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        assert_eq!(app.conversations[&chat].rows["album-1"].height, 0.0);
        app.selection = Some((chat.clone(), vec!["album-1".into()]));
        render(&mut app, &ctx);
        for index in 0..4 {
            assert!(app.conversations[&chat].rows[&format!("album-{index}")].height > 0.0);
        }
    }

    /// Download all is asked for, so it fetches every missing member whatever
    /// its size, and leaves active or saved members alone.
    #[test]
    fn album_download_all_fetches_oversized_members_and_skips_active_or_saved_ones() {
        use crate::model::MediaState;
        let mut app = app();
        media_album_sample(&mut app, true);
        let chat = app.open_chat.clone().unwrap();
        let (saved, _) = sample_files(&app);
        let messages = &mut app.conversations.get_mut(&chat).unwrap().messages;
        messages[1].content.media_at_mut(None).unwrap().size =
            crate::model::ATTACHMENT_DOWNLOAD_LIMIT + 1;
        messages[2].content.media_at_mut(None).unwrap().state = MediaState::Downloading;
        messages[3].content.media_at_mut(None).unwrap().path = Some(saved.clone());
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        app.attach(&ctx);
        render(&mut app, &ctx);
        app.backend.record_demo_commands();
        for first_click in [true, false] {
            let nodes = accessible_nodes(&mut app, &ctx, Vec::new());
            assert!(nodes.iter().any(|(label, _, _)| label == "Save all…"));
            let Some(pos) = nodes
                .iter()
                .find(|(label, _, _)| label == "Download all")
                .map(|(_, _, pos)| *pos)
            else {
                // Every member is downloading or saved: nothing is left to ask for.
                assert!(!first_click, "the first frame offers Download all");
                break;
            };
            for pressed in [true, false] {
                accessible_nodes(
                    &mut app,
                    &ctx,
                    vec![
                        egui::Event::PointerMoved(pos),
                        egui::Event::PointerButton {
                            pos,
                            pressed,
                            button: egui::PointerButton::Primary,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                );
            }
            render(&mut app, &ctx);
            let commands = app.backend.take_demo_commands();
            let downloads: Vec<_> = commands
                .iter()
                .filter_map(|command| match command {
                    crate::backend::Command::Download {
                        message,
                        manual: true,
                        ..
                    } => Some(message.as_str()),
                    _ => None,
                })
                .collect();
            assert_eq!(
                downloads,
                if first_click {
                    vec!["album-0", "album-1"]
                } else {
                    vec![]
                }
            );
            let rows = &app.conversations[&chat].messages;
            assert!(matches!(
                rows[1].content.media().unwrap().state,
                MediaState::Downloading
            ));
            assert!(matches!(
                rows[2].content.media().unwrap().state,
                MediaState::Downloading
            ));
            assert_eq!(rows[3].content.media().unwrap().path.as_ref(), Some(&saved));
        }
    }

    /// Album buttons expose their media kind and download state, including failures.
    #[test]
    fn album_tiles_name_each_download_state_for_screen_readers() {
        for video in [false, true] {
            let mut app = app();
            media_album_sample(&mut app, true);
            let chat = app.open_chat.clone().unwrap();
            let rows = &mut app.conversations.get_mut(&chat).unwrap().messages;
            for (index, row) in rows.iter_mut().enumerate() {
                let mut media = row.content.media().unwrap().clone();
                media.state = match index {
                    1 => crate::model::MediaState::Downloading,
                    2 => crate::model::MediaState::Failed("Synthetic download error".into()),
                    _ => crate::model::MediaState::Idle,
                };
                if index == 3 {
                    // A synthetic local path exercises the Open label without real media.
                    media.path = Some(std::path::PathBuf::from("synthetic-album-file"));
                }
                row.content = if video {
                    Content::Video {
                        caption: None,
                        media,
                        seconds: Some(1),
                        gif: false,
                        note: false,
                    }
                } else {
                    Content::Image {
                        motion: None,
                        caption: None,
                        media,
                    }
                };
                row.thumbnail = None;
            }
            let ctx = egui::Context::default();
            ctx.enable_accesskit();
            app.attach(&ctx);
            render(&mut app, &ctx);
            let nodes = accessible_nodes(&mut app, &ctx, Vec::new());
            let kind = if video { "Video" } else { "Photo" };
            for state in [
                "Download",
                "Downloading",
                "Failed: Synthetic download error",
                "Open",
            ] {
                let label = format!("{kind}, {state}");
                assert!(
                    nodes
                        .iter()
                        .any(|(name, role, _)| name == &label
                            && *role == egui::accesskit::Role::Button),
                    "{label}"
                );
            }
        }
    }

    /// Earlier outgoing failures stay visible even when the album's final member was sent.
    #[test]
    fn albums_show_each_members_delivery_failure() {
        for flags in ["media-album-failed", "media-album-failed,light"] {
            let mut app = app();
            apply_flags(&mut app, Some(flags));
            let ctx = egui::Context::default();
            ctx.enable_accesskit();
            app.attach(&ctx);
            render(&mut app, &ctx);
            let shapes = frame_sized(&mut app, &ctx, 560.0, Vec::new());
            assert!(
                shapes
                    .iter()
                    .any(|shape| matches!(&shape.shape, egui::Shape::Text(text)
                if text.galley.text() == "Not sent"))
            );
            let chat = app.open_chat.as_deref().unwrap();
            assert!(ctx.data(|data| {
                data.get_temp::<egui::Rect>(crate::ui::conversation::footer_id(chat, "album-1"))
                    .is_some()
            }));
            let nodes = accessible_nodes(&mut app, &ctx, Vec::new());
            assert!(
                nodes
                    .iter()
                    .any(|(label, _, _)| label.contains("will not retry it"))
            );
        }
    }

    /// Quotes on later album members remain visible alongside partial-download actions.
    #[test]
    fn partially_downloaded_albums_keep_member_quotes() {
        let mut app = app();
        apply_flags(&mut app, Some("media-album-partial"));
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        app.attach(&ctx);
        render(&mut app, &ctx);
        let nodes = accessible_nodes(&mut app, &ctx, Vec::new());
        assert!(nodes.iter().any(|(label, _, _)| label == "Save all…"));
        assert!(nodes.iter().any(|(label, _, _)| label == "Download all"));
        let shapes = frame_sized(&mut app, &ctx, 780.0, Vec::new());
        assert!(
            shapes
                .iter()
                .any(|shape| matches!(&shape.shape, egui::Shape::Text(text)
            if text.galley.text().contains("A reference for the album")))
        );
    }

    /// Both local-date dividers and both grouped rows survive an album spanning midnight.
    #[test]
    fn albums_crossing_midnight_keep_both_date_dividers() {
        for light in [false, true] {
            let mut app = app();
            apply_flags(
                &mut app,
                Some(if light {
                    "media-album-midnight,light"
                } else {
                    "media-album-midnight"
                }),
            );
            let chat = app.open_chat.clone().unwrap();
            let messages = &app.conversations[&chat].messages;
            let expected = [messages[0].timestamp, messages[2].timestamp]
                .map(|timestamp| crate::util::day_label(app.locale, timestamp));
            let ctx = egui::Context::default();
            app.attach(&ctx);
            render(&mut app, &ctx);
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1180.0, 780.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    let ctx = ui.ctx().clone();
                    app.background_frame(&ctx);
                    app.frame_ui(ui);
                },
            );
            output.textures_delta.clear();
            let mut labels = std::collections::HashSet::new();
            /// Collects painted labels, including date chips drawn directly by the painter.
            fn visit(shape: &egui::epaint::Shape, labels: &mut std::collections::HashSet<String>) {
                match shape {
                    egui::epaint::Shape::Vec(shapes) => {
                        for shape in shapes {
                            visit(shape, labels);
                        }
                    }
                    egui::epaint::Shape::Text(text) => {
                        labels.insert(text.galley.job.text.clone());
                    }
                    _ => {}
                }
            }
            for shape in &output.shapes {
                visit(&shape.shape, &mut labels);
            }
            for date in expected {
                assert!(labels.contains(&date), "missing {date}: {labels:?}");
            }
            let rows = &app.conversations[&chat].rows;
            assert!(rows["album-0"].height > 0.0);
            assert_eq!(rows["album-1"].height, 0.0);
            assert!(rows["album-2"].height > 0.0);
            assert_eq!(rows["album-3"].height, 0.0);
        }
    }

    /// The composer keeps its draft while the preview is open: Enter does not
    /// send it, and Tab and Enter reach the preview's own controls instead.
    #[test]
    fn the_image_preview_owns_the_keyboard() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        app.focus_composer = true;
        render(&mut app, &ctx);
        frame_with(&mut app, &ctx, vec![egui::Event::Text("draft".into())]);
        assert_eq!(app.composer, "draft");

        let (photo, _) = sample_files(&app);
        app.actions.push(crate::model::Action::PreviewImage {
            card: None,
            path: photo,
            chat: crate::demo::SAMPLES[0].id.to_owned(),
            message: "ada-photo".to_owned(),
        });
        // Enter in the very frame the preview opens, before egui knows about
        // the modal layer.
        frame_with(
            &mut app,
            &ctx,
            vec![key(egui::Key::Enter, egui::Modifiers::NONE)],
        );
        render(&mut app, &ctx);
        assert_eq!(app.composer, "draft", "Enter must not send the draft");
        assert!(app.image_preview.is_some());

        frame_with(
            &mut app,
            &ctx,
            vec![key(egui::Key::Tab, egui::Modifiers::NONE)],
        );
        render(&mut app, &ctx);
        let focused = ctx
            .memory(|memory| memory.focused())
            .and_then(|id| ctx.read_response(id))
            .expect("Tab focuses a preview control");
        assert_eq!(focused.layer_id.id, egui::Id::new("image-preview"));

        // The first control is Close; Enter activates it.
        frame_with(
            &mut app,
            &ctx,
            vec![key(egui::Key::Enter, egui::Modifiers::NONE)],
        );
        render(&mut app, &ctx);
        assert!(app.image_preview.is_none(), "Enter activates Close");
        assert_eq!(app.composer, "draft");
    }

    #[test]
    fn preview_arrows_browse_photos_without_touching_the_draft() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        apply_flags(&mut app, Some("preview"));
        app.composer = "unsent draft".into();
        render(&mut app, &ctx);
        let initial = app
            .image_preview
            .as_ref()
            .unwrap()
            .path()
            .map(std::path::Path::to_owned);
        frame_with(
            &mut app,
            &ctx,
            vec![key(egui::Key::ArrowRight, egui::Modifiers::NONE)],
        );
        assert_ne!(
            app.image_preview
                .as_ref()
                .unwrap()
                .path()
                .map(std::path::Path::to_owned),
            initial
        );
        frame_with(
            &mut app,
            &ctx,
            vec![key(egui::Key::ArrowLeft, egui::Modifiers::NONE)],
        );
        assert_eq!(
            app.image_preview
                .as_ref()
                .unwrap()
                .path()
                .map(std::path::Path::to_owned),
            initial
        );
        frame_with(
            &mut app,
            &ctx,
            vec![key(egui::Key::Escape, egui::Modifiers::NONE)],
        );
        assert!(app.image_preview.is_none());
        assert_eq!(app.composer, "unsent draft");
    }

    /// A sticker shown large offers the favorite toggle that matches its
    /// state, and a click on the sticker itself closes it.
    #[test]
    fn a_large_sticker_toggles_its_favorite_and_closes_on_click() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        apply_flags(&mut app, Some("sticker-view"));
        app.backend.record_demo_commands();
        render(&mut app, &ctx);
        let received = app.sticker_view.as_ref().expect("open").path().to_owned();
        let click = |app: &mut App, pos: egui::Pos2| {
            let press = |pressed| egui::Event::PointerButton {
                pos,
                pressed,
                button: egui::PointerButton::Primary,
                modifiers: egui::Modifiers::NONE,
            };
            frame_with(app, &ctx, vec![egui::Event::PointerMoved(pos), press(true)]);
            frame_with(app, &ctx, vec![press(false)]);
            render(app, &ctx);
        };
        let button = || {
            ctx.data(|data| {
                data.get_temp::<egui::Rect>(crate::ui::image_preview::sticker_favorite_id())
            })
            .expect("the favorite button is drawn")
        };

        click(&mut app, button().center());
        let commands = app.backend.take_demo_commands();
        assert!(
            commands.iter().any(
                |command| matches!(command, crate::backend::Command::SaveSticker { path } if *path == received)
            ),
            "{commands:?}"
        );

        // Once the favorite copy arrives, the same button removes it.
        let copy = app.dirs.media_cache_dir().join(format!(
            "{}.webp",
            crate::backend::sticker_store::content_hash(&std::fs::read(&received).unwrap())
        ));
        app.stickers_saved.push(copy.clone());
        render(&mut app, &ctx);
        click(&mut app, button().center());
        let commands = app.backend.take_demo_commands();
        assert!(
            commands.iter().any(
                |command| matches!(command, crate::backend::Command::ForgetSticker { path } if *path == copy)
            ),
            "{commands:?}"
        );
        assert!(app.sticker_view.is_some(), "the buttons leave it open");

        let above = button().center_top() - egui::vec2(0.0, 120.0);
        click(&mut app, above);
        assert!(
            app.sticker_view.is_none(),
            "a click on the sticker closes it"
        );
    }

    /// The demo app with the sender of its account's events, and the open
    /// chat's sticker moved to the bottom, where it is drawn, with its file
    /// at `file`.
    fn unreadable_sticker(
        file: &str,
        animated: bool,
    ) -> (
        App,
        egui::Context,
        std::sync::mpsc::Sender<crate::backend::Event>,
        std::path::PathBuf,
    ) {
        let root = tempfile::Builder::new()
            .prefix("zapfast-demo-")
            .tempdir()
            .unwrap();
        let (mut app, events) = App::headless(AppDirs::under(root.path()), Settings::default());
        ROOTS.with(|roots| roots.borrow_mut().push(root));
        populate(&mut app);
        let path = app.dirs.media_cache_dir().join(file);
        let chat = app.open_chat.clone().unwrap();
        let messages = &mut app.conversations.get_mut(&chat).unwrap().messages;
        let at = messages
            .iter()
            .position(|row| row.id == "ada-sticker")
            .expect("the sample has a sticker");
        let mut row = messages.remove(at);
        row.timestamp = messages.last().map_or(0, |last| last.timestamp) + 1;
        if let Content::Sticker {
            media,
            animated: flag,
        } = &mut row.content
        {
            media.path = Some(path.clone());
            *flag = animated;
        }
        messages.push(row);
        let ctx = egui::Context::default();
        app.attach(&ctx);
        app.backend.record_demo_commands();
        (app, ctx, events, path)
    }

    fn sticker_media(app: &App) -> crate::model::Media {
        let chat = app.open_chat.clone().unwrap();
        app.conversations[&chat]
            .messages
            .iter()
            .find(|row| row.id == "ada-sticker")
            .and_then(|row| row.content.media().cloned())
            .expect("the sticker is loaded")
    }

    /// Renders until `done`, collecting the sticker's download commands as
    /// their `manual` flags.
    fn render_until(
        app: &mut App,
        ctx: &egui::Context,
        downloads: &mut Vec<bool>,
        done: impl Fn(&App, &[bool]) -> bool,
    ) -> bool {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            render(app, ctx);
            downloads.extend(
                app.backend
                    .take_demo_commands()
                    .iter()
                    .filter_map(|command| match command {
                        crate::backend::Command::Download {
                            message, manual, ..
                        } if message == "ada-sticker" => Some(*manual),
                        _ => None,
                    }),
            );
            if done(app, downloads) {
                return true;
            }
            if std::time::Instant::now() > deadline {
                return false;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    /// A sticker whose file was deleted is forgotten and downloads again on
    /// its own, instead of staying a blank placeholder (#603).
    #[test]
    fn a_sticker_whose_file_is_gone_downloads_again() {
        for animated in [true, false] {
            let (mut app, ctx, _events, _path) = unreadable_sticker("gone.webp", animated);
            let mut downloads = Vec::new();
            assert!(
                render_until(&mut app, &ctx, &mut downloads, |_, downloads| {
                    !downloads.is_empty()
                }),
                "animated: {animated}: the sticker downloads again"
            );
            assert_eq!(downloads, [false], "animated: {animated}");
            let media = sticker_media(&app);
            assert_eq!(media.path, None, "animated: {animated}");
            assert!(
                matches!(media.state, crate::model::MediaState::Downloading),
                "animated: {animated}: {:?}",
                media.state
            );
        }
    }

    /// A sticker whose file does not decode shows the failed state and
    /// downloads again only when clicked: a fresh copy that still does not
    /// decode stays failed rather than downloading forever, and one that
    /// decodes is drawn, not answered with the earlier failure (#603).
    #[test]
    fn a_sticker_that_does_not_decode_retries_only_when_clicked() {
        for animated in [true, false] {
            let (mut app, ctx, events, path) = unreadable_sticker("broken.webp", animated);
            std::fs::write(&path, b"not a sticker").unwrap();
            let chat = app.open_chat.clone().unwrap();
            let failed = |app: &App, _: &[bool]| {
                matches!(
                    sticker_media(app).state,
                    crate::model::MediaState::Failed(_)
                )
            };
            let click = |app: &mut App, downloads: &mut Vec<bool>| {
                let pos = bubble_rect(&ctx, &chat, "ada-sticker")
                    .expect("the sticker is drawn")
                    .center();
                for pressed in [true, false] {
                    frame_with(
                        app,
                        &ctx,
                        vec![
                            egui::Event::PointerMoved(pos),
                            egui::Event::PointerButton {
                                pos,
                                pressed,
                                button: egui::PointerButton::Primary,
                                modifiers: egui::Modifiers::NONE,
                            },
                        ],
                    );
                }
                assert!(
                    render_until(app, &ctx, downloads, |_, downloads| !downloads.is_empty()),
                    "animated: {animated}: the click downloads it again"
                );
                assert_eq!(*downloads, [true], "animated: {animated}");
                assert!(app.sticker_view.is_none(), "the click is not a preview");
                downloads.clear();
            };
            let arrives = |events: &std::sync::mpsc::Sender<crate::backend::Event>| {
                events
                    .send(crate::backend::Event::Media {
                        card: None,
                        chat: chat.clone(),
                        message: "ada-sticker".into(),
                        result: Ok(path.clone()),
                    })
                    .unwrap();
            };
            let mut downloads = Vec::new();
            for attempt in 0..2 {
                assert!(
                    render_until(&mut app, &ctx, &mut downloads, failed),
                    "animated: {animated}, attempt {attempt}: the bubble shows the failure"
                );
                assert_eq!(sticker_media(&app).path, None);
                // Settled: no download starts by itself.
                let settled = std::time::Instant::now() + std::time::Duration::from_millis(200);
                render_until(&mut app, &ctx, &mut downloads, |_, _| {
                    std::time::Instant::now() > settled
                });
                assert!(
                    downloads.is_empty(),
                    "animated: {animated}, attempt {attempt}: no automatic download"
                );
                click(&mut app, &mut downloads);
                if attempt == 0 {
                    // The same file arrives again, and still does not decode.
                    arrives(&events);
                }
            }
            // This time a good copy arrives under the same name.
            std::fs::write(&path, stock::ANIMATED_STICKER).unwrap();
            arrives(&events);
            let shown = std::time::Instant::now() + std::time::Duration::from_millis(500);
            render_until(&mut app, &ctx, &mut downloads, |app, _| {
                sticker_media(app).path.is_none() || std::time::Instant::now() > shown
            });
            let media = sticker_media(&app);
            assert_eq!(media.path.as_ref(), Some(&path), "animated: {animated}");
            assert!(
                matches!(media.state, crate::model::MediaState::Idle),
                "animated: {animated}: {:?}",
                media.state
            );
            assert!(downloads.is_empty());
        }
    }

    /// The viewer steps through an animated sticker's frames on its own.
    #[test]
    fn an_animated_sticker_plays_in_the_viewer() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        animated_sticker_sample(&mut app);
        app.picker = None;
        let path = app.stickers[0].clone();
        render(&mut app, &ctx);
        app.actions
            .push(crate::model::Action::PreviewSticker(path.clone()));
        let drawn = |app: &mut App| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1180.0, 780.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    let ctx = ui.ctx().clone();
                    app.background_frame(&ctx);
                    app.frame_ui(ui);
                },
            );
            output.textures_delta.clear();
            output
                .shapes
                .iter()
                .map(|clipped| clipped.shape.texture_id())
                .collect::<std::collections::HashSet<_>>()
        };
        let first = drawn(&mut app);
        assert!(app.sticker_view.as_ref().expect("open").animated());
        let mut seen = std::collections::HashSet::new();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while seen.len() < 3 && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(30));
            seen.extend(drawn(&mut app).difference(&first).copied());
        }
        assert!(
            seen.len() >= 3,
            "the viewer showed {} new frames",
            seen.len()
        );
    }

    /// Escape closes a large sticker without reaching the chat behind it.
    #[test]
    fn escape_closes_a_large_sticker() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        apply_flags(&mut app, Some("sticker-view"));
        render(&mut app, &ctx);
        assert!(app.sticker_view.is_some());
        frame_with(
            &mut app,
            &ctx,
            vec![key(egui::Key::Escape, egui::Modifiers::NONE)],
        );
        render(&mut app, &ctx);
        assert!(app.sticker_view.is_none());
        assert!(app.open_chat.is_some(), "the chat stays open");
    }

    /// "Show in the chat" closes the viewer and brings the message it was
    /// showing into view. Closing alone left the chat where it was, so after
    /// stepping back through the album the message the button names stayed out
    /// of sight.
    #[test]
    fn showing_the_previewed_message_in_the_chat_scrolls_to_it() {
        use egui::accesskit::Role;
        let mut app = app();
        // The demo page that opens the viewer over the sample chat's album:
        // the header's message actions only exist when the viewer has an item.
        apply_flags(&mut app, Some("preview"));
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        app.attach(&ctx);
        render(&mut app, &ctx);
        assert!(app.image_preview.is_some(), "the viewer is open");
        assert!(
            !app.viewer_media.is_empty(),
            "the album the actions belong to is loaded"
        );

        let at = accessible_nodes(&mut app, &ctx, Vec::new())
            .into_iter()
            .find(|(label, role, _)| label == "Show in the chat" && *role == Role::Button)
            .map(|(_, _, centre)| centre)
            .expect("the viewer offers Show in the chat");
        let press = |pressed| egui::Event::PointerButton {
            pos: at,
            pressed,
            button: egui::PointerButton::Primary,
            modifiers: egui::Modifiers::NONE,
        };
        frame_with(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(at), press(true)],
        );
        frame_with(&mut app, &ctx, vec![press(false)]);
        render(&mut app, &ctx);

        assert!(app.image_preview.is_none(), "the viewer closes");
        // The anchor itself is consumed by the frame that scrolls to it, so
        // the highlight is what still names the message here.
        let highlight = app
            .jump_highlight
            .as_ref()
            .expect("the chat is brought to the message the viewer was showing");
        assert_eq!(highlight.message, "preview-1");
        assert_eq!(highlight.chat, crate::demo::SAMPLES[0].id);
    }

    /// Ctrl++ zooms the picture, not the whole interface, including when the
    /// layout needs Shift to type the plus.
    #[test]
    fn zoom_shortcuts_zoom_the_previewed_image_only() {
        let mut app = app();
        let ctx = egui::Context::default();
        let fitted = open_sample_preview(&mut app, &ctx);
        let interface_zoom = ctx.zoom_factor();

        let ctrl_shift = egui::Modifiers {
            ctrl: true,
            shift: true,
            command: !cfg!(target_os = "macos"),
            ..Default::default()
        };
        frame_with(&mut app, &ctx, vec![key(egui::Key::Equals, ctrl_shift)]);
        render(&mut app, &ctx);
        let first = app.image_preview.as_ref().unwrap().zoom();
        assert!(
            (first - fitted * 1.25).abs() < 1e-4,
            "zooms from the fitted size"
        );
        frame_with(
            &mut app,
            &ctx,
            vec![key(egui::Key::Plus, egui::Modifiers::COMMAND)],
        );
        render(&mut app, &ctx);
        assert!((app.image_preview.as_ref().unwrap().zoom() - first * 1.25).abs() < 1e-4);
        frame_with(
            &mut app,
            &ctx,
            vec![key(egui::Key::Num0, egui::Modifiers::COMMAND)],
        );
        render(&mut app, &ctx);
        assert!(app.image_preview.as_ref().unwrap().is_fit());
        assert_eq!(ctx.zoom_factor(), interface_zoom);
    }

    /// In a narrow window a long image name stays clear of the preview's
    /// buttons: cut before the first of them, it rests at its start, scrolls
    /// to its end, rests, and scrolls back.
    #[test]
    fn a_long_image_name_scrolls_beside_the_preview_buttons() {
        long_image_name_scrolls(
            "holiday_photo_from_the_beach_with_everyone_at_sunset.jpg",
            false,
        );
    }

    /// A right-to-left name begins at its right edge, so it rests there first
    /// and scrolls towards its end on the left.
    #[test]
    fn a_long_right_to_left_image_name_starts_at_its_right_edge() {
        long_image_name_scrolls("תמונה_מהחופשה_בים_עם_כל_המשפחה_בשקיעה_הארוכה.jpg", true);
    }

    fn long_image_name_scrolls(name: &str, rtl: bool) {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        let (photo, _) = sample_files(&app);
        let long = photo.with_file_name(name);
        std::fs::copy(&photo, &long).unwrap();
        // A chat with no name, so the viewer's title is the file's name. The
        // viewer shows the open chat's media and closes when it is left.
        app.open_chat = Some("unknown@s.whatsapp.net".into());
        app.actions.push(crate::model::Action::PreviewImage {
            card: None,
            path: long,
            chat: "unknown@s.whatsapp.net".into(),
            message: "long-name".into(),
        });
        let mut narrow = |time: f64| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(360.0, 640.0),
                    )),
                    time: Some(time),
                    ..Default::default()
                },
                |ui| {
                    let ctx = ui.ctx().clone();
                    app.background_frame(&ctx);
                    app.frame_ui(ui);
                },
            );
            output.textures_delta.clear();
            output.shapes
        };
        // Where a text was painted, its width, and the rect it was clipped to.
        let find = |shapes: &[egui::epaint::ClippedShape], text: &str| {
            shapes
                .iter()
                .find_map(|clipped| match &clipped.shape {
                    egui::Shape::Text(shape) if shape.galley.job.text == text => {
                        Some((shape.pos.x, shape.galley.size().x, clipped.clip_rect))
                    }
                    _ => None,
                })
                .unwrap_or_else(|| panic!("{text} was not painted"))
        };
        narrow(0.0);
        let shapes = narrow(0.0);
        let (x, width, clip) = find(&shapes, name);
        let (fit, _, _) = find(&shapes, "Fit");
        assert!(clip.right() < fit, "the name runs under the buttons");
        assert!(
            width > clip.width(),
            "the name should not fit in 360 points"
        );
        // Where the name's start and end show when the line rests at either.
        let (start, finish) = (clip.left(), clip.right() - width);
        let (start, finish) = if rtl {
            (finish, start)
        } else {
            (start, finish)
        };
        assert!((x - start).abs() < 0.5, "the name starts at its start");

        let travel = f64::from(width - clip.width()) / 40.0;
        assert_eq!(find(&narrow(2.9), name).0, x, "it rests three seconds");
        let (end, _, end_clip) = find(&narrow(3.0 + travel + 1.0), name);
        assert!(
            (end - finish).abs() < 0.5,
            "the name's end shows at the far edge"
        );
        assert_eq!(end_clip, clip, "the name stays inside its room");
        assert_eq!(find(&narrow(3.0 + travel + 2.9), name).0, end);
        let (back, _, _) = find(&narrow(2.0 * (3.0 + travel) + 0.5), name);
        assert_eq!(back, x, "and it scrolls back to its start");
    }

    #[test]
    fn preview_navigation_stays_at_the_stage_edges() {
        fn controls(shapes: &[egui::epaint::ClippedShape]) -> Vec<egui::Rect> {
            fn collect(shape: &egui::Shape, out: &mut Vec<egui::Rect>) {
                match shape {
                    egui::Shape::Circle(circle) if (circle.radius - 20.0).abs() < 1.1 => {
                        out.push(egui::Rect::from_center_size(
                            circle.center,
                            egui::Vec2::splat(40.0),
                        ));
                    }
                    egui::Shape::Vec(shapes) => {
                        for shape in shapes {
                            collect(shape, out);
                        }
                    }
                    _ => {}
                }
            }
            let mut out = Vec::new();
            for clipped in shapes {
                collect(&clipped.shape, &mut out);
            }
            out
        }
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        apply_flags(&mut app, Some("preview"));
        render(&mut app, &ctx);
        let paths: Vec<_> = app.conversations[app.open_chat.as_ref().unwrap()]
            .messages
            .iter()
            .filter_map(|message| message.content.media()?.path.as_ref())
            .map(|path| crate::util::image_uri(path))
            .collect();
        for _ in 0..200 {
            render(&mut app, &ctx);
            let ready = paths.iter().all(|uri| {
                matches!(
                    ctx.try_load_texture(
                        uri,
                        egui::TextureOptions::default(),
                        egui::SizeHint::default()
                    ),
                    Ok(egui::load::TexturePoll::Ready { .. })
                )
            });
            if ready {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        render(&mut app, &ctx);
        app.composer = "unsent draft".into();
        let picture = preview_frame(&mut app, &ctx, vec![]);
        let initial = app
            .image_preview
            .as_ref()
            .unwrap()
            .path()
            .map(std::path::Path::to_owned);
        let hidden = frame_sized(
            &mut app,
            &ctx,
            780.0,
            vec![egui::Event::PointerMoved(egui::pos2(5.0, 5.0))],
        );
        assert!(
            controls(&hidden).is_empty(),
            "outside the modal hides the arrows"
        );
        let beside = egui::pos2(picture.left() - 5.0, picture.center().y);
        let hidden = frame_sized(
            &mut app,
            &ctx,
            780.0,
            vec![egui::Event::PointerMoved(beside)],
        );
        assert!(
            controls(&hidden).len() == 2,
            "the stage beside the photo also shows the arrows"
        );
        frame_sized(
            &mut app,
            &ctx,
            780.0,
            vec![egui::Event::PointerMoved(picture.center())],
        );
        let shown = frame_sized(&mut app, &ctx, 780.0, vec![]);
        let buttons = controls(&shown);
        assert_eq!(buttons.len(), 2);
        assert!(
            buttons[0].left() < picture.left() && buttons[1].right() > picture.right(),
            "the arrows sit at the stage edges, outside this photo"
        );
        assert!(buttons[0].center().x < picture.center().x);
        assert!(buttons[1].center().x > picture.center().x);

        let original_buttons = buttons.clone();
        let next = buttons[1].center();
        frame_sized(&mut app, &ctx, 780.0, vec![egui::Event::PointerMoved(next)]);
        let hovered = frame_sized(&mut app, &ctx, 780.0, vec![]);
        assert_eq!(
            controls(&hovered).len(),
            2,
            "hovering an arrow keeps it visible"
        );
        frame_sized(&mut app, &ctx, 780.0, vec![primary(next, true)]);
        frame_sized(&mut app, &ctx, 780.0, vec![primary(next, false)]);
        assert_ne!(
            app.image_preview
                .as_ref()
                .unwrap()
                .path()
                .map(std::path::Path::to_owned),
            initial
        );
        assert!(app.image_preview.as_ref().unwrap().is_fit());
        preview_frame(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(picture.center())],
        );
        let shown = frame_sized(&mut app, &ctx, 780.0, vec![]);
        let buttons = controls(&shown);
        assert_eq!(buttons.len(), 2);
        assert_eq!(
            buttons, original_buttons,
            "changing photo preserves button positions"
        );
        let previous = buttons[0].center();
        frame_sized(
            &mut app,
            &ctx,
            780.0,
            vec![egui::Event::PointerMoved(previous)],
        );
        frame_sized(&mut app, &ctx, 780.0, vec![primary(previous, true)]);
        frame_sized(&mut app, &ctx, 780.0, vec![primary(previous, false)]);
        assert_eq!(
            app.image_preview
                .as_ref()
                .unwrap()
                .path()
                .map(std::path::Path::to_owned),
            initial
        );
        assert_eq!(app.composer, "unsent draft");
        app.actions.push(crate::model::Action::ImageActualSize);
        preview_frame(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(picture.center())],
        );
        let zoomed = frame_sized(&mut app, &ctx, 780.0, vec![]);
        assert_eq!(
            controls(&zoomed).len(),
            2,
            "a zoomed photo keeps its arrows"
        );
        let gone = frame_sized(&mut app, &ctx, 780.0, vec![egui::Event::PointerGone]);
        assert!(
            controls(&gone).is_empty(),
            "leaving the window hides the arrows"
        );
    }

    /// Opens the sample photo in the preview and waits for it to decode, so
    /// input meets a settled, fitted picture. Returns the fitted scale.
    fn open_sample_preview(app: &mut App, ctx: &egui::Context) -> f32 {
        app.attach(ctx);
        render(app, ctx);
        let (photo, _) = sample_files(app);
        let uri = crate::util::image_uri(&photo);
        app.actions.push(crate::model::Action::PreviewImage {
            card: None,
            path: photo,
            chat: crate::demo::SAMPLES[0].id.to_owned(),
            message: "ada-photo".to_owned(),
        });
        for _ in 0..200 {
            render(app, ctx);
            if matches!(
                ctx.try_load_texture(
                    &uri,
                    egui::TextureOptions::default(),
                    egui::SizeHint::default(),
                ),
                Ok(egui::load::TexturePoll::Ready { .. })
            ) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        render(app, ctx);
        app.image_preview.as_ref().unwrap().scale()
    }

    /// Runs one frame and returns where the preview drew its picture.
    fn preview_frame(app: &mut App, ctx: &egui::Context, events: Vec<egui::Event>) -> egui::Rect {
        picture_rect(&frame_sized(app, ctx, 780.0, events))
    }

    /// The largest textured shape: the chat behind shows the same file smaller.
    fn picture_rect(shapes: &[egui::epaint::ClippedShape]) -> egui::Rect {
        shapes
            .iter()
            .filter(|clipped| clipped.shape.texture_id() != egui::TextureId::default())
            .map(|clipped| clipped.shape.visual_bounding_rect())
            .max_by(|a, b| a.area().total_cmp(&b.area()))
            .expect("the preview draws its picture")
    }

    /// Moves the pointer to `pos` and turns the wheel with `modifiers` held;
    /// egui smooths the notch over several frames, so this runs until it has
    /// all arrived.
    fn wheel_at(
        app: &mut App,
        ctx: &egui::Context,
        pos: egui::Pos2,
        unit: egui::MouseWheelUnit,
        delta: f32,
        modifiers: egui::Modifiers,
    ) {
        preview_frame(
            app,
            ctx,
            vec![
                egui::Event::ModifiersChanged(modifiers),
                egui::Event::PointerMoved(pos),
                egui::Event::MouseWheel {
                    unit,
                    delta: egui::vec2(0.0, delta),
                    modifiers,
                    phase: egui::TouchPhase::Move,
                },
            ],
        );
        for _ in 0..40 {
            preview_frame(app, ctx, vec![]);
        }
    }

    fn primary(pos: egui::Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        }
    }

    fn double_click_at(app: &mut App, ctx: &egui::Context, pos: egui::Pos2) {
        preview_frame(
            app,
            ctx,
            vec![egui::Event::PointerMoved(pos), primary(pos, true)],
        );
        preview_frame(app, ctx, vec![primary(pos, false)]);
        preview_frame(app, ctx, vec![primary(pos, true)]);
        preview_frame(app, ctx, vec![primary(pos, false)]);
        render(app, ctx);
    }

    /// One notch of the mouse wheel zooms like the + button and the opposite
    /// notch undoes it, also with Ctrl/Cmd held, which egui turns into its own
    /// steeper zoom curve. The interface keeps its own zoom.
    #[test]
    fn a_wheel_notch_zooms_the_previewed_image_like_the_zoom_buttons() {
        let mut app = app();
        let ctx = egui::Context::default();
        let fitted = open_sample_preview(&mut app, &ctx);
        let interface_zoom = ctx.zoom_factor();
        let picture = preview_frame(&mut app, &ctx, vec![]);
        for modifiers in [egui::Modifiers::NONE, egui::Modifiers::COMMAND] {
            for (notch, expected) in [(1.0, fitted * 1.25), (-1.0, fitted)] {
                wheel_at(
                    &mut app,
                    &ctx,
                    picture.center(),
                    egui::MouseWheelUnit::Line,
                    notch,
                    modifiers,
                );
                let scale = app.image_preview.as_ref().unwrap().scale();
                assert!(
                    (scale - expected).abs() < 1e-4,
                    "{modifiers:?} notch {notch}: {scale}, not {expected}"
                );
            }
        }
        assert_eq!(ctx.zoom_factor(), interface_zoom);
    }

    /// egui keeps zooming a Ctrl/Cmd+wheel notch after the key comes up, so
    /// letting go early must still leave exactly one step.
    #[test]
    fn releasing_ctrl_mid_notch_still_zooms_one_step() {
        let mut app = app();
        let ctx = egui::Context::default();
        let fitted = open_sample_preview(&mut app, &ctx);
        let centre = preview_frame(&mut app, &ctx, vec![]).center();
        preview_frame(
            &mut app,
            &ctx,
            vec![
                egui::Event::ModifiersChanged(egui::Modifiers::COMMAND),
                egui::Event::PointerMoved(centre),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Line,
                    delta: egui::vec2(0.0, 1.0),
                    modifiers: egui::Modifiers::COMMAND,
                    phase: egui::TouchPhase::Move,
                },
            ],
        );
        for _ in 0..3 {
            preview_frame(&mut app, &ctx, vec![]);
        }
        preview_frame(
            &mut app,
            &ctx,
            vec![egui::Event::ModifiersChanged(egui::Modifiers::NONE)],
        );
        for _ in 0..40 {
            preview_frame(&mut app, &ctx, vec![]);
        }
        let scale = app.image_preview.as_ref().unwrap().scale();
        assert!(
            (scale - fitted * 1.25).abs() < 1e-4,
            "{scale}, not {}",
            fitted * 1.25
        );
    }

    /// A pinch follows the fingers, even with Ctrl/Cmd held.
    #[test]
    fn a_pinch_zooms_the_previewed_image_by_its_own_factor() {
        let mut app = app();
        let ctx = egui::Context::default();
        let fitted = open_sample_preview(&mut app, &ctx);
        let centre = preview_frame(&mut app, &ctx, vec![]).center();
        preview_frame(
            &mut app,
            &ctx,
            vec![
                egui::Event::ModifiersChanged(egui::Modifiers::COMMAND),
                egui::Event::PointerMoved(centre),
                egui::Event::Zoom(1.2),
            ],
        );
        let scale = app.image_preview.as_ref().unwrap().scale();
        assert!(
            (scale - fitted * 1.2).abs() < 1e-4,
            "{scale}, not {}",
            fitted * 1.2
        );
    }

    /// The batch-deletion demo references real messages from its chat.
    #[test]
    fn the_batch_delete_demo_uses_existing_messages() {
        let mut app = app();
        apply_flags(&mut app, Some("delete-selection"));
        let Some(Dialog::ConfirmDeleteSelection { chat, messages, .. }) = &app.dialog else {
            panic!("batch confirmation");
        };
        assert_eq!(messages.len(), 2);
        for id in messages {
            assert!(app.conversations[chat].message(id).is_some(), "{id}");
        }
    }

    /// The selection bar's Delete menu opens the batch confirmation for the
    /// whole selection, and offers Delete for everyone only when every
    /// selected message can still be revoked.
    #[test]
    fn the_selection_bar_deletes_the_whole_selection() {
        for own in [false, true] {
            let mut app = app();
            let chat = SAMPLES[0].id.to_owned();
            let ids = ["ada-format", "ada-reply"];
            if own {
                let conversation = app.conversations.get_mut(&chat).unwrap();
                for id in ids {
                    let message = conversation.message_mut(id).unwrap();
                    message.from_me = true;
                    message.timestamp = crate::util::now();
                }
            }
            app.selection = Some((chat.clone(), ids.map(str::to_owned).to_vec()));
            let ctx = egui::Context::default();
            ctx.enable_accesskit();
            app.attach(&ctx);
            for _ in 0..3 {
                render(&mut app, &ctx);
            }
            let click = |app: &mut App, pos| {
                let event = |pressed| egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                };
                frame_with(app, &ctx, vec![egui::Event::PointerMoved(pos), event(true)]);
                frame_with(app, &ctx, vec![event(false)]);
            };
            let find = |app: &mut App, label: &str| {
                accessible_nodes(app, &ctx, Vec::new())
                    .into_iter()
                    .find(|(text, _, _)| text == label)
                    .map(|(_, _, pos)| pos)
            };
            let delete = find(&mut app, "Delete…").expect("the selection bar offers Delete");
            click(&mut app, delete);
            render(&mut app, &ctx);
            let everyone = find(&mut app, "Delete for everyone").expect("menu item");
            click(&mut app, everyone);
            if own {
                assert_eq!(
                    app.dialog,
                    Some(Dialog::ConfirmDeleteSelection {
                        chat: chat.clone(),
                        messages: ids.map(str::to_owned).to_vec(),
                        for_everyone: true,
                    })
                );
                continue;
            }
            assert!(
                app.dialog.is_none(),
                "others' messages cannot be deleted for everyone"
            );
            render(&mut app, &ctx);
            let pos = match find(&mut app, "Delete for me") {
                Some(pos) => pos,
                None => {
                    click(&mut app, delete);
                    render(&mut app, &ctx);
                    find(&mut app, "Delete for me").expect("menu item")
                }
            };
            click(&mut app, pos);
            assert_eq!(
                app.dialog,
                Some(Dialog::ConfirmDeleteSelection {
                    chat: chat.clone(),
                    messages: ids.map(str::to_owned).to_vec(),
                    for_everyone: false,
                })
            );
        }
    }

    /// macOS and Wayland report trackpad scrolling in points; a two-finger
    /// scroll moves a picture larger than the area.
    #[test]
    fn a_trackpad_scroll_moves_the_previewed_image_instead_of_zooming() {
        let mut app = app();
        let ctx = egui::Context::default();
        open_sample_preview(&mut app, &ctx);
        app.actions.push(crate::model::Action::ImageActualSize);
        render(&mut app, &ctx);
        let before = preview_frame(&mut app, &ctx, vec![]);
        wheel_at(
            &mut app,
            &ctx,
            before.center(),
            egui::MouseWheelUnit::Point,
            -60.0,
            egui::Modifiers::NONE,
        );
        let after = preview_frame(&mut app, &ctx, vec![]);
        assert!(
            before.top() - after.top() > 30.0,
            "{before:?} did not scroll to {after:?}"
        );
        let preview = app.image_preview.as_ref().unwrap();
        assert!(!preview.is_fit());
        assert_eq!(preview.zoom(), 1.0);
    }

    /// Zooming with the wheel keeps the picture point under the pointer in
    /// place, as laid out on screen.
    #[test]
    fn wheel_zoom_keeps_the_pointed_at_pixel_under_the_pointer() {
        let mut app = app();
        let ctx = egui::Context::default();
        open_sample_preview(&mut app, &ctx);
        // A fitted picture fills the area's height, centred: its centre is
        // the area's.
        let pointer = preview_frame(&mut app, &ctx, vec![]).center() + egui::vec2(-200.0, -150.0);
        // Larger than the area both ways, so each axis can scroll.
        app.actions.push(crate::model::Action::ImageActualSize);
        app.actions.push(crate::model::Action::ZoomImageIn);
        render(&mut app, &ctx);
        let before = preview_frame(&mut app, &ctx, vec![]);
        let spot = (pointer - before.min) / before.size();
        wheel_at(
            &mut app,
            &ctx,
            pointer,
            egui::MouseWheelUnit::Line,
            1.0,
            egui::Modifiers::NONE,
        );
        let after = preview_frame(&mut app, &ctx, vec![]);
        assert!(after.width() > before.width() * 1.2, "the wheel zoomed");
        let drift = after.min + spot * after.size() - pointer;
        assert!(drift.length() < 1.5, "the pixel drifted by {drift:?}");
    }

    /// A picture larger than the area follows a mouse drag.
    #[test]
    fn dragging_moves_a_zoomed_picture() {
        let mut app = app();
        let ctx = egui::Context::default();
        open_sample_preview(&mut app, &ctx);
        let start = preview_frame(&mut app, &ctx, vec![]).center();
        app.actions.push(crate::model::Action::ImageActualSize);
        render(&mut app, &ctx);
        let before = preview_frame(&mut app, &ctx, vec![]);
        preview_frame(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(start), primary(start, true)],
        );
        let end = start - egui::vec2(0.0, 150.0);
        for step in 1..=10 {
            let pos = start - egui::vec2(0.0, 15.0 * step as f32);
            preview_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(pos)]);
        }
        preview_frame(&mut app, &ctx, vec![primary(end, false)]);
        let after = preview_frame(&mut app, &ctx, vec![]);
        assert!(
            before.top() - after.top() > 100.0,
            "{before:?} did not follow the drag to {after:?}"
        );
    }

    /// Double-clicking switches between fitting and the original size, as
    /// the header's Fit/% control does.
    #[test]
    fn double_clicking_the_picture_toggles_fit_and_original_size() {
        let mut app = app();
        let ctx = egui::Context::default();
        open_sample_preview(&mut app, &ctx);
        let centre = preview_frame(&mut app, &ctx, vec![]).center();
        double_click_at(&mut app, &ctx, centre);
        let preview = app.image_preview.as_ref().unwrap();
        assert!(!preview.is_fit());
        assert_eq!(preview.zoom(), 1.0);
        // Frames are 1/60 s apart: wait out egui's triple-click window
        // (twice the double-click delay) so the next two clicks are a
        // double click of their own.
        for _ in 0..14 {
            render(&mut app, &ctx);
        }
        double_click_at(&mut app, &ctx, centre);
        assert!(app.image_preview.as_ref().unwrap().is_fit());
    }

    /// The original size opens with the double-clicked point in the middle
    /// of the area rather than wherever the top left lands.
    #[test]
    fn double_click_centres_the_clicked_point_at_original_size() {
        let mut app = app();
        let ctx = egui::Context::default();
        open_sample_preview(&mut app, &ctx);
        // Fitted, the picture spans the area's height, centred.
        let fitted = preview_frame(&mut app, &ctx, vec![]);
        let spot = egui::vec2(0.5, 0.4);
        double_click_at(&mut app, &ctx, fitted.min + spot * fitted.size());
        let actual = preview_frame(&mut app, &ctx, vec![]);
        let clicked = actual.min + spot * actual.size();
        assert!(
            (clicked - fitted.center()).length() < 1.5,
            "{clicked:?} is not the centre {:?}",
            fitted.center()
        );
    }

    #[test]
    fn colon_starts_emoji_autocomplete_in_the_composer() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);

        frame_with(&mut app, &ctx, vec![egui::Event::Text(":".into())]);
        render(&mut app, &ctx);

        assert_eq!(app.composer, ":");
        assert_eq!(app.emoji_start, Some(0));
        assert_eq!(app.picker, None);
        assert!(ctx.memory(|memory| memory.has_focus(egui::Id::new("composer-text"))));
    }

    #[test]
    fn emoji_autocomplete_selects_with_the_keyboard() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);

        frame_with(&mut app, &ctx, vec![egui::Event::Text(":".into())]);
        frame_with(&mut app, &ctx, vec![egui::Event::Text("gri".into())]);
        render(&mut app, &ctx);
        assert_eq!(app.composer, ":gri");
        assert_eq!(app.emoji_selected, 0, "a new query selects its first match");
        frame_with(
            &mut app,
            &ctx,
            vec![key(egui::Key::ArrowDown, egui::Modifiers::NONE)],
        );
        assert_eq!(app.emoji_selected, 1);
        frame_with(
            &mut app,
            &ctx,
            vec![key(egui::Key::Enter, egui::Modifiers::NONE)],
        );
        render(&mut app, &ctx);

        assert!(!app.composer.is_empty() && !app.composer.contains(':'));
        assert!(app.emoji_start.is_none());
        assert_eq!(app.picker, None);
        assert_eq!(app.settings.recent_emoji.first(), Some(&app.composer));
        assert!(ctx.memory(|memory| memory.has_focus(egui::Id::new("composer-text"))));
    }

    #[test]
    fn enter_keeps_its_normal_behavior_when_no_emoji_matches() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);

        frame_with(&mut app, &ctx, vec![egui::Event::Text(":".into())]);
        frame_with(
            &mut app,
            &ctx,
            vec![egui::Event::Text("notarealemojiquery".into())],
        );
        render(&mut app, &ctx);
        frame_with(
            &mut app,
            &ctx,
            vec![key(egui::Key::Enter, egui::Modifiers::NONE)],
        );

        assert!(app.composer.is_empty(), "Enter sends the literal text");
        assert!(app.emoji_start.is_none());
    }

    #[test]
    fn escape_dismisses_emoji_autocomplete_without_changing_text() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);

        frame_with(&mut app, &ctx, vec![egui::Event::Text(":".into())]);
        frame_with(&mut app, &ctx, vec![egui::Event::Text("gri".into())]);
        render(&mut app, &ctx);
        frame_with(
            &mut app,
            &ctx,
            vec![key(egui::Key::Escape, egui::Modifiers::NONE)],
        );
        render(&mut app, &ctx);

        assert_eq!(app.composer, ":gri");
        assert!(app.emoji_start.is_none());
        assert!(ctx.memory(|memory| memory.has_focus(egui::Id::new("composer-text"))));
    }

    #[test]
    fn space_ends_emoji_autocomplete_as_literal_text() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);

        frame_with(&mut app, &ctx, vec![egui::Event::Text(":".into())]);
        frame_with(&mut app, &ctx, vec![egui::Event::Text("gri".into())]);
        frame_with(&mut app, &ctx, vec![egui::Event::Text(" ".into())]);

        assert_eq!(app.composer, ":gri ");
        assert!(app.emoji_start.is_none());
    }

    #[test]
    fn configured_send_shortcut_bypasses_emoji_autocomplete() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.settings.enter_sends = false;
        app.attach(&ctx);
        render(&mut app, &ctx);

        frame_with(&mut app, &ctx, vec![egui::Event::Text(":".into())]);
        frame_with(&mut app, &ctx, vec![egui::Event::Text("gri".into())]);
        render(&mut app, &ctx);
        frame_with(
            &mut app,
            &ctx,
            vec![key(egui::Key::Enter, egui::Modifiers::COMMAND)],
        );

        assert!(app.composer.is_empty(), "Ctrl+Enter still sends");
        assert!(app.emoji_start.is_none());
    }

    #[test]
    fn escape_returns_from_search_to_the_composer() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);

        frame_with(
            &mut app,
            &ctx,
            vec![key(egui::Key::K, egui::Modifiers::COMMAND)],
        );
        render(&mut app, &ctx);
        assert!(ctx.memory(|memory| memory.has_focus(egui::Id::new("chat-search"))));

        frame_with(&mut app, &ctx, vec![egui::Event::Text("ada".into())]);
        assert_eq!(app.search, "ada");
        frame_with(
            &mut app,
            &ctx,
            vec![key(egui::Key::Escape, egui::Modifiers::NONE)],
        );
        render(&mut app, &ctx);

        assert!(app.search.is_empty());
        assert!(ctx.memory(|memory| memory.has_focus(egui::Id::new("composer-text"))));
    }

    #[test]
    fn arrows_pick_a_chat_search_result_and_enter_opens_it_for_typing() {
        let mut app = app();
        let mut first = Chat::new(
            "491700009001@s.whatsapp.net".into(),
            "Forsaken Alpha".into(),
        );
        first.last_activity = 2_000_000_000;
        let mut second = Chat::new("491700009002@s.whatsapp.net".into(), "Forsaken Beta".into());
        second.last_activity = first.last_activity - 1;
        app.chats.extend([first, second]);
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        frame_with(
            &mut app,
            &ctx,
            vec![key(egui::Key::K, egui::Modifiers::COMMAND)],
        );
        render(&mut app, &ctx);
        frame_with(&mut app, &ctx, vec![egui::Event::Text("Forsaken".into())]);
        assert_eq!(app.search, "Forsaken");

        let matches: Vec<_> = app
            .visible_chats()
            .into_iter()
            .map(|chat| chat.id.clone())
            .collect();
        assert_eq!(matches.len(), 2, "only the two fixtures match");
        // Shift+↓ keeps selecting text in the field.
        frame_with(
            &mut app,
            &ctx,
            vec![key(egui::Key::ArrowDown, egui::Modifiers::SHIFT)],
        );
        assert_eq!(app.search_selected, None);

        frame_with(
            &mut app,
            &ctx,
            vec![key(egui::Key::ArrowDown, egui::Modifiers::NONE)],
        );
        assert_eq!(app.search_selected.as_ref(), matches.first());
        frame_with(
            &mut app,
            &ctx,
            vec![key(egui::Key::ArrowDown, egui::Modifiers::NONE)],
        );
        assert_eq!(app.search_selected.as_ref(), matches.get(1));

        frame_with(
            &mut app,
            &ctx,
            vec![key(egui::Key::Enter, egui::Modifiers::NONE)],
        );
        render(&mut app, &ctx);

        assert_eq!(app.open_chat.as_ref(), matches.get(1));
        assert!(app.search.is_empty());
        assert!(ctx.memory(|memory| memory.has_focus(egui::Id::new("composer-text"))));
    }

    #[test]
    fn at_sign_selects_a_group_member_without_leaving_the_composer() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        app.actions
            .push(crate::model::Action::OpenChat(SAMPLES[1].id.into()));
        render(&mut app, &ctx);

        frame_with(&mut app, &ctx, vec![egui::Event::Text("@".into())]);
        frame_with(&mut app, &ctx, vec![egui::Event::Text("mi".into())]);
        frame_with(
            &mut app,
            &ctx,
            vec![key(egui::Key::Enter, egui::Modifiers::NONE)],
        );

        assert_eq!(app.composer, "@Mira ");
        assert!(app.mention_start.is_none());
        assert!(ctx.memory(|memory| memory.has_focus(egui::Id::new("composer-text"))));
    }

    #[test]
    fn right_click_anywhere_on_a_message_opens_its_menu() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        for _ in 0..3 {
            render(&mut app, &ctx);
        }
        let chat = sample_ids()[0].to_owned();
        // Test right-click through an inner link-preview response.
        let id = crate::ui::conversation::bubble_id(&chat, "ada-link");
        let rect = ctx
            .read_response(id)
            .expect("the link message is on screen")
            .rect;
        let on_card = rect.left_top() + egui::vec2(rect.width() / 2.0, 40.0);
        let button = |pressed| egui::Event::PointerButton {
            pos: on_card,
            button: egui::PointerButton::Secondary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame_with(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(on_card), button(true)],
        );
        frame_with(&mut app, &ctx, vec![button(false)]);
        let popup = id.with("popup");
        assert!(
            egui::Popup::is_id_open(&ctx, popup),
            "a right-click on the preview card opens the message menu"
        );
        render(&mut app, &ctx);
        assert!(egui::Popup::is_id_open(&ctx, popup), "and it stays open");
    }

    /// #240: a right-click in the empty strip beside a bubble opens that
    /// message's menu, on either side of the chat.
    #[test]
    fn right_click_beside_a_message_opens_its_menu() {
        let chat = sample_ids()[0].to_owned();
        for (message, own) in [("ada-voice", false), ("ada-doc", true)] {
            let mut app = app();
            let ctx = egui::Context::default();
            app.attach(&ctx);
            for _ in 0..3 {
                render(&mut app, &ctx);
            }
            let id = crate::ui::conversation::bubble_id(&chat, message);
            let rect = ctx
                .data(|data| data.get_temp::<egui::Rect>(id.with("rect")))
                .unwrap_or_else(|| panic!("{message} is on screen"));
            let beside = if own {
                egui::pos2(rect.left() - 60.0, rect.center().y)
            } else {
                egui::pos2(rect.right() + 60.0, rect.center().y)
            };
            let button = |pressed| egui::Event::PointerButton {
                pos: beside,
                button: egui::PointerButton::Secondary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            frame_with(
                &mut app,
                &ctx,
                vec![egui::Event::PointerMoved(beside), button(true)],
            );
            frame_with(&mut app, &ctx, vec![button(false)]);
            render(&mut app, &ctx);
            assert!(
                egui::Popup::is_id_open(&ctx, id.with("popup")),
                "a right-click beside {message} opens its menu"
            );
        }
    }

    #[test]
    fn ctrl_click_selects_messages_and_shift_click_takes_the_ones_between() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        for _ in 0..3 {
            render(&mut app, &ctx);
        }
        let chat = sample_ids()[0].to_owned();
        // A click in the bubble's margin, beside its time: the text and
        // media inside take clicks for themselves.
        let click = |app: &mut App, message: &str, modifiers: egui::Modifiers| {
            let id = crate::ui::conversation::bubble_id(&chat, message);
            let rect = ctx
                .data(|data| data.get_temp::<egui::Rect>(id.with("rect")))
                .unwrap_or_else(|| panic!("{message} is on screen"));
            let pos = rect.right_bottom() - egui::vec2(5.0, 2.0);
            let button = |pressed| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers,
            };
            frame_with(
                app,
                &ctx,
                vec![
                    egui::Event::ModifiersChanged(modifiers),
                    egui::Event::PointerMoved(pos),
                    button(true),
                ],
            );
            frame_with(app, &ctx, vec![button(false)]);
            frame_with(
                app,
                &ctx,
                vec![egui::Event::ModifiersChanged(egui::Modifiers::NONE)],
            );
        };
        let selected = |app: &App| app.selection.as_ref().map(|(_, ids)| ids.clone());
        click(&mut app, "ada-doc", egui::Modifiers::NONE);
        assert_eq!(selected(&app), None, "a plain click selects nothing");
        click(&mut app, "ada-doc", egui::Modifiers::COMMAND);
        assert_eq!(selected(&app), Some(vec!["ada-doc".to_owned()]));
        click(&mut app, "ada-reply", egui::Modifiers::SHIFT);
        assert_eq!(
            selected(&app),
            Some(
                ["ada-doc", "ada-voice", "you-voice", "ada-reply"]
                    .map(str::to_owned)
                    .to_vec()
            ),
            "Shift-click takes every message between"
        );
        click(&mut app, "ada-voice", egui::Modifiers::NONE);
        assert_eq!(
            selected(&app),
            Some(
                ["ada-doc", "you-voice", "ada-reply"]
                    .map(str::to_owned)
                    .to_vec()
            ),
            "a click while selecting leaves one out"
        );
        frame_with(
            &mut app,
            &ctx,
            vec![key(egui::Key::Escape, egui::Modifiers::NONE)],
        );
        frame_with(&mut app, &ctx, Vec::new());
        assert_eq!(selected(&app), None, "Escape ends the selection");
        assert_eq!(app.open_chat, Some(chat), "and leaves the chat open");
    }

    /// A chat of `count` short text messages, alternating sides from an
    /// outgoing first one, opened and drawn at its end.
    fn sweep_chat(count: u32) -> (App, egui::Context, String) {
        let mut app = app();
        let chat = sample_ids()[0].to_owned();
        app.conversations.get_mut(&chat).unwrap().messages = (0..count)
            .map(|i| {
                message(
                    &chat,
                    &format!("m{i:03}"),
                    i % 2 == 0,
                    1_700_000_000 + i64::from(i),
                    Content::text(format!("message number {i}")),
                )
            })
            .collect();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        render(&mut app, &ctx);
        (app, ctx, chat)
    }

    fn drawn_rect(ctx: &egui::Context, chat: &str, message: &str, part: &str) -> egui::Rect {
        let id = crate::ui::conversation::bubble_id(chat, message).with(part);
        ctx.data(|data| data.get_temp::<egui::Rect>(id))
            .unwrap_or_else(|| panic!("{message} is on screen"))
    }

    /// A point in the empty strip beside a message's bubble.
    fn beside(ctx: &egui::Context, chat: &str, message: &str, from_me: bool) -> egui::Pos2 {
        let rect = drawn_rect(ctx, chat, message, "rect");
        let x = if from_me {
            rect.left() - 100.0
        } else {
            rect.right() + 100.0
        };
        // Where the strip took input last, which a history still settling
        // may have moved from the bubble's last rect.
        let row = ctx
            .read_response(crate::ui::conversation::bubble_id(chat, message).with("row"))
            .map_or(rect, |row| row.rect);
        egui::pos2(x, row.center().y)
    }

    fn selected_ids(app: &App) -> Vec<String> {
        app.selection
            .as_ref()
            .map(|(_, ids)| ids.clone())
            .unwrap_or_default()
    }

    fn text_selected(ctx: &egui::Context) -> bool {
        ctx.plugin_opt::<egui::text_selection::LabelSelectionState>()
            .is_some_and(|plugin| plugin.lock().has_selection())
    }

    /// #246: while selecting, a drag over messages sweeps them into the
    /// selection, text included, and dragging back leaves rows out again.
    #[test]
    fn a_drag_while_selecting_sweeps_messages_over_their_text() {
        let (mut app, ctx, chat) = sweep_chat(8);
        app.actions
            .push(crate::model::Action::SelectMessage("m001".into()));
        render(&mut app, &ctx);
        let body = |id: &str| drawn_rect(&ctx, &chat, id, "body").center();
        let (from, via, to) = (body("m003"), body("m004"), body("m005"));
        frame_with(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(from), primary(from, true)],
        );
        for pos in [via, to, to] {
            frame_with(&mut app, &ctx, vec![egui::Event::PointerMoved(pos)]);
        }
        assert_eq!(
            selected_ids(&app),
            ["m001", "m003", "m004", "m005"],
            "the sweep adds every row from the press to the pointer"
        );
        assert!(!text_selected(&ctx), "the drag swept messages, not text");
        // Back over the row it began on: the rows passed again drop out.
        for _ in 0..2 {
            frame_with(&mut app, &ctx, vec![egui::Event::PointerMoved(from)]);
        }
        assert_eq!(selected_ids(&app), ["m001", "m003"]);
        frame_with(&mut app, &ctx, vec![egui::Event::PointerMoved(via)]);
        frame_with(&mut app, &ctx, vec![primary(via, false)]);
        render(&mut app, &ctx);
        assert_eq!(selected_ids(&app), ["m001", "m003", "m004"]);
        assert!(app.sweep.is_none(), "releasing ends the sweep");
        // Moving without the button does not sweep further.
        frame_with(&mut app, &ctx, vec![egui::Event::PointerMoved(to)]);
        render(&mut app, &ctx);
        assert_eq!(selected_ids(&app), ["m001", "m003", "m004"]);
        // A click still toggles, and Shift-click still takes a range.
        let click = |app: &mut App, pos: egui::Pos2, modifiers: egui::Modifiers| {
            let button = |pressed| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers,
            };
            frame_with(
                app,
                &ctx,
                vec![
                    egui::Event::ModifiersChanged(modifiers),
                    egui::Event::PointerMoved(pos),
                    button(true),
                ],
            );
            frame_with(app, &ctx, vec![button(false)]);
            frame_with(
                app,
                &ctx,
                vec![egui::Event::ModifiersChanged(egui::Modifiers::NONE)],
            );
            render(app, &ctx);
        };
        click(&mut app, body("m003"), egui::Modifiers::NONE);
        assert_eq!(selected_ids(&app), ["m001", "m004"]);
        click(&mut app, body("m006"), egui::Modifiers::SHIFT);
        // From the message clicked last, as before.
        assert_eq!(selected_ids(&app), ["m001", "m003", "m004", "m005", "m006"]);
    }

    /// #246: outside selection mode, a drag that starts beside the bubbles,
    /// off the text, starts selecting and sweeps the messages it passes. A
    /// drag over the text still selects the text.
    #[test]
    fn a_drag_beside_the_bubbles_starts_selecting_messages() {
        let (mut app, ctx, chat) = sweep_chat(8);
        // Over the text, a drag selects text as before.
        let text = drawn_rect(&ctx, &chat, "m002", "body");
        let (start, end) = (
            egui::pos2(text.left() + 2.0, text.center().y),
            text.center(),
        );
        frame_with(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(start), primary(start, true)],
        );
        frame_with(&mut app, &ctx, vec![egui::Event::PointerMoved(end)]);
        frame_with(&mut app, &ctx, vec![egui::Event::PointerMoved(end)]);
        frame_with(&mut app, &ctx, vec![primary(end, false)]);
        assert!(text_selected(&ctx), "the drag over the text selects text");
        assert!(app.selection.is_none(), "and no messages");
        render(&mut app, &ctx);
        // Beside the bubbles, it sweeps messages instead.
        let from = beside(&ctx, &chat, "m003", false);
        let to = beside(&ctx, &chat, "m005", false);
        let via = beside(&ctx, &chat, "m004", true);
        frame_with(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(from), primary(from, true)],
        );
        for pos in [via, to, to] {
            frame_with(&mut app, &ctx, vec![egui::Event::PointerMoved(pos)]);
        }
        frame_with(&mut app, &ctx, vec![primary(to, false)]);
        render(&mut app, &ctx);
        assert_eq!(selected_ids(&app), ["m003", "m004", "m005"]);
        assert!(app.sweep.is_none());
        // A click beside a bubble in selection mode still toggles it.
        let pos = beside(&ctx, &chat, "m004", true);
        frame_with(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(pos), primary(pos, true)],
        );
        frame_with(&mut app, &ctx, vec![primary(pos, false)]);
        render(&mut app, &ctx);
        assert_eq!(selected_ids(&app), ["m003", "m005"]);
    }

    /// #246: a sweep held at the top edge scrolls the list, and takes every
    /// message it passes, rows the list only estimated included.
    #[test]
    fn a_sweep_held_at_the_top_edge_scrolls_and_selects_what_it_passes() {
        let (mut app, ctx, chat) = sweep_chat(400);
        // Let the rows measured for the first time settle.
        let mut last = None;
        for _ in 0..20 {
            render(&mut app, &ctx);
            let rect = drawn_rect(&ctx, &chat, "m398", "rect");
            if last == Some(rect) {
                break;
            }
            last = Some(rect);
        }
        let view = app
            .selection_view
            .lock()
            .expect("the view rect")
            .expect("the conversation was drawn");
        let on_screen = (0..400)
            .filter(|i| {
                let id = crate::ui::conversation::bubble_id(&chat, &format!("m{i:03}"));
                ctx.data(|data| data.get_temp::<egui::Rect>(id.with("rect")))
                    .is_some_and(|rect| view.intersects(rect))
            })
            .count();
        let from = beside(&ctx, &chat, "m398", true);
        let hold = egui::pos2(from.x, view.top() + 4.0);
        frame_with(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(from), primary(from, true)],
        );
        for _ in 0..120 {
            frame_with(&mut app, &ctx, vec![egui::Event::PointerMoved(hold)]);
        }
        frame_with(&mut app, &ctx, vec![primary(hold, false)]);
        render(&mut app, &ctx);
        let ids = selected_ids(&app);
        assert_eq!(ids.last().map(String::as_str), Some("m398"), "{ids:?}");
        assert!(
            ids.len() > on_screen + 5,
            "the sweep scrolled past the first screen: {} of {on_screen}",
            ids.len()
        );
        // Every message between its ends, in order, with none left out.
        let first: usize = ids[0][1..].parse().expect("a numbered id");
        let expected: Vec<String> = (first..=398).map(|i| format!("m{i:03}")).collect();
        assert_eq!(ids, expected);
        assert!(!app.scroll_to_bottom, "heading up releases the pin");
    }

    /// #241: while selecting, a click on a message's text or beside its
    /// bubble adds it, not only a click on the bubble's padding.
    #[test]
    fn while_selecting_a_click_on_the_text_or_beside_it_selects() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        for _ in 0..3 {
            render(&mut app, &ctx);
        }
        let chat = sample_ids()[0].to_owned();
        app.actions
            .push(crate::model::Action::SelectMessage("ada-doc".into()));
        render(&mut app, &ctx);
        let rect = |message: &str, part: &str| {
            let id = crate::ui::conversation::bubble_id(&chat, message).with(part);
            ctx.data(|data| data.get_temp::<egui::Rect>(id))
                .unwrap_or_else(|| panic!("{message} is on screen"))
        };
        let click = |app: &mut App, pos: egui::Pos2| {
            let button = |pressed| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            frame_with(
                app,
                &ctx,
                vec![egui::Event::PointerMoved(pos), button(true)],
            );
            frame_with(app, &ctx, vec![button(false)]);
            render(app, &ctx);
        };
        let selected = |app: &App| {
            app.selection
                .as_ref()
                .map(|(_, ids)| ids.clone())
                .unwrap_or_default()
        };
        // The middle of the text, where the body takes clicks and drags.
        click(&mut app, rect("ada-reply", "body").center());
        assert_eq!(selected(&app), ["ada-doc", "ada-reply"], "the text selects");
        // The empty strip to the right of an incoming bubble.
        let voice = rect("ada-voice", "rect");
        click(&mut app, egui::pos2(voice.right() + 60.0, voice.center().y));
        assert_eq!(
            selected(&app),
            ["ada-doc", "ada-voice", "ada-reply"],
            "the strip beside the bubble selects"
        );
        // And a second click on the text leaves the message out again.
        click(&mut app, rect("ada-reply", "body").center());
        assert_eq!(selected(&app), ["ada-doc", "ada-voice"]);
    }

    /// While selecting, as in WhatsApp Web, every row has a check box in a
    /// column on the left, the whole width of the row picks it, and the
    /// selection stays open with nothing in it.
    #[test]
    fn while_selecting_every_row_has_a_check_box_on_the_left() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        for _ in 0..3 {
            render(&mut app, &ctx);
        }
        let chat = sample_ids()[0].to_owned();
        let check = |message: &str| {
            let id = crate::ui::conversation::bubble_id(&chat, message).with("check");
            ctx.data(|data| data.get_temp::<egui::Rect>(id))
        };
        assert!(
            check("ada-reply").is_none(),
            "no check boxes outside a selection"
        );
        app.actions.push(crate::model::Action::StartSelection);
        render(&mut app, &ctx);
        render(&mut app, &ctx);
        assert_eq!(app.selection, Some((chat.clone(), Vec::new())));
        let rect = |message: &str| {
            let id = crate::ui::conversation::bubble_id(&chat, message).with("rect");
            ctx.data(|data| data.get_temp::<egui::Rect>(id))
                .unwrap_or_else(|| panic!("{message} is on screen"))
        };
        for (message, own) in [
            ("ada-voice", false),
            ("you-voice", true),
            ("ada-reply", false),
        ] {
            let boxed = check(message).unwrap_or_else(|| panic!("{message} has a check box"));
            let bubble = rect(message);
            assert!(
                boxed.right() < bubble.left(),
                "{message}'s check box sits left of its bubble"
            );
            assert!(bubble.y_range().contains(boxed.center().y));
            if own {
                assert!(
                    bubble.right() > rect("ada-voice").right() + 50.0,
                    "an own bubble stays on the right"
                );
            }
        }
        let click = |app: &mut App, pos: egui::Pos2| {
            let button = |pressed| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            frame_with(
                app,
                &ctx,
                vec![egui::Event::PointerMoved(pos), button(true)],
            );
            frame_with(app, &ctx, vec![button(false)]);
            render(app, &ctx);
        };
        click(&mut app, check("ada-voice").unwrap().center());
        assert_eq!(selected_ids(&app), ["ada-voice"], "the check box picks");
        // The view's margin, left of the check box.
        let margin = check("ada-reply").unwrap().left_center() - egui::vec2(10.0, 0.0);
        click(&mut app, margin);
        assert_eq!(
            selected_ids(&app),
            ["ada-voice", "ada-reply"],
            "the margin picks too"
        );
        click(&mut app, check("ada-voice").unwrap().center());
        click(&mut app, check("ada-reply").unwrap().center());
        assert_eq!(
            app.selection,
            Some((chat.clone(), Vec::new())),
            "unticking every message keeps selecting"
        );
    }

    #[test]
    fn message_checkboxes_describe_the_message_and_checked_state() {
        let mut app = app();
        let chat = sample_ids()[0].to_owned();
        let conversation = app.conversations.get_mut(&chat).unwrap();
        conversation
            .messages
            .retain(|row| matches!(row.id.as_str(), "ada-voice" | "you-voice" | "ada-reply"));
        app.selection = Some((chat.clone(), vec!["ada-voice".into()]));
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        app.attach(&ctx);
        for _ in 0..3 {
            render(&mut app, &ctx);
        }
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1180.0, 780.0),
                )),
                ..Default::default()
            },
            |ui| app.frame_ui(ui),
        );
        output.textures_delta.clear();
        let tree = output.platform_output.accesskit_update.unwrap();
        let boxes: Vec<_> = tree
            .nodes
            .iter()
            .filter(|(_, node)| node.role() == egui::accesskit::Role::CheckBox)
            .map(|(_, node)| node)
            .collect();
        assert_eq!(boxes.len(), 3);
        let labels: std::collections::HashSet<_> =
            boxes.iter().map(|node| node.label().unwrap()).collect();
        assert_eq!(labels.len(), 3, "each message has a distinct label");
        for row in &app.conversations[&chat].messages {
            let node = boxes
                .iter()
                .find(|node| node.label().unwrap().contains(&row.content.summary()))
                .expect("label identifies the message content");
            let label = node.label().unwrap();
            let sender = if row.from_me {
                "You".to_owned()
            } else {
                app.display_name_or(&row.sender, row.sender_name.as_deref())
            };
            assert!(label.contains(&sender), "label identifies the sender");
            assert!(label.contains(&crate::util::moment_stamp(app.locale, row.timestamp)));
            assert_eq!(
                node.toggled(),
                Some(if row.id == "ada-voice" {
                    egui::accesskit::Toggled::True
                } else {
                    egui::accesskit::Toggled::False
                })
            );
        }
    }

    #[test]
    fn ineligible_messages_have_neither_selection_boxes_nor_menu_actions() {
        for content in [
            Content::Revoked,
            Content::PhoneOnly {
                view_once: true,
                live_location: false,
                once: None,
            },
            Content::Unsupported {
                what: "Test".into(),
            },
            Content::Poll {
                question: "Lunch?".into(),
                options: vec!["Yes".into(), "No".into()],
                state: Default::default(),
            },
            Content::Interactive {
                text: "Choose an option".into(),
                card: None,
            },
            Content::text("A selectable message"),
        ] {
            let selectable = matches!(content, Content::Text { .. });
            let mut app = app();
            let chat = sample_ids()[0].to_owned();
            let conversation = app.conversations.get_mut(&chat).unwrap();
            conversation.messages.retain(|row| row.id == "ada-reply");
            conversation.messages[0].content = content;
            app.selection = Some((chat.clone(), Vec::new()));
            let ctx = egui::Context::default();
            ctx.enable_accesskit();
            app.attach(&ctx);
            for _ in 0..3 {
                render(&mut app, &ctx);
            }
            let check = crate::ui::conversation::bubble_id(&chat, "ada-reply").with("check");
            assert_eq!(
                ctx.data(|data| data.get_temp::<egui::Rect>(check).is_some()),
                selectable
            );
            let pick = crate::ui::conversation::bubble_id(&chat, "ada-reply").with("pick");
            assert_eq!(
                ctx.read_response(pick).unwrap().sense,
                if selectable {
                    egui::Sense::click_and_drag()
                } else {
                    egui::Sense::hover()
                },
                "only a visible checkbox can receive focus or start a sweep"
            );
            app.selection = None;
            render(&mut app, &ctx);
            app.open_message_menu = message_menu(&app, "ada-reply");
            render(&mut app, &ctx);
            let nodes = accessible_nodes(&mut app, &ctx, Vec::new());
            assert!(nodes.iter().any(|(label, _, _)| label == "Copy message ID"));
            for action in ["Select", "Forward"] {
                assert_eq!(
                    nodes.iter().any(|(label, _, _)| label == action),
                    selectable,
                    "only eligible messages offer {action}"
                );
            }
        }
    }

    #[test]
    fn message_checkboxes_reveal_keyboard_focus_and_outline_the_box() {
        let mut app = app();
        let chat = sample_ids()[0].to_owned();
        let conversation = app.conversations.get_mut(&chat).unwrap();
        let timestamp = conversation.messages.last().unwrap().timestamp;
        conversation.messages = (0..12)
            .map(|index| {
                message(
                    &chat,
                    &format!("focus-{index}"),
                    false,
                    timestamp + index,
                    Content::text("A short message for keyboard selection"),
                )
            })
            .collect();
        app.selection = Some((chat.clone(), Vec::new()));
        let ctx = egui::Context::default();
        app.attach(&ctx);
        for _ in 0..3 {
            frame_sized(&mut app, &ctx, 360.0, Vec::new());
        }
        frame_sized(&mut app, &ctx, 360.0, tab());
        let (message, pick) = app.conversations[&chat]
            .messages
            .iter()
            .find_map(|row| {
                let id = crate::ui::conversation::bubble_id(&chat, &row.id).with("pick");
                ctx.read_response(id)
                    .filter(|response| {
                        response.interact_rect.is_positive()
                            && response.interact_rect.y_range() != response.rect.y_range()
                    })
                    .map(|response| (row.id.clone(), response))
            })
            .expect("a partially clipped message checkbox is laid out");
        // Request focus inside the pass, as keyboard and accessibility
        // events do, so egui reports gained_focus to the scroll area.
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1180.0, 360.0),
                )),
                ..Default::default()
            },
            |ui| {
                pick.request_focus();
                app.background_frame(ui.ctx());
                app.frame_ui(ui);
            },
        );
        output.textures_delta.clear();
        for _ in 0..3 {
            frame_sized(&mut app, &ctx, 360.0, Vec::new());
        }
        assert_eq!(ctx.memory(|memory| memory.focused()), Some(pick.id));
        assert!(
            !app.scroll_to_bottom,
            "keyboard focus releases the bottom pin"
        );
        let focused = ctx.read_response(pick.id).unwrap();
        assert_eq!(focused.interact_rect.y_range(), focused.rect.y_range());
        let check = ctx
            .data(|data| {
                data.get_temp::<egui::Rect>(
                    crate::ui::conversation::bubble_id(&chat, &message).with("check"),
                )
            })
            .unwrap();
        let outline = ring(&ctx).expect("keyboard focus is visible");
        assert!(outline.contains_rect(check));
        assert!(outline.width() < check.width() + 12.0);
        let mut next_message = None;
        for _ in 0..50 {
            frame_sized(&mut app, &ctx, 360.0, tab());
            frame_sized(&mut app, &ctx, 360.0, Vec::new());
            let focused = ctx.memory(|memory| memory.focused());
            next_message = app.conversations[&chat].messages.iter().find_map(|row| {
                let id = crate::ui::conversation::bubble_id(&chat, &row.id).with("pick");
                (focused == Some(id) && row.id != message).then(|| row.id.clone())
            });
            if next_message.is_some() {
                break;
            }
        }
        let next_message = next_message.expect("Tab reaches another message checkbox");
        frame_sized(
            &mut app,
            &ctx,
            360.0,
            vec![key(egui::Key::Space, egui::Modifiers::NONE)],
        );
        assert_eq!(selected_ids(&app), [next_message]);
    }

    /// Where every copy of a label landed in the last frame.
    fn painted_label_centers(ctx: &egui::Context, needle: &str) -> Vec<egui::Pos2> {
        let mut found = Vec::new();
        let layers: Vec<_> = ctx.memory(|memory| memory.layer_ids().collect());
        for layer in layers {
            let transform = ctx.layer_transform_to_global(layer).unwrap_or_default();
            ctx.graphics(|graphics| {
                if let Some(list) = graphics.get(layer) {
                    for clipped in list.all_entries() {
                        if let egui::Shape::Text(text) = &clipped.shape
                            && text.galley.text() == needle
                        {
                            let rect = egui::Rect::from_min_size(text.pos, text.galley.size());
                            found.push(transform * rect.center());
                        }
                    }
                }
            });
        }
        found
    }

    /// Where a label landed in the last frame, read from the shapes egui
    /// still holds for this pass.
    fn painted_label_center(ctx: &egui::Context, needle: &str) -> Option<egui::Pos2> {
        let mut found = None;
        let layers: Vec<_> = ctx.memory(|memory| memory.layer_ids().collect());
        for layer in layers {
            let transform = ctx.layer_transform_to_global(layer).unwrap_or_default();
            ctx.graphics(|graphics| {
                if let Some(list) = graphics.get(layer) {
                    for clipped in list.all_entries() {
                        if let egui::Shape::Text(text) = &clipped.shape
                            && text.galley.text() == needle
                        {
                            let rect = egui::Rect::from_min_size(text.pos, text.galley.size());
                            found = Some(transform * rect.center());
                        }
                    }
                }
            });
        }
        found
    }

    /// The pinned page shows each pin's notice beside the message it names,
    /// rather than every notice together at the end of the transcript.
    #[test]
    fn the_pinned_page_spreads_its_notices_over_the_transcript() {
        let mut app = app();
        apply_flags(&mut app, Some("pinned"));
        let ctx = egui::Context::default();
        app.attach(&ctx);
        let mut notices = Vec::new();
        for _ in 0..3 {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1180.0, 780.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    let ctx = ui.ctx().clone();
                    app.background_frame(&ctx);
                    app.frame_ui(ui);
                    notices = painted_label_centers(&ctx, "You pinned a message");
                },
            );
            output.textures_delta.clear();
        }
        let mut ys: Vec<f32> = notices.iter().map(|pos| pos.y).collect();
        ys.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        assert!(
            ys.len() >= 2,
            "the page shows the notices of the pins on screen: {ys:?}"
        );
        assert!(
            ys[ys.len() - 1] - ys[0] > 40.0,
            "each notice sits with its own message, not beside the others: {ys:?}"
        );
    }

    #[test]
    fn the_bubble_menu_stars_the_message() {
        let mut app = app();
        app.backend.record_demo_commands();
        apply_flags(&mut app, Some("react-menu"));
        let ctx = egui::Context::default();
        app.attach(&ctx);
        let mut pos = None;
        for _ in 0..3 {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1180.0, 2400.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    let ctx = ui.ctx().clone();
                    app.background_frame(&ctx);
                    app.frame_ui(ui);
                    pos = painted_label_center(&ctx, "Star");
                },
            );
            output.textures_delta.clear();
        }
        let pos = pos.expect("Star is on the bubble menu");
        let button = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame_sized(
            &mut app,
            &ctx,
            2400.0,
            vec![egui::Event::PointerMoved(pos), button(true)],
        );
        frame_sized(&mut app, &ctx, 2400.0, vec![button(false)]);
        let chat = sample_ids()[0].to_owned();
        let commands = app.backend.take_demo_commands();
        assert!(
            commands.iter().any(|command| matches!(
                command,
                crate::backend::Command::SetStar {
                    chat: starred_chat,
                    message: starred_message,
                    starred: true,
                } if starred_chat == &chat && starred_message == "ada-link"
            )),
            "the bubble menu sends SetStar for that message: {commands:?}"
        );
    }

    #[test]
    fn selected_documents_use_full_rows_and_delete_the_complete_selection() {
        for (for_everyone, switch_chat) in
            [(true, false), (false, false), (true, true), (false, true)]
        {
            let mut app = app();
            let (backend, mut commands) = crate::backend::Backend::recording();
            app.backend = backend;
            let chat = sample_ids()[0].to_owned();
            let conversation = app.conversations.get_mut(&chat).unwrap();
            let mut document = conversation.message("ada-doc").unwrap().clone();
            if let Content::Document { media, .. } = &mut document.content {
                // Exercise the downloaded-file control that normally opens a PDF.
                media.path = Some(std::path::PathBuf::from("synthetic-selection-document.pdf"));
            }
            document.from_me = true;
            document.timestamp = crate::util::now();
            conversation.messages = ["pdf-1", "pdf-2", "pdf-3"]
                .map(|id| {
                    let mut message = document.clone();
                    message.id = id.into();
                    message
                })
                .to_vec();
            app.selection = Some((chat.clone(), vec!["pdf-1".into()]));
            let ctx = egui::Context::default();
            ctx.enable_accesskit();
            app.attach(&ctx);
            for _ in 0..3 {
                render(&mut app, &ctx);
            }
            let click = |app: &mut App, pos, button| {
                let event = |pressed| egui::Event::PointerButton {
                    pos,
                    button,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                };
                frame_with(app, &ctx, vec![egui::Event::PointerMoved(pos), event(true)]);
                frame_with(app, &ctx, vec![event(false)]);
            };
            let rect = |message: &str| {
                ctx.data(|data| {
                    data.get_temp::<egui::Rect>(
                        crate::ui::conversation::bubble_id(&chat, message).with("rect"),
                    )
                })
                .unwrap()
            };
            click(
                &mut app,
                rect("pdf-2").center(),
                egui::PointerButton::Primary,
            );
            let row = ctx
                .read_response(crate::ui::conversation::bubble_id(&chat, "pdf-3").with("pick"))
                .unwrap()
                .rect;
            click(
                &mut app,
                egui::pos2(row.left() + 15.0, row.center().y),
                egui::PointerButton::Primary,
            );
            assert_eq!(
                app.selection.as_ref().unwrap().1,
                ["pdf-1", "pdf-2", "pdf-3"]
            );
            click(
                &mut app,
                rect("pdf-2").center(),
                egui::PointerButton::Secondary,
            );
            let label = if for_everyone {
                "Delete for everyone"
            } else {
                "Delete for me"
            };
            let pos = accessible_nodes(&mut app, &ctx, Vec::new())
                .into_iter()
                .find(|(text, _, _)| text == label)
                .unwrap()
                .2;
            click(&mut app, pos, egui::PointerButton::Primary);
            assert_eq!(
                app.dialog,
                Some(Dialog::ConfirmDeleteSelection {
                    chat: chat.clone(),
                    messages: ["pdf-1", "pdf-2", "pdf-3"].map(str::to_owned).to_vec(),
                    for_everyone,
                })
            );
            assert_eq!(app.conversations[&chat].messages.len(), 3);
            if switch_chat {
                app.open_chat = Some(SAMPLES[1].id.to_owned());
            }
            let other_count = app.conversations[SAMPLES[1].id].messages.len();
            for _ in 0..3 {
                render(&mut app, &ctx);
            }
            let pos = accessible_nodes(&mut app, &ctx, Vec::new())
                .into_iter()
                .find(|(text, role, _)| text == "Delete" && *role == egui::accesskit::Role::Button)
                .unwrap()
                .2;
            click(&mut app, pos, egui::PointerButton::Primary);
            assert!(
                app.dialog.is_none(),
                "confirmation closes after Delete: {:?}",
                app.dialog
            );
            if for_everyone {
                assert!(
                    app.conversations[&chat]
                        .messages
                        .iter()
                        .all(|message| matches!(message.content, Content::Revoked))
                );
            } else {
                assert_eq!(
                    app.conversations[&chat].messages.len(),
                    3,
                    "local removal waits for WhatsApp acknowledgement"
                );
            }
            let requested: Vec<_> = std::iter::from_fn(|| commands.try_recv().ok())
                .filter_map(|command| match command {
                    crate::backend::Command::DeleteLocal { chat: target, id }
                        if !for_everyone && target == chat =>
                    {
                        Some(id)
                    }
                    crate::backend::Command::Revoke { chat: target, id }
                        if for_everyone && target == chat =>
                    {
                        Some(id)
                    }
                    _ => None,
                })
                .collect();
            assert_eq!(requested, ["pdf-1", "pdf-2", "pdf-3"]);
            assert!(app.selection.is_none());
            assert_eq!(
                app.conversations[SAMPLES[1].id].messages.len(),
                other_count,
                "confirmation only affects the chat that owned the selection"
            );
        }
    }

    /// One frame with AccessKit on; returns (label, role, centre) per node.
    fn accessible_nodes(
        app: &mut App,
        ctx: &egui::Context,
        events: Vec<egui::Event>,
    ) -> Vec<(String, egui::accesskit::Role, egui::Pos2)> {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1180.0, 780.0),
                )),
                events,
                ..Default::default()
            },
            |ui| {
                let ctx = ui.ctx().clone();
                app.background_frame(&ctx);
                app.frame_ui(ui);
            },
        );
        output.textures_delta.clear();
        let scale = ctx.pixels_per_point() as f64;
        output
            .platform_output
            .accesskit_update
            .map(|tree| {
                tree.nodes
                    .iter()
                    .filter_map(|(_, node)| {
                        let label = node.label().or_else(|| node.value())?.to_owned();
                        let bounds = node.bounds()?;
                        let centre = egui::pos2(
                            ((bounds.x0 + bounds.x1) / 2.0 / scale) as f32,
                            ((bounds.y0 + bounds.y1) / 2.0 / scale) as f32,
                        );
                        Some((label, node.role(), centre))
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The menu lists actions only: sent, delivery, and read times open from
    /// "Message info", and the message id is copied by a row that says so.
    #[test]
    fn message_menu_lists_actions_only() {
        use egui::accesskit::Role;
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut app = app();
        apply_flags(&mut app, Some("react-menu"));
        app.attach(&ctx);
        render(&mut app, &ctx);
        let nodes = accessible_nodes(&mut app, &ctx, Vec::new());
        let find = |prefix: &str| {
            nodes
                .iter()
                .find(|(label, _, _)| label.starts_with(prefix))
                .unwrap_or_else(|| panic!("no {prefix} row"))
                .clone()
        };
        for status in ["Sent ", "Delivered ", "Read "] {
            assert!(
                nodes.iter().all(|(label, _, _)| !label.starts_with(status)),
                "the menu has no {status}row"
            );
        }
        assert_eq!(find("Message info").1, Role::Button);
        let (_, role, copy) = find("Copy message ID");
        assert_eq!(role, Role::Button);

        let click = |app: &mut App, pos: egui::Pos2| {
            let press = |pressed| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            accessible_nodes(app, &ctx, vec![egui::Event::PointerMoved(pos), press(true)]);
            accessible_nodes(app, &ctx, vec![press(false)]);
        };
        click(&mut app, copy);
        assert!(app.toasts.iter().any(|toast| toast.message == "Copied"));
    }

    #[test]
    fn history_fidelity_message_menu_requests_before_the_chosen_message() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut app = app();
        apply_flags(&mut app, Some("react-menu"));
        app.attach(&ctx);
        app.backend.record_demo_commands();
        render(&mut app, &ctx);
        let nodes = accessible_nodes(&mut app, &ctx, Vec::new());
        let (_, _, pos) = nodes
            .into_iter()
            .find(|(label, _, _)| label == "Reload earlier messages")
            .expect("reload action");
        let press = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        accessible_nodes(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(pos), press(true)],
        );
        accessible_nodes(&mut app, &ctx, vec![press(false)]);
        let commands = app.backend.take_demo_commands();
        assert!(commands.iter().any(|command| matches!(command,
            crate::backend::Command::ReloadHistory { chat, message }
            if Some(chat) == app.open_chat.as_ref() && message == "ada-link")));
        assert!(app.conversations[app.open_chat.as_ref().unwrap()].fetching_phone);
    }

    /// The label of the node with keyboard focus, as a screen reader gets it.
    fn focused_label(app: &mut App, ctx: &egui::Context) -> Option<String> {
        let tree = tree_after(app, ctx, None);
        node(&tree, tree.focus).and_then(|node| node.label().map(str::to_owned))
    }

    /// Down through an open menu until the item named `label` has focus.
    fn walk_menu_to(app: &mut App, ctx: &egui::Context, label: &str) {
        for _ in 0..30 {
            if focused_label(app, ctx).as_deref() == Some(label) {
                return;
            }
            press_key(app, ctx, egui::Key::ArrowDown, egui::Modifiers::NONE);
        }
        panic!("no menu item {label}");
    }

    /// Chooses the open menu's item named `label` from the keys: down to it,
    /// then Enter.
    fn choose_menu_item(app: &mut App, ctx: &egui::Context, label: &str) {
        walk_menu_to(app, ctx, label);
        press_key(app, ctx, egui::Key::Enter, egui::Modifiers::NONE);
        render(app, ctx);
    }

    /// "Go to replied message" goes where clicking the quote goes, and
    /// the list keeps focus on the replied message.
    #[test]
    fn the_menus_go_to_replied_message_does_what_the_quote_click_does() {
        let (mut on, target) = on_a_reply();
        on.ctx.enable_accesskit();
        open_menu_by_keys(&mut on.app, &on.ctx);
        choose_menu_item(&mut on.app, &on.ctx, "Go to replied message");
        assert_eq!(on.app.message_cursor.get(&on.chat), Some(&target));
        let label = current_message_label(&mut on).expect("the list has focus again");
        let body = on.app.conversations[&on.chat].messages[0].summary();
        assert!(label.contains(body.trim()), "{label} / {body}");
        let (mut clicked, target) = on_a_reply();
        clicked.click(Part::Named("quote"));
        assert_eq!(clicked.app.message_cursor.get(&clicked.chat), Some(&target));
    }

    /// A downloaded video's keyboard menu fills the window with it through
    /// the menu's own "Open in the viewer", where its "Fill the window"
    /// button takes it too.
    #[test]
    fn the_menus_open_in_the_viewer_does_what_the_videos_button_does() {
        let mut video = video_to_fetch();
        if let Some(media) = video.media_at_mut(None) {
            media.path = Some(sample_video());
        }
        let mut on = app_on_attachment(video.clone());
        on.ctx.enable_accesskit();
        on.app.video.silence();
        open_menu_by_keys(&mut on.app, &on.ctx);
        choose_menu_item(&mut on.app, &on.ctx, "Open in the viewer");
        let shown = |app: &App| {
            app.image_preview
                .as_ref()
                .map(|preview| preview.message().to_owned())
        };
        assert_eq!(shown(&on.app).as_deref(), Some("key-fixture"));
        // The video's own button, under a paused video's controls.
        let mut clicked = app_on_attachment(video);
        clicked.app.video.silence();
        clicked.app.actions.push(crate::model::Action::PlayVideo {
            message: "key-fixture".into(),
            path: sample_video(),
        });
        let started = std::time::Instant::now();
        while clicked
            .app
            .video
            .status("key-fixture")
            .is_none_or(|status| status.state != crate::video::State::Playing)
        {
            assert!(started.elapsed() < std::time::Duration::from_secs(10));
            render(&mut clicked.app, &clicked.ctx);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        clicked.app.actions.push(crate::model::Action::PlayVideo {
            message: "key-fixture".into(),
            path: sample_video(),
        });
        render(&mut clicked.app, &clicked.ctx);
        let window = clicked
            .ctx
            .data(|data| {
                data.get_temp::<egui::Rect>(egui::Id::new(("video-window", "key-fixture")))
            })
            .expect("the paused video shows its controls");
        let at = window.center();
        let press = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame_with(
            &mut clicked.app,
            &clicked.ctx,
            vec![egui::Event::PointerMoved(at), press(true), press(false)],
        );
        render(&mut clicked.app, &clicked.ctx);
        assert_eq!(shown(&clicked.app), shown(&on.app));
    }

    /// In a group, "Info about {name}" opens the sender's info panel, as clicking
    /// their name or picture does; one's own messages have none.
    #[test]
    fn the_menus_sender_info_does_what_the_name_click_does() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut app = app();
        app.attach(&ctx);
        // The groups in a fixed order (the map's own order changes from run
        // to run), and the first plain text message from someone else with
        // their name drawn well inside the window. A text message's click
        // target spans its bubble for selecting, so its name is where a
        // click could be lost.
        let mut groups: Vec<_> = app
            .conversations
            .keys()
            .filter(|chat| chat.ends_with("@g.us"))
            .cloned()
            .collect();
        groups.sort();
        let inside =
            egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1180.0, 780.0)).shrink(120.0);
        let (chat, message, name_rect) = groups
            .into_iter()
            .find_map(|chat| {
                app.open_chat = Some(chat.clone());
                // A chat scrolls into place over its first frames.
                for _ in 0..6 {
                    render(&mut app, &ctx);
                }
                app.conversations[&chat]
                    .messages
                    .iter()
                    .filter(|message| {
                        !message.from_me && matches!(message.content, Content::Text { .. })
                    })
                    .find_map(|message| {
                        ctx.data(|data| {
                            data.get_temp::<egui::Rect>(
                                crate::ui::conversation::bubble_id(&chat, &message.id)
                                    .with("sender"),
                            )
                        })
                        .filter(|rect| inside.contains_rect(*rect))
                        .map(|rect| (chat.clone(), message.clone(), rect))
                    })
            })
            .expect("a group text message from someone else, its sender's name on screen");
        let at = name_rect.center();
        let press = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame_with(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(at), press(true), press(false)],
        );
        render(&mut app, &ctx);
        let by_click = app.info.take().map(|info| info.chat);
        assert_eq!(
            by_click.as_ref(),
            Some(&message.sender),
            "the sender's info"
        );
        render(&mut app, &ctx);
        app.message_cursor.insert(chat.clone(), message.id.clone());
        focus_messages(&mut app, &ctx);
        press_key(&mut app, &ctx, egui::Key::F10, egui::Modifiers::SHIFT);
        let name = app.display_name_or(&message.sender, message.sender_name.as_deref());
        choose_menu_item(
            &mut app,
            &ctx,
            &format!("Info about {}", name.trim_start_matches('~')),
        );
        assert_eq!(
            app.info.map(|info| info.chat),
            by_click,
            "the menu does what the click does"
        );
    }

    /// From the keys' menu, a replied message that is not loaded is waited
    /// for, as G waits for it.
    #[test]
    fn the_menus_go_to_replied_message_waits_for_one_not_loaded() {
        let (mut on, target) = on_a_reply_not_loaded();
        on.ctx.enable_accesskit();
        open_menu_by_keys(&mut on.app, &on.ctx);
        choose_menu_item(&mut on.app, &on.ctx, "Go to replied message");
        assert_eq!(current_message(&on), "key-fixture");
        assert!(
            spoke(&on.app, "Loading the replied message"),
            "{:?}",
            on.app.announcer.spoken()
        );
        // Focus back in the list from the menu is not a move on.
        deliver(&mut on, vec![target]);
        assert_eq!(current_message(&on), "not-loaded");
    }

    /// Every button of an interactive card is an entry of the menu the keys
    /// open: usable ones act as a click does, unavailable ones do nothing.
    /// The menu opened otherwise, as with the pointer, is as it was.
    #[test]
    fn the_message_menu_lists_interactive_buttons() {
        let count = |nodes: &[(String, egui::accesskit::Role, egui::Pos2)], label: &str| {
            nodes.iter().filter(|(name, _, _)| name == label).count()
        };
        let labels = ["Tell me more", "Choose a session", "Copy invitation code"];
        let unavailable = "Open registration form, unavailable: Open this option in WhatsApp Web or on your phone";
        let start = || {
            let ctx = egui::Context::default();
            ctx.enable_accesskit();
            let mut app = app();
            apply_flags(&mut app, Some("interactive-actions"));
            app.backend.record_demo_commands();
            app.attach(&ctx);
            render(&mut app, &ctx);
            (app, ctx)
        };
        let (mut app, ctx) = start();
        let before = accessible_nodes(&mut app, &ctx, Vec::new());
        app.open_message_menu = message_menu(&app, "interactive-card");
        render(&mut app, &ctx);
        let after = accessible_nodes(&mut app, &ctx, Vec::new());
        for label in labels.into_iter().chain([unavailable]) {
            assert_eq!(
                count(&after, label),
                count(&before, label),
                "{label} is no entry of a menu the keys did not open"
            );
        }
        let (mut app, ctx) = start();
        let before = accessible_nodes(&mut app, &ctx, Vec::new());
        ctx.data_mut(|data| data.insert_temp(crate::theme::keyboard_focus_id(), true));
        app.open_message_menu = message_menu(&app, "interactive-card");
        render(&mut app, &ctx);
        let after = accessible_nodes(&mut app, &ctx, Vec::new());
        // The card's own rows carry the same names, so each name appears once
        // more with the menu open.
        for label in labels {
            assert_eq!(
                count(&after, label),
                count(&before, label) + 1,
                "{label} is a menu entry"
            );
        }
        // The unavailable button's entry says why in its name, and is
        // disabled, as the card's own row is.
        assert_eq!(count(&after, unavailable), 1, "the entry names its reason");
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1180.0, 780.0),
                )),
                ..Default::default()
            },
            |ui| {
                let ctx = ui.ctx().clone();
                app.background_frame(&ctx);
                app.frame_ui(ui);
            },
        );
        output.textures_delta.clear();
        let tree = output.platform_output.accesskit_update.unwrap();
        let states: Vec<(bool, bool)> = tree
            .nodes
            .iter()
            .filter(|(_, node)| {
                node.role() == egui::accesskit::Role::Button
                    && (node.label() == Some(unavailable)
                        || node.label() == Some("Open registration form"))
            })
            .map(|(_, node)| (node.is_disabled(), node.label() == Some(unavailable)))
            .collect();
        assert_eq!(states.len(), 2, "{states:?}");
        assert!(states.iter().all(|(disabled, _)| *disabled), "{states:?}");
        // The disabled row takes the click: nothing under the menu, which
        // the keys open over the message, acts.
        let commands = click_entry(&mut app, &ctx, unavailable);
        assert!(
            !commands.iter().any(|command| matches!(
                command,
                egui::OutputCommand::OpenUrl(_) | egui::OutputCommand::CopyText(_)
            )),
            "a click on a disabled row reached the card below: {commands:?}"
        );
        assert!(app.dialog.is_none());
        // Chosen from the keys, an entry does what its button's click does.
        let (mut app, ctx) = start();
        let choose = |app: &mut App, label: &str| {
            app.message_cursor
                .insert(SAMPLES[0].id.into(), "interactive-card".into());
            focus_messages(app, &ctx);
            press_key(app, &ctx, egui::Key::F10, egui::Modifiers::SHIFT);
            choose_menu_item(app, &ctx, label);
        };
        choose(&mut app, "Copy invitation code");
        assert!(
            app.toasts.iter().any(|toast| toast.message == "Copied"),
            "the code is copied"
        );
        choose(&mut app, "Choose a session");
        assert!(matches!(
            app.dialog,
            Some(Dialog::InteractiveList { button: 1, .. })
        ));
    }

    /// Clicks the centre of the menu entry named `label` and returns what the
    /// frames asked the platform to do.
    fn click_entry(app: &mut App, ctx: &egui::Context, label: &str) -> Vec<egui::OutputCommand> {
        let nodes = accessible_nodes(app, ctx, Vec::new());
        // A card's own button rows carry the same names: the entry is the one
        // that is not at a row's place, in the menu's layer above the chat.
        let rows: Vec<egui::Pos2> = (0..8usize)
            .filter_map(|index| {
                ctx.data(|data| {
                    data.get_temp::<egui::Rect>(
                        crate::ui::conversation::bubble_id(SAMPLES[0].id, "interactive-card")
                            .with(("interactive-button", index)),
                    )
                })
            })
            .map(|rect| rect.center())
            .collect();
        let (_, _, pos) = nodes
            .iter()
            .find(|(name, _, pos)| {
                name == label
                    && !rows.iter().any(|row| row.distance(*pos) < 1.0)
                    && ctx
                        .layer_id_at(*pos)
                        .is_some_and(|layer| layer.order == egui::Order::Foreground)
            })
            .unwrap_or_else(|| panic!("no menu entry named {label}"))
            .clone();
        let press = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let mut commands = Vec::new();
        for events in [
            vec![egui::Event::PointerMoved(pos), press(true)],
            vec![press(false)],
            // The app applies the click's actions at the start of a frame.
            Vec::new(),
        ] {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1180.0, 780.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    let ctx = ui.ctx().clone();
                    app.background_frame(&ctx);
                    app.frame_ui(ui);
                },
            );
            output.textures_delta.clear();
            commands.extend(output.platform_output.commands);
        }
        commands
    }

    /// Opening the log hands it to the worker, which waits to see it open or
    /// shows it in its folder, instead of the fire-and-forget attachment path
    /// that opened nothing on Linux desktops without a handler for it.
    #[test]
    fn opening_the_log_goes_through_the_checked_opener() {
        let ctx = egui::Context::default();
        let mut app = app();
        app.attach(&ctx);
        app.backend.record_demo_commands();
        let log = app.dirs.log_file();
        app.actions.push(crate::model::Action::OpenLog(log.clone()));
        render(&mut app, &ctx);
        assert!(
            app.backend
                .take_demo_commands()
                .iter()
                .any(|command| matches!(
                    command,
                    crate::backend::Command::OpenLog(path) if path == &log
                ))
        );
    }

    /// A downloaded image's menu copies the picture itself, as the preview
    /// does; one that is not downloaded yet offers no copy.
    #[test]
    fn the_menu_of_a_downloaded_image_copies_it() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let chat = sample_ids()[0].to_owned();
        for downloaded in [true, false] {
            let mut app = app();
            // A real file: a path whose file is gone downloads again instead.
            let (path, _) = sample_files(&app);
            app.attach(&ctx);
            app.backend.record_demo_commands();
            let mut photo = media("image/jpeg", 120_000, Some(800), Some(600));
            photo.path = downloaded.then(|| path.clone());
            app.conversations.get_mut(&chat).unwrap().messages = vec![message(
                &chat,
                "copy-photo",
                false,
                100,
                Content::Image {
                    motion: None,
                    caption: None,
                    media: photo,
                },
            )];
            app.open_message_menu = Some((chat.clone(), "copy-photo".into()));
            render(&mut app, &ctx);
            let nodes = accessible_nodes(&mut app, &ctx, Vec::new());
            let copy = nodes
                .iter()
                .find(|(label, _, _)| label == "Copy image")
                .map(|(_, _, pos)| *pos);
            assert_eq!(
                copy.is_some(),
                downloaded,
                "Copy image only when downloaded"
            );
            let Some(pos) = copy else { continue };
            let press = |pressed| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            accessible_nodes(
                &mut app,
                &ctx,
                vec![egui::Event::PointerMoved(pos), press(true)],
            );
            accessible_nodes(&mut app, &ctx, vec![press(false)]);
            assert!(
                app.backend
                    .take_demo_commands()
                    .iter()
                    .any(|command| matches!(
                        command,
                        crate::backend::Command::PrepareClipboardImage(copied) if copied == &path
                    )),
                "the image goes to the clipboard"
            );
        }
    }

    #[test]
    fn enter_submits_the_locked_chat_code_and_keeps_wrong_codes_locked() {
        for code in ["wrong-code", "demo-code"] {
            let mut app = app();
            apply_flags(&mut app, Some("locked-prompt"));
            let ctx = egui::Context::default();
            app.attach(&ctx);
            render(&mut app, &ctx);
            frame_with(&mut app, &ctx, vec![egui::Event::Text(code.into())]);
            assert_eq!(app.chat_lock_entry, code);
            frame_with(
                &mut app,
                &ctx,
                vec![key(egui::Key::Enter, egui::Modifiers::NONE)],
            );
            assert_eq!(app.locked_folder, code == "demo-code");
            assert_eq!(app.dialog.is_none(), code == "demo-code");
        }
    }

    #[test]
    fn enter_creates_a_lock_code_only_when_confirmation_matches() {
        for confirmation in ["different", "fixture-code"] {
            let mut app = app();
            apply_flags(&mut app, Some("locked-setup"));
            let ctx = egui::Context::default();
            app.attach(&ctx);
            render(&mut app, &ctx);
            frame_with(
                &mut app,
                &ctx,
                vec![egui::Event::Text("fixture-code".into())],
            );
            frame_with(
                &mut app,
                &ctx,
                vec![key(egui::Key::Tab, egui::Modifiers::NONE)],
            );
            frame_with(&mut app, &ctx, vec![egui::Event::Text(confirmation.into())]);
            assert_eq!(app.chat_lock_confirm, confirmation);
            frame_with(
                &mut app,
                &ctx,
                vec![key(egui::Key::Enter, egui::Modifiers::NONE)],
            );
            assert_eq!(app.locked_folder, confirmation == "fixture-code");
            assert_eq!(
                app.settings.verifies_chat_lock_code("fixture-code"),
                confirmation == "fixture-code"
            );
        }
    }

    #[test]
    fn an_open_context_menu_outlines_its_message_without_a_reaction_picker() {
        let mut app = app();
        apply_flags(&mut app, Some("react-menu"));
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        let shapes = frame_sized(&mut app, &ctx, 780.0, Vec::new());
        let id = crate::ui::conversation::bubble_id(sample_ids()[0], "ada-link");
        let target = ctx
            .data(|data| data.get_temp::<egui::Rect>(id.with("rect")))
            .unwrap()
            .expand(2.0);
        let accent = app.palette.accent;
        let outline_count = |shapes: &[egui::epaint::ClippedShape]| {
            shapes
                .iter()
                .filter(|shape| {
                    matches!(&shape.shape, egui::Shape::Rect(rect)
                if rect.rect == target && rect.stroke.color == accent
                    && rect.stroke.width == crate::theme::FOCUS_STROKE_WIDTH
                    && rect.stroke_kind == egui::StrokeKind::Outside)
                })
                .count()
        };
        assert!(app.reaction_target.is_none());
        assert_eq!(outline_count(&shapes), 1);
        app.open_message_menu = None;
        egui::Popup::close_id(&ctx, id.with("popup"));
        let shapes = frame_sized(&mut app, &ctx, 780.0, Vec::new());
        assert_eq!(outline_count(&shapes), 0);
    }

    #[test]
    fn a_demo_flag_keeps_the_reaction_menu_open() {
        let mut app = app();
        apply_flags(&mut app, Some("react-menu"));
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        let chat = sample_ids()[0].to_owned();
        let popup = crate::ui::conversation::bubble_id(&chat, "ada-link").with("popup");
        assert!(
            egui::Popup::is_id_open(&ctx, popup),
            "react-menu opens the message context menu"
        );
    }

    #[test]
    fn a_reaction_picker_choice_uses_the_same_react_path() {
        let mut app = app();
        app.backend.record_demo_commands();
        apply_flags(&mut app, Some("react-picker"));
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        assert!(app.reaction_target.is_some());

        let chat = sample_ids()[0].to_owned();
        let current = app
            .conversations
            .get(&chat)
            .and_then(|conversation| conversation.message("ada-link"))
            .and_then(crate::ui::conversation::own_reaction);
        let emoji = crate::ui::conversation::reaction_choice(current, "🦀");
        assert_eq!(emoji, "🦀");
        app.actions.push(crate::model::Action::React {
            chat,
            message: "ada-link".into(),
            emoji,
        });
        render(&mut app, &ctx);
        assert!(app.reaction_target.is_none());
        let commands = app.backend.take_demo_commands();
        assert!(commands.iter().any(|command| matches!(
            command,
            crate::backend::Command::React { emoji, .. } if emoji == "🦀"
        )));
    }

    /// Clicks the published hover control and checks it opens the picker for
    /// exactly that message without leaking into reply or the context menu.
    #[test]
    fn clicking_the_hover_reaction_control_opens_the_picker_for_that_message() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        for _ in 0..3 {
            render(&mut app, &ctx);
        }
        let chat = sample_ids()[0].to_owned();
        let message = "ada-link";
        assert!(app.reaction_target.is_none());

        let id = crate::ui::conversation::bubble_id(&chat, message);
        let affordance = ctx
            .data(|data| data.get_temp::<egui::Rect>(id.with("react-rect")))
            .expect("the hover control publishes its rect");
        let bubble = ctx
            .data(|data| data.get_temp::<egui::Rect>(id.with("rect")))
            .expect("the bubble publishes its rect");

        // The control sits beside the bubble, never over its text or link, so
        // it cannot swallow link clicks or text selection.
        assert!(
            !affordance.intersects(bubble),
            "the control {affordance:?} overlaps the bubble {bubble:?}"
        );
        let body = ctx.data(|data| data.get_temp::<egui::Rect>(id.with("body")));
        assert!(
            body.is_none_or(|body| !affordance.intersects(body)),
            "the control {affordance:?} covers the message body"
        );

        // Hover the message, then click the published control.
        let pos = affordance.center();
        let press = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame_with(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(bubble.center())],
        );
        frame_with(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(pos), press(true)],
        );
        frame_with(&mut app, &ctx, vec![press(false)]);
        assert_eq!(
            app.reaction_target,
            Some((chat.clone(), message.to_owned())),
            "the hover control opens the picker for this exact message"
        );
        assert!(
            app.open_message_menu.is_none(),
            "the click opens the picker, not the context menu"
        );
        for _ in 0..3 {
            render(&mut app, &ctx);
        }
        assert!(
            !egui::Popup::is_id_open(&ctx, id.with("popup")),
            "the context menu stays closed beside a picker opened from hover"
        );
        assert!(app.reaction_target.is_some(), "the picker stays open");
        assert!(app.reply_to.is_none(), "the click does not start a reply");
    }

    /// The hover control is beside the bubble, so double-click reply and the
    /// right-click context menu keep working while it is registered.
    #[test]
    fn the_hover_reaction_control_leaves_reply_and_the_menu_alone() {
        let mut app = app();
        let chat = sample_ids()[0].to_owned();
        app.conversations.get_mut(&chat).unwrap().messages = vec![message(
            &chat,
            "text",
            false,
            100,
            Content::text("Double-click me"),
        )];
        let ctx = egui::Context::default();
        app.attach(&ctx);
        for _ in 0..3 {
            render(&mut app, &ctx);
        }

        let id = crate::ui::conversation::bubble_id(&chat, "text");
        let affordance = ctx
            .data(|data| data.get_temp::<egui::Rect>(id.with("react-rect")))
            .expect("the hover control publishes its rect");
        let bubble = ctx
            .data(|data| data.get_temp::<egui::Rect>(id.with("rect")))
            .expect("the bubble publishes its rect");
        assert!(
            !affordance.intersects(bubble),
            "the control {affordance:?} overlaps the bubble {bubble:?}"
        );

        // A double-click on the bubble padding still replies, not reacts.
        let pad = bubble.left_center() + egui::vec2(4.0, 0.0);
        let press = |pressed| egui::Event::PointerButton {
            pos: pad,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        for events in [
            vec![egui::Event::PointerMoved(pad), press(true)],
            vec![press(false)],
            vec![press(true)],
            vec![press(false)],
            vec![],
        ] {
            frame_with(&mut app, &ctx, events);
        }
        assert_eq!(
            app.reply_to.as_deref(),
            Some("text"),
            "double-click reply keeps working beside the control"
        );
        assert!(app.reaction_target.is_none(), "a double-click never reacts");

        // A right-click on the bubble still opens the context menu, not the
        // picker. The reply bar made the view shorter, and the chat, which
        // sits at the bottom of the view, moved up with it.
        let bubble = ctx
            .data(|data| data.get_temp::<egui::Rect>(id.with("rect")))
            .expect("the bubble publishes its rect");
        let press = |pressed| egui::Event::PointerButton {
            pos: bubble.center(),
            button: egui::PointerButton::Secondary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame_with(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(bubble.center()), press(true)],
        );
        frame_with(&mut app, &ctx, vec![press(false)]);
        assert!(
            egui::Popup::is_id_open(&ctx, id.with("popup")),
            "a right-click on the bubble still opens the context menu"
        );
        assert!(
            app.reaction_target.is_none(),
            "the context menu does not open the reaction picker"
        );
    }

    /// A message scrolled up under the chat header is hidden there, so a
    /// right-click on the header does not open that message's menu.
    #[test]
    fn right_click_on_the_header_does_not_reach_a_message_under_it() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        let viewport = app
            .selection_view
            .lock()
            .unwrap()
            .expect("the transcript was drawn");
        let chat = app.open_chat.clone().expect("a chat is open");
        let (id, bubble) = app.conversations[&chat]
            .messages
            .iter()
            .map(|message| crate::ui::conversation::bubble_id(&chat, &message.id))
            .find_map(|id| {
                let rect = ctx.data(|data| data.get_temp::<egui::Rect>(id.with("rect")))?;
                (rect.top() < viewport.top() - 8.0 && rect.bottom() > viewport.top() + 8.0)
                    .then_some((id, rect))
            })
            .expect("a message runs under the header");
        let right_click = |app: &mut App, pos: egui::Pos2| {
            let press = |pressed| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Secondary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            frame_with(app, &ctx, vec![egui::Event::PointerMoved(pos), press(true)]);
            frame_with(app, &ctx, vec![press(false)]);
            frame_with(app, &ctx, Vec::new());
        };
        let hidden = egui::pos2(bubble.center().x, viewport.top() - 4.0);
        assert!(bubble.contains(hidden));
        right_click(&mut app, hidden);
        assert!(
            !egui::Popup::is_id_open(&ctx, id.with("popup")),
            "the header's right-click opened the hidden message's menu"
        );

        // The visible part of the same message still opens it.
        right_click(
            &mut app,
            egui::pos2(bubble.center().x, viewport.top() + 4.0),
        );
        assert!(
            egui::Popup::is_id_open(&ctx, id.with("popup")),
            "a right-click on the visible part opens the menu"
        );
    }

    /// A location in our own bubble keeps one width from frame to frame
    /// instead of flickering, and stays a card rather than spanning the chat.
    #[test]
    fn our_location_cards_hold_still() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        apply_flags(&mut app, Some("live"));
        let id = crate::ui::conversation::bubble_id(sample_ids()[0], "ada-location-now");
        let rect =
            |ctx: &egui::Context| ctx.data(|data| data.get_temp::<egui::Rect>(id.with("rect")));
        for _ in 0..3 {
            render(&mut app, &ctx);
        }
        let first = rect(&ctx).expect("the location bubble was drawn");
        for _ in 0..4 {
            render(&mut app, &ctx);
            assert_eq!(rect(&ctx), Some(first), "the bubble moved between frames");
        }
        assert!(first.width() < 360.0, "the card spans {}", first.width());
    }

    #[test]
    fn switching_chats_closes_the_reaction_picker() {
        let mut app = app();
        apply_flags(&mut app, Some("react-picker"));
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        assert!(app.reaction_target.is_some());
        assert_eq!(app.open_chat.as_deref(), Some(sample_ids()[0]));

        app.actions
            .push(crate::model::Action::OpenChat(sample_ids()[1].into()));
        render(&mut app, &ctx);

        assert_eq!(app.open_chat.as_deref(), Some(sample_ids()[1]));
        assert!(
            app.reaction_target.is_none(),
            "switching chats must drop the previous reaction target"
        );
        assert!(app.reaction_anchor.is_none());
    }

    #[test]
    fn reaction_picker_keeps_its_menu_and_target_visible_and_freezes_the_chat() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        apply_flags(&mut app, Some("react-picker"));
        for _ in 0..3 {
            render(&mut app, &ctx);
        }
        let id = crate::ui::conversation::bubble_id(sample_ids()[0], "ada-link");
        let menu = ctx
            .data(|data| data.get_temp::<egui::Rect>(id.with("menu-rect")))
            .unwrap();
        let picker = ctx
            .data(|data| data.get_temp::<egui::Rect>(egui::Id::new("reaction-picker-rect")))
            .unwrap();
        assert!(egui::Popup::is_id_open(&ctx, id.with("popup")));
        assert!(
            !menu.intersects(picker),
            "menu {menu:?} overlaps picker {picker:?}"
        );
        let before = ctx
            .data(|data| data.get_temp::<egui::Rect>(id.with("rect")))
            .unwrap();
        assert!(before.intersects(ctx.content_rect()), "target is visible");
        frame_with(
            &mut app,
            &ctx,
            vec![
                egui::Event::PointerMoved(egui::pos2(1150.0, 400.0)),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    phase: egui::TouchPhase::Move,
                    delta: egui::vec2(0.0, 180.0),
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        render(&mut app, &ctx);
        let after = ctx
            .data(|data| data.get_temp::<egui::Rect>(id.with("rect")))
            .unwrap();
        assert_eq!(before, after, "wheel does not move the target conversation");
        frame_with(
            &mut app,
            &ctx,
            vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        render(&mut app, &ctx);
        assert!(app.reaction_target.is_none());
        assert!(!egui::Popup::is_id_open(&ctx, id.with("popup")));
    }

    #[test]
    fn opening_settings_does_not_write_account_privacy() {
        let mut app = app();
        app.backend.record_demo_commands();
        apply_flags(&mut app, Some("settings"));
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        let commands = app.backend.take_demo_commands();
        assert!(
            !commands.iter().any(|command| matches!(
                command,
                crate::backend::Command::SetAccountPrivacy { .. }
            )),
            "opening Settings must not write privacy"
        );
    }

    #[test]
    fn set_account_privacy_enqueues_the_phone_write() {
        let mut app = app();
        app.backend.record_demo_commands();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        app.actions.push(crate::model::Action::SetAccountPrivacy {
            kind: crate::privacy::PrivacyKind::Profile,
            choice: crate::privacy::PrivacyChoice::Nobody,
        });
        render(&mut app, &ctx);
        let commands = app.backend.take_demo_commands();
        assert!(
            commands.iter().any(|command| matches!(
                command,
                crate::backend::Command::SetAccountPrivacy {
                    kind: crate::privacy::PrivacyKind::Profile,
                    choice: crate::privacy::PrivacyChoice::Nobody,
                }
            )),
            "picking a value writes it to the phone"
        );
        // A second pick waits for the first, and an Except list is never
        // written from here.
        for choice in [
            crate::privacy::PrivacyChoice::Everyone,
            crate::privacy::PrivacyChoice::Except,
        ] {
            app.actions.push(crate::model::Action::SetAccountPrivacy {
                kind: crate::privacy::PrivacyKind::Profile,
                choice,
            });
        }
        app.actions.push(crate::model::Action::SetAccountPrivacy {
            kind: crate::privacy::PrivacyKind::About,
            choice: crate::privacy::PrivacyChoice::Except,
        });
        render(&mut app, &ctx);
        assert!(
            !app.backend
                .take_demo_commands()
                .iter()
                .any(|command| matches!(
                    command,
                    crate::backend::Command::SetAccountPrivacy { .. }
                )),
            "nothing else is written"
        );
    }

    #[test]
    fn opening_settings_reads_account_privacy_again() {
        let mut app = app();
        app.backend.record_demo_commands();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        app.actions
            .push(crate::model::Action::Open(crate::model::Page::Settings));
        render(&mut app, &ctx);
        assert!(
            app.backend
                .take_demo_commands()
                .iter()
                .any(|command| matches!(command, crate::backend::Command::FetchAccountPrivacy))
        );
    }

    /// The gear opens settings, and a second click on it closes them again,
    /// landing back on the chat that was open. No close button is added.
    #[test]
    fn a_second_click_on_the_settings_button_closes_settings() {
        use crate::ui::focus::Stop;
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        let chat = app.open_chat.clone().expect("a chat is open to start");
        // The macOS header carries neither the avatar nor the gear: Settings
        // lives in the menu bar there, and a headless test draws no menu bar.
        // The toggle is the same action everywhere, so macOS drives it through
        // the action and the clicks are exercised where the buttons exist.
        if crate::theme::macos_chrome(&ctx) {
            for expected in [crate::model::Page::Settings, crate::model::Page::Chats] {
                app.actions.push(crate::model::Action::ToggleSettings);
                frame_sized(&mut app, &ctx, 780.0, Vec::new());
                assert_eq!(app.page, expected, "the action toggles the page");
            }
            assert_eq!(
                app.open_chat.as_deref(),
                Some(chat.as_str()),
                "closing settings lands back on the chat that was open"
            );
            return;
        }
        let gear = |ctx: &egui::Context| {
            let id = crate::ui::focus::stops(ctx)
                .into_iter()
                .find(|(stop, _)| *stop == Stop::Settings)
                .map(|(_, id)| id)
                .expect("the settings button is drawn");
            ctx.read_response(id).expect("it publishes its rect").rect
        };
        let click = |app: &mut App, ctx: &egui::Context, rect: egui::Rect| {
            let pos = rect.center();
            for pressed in [true, false] {
                frame_sized(
                    app,
                    ctx,
                    780.0,
                    vec![
                        egui::Event::PointerMoved(pos),
                        egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                );
            }
        };
        click(&mut app, &ctx, gear(&ctx));
        assert_eq!(
            app.page,
            crate::model::Page::Settings,
            "the first click opens settings"
        );
        click(&mut app, &ctx, gear(&ctx));
        assert_eq!(
            app.page,
            crate::model::Page::Chats,
            "the second click closes settings"
        );
        assert_eq!(
            app.open_chat.as_deref(),
            Some(chat.as_str()),
            "closing settings lands back on the chat that was open"
        );
    }

    /// While Settings are showing, the gear says what a click does now: a
    /// screen reader reads the label, not the accent colour. The avatar keeps
    /// switching accounts.
    #[test]
    fn the_settings_button_says_it_closes_settings() {
        let mut app = app();
        let ctx = egui::Context::default();
        // The macOS header has no settings button: the app menu holds them.
        if crate::theme::macos_chrome(&ctx) {
            return;
        }
        ctx.enable_accesskit();
        app.attach(&ctx);
        let labels = |app: &mut App, ctx: &egui::Context| -> Vec<String> {
            let mut labels = Vec::new();
            for _ in 0..3 {
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(1180.0, 780.0),
                        )),
                        ..Default::default()
                    },
                    |ui| {
                        let ctx = ui.ctx().clone();
                        app.background_frame(&ctx);
                        app.frame_ui(ui);
                    },
                );
                output.textures_delta.clear();
                labels = output
                    .platform_output
                    .accesskit_update
                    .expect("accessibility tree")
                    .nodes
                    .iter()
                    .filter_map(|(_, node)| node.label().map(str::to_owned))
                    .collect();
            }
            labels
        };
        let closed = labels(&mut app, &ctx);
        assert!(
            closed.contains(&"Switch account".to_owned()),
            "the avatar opens the account switcher: {closed:?}"
        );
        assert!(
            closed.contains(&"Settings (Ctrl+,)".to_owned()),
            "the gear opens settings: {closed:?}"
        );
        app.actions
            .push(crate::model::Action::Open(crate::model::Page::Settings));
        let open = labels(&mut app, &ctx);
        assert!(
            open.contains(&"Close settings (Ctrl+,)".to_owned()),
            "the gear says it closes settings: {open:?}"
        );
    }

    /// Our avatar opens the accounts and "Add account", nothing else: the
    /// settings have their own button beside it. With one number that is how
    /// a second one is added.
    #[test]
    fn the_switcher_lists_only_accounts_and_adding_one() {
        let mut app = app();
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        app.attach(&ctx);
        app.account_menu = true;
        let mut labels = Vec::new();
        for _ in 0..3 {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1180.0, 780.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    let ctx = ui.ctx().clone();
                    app.background_frame(&ctx);
                    app.frame_ui(ui);
                },
            );
            output.textures_delta.clear();
            labels = output
                .platform_output
                .accesskit_update
                .expect("accessibility tree")
                .nodes
                .iter()
                .filter_map(|(_, node)| node.label().map(str::to_owned))
                .collect();
        }
        assert!(labels.contains(&"Add account".to_owned()), "{labels:?}");
        let name = app.account().display_label(app.locale);
        assert!(
            labels.iter().any(|label| label.starts_with(&name)),
            "the one account is listed: {labels:?}"
        );
        for gone in ["Profile and settings", "Close settings"] {
            assert!(!labels.contains(&gone.to_owned()), "{gone}: {labels:?}");
        }
    }

    /// Each account in the switcher has a bell, and a muted one says so to a
    /// screen reader on its row, not only by its crossed-out bell.
    #[test]
    fn the_switcher_says_which_accounts_are_muted() {
        let mut app = app();
        apply_flags(&mut app, Some("accounts-muted"));
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        app.attach(&ctx);
        let mut nodes = Vec::new();
        for _ in 0..3 {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1180.0, 780.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    let ctx = ui.ctx().clone();
                    app.background_frame(&ctx);
                    app.frame_ui(ui);
                },
            );
            output.textures_delta.clear();
            nodes = output
                .platform_output
                .accesskit_update
                .expect("accessibility tree")
                .nodes
                .into_iter()
                .map(|(_, node)| node)
                .collect();
        }
        let bells: Vec<_> = nodes
            .iter()
            .filter(|node| node.label() == Some("Mute notifications"))
            .map(|node| node.toggled() == Some(egui::accesskit::Toggled::True))
            .collect();
        assert_eq!(bells, [false, true], "one bell per account, the second on");
        // Only the muted account's row says so, not the one on screen.
        let said: Vec<_> = nodes
            .iter()
            .filter_map(|node| node.label())
            .filter(|label| label.ends_with(", notifications muted"))
            .collect();
        let muted = app.accounts[1].display_label(app.locale);
        assert_eq!(said.len(), 1, "{said:?}");
        assert!(said[0].starts_with(&muted), "{said:?}");
    }

    #[test]
    fn account_privacy_fetch_fills_the_rows() {
        let mut app = app();
        app.account_privacy = crate::privacy::Snapshot::default();
        app.account_privacy.apply_fetch(
            vec![(
                crate::privacy::PrivacyKind::LastSeen,
                crate::privacy::PrivacyChoice::Nobody,
            )],
            false,
        );
        assert_eq!(
            app.account_privacy
                .get(crate::privacy::PrivacyKind::LastSeen),
            Some(crate::privacy::PrivacyChoice::Nobody)
        );
        assert!(app.account_privacy.loaded);
    }

    #[test]
    fn picking_the_current_reaction_from_the_picker_clears_it() {
        let mut app = app();
        apply_flags(&mut app, Some("react-custom"));
        let chat = sample_ids()[0].to_owned();
        let current = app
            .conversations
            .get(&chat)
            .and_then(|conversation| conversation.message("ada-link"))
            .and_then(crate::ui::conversation::own_reaction);
        assert_eq!(current, Some("🦀"));
        assert_eq!(crate::ui::conversation::reaction_choice(current, "🦀"), "");
        assert_eq!(
            crate::ui::conversation::reaction_choice(current, "🎉"),
            "🎉"
        );
    }

    #[test]
    fn another_users_trophy_reaction_is_on_the_group_photo() {
        let mut app = app();
        apply_flags(&mut app, Some("react-other"));
        assert_eq!(app.open_chat.as_deref(), Some(sample_ids()[1]));
        let photo = app
            .conversations
            .get(sample_ids()[1])
            .and_then(|conversation| conversation.message("group-photo"))
            .expect("group photo");
        assert!(
            photo
                .reactions
                .iter()
                .any(|reaction| !reaction.from_me && reaction.emoji == "🏆"),
            "Mira's trophy should sit on the group photo"
        );
    }

    #[test]
    fn the_favorites_chip_lists_favorites() {
        use crate::model::ChatFilter;
        let mut app = app();
        // Every chip has to be on screen to be clicked, and the row scrolls
        // once the sidebar is too narrow for all of them.
        app.settings.sidebar_width = 520.0;
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        render(&mut app, &ctx);
        // Clicking the chip's own rect, so a translated label does not matter.
        let click = |app: &mut App, filter: ChatFilter| {
            let rect = ctx
                .data(|data| data.get_temp::<egui::Rect>(crate::ui::chats::filter_chip_id(filter)))
                .expect("the chip is on screen");
            let pos = rect.center();
            let press = |pressed| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            frame_with(app, &ctx, vec![egui::Event::PointerMoved(pos), press(true)]);
            frame_with(app, &ctx, vec![press(false)]);
            render(app, &ctx);
        };
        click(&mut app, ChatFilter::Favorites);
        assert_eq!(app.chat_filter, ChatFilter::Favorites);
        let favorites = app.visible_chats();
        assert!(!favorites.is_empty(), "the sample has a favorite");
        assert!(favorites.iter().all(|chat| chat.favorite));
        let favorite = favorites[0].clone();
        // The mark itself comes off the menu, and the chip follows it.
        let mark = app.chat(&favorite.id).expect("the chat").favorite;
        app.actions.push(crate::model::Action::SetFavorite(
            favorite.id.clone(),
            !mark,
        ));
        render(&mut app, &ctx);
        assert!(!app.chat(&favorite.id).expect("the chat").favorite);
        assert!(
            app.visible_chats().iter().all(|chat| chat.favorite),
            "an unmarked chat leaves the chip"
        );
    }

    #[test]
    fn a_filter_chip_narrows_the_chat_list_and_a_second_click_clears_it() {
        use crate::model::ChatFilter;
        let mut app = app();
        // Every chip has to be on screen to be clicked, and the row scrolls
        // once the sidebar is too narrow for all of them.
        app.settings.sidebar_width = 520.0;
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        render(&mut app, &ctx);
        let everything = app.visible_chats().len();
        let click = |app: &mut App, filter| {
            let rect = ctx
                .data(|data| data.get_temp::<egui::Rect>(crate::ui::chats::filter_chip_id(filter)))
                .expect("the chip is on screen");
            let pos = rect.center();
            let press = |pressed| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            frame_with(app, &ctx, vec![egui::Event::PointerMoved(pos), press(true)]);
            frame_with(app, &ctx, vec![press(false)]);
            render(app, &ctx);
        };
        click(&mut app, ChatFilter::Groups);
        assert_eq!(app.chat_filter, ChatFilter::Groups);
        let groups = app.visible_chats();
        assert!(!groups.is_empty() && groups.len() < everything);
        assert!(groups.iter().all(|chat| chat.is_group()));
        click(&mut app, ChatFilter::Groups);
        assert_eq!(app.chat_filter, ChatFilter::All);
        assert_eq!(app.visible_chats().len(), everything);
    }

    #[test]
    fn a_chat_clicked_in_the_unread_list_stays_there_once_read() {
        use crate::model::ChatFilter;
        let mut app = app();
        app.chats
            .iter_mut()
            .find(|chat| chat.name == "Grace Hopper")
            .expect("sample chat")
            .unread = 1;
        app.chat_filter = ChatFilter::Unread;
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        render(&mut app, &ctx);
        let listed: Vec<String> = app
            .visible_chats()
            .iter()
            .map(|chat| chat.id.clone())
            .collect();
        assert!(listed.len() >= 2, "the sample has several unread chats");
        for id in &listed {
            let rect = ctx
                .data(|data| data.get_temp::<egui::Rect>(crate::ui::chats::chat_row_id(id)))
                .expect("the row is on screen");
            let pos = rect.center();
            let press = |pressed| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            frame_with(
                &mut app,
                &ctx,
                vec![egui::Event::PointerMoved(pos), press(true)],
            );
            frame_with(&mut app, &ctx, vec![press(false)]);
            assert_eq!(app.open_chat.as_deref(), Some(id.as_str()));
            // The headless window may not read the chat; read it here.
            for chat in &mut app.chats {
                if chat.id == *id {
                    chat.unread = 0;
                }
            }
            render(&mut app, &ctx);
        }
        let after: Vec<String> = app
            .visible_chats()
            .iter()
            .map(|chat| chat.id.clone())
            .collect();
        assert_eq!(after, listed, "every opened chat is still listed");
    }

    /// Opening the dialog must not delete anything on its own, and confirming
    /// must delete with the scope the menu asked for.
    #[test]
    fn confirming_a_message_deletion_uses_the_chosen_scope() {
        for (page, message, expected_everyone) in [
            ("delete-message", "ada-emoji", true),
            ("delete-message-mine", "ada-format", false),
        ] {
            let mut app = app();
            let (backend, mut commands, events) = crate::backend::Backend::recording_with_events();
            app.backend = backend;
            apply_flags(&mut app, Some(page));
            assert_eq!(
                app.dialog,
                Some(crate::model::Dialog::ConfirmDeleteMessage {
                    chat: SAMPLES[0].id.to_owned(),
                    message: message.to_owned(),
                    for_everyone: expected_everyone,
                })
            );
            let ctx = egui::Context::default();
            ctx.enable_accesskit();
            app.attach(&ctx);
            render(&mut app, &ctx);
            // Laying the dialog out must not delete the message.
            assert!(
                app.conversations
                    .values()
                    .any(|conversation| conversation.message(message).is_some()),
                "{page}: the message survives an open dialog"
            );

            let pos = accessible_nodes(&mut app, &ctx, Vec::new())
                .into_iter()
                .find(|(label, role, _)| {
                    label == "Delete" && *role == egui::accesskit::Role::Button
                })
                .map(|(_, _, centre)| centre)
                .expect("the confirm button is on screen");
            let press = |pressed| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            frame_with(
                &mut app,
                &ctx,
                vec![egui::Event::PointerMoved(pos), press(true)],
            );
            frame_with(&mut app, &ctx, vec![press(false)]);

            assert!(app.dialog.is_none(), "{page}: the dialog closes");
            let row = app
                .conversations
                .values()
                .find_map(|conversation| conversation.message(message));
            if expected_everyone {
                assert!(
                    matches!(
                        row.map(|message| &message.content),
                        Some(crate::model::Content::Revoked)
                    ),
                    "{page}: a revoked message stays as a tombstone"
                );
            } else {
                assert!(
                    row.is_some(),
                    "{page}: the message waits for sync acceptance"
                );
                assert!(std::iter::from_fn(|| commands.try_recv().ok()).any(
                    |command| matches!(command, crate::backend::Command::DeleteLocal { chat, id }
                        if chat == SAMPLES[0].id && id == message)
                ));
                events
                    .send(crate::backend::Event::MessageDeleted {
                        chat: SAMPLES[0].id.to_owned(),
                        id: message.to_owned(),
                    })
                    .unwrap();
                app.background_frame(&ctx);
                assert!(app.conversations[SAMPLES[0].id].message(message).is_none());
            }
        }
    }

    /// Alt+Up/Down and Ctrl+Shift+[ ] switch chats while the question is up.
    /// Confirming afterwards must still delete in the chat the message came
    /// from, not in whichever chat is open by then.
    #[test]
    fn a_message_deletion_confirmed_after_switching_chats_stays_in_its_chat() {
        let own_chat = SAMPLES[0].id;
        for (page, message, everyone) in [
            ("delete-message", "ada-emoji", true),
            ("delete-message-mine", "ada-format", false),
        ] {
            let mut app = app();
            let (backend, mut commands, events) = crate::backend::Backend::recording_with_events();
            app.backend = backend;
            apply_flags(&mut app, Some(page));
            app.open_chat = Some(SAMPLES[1].id.to_owned());
            let ctx = egui::Context::default();
            ctx.enable_accesskit();
            app.attach(&ctx);
            render(&mut app, &ctx);

            let pos = accessible_nodes(&mut app, &ctx, Vec::new())
                .into_iter()
                .find(|(label, role, _)| {
                    label == "Delete" && *role == egui::accesskit::Role::Button
                })
                .map(|(_, _, centre)| centre)
                .expect("the confirm button is on screen");
            let press = |pressed| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            frame_with(
                &mut app,
                &ctx,
                vec![egui::Event::PointerMoved(pos), press(true)],
            );
            frame_with(&mut app, &ctx, vec![press(false)]);

            let row = app.conversations[own_chat].message(message);
            if everyone {
                assert!(
                    matches!(
                        row.map(|message| &message.content),
                        Some(crate::model::Content::Revoked)
                    ),
                    "{page}: the message is revoked in its own chat"
                );
            } else {
                assert!(row.is_some(), "{page}: the message waits in its own chat");
                assert!(std::iter::from_fn(|| commands.try_recv().ok()).any(
                    |command| matches!(command, crate::backend::Command::DeleteLocal { chat, id }
                        if chat == own_chat && id == message)
                ));
                events
                    .send(crate::backend::Event::MessageDeleted {
                        chat: own_chat.to_owned(),
                        id: message.to_owned(),
                    })
                    .unwrap();
                app.background_frame(&ctx);
                assert!(app.conversations[own_chat].message(message).is_none());
            }
        }
    }

    /// The confirmation opens over the sticker picker; answering it must not
    /// count as a click outside the picker.
    #[test]
    fn confirming_a_pack_removal_keeps_the_picker_open() {
        let mut app = app();
        // Without the chat list the picker sits left of the centred dialog.
        apply_flags(&mut app, Some("remove-pack,nosidebar"));
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        app.attach(&ctx);
        render(&mut app, &ctx);
        assert!(app.picker.is_some(), "the picker is open under the dialog");
        let pos = accessible_nodes(&mut app, &ctx, Vec::new())
            .into_iter()
            .find(|(label, role, _)| label == "Remove" && *role == egui::accesskit::Role::Button)
            .map(|(_, _, centre)| centre)
            .expect("the confirm button is on screen");
        let picker = ctx
            .memory(|memory| memory.area_rect(egui::Id::new("picker")))
            .expect("the picker is laid out");
        assert!(
            !picker.contains(pos),
            "the button must lie outside the picker for this test to mean anything"
        );
        let press = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame_with(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(pos), press(true)],
        );
        frame_with(&mut app, &ctx, vec![press(false)]);
        assert!(app.dialog.is_none(), "the dialog closes");
        assert_eq!(
            app.picker,
            Some(crate::model::PickerTab::Stickers),
            "the picker stays open"
        );
    }

    #[test]
    fn errors_stay_until_dismissed_while_info_fades() {
        let mut app = app();
        app.toast("Copied");
        app.toast_error("Could not open the folder: permission denied");
        let long_ago = std::time::Instant::now() - std::time::Duration::from_secs(60);
        for toast in &mut app.toasts {
            toast.created = long_ago;
        }
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        let kinds: Vec<_> = app.toasts.iter().map(|toast| toast.kind.clone()).collect();
        assert_eq!(kinds, [crate::model::ToastKind::Error]);

        let rect = ctx
            .data(|data| data.get_temp::<egui::Rect>(crate::ui::toast_close_id(0)))
            .expect("the error has a close button");
        let pos = rect.center();
        let press = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame_with(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(pos), press(true)],
        );
        frame_with(&mut app, &ctx, vec![press(false)]);
        assert!(app.toasts.is_empty(), "the close button dismisses it");
    }

    #[test]
    fn repeated_errors_neither_stack_nor_pile_up() {
        let mut app = app();
        app.toast_error("Offline");
        app.toast_error("Offline");
        assert_eq!(app.toasts.len(), 1, "a repeat replaces the earlier copy");
        for index in 0..5 {
            app.toast_error(format!("Failure {index}"));
        }
        let messages: Vec<_> = app
            .toasts
            .iter()
            .map(|toast| toast.message.as_str())
            .collect();
        assert_eq!(messages, ["Failure 2", "Failure 3", "Failure 4"]);
        for index in 0..8 {
            app.toast(format!("Information {index}"));
        }
        let errors: Vec<_> = app
            .toasts
            .iter()
            .filter(|toast| toast.kind == crate::model::ToastKind::Error)
            .map(|toast| toast.message.as_str())
            .collect();
        assert_eq!(errors, ["Failure 2", "Failure 3", "Failure 4"]);
    }

    #[test]
    fn toasts_leave_the_composer_uncovered() {
        let mut app = app();
        apply_flags(&mut app, Some("toasts"));
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        let composer = ctx
            .data(|data| data.get_temp::<egui::Rect>(crate::ui::composer_rect_id()))
            .expect("a chat is open");
        let toasts = ctx
            .memory(|memory| memory.area_rect(egui::Id::new("toasts")))
            .expect("toasts are shown");
        assert!(
            toasts.bottom() <= composer.top(),
            "toasts {toasts:?} overlap the composer {composer:?}"
        );
    }

    #[test]
    fn toast_text_and_buttons_share_a_vertical_center() {
        for message in [
            "This message is not stored on this computer",
            "A longer synthetic error with enough words to wrap onto several lines without pushing the buttons out of alignment 🙂",
        ] {
            let mut app = app();
            app.toast_error(message);
            let ctx = egui::Context::default();
            app.attach(&ctx);
            for _ in 0..3 {
                render(&mut app, &ctx);
            }
            let id = crate::ui::toast_close_id(0);
            let (text, close) = ctx.data(|data| {
                (
                    data.get_temp::<egui::Rect>(id.with("text")).unwrap(),
                    data.get_temp::<egui::Rect>(id).unwrap(),
                )
            });
            assert!(
                (text.center().y - close.center().y).abs() < 1.0,
                "text {text:?}, close {close:?}"
            );
        }
    }

    #[test]
    fn bubble_hit_rects_follow_the_layout() {
        // Ensure the right-click rect follows messages after initial scrolling.
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        let chat = sample_ids()[0].to_owned();
        let id = crate::ui::conversation::bubble_id(&chat, "ada-link");
        for _ in 0..3 {
            render(&mut app, &ctx);
        }
        let settled = ctx.read_response(id).expect("on screen").rect;
        assert!(
            settled.top() >= 0.0 && settled.bottom() <= 780.0,
            "the last message's hit rect is where it is drawn: {settled:?}"
        );
    }

    #[test]
    fn an_opened_chat_stays_at_its_end_until_the_reader_scrolls() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        for _ in 0..3 {
            render(&mut app, &ctx);
        }
        assert!(app.at_bottom, "opens at the end");
        // Keep the view pinned when content grows after opening.
        let chat = sample_ids()[0].to_owned();
        let when = crate::util::now();
        let tall = message(
            &chat,
            "late-tall",
            false,
            when,
            Content::text("a\nb\nc\nd\ne\nf\ng\nh\ni\nj\nk\nl"),
        );
        app.conversations
            .get_mut(&chat)
            .expect("open chat")
            .messages
            .push(tall);
        for _ in 0..3 {
            render(&mut app, &ctx);
        }
        assert!(app.at_bottom, "still at the end after content grew");
        assert!(app.scroll_to_bottom, "and still pinned");
        // Wheel input releases automatic bottom pinning.
        frame_with(
            &mut app,
            &ctx,
            vec![
                egui::Event::PointerMoved(egui::pos2(800.0, 400.0)),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(0.0, 300.0),
                    modifiers: egui::Modifiers::NONE,
                    phase: egui::TouchPhase::Move,
                },
            ],
        );
        assert!(!app.scroll_to_bottom, "a wheel releases the pin");
    }

    #[test]
    fn page_keys_scroll_the_open_chat_and_jump_to_its_ends() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        let chat = sample_ids()[0].to_owned();
        let when = crate::util::now();
        let conversation = app.conversations.get_mut(&chat).expect("open chat");
        for n in 0..60 {
            conversation.messages.push(message(
                &chat,
                &format!("page-key-{n}"),
                n % 2 == 0,
                when - (60 - n),
                Content::text(format!("Line {n}")),
            ));
        }
        render(&mut app, &ctx);
        assert!(app.at_bottom, "opens at the end");
        // PgUp scrolls up by about a page and releases the pin to the end.
        frame_with(
            &mut app,
            &ctx,
            vec![key(egui::Key::PageUp, egui::Modifiers::NONE)],
        );
        settle_key_scroll(&mut app, &ctx);
        assert!(!app.at_bottom, "PgUp leaves the end");
        assert!(!app.scroll_to_bottom, "PgUp releases the pin");
        // PgDn pages back down by the same amount.
        frame_with(
            &mut app,
            &ctx,
            vec![key(egui::Key::PageDown, egui::Modifiers::NONE)],
        );
        settle_key_scroll(&mut app, &ctx);
        assert!(app.at_bottom, "PgDn returns to the end it paged from");
        // Home reaches the top of the loaded history.
        frame_with(
            &mut app,
            &ctx,
            vec![key(egui::Key::Home, egui::Modifiers::NONE)],
        );
        settle_key_scroll(&mut app, &ctx);
        assert!(!app.at_bottom, "Home leaves the end");
        // End returns to the newest message.
        frame_with(
            &mut app,
            &ctx,
            vec![key(egui::Key::End, egui::Modifiers::NONE)],
        );
        settle_key_scroll(&mut app, &ctx);
        assert!(app.at_bottom, "End reaches the newest message");
        // With text in the focused composer, Home moves the text cursor
        // instead of scrolling.
        app.composer = "draft".into();
        frame_with(
            &mut app,
            &ctx,
            vec![key(egui::Key::Home, egui::Modifiers::NONE)],
        );
        settle_key_scroll(&mut app, &ctx);
        assert!(app.at_bottom, "Home in a non-empty composer did not scroll");
        assert_eq!(app.composer, "draft", "the composer text is unchanged");
    }

    /// Pushes `count` short synthetic messages onto the open sample chat so
    /// its message list needs several pages to scroll through.
    fn lengthen_chat(app: &mut App, chat: &str, count: i32, prefix: &str) {
        let when = crate::util::now();
        let conversation = app.conversations.get_mut(chat).expect("open chat");
        for n in 0..count {
            conversation.messages.push(message(
                chat,
                &format!("{prefix}-{n}"),
                n % 2 == 0,
                when - i64::from(count - n),
                Content::text(format!("Line {n}")),
            ));
        }
    }

    /// Goes to the top and back to the end, so every row has been measured.
    /// A page step also carries the offset along with rows above the
    /// viewport that grow when first measured, so exact offset checks need
    /// them measured first.
    fn measure_all_rows(app: &mut App, ctx: &egui::Context) {
        for end in [egui::Key::Home, egui::Key::End] {
            frame_with(app, ctx, vec![key(end, egui::Modifiers::NONE)]);
            settle_key_scroll(app, ctx);
        }
    }

    /// The open chat's message list scroll offset and viewport height, as
    /// stashed by `conversation::scroll_metrics_id`.
    fn scroll_metrics(ctx: &egui::Context, chat: &str) -> (f32, f32) {
        ctx.data(|data| data.get_temp(crate::ui::conversation::scroll_metrics_id(&chat.to_owned())))
            .expect("the message list has drawn")
    }

    /// Inserts a voted poll at `index`, whose "Show votes" button is a real
    /// focusable control inside its bubble's rect (a message bubble itself
    /// uses `Sense::CLICK`, which egui never focuses).
    fn insert_poll_message(app: &mut App, chat: &str, index: usize, id: &str, when: i64) {
        let conversation = app.conversations.get_mut(chat).expect("open chat");
        let poll = message(
            chat,
            id,
            false,
            when,
            Content::Poll {
                question: "Pizza tonight?".into(),
                options: vec!["Yes".into(), "No".into()],
                state: crate::model::PollState {
                    voters: 1,
                    ..Default::default()
                },
            },
        );
        conversation.messages.insert(index, poll);
    }

    /// Gives a poll's "Show votes" button keyboard focus, the way real Tab
    /// navigation does (see `track_keyboard_focus` in `ui/mod.rs`).
    fn focus_bubble(ctx: &egui::Context, chat: &str, message_id: &str) {
        let target = crate::ui::conversation::bubble_id(chat, message_id).with("poll-results");
        ctx.data_mut(|data| data.insert_temp(crate::theme::keyboard_focus_id(), true));
        ctx.memory_mut(|memory| memory.request_focus(target));
    }

    #[test]
    fn ctrl_end_reaches_the_bottom_despite_a_focused_message_bubble() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        let chat = sample_ids()[0].to_owned();
        lengthen_chat(&mut app, &chat, 25, "bubble-focus");
        insert_poll_message(
            &mut app,
            &chat,
            20,
            "bubble-focus-poll",
            crate::util::now() - 5,
        );
        render(&mut app, &ctx);
        assert!(app.at_bottom, "opens at the end");
        frame_with(
            &mut app,
            &ctx,
            vec![key(egui::Key::PageUp, egui::Modifiers::NONE)],
        );
        settle_key_scroll(&mut app, &ctx);
        assert!(!app.at_bottom, "set up off the end");
        // A message bubble keeps keyboard focus, as it would while reading
        // older messages after Tab navigation.
        focus_bubble(&ctx, &chat, "bubble-focus-poll");
        render(&mut app, &ctx);
        // Ctrl+End is explicit: it must still reach the bottom.
        frame_with(
            &mut app,
            &ctx,
            vec![key(egui::Key::End, egui::Modifiers::COMMAND)],
        );
        settle_key_scroll(&mut app, &ctx);
        assert!(
            app.at_bottom,
            "Ctrl+End reached the end despite the focused bubble"
        );
    }

    #[test]
    fn plain_end_animates_to_the_bottom_despite_a_focused_message_bubble() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        let chat = sample_ids()[0].to_owned();
        lengthen_chat(&mut app, &chat, 25, "end-focus");
        insert_poll_message(
            &mut app,
            &chat,
            20,
            "end-focus-poll",
            crate::util::now() - 5,
        );
        render(&mut app, &ctx);
        frame_with(
            &mut app,
            &ctx,
            vec![key(egui::Key::PageUp, egui::Modifiers::NONE)],
        );
        settle_key_scroll(&mut app, &ctx);
        assert!(!app.at_bottom, "set up off the end");
        focus_bubble(&ctx, &chat, "end-focus-poll");
        render(&mut app, &ctx);
        // End does not depend on the keyboard-navigation guard at all, so it
        // reaches the bottom despite the focused bubble, eased rather than
        // instant.
        frame_with(
            &mut app,
            &ctx,
            vec![key(egui::Key::End, egui::Modifiers::NONE)],
        );
        render(&mut app, &ctx);
        assert!(!app.at_bottom, "End eases rather than jumping instantly");
        settle_key_scroll(&mut app, &ctx);
        assert!(
            app.at_bottom,
            "End reached the end despite the focused bubble"
        );
    }

    #[test]
    fn home_reaches_the_top_exactly_and_pgup_moves_about_a_page() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        let chat = sample_ids()[0].to_owned();
        lengthen_chat(&mut app, &chat, 60, "home-exact");
        render(&mut app, &ctx);
        measure_all_rows(&mut app, &ctx);
        let (before, viewport_height) = scroll_metrics(&ctx, &chat);
        frame_with(
            &mut app,
            &ctx,
            vec![key(egui::Key::PageUp, egui::Modifiers::NONE)],
        );
        settle_key_scroll(&mut app, &ctx);
        let (after_page_up, _) = scroll_metrics(&ctx, &chat);
        let moved = before - after_page_up;
        assert!(
            (moved - viewport_height * 0.9).abs() < 2.0,
            "PgUp moved {moved}, expected about {} of a {viewport_height} viewport",
            viewport_height * 0.9
        );
        frame_with(
            &mut app,
            &ctx,
            vec![key(egui::Key::Home, egui::Modifiers::NONE)],
        );
        settle_key_scroll(&mut app, &ctx);
        let (after_home, _) = scroll_metrics(&ctx, &chat);
        assert!(
            after_home.abs() < 0.5,
            "Home settled at {after_home}, not the top"
        );
    }

    #[test]
    fn pgup_eases_toward_its_target_instead_of_jumping() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        let chat = sample_ids()[0].to_owned();
        lengthen_chat(&mut app, &chat, 60, "pgup-anim");
        render(&mut app, &ctx);
        measure_all_rows(&mut app, &ctx);
        let (before, viewport_height) = scroll_metrics(&ctx, &chat);
        let target = before - viewport_height * 0.9;
        frame_with(
            &mut app,
            &ctx,
            vec![key(egui::Key::PageUp, egui::Modifiers::NONE)],
        );
        render(&mut app, &ctx);
        let (soon, _) = scroll_metrics(&ctx, &chat);
        assert!(
            soon < before && soon > target,
            "PgUp jumped straight to {soon} instead of easing from {before} toward {target}"
        );
        settle_key_scroll(&mut app, &ctx);
        let (settled, _) = scroll_metrics(&ctx, &chat);
        assert!(
            (settled - target).abs() < 2.0,
            "PgUp settled at {settled}, expected about {target}"
        );
    }

    #[test]
    fn a_wheel_event_stops_an_in_flight_key_scroll_animation() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        let chat = sample_ids()[0].to_owned();
        lengthen_chat(&mut app, &chat, 60, "wheel-stop");
        render(&mut app, &ctx);
        frame_with(
            &mut app,
            &ctx,
            vec![key(egui::Key::PageUp, egui::Modifiers::NONE)],
        );
        render(&mut app, &ctx);
        // A wheel event interrupts the in-flight animation.
        frame_with(
            &mut app,
            &ctx,
            vec![
                egui::Event::PointerMoved(egui::pos2(800.0, 400.0)),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(0.0, -300.0),
                    modifiers: egui::Modifiers::NONE,
                    phase: egui::TouchPhase::Move,
                },
            ],
        );
        let (after_wheel, _) = scroll_metrics(&ctx, &chat);
        // The wheel turns down, against PgUp. egui eases a wheel turn over a
        // few frames, so the offset may keep moving down; a PgUp animation
        // still in flight would pull it back up toward its target instead.
        settle_key_scroll(&mut app, &ctx);
        let (later, _) = scroll_metrics(&ctx, &chat);
        assert!(
            later >= after_wheel - 2.0,
            "the PgUp animation kept pulling up ({after_wheel} -> {later}) after the wheel event"
        );
    }

    #[test]
    fn ctrl_end_during_a_pgup_animation_still_reaches_the_bottom() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        let chat = sample_ids()[0].to_owned();
        lengthen_chat(&mut app, &chat, 60, "redirect");
        render(&mut app, &ctx);
        frame_with(
            &mut app,
            &ctx,
            vec![key(egui::Key::PageUp, egui::Modifiers::NONE)],
        );
        // Partway through the eased eighteen or so frames: enough progress to
        // leave the end, but the PgUp animation is still under way.
        for _ in 0..3 {
            render(&mut app, &ctx);
        }
        assert!(!app.at_bottom, "PgUp is under way");
        // An instant jump requested while the PgUp animation is in flight
        // must still land exactly at the end, not be pulled back toward the
        // PgUp target by the animation still in progress.
        frame_with(
            &mut app,
            &ctx,
            vec![key(egui::Key::End, egui::Modifiers::COMMAND)],
        );
        settle_key_scroll(&mut app, &ctx);
        assert!(
            app.at_bottom,
            "Ctrl+End during the PgUp animation still reaches the end"
        );
    }

    /// Presses a plain key, then lets one more frame run so the key scroll
    /// it queued starts and moves part of the way.
    fn press_and_step(app: &mut App, ctx: &egui::Context, key_pressed: egui::Key) {
        frame_with(app, ctx, vec![key(key_pressed, egui::Modifiers::NONE)]);
        render(app, ctx);
    }

    #[test]
    fn home_during_an_end_animation_reaches_the_top_exactly() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        let chat = sample_ids()[0].to_owned();
        lengthen_chat(&mut app, &chat, 60, "end-home");
        render(&mut app, &ctx);
        press_and_step(&mut app, &ctx, egui::Key::PageUp);
        settle_key_scroll(&mut app, &ctx);
        press_and_step(&mut app, &ctx, egui::Key::End);
        press_and_step(&mut app, &ctx, egui::Key::Home);
        settle_key_scroll(&mut app, &ctx);
        let (offset, _) = scroll_metrics(&ctx, &chat);
        assert!(offset.abs() < 0.5, "Home settled at {offset}, not the top");
    }

    #[test]
    fn home_during_a_pgdn_animation_reaches_the_top_exactly() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        let chat = sample_ids()[0].to_owned();
        lengthen_chat(&mut app, &chat, 60, "pgdn-home");
        render(&mut app, &ctx);
        for _ in 0..2 {
            press_and_step(&mut app, &ctx, egui::Key::PageUp);
            settle_key_scroll(&mut app, &ctx);
        }
        press_and_step(&mut app, &ctx, egui::Key::PageDown);
        press_and_step(&mut app, &ctx, egui::Key::Home);
        settle_key_scroll(&mut app, &ctx);
        let (offset, _) = scroll_metrics(&ctx, &chat);
        assert!(offset.abs() < 0.5, "Home settled at {offset}, not the top");
    }

    #[test]
    fn end_includes_a_message_that_arrives_during_its_animation() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        let chat = sample_ids()[0].to_owned();
        lengthen_chat(&mut app, &chat, 60, "end-arrival");
        render(&mut app, &ctx);
        for _ in 0..2 {
            press_and_step(&mut app, &ctx, egui::Key::PageUp);
            settle_key_scroll(&mut app, &ctx);
        }
        press_and_step(&mut app, &ctx, egui::Key::End);
        assert!(!app.at_bottom, "End is under way");
        let tall = (0..30)
            .map(|n| format!("Line {n}"))
            .collect::<Vec<_>>()
            .join("\n");
        app.conversations
            .get_mut(&chat)
            .expect("open chat")
            .messages
            .push(message(
                &chat,
                "end-arrival-tall",
                false,
                crate::util::now() + 1,
                Content::text(tall),
            ));
        settle_key_scroll(&mut app, &ctx);
        assert!(
            app.at_bottom,
            "End reached the bottom below the new message"
        );
        assert!(app.scroll_to_bottom, "End pins later messages to the end");
    }

    #[test]
    fn ctrl_end_during_a_pgup_animation_jumps_at_once() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        let chat = sample_ids()[0].to_owned();
        lengthen_chat(&mut app, &chat, 60, "ctrl-end-now");
        render(&mut app, &ctx);
        press_and_step(&mut app, &ctx, egui::Key::PageUp);
        render(&mut app, &ctx);
        assert!(!app.at_bottom, "PgUp is under way");
        frame_with(
            &mut app,
            &ctx,
            vec![key(egui::Key::End, egui::Modifiers::COMMAND)],
        );
        frame_with(&mut app, &ctx, Vec::new());
        assert!(app.at_bottom, "Ctrl+End jumped to the end at once");
        let (jumped, _) = scroll_metrics(&ctx, &chat);
        for _ in 0..20 {
            frame_with(&mut app, &ctx, Vec::new());
            let (offset, _) = scroll_metrics(&ctx, &chat);
            assert!(
                offset >= jumped - 0.5 && app.at_bottom,
                "the PgUp animation pulled the view back up ({jumped} -> {offset})"
            );
        }
    }

    /// A chat long enough to scroll, open, whose contact is typing below its
    /// newest message, its message list focused from the keys, and the
    /// sender for the backend's events.
    fn typing_long_chat() -> (
        App,
        egui::Context,
        String,
        std::sync::mpsc::Sender<crate::backend::Event>,
    ) {
        let root = tempfile::Builder::new()
            .prefix("zapfast-demo-")
            .tempdir()
            .unwrap();
        let (mut app, events) = App::headless(AppDirs::under(root.path()), Settings::default());
        ROOTS.with(|roots| roots.borrow_mut().push(root));
        populate(&mut app);
        let ctx = egui::Context::default();
        app.attach(&ctx);
        // As with a screen reader attached, the case these keys are for.
        ctx.enable_accesskit();
        let chat = sample_ids()[0].to_owned();
        assert_eq!(app.open_chat.as_deref(), Some(chat.as_str()));
        lengthen_chat(&mut app, &chat, 60, "true-bottom");
        // The contact types for the whole test. The bubble goes away
        // `TYPING_TIMEOUT` after this instant by the wall clock, and a loaded
        // parallel run can take longer than that, dropping the bubble between
        // two measurements. An instant an hour ahead keeps it drawn however
        // long the run takes: the age of a later instant saturates to zero.
        app.typing.insert(
            chat.clone(),
            vec![(
                chat.clone(),
                std::time::Instant::now() + std::time::Duration::from_secs(3600),
            )],
        );
        render(&mut app, &ctx);
        press_key(&mut app, &ctx, egui::Key::Num2, messages_shortcut());
        assert_eq!(
            ctx.memory(|memory| memory.focused()),
            Some(crate::ui::conversation::messages_list_id(&chat)),
            "Alt+2 focused the message list"
        );
        (app, ctx, chat, events)
    }

    /// Whether the floating return-to-bottom button is drawn, found by its
    /// name in the accessibility tree.
    fn return_to_bottom_shown(app: &mut App, ctx: &egui::Context) -> bool {
        tree_after(app, ctx, None).nodes.iter().any(|(_, node)| {
            node.role() == egui::accesskit::Role::Button && node.label() == Some("Newest message")
        })
    }

    /// Where the pointer's return-to-bottom button takes the list: the same
    /// action as its click, from the offset the keys left.
    fn offset_after_the_button(app: &mut App, ctx: &egui::Context, chat: &str) -> f32 {
        app.actions.push(crate::model::Action::ScrollToBottom);
        settle_key_scroll(app, ctx);
        scroll_metrics(ctx, chat).0
    }

    /// Arrows the list's cursor up past the top of the view and back down
    /// to the message before the newest, as a reader does: each step down
    /// shows its message at the view's lower edge, short of the newest
    /// message and the typing bubble below it.
    fn short_of_the_end(app: &mut App, ctx: &egui::Context, chat: &str) {
        for _ in 0..25 {
            press_key(app, ctx, egui::Key::ArrowUp, egui::Modifiers::NONE);
        }
        settle_key_scroll(app, ctx);
        assert!(!app.at_bottom, "Up left the end");
        for _ in 0..24 {
            press_key(app, ctx, egui::Key::ArrowDown, egui::Modifiers::NONE);
        }
        settle_key_scroll(app, ctx);
        let messages = &app.conversations[chat].messages;
        assert_eq!(
            app.message_cursor.get(chat),
            Some(&messages[messages.len() - 2].id)
        );
        assert!(!app.at_bottom, "set up short of the end");
        assert!(
            return_to_bottom_shown(app, ctx),
            "the button is drawn, and named, away from the end"
        );
    }

    /// The view is where the return-to-bottom button would take it, with
    /// the list's cursor on the newest message.
    fn at_the_true_bottom(app: &mut App, ctx: &egui::Context, chat: &str, how: &str) {
        let newest = app.conversations[chat].messages.last().unwrap().id.clone();
        assert_eq!(app.message_cursor.get(chat), Some(&newest), "{how}");
        assert!(app.at_bottom, "{how} reached the end");
        assert!(
            !return_to_bottom_shown(app, ctx),
            "{how}: no button at the end"
        );
        let (offset, _) = scroll_metrics(ctx, chat);
        let button = offset_after_the_button(app, ctx, chat);
        assert!(
            (button - offset).abs() < 0.5,
            "{how} left the view at {offset}, the button at {button}: short of the typing bubble"
        );
    }

    /// G18: End in the focused message list does what the return-to-bottom
    /// button does: the view reaches the true bottom, below the typing
    /// bubble, so the button goes away and the bubble is in view. From near
    /// the end, and from the top.
    #[test]
    fn end_in_the_message_list_reaches_the_true_bottom() {
        let (mut app, ctx, chat, _events) = typing_long_chat();
        short_of_the_end(&mut app, &ctx, &chat);
        press_key(&mut app, &ctx, egui::Key::End, egui::Modifiers::NONE);
        settle_key_scroll(&mut app, &ctx);
        at_the_true_bottom(&mut app, &ctx, &chat, "End");

        press_key(&mut app, &ctx, egui::Key::Home, egui::Modifiers::NONE);
        settle_key_scroll(&mut app, &ctx);
        press_key(&mut app, &ctx, egui::Key::End, egui::Modifiers::NONE);
        settle_key_scroll(&mut app, &ctx);
        at_the_true_bottom(&mut app, &ctx, &chat, "End from the top");
    }

    /// G18: Down, or PgDn, onto the newest message reaches the true bottom
    /// as End does.
    #[test]
    fn down_onto_the_newest_message_reaches_the_true_bottom() {
        for key in [egui::Key::ArrowDown, egui::Key::PageDown] {
            let (mut app, ctx, chat, _events) = typing_long_chat();
            short_of_the_end(&mut app, &ctx, &chat);
            press_key(&mut app, &ctx, key, egui::Modifiers::NONE);
            settle_key_scroll(&mut app, &ctx);
            at_the_true_bottom(&mut app, &ctx, &chat, &format!("{key:?}"));
        }
    }

    /// G18: Alt+2 landing on the newest message, with the view short of it,
    /// reaches the true bottom too.
    #[test]
    fn alt_2_landing_on_the_newest_message_reaches_the_true_bottom() {
        let (mut app, ctx, chat, _events) = typing_long_chat();
        short_of_the_end(&mut app, &ctx, &chat);
        let newest = app.conversations[&chat].messages.last().unwrap().id.clone();
        app.message_cursor.insert(chat.clone(), newest);
        press_key(&mut app, &ctx, egui::Key::Tab, egui::Modifiers::NONE);
        assert_ne!(
            ctx.memory(|memory| memory.focused()),
            Some(crate::ui::conversation::messages_list_id(&chat)),
            "Tab left the list"
        );
        press_key(&mut app, &ctx, egui::Key::Num2, messages_shortcut());
        settle_key_scroll(&mut app, &ctx);
        at_the_true_bottom(&mut app, &ctx, &chat, "Alt+2");
    }

    /// G18: after End, a new message from the contact is followed, as it is
    /// after the button: the view stays at the true bottom below it.
    #[test]
    fn new_messages_follow_after_end_in_the_message_list() {
        let (mut app, ctx, chat, events) = typing_long_chat();
        short_of_the_end(&mut app, &ctx, &chat);
        press_key(&mut app, &ctx, egui::Key::End, egui::Modifiers::NONE);
        settle_key_scroll(&mut app, &ctx);
        assert!(app.at_bottom);
        let (before, _) = scroll_metrics(&ctx, &chat);
        let tall = (0..12)
            .map(|n| format!("Line {n}"))
            .collect::<Vec<_>>()
            .join("\n");
        events
            .send(crate::backend::Event::Messages {
                chat: chat.clone(),
                messages: vec![message(
                    &chat,
                    "true-bottom-arrival",
                    false,
                    crate::util::now() + 1,
                    Content::text(tall),
                )],
                older: false,
                complete: true,
            })
            .unwrap();
        settle_key_scroll(&mut app, &ctx);
        let (after, _) = scroll_metrics(&ctx, &chat);
        assert!(
            after > before + 100.0,
            "the view followed ({before} -> {after})"
        );
        assert!(app.at_bottom, "still at the end below the new message");
        assert!(!return_to_bottom_shown(&mut app, &ctx));
        assert!(
            (offset_after_the_button(&mut app, &ctx, &chat) - after).abs() < 0.5,
            "followed to where the button would take it"
        );
    }

    /// G18: after End, deleting the followed newest message moves the
    /// cursor to the message before it, and new messages are still followed
    /// from there.
    #[test]
    fn new_messages_follow_after_the_followed_message_is_deleted() {
        let (mut app, ctx, chat, events) = typing_long_chat();
        short_of_the_end(&mut app, &ctx, &chat);
        press_key(&mut app, &ctx, egui::Key::End, egui::Modifiers::NONE);
        settle_key_scroll(&mut app, &ctx);
        assert!(app.at_bottom);
        let messages = &app.conversations[&chat].messages;
        let newest = messages.last().unwrap().id.clone();
        let before_newest = messages[messages.len() - 2].id.clone();
        events
            .send(crate::backend::Event::MessageDeleted {
                chat: chat.clone(),
                id: newest,
            })
            .unwrap();
        settle_key_scroll(&mut app, &ctx);
        assert_eq!(app.message_cursor[&chat], before_newest);
        let (before, _) = scroll_metrics(&ctx, &chat);
        let tall = (0..12)
            .map(|n| format!("Line {n}"))
            .collect::<Vec<_>>()
            .join("\n");
        events
            .send(crate::backend::Event::Messages {
                chat: chat.clone(),
                messages: vec![message(
                    &chat,
                    "after-deletion-arrival",
                    false,
                    crate::util::now() + 1,
                    Content::text(tall),
                )],
                older: false,
                complete: true,
            })
            .unwrap();
        settle_key_scroll(&mut app, &ctx);
        let (after, _) = scroll_metrics(&ctx, &chat);
        assert!(
            after > before + 100.0,
            "the view followed ({before} -> {after})"
        );
        assert!(app.at_bottom, "still at the end below the new message");
        assert!(!return_to_bottom_shown(&mut app, &ctx));
        assert!(
            (offset_after_the_button(&mut app, &ctx, &chat) - after).abs() < 0.5,
            "followed to where the button would take it"
        );
        assert_eq!(
            app.cursor_followed,
            Some((chat.clone(), before_newest)),
            "the cursor's new message is followed"
        );
    }

    /// G18: Ctrl+End with the list's cursor on the newest message, reached
    /// by a jump rather than the arrows, reaches the true bottom and follows
    /// new messages there too.
    #[test]
    fn ctrl_end_on_the_newest_message_follows_new_messages() {
        let (mut app, ctx, chat, events) = typing_long_chat();
        short_of_the_end(&mut app, &ctx, &chat);
        let newest = app.conversations[&chat].messages.last().unwrap().id.clone();
        app.message_cursor.insert(chat.clone(), newest);
        press_key(&mut app, &ctx, egui::Key::End, egui::Modifiers::COMMAND);
        settle_key_scroll(&mut app, &ctx);
        at_the_true_bottom(&mut app, &ctx, &chat, "Ctrl+End");
        let (before, _) = scroll_metrics(&ctx, &chat);
        events
            .send(crate::backend::Event::Messages {
                chat: chat.clone(),
                messages: vec![message(
                    &chat,
                    "ctrl-end-arrival",
                    false,
                    crate::util::now() + 1,
                    Content::text("One more\nover\nthree lines"),
                )],
                older: false,
                complete: true,
            })
            .unwrap();
        settle_key_scroll(&mut app, &ctx);
        let (after, _) = scroll_metrics(&ctx, &chat);
        assert!(
            after > before + 20.0,
            "the view followed ({before} -> {after})"
        );
        assert!(app.at_bottom);
    }

    /// G18: the wheel still scrolls away from the end after End; the keys'
    /// jump to the end does not hold the view there against the pointer.
    #[test]
    fn the_wheel_still_leaves_the_end_after_end_in_the_message_list() {
        let (mut app, ctx, chat, _events) = typing_long_chat();
        short_of_the_end(&mut app, &ctx, &chat);
        press_key(&mut app, &ctx, egui::Key::End, egui::Modifiers::NONE);
        settle_key_scroll(&mut app, &ctx);
        assert!(app.at_bottom);
        let (bottom, _) = scroll_metrics(&ctx, &chat);
        let over_messages = egui::pos2(700.0, 400.0);
        for _ in 0..3 {
            frame_with(
                &mut app,
                &ctx,
                vec![
                    egui::Event::PointerMoved(over_messages),
                    egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        delta: egui::vec2(0.0, 120.0),
                        modifiers: egui::Modifiers::NONE,
                        phase: egui::TouchPhase::Move,
                    },
                ],
            );
        }
        for _ in 0..30 {
            frame_with(&mut app, &ctx, Vec::new());
        }
        let (offset, _) = scroll_metrics(&ctx, &chat);
        assert!(
            offset < bottom - 100.0,
            "the wheel scrolled up ({bottom} -> {offset})"
        );
        assert!(!app.at_bottom);
        assert!(!app.scroll_to_bottom, "nothing pulls the view back down");
        assert!(return_to_bottom_shown(&mut app, &ctx));
    }

    /// Runs one frame with input events at an explicit input time.
    fn frame_at(app: &mut App, ctx: &egui::Context, events: Vec<egui::Event>, time: f64) {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1180.0, 780.0),
                )),
                time: Some(time),
                events,
                ..Default::default()
            },
            |ui| {
                let ctx = ui.ctx().clone();
                app.background_frame(&ctx);
                app.frame_ui(ui);
            },
        );
        output.textures_delta.clear();
    }

    /// Runs `frames` frames 1/60 s apart, each with `events`, and returns
    /// the message list offset after each.
    fn frames_at_60(
        app: &mut App,
        ctx: &egui::Context,
        chat: &str,
        time: &mut f64,
        frames: usize,
        events: &[egui::Event],
    ) -> Vec<f32> {
        (0..frames)
            .map(|_| {
                *time += 1.0 / 60.0;
                frame_at(app, ctx, events.to_vec(), *time);
                scroll_metrics(ctx, chat).0
            })
            .collect()
    }

    #[test]
    fn a_held_pgup_moves_on_every_frame_and_stops_after_release() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        let chat = sample_ids()[0].to_owned();
        lengthen_chat(&mut app, &chat, 400, "held-pgup");
        render(&mut app, &ctx);
        let mut time = ctx.input(|input| input.time);
        let held = [key(egui::Key::PageUp, egui::Modifiers::NONE)];
        // A first run measures the rows the held key passes, so the second
        // run's offsets carry no row-height compensation.
        frames_at_60(&mut app, &ctx, &chat, &mut time, 20, &held);
        frames_at_60(&mut app, &ctx, &chat, &mut time, 40, &[]);
        let end = [key(egui::Key::End, egui::Modifiers::NONE)];
        frames_at_60(&mut app, &ctx, &chat, &mut time, 1, &end);
        frames_at_60(&mut app, &ctx, &chat, &mut time, 40, &[]);
        assert!(app.at_bottom, "set up at the end");
        let (before, viewport_height) = scroll_metrics(&ctx, &chat);
        let offsets = frames_at_60(&mut app, &ctx, &chat, &mut time, 20, &held);
        // The first frame only queues the key; each later one moves.
        let mut previous = offsets[0];
        let mut stalled = 0;
        for (frame, &offset) in offsets.iter().enumerate().skip(1) {
            if offset < previous - 0.5 {
                stalled = 0;
            } else {
                stalled += 1;
                assert!(
                    stalled <= 1,
                    "a held PgUp paused at frame {frame}: {offsets:?}"
                );
            }
            previous = offset;
        }
        // After release it settles within the animation's longest duration.
        let released = frames_at_60(&mut app, &ctx, &chat, &mut time, 40, &[]);
        let settled = released[20];
        assert!(
            (released[39] - settled).abs() < 0.5,
            "still moving after release: {released:?}"
        );
        let expected = before - 20.0 * viewport_height * 0.9;
        assert!(
            (settled - expected).abs() < 3.0,
            "twenty PgUp presses settled at {settled}, expected about {expected}"
        );
    }

    #[test]
    fn a_wheel_turn_stops_home_before_its_next_step() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        let chat = sample_ids()[0].to_owned();
        lengthen_chat(&mut app, &chat, 400, "wheel-home");
        render(&mut app, &ctx);
        let mut time = ctx.input(|input| input.time);
        // A held PgUp to the top and End back measure every row, so the
        // offsets below carry no row-height compensation.
        let held = [key(egui::Key::PageUp, egui::Modifiers::NONE)];
        frames_at_60(&mut app, &ctx, &chat, &mut time, 80, &held);
        let end = [key(egui::Key::End, egui::Modifiers::NONE)];
        frames_at_60(&mut app, &ctx, &chat, &mut time, 1, &end);
        frames_at_60(&mut app, &ctx, &chat, &mut time, 40, &[]);
        assert!(app.at_bottom, "set up at the end");
        let home = [key(egui::Key::Home, egui::Modifiers::NONE)];
        frames_at_60(&mut app, &ctx, &chat, &mut time, 1, &home);
        let under_way = frames_at_60(&mut app, &ctx, &chat, &mut time, 3, &[]);
        let before = under_way[2];
        assert!(before > 5000.0, "Home is far from the top: {before}");
        let wheel = [
            egui::Event::PointerMoved(egui::pos2(800.0, 400.0)),
            egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, -300.0),
                modifiers: egui::Modifiers::NONE,
                phase: egui::TouchPhase::Move,
            },
        ];
        let turned = frames_at_60(&mut app, &ctx, &chat, &mut time, 1, &wheel)[0];
        // The wheel turns down, against Home: the offset may only grow.
        assert!(
            turned >= before - 0.5,
            "Home still stepped on the wheel's frame ({before} -> {turned})"
        );
        let later = frames_at_60(&mut app, &ctx, &chat, &mut time, 30, &[]);
        assert!(
            later.iter().all(|&offset| offset >= turned - 0.5),
            "Home kept stepping after the wheel: {later:?}"
        );
    }

    #[test]
    fn a_repeated_pgup_during_its_animation_adds_another_page() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        let chat = sample_ids()[0].to_owned();
        lengthen_chat(&mut app, &chat, 60, "pgup-repeat");
        render(&mut app, &ctx);
        measure_all_rows(&mut app, &ctx);
        let (before, viewport_height) = scroll_metrics(&ctx, &chat);
        press_and_step(&mut app, &ctx, egui::Key::PageUp);
        press_and_step(&mut app, &ctx, egui::Key::PageUp);
        settle_key_scroll(&mut app, &ctx);
        let (settled, _) = scroll_metrics(&ctx, &chat);
        let expected = before - viewport_height * 1.8;
        assert!(
            (settled - expected).abs() < 2.0,
            "two PgUp presses settled at {settled}, expected about {expected}"
        );
    }

    /// Platforms deliver only the Ctrl+V release for a picture paste, and a
    /// quick tap lets go of Ctrl before V (#494). A typed V is never a paste.
    #[test]
    fn a_picture_paste_is_seen_on_the_key_release() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        let picture = || {
            ctx.data_mut(|data| {
                data.insert_temp(
                    egui::Id::new("fixture-clipboard-image"),
                    (2_usize, 2_usize, vec![200_u8; 16]),
                )
            })
        };
        let v = |pressed, modifiers| egui::Event::Key {
            key: egui::Key::V,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers,
        };
        let command = egui::Event::ModifiersChanged(egui::Modifiers::COMMAND);
        let none = egui::Event::ModifiersChanged(egui::Modifiers::NONE);
        // Ctrl held until V goes up; egui-winit swallowed the press.
        picture();
        frame_with(&mut app, &ctx, vec![command.clone()]);
        frame_with(
            &mut app,
            &ctx,
            vec![v(false, egui::Modifiers::COMMAND), none.clone()],
        );
        assert_eq!(app.pending.len(), 1, "a held Ctrl+V");
        // A quick tap: Ctrl comes up first, all within one frame.
        picture();
        frame_with(
            &mut app,
            &ctx,
            vec![
                command.clone(),
                none.clone(),
                v(false, egui::Modifiers::NONE),
            ],
        );
        assert_eq!(app.pending.len(), 2, "a quick tap releasing Ctrl first");
        // The same tap spread over frames.
        picture();
        frame_with(&mut app, &ctx, vec![command]);
        frame_with(&mut app, &ctx, vec![none]);
        frame_with(&mut app, &ctx, vec![v(false, egui::Modifiers::NONE)]);
        assert_eq!(app.pending.len(), 3, "a tap across frames");
        // Typing V reaches us as a press, and its release is no paste.
        picture();
        frame_with(&mut app, &ctx, vec![v(true, egui::Modifiers::NONE)]);
        frame_with(&mut app, &ctx, vec![v(false, egui::Modifiers::NONE)]);
        assert_eq!(app.pending.len(), 3, "a plain V is typing");
    }

    #[test]
    fn switching_chats_cannot_send_another_chats_pasted_images_on_enter() {
        use crate::model::Action;
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        app.backend.record_demo_commands();
        let first = SAMPLES[0].id.to_owned();
        let second = SAMPLES[1].id.to_owned();
        for color in [40, 80] {
            app.actions.push(Action::PasteImage {
                width: 2,
                height: 2,
                rgba: vec![color; 16],
            });
        }
        render(&mut app, &ctx);
        app.composer = "Pictures for the first chat".into();
        app.actions.push(Action::OpenChat(second));
        render(&mut app, &ctx);
        assert!(app.pending.is_empty());
        assert!(app.composer.is_empty());
        app.backend.take_demo_commands();
        frame_with(
            &mut app,
            &ctx,
            vec![key(egui::Key::Enter, egui::Modifiers::NONE)],
        );
        assert!(
            !app.backend
                .take_demo_commands()
                .iter()
                .any(|command| matches!(
                    command,
                    crate::backend::Command::SendImage { .. }
                        | crate::backend::Command::SendFiles { .. }
                        | crate::backend::Command::SendText { .. }
                ))
        );
        app.actions.push(Action::OpenChat(first.clone()));
        render(&mut app, &ctx);
        assert_eq!(app.pending.len(), 2);
        assert_eq!(app.composer, "Pictures for the first chat");
        app.backend.take_demo_commands();
        frame_with(
            &mut app,
            &ctx,
            vec![key(egui::Key::Enter, egui::Modifiers::NONE)],
        );
        let commands = app.backend.take_demo_commands();
        let images: Vec<_> = commands
            .iter()
            .filter_map(|command| match command {
                crate::backend::Command::SendImage { chat, caption, .. } => Some((chat, caption)),
                _ => None,
            })
            .collect();
        assert_eq!(images.len(), 2);
        assert!(images.iter().all(|(chat, _)| *chat == &first));
        assert_eq!(images[0].1.as_deref(), Some("Pictures for the first chat"));
        assert!(images[1].1.is_none());
        assert!(app.pending.is_empty());
    }

    #[test]
    fn a_pasted_picture_waits_for_its_caption() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        let chat = sample_ids()[0].to_owned();
        let before = app.conversations[&chat].messages.len();
        app.actions.push(crate::model::Action::PasteImage {
            width: 2,
            height: 2,
            rgba: vec![200; 16],
        });
        render(&mut app, &ctx);
        assert_eq!(app.pending.len(), 1, "staged, not sent");
        assert_eq!(app.conversations[&chat].messages.len(), before);
        app.actions.push(crate::model::Action::SendPending {
            account: app.account().id.clone(),
            chat: chat.clone(),
            caption: "look".into(),
            mentions: Vec::new(),
        });
        render(&mut app, &ctx);
        assert!(app.pending.is_empty(), "sent with the caption");
    }

    #[test]
    fn widths_probe() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        render(&mut app, &ctx);
        let chat = sample_ids()[0].to_owned();
        for id in ["ada-doc", "you-voice", "ada-reply", "ada-link", "ada-photo"] {
            let key = crate::ui::conversation::bubble_id(&chat, id).with("rect");
            if let Some(rect) = ctx.data(|data| data.get_temp::<egui::Rect>(key)) {
                eprintln!(
                    "{id}: {:.0} wide, {:.0}..{:.0}",
                    rect.width(),
                    rect.left(),
                    rect.right()
                );
            }
            let card = crate::ui::conversation::bubble_id(&chat, id).with("card");
            if let Some(rect) = ctx.data(|data| data.get_temp::<egui::Rect>(card)) {
                eprintln!(
                    "  card: {:.0} wide, {:.0}..{:.0}",
                    rect.width(),
                    rect.left(),
                    rect.right()
                );
            }
            for kind in ["quote", "preview"] {
                let key = crate::ui::conversation::bubble_id(&chat, id).with(kind);
                if let Some(rect) = ctx.data(|data| data.get_temp::<egui::Rect>(key)) {
                    eprintln!(
                        "  {kind}: {:.0} wide, {:.0}..{:.0}",
                        rect.width(),
                        rect.left(),
                        rect.right()
                    );
                }
            }
            let body = crate::ui::conversation::bubble_id(&chat, id).with("body");
            if let Some(rect) = ctx.data(|data| data.get_temp::<egui::Rect>(body)) {
                eprintln!(
                    "  body: {:.0} wide, {:.0}..{:.0}",
                    rect.width(),
                    rect.left(),
                    rect.right()
                );
            }
        }
    }

    /// Message text can be selected and copied.
    #[test]
    fn message_text_can_be_swept_and_copied() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        render(&mut app, &ctx);
        let chat = sample_ids()[0].to_owned();
        // Use a currently visible message body.
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1180.0, 780.0));
        let rect = ["ada-format", "ada-link", "ada-reply", "ada-tall"]
            .iter()
            .find_map(|id| {
                let key = crate::ui::conversation::bubble_id(&chat, id).with("body");
                ctx.data(|data| data.get_temp::<egui::Rect>(key))
                    .filter(|rect| screen.contains_rect(*rect))
            })
            .expect("a text body on screen");
        let from = egui::pos2(rect.left() + 2.0, rect.center().y);
        let to = egui::pos2(rect.center().x, rect.center().y);
        let press = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let mut copied = None;
        for events in [
            vec![egui::Event::PointerMoved(from), press(from, true)],
            vec![egui::Event::PointerMoved(to)],
            vec![press(to, false)],
            vec![egui::Event::Copy],
            vec![],
        ] {
            let input = egui::RawInput {
                screen_rect: Some(screen),
                events,
                ..Default::default()
            };
            let mut output = ctx.run_ui(input, |ui| {
                let ctx = ui.ctx().clone();
                app.background_frame(&ctx);
                app.frame_ui(ui);
            });
            output.textures_delta.clear();
            for command in output.platform_output.commands {
                if let egui::OutputCommand::CopyText(text) = command {
                    copied = Some(text);
                }
            }
        }
        let copied = copied.expect("the sweep put text on the clipboard");
        assert!(!copied.trim().is_empty(), "{copied:?}");
    }

    /// Seeding a selection turns on full layout for the virtualized row
    /// list, renumbering every positional widget id under the pressed row.
    /// The press's own row must stay addressable or egui drops the
    /// selection it just created.
    #[test]
    fn message_text_selects_with_virtualized_rows() {
        let mut app = app();
        let chat = sample_ids()[0].to_owned();
        // A history taller than the nearby-row margin puts the earliest
        // messages in placeholder rows, so the first selection flips the
        // layout mode while the drag is already under way.
        app.conversations.get_mut(&chat).unwrap().messages = (0..400)
            .map(|i| {
                message(
                    &chat,
                    &format!("m{i:03}"),
                    i % 3 == 0,
                    1_700_000_000 + i64::from(i),
                    Content::text(format!("message number {i}")),
                )
            })
            .collect();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        render(&mut app, &ctx);
        // The end-pinned list shows the newest message: press it, sweep
        // across the text, and copy the result.
        let key = crate::ui::conversation::bubble_id(&chat, "m399").with("body");
        let body = ctx
            .data(|data| data.get_temp::<egui::Rect>(key))
            .expect("the newest message is on screen");
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1180.0, 780.0));
        assert!(screen.contains_rect(body), "m399 body on screen: {body:?}");
        let from = egui::pos2(body.left() + 2.0, body.center().y);
        let to = egui::pos2(body.center().x, body.center().y);
        let press = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let mut copied = None;
        for events in [
            vec![egui::Event::PointerMoved(from), press(from, true)],
            vec![egui::Event::PointerMoved(to)],
            vec![egui::Event::PointerMoved(to)],
            vec![press(to, false)],
            vec![egui::Event::Copy],
            vec![],
        ] {
            let input = egui::RawInput {
                screen_rect: Some(screen),
                events,
                ..Default::default()
            };
            let mut output = ctx.run_ui(input, |ui| {
                let ctx = ui.ctx().clone();
                app.background_frame(&ctx);
                app.frame_ui(ui);
            });
            output.textures_delta.clear();
            for command in output.platform_output.commands {
                if let egui::OutputCommand::CopyText(text) = command {
                    copied = Some(text);
                }
            }
        }
        let copied = copied.expect("the sweep put text on the clipboard");
        assert!(copied.contains("number 39"), "{copied:?}");
        // The sweep stays inside one short line. A selection that silently
        // re-anchors to another row copies every row it lands across.
        assert!(!copied.contains('\n'), "{copied:?}");
    }

    /// A sweep across messages in a long history copies each of them (#269),
    /// and a row the list stopped laying out keeps no body rect, so the
    /// selection tests never aim at where it used to be.
    #[test]
    fn a_copy_across_messages_survives_rows_skipped_above() {
        let mut app = app();
        let chat = sample_ids()[0].to_owned();
        app.conversations.get_mut(&chat).unwrap().messages = (0..400)
            .map(|i| {
                message(
                    &chat,
                    &format!("m{i:03}"),
                    i % 3 == 0,
                    1_700_000_000 + i64::from(i),
                    Content::text(format!("message number {i}")),
                )
            })
            .collect();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        // The rows near the end settle over the first frames of a long chat's
        // first showing; the sweep starts once they have.
        render(&mut app, &ctx);
        render(&mut app, &ctx);
        render(&mut app, &ctx);
        let body = |id: &str| {
            let key = crate::ui::conversation::bubble_id(&chat, id).with("body");
            ctx.data(|data| data.get_temp::<egui::Rect>(key))
        };
        assert!(body("m000").is_none(), "{:?}", body("m000"));
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1180.0, 780.0));
        let first = body("m397").expect("m397 on screen");
        let last = body("m399").expect("m399 on screen");
        assert!(screen.contains_rect(first) && screen.contains_rect(last));
        let from = egui::pos2(first.left() + 2.0, first.center().y);
        let to = last.center();
        let press = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let mut copied = None;
        for events in [
            vec![egui::Event::PointerMoved(from), press(from, true)],
            vec![egui::Event::PointerMoved(to)],
            vec![egui::Event::PointerMoved(to)],
            vec![press(to, false)],
            vec![egui::Event::Copy],
            vec![],
        ] {
            let input = egui::RawInput {
                screen_rect: Some(screen),
                events,
                ..Default::default()
            };
            let mut output = ctx.run_ui(input, |ui| {
                let ctx = ui.ctx().clone();
                app.background_frame(&ctx);
                app.frame_ui(ui);
            });
            output.textures_delta.clear();
            for command in output.platform_output.commands {
                if let egui::OutputCommand::CopyText(text) = command {
                    copied = Some(text);
                }
            }
        }
        let copied = copied.expect("the sweep put text on the clipboard");
        for number in ["number 397", "number 398", "number 39"] {
            assert!(copied.contains(number), "{number}: {copied:?}");
        }
        assert_eq!(copied.matches("] ").count(), 3, "{copied:?}");
    }

    #[test]
    fn message_text_selection_starts_in_the_bubble_padding() {
        let mut app = app();
        let chat = sample_ids()[0].to_owned();
        app.conversations.get_mut(&chat).unwrap().messages = vec![message(
            &chat,
            "padding-target",
            false,
            1_700_000_000,
            Content::text("A forgiving selection target"),
        )];
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        render(&mut app, &ctx);

        let id = crate::ui::conversation::bubble_id(&chat, "padding-target");
        let body = ctx
            .data(|data| data.get_temp::<egui::Rect>(id.with("body")))
            .expect("the message body is on screen");
        let target = ctx
            .read_response(id.with("body-text"))
            .expect("the selection target is on screen")
            .rect;
        let bubble = ctx
            .data(|data| data.get_temp::<egui::Rect>(id.with("rect")))
            .expect("the message bubble is on screen");
        assert!(target.contains(bubble.center()));
        let from = egui::pos2(target.right() - 2.0, target.bottom() - 2.0);
        let to = body.center();
        assert!(target.contains(from));
        assert!(
            !body.contains(from),
            "the sweep starts outside the text: {from:?}"
        );

        let press = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1180.0, 780.0));
        let mut copied = None;
        for events in [
            vec![egui::Event::PointerMoved(from), press(from, true)],
            vec![egui::Event::PointerMoved(to)],
            vec![press(to, false)],
            vec![egui::Event::Copy],
            vec![],
        ] {
            let input = egui::RawInput {
                screen_rect: Some(screen),
                events,
                ..Default::default()
            };
            let mut output = ctx.run_ui(input, |ui| {
                let ctx = ui.ctx().clone();
                app.background_frame(&ctx);
                app.frame_ui(ui);
            });
            output.textures_delta.clear();
            for command in output.platform_output.commands {
                if let egui::OutputCommand::CopyText(text) = command {
                    copied = Some(text);
                }
            }
        }

        let copied = copied.expect("a sweep from the padding copies text");
        assert!(!copied.trim().is_empty(), "{copied:?}");
        assert!("A forgiving selection target".contains(copied.trim()));
    }

    /// Link navigation uses the painted text bounds, while padding remains
    /// available for selection and reply without opening a browser.
    #[test]
    fn expanded_selection_padding_does_not_activate_message_links() {
        let mut app = app();
        let chat = SAMPLES[0].id.to_owned();
        let row = message(
            &chat,
            "link-padding",
            false,
            1_700_000_000,
            Content::text("https://example.com/"),
        );
        app.conversations.get_mut(&chat).unwrap().messages = vec![row];
        let ctx = egui::Context::default();
        app.attach(&ctx);
        app.backend.record_demo_commands();
        for _ in 0..4 {
            render(&mut app, &ctx);
        }
        let id = crate::ui::conversation::bubble_id(&chat, "link-padding");
        let body = ctx
            .data(|data| data.get_temp::<egui::Rect>(id.with("body")))
            .unwrap();
        let bubble = ctx
            .data(|data| data.get_temp::<egui::Rect>(id.with("rect")))
            .unwrap();
        let padding = egui::pos2(bubble.left() + 2.0, body.center().y);
        assert!(!body.contains(padding));
        let press = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let mut opened = Vec::new();
        for pos in [padding, body.center()] {
            for events in [
                vec![egui::Event::PointerMoved(pos), press(pos, true)],
                vec![press(pos, false)],
            ] {
                let input = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1180.0, 780.0),
                    )),
                    events,
                    ..Default::default()
                };
                let mut output = ctx.run_ui(input, |ui| {
                    app.background_frame(ui.ctx());
                    app.frame_ui(ui);
                });
                output.textures_delta.clear();
                for command in output.platform_output.commands {
                    if let egui::OutputCommand::OpenUrl(url) = command {
                        opened.push(url.url);
                    }
                }
            }
            if pos == padding {
                assert!(opened.is_empty(), "padding cannot open the link");
            }
        }
        assert_eq!(
            opened,
            ["https://example.com/"],
            "the painted link still opens"
        );
    }

    #[test]
    fn expanded_text_selection_preserves_quote_and_preview_clicks() {
        for kind in ["quote", "preview"] {
            let mut app = app();
            let chat = sample_ids()[0].to_owned();
            let mut row = message(
                &chat,
                "cards",
                false,
                1_700_000_001,
                Content::Text {
                    text: "Text below a card".into(),
                    preview: (kind == "preview").then(|| LinkPreview {
                        url: "https://example.com/selection-fixture".into(),
                        title: Some("Fixture preview".into()),
                        description: None,
                    }),
                },
            );
            if kind == "quote" {
                row.quoted = Some(Quoted {
                    labelled: None,
                    chat: None,
                    id: "original".into(),
                    sender: chat.clone(),
                    sender_name: Some("Fixture".into()),
                    summary: "Original text".into(),
                    mentions: Vec::new(),
                });
            }
            app.conversations.get_mut(&chat).unwrap().messages = vec![
                message(
                    &chat,
                    "original",
                    false,
                    1_700_000_000,
                    Content::text("Original text"),
                ),
                row,
            ];
            let ctx = egui::Context::default();
            app.attach(&ctx);
            for _ in 0..3 {
                render(&mut app, &ctx);
            }
            let id = crate::ui::conversation::bubble_id(&chat, "cards");
            let card = ctx
                .data(|data| data.get_temp::<egui::Rect>(id.with(kind)))
                .unwrap();
            let target = ctx.read_response(id.with("body-text")).unwrap().rect;
            assert!(
                !target.contains(card.center()),
                "{kind}: {target:?} overlaps {card:?}"
            );
            let pos = card.center();
            let mut opened = Vec::new();
            for pressed in [true, false] {
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(1180.0, 780.0),
                        )),
                        events: vec![
                            egui::Event::PointerMoved(pos),
                            egui::Event::PointerButton {
                                pos,
                                button: egui::PointerButton::Primary,
                                pressed,
                                modifiers: egui::Modifiers::NONE,
                            },
                        ],
                        ..Default::default()
                    },
                    |ui| {
                        let ctx = ui.ctx().clone();
                        app.background_frame(&ctx);
                        app.frame_ui(ui);
                    },
                );
                output.textures_delta.clear();
                opened.extend(
                    output.platform_output.commands.into_iter().filter_map(
                        |command| match command {
                            egui::OutputCommand::OpenUrl(url) => Some(url.url),
                            _ => None,
                        },
                    ),
                );
            }
            if kind == "quote" {
                assert_eq!(app.scroll_anchor.as_deref(), Some("original"));
            } else {
                assert_eq!(opened, ["https://example.com/selection-fixture"]);
            }
        }
    }

    #[test]
    fn a_drag_selects_short_messages_on_opposite_sides_of_the_chat() {
        let mut app = app();
        let chat = sample_ids()[0].to_owned();
        app.conversations.get_mut(&chat).unwrap().messages = vec![
            message(&chat, "left", false, 100, Content::text("Left first")),
            message(&chat, "right", true, 200, Content::text("Right second")),
            message(&chat, "last", false, 300, Content::text("Left last")),
        ];
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        render(&mut app, &ctx);
        let body = |id| {
            ctx.data(|data| {
                data.get_temp::<egui::Rect>(
                    crate::ui::conversation::bubble_id(&chat, id).with("body"),
                )
            })
            .unwrap()
        };
        let left = body("left");
        let right = body("right");
        assert!(
            left.right() < right.left(),
            "the bubbles must not overlap horizontally"
        );
        let from = egui::pos2(left.left() + 1.0, left.center().y);
        let below = egui::pos2(750.0, 1100.0);
        let press = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let mut copied = String::new();
        for events in [
            vec![egui::Event::PointerMoved(from), press(from, true)],
            vec![egui::Event::PointerMoved(below), egui::Event::PointerGone],
            vec![egui::Event::PointerMoved(below)],
            vec![press(below, false)],
            vec![egui::Event::Copy],
            vec![],
        ] {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1180.0, 780.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    app.background_frame(&ui.ctx().clone());
                    app.frame_ui(ui);
                },
            );
            output.textures_delta.clear();
            for command in output.platform_output.commands {
                if let egui::OutputCommand::CopyText(text) = command {
                    copied = text;
                }
            }
        }
        assert_eq!(copied.matches("] ").count(), 3, "{copied:?}");
        for text in ["Left first", "Right second", "Left last"] {
            assert!(copied.contains(text), "{copied:?}");
        }
    }

    /// Opens a chat with one text and one deleted message, double-clicks the
    /// point chosen from the named bubble's rect and its body, and returns the
    /// message being replied to.
    fn reply_after_double_click(
        id: &str,
        point: impl Fn(egui::Rect, Option<egui::Rect>) -> egui::Pos2,
    ) -> Option<String> {
        let mut app = app();
        let chat = sample_ids()[0].to_owned();
        app.conversations.get_mut(&chat).unwrap().messages = vec![
            message(&chat, "text", false, 100, Content::text("Double-click me")),
            message(&chat, "gone", false, 200, Content::Revoked),
        ];
        let ctx = egui::Context::default();
        app.attach(&ctx);
        for _ in 0..3 {
            render(&mut app, &ctx);
        }
        let key = crate::ui::conversation::bubble_id(&chat, id);
        let rect = ctx
            .data(|data| data.get_temp::<egui::Rect>(key.with("rect")))
            .expect("the bubble is on screen");
        let body = ctx.data(|data| data.get_temp::<egui::Rect>(key.with("body")));
        let pos = point(rect, body);
        let press = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        for events in [
            vec![egui::Event::PointerMoved(pos), press(true)],
            vec![press(false)],
            vec![press(true)],
            vec![press(false)],
            vec![],
        ] {
            frame_with(&mut app, &ctx, events);
        }
        app.reply_to.map(|target| target.id)
    }

    #[test]
    fn a_double_click_on_the_bubble_padding_replies() {
        let reply =
            reply_after_double_click("text", |rect, _| rect.left_center() + egui::vec2(4.0, 0.0));
        assert_eq!(reply.as_deref(), Some("text"));
    }

    #[test]
    fn a_double_click_beside_the_bubble_replies() {
        let reply = reply_after_double_click("text", |rect, _| {
            rect.right_center() + egui::vec2(120.0, 0.0)
        });
        assert_eq!(reply.as_deref(), Some("text"));
    }

    #[test]
    fn a_double_click_on_the_text_selects_the_word_without_replying() {
        let reply = reply_after_double_click("text", |_, body| {
            let body = body.expect("a text body");
            body.left_center() + egui::vec2(12.0, 0.0)
        });
        assert_eq!(reply, None);
    }

    #[test]
    fn a_double_click_on_a_deleted_message_does_not_reply() {
        let reply = reply_after_double_click("gone", |rect, _| {
            rect.right_center() + egui::vec2(120.0, 0.0)
        });
        assert_eq!(reply, None);
    }

    /// Selection continues and scrolls after the pointer leaves the window.
    #[test]
    fn a_drag_out_of_the_window_keeps_selecting() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        render(&mut app, &ctx);
        let chat = sample_ids()[0].to_owned();
        // Scroll away from the end before extending the selection.
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1180.0, 780.0));
        let ids: Vec<String> = app.conversations[&chat]
            .messages
            .iter()
            .map(|message| message.id.clone())
            .collect();
        let body_of = |ctx: &egui::Context, id: &str| {
            let key = crate::ui::conversation::bubble_id(&chat, id).with("body");
            ctx.data(|data| data.get_temp::<egui::Rect>(key))
                .filter(|rect| screen.contains_rect(*rect))
        };
        let sweepable = |content: &crate::model::Content| -> Option<String> {
            match content {
                crate::model::Content::Text { text, .. } => Some(crate::markup::plain(text, &[])),
                crate::model::Content::Image {
                    caption: Some(caption),
                    ..
                } => Some(crate::markup::plain(caption, &[])),
                _ => None,
            }
        };
        let (start, start_text) = ids
            .iter()
            .find_map(|id| {
                let rect = body_of(&ctx, id)?;
                let text = sweepable(&app.conversations[&chat].message(id)?.content)?;
                Some((rect, text))
            })
            .expect("a swept text body on screen");
        let from = egui::pos2(start.left() + 4.0, start.center().y);
        let press = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        // Simulate leaving the window with a PointerGone event.
        let centre = app
            .selection_view
            .lock()
            .expect("the view rect")
            .expect("the conversation was drawn")
            .center()
            .x;
        let below = egui::pos2(centre, 1100.0);
        let mut frames: Vec<Vec<egui::Event>> = vec![
            vec![egui::Event::PointerMoved(from), press(from, true)],
            vec![egui::Event::PointerMoved(below), egui::Event::PointerGone],
        ];
        frames.extend((0..14).map(|_| vec![egui::Event::PointerMoved(below)]));
        frames.push(vec![press(below, false)]);
        frames.push(vec![egui::Event::Copy]);
        frames.push(vec![]);
        let mut copied = None;
        for events in frames {
            let input = egui::RawInput {
                screen_rect: Some(screen),
                events,
                ..Default::default()
            };
            let mut output = ctx.run_ui(input, |ui| {
                let ctx = ui.ctx().clone();
                app.background_frame(&ctx);
                app.frame_ui(ui);
            });
            output.textures_delta.clear();
            for command in output.platform_output.commands {
                if let egui::OutputCommand::CopyText(text) = command {
                    copied = Some(text);
                }
            }
        }
        let copied = copied.expect("the drag still put text on the clipboard");
        assert!(
            copied.matches("] ").count() >= 2,
            "the selection should span messages: {copied:?}"
        );
        // The copied text must include the off-screen selection start.
        let opening: String = start_text.chars().take(12).collect();
        assert!(
            copied.contains(opening.trim_end()),
            "the scrolled-away start should be copied: {copied:?}"
        );
    }

    /// Dragging near the top scrolls up from a bottom-pinned list.
    #[test]
    fn a_held_drag_at_the_top_edge_scrolls_the_list_up() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        render(&mut app, &ctx);
        let chat = sample_ids()[0].to_owned();
        let ids: Vec<String> = app.conversations[&chat]
            .messages
            .iter()
            .map(|message| message.id.clone())
            .collect();
        let rect_of = |ctx: &egui::Context, id: &str| {
            let key = crate::ui::conversation::bubble_id(&chat, id).with("rect");
            ctx.data(|data| data.get_temp::<egui::Rect>(key))
        };
        let before: Vec<(String, f32)> = ids
            .iter()
            .filter_map(|id| rect_of(&ctx, id).map(|rect| (id.clone(), rect.top())))
            .collect();
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1180.0, 780.0));
        // Use the frame's stored message-view rect because platform insets vary.
        let view = app
            .selection_view
            .lock()
            .expect("the view rect")
            .expect("the conversation was drawn");
        // Press lower down, then drag into the top edge, as when selecting.
        let start = egui::pos2(view.center().x, view.top() + 80.0);
        let hold = egui::pos2(view.center().x, view.top() + 10.0);
        let press = egui::Event::PointerButton {
            pos: start,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        };
        let mut frames: Vec<Vec<egui::Event>> = vec![vec![egui::Event::PointerMoved(start), press]];
        frames.extend((0..12).map(|_| vec![egui::Event::PointerMoved(hold)]));
        for events in frames {
            let input = egui::RawInput {
                screen_rect: Some(screen),
                events,
                ..Default::default()
            };
            let mut output = ctx.run_ui(input, |ui| {
                let ctx = ui.ctx().clone();
                app.background_frame(&ctx);
                app.frame_ui(ui);
            });
            output.textures_delta.clear();
        }
        let moved = before
            .iter()
            .filter_map(|(id, top)| rect_of(&ctx, id).map(|rect| rect.top() - top))
            .fold(f32::MIN, f32::max);
        assert!(
            moved > 20.0,
            "the list should have scrolled up; best {moved}"
        );
        assert!(!app.scroll_to_bottom, "heading up releases the pin");
    }

    /// A click held still near the top edge does not scroll.
    #[test]
    fn a_click_held_at_the_top_edge_does_not_scroll() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        render(&mut app, &ctx);
        let chat = sample_ids()[0].to_owned();
        let ids: Vec<String> = app.conversations[&chat]
            .messages
            .iter()
            .map(|message| message.id.clone())
            .collect();
        let rect_of = |ctx: &egui::Context, id: &str| {
            let key = crate::ui::conversation::bubble_id(&chat, id).with("rect");
            ctx.data(|data| data.get_temp::<egui::Rect>(key))
        };
        let before: Vec<(String, f32)> = ids
            .iter()
            .filter_map(|id| rect_of(&ctx, id).map(|rect| (id.clone(), rect.top())))
            .collect();
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1180.0, 780.0));
        // Use the frame's stored message-view rect because platform insets vary.
        let view = app
            .selection_view
            .lock()
            .expect("the view rect")
            .expect("the conversation was drawn");
        let start = egui::pos2(view.center().x, view.top() + 10.0);
        let hold = egui::pos2(view.center().x, view.top() + 10.0);
        let press = egui::Event::PointerButton {
            pos: start,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        };
        let mut frames: Vec<Vec<egui::Event>> = vec![vec![egui::Event::PointerMoved(start), press]];
        frames.extend((0..12).map(|_| vec![egui::Event::PointerMoved(hold)]));
        for events in frames {
            let input = egui::RawInput {
                screen_rect: Some(screen),
                events,
                ..Default::default()
            };
            let mut output = ctx.run_ui(input, |ui| {
                let ctx = ui.ctx().clone();
                app.background_frame(&ctx);
                app.frame_ui(ui);
            });
            output.textures_delta.clear();
        }
        let moved = before
            .iter()
            .filter_map(|(id, top)| rect_of(&ctx, id).map(|rect| rect.top() - top))
            .fold(f32::MIN, f32::max);
        assert!(
            moved.abs() < 1.0,
            "a still click should not scroll; moved {moved}"
        );
    }

    /// Selection scrolls only near a view edge.
    #[test]
    fn a_drag_at_the_edge_scrolls_and_in_the_middle_does_not() {
        use crate::ui::conversation::edge_scroll;
        assert_eq!(edge_scroll(300.0, 100.0, 700.0), 0.0);
        assert!(edge_scroll(110.0, 100.0, 700.0) < 0.0, "near the top: up");
        assert!(
            edge_scroll(690.0, 100.0, 700.0) > 0.0,
            "near the bottom: down"
        );
        assert!(
            edge_scroll(105.0, 100.0, 700.0) < edge_scroll(130.0, 100.0, 700.0),
            "closer pulls harder"
        );
        assert_eq!(
            edge_scroll(-500.0, 100.0, 700.0),
            edge_scroll(20.0, 100.0, 700.0),
            "the pull tops out past the edge"
        );
    }

    /// Multi-message copies include WhatsApp-style timestamps and senders.
    #[test]
    fn a_copy_across_messages_names_each_writer() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        // Let asynchronous sample-image decoding finish before dragging.
        std::thread::sleep(std::time::Duration::from_millis(300));
        render(&mut app, &ctx);
        let chat = sample_ids()[0].to_owned();
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1180.0, 780.0));
        let ids: Vec<String> = app.conversations[&chat]
            .messages
            .iter()
            .map(|message| message.id.clone())
            .collect();
        let mut bodies: Vec<egui::Rect> = ids
            .iter()
            .filter_map(|id| {
                let key = crate::ui::conversation::bubble_id(&chat, id).with("body");
                ctx.data(|data| data.get_temp::<egui::Rect>(key))
                    .filter(|rect| screen.contains_rect(*rect))
            })
            .collect();
        bodies.sort_by(|a, b| a.top().total_cmp(&b.top()));
        assert!(bodies.len() >= 2, "two text bodies on screen");
        let from = egui::pos2(bodies[0].left() + 2.0, bodies[0].center().y);
        let to = bodies[1].center();
        let press = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let mut copied = None;
        for events in [
            vec![egui::Event::PointerMoved(from), press(from, true)],
            vec![egui::Event::PointerMoved(to)],
            vec![press(to, false)],
            vec![egui::Event::Copy],
            vec![],
        ] {
            let input = egui::RawInput {
                screen_rect: Some(screen),
                events,
                ..Default::default()
            };
            let mut output = ctx.run_ui(input, |ui| {
                let ctx = ui.ctx().clone();
                app.background_frame(&ctx);
                app.frame_ui(ui);
            });
            output.textures_delta.clear();
            for command in output.platform_output.commands {
                if let egui::OutputCommand::CopyText(text) = command {
                    copied = Some(text);
                }
            }
        }
        let copied = copied.expect("the sweep put text on the clipboard");
        assert!(copied.starts_with('['), "{copied:?}");
        assert!(copied.matches("] ").count() >= 2, "{copied:?}");
        assert!(copied.lines().count() >= 2, "{copied:?}");
    }

    /// A failed message must say so in words, to screen readers as well as on
    /// screen, not only with a red icon.
    #[test]
    fn a_failed_message_is_labelled_for_screen_readers() {
        let mut app = app();
        apply_flags(&mut app, Some("failed"));
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        app.attach(&ctx);
        render(&mut app, &ctx);
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1180.0, 780.0),
                )),
                ..Default::default()
            },
            |ui| {
                let ctx = ui.ctx().clone();
                app.background_frame(&ctx);
                app.frame_ui(ui);
            },
        );
        output.textures_delta.clear();
        let tree = output
            .platform_output
            .accesskit_update
            .expect("accessibility tree");
        let hints = tree
            .nodes
            .iter()
            .filter(|(_, node)| {
                node.label()
                    .or_else(|| node.value())
                    .is_some_and(|label| label.starts_with("This message could not be sent"))
            })
            .count();
        assert_eq!(hints, 1, "exactly the failed message carries the hint");
    }

    /// Every control the line under the header draws is reachable without a
    /// pointer: the row opens its message and the pin beside it steps to the
    /// next pin, and both are labelled buttons.
    #[test]
    fn the_pinned_line_is_one_button_that_names_its_place() {
        let mut app = app();
        apply_flags(&mut app, Some("pinned"));
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        app.attach(&ctx);
        render(&mut app, &ctx);
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1180.0, 780.0),
                )),
                ..Default::default()
            },
            |ui| {
                let ctx = ui.ctx().clone();
                app.background_frame(&ctx);
                app.frame_ui(ui);
            },
        );
        output.textures_delta.clear();
        let tree = output
            .platform_output
            .accesskit_update
            .expect("accessibility tree");
        let labels: Vec<&str> = tree
            .nodes
            .iter()
            .filter_map(|(_, node)| node.label())
            .collect();
        let lines: Vec<&&str> = labels
            .iter()
            .filter(|label| label.starts_with("Pinned message "))
            .collect();
        assert_eq!(lines.len(), 1, "one line for all the pins: {labels:?}");
        assert!(
            lines[0].starts_with("Pinned message 1 of 3: "),
            "it names the newest pin and the count: {lines:?}"
        );
    }

    /// Voice controls keep their width and order in right-aligned bubbles.
    #[test]
    fn an_own_voice_message_keeps_its_bubble_narrow() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        render(&mut app, &ctx);
        let chat = sample_ids()[0].to_owned();
        let id = crate::ui::conversation::bubble_id(&chat, "you-voice").with("rect");
        let rect = ctx
            .data(|data| data.get_temp::<egui::Rect>(id))
            .expect("the bubble was drawn");
        assert!(
            (240.0..=345.0).contains(&rect.width()),
            "{} wide",
            rect.width()
        );
    }

    /// Whether AccessKit reports the button with this label as disabled.
    fn button_disabled(app: &mut App, ctx: &egui::Context, label: &str) -> bool {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1180.0, 780.0),
                )),
                ..Default::default()
            },
            |ui| {
                let ctx = ui.ctx().clone();
                app.background_frame(&ctx);
                app.frame_ui(ui);
            },
        );
        output.textures_delta.clear();
        let tree = output
            .platform_output
            .accesskit_update
            .expect("accessibility tree");
        tree.nodes
            .iter()
            // The composer field is also labelled "Message"; match buttons only.
            .find(|(_, node)| {
                node.role() == egui::accesskit::Role::Button && node.label() == Some(label)
            })
            .unwrap_or_else(|| panic!("no {label} button"))
            .1
            .is_disabled()
    }

    /// A button that cannot act yet must say so, on screen and to screen
    /// readers, instead of looking like any other button and ignoring clicks.
    #[test]
    fn number_dialogs_disable_their_actions_until_the_number_is_complete() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();

        let mut app = app();
        apply_flags(&mut app, Some("phone"));
        app.attach(&ctx);
        render(&mut app, &ctx);
        assert!(button_disabled(&mut app, &ctx, "Get a code"));
        app.pair_phone = "15551234567".into();
        render(&mut app, &ctx);
        assert!(!button_disabled(&mut app, &ctx, "Get a code"));

        let mut app = self::app();
        apply_flags(&mut app, Some("new-contact"));
        app.attach(&ctx);
        render(&mut app, &ctx);
        assert!(button_disabled(&mut app, &ctx, "Message"));
        assert!(button_disabled(&mut app, &ctx, "Save contact"));
        assert!(!button_disabled(&mut app, &ctx, "Cancel"));
        app.new_contact_phone = "15551234567".into();
        render(&mut app, &ctx);
        assert!(!button_disabled(&mut app, &ctx, "Message"));
        assert!(!button_disabled(&mut app, &ctx, "Save contact"));
    }

    /// The chat list's menu offers Mute as one submenu, with WhatsApp's
    /// choices of how long; a muted chat offers Unmute alone.
    #[test]
    fn the_chat_menu_mutes_through_a_submenu() {
        fn texts(shape: &egui::Shape, out: &mut Vec<(String, egui::Rect)>) {
            match shape {
                egui::Shape::Text(text) => out.push((
                    text.galley.text().to_owned(),
                    text.galley.rect.translate(text.pos.to_vec2()),
                )),
                egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| texts(shape, out)),
                _ => {}
            }
        }
        let drawn = |shapes: &[egui::epaint::ClippedShape]| {
            let mut out = Vec::new();
            for clipped in shapes {
                texts(&clipped.shape, &mut out);
            }
            out
        };
        let find = |shapes: &[egui::epaint::ClippedShape], label: &str| {
            drawn(shapes)
                .into_iter()
                .find(|(text, _)| text == label)
                .map(|(_, rect)| rect)
        };
        let click = |pos| {
            [true, false].map(|pressed| egui::Event::PointerButton {
                pos,
                pressed,
                button: egui::PointerButton::Primary,
                modifiers: egui::Modifiers::NONE,
            })
        };
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        let chat = sample_ids()[0].to_owned();
        app.open_chat_menu = Some(chat.clone());
        let mut shapes = Vec::new();
        for _ in 0..3 {
            shapes = frame_sized(&mut app, &ctx, 780.0, Vec::new());
        }
        for old in [
            "Mute for 8 hours",
            "Mute for a week",
            "Mute indefinitely",
            "Unmute",
        ] {
            assert!(find(&shapes, old).is_none(), "{old} is not a menu row");
        }
        let mute = find(&shapes, "Mute").expect("the menu has a Mute row");
        for _ in 0..4 {
            shapes = frame_sized(
                &mut app,
                &ctx,
                780.0,
                vec![egui::Event::PointerMoved(mute.center())],
            );
        }
        for choice in ["8 hours", "1 week"] {
            assert!(
                find(&shapes, choice).is_some(),
                "the submenu offers {choice}"
            );
        }
        let always = find(&shapes, "Always").expect("the submenu offers Always");
        app.actions.clear();
        let [press, release] = click(always.center());
        frame_sized(
            &mut app,
            &ctx,
            780.0,
            vec![egui::Event::PointerMoved(always.center()), press],
        );
        frame_sized(&mut app, &ctx, 780.0, vec![release]);
        let now = crate::util::now();
        assert!(
            app.chat(&chat).expect("chat").muted(now),
            "Always mutes the chat"
        );

        // Muted, the menu offers Unmute alone.
        app.open_chat_menu = Some(chat.clone());
        for _ in 0..3 {
            shapes = frame_sized(&mut app, &ctx, 780.0, Vec::new());
        }
        assert!(
            find(&shapes, "Unmute").is_some(),
            "a muted chat offers Unmute"
        );
        assert!(find(&shapes, "Mute").is_none(), "and no Mute submenu");
    }

    #[test]
    fn muting_a_chat_takes_effect_at_once() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        let chat = sample_ids()[0].to_owned();
        let now = crate::util::now();
        assert!(!app.chat(&chat).expect("chat").muted(now));
        app.actions
            .push(crate::model::Action::SetMuted(chat.clone(), Some(0)));
        render(&mut app, &ctx);
        assert!(app.chat(&chat).expect("chat").muted(now));
        app.actions
            .push(crate::model::Action::SetMuted(chat.clone(), None));
        render(&mut app, &ctx);
        assert!(!app.chat(&chat).expect("chat").muted(now));
    }

    #[test]
    fn editing_puts_the_text_back_and_escape_stops() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        let chat = sample_ids()[0].to_owned();
        let own = app
            .conversations
            .get(&chat)
            .and_then(|conversation| {
                conversation.messages.iter().rev().find(|message| {
                    message.from_me && matches!(message.content, Content::Text { .. })
                })
            })
            .map(|message| message.id.clone())
            .expect("an own text message");
        app.actions.push(crate::model::Action::Edit(own.clone()));
        render(&mut app, &ctx);
        assert_eq!(app.editing.as_deref(), Some(own.as_str()));
        assert!(!app.composer.is_empty());
        let composer_id = egui::Id::new("composer-text");
        let cursor = egui::TextEdit::load_state(&ctx, composer_id)
            .and_then(|state| state.cursor.char_range())
            .map(|range| range.primary.index.0);
        assert_eq!(cursor, Some(app.composer.chars().count()));
        frame_with(
            &mut app,
            &ctx,
            vec![key(egui::Key::Escape, egui::Modifiers::NONE)],
        );
        assert!(app.editing.is_none());
        assert!(app.composer.is_empty());
        let cursor = egui::TextEdit::load_state(&ctx, composer_id)
            .and_then(|state| state.cursor.char_range())
            .map(|range| range.primary.index.0);
        assert_eq!(cursor, Some(0));
    }

    #[test]
    fn arrow_up_in_an_empty_composer_edits_the_previous_own_message() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        let chat = sample_ids()[0].to_owned();
        let expected = app
            .conversations
            .get(&chat)
            .and_then(|conversation| {
                conversation.messages.iter().rev().find_map(|message| {
                    match (&message.from_me, &message.content) {
                        (true, Content::Text { text, .. }) => {
                            Some((message.id.clone(), text.clone()))
                        }
                        _ => None,
                    }
                })
            })
            .expect("an own text message");
        frame_with(
            &mut app,
            &ctx,
            vec![key(egui::Key::ArrowUp, egui::Modifiers::NONE)],
        );
        assert_eq!(app.editing.as_deref(), Some(expected.0.as_str()));
        assert_eq!(app.composer, expected.1);
        let composer_id = egui::Id::new("composer-text");
        let cursor = egui::TextEdit::load_state(&ctx, composer_id)
            .and_then(|state| state.cursor.char_range())
            .map(|range| range.primary.index.0);
        assert_eq!(cursor, Some(app.composer.chars().count()));
    }

    #[test]
    fn arrow_up_leaves_a_non_empty_composer_alone() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        frame_with(&mut app, &ctx, vec![egui::Event::Text("draft".into())]);
        frame_with(
            &mut app,
            &ctx,
            vec![key(egui::Key::ArrowUp, egui::Modifiers::NONE)],
        );
        assert!(app.editing.is_none());
        assert_eq!(app.composer, "draft");
    }

    #[test]
    fn brackets_are_text_in_the_composer_and_switch_chats_with_ctrl_shift() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        let open = app.open_chat.clone().expect("the demo opens a chat");
        // Plain and shifted brackets are text for the focused composer.
        frame_with(
            &mut app,
            &ctx,
            vec![
                key(egui::Key::CloseBracket, egui::Modifiers::NONE),
                egui::Event::Text("]".into()),
                key(egui::Key::CloseCurlyBracket, egui::Modifiers::SHIFT),
                egui::Event::Text("}".into()),
            ],
        );
        assert_eq!(app.composer, "]}");
        assert_eq!(app.open_chat.as_deref(), Some(open.as_str()));
        // With Ctrl+Shift the same key steps to the next chat and the input
        // keeps the focus, as Alt+Down does.
        let next = {
            let visible = app.visible_chats();
            let at = visible
                .iter()
                .position(|chat| chat.id == open)
                .expect("the open chat is listed");
            visible[(at + 1) % visible.len()].id.clone()
        };
        frame_with(
            &mut app,
            &ctx,
            vec![key(
                egui::Key::CloseCurlyBracket,
                egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
            )],
        );
        assert_eq!(app.open_chat.as_deref(), Some(next.as_str()));
        render(&mut app, &ctx);
        assert!(ctx.memory(|memory| memory.has_focus(egui::Id::new("composer-text"))));
    }

    /// Checks that the group context menu opens the sender draft without dispatching a message.
    #[test]
    fn the_group_menu_opens_a_private_reply_without_sending() {
        for light in [false, true] {
            let ctx = egui::Context::default();
            ctx.enable_accesskit();
            let mut app = app();
            apply_flags(
                &mut app,
                Some(if light {
                    "private-reply-menu,light"
                } else {
                    "private-reply-menu"
                }),
            );
            app.attach(&ctx);
            render(&mut app, &ctx);
            app.backend.record_demo_commands();
            let original = app
                .conversations
                .get(SAMPLES[1].id)
                .unwrap()
                .message("group-photo")
                .unwrap()
                .clone();
            let nodes = accessible_nodes(&mut app, &ctx, Vec::new());
            let (_, _, position) = nodes
                .iter()
                .find(|(label, _, _)| label == "Reply privately")
                .expect("the group menu offers private replies");
            let position = *position;
            let press = |pressed| egui::Event::PointerButton {
                pos: position,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            accessible_nodes(
                &mut app,
                &ctx,
                vec![egui::Event::PointerMoved(position), press(true)],
            );
            accessible_nodes(&mut app, &ctx, vec![press(false)]);
            render(&mut app, &ctx);
            assert_eq!(app.open_chat.as_deref(), Some(original.sender.as_str()));
            assert_eq!(
                app.reply_to.as_ref().unwrap().chat.as_deref(),
                Some(SAMPLES[1].id)
            );
            assert!(
                ctx.data(
                    |data| data.get_temp::<egui::Rect>(crate::ui::conversation::reply_strip_id())
                )
                .is_some(),
                "the source-group quote remains visible in the DM"
            );
            assert!(
                !app.backend
                    .take_demo_commands()
                    .iter()
                    .any(|command| matches!(command, crate::backend::Command::SendText { .. })),
                "opening the private reply sends nothing"
            );
        }
    }
    /// Checks that evicting a source page keeps the private quote target rather than silently clearing it.
    #[test]
    fn a_private_reply_keeps_its_quote_when_the_source_page_is_missing() {
        let ctx = egui::Context::default();
        let mut app = app();
        apply_flags(&mut app, Some("private-reply"));
        app.attach(&ctx);
        render(&mut app, &ctx);
        let target = app.reply_to.clone().unwrap();
        app.conversations.remove(target.chat.as_ref().unwrap());
        render(&mut app, &ctx);
        assert_eq!(app.reply_to, Some(target));
    }

    /// A locked group's cached quote must not appear in the direct-chat composer.
    #[test]
    fn a_private_reply_hides_cached_locked_source_content() {
        for theme in [
            "private-reply-locked-source",
            "private-reply-locked-source,light",
        ] {
            let ctx = egui::Context::default();
            let mut app = app();
            apply_flags(&mut app, Some(theme));
            let target = app.reply_to.clone().unwrap();
            let source = target.chat.as_ref().unwrap();
            let original = app
                .conversations
                .get_mut(source)
                .unwrap()
                .messages
                .iter_mut()
                .find(|message| message.id == target.id)
                .unwrap();
            original.content = Content::text("Hidden source fixture sentinel");
            original.sender_name = Some("Hidden sender fixture sentinel".into());
            app.attach(&ctx);
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1180.0, 780.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    let ctx = ui.ctx().clone();
                    app.background_frame(&ctx);
                    app.frame_ui(ui);
                },
            );
            output.textures_delta.clear();
            let mut stack: Vec<_> = output.shapes.iter().map(|shape| &shape.shape).collect();
            let mut texts = Vec::new();
            while let Some(shape) = stack.pop() {
                match shape {
                    egui::Shape::Text(text) => texts.push(text.galley.text().to_owned()),
                    egui::Shape::Vec(shapes) => stack.extend(shapes),
                    _ => {}
                }
            }
            assert!(
                texts
                    .iter()
                    .any(|text| text == "Original group message unavailable"),
                "{texts:?}"
            );
            assert!(!texts.iter().any(|text| text.contains("fixture sentinel")));
            assert_eq!(app.reply_to, Some(target));
        }
    }
    /// Private quotes open existing source groups and ignore locally deleted sources.
    #[test]
    fn a_private_quote_returns_to_its_source_group() {
        for source_present in [true, false] {
            let ctx = egui::Context::default();
            let mut app = app();
            let group = SAMPLES[1].id;
            let original = app
                .conversations
                .get(group)
                .unwrap()
                .message("group-photo")
                .unwrap()
                .clone();
            let chat = original.sender.clone();
            let mut reply = message(
                &chat,
                "private-reply",
                true,
                original.timestamp + 1,
                Content::text("See you there"),
            );
            reply.quoted = Some(Quoted {
                labelled: Some(original.content.labelled()),
                chat: Some(group.into()),
                id: original.id,
                sender: original.sender,
                sender_name: original.sender_name,
                summary: original.content.summary(),
                mentions: original.mentions,
            });
            app.chats.push(Chat::new(chat.clone(), "Tom".into()));
            app.conversations.insert(
                chat.clone(),
                crate::app::Conversation {
                    messages: vec![reply],
                    requested: true,
                    complete: true,
                    ..Default::default()
                },
            );
            app.open_chat = Some(chat.clone());
            if !source_present {
                app.chats.retain(|known| known.id != group);
                app.conversations.remove(group);
            }
            app.attach(&ctx);
            render(&mut app, &ctx);
            let rect = ctx
                .data(|data| {
                    data.get_temp::<egui::Rect>(
                        crate::ui::conversation::bubble_id(&chat, "private-reply").with("quote"),
                    )
                })
                .expect("the private quote is drawn");
            let pos = rect.center();
            for pressed in [true, false] {
                accessible_nodes(
                    &mut app,
                    &ctx,
                    vec![
                        egui::Event::PointerMoved(pos),
                        egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                );
            }
            assert_eq!(
                app.open_chat.as_deref(),
                Some(if source_present { group } else { &chat })
            );
            if source_present {
                assert_eq!(app.scroll_anchor.as_deref(), Some("group-photo"));
            }
        }
    }

    /// Long translated unavailable-quote labels leave Cancel visible and clickable.
    #[test]
    fn an_unavailable_private_quote_keeps_cancel_inside_a_narrow_composer() {
        for locale in [crate::i18n::Locale::English, crate::i18n::Locale::German] {
            for width in [260.0, 360.0] {
                let ctx = egui::Context::default();
                ctx.enable_accesskit();
                let mut app = app();
                apply_flags(&mut app, Some("private-reply-locked-source"));
                app.locale = locale;
                app.sidebar_visible = false;
                app.attach(&ctx);
                let mut cancel = None;
                let mut frame = |events| {
                    let mut output = ctx.run_ui(
                        egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(
                                egui::Pos2::ZERO,
                                egui::vec2(width, 780.0),
                            )),
                            events,
                            ..Default::default()
                        },
                        |ui| {
                            let ctx = ui.ctx().clone();
                            app.background_frame(&ctx);
                            app.frame_ui(ui);
                        },
                    );
                    output.textures_delta.clear();
                    if let Some(tree) = output.platform_output.accesskit_update {
                        for (_, node) in tree.nodes {
                            if node.label() == Some("Cancel reply (Esc)")
                                && let Some(bounds) = node.bounds()
                            {
                                let scale = ctx.pixels_per_point() as f64;
                                cancel = Some(egui::Rect::from_min_max(
                                    egui::pos2(
                                        (bounds.x0 / scale) as f32,
                                        (bounds.y0 / scale) as f32,
                                    ),
                                    egui::pos2(
                                        (bounds.x1 / scale) as f32,
                                        (bounds.y1 / scale) as f32,
                                    ),
                                ));
                            }
                        }
                    }
                    cancel
                };
                frame(vec![]);
                let cancel = frame(vec![]).expect("the unavailable quote exposes Cancel");
                let strip = ctx
                    .data(|data| {
                        data.get_temp::<egui::Rect>(crate::ui::conversation::reply_strip_id())
                    })
                    .unwrap();
                assert!(
                    strip.contains_rect(cancel),
                    "{locale:?}: {strip:?} {cancel:?}"
                );
                assert!(cancel.left() >= 0.0 && cancel.right() <= width);
                for pressed in [true, false] {
                    frame(vec![
                        egui::Event::PointerMoved(cancel.center()),
                        egui::Event::PointerButton {
                            pos: cancel.center(),
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ]);
                }
                assert!(app.reply_to.is_none());
            }
        }
    }

    #[test]
    fn numbered_shortcuts_clear_the_arrow_selection_in_search_results() {
        let mut app = app();
        let mut first = Chat::new("search-fixture-1@g.us".into(), "Search fixture one".into());
        first.last_activity = 2_000_000_000;
        let mut second = Chat::new("search-fixture-2@g.us".into(), "Search fixture two".into());
        second.last_activity = first.last_activity - 1;
        app.chats.extend([first, second]);
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        frame_with(
            &mut app,
            &ctx,
            vec![key(egui::Key::K, egui::Modifiers::COMMAND)],
        );
        render(&mut app, &ctx);
        frame_with(
            &mut app,
            &ctx,
            vec![egui::Event::Text("Search fixture".into())],
        );
        let matches: Vec<_> = app
            .visible_chats()
            .into_iter()
            .map(|chat| chat.id.clone())
            .collect();
        assert_eq!(matches.len(), 2);
        frame_with(
            &mut app,
            &ctx,
            vec![key(egui::Key::ArrowDown, egui::Modifiers::NONE)],
        );
        assert_eq!(app.search_selected.as_ref(), matches.first());
        frame_with(
            &mut app,
            &ctx,
            vec![key(egui::Key::Num2, egui::Modifiers::COMMAND)],
        );
        render(&mut app, &ctx);
        assert_eq!(app.open_chat.as_ref(), matches.get(1));
        assert!(app.search_selected.is_none());
        assert_eq!(app.search, "Search fixture");
        assert!(ctx.memory(|memory| memory.has_focus(egui::Id::new("composer-text"))));
    }

    #[test]
    fn numbered_shortcuts_switch_chats_keep_drafts_and_focus_the_composer() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        let original = app.open_chat.clone().unwrap();
        assert_eq!(app.visible_chats()[0].id, original);
        frame_with(
            &mut app,
            &ctx,
            vec![egui::Event::Text("unsent draft".into())],
        );
        let target = app.visible_chats()[1].id.clone();
        assert_ne!(target, original);
        frame_with(
            &mut app,
            &ctx,
            vec![key(egui::Key::Num2, egui::Modifiers::COMMAND)],
        );
        assert_eq!(app.open_chat.as_ref(), Some(&target));
        render(&mut app, &ctx);
        assert!(ctx.memory(|memory| memory.has_focus(egui::Id::new("composer-text"))));
        frame_with(
            &mut app,
            &ctx,
            vec![key(egui::Key::Num1, egui::Modifiers::COMMAND)],
        );
        assert_eq!(app.open_chat.as_ref(), Some(&original));
        assert_eq!(app.composer, "unsent draft");
    }

    #[test]
    fn a_quote_names_the_people_its_text_mentions() {
        let mut app = app();
        apply_flags(&mut app, Some("quotes"));
        let ctx = egui::Context::default();
        app.attach(&ctx);
        let mut shapes = Vec::new();
        for _ in 0..3 {
            shapes = frame_sized(&mut app, &ctx, 780.0, Vec::new());
        }
        let quote = app.conversations[SAMPLES[1].id]
            .messages
            .iter()
            .find(|row| row.id == "quote-own")
            .and_then(|row| row.quoted.clone())
            .expect("a quote");
        let mention = quote.mentions.first().expect("the quote mentions Jonas");
        assert!(quote.summary.contains(&format!("@{}", mention.user)));
        let named = format!("@{} will do", app.mention_name(&mention.id));
        fn texts(shape: &egui::Shape, out: &mut Vec<String>) {
            match shape {
                egui::Shape::Text(text) => out.push(text.galley.text().to_owned()),
                egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| texts(shape, out)),
                _ => {}
            }
        }
        let mut drawn = Vec::new();
        for clipped in &shapes {
            texts(&clipped.shape, &mut drawn);
        }
        let quotes: Vec<_> = drawn
            .iter()
            .filter(|text| text.contains("will do"))
            .collect();
        assert!(
            quotes.iter().any(|text| text.starts_with(&named)),
            "{named:?} not among {quotes:?}"
        );
        assert!(
            quotes
                .iter()
                .all(|text| !text.contains(&format!("@{}", mention.user))),
            "a raw number is drawn: {quotes:?}"
        );
    }

    #[test]
    fn a_quote_bar_takes_the_quoted_senders_colour() {
        let mut app = app();
        apply_flags(&mut app, Some("quotes"));
        let ctx = egui::Context::default();
        app.attach(&ctx);
        let mut shapes = Vec::new();
        for _ in 0..3 {
            shapes = frame_sized(&mut app, &ctx, 780.0, Vec::new());
        }
        let group = SAMPLES[1].id;
        let quoted = |id: &str| {
            app.conversations[group]
                .messages
                .iter()
                .find(|row| row.id == id)
                .and_then(|row| row.quoted.clone())
                .expect("a quote")
                .sender
        };
        let palette = app.palette;
        let bar = |sender: &str, bubble: egui::Color32| {
            crate::theme::readable_on(
                bubble,
                palette.sender(crate::util::hue(sender)),
                palette.text,
                3.0,
            )
        };
        let expected = [
            bar(&quoted("group-reply"), palette.bubble_in),
            bar(&quoted("quote-own"), palette.bubble_out),
        ];
        fn bars(shape: &egui::Shape, out: &mut Vec<egui::Color32>) {
            match shape {
                egui::Shape::Rect(rect) if (rect.rect.width() - 4.0).abs() < 0.01 => {
                    out.push(rect.fill)
                }
                egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| bars(shape, out)),
                _ => {}
            }
        }
        let mut drawn = Vec::new();
        for clipped in &shapes {
            bars(&clipped.shape, &mut drawn);
        }
        for colour in expected {
            assert!(drawn.contains(&colour), "{colour:?} not among {drawn:?}");
        }
    }

    #[test]
    fn a_message_reached_from_a_quote_flashes_across_the_view_then_fades() {
        fn rects(shape: &egui::Shape, out: &mut Vec<(egui::Rect, egui::Color32)>) {
            match shape {
                egui::Shape::Rect(rect) => out.push((rect.rect, rect.fill)),
                egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| rects(shape, out)),
                _ => {}
            }
        }
        fn frame_at(
            app: &mut App,
            ctx: &egui::Context,
            time: f64,
        ) -> Vec<(egui::Rect, egui::Color32)> {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1180.0, 780.0),
                    )),
                    time: Some(time),
                    ..Default::default()
                },
                |ui| {
                    let ctx = ui.ctx().clone();
                    app.background_frame(&ctx);
                    app.frame_ui(ui);
                },
            );
            output.textures_delta.clear();
            let mut out = Vec::new();
            for clipped in &output.shapes {
                rects(&clipped.shape, &mut out);
            }
            out
        }
        let mut app = app();
        apply_flags(&mut app, Some("quote-jump"));
        let ctx = egui::Context::default();
        app.attach(&ctx);
        for _ in 0..3 {
            frame_at(&mut app, &ctx, 10.0);
        }
        let since = app
            .jump_highlight
            .as_ref()
            .and_then(|jump| jump.since)
            .expect("the quoted message came into view");
        let band = app.palette.accent.gamma_multiply(0.22);
        let shown = frame_at(&mut app, &ctx, since + 0.5);
        let widest = shown
            .iter()
            .filter(|(_, fill)| *fill == band)
            .map(|(rect, _)| rect.width())
            .fold(0.0, f32::max);
        assert!(
            widest > 600.0,
            "the band spans the message view, not the bubble: {widest}"
        );
        frame_at(
            &mut app,
            &ctx,
            since + crate::app::JumpHighlight::DURATION + 0.1,
        );
        assert!(app.jump_highlight.is_none(), "the flash ends");
        let after = frame_at(
            &mut app,
            &ctx,
            since + crate::app::JumpHighlight::DURATION + 0.2,
        );
        assert!(!after.iter().any(|(_, fill)| *fill == band));
    }

    /// Runs one frame of the given height with these input events.
    fn frame_sized(
        app: &mut App,
        ctx: &egui::Context,
        height: f32,
        events: Vec<egui::Event>,
    ) -> Vec<egui::epaint::ClippedShape> {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1180.0, height),
                )),
                events,
                ..Default::default()
            },
            |ui| {
                let ctx = ui.ctx().clone();
                app.background_frame(&ctx);
                app.frame_ui(ui);
            },
        );
        output.textures_delta.clear();
        output.shapes
    }

    fn tab() -> Vec<egui::Event> {
        vec![egui::Event::Key {
            key: egui::Key::Tab,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }]
    }

    fn ring(ctx: &egui::Context) -> Option<egui::Rect> {
        ctx.data(|data| data.get_temp::<egui::Rect>(crate::ui::focus_ring_id()))
    }

    /// Tab outlines the focused control; a click hides the outline again.
    #[test]
    fn keyboard_focus_is_outlined_until_the_pointer_is_used() {
        let mut app = app();
        apply_flags(&mut app, Some("settings"));
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        assert_eq!(ring(&ctx), None, "no outline before any key");
        for _ in 0..3 {
            frame_sized(&mut app, &ctx, 780.0, tab());
            frame_sized(&mut app, &ctx, 780.0, Vec::new());
        }
        let focused = ctx
            .memory(|memory| memory.focused())
            .and_then(|id| ctx.read_response(id))
            .expect("Tab focuses a control");
        let outline = ring(&ctx).expect("the focused control is outlined");
        assert!(outline.contains_rect(focused.interact_rect));

        let pos = egui::pos2(900.0, 40.0);
        frame_sized(
            &mut app,
            &ctx,
            780.0,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        assert_eq!(ring(&ctx), None, "the pointer hides the outline");
    }

    #[test]
    fn composer_focus_uses_the_whole_field_and_tab_uses_a_circular_record_ring() {
        let mut app = app();
        app.settings.show_shortcut_hints = true;
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        let input = egui::Id::new("composer-text");
        ctx.memory_mut(|memory| memory.request_focus(input));
        frame_sized(&mut app, &ctx, 780.0, Vec::new());
        let field = ctx
            .data(|data| data.get_temp::<crate::theme::FocusOutline>(input.with("focus-outline")))
            .unwrap();
        let text = ctx.read_response(input).unwrap();
        assert!(field.rect.contains_rect(text.rect));
        assert!(field.rect.width() > text.rect.width());
        assert_eq!(
            ring(&ctx),
            Some(field.rect),
            "input focus outlines the composer's rounded field"
        );
        assert_eq!(
            ctx.data(
                |data| data.get_temp::<egui::LayerId>(crate::ui::focus_ring_id().with("layer"))
            ),
            Some(text.layer_id)
        );
        frame_sized(&mut app, &ctx, 780.0, tab());
        frame_sized(&mut app, &ctx, 780.0, Vec::new());
        let focused = ctx.memory(|memory| memory.focused()).unwrap();
        let record = ctx
            .data(|data| data.get_temp::<crate::theme::FocusOutline>(focused.with("focus-outline")))
            .unwrap();
        assert!((record.radius * 2.0 - record.rect.width()).abs() < 0.01);
        assert_eq!(record.rect.width(), record.rect.height());
        assert_eq!(ring(&ctx), Some(record.rect));
        // Continue through the primary controls, never the message contents.
        for _ in 0..30 {
            frame_sized(&mut app, &ctx, 780.0, tab());
            for _ in 0..3 {
                frame_sized(&mut app, &ctx, 780.0, Vec::new());
            }
            if let Some(id) = ctx.memory(|memory| memory.focused()) {
                let response = ctx.read_response(id).expect("focused target is rendered");
                assert!(
                    ring(&ctx).is_some(),
                    "missing outline for {id:?}: {:?}",
                    response.rect
                );
                assert!(
                    response.interact_rect.is_positive(),
                    "focus is visible: {id:?} {:?} {:?}",
                    response.rect,
                    response.interact_rect
                );
            }
        }
    }

    #[test]
    fn filters_stay_on_one_line_and_locked_is_only_shown_when_needed() {
        use crate::ui::rail::{Destination, destination_id};
        let mut app = app();
        for chat in &mut app.chats {
            chat.locked = false;
        }
        app.open_chat = None;
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        assert!(
            ctx.read_response(destination_id(Destination::Locked))
                .is_none(),
            "no locked chats: no Locked in the rail"
        );
        app.chats[0].locked = true;
        render(&mut app, &ctx);
        let locked = ctx
            .read_response(destination_id(Destination::Locked))
            .expect("Locked joins the rail's bottom group")
            .rect;
        let settings = ctx
            .read_response(destination_id(Destination::Settings))
            .unwrap()
            .rect;
        assert!(locked.bottom() <= settings.top());
        let chips: Vec<_> = crate::model::ChatFilter::EVERY
            .into_iter()
            .map(|filter| {
                ctx.data(|data| {
                    data.get_temp::<egui::Rect>(crate::ui::chats::filter_chip_id(filter))
                })
                .unwrap()
            })
            .collect();
        for rect in &chips {
            assert!((rect.top() - chips[0].top()).abs() < 0.1);
        }
    }

    #[test]
    fn tab_after_record_does_not_focus_a_group_sender_or_passive_message() {
        let mut app = app();
        let group = app
            .chats
            .iter()
            .find(|chat| chat.is_group())
            .unwrap()
            .id
            .clone();
        let sender = "15550000123@s.whatsapp.net";
        app.contacts.insert(
            sender.into(),
            Contact {
                id: sender.into(),
                full_name: Some("Alex Fixture".into()),
                first_name: None,
                push_name: None,
            },
        );
        let mut row = message(
            &group,
            "plain-group-message",
            false,
            crate::util::now(),
            Content::text("A synthetic message"),
        );
        row.sender = sender.into();
        app.conversations.get_mut(&group).unwrap().messages = vec![row];
        app.open_chat = Some(group.clone());
        app.settings.show_shortcut_hints = false;
        app.focus_composer = true;
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        let bubble = crate::ui::conversation::bubble_id(&group, "plain-group-message");
        assert!(!ctx.read_response(bubble).unwrap().sense.is_focusable());
        for _ in 0..2 {
            frame_sized(&mut app, &ctx, 780.0, tab());
            render(&mut app, &ctx);
        }
        assert_eq!(focused_stop(&ctx), Some(crate::ui::focus::Stop::Attach));
        assert!(ring(&ctx).is_some());
        frame_with(
            &mut app,
            &ctx,
            vec![egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert!(
            app.composer_tools_open,
            "Enter opens the composer tools menu"
        );
    }

    /// A draft sits as far below the field's top as above its bottom: the
    /// span from the capitals' top to the descenders' bottom centres on the
    /// plus and emoji buttons at every scale. Centring the line box left
    /// typed text two points high at 133%; centring the capitals alone left
    /// it low, since descenders reach further down than accents rise.
    #[test]
    fn the_composers_text_centres_on_its_controls_at_every_scale() {
        use crate::ui::focus::Stop;
        const DRAFT: &str = "Hy";
        for scale in [1.0_f32, 1.25, 1.333_333, 1.5, 2.0] {
            let mut app = app();
            app.composer = DRAFT.into();
            let ctx = egui::Context::default();
            app.attach(&ctx);
            let mut shapes = Vec::new();
            for _ in 0..4 {
                let mut input = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1180.0, 780.0),
                    )),
                    ..Default::default()
                };
                input
                    .viewports
                    .entry(egui::ViewportId::ROOT)
                    .or_default()
                    .native_pixels_per_point = Some(scale);
                let mut output = ctx.run_ui(input, |ui| {
                    let ctx = ui.ctx().clone();
                    app.background_frame(&ctx);
                    app.frame_ui(ui);
                });
                output.textures_delta.clear();
                shapes = output.shapes;
            }
            assert!((ctx.pixels_per_point() - scale).abs() < 1e-4);
            let id = crate::ui::focus::stops(&ctx)
                .into_iter()
                .find(|(found, _)| *found == Stop::Attach)
                .map(|(_, id)| id)
                .expect("the plus is drawn");
            let control = ctx.read_response(id).unwrap().rect.center().y;
            let mut ink: Option<(f32, f32)> = None;
            let mut stack: Vec<&egui::Shape> =
                shapes.iter().map(|clipped| &clipped.shape).collect();
            while let Some(shape) = stack.pop() {
                match shape {
                    egui::Shape::Vec(shapes) => stack.extend(shapes.iter()),
                    egui::Shape::Text(text) if text.galley.text() == DRAFT => {
                        for row in &text.galley.rows {
                            for glyph in row
                                .glyphs
                                .iter()
                                .filter(|glyph| !glyph.uv_rect.is_nothing())
                            {
                                let top =
                                    text.pos.y + row.pos.y + glyph.pos.y + glyph.uv_rect.offset.y;
                                let bottom = top + glyph.uv_rect.size.y;
                                ink =
                                    Some(ink.map_or((top, bottom), |(t, b)| {
                                        (t.min(top), b.max(bottom))
                                    }));
                            }
                        }
                    }
                    _ => {}
                }
            }
            let (top, bottom) = ink.expect("the draft is painted");
            let middle = (top + bottom) / 2.0;
            // Within a physical pixel, plus rounding: snapping to the grid
            // may cost up to one, and macOS positions glyphs unhinted.
            assert!(
                (middle - control).abs() <= 1.0 / scale + 0.05,
                "at {scale}x the text centres on {middle}, the controls on {control}"
            );
        }
    }

    /// Every painted copy of `needle` in `shapes`: its ink's top and bottom
    /// (the glyphs' own pixels, not the line box) and its galley's rect.
    fn painted_ink(
        shapes: &[egui::epaint::ClippedShape],
        needle: &str,
    ) -> Vec<(f32, f32, egui::Rect)> {
        let mut found = Vec::new();
        let mut stack: Vec<&egui::Shape> = shapes.iter().map(|clipped| &clipped.shape).collect();
        while let Some(shape) = stack.pop() {
            match shape {
                egui::Shape::Vec(shapes) => stack.extend(shapes.iter()),
                egui::Shape::Text(text) if text.galley.text() == needle => {
                    let mut ink: Option<(f32, f32)> = None;
                    for row in &text.galley.rows {
                        for glyph in row
                            .glyphs
                            .iter()
                            .filter(|glyph| !glyph.uv_rect.is_nothing())
                        {
                            let top = text.pos.y + row.pos.y + glyph.pos.y + glyph.uv_rect.offset.y;
                            let bottom = top + glyph.uv_rect.size.y;
                            ink = Some(
                                ink.map_or((top, bottom), |(t, b)| (t.min(top), b.max(bottom))),
                            );
                        }
                    }
                    if let Some((top, bottom)) = ink {
                        found.push((
                            top,
                            bottom,
                            egui::Rect::from_min_size(text.pos, text.galley.size()),
                        ));
                    }
                }
                _ => {}
            }
        }
        found
    }

    /// The chat list's search field centres a capital's ink, from its top to
    /// the baseline, on the field and its icons at every scale; the hint sits
    /// on the same line, and the clear button's cross on the same centre.
    /// Centring the line box left text below the magnifier and the cross
    /// lower still, and centring descenders too lifted most words above it.
    #[test]
    fn the_search_fields_text_centres_on_its_icons_at_every_scale() {
        // A capital's ink, from its top to the baseline, centres on the field.
        const DRAFT: &str = "H";
        let field = egui::Id::new("chat-search");
        for scale in [1.0_f32, 1.25, 1.333_333, 1.5, 2.0] {
            let mut ink_at = None;
            let mut hint_at = None;
            for text in [DRAFT, ""] {
                let mut app = app();
                app.search = text.into();
                let ctx = egui::Context::default();
                ctx.enable_accesskit();
                app.attach(&ctx);
                let mut shapes = Vec::new();
                let mut nodes = Vec::new();
                for _ in 0..4 {
                    let mut input = egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(1180.0, 780.0),
                        )),
                        ..Default::default()
                    };
                    input
                        .viewports
                        .entry(egui::ViewportId::ROOT)
                        .or_default()
                        .native_pixels_per_point = Some(scale);
                    let mut output = ctx.run_ui(input, |ui| {
                        let ctx = ui.ctx().clone();
                        app.background_frame(&ctx);
                        app.frame_ui(ui);
                    });
                    output.textures_delta.clear();
                    shapes = output.shapes;
                    nodes = output
                        .platform_output
                        .accesskit_update
                        .map(|update| {
                            update
                                .nodes
                                .into_iter()
                                .filter_map(|(_, node)| {
                                    let label = node.label()?.to_owned();
                                    let bounds = node.bounds()?;
                                    Some((label, ((bounds.y0 + bounds.y1) / 2.0) as f32))
                                })
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default();
                }
                let pill = ctx
                    .read_response(field.with("pill"))
                    .expect("the search field is drawn")
                    .rect;
                let centre = pill.center().y;
                let tolerance = 1.0 / scale + 0.05;
                let galley = ctx
                    .data(|data| {
                        data.get_temp::<egui::Rect>(crate::ui::widgets::search_galley_id(field))
                    })
                    .expect("the field publishes its text's place");
                if text.is_empty() {
                    let (_, _, hint) = painted_ink(&shapes, "Search")
                        .into_iter()
                        .find(|(_, _, rect)| pill.contains(rect.center()))
                        .expect("the hint is painted in the field");
                    assert!(
                        (hint.top() - galley.top()).abs() < 0.01,
                        "at {scale}x the hint starts at {}, typed text at {}",
                        hint.top(),
                        galley.top()
                    );
                    hint_at = Some(hint.top());
                } else {
                    let (top, bottom, _) = painted_ink(&shapes, DRAFT)
                        .into_iter()
                        .find(|(_, _, rect)| pill.contains(rect.center()))
                        .expect("the draft is painted in the field");
                    let middle = (top + bottom) / 2.0;
                    assert!(
                        (middle - centre).abs() <= tolerance,
                        "at {scale}x the text centres on {middle}, the field on {centre}"
                    );
                    let clear = nodes
                        .iter()
                        .find(|(label, _)| label == "Clear")
                        .map(|(_, y)| *y)
                        .expect("the clear button is labelled");
                    assert!(
                        (clear - centre).abs() <= tolerance,
                        "at {scale}x the clear button centres on {clear}, the field on {centre}"
                    );
                    ink_at = Some(galley.top());
                }
            }
            assert_eq!(ink_at, hint_at, "at {scale}x hint and text share a line");
        }
    }

    /// A call entry draws as WhatsApp's: the glyph's disc centred on the
    /// title and detail together, the detail under the title at its start,
    /// and the time on the detail's line at the bubble's end, on both sides
    /// of a group, where a sender's name heads the bubble.
    #[test]
    fn a_call_entry_centres_its_glyph_and_tucks_its_time_beside_the_detail() {
        let mut app = app();
        apply_flags(&mut app, Some("call-bubbles"));
        let chat = SAMPLES[1].id;
        let ctx = egui::Context::default();
        app.attach(&ctx);
        let mut shapes = Vec::new();
        for _ in 0..4 {
            shapes = frame_sized(&mut app, &ctx, 1400.0, Vec::new());
        }
        let calls: Vec<Message> = app.conversations[chat]
            .messages
            .iter()
            .filter(|message| matches!(message.content, Content::Call { .. }))
            .cloned()
            .collect();
        assert_eq!(calls.len(), 6, "the sample files six calls");
        let mut discs = Vec::new();
        let mut stack: Vec<&egui::Shape> = shapes.iter().map(|clipped| &clipped.shape).collect();
        while let Some(shape) = stack.pop() {
            match shape {
                egui::Shape::Vec(shapes) => stack.extend(shapes.iter()),
                egui::Shape::Circle(circle) if (circle.radius - 18.0).abs() < 0.01 => {
                    discs.push(circle.center);
                }
                _ => {}
            }
        }
        for message in &calls {
            let entry = crate::ui::calls::Entry::of(message).unwrap();
            let bubble = ctx
                .data(|data| {
                    data.get_temp::<egui::Rect>(
                        crate::ui::conversation::bubble_id(chat, &message.id).with("rect"),
                    )
                })
                .unwrap_or_else(|| panic!("{} is drawn", message.id));
            let inside = |rect: &egui::Rect| bubble.contains(rect.center());
            let title = painted_ink(&shapes, &entry.title(app.locale))
                .into_iter()
                .map(|(_, _, rect)| rect)
                .find(inside)
                .unwrap_or_else(|| panic!("{}'s title is painted", message.id));
            let disc = discs
                .iter()
                .copied()
                .find(|centre| bubble.contains(*centre))
                .unwrap_or_else(|| panic!("{}'s glyph sits in a disc", message.id));
            let footer = ctx
                .data(|data| {
                    data.get_temp::<egui::Rect>(crate::ui::conversation::footer_id(
                        chat,
                        &message.id,
                    ))
                })
                .expect("the time is drawn");
            assert!(disc.x < title.left(), "{}: the disc leads", message.id);
            assert!(
                footer.right() <= bubble.right() - 9.0 && footer.bottom() <= bubble.bottom(),
                "{}: the time stays inside the bubble's padding",
                message.id
            );
            match entry.detail(app.locale) {
                Some(detail) => {
                    let detail = painted_ink(&shapes, &detail)
                        .into_iter()
                        .map(|(_, _, rect)| rect)
                        .find(inside)
                        .unwrap_or_else(|| panic!("{}'s detail is painted", message.id));
                    assert!(
                        (detail.left() - title.left()).abs() < 0.01,
                        "{}: title and detail share a start",
                        message.id
                    );
                    assert!(detail.top() >= title.bottom(), "{}", message.id);
                    let block = (title.top() + detail.bottom()) / 2.0;
                    assert!(
                        (disc.y - block).abs() <= 1.0,
                        "{}: the disc centres on {}, the words on {block}",
                        message.id,
                        disc.y
                    );
                    assert!(
                        (footer.center().y - detail.center().y).abs() <= 0.5,
                        "{}: the time sits on the detail's line",
                        message.id
                    );
                    assert!(
                        footer.left() >= detail.right() + 8.0,
                        "{}: the time follows the detail",
                        message.id
                    );
                }
                None => {
                    assert!(footer.top() >= title.bottom(), "{}", message.id);
                    let block = (title.top() + footer.bottom()) / 2.0;
                    assert!(
                        (disc.y - block).abs() <= 1.0,
                        "{}: the disc centres on {}, title and time on {block}",
                        message.id,
                        disc.y
                    );
                }
            }
        }
    }

    /// The theme row links to the website's guide to writing a theme.
    #[test]
    fn the_theme_row_opens_the_guide_to_making_a_theme() {
        let mut app = app();
        app.page = crate::model::Page::Settings;
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        app.attach(&ctx);
        render(&mut app, &ctx);
        let nodes = accessible_nodes(&mut app, &ctx, Vec::new());
        let pos = nodes
            .into_iter()
            .find(|(label, role, _)| {
                label == "How to make a theme" && *role == egui::accesskit::Role::Button
            })
            .map(|(_, _, centre)| centre)
            .expect("the guide button is on the Settings page");
        let mut opened = Vec::new();
        for pressed in [true, false] {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1180.0, 780.0),
                    )),
                    events: vec![
                        egui::Event::PointerMoved(pos),
                        egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                    ..Default::default()
                },
                |ui| {
                    let ctx = ui.ctx().clone();
                    app.background_frame(&ctx);
                    app.frame_ui(ui);
                },
            );
            output.textures_delta.clear();
            opened.extend(
                output
                    .platform_output
                    .commands
                    .into_iter()
                    .filter_map(|command| match command {
                        egui::OutputCommand::OpenUrl(open) => Some(open.url),
                        _ => None,
                    }),
            );
        }
        assert_eq!(opened, ["https://zapfast.rocks/themes/"]);
    }

    /// The chat list is one clickable surface: each row starts where the one
    /// above ends, with no gap or rule between them.
    #[test]
    fn chat_rows_touch_with_no_gap_between_them() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        render(&mut app, &ctx);
        let rects: Vec<egui::Rect> = app
            .visible_chats()
            .iter()
            .filter_map(|chat| {
                ctx.data(|data| {
                    data.get_temp::<egui::Rect>(crate::ui::chats::chat_row_id(&chat.id))
                })
            })
            .collect();
        assert!(rects.len() >= 3, "several rows are on screen");
        for pair in rects.windows(2) {
            assert!(
                (pair[1].top() - pair[0].bottom()).abs() < 0.01,
                "a gap between rows: {:?} then {:?}",
                pair[0],
                pair[1]
            );
        }
    }

    /// A reply strip sits right on the composer it belongs to.
    #[test]
    fn the_reply_strip_sits_close_above_the_composer() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        let chat = app.open_chat.clone().expect("a chat is open");
        app.reply_to = app
            .conversations
            .get(&chat)
            .and_then(|conversation| conversation.messages.last())
            .map(|message| message.id.clone().into());
        for _ in 0..4 {
            frame_sized(&mut app, &ctx, 780.0, Vec::new());
        }
        let (strip, pill) = ctx.data(|data| {
            (
                data.get_temp::<egui::Rect>(crate::ui::conversation::reply_strip_id()),
                data.get_temp::<egui::Rect>(crate::ui::conversation::composer_pill_id()),
            )
        });
        let (strip, pill) = (
            strip.expect("the strip is drawn"),
            pill.expect("the composer"),
        );
        let gap = pill.top() - strip.bottom();
        assert!(
            (gap - crate::ui::conversation::STRIP_GAP).abs() < 0.5,
            "the strip ends {gap} above the composer"
        );
    }

    /// The emoji and @mention suggestion lists sit on the composer as the
    /// strips do: as wide as it, and a strip's gap above it.
    #[test]
    fn suggestion_lists_sit_close_above_the_composer() {
        for page in ["emoji-complete", "mention"] {
            let mut app = app();
            let ctx = egui::Context::default();
            app.attach(&ctx);
            apply_flags(&mut app, Some(page));
            for _ in 0..4 {
                frame_sized(&mut app, &ctx, 780.0, Vec::new());
            }
            let (list, pill) = ctx.data(|data| {
                (
                    data.get_temp::<egui::Rect>(crate::ui::conversation::suggestion_list_id()),
                    data.get_temp::<egui::Rect>(crate::ui::conversation::composer_pill_id()),
                )
            });
            let (list, pill) = (
                list.expect("the list is drawn"),
                pill.expect("the composer"),
            );
            let gap = pill.top() - list.bottom();
            assert!(
                (gap - crate::ui::conversation::STRIP_GAP).abs() < 0.5,
                "{page}: the list ends {gap} above the composer"
            );
            assert!(
                (list.left() - pill.left()).abs() < 0.5
                    && (list.right() - pill.right()).abs() < 0.5,
                "{page}: the list {list:?} spans the composer {pill:?}"
            );
        }
    }

    /// The chat list's header and the conversation's start their first row
    /// at the same height and make it as tall, so the titles line up, in the
    /// macOS layout (no title bar, traffic lights on the row) as elsewhere.
    #[test]
    fn both_headers_share_their_first_row() {
        for macos in [false, true] {
            let mut app = app();
            let ctx = egui::Context::default();
            app.attach(&ctx);
            if macos {
                crate::theme::preview_macos(&ctx);
            }
            for _ in 0..3 {
                frame_sized(&mut app, &ctx, 780.0, Vec::new());
            }
            let (list, chat) = ctx.data(|data| {
                (
                    data.get_temp::<egui::Rect>(crate::ui::chats::header_row_id()),
                    data.get_temp::<egui::Rect>(crate::ui::conversation::header_row_id()),
                )
            });
            let (list, chat) = (
                list.expect("the chat list"),
                chat.expect("the conversation"),
            );
            assert!(
                (list.top() - chat.top()).abs() < 0.5
                    && (list.height() - chat.height()).abs() < 0.5,
                "macOS {macos}: chat list row {list:?}, conversation row {chat:?}"
            );
        }
    }

    /// The text starts right after the plus and emoji pair, as close to the
    /// emoji as the emoji is to the plus, not a field's width away.
    #[test]
    fn the_composers_text_follows_the_emoji_button_closely() {
        use crate::ui::focus::Stop;
        for draft in ["", "A synthetic draft"] {
            let mut app = app();
            app.composer = draft.into();
            let ctx = egui::Context::default();
            app.attach(&ctx);
            for _ in 0..4 {
                frame_sized(&mut app, &ctx, 780.0, Vec::new());
            }
            let emoji = crate::ui::focus::stops(&ctx)
                .into_iter()
                .find(|(found, _)| *found == Stop::Emoji)
                .and_then(|(_, id)| ctx.read_response(id))
                .expect("the emoji button is drawn")
                .rect;
            let text = ctx
                .read_response(egui::Id::new("composer-text"))
                .expect("the field is drawn")
                .rect;
            let gap = text.left() - emoji.right();
            assert!(
                (gap - crate::ui::conversation::COMPOSER_TEXT_GAP).abs() < 0.5,
                "{draft:?}: the text starts {gap} after the emoji button"
            );
        }
    }

    /// Plus, emoji, the first line of text and send or record share the
    /// rounded field's vertical centre; a longer draft keeps them on its
    /// last line.
    #[test]
    fn composer_controls_share_the_fields_vertical_centre() {
        use crate::ui::focus::Stop;
        let centre = |ctx: &egui::Context, stop: Stop| {
            let id = crate::ui::focus::stops(ctx)
                .into_iter()
                .find(|(found, _)| *found == stop)
                .map(|(_, id)| id)
                .unwrap_or_else(|| panic!("{stop:?} is drawn"));
            ctx.read_response(id).unwrap().rect.center().y
        };
        let measure = |app: &mut App, ctx: &egui::Context| {
            for _ in 0..3 {
                frame_sized(app, ctx, 780.0, Vec::new());
            }
            let pill = ctx
                .data(|data| {
                    data.get_temp::<egui::Rect>(crate::ui::conversation::composer_pill_id())
                })
                .expect("the composer is drawn");
            let text = ctx
                .read_response(egui::Id::new("composer-text"))
                .unwrap()
                .rect;
            (
                pill,
                text,
                [Stop::Attach, Stop::Emoji, Stop::Send].map(|stop| centre(ctx, stop)),
            )
        };
        for (draft, hints) in [("", false), ("A synthetic draft", false), ("", true)] {
            let mut app = app();
            app.settings.show_shortcut_hints = hints;
            app.composer = draft.into();
            let ctx = egui::Context::default();
            app.attach(&ctx);
            render(&mut app, &ctx);
            let (pill, text, controls) = measure(&mut app, &ctx);
            let middle = pill.center().y;
            // The text's ink, not its line box, centres on the field: see
            // `the_composers_text_centres_on_its_controls_at_every_scale`.
            assert!(
                pill.contains_rect(text),
                "text {text:?} leaves the field {pill:?}"
            );
            for (stop, y) in [Stop::Attach, Stop::Emoji, Stop::Send].iter().zip(controls) {
                assert!(
                    (y - middle).abs() <= 1.0,
                    "{stop:?} {y} vs field {middle} ({draft:?})"
                );
            }
        }
        // Three lines: the controls stay centred on the last line.
        let mut app = app();
        app.composer = "one\ntwo\nthree".into();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        let (pill, text, controls) = measure(&mut app, &ctx);
        // One line is 40pt tall; three clearly outgrow it.
        assert!(pill.height() > 70.0, "the field grew: {pill:?}");
        let line = text.height() / 3.0;
        let last = text.bottom() - line / 2.0;
        // The last line's ink centres on the controls, which puts its line box
        // up to a couple of points higher (Inter's box is roomier above).
        for (stop, y) in [Stop::Attach, Stop::Emoji, Stop::Send].iter().zip(controls) {
            assert!(
                (0.0..=2.5).contains(&(y - last)),
                "{stop:?} {y} vs last line {last}"
            );
        }
    }

    /// A keystroke that wraps the draft onto another line, or joins it back,
    /// shows the grown field in that same frame: sizing it from the frame
    /// before made the field and its text jump while typing.
    #[test]
    fn typing_never_shows_the_composer_a_frame_late() {
        for scale in [1.0_f32, 1.25, 1.5] {
            let mut app = app();
            app.focus_composer = true;
            let ctx = egui::Context::default();
            ctx.set_pixels_per_point(scale);
            app.attach(&ctx);
            render(&mut app, &ctx);
            for _ in 0..3 {
                frame_with(&mut app, &ctx, Vec::new());
            }
            let snap = |ctx: &egui::Context| {
                ctx.data(|data| {
                    (
                        data.get_temp::<egui::Rect>(crate::ui::conversation::composer_pill_id()),
                        data.get_temp::<egui::Rect>(crate::ui::conversation::composer_text_id()),
                    )
                })
            };
            let key = |key| egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            };
            let draft = "A synthetic line that keeps going until it wraps across the composer, with enough words after that to be sure it reaches a second row.";
            let mut events: Vec<egui::Event> = draft
                .chars()
                .map(|ch| egui::Event::Text(ch.to_string()))
                .collect();
            events.extend((0..draft.len()).map(|_| key(egui::Key::Backspace)));
            events.push(egui::Event::Text("one\ntwo\nthree".into()));
            let mut heights = std::collections::BTreeSet::new();
            for (index, event) in events.into_iter().enumerate() {
                frame_with(&mut app, &ctx, vec![event]);
                let typed = snap(&ctx);
                frame_with(&mut app, &ctx, Vec::new());
                let settled = snap(&ctx);
                let (Some(pill), Some(text)) = settled else {
                    panic!("the composer is drawn")
                };
                // The field, and where its text starts, must not move once
                // the keystroke has landed. (egui lays out the first
                // character typed into an empty field a frame late, so the
                // text's far corner is not compared.)
                let (Some(typed_pill), Some(typed_text)) = typed else {
                    panic!("the composer is drawn")
                };
                assert!(
                    (typed_pill.min - pill.min).length() < 0.5
                        && (typed_pill.max - pill.max).length() < 0.5
                        && (typed_text.min - text.min).length() < 0.5,
                    "event {index} at scale {scale}: {typed:?} then {settled:?}"
                );
                assert!(
                    pill.contains_rect(text),
                    "text {text:?} leaves the field {pill:?}"
                );
                heights.insert(pill.height() as u32);
            }
            assert!(
                heights.len() >= 3,
                "the draft wrapped and grew: {heights:?}"
            );
        }
    }

    /// The send button sits evenly in the field's rounded end: as far from
    /// its right edge as from its top and bottom.
    #[test]
    fn the_send_button_is_inset_evenly_in_the_field() {
        let mut app = app();
        app.composer = "A synthetic draft".into();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        for _ in 0..3 {
            frame_sized(&mut app, &ctx, 780.0, Vec::new());
        }
        let pill = ctx
            .data(|data| data.get_temp::<egui::Rect>(crate::ui::conversation::composer_pill_id()))
            .expect("the composer is drawn");
        let id = crate::ui::focus::stops(&ctx)
            .into_iter()
            .find(|(stop, _)| *stop == crate::ui::focus::Stop::Send)
            .map(|(_, id)| id)
            .expect("send is drawn");
        let send = ctx.read_response(id).unwrap().rect;
        let right = pill.right() - send.right();
        let bottom = pill.bottom() - send.bottom();
        assert!(
            (right - bottom).abs() <= 1.0,
            "send is {right} from the right and {bottom} from the bottom"
        );
        // The plus mirrors it in the left end, and emoji stays close by.
        let rect = |stop| {
            let id = crate::ui::focus::stops(&ctx)
                .into_iter()
                .find(|(found, _)| *found == stop)
                .map(|(_, id)| id)
                .unwrap_or_else(|| panic!("{stop:?} is drawn"));
            ctx.read_response(id).unwrap().rect
        };
        let plus = rect(crate::ui::focus::Stop::Attach);
        let emoji = rect(crate::ui::focus::Stop::Emoji);
        let left = plus.center().x - pill.left();
        let right = pill.right() - send.center().x;
        assert!(
            (left - right).abs() <= 1.0,
            "plus centre is {left} from the left, send's {right} from the right"
        );
        let apart = emoji.center().x - plus.center().x;
        assert!(apart <= 32.0, "plus and emoji are {apart} apart");
    }

    /// The recorder runs discard, time, waveform and send from left to right,
    /// with the waveform taking the space between.
    #[test]
    fn the_recorder_waveform_fills_the_field() {
        let mut app = app();
        app.recording = Some(crate::audio::Recorder::rehearsal());
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        for _ in 0..3 {
            frame_sized(&mut app, &ctx, 780.0, Vec::new());
        }
        let wave = ctx
            .data(|data| data.get_temp::<egui::Rect>(crate::ui::conversation::recording_wave_id()))
            .expect("the recorder is drawn");
        // The window is 1180 points wide; the old strip stopped at 150.
        assert!(
            wave.width() > 400.0,
            "the waveform is {} wide",
            wave.width()
        );
    }

    #[test]
    fn the_plus_menu_sends_files_or_creates_a_poll_and_closes() {
        let click = |app: &mut App, ctx: &egui::Context, pos: egui::Pos2| {
            let press = |pressed| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            frame_with(app, ctx, vec![egui::Event::PointerMoved(pos), press(true)]);
            frame_with(app, ctx, vec![press(false)]);
            render(app, ctx);
        };
        let plus = |ctx: &egui::Context| {
            let id = crate::ui::focus::stops(ctx)
                .into_iter()
                .find(|(stop, _)| *stop == crate::ui::focus::Stop::Attach)
                .map(|(_, id)| id)
                .expect("the plus button is a tab stop");
            (id, ctx.read_response(id).unwrap().rect.center())
        };
        // Row 0 sends files, row 1 documents, row 2 sends a location, row 3
        // creates a poll.
        for row in [0.0, 1.0, 2.0, 3.0] {
            let mut app = app();
            app.settings.show_shortcut_hints = false;
            let chat = app.open_chat.clone().unwrap();
            app.backend.record_demo_commands();
            let ctx = egui::Context::default();
            app.attach(&ctx);
            render(&mut app, &ctx);
            let (id, center) = plus(&ctx);
            click(&mut app, &ctx, center);
            assert!(app.composer_tools_open, "the plus button opens the menu");
            let menu = ctx
                .memory(|memory| memory.area_rect(id.with("composer-tools")))
                .expect("the menu is shown");
            assert!(
                menu.bottom() <= center.y,
                "the menu opens above the composer"
            );
            let item = egui::pos2(
                menu.center().x,
                menu.top() + menu.height() * (1.0 + 2.0 * row) / 8.0,
            );
            click(&mut app, &ctx, item);
            assert!(
                !app.composer_tools_open,
                "choosing an entry closes the menu"
            );
            let commands = app.backend.take_demo_commands();
            let picked = commands.iter().find_map(|command| match command {
                crate::backend::Command::PickFiles {
                    chat: id,
                    documents,
                } if *id == chat => Some(*documents),
                _ => None,
            });
            if row == 0.0 {
                assert_eq!(picked, Some(false), "Send files opens the file picker");
                assert_eq!(app.dialog, None);
            } else if row == 1.0 {
                assert_eq!(picked, Some(true), "documents open the file picker");
                assert_eq!(app.dialog, None);
            } else if row == 2.0 {
                assert_eq!(picked, None);
                assert_eq!(app.dialog, Some(crate::model::Dialog::SendLocation(chat)));
            } else {
                assert_eq!(picked, None);
                assert_eq!(app.dialog, Some(crate::model::Dialog::CreatePoll(chat)));
            }
        }
    }

    #[test]
    fn the_plus_menu_closes_for_the_picker_and_is_hidden_while_editing() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        app.actions
            .push(crate::model::Action::SetComposerTools(true));
        render(&mut app, &ctx);
        assert!(app.composer_tools_open);
        app.actions.push(crate::model::Action::TogglePicker(
            crate::model::PickerTab::Emoji,
        ));
        render(&mut app, &ctx);
        assert!(!app.composer_tools_open, "the emoji picker closes the menu");
        assert!(app.picker.is_some());
        app.actions
            .push(crate::model::Action::SetComposerTools(true));
        render(&mut app, &ctx);
        assert!(app.picker.is_none(), "the menu closes the emoji picker");
        let own = app.conversations[app.open_chat.as_deref().unwrap()]
            .messages
            .iter()
            .rev()
            .find(|message| message.from_me && matches!(message.content, Content::Text { .. }))
            .map(|message| message.id.clone())
            .expect("an own text message to edit");
        assert!(app.composer_tools_open);
        app.actions.push(crate::model::Action::Edit(own));
        render(&mut app, &ctx);
        assert!(app.editing.is_some());
        assert!(!app.composer_tools_open, "editing closes the menu");
        assert!(
            !crate::ui::focus::stops(&ctx)
                .iter()
                .any(|(stop, _)| *stop == crate::ui::focus::Stop::Attach),
            "editing hides the plus button"
        );
    }

    fn focused_stop(ctx: &egui::Context) -> Option<crate::ui::focus::Stop> {
        let focused = ctx.memory(|memory| memory.focused());
        crate::ui::focus::stops(ctx)
            .into_iter()
            .find(|(_, id)| Some(*id) == focused)
            .map(|(stop, _)| stop)
    }

    fn assert_single_focus_border(
        app: &App,
        ctx: &egui::Context,
        shapes: &[egui::epaint::ClippedShape],
    ) {
        let rect = ring(ctx).expect("a visible focus border at every stop");
        let outline = ctx.memory(|memory| memory.focused()).and_then(|id| {
            ctx.data(|data| data.get_temp::<crate::theme::FocusOutline>(id.with("focus-outline")))
        });
        let color = if outline.is_some_and(|outline| outline.fill == app.palette.accent) {
            app.palette.on_accent
        } else {
            app.palette.accent
        };
        fn borders(shape: &egui::Shape, rect: egui::Rect, accent: egui::Color32) -> usize {
            match shape {
                egui::Shape::Vec(shapes) => shapes
                    .iter()
                    .map(|shape| borders(shape, rect, accent))
                    .sum(),
                egui::Shape::Rect(shape)
                    if shape.stroke.color == accent
                        && shape.rect.intersects(rect)
                        && shape.stroke.width > 0.0 =>
                {
                    assert_eq!(shape.rect, rect, "no second inner or outer border");
                    assert_eq!(shape.stroke.width, 1.0, "every focus border is one point");
                    assert_eq!(shape.stroke_kind, egui::StrokeKind::Inside);
                    1
                }
                _ => 0,
            }
        }
        let mut count = 0;
        for shape in shapes {
            let found = borders(&shape.shape, rect, color);
            if found > 0 {
                assert!(
                    shape.clip_rect.contains_rect(rect),
                    "unclipped border: {rect:?} in {:?}, stop {:?}",
                    shape.clip_rect,
                    focused_stop(ctx)
                );
            }
            count += found;
        }
        assert_eq!(count, 1, "exactly one focus border");
    }

    #[test]
    fn main_tab_cycle_skips_rich_messages_and_chat_rows_in_both_directions() {
        use crate::ui::focus::Stop;
        for (hints, ready, macos) in [
            (false, false, false),
            (true, false, false),
            (false, true, false),
            (false, false, true),
        ] {
            let mut app = app();
            app.settings.show_shortcut_hints = hints;
            // This tests navigation through a fixed transcript, not live
            // typing-indicator expiry while a slower CI runner draws it.
            app.typing.clear();
            if ready {
                app.composer = "A synthetic draft".into();
            }
            app.settings.sidebar_width = 280.0;
            // Keep all the sample images, replies and reactions, but render
            // them as group messages too, including clickable sender avatars.
            let direct = app.open_chat.clone().unwrap();
            let group = app
                .chats
                .iter()
                .find(|chat| chat.is_group())
                .unwrap()
                .id
                .clone();
            let messages = app.conversations[&direct].messages.clone();
            assert!(
                messages
                    .iter()
                    .any(|message| matches!(message.content, Content::Image { .. }))
            );
            assert!(messages.iter().any(|message| !message.reactions.is_empty()));
            assert!(messages.iter().any(|message| message.quoted.is_some()));
            app.conversations.get_mut(&group).unwrap().messages = messages;
            app.open_chat = Some(group.clone());
            app.focus_composer = true;
            let ctx = egui::Context::default();
            app.attach(&ctx);
            if macos {
                crate::theme::preview_macos(&ctx);
            }
            render(&mut app, &ctx);
            // Let media decoding and the initial bottom-scroll settle before
            // measuring whether keyboard navigation moves the transcript.
            for _ in 0..20 {
                frame_sized(&mut app, &ctx, 780.0, Vec::new());
            }
            // The same on macOS: the rail carries Settings there too, beside
            // the app menu.
            let expected = vec![
                Stop::Composer,
                Stop::Send,
                Stop::Attach,
                Stop::Emoji,
                Stop::ChatSearch,
                Stop::Messages,
                Stop::Chats,
                Stop::Calls,
                Stop::Starred,
                Stop::Archived,
                Stop::Locked,
                Stop::Settings,
                Stop::Profile,
                Stop::NewChat,
                Stop::Sidebar,
                Stop::Search,
                Stop::All,
                Stop::Unread,
                Stop::Private,
                Stop::Favorites,
                Stop::Groups,
                Stop::Channels,
            ];
            assert_eq!(
                crate::ui::focus::stops(&ctx)
                    .iter()
                    .map(|(stop, _)| *stop)
                    .collect::<Vec<_>>(),
                expected
            );
            let last = &app.conversations[&group].messages.last().unwrap().id;
            let bubble_rect = crate::ui::conversation::bubble_id(&group, last).with("rect");
            let initial_rect = ctx
                .data(|data| data.get_temp::<egui::Rect>(bubble_rect))
                .unwrap();
            for backwards in [false, true] {
                ctx.memory_mut(|memory| memory.request_focus(egui::Id::new("composer-text")));
                for step in 1..=expected.len() * 2 {
                    let modifiers = if backwards {
                        egui::Modifiers::SHIFT
                    } else {
                        egui::Modifiers::NONE
                    };
                    frame_sized(&mut app, &ctx, 780.0, vec![key(egui::Key::Tab, modifiers)]);
                    let shapes = frame_sized(&mut app, &ctx, 780.0, Vec::new());
                    let index = if backwards {
                        (expected.len() - step % expected.len()) % expected.len()
                    } else {
                        step % expected.len()
                    };
                    assert_eq!(
                        focused_stop(&ctx),
                        Some(expected[index]),
                        "step {step}, backwards {backwards}"
                    );
                    assert_single_focus_border(&app, &ctx, &shapes);
                    assert_eq!(
                        ctx.data(|data| data.get_temp::<egui::Rect>(bubble_rect)),
                        Some(initial_rect),
                        "Tab never scrolls the conversation: step {step}, backwards {backwards}"
                    );
                }
            }
            // Pointer/accessibility focus on a chat row must not trap Tab
            // within the list. Both directions rejoin the primary cycle.
            let row = ctx
                .data(|data| {
                    data.get_temp::<egui::Id>(crate::ui::chats::chat_row_id(&direct).with("widget"))
                })
                .unwrap();
            assert!(ctx.read_response(row).unwrap().sense.is_focusable());
            for (modifiers, expected_stop) in [
                (egui::Modifiers::NONE, Stop::Composer),
                (egui::Modifiers::SHIFT, Stop::Channels),
            ] {
                ctx.memory_mut(|memory| memory.request_focus(row));
                frame_sized(&mut app, &ctx, 780.0, vec![key(egui::Key::Tab, modifiers)]);
                assert_eq!(focused_stop(&ctx), Some(expected_stop));
            }
        }
    }

    #[test]
    fn main_tab_cycle_tracks_hidden_and_read_only_controls() {
        use crate::ui::focus::Stop;
        for page in [
            "nosidebar",
            "rail",
            "empty",
            "channel",
            "search",
            "chat",
            "chat-search",
        ] {
            let mut app = app();
            apply_flags(&mut app, Some(page));
            let ctx = egui::Context::default();
            app.attach(&ctx);
            render(&mut app, &ctx);
            let controls = crate::ui::focus::stops(&ctx);
            assert!(!controls.is_empty());
            assert_eq!(
                controls.iter().any(|(stop, _)| *stop == Stop::Composer),
                !matches!(page, "empty" | "channel")
            );
            assert_eq!(
                controls
                    .iter()
                    .filter(|(stop, _)| *stop == Stop::Sidebar)
                    .count(),
                1,
                "{page}: one button hides or shows the list"
            );
            if page == "nosidebar" {
                assert_eq!(
                    controls.iter().map(|(stop, _)| *stop).collect::<Vec<_>>(),
                    [
                        Stop::Composer,
                        Stop::Send,
                        Stop::Attach,
                        Stop::Emoji,
                        Stop::ChatSearch,
                        Stop::Messages,
                        Stop::Chats,
                        Stop::Calls,
                        Stop::Starred,
                        Stop::Archived,
                        Stop::Locked,
                        Stop::Settings,
                        Stop::Profile,
                        Stop::Sidebar
                    ]
                );
            }
            for backwards in [false, true] {
                ctx.memory_mut(|memory| memory.request_focus(controls[0].1));
                for step in 1..=controls.len() {
                    let modifiers = if backwards {
                        egui::Modifiers::SHIFT
                    } else {
                        egui::Modifiers::NONE
                    };
                    frame_sized(&mut app, &ctx, 780.0, vec![key(egui::Key::Tab, modifiers)]);
                    let index = if backwards {
                        (controls.len() - step % controls.len()) % controls.len()
                    } else {
                        step % controls.len()
                    };
                    assert_eq!(
                        ctx.memory(|memory| memory.focused()),
                        Some(controls[index].1),
                        "{page}, step {step}"
                    );
                }
            }
        }
    }

    #[test]
    fn dialogs_keep_local_navigation_and_single_focus_borders() {
        for page in [
            "locked-setup",
            "poll-create",
            "new-chat",
            "new-group",
            "new-group-name",
            "location",
        ] {
            let mut app = app();
            apply_flags(&mut app, Some(page));
            let ctx = egui::Context::default();
            app.attach(&ctx);
            // Let the dialog's opening opacity animation finish.
            for _ in 0..8 {
                render(&mut app, &ctx);
            }
            for _ in 0..8 {
                frame_sized(&mut app, &ctx, 780.0, tab());
                // egui's local order transfers focus at the end of a pass,
                // then reveals off-screen dialog rows on subsequent frames.
                render(&mut app, &ctx);
                let shapes = frame_sized(&mut app, &ctx, 780.0, Vec::new());
                if let Some(id) = ctx.memory(|memory| memory.focused()) {
                    assert!(
                        ctx.read_response(id).unwrap().layer_id.order >= egui::Order::Foreground,
                        "{page}: focus stays in the dialog"
                    );
                    assert_single_focus_border(&app, &ctx, &shapes);
                }
            }
        }
    }

    #[test]
    fn tab_still_completes_emoji_and_mentions_before_leaving_the_input() {
        for page in ["emoji-complete", "mention"] {
            let mut app = app();
            apply_flags(&mut app, Some(page));
            let before = app.composer.clone();
            let ctx = egui::Context::default();
            app.attach(&ctx);
            render(&mut app, &ctx);
            frame_sized(&mut app, &ctx, 780.0, tab());
            render(&mut app, &ctx);
            assert_ne!(app.composer, before);
            assert!(app.emoji_start.is_none());
            assert!(app.mention_start.is_none());
            assert_eq!(focused_stop(&ctx), Some(crate::ui::focus::Stop::Composer));
        }
    }

    #[test]
    fn question_mark_opens_help_but_never_steals_it_from_text_fields() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        let input = egui::Id::new("composer-text");
        ctx.memory_mut(|memory| memory.request_focus(input));
        frame_sized(&mut app, &ctx, 780.0, Vec::new());
        let events = || {
            vec![
                egui::Event::Key {
                    key: egui::Key::Questionmark,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::SHIFT,
                },
                egui::Event::Text("?".into()),
            ]
        };
        frame_sized(&mut app, &ctx, 780.0, events());
        assert_eq!(app.composer, "?");
        assert!(app.dialog.is_none());
        ctx.memory_mut(|memory| memory.surrender_focus(input));
        frame_sized(&mut app, &ctx, 780.0, Vec::new());
        frame_sized(&mut app, &ctx, 780.0, events());
        assert_eq!(app.dialog, Some(crate::model::Dialog::Shortcuts));
    }

    /// Sending from the emoji picker's Recent row leaves the row in its
    /// order until the picker opens again, so the same emoji can be sent
    /// twice from where it was (#294).
    #[test]
    fn recent_emoji_keep_their_order_while_the_picker_is_open() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        app.settings.recent_emoji = vec!["😀".into(), "❤️".into()];
        apply_flags(&mut app, Some("picker"));
        render(&mut app, &ctx);
        assert_eq!(app.picker_recent, Some(vec!["😀".into(), "❤️".into()]));
        app.actions
            .push(crate::model::Action::InsertEmoji("❤️".into()));
        render(&mut app, &ctx);
        assert_eq!(app.settings.recent_emoji[0], "❤️", "remembered at once");
        assert_eq!(
            app.picker_recent,
            Some(vec!["😀".into(), "❤️".into()]),
            "shown as it was"
        );
        app.picker = None;
        render(&mut app, &ctx);
        assert_eq!(app.picker_recent, None);
        apply_flags(&mut app, Some("picker"));
        render(&mut app, &ctx);
        assert_eq!(
            app.picker_recent.as_ref().map(|recent| recent[0].as_str()),
            Some("❤️")
        );
    }

    /// The About dialog shows the mark the app ships, rendered at its pixel
    /// size from the packaged SVG, not a disc drawn in the theme's colours.
    #[test]
    fn about_shows_the_shipped_mark() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        app.dialog = Some(crate::model::Dialog::About);
        render(&mut app, &ctx);
        let texture = ctx
            .data_mut(|data| {
                data.get_temp::<egui::TextureHandle>(egui::Id::new(("zapfast-mark", 44_usize)))
            })
            .expect("the mark was drawn at 44 pixels");
        assert_eq!(texture.size(), [44, 44]);
    }

    /// The shortcuts dialog stays inside the window: two columns where the
    /// window is wide enough, and a list that scrolls where it is too short.
    #[test]
    fn the_shortcuts_dialog_fits_the_window() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        app.dialog = Some(crate::model::Dialog::Shortcuts);
        for size in [
            [1180.0, 780.0],
            [1600.0, 1000.0],
            [760.0, 560.0],
            [420.0, 380.0],
        ] {
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::Vec2::from(size));
            for _ in 0..4 {
                let input = egui::RawInput {
                    screen_rect: Some(screen),
                    ..Default::default()
                };
                let mut output = ctx.run_ui(input, |ui| {
                    let ctx = ui.ctx().clone();
                    app.background_frame(&ctx);
                    app.frame_ui(ui);
                });
                output.textures_delta.clear();
            }
            let dialog = ctx
                .memory(|memory| memory.area_rect(egui::Id::new("dialog")))
                .expect("the dialog is open");
            assert!(
                screen.shrink(8.0).contains_rect(dialog),
                "{size:?}: {dialog:?}"
            );
        }
    }

    /// The profile picture in the chat-list header is the first control Tab
    /// reaches outside macOS. Registered as hover first and made clickable
    /// afterwards, it dropped focus on every frame and Tab went nowhere.
    #[test]
    fn a_clickable_avatar_keeps_keyboard_focus() {
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        let palette = crate::theme::Palette::dark();
        let mut id = None;
        for _ in 0..3 {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                let response = crate::ui::widgets::clickable_avatar(
                    ui,
                    &palette,
                    "Fixture",
                    "1@s.whatsapp.net",
                    34.0,
                    None,
                    "Your profile and settings",
                );
                if id.is_none() {
                    response.request_focus();
                    id = Some(response.id);
                }
            });
            output.textures_delta.clear();
        }
        assert_eq!(ctx.memory(|memory| memory.focused()), id);
    }

    /// egui does not scroll to focus by itself; Tab must not lead the focus
    /// out of sight in a long page.
    #[test]
    fn tab_scrolls_the_focused_control_into_view() {
        let mut app = app();
        apply_flags(&mut app, Some("settings"));
        let ctx = egui::Context::default();
        app.attach(&ctx);
        let height = 420.0;
        for _ in 0..3 {
            frame_sized(&mut app, &ctx, height, Vec::new());
        }
        // The chat list only lays out rows it shows.
        let row = |ctx: &egui::Context, id: &str| {
            ctx.data(|data| data.get_temp::<egui::Rect>(crate::ui::chats::chat_row_id(id)))
        };
        let ids: Vec<String> = app.chats.iter().map(|chat| chat.id.clone()).collect();
        let hidden: Vec<&String> = ids
            .iter()
            .filter(|id| row(&ctx, id).is_none_or(|rect| rect.top() >= height))
            .collect();
        assert!(
            !hidden.is_empty(),
            "the window is short enough to hide rows"
        );
        let mut reached_hidden = false;
        for _ in 0..30 {
            frame_sized(&mut app, &ctx, height, tab());
            // egui moves focus at the end of the Tab frame; the next frame
            // scrolls, and egui asks for the frames that show the result.
            for _ in 0..3 {
                frame_sized(&mut app, &ctx, height, Vec::new());
            }
            let Some(focused) = ctx
                .memory(|memory| memory.focused())
                .and_then(|id| ctx.read_response(id))
            else {
                continue;
            };
            assert_eq!(
                focused.interact_rect, focused.rect,
                "the focused control is fully visible"
            );
            reached_hidden |= hidden
                .iter()
                .any(|id| row(&ctx, id).is_some_and(|rect| rect == focused.rect));
        }
        assert!(reached_hidden, "Tab reached chats that were out of view");
    }

    /// The same with the list hidden, at the window's least height: the
    /// chats' avatars scroll between the rail's groups, and the macOS layout,
    /// which starts the rail under the traffic lights, leaves them least.
    #[test]
    fn tab_scrolls_a_focused_collapsed_avatar_into_view() {
        for macos in [false, true] {
            let mut app = app();
            apply_flags(&mut app, Some("settings,rail"));
            let ctx = egui::Context::default();
            app.attach(&ctx);
            if macos {
                crate::theme::preview_macos(&ctx);
            }
            // `main`'s least window height.
            let height = 300.0;
            for _ in 0..3 {
                frame_sized(&mut app, &ctx, height, Vec::new());
            }
            let avatar = |ctx: &egui::Context, id: &str| {
                ctx.data(|data| data.get_temp::<egui::Rect>(crate::ui::chats::compact_chat_id(id)))
            };
            let ids: Vec<String> = app.chats.iter().map(|chat| chat.id.clone()).collect();
            let hidden: Vec<&String> = ids
                .iter()
                .filter(|id| avatar(&ctx, id).is_none_or(|rect| rect.top() >= height))
                .collect();
            assert!(
                !hidden.is_empty(),
                "{macos}: the window is short enough to hide avatars"
            );
            let mut reached_hidden = false;
            for _ in 0..40 {
                frame_sized(&mut app, &ctx, height, tab());
                for _ in 0..3 {
                    frame_sized(&mut app, &ctx, height, Vec::new());
                }
                let Some(focused) = ctx
                    .memory(|memory| memory.focused())
                    .and_then(|id| ctx.read_response(id))
                else {
                    continue;
                };
                // Settings has its own scrolling; only the rail is under test.
                if !ids
                    .iter()
                    .any(|id| avatar(&ctx, id).is_some_and(|rect| rect == focused.rect))
                {
                    continue;
                }
                assert_eq!(
                    focused.interact_rect, focused.rect,
                    "{macos}: the focused avatar is fully visible"
                );
                assert!(
                    focused.rect.bottom() <= height,
                    "{macos}: inside the window: {:?}",
                    focused.rect
                );
                reached_hidden |= hidden
                    .iter()
                    .any(|id| avatar(&ctx, id).is_some_and(|rect| rect == focused.rect));
            }
            assert!(
                reached_hidden,
                "{macos}: Tab reached avatars that were out of view"
            );
        }
    }

    /// Records the images the UI asks for, answering at once so a frame can be
    /// inspected without waiting on a decoding thread.
    struct CountingImages(std::sync::Arc<std::sync::Mutex<Vec<String>>>);

    impl egui::load::ImageLoader for CountingImages {
        fn id(&self) -> &str {
            "zapfast::demo::tests::CountingImages"
        }

        fn load(
            &self,
            _ctx: &egui::Context,
            uri: &str,
            _size_hint: egui::load::SizeHint,
        ) -> egui::load::ImageLoadResult {
            self.0
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(uri.to_owned());
            Ok(egui::load::ImagePoll::Ready {
                image: std::sync::Arc::new(egui::ColorImage::filled(
                    [900, 1200],
                    egui::Color32::from_rgb(20, 40, 60),
                )),
            })
        }

        fn forget(&self, _uri: &str) {}

        fn forget_all(&self) {}

        fn byte_size(&self) -> usize {
            0
        }
    }

    #[test]
    fn pictures_out_of_view_are_not_decoded() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        let loads = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        ctx.add_image_loader(std::sync::Arc::new(CountingImages(std::sync::Arc::clone(
            &loads,
        ))));

        // Far more picture rows than the window holds, each with its own path
        // so every row would ask for an image of its own.
        let chat = app.chats[0].id.clone();
        app.open_chat = Some(chat.clone());
        let rows: Vec<Message> = (0..40)
            .map(|index| {
                let mut media = media("image/jpeg", 1_000, Some(900), Some(1200));
                media.path = Some(std::path::PathBuf::from(format!(
                    "/nonexistent/demo-picture-{index}.jpg"
                )));
                message(
                    &chat,
                    &format!("picture-{index}"),
                    false,
                    1_700_000_000 + index,
                    Content::Image {
                        motion: None,
                        caption: None,
                        media,
                    },
                )
            })
            .collect();
        app.conversations.entry(chat).or_default().messages = rows;

        render(&mut app, &ctx);

        let asked = loads
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .filter(|uri| uri.contains("demo-picture-"))
            .count();
        assert!(
            asked < 15,
            "only the rows on screen should be decoded, {asked} of 40 were"
        );
    }

    #[test]
    fn video_posters_out_of_view_are_not_registered() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);

        let chat = app.chats[0].id.clone();
        app.open_chat = Some(chat.clone());
        let id = |index: usize| format!("poster-{index}");
        let rows: Vec<Message> = (0..40)
            .map(|index| {
                let mut row = message(
                    &chat,
                    &id(index),
                    false,
                    1_700_000_000 + index as i64,
                    Content::Video {
                        caption: None,
                        media: media("video/mp4", 820_000, Some(1280), Some(720)),
                        seconds: Some(5),
                        gif: false,
                        note: false,
                    },
                );
                row.thumbnail = Some(sample_thumbnail(index as u32));
                row
            })
            .collect();
        app.conversations.entry(chat.clone()).or_default().messages = rows;

        render(&mut app, &ctx);

        // Registering a poster decodes it, so only the rows on screen may have
        // done so, and at least one of them must have.
        let key: String = chat.chars().filter(char::is_ascii_alphanumeric).collect();
        let registered = (0..40)
            .filter(|index| {
                let uri = format!("bytes://thumb-{key}-{}", id(*index));
                ctx.try_load_bytes(&uri).is_ok()
            })
            .count();
        assert!(
            (1..15).contains(&registered),
            "only the rows on screen should register a poster, {registered} of 40 did"
        );
    }

    /// A video sent before ZapFast made thumbnails has none. With its file
    /// here it still shows as a video; only without either is it a file card.
    #[test]
    fn a_video_without_a_thumbnail_shows_from_its_file() {
        fn texts(shape: &egui::Shape, out: &mut Vec<String>) {
            match shape {
                egui::Shape::Text(text) => out.push(text.galley.text().to_owned()),
                egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| texts(shape, out)),
                _ => {}
            }
        }
        let drawn = |app: &mut App, ctx: &egui::Context| {
            let mut shapes = Vec::new();
            for _ in 0..3 {
                shapes = frame_sized(app, ctx, 780.0, Vec::new());
            }
            let mut drawn = Vec::new();
            for clipped in &shapes {
                texts(&clipped.shape, &mut drawn);
            }
            drawn
        };
        let mut app = app();
        apply_flags(&mut app, Some("video"));
        let ctx = egui::Context::default();
        app.attach(&ctx);
        let chat = SAMPLES[0].id;
        let rows = &mut app.conversations.get_mut(chat).unwrap().messages;
        rows.retain(|row| row.id == "demo-video");
        rows[0].thumbnail = None;
        let card = |drawn: &[String]| drawn.iter().any(|text| text == "Video");
        assert!(
            !card(&drawn(&mut app, &ctx)),
            "a video with its file here is not a file card"
        );

        let rows = &mut app.conversations.get_mut(chat).unwrap().messages;
        if let Content::Video { media, .. } = &mut rows[0].content {
            media.path = None;
        }
        assert!(
            card(&drawn(&mut app, &ctx)),
            "without a thumbnail or a file it is a card to download"
        );
    }

    #[test]
    fn sidebar_can_be_hidden_and_the_composer_sends() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        app.actions.push(crate::model::Action::ToggleSidebar);
        render(&mut app, &ctx);
        assert!(!app.sidebar_visible);
        app.composer = "hello".into();
        app.actions.push(crate::model::Action::SendText {
            chat: sample_ids()[0].into(),
            text: "hello".into(),
            quoting: None,
        });
        render(&mut app, &ctx);
        assert!(app.reply_to.is_none());
    }

    /// Where the chat list's right edge is: the first chat row spans the
    /// list's width.
    fn chat_list_right(ctx: &egui::Context) -> f32 {
        ctx.data(|data| data.get_temp::<egui::Rect>(crate::ui::chats::chat_row_id(sample_ids()[0])))
            .expect("the first chat is listed")
            .right()
    }

    /// Drags from `from` through each x in `path` at one height, returning
    /// the chat list's right edge after every step, and releases.
    fn drag_list_edge(app: &mut App, ctx: &egui::Context, from: f32, path: &[f32]) -> Vec<f32> {
        let y = 400.0;
        let press = |x: f32, pressed| egui::Event::PointerButton {
            pos: egui::pos2(x, y),
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame_with(
            app,
            ctx,
            vec![egui::Event::PointerMoved(egui::pos2(from, y))],
        );
        frame_with(app, ctx, vec![press(from, true)]);
        let mut edges = Vec::new();
        for &x in path {
            frame_with(app, ctx, vec![egui::Event::PointerMoved(egui::pos2(x, y))]);
            edges.push(chat_list_right(ctx));
        }
        let last = path.last().copied().unwrap_or(from);
        frame_with(app, ctx, vec![press(last, false)]);
        render(app, ctx);
        edges
    }

    /// #239: a chat list dragged out past its widest follows the pointer
    /// back in the same drag, instead of sticking at its widest.
    #[test]
    fn the_chat_list_shrinks_back_from_its_widest() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        let edge = chat_list_right(&ctx);
        let out = (1..=30).map(|step| edge + 10.0 * step as f32);
        let back = (1..=40).map(|step| edge + 300.0 - 5.0 * step as f32);
        let path: Vec<f32> = out.chain(back).collect();
        let edges = drag_list_edge(&mut app, &ctx, edge, &path);
        let end = *path.last().expect("a path");
        assert!(
            (chat_list_right(&ctx) - end).abs() < 12.0,
            "the list stopped at {} with the pointer at {end}: {edges:?}",
            chat_list_right(&ctx)
        );
    }

    /// #239: a drag to the left, taken on the conversation's side of the
    /// list's edge, narrows the list and never widens it.
    #[test]
    fn dragging_the_chat_list_edge_left_never_widens_it() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        let start = chat_list_right(&ctx) + 2.0;
        let path: Vec<f32> = (1..=6).map(|step| start - 5.0 * step as f32).collect();
        let edges = drag_list_edge(&mut app, &ctx, start, &path);
        assert!(
            edges.iter().all(|&edge| edge < start),
            "dragging left widened the list: {edges:?}"
        );
        assert!(
            (chat_list_right(&ctx) - (start - 30.0)).abs() < 12.0,
            "the list stopped at {} instead of {}: {edges:?}",
            chat_list_right(&ctx),
            start - 30.0
        );
    }
}

/// A long transcript lays out only the rows near the screen.
#[cfg(test)]
mod wallpaper_tests {
    use super::tests::{app, render};
    use super::*;
    use crate::settings::WallpaperColor;
    use egui::epaint::{ClippedShape, Shape};

    fn shapes(app: &mut App, ctx: &egui::Context) -> Vec<Shape> {
        render(app, ctx);
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1180.0, 780.0),
                )),
                ..Default::default()
            },
            |ui| {
                let ctx = ui.ctx().clone();
                app.background_frame(&ctx);
                app.frame_ui(ui);
            },
        );
        output.textures_delta.clear();
        let mut flat = Vec::new();
        fn flatten(shape: Shape, into: &mut Vec<Shape>) {
            match shape {
                Shape::Vec(shapes) => shapes.into_iter().for_each(|shape| flatten(shape, into)),
                shape => into.push(shape),
            }
        }
        for ClippedShape { shape, .. } in output.shapes {
            flatten(shape, &mut flat);
        }
        flat
    }

    /// Where meshes with this texture were painted.
    fn textured(shapes: &[Shape], texture: egui::TextureId) -> Vec<egui::Rect> {
        shapes
            .iter()
            .filter_map(|shape| match shape {
                Shape::Mesh(mesh) if mesh.texture_id == texture => Some(mesh.calc_bounds()),
                _ => None,
            })
            .collect()
    }

    /// Where rectangles of this colour were filled.
    fn filled(shapes: &[Shape], color: egui::Color32) -> Vec<egui::Rect> {
        shapes
            .iter()
            .filter_map(|shape| match shape {
                Shape::Rect(rect) if rect.fill == color => Some(rect.rect),
                _ => None,
            })
            .collect()
    }

    /// The default Theme wallpaper is the palette's chat colour, in the chat,
    /// the preview, and its swatch, and follows a theme switch at once.
    #[test]
    fn the_theme_wallpaper_follows_the_palette() {
        let mut app = app();
        apply_flags(&mut app, Some("theme=Nord.json"));
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        assert_eq!(app.settings.dark_wallpaper_color, WallpaperColor::Theme);
        assert!(app.current_chat().is_some(), "the sample opens a chat");
        let nord = app.palette.chat;
        assert_ne!(nord, crate::theme::Palette::dark().chat);
        let chat = shapes(&mut app, &ctx);
        assert!(
            filled(&chat, nord)
                .iter()
                .any(|rect| rect.width() > 600.0 && rect.height() > 600.0),
            "the conversation is filled with the theme's chat colour"
        );
        let (_, doodles) = crate::wallpaper::texture_ids(&ctx);
        assert!(!textured(&chat, doodles).is_empty(), "doodles draw over it");

        app.page = Page::Wallpaper;
        let page = shapes(&mut app, &ctx);
        let fills = filled(&page, nord);
        assert!(
            fills
                .iter()
                .any(|rect| rect.width() > 300.0 && rect.height() > 600.0),
            "the preview shows the theme's chat colour"
        );
        assert!(
            fills.iter().any(|rect| (rect.width() - 80.0).abs() < 1.0),
            "the Theme swatch shows it too"
        );

        // Switching to a light theme recolours the preview with no new choice.
        apply_flags(&mut app, Some("theme=Rose Pine Dawn.json"));
        render(&mut app, &ctx);
        assert!(!app.palette.dark);
        let dawn = app.palette.chat;
        assert_eq!(app.settings.wallpaper_color, WallpaperColor::Theme);
        let page = shapes(&mut app, &ctx);
        assert!(
            filled(&page, dawn)
                .iter()
                .any(|rect| rect.width() > 300.0 && rect.height() > 600.0)
        );
    }

    /// An image covers the chat and the preview in place of colour and
    /// doodles, in light and dark mode, until it is removed.
    #[test]
    fn a_wallpaper_image_replaces_colour_and_doodles() {
        let mut app = app();
        apply_flags(&mut app, Some("wallpaper,wallpaper-image"));
        let ctx = egui::Context::default();
        app.attach(&ctx);
        let page = shapes(&mut app, &ctx);
        let (image, doodles) = crate::wallpaper::texture_ids(&ctx);
        let image = image.expect("the image is uploaded");
        let previews = textured(&page, image);
        assert_eq!(previews.len(), 1, "one image fills the preview");
        assert!(previews[0].width() > 300.0 && previews[0].height() > 600.0);
        assert!(textured(&page, doodles).is_empty(), "no doodles over it");

        app.page = Page::Chats;
        let chat = shapes(&mut app, &ctx);
        assert_eq!(
            crate::wallpaper::texture_ids(&ctx).0,
            Some(image),
            "the texture is made once"
        );
        let covers = textured(&chat, image);
        assert_eq!(covers.len(), 1);
        assert!(covers[0].width() > 600.0 && covers[0].height() > 600.0);
        assert!(textured(&chat, doodles).is_empty());

        apply_flags(&mut app, Some("light"));
        let light = shapes(&mut app, &ctx);
        assert!(!app.palette.dark);
        assert_eq!(textured(&light, image).len(), 1, "light mode keeps it");

        app.actions.push(crate::model::Action::RemoveWallpaperImage);
        let chat = shapes(&mut app, &ctx);
        assert!(app.account().settings.wallpaper_image.is_none());
        assert_eq!(crate::wallpaper::texture_ids(&ctx).0, None);
        assert!(textured(&chat, image).is_empty());
        assert!(!textured(&chat, doodles).is_empty(), "the doodles return");
    }
}

#[cfg(test)]
mod long_chat_tests {
    use super::tests::app;
    use super::*;
    use crate::model::{Action, Content};

    const ROWS: i64 = 1500;

    /// Opens the first sample chat with `ROWS` messages of varied length, far
    /// more than a screen, spread over many days.
    ///
    /// The rows' heights depend on the clock: where the local days start
    /// places the day separators, and the clock format sets each footer's
    /// width and so where a text wraps. The time, zone, and format are fixed
    /// until the returned guard drops, so every run lays out the same rows.
    fn long_chat() -> (App, egui::Context, String, crate::util::fixed_clock::Guard) {
        let clock = crate::util::fixed_clock::set(crate::util::fixed_clock::Clock {
            // 14:00 UTC on Wednesday 11 March 2026.
            now: 1_773_237_600,
            zone: jiff::tz::TimeZone::UTC,
            twelve_hour: false,
        });
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        let chat = SAMPLES[0].id.to_owned();
        let conversation = app.conversations.get_mut(&chat).unwrap();
        let template = conversation.messages.last().unwrap().clone();
        conversation.messages.clear();
        for n in 0..ROWS {
            let mut row = template.clone();
            row.id = format!("long-{n}");
            row.from_me = n % 3 == 0;
            row.timestamp = template.timestamp - (ROWS - n) * 3_000;
            // Lengths vary from one line to several, so estimates are off.
            let words = 2 + (n * 7_919 % 70) as usize;
            let body: Vec<&str> = std::iter::repeat_n("lorem", words).collect();
            row.content = Content::text(format!("Row {n} {}", body.join(" ")));
            conversation.messages.push(row);
        }
        // No unread divider: its first placement lays out every row.
        for row in &mut app.chats {
            row.unread = 0;
        }
        app.actions.push(Action::OpenChat(chat.clone()));
        (app, ctx, chat, clock)
    }

    fn frame(
        app: &mut App,
        ctx: &egui::Context,
        events: Vec<egui::Event>,
    ) -> Vec<egui::epaint::ClippedShape> {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1180.0, 780.0),
                )),
                events,
                ..Default::default()
            },
            |ui| {
                let ctx = ui.ctx().clone();
                app.background_frame(&ctx);
                app.frame_ui(ui);
            },
        );
        output.textures_delta.clear();
        output.shapes
    }

    /// A trackpad scroll by `delta` points. Steps under 8 points apply at
    /// once; a larger wheel step would be smoothed over several frames.
    fn wheel(delta: f32) -> Vec<egui::Event> {
        let step = delta / 50.0;
        std::iter::once(egui::Event::PointerMoved(egui::pos2(700.0, 400.0)))
            .chain((0..50).map(|_| egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, step),
                modifiers: egui::Modifiers::NONE,
                phase: egui::TouchPhase::Move,
            }))
            .collect()
    }

    /// Where each row's text (`Row N ...`) is painted inside its clip rect.
    fn painted_rows(shapes: &[egui::epaint::ClippedShape]) -> std::collections::HashMap<i64, f32> {
        shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                egui::Shape::Text(text)
                    if clipped.clip_rect.contains(text.pos + egui::vec2(1.0, 1.0)) =>
                {
                    let number = text.galley.text().strip_prefix("Row ")?;
                    let number = number.split_whitespace().next()?.parse().ok()?;
                    Some((number, text.pos.y))
                }
                _ => None,
            })
            .collect()
    }

    fn laid_out(ctx: &egui::Context, chat: &str, row: i64) -> bool {
        let id = crate::ui::conversation::bubble_id(chat, &format!("long-{row}")).with("rect");
        ctx.data(|data| data.get_temp::<egui::Rect>(id)).is_some()
    }

    #[test]
    fn only_rows_near_the_screen_are_laid_out() {
        let (mut app, ctx, chat, _clock) = long_chat();
        for _ in 0..3 {
            frame(&mut app, &ctx, Vec::new());
        }
        let rows = painted_rows(&frame(&mut app, &ctx, Vec::new()));
        assert!(
            rows.contains_key(&(ROWS - 1)),
            "the newest row is on screen"
        );
        assert!(laid_out(&ctx, &chat, ROWS - 1));
        // The first frame starts at the top, so the oldest rows were laid
        // out once; the middle of the history never was.
        assert!(!laid_out(&ctx, &chat, ROWS / 2), "a far row is skipped");
    }

    /// Rows scrolled past are measured for the first time, and their real
    /// heights differ from the estimates, but what is on screen moves only
    /// with the scroll: the same distance for the same scroll every frame.
    #[test]
    fn scrolling_up_moves_the_rows_by_the_scroll_alone() {
        let (mut app, ctx, _, _clock) = long_chat();
        for _ in 0..3 {
            frame(&mut app, &ctx, Vec::new());
        }
        // The first scroll only releases the view from the end.
        frame(&mut app, &ctx, wheel(150.0));
        let mut before = painted_rows(&frame(&mut app, &ctx, wheel(150.0)));
        let mut step = None;
        for n in 0..60 {
            let after = painted_rows(&frame(&mut app, &ctx, wheel(150.0)));
            let moves: Vec<f32> = before
                .iter()
                .filter_map(|(row, y)| Some(after.get(row)? - y))
                .collect();
            assert!(!moves.is_empty(), "some row stays on screen");
            // Whatever the frame moved by, every row on screen took it: a row
            // measured for the first time moves the rows below it and nothing
            // else, so it would stand out here.
            for moved in &moves {
                assert!(
                    (moved - moves[0]).abs() < 1.0,
                    "a row moved {moved} where the frame moves {}",
                    moves[0]
                );
            }
            // The delta that accumulated while the view was still held at the
            // end lands in one frame, so the first frame's step is not the
            // wheel's. Every frame after it is, and it does not change.
            if n > 0 {
                let step = *step.get_or_insert(moves[0]);
                assert!(step > 100.0, "the view scrolls up: {step}");
                assert!(
                    (moves[0] - step).abs() < 1.0,
                    "a frame moved {} where the scroll moves {step}",
                    moves[0]
                );
            }
            before = after;
        }
        assert!(
            before.keys().all(|row| *row < ROWS - 60),
            "the scroll went well into the history: {:?}",
            before.keys()
        );
    }

    #[test]
    fn a_jump_far_up_lands_on_the_message_and_stays_there() {
        let (mut app, ctx, chat, _clock) = long_chat();
        for _ in 0..3 {
            frame(&mut app, &ctx, Vec::new());
        }
        app.actions.push(Action::OpenMessage {
            chat: chat.clone(),
            message: "long-40".into(),
        });
        for _ in 0..3 {
            frame(&mut app, &ctx, Vec::new());
        }
        let first = painted_rows(&frame(&mut app, &ctx, Vec::new()));
        let y = *first.get(&40).expect("the message is on screen");
        assert!(
            (150.0..650.0).contains(&y),
            "the message is near the middle: {y}"
        );
        for _ in 0..5 {
            frame(&mut app, &ctx, Vec::new());
        }
        let later = painted_rows(&frame(&mut app, &ctx, Vec::new()));
        assert_eq!(
            later.get(&40),
            Some(&y),
            "the message stays where it landed"
        );
    }

    /// One frame of a trackpad gesture at `pos`: several small steps, which
    /// egui applies at once, or the fingers lifting when `steps` is empty.
    fn swipe(pos: egui::Pos2, steps: &[f32]) -> Vec<egui::Event> {
        let wheel = |delta: f32, phase| egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, delta),
            modifiers: egui::Modifiers::NONE,
            phase,
        };
        std::iter::once(egui::Event::PointerMoved(pos))
            .chain(if steps.is_empty() {
                vec![wheel(0.0, egui::TouchPhase::End)]
            } else {
                steps
                    .iter()
                    .map(|&step| wheel(step, egui::TouchPhase::Move))
                    .collect()
            })
            .collect()
    }

    /// #274: a trackpad scroll that begins over the messages stays with them
    /// when the pointer drifts over the chat list, fingers down and in the
    /// glide after they lift; a gesture that begins over the list scrolls it.
    #[test]
    fn a_scroll_stays_with_the_pane_it_began_over() {
        let (mut app, ctx, chat, _clock) = long_chat();
        // Enough chats for the list to scroll.
        let template = app.chats[1].clone();
        for n in 0..60 {
            let mut extra = template.clone();
            extra.id = format!("39000000{n:04}@s.whatsapp.net");
            app.chats.push(extra);
        }
        for _ in 0..3 {
            frame(&mut app, &ctx, Vec::new());
        }
        let list = |ctx: &egui::Context| {
            ctx.data(|data| data.get_temp::<f32>(crate::ui::chats::list_offset_id()))
                .expect("the chat list has drawn")
        };
        let messages = |ctx: &egui::Context| {
            ctx.data(|data| {
                data.get_temp::<(f32, f32)>(crate::ui::conversation::scroll_metrics_id(&chat))
            })
            .expect("the message list has drawn")
            .0
        };
        let over_list = egui::pos2(150.0, 400.0);
        let over_messages = egui::pos2(700.0, 400.0);
        let settle = |app: &mut App| {
            for _ in 0..90 {
                frame(app, &ctx, Vec::new());
            }
        };

        // Down the list first, so it has room to move either way.
        let before = list(&ctx);
        for _ in 0..4 {
            frame(&mut app, &ctx, swipe(over_list, &[-6.0; 6]));
        }
        frame(&mut app, &ctx, swipe(over_list, &[]));
        settle(&mut app);
        assert!(
            list(&ctx) > before + 50.0,
            "a swipe over the list scrolls it"
        );

        // Up the messages, then over the list with the fingers still down.
        let list_before = list(&ctx);
        let start = messages(&ctx);
        for _ in 0..3 {
            frame(&mut app, &ctx, swipe(over_messages, &[6.0; 6]));
        }
        let over = messages(&ctx);
        assert!(over < start, "the swipe scrolls the messages up");
        for _ in 0..3 {
            frame(&mut app, &ctx, swipe(over_list, &[6.0; 6]));
        }
        assert!(
            messages(&ctx) < over - 50.0,
            "the messages keep scrolling with the pointer over the list"
        );
        assert_eq!(list(&ctx), list_before, "the list holds still");
        // The fingers lift over the list; the glide stays with the messages.
        let lifted = messages(&ctx);
        frame(&mut app, &ctx, swipe(over_list, &[]));
        settle(&mut app);
        assert_eq!(list(&ctx), list_before, "the glide leaves the list alone");
        if cfg!(target_os = "linux") {
            assert!(messages(&ctx) < lifted, "the glide scrolls the messages");
        }

        // A new gesture over the list scrolls the list.
        let still = messages(&ctx);
        for _ in 0..3 {
            frame(&mut app, &ctx, swipe(over_list, &[6.0; 6]));
        }
        assert!(
            list(&ctx) < list_before,
            "a new swipe over the list scrolls it"
        );
        assert_eq!(messages(&ctx), still, "and leaves the messages alone");
    }
}

/// A picture whose file does not have the proportions its message states
/// takes one height on screen and another while scrolled away. At the edge of
/// a transcript held at its end, that must not shake the chat (#179).
#[cfg(test)]
mod picture_edge_tests {
    use super::tests::app;
    use super::*;
    use crate::model::{Action, Content};

    /// Opens a chat of text rows with an undimensioned portrait picture
    /// `after` rows from the end, in a window `height` points tall.
    fn chat_with_picture(after: i64, height: f32) -> Vec<(String, egui::Pos2)> {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        app.typing.clear();
        let (photo, _) = sample_files(&app);
        let chat = SAMPLES[0].id.to_owned();
        let conversation = app.conversations.get_mut(&chat).unwrap();
        let template = conversation.messages.last().unwrap().clone();
        conversation.messages.clear();
        let rows = 30 + 1 + after;
        for n in 0..rows {
            let mut row = template.clone();
            row.id = format!("edge-{n}");
            row.from_me = n % 3 == 0;
            row.reactions.clear();
            row.quoted = None;
            row.timestamp = template.timestamp - (rows - n) * 60;
            let words: Vec<&str> =
                std::iter::repeat_n("lorem", 3 + (n * 7 % 20) as usize).collect();
            row.content = Content::text(format!("Row {n} {}", words.join(" ")));
            if n == 30 {
                // The sample photo is portrait; the message gives no size.
                let mut picture = media("image/jpeg", 402_113, None, None);
                picture.path = Some(photo.clone());
                row.content = Content::Image {
                    motion: None,
                    caption: Some("Row 30 picture".into()),
                    media: picture,
                };
            }
            conversation.messages.push(row);
        }
        for row in &mut app.chats {
            row.unread = 0;
        }
        app.open_chat = None;
        app.actions.push(Action::OpenChat(chat));
        let mut time = 0.0;
        let mut frame = |app: &mut App| {
            time += 1.0 / 60.0;
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1180.0, height),
                    )),
                    time: Some(time),
                    ..Default::default()
                },
                |ui| {
                    let ctx = ui.ctx().clone();
                    app.background_frame(&ctx);
                    app.frame_ui(ui);
                },
            );
            output.textures_delta.clear();
            let view = app.selection_view.lock().unwrap().unwrap();
            output
                .shapes
                .iter()
                .filter(|clipped| clipped.clip_rect == view)
                .filter_map(|clipped| match &clipped.shape {
                    egui::Shape::Text(text) => Some((text.galley.text().to_owned(), text.pos)),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        // Wait for the actual decode before measuring stability. A fixed
        // sleep can expire before the loader runs on a busy CI host.
        let uri = crate::util::image_uri(&photo);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            frame(&mut app);
            if matches!(
                ctx.try_load_texture(
                    &uri,
                    egui::TextureOptions::default(),
                    egui::SizeHint::default()
                ),
                Ok(egui::load::TexturePoll::Ready { .. })
            ) {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the sample picture did not decode"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        // Let the decoded dimensions reach the transcript's cached row layout.
        for _ in 0..30 {
            frame(&mut app);
        }
        let settled = frame(&mut app);
        for _ in 0..6 {
            assert_eq!(
                frame(&mut app),
                settled,
                "the transcript moved without input ({after} rows below, {height} tall)"
            );
        }
        settled
    }

    #[test]
    fn a_picture_across_the_top_edge_keeps_a_chat_at_its_end_still() {
        // Somewhere in this range the picture straddles the top edge.
        for height in (480..=720).step_by(20) {
            let shown = chat_with_picture(2, height as f32);
            assert!(!shown.is_empty(), "the chat shows its end ({height} tall)");
        }
    }
}

#[cfg(test)]
mod bubble_detail_tests {
    use super::tests::app;
    use super::*;

    /// Runs frames at `size` and returns the last frame's shapes.
    fn frames(
        app: &mut App,
        ctx: &egui::Context,
        size: egui::Vec2,
    ) -> Vec<egui::epaint::ClippedShape> {
        let mut shapes = Vec::new();
        for _ in 0..3 {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                    ..Default::default()
                },
                |ui| {
                    let ctx = ui.ctx().clone();
                    app.background_frame(&ctx);
                    app.frame_ui(ui);
                },
            );
            output.textures_delta.clear();
            shapes = output.shapes;
        }
        shapes
    }

    fn rect(ctx: &egui::Context, id: egui::Id) -> Option<egui::Rect> {
        ctx.data(|data| data.get_temp::<egui::Rect>(id))
    }

    /// The `bubbles` page in both Messages palettes: every message of the
    /// sample is drawn, and only each run's last balloon curls a tail out of
    /// its side, as far as Apple's reaches at ZapFast's text size; a lone
    /// photo leaves the tail to the balloon after it. All of it fits a 1280
    /// by 800 window.
    #[test]
    fn messages_balloons_tail_only_the_last_of_each_run() {
        // These runs belong to one day. Near midnight a real clock puts a
        // day boundary between them, correctly giving another bubble a tail.
        let _clock = crate::util::fixed_clock::set(crate::util::fixed_clock::Clock {
            // 14:00 UTC on Wednesday 11 March 2026.
            now: 1_773_237_600,
            zone: jiff::tz::TimeZone::UTC,
            twelve_hour: false,
        });
        const SIZE: egui::Vec2 = egui::vec2(1280.0, 800.0);
        let reach = crate::ui::widgets::balloon::TIP[0] * crate::ui::widgets::MESSAGES_SCALE;
        for page in ["bubbles,messages", "bubbles,messages-light"] {
            let mut app = app();
            let ctx = egui::Context::default();
            app.attach(&ctx);
            apply_flags(&mut app, Some(page));
            let shapes = frames(&mut app, &ctx, SIZE);
            let chat = SAMPLES[0].id;
            let meshes: Vec<egui::Rect> = shapes
                .iter()
                .filter_map(|clipped| match &clipped.shape {
                    egui::Shape::Mesh(mesh) if mesh.texture_id == egui::TextureId::default() => {
                        Some(egui::Rect::from_points(
                            &mesh.vertices.iter().map(|v| v.pos).collect::<Vec<_>>(),
                        ))
                    }
                    _ => None,
                })
                .collect();
            let out_past = |id: &str, own: bool| {
                let bubble = rect(
                    &ctx,
                    crate::ui::conversation::bubble_id(chat, id).with("rect"),
                )
                .unwrap_or_else(|| panic!("{page}: {id} is drawn"));
                let balloon = meshes
                    .iter()
                    .find(|mesh| {
                        (mesh.top() - bubble.top()).abs() < 1.5
                            && (mesh.bottom() - bubble.bottom()).abs() < 1.5
                            && mesh.contains(bubble.center())
                    })
                    .unwrap_or_else(|| panic!("{page}: {id} has a balloon"));
                if own {
                    balloon.right() - bubble.right()
                } else {
                    bubble.left() - balloon.left()
                }
            };
            for (id, own, tailed) in [
                ("bubbles-in-1", false, false),
                ("bubbles-in-2", false, true),
                ("bubbles-out-1", true, false),
                ("bubbles-out-2", true, false),
                ("bubbles-out-3", true, true),
                ("bubbles-in-4", false, true),
                ("bubbles-out-4", true, true),
            ] {
                let past = out_past(id, own);
                if tailed {
                    assert!(
                        (past - reach).abs() < 1.0,
                        "{page}: {id}'s tail reaches {past}, not {reach}"
                    );
                } else {
                    assert!(past < 1.0, "{page}: {id} has a tail ({past})");
                }
            }
            // Every row, the lone photo included, shows in the window: the
            // view is not scrolled past the first runs.
            for id in [
                "bubbles-in-1",
                "bubbles-in-2",
                "bubbles-out-1",
                "bubbles-out-2",
                "bubbles-out-3",
                "bubbles-in-3",
                "bubbles-in-4",
                "bubbles-out-4",
            ] {
                let shown = rect(
                    &ctx,
                    crate::ui::conversation::bubble_id(chat, id).with("rect"),
                )
                .unwrap_or_else(|| panic!("{page}: {id} is drawn"));
                assert!(
                    shown.top() >= 0.0 && shown.bottom() <= SIZE.y,
                    "{page}: {id} is off the 1280 by 800 window at {shown:?}"
                );
            }
        }
    }

    fn photos() -> (App, egui::Context) {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        apply_flags(&mut app, Some("photos"));
        frames(&mut app, &ctx, egui::vec2(1180.0, 1400.0));
        (app, ctx)
    }

    /// Their voice message carries a dot until it is heard; ours never does.
    #[test]
    fn an_unheard_voice_message_has_a_dot() {
        for (from_me, status, dot) in [
            (false, Delivery::None, true),
            (false, Delivery::Played, false),
            (true, Delivery::Read, false),
        ] {
            let mut app = app();
            apply_flags(&mut app, Some("audio-types"));
            let chat = app.open_chat.clone().unwrap();
            for row in &mut app.conversations.get_mut(&chat).unwrap().messages {
                row.from_me = from_me;
                row.status = status;
            }
            let ctx = egui::Context::default();
            app.attach(&ctx);
            frames(&mut app, &ctx, egui::vec2(1180.0, 780.0));
            let bubble = crate::ui::conversation::bubble_id(&chat, "audio-original");
            let shown = rect(&ctx, bubble.with("unheard"));
            assert_eq!(shown.is_some(), dot, "from me: {from_me}, {status:?}");
            if let Some(shown) = shown {
                let frame = rect(&ctx, bubble.with("rect")).expect("voice bubble drawn");
                assert!(frame.contains_rect(shown), "the dot is inside its bubble");
            }
            assert!(
                rect(
                    &ctx,
                    crate::ui::conversation::bubble_id(&chat, "audio-file").with("unheard")
                )
                .is_none(),
                "an audio file has no dot"
            );
        }
    }

    /// Both audio types keep their own player treatment and forwarding label
    /// across themes, bubble sides, download states, and narrow layouts.
    #[test]
    fn audio_files_have_a_track_and_music_badge_independent_of_forwarding() {
        for light in [false, true] {
            for from_me in [false, true] {
                for downloaded in [false, true] {
                    for width in [1180.0, 560.0] {
                        let mut app = app();
                        apply_flags(
                            &mut app,
                            Some(if light {
                                "audio-types,light"
                            } else {
                                "audio-types"
                            }),
                        );
                        let chat = app.open_chat.clone().unwrap();
                        let audio_path = app
                            .account()
                            .dirs
                            .media_cache_dir()
                            .join("synthetic-audio.ogg");
                        for row in &mut app.conversations.get_mut(&chat).unwrap().messages {
                            row.from_me = from_me;
                            if downloaded && let Content::Audio { media, .. } = &mut row.content {
                                media.path = Some(audio_path.clone());
                            }
                        }
                        let ctx = egui::Context::default();
                        app.attach(&ctx);
                        frames(&mut app, &ctx, egui::vec2(width, 780.0));
                        for (id, voice, forwarded) in [
                            ("audio-original", true, false),
                            ("audio-file", false, false),
                            ("audio-forwarded", false, true),
                        ] {
                            let bubble = crate::ui::conversation::bubble_id(&chat, id);
                            let frame =
                                rect(&ctx, bubble.with("rect")).expect("audio bubble drawn");
                            assert_eq!(rect(&ctx, bubble.with("voice-waveform")).is_some(), voice);
                            assert_eq!(rect(&ctx, bubble.with("audio-track")).is_some(), !voice);
                            let badge = rect(&ctx, bubble.with("audio-file"));
                            assert_eq!(badge.is_some(), !voice);
                            if let Some(badge) = badge {
                                assert!(frame.contains_rect(badge), "music badge fits in bubble");
                            }
                            let chip =
                                rect(&ctx, crate::ui::conversation::speed_chip_id(&chat, id));
                            assert_eq!(
                                chip.is_some(),
                                downloaded,
                                "playable audio retains speed control"
                            );
                            if let Some(chip) = chip {
                                assert!(frame.contains_rect(chip), "speed control fits in bubble");
                            }
                            assert_eq!(rect(&ctx, bubble.with("forwarded")).is_some(), forwarded);
                        }
                    }
                }
            }
        }
    }

    /// "Forwarded" sits at the start of the bubble, just under its top, on
    /// incoming and own messages alike (#250).
    #[test]
    fn the_forwarded_label_starts_at_the_bubble_start_on_both_sides() {
        let (_app, ctx) = photos();
        let chat = SAMPLES[0].id;
        for id in ["photos-forwarded-in", "photos-forwarded-out", "photos-in"] {
            let bubble = crate::ui::conversation::bubble_id(chat, id);
            let frame = rect(&ctx, bubble.with("rect")).expect("the bubble was drawn");
            let label = rect(&ctx, bubble.with("forwarded")).expect("the label was drawn");
            assert!(
                (label.left() - (frame.left() + 10.0)).abs() < 0.5,
                "{id}: the label starts at the bubble's start ({label:?} in {frame:?})"
            );
            assert!(
                label.top() - frame.top() <= 7.0,
                "{id}: the label sits just under the bubble's top ({label:?} in {frame:?})"
            );
        }
        let bubble = crate::ui::conversation::bubble_id(chat, "photos-in");
        let label = rect(&ctx, bubble.with("forwarded")).unwrap();
        let picture = rect(&ctx, bubble.with("picture")).expect("the picture was drawn");
        assert!(
            picture.top() - label.bottom() <= 3.5,
            "the picture follows the label closely ({label:?}, {picture:?})"
        );
    }

    /// A picture without a caption carries its time and ticks over its
    /// bottom corner, and its bubble closes just under it; with a caption
    /// they stay on the caption's line below the picture (#251).
    #[test]
    fn a_picture_without_a_caption_carries_its_time_over_its_corner() {
        let (_app, ctx) = photos();
        let chat = SAMPLES[0].id;
        for id in ["photos-in", "photos-out"] {
            let bubble = crate::ui::conversation::bubble_id(chat, id);
            let frame = rect(&ctx, bubble.with("rect")).expect("the bubble was drawn");
            let picture = rect(&ctx, bubble.with("picture")).expect("the picture was drawn");
            let footer = rect(&ctx, crate::ui::conversation::footer_id(chat, id))
                .expect("the time was drawn");
            assert!(
                picture.contains_rect(footer),
                "{id}: the time is over the picture ({footer:?} in {picture:?})"
            );
            assert!(
                picture.right() - footer.right() <= 12.0
                    && picture.bottom() - footer.bottom() <= 8.0,
                "{id}: the time sits in the picture's bottom corner ({footer:?} in {picture:?})"
            );
            assert!(
                frame.bottom() - picture.bottom() <= 7.0,
                "{id}: no strip under the picture ({picture:?} in {frame:?})"
            );
            assert!(
                (frame.left() + 10.0 - picture.left()).abs() < 0.5
                    && (frame.right() - 10.0 - picture.right()).abs() < 0.5,
                "{id}: the bubble wraps the picture ({picture:?} in {frame:?})"
            );
        }
        let bubble = crate::ui::conversation::bubble_id(chat, "photos-caption");
        let picture = rect(&ctx, bubble.with("picture")).expect("the captioned picture");
        let footer = rect(
            &ctx,
            crate::ui::conversation::footer_id(chat, "photos-caption"),
        )
        .expect("the captioned picture's time");
        assert!(
            footer.top() >= picture.bottom(),
            "with a caption the time stays below the picture ({footer:?}, {picture:?})"
        );
    }

    /// A voice message carries its time and ticks on the duration's line,
    /// as on the phone, rather than on a line of their own under it.
    #[test]
    fn a_voice_message_keeps_its_time_on_the_duration_line() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        apply_flags(&mut app, Some("voice"));
        frames(&mut app, &ctx, egui::vec2(1180.0, 1400.0));
        let chat = SAMPLES[0].id;
        for id in ["ada-voice", "you-voice"] {
            let frame = rect(
                &ctx,
                crate::ui::conversation::bubble_id(chat, id).with("rect"),
            )
            .expect("the bubble was drawn");
            let footer = rect(&ctx, crate::ui::conversation::footer_id(chat, id))
                .expect("the time was drawn");
            assert!(
                frame.contains_rect(footer),
                "{id}: the time is inside the bubble ({footer:?} in {frame:?})"
            );
            assert!(
                frame.bottom() - footer.bottom() <= 8.0,
                "{id}: the bubble closes under the time ({footer:?} in {frame:?})"
            );
            assert!(
                frame.height() <= 60.0,
                "{id}: no line of its own for the time ({footer:?} in {frame:?})"
            );
        }
    }

    /// A chat with unsent text shows it in its row, after an accent
    /// "Draft:", while the open chat's text stays in the composer (#245).
    #[test]
    fn a_chat_with_a_draft_shows_it_in_the_list() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        apply_flags(&mut app, Some("drafts"));
        let ordered = app.visible_chats();
        assert_eq!(ordered[0].id, SAMPLES[0].id, "the pin stays above drafts");
        assert_eq!(ordered[1].id, SAMPLES[2].id, "the newer draft follows pins");
        assert_eq!(
            ordered[2].id, SAMPLES[3].id,
            "drafts precede newer messages"
        );
        let shapes = frames(&mut app, &ctx, egui::vec2(1180.0, 780.0));
        let texts: Vec<(String, egui::Color32)> = shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                egui::Shape::Text(text) => Some((
                    text.galley.text().to_owned(),
                    text.galley
                        .job
                        .sections
                        .first()
                        .map_or(text.fallback_color, |section| section.format.color),
                )),
                _ => None,
            })
            .collect();
        let labels: Vec<_> = texts
            .iter()
            .filter(|(text, _)| text.trim() == "Draft:")
            .collect();
        assert_eq!(labels.len(), 2, "two rows carry a draft: {texts:?}");
        assert!(
            labels.iter().all(|(_, color)| *color == app.palette.accent),
            "the label is in the accent: {labels:?}"
        );
        assert!(
            texts
                .iter()
                .any(|(text, _)| text.starts_with("Sounds good, see you at")),
            "a draft reads on one line: {texts:?}"
        );
        assert_eq!(
            app.draft_preview(SAMPLES[2].id).as_deref(),
            Some("Sounds good, see you at the station")
        );
        assert_eq!(
            app.draft_preview(SAMPLES[0].id),
            None,
            "the open chat's text is in the composer"
        );
        app.drafts.insert(SAMPLES[1].id.into(), " \n ".into());
        assert_eq!(
            app.draft_preview(SAMPLES[1].id),
            None,
            "blank text is no draft"
        );
    }

    /// In a wide window the settings keep their width and sit in the
    /// middle of the page instead of against its left edge (#242).
    #[test]
    fn the_settings_column_is_centred_in_a_wide_window() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        apply_flags(&mut app, Some("settings"));
        let column = |ctx: &egui::Context| {
            ctx.data(|data| data.get_temp::<egui::Rangef>(crate::ui::settings::column_id()))
                .expect("the settings were drawn")
        };
        let mut columns = Vec::new();
        for width in [1400.0, 1800.0] {
            frames(&mut app, &ctx, egui::vec2(width, 900.0));
            let drawn = column(&ctx);
            assert!((drawn.span() - 640.0).abs() < 0.5, "{drawn:?}");
            columns.push(drawn);
        }
        let moved = columns[1].center() - columns[0].center();
        assert!(
            (moved - 200.0).abs() < 1.0,
            "the column follows the middle of the page: moved {moved}"
        );
        // A narrow page keeps the usual margins instead.
        frames(&mut app, &ctx, egui::vec2(900.0, 900.0));
        assert!(column(&ctx).span() < 640.0, "{:?}", column(&ctx));
    }

    /// The background history row's title fits on its line in every
    /// language, instead of ending in an ellipsis. Checked in a window
    /// narrower than the default (where the longer earlier title was already
    /// cut short here), so a wider system font still has room.
    #[test]
    fn the_older_history_title_fits_in_every_language() {
        /// Every text drawn, and whether it was cut short with an ellipsis.
        fn texts(shape: &egui::Shape, out: &mut Vec<(String, bool)>) {
            match shape {
                egui::Shape::Text(text) => {
                    out.push((text.galley.text().to_owned(), text.galley.elided))
                }
                egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| texts(shape, out)),
                _ => {}
            }
        }
        for locale in crate::i18n::Locale::ALL {
            let mut app = app();
            let ctx = egui::Context::default();
            app.attach(&ctx);
            let title = crate::i18n::gettext(locale, "Fetch older history").into_owned();
            apply_flags(&mut app, Some("settings"));
            app.locale = locale;
            app.settings_search = title.clone();
            let mut drawn = Vec::new();
            // The rail takes 60 points of the old 900.
            for clipped in &frames(&mut app, &ctx, egui::vec2(960.0, 780.0)) {
                texts(&clipped.shape, &mut drawn);
            }
            let titles: Vec<bool> = drawn
                .iter()
                .filter(|(text, _)| *text == title)
                .map(|(_, elided)| *elided)
                .collect();
            assert!(!titles.is_empty(), "{locale:?}: {title:?} is not drawn");
            assert!(
                !titles.contains(&true),
                "{locale:?}: {title:?} is cut short"
            );
        }
    }
}

/// The call screen as a window: what opens it, what puts it aside without ending the call, and the
/// one control that does end it.
#[cfg(test)]
mod call_surface_tests {
    use super::tests::{app, frame_with};
    use super::*;
    use crate::model::Action;

    /// A key press, as egui reports one.
    fn key(key: egui::Key) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }
    }

    /// A running call takes the screen, with the window attached to it.
    fn on_a_call() -> (App, egui::Context) {
        let mut app = app();
        apply_flags(&mut app, Some("call-active"));
        let ctx = egui::Context::default();
        app.attach(&ctx);
        (app, ctx)
    }

    /// The labels one frame publishes to a screen reader, after the surface has settled.
    fn labels(app: &mut App, ctx: &egui::Context) -> Vec<String> {
        labelled(app, ctx)
            .into_iter()
            .map(|(label, _)| label)
            .collect()
    }

    /// The labels one frame publishes, with whether each control is disabled.
    fn labelled(app: &mut App, ctx: &egui::Context) -> Vec<(String, bool)> {
        let mut labels = Vec::new();
        for _ in 0..3 {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1180.0, 780.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    let ctx = ui.ctx().clone();
                    app.background_frame(&ctx);
                    app.frame_ui(ui);
                },
            );
            output.textures_delta.clear();
            labels = output
                .platform_output
                .accesskit_update
                .expect("accessibility tree")
                .nodes
                .iter()
                .filter_map(|(_, node)| {
                    node.label()
                        .map(|label| (label.to_owned(), node.is_disabled()))
                })
                .collect();
        }
        labels
    }

    #[test]
    fn escape_leaves_full_screen_before_it_puts_the_call_aside() {
        let (mut app, ctx) = on_a_call();
        app.actions.push(Action::ToggleCallFullscreen);
        frame_with(&mut app, &ctx, Vec::new());
        assert!(app.call_fullscreen, "the surface takes the whole screen");
        assert!(!app.call_surface_hidden);

        frame_with(&mut app, &ctx, vec![key(egui::Key::Escape)]);
        assert!(!app.call_fullscreen, "escape leaves full screen");
        assert!(
            !app.call_surface_hidden,
            "and the call screen is still the screen"
        );
        assert_eq!(
            app.call.as_ref().map(|call| call.phase),
            Some(CallPhase::Active),
            "a window state is not a call state"
        );

        frame_with(&mut app, &ctx, vec![key(egui::Key::Escape)]);
        assert!(
            app.call_surface_hidden,
            "a second escape is the same move the back button makes"
        );
        assert_eq!(
            app.call.as_ref().map(|call| call.phase),
            Some(CallPhase::Active),
            "and the call runs on behind the bar"
        );
    }

    #[test]
    fn only_the_hang_up_button_ends_a_call() {
        let (mut app, ctx) = on_a_call();
        app.backend.record_demo_commands();
        app.actions.push(Action::ToggleCallFullscreen);
        frame_with(&mut app, &ctx, Vec::new());
        // Everything a reader can do to the window short of hanging up.
        frame_with(&mut app, &ctx, vec![key(egui::Key::Escape)]);
        frame_with(&mut app, &ctx, vec![key(egui::Key::Escape)]);
        app.actions.push(Action::ReturnToCall);
        frame_with(&mut app, &ctx, Vec::new());
        app.actions.push(Action::ToggleCallFullscreen);
        frame_with(&mut app, &ctx, Vec::new());
        frame_with(&mut app, &ctx, vec![key(egui::Key::Escape)]);
        let hang_ups = |commands: Vec<crate::backend::Command>| {
            commands
                .into_iter()
                .filter(|command| matches!(command, crate::backend::Command::HangupCall))
                .count()
        };
        assert_eq!(
            hang_ups(app.backend.take_demo_commands()),
            0,
            "stepping away, returning, and full screen never hang up"
        );
        app.actions.push(Action::HangupCall);
        frame_with(&mut app, &ctx, Vec::new());
        assert_eq!(
            hang_ups(app.backend.take_demo_commands()),
            1,
            "the hang-up button is the one that ends the call"
        );
    }

    #[test]
    fn an_incoming_call_stays_on_screen_until_it_is_answered_or_declined() {
        let mut app = app();
        apply_flags(&mut app, Some("call"));
        let ctx = egui::Context::default();
        app.attach(&ctx);
        app.backend.record_demo_commands();
        // Escape is not an answer and not a refusal, so the ringing call stays where it can be
        // answered.
        frame_with(&mut app, &ctx, vec![key(egui::Key::Escape)]);
        assert!(!app.call_surface_hidden, "the ringing call is still there");
        assert_eq!(
            app.call.as_ref().map(|call| call.phase),
            Some(CallPhase::Incoming)
        );
        assert!(
            !app.backend
                .take_demo_commands()
                .iter()
                .any(|command| matches!(
                    command,
                    crate::backend::Command::AnswerCall | crate::backend::Command::DeclineCall
                )),
            "escape neither answers nor declines"
        );
    }

    #[test]
    fn the_bar_brings_the_call_screen_back() {
        let (mut app, ctx) = on_a_call();
        ctx.enable_accesskit();
        app.actions.push(Action::LeaveCallSurface);
        let bar = labels(&mut app, &ctx);
        assert!(
            bar.contains(&"Return to the call".to_owned()),
            "the bar offers the way back: {bar:?}"
        );
        assert!(
            !bar.contains(&"Hide the call screen".to_owned()),
            "and the surface that was put aside is not on screen: {bar:?}"
        );
        assert_eq!(
            app.call.as_ref().map(|call| call.phase),
            Some(CallPhase::Active),
            "the bar is only ever a way back to a call that is still up"
        );

        app.actions.push(Action::ReturnToCall);
        let screen = labels(&mut app, &ctx);
        assert!(
            screen.contains(&"Hide the call screen".to_owned()),
            "the surface comes back: {screen:?}"
        );
        assert!(
            !screen.contains(&"Return to the call".to_owned()),
            "and the bar goes with it: {screen:?}"
        );
        assert_eq!(
            app.call.as_ref().map(|call| call.phase),
            Some(CallPhase::Active),
            "returning is not a new call"
        );
    }

    /// The bar is a way back to a call that is still up, so it goes when the call does.
    #[test]
    fn the_bar_leaves_with_the_call_it_belongs_to() {
        let (mut app, ctx) = on_a_call();
        ctx.enable_accesskit();
        app.actions.push(Action::LeaveCallSurface);
        frame_with(&mut app, &ctx, Vec::new());
        let bar = labels(&mut app, &ctx);
        assert!(
            bar.contains(&"Return to the call".to_owned()),
            "the call is still up behind the bar: {bar:?}"
        );

        let call = app.call.as_mut().expect("a call is up");
        call.phase = CallPhase::Ended;
        call.outcome = Some(crate::calls::CallOutcome::Answered);
        let after = labels(&mut app, &ctx);
        assert!(
            !after.contains(&"Return to the call".to_owned()),
            "a call that is over has no way back: {after:?}"
        );
    }

    /// A locked chat's farewell, and any call the reader put aside, still ends behind the bar.
    ///
    /// `call_surface_hidden` is set for a locked call's terminal snapshot, but the surface only
    /// looked at the bar while the phase was live, so the four-second farewell painted the whole
    /// window once more and announced that a hidden chat had a call. A hidden snapshot stays behind
    /// the bar whatever the phase.
    #[test]
    fn a_hidden_farewell_stays_behind_the_bar() {
        let (mut app, ctx) = on_a_call();
        ctx.enable_accesskit();
        app.call_surface_hidden = true;
        let call = app.call.as_mut().expect("a call is up");
        call.phase = CallPhase::Ended;
        call.outcome = Some(crate::calls::CallOutcome::Answered);
        let shown = labels(&mut app, &ctx);
        for absent in ["Full screen", "Mute", "Hang up"] {
            assert!(
                !shown.contains(&absent.to_owned()),
                "a hidden farewell must not paint the call surface: {absent} in {shown:?}"
            );
        }
    }

    /// A call that is over offers no control: the backend has already released the runtime.
    ///
    /// The farewell keeps the controls it was drawn with so the outcome reads where the call was,
    /// but the red Hang up stayed enabled, so a click sent a command nobody was listening for and
    /// the screen offered an action for a call that had ended.
    #[test]
    fn the_farewell_offers_no_action_for_a_call_that_is_over() {
        let (mut app, ctx) = on_a_call();
        ctx.enable_accesskit();
        let call = app.call.as_mut().expect("a call is up");
        call.phase = CallPhase::Ended;
        call.outcome = Some(crate::calls::CallOutcome::Answered);
        let shown = labelled(&mut app, &ctx);
        for tip in ["Mute", "Hang up"] {
            let control = shown.iter().find(|(label, _)| label == tip);
            assert!(
                matches!(control, Some((_, true))),
                "{tip} is disabled once the call is over: {shown:?}"
            );
        }
    }

    /// A group's chat offers a voice and a video call in its header, as a person's does, while
    /// group calls are on; a channel's offers neither.
    #[test]
    fn a_groups_header_offers_calls() {
        let _on = crate::calls::override_group_calls(true);
        let (mut app, ctx) = header_app();
        let shown = labels(&mut app, &ctx);
        for expected in ["Voice call", "Video call"] {
            assert!(
                shown.contains(&expected.to_owned()),
                "{expected} is in the group's header: {shown:?}"
            );
        }

        let channel = SAMPLES
            .iter()
            .find(|sample| sample.id.ends_with("@newsletter"))
            .unwrap()
            .id;
        app.open_chat = Some(channel.into());
        let shown = labels(&mut app, &ctx);
        assert!(
            !shown.contains(&"Voice call".to_owned()),
            "a channel has no call button: {shown:?}"
        );
    }

    /// With group calls off, a group's header has no call buttons, and a person's keeps them.
    #[test]
    fn with_group_calls_off_a_groups_header_offers_no_calls() {
        let _off = crate::calls::override_group_calls(false);
        let (mut app, ctx) = header_app();
        let shown = labels(&mut app, &ctx);
        for absent in ["Voice call", "Video call"] {
            assert!(
                !shown.contains(&absent.to_owned()),
                "{absent} is not in the group's header: {shown:?}"
            );
        }
        let person = SAMPLES
            .iter()
            .find(|sample| sample.id.ends_with("@s.whatsapp.net"))
            .unwrap()
            .id;
        app.open_chat = Some(person.into());
        let shown = labels(&mut app, &ctx);
        assert!(
            shown.contains(&"Voice call".to_owned()),
            "a person's chat still calls: {shown:?}"
        );
    }

    /// The demo with a group's chat open, ready to read its header's labels.
    fn header_app() -> (App, egui::Context) {
        let mut app = app();
        let group = SAMPLES
            .iter()
            .find(|sample| sample.id.ends_with("@g.us"))
            .unwrap()
            .id;
        app.open_chat = Some(group.into());
        let ctx = egui::Context::default();
        app.attach(&ctx);
        ctx.enable_accesskit();
        (app, ctx)
    }

    /// A group call that only the phone can answer leaves the screen alone and says so, naming
    /// who is calling which group.
    #[test]
    fn a_group_call_on_the_phone_only_is_a_notice() {
        let mut app = app();
        apply_flags(&mut app, Some("call-group-on-phone"));
        assert!(app.call.is_none(), "no call screen");
        let notice = app
            .toasts
            .iter()
            .find(|toast| toast.message.contains("answer on your phone"))
            .expect("the window says so");
        let group = app.chat_title_of(SAMPLES[1].id);
        assert!(notice.message.contains(&group), "{}", notice.message);
    }

    /// A call we started in a group is drawn as the group's, ringing everyone on it: nobody has
    /// joined, so each reads as invited, and nobody is named as having rung.
    #[test]
    fn a_group_call_we_started_rings_its_members() {
        let mut app = app();
        apply_flags(&mut app, Some("call-group-calling"));
        assert_eq!(app.call_name(), "Rust Berlin");
        assert_eq!(app.call_caller_name(), None);
        assert!(app.call_people().iter().all(|person| !person.joined));
        let ctx = egui::Context::default();
        app.attach(&ctx);
        ctx.enable_accesskit();
        let shown = labels(&mut app, &ctx);
        assert!(shown.contains(&"Hang up".to_owned()), "{shown:?}");
    }

    /// A group's call rings under the group's name and says who started it, and once up it has a
    /// tile for everyone on its roster, saying who has not joined. A voice call in a group offers
    /// no camera: WhatsApp takes video only in a group call that began as one.
    #[test]
    fn a_group_call_names_its_group_who_rang_and_who_is_on_it() {
        let mut ringing = app();
        apply_flags(&mut ringing, Some("call-group"));
        assert_eq!(ringing.call_name(), "Rust Berlin", "named by its group");
        let caller = ringing.call_caller_name();
        assert_eq!(caller.as_deref(), Some("Jonas"));
        let call = ringing.call.clone().unwrap();
        assert_eq!(
            crate::ui::call::incoming_text(ringing.locale, &call, caller.as_deref()),
            "Group voice call from Jonas"
        );

        let mut running = app();
        apply_flags(&mut running, Some("call-group-active"));
        let people: Vec<(String, bool)> = running
            .call_people()
            .into_iter()
            .map(|person| (person.name, person.joined))
            .collect();
        assert_eq!(
            people,
            [
                ("Jonas".to_owned(), true),
                ("Mira".to_owned(), true),
                ("Tom".to_owned(), true),
                ("Ada Lovelace".to_owned(), true),
                ("Grace Hopper".to_owned(), false),
            ]
        );
        let ctx = egui::Context::default();
        running.attach(&ctx);
        ctx.enable_accesskit();
        let shown = labels(&mut running, &ctx);
        for expected in ["Mute", "Devices", "Hang up"] {
            assert!(
                shown.contains(&expected.to_owned()),
                "{expected} is on the call screen: {shown:?}"
            );
        }
        assert!(
            !shown.contains(&"Start video".to_owned()),
            "a group voice call offers no camera: {shown:?}"
        );

        // A locked group's call names nobody on it.
        let group = running.call.as_ref().unwrap().chat.clone();
        for chat in running.chats.iter_mut().filter(|chat| chat.id == group) {
            chat.locked = true;
        }
        running.settings.set_chat_lock_code(Some("test-code"));
        assert!(running.call_people().is_empty());
        assert_eq!(
            running.call_self().map(|me| me.name).as_deref(),
            Some("You"),
            "our own tile is ours to name"
        );
        assert_eq!(running.call_caller_name(), None);
    }

    #[test]
    fn the_call_screen_labels_every_control_it_offers() {
        let (mut running, ctx) = on_a_call();
        ctx.enable_accesskit();
        let screen = labels(&mut running, &ctx);
        for expected in [
            "Hide the call screen",
            "Full screen",
            "Mute",
            "Start video",
            "Devices",
            "Hang up",
        ] {
            assert!(
                screen.contains(&expected.to_owned()),
                "missing accessible label: {expected}"
            );
        }
        // The same controls say what they do when pressed, not just which button they are.
        running.actions.push(Action::ToggleCallFullscreen);
        let full = labels(&mut running, &ctx);
        assert!(
            full.contains(&"Exit full screen".to_owned()),
            "the full-screen control says how to leave: {full:?}"
        );

        let mut ringing = app();
        apply_flags(&mut ringing, Some("call"));
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        ringing.attach(&ctx);
        let incoming = labels(&mut ringing, &ctx);
        for expected in ["Accept", "Decline"] {
            assert!(
                incoming.contains(&expected.to_owned()),
                "missing accessible label: {expected}"
            );
        }
    }
}

/// The call surface's previews: what they hand it, and the picture they hand it with.
#[cfg(test)]
mod call_preview_tests {
    use super::tests::app;
    use super::*;

    #[test]
    fn the_call_previews_hand_the_surface_a_live_call() {
        let mut screen = app();
        call_sample(&mut screen, true, false);
        let incoming = screen.call.clone().expect("a call is on screen");
        assert_eq!(incoming.phase, CallPhase::Incoming);
        assert_eq!(incoming.direction, CallDirection::Incoming);
        assert!(incoming.phase.is_live(), "a ringing call is still a call");
        assert!(
            !incoming.video,
            "the ringing preview is the voice one, so the two do not read alike"
        );

        let mut screen = app();
        call_sample(&mut screen, false, true);
        let live = screen.call.clone().expect("a call is on screen");
        assert_eq!(live.phase, CallPhase::Active);
        assert_eq!(live.direction, CallDirection::Outgoing);
        assert!(
            live.started.is_some(),
            "the duration has something to count from"
        );
        assert!(live.remote_video && live.camera_on);
        assert!(
            screen.call_remote_frame.is_some() && screen.call_local_frame.is_some(),
            "a video preview has both pictures"
        );
        assert_eq!(
            screen.call_remote_frame.as_ref().map(|frame| frame.size),
            Some([540, 720]),
            "the peer's picture keeps the shape a portrait phone sends"
        );
    }
}

/// The navigation rail beside the chat list, and the Calls tab beside the
/// open chat.
#[cfg(test)]
mod rail_tests {
    use super::tests::app;
    use super::*;
    use crate::model::{Action, Page};
    use crate::ui::focus::{self, Stop};
    use crate::ui::rail::{self, Destination};

    /// One frame at `width` by 780 points, with the layout records of the
    /// last one cleared, so a part not drawn leaves none.
    fn frame(app: &mut App, ctx: &egui::Context, width: f32, events: Vec<egui::Event>) {
        frame_at(app, ctx, egui::vec2(width, 780.0), events);
    }

    /// One frame at `size`, as [`frame`]; returns the text it painted.
    fn frame_at(
        app: &mut App,
        ctx: &egui::Context,
        size: egui::Vec2,
        events: Vec<egui::Event>,
    ) -> Vec<String> {
        ctx.data_mut(|data| {
            data.remove::<egui::Rect>(rail::rect_id());
            data.remove::<egui::Rect>(crate::ui::chats::header_row_id());
            data.remove::<egui::Rect>(crate::ui::conversation::header_row_id());
            for chat in &app.chats {
                data.remove::<egui::Rect>(crate::ui::chats::compact_chat_id(&chat.id));
            }
        });
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                events,
                ..Default::default()
            },
            |ui| {
                let ctx = ui.ctx().clone();
                app.background_frame(&ctx);
                app.frame_ui(ui);
            },
        );
        output.textures_delta.clear();
        fn texts(shape: &egui::Shape, out: &mut Vec<String>) {
            match shape {
                egui::Shape::Text(text) => out.push(text.galley.text().to_owned()),
                egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| texts(shape, out)),
                _ => {}
            }
        }
        let mut drawn = Vec::new();
        for clipped in &output.shapes {
            texts(&clipped.shape, &mut drawn);
        }
        drawn
    }

    /// The demo on `page`, laid out wide, in the macOS chrome when `macos`.
    fn open(page: &str, macos: bool) -> (App, egui::Context) {
        let mut app = app();
        apply_flags(&mut app, Some(page));
        let ctx = egui::Context::default();
        app.attach(&ctx);
        if macos {
            crate::theme::preview_macos(&ctx);
        }
        for _ in 0..3 {
            frame(&mut app, &ctx, 1180.0, Vec::new());
        }
        (app, ctx)
    }

    fn recorded(ctx: &egui::Context, id: egui::Id) -> Option<egui::Rect> {
        ctx.data(|data| data.get_temp::<egui::Rect>(id))
    }

    fn rail_rect(ctx: &egui::Context) -> Option<egui::Rect> {
        recorded(ctx, rail::rect_id())
    }

    fn list_header(ctx: &egui::Context) -> Option<egui::Rect> {
        recorded(ctx, crate::ui::chats::header_row_id())
    }

    fn conversation_header(ctx: &egui::Context) -> Option<egui::Rect> {
        recorded(ctx, crate::ui::conversation::header_row_id())
    }

    /// Where the control registered for `stop` was drawn this frame.
    fn stop_rect(ctx: &egui::Context, stop: Stop) -> Option<egui::Rect> {
        focus::stops(ctx)
            .into_iter()
            .find(|(known, _)| *known == stop)
            .and_then(|(_, id)| ctx.read_response(id))
            .map(|response| response.rect)
    }

    fn destination_rect(ctx: &egui::Context, destination: Destination) -> egui::Rect {
        ctx.read_response(rail::destination_id(destination))
            .expect("the rail draws every destination")
            .rect
    }

    fn click(app: &mut App, ctx: &egui::Context, width: f32, pos: egui::Pos2) {
        frame(app, ctx, width, vec![egui::Event::PointerMoved(pos)]);
        for pressed in [true, false] {
            frame(
                app,
                ctx,
                width,
                vec![egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
        }
        frame(app, ctx, width, Vec::new());
    }

    /// Ctrl+Shift (Cmd+Shift on macOS) and the digit at `physical`, which
    /// the layout reports as `key` with Shift held.
    fn shifted(key: egui::Key, physical: egui::Key) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: Some(physical),
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
        }
    }

    fn escape() -> egui::Event {
        egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }
    }

    /// The rail stands at the window's left edge beside the list, with Chats
    /// and Calls at its top and Starred, Settings and our avatar at its foot;
    /// with the list hidden, the chats' avatars fill it between the two. On
    /// macOS it starts below the traffic lights.
    #[test]
    fn the_rail_carries_the_destinations_with_the_list_shown_or_hidden() {
        for macos in [false, true] {
            let (app, ctx) = open("chat", macos);
            let rail = rail_rect(&ctx).expect("the rail is drawn");
            assert!(rail.left() < 1.0, "{macos}: at the left edge: {rail:?}");
            assert!(
                (rail.width() - crate::ui::chats::compact_width(&ctx)).abs() < 1.0,
                "{macos}: as wide as the collapsed list was: {rail:?}"
            );
            let header = list_header(&ctx).expect("the list is drawn");
            assert!(
                header.left() >= rail.right(),
                "{macos}: the list is beside it"
            );
            let top = [Destination::Chats, Destination::Calls]
                .map(|destination| destination_rect(&ctx, destination));
            let starred = destination_rect(&ctx, Destination::Starred);
            let shown_starred = starred;
            let settings = destination_rect(&ctx, Destination::Settings);
            let shown_settings = settings;
            let me = stop_rect(&ctx, Stop::Profile).expect("our avatar is drawn");
            for rect in top.iter().chain([&starred, &settings, &me]) {
                assert!(rail.contains_rect(*rect), "{macos}: {rect:?} in {rail:?}");
            }
            assert!(top[0].bottom() <= top[1].top());
            // Starred heads the bottom group, a plate and a gap above
            // Archived, or Settings when nothing is archived or locked.
            let below = if Destination::Archived.offered(&app) {
                destination_rect(&ctx, Destination::Archived)
            } else {
                settings
            };
            assert!(
                (below.top() - starred.bottom() - 8.0).abs() < 0.5,
                "{macos}: Starred directly above the next bottom button: {starred:?} {below:?}"
            );
            assert!(
                starred.top() > top[1].bottom() + 100.0,
                "{macos}: at the foot"
            );
            assert_eq!(starred.size(), egui::vec2(36.0, 36.0));
            assert!(
                (me.top() - settings.bottom() - 8.0).abs() < 0.5
                    && (rail.bottom() - me.bottom() - 8.0).abs() < 0.5,
                "{macos}: Settings, our avatar and the edge 8 points apart: {settings:?} {me:?}"
            );
            assert_eq!(settings.size(), egui::vec2(36.0, 36.0));
            assert_eq!(me.size(), egui::vec2(32.0, 32.0));
            // A Mac lays out its own way whichever the preview says.
            if crate::theme::macos_chrome(&ctx) {
                assert!(
                    top[0].top() >= 36.0 && top[0].top() < 60.0,
                    "just below the traffic lights: {:?}",
                    top[0]
                );
                // The lights reach over the list header, whose title makes
                // way for them.
                assert!(header.left() + 14.0 + crate::ui::rail::lights_overhang(&ctx) >= 84.0);
            } else {
                // Chats sits level with the list header's title row.
                assert!((top[0].center().y - header.center().y).abs() < 3.0);
            }
            assert!(Destination::Chats.lit(&app));
            // Beside the list the rail does not repeat its chats.
            let first = app.visible_chats()[0].id.clone();
            assert!(
                recorded(&ctx, crate::ui::chats::compact_chat_id(&first)).is_none(),
                "{macos}: no avatars in the rail beside the list"
            );

            let (app, ctx) = open("rail", macos);
            let rail = rail_rect(&ctx).expect("the rail is drawn with the list hidden");
            assert!(list_header(&ctx).is_none(), "{macos}: the list is hidden");
            let chats = destination_rect(&ctx, Destination::Chats);
            let starred = destination_rect(&ctx, Destination::Starred);
            let settings = destination_rect(&ctx, Destination::Settings);
            let calls = destination_rect(&ctx, Destination::Calls);
            let show = stop_rect(&ctx, Stop::Sidebar).expect("Show the chat list");
            assert!(rail.contains_rect(show) && show.top() >= calls.bottom());
            // Nothing the list's toggle leaves in place moves.
            assert_eq!(chats, top[0], "{macos}: Chats stays put");
            assert_eq!(calls, top[1], "{macos}: Calls stays put");
            assert_eq!(starred, shown_starred, "{macos}: Starred stays put");
            assert_eq!(settings, shown_settings, "{macos}: Settings stays put");
            let first = app.visible_chats()[0].id.clone();
            let avatar = recorded(&ctx, crate::ui::chats::compact_chat_id(&first))
                .expect("the chats' avatars fill the rail");
            assert!(
                rail.contains_rect(avatar),
                "{macos}: {avatar:?} in {rail:?}"
            );
            assert!(
                avatar.top() >= calls.bottom() && avatar.bottom() <= starred.top(),
                "{macos}: between the top group and Starred: {avatar:?}"
            );
            // Under Show the chat list, which takes a cell of its own.
            assert!(avatar.top() >= show.bottom());
        }
    }

    /// At the minimum window height, Tab and Shift+Tab reveal every rail
    /// control, including the bottom group, without crossing the title bar.
    #[test]
    fn tab_reveals_every_rail_control_in_a_short_window() {
        for macos in [false, true] {
            for page in ["chat", "rail"] {
                for reverse in [false, true] {
                    let (mut app, ctx) = open(page, macos);
                    let size = egui::vec2(1180.0, 300.0);
                    for _ in 0..3 {
                        frame_at(&mut app, &ctx, size, Vec::new());
                    }
                    let mut expected = vec![
                        Stop::Chats,
                        Stop::Calls,
                        Stop::Starred,
                        Stop::Archived,
                        Stop::Locked,
                        Stop::Settings,
                        Stop::Profile,
                    ];
                    if page == "rail" {
                        expected.push(Stop::Sidebar);
                    }
                    for stop in &expected {
                        assert!(stop_rect(&ctx, *stop).is_some(), "{stop:?} is offered");
                    }
                    ctx.memory_mut(|memory| {
                        memory.request_focus(rail::destination_id(Destination::Chats));
                    });
                    let mut reached = std::collections::BTreeSet::new();
                    for _ in 0..40 {
                        frame_at(
                            &mut app,
                            &ctx,
                            size,
                            vec![egui::Event::Key {
                                key: egui::Key::Tab,
                                physical_key: None,
                                pressed: true,
                                repeat: false,
                                modifiers: if reverse {
                                    egui::Modifiers::SHIFT
                                } else {
                                    egui::Modifiers::NONE
                                },
                            }],
                        );
                        for _ in 0..3 {
                            frame_at(&mut app, &ctx, size, Vec::new());
                        }
                        let focused = ctx.memory(|memory| memory.focused());
                        let Some((stop, id)) = focus::stops(&ctx)
                            .into_iter()
                            .find(|(stop, id)| Some(*id) == focused && expected.contains(stop))
                        else {
                            continue;
                        };
                        let response = ctx.read_response(id).unwrap();
                        let mut viewport = rail_rect(&ctx).unwrap();
                        if crate::theme::traffic_light_inset(&ctx) > 0.0 {
                            viewport.min.y += crate::theme::TOP_BAR_HEIGHT / 2.0 + 12.0;
                        }
                        assert_eq!(
                            response.interact_rect, response.rect,
                            "{page}, macOS={macos}, reverse={reverse}: {stop:?} is fully visible"
                        );
                        assert!(
                            viewport.contains_rect(response.rect),
                            "{stop:?} stays in the rail below the title bar"
                        );
                        reached.insert(stop);
                    }
                    assert_eq!(reached, expected.into_iter().collect());
                }
            }
        }
    }

    /// Wheel scrolling reaches the account button at the minimum height;
    /// growing the window restores the usual top and bottom positions.
    #[test]
    fn a_short_rail_scrolls_and_resets_when_the_window_grows() {
        for macos in [false, true] {
            for page in ["chat", "rail"] {
                let (mut app, ctx) = open(page, macos);
                let chats = destination_rect(&ctx, Destination::Chats);
                let profile = stop_rect(&ctx, Stop::Profile).unwrap();
                let size = egui::vec2(1180.0, 300.0);
                for _ in 0..3 {
                    frame_at(&mut app, &ctx, size, Vec::new());
                }
                assert!(stop_rect(&ctx, Stop::Profile).unwrap().bottom() > size.y);
                let pointer = destination_rect(&ctx, Destination::Calls).center();
                frame_at(
                    &mut app,
                    &ctx,
                    size,
                    vec![
                        egui::Event::PointerMoved(pointer),
                        egui::Event::MouseWheel {
                            unit: egui::MouseWheelUnit::Point,
                            delta: egui::vec2(0.0, -600.0),
                            modifiers: egui::Modifiers::NONE,
                            phase: egui::TouchPhase::Move,
                        },
                    ],
                );
                for _ in 0..30 {
                    frame_at(&mut app, &ctx, size, Vec::new());
                }
                let id = focus::control(&ctx, Stop::Profile).unwrap();
                let response = ctx.read_response(id).unwrap();
                assert_eq!(
                    response.interact_rect, response.rect,
                    "{page}, macOS={macos}: the account button is fully visible after scrolling"
                );
                assert!(rail_rect(&ctx).unwrap().contains_rect(response.rect));
                for _ in 0..3 {
                    frame(&mut app, &ctx, 1180.0, Vec::new());
                }
                assert_eq!(destination_rect(&ctx, Destination::Chats), chats);
                assert_eq!(stop_rect(&ctx, Stop::Profile), Some(profile));
            }
        }
    }

    /// The Calls page's chips, All and Missed, are the chat list's own,
    /// under its search field and in its Tab order. Missed keeps only calls
    /// to us nobody took, a group's included; a search looks among them,
    /// and the choice is kept with the account.
    #[test]
    fn the_calls_chips_pick_missed_calls_and_combine_with_search() {
        use crate::model::CallFilter;
        use crate::ui::calls;
        let (mut app, ctx) = open("calls", false);
        let chip = |ctx: &egui::Context, filter| {
            recorded(ctx, calls::filter_chip_id(filter)).expect("the chip is drawn")
        };
        let (all, missed) = (chip(&ctx, CallFilter::All), chip(&ctx, CallFilter::Missed));
        let search = stop_rect(&ctx, Stop::Search).expect("the search field");
        assert!(all.top() >= search.bottom() && all.right() <= missed.left());
        assert_eq!(all.height(), 28.0, "the chat list's chip");
        assert_eq!(stop_rect(&ctx, Stop::All), Some(all));
        assert_eq!(stop_rect(&ctx, Stop::Missed), Some(missed));
        let rows: Vec<(Message, bool)> = calls::rows(&app.call_log)
            .into_iter()
            .map(|row| {
                let missed = calls::Entry::of(row[0]).unwrap().missed();
                (row[0].clone(), missed)
            })
            .collect();
        assert!(rows.iter().filter(|(_, missed)| *missed).count() >= 2);
        assert!(
            rows.iter().any(|(call, missed)| *missed
                && matches!(call.content, Content::Call { group: true, .. })),
            "a missed group call"
        );
        let drawn = |ctx: &egui::Context| -> Vec<String> {
            rows.iter()
                .filter(|(call, _)| {
                    ctx.read_response(calls::row_id(&call.chat, &call.id))
                        .is_some()
                })
                .map(|(call, _)| call.id.clone())
                .collect()
        };
        let ids = |keep: &dyn Fn(&Message, bool) -> bool| -> Vec<String> {
            rows.iter()
                .filter(|(call, missed)| keep(call, *missed))
                .map(|(call, _)| call.id.clone())
                .collect()
        };
        assert_eq!(drawn(&ctx), ids(&|_, _| true), "All lists every row");

        click(&mut app, &ctx, 1180.0, missed.center());
        // A response can come from the pass before the last.
        frame(&mut app, &ctx, 1180.0, Vec::new());
        assert_eq!(app.call_filter, CallFilter::Missed);
        assert_eq!(
            app.account().settings.call_filter.as_deref(),
            Some("missed")
        );
        assert_eq!(drawn(&ctx), ids(&|_, missed| missed), "only missed calls");

        // A search looks among the missed calls only.
        let ada = SAMPLES[0].id;
        app.calls_search = app.display_name(ada);
        frame(&mut app, &ctx, 1180.0, Vec::new());
        frame(&mut app, &ctx, 1180.0, Vec::new());
        assert_eq!(
            drawn(&ctx),
            ids(&|call, missed| missed && call.chat == ada),
            "Ada's missed calls"
        );
        let grace = SAMPLES[2].id;
        app.calls_search = app.display_name(grace);
        frame(&mut app, &ctx, 1180.0, Vec::new());
        frame(&mut app, &ctx, 1180.0, Vec::new());
        let texts = frame_at(&mut app, &ctx, egui::vec2(1180.0, 780.0), Vec::new());
        assert!(drawn(&ctx).is_empty(), "Grace has no missed call");
        assert!(texts.iter().any(|text| text == "No matching calls"));
        // The chips stay while searching: All brings Grace's call back.
        let missed = chip(&ctx, CallFilter::Missed);
        click(&mut app, &ctx, 1180.0, missed.center());
        assert_eq!(app.call_filter, CallFilter::All, "a second click is All");
        assert_eq!(app.account().settings.call_filter, None);
        frame(&mut app, &ctx, 1180.0, Vec::new());
        assert_eq!(drawn(&ctx), ids(&|call, _| call.chat == grace));
    }

    /// Missed with no missed call says so, and points back to All.
    #[test]
    fn missed_with_no_missed_calls_says_so() {
        use crate::model::CallFilter;
        use crate::ui::calls;
        let (mut app, ctx) = open("calls-missed-none", false);
        assert_eq!(app.call_filter, CallFilter::Missed);
        assert!(!app.call_log.is_empty(), "other calls remain");
        let texts = frame_at(&mut app, &ctx, egui::vec2(1180.0, 780.0), Vec::new());
        assert!(
            texts.iter().any(|text| text == "No missed calls"),
            "{texts:?}"
        );
        assert!(
            texts
                .iter()
                .any(|text| text == "Choose All to see every call."),
            "{texts:?}"
        );
        assert!(
            app.call_log.iter().all(|call| ctx
                .read_response(calls::row_id(&call.chat, &call.id))
                .is_none()),
            "no rows"
        );
        let (app, ctx) = open("calls-missed", false);
        assert_eq!(app.call_filter, CallFilter::Missed);
        assert!(recorded(&ctx, calls::filter_chip_id(CallFilter::Missed)).is_some());
    }

    /// Archived sits in the rail's bottom group, lit while the archive fills
    /// the list column, and Chats leads back; the chip row has no Archived.
    #[test]
    fn the_archive_opens_from_the_rail_into_the_list() {
        let (mut app, ctx) = open("chat", false);
        assert!(
            Destination::Archived.offered(&app),
            "the demo archives chats"
        );
        let archived = destination_rect(&ctx, Destination::Archived);
        let settings = destination_rect(&ctx, Destination::Settings);
        let rail = rail_rect(&ctx).unwrap();
        assert!(rail.contains_rect(archived) && archived.bottom() <= settings.top());
        assert!(
            ctx.data(|data| data.get_temp::<egui::Rect>(egui::Id::new("archived-chip")))
                .is_none(),
            "no Archived chip"
        );
        click(&mut app, &ctx, 1180.0, archived.center());
        assert!(app.show_archived && Destination::Archived.lit(&app));
        assert!(!Destination::Chats.lit(&app));
        assert!(
            list_header(&ctx).is_some(),
            "the archive is in the list column"
        );
        let chats = destination_rect(&ctx, Destination::Chats);
        click(&mut app, &ctx, 1180.0, chats.center());
        assert!(!app.show_archived && Destination::Chats.lit(&app));
    }

    /// The rail is one shade darker than the list in every palette, light,
    /// dark and the Messages theme's.
    #[test]
    fn the_rail_is_a_shade_darker_than_the_list() {
        let luma = |color: egui::Color32| {
            u32::from(color.r()) + u32::from(color.g()) + u32::from(color.b())
        };
        for palette in [
            crate::theme::Palette::light(),
            crate::theme::Palette::dark(),
        ] {
            assert!(luma(palette.rail()) < luma(palette.sidebar()));
            assert!(palette.rail().a() >= palette.sidebar().a());
        }
    }

    /// The list header keeps only New chat and Hide the chat list beside its
    /// title; the starred messages' header has only Back and its title, and
    /// the Calls tab's only New call.
    #[test]
    fn the_list_header_has_only_its_own_buttons() {
        let in_header = |ctx: &egui::Context| {
            let header = list_header(ctx).expect("the list header is drawn");
            focus::stops(ctx)
                .into_iter()
                .filter(|(_, id)| {
                    ctx.read_response(*id)
                        .is_some_and(|response| header.intersects(response.rect))
                })
                .map(|(stop, _)| stop)
                .collect::<Vec<_>>()
        };
        for macos in [false, true] {
            let (_app, ctx) = open("chat", macos);
            assert_eq!(in_header(&ctx), [Stop::NewChat, Stop::Sidebar], "{macos}");
            let (_app, ctx) = open("starred", macos);
            assert_eq!(in_header(&ctx), [Stop::Back], "{macos}");
            let (_app, ctx) = open("calls", macos);
            assert_eq!(in_header(&ctx), [Stop::NewChat], "{macos}: New call");
        }
    }

    /// Opening Calls keeps the open chat beside the history; picking a row
    /// opens that row's chat there, still on Calls. With no chat open the
    /// pane offers to start a call.
    #[test]
    fn the_calls_tab_keeps_the_open_chat_beside_its_history() {
        let (mut app, ctx) = open("chat", false);
        let chat = app.open_chat.clone().expect("a chat is open to start");
        let calls = destination_rect(&ctx, Destination::Calls);
        click(&mut app, &ctx, 1180.0, calls.center());
        assert_eq!(app.page, Page::Calls);
        assert!(Destination::Calls.lit(&app) && !Destination::Chats.lit(&app));
        assert_eq!(app.open_chat.as_deref(), Some(chat.as_str()));
        assert!(
            conversation_header(&ctx).is_some(),
            "the chat stays on the right"
        );
        assert!(list_header(&ctx).is_some(), "the history is on the left");

        let (mut app, ctx) = open("calls-chat", false);
        let opened = app.open_chat.clone().unwrap();
        assert!(conversation_header(&ctx).is_some());
        let other = app
            .call_log
            .iter()
            .find(|call| call.chat != opened)
            .cloned()
            .expect("a call with someone else");
        let row = ctx
            .read_response(crate::ui::calls::row_id(&other.chat, &other.id))
            .expect("the row is drawn")
            .rect;
        click(
            &mut app,
            &ctx,
            1180.0,
            row.left_center() + egui::vec2(40.0, 0.0),
        );
        assert_eq!(app.page, Page::Calls, "picking a call stays on Calls");
        assert_eq!(app.open_chat.as_deref(), Some(other.chat.as_str()));
        assert!(app.call_picked());
        assert!(conversation_header(&ctx).is_some());

        let (app, ctx) = open("calls-info", false);
        assert_eq!(app.page, Page::Calls);
        assert!(app.call_picked() && conversation_header(&ctx).is_some());

        let (_app, ctx) = open("calls", false);
        assert!(
            conversation_header(&ctx).is_none(),
            "no chat: the start pane"
        );
    }

    /// Calls and Starred are left through Chats or Escape, and Ctrl+Shift+1
    /// and Ctrl+Shift+2 lead to Chats and Calls from anywhere.
    #[test]
    fn escape_and_the_shortcuts_return_to_the_chats() {
        let (mut app, ctx) = open("calls-chat", false);
        let chat = app.open_chat.clone().unwrap();
        frame(&mut app, &ctx, 1180.0, vec![escape()]);
        frame(&mut app, &ctx, 1180.0, Vec::new());
        assert_eq!(app.page, Page::Chats);
        assert_eq!(
            app.open_chat.as_deref(),
            Some(chat.as_str()),
            "the chat stays open"
        );

        frame(
            &mut app,
            &ctx,
            1180.0,
            vec![shifted(egui::Key::Num2, egui::Key::Num2)],
        );
        frame(&mut app, &ctx, 1180.0, Vec::new());
        assert_eq!(app.page, Page::Calls);
        frame(
            &mut app,
            &ctx,
            1180.0,
            vec![shifted(egui::Key::Exclamationmark, egui::Key::Num1)],
        );
        frame(&mut app, &ctx, 1180.0, Vec::new());
        assert_eq!(app.page, Page::Chats);
        assert_eq!(app.open_chat.as_deref(), Some(chat.as_str()));

        let starred = destination_rect(&ctx, Destination::Starred);
        click(&mut app, &ctx, 1180.0, starred.center());
        assert!(app.show_starred && Destination::Starred.lit(&app));
        click(&mut app, &ctx, 1180.0, starred.center());
        assert!(app.show_starred, "Starred is not a toggle");
        frame(&mut app, &ctx, 1180.0, vec![escape()]);
        frame(&mut app, &ctx, 1180.0, Vec::new());
        assert!(!app.show_starred && Destination::Chats.lit(&app));
        click(&mut app, &ctx, 1180.0, starred.center());
        assert!(app.show_starred);
        let chats = destination_rect(&ctx, Destination::Chats);
        click(&mut app, &ctx, 1180.0, chats.center());
        assert!(!app.show_starred && app.page == Page::Chats);
    }

    /// A narrow window keeps the rail beside the list, and the rail goes
    /// with the list when a picked call's chat slides over it; Escape
    /// brings the history back.
    #[test]
    fn a_narrow_window_hides_the_rail_with_the_list() {
        let (mut app, ctx) = open("calls", false);
        for _ in 0..30 {
            frame(&mut app, &ctx, 480.0, Vec::new());
        }
        let rail = rail_rect(&ctx).expect("the rail stands beside the history");
        let header = list_header(&ctx).expect("the history fills the window");
        assert!(rail.right() <= header.left());
        let call = app.call_log[0].clone();
        app.actions.push(Action::PickCall {
            chat: call.chat.clone(),
            message: call.id.clone(),
        });
        for _ in 0..30 {
            frame(&mut app, &ctx, 480.0, Vec::new());
        }
        assert!(conversation_header(&ctx).is_some(), "the chat slides over");
        assert!(rail_rect(&ctx).is_none() && list_header(&ctx).is_none());
        frame(&mut app, &ctx, 480.0, vec![escape()]);
        for _ in 0..30 {
            frame(&mut app, &ctx, 480.0, Vec::new());
        }
        assert_eq!(app.page, Page::Calls);
        assert!(app.open_chat.is_none());
        assert!(rail_rect(&ctx).is_some() && list_header(&ctx).is_some());
    }
}
