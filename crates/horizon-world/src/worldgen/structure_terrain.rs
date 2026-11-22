//! Structure placement on a terrain that isn't the noise generator's.
//!
//! Structure starts are worked out before a chunk's blocks exist, from the
//! biome source and the noise router's heights (`ChunkGenerator.createStructures`).
//! A generator that builds its own land afterwards (a mod's `chunk_generate`)
//! disagrees with them: a desert pyramid in its forest, a village over its
//! valley, a monument on its dry land. This lets that generator say what its land
//! is, so the starts are worked out on it.
//!
//! A [`Provider`] is asked about each chunk whose starts are being built, once
//! (the starts are kept), and about `/place structure`. The first answer that
//! isn't [`Answer::Vanilla`] is used:
//!
//! - [`Answer::Vanilla`]: the biome source and the noise heights, as always.
//! - [`Answer::NoStructures`]: no structure starts in the chunk (pieces of a start
//!   in another chunk may still reach into it).
//! - [`Answer::Terrain`]: the starts are built with every biome and height
//!   question answered by that [`Terrain`]: which biomes a start finds
//!   (`Structure.findGenerationPoint`'s biome check, jigsaw pieces' column checks,
//!   the stronghold rings' biome search), the heightmaps a start and its pieces
//!   stand on, and the noise column structures that test blocks read (monuments,
//!   ruined portals). The seed, the old chunks' saved starts and whether a biome
//!   is cold enough to snow still come from the generator.
//!
//! With no provider registered nothing changes: a chunk whose starts are built
//! costs one check of [`any_registered`], and no request is made.
//!
//! A chunk that already has saved starts keeps them whatever a provider answers
//! ([`Answer::NoStructures`] only stops new starts).

pub use super::anchor::WorldGenerationContext;
use super::biome::BiomeId;
use super::structure::jigsaw::JigsawContext;
use super::structure::start::{NoiseBlock, NoiseColumnTarget, StartContext};
use crate::heightmap::HeightmapType;
pub use horizon_core::position::ChunkPos;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock, RwLock};

/// A block of a provider's terrain, as structure placement tells them apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Block {
    Air,
    Water,
    Lava,
    /// Anything a structure stands on (`BlockState.isSolid`).
    Solid,
}

/// One column of a provider's terrain.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Column {
    /// The top solid block's y.
    pub surface: i32,
    /// The top water block's y: water fills from `surface + 1` up to it. At or
    /// under `surface` for a dry column.
    pub water_top: i32,
    /// The biome's registry name (`minecraft:plains`).
    pub biome: &'static str,
}

impl Column {
    /// Ground at `surface`, air above.
    pub fn dry(surface: i32, biome: &'static str) -> Column {
        Column {
            surface,
            water_top: i32::MIN,
            biome,
        }
    }
    /// Ground at `surface`, water above it up to `water_top`.
    pub fn with_water(surface: i32, water_top: i32, biome: &'static str) -> Column {
        Column {
            surface,
            water_top,
            biome,
        }
    }
    /// Solid up to the surface, then water, then air.
    pub fn block_at(&self, y: i32) -> Block {
        if y <= self.surface {
            Block::Solid
        } else if y <= self.water_top {
            Block::Water
        } else {
            Block::Air
        }
    }
}

/// A terrain structures are placed on. Asked on generation workers, many times a
/// chunk: it has to be fast, take no lock the server thread holds, and give the
/// same answer for the same position every time.
pub trait Terrain: Send + Sync {
    fn column(&self, x: i32, z: i32) -> Column;
    /// The block at `x y z` of `column` (`column(x, z)`): overhangs, floating
    /// land. Heights scan down from the top of the level, so the highest
    /// counting block is the ground.
    fn block(&self, x: i32, y: i32, z: i32, column: &Column) -> Block {
        let _ = (x, z);
        column.block_at(y)
    }
    /// The biome at `x y z` of `column`: underground biomes. Sampled at the
    /// middle of each quart, as the biome source is.
    fn biome(&self, x: i32, y: i32, z: i32, column: &Column) -> &'static str {
        let _ = (x, y, z);
        column.biome
    }
}

