//! Taking a photo with the camera from the composer's plus menu.
//!
//! A [`Viewfinder`] runs the first camera on a thread of its own, through the same
//! [`crate::camera::Source`] calls use, and keeps only the newest picture for the window. The shot
//! is that picture as RGBA, which the composer stages like a pasted image, so it gets the same
//! review, caption and editor before it goes. Dropping the viewfinder stops the camera.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use egui::ColorImage;

/// The largest picture asked of the camera, either way round: a sharp photo, still cheap to
/// convert at the camera's rate.
const BUDGET: (usize, usize) = (1920, 1080);
/// The frame rate asked of the camera, enough for a smooth preview.
const FPS: u32 = 30;

/// What the viewfinder shows.
#[derive(Clone, Debug)]
pub enum State {
    /// The camera is starting, or waiting on the system's permission prompt.
    Opening,
    /// The camera's newest picture.
    Live(Arc<ColorImage>),
    /// No camera, one another program holds, a refused permission, or one that stopped.
    Failed,
}

/// The state and a count of the pictures delivered, so the texture is updated only when a new
/// picture has arrived.
struct Shared {
    state: State,
    generation: u64,
}

/// A running camera and the texture its newest picture is drawn from.
pub struct Viewfinder {
    shared: Arc<Mutex<Shared>>,
    stopping: Arc<AtomicBool>,
    /// The texture and the generation of the picture it holds.
    texture: Option<(egui::TextureHandle, u64)>,
}

impl Viewfinder {
    /// Opens the first camera on a thread of its own. The window repaints whenever a picture
    /// arrives or the camera fails.
    pub fn start(ctx: egui::Context) -> Self {
        let shared = Arc::new(Mutex::new(Shared {
            state: State::Opening,
            generation: 0,
        }));
        let stopping = Arc::new(AtomicBool::new(false));
        let thread_shared = Arc::clone(&shared);
        let thread_stopping = Arc::clone(&stopping);
        let spawned = std::thread::Builder::new()
            .name("viewfinder".into())
            .spawn(move || capture(&thread_shared, &thread_stopping, &ctx));
        if let Err(error) = spawned {
            log::error!("[PHOTO] the camera thread could not start: {error}");
            set(&shared, State::Failed);
        }
        Self {
            shared,
            stopping,
            texture: None,
        }
    }

    /// A viewfinder showing `image`, with no camera behind it: for tests and demo captures.
    #[cfg(any(test, feature = "demo"))]
    pub fn still(image: ColorImage) -> Self {
        Self {
            shared: Arc::new(Mutex::new(Shared {
                state: State::Live(Arc::new(image)),
                generation: 1,
            })),
            stopping: Arc::new(AtomicBool::new(true)),
            texture: None,
        }
    }

    /// What the viewfinder shows right now.
    pub fn state(&self) -> State {
        lock(&self.shared).state.clone()
    }

    /// The texture of the newest picture, uploaded again only when a new one has arrived.
    pub fn texture(&mut self, ctx: &egui::Context) -> Option<&egui::TextureHandle> {
        let (image, generation) = {
            let shared = lock(&self.shared);
            match &shared.state {
                State::Live(image) => (Arc::clone(image), shared.generation),
                _ => return None,
            }
        };
        match &mut self.texture {
            Some((texture, shown)) => {
                if *shown != generation {
                    texture.set(image, egui::TextureOptions::LINEAR);
                    *shown = generation;
                }
            }
            None => {
                let texture = ctx.load_texture("viewfinder", image, egui::TextureOptions::LINEAR);
                self.texture = Some((texture, generation));
            }
        }
        self.texture.as_ref().map(|(texture, _)| texture)
    }

    /// The newest picture as `(width, height, rgba)`, straight alpha, as the composer stages a
    /// pasted image. `None` until the camera has delivered one.
    pub fn shot(&self) -> Option<(usize, usize, Vec<u8>)> {
        match self.state() {
            State::Live(image) => Some(rgba(&image)),
            _ => None,
        }
    }
}

impl Drop for Viewfinder {
    fn drop(&mut self) {
        // The thread lets the camera go after the frame it is waiting on.
        self.stopping.store(true, Ordering::Relaxed);
    }
}

