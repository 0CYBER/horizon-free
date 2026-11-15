//! The `should_cause_reequip_animation` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// NeoForge's `IItemExtension.shouldCauseReequipAnimation`: does the
    /// first-person hand lower and raise again to show `stack`, the stack the
    /// hand holds now? `old` is the stack the hand shows (`None` for an empty
    /// hand), `slot_changed` whether `stack` came from another slot (always
    /// false for the off hand). `reequip` arrives with vanilla's answer
    /// (`!FirstPersonHandsAndItems.shouldInstantlyReplaceVisibleItem`: the
    /// same stack changed in place, or an equal one but for its damage, is
    /// shown at once). `true` lowers the hand and raises it with `stack`;
    /// `false` shows `stack` at once. Asked each client tick, for each hand,
    /// while the shown stack differs from the held one by slot or by value;
    /// an unchanged hand and an empty hand aren't asked. Only the `ItemPart`s
    /// attached to `stack`'s item are asked. Client-side.
    #[hook(
        side = "client",
        contract = "listen",
        scope = "item",
        key = "stack",
        neoforge = "IItemExtension.shouldCauseReequipAnimation"
    )]
    fn should_cause_reequip_animation();
}
