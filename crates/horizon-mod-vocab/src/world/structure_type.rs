//! Structure types a trait mod registers: vanilla's `StructureType` with its
//! own `Structure.findGenerationPoint` and `StructurePiece`s, as
//! [`Registrar::register_structure_type`](crate::Registrar::register_structure_type)
//! hands it over once at startup (NeoForge's registries `STRUCTURE_TYPE` and
//! `STRUCTURE_PIECE`). A data pack structure with `"type": "modid:tower"`
//! then generates by the mod's code.
//!
//! There is no `Host` here: starts build on the worldgen worker threads and
//! pieces place per chunk during generation, so the mod sees only the small
//! contexts below, `Send + Sync` and deterministic — the same seed gives the
//! same pieces, and a type keeps no mutable state because two workers can
//! build neighbouring chunks at once. Everything around the type stays the
//! game's own: the set's spacing, the structure's biomes and its
//! `terrain_adaptation`, and the pieces ride the one piece mechanism the game
//! has, next to a fortress piece's.
use crate::{BlockKey, BlockMetadata, BlockState};
pub use horizon_core::position::ChunkPos;
use horizon_core::{
    bounding_box::BoundingBox,
    nbt::{Compound, Tag},
};
use std::sync::Arc;

/// A piece's placement failed; the generation of that piece stops there, as a
/// piece's own `Result` does in the engine.
#[derive(Debug)]
// exhaustive: mods build it themselves as a piece's error answer
#[derive(horizon_mod_api_macros::Wire)]
pub struct StructureError(pub String);
impl std::fmt::Display for StructureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for StructureError {}
impl From<(crate::Identifier, String)> for StructureError {
    fn from((source, reason): (crate::Identifier, String)) -> Self {
        Self(format!("{source}: {reason}"))
    }
}

/// What [`StructureType::find_generation_point`](StructureType::find_generation_point)
/// found: the start's position (vanilla's `Structure.findGenerationPoint`
/// answer) and its pieces (`Structure.generatePieces`), at least one.
// exhaustive: mods build it themselves as find_generation_point's answer
#[derive(horizon_mod_api_macros::Wire)]
pub struct FoundStructure {
    /// Where the start sits, in world blocks.
    pub position: [i32; 3],
    /// The pieces the start places, at least one, shared with the game, which keeps them in
    /// its start.
    pub pieces: Vec<Arc<dyn StructurePiece>>,
}

/// A mod's structure type: vanilla's `StructureType`, whose code answers for a
/// data pack structure with `"type": "modid:tower"`. Implemented once, kept by
/// the game; deterministic and stateless.
pub trait StructureType: Send + Sync + 'static {
    /// `Structure.findGenerationPoint` with `generatePieces`: the start's
    /// position and its pieces for the chunk `context` names, or `None` for
    /// "no start in this chunk". The height and biome queries the context
    /// answers are the ones vanilla's own types ask of a
    /// `GenerationContext`; `context.config` lends its NBT config, converted once at load.
    fn find_generation_point(&self, context: &mut dyn GenerationContext) -> Option<FoundStructure>;
    /// The `StructurePieceType` loader: rebuild a saved piece from its tag
    /// (`StructurePieceType.load`), exactly as it was saved. Required: every
    /// piece type has its loader, and a saved piece is never regenerated. Resolve saved block
    /// states once here, against the same retained metadata the worker contexts borrow.
    /// Invalid saved blocks, properties or piece data return a typed error, never a panic.
    fn load_piece(
        &self,
        metadata: &dyn BlockMetadata,
        tag: &Compound,
    ) -> Result<Arc<dyn StructurePiece>, StructureError>;
    /// `Structure.afterPlace`, run once per chunk after all pieces were
    /// placed, with every piece's current box. Optional: most types place
    /// nothing here.
    fn after_place(
        &self,
        _context: &mut dyn PieceContext,
        _pieces: &[BoundingBox],
    ) -> Result<(), StructureError> {
        Ok(())
    }
}

