use super::*;

impl Game<'_> {
    /// Per-stack ingredient and fuel remainder, initialized from item data.
    pub(in crate::game) fn native_crafting_remainder(
        &self,
        stack: &ItemStack,
        remainder: &mut Option<ItemStack>,
    ) {
        self.fire_crafting_remainder(stack, remainder);
    }
}
