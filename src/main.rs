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
    #[arg(long, hide = true)]
    update_receipt: Option<std::path::PathBuf>,
    #[arg(long, hide = true)]
    update_error: Option<String>,
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
    let arguments: Vec<_> = std::env::args_os().collect();
    if arguments.len() == 3 && arguments[1] == "--apply-update" {
        return zapfast::updates::install::run_helper(std::path::Path::new(&arguments[2]))
            .map_err(|error| eframe::Error::AppCreation(error.into()));
    }
    let cli = Cli::parse();
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
    let mut logger =
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(default_filter));
    // Write desktop-session logs to disk. Demo runs use stderr so they do not
    // replace a live session's log.
    if !demo {
        match std::fs::File::create(dirs.log_file()) {
            Ok(file) => {
                logger.target(env_logger::Target::Pipe(Box::new(Tee(file))));
            }
            Err(error) => eprintln!("not keeping a log file: {error}"),
        }
    }
    logger.format(|buffer, record| {
        use std::io::Write;
        let message = record.args().to_string();
        let message = if zapfast::diagnostics::is_protocol_target(record.target())
            || zapfast::diagnostics::is_protocol_target(record.module_path().unwrap_or_default())
        {
            zapfast::diagnostics::protocol_summary(&message)
        } else {
            &message
        };
        writeln!(
            buffer,
            "[{} {} {}] {}",
            buffer.timestamp(),
            record.level(),
            record.target(),
            message
        )
    });
    logger.init();
    log_panics(dirs.panic_log());
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
    if let Some(error) = cli.update_error {
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
            zapfast::demo::tour::prepare(&mut app);
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
    let slot = std::sync::Arc::new(std::sync::Mutex::new(Some(app)));

    let mut update_receipt = cli.update_receipt;
    // Without a tray there is no way back to a hidden window, so show it.
    let mut start_hidden = cli.start_hidden
        && !demo
        && update_receipt.is_none()
        && slot
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()
            .is_some_and(app::App::hides_to_tray);

    // The link, archive, and tray outlive windows. Recreate a window when the
    // tray, notification, or another launch requests one.
    loop {
        if std::mem::take(&mut start_hidden) {
            slot.lock()
                .unwrap_or_else(|p| p.into_inner())
                .as_mut()
                .expect("application state present")
                .hide_intent = true;
        } else {
            let creator_slot = std::sync::Arc::clone(&slot);
            let creator_waker = waker.clone();
            let creator_receipt = update_receipt.take();
            #[cfg(feature = "demo")]
            let creator_shot = shot.clone();
            #[cfg(feature = "demo")]
            let creator_tour_events = cli.demo_tour_events.clone();
            eframe::run_native(
                "ZapFast",
                native_options(demo_persistence.clone()),
                Box::new(move |cc| {
                    creator_waker.attach(&cc.egui_ctx);
                    let mut app = creator_slot
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .take()
                        .expect("application state present");
                    app.attach(&cc.egui_ctx);
                    #[cfg(feature = "demo")]
                    if cli.demo_macos {
                        zapfast::theme::preview_macos(&cc.egui_ctx);
                    }
                    Ok(Box::new(Shell {
                        app: Some(app),
                        window_recovery_checked: false,
                        update_receipt: creator_receipt,
                        slot: std::sync::Arc::clone(&creator_slot),
                        #[cfg(feature = "demo")]
                        shot: creator_shot,
                        #[cfg(feature = "demo")]
                        hover: demo_hover,
                        #[cfg(feature = "demo")]
                        tour: cli.demo_tour.then(|| {
                            zapfast::demo::tour::Tour::new(
                                cli.demo_tour_delay.map(std::time::Duration::from_millis),
                                creator_tour_events,
                            )
                        }),
                    }))
                }),
            )?;
            waker.detach();
        }

        let hide = {
            let guard = slot.lock().unwrap_or_else(|p| p.into_inner());
            let app = guard.as_ref().expect("application state present");
            !app.quit_requested && app.hide_intent
        };
        if !hide {
            break;
        }

        // Keep updating link, archive, and tray while no window exists.
        let headless = egui::Context::default();
        slot.lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_mut()
            .expect("application state present")
            .window_gone();
        loop {
            {
                let mut guard = slot.lock().unwrap_or_else(|p| p.into_inner());
                let app = guard.as_mut().expect("application state present");
                app.background_frame(&headless);
                if app.quit_requested || app.wants_show {
                    break;
                }
            }
            zapfast::tray::idle(std::time::Duration::from_millis(150));
        }
        let quit = slot
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()
            .expect("application state present")
            .quit_requested;
        if quit {
            break;
        }
    }

    if let Some(mut app) = slot.lock().unwrap_or_else(|p| p.into_inner()).take() {
        app.shutdown();
    }
    drop(instance);
    Ok(())
}

/// Logger that writes to stderr and the current-run log file.
struct Tee(std::fs::File);

