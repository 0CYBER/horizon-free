//! The `ender_entity_teleport` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// An enderman or a shulker is about to teleport on its own
    /// (`EntityTeleportEvent.EnderEntity`). A mod may move `target` or cancel. It is given the loop `context` of the side that ticks the entity (its registries, settings and mod list); it fires inside the tick's own AI step, before the teleport's own random draws, so it takes no `Host`.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        context = "loop",
        neoforge = "EntityTeleportEvent.EnderEntity",
        cancellable = true,
        version = 2,
        target = "horizon_client_lib::mobs::enderman::native_ender_entity_teleport"
    )]
    fn ender_entity_teleport();
}
