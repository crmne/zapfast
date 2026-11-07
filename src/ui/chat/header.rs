//! Chat header and pinned message banner objects.

use super::*;

pub struct ChatHeader;

impl ChatHeader {
    pub fn show(app: &mut App, ui: &mut egui::Ui, chat: &Chat) -> Rect {
        header(app, ui, chat)
    }

    pub fn subtitle(app: &App, chat: &Chat) -> (String, Color32) {
        subtitle(app, chat)
    }
}

pub struct PinBanner;

impl PinBanner {
    pub fn show(app: &mut App, ui: &mut egui::Ui, chat: &Chat) {
        pin_banner(app, ui, chat);
    }
}

pub(crate) const PIN_ROW_HEIGHT: f32 = 44.0;
/// Gap between the banner edge and the pin count bar, and the icon's own box.
pub(crate) const PIN_ROW_INSET: f32 = 12.0;
pub(crate) const PIN_ICON: f32 = 16.0;
/// The bar at the line's left edge: one segment per pin, the shown one lit.
pub(crate) const PIN_BAR_WIDTH: f32 = 3.0;
pub(crate) const PIN_BAR_GAP: f32 = 2.0;

pub(crate) fn shown_pin(rows: &[crate::archive::Pinned], cursor: Option<&str>) -> usize {
    cursor
        .and_then(|id| rows.iter().position(|row| row.id == id))
        .unwrap_or(0)
}

pub(crate) fn next_pin(rows: usize, shown: usize) -> usize {
    if rows == 0 { 0 } else { (shown + 1) % rows }
}

pub(crate) fn pin_bar_segments(top: f32, height: f32, count: usize) -> Vec<(f32, f32)> {
    let count = count.max(1);
    let gaps = PIN_BAR_GAP * (count - 1) as f32;
    let segment = ((height - gaps) / count as f32).max(1.0);
    (0..count)
        .map(|index| {
            let start = top + (segment + PIN_BAR_GAP) * index as f32;
            (start, start + segment)
        })
        .collect()
}

pub(crate) fn pin_banner(app: &mut App, ui: &mut egui::Ui, chat: &Chat) {
    let now = crate::util::now();
    if let Some(wait) = app.chat_pins.get(&chat.id).and_then(|rows| {
        rows.iter()
            .filter(|row| row.expires_at > now)
            .map(|row| row.expires_at)
            .min()
    }) {
        let left = wait.saturating_sub(now).clamp(0, 86_400) as u64;
        ui.ctx()
            .request_repaint_after(Duration::from_secs(left.saturating_add(1)));
    }
    let rows = live_pins(
        app.chat_pins.get(&chat.id).map_or(&[][..], Vec::as_slice),
        now,
    );
    if rows.is_empty() {
        return;
    }
    // Which pin is shown is the view's own state, kept per chat.
    let cursor_id = pin_cursor_id(&chat.id);
    let cursor = ui.ctx().data(|data| data.get_temp::<String>(cursor_id));
    let shown = shown_pin(&rows, cursor.as_deref());
    let row = &rows[shown];
    let palette = app.palette;
    let (bar, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), PIN_ROW_HEIGHT), Sense::click());
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    ui.painter().rect_filled(
        bar,
        0.0,
        if response.hovered() {
            palette.surface_hover
        } else {
            palette.panel
        },
    );
    // Newest pin at the bottom, so the bar fills upward as pins are added.
    let segments = pin_bar_segments(bar.top() + 8.0, bar.height() - 16.0, rows.len());
    let left = bar.left() + PIN_ROW_INSET;
    for (index, (top, bottom)) in segments.iter().enumerate() {
        let pin = rows.len() - 1 - index;
        let colour = if pin == shown {
            palette.accent
        } else {
            palette.dim
        };
        ui.painter().rect_filled(
            Rect::from_x_y_ranges(left..=left + PIN_BAR_WIDTH, *top..=*bottom),
            PIN_BAR_WIDTH / 2.0,
            colour,
        );
    }
    let icon_left = left + PIN_BAR_WIDTH + 10.0;
    let icon = Rect::from_center_size(
        pos2(icon_left + PIN_ICON / 2.0, bar.center().y),
        Vec2::splat(PIN_ICON),
    );
    theme::paint_icon(ui, Icon::Pin, icon, 14.0, palette.accent);
    let text_left = icon_left + PIN_ICON + 8.0;
    let text_width = (bar.right() - PIN_ROW_INSET - text_left).max(1.0);
    let label = widgets::line(
        ui,
        &crate::markup::plain(&app.resolve_mention_tokens(&row.text), &[]),
        theme::regular(13.0),
        palette.text,
        text_width,
        1,
    );
    label.paint(
        ui,
        pos2(text_left, bar.center().y - label.size().y / 2.0),
        palette.text,
    );
    theme::reveal_focus(&response);
    let described = crate::i18n::gettext(app.locale, "Pinned message {index} of {count}: {text}")
        .replace("{index}", &(shown + 1).to_string())
        .replace("{count}", &rows.len().to_string())
        .replace("{text}", &row.text);
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), &described)
    });
    if response.clicked() {
        app.actions.push(Action::OpenMessage {
            chat: chat.id.clone(),
            message: row.id.clone(),
        });
        let next = rows[next_pin(rows.len(), shown)].id.clone();
        ui.ctx().data_mut(|data| data.insert_temp(cursor_id, next));
    }
}

