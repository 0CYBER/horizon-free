//! Resource decoding for the pinned Java Edition client.
#![forbid(unsafe_code)]

pub mod animation;
pub mod bake;
mod json;
pub mod model;
pub mod resolve;

pub use json::DecodeError;
