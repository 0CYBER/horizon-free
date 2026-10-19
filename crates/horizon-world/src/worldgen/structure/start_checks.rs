//! Start rules a carried structure mod adds on top of vanilla's jigsaw.
//!
//! Repurposed Structures and Moog's Structure Lib give their jigsaw
//! structures checks that run before the pieces are laid out: every biome
//! in a radius must be one the structure allows, the start column may not
//! end in liquid, and the terrain in a radius must lie within a height range.
//! Carry (`horizon-port-pack`) maps those structures to `minecraft:jigsaw`
//! and writes the checks under `"horizon:start_checks"`; vanilla data never
//! has the key, so vanilla structures are unchanged.
//!
//! ```json
//! "horizon:start_checks": {
//!   "biome_radius": 2,        // chunks; every noise biome must be allowed
//!   "liquid": "top",          // "top": the column's highest block; "lowest": the block above min_y
//!   "terrain_radius": 1,      // chunks, with terrain_range and/or min_y, max_y
//!   "terrain_range": 15,
//!   "min_y": 60,
//!   "max_y": 200,
//!   "repurposed": "city_nether" // or "shipwreck_nether", "stronghold_end": below
//! }
//! ```
//!
//! `repurposed` is the extra check of one of Repurposed Structures' jigsaw
//! types, after the others: its Nether city needs open air above the lava sea
//! in the chunks around, its Nether shipwreck open air or lava around the
//! start, and its End stronghold land around the chunk.
use crate::{heightmap::HeightmapType, worldgen::biome::BiomeId};
use horizon_core::position::ChunkPos;
use serde_json::Value;
use std::collections::BTreeSet;

/// Which block of the start column must not be a liquid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LiquidCheck {
    /// The highest non-air block of the noise column (Moog's).
    Top,
    /// The block above the column's bottom (Repurposed Structures: its loop
    /// keeps the last block it reads, not the first solid one).
    Lowest,
}

/// The extra start check of a Repurposed Structures jigsaw type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Repurposed {
    /// CityNetherStructure: air from sea level + 20 to + 45 (or the top of
    /// the noise) at the corner of each of the 8 chunks around.
    CityNether,
    /// ShipwreckNetherStructure: no solid block at 5×5 columns 8 apart
    /// around the chunk corner, from the start height up 30 in steps of 5.
    ShipwreckNether,
    /// StrongholdEndStructure: land at least 45 above the bottom 35 and 70
    /// blocks away in each direction and at the chunk corner.
    StrongholdEnd,
}

/// The checks of one structure; all empty for vanilla's.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StartChecks {
    biome_radius: Option<i32>,
    liquid: Option<LiquidCheck>,
    terrain_radius: Option<i32>,
    terrain_range: Option<i32>,
    min_y: Option<i32>,
    max_y: Option<i32>,
    repurposed: Option<Repurposed>,
}

/// The column and biome queries the checks need.
pub trait CheckQueries {
    /// ChunkGenerator.getFirstFreeHeight.
    fn first_free_height(&mut self, x: i32, z: i32, kind: HeightmapType) -> i32;
    /// The noise column at (`x`, `z`) as (not air, blocks motion) per block
    /// from the noise's bottom up: in a noise column a block that is not air
    /// and does not block motion is a liquid.
    fn liquid_column(&mut self, x: i32, z: i32) -> (Vec<bool>, Vec<bool>);
    fn noise_biome(&mut self, quart: [i32; 3]) -> BiomeId;
    fn sea_level(&self) -> i32;
    /// The noise's bottom, where the columns start.
    fn bottom(&self) -> i32;
}

impl<T: super::start::StartContext> CheckQueries for T {
    fn first_free_height(&mut self, x: i32, z: i32, kind: HeightmapType) -> i32 {
        self.base_height(x, z, kind)
    }
    fn liquid_column(&mut self, x: i32, z: i32) -> (Vec<bool>, Vec<bool>) {
        let mut columns = self.base_columns(
            x,
            z,
            &[HeightmapType::WorldSurfaceWg, HeightmapType::OceanFloorWg],
        );
        let solid = columns.pop().unwrap_or_default();
        (columns.pop().unwrap_or_default(), solid)
    }
    fn noise_biome(&mut self, quart: [i32; 3]) -> BiomeId {
        super::start::StartContext::noise_biome(self, quart)
    }
    fn sea_level(&self) -> i32 {
        super::start::StartContext::sea_level(self)
    }
    fn bottom(&self) -> i32 {
        self.min_y()
    }
}

