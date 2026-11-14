//! `/give`'s handing over of one stack.
use super::{Game, Target};
use horizon_client::items::ItemStack;
use horizon_core::random::RandomSource;

impl Game<'_> {
    /// `GiveCommand.giveItem`'s handing over of one stack to the acting player:
    /// `Inventory.add`, then what did not fit thrown from the eyes with no pickup delay and
    /// `owner` (the player's UUID) its only taker, or the pickup sound when it all went in.
    pub(in crate::game) fn give_acting_stack(&mut self, stack: ItemStack, owner: Option<[i32; 4]>) {
        let uuid = self.native_acting_id();
        self.journal_player(uuid);
        let left = self.add_stack_to_inventory(stack);
        // At the player who gets them.
        if let Some(rest) = left {
            let uuid = self.native_acting_id();
            self.after_commit(crate::game::transactions::After::GiveRest(
                uuid, rest, owner,
            ));
        } else {
            let uuid = self.native_acting_id();
            self.after_commit(crate::game::transactions::After::GivePickup(uuid));
        }
    }
    pub(in crate::game) fn give_pickup(&mut self) {
        let p = self
            .target_position(Target::Player)
            .unwrap_or(self.source_position());
        let r = &mut self.world.random;
        let pitch = ((r.next_float() - r.next_float()) * 0.7 + 1.0) * 2.0;
        self.audio
            .play("minecraft:entity.item.pickup", Some(p), 0.2, pitch);
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
        self.throw_from_eyes(rest, eye, false, false);
        if let Some(item) = self.just_spawned_item() {
            item.thrower = None;
            item.pickup_delay = 0;
            item.target = owner;
        }
    }
}
