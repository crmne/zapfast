//! Review staged files before sending them, without leaving the chat's composer.

use egui::{Align, Color32, Frame, Layout, Margin, Rect, Sense, Stroke, Vec2, pos2, vec2};

use crate::app::{App, AttachmentId, Pending, Staged};
use crate::model::Action;
use crate::theme::{self, Icon};

use super::conversation::{VideoControls, fit_within, paint_texture, video_controls};
use super::focus::{Stop, TabStop};
use super::widgets;

const TILE: f32 = 56.0;

/// What the video player calls a staged video; no WhatsApp message id
/// starts this way.
const VIDEO_KEY: &str = "staged-attachment-";

/// The video player's name for the staged video `id`.
pub(crate) fn video_key(id: AttachmentId) -> String {
    format!("{VIDEO_KEY}{id}")
}

/// The staged attachment a video player's name stands for, if it is one.
pub(crate) fn video_attachment(key: &str) -> Option<AttachmentId> {
    key.strip_prefix(VIDEO_KEY)?.parse().ok()
}

/// The attachment the review shows large, if any is staged.
pub(crate) fn selected_id(app: &App, ctx: &egui::Context) -> Option<AttachmentId> {
    app.pending.get(selected(app, ctx)).map(|staged| staged.id)
}

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

/// Only what goes as a video message is previewed and played as a video.
fn is_video(item: &Pending) -> bool {
    matches!(item, Pending::File(path) if Pending::is_video_file(path))
}

/// A staged video's still, uploaded once by the window that shows it, for
/// its tile and the stage alike.
fn still(ui: &egui::Ui, staged: &mut Staged) -> Option<egui::TextureHandle> {
    let video = staged.video.as_mut()?;
    if video.texture.is_none()
        && let Some(image) = &staged.preview
    {
        video.texture = Some(ui.ctx().load_texture(
            format!("pending-video-{}", staged.id),
            std::sync::Arc::clone(image),
            egui::TextureOptions::LINEAR,
        ));
    }
    video.texture.clone()
}

/// How long a staged video plays, as its tile and stage say it: in whole
/// seconds, as the player's controls count it, so the two agree.
fn length_label(staged: &Staged) -> Option<String> {
    let length = staged.video.as_ref()?.length?;
    Some(crate::util::duration(length.as_secs() as u32))
}

