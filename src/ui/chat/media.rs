//! Media rendering: pictures, stickers, video players, video notes, documents, and voice notes.

use super::*;

pub(crate) fn thumbnail_uri(ctx: &egui::Context, chat: &str, id: &str, bytes: &[u8]) -> String {
    let uri = format!(
        "bytes://thumb-{}-{}",
        chat.chars()
            .filter(char::is_ascii_alphanumeric)
            .collect::<String>(),
        id
    );
    crate::image_cache::include(ctx, &uri, bytes);
    uri
}

/// Default image bounds based on [`CARD_WIDTH`].
pub(crate) const PICTURE_WIDTH: f32 = CARD_WIDTH;
pub(crate) const PICTURE_HEIGHT: f32 = 440.0;
pub(crate) const STICKER_SIDE: f32 = 180.0;
pub(crate) const HEADER_ROW: f32 = 44.0;

/// Fits an image within bounds without upscaling and with a readable minimum.
pub(crate) fn fit_picture(width: f32, height: f32, max_width: f32, max_height: f32) -> Vec2 {
    let (width, height) = if width > 0.0 && height > 0.0 {
        (width, height)
    } else {
        (4.0, 3.0)
    };
    let max_width = max_width.max(0.0);
    let max_height = max_height.max(0.0);
    let scale = (max_width / width).min(max_height / height).clamp(0.0, 1.0);
    let scale = if width * scale < 120.0 {
        (120.0 / width).min(max_width / width).max(0.0)
    } else {
        scale
    };
    vec2(
        (width * scale).max(0.0),
        (height * scale).max(90.0).max(0.0),
    )
}

/// Fits a sticker to the standard square size.
pub(crate) fn fit_sticker(width: f32, height: f32) -> Vec2 {
    let (width, height) = if width > 0.0 && height > 0.0 {
        (width, height)
    } else {
        (1.0, 1.0)
    };
    let scale = (STICKER_SIDE / width).min(STICKER_SIDE / height);
    vec2(width * scale, height * scale)
}

/// Reserved size for an image or video before and after download.
pub(crate) fn frame_size(
    media: &Media,
    thumbnail_hint: Option<(u32, u32)>,
    max_width: f32,
    max_height: f32,
) -> Vec2 {
    let (w, h) = match (media.width, media.height) {
        (Some(w), Some(h)) if w > 0 && h > 0 => (w as f32, h as f32),
        _ => match thumbnail_hint {
            Some((w, h)) if w > 0 && h > 0 => (w as f32, h as f32),
            _ => (4.0, 3.0),
        },
    };
    fit_picture(w, h, max_width, max_height)
}

