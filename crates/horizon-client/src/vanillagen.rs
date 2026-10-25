//! Binds `horizon_world`'s vanilla overworld generator (noise terrain,
//! aquifers, surface rules, carvers and placed features) to chunk streaming.
use crate::{
    biome::Biomes,
    chunk::{BIOME_STRATEGY, Chunk, HEIGHT, MIN_Y, SECTIONS, Section},
    save::SavedProto,
    streaming::{ChunkGenerator, LodSampler, Proto},
};
use horizon_core::{
    paletted::PalettedContainer,
    position::ChunkPos,
    ticks::{ProtoChunkTicks, SavedTick, TickTypeId},
};
use horizon_world::{
    heightmap::HeightmapType,
    nbt::Tag,
    registry::BlockRegistry,
    worldgen::{
        generator::{
            GeneratorData, GeneratorWorker, OverworldGenerator, ProtoChunk, SurfaceSample,
        },
        terrain::PreLightChunkStage,
    },
};
use std::{path::Path, sync::Arc};
mod lod;
use lod::VanillaLodSampler;

pub struct VanillaTerrain {
    generator: OverworldGenerator,
    /// Generator biome index -> client biome table index.
    biome_map: Vec<u16>,
    /// Loot tables for generated containers.
    loot: Option<Arc<horizon_world::loot::LootData>>,
    /// A worker for structure queries from the game thread.
    query: std::sync::Mutex<Option<GeneratorWorker>>,
    /// Block state id -> block index, the game's block tick type: worldgen
    /// schedules block ticks by default state id.
    tick_blocks: Vec<u32>,
    /// Block index -> default state id: `tick_blocks` the other way.
    tick_states: Vec<u32>,
    seed: i64,
    dimension: String,
    /// When the last proto chunk was generated, in milliseconds since
    /// `created`: the ring search waits for a pause in generation.
    last_proto: std::sync::atomic::AtomicU64,
    created: std::time::Instant,
}
/// A proto chunk, or the position of one whose generation failed.
enum VanillaProto {
    Ready(Box<ProtoChunk>),
    Failed(i32, i32),
}
impl VanillaTerrain {
    fn client_surface(&self, surface: SurfaceSample) -> crate::lod::Surface {
        crate::lod::Surface {
            height: surface.height,
            state: surface.state,
            water: surface.water,
            biome: self
                .biome_map
                .get(surface.biome.0 as usize)
                .copied()
                .unwrap_or(0),
        }
    }
    pub fn new(
        dir: &Path,
        registry: &'static BlockRegistry,
        biomes: &Biomes,
        seed: i64,
    ) -> crate::Result<Self> {
        Self::for_dimension(dir, registry, biomes, seed, "minecraft:overworld")
    }
    pub fn for_dimension(
        dir: &Path,
        registry: &'static BlockRegistry,
        biomes: &Biomes,
        seed: i64,
        dimension: &str,
    ) -> crate::Result<Self> {
        Self::for_world(dir, registry, biomes, seed, dimension, true)
    }
    /// A dimension of a world whose `generate_structures` is `structures`
    /// (`ChunkGenerator.createStructures` and `findNearestMapStructure` do
    /// nothing without).
    pub fn for_world(
        dir: &Path,
        registry: &'static BlockRegistry,
        biomes: &Biomes,
        seed: i64,
        dimension: &str,
        structures: bool,
    ) -> crate::Result<Self> {
        let data: Arc<GeneratorData> = shared_data(dir, registry)?;
        let biome_map = data
            .biome_names()
            .iter()
            .map(|name| biomes.id(name))
            .collect();
        let generator =
            OverworldGenerator::for_world(data, seed, "minecraft:normal", dimension, structures)?;
        let unsupported = generator.unsupported_features();
        if !unsupported.is_empty() {
            log::warn!(
                "Horizon: {} placed features cannot run and will be skipped",
                unsupported.len()
            );
            if !dir.join("blocks/block-properties.nbt").exists() {
                log::warn!(
                    "Horizon: the worldgen bundle lacks the block behavior exports (worldgen/blocks/*.nbt); \
                     prepare again (the launcher, or horizon-prepare --prepare) to add them"
                );
            }
        }
        let loot = dir.parent().and_then(crate::structure_blocks::loot_data);
        if let Some(root) = dir.parent() {
            crate::mobs::trial_spawner::load_registry(root);
        }
        let mut tick_blocks = Vec::with_capacity(registry.len());
        let mut tick_states = Vec::new();
        let mut last: Option<&str> = None;
        let mut index = 0u32;
        for id in 0..registry.len() {
            let name = registry
                .state(horizon_world::registry::BlockStateId(id as u32))
                .map_or("", |s| s.name.as_str());
            if last.is_some_and(|l| l != name) {
                index += 1;
            }
            if last != Some(name) {
                tick_states.push(registry.default_state(name).map_or(id as u32, |s| s.0));
            }
            last = Some(name);
            tick_blocks.push(index);
        }
        Ok(Self {
            generator,
            biome_map,
            loot,
            query: std::sync::Mutex::new(None),
            tick_blocks,
            tick_states,
            seed,
            dimension: dimension.to_owned(),
            last_proto: std::sync::atomic::AtomicU64::new(0),
            created: std::time::Instant::now(),
        })
    }
    /// A proto chunk's blocks, biomes, scheduled ticks and post-processing
    /// marks as a column.
    fn column(&self, chunk: &ProtoChunk) -> Chunk {
        let pos = chunk.position();
        let mut out = Chunk::empty(pos.x, pos.z);
        let (bx, bz) = (pos.x * 16, pos.z * 16);
        // Only the generated range (the Nether is 0..256) is copied.
        let (lo, hi) = (
            chunk.terrain.min_y().max(MIN_Y),
            (chunk.terrain.min_y() + chunk.terrain.height()).min(MIN_Y + HEIGHT),
        );
        out.structure_refs = chunk
            .structures()
            .into_iter()
            .map(|(id, start, _, _)| (Arc::from(id), (start.x, start.z)))
            .collect();
        out.structure_starts = chunk.own_starts();
        let mut values = Box::new([0u32; 4096]);
        for si in 0..SECTIONS as i32 {
            let base = MIN_Y + si * 16;
            if base + 16 <= lo || base >= hi {
                continue;
            }
            values.fill(0);
            let mut any = false;
            for y in base.max(lo)..(base + 16).min(hi) {
                for z in 0..16 {
                    for x in 0..16 {
                        let id = chunk.block(bx + x, y, bz + z).0;
                        values[((y & 15) << 8 | z << 4 | x) as usize] = id;
                        any |= id != 0;
                    }
                }
            }
            if any {
                Arc::make_mut(&mut out.sections[si as usize]).blocks =
                    Section::from_blocks(&values);
            }
        }
        // Ticks the features scheduled (leaves reshaped at a tree's edge,
        // fluids of lakes and springs) run once the chunk is live, as
        // LevelChunk takes over ProtoChunk's ticks.
        let block_ticks: Vec<_> = chunk
            .terrain
            .block_ticks()
            .pack(0)
            .iter()
            .filter_map(|t| {
                let kind = *self.tick_blocks.get(t.kind.0 as usize)?;
                Some(horizon_core::ticks::SavedTick {
                    kind: horizon_core::ticks::TickTypeId(kind),
                    ..*t
                })
            })
            .collect();
        let fluid_ticks = chunk.terrain.fluid_ticks().pack(0).to_vec();
        if !block_ticks.is_empty() || !fluid_ticks.is_empty() {
            out.saved = Some(Arc::new(crate::save::ChunkExtras {
                block_ticks,
                fluid_ticks,
                ..Default::default()
            }));
        }
        let base_section = chunk.terrain.min_y() >> 4;
        for (i, marks) in chunk.terrain.postprocessing().iter().enumerate() {
            let sy = (base_section + i as i32) * 16;
            for &packed in marks {
                let p = i32::from(packed);
                let y = sy + ((p >> 4) & 15);
                if (lo..hi).contains(&y) {
                    out.postprocess
                        .push([bx + (p & 15), y, bz + ((p >> 8) & 15)]);
                }
            }
        }
        for si in 0..SECTIONS as i32 {
            let mut cells = [0u32; 64];
            for (j, cell) in cells.iter_mut().enumerate() {
                let (qx, qz, qy) = (j as i32 & 3, (j as i32 >> 2) & 3, si * 4 + (j as i32 >> 4));
                let y = (MIN_Y + qy * 4).clamp(lo, hi - 1);
                let id = chunk.biome_id_at(bx + qx * 4, y, bz + qz * 4);
                *cell = u32::from(self.biome_map.get(id.0 as usize).copied().unwrap_or(0));
            }
            Arc::make_mut(&mut out.sections[si as usize]).biomes =
                PalettedContainer::from_values(BIOME_STRATEGY, &cells);
        }
        out
    }
    /// The structure starts owned by a chunk (cached by the generator).
    pub fn starts_at(
        &self,
        x: i32,
        z: i32,
    ) -> Vec<Arc<horizon_world::worldgen::structure::start::StructureStart>> {
        let mut worker = self.query.lock().unwrap_or_else(|e| e.into_inner());
        let worker = worker.get_or_insert_with(|| self.generator.worker());
        worker.structure_starts(ChunkPos::new(x, z))
    }
    /// `/locate structure`: the nearest start of one of `names` within
    /// `radius` placement sectors of block `from`.
    pub fn locate(
        &self,
        names: &[String],
        from: [i32; 3],
        radius: i32,
    ) -> Option<(String, [i32; 3])> {
        self.generator.worker().locate(names, from, radius)
    }
    /// `/locate biome`: `BiomeSource.findClosestBiome3d`.
    pub fn find_closest_biome(
        &self,
        search: &horizon_world::worldgen::generator::BiomeSearch<'_>,
    ) -> Option<(String, [i32; 3])> {
        let mut worker = self.query.lock().unwrap_or_else(|e| e.into_inner());
        let worker = worker.get_or_insert_with(|| self.generator.worker());
        worker.find_closest_biome(search)
    }
    /// `BiomeSource.getNoiseBiome` at quart positions, for places that may not
    /// be generated yet (`MapItem.renderBiomePreviewMap`).
    pub fn noise_biomes(&self, quarts: &[[i32; 3]]) -> Vec<Option<String>> {
        let mut worker = self.query.lock().unwrap_or_else(|e| e.into_inner());
        let worker = worker.get_or_insert_with(|| self.generator.worker());
        worker.noise_biomes(quarts)
    }
    /// `/place jigsaw`: `JigsawPlacement.addPieces`.
    pub fn jigsaw_pieces(
        &self,
        query: &horizon_world::worldgen::generator::JigsawQuery<'_>,
    ) -> Option<horizon_world::worldgen::generator::JigsawPieces> {
        let mut worker = self.query.lock().unwrap_or_else(|e| e.into_inner());
        let worker = worker.get_or_insert_with(|| self.generator.worker());
        worker.jigsaw_pieces(query)
    }
    /// NoiseBasedChunkGenerator.getOrigin: where the world spawn search starts.
    pub fn origin(&self) -> crate::Result<ChunkPos> {
        Ok(self.generator.origin()?)
    }
    /// Concentric ring positions of a structure set (strongholds), searched
    /// for now if no chunk needed them yet.
    pub fn ring_positions(&self, set: &str) -> Option<Vec<ChunkPos>> {
        let mut worker = self.query.lock().unwrap_or_else(|e| e.into_inner());
        let worker = worker.get_or_insert_with(|| self.generator.worker());
        worker.ring_positions(set)
    }
    /// Searches the concentric rings on a thread of their own, as vanilla's
    /// background executor does, once generation pauses for two seconds
    /// (at the latest a minute after the world opens) and in the idle
    /// class: on a laptop's few cores, sharing them with the first chunks,
    /// which don't need the rings, made those 3 s slower. An eye of ender
    /// thrown later finds the rings ready, and a chunk or a query that
    /// needs them first shares the remaining searches with this thread.
    pub fn search_rings_behind(self: &Arc<Self>) {
        const QUIET: u64 = 2_000;
        const LATEST: u64 = 60_000;
        let terrain = Arc::downgrade(self);
        let spawned = std::thread::Builder::new()
            .name("Ring search".into())
            .spawn(move || {
                horizon_threads::idle_current();
                loop {
                    std::thread::sleep(std::time::Duration::from_millis(500));
                    // The world closed: nothing to search for.
                    let Some(terrain) = terrain.upgrade() else {
                        return;
                    };
                    let now = terrain.elapsed_ms();
                    let last = terrain
                        .last_proto
                        .load(std::sync::atomic::Ordering::Relaxed);
                    if now >= LATEST || (last > 0 && now - last >= QUIET) {
                        let _ = terrain
                            .generator
                            .worker()
                            .ring_positions("minecraft:strongholds");
                        return;
                    }
                }
            });
        if let Err(e) = spawned {
            log::warn!("ring search thread: {e}");
        }
    }
    fn elapsed_ms(&self) -> u64 {
        u64::try_from(self.created.elapsed().as_millis()).unwrap_or(u64::MAX)
    }
}
/// The worldgen bundle at `dir`, loaded once and shared by every generator
/// and by live feature placement (sapling growth, bone meal).
pub fn shared_data(
    dir: &Path,
    registry: &'static BlockRegistry,
) -> crate::Result<Arc<GeneratorData>> {
    static CACHE: std::sync::Mutex<Vec<(std::path::PathBuf, Arc<GeneratorData>)>> =
        std::sync::Mutex::new(Vec::new());
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, data)) = cache.iter().find(|(path, _)| path == dir) {
        return Ok(data.clone());
    }
    let data = GeneratorData::load(dir, registry)?;
    cache.push((dir.to_owned(), data.clone()));
    Ok(data)
}
impl ChunkGenerator for VanillaTerrain {
    type Worker = GeneratorWorker;
    fn worker(&self) -> GeneratorWorker {
        self.generator.worker()
    }
    fn lod_sampler(self: Arc<Self>) -> Box<dyn LodSampler> {
        Box::new(VanillaLodSampler::new(self))
    }
    fn lod_surface(
        &self,
        worker: &mut GeneratorWorker,
        x: i32,
        z: i32,
    ) -> Option<crate::lod::Surface> {
        Some(self.client_surface(worker.surface_sample(x, z)?))
    }
    /// The structure starts a saved chunk references, as vanilla reads them
    /// with the chunk: built here, on the loading worker, into the
    /// generator's cache, rather than on the game thread the first time
    /// spawning or a mob asks which structures hold a position (a village
    /// start took 70 to 110 ms there, one tick each, after a world loaded).
    fn prepare_loaded(&self, worker: &mut GeneratorWorker, chunk: &Chunk) {
        let mut starts: Vec<(i32, i32)> = chunk.structure_refs.iter().map(|(_, at)| *at).collect();
        starts.sort_unstable();
        starts.dedup();
        for (x, z) in starts {
            worker.structure_starts(ChunkPos::new(x, z));
        }
    }
    fn proto(&self, worker: &mut GeneratorWorker, x: i32, z: i32) -> Proto {
        self.last_proto.store(
            self.elapsed_ms().max(1),
            std::sync::atomic::Ordering::Relaxed,
        );
        match worker.generate_proto(ChunkPos::new(x, z)) {
            Ok(chunk) => Box::new(VanillaProto::Ready(Box::new(chunk))),
            Err(e) => {
                log::warn!("Horizon: generating chunk {x} {z} failed: {e}");
                Box::new(VanillaProto::Failed(x, z))
            }
        }
    }
    fn decorates(&self) -> bool {
        true
    }
    fn decorate(&self, worker: &mut GeneratorWorker, region: &mut [Proto; 9]) {
        let taken: Vec<Box<VanillaProto>> = region
            .iter_mut()
            .map(|p| {
                std::mem::replace(p, Box::new(()))
                    .downcast::<VanillaProto>()
                    .expect("vanilla proto")
            })
            .collect();
        let all_ready = taken.iter().all(|p| matches!(**p, VanillaProto::Ready(_)));
        if all_ready {
            let chunks: Vec<ProtoChunk> = taken
                .into_iter()
                .map(|p| match *p {
                    VanillaProto::Ready(c) => *c,
                    VanillaProto::Failed(..) => unreachable!(),
                })
                .collect();
            let mut array: [ProtoChunk; 9] = match chunks.try_into() {
                Ok(a) => a,
                Err(_) => unreachable!(),
            };
            if let Err(e) = worker.decorate(&mut array) {
                log::warn!("Horizon: decorating chunk failed: {e}");
            }
            for (slot, chunk) in region.iter_mut().zip(array) {
                *slot = Box::new(VanillaProto::Ready(Box::new(chunk)));
            }
        } else {
            for (slot, p) in region.iter_mut().zip(taken) {
                *slot = p;
            }
        }
    }
    fn is_decorated(&self, proto: &Proto) -> bool {
        matches!(proto.downcast_ref::<VanillaProto>(), Some(VanillaProto::Ready(c)) if c.is_decorated())
    }
    fn saved_proto(&self, proto: &Proto) -> Option<SavedProto> {
        let Some(VanillaProto::Ready(chunk)) = proto.downcast_ref::<VanillaProto>() else {
            return None;
        };
        let block_entities = chunk
            .block_entities
            .iter()
            .map(|(pos, nbt)| {
                let mut nbt = nbt.clone();
                for (key, v) in ["x", "y", "z"].into_iter().zip(pos) {
                    nbt.insert(key.into(), Tag::Int(*v));
                }
                Tag::Compound(nbt)
            })
            .collect();
        Some(SavedProto {
            column: self.column(chunk),
            decorated: chunk.is_decorated(),
            block_entities,
            entities: chunk.entities.iter().cloned().map(Tag::Compound).collect(),
        })
    }
    /// The saved state replaces what decoration can change; biomes, structure
    /// starts and references are generated again as they were.
    fn restore_proto(&self, proto: &mut Proto, saved: SavedProto) {
        let Some(VanillaProto::Ready(chunk)) = proto.downcast_mut::<VanillaProto>() else {
            return;
        };
        let column = &saved.column;
        let (bx, bz) = (column.x * 16, column.z * 16);
        let (lo, hi) = (
            chunk.terrain.min_y().max(MIN_Y),
            (chunk.terrain.min_y() + chunk.terrain.height()).min(MIN_Y + HEIGHT),
        );
        for y in lo..hi {
            for z in 0..16 {
                for x in 0..16 {
                    let state = column.block(x, y, z);
                    if chunk.block(bx + x, y, bz + z) != state {
                        let _ = chunk.terrain.set_block_raw(bx + x, y, bz + z, state);
                    }
                }
            }
        }
        if saved.decorated {
            chunk
                .terrain
                .set_generation_stage(PreLightChunkStage::Features);
            chunk.set_decorated();
        }
        // Heightmaps follow the blocks (ProtoChunk primes missing ones on read).
        let stage = chunk.terrain.generation_stage().heightmaps();
        let kinds: Vec<HeightmapType> = HeightmapType::ALL
            .into_iter()
            .filter(|&k| chunk.terrain.heightmap(k).is_some() || stage.contains(&k))
            .collect();
        let _ = chunk.terrain.prime_heightmaps(&kinds);
        let ticks = |ticks: &[SavedTick], kind: &dyn Fn(u32) -> Option<u32>| {
            ProtoChunkTicks::from_saved(ticks.iter().filter_map(|t| {
                Some(SavedTick {
                    kind: TickTypeId(kind(t.kind.0)?),
                    ..*t
                })
            }))
        };
        if let Some(extras) = column.saved.as_deref() {
            *chunk.terrain.block_ticks_mut() = ticks(&extras.block_ticks, &|k| {
                self.tick_states.get(k as usize).copied()
            });
            *chunk.terrain.fluid_ticks_mut() = ticks(&extras.fluid_ticks, &Some);
        }
        chunk.terrain.clear_postprocessing();
        for &[x, y, z] in &column.postprocess {
            chunk.terrain.mark_postprocessing(x, y, z);
        }
        chunk.block_entities = saved
            .block_entities
            .into_iter()
            .filter_map(|tag| {
                let Tag::Compound(mut nbt) = tag else {
                    return None;
                };
                let mut coordinate = |key: &str| match nbt.remove(&key.into()) {
                    Some(Tag::Int(v)) => Some(v),
                    _ => None,
                };
                let pos = [coordinate("x")?, coordinate("y")?, coordinate("z")?];
                Some((pos, nbt))
            })
            .collect();
        chunk.entities = saved
            .entities
            .into_iter()
            .filter_map(|tag| match tag {
                Tag::Compound(nbt) => Some(nbt),
                _ => None,
            })
            .collect();
    }
    fn finish(&self, proto: Proto) -> Chunk {
        let proto = proto.downcast::<VanillaProto>().expect("vanilla proto");
        let mut chunk = match *proto {
            VanillaProto::Ready(c) => c,
            VanillaProto::Failed(x, z) => return Chunk::empty(x, z),
        };
        let mut out = self.column(&chunk);
        out.block_entities = crate::structure_blocks::block_entities(&chunk, self.loot.as_deref());
        // The block entities Horizon does not model (decorated pots with their
        // sherds and loot table, brushable blocks) stay as placed, like a
        // loaded chunk's.
        let unmodelled: Vec<Tag> = chunk
            .block_entities
            .iter()
            .filter(|(pos, _)| {
                !out.block_entities
                    .iter()
                    .any(|(p, _)| *p == (pos[0], pos[1], pos[2]))
            })
            .filter_map(|(pos, nbt)| {
                let id = crate::lan::chunks::block_entity_type(
                    chunk.block_name(pos[0], pos[1], pos[2]),
                )?;
                let mut nbt = nbt.clone();
                nbt.insert("id".into(), Tag::String(id.into()));
                for (key, v) in ["x", "y", "z"].into_iter().zip(pos) {
                    nbt.insert(key.into(), Tag::Int(*v));
                }
                Some(Tag::Compound(nbt))
            })
            .collect();
        if !unmodelled.is_empty() {
            let mut extras = out
                .saved
                .take()
                .map(Arc::unwrap_or_clone)
                .unwrap_or_default();
            extras.block_entities = unmodelled;
            out.saved = Some(Arc::new(extras));
        }
        let entities = std::mem::take(&mut chunk.entities);
        out.entities = entities
            .into_iter()
            .map(|nbt| {
                crate::structure_blocks::unpack_entity_loot(&chunk, nbt, self.loot.as_deref())
            })
            .map(horizon_world::nbt::Tag::Compound)
            .collect();
        for s in &mut out.sections {
            Arc::make_mut(s).compact();
        }
        out.fresh = true;
        out
    }
}
