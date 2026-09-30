//! One adapter per network. WhatsApp keeps the original worker under
//! [`crate::backend`]; every other network projects its own protocol into the
//! shared chat and message models.

pub mod matrix;
pub mod slack;
pub mod telegram;
