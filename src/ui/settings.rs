//! The settings page.

use egui::{Align, CornerRadius, Frame, Layout, Margin, Rect, Stroke, Vec2, pos2, vec2};

use crate::app::App;
use crate::model::{Action, Dialog, Page};
use crate::settings::{ThemeChoice, WallpaperColor};
use crate::theme::{self, Icon, Palette};
use crate::wallpaper;

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
                    ui.set_max_width(ui.available_width().min(640.0));
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
                        theme::text(ui, crate::i18n::gettext(app.locale, "Settings"), theme::bold(24.0), palette.text);
                    });
                    ui.add_space(18.0);

                    section(ui, &palette, &crate::i18n::gettext(app.locale, "Appearance"), |ui| {
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
                            "Wallpaper",
                            app.settings.wallpaper_color_for(palette.dark).label(),
                            |ui| {
                                if theme::soft_button(
                                    ui,
                                    &palette,
                                    Some(Icon::ChevronRight),
                                    app.settings.wallpaper_color_for(palette.dark).label(),
                                    false,
                                )
                                .clicked()
                                {
                                    app.actions.push(Action::Open(Page::Wallpaper));
                                }
                            },
                        );
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

                        widgets::setting_row(
                            ui,
                            &palette,
                            crate::i18n::gettext(app.locale, "Language").as_ref(),
                            "",
                            |ui| {
                                let selected = app.settings.interface_language;
                                let label = match selected {
                                    Some(locale) => locale.label().to_owned(),
                                    None => crate::i18n::gettext(app.locale, "Auto").into_owned(),
                                };
                                egui::ComboBox::from_id_salt("interface_language")
                                    .selected_text(label)
                                    .width(200.0_f32.min(ui.available_width()))
                                    .show_ui(ui, |ui| {
                                        if theme_option(
                                            ui,
                                            &palette,
                                            crate::i18n::gettext(app.locale, "Auto").as_ref(),
                                            selected.is_none(),
                                        ) {
                                            app.actions.push(Action::SetInterfaceLanguage(None));
                                        }
                                        for locale in crate::i18n::Locale::ALL {
                                            if theme_option(
                                                ui,
                                                &palette,
                                                locale.label(),
                                                selected == Some(locale),
                                            ) {
                                                app.actions
                                                    .push(Action::SetInterfaceLanguage(Some(locale)));
                                            }
                                        }
                                    });
                            },
                        );
                    });

                    section(ui, &palette, &crate::i18n::gettext(app.locale, "Chats"), |ui| {
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
                        toggle(ui, app, "Collapse the chat list to icons", "Hiding the chat list (Ctrl+B) leaves a narrow column of avatars with unread badges instead of removing it. Clicking an avatar opens that chat, and Ctrl+B brings the full list back.", |settings| &mut settings.collapse_chat_list);

                        // macOS has no public API to pause other apps' media.
                        if crate::media_pause::SUPPORTED {
                            let locale = app.locale;
                            toggle(ui, app, &crate::i18n::gettext(locale, "Pause music while recording"), &crate::i18n::gettext(locale, "Pause media players while you record a voice message and resume them afterwards."), |settings| &mut settings.pause_media_while_recording);
                            toggle(ui, app, &crate::i18n::gettext(locale, "Pause music while playing voice messages"), &crate::i18n::gettext(locale, "Pause media players while a voice or audio message plays and resume them when it stops."), |settings| &mut settings.pause_media_while_playing);
                        }

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
                    });

                    section(ui, &palette, &crate::i18n::gettext(app.locale, "Window"), |ui| {
                        toggle(ui, app, "Keep running when the window closes", "Keep ZapFast linked in the system tray. Quit from the tray menu or with Ctrl+Q.", |settings| &mut settings.keep_running_in_background);
                        if let Some(mut enabled) = app.start_with_system {
                            widgets::setting_row(ui, &palette, "Start at login", "Start ZapFast when you log in. It waits in the system tray, without a window, while it keeps running in the background.", |ui| {
                                let response = widgets::switch(ui, &palette, &mut enabled);
                                theme::reveal_focus(&response);
                                response.widget_info(|| {
                                    egui::WidgetInfo::selected(egui::WidgetType::Checkbox, ui.is_enabled(), enabled, "Start at login")
                                });
                                if response.changed() {
                                    app.actions.push(Action::SetStartWithSystem(enabled));
                                }
                            });
                        }
                        toggle(ui, app, "Notify about new messages", "Show desktop notifications when the window is hidden, in the background, or showing another chat. Muted chats do not notify you.", |settings| &mut settings.notifications);
                        if app.settings.notifications {
                            sound_row(ui, app, false);
                            sound_row(ui, app, true);
                        }
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
                    });

                    section(ui, &palette, &crate::i18n::gettext(app.locale, "Network"), |ui| {
                        let environment = app
                            .settings
                            .proxy
                            .is_empty()
                            .then(crate::proxy::for_whatsapp)
                            .flatten();
                        let description = match environment {
                            Some(proxy) => format!("Using {} from the environment. Enter a proxy to replace it.", proxy.redacted()),
                            None => "socks5h://, socks5://, or http:// with an optional user:password@. Leave empty to use ALL_PROXY or HTTPS_PROXY.".to_owned(),
                        };
                        widgets::setting_row(ui, &palette, "Proxy", &description, |ui| {
                            let id = ui.id().with("proxy-draft");
                            let mut draft = ui
                                .data(|data| data.get_temp::<String>(id))
                                .unwrap_or_else(|| app.settings.proxy.clone());
                            let response = ui.add(
                                egui::TextEdit::singleline(&mut draft)
                                    .hint_text("socks5h://127.0.0.1:9050")
                                    .font(theme::regular(13.0))
                                    .text_color(palette.text)
                                    .desired_width(220.0),
                            );
                            if response.lost_focus() {
                                app.actions.push(Action::SetProxy(draft.clone()));
                                ui.data_mut(|data| data.remove::<String>(id));
                            } else if response.has_focus() {
                                ui.data_mut(|data| data.insert_temp(id, draft));
                            }
                        });
                    });

                    section(ui, &palette, &crate::i18n::gettext(app.locale, "Account"), |ui| {
                        account(app, ui);
                    });

                    section(ui, &palette, &crate::i18n::gettext(app.locale, "Files"), |ui| {
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
                        let custom = app.settings.download_folder.clone();
                        let media = custom.clone().unwrap_or_else(|| app.dirs.media_cache_dir());
                        let description = if custom.is_some() {
                            format!("{}. Earlier downloads stay where they are.", media.display())
                        } else {
                            media.display().to_string()
                        };
                        widgets::setting_row(
                            ui,
                            &palette,
                            "Downloaded attachments",
                            &description,
                            |ui| {
                                if theme::soft_button(ui, &palette, Some(Icon::ExternalLink), "Open folder", false).clicked() {
                                    let _ = std::fs::create_dir_all(&media);
                                    app.actions.push(Action::OpenFolder(media.clone()));
                                }
                                if theme::soft_button(ui, &palette, None, "Change…", false).clicked() {
                                    app.actions.push(Action::PickDownloadFolder);
                                }
                                if custom.is_some() && theme::soft_button(ui, &palette, None, "Use default", false).clicked() {
                                    app.actions.push(Action::SetDownloadFolder(None));
                                }
                            },
                        );
                        let log = app.dirs.log_file();
                        widgets::setting_row(ui, &palette, "Log of this run", &log.display().to_string(), |ui| {
                            if theme::soft_button(ui, &palette, Some(Icon::FileText), "Open", false).clicked() {
                                app.actions.push(Action::OpenFile(log.clone()));
                            }
                        });
                    });

                    section(ui, &palette, &crate::i18n::gettext(app.locale, "About"), |ui| {
                        about(app, ui);
                    });
                });
        });
}