/// A tile's sign that it holds a video, with the video's length once known.
fn video_mark(ui: &egui::Ui, rect: Rect, length: Option<String>) {
    let galley = length.map(|length| {
        ui.painter()
            .layout_no_wrap(length, theme::medium(10.0), Color32::WHITE)
    });
    let width = galley
        .as_ref()
        .map_or(18.0, |galley| 21.0 + galley.size().x);
    let chip = Rect::from_min_size(
        pos2(rect.left() + 3.0, rect.bottom() - 17.0),
        vec2(width, 14.0),
    );
    ui.painter()
        .rect_filled(chip, 7.0, Color32::from_black_alpha(150));
    theme::paint_icon(
        ui,
        Icon::Video,
        Rect::from_min_size(chip.min + vec2(3.0, 1.0), Vec2::splat(12.0)),
        10.0,
        Color32::WHITE,
    );
    if let Some(galley) = galley {
        let at = pos2(chip.left() + 17.0, chip.center().y - galley.size().y / 2.0);
        ui.painter().galley(at, galley, Color32::WHITE);
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
        .resizable(false)
        .exact_size(TILE + 12.0)
        .frame(
            Frame::new()
                .fill(palette.surface)
                // In line with the caption field above it.
                .inner_margin(Margin {
                    left: 8,
                    right: 8,
                    top: 2,
                    bottom: 10,
                }),
        )
        .show(ui, |ui| {
            // Rows as tall as a tile: a shorter centred row pushes tiles down
            // and off the bottom of the panel.
            let row = vec2(ui.available_width(), TILE);
            ui.allocate_ui_with_layout(row, Layout::left_to_right(Align::Center), |ui| {
                let width = (ui.available_width() - 60.0).max(0.0);
                ui.allocate_ui_with_layout(
                    vec2(width, TILE),
                    Layout::left_to_right(Align::Center),
                    |ui| {
                        egui::ScrollArea::horizontal()
                            .id_salt("pending-files")
                            .max_height(TILE)
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                ui.with_layout(Layout::left_to_right(Align::Min), |ui| {
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
                                    let button = add_tile(ui, &palette);
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

/// The tile after the files, the same size, that adds another.
fn add_tile(ui: &mut egui::Ui, palette: &theme::Palette) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(TILE), Sense::click());
    let response = response.tab_stop(Stop::ReviewAdd);
    theme::reveal_focus(&response);
    if ui.is_rect_visible(rect) {
        let hovered = response.hovered() || response.has_focus();
        ui.painter().rect(
            rect,
            6.0,
            if hovered {
                palette.surface_hover
            } else {
                egui::Color32::TRANSPARENT
            },
            Stroke::new(1.0, palette.secondary.gamma_multiply(0.5)),
            egui::StrokeKind::Inside,
        );
        theme::paint_icon(
            ui,
            Icon::Plus,
            Rect::from_center_size(rect.center(), Vec2::splat(22.0)),
            22.0,
            if hovered {
                palette.text
            } else {
                palette.secondary
            },
        );
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
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
    } else if is_video(&item.item) {
        match still(ui, item) {
            Some(texture) => {
                egui::Image::from_texture(&texture)
                    .fit_to_exact_size(rect.size() - Vec2::splat(8.0))
                    .corner_radius(4.0)
                    .paint_at(ui, rect.shrink(4.0));
                video_mark(ui, rect.shrink(4.0), length_label(item));
            }
            None => theme::paint_icon(ui, Icon::Video, rect, 24.0, palette.secondary),
        }
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
    let mut actions = Vec::new();
    ui.painter()
        .rect_filled(ui.available_rect_before_wrap(), 0.0, palette.surface);
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.add_space(8.0);
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
        } else if matches!(item.item, Pending::Picture { .. })
            || (is_video(&item.item) && item.video.is_none())
        {
            ui.put(
                Rect::from_center_size(rect.center(), Vec2::splat(24.0)),
                egui::Spinner::new(),
            );
        } else if is_video(&item.item) && item.preview.is_some() {
            video_stage(
                ui,
                item,
                &VideoControls {
                    player: &app.video,
                    locale: app.locale,
                    accent: palette.accent,
                    expanded: false,
                    fill_window: false,
                },
                rect,
                &mut actions,
            );
        } else {
            let mut body = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(rect)
                    .layout(Layout::top_down(Align::Center)),
            );
            body.add_space((rect.height() / 2.0 - 65.0).max(0.0));
            let (icon, _) = body.allocate_exact_size(Vec2::splat(64.0), Sense::hover());
            // A video that cannot be shown here still goes as a video.
            let icon_kind = if is_video(&item.item) {
                Icon::Video
            } else {
                Icon::FileText
            };
            theme::paint_icon(&body, icon_kind, icon, 54.0, palette.secondary);
            body.add(egui::Label::new(egui::RichText::new(name).color(palette.text)).wrap());
            let size = match &item.item {
                Pending::Picture { .. } => None,
                Pending::File(path) | Pending::Document(path) => file_size(ui.ctx(), path),
            };
            let detail: Vec<String> = length_label(item)
                .into_iter()
                .chain(size.map(crate::util::bytes))
                .collect();
            if !detail.is_empty() {
                body.label(egui::RichText::new(detail.join(" · ")).color(palette.secondary));
            }
            // The player here decodes H.264 only: another codec, or a file it
            // cannot read, can still be checked in the system's player.
            if let Pending::File(path) = &item.item
                && is_video(&item.item)
            {
                body.add_space(10.0);
                let open = crate::i18n::gettext(app.locale, "Open file");
                if theme::soft_button(&mut body, &palette, Some(Icon::ExternalLink), &open, false)
                    .tab_stop(Stop::ReviewPlay)
                    .clicked()
                {
                    actions.push(Action::OpenFile(path.clone()));
                }
            }
        }
    }
    app.actions.extend(actions);
    if edit {
        app.actions.push(Action::EditPicture(target));
    }
    if close {
        app.actions.push(Action::ClearPending);
    }
}

/// The selected video, fitted whole: its still with a play button, and once
/// it plays, its frames under the controls a video message has. It plays
/// from the file as picked, with no message behind it.
fn video_stage(
    ui: &mut egui::Ui,
    staged: &mut Staged,
    controls: &VideoControls<'_>,
    rect: Rect,
    actions: &mut Vec<Action>,
) {
    use crate::video::State;
    let Pending::File(path) = &staged.item else {
        return;
    };
    let path = path.clone();
    let Some(still) = still(ui, staged) else {
        return;
    };
    let key = video_key(staged.id);
    // The still's proportions hold the place, so starting to play moves nothing.
    let picture = fit_within(still.size_vec2(), rect);
    let response = ui
        .interact(
            picture,
            ui.id().with(("staged-video", staged.id)),
            Sense::click(),
        )
        .tab_stop(Stop::ReviewPlay);
    theme::reveal_focus(&response);
    let status = controls.player.status(&key);
    if status.is_some() {
        controls.player.saw(&key);
    }
    let state = status.as_ref().map(|status| status.state);
    let label = if state == Some(State::Playing) {
        crate::i18n::gettext(controls.locale, "Pause")
    } else {
        crate::i18n::gettext(controls.locale, "Play")
    };
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &label));
    let whole = Rect::from_min_max(egui::Pos2::ZERO, pos2(1.0, 1.0));
    ui.painter().rect_filled(picture, 6.0, Color32::BLACK);
    match status.as_ref().and_then(|status| status.frame.as_ref()) {
        Some(frame) => paint_texture(
            ui,
            fit_within(frame.size_vec2(), picture),
            frame.id(),
            whole,
            6.0,
        ),
        None => paint_texture(ui, picture, still.id(), whole, 6.0),
    }
    if state != Some(State::Playing) {
        ui.painter()
            .rect_filled(picture, 6.0, Color32::from_black_alpha(40));
        let disc = Rect::from_center_size(picture.center(), Vec2::splat(56.0));
        ui.painter()
            .circle_filled(disc.center(), 28.0, Color32::from_black_alpha(140));
        if state == Some(State::Loading) {
            theme::paint_spinner(ui, disc, 26.0, Color32::WHITE);
        } else {
            theme::paint_icon(ui, Icon::Play, disc, 26.0, Color32::WHITE);
        }
    }
    match &status {
        // As in a message: while paused, and while the pointer is over it.
        Some(status)
            if status.state == State::Paused
                || (status.state == State::Playing && ui.rect_contains_pointer(picture)) =>
        {
            video_controls(ui, controls, &key, &path, picture, status, actions);
        }
        Some(_) => {}
        None => {
            if let Some(length) = length_label(staged) {
                let galley =
                    ui.painter()
                        .layout_no_wrap(length, theme::medium(11.5), Color32::WHITE);
                let chip = Rect::from_min_size(
                    pos2(
                        picture.left() + 8.0,
                        picture.bottom() - galley.size().y - 14.0,
                    ),
                    galley.size() + vec2(12.0, 6.0),
                );
                ui.painter()
                    .rect_filled(chip, chip.height() / 2.0, Color32::from_black_alpha(140));
                ui.painter()
                    .galley(chip.min + vec2(6.0, 3.0), galley, Color32::WHITE);
            }
        }
    }
    if response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
    {
        actions.push(Action::PlayVideo { message: key, path });
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
