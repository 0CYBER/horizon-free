//! The `player_teleported` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0264

pub trait Hooks {
    /// A server player a command teleported (NeoForge's
    /// `EntityTeleportEvent`, for a player): `/tp` and `/teleport`, inside
    /// their level or into another one. `from` and `to` are their feet before
    /// and after, `dimension` the level they are in now. Heard after the move
    /// has reached the player's state and their client (an inter-level
    /// teleport: when the new level is entered, before it has loaded around
    /// them), so it cannot stop it. The player's state is swapped in, so the
    /// helpers that read the acting player and `PlayerRef::tell` work. Once for
    /// each teleported player, on a server. Ender pearls and chorus fruit
    /// have their own hooks; a spectator's teleport menu is not heard yet;
    /// portals and respawns are not teleports here.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (EntityTeleportEvent.TeleportCommand fires before the move and can cancel it; this hook is told after the move)"
    )]
    fn player_teleported();
}
