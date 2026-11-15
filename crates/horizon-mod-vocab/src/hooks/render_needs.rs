//! The `render_needs` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0070

pub trait Hooks {
    /// Asked as a frame is built and drawn: set what your mod needs in
    /// `RenderNeeds`, including entity runs and the world's depth without a shader pack.
    #[hook(
        side = "client",
        contract = "collect",
        out = "needs",
        target = "mod_render::render::render_needs",
        neoforge = "none (Horizon-only; NeoForge has no counterpart)"
    )]
    fn render_needs();
}
