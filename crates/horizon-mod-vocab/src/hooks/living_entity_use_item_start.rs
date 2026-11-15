//! The `living_entity_use_item_start` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0104

pub trait Hooks {
    /// NeoForge's `LivingEntityUseItemEvent.Start`: `entity` starts using
    /// `stack` in `hand` (`LivingEntity.startUsingItem`), for `duration`
    /// ticks (`use_duration`'s answer). `entity` is a mob's
    /// `Entity.getId`, None for a player (`host.players().acting_player()` says who);
    /// a mob fires it only for `LivingRef::start_using_item`. Change `duration` (consumables, potions,
    /// spyglasses and mods' items count to it; below 0 refuses the use, and
    /// 0 runs it out on its first tick, as `living_entity_use_item_tick`
    /// running it out, unless the item's own is 0 already), or set
    /// `canceled` and the use doesn't start. Both sides, as
    /// vanilla's: on the server for every player of the world, and on a
    /// server's client for the uses it predicts. Answer the same on both,
    /// and change the world only on the server (`native_window` is false
    /// there), not on a server's client.
    #[hook(
        side = "both",
        contract = "cancel",
        out = "canceled",
        neoforge = "LivingEntityUseItemEvent.Start",
        cancellable = true
    )]
    fn living_entity_use_item_start();
}
