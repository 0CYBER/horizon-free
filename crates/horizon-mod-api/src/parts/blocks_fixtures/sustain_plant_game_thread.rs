/// Install soil parts: glass sustains any plant, and grass sustains none.
pub fn sustain_plant_game_thread() {
    struct Soil(bool);
    #[mod_api::horizon_part]
    impl BlockPart for Soil {
        fn can_sustain_plant(
            &self,
            _: &dyn LevelHost,
            _: &mut CanSustainPlantEvent,
        ) -> Option<bool> {
            Some(self.0)
        }
    }
    struct SoilMod;
    #[mod_api::__subscriptions]
    impl Hooks for SoilMod {
        fn register(&self, r: &mut dyn Registrar, _: &crate::RegistrationContext) {
            let glass = r.block_key("minecraft:glass");
            r.block_part(glass, Soil(true));
            let grass = r.block_key("minecraft:grass_block");
            r.block_part(grass, Soil(false));
        }
    }
    let mods: crate::Mods = vec![ModEntry::new("soil_test", Box::new(SoilMod))].into();
    crate::__private::install(mods.clone());
    install_parts(&mods, &crate::parts::tests::rule_schema(), |_| false).unwrap();
    mark_game_thread();
}
