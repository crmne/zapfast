//! Message body content, carousels, album grids, interactive buttons, and location cards.

use super::*;

pub struct MessageContent;

impl MessageContent {
    pub(crate) fn show(
        ui: &mut egui::Ui,
        view: &View<'_>,
        message: &Message,
        width: f32,
        reserve: f32,
        actions: &mut Vec<Action>,
    ) -> Option<Rect> {
        content(ui, view, message, width, reserve, actions)
    }
}

/// Draws a message body and returns optional footer space on its last line.
pub(crate) fn content(
    ui: &mut egui::Ui,
    view: &View<'_>,
    message: &Message,
    width: f32,
    reserve: f32,
    actions: &mut Vec<Action>,
) -> Option<Rect> {
    let palette = view.palette;
    let own = message.from_me;
    // Add non-text messages to cross-message transcript copies.
    let has_body = match &message.content {
        Content::Text { .. } => true,
        // Image-only cards and carousels without a body draw no text.
        Content::Interactive { card, .. } => card.as_ref().is_none_or(|card| {
            !card.body.is_empty() || (card.image.is_none() && card.carousel.is_empty())
        }),
        Content::Image { caption, .. }
        | Content::Video { caption, .. }
        | Content::Document { caption, .. } => caption.is_some(),
        _ => false,
    };
    if !has_body && let Some(copy_rows) = view.copy_rows {
        copy_rows
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(transcript_row(view, message, String::new(), Vec::new()));
    }
    match &message.content {
        Content::Interactive { text, card } => {
            let Some(card) = card else {
                let span = message.quoted.is_some().then_some(width);
                return rich_body(ui, view, message, text, width, Some(reserve), span, actions);
            };
            if !card.carousel.is_empty() {
                carousel(ui, view, message, card, width, actions);
                return None;
            }
            if let Some(image) = &card.image {
                picture(ui, view, message, image, width, None, actions);
                ui.add_space(4.0);
                if card.body.is_empty() {
                    return None;
                }
            }
            let body = if card.body.is_empty() {
                "Interactive message"
            } else {
                &card.body
            };
            rich_body(
                ui,
                view,
                message,
                body,
                width,
                Some(reserve),
                Some(width),
                actions,
            )
        }
        Content::Text { text, preview } => {
            if let Some(preview) = preview {
                preview_card(ui, view, message, preview, width, actions);
            }
            let span = (message.quoted.is_some() || preview.is_some()).then_some(width);
            rich_body(ui, view, message, text, width, Some(reserve), span, actions)
        }
        Content::Image {
            caption,
            media,
            motion,
        } => {
            let drawn = picture(ui, view, message, media, width, None, actions);
            if let Some(motion) = motion {
                motion_clip(ui, view, message, motion, drawn, actions);
            }
            ui.ctx().data_mut(|data| {
                data.insert_temp(bubble_id(&view.chat.id, &message.id).with("picture"), drawn);
            });
            let Some(caption) = caption else {
                // The time and ticks go over the picture's corner.
                return Some(drawn);
            };
            let drawn = drawn.width();
            {
                // Wrap the caption to the settled image or quote width.
                let wrap = if message.quoted.is_some() {
                    width.max(drawn)
                } else {
                    drawn
                };
                rich_body(
                    ui,
                    view,
                    message,
                    caption,
                    wrap,
                    Some(reserve),
                    Some(wrap),
                    actions,
                )
            }
        }
        Content::Sticker { media, animated } => {
            picture(ui, view, message, media, width, Some(*animated), actions);
            None
        }
        Content::Video {
            caption,
            media,
            seconds,
            gif,
            note,
        } => {
            let drawn = if *note {
                video_note(ui, view, message, media, *seconds, actions)
            } else {
                video(ui, view, message, media, *seconds, *gif, width, actions)
            };
            caption.as_ref().and_then(|caption| {
                let wrap = if message.quoted.is_some() {
                    width.max(drawn)
                } else {
                    drawn
                };
                rich_body(
                    ui,
                    view,
                    message,
                    caption,
                    wrap,
                    Some(reserve),
                    Some(wrap),
                    actions,
                )
            })
        }
        Content::Audio {
            media,
            seconds,
            waveform,
            ..
        } => voice_player(
            ui, view, message, media, *seconds, waveform, width, reserve, actions,
        ),
        Content::Document {
            media,
            file_name,
            caption,
            pages,
        } => {
            let mut detail = Vec::new();
            if let Some(pages) = pages {
                detail.push(format!(
                    "{pages} page{}",
                    if *pages == 1 { "" } else { "s" }
                ));
            }
            detail.push(crate::util::bytes(media.size));
            attachment(
                ui,
                view,
                message,
                media,
                Icon::FileText,
                file_name,
                &detail.join(" · "),
                width,
                actions,
            );
            caption.as_ref().and_then(|caption| {
                rich_body(
                    ui,
                    view,
                    message,
                    caption,
                    width,
                    Some(reserve),
                    Some(width),
                    actions,
                )
            })
        }
        Content::Location {
            latitude,
            longitude,
            name,
            address,
        } => {
            let title = name
                .clone()
                .unwrap_or_else(|| crate::i18n::gettext(view.locale, "Location").into_owned());
            let detail = address.clone().or_else(|| {
                Some(
                    Spot {
                        latitude: *latitude,
                        longitude: *longitude,
                    }
                    .text(),
                )
            });
            location_card(
                ui,
                view,
                message,
                &message.id,
                &title,
                detail,
                None,
                (*latitude, *longitude),
                actions,
            );
            None
        }
        Content::LiveLocation {
            latitude,
            longitude,
            accuracy_m,
            speed_mps,
            sequence,
            updated,
            newer_on_phone,
            ..
        } => {
            let over = message
                .content
                .live_location_over(message.timestamp, view.now);
            if !over {
                let until_expiry =
                    (message.timestamp + crate::model::LIVE_LOCATION_LIMIT - view.now).max(1);
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_secs(until_expiry as u64));
            }
            let title = if over {
                crate::i18n::gettext(view.locale, "Live location ended")
            } else {
                crate::i18n::gettext(view.locale, "Live location")
            };
            let detail = (!over).then(|| {
                // An absolute time stays true without repainting.
                let at = if *updated > 0 {
                    *updated
                } else {
                    message.timestamp
                };
                let mut meta = vec![
                    crate::i18n::gettext(view.locale, "Updated {time}")
                        .replace("{time}", &crate::util::clock(at)),
                ];
                if let Some(speed) = speed_mps.filter(|speed| *speed >= 0.5) {
                    meta.push(format!("{:.0} km/h", speed * 3.6));
                }
                if let Some(accuracy) = accuracy_m {
                    meta.push(format!("±{accuracy} m"));
                }
                meta.join(" · ")
            });
            let note = (!over && *newer_on_phone).then(|| {
                crate::i18n::gettext(view.locale, "Open WhatsApp on your phone to follow it.")
                    .into_owned()
            });
            // Each position gets its own preview, as a later one replaces the first.
            let key = format!("{}-{sequence}-{updated}", message.id);
            location_card(
                ui,
                view,
                message,
                &key,
                &title,
                detail,
                note,
                (*latitude, *longitude),
                actions,
            );
            None
        }
        Content::Contact {
            display_name,
            vcard,
        } => {
            let icon = |ui: &mut egui::Ui| {
                theme::icon(ui, Icon::Contact, 18.0, palette.accent);
            };
            let details = shared_contact_details(vcard, display_name);
            mirrored_row(ui, own, icon, |ui| {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 5.0;
                    let name = details
                        .as_ref()
                        .map_or(display_name.as_str(), |contact| contact.name.as_str());
                    widgets::rich_text(ui, name, theme::medium(14.0), palette.text);
                    match details.as_ref() {
                        Some(SharedContact {
                            account: Some(account),
                            first_name,
                            ..
                        }) => {
                            let id = format!("{account}@s.whatsapp.net");
                            ui.horizontal(|ui| {
                                if theme::pill_button(
                                    ui,
                                    &palette,
                                    crate::i18n::gettext(view.locale, "Chat").as_ref(),
                                    true,
                                )
                                .clicked()
                                {
                                    actions.push(Action::StartChat {
                                        id: id.clone(),
                                        name: name.to_owned(),
                                        dismiss_dialog: true,
                                    });
                                }
                                if !view.contacts.contains_key(&id)
                                    && theme::pill_button(
                                        ui,
                                        &palette,
                                        crate::i18n::gettext(view.locale, "Add").as_ref(),
                                        false,
                                    )
                                    .clicked()
                                {
                                    let (first, last) =
                                        crate::util::editor_names(name, first_name.as_deref());
                                    actions.push(Action::NewContact {
                                        phone: account.clone(),
                                        first,
                                        last,
                                        to_phone: None,
                                    });
                                }
                            });
                        }
                        Some(contact) => {
                            theme::text(
                                ui,
                                &contact.number,
                                theme::regular(12.5),
                                palette.secondary,
                            );
                        }
                        None => {}
                    }
                });
            });
            None
        }
        Content::StickerPack {
            name,
            publisher,
            count,
            caption,
        } => {
            let icon = |ui: &mut egui::Ui| {
                theme::icon(ui, Icon::Sticker, 20.0, palette.accent);
            };
            mirrored_row(ui, own, icon, |ui| {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    widgets::rich_text(ui, name, theme::medium(14.0), palette.text);
                    let stickers =
                        crate::i18n::ngettext(view.locale, "{} sticker", "{} stickers", *count)
                            .replace("{}", &count.to_string());
                    let detail = if publisher.trim().is_empty() {
                        stickers
                    } else {
                        format!("{publisher} · {stickers}")
                    };
                    widgets::rich_text(ui, &detail, theme::regular(12.5), palette.secondary);
                    if let Some(caption) = caption {
                        widgets::rich_text(ui, caption, theme::regular(13.5), palette.text);
                    }
                    if theme::link(
                        ui,
                        crate::i18n::gettext(view.locale, "View stickers"),
                        theme::regular(12.5),
                        palette.link,
                    )
                    .clicked()
                    {
                        actions.push(Action::ViewStickerPack(message.id.clone()));
                    }
                });
            });
            None
        }
        Content::Poll { .. } => {
            super::polls::ballot(
                ui,
                &palette,
                view.locale,
                message,
                width,
                view.connected,
                view.poll_voting
                    .contains(&(message.chat.clone(), message.id.clone())),
                actions,
            );
            None
        }
        Content::Call {
            video,
            status,
            duration,
            group,
            participants,
        } => {
            let entry = super::calls::Entry {
                from_me: own,
                video: *video,
                status: *status,
                duration: *duration,
                group: *group,
                participants: *participants,
            };
            let tint = if status.is_bad() {
                palette.danger
            } else {
                palette.accent
            };
            call_entry(ui, &palette, &entry, view.locale, tint, width, reserve)
        }
        Content::Revoked => {
            mirrored_row(
                ui,
                own,
                |ui| {
                    theme::icon(ui, Icon::Ban, 14.0, palette.dim);
                },
                |ui| {
                    theme::text(
                        ui,
                        "This message was deleted",
                        theme::regular(13.5),
                        palette.secondary,
                    );
                },
            );
            None
        }
        Content::PhoneOnly {
            live_location: true,
            ..
        } => {
            mirrored_row(
                ui,
                own,
                |ui| {
                    theme::icon(ui, Icon::MapPin, 18.0, palette.accent);
                },
                |ui| {
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 1.0;
                        widgets::rich_text(
                            ui,
                            &crate::i18n::gettext(view.locale, "Live location"),
                            theme::medium(14.0),
                            palette.text,
                        );
                        widgets::rich_text(
                            ui,
                            &crate::i18n::gettext(
                                view.locale,
                                "Open WhatsApp on your phone to follow it.",
                            ),
                            theme::regular(12.5),
                            palette.secondary,
                        );
                    });
                },
            );
            None
        }
        Content::PhoneOnly {
            view_once, once, ..
        } => {
            use crate::i18n::gettext;
            use crate::model::OnceMedia;
            let text = match once {
                Some(OnceMedia::Photo) => gettext(
                    view.locale,
                    "View once photo. For your privacy, it opens only on your phone.",
                ),
                Some(OnceMedia::Video) => gettext(
                    view.locale,
                    "View once video. For your privacy, it opens only on your phone.",
                ),
                Some(OnceMedia::Voice) => gettext(
                    view.locale,
                    "View once voice message. For your privacy, it opens only on your phone.",
                ),
                Some(OnceMedia::Audio) => gettext(
                    view.locale,
                    "View once audio. For your privacy, it opens only on your phone.",
                ),
                None if *view_once => gettext(
                    view.locale,
                    "View once message. For your privacy, it opens only on your phone.",
                ),
                None => gettext(view.locale, "This message can only be seen on your phone."),
            };
            mirrored_row(
                ui,
                own,
                |ui| {
                    theme::icon(ui, Icon::Smartphone, 14.0, palette.dim);
                },
                |ui| {
                    theme::text(ui, text, theme::regular(13.5), palette.secondary);
                },
            );
            None
        }
        Content::Unsupported { what } => {
            mirrored_row(
                ui,
                own,
                |ui| {
                    theme::icon(ui, Icon::CircleAlert, 14.0, palette.dim);
                },
                |ui| {
                    theme::text(
                        ui,
                        format!("Unsupported: {what}"),
                        theme::regular(13.5),
                        palette.secondary,
                    );
                },
            );
            None
        }
        Content::DisappearingTimer { .. } => {
            None
        }
    }
}

