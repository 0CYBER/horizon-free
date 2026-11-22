//! Enchantment effects on gameplay, read from the data-driven effect
//! components: efficiency and aqua affinity (`Player.getDestroySpeed`),
//! unbreaking (`minecraft:item_damage`), sharpness/smite/bane and power
//! (`minecraft:damage`), knockback, fire aspect, infinity (`minecraft:ammo_use`),
//! thorns (`minecraft:post_attack`), mending (`minecraft:repair_with_xp`) and
//! the armor attribute effects (respiration, depth strider, ...), Soul
//! Speed's `location_changed` and `tick` effects and the
//! `minecraft:location_changed` disk effect (frost walker).
use super::Game;
use super::durability::Wearer;
use horizon_client::{
    enchant::{EffectContext, Enchantments, level_value},
    items::{ItemStack, Items},
    mobs::{Damage, DamageKind, Who},
    random_source,
};
use horizon_core::{
    geometry::Vec3,
    position::BlockPos,
    random::{LegacyRandom, RandomSource},
};
use horizon_world::{fluid::FluidKind, level::LevelAccess, registry::BlockStateId};
use serde_json::Value;

/// Shared weapon damage effects; callers choose live RNG for an attack or a snapshot for a read.
pub(super) fn stack_enchanted_damage(
    enchantments: &Enchantments,
    stack: &ItemStack,
    ctx: &EffectContext<'_>,
    random: &mut horizon_core::random::LegacyRandom,
    base: f32,
) -> f32 {
    enchantments.modify_value(Some(stack), "minecraft:damage", ctx, random, base)
}

/// `EquipmentSlot.getName` of each `mobs::finalize` slot (hands, feet to head).
const MOB_SLOTS: [&str; 6] = ["mainhand", "offhand", "feet", "legs", "chest", "head"];

/// One of the player's equipment slots (`EnchantedItemInUse.inSlot`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PlayerSlot {
    Main,
    Off,
    /// An `armor` index (head first).
    Armor(usize),
}

/// `EquipmentSlot.VALUES` over what the player holds and wears: the main
/// hand, the off hand, then feet, legs, chest and head (`armor` holds head
/// first).
const PLAYER_SLOTS: [PlayerSlot; 6] = [
    PlayerSlot::Main,
    PlayerSlot::Off,
    PlayerSlot::Armor(3),
    PlayerSlot::Armor(2),
    PlayerSlot::Armor(1),
    PlayerSlot::Armor(0),
];

impl PlayerSlot {
    /// `EquipmentSlot.getName`.
    fn name(self) -> &'static str {
        match self {
            Self::Main => "mainhand",
            Self::Off => "offhand",
            Self::Armor(i) => ["head", "chest", "legs", "feet"][i],
        }
    }
    fn equipment_slot(self) -> mod_api::EquipmentSlot {
        use mod_api::EquipmentSlot;
        match self {
            Self::Main => EquipmentSlot::MainHand,
            Self::Off => EquipmentSlot::OffHand,
            Self::Armor(i) => [
                EquipmentSlot::Head,
                EquipmentSlot::Chest,
                EquipmentSlot::Legs,
                EquipmentSlot::Feet,
            ][i],
        }
    }
}

/// `ApplyEntityImpulse`'s impulse for an entity looking along `yaw` and
/// `pitch`: the effect's `direction` turned by the look
/// (`Entity.getLookQuaternion`: pitch about x, then yaw about y), scaled by
/// `coordinate_scale` and the level's `magnitude`.
pub(super) fn look_impulse(effect: &Value, level: u32, yaw: f32, pitch: f32) -> Vec3 {
    let vec3 = |key: &str| [0, 1, 2].map(|i| effect[key][i].as_f64().unwrap_or(0.0));
    let (d, scale) = (vec3("direction"), vec3("coordinate_scale"));
    let magnitude = f64::from(level_value(&effect["magnitude"], level));
    // JOML Quaternionf.rotationY(...).rotateX(...).transform(Vector3d):
    // quaternion and matrix coefficients round to float before multiplying
    // the double direction. Do not replace this with double Euler rotation.
    let half_y = (-yaw * (std::f32::consts::PI / 180.0)) * 0.5;
    let half_x = (pitch * (std::f32::consts::PI / 180.0)) * 0.5;
    let sy = f64::from(half_y).sin() as f32;
    let sx = f64::from(half_x).sin() as f32;
    let cy = horizon_assets::math::cos_from_sin(sy, half_y);
    let cx = horizon_assets::math::cos_from_sin(sx, half_x);
    let (x, y, z, w) = (cy * sx, sy * cx, -sy * sx, cy * cx);
    let (xx, yy, zz) = (x * x, y * y, z * z);
    let (xy, xz, yz, xw, yw, zw) = (x * y, x * z, y * z, x * w, y * w, z * w);
    let matrix = [
        [1.0 - 2.0 * (yy + zz), 2.0 * (xy - zw), 2.0 * (xz + yw)],
        [2.0 * (xy + zw), 1.0 - 2.0 * (xx + zz), 2.0 * (yz - xw)],
        [2.0 * (xz - yw), 2.0 * (yz + xw), 1.0 - 2.0 * (xx + yy)],
    ];
    let result = matrix
        .map(|row| f64::from(row[0]) * d[0] + f64::from(row[1]) * d[1] + f64::from(row[2]) * d[2]);
    Vec3::new(
        result[0] * scale[0] * magnitude,
        result[1] * scale[1] * magnitude,
        result[2] * scale[2] * magnitude,
    )
}
/// `Player.getDestroySpeed` multiplies by `BLOCK_BREAK_SPEED`; a player
/// without the attribute keeps the vanilla default of 1.0.
fn block_break_speed_factor(value: Option<f64>) -> f32 {
    value.map_or(1.0, |value| value as f32)
}

/// Says once per item that its parts' `xp_repair_ratio` answered `ratio`,
/// which isn't finite.
fn refuse_xp_repair_ratio(item: &str, ratio: f32) {
    static REFUSED: std::sync::Mutex<std::collections::BTreeSet<String>> =
        std::sync::Mutex::new(std::collections::BTreeSet::new());
    if REFUSED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(item.to_owned())
    {
        log::warn!(
            "Horizon: an xp_repair_ratio part of {item} answered {ratio}, which isn't finite; Mending's own repair is used"
        );
    }
}

