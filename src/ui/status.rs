//! WhatsApp-style Status list and immersive story viewer.

use super::widgets;
use crate::app::App;
use crate::i18n::gettext;
use crate::model::{Action, Content, MediaState};
use crate::status::Item;
use crate::theme::{self, Icon};
use egui::{
    Align, Color32, FontId, Frame, Layout, Margin, Pos2, Rect, RichText, Sense, Stroke, Vec2, pos2,
    vec2,
};
use std::collections::{HashMap, HashSet};
mod compose;
mod photo_tools;
mod privacy;
mod published;

pub(super) fn input_frame(palette: &theme::Palette) -> Frame {
    Frame::new()
        .fill(palette.surface)
        .inner_margin(Margin::symmetric(12, 9))
        .corner_radius(8.0)
}

pub(super) fn audience_button(app: &App, ui: &mut egui::Ui) -> egui::Response {
    let palette = app.palette;
    let label = match app.status_audience.as_ref() {
        Some(audience) => match audience.mode {
            crate::status::AudienceMode::Contacts => {
                gettext(app.locale, "Status (contacts)").into_owned()
            }
            crate::status::AudienceMode::Only => gettext(app.locale, "Status ({count} included)")
                .replace("{count}", &audience.selected.len().to_string()),
            crate::status::AudienceMode::Except => gettext(app.locale, "Status ({count} excluded)")
                .replace("{count}", &audience.selected.len().to_string()),
        },
        None => gettext(app.locale, "Status (choose audience)").into_owned(),
    };
    let tint = if app
        .status_draft
        .as_ref()
        .is_some_and(|draft| draft.media.is_some())
    {
        palette.accent
    } else {
        // White over the dark text backgrounds, black over the light ones.
        ui.visuals().text_color()
    };
    let line = widgets::line(
        ui,
        &label,
        theme::regular(12.0),
        tint,
        ui.available_width().min(230.0),
        1,
    );
    let (rect, response) = ui.allocate_exact_size(vec2(line.size().x + 46.0, 32.0), Sense::click());
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    theme::reveal_focus(&response);
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            ui.is_enabled(),
            gettext(app.locale, "Status audience"),
        )
    });
    ui.painter().rect_stroke(
        rect,
        16.0,
        Stroke::new(1.0, tint.gamma_multiply(0.55)),
        egui::StrokeKind::Inside,
    );
    theme::paint_icon(
        ui,
        Icon::Status,
        Rect::from_center_size(pos2(rect.left() + 17.0, rect.center().y), Vec2::splat(18.0)),
        18.0,
        tint,
    );
    line.paint(
        ui,
        pos2(rect.left() + 32.0, rect.center().y - line.size().y / 2.0),
        tint,
    );
    theme::focus_outline(ui, response.id, rect, 16.0);
    response
}

/// The contact list stays a navigation destination; stories take the window.
pub fn show(app: &mut App, ui: &mut egui::Ui) {
    page(app, ui);
    privacy::show(app, ui.ctx());
    published::show(app, ui.ctx());
}

fn page(app: &mut App, ui: &mut egui::Ui) {
    if app.status_draft.is_some() {
        compose::show(app, ui);
        return;
    }
    if let Some(item) = app
        .status_selected
        .as_ref()
        .and_then(|(sender, id)| {
            app.statuses
                .iter()
                .find(|item| item.sender == *sender && item.id == *id && app.can_view_status(item))
        })
        .cloned()
    {
        viewer(app, ui, &item);
        return;
    }
    if super::macos_title_strip(app, ui.ctx()) {
        super::title_strip(app, ui);
    }
    super::rail::show(app, ui, false);
    let palette = app.palette;
    if super::narrow(ui.ctx()) {
        egui::CentralPanel::default()
            .frame(
                Frame::new()
                    .fill(palette.sidebar())
                    .inner_margin(Margin::ZERO),
            )
            .show(ui, |ui| {
                if app.status_privacy_open {
                    privacy::inline(app, ui);
                } else {
                    list(app, ui);
                }
            });
    } else {
        let response = egui::Panel::left("status-list")
            .default_size(app.settings.sidebar_width)
            .size_range(260.0..=520.0)
            .resizable(true)
            .show_separator_line(false)
            .frame(
                Frame::new()
                    .fill(palette.sidebar())
                    .inner_margin(Margin::ZERO),
            )
            .show(ui, |ui| {
                if app.status_privacy_open {
                    privacy::inline(app, ui);
                } else {
                    list(app, ui);
                }
            });
        let width = response.response.rect.width();
        if (width - app.settings.sidebar_width).abs() > 1.0 {
            app.settings.sidebar_width = width;
            app.actions.push(Action::SettingsChanged);
        }
        egui::CentralPanel::default()
            .frame(super::central_frame(app).inner_margin(Margin::same(24)))
            .show(ui, |ui| {
                let area = ui.available_rect_before_wrap();
                let prompt =
                    Rect::from_center_size(area.center(), vec2(area.width().min(540.0), 260.0));
                ui.scope_builder(
                    egui::UiBuilder::new()
                        .max_rect(prompt)
                        .layout(Layout::top_down(Align::Center)),
                    |ui| {
                        let (rect, _) = ui.allocate_exact_size(vec2(76.0, 76.0), Sense::hover());
                        theme::paint_icon(ui, Icon::Status, rect, 48.0, palette.secondary);
                        ui.add_space(16.0);
                        ui.label(
                            RichText::new(gettext(app.locale, "Share status updates")).size(24.0),
                        );
                        ui.add_space(8.0);
                        ui.label(gettext(
                            app.locale,
                            "Share photos, videos and text that disappear after 24 hours.",
                        ));
                        ui.add_space(18.0);
                        ui.horizontal(|ui| {
                            ui.add_space(((ui.available_width() - 184.0) / 2.0).max(0.0));
                            let account = app.account().id.clone();
                            if start_button(
                                ui,
                                Icon::Pencil,
                                &gettext(app.locale, "Text status"),
                                &palette,
                            )
                            .clicked()
                            {
                                app.actions.push(Action::ComposeStatus {
                                    account: account.clone(),
                                    media: false,
                                });
                            }
                            if start_button(
                                ui,
                                Icon::Camera,
                                &gettext(app.locale, "Photo and video"),
                                &palette,
                            )
                            .clicked()
                            {
                                app.actions.push(Action::ComposeStatus {
                                    account,
                                    media: true,
                                });
                            }
                        });
                    },
                );
                ui.allocate_rect(area, Sense::hover());
            });
    }
}

