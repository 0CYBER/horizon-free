//! The game's own half of `Mob.finalizeSpawn`: `Game` as a
//! [`finalize::FinalizeLevel`], so the live level finalizes mobs through the
//! same [`finalize::Ctx`] world generation does (`mobs::finalize`).
//!
//! `Ctx.random` is the level's own random (`world.random`): vanilla draws
//! finalizeSpawn's rolls from `level.getRandom()` (`Mob.finalizeSpawn` the
//! follow-range triangle and the left-handed roll, `NaturalSpawner` the yaw
//! just before the call), never from the mob's own random. The game used to
//! draw them from each mob's own random in `mobs::spawning::finalize`; this
//! is that divergence removed.

use super::Game;
use horizon_client::collisions::{SceneCollisions, no_collision_empty};
use horizon_client::mobs::{Mob, Species, finalize, spawn_rules::Reason, spawning};
use horizon_core::geometry::{Aabb, Vec3};
use horizon_world::food::Difficulty;

/// `Game` seen by a finalizeSpawn `Ctx`.
pub(super) struct FinalizeGame<'a, 'g> {
    pub(super) game: &'a mut Game<'g>,
}

/// A finalizeSpawn biome lookup with its chunk not loaded: the plains' id,
/// the same default the biome facts fall back to.
pub(super) fn biome_or_plains(name: Option<String>) -> String {
    name.unwrap_or_else(|| "minecraft:plains".to_owned())
}

impl FinalizeGame<'_, '_> {
    /// The biome id at a block, when its chunk is loaded.
    fn biome_name(&self, pos: [i32; 3]) -> Option<String> {
        let chunk = self.game.world.chunks.get(&(pos[0] >> 4, pos[2] >> 4))?;
        Some(
            world_biomes!(self.game)
                .get(chunk.biome(pos[0], pos[1], pos[2]))
                .name
                .clone(),
        )
    }
}

impl finalize::FinalizeLevel for FinalizeGame<'_, '_> {
    fn mob_maps(&self) -> &horizon_client::neoforge_mob_maps::NeoForgeMobMaps {
        self.game.mob_maps()
    }
    fn biome(&self, pos: [i32; 3]) -> String {
        // A spawn site without its chunk (a loaded marker mob) is neutral.
        biome_or_plains(self.biome_name(pos))
    }
    fn biome_tag(&self, tag: &str, pos: [i32; 3]) -> bool {
        self.biome_name(pos)
            .is_some_and(|b| self.game.res.mob_data().biome_has(tag, &b))
    }
    fn structures_with_piece_at(&self, pos: [i32; 3]) -> Vec<String> {
        self.game.structures_at(pos)
    }
    fn moon_phase(&self, _: [i32; 3]) -> usize {
        self.game.finalize_moon_phase()
    }
    fn spawning_monsters(&self) -> bool {
        self.game.spawning_monsters()
    }
    /// `Level.noCollision(null, box)`: block collisions, and living shulkers
    /// are solid (`getEntityCollisions`).
    fn no_collision(&self, bounds: Aabb) -> bool {
        let mut scene = SceneCollisions {
            world: &self.game.world,
            binding: self.game.res.collision(),
            subject: None,
        };
        let grown = bounds.inflate(Vec3::new(1e-7, 1e-7, 1e-7));
        no_collision_empty(&mut scene, bounds)
            && !self.game.mobs.iter().any(|m| {
                m.species() == Species::Shulker && m.alive() && m.bounds().intersects(grown)
            })
    }
    fn free_chicken(&self, bounds: Aabb) -> Option<u32> {
        self.game.finalize_free_chicken(bounds)
    }
    fn ride_chicken(&mut self, chicken: u32, rider: u32) {
        if let Some(c) = self.game.mobs.iter_mut().find(|m| m.id == chicken) {
            c.extra.jockey = true;
            c.extra.rider = Some(rider);
        }
    }
    fn game_time(&self) -> i64 {
        self.game.finalize_game_time()
    }
    fn halloween(&self) -> bool {
        self.game.finalize_halloween()
    }
    /// `EntityType.create(level, reason)` over the live level: a new id and
    /// the type's own table entry, or None where the type cannot exist.
    fn spawn_event(&mut self, mob: &mut Mob, reason: Reason) -> finalize::SpawnEvent {
        self.game.finalize_spawn_event_result(mob, reason)
    }
    fn create(&mut self, kind: &str, _: Reason) -> Option<Mob> {
        if self.game.world.health_rules.difficulty == Difficulty::Peaceful
            && !finalize::allowed_in_peaceful(kind)
        {
            return None;
        }
        let t = self.game.res.mob_types().get(kind)?.clone();
        self.game.next_entity += 1;
        Some(Mob::new(self.game.next_entity, t, Vec3::ZERO, 0.0))
    }
    fn spawn_data(&self) -> finalize::SpawnData<'_> {
        self.game.finalize_spawn_data()
    }
}