/// Draws an image or sticker, using its preview until downloaded. Returns its width.
pub(crate) fn picture(
    ui: &mut egui::Ui,
    view: &View<'_>,
    message: &Message,
    media: &Media,
    width: f32,
    sticker: Option<bool>,
    actions: &mut Vec<Action>,
) -> Rect {
    let palette = view.palette;
    let radius = picture_radius(&palette, message);
    let (max_width, max_height) = match sticker {
        Some(_) => (STICKER_SIDE, STICKER_SIDE),
        None => (width.min(PICTURE_WIDTH), PICTURE_HEIGHT),
    };
    if let Some(path) = &media.path {
        let sticker = sticker.map(|animated| animated || crate::ui::picker::moves(ui.ctx(), path));
        if sticker == Some(true) {
            let size = fit_sticker(
                media.width.unwrap_or(180) as f32,
                media.height.unwrap_or(180) as f32,
            );
            let (rect, response) = ui.allocate_exact_size(size, Sense::click());
            let mut unreadable = false;
            if ui.is_rect_visible(rect) {
                match animation::frame(ui, path, rect, view.animate && response.hovered()) {
                    animation::Frame::Ready(texture) => {
                        ui.painter().image(
                            texture.id(),
                            rect,
                            Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                            Color32::WHITE,
                        );
                    }
                    frame => {
                        ui.painter().rect_filled(rect, radius, palette.surface);
                        theme::paint_icon(ui, Icon::Sticker, rect, 32.0, palette.secondary);
                        unreadable = matches!(frame, animation::Frame::Unavailable);
                    }
                }
            }
            if unreadable {
                actions.push(unreadable_media(view, message, path));
            } else if response
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked()
            {
                actions.push(Action::PreviewSticker(path.clone()));
            }
            return rect;
        }
        let fit = |pixels: Vec2| {
            if sticker.is_some() {
                fit_sticker(pixels.x, pixels.y)
            } else {
                fit_picture(pixels.x, pixels.y, max_width, max_height)
            }
        };
        let pixels_id = egui::Id::new(("picture-pixels", path));
        let drawn = ui.ctx().data(|data| data.get_temp::<Vec2>(pixels_id));
        let reserved = drawn.map_or_else(|| frame_size(media, None, max_width, max_height), fit);
        let position = ui.next_widget_position();
        if !ui.is_rect_visible(Rect::from_min_size(position, reserved)) {
            return ui.allocate_exact_size(reserved, Sense::hover()).0;
        }
        let image = widgets::file_image(ui, path);
        return match image.load_for_size(ui.ctx(), vec2(max_width, max_height)) {
            Ok(egui::load::TexturePoll::Ready { texture }) => {
                if drawn != Some(texture.size) {
                    ui.ctx()
                        .data_mut(|data| data.insert_temp(pixels_id, texture.size));
                }
                let size = fit(texture.size);
                let response = ui.add(
                    image
                        .fit_to_exact_size(size)
                        .corner_radius(if sticker.is_some() { 0.0 } else { radius })
                        .sense(Sense::click()),
                );
                let rect = response.rect;
                if response
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .clicked()
                {
                    let action = match sticker {
                        Some(_) => Action::PreviewSticker(path.clone()),
                        None => match crate::image_preview::open_target(path, true) {
                            crate::image_preview::OpenTarget::Preview => Action::PreviewImage {
                                path: path.clone(),
                                chat: view.chat.id.clone(),
                                message: message.id.clone(),
                                card: None,
                            },
                            crate::image_preview::OpenTarget::External => {
                                Action::OpenFile(path.clone())
                            }
                        },
                    };
                    actions.push(action);
                }
                rect
            }
            Ok(egui::load::TexturePoll::Pending { .. }) => {
                let size = match drawn {
                    Some(pixels) => fit(pixels),
                    None if sticker.is_some() => Vec2::splat(STICKER_SIDE),
                    None => frame_size(media, None, max_width, max_height),
                };
                let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
                if ui.is_rect_visible(rect) {
                    ui.painter().rect_filled(rect, radius, palette.surface);
                    theme::paint_spinner(ui, rect, 22.0, palette.accent);
                }
                rect
            }
            Err(_) if sticker.is_some() || !path.is_file() => {
                let size = drawn.map_or_else(
                    || {
                        if sticker.is_some() {
                            Vec2::splat(STICKER_SIDE)
                        } else {
                            frame_size(media, None, max_width, max_height)
                        }
                    },
                    fit,
                );
                let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
                if ui.is_rect_visible(rect) {
                    ui.painter().rect_filled(rect, radius, palette.surface);
                }
                actions.push(unreadable_media(view, message, path));
                rect
            }
            Err(_) => {
                let size = frame_size(media, None, max_width, max_height);
                let (rect, response) = ui.allocate_exact_size(size, Sense::click());
                if ui.is_rect_visible(rect) {
                    ui.painter().rect_filled(rect, radius, palette.surface);
                    theme::paint_icon(ui, Icon::CircleAlert, rect, 24.0, palette.danger);
                    ui.painter().text(
                        rect.center() + vec2(0.0, 24.0),
                        Align2::CENTER_CENTER,
                        "Could not display this picture. Click to open it.",
                        theme::regular(11.5),
                        palette.secondary,
                    );
                }
                if response.clicked() {
                    actions.push(Action::OpenFile(path.clone()));
                }
                rect
            }
        };
    }
    let size = frame_size(media, None, max_width, max_height);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    if ui.is_rect_visible(rect) {
        let thumbnail = message
            .thumbnail
            .as_deref()
            .filter(|_| sticker.is_none())
            .map(|bytes| thumbnail_uri(ui.ctx(), &message.chat, &message.id, bytes));
        match thumbnail {
            Some(uri) => {
                egui::Image::new(uri)
                    .fit_to_exact_size(size)
                    .corner_radius(radius)
                    .paint_at(ui, rect);
                ui.painter()
                    .rect_filled(rect, radius, Color32::from_black_alpha(60));
            }
            None => {
                ui.painter().rect_filled(rect, radius, palette.surface);
            }
        }
        let disc = Rect::from_center_size(rect.center(), Vec2::splat(44.0));
        match &media.state {
            MediaState::Downloading => {
                ui.painter()
                    .circle_filled(disc.center(), 22.0, Color32::from_black_alpha(120));
                theme::paint_progress(
                    ui,
                    disc,
                    22.0,
                    download_fraction(view, message),
                    Color32::WHITE,
                );
            }
            MediaState::Failed(_) => {
                ui.painter()
                    .circle_filled(disc.center(), 22.0, Color32::from_black_alpha(120));
                theme::paint_icon(ui, Icon::CircleAlert, disc, 22.0, palette.danger);
                ui.painter().text(
                    rect.center() + vec2(0.0, 34.0),
                    Align2::CENTER_CENTER,
                    crate::i18n::gettext(view.locale, "Download failed. Click to retry."),
                    theme::regular(11.5),
                    Color32::WHITE,
                );
            }
            MediaState::Idle => {
                ui.painter()
                    .circle_filled(disc.center(), 22.0, Color32::from_black_alpha(120));
                theme::paint_icon(
                    ui,
                    if sticker.is_some() {
                        Icon::Sticker
                    } else {
                        Icon::Download
                    },
                    disc,
                    22.0,
                    Color32::WHITE,
                );
                if sticker.is_none() {
                    ui.painter().text(
                        rect.center() + vec2(0.0, 34.0),
                        Align2::CENTER_CENTER,
                        crate::util::bytes(media.size),
                        theme::regular(11.5),
                        Color32::WHITE,
                    );
                }
            }
        }
    }
    let wants = response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
        && !matches!(media.state, MediaState::Downloading);
    let auto = ui.is_rect_visible(rect)
        && matches!(media.state, MediaState::Idle)
        && auto_download_allowed(media, sticker.is_some(), view.auto_download);
    if wants || auto {
        actions.push(Action::Download {
            manual: wants,
            card: None,
            chat: view.chat.id.clone(),
            message: message.id.clone(),
        });
    }
    rect
}

pub(crate) fn unreadable_media(view: &View<'_>, message: &Message, path: &Path) -> Action {
    Action::UnreadableMedia {
        chat: view.chat.id.clone(),
        message: message.id.clone(),
        path: path.to_path_buf(),
    }
}

