//! Client-only gameplay: what vanilla's client jar adds to the logical side.
#![forbid(unsafe_code)]
extern crate horizon_client_lib as horizon_client;
pub mod occlusion_culling;
pub mod particle_culling;
pub mod section_scene;
pub mod transient_blocks;
