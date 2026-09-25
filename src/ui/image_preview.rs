//! The media viewer: a full-window photo and video view with the chat's
//! album along the bottom, in the shape WhatsApp's own viewer uses.

use egui::{Align, CornerRadius, Layout, Rect, Sense, Stroke, UiBuilder, Vec2, pos2, vec2};

use std::path::Path;

use crate::app::App;
use crate::archive::ChatMedia;
use crate::image_preview::PreviewState;
use crate::model::{Action, Dialog};
use crate::theme::{self, Icon};

use super::conversation::thumbnail_uri;

/// Height of the header across the top.
const HEADER: f32 = 56.0;
/// Height of the album strip along the bottom.
const STRIP: f32 = 78.0;
const THUMB: f32 = 56.0;
const STRIP_GAP: f32 = 6.0;
const STRIP_PAD: f32 = 4.0;

pub fn show(app: &mut App, ctx: &egui::Context) {
    // The album carries every thumbnail of the chat, so it is moved out for
    // the frame and put back at the end instead of being copied: a copy per
    // repaint is megabytes of bytes nobody reads. The preview holds two ids and
    // two strings, and it is moved the same way so the drawing functions can
    // borrow it mutably while `app` is borrowed too.
    let Some(mut preview) = app.image_preview.take() else {
        return;
    };
    let album = std::mem::take(&mut app.viewer_media);
    let palette = app.palette;
    // The album the viewer browses, and where this picture sits in it.
    let position = preview.position(&album);
    let item = position.and_then(|index| album.get(index));
    let message = preview.message().to_owned();
    let chat = preview.chat().to_owned();
    let thumbnail = item.and_then(|item| item.thumbnail.as_deref());
    // With no album item yet, the file's own name still says whether this is a
    // clip, so a clip is never handed to the image decoder.
    let clip = preview.shows_video(&album);
    let strip = album.len() > 1;
    let mut actions: Vec<Action> = Vec::new();

    let mut backdrop_clicked = false;
    // The viewer fills the window instead of floating a dialog over the chat:
    // the header across the top, the picture on the field, and the album along
    // the bottom, which is the shape WhatsApp's own viewer uses. It stays a
    // modal, so it keeps the keyboard: Tab walks its own controls and Enter
    // cannot reach the chat behind it. The field and every control over it come
    // from the palette, so the light theme reads as well as the dark one.
    let screen = ctx.content_rect();
    let area = egui::Area::new(egui::Id::new("image-preview"))
        .order(egui::Order::Foreground)
        .fixed_pos(screen.min);
    let response = egui::Modal::new(egui::Id::new("image-preview"))
        .area(area)
        .frame(egui::Frame::new().fill(palette.overlay))
        .backdrop_color(palette.shadow)
        .show(ctx, |ui| {
            // Sized to the window with no margin around it, so the field runs
            // edge to edge.
            ui.set_min_size(screen.size());
            let window = Rect::from_min_size(screen.min, screen.size());
            ui.painter().rect_filled(window, 0.0, palette.overlay);
            // Claimed first, so every control drawn over it wins the click.
            // What reaches this is a click on the empty field, which closes the
            // viewer, as it does on the phone.
            let (_, backdrop) = ui.allocate_exact_size(window.size(), Sense::click());
            let (header_rect, stage, strip_rect) = panes(window, strip);

            let mut head = ui.new_child(UiBuilder::new().max_rect(header_rect));
            head.set_clip_rect(header_rect);
            header(
                app,
                &mut head,
                &preview,
                item,
                chat.as_str(),
                clip,
                &mut actions,
            );

            let mut body = ui.new_child(UiBuilder::new().max_rect(stage));
            body.set_clip_rect(stage);
            // The file on screen is the preview's, not the album item's: a clip
            // opened from the message menu is shown before the album arrives,
            // and the album may never hold that message at all.
            let shown = preview.path().map(Path::to_path_buf);
            if clip {
                video(
                    app,
                    &mut body,
                    shown.as_deref(),
                    thumbnail,
                    stage,
                    &chat,
                    &message,
                    &mut actions,
                );
            } else {
                still(
                    app,
                    &mut body,
                    &mut preview,
                    shown.as_deref(),
                    thumbnail,
                    clip,
                    &chat,
                    &message,
                    stage,
                    &mut actions,
                );
            }
            if strip {
                chevrons(app, ui, &palette, stage, &mut actions);
                let mut bar = ui.new_child(UiBuilder::new().max_rect(strip_rect));
                bar.set_clip_rect(strip_rect);
                strip_bar(
                    app,
                    &mut bar,
                    &album,
                    &chat,
                    &message,
                    strip_rect,
                    &mut actions,
                );
            }
            backdrop_clicked = backdrop.clicked();
        });
    if response.should_close() || backdrop_clicked {
        actions.push(Action::CloseImagePreview);
    }
    app.image_preview = Some(preview);
    app.viewer_media = album;
    app.actions.extend(actions);
}