fn header_room(ui: &mut egui::Ui) {
    let rect = Rect::from_min_size(
        ui.max_rect().min,
        vec2(ui.available_width(), theme::TOP_BAR_HEIGHT),
    );
    super::caption_drag(ui, rect);
    let room = super::caption_room(ui.ctx(), ui.max_rect(), ui.available_width());
    if room.top > 0.0 {
        ui.add_space(room.top + 8.0);
    }
}

fn list(app: &mut App, ui: &mut egui::Ui) {
    header_room(ui);
    let locale = app.locale;
    let palette = app.palette;
    Frame::new()
        .inner_margin(Margin::symmetric(14, 8))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(gettext(locale, "Status")).font(theme::bold(20.0)));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let plus = theme::icon_button(
                        ui,
                        Icon::CirclePlus,
                        20.0,
                        palette.text,
                        palette.text,
                        &gettext(locale, "Add status"),
                    );
                    egui::Popup::menu(&plus).show(|ui| {
                        // Literal calls, so xgettext extracts each label.
                        for (media, label) in [
                            (true, gettext(locale, "Photos and videos")),
                            (false, gettext(locale, "Text")),
                        ] {
                            if ui.button(label).clicked() {
                                app.actions.push(Action::ComposeStatus {
                                    account: app.account().id.clone(),
                                    media,
                                });
                                ui.close();
                            }
                        }
                    });
                    let more = theme::icon_button(
                        ui,
                        Icon::EllipsisVertical,
                        20.0,
                        palette.text,
                        palette.text,
                        &gettext(locale, "More options"),
                    );
                    egui::Popup::menu(&more).show(|ui| {
                        if ui.button(gettext(locale, "Status privacy")).clicked() {
                            app.status_privacy_open = true;
                            ui.close();
                        }
                        if ui.button(gettext(locale, "Search contacts")).clicked() {
                            app.status_search_open = !app.status_search_open;
                            if !app.status_search_open {
                                app.status_search.clear();
                            }
                            ui.close();
                        }
                        if !app.status_hidden.is_empty()
                            && ui.button(gettext(locale, "Show hidden updates")).clicked()
                        {
                            app.status_hidden.clear();
                            ui.close();
                        }
                    });
                });
            });
            if app.status_search_open {
                ui.add_space(12.0);
                ui.add(
                    egui::TextEdit::singleline(&mut app.status_search)
                        .hint_text(gettext(locale, "Search contacts"))
                        .desired_width(f32::INFINITY),
                );
            }
        });
    let me = app.me.clone().unwrap_or_default();
    let mine = app
        .statuses
        .iter()
        .filter(|item| app.is_own_status(item) && app.can_view_status(item))
        .max_by_key(|item| item.timestamp)
        .map(|item| {
            (
                item.sender.clone(),
                item.id.clone(),
                item.timestamp,
                item.content.media().and_then(|media| media.path.clone()),
            )
        });
    let picture = mine
        .as_ref()
        .and_then(|(_, _, _, path)| path.clone())
        .or_else(|| app.avatar(&me));
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 64.0), Sense::click());
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    theme::reveal_focus(&response);
    if response.hovered() || response.has_focus() {
        ui.painter()
            .rect_filled(rect.shrink2(vec2(8.0, 4.0)), 12.0, palette.surface_hover);
    }
    theme::focus_outline(ui, response.id, rect.shrink2(vec2(8.0, 4.0)), 12.0);
    let avatar =
        Rect::from_center_size(pos2(rect.left() + 40.0, rect.center().y), Vec2::splat(40.0));
    widgets::paint_avatar(
        ui,
        &palette,
        avatar,
        &gettext(locale, "My status"),
        &me,
        picture.as_deref(),
    );
    if mine.is_some() {
        let count = app
            .statuses
            .iter()
            .filter(|item| app.is_own_status(item) && app.can_view_status(item))
            .count();
        ring(
            ui,
            avatar.center(),
            23.0,
            count,
            count,
            palette.accent,
            palette.secondary,
        );
    } else {
        let badge = avatar.right_bottom() - vec2(2.0, 2.0);
        ui.painter().circle_filled(badge, 9.0, palette.accent);
        theme::paint_icon(
            ui,
            Icon::Plus,
            Rect::from_center_size(badge, Vec2::splat(14.0)),
            14.0,
            palette.window,
        );
    }
    ui.scope_builder(
        egui::UiBuilder::new().max_rect(Rect::from_min_max(
            pos2(rect.left() + 72.0, rect.top() + 20.0),
            rect.max,
        )),
        |ui| {
            widgets::rich_text(
                ui,
                &gettext(locale, "My status"),
                theme::regular(14.0),
                palette.text,
            );
            let hint = mine
                .as_ref()
                .map(|(_, _, at, _)| crate::util::moment_stamp(locale, *at))
                .unwrap_or_else(|| gettext(locale, "Click to add a status update").into_owned());
            widgets::rich_text(ui, &hint, theme::regular(12.0), palette.secondary);
        },
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, gettext(locale, "My status"))
    });
    if response.clicked() {
        let account = app.account().id.clone();
        app.actions.push(if let Some((sender, id, _, _)) = mine {
            Action::SelectStatus {
                account,
                sender,
                id,
            }
        } else {
            Action::ComposeStatus {
                account,
                media: false,
            }
        });
    }
    ui.add_space(8.0);
    let query = if app.status_search_open {
        app.status_search.to_lowercase()
    } else {
        String::new()
    };
    // Only contact keys and labels are copied; media and message bodies stay in the account.
    let mut posts_by_author: HashMap<&str, Vec<&Item>> = HashMap::new();
    for item in app
        .statuses
        .iter()
        .rev()
        .filter(|item| !app.is_own_status(item) && app.can_view_status(item))
    {
        posts_by_author.entry(&item.sender).or_default().push(item);
    }
    let mut authors = HashSet::new();
    let rows: Vec<_> = app
        .statuses
        .iter()
        .filter(|item| !app.is_own_status(item) && app.can_view_status(item))
        .filter(|item| authors.insert(item.sender.clone()))
        .filter_map(|latest| {
            let name = app.display_name_or(&latest.sender, latest.name.as_deref());
            if !query.is_empty() && !name.to_lowercase().contains(&query) {
                return None;
            }
            let posts = posts_by_author.get(latest.sender.as_str())?;
            let unseen = posts
                .iter()
                .filter(|item| {
                    !app.status_seen
                        .contains(&(item.sender.clone(), item.id.clone()))
                })
                .count();
            let first = posts
                .iter()
                .find(|item| {
                    !app.status_seen
                        .contains(&(item.sender.clone(), item.id.clone()))
                })
                .copied()
                .unwrap_or(latest);
            Some((
                latest.sender.clone(),
                first.id.clone(),
                name,
                latest.timestamp,
                posts.len(),
                unseen,
            ))
        })
        .collect();
    egui::ScrollArea::vertical()
        .id_salt("status-contacts")
        .show(ui, |ui| {
            if rows.is_empty() {
                Frame::new().inner_margin(Margin::same(20)).show(ui, |ui| {
                    ui.label(gettext(locale, "No status updates loaded"));
                    ui.small(gettext(
                        locale,
                        "Updates synchronized from your phone will appear here",
                    ));
                });
            }
            for viewed in [false, true] {
                if !rows.iter().any(|row| (row.5 == 0) == viewed) {
                    continue;
                }
                Frame::new()
                    .inner_margin(Margin::symmetric(20, 12))
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new(if viewed {
                                gettext(locale, "VIEWED")
                            } else {
                                gettext(locale, "RECENT")
                            })
                            .color(palette.secondary)
                            .size(12.0),
                        );
                    });
                for (sender, id, name, at, total, unseen) in
                    rows.iter().filter(|row| (row.5 == 0) == viewed)
                {
                    let (rect, response) =
                        ui.allocate_exact_size(vec2(ui.available_width(), 64.0), Sense::click());
                    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
                    if !ui.is_rect_visible(rect) {
                        continue;
                    }
                    let picture = app.avatar(sender);
                    response.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), name)
                    });
                    if response.hovered() || response.has_focus() {
                        ui.painter().rect_filled(rect, 0.0, palette.surface_hover);
                    }
                    theme::focus_outline(ui, response.id, rect, 0.0);
                    let avatar = Rect::from_center_size(
                        pos2(rect.left() + 40.0, rect.center().y),
                        Vec2::splat(40.0),
                    );
                    widgets::paint_avatar(ui, &palette, avatar, name, sender, picture.as_deref());
                    ring(
                        ui,
                        avatar.center(),
                        23.0,
                        *total,
                        *unseen,
                        palette.accent,
                        palette.secondary,
                    );
                    let text = Rect::from_min_max(
                        pos2(rect.left() + 72.0, rect.top() + 13.0),
                        pos2(rect.right() - 12.0, rect.bottom() - 10.0),
                    );
                    ui.scope_builder(egui::UiBuilder::new().max_rect(text), |ui| {
                        widgets::rich_text(ui, name, theme::regular(15.0), palette.text);
                        ui.add_space(4.0);
                        widgets::rich_text(
                            ui,
                            &crate::util::moment_stamp(locale, *at),
                            theme::regular(12.5),
                            palette.secondary,
                        );
                    });
                    if response.clicked() {
                        app.actions.push(Action::SelectStatus {
                            account: app.account().id.clone(),
                            sender: sender.clone(),
                            id: id.clone(),
                        });
                    }
                }
            }
        });
}