/// A camera picture as `(width, height, rgba)`. Camera pictures are opaque, so their colours are
/// the same premultiplied or not.
pub fn rgba(image: &ColorImage) -> (usize, usize, Vec<u8>) {
    let [width, height] = image.size;
    let bytes = image
        .pixels
        .iter()
        .flat_map(|pixel| pixel.to_srgba_unmultiplied())
        .collect();
    (width, height, bytes)
}

fn lock(shared: &Mutex<Shared>) -> std::sync::MutexGuard<'_, Shared> {
    shared
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn set(shared: &Mutex<Shared>, state: State) {
    let mut shared = lock(shared);
    shared.state = state;
    shared.generation += 1;
}

/// Reads the camera until the viewfinder is dropped, keeping only its newest picture. Runs on its
/// own thread: the platforms' capture objects are not all `Send`, and reading blocks.
fn capture(shared: &Mutex<Shared>, stopping: &AtomicBool, ctx: &egui::Context) {
    let mut source = match crate::camera::Source::open(None, BUDGET, FPS) {
        Ok(source) => source,
        Err(error) => {
            log::warn!("[PHOTO] the camera could not be opened: {error}");
            set(shared, State::Failed);
            ctx.request_repaint();
            return;
        }
    };
    let (width, height) = source.size();
    let (chroma_w, chroma_h) = (width / 2, height / 2);
    let mut frame = vec![0u8; width * height + 2 * chroma_w * chroma_h];
    while !stopping.load(Ordering::Relaxed) {
        match source.read(&mut frame, (width, height)) {
            crate::camera::Read::Frame(_) => {
                let (luma, chroma) = frame.split_at(width * height);
                let (u, v) = chroma.split_at(chroma_w * chroma_h);
                // A camera's own frames arrive upright: there is no rotation to undo.
                let image = crate::video::rgb_image(
                    luma,
                    u,
                    v,
                    (width, chroma_w, chroma_w),
                    (width, height),
                    (width, height),
                    0,
                );
                set(shared, State::Live(Arc::new(image)));
                ctx.request_repaint();
            }
            crate::camera::Read::Idle => {}
            crate::camera::Read::Ended => {
                set(shared, State::Failed);
                ctx.request_repaint();
                break;
            }
        }
    }
    source.stop();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shot_is_the_newest_picture_as_straight_rgba() {
        let image = ColorImage::new([2, 1], vec![egui::Color32::RED, egui::Color32::BLUE]);
        let viewfinder = Viewfinder::still(image);
        let (width, height, rgba) = viewfinder.shot().expect("a picture is on screen");
        assert_eq!((width, height), (2, 1));
        assert_eq!(rgba, vec![255, 0, 0, 255, 0, 0, 255, 255]);
    }

    #[test]
    fn there_is_no_shot_before_the_camera_delivers_or_after_it_fails() {
        let viewfinder = Viewfinder::still(ColorImage::new([1, 1], vec![egui::Color32::WHITE]));
        set(&viewfinder.shared, State::Opening);
        assert!(viewfinder.shot().is_none());
        set(&viewfinder.shared, State::Failed);
        assert!(viewfinder.shot().is_none());
    }

    #[test]
    fn the_texture_is_uploaded_again_only_for_a_new_picture() {
        let ctx = egui::Context::default();
        let mut viewfinder = Viewfinder::still(ColorImage::new([1, 1], vec![egui::Color32::WHITE]));
        let first = viewfinder.texture(&ctx).expect("a texture").id();
        assert_eq!(
            viewfinder.texture.as_ref().map(|(_, shown)| *shown),
            Some(1)
        );
        set(
            &viewfinder.shared,
            State::Live(Arc::new(ColorImage::new(
                [1, 1],
                vec![egui::Color32::BLACK],
            ))),
        );
        let second = viewfinder.texture(&ctx).expect("a texture").id();
        assert_eq!(first, second, "the same texture takes the new picture");
        assert_eq!(
            viewfinder.texture.as_ref().map(|(_, shown)| *shown),
            Some(2)
        );
    }
}
