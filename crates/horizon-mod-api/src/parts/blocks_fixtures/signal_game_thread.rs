/// Install a signal part on glass for a real-game query.
pub fn signal_game_thread() {
    struct Signal;
    #[mod_api::horizon_part]
    impl BlockPart for Signal {
        fn block_signal(&self, level: &dyn LevelHost, event: &mut BlockSignalEvent) {
            assert_eq!(
                level.block_state([event.pos.x, event.pos.y, event.pos.z]),
                Some(event.state)
            );
            event.signal = if event.direct { -2 } else { 20 };
        }
    }
    struct SignalMod;
    #[mod_api::__subscriptions]
    impl Hooks for SignalMod {
        fn register(&self, r: &mut dyn Registrar, _: &crate::RegistrationContext) {
            let block_key = r.block_key("minecraft:glass");
            r.block_part(block_key, Signal);
        }
    }
    let mods: crate::Mods = vec![ModEntry::new("signal_test", Box::new(SignalMod))].into();
    crate::__private::install(mods.clone());
    install_parts(&mods, &crate::parts::tests::rule_schema(), |_| false).unwrap();
    mark_game_thread();
}
