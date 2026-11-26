//! WhatsApp Communities tab, overview pane, header switcher popup, and info card.

use egui::{
    Align, Color32, CornerRadius, Frame, Layout, Margin, Rect, Sense, Stroke, Vec2, pos2, vec2,
};

use crate::app::App;
use crate::i18n::gettext;
use crate::model::{Action, Chat, CommunityInfo, CommunitySubgroup};
use crate::theme::{self, Icon, Palette};

use super::widgets;

/// Accent tint for Announcements badges (emerald/teal).
pub const ANNOUNCEMENTS_COLOR: Color32 = Color32::from_rgb(20, 184, 166);
/// Accent tint for Community badges on chat cards (cyan-indigo).
pub const COMMUNITY_COLOR: Color32 = Color32::from_rgb(56, 189, 248);
/// Extra height added to a chat-list row when it displays a community badge.
pub const CARD_BADGE_ROW_EXTRA: f32 = 20.0;

/// Returns the community or Announcements badge to display on a chat card, if any.
pub fn chat_badge(app: &App, chat: &Chat) -> Option<(String, Color32)> {
    if app.is_community_announcements(chat) {
        Some((
            gettext(app.locale, "Announcements").into_owned(),
            ANNOUNCEMENTS_COLOR,
        ))
    } else {
        app.community_of_chat(chat)
            .map(|community| (community.name.clone(), COMMUNITY_COLOR))
    }
}

/// Paints a compact coloured mini-pill badge for a chat card's community affiliation.
pub fn paint_card_badge(
    ui: &egui::Ui,
    palette: &Palette,
    name: &str,
    color: Color32,
    left: f32,
    right: f32,
    top: f32,
) {
    if name.is_empty() || right <= left {
        return;
    }
    const PILL_HEIGHT: f32 = 18.0;
    const PILL_RADIUS: f32 = PILL_HEIGHT / 2.0;
    const PAD_LEFT: f32 = 6.0;
    const PAD_RIGHT: f32 = 7.0;
    const DOT_RADIUS: f32 = 2.8;
    const DOT_GAP: f32 = 4.5;

    let chrome = PAD_LEFT + DOT_RADIUS * 2.0 + DOT_GAP + PAD_RIGHT;
    let available = right - left;
    let max_text_width = (available - chrome).max(0.0);
    if max_text_width < 8.0 {
        return;
    }
    let text = widgets::line(
        ui,
        name,
        theme::medium(10.5),
        palette.text,
        max_text_width,
        1,
    );
    let pill_width = (chrome + text.size().x).min(available).max(chrome);
    let pill_rect = Rect::from_min_size(pos2(left, top), vec2(pill_width, PILL_HEIGHT));
    let painter = ui.painter();

    painter.rect_filled(pill_rect, PILL_RADIUS, color.gamma_multiply(0.16));
    painter.rect_stroke(
        pill_rect,
        PILL_RADIUS,
        Stroke::new(1.0, color.gamma_multiply(0.38)),
        egui::StrokeKind::Inside,
    );
    painter.circle_filled(
        pos2(
            pill_rect.left() + PAD_LEFT + DOT_RADIUS,
            pill_rect.center().y,
        ),
        DOT_RADIUS,
        color,
    );
    text.paint(
        ui,
        pos2(
            pill_rect.left() + PAD_LEFT + DOT_RADIUS * 2.0 + DOT_GAP,
            pill_rect.center().y - text.size().y / 2.0,
        ),
        palette.text,
    );
}