pub(crate) fn carousel(
    ui: &mut egui::Ui,
    view: &View<'_>,
    message: &Message,
    card: &crate::model::InteractiveCard,
    width: f32,
    actions: &mut Vec<Action>,
) {
    if !card.body.is_empty() {
        rich_body(
            ui,
            view,
            message,
            &card.body,
            width,
            None,
            Some(width),
            actions,
        );
        ui.add_space(4.0);
    }
    // Reserve a glimpse of the next card only when there is another card.
    let inset = if card.carousel.len() > 1 { 36.0 } else { 20.0 };
    let card_width = CAROUSEL_CARD_WIDTH.min((width - inset).max(140.0));
    let id = bubble_id(&message.chat, &message.id);
    for direction in [-1, 1] {
        ui.ctx().data_mut(|data| {
            data.remove::<Rect>(id.with(("carousel-arrow", direction)));
        });
    }
    let output = egui::ScrollArea::horizontal()
        .id_salt(("carousel", &message.chat, &message.id))
        .max_width(width)
        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
        .auto_shrink([false, true])
        .show_viewport(ui, |ui, viewport| {
            let strip = ui
                .with_layout(Layout::left_to_right(Align::Min), |ui| {
                    ui.spacing_mut().item_spacing.x = CAROUSEL_GAP;
                    for (index, child) in card.carousel.iter().enumerate() {
                        let row = carousel_row(message, index, child);
                        ui.push_id(("carousel-card", index), |ui| {
                            let response = Frame::new()
                                .fill(if message.from_me {
                                    view.palette.bubble_out
                                } else {
                                    view.palette.bubble_in
                                })
                                .corner_radius(10)
                                .inner_margin(Margin {
                                    left: 10,
                                    right: 10,
                                    top: 6,
                                    bottom: 5,
                                })
                                .show(ui, |ui| {
                                    ui.set_width(card_width);
                                    ui.with_layout(Layout::top_down(Align::Min), |ui| {
                                        ui.set_width(card_width);
                                        if child.image.is_some() {
                                            carousel_picture(
                                                ui, view, message, index, child, card_width,
                                                actions,
                                            );
                                        }
                                        if !child.body.is_empty() {
                                            rich_body(
                                                ui,
                                                view,
                                                &row,
                                                &child.body,
                                                card_width,
                                                None,
                                                Some(card_width),
                                                actions,
                                            );
                                        }
                                        interactive_buttons(
                                            ui, view, &row, child, card_width, actions,
                                        );
                                    });
                                });
                            ui.ctx().data_mut(|data| {
                                data.insert_temp(
                                    bubble_id(&message.chat, &message.id)
                                        .with(("carousel-card", index)),
                                    response.response.rect,
                                )
                            });
                        });
                    }
                })
                .response
                .rect;
            let limit = (strip.width() - viewport.width()).max(0.0);
            let visible = Rect::from_min_size(
                pos2(strip.left() + viewport.left(), strip.top()),
                vec2(viewport.width(), strip.height()),
            );
            if limit > 1.0 {
                for direction in [-1, 1] {
                    let available = if direction < 0 {
                        viewport.left() > 1.0
                    } else {
                        viewport.left() < limit - 1.0
                    };
                    if !available {
                        continue;
                    }
                    let x = if direction < 0 {
                        visible.left() + 26.0
                    } else {
                        visible.right() - 26.0
                    };
                    let rect =
                        Rect::from_center_size(pos2(x, visible.center().y), Vec2::splat(40.0));
                    let arrow = carousel_arrow(
                        ui,
                        &view.palette,
                        rect,
                        id.with(("carousel-arrow", direction)),
                        direction < 0,
                    );
                    if arrow.clicked() {
                        let step = card_width + 20.0 + CAROUSEL_GAP;
                        let target = (viewport.left() + direction as f32 * step).clamp(0.0, limit);
                        ui.scroll_with_delta(vec2(viewport.left() - target, 0.0));
                    }
                }
            }
            visible
        });
    ui.ctx()
        .data_mut(|data| data.insert_temp(id.with("carousel-viewport"), output.inner));
}