fn ring(
    ui: &egui::Ui,
    center: egui::Pos2,
    radius: f32,
    count: usize,
    unseen: usize,
    accent: Color32,
    seen: Color32,
) {
    let count = count.max(1);
    for part in 0..count {
        let span = std::f32::consts::TAU / count as f32;
        let gap = if count > 1 {
            0.09_f32.min(span * 0.25)
        } else {
            0.0
        };
        let start = -std::f32::consts::FRAC_PI_2 + part as f32 * span + gap;
        let end = start + span - 2.0 * gap;
        let points = (0..=16)
            .map(|step| center + Vec2::angled(start + (end - start) * step as f32 / 16.0) * radius)
            .collect();
        ui.painter().add(egui::Shape::line(
            points,
            Stroke::new(2.5, if part < unseen { accent } else { seen }),
        ));
    }
}

fn button(ui: &mut egui::Ui, icon: Icon, label: &str) -> egui::Response {
    theme::icon_button(
        ui,
        icon,
        20.0,
        Color32::from_gray(210),
        Color32::WHITE,
        label,
    )
}

fn start_button(
    ui: &mut egui::Ui,
    icon: Icon,
    label: &str,
    palette: &theme::Palette,
) -> egui::Response {
    ui.allocate_ui_with_layout(vec2(88.0, 72.0), Layout::top_down(Align::Center), |ui| {
        let (rect, response) = ui.allocate_exact_size(vec2(48.0, 40.0), Sense::click());
        let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
        ui.painter().rect_filled(
            rect,
            20.0,
            if response.hovered() {
                palette.surface_active
            } else {
                palette.surface_hover
            },
        );
        theme::paint_icon(ui, icon, rect, 22.0, palette.text);
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
        theme::focus_outline(ui, response.id, rect, 20.0);
        ui.label(RichText::new(label).size(10.5).color(palette.secondary));
        response
    })
    .inner
}

