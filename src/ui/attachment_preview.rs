//! Review staged files before sending them, without leaving the chat's composer.

use egui::{Align, Frame, Layout, Margin, Rect, Sense, Stroke, Vec2, vec2};

use crate::app::{App, Pending, Staged};
use crate::model::Action;
use crate::theme::{self, Icon};

use super::focus::{Stop, TabStop};
use super::widgets;

const TILE: f32 = 56.0;

/// Keep each chat and account's file selection independent.
fn selection_key(app: &App) -> egui::Id {
    egui::Id::new(("attachment-review", &app.account().id, &app.open_chat))
}

/// Resolve the selected attachment by identity after additions and removals.
fn selected(app: &App, ctx: &egui::Context) -> usize {
    let id = ctx.data(|data| data.get_temp::<u64>(selection_key(app)));
    app.pending
        .iter()
        .position(|item| Some(item.id) == id)
        .unwrap_or(0)
}

/// Name a file without exposing its directory.
fn file_name(item: &Pending) -> String {
    match item {
        Pending::Picture { .. } => String::new(),
        Pending::File(path) | Pending::Document(path) => path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default(),
    }
}

/// Only photos sent as photos may enter the cropper.
fn editable(item: &Pending) -> bool {
    match item {
        Pending::Picture { .. } => true,
        Pending::File(path) => Pending::is_picture_file(path),
        Pending::Document(_) => false,
    }
}

/// Reuses one bounded preview texture for a pasted picture, in its tile and on the stage.
fn image(ui: &egui::Ui, staged: &mut Staged) -> Option<egui::Image<'static>> {
    match &mut staged.item {
        Pending::Picture {
            width,
            height,
            rgba,
            texture,
        } => {
            if texture.is_none()
                && let Some(image) = &staged.preview
            {
                *texture = Some(ui.ctx().load_texture(
                    format!("pending-picture-{}", staged.id),
                    std::sync::Arc::clone(image),
                    egui::TextureOptions::LINEAR,
                ));
            }
            if texture.is_none() {
                // Larger buffers are resized by the backend before the UI sees them.
                if *width > crate::backend::picture_edit::PREVIEW_SIDE
                    || *height > crate::backend::picture_edit::PREVIEW_SIDE
                {
                    return None;
                }
                *texture = Some(ui.ctx().load_texture(
                    format!("pending-picture-{}", staged.id),
                    egui::ColorImage::from_rgba_unmultiplied([*width, *height], rgba),
                    egui::TextureOptions::LINEAR,
                ));
            }
            let handle = texture.as_ref().expect("prepared above");
            Some(egui::Image::from_texture(handle))
        }
        Pending::File(path) if Pending::is_picture_file(path) => {
            Some(widgets::file_image(ui, path))
        }
        _ => None,
    }
}

/// Bottom file strip and Send. The composer consumes the click through its normal send path.
pub fn bar(app: &mut App, ui: &mut egui::Ui) -> bool {
    let palette = app.palette;
    let selection_key = selection_key(app);
    let selected = selected(app, ui.ctx());
    let locale = app.locale;
    let mut send = false;
    let mut remove = None;
    let mut add = false;
    egui::Panel::bottom("attachment-review-strip")
        .show_separator_line(false)
        .frame(
            Frame::new()
                .fill(palette.surface)
                .inner_margin(Margin::same(10)),
        )
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let width = (ui.available_width() - 60.0).max(0.0);
                ui.allocate_ui_with_layout(
                    vec2(width, 72.0),
                    Layout::left_to_right(Align::Center),
                    |ui| {
                        egui::ScrollArea::horizontal()
                            .id_salt("pending-files")
                            .max_height(72.0)
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    for (index, item) in app.pending.iter_mut().enumerate() {
                                        let (clicked, removed) = tile(
                                            ui,
                                            &palette,
                                            item,
                                            index,
                                            index == selected,
                                            locale,
                                        );
                                        if removed {
                                            remove = Some(index);
                                        } else if clicked {
                                            ui.ctx().data_mut(|data| {
                                                data.insert_temp(selection_key, item.id)
                                            });
                                        }
                                    }
                                    let button = ui
                                        .add_sized(Vec2::splat(TILE), egui::Button::new("+"))
                                        .tab_stop(Stop::ReviewAdd);
                                    button.widget_info(|| {
                                        egui::WidgetInfo::labeled(
                                            egui::WidgetType::Button,
                                            true,
                                            crate::i18n::gettext(locale, "Add attachments"),
                                        )
                                    });
                                    add = button
                                        .on_hover_text(crate::i18n::gettext(
                                            locale,
                                            "Add attachments",
                                        ))
                                        .clicked();
                                });
                            });
                    },
                );
                send = theme::circle_button(
                    ui,
                    Icon::Send,
                    44.0,
                    palette.accent,
                    palette.accent_hover,
                    palette.on_accent,
                    "Send",
                )
                .tab_stop(Stop::Send)
                .clicked();
            });
        });
    if let Some(index) = remove {
        app.actions.push(Action::RemovePending(index));
    }
    if add {
        app.actions.push(Action::Attach { documents: false });
    }
    send
}

