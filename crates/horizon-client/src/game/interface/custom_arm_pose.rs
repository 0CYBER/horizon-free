use super::*;
use horizon_core::arm_pose::{ArmPose, application_order};

impl Game<'_> {
    pub(super) fn transform_selected_arm_poses(
        &self,
        mob: &Mob,
        poses: &mut [Option<ArmPose>; 2],
        humanoid: mod_api::HumanoidRenderState<'_>,
    ) {
        use super::api_values::TryIntoApi;
        if !poses
            .iter()
            .any(|pose| matches!(pose, Some(ArmPose::Custom { .. })))
        {
            return;
        }
        let selected =
            horizon_client::mobs::anim::humanoid_arm_poses(mob, *poses, mob.anim.attack_anim);
        let humanoid = mod_api::HumanoidRenderState {
            main_arm_pose: selected[0].try_into_api().unwrap(),
            off_arm_pose: selected[1].try_into_api().unwrap(),
            ..humanoid
        };
        let [right, left] = if mob.main_arm_left() {
            [selected[1], selected[0]]
        } else {
            selected
        };
        for right_arm in application_order(right, left).into_iter().flatten() {
            let hand = usize::from(right_arm == mob.main_arm_left());
            let mut resolved = selected[hand];
            if let ArmPose::Custom { slot, angles, .. } = &mut resolved
                && let Some(registered) = mod_api::CustomArmPose::from_slot(*slot)
            {
                let mut value = mod_api::ArmAngles {
                    right: [f32::NAN; 3],
                    left: [f32::NAN; 3],
                };
                self.native_arm_pose_transform(registered, &humanoid, right_arm, &mut value);
                *angles = [value.right.map(f32::to_bits), value.left.map(f32::to_bits)];
                poses[hand] = Some(resolved);
            }
        }
    }
}
