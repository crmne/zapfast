//! The macOS application menu is made once, in the interface's language, by a
//! source that only compiles on macOS. This reads that source instead, so on
//! every other platform too each word of the menu is checked to be asked for
//! by its own literal, whatever rustfmt does to the lines. "Edit" and "Zoom"
//! are asked for in a context of their own: the catalogs already hold both words
//! in another sense, as an action and as a setting.
use std::path::PathBuf;

#[test]
fn every_macos_menu_label_is_asked_for_in_the_readers_language() {
    let source =
        std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/macos.rs"))
            .expect("src/macos.rs");
    let menu = source
        .split("fn build_menu(")
        .nth(1)
        .and_then(|rest| rest.split("\n}\n").next())
        .expect("the function that makes the menu");
    for label in [
        "About ZapFast",
        "Settings…",
        "Hide ZapFast",
        "Quit ZapFast",
        "File",
        "New Chat…",
        "Close Window",
        "Edit",
        "Undo",
        "Redo",
        "Cut",
        "Copy",
        "Paste",
        "Select All",
        "Find…",
        "View",
        "Toggle Sidebar",
        "Zoom In",
        "Zoom Out",
        "Actual Size",
        "Zoom",
        "Window",
        "Show ZapFast",
        "Help",
        "Keyboard Shortcuts",
        "ZapFast Help",
    ] {
        let asked = menu.match_indices(&format!("\"{label}\"")).any(|(at, _)| {
            let before = menu[..at].trim_end();
            before.ends_with("gettext(locale,")
                || before.ends_with("\"application menu\", ".trim_end())
        });
        assert!(asked, "{label:?} is not asked for by its own literal");
    }
    // The two words with two senses take the context, and no other label does.
    assert_eq!(menu.matches("pgettext(").count(), 2, "{menu}");
}
