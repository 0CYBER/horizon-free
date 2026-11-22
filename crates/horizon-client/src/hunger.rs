//! Hunger and regeneration over the resident player's actual health, damage,
//! attribute and exhaustion storage. Starvation borrows the same FoodData that
//! is mid-tick, preserving damage callbacks and the following timer reset.
use crate::{
    Result,
    health::{
        Attributes, DamageLevel, Environment, HealthEvent, HealthResources, HealthRules, State,
        Updates,
    },
    motion::{MotionEvent, PlayerMotion},
};
use horizon_world::native_load_context::LoopContext;
use horizon_world::{
    food::{Difficulty, FoodData, FoodWorld},
    player_health::{PlayerAttributeHealth, PlayerFloatIds},
    server_player_combat::ServerPlayerCombat,
    synced_floats::SyncedFloats,
};

/// A living entity is about to heal. Mods may change the amount or cancel.
/// `entity` identifies the affected entity.
#[inline(never)]
pub fn native_living_heal(context: &LoopContext, entity: u32, amount: &mut f32, cancel: &mut bool) {
    crate::native_client_hooks::living_heal(context, entity, amount, cancel);
}

impl PlayerMotion {
    /// Player.causeFoodExhaustion: creative and spectator abilities suppress
    /// exhaustion from movement, attacks and effects.
    pub fn cause_food_exhaustion(&mut self, amount: f32) {
        if !self.invulnerable && !self.spectator {
            self.health.food.add_exhaustion(amount);
        }
    }

