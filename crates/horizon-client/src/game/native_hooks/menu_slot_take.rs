use super::*;

impl Game<'_> {
    /// `Slot.onTake`: a click took `taken` out of slot `slot` of the menu `player` has open
    /// (`menu` its type, `pos` the block it is of, none for an ender chest, a trade or a mod's
    /// own menu). `left` is what stayed in the slot. Only the menu's own slots, not the player's
    /// inventory; a shift-click that empties a slot is told once with the whole stack. Heard after
    /// the click, once for each slot it took from. No player state is swapped in. Server-side,
    /// for a guest's or a dedicated server player's click.
    pub(in crate::game) fn native_menu_slot_take(
        &mut self,
        player: u128,
        menu: &str,
        pos: Option<[i32; 3]>,
        slot: i32,
        taken: &ItemStack,
        left: Option<&ItemStack>,
    ) {
        self.fire_menu_slot_take(player, menu, pos, slot, taken, left);
    }
}
