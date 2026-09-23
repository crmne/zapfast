//! Turns a picture into a WhatsApp sticker: a square crop scaled to 512
//! pixels, encoded as a still WebP under WhatsApp's 100 KB limit, with the
//! chosen emojis in its metadata.

use crate::model::StickerCrop;

/// WhatsApp's sticker side, in pixels.
pub const SIDE: u32 = 512;
/// WhatsApp rejects still stickers larger than this.
const LIMIT: usize = 100 * 1024;

/// A picture's size and whether it has any transparent pixel.
pub fn inspect(bytes: &[u8]) -> Result<(u32, u32, bool), String> {
    let picture = image::load_from_memory(bytes)
        .map_err(|error| format!("This picture could not be read: {error}"))?;
    let transparent =
        picture.color().has_alpha() && picture.to_rgba8().pixels().any(|pixel| pixel[3] < 255);
    Ok((picture.width(), picture.height(), transparent))
}

/// Makes the sticker. Without `transparent`, see-through areas become white.
pub fn make(
    bytes: &[u8],
    crop: StickerCrop,
    transparent: bool,
    emojis: &[String],
) -> Result<Vec<u8>, String> {
    let picture = image::load_from_memory(bytes)
        .map_err(|error| format!("This picture could not be read: {error}"))?
        .to_rgba8();
    let crop = crop.clamped(picture.width(), picture.height());
    let square =
        image::imageops::crop_imm(&picture, crop.x, crop.y, crop.side, crop.side).to_image();
    let mut sticker =
        image::imageops::resize(&square, SIDE, SIDE, image::imageops::FilterType::Lanczos3);
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
    let webp = encode(&sticker)?;
    let info = crate::sticker_meta::StickerInfo {
        pack_id: "zapfast".to_owned(),
        pack_name: "ZapFast".to_owned(),
        emojis: emojis.to_vec(),
        ..Default::default()
    };
    Ok(crate::sticker_meta::write(&webp, &info).unwrap_or(webp))
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
        assert_eq!(inspect(&source).expect("reads"), (800, 600, true));
        let crop = StickerCrop::centered(800, 600);
        assert_eq!((crop.x, crop.y, crop.side), (100, 0, 600));
        let sticker = make(&source, crop, true, &["🎨".to_owned()]).expect("makes");
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

    #[test]
    fn without_transparency_the_background_turns_white() {
        let source = png(64, 64, true);
        let sticker = make(&source, StickerCrop::centered(64, 64), false, &[]).expect("makes");
        let picture = image::load_from_memory(&sticker)
            .expect("decodes")
            .to_rgba8();
        let corner = picture.get_pixel(5, 256);
        assert_eq!(corner[3], 255);
        assert!(corner[0] > 240 && corner[1] > 240 && corner[2] > 240);
        assert!(!inspect(&png(8, 8, false)).expect("reads").2);
    }
}
