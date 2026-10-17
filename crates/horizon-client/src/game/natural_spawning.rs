//! The game's side of `NaturalSpawner` (`mobs::spawn_selection`): the level
//! it reads, the spawning chunks and caps of `ServerChunkCache.tickChunks`,
//! and the mobs it adds (finalized with the game's `finalizeSpawn`).
use super::{Dim, Game};
use horizon_client::{
    biome::{BiomeInfo, SpawnEntry},
    blocks::BlockFacts,
    collisions::{SceneCollisions, no_collision_empty},
    mobs::{
        Mob, MobWorld, Species,
        data::{self, MobData},
        spawn_rules::{Dimension, SpawnLevel},
        spawn_selection::{
            self, Category, Counted, Heightmap, NaturalLevel, SpawnCost, SpawnPlayer, SpawnRequest,
            SpawnState, Spawned, StructureAt, StructureSpawns,
        },
    },
};
use horizon_core::{
    geometry::{Aabb, Vec3},
    random::{LegacyRandom, RandomSource, WorldgenRandom},
};
use horizon_world::registry::BlockStateId;
use std::collections::HashMap;

/// The game level as natural spawning reads it.
pub(super) struct GameLevel<'g, 'a> {
    pub(super) game: &'g Game<'a>,
    pub(super) w: MobWorld<'g>,
    /// Boxes of the mobs this pass created, not yet in the game's list.
    pub(super) pending: std::cell::RefCell<Vec<Aabb>>,
    /// The types and boxes of what a spawner created this tick.
    pub(super) pending_ids: std::cell::RefCell<Vec<(String, Aabb)>>,
}
impl GameLevel<'_, '_> {
    fn biome(&self, x: i32, y: i32, z: i32) -> Option<&BiomeInfo> {
        let chunk = self.game.world.chunks.get(&(x >> 4, z >> 4))?;
        Some(self.game.res.biomes().get(chunk.biome(x, y, z)))
    }
}
impl SpawnLevel for GameLevel<'_, '_> {
    fn state(&self, x: i32, y: i32, z: i32) -> BlockStateId {
        self.w.state(x, y, z)
    }
    fn facts(&self) -> &BlockFacts {
        self.w.facts
    }
    fn mob_data(&self) -> &MobData {
        self.w.data
    }
    fn block_tag(&self, tag: &str, block: &str) -> bool {
        self.w.block_tag(tag, block)
    }
    fn biome_tag(&self, tag: &str, x: i32, y: i32, z: i32) -> bool {
        self.w.biome_tag(tag, x, y, z)
    }
    fn sky_light(&self, x: i32, y: i32, z: i32) -> i32 {
        self.w.sky_light(x, y, z)
    }
    fn block_light(&self, x: i32, y: i32, z: i32) -> i32 {
        self.w.block_light(x, y, z)
    }
    fn sky_darken(&self) -> i32 {
        SpawnLevel::sky_darken(&self.w)
    }
    fn thundering(&self) -> bool {
        self.w.thundering
    }
    fn sea_level(&self) -> i32 {
        self.w.sea_level()
    }
    fn world_surface(&self, x: i32, z: i32) -> i32 {
        self.w.world_surface(x, z)
    }
    fn peaceful(&self) -> bool {
        self.w.peaceful()
    }
    fn dimension(&self) -> Dimension {
        self.w.dimension()
    }
    fn surface_slime_chance(&self, x: i32, y: i32, z: i32) -> f32 {
        self.w.surface_slime_chance(x, y, z)
    }
    fn world_seed(&self) -> i64 {
        self.w.world_seed
    }
    fn player_near(&self, x: f64, y: f64, z: f64, distance: f64) -> bool {
        self.w.player_near(x, y, z, distance)
    }
}
impl NaturalLevel for GameLevel<'_, '_> {
    fn min_y(&self) -> i32 {
        horizon_client::chunk::MIN_Y
    }
    fn max_y(&self) -> i32 {
        horizon_client::chunk::MIN_Y + horizon_client::chunk::HEIGHT - 1
    }
    fn ambient_light(&self) -> f32 {
        match self.game.dim {
            Dim::Nether => 0.1,
            Dim::End => 0.25,
            _ => 0.0,
        }
    }
    fn has_ceiling(&self) -> bool {
        self.game.dim == Dim::Nether
    }
    fn mob_type(&self, entity: &str) -> Option<(Species, f32, f32, Category)> {
        let t = self.game.res.mob_types().get(entity)?;
        Some((
            t.species,
            t.width as f32,
            t.height as f32,
            Category::from_name(&t.category),
        ))
    }
    fn biome_spawns(&self, x: i32, y: i32, z: i32) -> Option<&HashMap<String, Vec<SpawnEntry>>> {
        self.biome(x, y, z).map(|b| &b.spawns)
    }
    fn spawn_cost(&self, entity: &str, x: i32, y: i32, z: i32) -> Option<SpawnCost> {
        self.biome(x, y, z)?.spawn_costs.get(entity).copied()
    }
    fn creature_probability(&self, x: i32, y: i32, z: i32) -> f32 {
        self.biome(x, y, z).map_or(0.1, |b| b.creature_probability)
    }
    fn structures_at(&self, x: i32, y: i32, z: i32) -> Vec<StructureAt> {
        self.game
            .structures_reaching([x, y, z])
            .into_iter()
            .map(|(id, in_piece)| StructureAt {
                id,
                in_piece,
                in_bounds: true,
            })
            .collect()
    }
    fn structure_spawns(&self) -> &StructureSpawns {
        StructureSpawns::shared(&self.game.options.resources)
    }
    fn nearest_player_distance_sqr(&self, x: f64, y: f64, z: f64) -> Option<f64> {
        let p = self.game.player.as_ref()?;
        if self.game.spectator() {
            return None;
        }
        let d = [p.position.x - x, p.position.y - y, p.position.z - z];
        Some(d[0] * d[0] + d[1] * d[1] + d[2] * d[2])
    }
    fn respawn(&self) -> Option<[i32; 3]> {
        // The world spawn point lives in the overworld.
        if self.game.dim != Dim::Overworld {
            return None;
        }
        self.game.level.spawn.map(|s| {
            [
                s[0].floor() as i32,
                s[1].floor() as i32,
                s[2].floor() as i32,
            ]
        })
    }
    fn can_spawn_in_chunk(&self, cx: i32, cz: i32) -> bool {
        self.game.world.chunks.contains_key(&(cx, cz))
    }
    fn no_collision(&self, bounds: Aabb) -> bool {
        let mut scene = SceneCollisions {
            world: &self.game.world,
            binding: self.w.collision,
            subject: None,
        };
        // getEntityCollisions(null, box): living shulkers are solid.
        let grown = bounds.inflate(Vec3::new(1e-7, 1e-7, 1e-7));
        no_collision_empty(&mut scene, bounds)
            && !self.game.mobs.iter().any(|m| {
                m.species() == Species::Shulker
                    && m.alive()
                    && !m.removed
                    && m.bounds().intersects(grown)
            })
            && !self
                .pending_ids
                .borrow()
                .iter()
                .any(|(id, b)| id == "minecraft:shulker" && b.intersects(grown))
    }
    fn unobstructed(&self, bounds: Aabb) -> bool {
        let player = self
            .game
            .player_view()
            .is_some_and(|p| p.alive && !p.spectator && p.bounds.intersects(bounds));
        !player
            && !self.pending.borrow().iter().any(|b| b.intersects(bounds))
            && !self
                .game
                .mobs
                .iter()
                .any(|m| m.alive() && !m.removed && m.bounds().intersects(bounds))
    }
    fn spawn_mobs(&self) -> bool {
        self.game.ui.rules.bool("spawn_mobs")
    }
    /// Scanned down the column: sections of air alone are skipped (air counts
    /// for no heightmap), and an unloaded column reads as air.
    fn heightmap(&self, kind: Heightmap, x: i32, z: i32) -> i32 {
        let world = self.w.world;
        world
            .chunks
            .get(&(x >> 4, z >> 4))
            .and_then(|chunk| {
                chunk.highest(world.air.0, |y| {
                    spawn_selection::heightmap_blocks(self, kind, x, y, z)
                })
            })
            .map_or(horizon_client::chunk::MIN_Y, |y| y + 1)
    }
    fn pathfindable_land(&self, x: i32, y: i32, z: i32) -> bool {
        self.w.data.state(self.state(x, y, z)).flags & data::LAND != 0
    }
}
impl horizon_client::mobs::spawner::SpawnerLevel for GameLevel<'_, '_> {
    fn alive_player_near(&self, x: f64, y: f64, z: f64, range: f64) -> bool {
        let Some(p) = self.game.player_view() else {
            return false;
        };
        if !p.alive || p.spectator {
            return false;
        }
        let at = self.game.player.as_ref().map(|p| p.position);
        at.is_some_and(|a| {
            let d = (a.x - x).powi(2) + (a.y - y).powi(2) + (a.z - z).powi(2);
            range < 0.0 || d < range * range
        })
    }
    fn spawner_blocks_work(&self) -> bool {
        self.game.ui.rules.bool("spawner_blocks_work")
    }
    fn entity_exists(&self, id: &str) -> bool {
        self.game.res.mob_types().contains_key(id) || self.game.entity_type_exists(id)
    }
    fn count_entities(&self, id: &str, bounds: Aabb) -> usize {
        let mobs = self
            .game
            .mobs
            .iter()
            .filter(|m| !m.removed && m.kind.id == id && m.bounds().intersects(bounds))
            .count();
        let pending = self
            .pending_ids
            .borrow()
            .iter()
            .filter(|(k, b)| k == id && b.intersects(bounds))
            .count();
        mobs + pending
    }
}
impl<'a> Game<'a> {
    pub(super) fn natural_level(&self) -> GameLevel<'_, 'a> {
        GameLevel {
            game: self,
            w: self.mob_world_base(&[]),
            pending: Default::default(),
            pending_ids: Default::default(),
        }
    }
    /// Creates a mob the spawner accepted, finalized the game's way; the
    /// caller adds it with [`Game::add_spawned`].
    fn finalize_spawned(&self, r: &SpawnRequest, pending: &mut Vec<Mob>) -> Option<Spawned> {
        let kind = self.res.mob_types().get(r.entity)?.clone();
        let [x, y, z] = r.position;
        let mut mob = Mob::new(0, kind, Vec3::new(x, y, z), r.yaw);
        let facts = self.biome_facts(x.floor() as i32, y.floor() as i32, z.floor() as i32);
        let difficulty = self.local_difficulty(r.position);
        horizon_client::mobs::spawning::finalize(&mut mob, &facts, &difficulty);
        horizon_client::mobs::raider::finalize(
            &mut mob,
            horizon_client::mobs::raider::Reason::Natural,
        );
        let bounds = mob.bounds();
        pending.push(mob);
        Some(Spawned {
            bounds,
            school: true,
        })
    }
    /// Adds the mobs a spawn pass created: ids, enchanted gear and jockeys.
    fn add_spawned(&mut self, pending: Vec<Mob>, persistent: bool) {
        for mut mob in pending {
            self.next_entity += 1;
            mob.id = self.next_entity;
            mob.persistent = persistent;
            let id = mob.id;
            self.mobs.push(mob);
            self.enchant_spawn_equipment(id);
            if !persistent {
                self.spawn_jockeys(id, true);
            }
        }
    }
    /// `ServerChunkCache.tickChunks`: the spawn state over the loaded mobs,
    /// the categories under their global caps (creatures every 400 ticks),
    /// then `spawnForChunk` in the shuffled chunks within 128 blocks of the
    /// player.
    pub(super) fn spawn_natural(&mut self) {
        let Some(p) = self.player.as_ref().map(|p| p.position) else {
            return;
        };
        let (pcx, pcz) = (p.x.floor() as i32 >> 4, p.z.floor() as i32 >> 4);
        // ChunkMap.collectSpawningChunks: tracked chunks whose centre is
        // within 128 blocks of a non-spectator.
        let spectator = self.spectator();
        let mut chunks: Vec<(i32, i32)> = (-8..=8)
            .flat_map(|dx| (-8..=8).map(move |dz| (pcx + dx, pcz + dz)))
            .filter(|&(cx, cz)| {
                let (x, z) = (f64::from(cx * 16 + 8) - p.x, f64::from(cz * 16 + 8) - p.z);
                !spectator && x * x + z * z < 16384.0 && self.world.chunks.contains_key(&(cx, cz))
            })
            .collect();
        let spawn_mobs = self.ui.rules.bool("spawn_mobs");
        let spawn_enemies = self.ui.rules.bool("spawn_monsters");
        let mut random = std::mem::replace(&mut self.world.random, LegacyRandom::new(0));
        let mut pending = Vec::new();
        {
            let level = self.natural_level();
            let counted: Vec<Counted> = self
                .mobs
                .iter()
                .map(|m| Counted {
                    entity: &m.kind.id,
                    category: Category::from_name(&m.kind.category),
                    block: [
                        m.position.x.floor() as i32,
                        m.position.y.floor() as i32,
                        m.position.z.floor() as i32,
                    ],
                    is_mob: true,
                    persistent: m.persistent,
                })
                .collect();
            let players = vec![SpawnPlayer {
                position: [p.x, p.y, p.z],
                spectator,
            }];
            // A single player's natural spawn chunk tracker covers 17 x 17 chunks.
            let mut state = SpawnState::new(&level, 289, players, &counted, |cx, cz| {
                self.world.chunks.contains_key(&(cx, cz))
            });
            let categories = if spawn_mobs {
                state.filtered_categories(spawn_enemies, self.world.game_time % 400 == 0)
            } else {
                Vec::new()
            };
            // Util.shuffle.
            for i in (1..chunks.len()).rev() {
                let j = random.next_int_bound(i as i32 + 1).unwrap_or(0) as usize;
                chunks.swap(i, j);
            }
            if !categories.is_empty() {
                for chunk in chunks {
                    let mut spawn = |l: &GameLevel, r: &SpawnRequest| {
                        let spawned = l.game.finalize_spawned(r, &mut pending);
                        if let Some(s) = &spawned {
                            l.pending.borrow_mut().push(s.bounds);
                        }
                        spawned
                    };
                    spawn_selection::spawn_for_chunk(
                        &level,
                        &mut random,
                        &mut state,
                        &categories,
                        chunk,
                        &mut spawn,
                        &mut |_| {},
                    );
                }
            }
        }
        self.world.random = random;
        self.add_spawned(pending, false);
    }
    /// `NoiseBasedChunkGenerator.spawnOriginalMobs`: creature packs for a
    /// freshly generated chunk, from the biome at its corner column's top,
    /// with the chunk's decoration-seeded random (mobs are not generated in
    /// the End).
    pub(super) fn spawn_generation_creatures(&mut self, cx: i32, cz: i32) {
        if !self.world.chunks.contains_key(&(cx, cz)) || self.dim == Dim::End {
            return;
        }
        let mut random = WorldgenRandom::new(LegacyRandom::new(0));
        random.set_decoration_seed(self.level.seed, cx * 16, cz * 16);
        // The region's own random: no creature's spawn predicate draws from it.
        let mut rules = LegacyRandom::new(self.level.seed ^ (i64::from(cx) << 32 | i64::from(cz)));
        let mut pending = Vec::new();
        {
            let level = self.natural_level();
            let source = [cx * 16, level.max_y(), cz * 16];
            let mut spawn = |l: &GameLevel, r: &SpawnRequest| {
                let spawned = l.game.finalize_spawned(r, &mut pending);
                if let Some(s) = &spawned {
                    l.pending.borrow_mut().push(s.bounds);
                }
                spawned
            };
            spawn_selection::spawn_mobs_for_chunk_generation(
                &level,
                source,
                (cx, cz),
                &mut random,
                &mut rules,
                &mut spawn,
                &mut |_| {},
            );
        }
        self.add_spawned(pending, true);
    }
}
