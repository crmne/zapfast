//! Desktop entry point.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use zapfast::{app, backend, paths, settings, single_instance};

use clap::Parser;

/// A fast, native WhatsApp client.
#[derive(Debug, Parser)]
#[command(name = "zapfast", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Option<Control>,
    /// Log more from the WhatsApp library.
    #[arg(short, long)]
    verbose: bool,
    /// Start in the tray without opening a window, when the tray is available
    /// and ZapFast keeps running in the background. For login autostart.
    #[arg(long)]
    start_hidden: bool,

    /// Start with offline sample chats.
    #[cfg(feature = "demo")]
    #[arg(long)]
    demo: bool,

    /// Prepare an offline, scripted tour. Press Space to play or replay it.
    #[cfg(feature = "demo")]
    #[arg(long, conflicts_with = "demo_page")]
    demo_tour: bool,

    /// Start the tour automatically after this many milliseconds.
    #[cfg(feature = "demo")]
    #[arg(long, requires = "demo_tour", value_name = "MS")]
    demo_tour_delay: Option<u64>,

    /// Save pointer and shortcut timing for video captions (demo tour only).
    #[cfg(feature = "demo")]
    #[arg(long, requires = "demo_tour", value_name = "PATH")]
    demo_tour_events: Option<std::path::PathBuf>,

    /// Which tour to play: `launch` (41 seconds) or `whats-new` (what 0.16
    /// added, 86 seconds).
    #[cfg(feature = "demo")]
    #[arg(
        long,
        requires = "demo_tour",
        value_name = "NAME",
        default_value = "launch",
        value_parser = clap::builder::PossibleValuesParser::new(zapfast::demo::tour::Script::NAMES),
    )]
    demo_tour_script: String,

    /// Play the tour at once on a virtual clock, save every frame as a PNG in
    /// this directory, and quit when it ends.
    #[cfg(feature = "demo")]
    #[arg(
        long,
        requires = "demo_tour",
        conflicts_with = "demo_tour_delay",
        value_name = "DIR"
    )]
    demo_tour_frames: Option<std::path::PathBuf>,

    /// Frames per second for `--demo-tour-frames` (default 30).
    #[cfg(feature = "demo")]
    #[arg(long, requires = "demo_tour_frames", value_name = "FPS", value_parser = clap::value_parser!(u32).range(1..=120))]
    demo_fps: Option<u32>,

    /// Preview macOS content layout on another platform (demo only).
    #[cfg(feature = "demo")]
    #[arg(long, requires = "demo")]
    demo_macos: bool,

    /// Demo view: `chat`, `empty`, `settings`, `login`,
    /// `pair`, `shortcuts`, `about`, `info`, `mention`, `light`, or a comma-separated
    /// mix such as `chat,light`.
    #[cfg(feature = "demo")]
    #[arg(long)]
    demo_page: Option<String>,

    /// Save the demo window as a PNG and exit. Implies `--demo`.
    #[cfg(feature = "demo")]
    #[arg(long, value_name = "PATH")]
    demo_shot: Option<std::path::PathBuf>,
    /// Screenshot window size as WxH logical points.
    #[arg(long, value_name = "WxH")]
    demo_size: Option<String>,

    /// Delay before taking the screenshot, in milliseconds.
    #[cfg(feature = "demo")]
    #[arg(long, value_name = "MS", default_value_t = 1500)]
    demo_shot_delay: u64,

    /// Hold a synthetic pointer at `X,Y` (logical points) to capture hover
    /// states in demo screenshots without moving the real cursor.
    #[cfg(feature = "demo")]
    #[arg(long, value_name = "X,Y")]
    demo_hover: Option<String>,
}

#[derive(Debug, clap::Subcommand)]
enum Control {
    /// Reload palettes in an already-running ZapFast without showing its window.
    ReloadThemes,
}

/// Default log filter, used when `RUST_LOG` is unset.
///
/// `arboard` warns on every clipboard open when a Wayland compositor has no
/// data-control protocol (GNOME, mutter) and it falls back to X11, which works
/// there. Quiet that one target so it does not fill the log file, without
/// hiding real clipboard failures (`arboard=error`) or any other warning.
fn default_log_filter(verbose: bool) -> &'static str {
    if verbose {
        "info,zapfast=debug,whatsapp_rust=debug,wacore=debug"
    } else {
        "warn,zapfast=info,arboard=error"
    }
}