/// Wallpaper colour picker and live preview.
pub fn wallpaper_show(app: &mut App, ui: &mut egui::Ui) {
    super::standalone_header(app, ui);
    if theme::macos_chrome(ui.ctx()) {
        super::banner(app, ui);
    }
    let palette = app.palette;
    let body_height = ui.available_height().max(0.0);
    ui.with_layout(
        Layout::left_to_right(Align::Min).with_main_align(Align::Min),
        |ui| {
            const HEADER_HEIGHT: f32 = 52.0;
            const MIN_PREVIEW_WIDTH: f32 = 180.0;
            let total_width = ui.available_width();
            let left_width = (total_width * 0.42)
                .clamp(220.0, 520.0)
                .min((total_width - MIN_PREVIEW_WIDTH).max(0.0));
            let palette_width = (left_width - 40.0).max(0.0);
            let preview_width = (total_width - left_width).max(0.0);
            ui.allocate_ui_with_layout(
                vec2(left_width, body_height),
                Layout::top_down(Align::Min).with_main_align(Align::Min),
                |ui| {
                    let section = ui.max_rect();
                    let header = Rect::from_min_size(
                        section.left_top(),
                        vec2(left_width, HEADER_HEIGHT),
                    );
                    ui.painter().rect_filled(header, 0.0, palette.panel);
                    ui.scope_builder(
                        egui::UiBuilder::new()
                            .max_rect(header)
                            .layout(
                                Layout::left_to_right(Align::Center)
                                    .with_main_align(Align::Min),
                            ),
                        |ui| {
                            ui.add_space(24.0);
                            if theme::icon_button(
                                ui,
                                Icon::ArrowLeft,
                                20.0,
                                palette.secondary,
                                palette.text,
                                "Back to settings",
                            )
                            .clicked()
                            {
                                app.actions.push(Action::Open(Page::Settings));
                            }
                            theme::text(ui, "Set chat wallpaper", theme::bold(18.0), palette.text);
                        },
                    );

                    let palette_rect = Rect::from_min_size(
                        pos2(section.left() + 20.0, header.bottom() + 20.0),
                        vec2(palette_width, (body_height - HEADER_HEIGHT - 20.0).max(0.0)),
                    );
                    ui.scope_builder(
                        egui::UiBuilder::new()
                            .max_rect(palette_rect)
                            .layout(Layout::top_down(Align::Min).with_main_align(Align::Min)),
                        |ui| {
                            egui::ScrollArea::vertical()
                                .id_salt("wallpaper-palette")
                                .auto_shrink([false, false])
                                .show(ui, |ui| {
                                    ui.allocate_ui_with_layout(
                                        vec2(palette_width, 28.0),
                                        Layout::top_down(Align::Center),
                                        |ui| {
                                            let mut doodles = app.settings.show_wallpaper;
                                            let checkbox = ui.checkbox(
                                                &mut doodles,
                                                crate::i18n::gettext(app.locale, "Add doodles"),
                                            );
                                            checkbox.on_hover_text(crate::i18n::gettext(
                                                app.locale,
                                                "Show the default doodles over the selected colour.",
                                            ));
                                            if doodles != app.settings.show_wallpaper {
                                                app.actions.push(Action::SetWallpaperDoodles(doodles));
                                            }
                                        },
                                    );
                                    ui.add_space(18.0);
                                    let button_width = 80.0;
                                    let item_spacing = ui.spacing().item_spacing.x;
                                    let columns = ((palette_width + item_spacing)
                                        / (button_width + item_spacing))
                                        .floor()
                                        .max(1.0)
                                        as usize;
                                    let grid_width = button_width * columns as f32
                                        + item_spacing * columns.saturating_sub(1) as f32;
                                    ui.horizontal(|ui| {
                                        ui.add_space((palette_width - grid_width).max(0.0) / 2.0);
                                        ui.horizontal_wrapped(|ui| {
                                            let selected =
                                                app.settings.wallpaper_color_for(palette.dark);
                                            for color in WallpaperColor::choices(palette.dark) {
                                                if wallpaper_color_button(ui, *color, selected) {
                                                    app.actions.push(Action::SetWallpaperColor(*color));
                                                }
                                            }
                                        });
                                    });
                                });
                        },
                    );
                },
            );
            let divider_x = ui.cursor().left();
            let divider_top = ui.cursor().top();
            ui.allocate_ui_with_layout(
                vec2(preview_width, body_height),
                Layout::top_down(Align::Min).with_main_align(Align::Min),
                |ui| {
                    let section = ui.max_rect();
                    let header = Rect::from_min_size(
                        section.left_top(),
                        vec2(preview_width, HEADER_HEIGHT),
                    );
                    ui.painter().rect_filled(header, 0.0, palette.panel);
                    ui.painter().text(
                        header.center(),
                        egui::Align2::CENTER_CENTER,
                        "Wallpaper preview",
                        theme::bold(18.0),
                        palette.text,
                    );
                    let preview = Rect::from_min_size(
                        header.left_bottom(),
                        vec2(preview_width, (body_height - HEADER_HEIGHT).max(0.0)),
                    );
                    wallpaper::paint_rect(
                        ui,
                        preview,
                        app.settings.wallpaper_color_for(palette.dark),
                        app.settings.show_wallpaper,
                    );
                },
            );
            ui.painter().line_segment(
                [
                    pos2(divider_x, divider_top),
                    pos2(divider_x, divider_top + body_height),
                ],
                Stroke::new(1.0, palette.outline),
            );
        },
    );
}

