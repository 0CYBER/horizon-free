use super::*;

impl Game<'_> {
    /// `IItemExtension.applyEnchantments`: the enchanting table applies `enchantments` (the
    /// list it chose, as (id, level)) to `stack`, its input. `result` arrives with vanilla's
    /// result; the parts of the input's item may replace it. Server-side.
    #[inline(never)]
    pub(in crate::game) fn native_apply_enchantments(
        &self,
        stack: &ItemStack,
        enchantments: &[(std::sync::Arc<str>, u32)],
        result: &mut ItemStack,
    ) {
        self.fire_apply_enchantments(stack, enchantments, result);
    }
}
