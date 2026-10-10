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

    /// Demo view: `chat`, `phone-menu`, `empty`, `settings`, `login`,
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

    /// A number, a `wa.me` link, or a `whatsapp://` URI naming the chat to
    /// open. Brings the running copy's window forward on that chat.
    #[arg(value_name = "TARGET")]
    target: Option<String>,

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
/// `fastframe_fonts` logs, once at startup, which installed face draws each
/// script Inter lacks, which is what a report of odd Arabic or CJK text
/// needs first.
///
/// `arboard` warns on every clipboard open when a Wayland compositor has no
/// data-control protocol (GNOME, mutter) and it falls back to X11, which works
/// there. Quiet that one target so it does not fill the log file, without
/// hiding real clipboard failures (`arboard=error`) or any other warning.
fn default_log_filter(verbose: bool) -> &'static str {
    if verbose {
        "info,zapfast=debug,whatsapp_rust=debug,wacore=debug"
    } else {
        // env_logger treats '/' as a message-regex separator, so it cannot
        // select the literal target Client/AppState here. Enable Client INFO;
        // the protocol redactor reduces every such line to a fixed category.
        "warn,zapfast=info,fastframe_fonts=info,Client=info,arboard=error"
    }
}

fn main() -> eframe::Result<()> {
    let result = run();
    // A Windows release has no console, so an error that ends the start
    // would otherwise leave the reader nothing to see (#349). `run` has
    // already released the single-instance lock.
    #[cfg(windows)]
    if let Err(error) = &result {
        startup_failure_dialog(error);
    }
    result
}

fn run() -> eframe::Result<()> {
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
    // A number, a `wa.me` link, or the `whatsapp://` URI the desktop entry
    // passes. Read before the single-instance guard, because it changes the
    // verb: a running copy is asked to open the chat instead of to show.
    let target = cli.target.as_deref().and_then(zapfast::target::parse);
    if let Some(argument) = cli.target.as_deref()
        && target.is_none()
    {
        eprintln!("ZapFast cannot read a phone number from {argument}");
        std::process::exit(2);
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
        let verb = match &target {
            Some(request) => request.verb(),
            None if cli.start_hidden => "ping".to_owned(),
            None => "show".to_owned(),
        };
        match single_instance::acquire(&discovered.runtime, &waker, &verb) {
            single_instance::Outcome::Only(guard) => Some(guard),
            single_instance::Outcome::Surfaced if cli.start_hidden && target.is_none() => {
                eprintln!("ZapFast is already running");
                return Ok(());
            }
            single_instance::Outcome::Surfaced => {
                eprintln!(
                    "ZapFast or FastsApp is already running; asked it to {}",
                    if target.is_some() {
                        "open the chat"
                    } else {
                        "show its window"
                    }
                );
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
    // Moving a single-account setup into its account folder needs the
    // keyring; when it cannot finish, the log says why.
    if !demo && let Err(error) = dirs.adopt_single_account() {
        log::error!("could not move the linked account into its folder: {error}");
        return Err(eframe::Error::AppCreation(error.into()));
    }
    let settings = settings::Settings::load(&dirs.settings_file());
    let demo_persistence = demo.then(|| dirs.state.join("window.ron"));

    #[allow(unused_mut)]
    let mut app = if demo {
        app::App::headless(dirs, settings).0
    } else {
        app::App::new(&waker, dirs, settings, app::AppOptions { tray: true })
            .map_err(|error| eframe::Error::AppCreation(error.into()))?
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
    // The first launch owns the slot, so no verb reached a running copy and
    // the chat is opened here. The app drains its queue after the frame.
    if let Some(request) = &target {
        app.actions.push(zapfast::model::Action::StartChat {
            name: request.display_name(),
            id: request.chat.clone(),
        });
        if let Some(text) = &request.text {
            app.actions.push(zapfast::model::Action::PrefillComposer {
                chat: request.chat.clone(),
                text: text.clone(),
            });
        }
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
    #[cfg(feature = "demo")]
    let demo_phone_menu = cli
        .demo_page
        .as_deref()
        .is_some_and(|page| page.split(',').any(|part| part.trim() == "phone-menu"));
    let mut update_receipt = launch.receipt;
    // The link, archive, and tray outlive windows. The shell recreates a
    // window when the tray, a notification, or another launch requests one;
    // without a tray a hidden start shows the window (App::start_hidden).
    // A link asked to be seen, so it overrides a hidden start: the chat it
    // named would otherwise open behind a tray nobody is looking at.
    let start_hidden = start_hidden(
        cli.start_hidden,
        target.is_some(),
        demo,
        update_receipt.is_some(),
    );
    fastframe_shell::Shell::new(app, &waker)
        .start_hidden(start_hidden)
        .idle(fastframe_tray::idle)
        .run(|lease| {
            let receipt = update_receipt.take();
            // The latest geometry lives in the app waiting in the shell, so
            // a reopened window (tray, notification, Wayland reopen) uses
            // what the last one remembered, not what the process started
            // with. Demo runs keep their fixed screenshot size.
            let geometry = if demo_persistence.is_some() {
                zapfast::window::Geometry::default()
            } else {
                lease.peek(|app: &app::App| app.settings.window_geometry())
            };
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
                native_options(demo_persistence.clone(), geometry),
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
                        #[cfg(target_os = "windows")]
                        taskbar: Default::default(),
                        #[cfg(feature = "demo")]
                        shot,
                        #[cfg(feature = "demo")]
                        hover: demo_hover,
                        #[cfg(feature = "demo")]
                        open_phone_menu: demo_phone_menu,
                        #[cfg(feature = "demo")]
                        tour,
                    }))
                }),
            )
        })?;
    drop(instance);
    Ok(())
}

