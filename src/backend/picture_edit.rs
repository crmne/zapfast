//! Draws on, crops and turns a picture on its way out, so an attachment can
//! be marked up and trimmed before it is sent.

use std::borrow::Cow;
use std::path::{Path, PathBuf};

use crate::model::{Mark, PictureBlocks, PictureCrop, PictureOrigin, PictureSource};

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
pub(crate) const PREVIEW_SIDE: usize = 1024;

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

/// The colours a pixelated region of this picture is painted in, read
/// from the same pixels writing the edit reads.
pub fn sample(source: &PictureSource, width: u32, height: u32) -> Result<PictureBlocks, String> {
    let picture = match source {
        PictureSource::File(path) => {
            let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
            image::load_from_memory(&bytes)
                .map_err(|error| format!("This picture could not be read: {error}"))?
                .to_rgba8()
        }
        PictureSource::Pasted(rgba) => {
            image::RgbaImage::from_raw(width.max(1), height.max(1), rgba.to_vec())
                .ok_or_else(|| "This picture could not be read".to_owned())?
        }
    };
    Ok(blocks(&picture))
}

/// Every pixelation block of the picture, in its average colour.
fn blocks(picture: &image::RgbaImage) -> PictureBlocks {
    let (width, height) = picture.dimensions();
    PictureBlocks::new(width, height, |cell| average(picture, cell))
}

/// The picture with its marks drawn, then turned by whole quarter turns
/// clockwise, then cropped. The marks are in the source's own pixels; the
/// crop is measured in the turned picture, which is what the editor showed.
pub fn edit(bytes: &[u8], crop: PictureCrop, turns: u8, marks: &[Mark]) -> Result<Edited, String> {
    let picture = image::load_from_memory(bytes)
        .map_err(|error| format!("This picture could not be read: {error}"))?
        .to_rgba8();
    turned_and_cropped(marked(picture, marks)?, crop, turns)
}

/// The same for a pasted picture, which has no file to read back.
pub fn edit_pasted(
    rgba: &[u8],
    width: u32,
    height: u32,
    crop: PictureCrop,
    turns: u8,
    marks: &[Mark],
) -> Result<Edited, String> {
    let picture = image::RgbaImage::from_raw(width.max(1), height.max(1), rgba.to_vec())
        .ok_or_else(|| "This picture could not be read".to_owned())?;
    turned_and_cropped(marked(picture, marks)?, crop, turns)
}

/// Draws on, turns and crops a staged picture, writes the result in `dir`
/// beside the other media, and answers with the file it wrote. The name comes
/// from the picture's own bytes, so the same edit of the same picture is
/// written once.
pub fn write(origin: &PictureOrigin, dir: &Path) -> Result<PathBuf, String> {
    let PictureOrigin {
        source,
        width,
        height,
        crop,
        turns,
        marks,
    } = origin;
    let picture = match source {
        PictureSource::File(path) => std::fs::read(path)
            .map_err(|error| error.to_string())
            .and_then(|bytes| edit(&bytes, *crop, *turns, marks)),
        PictureSource::Pasted(rgba) => edit_pasted(rgba, *width, *height, *crop, *turns, marks),
    }?;
    std::fs::create_dir_all(dir).map_err(|error| error.to_string())?;
    let name = super::sticker_store::content_hash(&picture.bytes);
    let path = dir.join(format!("{name}.{}", picture.extension));
    std::fs::write(&path, picture.bytes).map_err(|error| error.to_string())?;
    Ok(path)
}

