//! Shader packs: the `shaderpacks/*.zip` format, written from Horizon's clean-room specification
//! (`shader-packs-spec.md`, revision 1). This crate reads the format and knows nothing of wgpu;
//! the game's renderer runs what it produces.
//!
//! - [`Pack`]: a pack as a folder or a zip, with paths relative to its `shaders` folder (spec 1.1).
//! - [`discover`]: the packs in a `shaderpacks` folder.
//! - [`Lang`]: the pack's translated names (8.4).

mod discover;
mod lang;
mod pack;

pub use discover::{Found, discover};
pub use lang::Lang;
pub use pack::{Pack, PackError};
