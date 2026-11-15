//! Resource decoding for the pinned Java Edition client.
#![forbid(unsafe_code)]

pub mod animation;
pub mod bake;
pub mod blockstate;
pub mod item_bake;
pub mod item_model;
mod json;
pub mod math;
pub mod model;
pub mod model_loaders;
pub mod obj;
pub mod palette;
pub mod resolve;
pub mod resource_source;
#[cfg(not(target_arch = "wasm32"))]
pub mod zip;

pub use json::DecodeError;
