/// Install piston parts on real blocks: gold is sticky and sticks only to itself, diamond is
/// sticky, and emerald is a sticky slime block.
pub fn sticky_game_thread() {
    struct Gold;
    #[mod_api::horizon_part]
    impl BlockPart for Gold {
        fn is_sticky_block(&self, _: &dyn LevelHost, event: &mut IsStickyBlockEvent) {
            event.sticky = true;
        }
        fn can_stick_to(&self, _: &dyn LevelHost, event: &mut CanStickToEvent) {
            event.sticks = event.other == event.state;
        }
    }
    struct Diamond;
    #[mod_api::horizon_part]
    impl BlockPart for Diamond {
        fn is_sticky_block(&self, _: &dyn LevelHost, event: &mut IsStickyBlockEvent) {
            event.sticky = true;
        }
    }
    struct Emerald;
    #[mod_api::horizon_part]
    impl BlockPart for Emerald {
        fn is_sticky_block(&self, _: &dyn LevelHost, event: &mut IsStickyBlockEvent) {
            event.sticky = true;
        }
        fn is_slime_block(&self, _: &dyn LevelHost, event: &mut IsSlimeBlockEvent) {
            event.slime = true;
        }
    }
    struct StickyMod;
    #[mod_api::__subscriptions]
    impl Hooks for StickyMod {
        fn register(&self, r: &mut dyn Registrar, _: &crate::RegistrationContext) {
            let gold = r.block_key("minecraft:gold_block");
            r.block_part(gold, Gold);
            let diamond = r.block_key("minecraft:diamond_block");
            r.block_part(diamond, Diamond);
            let emerald = r.block_key("minecraft:emerald_block");
            r.block_part(emerald, Emerald);
        }
    }
    let mods: crate::Mods = vec![ModEntry::new("sticky_test", Box::new(StickyMod))].into();
    crate::__private::install(mods.clone());
    install_parts(&mods, &crate::parts::tests::rule_schema(), |_| false).unwrap();
    mark_game_thread();
}
