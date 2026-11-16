//! The `entity_tick_pre` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0154

pub trait Hooks {
    /// An entity of type `kind` (`minecraft:zombie`, `minecraft:item`) with ID `entity` is
    /// about to tick in the server level `dimension` (NeoForge's `EntityTickEvent.Pre`, posted
    /// around `Entity.commonTick` and `tick` in `ServerLevel.tickNonPassenger`). Set `canceled`
    /// and it does not tick this tick: a mob has no AI, physics or effects, an item no movement,
    /// ageing or merging, a minecart or boat no motion, and `entity_tick_post` is not told.
    /// Horizon ticks a family of entities together, so mobs and items hear every Pre before the
    /// first of them ticks; minecarts and boats hear theirs one by one. Told for the mobs, item
    /// entities, minecarts and boats that tick: those in an entity-ticking chunk, not a player's
    /// boat or cart (it ticks with the player). Check `kind`; this runs for every ticking entity
    /// each tick, so keep it cheap. Server-side.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "canceled",
        neoforge = "EntityTickEvent.Pre",
        cancellable = true
    )]
    fn entity_tick_pre();
}
