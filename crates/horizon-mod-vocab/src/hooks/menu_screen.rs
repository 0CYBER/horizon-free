//! Layout for a content menu that has no matching block-owned screen.

pub trait Hooks {
    /// Lay out a content menu opened without a matching mod block, keyed by its
    /// registered menu type. Place every container slot declared by that type.
    /// The server can move items into every slot of a blockless menu, so this
    /// layout must show them all. Leaving the answer empty closes the window.
    /// The block-scoped screen and draw hooks still handle block-owned menus.
    /// To add controls, use the screen render and input hooks, matching
    /// the menu type in `ScreenView::menu`.
    #[hook(
        side = "client",
        contract = "replace",
        out = "screen",
        scope = "menu",
        key = "menu",
        neoforge = "RegisterMenuScreensEvent"
    )]
    fn menu_screen(menu: &str, screen: &mut Option<Screen>);
}
