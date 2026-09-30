//! ZapFast internals exposed for diagnostics and tests.

// The Matrix SDK's generic async types nest deeply. Test builds pull extra
// features into the shared dependency graph, and type-checking the adapter
// against that build needs more trait-query depth than the default.
#![recursion_limit = "512"]

pub mod account;
pub mod adapter;
pub mod animation;
pub mod app;
pub mod app_lock;
pub mod archive;
pub mod audio;
pub mod autostart;
pub mod backend;
pub mod bidi;
#[cfg(any(test, feature = "demo"))]
pub mod demo;
pub mod diagnostics;
pub mod emoji;
pub mod host;
pub mod i18n;
pub mod image_cache;
pub mod image_preview;
pub mod lottie;
#[cfg(target_os = "macos")]
pub mod macos;
pub mod markup;
pub mod media_pause;
pub mod model;
pub mod notify;
pub mod opener;
pub mod paths;
pub mod privacy;
pub mod proxy;
pub mod qr;
pub mod safety;
pub mod secrets;
pub mod settings;
pub mod single_instance;
pub mod sticker_meta;
pub mod sticker_search;
pub mod theme;
pub mod timestretch;
pub mod transcript;
pub mod transport;
pub mod ui;
pub mod updates;
pub mod util;
pub mod video;
pub mod voice;
pub mod wallpaper;
