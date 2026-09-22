//! Offline sample data for screenshots, recorded tours, and headless UI tests.

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

/// Generates a small JPEG attachment preview.
pub fn sample_thumbnail(seed: u32) -> Vec<u8> {
    let (width, height) = (64u32, 48u32);
    let mut image = image::RgbImage::new(width, height);
    for (x, y, pixel) in image.enumerate_pixels_mut() {
        let t = x as f32 / width as f32;
        let u = y as f32 / height as f32;
        let hue = ((seed * 37) % 360) as f32;
        let base = crate::theme::hsl_rgb(hue, 0.45, 0.35 + 0.3 * u);
        let dark = ((x as i32 - 40).pow(2) + (y as i32 - 30).pow(2)) < 120;
        *pixel = if dark {
            image::Rgb([30, 30, 34])
        } else {
            image::Rgb([
                (base[0] as f32 * (0.7 + 0.3 * t)) as u8,
                (base[1] as f32 * (0.7 + 0.3 * t)) as u8,
                (base[2] as f32 * (0.7 + 0.3 * t)) as u8,
            ])
        };
    }
    let mut bytes = Vec::new();
    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 70);
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
    for chat in &app.chats {
        everyone.extend(chat.participants.iter().cloned());
    }
    for id in everyone {
        let kind = if id.ends_with("@g.us") {
            "group"
        } else {
            "person"
        };
        let path = dir.join(format!("demo-{kind}-{}.jpg", crate::util::hue(&id) as u32));
        if !path.exists() {
            let picture = painted_avatar(&id);
            if picture.save(&path).is_err() {
                continue;
            }
        }
        app.adopt_avatar(&id, path);
    }
}

/// Generates one 128-pixel id-colored profile picture.
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
    let _ = std::fs::create_dir_all(&dir);
    let photo = dir.join("demo-photo.jpg");
    if !photo.exists() {
        let (width, height) = (900u32, 1200u32);
        let mut image = image::RgbImage::new(width, height);
        for (x, y, pixel) in image.enumerate_pixels_mut() {
            let t = y as f32 / height as f32;
            let base = crate::theme::hsl_rgb(200.0 + 40.0 * t, 0.5, 0.35 + 0.35 * t);
            let ring = ((x as i32 - 450).pow(2) + (y as i32 - 700).pow(2)) as f32;
            let on_ring = (ring.sqrt() - 260.0).abs() < 14.0;
            *pixel = if on_ring {
                image::Rgb([250, 244, 220])
            } else {
                image::Rgb(base)
            };
        }
        let _ = image.save(&photo);
    }
    let sticker = dir.join("demo-sticker.gif");
    if !sticker.exists()
        && let Ok(file) = std::fs::File::create(&sticker)
    {
        let mut encoder = image::codecs::gif::GifEncoder::new(file);
        let _ = encoder.set_repeat(image::codecs::gif::Repeat::Infinite);
        for step in 0..8u32 {
            let mut frame = image::RgbaImage::from_pixel(160, 160, image::Rgba([0, 0, 0, 0]));
            let angle = step as f32 * std::f32::consts::TAU / 8.0;
            let (cx, cy) = (80.0 + 40.0 * angle.cos(), 80.0 + 40.0 * angle.sin());
            for (x, y, pixel) in frame.enumerate_pixels_mut() {
                let d = ((x as f32 - cx).powi(2) + (y as f32 - cy).powi(2)).sqrt();
                if d < 28.0 {
                    *pixel = image::Rgba([255, 200, 60, 255]);
                }
            }
            let _ = encoder.encode_frame(image::Frame::from_parts(
                frame,
                0,
                0,
                image::Delay::from_numer_denom_ms(120, 1),
            ));
        }
    }
    (photo, sticker)
}

