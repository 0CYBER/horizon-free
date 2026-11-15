use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// The camera this frame is drawn from, `partial` ticks past the last tick, after vanilla
    /// placed it (`Camera.setup`): its position, and its yaw and pitch in degrees. Set them to
    /// draw the world from anywhere: a cinematic or replay camera. Only the picture, the sound's
    /// listener and the billboards facing the camera follow it; the player, where they aim, the
    /// first-person hand and which chunks load stay the player's. Values that aren't finite keep
    /// vanilla's. Not called on the title screen's panorama. API 402.
    pub(in crate::game) fn native_camera_pose(
        &self,
        partial: f32,
        position: &mut [f64; 3],
        yaw: &mut f32,
        pitch: &mut f32,
    ) {
        self.fire_camera_pose(partial, position, yaw, pitch);
    }
}

/// The pose a native mod asks for the first-person hand, each out-of-range
/// or non-finite component replaced by its identity.
pub(in crate::game) fn hand_pose_matrix(values: [f32; 7]) -> glam::Mat4 {
    let valid = |value: f32, min: f32, max: f32, default: f32| {
        if value.is_finite() && (min..=max).contains(&value) {
            value
        } else {
            default
        }
    };
    let [x, y, z, pitch, yaw, roll, scale] = values;
    glam::Mat4::from_translation(glam::Vec3::new(
        valid(x, -16.0, 16.0, 0.0),
        valid(y, -16.0, 16.0, 0.0),
        valid(z, -16.0, 16.0, 0.0),
    )) * glam::Mat4::from_rotation_x(valid(pitch, -3600.0, 3600.0, 0.0).to_radians())
        * glam::Mat4::from_rotation_y(valid(yaw, -3600.0, 3600.0, 0.0).to_radians())
        * glam::Mat4::from_rotation_z(valid(roll, -3600.0, 3600.0, 0.0).to_radians())
        * glam::Mat4::from_scale(glam::Vec3::splat(valid(scale, 0.01, 100.0, 1.0)))
}

/// `shake` scaled by `scale` (Distortion Effects) and bounded on each axis;
/// a value that isn't finite counts as none.
pub(super) fn bounded_shake(shake: Shake, scale: f32) -> Shake {
    let scale = if scale.is_finite() {
        scale.clamp(0.0, 1.0)
    } else {
        1.0
    };
    let turn = |v: f32| {
        if v.is_finite() {
            (v * scale).clamp(-SHAKE_MAX_DEGREES, SHAKE_MAX_DEGREES)
        } else {
            0.0
        }
    };
    let shift = |v: f32| {
        if v.is_finite() {
            (v * scale).clamp(-SHAKE_MAX_OFFSET, SHAKE_MAX_OFFSET)
        } else {
            0.0
        }
    };
    Shake {
        pitch: turn(shake.pitch),
        yaw: turn(shake.yaw),
        roll: turn(shake.roll),
        offset: shake.offset.map(shift),
    }
}
