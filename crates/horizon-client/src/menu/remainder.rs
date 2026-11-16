use super::*;

impl Env<'_> {
    pub(super) fn remainder(&self, stack: &ItemStack) -> Option<ItemStack> {
        self.crafting_remainder
            .unwrap_or(self.items)
            .crafting_remainder(stack)
    }
}
