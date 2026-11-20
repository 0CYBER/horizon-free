//! Rules for the terrain a server sends to a player.
use crate::BlockState;

/// An immutable outbound terrain view. All state IDs come from the server's block registry.
#[derive(Clone, Debug, PartialEq, Eq, horizon_mod_api_macros::Wire)]
#[non_exhaustive]
pub struct TerrainView {
    /// First matching band wins. Bands may overlap.
    pub bands: Vec<TerrainBand>,
    /// States that count as covering a face.
    pub occluding: Vec<BlockState>,
    /// Whether to count an unavailable face as covered. Unavailable is distinct from known air.
    pub unavailable_occludes: bool,
    /// Manhattan radius around a successfully broken block to reveal, at most four.
    /// Zero leaves the ordinary exposure rules in effect.
    pub reveal_radius: u8,
}

/// Uses only the authoritative state of the block and its six face neighbours.
#[derive(Clone, Debug, PartialEq, Eq, horizon_mod_api_macros::Wire)]
#[non_exhaustive]
pub struct TerrainBand {
    /// Inclusive lower world Y.
    pub min_y: i32,
    /// Exclusive upper world Y.
    pub max_y: i32,
    /// Eligible states. A block is replaced only when every face is covered.
    pub targets: Vec<BlockState>,
    /// Ordered choices. Repetition weights a state; one choice always selects that state.
    pub replacements: Vec<BlockState>,
    /// Spatial granularity of the deterministic choice.
    pub pattern: TerrainPattern,
}

/// The position used to choose a replacement. The mod supplies the seed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, horizon_mod_api_macros::Wire)]
#[non_exhaustive]
pub enum TerrainPattern {
    /// One choice per block.
    Block(u64),
    /// One choice per horizontal layer in a chunk.
    Layer(u64),
}

/// Why defining or selecting a terrain view failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, horizon_mod_api_macros::Wire)]
#[non_exhaustive]
pub enum TerrainViewError {
    /// No active server, calling mod or connected player.
    Unavailable,
    /// Unregistered view ID or invalid dimension identifier.
    UnknownView,
    /// Another mod owns the player's selection for this dimension.
    Owned,
    /// Invalid bounds, list size, state ID or reveal radius.
    InvalidRules,
    /// The server or player has reached its view limit.
    Limit,
}

impl TerrainView {
    /// Create outbound rules from bands and a face test.
    pub fn new(
        bands: Vec<TerrainBand>,
        occluding: Vec<BlockState>,
        unavailable_occludes: bool,
        reveal_radius: u8,
    ) -> Self {
        Self {
            bands,
            occluding,
            unavailable_occludes,
            reveal_radius,
        }
    }
}

impl TerrainBand {
    /// Create a band covering `min_y..max_y`.
    pub fn new(
        min_y: i32,
        max_y: i32,
        targets: Vec<BlockState>,
        replacements: Vec<BlockState>,
        pattern: TerrainPattern,
    ) -> Self {
        Self {
            min_y,
            max_y,
            targets,
            replacements,
            pattern,
        }
    }
}
