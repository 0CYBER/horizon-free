//! The shared `[biome_regions]` server settings, read by the supervisor and the game.

/// `[biome_regions]`: what an existing world does about its saved biome regions. `missing = "vanilla"` opens a
/// world whose region mods are gone with vanilla's biomes there (`"refuse"`, the default, does not start);
/// `adopt_installed = true` takes the installed mods' regions for new chunks.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BiomeRegions {
    pub missing: MissingRegions,
    pub adopt_installed: bool,
}
impl BiomeRegions {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// `[biome_regions] missing`.
#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MissingRegions {
    #[default]
    Refuse,
    Vanilla,
}