fn wallpaper_color_button(
    ui: &mut egui::Ui,
    color: WallpaperColor,
    selected: WallpaperColor,
) -> bool {
    let button = egui::Button::new(egui::RichText::new(" "))
        .min_size(Vec2::splat(80.0))
        .fill(color.color32())
        .stroke(if color == selected {
            Stroke::new(4.0, color.color32().gamma_multiply(0.5))
        } else {
            Stroke::NONE
        })
        .corner_radius(CornerRadius::ZERO);
    let response = ui.add(button).on_hover_text(color.label());
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::Button,
            ui.is_enabled(),
            color == selected,
            color.label(),
        )
    });
    response.clicked()
}

/// A titled group of settings on a rounded card.
fn section(
    ui: &mut egui::Ui,
    palette: &Palette,
    title: &str,
    add_contents: impl FnOnce(&mut egui::Ui),
) {
    ui.add_space(12.0);
    theme::text(ui, title, theme::bold(18.0), palette.text);
    ui.add_space(8.0);
    Frame::new()
        .fill(theme::blend(palette.panel, palette.surface, 0.5))
        .stroke(Stroke::new(1.0, palette.outline))
        .corner_radius(CornerRadius::same(theme::RADIUS + 2))
        // Every row ends with its own gap, so the bottom margin is smaller.
        .inner_margin(Margin {
            left: 20,
            right: 20,
            top: 16,
            bottom: 6,
        })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add_contents(ui);
        });
    ui.add_space(8.0);
}

