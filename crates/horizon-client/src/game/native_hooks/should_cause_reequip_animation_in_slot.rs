use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// `IItemExtension.shouldCauseReequipAnimation` with the equipment slot
    /// named: does `hand` lower and rise again to show `stack` in place of
    /// `old`? `slot` is `mainhand` or `offhand`; `reequip` arrives with the
    /// answer of `native_should_cause_reequip_animation`. Client-side.
    #[inline(never)]
    pub(in crate::game) fn native_should_cause_reequip_animation_in_slot(
        &self,
        stack: &ItemStack,
        old: Option<&ItemStack>,
        hand: Hand,
        slot: &str,
        slot_changed: bool,
        reequip: &mut bool,
    ) {
        self.fire_should_cause_reequip_animation_in_slot(
            stack,
            old,
            hand,
            slot,
            slot_changed,
            reequip,
        );
    }
}
