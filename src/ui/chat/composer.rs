use super::*;

pub(crate) const COMPOSER_RADIUS: u8 = 20;
pub(crate) const COMPOSER_PADDING: f32 = 8.0;
pub(crate) const COMPOSER_CONTROL: f32 = 36.0;
pub(crate) const COMPOSER_INSET: i8 = 2;

pub(crate) fn composer_id() -> egui::Id {
    egui::Id::new("composer-text")
}

pub(crate) fn composer_text_id() -> egui::Id {
    egui::Id::new("composer-text-rect")
}

pub(crate) fn composer_pill_id() -> egui::Id {
    egui::Id::new("composer-pill")
}

pub(crate) fn scroll_metrics_id(chat: &ChatId) -> egui::Id {
    egui::Id::new(("message-scroll-metrics", chat))
}

pub(crate) fn recording_wave_id() -> egui::Id {
    egui::Id::new("recording-wave")
}

pub(crate) fn last_line<R>(ui: &mut egui::Ui, line: f32, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.allocate_ui_with_layout(vec2(0.0, line), Layout::left_to_right(Align::Center), add)
        .inner
}

pub(crate) fn reviewing_attachments(app: &App, chat: &Chat) -> bool {
    !app.pending.is_empty()
        && chat.can_send()
        && app.selection.is_none()
        && app.editing.is_none()
        && app.recording.is_none()
}

pub(crate) fn attachment_review(app: &mut App, ui: &mut egui::Ui, chat: &Chat) -> Option<bool> {
    reviewing_attachments(app, chat).then(|| super::attachment_preview::bar(app, ui))
}

/// Top-level bottom panel for composing messages, handling read-only states, strips, and send dispatch.
pub struct Composer;

impl Composer {
    pub fn show(app: &mut App, ui: &mut egui::Ui, chat: &Chat, review_send: Option<bool>) {
        let palette = app.palette;
        let shown = egui::Panel::bottom("composer")
            .show_separator_line(false)
            .frame(
                Frame::new()
                    .fill(Color32::TRANSPARENT)
                    .inner_margin(Margin {
                        left: 16,
                        right: 16,
                        top: 0,
                        bottom: 10,
                    }),
            )
            .show(ui, |ui| {
                ComposerStrips::uploads(app, ui, &chat.id);
                if let Some((selected_chat, selected)) = app.selection.clone()
                    && selected_chat == chat.id
                {
                    SelectionBar::show(app, ui, &chat.id, &selected);
                    return;
                }
                if app.blocklist.contains(&chat.id) {
                    if app.recording.is_some() {
                        widgets::raised(ui, &palette, ComposerPill::frame(&palette), |ui| {
                            VoiceRecorderBar::show(app, ui);
                        });
                    }
                    ui.vertical_centered(|ui| {
                        theme::text(
                            ui,
                            crate::i18n::gettext(app.locale, "You blocked this contact"),
                            theme::regular(13.5),
                            palette.secondary,
                        );
                        super::blocking::button(app, ui, &chat.id, Stop::Composer);
                    });
                    return;
                }
                if !chat.can_send() {
                    Self::show_read_only_notice(app, ui, chat, &palette);
                    return;
                }

                let unsent_voice = app
                    .unsent_voice
                    .as_ref()
                    .filter(|(unsent, _)| *unsent == chat.id)
                    .map(|(_, samples)| samples.len());
                if let Some(samples) = unsent_voice
                    && app.editing.is_none()
                {
                    ComposerStrips::unsent_voice(app, ui, samples);
                }
                if app.editing.is_some() {
                    ComposerStrips::edit(app, ui);
                }

                let reply_drawer = ReplyDrawer::prepare(app, ui, chat);
                let is_animating_reply =
                    reply_drawer.progress > 0.0 && reply_drawer.progress < 1.0;
                ui.ctx().data_mut(|data| {
                    data.insert_temp(egui::Id::new("reply-drawer-animating"), is_animating_reply)
                });

                if app.pending.is_empty() && app.recording.is_none() {
                    ComposerStrips::link_preview(app, ui);
                }

                app.prepare_composer_undo(ui.ctx());
                let id = composer_id();
                let has_focus = ui.memory(|memory| memory.has_focus(id));
                let enter_sends = app.settings.enter_sends;
                let (typed_colon, typed_at) = ui.input(|input| {
                    let typed = |needle: &str| {
                        input
                            .events
                            .iter()
                            .any(|event| matches!(event, egui::Event::Text(text) if text == needle))
                    };
                    (has_focus && typed(":"), has_focus && typed("@"))
                });

                if app.recording.is_some() {
                    widgets::raised(ui, &palette, ComposerPill::frame(&palette), |ui| {
                        VoiceRecorderBar::show(app, ui);
                    });
                    return;
                }

                EmojiSuggestions::show(app, ui, id);
                MentionPicker::show(app, ui, chat, id);

                let send_key = has_focus
                    && ui.input_mut(|input| {
                        let mut sent = false;
                        input.events.retain(|event| {
                            if sent {
                                return true;
                            }
                            let is_send = matches!(
                                event,
                                egui::Event::Key {
                                    key: Key::Enter,
                                    pressed: true,
                                    modifiers,
                                    ..
                                } if !modifiers.shift && !modifiers.alt
                                    && (if enter_sends { !modifiers.command && !modifiers.ctrl } else { modifiers.command })
                            );
                            sent |= is_send;
                            !is_send
                        });
                        sent
                    });

                let mut send_click = review_send.unwrap_or(false);
                let pill_rect = ComposerPill::show(
                    app,
                    ui,
                    chat,
                    &reply_drawer,
                    review_send,
                    unsent_voice,
                    enter_sends,
                    typed_colon,
                    typed_at,
                    &mut send_click,
                );
                ui.ctx()
                    .data_mut(|data| data.insert_temp(composer_pill_id(), pill_rect));

                if (send_key || send_click)
                    && (!app.composer.trim().is_empty() || !app.pending.is_empty())
                {
                    let text = std::mem::take(&mut app.composer);
                    if app.pending.is_empty() {
                        app.actions.push(Action::SendText {
                            chat: chat.id.clone(),
                            text,
                            quoting: app.reply_to.clone(),
                        });
                    } else {
                        app.actions.push(Action::SendPending {
                            account: app.account().id.clone(),
                            mentions: app.composer_mentions(),
                            chat: chat.id.clone(),
                            caption: text,
                        });
                    }
                    app.focus_composer = true;
                } else if (send_key || send_click)
                    && unsent_voice.is_some()
                    && app.editing.is_none()
                {
                    app.actions.push(Action::SendRecording);
                    app.focus_composer = true;
                }
            });

        let is_animating = ui
            .ctx()
            .data(|data| data.get_temp::<bool>(egui::Id::new("reply-drawer-animating")))
            .unwrap_or(false);
        let height = shown.response.rect.height();
        let height_id = egui::Id::new("composer-panel-height");
        let previous = ui.ctx().data_mut(|data| {
            let previous = data.get_temp::<f32>(height_id);
            data.insert_temp(height_id, height);
            previous
        });
        if previous.is_some_and(|previous| (previous - height).abs() > 0.5) && !is_animating {
            ui.ctx().request_discard("composer height changed");
        }
        ui.ctx()
            .data_mut(|data| data.insert_temp(super::composer_rect_id(), shown.response.rect));
    }

