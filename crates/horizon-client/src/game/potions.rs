//! Potions and status effects in the game: drinking (`PotionItem` via
//! `Consumable`), throwing splash/lingering potions (`ThrownSplashPotion`),
//! filling glass bottles (`BottleItem`), the per-tick effect application,
//! effect attribute modifiers and the HUD effect icons (`Hud.extractEffects`).
use super::{Game, Screen};
use horizon_client::{
    brewing,
    effects::{EffectAction, EffectInstance, PlayerEffects},
    hud::{HudBatch, rgba},
    items::ItemStack,
};
use horizon_core::{geometry::Vec3, random::RandomSource};

/// A thrown splash or lingering potion in flight.
#[derive(Clone, Debug)]
pub(super) struct ThrownPotion {
    stack: ItemStack,
    position: Vec3,
    velocity: Vec3,
    age: u32,
}
/// Effects, thrown potions and the attribute modifiers last applied.
#[derive(Default)]
pub(super) struct PotionState {
    pub(super) effects: PlayerEffects,
    thrown: Vec<ThrownPotion>,
    applied: Vec<(&'static str, &'static str, f64, &'static str)>,
    /// Armor the protection table was computed for.
    protection_for: Option<Vec<Option<ItemStack>>>,
    /// Mobs set on fire by fire aspect: (mob id, ticks left).
    pub(super) burning: Vec<(u32, i32)>,
    /// Enchantment attribute modifiers on the player: (attribute, id, amount, operation).
    pub(super) enchant_modifiers: Vec<(String, String, f64, String)>,
    /// The level's trade-set `RandomSequence`s, by sequence id.
    pub(super) trade_randoms:
        std::collections::HashMap<String, horizon_core::random::XoroshiroRandom>,
}

impl Game<'_> {
    /// Right-click with a potion, splash/lingering potion or glass bottle.
    pub(super) fn use_potion_item(&mut self, stack: &ItemStack) -> horizon_client::Result<bool> {
        match &*stack.item {
            "minecraft:potion" => {
                if self.using.is_none() {
                    self.using = Some((self.selected, 0));
                }
                Ok(true)
            }
            "minecraft:splash_potion" | "minecraft:lingering_potion" => {
                let Some(eye) = self.eye_position() else {
                    return Ok(true);
                };
                let pitch = 0.4 / (self.world.random.next_float() * 0.4 + 0.8);
                let sound = if &*stack.item == "minecraft:splash_potion" {
                    "minecraft:entity.splash_potion.throw"
                } else {
                    "minecraft:entity.lingering_potion.throw"
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
                    (f64::from(dir[0]) / l + jitter()) * 0.5,
                    (f64::from(dir[1]) / l + jitter()) * 0.5,
                    (f64::from(dir[2]) / l + jitter()) * 0.5,
                );
                let pv = self.player.as_ref().map_or(Vec3::ZERO, |p| p.velocity);
                let on_ground = self.player.as_ref().is_some_and(|p| p.ground.on_ground);
                let v = Vec3::new(
                    v.x + pv.x,
                    v.y + if on_ground { 0.0 } else { pv.y },
                    v.z + pv.z,
                );
                self.potions.thrown.push(ThrownPotion {
                    stack: ItemStack {
                        count: 1,
                        ..stack.clone()
                    },
                    position: Vec3::new(eye[0], eye[1] - 0.1, eye[2]),
                    velocity: v,
                    age: 0,
                });
                self.swing_arm();
                self.consume_held();
                Ok(true)
            }
            "minecraft:glass_bottle" => {
                let Some(hit) = self.pick_fluid() else {
                    return Ok(false);
                };
                let state = self.world.block(hit.pos[0], hit.pos[1], hit.pos[2]);
                if self.res.facts.get(state).fluid != horizon_client::blocks::Fluid::Water {
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
        if self.inventory[self.selected].is_none() {
            self.inventory[self.selected] = Some(filled);
        } else if let Some(rest) = self.give(filled)
            && let Some(eye) = self.eye_position()
        {
            self.spawn_item(rest, eye, [0.0, 0.1, 0.0], 40);
        }
    }
    /// Drinking a potion while right-click is held; true when handled.
    pub(super) fn tick_drinking(&mut self, slot: usize, ticks: u32, stack: &ItemStack) -> bool {
        if &*stack.item != "minecraft:potion" {
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
        let (lasting, now) = PlayerEffects::potion_effects(stack.potion(), 1.0);
        for e in lasting {
            self.potions.effects.add(e);
        }
        for a in now {
            self.apply_effect_action(a);
        }
        if !self.creative {
            if let Some(s) = self.inventory[slot].as_mut() {
                s.count -= 1;
                if s.count == 0 {
                    self.inventory[slot] = None;
                }
            }
            let bottle = ItemStack::new("minecraft:glass_bottle", 1);
            if self.inventory[slot].is_none() {
                self.inventory[slot] = Some(bottle);
            } else if let Some(rest) = self.give(bottle) {
                self.spawn_item(rest, eye, [0.0, 0.1, 0.0], 40);
            }
        }
        true
    }
    fn apply_effect_action(&mut self, action: EffectAction) {
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
            EffectAction::Exhaust(amount) => {
                p.health.food.exhaustion = (p.health.food.exhaustion + amount).min(40.0);
            }
            EffectAction::Eat(nutrition, saturation) => {
                let food = &mut p.health.food;
                food.food = (food.food + nutrition).clamp(0, 20);
                food.saturation = (food.saturation + nutrition as f32 * saturation * 2.0)
                    .clamp(0.0, food.food as f32);
            }
        }
    }
    /// `LivingEntity.tickEffects`, attribute modifiers and the status flags.
    pub(super) fn tick_effects(&mut self) {
        let Some(player) = self.player.as_ref() else {
            return;
        };
        if player.health.dead_or_dying() {
            // LivingEntity.die → removeAllEffects.
            self.potions.effects.clear();
        }
        for action in self.potions.effects.tick() {
            self.apply_effect_action(action);
        }
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
        // EnchantmentHelper.getDamageProtection per damage type, recomputed on armor change.
        let armor: Vec<Option<ItemStack>> = self.armor.to_vec();
        if self.potions.protection_for.as_ref() != Some(&armor) {
            let mut table = Vec::new();
            let e = &self.res.stations.enchantments;
            let mut random = horizon_core::random::LegacyRandom::new(0);
            for (_, def) in self.motion.health.damage.entries() {
                let ctx = horizon_client::enchant::EffectContext {
                    items: Some(&self.res.items),
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
        self.tick_thrown_potions();
    }
    fn tick_thrown_potions(&mut self) {
        let mut thrown = std::mem::take(&mut self.potions.thrown);
        let mut landed = Vec::new();
        thrown.retain_mut(|t| {
            t.age += 1;
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
                if self.res.facts.get(state).full_collision
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
                landed.push((t.stack.clone(), p));
                return false;
            }
            t.position = next;
            // ThrowableItemProjectile: drag 0.99 then gravity 0.05.
            t.velocity = Vec3::new(
                t.velocity.x * 0.99,
                t.velocity.y * 0.99 - 0.05,
                t.velocity.z * 0.99,
            );
            t.age < 1200 && t.position.y > -128.0
        });
        // ThrownItemRenderer stand-in: a short-lived sprite of the potion at its position.
        for t in &thrown {
            let sprite = if &*t.stack.item == "minecraft:lingering_potion" {
                "item/lingering_potion"
            } else {
                "item/splash_potion"
            };
            let mut fixed = horizon_core::random::LegacyRandom::new(0);
            let mut p = horizon_client::particles::Particle::terrain(
                &mut fixed,
                [t.position.x, t.position.y, t.position.z],
                [0.0; 3],
                sprite.into(),
                0xFFFFFF,
            );
            p.velocity = [0.0; 3];
            p.gravity = 0.0;
            p.lifetime = 2;
            p.size = 0.25;
            p.uv = [0.0, 0.0, 1.0, 1.0];
            self.particles.push(p);
        }
        self.potions.thrown = thrown;
        for (stack, at) in landed {
            self.splash(&stack, at);
        }
    }
    /// `ThrownSplashPotion.onHitAsPotion` (lingering clouds apply as a weaker splash).
    fn splash(&mut self, stack: &ItemStack, at: Vec3) {
        let lingering = &*stack.item == "minecraft:lingering_potion";
        let event = if lingering {
            "minecraft:entity.lingering_potion.break"
        } else {
            "minecraft:entity.splash_potion.break"
        };
        let pitch = self.world.random.next_float() * 0.1 + 0.9;
        self.audio.play(event, Some([at.x, at.y, at.z]), 1.0, pitch);
        let color = brewing::potion_color(stack.potion());
        for _ in 0..8 {
            let v = [
                f64::from(self.world.random.next_float() - 0.5) * 0.3,
                0.2,
                f64::from(self.world.random.next_float() - 0.5) * 0.3,
            ];
            let p = horizon_client::particles::Particle::terrain(
                &mut self.world.random,
                [at.x, at.y, at.z],
                v,
                "item/splash_potion".into(),
                color,
            );
            self.particles.push(p);
        }
        let scale = if lingering { 0.25 } else { 1.0 };
        let Some(def) = stack.potion().and_then(brewing::potion) else {
            return;
        };
        // The player.
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
                for (id, duration, amp) in def.effects {
                    let instant = brewing::effect(id).is_some_and(|e| e.instant);
                    if instant {
                        if let Some(a) = horizon_client::effects::instant(id, *amp, intensity) {
                            self.apply_effect_action(a);
                        }
                    } else {
                        let d = (intensity * f64::from(*duration) * scale + 0.5) as i32;
                        if d > 20
                            && let Some(e) = brewing::effect(id)
                        {
                            self.potions.effects.add(EffectInstance::new(e.id, d, *amp));
                        }
                    }
                }
            }
        }
        // Mobs take the instant effects (undead invert healing and harming).
        let undead = &self.res.stations.enchantments.entity_tags;
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
            for (id, _, amp) in def.effects {
                let harm = match *id {
                    "instant_damage" => !inverted,
                    "instant_health" => inverted,
                    _ => continue,
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
        let mut list: Vec<&EffectInstance> = self.potions.effects.list.iter().collect();
        let color = |e: &EffectInstance| brewing::effect(e.id).map_or(0, |d| d.color);
        list.sort_by(|a, b| {
            (a.ambient, a.duration == -1, a.duration, color(a))
                .cmp(&(b.ambient, b.duration == -1, b.duration, color(b)))
                .reverse()
        });
        let (mut good, mut bad) = (0, 0);
        let atlas = &self.res.atlas;
        for e in list {
            let beneficial =
                brewing::effect(e.id).is_some_and(|d| d.category == brewing::Category::Beneficial);
            let mut x = batch.width;
            let mut y = 1.0;
            if beneficial {
                good += 1;
                x -= 25.0 * good as f32;
            } else {
                bad += 1;
                x -= 25.0 * bad as f32;
                y += 26.0;
            }
            let mut alpha = 1.0f32;
            if e.ambient {
                batch.sprite(atlas, "gui/sprites/hud/effect_background_ambient", x, y);
            } else {
                batch.sprite(atlas, "gui/sprites/hud/effect_background", x, y);
                if e.duration != -1 && e.duration <= 200 {
                    let d = e.duration as f32;
                    let used = 10.0 - (e.duration / 20) as f32;
                    alpha = ((d / 10.0 / 5.0 * 0.5).clamp(0.0, 0.5)
                        + (d * std::f32::consts::PI / 5.0).cos()
                            * (used / 10.0 * 0.25).clamp(0.0, 0.25))
                    .clamp(0.0, 1.0);
                }
            }
            batch.sprite_scaled(
                atlas,
                &format!("mob_effect/{}", e.id),
                x + 3.0,
                y + 3.0,
                18.0,
                18.0,
                rgba(255, 255, 255, (alpha * 255.0) as u8),
            );
        }
    }
    /// Effects saved in level.dat (`Player.active_effects`).
    pub(super) fn restore_effects(&mut self) {
        self.potions = PotionState::default();
        for (id, duration, amplifier, ambient, _) in self.level.active_effects.clone() {
            if let Some(def) = brewing::effect(&id) {
                let mut e = EffectInstance::new(def.id, duration, amplifier);
                e.ambient = ambient;
                self.potions.effects.add(e);
            }
        }
    }
    pub(super) fn effects_json(&self) -> serde_json::Value {
        serde_json::json!({
            "active": self.potions.effects.list.iter().map(|e| serde_json::json!({ "id": e.id, "duration": e.duration, "amplifier": e.amplifier })).collect::<Vec<_>>(),
            "thrown": self.potions.thrown.iter().map(|t| [t.position.x, t.position.y, t.position.z]).collect::<Vec<_>>(),
        })
    }
}