pub(crate) fn carousel_row(message: &Message, index: usize, card: &crate::model::InteractiveCard) -> Message {
    Message {
        id: format!("{}-card-{index}", message.id),
        chat: message.chat.clone(),
        sender: message.sender.clone(),
        sender_name: message.sender_name.clone(),
        from_me: message.from_me,
        timestamp: message.timestamp,
        history_order: message.history_order,
        content: Content::Interactive {
            text: card.body.clone(),
            card: None,
        },
        status: message.status,
        delivered_at: None,
        read_at: None,
        quoted: None,
        reactions: Vec::new(),
        edited: message.edited,
        mentions: message.mentions.clone(),
        forwarded: false,
        thumbnail: None,
    }
}

pub(crate) fn carousel_arrow(
    ui: &mut egui::Ui,
    palette: &Palette,
    rect: Rect,
    id: egui::Id,
    previous: bool,
) -> egui::Response {
    let label = if previous {
        "Previous card"
    } else {
        "Next card"
    };
    let response = ui.interact(rect, id, Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    ui.ctx().data_mut(|data| data.insert_temp(id, rect));
    theme::reveal_focus(&response);
    if ui.is_rect_visible(rect) {
        let fill = if response.hovered() || response.has_focus() {
            palette.surface_hover
        } else {
            palette.overlay
        };
        ui.painter()
            .circle_filled(rect.center() + vec2(0.0, 2.0), 21.0, palette.shadow);
        ui.painter().circle_filled(rect.center(), 20.0, fill);
        ui.painter().circle_stroke(
            rect.center(),
            19.5,
            Stroke::new(
                1.0,
                if response.has_focus() {
                    palette.link
                } else {
                    palette.outline
                },
            ),
        );
        theme::paint_icon(
            ui,
            if previous {
                Icon::ChevronLeft
            } else {
                Icon::ChevronRight
            },
            rect,
            if response.is_pointer_button_down_on() {
                21.0
            } else {
                23.0
            },
            palette.text,
        );
    }
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(label)
}

pub(crate) fn album_content(
    ui: &mut egui::Ui,
    view: &View<'_>,
    messages: &[Message],
    width: f32,
    actions: &mut Vec<Action>,
) -> Vec<(Rect, String)> {
    let gap = 3.0;
    let columns = 2;
    let tile = ((width - gap * (columns - 1) as f32) / columns as f32).max(80.0);
    let mut tiles = Vec::with_capacity(messages.len());
    for (row, chunk) in messages.chunks(columns).enumerate() {
        // An own bubble lays out right to left; the grid keeps reading order.
        ui.allocate_ui_with_layout(
            vec2(tile * columns as f32 + gap, tile),
            Layout::left_to_right(Align::Min),
            |ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for (column, message) in chunk.iter().enumerate() {
                    if let Some(rect) =
                        album_tile(ui, view, message, vec2(tile, tile), row, column, actions)
                    {
                        tiles.push((rect, message.id.clone()));
                    }
                }
            },
        );
    }

    for message in messages {
        let caption = match &message.content {
            Content::Image { caption, .. } | Content::Video { caption, .. } => caption.as_deref(),
            _ => None,
        };
        if let Some(caption) = caption.filter(|caption| !caption.is_empty()) {
            let _ = rich_body(
                ui,
                view,
                message,
                caption,
                width,
                None,
                Some(width),
                actions,
            );
        } else if let Some(copy_rows) = view.copy_rows {
            copy_rows
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .push(transcript_row(view, message, String::new(), Vec::new()));
        }
    }

    let missing: Vec<&Message> = messages
        .iter()
        .filter(|message| {
            message
                .content
                .media()
                .is_some_and(|media| media.path.is_none())
        })
        .collect();
    let files: Vec<(PathBuf, String)> = messages
        .iter()
        .filter_map(|message| {
            let path = message.content.media()?.path.as_ref()?;
            Some((path.clone(), attachment_name(&message.content, path)))
        })
        .collect();
    if !files.is_empty()
        && theme::pill_button(
            ui,
            &view.palette,
            &crate::i18n::gettext(view.locale, "Save all…"),
            true,
        )
        .clicked()
    {
        actions.push(Action::SaveAttachments { files });
    }
    if !missing.is_empty() {
        let downloading = missing.iter().all(|message| {
            message
                .content
                .media()
                .is_some_and(|media| matches!(media.state, MediaState::Downloading))
        });
        if theme::pill_button(
            ui,
            &view.palette,
            &crate::i18n::gettext(
                view.locale,
                if downloading {
                    "Downloading…"
                } else {
                    "Download all"
                },
            ),
            !downloading,
        )
        .clicked()
        {
            for message in missing {
                if let Some(media) = message.content.media()
                    && !matches!(media.state, MediaState::Downloading)
                {
                    actions.push(Action::Download {
                        manual: true,
                        card: None,
                        chat: message.chat.clone(),
                        message: message.id.clone(),
                    });
                }
            }
        }
    }
    tiles
}