impl StartChecks {
    /// `"horizon:start_checks"` of a structure, or none.
    pub fn decode(value: Option<&Value>) -> Self {
        let Some(v) = value else {
            return Self::default();
        };
        let int = |k: &str| v.get(k).and_then(Value::as_i64).map(|n| n as i32);
        Self {
            biome_radius: int("biome_radius"),
            liquid: match v.get("liquid").and_then(Value::as_str) {
                Some("top") => Some(LiquidCheck::Top),
                Some("lowest") => Some(LiquidCheck::Lowest),
                _ => None,
            },
            terrain_radius: int("terrain_radius"),
            terrain_range: int("terrain_range"),
            min_y: int("min_y"),
            max_y: int("max_y"),
            repurposed: match v.get("repurposed").and_then(Value::as_str) {
                Some("city_nether") => Some(Repurposed::CityNether),
                Some("shipwreck_nether") => Some(Repurposed::ShipwreckNether),
                Some("stronghold_end") => Some(Repurposed::StrongholdEnd),
                _ => None,
            },
        }
    }

    /// GenericJigsawStructure.extraSpawningChecks: whether the structure may
    /// start in `chunk` at `y` (the sampled start height).
    pub fn pass(
        &self,
        queries: &mut impl CheckQueries,
        biomes: &BTreeSet<BiomeId>,
        chunk: ChunkPos,
        y: i32,
        project_to: Option<HeightmapType>,
    ) -> bool {
        let (x0, z0) = (chunk.x * 16, chunk.z * 16);
        if let Some(radius) = self.biome_radius {
            let mut section_y = y;
            if let Some(kind) = project_to {
                section_y += queries.first_free_height(x0 + 7, z0 + 7, kind) - 1;
            }
            for cx in chunk.x - radius..=chunk.x + radius {
                for cz in chunk.z - radius..=chunk.z + radius {
                    if !biomes.contains(&queries.noise_biome([cx << 2, section_y >> 2, cz << 2])) {
                        return false;
                    }
                }
            }
        }
        if let Some(check) = self.liquid {
            let (filled, solid) = queries.liquid_column(x0 + 7, z0 + 7);
            // From the top down to the block above the bottom, as the mods loop.
            let index = match check {
                LiquidCheck::Top => (1..filled.len()).rev().find(|&i| filled[i]),
                LiquidCheck::Lowest => Some(1).filter(|&i| i < filled.len()),
            };
            if index.is_some_and(|i| filled[i] && !solid[i]) {
                return false;
            }
        }
        if let Some(radius) = self
            .terrain_radius
            .filter(|_| self.terrain_range.is_some() || self.min_y.is_some())
        {
            let kind = project_to.unwrap_or(HeightmapType::WorldSurfaceWg);
            let (mut highest, mut lowest) = (i32::MIN, i32::MAX);
            for cx in chunk.x - radius..=chunk.x + radius {
                for cz in chunk.z - radius..=chunk.z + radius {
                    let height = queries.first_free_height((cx << 4) + 7, (cz << 4) + 7, kind);
                    highest = highest.max(height);
                    lowest = lowest.min(height);
                    // Both bounds test the lowest height so far, as the mods do.
                    if self.min_y.is_some_and(|m| lowest < m)
                        || self.max_y.is_some_and(|m| lowest > m)
                    {
                        return false;
                    }
                }
            }
            if self
                .terrain_range
                .is_some_and(|range| highest - lowest > range)
            {
                return false;
            }
        }
        match self.repurposed {
            Some(check) => repurposed(check, queries, chunk, y),
            None => true,
        }
    }
}

