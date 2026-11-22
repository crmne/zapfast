//! How round the interface's corners are, from Settings > Appearance.
//!
//! Views keep drawing their own radii; [`Roundness`] scales every rectangle's
//! corners in the frame's output just before it is tessellated, so bubbles,
//! dialogs, menus, pills, pictures and egui's own widgets all follow one
//! setting. Circles (avatars, round buttons, dots) become rectangles whose
//! corners scale the same way, so at 0 they are square too. Paths keep their
//! shape: a bubble's tail and the Messages balloon are outlines, not corners.

use egui::epaint::{CircleShape, ClippedShape, CornerRadius, RectShape, Shape, StrokeKind};
use egui::{Rect, Vec2};

/// Where the factor lives in egui's memory, for the plugin to read.
fn key() -> egui::Id {
    egui::Id::new("zapfast-corner-roundness")
}

/// Sets how round corners are from now on: 0 square, 1 as each view draws
/// them. Asks for a repaint so the change shows at once.
pub fn set(ctx: &egui::Context, roundness: f32) {
    let roundness = roundness.clamp(0.0, 1.0);
    ctx.data_mut(|data| data.insert_temp(key(), roundness));
    ctx.request_repaint();
}

/// The factor [`set`] last stored, 1 when none was.
pub fn get(ctx: &egui::Context) -> f32 {
    ctx.data(|data| data.get_temp(key())).unwrap_or(1.0)
}

/// Scales the corners of everything painted in a frame by [`get`].
#[derive(Default)]
pub struct Roundness;

impl egui::plugin::Plugin for Roundness {
    fn debug_name(&self) -> &'static str {
        "zapfast-corners"
    }

    fn output_hook(&mut self, ctx: &egui::Context, output: &mut egui::FullOutput) {
        let factor = get(ctx);
        if factor >= 1.0 {
            return;
        }
        for ClippedShape { shape, .. } in &mut output.shapes {
            scale(shape, factor);
        }
    }
}

/// Scales one shape's corners, and those of the shapes it groups.
pub fn scale(shape: &mut Shape, factor: f32) {
    match shape {
        Shape::Vec(shapes) => {
            for shape in shapes {
                scale(shape, factor);
            }
        }
        Shape::Rect(rect) => rect.corner_radius = scaled(rect.corner_radius, factor),
        Shape::Circle(circle) => *shape = Shape::Rect(squared(circle, factor)),
        _ => {}
    }
}

fn scaled(radius: CornerRadius, factor: f32) -> CornerRadius {
    let corner = |r: u8| (f32::from(r) * factor).round() as u8;
    CornerRadius {
        nw: corner(radius.nw),
        ne: corner(radius.ne),
        sw: corner(radius.sw),
        se: corner(radius.se),
    }
}

/// A circle as the square around it, its corners `factor` of the radius. A
/// circle's stroke runs along its edge, as a middle stroke does.
fn squared(circle: &CircleShape, factor: f32) -> RectShape {
    RectShape::new(
        Rect::from_center_size(circle.center, Vec2::splat(circle.radius * 2.0)),
        CornerRadius::from((circle.radius * factor).min(255.0)),
        circle.fill,
        circle.stroke,
        StrokeKind::Middle,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Color32, Stroke, pos2};

    fn rect(radius: u8) -> Shape {
        Shape::rect_filled(
            Rect::from_min_size(pos2(0.0, 0.0), Vec2::splat(40.0)),
            radius,
            Color32::WHITE,
        )
    }

    fn radius_of(shape: &Shape) -> CornerRadius {
        match shape {
            Shape::Rect(rect) => rect.corner_radius,
            other => panic!("not a rectangle: {other:?}"),
        }
    }

    #[test]
    fn zero_squares_rectangles_and_circles() {
        let mut shape = Shape::Vec(vec![
            rect(10),
            Shape::circle_filled(pos2(20.0, 20.0), 12.0, Color32::RED),
        ]);
        scale(&mut shape, 0.0);
        let Shape::Vec(shapes) = &shape else {
            panic!("the group stays a group");
        };
        assert_eq!(radius_of(&shapes[0]), CornerRadius::ZERO);
        assert_eq!(radius_of(&shapes[1]), CornerRadius::ZERO);
        let Shape::Rect(square) = &shapes[1] else {
            unreachable!()
        };
        assert_eq!(
            square.rect,
            Rect::from_center_size(pos2(20.0, 20.0), Vec2::splat(24.0))
        );
        assert_eq!(square.fill, Color32::RED);
    }

    #[test]
    fn half_halves_each_corner() {
        let mut shape = rect(10);
        if let Shape::Rect(rect) = &mut shape {
            rect.corner_radius.nw = 0;
        }
        scale(&mut shape, 0.5);
        assert_eq!(
            radius_of(&shape),
            CornerRadius {
                nw: 0,
                ne: 5,
                sw: 5,
                se: 5
            }
        );
    }

    #[test]
    fn a_ring_keeps_its_stroke() {
        let stroke = Stroke::new(2.0, Color32::BLUE);
        let mut shape = Shape::circle_stroke(pos2(0.0, 0.0), 8.0, stroke);
        scale(&mut shape, 0.0);
        let Shape::Rect(square) = shape else {
            panic!("a circle becomes a rectangle");
        };
        assert_eq!(square.stroke, stroke);
        assert_eq!(square.stroke_kind, StrokeKind::Middle);
    }

    #[test]
    fn the_factor_defaults_to_as_drawn() {
        let ctx = egui::Context::default();
        assert_eq!(get(&ctx), 1.0);
        set(&ctx, 3.0);
        assert_eq!(get(&ctx), 1.0);
        set(&ctx, 0.25);
        assert_eq!(get(&ctx), 0.25);
    }
}
