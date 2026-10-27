//! Potions and status effects in the game: drinking (`PotionItem` via
//! `Consumable`), throwing splash/lingering potions (`ThrownSplashPotion`),
//! filling glass bottles (`BottleItem`), the per-tick effect application,
//! effect attribute modifiers and the HUD effect icons (`Hud.extractEffects`).
use super::{Game, Screen, item_draw::owner_at};
use horizon_assets::item_model::DisplayContext;
use horizon_client::{
    block_entity_render::vertex_light,
    brewing,
    effects::{EffectAction, EffectInstance, PlayerEffects},
    hud::HudBatch,
    hud_layout, hud_overlays, inventory_effects,
    items::ItemStack,
    mesher::Vertex,
    text::Text,
};
use horizon_core::{geometry::Vec3, random::RandomSource};

/// A thrown splash or lingering potion in flight.
#[derive(Clone, Debug)]
pub(super) struct ThrownPotion {
    /// Its entity id once a server's clients track it (0 until then).
    id: u32,
    stack: ItemStack,
    /// The position before the last tick, for drawing between ticks.
    previous: Vec3,
    position: Vec3,
    velocity: Vec3,
    age: u32,
    /// `Projectile.getOwner`'s display name: the player who threw it (none
    /// for a dispenser's).
    owner: Option<Text>,
}
/// A mob's effect ids carry the `minecraft:` namespace.
fn mob_effect_id(id: &str) -> Option<&'static str> {
    use std::{collections::HashMap, sync::OnceLock};
    static IDS: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    let id = id.trim_start_matches("minecraft:");
    IDS.get_or_init(|| {
        brewing::EFFECTS
            .iter()
            .map(|e| {
                let full: &'static str = Box::leak(format!("minecraft:{}", e.id).into_boxed_str());
                (e.id, full)
            })
            .collect()
    })
    .get(id)
    .copied()
}
impl Game<'_> {
    /// A splash potion, lingering potion or experience bottle in flight (the
    /// player's throws and dispensers both end up here).
    pub(super) fn launch_thrown(
        &mut self,
        stack: ItemStack,
        position: Vec3,
        velocity: Vec3,
        owner: Option<Text>,
    ) {
        self.potions.thrown.push(ThrownPotion {
            id: 0,
            stack,
            previous: position,
            position,
            velocity,
            age: 0,
            owner,
        });
    }
}
/// What a player's body was last given for its effects and enchantments:
/// a guest keeps their own while the host's are in `PotionState`.
#[derive(Default)]
pub(super) struct BodyPotions {
    applied: Vec<(&'static str, &'static str, f64, &'static str)>,
    protection_for: Option<Vec<Option<ItemStack>>>,
    enchant_modifiers: Vec<(String, String, f64, String)>,
}
/// Effects, thrown potions and the attribute modifiers last applied.
#[derive(Default)]
pub(super) struct PotionState {
    pub(super) effects: PlayerEffects,
    thrown: Vec<ThrownPotion>,
    /// The area effect clouds a server tracks (game/remote_projectiles.rs).
    pub(super) clouds: Vec<super::remote_projectiles::RemoteCloud>,
    applied: Vec<(&'static str, &'static str, f64, &'static str)>,
    /// Armor the protection table was computed for.
    protection_for: Option<Vec<Option<ItemStack>>>,

    /// Enchantment attribute modifiers on the player: (attribute, id, amount, operation).
    pub(super) enchant_modifiers: Vec<(String, String, f64, String)>,
}

impl PotionState {
    /// Trade the player's own state (effects and what their body was given)
    /// with a guest's, around `as_guest`. Potions in flight stay.
    pub(super) fn swap_player(&mut self, effects: &mut PlayerEffects, body: &mut BodyPotions) {
        std::mem::swap(&mut self.effects, effects);
        std::mem::swap(&mut self.applied, &mut body.applied);
        std::mem::swap(&mut self.protection_for, &mut body.protection_for);
        std::mem::swap(&mut self.enchant_modifiers, &mut body.enchant_modifiers);
    }
    /// Thrown potions in flight as (entity type, position, motion), for the
    /// dispenser parity fixture.
    pub(super) fn thrown_entities(&self) -> Vec<(&'static str, Vec3, Vec3)> {
        self.thrown
            .iter()
            .map(|t| (thrown_kind(&t.stack), t.position, t.velocity))
            .collect()
    }
    pub(super) fn clear_thrown(&mut self) {
        self.thrown.clear();
    }
    /// Their entity ids, for a server to number them.
    pub(super) fn thrown_ids(&mut self) -> impl Iterator<Item = &mut u32> {
        self.thrown.iter_mut().map(|t| &mut t.id)
    }
    /// The numbered ones as a server's clients track them: (id, entity
    /// type, item, position, motion).
    pub(super) fn thrown_tracked(
        &self,
    ) -> impl Iterator<Item = (u32, &'static str, &ItemStack, Vec3, Vec3)> {
        self.thrown.iter().filter(|t| t.id != 0).map(|t| {
            (
                t.id,
                thrown_kind(&t.stack),
                &t.stack,
                t.position,
                t.velocity,
            )
        })
    }
    /// `handleAddEntity` for a thrown potion or bottle on a server's client:
    /// drawn where the server says (its item comes in its entity data).
    pub(super) fn add_remote_thrown(&mut self, id: u32, kind: &str, at: Vec3, velocity: Vec3) {
        self.thrown.retain(|t| t.id != id);
        let item = match kind {
            "minecraft:lingering_potion" | "minecraft:experience_bottle" => kind,
            _ => "minecraft:splash_potion",
        };
        self.thrown.push(ThrownPotion {
            id,
            stack: ItemStack::new(item, 1),
            previous: at,
            position: at,
            velocity,
            age: 0,
            owner: None,
        });
    }
    /// ThrowableItemProjectile.DATA_ITEM_STACK: the potion it shows.
    pub(super) fn remote_thrown_item(&mut self, id: u32, stack: ItemStack) {
        if let Some(t) = self.thrown.iter_mut().find(|t| t.id == id) {
            t.stack = stack;
        }
    }
    /// A server's move: thrown potions have no interpolation handler, so
    /// `moveOrInterpolateTo` puts them there at once. False when `id` isn't one.
    pub(super) fn move_remote_thrown(&mut self, id: u32, at: Vec3) -> bool {
        let Some(t) = self.thrown.iter_mut().find(|t| t.id == id) else {
            return false;
        };
        t.position = at;
        true
    }
    /// `handleSetEntityMotion` for a thrown potion. False when `id` isn't one.
    pub(super) fn remote_thrown_motion(&mut self, id: u32, velocity: Vec3) -> bool {
        let Some(t) = self.thrown.iter_mut().find(|t| t.id == id) else {
            return false;
        };
        t.velocity = velocity;
        true
    }
    pub(super) fn remove_remote_thrown(&mut self, id: u32) {
        self.thrown.retain(|t| t.id != id);
    }
    pub(super) fn has_thrown(&self, id: u32) -> bool {
        self.thrown.iter().any(|t| t.id == id)
    }
}

/// The entity type of a thrown potion or bottle.
fn thrown_kind(stack: &ItemStack) -> &'static str {
    match &*stack.item {
        "minecraft:lingering_potion" => "minecraft:lingering_potion",
        "minecraft:experience_bottle" => "minecraft:experience_bottle",
        _ => "minecraft:splash_potion",
    }
}

impl Game<'_> {
    /// Right-click with a potion, splash/lingering potion or glass bottle.
    pub(super) fn use_potion_item(&mut self, stack: &ItemStack) -> horizon_client::Result<bool> {
        match &*stack.item {
            "minecraft:potion" | "minecraft:ominous_bottle" => {
                if self.using.is_none() {
                    self.using = Some((self.held_slot(), 0));
                }
                Ok(true)
            }
            "minecraft:splash_potion"
            | "minecraft:lingering_potion"
            | "minecraft:experience_bottle" => {
                let Some(eye) = self.eye_position() else {
                    return Ok(true);
                };
                let pitch = 0.4 / (self.world.random.next_float() * 0.4 + 0.8);
                let (sound, power) = match &*stack.item {
                    "minecraft:splash_potion" => ("minecraft:entity.splash_potion.throw", 0.5),
                    "minecraft:experience_bottle" => {
                        ("minecraft:entity.experience_bottle.throw", 0.7)
                    }
                    _ => ("minecraft:entity.lingering_potion.throw", 0.5),
                };
                self.audio.play(sound, Some(eye), 0.5, pitch);
                // Projectile.shootFromRotation(xRot, yRot, -20, 0.5, 1.0).
                let (yaw, pitch) = (self.yaw.to_radians(), self.pitch.to_radians());
                let dir = [
                    -yaw.sin() * pitch.cos(),
                    -(self.pitch - 20.0).to_radians().sin(),
                    yaw.cos() * pitch.cos(),
                ];
                let l = dir
                    .iter()
                    .map(|c| f64::from(*c).powi(2))
                    .sum::<f64>()
                    .sqrt()
                    .max(1e-9);
                // getMovementToShoot: normalize, add triangle(0, 0.0172275 * uncertainty), scale.
                let r = &mut self.world.random;
                let mut jitter = || r.triangle(0.0, 0.0172275);
                let v = Vec3::new(
                    (f64::from(dir[0]) / l + jitter()) * power,
                    (f64::from(dir[1]) / l + jitter()) * power,
                    (f64::from(dir[2]) / l + jitter()) * power,
                );
                let pv = self.player.as_ref().map_or(Vec3::ZERO, |p| p.velocity);
                let on_ground = self.player.as_ref().is_some_and(|p| p.ground.on_ground);
                let v = Vec3::new(
                    v.x + pv.x,
                    v.y + if on_ground { 0.0 } else { pv.y },
                    v.z + pv.z,
                );
                self.launch_thrown(
                    ItemStack {
                        count: 1,
                        ..stack.clone()
                    },
                    Vec3::new(eye[0], eye[1] - 0.1, eye[2]),
                    v,
                    Some(self.player_display_text()),
                );
                self.swing_arm();
                self.consume_held();
                Ok(true)
            }
            "minecraft:glass_bottle" => {
                let Some(hit) = self.pick_fluid() else {
                    return Ok(false);
                };
                let state = self.world.block(hit.pos[0], hit.pos[1], hit.pos[2]);
                if self.res.facts().get(state).fluid != horizon_client::blocks::Fluid::Water {
                    return Ok(false);
                }
                let at = [
                    f64::from(hit.pos[0]) + 0.5,
                    f64::from(hit.pos[1]) + 0.5,
                    f64::from(hit.pos[2]) + 0.5,
                ];
                self.audio
                    .play("minecraft:item.bottle.fill", Some(at), 1.0, 1.0);
                let water = ItemStack::new("minecraft:potion", 1).with_potion("minecraft:water");
                self.fill_held(water);
                Ok(true)
            }
            _ => Ok(false),
        }
    }
    /// `ItemUtils.createFilledResult`: replace one of the held stack.
    fn fill_held(&mut self, filled: ItemStack) {
        if self.creative {
            let _ = self.give(filled);
            return;
        }
        self.consume_held();
        if self.held().is_none() {
            *self.held_mut() = Some(filled);
        } else if let Some(rest) = self.give(filled)
            && let Some(eye) = self.eye_position()
        {
            // `player.drop(stack, false)`.
            self.throw_from_eyes(rest, eye, false);
        }
    }
    /// Drinking a potion while right-click is held; true when handled.
    pub(super) fn tick_drinking(&mut self, slot: usize, ticks: u32, stack: &ItemStack) -> bool {
        let ominous = &*stack.item == "minecraft:ominous_bottle";
        if &*stack.item != "minecraft:potion" && !ominous {
            return false;
        }
        let ticks = ticks + 1;
        let remaining = 32u32.saturating_sub(ticks);
        let eye = self.eye_position().unwrap_or_default();
        // Consumable.emitParticlesAndSounds: the drink sound every 4 ticks near the end.
        if remaining <= 25 && remaining.is_multiple_of(4) {
            let pitch = self.world.random.next_float() * 0.1 + 0.9;
            self.audio
                .play("minecraft:entity.generic.drink", Some(eye), 0.5, pitch);
        }
        if ticks < 32 {
            self.using = Some((slot, ticks));
            return true;
        }
        self.using = None;
        self.post_entity_event("minecraft:drink", horizon_client::mobs::Who::Player);
        if ominous {
            self.drink_ominous_bottle(slot, stack);
            return true;
        }
        let (lasting, now) = PlayerEffects::potion_effects(stack.potion(), 1.0);
        for e in lasting {
            self.add_player_effect(e);
        }
        for a in now {
            self.apply_effect_action(a);
        }
        if !self.creative {
            if let Some(s) = self.stack_in_mut(slot).as_mut() {
                s.count -= 1;
                if s.count == 0 {
                    *self.stack_in_mut(slot) = None;
                }
            }
            let bottle = ItemStack::new("minecraft:glass_bottle", 1);
            if self.stack_in(slot).is_none() {
                *self.stack_in_mut(slot) = Some(bottle);
            } else if let Some(rest) = self.give(bottle) {
                self.spawn_item(rest, eye, [0.0, 0.1, 0.0], 40);
            }
        }
        true
    }
    pub(super) fn apply_effect_action(&mut self, action: EffectAction) {
        let creative = self.creative;
        let rules = self.world.health_rules;
        let Some(p) = self.player.as_mut() else {
            return;
        };
        match action {
            EffectAction::Heal(amount) => {
                let max = p.attribute("minecraft:max_health").unwrap_or(20.0) as f32;
                let h = p.health.health();
                if h > 0.0 {
                    let _ = p.set_health((h + amount).min(max), false);
                }
            }
            EffectAction::Hurt(source, amount) => {
                if creative {
                    return;
                }
                if let Some(id) = self.motion.health.damage.id(source) {
                    let _ = p.hurt(&self.motion.health, rules, id, amount);
                }
            }
            EffectAction::Poison => {
                if !creative
                    && p.health.health() > 1.0
                    && let Some(id) = self.motion.health.damage.id("minecraft:magic")
                {
                    let _ = p.hurt(&self.motion.health, rules, id, 1.0);
                }
            }
            EffectAction::Exhaust(amount) => {
                p.cause_food_exhaustion(amount);
            }
            EffectAction::Eat(nutrition, saturation) => {
                let food = &mut p.health.food;
                food.food = (food.food + nutrition).clamp(0, 20);
                food.saturation = (food.saturation + nutrition as f32 * saturation * 2.0)
                    .clamp(0.0, food.food as f32);
            }
        }
    }
    /// The local player's effects (`tick_mob_effects`), then the potions
    /// in flight, which a server without a local player still has. A frozen
    /// tick (`/tick freeze`) leaves the potions where they are
    /// (`TickRateManager.isEntityFrozen`); players' effects still run.
    pub(super) fn tick_effects(&mut self, running: bool) {
        self.tick_mob_effects();
        if running {
            self.tick_thrown_potions();
        }
    }
    /// `LivingEntity.tickEffects` (server side) for `self.player`, the local
    /// player or a guest acting (`as_guest`), then its attribute modifiers
    /// and status flags.
    pub(super) fn tick_mob_effects(&mut self) {
        let Some(player) = self.player.as_ref() else {
            return;
        };
        if player.health.dead_or_dying() {
            // LivingEntity.die → removeAllEffects.
            self.potions.effects.clear();
        }
        // The omens act from inside the effect tick (BadOmenMobEffect,
        // RaidOmenMobEffect).
        let mut effects = std::mem::take(&mut self.potions.effects);
        let (actions, raid_omen_added) = effects.tick_with(&mut |e| self.omen_tick(e));
        self.potions.effects = effects;
        if raid_omen_added {
            self.raid_omen_added();
        }
        for action in actions {
            self.apply_effect_action(action);
        }
        self.turtle_helmet_tick();
        self.apply_effect_status();
    }
    /// `Player.tick` -> `turtleHelmetTick`: with a turtle helmet on the head
    /// and the eyes out of the water, Water Breathing for 200 ticks (no
    /// particles, an icon).
    fn turtle_helmet_tick(&mut self) {
        let Some(p) = self.player.as_ref() else {
            return;
        };
        let eye_y = p.position.y + f64::from(p.dimensions.eye_height);
        if !p.health.dead_or_dying()
            && self.armor[0]
                .as_ref()
                .is_some_and(|s| &*s.item == "minecraft:turtle_helmet")
            && !self.eye_in_water(p.position.x, eye_y, p.position.z)
        {
            let mut effect = EffectInstance::new("water_breathing", 200, 0);
            effect.visible = false;
            self.potions.effects.add(effect);
        }
    }
    /// A client's own effects (`LivingEntity.tickEffects` on the client,
    /// `MobEffectInstance.tickClient`): they count down, and the server
    /// removes them. Their modifiers and flags move the predicted player.
    pub(super) fn tick_client_effects(&mut self) {
        for e in &mut self.potions.effects.list {
            e.tick_client();
        }
        self.apply_effect_status();
    }
    /// Add an active effect to the player and update attributes / absorption.
    pub(super) fn add_player_effect(&mut self, effect: EffectInstance) -> bool {
        let id = effect.id;
        let amp = effect.amplifier;
        let changed = self.potions.effects.add(effect);
        self.apply_effect_status();
        if id == "absorption"
            && let Some(p) = self.player.as_mut()
        {
            let target = 4.0 * (amp + 1) as f32;
            let current = p.health.absorption();
            let _ = p.set_health(current.max(target), true);
        }
        changed
    }
    /// The effects' attribute modifiers, armor protection and status flags
    /// on `self.player`.
    pub(super) fn apply_effect_status(&mut self) {
        let modifiers = self.potions.effects.modifiers();
        if modifiers != self.potions.applied
            && let Some(p) = self.player.as_mut()
        {
            for (attr, id, _, op) in &self.potions.applied {
                let _ = p.set_modifier(attr, id, 0.0, op, false);
            }
            for (attr, id, amount, op) in &modifiers {
                let _ = p.set_modifier(attr, id, *amount, op, true);
            }
            self.potions.applied = modifiers;
        }
        if let Some(p) = self.player.as_mut()
            && self.potions.effects.amplifier("absorption").is_none()
            && p.health.absorption() > 0.0
        {
            let _ = p.set_health(0.0, true);
        }
        // EnchantmentHelper.getDamageProtection per damage type, recomputed on armor change.
        let armor: Vec<Option<ItemStack>> = self.armor.to_vec();
        if self.potions.protection_for.as_ref() != Some(&armor) {
            let mut table = Vec::new();
            let e = &self.res.stations().enchantments;
            let mut random = horizon_core::random::LegacyRandom::new(0);
            for (_, def) in self.motion.health.damage.entries() {
                let ctx = horizon_client::enchant::EffectContext {
                    items: Some(self.res.items()),
                    damage_type: Some(&def.name),
                    this: Some("minecraft:player"),
                    ..Default::default()
                };
                let total: f32 = armor
                    .iter()
                    .flatten()
                    .map(|s| {
                        e.modify_value(
                            Some(s),
                            "minecraft:damage_protection",
                            &ctx,
                            &mut random,
                            0.0,
                        )
                    })
                    .sum();
                if total > 0.0 {
                    table.push((def.name.clone(), total));
                }
            }
            self.potions.protection_for = Some(armor);
            if let Some(p) = self.player.as_mut() {
                p.status.protection = table;
            }
        }
        let mut status = self.potions.effects.status();
        if let Some(p) = self.player.as_mut() {
            status.protection = std::mem::take(&mut p.status.protection);
            p.status = status;
        }
    }
    fn tick_thrown_potions(&mut self) {
        let mut thrown = std::mem::take(&mut self.potions.thrown);
        let mut landed = Vec::new();
        thrown.retain_mut(|t| {
            t.age += 1;
            t.previous = t.position;
            self.thrown_potion_motion(t);
            let next = Vec3::new(
                t.position.x + t.velocity.x,
                t.position.y + t.velocity.y,
                t.position.z + t.velocity.z,
            );
            // Hit a block along the step (sampled), a mob, or fell out of the world.
            let steps = 4;
            let mut hit = None;
            for i in 1..=steps {
                let f = f64::from(i) / f64::from(steps);
                let p = Vec3::new(
                    t.position.x + t.velocity.x * f,
                    t.position.y + t.velocity.y * f,
                    t.position.z + t.velocity.z * f,
                );
                let state =
                    self.world
                        .block(p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32);
                if self.res.facts().get(state).full_collision
                    || self.mobs.iter().any(|m| {
                        m.alive()
                            && (m.position.x - p.x).abs() < m.kind.width / 2.0 + 0.125
                            && (m.position.z - p.z).abs() < m.kind.width / 2.0 + 0.125
                            && p.y >= m.position.y
                            && p.y <= m.position.y + m.kind.height
                    })
                {
                    hit = Some(p);
                    break;
                }
            }
            if let Some(p) = hit {
                landed.push((t.stack.clone(), p, t.owner.clone()));
                return false;
            }
            t.position = next;
            t.age < 1200 && t.position.y > -128.0
        });
        self.potions.thrown = thrown;
        for (stack, at, owner) in landed {
            self.splash(&stack, at, owner);
        }
    }
    /// `ThrowableProjectile.tick`'s `applyGravity` and `applyInertia`, which
    /// come before the move: gravity 0.05 (0.07 for an experience bottle),
    /// then drag 0.99 (0.8 in water).
    fn thrown_potion_motion(&self, t: &mut ThrownPotion) {
        let gravity = if &*t.stack.item == "minecraft:experience_bottle" {
            0.07
        } else {
            0.05
        };
        let at = t.position;
        let water = self
            .res
            .facts()
            .get(self.world.block(
                at.x.floor() as i32,
                at.y.floor() as i32,
                at.z.floor() as i32,
            ))
            .fluid
            == horizon_client::blocks::Fluid::Water;
        let inertia = if water { 0.8 } else { 0.99 };
        t.velocity = Vec3::new(
            t.velocity.x * inertia,
            (t.velocity.y - gravity) * inertia,
            t.velocity.z * inertia,
        );
    }
    /// `ThrowableProjectile.tick` on a server's client: it falls and moves
    /// by itself between the server's updates and stops at a block; only
    /// the server breaks it.
    pub(super) fn step_remote_thrown(&mut self) {
        let mut thrown = std::mem::take(&mut self.potions.thrown);
        for t in &mut thrown {
            t.previous = t.position;
            t.age += 1;
            self.thrown_potion_motion(t);
            let next = Vec3::new(
                t.position.x + t.velocity.x,
                t.position.y + t.velocity.y,
                t.position.z + t.velocity.z,
            );
            let state = self.world.block(
                next.x.floor() as i32,
                next.y.floor() as i32,
                next.z.floor() as i32,
            );
            if self.res.facts().get(state).full_collision {
                t.velocity = Vec3::ZERO;
            } else {
                t.position = next;
            }
        }
        self.potions.thrown = thrown;
    }
    /// `ThrownItemRenderer` for thrown splash and lingering potions and
    /// experience bottles: the item (GROUND) facing the camera at scale 1,
    /// lit at the light probe (the eye, 0.85 of the 0.25 height).
    pub(super) fn thrown_potion_geometry(
        &self,
        camera: glam::DVec3,
        alpha: f64,
        out: &mut Vec<Vertex>,
    ) {
        let orientation = glam::Mat4::from_quat(self.camera_orientation());
        for t in &self.potions.thrown {
            let previous = glam::DVec3::new(t.previous.x, t.previous.y, t.previous.z);
            let position = glam::DVec3::new(t.position.x, t.position.y, t.position.z);
            // ThrowableProjectile.shouldRenderAtSqrDistance: hidden for its
            // first two ticks within 3.5 blocks (not in the thrower's face).
            if t.age < 2 && position.distance_squared(camera) < 12.25 {
                continue;
            }
            if !self.standing_entity_in_view(
                camera,
                super::entity_culling::at(t.previous),
                super::entity_culling::at(t.position),
                alpha,
                0.25,
                0.25,
                64.0,
                0.0,
            ) {
                continue;
            }
            let p = previous.lerp(position, alpha);
            let owner = owner_at(p.to_array(), 0.0, &t.stack.item);
            let state = self.resolve_item(&t.stack, DisplayContext::Ground, Some(&owner), 0);
            if state.is_empty() {
                continue;
            }
            let pose = glam::Mat4::from_translation((p - camera).as_vec3()) * orientation;
            let light = self.world.light(
                p.x.floor() as i32,
                (p.y + 0.2125).floor() as i32,
                p.z.floor() as i32,
            );
            self.submit_item_world(&state, pose, vertex_light(light), out);
        }
    }
    /// A splash potion's effects on the player in place, by their distance.
    fn splash_on_player(
        &mut self,
        effects: &'static [(&'static str, i32, i32)],
        at: Vec3,
        scale: f64,
        source: &Text,
    ) {
        if let Some(p) = self.player.as_ref() {
            let center = Vec3::new(
                p.position.x,
                p.position.y + f64::from(p.dimensions.height) / 2.0,
                p.position.z,
            );
            let d2 =
                (center.x - at.x).powi(2) + (center.y - at.y).powi(2) + (center.z - at.z).powi(2);
            if d2 < 16.0 && (center.y - at.y).abs() <= 2.0 + f64::from(p.dimensions.height) {
                let intensity = 1.0 - d2.sqrt() / 4.0;
                for (id, duration, amp) in effects {
                    let instant = brewing::effect(id).is_some_and(|e| e.instant);
                    if instant {
                        if let Some(a) = horizon_client::effects::instant(id, *amp, intensity) {
                            // HealOrHarmMobEffect.applyInstantaneousEffect with
                            // a source: indirectMagic, not plain magic.
                            let a = match a {
                                EffectAction::Hurt("minecraft:magic", amount) => {
                                    self.note_direct_entity(
                                        "minecraft:indirect_magic",
                                        source.clone(),
                                    );
                                    EffectAction::Hurt("minecraft:indirect_magic", amount)
                                }
                                a => a,
                            };
                            self.apply_effect_action(a);
                        }
                    } else {
                        let d = (intensity * f64::from(*duration) * scale + 0.5) as i32;
                        if d > 20
                            && let Some(e) = brewing::effect(id)
                        {
                            self.add_player_effect(EffectInstance::new(e.id, d, *amp));
                        }
                    }
                }
            }
        }
    }
    /// `ThrownSplashPotion.onHitAsPotion` (lingering clouds apply as a weaker splash).
    fn splash(&mut self, stack: &ItemStack, at: Vec3, owner: Option<Text>) {
        let block = [at.x, at.y, at.z].map(|v| v.floor() as i32);
        if &*stack.item == "minecraft:experience_bottle" {
            // ThrownExperienceBottle.onHit: the water-coloured splash (level
            // event 2002, its sound 1053) and 3 to 11 experience.
            self.level_event_at(2002, block, 0xFF38_5DC6u32 as i32);
            self.level_event_at(1053, block, 0);
            let r = &mut self.world.random;
            let value = 3 + r.next_int_bound(5).unwrap_or(0) + r.next_int_bound(5).unwrap_or(0);
            self.spawn_orbs([at.x, at.y, at.z], value as u32);
            return;
        }
        let lingering = &*stack.item == "minecraft:lingering_potion";
        // AbstractThrownPotion.onHit: level event 2007 and sound 1054 for a
        // potion with instant effects, 2002 and 1053 otherwise.
        let potion = stack.potion().and_then(brewing::potion);
        let instant = potion.is_some_and(|d| {
            d.effects
                .iter()
                .any(|(id, ..)| brewing::effect(id).is_some_and(|e| e.instant))
        });
        let color = brewing::potion_color(stack.potion());
        self.level_event_at(if instant { 2007 } else { 2002 }, block, color as i32);
        self.level_event_at(if instant { 1054 } else { 1053 }, block, 0);
        let scale = if lingering { 0.25 } else { 1.0 };
        let Some(def) = potion else {
            return;
        };
        // Every player in range (`ThrownSplashPotion.applySplash` over
        // `LivingEntity`s, the players among them).
        // Instant damage is indirectMagic(potion or cloud, owner), which the
        // death message names by the owner, else by the potion or cloud.
        let source = owner.unwrap_or_else(|| {
            let entity = if lingering {
                "entity.minecraft.area_effect_cloud"
            } else {
                "entity.minecraft.splash_potion"
            };
            Text::translate(entity, &[])
        });
        self.for_each_player(|g| g.splash_on_player(def.effects, at, scale, &source));
        // Mobs take the instant effects (undead invert healing and harming).
        let undead = &self.res.stations().enchantments.entity_tags;
        let mut hits = Vec::new();
        for (i, m) in self.mobs.iter().enumerate() {
            let d2 = (m.position.x - at.x).powi(2)
                + (m.position.y + m.kind.height / 2.0 - at.y).powi(2)
                + (m.position.z - at.z).powi(2);
            if m.alive() && d2 < 16.0 {
                let inverted = undead
                    .get("minecraft:inverted_healing_and_harm")
                    .is_some_and(|t| t.contains(&m.kind.id));
                hits.push((i, 1.0 - d2.sqrt() / 4.0, inverted));
            }
        }
        for (i, intensity, inverted) in hits {
            for (id, duration, amp) in def.effects {
                let harm = match *id {
                    "instant_damage" => !inverted,
                    "instant_health" => inverted,
                    _ => {
                        // `Potion.applySplash`: timed effects last in
                        // proportion to how close the mob was.
                        let d = (intensity * f64::from(*duration) * scale + 0.5) as i32;
                        if d > 20
                            && let Some(name) = mob_effect_id(id)
                        {
                            self.mobs[i].add_effect(name, d, *amp);
                        }
                        continue;
                    }
                };
                let m = &mut self.mobs[i];
                if harm {
                    m.hurt(
                        (intensity * f64::from(6 << amp)) as i32 as f32,
                        Some([at.x, at.y, at.z]),
                        &mut self.world.random,
                    );
                } else {
                    m.health = (m.health + (intensity * f64::from(4 << amp)) as i32 as f32)
                        .min(m.kind.max_health);
                }
            }
        }
    }
    /// `Hud.extractEffects`: icons top-right, beneficial row then harmful row.
    pub(super) fn draw_effect_icons(&self, batch: &mut HudBatch) {
        if self.screen != Screen::Playing || self.potions.effects.list.is_empty() {
            return;
        }
        let icons: Vec<hud_overlays::EffectIcon> = self
            .potions
            .effects
            .list
            .iter()
            .map(hud_overlays::EffectIcon::from)
            .collect();
        let draws = hud_overlays::effects(batch.width as i32, false, &icons);
        hud_layout::paint(batch, &self.gui.atlas, &draws);
    }
    /// `EffectsInInventory`: panels to the right of survival/creative inventory.
    pub(super) fn draw_inventory_effects(
        &self,
        batch: &mut HudBatch,
        origin: (f32, f32),
        image_width: i32,
    ) {
        let panels = inventory_effects::panels(
            batch.width as i32,
            [origin.0 as i32, origin.1 as i32],
            image_width,
            &self.potions.effects.list,
            self.clock.rate(),
            &|key| self.pack.names.display_get(key),
            &|text| self.gui.atlas.font.width(text).ceil() as i32,
        );
        if let Some(lines) = inventory_effects::paint(
            batch,
            &self.gui.atlas,
            &panels,
            [self.cursor.0 as i32, self.cursor.1 as i32],
        ) {
            self.draw_tooltip_lines(batch, &lines, self.cursor.0, self.cursor.1);
        }
    }
    /// Effects saved in level.dat (`Player.active_effects`).
    pub(super) fn restore_effects(&mut self) {
        self.potions = PotionState::default();
        self.potions.effects = PlayerEffects::from_saved(&self.level.active_effects);
    }
    pub(super) fn effects_json(&self) -> serde_json::Value {
        serde_json::json!({
            "active": self.potions.effects.list.iter().map(|e| serde_json::json!({ "id": e.id, "duration": e.duration, "amplifier": e.amplifier })).collect::<Vec<_>>(),
            "thrown": self.potions.thrown.iter().map(|t| [t.position.x, t.position.y, t.position.z]).collect::<Vec<_>>(),
        })
    }
}
