//! The macOS application menu reaches the app once the tray item exists
//! (#215).
//!
//! muda keeps the first menu-event handler it is given and ignores later
//! ones, and fastframe-tray installs one when it makes its item on the first
//! window. If that came first, Settings… (and its ⌘, shortcut), About, Quit
//! and every other ZapFast menu item did nothing.
//!
//! AppKit menus work only on the main thread, where libtest never runs a
//! test, so this file has no harness and does its work in `main`.

#[cfg(target_os = "macos")]
fn main() {
    use objc2_app_kit::NSApplication;
    use zapfast::model::Page;

    let mtm = objc2::MainThreadMarker::new().expect("a harness-less test runs on the main thread");
    let root = tempfile::tempdir().expect("a temporary directory");
    let dirs = zapfast::paths::AppDirs::under(root.path());
    let settings = zapfast::settings::Settings::default();
    // The backend waits for the first drawn frame before it starts, so it
    // never links or opens an archive here.
    let mut app = zapfast::app::App::new(
        &zapfast::backend::Waker::default(),
        dirs,
        settings,
        zapfast::app::AppOptions { tray: true },
    )
    .expect("a fresh app over an empty folder");
    let ctx = egui::Context::default();
    app.attach(&ctx);

    let menu = NSApplication::sharedApplication(mtm)
        .mainMenu()
        .expect("the application menu is installed");
    let submenu = menu
        .itemAtIndex(0)
        .and_then(|item| item.submenu())
        .expect("the ZapFast menu");
    let index = (0..submenu.numberOfItems())
        .find(|&index| {
            submenu
                .itemAtIndex(index)
                .is_some_and(|item| item.title().to_string() == "Settings…")
        })
        .expect("a Settings… item");
    submenu.performActionForItemAtIndex(index);

    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
        app.background_frame(ui.ctx());
    });
    // Headless contexts apply font-atlas updates themselves.
    output.textures_delta.clear();
    assert_eq!(app.page, Page::Settings, "Settings… opens Settings");
    println!("macos_menu: Settings… opens Settings");
}

#[cfg(not(target_os = "macos"))]
fn main() {}
