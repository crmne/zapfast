//! Details and confirmed deletion of our own session-local updates.
use super::*;

pub(super) fn show(app: &mut App, ctx: &egui::Context) {
    let locale = app.locale;
    let palette = app.palette;
    if let Some(id) = app.status_delete_confirm.clone() {
        let response = egui::Modal::new(egui::Id::new("delete-own-status")).show(ctx, |ui| {
            ui.set_max_width(340.0);
            ui.heading(gettext(locale, "Delete status?"));
            ui.label(gettext(
                locale,
                "This update will be removed for everyone it was shared with.",
            ));
            ui.horizontal(|ui| {
                if ui.button(gettext(locale, "Cancel")).clicked() {
                    app.status_delete_confirm = None;
                }
                if ui.button(gettext(locale, "Delete status")).clicked() {
                    app.actions.push(Action::DeleteStatus {
                        account: app.account().id.clone(),
                        id,
                    });
                    app.status_delete_confirm = None;
                }
            });
        });
        if response.should_close() {
            app.status_delete_confirm = None;
        }
    }
    if app.status_viewers_open {
        /// A row: the 36 px avatar and the space under it.
        const ROW: f32 = 44.0;
        // The list is shared, not copied; only the people shown are gathered.
        let all = app
            .status_selected
            .as_ref()
            .and_then(|(_, id)| app.status_views.get(id))
            .cloned()
            .unwrap_or_default();
        let views: Vec<_> = all
            .iter()
            .filter(|viewer| {
                !app.blocklist.contains(&viewer.contact)
                    && !app.chat(&viewer.contact).is_some_and(|chat| chat.locked)
            })
            .collect();
        let response = egui::Modal::new(egui::Id::new("own-status-viewers")).show(ctx, |ui| {
            ui.set_width((ctx.content_rect().width() - 64.0).clamp(240.0, 360.0));
            ui.horizontal(|ui| {
                if theme::icon_button(
                    ui,
                    Icon::X,
                    20.0,
                    palette.text,
                    palette.text,
                    &gettext(locale, "Close status viewers"),
                )
                .clicked()
                {
                    app.status_viewers_open = false;
                }
                ui.heading(gettext(locale, "Status views"));
            });
            ui.label(gettext(
                locale,
                "View receipts received by this device. Earlier views may be unavailable.",
            ));
            if views.is_empty() {
                ui.add_space(20.0);
                ui.label(gettext(locale, "No views received yet"));
            }
            // Rows are exactly ROW apart, as `show_rows` counts them.
            let gap = ui.spacing().item_spacing.y;
            ui.spacing_mut().item_spacing.y = 0.0;
            egui::ScrollArea::vertical()
                .max_height(440.0)
                .auto_shrink([false, true])
                .show_rows(ui, ROW, views.len(), |ui, range| {
                    for viewer in &views[range] {
                        let (row, _) =
                            ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::hover());
                        ui.scope_builder(
                            egui::UiBuilder::new()
                                .max_rect(row)
                                .layout(Layout::left_to_right(Align::Min)),
                            |ui| {
                                ui.spacing_mut().item_spacing.y = gap;
                                let name = app.display_name(&viewer.contact);
                                let path = app.avatar(&viewer.contact);
                                widgets::avatar(
                                    ui,
                                    &palette,
                                    &name,
                                    &viewer.contact,
                                    36.0,
                                    path.as_deref(),
                                );
                                ui.vertical(|ui| {
                                    ui.label(name);
                                    ui.small(crate::util::moment_stamp(locale, viewer.timestamp));
                                });
                            },
                        );
                    }
                });
        });
        if response.should_close() {
            app.status_viewers_open = false;
        }
    }
}
