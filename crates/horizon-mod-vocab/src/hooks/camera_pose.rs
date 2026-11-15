//! The `camera_pose` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The camera this frame is drawn from, `partial` ticks past the last tick, after vanilla
    /// placed it (`Camera.setup`): its position, and its yaw and pitch in degrees. Set them to
    /// draw the world from anywhere: a cinematic or replay camera. Only the picture, the sound's
    /// listener and the billboards facing the camera follow it; the player, where they aim, the
    /// first-person hand and which chunks load stay the player's. Values that aren't finite keep
    /// vanilla's. Not called on the title screen's panorama.
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "ViewportEvent.ComputeCameraAngles",
        cancellable = false
    )]
    fn camera_pose();
}
