//! Declarations for the container mouse input hook (see `hooks/mod.rs`).
// catalog-order: 0359

pub trait Hooks {
    /// Real mouse input on an open non-creative container screen, after the matching
    /// read-only screen Pre hook let it through and before the screen handles it.
    /// `menu` is the screen's live menu, `input` the button, wheel or drag, and
    /// `modifiers` GLFW's bits (shift 1, control 2, alt 4, super 8).
    /// `menu.hovered_slot(host)` uses the same pointer and hit test as the screen.
    /// Use `menu.click(host, ...)` for normal predicted clicks. Cancel to replace
    /// the screen's own handling; its matching screen Post will not fire.
    #[hook(
        side = "client",
        contract = "cancel",
        out = "canceled",
        neoforge = "none (the read-only screen hooks cover NeoForge's ScreenEvent)",
        cancellable = true
    )]
    fn container_mouse_input_pre();
}