fn main() -> eframe::Result<()> {
    // First, before parsing the command line or touching any state: run the
    // update helper when asked (`--apply-update <job>`, then exit), and take
    // `--update-receipt` and `--update-error` off the command line.
    let launch = fastframe_update::intercept(&zapfast::updates::CONFIG);
    let cli = Cli::parse_from(&launch.arguments);
    let discovered = paths::AppDirs::discover();
    if matches!(cli.command, Some(Control::ReloadThemes)) {
        if let Err(error) = single_instance::send(&discovered.runtime, "reload-themes") {
            use std::io::ErrorKind;
            if matches!(
                error.kind(),
                ErrorKind::NotFound | ErrorKind::ConnectionRefused
            ) {
                eprintln!("ZapFast is not running, so there are no themes to reload.");
            } else {
                eprintln!("Could not reach the running ZapFast: {error}");
            }
            std::process::exit(1);
        }
        return Ok(());
    }
    let waker = backend::Waker::default();
    #[cfg(feature = "demo")]
    let demo = cli.demo || cli.demo_shot.is_some() || cli.demo_tour;
    #[cfg(not(feature = "demo"))]
    let demo = false;
    // Keep one linked instance. Demo runs do not participate.
    let instance = if demo {
        None
    } else {
        // A hidden start must not surface a copy that is already running.
        let verb = if cli.start_hidden { "ping" } else { "show" };
        match single_instance::acquire(&discovered.runtime, &waker, verb) {
            single_instance::Outcome::Only(guard) => Some(guard),
            single_instance::Outcome::Surfaced if cli.start_hidden => {
                eprintln!("ZapFast is already running");
                return Ok(());
            }
            single_instance::Outcome::Surfaced => {
                eprintln!("ZapFast or FastsApp is already running; asked it to show its window");
                return Ok(());
            }
            single_instance::Outcome::Unanswered => {
                eprintln!("ZapFast is already running but did not answer");
                return Ok(());
            }
        }
    };
    let default_filter = default_log_filter(cli.verbose);
    // A demo must not create empty ZapFast directories that would prevent a
    // later real launch from adopting the existing FastsApp session.
    let dirs = if demo {
        paths::AppDirs::under(&std::env::temp_dir().join(format!(
            "zapfast-demo-{}-{}",
            std::process::id(),
            jiff::Timestamp::now().as_millisecond(),
        )))
    } else {
        discovered
    };
    if !demo {
        dirs.adopt_previous_names()
            .map_err(|error| eframe::Error::AppCreation(error.into()))?;
    }
    // Do not open logs, settings, or either database unless their parent
    // directories have been created and secured successfully.
    dirs.ensure()
        .map_err(|error| eframe::Error::AppCreation(error.into()))?;
    let logging = fastframe_log::Logging::new("zapfast", env!("CARGO_PKG_VERSION"))
        .filter(default_filter)
        .panic_log(dirs.panic_log())
        .redact(redact_protocol);
    // Write desktop-session logs to disk. Demo runs use stderr so they do not
    // replace a live session's log.
    let logging = if demo {
        logging
    } else {
        logging.file(dirs.log_file())
    };
    logging
        .init()
        .map_err(|error| eframe::Error::AppCreation(error.into()))?;
    let settings = settings::Settings::load(&dirs.settings_file());
    let demo_persistence = demo.then(|| dirs.state.join("window.ron"));

    #[allow(unused_mut)]
    let mut app = if demo {
        app::App::headless(dirs, settings).0
    } else {
        app::App::new(&waker, dirs, settings, app::AppOptions { tray: true })
    };
    if cli.verbose {
        app.update_arguments.push("--verbose".into());
    }
    if let Some(error) = launch.error {
        app.toast_error(error);
    }
    if let Some(guard) = &instance {
        app.set_remote_control(guard);
    }
    #[cfg(feature = "demo")]
    if demo {
        zapfast::demo::populate(&mut app);
        zapfast::demo::apply_flags(&mut app, cli.demo_page.as_deref());
        if cli.demo_tour {
            tour_script(&cli.demo_tour_script).prepare(&mut app);
        }
    }
    #[cfg(feature = "demo")]
    let shot = cli.demo_shot.clone().map(|path| Shot {
        path,
        due: std::time::Instant::now() + std::time::Duration::from_millis(cli.demo_shot_delay),
        asked: false,
    });
    #[cfg(feature = "demo")]
    let demo_hover = cli.demo_hover.as_deref().and_then(|value| {
        let (x, y) = value.split_once(',')?;
        Some(egui::pos2(x.trim().parse().ok()?, y.trim().parse().ok()?))
    });
    let mut update_receipt = launch.receipt;
    // The link, archive, and tray outlive windows. The shell recreates a
    // window when the tray, a notification, or another launch requests one;
    // without a tray a hidden start shows the window (App::start_hidden).
    let start_hidden = cli.start_hidden && !demo && update_receipt.is_none();
    fastframe_shell::Shell::new(app, &waker)
        .start_hidden(start_hidden)
        .idle(fastframe_tray::idle)
        .run(|lease| {
            let receipt = update_receipt.take();
            #[cfg(feature = "demo")]
            let shot = shot.clone();
            #[cfg(feature = "demo")]
            let tour = cli.demo_tour.then(|| {
                zapfast::demo::tour::Tour::scripted(
                    tour_script(&cli.demo_tour_script),
                    cli.demo_tour_delay.map(std::time::Duration::from_millis),
                    cli.demo_tour_events.clone(),
                    cli.demo_tour_frames
                        .clone()
                        .map(|dir| zapfast::demo::tour::Capture {
                            dir,
                            fps: cli.demo_fps.unwrap_or(30),
                        }),
                )
            });
            eframe::run_native(
                "ZapFast",
                native_options(demo_persistence.clone()),
                Box::new(move |cc| {
                    let mut app = lease.take(&cc.egui_ctx);
                    app.attach(&cc.egui_ctx);
                    #[cfg(feature = "demo")]
                    if cli.demo_macos {
                        zapfast::theme::preview_macos(&cc.egui_ctx);
                    }
                    Ok(Box::new(Shell {
                        app,
                        window_recovery_checked: false,
                        update_receipt: receipt,
                        #[cfg(feature = "demo")]
                        shot,
                        #[cfg(feature = "demo")]
                        hover: demo_hover,
                        #[cfg(feature = "demo")]
                        tour,
                    }))
                }),
            )
        })?;
    drop(instance);
    Ok(())
}

