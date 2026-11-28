//! Crops and turns a picture on its way out, so an attachment can be trimmed
//! before it is sent.

use std::borrow::Cow;
use std::path::{Path, PathBuf};

use crate::model::{PictureCrop, PictureMark, PictureShape, PictureSource, PictureStamp};

/// Quality of an opaque picture written back. The send path passes a JPEG
/// through untouched, so this is the only lossy step for one.
const QUALITY: u8 = 92;

/// A cropped picture, encoded, with the extension its format goes by.
#[derive(Debug)]
pub struct Edited {
    pub bytes: Vec<u8>,
    pub extension: &'static str,
}

/// Largest side of a staged picture's review texture.
pub(crate) const PREVIEW_SIDE: usize = 2048;

/// Resize pasted pixels on a blocking worker, premultiplying transparent colours first.
pub(crate) fn preview(width: usize, height: usize, rgba: &[u8]) -> Option<egui::ColorImage> {
    let (width, height) = (u32::try_from(width).ok()?, u32::try_from(height).ok()?);
    if width == 0 || height == 0 {
        return None;
    }
    // Validate before allocating; opaque pastes can keep borrowing their source.
    image::ImageBuffer::<image::Rgba<u8>, _>::from_raw(width, height, rgba)?;
    let mut pixels = Cow::Borrowed(rgba);
    if rgba.as_chunks::<4>().0.iter().any(|pixel| pixel[3] != 255) {
        for pixel in pixels.to_mut().as_chunks_mut::<4>().0 {
            *pixel = egui::Color32::from_rgba_unmultiplied(pixel[0], pixel[1], pixel[2], pixel[3])
                .to_array();
        }
    }
    let picture = image::ImageBuffer::<image::Rgba<u8>, _>::from_raw(width, height, &*pixels)?;
    let scale = (PREVIEW_SIDE as f32 / width.max(height) as f32).min(1.0);
    let (w, h) = (
        ((width as f32 * scale) as u32).max(1),
        ((height as f32 * scale) as u32).max(1),
    );
    let small = image::imageops::resize(&picture, w, h, image::imageops::FilterType::Triangle);
    Some(egui::ColorImage::from_rgba_premultiplied(
        [w as usize, h as usize],
        small.as_raw(),
    ))
}

/// A picture's size, read without keeping its pixels.
pub fn inspect(bytes: &[u8]) -> Result<(u32, u32), String> {
    let picture = image::load_from_memory(bytes)
        .map_err(|error| format!("This picture could not be read: {error}"))?;
    Ok((picture.width(), picture.height()))
}

/// The picture turned by whole quarter turns clockwise, then cropped. The
/// crop is measured in the turned picture, which is what the editor showed.
pub fn edit(bytes: &[u8], crop: PictureCrop, turns: u8) -> Result<Edited, String> {
    let picture = image::load_from_memory(bytes)
        .map_err(|error| format!("This picture could not be read: {error}"))?
        .to_rgba8();
    turned_and_cropped(picture, crop, turns)
}

/// The same for a pasted picture, which has no file to read back.
pub fn edit_pasted(
    rgba: &[u8],
    width: u32,
    height: u32,
    crop: PictureCrop,
    turns: u8,
) -> Result<Edited, String> {
    let picture = image::RgbaImage::from_raw(width.max(1), height.max(1), rgba.to_vec())
        .ok_or_else(|| "This picture could not be read".to_owned())?;
    turned_and_cropped(picture, crop, turns)
}

/// Turns and crops a staged picture, writes the result in `dir` beside the
/// other media, and answers with the file it wrote. The name comes from the
/// picture's own bytes, so the same crop of the same picture is written once.
pub fn write(
    source: &PictureSource,
    width: u32,
    height: u32,
    crop: PictureCrop,
    turns: u8,
    dir: &Path,
) -> Result<PathBuf, String> {
    let picture = match source {
        PictureSource::File(path) => std::fs::read(path)
            .map_err(|error| error.to_string())
            .and_then(|bytes| edit(&bytes, crop, turns)),
        PictureSource::Pasted(rgba) => edit_pasted(rgba, width, height, crop, turns),
    }?;
    save(picture, dir)
}

pub fn write_marked(
    source: &PictureSource,
    size: (u32, u32),
    crop: PictureCrop,
    turns: u8,
    marks: &[PictureMark],
    hd: bool,
    dir: &Path,
) -> Result<PathBuf, String> {
    let (width, height) = size;
    if marks.is_empty() && hd {
        return write(source, width, height, crop, turns, dir);
    }
    let picture = match source {
        PictureSource::File(path) => std::fs::read(path)
            .map_err(|error| error.to_string())
            .and_then(|bytes| {
                image::load_from_memory(&bytes)
                    .map(|image| image.to_rgba8())
                    .map_err(|error| error.to_string())
            }),
        PictureSource::Pasted(rgba) => image::RgbaImage::from_raw(width, height, rgba.to_vec())
            .ok_or_else(|| "This picture could not be read".to_owned()),
    }?;
    let picture = turned_cropped_marked_quality(picture, crop, turns, marks, hd)?;
    save(picture, dir)
}

