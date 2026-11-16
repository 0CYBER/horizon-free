/// Install a redstone connector on glass for real-game tests.
pub fn redstone_game_thread() {
    struct RedstoneMod;
    #[mod_api::__subscriptions]
    impl Hooks for RedstoneMod {
        fn register(&self, r: &mut dyn Registrar, _: &crate::RegistrationContext) {
            let block_key = r.block_key("minecraft:glass");
            r.block_part(block_key, Connector);
        }
    }
    let mods: crate::Mods = vec![ModEntry::new("redstone_test", Box::new(RedstoneMod))].into();
    crate::__private::install(mods.clone());
    install_parts(&mods, &crate::parts::tests::rule_schema(), |_| false).unwrap();
    mark_game_thread();
}
