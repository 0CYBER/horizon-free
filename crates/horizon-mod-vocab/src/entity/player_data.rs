//! Refusals when a mod writes a player's inventory, ender chest or saved data.

/// Why a write to an online player's inventory, ender chest, persistent data or hands was
/// refused. Nothing was written when one comes back.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum PlayerDataRefusal {
    /// The handle no longer identifies an online player: it is stale, or the player has left.
    Stale,
    /// The call was made on a client; these writes are the server's.
    Client,
    /// The player is dead; the new player a respawn makes is the one to write to.
    Dead,
    /// The slot is past the last one of that container.
    SlotOutOfRange {
        /// The slot asked for.
        slot: u32,
        /// The last slot there is: 42 for the inventory, 26 for the ender chest.
        last: u32,
    },
    /// The compound would sit 512 levels deep or more in the player's file.
    TooDeep,
    /// The game's item conversion boundary refused the supplied stack: an item the world
    /// lacks, a count or damage the item does not allow, malformed components.
    Stack(crate::StackWriteError),
}

impl From<crate::StackWriteError> for PlayerDataRefusal {
    fn from(error: crate::StackWriteError) -> Self {
        Self::Stack(error)
    }
}

impl crate::NoBackend for PlayerDataRefusal {
    fn no_backend() -> Self {
        Self::Stale
    }
}
