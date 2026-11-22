//! Player statistics (`ServerStatsCounter`): the custom counters, blocks
//! mined, items used/crafted/broken/picked up/dropped and mobs killed or
//! killed by, saved to `players/stats/<uuid>.json`, plus the phantom
//! spawner that reads the time since rest (`PhantomSpawner`).
use super::native_hooks::PhantomSpawnResult;
use super::{Dim, Game};
use horizon_core::geometry::Vec3;
use horizon_core::random::RandomSource;
pub(super) use horizon_gameplay::stats::{
    Stats, locate_player_json, player_json, write_player_json,
};
use horizon_gameplay::stats::{namespaced, phantom_group_size};
use std::path::PathBuf;

#[cfg(test)]
#[path = "tests/phantom_spawn_tests.rs"]
mod phantom_spawn_tests;
#[cfg(test)]
#[path = "tests/player_stat_award_tests.rs"]
mod player_stat_award_tests;
#[cfg(test)]
#[path = "tests/stat_award_tests.rs"]
mod stat_award_tests;

/// One player `PhantomSpawner.tick` visits. `identity` is the server player's uuid and
/// name, absent for a game's own player, which no server player stands for.
struct PhantomTarget {
    position: Vec3,
    since_rest: i32,
    identity: Option<(u128, String)>,
}

/// The value registry owned by each vanilla statistic type.
fn stat_registry(ty: &str) -> Option<&'static str> {
    Some(match ty {
        "minecraft:mined" => "minecraft:block",
        "minecraft:crafted"
        | "minecraft:used"
        | "minecraft:broken"
        | "minecraft:picked_up"
        | "minecraft:dropped" => "minecraft:item",
        "minecraft:killed" | "minecraft:killed_by" => "minecraft:entity_type",
        "minecraft:custom" => "minecraft:custom_stat",
        _ => return None,
    })
}

