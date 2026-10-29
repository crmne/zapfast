//! Crops and turns a picture on its way out, so an attachment can be trimmed
//! before it is sent.

use std::path::{Path, PathBuf};

use crate::model::{PictureCrop, PictureSource};

/// Quality of an opaque picture written back. The send path passes a JPEG
/// through untouched, so this is the only lossy step for one.
const QUALITY: u8 = 92;

/// A cropped picture, encoded, with the extension its format goes by.
#[derive(Debug)]
pub struct Edited {
    pub bytes: Vec<u8>,
    pub extension: &'static str,
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
