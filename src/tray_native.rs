//! Windows notification-area and macOS menu-bar item.
//!
//! Windows uses a separate message-loop thread. macOS creates the item on the
//! main thread and pumps AppKit events while no window exists.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender};
use std::time::Duration;

#[cfg(windows)]
use tray_icon::menu::MenuEvent;
#[cfg(windows)]
use tray_icon::menu::MenuId;
use tray_icon::menu::{Menu, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayCommand {
    Show,
    ShowHide,
    Quit,
}

type Wake = Arc<dyn Fn() + Send + Sync>;

const SHOW: &str = "show";
const QUIT: &str = "quit";

#[cfg(windows)]
fn command_for(id: &MenuId) -> Option<TrayCommand> {
    match id.0.as_str() {
        SHOW => Some(TrayCommand::ShowHide),
        QUIT => Some(TrayCommand::Quit),
        _ => None,
    }
}

/// Tray item lifetime guard.
struct Item {
    _icon: TrayIcon,
}

/// Creates the tray item and forwards its events to `sender`.
fn build(sender: Sender<TrayCommand>, wake: Wake) -> Result<Item, Box<dyn std::error::Error>> {
    let size = 32u32;
    #[cfg(not(target_os = "macos"))]
    let icon = Icon::from_rgba(crate::util::app_icon_rgba(size as usize), size, size)?;
    // Use a macOS template image so the system selects its color.
    #[cfg(target_os = "macos")]
    let icon = Icon::from_rgba(crate::util::tray_template_rgba(size as usize), size, size)?;
    let menu = Menu::new();
    menu.append_items(&[
        &MenuItem::with_id(SHOW, "Show or hide ZapFast", true, None),
        &PredefinedMenuItem::separator(),
        &MenuItem::with_id(QUIT, "Quit", true, None),
    ])?;
    let builder = TrayIconBuilder::new()
        .with_icon(icon)
        .with_tooltip("ZapFast")
        .with_menu(Box::new(menu));
    // Left-click toggles the window; right-click opens the menu.
    #[cfg(target_os = "macos")]
    let builder = builder
        .with_icon_as_template(true)
        .with_menu_on_left_click(false);
    let icon = builder.build()?;

    #[cfg(windows)]
    {
        let menu_sender = sender.clone();
        let menu_wake = Arc::clone(&wake);
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            if let Some(command) = command_for(&event.id)
                && menu_sender.send(command).is_ok()
            {
                menu_wake();
            }
        }));
    }
    tray_icon::TrayIconEvent::set_event_handler(Some(move |event: tray_icon::TrayIconEvent| {
        if let tray_icon::TrayIconEvent::Click {
            button: tray_icon::MouseButton::Left,
            button_state: tray_icon::MouseButtonState::Up,
            ..
        } = event
            && sender.send(TrayCommand::ShowHide).is_ok()
        {
            wake();
        }
    }));

    Ok(Item { _icon: icon })
}

#[cfg(windows)]
mod host {
    use super::*;
    use windows_sys::Win32::System::Threading::GetCurrentThreadId;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, GetMessageW, MSG, TranslateMessage,
    };

    /// Runs the item on its own thread and reports startup result.
    pub fn start(sender: Sender<TrayCommand>, wake: Wake) -> Result<u32, String> {
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("zapfast-tray".to_owned())
            .spawn(move || {
                let _item = match build(sender, wake) {
                    Ok(item) => item,
                    Err(error) => {
                        let _ = ready_tx.send(Err(error.to_string()));
                        return;
                    }
                };
                let _ = ready_tx.send(Ok(unsafe { GetCurrentThreadId() }));
                let mut message: MSG = unsafe { std::mem::zeroed() };
                while unsafe { GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) } > 0 {
                    unsafe {
                        TranslateMessage(&message);
                        DispatchMessageW(&message);
                    }
                }
            });
        if let Err(error) = spawned {
            return Err(error.to_string());
        }
        ready_rx
            .recv_timeout(Duration::from_secs(5))
            .map_err(|_| "The tray thread stopped responding".to_string())?
    }
}

#[cfg(windows)]
pub struct TrayService {
    commands: Receiver<TrayCommand>,
    _thread_id: u32,
}

#[cfg(windows)]
impl TrayService {
    /// Registers the tray item, or returns `None` on failure.
    pub fn spawn(wake: impl Fn() + Send + Sync + 'static) -> Option<Self> {
        let (sender, commands) = std::sync::mpsc::channel();
        match host::start(sender, Arc::new(wake)) {
            Ok(thread_id) => Some(Self {
                commands,
                _thread_id: thread_id,
            }),
            Err(error) => {
                log::info!("no system tray available: {error}");
                None
            }
        }
    }

    pub fn drain_commands(&self) -> Vec<TrayCommand> {
        self.commands.try_iter().collect()
    }

    /// The tray already runs on its own thread.
    pub fn attach(&mut self) {}

    /// No per-window cleanup is needed.
    pub fn hidden(&mut self) {}
}

/// Waits while headless; the Windows tray continues on its own thread.
#[cfg(windows)]
pub fn idle(duration: Duration) {
    std::thread::sleep(duration);
}

#[cfg(target_os = "macos")]
mod host {
    use std::cell::RefCell;
    use std::ffi::CString;

    use objc2::runtime::{AnyClass, AnyObject, Bool, MethodImplementation, Sel};
    use objc2::{Encode, MainThreadMarker, sel};
    use objc2_app_kit::{NSApplication, NSEventMask};
    use objc2_foundation::{NSDate, NSDefaultRunLoopMode};

