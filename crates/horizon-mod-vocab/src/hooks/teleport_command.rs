//! The `teleport_command` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A server entity is about to be teleported by `/tp` or `/teleport` (NeoForge's
    /// `EntityTeleportEvent.TeleportCommand`, in `TeleportCommand.performTeleport`, once per
    /// teleported entity): `dimension` is the level it arrives in, `entity` its network id,
    /// `from` its feet now and `to` where the command sends it. A mod may move `to` (a value
    /// that is not a number is ignored). Canceling leaves the entity where it is; the command
    /// still reports success. `entity_teleport` and `player_teleported` hear it after.
    /// Server-side. Diverges from upstream: new hook.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "EntityTeleportEvent.TeleportCommand",
        cancellable = true
    )]
    fn teleport_command();
}