impl Game<'_> {
    /// `Attributes.MINING_EFFICIENCY` (efficiency), which `Player.getDestroySpeed`
    /// adds to a tool speed above 1: 0 without a player or attribute.
    pub(super) fn mining_efficiency(&mut self) -> f32 {
        self.player
            .as_mut()
            .and_then(|player| player.attribute("minecraft:mining_efficiency").ok())
            .unwrap_or(0.0) as f32
    }
    /// `Player.getDestroySpeed`'s final `Attributes.BLOCK_BREAK_SPEED` factor:
    /// 1.0 by default, so a missing player or attribute changes nothing.
    pub(super) fn block_break_speed(&mut self) -> f32 {
        block_break_speed_factor(
            self.player
                .as_mut()
                .and_then(|p| p.attribute("minecraft:block_break_speed").ok()),
        )
    }
    /// `Attributes.SUBMERGED_MINING_SPEED` (0.2, raised by aqua affinity).
    pub(super) fn submerged_mining_speed(&self) -> f32 {
        let mut total = 0.0;
        for stack in self.armor.iter().flatten() {
            for (attr, _, amount, op) in self.stations().enchantments.attributes(stack) {
                if attr == "minecraft:submerged_mining_speed" && op == "add_multiplied_total" {
                    total += amount;
                }
            }
        }
        (0.2 * (1.0 + total)) as f32
    }
    /// The wear `ItemStack.hurtAndBreak` goes on to apply for `amount` points
    /// on the stack `wearer` wears or uses: what Unbreaking leaves
    /// (`EnchantmentHelper.processDurabilityChange`), then what the item's
    /// parts make of it (`IItemExtension.damageItem`).
    pub(super) fn durability_loss(
        &mut self,
        stack: &ItemStack,
        amount: u32,
        wearer: Wearer,
    ) -> u32 {
        let loss = self.unbreaking_loss(stack, amount);
        if loss == 0 || self.remote.is_some() {
            return loss;
        }
        // One table lookup tells whether any part of this item listens; with none,
        // the stack is not converted and no host is made.
        if !mod_api::__private::item_listens(mod_api::HookId::DamageItem, &stack.item) {
            return loss;
        }
        self.damage_item_loss(stack, loss, wearer)
    }
    /// `EnchantmentHelper.processDurabilityChange` (unbreaking) for `amount` points.
    fn unbreaking_loss(&mut self, stack: &ItemStack, amount: u32) -> u32 {
        let e = &world_stations!(self).enchantments;
        if !e.has_component(stack, "minecraft:item_damage") {
            return amount;
        }
        let ctx = EffectContext {
            tool: Some(&stack.item),
            items: Some(world_items!(self)),
            ..Default::default()
        };
        e.modify_value(
            Some(stack),
            "minecraft:item_damage",
            &ctx,
            &mut self.world.random,
            amount as f32,
        )
        .max(0.0) as u32
    }
    /// `ItemStack.hurtAndBreak` for the player's `stack`: the wear Unbreaking
    /// leaves, and `ItemDurabilityTrigger` when some is left (`applyDamage`).
    pub(super) fn wear(&mut self, stack: &ItemStack, amount: u32) -> u32 {
        // ItemStack.isDamageableItem: an unbreakable stack, or one without
        // damage/max_damage, takes no wear before Unbreaking or mods are asked.
        let component = |id| stack.data().and_then(|data| data.other(id));
        if component("minecraft:unbreakable").is_some()
            || component("!minecraft:max_damage").is_some()
            || component("!minecraft:damage").is_some()
        {
            return 0;
        }
        let loss = self.durability_loss(stack, amount, Wearer::Player);
        if loss > 0 {
            self.trigger_item_durability(stack, (stack.damage as i32).wrapping_add(loss as i32));
        }
        loss
    }

    /// `getEnchantedDamage - base`: damage bonus of the held weapon against a mob type.
    pub(super) fn enchant_damage_bonus(&mut self, target: &str, victim: Option<u32>) -> f32 {
        self.enchanted_damage(target, 0.0, victim)
    }
    /// `Player.getEnchantedDamage`: `base` run through the held weapon's
    /// damage effects against a mob type.
    pub(super) fn enchanted_damage(&mut self, target: &str, base: f32, victim: Option<u32>) -> f32 {
        let Some(held) = self.held().map(|stack| self.item_combat_stack(stack)) else {
            return base;
        };
        let actors = self
            .player
            .as_ref()
            .and_then(|player| victim.map(|victim| (player.entity_id as u32, victim)));
        self.weapon_enchanted_damage(
            &held,
            actors,
            target,
            &self.stack_melee_damage_type(
                Some(&held),
                self.player
                    .as_ref()
                    .map_or((0.0, false), |p| (p.fall_distance, p.fall_flying)),
                true,
            ),
            base,
            self.player
                .as_ref()
                .map(|p| (p.fall_distance, p.fall_flying)),
        )
    }

    /// Evaluate a pre-hit weapon snapshot with actual actor identities supplied by the hit.
    pub(super) fn weapon_enchanted_damage(
        &mut self,
        held: &ItemStack,
        actors: Option<(u32, u32)>,
        target: &str,
        damage_type: &str,
        base: f32,
        direct_motion: Option<(f64, bool)>,
    ) -> f32 {
        let ctx = EffectContext {
            this: Some(target),
            attacker: Some("minecraft:player"),
            direct_attacker: Some("minecraft:player"),
            is_direct: true,
            damage_type: Some(damage_type),
            direct_motion,
            items: None,
            ..Default::default()
        };
        self.evaluate_weapon_damage(held, actors, ctx, base)
    }

    pub(super) fn evaluate_weapon_damage(
        &mut self,
        held: &ItemStack,
        actors: Option<(u32, u32)>,
        ctx: EffectContext<'_>,
        base: f32,
    ) -> f32 {
        let ctx = EffectContext {
            items: Some(world_items!(self)),
            ..ctx
        };
        // Read effective levels once: get_enchantment_level may itself be mod-provided.
        let levels = held.all_enchantments();
        let custom = self.remote.is_none()
            && self.is_logical_server()
            && world_stations!(self)
                .enchantments
                .has_custom_damage(&levels);
        let actors = actors.filter(|_| custom).and_then(|(attacker, victim)| {
            let epoch = self.native_lifecycle.epoch()?;
            Some((
                mod_api::EntityRef::new(epoch, attacker),
                mod_api::EntityRef::new(epoch, victim),
            ))
        });
        let weapon = actors.and_then(|_| {
            use crate::game::api_values::TryIntoApi;
            held.try_into_api().ok()
        });
        let mut random = self.world.random.clone();
        let value = world_stations!(self).enchantments.modify_damage(
            &levels,
            &ctx,
            &mut random,
            base,
            &mut |slot, config, level, value| {
                let Some(((attacker, victim), weapon)) = actors.zip(weapon.as_ref()) else {
                    return value;
                };
                let Some(effect) = super::mods::ENCHANTMENT_DAMAGE_EFFECTS
                    .get()
                    .and_then(|effects| effects.get(slot))
                else {
                    return value;
                };
                effect.apply(
                    self,
                    &mod_api::EnchantmentDamageContext {
                        attacker: mod_api::LivingRef::new(attacker),
                        victim,
                        weapon,
                        level,
                        config,
                    },
                    value,
                )
            },
        );
        self.world.random = random;
        value
    }
    /// `Player.attack`'s extra knockback: `getKnockback` (the ATTACK_KNOCKBACK
    /// attribute, plus the weapon's knockback enchantment, halved) plus 0.5 for a
    /// sprinting full-strength hit, and whether the hit is that knockback attack.
    pub(super) fn extra_knockback(&mut self, target_kind: &str, scale: f32) -> (f32, bool) {
        let sprinting = self.player.as_ref().is_some_and(|p| p.sprinting);
        // LivingEntity.getKnockback: `(float)getAttributeValue(ATTACK_KNOCKBACK)`, which
        // the weapon's knockback enchantment then adds to (EnchantmentHelper.modifyKnockback).
        let base = self
            .player
            .as_mut()
            .and_then(|p| p.attribute("minecraft:attack_knockback").ok())
            .unwrap_or(0.0) as f32;
        let enchanted = match self.held().cloned() {
            Some(held) => {
                let ctx = EffectContext {
                    this: Some(target_kind),
                    attacker: Some("minecraft:player"),
                    direct_attacker: Some("minecraft:player"),
                    is_direct: true,
                    items: Some(world_items!(self)),
                    ..Default::default()
                };
                world_stations!(self).enchantments.modify_value(
                    Some(&held),
                    "minecraft:knockback",
                    &ctx,
                    &mut self.world.random,
                    base,
                )
            }
            None => base,
        };
        horizon_client::attack_strength::extra_knockback(enchanted, scale, sprinting)
    }
    /// `causeExtraKnockback`'s direction, `(sin yRot, -cos yRot)` from Mth's
    /// sine table: the target flies where the attacker looks, not away from
    /// it (straight along an axis when the attacker faces one).
    pub(super) fn extra_knockback_direction(&self) -> (f64, f64) {
        use horizon_core::geometry::trig;
        let yaw = f64::from(self.yaw * trig::DEG_TO_RAD);
        (f64::from(trig::sin(yaw)), -f64::from(trig::cos(yaw)))
    }
    /// The attacker's side of `causeExtraKnockback` as the attacking client
    /// sees it after a hit on a mob: the server's player stops sprinting, and
    /// its flag reaches the client. (The server's 0.6 speed cut never does:
    /// the client's own `Player.attack` simulates no hurt on a mob, so it
    /// skips the knockback. A hit on another player does simulate one:
    /// `remote_player_attack`.)
    pub(super) fn stop_sprint_after_extra_knockback(&mut self) {
        if let Some(p) = self.player.as_mut() {
            let _ = p.set_sprinting(false);
        }
    }
    /// `EnchantmentHelper.doPostAttackEffectsWithItemSource` after a successful
    /// melee hit (`Player.itemAttackInteraction`, the sweep's
    /// `doPostAttackEffects`): the victim's equipment first (Thorns), then the
    /// attacker's weapon on the victim (fire aspect and wind burst).
    pub(super) fn post_attack(&mut self, mob: usize) {
        if let Some(id) = self.mobs.get(mob).map(|m| m.id) {
            self.mob_post_attack(id, Some(Who::Player), Some("minecraft:player"), None);
        }
        let Some(held) = self.held().cloned() else {
            return;
        };
        let Some(m) = self.mobs.get(mob) else { return };
        let kind = m.kind.id.clone();
        let victim = m.id;
        let ctx = EffectContext {
            this: Some(&kind),
            attacker: Some("minecraft:player"),
            direct_attacker: Some("minecraft:player"),
            is_direct: true,
            items: Some(world_items!(self)),
            ..Default::default()
        };
        // minecraft:post_attack effects of the attacker's weapon on the victim:
        // ignite (fire aspect) and apply_mob_effect (bane of arthropods).
        let mut ignite = 0.0f32;
        let mut mob_effects = Vec::new();
        let mut hook_calls: Vec<(String, String, std::sync::Arc<horizon_core::nbt::Tag>, u32)> =
            Vec::new();
        for (id, level) in held.all_enchantments().iter() {
            let Some(def) = world_stations!(self).enchantments.get(id) else {
                continue;
            };
            for (effect_index, effect) in def.effects["minecraft:post_attack"]
                .as_array()
                .into_iter()
                .flatten()
                .enumerate()
            {
                let e = &effect["effect"];
                if effect["enchanted"] != "attacker" || effect["affected"] != "victim" {
                    continue;
                }
                // A bare type name is minecraft's own, as a bare item id is.
                let kind_of = match e["type"].as_str() {
                    Some(k) if k.contains(':') => k.to_owned(),
                    Some(k) => format!("minecraft:{k}"),
                    None => continue,
                };
                let own = !kind_of.starts_with("minecraft:");
                if !own && !matches!(&*kind_of, "minecraft:ignite" | "minecraft:apply_mob_effect") {
                    continue;
                }
                let mut random = self.world.random.clone();
                let ok = effect.get("requirements").is_none_or(|r| {
                    self.res
                        .stations()
                        .enchantments
                        .condition(r, *level, &ctx, &mut random)
                });
                self.world.random = random;
                if !ok {
                    continue;
                }
                // A mod's own kind: its hook answers, after this loop (the
                // stations borrow is held here).
                if own {
                    if let Some(data) = def.native_post_attack.get(&effect_index) {
                        hook_calls.push((id.to_string(), kind_of, data.clone(), *level));
                    }
                } else if kind_of == "minecraft:ignite" {
                    ignite =
                        ignite.max(horizon_client::enchant::level_value(&e["duration"], *level));
                } else {
                    mob_effects.push((e.clone(), *level));
                }
            }
        }
        if ignite > 0.0
            && let Some(m) = self.mobs.get_mut(mob)
            && !m.kind.flags.fire_immune
        {
            m.ignite_for_ticks((ignite * 20.0) as i32);
        }
        for (effect, level) in mob_effects {
            self.apply_mob_effect(mob, &effect, level);
        }
        for (id, kind_of, data, level) in hook_calls {
            self.native_enchant_effect(&id, &kind_of, &data, level, victim);
        }
        self.post_attack_bursts(&held, &kind);
    }
    /// `EnchantmentHelper.doPostPiercingAttackEffects`: the
    /// `minecraft:post_piercing_attack` effects of the weapon in hand after a
    /// jab (Lunge's dash).
    pub(super) fn post_piercing_attack(&mut self) {
        let Some(held) = self.held().cloned() else {
            return;
        };
        let mut effects = Vec::new();
        for (id, level) in held.all_enchantments().iter() {
            let Some(def) = self.stations().enchantments.get(id) else {
                continue;
            };
            for entry in def.effects["minecraft:post_piercing_attack"]
                .as_array()
                .into_iter()
                .flatten()
            {
                effects.push((*level, entry["effect"].clone()));
            }
        }
        // Lunge's requirements: not riding, gliding or in water, and a
        // player outside creative with at least 7 food.
        let Some(p) = self.player.as_ref() else {
            return;
        };
        if effects.is_empty()
            || self.riding.is_some()
            || p.fall_flying
            || p.fluids.was_touching_water
            || !(self.creative || p.health.food.food >= 7)
        {
            return;
        }
        for (level, effect) in effects {
            self.apply_piercing_effect(&effect, level);
        }
    }
    /// One `post_piercing_attack` effect on the player: `AllOf`,
    /// `ChangeItemDamage`, `ApplyExhaustion`, `ApplyEntityImpulse` and
    /// `PlaySoundEffect`.
    fn apply_piercing_effect(&mut self, effect: &serde_json::Value, level: u32) {
        let amount = || level_value(&effect["amount"], level);
        match effect["type"].as_str() {
            Some("minecraft:all_of") => {
                for e in effect["effects"].as_array().into_iter().flatten() {
                    self.apply_piercing_effect(e, level);
                }
            }
            Some("minecraft:change_item_damage") => self.damage_held(amount() as u32),
            Some("minecraft:apply_exhaustion") => {
                if let Some(p) = self.player.as_mut() {
                    p.cause_food_exhaustion(amount());
                }
            }
            // The direction turned by the look (`Entity.getLookQuaternion`:
            // pitch about x, then yaw about y), scaled by coordinate_scale
            // and the magnitude, added to the velocity (the guest's client
            // is told of it by `guest_hit_by`).
            Some("minecraft:apply_impulse") => {
                let impulse = look_impulse(effect, level, self.yaw, self.pitch);
                if let Some(p) = self.player.as_mut() {
                    p.velocity = p.velocity.add_vector(impulse);
                }
            }
            // The sound for the level (a list picks by level), heard by all.
            Some("minecraft:play_sound") => {
                let sounds: Vec<&str> = match &effect["sound"] {
                    serde_json::Value::Array(list) => {
                        list.iter().filter_map(|s| s.as_str()).collect()
                    }
                    one => one.as_str().into_iter().collect(),
                };
                let index = (level as usize).saturating_sub(1);
                if let Some(sound) = sounds.get(index.min(sounds.len().saturating_sub(1))) {
                    let float = |key: &str| effect[key].as_f64().unwrap_or(1.0) as f32;
                    let at = self
                        .player
                        .as_ref()
                        .map(|p| [p.position.x, p.position.y, p.position.z]);
                    self.audio.play(sound, at, float("volume"), float("pitch"));
                }
            }
            _ => {}
        }
    }
    /// `ApplyMobEffect.apply`: one of `to_apply` (a single id or a list) for a
    /// random duration and amplifier between the level-based bounds.
    fn apply_mob_effect(&mut self, mob: usize, effect: &serde_json::Value, level: u32) {
        use horizon_client::enchant::level_value;
        let Some(m) = self.mobs.get_mut(mob) else {
            return;
        };
        let ids: Vec<&str> = match &effect["to_apply"] {
            serde_json::Value::String(id) => vec![id.as_str()],
            serde_json::Value::Array(list) => list.iter().filter_map(|v| v.as_str()).collect(),
            _ => Vec::new(),
        };
        if ids.is_empty() {
            return;
        }
        // HolderSet.getRandomElement, then Mth.randomBetween for both values.
        let pick = m.random.next_int_bound(ids.len() as i32).unwrap_or(0) as usize;
        let mut between = |min: &str, max: &str| {
            let (lo, hi) = (
                level_value(&effect[min], level),
                level_value(&effect[max], level),
            );
            m.random.next_float() * (hi - lo) + lo
        };
        let ticks = (between("min_duration", "max_duration") * 20.0).round() as i32;
        let amplifier = (between("min_amplifier", "max_amplifier").round() as i32).max(0);
        if let Some(id) = super::potions::mob_effect_id(ids[pick]) {
            let (new, entity) = (!m.has_effect(id), m.id as i32);
            m.add_effect(id, ticks, amplifier);
            self.mod_effect_added(id, Some(entity), amplifier, new);
        }
    }
    /// `minecraft:post_attack` explode effects on the attacker (wind burst):
    /// a burst at the player's feet that only knocks back and triggers
    /// blocks (`ExplodeEffect` with `block_interaction: trigger`).
    pub(super) fn post_attack_bursts(&mut self, held: &ItemStack, victim: &str) {
        let Some(p) = self.player.as_ref() else {
            return;
        };
        // EntityFlagsPredicate.isFlying: gliding or flying.
        let motion = (p.fall_distance, p.fall_flying || p.flying);
        let feet = [p.position.x, p.position.y, p.position.z];
        let ctx = EffectContext {
            this: Some(victim),
            attacker: Some("minecraft:player"),
            direct_attacker: Some("minecraft:player"),
            is_direct: true,
            items: Some(world_items!(self)),
            direct_motion: Some(motion),
            ..Default::default()
        };
        let mut bursts = Vec::new();
        for (id, level) in held.all_enchantments().iter() {
            let Some(def) = world_stations!(self).enchantments.get(id) else {
                continue;
            };
            for effect in def.effects["minecraft:post_attack"]
                .as_array()
                .into_iter()
                .flatten()
            {
                let e = &effect["effect"];
                if effect["enchanted"] != "attacker"
                    || effect["affected"] != "attacker"
                    || e["type"] != "minecraft:explode"
                    || e["block_interaction"] != "trigger"
                {
                    continue;
                }
                let ok = effect.get("requirements").is_none_or(|r| {
                    world_stations!(self).enchantments.condition(
                        r,
                        *level,
                        &ctx,
                        &mut self.world.random,
                    )
                });
                if !ok {
                    continue;
                }
                let radius = horizon_client::enchant::level_value(&e["radius"], *level).max(0.0);
                // ExplosionDamageCalculator.getKnockbackMultiplier: 1 unless given.
                let knockback = if e["knockback_multiplier"].is_null() {
                    1.0
                } else {
                    horizon_client::enchant::level_value(&e["knockback_multiplier"], *level)
                };
                // ExplodeEffect.attribute_to_user: the wielder is the source.
                bursts.push((radius, knockback, e["attribute_to_user"] == true));
            }
        }
        let wielder = self.own_entity_id() as u32;
        for (radius, knockback, attributed) in bursts {
            let cause = super::wind::BurstCause {
                source: attributed.then_some(wielder),
                owner: attributed.then_some(wielder),
            };
            let _ = self.wind_burst(
                feet,
                radius,
                knockback,
                super::wind::Source::WIND_BURST.with_cause(cause),
            );
        }
    }
    /// `EnchantmentHelper.doPostAttackEffects`' `VICTIM` loop after the mob
    /// `attacker` hurt the player: Thorns on what the player holds or wears
    /// hurts the mob by `thorns` damage the player caused, so it is knocked
    /// back from the player, turns on them and is theirs to kill.
    pub(super) fn thorns(&mut self, attacker: u32) {
        self.player_thorns(|g, min, max| {
            // DamageEntity: Mth.randomBetween from the hurt entity's own random.
            let Some(index) = g.mob_index.find(&g.mobs, attacker) else {
                g.dragon_thorns(attacker, min, max);
                return;
            };
            let damage = g.mobs[index].random.next_float() * (max - min) + min;
            let Some(p) = g.player.as_ref() else { return };
            let source = Damage {
                kind: DamageKind::Thorns,
                attacker: Some(Who::Player),
                from: Some([p.position.x, p.position.y, p.position.z]),
                attacker_species: None,
            };
            let alive = g.mobs[index].alive();
            if g.hurt_by_player(index, damage, source) {
                if alive && !g.mobs[index].alive() {
                    g.native_kill_event(index);
                }
                g.remember_mob_player_credit(attacker);
            }
        });
    }
    /// The victim's side of thorns when another player hit this one: the
    /// damage each piece deals that player, rolled from the attacker's own
    /// random (a player's, which no level draw shares).
    pub(super) fn thorns_damage(&mut self) -> Vec<f32> {
        let mut hits = Vec::new();
        self.player_thorns(|_, min, max| {
            hits.push(random_source::entity().next_float() * (max - min) + min);
        });
        hits
    }
    /// `EnchantmentHelper.runIterationOnEquipment` over the player's hands and
    /// armor, in `EquipmentSlot` order, running `Enchantment.doPostAttack` for
    /// the `minecraft:post_attack` effects with `enchanted: victim` and
    /// `affected: attacker` (Thorns): the chance from the level's random
    /// (`Enchantment.damageContext`), then each `damage_entity` handed to `hit` (min,
    /// max) and each `change_item_damage` worn off the piece, in the effect's
    /// order. The attacker being dead stops neither the rolls nor the wear.
    fn player_thorns(&mut self, mut hit: impl FnMut(&mut Self, f32, f32)) {
        for slot in PLAYER_SLOTS {
            let Some(stack) = self.player_slot(slot).clone() else {
                continue;
            };
            let mut effects = Vec::new();
            for (enchantment, level) in stack.enchantments() {
                let Some(def) = world_stations!(self).enchantments.get(enchantment) else {
                    continue;
                };
                // Enchantment.matchingSlot
                let (name, hand) = (
                    slot.name(),
                    matches!(slot, PlayerSlot::Main | PlayerSlot::Off),
                );
                if !def.slots.iter().any(|s| {
                    s == name || s == "any" || (s == "armor" && !hand) || (s == "hand" && hand)
                }) {
                    continue;
                }
                for effect in def.effects["minecraft:post_attack"]
                    .as_array()
                    .into_iter()
                    .flatten()
                {
                    if effect["enchanted"] == "victim" && effect["affected"] == "attacker" {
                        effects.push((effect.clone(), *level));
                    }
                }
            }
            for (effect, level) in effects {
                let ctx = EffectContext {
                    this: Some("minecraft:player"),
                    items: Some(world_items!(self)),
                    ..Default::default()
                };
                let ok = effect.get("requirements").is_none_or(|r| {
                    world_stations!(self).enchantments.condition(
                        r,
                        level,
                        &ctx,
                        &mut self.world.random,
                    )
                });
                if ok {
                    self.thorns_effect(&effect["effect"], level, slot, &mut hit);
                }
            }
        }
    }
    /// `AllOf.apply`, `DamageEntity.apply` and `ChangeItemDamage.apply` of one
    /// Thorns-like effect of the piece in `slot`.
    fn thorns_effect(
        &mut self,
        effect: &Value,
        level: u32,
        slot: PlayerSlot,
        hit: &mut impl FnMut(&mut Self, f32, f32),
    ) {
        match effect["type"]
            .as_str()
            .map(|t| t.trim_start_matches("minecraft:"))
        {
            Some("all_of") => {
                for e in effect["effects"].as_array().into_iter().flatten() {
                    self.thorns_effect(e, level, slot, hit);
                }
            }
            Some("damage_entity") => {
                let min = level_value(&effect["min_damage"], level);
                let max = level_value(&effect["max_damage"], level);
                hit(self, min, max);
            }
            Some("change_item_damage") => {
                let amount = level_value(&effect["amount"], level) as u32;
                self.wear_player_piece(slot, amount);
            }
            _ => {}
        }
    }
    /// `ItemStack.hurtAndBreak` for what the player has in `slot`: nothing for
    /// an item that takes no wear or a creative player, Unbreaking's share
    /// from the level's random, and a piece worn out leaves the slot.
    fn wear_player_piece(&mut self, slot: PlayerSlot, amount: u32) {
        self.damage_player_equipment(slot.equipment_slot(), amount.min(i32::MAX as u32) as i32);
    }
    /// `EnchantmentHelper.doPostAttackEffectsWithItemSource`'s victim loop:
    /// each equipment slot's chance, damage from the attacker's random, then wear.
    pub(super) fn mob_post_attack(
        &mut self,
        target: u32,
        attacker: Option<Who>,
        direct: Option<&str>,
        damage_type: Option<&str>,
    ) {
        let attacker_kind = match attacker {
            Some(Who::Player) => self.player.as_ref().map(|_| "minecraft:player".to_owned()),
            Some(Who::Mob(id)) => self
                .mobs
                .iter()
                .find(|m| m.id == id && !m.removed)
                .map(|m| m.kind.id.clone())
                .or_else(|| {
                    self.dragon
                        .as_ref()
                        .filter(|d| d.id == id)
                        .map(|_| "minecraft:ender_dragon".to_owned())
                }),
            None => None,
        };
        let attacker = attacker.filter(|_| attacker_kind.is_some());
        let direct = direct.or(attacker_kind.as_deref());
        use horizon_client::mobs::finalize as f;
        let Some(m) = self.mobs.iter().find(|m| m.id == target) else {
            return;
        };
        let (id, kind) = (m.id, m.kind.id.clone());
        // EquipmentSlot.VALUES: the hands, then the feet up to the head (the
        // body and the saddle keep no enchantments here).
        for slot in [f::MAINHAND, f::OFFHAND, f::FEET, f::LEGS, f::CHEST, f::HEAD] {
            let Some(stack) = self
                .mobs
                .iter()
                .find(|m| m.id == id)
                .and_then(|m| f::item_stack(m, slot))
            else {
                continue;
            };
            let name = MOB_SLOTS[slot];
            let hand = slot == f::MAINHAND || slot == f::OFFHAND;
            let mut effects = Vec::new();
            for (enchantment, level) in stack.enchantments() {
                let Some(def) = world_stations!(self).enchantments.get(enchantment) else {
                    continue;
                };
                // Enchantment.matchingSlot
                if !def.slots.iter().any(|s| {
                    s == name || s == "any" || (s == "armor" && !hand) || (s == "hand" && hand)
                }) {
                    continue;
                }
                for effect in def.effects["minecraft:post_attack"]
                    .as_array()
                    .into_iter()
                    .flatten()
                {
                    if effect["enchanted"] == "victim" {
                        effects.push((effect.clone(), *level));
                    }
                }
            }
            for (effect, level) in effects {
                // Enchantment.damageContext: random_chance draws from the level's random.
                let ctx = EffectContext {
                    this: Some(&kind),
                    attacker: attacker_kind.as_deref(),
                    direct_attacker: direct,
                    is_direct: direct.is_some() && direct == attacker_kind.as_deref(),
                    damage_type,
                    items: Some(world_items!(self)),
                    ..Default::default()
                };
                let ok = effect.get("requirements").is_none_or(|r| {
                    world_stations!(self).enchantments.condition(
                        r,
                        level,
                        &ctx,
                        &mut self.world.random,
                    )
                });
                if ok
                    && effect["affected"] == "attacker"
                    && let Some(attacker) = attacker
                {
                    self.victim_effect(&effect["effect"], level, id, slot, attacker);
                }
            }
        }
    }
    /// `DamageEntity.apply`, `ChangeItemDamage.apply` (on the piece) and
    /// `AllOf` for one `post_attack` effect of the piece the mob `wearer` has
    /// in `slot`, on the entity that hit it.
    fn victim_effect(
        &mut self,
        effect: &Value,
        level: u32,
        wearer: u32,
        slot: usize,
        attacker: Who,
    ) {
        match effect["type"]
            .as_str()
            .map(|t| t.trim_start_matches("minecraft:"))
        {
            Some("all_of") => {
                for e in effect["effects"].as_array().into_iter().flatten() {
                    self.victim_effect(e, level, wearer, slot, attacker);
                }
            }
            // DamageEntity.apply: the affected attacker's own random, then
            // hurtServer by a source the wearer caused, from its feet.
            Some("damage_entity") => {
                let min = level_value(&effect["min_damage"], level);
                let max = level_value(&effect["max_damage"], level);
                let random = match attacker {
                    Who::Player => random_source::entity().next_float(),
                    Who::Mob(id) => match self.mob_index.find(&self.mobs, id) {
                        Some(index) => self.mobs[index].random.next_float(),
                        None => match self.dragon_effect_random(id) {
                            Some(random) => random,
                            None => return,
                        },
                    },
                };
                let damage = random * (max - min) + min;
                let Some(m) = self.mobs.iter().find(|m| m.id == wearer) else {
                    return;
                };
                let from = [m.position.x, m.position.y, m.position.z];
                let kind = effect["damage_type"].as_str().unwrap_or("minecraft:thorns");
                match attacker {
                    Who::Player => {
                        self.mob_hits_player_caused(damage, from, kind, Some(wearer));
                    }
                    Who::Mob(id) => {
                        let source = horizon_client::mobs::Damage {
                            kind: Self::damage_kind(&self.health_resources.damage, kind),
                            attacker: Some(Who::Mob(wearer)),
                            from: Some(from),
                            attacker_species: Some(m.species()),
                        };
                        self.mob_hits_mob(id, damage, source);
                    }
                }
            }
            Some("change_item_damage") => {
                let amount = level_value(&effect["amount"], level) as u32;
                self.wear_mob_piece(wearer, slot, amount);
            }
            _ => {}
        }
    }
    /// `ItemStack.hurtAndBreak` with no player for what the mob `id` has in
    /// `slot`: Unbreaking takes its share from the level's random, and a
    /// piece worn out leaves the slot with its break event
    /// (`LivingEntity.onEquippedItemBroken`).
    fn wear_mob_piece(&mut self, id: u32, slot: usize, amount: u32) {
        use horizon_client::mobs::finalize as f;
        let Some(m) = self.mobs.iter().find(|m| m.id == id) else {
            return;
        };
        let at = [m.position.x, m.position.y, m.position.z];
        let Some(stack) = f::item_stack(m, slot).filter(|s| {
            amount > 0 && horizon_client::stations::is_damageable(world_items!(self), s)
        }) else {
            return;
        };
        let loss = self.durability_loss(&stack, amount, Wearer::Living(id));
        if loss == 0 {
            return;
        }
        let damage = stack.damage + loss;
        let max = world_items!(self)
            .get(&stack.item)
            .map_or(0, |i| i.max_damage);
        let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) else {
            return;
        };
        if damage < max {
            f::set_slot_damage(m, slot, damage);
            return;
        }
        horizon_client::mobs::drops::clear_slot(m, MOB_SLOTS[slot]);
        // LivingEntity.entityEventForEquipmentBreak
        let event = [47, 48, 52, 51, 50, 49][slot];
        self.lan_entity_event(id as i32, event, at);
    }
    /// `ExperienceOrb.repairPlayerItems`: returns the experience left over.
    pub(in crate::game) fn mending(&mut self, amount: u32) -> u32 {
        let mut amount = amount;
        let player = (self.serving.is_some() || self.options.dedicated.is_some())
            .then(|| self.native_acting_id());
        loop {
            // EnchantmentHelper.getRandomItemWith(REPAIR_WITH_XP, damaged), in
            // EquipmentSlot order, drawn from the player's own random.
            let slots = PLAYER_SLOTS.map(|slot| (slot, self.player_slot(slot)));
            let Some(slot) = pick_mending_slot(
                slots,
                &world_stations!(self).enchantments,
                &mut random_source::entity(),
            ) else {
                return amount;
            };
            let Some(mut stack) = self.player_slot(slot).clone() else {
                return amount;
            };
            let vanilla_repair = repair_from_xp(
                &world_stations!(self).enchantments,
                Some(world_items!(self)),
                &stack,
                amount,
                &mut self.world.random,
            );
            let to_repair_xp = self.xp_repair(player, &stack, amount, vanilla_repair);
            // No re-check that the slot still holds the picked stack, eligible
            // and damaged: nothing between the pick and here can change it. The
            // `xp_repair_ratio` parts are asked through `&self` with a read-only
            // host, and the `repair_with_xp` value only draws from the world's
            // random. The repair is written back as the picked stack. A path that
            // could change the slot in this window must bring the re-check back.
            if to_repair_xp == 0 {
                // Vanilla consumes the XP when its own repair effect returns zero.
                return if vanilla_repair == 0 { 0 } else { amount };
            }
            let (repaired, remaining) = repaired_and_left(amount, stack.damage, to_repair_xp);
            if repaired == 0 {
                return amount;
            }
            stack.damage -= repaired;
            let item = stack.item.to_string();
            *self.player_slot_mut(slot) = Some(stack);

            if let Some(player) = player
                && let Some(name) = self
                    .native_server_players()
                    .into_iter()
                    .find(|p| p.uuid() == player)
                    .and_then(|p| self.player_name(p))
            {
                self.native_player_item_mend(player, &name, &item, repaired, amount);
            }
            if remaining == 0 {
                return 0;
            }
            amount = remaining;
        }
    }
    /// The repair Mending gives `stack` for `experience` points: `vanilla`
    /// (its `minecraft:repair_with_xp` repair) as the item's parts scale it
    /// (`IItemExtension.getXpRepairRatio`), `floor(experience × ratio)` within
    /// `0..=i32::MAX`. A ratio left as it arrived keeps `vanilla` exactly, and
    /// one that isn't finite is refused for it. `player` is the acting player,
    /// `None` when no server runs (no part is asked then).
    fn xp_repair(
        &self,
        player: Option<u128>,
        stack: &ItemStack,
        experience: u32,
        vanilla: u32,
    ) -> u32 {
        let Some(player) = player else {
            return vanilla;
        };
        // One table lookup tells whether any part of this item listens; with none,
        // the stack is not converted and no host is made.
        if experience == 0
            || !mod_api::__private::item_listens(mod_api::HookId::XpRepairRatio, &stack.item)
        {
            return vanilla;
        }
        let arrived = vanilla as f32 / experience as f32;
        let mut ratio = arrived;
        self.native_xp_repair_ratio(player, stack, experience, &mut ratio);
        if ratio.to_bits() == arrived.to_bits() {
            return vanilla;
        }
        if !ratio.is_finite() {
            refuse_xp_repair_ratio(&stack.item, ratio);
            return vanilla;
        }
        // `as` saturates: a negative product repairs nothing.
        (f64::from(experience) * f64::from(ratio))
            .floor()
            .min(f64::from(i32::MAX)) as u32
    }
    /// `LivingEntity.getItemBySlot` for the player.
    fn player_slot(&self, slot: PlayerSlot) -> &Option<ItemStack> {
        match slot {
            PlayerSlot::Main => &self.inventory[self.selected],
            PlayerSlot::Off => &self.offhand,
            PlayerSlot::Armor(i) => &self.armor[i],
        }
    }
    fn player_slot_mut(&mut self, slot: PlayerSlot) -> &mut Option<ItemStack> {
        match slot {
            PlayerSlot::Main => &mut self.inventory[self.selected],
            PlayerSlot::Off => &mut self.offhand,
            PlayerSlot::Armor(i) => &mut self.armor[i],
        }
    }
    /// Bow enchantments at release: (extra base damage, infinity keeps the arrow).
    /// `ammo` is the item being fired: `Enchantment.itemContext` hands it to the
    /// `ammo_use` conditions as the tool, so Infinity only frees plain arrows.
    pub(super) fn bow_enchantments(&mut self, ammo: &str) -> (f64, bool) {
        let Some(bow) = self.held().cloned() else {
            return (0.0, false);
        };
        let e = &world_stations!(self).enchantments;
        // Damage is evaluated at impact, with the real victim (AbstractArrow.onHitEntity).
        let power = 0.0;
        let ammo_ctx = EffectContext {
            tool: Some(ammo),
            items: Some(world_items!(self)),
            ..Default::default()
        };
        let infinity = e.has_component(&bow, "minecraft:ammo_use")
            && e.modify_value(
                Some(&bow),
                "minecraft:ammo_use",
                &ammo_ctx,
                &mut self.world.random,
                1.0,
            ) == 0.0;
        (power, infinity)
    }
    /// Soul Speed (`data/minecraft/enchantment/soul_speed.json`), the parts of
    /// `Enchantment.runLocationChangedEffects` and `tick` that file uses. The
    /// `location_changed` effects run when the player enters a new block, lands,
    /// or its boots change:
    /// - the speed and movement efficiency modifiers turn on over a soul speed
    ///   block (not flying, not riding) and stay on while airborne, then off
    ///   (`enchantment_active_check`);
    /// - the boots lose 1 durability with a 4% chance when on such a block.
    ///
    /// Every fifth tick the player walking on one also gets a soul particle
    /// (returned as its position and velocity, for the caller to show or send
    /// to the players around) and sometimes a sound. Runs for the window's own
    /// player and for each guest on the server (`guest_server_tick`); a remote
    /// client leaves it to its server.
    pub(super) fn tick_soul_speed(&mut self) -> Option<([f64; 3], [f64; 3])> {
        const ID: &str = "minecraft:soul_speed";
        let player = self.player.as_mut()?;
        if player.client_side {
            return None;
        }
        let level = self.armor[3]
            .as_ref()
            .map_or(0, |s| s.enchantment_level(ID));
        let position = player.position;
        let (on_ground, flying) = (player.ground.on_ground, player.flying);
        let moved = position.subtract(player.old_position);
        let (tick, width) = (player.health.tick_count, player.dimensions.width);
        let fx = &mut player.location_effects;
        let on_soul = fx.on_soul_speed_block;
        let block = BlockPos::containing(position);
        let changed_block = fx.block.replace(block) != Some(block);
        let landed = std::mem::take(&mut fx.landed);
        // Changed boots: `stopLocationBasedEffects`, then the effects run again.
        let equipped = std::mem::replace(&mut fx.boots_level, level) != level;
        let was_active = fx.applied_level;
        let mut active = was_active > 0 && !equipped;
        let mut wear = false;
        if level > 0 && (changed_block || landed || equipped) {
            let riding = self.riding.is_some();
            let wanted = !riding && !flying && (on_soul || (active && !on_ground));
            active = wanted;
            // `random_chance` is the first term of the wear's `all_of`: it
            // draws from the level random before the block is looked at.
            let roll = self.world.random.next_float() < 0.04;
            wear = roll && on_ground && on_soul;
        }
        fx.applied_level = if active { level } else { 0 };
        // `Entity.getKnownMovement` horizontal speed of at least 1.0E-5.
        let walking = moved.x * moved.x + moved.z * moved.z >= 1.0e-10;
        let tick_effects = level > 0 && !flying && on_ground && walking && on_soul && tick % 5 == 0;
        // The attribute effects were applied to the player before: swap them.
        if was_active > 0 && (!active || equipped) {
            for (attribute, id, _, op) in
                soul_speed_modifiers(&world_stations!(self).enchantments, was_active)
            {
                let _ = player.set_modifier(&attribute, &id, 0.0, &op, false);
            }
        }
        if active && (was_active == 0 || equipped) {
            for (attribute, id, amount, op) in
                soul_speed_modifiers(&world_stations!(self).enchantments, level)
            {
                let _ = player.set_modifier(&attribute, &id, amount, &op, true);
            }
        }
        if wear && !self.creative {
            self.wear_boots(1);
        }
        if level == 0 {
            return None;
        }
        let (particle, pitch) = soul_speed_tick(
            [position.x, position.y, position.z],
            [moved.x, moved.y, moved.z],
            width,
            tick_effects,
            &mut self.world.random,
            &mut random_source::entity(),
        );
        if let Some(pitch) = pitch {
            self.audio.play(
                "minecraft:particle.soul_escape",
                Some([position.x, position.y, position.z]),
                0.6,
                pitch,
            );
        }
        particle
    }
    /// `ItemStack.hurtAndBreak` on the worn boots (`change_item_damage`):
    /// Unbreaking applies, and they break when worn out.
    fn wear_boots(&mut self, amount: u32) {
        self.damage_player_equipment(
            mod_api::EquipmentSlot::Feet,
            amount.min(i32::MAX as u32) as i32,
        );
    }
    /// `EnchantmentHelper.runLocationChangedEffects` for the worn armor of the
    /// player, run on the server once the player has moved: `LivingEntity.baseTick`
    /// runs it when the block position changed, `checkFallDamage` when it lands.
    /// `from` and `was_on_ground` are where and how the tick began.
    pub(super) fn location_changed_effects(&mut self, from: [f64; 3], was_on_ground: bool) {
        let Some(p) = self.player.as_ref() else {
            return;
        };
        let at = [p.position.x, p.position.y, p.position.z];
        let on_ground = p.ground.on_ground;
        let cell = |v: [f64; 3]| v.map(|c| c.floor() as i32);
        // LivingEntity.collectEquipmentChanges: newly worn armor runs its
        // location_changed effects at once, even when standing still.
        let changed = armor_changes(&mut self.location_armor, &self.armor);
        let moved = cell(from) != cell(at) || (on_ground && !was_on_ground);
        if !moved && !changed.iter().any(|c| *c) {
            return;
        }
        let riding = self.riding.is_some();
        let slots = ["head", "chest", "legs", "feet"];
        let mut disks = Vec::new();
        for i in 0..4 {
            if moved || changed[i] {
                disks.extend(location_disks(
                    &self.stations().enchantments,
                    std::iter::once((&self.armor[i], slots[i])),
                    on_ground,
                    riding,
                ));
            }
        }
        for (effect, level) in disks {
            self.replace_disk(&effect, level, at, Who::Player);
        }
    }
    /// `LivingEntity.onChangedBlock`, landing and `collectEquipmentChanges`
    /// for an armor stand: run location effects before travel (including a
    /// piston push), on landing, and for newly equipped items.
    pub(super) fn armor_stand_location_effects(&mut self, index: usize, landed: bool) {
        // Nothing to tell: read without a `&mut`, which the list would count as a touch.
        {
            let stand = &self.armor_stands[index];
            if !landed
                && stand.enchantment_block == Some(BlockPos::containing(stand.position))
                && (0..8).all(|i| stand.equipment[i] == stand.enchantment_equipment[i])
            {
                return;
            }
        }
        let stand = &mut self.armor_stands[index];
        let block = BlockPos::containing(stand.position);
        let moved = !landed && stand.enchantment_block.replace(block) != Some(block);
        // `collectEquipmentChanges` walks every `EquipmentSlot`, body and
        // saddle included; only the six slots below wear enchantments.
        let changed: [bool; 8] =
            std::array::from_fn(|i| stand.equipment[i] != stand.enchantment_equipment[i]);
        if !moved && !landed && !changed.iter().any(|c| *c) {
            return;
        }
        let id = stand.id;
        let told: Vec<_> = (0..changed.len())
            .filter(|&i| changed[i])
            .map(|i| {
                let slot = horizon_items::equipment::SLOT_NAMES[i];
                (
                    slot,
                    stand.enchantment_equipment[i].clone(),
                    stand.equipment[i].clone(),
                )
            })
            .collect();
        stand.enchantment_equipment.clone_from(&stand.equipment);
        if self.remote.is_none() && !self.options.execution_role.is_client() {
            for (slot, from, to) in told {
                self.native_living_equipment_change(
                    id,
                    "minecraft:armor_stand",
                    slot,
                    from.as_ref(),
                    to.as_ref(),
                );
            }
        }
        let stand = &self.armor_stands[index];
        let at = [stand.position.x, stand.position.y, stand.position.z];
        let who = Who::Mob(stand.id);
        let equipment = stand
            .equipment
            .iter()
            .zip(horizon_client::entities::ArmorStand::SLOTS)
            .enumerate()
            .filter(|(i, _)| moved || landed || changed[*i])
            .filter(|(_, (stack, _))| {
                moved
                    || landed
                    || !stack.as_ref().is_some_and(|stack| {
                        let data = stack.data();
                        let component = |id| data.and_then(|data| data.other(id));
                        let max_damage = match component("minecraft:max_damage") {
                            Some(horizon_core::nbt::Tag::Int(value)) => *value as u32,
                            _ => self
                                .res
                                .items()
                                .get(&stack.item)
                                .map_or(0, |i| i.max_damage),
                        };
                        max_damage > 0
                            && component("minecraft:unbreakable").is_none()
                            && component("!minecraft:max_damage").is_none()
                            && component("!minecraft:damage").is_none()
                            && stack.damage >= max_damage
                    })
            })
            .map(|(_, item)| item);
        let disks = location_disks(
            &self.stations().enchantments,
            equipment,
            stand.on_ground,
            stand.vehicle.is_some(),
        );
        for (effect, level) in disks {
            self.replace_disk(&effect, level, at, who);
        }
    }
    /// `ReplaceDisk.apply`: the blocks of the disk around `at` that pass the
    /// predicate become the effect's state, and each placement is a game event.
    fn replace_disk(&mut self, effect: &Value, level: u32, at: [f64; 3], who: Who) {
        let registry = self.pack.assets.registry;
        let spec = &effect["block_state"];
        let Some(default) = spec["id"].as_str().and_then(|n| registry.default_state(n)) else {
            return;
        };
        let Some(mut state) = registry.state(default).cloned() else {
            return;
        };
        for (k, v) in spec["properties"].as_object().into_iter().flatten() {
            state
                .properties
                .insert(k.clone(), v.as_str().unwrap_or("").to_owned());
        }
        let new = registry.resolve(&state).unwrap_or(default);
        let radius = level_value(&effect["radius"], level) as i32;
        let height = level_value(&effect["height"], level) as i32;
        let offset = cell_offset(effect);
        let center = [0, 1, 2].map(|i| at[i].floor() as i32 + offset[i]);
        let event = effect["trigger_game_event"].as_str() == Some("minecraft:block_place");
        for dy in (height - 1).min(0)..=0 {
            for dz in -radius..=radius {
                for dx in -radius..=radius {
                    let pos = [center[0] + dx, center[1] + dy, center[2] + dz];
                    let (x, z) = (
                        f64::from(pos[0]) + 0.5 - at[0],
                        f64::from(pos[2]) + 0.5 - at[2],
                    );
                    if x * x + z * z >= f64::from(radius * radius)
                        || !self.disk_predicate(&effect["predicate"], pos, new)
                        || self.level_set(pos, new).is_err()
                    {
                        continue;
                    }
                    if event {
                        self.post_block_game_event(
                            "minecraft:block_place",
                            pos,
                            Some(who),
                            Some(new),
                        );
                    }
                }
            }
        }
    }
    /// The `BlockPredicate` subset the disk effect uses, tested at `pos`; no
    /// predicate matches everything.
    fn disk_predicate(&mut self, p: &Value, pos: [i32; 3], placed: BlockStateId) -> bool {
        if p.is_null() {
            return true;
        }
        let offset = cell_offset(p);
        let at = [0, 1, 2].map(|i| pos[i] + offset[i]);
        let state = self.world.block(at[0], at[1], at[2]);
        let b = self.res.behaviours();
        let names = |v: &Value| -> Vec<String> {
            match v {
                Value::String(s) => vec![s.clone()],
                Value::Array(a) => a
                    .iter()
                    .filter_map(|n| n.as_str().map(String::from))
                    .collect(),
                _ => Vec::new(),
            }
        };
        match p["type"].as_str().unwrap_or("") {
            "minecraft:all_of" => p["predicates"]
                .as_array()
                .into_iter()
                .flatten()
                .all(|q| self.disk_predicate(q, pos, placed)),
            // `MatchingBlockTagPredicate`
            "minecraft:matching_block_tag" => b
                .tag(p["tag"].as_str().unwrap_or("").trim_start_matches('#'))
                .is_some_and(|t| t.contains(b.block(state))),
            // `MatchingBlocksPredicate`
            "minecraft:matching_blocks" => {
                names(&p["blocks"]).iter().any(|n| n == b.table.name(state))
            }
            // `MatchingFluidsPredicate`: Fluids.WATER is the source.
            "minecraft:matching_fluids" => {
                let fluid = b.fluid(state);
                names(&p["fluids"]).iter().any(|n| match n.as_str() {
                    "minecraft:water" => fluid.kind() == FluidKind::Water && fluid.is_source(),
                    "minecraft:flowing_water" => {
                        fluid.kind() == FluidKind::Water && !fluid.is_source()
                    }
                    _ => false,
                })
            }
            // `UnobstructedPredicate`: no building-blocking entity in the block.
            "minecraft:unobstructed" => self.with_level(|_, level, _| {
                level.is_unobstructed(BlockPos::new(pos[0], pos[1], pos[2]), placed)
            }),
            _ => false,
        }
    }
    /// Whether a worn enchantment gives immunity to `#burn_from_stepping`
    /// damage (frost walker's `minecraft:damage_immunity`).
    pub(crate) fn wears_burn_from_stepping_immunity(&self) -> bool {
        self.armor
            .iter()
            .zip(["head", "chest", "legs", "feet"])
            .any(|(stack, slot)| {
                stack
                    .iter()
                    .flat_map(ItemStack::enchantments)
                    .any(|(id, _)| {
                        self.stations().enchantments.get(id).is_some_and(|def| {
                            def.effects.get("minecraft:damage_immunity").is_some()
                                && def
                                    .slots
                                    .iter()
                                    .any(|s| s == slot || s == "armor" || s == "any")
                        })
                    })
            })
    }
    /// Enchantment attribute modifiers of active equipment, including held Efficiency.
    /// Each piece's modifier id ends with its slot (`EnchantmentAttributeEffect.idForSlot`),
    /// so the same enchantment on two pieces stacks instead of replacing itself.
    pub(super) fn apply_enchantment_attributes(&mut self) {
        let mut wanted: Vec<(String, String, f64, String)> = Vec::new();
        for (stack, slot) in self
            .armor
            .iter()
            .zip(["head", "chest", "legs", "feet"])
            .chain([
                (&self.inventory[self.selected], "mainhand"),
                (&self.offhand, "offhand"),
            ])
        {
            let Some(stack) = stack else { continue };
            for horizon_items::item_data::ModifierRow {
                attribute: attr,
                id,
                amount,
                operation: op,
                slot: group,
            } in self.stations().enchantments.modifier_rows(stack)
            {
                if super::equipment_modifier_applies(&group, slot) {
                    wanted.push((attr, format!("{id}/{slot}"), amount, op));
                }
            }
        }
        let Some(p) = self.player.as_mut() else {
            return;
        };
        for (attr, id, _, op) in std::mem::take(&mut self.potions.enchant_modifiers) {
            let _ = p.set_modifier_unclamped(&attr, &id, 0.0, &op, false);
        }
        for (attr, id, amount, op) in &wanted {
            let _ = p.set_modifier_unclamped(attr, id, *amount, op, true);
        }
        let _ = p.clamp_health_to_max();
        self.potions.enchant_modifiers = wanted;
    }
}

