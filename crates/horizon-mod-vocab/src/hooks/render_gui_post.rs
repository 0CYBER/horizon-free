//! The `render_gui_post` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The HUD (not drawn under F1), after the minimap (NeoForge's
    /// `RenderGuiEvent.Post`): place a box with `corners.place` (boxes stack
    /// away from their edge) and draw it with `native_fill` and `native_text`.
    /// Skip a corner `native_corner_shown` says is taken. Not heard when a mod
    /// canceled `render_gui_pre`. Client-side.
    #[hook(side = "client", contract = "listen", neoforge = "RenderGuiEvent.Post")]
    fn render_gui_post();
}
