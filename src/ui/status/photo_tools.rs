//! One canvas for markup, effects and typography; pixels are prepared off the UI thread.
use super::*;
use crate::photo::{Editor, Effect, Mark, Tool};

pub(super) fn font_menu(ui: &mut egui::Ui, font: &mut usize, locale: crate::i18n::Locale) {
    let response = theme::icon_button(
        ui,
        Icon::Type,
        20.0,
        ui.visuals().text_color(),
        ui.visuals().text_color(),
        &gettext(locale, "Font style"),
    );
    egui::Popup::menu(&response).show(|ui| {
        // Literal calls, so xgettext extracts each label.
        let labels = [
            gettext(locale, "Regular"),
            gettext(locale, "Bold"),
            gettext(locale, "Monospace"),
        ];
        for (index, label) in labels.into_iter().enumerate() {
            if ui
                .selectable_label(
                    *font == index,
                    RichText::new(label).font(crate::photo::font(index, 16.0)),
                )
                .clicked()
            {
                *font = index;
                ui.close();
            }
        }
    });
}

/// A tool's icon and label. The shapes share one button. Literal calls, so
/// xgettext extracts each.
pub(super) fn tool_face(
    tool: Tool,
    locale: crate::i18n::Locale,
) -> (Icon, std::borrow::Cow<'static, str>) {
    match tool {
        Tool::Pen => (Icon::Pencil, gettext(locale, "Draw")),
        Tool::Text => (Icon::Type, gettext(locale, "Text")),
        Tool::Rectangle | Tool::Ellipse | Tool::Arrow => (Icon::Shapes, gettext(locale, "Shapes")),
        Tool::Blur => (Icon::Blur, gettext(locale, "Blur")),
        Tool::Effects => (Icon::Effects, gettext(locale, "Effects")),
    }
}

fn tool_button(
    ui: &mut egui::Ui,
    tool: Tool,
    current: Tool,
    palette: &theme::Palette,
    locale: crate::i18n::Locale,
) -> bool {
    let (icon, label) = tool_face(tool, locale);
    let color = if current == tool {
        palette.accent
    } else {
        palette.text
    };
    theme::icon_button(ui, icon, 20.0, color, color, &label).clicked()
}

