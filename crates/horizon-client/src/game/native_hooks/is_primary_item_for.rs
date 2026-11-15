use super::*;

impl Game<'_> {
    /// `IItemExtension.isPrimaryItemFor`: is `stack`'s item primary for `enchantment`, so the
    /// enchanting table, loot `enchant_with_levels`, trades and spawn gear may offer it?
    /// `primary` arrives with vanilla's `Enchantment.isPrimaryItem`; the parts of the stack's
    /// item may set it. Asked once per enchantment when parts install, with the item's default
    /// stack (`resolve_primary_items`). Server-side.
    #[inline(never)]
    pub(in crate::game) fn native_is_primary_item_for(
        &self,
        stack: &ItemStack,
        enchantment: &str,
        primary: &mut bool,
    ) {
        self.fire_is_primary_item_for(stack, enchantment, primary);
    }
}
