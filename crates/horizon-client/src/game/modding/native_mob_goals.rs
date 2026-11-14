//! AI goals of native mod mob handles.
use super::native_namespaced_id::namespaced;
use super::{Game, commands::Target};
use horizon_client::mobs::{
    self, Species,
    goals::{
        Avoid, AvoidEntity, BreathAir, Breed, ClimbOnTopOfPowderSnow, Cx, EatBlock, FleeSun, Float,
        FollowMob, FollowParent, Goal, HurtByTarget, LeapAtTarget, LookAtPlayer, MeleeAttack,
        NearestTarget, Panic, Prey, RandomLookAround, RestrictSun, Stroll, Tempt, TryFindWater,
    },
    nav::NavKind,
};
use mod_api::{GoalSpec, MobFilter, MobGoalEntry, MobRef};

/// The goal's kind id ([`GoalSpec::id`]) of a registered goal, by its
/// internal name: the plain vanilla kinds map to their snake_case id, and a
/// species variant of one (a bee's `BeeHurtByOtherGoal`, a labeled stroll)
/// keeps its own vanilla name, as `goals` reads it and `remove_goal` takes
/// it to remove that one.
pub(super) fn goal_id(name: &str) -> &str {
    match name {
        "Float" => "float",
        "ClimbOnTopOfPowderSnow" => "climb_on_top_of_powder_snow",
        "Panic" => "panic",
        "Stroll" => "stroll",
        "LookAtPlayer" => "look_at_player",
        "RandomLookAround" => "random_look_around",
        "MeleeAttack" => "melee_attack",
        "LeapAtTarget" => "leap_at_target",
        "AvoidEntity" => "avoid_entity",
        "Tempt" => "tempt",
        "Breed" => "breed",
        "FollowParent" => "follow_parent",
        "FollowMobGoal" => "follow_mob",
        "RestrictSun" => "restrict_sun",
        "FleeSun" => "flee_sun",
        "EatBlock" => "eat_block",
        "BreathAir" => "breath_air",
        "TryFindWater" => "try_find_water",
        "NearestTarget" => "nearest_target",
        "HurtByTarget" => "hurt_by_target",
        other => other,
    }
}

/// The supported `Mob` classes that do not inherit `PathfinderMob`.
fn pathfinder(species: Species) -> bool {
    !matches!(species, Species::Bat | Species::Ghast | Species::Phantom)
}

