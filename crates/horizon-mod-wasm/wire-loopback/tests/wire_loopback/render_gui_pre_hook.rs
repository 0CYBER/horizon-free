pub(super) struct RenderGuiPreHooks;
impl RenderGuiPreHooks {
    pub(super) fn subscribed(&self) -> &'static [Option<mod_api::HookId>] {
        &[Some(mod_api::HookId::RenderGuiPre)]
    }
    pub(super) fn listeners(&self) -> &'static [mod_api::Listener] {
        &[mod_api::Listener {
            hook: Some(mod_api::HookId::RenderGuiPre),
            priority: -37,
            receive_cancelled: true,
        }]
    }
    pub(super) fn render_gui_pre(
        &self,
        _: &dyn mod_api::ClientHost,
        event: &mut mod_api::RenderGuiPreEvent,
    ) -> mod_api::Flow {
        assert!(event.cancelled);
        event.cancelled = false;
        mod_api::Flow::Continue
    }
}
