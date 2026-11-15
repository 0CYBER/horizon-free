//! The `render_gui_pre` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Before the in-world HUD is drawn (NeoForge's `RenderGuiEvent.Pre`):
    /// return `Flow::Cancel` and none of it is, as under F1: no vanilla layer (and so
    /// no `gui_layer_pre` or `gui_layer_post`), no `render_gui_post`, and no
    /// chat or toasts. Fires once a frame in a world, not under F1 and not
    /// while a server dialog or the connection screen covers the window.
    /// Client-side.
    #[hook(
        side = "client",
        contract = "cancel",
        out = "canceled",
        neoforge = "RenderGuiEvent.Pre",
        cancellable = true
    )]
    fn render_gui_pre();
}
