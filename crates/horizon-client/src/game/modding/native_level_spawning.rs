//! Mob spawning and a level's natural spawning policy through a `LevelRef`
//! (docs/modding/src/how-to/world/spawning.md, "Spawning from a level"): a mob made
//! as `EntityType.spawn` makes one, the per-category limits and the flags
//! natural spawning reads (`natural_spawning::SpawnPolicy`), and the chunk
//! question it asks (`ServerLevel.canSpawnEntitiesInChunk`).
use super::{Game, creatures::SpawnSite, natural_spawning};
use mod_api::{EntityRef, LevelRef, SpawnFlags, SpawnLimits};

impl Game<'_> {
    /// `EntityType.spawn(level, pos, reason)` for a mob: `spawn_mob_spawned`'s path
    /// (creation, `finalize_spawn` and `finalizeSpawn` with `reason`, the join), not
    /// persistent. None for a level not in place, a client, a position that isn't finite,
    /// outside the spawnable bounds or in an unloaded chunk, a kind that isn't a mob, and
    /// a spawn a mod cancelled or kept out.
    pub(super) fn level_spawn_mob(
        &mut self,
        level: LevelRef,
        kind: &str,
        pos: [f64; 3],
        reason: mod_api::SpawnReason,
    ) -> Option<EntityRef> {
        if self.entity_ids_exhausted() {
            return None;
        }
        if !self.level_in_place(level) || !pos.iter().all(|v| v.is_finite()) {
            return None;
        }
        // BlockPos.containing. `as` saturates, and a value past `i32` is out of bounds.
        let block = pos.map(|v| v.floor() as i32);
        if self.level_in_spawnable_bounds(level, block) != Some(true)
            || !self.world.loaded(block[0], block[2])
        {
            return None;
        }
        let id = self.spawn_mob_spawned(kind, pos, SpawnSite::Finalizing(reason))?;
        Some(EntityRef::new(level.epoch(), id))
    }
    /// The level's natural spawning limits. None for a stale handle or on a client.
    pub(super) fn level_spawn_limits(&self, level: LevelRef) -> Option<SpawnLimits> {
        self.native_level_world(level)?;
        Some(self.spawn_policy(level.dimension()).limits)
    }
    /// The level's natural spawning flags. None for a stale handle or on a client.
    pub(super) fn level_spawn_flags(&self, level: LevelRef) -> Option<SpawnFlags> {
        self.native_level_world(level)?;
        Some(self.spawn_policy(level.dimension()).flags)
    }
    /// Sets the level's natural spawning limits for the rest of the open. False for a
    /// stale handle or on a client.
    pub(super) fn level_set_spawn_limits(&mut self, level: LevelRef, limits: SpawnLimits) -> bool {
        self.native_set_spawn_policy(level, |policy| policy.limits = limits)
    }
    /// Sets the level's natural spawning flags for the rest of the open. False as for
    /// `level_set_spawn_limits`.
    pub(super) fn level_set_spawn_flags(&mut self, level: LevelRef, flags: SpawnFlags) -> bool {
        self.native_set_spawn_policy(level, |policy| policy.flags = flags)
    }
    fn native_set_spawn_policy(
        &mut self,
        level: LevelRef,
        set: impl FnOnce(&mut natural_spawning::SpawnPolicy),
    ) -> bool {
        if self.native_level_world(level).is_none() {
            return false;
        }
        set(self.spawn_policies.entry(level.dimension()).or_default());
        true
    }
    /// `ServerLevel.canSpawnEntitiesInChunk` as natural spawning answers it. None for a
    /// stale handle or on a client.
    pub(super) fn level_can_spawn_entities_in_chunk(
        &self,
        level: LevelRef,
        pos: [i32; 2],
    ) -> Option<bool> {
        let world = self.native_level_world(level)?;
        Some(natural_spawning::can_spawn_entities_in_chunk(
            world, pos[0], pos[1],
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::super::Game;
    use super::super::native_entity_refs::tests::{with_client_game, with_game};
    use horizon_client::chunk::Chunk;
    use horizon_client::mobs::spawn_selection::{Category, Counted, SpawnPlayer, SpawnState};
    use mod_api::{
        EntityConstructingEvent, FinalizeSpawnEvent, Flow, Holder, Host, HostSide, LevelRef,
        SpawnFlags, SpawnLimits, WorldEpoch,
    };
    use std::sync::{Arc, Mutex};

    /// What a spawn told the mods, in order.
    type Heard = Arc<Mutex<Vec<String>>>;
    struct Listener(Heard);
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Listener {
        fn entity_constructing(&self, _: HostSide<'_>, event: &mut EntityConstructingEvent) {
            if let Holder::Entity(_) = event.holder {
                self.0
                    .lock()
                    .unwrap()
                    .push(format!("construct {}", event.kind));
            }
        }
        fn finalize_spawn(&self, _: &dyn Host, event: &mut FinalizeSpawnEvent<'_>) -> Flow {
            self.0
                .lock()
                .unwrap()
                .push(format!("finalize {} {:?}", event.kind, event.reason));
            Flow::Continue
        }
    }

    /// A cow (no jockey or rider rolls) spawns through the entity type's own path: constructed,
    /// then finalized with the caller's reason, and it joins not persistent. Every refusal
    /// spawns nothing and tells no mod.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES and an isolated process for native mods"]
    fn a_level_spawns_a_mob_as_its_entity_type_does_and_refuses_the_rest() {
        let heard = Heard::default();
        let mods = vec![mod_api::ModEntry::new(
            "level-spawning",
            Box::new(Listener(heard.clone())),
        )];
        mod_platform::testing::with_mods(mods.into(), || {
            with_game(|game| {
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                let level = game.level(game.dim.id()).unwrap();
                let cow = game
                    .level_spawn_mob(
                        level,
                        "minecraft:cow",
                        [8.5, 64.0, 8.25],
                        mod_api::SpawnReason::Command,
                    )
                    .unwrap();
                assert_eq!(
                    game.entity_kind(cow).map(|kind| kind.id(game)).as_deref(),
                    Some("minecraft:cow")
                );
                assert_eq!(game.entity_position(cow), Some([8.5, 64.0, 8.25]));
                let mob = game.mobs.iter().find(|m| m.id == cow.id()).unwrap();
                assert!(!mob.persistent);
                assert_eq!(
                    *heard.lock().unwrap(),
                    ["construct minecraft:cow", "finalize minecraft:cow Command"]
                );

                let mobs = game.mobs.len();
                let stale = LevelRef::new(WorldEpoch::new(0), game.dim.key());
                let at = [8.5, 64.0, 8.5];
                let refused: [(LevelRef, &str, [f64; 3]); 9] = [
                    (level, "minecraft:cow", [f64::NAN, 64.0, 8.5]),
                    (level, "minecraft:cow", [8.5, f64::INFINITY, 8.5]),
                    // Past i32: `as` saturates, and the bounds refuse it.
                    (level, "minecraft:cow", [1.0e300, 64.0, 8.5]),
                    (level, "minecraft:cow", [8.5, 64.0, f64::from(i32::MIN)]),
                    (level, "minecraft:cow", [f64::from(i32::MAX), 64.0, 8.5]),
                    // Level.isInSpawnableBounds: y below 20,000,000.
                    (level, "minecraft:cow", [8.5, 20_000_000.0, 8.5]),
                    (level, "minecraft:cow", [1000.5, 64.0, 8.5]),
                    (level, "minecraft:stone", at),
                    (stale, "minecraft:cow", at),
                ];
                for (level, kind, pos) in refused {
                    assert_eq!(
                        game.level_spawn_mob(level, kind, pos, mod_api::SpawnReason::Command),
                        None,
                        "{kind} at {pos:?}"
                    );
                }
                assert_eq!(game.mobs.len(), mobs);
                assert_eq!(heard.lock().unwrap().len(), 2, "no refusal told a mod");
            });
        });
    }

    /// One player in chunk (0, 0).
    fn player() -> Vec<SpawnPlayer> {
        vec![SpawnPlayer {
            position: [8.0, 64.0, 8.0],
            spectator: false,
        }]
    }

    /// The state and categories the level's next spawn pass reads over vanilla's 289
    /// spawnable chunks, with one loose zombie counted in chunk (0, 0).
    fn next_pass(game: &Game<'_>) -> (SpawnState, Vec<Category>) {
        let zombie = [Counted {
            entity: "minecraft:zombie",
            category: Category::Monster,
            block: [8, 64, 8],
            is_mob: true,
            persistent: false,
        }];
        let level = game.natural_level();
        let state = game.natural_spawn_state(&level, 289, player(), &zombie);
        let categories = game.natural_categories(&state);
        (state, categories)
    }

    /// Without a change the pass is vanilla's: every spawning category under the caps. A
    /// monster limit of 1 is filled by the one zombie counted (1 * 289 / 289), so monsters
    /// drop out; 2 lets them back. The flags take categories away, and the limits hold
    /// beside them. A stale handle and a client read nothing and set nothing.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_levels_spawn_policy_changes_what_its_next_pass_spawns() {
        use Category as C;
        let friendly = [
            C::Creature,
            C::Ambient,
            C::Axolotls,
            C::UndergroundWaterCreature,
            C::WaterCreature,
            C::WaterAmbient,
        ];
        with_game(|game| {
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            // The persistent categories' pass, under vanilla's default rules.
            game.world.game_time = 400;
            let level = game.level(game.dim.id()).unwrap();
            assert_eq!(game.level_spawn_limits(level), Some(SpawnLimits::VANILLA));
            let both = SpawnFlags {
                monsters: true,
                friendlies: true,
            };
            assert_eq!(game.level_spawn_flags(level), Some(both));
            let (mut state, categories) = next_pass(game);
            let mut all = vec![C::Monster];
            all.extend(friendly);
            assert_eq!(categories, all);
            assert!(state.can_spawn_local(C::Monster, 0, 0));

            let one = SpawnLimits {
                monster: 1,
                ..SpawnLimits::VANILLA
            };
            assert!(game.level_set_spawn_limits(level, one));
            assert_eq!(game.level_spawn_limits(level), Some(one));
            let (mut state, categories) = next_pass(game);
            assert_eq!(categories, friendly);
            assert!(!state.can_spawn_local(C::Monster, 0, 0));
            let two = SpawnLimits {
                monster: 2,
                ..SpawnLimits::VANILLA
            };
            assert!(game.level_set_spawn_limits(level, two));
            assert_eq!(next_pass(game).1, all);

            let monsters_only = SpawnFlags {
                monsters: true,
                friendlies: false,
            };
            assert!(game.level_set_spawn_flags(level, monsters_only));
            assert_eq!(game.level_spawn_flags(level), Some(monsters_only));
            assert_eq!(next_pass(game).1, [C::Monster]);
            assert_eq!(
                game.level_spawn_limits(level),
                Some(two),
                "flags leave limits"
            );

            let stale = LevelRef::new(WorldEpoch::new(0), game.dim.key());
            assert_eq!(game.level_spawn_limits(stale), None);
            assert_eq!(game.level_spawn_flags(stale), None);
            assert!(!game.level_set_spawn_limits(stale, SpawnLimits::VANILLA));
            assert!(!game.level_set_spawn_flags(stale, both));
            assert_eq!(game.level_can_spawn_entities_in_chunk(stale, [0, 0]), None);
            assert_eq!(game.level_spawn_limits(level), Some(two));

            // canSpawnEntitiesInChunk: the loaded chunk, not its unloaded neighbour, and the
            // ends of i32 read as any other chunk.
            assert_eq!(
                game.level_can_spawn_entities_in_chunk(level, [0, 0]),
                Some(true)
            );
            assert_eq!(
                game.level_can_spawn_entities_in_chunk(level, [1, 0]),
                Some(false)
            );
            assert_eq!(
                game.level_can_spawn_entities_in_chunk(level, [i32::MIN, i32::MAX]),
                Some(false)
            );
        });
        with_client_game(|game| {
            let epoch = game.native_lifecycle.epoch().unwrap();
            let level = LevelRef::new(epoch, game.dim.key());
            assert_eq!(game.level_spawn_limits(level), None);
            assert!(!game.level_set_spawn_limits(level, SpawnLimits::VANILLA));
            assert_eq!(game.level_can_spawn_entities_in_chunk(level, [0, 0]), None);
        });
    }
}
