//! Shader packs: the `shaderpacks/*.zip` format, written from Horizon's clean-room specification
//! (`shader-packs-spec.md`, revision 1). This crate reads the format and knows nothing of wgpu;
//! the game's renderer runs what it produces.
//!
//! - [`Pack`]: a pack as a folder or a zip, with paths relative to its `shaders` folder (spec 1.1).
//! - [`discover`]: the packs in a `shaderpacks` folder.
//! - [`Properties`]: `shaders.properties` and the id files, with their conditional directives (3.1, 1.5).
//! - [`Lang`]: the pack's translated names (8.4).
//! - [`preprocess`]: a stage file through `#include` and the GLSL preprocessor, lines mapped back (1.4, 1.5).
//! - [`translate_composite`], [`translate_gbuffers`], [`translate_shadow`]: a program into naga modules and its interface (1.6, 2.2).
//! - [`Config`] and [`draw_buffers`]: the buffer settings and output lists stage files carry (1.7, 6.1).
//! - [`load_post`]: a dimension's `prepare`, `deferred`, `composite` and `final` programs, in order (2.1).
//! - [`Macros`] and [`eval_condition`]: the macros packs test, and the `#if` arithmetic (1.5).

mod biomes;
mod block_ids;
mod condition;
mod config;
mod custom;
mod discover;
mod lang;
mod macros;
mod options;
mod pack;
mod post;
mod preprocess;
mod properties;
mod translate;

pub use biomes::{category as biome_category, precipitation as biome_precipitation};
pub use block_ids::BlockIds;
pub use condition::eval_condition;
pub use config::{BUFFERS, Buffer, Config, Format, Kind, Shadow, buffer_index, draw_buffers};
pub use custom::{Custom, Val};
pub use discover::{Found, discover};
pub use lang::Lang;
pub use macros::{MC_VERSION, Macros, RENDER_STAGES, render_stage};
pub use options::switches_on;
pub use pack::{Pack, PackError};
pub use post::{
    Blend, CELESTIAL_BLEND, Factor, GameInputs, Group, Post, PostProgram, TRANSLUCENT_BLEND,
    dimension_folder, load_post,
};
pub use preprocess::{Diagnostic, Origin, Preprocessed, preprocess};
pub use properties::{Entry, Note, Properties};
pub use translate::{
    AlphaTest, DRAW_UNIFORMS, Error, GbuffersInputs, Interface, Program, Sampler, Target, Uniform,
    translate_composite, translate_gbuffers, translate_shadow,
};
