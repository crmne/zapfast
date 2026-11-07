//! Bounded, off-thread rasterization of received WhatsApp Lottie stickers.
//!
//! WhatsApp transports these as ZIP-based `.was` archives containing Lottie JSON,
//! not as WebP. The existing animation worker schedules decoding only for visible
//! stickers; paused stickers load only their poster.

use std::io::Read;
use std::path::Path;
use std::time::Duration;

use egui::ColorImage;
use rasterlottie::{Animation, RenderConfig, Renderer, Rgba8};

const MAX_ARCHIVE_BYTES: u64 = 4 * 1024 * 1024;
const MAX_JSON_BYTES: u64 = 2 * 1024 * 1024;
const MAX_CANVAS_SIDE: u32 = 1024;
const MAX_DURATION: f32 = 15.0;
const MAX_FPS: f32 = 24.0;
const MAX_FRAMES: usize = 72;
const MAX_OUTPUT_SIDE: f32 = 320.0;

/// Read just the animation JSON. Ignore metadata and trust tokens, and never
/// extract archive members onto disk. Size caps also bound ZIP decompression.
fn animation_json(path: &Path) -> Option<String> {
    let file = std::fs::File::open(path).ok()?;
    if file.metadata().ok()?.len() > MAX_ARCHIVE_BYTES {
        return None;
    }
    let mut archive = zip::ZipArchive::new(file).ok()?;
    let mut fallback = None;
    let mut preferred = None;
    for (index, name) in archive.file_names().enumerate() {
        // A name zip cannot read is no animation: its neighbours still count.
        let Ok(name) = name else { continue };
        if name == "animation.json" || name.ends_with("/animation.json") {
            preferred = Some(index);
            break;
        }
        if name.ends_with(".json") && !name.contains("metadata") && !name.contains("trust_token") {
            fallback.get_or_insert(index);
        }
    }
    let entry = archive.by_index(preferred.or(fallback)?).ok()?;
    if entry.size() > MAX_JSON_BYTES {
        return None;
    }
    let mut data = Vec::with_capacity(entry.size() as usize);
    entry.take(MAX_JSON_BYTES + 1).read_to_end(&mut data).ok()?;
    if data.len() as u64 > MAX_JSON_BYTES {
        return None;
    }
    String::from_utf8(data).ok()
}

/// Rasterize at most `requested` frames. One requested frame is the poster;
/// the shared animation cache only requests the rest during actual playback.
pub(crate) fn decode(path: &Path, requested: usize) -> Option<Vec<(ColorImage, Duration)>> {
    if requested == 0 {
        return None;
    }
    let json = animation_json(path)?;
    let animation = Animation::from_json_str(&json).ok()?;
    if animation.width == 0
        || animation.height == 0
        || animation.width > MAX_CANVAS_SIDE
        || animation.height > MAX_CANVAS_SIDE
        || !animation.frame_rate.is_finite()
        || animation.frame_rate <= 0.0
        || !animation.in_point.is_finite()
        || !animation.out_point.is_finite()
    {
        return None;
    }
    let seconds = animation.duration_seconds();
    if !seconds.is_finite() || seconds <= 0.0 || seconds > MAX_DURATION {
        return None;
    }

    // The prepared renderer caches tessellation and reusable layer geometry.
    // Repreparing each frame would be expensive for complicated stickers.
    let prepared = Renderer::default().prepare(&animation).ok()?;
    let scale = (MAX_OUTPUT_SIDE / animation.width.max(animation.height) as f32).min(1.0);
    let config = RenderConfig::new(Rgba8::TRANSPARENT, scale);
    let fps = animation.frame_rate.min(MAX_FPS);
    let count = ((seconds * fps).ceil() as usize)
        .max(1)
        .min(requested)
        .min(MAX_FRAMES);
    let delay = Duration::from_secs_f32(seconds / count as f32).max(Duration::from_millis(20));
    let mut frames = Vec::with_capacity(count);
    for index in 0..count {
        let at = animation.in_point + index as f32 * animation.duration_frames() / count as f32;
        let raster = prepared.render_frame(at, config).ok()?;
        // rasterlottie uses premultiplied RGBA pixels, as egui expects here.
        frames.push((
            ColorImage::from_rgba_premultiplied(
                [raster.width as usize, raster.height as usize],
                &raster.pixels,
            ),
            delay,
        ));
    }
    Some(frames)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    const JSON: &str = r#"{"v":"5.7.6","fr":30,"ip":0,"op":30,"w":64,"h":64,"layers":[]}"#;

    fn archive(path: &Path, json: &str) {
        let file = std::fs::File::create(path).expect("archive file");
        let mut zip = zip::ZipWriter::new(file);
        zip.start_file("animation.json", zip::write::SimpleFileOptions::default())
            .expect("entry");
        zip.write_all(json.as_bytes()).expect("write");
        zip.finish().expect("finish");
    }

    #[test]
    fn reads_was_and_only_renders_one_frame_when_paused() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("sticker.was");
        archive(&path, JSON);
        let poster = decode(&path, 1).expect("poster");
        assert_eq!(poster.len(), 1);
        assert_eq!(poster[0].0.size, [64, 64]);
        let playing = decode(&path, 150).expect("frames");
        assert_eq!(playing.len(), 24);
        assert!(
            playing
                .iter()
                .all(|(_, delay)| *delay >= Duration::from_millis(20))
        );
    }

    #[test]
    fn rejects_invalid_json_canvas_and_large_archives() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("bad.was");
        archive(&path, "{not valid JSON");
        assert!(decode(&path, 1).is_none());
        archive(&path, &JSON.replace("\"w\":64", "\"w\":5000"));
        assert!(decode(&path, 1).is_none());
        std::fs::write(&path, vec![0u8; MAX_ARCHIVE_BYTES as usize + 1]).expect("oversized");
        assert!(decode(&path, 1).is_none());
    }
}
