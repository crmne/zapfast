//! Copy-only native attachment drags. The saved source is never moved or removed.
#[cfg(target_os = "linux")]
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use std::path::Path;

#[cfg(windows)]
mod windows;
#[cfg(target_os = "linux")]
mod x11;

pub const SUPPORTED: bool = cfg!(any(windows, target_os = "macos", target_os = "linux"));

#[derive(Clone, Debug)]
pub enum Failure {
    Unavailable,
    Unsupported,
    Native(String),
}

pub fn failure_message(locale: crate::i18n::Locale, failure: &Failure) -> String {
    let detail = match failure {
        Failure::Unavailable => crate::i18n::gettext(
            locale,
            "The attachment is unavailable. Download it before dragging it.",
        )
        .into_owned(),
        Failure::Unsupported => crate::i18n::gettext(
            locale,
            "This window cannot start a native file drag. Use Save as instead.",
        )
        .into_owned(),
        Failure::Native(error) => error.clone(),
    };
    format!(
        "{}: {detail}",
        crate::i18n::gettext(locale, "Could not drag the attachment")
    )
}

/// Called by the shell on the window thread, after the views have released their borrows.
pub fn start(
    window: &winit::window::Window,
    path: &Path,
    ctx: &egui::Context,
) -> Result<(), Failure> {
    // The worker already resolved and checked this file off the window thread.
    let path = path.to_owned();
    #[cfg(windows)]
    {
        let _ = window;
        windows::start(&path).map_err(|error| Failure::Native(error.to_string()))?;
        ctx.request_repaint();
        Ok(())
    }
    #[cfg(target_os = "macos")]
    {
        use std::io::Cursor;
        let preview = image::RgbaImage::from_fn(24, 32, |x, y| {
            image::Rgba(if !(2..=21).contains(&x) || !(2..=29).contains(&y) {
                [0, 0, 0, 0]
            } else {
                [240, 240, 240, 255]
            })
        });
        let mut bytes = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(preview)
            .write_to(&mut bytes, image::ImageFormat::Png)
            .map_err(|error| Failure::Native(error.to_string()))?;
        let ctx = ctx.clone();
        drag::start_drag(
            window,
            drag::DragItem::Files(vec![path]),
            drag::Image::Raw(bytes.into_inner()),
            move |_, _| ctx.request_repaint(),
            drag::Options {
                mode: drag::DragMode::Copy,
                ..Default::default()
            },
        )
        .map_err(|error| Failure::Native(error.to_string()))
    }
    #[cfg(target_os = "linux")]
    {
        use raw_window_handle::{RawDisplayHandle, RawWindowHandle};
        let handle = window
            .window_handle()
            .map_err(|error| Failure::Native(error.to_string()))?
            .as_raw();
        match handle {
            RawWindowHandle::Wayland(_) => {
                use winit::platform::wayland::WindowExtWayland;
                let ctx = ctx.clone();
                window
                    .start_file_drag(&[path], Box::new(move |_| ctx.request_repaint()))
                    .map_err(|error| Failure::Native(error.to_string()))
            }
            RawWindowHandle::Xlib(handle) => {
                let RawDisplayHandle::Xlib(display) = window
                    .display_handle()
                    .map_err(|error| Failure::Native(error.to_string()))?
                    .as_raw()
                else {
                    return Err(Failure::Unsupported);
                };
                let display = display.display.ok_or(Failure::Unsupported)?;
                let xlib = x11_dl::xlib::Xlib::open()
                    .map_err(|error| Failure::Native(error.to_string()))?;
                // SAFETY: The live window owns this Xlib connection. This runs on its UI thread.
                // Release its implicit button grab before the XDND helper takes its explicit one.
                unsafe {
                    (xlib.XUngrabPointer)(display.as_ptr().cast(), x11_dl::xlib::CurrentTime);
                    (xlib.XFlush)(display.as_ptr().cast());
                }
                // Our implicit grab is gone even if the helper fails to start.
                ctx.stop_dragging();
                ctx.input_mut(|input| input.pointer = Default::default());
                x11::start(
                    handle.window.try_into().map_err(|_| Failure::Unsupported)?,
                    path,
                    ctx.clone(),
                )
            }
            _ => Err(Failure::Unsupported),
        }
    }
    #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
    {
        let _ = (window, path, ctx);
        Err(Failure::Unsupported)
    }
}

pub fn take_failure(ctx: &egui::Context) -> Option<Failure> {
    ctx.data_mut(|data| {
        let id = egui::Id::new("attachment-drag-failure");
        let failure = data.get_temp(id);
        data.remove::<Failure>(id);
        failure
    })
}

#[cfg(target_os = "linux")]
fn report_failure(ctx: &egui::Context, error: impl ToString) {
    ctx.data_mut(|data| {
        data.insert_temp(
            egui::Id::new("attachment-drag-failure"),
            Failure::Native(error.to_string()),
        )
    });
    ctx.request_repaint();
}
