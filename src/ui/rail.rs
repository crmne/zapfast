//! The navigation rail on the left of the chat list, as WhatsApp's desktop
//! clients have it: Chats and Calls at the top; Starred, Archived and Locked
//! chats (while there are any), Settings and our own avatar (the account
//! switcher) at the bottom. While the chat list is hidden, the chats'
//! avatars fill the rail between the two, so every chat is still one click
//! away. Beside the list they would only repeat it.

use egui::{Frame, Margin, Rect, Sense, Vec2, vec2};

use crate::app::App;
use crate::i18n::gettext;
use crate::model::{Action, Page};
use crate::theme::{self, Icon, Palette};

use super::focus::{Stop, TabStop};
use super::widgets;

/// Width of the rail, on every platform.
pub const WIDTH: f32 = 52.0;
/// Height of one destination at the top: the list header's title and
/// search field sit level with Chats and Calls.
const CELL: f32 = 48.0;
/// The lit square behind a destination's icon.
const PLATE: f32 = 36.0;
/// A destination's icon.
const ICON: f32 = 20.0;
/// Our avatar at the foot of the rail.
const AVATAR: f32 = 32.0;
/// Space above the first destination, as the list header's own margin, and
/// between the bottom group's plates, the avatar and the window's edge.
const GAP: f32 = 8.0;
/// The least height the chats' avatars take. If the rail cannot fit this
/// and both groups, the whole rail scrolls below the window buttons.
const MIN_AVATARS: f32 = 64.0;
/// Width of the line between the destinations and the chats' avatars.
const DIVIDER: f32 = 24.0;
/// How far below the window's top the macOS traffic lights end, centred as
/// they are in the 60-point title bar, with a little room under them.
const LIGHTS_CLEARANCE: f32 = theme::TOP_BAR_HEIGHT / 2.0 + 12.0;

/// Where the rail was laid out, for layout tests.
pub(crate) fn rect_id() -> egui::Id {
    egui::Id::new("nav-rail")
}

/// Stable id of a destination's button, for tests and the demo.
pub fn destination_id(destination: Destination) -> egui::Id {
    egui::Id::new(("nav-rail-destination", destination as u8))
}

/// How far the macOS traffic lights reach past the rail: what a header
/// beside the rail leaves free at its left. Zero elsewhere and in full
/// screen. The caption buttons ZapFast draws on the left (GNOME) are not
/// counted here: headers measure their room from `ui::caption_room`.
pub fn lights_overhang(ctx: &egui::Context) -> f32 {
    if !theme::macos_chrome(ctx) {
        return 0.0;
    }
    (theme::traffic_light_inset(ctx) - WIDTH).max(0.0)
}

/// A place the rail leads to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Destination {
    Chats,
    Calls,
    Starred,
    Archived,
    Locked,
    Settings,
}

impl Destination {
    /// Whether the window shows this destination now: its icon is lit.
    pub fn lit(self, app: &App) -> bool {
        let chats = app.page == Page::Chats && !app.show_starred;
        match self {
            Self::Chats => chats && !app.show_archived && !app.locked_folder_open(),
            Self::Calls => app.page == Page::Calls,
            Self::Starred => app.page == Page::Chats && app.show_starred,
            Self::Archived => chats && app.show_archived,
            Self::Locked => chats && app.locked_folder_open(),
            Self::Settings => matches!(app.page, Page::Settings | Page::Wallpaper),
        }
    }

    /// Whether the rail offers this destination: Archived while chats are
    /// archived and Locked while chats are locked, as their chips used to.
    pub fn offered(self, app: &App) -> bool {
        match self {
            Self::Archived => app.archived_count() > 0 || app.show_archived,
            Self::Locked => app.locked_count() > 0 || app.locked_folder_open(),
            _ => true,
        }
    }

