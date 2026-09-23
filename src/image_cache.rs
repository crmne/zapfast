//! Releases egui's per-URI image caches for images that leave the screen.
//!
//! egui caches the bytes, the decoded pixels, and the GPU texture of every URI
//! it has loaded. It only ever evicts the textures of `.svg` files, so left
//! alone a session that scrolls through photos holds every one of them for
//! good. Each draw site registers its URI here, and [`sweep`] forgets the ones
//! that fall out of the resident window.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Image URIs kept resident. Anything the current frame drew is always kept,
/// so this only bounds what has scrolled away.
const RESIDENT: usize = 96;

struct Entry {
    /// Frame the image was last drawn on.
    frame: u64,
    /// Whether its bytes are registered with egui's byte loader. egui drops
    /// them once the texture is uploaded, so this is what says a later draw
    /// has to register them again.
    registered: bool,
}

#[derive(Clone, Default)]
struct Registry(Arc<Mutex<HashMap<String, Entry>>>);

fn registry(ctx: &egui::Context) -> Registry {
    ctx.data_mut(|data| {
        data.get_temp_mut_or_default::<Registry>(egui::Id::new("image-cache"))
            .clone()
    })
}

/// Registers `bytes` under `uri`, and marks the image as drawn this frame.
///
/// Re-registers them after [`sweep`] has forgotten the image, or after egui
/// has dropped them once the texture was uploaded.
pub fn include(ctx: &egui::Context, uri: String, bytes: &[u8]) {
    let frame = ctx.cumulative_frame_nr();
    let registry = registry(ctx);
    let mut entries = registry.0.lock().unwrap_or_else(|p| p.into_inner());
    let fresh = match entries.get_mut(&uri) {
        Some(entry) => {
            entry.frame = frame;
            !std::mem::replace(&mut entry.registered, true)
        }
        None => {
            entries.insert(
                uri.clone(),
                Entry {
                    frame,
                    registered: true,
                },
            );
            true
        }
    };
    drop(entries);
    if fresh {
        ctx.include_bytes(uri, bytes.to_vec());
    }
}

/// Marks an image URI as drawn this frame.
pub fn touch(ctx: &egui::Context, uri: &str) {
    let frame = ctx.cumulative_frame_nr();
    let registry = registry(ctx);
    let mut entries = registry.0.lock().unwrap_or_else(|p| p.into_inner());
    match entries.get_mut(uri) {
        Some(entry) => entry.frame = frame,
        None => {
            entries.insert(
                uri.to_owned(),
                Entry {
                    frame,
                    registered: false,
                },
            );
        }
    }
}

