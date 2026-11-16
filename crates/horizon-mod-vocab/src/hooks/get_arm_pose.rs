//! The `get_arm_pose` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// NeoForge's `IClientItemExtensions.getArmPose`: the pose of the arm that holds `stack` in
    /// `hand`, as a humanoid model draws it (local and remote players and humanoid mobs).
    /// `humanoid` borrows both held stacks, the main arm, use state and selected poses
    /// before listeners answer; item declarations are included in those poses.
    /// Set `pose` to a vanilla or registered custom pose and it
    /// replaces what the item would give, whatever the use animation, a loaded crossbow or the
    /// item's own `arm_pose` declaration say; leave it unset to keep those. Asked once a
    /// tick per humanoid model and non-empty hand, never for an empty hand. A two-handed
    /// pose (`bow_and_arrow`, `crossbow_hold`, `crossbow_charge`) still poses the other hand as
    /// vanilla's does. Vanilla's `spear` pose poses an arm only for an
    /// item with a kinetic weapon. Register a custom pose with `ArmPosesRegistrar`,
    /// return `ArmPose::Custom` here, and write its angles in `arm_pose_transform`.
    /// Unknown registration slots are ignored. Mob-specific animations still run after
    /// the humanoid base pose, as in vanilla. Client-side, with a `ClientHost`.
    #[hook(
        side = "client",
        contract = "replace",
        out = "pose",
        neoforge = "IClientItemExtensions.getArmPose"
    )]
    fn get_arm_pose();
}