/// Summarises the WhatsApp library's lines, which can quote protocol
/// payloads, into fixed categories.
fn redact_protocol(
    record: &log::Record<'_>,
    message: &str,
) -> Option<std::borrow::Cow<'static, str>> {
    use zapfast::diagnostics::{is_protocol_target, protocol_summary};
    (is_protocol_target(record.target())
        || is_protocol_target(record.module_path().unwrap_or_default()))
    .then(|| protocol_summary(message).into())
}

/// Parses `--demo-size WxH`.
fn demo_size_arg() -> Option<[f32; 2]> {
    let value = std::env::args()
        .skip_while(|arg| arg != "--demo-size")
        .nth(1)?;
    let (w, h) = value.split_once('x')?;
    Some([w.parse::<f32>().ok()?, h.parse::<f32>().ok()?])
}

/// The tour `--demo-tour-script` names; clap has already checked the name.
#[cfg(feature = "demo")]
fn tour_script(name: &str) -> zapfast::demo::tour::Script {
    zapfast::demo::tour::Script::from_name(name).unwrap_or_default()
}

fn native_options(demo_persistence: Option<std::path::PathBuf>) -> eframe::NativeOptions {
    let demo_size = demo_size_arg().unwrap_or([1180.0, 780.0]);
    let demo = demo_persistence.is_some();
    let viewport = egui::ViewportBuilder::default()
        .with_title(if demo { "ZapFast Demo" } else { "ZapFast" })
        .with_app_id(if demo {
            "zapfast-demo".to_owned()
        } else {
            std::env::var("FLATPAK_ID").unwrap_or_else(|_| "zapfast".to_owned())
        })
        .with_inner_size(demo_size)
        // Keep the floor small enough that Windows can still snap the window
        // into narrow Aero Snap and LG Screen Split zones (a 2560 px ultrawide
        // split four ways is about 640 px wide, which a 720 px minimum blocks).
        .with_min_inner_size([400.0, 300.0])
        .with_icon(app_icon())
        // macOS uses a full-size content view under the traffic lights.
        .with_fullsize_content_view(true)
        .with_titlebar_shown(false)
        .with_title_shown(false);
    eframe::NativeOptions {
        viewport,
        persistence_path: demo_persistence,
        // Do not restore window size during fixed-size screenshot runs.
        persist_window: !demo,
        ..Default::default()
    }
}

