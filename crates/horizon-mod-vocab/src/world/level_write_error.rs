//! Typed refusals of state-bearing writes through a server level handle.
use crate::{BlockStateError, LevelReadError};

/// Why a block write cannot reach its authoritative level.
/// `Ok(false)` instead means vanilla refused an otherwise valid write, for example an
/// unloaded or out-of-height position or an unchanged block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum LevelWriteError {
    /// The level is gone or its handle belongs to an earlier lifetime.
    Stale,
    /// The operation is unavailable on this side, phase or current level.
    Unavailable,
    /// The supplied block state does not exist in the owning registry.
    InvalidState(BlockStateError),
}
impl From<LevelReadError> for LevelWriteError {
    fn from(error: LevelReadError) -> Self {
        match error {
            LevelReadError::Stale => Self::Stale,
            LevelReadError::Unavailable => Self::Unavailable,
        }
    }
}
impl From<BlockStateError> for LevelWriteError {
    fn from(error: BlockStateError) -> Self {
        match error {
            BlockStateError::Unavailable => Self::Unavailable,
            error => Self::InvalidState(error),
        }
    }
}
impl crate::NoBackend for LevelWriteError {
    fn no_backend() -> Self {
        Self::Unavailable
    }
}