/// `EnchantmentHelper.runLocationChangedEffects`: the replace-disk effects of
/// active equipment slots, with the wearer's ground/passenger requirements.
fn location_disks<'a>(
    enchantments: &Enchantments,
    equipment: impl IntoIterator<Item = (&'a Option<ItemStack>, &'static str)>,
    on_ground: bool,
    riding: bool,
) -> Vec<(Value, u32)> {
    let mut disks = Vec::new();
    for (stack, slot) in equipment {
        let Some(stack) = stack else { continue };
        for (id, level) in stack.all_enchantments().iter() {
            let Some(def) = enchantments.get(id) else {
                continue;
            };
            let armor = matches!(slot, "head" | "chest" | "legs" | "feet");
            let hand = matches!(slot, "mainhand" | "offhand");
            if !def.slots.iter().any(|s| {
                s == slot || s == "any" || (s == "armor" && armor) || (s == "hand" && hand)
            }) {
                continue;
            }
            for effect in def.effects["minecraft:location_changed"]
                .as_array()
                .into_iter()
                .flatten()
            {
                if effect["effect"]["type"] == "minecraft:replace_disk"
                    && location_requirements(&effect["requirements"], on_ground, riding)
                {
                    disks.push((effect["effect"].clone(), *level));
                }
            }
        }
    }
    disks
}

