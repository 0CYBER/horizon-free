//! Biome sources a native mod registers: vanilla's `BiomeSource` with its own
//! placement, as
//! [`register_biome_source`](crate::Registrar::register_biome_source) hands it
//! over once at startup. A level whose stem's `biome_source` is
//! `{"type": "modid:chaos"}`, one
//! [`register_dimension`](crate::Registrar::register_dimension) made or a data
//! pack's, then takes its biomes from the mod's code, whether the game's noise
//! generator or a mod's chunk generator makes its terrain.
//!
//! There is no `Host` here: biomes are placed on the worldgen worker threads,
//! two workers can ask at once, and a source keeps no mutable state.
use std::sync::Arc;

/// A mod's biome source: vanilla's `BiomeSource`, whose code answers for a
/// level stem with `"biome_source": {"type": "modid:chaos"}`. Implemented once,
/// kept by the game; deterministic and stateless.
pub trait BiomeSource: Send + Sync + 'static {
    /// `BiomeSource.possibleBiomes`: the biome ids (`minecraft:desert`, a data
    /// pack's or the mod's own) the source places, at least one. Asked once, at
    /// startup; a level whose world lacks one of them refuses to open, naming it.
    /// Structures and decoration read this list, as vanilla's do.
    fn possible_biomes(&self) -> Vec<String>;
    /// `BiomeResolver.getNoiseBiome`: the biome at quart cell `quart` (a block
    /// position >> 2, as `[x, y, z]`) of a world with `seed`, as an index into
    /// [`possible_biomes`](Self::possible_biomes); an index past the list wraps
    /// around it. Asked for every 4x4x4 cell of every chunk generated, so keep it
    /// cheap and allocation-free. The same seed and cell give the same biome.
    fn noise_biome(&self, quart: [i32; 3], seed: i64) -> u16;
}

/// A registered biome source, its possible biomes listed once at registration.
pub(crate) struct BiomeSourceInfo {
    pub(crate) id: Box<str>,
    pub(crate) biomes: Vec<String>,
    pub(crate) source: Arc<dyn BiomeSource>,
}

/// Registered biome sources for the hidden game integration boundary: each id,
/// its possible biomes and the source.
// hidden: the game's startup adapter reads the registered sources; mods declare them through the registrar.
#[doc(hidden)]
pub fn biome_source_declarations() -> Vec<(String, Vec<String>, Arc<dyn BiomeSource>)> {
    crate::parts::installed_biome_source_list()
        .iter()
        .map(|info| {
            (
                info.id.to_string(),
                info.biomes.clone(),
                info.source.clone(),
            )
        })
        .collect()
}