/// Whether eframe stopped because the graphics driver gave it no usable
/// OpenGL: no pixel format, no context of any version it asks for, or a
/// context the renderer cannot use.
#[cfg(windows)]
fn is_graphics_failure(error: &eframe::Error) -> bool {
    matches!(
        error,
        eframe::Error::Glutin(_) | eframe::Error::NoGlutinConfigs(..) | eframe::Error::OpenGL(_)
    )
}

/// The text of the dialog shown when ZapFast cannot start, kept apart from
/// the dialog so it is tested on every platform.
#[cfg(any(windows, test))]
fn startup_failure_text(graphics: bool, details: &str, log: &std::path::Path) -> String {
    let summary = if graphics {
        "ZapFast could not start because the graphics driver does not offer \
         OpenGL 2.1 or newer, which ZapFast needs to draw its window.\n\n\
         Install the current driver from the maker of the graphics chip \
         (Intel, AMD or NVIDIA). The Microsoft Basic Display Adapter, some \
         virtual machines and some remote desktop sessions offer no usable \
         OpenGL."
    } else {
        "ZapFast could not start."
    };
    format!(
        "{summary}\n\nDetails: {details}\n\nThe log may say more: {}",
        log.display()
    )
}

/// Explains a failed start in a message box, the only thing a release
/// without a console can show before its window exists.
#[cfg(windows)]
fn startup_failure_dialog(error: &eframe::Error) {
    use windows::Win32::UI::WindowsAndMessaging::{
        MB_ICONERROR, MB_OK, MB_SETFOREGROUND, MessageBoxW,
    };
    use windows::core::PCWSTR;

    let text = startup_failure_text(
        is_graphics_failure(error),
        &error.to_string(),
        &paths::AppDirs::discover().log_file(),
    );
    let wide = |text: &str| text.encode_utf16().chain([0]).collect::<Vec<u16>>();
    let (text, caption) = (wide(&text), wide("ZapFast"));
    // SAFETY: both strings are NUL-terminated and outlive the call.
    unsafe {
        MessageBoxW(
            None,
            PCWSTR(text.as_ptr()),
            PCWSTR(caption.as_ptr()),
            MB_OK | MB_ICONERROR | MB_SETFOREGROUND,
        );
    }
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
    .then(|| protocol_summary(message))
}

