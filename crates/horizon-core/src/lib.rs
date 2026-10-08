//! Horizon's initial simulation primitives, targeting Minecraft Java Edition 26.3.
//!
//! This is not a playable game. Differential checks cover only the operations
//! listed in docs/rust-parity.md; the rest of vanilla remains unimplemented.
#![forbid(unsafe_code)]

pub mod biome;
pub mod carving;
pub mod density;
pub mod geometry;
pub mod noise;
pub mod position;
pub mod random;
pub mod storage;
pub mod text;
pub mod ticks;

pub const VANILLA_VERSION: &str = "26.3";
