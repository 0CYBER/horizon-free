//! Items, stacks, enchantments, recipes, menus and what a use of an item answers.

pub(crate) mod capability;
pub(crate) mod capability_cache;
// hidden: a module of game-side types; mods use their re-exports
#[doc(hidden)]
pub mod containers;
// hidden: a module of game-side types; mods use their re-exports
#[doc(hidden)]
pub mod crafting;
pub(crate) mod enchantment_damage;
// hidden: a module of game-side types; mods use their re-exports
#[doc(hidden)]
pub mod enchantments;
pub(crate) mod energy_capability;
pub(crate) mod item_component_handlers;
// hidden: a module of game-side types; mods use their re-exports
#[doc(hidden)]
pub mod item_attributes;
pub(crate) mod item_model_context;
/// Owned item values and typed component patches.
pub mod item_stack;
// hidden: a module of game-side types; mods use their re-exports
#[doc(hidden)]
pub mod item_tooltip;
// hidden: a module of game-side types; mods use their re-exports
#[doc(hidden)]
pub mod menu;
pub(crate) mod menu_slot;
pub(crate) mod menu_slot_place;
// hidden: owned trading values are re-exported at the API root
#[doc(hidden)]
pub mod merchant;
// hidden: a module of game-side types; mods use their re-exports
#[doc(hidden)]
pub mod recipe_book;
pub(crate) mod recipe_rule;
// hidden: a module of game-side types; mods use their re-exports
#[doc(hidden)]
pub mod stations;
pub(crate) mod storage;
pub(crate) mod tooltip_appender;
pub(crate) mod tooltip_element;
pub(crate) mod transaction;
pub(crate) mod types;
