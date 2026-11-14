//! Navigation helpers for native mod mob handles.
use super::{Game, commands::Target};
use mod_api::{MobRef, PathType};

/// The numbers a mod hands a mob's navigation (`moveTo(x, y, z, speed)`): a
/// finite speed, and finite coordinates small enough for the pathfinder's
/// block arithmetic. The mob handles' boundary, shared with a goal tick's own
/// `move_to` (#1077) — one check, not a copy per caller.
pub(super) fn navigation_move_accepts(at: [f64; 3], speed: f64) -> bool {
    speed.is_finite()
        && at
            .iter()
            .all(|v| v.is_finite() && v.abs() <= 2_000_000_000.0)
}

/// The coordinates a mod hands a mob's look control (`setLookAt(x, y, z)`):
/// finite ones — the same boundary the navigation takes, for the control
/// whose input it shares.
pub(super) fn navigation_look_accepts(at: [f64; 3]) -> bool {
    at.iter().all(|v| v.is_finite())
}

fn navigation_flags_supported(mob: &horizon_client::mobs::Mob) -> bool {
    use horizon_client::mobs::nav::NavKind;
    matches!(
        mob.ai.nav.kind,
        NavKind::Ground | NavKind::WallClimber | NavKind::Flying
    )
}

impl Game<'_> {
    /// `PathNavigation.moveTo(x, y, z, speed)`: true if a path was started.
    /// Refuses a stale or unavailable mob, a client, non-finite arguments,
    /// and coordinates too large for safe block arithmetic. As in vanilla,
    /// the mob's own goals may replace the path next tick. False for the
    /// dragon, which flies by its phase and keeps no navigation.
    pub(super) fn mob_move_to(&mut self, mob: MobRef, at: [f64; 3], speed: f64) -> bool {
        if !navigation_move_accepts(at, speed) {
            return false;
        }
        let Some(Target::Mob(id)) = self.native_entity_target(mob.entity()) else {
            return false;
        };
        let Some(index) = self.mobs.iter().position(|m| m.id == id) else {
            return false;
        };
        // The pathfinder reads terrain only. Take the mob list so its selected
        // mob can be mutably borrowed alongside the shared MobWorld, just as
        // the mob tick does. No mod hook fires during pathfinding.
        let mut mobs = self.mobs.lift();
        #[cfg(test)]
        super::entity::taken_pass_tests::after_take(self);
        let moved = mobs[index].nav_move_to(&self.mob_world_base(&[]), at, speed);
        self.mobs
            .restore(mobs, horizon_client::join_seq::RestoreOrder::SurvivorsFirst);
        moved
    }

    /// `PathNavigation.stop`. False for a stale or unavailable mob, or on a
    /// client, and for the dragon (no navigation).
    pub(super) fn mob_stop(&mut self, mob: MobRef) -> bool {
        let Some(Target::Mob(id)) = self.native_entity_target(mob.entity()) else {
            return false;
        };
        let Some(mob) = self.mobs.iter_mut().find(|m| m.id == id) else {
            return false;
        };
        mob.nav_stop();
        true
    }

    /// `PathNavigation.isDone`; None for a stale or unavailable mob, or on a
    /// client, and for the dragon (no navigation).
    pub(super) fn mob_is_done(&self, mob: MobRef) -> Option<bool> {
        let Target::Mob(id) = self.native_entity_target(mob.entity())? else {
            return None;
        };
        Some(self.mobs.iter().find(|m| m.id == id)?.ai.nav.is_done())
    }

    /// `PathNavigation.getSpeedModifier`: the multiplier the current path
    /// is followed at (1.0 is the mob's normal speed; 0 until a path sets
    /// one). This is not the mob's movement-speed attribute. None for a
    /// stale or unavailable mob, or on a client, and for the dragon.
    pub(super) fn mob_navigation_speed(&self, mob: MobRef) -> Option<f64> {
        let Target::Mob(id) = self.native_entity_target(mob.entity())? else {
            return None;
        };
        Some(self.mobs.iter().find(|m| m.id == id)?.ai.nav.speed_modifier)
    }

    /// The live supported navigation's `can_open_doors` flag; unavailable mobs refuse.
    pub(super) fn mob_can_open_doors(&self, mob: MobRef) -> Option<bool> {
        let Target::Mob(id) = self.native_entity_target(mob.entity())? else {
            return None;
        };
        let mob = self.mobs.iter().find(|m| m.id == id && !m.removed)?;
        navigation_flags_supported(mob).then_some(mob.ai.nav.can_open_doors)
    }

    /// `PathNavigation.setCanOpenDoors`; leaves the current path intact.
    pub(super) fn mob_set_can_open_doors(&mut self, mob: MobRef, value: bool) -> bool {
        let Some(Target::Mob(id)) = self.native_entity_target(mob.entity()) else {
            return false;
        };
        let Some(mob) = self.mobs.iter_mut().find(|m| m.id == id && !m.removed) else {
            return false;
        };
        if !navigation_flags_supported(mob) {
            return false;
        }
        mob.ai.nav.can_open_doors = value;
        true
    }

    /// The live supported navigation's `can_pass_doors` flag; unavailable mobs refuse.
    pub(super) fn mob_can_pass_doors(&self, mob: MobRef) -> Option<bool> {
        let Target::Mob(id) = self.native_entity_target(mob.entity())? else {
            return None;
        };
        let mob = self.mobs.iter().find(|m| m.id == id && !m.removed)?;
        navigation_flags_supported(mob).then_some(mob.ai.nav.can_pass_doors)
    }

    /// Sets the pass-door navigation wish; leaves the current path intact.
    pub(super) fn mob_set_can_pass_doors(&mut self, mob: MobRef, value: bool) -> bool {
        let Some(Target::Mob(id)) = self.native_entity_target(mob.entity()) else {
            return false;
        };
        let Some(mob) = self.mobs.iter_mut().find(|m| m.id == id && !m.removed) else {
            return false;
        };
        if !navigation_flags_supported(mob) {
            return false;
        }
        mob.ai.nav.can_pass_doors = value;
        true
    }

    /// The live supported navigation's `can_float` flag; unavailable mobs refuse.
    pub(super) fn mob_can_float(&self, mob: MobRef) -> Option<bool> {
        let Target::Mob(id) = self.native_entity_target(mob.entity())? else {
            return None;
        };
        let mob = self.mobs.iter().find(|m| m.id == id && !m.removed)?;
        navigation_flags_supported(mob).then_some(mob.ai.nav.can_float)
    }

    /// `PathNavigation.setCanFloat`; leaves the current path intact.
    pub(super) fn mob_set_can_float(&mut self, mob: MobRef, value: bool) -> bool {
        let Some(Target::Mob(id)) = self.native_entity_target(mob.entity()) else {
            return false;
        };
        let Some(mob) = self.mobs.iter_mut().find(|m| m.id == id && !m.removed) else {
            return false;
        };
        if !navigation_flags_supported(mob) {
            return false;
        }
        mob.ai.nav.can_float = value;
        true
    }

    /// `Mob.getPathfindingMalus`: what a node of `kind` costs this mob's
    /// pathfinder — its own override, else the kind's default (a negative
    /// malus closes the kind). None for a stale or unavailable mob, the
    /// dragon, or on a client.
    pub(super) fn mob_pathfinding_malus(&self, mob: MobRef, kind: PathType) -> Option<f32> {
        let Target::Mob(id) = self.native_entity_target(mob.entity())? else {
            return None;
        };
        Some(
            self.mobs
                .iter()
                .find(|m| m.id == id && !m.removed)?
                .malus(kind),
        )
    }

    /// `Mob.setPathfindingMalus`: what the mob's next paths cost through
    /// `kind` (negative closes it, as the species' own overrides do); not
    /// saved, as in vanilla, so a reload or conversion keeps the species'
    /// own. False for a malus that is not finite, a stale or unavailable mob,
    /// the dragon, or on a client.
    pub(super) fn mob_set_pathfinding_malus(
        &mut self,
        mob: MobRef,
        kind: PathType,
        malus: f32,
    ) -> bool {
        if !malus.is_finite() {
            return false;
        }
        let Some(Target::Mob(id)) = self.native_entity_target(mob.entity()) else {
            return false;
        };
        self.mobs
            .iter_mut()
            .find(|m| m.id == id && !m.removed)
            .map(|m| m.set_malus(kind, malus))
            .is_some()
    }

    /// `PathNavigation.setSpeedModifier`: sets the multiplier the current
    /// path is followed at, as `move_to`'s speed argument does. The mob's
    /// own AI — a goal's or a brain species' next `moveTo` — sets its own
    /// again, as in vanilla. False for a non-finite speed, a stale or
    /// unavailable mob, or on a client, and for the dragon.
    pub(super) fn mob_set_navigation_speed(&mut self, mob: MobRef, speed: f64) -> bool {
        if !speed.is_finite() {
            return false;
        }
        let Some(Target::Mob(id)) = self.native_entity_target(mob.entity()) else {
            return false;
        };
        self.mobs
            .iter_mut()
            .find(|m| m.id == id)
            .map(|m| m.ai.nav.speed_modifier = speed)
            .is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::super::native_entity_refs::tests::with_game;
    use horizon_client::{chunk::Chunk, mobs::Mob};
    use horizon_core::geometry::Vec3;
    use mod_api::{EntityRef, MobRef, WorldEpoch};
    use std::sync::Arc;

    #[test]
    #[ignore = "needs prepared resources"]
    fn navigation_starts_a_real_path_and_refuses_stale_handles() {
        crate::game::entity::taken_pass_tests::in_both_dimensions(|game, balance| {
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            for x in 0..16 {
                for z in 0..16 {
                    game.world.set(x, 64, z, stone).unwrap();
                }
            }
            let kind = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            let mut zombie = Mob::new(99, kind, Vec3::new(3.5, 65.0, 3.5), 0.0);
            zombie.on_ground = true;
            game.mobs.push(zombie);
            let entity = game.entity(99).unwrap();
            let mob = game.mob(entity).unwrap();
            assert_eq!(mob.is_done(game), Some(true));
            crate::game::entity::taken_pass_tests::assert_mob_spawn_survives(
                game,
                balance,
                |game| {
                    assert!(mob.move_to(game, [10.5, 65.0, 3.5], 1.25));
                },
            );
            assert_eq!(mob.is_done(game), Some(false));
            assert_eq!(game.mobs[0].ai.nav.speed_modifier, 1.25);
            assert!(!mob.move_to(game, [f64::NAN, 65.0, 3.5], 1.0));
            assert_eq!(mob.is_done(game), Some(false));
            assert!(mob.stop(game));
            assert_eq!(mob.is_done(game), Some(true));
            assert!(!mob.move_to(game, [1000.0, 65.0, 0.0], 1.0));
            let compare_extreme =
                |game: &mut super::super::Game<'_>, extreme: f64, boundary: f64| {
                    let result = mob.move_to(game, [10.5, boundary, 3.5], 1.0);
                    let path = game.mobs[0].ai.nav.path.clone();
                    mob.stop(game);
                    assert_eq!(mob.move_to(game, [10.5, extreme, 3.5], 1.0), result);
                    assert_eq!(game.mobs[0].ai.nav.path, path);
                    mob.stop(game);
                };
            let top = f64::from(game.world.top_y() - 1);
            let bottom = f64::from(game.world.bottom_y());
            compare_extreme(game, 2_000_000_000.0, top);
            compare_extreme(game, -2_000_000_000.0, bottom);
            game.world.shape =
                horizon_client::chunk::LevelShape::new(0, 128, 128, true, false).unwrap();
            assert_eq!(game.world.shape.max_y(), 128);
            compare_extreme(game, 2_000_000_000.0, 127.0);
            compare_extreme(game, -2_000_000_000.0, 0.0);
            game.world.shape = horizon_client::chunk::LevelShape::OVERWORLD;
            let stale = MobRef::new(EntityRef::new(
                WorldEpoch::new(entity.epoch().get() + 1),
                99,
            ));
            assert!(!stale.move_to(game, [5.0, 65.0, 3.0], 1.0));
            assert!(!stale.stop(game));
            assert_eq!(stale.is_done(game), None);
            let taken = game.mobs.lift();
            assert_eq!(mob.is_done(game), None);
            assert!(!mob.stop(game));
            game.mobs.restore(
                taken,
                horizon_client::join_seq::RestoreOrder::SurvivorsFirst,
            );
            game.armor_stands
                .push(horizon_client::entities::ArmorStand::new(
                    102,
                    Vec3::ZERO,
                    0.0,
                ));
            let wrong = MobRef::new(game.entity(102).unwrap());
            assert!(!wrong.move_to(game, [5.0, 65.0, 3.0], 1.0));
            assert!(!wrong.stop(game));
            assert_eq!(wrong.is_done(game), None);
            let parrot = game
                .res
                .mob_types()
                .get("minecraft:parrot")
                .unwrap()
                .clone();
            game.mobs
                .push(Mob::new(100, parrot, Vec3::new(3.5, 66.0, 5.5), 0.0));
            let flying = game.mob(game.entity(100).unwrap()).unwrap();
            assert!(flying.move_to(game, [9.5, 66.0, 5.5], 1.0));
            assert_eq!(flying.is_done(game), Some(false));
            let cod = game.res.mob_types().get("minecraft:cod").unwrap().clone();
            game.mobs
                .push(Mob::new(101, cod, Vec3::new(3.5, 65.0, 7.5), 0.0));
            let fish = game.mob(game.entity(101).unwrap()).unwrap();
            assert!(!fish.move_to(game, [9.5, 65.0, 7.5], 1.0));
            assert_eq!(fish.is_done(game), Some(true));
        });
    }

    #[test]
    #[ignore = "needs prepared resources"]
    fn navigation_speed_is_the_paths_speed_modifier() {
        with_game(|game| {
            let kind = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs
                .push(Mob::new(99, kind, Vec3::new(3.5, 65.0, 3.5), 0.0));
            let entity = game.entity(99).unwrap();
            let mob = game.mob(entity).unwrap();
            // A fresh navigation has no path, so no speed was set for one.
            assert_eq!(mob.navigation_speed(game), Some(0.0));
            assert!(mob.set_navigation_speed(game, 1.25));
            assert_eq!(mob.navigation_speed(game), Some(1.25));
            assert_eq!(game.mobs[0].ai.nav.speed_modifier, 1.25);
            assert!(!mob.set_navigation_speed(game, f64::NAN));
            assert_eq!(game.mobs[0].ai.nav.speed_modifier, 1.25);
            // Not the movement-speed attribute.
            let attribute = game.mobs[0].movement_speed();
            assert_ne!(attribute, 1.25);
            let stale = MobRef::new(EntityRef::new(
                WorldEpoch::new(entity.epoch().get() + 1),
                99,
            ));
            assert_eq!(stale.navigation_speed(game), None);
            assert!(!stale.set_navigation_speed(game, 1.0));
            assert_eq!(game.mobs[0].ai.nav.speed_modifier, 1.25);
        });
    }
}
