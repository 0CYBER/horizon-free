use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// A player took a crafting grid's result (`ResultSlot.onTake`, NeoForge's
    /// `PlayerEvent.ItemCraftedEvent`): `player` is the player's uuid, `recipe`
    /// the recipe the grid matched (`minecraft:oak_planks`) and `stack` the
    /// result it took, with its components. `inputs` is the grid row by row
    /// as it was before this take used it (4 cells for an inventory's grid,
    /// 9 for a crafting table's; `None` for an empty cell): a shift-click
    /// that crafts several times is heard once per craft, each with the grid
    /// before that craft. It only hears: the take has
    /// already happened. It fires for the crafting grid of an inventory or a
    /// crafting table, on the integrated server and a dedicated server, guests
    /// included, and not for a stonecutter's, a smithing table's or a
    /// furnace's result. Server-side.
    pub(in crate::game) fn native_item_crafted(
        &mut self,
        player: u128,
        recipe: &str,
        stack: &ItemStack,
        inputs: &[Option<ItemStack>],
    ) {
        self.fire_item_crafted(player, recipe, stack, inputs);
    }
}
