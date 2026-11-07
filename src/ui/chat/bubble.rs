//! Message bubble frame, quoted blocks, delivery footers, and reactions bar.

use super::*;

/// Draws a message row and returns its bubble response for scrolling.
pub(crate) fn bubble(
    ui: &mut egui::Ui,
    view: &View<'_>,
    message: &Message,
    album: Option<&[Message]>,
    place: RunPlace,
    actions: &mut Vec<Action>,
) -> Option<egui::Response> {
    if let Content::DisappearingTimer { seconds } = &message.content {
        return Some(timer_notice(ui, view, message, *seconds));
    }
    let RunPlace {
        show_sender,
        show_avatar,
        tailed,
    } = place;
    let own = message.from_me;
    // Greys that read on the panel can vanish on the bubble; use its own.
    let view = &View {
        palette: view.palette.on_bubble(own),
        ..*view
    };
    let with_avatar = !own && shows_sender_pictures(view.chat);
    let carousel = matches!(&message.content, Content::Interactive { card: Some(card), .. } if !card.carousel.is_empty());
    let share = if carousel {
        0.95
    } else if view.palette.bubbles == theme::BubbleStyle::Messages {
        MESSAGES_WIDTH_SHARE
    } else {
        WHATSAPP_WIDTH_SHARE
    };
    let max_width = ((ui.available_width() * share).min(if carousel { 920.0 } else { 560.0 })
        - if with_avatar {
            SENDER_AVATAR + 8.0
        } else {
            0.0
        })
    .max(0.0);
    let id = bubble_id(&view.chat.id, &message.id);
    let previous = ui.ctx().data(|data| data.get_temp::<Rect>(id.with("rect")));
    if let Some(rect) = previous {
        let strip = Rect::from_x_y_ranges(ui.max_rect().x_range(), rect.y_range());
        let strip = ui.interact(strip, id.with("row"), Sense::click_and_drag());
        if strip.clicked() {
            actions.push(Action::FocusComposer);
        }
        if strip.drag_started() {
            actions.push(Action::SweepMessages {
                anchor: message.id.clone(),
                to: message.id.clone(),
            });
        }
        reply_on_double_click(&strip, message, actions);
    }
    let mut response = None;
    const ENTRANCE_DURATION: f32 = 0.28;
    let progress = if view.animate
        && let Some(&arrival) = view.arrivals.get(&message.id).or_else(|| {
            album.and_then(|members| members.iter().find_map(|m| view.arrivals.get(&m.id)))
        }) {
        let elapsed = arrival.elapsed().as_secs_f32();
        if elapsed < ENTRANCE_DURATION {
            ui.ctx().request_repaint();
            let t = (elapsed / ENTRANCE_DURATION).clamp(0.0, 1.0);
            egui::emath::easing::cubic_out(t)
        } else {
            1.0
        }
    } else {
        1.0
    };
    let render_bubble = |ui: &mut egui::Ui, response: &mut Option<egui::Response>, actions: &mut Vec<Action>| {
        if with_avatar {
            ui.horizontal_top(|ui| {
                let (space, _) =
                    ui.allocate_exact_size(Vec2::splat(SENDER_AVATAR), Sense::hover());
                // Keep bubble content vertically laid out inside the row.
                ui.vertical(|ui| {
                    *response = Some(bubble_frame(
                        ui,
                        view,
                        message,
                        album,
                        show_sender,
                        tailed,
                        max_width,
                        actions,
                    ));
                });
                if !show_avatar {
                    return;
                }
                let rect = match response.as_ref() {
                    Some(bubble) if view.palette.bubbles == theme::BubbleStyle::Messages => {
                        Rect::from_min_size(
                            pos2(
                                space.left(),
                                (bubble.rect.bottom() - SENDER_AVATAR).max(space.top()),
                            ),
                            Vec2::splat(SENDER_AVATAR),
                        )
                    }
                    _ => space,
                };
                let avatar = ui.interact(rect, id.with("avatar"), Sense::CLICK);
                theme::focus_outline(ui, avatar.id, rect, SENDER_AVATAR / 2.0);
                if avatar
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .clicked()
                {
                    actions.push(sender_info_action(message));
                }
                if ui.is_rect_visible(rect) {
                    let name = (view.names_or)(&message.sender, message.sender_name.as_deref());
                    widgets::paint_avatar(
                        ui,
                        &view.palette,
                        rect,
                        name.trim_start_matches('~'),
                        &message.sender,
                        view.avatars
                            .get(&message.sender)
                            .and_then(|picture| picture.as_deref()),
                    );
                }
            });
        } else {
            *response = Some(bubble_frame(
                ui,
                view,
                message,
                album,
                show_sender,
                tailed,
                max_width,
                actions,
            ));
        }
        let members = album.unwrap_or(std::slice::from_ref(message));
        if members.iter().any(|member| !member.reactions.is_empty()) {
            ui.add_space(-7.0);
            ui.horizontal(|ui| {
                if with_avatar {
                    ui.add_space(SENDER_AVATAR + 8.0);
                }
                for member in members.iter().filter(|member| !member.reactions.is_empty()) {
                    reactions(ui, view, member, actions);
                }
            });
            ui.add_space(2.0);
        }
    };
    let layout = Layout::top_down(if own { Align::Max } else { Align::Min });
    if progress < 1.0 {
        let dy = (1.0 - progress) * 16.0;
        let transform = egui::emath::TSTransform::from_translation(vec2(0.0, dy));
        let opacity = (progress * 1.6).min(1.0);
        ui.with_visual_transform(transform, |ui| {
            ui.multiply_opacity(opacity);
            ui.with_layout(layout, |ui| {
                render_bubble(ui, &mut response, actions);
            });
        });
    } else {
        ui.with_layout(layout, |ui| {
            render_bubble(ui, &mut response, actions);
        });
    }
    response
}


pub(crate) fn speed_pill(
    ui: &mut egui::Ui,
    view: &View<'_>,
    size: Vec2,
    speed: f32,
    active: bool,
    preparing: bool,
) -> egui::Response {
    let palette = view.palette;
    let label = crate::audio::speed_label(speed);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::Button,
            ui.is_enabled(),
            active,
            format!("Playback speed {label}"),
        )
    });
    theme::reveal_focus(&response);
    theme::focus_outline(ui, response.id, rect, rect.height() / 2.0);
    if ui.is_rect_visible(rect) {
        let hovered = response.hovered();
        let fill = if active {
            palette
                .accent
                .gamma_multiply(if hovered { 0.42 } else { 0.30 })
        } else if hovered {
            palette.surface_active
        } else {
            palette.surface_hover
        };
        ui.painter().rect_filled(rect, rect.height() / 2.0, fill);
        let colour = if active {
            palette.accent
        } else {
            palette.secondary
        };
        let colour = if preparing {
            colour.gamma_multiply(0.5)
        } else {
            colour
        };
        let galley = ui
            .painter()
            .layout_no_wrap(label, theme::medium(11.0), colour);
        ui.painter()
            .galley(rect.center() - galley.size() / 2.0, galley, colour);
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

pub(crate) fn speed_action(chat: &Chat, speed: f32) -> Action {
    if chat.voice_speed.is_some() {
        Action::SetChatVoiceSpeed {
            chat: chat.id.clone(),
            speed: Some(speed),
        }
    } else {
        Action::SetVoiceSpeed(speed)
    }
}

pub(crate) fn speed_menu_row(
    ui: &mut egui::Ui,
    view: &View<'_>,
    message: &Message,
    actions: &mut Vec<Action>,
) {
    let speed = view.voice_speed;
    let preparing = view.player.preparing_speed(&message.id);
    widgets::menu_separator(ui, &view.palette);
    ui.allocate_ui_with_layout(
        vec2(ui.available_width(), 28.0),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.add_space(6.0);
            for option in crate::audio::SPEEDS {
                let selected = option == speed;
                let response = speed_pill(
                    ui,
                    view,
                    vec2(44.0, 24.0),
                    option,
                    selected,
                    preparing && selected,
                );
                ui.ctx().data_mut(|data| {
                    data.insert_temp(
                        speed_button_id(&view.chat.id, &message.id, option),
                        response.rect,
                    );
                });
                if response.clicked() {
                    actions.push(speed_action(view.chat, option));
                    ui.close();
                }
            }
        },
    );
}