fn save(picture: Edited, dir: &Path) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|error| error.to_string())?;
    let name = super::sticker_store::content_hash(&picture.bytes);
    let path = dir.join(format!("{name}.{}", picture.extension));
    std::fs::write(&path, picture.bytes).map_err(|error| error.to_string())?;
    Ok(path)
}

/// Turns the picture, keeps the region, and writes it out.
fn turned_and_cropped(
    picture: image::RgbaImage,
    crop: PictureCrop,
    turns: u8,
) -> Result<Edited, String> {
    turned_cropped_marked(picture, crop, turns, &[])
}

fn turned_cropped_marked(
    picture: image::RgbaImage,
    crop: PictureCrop,
    turns: u8,
    marks: &[PictureMark],
) -> Result<Edited, String> {
    turned_cropped_marked_quality(picture, crop, turns, marks, true)
}

fn turned_cropped_marked_quality(
    picture: image::RgbaImage,
    crop: PictureCrop,
    turns: u8,
    marks: &[PictureMark],
    hd: bool,
) -> Result<Edited, String> {
    let mut picture = match turns % 4 {
        1 => image::imageops::rotate90(&picture),
        2 => image::imageops::rotate180(&picture),
        3 => image::imageops::rotate270(&picture),
        _ => picture,
    };
    // A subpixel pen on a small source would otherwise become one enormous
    // displayed pixel. Upscale only enough to give the thinnest mark a pixel,
    // and keep the longest side at most 1024 pixels for this extra step.
    let narrowest = marks
        .iter()
        .filter_map(|mark| match mark {
            PictureMark::Stroke { radius, .. } => Some(*radius),
            _ => None,
        })
        .fold(f32::INFINITY, f32::min);
    let maximum = (1024 / picture.width().max(picture.height()).max(1)).max(1);
    let factor = if narrowest.is_finite() && narrowest > 0.0 {
        ((1.0 / narrowest).ceil() as u32).clamp(1, maximum)
    } else {
        1
    };
    if factor > 1 {
        picture = image::imageops::resize(
            &picture,
            picture.width() * factor,
            picture.height() * factor,
            image::imageops::FilterType::Triangle,
        );
    }
    for mark in marks {
        match mark {
            PictureMark::Stroke {
                points,
                color,
                radius,
            } => {
                for pair in points.windows(2) {
                    let (a, b) = (pair[0], pair[1]);
                    let steps = ((a.0 - b.0).abs().max((a.1 - b.1).abs()) * factor as f32)
                        .ceil()
                        .max(1.0) as u32;
                    for step in 0..=steps {
                        let x = (a.0 + (b.0 - a.0) * step as f32 / steps as f32) * factor as f32;
                        let y = (a.1 + (b.1 - a.1) * step as f32 / steps as f32) * factor as f32;
                        dab(&mut picture, x, y, *radius * factor as f32, *color);
                    }
                }
                if let Some(&(x, y)) = points.first() {
                    dab(
                        &mut picture,
                        x * factor as f32,
                        y * factor as f32,
                        *radius * factor as f32,
                        *color,
                    );
                }
            }
            PictureMark::Redact(rect) => {
                let rect = rect.clamped(picture.width() / factor, picture.height() / factor);
                // An opaque, blocky cover cannot leak readable detail through a blur.
                for y in rect.y * factor..(rect.y + rect.height) * factor {
                    for x in rect.x * factor..(rect.x + rect.width) * factor {
                        picture.put_pixel(x, y, image::Rgba([24, 24, 24, 255]));
                    }
                }
            }
            PictureMark::Mosaic(rect) => mosaic(&mut picture, *rect, factor),
            PictureMark::Shape {
                kind,
                from,
                to,
                color,
                width,
                filled,
            } => {
                shape(
                    &mut picture,
                    *kind,
                    (from.0 * factor as f32, from.1 * factor as f32),
                    (to.0 * factor as f32, to.1 * factor as f32),
                    *color,
                    *width * factor as f32,
                    *filled,
                );
            }
            PictureMark::Text {
                at,
                text,
                color,
                background,
                size,
            } => {
                annotation_text(
                    &mut picture,
                    (at.0 * factor as f32, at.1 * factor as f32),
                    text,
                    *color,
                    *size * factor as f32,
                    *background,
                );
            }
            PictureMark::Stamp {
                at,
                kind,
                color,
                size,
            } => stamp(
                &mut picture,
                *kind,
                (at.0 * factor as f32, at.1 * factor as f32),
                *size * factor as f32,
                *color,
            ),
            PictureMark::Emoji { at, emoji, size } => {
                emoji_mark(
                    &mut picture,
                    emoji,
                    (at.0 * factor as f32, at.1 * factor as f32),
                    *size * factor as f32,
                );
            }
        }
    }
    let (width, height) = picture.dimensions();
    let original = crop.clamped(width / factor, height / factor);
    let crop = PictureCrop {
        x: original.x * factor,
        y: original.y * factor,
        width: original.width * factor,
        height: original.height * factor,
    };
    let kept =
        image::imageops::crop_imm(&picture, crop.x, crop.y, crop.width, crop.height).to_image();
    let kept = if !hd && kept.width().max(kept.height()) > 1600 {
        let scale = 1600.0 / kept.width().max(kept.height()) as f32;
        image::imageops::resize(
            &kept,
            (kept.width() as f32 * scale).round() as u32,
            (kept.height() as f32 * scale).round() as u32,
            image::imageops::FilterType::Lanczos3,
        )
    } else {
        kept
    };
    encode_quality(&kept, if hd { QUALITY } else { 80 })
}

