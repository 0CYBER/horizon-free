//! A cart's `applyEffectsFromBlocks`: bubble columns, powder snow, cobwebs
//! and cactus (`CactusBlock.entityInside`), whose hits are taken once the
//! ticked cart is stored.
use super::*;

impl Game<'_> {
    /// `applyEffectsFromBlocks` for a cart: a bubble column lifts or drags it
    /// and resets the fall distance.
    pub(super) fn cart_block_effects(&self, c: &mut Minecart) {
        if c.push_by_bubble_column(
            &self.world,
            self.pack.assets.registry,
            self.res.facts(),
            self.collision,
        ) {
            c.fall_distance = 0.0;
        }
        // PowderSnowBlock's FREEZE contact is counted here too, and each
        // cactus the box is in (`CactusBlock.entityInside`, hurt on the server).
        // WebBlock.entityInside: makeStuckInBlock(0.25, 0.05, 0.25) for every
        // cobweb the box (deflated by 1.0E-5) is in; it resets the fall distance.
        let b = c.bounds().deflate(Vec3::new(1e-5, 1e-5, 1e-5));
        let (lo, hi) = (b.min_position(), b.max_position());
        let facts = self.res.facts();
        let mut froze = false;
        for x in lo.x.floor() as i32..=hi.x.floor() as i32 {
            for y in lo.y.floor() as i32..=hi.y.floor() as i32 {
                for z in lo.z.floor() as i32..=hi.z.floor() as i32 {
                    let name = &*facts.get(self.world.block(x, y, z)).name;
                    if name == "minecraft:powder_snow" {
                        froze = true;
                    }
                    if name == "minecraft:cactus" && self.remote.is_none() {
                        c.cactus_hits += 1;
                    }
                    if name == "minecraft:cobweb" {
                        c.fall_distance = 0.0;
                        c.stuck = Vec3::new(0.25, f64::from(0.05_f32), 0.25);
                    }
                }
            }
        }
        if froze {
            // InsideBlockEffectType.FREEZE: once per applyEffectsFromBlocks.
            c.ticks_frozen = (c.ticks_frozen + 1)
                .min(horizon_client::motion::PlayerMotion::TICKS_REQUIRED_TO_FREEZE);
        }
    }
    /// The cactus hits cart `id` (at `slot`) met in this tick's
    /// `applyEffectsFromBlocks`: `VehicleEntity.hurtServer` for 1 each, which
    /// an `Invulnerable` cart ignores. A hit that breaks it ends them.
    pub(super) fn cart_cactus_tick(&mut self, id: u32, slot: usize) {
        let Some(i) = cart_at(&self.carts, id, slot) else {
            return;
        };
        let hits = std::mem::take(&mut self.carts[i].cactus_hits);
        let invulnerable = self
            .persist
            .raw
            .get(&id)
            .is_some_and(|tag| horizon_client::entity_nbt::flag(tag, "Invulnerable"));
        for _ in 0..if invulnerable { 0 } else { hits } {
            let Some(i) = cart_at(&self.carts, id, slot) else {
                return;
            };
            self.hurt_cart_result(i, 1.0, false, false, CartHit::default());
        }
    }
}
