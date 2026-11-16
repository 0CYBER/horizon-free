#[test]
fn foreground_gui_preserves_draws_and_dispatches_with_visibility_flags() {
    struct Indicator(u32);
    #[mod_api::__subscriptions]
    impl Hooks for Indicator {
        fn gui_render_post(&self, _: &dyn ClientHost, event: &mut GuiRenderPostEvent<'_>) {
            // A persistent indicator opts into F1 and hides on focus loss.
            if event.focused {
                event.batch.mark(self.0 + u32::from(event.gui_hidden) * 10);
            }
        }
    }
    install(
        vec![
            ModEntry::new("first", Box::new(Indicator(1))),
            ModEntry::new("second", Box::new(Indicator(2))),
        ]
        .into(),
    );
    for (hidden, focused, expected) in [
        (false, true, vec![99, 1, 2]),
        (true, true, vec![99, 11, 12]),
        (true, false, vec![99]),
    ] {
        let mut batch = HudBatch(vec![99]);
        dispatch_gui_render_post(&TestHost, &mut batch, hidden, focused);
        assert_eq!(batch.0, expected);
    }
    let mut batch = HudBatch(vec![99]);
    install(Vec::<ModEntry>::new().into());
    dispatch_gui_render_post(&TestHost, &mut batch, true, true);
    assert_eq!(batch.0, [99]);
}
