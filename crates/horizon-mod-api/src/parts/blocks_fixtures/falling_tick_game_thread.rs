/// Install a falling part: falling sand counts its airborne ticks.
pub fn falling_tick_game_thread() {
    struct Sand;
    #[mod_api::horizon_part]
    impl BlockPart for Sand {
        fn falling_tick(&self, _: &mut dyn ServerHost, _: &mut FallingTickEvent) {
            FALLING_TICKS.with(|heard| heard.set(heard.get() + 1));
        }
    }
    struct FallingMod;
    #[mod_api::__subscriptions]
    impl Hooks for FallingMod {
        fn register(&self, r: &mut dyn Registrar, _: &crate::RegistrationContext) {
            let sand = r.block_key("minecraft:sand");
            r.block_part(sand, Sand);
        }
    }
    FALLING_TICKS.with(|heard| heard.set(0));
    let mods: crate::Mods = vec![ModEntry::new("falling_test", Box::new(FallingMod))].into();
    crate::__private::install(mods.clone());
    install_parts(&mods, &crate::parts::tests::rule_schema(), |_| false).unwrap();
    mark_game_thread();
}
