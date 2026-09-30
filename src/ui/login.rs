//! Phone linking with a QR code or pairing code.

use egui::{Align, CornerRadius, Frame, Layout, Margin, Stroke, Vec2};

use crate::app::App;
use crate::backend::LinkStatus;
use crate::model::{Action, Dialog};
use crate::qr::Qr;
use crate::theme::{self, Icon};

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    card(app, ui, "login", "A native WhatsApp client.", body);
}

/// The centred card of the linking and lock screens: the logo, the name,
/// a line under it, and `body`.
pub(super) fn card(
    app: &mut App,
    ui: &mut egui::Ui,
    salt: &str,
    tagline: &str,
    body: impl FnOnce(&mut App, &mut egui::Ui),
) {
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
            let height_id = ui.id().with(format!("{salt}-card-height"));
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
                    theme::text(ui, tagline, theme::regular(14.5), palette.secondary);
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
            let key_lost = archive_key_lost(&message);
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
            ui.horizontal(|ui| {
                if theme::pill_button(ui, &palette, "Try again", true).clicked() {
                    app.actions.push(Action::Reconnect);
                }
                if key_lost && theme::pill_button(ui, &palette, "Start over…", false).clicked() {
                    app.actions
                        .push(Action::ShowDialog(crate::model::Dialog::ConfirmStartOver));
                }
            });
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

pub(super) fn busy(ui: &mut egui::Ui, color: egui::Color32, label: &str) {
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

/// Whether the archive's key is gone for good, rather than the keyring being
/// locked or unavailable, which "Try again" can fix.
fn archive_key_lost(message: &str) -> bool {
    message.contains("OS keyring key is missing")
        || message.contains("archive key in the OS keyring is invalid")
        || message.contains("could not be unlocked with its OS keyring key")
}

/// The sign-in card for an account that is not the linked WhatsApp device.
/// Values typed here live only in memory until the account is signed in.
pub(super) fn telegram(app: &mut App, ui: &mut egui::Ui) {
    let Some((account, state)) = app.sign_in() else {
        return;
    };
    let palette = app.palette;
    let card_width = 460.0_f32.min(ui.available_width() - 24.0).max(0.0);
    ui.vertical_centered(|ui| {
        ui.add_space(60.0);
        Frame::new()
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
            .show(ui, |ui| {
                ui.set_width((card_width - 64.0).max(0.0));
                ui.spacing_mut().item_spacing.y = 8.0;
                theme::text(ui, "ZapFast", theme::bold(28.0), palette.text);
                theme::text(
                    ui,
                    "Sign in to Telegram.",
                    theme::regular(14.5),
                    palette.secondary,
                );
                ui.add_space(16.0);
                match state {
                crate::account::AuthState::SignedOut
                | crate::account::AuthState::Failed { .. } => {
                    if let crate::account::AuthState::Failed { reason } = &state {
                        theme::paragraph(ui, reason, theme::regular(13.0), palette.danger);
                        ui.add_space(4.0);
                    }
                    theme::paragraph(
                        ui,
                        "Enter the phone number of your Telegram account. The sign-in code comes to your other devices.",
                        theme::regular(13.0),
                        palette.secondary,
                    );
                    let submit =
                        field(ui, &palette, "telegram-phone", &mut app.telegram_phone, false);
                    let ready = app
                        .telegram_phone
                        .chars()
                        .filter(char::is_ascii_digit)
                        .count()
                        >= 7;
                    let mut pressed = false;
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        pressed = ui
                            .add_enabled_ui(ready, |ui| {
                                theme::pill_button(ui, &palette, "Send code", true)
                            })
                            .inner
                            .clicked();
                    });
                    if (submit && ready) || pressed {
                        app.telegram_code.clear();
                        app.telegram_password.clear();
                        app.actions.push(Action::Login {
                            account,
                            step: crate::backend::LoginStep::TelegramPhone(
                                app.telegram_phone.trim().to_owned(),
                            ),
                        });
                    }
                    ui.add_space(4.0);
                    theme::paragraph(
                        ui,
                        "Live sign-in also needs ZAPFAST_TELEGRAM_API_ID and ZAPFAST_TELEGRAM_API_HASH from my.telegram.org.",
                        theme::regular(12.0),
                        palette.secondary,
                    );
                }
                crate::account::AuthState::TelegramCode { .. } => {
                    theme::paragraph(
                        ui,
                        "Enter the code Telegram sent to your other devices.",
                        theme::regular(13.0),
                        palette.secondary,
                    );
                    let submit =
                        field(ui, &palette, "telegram-code", &mut app.telegram_code, false);
                    let ready = !app.telegram_code.trim().is_empty();
                    let mut pressed = false;
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        pressed = ui
                            .add_enabled_ui(ready, |ui| {
                                theme::pill_button(ui, &palette, "Sign in", true)
                            })
                            .inner
                            .clicked();
                    });
                    if (submit && ready) || pressed {
                        app.actions.push(Action::Login {
                            account,
                            step: crate::backend::LoginStep::TelegramCode(
                                app.telegram_code.trim().to_owned(),
                            ),
                        });
                    }
                }
                crate::account::AuthState::TelegramPassword { .. } => {
                    theme::paragraph(
                        ui,
                        "This account has a two-step password. Enter it to finish signing in.",
                        theme::regular(13.0),
                        palette.secondary,
                    );
                    let submit = field(
                        ui,
                        &palette,
                        "telegram-password",
                        &mut app.telegram_password,
                        true,
                    );
                    let ready = !app.telegram_password.is_empty();
                    let mut pressed = false;
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        pressed = ui
                            .add_enabled_ui(ready, |ui| {
                                theme::pill_button(ui, &palette, "Sign in", true)
                            })
                            .inner
                            .clicked();
                    });
                    if (submit && ready) || pressed {
                        app.actions.push(Action::Login {
                            account,
                            step: crate::backend::LoginStep::TelegramPassword(
                                app.telegram_password.clone(),
                            ),
                        });
                    }
                }
                crate::account::AuthState::WhatsappLink
                | crate::account::AuthState::Ready
                | crate::account::AuthState::XAuthorize { .. } => {}
                }
            });
    });
}

