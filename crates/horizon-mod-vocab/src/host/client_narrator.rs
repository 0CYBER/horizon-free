//! The client narrator, reached by mods.
#![allow(unused_variables)]
use crate::*;

/// Lines a client mod hands the player's narrator (`Minecraft.getNarrator()`).
#[horizon_mod_api_macros::host_area]
pub trait ClientNarratorClientHost {
    /// Side: client.
    /// Has the narrator say `text` as a system message, through the same
    /// `GameNarrator` the game's own narration uses: `Narration::Now` is
    /// `saySystemNow` (cuts off what is being said), `Narration::Queued` is
    /// `saySystemQueued`. The player's Narrator option, the voice volume and
    /// the narrator's log of recent lines apply as to vanilla's lines. The
    /// text is the component's plain string (`Component.getString`) in the
    /// current language, bounded only by the component's own form limits.
    /// Nothing reaches the server. Refused with nothing said, by name, as
    /// [`NarrateError`] lists: an empty line, the option Off or Chat only, or
    /// no speech. Available from client setup on, on native and wasm alike.
    #[access(ClientSetup)]
    fn narrate(&mut self, text: &Text, when: Narration) -> Result<(), NarrateError>;
}
