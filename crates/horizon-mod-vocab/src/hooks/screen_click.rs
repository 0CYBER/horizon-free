//! The `screen_click` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0029

pub trait Hooks {
    /// A left or right (`right`) press at `at` (GUI pixels from the screen's
    /// top left, so outside it too) on block `block`'s screen, a mod's, over
    /// no slot: set `button` to a menu button id when it is on one of yours.
    /// Horizon sends it to the server (`ContainerButtonClick`,
    /// `menu_button` there) and the press goes no further.
    #[hook(
        side = "client",
        contract = "replace",
        out = "button",
        scope = "block_entity",
        key = "block",
        neoforge = "none (see screen)"
    )]
    fn screen_click();
}
