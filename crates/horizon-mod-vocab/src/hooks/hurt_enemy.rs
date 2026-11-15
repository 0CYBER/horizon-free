//! The `hurt_enemy` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0051

pub trait Hooks {
    /// A player's hit landed on the mob `entity` of type `kind`
    /// (`minecraft:zombie`), holding `stack`: where `Item.hurtEnemy` is called,
    /// before the post-attack enchantments and the weapon's wear. It only
    /// hears. Another player as the target is not told. Server-side, for
    /// every player of the world (the helpers read the acting one).
    #[hook(
        side = "server",
        contract = "listen",
        scope = "item",
        key = "stack",
        neoforge = "none (vanilla Item.hurtEnemy; NeoForge adds no event or extension for it); LivingDamageEvent.Post comes later"
    )]
    fn hurt_enemy();
}
