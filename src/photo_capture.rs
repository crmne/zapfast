//! Takes a photo with the computer's camera from the composer. A thread reads
//! the camera through [`crate::camera::Source`] and keeps its newest frame as
//! RGBA, which the camera window shows as a live preview until the photo is
//! taken; the camera is let go when the [`Capture`] is dropped.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// The largest picture asked of the camera: a camera that does better is
/// scaled down to Full HD, a smaller one delivers its own size.
const BUDGET: (usize, usize) = (1920, 1080);
const FPS: u32 = 30;

/// One camera frame as straight RGBA.
#[derive(Clone)]
pub struct Frame {
    pub width: usize,
    pub height: usize,
    pub rgba: Arc<Vec<u8>>,
    /// Counts frames, so the window uploads a texture only for a new one.
    pub serial: u64,
}

#[derive(Default)]
struct Shared {
    frame: Option<Frame>,
    error: Option<String>,
}

/// An open camera, delivering frames until it is dropped.
pub struct Capture {
    shared: Arc<Mutex<Shared>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
    /// The preview's texture and the serial of the frame it shows.
    pub texture: Option<(u64, egui::TextureHandle)>,
}

impl Capture {
    /// Opens the default camera on a thread of its own; `ctx` is repainted
    /// for every frame.
    pub fn start(ctx: &egui::Context) -> Self {
        let shared = Arc::new(Mutex::new(Shared::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let shared = Arc::clone(&shared);
            let stop = Arc::clone(&stop);
            let ctx = ctx.clone();
            std::thread::Builder::new()
                .name("photo-capture".into())
                .spawn(move || run(&shared, &stop, &ctx))
                .ok()
        };
        Self {
            shared,
            stop,
            thread,
            texture: None,
        }
    }

    /// A camera that never opens one: a still gradient, for demos and tests.
    #[cfg(any(test, feature = "demo"))]
    pub fn simulated() -> Self {
        let (width, height) = (640, 360);
        let mut rgba = Vec::with_capacity(width * height * 4);
        for y in 0..height {
            for x in 0..width {
                rgba.extend_from_slice(&[
                    (x * 255 / width) as u8,
                    (y * 255 / height) as u8,
                    160,
                    255,
                ]);
            }
        }
        let shared = Shared {
            frame: Some(Frame {
                width,
                height,
                rgba: Arc::new(rgba),
                serial: 1,
            }),
            error: None,
        };
        Self {
            shared: Arc::new(Mutex::new(shared)),
            stop: Arc::new(AtomicBool::new(true)),
            thread: None,
            texture: None,
        }
    }

    /// The newest frame, once the camera has delivered one.
    pub fn frame(&self) -> Option<Frame> {
        self.shared.lock().ok()?.frame.clone()
    }

    /// Why the camera cannot be used, once it has failed.
    pub fn error(&self) -> Option<String> {
        self.shared.lock().ok()?.error.clone()
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            // The thread lets the camera go after its current read; the
            // window does not wait for that.
            std::thread::spawn(move || {
                let _ = thread.join();
            });
        }
    }
}

fn run(shared: &Mutex<Shared>, stop: &AtomicBool, ctx: &egui::Context) {
    let fail = |error: String| {
        log::warn!("[PHOTO] the camera could not be used: {error}");
        if let Ok(mut shared) = shared.lock() {
            shared.error = Some(error);
        }
        ctx.request_repaint();
    };
    let mut source = match crate::camera::Source::open(None, BUDGET, FPS) {
        Ok(source) => source,
        Err(error) => return fail(error),
    };
    let size = source.size();
    let mut yuv = vec![0u8; size.0 * size.1 * 3 / 2];
    let mut serial = 0u64;
    while !stop.load(Ordering::Relaxed) {
        match source.read(&mut yuv, size) {
            crate::camera::Read::Frame(_) => {
                serial += 1;
                let rgba = i420_to_rgba(&yuv, size.0, size.1);
                if let Ok(mut shared) = shared.lock() {
                    shared.frame = Some(Frame {
                        width: size.0,
                        height: size.1,
                        rgba: Arc::new(rgba),
                        serial,
                    });
                }
                ctx.request_repaint();
            }
            crate::camera::Read::Idle => {}
            crate::camera::Read::Ended => {
                fail("the camera stopped delivering pictures".into());
                break;
            }
        }
    }
    source.stop();
}

