//! `/give`'s handing over of one stack.
use super::{Game, Target};
use horizon_client::items::ItemStack;

impl Game<'_> {
    /// `GiveCommand.giveItem`'s handing over of one stack to the acting player:
    /// `Inventory.add`, then what did not fit thrown from the eyes with no pickup delay and
    /// `owner` (the player's UUID) its only taker, or the fake item `makeFakeItem` flashes
    /// and the pickup sound when it all went in.
    pub(in crate::game) fn give_acting_stack(&mut self, stack: ItemStack, owner: Option<[i32; 4]>) {
        let uuid = self.native_acting_id();
        self.journal_player(uuid);
        // `GiveCommand.giveItem` flashes `prototypeItemStack.copy()`: one item.
        let mut flash = stack.clone();
        flash.count = 1;
        let left = self.add_stack_to_inventory(stack);
        // At the player who gets them.
        if let Some(rest) = left {
            let uuid = self.native_acting_id();
            self.after_commit(crate::game::transactions::After::GiveRest(
                uuid, rest, owner,
            ));
        } else {
            let uuid = self.native_acting_id();
            self.after_commit(crate::game::transactions::After::GivePickup(uuid, flash));
        }
    }
    pub(in crate::game) fn give_pickup(&mut self, stack: ItemStack) {
        let p = self
            .target_position(Target::Player)
            .unwrap_or(self.source_position());
        // ItemEntity.makeFakeItem: the item only flashes out of the player's
        // head, and the pickup sound plays.
        let ItemStack {
            item,
            damage,
            extra,
            ..
        } = stack;
        self.give_flash(&item, damage, extra, p);
    }
    pub(in crate::game) fn give_remainder(&mut self, rest: ItemStack, owner: Option<[i32; 4]>) {
        let p = self
            .target_position(Target::Player)
            .unwrap_or(self.source_position());
        // Inventory full: Player.createItemStackToDrop (thrown from
        // the eyes), then setNoPickUpDelay and setTarget(player).
        let eye = self.eye_position().unwrap_or([
            p[0],
            p[1] + self.target_eye_height(Target::Player),
            p[2],
        ]);
        if let Some(id) = self.throw_from_eyes(rest, eye, false, false)
            && let Some(item) = self.dropped_item_mut(id)
        {
            item.thrower = None;
            item.pickup_delay = 0;
            item.target = owner;
        }
    }
}
