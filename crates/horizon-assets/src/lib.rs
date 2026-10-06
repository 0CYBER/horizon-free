//! Resource decoding for the pinned Java Edition client.
#![forbid(unsafe_code)]

mod json;
pub mod model;
pub mod resolve;

pub use json::DecodeError;
