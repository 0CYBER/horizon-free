use crate::game::native_entity_refs::tests::with_game;
use horizon_client::{
    items::ItemStack,
    mobs::finalize::{OFFHAND, item_stack},
};

fn stack(sword: bool) -> ItemStack {
    horizon_items::item_data::parse_stack(if sword {
        r#"{id:"minecraft:golden_sword",count:1,components:{"minecraft:custom_name":"Blade","minecraft:enchantments":{"minecraft:sharpness":3}}}"#
    } else {
        r#"{id:"minecraft:gold_ingot",count:1,components:{"minecraft:custom_name":"Tribute"}}"#
    }).unwrap().unwrap()
}

#[test]
#[ignore = "daily"]
fn piglin_admire_preserves_components_on_death_and_finish() {
    mod_platform::testing::with_mods(Vec::new().into(), || {
        with_game(|g| {
            g.build_registries();
            let loot = horizon_world::loot::LootData::load(
                &g.options.resources,
                &Default::default(),
                g.res.item_defaults(),
            )
            .unwrap();
            g.server_loot.set(Some(std::sync::Arc::new(loot)));
            g.set_allow_commands(true);
            g.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(horizon_client::chunk::Chunk::empty(0, 0)),
            );
            for sword in [true, false] {
                g.mobs.clear();
                g.dropped.clear();
                assert!(
                    g.run_command("summon minecraft:piglin 0 100 0 {IsImmuneToZombification:1b}")
                );
                g.mobs[0].extra.set_main_hand(None);
                g.mobs[0].extra.age = 0;
                let expected = stack(sword);
                if sword {
                    let id = g.mobs[0].id;
                    g.creatures.ticked.insert(id);
                    g.spawn_item(expected.clone(), [0.0, 100.0, 0.0], [0.0; 3], 0);
                    g.piglin_pickups();
                } else {
                    assert!(g.give_piglin(0, &expected));
                }
                let held = item_stack(&g.mobs[0], OFFHAND).unwrap();
                assert!(held.same_item_same_components(&expected));
                assert_eq!(held.count, 1);
                let corpse = g.mobs[0].clone();
                g.mob_died(&corpse);
                assert!(g.dropped.iter().any(|d| d.stack.count == 1 && d.stack.same_item_same_components(&expected)));
                if sword {
                    let id = g.mobs[0].id;
                    g.finish_admiring(id);
                    let kept = item_stack(&g.mobs[0], 0).unwrap();
                    assert!(kept.same_item_same_components(&expected));
                    assert_eq!(kept.count, 1);
                }
            }
        })
    });
}

#[test]
#[ignore = "daily"]
fn piglin_admire_hit_keeps_baby_gold_and_cancels_adult_barter() {
    mod_platform::testing::with_mods(Vec::new().into(), || {
        with_game(|g| {
            g.build_registries();
            let loot = horizon_world::loot::LootData::load(
                &g.options.resources,
                &Default::default(),
                g.res.item_defaults(),
            )
            .unwrap();
            g.server_loot.set(Some(std::sync::Arc::new(loot)));
            g.set_allow_commands(true);
            g.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(horizon_client::chunk::Chunk::empty(0, 0)),
            );
            for baby in [true, false] {
                g.mobs.clear();
                g.dropped.clear();
                assert!(
                    g.run_command("summon minecraft:piglin 0 100 0 {IsImmuneToZombification:1b}")
                );
                g.mobs[0].extra.age = if baby { -24000 } else { 0 };
                g.mobs[0].extra.set_main_hand(None);
                let id = g.mobs[0].id;
                g.creatures.ticked.insert(id);
                g.spawn_item(
                    ItemStack::new("minecraft:gold_ingot", 1),
                    [0.0, 100.0, 0.0],
                    [0.0; 3],
                    0,
                );
                g.piglin_pickups();
                assert!(item_stack(&g.mobs[0], OFFHAND).is_some());
                horizon_client::mobs::ai::after_hurt(
                    &mut g.mobs[0],
                    &horizon_client::mobs::Damage::player([1.0, 100.0, 0.0]),
                );
                let events = std::mem::take(&mut g.mobs[0].outbox);
                g.handle_mob_events(events).unwrap();
                assert!(item_stack(&g.mobs[0], OFFHAND).is_none());
                assert_eq!(
                    g.mobs[0].extra.main_hand_id(),
                    baby.then_some("minecraft:gold_ingot")
                );
                assert!(g.dropped.is_empty());
            }
        })
    });
}
