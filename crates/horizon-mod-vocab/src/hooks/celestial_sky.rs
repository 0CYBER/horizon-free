//! The native celestial sky hook: declaration read by the mod API generator.

pub trait Hooks {
    /// Add textured triangles to the main view's camera-relative sky, before the celestial bodies.
    /// Native only: the renderer lends a bounded geometry batch, not the GPU.
    #[hook(
        side = "client",
        contract = "listen",
        target = "mod_render::render::celestial_sky",
        neoforge = "none (Horizon-only; NeoForge has no counterpart)"
    )]
    fn celestial_sky();
}
