//! Keeps the clipboard readable after the window closes.
//!
//! Wayland gives the selection to a client, and the client answers every
//! paste request itself. Closing the window to the tray destroys it but keeps
//! eframe's event loop, and winit's Wayland connection with it, for the next
//! window; nothing dispatches that connection until one opens. A selection
//! winit still owns is then never answered, so every other app that reads the
//! clipboard waits for its own timeout before pasting nothing. Omarchy's menu
//! opens its search field, which asks for the clipboard, and took two seconds.
//!
//! So when the window goes away this hands the text to a clipboard source of
//! its own, which wl-clipboard-rs serves on a thread of its own. The text
//! stays pasteable while ZapFast runs without a window, and no client waits on
//! a client that stopped running its loop. The source ends when another app
//! takes the selection, and with the process.

/// The MIME type winit offers with a selection of its own, so it recognises
/// it when the offer comes back and reads it from memory instead of waiting
/// on an event loop that may not be running.
const WINIT_SELECTION_PREFIX: &str = "application/x-winit-selection;pid=";

/// Whether `types` carries a selection `pid` made.
///
/// Winit appends the display pointer after the pid, so only the start of the
/// type identifies the process.
fn ours(types: &std::collections::HashSet<String>, pid: u32) -> bool {
    let marker = format!("{WINIT_SELECTION_PREFIX}{pid};");
    types.iter().any(|mime| mime.starts_with(&marker))
}

/// Hands the selection of a window that is gone to a source of its own.
///
/// `clipboard` is the window's winit clipboard, taken as the window goes.
/// Reading our own selection returns it from memory, so no part of this waits
/// on another client.
pub fn keep_after_window(clipboard: winit::platform::wayland::Clipboard) {
    let pid = std::process::id();
    let spawned = std::thread::Builder::new()
        .name("clipboard-owner".to_owned())
        .spawn(move || hand_off(clipboard, pid));
    if let Err(error) = spawned {
        log::warn!("could not keep the clipboard after the window closed: {error}");
    }
}

/// Takes the text of the window's selection and offers it again, so it stays
/// readable while no window exists.
fn hand_off(clipboard: winit::platform::wayland::Clipboard, pid: u32) {
    use wl_clipboard_rs::copy::{ClipboardType, MimeType, Options, Source};
    use wl_clipboard_rs::paste;

    // Another app may have copied since this window did. Its event loop
    // answers for its selection, and taking it would overwrite it with what
    // this process still holds.
    let types = match paste::get_mime_types(paste::ClipboardType::Regular, paste::Seat::Unspecified)
    {
        Ok(types) => types,
        Err(error) => {
            log::debug!("could not read the clipboard's types: {error}");
            return;
        }
    };
    if !ours(&types, pid) {
        return;
    }

    let text = match clipboard.load_text() {
        Ok(text) => text,
        Err(error) => {
            log::debug!("could not read the clipboard to keep it: {error}");
            return;
        }
    };
    if text.is_empty() {
        return;
    }

    // A background copy: the crate serves the selection on a thread of its
    // own until another app takes it, so this one only has to set it up. It
    // offers on every seat, as wl-copy does by default; desktops have one.
    let mut options = Options::new();
    options.clipboard(ClipboardType::Regular);
    if let Err(error) = options.copy(Source::Bytes(text.into_bytes().into()), MimeType::Text) {
        log::debug!("could not keep the clipboard after the window closed: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn types(mimes: &[&str]) -> HashSet<String> {
        mimes.iter().map(|mime| (*mime).to_owned()).collect()
    }

    #[test]
    fn a_selection_this_process_made_is_ours() {
        let offered = types(&[
            "text/plain",
            "application/x-winit-selection;pid=42;display=7f6d6e1a0c80",
        ]);
        assert!(ours(&offered, 42));
    }

    #[test]
    fn another_process_selection_is_not_ours() {
        let offered = types(&[
            "text/plain",
            "application/x-winit-selection;pid=7;display=7f6d6e1a0c80",
        ]);
        assert!(!ours(&offered, 42));
    }

    #[test]
    fn an_ordinary_selection_is_not_ours() {
        assert!(!ours(
            &types(&["text/plain", "text/plain;charset=utf-8"]),
            42
        ));
    }
}