    fn show_read_only_notice(app: &App, ui: &mut egui::Ui, chat: &Chat, palette: &Palette) {
        if chat.kind == crate::model::ChatKind::Broadcast {
            ui.vertical_centered(|ui| {
                theme::text(
                    ui,
                    if chat.left {
                        crate::i18n::gettext(app.locale, "You left this channel")
                    } else {
                        crate::i18n::gettext(app.locale, "Channels are read-only in ZapFast")
                    }
                    .as_ref(),
                    theme::regular(13.5),
                    palette.secondary,
                );
            });
            return;
        }
        ui.vertical_centered(|ui| {
            ui.add_space(8.0);
            let ours = app.our_ids();
            let left = chat.left
                || (!ours.is_empty() && !chat.participants.is_empty() && !chat.lists_any(&ours));
            if left {
                theme::text(
                    ui,
                    crate::i18n::gettext(app.locale, "You left this group"),
                    theme::regular(13.5),
                    palette.secondary,
                );
            } else {
                ui.horizontal(|ui| {
                    let width = 230.0;
                    ui.add_space((ui.available_width() - width).max(0.0) * 0.5);
                    theme::text(ui, "Only", theme::regular(13.5), palette.secondary);
                    theme::text(ui, "admins", theme::semibold(13.5), palette.accent);
                    theme::text(ui, "can send messages", theme::regular(13.5), palette.secondary);
                });
            }
            ui.add_space(8.0);
        });
    }
}

/// Animated reply preview drawer that expands the top of `ComposerPill` upwards without moving the input row.
pub struct ReplyDrawer {
    pub quoted: Option<Message>,
    pub progress: f32,
}

impl ReplyDrawer {
    pub const TARGET_HEIGHT: f32 = 46.0;
    pub const ANIM_DURATION: f32 = 0.22;

    pub fn prepare(app: &mut App, ui: &egui::Ui, chat: &Chat) -> Self {
        let reply_quoted = if app.editing.is_some() {
            None
        } else if let Some(reply_id) = app.reply_to.clone() {
            let quoted = app.reply_preview(&reply_id, &chat.id).cloned();
            if quoted.is_none() && reply_id.chat.is_none() {
                app.reply_to = None;
            }
            quoted
        } else {
            None
        };

        let anim_id = egui::Id::new("composer-reply-tween");
        let is_open = app.reply_to.is_some() && app.editing.is_none();
        let raw_progress = ui
            .ctx()
            .animate_bool_with_time(anim_id, is_open, Self::ANIM_DURATION);
        let progress = if is_open {
            egui::emath::easing::cubic_out(raw_progress)
        } else {
            egui::emath::easing::cubic_in(raw_progress)
        };
        let cache_id = anim_id.with("cached-quoted");

        let quoted = if let Some(msg) = reply_quoted {
            ui.ctx().data_mut(|d| d.insert_temp(cache_id, msg.clone()));
            Some(msg)
        } else if progress > 0.0 {
            ui.ctx().data(|d| d.get_temp::<Message>(cache_id))
        } else {
            None
        };

        Self { quoted, progress }
    }

