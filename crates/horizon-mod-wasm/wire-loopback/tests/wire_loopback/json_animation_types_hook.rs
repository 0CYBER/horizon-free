pub(super) struct JsonAnimationTypesHooks;
impl JsonAnimationTypesHooks {
    pub(super) fn register_json_animation_types(
        &self,
        _: &mod_api::RegistrationContext,
        event: &mut mod_api::RegisterJsonAnimationTypesEvent<'_>,
    ) {
        assert!(
            event.types.is_empty(),
            "the guest starts from its own batch"
        );
        for (target, id) in [(true, "wire_test:pulse"), (false, "wire_test:step")] {
            let id = mod_api::Identifier::parse(id).unwrap();
            event.types.push(if target {
                mod_api::JsonAnimationType::Target(id)
            } else {
                mod_api::JsonAnimationType::Interpolation(id)
            });
        }
    }
}
