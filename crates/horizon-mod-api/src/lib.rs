//! The Horizon mod API: the one crate a Horizon code mod depends on.
//!
//! Horizon is Minecraft: Java Edition 26.3 written in Rust. A code mod is a Rust crate that implements
//! [`Hooks`], built either natively by the Horizon launcher or to `mod.wasm` for the game's sandbox. Both
//! targets use this same API; only the build differs. Add it to your mod's `Cargo.toml` as
//! `horizon-mod-api` (the library name is `mod_api`).
//!
//! The [modding book](https://arismoko.github.io/horizon/) is the guide: start with
//! [Your first native mod](https://arismoko.github.io/horizon/start/first-mod.html). This reference documents
//! every item of the API.
//!
//! # A first mod
//!
//! ```ignore
//! use mod_api::{self, Hooks, PlayerLoggedInEvent, ServerHost, Text};
//!
//! #[derive(Default)]
//! struct Mod;
//!
//! #[mod_api::horizon_mod]
//! impl Hooks for Mod {
//!     /// A player joined the server: greet them.
//!     fn player_logged_in(&self, host: &mut dyn ServerHost, event: &mut PlayerLoggedInEvent<'_>) {
//!         let line = format!("greeter says hello, {}!", event.name);
//!         if let Some(player) = host.player(event.player) {
//!             let _ = player.tell(host, Text::literal(line));
//!         }
//!     }
//! }
//! ```
//!
//! `#[horizon_mod]` marks the one `impl Hooks` at the crate root and subscribes the mod to exactly the hooks it
//! implements. The book's examples crate compiles this mod against the API.
//!
//! # Where to start
//!
//! | Item | What it is |
//! | --- | --- |
//! | [`Hooks`] | Every moment a mod can answer, one method each, with its side, its NeoForge 26.3 counterpart and whether it can be cancelled. |
//! | [`BlockPart`], [`ItemPart`], [`BlockEntityPart`], [`MenuPart`], [`EntityPart`] | Behaviour for the objects a mod attaches a Part to. |
//! | [`Registrar`] | What a mod declares in [`Hooks::register`]: registry keys, Parts and settings. |
//! | [`Host`], [`ServerHost`], [`ClientHost`] | The game as a hook sees it, split by side so a call on the wrong side does not compile. |
//! | [`AttachmentType`] | Data a mod keeps on levels, chunks, entities and other holders. |
//! | [`Wire`](derive@Wire) | The derive that lets a mod's own values cross to and from a WebAssembly mod. |
//!
//! An item whose documentation says **Not on wasm** is unavailable to WebAssembly mods.
//!
//! # Guides in the book
//!
//! - [Hooks and the Host](https://arismoko.github.io/horizon/concepts/hooks-and-the-host.html)
//! - [Load phases and access](https://arismoko.github.io/horizon/concepts/load-phases.html)
//! - [Parts](https://arismoko.github.io/horizon/concepts/parts.html),
//!   [capabilities](https://arismoko.github.io/horizon/concepts/capabilities.html) and
//!   [attachments](https://arismoko.github.io/horizon/concepts/attachments.html)
//! - [Wire types](https://arismoko.github.io/horizon/concepts/wire-types.html)
//! - [The manifest and the package](https://arismoko.github.io/horizon/reference/manifest.html)
extern crate self as mod_api;
mod equip_reads;

pub use mod_vocab::*;
/// Shared Wire values and the mod-side transport.
pub mod wire {
    pub use mod_vocab::wire::*;
    #[path = "wire_resources.rs"]
    mod resources;
    mod static_strings;
    pub(crate) fn initialize() -> Result<(), &'static str> {
        resources::ensure_installed()
    }
    #[doc(hidden)]
    pub use resources::{
        ResourceCodec, ResourceScope, ScopeError, ScopeLease, ScopeToken, borrow_token,
    };
    /// The mod side of the WebAssembly boundary.
    #[cfg(any(target_arch = "wasm32", feature = "wasm-guest"))]
    pub mod wire_guest;
    #[cfg(any(target_arch = "wasm32", feature = "wasm-guest"))]
    pub use wire_guest as guest;
}

/// Platform-owned crash attribution with shared vocabulary types.
#[doc(hidden)]
pub mod content {
    #[doc(hidden)]
    pub mod crash;
}
/// Platform runtime operations and engine-owned vocabulary installations.
#[doc(hidden)]
pub mod __private;

mod load_state;
#[doc(hidden)]
pub use load_state::install_guest_packages;

mod fluid_state;
mod parts;

#[cfg(any(test, feature = "testing"))]
pub mod testing;
#[cfg(all(test, feature = "wasm-guest"))]
mod phase_tests {
    use super::*;
    use prelude::*;
    #[test]
    fn frame_services_do_not_read_guest_phase() {
        let _phase = __private::PhaseOwner::scope(__private::ModPhase::Packages);
        let mut guest = wire::guest::Guest;
        let _: &dyn Messages = guest.messages();
        let _: &dyn Players = guest.players();
        let _: &dyn Levels = guest.levels();
        let _: &mut dyn Commands = guest.commands();
    }
}