pub(crate) fn bubbleless(palette: &Palette, message: &Message) -> bool {
    matches!(
        &message.content,
        Content::Sticker { .. } | Content::Video { note: true, .. }
    ) || matches!(&message.content, Content::Interactive { card: Some(card), .. } if !card.carousel.is_empty())
        || bare_picture(palette, message)
}

pub(crate) fn last_of_run(messages: &[Message], index: usize) -> bool {
    let message = &messages[index];
    messages.get(index + 1).is_none_or(|next| {
        !same_run(message, next)
            || crate::util::day_key(message.timestamp) != crate::util::day_key(next.timestamp)
    })
}

pub(crate) fn last_bubble_of_run(palette: &Palette, messages: &[Message], index: usize) -> bool {
    let message = &messages[index];
    !messages[index + 1..]
        .iter()
        .take_while(|next| {
            same_run(message, next)
                && crate::util::day_key(message.timestamp) == crate::util::day_key(next.timestamp)
        })
        .any(|next| !bubbleless(palette, next))
}

pub(crate) fn same_run(message: &Message, next: &Message) -> bool {
    !matches!(message.content, Content::DisappearingTimer { .. })
        && !matches!(next.content, Content::DisappearingTimer { .. })
        && message.from_me == next.from_me
        && (message.from_me || message.sender == next.sender)
}

pub(crate) fn bubble_margin(palette: &Palette, message: &Message, over_picture: bool) -> Margin {
    if bare_picture(palette, message) {
        Margin::ZERO
    } else if palette.bubbles == theme::BubbleStyle::Messages {
        MESSAGES_BUBBLE_MARGIN
    } else {
        Margin {
            left: 10,
            right: 10,
            top: 6,
            bottom: if over_picture { 6 } else { 5 },
        }
    }
}

