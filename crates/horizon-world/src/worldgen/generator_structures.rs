//! STRUCTURE_STARTS / STRUCTURE_REFERENCES for the chunk generator.
use super::structure_terrain::{Answer, LandContext, Request};
use super::{
    biome::BiomeId,
    block_feature::FeatureBlockBindings,
    generator::{ChunkWork, GeneratorData, GeneratorError, shard_of},
    structure::{
        StructureResources,
        jigsaw::PoolPiece,
        start::{
            Piece, Rings, StartContext, StructureDef, StructureKind, StructureRegistry,
            StructureStart, beardifier, create_starts, create_starts_with_saved, may_start,
            try_generate,
        },
        stronghold::RingSearch,
    },
};
use horizon_core::{density::Beardifier, geometry::Direction, position::ChunkPos};
use serde_json::Value;
use std::collections::BTreeMap;
use std::{
    collections::{BTreeSet, HashMap},
    sync::{
        Arc, Condvar, Mutex, OnceLock, RwLock, TryLockError,
        atomic::{AtomicUsize, Ordering},
    },
};

pub(super) type Starts = Arc<Vec<Arc<StructureStart>>>;
impl StructureState {
    /// [`StartCache::census`].
    pub(super) fn start_census(&self) -> (usize, usize) {
        self.starts.census()
    }
}
pub(super) struct StructureState {
    pub(super) registry: StructureRegistry,
    pub(super) resources: Arc<StructureResources>,
    pub(super) bindings: FeatureBlockBindings<'static>,
    possible_sets: Vec<usize>,
    /// Structure indices of each decoration step, in registry order.
    pub(super) by_step: Vec<Vec<usize>>,
    starts: StartCache,
    saved: OnceLock<Arc<super::generator::SavedStructureStarts>>,
    start_work: ChunkWork,
    /// Concentric ring positions, computed on first use and shared with
    /// later generators of the same seed and dimension.
    rings: Arc<OnceLock<Rings>>,
    /// Their biome searches while they are computed.
    ring_work: OnceLock<RingWork>,
    /// The seed the rings are searched with when it is not the world's (a superflat world's 0).
    pub(super) ring_seed: Option<i64>,
    /// The biome source is a `FixedBiomeSource` (a single-biome world, a superflat one).
    pub(super) fixed_biome: bool,
    /// `StructureRegistry::zone_reach`.
    zone_reach: i32,
    /// The biome registry by name, for a `structure_terrain` provider's terrain.
    biome_names: super::structure_terrain::BiomeNames,
}

/// generateRingPositions' biome searches, shared out. Vanilla runs each as
/// its own task on its worker pool; here every thread that needs the rings
/// before they exist takes searches until none are left, then waits for the
/// ones others took. Done by the first thread alone, the 128 stronghold
/// searches kept every other worker blocked for seconds after each world load.
struct RingWork {
    /// The biome source is a `FixedBiomeSource`, which has its own horizontal search.
    fixed: bool,
    /// The concentric-ring sets, some with no positions.
    sets: Vec<usize>,
    /// Set index and search, sets in `possible_sets` order.
    searches: Vec<(usize, RingSearch)>,
    next: AtomicUsize,
    found: Vec<OnceLock<ChunkPos>>,
    left: Mutex<usize>,
    finished: Condvar,
}
impl RingWork {
    fn new(sets: Vec<(usize, Vec<RingSearch>)>) -> Self {
        let indices = sets.iter().map(|(set, _)| *set).collect();
        let searches: Vec<(usize, RingSearch)> = sets
            .into_iter()
            .flat_map(|(set, searches)| searches.into_iter().map(move |s| (set, s)))
            .collect();
        Self {
            fixed: false,
            sets: indices,
            next: AtomicUsize::new(0),
            found: (0..searches.len()).map(|_| OnceLock::new()).collect(),
            left: Mutex::new(searches.len()),
            finished: Condvar::new(),
            searches,
        }
    }
    /// Run searches until none are left to take, then wait for the rest.
    fn help<'a>(
        &self,
        preferred: impl Fn(usize) -> &'a BTreeSet<BiomeId>,
        biomes: &mut dyn FnMut(&[[i32; 3]]) -> Vec<BiomeId>,
    ) {
        loop {
            let i = self.next.fetch_add(1, Ordering::Relaxed);
            let Some((set, search)) = self.searches.get(i) else {
                break;
            };
            let found = if self.fixed {
                search.find_fixed(preferred(*set), biomes)
            } else {
                search.find(preferred(*set), biomes)
            };
            let _ = self.found[i].set(found);
            let mut left = self.left.lock().unwrap();
            *left -= 1;
            if *left == 0 {
                self.finished.notify_all();
            }
        }
        let mut left = self.left.lock().unwrap();
        while *left > 0 {
            left = self.finished.wait(left).unwrap();
        }
    }
    /// The positions by set name, once every search is done.
    fn rings(&self, name: impl Fn(usize) -> String) -> Rings {
        let mut rings: Rings = self
            .sets
            .iter()
            .map(|&set| (name(set), Vec::new()))
            .collect();
        for ((set, _), found) in self.searches.iter().zip(&self.found) {
            if let Some(positions) = rings.get_mut(&name(*set)) {
                positions.push(*found.get().expect("every search finished"));
            }
        }
        rings
    }
}
/// Chunks kept per generation of the cache. Each player's generated area
/// reads the starts of some 2,000 chunks (its view distance plus the
/// references' reach, at view distance 10): at 65,536, a server with 200
/// players spread over the map overflowed it every few ticks and rebuilt
/// outposts' and trial chambers' jigsaws on the server thread, where natural
/// spawning asks about them. Most entries are chunks with no start.
const START_CACHE_LIMIT: usize = 262_144;
const START_SHARDS: usize = 64;

/// The built starts by chunk, in shards: a lookup locks one shard for
/// reading. Each generated chunk reads the 289 start chunks around it
/// (`references`) and the server thread reads the starts of the chunks its
/// mobs and players are in; with one lock for all, the server thread waited
/// on the workers.
///
/// Each shard keeps two generations: a full one becomes the old one, and a
/// start found in the old one moves back to the new. The starts still in use
/// (the chunks around every player, which natural spawning asks about each
/// tick) stay, where clearing a full shard threw them away and players spread
/// far apart had their villages' jigsaws built again and again.
struct StartCache {
    shards: Box<[RwLock<StartShard>]>,
}
#[derive(Default)]
struct StartShard {
    new: HashMap<(i32, i32), Starts>,
    old: HashMap<(i32, i32), Starts>,
}
impl StartShard {
    /// Room for one more in the new generation.
    fn make_room(&mut self) {
        if self.new.len() >= START_CACHE_LIMIT / START_SHARDS {
            self.old = std::mem::take(&mut self.new);
        }
    }
}
impl StartCache {
    fn new() -> Self {
        Self {
            shards: (0..START_SHARDS).map(|_| RwLock::default()).collect(),
        }
    }
    fn shard(&self, key: (i32, i32)) -> &RwLock<StartShard> {
        &self.shards[shard_of(key, START_SHARDS)]
    }
    fn get(&self, key: (i32, i32)) -> Option<Starts> {
        let shard = self.shard(key);
        {
            let read = shard.read().unwrap();
            if let Some(found) = read.new.get(&key) {
                return Some(found.clone());
            }
            if !read.old.contains_key(&key) {
                return None;
            }
        }
        let mut write = shard.write().unwrap();
        if let Some(found) = write.new.get(&key) {
            return Some(found.clone());
        }
        let found = write.old.remove(&key)?;
        write.make_room();
        write.new.insert(key, found.clone());
        Some(found)
    }
    /// Keeps `starts` unless another thread built the same chunk's first:
    /// everyone keeps the first, so placement state on its pieces is shared.
    fn insert_first(&self, key: (i32, i32), starts: Starts) -> Starts {
        let mut shard = self.shard(key).write().unwrap();
        let first = shard.new.remove(&key).or_else(|| shard.old.remove(&key));
        shard.make_room();
        shard
            .new
            .entry(key)
            .or_insert(first.unwrap_or(starts))
            .clone()
    }
    /// Chunks cached, and how many of them hold a start.
    fn census(&self) -> (usize, usize) {
        let mut census = (0, 0);
        for shard in &self.shards {
            let shard = shard.read().unwrap();
            census.0 += shard.new.len() + shard.old.len();
            census.1 += shard
                .new
                .values()
                .chain(shard.old.values())
                .filter(|s| !s.is_empty())
                .count();
        }
        census
    }
    fn replace(&self, key: (i32, i32), starts: Starts) {
        let mut shard = self.shard(key).write().unwrap();
        shard.old.remove(&key);
        if !shard.new.contains_key(&key) {
            shard.make_room();
        }
        shard.new.insert(key, starts);
    }
    fn clear(&self) {
        for shard in &self.shards {
            *shard.write().unwrap() = StartShard::default();
        }
    }
}

/// A structure's biome holder set in any form `crate::holder_set` reads. An
/// absent field, an unknown biome or tag names nothing, and a list may also
/// hold tags, as this reader always allowed.
fn biome_set(
    lookup: &impl crate::holder_set::HolderLookup<Id = BiomeId>,
    value: &Value,
) -> Result<BTreeSet<BiomeId>, super::DecodeError> {
    let resolve = |set: &Value| {
        crate::holder_set::resolve(set, lookup)
            .map(|ids| ids.into_iter().collect::<BTreeSet<_>>())
            .map_err(super::DecodeError::Registry)
    };
    match value {
        Value::Array(list) => list.iter().filter(|value| value.is_string()).try_fold(
            BTreeSet::new(),
            |mut out, set| {
                out.extend(resolve(set)?);
                Ok::<_, super::DecodeError>(out)
            },
        ),
        set @ (Value::String(_) | Value::Object(_)) => resolve(set),
        _ => Ok(BTreeSet::new()),
    }
}
struct StructureBiomes<'a>(&'a GeneratorData);
impl crate::holder_set::HolderLookup for StructureBiomes<'_> {
    type Id = BiomeId;
    fn what(&self) -> &str {
        "biome"
    }
    fn entry(&self, id: &str) -> Result<Vec<BiomeId>, String> {
        Ok(self.0.biome_id(id).into_iter().collect())
    }
    fn tag(&self, tag: &str) -> Result<Vec<BiomeId>, String> {
        let members = self.0.biome_tags.get(tag).into_iter().flatten();
        Ok(members.filter_map(|biome| self.0.biome_id(biome)).collect())
    }
    fn all(&self) -> Result<Vec<BiomeId>, String> {
        Ok((0..self.0.biome_names().len() as u32)
            .map(BiomeId)
            .collect())
    }
}
impl GeneratorData {
    pub(super) fn structure_biome_set(
        &self,
        value: &Value,
    ) -> Result<BTreeSet<BiomeId>, super::DecodeError> {
        biome_set(&StructureBiomes(self), value)
    }
    pub(in crate::worldgen) fn structure_block_bindings(
        &self,
    ) -> Result<FeatureBlockBindings<'static>, super::DecodeError> {
        let mut bindings = self.feature_block_bindings()?;
        // Processors that test what a block is made of read canOcclude.
        if let Some(blocks) = self
            .feature_resources
            .as_ref()
            .and_then(|r| r.blocks.as_ref())
        {
            bindings = bindings.with_block_properties(&blocks.properties)?;
        }
        Ok(bindings)
    }
}

