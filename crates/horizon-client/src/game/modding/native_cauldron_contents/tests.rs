use super::*;
use crate::game::api_values::TryIntoApi;
use mod_api::{Hooks, Host};

#[test]
#[ignore = "daily"]
fn registered_contents_and_rebuilt_tables_are_read_through_public_host() {
    use crate::game::native_entity_refs::tests::{run_fresh_cases, with_game_at};
    if run_fresh_cases(
        "game::native_cauldron_contents::tests::registered_contents_and_rebuilt_tables_are_read_through_public_host",
        "HORIZON_CAULDRON_READ_CHILD",
        &["registered"],
    ) {
        return;
    }
    struct Honey;
    #[mod_api::__subscriptions]
    impl Hooks for Honey {
        fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
            r.block_key("test:honey_cauldron");
            r.declare_fluid("honey");
            r.declare_fluid("unregistered");
        }
        fn blocks(&self, _: &mod_api::RegistrationContext, event: &mut mod_api::BlocksEvent<'_>) {
            event.blocks.push(mod_api::Block::new(
                "test:honey_cauldron",
                "minecraft:lava_cauldron",
            ));
        }
        fn register_cauldron_fluid_content(
            &self,
            _: &mod_api::RegistrationContext,
            event: &mut mod_api::RegisterCauldronFluidContentEvent<'_>,
        ) {
            event.contents.register(
                mod_api::__private::block_key("test:honey_cauldron").unwrap(),
                mod_api::__private::fluid_key("test:honey").unwrap(),
                2000,
                None,
            );
        }
    }
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new("test", Box::new(Honey))].into();
    crate::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        let base = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
        let root = crate::game::mods::content_root(&base, &Default::default())
            .unwrap()
            .prepared()
            .to_path_buf();
        with_game_at(root, |game| {
            let block = |id| mod_api::__private::block_key(id).unwrap();
            let fluid = |id| mod_api::__private::fluid_key(id).unwrap();
            let host: &dyn Host = game;
            let water = host
                .cauldron_content_for_block(block("minecraft:water_cauldron"))
                .unwrap();
            assert_eq!(water.fluid, fluid("minecraft:water"));
            assert_eq!(
                (
                    water.total,
                    water.max_level,
                    water.level_property.as_deref()
                ),
                (1000, 3, Some("level"))
            );
            let state = |name: &str| {
                let id = game.block_state_id(name).unwrap();
                id.try_into_api().unwrap()
            };
            assert_eq!(
                host.cauldron_level(state("minecraft:water_cauldron[level=2]")),
                Some(2)
            );
            assert_eq!(
                host.cauldron_level(state("minecraft:lava_cauldron")),
                Some(1)
            );
            assert_eq!(host.cauldron_level(state("minecraft:stone")), None);
            cauldron_contents::without_built_table(|| {
                assert_eq!(
                    host.cauldron_content_for_block(block("minecraft:water_cauldron")),
                    None
                );
                assert_eq!(
                    host.cauldron_content_for_fluid(fluid("minecraft:water")),
                    None
                );
                assert_eq!(host.cauldron_level(state("minecraft:lava_cauldron")), None);
            });
            let honey = host
                .cauldron_content_for_block(block("test:honey_cauldron"))
                .unwrap();
            assert_eq!(honey.fluid, fluid("test:honey"));
            assert_eq!(
                (honey.total, honey.max_level, honey.level_property),
                (2000, 1, None)
            );
            assert_eq!(
                host.cauldron_content_for_fluid(fluid("test:honey"))
                    .unwrap()
                    .block,
                block("test:honey_cauldron")
            );
            assert_eq!(
                host.cauldron_content_for_fluid(fluid("test:unregistered")),
                None
            );
            let old = cauldron_contents::current();
            cauldron_contents::install(cauldron_contents::CauldronContents::default());
            assert_eq!(host.cauldron_content_for_fluid(fluid("test:honey")), None);
            crate::game::mods::register_cauldron_fluid_contents(game.pack.assets.registry);
            assert_eq!(
                host.cauldron_content_for_fluid(fluid("test:honey"))
                    .unwrap()
                    .total,
                2000
            );
            assert_eq!(old.of_fluid("test:honey").unwrap().1.total, 2000);
        });
    });
}