/// The sign-in card for a Matrix account. The homeserver, user, and password
/// go straight to the account's adapter and are never stored here.
pub(super) fn matrix(app: &mut App, ui: &mut egui::Ui) {
    let Some((account, state)) = app.sign_in() else {
        return;
    };
    let palette = app.palette;
    let card_width = 460.0_f32.min(ui.available_width() - 24.0).max(0.0);
    ui.vertical_centered(|ui| {
        ui.add_space(60.0);
        Frame::new()
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
            .show(ui, |ui| {
                ui.set_width((card_width - 64.0).max(0.0));
                ui.spacing_mut().item_spacing.y = 8.0;
                theme::text(ui, "ZapFast", theme::bold(28.0), palette.text);
                theme::text(
                    ui,
                    "Sign in to Matrix.",
                    theme::regular(14.5),
                    palette.secondary,
                );
                ui.add_space(16.0);
                if let crate::account::AuthState::Failed { reason } = &state {
                    theme::paragraph(ui, reason, theme::regular(13.0), palette.danger);
                    ui.add_space(4.0);
                }
                theme::paragraph(
                    ui,
                    "Sign in with an account on a homeserver. Messages are end-to-end encrypted; verify this session after signing in.",
                    theme::regular(13.0),
                    palette.secondary,
                );
                field(
                    ui,
                    &palette,
                    "matrix-homeserver",
                    &mut app.matrix_homeserver,
                    false,
                );
                field(ui, &palette, "matrix-user", &mut app.matrix_user, false);
                let submit = field(
                    ui,
                    &palette,
                    "matrix-password",
                    &mut app.matrix_password,
                    true,
                );
                let ready = !app.matrix_homeserver.trim().is_empty()
                    && !app.matrix_user.trim().is_empty()
                    && !app.matrix_password.is_empty();
                let mut pressed = false;
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    pressed = ui
                        .add_enabled_ui(ready, |ui| {
                            theme::pill_button(ui, &palette, "Sign in", true)
                        })
                        .inner
                        .clicked();
                });
                if (submit && ready) || pressed {
                    let step = crate::backend::LoginStep::MatrixPassword {
                        homeserver: app.matrix_homeserver.trim().to_owned(),
                        user: app.matrix_user.trim().to_owned(),
                        password: app.matrix_password.clone(),
                    };
                    app.matrix_password.clear();
                    app.actions.push(Action::Login { account, step });
                }
            });
    });
}

