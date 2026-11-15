//! The `mob_effect_added` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0064

pub trait Hooks {
    /// NeoForge's `MobEffectEvent.Added`, vanilla's `MobEffect.onEffectStarted`:
    /// every `LivingEntity.addEffect` of the content mod's effect `effect`,
    /// new or not, changed or not, after `on_effect_added`.
    /// Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "MobEffectEvent.Added"
    )]
    fn mob_effect_added();
}