/// Draws a message bubble and its menu.
#[allow(clippy::too_many_arguments)]
pub(crate) fn bubble_frame(
    ui: &mut egui::Ui,
    view: &View<'_>,
    message: &Message,
    album: Option<&[Message]>,
    show_sender: bool,
    tailed: bool,
    max_width: f32,
    actions: &mut Vec<Action>,
) -> egui::Response {
    let palette = view.palette;
    let own = message.from_me;
    let fill = if bubbleless(&palette, message) {
        Color32::TRANSPARENT
    } else if own {
        palette.bubble_out
    } else {
        palette.bubble_in
    };
    let bubble_id = bubble_id(&view.chat.id, &message.id);
    let rect_id = bubble_id.with("rect");
    let previous = ui.ctx().data(|data| data.get_temp::<Rect>(rect_id));
    let early = previous.map(|rect| ui.interact(rect, bubble_id, Sense::CLICK));
    // Painted once the contents are measured, beneath them.
    let backdrop = ui.painter().add(egui::Shape::Noop);
    let tail = (tailed && fill != Color32::TRANSPARENT).then_some(if own {
        widgets::Side::Right
    } else {
        widgets::Side::Left
    });
    let over_picture = album.is_none() && time_over_picture(message);
    let mut tiles: Vec<(Rect, String)> = Vec::new();
    let inner = Frame::new()
        .inner_margin(bubble_margin(&palette, message, over_picture))
        .show(ui, |ui| {
            ui.set_max_width(max_width);
            ui.spacing_mut().item_spacing.y = 4.0;
            if show_sender && view.chat.is_group() {
                let name = (view.names_or)(&message.sender, message.sender_name.as_deref());
                let response = widgets::rich_text(
                    ui,
                    &name,
                    theme::semibold(13.0),
                    palette.sender(crate::util::hue(&message.sender)),
                );
                let response = ui
                    .interact(
                        response.rect,
                        ui.id().with(("sender", &message.id)),
                        Sense::CLICK,
                    )
                    .on_hover_cursor(egui::CursorIcon::PointingHand);
                ui.ctx()
                    .data_mut(|data| data.insert_temp(bubble_id.with("sender"), response.rect));
                if response.clicked() {
                    actions.push(sender_info_action(message));
                }
            } else {
                ui.ctx()
                    .data_mut(|data| data.remove::<Rect>(bubble_id.with("sender")));
            }
            let forwarded = message.forwarded.then(|| forwarded_label(ui, &palette));
            let inset = if palette.bubbles == theme::BubbleStyle::Messages
                && !bare_picture(&palette, message)
            {
                MESSAGES_BUBBLE_MARGIN.sum().x
            } else {
                20.0
            };
            let cap = ((max_width - inset).min(ui.available_width())).max(0.0);
            let last = album
                .and_then(|messages| messages.last())
                .unwrap_or(message);
            // An album's footer is its last member's, marks included.
            let starred = view.starred.is_some_and(|ids| ids.contains(&last.id));
            let pinned = view.pins.is_some_and(|ids| ids.contains(&last.id));
            let reserve = footer_width(ui, last, starred, pinned);
            let settled = album
                .map(|_| CARD_WIDTH.min(cap))
                .or_else(|| settled_width(ui, view, message, cap));
            let slot = if let Some(messages) = album {
                let mut quoted_targets = HashMap::new();
                let mut quotes: Vec<(&Message, &crate::model::Quoted)> = Vec::new();
                for member in messages {
                    if let Some(quoted) = &member.quoted {
                        let target = (
                            quoted.chat.as_deref().unwrap_or(&view.chat.id),
                            quoted.id.as_str(),
                            quoted.sender.as_str(),
                        );
                        let index = *quoted_targets.entry(target).or_insert_with(|| {
                            quotes.push((member, quoted));
                            quotes.len() - 1
                        });
                        if quotes[index].1.summary.trim().is_empty()
                            && !quoted.summary.trim().is_empty()
                        {
                            quotes[index] = (member, quoted);
                        }
                    }
                }
                for (member, quoted) in quotes {
                    quote_block(ui, view, member, quoted, settled.unwrap_or(cap), actions);
                }
                tiles = album_content(ui, view, messages, settled.unwrap_or(cap), actions);
                None
            } else {
                match settled {
                    Some(width) => {
                        if let Some(quoted) = &message.quoted {
                            quote_block(ui, view, message, quoted, width, actions);
                        }
                        content(ui, view, message, width, reserve, actions)
                    }
                    None => content(ui, view, message, cap, reserve, actions),
                }
            };
            if over_picture && let Some(picture) = slot {
                footer_over_picture(ui, view, message, picture, starred, pinned, actions);
            } else {
                footer(ui, view, last, slot, starred, pinned, actions);
            }
            if matches!(message.content, Content::Poll { .. }) {
                super::polls::results_button(
                    ui,
                    &palette,
                    view.locale,
                    message,
                    settled.unwrap_or(cap),
                    actions,
                );
            }
            if let Content::Interactive {
                card: Some(card), ..
            } = &message.content
            {
                interactive_buttons(ui, view, message, card, settled.unwrap_or(cap), actions);
            }
            if let Some((reserved, galley)) = forwarded {
                let at = pos2(ui.min_rect().left(), reserved.top());
                let painted = paint_forwarded_label(ui, &palette, at, galley);
                ui.ctx().data_mut(|data| {
                    data.insert_temp(bubble_id.with("forwarded"), painted);
                });
            }
        });
    if fill != Color32::TRANSPARENT && ui.is_rect_visible(inner.response.rect) {
        ui.painter().set(
            backdrop,
            widgets::bubble_shape(
                &palette,
                inner.response.rect,
                fill,
                tail,
                ui.ctx().pixels_per_point(),
                // The transcript's viewport: own balloons shade down it.
                ui.clip_rect().y_range(),
            ),
        );
    }
    ui.ctx()
        .data_mut(|data| data.insert_temp(rect_id, inner.response.rect));
    let bubble = early.unwrap_or_else(|| ui.interact(inner.response.rect, bubble_id, Sense::CLICK));
    if view.selecting {
        return bubble;
    }
    theme::reveal_focus(&bubble);
    if ui.ctx().data(|data| {
        data.get_temp::<bool>(theme::keyboard_focus_id())
            .unwrap_or(false)
    }) && let Some(focused) = ui
        .memory(|memory| memory.focused())
        .and_then(|id| ui.ctx().read_response(id))
        && (focused.id == bubble.id || inner.response.rect.contains(focused.rect.center()))
    {
        view.keyboard_navigation.set(true);
        if focused.gained_focus() && !ui.clip_rect().contains_rect(focused.rect) {
            ui.scroll_to_rect_animation(
                inner.response.rect.expand(4.0),
                None,
                egui::style::ScrollAnimation::none(),
            );
        }
    }
    let reacting = view.reaction_menu && view.reaction == Some(message.id.as_str());
    reply_on_double_click(&bubble, message, actions);

    let viewport = ui.clip_rect();
    let shown =
        Rect::from_x_y_ranges(viewport.x_range(), bubble.rect.y_range()).intersect(viewport);
    let right_clicked = ui.input(|input| {
        input.pointer.secondary_clicked()
            && input
                .pointer
                .interact_pos()
                .is_some_and(|pos| shown.contains(pos))
    }) && ui
        .input(|input| input.pointer.interact_pos())
        .is_some_and(|pos| {
            ui.ctx()
                .layer_id_at(pos)
                .is_none_or(|layer| layer == bubble.layer_id)
        });
    let target_id = bubble_id.with("album-target");
    if album.is_some() && right_clicked {
        let pointer = ui.input(|input| input.pointer.interact_pos());
        let hit = tiles
            .iter()
            .find(|(rect, _)| pointer.is_some_and(|pos| rect.contains(pos)))
            .map(|(_, id)| id.clone());
        ui.ctx().data_mut(|data| {
            if let Some(id) = hit {
                data.insert_temp(target_id, id);
            } else {
                data.remove::<String>(target_id);
            }
        });
    }
    if let Some(members) = album
        && let Some(opened) = view.open_menu
        && members.iter().any(|member| member.id == opened)
    {
        ui.ctx()
            .data_mut(|data| data.insert_temp(target_id, opened.to_owned()));
    }
    let menu_target = album
        .and_then(|members| {
            let id = ui.ctx().data(|data| data.get_temp::<String>(target_id))?;
            members.iter().find(|member| member.id == id)
        })
        .unwrap_or(message);
    let force_menu = match album {
        Some(members) => view
            .open_menu
            .is_some_and(|opened| members.iter().any(|member| member.id == opened)),
        None => view.open_menu == Some(message.id.as_str()),
    };
    let reacting = reacting
        || (album.is_some()
            && view.reaction_menu
            && view.reaction == Some(menu_target.id.as_str()));
    let width = menu_width(ui, view.locale, view.reaction_emoji, menu_target);
    let keyboard_clicked =
        bubble.clicked() && bubble.has_focus() && !ui.input(|input| input.pointer.any_click());
    let bubble_rect = inner.response.rect;
    let pointer = ui.input(|input| input.pointer.interact_pos());
    let hover_rect = if tail == Some(widgets::Side::Right) {
        bubble_rect.with_max_x(bubble_rect.right() + widgets::TAIL_WIDTH)
    } else {
        bubble_rect
    };
    let bubble_hovered = pointer.is_some_and(|pos| hover_rect.contains(pos));

    let popup_id = bubble_id.with("popup");
    let was_open = egui::Popup::is_id_open(ui.ctx(), popup_id);

    // Smooth hover animation only on entrance; instant pop on exit.
    let anim_id = bubble_id.with("chevron-slide-anim");
    let target_open = (bubble_hovered || was_open) && !view.selecting;
    let anim_progress = if target_open {
        ui.ctx().animate_bool_with_time(anim_id, true, 0.14)
    } else {
        ui.ctx().animate_bool_with_time(anim_id, false, 0.0);
        0.0
    };
    let ease = egui::emath::easing::cubic_out(anim_progress);

    let target_x = bubble_rect.right() - 14.0;
    let target_y = if bubble_rect.height() <= 28.0 {
        bubble_rect.center().y
    } else {
        bubble_rect.top() + 12.0
    };
    let slide_offset = (1.0 - ease) * 10.0;
    let c = pos2(target_x + slide_offset, target_y);
    let chevron_rect = Rect::from_center_size(c, vec2(20.0, 20.0));

    let chevron_id = bubble_id.with("chevron");
    let (chevron_resp, chevron_clicked) = if anim_progress > 0.08 {
        let resp = ui
            .interact(chevron_rect, chevron_id, Sense::CLICK)
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        let clicked = resp.clicked();
        (Some(resp), clicked)
    } else {
        (None, false)
    };

    let at_chevron_id = bubble_id.with("menu-at-chevron");
    if chevron_clicked && !was_open {
        let raw_x = if own {
            chevron_rect.right() - width
        } else {
            chevron_rect.left()
        };
        ui.ctx().data_mut(|data| {
            data.insert_temp(
                at_chevron_id,
                (raw_x, chevron_rect.bottom() + 4.0, chevron_rect.top() - 4.0),
            );
        });
    } else if right_clicked {
        let p = pointer.unwrap_or_else(|| bubble.rect.center());
        let raw_x = if own { p.x - width * 0.5 } else { p.x };
        ui.ctx()
            .data_mut(|data| data.insert_temp(at_chevron_id, (raw_x, p.y, p.y)));
    }
    let anchor_coords = ui
        .ctx()
        .data(|data| data.get_temp::<(f32, f32, f32)>(at_chevron_id));

    if anim_progress > 0.001 {
        let is_chevron_hovered = chevron_resp.as_ref().is_some_and(|r| r.hovered());
        let peak_alpha = if palette.dark { 85.0 } else { 45.0 };
        let corner_dark = Color32::from_black_alpha((peak_alpha * ease) as u8);
        let clear = Color32::from_rgba_premultiplied(0, 0, 0, 0);

        if tail == Some(widgets::Side::Right) {
            let edge = bubble_rect.right();
            let top = bubble_rect.top();
            let tip_dark = Color32::from_black_alpha((peak_alpha * ease * 0.81) as u8);
            let bottom_dark = Color32::from_black_alpha((peak_alpha * ease * 0.74) as u8);
            let mut tail_mesh = egui::Mesh::default();
            tail_mesh.colored_vertex(pos2(edge - 0.5, top), corner_dark);
            tail_mesh.colored_vertex(pos2(edge + widgets::TAIL_WIDTH, top), tip_dark);
            tail_mesh.colored_vertex(pos2(edge - 0.5, top + 11.0), bottom_dark);
            tail_mesh.add_triangle(0, 1, 2);
            ui.painter().add(egui::Shape::mesh(tail_mesh));
        }

        let painter = ui.painter().with_clip_rect(bubble_rect);

        let origin = pos2(bubble_rect.right(), bubble_rect.top());
        let radius: f32 = 42.0;
        let segments = 8;
        let mut mesh = egui::Mesh::default();
        mesh.colored_vertex(origin, corner_dark);
        for i in 0..=segments {
            let t = i as f32 / segments as f32;
            let angle = std::f32::consts::PI - t * std::f32::consts::FRAC_PI_2;
            let p = pos2(origin.x + radius * angle.cos(), origin.y + radius * angle.sin());
            mesh.colored_vertex(p, clear);
        }
        for i in 1..=segments as u32 {
            mesh.add_triangle(0, i, i + 1);
        }
        painter.add(egui::Shape::mesh(mesh));

        if is_chevron_hovered || was_open {
            let button_fill = if palette.dark {
                Color32::from_black_alpha((40.0 * ease) as u8)
            } else {
                Color32::from_black_alpha((20.0 * ease) as u8)
            };
            painter.rect_filled(chevron_rect.shrink(1.0), CornerRadius::same(5), button_fill);
        }

        let alpha = if is_chevron_hovered || was_open { 255.0 } else { 220.0 } * ease;
        let chevron_color = Color32::from_rgba_unmultiplied(255, 255, 255, alpha as u8);
        let stroke_width = 1.8;
        let stroke = Stroke::new(stroke_width, chevron_color);

        let half_w = 4.8;
        let half_h = 2.4;
        let left_pt = pos2(c.x - half_w, c.y - half_h);
        let tip_pt = pos2(c.x, c.y + half_h);
        let right_pt = pos2(c.x + half_w, c.y - half_h);

        painter.line_segment([left_pt, tip_pt], stroke);
        painter.line_segment([tip_pt, right_pt], stroke);
        painter.circle_filled(left_pt, stroke_width * 0.5, chevron_color);
        painter.circle_filled(tip_pt, stroke_width * 0.5, chevron_color);
        painter.circle_filled(right_pt, stroke_width * 0.5, chevron_color);
    }

    let at_message_id = bubble_id.with("menu-at-message");
    let at_message = force_menu
        || keyboard_clicked
        || (was_open
            && !right_clicked
            && ui
                .ctx()
                .data(|data| data.get_temp::<bool>(at_message_id))
                .unwrap_or(false));
    let from_keys_id = bubble_id.with("menu-from-keys");
    if force_menu {
        // Opened once: from here on the menu closes as any menu does.
        actions.push(Action::MessageMenuOpened);
        let keyboard = ui.ctx().data(|data| {
            data.get_temp::<bool>(theme::keyboard_focus_id())
                .unwrap_or(false)
        });
        if keyboard {
            ui.ctx()
                .data_mut(|data| data.insert_temp(from_keys_id, true));
        }
    }
    let from_keys = ui.ctx().data(|data| data.get_temp::<bool>(from_keys_id));
    let focus_in_menu = |ctx: &egui::Context| {
        ctx.memory(|memory| memory.focused())
            .and_then(|id| ctx.read_response(id))
            .map(|response| response.layer_id.id == popup_id)
    };
    if from_keys == Some(false) && was_open && !reacting && focus_in_menu(ui.ctx()) == Some(true) {
        let step = ui.ctx().input_mut(|input| {
            if super::keys::take_plain(input, egui::Key::ArrowDown) {
                Some(egui::FocusDirection::Next)
            } else if super::keys::take_plain(input, egui::Key::ArrowUp) {
                Some(egui::FocusDirection::Previous)
            } else {
                None
            }
        });
        if let Some(step) = step {
            ui.memory_mut(|memory| memory.move_focus(step));
        }
    }
    let keys_menu = from_keys.is_some();
    ui.ctx()
        .data_mut(|data| data.insert_temp(keys_menu_id(popup_id), keys_menu));
    let width = if keys_menu {
        let labels: Vec<String> = inner_entries(view, menu_target)
            .into_iter()
            .map(|entry| entry.label)
            .collect();
        let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
        width.max(widgets::menu_width(ui, &labels, true))
    } else {
        width
    };
    let open = if chevron_clicked {
        if was_open {
            Some(egui::SetOpenCommand::Bool(false))
        } else {
            Some(egui::SetOpenCommand::Bool(true))
        }
    } else if right_clicked || force_menu || reacting || keyboard_clicked {
        Some(egui::SetOpenCommand::Bool(true))
    } else if bubble.clicked() {
        Some(egui::SetOpenCommand::Bool(false))
    } else {
        None
    };
    let est_h = estimated_menu_height(view, menu_target, keys_menu);
    let menu_h = ui
        .ctx()
        .data(|data| data.get_temp::<Rect>(bubble_id.with("menu-rect")))
        .map_or(est_h, |rect| rect.height().max(120.0));

    let (raw_x, down_y, up_y) = anchor_coords.unwrap_or_else(|| {
        let p = bubble.rect.left_top() + vec2(12.0, 8.0);
        (p.x, p.y, bubble.rect.top())
    });

    let pad = 8.0;
    let min_x = viewport.left() + pad;
    let max_x = if reacting {
        (viewport.right() - width - 340.0).max(min_x)
    } else {
        (viewport.right() - width - pad).max(min_x)
    };
    let clamped_x = raw_x.clamp(min_x, max_x);

    let min_y = viewport.top() + pad;
    let max_y = (viewport.bottom() - menu_h - pad).max(min_y);
    let (base_y, opens_down) = if down_y + menu_h <= viewport.bottom() - pad {
        (down_y.clamp(min_y, max_y), true)
    } else {
        ((up_y - menu_h).clamp(min_y, max_y), false)
    };

    let popup = egui::Popup::menu(&bubble)
        .sense(Sense::CLICK)
        .open_memory(open)
        .close_behavior(if reacting {
            egui::PopupCloseBehavior::IgnoreClicks
        } else {
            egui::PopupCloseBehavior::CloseOnClickOutside
        })
        .width(width)
        .frame(egui::Frame::NONE)
        .at_position(pos2(clamped_x, base_y));

    let menu = popup.show(|ui| {
        context_menu(
            ui,
            view,
            menu_target,
            MenuOrigin::Chat { opens_down },
            actions,
        );
    });
    if let Some(menu) = &menu {
        ui.ctx()
            .data_mut(|data| data.insert_temp(bubble_id.with("menu-rect"), menu.response.rect));
    }
    let mut open_now = egui::Popup::is_id_open(ui.ctx(), popup_id);
    if !open_now {
        ui.ctx().data_mut(|data| {
            data.remove::<(f32, f32, f32)>(at_chevron_id);
            data.remove::<f64>(egui::Id::new(("msg-menu-open-time", &menu_target.id)));
        });
    }
    if let Some(focus_pending) = from_keys {
        if open_now && !focus_pending && !reacting && focus_in_menu(ui.ctx()) == Some(false) {
            egui::Popup::close_id(ui.ctx(), popup_id);
            open_now = false;
            ui.ctx().data_mut(|data| data.remove::<bool>(from_keys_id));
        } else if !open_now {
            ui.ctx().data_mut(|data| data.remove::<bool>(from_keys_id));
            if was_open
                && !ui.input(|input| input.pointer.any_pressed() || input.pointer.any_click())
            {
                let item = ui.memory(|memory| memory.focused());
                super::overlay::hand_back(ui.ctx(), messages_list_id(&view.chat.id), item);
            }
        } else if let Some(menu) = &menu
            && !reacting
        {
            ui.memory_mut(|memory| memory.set_modal_layer(menu.response.layer_id));
            // The first item, drawn in this pass, takes focus once.
            if focus_pending
                && let Some(first) = ui
                    .ctx()
                    .data(|data| {
                        data.get_temp::<Option<egui::Id>>(menu_first_item_id(
                            &view.chat.id,
                            &menu_target.id,
                        ))
                    })
                    .flatten()
            {
                ui.memory_mut(|memory| memory.request_focus(first));
                ui.ctx()
                    .data_mut(|data| data.insert_temp(from_keys_id, false));
            }
        }
    }
    if at_message || was_open {
        ui.ctx().data_mut(|data| {
            if at_message && open_now {
                data.insert_temp(at_message_id, true);
            } else {
                data.remove::<bool>(at_message_id);
            }
        });
    }
    if reacting && !egui::Popup::is_id_open(ui.ctx(), bubble_id.with("popup")) {
        actions.push(Action::ClosePicker);
    }
    inner.response.union(bubble)
}

