//! Entity reads from mobs lent to an AI goal or target-change callback.
//!
//! These use the mob in hand or the disjoint live loan; a snapshot does not
//! carry these fields and must not be mistaken for a live entity.
use super::super::{
    native_entity_moves::facing,
    native_entity_sounds::hostile_species,
    native_entity_state::{POSES, max_air},
};
use super::{GoalCx, Resolved};
use crate::mobs::Mob;
use mod_api::{EntityRef, MobCategory};

impl GoalCx<'_, '_> {
    pub(super) fn r_entity_tags(&self, entity: EntityRef) -> Option<Vec<String>> {
        self.w.goal_host?.mob_tags(self.entity_mob(entity)?)
    }

    pub(super) fn r_entity_name(&self, entity: EntityRef) -> Option<String> {
        Some(
            self.w
                .goal_host?
                .mob_name(self.entity_mob(entity)?, self.m, self.w),
        )
    }

    pub(super) fn r_entity_added_to_level(&self, entity: EntityRef) -> bool {
        match (self.w.goal_host, self.entity_mob(entity)) {
            (Some(host), Some(mob)) => host.mob_added_to_level(mob),
            _ => false,
        }
    }

    pub(super) fn r_entity_portal_cooldown(&self, entity: EntityRef) -> Option<i32> {
        Some(
            self.w
                .goal_host?
                .mob_portal_cooldown(self.entity_mob(entity)?),
        )
    }

    pub(super) fn r_entity_invulnerable_to_damage(
        &self,
        entity: EntityRef,
        damage_type: &str,
    ) -> Option<bool> {
        Some(
            self.w
                .goal_host?
                .mob_invulnerable_to_damage(self.entity_mob(entity)?, damage_type),
        )
    }
    pub(super) fn entity_mob(&self, entity: EntityRef) -> Option<&Mob> {
        match self.resolve(entity)? {
            Resolved::Own => Some(self.m),
            Resolved::Live(mob) => Some(mob),
            _ => None,
        }
    }

    pub(super) fn r_entity_custom_name_visible(&self, entity: EntityRef) -> Option<bool> {
        Some(self.entity_mob(entity)?.name_visible)
    }

    pub(super) fn r_entity_classification(&self, entity: EntityRef) -> Option<MobCategory> {
        Some(match self.entity_mob(entity)?.category() {
            "monster" => MobCategory::Monster,
            "creature" => MobCategory::Creature,
            "ambient" => MobCategory::Ambient,
            "axolotls" => MobCategory::Axolotls,
            "underground_water_creature" => MobCategory::UndergroundWaterCreature,
            "water_creature" => MobCategory::WaterCreature,
            "water_ambient" => MobCategory::WaterAmbient,
            _ => MobCategory::Misc,
        })
    }

    pub(super) fn r_entity_permanently_invulnerable(&self, entity: EntityRef) -> Option<bool> {
        Some(self.entity_mob(entity)?.invulnerable_flag)
    }

    pub(super) fn r_entity_invulnerable_time(&self, entity: EntityRef) -> Option<i32> {
        Some(self.entity_mob(entity)?.invulnerable_time)
    }

    pub(super) fn r_entity_damage_cooldown_time(&self, entity: EntityRef) -> Option<i32> {
        Some(self.entity_mob(entity)?.invulnerable)
    }

    pub(super) fn r_entity_invulnerable_to_piercing_weapon(
        &self,
        entity: EntityRef,
    ) -> Option<bool> {
        Some(self.entity_mob(entity)?.is_invulnerable())
    }

    pub(super) fn r_entity_no_physics(&self, entity: EntityRef) -> Option<bool> {
        self.entity_mob(entity).map(|mob| mob.no_physics)
    }

    pub(super) fn r_entity_spectator(&self, entity: EntityRef) -> Option<bool> {
        self.entity_mob(entity).map(|_| false)
    }

    pub(super) fn r_entity_is_shift_key_down(&self, entity: EntityRef) -> Option<bool> {
        self.entity_mob(entity).map(|_| false)
    }

    pub(super) fn r_entity_is_sprinting(&self, entity: EntityRef) -> Option<bool> {
        Some(self.entity_mob(entity)?.extra.sprinting)
    }

    pub(super) fn r_entity_is_in_water(&self, entity: EntityRef) -> Option<bool> {
        Some(self.entity_mob(entity)?.fluids.in_water)
    }

    pub(super) fn r_entity_is_in_lava(&self, entity: EntityRef) -> Option<bool> {
        Some(self.entity_mob(entity)?.fluids.in_lava)
    }

    pub(super) fn r_entity_fire_immune(&self, entity: EntityRef) -> Option<bool> {
        Some(crate::mobs::spawn_rules::fire_immune(
            self.entity_mob(entity)?.species(),
        ))
    }

    pub(super) fn r_entity_fire_ticks(&self, entity: EntityRef) -> Option<i32> {
        Some(self.entity_mob(entity)?.fire_ticks)
    }

    pub(super) fn r_entity_air_supply(&self, entity: EntityRef) -> Option<i32> {
        Some(self.entity_mob(entity)?.air)
    }

    pub(super) fn r_entity_max_air_supply(&self, entity: EntityRef) -> Option<i32> {
        Some(max_air(Some(self.entity_mob(entity)?.species())))
    }

    pub(super) fn r_entity_is_silent(&self, entity: EntityRef) -> Option<bool> {
        Some(self.entity_mob(entity)?.silent)
    }

    pub(super) fn r_entity_glowing(&self, entity: EntityRef) -> Option<bool> {
        Some(self.entity_mob(entity)?.glowing)
    }

