//! Pacing for decorative animations while a screen reader is attached.
//!
//! On Windows a window that redraws every frame keeps its thread busy
//! drawing or blocked in the vsync swap, and that thread is the only one
//! that can answer the `WM_GETOBJECT` messages a screen reader sends to the
//! window. Each one waits for the next frame, a key press needs many, and
//! NVDA freezes for a second or two per key press while, say, a contact is
//! typing. So while a reader is attached, animations that only show that
//! something is happening (typing dots, spinners, the recording light) draw
//! about ten frames a second. Their phase follows the clock, so they move in
//! steps rather than slow down. Without a reader every request is exactly
//! what it was. Video and other content keep their own rate.

use std::time::Duration;

/// The wait between frames of a stepped animation: about ten frames a
/// second, which leaves the window's thread about as free as when idle.
pub const STEP: Duration = Duration::from_millis(100);

/// Leave an idle text cursor visible instead of repainting it indefinitely.
const CURSOR_BLINK_IDLE: f64 = 5.0;

#[derive(Clone, Default)]
struct CursorActivity {
    focus: Option<egui::Id>,
    last_input: Option<f64>,
}

impl CursorActivity {
    fn blinking(&mut self, now: f64, focus: Option<egui::Id>, input: bool) -> bool {
        if focus.is_none() {
            self.focus = None;
            self.last_input = None;
            return false;
        }
        if input || self.focus != focus || self.last_input.is_none() {
            self.focus = focus;
            self.last_input = Some(now);
        }
        now - self.last_input.unwrap_or(now) < CURSOR_BLINK_IDLE
    }
}

/// Restores the context style after the frame, including on early returns.
pub struct CursorStyleGuard(egui::Context);

impl Drop for CursorStyleGuard {
    fn drop(&mut self) {
        self.0
            .global_style_mut(|style| style.visuals.text_cursor.blink = true);
    }
}

/// Stop blinking after inactivity, and restart on input or a focus change.
/// The state belongs to the egui context so a replacement window starts fresh.
/// Keep the returned guard alive while drawing, including top-level modals.
pub fn idle_text_cursor(ui: &mut egui::Ui, user_input: bool) -> Option<CursorStyleGuard> {
    if !ui.visuals().text_cursor.blink {
        return None;
    }
    let ctx = ui.ctx().clone();
    let (now, window_focused) = ctx.input(|input| (input.time, input.focused));
    let focus = if window_focused {
        ctx.memory(|memory| memory.focused())
    } else {
        None
    };
    let id = egui::Id::new("idle-text-cursor");
    let mut activity = ctx.data_mut(|data| data.get_temp::<CursorActivity>(id).unwrap_or_default());
    let blink = activity.blinking(now, focus, user_input);
    ctx.data_mut(|data| data.insert_temp(id, activity));
    ui.style_mut().visuals.text_cursor.blink = blink;
    if !blink && ctx.global_style().visuals.text_cursor.blink {
        ctx.global_style_mut(|style| style.visuals.text_cursor.blink = false);
        Some(CursorStyleGuard(ctx))
    } else {
        None
    }
}

/// Whether a screen reader, or another assistive client, reads the window.
/// egui has no public flag, but it builds accessibility nodes only while
/// AccessKit is active, and the root's node always exists then, so probing
/// it adds nothing to the tree.
pub fn screen_reader_attached(ctx: &egui::Context) -> bool {
    ctx.accesskit_node_builder(egui::accesskit_root_id(), |_| ())
        .is_some()
}

/// Asks for the next frame of a decorative animation: at the next refresh,
/// or a [`STEP`] later while a screen reader is attached.
#[track_caller]
pub fn request_frame(ctx: &egui::Context) {
    if screen_reader_attached(ctx) {
        ctx.request_repaint_after(STEP);
    } else {
        ctx.request_repaint();
    }
}

/// Like [`request_frame`], for an animation that already waits `delay`
/// between frames: at least a [`STEP`] while a screen reader is attached.
#[track_caller]
pub fn request_frame_after(ctx: &egui::Context, delay: Duration) {
    ctx.request_repaint_after(frame_delay(ctx, delay));
}

/// `delay`, or at least a [`STEP`] while a screen reader is attached.
pub fn frame_delay(ctx: &egui::Context, delay: Duration) -> Duration {
    if screen_reader_attached(ctx) {
        delay.max(STEP)
    } else {
        delay
    }
}

