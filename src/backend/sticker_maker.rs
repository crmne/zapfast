//! Turns a picture into a WhatsApp sticker: a square crop scaled to 512
//! pixels, encoded as a WebP under WhatsApp's size limits, with the chosen
//! emojis in its metadata. An animated GIF, WebP or APNG, or an H.264 video
//! of up to 10 seconds, becomes an animated sticker, and one colour can be made
//! see-through.

use std::sync::Arc;

use crate::model::{ColorKey, StickerCrop};

/// WhatsApp's sticker side, in pixels.
pub const SIDE: u32 = 512;
/// WhatsApp rejects still stickers larger than this.
const LIMIT: usize = 100 * 1024;
/// WhatsApp rejects animated stickers larger than this.
const ANIMATED_LIMIT: usize = 500 * 1024;
/// WhatsApp plays at most this much of an animated sticker.
const LONGEST_MS: u32 = 10_000;
/// The side of the poster the maker samples and previews from.
const POSTER_SIDE: u32 = 360;
/// The longest video that becomes a sticker: as long as WhatsApp plays.
const LONGEST_VIDEO_MS: u32 = LONGEST_MS;
/// How much longer than [`LONGEST_VIDEO_MS`] a video may say it lasts, as a
/// clip trimmed to 10 seconds often does; the excess is dropped.
const VIDEO_SLACK_MS: u32 = 500;
/// The least time between a video's frames, about 15 a second: a sticker
/// plays no smoother, and every frame held costs memory and size.
const VIDEO_FRAME_MS: u32 = 66;
/// Most frames read from a moving picture.
const MOST_FRAMES: usize = 200;
/// Most pixels the frames of one moving picture hold, 128 MB of RGBA. Past
/// it frames are dropped evenly, each kept one showing for the time of those
/// dropped after it, so a large GIF cannot take gigabytes while it decodes.
const DECODED_PIXELS: u64 = 32 * 1024 * 1024;

/// What the sticker maker needs to know about a picture before it opens.
#[derive(Clone, Debug, PartialEq)]
pub struct Inspected {
    pub width: u32,
    pub height: u32,
    /// Whether any pixel is see-through.
    pub transparent: bool,
    /// Whether it moves, and so becomes an animated sticker.
    pub animated: bool,
    /// The first frame, at most [`POSTER_SIDE`] pixels on its longer side.
    pub poster: Arc<image::RgbaImage>,
}

/// A picture's size, whether it has any transparent pixel or moves, and its
/// first frame.
pub fn inspect(bytes: &[u8]) -> Result<Inspected, String> {
    let frames = moving_frames(bytes)?;
    let first = match &frames {
        Some(frames) => frames[0].0.clone(),
        None => image::load_from_memory(bytes)
            .map_err(|error| format!("This picture could not be read: {error}"))?
            .to_rgba8(),
    };
    let transparent = match &frames {
        Some(frames) => frames
            .iter()
            .any(|(frame, _)| frame.pixels().any(|pixel| pixel[3] < 255)),
        None => first.pixels().any(|pixel| pixel[3] < 255),
    };
    let (width, height) = first.dimensions();
    let poster = if width.max(height) > POSTER_SIDE {
        image::imageops::thumbnail(
            &first,
            (width * POSTER_SIDE / width.max(height)).max(1),
            (height * POSTER_SIDE / width.max(height)).max(1),
        )
    } else {
        first
    };
    Ok(Inspected {
        width,
        height,
        transparent,
        animated: frames.is_some(),
        poster: Arc::new(poster),
    })
}

