/// Install a viewpoint part: glass is seen as water from below half its height, and as
/// itself from above.
pub fn state_at_viewpoint_game_thread() {
    struct Pool;
    #[mod_api::horizon_part]
    impl BlockPart for Pool {
        fn state_at_viewpoint(&self, host: &dyn ClientHost, event: &mut StateAtViewpointEvent<'_>) {
            if event.viewpoint[1] < f64::from(event.pos.y) + 0.5
                && let Some(water) = host.block_state_id("minecraft:water")
            {
                *event.seen = water;
            }
        }
    }
    struct ViewpointMod;
    #[mod_api::__subscriptions]
    impl Hooks for ViewpointMod {
        fn register(&self, r: &mut dyn Registrar, _: &crate::RegistrationContext) {
            let glass = r.block_key("minecraft:glass");
            r.block_part(glass, Pool);
        }
    }
    let mods: crate::Mods = vec![ModEntry::new("viewpoint_test", Box::new(ViewpointMod))].into();
    crate::__private::install(mods.clone());
    install_parts(&mods, &crate::parts::tests::rule_schema(), |_| false).unwrap();
    mark_game_thread();
}
