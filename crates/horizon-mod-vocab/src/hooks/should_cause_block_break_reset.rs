//! The `should_cause_block_break_reset` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// NeoForge's `IItemExtension.shouldCauseBlockBreakReset`: does a break
    /// restart because the main hand changed? `stack` is the stack the break
    /// began with (vanilla's `destroyingItem`), `held` the main hand now
    /// (`None` for an empty hand). `reset` arrives with vanilla's answer: not
    /// `ItemStack.isSameItemSameComponents(held, stack)` (count and damage
    /// left out). `false` keeps the break and its progress going; `true`
    /// aborts it and starts it again from 0. A `true` for an unchanged stack
    /// restarts the break every tick, so it never finishes. Asked on each
    /// `continueDestroyBlock` and `startDestroyBlock` of the block the break
    /// targets, on the local player's client (single player and a server's
    /// client alike). Only the `ItemPart`s attached to `stack`'s item are
    /// asked. Client-side.
    #[hook(
        side = "client",
        contract = "listen",
        scope = "item",
        key = "stack",
        neoforge = "IItemExtension.shouldCauseBlockBreakReset"
    )]
    fn should_cause_block_break_reset();
}