/// Makes the sticker. Without `transparent`, see-through areas become
/// white; a colour `key` turns its colour see-through first, and keeps the
/// result see-through whatever `transparent` says.
pub fn make(
    bytes: &[u8],
    crop: StickerCrop,
    transparent: bool,
    key: Option<ColorKey>,
    emojis: &[String],
) -> Result<Vec<u8>, String> {
    let transparent = transparent || key.is_some();
    let webp = match moving_frames(bytes)? {
        Some(frames) => {
            let (width, height) = frames[0].0.dimensions();
            let crop = crop.clamped(width, height);
            // What lies beyond the picture's edges stays see-through.
            let transparent = transparent || !crop.inside(width, height);
            let frames = frames
                .into_iter()
                .map(|(frame, delay)| {
                    let frame = shape(
                        &frame,
                        crop,
                        transparent,
                        key,
                        image::imageops::FilterType::Triangle,
                    );
                    (frame, delay)
                })
                .collect();
            encode_animated_sticker(frames)?
        }
        None => {
            let picture = image::load_from_memory(bytes)
                .map_err(|error| format!("This picture could not be read: {error}"))?
                .to_rgba8();
            let crop = crop.clamped(picture.width(), picture.height());
            let transparent = transparent || !crop.inside(picture.width(), picture.height());
            let sticker = shape(
                &picture,
                crop,
                transparent,
                key,
                image::imageops::FilterType::Lanczos3,
            );
            encode(&sticker)?
        }
    };
    let info = crate::sticker_meta::StickerInfo {
        pack_id: "zapfast".to_owned(),
        pack_name: "ZapFast".to_owned(),
        emojis: emojis.to_vec(),
        ..Default::default()
    };
    Ok(crate::sticker_meta::write(&webp, &info).unwrap_or(webp))
}

/// One frame of the sticker: the crop, at `side` pixels, keyed and, without
/// `transparent`, on white. The maker's preview draws the same.
pub fn shape_at(
    picture: &image::RgbaImage,
    crop: StickerCrop,
    transparent: bool,
    key: Option<ColorKey>,
    side: u32,
    filter: image::imageops::FilterType,
) -> image::RgbaImage {
    let mut sticker = place(picture, crop, side, filter);
    if let Some(key) = key {
        key_out(&mut sticker, key);
    }
    if !transparent {
        for pixel in sticker.pixels_mut() {
            let alpha = f32::from(pixel[3]) / 255.0;
            for channel in 0..3 {
                pixel[channel] =
                    (f32::from(pixel[channel]) * alpha + 255.0 * (1.0 - alpha)).round() as u8;
            }
            pixel[3] = 255;
        }
    }
    sticker
}

/// The part of `picture` inside `crop`, drawn at `side` pixels on a
/// see-through square. Only the overlap is scaled, so a crop much larger
/// than the picture costs no more than the sticker itself.
fn place(
    picture: &image::RgbaImage,
    crop: StickerCrop,
    side: u32,
    filter: image::imageops::FilterType,
) -> image::RgbaImage {
    let (width, height) = (i64::from(picture.width()), i64::from(picture.height()));
    let span = i64::from(crop.side.max(1));
    let left = crop.x.max(0);
    let top = crop.y.max(0);
    let right = (crop.x + span).min(width);
    let bottom = (crop.y + span).min(height);
    if crop.inside(picture.width(), picture.height()) {
        let square =
            image::imageops::crop_imm(picture, crop.x as u32, crop.y as u32, crop.side, crop.side)
                .to_image();
        return image::imageops::resize(&square, side, side, filter);
    }
    let mut canvas = image::RgbaImage::new(side, side);
    if right <= left || bottom <= top {
        return canvas;
    }
    let scale = f64::from(side) / span as f64;
    let to = |at: i64, from: i64| ((at - from) as f64 * scale).round() as i64;
    let (x0, y0) = (to(left, crop.x), to(top, crop.y));
    let (x1, y1) = (to(right, crop.x), to(bottom, crop.y));
    let part = image::imageops::crop_imm(
        picture,
        left as u32,
        top as u32,
        (right - left) as u32,
        (bottom - top) as u32,
    )
    .to_image();
    let scaled = image::imageops::resize(
        &part,
        (x1 - x0).max(1) as u32,
        (y1 - y0).max(1) as u32,
        filter,
    );
    image::imageops::replace(&mut canvas, &scaled, x0, y0);
    canvas
}

fn shape(
    picture: &image::RgbaImage,
    crop: StickerCrop,
    transparent: bool,
    key: Option<ColorKey>,
    filter: image::imageops::FilterType,
) -> image::RgbaImage {
    shape_at(picture, crop, transparent, key, SIDE, filter)
}

/// Makes the pixels close to `key`'s colour see-through. Within its
/// tolerance they vanish; just beyond it they fade, so edges do not fringe.
pub fn key_out(picture: &mut image::RgbaImage, key: ColorKey) {
    let tolerance = i32::from(key.tolerance);
    let feather = tolerance / 2 + 8;
    for pixel in picture.pixels_mut() {
        let distance = (0..3)
            .map(|channel| (i32::from(pixel[channel]) - i32::from(key.color[channel])).abs())
            .max()
            .unwrap_or(0);
        if distance <= tolerance {
            pixel[3] = 0;
        } else if distance <= tolerance + feather {
            let kept = (distance - tolerance) as f32 / feather as f32;
            pixel[3] = (f32::from(pixel[3]) * kept).round() as u8;
        }
    }
}

