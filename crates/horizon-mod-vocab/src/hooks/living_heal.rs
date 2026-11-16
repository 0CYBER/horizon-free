//! The `living_heal` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0347

pub trait Hooks {
    /// A living entity is about to heal. Mods may change the amount or cancel.
    /// `entity` identifies the affected entity. It is given the loop `context` of the side that ticks the entity (its registries, settings and mod list), because it fires inside the tick's own health update, where a deferred call would reorder vanilla's effects.
    #[hook(
        side = "both",
        contract = "cancel",
        out = "cancel",
        context = "loop",
        neoforge = "LivingHealEvent",
        version = 2,
        target = "horizon_client_lib::hunger::native_living_heal"
    )]
    fn living_heal();
}