/// Minimum shared width for cards inside message bubbles.
pub(crate) const CARD_WIDTH: f32 = 320.0;
pub(crate) const CAROUSEL_CARD_WIDTH: f32 = 280.0;
/// Location cards: a map preview across the top, then the details.
pub(crate) const LOCATION_CARD_WIDTH: f32 = 300.0;
pub(crate) const CAROUSEL_GAP: f32 = 8.0;

/// Returns the shared card width, bounded by [`CARD_WIDTH`] and `cap`.
pub(crate) fn settled_width(ui: &egui::Ui, view: &View<'_>, message: &Message, cap: f32) -> Option<f32> {
    if matches!(
        message.content,
        Content::Location { .. } | Content::LiveLocation { .. }
    ) {
        return Some(LOCATION_CARD_WIDTH.min(cap));
    }
    if let Content::Interactive {
        card: Some(card), ..
    } = &message.content
        && !card.carousel.is_empty()
    {
        let count = card.carousel.len();
        let cards = count as f32 * (CAROUSEL_CARD_WIDTH + 20.0);
        let gaps = count.saturating_sub(1) as f32 * CAROUSEL_GAP;
        return Some((cards + gaps).min(cap));
    }
    if let Content::Interactive {
        card: Some(card), ..
    } = &message.content
        && card.image.is_some()
    {
        return Some(CARD_WIDTH.min(cap));
    }
    let card = message.quoted.is_some()
        || match &message.content {
            Content::Text { preview, .. } => preview.is_some(),
            Content::Document { .. } | Content::Audio { .. } | Content::Poll { .. } => true,
            Content::Interactive { card, .. } => card.is_some(),
            Content::Video { note, .. } => !note && message.thumbnail.is_none(),
            _ => false,
        };
    card.then(|| {
        let floor = CARD_WIDTH.min(cap).max(0.0);
        natural_text_width(ui, view, message, cap).map_or(floor, |width| width.clamp(floor, cap))
    })
}

