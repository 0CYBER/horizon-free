//! The `player_container_open` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0163

pub trait Hooks {
    /// `player` (`name`) opened a menu (NeoForge's `PlayerContainerEvent.Open`,
    /// posted when `ServerPlayer.openMenu` sets the new `containerMenu`): any
    /// menu, vanilla or a mod's. `menu` is the menu type's name
    /// (`minecraft:generic_9x3`, `minecraft:furnace`, `minecraft:merchant`,
    /// `minecraft:crafting`, `minecraft:lectern`, a mod's own), `pos` the
    /// block it is of in `dimension` (none for a trade, a vehicle's, a mod's
    /// native menu or a mount's inventory, which has no menu type in vanilla
    /// and is told as `horizon:mount_inventory`). The player's own inventory
    /// is not one, nor is failed construction. Heard synchronously after construction, the open packet
    /// is queued and full contents sent, with the new menu already current.
    /// Replacing a menu follows its close; even inside one tick, open precedes
    /// `player_container_close`. The player's state is swapped in. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "PlayerContainerEvent.Open",
        cancellable = false
    )]
    fn player_container_open();
}