/// Header, stage and album strip across the window, top to bottom. Without an
/// album the strip is empty and the stage runs to the bottom edge.
fn panes(window: Rect, strip: bool) -> (Rect, Rect, Rect) {
    let header = Rect::from_min_max(window.min, pos2(window.max.x, window.min.y + HEADER));
    let strip_rect = if strip {
        Rect::from_min_max(pos2(window.min.x, window.max.y - STRIP), window.max)
    } else {
        Rect::from_min_max(pos2(window.max.x, window.max.y), window.max)
    };
    let stage = Rect::from_min_max(
        pos2(window.min.x, header.max.y),
        pos2(window.max.x, strip_rect.min.y),
    );
    (header, stage, strip_rect)
}

/// Title, the actions that reach the message, and the zoom controls.
fn header(
    app: &mut App,
    ui: &mut egui::Ui,
    preview: &PreviewState,
    item: Option<&ChatMedia>,
    chat: &str,
    clip: bool,
    actions: &mut Vec<Action>,
) {
    let palette = app.palette;
    // The chat's name is the useful title once the viewer browses an album;
    // the file name is all there is for a picture opened on its own.
    let title = app
        .chat(chat)
        .map(|known| app.chat_title(known))
        .unwrap_or_else(|| {
            preview
                .path()
                .and_then(Path::file_name)
                .and_then(|name| name.to_str())
                .map_or_else(
                    || crate::i18n::gettext(app.locale, "Image").into_owned(),
                    str::to_owned,
                )
        });
    let message = preview.message().to_owned();
    // The file already on the computer, when the item has one: the header
    // offers Save as for it, as the message menu does, instead of fetching it
    // again and leaving the viewer.
    let saved = item.and_then(|item| item.path.clone());
    let has_message = item.is_some();
    ui.horizontal(|ui| {
        // The actions are laid out from the right edge first, and the title
        // takes what is left of the row, truncated: a long chat name used to
        // run under the icons.
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if theme::icon_button(
                ui,
                Icon::X,
                18.0,
                palette.secondary,
                palette.text,
                crate::i18n::gettext(app.locale, "Close preview (Esc)").as_ref(),
            )
            .clicked()
            {
                actions.push(Action::CloseImagePreview);
            }
            if let Some(path) = preview.path() {
                if theme::icon_button(
                    ui,
                    Icon::ExternalLink,
                    18.0,
                    palette.secondary,
                    palette.text,
                    crate::i18n::gettext(app.locale, "Open in another app").as_ref(),
                )
                .clicked()
                {
                    actions.push(Action::OpenFile(path.to_owned()));
                }
                let copy_hint = format!(
                    "{} ({})",
                    crate::i18n::gettext(app.locale, "Copy image"),
                    super::keys::label("Ctrl+C"),
                );
                // A clip is played, never decoded as an image, so copying it
                // could only fail: the action is left out here, and the
                // shortcut is gated on the same rule in `keys`.
                if !clip
                    && theme::icon_button(
                        ui,
                        Icon::Copy,
                        18.0,
                        palette.secondary,
                        palette.text,
                        &copy_hint,
                    )
                    .clicked()
                {
                    actions.push(Action::CopyImage(path.to_owned()));
                }
            }
            ui.add_space(8.0);
            // Right to left: zoom in, the current scale, zoom out.
            if theme::icon_button(
                ui,
                Icon::Plus,
                18.0,
                palette.secondary,
                palette.text,
                crate::i18n::gettext(app.locale, "Zoom in").as_ref(),
            )
            .clicked()
            {
                actions.push(Action::ZoomImageIn);
            }
            // One control shows the scale and switches between fitting the
            // window and the original size.
            let (label, hint, action) = if preview.is_fit() {
                (
                    crate::i18n::gettext(app.locale, "Fit").into_owned(),
                    crate::i18n::gettext(app.locale, "Show at original size"),
                    Action::ImageActualSize,
                )
            } else {
                (
                    format!("{:.0}%", preview.zoom() * 100.0),
                    crate::i18n::gettext(app.locale, "Fit to the window (0)"),
                    Action::FitImage,
                )
            };
            if theme::soft_button(ui, &palette, None, &label, false)
                .on_hover_text(hint)
                .clicked()
            {
                actions.push(action);
            }
            if theme::icon_button(
                ui,
                Icon::Minus,
                18.0,
                palette.secondary,
                palette.text,
                crate::i18n::gettext(app.locale, "Zoom out").as_ref(),
            )
            .clicked()
            {
                actions.push(Action::ZoomImageOut);
            }
            // What the message itself can do, when the viewer knows which one
            // it is. A picture opened on its own has nothing to act on.
            if has_message {
                ui.add_space(8.0);
                if theme::icon_button(
                    ui,
                    Icon::Reply,
                    18.0,
                    palette.secondary,
                    palette.text,
                    crate::i18n::gettext(app.locale, "Reply").as_ref(),
                )
                .clicked()
                {
                    actions.push(Action::CloseImagePreview);
                    actions.push(Action::Reply(message.clone()));
                }
                if theme::icon_button(
                    ui,
                    Icon::Smile,
                    18.0,
                    palette.secondary,
                    palette.text,
                    crate::i18n::gettext(app.locale, "React").as_ref(),
                )
                .clicked()
                {
                    // The viewer draws after the picker, so it has to go first:
                    // otherwise the picker opens behind it and cannot be used.
                    actions.push(Action::CloseImagePreview);
                    actions.push(Action::OpenReactionPicker {
                        chat: chat.to_owned(),
                        message: message.clone(),
                        beside_menu: false,
                    });
                }
                if theme::icon_button(
                    ui,
                    Icon::Forward,
                    18.0,
                    palette.secondary,
                    palette.text,
                    crate::i18n::gettext(app.locale, "Forward").as_ref(),
                )
                .clicked()
                {
                    actions.push(Action::CloseImagePreview);
                    actions.push(Action::ShowDialog(Dialog::Forward {
                        chat: chat.to_owned(),
                        messages: vec![message.clone()],
                    }));
                }
                match saved {
                    // Already on the computer: hand it to the save dialog, the
                    // way the message menu does.
                    Some(path) => {
                        let name = path
                            .file_name()
                            .and_then(|name| name.to_str())
                            .unwrap_or("attachment")
                            .to_owned();
                        if theme::icon_button(
                            ui,
                            Icon::Download,
                            18.0,
                            palette.secondary,
                            palette.text,
                            crate::i18n::gettext(app.locale, "Save as…").as_ref(),
                        )
                        .clicked()
                        {
                            actions.push(Action::CloseImagePreview);
                            actions.push(Action::SaveAttachmentAs { path, name });
                        }
                    }
                    // Not here yet: fetch it, and return to the chat with it.
                    None => {
                        if theme::icon_button(
                            ui,
                            Icon::Download,
                            18.0,
                            palette.secondary,
                            palette.text,
                            crate::i18n::gettext(app.locale, "Download").as_ref(),
                        )
                        .clicked()
                        {
                            actions.push(Action::CloseImagePreview);
                            actions.push(Action::Download {
                                card: None,
                                chat: chat.to_owned(),
                                message: message.clone(),
                            });
                        }
                    }
                }
                if theme::icon_button(
                    ui,
                    Icon::MessageCircle,
                    18.0,
                    palette.secondary,
                    palette.text,
                    crate::i18n::gettext(app.locale, "Show in the chat").as_ref(),
                )
                .clicked()
                {
                    // Closing leaves the chat where it was, so the message the
                    // viewer was showing is brought into view. `ScrollTo` also
                    // pages older archive history toward it when it is not in
                    // the transcript yet, which is the usual case after
                    // stepping back through the album.
                    actions.push(Action::CloseImagePreview);
                    actions.push(Action::ScrollTo(message.clone()));
                }
                ui.add_space(8.0);
            }
            // Last, so it takes what the actions left, and truncated rather
            // than drawn under them.
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                crate::ui::widgets::rich_text(ui, &title, theme::semibold(14.0), palette.text);
            });
        });
    });
}

