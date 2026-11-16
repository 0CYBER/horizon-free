/// Install parts that decide whether a tool harvests: stone is harvested by any tool, bare
/// hand included, and dirt by none.
pub fn harvest_game_thread() {
    struct Answer(bool);
    #[mod_api::horizon_part]
    impl BlockPart for Answer {
        fn can_harvest_block(&self, _: HostSide<'_>, event: &mut CanHarvestBlockEvent) {
            event.can_harvest = self.0;
        }
    }
    struct HarvestMod;
    #[mod_api::__subscriptions]
    impl Hooks for HarvestMod {
        fn player_harvest_check(&self, _: HostSide<'_>, event: &mut PlayerHarvestCheckEvent) {
            event.can_harvest = !event.can_harvest;
        }
        fn register(&self, r: &mut dyn Registrar, _: &crate::RegistrationContext) {
            let stone = r.block_key("minecraft:stone");
            r.block_part(stone, Answer(true));
            let dirt = r.block_key("minecraft:dirt");
            r.block_part(dirt, Answer(false));
        }
    }
    let mods: crate::Mods = vec![ModEntry::new("harvest_test", Box::new(HarvestMod))].into();
    crate::__private::install(mods.clone());
    install_parts(&mods, &crate::parts::tests::rule_schema(), |_| false).unwrap();
    mark_game_thread();
}

std::thread_local! {
    static TOOL_SIMULATIONS: std::cell::RefCell<Vec<bool>> = const { std::cell::RefCell::new(Vec::new()) };
}