/// Widest wrapped text row, including footer space on the last line.
pub(crate) fn natural_text_width(ui: &egui::Ui, view: &View<'_>, message: &Message, cap: f32) -> Option<f32> {
    let palette = view.palette;
    let text = match &message.content {
        Content::Text { text, .. } | Content::Interactive { text, .. } => text,
        Content::Image {
            caption: Some(caption),
            ..
        }
        | Content::Video {
            caption: Some(caption),
            ..
        }
        | Content::Document {
            caption: Some(caption),
            ..
        } => caption,
        _ => return None,
    };
    let style = markup::Style {
        size: BODY_SIZE,
        color: palette.text,
        secondary: palette.secondary,
        link: palette.link,
        mention: palette.accent,
    };
    let laid = markup::layout(ui, text, &mentions_of(view, message), &style, cap);
    let widest = laid
        .galley
        .rows
        .iter()
        .map(|row| row.row.size.x)
        .fold(0.0, f32::max);
    let last = laid.galley.rows.last().map_or(0.0, |row| row.row.size.x);
    let starred = view.starred.is_some_and(|ids| ids.contains(&message.id));
    let pinned = view.pins.is_some_and(|ids| ids.contains(&message.id));
    let reserve = footer_width(ui, message, starred, pinned);
    Some(if last + 8.0 + reserve <= cap {
        widest.max(last + 8.0 + reserve)
    } else {
        widest
    })
}

pub(crate) const QUOTE_BAR: f32 = 4.0;
pub(crate) const QUOTE_RADIUS: u8 = 6;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct QuoteColours {
    pub(crate) fill: Color32,
    pub(crate) bar: Color32,
    pub(crate) name: Color32,
    pub(crate) text: Color32,
}

pub(crate) fn quote_colours(palette: &Palette, own: bool, tint: impl FnOnce() -> Color32) -> QuoteColours {
    if own && palette.bubbles == theme::BubbleStyle::Messages {
        return QuoteColours {
            fill: Color32::WHITE.gamma_multiply(0.15),
            bar: Color32::WHITE.gamma_multiply(0.6),
            name: Color32::WHITE,
            text: Color32::WHITE.gamma_multiply(0.6),
        };
    }
    let tint = tint();
    QuoteColours {
        fill: palette.window.gamma_multiply(0.35),
        bar: tint,
        name: tint,
        text: palette.secondary,
    }
}

/// Whether a quote is of one of our own messages.
pub(crate) fn quotes_me(view: &View<'_>, quoted: &crate::model::Quoted) -> bool {
    view.me == Some(quoted.sender.as_str())
}

pub(crate) fn quoted_sender_name(view: &View<'_>, quoted: &crate::model::Quoted, you: &str) -> String {
    if quotes_me(view, quoted) {
        you.to_owned()
    } else {
        (view.names_or)(&quoted.sender, quoted.sender_name.as_deref())
    }
}