pub(crate) fn album_tile(
    ui: &mut egui::Ui,
    view: &View<'_>,
    message: &Message,
    size: Vec2,
    row: usize,
    column: usize,
    actions: &mut Vec<Action>,
) -> Option<Rect> {
    let media = message.content.media()?;
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let kind = if matches!(message.content, Content::Image { .. }) {
        "Photo"
    } else {
        "Video"
    };
    let state = match (&media.path, &media.state) {
        (Some(_), _) => "Open".to_owned(),
        (None, MediaState::Downloading) => "Downloading".to_owned(),
        (None, MediaState::Failed(error)) => format!("Failed: {error}"),
        (None, MediaState::Idle) => "Download".to_owned(),
    };
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("{kind}, {state}"))
    });
    theme::reveal_focus(&response);
    if ui.is_rect_visible(rect) {
        ui.painter().rect_filled(rect, 5.0, view.palette.surface);
        let uri = media
            .path
            .as_ref()
            .filter(|_| matches!(message.content, Content::Image { .. }))
            .map(|path| {
                let uri = crate::util::image_uri(path);
                crate::image_cache::touch(ui.ctx(), &uri);
                uri
            })
            .or_else(|| {
                message.thumbnail.as_deref().map(|bytes| {
                    thumbnail_uri(
                        ui.ctx(),
                        &message.chat,
                        &format!("{}-album-{row}-{column}", message.id),
                        bytes,
                    )
                })
            });
        if let Some(uri) = uri {
            let image = egui::Image::new(uri);
            let dimensions = match image.load_for_size(ui.ctx(), size) {
                Ok(egui::load::TexturePoll::Ready { texture }) => Some(texture.size),
                Ok(egui::load::TexturePoll::Pending { .. }) => Some(vec2(
                    media.width.unwrap_or(1) as f32,
                    media.height.unwrap_or(1) as f32,
                )),
                Err(_) => None,
            };
            if let Some(dimensions) = dimensions {
                let ratio = dimensions.x / dimensions.y.max(1.0);
                let target = size.x / size.y.max(1.0);
                let uv = if ratio > target {
                    vec2(target / ratio, 1.0)
                } else {
                    vec2(1.0, ratio / target)
                };
                image
                    .uv(Rect::from_center_size(pos2(0.5, 0.5), uv))
                    .fit_to_exact_size(size)
                    .corner_radius(5.0)
                    .paint_at(ui, rect);
            }
        }
        if matches!(message.content, Content::Video { .. }) {
            theme::paint_icon(ui, Icon::Play, rect, 30.0, Color32::WHITE);
        }
        if media.path.is_none() {
            let disc = Rect::from_center_size(rect.center(), Vec2::splat(42.0));
            ui.painter()
                .circle_filled(disc.center(), 21.0, Color32::from_black_alpha(130));
            match media.state {
                MediaState::Downloading => theme::paint_progress(
                    ui,
                    disc,
                    20.0,
                    download_fraction(view, message),
                    Color32::WHITE,
                ),
                MediaState::Failed(_) => {
                    theme::paint_icon(ui, Icon::CircleAlert, disc, 20.0, Color32::WHITE)
                }
                MediaState::Idle => {
                    theme::paint_icon(ui, Icon::Download, disc, 20.0, Color32::WHITE)
                }
            }
        }
        if not_sent(message) {
            let starred = view.starred.is_some_and(|ids| ids.contains(&message.id));
            let pinned = view.pins.is_some_and(|ids| ids.contains(&message.id));
            footer_over_picture(ui, view, message, rect, starred, pinned, actions);
        }
    }
    // A motion photo in an album keeps its own badge and clip.
    if let Content::Image {
        motion: Some(motion),
        ..
    } = &message.content
    {
        motion_clip(ui, view, message, motion, rect, actions);
    }
    theme::focus_outline(ui, response.id, rect, 5.0);
    let response = if let MediaState::Failed(error) = &media.state {
        response.on_hover_text(format!("{error} · Click to retry"))
    } else {
        response
    };
    if response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
    {
        if let Some(path) = &media.path {
            actions.push(match crate::image_preview::open_target(path, true) {
                crate::image_preview::OpenTarget::Preview => Action::PreviewImage {
                    path: path.clone(),
                    chat: message.chat.clone(),
                    message: message.id.clone(),
                    card: None,
                },
                crate::image_preview::OpenTarget::External => Action::OpenFile(path.clone()),
            });
        } else if !matches!(media.state, MediaState::Downloading) {
            actions.push(Action::Download {
                manual: true,
                card: None,
                chat: message.chat.clone(),
                message: message.id.clone(),
            });
        }
    }
    ui.ctx()
        .data_mut(|data| data.insert_temp(album_tile_id(&view.chat.id, &message.id), rect));
    Some(rect)
}

