//! `ItemEntity.hurtServer`'s hit for the damage an item's own tick deals it:
//! the health off, the contents of an item destroyed by it spilled, and the
//! `entity_damage` game event every accepted hit posts before that.
use super::{BlockFacts, Fluid, ItemEntity, World};
use crate::items::ItemStack;

/// What a run of hits spilled, and how many `entity_damage` events they post.
#[derive(Default)]
pub struct ItemHits {
    pub spilled: Vec<ItemStack>,
    pub events: u32,
}

impl ItemHits {
    /// One `hurtServer`: `amount` off the item's health, and the stacks it
    /// leaves when that empties it (`ItemStack.onDestroyed`). A destroyed item
    /// takes no more hits: `Entity.checkInsideBlocks` stops at
    /// `!Entity.isAlive()` and `InsideBlockEffectApplier.applyAndClear` skips
    /// the effects still queued.
    pub fn hit(&mut self, item: &mut ItemEntity, amount: i32) {
        if item.health <= 0 {
            return;
        }
        self.spilled.extend(item.hurt(amount).unwrap_or_default());
        self.events += 1;
    }
}

impl ItemEntity {
    /// `Entity.baseTick`'s fire: a fire-immune item is put out, and a burning
    /// one is hurt every 20 ticks unless `isInLava`, which is the fluid it
    /// started the tick in (`in_lava`): it runs before `ItemEntity.tick`'s
    /// move, so its `entity_damage` is posted where the tick started.
    pub fn burning_tick(
        &mut self,
        in_lava: bool,
        can_be_hurt_by: &dyn Fn(&ItemStack, &str) -> bool,
    ) -> ItemHits {
        self.burning_tick_with_hook(in_lava, can_be_hurt_by, |_| false)
    }
    /// As [`Self::burning_tick`], allowing the caller to decide the actual
    /// `minecraft:on_fire` damage attempt (a mod's invulnerability).
    pub fn burning_tick_with_hook(
        &mut self,
        in_lava: bool,
        can_be_hurt_by: &dyn Fn(&ItemStack, &str) -> bool,
        mut invulnerable: impl FnMut(&str) -> bool,
    ) -> ItemHits {
        let mut hits = ItemHits::default();
        if !can_be_hurt_by(&self.stack, "minecraft:in_fire") {
            self.fire_ticks = 0;
        }
        if self.fire_ticks > 0 {
            if self.fire_ticks % 20 == 0
                && !in_lava
                && can_be_hurt_by(&self.stack, "minecraft:on_fire")
                && !invulnerable("minecraft:on_fire")
            {
                hits.hit(self, 1);
            }
            self.fire_ticks -= 1;
        }
        hits
    }
    /// Whether the item's feet are in lava, for [`Self::burning_tick`].
    pub fn in_lava(&self, world: &World, facts: &BlockFacts) -> bool {
        self.fluid_at_feet(world, facts) == Fluid::Lava
    }
    pub(super) fn fluid_at_feet(&self, world: &World, facts: &BlockFacts) -> Fluid {
        let at = world.block(
            self.position.x.floor() as i32,
            (self.position.y + 0.1).floor() as i32,
            self.position.z.floor() as i32,
        );
        facts.get(at).fluid
    }
}
