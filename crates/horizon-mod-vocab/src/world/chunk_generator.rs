//! Chunk generators a native mod registers: vanilla's `ChunkGenerator` with
//! its own terrain, as
//! [`register_chunk_generator`](crate::Registrar::register_chunk_generator)
//! hands it over once at startup. A level whose stem names `modid:void`, one
//! [`register_dimension`](crate::Registrar::register_dimension) made or a data
//! pack's, then generates by the mod's code.
//!
//! There is no `Host` here: chunks fill on the worldgen worker threads, so the
//! mod sees only the [`ChunkAccess`] of the chunk it fills, and a generator
//! keeps no mutable state because two workers can fill neighbouring chunks at
//! once. The level gets the generator's terrain, then the
//! [`VanillaStages`] it asks for.
use crate::{BlockKey, BlockMetadata, BlockState, ChunkPos};
use std::sync::Arc;

/// The chunk a generator fills: vanilla's `ChunkAccess`, as far as terrain needs it. Positions
/// are world blocks.
pub trait ChunkAccess {
    /// Immutable block metadata from this worker's resource snapshot.
    fn metadata(&self) -> &dyn BlockMetadata;
    /// `ChunkAccess.getPos`.
    fn pos(&self) -> ChunkPos;
    /// The id of the level this chunk belongs to (`mymod:pocket`), so one stateless generator
    /// can tell the levels it serves apart.
    fn level(&self) -> &str;
    /// The level's seed: the world's seed.
    fn seed(&self) -> i64;
    /// The lowest block Y the terrain fills: the level's (`getMinY`), or with
    /// [`VanillaStages`] its noise settings', as vanilla's noise fill reads them.
    fn min_y(&self) -> i32;
    /// How many blocks up from [`min_y`](Self::min_y) the terrain fills: the
    /// level's build height (`getHeight`), or with [`VanillaStages`] its noise
    /// settings' height inside it.
    fn height(&self) -> i32;
    /// The block's default state. Resolve before the terrain-writing loop.
    fn state(&self, block: BlockKey) -> BlockState;
    /// A block state with partial properties; missing properties keep defaults.
    /// Resolve before the terrain-writing loop, never once per placed block.
    fn state_with(&self, block: BlockKey, properties: &[(&str, &str)]) -> BlockState;
    /// `setBlockState`: write `state` at `pos`, answering whether it was written. A position
    /// outside the chunk or the range [`min_y`](Self::min_y) and [`height`](Self::height) give is
    /// skipped.
    fn set_block(&mut self, pos: [i32; 3], state: BlockState) -> bool;
}

/// The vanilla stages a level runs around a mod's terrain, in vanilla's chunk
/// status order: the structure starts and references before the terrain;
/// `surface` and then `carvers` after it, as `NoiseBasedChunkGenerator.buildTerrain`
/// runs them after its noise fill; then the FEATURES status, where
/// `ChunkGenerator.applyBiomeDecoration` places the structure pieces and the
/// biomes' placed features. The default is none of them: the level is the
/// generator's terrain alone.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
// exhaustive: a generator builds it with a literal as its vanilla_stages answer
#[derive(horizon_mod_api_macros::Wire)]
pub struct VanillaStages {
    /// The level's material rule over the terrain: grass, sand, deepslate,
    /// bedrock and ore veins, from its `noise_settings`.
    pub surface: bool,
    /// The carvers of the level's biomes: caves and canyons, with the aquifer
    /// of its `noise_settings`.
    pub carvers: bool,
    /// The structures of the world's structure sets that may start in the
    /// level's biomes (`ChunkGenerator.createStructures`) and their pieces. A
    /// start is placed at the height of the level's `noise_settings` terrain
    /// (`getBaseHeight`), not of the generator's. None while the world's
    /// structures are turned off.
    pub structures: bool,
    /// The placed features of the level's biomes.
    pub features: bool,
}

/// A mod's chunk generator: vanilla's `ChunkGenerator`, whose code answers for
/// a level stem with `"generator": {"type": "modid:void"}`. Implemented once,
/// kept by the game; deterministic and stateless.
pub trait ChunkGenerator: Send + Sync + 'static {
    /// `ChunkGenerator.buildTerrain`: write the terrain of `chunk`, which
    /// starts as air. The same level id, seed and chunk give the same blocks.
    fn build_terrain(&self, chunk: &mut dyn ChunkAccess);
    /// The vanilla stages the level runs after [`build_terrain`](Self::build_terrain),
    /// asked once, at registration. None by default.
    fn vanilla_stages(&self) -> VanillaStages {
        VanillaStages::default()
    }
}

/// A registered API generator, before the game builds its worker adapter.
pub(crate) struct ChunkGeneratorInfo {
    pub(crate) id: Box<str>,
    /// Its [`ChunkGenerator::vanilla_stages`], asked once at registration.
    pub(crate) stages: VanillaStages,
    pub(crate) generator: Arc<dyn ChunkGenerator>,
}

/// Registered generators for the hidden game integration boundary: each id, its
/// vanilla stages and the generator.
// hidden: the game's startup adapter reads the registered generators; mods declare them through the registrar.
#[doc(hidden)]
pub fn chunk_generator_declarations() -> Vec<(String, VanillaStages, Arc<dyn ChunkGenerator>)> {
    crate::parts::installed_chunk_generator_list()
        .iter()
        .map(|info| (info.id.to_string(), info.stages, info.generator.clone()))
        .collect()
}