    /// The actions that bring the window here. None of the destinations is
    /// a toggle but Settings, whose button closes it again as Escape does.
    pub fn actions(self, app: &App) -> Vec<Action> {
        let mut actions = Vec::new();
        let to_chats = |actions: &mut Vec<Action>| {
            if app.page != Page::Chats {
                actions.push(Action::Open(Page::Chats));
            }
            if app.show_starred {
                actions.push(Action::ToggleStarred);
            }
        };
        match self {
            Self::Chats => {
                to_chats(&mut actions);
                if app.show_archived {
                    actions.push(Action::ShowArchived(false));
                }
                if app.locked_folder {
                    actions.push(Action::CloseLockedFolder);
                }
            }
            Self::Calls => {
                if app.page != Page::Calls {
                    actions.push(Action::Open(Page::Calls));
                }
            }
            Self::Starred => {
                if app.page != Page::Chats {
                    actions.push(Action::Open(Page::Chats));
                }
                if !app.show_starred {
                    actions.push(Action::ToggleStarred);
                }
            }
            Self::Archived => {
                if !self.lit(app) {
                    to_chats(&mut actions);
                    // The archive opens in the list column, as it did.
                    if !app.sidebar_visible {
                        actions.push(Action::ToggleSidebar);
                    }
                    actions.push(Action::ShowArchived(true));
                }
            }
            Self::Locked => {
                if !self.lit(app) {
                    to_chats(&mut actions);
                    actions.push(Action::OpenLockedFolder);
                }
            }
            Self::Settings => actions.push(Action::ToggleSettings),
        }
        actions
    }
}

/// The rail as a fixed panel on the left, with the chats' avatars between
/// its two groups. `hidden` says the chat list is hidden: the rail then
/// offers to show it again and stands beside the conversation.
pub fn show(app: &mut App, ui: &mut egui::Ui, hidden: bool) {
    let palette = app.palette;
    // Its own panel id: sharing the list's would hand that resizable panel
    // this narrow width, and it would save it as the list's width.
    let panel = egui::Panel::left("chat-rail")
        .resizable(false)
        .exact_size(WIDTH)
        .show_separator_line(false)
        .frame(Frame::new().fill(palette.rail()).inner_margin(Margin::ZERO));
    let response = panel.show(ui, |ui| {
        // The avatars keep the spacing they always had; the rest is tight.
        let row_gap = ui.spacing().item_spacing.y;
        ui.spacing_mut().item_spacing.y = 0.0;
        let width = ui.available_width();
        if theme::traffic_light_inset(ui.ctx()) > 0.0 {
            // The traffic lights, or the caption buttons ZapFast draws where
            // the desktop puts them on the left (GNOME's round ones end 42
            // points down), take the rail's top; it starts under them.
            let (strip, _) = ui.allocate_exact_size(vec2(width, LIGHTS_CLEARANCE), Sense::hover());
            super::titlebar_drag(ui, strip);
        } else {
            ui.add_space(GAP);
        }
        let bottom: Vec<Destination> = [
            Destination::Starred,
            Destination::Archived,
            Destination::Locked,
            Destination::Settings,
        ]
        .into_iter()
        .filter(|destination| destination.offered(app))
        .collect();
        let foot = bottom.len() as f32 * (PLATE + GAP) + GAP + AVATAR + GAP;
        let minimum = 2.0 * CELL
            + foot
            + if hidden {
                CELL + GAP + MIN_AVATARS
            } else {
                0.0
            };
        let height = ui.available_height().max(minimum);
        crate::autoscroll::show(
            ui,
            egui::ScrollArea::vertical()
                .id_salt("navigation-rail")
                .auto_shrink([false, false])
                .min_scrolled_height(0.0),
            |ui| {
                // Keep the usual spacing in a tall window. In a short one
                // the content keeps its minimum height and scrolls instead
                // of leaving the bottom buttons beyond the window.
                ui.set_height(height);
                for destination in [Destination::Chats, Destination::Calls] {
                    destination_button(app, ui, &palette, destination, CELL);
                }
                if hidden {
                    // A cell of its own under Calls: nothing above it moves
                    // when the list comes and goes.
                    show_list_button(app, ui, &palette);
                    ui.add_space(GAP / 2.0);
                    let line = Rect::from_center_size(
                        ui.cursor().min + vec2(width / 2.0, 0.0),
                        vec2(DIVIDER, 1.0),
                    );
                    ui.painter().rect_filled(line, 0.0, palette.outline);
                    ui.add_space(GAP / 2.0);
                    let middle = (ui.available_height() - foot).max(MIN_AVATARS);
                    ui.allocate_ui_with_layout(
                        vec2(width, middle),
                        egui::Layout::top_down(egui::Align::Center),
                        |ui| {
                            ui.spacing_mut().item_spacing.y = row_gap;
                            super::chats::compact_list(app, ui);
                        },
                    );
                }
                let rest = (ui.available_height() - foot).max(0.0);
                ui.add_space(rest);
                for destination in bottom {
                    ui.add_space(GAP);
                    destination_button(app, ui, &palette, destination, PLATE);
                }
                ui.add_space(GAP);
                ui.vertical_centered(|ui| {
                    super::accounts::avatar_button(app, ui, AVATAR).tab_stop(Stop::Profile);
                });
            },
        );
    });
    let rect = response.response.rect;
    ui.ctx().data_mut(|data| data.insert_temp(rect_id(), rect));
    // No line between the rail and the list, whose shades tell them apart;
    // beside the conversation the rail ends as the list does.
    if hidden {
        widgets::paint_edge_beside(ui, &palette, rect);
    }
}

