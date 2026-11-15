//! The `menu_clicked` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0034

pub trait Hooks {
    /// A click in a menu a mod opened (`native_open_menu`), which moved
    /// nothing: the menu's `slot` (its own first, then the player's
    /// inventory, `native_menu_inventory_slot`; -999 outside), the `button`
    /// and the click's `mode` (`ClickType`'s id) of the player `uuid`.
    /// No player's state is swapped in: the menu helpers take the player's
    /// UUID. `ItemPlace::MenuSlot` reaches the menu's own storage and its player's
    /// inventory and hotbar slots. `PlayerSlot` and `PlayerHand` resolve the online
    /// clicking player through the usual acting-player or guest path. All writes
    /// use transactions; stale menus and player identities refuse writes.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (vanilla AbstractContainerMenu.clicked; NeoForge adds no event or extension for it)"
    )]
    fn menu_clicked();
}
