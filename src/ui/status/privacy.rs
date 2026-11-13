use super::*;
use crate::status::{Audience, AudienceMode};

/// One modal, with an uncommitted contact-selection step inside it.
pub(super) fn show(app: &mut App, ctx: &egui::Context) {
    if !app.status_privacy_open {
        return;
    }
    let palette = app.palette;
    let inline = app.status_draft.is_none();
    if inline && app.status_privacy_picker.is_none() {
        return;
    }
    let response = egui::Modal::new(egui::Id::new("status-audience"))
        .frame(
            Frame::new()
                .fill(palette.window)
                .inner_margin(Margin::same(16))
                .corner_radius(16.0),
        )
        .show(ctx, |ui| {
            let size = vec2(
                (ctx.content_rect().width() - 64.0).clamp(240.0, 318.0),
                (ctx.content_rect().height() - 112.0).clamp(300.0, 628.0),
            );
            let (panel, _) = ui.allocate_exact_size(size, Sense::hover());
            let picking = app.status_privacy_picker.is_some();
            #[cfg(any(test, feature = "demo"))]
            ctx.data_mut(|data| data.insert_temp(egui::Id::new("status-audience-panel"), panel));
            let mut child = ui.new_child(
                egui::UiBuilder::new()
                    .id(egui::Id::new(("status-audience-content", picking)))
                    .layout(Layout::top_down(Align::Min))
                    .max_rect(panel),
            );
            child.set_clip_rect(panel.expand(16.0));
            if picking {
                contacts(app, &mut child, &palette);
            } else {
                choices(app, &mut child, &palette, false);
            }
        });
    // A click beside it leaves the contact step, and the modal too unless
    // the choices behind it are the Status page's own.
    if response.should_close() {
        app.status_privacy_picker = None;
        if !inline {
            app.status_privacy_open = false;
        }
    }
}

pub(super) fn inline(app: &mut App, ui: &mut egui::Ui) {
    header_room(ui);
    let palette = app.palette;
    Frame::new()
        .inner_margin(Margin::same(20))
        .show(ui, |ui| choices(app, ui, &palette, true));
}

fn choices(app: &mut App, ui: &mut egui::Ui, palette: &theme::Palette, inline: bool) {
    let locale = app.locale;
    ui.horizontal(|ui| {
        if theme::icon_button(
            ui,
            if inline { Icon::ArrowLeft } else { Icon::X },
            18.0,
            palette.text,
            palette.text,
            &gettext(locale, "Close status privacy"),
        )
        .clicked()
        {
            app.status_privacy_open = false;
        }
        ui.label(gettext(locale, "Status privacy"));
    });
    ui.add_space(24.0);
    ui.label(
        RichText::new(gettext(locale, "Who can see this status update"))
            .color(palette.secondary)
            .size(13.0),
    );
    ui.add_space(20.0);
    // Literal calls, so xgettext extracts each label.
    for (mode, title, hint) in [
        (
            AudienceMode::Contacts,
            gettext(locale, "My contacts"),
            gettext(locale, "Share with all saved contacts"),
        ),
        (
            AudienceMode::Except,
            gettext(locale, "My contacts except…"),
            gettext(locale, "Share with all contacts except those selected"),
        ),
        (
            AudienceMode::Only,
            gettext(locale, "Only share with…"),
            gettext(locale, "Share only with selected contacts"),
        ),
    ] {
        let selected = app
            .status_audience
            .as_ref()
            .is_some_and(|audience| audience.mode == mode);
        if widgets::radio_option(ui, palette, &title, &hint, selected).clicked() {
            if mode == AudienceMode::Contacts {
                app.actions.push(Action::SetStatusAudience {
                    account: app.account().id.clone(),
                    audience: Audience {
                        mode,
                        selected: HashSet::new(),
                    },
                });
            } else {
                let selected = app
                    .status_audience
                    .as_ref()
                    .filter(|audience| audience.mode == mode)
                    .map(|audience| audience.selected.clone())
                    .unwrap_or_else(|| app.status_audiences.selected(mode));
                app.status_privacy_contacts = None;
                app.status_privacy_picker = Some(Audience { mode, selected });
                app.status_privacy_search.clear();
                ui.memory_mut(|memory| {
                    memory.request_focus(egui::Id::new("status-audience-search"))
                });
            }
        }
    }
    ui.add_space(16.0);
    ui.small(gettext(
        locale,
        "Saved on this device. Your phone's default is unchanged.",
    ));
}

