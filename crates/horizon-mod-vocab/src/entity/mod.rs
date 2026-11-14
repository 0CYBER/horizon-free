//! Entities, mobs, players, effects and their handles.

pub(crate) mod attachment_holder;
pub(crate) mod attachment_query;
pub(crate) mod chunk_areas;
pub(crate) mod player_data;
// hidden: a module of game-side types; mods use their re-exports
#[doc(hidden)]
pub mod behaviors;
pub(crate) mod effect_category;
pub(crate) mod entity_capability;
pub(crate) mod goals;
// hidden: a module of game-side types; mods use their re-exports
#[doc(hidden)]
pub mod health;
// hidden: a module of game-side types; mods use their re-exports
#[doc(hidden)]
pub mod mobs;
pub(crate) mod player_bots;
pub(crate) mod types;
pub(crate) mod unspawned_mob;
