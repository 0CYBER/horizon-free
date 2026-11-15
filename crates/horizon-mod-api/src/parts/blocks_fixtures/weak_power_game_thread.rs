/// Install a weak-power part on glass for a real-game query.
pub fn weak_power_game_thread() {
    struct WeakPower;
    #[mod_api::horizon_part]
    impl BlockPart for WeakPower {
        fn should_check_weak_power(
            &self,
            _: &dyn LevelHost,
            event: &mut ShouldCheckWeakPowerEvent,
        ) {
            event.check = true;
        }
    }
    struct WeakPowerMod;
    #[mod_api::__subscriptions]
    impl Hooks for WeakPowerMod {
        fn register(&self, r: &mut dyn Registrar, _: &crate::RegistrationContext) {
            let block_key = r.block_key("minecraft:glass");
            r.block_part(block_key, WeakPower);
        }
    }
    let mods: crate::Mods = vec![ModEntry::new("weak_power_test", Box::new(WeakPowerMod))].into();
    crate::__private::install(mods.clone());
    install_parts(&mods, &crate::parts::tests::rule_schema(), |_| false).unwrap();
    mark_game_thread();
}
