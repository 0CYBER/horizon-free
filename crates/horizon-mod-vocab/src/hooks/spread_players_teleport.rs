//! The `spread_players_teleport` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A server entity is about to be placed by `/spreadplayers` (NeoForge's
    /// `EntityTeleportEvent.SpreadPlayersCommand`, vanilla's
    /// `SpreadPlayersCommand.setPlayerPositions`): once per entity, after the
    /// spread worked out its spot. `entity` is its network id, `player` its UUID (None for a
    /// non-player), `from` where
    /// it stands and `target` the spot. A mod may move `target` (a value that
    /// is not a number is ignored). Canceling leaves the entity where it is;
    /// the spread's other entities, its random draws and its feedback are
    /// unchanged.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "EntityTeleportEvent.SpreadPlayersCommand",
        cancellable = true
    )]
    fn spread_players_teleport();
}
