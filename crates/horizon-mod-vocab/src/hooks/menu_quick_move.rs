//! The `menu_quick_move` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0235

pub trait Hooks {
    /// A player shift-clicked slot `slot` of the menu of the mod block
    /// entity at `pos` (`AbstractContainerMenu.quickMoveStack`, the method a
    /// Java menu overrides), on the server, before the game's own routing:
    /// move the stack where your block wants it. `slot` is the menu's index:
    /// below `container_slots` it is the block's own slot (`entity.items`),
    /// from there on the player's inventory (27 main slots, then the hotbar).
    /// `stack` is a copy of what the slot holds; take what you move out of
    /// it (put it in `entity.items`, or in another block through its
    /// `ITEMS` capability) and set `handled`: the slot then holds what is
    /// left of `stack` (whatever you wrote to that slot of `entity.items`),
    /// the game's routing is skipped, and the player's client gets the menu
    /// again. Left unhandled, `entity.items` goes back to what it held and
    /// the game moves the stack as for any container; a later mod sees what
    /// an earlier one did to `stack`.
    ///
    /// For a mod container on a vanilla menu type and on a mod menu type,
    /// the left or right button, a slot that holds something and that the
    /// mod's screen shows; not for a spectator or a dead player, a menu no
    /// longer valid, nor during a drag.
    /// Asked once per click, where vanilla asks `quickMoveStack` again while
    /// it keeps moving the same item. The player's state is swapped in, so
    /// `host.players().acting_player()` names them; `entity` is out of the world
    /// during the call, as in `menu_button`.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "handled",
        scope = "block_entity",
        key = "entity",
        neoforge = "none (a Java menu overrides AbstractContainerMenu.quickMoveStack; no event)",
        cancellable = true
    )]
    fn menu_quick_move();
}
