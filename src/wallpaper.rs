//! The chat wallpaper: a colour, the default doodles over it, or an image of
//! the reader's own in place of both.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, mpsc};

use egui::{Color32, ColorImage, Rect, TextureHandle, TextureOptions, Vec2, pos2};

/// The default doodle tile, drawn from Lucide icons (ISC, see
/// `assets/icons/LICENSE.txt`). It is embedded so the wallpaper works offline
/// and does not depend on a third-party request at runtime.
const DEFAULT_SVG: &[u8] = include_bytes!("../assets/wallpaper.svg");

/// The longest side a wallpaper image keeps, which bounds its texture to about
/// 26 MB even for a square picture while staying sharp on a 1440p display.
pub const MAX_SIDE: u32 = 2560;

/// The extensions a copied wallpaper image may have.
const EXTENSIONS: [&str; 4] = ["jpg", "png", "webp", "gif"];

#[derive(Clone, Default)]
struct Cache(Arc<Mutex<Option<TextureHandle>>>);

/// The uploaded custom image and which image it was made from.
#[derive(Clone)]
struct ImageTexture {
    generation: u64,
    texture: TextureHandle,
}

/// What the conversation shows behind its bubbles.
#[derive(Clone)]
pub struct Look {
    /// The background colour, with the Theme choice already resolved.
    pub color: Color32,
    /// Whether the doodles are drawn over the colour.
    pub doodles: bool,
    /// A decoded image of the reader's, which wins over colour and doodles.
    pub image: Option<(u64, Arc<ColorImage>)>,
}

/// Paint the wallpaper over the conversation panel.
pub fn paint(ui: &egui::Ui, look: &Look) {
    paint_rect(ui, ui.max_rect(), look);
}

/// Paint the wallpaper into a bounded rectangle: the image cover-fitted if
/// there is one, otherwise the colour with the SVG's intrinsic 374 x 666
/// logical-pixel doodle tile repeated in both axes.
pub fn paint_rect(ui: &egui::Ui, rect: Rect, look: &Look) {
    let painter = ui.painter().with_clip_rect(rect);
    painter.rect_filled(rect, 0.0, look.color);

    match &look.image {
        Some(image) => {
            let texture = image_texture(ui.ctx(), image);
            let uv = cover_uv(texture.size_vec2(), rect.size());
            painter.image(texture.id(), rect, uv, Color32::WHITE);
            return;
        }
        None => forget_image(ui.ctx()),
    }

    if !look.doodles {
        return;
    }

    let Some(texture) = texture(ui.ctx()) else {
        return;
    };

    let tile = texture.size_vec2();
    if tile.x <= 0.0 || tile.y <= 0.0 {
        return;
    }

    let line = doodle_ink(look.color);
    let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    let origin = pos2(0.0, 0.0);
    let first_x = origin.x + (rect.left() - origin.x).div_euclid(tile.x) * tile.x;
    let first_y = origin.y + (rect.top() - origin.y).div_euclid(tile.y) * tile.y;

    let mut y = first_y;
    while y < rect.bottom() {
        let mut x = first_x;
        while x < rect.right() {
            let tile_rect = Rect::from_min_size(pos2(x, y), tile);
            if tile_rect.intersects(rect) {
                painter.image(texture.id(), tile_rect, uv, line);
            }
            x += tile.x;
        }
        y += tile.y;
    }
}

/// The doodles' tint over `background`: whichever of near-black or white
/// stands further from it, as opaque as it takes to differ from the
/// background by a fixed step. Any theme colour then shows the doodles about
/// as faintly as WhatsApp's own colours do, mid-tones included.
pub fn doodle_ink(background: Color32) -> Color32 {
    const DARK_INK: u8 = 30;
    const LIGHT_INK: u8 = 255;
    let luma = 0.2126 * f32::from(background.r())
        + 0.7152 * f32::from(background.g())
        + 0.0722 * f32::from(background.b());
    let (ink, step) = if luma - f32::from(DARK_INK) > f32::from(LIGHT_INK) - luma {
        (DARK_INK, 23.2)
    } else {
        (LIGHT_INK, 38.0)
    };
    let distance = (luma - f32::from(ink)).abs().max(1.0);
    let alpha = (step / distance).clamp(0.1, 0.4);
    Color32::from_rgba_unmultiplied(ink, ink, ink, (alpha * 255.0).round() as u8)
}

