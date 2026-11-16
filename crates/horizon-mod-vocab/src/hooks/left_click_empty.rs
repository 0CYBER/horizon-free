//! The `left_click_empty` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The local player left-clicks empty space, with any main-hand stack.
    /// Client-side only; the server does not receive this event.
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "PlayerInteractEvent.LeftClickEmpty",
        cancellable = false
    )]
    fn left_click_empty();
}