/// Installs a Win32 subclass that applies Windows' suggested rectangle during
/// caption drags across monitors with different DPI settings.
///
/// winit's `WM_DPICHANGED` monitor-nudge logic can return a caption-dragged
/// window to its previous monitor and repeatedly grow it (winit#4600). Windows
/// supplies the correct outer rectangle in `lParam`; apply it while winit is
/// positioning the window. These notifications arrive synchronously in the
/// window procedure, so the event-loop message hook cannot handle them.
#[cfg(target_os = "windows")]
fn install_windows_dpi_drag_workaround(window: &impl raw_window_handle::HasWindowHandle) -> bool {
    use raw_window_handle::RawWindowHandle;
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::UI::Shell::SetWindowSubclass;

    let Ok(handle) = window.window_handle() else {
        return false;
    };
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        return false;
    };
    let hwnd = handle.hwnd.get() as HWND;

    // SAFETY: this runs on winit's window thread, and the subclass procedure
    // forwards every message it does not handle to the existing window proc.
    unsafe {
        SetWindowSubclass(
            hwnd,
            Some(windows_dpi_drag_subclass),
            WINDOWS_DPI_DRAG_SUBCLASS_ID,
            0,
        ) != 0
    }
}

#[cfg(target_os = "windows")]
const WINDOWS_DPI_DRAG_SUBCLASS_ID: usize = 0x5a46_4153;

#[cfg(target_os = "windows")]
thread_local! {
    static WINDOWS_CAPTION_DRAG: std::cell::Cell<windows_sys::Win32::Foundation::HWND> =
        const { std::cell::Cell::new(std::ptr::null_mut()) };
    static WINDOWS_DPI_SUGGESTED_RECT: std::cell::Cell<Option<(windows_sys::Win32::Foundation::HWND, windows_sys::Win32::Foundation::RECT)>> =
        const { std::cell::Cell::new(None) };
}

