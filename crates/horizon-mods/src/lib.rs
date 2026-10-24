//! Horizon's mods (`docs/mods.md`): WebAssembly components written against
//! `crates/horizon-mod-sdk/wit/horizon.wit`, each in its own sandbox with
//! no file, network or clock access, a memory cap and a time budget per
//! call. A mod that traps or overruns is turned off; the game goes on.
//!
//! Nothing here runs unless a mod is installed and turned on: an empty
//! [`Mods`] makes no engine and every hook returns at once.
pub mod content;
pub mod manifest;
#[cfg(feature = "runtime")]
mod runtime;

pub use content::Content;
pub use manifest::{Capability, Manifest, Package, Side, find};
#[cfg(feature = "runtime")]
pub use runtime::*;