/// The sign-in card for a Slack account. The tokens go straight to the OS
/// keyring after the adapter has checked them; they are never stored in the
/// settings file.
pub(super) fn slack(app: &mut App, ui: &mut egui::Ui) {
    let Some((account, state)) = app.sign_in() else {
        return;
    };
    let palette = app.palette;
    let card_width = 460.0_f32.min(ui.available_width() - 24.0).max(0.0);
    ui.vertical_centered(|ui| {
        ui.add_space(60.0);
        Frame::new()
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
            .show(ui, |ui| {
                ui.set_width((card_width - 64.0).max(0.0));
                ui.spacing_mut().item_spacing.y = 8.0;
                theme::text(ui, "ZapFast", theme::bold(28.0), palette.text);
                theme::text(
                    ui,
                    "Connect Slack.",
                    theme::regular(14.5),
                    palette.secondary,
                );
                ui.add_space(16.0);
                if let crate::account::AuthState::Failed { reason } = &state {
                    theme::paragraph(ui, reason, theme::regular(13.0), palette.danger);
                    ui.add_space(4.0);
                }
                theme::paragraph(
                    ui,
                    "Both tokens come from api.slack.com: the app-level token (xapp-) for Socket Mode, and the bot token (xoxb-) for the messages.",
                    theme::regular(13.0),
                    palette.secondary,
                );
                field(ui, &palette, "slack-app-token", &mut app.slack_app_token, true);
                let submit = field(
                    ui,
                    &palette,
                    "slack-bot-token",
                    &mut app.slack_bot_token,
                    true,
                );
                let ready = !app.slack_app_token.trim().is_empty()
                    && !app.slack_bot_token.trim().is_empty();
                let mut pressed = false;
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    pressed = ui
                        .add_enabled_ui(ready, |ui| {
                            theme::pill_button(ui, &palette, "Connect", true)
                        })
                        .inner
                        .clicked();
                });
                if (submit && ready) || pressed {
                    let step = crate::backend::LoginStep::SlackTokens {
                        app: app.slack_app_token.trim().to_owned(),
                        bot: app.slack_bot_token.trim().to_owned(),
                    };
                    app.slack_app_token.clear();
                    app.slack_bot_token.clear();
                    app.actions.push(Action::Login { account, step });
                }
            });
    });
}

/// The sign-in card for a Zulip account. The server and email are ordinary
/// text; the API key is masked and goes to the OS keyring once it works.
pub(super) fn zulip(app: &mut App, ui: &mut egui::Ui) {
    let Some((account, state)) = app.sign_in() else {
        return;
    };
    let palette = app.palette;
    let card_width = 460.0_f32.min(ui.available_width() - 24.0).max(0.0);
    ui.vertical_centered(|ui| {
        ui.add_space(60.0);
        Frame::new()
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
            .show(ui, |ui| {
                ui.set_width((card_width - 64.0).max(0.0));
                ui.spacing_mut().item_spacing.y = 8.0;
                theme::text(ui, "ZapFast", theme::bold(28.0), palette.text);
                theme::text(ui, "Connect Zulip.", theme::regular(14.5), palette.secondary);
                ui.add_space(16.0);
                if let crate::account::AuthState::Failed { reason } = &state {
                    theme::paragraph(ui, reason, theme::regular(13.0), palette.danger);
                    ui.add_space(4.0);
                }
                theme::paragraph(
                    ui,
                    "The server is your Zulip URL. The email and API key are the ones under your Zulip account settings; the key stays in the OS keyring.",
                    theme::regular(13.0),
                    palette.secondary,
                );
                field(ui, &palette, "zulip-server", &mut app.zulip_server, false);
                field(ui, &palette, "zulip-email", &mut app.zulip_email, false);
                let submit = field(ui, &palette, "zulip-key", &mut app.zulip_key, true);
                let ready = !app.zulip_server.trim().is_empty()
                    && !app.zulip_email.trim().is_empty()
                    && !app.zulip_key.trim().is_empty();
                let mut pressed = false;
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    pressed = ui
                        .add_enabled_ui(ready, |ui| {
                            theme::pill_button(ui, &palette, "Connect", true)
                        })
                        .inner
                        .clicked();
                });
                if (submit && ready) || pressed {
                    let step = crate::backend::LoginStep::ZulipCredentials {
                        server: app.zulip_server.trim().to_owned(),
                        email: app.zulip_email.trim().to_owned(),
                        api_key: app.zulip_key.trim().to_owned(),
                    };
                    app.zulip_key.clear();
                    app.actions.push(Action::Login { account, step });
                }
            });
    });
}

