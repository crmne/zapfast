//! Auxiliary strips displayed above the composer pill (uploads, edits, link previews, unsent voice).

use super::*;

pub struct ComposerStrips;

impl ComposerStrips {
    pub fn uploads(app: &mut App, ui: &mut egui::Ui, chat: &str) {
        upload_strips(app, ui, chat);
    }

    pub fn unsent_voice(app: &mut App, ui: &mut egui::Ui, samples: usize) {
        unsent_voice_strip(app, ui, samples);
    }

    pub fn edit(app: &mut App, ui: &mut egui::Ui) {
        edit_strip(app, ui);
    }

    pub fn link_preview(app: &mut App, ui: &mut egui::Ui) {
        link_preview_strip(app, ui);
    }
}

pub(crate) const STRIP_GAP: f32 = 3.0;

/// Leaves [`STRIP_GAP`] below a strip, the layout's own spacing included.
pub(crate) fn strip_gap(ui: &mut egui::Ui) {
    ui.add_space(STRIP_GAP - ui.spacing().item_spacing.y);
}

pub(crate) const STRIP_RADIUS: u8 = 16;

pub(crate) fn strip_frame(palette: &Palette) -> Frame {
    Frame::new()
        .fill(palette.bubble_in)
        .corner_radius(CornerRadius::same(STRIP_RADIUS))
        .inner_margin(Margin {
            left: 14,
            right: 10,
            top: 6,
            bottom: 6,
        })
}

pub(crate) fn suggestion_frame(palette: &Palette) -> Frame {
    Frame::new()
        .fill(palette.bubble_in)
        .corner_radius(CornerRadius::same(STRIP_RADIUS))
        .inner_margin(Margin::same(SUGGESTION_INSET))
}

/// Space between a suggestion list's edge and its rows.
pub(crate) const SUGGESTION_INSET: i8 = 4;
/// A suggestion row's corners, concentric with the list's.
pub(crate) const SUGGESTION_ROW_RADIUS: f32 = (STRIP_RADIUS as i8 - SUGGESTION_INSET) as f32;

pub(crate) fn unsent_voice_strip(app: &mut App, ui: &mut egui::Ui, samples: usize) {
    let palette = app.palette;
    let seconds = (samples as f64 / f64::from(crate::voice::RATE))
        .round()
        .max(1.0) as u32;
    let label = crate::i18n::gettext(app.locale, "Voice message ({duration}) not sent")
        .replace("{duration}", &crate::util::duration(seconds));
    let discard = crate::i18n::gettext(app.locale, "Discard voice message");
    widgets::raised(ui, &palette, strip_frame(&palette), |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            theme::icon(ui, Icon::Mic, 16.0, palette.danger);
            theme::text(ui, &label, theme::semibold(12.5), palette.danger);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let button = theme::icon_button(
                    ui,
                    Icon::X,
                    16.0,
                    palette.secondary,
                    palette.text,
                    discard.as_ref(),
                );
                #[cfg(test)]
                ui.ctx().data_mut(|data| {
                    data.insert_temp(egui::Id::new("unsent-voice-discard"), button.rect)
                });
                if button.clicked() {
                    app.actions.push(Action::DiscardUnsentVoice);
                }
            });
        });
    });
    strip_gap(ui);
}

pub(crate) fn upload_strips(app: &mut App, ui: &mut egui::Ui, chat: &str) {
    let palette = app.palette;
    let cancel = crate::i18n::gettext(app.locale, "Cancel sending {name}");
    let mut cancelled = None;
    for upload in app.uploads.iter().filter(|upload| upload.chat == chat) {
        let name = if upload.name.is_empty() {
            crate::i18n::gettext(app.locale, "Photo").into_owned()
        } else {
            upload.name.clone()
        };
        let label = crate::i18n::gettext(app.locale, "Sending {name}").replace("{name}", &name);
        let cancel = cancel.replace("{name}", &name);
        let amount = match upload.seconds_left {
            Some(seconds) => format!(
                "{}% · {}",
                upload.percent,
                crate::i18n::gettext(app.locale, "{time} left")
                    .replace("{time}", &crate::util::duration(seconds))
            ),
            None => format!("{}%", upload.percent),
        };
        // Before it is prepared nothing has gone, and how long that takes is unknown.
        let fraction = (upload.percent > 0).then(|| f32::from(upload.percent) / 100.0);
        let _strip = widgets::raised(ui, &palette, strip_frame(&palette), |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                theme::progress(ui, 16.0, fraction, palette.accent);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let button = theme::icon_button(
                        ui,
                        Icon::X,
                        16.0,
                        palette.secondary,
                        palette.text,
                        &cancel,
                    );
                    #[cfg(test)]
                    ui.ctx().data_mut(|data| {
                        data.insert_temp(egui::Id::new(("upload-cancel", upload.id)), button.rect)
                    });
                    if button.clicked() {
                        cancelled = Some(upload.id);
                    }
                    theme::text(
                        ui,
                        amount,
                        theme::tabular(fastframe_fonts::Weight::Regular, 12.5),
                        palette.secondary,
                    );
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        theme::text(ui, &label, theme::semibold(12.5), palette.text);
                    });
                });
            });
        });
        #[cfg(test)]
        ui.ctx().data_mut(|data| {
            data.insert_temp(
                egui::Id::new(("upload-strip", upload.id)),
                _strip.response.rect,
            )
        });
        strip_gap(ui);
    }
    if let Some(id) = cancelled {
        app.actions.push(Action::CancelUpload(id));
    }
}

