//! Client-only surfaces: screens, input, sound, drawing and options.

pub(crate) mod baked_models;
pub(crate) mod block_state_definition;
// hidden: a module of game-side types; mods use their re-exports
#[doc(hidden)]
pub mod camera;
pub(crate) mod client_reload;
pub(crate) mod client_session;
pub(crate) mod closing_save;
pub(crate) mod debug_entries;
pub(crate) mod entity_layers;
pub(crate) mod entity_renderers;
pub(crate) mod export;
// hidden: a module of game-side types; mods use their re-exports
#[doc(hidden)]
pub mod extras;
// hidden: a module of game-side types; mods use their re-exports
#[doc(hidden)]
pub mod extras_hud;
// hidden: a module of game-side types; mods use their re-exports
#[doc(hidden)]
pub mod first_person;
pub(crate) mod fluid_draw;
// hidden: a module of game-side types; mods use their re-exports
#[doc(hidden)]
pub mod fog;
pub(crate) mod gui_clip;
pub(crate) mod gui_entity;
pub(crate) mod gui_layers;
// hidden: a module of game-side types; mods use their re-exports
#[doc(hidden)]
pub mod hud_layout;
// hidden: module of API-owned HUD values; mods use the re-export
#[doc(hidden)]
pub mod hud_mode;
pub(crate) mod input_glyph;
pub(crate) mod key_mapping;
pub(crate) mod keyframe_animation;
pub(crate) mod loading_overlay;
// hidden: a module of game-side types; mods use their re-exports
#[doc(hidden)]
pub mod mesher;
pub(crate) mod model_instance;
pub(crate) mod model_layers;
pub(crate) mod model_loader;
// hidden: a module of game-side types; mods use their re-exports
#[doc(hidden)]
pub mod music;
pub(crate) mod narration;
pub(crate) mod pack_finders;
pub(crate) mod particle_keys;
pub(crate) mod particle_providers;
mod particle_stacks;
// hidden: a module of game-side types; mods use their re-exports
#[doc(hidden)]
pub mod particles;
// hidden: a module of game-side types; mods use their re-exports
#[doc(hidden)]
pub mod player_chat;
pub(crate) mod posed_parts;
pub(crate) mod render_pipeline;
pub(crate) mod replay;
pub(crate) mod scene;
pub(crate) mod screenshot;
pub(crate) mod skull_models;
pub(crate) mod special_model_renderers;
pub(crate) mod star_mesh;
pub(crate) mod terrain_draw;
pub(crate) mod title_request;
pub(crate) mod types;
/// Buttons and text fields for mod screens.
pub mod widgets;
pub(crate) mod world_draw;
pub(crate) mod world_request;

pub(crate) mod spectator_shader;

pub(crate) mod arm_pose;
