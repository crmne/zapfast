//! "Message info": who has received, read, or played one of our messages.

use egui::{Align, CornerRadius, Frame, Layout, Margin, Sense, vec2};

use super::widgets;
use crate::app::App;
use crate::i18n::{gettext, ngettext};
use crate::model::{Action, Delivery, Message, MessageReceipts, Recipient};
use crate::theme::{self, Icon, Palette};

pub fn show(app: &mut App, ui: &mut egui::Ui, chat: &str, id: &str) {
    let palette = app.palette;
    let locale = app.locale;
    ui.horizontal(|ui| {
        theme::text(
            ui,
            gettext(locale, "Message info"),
            theme::semibold(18.0),
            palette.text,
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if theme::icon_button(
                ui,
                Icon::X,
                16.0,
                palette.secondary,
                palette.text,
                &gettext(locale, "Close"),
            )
            .clicked()
            {
                app.actions.push(Action::CloseDialog);
            }
        });
    });
    let Some(message) = app
        .conversations
        .get(chat)
        .and_then(|c| c.message(id))
        .cloned()
    else {
        widgets::rich_text(
            ui,
            &gettext(locale, "This message is no longer available."),
            theme::regular(14.0),
            palette.secondary,
        );
        return;
    };
    ui.add_space(8.0);
    preview(ui, &palette, &message);
    if crate::model::ChatKind::from_id(chat) != crate::model::ChatKind::Group {
        direct(app, ui, &message);
        return;
    }
    let Some(receipts) = app
        .message_receipts
        .clone()
        .filter(|receipts| receipts.chat == chat && receipts.message == id)
    else {
        widgets::rich_text(
            ui,
            &gettext(locale, "Loading…"),
            theme::regular(13.0),
            palette.secondary,
        );
        return;
    };
    group(app, ui, &receipts);
}

/// The message as its bubble shows it, shortened to a few lines.
fn preview(ui: &mut egui::Ui, palette: &Palette, message: &Message) {
    let text = widgets::line(
        ui,
        &message.content.summary(),
        theme::regular(14.0),
        palette.text,
        (ui.available_width() * 0.85 - 24.0).max(1.0),
        4,
    );
    // Our bubbles sit on the right, as wide as their text.
    let width = text.size().x.max(90.0);
    ui.allocate_ui_with_layout(
        vec2(ui.available_width(), 0.0),
        Layout::right_to_left(Align::Min),
        |ui| {
            bubble(ui, palette, message, text, width);
        },
    );
}

fn bubble(
    ui: &mut egui::Ui,
    palette: &Palette,
    message: &Message,
    text: widgets::Line,
    width: f32,
) {
    Frame::new()
        .fill(palette.bubble_out)
        .corner_radius(CornerRadius::same(theme::RADIUS))
        .inner_margin(Margin::symmetric(12, 8))
        .show(ui, |ui| {
            let clock = ui.painter().layout_no_wrap(
                crate::util::clock(message.timestamp),
                theme::regular(11.5),
                palette.secondary,
            );
            let (rect, _) =
                ui.allocate_exact_size(vec2(width, text.size().y + 18.0), Sense::hover());
            text.paint(ui, rect.min, palette.text);
            let ticks = egui::Rect::from_min_size(
                egui::pos2(rect.right() - 16.0, rect.bottom() - 16.0),
                vec2(16.0, 16.0),
            );
            widgets::ticks(ui, palette, ticks, message.status);
            ui.painter().galley(
                egui::pos2(
                    ticks.left() - 4.0 - clock.size().x,
                    ticks.center().y - clock.size().y / 2.0,
                ),
                clock,
                palette.secondary,
            );
        });
}

/// A direct chat's single recipient: the times its ticks turned.
fn direct(app: &App, ui: &mut egui::Ui, message: &Message) {
    let palette = app.palette;
    let locale = app.locale;
    let read = matches!(message.status, Delivery::Read | Delivery::Played);
    let delivered = read || message.status == Delivery::Delivered;
    let label = if message.status == Delivery::Played {
        gettext(locale, "Played")
    } else {
        gettext(locale, "Read")
    };
    let stages = [
        (label, read, message.read_at, palette.read),
        (
            gettext(locale, "Delivered"),
            delivered,
            message.delivered_at,
            palette.secondary,
        ),
    ];
    for (label, reached, at, color) in stages {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
            theme::paint_icon(ui, Icon::CheckCheck, rect, 18.0, color);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                theme::text(ui, label, theme::medium(14.0), palette.text);
                let when = match (reached, at) {
                    (true, Some(at)) => crate::util::moment_stamp(locale, at),
                    (true, None) => gettext(locale, "Time not recorded").into_owned(),
                    (false, _) => "—".to_owned(),
                };
                theme::text(ui, when, theme::regular(12.0), palette.secondary);
            });
        });
    }
}

