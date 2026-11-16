//! The `living_conversion_pre` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A mob is about to convert into another type (NeoForge's
    /// `LivingConversionEvent.Pre`, vanilla's `Mob.convertTo`): a zombie
    /// drowning, a piglin or hoglin in the Overworld, a tadpole growing, a
    /// mooshroom sheared by a dispenser, a pig or villager struck by
    /// lightning. `outcome` is the new type. Set `cancel` to keep the mob as
    /// it is; a drowning zombie starts its timers again. Not told: a
    /// mooshroom a player shears, a zombie villager's cure, a villager turned
    /// by a zombie.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "LivingConversionEvent.Pre",
        cancellable = true
    )]
    fn living_conversion_pre();
}
