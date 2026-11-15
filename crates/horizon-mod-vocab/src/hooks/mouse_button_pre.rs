//! The `mouse_button_pre` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0116

pub trait Hooks {
    /// Raw mouse button input on any screen, before dispatch; cancel skips dispatch and Post.
    /// Button/modifiers use the same GLFW encoding as screen hooks. Synthetic clicks skip it.
    #[hook(
        side = "client",
        contract = "cancel",
        out = "canceled",
        neoforge = "InputEvent.MouseButton.Pre",
        cancellable = true
    )]
    fn mouse_button_pre();
}
