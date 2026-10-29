//! ZapFast internals exposed for diagnostics and tests.

pub mod a11y;
pub mod account;
pub mod announcer;
pub mod app;
pub mod backdrop;
#[cfg(any(test, feature = "demo"))]
pub mod demo;
pub mod image_preview;
#[cfg(target_os = "macos")]
pub mod macos;
pub mod nav;
pub mod ui;

#[cfg(target_os = "linux")]
pub use zapfast_core::appearance;
pub use zapfast_core::{
    animation, app_lock, archive, audio, autostart, backend, bidi, call_audio, call_diag,
    call_tones, calls, camera, diagnostics, emoji, geo, group_admin, i18n, image_cache, markup,
    media_pause, mentions, model, motion, new_group, notify, opener, paths, privacy, proxy, qr,
    safety, settings, single_instance, sticker_meta, sticker_search, target, theme, timestretch,
    transcript, transport, updates, util, video, video_rate, voice, wallpaper,
};

#[cfg(test)]
mod tests {
    #[test]
    fn the_core_crate_carries_the_app_version() {
        assert_eq!(
            crate::updates::CONFIG.current_version,
            env!("CARGO_PKG_VERSION")
        );
    }
}