/// A visible tile selects its file; its corner button removes only that file.
fn tile(
    ui: &mut egui::Ui,
    palette: &theme::Palette,
    item: &mut Staged,
    index: usize,
    selected: bool,
    locale: crate::i18n::Locale,
) -> (bool, bool) {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(TILE), Sense::click());
    let response = response.tab_stop(Stop::ReviewFile(index));
    theme::reveal_focus(&response);
    let name = file_name(&item.item);
    let label = if name.is_empty() {
        crate::i18n::gettext(locale, "Photo {}").replace("{}", &(index + 1).to_string())
    } else {
        format!(
            "{}: {name}",
            crate::i18n::gettext(locale, "Attachment {}").replace("{}", &(index + 1).to_string())
        )
    };
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Button, true, selected, &label)
    });
    if !ui.is_rect_visible(rect) {
        return (false, false);
    }
    ui.painter().rect_filled(rect, 6.0, palette.surface_hover);
    if let Some(image) = image(ui, item) {
        image
            .fit_to_exact_size(rect.size() - Vec2::splat(8.0))
            .corner_radius(4.0)
            .paint_at(ui, rect.shrink(4.0));
    } else {
        theme::paint_icon(ui, Icon::FileText, rect, 24.0, palette.secondary);
    }
    if selected {
        ui.painter().rect_stroke(
            rect,
            6.0,
            Stroke::new(2.0, palette.accent),
            egui::StrokeKind::Inside,
        );
    }
    let mut removed = false;
    if response.hovered() || response.has_focus() || selected {
        let close = Rect::from_min_size(rect.right_top() + vec2(-20.0, 0.0), Vec2::splat(20.0));
        let close_response = ui
            .interact(close, ui.id().with(("unstage", item.id)), Sense::click())
            .tab_stop(Stop::ReviewRemove(index));
        close_response.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Button,
                true,
                crate::i18n::gettext(locale, "Remove attachment"),
            )
        });
        ui.painter()
            .circle_filled(close.center(), 10.0, palette.overlay);
        theme::paint_icon(ui, Icon::X, close, 12.0, palette.text);
        removed = close_response
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .clicked();
    }
    let clicked = response.clicked();
    response
        .on_hover_text(label)
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    (clicked, removed)
}

/// Large selected file, below the chat header and above its existing caption field.
pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    let selected = selected(app, ui.ctx());
    let Some(item) = app.pending.get_mut(selected) else {
        return;
    };
    let target = item.id;
    let can_edit = editable(&item.item);
    let name = file_name(&item.item);
    let mut edit = false;
    let mut close = false;
    ui.painter()
        .rect_filled(ui.available_rect_before_wrap(), 0.0, palette.surface);
    ui.horizontal(|ui| {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            close = theme::icon_button(
                ui,
                Icon::X,
                20.0,
                palette.secondary,
                palette.text,
                &crate::i18n::gettext(app.locale, "Discard attachments"),
            )
            .tab_stop(Stop::ReviewClose)
            .clicked();
            if can_edit {
                edit = theme::icon_button(
                    ui,
                    Icon::Pencil,
                    20.0,
                    palette.secondary,
                    palette.text,
                    &crate::i18n::gettext(app.locale, "Edit picture"),
                )
                .tab_stop(Stop::ReviewEdit)
                .clicked();
            }
        });
    });
    let rect = ui.available_rect_before_wrap().shrink(20.0);
    if rect.is_positive() {
        if let Some(image) = image(ui, item) {
            if let Ok(egui::load::TexturePoll::Ready { texture }) =
                image.load_for_size(ui.ctx(), rect.size())
            {
                let scale = (rect.width() / texture.size.x).min(rect.height() / texture.size.y);
                image.paint_at(
                    ui,
                    Rect::from_center_size(rect.center(), texture.size * scale),
                );
            } else {
                image.max_size(rect.size()).paint_at(ui, rect);
            }
        } else if matches!(item.item, Pending::Picture { .. }) {
            ui.put(
                Rect::from_center_size(rect.center(), Vec2::splat(24.0)),
                egui::Spinner::new(),
            );
        } else {
            let mut body = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(rect)
                    .layout(Layout::top_down(Align::Center)),
            );
            body.add_space((rect.height() / 2.0 - 65.0).max(0.0));
            let (icon, _) = body.allocate_exact_size(Vec2::splat(64.0), Sense::hover());
            theme::paint_icon(&body, Icon::FileText, icon, 54.0, palette.secondary);
            body.add(egui::Label::new(egui::RichText::new(name).color(palette.text)).wrap());
            let size = match &item.item {
                Pending::Picture { .. } => None,
                Pending::File(path) | Pending::Document(path) => file_size(ui.ctx(), path),
            };
            if let Some(size) = size {
                body.label(egui::RichText::new(crate::util::bytes(size)).color(palette.secondary));
            }
        }
    }
    if edit {
        app.actions.push(Action::EditPicture(target));
    }
    if close {
        app.actions.push(Action::ClearPending);
    }
}

/// File inspection happens off the UI thread; only the selected file's answer is retained.
#[derive(Clone)]
struct FileSize {
    path: std::path::PathBuf,
    size: std::sync::Arc<std::sync::OnceLock<Option<u64>>>,
}

fn file_size(ctx: &egui::Context, path: &std::path::Path) -> Option<u64> {
    let key = egui::Id::new("attachment-review-file-size");
    let current = ctx.data(|data| data.get_temp::<FileSize>(key));
    if let Some(current) = current.filter(|current| current.path == path) {
        return current.size.get().copied().flatten();
    }
    let size = std::sync::Arc::new(std::sync::OnceLock::new());
    ctx.data_mut(|data| {
        data.insert_temp(
            key,
            FileSize {
                path: path.to_owned(),
                size: size.clone(),
            },
        )
    });
    let path = path.to_owned();
    let ctx = ctx.clone();
    let _ = std::thread::Builder::new()
        .name("attachment-info".into())
        .spawn(move || {
            let _ = size.set(std::fs::metadata(path).ok().map(|info| info.len()));
            ctx.request_repaint();
        });
    None
}
