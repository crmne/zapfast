//! Linux status-notifier tray item.
//!
//! It runs on a separate thread. If no tray host is available, window close
//! falls back to quitting.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender};
use std::time::Duration;

use ksni::blocking::TrayMethods;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayCommand {
    Show,
    ShowHide,
    Quit,
}

struct FastTray {
    commands: Sender<TrayCommand>,
    wake: Arc<dyn Fn() + Send + Sync>,
}

impl FastTray {
    fn send(&self, command: TrayCommand) {
        if self.commands.send(command).is_ok() {
            (self.wake)();
        }
    }
}

impl ksni::Tray for FastTray {
    fn id(&self) -> String {
        crate::profile::running().slug()
    }

    fn title(&self) -> String {
        crate::profile::running().label()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        let size = 64usize;
        let rgba = crate::util::app_icon_rgba(size);
        // ksni expects network-byte-order ARGB32.
        let mut data = Vec::with_capacity(rgba.len());
        let (pixels, _) = rgba.as_chunks::<4>();
        for [r, g, b, a] in pixels {
            data.extend_from_slice(&[*a, *r, *g, *b]);
        }
        vec![ksni::Icon {
            width: size as i32,
            height: size as i32,
            data,
        }]
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        self.send(TrayCommand::ShowHide);
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::*;
        vec![
            StandardItem {
                label: format!("Show or hide {}", crate::profile::running().label()),
                activate: Box::new(|tray: &mut Self| tray.send(TrayCommand::ShowHide)),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Quit".into(),
                activate: Box::new(|tray: &mut Self| tray.send(TrayCommand::Quit)),
                ..Default::default()
            }
            .into(),
        ]
    }
}

pub struct TrayService {
    _handle: ksni::blocking::Handle<FastTray>,
    commands: Receiver<TrayCommand>,
}

impl TrayService {
    /// Registers the tray item, or returns `None` without a host.
    pub fn spawn(wake: impl Fn() + Send + Sync + 'static) -> Option<Self> {
        let (sender, commands) = std::sync::mpsc::channel();
        let tray = FastTray {
            commands: sender,
            wake: Arc::new(wake),
        };
        match tray.spawn() {
            Ok(handle) => Some(Self {
                _handle: handle,
                commands,
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

/// Waits while headless; the tray continues on its own thread.
pub fn idle(duration: Duration) {
    std::thread::sleep(duration);
}