/// Left-pane list of all participating communities and their key groups.
pub fn list(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    if app.communities.is_empty() {
        if app.communities_loading {
            widgets::empty_state(
                ui,
                &palette,
                Icon::Users,
                "Loading communities",
                "Fetching your WhatsApp Communities and subgroups…",
            );
        } else {
            widgets::empty_state(
                ui,
                &palette,
                Icon::Users,
                "No communities yet",
                "Communities you participate in on WhatsApp appear here.",
            );
        }
        return;
    }

    let needle = crate::util::search_key(app.communities_search.trim());
    let communities: Vec<CommunityInfo> = app
        .communities
        .iter()
        .filter(|community| {
            if needle.is_empty() {
                return true;
            }
            app.name_matches(&community.name, &needle)
                || community
                    .subgroups
                    .iter()
                    .any(|sg| app.name_matches(&sg.name, &needle))
        })
        .cloned()
        .collect();

    if communities.is_empty() {
        widgets::empty_state(
            ui,
            &palette,
            Icon::Search,
            "No matching communities",
            "Try another community or group name.",
        );
        return;
    }

    egui::ScrollArea::vertical()
        .id_salt("communities-list")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            Frame::new()
                .inner_margin(Margin::symmetric(10, 6))
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 10.0;
                    for community in &communities {
                        community_card(app, ui, &palette, community, &needle);
                    }
                    ui.add_space(8.0);
                });
        });
}

fn community_card(
    app: &mut App,
    ui: &mut egui::Ui,
    palette: &Palette,
    community: &CommunityInfo,
    needle: &str,
) {
    let selected =
        app.selected_community.as_deref() == Some(community.id.as_str()) && app.open_chat.is_none();
    let fill = if selected {
        palette.surface_active
    } else {
        palette.surface
    };

    Frame::new()
        .fill(fill)
        .stroke(Stroke::new(1.0, palette.outline))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::symmetric(8, 8))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;

            // Community header row
            let (header_rect, header_resp) =
                ui.allocate_exact_size(vec2(ui.available_width(), 48.0), Sense::click());
            if ui.is_rect_visible(header_rect) {
                if header_resp.hovered() {
                    ui.painter().rect_filled(
                        header_rect,
                        CornerRadius::same(8),
                        palette.surface_hover,
                    );
                }
                let avatar_rect = Rect::from_center_size(
                    pos2(header_rect.left() + 24.0, header_rect.center().y),
                    Vec2::splat(36.0),
                );
                let picture = app.avatar(&community.id);
                widgets::paint_avatar(
                    ui,
                    palette,
                    avatar_rect,
                    &community.name,
                    &community.id,
                    picture.as_deref(),
                );
                let text_left = header_rect.left() + 50.0;
                let unread = app.community_unread(community);
                let right_reserve = if unread > 0 { 28.0 } else { 18.0 };
                let text_width = (header_rect.right() - text_left - right_reserve).max(40.0);
                let title = widgets::line(
                    ui,
                    &community.name,
                    theme::semibold(13.5),
                    palette.text,
                    text_width,
                    1,
                );
                title.paint(ui, pos2(text_left, header_rect.top() + 7.0), palette.text);

                let group_count = community.subgroups.len();
                let meta = match (group_count, community.participant_count) {
                    (g, Some(m)) if g > 0 => format!("{g} groups · {m} members"),
                    (g, None) if g > 0 => format!("{g} groups"),
                    (_, Some(m)) => format!("{m} members"),
                    _ => "Community".to_owned(),
                };
                let meta_line =
                    widgets::line(ui, &meta, theme::regular(11.5), palette.dim, text_width, 1);
                meta_line.paint(ui, pos2(text_left, header_rect.top() + 26.0), palette.dim);

                if unread > 0 {
                    widgets::unread_indicator(
                        ui,
                        palette,
                        pos2(header_rect.right() - 14.0, header_rect.center().y),
                        u32::try_from(unread).unwrap_or(u32::MAX),
                        false,
                        false,
                    );
                } else {
                    let chev = Rect::from_center_size(
                        pos2(header_rect.right() - 10.0, header_rect.center().y),
                        Vec2::splat(14.0),
                    );
                    Icon::ChevronRight
                        .image(palette.dim, 14.0)
                        .paint_at(ui, chev);
                }
            }
            if header_resp
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked()
            {
                app.actions
                    .push(Action::OpenCommunity(community.id.clone()));
            }

            // Announcements row pinned right under the community header
            if let Some(announce) = community.announcements() {
                ui.add_space(2.0);
                subgroup_compact_row(app, ui, palette, community, announce, true);
            }

            // Up to 3 most-recent joined subgroups (or matching search)
            let mut joined: Vec<&CommunitySubgroup> = community
                .subgroups
                .iter()
                .filter(|sg| !sg.is_default_sub_group)
                .filter(|sg| app.chat(&sg.id).is_some())
                .filter(|sg| needle.is_empty() || app.name_matches(&sg.name, needle))
                .collect();
            joined.sort_by_key(|sg| {
                std::cmp::Reverse(app.chat(&sg.id).map_or(0, |chat| chat.last_activity))
            });
            for sg in joined.iter().take(3) {
                subgroup_compact_row(app, ui, palette, community, sg, false);
            }

            // "View all" footer button
            let total = community.subgroups.len();
            if total > 0 {
                ui.add_space(2.0);
                let (view_rect, view_resp) =
                    ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::click());
                if ui.is_rect_visible(view_rect) {
                    if view_resp.hovered() {
                        ui.painter().rect_filled(
                            view_rect,
                            CornerRadius::same(6),
                            palette.surface_hover,
                        );
                    }
                    let label = format!("View all ({total})");
                    let galley =
                        ui.painter()
                            .layout_no_wrap(label, theme::medium(12.0), palette.accent);
                    ui.painter().galley(
                        pos2(
                            view_rect.left() + 10.0,
                            view_rect.center().y - galley.size().y / 2.0,
                        ),
                        galley,
                        palette.accent,
                    );
                    let chev = Rect::from_center_size(
                        pos2(view_rect.right() - 10.0, view_rect.center().y),
                        Vec2::splat(13.0),
                    );
                    Icon::ChevronRight
                        .image(palette.accent, 13.0)
                        .paint_at(ui, chev);
                }
                if view_resp
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .clicked()
                {
                    app.actions
                        .push(Action::OpenCommunity(community.id.clone()));
                }
            }
        });
}