/// Where an album member's tile was drawn, for interaction tests.
pub fn album_tile_id(chat: &str, message: &str) -> egui::Id {
    bubble_id(chat, message).with("album-tile")
}

/// A carousel uses a short image preview; opening it shows the full picture.
pub(crate) fn carousel_picture(
    ui: &mut egui::Ui,
    view: &View<'_>,
    message: &Message,
    index: usize,
    card: &crate::model::InteractiveCard,
    width: f32,
    actions: &mut Vec<Action>,
) {
    let Some(media) = &card.image else { return };
    let size = vec2(width, width * 0.56);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let label = match (&media.path, &media.state) {
        (Some(_), _) => "Open card image",
        (None, MediaState::Failed(_)) => "Retry card image download",
        (None, MediaState::Downloading) => "Downloading card image",
        (None, MediaState::Idle) => "Download card image",
    };
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    // Tab can land on a card scrolled out of the carousel's viewport.
    theme::reveal_focus(&response);
    let visible = ui.is_rect_visible(rect);
    if visible {
        ui.painter().rect_filled(rect, 6.0, view.palette.surface);
        let uri = media
            .path
            .as_ref()
            .map(|path| {
                let uri = crate::util::image_uri(path);
                crate::image_cache::touch(ui.ctx(), &uri);
                uri
            })
            .or_else(|| {
                card.thumbnail.as_deref().map(|bytes| {
                    thumbnail_uri(
                        ui.ctx(),
                        &message.chat,
                        &format!("{}-card-{index}", message.id),
                        bytes,
                    )
                })
            });
        if let Some(uri) = uri {
            let image = egui::Image::new(uri);
            let dimensions = match image.load_for_size(ui.ctx(), size) {
                Ok(egui::load::TexturePoll::Ready { texture }) => Some(texture.size),
                Ok(egui::load::TexturePoll::Pending { .. }) => Some(vec2(
                    media.width.unwrap_or(16) as f32,
                    media.height.unwrap_or(9) as f32,
                )),
                Err(_) => None,
            };
            if let Some(dimensions) = dimensions {
                let ratio = dimensions.x / dimensions.y.max(1.0);
                let target = size.x / size.y.max(1.0);
                let uv_size = if ratio > target {
                    vec2(target / ratio, 1.0)
                } else {
                    vec2(1.0, ratio / target)
                };
                image
                    .uv(Rect::from_center_size(pos2(0.5, 0.5), uv_size))
                    .fit_to_exact_size(size)
                    .corner_radius(6.0)
                    .paint_at(ui, rect);
            } else if media.path.is_some() {
                theme::paint_icon(ui, Icon::CircleAlert, rect, 24.0, view.palette.danger);
                ui.painter().text(
                    rect.center() + vec2(0.0, 24.0),
                    Align2::CENTER_CENTER,
                    "Could not display this picture. Click to open it.",
                    theme::regular(11.5),
                    view.palette.secondary,
                );
            }
        }
        if media.path.is_none() {
            let disc = Rect::from_center_size(rect.center(), Vec2::splat(42.0));
            ui.painter()
                .circle_filled(disc.center(), 21.0, Color32::from_black_alpha(130));
            match &media.state {
                MediaState::Downloading => theme::paint_spinner(ui, disc, 20.0, Color32::WHITE),
                MediaState::Failed(_) => {
                    theme::paint_icon(ui, Icon::CircleAlert, disc, 20.0, Color32::WHITE);
                    ui.painter().text(
                        rect.center() + vec2(0.0, 34.0),
                        Align2::CENTER_CENTER,
                        crate::i18n::gettext(view.locale, "Download failed. Click to retry."),
                        theme::regular(11.5),
                        Color32::WHITE,
                    );
                }
                MediaState::Idle => {
                    theme::paint_icon(ui, Icon::Download, disc, 20.0, Color32::WHITE)
                }
            }
        }
        if response.has_focus() {
            ui.painter().rect_stroke(
                rect.shrink(1.0),
                6.0,
                Stroke::new(theme::FOCUS_STROKE_WIDTH, view.palette.link),
                egui::StrokeKind::Inside,
            );
        }
    }
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    let clicked = response.clicked();
    if let MediaState::Failed(error) = &media.state {
        response.on_hover_text(format!("{error} · Click to retry"));
    }
    if let Some(path) = &media.path {
        if clicked {
            actions.push(match crate::image_preview::open_target(path, true) {
                crate::image_preview::OpenTarget::Preview => Action::PreviewImage {
                    path: path.clone(),
                    chat: message.chat.clone(),
                    message: message.id.clone(),
                    card: Some(index),
                },
                crate::image_preview::OpenTarget::External => Action::OpenFile(path.clone()),
            });
        }
    } else if !matches!(media.state, MediaState::Downloading)
        && (clicked
            || (visible
                && auto_download_allowed(media, false, view.auto_download)
                && matches!(media.state, MediaState::Idle)))
    {
        actions.push(Action::Download {
            manual: clicked,
            chat: message.chat.clone(),
            message: message.id.clone(),
            card: Some(index),
        });
    }
}

pub(crate) fn interactive_enabled(
    view: &View<'_>,
    message: &Message,
    button: &crate::model::InteractiveButton,
) -> bool {
    use crate::model::InteractiveAction;
    button.url.is_some()
        || match &button.action {
            InteractiveAction::Copy(_) => true,
            InteractiveAction::Reply | InteractiveAction::Select(_) => {
                view.connected
                    && view.chat.can_send()
                    && !message.from_me
                    && !message.edited
                    && !view
                        .interactive_pending
                        .contains(&(message.chat.clone(), message.id.clone()))
            }
            InteractiveAction::Unavailable => false,
        }
}