/// Our WhatsApp profile, which can be edited in place, and unlinking.
fn account(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    let name = app.me_name.clone().unwrap_or_default();
    let me = app.me.clone().unwrap_or_default();
    let phone = crate::model::phone_of(&me)
        .map(crate::util::phone)
        .unwrap_or_else(|| me.clone());
    // The name and About being edited; `None` while not editing.
    let draft_id = ui.id().with("profile_draft");
    let mut draft: Option<(String, String)> = ui.data_mut(|data| data.get_temp(draft_id));
    let mut submitted = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 14.0;
        let picture = app.avatar_full(&me).or_else(|| app.avatar(&me));
        let change_picture = crate::i18n::gettext(app.locale, "Change profile picture");
        if widgets::clickable_avatar(
            ui,
            &palette,
            &name,
            &me,
            56.0,
            picture.as_deref(),
            &change_picture,
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(change_picture.as_ref())
        .clicked()
        {
            app.actions.push(Action::PickProfilePicture);
        }
        ui.vertical(|ui| {
            ui.set_width((ui.available_width() - 230.0).max(160.0));
            if let Some((draft_name, draft_about)) = &mut draft {
                submitted |= profile_field(
                    ui,
                    &palette,
                    draft_name,
                    &crate::i18n::gettext(app.locale, "Your name"),
                    25,
                );
                ui.add_space(6.0);
                submitted |= profile_field(
                    ui,
                    &palette,
                    draft_about,
                    &crate::i18n::gettext(app.locale, "About"),
                    139,
                );
                return;
            }
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let shown = if name.is_empty() {
                    crate::i18n::gettext(app.locale, "Linked device")
                } else {
                    name.clone().into()
                };
                theme::text(ui, shown, theme::semibold(16.0), palette.text);
                if theme::icon_button(
                    ui,
                    Icon::Pencil,
                    15.0,
                    palette.secondary,
                    palette.text,
                    &crate::i18n::gettext(app.locale, "Edit your name and About"),
                )
                .clicked()
                {
                    draft = Some((name.clone(), app.me_about.clone().unwrap_or_default()));
                }
            });
            theme::text(ui, &phone, theme::regular(13.0), palette.secondary);
            if let Some(about) = &app.me_about {
                theme::paragraph(ui, about, theme::regular(13.0), palette.secondary);
            }
        });
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if theme::soft_button(
                ui,
                &palette,
                Some(Icon::LogOut),
                &crate::i18n::gettext(app.locale, "Unlink this computer"),
                false,
            )
            .clicked()
            {
                app.actions.push(Action::ShowDialog(Dialog::ConfirmUnlink));
            }
        });
    });
    if let Some((draft_name, draft_about)) = &draft {
        ui.add_space(10.0);
        let mut done = false;
        ui.horizontal(|ui| {
            ui.add_space(56.0 + 14.0);
            let name = draft_name.trim();
            let about = draft_about.trim();
            let changed_name =
                (!name.is_empty() && app.me_name.as_deref() != Some(name)).then(|| name.to_owned());
            let changed_about =
                (app.me_about.as_deref().unwrap_or_default() != about).then(|| about.to_owned());
            let save = crate::i18n::gettext(app.locale, "Save");
            if theme::pill_button(ui, &palette, &save, true).clicked() || submitted {
                if changed_name.is_some() || changed_about.is_some() {
                    app.actions.push(Action::SetProfile {
                        name: changed_name,
                        about: changed_about,
                    });
                }
                done = true;
            }
            let cancel = crate::i18n::gettext(app.locale, "Cancel");
            if theme::pill_button(ui, &palette, &cancel, false).clicked() {
                done = true;
            }
        });
        if done {
            draft = None;
        }
    }
    ui.add_space(10.0);
    ui.data_mut(|data| {
        if let Some(draft) = draft {
            data.insert_temp(draft_id, draft);
        } else {
            data.remove::<(String, String)>(draft_id);
        }
    });
}

