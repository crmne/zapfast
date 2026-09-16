//! Phone linking with a QR code or pairing code.

use egui::{Align, CornerRadius, Frame, Layout, Margin, Stroke, Vec2};

use crate::app::App;
use crate::backend::LinkStatus;
use crate::model::{Action, Dialog};
use crate::qr::Qr;
use crate::theme::{self, Icon};

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    egui::CentralPanel::default()
        .frame(Frame::new().fill(palette.window))
        .show(ui, |ui| {
            let rect = ui.max_rect();
            let top = theme::blend(palette.window, palette.accent, 0.10);
            super::widgets::paint_vertical_gradient(ui, rect, top, palette.window);
            let card_width = (460.0_f32.min(rect.width() - 24.0)).max(0.0);
            // Center the card using its previous height. Its content determines
            // the next frame's height.
            let height_id = ui.id().with("login-card-height");
            let known_height = ui
                .ctx()
                .data(|data| data.get_temp::<f32>(height_id))
                .unwrap_or(560.0);
            let card_height = (known_height.min(rect.height() - 24.0)).max(0.0);
            let card =
                egui::Rect::from_center_size(rect.center(), Vec2::new(card_width, card_height));
            let mut card_ui = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(card)
                    .layout(Layout::top_down(Align::Center)),
            );
            let shown = Frame::new()
                .fill(palette.panel)
                .stroke(Stroke::new(1.0, palette.outline))
                .corner_radius(CornerRadius::same(theme::RADIUS + 8))
                .inner_margin(Margin::same(32))
                .shadow(egui::epaint::Shadow {
                    offset: [0, 16],
                    blur: 48,
                    spread: 0,
                    color: palette.shadow,
                })
                .show(&mut card_ui, |ui| {
                    ui.set_width((card_width - 64.0).max(0.0));
                    ui.spacing_mut().item_spacing.y = 8.0;
                    let (logo, _) = ui.allocate_exact_size(Vec2::splat(64.0), egui::Sense::hover());
                    theme::logo(
                        ui,
                        logo.center(),
                        64.0,
                        palette.accent,
                        egui::Color32::WHITE,
                    );
                    ui.add_space(4.0);
                    theme::text(ui, "ZapFast", theme::bold(28.0), palette.text);
                    theme::text(
                        ui,
                        "A native WhatsApp client.",
                        theme::regular(14.5),
                        palette.secondary,
                    );
                    ui.add_space(16.0);
                    body(app, ui);
                });
            let height = shown.response.rect.height();
            if (height - known_height).abs() > 2.0 {
                ui.ctx()
                    .data_mut(|data| data.insert_temp(height_id, height.round()));
                ui.ctx().request_repaint();
            }
        });
}

fn body(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    match app.link.clone() {
        LinkStatus::Starting | LinkStatus::Connecting => {
            busy(ui, palette.accent, "Connecting to WhatsApp…");
        }
        LinkStatus::Connected | LinkStatus::Disconnected { .. } => {
            busy(ui, palette.accent, "Linked. Waiting for your chats…");
        }
        LinkStatus::LoggedOut => {
            theme::icon(ui, Icon::Smartphone, 28.0, palette.warning);
            theme::paragraph(
                ui,
                "This computer was unlinked from your phone. Requesting a new code.",
                theme::regular(14.0),
                palette.text,
            );
            ui.add_space(8.0);
            busy(ui, palette.accent, "Requesting a new code…");
        }
        LinkStatus::Failed(message) => {
            theme::icon(ui, Icon::CircleAlert, 28.0, palette.danger);
            ui.add(
                egui::Label::new(
                    egui::RichText::new(message)
                        .font(theme::regular(13.5))
                        .color(palette.danger),
                )
                .wrap(),
            );
            ui.add_space(12.0);
            if theme::pill_button(ui, &palette, "Try again", true).clicked() {
                app.actions.push(Action::Reconnect);
            }
        }
        LinkStatus::Unlinked {
            qr,
            pair_code,
            pairing_phone,
        } => {
            if let Some(code) = pair_code {
                pair_code_view(app, ui, &code, pairing_phone.as_deref());
            } else if let Some(phone) = pairing_phone {
                busy(
                    ui,
                    palette.accent,
                    &format!("Requesting a code for +{phone}…"),
                );
            } else if let Some(qr) = qr {
                qr_view(app, ui, &qr);
            } else {
                busy(ui, palette.accent, "Waiting for a code from WhatsApp…");
            }
        }
    }
    ui.add_space(18.0);
    theme::paragraph(
        ui,
        "Unofficial client. Using it may be against WhatsApp's terms of service.",
        theme::regular(11.5),
        palette.dim,
    );
}