fn viewer(app: &mut App, ui: &mut egui::Ui, item: &Item) {
    let locale = app.locale;
    let account = app.account().id.clone();
    // Dark in every theme: the controls over the story are drawn in white.
    let canvas = Color32::from_gray(12);
    egui::CentralPanel::default()
        .frame(Frame::new().fill(canvas).inner_margin(Margin::ZERO))
        .show(ui, |ui| {
            header_room(ui);
            let full = ui.available_rect_before_wrap();
            if let Some(bytes) = &item.backdrop {
                let uri = format!("bytes://status-backdrop-{}-{}.png", account, item.id);
                crate::image_cache::include(ui.ctx(), &uri, bytes);
                egui::Image::new(uri)
                    .maintain_aspect_ratio(false)
                    .paint_at(ui, full);
                ui.painter()
                    .rect_filled(full, 0.0, Color32::from_black_alpha(145));
            }
            let top = full.top() + 12.0;
            let h = full.height().max(80.0);
            let aspect = item
                .content
                .media()
                .and_then(|media| Some(media.width? as f32 / media.height?.max(1) as f32))
                .unwrap_or(9.0 / 16.0)
                .clamp(9.0 / 16.0, 0.85);
            let width = (h * aspect)
                .min((full.width() - 24.0).max(140.0))
                .min(620.0);
            let story = Rect::from_center_size(full.center(), vec2(width, h));
            ui.painter().rect_filled(story, 0.0, Color32::BLACK);
            let (shift, opacity) = motion(ui, item, app.status_step);
            let shown = story.translate(vec2(shift, 0.0));
            ui.scope_builder(egui::UiBuilder::new().max_rect(shown), |ui| {
                ui.set_clip_rect(story.intersect(ui.clip_rect()));
                ui.multiply_opacity(opacity);
                content(app, ui, item, shown)
            });
            taps(app, ui, story, &account);
            let reply = Rect::from_center_size(
                pos2(full.center().x, full.bottom() - 30.0),
                vec2((full.width() - 80.0).clamp(180.0, 720.0), 44.0),
            );
            let replies = !app.is_own_status(item) && app.status_replies_allowed();
            if !replies {
                app.status_picker = None;
            }
            let panel = app.status_picker.map(|_| {
                let height = (reply.top() - full.top() - 80.0).clamp(120.0, 300.0);
                Rect::from_min_max(
                    pos2(reply.left(), reply.top() - height),
                    pos2(reply.right(), reply.top()),
                )
            });
            // Clicks between the reply bar's controls, or on the picker's
            // padding, stay there instead of stepping through the stories.
            ui.interact(
                panel.map_or(reply, |panel| panel.union(reply)).expand(8.0),
                ui.id().with("status-reply-backdrop"),
                Sense::CLICK,
            );
            widgets::paint_vertical_gradient(
                ui,
                Rect::from_min_size(story.min, vec2(story.width(), 120.0)),
                Color32::from_black_alpha(150),
                Color32::TRANSPARENT,
            );
            let header_width = (width - 20.0)
                .max(220.0)
                .min((full.width() - 100.0).max(140.0));
            let header =
                Rect::from_center_size(pos2(full.center().x, top + 29.0), vec2(header_width, 58.0));
            let posts: Vec<_> = app
                .statuses
                .iter()
                .rev()
                .filter(|other| other.sender == item.sender && app.can_view_status(other))
                .map(|other| other.id.clone())
                .collect();
            let index = posts.iter().position(|id| *id == item.id).unwrap_or(0);
            let progress = app
                .status_playback
                .as_ref()
                .map_or(0.0, crate::status::Playback::fraction);
            let segment = (header.width() - 4.0 * (posts.len().saturating_sub(1)) as f32)
                / posts.len().max(1) as f32;
            for i in 0..posts.len() {
                let track = Rect::from_min_size(
                    pos2(header.left() + i as f32 * (segment + 4.0), top),
                    vec2(segment, 3.0),
                );
                ui.painter()
                    .rect_filled(track, 2.0, Color32::from_white_alpha(70));
                let fill = if i < index {
                    1.0
                } else if i == index {
                    progress
                } else {
                    0.0
                };
                ui.painter().rect_filled(
                    Rect::from_min_size(track.min, vec2(segment * fill, 3.0)),
                    2.0,
                    Color32::WHITE,
                );
            }
            let name = app.display_name_or(&item.sender, item.name.as_deref());
            let picture = app.avatar(&item.sender);
            ui.scope_builder(
                egui::UiBuilder::new().max_rect(header.translate(vec2(0.0, 13.0))),
                |ui| {
                    ui.horizontal(|ui| {
                        widgets::avatar(
                            ui,
                            &app.palette,
                            &name,
                            &item.sender,
                            36.0,
                            picture.as_deref(),
                        );
                        ui.vertical(|ui| {
                            widgets::rich_text(ui, &name, theme::regular(14.0), Color32::WHITE);
                            widgets::rich_text(
                                ui,
                                &crate::util::moment_stamp(locale, item.timestamp),
                                theme::regular(11.5),
                                Color32::from_gray(190),
                            );
                        });
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let more = button(
                                ui,
                                Icon::EllipsisVertical,
                                &gettext(locale, "More options"),
                            );
                            egui::Popup::menu(&more).show(|ui| {
                                if app.is_own_status(item) {
                                    let available = app.status_deletable.contains(&item.id)
                                        && app.status_delete_pending.is_none();
                                    let unavailable = gettext(
                                        locale,
                                        "Only statuses with their original audience available on this device can be deleted here",
                                    );
                                    if ui
                                        .add_enabled(
                                            available,
                                            egui::Button::new(gettext(locale, "Delete status")),
                                        )
                                        .on_disabled_hover_text(unavailable)
                                        .clicked()
                                    {
                                        app.status_delete_confirm = Some(item.id.clone());
                                        ui.close();
                                    }
                                } else if ui
                                    .button(gettext(locale, "Hide updates from this contact"))
                                    .clicked()
                                {
                                    app.actions.push(Action::HideStatus {
                                        account: account.clone(),
                                        sender: item.sender.clone(),
                                    });
                                    ui.close();
                                }
                            });
                            let paused = app
                                .status_playback
                                .as_ref()
                                .is_some_and(|clock| clock.paused);
                            if button(
                                ui,
                                if paused { Icon::Play } else { Icon::Pause },
                                &if paused {
                                    gettext(locale, "Play")
                                } else {
                                    gettext(locale, "Pause")
                                },
                            )
                            .clicked()
                            {
                                app.actions.push(Action::ToggleStatusPlayback {
                                    account: account.clone(),
                                });
                            }
                            if matches!(item.content, Content::Video { .. })
                                && button(
                                    ui,
                                    if app.video.muted() {
                                        Icon::VolumeX
                                    } else {
                                        Icon::Volume2
                                    },
                                    &gettext(locale, "Mute / unmute"),
                                )
                                .clicked()
                            {
                                app.actions.push(Action::ToggleVideoSound);
                            }
                        });
                    });
                },
            );
            let close = Rect::from_min_size(pos2(full.right() - 48.0, top), vec2(36.0, 36.0));
            ui.scope_builder(egui::UiBuilder::new().max_rect(close), |ui| {
                if button(ui, Icon::X, &gettext(locale, "Close")).clicked() {
                    app.actions.push(Action::CloseStatus {
                        account: account.clone(),
                    });
                }
            });
            let back = Rect::from_min_size(pos2(full.left() + 14.0, top), vec2(36.0, 36.0));
            ui.scope_builder(egui::UiBuilder::new().max_rect(back), |ui| {
                if button(ui, Icon::ArrowLeft, &gettext(locale, "Back")).clicked() {
                    app.actions.push(Action::CloseStatus {
                        account: account.clone(),
                    });
                }
            });
            // Only when nothing else would take the keys: a focused button
            // or field, an open menu, or the reply picker.
            let free = !ui.ctx().text_edit_focused()
                && app.status_picker.is_none()
                && ui.ctx().memory(|memory| memory.focused().is_none())
                && !egui::Popup::is_any_open(ui.ctx());
            let keys = ui.input_mut(|input| {
                (
                    free && input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowLeft),
                    free && input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowRight),
                    free && input.consume_key(egui::Modifiers::NONE, egui::Key::Space),
                )
            });
            if keys.0 {
                app.actions.push(Action::StepStatus {
                    account: account.clone(),
                    direction: -1,
                });
            }
            if keys.1 {
                app.actions.push(Action::StepStatus {
                    account: account.clone(),
                    direction: 1,
                });
            }
            if keys.2 {
                app.actions.push(Action::ToggleStatusPlayback {
                    account: account.clone(),
                });
            }
            // Beside the story rather than at the window's edges, as in
            // WhatsApp; there is nothing to go back to from the first story.
            let has_previous = crate::status::step(
                app.statuses
                    .iter()
                    .filter(|other| app.can_view_status(other)),
                &(item.sender.clone(), item.id.clone()),
                -1,
            )
            .is_some();
            for (direction, x, icon, label) in [
                (
                    -1,
                    (story.left() - 36.0).max(full.left() + 34.0),
                    Icon::ChevronLeft,
                    gettext(locale, "Previous"),
                ),
                (
                    1,
                    (story.right() + 36.0).min(full.right() - 34.0),
                    Icon::ChevronRight,
                    gettext(locale, "Next"),
                ),
            ] {
                if direction < 0 && !has_previous {
                    continue;
                }
                let arrow = Rect::from_center_size(pos2(x, story.center().y), vec2(40.0, 40.0));
                ui.scope_builder(egui::UiBuilder::new().max_rect(arrow), |ui| {
                    if theme::circle_button(
                        ui,
                        icon,
                        40.0,
                        Color32::from_black_alpha(130),
                        Color32::from_black_alpha(190),
                        Color32::WHITE,
                        &label,
                    )
                    .clicked()
                    {
                        app.actions.push(Action::StepStatus {
                            account: account.clone(),
                            direction,
                        });
                    }
                });
            }
            if let Content::Image {
                caption: Some(caption),
                ..
            }
            | Content::Video {
                caption: Some(caption),
                ..
            } = &item.content
            {
                // A few lines at most, resting on the reply bar.
                let line = widgets::line(
                    ui,
                    caption,
                    theme::regular(14.0),
                    Color32::WHITE,
                    (story.width() - 28.0).max(40.0),
                    4,
                );
                let bottom = reply.top().min(story.bottom()) - 14.0;
                let at = pos2(story.center().x - line.size().x / 2.0, bottom - line.size().y);
                widgets::paint_vertical_gradient(
                    ui,
                    Rect::from_min_max(pos2(story.left(), at.y - 60.0), story.max),
                    Color32::TRANSPARENT,
                    Color32::from_black_alpha(200),
                );
                line.paint(ui, at, Color32::WHITE);
            }
            if app.is_own_status(item) {
                let rect = Rect::from_center_size(
                    pos2(full.center().x, full.bottom() - 30.0),
                    vec2(230.0, 40.0),
                );
                ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        let count = app.status_views.get(&item.id).map_or(0, |views| views.len());
                        if button(ui, Icon::Eye, &gettext(locale, "Show status viewers")).clicked()
                        {
                            app.status_viewers_open = true;
                        }
                        let views = gettext(locale, "Views on this device: {count}")
                            .replace("{count}", &count.to_string());
                        ui.label(RichText::new(views).color(Color32::WHITE));
                    });
                });
            } else if !replies {
                ui.scope_builder(
                    egui::UiBuilder::new()
                        .max_rect(reply)
                        .layout(Layout::centered_and_justified(egui::Direction::TopDown)),
                    |ui| {
                        ui.label(
                            RichText::new(gettext(
                                locale,
                                "Replies are off while read receipts are off",
                            ))
                            .color(Color32::from_gray(190)),
                        );
                    },
                );
            } else {
                if let (Some(tab), Some(panel)) = (app.status_picker, panel) {
                    ui.painter().rect_filled(
                        panel.union(reply).expand(8.0),
                        16.0,
                        app.palette.overlay,
                    );
                    let mut body = ui.new_child(
                        egui::UiBuilder::new()
                            .id_salt("status-reply-picker")
                            .max_rect(panel.shrink(10.0))
                            .layout(Layout::top_down(Align::Min)),
                    );
                    body.set_clip_rect(panel);
                    if tab == crate::model::PickerTab::Stickers {
                        if let Some(path) = crate::ui::picker::status_stickers(app, &mut body) {
                            app.actions.push(Action::ReplyStatusSticker {
                                account: account.clone(),
                                path,
                            });
                            app.status_picker = None;
                        }
                    } else if let Some(emoji) =
                        crate::ui::picker::status_emoji(app, &mut body)
                    {
                        app.status_reply.push_str(&emoji);
                    }
                    if ui.input(|input| {
                        input.pointer.any_click()
                            && input
                                .pointer
                                .interact_pos()
                                .is_some_and(|at| !panel.union(reply).expand(8.0).contains(at))
                    }) {
                        app.status_picker = None;
                    }
                }
                ui.scope_builder(egui::UiBuilder::new().max_rect(reply), |ui| {
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        if app.status_picker.is_some()
                            && button(ui, Icon::X, &gettext(locale, "Close picker")).clicked()
                        {
                            app.status_picker = None;
                        }
                        let emoji = button(ui, Icon::Smile, &gettext(locale, "Emoji"));
                        if emoji.clicked() {
                            app.status_picker =
                                if app.status_picker == Some(crate::model::PickerTab::Emoji) {
                                    None
                                } else {
                                    Some(crate::model::PickerTab::Emoji)
                                };
                            if app.status_picker.is_some() {
                                crate::ui::picker::open_status_emoji(ui.ctx());
                            }
                        }
                        let stickers = ui
                            .add_enabled_ui(!app.status_reply_busy, |ui| {
                                button(ui, Icon::Sticker, &gettext(locale, "Stickers"))
                            })
                            .inner;
                        if stickers.clicked() {
                            app.status_picker =
                                if app.status_picker == Some(crate::model::PickerTab::Stickers) {
                                    None
                                } else {
                                    Some(crate::model::PickerTab::Stickers)
                                };
                            if app.status_picker.is_some() {
                                ui.memory_mut(|memory| {
                                    memory.request_focus(egui::Id::new("sticker-search"))
                                });
                                app.actions.push(Action::FetchStatusStickers {
                                    account: account.clone(),
                                });
                            }
                        }
                        let written = !app.status_reply.trim().is_empty();
                        let field_width =
                            (ui.available_width() - if written { 104.0 } else { 64.0 }).max(60.0);
                        let input_palette = app.palette;
                        let busy = app.status_reply_busy;
                        let field = ui.add_enabled(
                            !busy,
                            egui::TextEdit::singleline(&mut app.status_reply)
                                .hint_text(gettext(locale, "Type a reply…"))
                                .frame(input_frame(&input_palette))
                                .font(theme::regular(14.0))
                                .desired_width(field_width)
                                .char_limit(4096),
                        );
                        let enter = field.lost_focus()
                            && ui.input(|input| input.key_pressed(egui::Key::Enter));
                        let send = written
                            && ui
                                .add_enabled_ui(!busy, |ui| {
                                    button(ui, Icon::Send, &gettext(locale, "Send reply"))
                                })
                                .inner
                                .clicked();
                        if send || enter {
                            app.actions.push(Action::ReplyStatus {
                                account: account.clone(),
                                reaction: None,
                            });
                        }
                        let reactions =
                            button(ui, Icon::Heart, &gettext(locale, "React to status"));
                        egui::Popup::menu(&reactions).show(|ui| {
                            ui.horizontal(|ui| {
                                for emoji in ["❤️", "😍", "😂", "😮", "😢", "🙏", "👏", "🎉"]
                                {
                                    let (rect, response) =
                                        ui.allocate_exact_size(Vec2::splat(30.0), Sense::click());
                                    let response =
                                        response.on_hover_cursor(egui::CursorIcon::PointingHand);
                                    crate::emoji::paint_cluster(ui, emoji, rect);
                                    response.widget_info(|| {
                                        egui::WidgetInfo::labeled(
                                            egui::WidgetType::Button,
                                            !busy,
                                            emoji,
                                        )
                                    });
                                    if !busy && response.clicked() {
                                        app.actions.push(Action::ReplyStatus {
                                            account: account.clone(),
                                            reaction: Some(emoji.into()),
                                        });
                                        ui.close();
                                    }
                                }
                            });
                        });
                    });
                });
            }
            ui.allocate_rect(full, Sense::hover());
        });
}

