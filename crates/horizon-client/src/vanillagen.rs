//! Binds `horizon_world`'s vanilla overworld generator (noise terrain,
//! aquifers, surface rules, carvers and placed features) to chunk streaming.
use crate::{
    biome::Biomes,
    chunk::{BIOME_STRATEGY, Chunk, ColumnLayout, Section},
    save::SavedProto,
    streaming::{ChunkGenerator, LodSampler, Proto},
};
use horizon_core::{
    paletted::PalettedContainer,
    position::ChunkPos,
    ticks::{ProtoChunkTicks, SavedTick, TickTypeId},
};
use std::{path::Path, sync::Arc};
use {
    horizon_core::nbt::Tag,
    horizon_world::{
        heightmap::HeightmapType,
        item_defaults::ItemData,
        registry::BlockRegistry,
        worldgen::{
            biome_source::regions::RegionLayout,
            generator::{
                GeneratorData, GeneratorWorker, OverworldGenerator, ProtoChunk, SurfaceSample,
            },
            overrides::WorldgenOverrides,
            structure::legacy::PieceRandom,
            terrain::PreLightChunkStage,
        },
    },
};
mod lod;
mod structure_mobs;
use lod::VanillaLodSampler;
use structure_mobs::StructureMobFinalizer;

/// The references a saved chunk keeps for its structure start: `(structure, chunk x, chunk z)`.
pub type SavedReferences = std::sync::Arc<dyn Fn(&str, i32, i32) -> i32 + Send + Sync>;

/// Borrowed generation inputs from the owner's published resource tables.
#[derive(Clone, Copy)]
pub struct TerrainResources<'a> {
    pub data: Option<&'a horizon_assets::resource_source::ResourceSource>,
    pub dir: &'a Path,
    pub registry: &'static BlockRegistry,
    pub biomes: &'a Biomes,
    pub item_data: &'a Arc<ItemData>,
}

pub struct VanillaTerrain {
    generator: OverworldGenerator,
    /// Generator biome index -> client biome table index.
    biome_map: Vec<u16>,
    /// Loot tables for generated containers.
    structure_mobs: Arc<StructureMobFinalizer>,
    /// A worker for structure queries from the game thread.
    query: std::sync::Mutex<Option<GeneratorWorker>>,
    /// Block state id -> block index, the game's block tick type: worldgen
    /// schedules block ticks by default state id.
    tick_blocks: Vec<u32>,
    /// Block index -> default state id: `tick_blocks` the other way.
    tick_states: Vec<u32>,
    /// Block state id -> the block entity type of its block, if it has one.
    block_entity_types: Vec<Option<&'static str>>,
    seed: i64,
    /// The start chunks an exploration map pointed at, by structure name: a
    /// skip check reads the set by name, without a String per candidate.
    referenced: std::sync::Mutex<
        crate::fast_hash::FxHashMap<String, crate::fast_hash::FxHashSet<(i32, i32)>>,
    >,
    reference_file: std::sync::Mutex<Option<std::path::PathBuf>>,
    saved_references: std::sync::Mutex<Option<SavedReferences>>,
    /// Each saved chunk's reference count as the skip check last read it, by
    /// structure name: a saved chunk's record does not change while the world
    /// is open, so the first map that searches past it is the last to read it
    /// (vanilla's `StructureCheck` keeps its answers for the session too).
    saved_checked: std::sync::Mutex<
        crate::fast_hash::FxHashMap<String, crate::fast_hash::FxHashMap<(i32, i32), i32>>,
    >,
    /// The world preset the terrain was generated with (`minecraft:normal`
    /// unless a saved world says otherwise); far decoration uses it too.
    preset: String,
    dimension: String,
    /// When the last proto chunk was generated, in milliseconds since
    /// `created`: the ring search waits for a pause in generation.
    last_proto: std::sync::atomic::AtomicU64,
    created: std::time::Instant,
    /// A superflat generator (`FlatLevelSource`).
    flat: bool,
    /// A mod's chunk generator with vanilla stages: its terrain is no noise to draw far away.
    mod_terrain: bool,
}
/// The far ring of a mod's terrain: none, as for the mod's terrain alone.
struct NoFarGround;
impl LodSampler for NoFarGround {
    fn sample(&mut self, _: i32, _: i32) -> Option<crate::lod::Surface> {
        None
    }
}
/// A proto chunk, or the position of one whose generation failed.
enum VanillaProto {
    Ready(Box<ResidentProto>),
    Failed(i32, i32),
}
/// Generator terrain plus the resident biome cells read from disk. The cells use
/// the world's shared biome table, so even names absent from generator data survive.
struct ResidentProto {
    terrain_chunk: ProtoChunk,
    saved_biomes: Option<SavedBiomeCells>,
}
struct SavedBiomeCells {
    min_y: i32,
    sections: Vec<PalettedContainer>,
}
impl SavedBiomeCells {
    fn capture(column: &Chunk) -> Self {
        Self {
            min_y: column.min_y(),
            sections: column.sections.iter().map(|s| s.biomes.clone()).collect(),
        }
    }
    fn apply(&self, column: &mut Chunk) {
        for (index, biomes) in self.sections.iter().enumerate() {
            let y = self.min_y + index as i32 * 16;
            if let Some(target) = column.layout().section_index(y) {
                Arc::make_mut(&mut column.sections[target]).biomes = biomes.clone();
            }
        }
    }
}
impl std::ops::Deref for ResidentProto {
    type Target = ProtoChunk;
    fn deref(&self) -> &ProtoChunk {
        &self.terrain_chunk
    }
}
impl std::ops::DerefMut for ResidentProto {
    fn deref_mut(&mut self) -> &mut ProtoChunk {
        &mut self.terrain_chunk
    }
}
impl ResidentProto {
    fn column(&self, terrain: &VanillaTerrain) -> Chunk {
        let mut column = terrain.column(self);
        if let Some(saved) = &self.saved_biomes {
            saved.apply(&mut column);
        }
        column
    }
}
/// WorldGenRegion.setBlock gives every block with a block entity a new
/// one: the blocks nothing placed data for (a sculk patch's sensors) get
/// vanilla's defaults.
fn add_default_block_entities(kinds: &[Option<&str>], out: &mut Chunk, raw: &mut Vec<Tag>) {
    let kind = |v: u32| kinds.get(v as usize).copied().flatten();
    let mut states = [0u32; 4096];
    for (si, section) in out.sections.iter().enumerate() {
        if !section.blocks.maybe_has(|v| kind(v).is_some()) {
            continue;
        }
        section.unpack(&mut states);
        for (i, &v) in states.iter().enumerate() {
            let Some(id) = kind(v) else {
                continue;
            };
            let i = i as i32;
            let pos = [
                out.x * 16 + (i & 15),
                out.min_y() + si as i32 * 16 + (i >> 8),
                out.z * 16 + ((i >> 4) & 15),
            ];
            let at = |t: &Tag| {
                ["x", "y", "z"]
                    .iter()
                    .zip(pos)
                    .all(|(k, p)| t.get(k) == Some(&Tag::Int(p)))
            };
            if raw.iter().any(at) || out.block_entities.iter().any(|(p, _)| *p == pos.into()) {
                continue;
            }
            let tag = Tag::compound([
                ("id", Tag::String(id.into())),
                ("x", Tag::Int(pos[0])),
                ("y", Tag::Int(pos[1])),
                ("z", Tag::Int(pos[2])),
            ]);
            let tag = crate::block_entity_nbt::canonical(&tag).unwrap_or(tag);
            match crate::save::decode_block_entity(&tag) {
                Some(modelled) => out.block_entities.push(modelled),
                None => raw.push(tag),
            }
        }
    }
}

