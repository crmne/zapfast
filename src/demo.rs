//! Offline sample data for screenshots, recorded tours, and headless UI tests.

pub mod stock;
pub mod tour;

use std::collections::HashMap;

use crate::app::{App, Conversation, Presence};
use crate::backend::LinkStatus;
use crate::model::{
    Chat, Contact, Content, Delivery, Dialog, LinkPreview, Media, MentionRef, Message, Page,
    Quoted, Reaction,
};
use crate::settings::ThemeChoice;

const ME: &str = "15550001111@s.whatsapp.net";

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

fn media(mime: &str, size: u64, width: Option<u32>, height: Option<u32>) -> Media {
    Media {
        mime: mime.to_owned(),
        size,
        width,
        height,
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
        }
        chat.last = conversation
            .messages
            .last()
            .map(|last| crate::model::LastMessage {
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
        message(
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
        ),
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
        id: "meta-ai-question".into(),
        sender: ME.into(),
        sender_name: None,
        summary: "How do I reverse a string in Rust?".into(),
        mentions: Vec::new(),
    });
    let mut chat = Chat::new(id.into(), "Meta AI".into());
    chat.last_activity = now;
    chat.last = Some(crate::model::LastMessage {
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

pub fn apply_flags(app: &mut App, page: Option<&str>) {
    let Some(page) = page else {
        return;
    };
    for part in page.split(',').map(str::trim) {
        match part {
            "chat" | "" => {}
            "phone-menu" => phone_menu_sample(app),
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
            "poll-empty" => poll_sample(app, false, false),
            "poll-voted" => poll_sample(app, true, false),
            "poll-results" => poll_sample(app, true, true),
            "photos" => photos_sample(app),
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
                app.actions.push(crate::model::Action::PreviewImage(
                    app.dirs.media_cache_dir().join(stock::LAUNCH.name),
                ));
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
            "info" => {
                app.dialog = app.open_chat.clone().map(Dialog::ChatInfo);
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
                app.dialog = Some(Dialog::ChatInfo(group.to_owned()));
                if part == "group-info-rename" {
                    app.group_name_edit = Some("Rust Berlin 🦀".to_owned());
                }
                if part == "group-info-saving" {
                    app.group_saving.insert(group.to_owned());
                }
            }
            "forward" => {
                app.dialog = app.open_chat.clone().map(|chat| Dialog::Forward {
                    chat,
                    messages: vec!["ada-format".to_owned()],
                });
            }
            "unlink" => app.dialog = Some(Dialog::ConfirmUnlink),
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
            "new-contact" => app.dialog = Some(Dialog::NewContact),
            // The switcher under our avatar open with the one account.
            "account-menu" => app.account_menu = true,
            // A second number linked beside the first, with unread chats of
            // its own, and the switcher under our avatar open.
            "accounts" | "accounts-closed" => {
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
                app.account_menu = part == "accounts";
            }
            "light" => {
                app.settings.theme = ThemeChoice::Light;
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
                let (photo, _) = sample_files(app);
                app.image_preview = Some(crate::image_preview::PreviewState::new(photo));
            }
            "compose-emoji" => {
                app.composer = "Andiamo 😊 con due 👍🏽 e poi testo normale".to_owned();
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
                app.pending.push(crate::app::Pending::Picture {
                    width: side,
                    height: side,
                    rgba: std::sync::Arc::new(rgba),
                    texture: None,
                });
                app.pending.push(crate::app::Pending::File(photo));
                app.pending
                    .push(crate::app::Pending::File("/tmp/notes.pdf".into()));
                app.composer = "Look at these".into();
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
            "voice-menu" => app.open_message_menu = Some("ada-voice".into()),
            "react-menu" => {
                app.open_message_menu = Some("ada-link".into());
                if let Some(row) = app
                    .conversations
                    .get_mut(SAMPLES[0].id)
                    .and_then(|conversation| conversation.message_mut("ada-link"))
                {
                    row.delivered_at = Some(row.timestamp);
                    row.read_at = Some(row.timestamp + 60 * 60 * 7);
                }
            }
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

fn unlink(app: &mut App) {
    app.chats.clear();
    app.conversations.clear();
    app.open_chat = None;
    app.me = None;
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

    pub(super) fn app() -> App {
        let root = std::env::temp_dir().join(format!(
            "zapfast-demo-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let (mut app, _events) = App::headless(AppDirs::under(&root), Settings::default());
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

    /// A sent message moves its chat up, and the scrolled chat list follows
    /// it back to the top.
    #[test]
    fn sending_a_message_scrolls_the_chat_list_back_to_the_top() {
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
        let last = app.visible_chats().last().map(|chat| chat.id.clone());
        app.scroll_chat_into_view.clone_from(&last);
        render(&mut app);
        assert!(offset() > 0.0, "the list starts scrolled down");

        app.actions.push(crate::model::Action::SendText {
            chat: last.expect("sample chats"),
            text: "Fixture".into(),
            quoting: None,
        });
        render(&mut app);
        assert_eq!(offset(), 0.0, "the list is back at the top");
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
        let body = "اتصل على +00 (000) 00000-0000";
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
        assert_eq!(copied, "+00 (000) 00000-0000");

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
                    } if phone == "00000000000000"
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
    fn locked_folder_explains_its_read_only_state() {
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
        assert!(!app.current_chat().unwrap().can_send());
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
            labels.contains(&"Locked chats are read-only in ZapFast"),
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

    #[test]
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
            "video",
            "video-playing",
            "video-expanded",
            "shared-contact",
            "note-playing",
            "empty",
            "rtl",
            "disappearing",
            "settings",
            "settings-search=Notifications",
            "settings-search=System",
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
            "group-info",
            "group-info-rename",
            "group-info-locked",
            "group-info-saving",
            "forward",
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
            "new-contact",
            "accounts",
            "accounts,light",
            "accounts-closed",
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
            "sticker-pack-message",
            "sticker-maker",
            "sticker-pack-view",
            "typing",
            "composer-tools",
            "mention",
            "emoji-complete",
            "reply",
            "edit",
            "unsent-voice",
            "scrolled",
            "reactions",
            "typers",
            "nosidebar",
            "wide",
            "rail",
            "search",
            "staged",
            "compose-emoji",
            "voice",
            "voice,voice-menu",
            "recording",
            "preview",
            "gifs",
            "gifs-badkey",
            "react-menu",
            "react-picker",
            "react-picker-empty",
            "react-custom",
            "react-other",
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

    #[test]
    fn macos_headers_fit_when_zoomed_with_and_without_the_sidebar() {
        for zoom in [0.6, 1.0, 2.0] {
            for page in [
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
    fn frame_with(app: &mut App, ctx: &egui::Context, events: Vec<egui::Event>) {
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

    /// Both platform shortcut variants produce exactly one clipboard write.
    #[test]
    fn selected_messages_copy_with_command_and_control_c() {
        for modifiers in [egui::Modifiers::COMMAND, egui::Modifiers::CTRL] {
            let (mut app, ctx, chat) = sweep_chat(2);
            app.selection = Some((chat, vec!["m001".into()]));
            render(&mut app, &ctx);
            let copied = copied_text(&mut app, &ctx, key(egui::Key::C, modifiers));
            assert_eq!(copied.len(), 1, "{modifiers:?}");
            assert!(copied[0].ends_with("message number 1"));
        }
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

    /// Dialogs, another chat, and missing IDs must not export selected messages.
    #[test]
    fn selected_messages_do_not_copy_through_dialogs_or_other_chats() {
        let (mut app, ctx, chat) = sweep_chat(2);
        app.selection = Some((chat.clone(), vec!["m001".into()]));
        app.dialog = Some(crate::model::Dialog::Shortcuts);
        render(&mut app, &ctx);
        assert!(copied_text(&mut app, &ctx, egui::Event::Copy).is_empty());
        app.dialog = None;
        app.selection = Some(("another-chat".into(), vec!["m001".into()]));
        render(&mut app, &ctx);
        assert!(copied_text(&mut app, &ctx, egui::Event::Copy).is_empty());
        app.selection = Some((chat, vec!["deleted-message".into()]));
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
                    from_me: false,
                    sender: sender.into(),
                    sender_name: Some("Linus Example".into()),
                    summary: long.into(),
                    full: long.into(),
                    status: Delivery::Read,
                });
            } else if chat.id == direct {
                chat.last = Some(crate::model::LastMessage {
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

    /// The group dialog offers the pencil and the photo menu only when we may
    /// change the group's info, and not while a change is on its way.
    #[test]
    fn group_info_offers_editing_only_when_allowed() {
        use crate::ui::dialogs::{group_name_button_id, group_photo_id};
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

    /// The pencil opens the name editor; Enter renames the group on WhatsApp,
    /// an unchanged name sends nothing, and Escape cancels without closing the
    /// dialog. The photo opens its menu.
    #[test]
    fn a_group_is_renamed_from_its_info_dialog() {
        use crate::backend::Command;
        use crate::ui::dialogs::{group_name_button_id, group_photo_id};
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
        assert!(
            matches!(app.dialog, Some(Dialog::ChatInfo(_))),
            "and keeps the dialog"
        );

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
        app.actions.push(crate::model::Action::PreviewImage(photo));
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
        app.actions.push(crate::model::Action::PreviewImage(long));
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

    /// Opens the sample photo in the preview and waits for it to decode, so
    /// input meets a settled, fitted picture. Returns the fitted scale.
    fn open_sample_preview(app: &mut App, ctx: &egui::Context) -> f32 {
        app.attach(ctx);
        render(app, ctx);
        let (photo, _) = sample_files(app);
        let uri = crate::util::image_uri(&photo);
        app.actions.push(crate::model::Action::PreviewImage(photo));
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
            app.open_message_menu = Some("ada-reply".into());
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
        let path = std::path::PathBuf::from("demo/photo.jpg");
        for downloaded in [true, false] {
            let mut app = app();
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
            app.open_message_menu = Some("copy-photo".into());
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

        // A right-click on the bubble still opens the context menu, not the picker.
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

    #[test]
    fn a_paste_is_seen_on_the_key_release() {
        // Platforms may deliver only the Ctrl+V key release for image paste.
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        let release = egui::Event::Key {
            key: egui::Key::V,
            physical_key: None,
            pressed: false,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        };
        frame_with(&mut app, &ctx, vec![release]);
        assert!(ctx.input(crate::app::wants_paste));
        let plain = egui::Event::Key {
            key: egui::Key::V,
            physical_key: None,
            pressed: false,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        frame_with(&mut app, &ctx, vec![plain]);
        assert!(!ctx.input(crate::app::wants_paste), "a plain V is typing");
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
            chat: chat.clone(),
            caption: "look".into(),
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
        app.reply_to
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
        frame_with(
            &mut app,
            &ctx,
            vec![key(egui::Key::Escape, egui::Modifiers::NONE)],
        );
        assert!(app.editing.is_none());
        assert!(app.composer.is_empty());
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
        let mut app = app();
        for chat in &mut app.chats {
            chat.locked = false;
        }
        app.open_chat = None;
        let ctx = egui::Context::default();
        app.attach(&ctx);
        render(&mut app, &ctx);
        let locked_id = egui::Id::new("locked-chip");
        assert!(
            ctx.data(|data| data.get_temp::<egui::Rect>(locked_id))
                .is_none()
        );
        app.chats[0].locked = true;
        render(&mut app, &ctx);
        let locked = ctx
            .data(|data| data.get_temp::<egui::Rect>(locked_id))
            .unwrap();
        for filter in crate::model::ChatFilter::EVERY {
            let rect = ctx
                .data(|data| data.get_temp::<egui::Rect>(crate::ui::chats::filter_chip_id(filter)))
                .unwrap();
            assert!((rect.top() - locked.top()).abs() < 0.1);
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
            .map(|message| message.id.clone());
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
        // Row 0 sends files, row 1 creates a poll.
        for row in [0.0, 1.0] {
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
                menu.top() + menu.height() * (1.0 + 2.0 * row) / 4.0,
            );
            click(&mut app, &ctx, item);
            assert!(
                !app.composer_tools_open,
                "choosing an entry closes the menu"
            );
            let commands = app.backend.take_demo_commands();
            let picked = commands.iter().any(
                |command| matches!(command, crate::backend::Command::PickFiles(id) if *id == chat),
            );
            if row == 0.0 {
                assert!(picked, "Send files opens the file picker");
                assert_eq!(app.dialog, None);
            } else {
                assert!(!picked);
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
            let expected: Vec<_> = [
                Stop::Composer,
                Stop::Send,
                Stop::Attach,
                Stop::Emoji,
                Stop::ChatSearch,
                Stop::Profile,
                Stop::Sidebar,
                Stop::NewChat,
                Stop::Settings,
                Stop::Search,
                Stop::All,
                Stop::Unread,
                Stop::Private,
                Stop::Favorites,
                Stop::Groups,
                Stop::Channels,
                Stop::Archived,
                Stop::Locked,
            ]
            .into_iter()
            .filter(|stop| {
                // The Mac header keeps the account switcher; the settings
                // live in the app menu there.
                !crate::theme::macos_chrome(&ctx) || *stop != Stop::Settings
            })
            .collect();
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
                (egui::Modifiers::SHIFT, Stop::Locked),
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
        for page in ["locked-setup", "poll-create", "new-chat"] {
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

    #[test]
    fn tab_scrolls_a_focused_collapsed_avatar_into_view() {
        let mut app = app();
        apply_flags(&mut app, Some("settings,rail"));
        let ctx = egui::Context::default();
        app.attach(&ctx);
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
            "the window is short enough to hide avatars"
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
                "the focused avatar is fully visible"
            );
            reached_hidden |= hidden
                .iter()
                .any(|id| avatar(&ctx, id).is_some_and(|rect| rect == focused.rect));
        }
        assert!(reached_hidden, "Tab reached avatars that were out of view");
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
        // The picture decodes on a loader thread.
        for _ in 0..30 {
            frame(&mut app);
            std::thread::sleep(std::time::Duration::from_millis(5));
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

    fn photos() -> (App, egui::Context) {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        apply_flags(&mut app, Some("photos"));
        frames(&mut app, &ctx, egui::vec2(1180.0, 1400.0));
        (app, ctx)
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

    /// A chat with unsent text shows it in its row, after an accent
    /// "Draft:", while the open chat's text stays in the composer (#245).
    #[test]
    fn a_chat_with_a_draft_shows_it_in_the_list() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.attach(&ctx);
        apply_flags(&mut app, Some("drafts"));
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
}
