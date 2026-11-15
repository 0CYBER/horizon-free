use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// `IItemExtension.canPerformAction`: can `stack` perform the item ability
    /// `ability` (`shield_block`, `shears_harvest`...)?
    /// `can_perform` arrives with vanilla's answer; the parts of the stack's
    /// item may set it. Both sides.
    #[inline(never)]
    pub(in crate::game) fn native_can_perform_action(
        &self,
        stack: &ItemStack,
        ability: &str,
        can_perform: &mut bool,
    ) {
        self.fire_can_perform_action(stack, ability, can_perform);
    }
}
