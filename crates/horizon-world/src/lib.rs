//! Vanilla-compatible world data for Horizon. This crate does not run gameplay.
#![forbid(unsafe_code)]

pub mod block_collision;
pub mod block_collision_scan;
pub mod block_entities;
pub mod block_placement;
pub mod block_properties;
pub mod block_render;
pub mod block_support;
pub mod block_survival;
pub mod chunk;
pub mod chunk_status;
pub mod collision;
pub mod collision_context;
pub mod compression;
pub mod dynamic_support;
pub mod entity_collision;
pub mod entity_obstruction;
pub mod fluid;
pub mod heightmap;
pub mod liquid_collision;
pub mod nbt;
pub mod piston_shape;
pub mod powder_snow_collision;
pub mod region;
pub mod registry;
pub mod registry_collision_scan;
pub mod retrogen;
pub mod suffocation;
pub mod tick_data;
pub mod world_movement;
pub mod worldgen;

pub const DATA_VERSION: i32 = 5023;