impl std::io::Write for Tee {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let _ = std::io::stderr().write_all(buf);
        self.0.write_all(buf)?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        let _ = std::io::stderr().flush();
        self.0.flush()
    }
}

/// Writes panics to `path` before process exit.
fn log_panics(path: std::path::PathBuf) {
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        let entry = format!(
            "{} zapfast {} on thread {:?}, panic at {} (payload omitted)\n",
            jiff::Timestamp::now(),
            env!("CARGO_PKG_VERSION"),
            thread.name().unwrap_or("unnamed"),
            info.location().map_or_else(
                || "unknown location".to_owned(),
                |location| location.to_string()
            ),
        );
        eprint!("{entry}");
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path);
        if let Ok(mut file) = file {
            use std::io::Write;
            let _ = file.write_all(entry.as_bytes());
        }
    }));
}

/// Parses `--demo-size WxH`.
fn demo_size_arg() -> Option<[f32; 2]> {
    let value = std::env::args()
        .skip_while(|arg| arg != "--demo-size")
        .nth(1)?;
    let (w, h) = value.split_once('x')?;
    Some([w.parse::<f32>().ok()?, h.parse::<f32>().ok()?])
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
        // Hidden Wayland windows stop receiving frame callbacks, so vsync is
        // only on where the patched winit can report them as occluded.
        glow_options: eframe::egui_glow::GlowConfiguration {
            vsync: zapfast::vsync::enabled(),
            ..Default::default()
        },
        ..Default::default()
    }
}

/// Where to move a restored window that no connected monitor shows, in
/// physical virtual-desktop pixels, or `None` to leave it where it is.
///
/// eframe already clamps a saved position on Windows, but a window can still
/// open on no monitor when displays are rearranged or a secondary display
/// comes up late after a restart, and Windows then leaves it unreachable.
/// Any overlap with any monitor counts as visible, so a valid position on a
/// secondary display (including negative coordinates left of or above the
/// primary one) is never moved. The window goes to the middle of the first
/// monitor listed (the caller puts the primary one first), pinned to its
/// top-left corner when it is larger than that monitor.
#[cfg(any(not(target_os = "macos"), test))]
fn recovered_window_position(window: egui::Rect, monitors: &[egui::Rect]) -> Option<egui::Pos2> {
    if monitors.is_empty()
        || monitors
            .iter()
            .any(|monitor| window.intersect(*monitor).area() > 0.0)
    {
        return None;
    }
    let target = monitors[0];
    let slack = (target.size() - window.size()).max(egui::Vec2::ZERO);
    Some((target.min + slack / 2.0).round())
}

/// Moves the window onto the primary monitor when its restored position is
/// on none of the connected ones. Checked once, on a new window's first
/// frame.
///
/// Wayland does not reveal global window positions, so `outer_position`
/// fails there and nothing happens. macOS keeps windows on a screen itself,
/// and winit's macOS coordinates disagree between displays with different
/// scale factors, so it is left alone. Windows and X11 report the window and
/// every monitor in the same physical pixels.
#[cfg(not(target_os = "macos"))]
fn recover_offscreen_window(ctx: &egui::Context, frame: &eframe::Frame) {
    let Some(window) = frame.winit_window() else {
        return;
    };
    // A minimized window on Windows reports a parking position far off
    // screen; restoring it brings back its real one.
    if window.is_minimized() == Some(true) {
        return;
    }
    let Ok(position) = window.outer_position() else {
        return;
    };
    let size = window.outer_size();
    let rect = |x: i32, y: i32, width: u32, height: u32| {
        egui::Rect::from_min_size(
            egui::pos2(x as f32, y as f32),
            egui::vec2(width as f32, height as f32),
        )
    };
    // The primary monitor comes first, as the place a lost window goes; a
    // platform without one falls back to the first connected monitor.
    let monitors: Vec<_> = window
        .primary_monitor()
        .into_iter()
        .chain(window.available_monitors())
        .map(|monitor| {
            let (position, size) = (monitor.position(), monitor.size());
            rect(position.x, position.y, size.width, size.height)
        })
        .collect();
    let Some(target) = recovered_window_position(
        rect(position.x, position.y, size.width, size.height),
        &monitors,
    ) else {
        return;
    };
    log::warn!("the restored window was on no connected monitor; moving it to the primary one");
    // egui-winit multiplies the position by the window's pixels per point,
    // so dividing by the same factor asks for exactly these physical pixels,
    // whatever the scale of the monitor the window is on now.
    let pixels_per_point = ctx.input(|input| input.pixels_per_point);
    ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(
        target / pixels_per_point,
    ));
}

