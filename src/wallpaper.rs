//! The default doodle chat wallpaper.

use std::sync::{Arc, Mutex};

use egui::{Color32, ColorImage, Rect, TextureHandle, TextureOptions, pos2};

use crate::settings::WallpaperColor;

/// The default doodle tile, drawn from Lucide icons (ISC, see
/// `assets/icons/LICENSE.txt`). It is embedded so the wallpaper works offline
/// and does not depend on a third-party request at runtime.
const DEFAULT_SVG: &[u8] = include_bytes!("../assets/wallpaper.svg");

#[derive(Clone, Default)]
struct Cache(Arc<Mutex<Option<TextureHandle>>>);

/// Paint the wallpaper over the conversation panel, preserving the SVG's
/// intrinsic 374 x 666 logical-pixel tile and repeating it in both axes.
pub fn paint(ui: &mut egui::Ui, color: WallpaperColor) {
    paint_rect(ui, ui.max_rect(), color, true);
}

/// Paint a wallpaper preview into a bounded rectangle.
pub fn paint_rect(ui: &egui::Ui, rect: Rect, color: WallpaperColor, doodles: bool) {
    let background = color.color32();
    let painter = ui.painter().with_clip_rect(rect);
    painter.rect_filled(rect, 0.0, background);

    if !doodles {
        return;
    }

    let Some(texture) = texture(ui.ctx()) else {
        return;
    };

    let tile = texture.size_vec2();
    if tile.x <= 0.0 || tile.y <= 0.0 {
        return;
    }

    let luminance = 0.2126 * f32::from(background.r())
        + 0.7152 * f32::from(background.g())
        + 0.0722 * f32::from(background.b());
    let line = if luminance > 150.0 {
        Color32::from_rgba_unmultiplied(30, 30, 30, 28)
    } else {
        Color32::from_rgba_unmultiplied(255, 255, 255, 42)
    };
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_tile_keeps_its_intrinsic_dimensions() {
        let image = rasterize().expect("default wallpaper SVG renders");
        assert_eq!(image.size, [374, 666]);
        assert!(image.pixels.iter().any(|pixel| pixel.a() != 0));
    }
}
