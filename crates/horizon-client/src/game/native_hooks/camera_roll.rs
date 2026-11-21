use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// The roll of the camera this frame is drawn from, in degrees about its view direction,
    /// asked right after `camera_pose` with the same `partial`. It starts at 0 each frame and is
    /// kept within a turn. It turns the drawn camera, a front view's too: at +90 the world's up
    /// is at the screen's left, as `Camera.setRotation`'s roll and NeoForge's
    /// `ViewportEvent.ComputeCameraAngles` turn it. The picture, the frustum, the billboards facing the camera, `Camera.view`
    /// and `project` follow it; the first-person hand, the HUD and where the player aims don't.
    /// Distortion Effects doesn't scale it, unlike `camera_shake`. A value that isn't finite is
    /// 0. Not called on the title screen's panorama.
    pub(in crate::game) fn native_camera_roll(&self, partial: f32, roll: &mut f32) {
        self.fire_camera_roll(partial, roll);
    }
}
