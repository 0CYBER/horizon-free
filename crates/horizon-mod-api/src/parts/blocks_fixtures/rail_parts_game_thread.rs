/// Install rail parts: a plain rail lays east-west for any cart and can't curve, a powered
/// rail can't climb, and an activator rail refuses east-west.
pub fn rail_parts_game_thread() {
    struct Switch;
    #[mod_api::horizon_part]
    impl BlockPart for Switch {
        fn rail_direction(&self, _: LevelSide<'_>, event: &mut RailDirectionEvent<'_>) {
            if event.cart.is_some() {
                *event.shape = RailShape::EastWest;
            }
        }
        fn rail_flexible(&self, _: &dyn LevelHost, event: &mut RailFlexibleEvent) {
            event.flexible = false;
        }
    }
    struct Flat;
    #[mod_api::horizon_part]
    impl BlockPart for Flat {
        fn rail_can_make_slopes(&self, _: &dyn LevelHost, event: &mut RailCanMakeSlopesEvent) {
            event.can = false;
        }
    }
    struct NoEastWest;
    #[mod_api::horizon_part]
    impl BlockPart for NoEastWest {
        fn rail_valid_shape(&self, _: &dyn LevelHost, event: &mut RailValidShapeEvent) {
            if event.shape == RailShape::EastWest {
                event.valid = false;
            }
        }
    }
    struct RailMod;
    #[mod_api::__subscriptions]
    impl Hooks for RailMod {
        fn register(&self, r: &mut dyn Registrar, _: &crate::RegistrationContext) {
            let rail = r.block_key("minecraft:rail");
            r.block_part(rail, Switch);
            let powered = r.block_key("minecraft:powered_rail");
            r.block_part(powered, Flat);
            let activator = r.block_key("minecraft:activator_rail");
            r.block_part(activator, NoEastWest);
        }
    }
    let mods: crate::Mods = vec![ModEntry::new("rail_test", Box::new(RailMod))].into();
    crate::__private::install(mods.clone());
    install_parts(&mods, &crate::parts::tests::rule_schema(), |_| false).unwrap();
    mark_game_thread();
}