/// Why [`interactive_enabled`] refuses a button.
pub(crate) fn interactive_reason(
    view: &View<'_>,
    message: &Message,
    button: &crate::model::InteractiveButton,
) -> ButtonBlocker {
    use crate::model::InteractiveAction;
    let sends = matches!(
        button.action,
        InteractiveAction::Reply | InteractiveAction::Select(_)
    );
    if sends
        && view
            .interactive_pending
            .contains(&(message.chat.clone(), message.id.clone()))
    {
        ButtonBlocker::Sending
    } else if sends && !view.connected {
        ButtonBlocker::Offline
    } else if sends && !view.chat.can_send() {
        ButtonBlocker::ReadOnly
    } else if sends && message.from_me {
        ButtonBlocker::OwnMessage
    } else {
        ButtonBlocker::Elsewhere
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ButtonBlocker {
    Sending,
    Offline,
    ReadOnly,
    OwnMessage,
    Elsewhere,
}

impl ButtonBlocker {
    #[cfg(test)]
    pub(crate) const ALL: [Self; 5] = [
        Self::Sending,
        Self::Offline,
        Self::ReadOnly,
        Self::OwnMessage,
        Self::Elsewhere,
    ];

    /// As the bubble's hover text shows it.
    pub(crate) fn shown(self) -> &'static str {
        match self {
            Self::Sending => "Sending reply…",
            Self::Offline => "Connect to WhatsApp to reply",
            Self::ReadOnly => "This conversation is read-only",
            Self::OwnMessage => "Reply options are for the recipient",
            Self::Elsewhere => "Open this option in WhatsApp Web or on your phone",
        }
    }

    /// As the keyboard menu speaks it, translated.
    pub(crate) fn said(self, locale: crate::i18n::Locale) -> String {
        use crate::i18n::gettext;
        match self {
            Self::Sending => gettext(locale, "Sending reply…"),
            Self::Offline => gettext(locale, "Connect to WhatsApp to reply"),
            Self::ReadOnly => gettext(locale, "This conversation is read-only"),
            Self::OwnMessage => gettext(locale, "Reply options are for the recipient"),
            Self::Elsewhere => gettext(locale, "Open this option in WhatsApp Web or on your phone"),
        }
        .into_owned()
    }
}

pub(crate) fn interactive_action(
    message: &Message,
    index: usize,
    button: &crate::model::InteractiveButton,
) -> Option<Action> {
    use crate::model::InteractiveAction;
    if let Some(url) = &button.url {
        return Some(Action::OpenUrl(url.clone()));
    }
    match &button.action {
        InteractiveAction::Reply => Some(Action::ReplyInteractive {
            chat: message.chat.clone(),
            message: message.id.clone(),
            button: index,
            choice: None,
        }),
        InteractiveAction::Copy(code) => Some(Action::CopyText(code.clone())),
        InteractiveAction::Select(_) => {
            Some(Action::ShowDialog(crate::model::Dialog::InteractiveList {
                chat: message.chat.clone(),
                message: message.id.clone(),
                button: index,
            }))
        }
        InteractiveAction::Unavailable => None,
    }
}

pub(crate) fn interactive_buttons(
    ui: &mut egui::Ui,
    view: &View<'_>,
    message: &Message,
    card: &crate::model::InteractiveCard,
    width: f32,
    actions: &mut Vec<Action>,
) {
    use crate::model::InteractiveAction;
    let palette = &view.palette;
    let spacing = ui.spacing().item_spacing.y;
    ui.spacing_mut().item_spacing.y = 0.0;
    for (index, button) in card.buttons.iter().enumerate() {
        let enabled = interactive_enabled(view, message, button);
        let color = if message.from_me {
            palette.text
        } else if enabled {
            palette.link
        } else {
            palette.secondary
        };
        let icon = if button.url.is_some() {
            Some(Icon::ExternalLink)
        } else {
            match button.action {
                InteractiveAction::Reply => None,
                InteractiveAction::Select(_) => Some(Icon::ListChecks),
                InteractiveAction::Copy(_) => Some(Icon::Copy),
                InteractiveAction::Unavailable => Some(Icon::Smartphone),
            }
        };
        let line = widgets::line(
            ui,
            &button.label,
            theme::medium(14.0),
            color,
            (width - 48.0).max(1.0),
            2,
        );
        let sense = if enabled {
            Sense::click()
        } else {
            Sense::hover()
        };
        let (rect, _) = ui.allocate_exact_size(
            vec2(width, (line.size().y + 22.0).max(44.0)),
            Sense::hover(),
        );
        let last = index + 1 == card.buttons.len() && !card.needs_phone;
        let row_rect = Rect::from_min_max(
            rect.min - vec2(10.0, 0.0),
            rect.max + vec2(10.0, if last { 5.0 } else { 0.0 }),
        );
        let corners = CornerRadius {
            sw: if last { 10 } else { 0 },
            se: if last { 10 } else { 0 },
            ..CornerRadius::ZERO
        };
        let response = ui.interact(
            row_rect,
            bubble_id(&message.chat, &message.id).with(("interactive-action", index)),
            sense,
        );
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, &button.label)
        });
        theme::reveal_focus(&response);
        ui.ctx().data_mut(|data| {
            data.insert_temp(
                bubble_id(&message.chat, &message.id).with(("interactive-button", index)),
                row_rect,
            )
        });
        if ui.is_rect_visible(rect) {
            if enabled && (response.hovered() || response.has_focus()) {
                ui.painter().rect_filled(
                    row_rect,
                    corners,
                    palette
                        .text
                        .gamma_multiply(if response.is_pointer_button_down_on() {
                            0.08
                        } else {
                            0.04
                        }),
                );
            }
            ui.painter().hline(
                row_rect.x_range(),
                rect.top(),
                Stroke::new(1.0, palette.secondary.gamma_multiply(0.2)),
            );
            if response.has_focus() {
                ui.painter().rect_stroke(
                    rect.shrink(2.0),
                    4.0,
                    Stroke::new(1.0, palette.link),
                    egui::StrokeKind::Inside,
                );
            }
            let inset = if icon.is_some() { 22.0 } else { 0.0 };
            let content_width = line.size().x + inset;
            let left = rect.center().x - content_width / 2.0;
            if let Some(icon) = icon {
                theme::paint_icon(
                    ui,
                    icon,
                    Rect::from_center_size(
                        egui::pos2(left + 7.0, rect.center().y),
                        Vec2::splat(14.0),
                    ),
                    14.0,
                    color,
                );
            }
            line.paint(
                ui,
                egui::pos2(left + inset, rect.center().y - line.size().y / 2.0),
                color,
            );
        }
        if !enabled {
            let reason = interactive_reason(view, message, button).shown();
            response.on_hover_text(format!("{}\n{reason}", button.label));
        } else if let Some(url) = &button.url {
            if response
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text(format!("{}\n{url}", button.label))
                .clicked()
                && let Some(action) = interactive_action(message, index, button)
            {
                actions.push(action);
            }
        } else {
            let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
            let response = match &button.action {
                InteractiveAction::Reply => response.on_hover_text(
                    crate::i18n::gettext(view.locale, "Send reply: {label}")
                        .replace("{label}", &button.label),
                ),
                InteractiveAction::Copy(_) => {
                    response.on_hover_text(crate::i18n::gettext(view.locale, "Copy code"))
                }
                InteractiveAction::Select(_) | InteractiveAction::Unavailable => response,
            };
            if response.clicked()
                && let Some(action) = interactive_action(message, index, button)
            {
                actions.push(action);
            }
        }
    }
    if card.needs_phone {
        ui.add_space(6.0);
        widgets::rich_text(
            ui,
            "More content in WhatsApp Web or on your phone",
            theme::regular(12.0),
            palette.secondary,
        );
    }
    ui.spacing_mut().item_spacing.y = spacing;
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn rich_body(
    ui: &mut egui::Ui,
    view: &View<'_>,
    message: &Message,
    text: &str,
    width: f32,
    reserve: Option<f32>,
    span: Option<f32>,
    actions: &mut Vec<Action>,
) -> Option<Rect> {
    let palette = view.palette;
    let mentions = mentions_of(view, message);
    let style = markup::Style {
        size: BODY_SIZE,
        color: palette.text,
        secondary: palette.secondary,
        link: palette.link,
        mention: palette.accent,
    };
    let laid = markup::layout(ui, text, &mentions, &style, width);
    let rtl = crate::bidi::message_rtl(text);
    let last_row = laid.galley.rows.last().map_or(0.0, |row| row.row.size.x);
    let single = laid.galley.rows.len() == 1 && span.is_none();
    let inline = reserve.filter(|reserve| last_row + 8.0 + reserve <= width && (!rtl || single));
    let size = laid.galley.size();
    let mut allocation = match inline {
        Some(reserve) => vec2(size.x.max(last_row + 8.0 + reserve), size.y),
        None => size,
    };
    if let Some(span) = span {
        // Span the card width and keep the text left-aligned in own bubbles.
        allocation.x = allocation.x.max(span);
    }
    // Register the body for transcript formatting when copying across messages.
    if let Some(copy_rows) = view.copy_rows {
        copy_rows
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(transcript_row(
                view,
                message,
                laid.galley.text().to_owned(),
                laid.placements().to_vec(),
            ));
    }
    let (_, rect) = ui.allocate_space(allocation);
    let id = bubble_id(&view.chat.id, &message.id);
    let selection_rect = ui
        .ctx()
        .data(|data| data.get_temp::<Rect>(id.with("rect")))
        .map(|bubble| {
            let bubble = ui
                .ctx()
                .data(|data| data.get_temp::<Rect>(id.with("body")))
                .map(|previous| {
                    let mut bubble = bubble.translate(rect.min - previous.min);
                    bubble.max += rect.size() - previous.size();
                    bubble
                })
                .unwrap_or(bubble);
            if matches!(message.content, Content::Text { .. }) {
                let mut target = bubble.union(rect);
                for kind in ["quote", "preview"] {
                    let present = if kind == "quote" {
                        message.quoted.is_some()
                    } else {
                        matches!(
                            message.content,
                            Content::Text {
                                preview: Some(_),
                                ..
                            }
                        )
                    };
                    if !present {
                        continue;
                    }
                    if let Some(card) = ui.ctx().data(|data| data.get_temp::<Rect>(id.with(kind))) {
                        target.min.y = target.min.y.max(card.bottom());
                    }
                }
                if let Some(name) = ui
                    .ctx()
                    .data(|data| data.get_temp::<Rect>(id.with("sender")))
                {
                    target.min.y = target.min.y.max(name.bottom());
                }
                target
            } else {
                rect.expand2(vec2(10.0, 6.0)).intersect(bubble).union(rect)
            }
        })
        .unwrap_or(rect);
    let response = ui.interact(
        selection_rect,
        id.with("body-text"),
        Sense::CLICK | Sense::DRAG,
    );
    let padding = response
        .interact_pointer_pos()
        .is_some_and(|pointer| !rect.contains(pointer));
    if padding {
        reply_on_double_click(&response, message, actions);
        if response.clicked() && !response.double_clicked() {
            let modifiers = ui.input(|input| input.modifiers);
            if view.selecting {
                actions.push(if modifiers.shift {
                    Action::SelectRange(message.id.clone())
                } else {
                    Action::ToggleSelected(message.id.clone())
                });
            } else if modifiers.command {
                actions.push(Action::SelectMessage(message.id.clone()));
            }
        }
    }
    // Store the body rect for selection tests.
    ui.ctx().data_mut(|data| {
        data.insert_temp(id.with("body"), rect);
    });
    let visible = ui.is_rect_visible(rect);
    let selection_alive = ui.input(|input| input.pointer.primary_down())
        || ui
            .ctx()
            .plugin_opt::<egui::text_selection::LabelSelectionState>()
            .is_some_and(|plugin| plugin.lock().has_selection());
    let origin = if rtl && inline.is_none() {
        pos2(rect.right() - size.x, rect.top())
    } else {
        rect.min
    };
    if visible || selection_alive {
        markup::paint_selectable(ui, &laid, &response, origin, palette.text, visible);
    }
    let message_number = crate::i18n::gettext(view.locale, "Message");
    let copy_number = crate::i18n::gettext(view.locale, "Copy number");
    let menu_labels = [message_number.as_ref(), copy_number.as_ref()];
    for (range, url) in &laid.links {
        let Some(phone) = url.strip_prefix("tel:") else {
            continue;
        };
        let number = phone.strip_prefix('+').unwrap_or(phone);
        let copy_text: String = laid
            .galley
            .text()
            .chars()
            .skip(range.start)
            .take(range.len())
            .collect();
        for (row, bounds) in crate::bidi::char_bounds_by_row(&laid.galley, range.start, range.end) {
            let link = ui
                .interact(
                    bounds.translate(origin.to_vec2()).expand(2.0),
                    bubble_id(&view.chat.id, &message.id).with((
                        "phone-link",
                        range.start,
                        range.end,
                        row,
                    )),
                    Sense::click(),
                )
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            link.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), number)
            });
            theme::reveal_focus(&link);
            let link = focus::phone_link(link);
            egui::Popup::menu(&link)
                .id(link.id.with("popup"))
                .width(widgets::menu_width(ui, &menu_labels, true))
                .frame(widgets::menu_frame(&view.palette))
                .close_behavior(egui::PopupCloseBehavior::CloseOnClick)
                .show(|ui| {
                    #[cfg(test)]
                    let message_rect =
                        Rect::from_min_size(ui.cursor().min, vec2(ui.available_width(), 28.0));
                    let message_clicked =
                        widgets::menu_item(ui, &view.palette, None, message_number.as_ref());
                    #[cfg(test)]
                    ui.ctx().data_mut(|data| {
                        data.insert_temp(link.id.with("test-message-action"), message_rect);
                    });
                    if message_clicked {
                        actions.push(Action::NewContact {
                            phone: number.to_owned(),
                            first: String::new(),
                            last: String::new(),
                            to_phone: None,
                        });
                    }
                    #[cfg(test)]
                    let copy_rect =
                        Rect::from_min_size(ui.cursor().min, vec2(ui.available_width(), 28.0));
                    let copy_clicked = widgets::menu_item(
                        ui,
                        &view.palette,
                        Some(Icon::Copy),
                        copy_number.as_ref(),
                    );
                    #[cfg(test)]
                    ui.ctx().data_mut(|data| {
                        data.insert_temp(link.id.with("test-copy-action"), copy_rect);
                    });
                    if copy_clicked {
                        actions.push(Action::CopyText(copy_text.clone()));
                    }
                });
        }
    }
    if !laid.links.is_empty()
        && let Some(pos) = response.hover_pos()
        && laid
            .galley
            .rows
            .iter()
            .any(|row| Rect::from_min_size(origin + row.pos.to_vec2(), row.row.size).contains(pos))
    {
        let cursor = laid.galley.cursor_from_pos(pos - origin);
        if let Some(url) = laid.link_at(cursor.index.0) {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            if !url.starts_with("tel:") && response.clicked() {
                actions.push(Action::OpenUrl(url.to_owned()));
            }
        }
    }
    inline.map(|reserve| {
        Rect::from_min_max(
            pos2(rect.right() - reserve, rect.bottom() - 15.0),
            rect.right_bottom(),
        )
    })
}