#[cfg(target_os = "windows")]
unsafe extern "system" fn windows_dpi_drag_subclass(
    hwnd: windows_sys::Win32::Foundation::HWND,
    message: u32,
    wparam: windows_sys::Win32::Foundation::WPARAM,
    lparam: windows_sys::Win32::Foundation::LPARAM,
    _subclass_id: usize,
    _reference_data: usize,
) -> windows_sys::Win32::Foundation::LRESULT {
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        HTCAPTION, WINDOWPOS, WM_DPICHANGED, WM_EXITSIZEMOVE, WM_NCDESTROY, WM_NCLBUTTONDOWN,
        WM_NCLBUTTONUP, WM_WINDOWPOSCHANGING,
    };

    let caption_dragging = || WINDOWS_CAPTION_DRAG.with(|active| active.get() == hwnd);
    match message {
        WM_NCLBUTTONDOWN if wparam == HTCAPTION as usize => {
            WINDOWS_CAPTION_DRAG.with(|active| active.set(hwnd));
        }
        WM_DPICHANGED if caption_dragging() && lparam != 0 => {
            // SAFETY: Windows supplies a RECT through lParam for WM_DPICHANGED.
            let suggested = unsafe { *(lparam as *const RECT) };
            let previous_suggestion =
                WINDOWS_DPI_SUGGESTED_RECT.with(|rect| rect.replace(Some((hwnd, suggested))));
            // SAFETY: forward to winit so it tracks the native move loop,
            // updates its scale factor and sends the corresponding egui input.
            // Its SetWindowPos call is corrected in WM_WINDOWPOSCHANGING below,
            // before an intermediate position can trigger another DPI change.
            let result = unsafe { DefSubclassProc(hwnd, message, wparam, lparam) };
            WINDOWS_DPI_SUGGESTED_RECT.with(|rect| rect.set(previous_suggestion));
            return result;
        }
        WM_WINDOWPOSCHANGING if lparam != 0 => {
            let suggested = WINDOWS_DPI_SUGGESTED_RECT.with(std::cell::Cell::get);
            if let Some((target, rect)) = suggested
                && target == hwnd
            {
                // SAFETY: Windows supplies a mutable WINDOWPOS through lParam.
                let window_pos = unsafe { &mut *(lparam as *mut WINDOWPOS) };
                apply_windows_dpi_suggested_rect(window_pos, rect);
            }
        }
        WM_EXITSIZEMOVE | WM_NCLBUTTONUP => {
            // Let winit finish its move/size cleanup before clearing our marker.
            // SAFETY: forwards this window message to the existing procedure.
            let result = unsafe { DefSubclassProc(hwnd, message, wparam, lparam) };
            WINDOWS_CAPTION_DRAG.with(|active| {
                if active.get() == hwnd {
                    active.set(std::ptr::null_mut());
                }
            });
            return result;
        }
        WM_NCDESTROY => {
            WINDOWS_CAPTION_DRAG.with(|active| {
                if active.get() == hwnd {
                    active.set(std::ptr::null_mut());
                }
            });
            // SAFETY: this removes the current subclass from the live window.
            unsafe {
                RemoveWindowSubclass(
                    hwnd,
                    Some(windows_dpi_drag_subclass),
                    WINDOWS_DPI_DRAG_SUBCLASS_ID,
                );
            }
        }
        _ => {}
    }

    // SAFETY: all unhandled messages continue through the existing window proc.
    unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
}

#[cfg(target_os = "windows")]
fn apply_windows_dpi_suggested_rect(
    window_pos: &mut windows_sys::Win32::UI::WindowsAndMessaging::WINDOWPOS,
    rect: windows_sys::Win32::Foundation::RECT,
) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{SWP_NOMOVE, SWP_NOSIZE};

    if window_pos.flags & SWP_NOMOVE == 0 {
        window_pos.x = rect.left;
        window_pos.y = rect.top;
    }
    if window_pos.flags & SWP_NOSIZE == 0 {
        window_pos.cx = rect.right - rect.left;
        window_pos.cy = rect.bottom - rect.top;
    }
}

#[cfg(all(test, target_os = "windows"))]
#[test]
fn dpi_suggestion_respects_independent_move_and_size_flags() {
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::UI::WindowsAndMessaging::{SWP_NOMOVE, SWP_NOSIZE, WINDOWPOS};

    let suggested = RECT {
        left: 100,
        top: 200,
        right: 500,
        bottom: 700,
    };
    for (flags, expected) in [
        (0, (100, 200, 400, 500)),
        (SWP_NOMOVE, (1, 2, 400, 500)),
        (SWP_NOSIZE, (100, 200, 3, 4)),
        (SWP_NOMOVE | SWP_NOSIZE, (1, 2, 3, 4)),
    ] {
        let mut pos = WINDOWPOS {
            hwnd: std::ptr::null_mut(),
            hwndInsertAfter: std::ptr::null_mut(),
            x: 1,
            y: 2,
            cx: 3,
            cy: 4,
            flags,
        };
        apply_windows_dpi_suggested_rect(&mut pos, suggested);
        assert_eq!((pos.x, pos.y, pos.cx, pos.cy), expected, "flags: {flags}");
    }
}

/// eframe adapter holding the long-lived [`app::App`] for one window; it goes
/// back to the shell when the window closes.
struct Shell {
    /// Whether this window's first frame checked that a monitor shows it.
    window_recovery_checked: bool,
    update_receipt: Option<fastframe_update::Receipt>,
    app: fastframe_shell::Held<app::App>,
    #[cfg(feature = "demo")]
    shot: Option<Shot>,
    #[cfg(feature = "demo")]
    tour: Option<zapfast::demo::tour::Tour>,
    #[cfg(feature = "demo")]
    hover: Option<egui::Pos2>,
}