    pub fn current_height(&self) -> f32 {
        if self.quoted.is_some() {
            Self::TARGET_HEIGHT * self.progress
        } else {
            0.0
        }
    }

    pub fn show(&self, app: &mut App, ui: &mut egui::Ui, palette: &Palette, width: f32) {
        let extra_height = self.current_height();
        let Some(quoted) = &self.quoted else {
            return;
        };
        if extra_height <= 0.0 {
            return;
        }

        let (_, section_rect) = ui.allocate_space(vec2(width, extra_height));
        ui.ctx()
            .data_mut(|data| data.insert_temp(reply_strip_id(), section_rect));

        let clip = section_rect.intersect(ui.clip_rect());
        if !clip.is_positive() {
            return;
        }

        let content_rect = Rect::from_min_size(
            pos2(section_rect.left(), section_rect.bottom() - Self::TARGET_HEIGHT),
            vec2(width, Self::TARGET_HEIGHT),
        );

        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(content_rect)
                .layout(Layout::top_down(Align::Min)),
        );
        child.set_clip_rect(clip);
        let opacity = egui::emath::easing::cubic_out(self.progress);
        child.multiply_opacity(opacity);

        let card_margin_x = 4.0;
        let card_rect = Rect::from_min_max(
            pos2(content_rect.left() + card_margin_x, content_rect.top() + 3.0),
            pos2(content_rect.right() - card_margin_x, content_rect.bottom() - 3.0),
        );
        let card_fill = if palette.dark {
            Color32::from_rgba_unmultiplied(0, 0, 0, 80)
        } else {
            Color32::from_rgba_unmultiplied(0, 0, 0, 18)
        };
        child.painter().rect_filled(
            card_rect,
            CornerRadius::same(crate::ui::chat::QUOTE_RADIUS),
            card_fill,
        );

        let bar_width = 4.0;
        let bar_inset_x = 6.0;
        let bar_inset_y = 5.0;
        let bar_rect = Rect::from_min_max(
            pos2(card_rect.left() + bar_inset_x, card_rect.top() + bar_inset_y),
            pos2(card_rect.left() + bar_inset_x + bar_width, card_rect.bottom() - bar_inset_y),
        );
        let sender_color = if quoted.from_me {
            palette.accent
        } else {
            palette.sender(crate::util::hue(&quoted.sender))
        };
        child.painter().rect_filled(
            bar_rect,
            CornerRadius::same(2),
            sender_color,
        );

        let mut card_ui = child.new_child(
            egui::UiBuilder::new()
                .max_rect(card_rect)
                .layout(Layout::left_to_right(Align::Center)),
        );
        card_ui.add_space(bar_inset_x + bar_width + 8.0);
        card_ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            ui.set_max_width((ui.available_width() - 36.0).max(0.0));
            let who = if quoted.from_me {
                crate::i18n::gettext(app.locale, "You").into_owned()
            } else {
                app.display_name_or(&quoted.sender, quoted.sender_name.as_deref())
            };
            let summary = markup::plain(&quoted.summary_in(app.locale), &app.mention_list(quoted));
            widgets::rich_text(ui, &who, theme::semibold(12.5), sender_color);
            widgets::rich_text(ui, &summary, theme::regular(12.0), palette.secondary);
        });
        card_ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.add_space(6.0);
            if theme::icon_button(
                ui,
                Icon::X,
                16.0,
                palette.secondary,
                palette.text,
                "Cancel reply (Esc)",
            )
            .clicked()
            {
                app.actions.push(Action::CancelReply);
            }
        });
    }
}

pub struct ComposerPill;

impl ComposerPill {
    pub fn frame(palette: &Palette) -> Frame {
        let (fill, edge) = if palette.vibrant() {
            (palette.glass(), palette.glass_edge())
        } else {
            (palette.bubble_in, Stroke::NONE)
        };
        Frame::new()
            .fill(fill)
            .stroke(edge)
            .corner_radius(CornerRadius::same(COMPOSER_RADIUS))
            .inner_margin(Margin {
                left: COMPOSER_INSET,
                right: COMPOSER_INSET,
                top: COMPOSER_INSET,
                bottom: COMPOSER_INSET,
            })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn show(
        app: &mut App,
        ui: &mut egui::Ui,
        chat: &Chat,
        reply_drawer: &ReplyDrawer,
        review_send: Option<bool>,
        unsent_voice: Option<usize>,
        enter_sends: bool,
        typed_colon: bool,
        typed_at: bool,
        send_click: &mut bool,
    ) -> Rect {
        let palette = app.palette;
        let id = composer_id();
        let line_height = ui
            .painter()
            .layout_no_wrap("x".to_owned(), theme::regular(BODY_SIZE), palette.text)
            .size()
            .y;
        let line = (line_height + COMPOSER_PADDING)
            .round()
            .max(COMPOSER_CONTROL);
        let field_margin = ((line - line_height) * 0.5).round().max(4.0);
        let wrap_id = id.with("wrap");
        let mut format = egui::TextFormat::simple(theme::regular(BODY_SIZE), palette.text);
        format.line_height = Some(20.0);
        let (measured_height, rows) = ui
            .ctx()
            .data(|data| data.get_temp::<f32>(wrap_id))
            .map(|wrap| {
                let galley =
                    crate::bidi::layout_editor(ui, &app.composer, &format, wrap, true).0;
                let rows = if app.composer.is_empty() {
                    1
                } else {
                    galley.rows.len().max(1)
                };
                let height = if app.composer.is_empty() {
                    line_height
                } else {
                    galley.size().y.max(line_height)
                };
                (height, rows)
            })
            .unwrap_or((line_height, 1));
        let max_composer_height = line_height * 5.0;
        let text_height = if rows > 5 {
            max_composer_height
        } else {
            measured_height.clamp(line_height, max_composer_height)
        };
        let row_height = (text_height + 2.0 * field_margin).max(line);

        let pill = widgets::raised(ui, &palette, Self::frame(&palette), |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let pill_width = ui.available_width();
            reply_drawer.show(app, ui, &palette, pill_width);

            let (_, row_rect) = ui.allocate_space(vec2(pill_width, row_height));
            let mut row_ui = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(row_rect)
                    .layout(Layout::left_to_right(Align::Max)),
            );
            ComposerInputRow::show(
                app,
                &mut row_ui,
                chat,
                line_height,
                line,
                field_margin,
                text_height,
                rows,
                review_send,
                unsent_voice,
                enter_sends,
                typed_colon,
                typed_at,
                send_click,
            );
        });
        pill.response.rect
    }
}

