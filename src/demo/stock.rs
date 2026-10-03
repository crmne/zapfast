//! Stock pictures, clips, and stickers for the demo, so its chats look like
//! chats: all CC0 or in the public domain (`assets/demo/SOURCES.md`).

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// A picture with its size in pixels.
#[derive(Clone, Copy)]
pub struct Photo {
    pub bytes: &'static [u8],
    pub width: u32,
    pub height: u32,
    /// The file it is saved as in the media directory.
    pub name: &'static str,
}

macro_rules! photo {
    ($file:literal, $width:literal, $height:literal) => {
        Photo {
            bytes: include_bytes!(concat!("../../assets/demo/photos/", $file)),
            width: $width,
            height: $height,
            name: concat!("demo-", $file),
        }
    };
}

/// Babbage's Difference Engine No. 1, a portrait.
pub const ENGINE: Photo = photo!("engine.jpg", 900, 1200);
/// A rocket leaving over the water.
pub const LAUNCH: Photo = photo!("launch.jpg", 1024, 683);
/// A neon bar sign.
pub const VENUE: Photo = photo!("venue.jpg", 1024, 683);
/// A rocket's trail at dusk, light towards the top.
pub const DUSK: Photo = photo!("dusk.jpg", 1200, 800);
/// An astronaut on a spacewalk, a portrait.
pub const SPACEWALK: Photo = photo!("spacewalk.jpg", 900, 1200);
/// Spotifast's logo, for its link preview.
pub const SPOTIFAST: Photo = photo!("spotifast.png", 256, 256);

/// The pictures `thumbnail_for` cycles through.
const PHOTOS: [Photo; 5] = [LAUNCH, ENGINE, VENUE, DUSK, SPACEWALK];

/// A four-second H.264 and AAC clip of a rocket leaving the pad, 640 by 360.
pub const VIDEO: &[u8] = include_bytes!("../../assets/demo/videos/launch.mp4");
/// The same launch from beside the pad, square, for a round video message.
pub const NOTE: &[u8] = include_bytes!("../../assets/demo/videos/note.mp4");
/// How long both clips run, in seconds.
pub const CLIP_SECONDS: u32 = 4;

/// A floating astronaut: an animated sticker.
pub const ANIMATED_STICKER: &[u8] =
    include_bytes!("../../assets/demo/stickers/astronaut-float.webp");

macro_rules! sticker {
    ($emoji:literal, $file:literal) => {
        (
            $emoji,
            include_bytes!(concat!("../../assets/demo/stickers/", $file)).as_slice(),
        )
    };
}

/// Captioned stickers, each under the emoji WhatsApp would tag it with.
pub const STICKERS: [(char, &[u8]); 7] = [
    sticker!('🚀', "astronaut.webp"),
    sticker!('🐸', "frog.webp"),
    sticker!('🤣', "raccoon.webp"),
    sticker!('🙅', "goat.webp"),
    sticker!('🐶', "pug.webp"),
    sticker!('🥱', "cat.webp"),
    sticker!('🦉', "owl.webp"),
];

/// The sticker tagged `emoji`, if there is one.
pub fn sticker(emoji: char) -> Option<&'static [u8]> {
    STICKERS
        .iter()
        .find(|(tag, _)| *tag == emoji)
        .map(|(_, bytes)| *bytes)
}

macro_rules! gif {
    ($file:literal) => {
        include_bytes!(concat!("../../assets/demo/gifs/", $file)).as_slice()
    };
}

/// Stills for the GIF search results, 320 by 240.
pub const GIFS: [&[u8]; 6] = [
    gif!("fireworks.jpg"),
    gif!("launch.jpg"),
    gif!("pug.jpg"),
    gif!("raccoon.jpg"),
    gif!("goat.jpg"),
    gif!("frog.jpg"),
];

macro_rules! avatar {
    ($id:literal, $file:literal) => {
        (
            $id,
            include_bytes!(concat!("../../assets/demo/avatars/", $file)).as_slice(),
        )
    };
}

/// Profile pictures by chat or contact id: the portraits of the people the
/// sample contacts are named after, and the pets and places others would use.
const AVATARS: [(&str, &[u8]); 14] = [
    avatar!("15550001111@s.whatsapp.net", "astronaut.jpg"), // ourselves
    avatar!("393331234567@s.whatsapp.net", "ada.jpg"),
    avatar!("441632960123@s.whatsapp.net", "grace.jpg"),
    avatar!("4915112345678@s.whatsapp.net", "katherine.jpg"),
    avatar!("14155550199@s.whatsapp.net", "earthrise.jpg"), // Margaret Hamilton
    avatar!("12025550137@s.whatsapp.net", "dorothy.jpg"),
    avatar!("491701111111@s.whatsapp.net", "goat.jpg"), // Jonas
    avatar!("491702222222@s.whatsapp.net", "owl.jpg"),  // Mira
    avatar!("491703333333@s.whatsapp.net", "pug.jpg"),  // Tom
    avatar!("972501234567@s.whatsapp.net", "cat.jpg"),  // Yael
    avatar!("120363012345678901@g.us", "berlin.jpg"),   // Rust Berlin
    avatar!("120363098765432109@g.us", "bread.jpg"),    // Family
    avatar!("120363011122233344@g.us", "bar.jpg"),      // Section 8 Berlin
    avatar!("120363055566677788@newsletter", "crab.jpg"), // Rust Weekly
];