fn eligible(app: &App, needle: &str) -> Vec<crate::model::Contact> {
    crate::ui::dialogs::contact_choices(app, needle)
        .into_iter()
        .filter(|contact| {
            app.contacts
                .get(&contact.id)
                .and_then(|saved| saved.full_name.as_deref())
                .is_some_and(|name| !name.is_empty())
                && !app.blocklist.contains(&contact.id)
        })
        .collect()
}

fn contacts(app: &mut App, ui: &mut egui::Ui, palette: &theme::Palette) {
    let locale = app.locale;
    let mode = app
        .status_privacy_picker
        .as_ref()
        .expect("a selection step has an audience")
        .mode;
    if app.status_privacy_contacts.is_none() {
        let all = eligible(app, "");
        let keys = all
            .iter()
            .map(|contact| {
                crate::util::search_key(&format!(
                    "{} {}",
                    app.display_name(&contact.id),
                    crate::model::phone_of(&contact.id).unwrap_or_default()
                ))
            })
            .collect();
        let ids = all.iter().map(|contact| contact.id.clone()).collect();
        app.status_privacy_contacts = Some(crate::status::ContactChoices {
            ids,
            all: std::sync::Arc::new(all),
            keys,
            ..Default::default()
        });
    }
    let all = std::sync::Arc::clone(
        &app.status_privacy_contacts
            .as_ref()
            .expect("the picker has a snapshot")
            .all,
    );
    ui.horizontal(|ui| {
        if theme::icon_button(
            ui,
            Icon::X,
            18.0,
            palette.text,
            palette.text,
            &gettext(locale, "Back to status privacy"),
        )
        .clicked()
        {
            app.status_privacy_picker = None;
        }
        ui.label(if mode == AudienceMode::Only {
            gettext(locale, "Only share with…")
        } else {
            gettext(locale, "My contacts except…")
        });
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let picked = !all.is_empty()
                && app.status_privacy_picker.as_ref().is_some_and(|audience| {
                    all.iter()
                        .all(|contact| audience.selected.contains(&contact.id))
                });
            let (check, _) = ui.allocate_exact_size(Vec2::splat(24.0), Sense::hover());
            if widgets::square_check(
                ui,
                palette,
                check,
                "status-select-all",
                picked,
                &gettext(locale, "Select all contacts"),
            ) && let Some(audience) = app.status_privacy_picker.as_mut()
            {
                for contact in all.iter() {
                    if picked {
                        audience.selected.remove(&contact.id);
                    } else {
                        audience.selected.insert(contact.id.clone());
                    }
                }
            }
        });
    });
    ui.add_space(8.0);
    let search_rect = Rect::from_min_size(ui.cursor().min, vec2(ui.available_width(), 34.0));
    let mut search_palette = *palette;
    search_palette.surface_hover = palette.window;
    let search = widgets::search_field(
        ui,
        &search_palette,
        egui::Id::new("status-audience-search"),
        &mut app.status_privacy_search,
        &gettext(locale, "Search name or number"),
        ui.available_width(),
    );
    if search.has_focus() {
        ui.painter().rect_stroke(
            search_rect,
            17.0,
            Stroke::new(1.5, palette.accent),
            egui::StrokeKind::Inside,
        );
    }
    ui.add_space(10.0);
    let mut selected: Vec<_> = app
        .status_privacy_picker
        .as_ref()
        .map(|audience| audience.selected.iter().cloned().collect())
        .unwrap_or_default();
    selected.retain(|id| {
        app.status_privacy_contacts
            .as_ref()
            .is_some_and(|cache| cache.ids.contains(id))
    });
    selected.sort_unstable();
    let mut chip_palette = *palette;
    chip_palette.surface = Color32::TRANSPARENT;
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        let _scroll = egui::ScrollArea::vertical()
            .id_salt("status-audience-chips")
            .max_height(78.0)
            .auto_shrink([false, true])
            .show_rows(ui, 36.0, selected.len().div_ceil(2), |ui, range| {
                for row in range {
                    let start = row * 2;
                    // Two chips to a row, each name as wide as half the row allows.
                    let chip_label = ((ui.available_width() - 6.0) / 2.0 - 58.0).clamp(30.0, 140.0);
                    if let Some(id) = crate::ui::new_group::selection_chips(
                        app,
                        ui,
                        &chip_palette,
                        &selected[start..(start + 2).min(selected.len())],
                        chip_label,
                    ) && let Some(audience) = app.status_privacy_picker.as_mut()
                    {
                        audience.selected.remove(&id);
                    }
                    ui.add_space(6.0);
                }
            });
        #[cfg(any(test, feature = "demo"))]
        ui.ctx().data_mut(|data| {
            data.insert_temp(egui::Id::new("status-chip-viewport"), _scroll.inner_rect)
        });
    });
    ui.add_space(10.0);
    ui.label(
        RichText::new(gettext(locale, "Contacts"))
            .size(12.0)
            .color(palette.secondary),
    );
    ui.add_space(6.0);
    let needle = crate::util::search_key(app.status_privacy_search.trim());
    let cache = app
        .status_privacy_contacts
        .as_mut()
        .expect("the picker has a snapshot");
    if cache.query.as_deref() != Some(&needle) {
        cache.matches = std::sync::Arc::new(
            cache
                .keys
                .iter()
                .enumerate()
                .filter_map(|(i, key)| crate::util::search_matches(key, &needle).then_some(i))
                .collect(),
        );
        cache.query = Some(needle);
    }
    let matches = std::sync::Arc::clone(&cache.matches);
    let height = (ui.available_height() - 38.0).max(60.0);
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        egui::ScrollArea::vertical()
            .id_salt("status-audience-contacts")
            .max_height(height)
            .auto_shrink([false, false])
            .show_rows(ui, 60.0, matches.len(), |ui, range| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for &index in &matches[range] {
                    let contact = &all[index];
                    let picked = app
                        .status_privacy_picker
                        .as_ref()
                        .is_some_and(|audience| audience.selected.contains(&contact.id));
                    if crate::ui::new_group::selectable_contact_row(
                        app,
                        ui,
                        palette,
                        &contact.id,
                        picked,
                        true,
                    ) && let Some(audience) = app.status_privacy_picker.as_mut()
                    {
                        if picked {
                            audience.selected.remove(&contact.id);
                        } else {
                            audience.selected.insert(contact.id.clone());
                        }
                    }
                    ui.add_space(8.0);
                }
                if matches.is_empty() {
                    ui.label(gettext(locale, "No matching contacts"));
                }
            });
    });
    let count = app
        .status_privacy_picker
        .as_ref()
        .map_or(0, |audience| audience.selected.len());
    ui.add_space(8.0);
    let footer = Rect::from_min_max(
        pos2(ui.max_rect().left() - 16.0, ui.max_rect().bottom() - 32.0),
        ui.max_rect().max + vec2(16.0, 16.0),
    );
    ui.painter()
        .with_clip_rect(ui.clip_rect().expand(16.0))
        .rect_filled(
            footer,
            egui::CornerRadius {
                nw: 0,
                ne: 0,
                sw: 16,
                se: 16,
            },
            palette.surface,
        );
    let mut row = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(footer.shrink2(vec2(16.0, 4.0)))
            .layout(Layout::left_to_right(Align::Center)),
    );
    let ui = &mut row;
    let label = if mode == AudienceMode::Only {
        crate::i18n::ngettext(
            locale,
            "{count} contact included",
            "{count} contacts included",
            count as u32,
        )
    } else {
        crate::i18n::ngettext(
            locale,
            "{count} contact excluded",
            "{count} contacts excluded",
            count as u32,
        )
    }
    .replace("{count}", &count.to_string());
    ui.label(RichText::new(label).size(13.0));
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        if ui
            .add_enabled_ui(mode != AudienceMode::Only || count > 0, |ui| {
                theme::circle_button(
                    ui,
                    Icon::Check,
                    40.0,
                    palette.accent,
                    palette.accent_hover,
                    palette.on_accent,
                    &gettext(locale, "Confirm audience"),
                )
            })
            .inner
            .clicked()
            && let Some(audience) = app.status_privacy_picker.take()
        {
            app.actions.push(Action::SetStatusAudience {
                account: app.account().id.clone(),
                audience,
            });
        }
    });
}