pub(crate) fn quote_block(
    ui: &mut egui::Ui,
    view: &View<'_>,
    message: &Message,
    quoted: &crate::model::Quoted,
    width: f32,
    actions: &mut Vec<Action>,
) {
    let palette = view.palette;
    let mine = quotes_me(view, quoted);
    let who = quoted_sender_name(view, quoted, "You");
    let summary = markup::plain(
        &quoted.summary_in(view.locale),
        &quote_mentions(view, quoted),
    );
    let bubble = if message.from_me {
        palette.bubble_out
    } else {
        palette.bubble_in
    };
    let colours = quote_colours(&palette, message.from_me, || {
        theme::readable_on(
            bubble,
            if mine {
                palette.accent
            } else {
                palette.sender(crate::util::hue(&quoted.sender))
            },
            palette.text,
            3.0,
        )
    });
    let response = Frame::new()
        .fill(colours.fill)
        .corner_radius(CornerRadius::same(QUOTE_RADIUS))
        .inner_margin(Margin {
            left: QUOTE_BAR as i8 + 7,
            right: 10,
            top: 5,
            bottom: 6,
        })
        .show(ui, |ui| {
            let inner_width = (width - QUOTE_BAR - 17.0).max(0.0);
            ui.allocate_ui_with_layout(
                vec2(inner_width, 0.0),
                Layout::top_down(Align::Min),
                |ui| {
                    ui.set_width(inner_width);
                    ui.spacing_mut().item_spacing.y = 1.0;
                    widgets::rich_text(ui, &who, theme::semibold(12.5), colours.name);
                    widgets::rich_text(ui, &summary, theme::regular(12.5), colours.text);
                },
            );
        })
        .response;
    // The bar runs the quote's full height along its rounded left edge.
    let bar = Rect::from_min_size(response.rect.min, vec2(QUOTE_BAR, response.rect.height()));
    ui.painter().rect_filled(
        bar,
        CornerRadius {
            nw: QUOTE_RADIUS,
            sw: QUOTE_RADIUS,
            ne: 0,
            se: 0,
        },
        colours.bar,
    );
    ui.ctx().data_mut(|data| {
        data.insert_temp(
            bubble_id(&view.chat.id, &message.id).with("quote"),
            response.rect,
        );
    });
    let response = ui
        .interact(
            response.rect,
            ui.id().with(("quote", &message.id, &quoted.id)),
            Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    if response.clicked() {
        actions.push(quote_jump_action(quoted));
    }
}

pub(crate) const CALL_BADGE: f32 = 36.0;
pub(crate) const CALL_GLYPH: f32 = 18.0;
pub(crate) const CALL_BADGE_GAP: f32 = 10.0;
pub(crate) const CALL_LINE_GAP: f32 = 2.0;
pub(crate) const FOOTER_HEIGHT: f32 = 15.0;

pub(crate) fn call_entry(
    ui: &mut egui::Ui,
    palette: &Palette,
    entry: &super::calls::Entry,
    locale: crate::i18n::Locale,
    tint: Color32,
    width: f32,
    reserve: f32,
) -> Option<Rect> {
    let title = entry.title(locale);
    let detail = entry.detail(locale);
    let title_galley =
        ui.painter()
            .layout_no_wrap(title.clone(), theme::medium(14.0), palette.text);
    let detail_galley = detail.as_ref().map(|detail| {
        ui.painter()
            .layout_no_wrap(detail.clone(), theme::regular(12.5), palette.secondary)
    });
    let detail_width = detail_galley.as_ref().map_or(0.0, |galley| galley.size().x);
    let gap = if detail_galley.is_some() { 8.0 } else { 0.0 };
    let lead = CALL_BADGE + CALL_BADGE_GAP;
    let inline = lead + detail_width + gap + reserve <= width;
    let second_width = if inline {
        detail_width + gap + reserve
    } else {
        detail_width
    };
    let second_height = match (&detail_galley, inline) {
        (Some(galley), true) => galley.size().y.max(FOOTER_HEIGHT),
        (Some(galley), false) => galley.size().y,
        (None, true) => FOOTER_HEIGHT,
        (None, false) => 0.0,
    };
    let block_height = title_galley.size().y
        + if second_height > 0.0 {
            CALL_LINE_GAP + second_height
        } else {
            0.0
        };
    let content_width = lead + title_galley.size().x.max(second_width);
    let row_width = content_width
        .max(ui.min_rect().width())
        .min(width.max(content_width));
    let (rect, response) = ui.allocate_exact_size(
        vec2(row_width, block_height.max(CALL_BADGE)),
        Sense::hover(),
    );
    response.widget_info(|| {
        let label = match &detail {
            Some(detail) => format!("{title}, {detail}"),
            None => title.clone(),
        };
        egui::WidgetInfo::labeled(egui::WidgetType::Label, true, label)
    });
    let badge = Rect::from_center_size(
        pos2(rect.left() + CALL_BADGE / 2.0, rect.center().y),
        Vec2::splat(CALL_BADGE),
    );
    let text_left = badge.right() + CALL_BADGE_GAP;
    let top = rect.center().y - block_height / 2.0;
    let second_top = top + title_galley.size().y + CALL_LINE_GAP;
    let second_middle = second_top + second_height / 2.0;
    if ui.is_rect_visible(rect) {
        ui.painter()
            .circle_filled(badge.center(), CALL_BADGE / 2.0, tint.gamma_multiply(0.16));
        theme::paint_icon(
            ui,
            entry.icon(),
            Rect::from_center_size(badge.center(), Vec2::splat(CALL_GLYPH)),
            CALL_GLYPH,
            tint,
        );
        ui.painter()
            .galley(pos2(text_left, top), title_galley, palette.text);
        if let Some(galley) = detail_galley {
            let at = pos2(text_left, second_middle - galley.size().y / 2.0);
            ui.painter().galley(at, galley, palette.secondary);
        }
    }
    inline.then(|| {
        Rect::from_min_size(
            pos2(rect.right() - reserve, second_middle - FOOTER_HEIGHT / 2.0),
            vec2(reserve, FOOTER_HEIGHT),
        )
    })
}

/// Keeps row contents left-to-right inside right-aligned own bubbles.
pub(crate) fn mirrored_row(
    ui: &mut egui::Ui,
    own: bool,
    first: impl FnOnce(&mut egui::Ui),
    second: impl FnOnce(&mut egui::Ui),
) {
    ui.horizontal(|ui| {
        if own {
            second(ui);
            first(ui);
        } else {
            first(ui);
            second(ui);
        }
    });
}

pub(crate) const FORWARDED_ICON: f32 = 14.0;
pub(crate) const FORWARDED_GAP: f32 = 4.0;
pub(crate) const FORWARDED_HEIGHT: f32 = 15.0;

pub(crate) fn forwarded_label(ui: &mut egui::Ui, palette: &Palette) -> (Rect, std::sync::Arc<egui::Galley>) {
    let mut job = egui::text::LayoutJob::default();
    job.append(
        "Forwarded",
        0.0,
        egui::TextFormat {
            font_id: theme::regular(12.5),
            color: palette.dim,
            italics: true,
            ..Default::default()
        },
    );
    let galley = ui.painter().layout_job(job);
    let width = FORWARDED_ICON + FORWARDED_GAP + galley.size().x;
    let (reserved, response) =
        ui.allocate_exact_size(vec2(width, FORWARDED_HEIGHT), Sense::hover());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, "Forwarded"));
    // Sit closer to what follows than the bubble's usual spacing.
    ui.add_space(-1.0);
    (reserved, galley)
}

pub(crate) fn paint_forwarded_label(
    ui: &egui::Ui,
    palette: &Palette,
    at: egui::Pos2,
    galley: std::sync::Arc<egui::Galley>,
) -> Rect {
    let rect = Rect::from_min_size(
        at,
        vec2(
            FORWARDED_ICON + FORWARDED_GAP + galley.size().x,
            FORWARDED_HEIGHT,
        ),
    );
    let icon = Rect::from_min_size(
        pos2(rect.left(), rect.center().y - FORWARDED_ICON / 2.0),
        Vec2::splat(FORWARDED_ICON),
    );
    theme::paint_icon(ui, Icon::Forward, icon, FORWARDED_ICON, palette.dim);
    ui.painter().galley(
        pos2(
            icon.right() + FORWARDED_GAP,
            rect.center().y - galley.size().y / 2.0,
        ),
        galley,
        palette.dim,
    );
    rect
}

pub(crate) const FOOTER_MARK: f32 = 13.0;
pub(crate) const FOOTER_MARK_GAP: f32 = 8.0;
pub(crate) const FOOTER_MARK_SLOT: f32 = FOOTER_MARK + FOOTER_MARK_GAP;
pub(crate) const FOOTER_STAR: Color32 = Color32::from_rgb(0xEA, 0xB3, 0x08);

pub(crate) fn clock_width(ui: &egui::Ui, message: &Message) -> f32 {
    ui.painter()
        .layout_no_wrap(
            crate::util::clock(message.timestamp),
            theme::regular(11.0),
            Color32::WHITE,
        )
        .size()
        .x
}

/// Width of the message footer, including the room its marks take.
pub(crate) fn footer_width(ui: &egui::Ui, message: &Message, starred: bool, pinned: bool) -> f32 {
    let font = theme::regular(11.0);
    let time = clock_width(ui, message);
    let edited = if message.edited {
        ui.painter()
            .layout_no_wrap("edited".to_owned(), font, Color32::WHITE)
            .size()
            .x
            + 4.0
    } else {
        0.0
    };
    let not_sent = if not_sent(message) {
        ui.painter()
            .layout_no_wrap(NOT_SENT.to_owned(), theme::medium(11.0), Color32::WHITE)
            .size()
            .x
            + 6.0
    } else {
        0.0
    };
    let marks =
        if starred { FOOTER_MARK_SLOT } else { 0.0 } + if pinned { FOOTER_MARK_SLOT } else { 0.0 };
    time + edited + not_sent + if message.from_me { 19.0 } else { 0.0 } + marks
}