/// Where the pin line keeps the id of the pin it shows, per chat.
pub(crate) fn pin_cursor_id(chat: &str) -> egui::Id {
    egui::Id::new(("pin-cursor", chat))
}

pub(crate) fn empty(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    let rect = ui.max_rect();
    let center = rect.center() - vec2(0.0, 30.0);
    theme::logo(
        ui,
        center - vec2(0.0, 60.0),
        72.0,
        palette.surface,
        palette.dim,
    );
    ui.painter().text(
        center,
        Align2::CENTER_CENTER,
        "ZapFast",
        theme::bold(24.0),
        palette.text,
    );
    ui.painter().text(
        center + vec2(0.0, 30.0),
        Align2::CENTER_CENTER,
        if app.chats.is_empty() {
            crate::i18n::gettext(app.locale, "Your chats appear on the left as they load.")
        } else {
            crate::i18n::gettext(app.locale, "Select a chat on the left.")
        },
        theme::regular(14.0),
        palette.secondary,
    );
}

pub(crate) const HEADER_EASE: f32 = 0.35;
/// How far below a vibrant header its frost and veil carry on, easing off.
pub(crate) const HEADER_TAIL: f32 = 24.0;
/// Rows of the veil's ease, enough for its curve to look smooth.
pub(crate) const VEIL_STEPS: usize = 8;

pub(crate) fn header_veil(palette: &Palette, rect: Rect, ease: f32) -> egui::Shape {
    let top = palette.vibrant_header(palette.chat);
    let start = rect.bottom() - ease;
    let mut mesh = egui::Mesh::default();
    let mut row = |y: f32, color: Color32| {
        mesh.colored_vertex(pos2(rect.left(), y), color);
        mesh.colored_vertex(pos2(rect.right(), y), color);
    };
    row(rect.top(), top);
    for step in 0..=VEIL_STEPS {
        let t = step as f32 / VEIL_STEPS as f32;
        // smoothstep, as the frost's shader eases.
        let strength = 1.0 - t * t * (3.0 - 2.0 * t);
        row(start + ease * t, top.gamma_multiply(strength));
    }
    let rows = mesh.vertices.len() as u32 / 2;
    for row in 0..rows - 1 {
        let at = row * 2;
        mesh.add_triangle(at, at + 1, at + 3);
        mesh.add_triangle(at, at + 3, at + 2);
    }
    egui::Shape::mesh(mesh)
}

pub(crate) const NAME_GLASS_RADIUS: f32 = 12.0;
pub(crate) const NAME_GLASS_PADDING: Vec2 = vec2(14.0, 3.5);
/// The room above and below a name alone on its glass.
pub(crate) const NAME_GLASS_SINGLE: f32 = 7.0;
pub(crate) const NAME_GLASS_LINES: f32 = 32.0;
/// How wide the round glass under each of a vibrant header's buttons is.
pub(crate) const BUTTON_GLASS: f32 = 36.0;
/// How wide `buttons` of the chat header are together.
pub(crate) fn controls_width(buttons: f32, vibrant: bool) -> f32 {
    if vibrant {
        buttons * 36.0 + (buttons - 0.5) * BUTTON_GAP + (BUTTON_GLASS - 30.0)
    } else {
        // Each button and the spacing beside it.
        buttons * 44.0
    }
}

