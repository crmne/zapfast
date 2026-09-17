//! Window layout: panels, overlays, keyboard shortcuts.

pub mod chats;
pub mod conversation;
pub mod dialogs;
pub mod keys;
pub mod login;
pub mod picker;
pub mod polls;
pub mod settings;
pub mod update;
pub mod widgets;

use egui::{Align2, CornerRadius, Frame, Margin, Stroke, vec2};

use crate::app::App;
use crate::backend::LinkStatus;
use crate::model::{Action, Page, ToastKind};
use crate::theme::{self, Icon};

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    let ctx = &ctx;
    keys::handle(app, ctx);
    titlebar_strip(app, ui);
    if !app.is_linked() {
        login::show(app, ui);
        dialogs::show(app, ctx);
        update::show(app, ctx);
        toasts(app, ctx);
        return;
    }
    let macos = theme::macos_chrome(ctx);
    if !macos {
        banner(app, ui);
    }
    if app.sidebar_visible {
        chats::show(app, ui);
    }
    let palette = app.palette;
    egui::CentralPanel::default()
        .frame(Frame::new().fill(palette.chat))
        .show(ui, |ui| match app.page {
            Page::Settings => settings::show(app, ui),
            Page::Chats => conversation::show(app, ui),
        });
    update::show(app, ctx);
    picker::show(app, ctx);
    dialogs::show(app, ctx);
    drop_target(app, ctx);
    toasts(app, ctx);
}

/// Shows where dragged files will be sent.
fn drop_target(app: &mut App, ctx: &egui::Context) {
    if !app.dropping {
        return;
    }
    let palette = app.palette;
    let name = app
        .current_chat()
        .map(|chat| app.chat_title(chat))
        .unwrap_or_default();
    egui::Area::new(egui::Id::new("drop-target"))
        .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
        .order(egui::Order::Foreground)
        .interactable(false)
        .show(ctx, |ui| {
            Frame::new()
                .fill(palette.overlay)
                .stroke(Stroke::new(2.0, palette.accent))
                .corner_radius(CornerRadius::same(theme::RADIUS + 4))
                .inner_margin(Margin::symmetric(28, 20))
                .show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        theme::icon(ui, Icon::Paperclip, 28.0, palette.accent);
                        theme::text(
                            ui,
                            format!("Drop to send to {name}"),
                            theme::semibold(15.0),
                            palette.text,
                        );
                    });
                });
        });
}

/// Connection and history-sync banner.
fn banner(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    let update = app.update.clone();
    let (icon, text, color, retry, download) = match &app.link {
        LinkStatus::Connected if app.syncing => (
            Icon::Refresh,
            match app.sync_percent {
                Some(percent) => format!("Loading chat history… {percent}%"),
                None => "Loading chat history…".to_owned(),
            },
            palette.accent,
            false,
            None,
        ),
        LinkStatus::Connected if update.is_some() => {
            let update = update.as_ref().expect("checked above");
            (
                Icon::Info,
                format!("ZapFast {} is available", update.version),
                palette.accent,
                false,
                Some(update.url.clone()),
            )
        }
        LinkStatus::Connected => return,
        LinkStatus::Starting | LinkStatus::Connecting => (
            Icon::Refresh,
            "Connecting to WhatsApp…".to_owned(),
            palette.secondary,
            false,
            None,
        ),
        LinkStatus::Disconnected { reason } => (
            Icon::WifiOff,
            format!("Offline ({reason}). Reconnecting…"),
            palette.warning,
            true,
            None,
        ),
        LinkStatus::Failed(message) => (
            Icon::CircleAlert,
            message.clone(),
            palette.danger,
            true,
            None,
        ),
        LinkStatus::Unlinked { .. } | LinkStatus::LoggedOut => (
            Icon::Smartphone,
            "Not linked to a phone".to_owned(),
            palette.warning,
            false,
            None,
        ),
    };
    egui::Panel::top("banner")
        .show_separator_line(false)
        .frame(
            Frame::new()
                .fill(palette.panel)
                .inner_margin(Margin::symmetric(14, 6)),
        )
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                // Progress/connection events wake the window. A long history
                // sync must not redraw every message just to spin this icon.
                theme::icon(ui, icon, 15.0, color);
                theme::text(ui, text, theme::medium(13.0), palette.text);
                if retry || download.is_some() {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if download.is_some() {
                            if theme::soft_button(
                                ui,
                                &palette,
                                Some(Icon::ExternalLink),
                                "Update",
                                false,
                            )
                            .clicked()
                            {
                                app.actions.push(Action::ShowUpdate);
                            }
                        } else if theme::soft_button(
                            ui,
                            &palette,
                            Some(Icon::Refresh),
                            "Retry",
                            false,
                        )
                        .clicked()
                        {
                            app.actions.push(Action::Reconnect);
                        }
                    });
                }
            });
        });
}