/// The picture itself, zoomed and panned exactly as before.
#[allow(clippy::too_many_arguments)]
fn still(
    app: &mut App,
    ui: &mut egui::Ui,
    preview: &mut PreviewState,
    path: Option<&Path>,
    thumbnail: Option<&[u8]>,
    video: bool,
    chat: &str,
    message: &str,
    stage: Rect,
    actions: &mut Vec<Action>,
) {
    let palette = app.palette;
    let canvas = stage.size();
    // An item whose attachment is not here yet is shown for what it is, with
    // its thumbnail and a way to fetch it, instead of standing in for another
    // picture.
    let Some(path) = path else {
        pending(app, ui, thumbnail, video, message, chat, stage, actions);
        return;
    };
    // Registered with the image cache like every other draw site, so a sweep
    // never releases the picture while it is on screen.
    let image = crate::ui::widgets::file_image(ui, path);
    match image.load_for_size(ui.ctx(), canvas) {
        Ok(egui::load::TexturePoll::Ready { texture }) => {
            let size = display_size(texture.size, canvas, preview.is_fit(), preview.zoom());
            if preview.is_fit() && texture.size.x > 0.0 {
                preview.set_fit_scale(size.x / texture.size.x);
            }
            egui::ScrollArea::both()
                .id_salt("image-preview-scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.allocate_ui_with_layout(
                        canvas.max(size),
                        Layout::centered_and_justified(egui::Direction::TopDown),
                        |ui| {
                            let image_response =
                                ui.add(image.fit_to_exact_size(size).sense(egui::Sense::click()));
                            // The same three actions the header offers, on the
                            // picture itself: copy it, save it, or hand it to
                            // another app.
                            let copy_label = crate::i18n::gettext(app.locale, "Copy image");
                            let save_label = crate::i18n::gettext(app.locale, "Save as…");
                            let open_label =
                                crate::i18n::gettext(app.locale, "Open in another app");
                            let menu_width = crate::ui::widgets::menu_width(
                                ui,
                                &[&copy_label, &save_label, &open_label],
                                true,
                            )
                            .max(180.0);
                            egui::Popup::context_menu(&image_response)
                                .width(menu_width)
                                .frame(crate::ui::widgets::menu_frame(&palette))
                                .show(|ui| {
                                    if crate::ui::widgets::menu_item(
                                        ui,
                                        &palette,
                                        Some(Icon::Copy),
                                        &copy_label,
                                    ) {
                                        app.actions.push(Action::CopyImage(path.to_owned()));
                                    }
                                    if crate::ui::widgets::menu_item(
                                        ui,
                                        &palette,
                                        Some(Icon::Download),
                                        &save_label,
                                    ) {
                                        let name = path
                                            .file_name()
                                            .and_then(|name| name.to_str())
                                            .unwrap_or("image.png")
                                            .to_owned();
                                        app.actions.push(Action::SaveAttachmentAs {
                                            path: path.to_owned(),
                                            name,
                                        });
                                    }
                                    if crate::ui::widgets::menu_item(
                                        ui,
                                        &palette,
                                        Some(Icon::ExternalLink),
                                        &open_label,
                                    ) {
                                        app.actions.push(Action::OpenFile(path.to_owned()));
                                    }
                                });
                        },
                    );
                });
        }
        Ok(egui::load::TexturePoll::Pending { .. }) => {
            theme::paint_spinner(ui, stage, 28.0, palette.accent);
        }
        Err(_) => {
            let path = path.to_owned();
            ui.allocate_ui_with_layout(
                canvas,
                Layout::centered_and_justified(egui::Direction::TopDown),
                |ui| {
                    ui.label(crate::i18n::gettext(
                        app.locale,
                        "This image could not be displayed in ZapFast.",
                    ));
                    if ui
                        .button(crate::i18n::gettext(app.locale, "Open externally"))
                        .clicked()
                    {
                        app.actions.push(Action::OpenFile(path.clone()));
                    }
                },
            );
        }
    }
}