/// [`Repurposed`]'s checks. A noise column reads air past its ends; its
/// length is the noise's depth (GeneralUtils.getMaxTerrainLimit).
fn repurposed(check: Repurposed, queries: &mut impl CheckQueries, chunk: ChunkPos, y: i32) -> bool {
    let (x0, z0) = (chunk.x * 16, chunk.z * 16);
    let bottom = queries.bottom();
    let at = |column: &[bool], y: i32| {
        usize::try_from(y - bottom)
            .ok()
            .and_then(|i| column.get(i).copied())
            .unwrap_or(false)
    };
    match check {
        Repurposed::CityNether => {
            let sea = queries.sea_level();
            for cx in chunk.x - 1..=chunk.x + 1 {
                for cz in chunk.z - 1..=chunk.z + 1 {
                    if (cx, cz) == (chunk.x, chunk.z) {
                        continue;
                    }
                    let (filled, _) = queries.liquid_column(cx << 4, cz << 4);
                    let top = (bottom + filled.len() as i32).min(sea + 45);
                    if (sea + 20..top).any(|y| at(&filled, y)) {
                        return false;
                    }
                }
            }
            true
        }
        Repurposed::ShipwreckNether => {
            for dx in (-16..=16).step_by(8) {
                for dz in (-16..=16).step_by(8) {
                    let (filled, solid) = queries.liquid_column(x0 + dx, z0 + dz);
                    // Not air and no fluid: in a noise column, a solid block.
                    if (0..=30)
                        .step_by(5)
                        .any(|dy| at(&filled, y + dy) && at(&solid, y + dy))
                    {
                        return false;
                    }
                }
            }
            true
        }
        Repurposed::StrongholdEnd => {
            let (filled, _) = queries.liquid_column(x0, z0);
            let min_land = (bottom + filled.len() as i32).min(bottom + 45);
            // getHeightAt: the lowest height so far, less one at every sample.
            let mut land = i32::MAX;
            let mut sample = |x: i32, z: i32, land: &mut i32| {
                let h = queries.first_free_height(x, z, HeightmapType::WorldSurfaceWg);
                *land = (*land).min(h) - 1;
                *land >= min_land
            };
            for distance in [70, 35] {
                // Direction.Plane.HORIZONTAL: north, east, south, west.
                for (dx, dz) in [(0, -1), (1, 0), (0, 1), (-1, 0)] {
                    if !sample(x0 + dx * distance, z0 + dz * distance, &mut land) {
                        return false;
                    }
                }
            }
            sample(x0, z0, &mut land)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::BTreeMap;

    /// Heights per (x, z), a liquid column, one biome per quart column.
    #[derive(Default)]
    struct Fake {
        heights: BTreeMap<(i32, i32), i32>,
        column: Vec<char>,
        biomes: BTreeMap<(i32, i32), u32>,
        asked: Vec<[i32; 3]>,
        /// Columns by (x, z), instead of `column`, from y 0.
        columns: BTreeMap<(i32, i32), Vec<char>>,
    }
    impl CheckQueries for Fake {
        fn first_free_height(&mut self, x: i32, z: i32, _: HeightmapType) -> i32 {
            self.heights.get(&(x, z)).copied().unwrap_or(64)
        }
        fn liquid_column(&mut self, x: i32, z: i32) -> (Vec<bool>, Vec<bool>) {
            // '.' air, '#' stone, '~' water, from the bottom up.
            let column = self.columns.get(&(x, z)).unwrap_or(&self.column);
            (
                column.iter().map(|&c| c != '.').collect(),
                column.iter().map(|&c| c == '#').collect(),
            )
        }
        fn sea_level(&self) -> i32 {
            32
        }
        fn bottom(&self) -> i32 {
            0
        }
        fn noise_biome(&mut self, quart: [i32; 3]) -> BiomeId {
            self.asked.push(quart);
            BiomeId(self.biomes.get(&(quart[0], quart[2])).copied().unwrap_or(1))
        }
    }
    fn checks(v: Value) -> StartChecks {
        StartChecks::decode(Some(&v))
    }
    const CHUNK: ChunkPos = ChunkPos { x: 2, z: -1 };

    #[test]
    fn vanilla_structures_pass_everything() {
        let mut fake = Fake {
            column: "#~~.".chars().collect(),
            ..Fake::default()
        };
        let none = StartChecks::decode(None);
        assert_eq!(none, StartChecks::default());
        assert!(none.pass(&mut fake, &BTreeSet::new(), CHUNK, 0, None));
        assert!(fake.asked.is_empty());
    }

    #[test]
    fn every_biome_in_the_radius_must_be_allowed() {
        let allowed = BTreeSet::from([BiomeId(1)]);
        let mut fake = Fake::default();
        fake.heights.insert((39, -9), 70);
        let c = checks(json!({"biome_radius": 1}));
        assert!(c.pass(
            &mut fake,
            &allowed,
            CHUNK,
            10,
            Some(HeightmapType::WorldSurfaceWg)
        ));
        // Section y: start height plus the projected height minus one, in quarts.
        assert_eq!(fake.asked.len(), 9);
        assert!(fake.asked.iter().all(|q| q[1] == (10 + 70 - 1) >> 2));
        assert_eq!(fake.asked[0], [4, 19, -8]);
        fake.biomes.insert((12, 0), 2); // chunk (3, 0), the radius' corner
        assert!(!c.pass(&mut fake, &allowed, CHUNK, 10, None));
    }

    #[test]
    fn liquid_checks_read_the_mods_block() {
        let top = checks(json!({"liquid": "top"}));
        let lowest = checks(json!({"liquid": "lowest"}));
        let none = BTreeSet::new();
        let mut ocean = Fake {
            column: "##~~~..".chars().collect(),
            ..Fake::default()
        };
        assert!(!top.pass(&mut ocean, &none, CHUNK, 0, None));
        assert!(lowest.pass(&mut ocean, &none, CHUNK, 0, None));
        let mut land = Fake {
            column: "#~###..".chars().collect(),
            ..Fake::default()
        };
        assert!(top.pass(&mut land, &none, CHUNK, 0, None));
        assert!(!lowest.pass(&mut land, &none, CHUNK, 0, None));
        // The bottom block itself is never read; an empty column passes.
        let mut empty = Fake {
            column: "~....".chars().collect(),
            ..Fake::default()
        };
        assert!(top.pass(&mut empty, &none, CHUNK, 0, None));
    }

    #[test]
    fn terrain_must_lie_within_its_range() {
        let none = BTreeSet::new();
        let mut fake = Fake::default();
        // Chunk centres around (2, -1): x 23, 39, 55; z -25, -9, 7.
        fake.heights.insert((23, -25), 60);
        fake.heights.insert((55, 7), 75);
        let range = |r: i32| checks(json!({"terrain_radius": 1, "terrain_range": r}));
        assert!(range(15).pass(&mut fake, &none, CHUNK, 0, None));
        assert!(!range(14).pass(&mut fake, &none, CHUNK, 0, None));
        // The radius alone, or with only max_y, checks nothing.
        assert!(
            checks(json!({"terrain_radius": 1, "max_y": 0})).pass(&mut fake, &none, CHUNK, 0, None)
        );
        let bounds =
            |min: i32, max: i32| checks(json!({"terrain_radius": 1, "min_y": min, "max_y": max}));
        assert!(bounds(60, 64).pass(&mut fake, &none, CHUNK, 0, None));
        assert!(!bounds(61, 100).pass(&mut fake, &none, CHUNK, 0, None));
        // max_y is compared with the lowest height so far: 75 never fails it.
        assert!(bounds(0, 64).pass(&mut fake, &none, CHUNK, 0, None));
        assert!(!bounds(0, 59).pass(&mut fake, &none, CHUNK, 0, None));
    }

    /// A column of `n` blocks: `fill` up to `top` (exclusive), air above.
    fn column(n: usize, top: usize, fill: char) -> Vec<char> {
        (0..n).map(|y| if y < top { fill } else { '.' }).collect()
    }

    #[test]
    fn repurposed_city_needs_air_above_the_lava_sea_around() {
        let none = BTreeSet::new();
        let city = checks(json!({"repurposed": "city_nether"}));
        // Sea 32: air needed from 52 to 76 (128 deep), in the 8 chunks around.
        let mut fake = Fake {
            column: column(128, 52, '#'),
            ..Fake::default()
        };
        // The chunk itself is not read.
        fake.columns.insert((32, -16), column(128, 100, '#'));
        assert!(city.pass(&mut fake, &none, CHUNK, 33, None));
        fake.columns.insert((48, 0), column(128, 53, '#'));
        assert!(!city.pass(&mut fake, &none, CHUNK, 33, None));
        // Above sea + 45 nothing is read.
        fake.columns.insert((48, 0), {
            let mut c = column(128, 52, '#');
            c[77] = '#';
            c
        });
        assert!(city.pass(&mut fake, &none, CHUNK, 33, None));
    }

    #[test]
    fn repurposed_shipwreck_needs_no_solid_block_around() {
        let none = BTreeSet::new();
        let wreck = checks(json!({"repurposed": "shipwreck_nether"}));
        // Lava up to 40, air above: the samples at 28..=58 meet only lava and air.
        let mut fake = Fake {
            column: column(128, 40, '~'),
            ..Fake::default()
        };
        assert!(wreck.pass(&mut fake, &none, CHUNK, 28, None));
        // Netherrack at y 43, between samples (28 + 15, + 20 are 43, 48).
        let mut rock = column(128, 40, '~');
        rock[43] = '#';
        fake.columns.insert((32 - 16, -16 + 16), rock.clone());
        assert!(!wreck.pass(&mut fake, &none, CHUNK, 28, None));
        rock[43] = '.';
        rock[44] = '#';
        fake.columns.insert((16, 0), rock);
        assert!(wreck.pass(&mut fake, &none, CHUNK, 28, None));
    }

    #[test]
    fn repurposed_end_stronghold_needs_land_around() {
        let none = BTreeSet::new();
        let end = checks(json!({"repurposed": "stronghold_end"}));
        let mut fake = Fake {
            column: column(256, 0, '#'),
            ..Fake::default()
        };
        // Every height 64 by default: 63, 62, ... 55 after the 9 samples.
        assert!(end.pass(&mut fake, &none, CHUNK, 0, None));
        // Land at least 45 above the bottom, after the sample's minus one:
        // 46 at the corner passes, 45 fails.
        fake.heights.insert((32, -16), 46);
        assert!(end.pass(&mut fake, &none, CHUNK, 0, None));
        fake.heights.insert((32, -16), 45);
        assert!(!end.pass(&mut fake, &none, CHUNK, 0, None));
        // Void 70 blocks north fails at once.
        fake.heights.clear();
        fake.heights.insert((32, -86), 0);
        assert!(!end.pass(&mut fake, &none, CHUNK, 0, None));
    }
}
