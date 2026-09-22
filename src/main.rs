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
}

#[derive(Debug, clap::Subcommand)]
enum Control {
    /// Reload palettes in an already-running ZapFast without showing its window.
    ReloadThemes,
}

fn main() -> eframe::Result<()> {
    let arguments: Vec<_> = std::env::args_os().collect();
    if arguments.len() == 3 && arguments[1] == "--apply-update" {
        return zapfast::updates::install::run_helper(std::path::Path::new(&arguments[2]))
            .map_err(|error| eframe::Error::AppCreation(error.into()));
    }
    let cli = Cli::parse();
    if matches!(cli.command, Some(Control::ReloadThemes)) {
        single_instance::send("reload-themes")
            .map_err(|error| eframe::Error::AppCreation(error.into()))?;
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
        match single_instance::acquire(&waker) {
            single_instance::Outcome::Only(guard) => Some(guard),
            single_instance::Outcome::Surfaced => {
                eprintln!("ZapFast or FastsApp is already running; asked it to show its window");
                return Ok(());
            }
        }
    };
    let default_filter = if cli.verbose {
        "info,zapfast=debug,whatsapp_rust=debug,wacore=debug"
    } else {
        "warn,zapfast=info"
    };
    // A demo must not create empty ZapFast directories that would prevent a
    // later real launch from adopting the existing FastsApp session.
    let mut dirs = if demo {
        paths::AppDirs::under(&std::env::temp_dir().join(format!(
            "zapfast-demo-{}-{}",
            std::process::id(),
            jiff::Timestamp::now().as_millisecond(),
        )))
    } else {
        paths::AppDirs::discover()
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
    dirs.custom_media = settings.custom_media_dir.clone();
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
    let slot = std::sync::Arc::new(std::sync::Mutex::new(Some(app)));

    let mut update_receipt = cli.update_receipt;

    // The link, archive, and tray outlive windows. Recreate a window when the
    // tray, notification, or another launch requests one.
    loop {
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
                    update_receipt: creator_receipt,
                    slot: std::sync::Arc::clone(&creator_slot),
                    #[cfg(feature = "demo")]
                    shot: creator_shot,
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
        // Disable vsync because hidden Wayland windows may stop receiving frame
        // callbacks and block the event loop. Repainting is event-driven.
        glow_options: eframe::egui_glow::GlowConfiguration {
            vsync: false,
            ..Default::default()
        },
        ..Default::default()
    }
}

/// eframe adapter that returns the long-lived [`app::App`] when a window closes.
struct Shell {
    update_receipt: Option<std::path::PathBuf>,
    app: Option<app::App>,
    slot: std::sync::Arc<std::sync::Mutex<Option<app::App>>>,
    #[cfg(feature = "demo")]
    shot: Option<Shot>,
    #[cfg(feature = "demo")]
    tour: Option<zapfast::demo::tour::Tour>,
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
    }

    /// Does not persist egui interaction state across windows. Window size and
    /// position are still persisted.
    fn persist_egui_memory(&self) -> bool {
        false
    }

    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if let Some(app) = self.app.as_mut() {
            #[cfg(feature = "demo")]
            if let Some(tour) = self.tour.as_mut() {
                tour.drive(app, ctx);
            }
            app.background_frame(ctx);
            #[cfg(target_os = "macos")]
            zapfast::macos::update_window(_frame, ctx, app.is_linked());
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