impl Game<'_> {
    /// `GoalSelector.addGoal(priority, goal)` on a mob's goal selector.
    /// Refuses a stale or unavailable mob, a client, and the dragon (it
    /// keeps no selector); a goal the mob's vanilla class does not hold (a
    /// `PathfinderMob` goal for a bat, ghast or phantom, `BreedGoal` or
    /// `FollowParentGoal` for a mob that is no animal); and non-finite
    /// numbers, negative speeds and distances, a probability outside
    /// 0..=1, a negative interval, and an unknown species id (in `prey`,
    /// `what` or `only`) or empty `items`. The target selector's kinds
    /// (`nearest_target`, `hurt_by_target`) go in through
    /// `add_target_goal`; both selectors take them, as vanilla's two
    /// selectors do.
    pub(super) fn mob_add_goal(&mut self, mob: MobRef, priority: i32, goal: &GoalSpec) -> bool {
        let Some(Target::Mob(id)) = self.native_entity_target(mob.entity()) else {
            return false;
        };
        let Some((species, navigation)) = self
            .mobs
            .iter()
            .find(|m| m.id == id)
            .map(|m| (m.species(), m.ai.nav.kind))
        else {
            return false;
        };
        let Some(goal) = self.boxed_goal(species, navigation, goal) else {
            return false;
        };
        let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) else {
            return false;
        };
        m.ai.goals.add_boxed(priority, goal);
        true
    }

    /// `GoalSelector.addGoal` on the mob's target selector. The same rules
    /// as `add_goal`.
    pub(super) fn mob_add_target_goal(
        &mut self,
        mob: MobRef,
        priority: i32,
        goal: &GoalSpec,
    ) -> bool {
        let Some(Target::Mob(id)) = self.native_entity_target(mob.entity()) else {
            return false;
        };
        let Some((species, navigation)) = self
            .mobs
            .iter()
            .find(|m| m.id == id)
            .map(|m| (m.species(), m.ai.nav.kind))
        else {
            return false;
        };
        let Some(goal) = self.boxed_goal(species, navigation, goal) else {
            return false;
        };
        let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) else {
            return false;
        };
        m.ai.targets.add_boxed(priority, goal);
        true
    }

    /// `GoalSelector.removeGoal` on the goal selector, by kind: every goal
    /// whose id, as `goals` reads it, is `id` goes (`removeAllGoals`), a
    /// running one stopped first. How many went; None for a stale or
    /// unavailable mob, a client, and the dragon. Nothing in a mob's tick
    /// resolves, as its copy is held out there: add what a mod needs in
    /// `entity_join`, which fires for a loaded chunk's mobs too.
    pub(super) fn mob_remove_goal(&mut self, mob: MobRef, id: &str) -> Option<usize> {
        let Some(Target::Mob(entity)) = self.native_entity_target(mob.entity()) else {
            return None;
        };
        let index = self.mobs.iter().position(|m| m.id == entity)?;
        Some(self.remove_from_selector(index, id, false))
    }

    /// `GoalSelector.removeGoal` on the target selector, by kind, as
    /// `remove_goal` does on the goal selector.
    pub(super) fn mob_remove_target_goal(&mut self, mob: MobRef, id: &str) -> Option<usize> {
        let Some(Target::Mob(entity)) = self.native_entity_target(mob.entity()) else {
            return None;
        };
        let index = self.mobs.iter().position(|m| m.id == entity)?;
        Some(self.remove_from_selector(index, id, true))
    }

    /// `GoalSelector.getRunningGoals` read wider: every goal of the goal
    /// selector as a `MobGoalEntry` (the priority, the kind id and
    /// whether it runs), in registration order. None for a stale or
    /// unavailable mob, a client, and the dragon.
    pub(super) fn mob_goals(&self, mob: MobRef) -> Option<Vec<MobGoalEntry>> {
        let Some(Target::Mob(id)) = self.native_entity_target(mob.entity()) else {
            return None;
        };
        let m = self.mobs.iter().find(|m| m.id == id)?;
        Some(
            m.ai.goals
                .entries()
                .map(|(priority, name, running)| MobGoalEntry {
                    priority,
                    id: goal_id(name),
                    running,
                })
                .collect(),
        )
    }

    /// Every goal of the target selector, as `goals` reads the goal
    /// selector's.
    pub(super) fn mob_target_goals(&self, mob: MobRef) -> Option<Vec<MobGoalEntry>> {
        let Some(Target::Mob(id)) = self.native_entity_target(mob.entity()) else {
            return None;
        };
        let m = self.mobs.iter().find(|m| m.id == id)?;
        Some(
            m.ai.targets
                .entries()
                .map(|(priority, name, running)| MobGoalEntry {
                    priority,
                    id: goal_id(name),
                    running,
                })
                .collect(),
        )
    }

    /// One `removeGoal` pass over the mob's selector: the mob list comes
    /// out, as the mob tick takes it, so the goal's `stop` sees the world
    /// the tick's `Cx` sees; no hook fires in between. The events a stop
    /// makes land as any tick's do.
    fn remove_from_selector(&mut self, index: usize, id: &str, targets: bool) -> usize {
        let snapshot: Vec<_> = self
            .mobs
            .iter()
            .map(|m| super::creatures::seen(m, self.level.player_uuid, self.items()))
            .collect();
        let mut mobs = self.mobs.lift();
        #[cfg(test)]
        super::entity::taken_pass_tests::after_take(self);
        let mut lent = horizon_client::health::LentLevel::take(&mut self.world);
        let mut w = self.mob_world_base(&snapshot);
        let pos = mobs[index].position;
        w.difficulty = self.local_difficulty([pos.x, pos.y, pos.z]);
        let mut events = Vec::new();
        let removed = {
            let mut level = horizon_client::health::DamageLevel::lent(
                &self.world,
                self.pack.assets.registry,
                &mut lent,
                Some(world_items!(self)),
                Some(&self.pack.block_registries),
                &self.damage_hooks,
            );
            let m = &mut mobs[index];
            let mut selector = if targets {
                std::mem::take(&mut m.ai.targets)
            } else {
                std::mem::take(&mut m.ai.goals)
            };
            let mut cx = Cx {
                w: &w,
                level: &mut level,
                events: &mut events,
            };
            let removed = selector.remove_all(|g| goal_id(g.name()) == id, m, &mut cx);
            if targets {
                mobs[index].ai.targets = selector;
            } else {
                mobs[index].ai.goals = selector;
            }
            mobs::apply_brain_events(&mut mobs, &mut level, &mut events, 0);
            removed
        };
        drop(w);
        lent.give_back(&mut self.world);
        self.mobs
            .restore(mobs, horizon_client::join_seq::RestoreOrder::SurvivorsFirst);
        self.handle_mob_events(events).ok();
        removed
    }

    /// The goal `goal` names, built as the species code builds it, for a
    /// mob of `species`: None when the numbers are not ones vanilla's
    /// constructor takes or the mob's class does not hold the goal. The
    /// species ids of a filter resolve through the mob type table, a bare
    /// id in `minecraft:`'s, as the effects' ids read.
    fn boxed_goal(
        &self,
        species: Species,
        navigation: NavKind,
        goal: &GoalSpec,
    ) -> Option<Box<dyn Goal>> {
        let needs_pathfinder = !matches!(
            goal,
            GoalSpec::Float
                | GoalSpec::ClimbOnTopOfPowderSnow
                | GoalSpec::LookAtPlayer { .. }
                | GoalSpec::RandomLookAround
                | GoalSpec::LeapAtTarget { .. }
                | GoalSpec::EatBlock
                | GoalSpec::FollowMob { .. }
                | GoalSpec::NearestTarget { .. }
        );
        if needs_pathfinder && !pathfinder(species) {
            return None;
        }
        if matches!(goal, GoalSpec::FollowMob { .. })
            && !matches!(
                navigation,
                NavKind::Ground | NavKind::WallClimber | NavKind::Flying
            )
        {
            return None;
        }
        let animal_only = matches!(goal, GoalSpec::Breed { .. } | GoalSpec::FollowParent { .. });
        if animal_only && !mobs::drops::is_animal(species) {
            return None;
        }
        match goal {
            GoalSpec::Float => Some(Box::new(Float)),
            GoalSpec::ClimbOnTopOfPowderSnow => Some(Box::new(ClimbOnTopOfPowderSnow)),
            GoalSpec::Panic { speed, water_range } => {
                if !speed.is_finite() || *speed < 0.0 || water_range.is_some_and(|r| r < 0) {
                    return None;
                }
                let mut g = Panic::new(*speed);
                g.water_range = *water_range;
                Some(Box::new(g))
            }
            GoalSpec::Stroll {
                speed,
                interval,
                water_avoiding,
                swim,
                flying,
            } => {
                if !speed.is_finite()
                    || *speed < 0.0
                    || *interval < 0
                    || water_avoiding.is_some_and(|p| !p.is_finite() || !(0.0..=1.0).contains(&p))
                {
                    return None;
                }
                let mut g = Stroll::new(*speed);
                g.interval = *interval;
                g.water_avoiding = *water_avoiding;
                g.swim = *swim;
                g.flying = *flying;
                Some(Box::new(g))
            }
            GoalSpec::LookAtPlayer {
                range,
                probability,
                only_horizontal,
                mobs,
                only,
            } => {
                if !range.is_finite()
                    || *range < 0.0
                    || !probability.is_finite()
                    || !(0.0..=1.0).contains(probability)
                {
                    return None;
                }
                let only = match only {
                    None => None,
                    Some(id) => Some(self.species_of(id)?),
                };
                let mut g = if *mobs {
                    LookAtPlayer::mobs(*range)
                } else {
                    LookAtPlayer::new(*range)
                };
                g.probability = *probability;
                g.only_horizontal = *only_horizontal;
                g.only = only;
                Some(Box::new(g))
            }
            GoalSpec::RandomLookAround => Some(Box::new(RandomLookAround::default())),
            GoalSpec::MeleeAttack {
                speed,
                follow_unseen,
            } => {
                if !speed.is_finite() || *speed < 0.0 {
                    return None;
                }
                Some(Box::new(MeleeAttack::new(*speed, *follow_unseen)))
            }
            GoalSpec::LeapAtTarget { yd } => {
                if !yd.is_finite() {
                    return None;
                }
                Some(Box::new(LeapAtTarget::new(*yd)))
            }
            GoalSpec::AvoidEntity {
                what,
                max_dist,
                walk,
                sprint,
            } => {
                if !max_dist.is_finite()
                    || *max_dist < 0.0
                    || !walk.is_finite()
                    || *walk < 0.0
                    || !sprint.is_finite()
                    || *sprint < 0.0
                {
                    return None;
                }
                let what = match what {
                    MobFilter::Player => Avoid::Player,
                    MobFilter::Species(ids) => Avoid::Species(self.species_list(ids)?),
                    _ => unreachable!("&MobFilter: unhandled variant"),
                };
                Some(Box::new(AvoidEntity::new(what, *max_dist, *walk, *sprint)))
            }
            GoalSpec::Tempt {
                speed,
                items,
                can_scare,
            } => {
                if !speed.is_finite() || *speed < 0.0 || items.is_empty() {
                    return None;
                }
                let items: Vec<String> = items.iter().map(|id| namespaced(id)).collect::<Vec<_>>();
                Some(Box::new(Tempt::new(*speed, items.into(), *can_scare)))
            }
            GoalSpec::Breed { speed } => {
                if !speed.is_finite() || *speed < 0.0 {
                    return None;
                }
                Some(Box::new(Breed::new(*speed)))
            }
            GoalSpec::FollowParent { speed } => {
                if !speed.is_finite() || *speed < 0.0 {
                    return None;
                }
                Some(Box::new(FollowParent::new(*speed)))
            }
            GoalSpec::FollowMob { speed, stop, area } => {
                if !speed.is_finite()
                    || *speed < 0.0
                    || !stop.is_finite()
                    || *stop < 0.0
                    || !area.is_finite()
                    || *area < 0.0
                {
                    return None;
                }
                Some(Box::new(FollowMob::new(*speed, *stop, *area)))
            }
            GoalSpec::RestrictSun => Some(Box::new(RestrictSun)),
            GoalSpec::FleeSun { speed } => {
                if !speed.is_finite() || *speed < 0.0 {
                    return None;
                }
                Some(Box::new(FleeSun::new(*speed)))
            }
            GoalSpec::EatBlock => Some(Box::new(EatBlock::default())),
            GoalSpec::BreathAir => Some(Box::new(BreathAir)),
            GoalSpec::TryFindWater => Some(Box::new(TryFindWater::default())),
            GoalSpec::NearestTarget {
                prey,
                must_see,
                must_reach,
                interval,
            } => {
                if *interval < 0 {
                    return None;
                }
                let prey = match prey {
                    MobFilter::Player => Prey::Player,
                    MobFilter::Species(ids) => Prey::Species(self.species_list(ids)?),
                    _ => unreachable!("&MobFilter: unhandled variant"),
                };
                let mut g = NearestTarget::new(prey, *must_see);
                g.must_reach = *must_reach;
                g.interval = *interval;
                Some(Box::new(g))
            }
            GoalSpec::HurtByTarget { alert } => {
                let mut g = HurtByTarget::new();
                g.alert = *alert;
                Some(Box::new(g))
            }
            _ => unreachable!("&GoalSpec: unhandled variant"),
        }
    }

    /// Every species id of a filter, all of them known; None when one is
    /// not. Bare ids read as `minecraft:`'s.
    fn species_list(&self, ids: &[String]) -> Option<Vec<Species>> {
        if ids.is_empty() {
            return None;
        }
        ids.iter().map(|id| self.species_of(id)).collect()
    }

    /// The species of the mob type `id` names (`minecraft:zombie`).
    fn species_of(&self, id: &str) -> Option<Species> {
        self.res.mob_types().get(&namespaced(id)).map(|t| t.species)
    }
}

