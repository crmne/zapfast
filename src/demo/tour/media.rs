//! Animated GIF-search thumbnails and stickers from the stock demo media, and
//! emoji pictures from the bundled emoji font.

use anyhow::{Context, Result};
use image::RgbaImage;
use skrifa::{FontRef, MetadataProvider, bitmap::BitmapData, instance::Size};

use super::super::stock;
use crate::{
    app::App,
    model::{Gif, StickerPack},
};

/// A character from the bundled color emoji font, `side` pixels square.
pub(crate) fn emoji_image(character: char, side: u32) -> Result<RgbaImage> {
    let font = FontRef::new(include_bytes!("../../../assets/fonts/NotoColorEmoji.ttf"))?;
    let glyph = font
        .charmap()
        .map(character as u32)
        .context("sample emoji glyph")?;
    let bitmap = font
        .bitmap_strikes()
        .glyph_for_size(Size::new(128.0), glyph)
        .context("sample emoji bitmap")?;
    let BitmapData::Png(png) = bitmap.data else {
        anyhow::bail!("expected PNG emoji");
    };
    let emoji = image::load_from_memory(png)?.to_rgba8();
    Ok(image::imageops::resize(
        &emoji,
        side,
        side,
        image::imageops::FilterType::Lanczos3,
    ))
}

/// Where the tour keeps its media, which a tour can drop on the window.
pub fn dir(app: &App) -> std::path::PathBuf {
    app.account().dirs.media_cache_dir().join("tour")
}

/// Gives each GIF result an animation of its own picture, a slow push in
/// and back out, in place of the one shared sticker.
pub fn animate_gifs(app: &mut App) {
    let dir = dir(app);
    for (index, gif) in app.gif_results.iter_mut().enumerate() {
        let path = dir.join(format!("reaction-{index}.gif"));
        let made = path.is_file()
            || gif
                .still
                .as_deref()
                .is_some_and(|still| push_in(still, &path).is_ok());
        if made {
            gif.animated = Some(path);
        }
    }
}

/// Writes the picture at `still` as a looping GIF that zooms in and out.
fn push_in(still: &std::path::Path, path: &std::path::Path) -> Result<()> {
    use image::codecs::gif::{GifEncoder, Repeat};
    const FRAMES: u32 = 16;
    let picture = image::open(still)?.to_rgba8();
    let (width, height) = picture.dimensions();
    let mut encoder = GifEncoder::new_with_speed(std::fs::File::create(path)?, 30);
    encoder.set_repeat(Repeat::Infinite)?;
    for step in 0..FRAMES {
        // Up to an eighth closer at the middle of the loop.
        let phase = (step as f32 / FRAMES as f32 * std::f32::consts::TAU).cos();
        let zoom = 1.0 + 0.0625 * (1.0 - phase);
        let (crop_width, crop_height) =
            ((width as f32 / zoom) as u32, (height as f32 / zoom) as u32);
        let crop = image::imageops::crop_imm(
            &picture,
            (width - crop_width) / 2,
            (height - crop_height) / 2,
            crop_width,
            crop_height,
        )
        .to_image();
        let frame =
            image::imageops::resize(&crop, width, height, image::imageops::FilterType::Triangle);
        encoder.encode_frame(image::Frame::from_parts(
            frame,
            0,
            0,
            image::Delay::from_numer_denom_ms(80, 1),
        ))?;
    }
    Ok(())
}

pub fn populate(app: &mut App) -> Result<()> {
    let dir = dir(app);
    std::fs::create_dir_all(&dir)?;
    let animated = stock::save(&dir, "reaction-animated.webp", stock::ANIMATED_STICKER);
    let gifs = stock::GIFS
        .into_iter()
        .enumerate()
        .map(|(index, still)| Gif {
            id: format!("tour-{index}"),
            still: Some(stock::save(&dir, &format!("reaction-{index}.jpg"), still)),
            animated: Some(animated.clone()),
            animated_source: None,
            mp4: String::new(),
            width: 320,
            height: 240,
        })
        .collect();
    let stickers: Vec<_> = stock::STICKERS
        .into_iter()
        .take(6)
        .enumerate()
        .map(|(index, (_, sticker))| {
            stock::save(&dir, &format!("still-sticker-{index}.webp"), sticker)
        })
        .collect();
    app.settings.giphy_key = "offline-demo".into();
    app.gif_results = gifs;
    app.gif_pending = false;
    app.gif_error = None;
    app.stickers_saved = stickers[..3].to_vec();
    app.stickers = stickers.clone();
    app.sticker_packs = vec![StickerPack {
        name: "Launch party".into(),
        dir,
        stickers,
        local: false,
    }];
    app.stickers_pending = false;
    Ok(())
}
