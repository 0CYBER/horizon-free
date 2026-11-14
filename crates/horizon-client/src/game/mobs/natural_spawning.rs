//! The game's side of `NaturalSpawner` (`mobs::spawn_selection`): the level
//! it reads, the spawning chunks and caps of `ServerChunkCache.tickChunks`,
//! and the mobs it adds (finalized with the game's `finalizeSpawn`).
use super::{Dim, Game, creatures::Finalized};
use horizon_client::vanillagen::VanillaTerrain;
use horizon_client::{
    biome::{BiomeInfo, SpawnEntry},
    blocks::BlockFacts,
    collisions::{SceneCollisions, no_collision_empty},
    mobs::{
        Mob, MobWorld, Species,
        data::{self, MobData},
        seen_grid::SeenGrid,
        spawn_rules::{Reason, SpawnLevel},
        spawn_selection::{
            self, Category, Counted, Heightmap, NaturalLevel, PlayerSquares, SpawnCost,
            SpawnPlayer, SpawnRequest, SpawnState, Spawned, StructureAt, StructureSpawns,
        },
    },
};
use horizon_core::{
    geometry::{Aabb, Vec3},
    position::BlockPos,
    random::{LegacyRandom, LevelRandom, RandomSource, WorldgenRandom, XoroshiroRandom},
};
use horizon_world::dimension_type::DimensionFacts;
use mod_api::{SpawnCheck, SpawnFlags, SpawnLimits};
use std::collections::HashMap;
use {
    horizon_core::nbt::{Compound, Tag},
    horizon_world::registry::BlockStateId,
};

/// A level's natural spawning policy, as mods set it through its `LevelRef`
/// (`Game::spawn_policies`): the categories' caps and which may spawn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct SpawnPolicy {
    pub(super) limits: SpawnLimits,
    pub(super) flags: SpawnFlags,
}

/// `ServerLevel.canSpawnEntitiesInChunk` of `world`: the chunk is loaded.
pub(super) fn can_spawn_entities_in_chunk(
    world: &horizon_client::world::World,
    cx: i32,
    cz: i32,
) -> bool {
    world.chunks.contains_key(&(cx, cz))
}

/// Natural spawning's candidate chunks (`ChunkMap.collectSpawningChunks`):
/// the union of every player's 17×17 chunk square, in player then dx/dz
/// order, each chunk once. `candidates` is one pass's working list (spawning
/// shuffles and drains it); the rest is the union for the players' last chunk
/// positions, kept between ticks. A chunk crossing rebuilds only the squares
/// it can change, and proximity to players is checked live on every pass.
#[derive(Default)]
pub(super) struct SpawnScratch {
    pub(super) candidates: Vec<(i32, i32)>,
    /// Each player's chunk at the last pass.
    chunks: Vec<(i32, i32)>,
    /// For each player, the others whose squares overlap its own, in order.
    neighbours: Vec<Vec<usize>>,
    /// The chunks each player adds to the union: those of its square, in
    /// dx/dz order, that no earlier player's square holds.
    segments: Vec<Vec<(i32, i32)>>,
    stale: Vec<bool>,
}
impl SpawnScratch {
    fn chunk(p: &LevelPlayer) -> (i32, i32) {
        (
            p.position.x.floor() as i32 >> 4,
            p.position.z.floor() as i32 >> 4,
        )
    }
    /// Whether the square around chunk `centre` holds chunk `c`.
    fn holds(centre: (i32, i32), c: (i32, i32)) -> bool {
        centre.0.abs_diff(c.0) <= 8 && centre.1.abs_diff(c.1) <= 8
    }
    fn overlap(a: (i32, i32), b: (i32, i32)) -> bool {
        a.0.abs_diff(b.0) <= 16 && a.1.abs_diff(b.1) <= 16
    }
    /// Bring the segments up to date with the players' chunks. A segment
    /// depends only on its player's chunk and those of the earlier players
    /// whose squares overlap it, so a move rebuilds the mover's segment and
    /// the later ones its old or new square overlaps.
    fn prepare(&mut self, players: &[LevelPlayer]) {
        let n = players.len();
        let all = n != self.chunks.len();
        self.stale.clear();
        self.stale.resize(n, all);
        if !all {
            let mut moved = false;
            for m in 0..n {
                let (old, new) = (self.chunks[m], Self::chunk(&players[m]));
                if old == new {
                    continue;
                }
                moved = true;
                self.stale[m] = true;
                for (i, p) in players.iter().enumerate().skip(m + 1) {
                    let c = Self::chunk(p);
                    self.stale[i] |= Self::overlap(old, c) || Self::overlap(new, c);
                }
            }
            if !moved {
                return;
            }
        }
        self.chunks.clear();
        self.chunks.extend(players.iter().map(Self::chunk));
        self.neighbours.resize_with(n, Vec::new);
        self.neighbours.truncate(n);
        self.segments.resize_with(n, Vec::new);
        self.segments.truncate(n);
        let chunks = &self.chunks;
        for (i, near) in self.neighbours.iter_mut().enumerate() {
            near.clear();
            near.extend((0..n).filter(|&j| j != i && Self::overlap(chunks[i], chunks[j])));
        }
        for (i, segment) in self.segments.iter_mut().enumerate() {
            if !self.stale[i] {
                continue;
            }
            let near = &self.neighbours[i];
            let earlier = &near[..near.partition_point(|&j| j < i)];
            let (pcx, pcz) = chunks[i];
            segment.clear();
            for dx in -8..=8 {
                for dz in -8..=8 {
                    let c = (pcx + dx, pcz + dz);
                    if !earlier.iter().any(|&j| Self::holds(chunks[j], c)) {
                        segment.push(c);
                    }
                }
            }
        }
    }
    /// Fill `candidates` with the union's chunks, in order, that are within
    /// 128 blocks of a non-spectator holding them in its square and that
    /// `keep` accepts. Returns the union's size.
    fn collect(
        &mut self,
        players: &[LevelPlayer],
        mut forced: impl FnMut((i32, i32)) -> bool,
        mut keep: impl FnMut((i32, i32)) -> bool,
    ) -> i32 {
        self.prepare(players);
        self.candidates.clear();
        let mut count = 0;
        for (i, segment) in self.segments.iter().enumerate() {
            count += segment.len();
            let near = &self.neighbours[i];
            for &c in segment {
                let close = close_enough_for_spawning(&players[i], c)
                    || near.iter().any(|&j| {
                        Self::holds(self.chunks[j], c) && close_enough_for_spawning(&players[j], c)
                    })
                    || forced(c);
                if close && keep(c) {
                    self.candidates.push(c);
                }
            }
        }
        count as i32
    }
}

/// A player in the level in place, as spawning and despawning see it.
pub(super) struct LevelPlayer {
    pub(super) uuid: u128,
    pub(super) position: Vec3,
    pub(super) bounds: Aabb,
    pub(super) alive: bool,
    pub(super) creative: bool,
    pub(super) spectator: bool,
}
impl LevelPlayer {
    fn distance_sqr(&self, x: f64, y: f64, z: f64) -> f64 {
        (self.position.x - x).powi(2)
            + (self.position.y - y).powi(2)
            + (self.position.z - z).powi(2)
    }
    /// `Level.hasNearbyAlivePlayer`'s test of one player (any distance for a
    /// negative range).
    fn alive_near(&self, x: f64, y: f64, z: f64, range: f64) -> bool {
        self.alive && !self.spectator && (range < 0.0 || self.distance_sqr(x, y, z) < range * range)
    }
}
/// `ChunkMap.euclideanDistanceSquared` below 128 blocks
/// (`playerIsCloseEnoughForSpawning`).
fn close_enough_for_spawning(p: &LevelPlayer, (cx, cz): (i32, i32)) -> bool {
    let (x, z) = (
        f64::from(cx * 16 + 8) - p.position.x,
        f64::from(cz * 16 + 8) - p.position.z,
    );
    !p.spectator && x * x + z * z < 16384.0
}
/// The chunks within 8 of a player, in `scratch.candidates`, and for each in
/// `scratch.close` whether a non-spectator is within 128 blocks or `forced`
/// says a ticket holds it for natural spawning. How many there are.
/// `EntityGetter.getNearestPlayer(x, y, z, -1, false)`: the nearest
/// non-spectator, alive or not.
pub(super) fn nearest_player(
    players: &[LevelPlayer],
    x: f64,
    y: f64,
    z: f64,
) -> Option<&LevelPlayer> {
    let mut best: Option<(&LevelPlayer, f64)> = None;
    for p in players.iter().filter(|p| !p.spectator) {
        let d = p.distance_sqr(x, y, z);
        if best.is_none_or(|(_, b)| d < b) {
            best = Some((p, d));
        }
    }
    best.map(|(p, _)| p)
}

/// [`nearest_player`] for many positions: the non-spectators by the 64-block
/// square they stand in, searched ring by ring outwards from the position.
/// Every mob asks each tick, and with players spread over the map that was a
/// look at every player for every mob.
pub(super) struct NearestPlayers<'p> {
    players: &'p [LevelPlayer],
    cells: horizon_client::fast_hash::FxHashMap<(i64, i64), Vec<usize>>,
    /// The cells' bounding box: (x0, z0, x1, z1).
    bounds: (i64, i64, i64, i64),
    /// Few players, or one too far out for cells: every question scans.
    scan: bool,
}
impl<'p> NearestPlayers<'p> {
    const CELL: f64 = 64.0;
    /// Further out than any world border: such positions are answered by a scan.
    const FAR: f64 = 1.0e9;
    pub(super) fn new(players: &'p [LevelPlayer]) -> Self {
        let mut cells = horizon_client::fast_hash::FxHashMap::<(i64, i64), Vec<usize>>::default();
        let mut bounds = (i64::MAX, i64::MAX, i64::MIN, i64::MIN);
        let mut scan = players.len() <= 8;
        for (i, p) in players.iter().enumerate().filter(|(_, p)| !p.spectator) {
            if !(p.position.x.abs() < Self::FAR && p.position.z.abs() < Self::FAR) {
                // Past any world border (or not a number): scans answer.
                scan = true;
                break;
            }
            let key = Self::cell(p.position.x, p.position.z);
            cells.entry(key).or_default().push(i);
            bounds = (
                bounds.0.min(key.0),
                bounds.1.min(key.1),
                bounds.2.max(key.0),
                bounds.3.max(key.1),
            );
        }
        Self {
            players,
            cells,
            bounds,
            scan,
        }
    }
    fn cell(x: f64, z: f64) -> (i64, i64) {
        (
            (x / Self::CELL).floor() as i64,
            (z / Self::CELL).floor() as i64,
        )
    }
    /// The same player [`nearest_player`] finds: the nearest, the first in
    /// the list among equals.
    pub(super) fn nearest(&self, x: f64, y: f64, z: f64) -> Option<&'p LevelPlayer> {
        if self.scan || !(x.abs() < Self::FAR && z.abs() < Self::FAR) {
            return nearest_player(self.players, x, y, z);
        }
        if self.cells.is_empty() {
            return None;
        }
        let (qx, qz) = Self::cell(x, z);
        let (x0, z0, x1, z1) = self.bounds;
        let last = (qx - x0)
            .abs()
            .max((qx - x1).abs())
            .max((qz - z0).abs())
            .max((qz - z1).abs());
        let mut best: Option<(usize, f64)> = None;
        let mut looked = 0usize;
        for ring in 0..=last {
            let mut look = |kx: i64, kz: i64| {
                looked += 1;
                for &i in self.cells.get(&(kx, kz)).into_iter().flatten() {
                    let d = self.players[i].distance_sqr(x, y, z);
                    if best.is_none_or(|(b, bd)| d < bd || d == bd && i < b) {
                        best = Some((i, d));
                    }
                }
            };
            if ring == 0 {
                look(qx, qz);
            } else {
                for k in -ring..=ring {
                    look(qx + k, qz - ring);
                    look(qx + k, qz + ring);
                }
                for k in 1 - ring..ring {
                    look(qx - ring, qz + k);
                    look(qx + ring, qz + k);
                }
            }
            // Everyone in a further ring is more than `ring` cells away on one
            // axis, so further than the one found.
            let bound = ring as f64 * Self::CELL;
            if best.is_some_and(|(_, d)| d <= bound * bound) {
                break;
            }
            if looked > self.players.len() * 4 {
                return nearest_player(self.players, x, y, z);
            }
        }
        best.map(|(i, _)| &self.players[i])
    }
}

