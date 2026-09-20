//! The settings page.

use egui::{CornerRadius, Frame, Margin};

use crate::app::App;
use crate::i18n::{Language, t};
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
    let lang = app.settings.language;
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
                            t(lang, "settings.back"),
                        )
                        .clicked()
                        {
                            app.actions.push(Action::Open(Page::Chats));
                        }
                        theme::text(
                            ui,
                            t(lang, "settings.title"),
                            theme::bold(24.0),
                            palette.text,
                        );
                    });
                    ui.add_space(16.0);

                    section(ui, app, t(lang, "settings.section.appearance"));
                    let detail = app
                        .custom_themes
                        .detail(app.settings.custom_theme.as_deref());
                    let detail = if !detail.is_empty() {
                        detail
                    } else if app.custom_themes.follows_omarchy() {
                        t(lang, "settings.theme.omarchy")
                    } else {
                        t(lang, "settings.theme.system")
                    };
                    widgets::setting_row(ui, &palette, t(lang, "settings.theme"), detail, |ui| {
                        ui.with_layout(egui::Layout::top_down(egui::Align::Max), |ui| {
                            let selected = app
                                .settings
                                .custom_theme
                                .as_deref()
                                .map(theme::custom::label)
                                .unwrap_or_else(|| theme_label(app.settings.theme, lang));
                            egui::ComboBox::from_id_salt("appearance_theme")
                                .selected_text(selected)
                                .width(200.0_f32.min(ui.available_width()))
                                .height(320.0)
                                .show_ui(ui, |ui| {
                                    for choice in ThemeChoice::ALL {
                                        if theme_option(
                                            ui,
                                            &palette,
                                            theme_label(choice, lang),
                                            app.settings.custom_theme.is_none()
                                                && app.settings.theme == choice,
                                        ) {
                                            app.actions.push(Action::SetTheme(choice));
                                        }
                                    }
                                    if app.custom_themes.picker_themes().next().is_some() {
                                        ui.separator();
                                    }
                                    for custom in app.custom_themes.picker_themes() {
                                        if theme_option(
                                            ui,
                                            &palette,
                                            theme::custom::label(&custom.filename),
                                            app.settings.custom_theme.as_deref()
                                                == Some(custom.filename.as_str()),
                                        ) {
                                            app.actions.push(Action::SetCustomTheme(
                                                custom.filename.clone(),
                                            ));
                                        }
                                    }
                                });
                            if theme::soft_button(
                                ui,
                                &palette,
                                Some(Icon::ExternalLink),
                                t(lang, "settings.open_themes"),
                                false,
                            )
                            .clicked()
                            {
                                app.actions.push(Action::OpenThemesFolder);
                            }
                        });
                    });
                    widgets::setting_row(
                        ui,
                        &palette,
                        t(lang, "settings.language"),
                        t(lang, "settings.language.hint"),
                        |ui| {
                            egui::ComboBox::from_id_salt("interface_language")
                                .selected_text(app.settings.language.label())
                                .width(200.0_f32.min(ui.available_width()))
                                .show_ui(ui, |ui| {
                                    for choice in Language::ALL {
                                        if ui
                                            .selectable_label(
                                                app.settings.language == choice,
                                                choice.label(),
                                            )
                                            .clicked()
                                        {
                                            app.settings.language = choice;
                                            app.actions.push(Action::SettingsChanged);
                                        }
                                    }
                                });
                        },
                    );
                    widgets::setting_row(
                        ui,
                        &palette,
                        t(lang, "settings.zoom"),
                        t(lang, "settings.zoom.hint"),
                        |ui| {
                            if theme::icon_button(
                                ui,
                                Icon::Plus,
                                16.0,
                                palette.secondary,
                                palette.text,
                                t(lang, "settings.zoom.in"),
                            )
                            .clicked()
                            {
                                app.actions.push(Action::ZoomBy(0.1));
                            }
                            theme::text(
                                ui,
                                format!("{:.0}%", app.settings.zoom * 100.0),
                                theme::medium(13.5),
                                palette.text,
                            );
                            if theme::icon_button(
                                ui,
                                Icon::Minus,
                                16.0,
                                palette.secondary,
                                palette.text,
                                t(lang, "settings.zoom.out"),
                            )
                            .clicked()
                            {
                                app.actions.push(Action::ZoomBy(-0.1));
                            }
                        },
                    );

                    section(ui, app, t(lang, "settings.section.chats"));
                    toggle(
                        ui,
                        app,
                        t(lang, "settings.enter_sends"),
                        t(lang, "settings.enter_sends.hint"),
                        |settings| &mut settings.enter_sends,
                    );
                    let receipts_note = if app.account_receipts_off {
                        t(lang, "settings.read_receipts.off")
                    } else {
                        t(lang, "settings.read_receipts.on")
                    };
                    toggle(
                        ui,
                        app,
                        t(lang, "settings.read_receipts"),
                        receipts_note,
                        |settings| &mut settings.send_read_receipts,
                    );
                    toggle(ui, app, t(lang, "settings.typing"), "", |settings| {
                        &mut settings.send_typing
                    });
                    toggle(
                        ui,
                        app,
                        t(lang, "settings.auto_download"),
                        t(lang, "settings.auto_download.hint"),
                        |settings| &mut settings.auto_download,
                    );
                    toggle(
                        ui,
                        app,
                        t(lang, "settings.sender_pictures"),
                        t(lang, "settings.sender_pictures.hint"),
                        |settings| &mut settings.show_sender_pictures,
                    );
                    toggle(
                        ui,
                        app,
                        t(lang, "settings.names_from_contacts"),
                        t(lang, "settings.names_from_contacts.hint"),
                        |settings| &mut settings.names_from_contacts,
                    );
                    toggle(
                        ui,
                        app,
                        t(lang, "settings.save_to_phone"),
                        t(lang, "settings.save_to_phone.hint"),
                        |settings| &mut settings.save_contacts_to_phone,
                    );
                    toggle(
                        ui,
                        app,
                        t(lang, "settings.shortcut_hints"),
                        "",
                        |settings| &mut settings.show_shortcut_hints,
                    );

                    section(ui, app, t(lang, "settings.section.window"));
                    toggle(
                        ui,
                        app,
                        t(lang, "settings.keep_running"),
                        t(lang, "settings.keep_running.hint"),
                        |settings| &mut settings.keep_running_in_background,
                    );
                    toggle(
                        ui,
                        app,
                        t(lang, "settings.notifications"),
                        t(lang, "settings.notifications.hint"),
                        |settings| &mut settings.notifications,
                    );
                    toggle(
                        ui,
                        app,
                        t(lang, "settings.auto_update"),
                        t(lang, "settings.auto_update.hint"),
                        |settings| &mut settings.download_updates_automatically,
                    );
                    toggle(
                        ui,
                        app,
                        t(lang, "settings.check_updates"),
                        t(lang, "settings.check_updates.hint"),
                        |settings| &mut settings.check_for_updates,
                    );

                    widgets::setting_row(
                        ui,
                        &palette,
                        t(lang, "settings.giphy"),
                        if crate::settings::BUILT_IN_GIPHY_KEY.is_some() {
                            t(lang, "settings.giphy.builtin")
                        } else {
                            t(lang, "settings.giphy.missing")
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

                    section(ui, app, t(lang, "settings.section.account"));
                    let name = app.me_name.clone().unwrap_or_default();
                    let me = app.me.clone().unwrap_or_default();
                    let phone = crate::model::phone_of(&me)
                        .map(crate::util::phone)
                        .unwrap_or_else(|| me.clone());
                    let description = match &app.me_about {
                        Some(about) => format!("{phone} · {about}"),
                        None => phone,
                    };
                    let title = if name.is_empty() {
                        t(lang, "settings.linked_device").to_owned()
                    } else {
                        name.clone()
                    };
                    ui.horizontal(|ui| {
                        let picture = app.avatar_full(&me).or_else(|| app.avatar(&me));
                        widgets::avatar(ui, &palette, &name, &me, 56.0, picture.as_deref());
                        ui.vertical(|ui| {
                            theme::text(ui, &title, theme::semibold(15.0), palette.text);
                            theme::text(ui, &description, theme::regular(13.0), palette.secondary);
                        });
                    });
                    ui.add_space(8.0);
                    widgets::setting_row(ui, &palette, &title, &description, |ui| {
                        if theme::soft_button(
                            ui,
                            &palette,
                            Some(Icon::LogOut),
                            t(lang, "settings.unlink"),
                            false,
                        )
                        .clicked()
                        {
                            app.actions.push(Action::ShowDialog(Dialog::ConfirmUnlink));
                        }
                    });

                    section(ui, app, t(lang, "settings.section.files"));
                    let archive = app.dirs.archive_db();
                    widgets::setting_row(
                        ui,
                        &palette,
                        t(lang, "settings.archive"),
                        &archive.display().to_string(),
                        |ui| {
                            if theme::soft_button(
                                ui,
                                &palette,
                                Some(Icon::ExternalLink),
                                t(lang, "settings.open_folder"),
                                false,
                            )
                            .clicked()
                            {
                                app.actions.push(Action::OpenFile(app.dirs.state.clone()));
                            }
                        },
                    );
                    let media = app.dirs.media_cache_dir();
                    widgets::setting_row(
                        ui,
                        &palette,
                        t(lang, "settings.media"),
                        &media.display().to_string(),
                        |ui| {
                            if theme::soft_button(
                                ui,
                                &palette,
                                Some(Icon::ExternalLink),
                                t(lang, "settings.open_folder"),
                                false,
                            )
                            .clicked()
                            {
                                let _ = std::fs::create_dir_all(&media);
                                app.actions.push(Action::OpenFile(media.clone()));
                            }
                        },
                    );
                    let log = app.dirs.log_file();
                    widgets::setting_row(
                        ui,
                        &palette,
                        t(lang, "settings.log"),
                        &log.display().to_string(),
                        |ui| {
                            if theme::soft_button(
                                ui,
                                &palette,
                                Some(Icon::FileText),
                                t(lang, "settings.open"),
                                false,
                            )
                            .clicked()
                            {
                                app.actions.push(Action::OpenFile(log.clone()));
                            }
                        },
                    );

                    section(ui, app, t(lang, "settings.section.about"));
                    widgets::setting_row(
                        ui,
                        &palette,
                        &format!("ZapFast {}", env!("CARGO_PKG_VERSION")),
                        t(lang, "settings.about.hint"),
                        |ui| {
                            if theme::soft_button(
                                ui,
                                &palette,
                                Some(Icon::Info),
                                t(lang, "settings.about"),
                                false,
                            )
                            .clicked()
                            {
                                app.actions.push(Action::ShowDialog(Dialog::About));
                            }
                            if theme::soft_button(
                                ui,
                                &palette,
                                Some(Icon::Keyboard),
                                t(lang, "settings.shortcuts"),
                                false,
                            )
                            .clicked()
                            {
                                app.actions.push(Action::ShowDialog(Dialog::Shortcuts));
                            }
                        },
                    );
                });
        });
}

fn theme_label(choice: ThemeChoice, lang: Language) -> &'static str {
    match choice {
        ThemeChoice::Dark => t(lang, "settings.theme.dark"),
        ThemeChoice::Light => t(lang, "settings.theme.light"),
        ThemeChoice::System => t(lang, "settings.theme.follow_system"),
    }
}

fn section(ui: &mut egui::Ui, app: &App, label: &str) {
    let palette = app.palette;
    ui.add_space(16.0);
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
        changed = widgets::switch(ui, &palette, &mut value).changed();
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