/// The profile picture for `id`, if it has one.
pub fn avatar(id: &str) -> Option<&'static [u8]> {
    AVATARS
        .iter()
        .find(|(owner, _)| *owner == id)
        .map(|(_, bytes)| *bytes)
}

/// Saves `bytes` as `name` in `dir` unless it is there already.
pub fn save(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let path = dir.join(name);
    let current = std::fs::metadata(&path).is_ok_and(|file| file.len() == bytes.len() as u64);
    if !current {
        let _ = std::fs::create_dir_all(dir);
        let _ = std::fs::write(&path, bytes);
    }
    path
}

/// Saves `photo` in `dir` and returns its path.
pub fn save_photo(dir: &Path, photo: Photo) -> PathBuf {
    save(dir, photo.name, photo.bytes)
}

/// The small JPEG preview WhatsApp sends ahead of `photo`.
pub fn thumbnail(photo: Photo) -> Vec<u8> {
    let Ok(image) = image::load_from_memory(photo.bytes) else {
        return Vec::new();
    };
    let small = image.thumbnail(72, 72).to_rgb8();
    let mut bytes = Vec::new();
    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 60);
    let _ = encoder.encode_image(&small);
    bytes
}

/// The preview of one of the pictures, chosen by `seed`. Made once each.
pub fn thumbnail_for(seed: u32) -> Vec<u8> {
    static CACHE: [OnceLock<Vec<u8>>; PHOTOS.len()] = [const { OnceLock::new() }; PHOTOS.len()];
    let index = seed as usize % PHOTOS.len();
    CACHE[index]
        .get_or_init(|| thumbnail(PHOTOS[index]))
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pictures_are_the_sizes_they_say() {
        for photo in PHOTOS.into_iter().chain([SPOTIFAST]) {
            let image = image::load_from_memory(photo.bytes).expect(photo.name);
            assert_eq!(
                (image.width(), image.height()),
                (photo.width, photo.height),
                "{}",
                photo.name
            );
            assert!(!thumbnail(photo).is_empty(), "{}", photo.name);
        }
        for still in GIFS {
            let image = image::load_from_memory(still).expect("a GIF still");
            assert_eq!((image.width(), image.height()), (320, 240));
        }
    }

    #[test]
    fn the_stickers_take_their_emoji_tags() {
        for (emoji, bytes) in STICKERS {
            let image = image::load_from_memory(bytes).expect("a sticker");
            assert_eq!((image.width(), image.height()), (512, 512), "{emoji}");
            let info = crate::sticker_meta::StickerInfo {
                emojis: vec![emoji.to_string()],
                ..Default::default()
            };
            let tagged = crate::sticker_meta::write(bytes, &info).expect("room for the tag");
            assert_eq!(crate::sticker_meta::emojis(&tagged), [emoji.to_string()]);
            assert!(image::load_from_memory(&tagged).is_ok(), "{emoji}");
        }
        assert!(sticker('🤣').is_some());
        assert!(sticker('🎉').is_none());
    }

    #[test]
    fn every_sample_chat_has_a_profile_picture_that_loads() {
        for (id, bytes) in AVATARS {
            let image = image::load_from_memory(bytes).expect(id);
            assert_eq!((image.width(), image.height()), (192, 192), "{id}");
        }
        for id in crate::demo::sample_ids() {
            // The dentist keeps a painted one, as a business without a photo would.
            assert!(
                avatar(id).is_some() || id == "33612345678@s.whatsapp.net",
                "{id}"
            );
        }
    }

    #[test]
    fn the_clips_decode_in_process() {
        let dir = std::env::temp_dir().join(format!("zapfast-stock-{}", std::process::id()));
        for (name, bytes, size) in [
            ("launch.mp4", VIDEO, [640, 360]),
            ("note.mp4", NOTE, [360, 360]),
        ] {
            let path = save(&dir, name, bytes);
            // Frames come scaled down; the shape is what the bubble needs.
            let frame = crate::animation::video_frame(&path, 12).expect(name);
            assert_eq!(
                frame.size[0] * size[1],
                frame.size[1] * size[0],
                "{name}: {:?}",
                frame.size
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
