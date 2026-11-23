//! The `container_click` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Paper's `InventoryClickEvent` for a click in the player's own inventory menu (`window` 0)
    /// or in an open block container menu (a chest, barrel, shulker box, hopper, dispenser, furnace,
    /// brewing stand or the ender chest), asked on the server before the click runs
    /// (`ServerGamePacketListenerImpl.handleContainerClick`). `slot` is the menu slot the packet
    /// names (-999 outside the window, -1 a drag's outside), `button` and `mode` are its button and
    /// `ClickType` id (0 pickup, 1 quick move, 2 swap, 3 clone, 4 throw, 6 pickup all). `carried` is
    /// the stack on the cursor, `clicked` the stack the client was last sent for that slot; either
    /// is none when empty. Set `canceled` and the click changes nothing: the whole menu and the
    /// cursor go back to the player's client (`sendAllDataToRemote`). Menus a mod opened with
    /// `open_menu` keep `menu_clicked`; a crafting table, a station, a trade and a mount's
    /// inventory are not asked yet, and a drag (`mode` 5) is not asked. A spectator's or a dead
    /// player's click is not asked either, as it changes nothing. Server-side.
    ///
    /// `container_click(host, event)` tells a mod about a click in the player's inventory menu or in a block container before the game acts on it, with the cursor stack in `event.carried`. A guard that keeps stacks out of a container, a live inventory view or a grave's menu reads the cursor and the clicked slot here and returns `Flow::Cancel`: nothing moves, and the client is sent the menu and the cursor again, as vanilla answers a click it refuses. `event.player` is the clicking player's UUID, and the usual player helpers reach them (their state is swapped in). The hook is not asked for menus a mod opened (`menu_clicked` hears those after the fact), for a crafting table, station, trade or mount menu, or for a drag.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "canceled",
        cancellable = true,
        neoforge = "none (NeoForge has no event for a container click)"
    )]
    fn container_click(
        player: u128,
        window: i32,
        slot: i32,
        button: i32,
        mode: i32,
        carried: Option<&ItemStack>,
        clicked: Option<&ItemStack>,
        canceled: &mut bool,
    );
}