pub(crate) fn time_over_picture(message: &Message) -> bool {
    matches!(message.content, Content::Image { caption: None, .. })
}

/// The ticks a message in this view shows (see [`display_delivery`]).
pub(crate) fn shown_delivery(view: &View<'_>, message: &Message) -> Delivery {
    display_delivery(
        message.status,
        message.from_me,
        view.chat.kind,
        view.account_receipts_off,
    )
}

pub fn display_delivery(
    status: Delivery,
    from_me: bool,
    chat_kind: crate::model::ChatKind,
    account_receipts_off: bool,
) -> Delivery {
    if account_receipts_off
        && chat_kind == crate::model::ChatKind::Direct
        && from_me
        && status == Delivery::Read
    {
        Delivery::Delivered
    } else {
        status
    }
}

pub(crate) fn bare_picture(palette: &Palette, message: &Message) -> bool {
    palette.bubbles == theme::BubbleStyle::Messages
        && time_over_picture(message)
        && message.quoted.is_none()
        && !message.forwarded
}

/// How round a picture's corners are: a bubble's, when it has none.
pub(crate) fn picture_radius(palette: &Palette, message: &Message) -> f32 {
    if bare_picture(palette, message) {
        f32::from(widgets::MESSAGES_BUBBLE_RADIUS)
    } else {
        6.0
    }
}

pub(crate) const OVER_PICTURE_INSET: Vec2 = vec2(10.0, 6.0);

pub(crate) fn footer_over_picture(
    ui: &mut egui::Ui,
    view: &View<'_>,
    message: &Message,
    picture: Rect,
    starred: bool,
    pinned: bool,
    actions: &mut Vec<Action>,
) {
    let palette = &view.palette;
    let font = theme::regular(11.0);
    let time =
        ui.painter()
            .layout_no_wrap(crate::util::clock(message.timestamp), font, Color32::WHITE);
    let failed = not_sent(message).then(|| {
        ui.painter()
            .layout_no_wrap(NOT_SENT.to_owned(), theme::medium(11.0), Color32::WHITE)
    });
    let tick_width = if message.from_me { 19.0 } else { 0.0 };
    let marks =
        if starred { FOOTER_MARK_SLOT } else { 0.0 } + if pinned { FOOTER_MARK_SLOT } else { 0.0 };
    let width = time.size().x
        + failed.as_ref().map_or(0.0, |galley| galley.size().x + 6.0)
        + tick_width
        + marks;
    let row = Rect::from_min_max(
        pos2(
            picture.right() - OVER_PICTURE_INSET.x - width,
            picture.bottom() - OVER_PICTURE_INSET.y - 15.0,
        ),
        picture.right_bottom() - OVER_PICTURE_INSET,
    );
    if ui.is_rect_visible(picture) {
        let scrim = row.expand2(vec2(6.0, 2.0)).intersect(picture);
        ui.painter()
            .rect_filled(scrim, scrim.height() / 2.0, Color32::from_black_alpha(110));
    }
    let mut x = row.right();
    if message.from_me {
        let ticks = Rect::from_center_size(pos2(x - 7.5, row.center().y), Vec2::splat(15.0));
        widgets::ticks_in(
            ui,
            palette,
            ticks,
            shown_delivery(view, message),
            Color32::WHITE,
        );
        x -= tick_width;
    }
    x -= time.size().x;
    ui.painter().galley(
        pos2(x, row.center().y - time.size().y / 2.0),
        time,
        Color32::WHITE,
    );
    let status =
        Rect::from_x_y_ranges(x..=row.right(), row.center().y - 7.5..=row.center().y + 7.5);
    status_opens_info(ui, view, message, status, actions);
    if let Some(failed) = failed {
        x -= failed.size().x + 6.0;
        let label = Rect::from_min_size(
            pos2(x, row.center().y - failed.size().y / 2.0),
            failed.size(),
        );
        ui.painter().galley(label.min, failed, Color32::WHITE);
        let status = Rect::from_min_max(label.min, pos2(row.right(), label.max.y));
        let response = ui.interact(
            status,
            ui.id().with(("not-sent", &message.id)),
            Sense::hover(),
        );
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Label, true, NOT_SENT_HINT)
        });
        response.on_hover_text(NOT_SENT_HINT);
    }
    if pinned {
        x -= FOOTER_MARK_SLOT;
        theme::paint_icon(
            ui,
            Icon::Pin,
            Rect::from_center_size(
                pos2(x + FOOTER_MARK / 2.0, row.center().y),
                Vec2::splat(FOOTER_MARK),
            ),
            FOOTER_MARK,
            Color32::WHITE,
        );
    }
    if starred {
        x -= FOOTER_MARK_SLOT;
        theme::paint_icon(
            ui,
            Icon::StarFill,
            Rect::from_center_size(
                pos2(x + FOOTER_MARK / 2.0, row.center().y),
                Vec2::splat(FOOTER_MARK),
            ),
            FOOTER_MARK,
            FOOTER_STAR,
        );
    }
    ui.ctx().data_mut(|data| {
        data.insert_temp(footer_id(&message.chat, &message.id), row);
    });
}

/// Where a message's time and ticks were drawn, for layout tests.
pub fn footer_id(chat: &str, message: &str) -> egui::Id {
    bubble_id(chat, message).with("footer")
}

/// Whether "Message info" has receipts to show: our own sent message.
pub(crate) fn has_message_info(message: &Message) -> bool {
    message.from_me
        && !matches!(message.content, Content::Revoked)
        && !matches!(
            message.status,
            Delivery::None | Delivery::Pending | Delivery::Failed
        )
}

/// Clicking our time and ticks opens "Message info", as in WhatsApp.
pub(crate) fn status_opens_info(
    ui: &mut egui::Ui,
    view: &View<'_>,
    message: &Message,
    status: Rect,
    actions: &mut Vec<Action>,
) {
    if !has_message_info(message) {
        return;
    }
    let label = crate::i18n::gettext(view.locale, "Message info");
    let response = ui.interact(
        status.expand(2.0),
        ui.id().with(("status", &message.id)),
        Sense::click(),
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &label));
    let response = response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(&*label);
    if response.clicked() {
        actions.push(Action::ShowDialog(Dialog::MessageInfo {
            chat: message.chat.clone(),
            message: message.id.clone(),
        }));
    }
}

pub(crate) fn not_sent(message: &Message) -> bool {
    message.from_me && message.status == Delivery::Failed
}

