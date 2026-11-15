//! Bounded photo markup shared by the Status canvas, preview and final pixels.

use egui::{Color32, ColorImage, FontId, Pos2, Rect, Vec2, pos2, vec2};
use std::{path::Path, sync::Arc};

pub const MAX_MARKS: usize = 64;
pub const PREVIEW_SIDE: u32 = 1024;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tool {
    #[default]
    Pen,
    Text,
    Rectangle,
    Ellipse,
    Arrow,
    Blur,
    Effects,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Effect {
    #[default]
    Original,
    Mono,
    Warm,
    Cool,
    Vivid,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Quality {
    #[default]
    Standard,
    Hd,
}

impl Quality {
    pub fn side(self) -> u32 {
        match self {
            Self::Standard => 1600,
            Self::Hd => 4096,
        }
    }
    fn jpeg(self) -> u8 {
        match self {
            Self::Standard => 80,
            Self::Hd => 92,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Mark {
    Pen {
        points: Vec<Pos2>,
        color: Color32,
        width: f32,
    },
    Shape {
        tool: Tool,
        from: Pos2,
        to: Pos2,
        color: Color32,
        width: f32,
        filled: bool,
    },
    Text {
        at: Pos2,
        text: String,
        font: usize,
        size: f32,
        color: Color32,
        boxed: bool,
    },
    Blur {
        from: Pos2,
        to: Pos2,
    },
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Scene {
    pub effect: Effect,
    pub marks: Vec<Arc<Mark>>,
}

impl Scene {
    pub fn validate(&self) -> Result<(), String> {
        let point = |p: Pos2| {
            p.x.is_finite()
                && p.y.is_finite()
                && (0.0..=1.0).contains(&p.x)
                && (0.0..=1.0).contains(&p.y)
        };
        let width = |w: f32| w.is_finite() && (0.001..=0.05).contains(&w);
        let mut points = 0;
        if self.marks.len() > MAX_MARKS {
            return Err("This photo has too many edits".into());
        }
        for mark in &self.marks {
            let valid = match mark.as_ref() {
                Mark::Pen {
                    points: path,
                    width: w,
                    ..
                } => {
                    points += path.len();
                    !path.is_empty() && path.iter().all(|p| point(*p)) && width(*w)
                }
                Mark::Shape {
                    tool,
                    from,
                    to,
                    width: w,
                    ..
                } => {
                    point(*from)
                        && point(*to)
                        && width(*w)
                        && matches!(tool, Tool::Rectangle | Tool::Ellipse | Tool::Arrow)
                }
                Mark::Text {
                    at,
                    text,
                    font,
                    size,
                    ..
                } => {
                    point(*at)
                        && text.chars().count() <= 700
                        && *font < 3
                        && size.is_finite()
                        && (0.015..=0.2).contains(size)
                }
                Mark::Blur { from, to } => point(*from) && point(*to),
            };
            if !valid || points > 8192 {
                return Err("This photo edit is invalid or too large".into());
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Editor {
    pub target: crate::app::AttachmentId,
    pub scene: Scene,
    pub undo: Vec<Scene>,
    pub redo: Vec<Scene>,
    pub tool: Tool,
    pub color: Color32,
    pub width: f32,
    pub filled: bool,
    pub highlighter: bool,
    pub text: String,
    pub font: usize,
    pub text_size: f32,
    pub boxed: bool,
    pub gesture: Vec<Pos2>,
    pub revision: u64,
    pub requested: Option<u64>,
    pub preview_revision: Option<u64>,
    pub preview: Option<Arc<ColorImage>>,
    pub dimensions: Option<[u32; 2]>,
    pub applying: bool,
    pub crop_after: bool,
    pub limit_reached: bool,
}

impl Editor {
    pub fn new(target: crate::app::AttachmentId, tool: Tool) -> Self {
        Self {
            target,
            scene: Scene::default(),
            undo: Vec::new(),
            redo: Vec::new(),
            tool,
            color: Color32::WHITE,
            width: 0.006,
            filled: false,
            highlighter: false,
            text: String::new(),
            font: 0,
            text_size: 0.06,
            boxed: false,
            gesture: Vec::new(),
            revision: 0,
            requested: None,
            preview_revision: None,
            preview: None,
            dimensions: None,
            applying: false,
            crop_after: false,
            limit_reached: false,
        }
    }
    pub fn remember(&mut self) {
        self.limit_reached = false;
        if self.undo.len() == 32 {
            self.undo.remove(0);
        }
        self.undo.push(self.scene.clone());
        self.redo.clear();
        self.revision = self.revision.wrapping_add(1);
    }
    pub fn add(&mut self, mark: Mark) {
        let mut candidate = self.scene.clone();
        candidate.marks.push(Arc::new(mark));
        if candidate.validate().is_err() {
            self.limit_reached = true;
            return;
        }
        self.remember();
        self.scene = candidate;
    }
    pub fn history(&mut self, redo: bool) {
        self.limit_reached = false;
        let previous = if redo {
            self.redo.pop()
        } else {
            self.undo.pop()
        };
        if let Some(previous) = previous {
            let current = std::mem::replace(&mut self.scene, previous);
            if redo {
                self.undo.push(current);
            } else {
                self.redo.push(current);
            }
            self.revision = self.revision.wrapping_add(1);
            self.gesture.clear();
        }
    }
}

pub fn font(style: usize, size: f32) -> FontId {
    match style % 3 {
        1 => crate::theme::bold(size),
        2 => FontId::monospace(size),
        _ => crate::theme::regular(size),
    }
}

/// No external resources or SVG text engine: export uses the same glyph atlas as the UI.
fn paint_text(
    picture: &mut image::RgbaImage,
    ctx: &egui::Context,
    at: Pos2,
    text: &str,
    font: FontId,
    color: Color32,
    boxed: bool,
) {
    let width = picture.width() as f32 * 0.9;
    let (galley, atlas) = ctx.fonts_mut(|fonts| {
        let galley = fonts.layout(text.to_owned(), font, color, width);
        (galley, fonts.image())
    });
    let origin = at - galley.size() * 0.5;
    if boxed {
        let rect = Rect::from_min_size(origin, galley.size()).expand(8.0);
        let ink = if color.intensity() > 0.5 {
            [0, 0, 0, 210]
        } else {
            [255, 255, 255, 230]
        };
        let background = image::RgbaImage::from_pixel(
            rect.width().ceil() as u32,
            rect.height().ceil() as u32,
            image::Rgba(ink),
        );
        image::imageops::overlay(picture, &background, rect.left() as i64, rect.top() as i64);
    }
    for row in &galley.rows {
        let mesh = &row.row.visuals.mesh;
        for quad in mesh.vertices[row.row.visuals.glyph_vertex_range.clone()]
            .as_chunks::<4>()
            .0
        {
            let min = quad[0].uv;
            let max = quad[3].uv;
            let (w, h) = ((max.x - min.x) as usize, (max.y - min.y) as usize);
            if w == 0 || h == 0 {
                continue;
            }
            let mut rgba = image::RgbaImage::new(w as u32, h as u32);
            let tint = quad[0].color.to_srgba_unmultiplied();
            for (x, y, pixel) in rgba.enumerate_pixels_mut() {
                let index =
                    (min.y as usize + y as usize) * atlas.size[0] + min.x as usize + x as usize;
                let Some(sample) = atlas.pixels.get(index) else {
                    continue;
                };
                let sample = sample.to_srgba_unmultiplied();
                *pixel = image::Rgba(std::array::from_fn(|i| {
                    ((u16::from(sample[i]) * u16::from(tint[i])) / 255) as u8
                }));
            }
            let extent = quad[3].pos - quad[0].pos;
            let rgba = image::imageops::resize(
                &rgba,
                extent.x.ceil().max(1.0) as u32,
                extent.y.ceil().max(1.0) as u32,
                image::imageops::FilterType::Triangle,
            );
            let pos = origin + row.pos.to_vec2() + quad[0].pos.to_vec2();
            image::imageops::overlay(picture, &rgba, pos.x.round() as i64, pos.y.round() as i64);
        }
    }
}

pub fn decode(bytes: &[u8]) -> Result<image::DynamicImage, String> {
    let read = || {
        image::ImageReader::new(std::io::Cursor::new(bytes))
            .with_guessed_format()
            .map_err(|_| "Could not read this photo".to_owned())
    };
    let (w, h) = read()?
        .into_dimensions()
        .map_err(|_| "Could not read this photo")?;
    if w == 0 || h == 0 || u64::from(w) * u64::from(h) > 32_000_000 {
        return Err("Choose a photo smaller than 32 megapixels".into());
    }
    let mut reader = read()?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(256 * 1024 * 1024);
    reader.limits(limits);
    use image::ImageDecoder;
    let mut decoder = reader
        .into_decoder()
        .map_err(|_| "Could not decode this photo")?;
    let orientation = decoder
        .orientation()
        .map_err(|_| "Could not read this photo orientation")?;
    let mut photo =
        image::DynamicImage::from_decoder(decoder).map_err(|_| "Could not decode this photo")?;
    photo.apply_orientation(orientation);
    Ok(photo)
}

pub fn render(
    bytes: &[u8],
    scene: &Scene,
    side: Option<u32>,
) -> Result<(image::RgbaImage, [u32; 2]), String> {
    scene.validate()?;
    let source = decode(bytes)?;
    let dimensions = [source.width(), source.height()];
    if side == Some(0) {
        return Err("Invalid preview size".into());
    }
    let source = if let Some(side) = side.filter(|side| source.width().max(source.height()) > *side)
    {
        source.thumbnail(side, side)
    } else {
        source
    };
    let mut picture = source.to_rgba8();
    for pixel in picture.pixels_mut() {
        let [r, g, b, a] = pixel.0;
        let channel = |v: f32| v.round().clamp(0.0, 255.0) as u8;
        pixel.0 = match scene.effect {
            Effect::Original => [r, g, b, a],
            Effect::Mono => {
                let l =
                    channel(0.2126 * f32::from(r) + 0.7152 * f32::from(g) + 0.0722 * f32::from(b));
                [l, l, l, a]
            }
            Effect::Warm => [
                r.saturating_add(14),
                g.saturating_add(4),
                b.saturating_sub(10),
                a,
            ],
            Effect::Cool => [
                r.saturating_sub(10),
                g.saturating_add(3),
                b.saturating_add(14),
                a,
            ],
            Effect::Vivid => [
                channel((f32::from(r) - 128.0) * 1.15 + 128.0),
                channel((f32::from(g) - 128.0) * 1.15 + 128.0),
                channel((f32::from(b) - 128.0) * 1.15 + 128.0),
                a,
            ],
        };
    }
    let size = vec2(picture.width() as f32, picture.height() as f32);
    let point = |p: Pos2| pos2(p.x * size.x, p.y * size.y);
    let ctx = scene
        .marks
        .iter()
        .any(|mark| matches!(mark.as_ref(), Mark::Text { .. }))
        .then(TextPass::begin);
    for mark in &scene.marks {
        match mark.as_ref() {
            Mark::Blur { from, to } => {
                let rect = Rect::from_two_pos(point(*from), point(*to));
                let (x, y) = (rect.left().floor() as u32, rect.top().floor() as u32);
                let w = (rect.width().ceil() as u32).min(picture.width().saturating_sub(x));
                let h = (rect.height().ceil() as u32).min(picture.height().saturating_sub(y));
                if w == 0 || h == 0 {
                    continue;
                }
                let patch = image::imageops::crop_imm(&picture, x, y, w, h).to_image();
                let patch = image::imageops::blur(&patch, size.min_elem() * 0.018);
                image::imageops::replace(&mut picture, &patch, i64::from(x), i64::from(y));
            }
            Mark::Text {
                at,
                text,
                font: style,
                size: scale,
                color,
                boxed,
            } => paint_text(
                &mut picture,
                &ctx.as_ref().expect("text has a font context").0,
                point(*at),
                text,
                font(*style, scale * size.min_elem()),
                *color,
                *boxed,
            ),
            mark => {
                paint_vector(&mut picture, mark, size)?;
            }
        }
    }
    drop(ctx);
    Ok((picture, dimensions))
}

/// One font context for every render: installing the fonts is the slow part. Renders
/// take turns with it, and a pass always ends, even when a render returns early.
struct TextPass(std::sync::MutexGuard<'static, egui::Context>);

impl TextPass {
    fn begin() -> Self {
        static CONTEXT: std::sync::OnceLock<std::sync::Mutex<egui::Context>> =
            std::sync::OnceLock::new();
        let ctx = CONTEXT
            .get_or_init(|| {
                let ctx = egui::Context::default();
                crate::theme::install_fonts(&ctx);
                std::sync::Mutex::new(ctx)
            })
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        ctx.begin_pass(egui::RawInput::default());
        Self(ctx)
    }
}

impl Drop for TextPass {
    fn drop(&mut self) {
        let mut output = self.0.end_pass();
        output.textures_delta.clear();
    }
}

fn paint_vector(picture: &mut image::RgbaImage, mark: &Mark, size: Vec2) -> Result<(), String> {
    use resvg::tiny_skia::{self, PathBuilder};
    let mut path = PathBuilder::new();
    let point = |p: Pos2| pos2(p.x * size.x, p.y * size.y);
    let (color, width, filled) = match mark {
        Mark::Pen {
            points,
            color,
            width,
        } => {
            let first = point(points[0]);
            path.move_to(first.x, first.y);
            if points.len() == 1 {
                path.line_to(first.x + 0.1, first.y);
            }
            for p in &points[1..] {
                let p = point(*p);
                path.line_to(p.x, p.y);
            }
            (*color, *width, false)
        }
        Mark::Shape {
            tool,
            from,
            to,
            color,
            width,
            filled,
        } => {
            let (from, to) = (point(*from), point(*to));
            let rect = Rect::from_two_pos(from, to);
            match tool {
                Tool::Rectangle => {
                    if let Some(rect) = tiny_skia::Rect::from_xywh(
                        rect.left(),
                        rect.top(),
                        rect.width().max(0.1),
                        rect.height().max(0.1),
                    ) {
                        path.push_rect(rect);
                    }
                }
                Tool::Ellipse => {
                    for i in 0..=64 {
                        let angle = i as f32 / 64.0 * std::f32::consts::TAU;
                        let p = rect.center()
                            + vec2(
                                angle.cos() * rect.width() / 2.0,
                                angle.sin() * rect.height() / 2.0,
                            );
                        if i == 0 {
                            path.move_to(p.x, p.y);
                        } else {
                            path.line_to(p.x, p.y);
                        }
                    }
                    path.close();
                }
                Tool::Arrow => {
                    path.move_to(from.x, from.y);
                    path.line_to(to.x, to.y);
                    let direction = (to - from).normalized();
                    let wing = size.min_elem() * 0.04;
                    for sign in [-1.0, 1.0] {
                        let p = to - direction * wing
                            + vec2(-direction.y, direction.x) * wing * 0.6 * sign;
                        path.move_to(to.x, to.y);
                        path.line_to(p.x, p.y);
                    }
                }
                _ => return Err("Unsupported photo shape".into()),
            }
            (*color, *width, *filled && *tool != Tool::Arrow)
        }
        _ => return Ok(()),
    };
    let Some(path) = path.finish() else {
        return Ok(());
    };
    let bounds = path.bounds();
    let pad = width * size.min_elem() + 2.0;
    let left = (bounds.left() - pad).floor().max(0.0) as u32;
    let top = (bounds.top() - pad).floor().max(0.0) as u32;
    let right = (bounds.right() + pad).ceil().min(size.x) as u32;
    let bottom = (bounds.bottom() + pad).ceil().min(size.y) as u32;
    if right <= left || bottom <= top {
        return Ok(());
    }
    let mut pixmap = tiny_skia::Pixmap::new(right - left, bottom - top)
        .ok_or("Could not prepare photo markup")?;
    let transform = tiny_skia::Transform::from_translate(-(left as f32), -(top as f32));
    let mut paint = tiny_skia::Paint::default();
    let [r, g, b, a] = color.to_srgba_unmultiplied();
    paint.set_color_rgba8(r, g, b, a);
    if filled {
        pixmap.fill_path(&path, &paint, tiny_skia::FillRule::Winding, transform, None);
    } else {
        let stroke = tiny_skia::Stroke {
            width: width * size.min_elem(),
            line_cap: tiny_skia::LineCap::Round,
            line_join: tiny_skia::LineJoin::Round,
            ..Default::default()
        };
        pixmap.stroke_path(&path, &paint, &stroke, transform, None);
    }
    for (index, ink) in pixmap.pixels().iter().enumerate() {
        let ink = ink.demultiply();
        if ink.alpha() > 0 {
            use image::Pixel;
            let x = left + index as u32 % pixmap.width();
            let y = top + index as u32 / pixmap.width();
            picture.get_pixel_mut(x, y).blend(&image::Rgba([
                ink.red(),
                ink.green(),
                ink.blue(),
                ink.alpha(),
            ]));
        }
    }
    Ok(())
}

pub fn read(path: &Path) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let file = std::fs::File::open(path).map_err(|_| "Could not open this photo")?;
    let mut bytes = Vec::new();
    file.take(128 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Could not read this photo")?;
    if bytes.len() > 128 * 1024 * 1024 {
        return Err("This photo file is too large".into());
    }
    Ok(bytes)
}

pub fn encode(bytes: &[u8], quality: Quality) -> Result<Vec<u8>, String> {
    use image::ImageEncoder;
    let photo = decode(bytes)?;
    let photo = if photo.width().max(photo.height()) > quality.side() {
        photo.thumbnail(quality.side(), quality.side())
    } else {
        photo
    };
    let photo = photo.to_rgb8();
    let mut bytes = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, quality.jpeg())
        .write_image(
            &photo,
            photo.width(),
            photo.height(),
            image::ExtendedColorType::Rgb8,
        )
        .map_err(|_| "Could not encode this photo")?;
    Ok(bytes)
}

#[derive(Clone, Debug, PartialEq)]
pub struct Preview {
    pub image: Arc<ColorImage>,
    pub dimensions: [u32; 2],
}

pub fn preview(path: &Path, scene: &Scene) -> Result<Preview, String> {
    let (image, dimensions) = render(&read(path)?, scene, Some(PREVIEW_SIDE))?;
    Ok(Preview {
        image: Arc::new(ColorImage::from_rgba_unmultiplied(
            [image.width() as usize, image.height() as usize],
            image.as_raw(),
        )),
        dimensions,
    })
}

pub fn write(path: &Path, scene: &Scene, dir: &Path) -> Result<std::path::PathBuf, String> {
    let (picture, _) = render(&read(path)?, scene, Some(Quality::Hd.side()))?;
    let mut bytes = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(picture)
        .write_to(&mut bytes, image::ImageFormat::Png)
        .map_err(|_| "Could not write this photo")?;
    crate::backend::picture_edit::save(
        crate::backend::picture_edit::Edited {
            bytes: bytes.into_inner(),
            extension: "png",
        },
        dir,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn png() -> Vec<u8> {
        let mut out = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            240,
            160,
            image::Rgba([100, 120, 150, 255]),
        ))
        .write_to(&mut out, image::ImageFormat::Png)
        .unwrap();
        out.into_inner()
    }
    #[test]
    fn markup_is_in_the_export_and_preview_uses_the_same_coordinates() {
        let mut scene = Scene::default();
        scene.marks.push(Arc::new(Mark::Shape {
            tool: Tool::Rectangle,
            from: pos2(0.2, 0.2),
            to: pos2(0.8, 0.8),
            color: Color32::RED,
            width: 0.01,
            filled: true,
        }));
        scene.marks.push(Arc::new(Mark::Text {
            at: pos2(0.5, 0.5),
            text: "Hello <&>".into(),
            font: 1,
            size: 0.1,
            color: Color32::WHITE,
            boxed: true,
        }));
        let (full, _) = render(&png(), &scene, None).unwrap();
        let (small, dimensions) = render(&png(), &scene, Some(120)).unwrap();
        assert_eq!(dimensions, [240, 160]);
        assert_eq!(small.dimensions(), (120, 80));
        assert_eq!(full.get_pixel(60, 40).0, [255, 0, 0, 255]);
        assert_eq!(small.get_pixel(30, 20).0, [255, 0, 0, 255]);
        assert!(
            full.pixels()
                .any(|p| p[0] > 220 && p[1] > 220 && p[2] > 220),
            "text glyphs are exported"
        );
        assert_eq!(full.get_pixel(0, 0).0, [100, 120, 150, 255]);
    }
    #[test]
    fn effects_blur_history_and_limits_are_real() {
        let mut editor = Editor::new(999, Tool::Blur);
        editor.add(Mark::Blur {
            from: pos2(0.1, 0.1),
            to: pos2(0.9, 0.9),
        });
        editor.remember();
        editor.scene.effect = Effect::Mono;
        let (photo, _) = render(&png(), &editor.scene, None).unwrap();
        assert_eq!(photo.get_pixel(0, 0)[0], photo.get_pixel(0, 0)[2]);
        editor.history(false);
        assert_eq!(editor.scene.effect, Effect::Original);
        editor.history(false);
        assert!(editor.scene.marks.is_empty());
        editor.history(true);
        assert_eq!(editor.scene.marks.len(), 1);
        editor.scene.marks.push(Arc::new(Mark::Pen {
            points: vec![pos2(f32::NAN, 0.5)],
            color: Color32::WHITE,
            width: 0.01,
        }));
        assert!(editor.scene.validate().is_err());
        assert!(decode(b"invalid").is_err());
    }
    #[test]
    fn hd_does_not_upscale_and_original_stays_untouched() {
        let original = png();
        for quality in [Quality::Standard, Quality::Hd] {
            let encoded = encode(&original, quality).unwrap();
            assert_eq!(decode(&encoded).unwrap().width(), 240);
        }
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source.png");
        std::fs::write(&source, &original).unwrap();
        let edited = write(
            &source,
            &Scene {
                effect: Effect::Warm,
                ..Default::default()
            },
            &root.path().join("edited"),
        )
        .unwrap();
        assert_ne!(source, edited);
        assert_eq!(std::fs::read(source).unwrap(), original);
    }
    #[test]
    fn every_drawing_tool_and_blur_changes_only_its_region() {
        for tool in [Tool::Rectangle, Tool::Ellipse, Tool::Arrow] {
            let scene = Scene {
                marks: vec![Arc::new(Mark::Shape {
                    tool,
                    from: pos2(0.3, 0.3),
                    to: pos2(0.7, 0.7),
                    color: Color32::RED,
                    width: 0.02,
                    filled: false,
                })],
                ..Default::default()
            };
            let (photo, _) = render(&png(), &scene, None).unwrap();
            assert!(
                photo.pixels().any(|p| p[0] > 200 && p[1] < 60),
                "{tool:?} must be drawn at its translated location"
            );
            assert_eq!(photo.get_pixel(0, 0).0, [100, 120, 150, 255]);
        }
        let scene = Scene {
            marks: vec![Arc::new(Mark::Pen {
                points: vec![pos2(0.3, 0.4), pos2(0.6, 0.6)],
                color: Color32::RED,
                width: 0.02,
            })],
            ..Default::default()
        };
        assert!(
            render(&png(), &scene, None)
                .unwrap()
                .0
                .pixels()
                .any(|p| p[0] > 200 && p[1] < 60)
        );
        let mut source = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image::RgbaImage::from_fn(120, 120, |x, y| {
            image::Rgba([if (x + y) % 2 == 0 { 0 } else { 255 }; 4])
        }))
        .write_to(&mut source, image::ImageFormat::Png)
        .unwrap();
        let scene = Scene {
            marks: vec![Arc::new(Mark::Blur {
                from: pos2(0.2, 0.2),
                to: pos2(0.8, 0.8),
            })],
            ..Default::default()
        };
        let (photo, _) = render(source.get_ref(), &scene, None).unwrap();
        assert!((30..225).contains(&photo.get_pixel(60, 60)[0]));
        assert_eq!(photo.get_pixel(0, 0)[0], 0);
    }

    #[test]
    fn hd_keeps_more_pixels_than_standard_without_upscaling() {
        let mut source = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(2400, 1200)
            .write_to(&mut source, image::ImageFormat::Png)
            .unwrap();
        assert_eq!(
            decode(&encode(source.get_ref(), Quality::Standard).unwrap())
                .unwrap()
                .width(),
            1600
        );
        assert_eq!(
            decode(&encode(source.get_ref(), Quality::Hd).unwrap())
                .unwrap()
                .width(),
            2400
        );
    }
}