/// Frames and millisecond delays for a video or an animated picture, or
/// `None` for a picture that does not move. Says why a video cannot become a
/// sticker.
fn moving_frames(bytes: &[u8]) -> Result<Option<Vec<(image::RgbaImage, u32)>>, String> {
    if is_video(bytes) {
        let frames = video_frames(bytes)?;
        return Ok((frames.len() > 1).then_some(frames));
    }
    Ok(animation_frames(bytes))
}

/// Whether `bytes` is an MP4 or QuickTime file, not a HEIF or AVIF picture,
/// which share the container.
fn is_video(bytes: &[u8]) -> bool {
    bytes.len() >= 12
        && &bytes[4..8] == b"ftyp"
        && !matches!(
            &bytes[8..12],
            b"avif" | b"avis" | b"heic" | b"heix" | b"heim" | b"heis" | b"mif1" | b"msf1"
        )
}

/// The frames of an H.264 video of up to 10 seconds, about 15 a second, with
/// the shorter side at most [`SIDE`] pixels and turned upright.
fn video_frames(bytes: &[u8]) -> Result<Vec<(image::RgbaImage, u32)>, String> {
    use std::cmp::Reverse;
    let unreadable = |error: String| format!("This video could not be read: {error}");
    let mut mp4 = mp4::Mp4Reader::read_header(std::io::Cursor::new(bytes), bytes.len() as u64)
        .map_err(|error| unreadable(error.to_string()))?;
    if mp4.duration()
        > std::time::Duration::from_millis(u64::from(LONGEST_VIDEO_MS + VIDEO_SLACK_MS))
    {
        return Err("This video is longer than 10 seconds".to_owned());
    }
    let (track_id, count, timescale, sps, pps, turns) = {
        let track = mp4
            .tracks()
            .values()
            .find(|track| track.track_type().ok() == Some(mp4::TrackType::Video))
            .ok_or_else(|| unreadable("it has no picture".to_owned()))?;
        if track.media_type().ok() != Some(mp4::MediaType::H264) {
            return Err("Only H.264 videos can become stickers".to_owned());
        }
        let matrix = &track.trak.tkhd.matrix;
        (
            track.track_id(),
            track.sample_count(),
            i64::from(track.timescale().max(1)),
            track
                .sequence_parameter_set()
                .map_err(|error| unreadable(error.to_string()))?
                .to_vec(),
            track
                .picture_parameter_set()
                .map_err(|error| unreadable(error.to_string()))?
                .to_vec(),
            crate::video::quarter_turns(matrix.a, matrix.b, matrix.c, matrix.d),
        )
    };
    // As in video playback: flushing after every packet breaks streams with
    // B-frames, so frames come out reordered and their times from a heap.
    let mut decoder = openh264::decoder::Decoder::with_api_config(
        openh264::OpenH264API::from_source(),
        openh264::decoder::DecoderConfig::new()
            .flush_after_decode(openh264::decoder::Flush::NoFlush),
    )
    .map_err(|error| unreadable(error.to_string()))?;
    let mut parameters = Vec::new();
    crate::animation::push_annex_b(&mut parameters, &sps);
    crate::animation::push_annex_b(&mut parameters, &pps);
    let _ = decoder.decode(&parameters);
    let mut times = std::collections::BinaryHeap::new();
    // Frames kept, each with the millisecond it shows at.
    let mut kept: Vec<(image::RgbaImage, u32)> = Vec::new();
    let mut first = None;
    let mut interval = VIDEO_FRAME_MS;
    let length = mp4
        .duration()
        .as_millis()
        .clamp(1, u128::from(LONGEST_VIDEO_MS)) as u64;
    let mut keep = |yuv: &openh264::decoder::DecodedYUV<'_>, at: i64| -> bool {
        let at = (at * 1000 / timescale).max(0);
        let at = (at - *first.get_or_insert(at)) as u32;
        if at >= LONGEST_VIDEO_MS {
            return false;
        }
        if kept.last().is_none_or(|(_, last)| at >= last + interval)
            && let Some(frame) = video_frame(yuv, turns)
        {
            // Fewer frames a second for a large picture, within the budget.
            let fit = (DECODED_PIXELS / pixels(&frame)).max(1);
            interval = interval.max((length / fit) as u32);
            kept.push((frame, at));
        }
        true
    };
    for sample_id in 1..=count {
        let Ok(Some(sample)) = mp4.read_sample(track_id, sample_id) else {
            break;
        };
        times.push(Reverse(
            sample.start_time as i64 + i64::from(sample.rendering_offset),
        ));
        let mut annex_b = Vec::with_capacity(sample.bytes.len() + 16);
        crate::animation::avcc_to_annex_b(&mut annex_b, &sample.bytes);
        if let Ok(Some(yuv)) = decoder.decode(&annex_b) {
            let Reverse(at) = times.pop().unwrap_or_default();
            if !keep(&yuv, at) {
                break;
            }
        }
    }
    for yuv in &decoder.flush_remaining().unwrap_or_default() {
        let Reverse(at) = times.pop().unwrap_or_default();
        if !keep(yuv, at) {
            break;
        }
    }
    if kept.is_empty() {
        return Err(unreadable("no frame decoded".to_owned()));
    }
    // Each frame shows until the next; the last for one frame's time.
    let starts: Vec<u32> = kept.iter().map(|(_, at)| *at).collect();
    Ok(kept
        .into_iter()
        .enumerate()
        .map(|(index, (frame, at))| {
            let next = starts
                .get(index + 1)
                .copied()
                .unwrap_or(at + VIDEO_FRAME_MS);
            (frame, next - at)
        })
        .collect())
}