/// Pending screenshot request.
#[cfg(feature = "demo")]
#[derive(Clone)]
struct Shot {
    path: std::path::PathBuf,
    due: std::time::Instant,
    asked: bool,
}

#[cfg(feature = "demo")]
impl Shell {
    fn drive_shot(&mut self, ctx: &egui::Context) {
        let Some(shot) = self.shot.as_mut() else {
            return;
        };
        ctx.request_repaint();
        if !shot.asked && std::time::Instant::now() >= shot.due {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
            shot.asked = true;
        }
        let image = ctx.input(|input| {
            input.events.iter().find_map(|event| match event {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        let Some(image) = image else {
            return;
        };
        let [width, height] = [image.size[0] as u32, image.size[1] as u32];
        let pixels: Vec<u8> = image
            .pixels
            .iter()
            .flat_map(|pixel| pixel.to_srgba_unmultiplied())
            .collect();
        match image::RgbaImage::from_raw(width, height, pixels) {
            Some(buffer) => match buffer.save(&shot.path) {
                Ok(()) => log::info!("wrote {}x{} to {}", width, height, shot.path.display()),
                Err(error) => log::error!("could not write {}: {error}", shot.path.display()),
            },
            None => log::error!("the frame buffer did not match {width}x{height}"),
        }
        self.shot = None;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }
}

impl eframe::App for Shell {
    #[cfg(feature = "demo")]
    fn raw_input_hook(&mut self, ctx: &egui::Context, input: &mut egui::RawInput) {
        if let Some(tour) = &mut self.tour {
            tour.input(&mut self.app, ctx, input);
        }
        // Moves the pointer only while it is elsewhere: each move restarts
        // egui's tooltip delay, so a fake pointer that kept moving in place
        // would never show one.
        if let Some(pos) = self.hover
            && ctx.input(|input| input.pointer.latest_pos()) != Some(pos)
        {
            input.events.push(egui::Event::PointerMoved(pos));
        }
    }

    /// Does not persist egui interaction state across windows. Window size and
    /// position are still persisted.
    fn persist_egui_memory(&self) -> bool {
        false
    }

    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        if !std::mem::replace(&mut self.window_recovery_checked, true) {
            #[cfg(target_os = "windows")]
            if let Some(window) = frame.winit_window()
                && !install_windows_dpi_drag_workaround(window)
            {
                log::warn!("could not install the Windows DPI drag workaround");
            }
            fastframe_shell::window::recover_offscreen(ctx, frame);
        }
        let app = &mut *self.app;
        #[cfg(feature = "demo")]
        if let Some(tour) = self.tour.as_mut() {
            tour.drive(app, ctx);
        }
        app.background_frame(ctx);
        // The chat header is 60 points and zooms; the linking screen keeps
        // AppKit's own 28-point strip.
        let title_bar = if app.is_linked() {
            zapfast::theme::TOP_BAR_HEIGHT
        } else {
            28.0 / ctx.zoom_factor()
        };
        fastframe_macos::align_traffic_lights(frame, ctx, title_bar);
        #[cfg(feature = "demo")]
        {
            // Keep requesting the configured screenshot size until it is applied.
            if self.shot.is_some()
                && let Some([w, h]) = demo_size_arg()
            {
                let now = ctx.input(|input| input.raw.screen_rect.map(|rect| rect.size()));
                if now.is_none_or(|now| (now.x - w).abs() > 1.0 || (now.y - h).abs() > 1.0) {
                    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(w, h)));
                }
            }
            self.drive_shot(ctx);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let app = &mut *self.app;
        app.frame_ui(ui);
        let startup = app.backend.take_startup();
        if let Some(receipt) = self.update_receipt.take() {
            std::thread::spawn(move || {
                if let Err(error) = receipt.acknowledge() {
                    log::warn!("could not acknowledge the update: {error:#}");
                    return;
                }
                if let Some(startup) = startup {
                    let _ = startup.send(());
                }
            });
        } else if let Some(startup) = startup {
            let _ = startup.send(());
        }
        #[cfg(feature = "demo")]
        if let Some(tour) = self.tour.as_mut() {
            tour.observe(app, ui.ctx());
        }
    }

    /// Saves essential state before the window closes.
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.app.save_state();
    }
}

fn app_icon() -> egui::IconData {
    #[cfg(target_os = "macos")]
    {
        // eframe replaces the bundle's Dock icon with this viewport icon.
        let image = image::load_from_memory(include_bytes!("../packaging/macos/icon-1024.png"))
            .expect("bundled macOS icon")
            .into_rgba8();
        egui::IconData {
            width: image.width(),
            height: image.height(),
            rgba: image.into_raw(),
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        const SIZE: usize = 128;
        egui::IconData {
            rgba: zapfast::util::app_icon_rgba(SIZE),
            width: SIZE as u32,
            height: SIZE as u32,
        }
    }
}

#[cfg(all(test, feature = "demo"))]
mod tests {
    use super::*;

    #[test]
    fn tour_cli_accepts_manual_and_delayed_starts() {
        let cli = Cli::try_parse_from(["zapfast", "--demo-tour"]).unwrap();
        assert!(cli.demo_tour);
        assert!(cli.demo_tour_delay.is_none());
        let cli =
            Cli::try_parse_from(["zapfast", "--demo-tour", "--demo-tour-delay", "5000"]).unwrap();
        assert_eq!(cli.demo_tour_delay, Some(5000));
        assert!(Cli::try_parse_from(["zapfast", "--demo-tour-delay", "5000"]).is_err());
        assert!(Cli::try_parse_from(["zapfast", "--demo-tour", "--demo-page", "login",]).is_err());
    }

    #[test]
    fn tour_cli_picks_a_script_and_a_frame_capture() {
        let cli = Cli::try_parse_from(["zapfast", "--demo-tour"]).unwrap();
        assert_eq!(
            tour_script(&cli.demo_tour_script),
            zapfast::demo::tour::Script::Launch
        );
        let cli = Cli::try_parse_from([
            "zapfast",
            "--demo-tour",
            "--demo-tour-script",
            "whats-new",
            "--demo-tour-frames",
            "frames",
            "--demo-fps",
            "60",
        ])
        .unwrap();
        assert_eq!(
            tour_script(&cli.demo_tour_script),
            zapfast::demo::tour::Script::WhatsNew
        );
        assert_eq!(
            cli.demo_tour_frames.as_deref(),
            Some(std::path::Path::new("frames"))
        );
        assert_eq!(cli.demo_fps, Some(60));
        assert!(
            Cli::try_parse_from(["zapfast", "--demo-tour", "--demo-tour-script", "other"]).is_err()
        );
        assert!(Cli::try_parse_from(["zapfast", "--demo-tour-script", "whats-new"]).is_err());
        assert!(Cli::try_parse_from(["zapfast", "--demo-tour", "--demo-fps", "30"]).is_err());
        assert!(
            Cli::try_parse_from([
                "zapfast",
                "--demo-tour",
                "--demo-tour-frames",
                "frames",
                "--demo-tour-delay",
                "5000",
            ])
            .is_err()
        );
    }
}

#[cfg(test)]
mod log_filter_tests {
    use super::*;

    fn matches(filter: &str, level: log::Level, target: &str) -> bool {
        let logger = env_logger::Builder::new().parse_filters(filter).build();
        logger.matches(
            &log::Record::builder()
                .level(level)
                .target(target)
                .args(format_args!("fixture"))
                .build(),
        )
    }

    /// A compositor without data-control makes arboard fall back to X11 and
    /// warn. That is expected, so the default log must not record it, while a
    /// genuine arboard failure still must.
    #[test]
    fn the_default_log_drops_arboards_wayland_fallback_warning() {
        let filter = default_log_filter(false);
        assert!(!matches(
            filter,
            log::Level::Warn,
            "arboard::platform::linux"
        ));
        assert!(matches(
            filter,
            log::Level::Error,
            "arboard::platform::linux"
        ));
        assert!(matches(
            filter,
            log::Level::Warn,
            "zapfast::backend::worker"
        ));
    }

    #[test]
    fn verbose_keeps_arboard_warnings() {
        assert!(matches(
            default_log_filter(true),
            log::Level::Warn,
            "arboard::platform::linux"
        ));
    }
}