/// An item whose attachment has not been downloaded: its own thumbnail, the
/// clip or picture it is, and the button that fetches it. Nothing here stands
/// in for another file.
#[allow(clippy::too_many_arguments)]
fn pending(
    app: &App,
    ui: &mut egui::Ui,
    thumbnail: Option<&[u8]>,
    video: bool,
    message: &str,
    chat: &str,
    stage: Rect,
    actions: &mut Vec<Action>,
) {
    let palette = app.palette;
    let disc = Rect::from_center_size(stage.center(), Vec2::splat(220.0));
    if let Some(bytes) = thumbnail {
        egui::Image::new(thumbnail_uri(ui.ctx(), chat, message, bytes))
            .fit_to_exact_size(disc.size())
            .corner_radius(8.0)
            .paint_at(ui, disc);
    } else {
        theme::paint_icon(
            ui,
            if video { Icon::Video } else { Icon::Image },
            disc,
            48.0,
            palette.secondary,
        );
    }
    let button = Rect::from_center_size(
        pos2(stage.center().x, disc.bottom() + 34.0),
        vec2(160.0, 32.0),
    );
    let response = ui
        .interact(
            button,
            egui::Id::new("viewer-pending-download"),
            Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    ui.painter().rect_filled(button, 6.0, palette.surface_hover);
    ui.painter().text(
        button.center(),
        egui::Align2::CENTER_CENTER,
        crate::i18n::gettext(app.locale, "Download"),
        theme::regular(13.5),
        palette.text,
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            ui.is_enabled(),
            crate::i18n::gettext(app.locale, "Download").as_ref(),
        )
    });
    theme::reveal_focus(&response);
    if response.clicked() {
        actions.push(Action::Download {
            card: None,
            chat: chat.to_owned(),
            message: message.to_owned(),
        });
    }
}

