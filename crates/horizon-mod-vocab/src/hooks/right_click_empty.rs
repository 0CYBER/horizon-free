//! The `right_click_empty` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The local player right-clicks empty space with an empty `hand`.
    /// Client-side only; the server does not receive this event.
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "PlayerInteractEvent.RightClickEmpty",
        cancellable = false
    )]
    fn right_click_empty();
}
