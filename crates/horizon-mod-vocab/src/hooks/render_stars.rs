//! The `render_stars` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Before the renderer submits a nonempty stock or `core/stars` draw for the main world view.
    /// `angle_degrees` is the frame's star angle, `brightness` is the applied factor in `(0, 1]`,
    /// and `sky_from_stars` is the applied column-major transform from star space into
    /// camera-relative world space (not into view or clip space). For built-in stars it includes
    /// an active pack's sky path tilt; a resource-pack `core/stars` shader uses its existing pose.
    ///
    /// Cancel to skip only this star draw, not the sun, moon, or sky itself. No callback on the
    /// title or auxiliary views, when the dimension or visibility suppresses the draw, when stars
    /// have zero brightness, or when a full pack sky owns the draw or hides its stars.
    #[hook(
        side = "client",
        contract = "cancel",
        out = "cancel",
        target = "horizon_client_render::gfx::stars::native_render_stars",
        neoforge = "none (vanilla SkyRenderer.renderStars)"
    )]
    fn render_stars();
}