fn subgroup_compact_row(
    app: &mut App,
    ui: &mut egui::Ui,
    palette: &Palette,
    community: &CommunityInfo,
    sg: &CommunitySubgroup,
    is_announcements: bool,
) {
    let active = app.open_chat.as_deref() == Some(sg.id.as_str());
    let chat = app.chat(&sg.id).cloned();
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 44.0), Sense::click());
    if ui.is_rect_visible(rect) {
        if active {
            ui.painter()
                .rect_filled(rect, CornerRadius::same(8), palette.surface_active);
        } else if resp.hovered() {
            ui.painter()
                .rect_filled(rect, CornerRadius::same(8), palette.surface_hover);
        }

        let icon_center = pos2(rect.left() + 24.0, rect.center().y);
        if is_announcements {
            let badge_rect = Rect::from_center_size(icon_center, Vec2::splat(32.0));
            ui.painter().rect_filled(
                badge_rect,
                CornerRadius::same(8),
                ANNOUNCEMENTS_COLOR.gamma_multiply(0.20),
            );
            let inner = Rect::from_center_size(icon_center, Vec2::splat(16.0));
            Icon::Bell
                .image(ANNOUNCEMENTS_COLOR, 16.0)
                .paint_at(ui, inner);
        } else {
            let avatar_rect = Rect::from_center_size(icon_center, Vec2::splat(32.0));
            let picture = app.avatar(&sg.id);
            widgets::paint_avatar(
                ui,
                palette,
                avatar_rect,
                &sg.name,
                &sg.id,
                picture.as_deref(),
            );
        }

        let left = rect.left() + 48.0;
        let right = rect.right() - 8.0;

        let stamp = chat
            .as_ref()
            .filter(|c| c.last_activity > 0)
            .map(|c| crate::util::chat_stamp(app.locale, c.last_activity))
            .unwrap_or_default();
        let unread = chat.as_ref().is_some_and(|c| c.looks_unread());
        let stamp_color = if unread { palette.accent } else { palette.dim };
        let stamp_galley = ui
            .painter()
            .layout_no_wrap(stamp, theme::regular(11.0), stamp_color);
        if !stamp_galley.is_empty() {
            ui.painter().galley(
                pos2(right - stamp_galley.size().x, rect.top() + 7.0),
                stamp_galley.clone(),
                stamp_color,
            );
        }

        let title_text = if is_announcements {
            "Announcements"
        } else {
            sg.name.as_str()
        };
        let title_width = (right - stamp_galley.size().x - 6.0 - left).max(40.0);
        let title_font = if unread {
            theme::semibold(13.0)
        } else {
            theme::medium(13.0)
        };
        let title_line = widgets::line(ui, title_text, title_font, palette.text, title_width, 1);
        title_line.paint(ui, pos2(left, rect.top() + 6.0), palette.text);

        let mut badge_right = right;
        if let Some(chat_ref) = &chat
            && chat_ref.looks_unread()
        {
            let width = widgets::unread_indicator(
                ui,
                palette,
                pos2(badge_right - 10.0, rect.top() + 31.0),
                chat_ref.unread,
                chat_ref.marked_unread,
                chat_ref.muted(crate::util::now()),
            );
            badge_right -= width + 6.0;
        }

        let preview_text = if let Some(chat_ref) = &chat {
            if let Some(last) = &chat_ref.last {
                crate::markup::plain(
                    &app.resolve_mention_tokens(&last.summary_in(app.locale)),
                    &[],
                )
            } else {
                String::new()
            }
        } else {
            sg.participant_count
                .map(|n| format!("{n} members"))
                .unwrap_or_else(|| "Not joined".to_owned())
        };
        if !preview_text.is_empty() {
            let preview_color = if unread {
                palette.secondary
            } else {
                palette.dim
            };
            let preview_line = widgets::line(
                ui,
                &preview_text,
                theme::regular(11.5),
                preview_color,
                (badge_right - left).max(20.0),
                1,
            );
            preview_line.paint(ui, pos2(left, rect.top() + 24.0), preview_color);
        }
    }

    if resp
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
    {
        if chat.is_some() {
            app.actions.push(Action::OpenChat(sg.id.clone()));
        } else {
            app.actions.push(Action::JoinCommunitySubgroup {
                community: community.id.clone(),
                subgroup: sg.id.clone(),
            });
        }
    }
}