/// A video plays in place, through the same player the chat bubble uses, so it
/// carries sound and its position is the one the player reports.
#[allow(clippy::too_many_arguments)]
fn video(
    app: &mut App,
    ui: &mut egui::Ui,
    path: Option<&Path>,
    thumbnail: Option<&[u8]>,
    stage: Rect,
    chat: &str,
    message: &str,
    actions: &mut Vec<Action>,
) {
    let palette = app.palette;
    // A clip keeps its own shape: the decoded frame knows it, and 16:9 is only
    // what an undecoded poster falls back to.
    let status = app.video.status(message);
    let aspect = status
        .as_ref()
        .and_then(|status| status.frame.as_ref())
        .map(|frame| frame.size())
        .filter(|size| size[0] > 0 && size[1] > 0)
        .map_or_else(
            || vec2(16.0, 9.0),
            |size| vec2(size[0] as f32, size[1] as f32),
        );
    let size = fit(aspect, stage.size());
    let media = Rect::from_center_size(stage.center(), size);
    // The player pauses a clip it has not been told about for 1.5 s, so the
    // viewer has to say it is still on screen, as the chat renderer does.
    app.video.saw(message);
    match status.as_ref().and_then(|status| status.frame.clone()) {
        Some(frame) => {
            ui.painter().image(
                frame.id(),
                media,
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        }
        None => {
            ui.painter().rect_filled(media, 8.0, palette.shadow);
            if let Some(bytes) = thumbnail {
                egui::Image::new(thumbnail_uri(ui.ctx(), chat, message, bytes))
                    .fit_to_exact_size(media.size())
                    .corner_radius(8.0)
                    .paint_at(ui, media);
            }
        }
    }
    // Clicking the frame plays or pauses, as in the bubble. A clip that is not
    // here yet is offered for download instead.
    let response = ui
        .interact(media, egui::Id::new("viewer-video"), Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let label = if path.is_some() {
        crate::i18n::gettext(app.locale, "Play or pause the video")
    } else {
        crate::i18n::gettext(app.locale, "Download")
    };
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label.as_ref())
    });
    theme::reveal_focus(&response);
    if response.clicked() {
        match path {
            Some(path) => actions.push(Action::PlayVideo {
                message: message.to_owned(),
                path: path.to_owned(),
            }),
            None => actions.push(Action::Download {
                card: None,
                chat: chat.to_owned(),
                message: message.to_owned(),
            }),
        }
    }
    let playing = status
        .as_ref()
        .is_some_and(|status| status.state == crate::video::State::Playing);
    if playing {
        // A thin line along the bottom of the frame, so a long clip has a
        // position. It sits inside the frame: the body is clipped to the
        // stage, and a wide window makes the frame as tall as the stage, so a
        // bar below it would be painted outside the clip and never seen.
        let bar = Rect::from_min_size(
            pos2(media.left(), media.bottom() - 7.0),
            vec2(media.width(), 3.0),
        );
        ui.painter().rect_filled(bar, 1.5, palette.shadow);
        let fraction = status.map_or(0.0, |status| status.fraction());
        let played = Rect::from_min_size(bar.min, vec2(bar.width() * fraction, bar.height()));
        ui.painter().rect_filled(played, 1.5, palette.accent);
    } else {
        let disc = Rect::from_center_size(media.center(), Vec2::splat(64.0));
        ui.painter()
            .circle_filled(disc.center(), 32.0, palette.shadow);
        // A control painted over a picture, so it stays light on its own scrim
        // in either theme, exactly as the bubble paints the same button.
        theme::paint_icon(ui, Icon::Play, disc, 28.0, egui::Color32::WHITE);
    }
}

