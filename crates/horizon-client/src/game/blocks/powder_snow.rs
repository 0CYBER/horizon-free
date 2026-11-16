//! Powder snow around the player: its snowflakes, fire put out by it, and the
//! bucket that picks it up (`BucketItem.use` on a `BucketPickup` block). The
//! sinking, slowing, climbing and freezing are `PlayerMotion`'s
//! (`crate::motion`); placing it is the powder snow bucket's `BlockItem.place`.
use super::Game;
use horizon_client::{Result, cauldron, items::ItemStack, mobs::Who};
use horizon_core::{position::BlockPos, random::RandomSource};

/// The two ordered float draws of `Entity.playEntityOnFireExtinguishedSound`.
pub(super) fn extinguish_pitch(random: &mut impl RandomSource) -> f32 {
    1.6 + (random.next_float() - random.next_float()) * 0.4
}

impl Game<'_> {
    /// Entity.playEntityOnFireExtinguishedSound
    pub(super) fn extinguish_sound(&mut self, at: [f64; 3]) {
        let pitch = extinguish_pitch(self.particles.random());
        self.audio.play(
            "minecraft:entity.generic.extinguish_fire",
            Some(at),
            0.7,
            pitch,
        );
    }
    /// `Entity.applyEffectsFromBlocks`: a burning player in the rain
    /// (`isInRain`) has the fire put out (`clearFire`), with the extinguish
    /// sound that everyone near hears.
    pub(super) fn rain_extinguish_player(&mut self) {
        let Some(player) = self.player.as_ref() else {
            return;
        };
        let (at, top) = (player.position, player.bounds().max_position().y);
        if player.fire_ticks <= 0 || !self.in_rain(at, top) {
            return;
        }
        if let Some(player) = self.player.as_mut() {
            player.fire_ticks = 0;
        }
        self.extinguish_sound([at.x, at.y, at.z]);
    }
    /// `PowderSnowBlock.entityInside` (runBefore EXTINGUISH) for a burning
    /// player: `Level.destroyBlock(pos, false)`, which needs `mayInteract`.
    /// A client of a remote server leaves it to the server.
    pub(super) fn powder_snow_melt(&mut self, pos: BlockPos) {
        let at = [pos.x, pos.y, pos.z];
        if self.remote.is_some() || !self.is_powder_snow(at) || !self.may_interact_at(pos) {
            return;
        }
        self.with_level(|b, level, ticks| b.destroy_block(level, ticks, pos, false));
        let _ = self.process_level_events();
        self.world.modified.insert((at[0] >> 4, at[2] >> 4));
    }
    /// Whether the block at `pos` is powder snow.
    pub(super) fn is_powder_snow(&self, pos: [i32; 3]) -> bool {
        &*self
            .res
            .facts()
            .get(self.world.block(pos[0], pos[1], pos[2]))
            .name
            == "minecraft:powder_snow"
    }
    /// `BucketItem.use` with an empty bucket on powder snow:
    /// `PowderSnowBlock.pickupBlock` (air, level event 2001), the used
    /// statistic, the fill sound, FLUID_PICKUP, `ItemUtils.createFilledResult`
    /// and the filled_bucket trigger.
    pub(super) fn pick_up_powder_snow(&mut self, pos: [i32; 3]) -> Result<()> {
        let at = BlockPos::new(pos[0], pos[1], pos[2]);
        let creative = self.creative;
        let picked = self.with_level(|b, level, ticks| b.bucket_pickup(level, ticks, at, creative));
        self.process_level_events()?;
        let Some(filled) = picked else {
            return Ok(());
        };
        self.world.modified.insert((pos[0] >> 4, pos[2] >> 4));
        self.award_stat("used", "minecraft:bucket", 1);
        if let Some(eye) = self
            .player
            .as_ref()
            .map(|p| [p.position.x, p.position.y, p.position.z])
        {
            self.audio.play(
                "minecraft:item.bucket.fill_powder_snow",
                Some(eye),
                1.0,
                1.0,
            );
        }
        self.post_block_game_event("minecraft:fluid_pickup", pos, Some(Who::Player), None);
        let filled = ItemStack::new(filled, 1);
        self.filled_result(filled.clone());
        self.trigger_item("minecraft:filled_bucket", &filled);
        self.swing_arm();
        Ok(())
    }
    /// `ItemUtils.createFilledResult(held, player, filled)` for the held stack.
    pub(super) fn filled_result(&mut self, filled: ItemStack) {
        let slot = self.held_slot();
        let mut held = HeldSlot {
            game: self,
            slot,
            dropper: None,
        };
        cauldron::create_filled_result(&mut held, filled, true);
    }
}

