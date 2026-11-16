use super::*;

impl Game<'_> {
    /// `IItemExtension.makesPiglinsNeutral`: does `stack`, worn by `wearer`,
    /// count as gold armour to a piglin? `neutral` arrives with vanilla's answer
    /// (`#minecraft:piglin_safe_armor`); the parts of the stack's item may set it.
    /// Server-side.
    #[inline(never)]
    pub(in crate::game) fn native_makes_piglins_neutral(
        &self,
        stack: &ItemStack,
        wearer: LivingRef,
        neutral: &mut bool,
    ) {
        self.fire_makes_piglins_neutral(stack, wearer, neutral);
    }
}