/// A single-line profile text field with its caption above it. Returns
/// whether Enter submitted it.
fn profile_field(
    ui: &mut egui::Ui,
    palette: &Palette,
    value: &mut String,
    label: &str,
    limit: usize,
) -> bool {
    theme::text(ui, label, theme::medium(12.5), palette.secondary);
    let response = ui.add(
        egui::TextEdit::singleline(value)
            .font(theme::regular(14.0))
            .text_color(palette.text)
            .char_limit(limit)
            .desired_width(ui.available_width()),
    );
    response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter))
}

/// The version, links to more about ZapFast, and who made it.
fn about(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 14.0;
        let (logo, _) = ui.allocate_exact_size(Vec2::splat(44.0), egui::Sense::hover());
        // The white glyph on the accent disc matches the app icon.
        theme::logo(
            ui,
            logo.center(),
            44.0,
            palette.accent,
            egui::Color32::WHITE,
        );
        ui.vertical(|ui| {
            theme::text(
                ui,
                format!("ZapFast {}", env!("CARGO_PKG_VERSION")),
                theme::semibold(16.0),
                palette.text,
            );
            theme::paragraph(
                ui,
                crate::i18n::gettext(
                    app.locale,
                    "A native WhatsApp client built with Rust, egui, and whatsapp-rust.",
                ),
                theme::regular(13.0),
                palette.secondary,
            );
        });
    });
    ui.add_space(12.0);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
        if let Some(update) = &app.update {
            let label = crate::i18n::gettext(app.locale, "Update to {version}")
                .replace("{version}", &update.version);
            if theme::soft_button(ui, &palette, Some(Icon::Download), &label, false).clicked() {
                app.actions.push(Action::ShowUpdate);
            }
        }
        if theme::soft_button(
            ui,
            &palette,
            Some(Icon::Keyboard),
            &crate::i18n::gettext(app.locale, "Keyboard shortcuts"),
            false,
        )
        .clicked()
        {
            app.actions.push(Action::ShowDialog(Dialog::Shortcuts));
        }
        if theme::soft_button(
            ui,
            &palette,
            Some(Icon::Info),
            &crate::i18n::gettext(app.locale, "About"),
            false,
        )
        .clicked()
        {
            app.actions.push(Action::ShowDialog(Dialog::About));
        }
        if theme::soft_button(
            ui,
            &palette,
            Some(Icon::ExternalLink),
            &crate::i18n::gettext(app.locale, "Source code"),
            false,
        )
        .clicked()
        {
            app.actions
                .push(Action::OpenUrl(env!("CARGO_PKG_REPOSITORY").to_owned()));
        }
    });
    ui.add_space(14.0);
    if widgets::credit(ui, &palette, app.locale) {
        app.actions
            .push(Action::OpenUrl(widgets::AUTHOR_URL.to_owned()));
    }
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