/// The part of an image of `image` size that covers `area` while keeping its
/// aspect ratio: the whole of its shorter dimension, centred on the longer.
pub fn cover_uv(image: Vec2, area: Vec2) -> Rect {
    let full = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    if image.x <= 0.0 || image.y <= 0.0 || area.x <= 0.0 || area.y <= 0.0 {
        return full;
    }
    let image_aspect = image.x / image.y;
    let area_aspect = area.x / area.y;
    if image_aspect > area_aspect {
        let visible = area_aspect / image_aspect;
        let left = (1.0 - visible) / 2.0;
        Rect::from_min_max(pos2(left, 0.0), pos2(left + visible, 1.0))
    } else {
        let visible = image_aspect / area_aspect;
        let top = (1.0 - visible) / 2.0;
        Rect::from_min_max(pos2(0.0, top), pos2(1.0, top + visible))
    }
}

fn image_texture_id() -> egui::Id {
    egui::Id::new("chat-wallpaper-image")
}

/// The custom image's texture, uploaded once per image and window.
fn image_texture(
    ctx: &egui::Context,
    (generation, image): &(u64, Arc<ColorImage>),
) -> TextureHandle {
    let cached = ctx.data(|data| data.get_temp::<ImageTexture>(image_texture_id()));
    if let Some(cached) = cached.filter(|cached| cached.generation == *generation) {
        return cached.texture;
    }
    let options = TextureOptions {
        mipmap_mode: Some(egui::TextureFilter::Linear),
        ..TextureOptions::LINEAR
    };
    let texture = ctx.load_texture("chat-wallpaper-image", Arc::clone(image), options);
    ctx.data_mut(|data| {
        data.insert_temp(
            image_texture_id(),
            ImageTexture {
                generation: *generation,
                texture: texture.clone(),
            },
        );
    });
    texture
}

/// Drops the custom image's texture, which frees it once nothing draws it.
pub fn forget_image(ctx: &egui::Context) {
    ctx.data_mut(|data| data.remove::<ImageTexture>(image_texture_id()));
}

/// The textures a window holds for the custom image and the doodle tile, for
/// telling them apart in painted shapes.
#[cfg(test)]
pub(crate) fn texture_ids(ctx: &egui::Context) -> (Option<egui::TextureId>, egui::TextureId) {
    let image = ctx.data(|data| data.get_temp::<ImageTexture>(image_texture_id()));
    let doodles = texture(ctx).expect("the doodle tile renders");
    (image.map(|image| image.texture.id()), doodles.id())
}

fn texture(ctx: &egui::Context) -> Option<TextureHandle> {
    let cache = ctx.data_mut(|data| {
        data.get_temp_mut_or_default::<Cache>(egui::Id::new("chat-wallpaper"))
            .clone()
    });
    let mut cached = cache
        .0
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(texture) = cached.as_ref() {
        return Some(texture.clone());
    }

    let image = rasterize()?;
    let texture = ctx.load_texture("chat-wallpaper", image, TextureOptions::LINEAR);
    *cached = Some(texture.clone());
    Some(texture)
}

fn rasterize() -> Option<ColorImage> {
    // The source uses currentColor. Render a white mask once, then use the
    // painter tint to adapt the line colour and opacity to the active theme.
    let source = String::from_utf8_lossy(DEFAULT_SVG).replace("currentColor", "#ffffff");
    let tree =
        resvg::usvg::Tree::from_data(source.as_bytes(), &resvg::usvg::Options::default()).ok()?;
    let size = tree.size();
    let width = size.width().round() as u32;
    let height = size.height().round() as u32;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height)?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::identity(),
        &mut pixmap.as_mut(),
    );
    let rgba = pixmap
        .pixels()
        .iter()
        .flat_map(|pixel| {
            let color = pixel.demultiply();
            [color.red(), color.green(), color.blue(), color.alpha()]
        })
        .collect::<Vec<u8>>();
    Some(ColorImage::from_rgba_unmultiplied(
        [width as usize, height as usize],
        &rgba,
    ))
}

