use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// A brewing stand at `pos` brewed (`PotionBrewEvent.Post`, at the end of
    /// `BrewingStandBlockEntity.doBrew`), or `potion_brew_pre` was cancelled:
    /// `stacks` holds its five slots after the brew (or as the cancelling
    /// mods left them), and what the mods leave there is the stand's. Fires
    /// last, as `doBrew` ends: after `brewing_complete`, the brew sound and
    /// the ingredient's remainder dropping. Server-side.
    pub(in crate::game) fn native_potion_brew_post(
        &mut self,
        pos: [i32; 3],
        stacks: &mut [Option<ItemStack>; 5],
    ) {
        self.fire_potion_brew_post(pos, stacks);
    }
}