/// Central-pane overview for the Communities tab when no chat is open.
pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    let selected = app
        .selected_community
        .as_deref()
        .and_then(|id| app.community(id))
        .cloned();

    let Some(community) = selected else {
        widgets::empty_state(
            ui,
            &palette,
            Icon::Users,
            "Communities",
            "Choose a community on the left to inspect its announcements and groups.",
        );
        return;
    };

    egui::ScrollArea::vertical()
        .id_salt(("community-overview", &community.id))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            Frame::new()
                .inner_margin(Margin::symmetric(24, 24))
                .show(ui, |ui| {
                    let content_width = ui.available_width().clamp(240.0, 640.0);
                    ui.set_width(content_width);
                    ui.set_max_width(content_width);
                    ui.spacing_mut().item_spacing.y = 14.0;

                    // Hero header
                    ui.horizontal(|ui| {
                        let picture = app
                            .avatar_full(&community.id)
                            .or_else(|| app.avatar(&community.id));
                        widgets::avatar(
                            ui,
                            &palette,
                            &community.name,
                            &community.id,
                            64.0,
                            picture.as_deref(),
                        );
                        ui.add_space(12.0);
                        ui.vertical(|ui| {
                            theme::text(ui, &community.name, theme::bold(20.0), palette.text);
                            let mut meta = vec!["Community".to_owned()];
                            meta.push(format!("{} groups", community.subgroups.len()));
                            if let Some(members) = community.participant_count {
                                meta.push(format!("{members} members"));
                            }
                            theme::text(
                                ui,
                                meta.join(" · "),
                                theme::regular(13.0),
                                palette.secondary,
                            );
                        });
                    });

                    // Action bar
                    ui.horizontal(|ui| {
                        if let Some(announce) = community.announcements() {
                            let has_chat = app.chat(&announce.id).is_some();
                            if theme::pill_button(ui, &palette, "Open Announcements", true)
                                .clicked()
                            {
                                if has_chat {
                                    app.actions.push(Action::OpenChat(announce.id.clone()));
                                } else {
                                    app.actions.push(Action::JoinCommunitySubgroup {
                                        community: community.id.clone(),
                                        subgroup: announce.id.clone(),
                                    });
                                }
                            }
                        }
                        if theme::pill_button(
                            ui,
                            &palette,
                            if app.communities_loading {
                                "Refreshing…"
                            } else {
                                "Refresh"
                            },
                            false,
                        )
                        .clicked()
                        {
                            app.actions.push(Action::RefreshCommunities);
                        }
                    });

                    // Description
                    if let Some(description) = community.description.as_deref() {
                        Frame::new()
                            .fill(palette.surface)
                            .stroke(Stroke::new(1.0, palette.outline))
                            .corner_radius(CornerRadius::same(10))
                            .inner_margin(Margin::symmetric(14, 12))
                            .show(ui, |ui| {
                                theme::paragraph(
                                    ui,
                                    description,
                                    theme::regular(13.5),
                                    palette.secondary,
                                );
                            });
                    }

                    // Joined groups
                    let joined: Vec<&CommunitySubgroup> = community
                        .subgroups
                        .iter()
                        .filter(|sg| app.chat(&sg.id).is_some())
                        .collect();
                    let unjoined: Vec<&CommunitySubgroup> = community
                        .subgroups
                        .iter()
                        .filter(|sg| app.chat(&sg.id).is_none())
                        .collect();

                    if !joined.is_empty() {
                        theme::text(
                            ui,
                            format!("Groups you're in ({})", joined.len()),
                            theme::semibold(13.5),
                            palette.secondary,
                        );
                        Frame::new()
                            .fill(palette.surface)
                            .stroke(Stroke::new(1.0, palette.outline))
                            .corner_radius(CornerRadius::same(12))
                            .inner_margin(Margin::symmetric(8, 6))
                            .show(ui, |ui| {
                                for sg in joined {
                                    subgroup_compact_row(
                                        app,
                                        ui,
                                        &palette,
                                        &community,
                                        sg,
                                        sg.is_default_sub_group,
                                    );
                                }
                            });
                    }

                    if !unjoined.is_empty() {
                        theme::text(
                            ui,
                            format!("Groups you can join ({})", unjoined.len()),
                            theme::semibold(13.5),
                            palette.secondary,
                        );
                        Frame::new()
                            .fill(palette.surface)
                            .stroke(Stroke::new(1.0, palette.outline))
                            .corner_radius(CornerRadius::same(12))
                            .inner_margin(Margin::symmetric(10, 8))
                            .show(ui, |ui| {
                                ui.spacing_mut().item_spacing.y = 6.0;
                                for sg in unjoined {
                                    unjoined_subgroup_row(app, ui, &palette, &community, sg);
                                }
                            });
                    }
                });
        });
}

