use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// A brewing stand at `pos` is about to brew (`PotionBrewEvent.Pre`,
    /// in `BrewingStandBlockEntity.doBrew`): `stacks` is a copy of its five
    /// slots (three bottles, the ingredient, the fuel). Set `cancel` to skip
    /// vanilla's brew: the stand then takes `stacks` as the mods left them,
    /// uses no ingredient and plays no brew sound, and `potion_brew_post`
    /// fires with those slots; `brewing_complete` doesn't, as nothing was
    /// brewed. Uncancelled, changes to `stacks` are dropped. Server-side.
    pub(in crate::game) fn native_potion_brew_pre(
        &mut self,
        pos: [i32; 3],
        stacks: &mut [Option<ItemStack>; 5],
        cancel: &mut bool,
    ) {
        self.fire_potion_brew_pre(pos, stacks, cancel);
    }
}