/// Link preview with image, title, and description.
pub(crate) fn preview_card(
    ui: &mut egui::Ui,
    view: &View<'_>,
    message: &Message,
    preview: &LinkPreview,
    width: f32,
    actions: &mut Vec<Action>,
) {
    let palette = view.palette;
    let thumbnail = message
        .thumbnail
        .as_deref()
        .map(|bytes| (message.chat.as_str(), message.id.as_str(), bytes));
    let response = Frame::new()
        .fill(palette.window.gamma_multiply(0.35))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(Margin::same(8))
        .show(ui, |ui| {
            // Include margins in the settled card width.
            let card_width = (width - 16.0).max(0.0);
            ui.set_width(card_width);
            preview_layout(ui, &palette, preview, thumbnail, card_width);
        })
        .response;
    ui.ctx().data_mut(|data| {
        data.insert_temp(
            bubble_id(&view.chat.id, &message.id).with("preview"),
            response.rect,
        );
    });
    let response = ui
        .interact(
            response.rect,
            ui.id().with(("preview", &message.id)),
            Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    if response.clicked() {
        actions.push(Action::OpenUrl(preview.url.clone()));
    }
}

pub(crate) fn preview_layout(
    ui: &mut egui::Ui,
    palette: &Palette,
    preview: &LinkPreview,
    thumbnail: Option<(&str, &str, &[u8])>,
    card_width: f32,
) -> Rect {
    let domain = preview
        .url
        .split("://")
        .nth(1)
        .unwrap_or(&preview.url)
        .split('/')
        .next()
        .unwrap_or_default()
        .to_owned();
    let column = (card_width - if thumbnail.is_some() { 72.0 } else { 0.0 }).max(0.0);
    ui.allocate_ui_with_layout(vec2(card_width, 0.0), Layout::top_down(Align::Min), |ui| {
        ui.set_width(card_width);
        ui.horizontal(|ui| {
            if let Some((chat, id, bytes)) = thumbnail {
                let (rect, _) = ui.allocate_exact_size(Vec2::splat(64.0), Sense::hover());
                if ui.is_rect_visible(rect) {
                    let uri = thumbnail_uri(ui.ctx(), chat, id, bytes);
                    egui::Image::new(uri)
                        .fit_to_exact_size(rect.size())
                        .corner_radius(4.0)
                        .paint_at(ui, rect);
                }
            }
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.set_width(column);
                if let Some(title) = &preview.title {
                    widgets::rich_text(ui, title, theme::semibold(13.5), palette.text);
                }
                if let Some(description) = &preview.description {
                    let line = widgets::line(
                        ui,
                        description,
                        theme::regular(12.5),
                        palette.secondary,
                        ui.available_width(),
                        2,
                    );
                    let (rect, _) = ui.allocate_exact_size(line.size(), Sense::hover());
                    line.paint(ui, rect.min, palette.secondary);
                }
                theme::text(ui, &domain, theme::regular(12.0), palette.dim);
            });
        });
    })
    .response
    .rect
}

