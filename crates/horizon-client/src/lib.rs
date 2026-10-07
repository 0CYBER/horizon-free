//! Shared resident-scene loading and queries for the native development client.
#![forbid(unsafe_code)]
pub mod assets;
pub mod attribute_updates;
pub mod clock;
pub mod collisions;
pub mod health;
pub mod hunger;
pub mod input;
pub mod landing;
pub mod models;
pub mod motion;
pub mod pose;
pub mod sprint;
pub mod sprites;
pub mod world;
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