    /// ServerPlayer.tickRegeneration, dispatched from Player.aiStep before
    /// inventory and living movement. The separately invoked food tick follows
    /// movement in ServerPlayer.doTick.
    pub fn peaceful_regeneration(
        &mut self,
        context: &LoopContext,
        rules: HealthRules,
    ) -> Result<()> {
        if rules.difficulty != Difficulty::Peaceful || !rules.natural_regeneration {
            return Ok(());
        }
        if self.health.tick_count % 20 == 0 {
            let health = self.health.health();
            let maximum = self.attribute("minecraft:max_health")? as f32;
            if health < maximum {
                let mut amount = 1.0;
                let mut cancel = false;
                native_living_heal(context, self.entity_id as u32, &mut amount, &mut cancel);
                if !cancel {
                    let h = &mut self.health;
                    let mut observer = Updates(&mut self.events);
                    PlayerAttributeHealth::new(
                        &mut self.attributes,
                        &mut h.data,
                        h.ids,
                        h.dead,
                        &mut observer,
                    )?
                    .heal(amount)?;
                }
            }
            let saturation = self.health.food.saturation;
            if saturation < 20.0 {
                self.health.food.saturation = saturation + 1.0;
            }
        }
        if self.health.tick_count % 10 == 0 && self.health.food.needs_food() {
            self.health.food.food = self.health.food.food.wrapping_add(1);
        }
        Ok(())
    }
    /// `FoodData.tick` in `level`, where starvation hurts.
    pub fn food_tick(
        &mut self,
        context: &LoopContext,
        level: &mut DamageLevel<'_>,
        resources: &HealthResources,
        rules: HealthRules,
    ) -> Result<()> {
        if self.client_side {
            return Ok(());
        }
        let victim = self.victim();
        let h = &mut self.health;
        let mut world = TickWorld {
            context,
            level: level.reborrow(),
            entity: self.entity_id,
            victim,
            resources,
            rules,
            invulnerable: self.invulnerable || self.spectator,
            client_loaded: self.client_loaded,
            attributes: &mut self.attributes,
            data: &mut h.data,
            ids: h.ids,
            hurt: &mut h.hurt,
            hurt_animation_time: &mut h.hurt_animation_time,
            dead: &mut h.dead,
            sync_velocity: &mut h.sync_velocity,
            game_time: h.game_time,
            events: &mut self.events,
            status: self.status.clone(),
            random: &mut self.random,
        };
        h.food.tick(&mut world)
    }
}
struct TickWorld<'a> {
    context: &'a LoopContext,
    level: DamageLevel<'a>,
    entity: i32,
    victim: crate::health::Victim,
    resources: &'a HealthResources,
    rules: HealthRules,
    invulnerable: bool,
    client_loaded: bool,
    attributes: &'a mut Attributes,
    data: &'a mut SyncedFloats,
    ids: PlayerFloatIds,
    hurt: &'a mut State,
    hurt_animation_time: &'a mut i32,
    dead: &'a mut bool,
    sync_velocity: &'a mut bool,
    game_time: i64,
    events: &'a mut Vec<MotionEvent>,
    /// Resistance, fire resistance and armor protection (effects.rs): the
    /// starvation hurt is a `LivingEntity.hurtServer` like any other, so its
    /// `onMobHurt` (Infested) and magic absorb see them.
    status: crate::effects::StatusFlags,
    random: &'a mut horizon_core::random::EntityRandom,
}
impl FoodWorld for TickWorld<'_> {
    type Error = Box<dyn std::error::Error>;
    type Level = ();
    fn level(&mut self, _: &mut FoodData) -> Result<()> {
        Ok(())
    }
    fn difficulty(&mut self, _: (), _: &mut FoodData) -> Result<Difficulty> {
        Ok(self.rules.difficulty)
    }
    fn natural_regeneration(&mut self, _: (), _: &mut FoodData) -> Result<bool> {
        Ok(self.rules.natural_regeneration)
    }
    fn is_hurt(&mut self, _: &mut FoodData) -> Result<bool> {
        let health = self
            .data
            .get(self.ids.health)
            .expect("validated health field");
        if health <= 0.0 || health.is_nan() {
            return Ok(false);
        }
        let max_health = self
            .attributes
            .value_of(
                "minecraft:max_health",
                horizon_world::attribute_map::ValueKind::Computed,
            )
            .map_err(|e| std::io::Error::other(format!("player health attribute lookup: {e:?}")))?
            as f32;
        Ok(health < max_health)
    }
    fn heal(&mut self, _: &mut FoodData, mut amount: f32) -> Result<()> {
        let mut cancel = false;
        native_living_heal(self.context, self.entity as u32, &mut amount, &mut cancel);
        if cancel {
            return Ok(());
        }
        let mut observer = Updates(self.events);
        Ok(PlayerAttributeHealth::new(
            self.attributes,
            self.data,
            self.ids,
            *self.dead,
            &mut observer,
        )?
        .heal(amount)?)
    }
    fn health(&mut self, _: &mut FoodData) -> Result<f32> {
        Ok(self
            .data
            .get(self.ids.health)
            .expect("validated health field"))
    }
    fn hurt_starvation(&mut self, _: (), food: &mut FoodData, amount: f32) -> Result<()> {
        let id = self
            .resources
            .damage
            .id("minecraft:starve")
            .expect("validated starvation source");
        let mut environment = Environment {
            level: self.level.reborrow(),
            invulnerability_checked: None,
            can_harm_player: true,
            entity: self.entity,
            victim: self.victim,
            causing: 0,
            invulnerable: self.invulnerable,
            permanently_invulnerable: false,
            invulnerable_time: false,
            creative_source: false,
            client_loaded: self.client_loaded,
            rules: self.rules,
            source: self
                .resources
                .damage
                .source::<()>(id, None, None, None)
                .expect("validated starvation source"),
            weapon: None,
            random: self.random,
            living_cause: false,
            events: self.events,
            dead: self.dead,
            sync_velocity: self.sync_velocity,
            game_time: self.game_time,
            status: self.status.clone(),
            helmet: false,
            blocking: None,
            blocked: 0.0,
            shield_damage: 0,
            blocking_item_used: false,
            fully_blocked: false,
            block_sound: false,
            knocked: false,
            totem: false,
            totem_off_hand: false,
            totem_fired: false,
            canceled: false,
            dealt: 0.0,
        };
        if ServerPlayerCombat::new(self.attributes, self.data, food, self.ids, &mut environment)?
            .hurt(self.hurt, amount)?
        {
            environment
                .events
                .push(MotionEvent::Health(HealthEvent::StarvationDamage(amount)));
            *self.hurt_animation_time = self.hurt.hurt_time;
        }
        Ok(())
    }
}
