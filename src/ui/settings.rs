//! The settings page.

use egui::{CornerRadius, Frame, Margin};

use crate::app::App;
use crate::model::{Action, Dialog, Page};
use crate::settings::ThemeChoice;
use crate::theme::{self, Icon};

use super::widgets;

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    super::standalone_header(app, ui);
    if theme::macos_chrome(ui.ctx()) {
        super::banner(app, ui);
    }
    let palette = app.palette;
    egui::ScrollArea::vertical()
        .id_salt("settings")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            Frame::new()
                .inner_margin(Margin::symmetric(32, 24))
                .show(ui, |ui| {
                    ui.set_max_width(640.0);
                    ui.horizontal(|ui| {
                        if theme::icon_button(
                            ui,
                            Icon::ArrowLeft,
                            20.0,
                            palette.secondary,
                            palette.text,
                            "Back (Esc)",
                        )
                        .clicked()
                        {
                            app.actions.push(Action::Open(Page::Chats));
                        }
                        theme::text(ui, "Settings", theme::bold(24.0), palette.text);
                    });
                    ui.add_space(18.0);

                    section(ui, app, "Appearance");
                    let detail = app.custom_themes.detail(app.settings.custom_theme.as_deref());
                    let detail = if !detail.is_empty() {
                        detail
                    } else if app.custom_themes.follows_omarchy() {
                        "Follow system uses your Omarchy colours."
                    } else {
                        "Follow system uses your desktop's light or dark appearance."
                    };
                    widgets::setting_row(ui, &palette, "Theme", detail, |ui| {
                        ui.with_layout(egui::Layout::top_down(egui::Align::Max), |ui| {
                            let selected = app.settings.custom_theme.as_deref()
                                .map(theme::custom::label)
                                .unwrap_or_else(|| app.settings.theme.label());
                            let response = egui::ComboBox::from_id_salt("appearance_theme")
                                .selected_text(" ")
                                .width(200.0_f32.min(ui.available_width()))
                                .height(320.0)
                                .show_ui(ui, |ui| {
                                    for choice in ThemeChoice::ALL {
                                        if theme_option(ui, &palette, choice.label(), app.settings.custom_theme.is_none() && app.settings.theme == choice) {
                                            app.actions.push(Action::SetTheme(choice));
                                        }
                                    }
                                    if app.custom_themes.picker_themes().next().is_some() {
                                        ui.separator();
                                    }
                                    for custom in app.custom_themes.picker_themes() {
                                        if theme_option(ui, &palette, theme::custom::label(&custom.filename), app.settings.custom_theme.as_deref() == Some(custom.filename.as_str())) {
                                            app.actions.push(Action::SetCustomTheme(custom.filename.clone()));
                                        }
                                    }
                                });
                            let rect = response.response.rect;
                            let text = widgets::line(ui, selected, theme::regular(14.0), palette.text, rect.width() - 36.0, 1);
                            text.paint(ui, egui::pos2(rect.left() + 8.0, rect.center().y - text.size().y / 2.0), palette.text);
                            response.response.widget_info(|| {
                                let mut info = egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, ui.is_enabled(), "Theme");
                                info.current_text_value = Some(selected.to_owned());
                                info
                            });
                            if theme::soft_button(ui, &palette, Some(Icon::ExternalLink), "Open themes folder", false).clicked() {
                                app.actions.push(Action::OpenThemesFolder);
                            }
                        });
                    });
                    widgets::setting_row(
                        ui,
                        &palette,
                        "Zoom",
                        "You can also use Ctrl+plus and Ctrl+minus.",
                        |ui| {
                            if theme::icon_button(ui, Icon::Plus, 16.0, palette.secondary, palette.text, "Larger").clicked() {
                                app.actions.push(Action::ZoomBy(0.1));
                            }
                            theme::text(
                                ui,
                                format!("{:.0}%", app.settings.zoom * 100.0),
                                theme::medium(13.5),
                                palette.text,
                            );
                            if theme::icon_button(ui, Icon::Minus, 16.0, palette.secondary, palette.text, "Smaller").clicked() {
                                app.actions.push(Action::ZoomBy(-0.1));
                            }
                        },
                    );

                    section(ui, app, "Chats");
                    toggle(ui, app, "Enter sends", "When off, Enter adds a line and Ctrl+Enter sends.", |settings| &mut settings.enter_sends);
                    let receipts_note = if app.account_receipts_off {
                        "Read receipts are disabled for your WhatsApp account. Direct chats will not send them. When this switch is on, groups still do. Read state syncs between your devices either way."
                    } else {
                        "Let people see when you read messages or play voice messages. Your WhatsApp privacy setting still applies. Read state syncs between your devices either way."
                    };
                    toggle(ui, app, "Send read receipts", receipts_note, |settings| &mut settings.send_read_receipts);
                    toggle(ui, app, "Show when you are typing", "", |settings| &mut settings.send_typing);
                    toggle(ui, app, "Download attachments automatically", "Download non-sticker attachments up to 64 MiB when they enter view. Visible stickers also download automatically up to this limit. When off, click an attachment up to this limit to download it.", |settings| &mut settings.auto_download);
                    toggle(ui, app, "Show sender pictures in every chat", "WhatsApp shows them in groups only.", |settings| &mut settings.show_sender_pictures);
                    toggle(ui, app, "Names from your address book", "Prefer saved contact names. When off, prefer public WhatsApp profile names. This applies throughout the app.", |settings| &mut settings.names_from_contacts);
                    toggle(ui, app, "Save contacts to the phone's address book", "Also add contacts saved here to your phone's address book. When off, they remain WhatsApp contacts. Names sync to linked devices either way.", |settings| &mut settings.save_contacts_to_phone);
                    toggle(ui, app, "Show shortcut hints", "", |settings| &mut settings.show_shortcut_hints);

                    {
                        // The buffer lives in egui memory: the hash is the only
                        // stored form, so there is nothing to read it back from.
                        let code_id = ui.id().with("chat_lock_code");
                        let mut code: String =
                            ui.data_mut(|data| data.get_temp(code_id).unwrap_or_default());
                        widgets::setting_row(
                            ui,
                            &palette,
                            "Secret code for locked chats",
                            "Open the Locked tab in the chat list and enter this local ZapFast code, separate from your phone's code. Leaving the tab or closing the window locks it again. Locked chats are hidden from ordinary search and notifications. This is a local visibility control, not an extra encryption layer. Keep it empty to remove the code.",
                            |ui| {
                                let response = ui.add(
                                    egui::TextEdit::singleline(&mut code)
                                        .font(theme::regular(13.0))
                                        .text_color(palette.text)
                                        .desired_width(220.0)
                                        .hint_text("Secret code")
                                        .password(true),
                                );
                                if response.changed() {
                                    let trimmed = code.trim().to_owned();
                                    app.actions.push(Action::SetChatLockCode(Some(trimmed)));
                                }
                                if app.settings.chat_lock_code_hash.is_some()
                                    && ui.small_button("Clear").clicked()
                                {
                                    code.clear();
                                    app.actions.push(Action::SetChatLockCode(None));
                                }
                                ui.data_mut(|data| data.insert_temp(code_id, code));
                            },
                        );
                    }

                    section(ui, app, "Window");
                    toggle(ui, app, "Keep running when the window closes", "Keep ZapFast linked in the system tray. Quit from the tray menu or with Ctrl+Q.", |settings| &mut settings.keep_running_in_background);
                    toggle(ui, app, "Notify about new messages", "Show desktop notifications when the window is hidden, in the background, or showing another chat. Muted chats do not notify you.", |settings| &mut settings.notifications);
                    toggle(ui, app, "Download updates automatically", "Download and verify new releases in the background. You choose when to restart. Native packages and Flatpak update through their package manager.", |settings| &mut settings.download_updates_automatically);
                    toggle(ui, app, "Check for updates", "Ask GitHub once a day whether a newer ZapFast release exists. The request identifies only ZapFast and its version.", |settings| &mut settings.check_for_updates);

                    widgets::setting_row(
                        ui,
                        &palette,
                        "GIPHY API key",
                        if crate::settings::BUILT_IN_GIPHY_KEY.is_some() {
                            "Used for GIF search. This build includes a key. Enter a key from developers.giphy.com to replace it."
                        } else {
                            "Required for GIF search. Get a free key from developers.giphy.com."
                        },
                        |ui| {
                            let response = ui.add(
                                egui::TextEdit::singleline(&mut app.settings.giphy_key)
                                    .font(theme::regular(13.0))
                                    .text_color(palette.text)
                                    .desired_width(220.0),
                            );
                            if response.changed() {
                                app.actions.push(Action::SettingsChanged);
                            }
                        },
                    );

                    section(ui, app, "Account");
                    let name = app.me_name.clone().unwrap_or_default();
                    let me = app.me.clone().unwrap_or_default();
                    let phone = crate::model::phone_of(&me)
                        .map(crate::util::phone)
                        .unwrap_or_else(|| me.clone());
                    let description = match &app.me_about {
                        Some(about) => format!("{phone} · {about}"),
                        None => phone,
                    };
                    ui.horizontal(|ui| {
                        let picture = app.avatar_full(&me).or_else(|| app.avatar(&me));
                        widgets::avatar(ui, &palette, &name, &me, 56.0, picture.as_deref());
                    });
                    ui.add_space(6.0);
                    widgets::setting_row(
                        ui,
                        &palette,
                        if name.is_empty() { "Linked device" } else { &name },
                        &description,
                        |ui| {
                            if theme::soft_button(ui, &palette, Some(Icon::LogOut), "Unlink this computer", false).clicked() {
                                app.actions.push(Action::ShowDialog(Dialog::ConfirmUnlink));
                            }
                        },
                    );

                    section(ui, app, "Files");
                    let archive = app.dirs.archive_db();
                    widgets::setting_row(
                        ui,
                        &palette,
                        "Message archive",
                        &archive.display().to_string(),
                        |ui| {
                            if theme::soft_button(ui, &palette, Some(Icon::ExternalLink), "Open folder", false).clicked() {
                                app.actions.push(Action::OpenFolder(app.dirs.state.clone()));
                            }
                        },
                    );
                    let media = app.dirs.media_dir();
                    widgets::setting_row(
                        ui,
                        &palette,
                        "Downloaded attachments",
                        &media.display().to_string(),
                        |ui| {
                            if theme::soft_button(ui, &palette, Some(Icon::ExternalLink), "Open folder", false).clicked() {
                                app.actions.push(Action::OpenMediaDir);
                            }
                            if theme::soft_button(ui, &palette, Some(Icon::Pencil), "Change", false).clicked() {
                                app.actions.push(Action::PickMediaDir);
                            }
                            if app.settings.custom_media_dir.is_some()
                                && theme::soft_button(ui, &palette, Some(Icon::Refresh), "Reset", false).clicked()
                            {
                                app.actions.push(Action::ResetMediaDir);
                            }
                        },
                    );
                    let log = app.dirs.log_file();
                    widgets::setting_row(ui, &palette, "Log of this run", &log.display().to_string(), |ui| {
                        if theme::soft_button(ui, &palette, Some(Icon::FileText), "Open", false).clicked() {
                            app.actions.push(Action::OpenFile(log.clone()));
                        }
                    });

                    section(ui, app, "About");
                    widgets::setting_row(
                        ui,
                        &palette,
                        &format!("ZapFast {}", env!("CARGO_PKG_VERSION")),
                        "A native WhatsApp client built with Rust, egui, and whatsapp-rust.",
                        |ui| {
                            if theme::soft_button(ui, &palette, Some(Icon::Info), "About", false).clicked() {
                                app.actions.push(Action::ShowDialog(Dialog::About));
                            }
                            if theme::soft_button(ui, &palette, Some(Icon::Keyboard), "Shortcuts", false).clicked() {
                                app.actions.push(Action::ShowDialog(Dialog::Shortcuts));
                            }
                        },
                    );
                });
        });
}