/// Up to this many players, a spawn start measures to every one.
const PLAYERS_SCANNED: usize = 8;
/// The game level as natural spawning reads it.
pub(super) struct GameLevel<'g, 'a> {
    pub(super) game: &'g Game<'a>,
    pub(super) w: MobWorld<'g>,
    /// Every player in the level ([`Game::level_players`]).
    pub(super) players: Vec<LevelPlayer>,
    /// Where the non-spectators among them are (`getNearestPlayer`).
    pub(super) nearest: Vec<[f64; 3]>,
    /// `nearest` by square, built when a spawn start first asks with more
    /// than [`PLAYERS_SCANNED`] players.
    pub(super) player_squares: std::cell::OnceCell<PlayerSquares>,
    /// Boxes of the mobs this pass created, not yet in the game's list.
    pub(super) pending: std::cell::RefCell<Vec<Aabb>>,
    /// The types and boxes of what a spawner created this tick.
    pub(super) pending_ids: std::cell::RefCell<Vec<(String, Aabb)>>,
    /// The mobs this pass's finalizeSpawn calls made (jockeys and mounts),
    /// and the chicken-jockey links on mobs already in the game's list, both
    /// applied by [`Game::add_spawned`].
    pub(super) spawned: std::cell::RefCell<Vec<Mob>>,
    pub(super) rides: std::cell::RefCell<Vec<(u32, u32)>>,
    /// How many provisional ids the pass has handed out (`u32::MAX - n`),
    /// mapped to real ones by [`Game::add_spawned`].
    pub(super) provisional: std::cell::Cell<u32>,
    /// The game's mobs by box, built once a pass has asked more than
    /// [`SCANNED_QUERIES`] box queries (the mobs do not change while it
    /// holds the game); the first ones scan, as the build costs a few scans.
    pub(super) boxes: std::cell::OnceCell<MobBoxes>,
    pub(super) box_queries: std::cell::Cell<u32>,
    /// The entity types some biome of the level gives a spawn cost (the
    /// nether's few): for any other the cost is none wherever it stands, and
    /// spawning asks for one for every loaded mob each tick.
    pub(super) costed: Vec<&'g str>,
    /// The spawner block whose tick this pass is, told to the spawn checks;
    /// None for natural spawning and the other passes.
    pub(super) spawner: Option<mod_api::Spawner>,
    /// The mob a listened position check created and lent, which the spawn
    /// that follows takes rather than creating it again ([`Checked`]).
    checked: std::cell::RefCell<Option<Checked>>,
}
/// The mob a listened `mob_spawn_position_check` created for its lend, held
/// until the spawn takes it, and the provisional count before it was
/// created: a refused check drops the mob and gives the id back, so a refused
/// attempt leaves the pass as an unlistened one would.
struct Checked {
    mob: Mob,
    provisional: u32,
}
const SCANNED_QUERIES: u32 = 8;
/// A mob not removed: its index in `Game::mobs`, box, alive, and whether
/// it is a living shulker.
type MobBox = (usize, Aabb, bool, bool);
fn mob_box(i: usize, m: &Mob) -> MobBox {
    let alive = m.alive();
    (
        i,
        m.bounds(),
        alive,
        alive && m.species() == Species::Shulker,
    )
}
/// The boxes of the game's mobs not removed, and a grid over them.
pub(super) struct MobBoxes {
    entries: Vec<MobBox>,
    grid: SeenGrid,
}
impl MobBoxes {
    fn new(mobs: &[Mob]) -> Self {
        let entries: Vec<_> = mobs
            .iter()
            .enumerate()
            .filter(|(_, m)| !m.removed)
            .map(|(i, m)| mob_box(i, m))
            .collect();
        let grid = SeenGrid::new(entries.iter().map(|e| e.1));
        Self { entries, grid }
    }
    /// The entries whose boxes meet `area` (and possibly a few more).
    fn near(&self, area: Aabb) -> impl Iterator<Item = MobBox> + '_ {
        let mut near = Vec::new();
        let indexed = self.grid.candidates(area, &mut near);
        let all = (!indexed).then(|| self.entries.iter());
        near.into_iter()
            .map(|i| self.entries[i as usize])
            .chain(all.into_iter().flatten().copied())
    }
}
impl GameLevel<'_, '_> {
    /// The mobs not removed whose boxes may meet `area` (callers test them).
    fn mobs_near(&self, area: Aabb) -> Box<dyn Iterator<Item = MobBox> + '_> {
        let asked = self.box_queries.get() + 1;
        self.box_queries.set(asked);
        if asked <= SCANNED_QUERIES && self.boxes.get().is_none() {
            return Box::new(
                self.game
                    .mobs
                    .iter()
                    .enumerate()
                    .filter(|(_, m)| !m.removed)
                    .map(|(i, m)| mob_box(i, m)),
            );
        }
        Box::new(
            self.boxes
                .get_or_init(|| MobBoxes::new(&self.game.mobs))
                .near(area),
        )
    }
    fn biome(&self, x: i32, y: i32, z: i32) -> Option<&BiomeInfo> {
        let chunk = self.game.world.chunks.get(&(x >> 4, z >> 4))?;
        Some(world_biomes!(self.game).get(chunk.biome(x, y, z)))
    }
    /// The block whose noise cell `BiomeManager.getBiome` reads.
    fn zoomed(&self, x: i32, y: i32, z: i32) -> [i32; 3] {
        let zoom = self.game.streaming.biome_zoom;
        let [qx, qy, qz] = zoom.quart_at_block(horizon_core::position::BlockPos::new(x, y, z));
        [qx << 2, qy << 2, qz << 2]
    }
    fn zoomed_biome(&self, x: i32, y: i32, z: i32) -> Option<&BiomeInfo> {
        let [x, y, z] = self.zoomed(x, y, z);
        self.biome(x, y, z)
    }
    /// A provisional id, distinct within the pass and mapped to a real one
    /// by [`Game::add_spawned`].
    pub(super) fn provisional_id(&self) -> u32 {
        let n = self.provisional.get();
        self.provisional.set(n + 1);
        u32::MAX - n
    }
    /// The handle of this pass's level, which the spawn checks are told.
    fn level_ref(&self) -> mod_api::LevelRef {
        self.game
            .level_ref(self.game.lend_epoch(), self.game.dim.key())
    }
    /// The mob the position check of this spawn created, when one did and it
    /// is the one asked for (`entity` at `position`, facing `yaw`): the spawn
    /// goes on with it rather than creating it again.
    pub(super) fn take_checked(&self, entity: &str, position: [f64; 3], yaw: f32) -> Option<Mob> {
        let Checked { mob, .. } = self.checked.take()?;
        let at = [mob.position.x, mob.position.y, mob.position.z];
        (mob.kind.id == entity && at == position && mob.yaw == yaw).then_some(mob)
    }
    /// Creates a mob the spawner accepted, finalized the game's way
    /// (`finalizeSpawn` on this pass's level, its draws on the spawner's own
    /// random, between its position draws, as vanilla interleaves them); the
    /// caller adds it with [`Game::add_spawned`].
    fn finalize_spawned<R: RandomSource>(
        &self,
        r: &SpawnRequest,
        pending: &mut Vec<Mob>,
        random: &mut R,
    ) -> Option<Spawned> {
        let mut mob = match self.take_checked(r.entity, r.position, r.yaw) {
            Some(mob) => mob,
            None => {
                let kind = self.game.res.mob_types().get(r.entity)?.clone();
                let [x, y, z] = r.position;
                Mob::new(self.provisional_id(), kind, Vec3::new(x, y, z), r.yaw)
            }
        };
        // No group data is threaded through the pack yet: each member starts
        // without, and finalization reads what a listener leaves.
        let mut group = None;
        let (outcome, difficulty) = self
            .game
            .finalize_spawn_event(&mut mob, r.reason, None, &mut group);
        match outcome {
            Finalized::Cancelled => return None,
            Finalized::Skipped => {}
            Finalized::Done => {
                let mut level = PassLevel::new(self);
                let mut cx = horizon_client::mobs::finalize::Ctx {
                    level: &mut level,
                    random,
                    difficulty,
                    created: Vec::new(),
                    jockey_rider: None,
                };
                horizon_client::mobs::finalize::finalize(&mut cx, &mut mob, r.reason, group);
                for m in cx.created {
                    let bounds = m.bounds();
                    self.pending.borrow_mut().push(bounds);
                    self.pending_ids
                        .borrow_mut()
                        .push((m.kind.id.clone(), bounds));
                    self.spawned.borrow_mut().push(m);
                }
            }
        }
        let bounds = mob.bounds();
        let vanilla = spawn_selection::max_cluster_size(r.species);
        let cluster_size = self
            .game
            .spawn_cluster_size(&mut mob, r.reason, None, vanilla);
        pending.push(mob);
        Some(Spawned {
            bounds,
            school: true,
            cluster_size,
        })
    }
}

