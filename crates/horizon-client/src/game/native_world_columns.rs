//! Column reads for server mods, as helpers on the level being ticked: many columns' heights, and the blocks round
//! them, in one call.
use super::Game;
use super::block_edits::MAX_COLUMNS;
use super::native_world_edit::NativeHeightmap;
use horizon_client::streaming::ChunkGenerator;

/// What `native_read_columns` read: the blocks round each column's height (`bases`, none where it isn't loaded),
/// as indexes into `palette`, a column after another, and three biome tints a column (grass, foliage, water).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct NativeColumnBlocks {
    pub palette: Vec<String>,
    pub bases: Vec<Option<i32>>,
    pub blocks: Vec<u16>,
    pub tints: Vec<u32>,
}

/// What `native_terrain_surface` found at one column of terrain nobody has loaded: the level's generator before
/// its features (trees, ores, lava pools, cacti) are placed.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct NativeTerrainColumn {
    /// One above the highest solid block the generator scans down to, fluids skipped (the
    /// `OceanFloor` heightmap's count). The scan starts at the top of the level, so in the Nether this is
    /// the bedrock roof, not a floor.
    pub height: i32,
    /// The block at `height - 1`, such as `minecraft:grass_block`.
    pub block: String,
    /// One above the highest water or other fluid block over the surface, none where the surface is dry.
    pub water: Option<i32>,
    /// The biome, such as `minecraft:plains`.
    pub biome: String,
}

/// The most columns one `native_terrain_surface` call asks about: each is a noise sample on the server tick.
pub const MAX_TERRAIN_COLUMNS: usize = 4096;

impl Game<'_> {
    /// For a server mod: what the generator makes at each of `columns` of the level `dimension` (the one being
    /// ticked) before its features are placed, whether or not the chunk is loaded: it reads the generator and
    /// creates no chunk. Decorated blocks (trees, lava, cacti) are not in it: read the chunk once it is loaded. None
    /// for a column with no solid block (the End's void) and where the world has no generator of its own. `Err` says why nothing was
    /// read: another dimension or a client of a server, or more than 4,096 columns (`MAX_TERRAIN_COLUMNS`). API
    /// 645.
    pub(super) fn native_terrain_surface(
        &self,
        dimension: &str,
        columns: &[(i32, i32)],
    ) -> Result<Vec<Option<NativeTerrainColumn>>, String> {
        if !self.native_level_open(dimension) {
            return Err(format!("{dimension} is not the level being ticked"));
        }
        if columns.len() > MAX_TERRAIN_COLUMNS {
            return Err(format!(
                "{} columns, more than {MAX_TERRAIN_COLUMNS} a call",
                columns.len()
            ));
        }
        let started = std::time::Instant::now();
        let out = match self.end.locator() {
            Some(terrain) => {
                // The generator's biome ids are the world's table (data packs may add biomes).
                let (facts, biomes) = (self.res.facts(), world_biomes!(self));
                let mut worker = terrain.worker();
                columns
                    .iter()
                    .map(|&(x, z)| {
                        let s = terrain.lod_surface(&mut worker, x, z)?;
                        Some(NativeTerrainColumn {
                            height: s.height,
                            block: facts.get(s.state).name.to_string(),
                            water: s.water.map(|(top, _)| top),
                            biome: biomes.name(s.biome).to_owned(),
                        })
                    })
                    .collect()
            }
            None => vec![None; columns.len()],
        };
        self.native_read_logged(started, "terrain query", columns.len());
        Ok(out)
    }
    /// For a server mod: each column's height by `heightmap` in the authoritative level `dimension` (the one being
    /// ticked), none for a column not loaded. `Err` says why nothing was read: another dimension or a client of a
    /// server, or more than 65,536 columns (`MAX_COLUMNS`). API 127.
    pub(super) fn native_surface_heights(
        &self,
        dimension: &str,
        columns: &[(i32, i32)],
        heightmap: NativeHeightmap,
    ) -> Result<Vec<Option<i32>>, String> {
        if !self.native_level_open(dimension) {
            return Err(format!("{dimension} is not the level being ticked"));
        }
        if columns.len() > MAX_COLUMNS {
            return Err(format!(
                "{} columns, more than {MAX_COLUMNS} a call",
                columns.len()
            ));
        }
        let started = std::time::Instant::now();
        let out = self.surface_heights(columns, heightmap);
        self.native_read_logged(started, "height query", columns.len());
        Ok(out)
    }
    /// For a server mod: `below` blocks under and `above` blocks from each column's height by `heightmap`, as a
    /// palette and indexes, with the biome's grass, foliage and water tints (air outside the column and in columns
    /// not loaded). The tints are the biome's at the column's top, not blended over the biome blend radius as the
    /// game draws them: near a biome border they can differ from what players see. `Err` says why nothing was
    /// read: another dimension or a client of a server, `below + above` of 0 or more than the level's height, or
    /// a read of more than 1,048,576 blocks (`MAX_READ`; it runs on the server tick, so read big areas in pieces).
    /// API 127.
    pub(super) fn native_read_columns(
        &self,
        dimension: &str,
        columns: &[(i32, i32)],
        heightmap: NativeHeightmap,
        range: (u32, u32),
    ) -> Result<NativeColumnBlocks, String> {
        let (below, above) = range;
        if !self.native_level_open(dimension) {
            return Err(format!("{dimension} is not the level being ticked"));
        }
        let started = std::time::Instant::now();
        let out = self.read_columns(columns, heightmap, below, above)?;
        self.native_read_logged(started, "column read", columns.len());
        Ok(out)
    }
    /// A read's time logged with `HORIZON_SPIKES`, as an edit's is.
    fn native_read_logged(&self, started: std::time::Instant, what: &str, columns: usize) {
        let ms = started.elapsed().as_secs_f32() * 1000.0;
        if self.options.spikes.is_some_and(|threshold| ms > threshold) {
            log::info!("Horizon: slow mod {what} {ms:.1} ms ({columns} columns)");
        }
    }
}

#[cfg(test)]
mod tests;