fn unjoined_subgroup_row(
    app: &mut App,
    ui: &mut egui::Ui,
    palette: &Palette,
    community: &CommunityInfo,
    sg: &CommunitySubgroup,
) {
    let joining = app.joining_subgroups.contains(&sg.id);
    ui.horizontal(|ui| {
        let picture = app.avatar(&sg.id);
        widgets::avatar(ui, palette, &sg.name, &sg.id, 34.0, picture.as_deref());
        ui.add_space(6.0);
        ui.vertical(|ui| {
            theme::text(ui, &sg.name, theme::medium(13.5), palette.text);
            if let Some(count) = sg.participant_count {
                theme::text(
                    ui,
                    format!("{count} members"),
                    theme::regular(11.5),
                    palette.dim,
                );
            }
        });
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let button = theme::pill_button(
                ui,
                palette,
                if joining { "Joining…" } else { "Join" },
                !joining,
            );
            if !joining && button.clicked() {
                app.actions.push(Action::JoinCommunitySubgroup {
                    community: community.id.clone(),
                    subgroup: sg.id.clone(),
                });
            }
        });
    });
}

/// Quick switcher popup shown from the conversation header of any community group.
pub fn switcher_popup(
    app: &mut App,
    ui: &mut egui::Ui,
    community: &CommunityInfo,
    active_chat_id: &str,
) {
    let palette = app.palette;
    ui.spacing_mut().item_spacing.y = 4.0;

    // Header
    ui.horizontal(|ui| {
        let picture = app.avatar(&community.id);
        widgets::avatar(
            ui,
            &palette,
            &community.name,
            &community.id,
            28.0,
            picture.as_deref(),
        );
        ui.add_space(6.0);
        ui.vertical(|ui| {
            let name = widgets::line(
                ui,
                &community.name,
                theme::semibold(13.0),
                palette.text,
                220.0,
                1,
            );
            let (rect, _) = ui.allocate_exact_size(name.size(), Sense::hover());
            name.paint(ui, rect.min, palette.text);
            theme::text(
                ui,
                format!("Community · {} groups", community.subgroups.len()),
                theme::regular(11.0),
                palette.dim,
            );
        });
    });

    widgets::menu_separator(ui, &palette);

    egui::ScrollArea::vertical()
        .id_salt(("community-switcher", &community.id))
        .max_height(320.0)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            for sg in &community.subgroups {
                let is_active = sg.id == active_chat_id;
                let joined = app.chat(&sg.id).is_some();
                let (rect, resp) =
                    ui.allocate_exact_size(vec2(ui.available_width(), 42.0), Sense::click());
                if ui.is_rect_visible(rect) {
                    if is_active {
                        ui.painter().rect_filled(
                            rect,
                            CornerRadius::same(8),
                            palette.surface_active,
                        );
                    } else if resp.hovered() {
                        ui.painter().rect_filled(
                            rect,
                            CornerRadius::same(8),
                            palette.surface_hover,
                        );
                    }
                    let center = pos2(rect.left() + 20.0, rect.center().y);
                    if sg.is_default_sub_group {
                        let badge = Rect::from_center_size(center, Vec2::splat(28.0));
                        ui.painter().rect_filled(
                            badge,
                            CornerRadius::same(7),
                            ANNOUNCEMENTS_COLOR.gamma_multiply(0.20),
                        );
                        Icon::Bell
                            .image(ANNOUNCEMENTS_COLOR, 14.0)
                            .paint_at(ui, Rect::from_center_size(center, Vec2::splat(14.0)));
                    } else {
                        let picture = app.avatar(&sg.id);
                        widgets::paint_avatar(
                            ui,
                            &palette,
                            Rect::from_center_size(center, Vec2::splat(28.0)),
                            &sg.name,
                            &sg.id,
                            picture.as_deref(),
                        );
                    }
                    let left = rect.left() + 42.0;
                    let right = rect.right() - 8.0;
                    let chat = app.chat(&sg.id);
                    let mut badge_right = right;
                    if let Some(c) = chat
                        && c.looks_unread()
                    {
                        let w = widgets::unread_indicator(
                            ui,
                            &palette,
                            pos2(badge_right - 10.0, rect.center().y),
                            c.unread,
                            c.marked_unread,
                            c.muted(crate::util::now()),
                        );
                        badge_right -= w + 6.0;
                    } else if !joined {
                        let tag = ui.painter().layout_no_wrap(
                            "Join".to_owned(),
                            theme::medium(11.0),
                            palette.accent,
                        );
                        ui.painter().galley(
                            pos2(
                                badge_right - tag.size().x,
                                rect.center().y - tag.size().y / 2.0,
                            ),
                            tag.clone(),
                            palette.accent,
                        );
                        badge_right -= tag.size().x + 8.0;
                    }

                    let title_text = if sg.is_default_sub_group {
                        "Announcements"
                    } else {
                        sg.name.as_str()
                    };
                    let title = widgets::line(
                        ui,
                        title_text,
                        theme::medium(12.5),
                        palette.text,
                        (badge_right - left).max(30.0),
                        1,
                    );
                    title.paint(ui, pos2(left, rect.top() + 5.0), palette.text);

                    let sub = if let Some(c) = chat {
                        c.last
                            .as_ref()
                            .map(|last| {
                                crate::markup::plain(
                                    &app.resolve_mention_tokens(&last.summary_in(app.locale)),
                                    &[],
                                )
                            })
                            .unwrap_or_default()
                    } else {
                        sg.participant_count
                            .map(|n| format!("{n} members"))
                            .unwrap_or_else(|| "Not joined".to_owned())
                    };
                    if !sub.is_empty() {
                        let sub_line = widgets::line(
                            ui,
                            &sub,
                            theme::regular(11.0),
                            palette.dim,
                            (badge_right - left).max(30.0),
                            1,
                        );
                        sub_line.paint(ui, pos2(left, rect.top() + 22.0), palette.dim);
                    }
                }
                if resp
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .clicked()
                {
                    if joined {
                        app.actions.push(Action::OpenChat(sg.id.clone()));
                    } else {
                        app.actions.push(Action::JoinCommunitySubgroup {
                            community: community.id.clone(),
                            subgroup: sg.id.clone(),
                        });
                    }
                    ui.close();
                }
            }
        });

    widgets::menu_separator(ui, &palette);
    if widgets::menu_item(ui, &palette, Some(Icon::Users), "View community") {
        app.actions
            .push(Action::OpenCommunity(community.id.clone()));
        ui.close();
    }
}