pub(super) fn show(app: &mut App, ui: &mut egui::Ui) {
    let locale = app.locale;
    let palette = app.palette;
    let account = app.account().id.clone();
    let mut actions = Vec::new();
    let mut close = false;
    let mut bottom = ui.ctx().content_rect().bottom();
    egui::CentralPanel::default()
        .frame(
            Frame::new()
                .fill(palette.window)
                .inner_margin(Margin::same(20)),
        )
        .show(ui, |ui| {
            // The panel's own bottom, above the call strip when one shows.
            bottom = ui.max_rect().bottom() + 20.0;
            header_room(ui);
            let full = ui.available_rect_before_wrap();
            let Some(editor) = app
                .status_draft
                .as_mut()
                .and_then(|draft| draft.editor.as_mut())
            else {
                return;
            };
            if !editor.applying && !ui.ctx().text_edit_focused() {
                let redo = ui.input_mut(|input| {
                    input.consume_key(
                        egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                        egui::Key::Z,
                    ) || input.consume_key(egui::Modifiers::COMMAND, egui::Key::Y)
                });
                let undo =
                    ui.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::Z));
                if redo {
                    editor.history(true);
                } else if undo {
                    editor.history(false);
                }
            }
            ui.add_enabled_ui(!editor.applying, |ui| {
                ui.horizontal_wrapped(|ui| {
                    if super::compose::button(ui, Icon::X, &gettext(locale, "Cancel photo edits"))
                        .clicked()
                    {
                        close = true;
                    }
                    if ui
                        .add_enabled_ui(!editor.undo.is_empty(), |ui| {
                            super::compose::button(ui, Icon::Undo, &gettext(locale, "Undo"))
                        })
                        .inner
                        .clicked()
                    {
                        editor.history(false);
                    }
                    if ui
                        .add_enabled_ui(!editor.redo.is_empty(), |ui| {
                            super::compose::button(ui, Icon::Redo, &gettext(locale, "Redo"))
                        })
                        .inner
                        .clicked()
                    {
                        editor.history(true);
                    }
                    ui.separator();
                    if super::compose::button(
                        ui,
                        Icon::Crop,
                        &gettext(locale, "Crop and rotate photo"),
                    )
                    .clicked()
                    {
                        actions.push(Action::ApplyStatusPhoto {
                            account: account.clone(),
                            crop: true,
                        });
                    }
                    for tool in [
                        Tool::Effects,
                        Tool::Pen,
                        Tool::Text,
                        Tool::Rectangle,
                        Tool::Blur,
                    ] {
                        if tool_button(ui, tool, editor.tool, &palette, locale) {
                            editor.tool = tool;
                            editor.gesture.clear();
                        }
                    }
                    if ui
                        .add_enabled_ui(!editor.scene.marks.is_empty(), |ui| {
                            super::compose::button(
                                ui,
                                Icon::Eraser,
                                &gettext(locale, "Remove last object"),
                            )
                        })
                        .inner
                        .clicked()
                    {
                        editor.remember();
                        editor.scene.marks.pop();
                    }
                });
                ui.add_space(8.0);
                controls(ui, editor, &palette, locale);
            });
            let canvas = Rect::from_min_max(
                pos2(full.left(), ui.cursor().top() + 10.0),
                pos2(full.right(), full.bottom() - 66.0),
            );
            if let Some(image) = &editor.preview {
                let key = egui::Id::new("status-photo-preview-texture");
                let cached = ui.ctx().data(|data| {
                    data.get_temp::<(crate::model::AccountId, u64, u64, egui::TextureHandle)>(key)
                });
                let revision = editor.preview_revision.unwrap_or_default();
                let texture = match cached {
                    Some((owner, target, version, texture))
                        if owner == account && target == editor.target && version == revision =>
                    {
                        texture
                    }
                    _ => {
                        let texture = ui.ctx().load_texture(
                            "Status photo preview",
                            image.as_ref().clone(),
                            egui::TextureOptions::LINEAR,
                        );
                        ui.ctx().data_mut(|data| {
                            data.insert_temp(
                                key,
                                (account.clone(), editor.target, revision, texture.clone()),
                            )
                        });
                        texture
                    }
                };
                let scale = (canvas.width() / image.size[0] as f32)
                    .min(canvas.height() / image.size[1] as f32)
                    .max(0.01);
                let photo = Rect::from_center_size(
                    canvas.center(),
                    vec2(image.size[0] as f32, image.size[1] as f32) * scale,
                );
                ui.painter().image(
                    texture.id(),
                    photo,
                    Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
                let response = ui
                    .interact(
                        photo,
                        egui::Id::new("status-photo-canvas"),
                        Sense::click_and_drag(),
                    )
                    .on_hover_cursor(if editor.tool == Tool::Text {
                        egui::CursorIcon::Text
                    } else {
                        egui::CursorIcon::Crosshair
                    });
                response.widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::WidgetType::Image,
                        !editor.applying,
                        gettext(locale, "Photo editing canvas"),
                    )
                });
                if !editor.applying {
                    gesture(editor, &response, photo);
                }
                if !editor.gesture.is_empty() {
                    paint_gesture(ui, editor, photo);
                }
                if editor.requested.is_some() {
                    ui.put(
                        Rect::from_min_size(canvas.left_top(), vec2(20.0, 20.0)),
                        egui::Spinner::new(),
                    );
                }
            } else {
                ui.put(canvas, egui::Spinner::new());
            }
            let footer = Rect::from_min_max(pos2(full.left(), full.bottom() - 48.0), full.max);
            ui.scope_builder(egui::UiBuilder::new().max_rect(footer), |ui| {
                ui.horizontal(|ui| {
                    if let Some([w, h]) = editor.dimensions {
                        ui.label(RichText::new(format!("{w} × {h}")).color(palette.secondary));
                    }
                    if editor.limit_reached || editor.scene.marks.len() >= crate::photo::MAX_MARKS {
                        ui.label(gettext(locale, "Photo edit limit reached"));
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if editor.applying {
                            ui.spinner();
                        }
                        if ui
                            .add_enabled(
                                !editor.applying && editor.preview.is_some(),
                                egui::Button::new(gettext(locale, "Done")),
                            )
                            .clicked()
                        {
                            actions.push(Action::ApplyStatusPhoto {
                                account: account.clone(),
                                crop: false,
                            });
                        }
                    });
                });
            });
            if !editor.applying
                && editor.requested.is_none()
                && editor.preview_revision != Some(editor.revision)
            {
                actions.push(Action::StatusPhotoPreview {
                    account: account.clone(),
                });
            }
            ui.allocate_rect(full, Sense::hover());
        });
    if let Some(error) = app
        .status_draft
        .as_ref()
        .and_then(|draft| draft.error.as_ref())
    {
        egui::Area::new(egui::Id::new("status-photo-error"))
            .order(egui::Order::Foreground)
            .fixed_pos(pos2(24.0, bottom - 92.0))
            .show(ui.ctx(), |ui| {
                ui.label(RichText::new(error).color(palette.text));
            });
    }
    if close && let Some(draft) = app.status_draft.as_mut() {
        draft.editor = None;
        draft.error = None;
    }
    app.actions.extend(actions);
}

