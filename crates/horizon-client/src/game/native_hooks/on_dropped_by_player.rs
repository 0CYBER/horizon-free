use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// `IItemExtension.onDroppedByPlayer`: `player` pressed the drop key on
    /// `stack` (`all`: the whole stack, else one item), before anything
    /// leaves the slot. Set `canceled` and the stack stays where it is.
    pub(in crate::game) fn native_on_dropped_by_player(
        &mut self,
        player: PlayerRef,
        stack: &ItemStack,
        all: bool,
        canceled: &mut bool,
    ) {
        self.fire_on_dropped_by_player(player, stack, all, canceled);
    }
}

impl Game<'_> {
    /// `ItemStack.onDroppedByPlayer` for the acting player's drop key: true
    /// when the selected stack's Part refused the drop. Nothing runs for an
    /// empty slot or an item without the Part.
    pub(in crate::game) fn native_dropped_by_player_event(&mut self, all: bool) -> bool {
        if !mod_api::__private::has_part_hook(mod_api::HookId::OnDroppedByPlayer) {
            return false;
        }
        let Some(stack) = self.inventory[self.selected]
            .as_ref()
            .filter(|s| s.count > 0)
        else {
            return false;
        };
        if !mod_api::__private::item_listens(mod_api::HookId::OnDroppedByPlayer, &stack.item) {
            return false;
        }
        let stack = stack.clone();
        let Some(player) = self.native_acting_player() else {
            return false;
        };
        let mut canceled = false;
        self.native_on_dropped_by_player(player, &stack, all, &mut canceled);
        canceled
    }
}