/// Which of vanilla's dimensions a request looks like, from its structures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dimension {
    Overworld,
    Nether,
    End,
    Other,
}

/// What a provider is asked.
pub struct Request<'a> {
    seed: i64,
    chunk: ChunkPos,
    placing: Option<&'a str>,
    generation: WorldGenerationContext,
    structures: Vec<&'a str>,
    biomes: Option<&'a BiomeNames>,
}

impl<'a> Request<'a> {
    /// A request: Horizon makes one per chunk, a provider's tests their own.
    /// `structures` are the ones the dimension can generate, `biomes` its
    /// registry's biome names by id (None: every name counts as known).
    pub fn new(
        seed: i64,
        chunk: ChunkPos,
        placing: Option<&'a str>,
        generation: WorldGenerationContext,
        structures: Vec<&'a str>,
        biomes: Option<&'a BiomeNames>,
    ) -> Request<'a> {
        Request {
            seed,
            chunk,
            placing,
            generation,
            structures,
            biomes,
        }
    }
    pub fn seed(&self) -> i64 {
        self.seed
    }
    pub fn chunk(&self) -> ChunkPos {
        self.chunk
    }
    /// The structure `/place structure` asks for in this chunk, whatever its
    /// biome; None while generating.
    pub fn placing(&self) -> Option<&str> {
        self.placing
    }
    /// The generator's minimum y, height and sea level.
    pub fn generation(&self) -> WorldGenerationContext {
        self.generation
    }
    /// The structures this dimension's sets can start, by name.
    pub fn structures(&self) -> &[&str] {
        &self.structures
    }
    pub fn can_generate(&self, structure: &str) -> bool {
        self.structures.contains(&structure)
    }
    /// Which dimension this looks like. Every dimension's chunks are asked
    /// about, so a provider for one checks it.
    pub fn dimension(&self) -> Dimension {
        let any = |names: &[&str]| names.iter().any(|name| self.can_generate(name));
        if any(&[
            "minecraft:fortress",
            "minecraft:bastion_remnant",
            "minecraft:nether_fossil",
        ]) {
            Dimension::Nether
        } else if any(&["minecraft:end_city"]) {
            Dimension::End
        } else if any(&[
            "minecraft:mineshaft",
            "minecraft:village_plains",
            "minecraft:stronghold",
            "minecraft:monument",
            "minecraft:trial_chambers",
        ]) {
            Dimension::Overworld
        } else {
            Dimension::Other
        }
    }
    /// Whether the dimension's biome registry has `name`: a terrain that names
    /// a biome the game version lacks answers its stand-in instead.
    pub fn has_biome(&self, name: &str) -> bool {
        self.biomes
            .is_none_or(|biomes| biomes.ids.contains_key(name))
    }
}

/// A provider's answer for one chunk.
#[derive(Clone)]
pub enum Answer {
    Vanilla,
    NoStructures,
    Terrain(Arc<dyn Terrain>),
}

/// Decides chunk by chunk whether starts are built on a terrain. Asked on
/// generation workers, once for each chunk's starts, which are then kept: the
/// answer for a chunk must not change while a level is open.
pub trait Provider: Send + Sync {
    fn answer(&self, request: &Request<'_>) -> Answer;
}

type Providers = Arc<Vec<(String, Arc<dyn Provider>)>>;

fn providers() -> &'static RwLock<Providers> {
    static PROVIDERS: OnceLock<RwLock<Providers>> = OnceLock::new();
    PROVIDERS.get_or_init(|| RwLock::new(Arc::new(Vec::new())))
}

/// Adds `provider` under `id`, or replaces the one registered under it in its
/// place. Providers are asked in the order they were first registered.
pub fn register(id: &str, provider: Arc<dyn Provider>) {
    let mut lock = providers().write().unwrap_or_else(|e| e.into_inner());
    let mut list = (**lock).clone();
    match list.iter_mut().find(|(each, _)| each == id) {
        Some(entry) => entry.1 = provider,
        None => list.push((id.to_owned(), provider)),
    }
    *lock = Arc::new(list);
}

