//! The `consume_effect` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0086

pub trait Hooks {
    /// A consumable's effect of a mod's own kind: the item's `on_consume_effects`
    /// entry is of a type in a namespace of its own, and the loader kept it
    /// for this call. The kind and the definition payload as a borrowed `Tag` are here; the acting
    /// player is the one consuming, and the game's own effects,
    /// sounds, teleports and drops are open. An unclaimed kind does
    /// nothing. This is the runtime `ConsumeEffect.apply` moment, not effect
    /// type registration through NeoForge's `RegisterEvent`.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (runtime ConsumeEffect.apply; RegisterEvent registers effect types, not invocations)"
    )]
    fn consume_effect();
}
