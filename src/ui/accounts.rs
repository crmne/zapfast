//! Account rail on the left of the chat list.

use egui::{Align2, CornerRadius, Frame, Margin, Stroke, vec2};

use crate::app::App;
use crate::model::{Action, Dialog};
use crate::theme::{self, Icon};

const RAIL: f32 = 56.0;

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    let locale = app.locale;
    let active = app.account().id.clone();
    egui::Panel::left("account-rail")
        .exact_size(RAIL)
        .resizable(false)
        .show_separator_line(false)
        .frame(Frame::new().fill(palette.window).inner_margin(Margin::ZERO))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            ui.add_space(10.0);
            let accounts: Vec<_> = app
                .accounts
                .iter()
                .map(|account| {
                    (
                        account.id.clone(),
                        account.display_label(locale),
                        account.unread_total(),
                        account.badge_color(),
                        account.me.clone().unwrap_or_default(),
                        account.me_name.clone().unwrap_or_default(),
                    )
                })
                .collect();
            for (id, label, unread, color, me, name) in accounts {
                let selected = id == active;
                let picture = app
                    .accounts
                    .iter()
                    .find(|account| account.id == id)
                    .and_then(|account| {
                        account
                            .avatars
                            .get(&me)
                            .and_then(|path| path.as_ref())
                            .cloned()
                    });
                ui.vertical_centered(|ui| {
                    let response = crate::ui::widgets::clickable_avatar(
                        ui,
                        &palette,
                        &name,
                        &me,
                        36.0,
                        picture.as_deref(),
                        &label,
                    );
                    if selected {
                        ui.painter().circle_stroke(
                            response.rect.center(),
                            20.0,
                            Stroke::new(2.0, palette.accent),
                        );
                    } else {
                        ui.painter().circle_filled(
                            response.rect.right_top() + vec2(-4.0, 4.0),
                            4.0,
                            color,
                        );
                    }
                    if unread > 0 {
                        let badge = format!("{unread}");
                        let pos = response.rect.right_bottom() + vec2(-2.0, -2.0);
                        ui.painter().circle_filled(pos, 8.0, palette.danger);
                        ui.painter().text(
                            pos,
                            Align2::CENTER_CENTER,
                            badge,
                            theme::medium(9.0),
                            palette.on_accent,
                        );
                    }
                    response.clone().on_hover_text(&label);
                    if response.clicked() {
                        app.actions.push(Action::SwitchAccount(id.clone()));
                    }
                    response.context_menu(|ui| {
                        if ui.button(crate::i18n::gettext(locale, "Unlink")).clicked() {
                            app.actions.push(Action::SwitchAccount(id.clone()));
                            app.actions.push(Action::ShowDialog(Dialog::ConfirmUnlink));
                            ui.close();
                        }
                        if ui.button(crate::i18n::gettext(locale, "Remove")).clicked() {
                            app.actions
                                .push(Action::ShowDialog(Dialog::ConfirmRemoveAccount(id.clone())));
                            ui.close();
                        }
                    });
                });
            }
            ui.with_layout(egui::Layout::bottom_up(egui::Align::Center), |ui| {
                ui.add_space(10.0);
                let add = crate::i18n::gettext(locale, "Add account");
                if theme::icon_button(ui, Icon::Plus, 18.0, palette.secondary, palette.text, &add)
                    .clicked()
                {
                    app.actions.push(Action::AddAccount);
                }
            });
        });
    let rect = ui.min_rect();
    ui.painter().vline(
        rect.right(),
        rect.y_range(),
        Stroke::new(1.0, palette.outline),
    );
    let _ = CornerRadius::same(theme::RADIUS);
}
