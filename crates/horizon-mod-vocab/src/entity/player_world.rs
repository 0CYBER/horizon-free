//! Refusals when a mod shows, places or acknowledges blocks as a player, or asks what a player sees.

/// Why `player.sees`, `player.ack_block_changes`, `player.show_block` or `player.place_block`
/// answered with nothing done: nothing was sent, written or queued.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum PlayerWorldRefusal {
    /// A player handle (the viewer, the target or the player acted as) no longer identifies a
    /// player in the server's list: it is stale, or the player has left.
    Stale,
    /// The call was made on a client; these are the server's.
    Client,
    /// The player has no connection to send to (a fake player).
    NoConnection,
    /// The player is dead; the new player a respawn makes is the one to act as.
    Dead,
    /// `ack_block_changes` with a sequence below 0.
    NegativeSequence {
        /// The sequence given.
        sequence: i32,
    },
    /// `show_block` at a position the block update packet cannot carry: y outside
    /// -2048..=2047, x or z outside -33554432..33554432.
    PositionNotSendable {
        /// The position given.
        pos: [i32; 3],
    },
    /// The block state is not in the world's registry.
    UnknownState,
    /// `place_block`'s level handle is of an earlier open of the world.
    StaleLevel,
    /// `place_block` in a level that is not the one in place, where the player is.
    OtherLevel,
    /// `place_block` above or below the world or outside the 30,000,000 block limit.
    OutOfWorld {
        /// The position given.
        pos: [i32; 3],
    },
    /// `place_block` where the chunk is not loaded.
    NotLoaded {
        /// The position given.
        pos: [i32; 3],
    },
    /// `place_block` of the state already standing there: `setBlock` writes nothing.
    Unchanged,
}

impl crate::NoBackend for PlayerWorldRefusal {
    fn no_backend() -> Self {
        Self::Stale
    }
}

impl From<crate::BlockStateError> for PlayerWorldRefusal {
    fn from(error: crate::BlockStateError) -> Self {
        match error {
            // The block state tables are not installed: no world is open.
            crate::BlockStateError::Unavailable => Self::Stale,
            _ => Self::UnknownState,
        }
    }
}
