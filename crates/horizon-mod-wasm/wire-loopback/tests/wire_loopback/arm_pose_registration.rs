pub(super) fn check(registrar: &mut dyn mod_api::Registrar) {
    let two = mod_api::ArmPose::Custom(registrar.register_arm_pose("wire_test:two", true, false));
    let off = mod_api::ArmPose::Custom(registrar.register_arm_pose("wire_test:off", false, true));
    // These assertions run inside the generated guest registration dispatcher,
    // before the host has installed this registration's table.
    assert!(two.two_handed());
    assert!(!two.affects_offhand());
    assert!(!off.two_handed());
    assert!(off.affects_offhand());
}

pub(super) struct ArmPoseRegistrationHooks;
impl ArmPoseRegistrationHooks {
    pub(super) fn register(
        &self,
        registrar: &mut dyn mod_api::Registrar,
        cx: &mod_api::RegistrationContext,
    ) {
        self::check(registrar);
    }
}