pub(crate) fn edit_strip(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    widgets::raised(ui, &palette, strip_frame(&palette), |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            theme::icon(ui, Icon::Pencil, 16.0, palette.accent);
            theme::text(ui, "Editing message", theme::semibold(12.5), palette.accent);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if theme::icon_button(
                    ui,
                    Icon::X,
                    16.0,
                    palette.secondary,
                    palette.text,
                    "Stop editing (Esc)",
                )
                .clicked()
                {
                    app.actions.push(Action::CancelEdit);
                }
            });
        });
    });
    strip_gap(ui);
}

pub(crate) fn reply_strip(app: &mut App, ui: &mut egui::Ui, quoted: &Message) {
    let palette = app.palette;
    let who = if quoted.from_me {
        "You".to_owned()
    } else {
        app.display_name_or(&quoted.sender, quoted.sender_name.as_deref())
    };
    let summary = markup::plain(&quoted.summary_in(app.locale), &app.mention_list(quoted));
    let strip = widgets::raised(ui, &palette, strip_frame(&palette), |ui| {
        ui.set_width(ui.available_width().max(0.0));
        ui.horizontal(|ui| {
            let (bar, _) = ui.allocate_exact_size(vec2(3.0, 34.0), Sense::hover());
            ui.painter().rect_filled(bar, 2.0, palette.accent);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                ui.set_max_width((ui.available_width() - 40.0).max(0.0));
                widgets::rich_text(
                    ui,
                    &format!("Replying to {who}"),
                    theme::semibold(12.5),
                    palette.accent,
                );
                widgets::rich_text(ui, &summary, theme::regular(12.5), palette.secondary);
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
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
        });
    });
    ui.ctx()
        .data_mut(|data| data.insert_temp(reply_strip_id(), strip.response.rect));
    strip_gap(ui);
}

pub(crate) fn link_preview_strip(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    let Some(shown) = app.shown_preview() else {
        return;
    };
    let mut dismiss = false;
    let strip = match &shown.state {
        LinkPreviewState::Dismissed => return,
        LinkPreviewState::Loading => {
            let loading = crate::i18n::gettext(app.locale, "Loading preview…");
            widgets::raised(ui, &palette, strip_frame(&palette), |ui| {
                ui.set_width(ui.available_width().max(0.0));
                ui.horizontal(|ui| {
                    let (bar, _) = ui.allocate_exact_size(vec2(3.0, 18.0), Sense::hover());
                    ui.painter().rect_filled(bar, 2.0, palette.accent);
                    theme::text(ui, loading, theme::regular(12.5), palette.secondary);
                });
            })
        }
        LinkPreviewState::Ready(composed) => {
            let remove = crate::i18n::gettext(app.locale, "Remove link preview");
            let image = composer_thumbnail_id(&composed.preview.url);
            let thumbnail = composed
                .thumbnail
                .as_deref()
                .map(|bytes| (shown.chat.as_str(), image.as_str(), bytes));
            let frame = strip_frame(&palette).inner_margin(Margin::same(10));
            widgets::raised(ui, &palette, frame, |ui| {
                ui.set_width(ui.available_width().max(0.0));
                ui.with_layout(Layout::left_to_right(Align::Min), |ui| {
                    ui.spacing_mut().item_spacing.x = 10.0;
                    let right = ui.max_rect().right();
                    let (bar, _) = ui.allocate_exact_size(vec2(3.0, 0.0), Sense::hover());
                    let width = (ui.available_width() - 46.0).max(0.0);
                    let card = preview_layout(ui, &palette, &composed.preview, thumbnail, width);
                    ui.painter().rect_filled(
                        Rect::from_x_y_ranges(bar.x_range(), card.y_range()),
                        2.0,
                        palette.accent,
                    );
                    let room = Rect::from_x_y_ranges(card.right()..=right, card.y_range());
                    ui.scope_builder(
                        egui::UiBuilder::new()
                            .max_rect(room)
                            .layout(Layout::right_to_left(Align::Center)),
                        |ui| {
                            let button = theme::icon_button(
                                ui,
                                Icon::X,
                                16.0,
                                palette.secondary,
                                palette.text,
                                remove.as_ref(),
                            );
                            #[cfg(test)]
                            ui.ctx().data_mut(|data| {
                                data.insert_temp(egui::Id::new("link-preview-dismiss"), button.rect)
                            });
                            dismiss = button.clicked();
                        },
                    );
                });
            })
        }
    };
    ui.ctx()
        .data_mut(|data| data.insert_temp(link_preview_strip_id(), strip.response.rect));
    if dismiss {
        app.actions.push(Action::DismissLinkPreview);
    }
    strip_gap(ui);
}

/// The image id of the composer card's thumbnail for `url`.
pub(crate) fn composer_thumbnail_id(url: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    url.hash(&mut hasher);
    format!("composer-preview-{:016x}", hasher.finish())
}

/// Where the composer's link preview was drawn, for layout tests.
pub(crate) fn link_preview_strip_id() -> egui::Id {
    egui::Id::new("link-preview-strip")
}

/// Where the emoji or @mention suggestion list was drawn, for layout tests.
pub(crate) fn suggestion_list_id() -> egui::Id {
    egui::Id::new("suggestion-list")
}

/// Where the reply strip was drawn, for layout tests.
pub(crate) fn reply_strip_id() -> egui::Id {
    egui::Id::new("reply-strip")
}