/// The album along the bottom: a thumbnail per item, the current one ringed.
#[allow(clippy::too_many_arguments)]
fn strip_bar(
    app: &App,
    ui: &mut egui::Ui,
    album: &[ChatMedia],
    chat: &str,
    current: &str,
    rect: Rect,
    actions: &mut Vec<Action>,
) {
    let palette = app.palette;
    ui.painter().rect_filled(rect, 0.0, palette.shadow);
    let inner = rect.shrink2(vec2(STRIP_PAD, STRIP_PAD));
    // Only the room the row cannot fill becomes padding. Adding half the width
    // on each side whatever the row needs made the content wider than the
    // strip, so the bar showed even for three items.
    let row = album.len() as f32 * THUMB + album.len().saturating_sub(1) as f32 * STRIP_GAP;
    let pad = ((inner.width() - row) * 0.5).max(0.0);
    let mut child = ui.new_child(UiBuilder::new().max_rect(inner));
    // The app style floats scrollbars over content. This strip needs the bar
    // under the thumbs, with a handle that is not the same colour as its rail.
    {
        let scroll = &mut child.spacing_mut().scroll;
        scroll.floating = false;
        scroll.bar_width = 6.0;
        scroll.bar_inner_margin = 3.0;
        scroll.bar_outer_margin = 2.0;
        scroll.foreground_color = true;
    }
    egui::ScrollArea::horizontal()
        .id_salt("viewer-strip")
        .auto_shrink([false, false])
        .show(&mut child, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                ui.set_min_height(THUMB);
                ui.add_space(pad);
                for (index, item) in album.iter().enumerate() {
                    if index > 0 {
                        ui.add_space(STRIP_GAP);
                    }
                    let (thumb_rect, response) =
                        ui.allocate_exact_size(Vec2::splat(THUMB), Sense::click());
                    if item.id == current {
                        // Keep the picture being looked at in the middle of the
                        // strip as the album is stepped through.
                        ui.scroll_to_rect_animation(
                            thumb_rect,
                            Some(egui::Align::Center),
                            egui::style::ScrollAnimation::none(),
                        );
                    }
                    if ui.is_rect_visible(thumb_rect) {
                        thumb(ui, &palette, chat, item, thumb_rect, item.id == current);
                    }
                    // Custom-painted and clickable, so it needs the same focus
                    // reveal and label every other custom control registers.
                    // The kind is looked up on its own so both literals stay
                    // direct arguments of `gettext` and the extractor emits
                    // them; picking between them inside the call hides both.
                    let kind = if item.video {
                        crate::i18n::gettext(app.locale, "Video")
                    } else {
                        crate::i18n::gettext(app.locale, "Photo")
                    };
                    let label = format!("{} {}", kind, index + 1);
                    response.widget_info(|| {
                        egui::WidgetInfo::selected(
                            egui::WidgetType::Button,
                            ui.is_enabled(),
                            item.id == current,
                            &label,
                        )
                    });
                    theme::reveal_focus(&response);
                    if response
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .clicked()
                    {
                        actions.push(Action::ViewImage {
                            message: item.id.clone(),
                        });
                    }
                }
                ui.add_space(pad);
            });
        });
}