fn toasts(app: &mut App, ctx: &egui::Context) {
    if app.toasts.is_empty() {
        return;
    }
    let palette = app.palette;
    egui::Area::new(egui::Id::new("toasts"))
        .anchor(Align2::RIGHT_BOTTOM, vec2(-20.0, -20.0))
        .order(egui::Order::Tooltip)
        .interactable(false)
        .show(ctx, |ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            for toast in &app.toasts {
                let age = toast.created.elapsed().as_secs_f32();
                let alpha = if age < 0.15 {
                    age / 0.15
                } else if age > 2.8 {
                    ((3.2 - age) / 0.4).clamp(0.0, 1.0)
                } else {
                    1.0
                };
                ui.set_opacity(alpha);
                Frame::new()
                    .fill(palette.overlay)
                    .stroke(Stroke::new(1.0, palette.outline))
                    .corner_radius(CornerRadius::same(theme::RADIUS))
                    .inner_margin(Margin::symmetric(14, 10))
                    .shadow(egui::epaint::Shadow {
                        offset: [0, 4],
                        blur: 16,
                        spread: 0,
                        color: palette.shadow,
                    })
                    .show(ui, |ui| {
                        // Size to the message up to a readable maximum.
                        let font = theme::medium(13.5);
                        let laid = ui.painter().layout(
                            toast.message.clone(),
                            font.clone(),
                            palette.text,
                            360.0,
                        );
                        ui.set_width(laid.size().x + 26.0);
                        ui.horizontal(|ui| {
                            let (icon, color) = match toast.kind {
                                ToastKind::Info => (Icon::CircleCheck, palette.accent),
                                ToastKind::Error => (Icon::CircleAlert, palette.danger),
                            };
                            theme::icon(ui, icon, 16.0, color);
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(&toast.message)
                                        .font(font)
                                        .color(palette.text),
                                )
                                .wrap(),
                            );
                        });
                    });
            }
        });
}

/// Draggable space for the macOS traffic-light title bar.
fn titlebar_strip(app: &App, ui: &mut egui::Ui) {
    if app.is_linked() {
        return;
    }
    let inset = theme::titlebar_inset(ui.ctx());
    if inset == 0.0 {
        return;
    }
    let fill = if app.is_linked() {
        app.palette.panel
    } else {
        app.palette.window
    };
    egui::Panel::top("titlebar")
        .exact_size(inset)
        .show_separator_line(false)
        .frame(Frame::new().fill(fill))
        .show(ui, |ui| {
            let rect = ui.max_rect();
            titlebar_drag(ui, rect);
        });
}

/// Makes `rect` drag the window.
pub fn titlebar_drag(ui: &mut egui::Ui, rect: egui::Rect) {
    let response = ui.interact(
        rect,
        ui.id().with("titlebar-drag"),
        egui::Sense::click_and_drag(),
    );
    // AppKit requires StartDrag during the original mouse-down event.
    if response.is_pointer_button_down_on() && ui.input(|input| input.pointer.primary_pressed()) {
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
    }
}

/// Header for pages without a conversation toolbar and with the sidebar hidden.
pub fn standalone_header(app: &mut App, ui: &mut egui::Ui) {
    if !theme::macos_chrome(ui.ctx()) || app.sidebar_visible {
        return;
    }
    let palette = app.palette;
    egui::Panel::top("standalone-header")
        .exact_size(60.0)
        .show_separator_line(false)
        .frame(
            Frame::new()
                .fill(palette.panel)
                .inner_margin(Margin::symmetric(14, 8)),
        )
        .show(ui, |ui| {
            let mut drag = ui.max_rect();
            drag.min.x += theme::traffic_light_inset(ui.ctx());
            titlebar_drag(ui, drag);
            ui.horizontal(|ui| {
                ui.set_min_height(44.0);
                ui.add_space((theme::traffic_light_inset(ui.ctx()) - 14.0).max(0.0));
                if theme::icon_button(
                    ui,
                    Icon::PanelLeft,
                    18.0,
                    palette.secondary,
                    palette.text,
                    &keys::label("Show the chat list (Ctrl+B)"),
                )
                .clicked()
                {
                    app.actions.push(Action::ToggleSidebar);
                }
            });
        });
}