/// One decoded video frame, its shorter side at most [`SIDE`] pixels, turned
/// `turns` quarter turns clockwise.
fn video_frame(yuv: &openh264::decoder::DecodedYUV<'_>, turns: u8) -> Option<image::RgbaImage> {
    use openh264::formats::YUVSource;
    let (width, height) = yuv.dimensions();
    if width == 0 || height == 0 {
        return None;
    }
    let mut rgba = vec![0u8; width * height * 4];
    yuv.write_rgba8(&mut rgba);
    let frame = image::RgbaImage::from_raw(width as u32, height as u32, rgba)?;
    let frame = fit(frame);
    Some(match turns {
        1 => image::imageops::rotate90(&frame),
        2 => image::imageops::rotate180(&frame),
        3 => image::imageops::rotate270(&frame),
        _ => frame,
    })
}

/// `frame` with its shorter side at most [`SIDE`] pixels: a sticker is no
/// larger, and a smaller frame keeps a long animation's memory down.
fn fit(frame: image::RgbaImage) -> image::RgbaImage {
    let (width, height) = frame.dimensions();
    let shorter = width.min(height);
    if shorter <= SIDE {
        return frame;
    }
    image::imageops::resize(
        &frame,
        (u64::from(width) * u64::from(SIDE) / u64::from(shorter)).max(1) as u32,
        (u64::from(height) * u64::from(SIDE) / u64::from(shorter)).max(1) as u32,
        image::imageops::FilterType::Triangle,
    )
}

fn pixels(frame: &image::RgbaImage) -> u64 {
    (u64::from(frame.width()) * u64::from(frame.height())).max(1)
}

/// Frames of a moving picture as they decode, fitted, and thinned evenly to
/// stay within [`DECODED_PIXELS`].
struct Frames {
    kept: Vec<(image::RgbaImage, u32)>,
    /// Every how many decoded frames one is kept.
    stride: usize,
    seen: usize,
}

impl Frames {
    fn new() -> Self {
        Self {
            kept: Vec::new(),
            stride: 1,
            seen: 0,
        }
    }

    /// Adds a decoded frame shown for `delay` milliseconds. `frame` is only
    /// called for a frame that is kept.
    fn add(&mut self, frame: impl FnOnce() -> Option<image::RgbaImage>, delay: u32) {
        let index = self.seen;
        self.seen += 1;
        if !index.is_multiple_of(self.stride) {
            if let Some((_, last)) = self.kept.last_mut() {
                *last += delay;
            }
            return;
        }
        let Some(frame) = frame() else {
            return;
        };
        self.kept.push((fit(frame), delay));
        while self.kept.len() > 1
            && self
                .kept
                .iter()
                .map(|(frame, _)| pixels(frame))
                .sum::<u64>()
                > DECODED_PIXELS
        {
            self.kept = std::mem::take(&mut self.kept)
                .chunks(2)
                .map(|pair| {
                    let delay = pair.iter().map(|(_, delay)| delay).sum();
                    (pair[0].0.clone(), delay)
                })
                .collect();
            self.stride *= 2;
        }
    }
}

