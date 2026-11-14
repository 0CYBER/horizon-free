//! Refusals when a mod relights or resends chunks of a server level.

/// Why `level.relight_chunks` or `chunk.resend` did nothing: no chunk was queued.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum ChunkRefusal {
    /// The level or chunk handle is of an earlier open of the world, or the level is gone.
    Stale,
    /// The call was made on a client; chunks are the server's.
    Client,
    /// The level is not the one in place, the one being ticked; only that one is written.
    NotInPlace,
    /// `chunk.resend` of a chunk that is not loaded.
    NotLoaded,
    /// `relight_chunks` was given more chunks than it may queue now.
    TooManyChunks {
        /// The chunks given.
        asked: u32,
        /// The most a call takes now: 256, less while the level's light queue is nearly full.
        max: u32,
    },
}

impl crate::NoBackend for ChunkRefusal {
    fn no_backend() -> Self {
        Self::Stale
    }
}
