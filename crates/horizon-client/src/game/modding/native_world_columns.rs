//! Column reads for server mods, as helpers on the level being ticked: many columns' heights, and the blocks round
//! them, in one call.
use super::Game;
use super::block_edits::MAX_COLUMNS;
use horizon_client::streaming::ChunkGenerator;
use mod_api::{LodTileFootprint, TerrainColumn};

/// The most columns one `terrain_surface` call asks about: each is a noise sample on the server tick.
pub(super) const MAX_TERRAIN_COLUMNS: usize = 4096;

impl Game<'_> {
    /// For a server mod: what the generator makes at each of `columns` of the level `dimension` (the one being
    /// ticked) before its features are placed, whether or not the chunk is loaded: it reads the generator and
    /// creates no chunk. Decorated blocks (trees, lava, cacti) are not in it: read the chunk once it is loaded. None
    /// for a column with no solid block (the End's void) and where the world has no generator of its own. `Err` says
    /// why nothing was read: another dimension or a client of a server, or more than 4,096 columns
    /// (`MAX_TERRAIN_COLUMNS`). API @API:terrain_surface@.
    pub(super) fn terrain_surface(
        &self,
        dimension: &str,
        columns: &[(i32, i32)],
    ) -> Result<Vec<Option<TerrainColumn>>, String> {
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
                        Some(mod_api::__private::built::terrain_column(
                            s.height,
                            facts.get(s.state).name.to_string(),
                            s.water.map(|(top, _)| top),
                            biomes.name(s.biome).to_owned(),
                        ))
                    })
                    .collect()
            }
            None => vec![None; columns.len()],
        };
        self.native_read_logged(started, "terrain query", columns.len());
        Ok(out)
    }
    /// For a client mod: each column's height by `heightmap` in the world this window shows,
    /// none for a column whose chunk this client does not hold. `Err` says why nothing was read:
    /// no client world, or more than 65,536 columns (`MAX_COLUMNS`).
    pub(super) fn client_surface_heights(
        &self,
        columns: &[(i32, i32)],
        heightmap: mod_api::Heightmap,
    ) -> Result<Vec<Option<i32>>, String> {
        if !self.remote.as_ref().is_some_and(|r| r.logged_in) {
            return Err("no client world is open".to_string());
        }
        if columns.len() > MAX_COLUMNS {
            return Err(format!(
                "{} columns, more than {MAX_COLUMNS} a call",
                columns.len()
            ));
        }
        Ok(self.loaded_column_heights(columns, heightmap))
    }
    /// For a client mod: each column's surface as this window's far terrain holds it (`Lod::surface`), whether
    /// or not the chunk is loaded; none where no far tile covers it and everywhere while far terrain is off. `Err`
    /// says why nothing was read: no client world, or more than 65,536 columns (`MAX_COLUMNS`).
    pub(super) fn client_far_surface(
        &self,
        columns: &[(i32, i32)],
    ) -> Result<Vec<Option<TerrainColumn>>, String> {
        if !self.remote.as_ref().is_some_and(|r| r.logged_in) {
            return Err("no client world is open".to_string());
        }
        if columns.len() > MAX_COLUMNS {
            return Err(format!(
                "{} columns, more than {MAX_COLUMNS} a call",
                columns.len()
            ));
        }
        let Some(lod) = self.lod.as_ref() else {
            return Ok(vec![None; columns.len()]);
        };
        // The far tiles' biome ids are the world's table, as their worker context's (`worker_context`).
        let (facts, biomes) = (self.res.facts(), world_biomes!(self));
        Ok(columns
            .iter()
            .map(|&(x, z)| {
                let s = lod.surface(x, z)?;
                Some(mod_api::__private::built::terrain_column(
                    s.height,
                    facts.get(s.state).name.to_string(),
                    s.water.map(|(top, _)| top),
                    biomes.name(s.biome).to_owned(),
                ))
            })
            .collect())
    }

    pub(super) fn client_lod_columns(
        &self,
        tiles: &[(i32, i32, u8)],
    ) -> Result<Vec<Option<LodTileFootprint>>, String> {
        use horizon_client::lod::TileKey;
        use mod_api::__private::built::{lod_column_footprint, lod_tile_footprint};

        const MAX_LOD_TILES: usize = MAX_COLUMNS / (16 * 16);
        if !self.remote.as_ref().is_some_and(|r| r.logged_in) {
            return Err("no client world is open".to_string());
        }
        if tiles.len() > MAX_LOD_TILES {
            return Err(format!(
                "{} tiles, more than {MAX_LOD_TILES} a call",
                tiles.len()
            ));
        }
        let Some(lod) = self.lod.as_ref() else {
            return Ok(vec![None; tiles.len()]);
        };
        let biomes = world_biomes!(self);
        Ok(tiles
            .iter()
            .map(|&(x, z, level)| {
                let key = TileKey { x, z, level };
                let columns = lod
                    .tile_top_spans(key)?
                    .map(|span| {
                        let span = span?;
                        Some(lod_column_footprint(
                            span.min,
                            span.max,
                            mod_api::BlockStateId(span.state.0),
                            biomes.name(span.biome).to_owned(),
                            ((span.light >> 4) & 15) as u8,
                            ((span.light >> 12) & 15) as u8,
                        ))
                    })
                    .collect();
                Some(lod_tile_footprint(key.size() / 16, columns))
            })
            .collect())
    }

    /// For a server mod: each column's height by `heightmap` in the authoritative level `dimension` (the one being
    /// ticked), none for a column not loaded. `Err` says why nothing was read: another dimension or a client of a
    /// server, or more than 65,536 columns (`MAX_COLUMNS`).
    pub(super) fn surface_heights(
        &self,
        dimension: &str,
        columns: &[(i32, i32)],
        heightmap: mod_api::Heightmap,
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
        let out = self.loaded_column_heights(columns, heightmap);
        self.native_read_logged(started, "height query", columns.len());
        Ok(out)
    }
    /// For a server mod: `below` blocks under and `above` blocks from each column's height by `heightmap`, as a
    /// palette and indexes, with the biome's grass, foliage and water tints (air outside the column and in columns
    /// not loaded). The tints are the biome's at the column's top, not blended over the biome blend radius as the
    /// game draws them: near a biome border they can differ from what players see. `Err` says why nothing was
    /// read: another dimension or a client of a server, `below + above` of 0 or more than the level's height, or
    /// a read of more than 1,048,576 blocks (`MAX_READ`; it runs on the server tick, so read big areas in pieces).
    pub(super) fn read_columns(
        &self,
        dimension: &str,
        columns: &[(i32, i32)],
        heightmap: mod_api::Heightmap,
        range: (u32, u32),
    ) -> Result<mod_api::ColumnBlocks, String> {
        let (below, above) = range;
        if !self.native_level_open(dimension) {
            return Err(format!("{dimension} is not the level being ticked"));
        }
        let started = std::time::Instant::now();
        let out = self.read_loaded_block_columns(columns, heightmap, below, above)?;
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
#[path = "native_world_columns/tests.rs"]
mod tests;