fn emoji_mark(picture: &mut image::RgbaImage, emoji: &str, at: (f32, f32), size: f32) {
    crate::emoji::available();
    let Some(glyph) = fastframe_emoji::get().render(emoji, size.round().clamp(1.0, 512.0) as u32)
    else {
        return;
    };
    for y in 0..glyph.size[1] {
        for x in 0..glyph.size[0] {
            let left = at.0.round() as i64 + x as i64;
            let top = at.1.round() as i64 + y as i64;
            if left < 0
                || top < 0
                || left >= picture.width() as i64
                || top >= picture.height() as i64
            {
                continue;
            }
            let offset = (y * glyph.size[0] + x) * 4;
            let alpha = glyph.rgba[offset + 3] as u32;
            if alpha == 0 {
                continue;
            }
            let pixel = picture.get_pixel_mut(left as u32, top as u32);
            let destination_alpha = pixel[3] as u32;
            let output_alpha = alpha + destination_alpha * (255 - alpha) / 255;
            for channel in 0..3 {
                let source = glyph.rgba[offset + channel] as u32;
                let destination =
                    pixel[channel] as u32 * destination_alpha * (255 - alpha) / (255 * 255);
                pixel[channel] = ((source + destination) * 255 / output_alpha).min(255) as u8;
            }
            pixel[3] = output_alpha as u8;
        }
    }
}

fn segment(
    picture: &mut image::RgbaImage,
    a: (f32, f32),
    b: (f32, f32),
    width: f32,
    color: [u8; 3],
) {
    let steps = (a.0 - b.0).abs().max((a.1 - b.1).abs()).ceil().max(1.0) as u32;
    for step in 0..=steps {
        let t = step as f32 / steps as f32;
        dab(
            picture,
            a.0 + (b.0 - a.0) * t,
            a.1 + (b.1 - a.1) * t,
            width / 2.0,
            color,
        );
    }
}

fn shape(
    picture: &mut image::RgbaImage,
    kind: PictureShape,
    from: (f32, f32),
    to: (f32, f32),
    color: [u8; 3],
    width: f32,
    filled: bool,
) {
    let (left, right) = (from.0.min(to.0), from.0.max(to.0));
    let (top, bottom) = (from.1.min(to.1), from.1.max(to.1));
    match kind {
        PictureShape::Rectangle => {
            if filled {
                for y in top.max(0.0) as u32
                    ..=(bottom.min(picture.height().saturating_sub(1) as f32) as u32)
                {
                    for x in left.max(0.0) as u32
                        ..=(right.min(picture.width().saturating_sub(1) as f32) as u32)
                    {
                        if x < picture.width() && y < picture.height() {
                            picture.put_pixel(
                                x,
                                y,
                                image::Rgba([color[0], color[1], color[2], 255]),
                            );
                        }
                    }
                }
            } else {
                for (a, b) in [
                    ((left, top), (right, top)),
                    ((right, top), (right, bottom)),
                    ((right, bottom), (left, bottom)),
                    ((left, bottom), (left, top)),
                ] {
                    segment(picture, a, b, width, color);
                }
            }
        }
        PictureShape::Ellipse => {
            let (cx, cy) = ((left + right) / 2.0, (top + bottom) / 2.0);
            let (rx, ry) = (
                ((right - left) / 2.0).max(0.5),
                ((bottom - top) / 2.0).max(0.5),
            );
            if filled {
                for y in top.max(0.0) as u32
                    ..=bottom.min(picture.height().saturating_sub(1) as f32) as u32
                {
                    for x in left.max(0.0) as u32
                        ..=right.min(picture.width().saturating_sub(1) as f32) as u32
                    {
                        if ((x as f32 - cx) / rx).powi(2) + ((y as f32 - cy) / ry).powi(2) <= 1.0 {
                            picture.put_pixel(
                                x,
                                y,
                                image::Rgba([color[0], color[1], color[2], 255]),
                            );
                        }
                    }
                }
            } else {
                let steps = ((rx + ry) * 2.0).ceil().max(32.0) as usize;
                let mut previous = (cx + rx, cy);
                for step in 1..=steps {
                    let angle = std::f32::consts::TAU * step as f32 / steps as f32;
                    let next = (cx + rx * angle.cos(), cy + ry * angle.sin());
                    segment(picture, previous, next, width, color);
                    previous = next;
                }
            }
        }
        PictureShape::Line | PictureShape::Arrow => {
            segment(picture, from, to, width, color);
            if kind == PictureShape::Arrow {
                let angle = (to.1 - from.1).atan2(to.0 - from.0);
                let head = ((to.0 - from.0).hypot(to.1 - from.1) * 0.25).clamp(8.0, 30.0);
                for side in [-0.55_f32, 0.55] {
                    let back = angle + std::f32::consts::PI + side;
                    segment(
                        picture,
                        to,
                        (to.0 + head * back.cos(), to.1 + head * back.sin()),
                        width,
                        color,
                    );
                }
            }
        }
    }
}