/// The Discord bot sign-in card. A bot token, never a personal account.
pub(super) fn discord(app: &mut App, ui: &mut egui::Ui) {
    let Some((account, state)) = app.sign_in() else {
        return;
    };
    let palette = app.palette;
    let card_width = 460.0_f32.min(ui.available_width() - 24.0).max(0.0);
    ui.vertical_centered(|ui| {
        ui.add_space(60.0);
        Frame::new()
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
            .show(ui, |ui| {
                ui.set_width((card_width - 64.0).max(0.0));
                ui.spacing_mut().item_spacing.y = 8.0;
                theme::text(ui, "ZapFast", theme::bold(28.0), palette.text);
                theme::text(
                    ui,
                    "Connect a Discord bot.",
                    theme::regular(14.5),
                    palette.secondary,
                );
                ui.add_space(16.0);
                if let crate::account::AuthState::Failed { reason } = &state {
                    theme::paragraph(ui, reason, theme::regular(13.0), palette.danger);
                    ui.add_space(4.0);
                }
                theme::paragraph(
                    ui,
                    "This connects a bot, not a personal account. Create an application at discord.com/developers, turn on the Message Content intent, and invite its bot to your servers. The token stays in the OS keyring.",
                    theme::regular(13.0),
                    palette.secondary,
                );
                let submit = field(ui, &palette, "discord-token", &mut app.discord_token, true);
                let ready = !app.discord_token.trim().is_empty();
                let mut pressed = false;
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    pressed = ui
                        .add_enabled_ui(ready, |ui| {
                            theme::pill_button(ui, &palette, "Connect", true)
                        })
                        .inner
                        .clicked();
                });
                if (submit && ready) || pressed {
                    let step = crate::backend::LoginStep::DiscordToken {
                        token: app.discord_token.trim().to_owned(),
                    };
                    app.discord_token.clear();
                    app.actions.push(Action::Login { account, step });
                }
            });
    });
}

/// The X sign-in card. Sign-in happens in the browser through OAuth 2.0 PKCE
/// against the user's own developer app.
pub(super) fn x(app: &mut App, ui: &mut egui::Ui) {
    let Some((account, state)) = app.sign_in() else {
        return;
    };
    let palette = app.palette;
    let card_width = 460.0_f32.min(ui.available_width() - 24.0).max(0.0);
    ui.vertical_centered(|ui| {
        ui.add_space(60.0);
        Frame::new()
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
            .show(ui, |ui| {
                ui.set_width((card_width - 64.0).max(0.0));
                ui.spacing_mut().item_spacing.y = 8.0;
                theme::text(ui, "ZapFast", theme::bold(28.0), palette.text);
                theme::text(ui, "Connect an X account.", theme::regular(14.5), palette.secondary);
                ui.add_space(16.0);
                if let crate::account::AuthState::Failed { reason } = &state {
                    theme::paragraph(ui, reason, theme::regular(13.0), palette.danger);
                    ui.add_space(4.0);
                }
                theme::paragraph(
                    ui,
                    "Sign-in opens X in your browser. X only returns direct messages from the last 30 days, and only legacy unencrypted messages; encrypted X Chat does not appear.",
                    theme::regular(13.0),
                    palette.secondary,
                );
                match &state {
                    crate::account::AuthState::XAuthorize { url } => {
                        theme::paragraph(
                            ui,
                            "Approve this account in your browser, then come back here.",
                            theme::regular(13.0),
                            palette.secondary,
                        );
                        theme::paragraph(ui, url, theme::regular(12.0), palette.secondary);
                        let mut open = false;
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            open = theme::pill_button(ui, &palette, "Open in browser", true)
                                .clicked();
                        });
                        if open {
                            ui.ctx().open_url(egui::OpenUrl::new_tab(url.clone()));
                        }
                    }
                    _ => {
                        theme::paragraph(
                            ui,
                            "This needs your own developer app with the redirect http://127.0.0.1:8787/callback registered. The client id comes from ZAPFAST_X_CLIENT_ID.",
                            theme::regular(13.0),
                            palette.secondary,
                        );
                        let mut pressed = false;
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            pressed = theme::pill_button(ui, &palette, "Connect", true).clicked();
                        });
                        if pressed {
                            app.actions.push(Action::Login {
                                account,
                                step: crate::backend::LoginStep::XConnect,
                            });
                        }
                    }
                }
            });
    });
}