pub struct ComposerInputRow;

impl ComposerInputRow {
    #[allow(clippy::too_many_arguments)]
    pub fn show(
        app: &mut App,
        ui: &mut egui::Ui,
        chat: &Chat,
        line_height: f32,
        line: f32,
        field_margin: f32,
        text_height: f32,
        rows: usize,
        review_send: Option<bool>,
        unsent_voice: Option<usize>,
        enter_sends: bool,
        typed_colon: bool,
        typed_at: bool,
        send_click: &mut bool,
    ) {
        let palette = app.palette;
        let id = composer_id();
        let wrap_id = id.with("wrap");
        let button_width = if review_send.is_some() { 0.0 } else { line };
        let view_once = app.editing.is_none()
            && (app.pending_supports_view_once() || app.pending_view_once);
        let view_once_width = if view_once {
            COMPOSER_CONTROL + ui.spacing().item_spacing.x
        } else {
            0.0
        };

        ui.add_space(6.0);
        if app.editing.is_none() {
            let tools = last_line(ui, line, |ui| {
                theme::icon_button(
                    ui,
                    Icon::Plus,
                    20.0,
                    if app.composer_tools_open {
                        palette.accent
                    } else {
                        palette.secondary
                    },
                    palette.text,
                    &crate::i18n::gettext(app.locale, "Attach"),
                )
            })
            .tab_stop(Stop::Attach);
            Self::tools_menu(app, chat, &tools);

            ui.add_space(2.0);
            let smile = last_line(ui, line, |ui| {
                theme::icon_button(
                    ui,
                    Icon::Smile,
                    20.0,
                    if app.picker.is_some() {
                        palette.accent
                    } else {
                        palette.secondary
                    },
                    palette.text,
                    "Emoji, GIFs, and stickers",
                )
            })
            .tab_stop(Stop::Emoji);
            app.picker_anchor = Some(smile.rect);
            if smile.clicked() {
                if app.composer_tools_open {
                    app.actions.push(Action::SetComposerTools(false));
                }
                app.actions.push(Action::TogglePicker(PickerTab::Emoji));
            }
            ui.add_space(4.0);
        }

        let field_width =
            (ui.available_width() - button_width - view_once_width - 6.0).max(60.0);
        Frame::new()
            .fill(Color32::TRANSPARENT)
            .inner_margin(Margin {
                left: 4,
                right: 4,
                top: field_margin as i8,
                bottom: field_margin as i8,
            })
            .show(ui, |ui| {
                ui.set_width((field_width - 8.0).max(0.0));
                let scroll_visibility = if rows > 5 {
                    egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded
                } else {
                    egui::scroll_area::ScrollBarVisibility::AlwaysHidden
                };
                egui::ScrollArea::vertical()
                    .id_salt("composer-scroll")
                    .min_scrolled_height(text_height)
                    .max_height(text_height)
                    .scroll_bar_visibility(scroll_visibility)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let mut clusters: Vec<(usize, usize, String)> = Vec::new();
                        let mut format =
                            egui::TextFormat::simple(theme::regular(BODY_SIZE), palette.text);
                        format.line_height = Some(20.0);
                        let composer_rtl = crate::bidi::base_rtl(&app.composer);
                        let mut layouter =
                            |ui: &egui::Ui, text: &dyn egui::TextBuffer, wrap: f32| {
                                let (galley, found) = crate::bidi::layout_editor(
                                    ui,
                                    text.as_str(),
                                    &format,
                                    wrap,
                                    true,
                                );
                                clusters = found;
                                galley
                            };
                        let wrap = ui.available_width();
                        ui.ctx().data_mut(|data| data.insert_temp(wrap_id, wrap));
                        let selection_before = egui::TextEdit::load_state(ui.ctx(), id)
                            .and_then(|state| state.cursor.char_range())
                            .filter(|range| !range.is_empty());
                        let mut output = egui::TextEdit::multiline(&mut app.composer)
                            .id(id)
                            .frame(Frame::NONE)
                            .margin(Margin::ZERO)
                            .hint_text(
                                egui::RichText::new(if app.pending.is_empty() {
                                    crate::i18n::gettext(app.locale, "Type a message")
                                        .into_owned()
                                } else {
                                    crate::i18n::gettext(app.locale, "Add a caption").into_owned()
                                })
                                .color(palette.dim)
                                .font(theme::regular(BODY_SIZE)),
                            )
                            .font(theme::regular(BODY_SIZE))
                            .text_color(palette.text)
                            .desired_rows(1)
                            .desired_width(f32::INFINITY)
                            .horizontal_align(if composer_rtl {
                                Align::RIGHT
                            } else {
                                Align::LEFT
                            })
                            .return_key(if enter_sends {
                                Some(KeyboardShortcut::new(Modifiers::SHIFT, Key::Enter))
                            } else {
                                Some(KeyboardShortcut::new(Modifiers::NONE, Key::Enter))
                            })
                            .layouter(&mut layouter)
                            .show(ui);

                        if (egui::Popup::is_id_open(ui.ctx(), id.with("popup"))
                            || (ui.input(|input| {
                                input.pointer.button_pressed(egui::PointerButton::Secondary)
                            }) && output.response.response.contains_pointer()))
                            && let Some(range) = selection_before
                        {
                            output.state.cursor.set_char_range(Some(range));
                            output.cursor_range = Some(range);
                            egui::TextEdit::store_state(ui.ctx(), id, output.state.clone());
                        }
                        for (start, length, cluster) in &clusters {
                            let Some(bounds) = crate::bidi::char_bounds(
                                &output.galley,
                                *start,
                                start + length,
                            ) else {
                                continue;
                            };
                            if bounds.height() > line_height * 1.5 {
                                continue;
                            }
                            let rect = bounds.translate(output.galley_pos.to_vec2());
                            crate::emoji::paint_cluster(ui, cluster, rect);
                        }

                        let painted =
                            Rect::from_min_size(output.galley_pos, output.galley.size());
                        let galley = crate::bidi::layout_editor(
                            ui,
                            &app.composer,
                            &format,
                            wrap,
                            true,
                        )
                        .0;
                        let after_height = if app.composer.is_empty() {
                            line_height
                        } else {
                            galley.size().y.max(line_height)
                        };
                        let measured = after_height.clamp(line_height, line_height * 5.0);
                        ui.ctx().data_mut(|data| {
                            data.insert_temp(composer_text_id(), painted);
                        });
                        if (measured - text_height).abs() > 0.5 {
                            ui.ctx().request_discard("composer height changed");
                        }

                        let response = output.response.response.clone().tab_stop(Stop::Composer);
                        ui.ctx()
                            .accesskit_node_builder(response.id, |node| node.set_label("Message"));
                        Self::format_context_menu(app, ui, &response, &output);
                        Self::handle_text_changes(
                            app,
                            chat,
                            &response,
                            &output,
                            typed_colon,
                            typed_at,
                        );

                        if app.focus_composer
                            && app.image_preview.is_none()
                            && app.sticker_view.is_none()
                        {
                            app.focus_composer = false;
                            response.request_focus();
                        }
                    });
            });

        if view_once
            && last_line(ui, line, |ui| Self::view_once_button(app, ui))
                .tab_stop(Stop::ViewOnce)
                .clicked()
        {
            app.actions
                .push(Action::SetPendingViewOnce(!app.pending_view_once));
        }

        let ready = !app.composer.trim().is_empty()
            || !app.pending.is_empty()
            || (unsent_voice.is_some() && app.editing.is_none());
        if review_send.is_none() {
            last_line(ui, line, |ui| {
                if !ready && app.editing.is_none() {
                    if theme::icon_button(
                        ui,
                        Icon::Mic,
                        20.0,
                        palette.secondary,
                        palette.text,
                        "Record a voice message",
                    )
                    .tab_stop(Stop::Send)
                    .clicked()
                    {
                        app.actions.push(Action::StartRecording);
                    }
                } else {
                    let icon_kind = if app.editing.is_some() {
                        Icon::Check
                    } else {
                        Icon::Send
                    };
                    if theme::circle_button(
                        ui,
                        icon_kind,
                        button_width - 4.0,
                        palette.accent,
                        palette.accent_hover,
                        palette.on_accent,
                        "Send",
                    )
                    .tab_stop(Stop::Send)
                    .clicked()
                    {
                        *send_click = true;
                    }
                }
            });
        }
        ui.add_space(2.0);
    }

    fn format_context_menu(
        app: &mut App,
        ui: &mut egui::Ui,
        response: &egui::Response,
        output: &egui::text_edit::TextEditOutput,
    ) {
        let keyboard_menu = response.has_focus()
            && ui.input_mut(|input| super::keys::take_exact(input, Modifiers::SHIFT, Key::F10));
        let menu_id = egui::Popup::default_response_id(response);
        let focus_pending_id = menu_id.with("focus-pending");
        if keyboard_menu {
            egui::Popup::open_id(ui.ctx(), menu_id);
            ui.ctx()
                .data_mut(|data| data.insert_temp(focus_pending_id, true));
        }
        let mut menu = egui::Popup::context_menu(response);
        if !response.secondary_clicked() && egui::Popup::position_of_id(ui.ctx(), menu_id).is_none()
        {
            menu = menu.at_position(response.rect.left_bottom());
        }
        let mut first_item = None;
        let shown_menu = menu.show(|ui| {
            use crate::model::ComposerFormat;
            let range = output
                .state
                .cursor
                .char_range()
                .map(|range| range.as_sorted_char_range());
            let selected = range.as_ref().is_some_and(|range| !range.is_empty());
            ui.add_enabled_ui(selected, |ui| {
                for (format, label) in [
                    (
                        ComposerFormat::Bold,
                        crate::i18n::gettext(app.locale, "Bold"),
                    ),
                    (
                        ComposerFormat::Italic,
                        crate::i18n::gettext(app.locale, "Italic"),
                    ),
                    (
                        ComposerFormat::Strikethrough,
                        crate::i18n::gettext(app.locale, "Strikethrough"),
                    ),
                    (
                        ComposerFormat::Monospace,
                        crate::i18n::gettext(app.locale, "Monospace"),
                    ),
                ] {
                    let button = ui.button(label);
                    if format == ComposerFormat::Bold && ui.is_visible() {
                        first_item = Some(button.id);
                    }
                    if button.clicked() {
                        if let Some(range) = &range {
                            app.actions.push(Action::FormatComposer {
                                format,
                                start: range.start.0,
                                end: range.end.0,
                            });
                        }
                        ui.close();
                    }
                }
            });
        });
        let focus_pending = ui.ctx().data(|data| data.get_temp::<bool>(focus_pending_id));
        if let Some(menu) = shown_menu {
            if focus_pending.is_some() {
                ui.memory_mut(|memory| memory.set_modal_layer(menu.response.layer_id));
            }
            if focus_pending == Some(true)
                && let Some(first) = first_item
            {
                ui.memory_mut(|memory| memory.request_focus(first));
                ui.ctx()
                    .data_mut(|data| data.insert_temp(focus_pending_id, false));
            }
        }
        if !egui::Popup::is_id_open(ui.ctx(), menu_id) && focus_pending.is_some() {
            ui.ctx()
                .data_mut(|data| data.remove::<bool>(focus_pending_id));
            if !ui.input(|input| input.pointer.any_pressed() || input.pointer.any_click()) {
                response.request_focus();
            }
        }
    }

    fn handle_text_changes(
        app: &mut App,
        chat: &Chat,
        response: &egui::Response,
        output: &egui::text_edit::TextEditOutput,
        typed_colon: bool,
        typed_at: bool,
    ) {
        if response.changed() {
            app.actions.push(Action::Composing {
                chat: chat.id.clone(),
                composing: true,
            });
            let cursor = output
                .cursor_range
                .map(|range| range.primary.index.0)
                .unwrap_or_else(|| app.composer.chars().count());
            if typed_colon && let Some(at) = standalone_trigger(&app.composer, cursor, ':') {
                app.picker = None;
                app.emoji_start = Some(at);
                app.emoji_selected = 0;
                app.mention_start = None;
            } else if typed_at
                && chat.is_group()
                && !chat.participants.is_empty()
                && let Some(at) = standalone_trigger(&app.composer, cursor, '@')
            {
                app.emoji_start = None;
                app.mention_start = Some(at);
                app.mention_selected = 0;
            } else if app.emoji_start.is_some() {
                app.emoji_selected = 0;
            }
        }
        let cursor = output
            .cursor_range
            .map(|range| range.primary.index.0)
            .unwrap_or_else(|| app.composer.chars().count());
        if app.emoji_start.is_some()
            && active_emoji(&app.composer, app.emoji_start, cursor).is_none()
        {
            app.emoji_start = None;
        }
        if app.mention_start.is_some()
            && active_mention(&app.composer, app.mention_start, cursor).is_none()
        {
            app.mention_start = None;
        }
    }

    pub fn tools_menu(app: &mut App, chat: &Chat, plus: &egui::Response) {
        let id = plus.id.with("composer-tools");
        let mut open = app.composer_tools_open;
        let picker_open = app.picker.is_some();
        if plus.clicked() {
            open = !open;
            if open && picker_open {
                app.actions.push(Action::ClosePicker);
            }
        }
        let anim_id = id.with("open-time");
        let mut draw_open = open;
        if draw_open && !picker_open {
            egui::Popup::menu(plus)
                .id(id)
                .open_bool(&mut draw_open)
                .close_behavior(egui::PopupCloseBehavior::CloseOnClick)
                .width(205.0)
                .frame(widgets::menu_frame(&app.palette))
                .show(|ui| {
                    widgets::show_animated_menu(ui, anim_id, false, |ui, stretch| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        let item_height = 38.0 * stretch;
                        let locale = app.locale;
                        let palette = app.palette;

                        let document = crate::i18n::gettext(locale, "Document");
                        if widgets::menu_item_icon_colored(
                            ui,
                            &palette,
                            Icon::FileText,
                            Color32::from_rgb(0x7f, 0x66, 0xff),
                            &document,
                            item_height,
                        ) {
                            app.actions.push(Action::Attach { documents: true });
                        }

                        let photos = if locale == crate::i18n::Locale::Spanish {
                            "Fotos y videos".to_string()
                        } else {
                            crate::i18n::gettext(locale, "Photos & videos").into_owned()
                        };
                        if widgets::menu_item_icon_colored(
                            ui,
                            &palette,
                            Icon::Image,
                            Color32::from_rgb(0x00, 0x7b, 0xfc),
                            &photos,
                            item_height,
                        ) {
                            app.actions.push(Action::Attach { documents: false });
                        }

                        let camera = crate::i18n::gettext(locale, "Camera");
                        if widgets::menu_item_icon_colored(
                            ui,
                            &palette,
                            Icon::Video,
                            Color32::from_rgb(0xeb, 0x12, 0x60),
                            &camera,
                            item_height,
                        ) {
                            app.actions.push(Action::Attach { documents: false });
                        }

                        let audio = crate::i18n::gettext(locale, "Audio");
                        if widgets::menu_item_icon_colored(
                            ui,
                            &palette,
                            Icon::Music,
                            Color32::from_rgb(0xf3, 0x6b, 0x00),
                            &audio,
                            item_height,
                        ) {
                            app.actions.push(Action::Attach { documents: false });
                        }

                        let contact = crate::i18n::gettext(locale, "Contact");
                        if widgets::menu_item_icon_colored(
                            ui,
                            &palette,
                            Icon::Contact,
                            Color32::from_rgb(0x00, 0xa5, 0xf4),
                            &contact,
                            item_height,
                        ) {
                            app.actions.push(Action::ShowDialog(Dialog::NewContact));
                        }

                        let poll = crate::i18n::gettext(locale, "Poll");
                        if widgets::menu_item_icon_colored(
                            ui,
                            &palette,
                            Icon::ListChecks,
                            Color32::from_rgb(0xff, 0xb0, 0x00),
                            &poll,
                            item_height,
                        ) {
                            app.actions
                                .push(Action::ShowDialog(Dialog::CreatePoll(chat.id.clone())));
                        }

                        let event = if locale == crate::i18n::Locale::Spanish {
                            "Evento".to_string()
                        } else {
                            crate::i18n::gettext(locale, "Event").into_owned()
                        };
                        if widgets::menu_item_icon_colored(
                            ui,
                            &palette,
                            Icon::Calendar,
                            Color32::from_rgb(0xff, 0x57, 0x57),
                            &event,
                            item_height,
                        ) {
                            app.actions
                                .push(Action::ShowDialog(Dialog::SendLocation(chat.id.clone())));
                        }

                        let sticker = if locale == crate::i18n::Locale::Spanish {
                            "Nuevo sticker".to_string()
                        } else {
                            crate::i18n::gettext(locale, "New sticker").into_owned()
                        };
                        if widgets::menu_item_icon_colored(
                            ui,
                            &palette,
                            Icon::Sticker,
                            Color32::from_rgb(0x00, 0xc9, 0x82),
                            &sticker,
                            item_height,
                        ) {
                            app.actions.push(Action::Attach { documents: false });
                        }
                    });
                });
        } else {
            widgets::cleanup_menu_anim(&plus.ctx, anim_id);
        }
        if draw_open != app.composer_tools_open {
            app.actions.push(Action::SetComposerTools(draw_open));
        }
    }

    pub fn view_once_button(app: &App, ui: &mut egui::Ui) -> egui::Response {
        let palette = app.palette;
        let selected = app.pending_view_once;
        let (rect, response) =
            ui.allocate_exact_size(Vec2::splat(COMPOSER_CONTROL), Sense::click());
        theme::reveal_focus(&response);
        theme::focus_outline(ui, response.id, rect, COMPOSER_CONTROL * 0.5);
        response.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::WidgetType::Checkbox,
                ui.is_enabled(),
                selected,
                crate::i18n::gettext(app.locale, "View once"),
            )
        });
        if ui.is_rect_visible(rect) {
            let hovered = response.hovered() || response.has_focus();
            let fill = if selected {
                if hovered {
                    palette.accent_hover
                } else {
                    palette.accent
                }
            } else if hovered {
                palette.surface_hover
            } else {
                Color32::TRANSPARENT
            };
            ui.painter()
                .circle_filled(rect.center(), COMPOSER_CONTROL * 0.5, fill);
            let color = if selected {
                palette.on_accent
            } else {
                palette.secondary
            };
            theme::paint_icon(ui, Icon::ViewOnce, rect, 24.0, color);
        }
        response
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(crate::i18n::gettext(
                app.locale,
                "The recipient can open this photo only once, on their phone.",
            ))
    }
}

