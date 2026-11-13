//! The line under a chat's header while a voice message from another chat
//! plays on, as its own bubble with the controls is not on screen: play or
//! pause, how far it has got, and stop. Clicking the line opens the message.

use egui::{Rect, Sense, pos2, vec2};

use crate::app::App;
use crate::audio::State;
use crate::model::Action;
use crate::theme::{self, Icon};
use crate::ui::widgets;

const HEIGHT: f32 = 44.0;
const INSET: f32 = 12.0;
const BUTTON: f32 = 28.0;
/// The line along the bottom that fills as the clip plays.
const PROGRESS: f32 = 2.0;

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let Some((chat, message)) = app.voice_elsewhere() else {
        return;
    };
    let status = app.player.status(&message);
    let palette = app.palette;
    let title = app.voice_chat_title(&chat);
    // Without a chat header, keep the full-width player below any caption
    // buttons at the window's top corners.
    let room = super::caption_room(
        ui.ctx(),
        ui.available_rect_before_wrap(),
        ui.available_width(),
    );
    ui.add_space(room.top);
    let (bar, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), HEIGHT), Sense::click());
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    ui.painter().rect_filled(
        bar,
        0.0,
        if response.hovered() {
            palette.surface_hover
        } else {
            palette.panel
        },
    );
    let playing = status.state == State::Playing;
    let (icon, tooltip) = if playing {
        (Icon::Pause, "Pause")
    } else {
        (Icon::Play, "Play")
    };
    let button_at = |left: f32| {
        Rect::from_center_size(
            pos2(left + BUTTON / 2.0, bar.center().y),
            egui::Vec2::splat(BUTTON),
        )
    };
    let tooltip = crate::i18n::gettext(app.locale, tooltip);
    let hover = palette.accent.gamma_multiply(0.22);
    let mut toggled = false;
    ui.scope_builder(
        egui::UiBuilder::new().max_rect(button_at(bar.left() + INSET)),
        |ui| {
            toggled = theme::circle_button(
                ui,
                icon,
                BUTTON,
                palette.accent.gamma_multiply(0.12),
                hover,
                palette.accent,
                &tooltip,
            )
            .clicked();
        },
    );
    let stop_left = bar.right() - INSET - BUTTON;
    let stop_label = crate::i18n::gettext(app.locale, "Stop");
    let mut stopped = false;
    ui.scope_builder(
        egui::UiBuilder::new().max_rect(button_at(stop_left)),
        |ui| {
            stopped = theme::circle_button(
                ui,
                Icon::X,
                BUTTON,
                egui::Color32::TRANSPARENT,
                palette.surface_hover,
                palette.secondary,
                &stop_label,
            )
            .clicked();
        },
    );
    let clock = crate::util::duration(status.position.as_secs() as u32);
    let clock = widgets::line(
        ui,
        &crate::markup::plain(&clock, &[]),
        theme::regular(12.0),
        palette.secondary,
        60.0,
        1,
    );
    let clock_left = stop_left - 8.0 - clock.size().x;
    clock.paint(
        ui,
        pos2(clock_left, bar.center().y - clock.size().y / 2.0),
        palette.secondary,
    );
    let text_left = bar.left() + INSET + BUTTON + 10.0;
    let label = widgets::line(
        ui,
        &crate::markup::plain(&title, &[]),
        theme::regular(13.0),
        palette.text,
        (clock_left - 8.0 - text_left).max(1.0),
        1,
    );
    label.paint(
        ui,
        pos2(text_left, bar.center().y - label.size().y / 2.0),
        palette.text,
    );
    let fraction = if status.total.is_zero() {
        0.0
    } else {
        (status.position.as_secs_f32() / status.total.as_secs_f32()).clamp(0.0, 1.0)
    };
    ui.painter().rect_filled(
        Rect::from_min_max(
            pos2(bar.left(), bar.bottom() - PROGRESS),
            pos2(bar.left() + bar.width() * fraction, bar.bottom()),
        ),
        0.0,
        palette.accent,
    );
    theme::reveal_focus(&response);
    let described = crate::i18n::gettext(app.locale, "Voice message playing in {chat}")
        .replace("{chat}", &title);
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), &described)
    });
    if toggled {
        app.actions.push(Action::ToggleVoice);
    } else if stopped {
        app.actions.push(Action::StopVoice);
    } else if response.clicked() {
        app.actions.push(Action::OpenMessage { chat, message });
    }
}
