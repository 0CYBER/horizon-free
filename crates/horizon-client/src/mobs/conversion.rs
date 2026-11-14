//! `Mob.convertTo` with `ConversionType.SINGLE`: one mob turns into another
//! of a different type in its place, keeping what the old one had.
use super::Mob;
use horizon_core::random::RandomSource;

/// `ConversionParams` of a `SINGLE` conversion.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Params {
    /// `keepEquipment`: the worn and held items move to the new mob, with
    /// their drop chances.
    pub keep_equipment: bool,
    /// `preserveCanPickUpLoot`.
    pub preserve_can_pick_up_loot: bool,
}
impl Params {
    /// `ConversionParams.single(mob, true, true)`, what `ConversionTracker`
    /// uses (zombie to drowned, husk to zombie, skeleton to stray).
    pub const KEEP_ALL: Self = Self {
        keep_equipment: true,
        preserve_can_pick_up_loot: true,
    };
    /// `ConversionParams.single(mob, false, false)`: the old mob's gear stays
    /// behind and the new mob keeps its own `canPickUpLoot` (`MushroomCow.shear`,
    /// `Villager.thunderHit`).
    pub const NONE: Self = Self {
        keep_equipment: false,
        preserve_can_pick_up_loot: false,
    };
}

/// `ConversionParams.AfterConversion`: what the converting mob does once the
/// new one exists (`MobEvent::Convert`'s `after`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum After {
    /// No step: `Mob.convertTo` with a bare `ConversionParams.single`.
    #[default]
    None,
    /// `AbstractPiglin.finishConversion` and `Hoglin.finishConversion`: the
    /// new mob is nauseous for 10 seconds.
    Nausea,
    /// `Pig.thunderHit`: `populateDefaultEquipmentSlots` off the pig's own
    /// random, then `setPersistenceRequired`.
    ZombifiedPiglinGear,
    /// `Villager.thunderHit`: `finalizeSpawn(CONVERSION)`, then
    /// `setPersistenceRequired`.
    Witch,
    /// `Tadpole.ageUp`: `finalizeSpawn(CONVERSION)`, then
    /// `setPersistenceRequired`.
    Finalize,
}

impl After {
    /// Whether the caller already asked `LivingConversionEvent.Pre`: the
    /// lightning picks its outcome from the answer, so `convert_to` must not
    /// ask again.
    pub fn pre_checked(self) -> bool {
        matches!(self, Self::ZombifiedPiglinGear | Self::Witch)
    }
}

/// `Pig.thunderHit`'s completion after `single`: choose the converted pig's
/// weapon from the source pig's RNG and prevent the result from despawning.
pub fn pig_thunder_hit(from: &mut Mob, to: &mut Mob) {
    let spear = from.random.next_int_bound(20).unwrap_or(1) == 0;
    to.extra.set_main_hand(Some(if spear {
        "minecraft:golden_spear".into()
    } else {
        "minecraft:golden_sword".into()
    }));
    to.persistent = true;
}

/// `ConversionType.SINGLE.convert` and `ConversionType.convertCommon`: `to`
/// is a fresh mob (`EntityType.create(level, CONVERSION)`, no spawn
/// finalization) that takes `from`'s place, motion, ride, lead, equipment,
/// effects, age, name and flags. Its health stays its own type's.
pub fn single(from: &Mob, to: &mut Mob, params: Params) {
    // copyPosition, setDeltaMovement and the old mob's vehicle.
    to.position = from.position;
    to.previous = from.position;
    to.yaw = from.yaw;
    to.previous_yaw = from.yaw;
    to.head_yaw = from.head_yaw;
    to.previous_head_yaw = from.head_yaw;
    to.head_pitch = from.head_pitch;
    to.previous_head_pitch = from.head_pitch;
    to.velocity = from.velocity;
    to.extra.mount = from.extra.mount;
    to.extra.passenger = from.extra.passenger;
    if params.keep_equipment {
        // Every slot, copyAndClear, and setDropChance from the old mob.
        for slot in horizon_items::equipment::MAINHAND..=horizon_items::equipment::BODY {
            to.extra
                .set_slot_stack(slot, from.extra.slot_stack(slot).cloned());
        }
        to.extra.gear = from.extra.gear.clone();
        to.extra.drop_chances = from.extra.drop_chances.clone();
    }
    to.fall_distance = from.fall_distance;
    to.extra.fall_flying = from.extra.fall_flying;
    to.ai.last_hurt_by_player = from.ai.last_hurt_by_player;
    to.hurt_time = from.hurt_time;
    to.body_yaw = from.body_yaw;
    to.previous_body_yaw = from.body_yaw;
    to.on_ground = from.on_ground;
    to.extra.leash = from.extra.leash;
    to.extra.leash_pending = from.extra.leash_pending;
    to.extra.leash_player = from.extra.leash_player;
    common(from, to, params.preserve_can_pick_up_loot);
    // A zombie that turns into another zombie keeps canBreakDoors.
    if super::doors::is_zombie(from.species()) && super::doors::is_zombie(to.species()) {
        super::doors::set_can_break_doors(to, from.extra.can_break_doors);
    }
}