/// Loads the sample account and opens its first chat.
pub fn populate(app: &mut App) {
    app.backend.set_offline(true);
    // Demo mode has no backend to handle downloads.
    app.settings.auto_download = false;
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
            push_name: None,
        },
    );
    for sample in SAMPLES {
        let mut chat = Chat::new(sample.id.to_owned(), sample.name.to_owned());
        chat.last_activity = now - sample.minutes_ago * 60;
        chat.unread = sample.unread;
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
        }
        chat.last = conversation
            .messages
            .last()
            .map(|last| crate::model::LastMessage {
                from_me: last.from_me,
                sender: last.sender.clone(),
                sender_name: last.sender_name.clone(),
                summary: last.summary(),
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
                    caption: Some("The difference engine, finally assembled".into()),
                    media: media("image/jpeg", 1_843_201, Some(1600), Some(1200)),
                },
            );
            row.thumbnail = Some(sample_thumbnail(1));
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
            row.thumbnail = Some(sample_thumbnail(3));
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
                },
            );
            row.thumbnail = Some(sample_thumbnail(2));
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
                    caption: None,
                    media: media("image/jpeg", 402_113, Some(900), Some(1200)),
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
                    media: media("image/webp", 20_000, Some(160), Some(160)),
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
                    caption: Some("Tonight's venue, doors at 18:30".into()),
                    media: media("image/jpeg", 1_204_551, Some(1600), Some(1200)),
                },
            );
            row.sender = tom.0.to_owned();
            row.sender_name = Some(tom.1.to_owned());
            row.thumbnail = Some(sample_thumbnail(2));
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
    for chat in &mut app.chats {
        if let Some(last) = app
            .conversations
            .get(&chat.id)
            .and_then(|conversation| conversation.messages.last())
        {
            chat.last_activity = last.timestamp;
            chat.last = Some(crate::model::LastMessage {
                from_me: last.from_me,
                sender: last.sender.clone(),
                sender_name: last.sender_name.clone(),
                summary: last.summary(),
                status: last.status,
            });
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
    app.scroll_to_bottom = true;
    app.focus_composer = false;
}

/// Applies the UI state selected by `--demo-page`.
pub fn apply_flags(app: &mut App, page: Option<&str>) {
    let Some(page) = page else {
        return;
    };
    for part in page.split(',').map(str::trim) {
        match part {
            "chat" | "" => {}
            "chat-menu" => app.open_chat_menu = Some(app.chats[0].id.clone()),
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
                ];
                if let Some(chat) = app.chats.iter_mut().find(|chat| chat.id == id) {
                    chat.name = "שלום יזמות ונדל\"ן".into();
                    if let Some(last) = &mut chat.last {
                        last.summary = "הכלב הגדול קפץ 🐕".into();
                    }
                }
                app.conversations.get_mut(id).expect("demo group").messages = messages;
                app.open_chat = Some(id.into());
                app.typing.clear();
                app.scroll_to_bottom = true;
            }
            "settings" => app.page = Page::Settings,
            "custom-media" => {
                app.page = Page::Settings;
                let custom = app
                    .dirs
                    .config
                    .parent()
                    .unwrap()
                    .join("Downloads/Attachments");
                app.settings.custom_media_dir = Some(custom.clone());
                app.dirs.custom_media = Some(custom);
            }
            "omarchy" | "omarchy-light" => {
                let mut themes: Vec<_> = crate::theme::presets::themes().collect();
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
                app.custom_themes = crate::theme::custom::Catalog::preview(themes, true);
            }
            choice if choice.starts_with("theme=") => {
                let themes: Vec<_> = crate::theme::presets::themes().collect();
                if let Some(theme) = themes
                    .iter()
                    .find(|theme| Some(theme.filename.as_str()) == choice.strip_prefix("theme="))
                {
                    app.settings.custom_theme = Some(theme.filename.clone());
                    app.settings.custom_theme_cache = Some(theme.clone());
                }
                app.custom_themes = crate::theme::custom::Catalog::preview(themes, false);
            }
            "themes" => {
                use crate::theme::custom::{Catalog, CustomTheme};
                let mut palette = crate::theme::Palette::dark();
                palette.accent = egui::Color32::from_rgb(137, 180, 250);
                palette.bubble_out = egui::Color32::from_rgb(41, 57, 84);
                let theme = CustomTheme {
                    filename: "Moonlight 🌙.json".into(),
                    palette,
                };
                let mut themes: Vec<_> = crate::theme::presets::themes().collect();
                themes.push(theme.clone());
                app.custom_themes = Catalog::preview(themes, false);
                app.settings.custom_theme = Some(theme.filename.clone());
                app.settings.custom_theme_cache = Some(theme);
                app.page = Page::Settings;
            }
            "update" | "update-downloading" | "update-ready" | "update-failed"
            | "update-managed" => {
                use crate::updates::{
                    DownloadState,
                    install::{Installation, Kind, Prepared},
                };
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
                    "update-ready" => DownloadState::Ready(Box::new(Prepared {
                        installation,
                        directory: "/demo/staging".into(),
                        payload: "/demo/staging/next".into(),
                        sha256: String::new(),
                        version: "99.0.0".into(),
                    })),
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
            "forward" => {
                app.dialog = app.open_chat.clone().map(|chat| Dialog::Forward {
                    chat,
                    message: "ada-format".to_owned(),
                });
            }
            "unlink" => app.dialog = Some(Dialog::ConfirmUnlink),
            "toasts" => {
                app.toast("History loaded");
                app.toast_error(
                    "Could not open the attachment: No application knows how to open \"Notes on the Engine.pdf\" (error -10814)",
                );
            }
            "delete-chat" => {
                app.dialog = app.open_chat.clone().map(Dialog::ConfirmDeleteChat);
            }
            "new-contact" => app.dialog = Some(Dialog::NewContact),
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
            "typing" => {
                app.composer = (1..=9)
                    .map(|line| format!("line {line}"))
                    .collect::<Vec<_>>()
                    .join("\n");
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
            "nosidebar" => app.sidebar_visible = false,
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
                for id in ["ada-voice", "you-voice"] {
                    if let Some(message) = app
                        .conversations
                        .get_mut(&app.open_chat.clone().unwrap_or_default())
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
            "unread" => app.chat_filter = crate::model::ChatFilter::Unread,
            "private" => app.chat_filter = crate::model::ChatFilter::Private,
            "groups" => app.chat_filter = crate::model::ChatFilter::Groups,
            "picker" => app.picker = Some(crate::model::PickerTab::Emoji),
            "stickers" => {
                app.picker = Some(crate::model::PickerTab::Stickers);
                let (_, sticker) = sample_files(app);
                app.stickers_saved = vec![sticker.clone(); 3];
                app.sticker_packs = vec![crate::model::StickerPack {
                    name: "Happy Frogs".to_owned(),
                    dir: std::path::PathBuf::from("Happy Frogs"),
                    stickers: vec![sticker.clone(); 6],
                }];
                app.stickers = vec![sticker; 7];
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

    #[test]
    fn the_sample_has_every_kind_of_row() {
        let app = app();
        assert!(app.chats.len() >= 5);
        assert!(app.chats.iter().any(|chat| chat.is_group()));
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
            "channel",
            "locked",
            "locked-open",
            "locked-prompt",
            "locked-setup",
            "new-chat",
            "unnamed-group",
            "keyring",
            "empty",
            "rtl",
            "disappearing",
            "settings",
            "custom-media",
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
            "forward",
            "unlink",
            "toasts",
            "delete-chat",
            "new-contact",
            "light",
            "archived",
            "unread",
            "private",
            "groups",
            "offline",
            "syncing",
            "picker",
            "stickers",
            "typing",
            "mention",
            "emoji-complete",
            "typers",
            "nosidebar",
            "search",
            "staged",
            "compose-emoji",
            "voice",
            "recording",
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

    fn key(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }
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

    /// The sent, delivered, and read rows only inform. The message id is
    /// copied by a row that says so, not by clicking "Sent".
    #[test]
    fn message_status_rows_are_not_actions() {
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
            let (label, role, _) = find(status);
            assert_eq!(role, Role::Label, "{label} is information, not a button");
        }
        assert_eq!(find("Copy message ID").1, Role::Button);

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
        click(&mut app, find("Sent ").2);
        assert!(
            app.toasts.iter().all(|toast| toast.message != "Copied"),
            "clicking Sent copies nothing"
        );

        app.open_message_menu = Some("ada-link".into());
        render(&mut app, &ctx);
        let nodes = accessible_nodes(&mut app, &ctx, Vec::new());
        let copy = nodes
            .iter()
            .find(|(label, _, _)| label == "Copy message ID")
            .expect("the menu is open again")
            .2;
        click(&mut app, copy);
        assert!(app.toasts.iter().any(|toast| toast.message == "Copied"));
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
    fn a_filter_chip_narrows_the_chat_list_and_a_second_click_clears_it() {
        use crate::model::ChatFilter;
        let mut app = app();
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
        let hold = egui::pos2(view.center().x, view.top() + 10.0);
        let press = egui::Event::PointerButton {
            pos: hold,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        };
        let mut frames: Vec<Vec<egui::Event>> = vec![vec![egui::Event::PointerMoved(hold), press]];
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
            "input focus stays inside the field"
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
        frame_sized(&mut app, &ctx, 780.0, tab());
        assert_eq!(focused_stop(&ctx), Some(crate::ui::focus::Stop::Poll));
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
        assert_eq!(app.dialog, Some(crate::model::Dialog::CreatePoll(group)));
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
                Stop::Poll,
                Stop::Emoji,
                Stop::Profile,
                Stop::Sidebar,
                Stop::NewChat,
                Stop::Settings,
                Stop::Search,
                Stop::All,
                Stop::Unread,
                Stop::Private,
                Stop::Groups,
                Stop::Locked,
            ]
            .into_iter()
            .filter(|stop| {
                !crate::theme::macos_chrome(&ctx) || !matches!(stop, Stop::Profile | Stop::Settings)
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
        for page in ["nosidebar", "empty", "channel", "search", "chat"] {
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
            if page == "nosidebar" {
                assert_eq!(
                    controls.iter().map(|(stop, _)| *stop).collect::<Vec<_>>(),
                    [
                        Stop::Composer,
                        Stop::Send,
                        Stop::Attach,
                        Stop::Poll,
                        Stop::Emoji,
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
}