/// The photo taken from `frame`: its RGBA, flipped left to right when
/// `mirror` is on, as the preview showed it.
pub fn taken(frame: Frame, mirror: bool) -> Vec<u8> {
    let mut rgba = Arc::unwrap_or_clone(frame.rgba);
    if mirror {
        flip_horizontal(&mut rgba, frame.width);
    }
    rgba
}

/// Flips straight RGBA left to right, row by row.
fn flip_horizontal(rgba: &mut [u8], width: usize) {
    if width == 0 {
        return;
    }
    for row in rgba.chunks_exact_mut(width * 4) {
        let (pixels, _) = row.as_chunks_mut::<4>();
        pixels.reverse();
    }
}

/// Converts tightly packed YUV 4:2:0 (BT.601, studio range), as
/// [`crate::camera::Source::read`] delivers it, to RGBA.
fn i420_to_rgba(yuv: &[u8], width: usize, height: usize) -> Vec<u8> {
    let (chroma_width, chroma_height) = (width / 2, height / 2);
    let (luma, chroma) = yuv.split_at(width * height);
    let (u_plane, v_plane) = chroma.split_at(chroma_width * chroma_height);
    let mut out = vec![255u8; width * height * 4];
    for row in 0..height {
        for column in 0..width {
            let y = 298 * (i32::from(luma[row * width + column]) - 16);
            let at = (row / 2) * chroma_width + column / 2;
            let u = i32::from(u_plane[at]) - 128;
            let v = i32::from(v_plane[at]) - 128;
            let pixel = (row * width + column) * 4;
            out[pixel] = ((y + 409 * v + 128) >> 8).clamp(0, 255) as u8;
            out[pixel + 1] = ((y - 100 * u - 208 * v + 128) >> 8).clamp(0, 255) as u8;
            out[pixel + 2] = ((y + 516 * u + 128) >> 8).clamp(0, 255) as u8;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn studio_range_black_white_and_grey_convert_to_their_rgb() {
        // 2x2 frames: four luma samples, then one U and one V.
        for (luma, rgb) in [(16, 0), (235, 255), (126, 128)] {
            let rgba = i420_to_rgba(&[luma, luma, luma, luma, 128, 128], 2, 2);
            for pixel in rgba.chunks(4) {
                assert!(
                    pixel[..3].iter().all(|&c| c.abs_diff(rgb) <= 1),
                    "luma {luma} gave {pixel:?}"
                );
                assert_eq!(pixel[3], 255);
            }
        }
    }

    fn frame() -> Frame {
        // Two rows of two pixels, each pixel's channels set to its number.
        let rgba = [1u8, 2, 3, 4].iter().flat_map(|&n| [n; 4]).collect();
        Frame {
            width: 2,
            height: 2,
            rgba: Arc::new(rgba),
            serial: 1,
        }
    }

    #[test]
    fn a_mirrored_photo_is_flipped_left_to_right_row_by_row() {
        let photo = taken(frame(), true);
        let firsts: Vec<u8> = photo.chunks(4).map(|pixel| pixel[0]).collect();
        assert_eq!(firsts, [2, 1, 4, 3]);
    }

    #[test]
    fn an_unmirrored_photo_is_sent_as_the_camera_saw_it() {
        assert_eq!(taken(frame(), false), *frame().rgba);
    }

    #[test]
    fn a_simulated_camera_has_a_frame_and_no_error() {
        let capture = Capture::simulated();
        let frame = capture.frame().expect("a still frame");
        assert_eq!(frame.rgba.len(), frame.width * frame.height * 4);
        assert!(capture.error().is_none());
    }
}
