//! Still GIF-search thumbnails and stickers from the bundled emoji font.

use anyhow::{Context, Result};
use image::{Rgba, RgbaImage};
use skrifa::{FontRef, MetadataProvider, bitmap::BitmapData, instance::Size};

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

pub fn populate(app: &mut App) -> Result<()> {
    let dir = app.dirs.media_cache_dir().join("tour");
    std::fs::create_dir_all(&dir)?;
    let mut gifs = Vec::new();
    let mut stickers = Vec::new();
    for (index, character) in ['🥳', '🎉', '🚀', '👋', '😎', '🐸'].into_iter().enumerate()
    {
        let still = dir.join(format!("reaction-{index}.png"));
        let sticker = dir.join(format!("still-sticker-{index}.webp"));
        if !sticker.exists() || !still.exists() {
            let resized = emoji_image(character, 136)?;
            let rgb = crate::theme::hsl_rgb(index as f32 * 57.0 + 210.0, 0.5, 0.78);
            let mut tile = RgbaImage::from_pixel(320, 240, Rgba([rgb[0], rgb[1], rgb[2], 255]));
            image::imageops::overlay(&mut tile, &resized, 92, 52);
            tile.save(&still)?;
            let mut tile = RgbaImage::new(192, 192);
            image::imageops::overlay(&mut tile, &resized, 28, 28);
            tile.save(&sticker)?;
        }

        gifs.push(Gif {
            id: format!("tour-{index}"),
            still: Some(still),
            mp4: String::new(),
            width: 320,
            height: 240,
        });
        stickers.push(sticker);
    }
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
