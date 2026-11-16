use super::*;

impl Game<'_> {
    /// The fluid a stack of a mod's own container item holds, as its
    /// `neoforge:fluid_container` item model draws it
    /// (`FluidUtil.getFirstStackContained`): `fluid` arrives `None` (holds
    /// none); set it to the fluid's key, taken once at registration with
    /// `r.fluid_key`. Asked on the client each time a stack of an item outside
    /// `minecraft:` is drawn through a fluid container model; vanilla's items
    /// keep their own answer and never ask. A fluid with no look in an item
    /// draws the missing texture; a key no fluid is registered under draws the
    /// stack as holding none, and the game logs it once per item.
    /// Answer from the stack alone, and fast.
    pub(in crate::game) fn native_contained_fluid(
        &self,
        stack: &mod_api::ItemStack,
        fluid: &mut Option<mod_api::FluidKey>,
    ) {
        self.fire_contained_fluid(stack, fluid);
    }
}
