//! The `station_closing` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A workstation `PlayerRef::open_station` opened is closing for `player` (UUID): the player
    /// closed it, it was no longer valid or another menu replaced it. `window`
    /// is the id `open_station` returned and `menu` its type. `inputs` holds its input slots
    /// (the crafting grid's nine, row by row; a station's in its menu's order) before they go
    /// back to the player's inventory, or drop when it is full (`clearContainer`). A listener
    /// may empty a slot to keep its stack itself, in an item's container component; only
    /// emptied slots count, and any other change to `inputs` is ignored, so no stack is made
    /// or grown here. Stations the game opened on a block never fire it, nor does a player
    /// leaving: their inputs go back to the inventory before it is saved.
    #[hook(
        side = "server",
        contract = "collect",
        out = "inputs",
        neoforge = "none"
    )]
    fn station_closing();
}