fn controls(
    ui: &mut egui::Ui,
    editor: &mut Editor,
    palette: &theme::Palette,
    locale: crate::i18n::Locale,
) {
    ui.horizontal_wrapped(|ui| {
        if editor.tool == Tool::Effects {
            // Literal calls, so xgettext extracts each label.
            for (effect, label) in [
                (Effect::Original, gettext(locale, "Original")),
                (Effect::Mono, gettext(locale, "Black and white")),
                (Effect::Warm, gettext(locale, "Warm")),
                (Effect::Cool, gettext(locale, "Cool")),
                (Effect::Vivid, gettext(locale, "Vivid")),
            ] {
                if ui
                    .selectable_label(editor.scene.effect == effect, label)
                    .clicked()
                    && editor.scene.effect != effect
                {
                    editor.remember();
                    editor.scene.effect = effect;
                }
            }
            return;
        }
        if matches!(editor.tool, Tool::Rectangle | Tool::Ellipse | Tool::Arrow) {
            for (tool, label) in [
                (Tool::Rectangle, gettext(locale, "Rectangle")),
                (Tool::Ellipse, gettext(locale, "Ellipse")),
                (Tool::Arrow, gettext(locale, "Arrow")),
            ] {
                if ui.selectable_label(editor.tool == tool, label).clicked() {
                    editor.tool = tool;
                }
            }
            widgets::checkbox(ui, palette, &mut editor.filled, &gettext(locale, "Fill"));
        }
        if editor.tool != Tool::Blur {
            for color in [
                Color32::WHITE,
                Color32::BLACK,
                Color32::from_rgb(255, 75, 86),
                Color32::from_rgb(255, 204, 0),
                Color32::from_rgb(30, 199, 103),
                Color32::from_rgb(65, 150, 255),
                Color32::from_rgb(194, 103, 234),
            ] {
                let (rect, response) = ui.allocate_exact_size(vec2(24.0, 28.0), Sense::click());
                ui.painter().circle_filled(rect.center(), 8.0, color);
                if editor.color == color {
                    ui.painter().circle_stroke(
                        rect.center(),
                        11.0,
                        Stroke::new(1.5, palette.accent),
                    );
                }
                let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
                response.widget_info(|| {
                    egui::WidgetInfo::selected(
                        egui::WidgetType::RadioButton,
                        true,
                        editor.color == color,
                        format!("#{:02X}{:02X}{:02X}", color.r(), color.g(), color.b()),
                    )
                });
                theme::focus_outline(ui, response.id, rect, 12.0);
                if response.clicked() {
                    editor.color = color;
                }
            }
            ui.color_edit_button_srgba(&mut editor.color)
                .on_hover_text(gettext(locale, "Custom colour"));
            ui.end_row();
        }
        match editor.tool {
            Tool::Text => {
                font_menu(ui, &mut editor.font, locale);
                ui.add(
                    egui::Slider::new(&mut editor.text_size, 0.015..=0.2)
                        .show_value(false)
                        .text(gettext(locale, "Text size")),
                );
                widgets::checkbox(ui, palette, &mut editor.boxed, &gettext(locale, "Text box"));
            }
            Tool::Pen | Tool::Rectangle | Tool::Ellipse | Tool::Arrow => {
                ui.add(
                    egui::Slider::new(&mut editor.width, 0.001..=0.05)
                        .show_value(false)
                        .text(gettext(locale, "Size")),
                );
                if editor.tool == Tool::Pen {
                    widgets::checkbox(
                        ui,
                        palette,
                        &mut editor.highlighter,
                        &gettext(locale, "Highlighter"),
                    );
                }
            }
            Tool::Blur => {
                ui.label(gettext(locale, "Drag across the area to blur"));
            }
            Tool::Effects => {}
        }
    });
    if editor.tool == Tool::Text {
        ui.horizontal(|ui| {
            let width = (ui.available_width() - 110.0).max(80.0);
            ui.add(
                egui::TextEdit::singleline(&mut editor.text)
                    .hint_text(gettext(locale, "Type text, then click the photo"))
                    .desired_width(width)
                    .char_limit(700),
            );
            if ui
                .add_enabled(
                    !editor.text.trim().is_empty(),
                    egui::Button::new(gettext(locale, "Add text")),
                )
                .clicked()
            {
                add_text(editor, pos2(0.5, 0.5));
            }
        });
    }
}