/// egui's own spinner, which asks for every frame, stepped while a screen
/// reader is attached. Without one this is exactly `ui.spinner()`.
pub fn egui_spinner(ui: &mut egui::Ui) -> egui::Response {
    if !screen_reader_attached(ui.ctx()) {
        return ui.spinner();
    }
    // The same size, node and stroke as `egui::Spinner`, but stepped.
    let size = ui.style().spacing.interact_size.y;
    let (rect, response) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
    response.widget_info(|| egui::WidgetInfo::new(egui::WidgetType::ProgressIndicator));
    if ui.is_rect_visible(rect) {
        request_frame(ui.ctx());
        let color = ui.visuals().strong_text_color();
        let radius = (rect.height().min(rect.width()) / 2.0) - 2.0;
        let n_points = (radius.round() as u32).clamp(8, 128);
        let time = ui.input(|input| input.time);
        let start_angle = time * std::f64::consts::TAU;
        let end_angle = start_angle + 240_f64.to_radians() * time.sin();
        let points = (0..n_points)
            .map(|index| {
                let angle = egui::emath::lerp(
                    start_angle..=end_angle,
                    f64::from(index) / f64::from(n_points),
                );
                let (sin, cos) = angle.sin_cos();
                rect.center() + radius * egui::vec2(cos as f32, sin as f32)
            })
            .collect();
        ui.painter()
            .add(egui::Shape::line(points, egui::Stroke::new(3.0, color)));
    }
    response
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[test]
    fn idle_cursor_stays_visible_and_resumes_on_input_or_focus_change() {
        let first = egui::Id::new("first");
        let second = egui::Id::new("second");
        let mut activity = CursorActivity::default();
        assert!(!activity.blinking(0.0, None, false));
        assert!(activity.blinking(1.0, Some(first), false));
        assert!(activity.blinking(5.9, Some(first), false));
        assert!(!activity.blinking(6.0, Some(first), false));
        assert!(activity.blinking(7.0, Some(first), true));
        assert!(!activity.blinking(12.0, Some(first), false));
        assert!(activity.blinking(12.0, Some(second), false));
        assert!(!activity.blinking(13.0, None, false));
        assert!(activity.blinking(30.0, Some(second), false));
    }

    #[test]
    fn idle_text_field_does_not_keep_requesting_cursor_repaints() {
        let ctx = reading(false);
        let field = egui::Id::new("idle-field");
        ctx.memory_mut(|memory| memory.request_focus(field));
        let mut text = String::new();
        let mut draw = |ui: &mut egui::Ui| {
            let _cursor_style = idle_text_cursor(ui, false);
            ui.add(egui::TextEdit::singleline(&mut text).id(field));
        };
        let _ = delay_after(&ctx, 1.0, &mut draw);
        let (idle, _) = delay_after(&ctx, 7.0, &mut draw);
        assert!(idle > Duration::from_secs(1), "idle redraw in {idle:?}");

        let (active, _) = delay_after(&ctx, 8.0, |ui| {
            let _cursor_style = idle_text_cursor(ui, true);
            ui.add(egui::TextEdit::singleline(&mut text).id(field));
        });
        assert!(active < Duration::from_secs(1), "cursor did not resume");
    }

    #[test]
    fn idle_modal_field_stops_blinking_and_restores_the_context_style() {
        let ctx = reading(false);
        let field = egui::Id::new("modal-field");
        let mut text = String::new();
        let mut draw = |ui: &mut egui::Ui| {
            let _cursor_style = idle_text_cursor(ui, false);
            egui::Modal::new(egui::Id::new("modal")).show(ui.ctx(), |ui| {
                ui.add(egui::TextEdit::singleline(&mut text).id(field));
            });
        };
        ctx.memory_mut(|memory| memory.request_focus(field));
        let _ = delay_after(&ctx, 1.0, &mut draw);
        let _ = delay_after(&ctx, 2.0, &mut draw);
        let (idle, _) = delay_after(&ctx, 7.0, &mut draw);
        assert!(idle > Duration::from_secs(1), "modal redraw in {idle:?}");
        assert!(ctx.global_style().visuals.text_cursor.blink);
    }

    /// Runs one pass at `time` and returns how long egui asks the window to
    /// wait before the next one.
    fn delay_after(
        ctx: &egui::Context,
        time: f64,
        draw: impl FnMut(&mut egui::Ui),
    ) -> (Duration, Vec<egui::epaint::ClippedShape>) {
        let mut output = ctx.run_ui(
            egui::RawInput {
                time: Some(time),
                ..Default::default()
            },
            draw,
        );
        output.textures_delta.clear();
        (
            output.viewport_output[&egui::ViewportId::ROOT].repaint_delay,
            output.shapes,
        )
    }

    /// A context with AccessKit on or off, past the passes egui asks for
    /// by itself after starting.
    pub(crate) fn reading(reader: bool) -> egui::Context {
        let ctx = egui::Context::default();
        if reader {
            ctx.enable_accesskit();
        }
        settle(&ctx);
        ctx
    }

    /// Runs empty passes until egui asks for no further frame.
    pub(crate) fn settle(ctx: &egui::Context) {
        for _ in 0..10 {
            let mut output = ctx.run_ui(egui::RawInput::default(), |_| ());
            output.textures_delta.clear();
            if output.viewport_output[&egui::ViewportId::ROOT].repaint_delay == Duration::MAX {
                return;
            }
        }
        panic!("egui kept asking for frames");
    }

    /// egui subtracts its predicted frame time, 1/60 s by default, from
    /// every delay: a step is asked for as about 83 ms.
    fn is_a_step(delay: Duration) -> bool {
        delay > Duration::from_millis(75) && delay <= STEP
    }

    #[test]
    fn the_probe_follows_accesskit_and_adds_no_node() {
        let nodes = |reader: bool, probe: bool| {
            let ctx = reading(reader);
            let mut attached = None;
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                ui.label("text");
                if probe {
                    attached = Some(screen_reader_attached(ui.ctx()));
                }
            });
            output.textures_delta.clear();
            let count = output
                .platform_output
                .accesskit_update
                .map(|update| update.nodes.len());
            (attached, count)
        };
        assert_eq!(nodes(false, true), (Some(false), None));
        let (attached, probed) = nodes(true, true);
        assert_eq!(attached, Some(true));
        assert_eq!(probed, nodes(true, false).1);
        assert!(probed.is_some());
    }

    #[test]
    fn decorative_frames_are_immediate_without_a_reader_and_stepped_with_one() {
        let (plain, _) = delay_after(&reading(false), 0.0, |ui| request_frame(ui.ctx()));
        assert_eq!(plain, Duration::ZERO);
        let (stepped, _) = delay_after(&reading(true), 0.0, |ui| request_frame(ui.ctx()));
        assert!(is_a_step(stepped), "{stepped:?}");
    }

    #[test]
    fn timed_animations_wait_at_least_a_step_with_a_reader() {
        let timed = |reader: bool, delay: u64| {
            delay_after(&reading(reader), 0.0, |ui| {
                request_frame_after(ui.ctx(), Duration::from_millis(delay));
            })
            .0
        };
        let predicted = Duration::from_secs_f32(1.0 / 60.0);
        // Without a reader every delay is what was asked for.
        assert_eq!(timed(false, 33), Duration::from_millis(33) - predicted);
        assert_eq!(timed(false, 150), Duration::from_millis(150) - predicted);
        assert!(is_a_step(timed(true, 33)), "{:?}", timed(true, 33));
        // A slower animation keeps its own pace.
        assert_eq!(timed(true, 150), Duration::from_millis(150) - predicted);
    }

    /// Schedules passes the way eframe does, the next one `repaint_delay`
    /// after the previous ended, and counts them over two seconds.
    fn frames_in_two_seconds(reader: bool) -> usize {
        let ctx = reading(reader);
        // A frame of a maximised 4K window costs about this much.
        let cost = 0.010;
        let mut time = 0.0;
        let mut frames = 0;
        while time < 2.0 {
            let (delay, _) = delay_after(&ctx, time, |ui| request_frame(ui.ctx()));
            frames += 1;
            // An immediate repaint waits for the next refresh at most.
            time += cost + delay.as_secs_f64().max(1.0 / 60.0 - cost);
        }
        frames
    }

    #[test]
    fn stepped_animations_draw_about_ten_frames_a_second() {
        let stepped = frames_in_two_seconds(true);
        assert!((18..=24).contains(&stepped), "{stepped} frames in 2 s");
        let plain = frames_in_two_seconds(false);
        assert!(plain >= 118, "{plain} frames in 2 s");
    }

    #[test]
    fn the_stepped_egui_spinner_draws_egui_s_own_at_the_same_time() {
        let draw = |reader: bool, time: f64| {
            let ctx = reading(reader);
            let mut node = None;
            let (delay, shapes) = delay_after(&ctx, time, |ui| {
                let response = egui_spinner(ui);
                node = Some(response.rect);
            });
            let shapes: Vec<_> = shapes.into_iter().map(|clipped| clipped.shape).collect();
            (delay, shapes, node)
        };
        for time in [0.25, 1.7, 3.3] {
            let (plain_delay, plain, plain_rect) = draw(false, time);
            let (stepped_delay, stepped, stepped_rect) = draw(true, time);
            assert_eq!(plain_delay, Duration::ZERO);
            assert!(is_a_step(stepped_delay), "{stepped_delay:?}");
            assert_eq!(plain_rect, stepped_rect);
            assert_eq!(format!("{plain:?}"), format!("{stepped:?}"));
        }
        // And it still turns between steps.
        assert_ne!(
            format!("{:?}", draw(true, 1.0).1),
            format!("{:?}", draw(true, 1.1).1)
        );
    }
}