impl StructureState {
    pub(super) fn new(
        data: &Arc<GeneratorData>,
        possible_biomes: &BTreeSet<BiomeId>,
        rings: Arc<OnceLock<Rings>>,
    ) -> Result<Option<Self>, GeneratorError> {
        let (Some(resources), Some(raw)) = (&data.structures, &data.structure_resources) else {
            return Ok(None);
        };
        let registry = StructureRegistry::new(
            &raw.structures,
            &raw.sets,
            raw.pools.clone(),
            &|value| biome_set(&StructureBiomes(data), value),
            super::structure::mod_types::mod_structure_types(),
        )?;
        let possible_sets = registry
            .sets
            .iter()
            .enumerate()
            .filter(|(_, set)| {
                set.structures
                    .iter()
                    .any(|(s, _)| !registry.structures[*s].biomes.is_disjoint(possible_biomes))
            })
            .map(|(i, _)| i)
            .collect();
        let mut by_step = vec![Vec::new(); super::structure::start::STEPS.len()];
        for (i, def) in registry.structures.iter().enumerate() {
            by_step[def.step].push(i);
        }
        let bindings = data.structure_block_bindings()?;
        let zone_reach = registry.zone_reach();
        let biome_names = super::structure_terrain::BiomeNames::new(data.biome_names());
        Ok(Some(Self {
            zone_reach,
            biome_names,
            registry,
            resources: resources.clone(),
            bindings,
            possible_sets,
            by_step,
            starts: StartCache::new(),
            saved: OnceLock::new(),
            start_work: ChunkWork::new(),
            rings,
            ring_work: OnceLock::new(),
            ring_seed: None,
            fixed_biome: false,
        }))
    }
    /// `FlatLevelGeneratorSettings.structureOverrides`: only the structure sets named
    /// (`minecraft:villages`) may generate.
    pub(super) fn restrict_to(&mut self, names: &[String]) {
        let registry = &self.registry;
        let bare = |name: &str| name.strip_prefix("minecraft:").unwrap_or(name).to_owned();
        // `lenientOptionalFieldOf`: a list naming a set that does not exist fails to decode,
        // which counts as the key being absent: every set generates.
        if names.iter().any(|name| {
            !registry
                .sets
                .iter()
                .any(|set| bare(&set.name) == bare(name))
        }) {
            return;
        }
        self.possible_sets.retain(|&set| {
            names.iter().any(|name| {
                name.strip_prefix("minecraft:").unwrap_or(name)
                    == registry.sets[set]
                        .name
                        .strip_prefix("minecraft:")
                        .unwrap_or(&registry.sets[set].name)
            })
        });
    }
    /// Concentric ring positions of a structure set (vanilla's
    /// getRingPositionsFor), computed now if no lookup needed them yet.
    pub(super) fn ring_positions(
        &self,
        context: &mut impl StartContext,
        set: &str,
    ) -> Option<Vec<ChunkPos>> {
        self.rings(context).get(set).cloned()
    }
    fn ring_work(&self, context: &impl StartContext) -> &RingWork {
        self.ring_work.get_or_init(|| {
            let mut work = RingWork::new(self.registry.ring_searches(
                &self.possible_sets,
                self.ring_seed.unwrap_or_else(|| context.seed()),
            ));
            work.fixed = self.fixed_biome;
            work
        })
    }
    /// The concentric ring positions, computing them with the threads that
    /// need them first (see `RingWork`).
    fn rings(&self, context: &mut impl StartContext) -> &Rings {
        if let Some(rings) = self.rings.get() {
            return rings;
        }
        let work = self.ring_work(context);
        let registry = &self.registry;
        work.help(|set| registry.ring_biomes(set), &mut |quarts| {
            context.noise_biomes(quarts)
        });
        self.rings
            .get_or_init(|| work.rings(|set| registry.sets[set].name.clone()))
    }
    /// The rings a chunk's starts are decided with. Before they are known,
    /// a chunk no ring search can end near (with the exclusion zones' reach)
    /// is decided without them: no ring chunk is near it, which is what the
    /// rings would say. Strongholds lie 1,400 blocks out or more, so a new
    /// world's first chunks don't wait for the 128 biome searches (seconds
    /// on four threads); they run when a chunk near a ring needs them.
    fn rings_for(&self, context: &mut impl StartContext, chunk: ChunkPos) -> &Rings {
        static NONE: Rings = Rings::new();
        if let Some(rings) = self.rings.get() {
            return rings;
        }
        let near = self
            .ring_work(context)
            .searches
            .iter()
            .any(|(_, search)| search.may_land_near(chunk.x, chunk.z, self.zone_reach));
        if near { self.rings(context) } else { &NONE }
    }
    /// The structures of the random-spread sets whose start chunk is `chunk`, each with
    /// [`may_start`]'s answer, for tests.
    #[cfg(test)]
    pub(super) fn may_start_answers(
        &self,
        context: &mut impl super::structure::start::StartContext,
        chunk: ChunkPos,
    ) -> Vec<(String, bool)> {
        let seed = context.seed();
        let rings = self.rings_for(context, chunk).clone();
        let mut checks = Vec::new();
        for &si in &self.possible_sets {
            let set = &self.registry.sets[si];
            if set.spacing().is_none()
                || StructureRegistry::potential_chunk(set, seed, chunk.x, chunk.z) != Some(chunk)
                || !(self.registry).is_structure_chunk(set, seed, chunk.x, chunk.z, &rings)
            {
                continue;
            }
            for &(s, _) in &set.structures {
                let may = may_start(
                    &self.registry,
                    &self.resources,
                    &self.bindings,
                    context,
                    s,
                    chunk,
                );
                checks.push((self.registry.structures[s].name.clone(), may));
            }
        }
        checks
    }
    /// `StructureRegistry::needs_dimension_origin`: whether any set is placed
    /// by the dimension's origin chunk alone.
    pub(super) fn needs_dimension_origin(&self) -> bool {
        self.registry.needs_dimension_origin()
    }
    /// `ChunkGenerator.getOrigin` for this dimension's structure state.
    pub(super) fn set_dimension_origin(&mut self, origin: ChunkPos) {
        self.registry.set_dimension_origin(origin);
    }
    /// Drop the cached starts, so that later chunks build fresh pieces (the
    /// pieces keep per-start placement state such as a shipwreck's height).
    pub(super) fn forget_starts(&self) {
        // As before, a build already in flight can publish after this clear.
        // Do not wait for work locks or retain initialized entries here.
        self.starts.clear();
    }
    /// The (cached) structure starts owned by a chunk.
    pub(super) fn starts_at(
        &self,
        context: &mut impl super::structure::start::StartContext,
        chunk: ChunkPos,
    ) -> Starts {
        self.build_starts(context, chunk, true)
            .expect("waits for another worker's build")
    }
    /// The starts a chunk owns when they are cached already, else None: nothing is read from
    /// the save and nothing is built, for a read that may load no chunk.
    pub(super) fn cached_starts_at(&self, chunk: ChunkPos) -> Option<Starts> {
        self.starts.get((chunk.x, chunk.z))
    }
    /// The starts a chunk saved (`structures.starts`), which vanilla loads with their pieces
    /// (`StructureStart.loadStaticStart`) so that the neighbours place the parts that cross
    /// into them and the beardifier shapes the terrain around them. A jigsaw start (villages,
    /// outposts, bastions, ancient cities, trial chambers, trail ruins) is loaded from its
    /// saved pieces (`PoolPiece::from_saved`); a template piece (shipwrecks, ocean ruins,
    /// ruined portals, igloos, mansions, nether fossils and end cities) loads through
    /// `saved_templates::read`, and a piece that reads its own tags (the mineshaft, fortress
    /// and stronghold families and the swamp hut) through `saved_legacy::read`.
    /// A piece of another type has no loader yet:
    /// it is the start this generator makes in the chunk when that one has the same name and the
    /// same pieces' box (the same layout from the same seed), else the name and box alone (found
    /// by `/locate` and maps, but not placed).
    fn saved_starts_at(
        &self,
        context: &mut impl super::structure::start::StartContext,
        chunk: ChunkPos,
    ) -> Starts {
        let saved = context.saved_starts(chunk);
        let mut generated: Option<Vec<Arc<StructureStart>>> = None;
        let starts = saved
            .into_iter()
            .filter_map(|start| {
                let index = (self.registry.structures.iter()).position(|s| s.name == start.name)?;
                let def = &self.registry.structures[index];
                if matches!(&def.kind, StructureKind::Monument(_)) {
                    // An old chunk's monument is `StructureStart.loadStaticStart`'s
                    // `PiecesContainer.load` plus `regeneratePiecesAfterLoad` too: the first
                    // saved piece that loads supplies the box and orientation, and the
                    // rooms are built again from the seed.
                    let top = start.pieces.iter().find_map(|piece| {
                        saved_anchor(piece, |piece| {
                            Self::saved_piece(piece, def, &self.resources, &self.bindings)
                        })
                    });
                    if let Some((bounding_box, orientation)) = top {
                        let piece =
                            super::structure::monument::MonumentBuilding::regenerate_after_load(
                                bounding_box,
                                orientation,
                                chunk,
                                context.seed(),
                            );
                        return Some(Arc::new(StructureStart::new(
                            index,
                            def,
                            chunk,
                            vec![Piece::Legacy(Box::new(piece))],
                        )));
                    }
                }
                if !start.pieces.is_empty()
                    && let Some(pieces) = start
                        .pieces
                        .iter()
                        .map(|piece| Self::saved_piece(piece, def, &self.resources, &self.bindings))
                        .collect::<Option<Vec<_>>>()
                {
                    return Some(Arc::new(StructureStart::new(index, def, chunk, pieces)));
                }
                if let Some(bounds) = start.bounds {
                    let made = generated.get_or_insert_with(|| {
                        let rings = self.rings_for(context, chunk);
                        create_starts(
                            &self.registry,
                            &self.resources,
                            &self.bindings,
                            context,
                            &self.possible_sets,
                            rings,
                            chunk,
                        )
                        .into_iter()
                        .map(Arc::new)
                        .collect()
                    });
                    let same = made.iter().find(|made| {
                        made.name == start.name && pieces_box(&made.pieces) == Some(bounds)
                    });
                    if let Some(same) = same {
                        return Some(same.clone());
                    }
                }
                let [x0, y0, z0, x1, y1, z1] = start.bounds.unwrap_or([
                    chunk.x * 16,
                    0,
                    chunk.z * 16,
                    chunk.x * 16 + 15,
                    0,
                    chunk.z * 16 + 15,
                ]);
                Some(Arc::new(StructureStart {
                    structure: index,
                    name: start.name,
                    chunk,
                    references: 0,
                    pieces: Vec::new(),
                    bounding_box: horizon_core::bounding_box::BoundingBox::new(
                        [x0, y0, z0],
                        [x1, y1, z1],
                    ),
                    has_piece_beardifier: false,
                }))
            })
            .collect();
        Arc::new(starts)
    }
    /// A saved `minecraft:jigsaw` piece, or `None` for any other piece or one that does not decode.
    fn saved_pool_piece(
        piece: &horizon_core::nbt::Tag,
        resources: &StructureResources,
        bindings: &FeatureBlockBindings<'_>,
    ) -> Option<Piece> {
        saved_piece_id(piece)?;
        PoolPiece::from_saved(&tag_json(piece), resources, bindings)
            .ok()
            .map(Piece::Pool)
    }
    /// A saved piece, as `StructureStart.loadStaticStart` loads it: a jigsaw
    /// piece from `PoolPiece::from_saved`, a template piece from
    /// `saved_templates::read`, a vanilla legacy piece that reads its own tags
    /// from `saved_legacy::read`, a mod type's piece from its `load_piece` (the
    /// `StructurePieceType` is the loader), none otherwise. `None` leaves the
    /// whole start to the fallback above.
    fn saved_piece(
        piece: &horizon_core::nbt::Tag,
        def: &StructureDef,
        resources: &StructureResources,
        bindings: &FeatureBlockBindings<'_>,
    ) -> Option<Piece> {
        if let Some(pool) = Self::saved_pool_piece(piece, resources, bindings) {
            return Some(pool);
        }
        let horizon_core::nbt::Tag::Compound(map) = piece else {
            return None;
        };
        let id = match map.get(&"id".into()) {
            Some(horizon_core::nbt::Tag::String(s)) => s.to_string_lossy(),
            _ => return None,
        };
        let vanilla = id.strip_prefix("minecraft:").unwrap_or(&id);
        if let Some(legacy) =
            super::structure::saved_templates::read(vanilla, piece, resources, bindings)
        {
            return Some(Piece::Legacy(legacy));
        }
        if let Some(legacy) = super::structure::saved_legacy::read(vanilla, piece, def) {
            return Some(Piece::Legacy(legacy));
        }
        let types = super::structure::mod_types::mod_structure_types();
        types
            .get(&*id)?
            .load_piece(resources.registry, map)
            .map(Piece::Legacy)
    }
    pub(super) fn set_saved_starts(&self, source: Arc<super::generator::SavedStructureStarts>) {
        let _ = self.saved.set(source);
    }
    /// `starts_at`, or `None` when another worker is building them and
    /// `wait` is false.
    fn build_starts(
        &self,
        context: &mut impl super::structure::start::StartContext,
        chunk: ChunkPos,
        wait: bool,
    ) -> Option<Starts> {
        if let Some(found) = self.starts.get((chunk.x, chunk.z)) {
            return Some(found);
        }
        if context.is_old_chunk(chunk) {
            // Decoded once: the neighbours ask for these starts again and again.
            let starts = self.saved_starts_at(context, chunk);
            return Some(self.starts.insert_first((chunk.x, chunk.z), starts));
        }
        match self.terrain_answer(context, chunk, None) {
            Answer::Vanilla => self.build_new_starts(context, chunk, wait, true),
            // Starts the chunk saved stay, as they do for the other answers: only
            // new ones are dropped.
            Answer::NoStructures => self.build_new_starts(context, chunk, wait, false),
            Answer::Terrain(terrain) => {
                let mut land = LandContext::new(context, terrain, &self.biome_names);
                self.build_new_starts(&mut land, chunk, wait, true)
            }
        }
    }
    /// What the `structure_terrain` providers answer for `chunk`.
    fn terrain_answer(
        &self,
        context: &impl StartContext,
        chunk: ChunkPos,
        placing: Option<&str>,
    ) -> Answer {
        if !super::structure_terrain::any_registered() {
            return Answer::Vanilla;
        }
        let structures = self.possible_sets.iter().flat_map(|&set| {
            (self.registry.sets[set].structures.iter())
                .map(|&(structure, _)| self.registry.structures[structure].name.as_str())
        });
        let mut names: Vec<&str> = Vec::new();
        for name in structures {
            if !names.contains(&name) {
                names.push(name);
            }
        }
        let request = Request::new(
            context.seed(),
            chunk,
            placing,
            context.generation(),
            names,
            Some(&self.biome_names),
        );
        super::structure_terrain::answer(&request)
    }
    /// `build_starts` for a chunk with no starts built or saved: they are
    /// built now, on `context`'s biomes and heights. With `new` false only the
    /// starts the chunk saved are kept and no other is made.
    fn build_new_starts(
        &self,
        context: &mut impl super::structure::start::StartContext,
        chunk: ChunkPos,
        wait: bool,
        new: bool,
    ) -> Option<Starts> {
        static NONE: Rings = Rings::new();
        // Ring searches enlist other workers and wait for them: never hold a
        // chunk-work lock while helping, even when this chunk has no starts.
        let rings = if new {
            self.rings_for(context, chunk)
        } else {
            &NONE
        };
        let possible: &[usize] = if new { &self.possible_sets } else { &[] };
        let work = self.start_work.at((chunk.x, chunk.z));
        let _work = match work.try_lock() {
            Ok(guard) => guard,
            Err(TryLockError::WouldBlock) if !wait => return None,
            Err(_) => work.lock().unwrap(),
        };
        if let Some(found) = self.starts.get((chunk.x, chunk.z)) {
            return Some(found);
        }
        let saved = self.saved.get().and_then(|source| source(chunk));
        let saved = saved
            .iter()
            .flat_map(|starts| starts.iter())
            .filter_map(|(name, tag)| {
                let index = self
                    .registry
                    .structures
                    .iter()
                    .position(|s| s.name == name.to_string_lossy())?;
                read_saved_start(
                    index,
                    &self.registry.structures[index],
                    chunk,
                    tag,
                    context.seed(),
                    &self.resources,
                    &self.bindings,
                )
            })
            .collect();
        let starts: Starts = Arc::new(
            create_starts_with_saved(
                &self.registry,
                &self.resources,
                &self.bindings,
                context,
                possible,
                rings,
                chunk,
                saved,
            )
            .into_iter()
            .map(Arc::new)
            .collect(),
        );
        Some(self.starts.insert_first((chunk.x, chunk.z), starts))
    }
    /// `Structure.generate` for `/place structure`: the start `structure`
    /// builds in `chunk`, whether or not a structure set would place one
    /// there (vanilla calls the structure's own generate, not the set's
    /// placement, with its `biome -> true` predicate). Nothing is cached: the
    /// chunk's own starts are untouched.
    pub(super) fn structure_start(
        &self,
        context: &mut impl StartContext,
        structure: &str,
        chunk: ChunkPos,
    ) -> Option<StructureStart> {
        let index = (self.registry.structures.iter()).position(|s| s.name == structure)?;
        match self.terrain_answer(context, chunk, Some(structure)) {
            Answer::Vanilla => try_generate(
                &self.registry,
                &self.resources,
                &self.bindings,
                context,
                index,
                chunk,
                true,
            ),
            Answer::NoStructures => None,
            Answer::Terrain(terrain) => try_generate(
                &self.registry,
                &self.resources,
                &self.bindings,
                &mut LandContext::new(context, terrain, &self.biome_names),
                index,
                chunk,
                true,
            ),
        }
    }
    /// Puts a saved start of `structure` in `chunk`, in place of the one
    /// this generator builds there from the same structure sets (a set
    /// starts one structure per chunk): `pieces` as a save lists them (see
    /// `PoolPiece::from_saved`). Until the starts are forgotten.
    pub(super) fn use_saved_start(
        &self,
        context: &mut impl super::structure::start::StartContext,
        chunk: ChunkPos,
        structure: &str,
        pieces: &[Value],
    ) -> Result<(), String> {
        let index = (self.registry.structures.iter())
            .position(|s| s.name == structure)
            .ok_or_else(|| format!("unknown structure {structure}"))?;
        let pieces = (pieces.iter())
            .map(|p| PoolPiece::from_saved(p, &self.resources, &self.bindings).map(Piece::Pool))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("{structure} at {chunk:?}: {e}"))?;
        if pieces.is_empty() {
            return Err(format!("{structure} at {chunk:?}: no pieces"));
        }
        let def = &self.registry.structures[index];
        let sets: Vec<_> = (self.registry.sets.iter())
            .filter(|set| set.structures.iter().any(|(s, _)| *s == index))
            .collect();
        let same_set = |s: usize| {
            s == index
                || sets
                    .iter()
                    .any(|set| set.structures.iter().any(|(t, _)| *t == s))
        };
        let mut starts: Vec<_> = (self.starts_at(context, chunk).iter())
            .filter(|s| !same_set(s.structure))
            .cloned()
            .collect();
        starts.push(Arc::new(StructureStart::new(index, def, chunk, pieces)));
        self.starts.replace((chunk.x, chunk.z), Arc::new(starts));
        Ok(())
    }
    /// ChunkGenerator.findNearestMapStructure: the nearest start of one of
    /// `names` from block `from` (rings: every ring chunk; random spreads:
    /// sectors in growing squares up to `radius`). Returns the structure and
    /// its locate position (the start chunk's corner at y 0). `skip` says a
    /// start (structure name, its chunk) can't be referenced again
    /// (`StructureStart.canBeReferenced`): the search goes on past it.
    pub(super) fn locate<C: super::structure::start::StartContext>(
        &self,
        context: &mut C,
        names: &[String],
        from: [i32; 3],
        radius: i32,
        skip: &dyn Fn(&str, ChunkPos) -> bool,
    ) -> Option<(String, [i32; 3])> {
        let seed = context.seed();
        let rings = self.rings(context).clone();
        let wanted = |s: usize| names.iter().any(|n| *n == self.registry.structures[s].name);
        let dist = |p: [i32; 3]| {
            let (dx, dz) = (f64::from(p[0] - from[0]), f64::from(p[2] - from[2]));
            dx * dx + dz * dz
        };
        let mut best: Option<(f64, String, [i32; 3])> = None;
        let wanted_structures: Vec<usize> = (0..self.registry.structures.len())
            .filter(|&s| wanted(s))
            .collect();
        let found_at = |this: &Self, context: &mut C, chunk: ChunkPos| -> Option<String> {
            // StructureCheck.checkStart: a chunk with no starts built or saved yet is first
            // asked whether a wanted structure could start there (canCreateStructure, a
            // jigsaw's start piece and its biome), and only then are all its starts built,
            // layouts included. A map's search for a camp of a rarer biome built thousands of
            // chunks' starts on the server thread, seconds of it (#8669).
            if this.starts.get((chunk.x, chunk.z)).is_none()
                && !context.is_old_chunk(chunk)
                && !wanted_structures.iter().any(|&s| {
                    may_start(
                        &this.registry,
                        &this.resources,
                        &this.bindings,
                        context,
                        s,
                        chunk,
                    )
                })
            {
                return None;
            }
            this.starts_at(context, chunk)
                .iter()
                .find(|s| wanted(s.structure) && !skip(&s.name, chunk))
                .map(|s| s.name.clone())
        };
        let mut spaced_sets = Vec::new();
        for &si in &self.possible_sets {
            let set = &self.registry.sets[si];
            if !set.structures.iter().any(|(s, _)| wanted(*s)) {
                continue;
            }
            match set.spacing() {
                None => {
                    // Rings: nearest by the chunk's (8, 32, 8) block.
                    let Some(chunks) = rings.get(&set.name) else {
                        continue;
                    };
                    let mut nearest: Option<(f64, ChunkPos, String)> = None;
                    for &c in chunks {
                        let d = dist([c.x * 16 + 8, 32, c.z * 16 + 8]);
                        if nearest.as_ref().is_none_or(|(n, ..)| d < *n)
                            && self
                                .registry
                                .is_structure_chunk(set, seed, c.x, c.z, &rings)
                            && let Some(name) = found_at(self, context, c)
                        {
                            nearest = Some((d, c, name));
                        }
                    }
                    if let Some((_, c, name)) = nearest {
                        let p = set.locate_pos(c);
                        let d = dist(p);
                        if best.as_ref().is_none_or(|(b, ..)| d < *b) {
                            best = Some((d, name, p));
                        }
                    }
                }
                Some(spacing) => {
                    spaced_sets.push((si, spacing));
                }
            }
        }
        if !spaced_sets.is_empty() {
            let (cx, cz) = (from[0] >> 4, from[2] >> 4);
            // ChunkGenerator.findNearestMapStructure: expands outward ring by ring across all
            // candidate sets, stops at the first ring with any hit, and takes the first hit in
            // each ring for each set.
            for r in 0..=radius {
                let mut found_in_ring = false;
                for &(si, spacing) in &spaced_sets {
                    let set = &self.registry.sets[si];
                    'ring: for dx in -r..=r {
                        let x_edge = dx == -r || dx == r;
                        for dz in -r..=r {
                            if !(x_edge || dz == -r || dz == r) {
                                continue;
                            }
                            let (sx, sz) = (cx + spacing * dx, cz + spacing * dz);
                            let Some(c) = StructureRegistry::potential_chunk(set, seed, sx, sz)
                            else {
                                continue;
                            };
                            if !self
                                .registry
                                .is_structure_chunk(set, seed, c.x, c.z, &rings)
                            {
                                continue;
                            }
                            if let Some(name) = found_at(self, context, c) {
                                let p = set.locate_pos(c);
                                let d = dist(p);
                                if best.as_ref().is_none_or(|(b, ..)| d < *b) {
                                    best = Some((d, name, p));
                                }
                                found_in_ring = true;
                                break 'ring;
                            }
                        }
                    }
                }
                if found_in_ring {
                    break;
                }
            }
        }
        best.map(|(_, n, p)| (n, p))
    }
    /// ChunkGenerator.createReferences: starts within 8 chunks whose box
    /// intersects this chunk's column.
    pub(super) fn references(
        &self,
        context: &mut impl super::structure::start::StartContext,
        chunk: ChunkPos,
    ) -> Vec<Arc<StructureStart>> {
        let (bx, bz) = (chunk.x * 16, chunk.z * 16);
        let mut out = Vec::new();
        let push_intersecting = |out: &mut Vec<Arc<StructureStart>>,
                                 starts: &[Arc<StructureStart>]| {
            for start in starts {
                let b = start.bounding_box;
                if b.max[0] >= bx && b.min[0] <= bx + 15 && b.max[2] >= bz && b.min[2] <= bz + 15 {
                    out.push(start.clone());
                }
            }
        };
        // Workers generating neighbouring chunks ask for the same starts in the
        // same order: build those no other worker is building first, and wait
        // for the others only then, rather than all queueing behind each one.
        let mut busy = [ChunkPos::new(0, 0); 289];
        let mut busy_count = 0usize;
        let mut deferred: [Option<Starts>; 289] = [const { None }; 289];
        let mut idx = 0usize;
        for sx in chunk.x - 8..=chunk.x + 8 {
            for sz in chunk.z - 8..=chunk.z + 8 {
                let at = ChunkPos::new(sx, sz);
                match self.build_starts(context, at, false) {
                    Some(starts) if !starts.is_empty() => {
                        if busy_count == 0 {
                            push_intersecting(&mut out, starts.as_slice());
                        } else {
                            deferred[idx] = Some(starts);
                        }
                    }
                    Some(_) => {}
                    None => {
                        busy[busy_count] = at;
                        busy_count += 1;
                    }
                }
                idx += 1;
            }
        }
        if busy_count > 0 {
            let mut bi = 0usize;
            let mut slot_idx = 0usize;
            for sx in chunk.x - 8..=chunk.x + 8 {
                for sz in chunk.z - 8..=chunk.z + 8 {
                    let at = ChunkPos::new(sx, sz);
                    if bi < busy_count && busy[bi] == at {
                        bi += 1;
                        let starts = self.starts_at(context, at);
                        push_intersecting(&mut out, starts.as_slice());
                    } else if let Some(starts) = deferred[slot_idx].take() {
                        push_intersecting(&mut out, starts.as_slice());
                    }
                    slot_idx += 1;
                }
            }
        }
        if out.len() <= 1 {
            return out;
        }
        // Vanilla keeps each structure's references in a LongOpenHashSet and
        // places the starts in its iteration order; reorder each structure's
        // starts within the slots they already occupy.
        let mut slots: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        for (i, start) in out.iter().enumerate() {
            slots.entry(start.structure).or_default().push(i);
        }
        let original = out.clone();
        for slots in slots.values().filter(|s| s.len() > 1) {
            let keys: Vec<i64> = slots
                .iter()
                .map(|&i| {
                    let c = original[i].chunk;
                    i64::from(c.x as u32) | (i64::from(c.z as u32) << 32)
                })
                .collect();
            for (&slot, &from) in slots.iter().zip(&super::long_set::iteration_order(&keys)) {
                out[slot] = original[slots[from]].clone();
            }
        }
        out
    }
    pub(super) fn beardifier(
        &self,
        references: &[Arc<StructureStart>],
        chunk: ChunkPos,
    ) -> Option<Beardifier> {
        beardifier(&self.registry, references, chunk)
    }
}

