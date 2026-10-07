//! Shared resident-scene loading and queries for the native development client.
#![forbid(unsafe_code)]
pub mod assets;
pub mod collisions;
pub mod models;
pub mod sprites;
pub mod world;
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