fn stamp(
    picture: &mut image::RgbaImage,
    kind: PictureStamp,
    at: (f32, f32),
    size: f32,
    color: [u8; 3],
) {
    let center = (at.0 + size / 2.0, at.1 + size / 2.0);
    let radius = size / 2.0;
    match kind {
        PictureStamp::Star => {
            let points: Vec<_> = (0..10)
                .map(|n| {
                    let angle =
                        -std::f32::consts::FRAC_PI_2 + n as f32 * std::f32::consts::PI / 5.0;
                    let r = if n % 2 == 0 { radius } else { radius * 0.45 };
                    (center.0 + r * angle.cos(), center.1 + r * angle.sin())
                })
                .collect();
            for n in 0..10 {
                segment(picture, points[n], points[(n + 1) % 10], size * 0.09, color);
            }
        }
        PictureStamp::Heart => {
            let mut previous = None;
            for n in 0..=64 {
                let t = std::f32::consts::TAU * n as f32 / 64.0;
                let next = (
                    center.0 + radius * 0.055 * 16.0 * t.sin().powi(3),
                    center.1
                        - radius
                            * 0.055
                            * (13.0 * t.cos()
                                - 5.0 * (2.0 * t).cos()
                                - 2.0 * (3.0 * t).cos()
                                - (4.0 * t).cos()),
                );
                if let Some(before) = previous {
                    segment(picture, before, next, size * 0.08, color);
                }
                previous = Some(next);
            }
        }
        PictureStamp::Smile => {
            let mut previous = None;
            for n in 0..=64 {
                let t = std::f32::consts::TAU * n as f32 / 64.0;
                let next = (
                    center.0 + radius * 0.85 * t.cos(),
                    center.1 + radius * 0.85 * t.sin(),
                );
                if let Some(before) = previous {
                    segment(picture, before, next, size * 0.07, color);
                }
                previous = Some(next);
            }
            dab(
                picture,
                center.0 - radius * 0.3,
                center.1 - radius * 0.2,
                size * 0.055,
                color,
            );
            dab(
                picture,
                center.0 + radius * 0.3,
                center.1 - radius * 0.2,
                size * 0.055,
                color,
            );
            let mut previous = None;
            for n in 0..=24 {
                let t = std::f32::consts::PI * (0.15 + n as f32 * 0.7 / 24.0);
                let next = (
                    center.0 - radius * 0.55 * t.cos(),
                    center.1 + radius * 0.48 * t.sin(),
                );
                if let Some(before) = previous {
                    segment(picture, before, next, size * 0.06, color);
                }
                previous = Some(next);
            }
        }
    }
}

fn mosaic(picture: &mut image::RgbaImage, rect: PictureCrop, factor: u32) {
    let source_width = picture.width() / factor;
    let source_height = picture.height() / factor;
    let rect = rect.clamped(source_width, source_height);
    let (x0, y0, x1, y1) = (
        rect.x * factor,
        rect.y * factor,
        (rect.x + rect.width) * factor,
        (rect.y + rect.height) * factor,
    );
    let source_block = (source_width.max(source_height) / 60).max(12);
    if rect.width < source_block / 2 || rect.height < source_block / 2 {
        for y in y0..y1 {
            for x in x0..x1 {
                picture.put_pixel(x, y, image::Rgba([24, 24, 24, 255]));
            }
        }
        return;
    }
    let block = source_block * factor;
    for y in (y0..y1).step_by(block as usize) {
        for x in (x0..x1).step_by(block as usize) {
            let (right, bottom) = ((x + block).min(x1), (y + block).min(y1));
            let mut sum = [0_u64; 4];
            for py in y..bottom {
                for px in x..right {
                    for (channel, value) in picture.get_pixel(px, py).0.iter().enumerate() {
                        sum[channel] += u64::from(*value);
                    }
                }
            }
            let count = u64::from((right - x) * (bottom - y));
            let pixel = image::Rgba(sum.map(|value| (value / count) as u8));
            for py in y..bottom {
                for px in x..right {
                    picture.put_pixel(px, py, pixel);
                }
            }
        }
    }
}

