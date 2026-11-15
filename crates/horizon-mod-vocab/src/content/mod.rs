//! Declared content, config files, registry keys and the crash report.

pub(crate) mod attached_storage;
pub(crate) mod config_files;
pub(crate) mod content_declarations;
// hidden: a module of game-side types; mods use their re-exports
#[doc(hidden)]
pub mod crash;
pub(crate) mod declarations;
pub(crate) mod file_to_id_converter;
pub(crate) mod game_event_key;
pub(crate) mod jar_inputs_error;
pub(crate) mod mod_settings;
pub(crate) mod package_side;
pub(crate) mod packets;
pub(crate) mod payload_budget;
pub(crate) mod payload_codec;
pub(crate) mod payload_schema;
pub(crate) mod registry_entry;
pub(crate) mod registry_keys;
pub(crate) mod resource;

pub(crate) mod professions;

pub(crate) mod load_conditions;
