//! Vector-painted pairing QR code for sharp output at any zoom.

use egui::{Color32, Rect, Ui, pos2, vec2};

/// QR quiet-zone width in modules.
const QUIET: usize = 4;

pub struct Qr {
    width: usize,
    dark: Vec<bool>,
}

impl Qr {
    pub fn encode(text: &str) -> Option<Self> {
        let code = qrcode::QrCode::new(text.as_bytes()).ok()?;
        let width = code.width();
        let dark = code
            .to_colors()
            .into_iter()
            .map(|color| color == qrcode::Color::Dark)
            .collect();
        Some(Self { width, dark })
    }

    /// Paints the largest centered code that fits `rect`, including a white quiet zone.
    pub fn paint(&self, ui: &Ui, rect: Rect, dark: Color32, light: Color32) {
        let total = self.width + 2 * QUIET;
        // Use whole device pixels per module to avoid seams.
        let ppp = ui.ctx().pixels_per_point();
        let module = ((rect.width().min(rect.height()) / total as f32) * ppp).floor() / ppp;
        if module <= 0.0 {
            return;
        }
        let side = module * total as f32;
        let origin = rect.center() - vec2(side, side) / 2.0;
        let painter = ui.painter();
        painter.rect_filled(Rect::from_min_size(origin, vec2(side, side)), 4.0, light);
        let content = origin + vec2(module, module) * QUIET as f32;
        for y in 0..self.width {
            let mut x = 0;
            while x < self.width {
                if !self.dark[y * self.width + x] {
                    x += 1;
                    continue;
                }
                let start = x;
                while x < self.width && self.dark[y * self.width + x] {
                    x += 1;
                }
                let min = content + vec2(start as f32 * module, y as f32 * module);
                let max = pos2(min.x + (x - start) as f32 * module, min.y + module);
                painter.rect_filled(Rect::from_min_max(min, max), 0.0, dark);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_a_linking_string() {
        let qr = Qr::encode("2@abcdefghijklmnopqrstuvwxyz,ABCDEFGHIJKLMNOPQRSTUVWXYZ,0123456789,=")
            .expect("encodes");
        assert!(qr.width >= 21);
        assert_eq!(qr.dark.len(), qr.width * qr.width);
        // The finder-pattern corner is always dark.
        assert!(qr.dark[0]);
    }
}
