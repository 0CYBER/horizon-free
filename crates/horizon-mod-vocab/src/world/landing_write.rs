//! Deferred writes requested by a block landing part.
use crate::BlockState;

/// A write at the block an entity landed on (`Block.fallOn`).
///
/// The complete callback output is checked before it enters the level's queue. If any
/// requested state is invalid, none of that callback's writes or sibling outputs commit.
/// Once the entity batch has ticked, writes apply in request order only if the block
/// still matches the original landed state. That guard is checked once for the batch,
/// not between writes (a preceding write may itself change the block).
// exhaustive: the closed serialized write vocabulary and case order are a link contract.
#[derive(Clone, Copy, Debug, PartialEq, Eq, horizon_mod_api_macros::Wire)]
pub enum LandingWrite {
    /// `Level.setBlockAndUpdate(pos, state)`, notifying neighbors and clients.
    SetBlock(BlockState),
    /// `Level.destroyBlock(pos, drop)`, with or without resource drops.
    DestroyBlock {
        /// Whether the destroyed block drops its resources.
        drop: bool,
    },
}
