//! Refusals when accessing a mod block entity's internal item slots.

/// Why a mod block entity's internal item slot could not be read or written.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum BlockEntityItemError {
    /// The handle no longer identifies a loaded entity in the requested level.
    Stale,
    /// The entity is vanilla, not a configured mod holder, or not writable here.
    Unavailable,
    /// The index is outside the current container's declared slots.
    SlotOutOfBounds {
        /// Submitted index.
        slot: u32,
        /// Current declared slot count.
        slots: u32,
    },
    /// The incoming stack has an unregistered item, air or zero count.
    /// Use None to clear a slot.
    InvalidStack,
    /// The incoming count exceeds the registered or component-overridden limit.
    StackTooLarge {
        /// Submitted count, without clamping.
        count: u32,
        /// Maximum count for this stack.
        limit: u32,
    },
    /// The game's item conversion boundary refused the supplied stack.
    StackRejected(crate::StackWriteError),
}

impl From<crate::StackWriteError> for BlockEntityItemError {
    fn from(error: crate::StackWriteError) -> Self {
        Self::StackRejected(error)
    }
}

impl crate::NoBackend for BlockEntityItemError {
    fn no_backend() -> Self {
        Self::Unavailable
    }
}