/// The picture with its marks drawn on in order. Lines are drawn with
/// anti-aliasing onto a see-through layer and laid over the picture; a
/// pixelated region paints each block in the average of the untouched
/// picture under it, from the same `PictureBlocks` the editor's preview
/// paints with.
fn marked(mut picture: image::RgbaImage, marks: &[Mark]) -> Result<image::RgbaImage, String> {
    if marks.is_empty() {
        return Ok(picture);
    }
    let (width, height) = picture.dimensions();
    let mut layer = blank_layer(width, height)?;
    let untouched = marks
        .iter()
        .any(|mark| matches!(mark, Mark::Pixelate { .. }))
        .then(|| blocks(&picture));
    let mut drawn = false;
    for mark in marks {
        match (mark, &untouched) {
            (Mark::Pixelate { from, to }, Some(untouched)) => {
                if std::mem::take(&mut drawn) {
                    lay_over(&mut picture, &layer);
                    layer.fill(tiny_skia::Color::TRANSPARENT);
                }
                for cell in crate::model::pixel_cells(*from, *to, width, height) {
                    let Some(colour) = untouched.colour(cell) else {
                        continue;
                    };
                    let colour = image::Rgba(colour);
                    for y in cell[1]..cell[3] {
                        for x in cell[0]..cell[2] {
                            picture.put_pixel(x, y, colour);
                        }
                    }
                }
            }
            _ => drawn |= stroke(&mut layer, mark, tiny_skia::Transform::identity()),
        }
    }
    if drawn {
        lay_over(&mut picture, &layer);
    }
    Ok(picture)
}

/// A see-through layer this big to draw marks on. Sending the picture
/// without the marks the person drew, a pixelated face among them, is worse
/// than not sending it, so a layer that cannot be made is an error.
fn blank_layer(width: u32, height: u32) -> Result<tiny_skia::Pixmap, String> {
    tiny_skia::Pixmap::new(width, height)
        .ok_or_else(|| format!("Could not draw on a picture {width} by {height} pixels"))
}

/// The marks as the editor shows them over the picture, drawn by the same
/// strokes writing draws: on a see-through layer `size` pixels big that
/// covers the source turned `turns` quarters clockwise. A pixelated block
/// is opaque, in `cell`'s colour, and hides whatever was drawn under it as
/// writing it does.
pub(crate) fn preview_layer<'a>(
    marks: impl IntoIterator<Item = &'a Mark>,
    (width, height): (u32, u32),
    turns: u8,
    [wide, tall]: [u32; 2],
    cell: impl Fn([u32; 4]) -> [u8; 3],
) -> Option<tiny_skia::Pixmap> {
    let mut layer = tiny_skia::Pixmap::new(wide, tall)?;
    let (shown_wide, shown_tall) = if turns.is_multiple_of(2) {
        (width, height)
    } else {
        (height, width)
    };
    let transform = turned(turns, width, height).post_scale(
        wide as f32 / shown_wide.max(1) as f32,
        tall as f32 / shown_tall.max(1) as f32,
    );
    // Without anti-aliasing, blocks that meet share their edge exactly,
    // and no seam lets the picture through.
    let mut paint = tiny_skia::Paint {
        anti_alias: false,
        ..Default::default()
    };
    for mark in marks {
        let Mark::Pixelate { from, to } = mark else {
            stroke(&mut layer, mark, transform);
            continue;
        };
        for block in crate::model::pixel_cells(*from, *to, width, height) {
            let [r, g, b] = cell(block);
            paint.set_color_rgba8(r, g, b, u8::MAX);
            let [left, top, right, bottom] = block.map(|edge| edge as f32);
            if let Some(rect) = tiny_skia::Rect::from_ltrb(left, top, right, bottom) {
                layer.fill_rect(rect, &paint, transform, None);
            }
        }
    }
    Some(layer)
}

/// Carries the source's pixels to the picture turned `turns` quarters
/// clockwise, as `crate::model::turned_point` does.
fn turned(turns: u8, width: u32, height: u32) -> tiny_skia::Transform {
    let (w, h) = (width as f32, height as f32);
    match turns % 4 {
        1 => tiny_skia::Transform::from_row(0.0, 1.0, -1.0, 0.0, h, 0.0),
        2 => tiny_skia::Transform::from_row(-1.0, 0.0, 0.0, -1.0, w, h),
        3 => tiny_skia::Transform::from_row(0.0, -1.0, 1.0, 0.0, 0.0, w),
        _ => tiny_skia::Transform::identity(),
    }
}