/// State shared by single and split conversions, without spawn finalization.
pub fn common(from: &Mob, to: &mut Mob, preserve_can_pick_up_loot: bool) {
    // setAbsorptionAmount runs before the effects are copied to the fresh mob.
    let maximum = *to
        .kind
        .attributes
        .get("minecraft:max_absorption")
        .unwrap_or(&0.0) as f32;
    to.absorption = from.absorption.clamp(0.0, maximum);
    // addEffect for each: Absorption's onEffectStarted raises it again.
    for effect in &from.extra.effects {
        to.add_effect(effect.id, effect.duration, effect.amplifier);
        // `new MobEffectInstance(effect)` copies its icon flag too.
        if let Some(copy) = to.extra.effects.iter_mut().find(|e| e.id == effect.id) {
            copy.show_icon = effect.show_icon;
        }
    }
    to.extra.hidden_effects = from.extra.hidden_effects.clone();
    to.extra.effect_chains = from.extra.effect_chains.clone();
    to.extra.ambient_effects = from.extra.ambient_effects.clone();
    if to
        .extra
        .effects
        .iter()
        .any(|e| e.id == "minecraft:health_boost")
    {
        super::finalize::health_boost_changed(to);
    }
    if from.is_baby() {
        super::finalize::set_baby(to, true);
    }
    // An ageable pair keeps the exact age, an adult's breeding cooldown too.
    if super::finalize::ageable(from.species()) && super::finalize::ageable(to.species()) {
        super::finalize::set_age(to, from.extra.age, None);
        to.extra.forced_age = from.extra.forced_age;
        to.extra.forced_age_timer = from.extra.forced_age_timer;
    }
    if preserve_can_pick_up_loot {
        to.extra.pick_up_loot = from.extra.pick_up_loot;
    }
    to.extra.left_handed = from.extra.left_handed;
    to.no_ai = from.no_ai;
    to.persistent |= from.persistent;
    to.name_visible = from.name_visible;
    to.invulnerable_flag = from.invulnerable_flag;
    // setInvulnerableTime(getInvulnerableTime()); the hurt cooldown is not copied.
    to.invulnerable_time = from.invulnerable_time;
    to.no_gravity = from.no_gravity;
    to.silent = from.silent;
    // COMPONENTS_TO_COPY: the custom name.
    to.custom_name = from.custom_name.clone();
    to.custom_name_text = from.custom_name_text.clone();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{items::ItemStack, mobs::world::with_test_world};
    use horizon_core::geometry::Vec3;
    use std::sync::Arc;

    #[test]
    fn absorption_is_clamped_then_restarted_by_a_copied_absorption_effect() {
        with_test_world(&[], |_, types| {
            let mob =
                |id, kind: &str| Mob::new(id, Arc::new(types.types[kind].clone()), Vec3::ZERO, 0.0);
            let mut s = mob(1, "minecraft:skeleton");
            s.absorption = 3.5;
            let mut plain = mob(2, "minecraft:stray");
            single(&s, &mut plain, Params::KEEP_ALL);
            assert_eq!(plain.absorption, 0.0);
            // AbsorptionMobEffect.onEffectStarted: 4 per level, amplifier 1.
            s.extra.effects.push(crate::mobs::StoredEffect::new(
                "minecraft:absorption",
                600,
                1,
            ));
            let mut boosted = mob(3, "minecraft:stray");
            single(&s, &mut boosted, Params::KEEP_ALL);
            assert_eq!(boosted.absorption, 8.0);
            assert_eq!(
                boosted.extra.effects,
                vec![crate::mobs::StoredEffect::new(
                    "minecraft:absorption",
                    600,
                    1
                )]
            );
        });
    }
    #[test]
    fn a_frozen_skeleton_becomes_a_stray_that_keeps_its_bow_and_name() {
        with_test_world(&[], |_, types| {
            let mob = |id, kind: &str| {
                Mob::new(
                    id,
                    Arc::new(types.types[kind].clone()),
                    Vec3::new(3.5, 70.0, -2.5),
                    0.0,
                )
            };
            let mut s = mob(1, "minecraft:skeleton");
            s.yaw = 90.0;
            s.custom_name = Some("Frosty".into());
            s.no_ai = true;
            s.absorption = 3.5;
            s.extra
                .set_main_hand_stack(Some(ItemStack::new("minecraft:bow", 1)));
            s.extra.set_slot_id(
                horizon_items::equipment::CHEST,
                Some("minecraft:chainmail_chestplate".into()),
            );
            let mut offhand = ItemStack::new("minecraft:shield", 2);
            offhand.damage = 5;
            crate::mobs::finalize::set_item_stack(
                &mut s,
                crate::mobs::finalize::OFFHAND,
                offhand.clone(),
            );
            let mut armor = ItemStack::new("minecraft:wolf_armor", 1);
            armor.damage = 7;
            armor.data_mut().custom_name = Some(horizon_core::nbt::Tag::String("kept".into()));
            s.extra.set_body_stack(Some(armor));
            s.extra.leash = Some(crate::mobs::leash::Holder::Knot([3, 71, -3]));
            s.extra.mount = Some(7);
            s.extra.passenger = true;
            let mut st = mob(2, "minecraft:stray");
            single(&s, &mut st, Params::KEEP_ALL);
            assert_eq!(st.custom_name.as_deref(), Some("Frosty"));
            assert!(st.no_ai);
            assert_eq!(st.absorption, 0.0);
            assert_eq!(st.yaw, 90.0);
            assert_eq!(
                st.extra.main_hand_stack().map(|s| &*s.item),
                Some("minecraft:bow")
            );
            assert_eq!(
                st.extra.slot_id(horizon_items::equipment::CHEST),
                Some("minecraft:chainmail_chestplate")
            );
            assert_eq!(
                st.extra.leash,
                Some(crate::mobs::leash::Holder::Knot([3, 71, -3]))
            );
            assert_eq!(st.extra.mount, Some(7));
            assert_eq!(st.extra.body_id(), Some("minecraft:wolf_armor"));
            assert_eq!(st.extra.body_damage(), 7);
            assert_eq!(st.equipment_stacks()[1], Some(offhand));
            assert_eq!(st.extra.body_stack(), s.extra.body_stack());
            // Health is the new type's own, not copied.
            assert_eq!(st.health, mob(3, "minecraft:stray").health);
            // Without keepEquipment the gear stays behind.
            let mut bare = mob(4, "minecraft:stray");
            single(
                &s,
                &mut bare,
                Params {
                    keep_equipment: false,
                    preserve_can_pick_up_loot: false,
                },
            );
            assert!(
                bare.extra.main_hand_id().is_none()
                    && bare
                        .extra
                        .slot_stack(horizon_items::equipment::CHEST)
                        .is_none()
                    && bare.extra.body_id().is_none()
            );
            assert_eq!(bare.custom_name.as_deref(), Some("Frosty"));
        });
    }
    /// `ConversionType.convertCommon` copies absorption through the target's
    /// setter, so it survives when the target's max absorption permits it.
    #[test]
    fn a_converting_mob_keeps_its_absorption() {
        with_test_world(&[], |_, types| {
            let mob = |id, kind: &str| {
                Mob::new(
                    id,
                    Arc::new(types.types[kind].clone()),
                    Vec3::new(3.5, 70.0, -2.5),
                    0.0,
                )
            };
            let mut z = mob(1, "minecraft:zombie");
            z.absorption = 3.0;
            let mut d = mob(2, "minecraft:drowned");
            // LivingEntity.setAbsorptionAmount clamps to the new mob's maximum
            // before effects are copied. An unmodified drowned has maximum 0.
            Arc::make_mut(&mut Arc::make_mut(&mut d.kind).attributes)
                .insert("minecraft:max_absorption".into(), 3.0);
            single(&z, &mut d, Params::KEEP_ALL);
            assert_eq!(d.absorption, 3.0, "the absorption crosses the conversion");
        });
    }
}