/// Slides a story in from the side the viewer stepped towards, and fades in
/// one opened directly, as WhatsApp's viewer does. Returns the horizontal
/// shift and the opacity for this frame; still when animations are off.
fn motion(ui: &egui::Ui, item: &Item, step: i8) -> (f32, f32) {
    #[derive(Clone)]
    struct Shown {
        key: (String, String),
        started: f64,
        from: f32,
        frame: u64,
    }
    const DURATION: f64 = 0.22;
    let ctx = ui.ctx();
    let id = egui::Id::new("status-story-motion");
    let now = ui.input(|input| input.time);
    let frame = ctx.cumulative_frame_nr();
    let key = (item.sender.clone(), item.id.clone());
    // A gap in frames means the viewer was closed in between: reopening the
    // same story animates again.
    let mut shown = ctx
        .data(|data| data.get_temp::<Shown>(id))
        .filter(|shown| shown.key == key && shown.frame + 1 >= frame)
        .unwrap_or(Shown {
            key,
            started: now,
            from: f32::from(step),
            frame,
        });
    shown.frame = frame;
    let (started, from) = (shown.started, shown.from);
    ctx.data_mut(|data| data.insert_temp(id, shown));
    if ui.style().animation_time <= 0.0 {
        return (0.0, 1.0);
    }
    let t = (((now - started) / DURATION) as f32).clamp(0.0, 1.0);
    if t < 1.0 {
        ctx.request_repaint();
    }
    let eased = 1.0 - (1.0 - t).powi(3);
    (from * 64.0 * (1.0 - eased), eased)
}