/// Draws one inked mark on `layer` through `transform`, answering whether
/// it drew anything.
fn stroke(layer: &mut tiny_skia::Pixmap, mark: &Mark, transform: tiny_skia::Transform) -> bool {
    use tiny_skia::{LineCap, LineJoin, PathBuilder};
    let Some([r, g, b, a]) = mark.rgba() else {
        return false;
    };
    let mut paint = tiny_skia::Paint::default();
    paint.set_color_rgba8(r, g, b, a);
    paint.anti_alias = true;
    let line = |points: &[[f32; 2]]| {
        let mut path = PathBuilder::new();
        for (at, [x, y]) in points.iter().enumerate() {
            if at == 0 {
                path.move_to(*x, *y);
            } else {
                path.line_to(*x, *y);
            }
        }
        path
    };
    let (path, width, line_cap, line_join) = match mark {
        Mark::Pen { points, width, .. } | Mark::Highlighter { points, width, .. }
            if points.len() < 2 =>
        {
            // A click leaves a dot as wide as the line.
            let Some(&[x, y]) = points.first() else {
                return false;
            };
            let Some(dot) = PathBuilder::from_circle(x, y, width / 2.0) else {
                return false;
            };
            layer.fill_path(&dot, &paint, tiny_skia::FillRule::Winding, transform, None);
            return true;
        }
        Mark::Pen { points, width, .. } => (line(points), *width, LineCap::Round, LineJoin::Round),
        Mark::Highlighter { points, width, .. } => {
            (line(points), *width, LineCap::Butt, LineJoin::Round)
        }
        Mark::Arrow {
            from, to, width, ..
        } => {
            let mut path = line(&[*from, *to]);
            for barb in crate::model::arrow_barbs(*from, *to, *width) {
                path.move_to(barb[0], barb[1]);
                path.line_to(to[0], to[1]);
            }
            (path, *width, LineCap::Round, LineJoin::Round)
        }
        Mark::Rectangle {
            from, to, width, ..
        } => {
            let mut path = line(&[*from, [to[0], from[1]], *to, [from[0], to[1]]]);
            path.close();
            (path, *width, LineCap::Butt, LineJoin::Miter)
        }
        Mark::Pixelate { .. } => return false,
    };
    let Some(path) = path.finish() else {
        return false;
    };
    let stroke = tiny_skia::Stroke {
        width,
        line_cap,
        line_join,
        ..Default::default()
    };
    layer.stroke_path(&path, &paint, &stroke, transform, None);
    true
}

/// Lays the premultiplied layer over the picture's straight colours.
fn lay_over(picture: &mut image::RgbaImage, layer: &tiny_skia::Pixmap) {
    for (pixel, over) in picture.pixels_mut().zip(layer.pixels()) {
        let over_alpha = f32::from(over.alpha()) / 255.0;
        if over_alpha == 0.0 {
            continue;
        }
        let under_alpha = f32::from(pixel[3]) / 255.0;
        let alpha = over_alpha + under_alpha * (1.0 - over_alpha);
        let blend = |over: u8, under: u8| {
            let premultiplied = f32::from(over) / 255.0
                + f32::from(under) / 255.0 * under_alpha * (1.0 - over_alpha);
            (premultiplied / alpha * 255.0).round().clamp(0.0, 255.0) as u8
        };
        *pixel = image::Rgba([
            blend(over.red(), pixel[0]),
            blend(over.green(), pixel[1]),
            blend(over.blue(), pixel[2]),
            (alpha * 255.0).round() as u8,
        ]);
    }
}

