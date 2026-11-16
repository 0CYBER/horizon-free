//! The `render_frame_pre` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Once a frame, in a world or not, after Horizon's extras and just before
    /// the frame is rendered (NeoForge's `RenderFrameEvent.Pre`). `elapsed` is
    /// the frame's time. `world_partial_tick` is the interpolation about to be
    /// drawn, held while paused or frozen; `partial_tick` is the window clock,
    /// which keeps advancing. Outside a world both use the window clock.
    /// Listener writes do not change this frame's captured interpolation. Client-side.
    #[hook(
        side = "client",
        contract = "listen",
        version = 2,
        neoforge = "RenderFrameEvent.Pre"
    )]
    fn render_frame_pre();
}
