//! HEIC and HEIF photos, the format iPhones shoot and Apple Photos hands
//! over on a drag. WhatsApp photos are JPEG and nothing in the tree decodes
//! HEVC stills, so macOS's own decoder turns one into a JPEG before it is
//! reviewed and sent. Elsewhere such a file still goes as a document.

use std::path::{Path, PathBuf};

/// Quality of the JPEG written. The send path passes a JPEG through
/// untouched, so this is the only visibly lossy step.
#[cfg(target_os = "macos")]
const QUALITY: u8 = 92;

/// Whether the file's name says it is a HEIC or HEIF photo.
pub fn is_heic(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("heic") || extension.eq_ignore_ascii_case("heif")
        })
}

/// Whether this system can turn a HEIC photo into a JPEG.
pub const fn convertible() -> bool {
    cfg!(target_os = "macos")
}

/// The photo as a JPEG with its rotation applied to the pixels, so nothing
/// downstream has to read an orientation tag to show it upright.
#[cfg(target_os = "macos")]
pub fn to_jpeg(path: &Path) -> Result<Vec<u8>, String> {
    use image::ImageDecoder;
    // `sips` ships with macOS and decodes through ImageIO. It keeps the
    // orientation as a tag instead of turning the pixels, so its output is
    // an intermediate at full quality that is turned and encoded once here.
    let dir = tempfile::tempdir().map_err(|error| error.to_string())?;
    let intermediate = dir.path().join("photo.jpg");
    // An absolute path cannot be mistaken for one of the tool's options.
    let source = std::path::absolute(path).map_err(|error| error.to_string())?;
    let converted = std::process::Command::new("/usr/bin/sips")
        .args(["-s", "format", "jpeg", "-s", "formatOptions", "100"])
        .arg(&source)
        .arg("--out")
        .arg(&intermediate)
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|error| format!("Could not start the HEIC decoder: {error}"))?;
    if !converted.status.success() {
        return Err("This picture could not be read".to_owned());
    }
    let bytes = std::fs::read(&intermediate).map_err(|error| error.to_string())?;
    let mut decoder =
        image::ImageReader::with_format(std::io::Cursor::new(&bytes), image::ImageFormat::Jpeg)
            .into_decoder()
            .map_err(|error| format!("This picture could not be read: {error}"))?;
    let orientation = decoder
        .orientation()
        .unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut picture = image::DynamicImage::from_decoder(decoder)
        .map_err(|error| format!("This picture could not be read: {error}"))?;
    picture.apply_orientation(orientation);
    let mut jpeg = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, QUALITY)
        .encode_image(&picture.to_rgb8())
        .map_err(|error| format!("Could not write the picture: {error}"))?;
    Ok(jpeg)
}

/// No decoder for HEVC stills is available here.
#[cfg(not(target_os = "macos"))]
pub fn to_jpeg(_path: &Path) -> Result<Vec<u8>, String> {
    Err("HEIC pictures cannot be converted on this system".to_owned())
}

/// Converts a staged HEIC photo and writes the JPEG in `dir` beside the
/// other media, answering with the file to stage in its place. The name
/// comes from the JPEG's bytes, so the same photo is written once.
pub fn write(path: &Path, dir: &Path) -> Result<PathBuf, String> {
    let jpeg = to_jpeg(path)?;
    std::fs::create_dir_all(dir).map_err(|error| error.to_string())?;
    let name = super::sticker_store::content_hash(&jpeg);
    let converted = dir.join(format!("{name}.jpg"));
    std::fs::write(&converted, jpeg).map_err(|error| error.to_string())?;
    Ok(converted)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heic_and_heif_names_are_recognised_in_any_case() {
        for name in ["photo.heic", "photo.HEIC", "photo.heif", "IMG_0001.Heif"] {
            assert!(is_heic(Path::new(name)), "{name}");
        }
        for name in ["photo.jpg", "photo.heic.png", "heic", "photo"] {
            assert!(!is_heic(Path::new(name)), "{name}");
        }
    }

    /// A file that only claims to be HEIC is refused instead of staged.
    #[test]
    fn a_file_that_is_not_a_picture_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let fake = dir.path().join("fake.heic");
        std::fs::write(&fake, b"not a real image").unwrap();
        assert!(write(&fake, &dir.path().join("converted")).is_err());
        assert!(!dir.path().join("converted").exists());
    }

    /// Encodes a real HEIC with the system encoder, tagged as shot on its
    /// side, and checks it comes back as an upright JPEG.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_rotated_heic_becomes_an_upright_jpeg() {
        let dir = tempfile::tempdir().unwrap();
        // 60x40, red on the left half and blue on the right.
        let mut sideways = image::RgbImage::new(60, 40);
        for (x, _, pixel) in sideways.enumerate_pixels_mut() {
            *pixel = image::Rgb(if x < 30 { [255, 0, 0] } else { [0, 0, 255] });
        }
        let mut jpeg = Vec::new();
        let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 95);
        // Orientation 6: the viewer turns the picture a quarter clockwise.
        image::ImageEncoder::set_exif_metadata(
            &mut encoder,
            vec![
                b'I', b'I', 42, 0, 8, 0, 0, 0, 1, 0, 0x12, 0x01, 3, 0, 1, 0, 0, 0, 6, 0, 0, 0, 0,
                0, 0, 0,
            ],
        )
        .unwrap();
        encoder.encode_image(&sideways).unwrap();
        let source = dir.path().join("source.jpg");
        std::fs::write(&source, jpeg).unwrap();
        let heic = dir.path().join("IMG_0001.HEIC");
        let encoded = std::process::Command::new("/usr/bin/sips")
            .args(["-s", "format", "heic"])
            .arg(&source)
            .arg("--out")
            .arg(&heic)
            .output()
            .unwrap();
        assert!(encoded.status.success(), "the system encodes HEIC");

        let converted = write(&heic, &dir.path().join("converted")).unwrap();
        assert_eq!(
            converted.extension().and_then(|value| value.to_str()),
            Some("jpg")
        );
        let bytes = std::fs::read(&converted).unwrap();
        let mut decoder = image::ImageReader::new(std::io::Cursor::new(&bytes))
            .with_guessed_format()
            .unwrap()
            .into_decoder()
            .unwrap();
        assert_eq!(
            image::ImageDecoder::orientation(&mut decoder).unwrap(),
            image::metadata::Orientation::NoTransforms,
            "no tag is left for a viewer to apply twice"
        );
        let upright = image::DynamicImage::from_decoder(decoder)
            .unwrap()
            .to_rgb8();
        assert_eq!(upright.dimensions(), (40, 60));
        // Turned clockwise, the red left half is now the top.
        let (top, bottom) = (upright.get_pixel(20, 8), upright.get_pixel(20, 52));
        assert!(top[0] > 200 && top[2] < 60, "red on top: {top:?}");
        assert!(bottom[2] > 200 && bottom[0] < 60, "blue below: {bottom:?}");
    }
}