/// The average colour of a block, weighed by how opaque each pixel is, so the
/// hidden colour of see-through pixels does not tint it.
fn average(picture: &image::RgbaImage, [left, top, right, bottom]: [u32; 4]) -> [u8; 4] {
    let (mut colour, mut alpha, mut count) = ([0u64; 3], 0u64, 0u64);
    for y in top..bottom {
        for x in left..right {
            let pixel = picture.get_pixel(x, y);
            let weight = u64::from(pixel[3]);
            for (sum, channel) in colour.iter_mut().zip(&pixel.0[..3]) {
                *sum += u64::from(*channel) * weight;
            }
            alpha += weight;
            count += 1;
        }
    }
    if alpha == 0 {
        return [0; 4];
    }
    let [r, g, b] = colour.map(|sum| ((sum + alpha / 2) / alpha) as u8);
    [r, g, b, ((alpha + count / 2) / count) as u8]
}

/// Turns the picture, keeps the region, and writes it out.
fn turned_and_cropped(
    picture: image::RgbaImage,
    crop: PictureCrop,
    turns: u8,
) -> Result<Edited, String> {
    let picture = match turns % 4 {
        1 => image::imageops::rotate90(&picture),
        2 => image::imageops::rotate180(&picture),
        3 => image::imageops::rotate270(&picture),
        _ => picture,
    };
    let (width, height) = picture.dimensions();
    let crop = crop.clamped(width, height);
    let kept =
        image::imageops::crop_imm(&picture, crop.x, crop.y, crop.width, crop.height).to_image();
    encode(&kept)
}