    pub(super) fn r_entity_no_gravity(&self, entity: EntityRef) -> Option<bool> {
        Some(self.entity_mob(entity)?.no_gravity)
    }

    pub(super) fn r_entity_ticks_frozen(&self, entity: EntityRef) -> Option<i32> {
        Some(self.entity_mob(entity)?.ticks_frozen)
    }

    pub(super) fn r_entity_tick_count(&self, entity: EntityRef) -> Option<i32> {
        Some(self.entity_mob(entity)?.age as i32)
    }

    pub(super) fn r_entity_is_invisible(&self, entity: EntityRef) -> Option<bool> {
        Some(self.entity_mob(entity)?.is_invisible())
    }
    pub(super) fn r_entity_is_multipart(&self, entity: EntityRef) -> Option<bool> {
        self.entity_mob(entity).map(|_| false)
    }

    pub(super) fn r_entity_parts(&self, entity: EntityRef) -> Option<Vec<EntityRef>> {
        self.entity_mob(entity).map(|_| Vec::new())
    }

    pub(super) fn r_entity_dimensions(&self, entity: EntityRef) -> Option<[f64; 2]> {
        let bounds = self.entity_mob(entity)?.bounds();
        use horizon_core::geometry::Axis;
        Some([
            bounds.max(Axis::X) - bounds.min(Axis::X),
            bounds.max(Axis::Y) - bounds.min(Axis::Y),
        ])
    }
    pub(super) fn r_entity_direction(&self, entity: EntityRef) -> Option<&'static str> {
        Some(facing(self.entity_mob(entity)?.yaw))
    }

    pub(super) fn r_entity_pose(&self, entity: EntityRef) -> Option<&'static str> {
        POSES
            .get(usize::from(crate::mobs::sync::pose_id(
                self.entity_mob(entity)?,
            )))
            .copied()
    }

    pub(super) fn r_entity_is_crouching(&self, entity: EntityRef) -> Option<bool> {
        Some(self.r_entity_pose(entity)? == "crouching")
    }

    pub(super) fn r_entity_view_vector(&self, entity: EntityRef) -> Option<[f64; 3]> {
        let mob = self.entity_mob(entity)?;
        let v = horizon_world::view_direction::view_vector(mob.head_pitch, mob.yaw);
        Some([v.x, v.y, v.z])
    }

    pub(super) fn r_entity_up_vector(&self, entity: EntityRef) -> Option<[f64; 3]> {
        let mob = self.entity_mob(entity)?;
        let v = horizon_world::view_direction::view_vector(mob.head_pitch - 90.0, mob.yaw);
        Some([v.x, v.y, v.z])
    }

    pub(super) fn r_entity_level(&self, entity: EntityRef) -> Option<mod_api::LevelRef> {
        self.entity_mob(entity)?;
        self.host.level(self.w.level.id())
    }

    pub(super) fn r_entity_vehicle(&self, entity: EntityRef) -> Option<EntityRef> {
        let id = self.entity_mob(entity)?.extra.mount?;
        self.lookup_entity(id).or_else(|| self.host.entity(id))
    }

    pub(super) fn r_entity_is_passenger(&self, entity: EntityRef) -> Option<bool> {
        self.entity_mob(entity)?;
        Some(self.r_entity_vehicle(entity).is_some())
    }
    pub(super) fn r_entity_sound_source(&self, entity: EntityRef) -> Option<mod_api::SoundSource> {
        let mob = self.entity_mob(entity)?;
        let evil = mob.species() == crate::mobs::Species::Rabbit
            && mob.extra.variant == crate::mobs::rabbit::EVIL;
        Some(if evil || hostile_species(mob.species()) {
            mod_api::SoundSource::Hostile
        } else {
            mod_api::SoundSource::Neutral
        })
    }

    pub(super) fn r_entity_leash_holder(&self, entity: EntityRef) -> Option<mod_api::LeashHolder> {
        let mob = self.entity_mob(entity)?;
        if mob.extra.leash_pending.is_some() {
            return None;
        }
        use crate::mobs::leash::Holder;
        Some(match mob.extra.leash {
            None => mod_api::LeashHolder::Unleashed,
            Some(Holder::Knot(pos)) => mod_api::LeashHolder::Knot(pos),
            Some(Holder::Mob(id) | Holder::Boat(id)) => mod_api::LeashHolder::Entity(
                self.lookup_entity(id).or_else(|| self.host.entity(id))?,
            ),
            Some(Holder::Player) => {
                let uuid = horizon_client::lan::playerdata::ints_uuid(mob.extra.leash_player?);
                mod_api::LeashHolder::Entity(
                    self.lookup_player(uuid)
                        .or_else(|| self.host.player(uuid))?
                        .entity(),
                )
            }
        })
    }

    pub(super) fn r_entity_pick(
        &self,
        entity: EntityRef,
        distance: f64,
    ) -> Option<mod_api::ClipHit> {
        if !(distance.is_finite() && distance >= 0.0) {
            return None;
        }
        let mob = self.entity_mob(entity)?;
        let from = [mob.position.x, mob.eye_y(), mob.position.z];
        let look = self.r_entity_view_vector(entity)?;
        let to = std::array::from_fn(|i| from[i] + look[i] * distance);
        let level = self.r_entity_level(entity)?;
        mod_api::__private::backend_read(self.host, |backend| {
            backend.level_clip(
                level,
                from,
                to,
                mod_api::ClipBlock::Outline,
                mod_api::ClipFluid::None,
            )
        })
        .flatten()
    }
}