    use super::*;

    thread_local! {
        /// Main-thread-only macOS status item.
        pub static ITEM: RefCell<Option<Item>> = const { RefCell::new(None) };
        pub(super) static REOPEN: RefCell<Option<Sender<TrayCommand>>> = const { RefCell::new(None) };
    }

    pub(super) fn request_reopen(has_visible_windows: bool) -> Bool {
        if !has_visible_windows {
            REOPEN.with(|slot| {
                if let Some(sender) = slot.borrow().as_ref() {
                    let _ = sender.send(TrayCommand::Show);
                }
            });
        }
        Bool::YES
    }

    extern "C-unwind" fn application_should_handle_reopen(
        _delegate: *mut AnyObject,
        _selector: Sel,
        _application: *mut NSApplication,
        has_visible_windows: Bool,
    ) -> Bool {
        request_reopen(has_visible_windows.as_bool())
    }

    fn install_reopen_handler(app: &NSApplication) {
        let Some(delegate) = app.delegate() else {
            log::warn!("the macOS application delegate is unavailable");
            return;
        };
        let delegate: &AnyObject = AsRef::<AnyObject>::as_ref(&*delegate);
        let class = delegate.class();
        let selector = sel!(applicationShouldHandleReopen:hasVisibleWindows:);
        if class.responds_to(selector) {
            return;
        }
        let implementation: extern "C-unwind" fn(
            *mut AnyObject,
            Sel,
            *mut NSApplication,
            Bool,
        ) -> Bool = application_should_handle_reopen;
        let types = CString::new(format!("{}@:@{}", Bool::ENCODING, Bool::ENCODING))
            .expect("valid Objective-C type encoding");
        let installed = unsafe {
            objc2::ffi::class_addMethod(
                class as *const AnyClass as *mut AnyClass,
                selector,
                implementation.__imp(),
                types.as_ptr(),
            )
        };
        if !installed.as_bool() {
            log::warn!("the macOS Dock reopen handler could not be installed");
        }
    }

    /// Creates the item once on the main thread.
    pub fn create(sender: Sender<TrayCommand>, wake: Wake) {
        let Some(mtm) = MainThreadMarker::new() else {
            log::warn!("the status item can only be made on the main thread");
            return;
        };
        REOPEN.with(|slot| *slot.borrow_mut() = Some(sender.clone()));
        install_reopen_handler(&NSApplication::sharedApplication(mtm));
        match build(sender, wake) {
            Ok(item) => ITEM.with(|slot| *slot.borrow_mut() = Some(item)),
            Err(error) => log::info!("no status item: {error}"),
        }
    }

    pub fn exists() -> bool {
        ITEM.with(|slot| slot.borrow().is_some())
    }

    pub fn activate() {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let app = NSApplication::sharedApplication(mtm);
        #[allow(deprecated)]
        app.activateIgnoringOtherApps(true);
    }

    /// Pumps AppKit events for `duration` while headless.
    pub fn pump(duration: Duration) {
        let Some(mtm) = MainThreadMarker::new() else {
            std::thread::sleep(duration);
            return;
        };
        let app = NSApplication::sharedApplication(mtm);
        let deadline = NSDate::dateWithTimeIntervalSinceNow(duration.as_secs_f64());
        // Safety: AppKit defines this immutable extern static.
        let mode = unsafe { NSDefaultRunLoopMode };
        loop {
            let event = app.nextEventMatchingMask_untilDate_inMode_dequeue(
                NSEventMask::Any,
                Some(&deadline),
                mode,
                true,
            );
            match event {
                Some(event) => app.sendEvent(&event),
                None => break,
            }
        }
    }
}

#[cfg(target_os = "macos")]
pub struct TrayService {
    commands: Receiver<TrayCommand>,
    /// State kept until the first window can create the item.
    pending: Option<(Sender<TrayCommand>, Wake)>,
}

#[cfg(target_os = "macos")]
impl TrayService {
    /// Stores item state until AppKit runs with the first window.
    pub fn spawn(wake: impl Fn() + Send + Sync + 'static) -> Option<Self> {
        let (sender, commands) = std::sync::mpsc::channel();
        Some(Self {
            commands,
            pending: Some((sender, Arc::new(wake))),
        })
    }

    pub fn drain_commands(&self) -> Vec<TrayCommand> {
        self.commands.try_iter().collect()
    }

    /// Creates the item if needed and activates the application.
    pub fn attach(&mut self) {
        if let Some((sender, wake)) = self.pending.take() {
            host::create(sender, wake);
        }
        if host::exists() {
            host::activate();
        }
    }

    /// Keeps the status item and Dock icon available without a window.
    pub fn hidden(&mut self) {}
}

/// Waits while headless and pumps AppKit events.
#[cfg(target_os = "macos")]
pub fn idle(duration: Duration) {
    host::pump(duration);
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[test]
    fn dock_reopen_requests_a_window_only_when_none_is_visible() {
        let (sender, commands) = std::sync::mpsc::channel();
        host::REOPEN.with(|slot| *slot.borrow_mut() = Some(sender));

        assert!(host::request_reopen(true).as_bool());
        assert!(commands.try_recv().is_err());

        assert!(host::request_reopen(false).as_bool());
        assert_eq!(commands.try_recv(), Ok(TrayCommand::Show));
        host::REOPEN.with(|slot| *slot.borrow_mut() = None);
    }
}