/// Active voice recording bar displayed inside `ComposerPill`.
pub struct VoiceRecorderBar;

impl VoiceRecorderBar {
    pub fn show(app: &mut App, ui: &mut egui::Ui) {
        let palette = app.palette;
        let Some(recorder) = app.recording.as_ref() else {
            return;
        };
        let (elapsed, levels, listening) =
            (recorder.elapsed(), recorder.levels(), recorder.listening());
        let line_height = ui
            .painter()
            .layout_no_wrap("x".to_owned(), theme::regular(BODY_SIZE), palette.text)
            .size()
            .y;
        let row_height = (line_height + COMPOSER_PADDING)
            .round()
            .max(COMPOSER_CONTROL);
        let button = row_height;
        ui.allocate_ui_with_layout(
            vec2(ui.available_width().max(0.0), row_height),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                if theme::circle_button(
                    ui,
                    Icon::Trash,
                    button,
                    palette.surface,
                    palette.surface_hover,
                    palette.secondary,
                    "Discard",
                )
                .clicked()
                {
                    app.actions.push(Action::CancelRecording);
                }
                let (dot, _) = ui.allocate_exact_size(Vec2::splat(12.0), Sense::hover());
                let light = if listening {
                    let pulse = 0.55 + 0.45 * (elapsed.as_secs_f32() * 3.0).sin().abs();
                    palette.danger.gamma_multiply(pulse)
                } else {
                    palette.secondary.gamma_multiply(0.5)
                };
                ui.painter().circle_filled(dot.center(), 5.0, light);
                theme::text(
                    ui,
                    crate::util::duration(elapsed.as_secs() as u32),
                    theme::tabular(fastframe_fonts::Weight::Medium, 14.0),
                    palette.text,
                );
                let spacing = ui.spacing().item_spacing.x;
                let wave_width = (ui.available_width() - button - spacing).max(0.0);
                let (rect, _) = ui.allocate_exact_size(vec2(wave_width, 28.0), Sense::hover());
                ui.ctx()
                    .data_mut(|data| data.insert_temp(recording_wave_id(), rect));
                let pitch = 3.0;
                let count = (rect.width() / pitch).floor() as usize;
                let shown = &levels[levels.len().saturating_sub(count)..];
                for (index, level) in shown.iter().enumerate() {
                    let height = 2.0_f32 + (level * 4.0).min(1.0) * 24.0;
                    let x = rect.right() - (shown.len() - index) as f32 * pitch + 1.0;
                    ui.painter().rect_filled(
                        Rect::from_center_size(egui::pos2(x, rect.center().y), vec2(2.0, height)),
                        1.0,
                        palette.accent,
                    );
                }
                if theme::circle_button(
                    ui,
                    Icon::Send,
                    button,
                    palette.accent,
                    palette.accent_hover,
                    palette.on_accent,
                    "Send",
                )
                .clicked()
                {
                    app.actions.push(Action::SendRecording);
                }
            },
        );
    }
}