/// Writes the picture as a PNG when any of it is see-through, so the crop
/// keeps its transparency, and as a JPEG otherwise.
fn encode(picture: &image::RgbaImage) -> Result<Edited, String> {
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
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, QUALITY)
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

    /// Both orientations are bounded, alpha is retained, and malformed buffers are refused.
    #[test]
    fn pasted_preview_is_bounded_and_rejects_bad_pixels() {
        for (width, height, expected) in [
            (2048, 1024, [1024, 512]),
            (1024, 2048, [512, 1024]),
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
                .repeat(1024)
        };
        let blue = preview(2048, 1, &pixels([0, 0, 255, 0])).unwrap();
        let black = preview(2048, 1, &pixels([0, 0, 0, 0])).unwrap();
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
            &[],
        )
        .expect("edits");
        assert_eq!(decoded(&bytes).dimensions(), (200, 120));
    }

    #[test]
    fn a_quarter_turn_clockwise_moves_the_top_left_to_the_top_right() {
        let bytes = edit(&png(4, 2), PictureCrop::full(2, 4), 1, &[]).expect("edits");
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
            &[],
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
        let edited = edit(&source, PictureCrop::full(8, 8), 0, &[]).expect("edits");
        assert_eq!(edited.extension, "png");
        let picture = decoded(&edited);
        assert_eq!(picture.get_pixel(2, 2)[3], 0);
        assert_eq!(*picture.get_pixel(6, 2), image::Rgba([10, 20, 30, 128]));
    }

    #[test]
    fn an_opaque_picture_is_written_as_a_jpeg() {
        let edited = edit(&png(8, 8), PictureCrop::full(8, 8), 0, &[]).expect("edits");
        assert_eq!(edited.extension, "jpg");
    }

    #[test]
    fn a_pasted_picture_is_turned_and_cropped_like_a_file() {
        // The same red corner as the encoded fixture, as straight RGBA.
        let (width, height) = (4u32, 2u32);
        let mut rgba = [0u8, 0, 255, 255].repeat((width * height) as usize);
        rgba[..4].copy_from_slice(&[255, 0, 0, 255]);
        let bytes =
            edit_pasted(&rgba, width, height, PictureCrop::full(2, 4), 1, &[]).expect("edits");
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
        assert!(edit_pasted(&[0, 0, 0], 4, 4, PictureCrop::full(4, 4), 0, &[]).is_err());
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
            &origin(PictureSource::File(source.clone()), 800, 600, crop),
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
        let again = write(&origin(PictureSource::File(source), 800, 600, crop), &dir)
            .expect("writes it again");
        assert_eq!(again, written);
        // A pasted picture goes the same way, from the pixels it was given.
        let pasted = std::sync::Arc::new([0u8, 0, 255, 255].repeat(800 * 600));
        let pasted = write(&origin(PictureSource::Pasted(pasted), 800, 600, crop), &dir)
            .expect("writes the pasted crop");
        assert_eq!(read(&pasted).dimensions(), (200, 120));
    }

    fn origin(source: PictureSource, width: u32, height: u32, crop: PictureCrop) -> PictureOrigin {
        PictureOrigin {
            source,
            width,
            height,
            crop,
            turns: 0,
            marks: Vec::new(),
        }
    }

    fn read(path: &Path) -> image::RgbaImage {
        image::open(path).expect("decodes").to_rgba8()
    }

    #[test]
    fn writing_a_crop_of_a_missing_picture_says_so() {
        let root = tempfile::tempdir().expect("makes the directory");
        let dir = root.path().to_path_buf();
        let error = write(
            &origin(
                PictureSource::File(dir.join("gone.png")),
                4,
                4,
                PictureCrop::full(4, 4),
            ),
            &dir,
        )
        .expect_err("refuses");
        assert!(!error.is_empty());
    }

    fn white(width: u32, height: u32) -> image::RgbaImage {
        image::RgbaImage::from_pixel(width, height, image::Rgba([255, 255, 255, 255]))
    }

    fn red_line(from: [f32; 2], to: [f32; 2]) -> Mark {
        Mark::Pen {
            points: vec![from, to],
            ink: crate::model::Ink::Red,
            width: 4.0,
        }
    }

    #[test]
    fn a_red_line_turns_the_pixels_under_it_red_and_leaves_the_rest() {
        let picture = marked(white(100, 100), &[red_line([10.0, 50.0], [90.0, 50.0])]).unwrap();
        assert_eq!(*picture.get_pixel(50, 50), image::Rgba([235, 64, 52, 255]));
        assert_eq!(*picture.get_pixel(0, 0), image::Rgba([255, 255, 255, 255]));
        assert_eq!(
            *picture.get_pixel(50, 55),
            image::Rgba([255, 255, 255, 255])
        );
    }

    #[test]
    fn a_highlighter_lets_the_picture_show_through() {
        let mark = Mark::Highlighter {
            points: vec![[10.0, 50.0], [90.0, 50.0]],
            ink: crate::model::Ink::Blue,
            width: 12.0,
        };
        let [r, g, b, a] = marked(white(100, 100), &[mark])
            .unwrap()
            .get_pixel(50, 50)
            .0;
        assert_eq!(a, 255, "an opaque picture stays opaque");
        // 40% blue over white.
        assert!(
            r > 150 && r < 200 && g > 180 && g < 220 && b > 245,
            "{r} {g} {b}"
        );
    }

    #[test]
    fn pixelating_paints_each_block_in_its_average() {
        // Black and white pixels by turns average to a mid grey.
        let checks = image::RgbaImage::from_fn(100, 100, |x, y| {
            let shade = if (x + y) % 2 == 0 { 0 } else { 255 };
            image::Rgba([shade, shade, shade, 255])
        });
        let picture = marked(
            checks,
            &[Mark::Pixelate {
                from: [2.0, 2.0],
                to: [10.0, 4.0],
            }],
        )
        .unwrap();
        // The region grows to whole 8-pixel blocks: two across, one down.
        for (x, y) in [(0, 0), (7, 7), (8, 0), (15, 7)] {
            assert_eq!(*picture.get_pixel(x, y), image::Rgba([128, 128, 128, 255]));
        }
        assert_eq!(
            picture.get_pixel(17, 0)[0],
            255,
            "the next block is left alone"
        );
        assert_eq!(picture.get_pixel(0, 8)[0], 0);
    }

    /// Black lines of print on white, two rows in every five, as in a
    /// screenshot of text.
    fn print(width: u32, height: u32) -> image::RgbaImage {
        image::RgbaImage::from_fn(width, height, |x, y| {
            let ink = y % 5 < 2 && x % 7 != 0;
            image::Rgba(if ink { [0, 0, 0, 255] } else { [255; 4] })
        })
    }

    /// What the editor paints a pixelated block in is what the written
    /// file has there: the block's grey, never one black or white pixel of
    /// the print under it.
    #[test]
    fn the_editors_pixelated_blocks_are_the_written_ones() {
        let (width, height) = (600, 300);
        let rgba = std::sync::Arc::new(print(width, height).into_raw());
        let source = PictureSource::Pasted(rgba);
        let blocks = sample(&source, width, height).expect("samples");
        let region = Mark::Pixelate {
            from: [100.0, 40.0],
            to: [260.0, 150.0],
        };
        let root = tempfile::tempdir().expect("makes the directory");
        let written = write(
            &PictureOrigin {
                marks: vec![region.clone()],
                ..origin(source, width, height, PictureCrop::full(width, height))
            },
            root.path(),
        )
        .expect("writes");
        let file = read(&written);
        let drawn = marked(print(width, height), std::slice::from_ref(&region)).expect("draws");
        let Mark::Pixelate { from, to } = region else {
            unreachable!()
        };
        let cells = crate::model::pixel_cells(from, to, width, height);
        assert!(cells.len() > 100);
        for cell in cells {
            let shown = blocks.colour(cell).expect("every cell has a block");
            let [left, top, right, bottom] = cell;
            let (x, y) = ((left + right) / 2, (top + bottom) / 2);
            assert_eq!(shown, drawn.get_pixel(x, y).0, "{cell:?}: what is encoded");
            let kept = file.get_pixel(x, y);
            for channel in 0..3 {
                let (shown, kept) = (shown[channel], kept[channel]);
                assert!(
                    (60..200).contains(&shown),
                    "{cell:?}: a grey, not the print, got {shown}"
                );
                // The JPEG the file is written as rings a little beside the
                // print around the region.
                assert!(
                    shown.abs_diff(kept) <= 12,
                    "{cell:?}: the editor shows {shown}, the file has {kept}"
                );
            }
        }
    }

    #[test]
    fn the_editors_turn_carries_points_where_the_model_does() {
        let (width, height) = (300, 200);
        for turns in 0..4 {
            for point in [[0.0, 0.0], [40.0, 170.0], [300.0, 12.5]] {
                let mut moved = [tiny_skia::Point::from_xy(point[0], point[1])];
                turned(turns, width, height).map_points(&mut moved);
                let expected = crate::model::turned_point(point, turns, width, height);
                assert_eq!([moved[0].x, moved[0].y], expected, "{turns} {point:?}");
            }
        }
    }

    /// The editor's preview of a highlighter that crosses itself, a pen
    /// line and a pixelated block, turned a quarter, is the written file:
    /// in particular the crossing is no darker than one pass, since the
    /// stroke is filled once.
    #[test]
    fn the_editors_preview_of_marks_is_the_written_picture() {
        use crate::model::Ink;
        let (width, height) = (200, 160);
        let zigzag = Mark::Highlighter {
            points: vec![[20.0, 20.0], [120.0, 120.0], [120.0, 20.0], [20.0, 120.0]],
            ink: Ink::Yellow,
            width: 14.0,
        };
        let marks = [
            zigzag,
            red_line([150.0, 10.0], [150.0, 150.0]),
            Mark::Pixelate {
                from: [160.0, 130.0],
                to: [190.0, 150.0],
            },
        ];
        let picture = print(width, height);
        let blocks = blocks(&picture);
        let cell = |block| {
            let [r, g, b, _] = blocks.colour(block).expect("a block");
            [r, g, b]
        };
        let layer =
            preview_layer(&marks, (width, height), 1, [height, width], cell).expect("a layer");
        let mut shown = image::imageops::rotate90(&picture);
        lay_over(&mut shown, &layer);
        let drawn = image::imageops::rotate90(&marked(picture.clone(), &marks).expect("draws"));
        // tiny-skia's anti-aliasing is not quite the same turned, so a few
        // pixels along a diagonal edge differ by part of their coverage.
        let differences: Vec<u8> = shown
            .pixels()
            .zip(drawn.pixels())
            .map(|(a, b)| (0..4).map(|c| a[c].abs_diff(b[c])).max().unwrap_or(0))
            .collect();
        let strays = differences.iter().filter(|&&by| by > 2).count();
        assert!(
            strays * 1000 <= differences.len(),
            "{strays} pixels of the preview stray from the file"
        );
        assert!(
            differences.iter().all(|&by| by < 32),
            "no pixel strays by more than an edge's coverage"
        );
        let at = |source: [f32; 2]| {
            let [x, y] = crate::model::turned_point(source, 1, width, height);
            shown.get_pixel(x as u32, y as u32).0
        };
        // (70, 70) is where the zigzag crosses itself; (45, 45) is on one
        // pass; both on white between the lines of print.
        let (crossing, once) = (at([70.5, 72.5]), at([45.5, 47.5]));
        assert!(
            (0..3).all(|c| crossing[c].abs_diff(once[c]) <= 1),
            "crossing {crossing:?}, one pass {once:?}"
        );
        let root = tempfile::tempdir().expect("makes the directory");
        let rgba = std::sync::Arc::new(picture.into_raw());
        let written = write(
            &PictureOrigin {
                turns: 1,
                marks: marks.to_vec(),
                ..origin(
                    PictureSource::Pasted(rgba),
                    width,
                    height,
                    PictureCrop::full(height, width),
                )
            },
            root.path(),
        )
        .expect("writes");
        let file = read(&written);
        let [x, y] = crate::model::turned_point([70.5, 72.5], 1, width, height);
        let kept = file.get_pixel(x as u32, y as u32);
        assert!(
            (0..3).all(|c| crossing[c].abs_diff(kept[c]) <= 12),
            "the editor shows {crossing:?} at the crossing, the file has {kept:?}"
        );
    }

    /// A picture the marks cannot be drawn on is refused rather than
    /// written without them.
    #[test]
    fn marks_that_cannot_be_drawn_refuse_the_picture() {
        let error = marked(white(0, 4), &[red_line([0.0, 1.0], [0.0, 3.0])])
            .expect_err("no layer for an empty picture");
        assert!(error.contains("Could not draw"), "{error}");
        assert!(
            blank_layer(u32::MAX, 1).is_err(),
            "too wide for a layer, refused before anything is allocated"
        );
    }

    #[test]
    fn marks_are_drawn_before_the_turn_and_the_crop() {
        // A dot at the source's top left ends up at the top right once the
        // picture is turned clockwise.
        let dot = Mark::Pen {
            points: vec![[4.0, 4.0]],
            ink: crate::model::Ink::Red,
            width: 6.0,
        };
        let rgba = white(40, 20).into_raw();
        let edited =
            edit_pasted(&rgba, 40, 20, PictureCrop::full(20, 40), 1, &[dot]).expect("edits");
        let picture = decoded(&edited);
        assert_eq!(picture.dimensions(), (20, 40));
        let corner = picture.get_pixel(15, 4);
        assert!(corner[0] > 200 && corner[1] < 120, "red, got {corner:?}");
        let other = picture.get_pixel(4, 4);
        assert!(other[1] > 200, "white, got {other:?}");
    }

    #[test]
    fn without_marks_the_picture_is_written_exactly_as_before() {
        let bytes = png(40, 30);
        let crop = PictureCrop {
            x: 3,
            y: 4,
            width: 20,
            height: 10,
        };
        let edited = edit(&bytes, crop, 1, &[]).expect("edits");
        let source = image::load_from_memory(&bytes).unwrap().to_rgba8();
        let turned = image::imageops::rotate90(&source);
        let kept = image::imageops::crop_imm(&turned, 3, 4, 20, 10).to_image();
        assert_eq!(edited.bytes, encode(&kept).unwrap().bytes);
        assert_eq!(marked(source.clone(), &[]).unwrap(), source);
    }
}