/// Soul Speed's `attribute` effects at `level` (attribute, id, amount, operation),
/// the id taking the boots slot like `EnchantmentAttributeEffect.idForSlot`.
fn soul_speed_modifiers(
    enchantments: &Enchantments,
    level: u32,
) -> Vec<(String, String, f64, String)> {
    let Some(e) = enchantments.get("minecraft:soul_speed") else {
        return Vec::new();
    };
    e.effects["minecraft:location_changed"][0]["effect"]["effects"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|a| a["type"] == "minecraft:attribute")
        .map(|a| {
            (
                a["attribute"].as_str().unwrap_or("").to_owned(),
                format!("{}/feet", a["id"].as_str().unwrap_or("")),
                f64::from(level_value(&a["amount"], level)),
                a["operation"].as_str().unwrap_or("add_value").to_owned(),
            )
        })
        .collect()
}

/// An effect's or predicate's `offset` vector (zero when absent).
fn cell_offset(v: &Value) -> [i32; 3] {
    let at = |i: usize| {
        v["offset"]
            .get(i)
            .and_then(Value::as_i64)
            .map_or(0, |n| n as i32)
    };
    [at(0), at(1), at(2)]
}
/// The loot-condition subset a `location_changed` effect requires of the
/// wearer: standing on the ground and not riding (frost walker).
fn location_requirements(req: &Value, on_ground: bool, riding: bool) -> bool {
    match req["type"].as_str().unwrap_or("") {
        "" if req.is_null() => true,
        "minecraft:all_of" => req["terms"]
            .as_array()
            .into_iter()
            .flatten()
            .all(|t| location_requirements(t, on_ground, riding)),
        "minecraft:inverted" => !location_requirements(&req["term"], on_ground, riding),
        "minecraft:entity_properties" => {
            let p = &req["predicate"];
            p["minecraft:flags"]["is_on_ground"]
                .as_bool()
                .is_none_or(|v| v == on_ground)
                && (p["minecraft:vehicle"].is_null() || riding)
        }
        _ => false,
    }
}