fn busy(ui: &mut egui::Ui, color: egui::Color32, label: &str) {
    ui.horizontal(|ui| {
        let width = 24.0
            + 8.0
            + ui.painter()
                .layout_no_wrap(label.to_owned(), theme::medium(14.0), color)
                .size()
                .x;
        ui.add_space((ui.available_width() - width).max(0.0) / 2.0);
        theme::spinner(ui, 18.0, color);
        theme::text(ui, label, theme::medium(14.0), ui.visuals().text_color());
    });
}

fn qr_view(app: &mut App, ui: &mut egui::Ui, code: &str) {
    let palette = app.palette;
    theme::text(
        ui,
        "Link this computer",
        theme::semibold(16.0),
        palette.text,
    );
    let side = 260.0;
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(side), egui::Sense::hover());
    match Qr::encode(code) {
        Some(qr) => qr.paint(ui, rect, egui::Color32::BLACK, egui::Color32::WHITE),
        None => {
            ui.painter().rect_filled(rect, 8.0, palette.surface);
            theme::paint_icon(ui, Icon::CircleAlert, rect, 32.0, palette.danger);
        }
    }
    ui.add_space(4.0);
    let steps = [
        "Open WhatsApp on your phone",
        "Tap Menu or Settings, then Linked devices",
        "Tap Link a device and point the phone at this code",
    ];
    for (index, step) in steps.iter().enumerate() {
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            theme::text(
                ui,
                format!("{}.", index + 1),
                theme::semibold(13.0),
                palette.accent,
            );
            theme::text(ui, *step, theme::regular(13.0), palette.secondary);
        });
    }
    ui.add_space(10.0);
    if theme::link(
        ui,
        "Link with phone number instead",
        theme::medium(13.0),
        palette.link,
    )
    .clicked()
    {
        app.actions.push(Action::ShowDialog(Dialog::PairWithPhone));
    }
}

fn pair_code_view(app: &mut App, ui: &mut egui::Ui, code: &str, phone: Option<&str>) {
    let palette = app.palette;
    theme::text(
        ui,
        "Enter this code on your phone",
        theme::semibold(16.0),
        palette.text,
    );
    if let Some(phone) = phone {
        theme::text(
            ui,
            format!("for +{phone}"),
            theme::regular(13.0),
            palette.secondary,
        );
    }
    ui.add_space(8.0);
    let shown = if code.len() == 8 && !code.contains('-') {
        format!("{}-{}", &code[..4], &code[4..])
    } else {
        code.to_owned()
    };
    Frame::new()
        .fill(palette.surface)
        .corner_radius(CornerRadius::same(theme::RADIUS))
        .inner_margin(Margin::symmetric(22, 12))
        .show(ui, |ui| {
            theme::text(ui, &shown, theme::bold(30.0), palette.text);
        });
    ui.add_space(8.0);
    let steps = [
        "Open WhatsApp on your phone",
        "Tap Menu or Settings, then Linked devices",
        "Tap Link a device, then Link with phone number instead",
    ];
    for (index, step) in steps.iter().enumerate() {
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            theme::text(
                ui,
                format!("{}.", index + 1),
                theme::semibold(13.0),
                palette.accent,
            );
            theme::text(ui, *step, theme::regular(13.0), palette.secondary);
        });
    }
    ui.add_space(10.0);
    ui.horizontal(|ui| {
        ui.add_space((ui.available_width() - 200.0).max(0.0) / 2.0);
        if theme::soft_button(ui, &palette, Some(Icon::Copy), "Copy code", false).clicked() {
            app.actions.push(Action::CopyText(code.to_owned()));
        }
    });
}