pub(crate) fn header_fits(width: f32, buttons: f32, extra: f32, vibrant: bool) -> bool {
    const AVATAR: f32 = 40.0 + 14.0;
    const NAME: f32 = 140.0;
    width >= AVATAR + NAME + controls_width(buttons, vibrant) + extra
}

/// Space between the header's last button and caption buttons `room` wide.
pub(crate) fn caption_gap(room: f32) -> f32 {
    if room > 0.0 { 8.0 } else { 0.0 }
}

/// The space between two of a vibrant header's buttons, beyond their glass.
pub(crate) const BUTTON_GAP: f32 = 10.0;
pub(crate) const HEADER_MIN_CONTENT: f32 = 320.0;

/// The round glass under one of a vibrant header's buttons.
pub(crate) fn glass_button(palette: &Palette, button: Rect) -> egui::Shape {
    glass(
        palette,
        Rect::from_center_size(button.center(), Vec2::splat(BUTTON_GLASS)),
        BUTTON_GLASS / 2.0,
    )
}

pub(crate) fn glass(palette: &Palette, rect: Rect, radius: f32) -> egui::Shape {
    let radius = CornerRadius::from(radius);
    egui::Shape::Vec(vec![
        egui::Shape::rect_filled(rect, radius, palette.vibrant_pill(palette.chat)),
        egui::Shape::rect_stroke(
            rect,
            radius,
            Stroke::new(1.0, palette.text.gamma_multiply(0.08)),
            egui::StrokeKind::Inside,
        ),
    ])
}

