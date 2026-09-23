//! Whether the window may wait for vsync.
//!
//! On Wayland, swapping buffers with vsync waits for a frame callback, and
//! compositors stop sending those to hidden windows. ZapFast's winit patch
//! reports such windows as occluded, so eframe stops painting them and only
//! runs `logic`, but that needs a compositor that tells clients when a window
//! is suspended (xdg_wm_base version 6). Older ones leave vsync off.
//! Shared with Spotifast.

/// Decided once per run.
pub fn enabled() -> bool {
    static VSYNC: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *VSYNC.get_or_init(|| {
        #[cfg(target_os = "linux")]
        let wayland = wayland::reports_suspended();
        #[cfg(not(target_os = "linux"))]
        let wayland = None;
        let vsync = is_safe(wayland);
        log::info!("vsync {}", if vsync { "on" } else { "off" });
        vsync
    })
}

/// `wayland` is `None` off Wayland, otherwise whether the compositor reports
/// hidden windows as suspended.
fn is_safe(wayland: Option<bool>) -> bool {
    wayland.unwrap_or(true)
}

#[cfg(target_os = "linux")]
mod wayland {
    use wayland_client::globals::{GlobalListContents, registry_queue_init};
    use wayland_client::protocol::wl_registry::{self, WlRegistry};
    use wayland_client::{Connection, Dispatch, QueueHandle};

    /// The first xdg_wm_base version with the `suspended` toplevel state.
    const SUSPENDED_SINCE: u32 = 6;

    struct Globals;

    impl Dispatch<WlRegistry, GlobalListContents> for Globals {
        fn event(
            _: &mut Self,
            _: &WlRegistry,
            _: wl_registry::Event,
            _: &GlobalListContents,
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
        }
    }

    /// `None` when this is not a Wayland session, as winit then uses X11.
    pub fn reports_suspended() -> Option<bool> {
        let connection = Connection::connect_to_env().ok()?;
        let (globals, _queue) = registry_queue_init::<Globals>(&connection).ok()?;
        Some(globals.contents().with_list(|list| {
            list.iter().any(|global| {
                global.interface == "xdg_wm_base" && global.version >= SUSPENDED_SINCE
            })
        }))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn vsync_waits_only_where_a_hidden_window_cannot_block() {
        // macOS, Windows and X11.
        assert!(super::is_safe(None));
        // Wayland compositors that suspend hidden windows.
        assert!(super::is_safe(Some(true)));
        // Older Wayland compositors send no frame callbacks and no warning.
        assert!(!super::is_safe(Some(false)));
    }
}