fn section(ui: &mut egui::Ui, app: &App, label: &str) {
    let palette = app.palette;
    ui.add_space(10.0);
    Frame::new()
        .fill(palette.panel)
        .corner_radius(CornerRadius::same(theme::RADIUS))
        .inner_margin(Margin::symmetric(14, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            theme::text(ui, label, theme::semibold(12.5), palette.accent);
        });
    ui.add_space(8.0);
}

fn toggle(
    ui: &mut egui::Ui,
    app: &mut App,
    label: &str,
    description: &str,
    field: impl Fn(&mut crate::settings::Settings) -> &mut bool,
) {
    let palette = app.palette;
    let mut value = *field(&mut app.settings);
    let mut changed = false;
    widgets::setting_row(ui, &palette, label, description, |ui| {
        let response = widgets::switch(ui, &palette, &mut value);
        theme::reveal_focus(&response);
        response.widget_info(|| {
            egui::WidgetInfo::selected(egui::WidgetType::Checkbox, ui.is_enabled(), value, label)
        });
        changed = response.changed();
    });
    if changed {
        *field(&mut app.settings) = value;
        app.actions.push(Action::SettingsChanged);
    }
}

/// Theme filenames can contain emoji, so paint them through the shared line renderer.
fn theme_option(ui: &mut egui::Ui, palette: &theme::Palette, text: &str, selected: bool) -> bool {
    let response = ui.add(
        egui::Button::selectable(selected, " ").min_size(egui::vec2(ui.available_width(), 28.0)),
    );
    let rect = response.rect;
    let line = widgets::line(
        ui,
        text,
        theme::regular(14.0),
        palette.text,
        rect.width() - 16.0,
        1,
    );
    if ui.is_rect_visible(rect) {
        line.paint(
            ui,
            egui::pos2(rect.left() + 8.0, rect.center().y - line.size().y / 2.0),
            palette.text,
        );
    }
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::SelectableLabel,
            ui.is_enabled(),
            selected,
            text,
        )
    });
    response.clicked()
}