/// The button that brings the hidden chat list back.
fn show_list_button(app: &mut App, ui: &mut egui::Ui, palette: &Palette) {
    ui.allocate_ui_with_layout(
        vec2(ui.available_width(), CELL),
        egui::Layout::centered_and_justified(egui::Direction::TopDown),
        |ui| {
            if theme::icon_button(
                ui,
                Icon::PanelLeft,
                18.0,
                palette.secondary,
                palette.text,
                &super::keys::label("Show the chat list (Ctrl+B)"),
            )
            .tab_stop(Stop::Sidebar)
            .clicked()
            {
                app.actions.push(Action::ToggleSidebar);
            }
        },
    );
}

/// One destination: its icon on a rounded plate, lit as the selected chat
/// row is while the window shows it, with its name and shortcut in the
/// tooltip, in a cell `height` points tall.
fn destination_button(
    app: &mut App,
    ui: &mut egui::Ui,
    palette: &Palette,
    destination: Destination,
    height: f32,
) {
    let lit = destination.lit(app);
    let mut badge = 0;
    let (icon, stop, label) = match destination {
        Destination::Chats => (
            Icon::MessageCircle,
            Stop::Chats,
            format!(
                "{} ({})",
                gettext(app.locale, "Chats"),
                super::keys::label("Ctrl+Shift+1")
            ),
        ),
        Destination::Calls => (
            Icon::Phone,
            Stop::Calls,
            format!(
                "{} ({})",
                gettext(app.locale, "Calls"),
                super::keys::label("Ctrl+Shift+2")
            ),
        ),
        Destination::Starred => (
            Icon::Star,
            Stop::Starred,
            gettext(app.locale, "Starred messages").into_owned(),
        ),
        Destination::Archived => {
            // The count the Archived chip carried: archived chats unread.
            badge = app.archived_unread() as u32;
            (
                Icon::Archive,
                Stop::Archived,
                gettext(app.locale, "Archived").into_owned(),
            )
        }
        Destination::Locked => (
            Icon::Lock,
            Stop::Locked,
            gettext(app.locale, "Locked").into_owned(),
        ),
        // Same as before the rail: the label says what the click does now.
        Destination::Settings => (
            Icon::Settings,
            Stop::Settings,
            super::keys::label(if lit {
                "Close settings (Ctrl+,)"
            } else {
                "Settings (Ctrl+,)"
            }),
        ),
    };
    let (cell, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    let plate = Rect::from_center_size(cell.center(), Vec2::splat(PLATE));
    let response = ui
        .interact(plate, destination_id(destination), Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    theme::reveal_focus(&response);
    let spoken = if badge > 0 {
        format!("{label}, {badge}")
    } else {
        label.clone()
    };
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::SelectableLabel,
            ui.is_enabled(),
            lit,
            spoken.as_str(),
        )
    });
    if ui.is_rect_visible(plate) {
        let hovered = response.hovered() || response.has_focus();
        if lit {
            widgets::highlight_card(ui, palette, plate, palette.surface_active);
        } else if hovered {
            widgets::highlight_card(ui, palette, plate, palette.surface_hover);
        }
        let tint = if lit || hovered {
            palette.text
        } else {
            palette.secondary
        };
        let scale = if response.is_pointer_button_down_on() {
            0.92
        } else {
            1.0
        };
        theme::paint_icon(ui, icon, plate, ICON * scale, tint);
        if badge > 0 {
            widgets::unread_indicator(
                ui,
                palette,
                plate.right_top() + vec2(-3.0, 5.0),
                badge,
                false,
                false,
            );
        }
    }
    let response = response.on_hover_text(label).tab_stop(stop);
    if response.clicked() {
        let actions = destination.actions(app);
        app.actions.extend(actions);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> (tempfile::TempDir, App) {
        let root = tempfile::tempdir().unwrap();
        let app = App::headless(
            crate::paths::AppDirs::under(root.path()),
            crate::settings::Settings::default(),
        )
        .0;
        (root, app)
    }

    #[test]
    fn one_destination_is_lit_at_a_time() {
        let (_root, mut app) = app();
        let lit = |app: &App| {
            [
                Destination::Chats,
                Destination::Calls,
                Destination::Starred,
                Destination::Archived,
                Destination::Locked,
                Destination::Settings,
            ]
            .into_iter()
            .filter(|destination| destination.lit(app))
            .collect::<Vec<_>>()
        };
        assert_eq!(lit(&app), [Destination::Chats]);
        app.show_archived = true;
        assert_eq!(lit(&app), [Destination::Archived]);
        app.show_archived = false;
        app.show_starred = true;
        assert_eq!(lit(&app), [Destination::Starred]);
        app.page = Page::Calls;
        assert_eq!(lit(&app), [Destination::Calls]);
        app.page = Page::Wallpaper;
        assert_eq!(lit(&app), [Destination::Settings]);
    }

    #[test]
    fn calls_and_starred_are_left_through_chats_not_toggled() {
        let (_root, mut app) = app();
        app.page = Page::Calls;
        assert!(
            Destination::Calls.actions(&app).is_empty(),
            "Calls is not a toggle"
        );
        assert!(matches!(
            Destination::Chats.actions(&app).as_slice(),
            [Action::Open(Page::Chats)]
        ));
        assert!(matches!(
            Destination::Starred.actions(&app).as_slice(),
            [Action::Open(Page::Chats), Action::ToggleStarred]
        ));
        app.page = Page::Chats;
        app.show_starred = true;
        assert!(Destination::Starred.actions(&app).is_empty());
        assert!(matches!(
            Destination::Chats.actions(&app).as_slice(),
            [Action::ToggleStarred]
        ));
    }

    /// Archived opens the archive in the list column, bringing a hidden
    /// list back, and Chats leaves it; it is offered only while chats are
    /// archived.
    #[test]
    fn archived_opens_in_the_list_and_chats_leaves_it() {
        let (_root, mut app) = app();
        assert!(!Destination::Archived.offered(&app));
        assert!(!Destination::Locked.offered(&app));
        let mut chat = crate::model::Chat::new("1@s.whatsapp.net".into(), "Ada".into());
        chat.archived = true;
        chat.last_activity = 1;
        app.chats.push(chat);
        assert!(Destination::Archived.offered(&app));
        app.sidebar_visible = false;
        assert!(matches!(
            Destination::Archived.actions(&app).as_slice(),
            [Action::ToggleSidebar, Action::ShowArchived(true)]
        ));
        app.sidebar_visible = true;
        app.show_archived = true;
        assert!(Destination::Archived.actions(&app).is_empty());
        assert!(matches!(
            Destination::Chats.actions(&app).as_slice(),
            [Action::ShowArchived(false)]
        ));
    }
}
