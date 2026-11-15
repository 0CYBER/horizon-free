//! Pick Block into the acting player's inventory, asking
//! `ItemPart::is_not_replaceable_by_pick_action` in `Inventory.getSuitableHotbarSlot`
//! (`horizon_client::pick_item`).
use super::*;

impl Game<'_> {
    /// `ServerGamePacketListenerImpl.tryPickItem` for the acting player (the host's own, or a
    /// guest swapped in): picks `stack` into the 36 main slots, `infinite` with infinite
    /// materials (creative). Returns whether anything changed.
    pub(in crate::game) fn pick_into_inventory(
        &mut self,
        stack: &ItemStack,
        infinite: bool,
    ) -> bool {
        // With no such part, no player id is made and each hotbar stack costs nothing more.
        let player =
            mod_api::__private::has_part_hook(mod_api::HookId::IsNotReplaceableByPickAction)
                .then(|| self.native_acting_id());
        let picked = horizon_client::pick_item::plan(
            &self.inventory[..36],
            self.selected,
            stack,
            infinite,
            &mut |held, slot| self.pick_keeps(player, held, slot),
        );
        let Some(picked) = picked else {
            return false;
        };
        horizon_client::pick_item::apply(
            &mut self.inventory[..36],
            &mut self.selected,
            stack,
            picked,
        );
        true
    }
    /// `ItemStack.isNotReplaceableByPickAction` for `held` in hotbar `slot`: vanilla's answer
    /// (enchanted), then the parts of its item, asked for `player` when parts are listening.
    fn pick_keeps(&self, player: Option<u128>, held: &ItemStack, slot: usize) -> bool {
        let mut not_replaceable = horizon_client::pick_item::vanilla_not_replaceable(held);
        let listens = |item: &str| {
            mod_api::__private::item_listens(mod_api::HookId::IsNotReplaceableByPickAction, item)
        };
        if let Some(player) = player
            && listens(&held.item)
        {
            self.native_is_not_replaceable_by_pick_action(
                player,
                held,
                slot as u32,
                &mut not_replaceable,
            );
        }
        not_replaceable
    }
}