pub(crate) fn motion_clip(
    ui: &mut egui::Ui,
    view: &View<'_>,
    message: &Message,
    motion: &crate::model::Motion,
    picture: Rect,
    actions: &mut Vec<Action>,
) {
    if !ui.is_rect_visible(picture) {
        return;
    }
    let playing = view.motion_playing == Some(message.id.as_str());
    let toggle = || Action::ToggleMotion {
        chat: view.chat.id.clone(),
        message: message.id.clone(),
    };
    let badge = Rect::from_min_size(picture.min + vec2(6.0, 6.0), Vec2::splat(28.0));
    let response = ui
        .interact(
            badge,
            bubble_id(&view.chat.id, &message.id).with("motion"),
            Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    if response.clicked() {
        actions.push(toggle());
    }
    let frame = motion
        .path
        .as_ref()
        .filter(|_| playing)
        .map(|path| (path, animation::frame(ui, path, picture, view.animate)));
    let icon = match (&frame, &motion.state) {
        (Some((_, animation::Frame::Ready(texture))), _) => {
            ui.painter().rect_filled(picture, 6.0, Color32::BLACK);
            paint_texture(
                ui,
                fit_within(texture.size_vec2(), picture),
                texture.id(),
                Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                6.0,
            );
            Some(Icon::Pause)
        }
        (Some((path, animation::Frame::Unavailable)), _) => {
            actions.push(toggle());
            actions.push(Action::OpenFile((*path).clone()));
            Some(Icon::Play)
        }
        (Some((_, animation::Frame::Pending)), _) => None,
        (None, MediaState::Downloading) if playing => None,
        (None, MediaState::Failed(_)) => Some(Icon::CircleAlert),
        (None, _) => Some(Icon::Play),
    };
    ui.painter()
        .circle_filled(badge.center(), 14.0, Color32::from_black_alpha(140));
    match icon {
        Some(icon) => theme::paint_icon(ui, icon, badge, 14.0, Color32::WHITE),
        None => theme::paint_spinner(ui, badge, 16.0, Color32::WHITE),
    }
    if let MediaState::Failed(error) = &motion.state {
        response.on_hover_text(error);
    }
}

pub(crate) fn download_starts_by_itself(content: &Content, auto_download: bool) -> bool {
    match content {
        Content::Sticker { media, .. } => auto_download_allowed(media, true, auto_download),
        other => other
            .media()
            .is_some_and(|media| auto_download_allowed(media, false, auto_download)),
    }
}

pub(crate) fn auto_download_allowed(media: &Media, sticker: bool, auto_download: bool) -> bool {
    media.is_within_download_limit() && (sticker || auto_download)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn video(
    ui: &mut egui::Ui,
    view: &View<'_>,
    message: &Message,
    media: &Media,
    seconds: Option<u32>,
    gif: bool,
    width: f32,
    actions: &mut Vec<Action>,
) -> f32 {
    use crate::video::State;
    let palette = view.palette;
    let thumbnail = message.thumbnail.as_deref();
    if thumbnail.is_none() && media.path.is_none() {
        let title = if gif { "GIF" } else { "Video" };
        let mut detail = Vec::new();
        if let Some(seconds) = seconds {
            detail.push(crate::util::duration(seconds));
        }
        detail.push(crate::util::bytes(media.size));
        attachment(
            ui,
            view,
            message,
            media,
            Icon::Video,
            title,
            &detail.join(" · "),
            width,
            actions,
        );
        return width;
    }
    let limit = width.min(PICTURE_WIDTH);
    let size = frame_size(
        media,
        Some((1280, 720)),
        limit,
        PICTURE_HEIGHT.min(limit * 1.3),
    );
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let playing = match (&media.path, gif) {
        (Some(path), true) => Some(animation::frame(
            ui,
            path,
            rect,
            view.animate && response.hovered(),
        )),
        _ => None,
    };
    if let Some(animation::Frame::Ready(texture)) = &playing {
        if ui.is_rect_visible(rect) {
            ui.painter().image(
                texture.id(),
                rect,
                Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        if response
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .clicked()
            && let Some(path) = &media.path
        {
            actions.push(Action::OpenFile(path.clone()));
        }
        return size.x;
    }
    let status = media
        .path
        .as_ref()
        .filter(|_| !gif)
        .and_then(|_| view.video.status(&message.id));
    if ui.is_rect_visible(rect) {
        if status.is_some() {
            view.video.saw(&message.id);
        }
        match status.as_ref().and_then(|status| status.frame.as_ref()) {
            Some(frame) => {
                ui.painter().rect_filled(rect, 6.0, Color32::BLACK);
                paint_texture(
                    ui,
                    fit_within(frame.size_vec2(), rect),
                    frame.id(),
                    Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                    6.0,
                );
            }
            None => match (thumbnail, &media.path) {
                (Some(thumbnail), _) => {
                    let uri = thumbnail_uri(ui.ctx(), &message.chat, &message.id, thumbnail);
                    egui::Image::new(uri)
                        .fit_to_exact_size(size)
                        .corner_radius(6.0)
                        .paint_at(ui, rect);
                }
                (None, Some(path)) => {
                    ui.painter().rect_filled(rect, 6.0, Color32::BLACK);
                    if let animation::Frame::Ready(texture) =
                        animation::frame(ui, path, rect, false)
                    {
                        paint_texture(
                            ui,
                            fit_within(texture.size_vec2(), rect),
                            texture.id(),
                            Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                            6.0,
                        );
                    }
                }
                (None, None) => {}
            },
        }
        let state = status.as_ref().map(|status| status.state);
        if state != Some(State::Playing) {
            ui.painter()
                .rect_filled(rect, 6.0, Color32::from_black_alpha(40));
            let disc = Rect::from_center_size(rect.center(), Vec2::splat(48.0));
            ui.painter()
                .circle_filled(disc.center(), 24.0, Color32::from_black_alpha(140));
            match (&media.path, &media.state) {
                (Some(_), _)
                    if state == Some(State::Loading)
                        || matches!(playing, Some(animation::Frame::Pending)) =>
                {
                    theme::paint_spinner(ui, disc, 24.0, Color32::WHITE)
                }
                (Some(_), _) if gif => {
                    theme::paint_icon(ui, Icon::ExternalLink, disc, 22.0, Color32::WHITE)
                }
                (None, MediaState::Downloading) => theme::paint_progress(
                    ui,
                    disc,
                    24.0,
                    download_fraction(view, message),
                    Color32::WHITE,
                ),
                (None, MediaState::Failed(_)) => {
                    theme::paint_icon(ui, Icon::CircleAlert, disc, 22.0, palette.danger)
                }
                (Some(_), _) | (None, MediaState::Idle) => {
                    theme::paint_icon(ui, Icon::Play, disc, 22.0, Color32::WHITE)
                }
            }
        }
        match (&status, &media.path) {
            (Some(status), Some(path)) => {
                if status.state == State::Paused
                    || (status.state == State::Playing && ui.rect_contains_pointer(rect))
                {
                    video_controls(
                        ui,
                        &VideoControls {
                            player: view.video,
                            locale: view.locale,
                            accent: view.palette.accent,
                            expanded: false,
                        },
                        &message.id,
                        path,
                        rect,
                        status,
                        actions,
                    );
                }
            }
            _ => {
                let mut label = Vec::new();
                if gif {
                    label.push("GIF".to_owned());
                }
                if let Some(seconds) = seconds {
                    label.push(crate::util::duration(seconds));
                }
                if media.path.is_none() {
                    label.push(crate::util::bytes(media.size));
                }
                if !label.is_empty() {
                    let galley = ui.painter().layout_no_wrap(
                        label.join(" · "),
                        theme::medium(11.5),
                        Color32::WHITE,
                    );
                    let chip = Rect::from_min_size(
                        pos2(rect.left() + 8.0, rect.bottom() - galley.size().y - 14.0),
                        galley.size() + vec2(12.0, 6.0),
                    );
                    ui.painter().rect_filled(
                        chip,
                        chip.height() / 2.0,
                        Color32::from_black_alpha(140),
                    );
                    ui.painter()
                        .galley(chip.min + vec2(6.0, 3.0), galley, Color32::WHITE);
                }
            }
        }
    }
    let auto = ui.is_rect_visible(rect)
        && media.path.is_none()
        && matches!(media.state, MediaState::Idle)
        && view.auto_download
        && media.is_within_download_limit();
    if auto {
        actions.push(Action::Download {
            manual: false,
            card: None,
            chat: view.chat.id.clone(),
            message: message.id.clone(),
        });
    }
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    if response.clicked() {
        video_clicked(view, message, media, gif, auto, actions);
    }
    if response.double_clicked()
        && !gif
        && let Some(path) = &media.path
    {
        actions.push(expand_video_action(&message.id, path));
    }
    size.x
}

pub(crate) fn expand_video_action(message: &str, path: &Path) -> Action {
    Action::ExpandVideo {
        message: message.to_owned(),
        path: path.to_owned(),
    }
}

pub(crate) fn video_clicked(
    view: &View<'_>,
    message: &Message,
    media: &Media,
    gif: bool,
    downloading: bool,
    actions: &mut Vec<Action>,
) {
    match &media.path {
        Some(path) if gif => actions.push(Action::OpenFile(path.clone())),
        Some(path) => actions.push(Action::PlayVideo {
            message: message.id.clone(),
            path: path.clone(),
        }),
        None => {
            if !downloading && !matches!(media.state, MediaState::Downloading) {
                actions.push(Action::Download {
                    manual: true,
                    card: None,
                    chat: view.chat.id.clone(),
                    message: message.id.clone(),
                });
            }
            if !gif {
                actions.push(Action::PlayVideoWhenDownloaded(message.id.clone()));
            }
        }
    }
}

pub(crate) struct VideoControls<'a> {
    pub player: &'a crate::video::Player,
    pub locale: crate::i18n::Locale,
    pub accent: Color32,
    /// Whether the video covers the window; its button then puts it back.
    pub expanded: bool,
}

pub(crate) fn video_controls(
    ui: &mut egui::Ui,
    controls: &VideoControls<'_>,
    message: &str,
    path: &Path,
    rect: Rect,
    status: &crate::video::Status,
    actions: &mut Vec<Action>,
) {
    let bar = Rect::from_min_max(pos2(rect.left(), rect.bottom() - 32.0), rect.max);
    ui.painter().rect_filled(
        bar,
        CornerRadius {
            nw: 0,
            ne: 0,
            sw: 6,
            se: 6,
        },
        Color32::from_black_alpha(150),
    );
    let id = ui.id().with(("video-controls", message));
    let toggle = Rect::from_center_size(pos2(bar.left() + 18.0, bar.center().y), Vec2::splat(26.0));
    let playing = status.state == crate::video::State::Playing;
    theme::paint_icon(
        ui,
        if playing { Icon::Pause } else { Icon::Play },
        toggle,
        16.0,
        Color32::WHITE,
    );
    let tooltip = if playing {
        crate::i18n::gettext(controls.locale, "Pause")
    } else {
        crate::i18n::gettext(controls.locale, "Play")
    };
    if ui
        .interact(toggle, id.with("toggle"), Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(tooltip.as_ref())
        .clicked()
    {
        actions.push(Action::PlayVideo {
            message: message.to_owned(),
            path: path.to_owned(),
        });
    }
    let window =
        Rect::from_center_size(pos2(bar.right() - 18.0, bar.center().y), Vec2::splat(26.0));
    ui.ctx()
        .data_mut(|data| data.insert_temp(egui::Id::new(("video-window", message)), window));
    theme::paint_icon(
        ui,
        if controls.expanded {
            Icon::Minimize
        } else {
            Icon::Maximize
        },
        window,
        16.0,
        Color32::WHITE,
    );
    let tooltip = if controls.expanded {
        crate::i18n::gettext(controls.locale, "Back to the message (Esc)")
    } else {
        crate::i18n::gettext(controls.locale, "Fill the window")
    };
    if ui
        .interact(window, id.with("window"), Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(tooltip.as_ref())
        .clicked()
    {
        actions.push(if controls.expanded {
            Action::CloseImagePreview
        } else {
            expand_video_action(message, path)
        });
    }
    let sound = Rect::from_center_size(
        pos2(window.left() - 15.0, bar.center().y),
        Vec2::splat(26.0),
    );
    let muted = controls.player.muted();
    theme::paint_icon(
        ui,
        if muted { Icon::VolumeX } else { Icon::Volume2 },
        sound,
        16.0,
        Color32::WHITE,
    );
    let tooltip = if muted {
        crate::i18n::gettext(controls.locale, "Unmute")
    } else {
        crate::i18n::gettext(controls.locale, "Mute")
    };
    if ui
        .interact(sound, id.with("sound"), Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(tooltip.as_ref())
        .clicked()
    {
        actions.push(Action::ToggleVideoSound);
    }
    let speed_label = crate::audio::speed_label(controls.player.speed());
    let speed_galley = ui.painter().layout_no_wrap(
        speed_label,
        theme::tabular(fastframe_fonts::Weight::Medium, 11.5),
        Color32::WHITE,
    );
    let speed = Rect::from_center_size(
        pos2(
            sound.left() - 4.0 - (speed_galley.size().x + 10.0) / 2.0,
            bar.center().y,
        ),
        vec2(speed_galley.size().x + 10.0, 18.0),
    );
    ui.painter()
        .rect_filled(speed, 9.0, Color32::from_white_alpha(40));
    ui.painter().galley(
        speed.center() - speed_galley.size() / 2.0,
        speed_galley,
        Color32::WHITE,
    );
    if ui
        .interact(speed, id.with("speed"), Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
    {
        actions.push(Action::CycleVideoSpeed);
    }
    let time = format!(
        "{} / {}",
        crate::util::duration(status.position.as_secs() as u32),
        crate::util::duration(status.total.as_secs() as u32)
    );
    let galley = ui.painter().layout_no_wrap(
        time,
        theme::tabular(fastframe_fonts::Weight::Medium, 11.5),
        Color32::WHITE,
    );
    let text = pos2(toggle.right() + 4.0, bar.center().y - galley.size().y / 2.0);
    let track = Rect::from_min_max(
        pos2(text.x + galley.size().x + 10.0, bar.center().y - 8.0),
        pos2(speed.left() - 6.0, bar.center().y + 8.0),
    );
    ui.painter().galley(text, galley, Color32::WHITE);
    if track.width() < 12.0 {
        return;
    }
    let response = ui
        .interact(track, id.with("seek"), Sense::click_and_drag())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let pointed = response
        .interact_pointer_pos()
        .map(|pointer| ((pointer.x - track.left()) / track.width()).clamp(0.0, 1.0));
    let fraction = match pointed {
        Some(fraction) if response.dragged() => fraction,
        _ => status.fraction(),
    };
    let line = Rect::from_center_size(track.center(), vec2(track.width(), 3.0));
    ui.painter()
        .rect_filled(line, 1.5, Color32::from_white_alpha(90));
    let played = pos2(line.left() + fraction * line.width(), line.center().y);
    ui.painter().rect_filled(
        Rect::from_min_max(line.min, pos2(played.x, line.bottom())),
        1.5,
        controls.accent,
    );
    ui.painter().circle_filled(played, 5.0, controls.accent);
    if (response.clicked() || response.drag_stopped())
        && let Some(fraction) = pointed
    {
        actions.push(Action::SeekVideo {
            message: message.to_owned(),
            fraction,
        });
    }
}

/// Side of a round video message.
pub(crate) const NOTE_SIDE: f32 = 220.0;

pub(crate) fn video_note(
    ui: &mut egui::Ui,
    view: &View<'_>,
    message: &Message,
    media: &Media,
    seconds: Option<u32>,
    actions: &mut Vec<Action>,
) -> f32 {
    use crate::video::State;
    let palette = view.palette;
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(NOTE_SIDE), Sense::click());
    let status = media
        .path
        .as_ref()
        .and_then(|_| view.video.status(&message.id));
    if ui.is_rect_visible(rect) {
        if status.is_some() {
            view.video.saw(&message.id);
        }
        let center = rect.center();
        let picture = rect.shrink(5.0);
        let frame = status
            .as_ref()
            .and_then(|status| status.frame.as_ref())
            .map(|frame| (frame.id(), frame.size_vec2()));
        let poster = || {
            let bytes = message.thumbnail.as_deref()?;
            let uri = thumbnail_uri(ui.ctx(), &message.chat, &message.id, bytes);
            match egui::Image::new(uri).load_for_size(ui.ctx(), picture.size()) {
                Ok(egui::load::TexturePoll::Ready { texture }) => Some((texture.id, texture.size)),
                _ => None,
            }
        };
        match frame.or_else(poster) {
            Some((texture, size)) => paint_texture(
                ui,
                picture,
                texture,
                crate::video::square_uv(size.x, size.y),
                picture.width() / 2.0,
            ),
            None => {
                ui.painter()
                    .circle_filled(center, picture.width() / 2.0, palette.surface);
            }
        }
        let state = status.as_ref().map(|status| status.state);
        let ring = NOTE_SIDE / 2.0 - 2.0;
        if let Some(status) = &status {
            ui.painter().circle_stroke(
                center,
                ring,
                Stroke::new(3.0, palette.secondary.gamma_multiply(0.35)),
            );
            ui.painter().add(egui::Shape::line(
                crate::video::arc(center, ring, status.fraction()),
                Stroke::new(3.0, palette.accent),
            ));
        }
        let hovered = ui.rect_contains_pointer(rect);
        let disc = Rect::from_center_size(center, Vec2::splat(48.0));
        let waiting = matches!(
            (&media.path, &media.state, state),
            (Some(_), _, Some(State::Loading)) | (None, MediaState::Downloading, _)
        );
        let icon = match (&media.path, &media.state, state) {
            _ if waiting => None,
            (Some(_), _, Some(State::Playing)) => hovered.then_some(Icon::Pause),
            (None, MediaState::Failed(_), _) => Some(Icon::CircleAlert),
            _ => Some(Icon::Play),
        };
        if state != Some(State::Playing) {
            ui.painter().circle_filled(
                center,
                picture.width() / 2.0,
                Color32::from_black_alpha(40),
            );
        }
        if waiting || icon.is_some() {
            ui.painter()
                .circle_filled(center, 24.0, Color32::from_black_alpha(140));
        }
        if waiting {
            let fraction = media
                .path
                .is_none()
                .then(|| download_fraction(view, message))
                .flatten();
            theme::paint_progress(ui, disc, 24.0, fraction, Color32::WHITE);
        } else if let Some(icon) = icon {
            let color = if icon == Icon::CircleAlert {
                palette.danger
            } else {
                Color32::WHITE
            };
            theme::paint_icon(ui, icon, disc, 22.0, color);
        }
        let label = match &status {
            Some(status) => Some(crate::util::duration(status.position.as_secs() as u32)),
            None => seconds
                .map(crate::util::duration)
                .or_else(|| media.path.is_none().then(|| crate::util::bytes(media.size))),
        };
        if let Some(label) = label {
            let galley = ui.painter().layout_no_wrap(
                label,
                theme::tabular(fastframe_fonts::Weight::Medium, 11.5),
                Color32::WHITE,
            );
            let chip = Rect::from_center_size(
                pos2(center.x, picture.bottom() - galley.size().y / 2.0 - 16.0),
                galley.size() + vec2(12.0, 6.0),
            );
            ui.painter()
                .rect_filled(chip, chip.height() / 2.0, Color32::from_black_alpha(140));
            ui.painter()
                .galley(chip.min + vec2(6.0, 3.0), galley, Color32::WHITE);
        }
    }
    let auto = ui.is_rect_visible(rect)
        && media.path.is_none()
        && matches!(media.state, MediaState::Idle)
        && view.auto_download
        && media.is_within_download_limit();
    if auto {
        actions.push(Action::Download {
            manual: false,
            card: None,
            chat: view.chat.id.clone(),
            message: message.id.clone(),
        });
    }
    if response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
    {
        video_clicked(view, message, media, false, auto, actions);
    }
    NOTE_SIDE
}

pub(crate) fn paint_texture(ui: &egui::Ui, rect: Rect, texture: egui::TextureId, uv: Rect, radius: f32) {
    ui.painter().add(
        egui::epaint::RectShape::filled(
            rect,
            CornerRadius::same(radius.round().clamp(0.0, 255.0) as u8),
            Color32::WHITE,
        )
        .with_texture(texture, uv),
    );
}

/// The largest rect with the proportions of `size` centred in `rect`.
pub(crate) fn fit_within(size: Vec2, rect: Rect) -> Rect {
    if size.x <= 0.0 || size.y <= 0.0 {
        return rect;
    }
    let scale = (rect.width() / size.x).min(rect.height() / size.y);
    Rect::from_center_size(rect.center(), size * scale)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn attachment(
    ui: &mut egui::Ui,
    view: &View<'_>,
    message: &Message,
    media: &Media,
    icon: Icon,
    title: &str,
    detail: &str,
    width: f32,
    actions: &mut Vec<Action>,
) {
    let palette = view.palette;
    let response = Frame::new()
        .fill(palette.window.gamma_multiply(0.35))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::symmetric(10, 8))
        .show(ui, |ui| {
            let card = (width - 20.0).max(0.0);
            ui.set_width(card);

            let disc = |ui: &mut egui::Ui| {
                let (disc, _) = ui.allocate_exact_size(Vec2::splat(36.0), Sense::hover());
                ui.painter().circle_filled(
                    disc.center(),
                    18.0,
                    palette.accent.gamma_multiply(0.25),
                );
                theme::paint_icon(ui, icon, disc, 18.0, palette.accent);
            };
            let action = |ui: &mut egui::Ui| match (&media.path, &media.state) {
                (Some(_), _) => {
                    theme::icon(ui, Icon::ExternalLink, 18.0, palette.secondary);
                }
                (None, MediaState::Downloading) => {
                    theme::progress(ui, 18.0, download_fraction(view, message), palette.accent);
                }
                (None, _) => {
                    theme::icon(ui, Icon::Download, 18.0, palette.secondary);
                }
            };
            let column = |ui: &mut egui::Ui| {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    ui.set_width((card - 70.0).max(0.0));
                    widgets::rich_text(ui, title, theme::medium(14.0), palette.text);
                    let detail = match &media.state {
                        MediaState::Failed(error) => format!("{error}. Click to retry."),
                        _ => detail.to_owned(),
                    };
                    theme::text(ui, detail, theme::regular(12.0), palette.secondary);
                });
            };
            ui.allocate_ui_with_layout(
                vec2(card, 52.0),
                Layout::left_to_right(egui::Align::Center),
                |ui| {
                    disc(ui);
                    column(ui);
                    action(ui);
                },
            );
        })
        .response;
    let auto = ui.is_rect_visible(response.rect)
        && media.path.is_none()
        && matches!(media.state, MediaState::Idle)
        && auto_download_allowed(media, false, view.auto_download);
    if auto {
        actions.push(Action::Download {
            manual: false,
            card: None,
            chat: view.chat.id.clone(),
            message: message.id.clone(),
        });
    }
    ui.ctx().data_mut(|data| {
        data.insert_temp(
            bubble_id(&view.chat.id, &message.id).with("card"),
            response.rect,
        );
    });
    let response = ui
        .interact(
            response.rect,
            ui.id().with(("attachment", &message.id)),
            Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    if response.clicked() && !auto {
        match &media.path {
            Some(path) => actions.push(Action::OpenFile(path.clone())),
            None if !matches!(media.state, MediaState::Downloading) => {
                actions.push(Action::Download {
                    manual: true,
                    card: None,
                    chat: view.chat.id.clone(),
                    message: message.id.clone(),
                })
            }
            None => {}
        }
    }
}

pub(crate) fn bottom_filler(viewport: f32, natural: Option<f32>, spacing: f32) -> f32 {
    natural.map_or(0.0, |natural| (viewport - natural - spacing).max(0.0))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn voice_player(
    ui: &mut egui::Ui,
    view: &View<'_>,
    message: &Message,
    media: &Media,
    seconds: Option<u32>,
    waveform: &[u8],
    width: f32,
    reserve: f32,
    actions: &mut Vec<Action>,
) -> Option<Rect> {
    use crate::audio::State;
    let palette = view.palette;
    let status = view.player.status(&message.id);
    let button = 36.0;
    let bar_height = 30.0;
    let chip = 44.0;
    let voice_note = matches!(
        message.content,
        Content::Audio {
            voice_note: true,
            ..
        }
    );
    let shows_chip = media.path.is_some();
    let wave_width = (width
        - button
        - 10.0
        - if shows_chip { chip + 10.0 } else { 0.0 }
        - if voice_note { 0.0 } else { button + 10.0 })
    .max(0.0);
    let bars: Vec<u8> = if !waveform.is_empty() {
        waveform.to_vec()
    } else if let Some(bars) = view.player.bars(&message.id) {
        bars.to_vec()
    } else {
        vec![12; crate::voice::BARS]
    };
    let fill = palette.accent.gamma_multiply(0.22);
    let hover = palette.accent.gamma_multiply(0.38);
    let waiting = |ui: &mut egui::Ui, fraction: Option<f32>| {
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(button), Sense::hover());
        ui.painter()
            .circle_filled(rect.center(), button / 2.0, fill);
        ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
            ui.centered_and_justified(|ui| {
                theme::progress(ui, 18.0, fraction, palette.accent);
            });
        });
    };
    let mut duration = None;
    let row = ui.allocate_ui_with_layout(
        vec2(width.max(0.0), button),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            match (&media.path, &media.state) {
                (None, MediaState::Downloading) => waiting(ui, download_fraction(view, message)),
                (None, _) => {
                    if theme::circle_button(
                        ui,
                        Icon::Download,
                        button,
                        fill,
                        hover,
                        palette.accent,
                        "Download",
                    )
                    .clicked()
                    {
                        actions.push(Action::Download {
                            manual: true,
                            card: None,
                            chat: view.chat.id.clone(),
                            message: message.id.clone(),
                        });
                    }
                }
                (Some(path), _) => match status.state {
                    State::Loading => waiting(ui, None),
                    State::Playing | State::Paused | State::Idle => {
                        let (icon, tooltip) = if status.state == State::Playing {
                            (Icon::Pause, "Pause")
                        } else {
                            (Icon::Play, "Play")
                        };
                        if theme::circle_button(
                            ui,
                            icon,
                            button,
                            fill,
                            hover,
                            palette.accent,
                            tooltip,
                        )
                        .clicked()
                        {
                            actions.push(Action::PlayVoice {
                                message: message.id.clone(),
                                path: path.clone(),
                            });
                        }
                    }
                },
            }
            let mut wave_middle = None;
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                let (rect, response) =
                    ui.allocate_exact_size(vec2(wave_width, bar_height), Sense::click());
                wave_middle = Some(rect.center().y);
                let pitch = 3.0;
                let count = (rect.width() / pitch).floor() as usize;
                let fraction = if status.total > Duration::ZERO {
                    status.position.as_secs_f32() / status.total.as_secs_f32()
                } else {
                    0.0
                };
                let played_until = rect.left() + fraction * rect.width();
                let quiet = palette.secondary.gamma_multiply(0.7);
                let kind = if voice_note {
                    "voice-waveform"
                } else {
                    "audio-track"
                };
                ui.ctx().data_mut(|data| {
                    data.insert_temp(bubble_id(&view.chat.id, &message.id).with(kind), rect);
                });
                if !voice_note {
                    let track = Rect::from_center_size(rect.center(), vec2(rect.width(), 4.0));
                    ui.painter().rect_filled(track, 2.0, quiet);
                    if fraction > 0.0 {
                        ui.painter().rect_filled(
                            Rect::from_min_max(track.min, pos2(played_until, track.bottom())),
                            2.0,
                            palette.accent,
                        );
                    }
                } else if count > 0 {
                    for index in 0..count {
                        let level = f32::from(bars[index * bars.len() / count]) / 100.0;
                        let height = (2.0 + level * (bar_height - 4.0)).max(2.0);
                        let x = rect.left() + index as f32 * pitch + 1.0;
                        let colour = if status.state != State::Idle && x <= played_until {
                            palette.accent
                        } else {
                            quiet
                        };
                        ui.painter().rect_filled(
                            Rect::from_center_size(
                                egui::pos2(x, rect.center().y),
                                vec2(2.0, height),
                            ),
                            1.0,
                            colour,
                        );
                    }
                }
                if (!voice_note || matches!(status.state, State::Playing | State::Paused))
                    && rect.width() >= 10.0
                {
                    let knob = played_until.clamp(rect.left() + 5.0, rect.right() - 5.0);
                    ui.painter().circle_filled(
                        egui::pos2(knob, rect.center().y),
                        5.0,
                        palette.accent,
                    );
                }
                if let Some(path) = &media.path {
                    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
                    if response.clicked()
                        && let Some(pointer) = response.interact_pointer_pos()
                    {
                        let fraction = if rect.width() > 0.0 {
                            ((pointer.x - rect.left()) / rect.width()).clamp(0.0, 1.0)
                        } else {
                            0.0
                        };
                        actions.push(Action::SeekVoice {
                            message: message.id.clone(),
                            path: path.clone(),
                            fraction,
                        });
                    }
                }
                let shown = match status.state {
                    State::Playing | State::Paused => {
                        crate::util::duration(status.position.as_secs() as u32)
                    }
                    _ => seconds
                        .or_else(|| {
                            (status.total > Duration::ZERO).then_some(status.total.as_secs() as u32)
                        })
                        .map(crate::util::duration)
                        .unwrap_or_else(|| crate::util::bytes(media.size)),
                };
                let (text, font) = match &media.state {
                    MediaState::Failed(error) => {
                        (format!("{error}. Click to retry."), theme::regular(11.5))
                    }
                    _ => (
                        shown,
                        theme::tabular(fastframe_fonts::Weight::Regular, 11.5),
                    ),
                };
                let shown = theme::text(ui, text, font, palette.secondary).rect;
                duration = Some(shown);
                if voice_note
                    && !message.from_me
                    && message.status != Delivery::Played
                    && !matches!(media.state, MediaState::Failed(_))
                {
                    let dot = Rect::from_center_size(
                        pos2(shown.right() + 7.0, shown.center().y),
                        Vec2::splat(6.0),
                    );
                    ui.painter()
                        .circle_filled(dot.center(), 3.0, palette.accent);
                    duration = Some(shown.union(dot));
                    ui.ctx().data_mut(|data| {
                        data.insert_temp(
                            bubble_id(&view.chat.id, &message.id).with("unheard"),
                            dot,
                        );
                    });
                }
            });
            if !voice_note {
                let (slot, response) = ui.allocate_exact_size(Vec2::splat(button), Sense::hover());
                let at = Rect::from_center_size(
                    pos2(slot.center().x, wave_middle.unwrap_or(slot.center().y)),
                    Vec2::splat(button),
                );
                ui.painter().circle_filled(at.center(), button / 2.0, fill);
                theme::paint_icon(ui, Icon::Music, at, 20.0, palette.accent);
                let label = crate::i18n::gettext(view.locale, "Audio");
                response.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::Label, true, label.as_ref())
                });
                ui.ctx().data_mut(|data| {
                    data.insert_temp(bubble_id(&view.chat.id, &message.id).with("audio-file"), at);
                });
            }
            if shows_chip {
                let speed = view.voice_speed;
                let preparing = view.player.preparing_speed(&message.id);
                let size = vec2(chip, 20.0);
                let (slot, _) = ui.allocate_exact_size(size, Sense::hover());
                let at = Rect::from_center_size(
                    egui::pos2(slot.center().x, wave_middle.unwrap_or(slot.center().y)),
                    size,
                );
                let response = ui
                    .scope_builder(egui::UiBuilder::new().max_rect(at), |ui| {
                        speed_pill(ui, view, size, speed, speed > 1.0, preparing)
                    })
                    .inner;
                ui.ctx().data_mut(|data| {
                    data.insert_temp(speed_chip_id(&view.chat.id, &message.id), response.rect);
                });
                if response.clicked() {
                    actions.push(speed_action(
                        view.chat,
                        crate::audio::next_cycled_speed(speed),
                    ));
                }
                response.on_hover_text(if preparing {
                    "Preparing playback speed"
                } else {
                    "Playback speed. Right-click for every speed."
                });
            }
        },
    );
    let auto = media.path.is_none()
        && matches!(media.state, MediaState::Idle)
        && view.auto_download
        && media.is_within_download_limit();
    if auto {
        actions.push(Action::Download {
            manual: false,
            card: None,
            chat: view.chat.id.clone(),
            message: message.id.clone(),
        });
    }
    let right = row.response.rect.right();
    duration
        .filter(|duration| duration.right() + 8.0 + reserve <= right)
        .map(|duration| {
            Rect::from_min_max(
                pos2(right - reserve, duration.center().y - 7.5),
                pos2(right, duration.center().y + 7.5),
            )
        })
}


pub(crate) fn attachment_name(content: &Content, path: &Path) -> String {
    let name = match content {
        Content::Document { file_name, .. } if !file_name.trim().is_empty() => file_name.clone(),
        _ => path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "attachment".to_owned()),
    };
    name.replace(['/', '\\'], "_")
}
