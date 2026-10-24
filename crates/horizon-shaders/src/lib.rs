//! Shader packs: the `shaderpacks/*.zip` format, written from Horizon's clean-room specification
//! (`shader-packs-spec.md`, revision 1). This crate reads the format and knows nothing of wgpu;
//! the game's renderer runs what it produces.
//!
//! - [`Pack`]: a pack as a folder or a zip, with paths relative to its `shaders` folder (spec 1.1).
//! - [`discover`]: the packs in a `shaderpacks` folder.
//! - [`Properties`]: `shaders.properties` and the id files, with their conditional directives (3.1, 1.5).
//! - [`Lang`]: the pack's translated names (8.4).
//! - [`Macros`] and [`eval_condition`]: the macros packs test, and the `#if` arithmetic (1.5).

mod condition;
mod discover;
mod lang;
mod macros;
mod pack;
mod properties;

pub use condition::eval_condition;
pub use discover::{Found, discover};
pub use lang::Lang;
pub use macros::{MC_VERSION, Macros};
pub use pack::{Pack, PackError};
pub use properties::{Entry, Note, Properties};
