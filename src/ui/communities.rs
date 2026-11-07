//! Expandable community headings inside the regular Chats list.
use super::widgets;
use crate::app::App;
use crate::i18n::{gettext, ngettext};
use crate::model::{Action, Chat};
use crate::theme::{self, Icon};
use egui::{Rect, Sense, Vec2, pos2, vec2};

pub fn row(app: &mut App, ui: &mut egui::Ui, chat: &Chat) -> egui::Response {
    let palette = app.palette;
    let expanded = !app.collapsed_communities.contains(&chat.id);
    let children: Vec<_> = app
        .chats
        .iter()
        .filter(|child| {
            !child.locked
                && !child.left
                && !child.archived
                && !child.pinned
                && child
                    .community
                    .as_ref()
                    .and_then(|info| info.parent.as_deref())
                    == Some(chat.id.as_str())
        })
        .collect();
    let unread = children
        .iter()
        .fold(0u32, |count, child| count.saturating_add(child.unread));
    let muted = children
        .iter()
        .filter(|child| child.looks_unread())
        .all(|child| child.muted(crate::util::now()));
    let count = children.len();
    let summary = if count == 0 {
        gettext(app.locale, "Groups are syncing…").into_owned()
    } else {
        ngettext(
            app.locale,
            "{} group",
            "{} groups",
            u32::try_from(count).unwrap_or(u32::MAX),
        )
        .replace("{}", &count.to_string())
    };
    let title = app.chat_title(chat);
    let (rect, response) = ui.allocate_exact_size(
        vec2(ui.available_width(), theme::ROW_HEIGHT),
        Sense::click(),
    );
    theme::reveal_focus(&response);
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::SelectableLabel,
            ui.is_enabled(),
            expanded,
            format!(
                "{title}, {summary}, {}",
                if expanded {
                    gettext(app.locale, "Expanded")
                } else {
                    gettext(app.locale, "Collapsed")
                }
            ),
        )
    });
    if ui.is_rect_visible(rect) {
        if response.hovered() {
            widgets::row_highlight(ui, &palette, rect, palette.surface_hover);
        }
        let avatar =
            Rect::from_center_size(pos2(rect.left() + 38.0, rect.center().y), Vec2::splat(48.0));
        let picture = app.avatar(&chat.id);
        widgets::paint_avatar(ui, &palette, avatar, &title, &chat.id, picture.as_deref());
        let line = widgets::line(
            ui,
            &title,
            theme::semibold(14.5),
            palette.text,
            rect.width() - 110.0,
            1,
        );
        line.paint(
            ui,
            pos2(rect.left() + 76.0, rect.top() + 14.0),
            palette.text,
        );
        let line = widgets::line(
            ui,
            &summary,
            theme::regular(13.0),
            palette.secondary,
            rect.width() - 125.0,
            1,
        );
        line.paint(
            ui,
            pos2(rect.left() + 76.0, rect.top() + 38.0),
            palette.secondary,
        );
        let arrow = Rect::from_center_size(
            pos2(rect.right() - 22.0, rect.top() + 23.0),
            Vec2::splat(18.0),
        );
        theme::paint_icon(
            ui,
            if expanded {
                Icon::ChevronDown
            } else {
                Icon::ChevronRight
            },
            arrow,
            18.0,
            palette.secondary,
        );
        if unread > 0 {
            widgets::unread_indicator(
                ui,
                &palette,
                pos2(rect.right() - 24.0, rect.top() + 46.0),
                unread,
                false,
                muted,
            );
        }
    }
    if response.clicked() {
        app.actions.push(Action::ToggleCommunity(chat.id.clone()));
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}
