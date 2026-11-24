//! The `build_stars` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The engine's deterministic `SkyRenderer.buildStars` positions, as a non-indexed triangle
    /// list, before they are uploaded to the GPU. `mesh.vertices()` borrows the current list;
    /// `mesh.replace(vertices)` accepts a whole new list (including empty) or returns a refusal
    /// without changing it. Later listeners see earlier accepted edits. Positions are in unrotated,
    /// camera-relative star space, in blocks, with vanilla stars on a sphere of radius 100.
    ///
    /// Once per renderer, at the first eligible world star draw, after client setup. A renderer
    /// rebuild starts from stock geometry again. The same accepted mesh feeds the built-in stars
    /// and the resource-pack `core/stars` shader; it does not affect a pack's full sky pass.
    ///
    /// Reloading just the stars shader does not rebuild the mesh. Enabling or disabling a listener
    /// after the mesh has been built affects the next renderer, not the current buffer.
    ///
    /// Not on wasm: the hook lends engine geometry to native mods without copying a vertex buffer across Wire.
    #[hook(
        side = "client",
        contract = "listen",
        target = "horizon_client_render::gfx::stars::native_build_stars",
        neoforge = "none (vanilla SkyRenderer.buildStars)"
    )]
    fn build_stars();
}