pub struct SelectionBar;

impl SelectionBar {
    pub fn show(app: &mut App, ui: &mut egui::Ui, chat: &str, selected: &[String]) {
        let palette = app.palette;
        let locale = app.locale;
        ui.horizontal(|ui| {
            if theme::icon_button(
                ui,
                Icon::X,
                18.0,
                palette.secondary,
                palette.text,
                &crate::i18n::gettext(locale, "Cancel selection"),
            )
            .clicked()
                || ui.input(|input| input.key_pressed(Key::Escape))
            {
                app.actions.push(Action::CancelSelection);
            }
            let count = crate::i18n::ngettext(
                locale,
                "{count} selected",
                "{count} selected",
                selected.len() as u32,
            )
            .replace("{count}", &selected.len().to_string());
            theme::text(ui, &count, theme::medium(14.5), palette.text);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let forward = ui
                    .add_enabled_ui(!selected.is_empty(), |ui| {
                        theme::pill_button(
                            ui,
                            &palette,
                            &crate::i18n::gettext(locale, "Forward…"),
                            true,
                        )
                    })
                    .inner;
                if forward.clicked() {
                    app.actions.push(Action::ShowDialog(Dialog::Forward {
                        chat: chat.to_owned(),
                        messages: selected.to_vec(),
                    }));
                }
                let delete = ui
                    .add_enabled_ui(!selected.is_empty(), |ui| {
                        theme::pill_button(
                            ui,
                            &palette,
                            &crate::i18n::gettext(locale, "Delete…"),
                            false,
                        )
                    })
                    .inner;
                #[cfg(any(test, feature = "demo"))]
                if app.open_selection_delete_menu {
                    egui::Popup::open_id(ui.ctx(), delete.id.with("popup"));
                }
                let can_revoke = app.conversations.get(chat).is_some_and(|conversation| {
                    selection_can_revoke(&conversation.messages, selected, crate::util::now())
                });
                let labels = [
                    crate::i18n::gettext(app.locale, "Delete for everyone"),
                    crate::i18n::gettext(app.locale, "Delete for me"),
                ];
                let width = widgets::menu_width(ui, &[&*labels[0], &*labels[1]], true);
                let mut actions = Vec::new();
                egui::Popup::menu(&delete)
                    .width(width)
                    .frame(widgets::menu_frame(&palette))
                    .show(|ui| {
                        selection_delete_items(
                            ui,
                            &palette,
                            app.locale,
                            chat,
                            can_revoke,
                            selected,
                            &mut actions,
                        );
                    });
                app.actions.extend(actions);
            });
        });
    }
}
