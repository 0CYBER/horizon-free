use super::*;

impl Game<'_> {
    /// Transform only a selected registered humanoid pose, before species animation.
    pub(in crate::game) fn native_arm_pose_transform(
        &self,
        pose: mod_api::CustomArmPose,
        humanoid: &mod_api::HumanoidRenderState<'_>,
        right_arm: bool,
        angles: &mut mod_api::ArmAngles,
    ) {
        self.fire_arm_pose_transform(pose, humanoid, right_arm, angles);
    }
}
