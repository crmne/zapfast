//! The Matrix account adapter. It runs one matrix-sdk client per account,
//! stores its state under the account directory, and projects rooms and
//! events through [`project`].
//!
//! Decrypted content is only painted once this session is verified; until
//! then encrypted messages show a placeholder. Interactive verification uses
//! the emoji SAS method.

pub mod project;
mod runtime;

pub(crate) use runtime::run;
