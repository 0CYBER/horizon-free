//! The `render_frame` declaration read by the mod API generator (see `hooks/mod.rs`).

pub trait Hooks {
    /// Before preparing an in-world frame, after `render_frame_pre`. Return `Flow::Cancel`
    /// to present just the normal HUD and screens over black instead of drawing the world.
    /// This is a decision for this frame only. Not called on the title screen, during
    /// connection, loading or saving screens, or without a renderer.
    ///
    /// A full-screen mod can check `ClientHost::mod_screen` here and draw its background in
    /// `screen_render_post`. The ordinary GUI hooks and screen input still run. World, camera
    /// extraction and hand render hooks do not run for a GUI-only frame; there is no render
    /// camera to read or project against during its GUI hooks. `render_frame_post` still runs,
    /// followed by `flip_frame` if the frame was presented.
    ///
    /// Client-side, with the same cancellation behavior for native and Wasm listeners.
    #[hook(side = "client", contract = "cancel", out = "gui_only")]
    fn render_frame();
}
