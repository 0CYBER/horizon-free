use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// `IItemExtension.shouldCauseBlockBreakReset`: does the break that began
    /// with `stack` restart now that the main hand holds `held`? `reset`
    /// arrives with vanilla's answer; the parts of `stack`'s item may set it.
    /// Client-side.
    #[inline(never)]
    pub(in crate::game) fn native_should_cause_block_break_reset(
        &self,
        stack: &ItemStack,
        held: Option<&ItemStack>,
        reset: &mut bool,
    ) {
        self.fire_should_cause_block_break_reset(stack, held, reset);
    }
}