/// Removes the provider registered under `id`; whether there was one.
pub fn unregister(id: &str) -> bool {
    let mut lock = providers().write().unwrap_or_else(|e| e.into_inner());
    let mut list = (**lock).clone();
    let before = list.len();
    list.retain(|(each, _)| each != id);
    let removed = list.len() != before;
    *lock = Arc::new(list);
    removed
}

/// Whether any provider is registered: with none, every answer is [`Answer::Vanilla`].
pub fn any_registered() -> bool {
    !providers()
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .is_empty()
}

/// The registered ids, in the order they are asked.
pub fn registered() -> Vec<String> {
    let list = providers()
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    list.iter().map(|(id, _)| id.clone()).collect()
}

/// The first answer that isn't [`Answer::Vanilla`].
pub fn answer(request: &Request<'_>) -> Answer {
    // A copy of the list, so a provider may register while it answers.
    let list = providers()
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    for (_, provider) in list.iter() {
        match provider.answer(request) {
            Answer::Vanilla => continue,
            other => return other,
        }
    }
    Answer::Vanilla
}

/// A generator's biome registry, by name, for a terrain's names.
pub struct BiomeNames {
    ids: HashMap<Arc<str>, BiomeId>,
    /// What a name the registry lacks becomes: plains, else the first biome.
    fallback: BiomeId,
}

impl BiomeNames {
    /// From the registry's names, indexed by id (`GeneratorData::biome_names`).
    pub fn new(names: &[Arc<str>]) -> BiomeNames {
        let ids: HashMap<_, _> = names
            .iter()
            .enumerate()
            .map(|(i, n)| (n.clone(), BiomeId(i as u32)))
            .collect();
        let fallback = ids.get("minecraft:plains").copied().unwrap_or(BiomeId(0));
        BiomeNames { ids, fallback }
    }
    /// `name`'s id. A name the registry lacks is logged once and placed as
    /// plains, so a typo shows rather than quietly stopping every start.
    fn id(&self, name: &str) -> BiomeId {
        if let Some(id) = self.ids.get(name) {
            return *id;
        }
        static WARNED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
        let warned = WARNED.get_or_init(|| Mutex::new(HashSet::new()));
        if warned
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(name.to_owned())
        {
            log::warn!("structure terrain: biome {name:?} isn't in the registry; placed as plains");
        }
        self.fallback
    }
}

/// The generator's start context, with the biome and height questions answered
/// by a [`Terrain`].
pub(super) struct LandContext<'a> {
    inner: &'a mut dyn StartContext,
    terrain: Arc<dyn Terrain>,
    names: &'a BiomeNames,
    min_y: i32,
    max_y: i32,
    valid: Option<BTreeSet<BiomeId>>,
    columns: HashMap<(i32, i32), Column>,
    heights: HashMap<(i32, i32, bool), i32>,
}

/// Entries the per-call caches keep before starting over.
const CACHE: usize = 1 << 16;

impl<'a> LandContext<'a> {
    pub(super) fn new(
        inner: &'a mut dyn StartContext,
        terrain: Arc<dyn Terrain>,
        names: &'a BiomeNames,
    ) -> LandContext<'a> {
        let (min_y, max_y) = (inner.min_y(), inner.max_y());
        LandContext {
            inner,
            terrain,
            names,
            min_y,
            max_y,
            valid: None,
            columns: HashMap::new(),
            heights: HashMap::new(),
        }
    }
    fn column(&mut self, x: i32, z: i32) -> Column {
        if let Some(column) = self.columns.get(&(x, z)) {
            return *column;
        }
        if self.columns.len() >= CACHE {
            self.columns.clear();
        }
        let column = self.terrain.column(x, z);
        self.columns.insert((x, z), column);
        column
    }
    fn block(&mut self, x: i32, y: i32, z: i32) -> Block {
        let column = self.column(x, z);
        self.terrain.block(x, y, z, &column)
    }
    fn quart_biome(&mut self, [qx, qy, qz]: [i32; 3]) -> BiomeId {
        let (x, y, z) = (qx * 4 + 2, qy * 4 + 2, qz * 4 + 2);
        let column = self.column(x, z);
        self.names.id(self.terrain.biome(x, y, z, &column))
    }
    /// The ocean floor heightmaps count solid blocks; the others fluids too
    /// (`Heightmap.Types`' predicates on these four kinds of block).
    fn counts(kind: HeightmapType, block: Block) -> bool {
        match kind {
            HeightmapType::OceanFloorWg | HeightmapType::OceanFloor => block == Block::Solid,
            _ => block != Block::Air,
        }
    }
}

