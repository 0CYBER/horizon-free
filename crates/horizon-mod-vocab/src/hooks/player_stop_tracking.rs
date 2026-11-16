//! The `player_stop_tracking` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0148

pub trait Hooks {
    /// A client stopped tracking an entity (NeoForge's
    /// `PlayerEvent.StopTracking`, posted at the end of
    /// `ServerEntity.removePairing`): it went out of range, left the level or
    /// died, or `player` (`name`) left, changed level or respawned, which
    /// stops everything they tracked. `entity`, `uuid` and `kind` are as
    /// `player_start_tracking` told. Once for each start, a leaving
    /// player's after `player_logged_out`. The entity is not
    /// necessarily still there, and a player who left is gone: no player's
    /// state is swapped in. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "PlayerEvent.StopTracking",
        cancellable = false
    )]
    fn player_stop_tracking();
}
