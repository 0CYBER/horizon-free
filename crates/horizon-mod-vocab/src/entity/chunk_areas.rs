//! Chunk areas a server mod asks for one player (arismoko/horizon#3716, "viewed levels"): squares of chunks in any
//! level that the engine keeps loaded, or ticking, and sends to that player's client, as
//! Immersive Portals' `ChunkLoader` does for what a portal shows. `PlayerRef::set_chunk_areas`
//! sets them (docs/modding/src/how-to/entities/chunk-areas.md).
use crate::{ChunkPos, LevelRef};

/// The chunks within `radius` of `center` (Chebyshev distance: a square of `2 * radius + 1`
/// chunks a side) in `level`, held in `mode`. Built with [`ChunkArea::new`] or a literal; there is
/// no default mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: the square and its mode; mods build it with a literal
#[derive(horizon_mod_api_macros::Wire)]
pub struct ChunkArea {
    /// The level, the player's own or another.
    pub level: LevelRef,
    /// The chunk at the centre of the square.
    pub center: ChunkPos,
    /// How many chunks the square reaches past its centre, 0 to 32.
    pub radius: u32,
    /// What the area's ticket does.
    pub mode: ChunkAreaMode,
}

impl ChunkArea {
    /// The square of chunks within `radius` of `center` in `level`, held in `mode`.
    pub fn new(level: LevelRef, center: ChunkPos, radius: u32, mode: ChunkAreaMode) -> Self {
        Self {
            level,
            center,
            radius,
            mode,
        }
    }
}

/// What an area's ticket does. No default: the mod chooses it for every area.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: the two ticket kinds of the spec; the game handles both
#[derive(horizon_mod_api_macros::Wire)]
pub enum ChunkAreaMode {
    /// Loaded to full, sent and kept up to date; nothing ticks there because of the area
    /// (vanilla's `TicketType.PLAYER_LOADING`).
    Loaded,
    /// Entity and block ticking, random ticks included, as a player's simulation ticket gives
    /// (`TicketType.PLAYER_SIMULATION`). It keeps the level ticking, for those chunks only.
    Ticking,
}

/// Why `set_chunk_areas` changed nothing: the areas asked before stay. Each names the cap it hit
/// and that cap's value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum ChunkAreaRefusal {
    /// The player is not online on this server.
    NotOnline,
    /// A radius over the engine's 32.
    RadiusTooLarge {
        /// The radius asked.
        radius: u32,
        /// The largest: 32, vanilla's largest view distance.
        max: u32,
    },
    /// A centre more than `max` chunks from (0, 0) on an axis.
    CenterOutOfRange {
        /// The centre asked.
        center: ChunkPos,
        /// 1,875,000: the chunk the largest world border's edge is in.
        max: i32,
    },
    /// More than the engine's 32 areas for this player, all mods together.
    TooManyAreas {
        /// The areas the player would have.
        asked: u32,
        /// The most: 32.
        max: u32,
    },
    /// Areas in more levels than the player's client can view.
    TooManyLevels {
        /// The levels besides the player's own the areas would name.
        asked: u32,
        /// The client's own answer; 0 for a client that views none (vanilla, or an older
        /// Horizon).
        cap: u32,
    },
    /// More chunks in `level` than `cap` allows.
    TooManyChunks {
        /// The cap hit.
        cap: ChunkCap,
        /// The level counted.
        level: LevelRef,
        /// The chunks there would be.
        asked: u32,
        /// The setting's value now.
        limit: u32,
    },
    /// No mod's call: a level handle of an earlier open of the world, or a call from outside
    /// any mod's hook (each mod's areas are kept apart by its id).
    Unavailable,
}

impl crate::NoBackend for ChunkAreaRefusal {
    fn no_backend() -> Self {
        Self::Unavailable
    }
}

/// Which chunk cap a refusal hit; each is a server setting (`server.toml` `[server]`).
/// Chunks are counted as the union of every mod's areas, by mode: a `Ticking` area's square
/// counts as ticking, and every chunk loaded for an area that is not ticking counts as loaded
/// (a `Loaded` area loads one ring past its radius, (2r + 3)² chunks, and a `Ticking` area two).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum ChunkCap {
    /// `max_viewed_loaded_chunks`, per player and level: 2048 by default, and at most.
    LoadedPerPlayer,
    /// `max_viewed_ticking_chunks`, per player and level: 441 by default.
    TickingPerPlayer,
    /// `max_viewed_loaded_chunks_per_level`, all players together: 16384 by default.
    LoadedPerLevel,
    /// `max_viewed_ticking_chunks_per_level`, all players together: 3528 by default.
    TickingPerLevel,
}
