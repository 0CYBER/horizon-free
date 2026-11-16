//! The `entity_tick_post` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0155

pub trait Hooks {
    /// The entity `entity` of type `kind` ticked in the server level `dimension` (NeoForge's
    /// `EntityTickEvent.Post`): for each entity `entity_tick_pre` let through, after it
    /// ticked, also one that died or was removed in its tick. Mobs and items hear their Posts
    /// once the whole family has ticked; minecarts and boats right after their own tick. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "EntityTickEvent.Post",
        cancellable = false
    )]
    fn entity_tick_post();
}