/// The terrain-shaping rule around a structure piece.
#[derive(Clone, Copy, Debug, PartialEq, Eq, horizon_mod_api_macros::Wire)]
// exhaustive: these are the five terrain rules supported by the density sampler.
pub enum TerrainAdjustment {
    /// Leave the surrounding terrain unchanged.
    None,
    /// Blend terrain around a buried piece.
    Bury,
    /// Shape terrain around the piece's ground plane.
    BeardThin,
    /// Shape terrain around the full piece box.
    BeardBox,
    /// Blend terrain around the box to enclose the piece.
    Encapsulate,
}

/// A piece's terrain shape, independent of its placement box and structure's setting.
#[derive(Clone, Copy, Debug, horizon_mod_api_macros::Wire)]
#[non_exhaustive]
pub struct PieceBeardifier {
    /// Terrain-shaping box, independent of the piece's placement bounds.
    pub bounds: BoundingBox,
    /// Terrain adjustment used for this piece.
    pub adjustment: TerrainAdjustment,
    /// Ground-plane offset above the terrain box's minimum Y.
    pub ground_level_delta: i32,
}
impl PieceBeardifier {
    /// Construct the cached terrain metadata for one piece.
    pub fn new(
        bounds: BoundingBox,
        adjustment: TerrainAdjustment,
        ground_level_delta: i32,
    ) -> Self {
        Self {
            bounds,
            adjustment,
            ground_level_delta,
        }
    }
}

/// One piece of a mod's structure: vanilla's `StructurePiece`. Its `save` is
/// the engine's (`StructurePiece.createTag`: the type id, `BB`, `O`, `GD`)
/// around [`add_additional_save_data`](StructurePiece::add_additional_save_data).
pub trait StructurePiece: std::fmt::Debug + Send + Sync {
    /// `StructurePiece.getBoundingBox`: the piece's box, in world blocks.
    fn bounding_box(&self) -> BoundingBox;
    /// Override terrain shaping, even when the structure's adjustment is `None`.
    /// Queried once at generation or load, never per chunk. An answer whose adjustment
    /// is `None` omits this piece; no answer preserves the structure's default.
    fn beardifier(&self) -> Option<PieceBeardifier> {
        None
    }
    /// `StructurePiece.postProcess`: place the piece's blocks and entities
    /// that fall in the chunk `context` clips to. Writes outside it are
    /// dropped silently, as vanilla's `placeBlock` skips them.
    fn post_process(&self, context: &mut dyn PieceContext) -> Result<(), StructureError>;
    /// `StructurePiece.addAdditionalSaveData`: the piece's own fields, saved
    /// beside `BB`/`O`/`GD` and read back by the type's `load_piece`.
    fn add_additional_save_data(&self, tag: &mut Compound);
}

/// The random a structure type or piece draws from, in vanilla's order: the
/// chunk's `WorldgenRandom` for a start, the piece's `RandomSource` for a piece.
pub trait StructureRandom {
    /// `nextInt(bound)`; 0 for a bound below 1.
    fn next_int_bound(&mut self, bound: i32) -> i32;
    /// `nextLong`.
    fn next_long(&mut self) -> i64;
    /// `nextFloat`.
    fn next_float(&mut self) -> f32;
    /// `nextDouble`.
    fn next_double(&mut self) -> f64;
    /// `nextBoolean`.
    fn next_boolean(&mut self) -> bool;
}

/// A noise-column block, classified by the questions structures ask.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum NoiseBlock {
    /// An air block.
    Air,
    /// Water, including water supplied by an aquifer.
    Water,
    /// Lava.
    Lava,
    /// A noise block whose state blocks motion.
    Solid,
    /// Any other noise-column state.
    Other,
}
impl NoiseBlock {
    /// Whether this class is solid, not merely non-air.
    pub fn is_solid(self) -> bool {
        self == Self::Solid
    }
}

