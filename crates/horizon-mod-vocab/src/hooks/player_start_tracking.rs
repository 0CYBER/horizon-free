//! The `player_start_tracking` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0147

pub trait Hooks {
    /// A client started tracking an entity (NeoForge's
    /// `PlayerEvent.StartTracking`, posted at the end of
    /// `ServerEntity.addPairing`): `player` (`name`) is now sent `entity`
    /// (its ID, `uuid` its UUID, `kind` its type, `minecraft:zombie`) and its
    /// changes. It comes into range, its client's chunks reach it, or it is
    /// put in the level; another player is an entity too, and a player never
    /// tracks their own. A mod sends the player what they need to show it
    /// (`PlayerRef::send_payload`). The entity is as it was last tick: it
    /// may be gone by now. Heard after the tick's players and entities have
    /// moved, in order, with no player's state swapped in. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "PlayerEvent.StartTracking",
        cancellable = false
    )]
    fn player_start_tracking();
}
