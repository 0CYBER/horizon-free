//! Errors when declaring a mod block entity's live menu data.

/// Why a mod block entity's menu data slots could not be sized.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum BlockEntityDataError {
    /// The handle no longer identifies a loaded entity in the requested level.
    Stale,
    /// The entity is vanilla, not a configured mod holder, or not writable here.
    Unavailable,
    /// The requested vector exceeds the game's mod-menu data-slot limit.
    TooManySlots {
        /// Number of slots submitted, without clamping.
        count: u64,
        /// Maximum number of menu data slots accepted by this game.
        limit: u64,
    },
}

impl crate::NoBackend for BlockEntityDataError {
    fn no_backend() -> Self {
        Self::Unavailable
    }
}
