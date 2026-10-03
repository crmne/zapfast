//! Still GIF-search thumbnails and stickers from the stock demo media, and
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

pub fn populate(app: &mut App) -> Result<()> {
    let dir = app.dirs.media_cache_dir().join("tour");
    std::fs::create_dir_all(&dir)?;
    let gifs = stock::GIFS
        .into_iter()
        .enumerate()
        .map(|(index, still)| Gif {
            id: format!("tour-{index}"),
            still: Some(stock::save(&dir, &format!("reaction-{index}.jpg"), still)),
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
