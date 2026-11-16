//! The `player_clone` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0248

pub trait Hooks {
    /// A player's data was copied to their new player (NeoForge's
    /// `PlayerEvent.Clone`, from `ServerPlayer.restoreFrom`): `was_death` is
    /// true after a death, false when they left the End after the credits
    /// (`respawn` with `keepAllPlayerData`). `original` is a hook-scoped view
    /// of the old player's attachments (all types), inventory, position and
    /// level. `player` is the replacement's UUID. Heard before placement on
    /// a server, without swapping a player's state in. Not cancellable.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "PlayerEvent.Clone (not cancellable)",
        cancellable = false
    )]
    fn player_clone();
}
