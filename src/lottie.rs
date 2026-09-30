//! Rasterizes Telegram TGS stickers into cached animated WebP.
//!
//! A TGS file is a gzipped Lottie animation. The player in
//! [`crate::animation`] already handles animated WebP (WhatsApp stickers use
//! that format), so a TGS sticker is rendered once, frame by frame, with the
//! pure-Rust rasterlottie rasterizer and cached as WebP beside the source.
//! Animations the rasterizer does not support fail here and the sticker keeps
//! its placeholder.

use std::io::Read;
use std::path::Path;

use anyhow::{Context, Result};

/// Largest side of a cached sticker. Telegram canvases are 512 px; stickers
/// are drawn at a fraction of that and the player scales what it gets.
const MAX_SIDE: u32 = 256;
/// Longest animation cached, mirroring the player's per-animation budget.
const MAX_FRAMES: u32 = 150;
/// Shortest frame delay written, so even tiny loops animate.
const MIN_FRAME_MS: i32 = 20;

/// Converts a TGS sticker into an animated WebP at `target` and returns the
/// pixel size of the converted animation.
pub fn convert_tgs(source: &Path, target: &Path) -> Result<(u32, u32)> {
    let animation = read_animation(source)?;
    let scale = (MAX_SIDE as f32 / animation.width.max(animation.height).max(1) as f32).min(1.0);
    let config = rasterlottie::RenderConfig::new(rasterlottie::Rgba8::TRANSPARENT, scale);
    let prepared = rasterlottie::Renderer::default()
        .prepare(&animation)
        .context("The TGS sticker uses animation features this rasterizer does not support")?;
    let mut pixmap = prepared
        .new_scratch_pixmap_for_config(config)
        .context("The TGS sticker has an invalid canvas")?;
    let (width, height) = (pixmap.width(), pixmap.height());
    let rate = animation.frame_rate.max(1.0);
    let span = (animation.out_point - animation.in_point).max(1.0).ceil() as u32;
    let step = span.div_ceil(MAX_FRAMES).max(1);
    let count = span.div_ceil(step).max(2);
    let frame_ms = ((step as f32 / rate) * 1000.0)
        .round()
        .max(MIN_FRAME_MS as f32) as i32;
    let mut encoder = webp_animation::Encoder::new((width, height))
        .map_err(|error| anyhow::anyhow!("The sticker encoder could not start: {error}"))?;
    for index in 0..count {
        let frame = animation.in_point + (index * step) as f32;
        prepared
            .render_frame_into_pixmap(frame, config, &mut pixmap)
            .context("A TGS sticker frame could not be rendered")?;
        let rgba = image::RgbaImage::from_raw(width, height, pixmap.data().to_vec())
            .context("A TGS sticker frame had an unexpected size")?;
        encoder
            .add_frame(&rgba, (index as i32) * frame_ms)
            .map_err(|error| anyhow::anyhow!("A sticker frame could not be encoded: {error}"))?;
    }
    let webp = encoder
        .finalize(count as i32 * frame_ms)
        .map_err(|error| anyhow::anyhow!("The sticker animation could not be encoded: {error}"))?;
    let partial = target.with_extension("webp.part");
    std::fs::write(&partial, &*webp).context("The sticker cache could not be written")?;
    std::fs::rename(&partial, target).context("The sticker cache could not be moved into place")?;
    Ok((width, height))
}

/// Reads a TGS file: gzipped Lottie JSON.
fn read_animation(source: &Path) -> Result<rasterlottie::Animation> {
    let mut compressed = Vec::new();
    std::fs::File::open(source)
        .context("The TGS sticker could not be opened")?
        .read_to_end(&mut compressed)?;
    let mut json = String::new();
    flate2::read::GzDecoder::new(compressed.as_slice())
        .read_to_string(&mut json)
        .context("The TGS sticker is not gzipped Lottie JSON")?;
    rasterlottie::Animation::from_json_str(&json).context("The TGS sticker could not be parsed")
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    /// An animated red rectangle, moving across a 64 px canvas.
    const STICKER: &str = r#"{
        "v":"5.7.6",
        "fr":30,
        "ip":0,
        "op":60,
        "w":64,
        "h":64,
        "layers":[
            {
                "nm":"Shape Layer 1",
                "ind":1,
                "ty":4,
                "shapes":[
                    {
                        "ty":"gr",
                        "it":[
                            {"ty":"rc","p":{"a":1,"k":[{"t":0,"s":[12,32],"e":[52,32],"i":{"x":[1,1],"y":[1,1]},"o":{"x":[0,0],"y":[0,0]}},{"t":10,"s":[52,32]}]},"s":{"a":0,"k":[16,16]},"r":{"a":0,"k":0}},
                            {"ty":"fl","c":{"a":0,"k":[1,0,0,1]},"o":{"a":0,"k":100}},
                            {"ty":"tr","a":{"a":0,"k":[0,0]},"p":{"a":0,"k":[0,0]},"s":{"a":0,"k":[100,100]},"r":{"a":0,"k":0},"o":{"a":0,"k":100}}
                        ]
                    }
                ]
            }
        ]
    }"#;

    fn gzip(text: &str) -> Vec<u8> {
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(text.as_bytes()).unwrap();
        encoder.finish().unwrap()
    }

    #[test]
    fn a_tgs_sticker_becomes_animated_webp() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("sticker");
        let target = directory.path().join("sticker.webp");
        std::fs::write(&source, gzip(STICKER)).unwrap();
        let size = convert_tgs(&source, &target).unwrap();
        assert_eq!(size, (64, 64));
        let bytes = std::fs::read(&target).unwrap();
        let decoder = webp_animation::Decoder::new(&bytes).unwrap();
        assert_eq!(decoder.dimensions(), (64, 64));
        assert!(decoder.into_iter().count() >= 2);
    }

    #[test]
    fn a_broken_tgs_leaves_no_cache() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("sticker");
        let target = directory.path().join("sticker.webp");
        std::fs::write(&source, b"not gzipped Lottie").unwrap();
        assert!(convert_tgs(&source, &target).is_err());
        assert!(!target.exists());
    }
}
