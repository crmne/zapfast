//! Starting ZapFast in the tray when the person logs in, through
//! fastframe-autostart.
//!
//! The platform's own login-item record is the only state: Settings reads it
//! back rather than keeping a copy that could disagree with it. Every read and
//! change may block (`SMAppService`, the registry, the Flatpak portal, which can
//! ask the person first), so each runs on a thread of its own.

use std::sync::mpsc::{Receiver, TryRecvError};

use fastframe_autostart::{Autostart, Start, State};

fn autostart() -> Autostart {
    Autostart::new("zapfast", "ZapFast")
}

/// What a finished read or change found.
#[derive(Debug)]
pub struct Outcome {
    /// The login item as the platform has it now, or why it could not be
    /// changed.
    pub state: Result<State, String>,
    /// Whether the person asked for the change, rather than a read at start.
    pub changed: bool,
}

/// The one read or change of the login item in flight.
#[derive(Default)]
pub struct LoginItem {
    job: Option<Receiver<Outcome>>,
}

impl LoginItem {
    /// Reads the login item, after moving an older macOS registration over.
    pub fn read(&mut self, wake: impl Fn() + Send + 'static) {
        self.spawn(
            move || {
                #[cfg(target_os = "macos")]
                move_launch_agent();
                Outcome {
                    state: Ok(autostart().state()),
                    changed: false,
                }
            },
            wake,
        );
    }

    /// Starts ZapFast at login, with its window or in the tray, or not at
    /// all.
    pub fn set(&mut self, start: Option<Start>, wake: impl Fn() + Send + 'static) {
        self.spawn(
            move || Outcome {
                state: autostart().set(start).map_err(|error| error.to_string()),
                changed: true,
            },
            wake,
        );
    }

    fn spawn(
        &mut self,
        work: impl FnOnce() -> Outcome + Send + 'static,
        wake: impl Fn() + Send + 'static,
    ) {
        let (sender, receiver) = std::sync::mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("login-item".into())
            .spawn(move || {
                let _ = sender.send(work());
                wake();
            });
        match spawned {
            Ok(_) => self.job = Some(receiver),
            Err(error) => log::warn!("could not read the login item: {error}"),
        }
    }

    /// The finished read or change, if there is one.
    pub fn poll(&mut self) -> Option<Outcome> {
        match self.job.as_ref()?.try_recv() {
            Ok(outcome) => {
                self.job = None;
                Some(outcome)
            }
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                self.job = None;
                None
            }
        }
    }
}

/// Whether Settings offers the switch, and where it stands: `None` where this
/// installation cannot start at login, `Some(None)` where it does not start,
/// and the start it makes where it does.
pub fn switch(state: State) -> Option<Option<Start>> {
    match state {
        State::Unsupported => None,
        State::Off => Some(None),
        State::On(start) | State::NeedsApproval(start) => Some(Some(start)),
    }
}

/// ZapFast 0.19 and earlier wrote their own LaunchAgent. Registers the bundled
/// agent in its place and deletes the old file, once; a copy that cannot
/// register (one run outside its bundle) leaves the file as it is.
#[cfg(target_os = "macos")]
fn move_launch_agent() {
    let Some(home) = std::env::var_os("HOME") else {
        return;
    };
    let old = std::path::Path::new(&home).join("Library/LaunchAgents/me.paolino.zapfast.plist");
    if !old.is_file() {
        return;
    }
    match autostart().set(Some(Start::Hidden)) {
        Ok(_) => {
            if let Err(error) = std::fs::remove_file(&old) {
                log::warn!("could not remove the old login item: {error}");
            }
        }
        Err(error) => log::info!("the old login item stays: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_switch_shows_only_where_zapfast_can_start_at_login() {
        assert_eq!(switch(State::Unsupported), None);
        assert_eq!(switch(State::Off), Some(None));
        assert_eq!(switch(State::On(Start::Hidden)), Some(Some(Start::Hidden)));
        assert_eq!(
            switch(State::NeedsApproval(Start::Hidden)),
            Some(Some(Start::Hidden))
        );
        assert_eq!(switch(State::On(Start::Window)), Some(Some(Start::Window)));
    }

    #[test]
    fn an_off_switch_hides_how_zapfast_starts() {
        // The start mode rides the login item itself: once the item is off,
        // no mode is left to show, and Settings offers the tray start again
        // when the switch comes back on.
        assert_eq!(switch(State::Off), Some(None));
    }
}