pub(crate) fn header(app: &mut App, ui: &mut egui::Ui, chat: &Chat) -> Rect {
    let palette = app.palette;
    let title = app.chat_title(chat);
    let rect = egui::Panel::top("chat-header")
        .show_separator_line(false)
        .frame(
            Frame::new()
                .fill(if palette.vibrant() {
                    Color32::TRANSPARENT
                } else {
                    palette.panel
                })
                .inner_margin(Margin::symmetric(14, 8)),
        )
        .show(ui, |ui| {
            // The chat list, expanded or collapsed, clears the traffic lights.
            if theme::chrome_in_headers(ui.ctx()) {
                super::titlebar_drag(ui, ui.max_rect());
            }
            let room = super::caption_room(ui.ctx(), ui.max_rect(), HEADER_MIN_CONTENT);
            ui.add_space(room.top);
            let narrow = super::narrow(ui.ctx());
            let row = ui.horizontal(|ui| {
                // Give both rows a fixed height so their contents align.
                ui.set_min_height(HEADER_ROW);
                if narrow && theme::macos_chrome(ui.ctx()) {
                    ui.add_space(theme::traffic_light_inset(ui.ctx()));
                } else if app.sidebar_mode() == crate::model::SidebarDisplayMode::CollapsedIconsOnly
                    && app.page == crate::model::Page::Chats
                    && !super::macos_title_strip(app, ui.ctx())
                {
                    ui.add_space(super::rail::lights_overhang(ui.ctx()));
                }
                ui.add_space(room.left);
                let picture = app.avatar(&chat.id);
                let (subtitle, color) = subtitle(app, chat);
                let call_here = app.call_live_here(&chat.id);
                let call_buttons = if callable(chat, app.me.as_deref())
                    && (call_here || !app.blocklist.contains(&chat.id))
                {
                    if call_here { 1.0 } else { 2.0 }
                } else {
                    0.0
                };
                let extra = room.right + caption_gap(room.right);
                let buttons = if narrow { 3.0 } else { 2.0 } + call_buttons;
                let folded = !header_fits(ui.available_width(), buttons, extra, palette.vibrant())
                    && !call_here;
                let (buttons, call_buttons) = if folded {
                    (buttons - call_buttons - 1.0, 0.0)
                } else {
                    (buttons, call_buttons)
                };
                let right_controls = controls_width(buttons, palette.vibrant()) + extra;
                let name_pill = ui.painter().add(egui::Shape::Noop);
                let mut text = Rect::NOTHING;
                let mut avatar = Rect::NOTHING;
                // Treat the avatar, name, and subtitle as one info button.
                let block = ui
                    .scope(|ui| {
                        // Fix the child height before centering its contents.
                        ui.allocate_ui_with_layout(
                            vec2(
                                (ui.available_width() - right_controls).max(80.0),
                                HEADER_ROW,
                            ),
                            Layout::left_to_right(Align::Center),
                            |ui| {
                                let avatar_response = widgets::avatar(
                                    ui,
                                    &palette,
                                    &title,
                                    &chat.id,
                                    40.0,
                                    picture.as_deref(),
                                );
                                avatar = avatar_response.rect;
                                if chat.disappearing_timer().is_some() {
                                    widgets::paint_disappearing_badge(
                                        ui,
                                        &palette,
                                        avatar_response.rect,
                                    );
                                }
                                // Room for the name's glass pill to clear the avatar.
                                ui.add_space(if palette.vibrant() { 14.0 } else { 4.0 });
                                ui.vertical(|ui| {
                                    let width = ui.available_width().max(80.0);
                                    ui.set_max_width(width);
                                    if palette.vibrant() {
                                        let column = if subtitle.is_empty() {
                                            40.0
                                        } else {
                                            NAME_GLASS_LINES
                                        };
                                        let top = avatar.center().y - column / 2.0;
                                        ui.add_space((top - ui.cursor().top()).max(0.0));
                                    }
                                    if subtitle.is_empty() {
                                        // Center the name on the avatar.
                                        ui.allocate_ui_with_layout(
                                            vec2(width, 40.0),
                                            Layout::left_to_right(Align::Center),
                                            |ui| {
                                                text = widgets::rich_text(
                                                    ui,
                                                    &title,
                                                    // Lighter on glass, as Apple's.
                                                    if palette.vibrant() {
                                                        theme::medium(15.0)
                                                    } else {
                                                        theme::semibold(17.0)
                                                    },
                                                    palette.text,
                                                )
                                                .rect;
                                            },
                                        );
                                    } else {
                                        ui.allocate_ui_with_layout(
                                            vec2(
                                                width,
                                                if palette.vibrant() {
                                                    NAME_GLASS_LINES
                                                } else {
                                                    40.0
                                                },
                                            ),
                                            Layout::top_down(Align::Min),
                                            |ui| {
                                                text = widgets::rich_text(
                                                    ui,
                                                    &title,
                                                    theme::semibold(if palette.vibrant() {
                                                        14.0
                                                    } else {
                                                        15.0
                                                    }),
                                                    palette.text,
                                                )
                                                .rect;
                                                ui.with_layout(
                                                    Layout::bottom_up(Align::Min),
                                                    |ui| {
                                                        text = text.union(
                                                            widgets::rich_text(
                                                                ui,
                                                                &subtitle,
                                                                theme::regular(
                                                                    if palette.vibrant() {
                                                                        11.5
                                                                    } else {
                                                                        12.5
                                                                    },
                                                                ),
                                                                color,
                                                            )
                                                            .rect,
                                                        );
                                                    },
                                                );
                                            },
                                        );
                                    }
                                });
                            },
                        );
                    })
                    .response;
                if palette.vibrant() && text.is_positive() {
                    // A lone name gets Apple's roomier padding.
                    let padding = vec2(
                        NAME_GLASS_PADDING.x,
                        if subtitle.is_empty() {
                            NAME_GLASS_SINGLE
                        } else {
                            NAME_GLASS_PADDING.y
                        },
                    );
                    let glass_rect = Rect::from_center_size(
                        pos2(text.center().x, avatar.center().y),
                        text.size() + 2.0 * padding,
                    );
                    ui.painter()
                        .set(name_pill, glass(&palette, glass_rect, NAME_GLASS_RADIUS));
                }
                let block = ui
                    .interact(block.rect, ui.id().with("chat-header-info"), Sense::click())
                    .on_hover_cursor(egui::CursorIcon::PointingHand);
                if block.clicked() {
                    app.actions.push(Action::OpenInfo(chat.id.clone()));
                }
                let leave_label = if chat.is_channel() {
                    crate::i18n::gettext(app.locale, "Leave channel")
                } else {
                    crate::i18n::gettext(app.locale, "Leave group")
                };
                let select_messages_label = crate::i18n::gettext(app.locale, "Select messages");
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.add_space(room.right + caption_gap(room.right));
                    if palette.vibrant() {
                        ui.add_space(BUTTON_GAP / 2.0);
                    }
                    let more_glass = ui.painter().add(egui::Shape::Noop);
                    let more = theme::icon_button(
                        ui,
                        Icon::Ellipsis,
                        18.0,
                        palette.secondary,
                        palette.text,
                        &crate::i18n::gettext(app.locale, "More"),
                    );
                    if palette.vibrant() {
                        ui.painter()
                            .set(more_glass, glass_button(&palette, more.rect));
                    }
                    let info_label = crate::i18n::gettext(app.locale, "Info");
                    let pin_label = if chat.pinned {
                        crate::i18n::gettext(app.locale, "Unpin")
                    } else {
                        crate::i18n::gettext(app.locale, "Pin to top")
                    };
                    let archive_label = if chat.archived {
                        crate::i18n::gettext(app.locale, "Unarchive")
                    } else {
                        crate::i18n::gettext(app.locale, "Archive")
                    };
                    let export_label = crate::i18n::gettext(app.locale, "Export chat…");
                    let clear_label = crate::i18n::gettext(app.locale, "Clear chat");
                    let copy_number_label = crate::i18n::gettext(app.locale, "Copy number");
                    let close_label = crate::i18n::gettext(app.locale, "Close chat");
                    let search_label = crate::i18n::gettext(app.locale, "Search messages");
                    let voice_call_label = crate::i18n::gettext(app.locale, "Voice call");
                    let video_call_label = crate::i18n::gettext(app.locale, "Video call");
                    let width = widgets::menu_width(
                        ui,
                        &[
                            search_label.as_ref(),
                            voice_call_label.as_ref(),
                            video_call_label.as_ref(),
                            info_label.as_ref(),
                            select_messages_label.as_ref(),
                            pin_label.as_ref(),
                            archive_label.as_ref(),
                            clear_label.as_ref(),
                            export_label.as_ref(),
                            leave_label.as_ref(),
                            copy_number_label.as_ref(),
                            close_label.as_ref(),
                            &super::blocking::label(app, &chat.id),
                        ],
                        true,
                    );
                    // Demo/test: hold this menu open for a screenshot.
                    #[cfg(any(test, feature = "demo"))]
                    if app.open_header_menu.as_deref() == Some(chat.id.as_str()) {
                        egui::Popup::open_id(ui.ctx(), more.id.with("popup"));
                    }
                    egui::Popup::menu(&more)
                        .width(width)
                        .frame(widgets::menu_frame(&palette))
                        .show(|ui| {
                            if folded {
                                if widgets::menu_item(
                                    ui,
                                    &palette,
                                    Some(Icon::Search),
                                    search_label.as_ref(),
                                ) {
                                    app.actions.push(if app.chat_search_open {
                                        Action::CloseChatSearch
                                    } else {
                                        Action::OpenChatSearch
                                    });
                                }
                                if callable(chat, app.me.as_deref()) {
                                    if widgets::menu_item(
                                        ui,
                                        &palette,
                                        Some(Icon::Phone),
                                        voice_call_label.as_ref(),
                                    ) {
                                        app.actions.push(Action::StartCall(chat.id.clone()));
                                    }
                                    if widgets::menu_item(
                                        ui,
                                        &palette,
                                        Some(Icon::Video),
                                        video_call_label.as_ref(),
                                    ) {
                                        app.actions.push(Action::StartVideoCall(chat.id.clone()));
                                    }
                                }
                                widgets::menu_separator(ui, &palette);
                            }
                            if widgets::menu_item(
                                ui,
                                &palette,
                                Some(Icon::Info),
                                info_label.as_ref(),
                            ) {
                                app.actions.push(Action::OpenInfo(chat.id.clone()));
                            }
                            if widgets::menu_item(
                                ui,
                                &palette,
                                Some(Icon::Check),
                                select_messages_label.as_ref(),
                            ) {
                                app.actions.push(Action::StartSelection);
                            }
                            if widgets::menu_item(
                                ui,
                                &palette,
                                Some(if chat.pinned { Icon::PinOff } else { Icon::Pin }),
                                pin_label.as_ref(),
                            ) {
                                app.actions
                                    .push(Action::SetPinned(chat.id.clone(), !chat.pinned));
                            }
                            if widgets::menu_item(
                                ui,
                                &palette,
                                Some(Icon::Archive),
                                archive_label.as_ref(),
                            ) {
                                app.actions
                                    .push(Action::SetArchived(chat.id.clone(), !chat.archived));
                            }
                            if widgets::menu_item_enabled(
                                ui,
                                &palette,
                                Some(Icon::Download),
                                export_label.as_ref(),
                                !chat.locked && app.chat_export.is_none(),
                            ) {
                                app.actions.push(Action::ShowDialog(Dialog::ExportChat {
                                    chat: chat.id.clone(),
                                    include_media: false,
                                }));
                            }
                            super::blocking::menu(app, ui, &chat.id);
                            if widgets::menu_item_enabled(
                                ui,
                                &palette,
                                Some(Icon::Eraser),
                                clear_label.as_ref(),
                                app.is_connected(),
                            ) {
                                app.actions
                                    .push(Action::ShowDialog(Dialog::ConfirmClearChat(
                                        chat.id.clone(),
                                    )));
                            }
                            widgets::menu_separator(ui, &palette);
                            if chat.can_leave(&app.our_ids())
                                && widgets::menu_item(
                                    ui,
                                    &palette,
                                    Some(Icon::LogOut),
                                    leave_label.as_ref(),
                                )
                            {
                                app.actions
                                    .push(Action::ShowDialog(Dialog::ConfirmLeaveGroup(
                                        chat.id.clone(),
                                    )));
                            }
                            if let Some(phone) = chat.phone()
                                && widgets::menu_item(
                                    ui,
                                    &palette,
                                    Some(Icon::Copy),
                                    copy_number_label.as_ref(),
                                )
                            {
                                app.actions.push(Action::CopyText(format!("+{phone}")));
                            }
                            if widgets::menu_item(ui, &palette, Some(Icon::X), close_label.as_ref())
                            {
                                app.actions.push(Action::CloseChat);
                            }
                        });
                    if call_buttons > 0.0 {
                        let (call_tooltip, call_icon, call_fill, call) = if call_here {
                            (
                                crate::i18n::gettext(app.locale, "Hang up").into_owned(),
                                Icon::PhoneOff,
                                palette.danger,
                                Action::HangupCall,
                            )
                        } else {
                            (
                                crate::i18n::gettext(app.locale, "Voice call").into_owned(),
                                Icon::Phone,
                                palette.secondary,
                                Action::StartCall(chat.id.clone()),
                            )
                        };
                        if palette.vibrant() {
                            ui.add_space(BUTTON_GAP);
                        }
                        let call_glass = ui.painter().add(egui::Shape::Noop);
                        let call_button = theme::icon_button(
                            ui,
                            call_icon,
                            18.0,
                            call_fill,
                            palette.text,
                            &call_tooltip,
                        );
                        if palette.vibrant() {
                            ui.painter()
                                .set(call_glass, glass_button(&palette, call_button.rect));
                        }
                        if call_button.clicked() {
                            app.actions.push(call);
                        }
                        if !call_here {
                            let tip = crate::i18n::gettext(app.locale, "Video call");
                            if palette.vibrant() {
                                ui.add_space(BUTTON_GAP);
                            }
                            let video_glass = ui.painter().add(egui::Shape::Noop);
                            let video = theme::icon_button(
                                ui,
                                Icon::Video,
                                18.0,
                                palette.secondary,
                                palette.text,
                                &tip,
                            );
                            if palette.vibrant() {
                                ui.painter()
                                    .set(video_glass, glass_button(&palette, video.rect));
                            }
                            if video.clicked() {
                                app.actions.push(Action::StartVideoCall(chat.id.clone()));
                            }
                        }
                    }
                    let searching = app.chat_search_open;
                    if !folded {
                        let tip = format!(
                            "{} ({})",
                            crate::i18n::gettext(app.locale, "Search messages"),
                            super::keys::label("Ctrl+F")
                        );
                        if palette.vibrant() {
                            ui.add_space(BUTTON_GAP);
                        }
                        let search_glass = ui.painter().add(egui::Shape::Noop);
                        let search = theme::icon_button(
                            ui,
                            Icon::Search,
                            18.0,
                            if searching {
                                palette.accent
                            } else {
                                palette.secondary
                            },
                            palette.text,
                            &tip,
                        )
                        .tab_stop(Stop::ChatSearch);
                        if palette.vibrant() {
                            ui.painter()
                                .set(search_glass, glass_button(&palette, search.rect));
                        }
                        if search.clicked() {
                            app.actions.push(if searching {
                                Action::CloseChatSearch
                            } else {
                                Action::OpenChatSearch
                            });
                        }
                    }
                    if narrow {
                        if palette.vibrant() {
                            ui.add_space(BUTTON_GAP);
                        }
                        let back_glass = ui.painter().add(egui::Shape::Noop);
                        let back_label = if app.page == crate::model::Page::Calls {
                            crate::i18n::gettext(app.locale, "Back to the calls").into_owned()
                        } else {
                            "Back to chats".to_owned()
                        };
                        let back = theme::icon_button(
                            ui,
                            Icon::ArrowLeft,
                            18.0,
                            palette.secondary,
                            palette.text,
                            &back_label,
                        )
                        .tab_stop(Stop::Back);
                        if palette.vibrant() {
                            ui.painter()
                                .set(back_glass, glass_button(&palette, back.rect));
                        }
                        ui.ctx()
                            .data_mut(|data| data.insert_temp(back_button_id(), back.rect));
                        if back.clicked() {
                            app.actions.push(Action::CloseChat);
                        }
                    }
                });
            });
            ui.ctx()
                .data_mut(|data| data.insert_temp(header_row_id(), row.response.rect));
        })
        .response
        .rect;
    // The info panel beside the chat ends its header where this one ends.
    ui.ctx()
        .data_mut(|data| data.insert_temp(header_rect_id(), rect));
    rect
}

