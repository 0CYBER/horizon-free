use super::*;

impl ItemEntity {
    /// `Entity.applyEffectsFromBlocks` after the move: water puts the item
    /// out, then lava, fire and cactus hurt it. Each hit posts `entity_damage`
    /// at the item's position, and the hit that destroys it ends the pass.
    /// The burning of `Entity.baseTick` is [`ItemEntity::burning_tick`]'s.
    pub fn environment_tick(
        &mut self,
        mut world: &World,
        facts: &BlockFacts,
        registry: &horizon_world::registry::BlockRegistry,
        can_be_hurt_by: &dyn Fn(&ItemStack, &str) -> bool,
    ) -> ItemEnvironmentTick {
        self.environment_tick_inner(&mut world, facts, registry, can_be_hurt_by, |_, _| false)
    }
    /// As `environment_tick`, allowing the caller to decide each actual damage attempt.
    pub fn environment_tick_with_hook(
        &mut self,
        mut world: &mut World,
        facts: &BlockFacts,
        registry: &horizon_world::registry::BlockRegistry,
        can_be_hurt_by: &dyn Fn(&ItemStack, &str) -> bool,
        mut invulnerable: impl FnMut(&mut World, &str) -> bool,
    ) -> ItemEnvironmentTick {
        self.environment_tick_inner(
            &mut world,
            facts,
            registry,
            can_be_hurt_by,
            |world, damage| invulnerable(world, damage),
        )
    }
    fn environment_tick_inner<W: std::ops::Deref<Target = World>>(
        &mut self,
        world: &mut W,
        facts: &BlockFacts,
        registry: &horizon_world::registry::BlockRegistry,
        can_be_hurt_by: &dyn Fn(&ItemStack, &str) -> bool,
        mut invulnerable: impl FnMut(&mut W, &str) -> bool,
    ) -> ItemEnvironmentTick {
        let fluid = self.fluid_at_feet(world, facts);
        let fire_immune = !can_be_hurt_by(&self.stack, "minecraft:in_fire");
        let mut hits = ItemHits::default();
        if fluid == Fluid::Water || fire_immune {
            self.fire_ticks = 0;
        }
        let mut lava_hurt = false;
        // LavaCauldronBlock.entityInside applies LAVA_IGNITE and then lavaHurt.
        // A cauldron is not a lava fluid, and removed items get no block effects.
        if self.health > 0 && !fire_immune {
            let contacts = usize::from(fluid == Fluid::Lava)
                + lava_cauldron_contacts(world, registry, self.bounds());
            if contacts > 0 {
                self.fire_ticks = self.fire_ticks.max(300);
            }
            // StepBasedCollector coalesces ignition but keeps each lavaHurt.
            for _ in 0..contacts {
                if self.health <= 0 {
                    break;
                }
                if can_be_hurt_by(&self.stack, "minecraft:lava")
                    && !invulnerable(world, "minecraft:lava")
                {
                    hits.hit(self, 4);
                    lava_hurt = true;
                }
            }
        }
        let fire_damage = crate::motion::environment::fire_contact(world, registry, self.bounds());
        if fire_damage > 0.0 && !fire_immune {
            self.fire_ticks = self.fire_ticks.max(160);
            if can_be_hurt_by(&self.stack, "minecraft:in_fire")
                && !invulnerable(world, "minecraft:in_fire")
            {
                hits.hit(self, fire_damage as i32);
            }
        }
        // CactusBlock.entityInside: `entity.hurt(cactus, 1.0)`, a point a tick
        // (an item has no hurt cooldown), unless `canBeHurtBy` skips it.
        if crate::motion::environment::cactus_contact(world, registry, self.bounds())
            && can_be_hurt_by(&self.stack, "minecraft:cactus")
            && !invulnerable(world, "minecraft:cactus")
        {
            hits.hit(self, 1);
        }
        ItemEnvironmentTick {
            alive: self.health > 0,
            lava_hurt,
            spilled: hits.spilled,
            damage_events: hits.events,
        }
    }
}
