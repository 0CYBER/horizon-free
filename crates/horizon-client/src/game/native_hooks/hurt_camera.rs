use super::*;

impl Game<'_> {
    /// The damage tilt and yaw axis `GameRenderer.bobHurt` is about to draw. Cancel to skip the
    /// whole hurt bob, including the death twist, without changing the walking bob or damage.
    pub(in crate::game) fn native_hurt_camera(&self, call: mod_api::__private::HurtCameraCall<'_>) {
        self.fire_hurt_camera(call);
    }
}
