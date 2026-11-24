//! The macOS status item needs a title for the exact unread count and a
//! lifetime independent of the Dock. It shares the existing muda handler,
//! icon artwork and the Fastframe Dock and headless AppKit pump.

use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};
use tray_icon::menu::{Menu, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

use crate::account::Account;
use crate::i18n::{Locale, gettext};
use crate::settings::Settings;

/// Sum the unread messages that can contribute to desktop badges.
pub(crate) fn unread_messages(accounts: &[Account]) -> u32 {
    let now = crate::util::now();
    accounts
        .iter()
        .filter(|account| account.settings.notifications)
        .flat_map(|account| &account.chats)
        .filter(|chat| !chat.archived && !chat.locked && !chat.muted(now))
        .map(|chat| chat.unread)
        .fold(0, u32::saturating_add)
}

#[derive(Default)]
pub(crate) struct Status {
    item: Option<TrayIcon>,
    failed: bool,
    state: Option<State>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct State {
    locale: Locale,
    count: u32,
    hidden: bool,
    lockable: bool,
    keep_running: bool,
}

impl Status {
    pub(crate) fn is_shown(&self) -> bool {
        self.item
            .as_ref()
            .and_then(TrayIcon::ns_status_item)
            .is_some_and(|item| item.isVisible())
    }

    pub(crate) fn sync(&mut self, settings: &Settings, locale: Locale, count: u32, hidden: bool) {
        let Some(main) = objc2::MainThreadMarker::new() else {
            return;
        };
        let state = State {
            locale,
            count: if settings.mac_show_unread_count {
                count
            } else {
                0
            },
            hidden,
            lockable: settings.app_lock_hash.is_some(),
            keep_running: settings.mac_keep_running_on_close,
        };
        let want_item = settings.keep_running_in_background;
        // Restore the Dock before removing the only other way back.
        if !want_item {
            self.failed = false;
            if dock_visible(main, true) {
                self.item = None;
                self.state = None;
                return;
            }
        }
        if self.item.is_none() && !self.failed {
            match create(state) {
                Ok(item) => self.item = Some(item),
                Err(error) => {
                    self.failed = true;
                    log::warn!("could not create the menu-bar item: {error}");
                }
            }
        }
        let show_dock = settings.mac_show_in_dock || !self.is_shown() || !want_item;
        dock_visible(main, show_dock);
        if self.state == Some(state) {
            return;
        }
        let Some(item) = &self.item else {
            return;
        };
        let old = self.state.replace(state);
        if old.is_none_or(|old| {
            (old.locale, old.hidden, old.lockable, old.keep_running)
                != (
                    state.locale,
                    state.hidden,
                    state.lockable,
                    state.keep_running,
                )
        }) {
            match menu(state) {
                Ok(menu) => item.set_menu(Some(Box::new(menu))),
                Err(error) => log::warn!("could not update the menu-bar menu: {error}"),
            }
        }
        let count = state.count;
        if old.is_none_or(|old| old.count != count || old.locale != locale) {
            item.set_title((count > 0).then(|| count.to_string()));
            let tip = if count == 0 {
                "ZapFast".into()
            } else {
                format!(
                    "ZapFast · {}",
                    crate::notify::badge_description(count, locale)
                )
            };
            let _ = item.set_tooltip(Some(tip));
        }
    }
}

fn dock_visible(main: objc2::MainThreadMarker, visible: bool) -> bool {
    let app = NSApplication::sharedApplication(main);
    let policy = if visible {
        NSApplicationActivationPolicy::Regular
    } else {
        NSApplicationActivationPolicy::Accessory
    };
    if app.activationPolicy() != policy {
        fastframe_dock::set_dock_icon_visible(visible);
    }
    app.activationPolicy() == policy
}

fn create(state: State) -> Result<TrayIcon, Box<dyn std::error::Error>> {
    Ok(TrayIconBuilder::new()
        .with_tooltip("ZapFast")
        .with_icon(icon())
        .with_icon_as_template(true)
        .with_menu(Box::new(menu(state)?))
        .build()?)
}

fn icon() -> Icon {
    const SIZE: usize = 36;
    Icon::from_rgba(
        crate::util::tray_template_rgba(SIZE),
        SIZE as u32,
        SIZE as u32,
    )
    .expect("the tray renderer returns square RGBA pixels")
}

fn menu(state: State) -> tray_icon::menu::Result<Menu> {
    let menu = Menu::new();
    let show = MenuItem::with_id(
        "status-show",
        gettext(state.locale, "Show ZapFast"),
        true,
        None,
    );
    menu.append(&show)?;
    if !state.hidden && state.keep_running {
        menu.append(&MenuItem::with_id(
            "status-hide",
            gettext(state.locale, "Close Window"),
            true,
            None,
        ))?;
    }
    if state.lockable {
        menu.append(&MenuItem::with_id(
            "status-lock",
            gettext(state.locale, "Lock ZapFast"),
            true,
            None,
        ))?;
    }
    menu.append_items(&[
        &PredefinedMenuItem::separator(),
        &MenuItem::with_id(
            "status-quit",
            gettext(state.locale, "Quit ZapFast"),
            true,
            None,
        ),
    ])?;
    Ok(menu)
}
