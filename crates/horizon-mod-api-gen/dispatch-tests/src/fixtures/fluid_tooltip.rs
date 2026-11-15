#[test]
fn fluid_tooltip_lets_each_mod_edit_the_lines_in_turn() {
    struct Edit(u32);
    #[mod_api::__subscriptions]
    impl Hooks for Edit {
        fn fluid_tooltip(&self, _: &dyn ClientHost, event: &mut FluidTooltipEvent<'_>) {
            // Each mod sees what the mods before it left.
            assert_eq!(event.lines.len() as u32, self.0);
            event.lines.push((Text, self.0));
        }
    }
    install(
        vec![
            ModEntry::new("first", Box::new(Edit(0))),
            ModEntry::new("second", Box::new(Edit(1))),
        ]
        .into(),
    );
    let mut lines = Vec::new();
    dispatch_fluid_tooltip(
        &TestHost,
        calls::FluidTooltipCall {
            stack: &FluidStack,
            lines: &mut lines,
        },
    );
    assert_eq!(lines.iter().map(|line| line.1).collect::<Vec<_>>(), [0, 1]);
}

mod stubs {
    pub(super) struct FluidStack;
}
