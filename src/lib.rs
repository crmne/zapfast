//! ZapFast internals exposed for diagnostics and tests.

pub mod account;
pub mod diagnostics;
pub mod model;
pub mod nav;
pub mod settings;
pub mod target;
pub mod util;

pub mod app;
pub mod archive;
pub mod audio;
pub mod backend;
pub mod call_audio;
pub mod calls;
#[cfg(any(test, feature = "demo"))]
pub mod demo;
pub mod media;
pub mod net;
pub mod platform;
pub mod security;
pub mod style;
pub mod text;
pub mod ui;

pub use calls::{call_diag, call_tones, camera, timestretch};
pub use media::*;
pub use net::*;
pub use platform::*;
pub use security::*;
pub use style::*;
pub use text::*;
