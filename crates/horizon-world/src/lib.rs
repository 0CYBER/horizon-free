//! Vanilla-compatible world data for Horizon. This crate does not run gameplay.
#![forbid(unsafe_code)]

pub mod block_collision;
pub mod block_properties;
pub mod block_support;
pub mod block_survival;
pub mod chunk;
pub mod collision;
pub mod compression;
pub mod dynamic_support;
pub mod entity_obstruction;
pub mod fluid;
pub mod heightmap;
pub mod nbt;
pub mod piston_shape;
pub mod region;
pub mod registry;
pub mod tick_data;
pub mod worldgen;

pub const DATA_VERSION: i32 = 5023;
