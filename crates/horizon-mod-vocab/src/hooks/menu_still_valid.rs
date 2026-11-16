//! The `menu_still_valid` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Whether a player's block menu of a mod menu type stays open
    /// (`AbstractContainerMenu.stillValid`). Asked on the server each tick a
    /// player has the menu of the block at `pos` open, and before each click
    /// on it. `valid` arrives as vanilla's answer: the block entity is there
    /// and `eye` is within the player's block interaction range plus 4 of each
    /// block behind the menu; a part that leaves it alone keeps that answer.
    /// Set it to keep the menu open or close it. Vanilla menu types are never
    /// asked, and a menu whose block entity is gone closes without asking.
    /// Runs in the restricted context; answer from the event and context
    /// alone, and fast.
    #[notes = r##"### Keeping a block menu open

`menu_still_valid` is vanilla's `AbstractContainerMenu.stillValid(Player)`, the method a Java menu overrides to
say whether a player may keep it open. The server asks the scoped `MenuPart` of the open menu's type (a
content mod's menu type, opened on its block) each tick a player has that menu open, and before each click,
drag, shift-click or button press on it. Vanilla menu types keep their own check and never ask a part. A false
answer closes the menu as walking away does: what the cursor held goes back to the inventory and the client's
window closes.

`event.valid` arrives as the game's own answer: the block entities behind the menu are there and the player's
eyes (`event.eye`) are within their block interaction range (4.5, 5 in creative) plus 4 of each of them.
`event.pos` is the menu's block, `event.player` the player's UUID and `event.menu` the menu type. Set `valid`
to `true` to keep the menu of a large machine open from anywhere on its shell, or to `false` to close it for
a reason of your own; a part that leaves it alone, or whose mod is switched off, keeps the game's answer. A
menu whose block entity is gone closes without asking, whatever a part would answer, and so does every menu
when its player respawns, changes dimension or leaves.

The hook runs in the loop context (no level or game Host) on every open menu of the type each tick, so answer
from the event and the context alone, and fast. Whether the type has a part that answers is looked up once,
when the menu opens: a menu with none does no work for this hook."##]
    #[hook(
        side = "server",
        contract = "listen",
        scope = "menu",
        key = "menu",
        context = "loop",
        target = "horizon_client_lib::lan::menu::native_menu_still_valid",
        neoforge = "none (vanilla AbstractContainerMenu.stillValid, which each menu overrides)"
    )]
    fn menu_still_valid();
}