/// A noise column from its minimum Y upward; reads outside it are air.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct NoiseColumn {
    /// The minimum Y represented by the first block.
    pub min: i32,
    /// Classified blocks in increasing Y order.
    pub blocks: Vec<NoiseBlock>,
}
impl NoiseColumn {
    /// Construct a column from its minimum Y and classified blocks.
    pub fn new(min: i32, blocks: Vec<NoiseBlock>) -> Self {
        Self { min, blocks }
    }
    /// Read a class, returning air outside the represented range.
    pub fn get(&self, y: i32) -> NoiseBlock {
        usize::try_from(i64::from(y) - i64::from(self.min))
            .ok()
            .and_then(|index| self.blocks.get(index).copied())
            .unwrap_or(NoiseBlock::Air)
    }
}

/// The vanilla heightmap opacity predicates used by structure queries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum HeightmapType {
    /// Highest non-air block during world generation.
    WorldSurfaceWg,
    /// Highest non-air block.
    WorldSurface,
    /// Highest motion-blocking block during world generation.
    OceanFloorWg,
    /// Highest motion-blocking block.
    OceanFloor,
    /// Highest motion-blocking block or fluid.
    MotionBlocking,
    /// Motion-blocking block or fluid, ignoring leaves.
    MotionBlockingNoLeaves,
}

/// An opaque, process-local biome identity. Do not persist it or perform arithmetic on it.
#[derive(horizon_mod_api_macros::Wire)]
#[wire(internal_field_layouts)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub struct BiomeId(pub(crate) u32);

/// Construct the game's biome identity at the hidden integration boundary.
// hidden: only the game's adapter builds a biome identity; mods receive one from the context.
#[doc(hidden)]
pub fn biome_id(index: u32) -> BiomeId {
    BiomeId(index)
}

/// The start context a type's
/// [`find_generation_point`](StructureType::find_generation_point) sees:
/// vanilla's `GenerationContext` over one chunk, on the worker thread;
/// deterministic for the seed.
pub trait GenerationContext {
    /// Immutable, load-owned block metadata, safe to borrow on this worker.
    fn metadata(&self) -> &dyn BlockMetadata;
    /// The chunk the start would be in.
    fn chunk(&self) -> ChunkPos;
    /// The world's seed.
    fn seed(&self) -> i64;
    /// The level's sea level.
    fn sea_level(&self) -> i32;
    /// The level's minimum block Y.
    fn min_y(&self) -> i32;
    /// The level's maximum block Y.
    fn max_y(&self) -> i32;
    /// The random vanilla's types draw from, seeded for this chunk
    /// (`WorldgenRandom.setLargeFeatureSeed` of the seed, chunk X and Z).
    fn random(&mut self) -> &mut dyn StructureRandom;
    /// `ChunkGenerator.getBaseHeight`: the first free height at (x, z) by the
    /// heightmap kind.
    fn base_height(&mut self, x: i32, z: i32, kind: HeightmapType) -> i32;
    /// `ChunkGenerator.getBaseColumn` as heightmap opacity, from `min_y` up,
    /// for several kinds from one noise-column pass.
    fn base_columns(&mut self, x: i32, z: i32, kinds: &[HeightmapType]) -> Vec<Vec<bool>>;
    /// `ChunkGenerator.getBaseColumn`'s blocks: the noise column at (x, z).
    fn noise_column(&mut self, x: i32, z: i32) -> NoiseColumn;
    /// Noise biome at a quart position (the resolver's, no zoom).
    fn noise_biome(&mut self, quart: [i32; 3]) -> BiomeId;
    /// `GenerationContext.validBiome` over inclusive block Y bounds of the
    /// column at (x, z): whether any biome the structure names occurs there.
    fn column_has_valid_biome(&mut self, x: i32, z: i32, min_y: i32, max_y: i32) -> bool;
    /// The structure's NBT config, converted once when its definition loads.
    fn config(&self) -> Arc<Tag>;
}