/// Returns frames and millisecond delays for animated GIF, WebP or APNG
/// data, or `None` for a picture that does not move.
pub(super) fn animation_frames(bytes: &[u8]) -> Option<Vec<(image::RgbaImage, u32)>> {
    use image::AnimationDecoder;
    let cursor = std::io::Cursor::new(bytes);
    let frames: Vec<(image::RgbaImage, u32)> = if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        let decoder = image::codecs::png::PngDecoder::new(cursor).ok()?;
        if !decoder.is_apng().ok()? {
            return None;
        }
        collect_frames(decoder.apng().ok()?.into_frames())
    } else if bytes.starts_with(b"GIF8") {
        collect_frames(
            image::codecs::gif::GifDecoder::new(cursor)
                .ok()?
                .into_frames(),
        )
    } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        webp_frames(bytes)?
    } else {
        return None;
    };
    (frames.len() > 1).then_some(frames)
}

/// Animated WebP frames through libwebp, which composes each frame on the
/// canvas; the `image` decoder does not apply frame disposal correctly.
fn webp_frames(bytes: &[u8]) -> Option<Vec<(image::RgbaImage, u32)>> {
    let decoder = webp_animation::Decoder::new(bytes).ok()?;
    let (width, height) = decoder.dimensions();
    let mut previous = 0;
    let mut frames = Frames::new();
    // Each frame's timestamp is when it ends.
    for frame in decoder.into_iter().take(MOST_FRAMES) {
        let delay = (frame.timestamp() - previous).max(0) as u32;
        previous = frame.timestamp();
        frames.add(
            || image::RgbaImage::from_raw(width, height, frame.data().to_vec()),
            if delay < 20 { 100 } else { delay },
        );
    }
    Some(frames.kept)
}

fn collect_frames(frames: image::Frames<'_>) -> Vec<(image::RgbaImage, u32)> {
    let mut kept = Frames::new();
    for frame in frames.take(MOST_FRAMES).filter_map(|frame| frame.ok()) {
        let (numerator, denominator) = frame.delay().numer_denom_ms();
        let delay = numerator / denominator.max(1);
        // A GIF that asks for no delay plays at the browsers' 100 ms.
        let delay = if delay < 20 { 100 } else { delay };
        kept.add(|| Some(frame.into_buffer()), delay);
    }
    kept.kept
}

/// Encodes frames as a lossy animated WebP at their own size.
pub(super) fn encode_frames(frames: &[(image::RgbaImage, u32)], quality: f32) -> Option<Vec<u8>> {
    use webp_animation::prelude::*;
    let (width, height) = frames.first().map(|(frame, _)| frame.dimensions())?;
    let mut encoder = Encoder::new_with_options(
        (width, height),
        EncoderOptions {
            encoding_config: Some(EncodingConfig {
                quality,
                encoding_type: EncodingType::Lossy(LossyEncodingConfig::default()),
                ..Default::default()
            }),
            ..Default::default()
        },
    )
    .ok()?;
    let mut clock = 0i32;
    for (frame, delay) in frames {
        encoder.add_frame(frame.as_raw(), clock).ok()?;
        clock += (*delay).clamp(10, 10_000) as i32;
    }
    Some(encoder.finalize(clock).ok()?.to_vec())
}

/// An animated sticker within WhatsApp's limits: at most ten seconds, and
/// under 500 KB, lowering the quality first and then dropping every other
/// frame, its time going to the frame before it.
fn encode_animated_sticker(mut frames: Vec<(image::RgbaImage, u32)>) -> Result<Vec<u8>, String> {
    let mut played = 0;
    frames.retain(|(_, delay)| {
        let keep = played < LONGEST_MS;
        played += delay;
        keep
    });
    loop {
        for quality in [75.0, 55.0, 35.0] {
            let webp = encode_frames(&frames, quality)
                .ok_or_else(|| "Could not encode the sticker".to_owned())?;
            if webp.len() <= ANIMATED_LIMIT {
                return Ok(webp);
            }
        }
        if frames.len() < 4 {
            return Err("This animation is too large for a sticker".to_owned());
        }
        frames = frames
            .chunks(2)
            .map(|pair| {
                let delay = pair.iter().map(|(_, delay)| delay).sum();
                (pair[0].0.clone(), delay)
            })
            .collect();
    }
}

