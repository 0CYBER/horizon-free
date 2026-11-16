//! Registered humanoid arm-pose transformers.

pub trait Hooks {
    /// Apply a registered pose (`HumanoidModel.ArmPose`'s transformer).
    /// Asked only for a selected registered pose, once per model and arm during
    /// humanoid extraction, in vanilla's physical-arm application order. Suppressed
    /// poses do not call the transformer. `humanoid` contains the selected poses
    /// after `get_arm_pose` answers. `right_arm` identifies the physical arm, not the hand.
    /// Write absolute pitch, yaw and roll in radians into `angles`; initially all
    /// components are non-finite, meaning preserve the base model angle. Each
    /// non-finite component is ignored. The ordinary swing and species animations
    /// still run afterward. Native and wasm mods receive the same value contract.
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "HumanoidModel.ArmPose"
    )]
    fn arm_pose_transform();
}
