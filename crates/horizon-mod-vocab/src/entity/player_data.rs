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
    /// The player is awaiting respawn.
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

/// The inventories available for a player by UUID, including from their save.
#[derive(Clone, Copy, Debug, PartialEq, Eq, horizon_mod_api_macros::Wire)]
#[non_exhaustive]
pub enum PlayerInventoryKind {
    /// The 36 main slots (hotbar first), boots through helmet, then offhand.
    Player,
    /// The 27 ender chest slots.
    Ender,
}

/// An inventory read by UUID. An empty slot is represented by None.
#[derive(Clone, Debug, PartialEq, Eq, horizon_mod_api_macros::Wire)]
#[non_exhaustive]
pub struct PlayerInventorySnapshot {
    /// Slots in the order described by the inventory kind.
    pub slots: Vec<Option<crate::ItemStack>>,
    /// Whether these slots came from a connected player's live inventory.
    pub online: bool,
}
impl PlayerInventorySnapshot {
    /// Build a snapshot of the given slots.
    pub fn new(slots: Vec<Option<crate::ItemStack>>, online: bool) -> Self {
        Self { slots, online }
    }
}

/// Move one complete stack if the source slot still holds the inspected stack.
#[derive(Clone, Debug, PartialEq, Eq, horizon_mod_api_macros::Wire)]
#[non_exhaustive]
pub struct PlayerInventoryMove {
    /// Source UUID, inventory and zero-based slot.
    pub from: (u128, PlayerInventoryKind, u32),
    /// The exact stack previously read from the source slot.
    pub expected: crate::ItemStack,
    /// Destination UUID and inventory.
    pub to: (u128, PlayerInventoryKind),
}
impl PlayerInventoryMove {
    /// Build a request using a snapshot's stack as the comparison value.
    pub fn new(
        from: (u128, PlayerInventoryKind, u32),
        expected: crate::ItemStack,
        to: (u128, PlayerInventoryKind),
    ) -> Self {
        Self { from, expected, to }
    }
}

/// Why a player's inventory could not be read or a stack could not move.
#[derive(Clone, Debug, PartialEq, Eq, horizon_mod_api_macros::Wire)]
#[non_exhaustive]
pub enum PlayerInventoryError {
    /// No authoritative server is open.
    NoServer,
    /// The open server has no world directory.
    NoWorld,
    /// No readable inventory file exists for the UUID.
    NoSavedPlayer,
    /// The player has joined the player list but has not been placed.
    LoggingIn,
    /// The player is awaiting respawn.
    Dead,
    /// A nested player turn has displaced the player state.
    SwappedOut,
    /// The source and destination name the same inventory.
    SameInventory,
    /// A move between two different files would require two independent commits.
    BothAway,
    /// The slot is now empty or contains a different stack.
    StackChanged,
    /// The destination cannot fit the full stack.
    NoRoom,
    /// A stored stack cannot be represented by the mod API.
    InvalidStack,
    /// A transaction is active and the file write cannot be rolled back.
    Transaction,
    /// The player file cannot keep the moved stack: it nests past the save's
    /// depth limit, so writing it would drop it.
    Unsaveable,
    /// Only the backup of the player file could be read. A move would replace
    /// that backup with the unreadable file, so it is refused; reading still works.
    BackupOnly,
    /// The player save could not finish or the edited file could not be written.
    File(String),
}
impl std::fmt::Display for PlayerInventoryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NoServer => "no server",
            Self::NoWorld => "no world",
            Self::NoSavedPlayer => "no saved player",
            Self::LoggingIn => "the player is logging in",
            Self::Dead => "the player is dead",
            Self::SwappedOut => "the player's inventory is swapped out",
            Self::SameInventory => "the same inventory",
            Self::BothAway => "both players are away",
            Self::StackChanged => "that stack changed",
            Self::NoRoom => "no room",
            Self::InvalidStack => "the stack cannot be represented",
            Self::Transaction => "inside an open transaction",
            Self::Unsaveable => "the player's file cannot keep that stack",
            Self::BackupOnly => "only the player's backup file is readable",
            Self::File(error) => return write!(f, "the player's file: {error}"),
        })
    }
}
impl std::error::Error for PlayerInventoryError {}
impl crate::NoBackend for PlayerInventoryError {
    fn no_backend() -> Self {
        Self::NoServer
    }
}

/// Why a whole saved player file could not be read or written by `offline_player_data` or
/// `set_offline_player_data`. Nothing was written when one comes back.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum OfflinePlayerDataRefusal {
    /// No authoritative server is open, or this is a client or a remote world.
    NoServer,
    /// The player is online: their live state is written with the player writes
    /// (`player_set_inventory_slot` and the others), not with their file.
    Online,
    /// The file could not be written; the saved file stays as it was.
    Write,
}
