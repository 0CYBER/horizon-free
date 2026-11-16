/// Install parts that decide what a living entity climbs: glass is a ladder to entities with
/// an even id, and a vine (tagged climbable) is a ladder to none.
pub fn ladder_game_thread() {
    struct Glass;
    #[mod_api::horizon_part]
    impl BlockPart for Glass {
        fn is_ladder(&self, _: &dyn LoopContext, event: &mut IsLadderEvent) {
            event.ladder = event.entity.is_multiple_of(2);
        }
    }
    struct Vine;
    #[mod_api::horizon_part]
    impl BlockPart for Vine {
        fn is_ladder(&self, _: &dyn LoopContext, event: &mut IsLadderEvent) {
            event.ladder = false;
        }
    }
    struct LadderMod;
    #[mod_api::__subscriptions]
    impl Hooks for LadderMod {
        fn register(&self, r: &mut dyn Registrar, _: &crate::RegistrationContext) {
            let glass = r.block_key("minecraft:glass");
            r.block_part(glass, Glass);
            let vine = r.block_key("minecraft:vine");
            r.block_part(vine, Vine);
        }
    }
    let mods: crate::Mods = vec![ModEntry::new("ladder_test", Box::new(LadderMod))].into();
    crate::__private::install(mods.clone());
    install_parts(&mods, &crate::parts::tests::rule_schema(), |_| false).unwrap();
    mark_game_thread();
}
