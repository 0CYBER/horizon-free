pub(super) struct RenderLivingHooks;
impl RenderLivingHooks {
    pub(super) fn render_living_pre(
        &self,
        _: &dyn mod_api::ClientHost,
        event: &mut mod_api::RenderLivingPreEvent<'_>,
    ) -> mod_api::Flow {
        assert!(event.hidden.contains_named("nose"));
        event.hidden.hide_named("hat_rim").unwrap();
        event.hidden.hide(mod_api::HumanoidPart::Head);
        mod_api::Flow::Continue
    }
    pub(super) fn render_living_post(
        &self,
        _: &dyn mod_api::ClientHost,
        event: &mut mod_api::RenderLivingPostEvent<'_>,
    ) {
        assert_eq!(event.entity.kind, "test:mob");
        assert_eq!(event.entity.custom_name, Some("custom"));
        event
            .draw
            .set_light(if event.parts.is_some() { 9 } else { 7 });
    }
}
