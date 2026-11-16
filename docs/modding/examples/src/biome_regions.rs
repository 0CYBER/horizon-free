//! A mod's biome regions: one in the Overworld, one in the Nether
//! (`how-to/biome-regions.md`).

// ANCHOR: regions
use mod_api::{BiomePoint, BiomeRegion, Hooks, Registrar, RegistrationContext};

/// A point at the surface; the region's points cover every temperature and weirdness, so the
/// whole region is snowy and only the land's shape picks among them.
fn surface(
    biome: &str,
    humidity: (f32, f32),
    continentalness: (f32, f32),
    erosion: (f32, f32),
) -> BiomePoint {
    BiomePoint {
        biome: biome.into(),
        temperature: (-1.0, 1.0),
        humidity,
        continentalness,
        erosion,
        depth: (0.0, 0.0),
        weirdness: (-1.0, 1.0),
        offset: 0.0,
    }
}

/// Vanilla's Nether point: every interval a single value.
fn nether(biome: &str, temperature: f32, humidity: f32, offset: f32) -> BiomePoint {
    BiomePoint {
        biome: biome.into(),
        temperature: (temperature, temperature),
        humidity: (humidity, humidity),
        continentalness: (0.0, 0.0),
        erosion: (0.0, 0.0),
        depth: (0.0, 0.0),
        weirdness: (0.0, 0.0),
        offset,
    }
}

#[derive(Default)]
pub struct RegionsMod;

#[mod_api::horizon_mod]
impl Hooks for RegionsMod {
    fn register(&self, registrar: &mut dyn Registrar, _: &RegistrationContext) {
        // A deliberately simplified snowy layout, not vanilla's parameter points: the bands follow
        // vanilla's order (ocean, coast, then inland by erosion), but frozen_ocean reaches into the deep
        // ocean band (-1.05), inland runs to 1.0, and temperature and humidity are merged into wide ranges.
        let all = (-1.0, 1.0);
        let inland = (-0.11, 1.0);
        let frostlands = BiomeRegion {
            dimension: "minecraft:overworld".into(),
            weight: 5, // half as common as vanilla's whole layout (10)
            points: vec![
                surface("minecraft:frozen_ocean", all, (-1.05, -0.19), all),
                surface("minecraft:snowy_beach", all, (-0.19, -0.11), all),
                surface("minecraft:frozen_peaks", all, inland, (-1.0, -0.78)),
                surface("minecraft:snowy_slopes", all, inland, (-0.78, -0.375)),
                surface("minecraft:snowy_taiga", (0.1, 1.0), inland, (-0.375, 1.0)),
                surface("minecraft:snowy_plains", (-1.0, 0.1), inland, (-0.375, 1.0)),
            ],
        };
        let ashlands = BiomeRegion {
            dimension: "minecraft:nether".into(),
            weight: 5,
            // Vanilla's own points for these two biomes.
            points: vec![
                nether("minecraft:basalt_deltas", -0.5, 0.0, 0.175),
                nether("minecraft:soul_sand_valley", 0.0, -0.5, 0.0),
            ],
        };
        // A refused region already stops the mods at startup, naming the mod and the reason.
        let _ = registrar.declare_biome_region("frostlands", frostlands);
        let _ = registrar.declare_biome_region("ashlands", ashlands);
    }
}
// ANCHOR_END: regions
