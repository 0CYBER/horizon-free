//! Refusals of reads through a level handle.

/// Why a `LevelRef` read cannot return its requested state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum LevelReadError {
    /// The handle's level is gone or has been reopened.
    Stale,
    /// The state is not readable from this side or phase.
    Unavailable,
}

impl crate::NoBackend for LevelReadError {
    fn no_backend() -> Self {
        Self::Unavailable
    }
}
