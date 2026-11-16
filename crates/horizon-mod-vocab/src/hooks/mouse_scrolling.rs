//! The `mouse_scrolling` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0126

pub trait Hooks {
    /// Gameplay scrolling after accumulation, before hotbar/spectator handling.
    /// `scroll` is scaled x/y, `accumulated` whole x/y steps, `buttons` left/middle/right,
    /// and `cursor` the GUI position. Cancel skips gameplay scrolling.
    #[hook(
        side = "client",
        contract = "cancel",
        out = "canceled",
        neoforge = "InputEvent.MouseScrollingEvent",
        cancellable = true
    )]
    fn mouse_scrolling();
}
