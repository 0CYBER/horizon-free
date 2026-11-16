//! A mod's biome region in the Overworld or the Nether: its own climate points, chosen for an
//! area by weight beside vanilla's layout, as data a mod registers with
//! [`Registrar::declare_biome_region`](crate::BiomeRegionsRegistrar::declare_biome_region).
use crate::world::dimension::BiomePoint;

/// The dimensions a region can join: the two whose biomes come from a multi-noise preset.
const DIMENSIONS: [&str; 2] = ["minecraft:overworld", "minecraft:nether"];

/// A biome region a mod declares: its biomes laid out by climate, in its own share of the
/// Overworld or the Nether. Vanilla's whole layout is one region of weight 10 beside it.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: mods build it themselves as declare_biome_region's region
#[derive(horizon_mod_api_macros::Wire)]
pub struct BiomeRegion {
    /// `minecraft:overworld` or `minecraft:nether`.
    pub dimension: String,
    /// How common the region is, 1 to 1000, against vanilla's 10. This is the default a new
    /// world records; the world's saved weight wins from then on.
    pub weight: u32,
    /// The region's biomes by climate, as a points biome source's: the nearest point to the
    /// climate at a position inside the region wins. Registration checks point shape,
    /// not biome existence or ownership; world setup must resolve names and refuse missing biomes.
    pub points: Vec<BiomePoint>,
}

impl BiomeRegion {
    /// Build a declaration; registration validates its dimension, weight and points.
    pub fn new(dimension: String, weight: u32, points: Vec<BiomePoint>) -> Self {
        Self {
            dimension,
            weight,
            points,
        }
    }

    /// The highest weight a region may declare.
    pub const MAX_WEIGHT: u32 = 1000;

    /// What the declaration refuses in the region itself, if anything.
    pub fn check(&self) -> Result<(), BiomeRegionError> {
        if !DIMENSIONS.contains(&self.dimension.as_str()) {
            return Err(BiomeRegionError::UnknownDimension);
        }
        if !(1..=Self::MAX_WEIGHT).contains(&self.weight) {
            return Err(BiomeRegionError::InvalidWeight);
        }
        if self.points.is_empty() {
            return Err(BiomeRegionError::NoPoints);
        }
        if !self.points.iter().all(BiomePoint::is_valid) {
            return Err(BiomeRegionError::InvalidPoint);
        }
        Ok(())
    }
}

/// Why a biome region was refused
/// ([`Registrar::declare_biome_region`](crate::BiomeRegionsRegistrar::declare_biome_region)).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum BiomeRegionError {
    /// The region's id is malformed or in a namespace the declaring mod does not own.
    InvalidId,
    /// A region with the id was already declared.
    Duplicate,
    /// The dimension is not `minecraft:overworld` or `minecraft:nether`.
    UnknownDimension,
    /// The weight is 0 or above [`BiomeRegion::MAX_WEIGHT`].
    InvalidWeight,
    /// The region has no points.
    NoPoints,
    /// A point has no biome, an interval outside -2 to 2 or with its min above its max, or an
    /// offset outside 0 to 1.
    InvalidPoint,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(biome: &str) -> BiomePoint {
        BiomePoint {
            biome: biome.into(),
            temperature: (-0.5, 0.5),
            humidity: (0.0, 0.0),
            continentalness: (0.3, 1.0),
            erosion: (-1.0, 1.0),
            depth: (0.0, 0.0),
            weirdness: (-2.0, 2.0),
            offset: 0.0,
        }
    }

    fn region() -> BiomeRegion {
        BiomeRegion {
            dimension: "minecraft:overworld".into(),
            weight: 5,
            points: vec![point("mymod:highland")],
        }
    }

    #[test]
    fn a_region_needs_a_preset_dimension_a_weight_and_valid_points() {
        assert_eq!(region().check(), Ok(()));
        let nether = BiomeRegion {
            dimension: "minecraft:nether".into(),
            ..region()
        };
        assert_eq!(nether.check(), Ok(()));
        for dimension in ["minecraft:the_end", "mymod:sky", "overworld", ""] {
            let other = BiomeRegion {
                dimension: dimension.into(),
                ..region()
            };
            assert_eq!(other.check(), Err(BiomeRegionError::UnknownDimension));
        }
        for (weight, result) in [
            (0, Err(BiomeRegionError::InvalidWeight)),
            (1, Ok(())),
            (1000, Ok(())),
            (1001, Err(BiomeRegionError::InvalidWeight)),
            (u32::MAX, Err(BiomeRegionError::InvalidWeight)),
        ] {
            assert_eq!(
                BiomeRegion { weight, ..region() }.check(),
                result,
                "{weight}"
            );
        }
        let empty = BiomeRegion {
            points: Vec::new(),
            ..region()
        };
        assert_eq!(empty.check(), Err(BiomeRegionError::NoPoints));
        let bad_points = [
            point(""),
            BiomePoint {
                temperature: (0.5, -0.5),
                ..point("mymod:a")
            },
            BiomePoint {
                weirdness: (-2.5, 0.0),
                ..point("mymod:a")
            },
            BiomePoint {
                depth: (f32::NAN, 0.0),
                ..point("mymod:a")
            },
            BiomePoint {
                offset: 1.5,
                ..point("mymod:a")
            },
        ];
        for bad in bad_points {
            let mixed = BiomeRegion {
                points: vec![point("mymod:good"), bad.clone()],
                ..region()
            };
            assert_eq!(
                mixed.check(),
                Err(BiomeRegionError::InvalidPoint),
                "{bad:?}"
            );
        }
    }
}