/// The wallpaper image named in the settings, decoded off the interface
/// thread. A file that is missing or cannot be decoded leaves the colour in
/// place and is not tried again until the setting changes.
#[derive(Default)]
pub struct CustomImage {
    /// The file last asked for.
    path: Option<PathBuf>,
    /// Bumped for every new file, so a window's texture of an earlier one is
    /// replaced even when the copy kept its name.
    generation: u64,
    image: Option<Arc<ColorImage>>,
    pending: Option<mpsc::Receiver<Result<ColorImage, String>>>,
}

impl CustomImage {
    /// Follows the setting: decodes a newly named file on a thread, drops the
    /// image when the setting is cleared, and collects a finished decode.
    pub fn sync(&mut self, wanted: Option<&Path>, waker: &crate::backend::Waker) {
        if self.path.as_deref() != wanted {
            self.path = wanted.map(Path::to_path_buf);
            self.generation += 1;
            self.image = None;
            self.pending = None;
            if let Some(path) = self.path.clone() {
                let (sender, receiver) = mpsc::channel();
                let waker = waker.clone();
                let spawned = std::thread::Builder::new()
                    .name("wallpaper-image".into())
                    .spawn(move || {
                        let _ = sender.send(decode(&path));
                        waker.wake();
                    });
                match spawned {
                    Ok(_) => self.pending = Some(receiver),
                    Err(error) => log::debug!("could not start decoding the wallpaper: {error}"),
                }
            }
        }
        let Some(pending) = &self.pending else {
            return;
        };
        match pending.try_recv() {
            Ok(Ok(image)) => {
                self.image = Some(Arc::new(image));
                self.pending = None;
            }
            Ok(Err(error)) => {
                let name = self
                    .path
                    .as_deref()
                    .and_then(Path::file_name)
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                log::debug!("wallpaper image {name} is not shown: {error}");
                self.pending = None;
            }
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => self.pending = None,
        }
    }

    /// Decodes the file again at the next [`Self::sync`], for a new image
    /// copied over the old one's name.
    pub fn reload(&mut self) {
        self.path = None;
    }

    /// The decoded image, if one is ready, with its generation.
    pub fn ready(&self) -> Option<(u64, Arc<ColorImage>)> {
        self.image
            .as_ref()
            .map(|image| (self.generation, Arc::clone(image)))
    }

    /// Shows `image` for `path` at once, for demos and layout tests.
    #[cfg(any(test, feature = "demo"))]
    pub fn show_now(&mut self, path: &Path, image: ColorImage) {
        self.path = Some(path.to_path_buf());
        self.generation += 1;
        self.image = Some(Arc::new(image));
        self.pending = None;
    }
}

/// Reads an image for display, no larger than [`MAX_SIDE`].
pub fn decode(path: &Path) -> Result<ColorImage, String> {
    let decoded = image::ImageReader::open(path)
        .and_then(image::ImageReader::with_guessed_format)
        .map_err(|error| error.to_string())?
        .decode()
        .map_err(|error| error.to_string())?;
    let rgba = bounded(decoded).to_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    Ok(ColorImage::from_rgba_unmultiplied(size, rgba.as_raw()))
}

fn bounded(image: image::DynamicImage) -> image::DynamicImage {
    if image.width().max(image.height()) > MAX_SIDE {
        image.resize(MAX_SIDE, MAX_SIDE, image::imageops::FilterType::Triangle)
    } else {
        image
    }
}