/// A lossy still WebP, lowering quality until it fits WhatsApp's limit.
fn encode(sticker: &image::RgbaImage) -> Result<Vec<u8>, String> {
    let mut smallest = Vec::new();
    for quality in [85.0, 70.0, 55.0, 40.0, 25.0] {
        let webp = encode_at(sticker, quality)?;
        if webp.len() <= LIMIT {
            return Ok(webp);
        }
        smallest = webp;
    }
    Ok(smallest)
}

/// One pass of libwebp's simple lossy encoder, which keeps the alpha channel.
fn encode_at(sticker: &image::RgbaImage, quality: f32) -> Result<Vec<u8>, String> {
    let (width, height) = sticker.dimensions();
    let mut output: *mut u8 = std::ptr::null_mut();
    // SAFETY: the buffer holds width * height RGBA pixels with a stride of
    // width * 4, and libwebp allocates `output`, which is copied out and
    // released with WebPFree before returning.
    let bytes = unsafe {
        let size = libwebp_sys::WebPEncodeRGBA(
            sticker.as_raw().as_ptr(),
            width as i32,
            height as i32,
            (width * 4) as i32,
            quality,
            &mut output,
        );
        if output.is_null() {
            return Err("Could not encode the sticker".to_owned());
        }
        let bytes = std::slice::from_raw_parts(output, size).to_vec();
        libwebp_sys::WebPFree(output.cast());
        bytes
    };
    if bytes.is_empty() {
        return Err("Could not encode the sticker".to_owned());
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(width: u32, height: u32, see_through: bool) -> Vec<u8> {
        use image::ImageEncoder;
        let picture = image::RgbaImage::from_fn(width, height, |x, _| {
            let alpha = if see_through && x < width / 2 { 0 } else { 255 };
            image::Rgba([200, (x % 256) as u8, 40, alpha])
        });
        let mut bytes = Vec::new();
        image::codecs::png::PngEncoder::new(&mut bytes)
            .write_image(&picture, width, height, image::ExtendedColorType::Rgba8)
            .expect("encodes");
        bytes
    }

    #[test]
    fn a_picture_becomes_a_still_512_pixel_sticker_with_its_emojis() {
        let source = png(800, 600, true);
        let inspected = inspect(&source).expect("reads");
        assert_eq!(
            (
                inspected.width,
                inspected.height,
                inspected.transparent,
                inspected.animated
            ),
            (800, 600, true, false)
        );
        assert_eq!(
            inspected.poster.dimensions(),
            (360, 270),
            "a scaled-down poster"
        );
        let crop = StickerCrop::centered(800, 600);
        assert_eq!((crop.x, crop.y, crop.side), (100, 0, 600));
        assert!(crop.inside(800, 600));
        let sticker = make(&source, crop, true, None, &["🎨".to_owned()]).expect("makes");
        assert!(sticker.len() <= LIMIT);
        let decoder =
            image::codecs::webp::WebPDecoder::new(std::io::Cursor::new(&sticker)).expect("webp");
        assert!(!decoder.has_animation(), "a still sticker");
        let picture = image::load_from_memory(&sticker)
            .expect("decodes")
            .to_rgba8();
        assert_eq!(picture.dimensions(), (SIDE, SIDE));
        assert!(
            picture.get_pixel(5, 256)[3] < 16,
            "the see-through half stays"
        );
        assert_eq!(crate::sticker_meta::emojis(&sticker), vec!["🎨"]);
    }

    /// A GIF of `count` 64x64 frames on a green background, a red square
    /// moving across it.
    /// A long, large animation keeps evenly spaced frames within the pixel
    /// budget, and still plays for as long.
    #[test]
    fn large_animations_drop_frames_evenly_to_fit_the_budget() {
        let mut frames = Frames::new();
        let mut made = 0;
        for _ in 0..40 {
            frames.add(
                || {
                    made += 1;
                    Some(image::RgbaImage::new(2000, 600))
                },
                50,
            );
        }
        let kept = frames.kept;
        assert!(made < 40, "dropped frames are never decoded into pictures");
        assert_eq!(kept[0].0.dimensions(), (1706, 512));
        assert!(kept.iter().map(|(frame, _)| pixels(frame)).sum::<u64>() <= DECODED_PIXELS);
        assert_eq!(kept.len(), 20);
        assert!(kept.iter().all(|(_, delay)| *delay == 100));
    }

    fn gif(count: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        {
            let mut encoder = image::codecs::gif::GifEncoder::new(&mut bytes);
            for index in 0..count {
                let picture = image::RgbaImage::from_fn(64, 64, |x, y| {
                    let inside = (8..24).contains(&y) && x >= index * 4 && x < index * 4 + 16;
                    if inside {
                        image::Rgba([220, 20, 20, 255])
                    } else {
                        image::Rgba([0, 200, 0, 255])
                    }
                });
                let delay = image::Delay::from_numer_denom_ms(0, 1);
                encoder
                    .encode_frame(image::Frame::from_parts(picture, 0, 0, delay))
                    .expect("frame");
            }
        }
        bytes
    }

    #[test]
    fn a_gif_becomes_an_animated_sticker_with_its_background_keyed_out() {
        let source = gif(6);
        let inspected = inspect(&source).expect("reads");
        assert!(inspected.animated);
        assert!(!inspected.transparent);
        let corner = *inspected.poster.get_pixel(0, 0);
        let key = ColorKey {
            color: [corner[0], corner[1], corner[2]],
            tolerance: 40,
        };
        let sticker = make(
            &source,
            StickerCrop::centered(64, 64),
            false,
            Some(key),
            &["🎨".to_owned()],
        )
        .expect("makes");
        assert!(sticker.len() <= ANIMATED_LIMIT);
        assert_eq!(crate::sticker_meta::emojis(&sticker), vec!["🎨"]);
        let decoder =
            image::codecs::webp::WebPDecoder::new(std::io::Cursor::new(&sticker)).expect("webp");
        assert!(decoder.has_animation(), "an animated sticker");
        assert_eq!(image::ImageDecoder::dimensions(&decoder), (SIDE, SIDE));
        use image::AnimationDecoder;
        let frames: Vec<_> = decoder
            .into_frames()
            .collect::<Result<_, _>>()
            .expect("frames");
        assert_eq!(frames.len(), 6);
        let first = frames[0].buffer();
        assert!(
            first.get_pixel(256, 480)[3] < 16,
            "the green background is gone"
        );
        assert!(first.get_pixel(40, 120)[3] > 240, "the red square stays");
        let delay = frames[0].delay().numer_denom_ms();
        assert_eq!(
            delay.0 / delay.1.max(1),
            100,
            "a zero delay plays at 100 ms"
        );
    }

    #[test]
    fn a_short_video_becomes_an_animated_sticker() {
        let source = include_bytes!("../../tests/fixtures/video/sample.mp4");
        let inspected = inspect(source).expect("reads");
        assert!(inspected.animated);
        assert_eq!((inspected.width, inspected.height), (320, 180));
        let frames = video_frames(source).expect("decodes");
        // 3 seconds at 15 frames a second, give or take a frame.
        assert!((40..=50).contains(&frames.len()), "{} frames", frames.len());
        let played: u32 = frames.iter().map(|(_, delay)| delay).sum();
        assert!((2_800..=3_200).contains(&played), "{played} ms");
        let sticker = make(
            source,
            StickerCrop::centered(320, 180),
            false,
            None,
            &["🎬".to_owned()],
        )
        .expect("makes");
        assert!(sticker.len() <= ANIMATED_LIMIT);
        let decoder =
            image::codecs::webp::WebPDecoder::new(std::io::Cursor::new(&sticker)).expect("webp");
        assert!(decoder.has_animation(), "an animated sticker");
        assert_eq!(image::ImageDecoder::dimensions(&decoder), (SIDE, SIDE));
    }

    #[test]
    fn a_video_longer_than_10_seconds_is_refused() {
        // The fixture's movie header says 3 seconds; claim 12 instead.
        let mut source = include_bytes!("../../tests/fixtures/video/sample.mp4").to_vec();
        let at = source
            .windows(4)
            .position(|window| window == b"mvhd")
            .expect("a movie header");
        let version = source[at + 4];
        let (scale_at, length_at) = if version == 1 {
            (at + 24, at + 28)
        } else {
            (at + 16, at + 20)
        };
        let scale = u32::from_be_bytes(source[scale_at..scale_at + 4].try_into().unwrap());
        let length = scale * 12;
        if version == 1 {
            source[length_at..length_at + 8].copy_from_slice(&u64::from(length).to_be_bytes());
        } else {
            source[length_at..length_at + 4].copy_from_slice(&length.to_be_bytes());
        }
        assert_eq!(
            inspect(&source).map(|_| ()),
            Err("This video is longer than 10 seconds".to_owned())
        );
    }

    #[test]
    fn an_animated_webp_keeps_its_frames() {
        let frames: Vec<_> = (0..3u8)
            .map(|index| {
                let picture = image::RgbaImage::from_fn(32, 32, |x, _| {
                    if x < 8 * (u32::from(index) + 1) {
                        image::Rgba([0, 0, 255, 255])
                    } else {
                        image::Rgba([0, 0, 0, 0])
                    }
                });
                (picture, 120)
            })
            .collect();
        let webp = encode_frames(&frames, 90.0).expect("encodes");
        let decoded = animation_frames(&webp).expect("moves");
        assert_eq!(decoded.len(), 3);
        assert_eq!(decoded[0].1, 120);
        assert!(
            decoded[2].0.get_pixel(20, 16)[3] > 240,
            "the last frame is whole"
        );
        assert!(inspect(&webp).expect("reads").animated);
    }

    #[test]
    fn a_crop_past_the_edges_shrinks_the_picture_on_a_clear_square() {
        // An opaque picture, made smaller in the sticker than its frame.
        let source = png(100, 100, false);
        let crop = StickerCrop::centered(100, 100).resized(200, 100, 100);
        assert_eq!((crop.x, crop.y, crop.side), (-50, -50, 200));
        assert!(!crop.inside(100, 100));
        let sticker = make(&source, crop, false, None, &[]).expect("makes");
        let picture = image::load_from_memory(&sticker)
            .expect("decodes")
            .to_rgba8();
        assert_eq!(picture.dimensions(), (SIDE, SIDE));
        assert!(
            picture.get_pixel(20, 20)[3] < 16,
            "the margin is see-through"
        );
        assert!(
            picture.get_pixel(256, 256)[3] > 240,
            "the picture sits in the middle"
        );
        assert!(
            picture.get_pixel(140, 256)[3] > 240,
            "a quarter in, still the picture"
        );
        assert!(
            picture.get_pixel(110, 256)[3] < 16,
            "an eighth in, the margin"
        );
    }

    #[test]
    fn a_crop_keeps_some_of_the_picture_and_a_bounded_size() {
        let far = StickerCrop {
            x: -10_000,
            y: 10_000,
            side: 80,
        }
        .clamped(100, 100);
        assert_eq!(
            (far.x, far.y),
            (-70, 90),
            "an eighth of the square stays on it"
        );
        let huge = StickerCrop::centered(100, 50).resized(10_000, 100, 50);
        assert_eq!(huge.side, StickerCrop::largest(100, 50));
    }

    #[test]
    fn the_key_fades_just_beyond_its_tolerance() {
        let mut picture = image::RgbaImage::from_fn(3, 1, |x, _| match x {
            0 => image::Rgba([10, 10, 10, 255]),
            1 => image::Rgba([60, 60, 60, 255]),
            _ => image::Rgba([200, 200, 200, 255]),
        });
        key_out(
            &mut picture,
            ColorKey {
                color: [0, 0, 0],
                tolerance: 40,
            },
        );
        assert_eq!(picture.get_pixel(0, 0)[3], 0);
        let edge = picture.get_pixel(1, 0)[3];
        assert!(edge > 0 && edge < 255, "the edge fades: {edge}");
        assert_eq!(picture.get_pixel(2, 0)[3], 255);
    }

    #[test]
    fn without_transparency_the_background_turns_white() {
        let source = png(64, 64, true);
        let sticker =
            make(&source, StickerCrop::centered(64, 64), false, None, &[]).expect("makes");
        let picture = image::load_from_memory(&sticker)
            .expect("decodes")
            .to_rgba8();
        let corner = picture.get_pixel(5, 256);
        assert_eq!(corner[3], 255);
        assert!(corner[0] > 240 && corner[1] > 240 && corner[2] > 240);
        assert!(!inspect(&png(8, 8, false)).expect("reads").transparent);
    }
}
