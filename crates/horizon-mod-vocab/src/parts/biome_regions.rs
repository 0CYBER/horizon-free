use super::*;
/// A biome region declared in code: the declaring mod, the region's id (`mymod:highlands`) and
/// the region. The game writes the declared regions into a new world's saved layout.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: record fixtures and the engine construct declarations
#[derive(horizon_mod_api_macros::Wire)]
pub struct BiomeRegionDeclaration {
    /// The declaring mod's id: the first key of the regions' order.
    pub mod_id: String,
    /// The region's namespaced id: the second key of the regions' order.
    pub id: String,
    /// The dimension, weight and points.
    pub region: crate::BiomeRegion,
}

impl BiomeRegionDeclaration {
    /// Build a declaration record; the registrar validates and qualifies actual registrations.
    pub fn new(mod_id: String, id: String, region: crate::BiomeRegion) -> Self {
        Self { mod_id, id, region }
    }
}

/// The biome regions mods declared, sorted by declaring mod id, then region id: the same mods
/// give the same list whatever order they load in.
pub fn biome_region_declarations() -> &'static [BiomeRegionDeclaration] {
    installed().map_or(&[], |installed| &installed.biome_regions)
}

/// Registrar operations for biome regions.
pub trait BiomeRegionsRegistrar {
    /// Declare a biome region `path` (`highlands` or `mymod:highlands`): the mod's own biomes,
    /// laid out by climate in `region.points`, in a share of the Overworld or the Nether set by
    /// `region.weight` against vanilla's whole layout at 10. Each area belongs to one region,
    /// and the nearest-point search runs among that region's points only, so a region never
    /// competes with another's points.
    ///
    /// The declared weight is the default a new world records; a world's saved layout, not the
    /// mod, decides its regions from then on, and an existing world takes a new or changed
    /// region only when its owner opts in. Regions are ordered by declaring mod id, then
    /// region id.
    ///
    /// An id outside the mod's namespaces, a malformed or repeated id, a dimension other than
    /// `minecraft:overworld` or `minecraft:nether`, a weight of 0 or above 1000, no points, or
    /// a point with no biome, an interval outside -2 to 2 or inverted, or an offset outside 0 to
    /// 1 refuses the mods, naming the mod. Biome existence and ownership are not checked
    /// here; world setup must resolve names and refuse missing biomes.
    fn declare_biome_region(
        &mut self,
        path: &str,
        region: crate::BiomeRegion,
    ) -> Result<(), crate::BiomeRegionError>;
}