/// Sound choice for chat or group notifications, like on the phone.
fn sound_row(ui: &mut egui::Ui, app: &mut App, group: bool) {
    use crate::settings::NotificationSound;
    let palette = app.palette;
    let current = if group {
        app.settings.group_sound.clone()
    } else {
        app.settings.message_sound.clone()
    };
    let (label, description) = if group {
        ("Group sound", "Played for new group messages.")
    } else {
        (
            "Message sound",
            "Played for new messages in one-to-one chats.",
        )
    };
    let selected = match &current {
        NotificationSound::Chime => "Chime".to_owned(),
        NotificationSound::Ripple => "Ripple".to_owned(),
        NotificationSound::System => "System default".to_owned(),
        NotificationSound::None => "None".to_owned(),
        NotificationSound::Custom(path) => path.file_name().map_or_else(
            || "Custom".to_owned(),
            |name| name.to_string_lossy().into_owned(),
        ),
    };
    widgets::setting_row(ui, &palette, label, description, |ui| {
        if !matches!(current, NotificationSound::System | NotificationSound::None)
            && theme::icon_button(
                ui,
                Icon::Play,
                16.0,
                palette.secondary,
                palette.text,
                "Play",
            )
            .clicked()
        {
            app.actions.push(Action::PreviewSound(current.clone()));
        }
        egui::ComboBox::from_id_salt(("notification-sound", group))
            .selected_text(selected)
            .width(170.0_f32.min(ui.available_width()))
            .show_ui(ui, |ui| {
                for (sound, name) in [
                    (NotificationSound::Chime, "Chime"),
                    (NotificationSound::Ripple, "Ripple"),
                    (NotificationSound::System, "System default"),
                    (NotificationSound::None, "None"),
                ] {
                    if ui.selectable_label(current == sound, name).clicked() {
                        app.actions
                            .push(Action::SetNotificationSound { group, sound });
                    }
                }
                if ui.selectable_label(false, "Choose a file…").clicked() {
                    app.actions.push(Action::PickNotificationSound { group });
                }
            });
    });
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
