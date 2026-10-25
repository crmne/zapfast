//! Middle-click scrolling on Windows, and on Linux once the user turns it on
//! (a middle click there usually pastes). macOS keeps its existing input.
//!
//! Views show their lists through [`show`] or [`show_rows`]; the controller
//! applies the gesture after drawing.

#[cfg_attr(any(windows, target_os = "linux"), path = "autoscroll/active.rs")]
#[cfg_attr(
    not(any(windows, target_os = "linux")),
    path = "autoscroll/inactive.rs"
)]
mod platform;

pub use platform::{Autoscroll, primary_drag, show, show_rows};

/// Whether middle-click scrolling is on: always on Windows, on Linux only
/// when turned on in the settings, and never on macOS.
pub fn enabled(chosen_on_linux: bool) -> bool {
    enabled_for(cfg!(windows), cfg!(target_os = "linux"), chosen_on_linux)
}

const fn enabled_for(windows: bool, linux: bool, chosen_on_linux: bool) -> bool {
    windows || (linux && chosen_on_linux)
}

#[cfg(test)]
mod choice_tests {
    use super::enabled_for;

    #[test]
    fn windows_always_autoscrolls_linux_by_choice_and_macos_never() {
        assert!(enabled_for(true, false, false));
        assert!(enabled_for(true, false, true));
        assert!(!enabled_for(false, true, false));
        assert!(enabled_for(false, true, true));
        assert!(!enabled_for(false, false, true));
        assert!(!enabled_for(false, false, false));
    }
}
