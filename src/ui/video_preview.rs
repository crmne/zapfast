//! A playing video over the whole window, with the bubble's controls.

use egui::{Color32, Rect, Sense, vec2};

use super::conversation::{VideoControls, video_controls};
use crate::app::App;
use crate::model::Action;
use crate::theme::{self, Icon};

/// Room left around the picture, in points.
const MARGIN: f32 = 28.0;

pub fn show(app: &mut App, ctx: &egui::Context) {
    if !app.video_expanded {
        return;
    }
    let Some((message, path)) = app
        .video
        .message()
        .map(str::to_owned)
        .zip(app.video.path().map(std::path::Path::to_owned))
    else {
        return;
    };
    let Some(status) = app.video.status(&message) else {
        return;
    };
    // Its bubble may be scrolled away, or under this; the video is on screen.
    app.video.saw(&message);
    let screen = ctx.content_rect();
    let mut actions = Vec::new();
    egui::Area::new(egui::Id::new("video-preview"))
        .order(egui::Order::Foreground)
        .fixed_pos(screen.min)
        .show(ctx, |ui| {
            let (rect, backdrop) = ui.allocate_exact_size(screen.size(), Sense::click());
            ui.painter()
                .rect_filled(rect, 0.0, Color32::from_black_alpha(238));
            let room = rect.shrink(MARGIN);
            let shape = status
                .frame
                .as_ref()
                .map_or(vec2(16.0, 9.0), |frame| frame.size_vec2());
            let picture = fitted(shape, room);
            let video = ui
                .interact(picture, ui.id().with("picture"), Sense::click())
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            match &status.frame {
                Some(frame) => {
                    ui.painter().image(
                        frame.id(),
                        picture,
                        Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                        Color32::WHITE,
                    );
                }
                None => {
                    let disc = Rect::from_center_size(picture.center(), egui::Vec2::splat(48.0));
                    theme::paint_spinner(ui, disc, 28.0, Color32::WHITE);
                }
            }
            video_controls(
                ui,
                &VideoControls {
                    player: &app.video,
                    locale: app.locale,
                    accent: app.palette.accent,
                    expanded: true,
                },
                &message,
                &path,
                picture,
                &status,
                &mut actions,
            );
            let close = Rect::from_center_size(
                rect.right_top() + vec2(-MARGIN, MARGIN),
                egui::Vec2::splat(32.0),
            );
            ui.painter()
                .circle_filled(close.center(), 16.0, Color32::from_black_alpha(150));
            theme::paint_icon(ui, Icon::X, close, 18.0, Color32::WHITE);
            let closed = ui
                .interact(close, ui.id().with("close"), Sense::click())
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text(
                    crate::i18n::gettext(app.locale, "Back to the message (Esc)").as_ref(),
                )
                .clicked();
            if closed || video.double_clicked() || backdrop.clicked() {
                actions.push(Action::CollapseVideo);
            } else if video.clicked() {
                actions.push(Action::PlayVideo {
                    message: message.clone(),
                    path: path.clone(),
                });
            }
        });
    app.actions.extend(actions);
}

/// The largest rectangle of `shape`'s proportions centred in `room`.
pub(super) fn fitted(shape: egui::Vec2, room: Rect) -> Rect {
    let scale = (room.width() / shape.x.max(1.0)).min(room.height() / shape.y.max(1.0));
    Rect::from_center_size(room.center(), shape * scale)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_picture_fills_the_room_in_one_direction() {
        let room = Rect::from_min_size(egui::pos2(10.0, 10.0), vec2(1000.0, 500.0));
        let wide = fitted(vec2(1920.0, 1080.0), room);
        assert!((wide.height() - 500.0).abs() < 0.01);
        assert!((wide.width() - 500.0 * 16.0 / 9.0).abs() < 0.01);
        assert_eq!(wide.center(), room.center());
        let tall = fitted(vec2(100.0, 1000.0), room);
        assert!((tall.height() - 500.0).abs() < 0.01 && tall.width() < 60.0);
    }
}
