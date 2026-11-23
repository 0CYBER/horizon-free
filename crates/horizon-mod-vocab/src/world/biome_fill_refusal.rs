//! Refusals when a mod sets the biomes of a box of a server level.

/// Why `level.fill_biome` changed nothing: the whole box was refused before any cell changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum BiomeFillRefusal {
    /// The level handle belongs to an earlier open of the world, or the level is gone.
    Stale,
    /// The call was made on a client; biomes are written by the server.
    Client,
    /// The level is not the one in place, the one being ticked; only that one is written.
    NotInPlace,
    /// No biome of that id is in the level's biome table.
    UnknownBiome,
    /// A column of the box is not loaded, or the box leaves the world's build height.
    NotLoaded,
    /// The box holds more 4x4x4 cells than one call may set.
    TooManyCells {
        /// The cells the box holds.
        asked: u32,
        /// The most cells one call may set.
        max: u32,
    },
}

impl crate::NoBackend for BiomeFillRefusal {
    fn no_backend() -> Self {
        Self::Stale
    }
}
