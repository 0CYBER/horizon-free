use crate::game::native_entity_refs::tests::with_game;
use horizon_client::{first_person::Hand, items::ItemStack};

#[test]
#[ignore = "daily"]
fn remote_mob_takes_only_adult_piglin_currency_with_free_offhand() {
    mod_platform::testing::with_mods(Vec::new().into(), || {
        with_game(|g| {
            g.build_registries();
            g.player = Some(
                horizon_client::motion::PlayerMotion::new(
                    g.motion,
                    horizon_core::geometry::Vec3::new(4.5, 100.0, 4.5),
                )
                .unwrap(),
            );
            g.set_allow_commands(true);
            assert!(g.run_command("summon minecraft:piglin 0 100 0"));
            g.mobs[0]
                .extra
                .set_slot_stack(horizon_items::equipment::OFFHAND, None);
            g.mobs[0].extra.age = 0;
            g.inventory[g.selected] = Some(ItemStack::new("minecraft:gold_ingot", 1));
            assert!(g.remote_mob_takes(0, Hand::Main));
            g.mobs[0].extra.age = -24000;
            assert!(!g.remote_mob_takes(0, Hand::Main));
            g.mobs[0].extra.age = 0;
            g.inventory[g.selected] = Some(ItemStack::new("minecraft:golden_apple", 1));
            assert!(!g.remote_mob_takes(0, Hand::Main));
            g.use_item().unwrap();
            assert!(g.using.is_some());
            g.inventory[g.selected] = Some(ItemStack::new("minecraft:gold_ingot", 1));
            g.mobs[0].extra.set_slot_stack(
                horizon_items::equipment::OFFHAND,
                Some(ItemStack::new("minecraft:gold_ingot", 1)),
            );
            assert!(!g.remote_mob_takes(0, Hand::Main));
        })
    });
}