/// `LivingEntity.collectEquipmentChanges`: which armor slots differ from the
/// `snapshot` taken at the last look, which is then brought up to date.
fn armor_changes(
    snapshot: &mut [Option<ItemStack>; 4],
    armor: &[Option<ItemStack>; 4],
) -> [bool; 4] {
    let changed: [bool; 4] = std::array::from_fn(|i| armor[i] != snapshot[i]);
    // Only a changed slot is copied: the check runs every tick for every player.
    for ((kept, worn), changed) in snapshot.iter_mut().zip(armor).zip(changed) {
        if changed {
            kept.clone_from(worn);
        }
    }
    changed
}

fn mending_entries(stack: &ItemStack, slot: PlayerSlot, enchantments: &Enchantments) -> usize {
    if stack.damage == 0 {
        return 0;
    }
    stack
        .all_enchantments()
        .iter()
        .filter(|(id, _)| {
            enchantments.get(id).is_some_and(|def| {
                def.effects.get("minecraft:repair_with_xp").is_some()
                    && def
                        .slots
                        .iter()
                        .any(|group| crate::game::equipment_modifier_applies(group, slot.name()))
            })
        })
        .count()
}

/// `EnchantmentHelper.modifyDurabilityToRepairFromXp`: the experience `amount`
/// repairs at the stack's `repair_with_xp` xp-per-point rate. Vanilla's
/// `Enchantment.itemContext` puts the repaired stack in
/// `LootContextParams.TOOL`, so `minecraft:match_tool` judges that item.
fn repair_from_xp(
    enchantments: &Enchantments,
    items: Option<&Items>,
    stack: &ItemStack,
    amount: u32,
    random: &mut LegacyRandom,
) -> u32 {
    let ctx = EffectContext {
        tool: Some(&stack.item),
        items,
        ..Default::default()
    };
    enchantments.modify_value(
        Some(stack),
        "minecraft:repair_with_xp",
        &ctx,
        random,
        amount as f32,
    ) as u32
}