impl Game<'_> {
    /// Resolve a hook's complete award before any statistic, scoreboard or packet is changed.
    /// Only listener calls allocate; ordinary per-tick awards keep their borrowed identifiers.
    pub(in crate::game) fn edited_stat_award(
        &mut self,
        player: u128,
        name: &str,
        ty: &str,
        key: &str,
        n: i32,
    ) -> Option<(String, String, i32)> {
        let mut new_ty = ty.to_owned();
        let mut new_key = key.to_owned();
        let mut amount = n;
        let mut cancel = false;
        self.native_player_stat_award(
            player,
            name,
            &mut new_ty,
            &mut new_key,
            &mut amount,
            &mut cancel,
        );
        if cancel {
            return None;
        }
        let valid = amount >= 0
            && stat_registry(&new_ty).is_some_and(|registry| {
                let known = |registry: &str, id: &str| {
                    if let Some(host) = self.lan.as_ref() {
                        use horizon_core::identifier::Identifier;
                        // Parsing aliases here would let counters, criteria and packets
                        // disagree about the spelling of the same statistic.
                        let (Some(registry), Some(id)) = (
                            Identifier::parse_namespaced(registry),
                            Identifier::parse_namespaced(id),
                        ) else {
                            return false;
                        };
                        return host.hub.synced_id(&registry, &id).is_some();
                    }
                    self.ui
                        .registries
                        .ids(registry)
                        .iter()
                        .any(|entry| entry == id)
                };
                known("minecraft:stat_type", &new_ty) && known(registry, &new_key)
            });
        if valid {
            Some((new_ty, new_key, amount))
        } else {
            static WARNED: std::sync::atomic::AtomicBool =
                std::sync::atomic::AtomicBool::new(false);
            if !WARNED.swap(true, std::sync::atomic::Ordering::Relaxed) {
                log::warn!(
                    "player_stat_award refused invalid rewrite ({new_ty}/{new_key}, {amount}); retaining original award"
                );
            }
            Some((ty.to_owned(), key.to_owned(), n))
        }
    }
    fn stats_path(&self) -> Option<PathBuf> {
        Some(locate_player_json(
            &self.world_root,
            "stats",
            self.level.player_uuid?,
            &self.options.player_name(),
        ))
    }
    /// `ServerStatsCounter.parseLocal`.
    pub(super) fn load_stats(&mut self) {
        if self.stats.loaded || self.remote.is_some() || self.no_player_here() {
            return;
        }
        self.stats.loaded = true;
        if let Some(path) = self.stats_path() {
            self.stats.load_from(Stats::read(&path));
        }
    }
    /// `ServerStatsCounter.save`.
    pub(super) fn save_stats(&self) {
        if !self.stats.loaded || self.remote.is_some() || self.no_player_here() {
            return;
        }
        if let Some(path) = self.stats_path() {
            self.stats.write(&path);
        }
    }
    /// `ServerPlayer.awardStat`: `n` more of the statistic `ty` (a bare type
    /// such as `used`) and `key`, unless a native mod cancels the award.
    pub(super) fn award_stat(&mut self, ty: &str, key: &str, n: i32) {
        if self.lan_acting_fake {
            return;
        }
        // No mod, no name copied: the per-tick counters come through here.
        let acting = if !mod_api::__private::listened(mod_api::HookId::PlayerStatAward) {
            None
        } else {
            self.lan_acting_uuid.zip(self.lan_acting_name.clone())
        };
        if let Some((player, name)) = acting {
            let award = namespaced(ty, |stat_type| {
                self.edited_stat_award(player, &name, stat_type, key, n)
            });
            if let Some((ty, key, amount)) = award {
                self.stats
                    .add(ty.strip_prefix("minecraft:").unwrap_or(&ty), &key, amount);
            }
            return;
        }
        self.stats.add(ty, key, n);
    }
    /// `Player.awardStat(Identifier)`: `n` more of the custom statistic `key`.
    pub(super) fn award_custom_stat(&mut self, key: &str, n: i32) {
        namespaced(key, |key| self.award_stat("custom", key, n));
    }
    /// The per-tick counters (`Player.tick`/`ServerPlayer.tick` awardStat).
    pub(super) fn tick_player_stat_counters(&mut self) {
        self.load_stats();
        self.count_tick_stats(self.sleeping.is_some());
        self.tick_movement_stats();
    }
    /// `Player.tick`'s counters for the player in `self.player`.
    pub(super) fn count_tick_stats(&mut self, sleeping: bool) {
        let Some(p) = self.player.as_ref() else {
            return;
        };
        let alive = !p.health.dead_or_dying();
        let crouching = p.pose == horizon_client::pose::Pose::Crouching;
        if !self.lan_acting_fake && !mod_api::__private::listened(mod_api::HookId::PlayerStatAward)
        {
            let mut mask = (1 << 0) | (1 << 1);
            if alive {
                mask |= 1 << 2;
            }
            if crouching {
                mask |= 1 << 3;
            }
            if !sleeping {
                mask |= 1 << 4;
            }
            let record_events = !self.score.board.objectives.is_empty();
            self.stats.add_tick_mask(mask, record_events);
            return;
        }
        self.award_custom_stat("play_time", 1);
        self.award_custom_stat("total_world_time", 1);
        if alive {
            self.award_custom_stat("time_since_death", 1);
        }
        if crouching {
            self.award_custom_stat("sneak_time", 1);
        }
        // ServerPlayer.doTick: dead or alive, every tick out of bed.
        if !sleeping {
            self.award_custom_stat("time_since_rest", 1);
        }
    }
    fn tick_movement_stats(&mut self) {
        let Some(p) = self.player.as_ref() else {
            return;
        };
        let d = p.position.subtract(self.previous_position);
        self.movement_stats(d);
    }
    /// `Player.checkMovementStatistics` / `checkRidingStatistics` for one move.
    pub(super) fn movement_stats(&mut self, d: Vec3) {
        let Some(p) = self.player.as_ref() else {
            return;
        };
        let horizontal = ((d.x * d.x + d.z * d.z).sqrt() * 100.0).round() as i32;
        let total = ((d.x * d.x + d.y * d.y + d.z * d.z).sqrt() * 100.0).round() as i32;
        // Teleports are not travel.
        if total <= 0 || total > 1000 {
            return;
        }
        let crouching = p.pose == horizon_client::pose::Pose::Crouching;
        let feet = [
            p.position.x.floor() as i32,
            p.position.y.floor() as i32,
            p.position.z.floor() as i32,
        ];
        let eye_in_water = self.res.facts().get(self.world.block(
            feet[0],
            (p.position.y + f64::from(p.dimensions.eye_height)).floor() as i32,
            feet[2],
        ));
        let in_water = self
            .res
            .facts()
            .get(self.world.block(feet[0], feet[1], feet[2]))
            .fluid
            == horizon_client::blocks::Fluid::Water;
        let on_ground = p.ground.on_ground;
        let (sprinting, gliding, swimming) = (
            p.sprinting,
            p.fall_flying,
            p.pose == horizon_client::pose::Pose::Swimming,
        );
        let climbing = p.last_climbable.is_some() && d.y > 0.0;
        let riding = self.riding.map(|id| {
            self.mobs
                .iter()
                .find(|m| m.id == id)
                .map(|m| m.species())
                .ok_or(())
        });
        match riding {
            Some(Ok(sp)) => {
                use horizon_client::mobs::Species;
                // `checkRidingStatistics`: only these mounts count, the horse
                // family by the full 3D distance; any other mount nothing.
                match sp {
                    Species::Pig => self.award_custom_stat("pig_one_cm", total),
                    Species::Strider => self.award_custom_stat("strider_one_cm", total),
                    Species::HappyGhast => self.award_custom_stat("happy_ghast_one_cm", total),
                    Species::Nautilus | Species::ZombieNautilus => {
                        self.award_custom_stat("nautilus_one_cm", total)
                    }
                    sp if horizon_client::mobs::equine::is_equine(sp) => {
                        self.award_custom_stat("horse_one_cm", total)
                    }
                    _ => {}
                }
            }
            Some(Err(())) => {
                if self.boats.iter().any(|b| Some(b.id) == self.riding) {
                    self.award_custom_stat("boat_one_cm", total)
                } else if self.carts.iter().any(|c| Some(c.id) == self.riding) {
                    self.award_custom_stat("minecart_one_cm", total)
                }
            }
            None if swimming => self.award_custom_stat("swim_one_cm", total),
            None if eye_in_water.fluid == horizon_client::blocks::Fluid::Water => {
                self.award_custom_stat("walk_under_water_one_cm", total)
            }
            None if in_water => {
                if horizontal > 0 {
                    self.award_custom_stat("walk_on_water_one_cm", horizontal)
                }
            }
            None if climbing => {
                self.award_custom_stat("climb_one_cm", (d.y * 100.0).round() as i32)
            }
            None if on_ground => {
                if horizontal > 0 {
                    if sprinting {
                        self.award_custom_stat("sprint_one_cm", horizontal)
                    } else if crouching {
                        self.award_custom_stat("crouch_one_cm", horizontal)
                    } else {
                        self.award_custom_stat("walk_one_cm", horizontal)
                    }
                }
            }
            None if gliding => self.award_custom_stat("aviate_one_cm", total),
            // Airborne and not gliding: more than 25 cm sideways per move
            // (creative flight, or a long jump), not only flying.
            None if horizontal > 25 => self.award_custom_stat("fly_one_cm", horizontal),
            None => {}
        }
    }
    /// `PhantomSpawner.tick`: tired players under an open night sky draw
    /// phantoms. A custom spawner ticks only while `spawn_mobs` is on
    /// (`ServerChunkCache.tickChunks`), and this one only spawns enemies
    /// (`ServerLevel.isSpawningMonsters`: `spawn_monsters` too). Every
    /// player of the level counts: the game's own and the hosted ones (a
    /// server thread's players are all hosted).
    pub(super) fn spawn_phantoms(&mut self) {
        let rules = &self.ui.rules;
        if !rules.bool_at(horizon_client::gamerules::vanilla::SPAWN_MOBS)
            || !rules.bool_at(horizon_client::gamerules::vanilla::SPAWN_MONSTERS)
            || !rules.bool_at(horizon_client::gamerules::vanilla::SPAWN_PHANTOMS)
            || self.dim != Dim::Overworld
            || !self.has_custom_spawner(mod_api::SpawnerKind::Phantom)
        {
            return;
        }
        if !self.stats.phantom_due() {
            return;
        }
        let r = &mut self.world.random;
        self.stats
            .phantom_delay((60 + r.next_int_bound(60).unwrap_or(0)) * 20);
        if self.sky_darken() < 5 {
            return;
        }
        let rest = |s: &Stats| s.get("custom", "minecraft:time_since_rest");
        // `level.players()`: each player who isn't a spectator.
        let mut players = Vec::new();
        if let Some(p) = self.player.as_ref()
            && !self.spectator()
        {
            players.push(PhantomTarget {
                position: p.position,
                since_rest: rest(&self.stats),
                identity: None,
            });
        }
        if let Some(host) = self.lan.as_ref() {
            let level = self.dim.id();
            for g in host.hub.guests.iter().filter(|g| {
                g.placed
                    && g.participates_online()
                    && g.dimension == level
                    && g.game_mode != horizon_client::lan::guest::SPECTATOR
            }) {
                let since = host.play.get(&g.conn).map_or(0, |p| rest(&p.stats));
                players.push(PhantomTarget {
                    position: Vec3::new(g.position[0], g.position[1], g.position[2]),
                    since_rest: since,
                    identity: (!mod_api::__private::mods().is_empty())
                        .then(|| (g.uuid, g.name.clone())),
                });
            }
        }
        for target in players {
            self.spawn_phantoms_near(target);
        }
    }
    /// One player's turn of `PhantomSpawner.tick`.
    fn spawn_phantoms_near(&mut self, target: PhantomTarget) {
        let PhantomTarget {
            position,
            since_rest,
            identity,
        } = target;
        let pos = [
            position.x.floor() as i32,
            position.y.floor() as i32,
            position.z.floor() as i32,
        ];
        let difficulty_id = self.difficulty_id();
        let mut result = None;
        let mut phantoms = None;
        if let Some((player, name)) = identity {
            phantoms = Some(phantom_group_size(&mut self.world.random, difficulty_id));
            self.native_player_spawn_phantoms(player, &name, pos, &mut result, &mut phantoms);
        }
        let attempt = match result.unwrap_or(PhantomSpawnResult::Default) {
            PhantomSpawnResult::Allow => true,
            PhantomSpawnResult::Default => {
                pos[1] >= self.dim.sea_level()
                    && self.mob_world_base(&[]).can_see_sky(pos[0], pos[1], pos[2])
            }
            PhantomSpawnResult::Deny => false,
            _ => unreachable!("PhantomSpawnResult: unhandled variant"),
        };
        if !attempt {
            return;
        }
        // `getCurrentDifficultyAt(playerPos).isHarderThan(random * 3)`.
        let difficulty = self.local_difficulty([position.x, position.y, position.z]);
        let r = &mut self.world.random;
        if difficulty.effective <= r.next_float() * 3.0 {
            return;
        }
        if r.next_int_bound(since_rest.max(1)).unwrap_or(0) < 72000 {
            return;
        }
        // `playerPos.above(20 + nextInt(15)).east(-10 + nextInt(21)).south(...)`:
        // the height is drawn first.
        let up = 20 + r.next_int_bound(15).unwrap_or(0);
        let east = -10 + r.next_int_bound(21).unwrap_or(10);
        let south = -10 + r.next_int_bound(21).unwrap_or(10);
        let at = [pos[0] + east, pos[1] + up, pos[2] + south];
        if !self
            .res
            .facts()
            .is_air(self.world.block(at[0], at[1], at[2]))
        {
            return;
        }
        let group =
            phantoms.unwrap_or_else(|| phantom_group_size(&mut self.world.random, difficulty_id));
        for _ in 0..group {
            let _ = self.spawn_mob(
                "minecraft:phantom",
                [
                    f64::from(at[0]) + 0.5,
                    f64::from(at[1]),
                    f64::from(at[2]) + 0.5,
                ],
                false,
                super::creatures::SpawnSite::Finalizing(
                    horizon_client::mobs::spawn_rules::Reason::Natural,
                ),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    #[ignore = "daily"]
    fn fake_players_do_not_advance_phantom_rng() {
        use horizon_core::random::RandomSource;
        super::super::native_entity_refs::tests::with_game(|game| {
            super::super::bossbars::audience_tests::add_real_and_fake(game);
            game.player = None;
            game.world.day_time = 18000;
            let host = game.lan.as_mut().unwrap();
            let guests = &mut host.hub.guests;
            let index = guests.iter().position(|g| g.uuid == 2).unwrap();
            guests.update(index, |g| g.position = [1000.0, 100.0, 0.0]);
            for guest in &host.hub.guests {
                let mut play = super::super::lan_actions::GuestPlay::default();
                play.stats.custom("time_since_rest", 200_000);
                host.play.insert(guest.conn, play);
            }
            game.world.random = horizon_core::random::LevelRandom::for_level(
                horizon_core::random::LegacyRandom::new(12345),
            );
            game.spawn_phantoms();
            assert!(
                game.mobs
                    .iter()
                    .all(|mob| (mob.position.x - 1000.0).abs() > 20.0)
            );
            let with_fake = game.world.random.next_int();
            game.world.random = horizon_core::random::LevelRandom::for_level(
                horizon_core::random::LegacyRandom::new(12345),
            );
            game.stats.set_phantom_tick(0);
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .retain(|g| g.uuid != 2);
            game.spawn_phantoms();
            assert_eq!(with_fake, game.world.random.next_int());
        });
    }

    #[test]
    fn tick_mask_stats_roundtrip_snapshots_and_player_predicate() {
        use super::Stats;
        use horizon_client::advancements::{self as adv, EntityView, PlayerFacts, World};
        use std::collections::BTreeMap;

        struct TestWorld;
        impl World for TestWorld {
            fn dimension(&self) -> &str {
                "minecraft:overworld"
            }
            fn weather(&self) -> (bool, bool) {
                (false, false)
            }
            fn biome(&self, _: [i32; 3]) -> String {
                "minecraft:plains".into()
            }
            fn structures(&self, _: [i32; 3]) -> Vec<String> {
                Vec::new()
            }
            fn block(&self, _: [i32; 3]) -> (String, BTreeMap<String, String>) {
                ("minecraft:air".into(), BTreeMap::new())
            }
            fn fluid(&self, _: [i32; 3]) -> Option<String> {
                None
            }
            fn can_see_sky(&self, _: [i32; 3]) -> bool {
                true
            }
            fn light(&self, _: [i32; 3]) -> i32 {
                15
            }
            fn smokey(&self, _: [i32; 3]) -> bool {
                false
            }
            fn in_tag(&self, _: &str, _: &str, _: &str) -> bool {
                false
            }
        }

        let predicate = serde_json::json!({
            "type_specific/player": {
                "stats": [{
                    "type": "minecraft:custom",
                    "stat": "minecraft:play_time",
                    "value": { "min": 2 }
                }]
            }
        });
        let matches = |stats: &Stats| {
            let view = EntityView {
                kind: "minecraft:player".into(),
                player_facts: PlayerFacts {
                    stats: Some(stats.merged_values()),
                    ..PlayerFacts::default()
                },
                ..EntityView::default()
            };
            adv::entity(&predicate, &view, &TestWorld, [0.0; 3])
        };

        let mut stats = Stats::default();
        // Bits: play_time (0), sneak_time (3), time_since_rest (4).
        let mask = (1 << 0) | (1 << 3) | (1 << 4);
        stats.add_tick_mask(mask, false);
        stats.add_tick_mask(mask, false);

        assert_eq!(stats.get("custom", "minecraft:play_time"), 2);
        assert_eq!(stats.get("custom", "minecraft:sneak_time"), 2);
        assert_eq!(stats.get("custom", "minecraft:time_since_rest"), 2);
        assert_eq!(stats.get("custom", "minecraft:total_world_time"), 0);

        let json = stats.to_json();
        assert_eq!(json["stats"]["minecraft:custom"]["minecraft:play_time"], 2);
        assert_eq!(json["stats"]["minecraft:custom"]["minecraft:sneak_time"], 2);
        assert_eq!(
            json["stats"]["minecraft:custom"]["minecraft:time_since_rest"],
            2
        );

        let dirty = stats.take_dirty();
        assert_eq!(
            dirty,
            vec![
                ("minecraft:custom".into(), "minecraft:play_time".into(), 2),
                ("minecraft:custom".into(), "minecraft:sneak_time".into(), 2),
                (
                    "minecraft:custom".into(),
                    "minecraft:time_since_rest".into(),
                    2
                ),
            ]
        );
        assert!(matches(&stats));

        let next = Stats::from_snapshot(&serde_json::json!({
            "stats": {
                "minecraft:custom": {
                    "minecraft:play_time": 42
                }
            }
        }))
        .unwrap();
        stats.replace(next);
        assert_eq!(stats.get("custom", "minecraft:play_time"), 42);
        assert_eq!(stats.get("custom", "minecraft:sneak_time"), 0);
        assert!(matches(&stats));
        let replace_dirty = stats.take_dirty();
        assert_eq!(
            replace_dirty,
            vec![
                ("minecraft:custom".into(), "minecraft:play_time".into(), 42),
                ("minecraft:custom".into(), "minecraft:sneak_time".into(), 0),
                (
                    "minecraft:custom".into(),
                    "minecraft:time_since_rest".into(),
                    0
                ),
            ]
        );

        let cleared = Stats::from_snapshot(&serde_json::json!({ "stats": {} })).unwrap();
        stats.replace(cleared);
        assert!(stats.is_empty());
        assert_eq!(stats.get("custom", "minecraft:play_time"), 0);
        assert!(!matches(&stats));
        assert_eq!(
            stats.take_dirty(),
            vec![("minecraft:custom".into(), "minecraft:play_time".into(), 0)]
        );
    }
}
