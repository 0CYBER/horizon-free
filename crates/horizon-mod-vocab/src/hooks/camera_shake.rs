//! The `camera_shake` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0095

pub trait Hooks {
    /// The camera shake native mods ask for in this frame, `partial` ticks
    /// past the last tick (0 to 1): add your pitch, yaw and roll in degrees
    /// and your offset in blocks, along the camera's right, up and back, to
    /// `shake`, so several mods' shakes add up. Read while playing only;
    /// Horizon scales the sum by Distortion Effects and bounds it
    /// (`native_shake_matrix`).
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "ViewportEvent.ComputeCameraAngles"
    )]
    fn camera_shake();
}