/// Community card rendered inside the Group Info / Community Info right panel.
pub fn info_section(
    app: &mut App,
    ui: &mut egui::Ui,
    community: &CommunityInfo,
    current_chat_id: &str,
    is_announcements: bool,
) {
    let palette = app.palette;
    ui.add_space(6.0);
    theme::text(ui, "Community", theme::medium(12.5), palette.secondary);
    Frame::new()
        .fill(palette.surface)
        .stroke(Stroke::new(1.0, palette.outline))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::symmetric(10, 10))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            ui.horizontal(|ui| {
                let picture = app.avatar(&community.id);
                widgets::avatar(
                    ui,
                    &palette,
                    &community.name,
                    &community.id,
                    36.0,
                    picture.as_deref(),
                );
                ui.add_space(6.0);
                ui.vertical(|ui| {
                    theme::text(ui, &community.name, theme::semibold(13.5), palette.text);
                    let meta = match community.participant_count {
                        Some(m) => format!("{} groups · {m} members", community.subgroups.len()),
                        None => format!("{} groups", community.subgroups.len()),
                    };
                    theme::text(ui, &meta, theme::regular(11.5), palette.dim);
                });
            });

            if let Some(desc) = community.description.as_deref() {
                theme::paragraph(ui, desc, theme::regular(12.0), palette.secondary);
            }

            if !is_announcements
                && let Some(announce) = community.announcements()
                && widgets::menu_item(ui, &palette, Some(Icon::Bell), "Open Announcements")
            {
                if app.chat(&announce.id).is_some() {
                    app.actions.push(Action::OpenChat(announce.id.clone()));
                } else {
                    app.actions.push(Action::JoinCommunitySubgroup {
                        community: community.id.clone(),
                        subgroup: announce.id.clone(),
                    });
                }
            }

            // Show other groups in this community (up to 6 directly in info panel)
            let others: Vec<&CommunitySubgroup> = community
                .subgroups
                .iter()
                .filter(|sg| sg.id != current_chat_id && !sg.is_default_sub_group)
                .take(6)
                .collect();
            if !others.is_empty() {
                widgets::menu_separator(ui, &palette);
                theme::text(
                    ui,
                    format!(
                        "Other groups ({})",
                        community.subgroups.len().saturating_sub(1)
                    ),
                    theme::medium(11.5),
                    palette.dim,
                );
                for sg in others {
                    let joined = app.chat(&sg.id).is_some();
                    let label = if joined {
                        sg.name.clone()
                    } else {
                        format!("{} (Join)", sg.name)
                    };
                    if widgets::menu_item(ui, &palette, Some(Icon::Users), &label) {
                        if joined {
                            app.actions.push(Action::OpenChat(sg.id.clone()));
                        } else {
                            app.actions.push(Action::JoinCommunitySubgroup {
                                community: community.id.clone(),
                                subgroup: sg.id.clone(),
                            });
                        }
                    }
                }
            }

            widgets::menu_separator(ui, &palette);
            if widgets::menu_item(ui, &palette, Some(Icon::ChevronRight), "View community") {
                app.actions
                    .push(Action::OpenCommunity(community.id.clone()));
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Chat, Page};
    use crate::paths::AppDirs;
    use crate::settings::Settings;

    #[test]
    fn community_subgroups_get_card_badges_and_parent_is_hidden_from_chats() {
        let directory = tempfile::tempdir().unwrap();
        let (mut app, _events) =
            App::headless(AppDirs::under(directory.path()), Settings::default());

        app.communities = vec![CommunityInfo {
            id: "comm@g.us".into(),
            name: "Rust Berlin Community".into(),
            description: Some("Community description".into()),
            participant_count: Some(43),
            subgroups: vec![
                CommunitySubgroup {
                    id: "announce@g.us".into(),
                    name: "Rust Berlin Community".into(),
                    participant_count: Some(43),
                    is_default_sub_group: true,
                    is_general_chat: false,
                    is_hidden_group: false,
                },
                CommunitySubgroup {
                    id: "sub1@g.us".into(),
                    name: "Rust Berlin".into(),
                    participant_count: Some(12),
                    is_default_sub_group: false,
                    is_general_chat: false,
                    is_hidden_group: false,
                },
            ],
        }];

        let mut parent = Chat::new("comm@g.us".into(), "Rust Berlin Community".into());
        parent.is_parent_group = true;
        parent.last_activity = 300;

        let mut announce = Chat::new("announce@g.us".into(), "Rust Berlin Community".into());
        announce.parent_group_jid = Some("comm@g.us".into());
        announce.is_default_sub_group = true;
        announce.last_activity = 200;

        let mut sub1 = Chat::new("sub1@g.us".into(), "Rust Berlin".into());
        sub1.parent_group_jid = Some("comm@g.us".into());
        sub1.last_activity = 100;

        app.chats = vec![parent.clone(), announce.clone(), sub1.clone()];

        let visible_ids: Vec<&str> = app
            .visible_chats()
            .into_iter()
            .map(|c| c.id.as_str())
            .collect();
        assert_eq!(visible_ids, vec!["announce@g.us", "sub1@g.us"]);

        assert_eq!(
            chat_badge(&app, &announce).map(|(name, _)| name),
            Some("Announcements".to_owned())
        );
        assert_eq!(
            chat_badge(&app, &sub1).map(|(name, _)| name),
            Some("Rust Berlin Community".to_owned())
        );

        // Render Communities tab list & overview without panicking
        app.page = Page::Communities;
        app.selected_community = Some("comm@g.us".into());
        let ctx = egui::Context::default();
        app.attach(&ctx);
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(1000.0, 700.0))),
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            crate::ui::show(&mut app, ui);
        });
        output.textures_delta.clear();
    }
}