impl VanillaTerrain {
    /// Generated native entities use their declared type; undeclared signs
    /// keep their template's vanilla type and start with editable text.
    pub fn add_mod_signs(&mut self, facts: &crate::blocks::BlockFacts) {
        for (state, kind) in self.block_entity_types.iter_mut().enumerate() {
            let state = horizon_world::registry::BlockStateId(state as u32);
            let block = facts.get(state);
            if let Some(declared) = crate::block_entity_types::type_for(&block.name) {
                *kind = Some(declared);
            } else if let Some(kind_id) = crate::lan::chunks::block_entity_type(&block.name) {
                *kind = Some(kind_id);
            } else if let Some(form) = block.sign_form() {
                *kind = Some(form.block_entity_type());
            }
        }
    }
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
    pub fn new(resources: TerrainResources<'_>, seed: i64) -> crate::Result<Self> {
        Self::for_dimension(resources, seed, "minecraft:overworld")
    }
    pub fn for_dimension(
        resources: TerrainResources<'_>,
        seed: i64,
        dimension: &str,
    ) -> crate::Result<Self> {
        Self::for_world(resources, seed, "minecraft:normal", dimension, true)
    }
    /// A dimension of a world preset (`worldgen/world_preset`) whose
    /// `generate_structures` is `structures` (`ChunkGenerator.createStructures`
    /// and `findNearestMapStructure` do nothing without).
    pub fn for_world(
        resources: TerrainResources<'_>,
        seed: i64,
        preset: &str,
        dimension: &str,
        structures: bool,
    ) -> crate::Result<Self> {
        let none = WorldgenOverrides::default();
        Self::for_world_with(resources, &none, seed, preset, dimension, structures)
    }
    /// `for_world` with the world's data packs over the bundle at `dir`
    /// (`datapack::worldgen_overrides`).
    pub fn for_world_with(
        resources: TerrainResources<'_>,
        overrides: &WorldgenOverrides,
        seed: i64,
        preset: &str,
        dimension: &str,
        structures: bool,
    ) -> crate::Result<Self> {
        Self::for_world_with_settings(
            resources, overrides, seed, preset, dimension, structures, None, None,
        )
    }
    /// `for_world_with` with the saved dimension's generator and type, below
    /// any dimension supplied by the world's data packs, and its saved biome region layout.
    pub fn for_world_with_settings(
        resources: TerrainResources<'_>,
        overrides: &WorldgenOverrides,
        seed: i64,
        preset: &str,
        dimension: &str,
        structures: bool,
        saved_stem: Option<&serde_json::Value>,
        regions: Option<Arc<RegionLayout>>,
    ) -> crate::Result<Self> {
        let data: Arc<GeneratorData> = shared_resource_data(resources, overrides)?;
        let generator = OverworldGenerator::for_world_with_settings(
            data, seed, preset, dimension, structures, saved_stem, regions,
        )?;
        Self::assemble(resources, generator, seed, preset, dimension)
    }
    /// A superflat overworld with the structures its settings allow
    /// (`FlatLevelSource`: its layers, `structure_overrides` and the fixed biome).
    pub fn for_flat(
        resources: TerrainResources<'_>,
        overrides: &WorldgenOverrides,
        seed: i64,
        flat: &horizon_world::worldgen::generator::FlatSettings,
        structures: bool,
    ) -> crate::Result<Self> {
        let data: Arc<GeneratorData> = shared_resource_data(resources, overrides)?;
        let generator = OverworldGenerator::flat(data, seed, flat, structures)?;
        let mut terrain = Self::assemble(
            resources,
            generator,
            seed,
            "minecraft:normal",
            "minecraft:overworld",
        )?;
        terrain.flat = true;
        Ok(terrain)
    }
    /// A level of a mod's chunk generator that asks for vanilla stages: its terrain, then the
    /// stages, over the noise settings and biome source of `stem` (the level's, as a pack or
    /// the save gives it). `structures` is the world's `generate_structures`.
    pub fn for_mod(
        resources: TerrainResources<'_>,
        overrides: &WorldgenOverrides,
        seed: i64,
        dimension: &str,
        stem: &serde_json::Value,
        generator: Arc<dyn horizon_world::worldgen::mod_generators::ModChunkGenerator>,
        structures: bool,
    ) -> crate::Result<Self> {
        let data: Arc<GeneratorData> = shared_resource_data(resources, overrides)?;
        let generator =
            OverworldGenerator::for_mod(data, seed, dimension, stem, generator, structures)?;
        let mut terrain =
            Self::assemble(resources, generator, seed, "minecraft:normal", dimension)?;
        terrain.mod_terrain = true;
        Ok(terrain)
    }
    /// `ChunkGenerator.getSeaLevel`: the active generator's loaded setting.
    pub fn sea_level(&self) -> i32 {
        self.generator.sea_level()
    }
    /// Whether the terrain is the noise's: not a superflat world's layers or a mod's chunk
    /// generator's terrain, whose `spawnOriginalMobs` places nothing and whose far ring is not
    /// the noise's.
    pub fn noise_ground(&self) -> bool {
        !self.flat && !self.mod_terrain
    }
    /// `NoiseGeneratorSettings.disableMobGeneration`: whether the generator
    /// adds the creature packs of a freshly generated chunk. The End's preset
    /// is the one shipped setting that turns it on.
    pub fn disable_mob_generation(&self) -> bool {
        self.generator.disable_mob_generation()
    }
    /// The terrain around a generator, with the tables the client's chunks need.
    fn assemble(
        resources: TerrainResources<'_>,
        generator: OverworldGenerator,
        seed: i64,
        preset: &str,
        dimension: &str,
    ) -> crate::Result<Self> {
        let TerrainResources {
            data: _,
            dir,
            registry,
            biomes,
            item_data,
        } = resources;
        let biome_map = generator
            .data()
            .biome_names()
            .iter()
            .map(|name| biomes.id(name))
            .collect();
        if let Some((asked, kept)) = generator.noise_clamped() {
            log::warn!(
                "Horizon: {dimension}'s noise settings reach y {}..{}, past its build height; \
                 only y {}..{} generates",
                asked.min_y,
                asked.min_y + asked.height,
                kept.min_y,
                kept.min_y + kept.height
            );
        }
        let unsupported = generator.unsupported_features();
        if !unsupported.is_empty() {
            log::warn!(
                "Horizon: {} placed features cannot run and will be skipped",
                unsupported.len()
            );
            // Name them, so a pack's author can find what to fix; capped, so a broken pack can't flood the log.
            const NAMED: usize = 20;
            for (name, reason) in unsupported.iter().take(NAMED) {
                log::warn!("Horizon:   {name}: {reason}");
            }
            if unsupported.len() > NAMED {
                log::warn!("Horizon:   and {} more", unsupported.len() - NAMED);
            }
            if !dir.join("blocks/block-properties.nbt").exists() {
                log::warn!(
                    "Horizon: the worldgen bundle lacks the block behavior exports (worldgen/blocks/*.nbt); \
                     prepare again (the launcher, or horizon-prepare --prepare) to add them"
                );
            }
        }
        if let Some(root) = dir.parent() {
            crate::mobs::trial_spawner::load_registry(root);
        }
        let mut tick_blocks = Vec::with_capacity(registry.len());
        let mut tick_states = Vec::new();
        let mut block_entity_types = Vec::with_capacity(registry.len());
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
            block_entity_types.push(crate::lan::chunks::block_entity_type(name));
        }
        Ok(Self {
            generator,
            biome_map,
            structure_mobs: Arc::new(StructureMobFinalizer::new(
                dir.parent().unwrap_or(dir).to_owned(),
                dimension,
                Arc::clone(item_data),
            )),
            query: std::sync::Mutex::new(None),
            tick_blocks,
            tick_states,
            block_entity_types,
            seed,
            referenced: Default::default(),
            saved_checked: Default::default(),
            reference_file: Default::default(),
            saved_references: Default::default(),
            preset: preset.to_owned(),
            dimension: dimension.to_owned(),
            last_proto: std::sync::atomic::AtomicU64::new(0),
            created: std::time::Instant::now(),
            flat: false,
            mod_terrain: false,
        })
    }
    /// The column the dimension's chunks hold (its build range's).
    fn layout(&self) -> ColumnLayout {
        let (min_y, height) = self.generator.level();
        ColumnLayout::of_level(min_y, height)
    }
    /// The difficulty (0 peaceful to 3 hard) and overworld clock structure
    /// mobs are finalized with.
    pub fn set_spawn_conditions(&self, difficulty: u8, day_time: i64) {
        self.structure_mobs.set_conditions(difficulty, day_time);
    }
    /// Publish this world's builtin generation to future structure finalizations.
    pub fn set_mob_maps(&self, snapshot: &Arc<crate::neoforge_mob_maps::NeoForgeMobMaps>) {
        self.structure_mobs.set_mob_maps(snapshot);
    }
    /// Reads what generated mobs are finalized with on a thread of its own
    /// (`StructureMobFinalizer::load_behind`).
    pub fn load_structure_mobs_behind(&self) {
        self.structure_mobs.load_behind();
    }
    /// The random a chunk's generation steps draw from (`WorldGenRegion`).
    pub fn region_random(&self, cx: i32, cz: i32) -> Box<dyn PieceRandom> {
        self.generator.region_random(ChunkPos::new(cx, cz))
    }
    /// A mob chunk generation spawns, as the chunk saves it
    /// (`StructureMobFinalizer::spawn_generated`).
    pub fn spawn_generated(
        &self,
        kind: &str,
        position: horizon_core::geometry::Vec3,
        yaw: f32,
        biome: &str,
        random: &mut dyn PieceRandom,
        group: &mut Option<crate::mobs::finalize::Group>,
    ) -> Option<horizon_core::nbt::Compound> {
        self.structure_mobs
            .spawn_generated(kind, position, yaw, biome, random, group)
    }
    /// Only the generated range of a proto chunk (the Nether is 0..256)
    /// is copied into a column.
    fn generated_range(chunk: &ProtoChunk, out: &Chunk) -> (i32, i32) {
        (
            chunk.terrain.min_y().max(out.min_y()),
            (chunk.terrain.min_y() + chunk.terrain.height()).min(out.max_y()),
        )
    }
    /// A proto chunk's blocks alone as a column: what a light job reads of
    /// its neighbours.
    fn block_column(&self, chunk: &ProtoChunk) -> Chunk {
        let pos = chunk.position();
        let mut out = Chunk::with_layout(pos.x, pos.z, self.layout());
        let (bx, bz) = (pos.x * 16, pos.z * 16);
        let (lo, hi) = Self::generated_range(chunk, &out);
        let mut values = Box::new([0u32; 4096]);
        for si in 0..out.sections.len() as i32 {
            let base = out.min_y() + si * 16;
            if base + 16 <= lo || base >= hi {
                continue;
            }
            let mut any = false;
            // A whole generated section is read in one pass (it runs on the
            // server thread for every light view); a partly generated one
            // block by block.
            if base >= lo && base + 16 <= hi && chunk.terrain.section_states(base, &mut values) {
                any = values.iter().any(|&id| id != 0);
            } else {
                values.fill(0);
                for y in base.max(lo)..(base + 16).min(hi) {
                    for z in 0..16 {
                        for x in 0..16 {
                            let id = chunk.block(bx + x, y, bz + z).0;
                            values[((y & 15) << 8 | z << 4 | x) as usize] = id;
                            any |= id != 0;
                        }
                    }
                }
            }
            if any {
                Arc::make_mut(&mut out.sections[si as usize]).blocks =
                    Section::from_blocks(&values);
            }
        }
        out
    }
    /// A proto chunk's blocks, biomes, scheduled ticks and post-processing
    /// marks as a column.
    fn column(&self, chunk: &ProtoChunk) -> Chunk {
        let mut out = self.block_column(chunk);
        let pos = chunk.position();
        let (bx, bz) = (pos.x * 16, pos.z * 16);
        let (lo, hi) = Self::generated_range(chunk, &out);
        out.structure_refs = chunk
            .structures()
            .into_iter()
            .map(|(id, start, _, _)| (Arc::from(id), (start.x, start.z)))
            .collect();
        out.structure_starts = chunk.own_starts();
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
        if !block_ticks.is_empty() || !fluid_ticks.is_empty() || !chunk.saved_root.is_empty() {
            let attachments = horizon_mod_runtime::attachments::attachment_compound(
                chunk
                    .saved_root
                    .get(&horizon_mod_runtime::attachments::ATTACHMENTS_KEY.into()),
            )
            .ok()
            .flatten()
            .map(|data| horizon_mod_runtime::attachments::Attachments::from_nbt(data.clone()))
            .unwrap_or_default();
            out.saved = Some(Arc::new(crate::save::ChunkExtras {
                block_ticks,
                fluid_ticks,
                root: chunk.saved_root.clone(),
                attachments,
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
        for si in 0..out.sections.len() as i32 {
            let mut cells = [0u32; 64];
            for (j, cell) in cells.iter_mut().enumerate() {
                let (qx, qz, qy) = (j as i32 & 3, (j as i32 >> 2) & 3, si * 4 + (j as i32 >> 4));
                let y = (out.min_y() + qy * 4).clamp(lo, hi - 1);
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
    /// `/place structure`: `Structure.generate` for `structure` in chunk
    /// `(x, z)`, whatever the chunk's structure sets place there.
    pub fn structure_start(
        &self,
        structure: &str,
        x: i32,
        z: i32,
    ) -> Option<Arc<horizon_world::worldgen::structure::start::StructureStart>> {
        let mut worker = self.query.lock().unwrap_or_else(|e| e.into_inner());
        let worker = worker.get_or_insert_with(|| self.generator.worker());
        worker.structure_start(structure, ChunkPos::new(x, z))
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
    /// The chunks an older version saved, which new chunks blend with
    /// (`Blender.of`).
    pub fn set_old_world(&self, world: Arc<dyn horizon_world::worldgen::blending::OldWorld>) {
        self.generator.set_old_world(world);
    }
    /// The generator's resources, for what reads saved chunks the way it does.
    pub fn generator_data(&self) -> &Arc<GeneratorData> {
        self.generator.data()
    }
    /// `NoiseBasedChunkGenerator.addDebugScreenInfo`: F3's `Density` line at
    /// the feet. It reads the samplers only, not the shared query worker, so
    /// a long `/locate biome` or `/place jigsaw` never holds it up.
    pub fn density_debug_line(&self, feet: [i32; 3]) -> Option<String> {
        self.generator.debug_density_line(feet)
    }
    /// `ChunkGenerator.findNearestMapStructure` for an exploration map: with
    /// `skip_known` (`skip_existing_chunks`) the nearest start no map pointed
    /// at yet, which then counts as pointed at (`StructureStart.addReference`,
    /// at most one: `getMaxReferences`).
    pub fn locate_for_map(
        &self,
        names: &[String],
        from: [i32; 3],
        radius: i32,
        skip_known: bool,
    ) -> Option<(String, [i32; 3])> {
        if !skip_known {
            return self.locate(names, from, radius);
        }
        let saved = self
            .saved_references
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        // The skip check locks per candidate: `locate_skipping` runs it here
        // on the calling thread, and a `dyn Fn` cannot carry a guard.
        let found =
            self.generator
                .worker()
                .locate_skipping(names, from, radius, &|name, chunk| {
                    // This session's references first: they grow as maps resolve.
                    if self
                        .referenced
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .get(name)
                        .is_some_and(|chunks| chunks.contains(&(chunk.x, chunk.z)))
                    {
                        return true;
                    }
                    let mut checked = self.saved_checked.lock().unwrap_or_else(|e| e.into_inner());
                    if let Some(count) = checked
                        .get(name)
                        .and_then(|chunks| chunks.get(&(chunk.x, chunk.z)))
                    {
                        return *count > 0;
                    }
                    let count = saved.as_ref().map_or(0, |f| f(name, chunk.x, chunk.z));
                    checked
                        .entry(name.to_owned())
                        .or_default()
                        .insert((chunk.x, chunk.z), count);
                    count > 0
                })?;
        self.referenced
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(found.0.clone())
            .or_default()
            .insert((found.1[0] >> 4, found.1[2] >> 4));
        self.save_references();
        Some(found)
    }
    /// Where the starts maps pointed at are kept, and the ones kept there:
    /// vanilla keeps the count in each start's chunk (`references`), which a
    /// start nobody visited has no file for.
    pub fn set_reference_file(&self, path: std::path::PathBuf) {
        let mut referenced = self.referenced.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(list) = std::fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice::<Vec<(String, i32, i32)>>(&b).ok())
        {
            for (name, x, z) in list {
                referenced.entry(name).or_default().insert((x, z));
            }
        }
        *self
            .reference_file
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(path);
    }
    /// StructureCheck's `onStructureLoad` and `tryLoadFromStorage`: the
    /// references a chunk saved for its starts (a world saved by vanilla, or
    /// by an older run) count as known, besides the ones kept here.
    pub fn set_saved_references(&self, saved: SavedReferences) {
        *self
            .saved_references
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(saved);
    }
    fn save_references(&self) {
        let referenced = self.referenced.lock().unwrap_or_else(|e| e.into_inner());
        let guard = self
            .reference_file
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let Some(path) = guard.as_ref() else { return };
        let mut list: Vec<(String, i32, i32)> = Vec::new();
        for (name, chunks) in referenced.iter() {
            list.extend(chunks.iter().map(|&(x, z)| (name.clone(), x, z)));
        }
        list.sort();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        // Written beside the file and renamed over it: a crash half way must
        // not leave truncated JSON, which loads as no references at all.
        let tmp = path.with_extension("json.tmp");
        let saved = serde_json::to_vec(&list)
            .map_err(std::io::Error::from)
            .and_then(|b| std::fs::write(&tmp, b))
            .and_then(|()| std::fs::rename(&tmp, path));
        if let Err(e) = saved {
            log::warn!("Horizon: could not save {}: {e}", path.display());
        }
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
        let spawned = horizon_guest_stack::builder()
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
/// `overrides` with the content mods' biome modifiers over the packs' biomes
/// (`mod_content::biome_modifiers_over_packs`), for the worldgen bundle
/// `dir` of a content root, and those mods declared in code; `None` when its biomes stand as
/// they are.
pub fn biome_modifiers_over_packs(
    dir: &Path,
    overrides: &WorldgenOverrides,
) -> Option<WorldgenOverrides> {
    let root = dir.parent()?;
    // The modifiers code mods declared: they register after the content root is built.
    let declared: Vec<[String; 3]> = mod_api::__private::biome_modifier_declarations()
        .iter()
        .filter_map(|d| {
            let (mod_id, _) = d.id.split_once(':')?;
            Some([mod_id.to_owned(), d.file(), d.modifier.json()])
        })
        .collect();
    crate::mod_content::biome_modifiers_over_packs(root, overrides, &declared).unwrap_or_else(|e| {
        log::warn!("Horizon: biome modifiers over data packs: {e}");
        None
    })
}
/// A bundle directory, the data packs over it, and what loading them gave.
type SharedEntry = (std::path::PathBuf, WorldgenOverrides, Arc<GeneratorData>);
static SHARED_DATA: std::sync::Mutex<Vec<SharedEntry>> = std::sync::Mutex::new(Vec::new());
/// Bundles with data packs kept besides the plain ones: a world's dimensions
/// share one, and returning to a world finds it.
const SHARED_WITH_PACKS: usize = 2;
/// Let go of the bundles `shared_data` loaded: for a preload no world used
/// (game/world/world_loading.rs). A generator that holds one keeps it.
pub fn forget_shared_data() {
    SHARED_DATA
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clear();
}
fn shared_resource_data(
    resources: TerrainResources<'_>,
    overrides: &WorldgenOverrides,
) -> crate::Result<Arc<GeneratorData>> {
    match resources.data {
        Some(source) => shared_data_from(resources.dir, overrides, resources.registry, source),
        None => shared_data_with(resources.dir, overrides, resources.registry),
    }
}
/// Shared worldgen tables with lazy passive structure templates below world packs.
pub fn shared_data_from(
    dir: &Path,
    overrides: &WorldgenOverrides,
    registry: &'static BlockRegistry,
    source: &horizon_assets::resource_source::ResourceSource,
) -> crate::Result<Arc<GeneratorData>> {
    let mut resolved = overrides.clone();
    {
        for name in source.list_under("data", &["structure"]) {
            if let Some((namespace, path)) = name
                .strip_prefix("data/")
                .and_then(|rest| rest.split_once("/structure/"))
                .and_then(|(ns, path)| Some((ns, path.strip_suffix(".nbt")?)))
            {
                let id = format!("{namespace}:{path}");
                if !resolved.templates().contains_key(&id) {
                    resolved.set_template(
                        id,
                        horizon_assets::resource_source::Resource::file(source.clone(), name),
                    );
                }
            }
        }
    }
    shared_data_with(dir, &resolved, registry)
}
/// The worldgen bundle at `dir`, loaded once and shared by every generator
/// and by live feature placement (sapling growth, bone meal).
pub fn shared_data(
    dir: &Path,
    registry: &'static BlockRegistry,
) -> crate::Result<Arc<GeneratorData>> {
    shared_data_with(dir, &WorldgenOverrides::default(), registry)
}
/// `shared_data` with a world's data packs over the bundle: loaded once for
/// each registry lifetime and set of packs' entries, the most recent few kept.
#[horizon_mod_api_macros::replaceable]
pub fn shared_data_with(
    dir: &Path,
    overrides: &WorldgenOverrides,
    registry: &'static BlockRegistry,
) -> crate::Result<Arc<GeneratorData>> {
    let mut cache = SHARED_DATA.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, _, data)) = cache.iter().find(|(path, o, data)| {
        path == dir && o == overrides && std::ptr::eq(data.registry(), registry)
    }) {
        return Ok(data.clone());
    }
    let started = std::time::Instant::now();
    let modified = biome_modifiers_over_packs(dir, overrides);
    let data = GeneratorData::load_with(dir, registry, modified.as_ref().unwrap_or(overrides))?;
    log::info!("Horizon: worldgen data loaded in {:?}", started.elapsed());
    for warning in data.warnings() {
        log::warn!("Horizon: data pack worldgen: {warning}");
    }
    if !overrides.is_empty()
        && cache.iter().filter(|e| !e.1.is_empty()).count() >= SHARED_WITH_PACKS
    {
        let oldest = cache.iter().position(|e| !e.1.is_empty());
        cache.remove(oldest.expect("counted"));
    }
    cache.push((dir.to_owned(), overrides.clone(), data.clone()));
    Ok(data)
}
impl ChunkGenerator for VanillaTerrain {
    type Worker = GeneratorWorker;
    fn set_saved_starts(
        &self,
        source: Arc<horizon_world::worldgen::generator::SavedStructureStarts>,
    ) {
        self.generator.set_saved_starts(source);
    }
    fn worker(&self) -> GeneratorWorker {
        let mut worker = self.generator.worker();
        worker.set_structure_mobs(self.structure_mobs.clone());
        worker
    }
    fn lod_sampler(self: Arc<Self>) -> Box<dyn LodSampler> {
        if self.mod_terrain {
            return Box::new(NoFarGround);
        }
        Box::new(VanillaLodSampler::new(self))
    }
    fn idle(&self, worker: &mut GeneratorWorker) {
        worker.release();
    }
    fn all_idle(&self) {
        self.generator.release_caches();
    }
    fn lod_surface(
        &self,
        worker: &mut GeneratorWorker,
        x: i32,
        z: i32,
    ) -> Option<crate::lod::Surface> {
        if self.mod_terrain {
            return None;
        }
        Some(self.client_surface(worker.surface_sample(x, z)?))
    }
    /// The structure starts a saved chunk references, as vanilla reads them
    /// with the chunk: read from the owning chunk's save (or regenerated for
    /// unsupported pieces) here, on the loading worker, into the
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
            Ok(chunk) => Box::new(VanillaProto::Ready(Box::new(ResidentProto {
                terrain_chunk: chunk,
                saved_biomes: None,
            }))),
            Err(e) => {
                log::warn!("Horizon: generating chunk {x} {z} failed: {e}");
                Box::new(VanillaProto::Failed(x, z))
            }
        }
    }
    fn decorates(&self) -> bool {
        true
    }
    fn decorates_beside_final(&self) -> bool {
        true
    }
    /// A final chunk's proto chunk: terrain generated again with the final
    /// chunk's blocks over it, so that features read and write it as they do
    /// the chunks around it (`ChunkAccess` of a full chunk in the region).
    fn proto_beside_final(&self, worker: &mut GeneratorWorker, snapshot: &Chunk) -> Option<Proto> {
        let mut proto = self.proto(worker, snapshot.x, snapshot.z);
        self.restore_proto(
            &mut proto,
            SavedProto {
                column: snapshot.clone(),
                decorated: true,
                block_entities: Vec::new(),
                entities: Vec::new(),
            },
        );
        // ChunkStatusTasks.full / SerializableChunkData.read: a full chunk stands in
        // the region as ImposterProtoChunk(chunk, false).
        if let Some(VanillaProto::Ready(chunk)) = proto.downcast_mut::<VanillaProto>() {
            chunk.set_full();
        }
        Some(proto)
    }
    fn final_writes(&self, snapshot: &Chunk, proto: &Proto) -> Vec<([i32; 3], u32)> {
        let Some(VanillaProto::Ready(chunk)) = proto.downcast_ref::<VanillaProto>() else {
            return Vec::new();
        };
        let (bx, bz) = (snapshot.x * 16, snapshot.z * 16);
        let (lo, hi) = (
            chunk.terrain.min_y().max(snapshot.min_y()),
            (chunk.terrain.min_y() + chunk.terrain.height()).min(snapshot.max_y()),
        );
        let mut writes = Vec::new();
        for y in lo..hi {
            for z in 0..16 {
                for x in 0..16 {
                    let state = chunk.block(bx + x, y, bz + z).0;
                    if state != snapshot.get(x, y, z) {
                        writes.push(([bx + x, y, bz + z], state));
                    }
                }
            }
        }
        writes
    }
    fn final_block_entities(
        &self,
        entries: &[(&Proto, &[([i32; 3], u32)])],
    ) -> Vec<((i32, i32, i32), crate::chunk::block_entity::BlockEntity)> {
        if !entries.iter().any(|(proto, _)| {
            matches!(proto.downcast_ref::<VanillaProto>(), Some(VanillaProto::Ready(chunk)) if !chunk.block_entities.is_empty())
        }) {
            return Vec::new();
        }
        entries
            .iter()
            .flat_map(|(proto, writes)| {
                let Some(VanillaProto::Ready(chunk)) = proto.downcast_ref::<VanillaProto>() else {
                    return Vec::new();
                };
                let (made, _) = crate::structure_blocks::block_entities(chunk);
                made.into_iter()
                    .filter(|((x, y, z), _)| writes.iter().any(|(p, _)| *p == [*x, *y, *z]))
                    .collect()
            })
            .collect()
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
            let (chunks, saved_biomes): (Vec<ProtoChunk>, Vec<Option<SavedBiomeCells>>) = taken
                .into_iter()
                .map(|p| match *p {
                    VanillaProto::Ready(c) => (c.terrain_chunk, c.saved_biomes),
                    VanillaProto::Failed(..) => unreachable!(),
                })
                .unzip();
            let mut array: [ProtoChunk; 9] = match chunks.try_into() {
                Ok(a) => a,
                Err(_) => unreachable!(),
            };
            if let Err(e) = worker.decorate(&mut array) {
                log::warn!("Horizon: decorating chunk failed: {e}");
            }
            for ((slot, chunk), saved_biomes) in region.iter_mut().zip(array).zip(saved_biomes) {
                *slot = Box::new(VanillaProto::Ready(Box::new(ResidentProto {
                    terrain_chunk: chunk,
                    saved_biomes,
                })));
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
    fn proto_status(&self, proto: &Proto) -> Option<horizon_world::chunk_status::ChunkStatus> {
        let VanillaProto::Ready(chunk) = proto.downcast_ref::<VanillaProto>()? else {
            return None;
        };
        Some(if chunk.is_decorated() {
            horizon_world::chunk_status::ChunkStatus::Features
        } else {
            horizon_world::chunk_status::ChunkStatus::Terrain
        })
    }
    fn proto_block(&self, proto: &Proto, [x, y, z]: [i32; 3]) -> Option<u32> {
        let VanillaProto::Ready(chunk) = proto.downcast_ref::<VanillaProto>()? else {
            return None;
        };
        let position = chunk.position();
        if x.div_euclid(16) != position.x
            || z.div_euclid(16) != position.z
            || y < chunk.terrain.min_y()
            || y >= chunk.terrain.min_y() + chunk.terrain.height()
        {
            return None;
        }
        Some(chunk.block(x, y, z).0)
    }
    fn light_view(&self, proto: &Proto) -> Option<Chunk> {
        match proto.downcast_ref::<VanillaProto>() {
            Some(VanillaProto::Ready(chunk)) => Some(self.block_column(chunk)),
            _ => None,
        }
    }
    fn cache_census(&self) -> Vec<(&'static str, usize, Option<usize>)> {
        let c = self.generator.cache_census();
        vec![
            ("biome_chunks", c.biome_chunks, Some(c.biome_bytes)),
            ("carver_biomes", c.carver_entries, None),
            ("start_chunks", c.start_chunks, None),
            ("start_lists", c.start_lists, None),
        ]
    }
    fn proto_heap_bytes(&self, proto: &Proto) -> ([usize; 6], [usize; 16]) {
        match proto.downcast_ref::<VanillaProto>() {
            Some(VanillaProto::Ready(chunk)) => (chunk.heap_bytes(), chunk.terrain.storage_bits()),
            _ => ([0; 6], [0; 16]),
        }
    }
    fn non_air_sections(&self, proto: &Proto) -> Option<crate::section_mask::SectionMask> {
        let Some(VanillaProto::Ready(chunk)) = proto.downcast_ref::<VanillaProto>() else {
            return None;
        };
        // The proto's sections from its own minimum Y; the column's from the
        // layout's.
        let layout = self.layout();
        let below = (chunk.terrain.min_y() - layout.min_y()) >> 4;
        let mut mask = crate::section_mask::SectionMask::EMPTY;
        for i in chunk.terrain.filled_sections() {
            if let Ok(i) = usize::try_from(i as i32 + below)
                && i < layout.sections()
            {
                mask.insert(i);
            }
        }
        Some(mask)
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
                // Placed data can lack its type (a brushable block's loot table
                // alone); vanilla saves every pending block entity with its id,
                // and skips one without when the chunk finishes.
                if let Some(id) =
                    crate::lan::chunks::block_entity_type(chunk.block_name(pos[0], pos[1], pos[2]))
                {
                    nbt.insert("id".into(), Tag::String(id.into()));
                }
                for (key, v) in ["x", "y", "z"].into_iter().zip(pos) {
                    nbt.insert(key.into(), Tag::Int(*v));
                }
                Tag::Compound(nbt)
            })
            .collect();
        Some(SavedProto {
            column: chunk.column(self),
            decorated: chunk.is_decorated(),
            block_entities,
            entities: chunk.entities.iter().cloned().map(Tag::Compound).collect(),
        })
    }
    /// Restore saved blocks, biome cells and decoration state. Structure lookups
    /// use the current generator; completed Features decoration stays done.
    fn restore_proto(&self, proto: &mut Proto, saved: SavedProto) {
        let Some(VanillaProto::Ready(chunk)) = proto.downcast_mut::<VanillaProto>() else {
            return;
        };
        let column = &saved.column;
        chunk.saved_biomes = Some(SavedBiomeCells::capture(column));
        let (bx, bz) = (column.x * 16, column.z * 16);
        let (lo, hi) = (
            chunk.terrain.min_y().max(column.min_y()),
            (chunk.terrain.min_y() + chunk.terrain.height()).min(column.max_y()),
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
        chunk.saved_root = column
            .saved
            .as_deref()
            .map_or_else(Default::default, |e| e.root.clone());
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
            VanillaProto::Failed(x, z) => return Chunk::with_layout(x, z, self.layout()),
        };
        let mut out = chunk.column(self);
        let (block_entities, pending_loot) = crate::structure_blocks::block_entities(&chunk);
        out.block_entities = block_entities;
        // The block entities Horizon does not model (decorated pots with their
        // sherds and loot table) stay as placed, like a
        // loaded chunk's.
        let mut unmodelled: Vec<Tag> = chunk
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
        add_default_block_entities(&self.block_entity_types, &mut out, &mut unmodelled);
        if !unmodelled.is_empty() || !pending_loot.is_empty() {
            let mut extras = out
                .saved
                .take()
                .map(Arc::unwrap_or_clone)
                .unwrap_or_default();
            extras.block_entities = unmodelled;
            extras.pending_loot = pending_loot;
            out.saved = Some(Arc::new(extras));
        }
        let entities = std::mem::take(&mut chunk.entities);
        // Chest minecarts keep their loot table until first inventory access.
        out.entities = entities
            .into_iter()
            .map(horizon_core::nbt::Tag::Compound)
            .collect();
        for s in &mut out.sections {
            Arc::make_mut(s).compact();
        }
        out.fresh = true;
        out
    }
}

#[cfg(test)]
mod default_block_entity_tests {
    use super::*;

    #[test]
    fn generated_blocks_get_their_block_entities() {
        let mut out = Chunk::empty(-2, 3);
        out.set(4, -40, 9, 7);
        out.set(5, -40, 9, 6);
        let mut kinds = vec![None; 8];
        kinds[6] = Some("minecraft:ender_chest");
        kinds[7] = Some("minecraft:sculk_sensor");
        let mut raw = vec![];
        add_default_block_entities(&kinds, &mut out, &mut raw);
        // Both are modelled now: an ender chest's `EnderChestBlock.newBlockEntity` is
        // an entity, so a generated one reads back like a placed chest.
        assert!(matches!(
            out.block_entities[..],
            [
                ((-28, -40, 57), crate::containers::BlockEntity::Sculk(_)),
                ((-27, -40, 57), crate::containers::BlockEntity::Chest(_))
            ]
        ));
        assert!(raw.is_empty());
        add_default_block_entities(&kinds, &mut out, &mut raw);
        assert_eq!(
            (out.block_entities.len(), raw.len()),
            (2, 0),
            "a placed block entity is kept, not doubled"
        );
    }
}

#[cfg(test)]
mod saved_biome_tests;