/// Whether the first window starts in the tray.
///
/// A link overrides a hidden start: the chat it named would otherwise open
/// behind a tray nobody is looking at. A demo and an update receipt have
/// their own reasons to keep it hidden.
fn start_hidden(requested: bool, has_target: bool, demo: bool, receipt: bool) -> bool {
    requested && !has_target && !demo && !receipt
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

fn native_options(
    demo_persistence: Option<std::path::PathBuf>,
    geometry: zapfast::window::Geometry,
) -> eframe::NativeOptions {
    let default_size = demo_size_arg().unwrap_or([1180.0, 780.0]);
    let demo = demo_persistence.is_some();
    let viewport = egui::ViewportBuilder::default()
        .with_title(if demo { "ZapFast Demo" } else { "ZapFast" })
        .with_app_id(if demo {
            "zapfast-demo".to_owned()
        } else {
            std::env::var("FLATPAK_ID").unwrap_or_else(|_| "zapfast".to_owned())
        });
    // The remembered size and position, from the settings. eframe's own
    // stored window (when present from an earlier version) still wins over
    // this, and is then remembered here on the first frame.
    let viewport = if demo {
        viewport.with_inner_size(default_size)
    } else {
        zapfast::window::viewport(viewport, geometry, default_size)
    };
    let viewport = viewport
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

/// eframe adapter holding the long-lived [`app::App`] for one window; it goes
/// back to the shell when the window closes.
struct Shell {
    /// Whether this window's first frame checked that a monitor shows it.
    window_recovery_checked: bool,
    update_receipt: Option<fastframe_update::Receipt>,
    app: fastframe_shell::Held<app::App>,
    /// This window's unread overlay on its taskbar button.
    #[cfg(target_os = "windows")]
    taskbar: zapfast::notify::Taskbar,
    #[cfg(feature = "demo")]
    shot: Option<Shot>,
    #[cfg(feature = "demo")]
    tour: Option<zapfast::demo::tour::Tour>,
    #[cfg(feature = "demo")]
    hover: Option<egui::Pos2>,
    #[cfg(feature = "demo")]
    open_phone_menu: bool,
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
            fastframe_shell::window::recover_offscreen(ctx, frame);
        }
        let app = &mut *self.app;
        #[cfg(feature = "demo")]
        if let Some(tour) = self.tour.as_mut() {
            tour.drive(app, ctx);
        }
        app.background_frame(ctx);
        #[cfg(target_os = "windows")]
        if let (Some(window), Some(count)) = (frame.winit_window(), app.taskbar_badge_count())
            && let Some(at) = self.taskbar.show(window, count, app.locale)
        {
            ctx.request_repaint_after(at.saturating_duration_since(std::time::Instant::now()));
        }
        // The chat header is 60 points and zooms; the linking screen keeps
        // AppKit's own 28-point strip.
        let title_bar = if app.is_linked() && !app.app_lock.is_locked() {
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
        #[cfg(feature = "demo")]
        if std::mem::take(&mut self.open_phone_menu) {
            egui::Popup::open_id(ui.ctx(), zapfast::demo::phone_menu_popup_id());
        }
        app.frame_ui(ui);
        let startups: Vec<_> = app
            .accounts
            .iter_mut()
            .filter_map(|account| account.backend.take_startup())
            .collect();
        if let Some(receipt) = self.update_receipt.take() {
            std::thread::spawn(move || {
                if let Err(error) = receipt.acknowledge() {
                    log::warn!("could not acknowledge the update: {error:#}");
                    return;
                }
                for startup in startups {
                    let _ = startup.send(());
                }
            });
        } else {
            for startup in startups {
                let _ = startup.send(());
            }
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

#[cfg(test)]
mod target_cli_tests {
    use super::*;

    /// The argument the desktop entry's `%U` passes, and the shapes a reader
    /// types or pastes, all reach the same chat.
    #[test]
    fn a_target_argument_names_a_chat() {
        for argument in [
            "whatsapp://send?phone=20123456789",
            "https://wa.me/20123456789",
            "+20123456789",
        ] {
            let cli = Cli::try_parse_from(["zapfast", argument]).unwrap();
            let request = zapfast::target::parse(cli.target.as_deref().unwrap()).unwrap();
            assert_eq!(request.chat, "20123456789@s.whatsapp.net", "{argument}");
            assert_eq!(request.text, None, "{argument}");
        }
    }

    /// A share link's template reaches the app decoded, both through the
    /// first launch's own queue and over the socket to a running copy.
    #[test]
    fn a_target_argument_carries_its_template() {
        let cli = Cli::try_parse_from([
            "zapfast",
            "whatsapp://send?phone=20123456789&text=Hi%20there",
        ])
        .unwrap();
        let request = zapfast::target::parse(cli.target.as_deref().unwrap()).unwrap();
        assert_eq!(request.text.as_deref(), Some("Hi there"));
        // What the second launch sends is what the running copy reads back.
        let verb = request.verb();
        assert!(verb.starts_with("open 20123456789@s.whatsapp.net\t"));
        assert_eq!(
            zapfast::target::Request::from_verb(verb.strip_prefix("open ").unwrap()),
            Some(request)
        );
    }

    /// `reload-themes` is a subcommand and a number is not: clap must not
    /// read one as the other. The subcommand takes no chat, so asking for
    /// both is refused rather than half-honoured.
    #[test]
    fn the_subcommand_and_the_target_stay_apart() {
        assert!(matches!(
            Cli::try_parse_from(["zapfast", "reload-themes"])
                .unwrap()
                .command,
            Some(Control::ReloadThemes)
        ));
        assert!(Cli::try_parse_from(["zapfast", "reload-themes", "20123456789"]).is_err());
        let cli = Cli::try_parse_from(["zapfast", "wa.me/20123456789"]).unwrap();
        assert!(cli.command.is_none());
        assert_eq!(cli.target.as_deref(), Some("wa.me/20123456789"));
    }

    /// A launch with nothing to open still works: the target is optional.
    #[test]
    fn a_launch_without_a_target_still_parses() {
        let cli = Cli::try_parse_from(["zapfast"]).unwrap();
        assert!(cli.target.is_none());
        let cli = Cli::try_parse_from(["zapfast", "--start-hidden"]).unwrap();
        assert!(cli.start_hidden);
        assert!(cli.target.is_none());
    }

    /// A link asked to be seen: `--start-hidden` with a chat to open must not
    /// leave the chat behind a tray nobody is looking at.
    #[test]
    fn a_link_overrides_a_hidden_start() {
        assert!(!start_hidden(true, true, false, false), "a link is shown");
        assert!(
            start_hidden(true, false, false, false),
            "autostart stays hidden"
        );
        assert!(!start_hidden(false, false, false, false));
        assert!(
            !start_hidden(true, false, true, false),
            "a demo is its own run"
        );
        assert!(
            !start_hidden(true, false, false, true),
            "an update is not seen"
        );
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
mod startup_failure_tests {
    use super::*;

    #[test]
    fn a_graphics_failure_names_the_driver_requirement_and_the_details() {
        let log = std::path::Path::new("C:/zapfast/zapfast.log");
        let text = startup_failure_text(
            true,
            "glutin error: extension to create ES context with wgl is not present",
            log,
        );
        assert!(text.contains("OpenGL 2.1 or newer"));
        assert!(text.contains("graphics driver"));
        assert!(text.contains("ES context with wgl is not present"));
        assert!(text.contains(&log.display().to_string()));
    }

    #[test]
    fn any_other_failure_shows_its_details_without_blaming_the_driver() {
        let text = startup_failure_text(
            false,
            "The directory is not writable",
            std::path::Path::new("zapfast.log"),
        );
        assert!(text.starts_with("ZapFast could not start."));
        assert!(!text.contains("OpenGL"));
        assert!(text.contains("The directory is not writable"));
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

    /// Every log names the face chosen for each fallback script, without
    /// asking a reporter to start with `--verbose`.
    #[test]
    fn the_default_log_records_the_fallback_fonts() {
        assert!(matches(
            default_log_filter(false),
            log::Level::Info,
            "fastframe_fonts::system"
        ));
        assert!(!matches(
            default_log_filter(false),
            log::Level::Debug,
            "fastframe_fonts::system"
        ));
    }

    #[test]
    fn default_log_records_protocol_recovery_lifecycle_and_redacts_other_client_info() {
        let filter = default_log_filter(false);
        assert!(matches(filter, log::Level::Info, "Client/AppState"));
        assert!(!matches(filter, log::Level::Info, "AppState"));
        // env_logger cannot select a literal target containing '/'. The
        // broader Client INFO target must remain inside the redaction boundary.
        assert!(matches(filter, log::Level::Info, "Client/Message"));
        let record = log::Record::builder()
            .level(log::Level::Info)
            .target("Client/Message")
            .module_path(Some("whatsapp_rust::client::message"))
            .args(format_args!("private"))
            .build();
        assert_eq!(
            redact_protocol(&record, "private 12345@s.whatsapp.net"),
            Some(std::borrow::Cow::Borrowed(
                "protocol diagnostic (private details omitted)"
            ))
        );
    }

    #[test]
    fn app_state_info_is_redacted_even_with_a_custom_log_target() {
        let record = log::Record::builder()
            .args(format_args!("private 12345@s.whatsapp.net"))
            .level(log::Level::Info)
            .target("Client/AppState")
            .module_path(Some("whatsapp_rust::client::app_state"))
            .build();
        assert_eq!(
            redact_protocol(
                &record,
                "Recovered the regular_low collection from the primary device: 12345@s.whatsapp.net"
            ),
            Some(std::borrow::Cow::Borrowed(
                "app-state regular_low recovery reply applied by library"
            ))
        );
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