fn annotation_text(
    picture: &mut image::RgbaImage,
    at: (f32, f32),
    text: &str,
    color: [u8; 3],
    size: f32,
    background: bool,
) {
    use ab_glyph::{Font, FontArc, PxScale, ScaleFont, point};
    static FONT: std::sync::OnceLock<FontArc> = std::sync::OnceLock::new();
    let font = FONT.get_or_init(|| {
        let fonts = egui::FontDefinitions::default();
        let data = fonts
            .font_data
            .get("Ubuntu-Light")
            .expect("egui default font");
        FontArc::try_from_vec(data.font.to_vec()).expect("egui font bytes")
    });
    let scale = PxScale::from(size.max(1.0));
    let scaled = font.as_scaled(scale);
    let advance: f32 = text
        .chars()
        .map(|ch| scaled.h_advance(font.glyph_id(ch)))
        .sum();
    if background {
        let pad = size * 0.22;
        let x0 = (at.0 - pad).max(0.0) as u32;
        let y0 = (at.1 - pad).max(0.0) as u32;
        for y in y0..((at.1 + size + pad) as u32).min(picture.height()) {
            for x in x0..((at.0 + advance + pad) as u32).min(picture.width()) {
                picture.put_pixel(x, y, image::Rgba([22, 28, 32, 255]));
            }
        }
    }
    let mut x = at.0;
    for ch in text.chars() {
        let id = font.glyph_id(ch);
        let glyph = id.with_scale_and_position(scale, point(x, at.1 + scaled.ascent()));
        if let Some(outline) = font.outline_glyph(glyph) {
            let bounds = outline.px_bounds();
            outline.draw(|gx, gy, coverage| {
                let (px, py) = (
                    bounds.min.x as i32 + gx as i32,
                    bounds.min.y as i32 + gy as i32,
                );
                if px >= 0
                    && py >= 0
                    && (px as u32) < picture.width()
                    && (py as u32) < picture.height()
                {
                    let pixel = picture.get_pixel_mut(px as u32, py as u32);
                    let alpha = coverage.clamp(0.0, 1.0);
                    for channel in 0..3 {
                        pixel[channel] = (pixel[channel] as f32 * (1.0 - alpha)
                            + color[channel] as f32 * alpha)
                            .round() as u8;
                    }
                    pixel[3] = 255;
                }
            });
        }
        x += scaled.h_advance(id);
    }
}

fn dab(picture: &mut image::RgbaImage, x: f32, y: f32, radius: f32, color: [u8; 3]) {
    let radius = radius.max(0.75);
    let r = radius.ceil() as i64;
    for dy in -r..=r {
        for dx in -r..=r {
            let (px, py) = (x.round() as i64 + dx, y.round() as i64 + dy);
            if ((px as f32 - x).powi(2) + (py as f32 - y).powi(2)) <= radius * radius
                && px >= 0
                && py >= 0
                && px < i64::from(picture.width())
                && py < i64::from(picture.height())
            {
                picture.put_pixel(
                    px as u32,
                    py as u32,
                    image::Rgba([color[0], color[1], color[2], 255]),
                );
            }
        }
    }
}