fn thumb(
    ui: &egui::Ui,
    palette: &crate::theme::Palette,
    chat: &str,
    item: &ChatMedia,
    rect: Rect,
    current: bool,
) {
    ui.painter().rect_filled(rect, 6.0, palette.surface_hover);
    // The stored thumbnail first: it is the 56 px the strip wants, and a
    // downloaded file is not an image, so decoding it here would show a load
    // error for every clip. The file itself is only a fallback, and through
    // `file_image` so egui releases its texture.
    if let Some(bytes) = item.thumbnail.as_deref() {
        // A message id is only unique inside its chat, and the image cache
        // keeps the first bytes for a URI, so the chat has to be part of it.
        egui::Image::new(thumbnail_uri(ui.ctx(), chat, &item.id, bytes))
            .fit_to_exact_size(rect.size())
            .corner_radius(6.0)
            .paint_at(ui, rect);
    } else if let Some(path) = item
        .path
        .as_deref()
        .filter(|_| !item.video)
        .filter(|path| path.is_file())
    {
        crate::ui::widgets::file_image(ui, path)
            .fit_to_exact_size(rect.size())
            .corner_radius(6.0)
            .paint_at(ui, rect);
    } else {
        theme::paint_icon(
            ui,
            if item.video { Icon::Video } else { Icon::Image },
            rect,
            22.0,
            palette.secondary,
        );
    }
    if item.video {
        let disc = Rect::from_center_size(rect.center(), Vec2::splat(18.0));
        ui.painter()
            .circle_filled(disc.center(), 9.0, palette.shadow);
        // Over the thumbnail, so it stays light in either theme.
        theme::paint_icon(ui, Icon::Play, disc, 12.0, egui::Color32::WHITE);
    }
    if current {
        ui.painter().rect_stroke(
            rect,
            CornerRadius::same(6),
            Stroke::new(2.0, palette.accent),
            egui::StrokeKind::Inside,
        );
    }
}

fn chevrons(
    app: &App,
    ui: &mut egui::Ui,
    palette: &crate::theme::Palette,
    stage: Rect,
    actions: &mut Vec<Action>,
) {
    // The tooltip names the key that steps: the arrows stay with the modal, so
    // "Left" and "Right" were describing a shortcut nobody had.
    for (left, icon, step, hint) in [
        (
            true,
            Icon::ChevronLeft,
            -1i8,
            crate::i18n::gettext(app.locale, "Previous (,)").as_ref(),
        ),
        (
            false,
            Icon::ChevronRight,
            1i8,
            crate::i18n::gettext(app.locale, "Next (.)").as_ref(),
        ),
    ] {
        let x = if left {
            stage.left() + 28.0
        } else {
            stage.right() - 28.0
        };
        let hit = Rect::from_center_size(pos2(x, stage.center().y), Vec2::splat(44.0));
        let mut child = ui.new_child(
            UiBuilder::new()
                .max_rect(hit)
                .layout(Layout::centered_and_justified(egui::Direction::LeftToRight)),
        );
        if theme::circle_button(
            &mut child,
            icon,
            36.0,
            palette.shadow,
            palette.surface_hover,
            palette.text,
            hint,
        )
        .clicked()
        {
            actions.push(Action::ViewerStep(step));
        }
    }
}