impl JigsawContext for LandContext<'_> {
    fn min_y(&self) -> i32 {
        self.min_y
    }
    fn max_y(&self) -> i32 {
        self.max_y
    }
    fn base_height(&mut self, x: i32, z: i32, kind: HeightmapType) -> i32 {
        let floor = matches!(
            kind,
            HeightmapType::OceanFloorWg | HeightmapType::OceanFloor
        );
        if let Some(height) = self.heights.get(&(x, z, floor)) {
            return *height;
        }
        if self.heights.len() >= CACHE {
            self.heights.clear();
        }
        let mut height = self.min_y;
        for y in (self.min_y..=self.max_y).rev() {
            if Self::counts(kind, self.block(x, y, z)) {
                height = y + 1;
                break;
            }
        }
        self.heights.insert((x, z, floor), height);
        height
    }
    fn column_has_valid_biome(&mut self, x: i32, z: i32, min_y: i32, max_y: i32) -> bool {
        let (qmin, qmax) = (min_y >> 2, max_y >> 2);
        if qmin > qmax {
            return false;
        }
        let Some(valid) = self.valid.clone() else {
            return true;
        };
        (qmin..=qmax).any(|qy| valid.contains(&self.quart_biome([x >> 2, qy, z >> 2])))
    }
}

impl StartContext for LandContext<'_> {
    fn is_old_chunk(&self, chunk: ChunkPos) -> bool {
        self.inner.is_old_chunk(chunk)
    }
    fn saved_starts(&self, chunk: ChunkPos) -> Vec<super::blending::SavedStart> {
        self.inner.saved_starts(chunk)
    }
    fn seed(&self) -> i64 {
        self.inner.seed()
    }
    fn sea_level(&self) -> i32 {
        self.inner.sea_level()
    }
    fn generator_min_y(&self) -> i32 {
        self.inner.generator_min_y()
    }
    fn base_column(&mut self, x: i32, z: i32, kind: HeightmapType) -> Vec<bool> {
        (self.min_y..=self.max_y)
            .map(|y| Self::counts(kind, self.block(x, y, z)))
            .collect()
    }
    fn base_columns_by(
        &mut self,
        x: i32,
        z: i32,
        count: usize,
        kind: &dyn Fn(usize) -> HeightmapType,
    ) -> Vec<Vec<bool>> {
        let blocks: Vec<Block> = (self.min_y..=self.max_y)
            .map(|y| self.block(x, y, z))
            .collect();
        (0..count)
            .map(|index| {
                blocks
                    .iter()
                    .map(|&block| Self::counts(kind(index), block))
                    .collect()
            })
            .collect()
    }
    fn noise_column_into(&mut self, x: i32, z: i32, column: &mut dyn NoiseColumnTarget) {
        column.resize(self.min_y, (self.max_y - self.min_y + 1) as usize);
        for (index, y) in (self.min_y..=self.max_y).enumerate() {
            let block = match self.block(x, y, z) {
                Block::Air => NoiseBlock::Air,
                Block::Water => NoiseBlock::Water,
                Block::Lava => NoiseBlock::Lava,
                Block::Solid => NoiseBlock::Solid,
            };
            column.set(index, block);
        }
    }
    fn cold_enough_to_snow(&mut self, biome: BiomeId, pos: [i32; 3]) -> bool {
        self.inner.cold_enough_to_snow(biome, pos)
    }
    fn generation(&self) -> WorldGenerationContext {
        self.inner.generation()
    }
    fn noise_biome(&mut self, quart: [i32; 3]) -> BiomeId {
        self.quart_biome(quart)
    }
    fn noise_biomes(&mut self, quarts: &[[i32; 3]]) -> Vec<BiomeId> {
        quarts
            .iter()
            .map(|&quart| self.quart_biome(quart))
            .collect()
    }
    fn set_valid_biomes(&mut self, biomes: Option<&BTreeSet<BiomeId>>) {
        self.valid = biomes.cloned();
        self.inner.set_valid_biomes(biomes);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worldgen::structure::start::NoiseColumn;

    /// The generator's side: level -64..319, sea 63; biome 7 is cold.
    struct Generator;
    impl JigsawContext for Generator {
        fn min_y(&self) -> i32 {
            -64
        }
        fn max_y(&self) -> i32 {
            319
        }
        fn base_height(&mut self, _: i32, _: i32, _: HeightmapType) -> i32 {
            unreachable!("answered by the terrain")
        }
        fn column_has_valid_biome(&mut self, _: i32, _: i32, _: i32, _: i32) -> bool {
            unreachable!("answered by the terrain")
        }
    }
    impl StartContext for Generator {
        fn seed(&self) -> i64 {
            42
        }
        fn sea_level(&self) -> i32 {
            63
        }
        fn base_column(&mut self, _: i32, _: i32, _: HeightmapType) -> Vec<bool> {
            unreachable!("answered by the terrain")
        }
        fn base_columns_by(
            &mut self,
            _: i32,
            _: i32,
            _: usize,
            _: &dyn Fn(usize) -> HeightmapType,
        ) -> Vec<Vec<bool>> {
            unreachable!("answered by the terrain")
        }
        fn noise_column_into(&mut self, _: i32, _: i32, _: &mut dyn NoiseColumnTarget) {
            unreachable!("answered by the terrain")
        }
        fn cold_enough_to_snow(&mut self, biome: BiomeId, _: [i32; 3]) -> bool {
            biome == BiomeId(7)
        }
        fn generation(&self) -> WorldGenerationContext {
            WorldGenerationContext::new(-64, 384, 63)
        }
        fn noise_biome(&mut self, _: [i32; 3]) -> BiomeId {
            unreachable!("answered by the terrain")
        }
        fn noise_biomes(&mut self, _: &[[i32; 3]]) -> Vec<BiomeId> {
            unreachable!("answered by the terrain")
        }
        fn set_valid_biomes(&mut self, _: Option<&BTreeSet<BiomeId>>) {}
    }

    /// Ocean (floor 40) west of x 0, desert rising eastward, an island floating
    /// at y 150..=155 over x 100..110, deep dark under y -20.
    struct Land;
    impl Terrain for Land {
        fn column(&self, x: i32, _: i32) -> Column {
            if x < 0 {
                Column::with_water(40, 63, "minecraft:ocean")
            } else {
                Column::dry(70 + x / 10, "minecraft:desert")
            }
        }
        fn block(&self, x: i32, y: i32, _: i32, column: &Column) -> Block {
            if (100..110).contains(&x) && (150..=155).contains(&y) {
                Block::Solid
            } else {
                column.block_at(y)
            }
        }
        fn biome(&self, _: i32, y: i32, _: i32, column: &Column) -> &'static str {
            if y < -20 {
                "minecraft:deep_dark"
            } else {
                column.biome
            }
        }
    }

    fn names() -> BiomeNames {
        let names = [
            "minecraft:plains",
            "minecraft:ocean",
            "minecraft:desert",
            "minecraft:deep_dark",
        ];
        BiomeNames::new(&names.map(Arc::<str>::from))
    }

    #[test]
    fn starts_see_the_terrains_biomes_and_heights() {
        let names = names();
        let mut generator = Generator;
        let mut land = LandContext::new(&mut generator, Arc::new(Land), &names);
        // Water counts for the surface heightmaps, not for the ocean floor's.
        assert_eq!(land.base_height(-5, 0, HeightmapType::WorldSurfaceWg), 64);
        assert_eq!(land.base_height(-5, 0, HeightmapType::OceanFloorWg), 41);
        assert_eq!(land.base_height(20, 0, HeightmapType::WorldSurfaceWg), 73);
        // The floating island is the ground there.
        assert_eq!(land.base_height(105, 0, HeightmapType::MotionBlocking), 156);
        let columns = land.base_columns_by(105, 0, 2, &|i| {
            [HeightmapType::WorldSurfaceWg, HeightmapType::OceanFloorWg][i]
        });
        assert_eq!(columns[0].len(), 384);
        assert!(columns[0][(80 + 64) as usize] && !columns[0][(120 + 64) as usize]);
        assert!(columns[1][(152 + 64) as usize]);
        let mut noise = NoiseColumn::default();
        land.noise_column_into(-5, 0, &mut noise);
        assert_eq!(
            [noise.get(40), noise.get(50), noise.get(70)],
            [NoiseBlock::Solid, NoiseBlock::Water, NoiseBlock::Air]
        );
        // Biomes by registry id, from the quart's middle, underground ones included.
        assert_eq!(land.noise_biome([-3, 16, 0]), BiomeId(1));
        assert_eq!(
            land.noise_biomes(&[[3, 16, 0], [3, -12, 0]]),
            vec![BiomeId(2), BiomeId(3)]
        );
        land.set_valid_biomes(Some(&BTreeSet::from([BiomeId(2)])));
        assert!(land.column_has_valid_biome(20, 0, 60, 80));
        assert!(!land.column_has_valid_biome(-20, 0, 60, 80));
        land.set_valid_biomes(Some(&BTreeSet::from([BiomeId(3)])));
        assert!(land.column_has_valid_biome(20, 0, -60, 80));
        assert!(!land.column_has_valid_biome(20, 0, 0, 80));
        // The rest is the generator's.
        assert_eq!(land.seed(), 42);
        assert!(land.cold_enough_to_snow(BiomeId(7), [0, 0, 0]));
        assert_eq!((land.min_y(), land.max_y()), (-64, 319));
    }

    /// Answers only for its own seed, so tests building starts elsewhere in
    /// the crate never meet it.
    struct Fixed(i64, fn() -> Answer);
    impl Provider for Fixed {
        fn answer(&self, request: &Request<'_>) -> Answer {
            if request.seed() == self.0 {
                (self.1)()
            } else {
                Answer::Vanilla
            }
        }
    }

    #[test]
    fn providers_answer_in_order() {
        const SEED: i64 = -7_301_966_244;
        let request = |seed| {
            let generation = WorldGenerationContext::new(-64, 384, 63);
            Request::new(
                seed,
                ChunkPos { x: 3, z: -2 },
                None,
                generation,
                vec![],
                None,
            )
        };
        register("test:a", Arc::new(Fixed(SEED, || Answer::Vanilla)));
        register("test:b", Arc::new(Fixed(SEED, || Answer::NoStructures)));
        register(
            "test:c",
            Arc::new(Fixed(SEED, || Answer::Terrain(Arc::new(Land)))),
        );
        assert!(matches!(answer(&request(SEED)), Answer::NoStructures));
        assert!(matches!(answer(&request(SEED + 1)), Answer::Vanilla));
        // Replacing a provider keeps its place in the order.
        register(
            "test:a",
            Arc::new(Fixed(SEED, || Answer::Terrain(Arc::new(Land)))),
        );
        assert!(matches!(answer(&request(SEED)), Answer::Terrain(_)));
        assert!(unregister("test:a") && !unregister("test:a"));
        assert!(unregister("test:b") && unregister("test:c"));
        assert!(matches!(answer(&request(SEED)), Answer::Vanilla));
    }

    #[test]
    fn requests_tell_dimensions_and_known_biomes_apart() {
        let names = names();
        let generation = WorldGenerationContext::new(0, 256, 32);
        let chunk = ChunkPos { x: 0, z: 0 };
        let nether = vec!["minecraft:fortress", "minecraft:nether_fossil"];
        let nether = Request::new(1, chunk, None, generation, nether, Some(&names));
        assert_eq!(nether.dimension(), Dimension::Nether);
        assert!(nether.has_biome("minecraft:desert") && !nether.has_biome("minecraft:pale_garden"));
        let overworld = Request::new(
            1,
            chunk,
            None,
            generation,
            vec!["minecraft:mineshaft"],
            None,
        );
        assert_eq!(overworld.dimension(), Dimension::Overworld);
        assert!(overworld.has_biome("anything"));
    }
}
