use super::*;

impl Game<'_> {
    /// `IItemExtension.isPiglinCurrency`: is `stack` barter currency to a piglin?
    /// `currency` arrives with vanilla's answer (a gold ingot); the parts of the
    /// stack's item may set it. Both sides.
    #[inline(never)]
    pub(in crate::game) fn native_is_piglin_currency(
        &self,
        stack: &ItemStack,
        currency: &mut bool,
    ) {
        self.fire_is_piglin_currency(stack, currency);
    }
}