/// Copies a chosen image to ZapFast's own `wallpaper.<ext>`, so the original
/// may move or go. An image larger than [`MAX_SIDE`] is scaled down first and
/// stored as JPEG, or PNG when it has transparency; a smaller one is copied as
/// it is. Earlier copies are removed once the new one is in place.
pub fn import(source: &Path, state: &Path) -> Result<PathBuf, String> {
    let bytes = std::fs::read(source).map_err(|error| error.to_string())?;
    let format = image::guess_format(&bytes).map_err(|_| "not an image".to_owned())?;
    let decoded =
        image::load_from_memory_with_format(&bytes, format).map_err(|error| error.to_string())?;
    let (contents, extension) = if decoded.width().max(decoded.height()) > MAX_SIDE {
        encode(&bounded(decoded))?
    } else {
        let extension = match format {
            image::ImageFormat::Jpeg => "jpg",
            image::ImageFormat::Png => "png",
            image::ImageFormat::WebP => "webp",
            image::ImageFormat::Gif => "gif",
            _ => return Err("unsupported image format".to_owned()),
        };
        (bytes, extension)
    };
    std::fs::create_dir_all(state).map_err(|error| error.to_string())?;
    let target = state.join(format!("wallpaper.{extension}"));
    let temporary = state.join("wallpaper.tmp");
    std::fs::write(&temporary, contents)
        .and_then(|()| std::fs::rename(&temporary, &target))
        .map_err(|error| {
            let _ = std::fs::remove_file(&temporary);
            error.to_string()
        })?;
    for other in EXTENSIONS.into_iter().filter(|other| *other != extension) {
        let _ = std::fs::remove_file(state.join(format!("wallpaper.{other}")));
    }
    Ok(target)
}

fn encode(image: &image::DynamicImage) -> Result<(Vec<u8>, &'static str), String> {
    let mut contents = Vec::new();
    let extension = if image.color().has_alpha() {
        image
            .write_to(
                &mut std::io::Cursor::new(&mut contents),
                image::ImageFormat::Png,
            )
            .map_err(|error| error.to_string())?;
        "png"
    } else {
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut contents, 90)
            .encode_image(&image.to_rgb8())
            .map_err(|error| error.to_string())?;
        "jpg"
    };
    Ok((contents, extension))
}