/// `Inventory.add`'s answer once `give`, its passes until one adds nothing, left `rest` of
/// `stack`: true when all of it went in. Otherwise the last pass added nothing, so it is false
/// with the rest left in `stack` for the caller to drop, except in creative, where the rest is
/// cleared and it is true.
fn added(stack: &mut ItemStack, rest: Option<ItemStack>, creative: bool) -> bool {
    stack.count = rest.map_or(0, |rest| rest.count);
    if stack.count > 0 && creative {
        stack.count = 0;
    }
    stack.count == 0
}

/// The player acting, holding the stack in `slot` (`stack_in_mut`'s numbers), as
/// `ItemUtils.createFilledResult` and the cauldron see them.
pub(super) struct HeldSlot<'g, 'a> {
    pub(super) game: &'g mut Game<'a>,
    pub(super) slot: usize,
    /// The UUID of the player a mod's write acts as: what doesn't fit waits for the root
    /// commit of an open transaction. None for the game's own use.
    pub(super) dropper: Option<u128>,
}
impl cauldron::Player for HeldSlot<'_, '_> {
    fn creative(&self) -> bool {
        self.game.creative
    }
    fn hand(&mut self) -> &mut Option<ItemStack> {
        self.game.stack_in_mut(self.slot)
    }
    fn add(&mut self, stack: &mut ItemStack) -> bool {
        if stack.count == 0 {
            return false;
        }
        let rest = self.game.add_stack_to_inventory(stack.clone());
        added(stack, rest, self.game.creative)
    }
    fn contains(&self, stack: &ItemStack) -> bool {
        self.game
            .inventory
            .iter()
            .flatten()
            .any(|have| have.same_item_same_components(stack))
    }
    fn drop_stack(&mut self, stack: ItemStack) {
        match self.dropper {
            Some(uuid) => self.game.drop_after_commit(uuid, stack),
            None => {
                self.game.drop_plain(stack);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use horizon_client::items::ItemStack;

    /// An inventory with room for 3 more arrows: one stack of 61 and stone everywhere else.
    fn room_for_three() -> Vec<Option<ItemStack>> {
        let mut inventory = vec![Some(ItemStack::new("minecraft:stone", 64)); 36];
        inventory[0] = Some(ItemStack::new("minecraft:arrow", 61));
        inventory
    }

    #[test]
    fn a_partial_add_leaves_the_rest_to_drop_in_survival_and_clears_it_in_creative() {
        for creative in [false, true] {
            let (mut inventory, mut offhand) = (room_for_three(), None);
            let mut arrows = ItemStack::new("minecraft:arrow", 5);
            let left =
                super::super::inventory_add(&mut inventory, &mut offhand, 0, 64, &mut arrows);
            let rest = left.then(|| arrows.clone());
            assert_eq!(inventory[0].as_ref().map(|s| s.count), Some(64));
            let added = super::added(&mut arrows, rest, creative);
            // Survival: false, with the 2 that found no slot left for `createFilledResult` to
            // drop. Creative: they are cleared, and it is true.
            assert_eq!(
                (added, arrows.count),
                if creative { (true, 0) } else { (false, 2) }
            );
        }
        let mut arrows = ItemStack::new("minecraft:arrow", 2);
        assert!(super::added(&mut arrows, None, false));
        assert_eq!(arrows.count, 0);
    }
}