/// The placement context a piece's
/// [`post_process`](StructurePiece::post_process) and a type's
/// [`after_place`](StructureType::after_place) see: the blocks and entities of
/// one chunk, the piece's random and what the blocks around it are made of.
pub trait PieceContext {
    /// Immutable block metadata for this worker's resource snapshot.
    fn metadata(&self) -> &dyn BlockMetadata;
    /// The chunk being placed.
    fn chunk(&self) -> ChunkPos;
    /// The writable box: a `set_block` outside it is dropped silently.
    fn chunk_box(&self) -> BoundingBox;
    /// The piece's random (`RandomSource` of `postProcess`).
    fn random(&mut self) -> &mut dyn StructureRandom;
    /// The block's default state; resolve it once when constructing or loading a piece.
    fn state(&self, block: BlockKey) -> BlockState;
    /// A block state with partial properties; missing properties keep defaults.
    /// Resolve once at piece construction or load, never once per placed block.
    fn state_with(&self, block: BlockKey, properties: &[(&str, &str)]) -> BlockState;
    /// `BlockState.isAir`.
    fn is_air(&self, state: BlockState) -> bool;
    /// The block at `pos`, or `None` outside the region.
    fn block(&mut self, pos: [i32; 3]) -> Option<BlockState>;
    /// `StructurePiece.placeBlock`: write `state` at `pos`, answering whether
    /// it was written. A position outside `chunk_box` is skipped silently, as
    /// vanilla's `placeBlock` skips it.
    fn set_block(&mut self, pos: [i32; 3], state: BlockState) -> bool;
    /// `WorldGenLevel.getHeight` of the heightmap kind at (x, z).
    fn height(&mut self, kind: HeightmapType, x: i32, z: i32) -> Option<i32>;
    /// `BlockState.isSolidRender` of a state: whether the block draws as a
    /// solid block, the way `StructurePiece.reorient` asks of its neighbours.
    fn solid_render(&self, state: BlockState) -> bool;
    /// `WorldGenLevel.addFreshEntity`: an entity from its saved NBT — its
    /// `id`, `Pos` and fields — which the generating chunk holds and the game
    /// finishes as it does a structure mob's when the chunk joins the world
    /// (`Mob.finalizeSpawn` with `STRUCTURE`; keep the
    /// `"horizon:finalize"` byte a structure template's entities carry to ask
    /// for it).
    fn entity(&mut self, nbt: Compound);
    /// `WorldGenLevel.setBlockEntity` at `pos`: the block entity's saved NBT
    /// (its `id` and fields).
    fn block_entity(&mut self, pos: [i32; 3], nbt: Compound);
}

/// A registered API behaviour; the game builds its engine adapter at startup.
pub(crate) struct StructureTypeInfo {
    pub(crate) id: Box<str>,
    pub(crate) ty: Arc<dyn StructureType>,
}

/// Registered structure behaviours, for the hidden game integration boundary.
// hidden: the game's startup adapter reads the registered behaviours; mods declare them through the registrar.
#[doc(hidden)]
pub fn structure_type_declarations() -> Vec<(String, Arc<dyn StructureType>)> {
    crate::parts::installed_structure_type_list()
        .iter()
        .map(|info| (info.id.to_string(), info.ty.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn noise_column_clips_outside_y_without_overflow() {
        let column = NoiseColumn::new(-2, vec![NoiseBlock::Water, NoiseBlock::Solid]);
        assert_eq!(column.get(-3), NoiseBlock::Air);
        assert_eq!(column.get(-2), NoiseBlock::Water);
        assert_eq!(column.get(-1), NoiseBlock::Solid);
        assert_eq!(column.get(0), NoiseBlock::Air);
        let extreme = NoiseColumn::new(i32::MAX, vec![NoiseBlock::Lava]);
        assert_eq!(extreme.get(i32::MAX), NoiseBlock::Lava);
        assert_eq!(extreme.get(i32::MIN), NoiseBlock::Air);
    }
}