/// Writes the picture as a PNG when any of it is see-through, so the crop
/// keeps its transparency, and as a JPEG otherwise.
fn encode_quality(picture: &image::RgbaImage, quality: u8) -> Result<Edited, String> {
    use image::ImageEncoder;
    let (width, height) = picture.dimensions();
    let mut bytes = Vec::new();
    if picture.pixels().any(|pixel| pixel[3] < u8::MAX) {
        image::codecs::png::PngEncoder::new(&mut bytes)
            .write_image(
                picture.as_raw(),
                width,
                height,
                image::ExtendedColorType::Rgba8,
            )
            .map_err(|error| format!("Could not write the picture: {error}"))?;
        return Ok(Edited {
            bytes,
            extension: "png",
        });
    }
    let opaque = image::DynamicImage::ImageRgba8(picture.clone()).to_rgb8();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, quality)
        .write_image(
            opaque.as_raw(),
            width,
            height,
            image::ExtendedColorType::Rgb8,
        )
        .map_err(|error| format!("Could not write the picture: {error}"))?;
    Ok(Edited {
        bytes,
        extension: "jpg",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chosen_emoji_is_composited_into_exported_pixels() {
        let background = image::Rgba([12, 20, 28, 255]);
        let pixels = image::RgbaImage::from_pixel(96, 96, background);
        let edited = turned_cropped_marked(
            pixels,
            PictureCrop::full(96, 96),
            0,
            &[PictureMark::Emoji {
                at: (20.0, 20.0),
                emoji: "😀".to_owned(),
                size: 48.0,
            }],
        )
        .expect("emoji export");
        let output = image::load_from_memory(&edited.bytes).unwrap().to_rgb8();
        assert!(output.pixels().any(|pixel| pixel[0] > 100 && pixel[1] > 60));
    }

    #[test]
    fn shapes_text_stamps_and_mosaic_survive_turn_and_crop() {
        let mut pixels = image::RgbaImage::from_fn(80, 60, |x, y| {
            image::Rgba([x as u8 * 3, y as u8 * 3, 220, 255])
        });
        pixels.put_pixel(0, 0, image::Rgba([0, 0, 0, 254]));
        let marks = [
            PictureMark::Shape {
                kind: PictureShape::Rectangle,
                from: (8.0, 8.0),
                to: (25.0, 20.0),
                color: [255, 0, 0],
                width: 2.0,
                filled: true,
            },
            PictureMark::Shape {
                kind: PictureShape::Ellipse,
                from: (30.0, 8.0),
                to: (48.0, 24.0),
                color: [0, 255, 0],
                width: 2.0,
                filled: false,
            },
            PictureMark::Shape {
                kind: PictureShape::Arrow,
                from: (6.0, 30.0),
                to: (28.0, 42.0),
                color: [255, 0, 0],
                width: 2.0,
                filled: false,
            },
            PictureMark::Shape {
                kind: PictureShape::Line,
                from: (6.0, 46.0),
                to: (28.0, 46.0),
                color: [255, 0, 0],
                width: 2.0,
                filled: false,
            },
            PictureMark::Mosaic(PictureCrop {
                x: 50,
                y: 5,
                width: 20,
                height: 20,
            }),
            PictureMark::Text {
                at: (6.0, 48.0),
                text: "Hi".into(),
                color: [255, 255, 255],
                background: true,
                size: 10.0,
            },
            PictureMark::Stamp {
                at: (50.0, 30.0),
                kind: PictureStamp::Star,
                color: [255, 255, 0],
                size: 20.0,
            },
        ];
        let edited = turned_cropped_marked(
            pixels,
            PictureCrop {
                x: 0,
                y: 0,
                width: 60,
                height: 80,
            },
            1,
            &marks,
        )
        .unwrap();
        let result = decoded(&edited);
        assert_eq!(result.dimensions(), (60, 80));
        assert_eq!(result.get_pixel(15, 14).0[..3], [255, 0, 0]);
        assert_eq!(
            result.get_pixel(50, 12),
            result.get_pixel(51, 12),
            "mosaic block is flat"
        );
        assert_eq!(
            result.get_pixel(4, 48).0[..3],
            [22, 28, 32],
            "text pill is exported"
        );
        assert!(
            result.pixels().any(|pixel| pixel.0[..3] == [255, 255, 0]),
            "stamp is exported"
        );
    }

    #[test]
    fn standard_quality_limits_size_and_hd_preserves_it() {
        let pixels = image::RgbaImage::from_pixel(1800, 8, image::Rgba([80, 120, 160, 255]));
        let crop = PictureCrop::full(1800, 8);
        let standard = turned_cropped_marked_quality(pixels.clone(), crop, 0, &[], false).unwrap();
        let hd = turned_cropped_marked_quality(pixels, crop, 0, &[], true).unwrap();
        assert_eq!(decoded(&standard).width(), 1600);
        assert_eq!(decoded(&hd).width(), 1800);
    }

    #[test]
    fn thin_strokes_on_small_photos_keep_their_editor_width() {
        let mut pixels = image::RgbaImage::from_pixel(32, 32, image::Rgba([255, 255, 255, 255]));
        pixels.put_pixel(0, 0, image::Rgba([255, 255, 255, 254]));
        let marks = [PictureMark::Stroke {
            points: vec![(16.0, 16.0)],
            color: [255, 0, 0],
            radius: 0.125,
        }];
        let edited = turned_cropped_marked(pixels, PictureCrop::full(32, 32), 0, &marks).unwrap();
        let result = decoded(&edited);
        assert_eq!(result.dimensions(), (256, 256));
        assert_eq!(result.get_pixel(128, 128).0[..3], [255, 0, 0]);
        assert_eq!(result.get_pixel(135, 128).0[..3], [255, 255, 255]);
    }

    #[test]
    fn marks_compose_after_turn_and_before_crop() {
        // Keep a transparent pixel so the export is lossless PNG.
        let mut pixels = image::RgbaImage::from_pixel(8, 6, image::Rgba([255, 255, 255, 255]));
        pixels.put_pixel(0, 5, image::Rgba([255, 255, 255, 254]));
        let marks = [
            PictureMark::Stroke {
                points: vec![(4.0, 2.0), (4.0, 3.0)],
                color: [255, 0, 0],
                radius: 1.0,
            },
            PictureMark::Redact(PictureCrop {
                x: 1,
                y: 1,
                width: 2,
                height: 2,
            }),
        ];
        let edited = turned_cropped_marked(
            pixels,
            PictureCrop {
                x: 0,
                y: 0,
                width: 5,
                height: 6,
            },
            1,
            &marks,
        )
        .unwrap();
        let result = decoded(&edited);
        assert_eq!(result.dimensions(), (5, 6));
        assert_eq!(result.get_pixel(4, 2).0[..3], [255, 0, 0]);
        assert_eq!(result.get_pixel(1, 1).0[..3], [24, 24, 24]);
        assert_eq!(result.get_pixel(0, 0).0[..3], [255, 255, 255]);
    }

    /// Both orientations are bounded, alpha is retained, and malformed buffers are refused.
    #[test]
    fn pasted_preview_is_bounded_and_rejects_bad_pixels() {
        for (width, height, expected) in [
            (2048, 1024, [2048, 1024]),
            (1024, 2048, [1024, 2048]),
            (3000, 1, [2048, 1]),
            (8, 4, [8, 4]),
        ] {
            let pixels = [80, 120, 180, 100].repeat(width * height);
            let result = preview(width, height, &pixels).unwrap();
            assert_eq!(result.size, expected);
            assert_eq!(
                result.pixels[0],
                egui::Color32::from_rgba_unmultiplied(80, 120, 180, 100)
            );
            assert_eq!(&pixels[..4], &[80, 120, 180, 100]);
        }
        assert!(preview(0, 1, &[]).is_none());
        assert!(preview(2, 2, &[255; 4]).is_none());
        assert!(preview(usize::MAX, 2, &[255; 4]).is_none());
    }

    /// Hidden RGB values cannot bleed through a transparent edge during downsampling.
    #[test]
    fn pasted_preview_transparent_edges_ignore_hidden_colour() {
        let pixels = |hidden: [u8; 4]| {
            [vec![255, 0, 0, 255], hidden.to_vec()]
                .concat()
                .repeat(2048)
        };
        let blue = preview(4096, 1, &pixels([0, 0, 255, 0])).unwrap();
        let black = preview(4096, 1, &pixels([0, 0, 0, 0])).unwrap();
        assert_eq!(blue.pixels, black.pixels);
        for pixel in blue.pixels {
            let [red, green, blue, alpha] = pixel.to_array();
            assert_eq!(red, alpha);
            assert_eq!((green, blue), (0, 0));
            assert!((100..=155).contains(&alpha));
        }
    }

    /// A picture whose top-left pixel is red, so a turn can be told apart
    /// from the other direction.
    fn png(width: u32, height: u32) -> Vec<u8> {
        use image::ImageEncoder;
        let picture = image::RgbaImage::from_fn(width, height, |x, y| {
            if x == 0 && y == 0 {
                image::Rgba([255, 0, 0, 255])
            } else {
                image::Rgba([0, 0, 255, 255])
            }
        });
        let mut bytes = Vec::new();
        image::codecs::png::PngEncoder::new(&mut bytes)
            .write_image(&picture, width, height, image::ExtendedColorType::Rgba8)
            .expect("encodes");
        bytes
    }

    fn decoded(edited: &Edited) -> image::RgbaImage {
        image::load_from_memory(&edited.bytes)
            .expect("decodes")
            .to_rgba8()
    }

    #[test]
    fn a_picture_reports_its_size() {
        assert_eq!(inspect(&png(800, 600)).expect("reads"), (800, 600));
        assert!(inspect(b"not a picture").is_err());
    }

    #[test]
    fn a_crop_keeps_only_the_region_it_names() {
        let bytes = edit(
            &png(800, 600),
            PictureCrop {
                x: 100,
                y: 50,
                width: 200,
                height: 120,
            },
            0,
        )
        .expect("edits");
        assert_eq!(decoded(&bytes).dimensions(), (200, 120));
    }

    #[test]
    fn a_quarter_turn_clockwise_moves_the_top_left_to_the_top_right() {
        let bytes = edit(&png(4, 2), PictureCrop::full(2, 4), 1).expect("edits");
        let picture = decoded(&bytes);
        assert_eq!(picture.dimensions(), (2, 4));
        // The red corner started top-left of a wide picture, so turning
        // clockwise carries it to the top-right of a tall one. The write is
        // lossy, so the channels are compared rather than their exact values.
        let top_right = picture.get_pixel(1, 0);
        assert!(
            top_right[0] > 200 && top_right[2] < 60,
            "red at the top right, got {top_right:?}"
        );
        let top_left = picture.get_pixel(0, 0);
        assert!(
            top_left[2] > 200 && top_left[0] < 60,
            "blue at the top left, got {top_left:?}"
        );
    }

    #[test]
    fn turning_and_cropping_measure_the_crop_after_the_turn() {
        // The same region the editor showed, in the turned picture.
        let bytes = edit(
            &png(4, 2),
            PictureCrop {
                x: 1,
                y: 0,
                width: 1,
                height: 2,
            },
            1,
        )
        .expect("edits");
        let picture = decoded(&bytes);
        assert_eq!(picture.dimensions(), (1, 2));
        let corner = picture.get_pixel(0, 0);
        assert!(
            corner[0] > 200 && corner[2] < 60,
            "the red corner, got {corner:?}"
        );
    }

    #[test]
    fn see_through_pixels_stay_see_through() {
        use image::ImageEncoder;
        let mut source = Vec::new();
        image::codecs::png::PngEncoder::new(&mut source)
            .write_image(
                image::RgbaImage::from_fn(8, 8, |x, _| {
                    image::Rgba([10, 20, 30, if x < 4 { 0 } else { 128 }])
                })
                .as_raw(),
                8,
                8,
                image::ExtendedColorType::Rgba8,
            )
            .expect("encodes");
        let edited = edit(&source, PictureCrop::full(8, 8), 0).expect("edits");
        assert_eq!(edited.extension, "png");
        let picture = decoded(&edited);
        assert_eq!(picture.get_pixel(2, 2)[3], 0);
        assert_eq!(*picture.get_pixel(6, 2), image::Rgba([10, 20, 30, 128]));
    }

    #[test]
    fn an_opaque_picture_is_written_as_a_jpeg() {
        let edited = edit(&png(8, 8), PictureCrop::full(8, 8), 0).expect("edits");
        assert_eq!(edited.extension, "jpg");
    }

    #[test]
    fn a_pasted_picture_is_turned_and_cropped_like_a_file() {
        // The same red corner as the encoded fixture, as straight RGBA.
        let (width, height) = (4u32, 2u32);
        let mut rgba = [0u8, 0, 255, 255].repeat((width * height) as usize);
        rgba[..4].copy_from_slice(&[255, 0, 0, 255]);
        let bytes = edit_pasted(&rgba, width, height, PictureCrop::full(2, 4), 1).expect("edits");
        let picture = decoded(&bytes);
        assert_eq!(picture.dimensions(), (2, 4));
        let top_right = picture.get_pixel(1, 0);
        assert!(
            top_right[0] > 200 && top_right[2] < 60,
            "red at the top right, got {top_right:?}"
        );
    }

    #[test]
    fn a_pasted_picture_that_is_not_rgba_is_refused() {
        assert!(edit_pasted(&[0, 0, 0], 4, 4, PictureCrop::full(4, 4), 0).is_err());
    }

    #[test]
    fn writing_a_crop_puts_it_under_its_own_bytes() {
        let root = tempfile::tempdir().expect("makes the directory");
        let dir = root.path().to_path_buf();
        let source = dir.join("source.png");
        std::fs::write(&source, png(800, 600)).expect("writes the source");
        let crop = PictureCrop {
            x: 100,
            y: 50,
            width: 200,
            height: 120,
        };
        let written = write(
            &PictureSource::File(source.clone()),
            800,
            600,
            crop,
            0,
            &dir,
        )
        .expect("writes the crop");
        assert_eq!(
            written.extension().and_then(|kind| kind.to_str()),
            Some("jpg")
        );
        assert_eq!(read(&written).dimensions(), (200, 120));
        // The same crop of the same picture lands on the same name, so a
        // second look at it writes nothing new.
        let again =
            write(&PictureSource::File(source), 800, 600, crop, 0, &dir).expect("writes it again");
        assert_eq!(again, written);
        // A pasted picture goes the same way, from the pixels it was given.
        let pasted = std::sync::Arc::new([0u8, 0, 255, 255].repeat(800 * 600));
        let pasted = write(&PictureSource::Pasted(pasted), 800, 600, crop, 0, &dir)
            .expect("writes the pasted crop");
        assert_eq!(read(&pasted).dimensions(), (200, 120));
    }

    fn read(path: &Path) -> image::RgbaImage {
        image::open(path).expect("decodes").to_rgba8()
    }

    #[test]
    fn writing_a_crop_of_a_missing_picture_says_so() {
        let root = tempfile::tempdir().expect("makes the directory");
        let dir = root.path().to_path_buf();
        let error = write(
            &PictureSource::File(dir.join("gone.png")),
            4,
            4,
            PictureCrop::full(4, 4),
            0,
            &dir,
        )
        .expect_err("refuses");
        assert!(!error.is_empty());
    }
}