#[cfg(test)]
mod tests {
    use super::super::native_entity_refs::tests::with_game;
    use horizon_client::{chunk::Chunk, mobs::Mob};
    use horizon_core::geometry::Vec3;
    use mod_api::{EntityRef, GoalSpec, MobFilter, MobRef, WorldEpoch};
    use std::sync::Arc;

    /// A mob of `kind` in the game's list, its species goals registered by
    /// `Mob::new`, as a handle.
    fn mob_in_game(game: &mut super::super::Game<'_>, kind: &str, id: u32) -> MobRef {
        let t = game.res.mob_types().get(kind).unwrap().clone();
        let mut m = Mob::new(id, t, Vec3::new(3.5, 65.0, 3.5), 0.0);
        m.on_ground = true;
        game.mobs.push(m);
        let entity = game.entity(id).unwrap();
        game.mob(entity).unwrap()
    }

    /// Whether the mob's goal selector holds a goal of kind `id`.
    fn has(game: &super::super::Game<'_>, mob: MobRef, id: &str) -> bool {
        mob.goals(game).unwrap().iter().any(|e| e.id == id)
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn goal_constructors_follow_vanilla_classes() {
        use horizon_client::mobs::Species;
        with_game(|game| {
            let mut mismatches = Vec::new();
            for (species, accepts) in [
                (Species::Slime, true),
                (Species::MagmaCube, true),
                (Species::Shulker, true),
                (Species::Ghast, false),
                (Species::Phantom, false),
                (Species::Bat, false),
            ] {
                let actual = game
                    .boxed_goal(
                        species,
                        super::NavKind::Ground,
                        &GoalSpec::Panic {
                            speed: 1.0,
                            water_range: None,
                        },
                    )
                    .is_some();
                if actual != accepts {
                    mismatches.push(format!(
                        "{species:?}: accepted {actual}, expected {accepts}"
                    ));
                }
            }
            assert!(mismatches.is_empty(), "{}", mismatches.join("; "));
            for species in [Species::Bat, Species::Ghast, Species::Phantom] {
                assert!(
                    game.boxed_goal(species, super::NavKind::Ground, &GoalSpec::EatBlock)
                        .is_some()
                );
                assert!(
                    game.boxed_goal(species, super::NavKind::Ground, &GoalSpec::RestrictSun)
                        .is_none()
                );
                assert!(
                    game.boxed_goal(
                        species,
                        super::NavKind::Ground,
                        &GoalSpec::HurtByTarget { alert: false }
                    )
                    .is_none()
                );
                assert!(
                    game.boxed_goal(
                        species,
                        super::NavKind::Ground,
                        &GoalSpec::FollowMob {
                            speed: 1.0,
                            stop: 2.0,
                            area: 8.0
                        }
                    )
                    .is_some()
                );
            }
            for species in [
                Species::Cod,
                Species::Dolphin,
                Species::Guardian,
                Species::Frog,
                Species::Drowned,
                Species::Turtle,
                Species::Axolotl,
            ] {
                assert!(
                    game.boxed_goal(
                        species,
                        super::NavKind::Water { breach: false },
                        &GoalSpec::FollowMob {
                            speed: 1.0,
                            stop: 2.0,
                            area: 8.0
                        }
                    )
                    .is_none()
                );
            }
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn follow_mob_uses_the_mobs_actual_navigation() {
        with_game(|game| {
            let mob = mob_in_game(game, "minecraft:cow", 501);
            let goal = GoalSpec::FollowMob {
                speed: 1.0,
                stop: 2.0,
                area: 8.0,
            };
            for nav in [
                super::NavKind::Water { breach: false },
                super::NavKind::Amphibious {
                    prefers_shallow: false,
                },
            ] {
                game.mobs[0].ai.nav.kind = nav;
                assert!(!game.mob_add_goal(mob, 0, &goal));
                assert!(!game.mob_add_target_goal(mob, 0, &goal));
            }
            for nav in [
                super::NavKind::Ground,
                super::NavKind::WallClimber,
                super::NavKind::Flying,
            ] {
                game.mobs[0].ai.nav.kind = nav;
                assert!(game.mob_add_goal(mob, 0, &goal));
                assert!(game.mob_add_target_goal(mob, 0, &goal));
            }
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn removing_running_goal_supplies_snapshot_and_persists_level_random() {
        use horizon_client::mobs::goals::{Cx, Goal};
        use horizon_core::random::RandomSource;
        #[derive(Clone, Debug)]
        struct StopProbe;
        impl Goal for StopProbe {
            fn flags(&self) -> u8 {
                0
            }
            fn can_use(&mut self, _: &mut Mob, _: &mut Cx) -> bool {
                true
            }
            fn box_clone(&self) -> Box<dyn Goal> {
                Box::new(self.clone())
            }
            fn stop(&mut self, m: &mut Mob, cx: &mut Cx) {
                m.extra.followers = cx.w.others.len() as i32;
                m.health = cx.level.random().next_int_bound(1000).unwrap() as f32;
            }
        }
        with_game(|game| {
            let handle = mob_in_game(game, "minecraft:zombie", 99);
            mob_in_game(game, "minecraft:zombie", 100);
            let mut mobs = game.mobs.lift();
            let snapshot: Vec<_> = mobs
                .iter()
                .map(|m| super::super::creatures::seen(m, game.level.player_uuid, game.items()))
                .collect();
            let mut test_level = horizon_client::health::TestLevel::new(0);
            let mut level = test_level.level();
            let mut events = Vec::new();
            let mut selector = std::mem::take(&mut mobs[0].ai.goals);
            selector.clear();
            selector.add(0, StopProbe);
            {
                let world = game.mob_world_base(&snapshot);
                let mut cx = Cx {
                    w: &world,
                    level: &mut level,
                    events: &mut events,
                };
                selector.tick(&mut mobs[0], &mut cx);
            }
            mobs[0].ai.goals = selector;
            game.mobs
                .restore(mobs, horizon_client::join_seq::RestoreOrder::SurvivorsFirst);
            let mut expected = game.world.random.clone();
            let draw = expected.next_int_bound(1000).unwrap();
            let next = expected.next_int_bound(1000).unwrap();
            assert_eq!(handle.remove_goal(game, "StopProbe"), Some(1));
            assert_eq!(game.mobs[0].extra.followers, 2);
            assert_eq!(game.mobs[0].health, draw as f32);
            assert_eq!(game.world.random.next_int_bound(1000).unwrap(), next);
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn removing_hold_ground_notifies_nearby_raiders() {
        use horizon_client::mobs::{
            Who,
            goals::Cx,
            raider::{self, HoldGroundAttack},
        };
        with_game(|game| {
            let handle = mob_in_game(game, "minecraft:pillager", 99);
            mob_in_game(game, "minecraft:pillager", 100);
            let mut mobs = game.mobs.lift();
            raider::state_mut(&mut mobs[0]).patrolling = true;
            mobs[0].ai.target = Some(Who::Player);
            let snapshot: Vec<_> = mobs
                .iter()
                .map(|m| super::super::creatures::seen(m, game.level.player_uuid, game.items()))
                .collect();
            let mut test_level = horizon_client::health::TestLevel::new(0);
            let mut level = test_level.level();
            let mut events = Vec::new();
            let mut selector = std::mem::take(&mut mobs[0].ai.goals);
            selector.clear();
            selector.add(0, HoldGroundAttack::new(8.0));
            {
                let world = game.mob_world_base(&snapshot);
                let mut cx = Cx {
                    w: &world,
                    level: &mut level,
                    events: &mut events,
                };
                selector.tick(&mut mobs[0], &mut cx);
            }
            assert!(selector.is_running("HoldGroundAttack"));
            mobs[0].ai.goals = selector;
            game.mobs
                .restore(mobs, horizon_client::join_seq::RestoreOrder::SurvivorsFirst);
            assert_eq!(handle.remove_goal(game, "HoldGroundAttack"), Some(1));
            assert!(game.mobs[1].ai.aggressive);
            assert_eq!(game.mobs[1].ai.target, Some(Who::Player));
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn removing_school_goal_updates_the_leader() {
        use horizon_client::mobs::{aquatic::FollowFlockLeader, goals::Cx};
        with_game(|game| {
            let follower = mob_in_game(game, "minecraft:cod", 99);
            let _leader = mob_in_game(game, "minecraft:cod", 100);
            let mut mobs = game.mobs.lift();
            mobs[0].extra.leader = Some(100);
            mobs[1].extra.followers = 1;
            let snapshot: Vec<_> = mobs
                .iter()
                .map(|m| m.seen(game.level.player_uuid, game.items()))
                .collect();
            let mut test_level = horizon_client::health::TestLevel::new(0);
            let mut level = test_level.level();
            let mut events = Vec::new();
            let mut selector = std::mem::take(&mut mobs[0].ai.goals);
            selector.clear();
            selector.add(0, FollowFlockLeader::new(0));
            {
                let world = game.mob_world_base(&snapshot);
                let mut context = Cx {
                    w: &world,
                    level: &mut level,
                    events: &mut events,
                };
                selector.tick(&mut mobs[0], &mut context);
            }
            assert!(selector.is_running("FollowFlockLeaderGoal"));
            assert!(
                events.is_empty(),
                "starting an established follower must not change the school"
            );
            mobs[0].ai.goals = selector;
            game.mobs
                .restore(mobs, horizon_client::join_seq::RestoreOrder::SurvivorsFirst);
            assert_eq!(follower.remove_goal(game, "FollowFlockLeaderGoal"), Some(1));
            assert_eq!(
                game.mobs[0].extra.leader, None,
                "the actual stop method must run"
            );
            assert_eq!(
                game.mobs[1].extra.followers, 0,
                "stopping a follower must decrement its leader"
            );
        });
    }

    #[test]
    #[ignore = "needs prepared resources"]
    fn goals_read_add_remove_and_refuse_what_the_mob_does_not_hold() {
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
            let zombie = mob_in_game(game, "minecraft:zombie", 99);
            let slime = mob_in_game(game, "minecraft:slime", 100);
            let cow = mob_in_game(game, "minecraft:cow", 101);
            // The species set reads in the canonical ids.
            assert!(has(game, zombie, "melee_attack"));
            assert!(has(game, zombie, "look_at_player"));
            let targets = zombie.target_goals(game).unwrap();
            assert!(targets.iter().any(|e| e.id == "nearest_target"));
            assert!(targets.iter().any(|e| e.id == "hurt_by_target"));
            // Add: a Panic the zombie did not have.
            assert!(zombie.add_goal(
                game,
                4,
                &GoalSpec::Panic {
                    speed: 1.4,
                    water_range: None,
                }
            ));
            assert!(
                zombie
                    .goals(game)
                    .unwrap()
                    .iter()
                    .any(|e| { e.priority == 4 && e.id == "panic" && !e.running })
            );
            // The target selector takes a species filter.
            assert!(zombie.add_target_goal(
                game,
                3,
                &GoalSpec::NearestTarget {
                    prey: MobFilter::Species(vec!["minecraft:skeleton".into()]),
                    must_see: true,
                    must_reach: false,
                    interval: 10,
                }
            ));
            // Refusals: non-finite speed, a mob with no PathfinderMob goals,
            // a goal only an animal holds, an unknown species id.
            assert!(!zombie.add_goal(
                game,
                4,
                &GoalSpec::Panic {
                    speed: f64::NAN,
                    water_range: None,
                }
            ));
            assert!(slime.add_goal(
                game,
                4,
                &GoalSpec::Panic {
                    speed: 1.0,
                    water_range: None,
                }
            ));
            assert!(slime.add_goal(
                game,
                4,
                &GoalSpec::MeleeAttack {
                    speed: 1.0,
                    follow_unseen: false,
                }
            ));
            assert!(slime.add_goal(game, 4, &GoalSpec::Float));
            assert!(!zombie.add_goal(game, 4, &GoalSpec::Breed { speed: 1.0 }));
            assert!(cow.add_goal(game, 4, &GoalSpec::Breed { speed: 1.0 }));
            assert!(!zombie.add_target_goal(
                game,
                3,
                &GoalSpec::NearestTarget {
                    prey: MobFilter::Species(vec!["minecraft:unknown".into()]),
                    must_see: true,
                    must_reach: false,
                    interval: 10,
                }
            ));
            // Remove: the species' melee attack goes, and only that one.
            crate::game::entity::taken_pass_tests::assert_mob_spawn_survives(
                game,
                balance,
                |game| {
                    assert_eq!(zombie.remove_goal(game, "melee_attack"), Some(1));
                },
            );
            assert!(!has(game, zombie, "melee_attack"));
            assert_eq!(zombie.remove_goal(game, "melee_attack"), Some(0));
            assert_eq!(zombie.remove_target_goal(game, "hurt_by_target"), Some(1));
            // A stale handle and a mob whose copy is held out.
            let stale = MobRef::new(EntityRef::new(WorldEpoch::new(999), 99));
            assert!(!stale.add_goal(game, 1, &GoalSpec::Float));
            assert!(stale.goals(game).is_none());
            let taken = game.mobs.lift();
            assert!(zombie.goals(game).is_none());
            assert_eq!(zombie.remove_goal(game, "panic"), None);
            game.mobs.restore(
                taken,
                horizon_client::join_seq::RestoreOrder::SurvivorsFirst,
            );
        });
    }
}
