use super::*;
use crate::status::BACKGROUNDS;

pub(super) fn show(app: &mut App, ui: &mut egui::Ui) {
    if app
        .status_draft
        .as_ref()
        .is_some_and(|draft| draft.editor.is_some())
    {
        super::photo_tools::show(app, ui);
        return;
    }
    let draft = app
        .status_draft
        .as_ref()
        .expect("the status editor has a draft")
        .clone();
    let locale = app.locale;
    let account = app.account().id.clone();
    let fill = if draft.media.is_some() {
        app.palette.window
    } else {
        argb(BACKGROUNDS[draft.background % BACKGROUNDS.len()])
    };
    // Text and icons over a coloured background follow its lightness.
    let ink = if fill.intensity() > 0.5 {
        Color32::BLACK
    } else {
        Color32::WHITE
    };
    egui::CentralPanel::default()
        .frame(Frame::new().fill(fill).inner_margin(Margin::same(20)))
        .show(ui, |ui| {
            if draft.media.is_none() {
                ui.visuals_mut().override_text_color = Some(ink);
            }
            // The panel's own bottom, above the call strip when one shows.
            let panel = ui.max_rect().expand(20.0);
            header_room(ui);
            let full = ui.available_rect_before_wrap();
            let editable = !draft.busy && !app.status_picture_busy();
            ui.horizontal(|ui| {
                if ui
                    .add_enabled_ui(!draft.busy, |ui| {
                        button(ui, Icon::X, &gettext(locale, "Close"))
                    })
                    .inner
                    .clicked()
                {
                    app.status_draft = None;
                }
                if draft.media.is_some() {
                    ui.add_space(if full.width() > 700.0 { 64.0 } else { 8.0 });
                    // Literal calls, so xgettext extracts each label.
                    for (redo, icon, label, ready) in [
                        (
                            false,
                            Icon::Undo,
                            gettext(locale, "Undo photo edit"),
                            !draft.undo.is_empty(),
                        ),
                        (
                            true,
                            Icon::Redo,
                            gettext(locale, "Redo photo edit"),
                            !draft.redo.is_empty(),
                        ),
                    ] {
                        if ui
                            .add_enabled_ui(editable && ready, |ui| button(ui, icon, &label))
                            .inner
                            .clicked()
                        {
                            app.actions.push(Action::StatusPictureHistory {
                                account: account.clone(),
                                redo,
                            });
                        }
                    }
                } else {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if button(ui, Icon::Palette, &gettext(locale, "Background colour"))
                            .clicked()
                            && editable
                            && let Some(draft) = app.status_draft.as_mut()
                        {
                            draft.background = (draft.background + 1) % BACKGROUNDS.len();
                        }
                        if let Some(draft) = app.status_draft.as_mut() {
                            super::photo_tools::font_menu(ui, &mut draft.font, locale);
                        }
                        emoji_menu(app, ui);
                    });
                }
            });
            if let Some(path) = &draft.media {
                let tools = Rect::from_center_size(
                    pos2(
                        full.center().x,
                        full.top() + if full.width() < 760.0 { 60.0 } else { 16.0 },
                    ),
                    vec2(
                        if full.width() > 620.0 {
                            272.0
                        } else {
                            full.width() - 24.0
                        },
                        32.0,
                    ),
                );
                ui.scope_builder(egui::UiBuilder::new().max_rect(tools), |ui| {
                    ui.horizontal(|ui| {
                        if crate::app::Pending::is_picture_file(path)
                            && ui
                                .add_enabled_ui(editable, |ui| {
                                    button(
                                        ui,
                                        Icon::Crop,
                                        &gettext(locale, "Crop and rotate photo"),
                                    )
                                })
                                .inner
                                .clicked()
                        {
                            app.actions.push(Action::EditStatusPicture {
                                account: account.clone(),
                            });
                        }
                        if crate::app::Pending::is_picture_file(path) {
                            for tool in [
                                crate::photo::Tool::Effects,
                                crate::photo::Tool::Pen,
                                crate::photo::Tool::Text,
                                crate::photo::Tool::Rectangle,
                                crate::photo::Tool::Blur,
                            ] {
                                let (icon, label) = super::photo_tools::tool_face(tool, locale);
                                if ui
                                    .add_enabled_ui(editable, |ui| button(ui, icon, &label))
                                    .inner
                                    .clicked()
                                {
                                    app.actions.push(Action::StatusPhotoTool {
                                        account: account.clone(),
                                        tool,
                                    });
                                }
                            }
                            let hd = ui
                                .add_enabled_ui(editable, |ui| {
                                    button(ui, Icon::Hd, &gettext(locale, "Photo quality"))
                                })
                                .inner;
                            egui::Popup::menu(&hd).show(|ui| {
                                if let Some(draft) = app.status_draft.as_mut() {
                                    ui.radio_value(
                                        &mut draft.quality,
                                        crate::photo::Quality::Standard,
                                        gettext(locale, "Standard · up to 1600 px"),
                                    );
                                    ui.radio_value(
                                        &mut draft.quality,
                                        crate::photo::Quality::Hd,
                                        gettext(locale, "HD · up to 4096 px"),
                                    );
                                }
                            });
                        }
                    });
                });
            }
            let editor = Rect::from_min_max(
                pos2(
                    full.left() + 24.0,
                    full.top() + if full.width() < 760.0 { 110.0 } else { 70.0 },
                ),
                pos2(
                    full.right() - 24.0,
                    full.bottom()
                        - if draft.media.is_some() && full.width() < 620.0 {
                            190.0
                        } else {
                            110.0
                        },
                ),
            );
            if let Some(path) = &draft.media {
                ui.scope_builder(
                    egui::UiBuilder::new()
                        .max_rect(editor)
                        .layout(Layout::centered_and_justified(egui::Direction::TopDown)),
                    |ui| {
                        if path
                            .extension()
                            .is_some_and(|ext| ext.eq_ignore_ascii_case("mp4"))
                        {
                            let key = format!("status:compose:{}", draft.token);
                            app.video.saw(&key);
                            if let Some(frame) =
                                app.video.status(&key).and_then(|status| status.frame)
                            {
                                ui.add(
                                    egui::Image::from_texture(&frame)
                                        .max_size(editor.size())
                                        .shrink_to_fit(),
                                );
                            } else if ui.button(gettext(locale, "Preview video")).clicked() {
                                app.actions.push(Action::PlayVideo {
                                    message: key,
                                    path: path.clone(),
                                });
                            }
                        } else {
                            ui.add(
                                widgets::file_image(ui, path)
                                    .max_size(editor.size())
                                    .shrink_to_fit(),
                            );
                        }
                    },
                );
                let caption = Rect::from_center_size(
                    pos2(
                        full.center().x,
                        panel.bottom() - if full.width() < 620.0 { 164.0 } else { 100.0 },
                    ),
                    vec2((full.width() - 24.0).min(520.0), 38.0),
                );
                ui.scope_builder(egui::UiBuilder::new().max_rect(caption), |ui| {
                    input_frame(&app.palette)
                        .inner_margin(Margin::symmetric(12, 2))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                let width = (ui.available_width() - 40.0).max(60.0);
                                if let Some(draft) = app.status_draft.as_mut() {
                                    ui.add_enabled(
                                        !draft.busy,
                                        egui::TextEdit::singleline(&mut draft.text)
                                            .hint_text(gettext(locale, "Add a caption"))
                                            .frame(Frame::NONE)
                                            .desired_width(width)
                                            .char_limit(700),
                                    );
                                }
                                emoji_menu(app, ui);
                            });
                        });
                });
            } else {
                let width = editor.width().min(1000.0);
                let size = if draft.text.chars().count() < 100 {
                    42.0
                } else if draft.text.chars().count() < 300 {
                    32.0
                } else {
                    24.0
                };
                let rows = draft
                    .text
                    .lines()
                    .map(|line| {
                        (line.chars().count() as f32 * size * 0.55 / width)
                            .ceil()
                            .max(1.0)
                    })
                    .sum::<f32>()
                    .max(1.0);
                let text_rect = Rect::from_center_size(
                    full.center(),
                    vec2(width, (rows * size * 1.3 + 12.0).min(editor.height())),
                );
                ui.scope_builder(egui::UiBuilder::new().max_rect(text_rect), |ui| {
                    if let Some(draft) = app.status_draft.as_mut() {
                        let font = crate::photo::font(draft.font, size);
                        ui.add_enabled(
                            !draft.busy,
                            egui::TextEdit::multiline(&mut draft.text)
                                .hint_text(
                                    RichText::new(gettext(locale, "Type a status update"))
                                        .font(font.clone())
                                        .color(ink.gamma_multiply(0.45)),
                                )
                                .font(font)
                                .text_color(ink)
                                .frame(Frame::NONE)
                                .desired_width(width)
                                .desired_rows(
                                    (rows as usize)
                                        .min((editor.height() / (size * 1.3)).floor().max(1.0)
                                            as usize),
                                )
                                .horizontal_align(Align::Center)
                                .char_limit(700),
                        );
                    }
                });
            }
            footer(app, ui, &draft, full, panel);
            ui.allocate_rect(full, Sense::hover());
        });
}