impl Game<'_> {
    /// `DimensionType.moonPhase` at the level's day time.
    pub(super) fn finalize_moon_phase(&self) -> usize {
        (self.world.day_time.div_euclid(24000).rem_euclid(8)) as usize
    }
    /// The level's game time (`Level.getDayTime`).
    pub(super) fn finalize_game_time(&self) -> i64 {
        self.world.day_time
    }
    /// The first free chicken a jockey can take: alive, unhindered, inside
    /// the bounds.
    pub(super) fn finalize_free_chicken(&self, bounds: Aabb) -> Option<u32> {
        self.mobs
            .iter()
            .find(|c| {
                c.species() == Species::Chicken
                    && c.alive()
                    && c.extra.rider.is_none()
                    && c.extra.mount.is_none()
                    && c.bounds().intersects(bounds)
            })
            .map(|c| c.id)
    }
    /// `Fallable.isHalloween` at the level's clock.
    pub(super) fn finalize_halloween(&self) -> bool {
        finalize::is_halloween()
    }
    /// The tables a finalizeSpawn `Ctx` borrows: the variant registries, the
    /// enchantments and items its gear rolls need, and `EntityTypes.CAMEL_HUSK`'s
    /// dimensions if the table has the type.
    pub(super) fn finalize_spawn_data(&self) -> finalize::SpawnData<'_> {
        let game = self;
        finalize::SpawnData {
            registries: game.res.spawn_registries(),
            enchantments: &world_stations!(game).enchantments,
            items: world_items!(game),
            camel_husk_size: game
                .res
                .mob_types()
                .get("minecraft:camel_husk")
                .map(|t| (t.width as f32, t.height as f32))
                .unwrap_or((1.7, 2.375)),
        }
    }
    /// The mod event half of a finalizeSpawn, mapped to the `Ctx`'s outcome:
    /// the level decides, and the game carries it out.
    pub(super) fn finalize_spawn_event_result(
        &self,
        mob: &mut Mob,
        reason: Reason,
    ) -> finalize::SpawnEvent {
        // A mob finalization makes starts without group data, and the
        // finalization that follows reads none.
        let (outcome, _) = self.finalize_spawn_event(mob, reason, None, &mut None);
        match outcome {
            super::creatures::Finalized::Cancelled => finalize::SpawnEvent::Cancelled,
            super::creatures::Finalized::Skipped => finalize::SpawnEvent::Skip,
            super::creatures::Finalized::Done => finalize::SpawnEvent::Finalize,
        }
    }
    /// The game's own finalization of a mob, with no event: the same
    /// `finalize::Ctx` world generation runs, over the live level, with the
    /// level's random (`world.random`). Mobs the call made (jockeys and
    /// mounts) join the level beside it.
    pub(super) fn finalize_plain(
        &mut self,
        mob: &mut Mob,
        difficulty: &spawning::LocalDifficulty,
        reason: Reason,
    ) {
        self.finalize_grouped(mob, difficulty, reason, None);
    }
    /// [`Game::finalize_plain`] with the pack's group data, as a listener of
    /// `finalize_spawn` left it.
    pub(super) fn finalize_grouped(
        &mut self,
        mob: &mut Mob,
        difficulty: &spawning::LocalDifficulty,
        reason: Reason,
        group: Option<finalize::Group>,
    ) {
        let mut random = self.world.random.clone();
        let mut level = FinalizeGame { game: self };
        let mut cx = finalize::Ctx {
            level: &mut level,
            random: &mut random,
            difficulty: *difficulty,
            created: Vec::new(),
            jockey_rider: None,
        };
        finalize::finalize(&mut cx, mob, reason, group);
        let _save_lease = horizon_client::join_seq::visibility::PassLease::enter();
        let created = std::mem::take(&mut cx.created);
        let game = level.game;
        game.world.random = random;
        for m in created {
            game.mobs.push(m);
            game.native_mobs_joined(game.mobs.len() - 1, false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_core::random::{LegacyRandom, RandomSource};

    /// A zombie for the spawn tests, at a spot with room around it.
    fn zombie(game: &Game<'_>) -> horizon_client::mobs::Mob {
        let kind = game
            .res
            .mob_types()
            .get("minecraft:zombie")
            .unwrap()
            .clone();
        horizon_client::mobs::Mob::new(1, kind, Vec3::new(8.0, 65.0, 8.0), 0.0)
    }

    /// Any other mob for the spawn tests, at the same spot.
    fn mob_of(game: &Game<'_>, id: u32, kind: &str) -> horizon_client::mobs::Mob {
        let kind = game.res.mob_types().get(kind).unwrap().clone();
        horizon_client::mobs::Mob::new(id, kind, Vec3::new(8.0, 65.0, 8.0), 0.0)
    }

    /// `Mob.finalizeSpawn`'s own two draws (`Mob.java:1078-1085`), then
    /// `Zombie.finalizeSpawn`'s, replayed on a fresh random of the same seed:
    /// the follow-range triangle, the left-handed roll, `pick_up_loot`, the
    /// baby roll and the break-doors roll, in vanilla's order.
    struct VanillaZombieDraws {
        follow_range: f64,
        left_handed: bool,
        pick_up_loot: bool,
        baby: bool,
        can_break_doors: bool,
    }
    fn vanilla_draws(random: &mut LegacyRandom, special: f32) -> VanillaZombieDraws {
        VanillaZombieDraws {
            follow_range: random.triangle(0.0, 0.114_850_000_000_000_01),
            left_handed: random.next_float() < 0.05,
            pick_up_loot: random.next_float() < 0.55 * special,
            baby: random.next_float() < 0.05,
            can_break_doors: random.next_float() < special * 0.1,
        }
    }

    /// The follow-range spawn bonus `finalizeSpawn` added, if any.
    fn random_spawn_bonus(mob: &horizon_client::mobs::Mob) -> Option<f64> {
        mob.extra
            .attributes
            .as_ref()?
            .modifiers
            .iter()
            .find(|m| m.id == "minecraft:random_spawn_bonus")
            .map(|m| m.amount)
    }

    /// finalizeSpawn draws from the level's random (`Ctx.random` is
    /// `world.random`), in vanilla's order: a shadow random of the same seed
    /// replaying `Mob.java`'s and `Zombie.finalizeSpawn`'s draws predicts
    /// every observable outcome, and the level random advances by exactly
    /// the draws the finalization made.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn finalize_plain_draws_the_level_random_in_vanilla_order() {
        crate::game::native_guest_turn_tests::with_guests(|game| {
            let mut mob = zombie(game);
            let seed = 0xC0FFEE_i64;
            game.world.random =
                horizon_core::random::LevelRandom::for_level(LegacyRandom::new(seed));
            let difficulty = game.local_difficulty([8.0, 65.0, 8.0]);
            game.finalize_plain(&mut mob, &difficulty, Reason::Natural);

            let mut shadow = LegacyRandom::new(seed);
            let draws = vanilla_draws(&mut shadow, difficulty.special());
            assert_eq!(mob.extra.left_handed, draws.left_handed);
            assert_eq!(mob.extra.pick_up_loot, draws.pick_up_loot);
            assert_eq!(mob.extra.can_break_doors, draws.can_break_doors);
            let bonus = random_spawn_bonus(&mob).expect("random_spawn_bonus added");
            assert!((bonus - draws.follow_range).abs() < 1e-12);
            // The baby roll picked its group state; a baby on this seed would
            // have ridden the same stream, so its flag matches the shadow too.
            assert_eq!(mob.extra.age < 0, draws.baby);
            // finalizeSpawn's later rolls (populate, enchant, the Halloween
            // head) advance the same stream past the shadow's replay.
            let before = shadow.next_int_bound(1000);
            assert_ne!(game.world.random.next_int_bound(1000), before);
        });
    }

    /// With no mod listening, the event half of the folded finalizeSpawn
    /// changes nothing: `finalize_with` finalizes exactly as `finalize_plain`
    /// does, on the same level stream.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn finalize_with_without_listeners_finalizes_like_finalize_plain() {
        crate::game::native_guest_turn_tests::with_guests(|game| {
            let mut with = zombie(game);
            let mut plain = zombie(game);
            let difficulty = game.local_difficulty([8.0, 65.0, 8.0]);
            game.world.random =
                horizon_core::random::LevelRandom::for_level(LegacyRandom::new(42_i64));
            let outcome = game.finalize_with(&mut with, Reason::Natural, None);
            assert_eq!(outcome, super::super::creatures::Finalized::Done);
            // The same stream for the plain path: same seed, same draws.
            game.world.random =
                horizon_core::random::LevelRandom::for_level(LegacyRandom::new(42_i64));
            game.finalize_plain(&mut plain, &difficulty, Reason::Natural);

            assert_eq!(with.extra.left_handed, plain.extra.left_handed);
            assert_eq!(with.extra.pick_up_loot, plain.extra.pick_up_loot);
            assert_eq!(with.extra.can_break_doors, plain.extra.can_break_doors);
            assert_eq!(with.extra.age < 0, plain.extra.age < 0);
            let a = random_spawn_bonus(&with);
            let b = random_spawn_bonus(&plain);
            assert_eq!(a.is_some(), b.is_some());
            if let (Some(a), Some(b)) = (a, b) {
                assert!((a - b).abs() < 1e-12);
            }
        });
    }

    /// #2783: `/summon`, eggs, spawners and natural spawns run
    /// `WitherSkeleton.finalizeSpawn`: attack damage base 4.0, not the
    /// default 2.0.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn runtime_wither_skeletons_hit_for_four() {
        crate::game::native_guest_turn_tests::with_guests(|game| {
            let mut w = mob_of(game, 1, "minecraft:wither_skeleton");
            let difficulty = game.local_difficulty([8.0, 65.0, 8.0]);
            game.finalize_plain(&mut w, &difficulty, Reason::MobSummoned);
            assert_eq!(finalize::value(&w, finalize::Attr::AttackDamage), 4.0);
            assert_eq!(w.attrs.attack_damage, 4.0);
        });
    }

    /// #2783: runtime spawns run `Mob.finalizeSpawn`: the follow range
    /// bonus and 5 percent left-handed mobs, drawn from the level random.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn runtime_spawns_run_the_mob_finalize_spawn() {
        crate::game::native_guest_turn_tests::with_guests(|game| {
            let difficulty = game.local_difficulty([8.0, 65.0, 8.0]);
            let mut left = 0;
            for id in 1..=400u32 {
                let mut m = mob_of(game, id, "minecraft:skeleton");
                game.finalize_plain(&mut m, &difficulty, Reason::MobSummoned);
                assert!(finalize::has_modifier(
                    &m,
                    finalize::Attr::FollowRange,
                    "minecraft:random_spawn_bonus"
                ));
                left += usize::from(m.extra.left_handed);
            }
            // 5 percent of 400 is 20.
            assert!((5..=40).contains(&left), "{left} of 400 left-handed");
            // A bred baby never ran it.
            let mut cow = mob_of(game, 1, "minecraft:cow");
            game.finalize_plain(&mut cow, &difficulty, Reason::MobSummoned);
            cow.extra.left_handed = true;
            spawning::bred_offspring(&mut cow);
            assert!(!finalize::has_modifier(
                &cow,
                finalize::Attr::FollowRange,
                "minecraft:random_spawn_bonus"
            ));
            assert!(!cow.extra.left_handed);
            assert_eq!(
                cow.attrs.follow_range,
                finalize::value(&cow, finalize::Attr::FollowRange)
            );
            // CaveSpider.finalizeSpawn does not call super.
            let mut c = mob_of(game, 1, "minecraft:cave_spider");
            game.finalize_plain(&mut c, &difficulty, Reason::MobSummoned);
            assert!(!finalize::has_modifier(
                &c,
                finalize::Attr::FollowRange,
                "minecraft:random_spawn_bonus"
            ));
        });
    }

    /// `Zombie` / `AbstractSkeleton` halloween garb: 25% of empty heads, never
    /// on other mobs, and not when it is not Halloween.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn halloween_pumpkins_only_undead_on_halloween() {
        crate::game::native_guest_turn_tests::with_guests(|game| {
            let mut pumpkins = 0;
            let mut lanterns = 0;
            for id in 1..=400u32 {
                let mut m = mob_of(game, id, "minecraft:skeleton");
                m.extra.set_slot_id(horizon_items::equipment::HEAD, None);
                let mut random = m.random.clone();
                finalize::halloween_head(&mut m, &mut random, true);
                match m.extra.head_id() {
                    Some("minecraft:jack_o_lantern") => {
                        pumpkins += 1;
                        lanterns += 1;
                        assert_eq!(finalize::stored_drop_chance(&m, 5), 0.0);
                    }
                    Some("minecraft:carved_pumpkin") => {
                        pumpkins += 1;
                        assert_eq!(finalize::stored_drop_chance(&m, 5), 0.0);
                    }
                    Some(other) => panic!("unexpected head {other}"),
                    None => {}
                }
            }
            assert!(
                (70..=130).contains(&pumpkins),
                "{pumpkins} of 400 skeletons"
            );
            assert!(lanterns < pumpkins / 2, "{lanterns} lanterns of {pumpkins}");
            let mut m = mob_of(game, 1, "minecraft:skeleton");
            m.extra.set_slot_id(horizon_items::equipment::HEAD, None);
            let mut random = m.random.clone();
            finalize::halloween_head(&mut m, &mut random, false);
            assert!(m.extra.head_id().is_none());
            let mut cow = mob_of(game, 1, "minecraft:cow");
            cow.extra.set_slot_id(horizon_items::equipment::HEAD, None);
            let mut random = cow.random.clone();
            finalize::halloween_head(&mut cow, &mut random, true);
            assert!(cow.extra.head_id().is_none());
        });
    }

    /// The strider's zombified-piglin rider needs `isSpawningMonsters`
    /// (`ServerLevel.isSpawningMonsters`: the difficulty and the `spawn_mobs`
    /// and `spawn_monsters` gamerules), not the difficulty alone.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn the_striders_piglin_rider_needs_spawning_monsters() {
        use horizon_client::gamerules::Value as RuleValue;
        crate::game::native_guest_turn_tests::with_guests(|game| {
            // The gate reads the difficulty setting (`ui.difficulty`), which a
            // fresh test game leaves peaceful; the rules default on.
            game.ui.difficulty = 2;
            let difficulty = game.local_difficulty([8.0, 65.0, 8.0]);
            let piglins = |game: &Game<'_>| {
                game.mobs
                    .iter()
                    .filter(|m| m.species() == Species::ZombifiedPiglin)
                    .count()
            };
            for id in 1..=400u32 {
                let mut s = mob_of(game, id, "minecraft:strider");
                game.finalize_plain(&mut s, &difficulty, Reason::Natural);
            }
            assert!(piglins(game) > 0, "riders with the rules on");
            // `spawn_monsters` off: no rider, though the difficulty allows it.
            assert!(game.ui.rules.set("spawn_monsters", RuleValue::Bool(false)));
            let before = piglins(game);
            for id in 1000..=1400u32 {
                let mut s = mob_of(game, id, "minecraft:strider");
                game.finalize_plain(&mut s, &difficulty, Reason::Natural);
            }
            assert_eq!(piglins(game), before);
        });
    }
}