/// Size the image is drawn at from the texture's intrinsic pixel dimensions:
/// fitted into the canvas, or scaled by the preview's zoom factor. Zoom is
/// applied here only. The size hint passed when loading does not change the
/// texture: egui decodes raster formats (all the preview accepts) once at full
/// resolution and reports the source size, whatever size is asked for.
fn display_size(original: Vec2, canvas: Vec2, fit: bool, zoom: f32) -> Vec2 {
    let (width, height) = if fit {
        crate::image_preview::fit_size(original.x, original.y, canvas.x, canvas.y)
    } else {
        crate::image_preview::zoomed_size(original.x, original.y, zoom)
    };
    vec2(width, height)
}

/// Scales `size` to fit inside `max`, keeping its aspect ratio.
fn fit(size: Vec2, max: Vec2) -> Vec2 {
    if size.x <= 0.0 || size.y <= 0.0 {
        return max;
    }
    let scale = (max.x / size.x).min(max.y / size.y);
    size * scale
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fitted_images_keep_aspect_ratio_inside_the_canvas() {
        assert_eq!(
            display_size(vec2(1600.0, 1200.0), vec2(800.0, 700.0), true, 1.0),
            vec2(800.0, 600.0)
        );
        assert_eq!(
            display_size(vec2(320.0, 240.0), vec2(800.0, 700.0), true, 1.0),
            vec2(320.0, 240.0)
        );
        assert_eq!(
            display_size(vec2(320.0, 240.0), vec2(800.0, 700.0), false, 2.0),
            vec2(640.0, 480.0)
        );
    }

    #[test]
    fn the_panes_fill_the_window_top_to_bottom() {
        let window = Rect::from_min_size(pos2(0.0, 0.0), vec2(1400.0, 900.0));
        let (header, stage, strip) = panes(window, true);
        assert_eq!(header.min, window.min, "the header starts at the top edge");
        assert_eq!(header.height(), HEADER);
        assert_eq!(strip.max, window.max, "the strip ends at the bottom edge");
        assert_eq!(strip.height(), STRIP);
        assert_eq!(stage.min.y, header.max.y, "no gap under the header");
        assert_eq!(stage.max.y, strip.min.y, "no gap over the strip");
        assert_eq!(stage.width(), window.width(), "the stage is full width");
        // Without an album the stage keeps the room the strip would have taken.
        let (_, stage, strip) = panes(window, false);
        assert!(strip.height() < f32::EPSILON);
        assert_eq!(stage.max.y, window.max.y);
    }

    #[test]
    fn a_video_frame_keeps_its_aspect_ratio_inside_the_stage() {
        assert_eq!(
            fit(vec2(1920.0, 1080.0), vec2(800.0, 600.0)),
            vec2(800.0, 450.0)
        );
        assert_eq!(fit(vec2(0.0, 0.0), vec2(800.0, 600.0)), vec2(800.0, 600.0));
    }

    /// The row of thumbnails never asks for more room than it has, so the strip
    /// keeps its bar for an album that overflows and not for one that fits.
    #[test]
    fn the_strip_pads_only_the_room_the_row_does_not_use() {
        let inner = Rect::from_min_size(pos2(0.0, 0.0), vec2(600.0, 70.0));
        let row = |count: usize| count as f32 * THUMB + count.saturating_sub(1) as f32 * STRIP_GAP;
        let pad = |count: usize| ((inner.width() - row(count)) * 0.5).max(0.0);
        // Three thumbnails fit, so the padding centres them and the content is
        // exactly the width of the strip.
        assert_eq!(row(3) + 2.0 * pad(3), inner.width());
        // More than the strip holds: no padding at all, and the bar appears
        // because the row is wider than the strip.
        assert_eq!(pad(40), 0.0);
        assert!(row(40) > inner.width());
    }
}
