//! A mod's own source as the mod tools read it, without the generator that builds the game's
//! API: the mod folder the uses scan reads ([`mod_uses`]), a crate's module files
//! ([`modules`]) and the Wire types a mod lets cross ([`wire_types`]).
pub mod mod_uses;
pub mod modules;
pub mod wire_types;
