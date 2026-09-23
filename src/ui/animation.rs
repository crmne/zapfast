//! Small, event-driven transitions for interface controls.

use std::time::Duration;

use egui::{Context, Id};

/// Initial timing target for WhatsApp-like interface motion.
pub const WHATSAPP_UI_DURATION: f32 = 0.150;

/// CSS `cubic-bezier(.31,.94,.34,1)` evaluated at `t` in `0..=1`.
///
/// CSS cubic Béziers use x as time, so solve the x component first and then
/// return the corresponding y component. A short fixed binary search is both
/// stable and more than accurate enough for a 150 ms interface transition.
pub fn whatsapp_ease(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    if t <= 0.0 || t >= 1.0 {
        return t;
    }
    let mut low = 0.0;
    let mut high = 1.0;
    for _ in 0..16 {
        let u = (low + high) * 0.5;
        if cubic(u, 0.31, 0.34) < t {
            low = u;
        } else {
            high = u;
        }
    }
    cubic((low + high) * 0.5, 0.94, 1.0)
}

fn cubic(t: f32, p1: f32, p2: f32) -> f32 {
    let inverse = 1.0 - t;
    3.0 * inverse * inverse * t * p1 + 3.0 * inverse * t * t * p2 + t * t * t
}

/// Animate a boolean with the shared timing and easing policy.
pub fn bool_factor(ctx: &Context, id: Id, target: bool) -> f32 {
    ctx.animate_bool_with_time_and_easing(id, target, WHATSAPP_UI_DURATION, whatsapp_ease)
}

/// Animate a scalar with the shared timing. The scalar is useful for opacity,
/// height, and visual scale where a boolean transition is not sufficient.
pub fn value(ctx: &Context, id: Id, target: f32) -> f32 {
    ctx.animate_value_with_time(id, target, WHATSAPP_UI_DURATION)
}

/// Keep a frame alive while a transition is expected to settle.
pub fn settle_repaint(ctx: &Context) {
    ctx.request_repaint_after(Duration::from_millis(16));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn easing_has_expected_endpoints_and_is_monotonic() {
        assert_eq!(whatsapp_ease(0.0), 0.0);
        assert_eq!(whatsapp_ease(1.0), 1.0);
        let mut previous = 0.0;
        for step in 1..=100 {
            let current = whatsapp_ease(step as f32 / 100.0);
            assert!(current >= previous);
            previous = current;
        }
    }

    #[test]
    fn duration_is_the_initial_whatsapp_target() {
        assert!((WHATSAPP_UI_DURATION - 0.150).abs() < f32::EPSILON);
    }
}
