//! Weighted biome regions inside a multi-noise preset (Overworld, Nether): the layout a world
//! saved, which the region selector reads. Design: `docs/design/mod-api/mod-biome-regions.md`.
use crate::worldgen::climate::{IndexError, Parameter, ParameterPoint};

/// The salt of the region selector's random stream (`from_hash_of`), in selector version 1.
pub const REGION_SALT: &str = "horizon:biome_region";
/// The width of a region cell in quarts, in selector version 1.
pub const REGION_CELL: u32 = 128;
/// Vanilla's default weight, in selector version 1.
pub const VANILLA_REGION_WEIGHT: u32 = 10;
/// The selector version this build writes for a new world.
pub const REGION_SELECTOR_VERSION: u32 = 1;

/// How a world's regions are chosen. Read from the world's saved layout, never from the
/// constants above at generation time, so a later version can exist beside version 1.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectorParams {
    /// Version 1 fixes the 3x3 neighbourhood; a different neighbourhood needs a new version.
    pub version: u32,
    /// Cell width in quarts.
    pub cell: u32,
    /// The name hashed into the selector's random stream.
    pub salt: String,
    /// Vanilla's region weight.
    pub vanilla_weight: u32,
}
impl SelectorParams {
    /// Version 1's parameters: what a new world records.
    pub fn current() -> Self {
        Self {
            version: REGION_SELECTOR_VERSION,
            cell: REGION_CELL,
            salt: REGION_SALT.to_owned(),
            vanilla_weight: VANILLA_REGION_WEIGHT,
        }
    }
}

/// One biome's climate point in a region, as a mod declared it (`BiomePoint`).
#[derive(Clone, Debug, PartialEq)]
pub struct RegionPoint {
    pub biome: String,
    /// Temperature, humidity, continentalness, erosion, depth, weirdness: `(min, max)`.
    pub intervals: [(f32, f32); 6],
    pub offset: f32,
}
impl RegionPoint {
    /// The point as the climate index stores it (`Climate.parameters`).
    pub fn parameter_point(&self) -> Result<ParameterPoint, IndexError> {
        let mut coordinates = [Parameter::new(0, 0); 6];
        for (coordinate, &(min, max)) in coordinates.iter_mut().zip(&self.intervals) {
            *coordinate = Parameter::span(min, max)?;
        }
        Ok(ParameterPoint {
            coordinates,
            offset: crate::worldgen::climate::quantize(self.offset),
        })
    }
}

/// A mod's region in a saved layout.
#[derive(Clone, Debug, PartialEq)]
pub struct LayoutRegion {
    /// The declaring mod's id.
    pub owner: String,
    /// The region's namespaced id.
    pub id: String,
    /// The weight the world uses (the mod's declared one only as the default at creation).
    pub weight: u32,
    pub points: Vec<RegionPoint>,
    /// The owner chose to resolve this region through vanilla's layout (its mod is missing):
    /// the region keeps its slot and weight, so the other regions' borders do not move.
    pub substituted: bool,
}

/// One dimension's saved region layout: vanilla's region first, then `regions` in their saved
/// (sorted) order, each owning cells by weight.
#[derive(Clone, Debug, PartialEq)]
pub struct RegionLayout {
    pub selector: SelectorParams,
    pub regions: Vec<LayoutRegion>,
}