fn group(app: &mut App, ui: &mut egui::Ui, receipts: &MessageReceipts) {
    let palette = app.palette;
    let locale = app.locale;
    let known = receipts.audience_known();
    // Receipts were not kept before this dialog existed, and a message sent
    // before then has no saved audience. Say so instead of implying that
    // nobody has read it.
    if receipts.recipients.is_empty() {
        ui.add_space(8.0);
        note(
            ui,
            &gettext(
                locale,
                "No receipts were recorded for this message. Details are available for messages sent from now on.",
            ),
            &palette,
        );
        return;
    }
    if !known {
        ui.add_space(8.0);
        note(
            ui,
            &gettext(
                locale,
                "Some receipts for this message were not recorded. Details are complete for messages sent from now on.",
            ),
            &palette,
        );
    }
    let height = (ui.ctx().content_rect().height() - 260.0).clamp(120.0, 460.0);
    egui::ScrollArea::vertical()
        .id_salt(("message-info", &receipts.chat, &receipts.message))
        .max_height(height)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            let played = receipts.played();
            if !played.is_empty() {
                section(
                    app,
                    ui,
                    &gettext(locale, "Played by"),
                    palette.read,
                    &played,
                    |recipient| recipient.played_at,
                );
            }
            section(
                app,
                ui,
                &gettext(locale, "Read by"),
                palette.read,
                &receipts.read(),
                |recipient| recipient.read_at,
            );
            section(
                app,
                ui,
                &gettext(locale, "Delivered to"),
                palette.secondary,
                &receipts.delivered(),
                |recipient| recipient.delivered_at,
            );
            let remaining = receipts.remaining();
            if remaining > 0 {
                ui.add_space(12.0);
                theme::text(
                    ui,
                    ngettext(locale, "{} remaining", "{} remaining", remaining as u32)
                        .replace("{}", &remaining.to_string()),
                    theme::regular(13.0),
                    palette.secondary,
                );
            }
        });
}

fn section(
    app: &mut App,
    ui: &mut egui::Ui,
    title: &str,
    color: egui::Color32,
    recipients: &[&Recipient],
    at: impl Fn(&Recipient) -> Option<i64>,
) {
    let palette = app.palette;
    ui.add_space(12.0);
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
        theme::paint_icon(ui, Icon::CheckCheck, rect, 16.0, color);
        theme::text(ui, title, theme::semibold(14.0), palette.text);
    });
    if recipients.is_empty() {
        ui.add_space(2.0);
        theme::text(
            ui,
            gettext(app.locale, "No one yet"),
            theme::regular(12.5),
            palette.secondary,
        );
        return;
    }
    for recipient in recipients {
        ui.add_space(6.0);
        let name = app.display_name(&recipient.id);
        let picture = app.avatar(&recipient.id);
        ui.horizontal(|ui| {
            widgets::avatar(ui, &palette, &name, &recipient.id, 34.0, picture.as_deref());
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                widgets::rich_text(ui, &name, theme::regular(14.0), palette.text);
                if let Some(at) = at(recipient) {
                    theme::text(
                        ui,
                        crate::util::moment_stamp(app.locale, at),
                        theme::regular(12.0),
                        palette.secondary,
                    );
                }
            });
        });
    }
}

/// Secondary text wrapped over as many lines as it needs.
fn note(ui: &mut egui::Ui, text: &str, palette: &Palette) {
    let width = ui.available_width().max(1.0);
    let line = widgets::line(
        ui,
        text,
        theme::regular(13.0),
        palette.secondary,
        width,
        usize::MAX,
    );
    let (rect, _) = ui.allocate_exact_size(vec2(width, line.size().y), Sense::hover());
    line.paint(ui, rect.min, palette.secondary);
}