/// eframe adapter that returns the long-lived [`app::App`] when a window closes.
struct Shell {
    /// Whether this window's first frame checked that a monitor shows it.
    window_recovery_checked: bool,
    update_receipt: Option<std::path::PathBuf>,
    app: Option<app::App>,
    slot: std::sync::Arc<std::sync::Mutex<Option<app::App>>>,
    #[cfg(feature = "demo")]
    shot: Option<Shot>,
    #[cfg(feature = "demo")]
    tour: Option<zapfast::demo::tour::Tour>,
    #[cfg(feature = "demo")]
    hover: Option<egui::Pos2>,
}

impl Drop for Shell {
    fn drop(&mut self) {
        *self.slot.lock().unwrap_or_else(|p| p.into_inner()) = self.app.take();
    }
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
        if let (Some(tour), Some(app)) = (&mut self.tour, &mut self.app) {
            tour.input(app, ctx, input);
        }
        if let Some(pos) = self.hover {
            input.events.push(egui::Event::PointerMoved(pos));
        }
    }

    /// Does not persist egui interaction state across windows. Window size and
    /// position are still persisted.
    fn persist_egui_memory(&self) -> bool {
        false
    }

    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        if !self.window_recovery_checked {
            self.window_recovery_checked = true;
            #[cfg(not(target_os = "macos"))]
            recover_offscreen_window(ctx, frame);
        }
        if let Some(app) = self.app.as_mut() {
            #[cfg(feature = "demo")]
            if let Some(tour) = self.tour.as_mut() {
                tour.drive(app, ctx);
            }
            app.background_frame(ctx);
            #[cfg(target_os = "macos")]
            zapfast::macos::update_window(frame, ctx, app.is_linked());
        }
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
        if let Some(app) = self.app.as_mut() {
            app.frame_ui(ui);
            let startup = app.backend.take_startup();
            if let Some(receipt) = self.update_receipt.take() {
                std::thread::spawn(move || {
                    if let Err(error) = zapfast::updates::install::acknowledge(&receipt) {
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
    }

    /// Saves essential state before the window closes.
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if let Some(app) = self.app.as_mut() {
            app.save_state();
        }
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

#[cfg(test)]
mod window_tests {
    use super::recovered_window_position;
    use egui::{Rect, pos2, vec2};

    fn monitor(x: f32, y: f32, width: f32, height: f32) -> Rect {
        Rect::from_min_size(pos2(x, y), vec2(width, height))
    }

    #[test]
    fn a_window_above_every_monitor_moves_to_the_middle_of_the_first() {
        let window = Rect::from_min_size(pos2(100.0, -500.0), vec2(400.0, 300.0));
        assert_eq!(
            recovered_window_position(window, &[monitor(0.0, 0.0, 1920.0, 1080.0)]),
            Some(pos2(760.0, 390.0))
        );
    }

    #[test]
    fn a_window_partly_on_a_monitor_stays() {
        let window = Rect::from_min_size(pos2(-100.0, 100.0), vec2(400.0, 300.0));
        assert_eq!(
            recovered_window_position(window, &[monitor(0.0, 0.0, 1920.0, 1080.0)]),
            None
        );
    }

    #[test]
    fn a_window_on_a_monitor_left_of_or_above_the_primary_stays() {
        let monitors = [
            monitor(0.0, 0.0, 1920.0, 1080.0),
            monitor(-1920.0, 0.0, 1920.0, 1080.0),
            monitor(0.0, -1440.0, 2560.0, 1440.0),
        ];
        let left = Rect::from_min_size(pos2(-1600.0, 100.0), vec2(800.0, 600.0));
        let above = Rect::from_min_size(pos2(200.0, -1300.0), vec2(800.0, 600.0));
        assert_eq!(recovered_window_position(left, &monitors), None);
        assert_eq!(recovered_window_position(above, &monitors), None);
    }

    #[test]
    fn a_window_left_where_a_disconnected_monitor_was_moves_to_the_primary() {
        // The reported case: a window saved on a display to the right, which
        // is gone after a restart. The primary one here is not at the origin.
        let window = Rect::from_min_size(pos2(3891.0, -358.0), vec2(1180.0, 780.0));
        let monitors = [
            monitor(1920.0, 0.0, 1920.0, 1080.0),
            monitor(0.0, 0.0, 1920.0, 1080.0),
        ];
        assert_eq!(
            recovered_window_position(window, &monitors),
            Some(pos2(2290.0, 150.0))
        );
    }

    #[test]
    fn a_window_larger_than_the_monitor_is_pinned_to_its_corner() {
        let window = Rect::from_min_size(pos2(5000.0, 5000.0), vec2(2000.0, 1200.0));
        assert_eq!(
            recovered_window_position(window, &[monitor(-1280.0, 0.0, 1280.0, 1024.0)]),
            Some(pos2(-1280.0, 0.0))
        );
    }

    #[test]
    fn without_monitors_nothing_moves() {
        let window = Rect::from_min_size(pos2(-5000.0, -5000.0), vec2(400.0, 300.0));
        assert_eq!(recovered_window_position(window, &[]), None);
    }
}