/// A click on the story's outer thirds steps back or forward, as a tap does
/// in WhatsApp. A held press only pauses (`tick_status` stops the clock while
/// the button is down), so releasing it does not step.
fn taps(app: &mut App, ui: &mut egui::Ui, story: Rect, account: &crate::model::AccountId) {
    let zone = story.width() * 0.3;
    for (direction, rect) in [
        (
            -1,
            Rect::from_min_max(story.min, pos2(story.left() + zone, story.bottom())),
        ),
        (
            1,
            Rect::from_min_max(pos2(story.right() - zone, story.top()), story.max),
        ),
    ] {
        // Clicks only: the zones are no tab stops of their own, the arrows
        // beside the story and the keys do the same for the keyboard.
        let response = ui.interact(rect, ui.id().with(("status-tap", direction)), Sense::CLICK);
        let pressed_id = response.id.with("pressed");
        let now = ui.input(|input| input.time);
        let pressed = ui.ctx().data(|data| data.get_temp::<f64>(pressed_id));
        // A press and release within one frame never recorded the press.
        if response.clicked() && pressed.is_none_or(|at| now - at < 0.3) {
            app.actions.push(Action::StepStatus {
                account: account.clone(),
                direction,
            });
        }
        if response.is_pointer_button_down_on() {
            if pressed.is_none() {
                ui.ctx().data_mut(|data| data.insert_temp(pressed_id, now));
            }
        } else if pressed.is_some() {
            ui.ctx().data_mut(|data| data.remove::<f64>(pressed_id));
        }
    }
}