/// `ExperienceOrb.repairPlayerItems`: the damage `to_repair` xp per point takes
/// off a stack with `damage`, and the experience left over after it
/// (`amount - repair * amount / toRepairFromXpAmount`, in vanilla's integer
/// order). Vanilla divides only when it repairs something, so a rate of zero
/// keeps all of `amount`.
fn repaired_and_left(amount: u32, damage: u32, to_repair: u32) -> (u32, u32) {
    let repaired = to_repair.min(damage);
    if repaired == 0 {
        return (0, amount);
    }
    let left = amount
        .saturating_sub((u64::from(repaired) * u64::from(amount) / u64::from(to_repair)) as u32);
    (repaired, left)
}

/// `EnchantmentHelper.getRandomItemWith(REPAIR_WITH_XP, player, ItemStack::isDamaged)`:
/// collects damaged effect-bearing pieces in `EquipmentSlot.VALUES` order and picks
/// one with the player's own random (`Entity.random`), never the level's.
fn pick_mending_slot(
    slots: [(PlayerSlot, &Option<ItemStack>); 6],
    enchantments: &Enchantments,
    random: &mut impl RandomSource,
) -> Option<PlayerSlot> {
    let candidates: Vec<PlayerSlot> = slots
        .into_iter()
        .flat_map(|(slot, stack)| {
            std::iter::repeat_n(
                slot,
                stack
                    .as_ref()
                    .map_or(0, |stack| mending_entries(stack, slot, enchantments)),
            )
        })
        .collect();
    if candidates.is_empty() {
        return None;
    }
    let count = candidates.len() as i32;
    let index = random.next_int_bound(count).unwrap_or(0) as usize;
    Some(candidates[index])
}

