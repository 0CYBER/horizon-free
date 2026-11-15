/// Install enchanting power and beam colour parts: glass is worth two bookshelves and tints
/// a beacon beam red, and a bookshelf is worth nothing.
pub fn enchant_power_and_beam_game_thread() {
    struct Glass;
    #[mod_api::horizon_part]
    impl BlockPart for Glass {
        fn enchant_power_bonus(&self, _: &dyn Host, event: &mut EnchantPowerBonusEvent) {
            event.power = 2.0;
        }
        fn beacon_color_multiplier(
            &self,
            _: &dyn Host,
            event: &mut BeaconColorMultiplierEvent<'_>,
        ) {
            *event.color = Some(0xFFFF_0000);
        }
    }
    struct Bookshelf;
    #[mod_api::horizon_part]
    impl BlockPart for Bookshelf {
        fn enchant_power_bonus(&self, _: &dyn Host, event: &mut EnchantPowerBonusEvent) {
            event.power = 0.0;
        }
    }
    struct PowerMod;
    #[mod_api::__subscriptions]
    impl Hooks for PowerMod {
        fn register(&self, r: &mut dyn Registrar, _: &crate::RegistrationContext) {
            let glass = r.block_key("minecraft:glass");
            r.block_part(glass, Glass);
            let bookshelf = r.block_key("minecraft:bookshelf");
            r.block_part(bookshelf, Bookshelf);
        }
    }
    let mods: crate::Mods = vec![ModEntry::new("power_test", Box::new(PowerMod))].into();
    crate::__private::install(mods.clone());
    install_parts(&mods, &crate::parts::tests::rule_schema(), |_| false).unwrap();
    mark_game_thread();
}