/// Where the chat header was last laid out, for the panes beside it.
pub(crate) fn header_rect_id() -> egui::Id {
    egui::Id::new("conversation-header")
}

/// Where a narrow window's Back was laid out, for interaction tests.
pub(crate) fn back_button_id() -> egui::Id {
    egui::Id::new("conversation-back")
}

/// Where the conversation header's row was laid out, for layout tests.
pub(crate) fn header_row_id() -> egui::Id {
    egui::Id::new("conversation-header-row")
}

/// Chat-header subtitle.
pub(crate) fn subtitle(app: &App, chat: &Chat) -> (String, Color32) {
    let palette = app.palette;
    let typing = app.typing_in(&chat.id);
    if !typing.is_empty() {
        let activity = typing[0].2;
        let text = if chat.is_group() {
            let describe = |activity| {
                let names: Vec<&str> = typing
                    .iter()
                    .filter(|(_, _, other)| *other == activity)
                    .map(|(_, name, _)| name.as_str())
                    .collect();
                match (activity, names.as_slice()) {
                    (_, []) => None,
                    (ChatActivity::RecordingAudio, [one]) => Some(
                        crate::i18n::gettext(app.locale, "{name} is recording audio…")
                            .replace("{name}", one),
                    ),
                    (ChatActivity::RecordingAudio, [rest @ .., last]) => Some(
                        crate::i18n::gettext(app.locale, "{names} and {last} are recording audio…")
                            .replace("{names}", &rest.join(", "))
                            .replace("{last}", last),
                    ),
                    (_, [one]) => Some(format!("{one} is typing…")),
                    (_, [rest @ .., last]) => {
                        Some(format!("{} and {last} are typing…", rest.join(", ")))
                    }
                }
            };
            [ChatActivity::Typing, ChatActivity::RecordingAudio]
                .into_iter()
                .filter_map(describe)
                .collect::<Vec<_>>()
                .join(" · ")
        } else if activity == ChatActivity::RecordingAudio {
            crate::i18n::gettext(app.locale, "recording audio…").into_owned()
        } else {
            "typing…".to_owned()
        };
        return (text, palette.accent);
    }
    if chat.is_group() {
        let names = app.participant_names(chat);
        return (
            if names.is_empty() {
                "Group".to_owned()
            } else {
                names
            },
            palette.secondary,
        );
    }
    if let Some(presence) = app.presence.get(&chat.id) {
        if presence.online {
            return ("online".to_owned(), palette.accent);
        }
        if let Some(seen) = presence.last_seen {
            return (crate::util::last_seen(app.locale, seen), palette.secondary);
        }
    }
    match chat.phone() {
        Some(phone) if !app.is_saved_contact(&chat.id) => {
            (crate::util::phone(phone), palette.secondary)
        }
        _ => (String::new(), palette.secondary),
    }
}