/// The union of the pieces' boxes as `[min x, min y, min z, max x, max y, max z]`.
fn pieces_box(pieces: &[Piece]) -> Option<[i32; 6]> {
    let mut union: Option<horizon_core::bounding_box::BoundingBox> = None;
    for piece in pieces {
        let b = piece.bounding_box();
        match union.as_mut() {
            Some(u) => u.encapsulate(&b),
            None => union = Some(b),
        }
    }
    union.map(|b| [b.min[0], b.min[1], b.min[2], b.max[0], b.max[1], b.max[2]])
}
/// `PiecesContainer.load` keeps one of a monument's own pieces when `StructurePieceType`
/// registers its id (`OMB` and the eleven room ids) and the `StructurePiece(StructurePieceType,
/// CompoundTag)` constructor reads its box. An unknown id is logged and dropped, and a
/// constructor that throws drops the piece too.
fn monument_piece(piece: &horizon_core::nbt::Tag) -> bool {
    matches!(
        saved_piece_name(piece).as_deref(),
        Some(
            "minecraft:omb"
                | "minecraft:omcr"
                | "minecraft:omdxr"
                | "minecraft:omdxyr"
                | "minecraft:omdyr"
                | "minecraft:omdyzr"
                | "minecraft:omdzr"
                | "minecraft:omentry"
                | "minecraft:ompenthouse"
                | "minecraft:omsimple"
                | "minecraft:omsimplet"
                | "minecraft:omwr"
        )
    ) && piece
        .get("BB")
        .and_then(super::structure::saved_legacy::read_box)
        .is_some()
}
/// The box and orientation `StructureStart.loadStaticStart` gives the first saved piece
/// `PiecesContainer.load` keeps, or `None` when the piece does not load. A monument
/// piece's own constructor keeps the saved `BB` (a `Codec.INT_STREAM`, so an int array or
/// a numeric list) and `O`; a template piece's constructor resets the orientation to NORTH
/// and rebuilds its box from the template (`TemplateStructurePiece`); every other loader
/// keeps the saved `O`.
fn saved_anchor(
    piece: &horizon_core::nbt::Tag,
    decode: impl FnOnce(&horizon_core::nbt::Tag) -> Option<Piece>,
) -> Option<(horizon_core::bounding_box::BoundingBox, Option<Direction>)> {
    if monument_piece(piece) {
        let bounding_box = piece
            .get("BB")
            .and_then(super::structure::saved_legacy::read_box)?;
        return Some((
            bounding_box,
            super::structure::monument::saved_orientation(piece),
        ));
    }
    let decoded = decode(piece)?;
    let orientation = if super::structure::saved_templates::is_template_piece(piece) {
        Some(Direction::North)
    } else {
        super::structure::monument::saved_orientation(piece)
    };
    Some((decoded.bounding_box(), orientation))
}
/// `StructureStart.loadStaticStart` / `PiecesContainer.load`, restricted to complete supported
/// starts. Unsupported or unreadable children leave the entire start to regeneration; an
/// ocean monument keeps the first child that loads and rebuilds its rooms.
pub(super) fn read_saved_start(
    index: usize,
    def: &StructureDef,
    chunk: ChunkPos,
    tag: &horizon_core::nbt::Tag,
    seed: i64,
    resources: &StructureResources,
    bindings: &FeatureBlockBindings<'_>,
) -> Option<StructureStart> {
    use horizon_core::nbt::Tag;
    if !matches!(tag.get("id"), Some(Tag::String(id)) if id.to_string_lossy() == def.name) {
        return None;
    }
    let int = |key| super::structure::saved_legacy::int(tag, key);
    if ChunkPos::new(int("ChunkX"), int("ChunkZ")) != chunk {
        return None;
    }
    let Tag::List(children) = tag.get("Children")? else {
        return None;
    };
    if children.is_empty() {
        return None;
    }
    // `StructureStart.loadStaticStart` decodes each child through `PiecesContainer.load`,
    // which drops every piece it cannot read and keeps the rest in saved order.
    let decode = |piece: &horizon_core::nbt::Tag| -> Option<Piece> {
        // `PiecesContainer.load`'s RENAMES read the old jigsaw ids as `jigsaw`; the other
        // supported types are template pieces and the mineshaft, fortress and stronghold
        // pieces that read their own tags.
        if saved_piece_id(piece).is_none() {
            let Tag::String(id) = piece.get("id")? else {
                return None;
            };
            let id = id.to_string_lossy();
            let id = id.strip_prefix("minecraft:").unwrap_or(&id);
            return super::structure::saved_templates::read(id, piece, resources, bindings)
                .or_else(|| super::structure::saved_legacy::read(id, piece, def))
                .map(Piece::Legacy);
        }
        // Pool pieces have no orientation and depth 0. Preserve other values by falling
        // back until the representation can hold them, rather than silently losing them.
        if piece.get("O") != Some(&Tag::Int(-1)) || piece.get("GD") != Some(&Tag::Int(0)) {
            return None;
        }
        PoolPiece::from_saved(&tag_json(piece), resources, bindings)
            .ok()
            .map(Piece::Pool)
    };
    if matches!(&def.kind, StructureKind::Monument(_)) {
        // `OceanMonumentStructure.regeneratePiecesAfterLoad` throws the saved rooms away,
        // reads the box and orientation of the first piece `PiecesContainer.load` kept and
        // builds the rooms again from the seed. A piece it cannot read (an unknown id, or
        // no box) is dropped and the next one is tried; no survivor leaves the start to
        // regeneration.
        let (bounding_box, orientation) = children
            .iter()
            .find_map(|piece| saved_anchor(piece, |piece| decode(piece)))?;
        let piece = super::structure::monument::MonumentBuilding::regenerate_after_load(
            bounding_box,
            orientation,
            chunk,
            seed,
        );
        let mut start =
            StructureStart::new(index, def, chunk, vec![Piece::Legacy(Box::new(piece))]);
        start.references = int("references");
        return Some(start);
    }
    let pieces = children.iter().map(decode).collect::<Option<Vec<_>>>()?;
    let mut start = StructureStart::new(index, def, chunk, pieces);
    start.references = int("references");
    Some(start)
}
/// `PiecesContainer.load`'s piece id, after its `RENAMES`: the four ids an older
/// save wrote for a jigsaw piece (`minecraft:nvi`, `minecraft:pcp`,
/// `minecraft:bastionremnant`, `minecraft:runtime`) are read as `minecraft:jigsaw`,
/// which is what vanilla's table maps them to. `None` for a piece of another type
/// (its NBT is not a pool piece's) and for one with no id.
fn saved_piece_id(piece: &horizon_core::nbt::Tag) -> Option<&'static str> {
    match saved_piece_name(piece)?.as_str() {
        "minecraft:jigsaw"
        | "minecraft:nvi"
        | "minecraft:pcp"
        | "minecraft:bastionremnant"
        | "minecraft:runtime" => Some("minecraft:jigsaw"),
        _ => None,
    }
}
/// `PiecesContainer.load`'s piece id: lowercased, and with `minecraft:` filled in for an id
/// with no namespace (`Identifier.parse`).
pub(crate) fn saved_piece_name(piece: &horizon_core::nbt::Tag) -> Option<String> {
    let horizon_core::nbt::Tag::Compound(map) = piece else {
        return None;
    };
    let horizon_core::nbt::Tag::String(id) = map.get(&"id".into())? else {
        return None;
    };
    let id = id.to_string_lossy().to_lowercase();
    super::tags::identifier(&id).ok()
}
/// A saved piece's tag as the JSON `PoolPiece::from_saved` reads: compounds and lists as they
/// are, numbers as numbers.
pub(crate) fn tag_json(tag: &horizon_core::nbt::Tag) -> Value {
    use horizon_core::nbt::Tag;
    match tag {
        Tag::Byte(v) => Value::from(i64::from(*v)),
        Tag::Short(v) => Value::from(i64::from(*v)),
        Tag::Int(v) => Value::from(i64::from(*v)),
        Tag::Long(v) => Value::from(*v),
        Tag::Float(bits) => Value::from(f64::from(f32::from_bits(*bits))),
        Tag::Double(bits) => Value::from(f64::from_bits(*bits)),
        Tag::String(s) => Value::from(s.to_string_lossy()),
        Tag::List(items) => Value::Array(items.iter().map(tag_json).collect()),
        Tag::ByteArray(v) => Value::Array(v.iter().map(|b| Value::from(i64::from(*b))).collect()),
        Tag::IntArray(v) => Value::Array(v.iter().map(|i| Value::from(i64::from(*i))).collect()),
        Tag::LongArray(v) => Value::Array(v.iter().map(|i| Value::from(*i)).collect()),
        Tag::Compound(map) => Value::Object(
            map.iter()
                .map(|(k, v)| (k.to_string_lossy(), tag_json(v)))
                .collect(),
        ),
        Tag::End => Value::Null,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::worldgen::structure::stronghold::{ring_positions, ring_searches};

    #[test]
    fn structure_biome_plain_lists_and_scalar_fields_keep_their_leniency() {
        struct Biomes;
        impl crate::holder_set::HolderLookup for Biomes {
            type Id = BiomeId;
            fn what(&self) -> &str {
                "biome"
            }
            fn entry(&self, id: &str) -> Result<Vec<BiomeId>, String> {
                Ok((id == "minecraft:plains")
                    .then_some(BiomeId(0))
                    .into_iter()
                    .collect())
            }
            fn tag(&self, id: &str) -> Result<Vec<BiomeId>, String> {
                self.entry(id)
            }
            fn all(&self) -> Result<Vec<BiomeId>, String> {
                Ok(vec![BiomeId(0), BiomeId(1)])
            }
        }
        assert_eq!(
            biome_set(
                &Biomes,
                &serde_json::json!(["#minecraft:plains", false, 19, {}])
            )
            .unwrap(),
            BTreeSet::from([BiomeId(0)])
        );
        for value in [
            serde_json::json!(null),
            serde_json::json!(19),
            serde_json::json!(false),
        ] {
            assert!(biome_set(&Biomes, &value).unwrap().is_empty());
        }
        assert_eq!(
            biome_set(&Biomes, &serde_json::json!({"type":"neoforge:any"})).unwrap(),
            BTreeSet::from([BiomeId(0), BiomeId(1)])
        );
    }

    /// The old-chunk path (`StructureState::saved_starts_at`) and the saved-chunk path
    /// (`read_saved_start`) are `StructureStart.loadStaticStart` for a monument too: both
    /// drop the unreadable first child, anchor on the first readable `omb`, and rebuild
    /// the same rooms from the seed. The 30-room expectation pinned from vanilla 26.3 is
    /// in `monument`'s saved-monument test; this one drives both Horizon paths over the
    /// same tag and compares the whole rebuilt building.
    #[test]
    fn old_chunk_and_saved_chunk_monuments_read_the_same_first_piece() {
        use crate::heightmap::HeightmapType;
        use crate::registry::BlockRegistry;
        use crate::worldgen::anchor::WorldGenerationContext;
        use crate::worldgen::{
            blending::SavedStart,
            structure::{
                jigsaw::JigsawContext, mod_types::mod_structure_types, start::NoiseColumnTarget,
            },
        };
        use horizon_core::nbt::Tag;

        let def = r#"{"type":"minecraft:ocean_monument"}"#.to_owned();
        let registry = StructureRegistry::new(
            &BTreeMap::from([("minecraft:monument".to_owned(), def)]),
            &BTreeMap::new(),
            BTreeMap::new(),
            &|_| Ok(BTreeSet::new()),
            mod_structure_types(),
        )
        .unwrap();
        let (mut tag, _) = horizon_core::nbt::parse_snbt(
            r#"{DataVersion:0,states:[
            {id:"minecraft:air"},{id:"minecraft:cave_air"},{id:"minecraft:void_air"},
            {id:"minecraft:water",properties:{level:"0"}},
            {id:"minecraft:prismarine"},
            {id:"minecraft:prismarine_bricks"},{id:"minecraft:dark_prismarine"},
            {id:"minecraft:sea_lantern"}],defaults:{
            "minecraft:air":0,"minecraft:cave_air":1,"minecraft:void_air":2,
            "minecraft:water":3,"minecraft:prismarine":4,
            "minecraft:prismarine_bricks":5,"minecraft:dark_prismarine":6,
            "minecraft:sea_lantern":7}}"#,
        )
        .unwrap();
        *tag.get_mut("DataVersion").unwrap() = Tag::Int(crate::DATA_VERSION);
        let blocks: &'static _ = Box::leak(Box::new(BlockRegistry::from_nbt(&tag).unwrap()));
        let resources = StructureResources::new(
            blocks,
            crate::worldgen::structure::StateTransforms::default(),
            std::path::Path::new("unused"),
            BTreeMap::new(),
            &BTreeMap::new(),
        );
        let bindings = FeatureBlockBindings::new(blocks, BTreeSet::new()).unwrap();
        let chunk = ChunkPos::new(-2, 3);
        let seed = 12345;
        let children = vec![
            Tag::compound([
                ("id", Tag::String("minecraft:future_room".into())),
                ("BB", Tag::IntArray(vec![0, 0, 0, 1, 1, 1])),
                ("O", Tag::Int(0)),
            ]),
            Tag::compound([
                ("id", Tag::String("minecraft:omb".into())),
                ("BB", Tag::IntArray(vec![100, 20, -300, 157, 42, -243])),
                ("O", Tag::Int(1)),
            ]),
            Tag::compound([
                ("id", Tag::String("minecraft:omb".into())),
                ("BB", Tag::IntArray(vec![400, 50, 400, 457, 72, 457])),
                ("O", Tag::Int(3)),
            ]),
        ];
        let saved = Tag::compound([
            ("id", Tag::String("minecraft:monument".into())),
            ("ChunkX", Tag::Int(chunk.x)),
            ("ChunkZ", Tag::Int(chunk.z)),
            ("references", Tag::Int(1)),
            ("Children", Tag::List(children.clone())),
        ]);
        let state = StructureState {
            registry,
            resources: Arc::new(resources),
            bindings,
            possible_sets: Vec::new(),
            by_step: vec![Vec::new(); crate::worldgen::structure::start::STEPS.len()],
            starts: StartCache::new(),
            saved: OnceLock::new(),
            start_work: ChunkWork::new(),
            rings: Arc::new(OnceLock::new()),
            ring_work: OnceLock::new(),
            ring_seed: None,
            fixed_biome: false,
            zone_reach: 0,
            biome_names: crate::worldgen::structure_terrain::BiomeNames::new(&[]),
        };
        let mut context = SavedChunk(vec![SavedStart {
            name: "minecraft:monument".into(),
            bounds: None,
            pieces: children,
        }]);
        let old = state.saved_starts_at(&mut context, chunk);
        let new = read_saved_start(
            0,
            &state.registry.structures[0],
            chunk,
            &saved,
            seed,
            &state.resources,
            &state.bindings,
        )
        .expect("both paths load the first readable piece");
        assert_eq!(old.len(), 1);
        assert_eq!(old[0].bounding_box, new.bounding_box);
        assert_eq!(
            format!("{:?}", old[0].pieces[0]),
            format!("{:?}", new.pieces[0]),
            "the same anchor, orientation and regenerated rooms"
        );

        struct SavedChunk(Vec<SavedStart>);
        impl JigsawContext for SavedChunk {
            fn min_y(&self) -> i32 {
                -64
            }
            fn max_y(&self) -> i32 {
                320
            }
            fn base_height(&mut self, _x: i32, _z: i32, _kind: HeightmapType) -> i32 {
                63
            }
            fn column_has_valid_biome(
                &mut self,
                _x: i32,
                _z: i32,
                _min_y: i32,
                _max_y: i32,
            ) -> bool {
                true
            }
        }
        impl StartContext for SavedChunk {
            fn is_old_chunk(&self, _chunk: ChunkPos) -> bool {
                true
            }
            fn saved_starts(&self, _chunk: ChunkPos) -> Vec<SavedStart> {
                self.0.clone()
            }
            fn seed(&self) -> i64 {
                12345
            }
            fn sea_level(&self) -> i32 {
                63
            }
            fn base_column(&mut self, _x: i32, _z: i32, _kind: HeightmapType) -> Vec<bool> {
                unimplemented!("not on the monument path")
            }
            fn base_columns_by(
                &mut self,
                _x: i32,
                _z: i32,
                _count: usize,
                _kind: &dyn Fn(usize) -> HeightmapType,
            ) -> Vec<Vec<bool>> {
                unimplemented!("not on the monument path")
            }
            fn noise_column_into(&mut self, _x: i32, _z: i32, _column: &mut dyn NoiseColumnTarget) {
                unimplemented!("not on the monument path")
            }
            fn cold_enough_to_snow(&mut self, _biome: BiomeId, _pos: [i32; 3]) -> bool {
                unimplemented!("not on the monument path")
            }
            fn generation(&self) -> WorldGenerationContext {
                unimplemented!("not on the monument path")
            }
            fn noise_biome(&mut self, _quart: [i32; 3]) -> BiomeId {
                unimplemented!("not on the monument path")
            }
            fn noise_biomes(&mut self, _quarts: &[[i32; 3]]) -> Vec<BiomeId> {
                unimplemented!("not on the monument path")
            }
            fn set_valid_biomes(&mut self, _biomes: Option<&BTreeSet<BiomeId>>) {
                unimplemented!("not on the monument path")
            }
        }
    }

    /// A provider that starts answering `NoStructures` (a mod update, a changed setting) leaves
    /// the starts a chunk already saved alone: the neighbours still get the pieces that reach
    /// into it. A chunk with none saved gets no start.
    #[test]
    fn no_structures_keeps_the_starts_a_chunk_saved() {
        use crate::heightmap::HeightmapType;
        use crate::registry::BlockRegistry;
        use crate::worldgen::anchor::WorldGenerationContext;
        use crate::worldgen::structure::{
            StateTransforms, jigsaw::JigsawContext, mod_types::mod_structure_types,
            start::NoiseColumnTarget,
        };
        use crate::worldgen::structure_terrain::{Provider, register, unregister};
        use horizon_core::nbt::Tag;

        const SEED: i64 = 5_117_204_463;
        struct Silent;
        impl Provider for Silent {
            fn answer(&self, request: &Request<'_>) -> Answer {
                if request.seed() == SEED {
                    Answer::NoStructures
                } else {
                    Answer::Vanilla
                }
            }
        }
        struct Context;
        impl JigsawContext for Context {
            fn min_y(&self) -> i32 {
                -64
            }
            fn max_y(&self) -> i32 {
                320
            }
            fn base_height(&mut self, _: i32, _: i32, _: HeightmapType) -> i32 {
                63
            }
            fn column_has_valid_biome(&mut self, _: i32, _: i32, _: i32, _: i32) -> bool {
                true
            }
        }
        impl StartContext for Context {
            fn seed(&self) -> i64 {
                SEED
            }
            fn sea_level(&self) -> i32 {
                63
            }
            fn base_column(&mut self, _: i32, _: i32, _: HeightmapType) -> Vec<bool> {
                unimplemented!("no start is built")
            }
            fn base_columns_by(
                &mut self,
                _: i32,
                _: i32,
                _: usize,
                _: &dyn Fn(usize) -> HeightmapType,
            ) -> Vec<Vec<bool>> {
                unimplemented!("no start is built")
            }
            fn noise_column_into(&mut self, _: i32, _: i32, _: &mut dyn NoiseColumnTarget) {
                unimplemented!("no start is built")
            }
            fn cold_enough_to_snow(&mut self, _: BiomeId, _: [i32; 3]) -> bool {
                unimplemented!("no start is built")
            }
            fn generation(&self) -> WorldGenerationContext {
                WorldGenerationContext::new(-64, 384, 63)
            }
            fn noise_biome(&mut self, _: [i32; 3]) -> BiomeId {
                unimplemented!("no start is built")
            }
            fn noise_biomes(&mut self, _: &[[i32; 3]]) -> Vec<BiomeId> {
                unimplemented!("no start is built")
            }
            fn set_valid_biomes(&mut self, _: Option<&BTreeSet<BiomeId>>) {}
        }

        let def = r#"{"type":"minecraft:ocean_monument"}"#.to_owned();
        let registry = StructureRegistry::new(
            &BTreeMap::from([("minecraft:monument".to_owned(), def)]),
            &BTreeMap::new(),
            BTreeMap::new(),
            &|_| Ok(BTreeSet::new()),
            mod_structure_types(),
        )
        .unwrap();
        let (mut tag, _) = horizon_core::nbt::parse_snbt(
            r#"{DataVersion:0,states:[{id:"minecraft:air"},{id:"minecraft:cave_air"},
            {id:"minecraft:void_air"},{id:"minecraft:water",properties:{level:"0"}},
            {id:"minecraft:prismarine"},{id:"minecraft:prismarine_bricks"},
            {id:"minecraft:dark_prismarine"},{id:"minecraft:sea_lantern"}],defaults:{
            "minecraft:air":0,"minecraft:cave_air":1,"minecraft:void_air":2,
            "minecraft:water":3,"minecraft:prismarine":4,
            "minecraft:prismarine_bricks":5,"minecraft:dark_prismarine":6,
            "minecraft:sea_lantern":7}}"#,
        )
        .unwrap();
        *tag.get_mut("DataVersion").unwrap() = Tag::Int(crate::DATA_VERSION);
        let blocks: &'static _ = Box::leak(Box::new(BlockRegistry::from_nbt(&tag).unwrap()));
        let resources = StructureResources::new(
            blocks,
            StateTransforms::default(),
            std::path::Path::new("unused"),
            BTreeMap::new(),
            &BTreeMap::new(),
        );
        let bindings = FeatureBlockBindings::new(blocks, BTreeSet::new()).unwrap();
        let (saved_chunk, empty_chunk) = (ChunkPos::new(-2, 3), ChunkPos::new(5, 5));
        let monument = Tag::compound([
            ("id", Tag::String("minecraft:monument".into())),
            ("ChunkX", Tag::Int(saved_chunk.x)),
            ("ChunkZ", Tag::Int(saved_chunk.z)),
            ("references", Tag::Int(1)),
            (
                "Children",
                Tag::List(vec![Tag::compound([
                    ("id", Tag::String("minecraft:omb".into())),
                    ("BB", Tag::IntArray(vec![100, 20, -300, 157, 42, -243])),
                    ("O", Tag::Int(1)),
                ])]),
            ),
        ]);
        let Tag::Compound(on_disk) = Tag::compound([("minecraft:monument", monument)]) else {
            unreachable!()
        };
        let state = StructureState {
            registry,
            resources: Arc::new(resources),
            bindings,
            possible_sets: Vec::new(),
            by_step: vec![Vec::new(); crate::worldgen::structure::start::STEPS.len()],
            starts: StartCache::new(),
            saved: OnceLock::new(),
            start_work: ChunkWork::new(),
            rings: Arc::new(OnceLock::new()),
            ring_work: OnceLock::new(),
            ring_seed: None,
            fixed_biome: false,
            zone_reach: 0,
            biome_names: crate::worldgen::structure_terrain::BiomeNames::new(&[]),
        };
        state.set_saved_starts(Arc::new(move |chunk| {
            (chunk == saved_chunk).then(|| on_disk.clone())
        }));

        register("test:silent-over-saved-starts", Arc::new(Silent));
        let kept = state.build_starts(&mut Context, saved_chunk, true).unwrap();
        let none = state.build_starts(&mut Context, empty_chunk, true).unwrap();
        unregister("test:silent-over-saved-starts");
        assert_eq!(kept.len(), 1, "the saved monument survives NoStructures");
        assert_eq!(kept[0].name, "minecraft:monument");
        assert!(none.is_empty(), "a chunk with nothing saved gets no start");
    }

    #[test]
    fn saved_jigsaw_round_trip_and_whole_start_fallback() {
        use crate::{
            registry::BlockRegistry,
            worldgen::structure::{
                Rotation, StateTransforms,
                jigsaw::{Junction, PoolElement, Projection},
                start::StructureKind,
            },
        };
        use horizon_core::{bounding_box::BoundingBox, density::TerrainAdjustment, nbt::Tag};
        let names = ["minecraft:air", "minecraft:cave_air", "minecraft:void_air"];
        let registry = Box::leak(Box::new(
            BlockRegistry::from_nbt(&Tag::compound([
                ("DataVersion", Tag::Int(crate::DATA_VERSION)),
                (
                    "states",
                    Tag::List(
                        names
                            .iter()
                            .map(|n| Tag::compound([("id", Tag::String((*n).into()))]))
                            .collect(),
                    ),
                ),
                (
                    "defaults",
                    Tag::Compound(
                        names
                            .iter()
                            .enumerate()
                            .map(|(i, n)| ((*n).into(), Tag::Int(i as i32)))
                            .collect(),
                    ),
                ),
            ]))
            .unwrap(),
        ));
        let transforms = StateTransforms::from_json(r#"{"rotate":[[0,0,0],[1,1,1],[2,2,2]],"mirror":[[0,0],[1,1],[2,2]],"full_block":[false,false,false]}"#, 3).unwrap();
        let resources = StructureResources::new(
            registry,
            transforms,
            std::path::Path::new("unused"),
            BTreeMap::new(),
            &BTreeMap::new(),
        );
        let bindings = FeatureBlockBindings::new(registry, BTreeSet::new()).unwrap();
        let def = StructureDef {
            name: "test:jigsaw".into(),
            kind: StructureKind::Unsupported("test".into()),
            biomes: BTreeSet::new(),
            step: 0,
            terrain: TerrainAdjustment::BeardThin,
            enhanced_terrain: None,
        };
        let chunk = ChunkPos::new(-2, 3);
        let pieces = Rotation::ALL
            .into_iter()
            .enumerate()
            .map(|(i, rotation)| {
                Piece::Pool(PoolPiece {
                    element: Arc::new(PoolElement::Feature {
                        feature: "test:tree".into(),
                        projection: Projection::TerrainMatching,
                    }),
                    position: [-32 + i as i32, 60, 48],
                    ground_level_delta: i as i32 - 2,
                    rotation,
                    bounding_box: BoundingBox::new([-32, 60, 48], [-20, 75, 62]),
                    junctions: vec![Junction {
                        source_x: -28,
                        source_ground_y: 61,
                        source_z: 50,
                        delta_y: -3,
                        dest_projection: Projection::Rigid,
                    }],
                    waterlogging: i % 2 == 0,
                })
            })
            .collect();
        let mut original = StructureStart::new(7, &def, chunk, pieces);
        original.references = 3;
        let tag = original.save().unwrap();
        let read = |tag: &Tag| read_saved_start(7, &def, chunk, tag, 1234, &resources, &bindings);
        let loaded = read(&tag).expect("complete jigsaw start");
        assert_eq!(loaded.save(), Some(tag.clone()));
        assert_eq!(
            format!("{loaded:?}"),
            format!("{original:?}"),
            "every runtime field"
        );
        for id in [
            "nvi",
            "minecraft:pcp",
            "minecraft:bastionremnant",
            "minecraft:runtime",
            "jigsaw",
        ] {
            let mut legacy = tag.clone();
            let Tag::Compound(start) = &mut legacy else {
                unreachable!()
            };
            let Some(Tag::List(children)) = start.get_mut(&"Children".into()) else {
                unreachable!()
            };
            let Tag::Compound(piece) = &mut children[0] else {
                unreachable!()
            };
            piece.insert("id".into(), Tag::String(id.into()));
            assert_eq!(read(&legacy).unwrap().save(), Some(tag.clone()));
        }
        for (key, value) in [
            ("id", Tag::String("minecraft:not_a_piece".into())),
            ("rotation", Tag::String("broken".into())),
            ("O", Tag::Int(2)),
            ("GD", Tag::Int(1)),
        ] {
            let mut broken = tag.clone();
            let Tag::Compound(start) = &mut broken else {
                unreachable!()
            };
            let Some(Tag::List(children)) = start.get_mut(&"Children".into()) else {
                unreachable!()
            };
            let Tag::Compound(piece) = &mut children[1] else {
                unreachable!()
            };
            piece.insert(key.into(), value);
            assert!(
                read(&broken).is_none(),
                "fallback for the entire start: {key}"
            );
        }
        assert!(read(&Tag::compound([("id", Tag::String("INVALID".into()))])).is_none());
    }

    /// The saved pieces of a desert pyramid, a jungle temple and a buried
    /// treasure read back with their state instead of leaving the whole start
    /// to the generator's rebuild.
    #[test]
    fn saved_scattered_pieces_and_a_buried_treasure_load_with_their_state() {
        use crate::{
            registry::BlockRegistry,
            worldgen::structure::{StateTransforms, saved_legacy, start::StructureKind},
        };
        use horizon_core::{density::TerrainAdjustment, nbt::Tag};
        let names = ["minecraft:air", "minecraft:cave_air", "minecraft:void_air"];
        let registry = Box::leak(Box::new(
            BlockRegistry::from_nbt(&Tag::compound([
                ("DataVersion", Tag::Int(crate::DATA_VERSION)),
                (
                    "states",
                    Tag::List(
                        names
                            .iter()
                            .map(|n| Tag::compound([("id", Tag::String((*n).into()))]))
                            .collect(),
                    ),
                ),
                (
                    "defaults",
                    Tag::Compound(
                        names
                            .iter()
                            .enumerate()
                            .map(|(i, n)| ((*n).into(), Tag::Int(i as i32)))
                            .collect(),
                    ),
                ),
            ]))
            .unwrap(),
        ));
        let transforms = StateTransforms::from_json(
            r#"{"rotate":[[0,0,0],[1,1,1],[2,2,2]],"mirror":[[0,0],[1,1],[2,2]],"full_block":[false,false,false]}"#,
            3,
        )
        .unwrap();
        let resources = StructureResources::new(
            registry,
            transforms,
            std::path::Path::new("unused"),
            BTreeMap::new(),
            &BTreeMap::new(),
        );
        let bindings = FeatureBlockBindings::new(registry, BTreeSet::new()).unwrap();
        let def = StructureDef {
            name: "test:saved-pieces".into(),
            kind: StructureKind::Unsupported("test".into()),
            biomes: BTreeSet::new(),
            step: 0,
            terrain: TerrainAdjustment::None,
            enhanced_terrain: None,
        };
        let piece = |id: &str, at: [i32; 3], extra: Vec<(&str, Tag)>| {
            let mut entries = vec![
                ("id", Tag::String(id.into())),
                (
                    "BB",
                    Tag::IntArray(vec![at[0], at[1], at[2], at[0], at[1], at[2]]),
                ),
                ("O", Tag::Int(-1)),
                ("GD", Tag::Int(0)),
            ];
            entries.extend(extra);
            Tag::compound(entries)
        };
        let tedp = piece(
            "minecraft:tedp",
            [3, 64, 5],
            vec![
                ("Width", Tag::Int(21)),
                ("Height", Tag::Int(15)),
                ("Depth", Tag::Int(21)),
                ("HPos", Tag::Int(62)),
                ("hasPlacedChest0", Tag::Byte(1)),
                ("hasPlacedChest1", Tag::Byte(0)),
                ("hasPlacedChest2", Tag::Byte(0)),
                ("hasPlacedChest3", Tag::Byte(1)),
            ],
        );
        let tejp = piece(
            "minecraft:tejp",
            [40, 64, 5],
            vec![
                ("Width", Tag::Int(12)),
                ("Height", Tag::Int(10)),
                ("Depth", Tag::Int(15)),
                ("HPos", Tag::Int(61)),
                ("placedMainChest", Tag::Byte(1)),
                ("placedHiddenChest", Tag::Byte(0)),
                ("placedTrap1", Tag::Byte(0)),
                ("placedTrap2", Tag::Byte(1)),
            ],
        );
        let btp = piece("minecraft:btp", [80, 40, 5], vec![]);
        for (id, tag) in [("tedp", &tedp), ("tejp", &tejp), ("btp", &btp)] {
            let piece = saved_legacy::read(id, tag, &def)
                .unwrap_or_else(|| panic!("{id} binds its tag constructor"));
            let Tag::Compound(compound) = tag else {
                unreachable!()
            };
            assert_eq!(piece.save(), Some(compound.clone()), "{id}");
        }
        // The same tags inside a saved start now give pieces instead of the
        // generator's rebuild (or an empty start).
        let start = Tag::compound([
            ("id", Tag::String(def.name.as_str().into())),
            ("ChunkX", Tag::Int(0)),
            ("ChunkZ", Tag::Int(0)),
            ("BB", Tag::IntArray(vec![0, 30, 0, 90, 70, 20])),
            ("Children", Tag::List(vec![tedp.clone(), tejp, btp])),
        ]);
        let loaded = read_saved_start(
            0,
            &def,
            ChunkPos::new(0, 0),
            &start,
            1234,
            &resources,
            &bindings,
        )
        .expect("the saved pieces load");
        assert_eq!(loaded.pieces.len(), 3);
        let Tag::Compound(tedp) = tedp else {
            unreachable!()
        };
        assert_eq!(loaded.pieces[0].save(), Some(tedp));
    }

    /// A template piece a save wrote (the shipwreck and ocean ruin families write
    /// these) loads an old chunk's start with the same `saved_templates::read`
    /// a newly loaded start uses, instead of leaving the whole start to the
    /// generator's rebuild.
    #[test]
    fn an_old_chunks_saved_template_pieces_load_with_their_state() {
        use crate::{
            registry::BlockRegistry,
            worldgen::structure::{
                Rotation, StateTransforms,
                legacy::LegacyPiece,
                ocean_ruin::{self, OceanRuinConfig},
                simple::shipwreck_piece,
                start::StructureKind,
            },
        };
        use horizon_core::{density::TerrainAdjustment, nbt::Tag, random::XoroshiroRandom};
        let names = [
            "minecraft:air",
            "minecraft:cave_air",
            "minecraft:void_air",
            "minecraft:structure_block",
            "minecraft:sand",
            "minecraft:suspicious_sand",
            "minecraft:gravel",
            "minecraft:suspicious_gravel",
        ];
        let registry = Box::leak(Box::new(
            BlockRegistry::from_nbt(&Tag::compound([
                ("DataVersion", Tag::Int(crate::DATA_VERSION)),
                (
                    "states",
                    Tag::List(
                        names
                            .iter()
                            .map(|n| Tag::compound([("id", Tag::String((*n).into()))]))
                            .collect(),
                    ),
                ),
                (
                    "defaults",
                    Tag::Compound(
                        names
                            .iter()
                            .enumerate()
                            .map(|(i, n)| ((*n).into(), Tag::Int(i as i32)))
                            .collect(),
                    ),
                ),
            ]))
            .unwrap(),
        ));
        let resources = StructureResources::new(
            registry,
            StateTransforms::default(),
            std::path::Path::new("unused"),
            BTreeMap::new(),
            &BTreeMap::new(),
        );
        let bindings = FeatureBlockBindings::new(registry, BTreeSet::new()).unwrap();
        let def = |name: &str, kind| StructureDef {
            name: name.into(),
            kind,
            biomes: BTreeSet::new(),
            step: 0,
            terrain: TerrainAdjustment::None,
            enhanced_terrain: None,
        };
        let mut random = XoroshiroRandom::new(1234);
        let wreck = shipwreck_piece(
            &resources,
            [16, 60, 32],
            Rotation::Clockwise90,
            &mut random,
            true,
        )
        .expect("a beached shipwreck");
        let tag = Tag::Compound(wreck.save().expect("a savable shipwreck"));
        let loaded = StructureState::saved_piece(
            &tag,
            &def(
                "minecraft:shipwreck",
                StructureKind::Shipwreck { beached: true },
            ),
            &resources,
            &bindings,
        )
        .expect("the saved shipwreck loads instead of regenerating");
        assert_eq!(loaded.save(), wreck.save());
        assert_eq!(
            format!("{loaded:?}"),
            format!("{:?}", Piece::Legacy(Box::new(wreck)))
        );

        let config = OceanRuinConfig {
            warm: true,
            large_probability: 0.0,
            cluster_probability: 0.0,
        };
        let ruin = ocean_ruin::pieces(
            &resources,
            &bindings,
            [40, 45, 32],
            Rotation::None,
            &mut random,
            &config,
        )
        .expect("a warm ocean ruin")
        .remove(0);
        let tag = Tag::Compound(ruin.save().expect("a savable ocean ruin"));
        let loaded = StructureState::saved_piece(
            &tag,
            &def(
                "minecraft:ocean_ruin",
                StructureKind::OceanRuin {
                    warm: true,
                    large: 0.0,
                    cluster: 0.0,
                },
            ),
            &resources,
            &bindings,
        )
        .expect("the saved ocean ruin loads instead of regenerating");
        assert_eq!(loaded.save(), ruin.save());
    }

    /// `PiecesContainer.load` reads a piece whose id is one of the four names an
    /// older save wrote for a jigsaw piece as a jigsaw piece (its `RENAMES`
    /// table); any other piece id keeps the start on the generator's rebuild.
    #[test]
    fn an_old_saves_jigsaw_piece_ids_are_read_as_jigsaw() {
        let piece = |id: &str| {
            horizon_core::nbt::Tag::compound([
                ("id", horizon_core::nbt::Tag::String(id.into())),
                ("PosX", horizon_core::nbt::Tag::Int(16)),
            ])
        };
        for id in [
            "minecraft:nvi",
            "minecraft:pcp",
            "minecraft:bastionremnant",
            "minecraft:runtime",
            "nvi",
        ] {
            assert_eq!(
                saved_piece_id(&piece(id)),
                Some("minecraft:jigsaw"),
                "{id} is a renamed jigsaw piece"
            );
        }
        assert_eq!(
            saved_piece_id(&piece("minecraft:jigsaw")),
            Some("minecraft:jigsaw")
        );
        // The monument, stronghold and jungle temple pieces are their own types: a
        // pool piece's reader would not decode them.
        for id in ["minecraft:OMB", "minecraft:SHCC", "minecraft:TeJP"] {
            assert_eq!(saved_piece_id(&piece(id)), None, "{id} is not a pool piece");
        }
        // No id, or not a compound at all: nothing to read.
        assert_eq!(
            saved_piece_id(&horizon_core::nbt::Tag::compound([(
                "PosX",
                horizon_core::nbt::Tag::Int(16)
            )])),
            None
        );
        assert_eq!(saved_piece_id(&horizon_core::nbt::Tag::Int(1)), None);
    }

    /// The start cache keeps the first starts built for a chunk, spreads a
    /// chunk's 289 reference lookups over the shards, and forgets on clear.
    #[test]
    fn start_cache_keeps_first_and_spreads_neighbours() {
        let cache = StartCache::new();
        let first: Starts = Arc::new(Vec::new());
        let second: Starts = Arc::new(Vec::new());
        assert!(cache.get((3, -4)).is_none());
        assert!(Arc::ptr_eq(
            &cache.insert_first((3, -4), first.clone()),
            &first
        ));
        assert!(Arc::ptr_eq(&cache.insert_first((3, -4), second), &first));
        assert!(Arc::ptr_eq(&cache.get((3, -4)).unwrap(), &first));
        for (cx, cz) in [(0, 0), (-1000, 77), (i32::MAX - 8, i32::MIN + 8)] {
            let mut used = vec![0; START_SHARDS];
            for x in cx - 8..=cx + 8 {
                for z in cz - 8..=cz + 8 {
                    let shard = cache.shard((x, z)) as *const _;
                    let i = cache
                        .shards
                        .iter()
                        .position(|s| std::ptr::eq(s, shard))
                        .unwrap();
                    used[i] += 1;
                }
            }
            assert!(used.iter().all(|&n| n > 0 && n < 20), "{used:?}");
        }
        cache.clear();
        assert!(cache.get((3, -4)).is_none());
    }

    /// A full shard forgets the starts nobody asked for since it last filled,
    /// and keeps the ones still asked for (a player's surroundings).
    #[test]
    fn start_cache_keeps_the_starts_still_asked_for() {
        let cache = StartCache::new();
        let hot: Starts = Arc::new(Vec::new());
        let cold: Starts = Arc::new(Vec::new());
        let shard = shard_of((0, 0), START_SHARDS);
        let mut same_shard = (1..)
            .map(|x| (x, 0))
            .filter(|&k| shard_of(k, START_SHARDS) == shard);
        let cold_key = same_shard.next().unwrap();
        cache.insert_first((0, 0), hot.clone());
        cache.insert_first(cold_key, cold);
        // Three shards' worth of other chunks, with the hot one asked for now and then.
        for (i, key) in same_shard
            .take(3 * START_CACHE_LIMIT / START_SHARDS)
            .enumerate()
        {
            cache.insert_first(key, Arc::new(Vec::new()));
            if i % 100 == 0 {
                assert!(Arc::ptr_eq(&cache.get((0, 0)).unwrap(), &hot));
            }
        }
        assert!(Arc::ptr_eq(&cache.get((0, 0)).unwrap(), &hot));
        assert!(cache.get(cold_key).is_none());
    }

    #[test]
    fn start_cache_counts_both_generations_and_replaces_old_starts() {
        let cache = StartCache::new();
        let key = (3, -4);
        let first: Starts = Arc::new(Vec::new());
        cache.insert_first(key, first.clone());
        {
            let mut shard = cache.shard(key).write().unwrap();
            shard.old = std::mem::take(&mut shard.new);
        }
        cache.insert_first((99, 100), Arc::new(Vec::new()));
        assert_eq!(cache.census(), (2, 0));
        assert!(Arc::ptr_eq(
            &cache.insert_first(key, Arc::new(Vec::new())),
            &first
        ));
        {
            let mut shard = cache.shard(key).write().unwrap();
            let value = shard.new.remove(&key).unwrap();
            shard.old.insert(key, value);
        }
        let replacement: Starts = Arc::new(Vec::new());
        cache.replace(key, replacement.clone());
        assert!(Arc::ptr_eq(&cache.get(key).unwrap(), &replacement));
        assert_eq!(cache.census(), (2, 0));
    }

    /// A ring search ends where `may_land_near` allows: on its ring chunk
    /// or in the square it samples, never further.
    #[test]
    fn ring_searches_end_where_they_may() {
        let none = BTreeSet::new();
        for seed in [0, 1234, -77, i64::MAX] {
            for (b, search) in ring_searches(seed, 32, 3, 128).iter().enumerate() {
                // Preferred biomes on one quart at a time, in turn at each
                // corner of the square, else nowhere (the ring chunk).
                let corner = (b % 5) as i32;
                let biomes = |quarts: &[[i32; 3]]| -> Vec<BiomeId> {
                    let (first, last) = (quarts[0], quarts[quarts.len() - 1]);
                    let pick = match corner {
                        0 => first,
                        1 => [last[0], 0, first[2]],
                        2 => [first[0], 0, last[2]],
                        3 => last,
                        _ => [i32::MIN, 0, i32::MIN],
                    };
                    quarts
                        .iter()
                        .map(|q| BiomeId(u32::from(*q == pick)))
                        .collect()
                };
                let preferred: BTreeSet<BiomeId> = [BiomeId(1)].into();
                for wanted in [&preferred, &none] {
                    let found = search.find(wanted, &mut { biomes });
                    assert!(search.may_land_near(found.x, found.z, 0));
                    assert!(!search.may_land_near(found.x + 17, found.z, 0));
                }
            }
        }
    }

    /// Searches shared between threads give the positions of one thread
    /// running them in order, set by set.
    #[test]
    fn ring_work_shared_between_threads_matches_serial_search() {
        let preferred: BTreeSet<BiomeId> = (0..20).map(|b| BiomeId(b * 3)).collect();
        let biomes = |quarts: &[[i32; 3]]| -> Vec<BiomeId> {
            quarts
                .iter()
                .map(|q| {
                    BiomeId((q[0].wrapping_mul(31) ^ q[2].wrapping_mul(17)).rem_euclid(64) as u32)
                })
                .collect()
        };
        let none = BTreeSet::new();
        for seed in [0, 1234, -77, i64::MAX] {
            let work = RingWork::new(vec![
                (0, ring_searches(seed, 32, 3, 128)),
                (1, ring_searches(seed ^ 5, 10, 4, 0)),
                (2, ring_searches(seed.wrapping_add(1), 20, 5, 40)),
            ]);
            let sets = [&preferred, &none, &preferred];
            std::thread::scope(|scope| {
                for _ in 0..4 {
                    scope.spawn(|| work.help(|set| sets[set], &mut { biomes }));
                }
            });
            let rings = work.rings(|set| format!("set{set}"));
            let serial = |s: i64, distance, spread, count, preferred| {
                ring_positions(s, distance, spread, count, preferred, &mut { biomes })
            };
            assert_eq!(rings["set0"], serial(seed, 32, 3, 128, &preferred));
            assert_eq!(rings["set1"], Vec::new());
            assert_eq!(
                rings["set2"],
                serial(seed.wrapping_add(1), 20, 5, 40, &preferred)
            );
        }
    }

    /// When one or more chunks in the 17×17 reference window are being built by
    /// another worker, the two-pass `deferred` path emits starts in the exact
    /// same grid order as the uncontended single-pass path (before the
    /// `LongOpenHashSet` per-structure reorder step runs).
    #[test]
    fn references_preserves_grid_order_across_busy_and_deferred_chunks() {
        use crate::heightmap::HeightmapType;
        use crate::registry::BlockRegistry;
        use crate::worldgen::anchor::WorldGenerationContext;
        use crate::worldgen::structure::{
            StateTransforms, jigsaw::JigsawContext, mod_types::mod_structure_types,
            start::NoiseColumnTarget,
        };
        use horizon_core::{bounding_box::BoundingBox, nbt::Tag};
        use std::sync::mpsc;

        struct ReleaseOnTrigger {
            trigger: ChunkPos,
            queried: Mutex<Vec<ChunkPos>>,
            release: Mutex<Option<mpsc::Sender<()>>>,
        }
        impl JigsawContext for ReleaseOnTrigger {
            fn min_y(&self) -> i32 {
                -64
            }
            fn max_y(&self) -> i32 {
                320
            }
            fn base_height(&mut self, _x: i32, _z: i32, _kind: HeightmapType) -> i32 {
                63
            }
            fn column_has_valid_biome(
                &mut self,
                _x: i32,
                _z: i32,
                _min_y: i32,
                _max_y: i32,
            ) -> bool {
                true
            }
        }
        impl StartContext for ReleaseOnTrigger {
            fn is_old_chunk(&self, chunk: ChunkPos) -> bool {
                self.queried.lock().unwrap().push(chunk);
                if chunk == self.trigger
                    && let Some(tx) = self.release.lock().unwrap().take()
                {
                    let _ = tx.send(());
                }
                false
            }
            fn seed(&self) -> i64 {
                12345
            }
            fn sea_level(&self) -> i32 {
                63
            }
            fn base_column(&mut self, _x: i32, _z: i32, _kind: HeightmapType) -> Vec<bool> {
                unimplemented!("not called when possible_sets is empty")
            }
            fn base_columns_by(
                &mut self,
                _x: i32,
                _z: i32,
                _count: usize,
                _kind: &dyn Fn(usize) -> HeightmapType,
            ) -> Vec<Vec<bool>> {
                unimplemented!("not called when possible_sets is empty")
            }
            fn noise_column_into(&mut self, _x: i32, _z: i32, _column: &mut dyn NoiseColumnTarget) {
                unimplemented!("not called when possible_sets is empty")
            }
            fn cold_enough_to_snow(&mut self, _biome: BiomeId, _pos: [i32; 3]) -> bool {
                unimplemented!("not called when possible_sets is empty")
            }
            fn generation(&self) -> WorldGenerationContext {
                WorldGenerationContext::new(-64, 384, 63)
            }
            fn noise_biome(&mut self, _quart: [i32; 3]) -> BiomeId {
                unimplemented!("not called when possible_sets is empty")
            }
            fn noise_biomes(&mut self, _quarts: &[[i32; 3]]) -> Vec<BiomeId> {
                unimplemented!("not called when possible_sets is empty")
            }
            fn set_valid_biomes(&mut self, _biomes: Option<&BTreeSet<BiomeId>>) {
                unimplemented!("not called when possible_sets is empty")
            }
        }

        type CaseEntry<'a> = (ChunkPos, &'a [(usize, BoundingBox)]);

        let names = ["minecraft:air", "minecraft:cave_air", "minecraft:void_air"];
        let blocks: &'static BlockRegistry = Box::leak(Box::new(
            BlockRegistry::from_nbt(&Tag::compound([
                ("DataVersion", Tag::Int(crate::DATA_VERSION)),
                (
                    "states",
                    Tag::List(
                        names
                            .iter()
                            .map(|n| Tag::compound([("id", Tag::String((*n).into()))]))
                            .collect(),
                    ),
                ),
                (
                    "defaults",
                    Tag::Compound(
                        names
                            .iter()
                            .enumerate()
                            .map(|(i, n)| ((*n).into(), Tag::Int(i as i32)))
                            .collect(),
                    ),
                ),
            ]))
            .unwrap(),
        ));
        let make_state = || {
            let registry = StructureRegistry::new(
                &BTreeMap::new(),
                &BTreeMap::new(),
                BTreeMap::new(),
                &|_| Ok(BTreeSet::new()),
                mod_structure_types(),
            )
            .unwrap();
            let resources = StructureResources::new(
                blocks,
                StateTransforms::default(),
                std::path::Path::new("unused"),
                BTreeMap::new(),
                &BTreeMap::new(),
            );
            let bindings = FeatureBlockBindings::new(blocks, BTreeSet::new()).unwrap();
            let rings = Arc::new(OnceLock::new());
            rings.set(Rings::new()).unwrap();
            StructureState {
                registry,
                resources: Arc::new(resources),
                bindings,
                possible_sets: Vec::new(),
                by_step: vec![Vec::new(); crate::worldgen::structure::start::STEPS.len()],
                starts: StartCache::new(),
                saved: OnceLock::new(),
                start_work: ChunkWork::new(),
                rings,
                ring_work: OnceLock::new(),
                ring_seed: None,
                fixed_biome: false,
                zone_reach: 0,
                biome_names: crate::worldgen::structure_terrain::BiomeNames::new(&[]),
            }
        };

        let hit = BoundingBox::new([0, 0, 0], [15, 10, 15]);
        let miss = BoundingBox::new([100, 0, 100], [115, 10, 115]);
        let make_starts = |chunk: ChunkPos, specs: &[(usize, BoundingBox)]| -> Starts {
            Arc::new(
                specs
                    .iter()
                    .map(|&(structure, bounding_box)| {
                        Arc::new(StructureStart {
                            structure,
                            name: format!("test:s{structure}"),
                            chunk,
                            references: 0,
                            pieces: Vec::new(),
                            bounding_box,
                            has_piece_beardifier: false,
                        })
                    })
                    .collect(),
            )
        };
        let summary = |starts: &[Arc<StructureStart>]| -> Vec<(usize, ChunkPos)> {
            starts.iter().map(|s| (s.structure, s.chunk)).collect()
        };

        let target = ChunkPos::new(0, 0);
        let trigger = ChunkPos::new(8, 8);

        let run_case = |precached: &[CaseEntry<'_>],
                        busy: &[CaseEntry<'_>],
                        expected: &[(usize, ChunkPos)]| {
            let state = make_state();
            for &(chunk, specs) in precached {
                state
                    .starts
                    .insert_first((chunk.x, chunk.z), make_starts(chunk, specs));
            }
            let busy_entries: Vec<(ChunkPos, Starts)> = busy
                .iter()
                .map(|&(chunk, specs)| (chunk, make_starts(chunk, specs)))
                .collect();

            let (tx_ready, rx_ready) = mpsc::channel();
            let (tx_release, rx_release) = mpsc::channel();
            let state_ref = &state;
            let busy_ref = &busy_entries;

            let (contended, queried) = std::thread::scope(|scope| {
                scope.spawn(move || {
                    let locks: Vec<Arc<Mutex<()>>> = busy_ref
                        .iter()
                        .map(|(chunk, _)| state_ref.start_work.at((chunk.x, chunk.z)))
                        .collect();
                    let guards: Vec<_> = locks.iter().map(|lock| lock.lock().unwrap()).collect();
                    let _ = tx_ready.send(());
                    if rx_release.recv().is_ok() {
                        for (chunk, starts) in busy_ref {
                            state_ref
                                .starts
                                .insert_first((chunk.x, chunk.z), starts.clone());
                        }
                    }
                    drop(guards);
                });
                let mut context = ReleaseOnTrigger {
                    trigger,
                    queried: Mutex::new(Vec::new()),
                    release: Mutex::new(Some(tx_release)),
                };
                rx_ready.recv().unwrap();
                let out = state.references(&mut context, target);
                (out, context.queried.into_inner().unwrap())
            });

            // Every busy chunk was queried before the trigger chunk released its lock,
            // proving Pass 1 observed `WouldBlock` and entered the two-pass `deferred` path.
            let trigger_pos = queried
                .iter()
                .position(|&c| c == trigger)
                .expect("trigger chunk must be queried during pass one");
            for &(chunk, _) in busy {
                let busy_pos = queried
                    .iter()
                    .position(|&c| c == chunk)
                    .expect("busy chunk must be queried during pass one");
                assert!(
                    busy_pos < trigger_pos,
                    "busy chunk {chunk:?} must hit pass one before locks are released"
                );
            }

            // Now all 289 chunks are cached and uncontended: the single-pass fast path
            // must produce the exact same order.
            let mut idle = ReleaseOnTrigger {
                trigger,
                queried: Mutex::new(Vec::new()),
                release: Mutex::new(None),
            };
            let uncontended = state.references(&mut idle, target);
            assert!(
                idle.queried.into_inner().unwrap().is_empty(),
                "uncontended pass must hit cached starts for all 289 chunks"
            );

            let contended_summary = summary(&contended);
            let uncontended_summary = summary(&uncontended);
            assert_eq!(contended_summary, uncontended_summary);
            assert_eq!(contended_summary.as_slice(), expected);
        };

        // Case 1: slots before the first busy chunk (emitted in pass one),
        // multiple busy chunks (with multiple starts and an empty busy chunk),
        // deferred slots between and after busy chunks (including `(0, 0)`),
        // non-intersecting boxes before and after busy chunks, and 5 starts of
        // structure 0 whose chunk keys all collide in bucket 0 of `LongOpenHashSet`
        // so that out-of-order emission changes the reordered output.
        run_case(
            &[
                (ChunkPos::new(-8, -8), &[(1, hit)]),
                (ChunkPos::new(-6, -3), &[(0, hit)]),
                (ChunkPos::new(-4, 5), &[(2, hit)]),
                (ChunkPos::new(-3, 2), &[(3, miss)]),
                (ChunkPos::new(-2, -7), &[(0, hit)]),
                (ChunkPos::new(0, 0), &[(6, hit)]),
                (ChunkPos::new(0, 3), &[(3, miss)]),
                (ChunkPos::new(2, -4), &[(7, hit)]),
                (ChunkPos::new(6, -1), &[(9, hit)]),
                (ChunkPos::new(8, 7), &[(0, hit)]),
            ],
            &[
                (ChunkPos::new(-2, -8), &[(0, hit), (4, hit)]),
                (ChunkPos::new(1, 1), &[]),
                (ChunkPos::new(4, 2), &[(0, hit), (8, hit)]),
            ],
            &[
                (1, ChunkPos::new(-8, -8)),
                (0, ChunkPos::new(8, 7)),
                (2, ChunkPos::new(-4, 5)),
                (0, ChunkPos::new(4, 2)),
                (4, ChunkPos::new(-2, -8)),
                (0, ChunkPos::new(-2, -7)),
                (6, ChunkPos::new(0, 0)),
                (7, ChunkPos::new(2, -4)),
                (0, ChunkPos::new(-2, -8)),
                (8, ChunkPos::new(4, 2)),
                (9, ChunkPos::new(6, -1)),
                (0, ChunkPos::new(-6, -3)),
            ],
        );

        // Case 2: the very first slot `(-8, -8)` and `(0, 0)` (which matches
        // `busy`'s zero-initialized sentinel and `LongOpenHashSet`'s zero key)
        // are busy, so every non-busy start is deferred to pass two.
        run_case(
            &[
                (ChunkPos::new(-6, -3), &[(0, hit)]),
                (ChunkPos::new(-2, -8), &[(0, hit), (1, hit)]),
                (ChunkPos::new(-2, -7), &[(0, hit)]),
                (ChunkPos::new(4, 2), &[(0, hit)]),
                (ChunkPos::new(8, 7), &[(0, hit), (2, hit)]),
            ],
            &[
                (ChunkPos::new(-8, -8), &[(3, hit)]),
                (ChunkPos::new(0, 0), &[(0, hit), (4, hit)]),
            ],
            &[
                (3, ChunkPos::new(-8, -8)),
                (0, ChunkPos::new(0, 0)),
                (0, ChunkPos::new(8, 7)),
                (1, ChunkPos::new(-2, -8)),
                (0, ChunkPos::new(4, 2)),
                (0, ChunkPos::new(-2, -7)),
                (4, ChunkPos::new(0, 0)),
                (0, ChunkPos::new(-2, -8)),
                (0, ChunkPos::new(-6, -3)),
                (2, ChunkPos::new(8, 7)),
            ],
        );
    }
}
