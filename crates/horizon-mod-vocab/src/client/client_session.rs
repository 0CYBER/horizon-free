//! Read-only client session and local-player status.

use crate::BlockPos;

/// The connected server, not the reconnect target of an earlier session.
#[derive(Clone, Debug, PartialEq, Eq)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(horizon_mod_api_macros::Wire)]
pub struct ClientServer {
    /// The integrated world's display name. External connections do not retain
    /// a server display name; this is `None`, never the connecting player's name.
    pub name: Option<String>,
    /// The joined address, with friend-invite secrets redacted. Memory
    /// connections have no address (`None`), not a made-up socket address.
    pub address: Option<String>,
    /// This window owns the connected integrated server. A loopback address
    /// alone does not make this true; a splitscreen guest does not own it.
    pub integrated_local: bool,
    /// Folder name relative to the instance's saves directory for the owned
    /// integrated world. Renaming the world in-game leaves this key unchanged.
    /// External connections and splitscreen guests return `None`.
    pub save_folder: Option<String>,
}

/// The local player's item use as the client sees it (`LivingEntity.isUsingItem`,
/// `getUsedItemHand`, `getTicksUsingItem`, `getUseItemRemainingTicks`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct ItemUse {
    /// The hand holding the item in use.
    pub hand: crate::Hand,
    /// Ticks the item has been used so far.
    pub ticks_used: i32,
    /// Ticks left until the use completes; with `ticks_used`, the item's use duration.
    pub ticks_remaining: i32,
}

impl ItemUse {
    /// An item use of `hand`, `ticks_used` ticks old with `ticks_remaining` left.
    pub fn new(hand: crate::Hand, ticks_used: i32, ticks_remaining: i32) -> Self {
        Self {
            hand,
            ticks_used,
            ticks_remaining,
        }
    }
}

/// Existing local frame counters, sampled only when requested by a mod.
#[derive(Clone, Copy, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(horizon_mod_api_macros::Wire)]
pub struct ClientFrameStats {
    /// The game's most recently computed frames per second (updated about once
    /// a second), not an inverse of a single frame time.
    pub fps: f32,
    /// Frames rendered by this window since it started, including menus.
    pub rendered_frames: u64,
    /// The most recent completed frame's wall interval in nanoseconds, including
    /// waits. `None` before a frame sample exists or after the chart was reset.
    pub frame_time_ns: Option<u64>,
}

/// The local player's current block-breaking operation. An unavailable player
/// returns `None` from the handle, distinct from [`Self::Idle`].
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum BlockBreakingState {
    /// No block is currently being broken.
    Idle,
    /// The client's current breaking target and predicted progress.
    Breaking {
        /// The target block.
        position: BlockPos,
        /// The accumulated breaking fraction, as the client holds it.
        progress: f32,
        /// The visible crack stage, 0 through 9; absent while progress is zero.
        stage: Option<u8>,
    },
}