/// Paints the time and ticks at the bubble's right edge without widening it.
pub(crate) fn footer(
    ui: &mut egui::Ui,
    view: &View<'_>,
    message: &Message,
    slot: Option<Rect>,
    starred: bool,
    pinned: bool,
    actions: &mut Vec<Action>,
) {
    let palette = &view.palette;
    let font = theme::regular(11.0);
    let time = ui.painter().layout_no_wrap(
        crate::util::clock(message.timestamp),
        font.clone(),
        palette.secondary,
    );
    let edited = message.edited.then(|| {
        ui.painter()
            .layout_no_wrap("edited".to_owned(), font, palette.dim)
    });
    let failed = not_sent(message).then(|| {
        ui.painter()
            .layout_no_wrap(NOT_SENT.to_owned(), theme::medium(11.0), palette.text)
    });
    let tick_width = if message.from_me { 19.0 } else { 0.0 };
    let width = time.size().x
        + edited.as_ref().map_or(0.0, |galley| galley.size().x + 4.0)
        + failed.as_ref().map_or(0.0, |galley| galley.size().x + 6.0)
        + tick_width;
    let rect = match slot {
        Some(slot) => slot,
        None => {
            let row_width = ui.min_rect().width().max(width);
            let (rect, _) = ui.allocate_exact_size(vec2(row_width, 15.0), Sense::hover());
            rect
        }
    };
    ui.ctx().data_mut(|data| {
        data.insert_temp(
            footer_id(&message.chat, &message.id),
            Rect::from_min_max(pos2(rect.right() - width, rect.top()), rect.max),
        );
    });
    let mut x = rect.right();
    if message.from_me {
        let ticks = Rect::from_center_size(pos2(x - 7.5, rect.center().y), Vec2::splat(15.0));
        widgets::ticks(ui, palette, ticks, shown_delivery(view, message));
        x -= tick_width;
    }
    x -= time.size().x;
    ui.painter().galley(
        pos2(x, rect.center().y - time.size().y / 2.0),
        time,
        palette.secondary,
    );
    let status = Rect::from_x_y_ranges(
        x..=rect.right(),
        rect.center().y - 7.5..=rect.center().y + 7.5,
    );
    status_opens_info(ui, view, message, status, actions);
    if pinned {
        // Left of the clock, in the room `footer_width` counted for it.
        x -= FOOTER_MARK_SLOT;
        theme::paint_icon(
            ui,
            Icon::Pin,
            Rect::from_center_size(
                pos2(x + FOOTER_MARK / 2.0, rect.center().y),
                Vec2::splat(FOOTER_MARK),
            ),
            FOOTER_MARK,
            palette.secondary,
        );
    }
    if let Some(edited) = edited {
        x -= edited.size().x + 4.0;
        ui.painter().galley(
            pos2(x, rect.center().y - edited.size().y / 2.0),
            edited,
            palette.dim,
        );
    }
    if let Some(failed) = failed {
        x -= failed.size().x + 6.0;
        let label = Rect::from_min_size(
            pos2(x, rect.center().y - failed.size().y / 2.0),
            failed.size(),
        );
        ui.painter().galley(label.min, failed, palette.text);
        let status = Rect::from_min_max(label.min, pos2(rect.right(), label.max.y));
        let response = ui.interact(
            status,
            ui.id().with(("not-sent", &message.id)),
            Sense::hover(),
        );
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Label, true, NOT_SENT_HINT)
        });
        response.on_hover_text(NOT_SENT_HINT);
    }
    if starred {
        x -= FOOTER_MARK_SLOT;
        theme::paint_icon(
            ui,
            Icon::StarFill,
            Rect::from_center_size(
                pos2(x + FOOTER_MARK / 2.0, rect.center().y),
                Vec2::splat(FOOTER_MARK),
            ),
            FOOTER_MARK,
            FOOTER_STAR,
        );
    }
}

/// Groups most-used emoji first; stable ties preserve their first-seen order.
pub(crate) fn reaction_groups(message: &Message) -> Vec<(&str, Vec<&crate::model::Reaction>)> {
    let mut groups: Vec<(&str, Vec<&crate::model::Reaction>)> = Vec::new();
    for reaction in &message.reactions {
        if let Some((_, people)) = groups
            .iter_mut()
            .find(|(emoji, _)| *emoji == reaction.emoji)
        {
            people.push(reaction);
        } else {
            groups.push((&reaction.emoji, vec![reaction]));
        }
    }
    groups.sort_by_key(|(_, people)| std::cmp::Reverse(people.len()));
    groups
}

pub(crate) fn reactions(ui: &mut egui::Ui, view: &View<'_>, message: &Message, actions: &mut Vec<Action>) {
    let palette = view.palette;
    let groups = reaction_groups(message);
    let mine = message.reactions.iter().any(|reaction| reaction.from_me);
    let label = format!(
        "{} {}",
        groups
            .iter()
            .take(3)
            .map(|(emoji, _)| *emoji)
            .collect::<Vec<_>>()
            .join(" "),
        message.reactions.len()
    );
    let line = widgets::line(ui, &label, theme::regular(13.0), palette.text, 200.0, 1);
    let (_, rect) = ui.allocate_space(line.size() + vec2(12.0, 6.0));
    let response = ui.interact(
        rect,
        bubble_id(&message.chat, &message.id).with("reactions"),
        Sense::click(),
    );
    if ui.is_rect_visible(rect) {
        let mut shadow = palette.bubble_shadow();
        shadow.color = shadow.color.gamma_multiply(0.6);
        ui.painter()
            .add(shadow.as_shape(rect, CornerRadius::from(rect.height() / 2.0)));
        ui.painter()
            .rect_filled(rect, rect.height() / 2.0, palette.overlay);
        ui.painter().rect_stroke(
            rect,
            rect.height() / 2.0,
            Stroke::new(1.0, if mine { palette.accent } else { palette.chat }),
            egui::StrokeKind::Inside,
        );
        line.paint(ui, rect.center() - line.size() / 2.0, palette.text);
    }
    let name = |reaction: &crate::model::Reaction| {
        if reaction.from_me {
            crate::i18n::gettext(view.locale, "You").into_owned()
        } else {
            (view.names_or)(&reaction.sender, None)
        }
    };
    let response = response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(
            groups
                .iter()
                .map(|(emoji, people)| {
                    format!(
                        "{emoji}: {}",
                        people
                            .iter()
                            .map(|reaction| name(reaction))
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                })
                .collect::<Vec<_>>()
                .join("\n"),
        );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &label));
    egui::Popup::menu(&response)
        .id(bubble_id(&message.chat, &message.id).with("reaction-list"))
        .width(280.0)
        .frame(widgets::menu_frame(&palette))
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            egui::ScrollArea::horizontal()
                .id_salt("reaction-summary")
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        for (emoji, people) in &groups {
                            widgets::rich_text(
                                ui,
                                &format!("{emoji} {}", people.len()),
                                theme::regular(13.0),
                                palette.text,
                            );
                        }
                    });
                });
            widgets::menu_separator(ui, &palette);
            egui::ScrollArea::vertical()
                .max_height(300.0)
                .show(ui, |ui| {
                    for (emoji, people) in &groups {
                        for reaction in people {
                            ui.horizontal(|ui| {
                                let who = name(reaction);
                                let id = if reaction.from_me {
                                    view.me.unwrap_or(&reaction.sender)
                                } else {
                                    &reaction.sender
                                };
                                widgets::avatar(
                                    ui,
                                    &palette,
                                    &who,
                                    id,
                                    28.0,
                                    view.avatars.get(id).and_then(|path| path.as_deref()),
                                );
                                if reaction.from_me {
                                    if ui
                                        .add_enabled(view.chat.can_send(), egui::Button::new(&who))
                                        .on_hover_text(crate::i18n::gettext(
                                            view.locale,
                                            "Remove reaction",
                                        ))
                                        .clicked()
                                    {
                                        actions.push(Action::React {
                                            chat: view.chat.id.clone(),
                                            message: message.id.clone(),
                                            emoji: String::new(),
                                        });
                                        ui.close();
                                    }
                                } else {
                                    theme::text(ui, who, theme::regular(13.0), palette.text);
                                }
                                widgets::rich_text(ui, emoji, theme::regular(16.0), palette.text);
                            });
                        }
                    }
                });
        });
}

pub(crate) const QUICK_REACTIONS: [&str; 6] = ["👍", "❤️", "😂", "😮", "😢", "🙏"];

/// Our existing reaction to a message.
pub(crate) fn own_reaction(message: &Message) -> Option<&str> {
    message
        .reactions
        .iter()
        .find(|reaction| reaction.from_me)
        .map(|reaction| reaction.emoji.as_str())
}