pub(super) fn argb(color: u32) -> Color32 {
    Color32::from_rgb((color >> 16) as u8, (color >> 8) as u8, color as u8)
}

pub(super) fn button(ui: &mut egui::Ui, icon: Icon, label: &str) -> egui::Response {
    let color = ui.visuals().text_color();
    theme::icon_button(ui, icon, 20.0, color, color, label)
}

fn emoji_menu(app: &mut App, ui: &mut egui::Ui) {
    let emoji = button(ui, Icon::Smile, &gettext(app.locale, "Emoji"));
    if emoji.clicked() {
        crate::ui::picker::open_status_emoji(ui.ctx());
    }
    egui::Popup::menu(&emoji).show(|ui| {
        ui.set_min_size(vec2(360.0, 280.0));
        ui.set_max_size(vec2(360.0, 280.0));
        if let Some(emoji) = crate::ui::picker::status_emoji(app, ui)
            && let Some(draft) = app.status_draft.as_mut().filter(|draft| !draft.busy)
        {
            draft.text.push_str(&emoji);
            ui.close();
        }
    });
}

fn footer(app: &mut App, ui: &mut egui::Ui, draft: &crate::status::Draft, full: Rect, panel: Rect) {
    let palette = app.palette;
    let locale = app.locale;
    let account = app.account().id.clone();
    let bar = Rect::from_min_max(pos2(panel.left(), panel.bottom() - 76.0), panel.max);
    if draft.media.is_some() {
        ui.painter().rect_filled(bar, 0.0, palette.window);
        ui.painter()
            .hline(bar.x_range(), bar.top(), Stroke::new(1.0, palette.outline));
    }
    let busy = draft.busy || app.status_picture_busy();
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(bar.shrink2(vec2(20.0, 14.0)))
            .layout(Layout::left_to_right(Align::Center)),
        |ui| {
            if ui
                .add_enabled_ui(!busy, |ui| audience_button(app, ui))
                .inner
                .clicked()
            {
                app.status_privacy_open = true;
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if busy {
                    ui.spinner();
                }
                let ready = !busy && (draft.media.is_some() || !draft.text.trim().is_empty());
                if ui
                    .add_enabled_ui(ready, |ui| {
                        theme::circle_button(
                            ui,
                            Icon::Send,
                            48.0,
                            palette.accent,
                            palette.accent_hover,
                            palette.on_accent,
                            &gettext(locale, "Publish status"),
                        )
                    })
                    .inner
                    .clicked()
                {
                    app.actions.push(Action::PublishStatus {
                        account: account.clone(),
                    });
                }
            });
        },
    );
    if let Some(path) = &draft.media {
        let centre = pos2(
            full.center().x,
            if full.width() < 620.0 {
                bar.top() - 34.0
            } else {
                bar.center().y
            },
        );
        ui.scope_builder(
            egui::UiBuilder::new().max_rect(Rect::from_center_size(centre, vec2(120.0, 56.0))),
            |ui| {
                ui.horizontal(|ui| {
                    ui.add_enabled_ui(!busy, |ui| {
                        let mut staged = crate::app::Staged {
                            id: draft.token,
                            item: crate::app::Pending::File(path.clone()),
                            preview: None,
                        };
                        let (_, removed) = crate::ui::attachment_preview::tile(
                            ui,
                            &palette,
                            &mut staged,
                            0,
                            true,
                            locale,
                        );
                        if removed {
                            app.actions.push(Action::RemoveStatusMedia {
                                account: account.clone(),
                            });
                        }
                        let add = crate::ui::attachment_preview::add_tile(ui, &palette);
                        add.widget_info(|| {
                            egui::WidgetInfo::labeled(
                                egui::WidgetType::Button,
                                true,
                                gettext(locale, "Choose another photo or video"),
                            )
                        });
                        if add
                            .on_hover_text(gettext(locale, "Choose another photo or video"))
                            .clicked()
                        {
                            app.actions.push(Action::ComposeStatus {
                                account: account.clone(),
                                media: true,
                            });
                        }
                    });
                });
            },
        );
    }
    if let Some(error) = &draft.error {
        let rect = Rect::from_center_size(
            pos2(full.center().x, bar.top() - 18.0),
            vec2(full.width().min(740.0), 32.0),
        );
        ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
            ui.label(RichText::new(error).color(palette.text));
        });
    }
}