/// Forgets the images that have not been drawn for the longest.
///
/// Images the current frame drew are never forgotten, so a frame that shows
/// more than `RESIDENT` images does not reload them on the next one.
pub fn sweep(ctx: &egui::Context) {
    let frame = ctx.cumulative_frame_nr();
    let stale: Vec<String> = {
        let registry = registry(ctx);
        let mut entries = registry.0.lock().unwrap_or_else(|p| p.into_inner());
        let over = entries.len().saturating_sub(RESIDENT);
        if over == 0 {
            return;
        }
        let mut idle: Vec<(u64, String)> = entries
            .iter()
            .filter(|(_, entry)| entry.frame < frame)
            .map(|(uri, entry)| (entry.frame, uri.clone()))
            .collect();
        idle.sort_unstable();
        idle.truncate(over);
        for (_, uri) in &idle {
            entries.remove(uri);
        }
        idle.into_iter().map(|(_, uri)| uri).collect()
    };
    for uri in stale {
        ctx.forget_image(&uri);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> egui::Context {
        let ctx = egui::Context::default();
        egui_extras::install_image_loaders(&ctx);
        ctx
    }

    /// Advances the frame counter the way a repaint would.
    fn frame(ctx: &egui::Context) {
        let mut output = ctx.run_ui(egui::RawInput::default(), |_| {});
        output.textures_delta.clear();
    }

    fn registered(ctx: &egui::Context, uri: &str) -> bool {
        ctx.try_load_bytes(uri).is_ok()
    }

    fn uri(index: usize) -> String {
        format!("bytes://test-{index:04}")
    }

    /// Answers in the same frame, so a test can watch a texture being
    /// allocated without waiting on a decoding thread.
    struct ReadyLoader;

    impl egui::load::ImageLoader for ReadyLoader {
        fn id(&self) -> &str {
            "zapfast::image_cache::tests::ReadyLoader"
        }

        fn load(
            &self,
            _ctx: &egui::Context,
            _uri: &str,
            _size_hint: egui::load::SizeHint,
        ) -> egui::load::ImageLoadResult {
            Ok(egui::load::ImagePoll::Ready {
                image: std::sync::Arc::new(egui::ColorImage::example()),
            })
        }

        fn forget(&self, _uri: &str) {}

        fn forget_all(&self) {}

        fn byte_size(&self) -> usize {
            0
        }
    }

    /// Draws `shown` and then sweeps, the way a frame of the app does.
    fn draw(ctx: &egui::Context, shown: &[String]) -> egui::FullOutput {
        ctx.run_ui(egui::RawInput::default(), |ui| {
            for uri in shown {
                ui.add(egui::Image::from_uri(uri.clone()));
            }
            sweep(ui.ctx());
        })
    }

    fn allocated(ctx: &egui::Context) -> usize {
        let manager = ctx.tex_manager();
        manager.read().num_allocated()
    }

    #[test]
    fn images_the_current_frame_drew_are_never_released() {
        let ctx = context();
        let total = RESIDENT + 40;
        for index in 0..total {
            include(&ctx, uri(index), b"image bytes");
        }
        // A frame that shows more images than the window holds must not drop
        // the ones it is showing right now.
        sweep(&ctx);
        for index in 0..total {
            assert!(registered(&ctx, &uri(index)), "{} was released", uri(index));
        }
    }

    #[test]
    fn images_that_scroll_away_are_released() {
        let ctx = context();
        let total = RESIDENT + 40;
        for index in 0..total {
            include(&ctx, uri(index), b"image bytes");
        }
        frame(&ctx);
        sweep(&ctx);
        let kept = (0..total)
            .filter(|index| registered(&ctx, &uri(*index)))
            .count();
        assert_eq!(kept, RESIDENT, "the window should hold exactly its budget");
    }

    #[test]
    fn a_released_image_is_registered_again_when_it_comes_back() {
        let ctx = context();
        for index in 0..RESIDENT + 40 {
            include(&ctx, uri(index), b"image bytes");
        }
        frame(&ctx);
        sweep(&ctx);
        // The oldest URI lost its bytes. Drawing it again must put them back,
        // because egui has no other way to resolve a `bytes://` image.
        include(&ctx, uri(0), b"image bytes");
        assert!(registered(&ctx, &uri(0)));
    }

    #[test]
    fn images_that_scroll_away_free_their_texture() {
        let ctx = context();
        ctx.add_image_loader(std::sync::Arc::new(ReadyLoader));
        let uris: Vec<String> = (0..RESIDENT + 8).map(uri).collect();
        for uri in &uris {
            include(&ctx, uri.clone(), b"image bytes");
        }
        let mut showing = draw(&ctx, &uris);
        let shown = allocated(&ctx);
        assert!(
            shown >= uris.len(),
            "a drawn image should own a texture, saw {shown} for {} images",
            uris.len()
        );
        showing.textures_delta.clear();

        // Nothing on screen: the images past the window go, and the frame that
        // released them is the one that tells the painter to delete them.
        let mut empty = draw(&ctx, &[]);
        assert_eq!(
            empty.textures_delta.free.len(),
            8,
            "the swept textures should be freed by the painter"
        );
        assert_eq!(allocated(&ctx), shown - 8);
        empty.textures_delta.clear();
    }
}