/// The emoji comparison card for Matrix session verification. It stays up
/// until the other device is confirmed or the request ends.
pub(super) fn verification(app: &mut App, ui: &mut egui::Ui) {
    let account = app.active_account;
    let Some(prompt) = app.verification.get(&account).cloned() else {
        return;
    };
    let palette = app.palette;
    let card_width = 460.0_f32.min(ui.available_width() - 24.0).max(0.0);
    let mut action = None;
    ui.vertical_centered(|ui| {
        ui.add_space(60.0);
        Frame::new()
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
            .show(ui, |ui| {
                ui.set_width((card_width - 64.0).max(0.0));
                ui.spacing_mut().item_spacing.y = 8.0;
                theme::text(ui, "Compare these emoji", theme::bold(22.0), palette.text);
                theme::paragraph(
                    ui,
                    "They must match, in order, on the other device.",
                    theme::regular(13.0),
                    palette.secondary,
                );
                ui.add_space(12.0);
                ui.horizontal_wrapped(|ui| {
                    for (symbol, name) in &prompt.emojis {
                        ui.vertical(|ui| {
                            theme::text(ui, symbol, theme::bold(28.0), palette.text);
                            theme::text(ui, name, theme::regular(11.0), palette.secondary);
                        });
                        ui.add_space(6.0);
                    }
                });
                if let Some((first, second, third)) = prompt.decimals {
                    theme::paragraph(
                        ui,
                        format!("{first} {second} {third}"),
                        theme::regular(16.0),
                        palette.text,
                    );
                }
                ui.add_space(12.0);
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    if theme::pill_button(ui, &palette, "They match", true).clicked() {
                        action = Some(crate::backend::VerifyAction::Confirm);
                    }
                    if theme::pill_button(ui, &palette, "They do not match", false).clicked() {
                        action = Some(crate::backend::VerifyAction::Mismatch);
                    }
                    if theme::pill_button(ui, &palette, "Cancel", false).clicked() {
                        action = Some(crate::backend::VerifyAction::Cancel);
                    }
                });
            });
    });
    if let Some(action) = action {
        app.actions.push(Action::Verification { account, action });
    }
}

/// One text field of the sign-in card, with Enter as a submit key.
fn field(
    ui: &mut egui::Ui,
    palette: &theme::Palette,
    salt: &str,
    value: &mut String,
    password: bool,
) -> bool {
    let id = egui::Id::new(salt);
    let submit = ui.memory(|memory| memory.has_focus(id))
        && ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
    let field = Frame::new()
        .fill(palette.surface)
        .corner_radius(CornerRadius::same(theme::RADIUS))
        .inner_margin(Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.add(
                egui::TextEdit::singleline(value)
                    .id(id)
                    .password(password)
                    .font(theme::regular(16.0))
                    .text_color(palette.text)
                    .frame(egui::Frame::NONE)
                    .desired_width(f32::INFINITY),
            )
        });
    theme::focus_outline(ui, id, field.inner.rect, f32::from(theme::RADIUS));
    submit
}

#[cfg(test)]
mod start_over_tests {
    #[test]
    fn only_a_lost_key_offers_to_start_over() {
        for lost in [
            "The archive is encrypted but its OS keyring key is missing. Restore the original keyring; the archive has not been changed",
            "The archive key in the OS keyring is invalid",
            "The archive could not be unlocked with its OS keyring key: file is not a database",
        ] {
            assert!(super::archive_key_lost(lost), "{lost}");
        }
        for recoverable in [
            "Unlock your OS keyring and restart ZapFast",
            "The OS keyring could not open ZapFast's archive key",
        ] {
            assert!(!super::archive_key_lost(recoverable), "{recoverable}");
        }
    }
}
