//! The `mouse_button_post` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0117

pub trait Hooks {
    /// An accepted raw button event finished dispatch, including a screen handler's early return.
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "InputEvent.MouseButton.Post"
    )]
    fn mouse_button_post();
}
