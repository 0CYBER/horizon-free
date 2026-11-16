//! Construction before the immutable pose table exists.
/// Cold authoring access to the opaque token representation.
pub fn arm_pose(slot: u32, both: bool, offhand: bool) -> crate::CustomArmPose {
    crate::CustomArmPose {
        slot,
        both,
        offhand,
    }
}