/// Deletes ZapFast's copy of the wallpaper image, whatever its extension.
pub fn remove(state: &Path) {
    for extension in EXTENSIONS {
        match std::fs::remove_file(state.join(format!("wallpaper.{extension}"))) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => log::debug!("could not delete the wallpaper image: {error}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::AppDirs;
    use egui::vec2;

    #[test]
    fn default_tile_keeps_its_intrinsic_dimensions() {
        let image = rasterize().expect("default wallpaper SVG renders");
        assert_eq!(image.size, [374, 666]);
        assert!(image.pixels.iter().any(|pixel| pixel.a() != 0));
    }

    #[test]
    fn cover_fit_fills_the_area_and_crops_the_centre() {
        let close = |a: Rect, b: Rect| {
            assert!(
                (a.min - b.min).length() < 1e-5 && (a.max - b.max).length() < 1e-5,
                "{a:?} != {b:?}"
            );
        };
        let full = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
        close(cover_uv(vec2(1600.0, 900.0), vec2(800.0, 450.0)), full);
        // A wide image in a square keeps its middle square.
        close(
            cover_uv(vec2(2000.0, 1000.0), vec2(500.0, 500.0)),
            Rect::from_min_max(pos2(0.25, 0.0), pos2(0.75, 1.0)),
        );
        // A tall image in a wide area keeps its middle band.
        close(
            cover_uv(vec2(1000.0, 2000.0), vec2(800.0, 400.0)),
            Rect::from_min_max(pos2(0.0, 0.375), pos2(1.0, 0.625)),
        );
        // The shown part always has the area's aspect ratio.
        let uv = cover_uv(vec2(1234.0, 567.0), vec2(300.0, 700.0));
        let shown = vec2(uv.width() * 1234.0, uv.height() * 567.0);
        assert!((shown.x / shown.y - 300.0 / 700.0).abs() < 1e-4);
        close(cover_uv(vec2(0.0, 10.0), vec2(10.0, 10.0)), full);
        close(cover_uv(vec2(10.0, 10.0), vec2(0.0, 0.0)), full);
    }

    #[test]
    fn doodles_show_on_every_background() {
        for level in 0..=255u8 {
            for background in [
                Color32::from_rgb(level, level, level),
                Color32::from_rgb(level, 0, 0),
                Color32::from_rgb(0, level, level / 2),
            ] {
                let ink = doodle_ink(background).to_srgba_unmultiplied();
                let alpha = f32::from(ink[3]) / 255.0;
                let over = |channel: u8, ink: u8| {
                    f32::from(channel) * (1.0 - alpha) + f32::from(ink) * alpha
                };
                let luma = |r: f32, g: f32, b: f32| 0.2126 * r + 0.7152 * g + 0.0722 * b;
                let before = luma(
                    f32::from(background.r()),
                    f32::from(background.g()),
                    f32::from(background.b()),
                );
                let after = luma(
                    over(background.r(), ink[0]),
                    over(background.g(), ink[1]),
                    over(background.b(), ink[2]),
                );
                assert!(
                    (after - before).abs() >= 20.0,
                    "doodles fade into {background:?}: {before} to {after}"
                );
            }
        }
        // WhatsApp's own light and dark defaults keep their familiar look.
        assert_eq!(ink_alpha(doodle_ink(Color32::from_rgb(245, 241, 235))), 28);
        assert_eq!(ink_alpha(doodle_ink(Color32::from_rgb(22, 23, 23))), 42);
    }

    fn ink_alpha(ink: Color32) -> u8 {
        ink.to_srgba_unmultiplied()[3]
    }

    fn write_image(path: &Path, width: u32, height: u32) {
        let image = image::RgbImage::from_fn(width, height, |x, y| {
            image::Rgb([(x % 256) as u8, (y % 256) as u8, 128])
        });
        image.save(path).unwrap();
    }

    #[test]
    fn a_chosen_image_is_copied_scaled_and_removed() {
        let root = tempfile::tempdir().unwrap();
        let dirs = AppDirs::under(&root.path().join("app"));
        let chosen = root.path().join("holiday.png");
        write_image(&chosen, 64, 32);

        let small = import(&chosen, &dirs.state).unwrap();
        assert_eq!(small, dirs.wallpaper_file("png"));
        assert_eq!(
            std::fs::read(&small).unwrap(),
            std::fs::read(&chosen).unwrap()
        );
        assert_eq!(decode(&small).unwrap().size, [64, 32]);
        assert!(chosen.exists(), "the original stays where it was");

        // A large photo is scaled to the bound, keeps its shape, and
        // replaces the earlier copy.
        let large = root.path().join("panorama.jpg");
        write_image(&large, 4000, 1000);
        let copy = import(&large, &dirs.state).unwrap();
        assert_eq!(copy, dirs.wallpaper_file("jpg"));
        assert!(!small.exists(), "the earlier copy is gone");
        let (width, height) = image::image_dimensions(&copy).unwrap();
        assert_eq!((width, height), (MAX_SIDE, 640));
        assert_eq!(decode(&copy).unwrap().size, [2560, 640]);

        // Something that is not an image is refused and the copy survives.
        let text = root.path().join("notes.png");
        std::fs::write(&text, "not a picture").unwrap();
        assert!(import(&text, &dirs.state).is_err());
        assert!(copy.exists());

        remove(&dirs.state);
        assert!(!copy.exists());
        assert!(
            std::fs::read_dir(&dirs.state).unwrap().next().is_none(),
            "nothing is left behind"
        );
        remove(&dirs.state);
    }

    #[test]
    fn a_missing_or_damaged_image_falls_back_to_the_colour() {
        let root = tempfile::tempdir().unwrap();
        let damaged = root.path().join("wallpaper.png");
        std::fs::write(&damaged, "damaged").unwrap();
        let waker = crate::backend::Waker::default();
        for path in [damaged, root.path().join("missing.jpg")] {
            let mut custom = CustomImage::default();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            custom.sync(Some(&path), &waker);
            while custom.pending.is_some() && std::time::Instant::now() < deadline {
                std::thread::yield_now();
                custom.sync(Some(&path), &waker);
            }
            assert!(custom.pending.is_none());
            assert!(custom.ready().is_none());
        }
    }

    #[test]
    fn the_image_follows_the_setting() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("wallpaper.png");
        write_image(&path, 20, 10);
        let waker = crate::backend::Waker::default();
        let mut custom = CustomImage::default();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        custom.sync(Some(&path), &waker);
        while custom.ready().is_none() && std::time::Instant::now() < deadline {
            std::thread::yield_now();
            custom.sync(Some(&path), &waker);
        }
        let (generation, image) = custom.ready().expect("decoded");
        assert_eq!(image.size, [20, 10]);
        custom.reload();
        custom.sync(Some(&path), &waker);
        assert!(custom.ready().is_none_or(|(next, _)| next > generation));
        custom.sync(None, &waker);
        assert!(custom.ready().is_none());
    }
}