/// `SpawnParticlesEffect.apply` and `PlaySoundEffect.apply` for Soul Speed's
/// `minecraft:tick` effects: the sound's `random_chance` (35%) draws from
/// `level_random` (`Enchantment.entityContext`), while the particle coordinates
/// and sound pitch draw from `wearer_random` (`entity.getRandom()`).
fn soul_speed_tick(
    position: [f64; 3],
    moved: [f64; 3],
    width: f32,
    tick_effects: bool,
    level_random: &mut impl RandomSource,
    wearer_random: &mut impl RandomSource,
) -> (Option<([f64; 3], [f64; 3])>, Option<f32>) {
    // `spawn_particles`: in the bounding box, 0.1 above the feet, thrown
    // back against the movement (-0.2) and up (0.1).
    let particle = tick_effects.then(|| {
        let x = position[0] + (wearer_random.next_double() - 0.5) * f64::from(width);
        let z = position[2] + (wearer_random.next_double() - 0.5) * f64::from(width);
        (
            [x, position[1] + 0.1, z],
            [moved[0] * -0.2, 0.1, moved[2] * -0.2],
        )
    });
    // `play_sound`: its `random_chance` (35%) comes first in its `all_of`,
    // so the level random is drawn every tick the boots are worn; volume
    // 0.6, pitch 0.6 to 1.0 from the wearer's random.
    let pitch = if level_random.next_float() < 0.35 && tick_effects {
        Some(wearer_random.next_float() * 0.4 + 0.6)
    } else {
        None
    };
    (particle, pitch)
}

#[cfg(test)]
mod tests {
    use crate::game::damage_item_tests::{Rule, with_wear_part};
    use horizon_client::mobs::{Mob, finalize};
    use horizon_core::geometry::Vec3;

    #[test]
    #[ignore = "daily"]
    fn mob_armour_obeys_negative_cancel_and_breaking_answers() {
        with_wear_part("minecraft:iron_helmet", Rule::Set(-1), |game, wear| {
            let kind = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            let mut mob = Mob::new(901, kind, Vec3::ZERO, 0.0);
            finalize::set_item_stack(&mut mob, 5, ItemStack::new("minecraft:iron_helmet", 1));
            game.mobs.push(mob);
            for rule in [Rule::Set(i32::MIN), Rule::Cancel] {
                wear.set(rule);
                game.wear_mob_piece(901, 5, 1);
                assert_eq!(
                    finalize::item_stack(game.mobs.last().unwrap(), 5)
                        .unwrap()
                        .damage,
                    0
                );
                assert_eq!(wear.take_asked(), [1]);
            }
            wear.set(Rule::Set(164));
            game.wear_mob_piece(901, 5, 1);
            assert_eq!(
                finalize::item_stack(game.mobs.last().unwrap(), 5)
                    .unwrap()
                    .damage,
                164
            );
            wear.set(Rule::Set(i32::MAX));
            game.wear_mob_piece(901, 5, 1);
            assert!(finalize::item_stack(game.mobs.last().unwrap(), 5).is_none());
            assert_eq!(wear.take_asked(), [1, 1]);
            game.wear_mob_piece(901, 5, 1);
            assert!(wear.take_asked().is_empty());
        });
    }

    use super::{
        Enchantments, armor_changes, block_break_speed_factor, pick_mending_slot, repair_from_xp,
        repaired_and_left, soul_speed_tick,
    };

    use horizon_client::items::ItemStack;
    use horizon_core::random::{LegacyRandom, RandomSource};
    use horizon_world::loot::PackOverlay;

    /// The snapshot reports a newly worn piece once, then settles.
    #[test]
    fn armor_changes_settle_after_one_look() {
        let worn = [
            None,
            None,
            None,
            Some(ItemStack::new("minecraft:iron_boots", 1)),
        ];
        let mut snapshot: [Option<ItemStack>; 4] = Default::default();
        assert_eq!(
            armor_changes(&mut snapshot, &worn),
            [false, false, false, true]
        );
        assert_eq!(armor_changes(&mut snapshot, &worn), [false; 4]);
    }

    #[test]
    #[ignore = "daily"]
    fn thorns_and_boots_wear_break_the_equipped_piece() {
        use horizon_client::items::ItemStack;
        use horizon_client::motion::PlayerMotion;
        use horizon_core::geometry::Vec3;
        super::super::native_entity_refs::tests::with_game(|game| {
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            let mut chest = ItemStack::new("minecraft:diamond_chestplate", 1);
            chest.damage = game.items().get(&chest.item).unwrap().max_damage - 1;
            chest.data_mut().enchantments = vec![("minecraft:thorns".into(), 255)];
            game.armor[1] = Some(chest);
            assert!(!game.thorns_damage().is_empty());
            assert!(game.armor[1].is_none());
            let mut boots = ItemStack::new("minecraft:diamond_boots", 1);
            boots.damage = game.items().get(&boots.item).unwrap().max_damage - 1;
            game.armor[3] = Some(boots);
            game.wear_boots(1);
            assert!(game.armor[3].is_none());
        });
    }

    /// arismoko/horizon#4094: `BLOCK_BREAK_SPEED` is a multiplier on the destroy speed.
    #[test]
    fn block_break_speed_is_a_destroy_speed_multiplier() {
        assert_eq!(block_break_speed_factor(None), 1.0);
        assert_eq!(block_break_speed_factor(Some(1.0)), 1.0);
        assert_eq!(block_break_speed_factor(Some(0.5)), 0.5);
        assert_eq!(block_break_speed_factor(Some(2.0)), 2.0);
        assert_eq!(8.0 * block_break_speed_factor(Some(0.5)), 4.0);
        assert_eq!(8.0 * block_break_speed_factor(Some(2.0)), 16.0);
    }

    /// #11309: `EnchantmentHelper.getRandomItemWith` picks among damaged
    /// Mending items in `EquipmentSlot.VALUES` order using the player's RNG,
    /// leaving the world RNG untouched.
    #[test]
    fn mixed_repair_enchantments_weight_each_entry() {
        let definition = serde_json::json!({
            "description": "Repair", "supported_items": "minecraft:diamond_pickaxe",
            "weight": 1, "max_level": 1, "anvil_cost": 1,
            "min_cost": {"base": 1, "per_level_above_first": 0},
            "max_cost": {"base": 1, "per_level_above_first": 0},
            "slots": ["any"],
            "effects": {"minecraft:repair_with_xp": [{"effect": {"type": "minecraft:multiply", "factor": 2}}]}
        });
        let enchantments = super::Enchantments::default()
            .with_entries([("test:first", &definition), ("test:second", &definition)])
            .unwrap()
            .unwrap();
        let mut main = ItemStack::new("minecraft:diamond_pickaxe", 1);
        main.damage = 20;
        main.set_crafting_enchantments(vec![("test:first".into(), 1), ("test:second".into(), 1)]);
        let mut off = ItemStack::new("minecraft:diamond_pickaxe", 1);
        off.damage = 20;
        off.set_crafting_enchantments(vec![("test:first".into(), 1)]);
        let main = Some(main);
        let off = Some(off);
        let none = None;
        use super::PlayerSlot::{Armor, Main, Off};
        let slots = [
            (Main, &main),
            (Off, &off),
            (Armor(3), &none),
            (Armor(2), &none),
            (Armor(1), &none),
            (Armor(0), &none),
        ];
        let expected_candidates = [Main, Main, Off];
        let mut actual_rng = LegacyRandom::new(20_002);
        let mut expected_rng = LegacyRandom::new(20_002);
        for _ in 0..32 {
            let index = expected_rng.next_int_bound(3).unwrap() as usize;
            assert_eq!(
                pick_mending_slot(slots, &enchantments, &mut actual_rng),
                Some(expected_candidates[index])
            );
        }
        assert_eq!(actual_rng.next_long(), expected_rng.next_long());
    }