/// A spawning pass's [`finalize::FinalizeLevel`]: the game's mobs and
/// conditions read-only, what finalizeSpawn makes (jockeys, mounts) recorded
/// for [`Game::add_spawned`], as the pass holds the game immutably.
pub(super) struct PassLevel<'a, 'g, 'h> {
    l: &'a GameLevel<'g, 'h>,
}
impl<'a, 'g, 'h> PassLevel<'a, 'g, 'h> {
    pub(super) fn new(l: &'a GameLevel<'g, 'h>) -> Self {
        Self { l }
    }
}
impl horizon_client::mobs::finalize::FinalizeLevel for PassLevel<'_, '_, '_> {
    fn mob_maps(&self) -> &horizon_client::neoforge_mob_maps::NeoForgeMobMaps {
        self.l.game.mob_maps()
    }
    fn spawn_event(
        &mut self,
        mob: &mut horizon_client::mobs::Mob,
        reason: horizon_client::mobs::spawn_rules::Reason,
    ) -> horizon_client::mobs::finalize::SpawnEvent {
        self.l.game.finalize_spawn_event_result(mob, reason)
    }
    fn biome(&self, pos: [i32; 3]) -> String {
        // A spawn site without its chunk (a loaded marker mob) is neutral.
        super::finalize_spawn::biome_or_plains(
            self.l.biome(pos[0], pos[1], pos[2]).map(|b| b.name.clone()),
        )
    }
    fn biome_tag(&self, tag: &str, pos: [i32; 3]) -> bool {
        let Some(b) = self.l.biome(pos[0], pos[1], pos[2]) else {
            return false;
        };
        self.l.game.res.mob_data().biome_has(tag, &b.name)
    }
    fn structures_with_piece_at(&self, pos: [i32; 3]) -> Vec<String> {
        self.l.game.structures_at(pos)
    }
    fn moon_phase(&self, _: [i32; 3]) -> usize {
        self.l.game.finalize_moon_phase()
    }
    fn spawning_monsters(&self) -> bool {
        self.l.game.spawning_monsters()
    }
    fn no_collision(&self, bounds: Aabb) -> bool {
        // The same answer the pass's `GameLevel` gives: block collisions with
        // the shulkers in the level and the pass's own adds.
        let mut scene = SceneCollisions {
            world: &self.l.game.world,
            binding: self.l.w.collision,
            subject: None,
        };
        let grown = bounds.inflate(Vec3::new(1e-7, 1e-7, 1e-7));
        no_collision_empty(&mut scene, bounds)
            && !self
                .l
                .mobs_near(grown)
                .any(|(_, b, _, shulker)| shulker && b.intersects(grown))
            && !self
                .l
                .pending_ids
                .borrow()
                .iter()
                .any(|(id, b)| id == "minecraft:shulker" && b.intersects(grown))
    }
    fn free_chicken(&self, bounds: Aabb) -> Option<u32> {
        self.l.game.finalize_free_chicken(bounds)
    }
    fn ride_chicken(&mut self, chicken: u32, rider: u32) {
        self.l.rides.borrow_mut().push((chicken, rider));
    }
    fn game_time(&self) -> i64 {
        self.l.game.finalize_game_time()
    }
    fn halloween(&self) -> bool {
        self.l.game.finalize_halloween()
    }
    fn create(&mut self, kind: &str, _: horizon_client::mobs::spawn_rules::Reason) -> Option<Mob> {
        use horizon_world::food::Difficulty;
        if self.l.game.world.health_rules.difficulty == Difficulty::Peaceful
            && !horizon_client::mobs::finalize::allowed_in_peaceful(kind)
        {
            return None;
        }
        let t = self.l.game.res.mob_types().get(kind)?.clone();
        Some(Mob::new(
            self.l.provisional_id(),
            t,
            horizon_core::geometry::Vec3::ZERO,
            0.0,
        ))
    }
    fn spawn_data(&self) -> horizon_client::mobs::finalize::SpawnData<'_> {
        self.l.game.finalize_spawn_data()
    }
}
impl SpawnLevel for GameLevel<'_, '_> {
    fn placement_check(&self, entity: &str, reason: Reason, at: [i32; 3], vanilla: bool) -> bool {
        if !mod_api::__private::listened(mod_api::HookId::MobSpawnPlacementCheck) {
            return vanilla;
        }
        let mut result = SpawnCheck::Default;
        self.game.native_mob_spawn_placement_check(
            mod_api::__private::MobSpawnPlacementCheckCall {
                dimension: self.game.dim.id(),
                kind: entity,
                reason,
                pos: BlockPos::new(at[0], at[1], at[2]),
                default_result: vanilla,
                level: self.level_ref(),
                spawner: self.spawner,
                result: &mut result,
            },
        );
        match result {
            SpawnCheck::Succeed => true,
            SpawnCheck::Fail => false,
            _ => vanilla,
        }
    }
    fn within_world_border(&self, x: i32, z: i32) -> bool {
        self.game
            .border
            .geometry()
            .is_within_bounds(f64::from(x), f64::from(z), 0.0)
    }
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
    // Level.getBiome: the zoomed biome.
    fn biome_tag(&self, tag: &str, x: i32, y: i32, z: i32) -> bool {
        let [x, y, z] = self.zoomed(x, y, z);
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
    fn dimension_type(&self) -> &DimensionFacts {
        self.game.dimension_type()
    }
    fn surface_slime_chance(&self, x: i32, y: i32, z: i32) -> f32 {
        self.w.surface_slime_chance(x, y, z)
    }
    fn world_seed(&self) -> i64 {
        self.w.world_seed
    }
    /// `getNearestPlayer(x, y, z, distance, true)`, over every player.
    fn player_near(&self, x: f64, y: f64, z: f64, distance: f64) -> bool {
        self.players.iter().any(|p| {
            p.alive && !p.creative && !p.spectator && p.distance_sqr(x, y, z) < distance * distance
        })
    }
}
impl NaturalLevel for GameLevel<'_, '_> {
    fn spawn_hooks(&self) -> bool {
        !mod_api::__private::mods().is_empty()
    }
    /// Vanilla creates the mob before its position checks, so with a listener
    /// the mob a fresh spawn is about to create is created here, the way the
    /// spawn creates it, lent to the listeners, and handed on to the spawn
    /// ([`GameLevel::take_checked`]); a refused check drops it
    /// ([`NaturalLevel::position_refused`]). Creating it draws nothing from
    /// the level's random. A site that does not create the mob fresh (a
    /// spawner's entity data, chunk generation's packs) lends its kind and
    /// position only. With no listener nothing is created.
    fn position_check(
        &self,
        entity: &str,
        reason: Reason,
        at: [f64; 3],
        pending: spawn_selection::Pending,
    ) -> SpawnCheck {
        let mut result = SpawnCheck::Default;
        if !mod_api::__private::listened(mod_api::HookId::MobSpawnPositionCheck) {
            return result;
        }
        // An earlier check's mob that no spawn took is not this one.
        self.checked.take();
        let provisional = self.provisional.get();
        let mut mob = pending
            .fresh
            .then(|| self.game.res.mob_types().get(entity))
            .flatten()
            .map(|kind| {
                let [x, y, z] = at;
                Mob::new(
                    self.provisional_id(),
                    kind.clone(),
                    Vec3::new(x, y, z),
                    pending.yaw,
                )
            });
        let level = self.level_ref();
        self.game
            .lend_unspawned(entity, at, mob.as_mut(), |handle, _| {
                self.game.native_mob_spawn_position_check(
                    mod_api::__private::MobSpawnPositionCheckCall {
                        dimension: self.game.dim.id(),
                        kind: entity,
                        reason,
                        pos: at,
                        level,
                        spawner: self.spawner,
                        mob: handle,
                        result: &mut result,
                    },
                );
            });
        if let Some(mob) = mob {
            *self.checked.borrow_mut() = Some(Checked { mob, provisional });
        }
        result
    }
    fn position_refused(&self) {
        if let Some(refused) = self.checked.take() {
            self.provisional.set(refused.provisional);
        }
    }
    fn potential_spawns(&self, category: Category, pos: [i32; 3], entries: &mut Vec<SpawnEntry>) {
        let mut canceled = false;
        self.game.native_potential_spawns(
            self.game.dim.id(),
            category.name(),
            pos,
            entries,
            &mut canceled,
        );
        if canceled {
            entries.clear();
        }
    }
    fn min_y(&self) -> i32 {
        self.w.world.bottom_y()
    }
    fn max_y(&self) -> i32 {
        self.w.world.top_y() - 1
    }
    fn ambient_light(&self) -> f32 {
        self.game.dimension_type().ambient_light
    }
    fn has_ceiling(&self) -> bool {
        self.w.world.shape.has_ceiling
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
    // The `natural_mob_spawns` attribute reads the zoomed biome
    // (fullResolutionBiomes), the spawn probability the noise cell.
    fn biome_spawns(&self, x: i32, y: i32, z: i32) -> Option<&HashMap<String, Vec<SpawnEntry>>> {
        self.zoomed_biome(x, y, z).map(|b| &b.spawns)
    }
    fn spawn_cost(&self, entity: &str, x: i32, y: i32, z: i32) -> Option<SpawnCost> {
        if !self.costed.contains(&entity) {
            return None;
        }
        self.zoomed_biome(x, y, z)?.spawn_costs.get(entity).copied()
    }
    fn creature_probability(&self, x: i32, y: i32, z: i32) -> f32 {
        self.biome(x, y, z).map_or(0.1, |b| b.creature_probability)
    }
    fn structures_at(
        &self,
        x: i32,
        y: i32,
        z: i32,
        wanted: &dyn Fn(&str) -> bool,
    ) -> Vec<StructureAt> {
        self.game
            .structures_reaching_where([x, y, z], wanted)
            .into_iter()
            .map(|(id, in_piece)| StructureAt {
                id,
                in_piece,
                in_bounds: true,
            })
            .collect()
    }
    fn structure_spawns(&self) -> &StructureSpawns {
        &self.game.structure_spawns
    }
    fn nearest_player_positions(&self) -> &[[f64; 3]] {
        &self.nearest
    }
    fn player_positions_near(&self, x: f64, z: f64, extent: f64, out: &mut Vec<[f64; 3]>) {
        if self.nearest.len() <= PLAYERS_SCANNED {
            out.extend_from_slice(&self.nearest);
            return;
        }
        let squares = (self.player_squares).get_or_init(|| PlayerSquares::new(&self.nearest));
        squares.near(x, z, extent, out);
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
    /// `ServerLevel.canSpawnEntitiesInChunk`: the whole chunk is inside the border.
    fn can_spawn_in_chunk(&self, cx: i32, cz: i32) -> bool {
        self.game.world.chunks.contains_key(&(cx, cz))
            && self.within_world_border(cx << 4, cz << 4)
            && self.within_world_border((cx << 4).wrapping_add(15), (cz << 4).wrapping_add(15))
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
            && !self
                .mobs_near(grown)
                .any(|(_, b, _, shulker)| shulker && b.intersects(grown))
            && !self
                .pending_ids
                .borrow()
                .iter()
                .any(|(id, b)| id == "minecraft:shulker" && b.intersects(grown))
    }
    fn unobstructed(&self, bounds: Aabb) -> bool {
        let player = self
            .players
            .iter()
            .any(|p| p.alive && !p.spectator && p.bounds.intersects(bounds));
        !player
            && !self.pending.borrow().iter().any(|b| b.intersects(bounds))
            && !self
                .mobs_near(bounds)
                .any(|(_, b, alive, _)| alive && b.intersects(bounds))
    }
    fn spawn_mobs(&self) -> bool {
        self.game
            .ui
            .rules
            .bool_at(horizon_client::gamerules::vanilla::SPAWN_MOBS)
    }
    /// WORLD_SURFACE (every spawn attempt's) is the chunk's kept heightmap.
    /// The others are scanned down the column: sections of air alone are
    /// skipped (air counts for no heightmap). An unloaded column reads as air.
    fn heightmap(&self, kind: Heightmap, x: i32, z: i32) -> i32 {
        let world = self.w.world;
        if kind == Heightmap::WorldSurface {
            return self.w.world_surface(x, z);
        }
        world
            .chunks
            .get(&(x >> 4, z >> 4))
            .and_then(|chunk| {
                chunk.highest(world.air.0, |y| {
                    spawn_selection::heightmap_blocks(self, kind, x, y, z)
                })
            })
            .map_or(self.w.world.bottom_y(), |y| y + 1)
    }
    fn pathfindable_land(&self, x: i32, y: i32, z: i32) -> bool {
        self.w
            .data
            .pathfindable(self.state(x, y, z), data::PathComputationType::Land)
    }
}
impl horizon_client::mobs::spawner::SpawnerLevel for GameLevel<'_, '_> {
    /// `Level.hasNearbyAlivePlayer`, over every player.
    fn alive_player_near(&self, x: f64, y: f64, z: f64, range: f64) -> bool {
        self.players.iter().any(|p| p.alive_near(x, y, z, range))
    }
    fn spawner_blocks_work(&self) -> bool {
        self.game
            .ui
            .rules
            .bool_at(horizon_client::gamerules::vanilla::SPAWNER_BLOCKS_WORK)
    }
    fn entity_exists(&self, id: &str) -> bool {
        self.game.res.mob_types().contains_key(id) || self.game.entity_type_exists(id)
    }
    fn count_entities(&self, id: &str, bounds: Aabb) -> usize {
        let mobs = self
            .mobs_near(bounds)
            .filter(|&(i, b, ..)| self.game.mobs[i].kind.id == id && b.intersects(bounds))
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
/// Every entity type with a spawn cost in some biome, each once.
fn spawn_cost_types(biomes: &horizon_client::biome::Biomes) -> Vec<&str> {
    let mut types: Vec<&str> = Vec::new();
    for t in biomes.list.iter().flat_map(|b| b.spawn_costs.keys()) {
        if !types.contains(&t.as_str()) {
            types.push(t);
        }
    }
    types
}
impl<'a> Game<'a> {
    /// The natural spawning policy of the level `dimension`: vanilla's until a
    /// mod sets one.
    pub(super) fn spawn_policy(&self, dimension: mod_api::DimensionKey) -> SpawnPolicy {
        self.spawn_policies
            .get(&dimension)
            .copied()
            .unwrap_or_default()
    }
    pub(super) fn natural_level(&self) -> GameLevel<'_, 'a> {
        let players = self.level_players();
        let nearest = players
            .iter()
            .filter(|p| !p.spectator)
            .map(|p| [p.position.x, p.position.y, p.position.z])
            .collect();
        GameLevel {
            game: self,
            w: self.mob_world_base(&[]),
            players,
            nearest,
            player_squares: Default::default(),
            pending: Default::default(),
            pending_ids: Default::default(),
            spawned: Default::default(),
            rides: Default::default(),
            provisional: Default::default(),
            boxes: Default::default(),
            box_queries: Default::default(),
            costed: spawn_cost_types(world_biomes!(self)),
            spawner: None,
            checked: Default::default(),
        }
    }
    /// Gives a spawn pass's provisional mobs (the `u32::MAX - n` ids
    /// [`GameLevel`] hands out) their real ids in order, rewrites the mount
    /// and rider links that reference them, and seats chicken jockeys on the
    /// chickens already in the level. The old→new map comes back for the
    /// pass's events.
    pub(super) fn resolve_spawn_ids(
        &mut self,
        mut groups: [&mut [Mob]; 2],
        rides: &[(u32, u32)],
    ) -> HashMap<u32, u32> {
        let mut links: HashMap<u32, u32> = HashMap::new();
        for group in groups.iter_mut() {
            for mob in group.iter_mut() {
                self.next_entity += 1;
                links.insert(mob.id, self.next_entity);
                mob.id = self.next_entity;
            }
        }
        for group in groups.iter_mut() {
            for mob in group.iter_mut() {
                if let Some(m) = mob.extra.mount.as_mut()
                    && let Some(&id) = links.get(m)
                {
                    *m = id;
                }
                if let Some(m) = mob.extra.rider.as_mut()
                    && let Some(&id) = links.get(m)
                {
                    *m = id;
                }
            }
        }
        for (chicken, rider) in rides {
            let rider = links.get(rider).copied().unwrap_or(*rider);
            if let Some(c) = self.mobs.iter_mut().find(|m| m.id == *chicken) {
                c.extra.jockey = true;
                c.extra.rider = Some(rider);
            }
        }
        links
    }
    /// Adds the mobs a spawn pass created: ids and the links that need them
    /// (finalizeSpawn's mounts ride by id, and its chickens carry their
    /// zombie). Every mob arrives with a provisional id; real ones are
    /// assigned in spawn order and the links rewritten.
    pub(super) fn add_spawned(
        &mut self,
        pending: Vec<Mob>,
        created: Vec<Mob>,
        rides: Vec<(u32, u32)>,
        persistent: bool,
    ) {
        let _save_lease = horizon_client::join_seq::visibility::PassLease::enter();
        let mut pending = pending;
        let mut created = created;
        for m in pending.iter_mut() {
            m.persistent = persistent;
        }
        self.resolve_spawn_ids([&mut pending, &mut created], &rides);
        for mob in pending {
            self.mobs.push(mob);
            self.native_mobs_joined(self.mobs.len() - 1, true);
        }
        for mob in created {
            self.mobs.push(mob);
            self.native_mobs_joined(self.mobs.len() - 1, true);
        }
    }
    /// Every player in the level in place: the local player, and the LAN
    /// guests or server players in it.
    pub(super) fn level_players(&self) -> Vec<LevelPlayer> {
        self.level_players_iter().collect()
    }
    /// `level_players` one at a time, without collecting them.
    pub(super) fn level_players_iter(&self) -> impl Iterator<Item = LevelPlayer> + '_ {
        self.local_level_player()
            .into_iter()
            .chain(self.level_guests().map(|g| LevelPlayer {
                uuid: g.uuid,
                position: Vec3::new(g.position[0], g.position[1], g.position[2]),
                bounds: super::lan_mobs::guest_bounds(g),
                alive: !g.dead,
                creative: g.creative(),
                spectator: g.spectator(),
            }))
    }
    fn local_level_player(&self) -> Option<LevelPlayer> {
        self.player_view().map(|p| LevelPlayer {
            uuid: horizon_client::lan::playerdata::ints_uuid(p.uuid),
            position: p.position,
            bounds: p.bounds,
            alive: p.alive,
            creative: p.creative,
            spectator: p.spectator,
        })
    }
    /// The UUIDs of the players in level `dimension`, for a query that only
    /// names them: no `PlayerView`, so nothing is allocated per player. For the
    /// level in place they are `level_players`', in its order; a parked level
    /// has only its guests.
    pub(super) fn level_player_uuids<'s>(
        &'s self,
        dimension: &'s str,
    ) -> impl Iterator<Item = u128> + 's {
        let local = (dimension == self.dim.id() && self.player.is_some() && !self.lan_acting_fake)
            .then(|| horizon_client::lan::playerdata::ints_uuid(self.acting_uuid()));
        local
            .into_iter()
            .chain(self.guests_placed_in(dimension).map(|g| g.uuid))
    }
    /// The guests in the level in place, past the acting one: `player_view`
    /// already reads that guest's live body, and its record lags behind it.
    fn level_guests(&self) -> impl Iterator<Item = &horizon_client::lan::guest::Guest> + '_ {
        self.lan
            .iter()
            .flat_map(|h| self.guests_placed_in(&h.hub.config.dimension))
    }
    /// The guests placed in level `dimension`, past the acting one.
    fn guests_placed_in<'s>(
        &'s self,
        dimension: &'s str,
    ) -> impl Iterator<Item = &'s horizon_client::lan::guest::Guest> + 's {
        self.lan.iter().flat_map(move |h| {
            h.hub
                .guests
                .iter()
                .filter(move |g| g.placed_in(dimension) && self.lan_acting != Some(g.conn))
        })
    }
    /// `Level.hasNearbyAlivePlayer` before a level is built for the spawn
    /// code: what a spawner asks first, every tick.
    pub(super) fn alive_player_near(&self, x: f64, y: f64, z: f64, range: f64) -> bool {
        if self
            .local_level_player()
            .is_some_and(|p| p.alive_near(x, y, z, range))
        {
            return true;
        }
        self.level_guests().any(|g| {
            let [px, py, pz] = g.position;
            !g.dead
                && !g.spectator()
                && (range < 0.0
                    || (px - x).powi(2) + (py - y).powi(2) + (pz - z).powi(2) < range * range)
        })
    }
    /// `ChunkMap.collectSpawningChunks`: the natural spawn chunk tracker's
    /// chunks (within 8 chunks of a player, `getSpawnCandidateChunks`) that
    /// are ticking and within 128 blocks of a non-spectator, or held by a mod
    /// ticket that forces natural spawning (NeoForge's
    /// `shouldForceNaturalSpawning`). Also returns how many chunks the tracker
    /// holds (`getNaturalSpawnChunkCount`).
    pub(super) fn spawning_chunks(
        &self,
        players: &[LevelPlayer],
        scratch: &mut SpawnScratch,
    ) -> i32 {
        let tickets = self.streaming.mod_tickets();
        let levels = self.tickets.levels();
        scratch.collect(
            players,
            |chunk| tickets.forces_natural_spawning(chunk),
            |(cx, cz)| levels.loaded_ticking(cx, cz) && super::ticking_chunk(&self.world, cx, cz),
        )
    }
    /// `NaturalSpawner.createState` for a pass over the level in place, under
    /// its spawn limits (`SpawnPolicy`).
    pub(super) fn natural_spawn_state(
        &self,
        level: &GameLevel<'_, '_>,
        spawnable: i32,
        players: Vec<SpawnPlayer>,
        counted: &[Counted],
    ) -> SpawnState {
        SpawnState::new(level, spawnable, players, counted, |cx, cz| {
            self.world.chunks.contains_key(&(cx, cz))
        })
        .with_limits(self.spawn_policy(self.dim.key()).limits)
    }
    /// The categories a pass over the level in place tries: none with the
    /// `spawn_mobs` rule off, else `getFilteredSpawningCategories` with the
    /// `spawn_monsters` rule, the persistent ones every 400 ticks, and the
    /// level's spawn flags (`SpawnPolicy`) taking away.
    pub(super) fn natural_categories(&self, state: &SpawnState) -> Vec<Category> {
        let rules = &self.ui.rules;
        if !rules.bool_at(horizon_client::gamerules::vanilla::SPAWN_MOBS) {
            return Vec::new();
        }
        state.policy_categories(
            self.spawn_policy(self.dim.key()).flags,
            rules.bool_at(horizon_client::gamerules::vanilla::SPAWN_MONSTERS),
            self.world.game_time % 400 == 0,
        )
    }
    /// The tick's spawning phases in `ServerLevel.tick`'s order: the raids
    /// tick first (`this.raids.tick`), then the chunk source's
    /// (`getChunkSource().tick`): natural spawning, then the custom spawners
    /// of `ServerChunkCache.tickChunks`. A wave that lands this tick therefore
    /// joins before that tick's spawns, with the lower entity ids. Nothing spawns once the
    /// entity ids are exhausted (`entity_ids_exhausted`).
    pub(super) fn tick_spawners(&mut self) {
        if self.entity_ids_exhausted() {
            return;
        }
        let from = self.lan_synced_from();
        horizon_client::spans::time("spawn_raids", || self.tick_raids());
        horizon_client::spans::time("spawn_natural", || self.spawn_natural());
        // ServerChunkCache.tickChunks: the custom spawners tick only with spawn_mobs.
        if self
            .ui
            .rules
            .bool_at(horizon_client::gamerules::vanilla::SPAWN_MOBS)
        {
            horizon_client::spans::time("spawn_custom", || self.tick_custom_spawners());
        }
        self.lan_synced_since(from);
    }
    /// `ServerChunkCache.tickChunks`: the spawn state over the loaded mobs,
    /// the categories under their global caps (creatures every 400 ticks),
    /// then `tickSpawningChunk` in the shuffled spawning chunks of every
    /// player: inhabited time, then `spawnForChunk`.
    pub(super) fn spawn_natural(&mut self) {
        // `tickChunks` does nothing in a Debug world.
        if self.world_debug {
            return;
        }
        let mut laps = horizon_client::spans::laps();
        let mut players = self.level_players();
        // Fake players (mods' bots) do not anchor natural spawning.
        if let Some(h) = self.lan.as_ref() {
            let fake: Vec<u128> = h
                .hub
                .guests
                .iter()
                .filter(|g| g.is_fake())
                .map(|g| g.uuid)
                .collect();
            if !fake.is_empty() {
                players.retain(|p| !fake.contains(&p.uuid));
            }
        }
        if players.is_empty() {
            return;
        }
        let mut scratch = std::mem::take(&mut self.spawn_scratch);
        let spawnable = self.spawning_chunks(&players, &mut scratch);
        laps.lap("sn_candidates");
        for &key in &scratch.candidates {
            *self.world.inhabited.entry(key).or_insert(0) += 1;
        }
        laps.lap("sn_inhabited");
        let mut random = std::mem::replace(
            &mut self.world.random,
            horizon_core::random::LevelRandom::for_level(LegacyRandom::new(0)),
        );
        let mut pending = Vec::new();
        let last_state;
        let extras = {
            let level = self.natural_level();
            laps.lap("sn_level");
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
                    persistent: m.persistent
                        || horizon_client::mobs::ai::requires_custom_persistence(m),
                })
                .collect();
            let players = players
                .iter()
                .map(|p| SpawnPlayer {
                    position: [p.position.x, p.position.y, p.position.z],
                    spectator: p.spectator,
                })
                .collect();
            laps.lap("sn_materialize");
            let mut state = self.natural_spawn_state(&level, spawnable, players, &counted);
            laps.lap("sn_state");
            last_state = state.counts();
            let categories = self.natural_categories(&state);
            laps.lap("sn_categories");
            // Util.shuffle.
            for i in (1..scratch.candidates.len()).rev() {
                let j = random.next_int_bound(i as i32 + 1).unwrap_or(0) as usize;
                scratch.candidates.swap(i, j);
            }
            laps.lap("sn_shuffle");
            spawn_selection::attempt_counters(false);
            if !categories.is_empty() {
                for chunk in scratch.candidates.drain(..) {
                    // ServerChunkCache.tickSpawningChunk gates the initial chunk too.
                    if !level.can_spawn_in_chunk(chunk.0, chunk.1) {
                        continue;
                    }
                    let mut spawn = |l: &GameLevel, r: &SpawnRequest, rng: &mut LevelRandom| {
                        let spawned = l.finalize_spawned(r, &mut pending, rng);
                        if let Some(s) = &spawned {
                            l.pending.borrow_mut().push(s.bounds);
                        }
                        spawned
                    };
                    horizon_threads::work::set_chunk(chunk.0, chunk.1);
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
            let created = std::mem::take(&mut *level.spawned.borrow_mut());
            let rides = std::mem::take(&mut *level.rides.borrow_mut());
            (created, rides)
        };
        horizon_threads::work::clear_work();
        spawn_selection::attempt_counters(true);
        laps.lap("sn_attempts");
        self.world.random = random;
        // ServerChunkCache.lastSpawnState, for the debug overlay.
        self.spawn_counts.insert(self.dim, last_state);
        let (created, rides) = extras;
        self.add_spawned(pending, created, rides, false);
        self.spawn_scratch = scratch;
        laps.lap("sn_finalize");
    }
    /// `NoiseBasedChunkGenerator.spawnOriginalMobs`: creature packs for a
    /// freshly generated chunk, from the biome at its corner column's top,
    /// with the chunk's decoration-seeded random, finalized with the
    /// region's random as generation adds them. The settings' own
    /// `disable_mob_generation` skips the pass, which is how the End's preset
    /// keeps its chunks empty (the dimension is also checked, as before, so a
    /// preset that fails to load cannot start filling the End).
    pub(super) fn spawn_generation_creatures(&mut self, cx: i32, cz: i32) {
        if !self.world.chunks.contains_key(&(cx, cz)) || self.dim == Dim::End {
            return;
        }
        let Some(terrain) = self.end.locator() else {
            return;
        };
        // `FlatLevelSource.spawnOriginalMobs` is empty: a superflat world places no packs, and
        // neither does a mod's chunk generator, whose own `spawnOriginalMobs` the API has none of.
        if !terrain.noise_ground() {
            return;
        }
        // `NoiseGeneratorSettings.disableMobGeneration`: the shipped End preset
        // asks for no packs at generation, and so would any dimension using it.
        if terrain.disable_mob_generation() {
            return;
        }
        let mut region = terrain.region_random(cx, cz);
        let any = region.as_any();
        let added = if let Some(r) = any.downcast_mut::<LegacyRandom>() {
            self.generation_creatures(cx, cz, &terrain, r)
        } else if let Some(r) = any.downcast_mut::<XoroshiroRandom>() {
            self.generation_creatures(cx, cz, &terrain, r)
        } else {
            return;
        };
        for nbt in added {
            self.load_mob(Tag::Compound(nbt), None);
        }
    }
    fn generation_creatures<Q: RandomSource + 'static>(
        &self,
        cx: i32,
        cz: i32,
        terrain: &VanillaTerrain,
        region: &mut Q,
    ) -> Vec<Compound> {
        let mut random = WorldgenRandom::new(LegacyRandom::new(0));
        random.set_decoration_seed(self.level.seed, cx * 16, cz * 16);
        let mut added = Vec::new();
        let mut group = None;
        let level = self.natural_level();
        let source = [cx * 16, level.max_y(), cz * 16];
        let mut spawn = |l: &GameLevel, r: &SpawnRequest, region: &mut Q| {
            // Each pack starts without group data.
            if r.group_index == 0 {
                group = None;
            }
            let [x, y, z] = r.position;
            let (bx, by, bz) = (x.floor() as i32, y.floor() as i32, z.floor() as i32);
            let biome = l.zoomed_biome(bx, by, bz).map_or("", |b| b.name.as_str());
            let at = Vec3::new(x, y, z);
            match terrain.spawn_generated(r.entity, at, r.yaw, biome, region, &mut group) {
                Some(nbt) => {
                    added.push(nbt);
                    true
                }
                None => false,
            }
        };
        spawn_selection::spawn_mobs_for_chunk_generation(
            &level,
            source,
            (cx, cz),
            &mut random,
            region,
            &mut spawn,
            &mut |_| {},
        );
        added
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `Level.hasNearbyAlivePlayer` as spawners ask it, against the same
    /// test on every player's full record (`level_players`).
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn alive_player_near_tests_guests_where_they_stand() {
        use horizon_client::lan::{Joined, NetConfig, PlayerSender, guest::Guest, hub::HubConfig};
        use horizon_protocol::packets::configuration::ClientInformation;
        use std::sync::{Arc, atomic::AtomicI32, mpsc};
        let mut resources = None;
        let mut game = crate::game::game_tests::tests::server_game(&mut resources);
        let net = NetConfig {
            announce: false,
            ..NetConfig::lan(0, "spawner-players-test".into(), 8)
        };
        game.lan_host(
            super::super::lan::LanSettings {
                port: 0,
                guest_commands: true,
                force_game_mode: false,
            },
            net,
            HubConfig {
                default_game_mode: 0,
                guest_commands: true,
                force_game_mode: false,
                hardcore: false,
                view_distance: 2,
                simulation_distance: 2,
                dimension: Dim::Overworld.id().into(),
                allow_flight: false,
                seed: 0,
                max_players: 8,
                integrated: false,
                levels: Default::default(),
            },
        )
        .unwrap();
        // (x, dead, spectator): alive at 20, dead at 5, a spectator at 2.
        for (conn, (x, dead, spectator)) in
            [(20.5, false, false), (5.5, true, false), (2.5, false, true)]
                .into_iter()
                .enumerate()
        {
            let conn = conn as u32 + 2;
            let (tx, _rx) = mpsc::channel();
            let mut guest = Guest::new(
                Joined {
                    conn,
                    uuid: u128::from(conn),
                    name: format!("Near{conn}"),
                    properties: Vec::new(),
                    info: ClientInformation::new(2),
                    brand: "test".into(),
                    sender: PlayerSender::new(tx, Arc::default()),
                    latency: Arc::new(AtomicI32::new(0)),
                    address: "127.0.0.1:25565".parse().unwrap(),
                    expected: false,
                },
                conn as i32,
                [x, 64.0, 0.5],
                1,
            );
            guest.placed = true;
            guest.loaded = true;
            guest.dead = dead;
            if spectator {
                guest.game_mode = 3;
            }
            game.lan.as_mut().unwrap().hub.guests.push(guest);
        }
        let full = |game: &Game, x: f64, range: f64| {
            game.level_players()
                .iter()
                .any(|p| p.alive_near(x, 64.0, 0.5, range))
        };
        for (x, range, near) in [
            (0.5, 16.0, false),   // only the dead and the spectator in range
            (5.5, 16.0, true),    // the live one 15 away
            (4.4, 16.0, false),   // 16.1 away
            (4.5, 16.0, false),   // 16 away: not below the range
            (-500.0, -1.0, true), // any distance
            (36.4, 16.0, true),
        ] {
            assert_eq!(
                game.alive_player_near(x, 64.0, 0.5, range),
                near,
                "{x} {range}"
            );
            assert_eq!(full(&game, x, range), near, "{x} {range}");
        }
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn authoritative_world_border_gates_positions_and_whole_chunks() {
        use horizon_core::position::BlockPos;
        use horizon_world::level::LevelAccess;
        crate::game::native_entity_refs::tests::with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(horizon_client::chunk::Chunk::empty(0, 0)),
            );
            game.border.set_center([8.0, 8.0]);
            game.border.set_size(16.0);
            assert!(game.natural_level().can_spawn_in_chunk(0, 0));
            assert!(!game.natural_level().can_spawn_in_chunk(1, 0)); // unloaded
            game.border.set_size(15.5);
            assert!(!game.natural_level().can_spawn_in_chunk(0, 0)); // partial
            assert!(game.natural_level().within_world_border(1, 1));
            assert!(!game.natural_level().within_world_border(0, 0));
            game.with_level(|_, level, _| {
                assert!(level.within_world_border(BlockPos::new(1, 64, 1)));
                assert!(!level.within_world_border(BlockPos::new(0, 64, 0)));
            });
            game.border.set_size(16.5);
            assert!(game.natural_level().can_spawn_in_chunk(0, 0));
            assert!(game.mob_world_base(&[]).within_world_border(0, 0));
            assert!(!game.mob_world_base(&[]).within_world_border(17, 0));
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn world_border_outer_chunk_gate_keeps_attempt_randomness() {
        use horizon_client::{chunk::Chunk, motion::PlayerMotion};
        use std::sync::Arc;
        crate::game::native_entity_refs::tests::with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(horizon_client::chunk::Chunk::empty(0, 0)),
            );
            // Only the center chunk can tick: its eight neighbors are loaded,
            // but none of those has all its own neighbors. The center is partial.
            for x in -1..=1 {
                for z in -1..=1 {
                    let mut chunk = Chunk::empty(x, z);
                    chunk.lit = x == 0 && z == 0;
                    game.world.chunks.insert((x, z), Arc::new(chunk));
                }
            }
            game.tickets.update(None);
            game.player = Some(PlayerMotion::new(game.motion, Vec3::new(8.5, 64.0, 8.5)).unwrap());
            game.border.set_center([8.0, 8.0]);
            game.border.set_size(2.0);
            let mut scratch = SpawnScratch::default();
            game.spawning_chunks(&game.level_players(), &mut scratch);
            assert_eq!(scratch.candidates, vec![(0, 0)]);
            assert!(
                game.ui
                    .rules
                    .bool_at(horizon_client::gamerules::vanilla::SPAWN_MOBS)
            );
            // A contained chunk must actually reach the spawning pass, so an
            // empty category list cannot satisfy the rejected-chunk assertion.
            game.border.set_size(64.0);
            game.world.random.set_seed(12345);
            game.spawn_natural();
            assert_ne!(
                game.world.random.next_long(),
                LegacyRandom::new(12345).next_long()
            );
            game.world.inhabited.clear();
            game.border.set_size(2.0);
            game.world.random.set_seed(12345);
            game.spawn_natural();
            // No shuffle draw for one candidate, and no attempt draws for a
            // rejected chunk. Inhabited time still advances before the gate.
            assert_eq!(game.world.inhabited.get(&(0, 0)), Some(&1));
            assert_eq!(
                game.world.random.next_long(),
                LegacyRandom::new(12345).next_long()
            );
        });
    }
    fn player(x: f64, y: f64, z: f64, spectator: bool) -> LevelPlayer {
        LevelPlayer {
            uuid: 0,
            position: Vec3::new(x, y, z),
            bounds: Aabb::new(Vec3::new(x, y, z), Vec3::new(x, y, z)),
            alive: true,
            creative: false,
            spectator,
        }
    }

    /// A chunk in a player's spawning area but past 128 blocks is a spawning
    /// chunk only while a ticket forces it; a chunk outside the area never is.
    #[test]
    fn a_forcing_ticket_makes_a_far_chunk_in_the_area_a_spawning_chunk() {
        let players = [player(8.0, 64.0, 8.0, false)];
        let mut scratch = SpawnScratch::default();
        let corner = (8, 8);
        let count = scratch.collect(&players, |_| false, |_| true);
        assert_eq!(count, 289);
        assert!(
            !scratch.candidates.contains(&corner),
            "no player within 128 blocks"
        );
        assert!(scratch.candidates.contains(&(1, 1)));
        scratch.collect(&players, |c| c == corner || c == (30, 30), |_| true);
        assert!(scratch.candidates.contains(&corner));
        assert!(
            !scratch.candidates.contains(&(-8, -8)),
            "only the held chunk"
        );
        assert!(!scratch.candidates.contains(&(30, 30)), "outside the area");
    }

    /// The candidates as the uncached port collected them every tick: each
    /// player's square in dx/dz order into one ordered set, close to any
    /// player that holds the chunk. Returns the set's size too.
    fn uncached_candidates(players: &[LevelPlayer]) -> (i32, Vec<(i32, i32)>) {
        let mut order = Vec::new();
        let mut close = Vec::new();
        let mut seen = HashMap::new();
        for p in players {
            let (pcx, pcz) = SpawnScratch::chunk(p);
            for dx in -8..=8 {
                for dz in -8..=8 {
                    let key = (pcx + dx, pcz + dz);
                    let i = *seen.entry(key).or_insert_with(|| {
                        order.push(key);
                        close.push(false);
                        order.len() - 1
                    });
                    close[i] |= close_enough_for_spawning(p, key);
                }
            }
        }
        let kept = order.iter().zip(&close).filter(|c| *c.1).map(|c| *c.0);
        (order.len() as i32, kept.collect())
    }

    #[test]
    fn spawn_candidates_keep_order_and_live_proximity_across_cache_hits() {
        let mut players = vec![
            player(0.0, 64.0, 0.0, false),
            player(16.0, 64.0, 0.0, false),
        ];
        let mut scratch = SpawnScratch::default();
        // Two adjacent 17x17 squares: the first square, then the new east edge.
        assert_eq!(scratch.collect(&players, |_| false, |_| true), 306);
        assert!(scratch.candidates.contains(&(-8, 0)));
        // Spawning consumes the working list; the next pass must not care.
        scratch.candidates.reverse();
        scratch.candidates.truncate(3);
        // Moving inside its chunk takes player 0 out of range of the west edge.
        players[0].position.x = 15.5;
        assert_eq!(scratch.collect(&players, |_| false, |_| true), 306);
        assert!(!scratch.candidates.contains(&(-8, 0)));
        assert_eq!(scratch.candidates, uncached_candidates(&players).1);
        // Nothing is close to spectators, though the squares still count.
        for p in &mut players {
            p.spectator = true;
        }
        assert_eq!(scratch.collect(&players, |_| false, |_| true), 306);
        assert!(scratch.candidates.is_empty());
        for p in &mut players {
            p.spectator = false;
        }
        // Player 1 near its chunk's east side reaches the middle of chunk 9.
        players[1].position.x = 31.9;
        assert_eq!(scratch.collect(&players, |_| false, |_| true), 306);
        assert!(scratch.candidates.contains(&(9, 0)));
        assert_eq!(scratch.candidates, uncached_candidates(&players).1);
        // `keep` filters without reordering.
        scratch.collect(&players, |_| false, |(x, z)| x == z);
        let diagonal: Vec<_> = uncached_candidates(&players)
            .1
            .into_iter()
            .filter(|&(x, z)| x == z)
            .collect();
        assert!(diagonal.len() > 1);
        assert_eq!(scratch.candidates, diagonal);
    }

    /// Players walking, crossing borders, meeting, splitting, joining, leaving
    /// and swapping order: every pass matches the uncached collection.
    #[test]
    #[ignore = "daily"]
    fn spawn_candidates_match_the_uncached_union_while_players_move() {
        let mut seed = 0x5DEECE66Du64;
        let mut next = move |bound: u64| {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (seed >> 33) % bound
        };
        let mut players: Vec<LevelPlayer> = [0.0, 40.0, 300.0, 1200.0, -500.0]
            .iter()
            .map(|&x| player(x, 64.0, x / 2.0, false))
            .collect();
        let mut scratch = SpawnScratch::default();
        for step in 0..3000 {
            match next(40) {
                0 if players.len() < 8 => {
                    let x = next(1200) as f64 - 600.0;
                    let at = next(players.len() as u64 + 1) as usize;
                    players.insert(at, player(x, 64.0, -x, false));
                }
                1 if players.len() > 1 => {
                    players.remove(next(players.len() as u64) as usize);
                }
                2 if players.len() > 1 => {
                    let i = next(players.len() as u64 - 1) as usize;
                    players.swap(i, i + 1);
                }
                3 => {
                    let i = next(players.len() as u64) as usize;
                    players[i].spectator = !players[i].spectator;
                }
                4 => {
                    // Teleport onto another player: identical squares.
                    let (i, j) = (next(players.len() as u64), next(players.len() as u64));
                    players[i as usize].position = players[j as usize].position;
                }
                _ => {
                    for p in &mut players {
                        p.position.x += next(13) as f64 - 6.0 + 0.37;
                        p.position.z += next(13) as f64 - 6.0 - 0.21;
                    }
                }
            }
            let (count, expected) = uncached_candidates(&players);
            assert_eq!(
                scratch.collect(&players, |_| false, |_| true),
                count,
                "step {step}"
            );
            assert_eq!(scratch.candidates, expected, "step {step}");
        }
    }

    /// The grid finds the same player a look at every one finds, the first of
    /// two at the same distance included, and never a spectator.
    #[test]
    fn nearest_players_find_what_a_scan_finds() {
        let mut seed = 0x9E37_79B9_7F4A_7C15u64;
        let mut next = |m: f64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % 1_000_000) as f64 / 1_000_000.0 * 2.0 * m - m
        };
        let mut players: Vec<LevelPlayer> = (0..150)
            .map(|i| player(next(3_000.0), 64.0 + next(60.0), next(3_000.0), i % 11 == 0))
            .collect();
        // Two at the same distance from the origin, the second one first in no cell order.
        players.push(player(300.0, 64.0, 0.0, false));
        players.push(player(-300.0, 64.0, 0.0, false));
        players.push(player(0.0, 64.0, 300.0, false));
        let grid = NearestPlayers::new(&players);
        let index = |p: Option<&LevelPlayer>| p.map(|p| std::ptr::from_ref(p) as usize);
        let mut probes: Vec<(f64, f64, f64)> = (0..400)
            .map(|_| (next(3_000.0), next(100.0), next(3_000.0)))
            .collect();
        probes.extend([(0.0, 64.0, 0.0), (1.0e12, 0.0, 0.0), (f64::NAN, 0.0, 0.0)]);
        for (x, y, z) in probes {
            assert_eq!(
                index(grid.nearest(x, y, z)),
                index(nearest_player(&players, x, y, z)),
                "({x}, {y}, {z})"
            );
        }
        players.push(player(NearestPlayers::FAR * 2.0, 64.0, 0.0, false));
        let grid = NearestPlayers::new(&players);
        assert!(grid.scan);
        for x in [0.0, NearestPlayers::FAR * 2.0] {
            assert_eq!(
                index(grid.nearest(x, 64.0, 0.0)),
                index(nearest_player(&players, x, 64.0, 0.0))
            );
        }
        let spectators: Vec<LevelPlayer> = (0..20)
            .map(|i| player(f64::from(i) * 100.0, 0.0, 0.0, true))
            .collect();
        assert!(
            NearestPlayers::new(&spectators)
                .nearest(0.0, 0.0, 0.0)
                .is_none()
        );
    }

    /// Answers both spawn checks with one fixed result.
    struct ChecksMod(SpawnCheck);
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for ChecksMod {
        fn mob_spawn_placement_check(
            &self,
            _: &dyn mod_api::Host,
            event: &mut mod_api::MobSpawnPlacementCheckEvent<'_>,
        ) {
            *event.result = self.0;
        }
        fn mob_spawn_position_check(
            &self,
            _: &dyn mod_api::Host,
            event: &mut mod_api::MobSpawnPositionCheckEvent<'_>,
        ) {
            *event.result = self.0;
        }
    }

    /// The placement check runs after vanilla's predicate (`default_result` is its answer), so
    /// its draws are vanilla's whatever a mod answers. The position check replaces vanilla's two
    /// checks: no listener and `Default` draw exactly as vanilla, `Succeed` and `Fail` draw nothing.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES and an isolated process for native mods"]
    fn spawn_checks_pin_the_random_draws_of_both_halves() {
        use super::super::native_entity_refs::tests::with_game;
        with_game(|game| {
            let level = game.natural_level();
            let placement = |random: &mut LegacyRandom| {
                let ok = horizon_client::mobs::spawn_rules::check_spawn_rules(
                    "minecraft:sulfur_cube",
                    Species::SulfurCube,
                    &level,
                    Reason::Natural,
                    random,
                    [8, 64, 8],
                );
                (ok, random.next_int())
            };
            let position = |random: &mut LegacyRandom| {
                let ok = spawn_selection::position_ok(
                    &level,
                    "minecraft:sulfur_cube",
                    Reason::Natural,
                    [8.5, 64.0, 8.5],
                    spawn_selection::Pending {
                        yaw: 0.0,
                        fresh: true,
                    },
                    || {
                        random.next_int();
                        false
                    },
                );
                (ok, random.next_int())
            };
            let run = |answer: Option<SpawnCheck>| {
                let go = || {
                    (
                        placement(&mut LegacyRandom::new(11)),
                        position(&mut LegacyRandom::new(11)),
                    )
                };
                match answer {
                    None => go(),
                    Some(a) => mod_platform::testing::with_mods(
                        vec![mod_api::ModEntry::new(
                            "spawn-checks",
                            Box::new(ChecksMod(a)),
                        )]
                        .into(),
                        go,
                    ),
                }
            };
            let mut seed = LegacyRandom::new(11);
            let first = seed.next_int();
            let second = seed.next_int();
            let none = run(None);
            assert!(none.0.0, "the sulfur cube's predicate has no conditions");
            assert_eq!(none.1, (false, second), "vanilla's closure drew once");
            assert_eq!(run(Some(SpawnCheck::Default)), none);
            let succeed = run(Some(SpawnCheck::Succeed));
            assert_eq!(
                succeed.0,
                (true, none.0.1),
                "placement draws stay vanilla's"
            );
            assert_eq!(
                succeed.1,
                (true, first),
                "Succeed skips the checks and their draws"
            );
            let fail = run(Some(SpawnCheck::Fail));
            assert_eq!(fail.0, (false, none.0.1));
            assert_eq!(fail.1, (false, first));
        });
    }

    /// What a spawn-check listener heard: the hook, the kind, the level and the spawner, and on
    /// the position check what the lent mob read (kind, position, whether its head slot is
    /// empty).
    #[derive(Clone, Debug, PartialEq)]
    struct Heard {
        hook: &'static str,
        kind: String,
        level: mod_api::LevelRef,
        spawner: Option<mod_api::Spawner>,
        mob: Option<MobReads>,
    }
    type MobReads = (
        Result<mod_api::Identifier, mod_api::UnspawnedMobError>,
        Result<[f64; 3], mod_api::UnspawnedMobError>,
        Result<bool, mod_api::UnspawnedMobError>,
    );
    type Log = std::sync::Arc<std::sync::Mutex<Vec<Heard>>>;
    /// Records both spawn checks and answers the position check with `answer`, reading the lent
    /// mob and changing nothing else.
    struct ContextMod {
        answer: SpawnCheck,
        log: Log,
    }
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for ContextMod {
        fn mob_spawn_placement_check(
            &self,
            _: &dyn mod_api::Host,
            event: &mut mod_api::MobSpawnPlacementCheckEvent<'_>,
        ) {
            self.log.lock().unwrap().push(Heard {
                hook: "placement",
                kind: event.kind.to_owned(),
                level: event.level,
                spawner: event.spawner,
                mob: None,
            });
        }
        fn mob_spawn_position_check(
            &self,
            host: &dyn mod_api::Host,
            event: &mut mod_api::MobSpawnPositionCheckEvent<'_>,
        ) {
            let mob = event.mob;
            let reads = (
                mob.kind(host),
                mob.position(host),
                mob.equipment(host, mod_api::EquipmentSlot::Head)
                    .map(|e| e.is_none()),
            );
            self.log.lock().unwrap().push(Heard {
                hook: "position",
                kind: event.kind.to_owned(),
                level: event.level,
                spawner: event.spawner,
                mob: Some(reads),
            });
            *event.result = self.answer;
        }
    }
    /// Gives every natural monster position one entry: the sulfur cube, whose spawn predicate
    /// has no conditions, in packs of four.
    struct SulfurCubesOnly;
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for SulfurCubesOnly {
        fn potential_spawns(
            &self,
            _: &dyn mod_api::Host,
            event: &mut mod_api::PotentialSpawnsEvent<'_>,
        ) -> mod_api::Flow {
            if event.category == "monster" {
                *event.entries = vec![SpawnEntry {
                    entity: SULFUR_CUBE.into(),
                    weight: 1,
                    min: 4,
                    max: 4,
                    constant: true,
                }];
            }
            mod_api::Flow::Continue
        }
    }
    const SULFUR_CUBE: &str = "minecraft:sulfur_cube";
    /// A spawned mob as the draw comparisons see it: id, kind, position, yaw, persistence and
    /// the next draw of its own random (seeded from its id).
    type Snap = (u32, String, [f64; 3], f32, bool, i32);
    fn snap(m: &Mob) -> Snap {
        (
            m.id,
            m.kind.id.clone(),
            [m.position.x, m.position.y, m.position.z],
            m.yaw,
            m.persistent,
            m.random.clone().next_int(),
        )
    }
    /// Runs `go` with no mod (`None`), or with a position-check listener answering `answer`
    /// beside `extra` mods, and what that listener heard.
    fn with_checks<R>(
        answer: Option<SpawnCheck>,
        extra: impl Fn() -> Vec<mod_api::ModEntry>,
        go: impl FnOnce() -> R,
    ) -> (R, Vec<Heard>) {
        let log = Log::default();
        let mut mods = extra();
        if let Some(answer) = answer {
            mods.push(mod_api::ModEntry::new(
                "spawn-context",
                Box::new(ContextMod {
                    answer,
                    log: log.clone(),
                }),
            ));
        }
        let out = if mods.is_empty() {
            go()
        } else {
            mod_platform::testing::with_mods(mods.into(), go)
        };
        let heard = log.lock().unwrap().clone();
        (out, heard)
    }
    /// A survival player standing at `at`, and empty lit chunks around the origin.
    fn spawn_check_world(game: &mut Game<'_>, at: Vec3) {
        use horizon_client::chunk::Chunk;
        use std::sync::Arc;
        game.player = Some(horizon_client::motion::PlayerMotion::new(game.motion, at).unwrap());
        for cx in -2..=4 {
            for cz in -2..=4 {
                let mut chunk = Chunk::empty(cx, cz);
                chunk.lit = true;
                game.world.chunks.insert((cx, cz), Arc::new(chunk));
            }
        }
    }
    fn level_of(game: &Game<'_>) -> mod_api::LevelRef {
        game.level(game.dim.id()).unwrap()
    }

    /// Natural spawning (`spawnCategoryForPosition`): a listener hears both checks with the
    /// level and no spawner, and reads the pending mob; answering `Default` it leaves every draw,
    /// id and spawned mob as with no listener, so the mob it read is the one that spawned, not
    /// created twice. A `Fail` drops each mob: none is added and no provisional id is used.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES and an isolated process for native mods"]
    fn natural_spawn_checks_lend_the_mob_and_keep_the_draws() {
        use super::super::native_entity_refs::tests::with_game;
        with_game(|game| {
            spawn_check_world(game, Vec3::new(0.5, 64.0, 0.5));
            let stone = game.pack.assets.registry.default_state("minecraft:stone");
            for x in 32..48 {
                for z in 32..48 {
                    game.world.set(x, 63, z, stone.unwrap()).unwrap();
                }
            }
            let level = level_of(game);
            let attempt = |game: &mut Game<'_>| {
                let mut random = LegacyRandom::new(42);
                let natural = game.natural_level();
                let players = natural
                    .players
                    .iter()
                    .map(|p| SpawnPlayer {
                        position: [p.position.x, p.position.y, p.position.z],
                        spectator: p.spectator,
                    })
                    .collect();
                let mut state = SpawnState::new(&natural, 289, players, &[], |_, _| true);
                let mut pending = Vec::new();
                let mut spawn = |l: &GameLevel, r: &SpawnRequest, rng: &mut LegacyRandom| {
                    let spawned = l.finalize_spawned(r, &mut pending, rng);
                    if let Some(s) = &spawned {
                        l.pending.borrow_mut().push(s.bounds);
                    }
                    spawned
                };
                spawn_selection::spawn_category_for_position(
                    &natural,
                    &mut random,
                    &mut state,
                    Category::Monster,
                    (2, 2),
                    [40, 64, 40],
                    &mut spawn,
                    &mut |_| {},
                );
                assert!(natural.checked.borrow().is_none(), "no created mob is left");
                let used = natural.provisional.get();
                drop(natural);
                assert!(game.lent_mobs.borrow().is_empty(), "every lend ended");
                (
                    pending.iter().map(snap).collect::<Vec<_>>(),
                    used,
                    random.next_int(),
                )
            };
            let spawns = || {
                vec![mod_api::ModEntry::new(
                    "sulfur-cubes",
                    Box::new(SulfurCubesOnly),
                )]
            };
            let (unlistened, heard) = with_checks(None, spawns, || attempt(game));
            assert!(heard.is_empty());
            assert!(!unlistened.0.is_empty(), "the attempt spawns");
            assert_eq!(
                unlistened.1 as usize,
                unlistened.0.len(),
                "one provisional id per spawned mob"
            );
            let (listened, heard) =
                with_checks(Some(SpawnCheck::Default), spawns, || attempt(game));
            assert_eq!(listened, unlistened, "same draws, ids and mobs");
            let positions: Vec<_> = heard.iter().filter(|h| h.hook == "position").collect();
            assert!(positions.len() >= unlistened.0.len());
            for h in &heard {
                assert_eq!(
                    (h.kind.as_str(), h.level, h.spawner),
                    (SULFUR_CUBE, level, None)
                );
            }
            assert!(heard.iter().any(|h| h.hook == "placement"));
            // Each spawned mob was read at its own position, a full mob (its head slot answers).
            for m in &unlistened.0 {
                let reads = (
                    Ok(mod_api::Identifier::parse(SULFUR_CUBE).unwrap()),
                    Ok(m.2),
                    Ok(true),
                );
                assert!(positions.iter().any(|h| h.mob.as_ref() == Some(&reads)));
            }
            let (refused, heard) = with_checks(Some(SpawnCheck::Fail), spawns, || attempt(game));
            assert!(refused.0.is_empty(), "a failed check adds nothing");
            assert_eq!(refused.1, 0, "and gives each provisional id back");
            assert!(heard.iter().any(|h| h.hook == "position"));
        });
    }

    /// A spawner block: both checks hear `Spawner::Block` at the block; a `Default` listener on
    /// the position check leaves the draws, the real ids and the mobs that join as with none
    /// (the lent mob is the one that joins); a `Fail` adds no mob, uses no entity id and leaves
    /// no tracking entry. Spawn data with entity data lends the kind and position only.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES and an isolated process for native mods"]
    fn spawner_checks_tell_the_block_and_keep_the_draws() {
        use super::super::native_entity_refs::tests::with_game;
        use horizon_client::chunk::block_entity::Next;
        use horizon_client::mobs::spawner::{SpawnData, SpawnDataExt};
        with_game(|game| {
            spawn_check_world(game, Vec3::new(0.5, 70.0, 0.5));
            let level = level_of(game);
            let key = (2, 70, 2);
            let block = mod_api::Spawner::Block {
                pos: mod_api::BlockPos::new(2, 70, 2),
            };
            let tick = |game: &mut Game<'_>, extra: Option<(&str, Tag)>| {
                let (mobs, entities, tracked) =
                    (game.mobs.len(), game.next_entity, game.persist.uuids.len());
                game.world.random.set_seed(7);
                let mut entity = Compound::default();
                entity.insert("id".into(), Tag::String(SULFUR_CUBE.into()));
                if let Some((name, value)) = extra {
                    entity.insert(name.into(), value);
                }
                let mut spawner = horizon_client::mobs::spawner::Spawner {
                    spawn_delay: 0,
                    next: Some(Next::Own(SpawnData::new(entity, None, None))),
                    ..Default::default()
                };
                game.server_tick_spawner_of(key, &mut spawner, None)
                    .unwrap();
                assert!(game.lent_mobs.borrow().is_empty(), "every lend ended");
                let added: Vec<Snap> = game.mobs[mobs..].iter().map(snap).collect();
                let out = (
                    added,
                    game.next_entity - entities,
                    game.persist.uuids.len() - tracked,
                    game.world.random.next_int(),
                );
                for m in game.mobs.drain(mobs..) {
                    game.persist.uuids.remove(&m.id);
                }
                game.next_entity = entities;
                out
            };
            let (unlistened, _) = with_checks(None, Vec::new, || tick(game, None));
            assert!(!unlistened.0.is_empty(), "the spawner spawns");
            let (listened, heard) =
                with_checks(Some(SpawnCheck::Default), Vec::new, || tick(game, None));
            assert_eq!(listened, unlistened, "same draws, ids and mobs");
            assert!(heard.iter().any(|h| h.hook == "placement"));
            for h in &heard {
                assert_eq!(
                    (h.kind.as_str(), h.level, h.spawner),
                    (SULFUR_CUBE, level, Some(block))
                );
                if let Some(reads) = &h.mob {
                    assert_eq!(reads.2, Ok(true), "a fresh spawn lends the whole mob");
                }
            }
            for m in &unlistened.0 {
                assert!(
                    heard
                        .iter()
                        .any(|h| h.mob.as_ref().is_some_and(|r| r.1 == Ok(m.2)))
                );
            }
            let (refused, heard) =
                with_checks(Some(SpawnCheck::Fail), Vec::new, || tick(game, None));
            assert!(heard.iter().any(|h| h.hook == "position"));
            assert_eq!(
                (refused.0.len(), refused.1, refused.2),
                (0, 0, 0),
                "no mob, no entity id, no tracking entry"
            );
            // Entity data: the mob is loaded after the tick, so the lend is its kind and position.
            let health = || Some(("Health", Tag::Float(5.0f32.to_bits())));
            let (_, heard) =
                with_checks(Some(SpawnCheck::Default), Vec::new, || tick(game, health()));
            let lent: Vec<_> = heard.iter().filter_map(|h| h.mob.clone()).collect();
            assert!(!lent.is_empty());
            for (kind, position, head) in lent {
                assert_eq!(kind, Ok(mod_api::Identifier::parse(SULFUR_CUBE).unwrap()));
                assert!(position.is_ok());
                assert_eq!(head, Err(mod_api::UnspawnedMobError::Unsupported));
            }
        });
    }

    /// A trial spawner: the position check (obstruction only) and the placement check hear
    /// `Spawner::Trial` at the block; a `Default` listener leaves the draws, ids and mob as with
    /// none; a `Fail` adds no mob and no tracking entry.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES and an isolated process for native mods"]
    fn trial_spawner_checks_tell_the_block_and_keep_the_draws() {
        use super::super::native_entity_refs::tests::with_game;
        use horizon_client::mobs::spawner::{SpawnData, SpawnDataExt};
        use horizon_client::mobs::trial_spawner::{TrialSpawner, TrialState};
        with_game(|game| {
            spawn_check_world(game, Vec3::new(0.5, 70.0, 0.5));
            let level = level_of(game);
            let key = (2, 70, 2);
            let trial = mod_api::Spawner::Trial {
                pos: mod_api::BlockPos::new(2, 70, 2),
            };
            let tick = |game: &mut Game<'_>| {
                let (mobs, entities, tracked) =
                    (game.mobs.len(), game.next_entity, game.persist.uuids.len());
                game.world.random.set_seed(9);
                let mut entity = Compound::default();
                entity.insert("id".into(), Tag::String(SULFUR_CUBE.into()));
                let mut spawner = TrialSpawner {
                    next_spawn_data: Some(SpawnData::new(entity, None, None)),
                    ..Default::default()
                };
                game.server_tick_trial_spawner(key, &mut spawner, TrialState::Active, false)
                    .unwrap();
                assert!(game.lent_mobs.borrow().is_empty(), "every lend ended");
                let added: Vec<Snap> = game.mobs[mobs..].iter().map(snap).collect();
                let out = (
                    added,
                    game.next_entity - entities,
                    game.persist.uuids.len() - tracked,
                    game.world.random.next_int(),
                );
                for m in game.mobs.drain(mobs..) {
                    game.persist.uuids.remove(&m.id);
                }
                game.next_entity = entities;
                out
            };
            let (unlistened, _) = with_checks(None, Vec::new, || tick(game));
            assert_eq!(unlistened.0.len(), 1, "the trial spawner spawns one mob");
            assert!(unlistened.0[0].4, "a trial spawner's mob is persistent");
            let (listened, heard) = with_checks(Some(SpawnCheck::Default), Vec::new, || tick(game));
            assert_eq!(listened, unlistened, "same draws, ids and mob");
            let lent = (
                Ok(mod_api::Identifier::parse(SULFUR_CUBE).unwrap()),
                Ok(unlistened.0[0].2),
                Ok(true),
            );
            assert!(heard.iter().any(|h| h.mob.as_ref() == Some(&lent)));
            assert!(heard.iter().any(|h| h.hook == "placement"));
            for h in &heard {
                assert_eq!(
                    (h.kind.as_str(), h.level, h.spawner),
                    (SULFUR_CUBE, level, Some(trial))
                );
            }
            let (refused, heard) = with_checks(Some(SpawnCheck::Fail), Vec::new, || tick(game));
            assert!(heard.iter().any(|h| h.hook == "position"));
            assert_eq!(
                (refused.0.len(), refused.1, refused.2),
                (0, 0, 0),
                "no mob, no entity id, no tracking entry"
            );
        });
    }

    /// Answers `finalize_spawn` the way a test asks.
    #[derive(Clone, Copy)]
    enum FinalizeAnswer {
        Pass,
        SkipFinalize,
        StopSpawn,
    }
    struct FinalizeMod(FinalizeAnswer);
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for FinalizeMod {
        fn finalize_spawn(
            &self,
            _: &dyn mod_api::Host,
            event: &mut mod_api::FinalizeSpawnEvent<'_>,
        ) -> mod_api::Flow {
            match self.0 {
                FinalizeAnswer::Pass => {}
                FinalizeAnswer::SkipFinalize => return mod_api::Flow::Cancel,
                FinalizeAnswer::StopSpawn => event.spawn_cancelled = true,
            }
            mod_api::Flow::Continue
        }
    }

    /// No listener and a listener that changes nothing finalize and draw as vanilla; a cancel
    /// skips finalization and its draws, the mob still spawning; `spawn_cancelled` adds no mob.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES and an isolated process for native mods"]
    fn finalize_spawn_cancels_pin_the_draws_and_the_outcome() {
        use super::super::native_entity_refs::tests::with_game;
        with_game(|game| {
            let kind = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .cloned()
                .unwrap();
            let run = |game: &mut Game<'_>, answer: Option<FinalizeAnswer>| {
                game.world.random.set_seed(5);
                let mut mob = Mob::new(1, kind.clone(), Vec3::new(8.5, 64.0, 8.5), 0.0);
                let go = |game: &mut Game<'_>, mob: &mut Mob| {
                    game.finalize_with(mob, Reason::Natural, None)
                };
                let outcome = match answer {
                    None => go(game, &mut mob),
                    Some(a) => mod_platform::testing::with_mods(
                        vec![mod_api::ModEntry::new("finalize", Box::new(FinalizeMod(a)))].into(),
                        || go(game, &mut mob),
                    ),
                };
                let tag = match outcome {
                    Finalized::Done => "done",
                    Finalized::Skipped => "skipped",
                    Finalized::Cancelled => "cancelled",
                };
                (
                    tag,
                    game.world.random.next_int(),
                    mob.random.clone().next_int(),
                )
            };
            let none = run(game, None);
            assert_eq!(none.0, "done");
            assert_eq!(run(game, Some(FinalizeAnswer::Pass)), none, "pass-through");
            let fresh = LegacyRandom::new(5).next_int();
            let skipped = run(game, Some(FinalizeAnswer::SkipFinalize));
            assert_eq!(skipped.0, "skipped");
            assert_eq!(skipped.1, fresh, "a skipped finalization draws nothing");
            let stopped = run(game, Some(FinalizeAnswer::StopSpawn));
            assert_eq!(stopped.0, "cancelled");
            assert_eq!(stopped.1, fresh, "a stopped spawn draws nothing");
        });
    }

    /// Counts the `finalize_spawn` events it hears.
    struct CountFinalize(std::sync::Arc<std::sync::atomic::AtomicUsize>);
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for CountFinalize {
        fn finalize_spawn(
            &self,
            _: &dyn mod_api::Host,
            _: &mut mod_api::FinalizeSpawnEvent<'_>,
        ) -> mod_api::Flow {
            self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            mod_api::Flow::Continue
        }
    }

    /// A site vanilla makes without `finalizeSpawn` (an infested block's silverfish) is not
    /// told; one it finalizes is.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES and an isolated process for native mods"]
    fn finalize_spawn_is_told_only_where_vanilla_finalizes() {
        use super::super::creatures::SpawnSite;
        use super::super::native_entity_refs::tests::with_game;
        let heard = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mods = vec![mod_api::ModEntry::new(
            "count-finalize",
            Box::new(CountFinalize(heard.clone())),
        )];
        mod_platform::testing::with_mods(mods.into(), || {
            with_game(|game| {
                let at = [8.5, 64.0, 8.5];
                let count = || heard.load(std::sync::atomic::Ordering::Relaxed);
                assert!(
                    game.spawn_mob("minecraft:silverfish", at, false, SpawnSite::Plain)
                        .is_some()
                );
                assert_eq!(count(), 0, "a plain site is not told");
                assert!(
                    game.spawn_mob(
                        "minecraft:silverfish",
                        at,
                        false,
                        SpawnSite::Finalizing(Reason::Spawner)
                    )
                    .is_some()
                );
                assert_eq!(count(), 1, "a finalizing site is told once");
            });
        });
    }

    /// Records the kind and reason of every `finalize_spawn` event.
    struct RecordFinalize(std::sync::Arc<std::sync::Mutex<Vec<(String, mod_api::SpawnReason)>>>);
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for RecordFinalize {
        fn finalize_spawn(
            &self,
            _: &dyn mod_api::Host,
            event: &mut mod_api::FinalizeSpawnEvent<'_>,
        ) -> mod_api::Flow {
            self.0
                .lock()
                .unwrap()
                .push((event.kind.to_owned(), event.reason));
            mod_api::Flow::Continue
        }
    }

    /// `ZombieHorse.finalizeSpawn` finalizes its rider with the horse's own
    /// `spawnReason`, a baby zombie's chicken with `JOCKEY`.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES and an isolated process for native mods"]
    fn jockeys_finalize_with_the_parents_reason() {
        use super::super::creatures::SpawnSite;
        use super::super::native_entity_refs::tests::with_game;
        let heard = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let mods = vec![mod_api::ModEntry::new(
            "record-finalize",
            Box::new(RecordFinalize(heard.clone())),
        )];
        mod_platform::testing::with_mods(mods.into(), || {
            with_game(|game| {
                game.ui.difficulty = 2;
                let at = [8.5, 64.0, 8.5];
                game.spawn_mob(
                    "minecraft:zombie_horse",
                    at,
                    false,
                    SpawnSite::Finalizing(Reason::Natural),
                )
                .unwrap();
                let heard = heard.lock().unwrap();
                let natural = Reason::Natural;
                assert_eq!(
                    *heard,
                    [
                        ("minecraft:zombie_horse".to_owned(), natural),
                        ("minecraft:zombie".to_owned(), natural)
                    ]
                );
            });
        });
    }

    /// Answers every `mob_despawn` event with the value in its cell and counts them.
    struct Despawns(
        std::sync::Arc<std::sync::Mutex<mod_api::Despawn>>,
        std::sync::Arc<std::sync::atomic::AtomicUsize>,
    );
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Despawns {
        fn mob_despawn(&self, _: &dyn mod_api::Host, event: &mut mod_api::MobDespawnEvent<'_>) {
            self.1.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            *event.result = *self.0.lock().unwrap();
        }
    }

    /// `MobDespawnEvent` wraps the whole of `Mob.checkDespawn`: `Default` and no
    /// listener leave vanilla's checks and their `nextInt(800)` draw as they
    /// are, `Deny` keeps a peaceful-mode zombie and draws nothing, `Allow`
    /// discards it in normal and draws nothing. The mob's own random is the
    /// real draw source (a `LegacyRandom`), compared with an untouched copy.
    /// A mob in an unloaded chunk is never asked.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES and an isolated process for native mods"]
    fn mob_despawn_answers_override_vanilla_and_default_passes_through() {
        use super::super::native_entity_refs::tests::with_game;
        use mod_api::Despawn::{Allow, Default as Vanilla, Deny};
        use std::sync::atomic::Ordering::Relaxed;
        let answer = std::sync::Arc::new(std::sync::Mutex::new(Vanilla));
        let asked = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mods = vec![mod_api::ModEntry::new(
            "despawns",
            Box::new(Despawns(answer.clone(), asked.clone())),
        )];
        mod_platform::testing::with_mods(mods.into(), || {
            with_game(|game| {
                let id = game
                    .spawn_mob(
                        "minecraft:zombie",
                        [8.5, 64.0, 8.5],
                        false,
                        super::super::creatures::SpawnSite::Plain,
                    )
                    .unwrap();
                // Asks the hook, then runs `despawn_check` for an idle zombie 40
                // blocks from the player. Returns whether it stayed and whether
                // its random is where an untouched copy of it is.
                let ask = |game: &mut super::Game<'_>, said, peaceful: bool| {
                    *answer.lock().unwrap() = said;
                    let mut mob = game.mobs.iter().find(|m| m.id == id).unwrap().clone();
                    mob.ai.no_action_time = 601;
                    let mut untouched = mob.random.clone();
                    let handle = mod_api::MobRef::new(mod_api::EntityRef::new(
                        game.native_lifecycle.epoch().unwrap(),
                        id,
                    ));
                    let pos = [mob.position.x, mob.position.y, mob.position.z];
                    game.native_mob_despawn(
                        "minecraft:overworld",
                        handle,
                        &mob.kind.id,
                        pos,
                        &mut mob.despawn,
                    );
                    let mut world = game.mob_world_base(&[]);
                    world.difficulty.effective = if peaceful { 0.0 } else { 2.0 };
                    let mut player = mob.position;
                    player.x += 40.0;
                    world.nearest_player = Some(player);
                    let kept = horizon_client::mobs::ai::despawn_check(&mut mob, &world);
                    let untouched_random =
                        mob.random.next_int_bound(1 << 30) == untouched.next_int_bound(1 << 30);
                    (kept, untouched_random)
                };
                assert!(!ask(game, Vanilla, true).0, "Default: peaceful discards it");
                assert_eq!(
                    ask(game, Deny, true),
                    (true, true),
                    "Deny keeps it, no draw"
                );
                assert_eq!(
                    ask(game, Allow, false),
                    (false, true),
                    "Allow discards it, no draw"
                );
                assert!(
                    !ask(game, Vanilla, false).1,
                    "Default rolls vanilla's nextInt(800)"
                );
                assert_eq!(asked.load(Relaxed), 4);
                // No chunk is loaded here, so the mob loop skips the mob before
                // the hook: an unloaded mob is never asked.
                *answer.lock().unwrap() = Deny;
                game.tick_creatures().unwrap();
                assert_eq!(asked.load(Relaxed), 4, "an unloaded mob is not asked");
            });
        });
    }

    /// With no mod listening, `Mob.despawn` stays `Default` and vanilla's checks
    /// and draws run as before.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn mob_despawn_with_no_listener_draws_as_vanilla() {
        use super::super::native_entity_refs::tests::with_game;
        with_game(|game| {
            let id = game
                .spawn_mob(
                    "minecraft:zombie",
                    [8.5, 64.0, 8.5],
                    false,
                    super::super::creatures::SpawnSite::Plain,
                )
                .unwrap();
            let mut mob = game.mobs.iter().find(|m| m.id == id).unwrap().clone();
            mob.ai.no_action_time = 601;
            let mut untouched = mob.random.clone();
            let mut world = game.mob_world_base(&[]);
            world.difficulty.effective = 2.0;
            let mut player = mob.position;
            player.x += 40.0;
            world.nearest_player = Some(player);
            horizon_client::mobs::ai::despawn_check(&mut mob, &world);
            assert_eq!(mob.despawn, mod_api::Despawn::Default);
            assert_ne!(
                mob.random.next_int_bound(1 << 30),
                untouched.next_int_bound(1 << 30),
                "vanilla's nextInt(800) is drawn"
            );
        });
    }

    /// Answers the group size limit of a type, reading the mob it is lent.
    struct ZombieClusters;
    #[mod_api::horizon_part]
    impl mod_api::EntityPart for ZombieClusters {
        fn spawn_cluster_size(
            &self,
            host: &dyn mod_api::Host,
            mob: mod_api::UnspawnedMobRef,
            reason: mod_api::SpawnReason,
            _: Option<mod_api::SpawnSource>,
            vanilla: u32,
        ) -> Option<u32> {
            assert_eq!(mob.kind(host).unwrap().to_string(), "minecraft:zombie");
            Some(if reason == mod_api::SpawnReason::Natural {
                vanilla * 2
            } else {
                1
            })
        }
    }
    /// A second part on the same type: asked only when the first does not answer.
    struct LaterClusters;
    #[mod_api::horizon_part]
    impl mod_api::EntityPart for LaterClusters {
        fn spawn_cluster_size(
            &self,
            _: &dyn mod_api::Host,
            _: mod_api::UnspawnedMobRef,
            _: mod_api::SpawnReason,
            _: Option<mod_api::SpawnSource>,
            _: u32,
        ) -> Option<u32> {
            Some(9)
        }
    }
    /// A part that leaves the limit to vanilla.
    struct SilentClusters;
    #[mod_api::horizon_part]
    impl mod_api::EntityPart for SilentClusters {}
    struct ClusterMod;
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for ClusterMod {
        fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
            r.entity_part("minecraft:zombie", ZombieClusters);
            r.entity_part("minecraft:zombie", LaterClusters);
            r.entity_part("minecraft:skeleton", SilentClusters);
        }
    }

    /// The parts of the new mob's type answer the group size limit: a part's answer replaces
    /// vanilla's for its type and reason (the first part to answer wins), a part that does not
    /// answer, and a type with no part, leave it to vanilla.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES and an isolated process for native mods"]
    fn entity_parts_answer_the_spawn_cluster_size() {
        use super::super::native_entity_refs::tests::{run_fresh_cases, with_game};
        if run_fresh_cases(
            "game::natural_spawning::tests::entity_parts_answer_the_spawn_cluster_size",
            "HORIZON_SPAWN_CLUSTER_CHILD",
            &["valid"],
        ) {
            return;
        }
        let entries = vec![mod_api::ModEntry::new("clusters", Box::new(ClusterMod))];
        mod_platform::__private::install(entries.into());
        crate::game::api_values::install_parts(&[mod_api::ModEntry::new(
            "clusters",
            Box::new(ClusterMod),
        )])
        .unwrap();
        with_game(|game| {
            game.resolve_parts().unwrap();
            let mob = |id: &str| {
                let kind = game.res.mob_types().get(id).unwrap().clone();
                Mob::new(1, kind, Vec3::ZERO, 0.0)
            };
            let ask = |id: &str, reason| game.spawn_cluster_size(&mut mob(id), reason, None, 4);
            assert_eq!(
                ask("minecraft:zombie", mod_api::SpawnReason::Natural),
                Some(8)
            );
            assert_eq!(
                ask("minecraft:zombie", mod_api::SpawnReason::Spawner),
                Some(1)
            );
            assert_eq!(
                ask("minecraft:skeleton", mod_api::SpawnReason::Natural),
                None
            );
            assert_eq!(ask("minecraft:cow", mod_api::SpawnReason::Natural), None);
        });
    }

    /// `ServerLevel.tick` ticks the raids before the chunk source spawns
    /// (`raids.tick`, then `getChunkSource().tick`): a wave that lands in a
    /// tick joins before that tick's natural and custom spawns, with the
    /// lower entity ids. The phases name themselves in the spans they run in.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn raids_tick_before_natural_and_custom_spawning() {
        let mut resources = None;
        let mut game = crate::game::game_tests::tests::server_game(&mut resources);
        horizon_client::spans::record_here(true);
        let _ = horizon_client::spans::take_raw();
        game.tick_spawners();
        let raw = horizon_client::spans::take_raw();
        horizon_client::spans::record_here(false);
        let phases: Vec<&str> = raw
            .iter()
            .map(|s| s.name)
            .filter(|name| name.starts_with("spawn_"))
            .collect();
        assert_eq!(
            phases,
            ["spawn_raids", "spawn_natural", "spawn_custom"],
            "ServerLevel.tick ticks the raids before the chunk source's spawns"
        );
    }
}