fn content(app: &mut App, ui: &mut egui::Ui, item: &Item, story: Rect) {
    let locale = app.locale;
    if let Content::Text { text, .. } = &item.content {
        let fill = item
            .background
            .map_or(Color32::from_rgb(45, 85, 75), compose::argb);
        ui.painter().rect_filled(story, 0.0, fill);
        let color = if fill.intensity() > 0.5 {
            Color32::BLACK
        } else {
            Color32::WHITE
        };
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(story.shrink(28.0))
                .layout(Layout::centered_and_justified(egui::Direction::TopDown)),
            |ui| {
                ui.add(
                    egui::Label::new(
                        RichText::new(text)
                            .font(FontId::proportional(24.0))
                            .color(color),
                    )
                    .wrap()
                    .selectable(true),
                );
            },
        );
        return;
    }
    let Some(media) = item.content.media() else {
        return;
    };
    let key = item.playback_id();
    let texture = app.video.status(&key).and_then(|status| status.frame);
    if let Some(path) = &media.path {
        match &item.content {
            Content::Image { .. } => {
                ui.with_layout(
                    Layout::centered_and_justified(egui::Direction::TopDown),
                    |ui| {
                        ui.add(
                            widgets::file_image(ui, path)
                                .max_size(story.size())
                                .shrink_to_fit()
                                .alt_text(gettext(locale, "Status photo")),
                        );
                    },
                );
            }
            Content::Video { .. } => {
                app.video.saw(&key);
                if let Some(texture) = texture {
                    ui.with_layout(
                        Layout::centered_and_justified(egui::Direction::TopDown),
                        |ui| {
                            ui.add(
                                egui::Image::from_texture(&texture)
                                    .max_size(story.size())
                                    .shrink_to_fit(),
                            );
                        },
                    );
                } else {
                    ui.centered_and_justified(|ui| {
                        ui.spinner();
                    });
                }
            }
            Content::Audio { .. } => {
                ui.centered_and_justified(|ui| {
                    ui.label(
                        RichText::new(gettext(locale, "Voice status"))
                            .color(Color32::WHITE)
                            .size(22.0),
                    );
                });
            }
            _ => {}
        }
    } else {
        ui.centered_and_justified(|ui| {
            ui.vertical_centered(|ui| match &media.state {
                // Idle is a download about to start, not a failure.
                MediaState::Idle | MediaState::Downloading => {
                    ui.spinner();
                    ui.label(
                        RichText::new(gettext(locale, "Loading status…")).color(Color32::WHITE),
                    );
                }
                MediaState::Failed(_) => {
                    ui.label(
                        RichText::new(gettext(locale, "This status could not be loaded"))
                            .color(Color32::WHITE),
                    );
                    if ui.button(gettext(locale, "Retry")).clicked() {
                        app.actions.push(Action::DownloadStatus {
                            account: app.account().id.clone(),
                            sender: item.sender.clone(),
                            id: item.id.clone(),
                        });
                    }
                }
            });
        });
    }
}