#[cfg(test)]
mod idle_tests {
    use super::*;
    #[test]
    fn history_sync_banner_does_not_animate_the_idle_window() {
        let root = tempfile::tempdir().unwrap();
        let mut app = App::headless(
            crate::paths::AppDirs::under(root.path()),
            crate::settings::Settings::default(),
        )
        .0;
        app.link = LinkStatus::Connected;
        app.syncing = true;
        app.sync_percent = Some(42);
        app.open_chat = None;
        let ctx = egui::Context::default();
        theme::install(&ctx);
        let mut delay = std::time::Duration::ZERO;
        for index in 0..6 {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    time: Some(index as f64),
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1180.0, 780.0),
                    )),
                    ..Default::default()
                },
                |ui| banner(&mut app, ui),
            );
            delay = output.viewport_output[&egui::ViewportId::ROOT].repaint_delay;
            output.textures_delta.clear();
        }
        assert!(
            delay > std::time::Duration::from_millis(100),
            "sync banner requested {delay:?}: {:?}",
            ctx.repaint_causes()
        );
    }

    #[test]
    fn conversation_at_bottom_does_not_request_continuous_repaints() {
        let root = tempfile::tempdir().unwrap();
        let mut app = App::headless(
            crate::paths::AppDirs::under(root.path()),
            crate::settings::Settings::default(),
        )
        .0;
        app.link = LinkStatus::Connected;
        let chat = crate::model::Chat::new("123@s.whatsapp.net".into(), "Alice".into());
        app.chats.push(chat.clone());
        app.open_chat = Some(chat.id.clone());
        app.scroll_to_bottom = true;
        let mut conversation = crate::app::Conversation::default();
        conversation.messages.push(crate::model::Message {
            id: "m1".into(),
            chat: chat.id.clone(),
            sender: "123@s.whatsapp.net".into(),
            sender_name: Some("Alice".into()),
            from_me: false,
            timestamp: 1000,
            content: crate::model::Content::text("Hello world"),
            status: crate::model::Delivery::None,
            delivered_at: None,
            read_at: None,
            quoted: None,
            reactions: Vec::new(),
            edited: false,
            mentions: Vec::new(),
            forwarded: false,
            thumbnail: None,
        });
        conversation.complete = true;
        app.conversations.insert(chat.id.clone(), conversation);

        let ctx = egui::Context::default();
        theme::install(&ctx);
        let mut delay = std::time::Duration::ZERO;
        for index in 0..6 {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    time: Some(index as f64),
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1180.0, 780.0),
                    )),
                    ..Default::default()
                },
                |ui| conversation::show(&mut app, ui),
            );
            delay = output.viewport_output[&egui::ViewportId::ROOT].repaint_delay;
            output.textures_delta.clear();
        }
        assert!(
            delay > std::time::Duration::from_millis(100),
            "idle conversation requested {delay:?}: {:?}",
            ctx.repaint_causes()
        );
    }

    #[test]
    fn reopening_a_scrolled_up_chat_scrolls_to_bottom() {
        let root = tempfile::tempdir().unwrap();
        let mut app = App::headless(
            crate::paths::AppDirs::under(root.path()),
            crate::settings::Settings::default(),
        )
        .0;
        let chat = crate::model::Chat::new("123@s.whatsapp.net".into(), "Alice".into());
        app.chats.push(chat.clone());
        let mut conversation = crate::app::Conversation::default();
        for i in 0..20 {
            conversation.messages.push(crate::model::Message {
                id: format!("m{i}"),
                chat: chat.id.clone(),
                sender: "123@s.whatsapp.net".into(),
                sender_name: Some("Alice".into()),
                from_me: false,
                timestamp: 1000 + i,
                content: crate::model::Content::text(format!("Message {i}")),
                status: crate::model::Delivery::None,
                delivered_at: None,
                read_at: None,
                quoted: None,
                reactions: Vec::new(),
                edited: false,
                mentions: Vec::new(),
                forwarded: false,
                thumbnail: None,
            });
        }
        conversation.complete = true;
        app.conversations.insert(chat.id.clone(), conversation);

        let ctx = egui::Context::default();
        theme::install(&ctx);

        // Open chat initially
        app.open_chat = Some(chat.id.clone());
        app.scroll_to_bottom = true;

        for _ in 0..3 {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1180.0, 780.0),
                    )),
                    ..Default::default()
                },
                |ui| conversation::show(&mut app, ui),
            );
            output.textures_delta.clear();
        }
        assert!(app.at_bottom, "at bottom after opening");

        // Simulate user scrolling up
        app.scroll_to_bottom = false;
        app.at_bottom = false;

        // Reopening chat requests scroll to bottom
        app.scroll_to_bottom = true;

        for _ in 0..3 {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1180.0, 780.0),
                    )),
                    ..Default::default()
                },
                |ui| conversation::show(&mut app, ui),
            );
            output.textures_delta.clear();
        }
        assert!(app.at_bottom, "at bottom after reopening");
    }
}