#[expect(clippy::too_many_arguments)]
pub(crate) fn location_card(
    ui: &mut egui::Ui,
    view: &View<'_>,
    message: &Message,
    preview_key: &str,
    title: &str,
    detail: Option<String>,
    note: Option<String>,
    (latitude, longitude): (f64, f64),
    actions: &mut Vec<Action>,
) {
    let palette = view.palette;
    let card = ui.available_width().min(LOCATION_CARD_WIDTH);
    ui.allocate_ui_with_layout(vec2(card, 0.0), Layout::top_down(Align::Min), |ui| {
        ui.set_width(card);
        ui.spacing_mut().item_spacing.y = 2.0;
        if let Some(bytes) = message.thumbnail.as_deref()
            && !bytes.is_empty()
        {
            let uri = thumbnail_uri(ui.ctx(), &message.chat, preview_key, bytes);
            let height = (card * 0.56).round();
            let crop = (1.0 - height / card) / 2.0;
            ui.add(
                egui::Image::new(uri)
                    .maintain_aspect_ratio(false)
                    .uv(Rect::from_min_max(pos2(0.0, crop), pos2(1.0, 1.0 - crop)))
                    .fit_to_exact_size(Vec2::new(card, height))
                    .corner_radius(8.0),
            );
            ui.add_space(4.0);
        }
        ui.horizontal(|ui| {
            theme::icon(ui, Icon::MapPin, 16.0, palette.accent);
            widgets::rich_text(ui, title, theme::medium(14.0), palette.text);
        });
        if let Some(detail) = detail {
            widgets::rich_text(ui, &detail, theme::regular(12.5), palette.secondary);
        }
        if let Some(note) = note {
            widgets::rich_text(ui, &note, theme::regular(12.5), palette.secondary);
        }
        if theme::link(
            ui,
            crate::i18n::gettext(view.locale, "Open in a map").as_ref(),
            theme::regular(12.5),
            palette.link,
        )
        .clicked()
        {
            let spot = Spot {
                latitude,
                longitude,
            };
            actions.push(Action::OpenUrl(crate::geo::map_url(spot)));
        }
    });
}