fn add_text(editor: &mut Editor, at: Pos2) {
    if !editor.text.trim().is_empty() {
        editor.add(Mark::Text {
            at,
            text: editor.text.clone(),
            font: editor.font,
            size: editor.text_size,
            color: editor.color,
            boxed: editor.boxed,
        });
    }
}

fn gesture(editor: &mut Editor, response: &egui::Response, photo: Rect) {
    if editor.tool == Tool::Effects {
        return;
    }
    let normalized = |point: Pos2| {
        pos2(
            ((point.x - photo.left()) / photo.width()).clamp(0.0, 1.0),
            ((point.y - photo.top()) / photo.height()).clamp(0.0, 1.0),
        )
    };
    if editor.tool == Tool::Text {
        if response.clicked()
            && let Some(point) = response.interact_pointer_pos()
        {
            add_text(editor, normalized(point));
        }
        return;
    }
    if response.is_pointer_button_down_on()
        && editor.gesture.is_empty()
        && let Some(point) = response.interact_pointer_pos()
    {
        editor.gesture = vec![normalized(point)];
    }
    if response.dragged()
        && let Some(point) = response.interact_pointer_pos()
    {
        let point = normalized(point);
        if editor.tool == Tool::Pen {
            if editor.gesture.len() < 4096
                && editor
                    .gesture
                    .last()
                    .is_none_or(|last| last.distance(point) > 0.002)
            {
                editor.gesture.push(point);
            }
        } else if editor.gesture.len() == 1 {
            editor.gesture.push(point);
        } else if let Some(last) = editor.gesture.last_mut() {
            *last = point;
        }
    }
    if response.clicked()
        && let Some(point) = response.interact_pointer_pos()
    {
        editor.gesture = vec![normalized(point)];
    }
    if response.drag_stopped() || response.clicked() {
        let points = std::mem::take(&mut editor.gesture);
        if editor.tool != Tool::Pen && points.len() < 2 {
            return;
        }
        if let Some(&from) = points.first() {
            let to = points.last().copied().unwrap_or(from);
            let color = if editor.highlighter && editor.tool == Tool::Pen {
                editor.color.gamma_multiply(0.35)
            } else {
                editor.color
            };
            let mark = match editor.tool {
                Tool::Pen => Mark::Pen {
                    points,
                    color,
                    width: editor.width,
                },
                Tool::Blur => Mark::Blur { from, to },
                tool => Mark::Shape {
                    tool,
                    from,
                    to,
                    color,
                    width: editor.width,
                    filled: editor.filled,
                },
            };
            editor.add(mark);
        }
    }
}

fn paint_gesture(ui: &egui::Ui, editor: &Editor, photo: Rect) {
    let point = |p: Pos2| photo.min + vec2(p.x * photo.width(), p.y * photo.height());
    let Some(&from) = editor.gesture.first() else {
        return;
    };
    let to = editor.gesture.last().copied().unwrap_or(from);
    let color = if editor.highlighter && editor.tool == Tool::Pen {
        editor.color.gamma_multiply(0.35)
    } else {
        editor.color
    };
    let stroke = Stroke::new(editor.width * photo.size().min_elem(), color);
    let painter = ui.painter().with_clip_rect(photo);
    match editor.tool {
        Tool::Pen => {
            painter.add(egui::Shape::line(
                editor.gesture.iter().map(|p| point(*p)).collect(),
                stroke,
            ));
        }
        Tool::Arrow => {
            painter.arrow(point(from), point(to) - point(from), stroke);
        }
        Tool::Ellipse => {
            let rect = Rect::from_two_pos(point(from), point(to));
            painter.add(egui::epaint::EllipseShape::stroke(
                rect.center(),
                rect.size() / 2.0,
                stroke,
            ));
        }
        _ => {
            painter.rect_stroke(
                Rect::from_two_pos(point(from), point(to)),
                0.0,
                stroke,
                egui::StrokeKind::Inside,
            );
        }
    }
}
