//! The `should_cause_reequip_animation_in_slot` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The same decision as `should_cause_reequip_animation`, asked right
    /// after it with the equipment slot named. `slot` is `mainhand` or
    /// `offhand`, the vanilla hand the stack is shown in. `stack`, `old`,
    /// `hand` and `slot_changed` are as in `should_cause_reequip_animation`,
    /// and `reequip` arrives with the answer that hook left (and the item
    /// parts', if any), so a mod that only cares about the slot can leave it
    /// alone. `true` lowers the hand
    /// and raises it with `stack`; `false` shows `stack` at once. Every
    /// mod that listens is asked, whatever the item. The original hook and its
    /// fields are unchanged. Client-side.
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "IItemExtension.shouldCauseReequipAnimation",
        target = "horizon_client::game::native_hooks::should_cause_reequip_animation_in_slot::Game<'_>::native_should_cause_reequip_animation_in_slot"
    )]
    fn should_cause_reequip_animation_in_slot();
}