    #[test]
    fn mending_picks_damaged_candidate_in_equipment_slot_order_from_player_rng() {
        let enchantments = super::Enchantments::default().with_entries([("minecraft:mending", &serde_json::json!({
            "description": "Mending", "supported_items": "minecraft:diamond_pickaxe", "weight": 1,
            "max_level": 1, "min_cost": {"base": 1, "per_level_above_first": 0},
            "max_cost": {"base": 1, "per_level_above_first": 0}, "anvil_cost": 1,
            "slots": ["any"], "effects": {"minecraft:repair_with_xp": [{"effect": {"type": "minecraft:multiply", "factor": 2}}]}
        }))]).unwrap().unwrap();
        let mending_item = |id: &str, damage: u32| {
            let mut stack = ItemStack::new(id, 1);
            stack.damage = damage;
            stack.set_crafting_enchantments(vec![("minecraft:mending".into(), 1)]);
            Some(stack)
        };
        let mut unbreaking = ItemStack::new("minecraft:diamond_leggings", 1);
        unbreaking.damage = 5;
        unbreaking.set_crafting_enchantments(vec![("minecraft:unbreaking".into(), 3)]);

        let main = mending_item("minecraft:diamond_pickaxe", 12);
        let off = mending_item("minecraft:shield", 3);
        let head = mending_item("minecraft:diamond_helmet", 4);
        let chest = mending_item("minecraft:diamond_chestplate", 0);
        let legs = Some(unbreaking);
        let feet = mending_item("minecraft:diamond_boots", 9);

        let slots = [
            (super::PlayerSlot::Main, &main),
            (super::PlayerSlot::Off, &off),
            (super::PlayerSlot::Armor(3), &feet),
            (super::PlayerSlot::Armor(2), &legs),
            (super::PlayerSlot::Armor(1), &chest),
            (super::PlayerSlot::Armor(0), &head),
        ];

        let mut world_rng = LegacyRandom::new(10_001);
        let mut expected_world = LegacyRandom::new(10_001);
        let mut player_rng = LegacyRandom::new(20_002);
        let mut expected_player = LegacyRandom::new(20_002);

        let expected_candidates = [
            super::PlayerSlot::Main,
            super::PlayerSlot::Off,
            super::PlayerSlot::Armor(3),
            super::PlayerSlot::Armor(0),
        ];
        for _ in 0..16 {
            let picked = pick_mending_slot(slots, &enchantments, &mut player_rng);
            let idx = expected_player.next_int_bound(4).unwrap() as usize;
            assert_eq!(picked, Some(expected_candidates[idx]));
        }
        assert_eq!(player_rng.next_long(), expected_player.next_long());
        assert_eq!(world_rng.next_long(), expected_world.next_long());

        let none: Option<ItemStack> = None;
        let empty = [
            (super::PlayerSlot::Main, &none),
            (super::PlayerSlot::Off, &chest),
            (super::PlayerSlot::Armor(3), &none),
            (super::PlayerSlot::Armor(2), &legs),
            (super::PlayerSlot::Armor(1), &chest),
            (super::PlayerSlot::Armor(0), &none),
        ];
        assert_eq!(
            pick_mending_slot(empty, &enchantments, &mut player_rng),
            None
        );
        assert_eq!(player_rng.next_long(), expected_player.next_long());
    }

    #[test]
    fn mending_pick_counts_each_matching_enchantment() {
        use super::{Enchantments, PLAYER_SLOTS, PlayerSlot};
        let definition = serde_json::json!({
            "description": "Repair", "supported_items": "minecraft:diamond_pickaxe",
            "weight": 1, "max_level": 1, "anvil_cost": 1,
            "min_cost": {"base": 1, "per_level_above_first": 0},
            "max_cost": {"base": 1, "per_level_above_first": 0},
            "slots": ["any"], "effects": {"minecraft:repair_with_xp": [
                {"effect": {"type": "minecraft:multiply", "factor": 2}}
            ]}
        });
        let enchantments = Enchantments::default()
            .with_entries([
                ("minecraft:mending", &definition),
                ("test:patch", &definition),
            ])
            .unwrap()
            .unwrap();
        let mut main = ItemStack::new("minecraft:diamond_pickaxe", 1);
        main.damage = 20;
        main.set_crafting_enchantments(vec![
            ("minecraft:mending".into(), 1),
            ("test:patch".into(), 1),
        ]);
        let mut off = ItemStack::new("minecraft:diamond_pickaxe", 1);
        off.damage = 20;
        off.set_crafting_enchantments(vec![("minecraft:mending".into(), 1)]);
        let (main, off, empty) = (Some(main), Some(off), None);
        let slots = PLAYER_SLOTS.map(|slot| {
            (
                slot,
                match slot {
                    PlayerSlot::Main => &main,
                    PlayerSlot::Off => &off,
                    _ => &empty,
                },
            )
        });
        let candidates = [PlayerSlot::Main, PlayerSlot::Main, PlayerSlot::Off];
        let mut actual = LegacyRandom::new(3215);
        let mut expected = LegacyRandom::new(3215);
        for _ in 0..64 {
            let index = expected.next_int_bound(3).unwrap() as usize;
            assert_eq!(
                pick_mending_slot(slots, &enchantments, &mut actual),
                Some(candidates[index])
            );
        }
        assert_eq!(actual.next_long(), expected.next_long());
    }

    /// #11309: Soul Speed's tick effects draw the 35% sound requirement from
    /// the level RNG and sample particle coordinates and sound pitch from the
    /// wearer's RNG.
    #[test]
    fn soul_speed_samples_particle_and_pitch_from_wearer_rng() {
        let pos = [10.0, 64.0, -4.0];
        let moved = [0.25, 0.0, -0.5];
        let width = 0.6f32;

        for seed in 0..32 {
            let mut l_rng = LegacyRandom::new(seed);
            let mut expected_level = LegacyRandom::new(seed);
            let mut w_rng = LegacyRandom::new(seed + 1_000);
            let mut expected_wearer = LegacyRandom::new(seed + 1_000);

            let out = soul_speed_tick(pos, moved, width, true, &mut l_rng, &mut w_rng);
            let want_x = pos[0] + (expected_wearer.next_double() - 0.5) * f64::from(width);
            let want_z = pos[2] + (expected_wearer.next_double() - 0.5) * f64::from(width);
            let want_pos = [want_x, pos[1] + 0.1, want_z];
            let want_vel = [moved[0] * -0.2, 0.1, moved[2] * -0.2];
            assert_eq!(out.0, Some((want_pos, want_vel)));
            let want_pitch = if expected_level.next_float() < 0.35 {
                Some(expected_wearer.next_float() * 0.4 + 0.6)
            } else {
                None
            };
            assert_eq!(out.1, want_pitch);
            assert_eq!(l_rng.next_long(), expected_level.next_long());
            assert_eq!(w_rng.next_long(), expected_wearer.next_long());
        }

        // When tick_effects is false, the leading 35% random_chance still draws
        // one float from level_rng while wearer_rng is untouched.
        let mut l_rng = LegacyRandom::new(77);
        let mut expected_level = LegacyRandom::new(77);
        let mut w_rng = LegacyRandom::new(88);
        let mut expected_wearer = LegacyRandom::new(88);
        let out = soul_speed_tick(pos, moved, width, false, &mut l_rng, &mut w_rng);
        let _ = expected_level.next_float();
        assert_eq!(out, (None, None));
        assert_eq!(l_rng.next_long(), expected_level.next_long());
        assert_eq!(w_rng.next_long(), expected_wearer.next_long());
    }

    /// A pack's `repair_with_xp` gated on swords: `Enchantment.itemContext`
    /// hands the repaired stack to the conditions as `LootContextParams.TOOL`,
    /// so the sword repairs at twice the xp and the axe at the plain rate.
    #[test]
    fn mending_repair_judges_match_tool_by_the_repaired_stack() {
        let files = [(
            "enchantment/pack:xp_repair".to_owned(),
            vec![
                br#"{"description":"xp repair","supported_items":"minecraft:diamond_sword","slots":["mainhand"],"effects":{"minecraft:repair_with_xp":[{"effect":{"type":"minecraft:multiply","factor":2.0},"requirements":{"type":"minecraft:match_tool","predicate":{"items":"minecraft:diamond_sword"}}}]}}"#
                    .to_vec(),
            ],
        )];
        let root = std::env::temp_dir().join(format!(
            "horizon-mending-tool-context-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("items.json"), "{}").unwrap();
        let defaults = horizon_world::item_defaults::read(&root).unwrap();
        let e = Enchantments::load_with(&root, &PackOverlay::new(&files), &defaults).unwrap();
        std::fs::remove_dir_all(root).unwrap();
        let gated = |id: &str| {
            let mut stack = ItemStack::new(id, 1);
            stack.damage = 20;
            stack.set_crafting_enchantments(vec![("pack:xp_repair".into(), 1)]);
            stack
        };
        let mut random = LegacyRandom::new(7);
        // The sword matches: 5 xp repairs 10 of the stack's 20 damage and uses
        // it all up; 2 damage would eat 1 xp (2 * 5 / 10, vanilla's integer
        // order) and leave 4.
        let sword = gated("minecraft:diamond_sword");
        let to_repair = repair_from_xp(&e, None, &sword, 5, &mut random);
        assert_eq!(to_repair, 10);
        assert_eq!(repaired_and_left(5, sword.damage, to_repair), (10, 0));
        assert_eq!(repaired_and_left(5, 2, to_repair), (2, 4));
        // The axe matches no tool condition: 1 xp per damage point, so 5 xp
        // repairs 5 damage; 2 damage would eat 2 xp and leave 3.
        let axe = gated("minecraft:diamond_axe");
        let to_repair = repair_from_xp(&e, None, &axe, 5, &mut random);
        assert_eq!(to_repair, 5);
        assert_eq!(repaired_and_left(5, axe.damage, to_repair), (5, 0));
        assert_eq!(repaired_and_left(5, 2, to_repair), (2, 3));
        // `minecraft:multiply` draws nothing: the level RNG is untouched.
        assert_eq!(random.next_long(), LegacyRandom::new(7).next_long());
    }
}

#[cfg(test)]
mod impulse_rounding_tests {
    #[test]
    fn impulse_matches_pinned_joml_components() {
        let effect =
            serde_json::json!({"direction":[0,0,1],"coordinate_scale":[1,1,1],"magnitude":1});
        let v = super::look_impulse(&effect, 1, 45.0, 30.0);
        assert_eq!(
            [v.x, v.y, v.z],
            [-0.6123724579811096, -0.4999999701976776, 0.6123723983764648]
        );
    }
}
