//! Blocks, block states and block entities.

pub(crate) mod block_entity_data;
pub(crate) mod block_entity_items;
pub(crate) mod block_entity_type;
/// Read-only block metadata and typed operation errors.
pub mod block_metadata;
pub(crate) mod block_screen_effect;
// hidden: a module of game-side types; mods use their re-exports
#[doc(hidden)]
pub mod block_sprites;
pub(crate) mod block_state;
pub(crate) mod block_state_range;
pub(crate) mod block_states;
pub(crate) mod block_tint;
pub(crate) mod bundle_items;
pub(crate) mod cauldron_fluid_content;
pub(crate) mod cauldron_interactions;
pub(crate) mod fluid_capability;
pub(crate) mod fluid_model;
pub(crate) mod fluid_states;
/// Fluid interaction helpers, as NeoForge's `FluidUtil`.
pub mod fluid_util;
pub(crate) mod item_place;
pub(crate) mod part_state;
pub(crate) mod poi_types;
pub(crate) mod rail_shape;
pub(crate) mod types;
pub(crate) mod unresolved_fluid;
