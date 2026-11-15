//! The `client_level_load` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0140

pub trait Hooks {
    /// The client made a new level for `dimension` (NeoForge's
    /// `LevelEvent.Load` from the `ClientLevel` constructor): on joining a
    /// server, single player's own included, and on changing dimension.
    /// Its chunks come after. When the dimension changes, the old level's
    /// `client_level_unload` follows this, as `Minecraft.setLevel`
    /// tells it after the new level is made. Client-side, on the window; the
    /// server's levels have `level_load`.
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "LevelEvent.Load",
        cancellable = false
    )]
    fn client_level_load();
}
