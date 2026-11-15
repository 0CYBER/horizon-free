#![cfg(test)]
use crate::game::lightning::Bolt;
use crate::game::native_entity_refs::tests::with_game;
use horizon_client::items::ItemStack;
use horizon_core::geometry::Vec3;

#[test]
#[ignore = "daily"]
fn lightning_items_take_five_damage_per_strike() {
    mod_platform::testing::with_mods(Vec::new().into(), || {
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            for item in [
                "minecraft:stone",
                "minecraft:netherite_ingot",
                "minecraft:stone",
            ] {
                game.spawn_item_own(ItemStack::new(item, 1), [0.0, 64.0, 0.0], 10);
            }
            let survivor = game.dropped[2].id;
            game.dropped[2].health = 10;
            let mut bolt = Bolt::struck_at(77, [0.5, 64.0, 0.5]);
            let mut resistant = ItemStack::new("minecraft:stone", 1);
            resistant.data_mut().set_other(
                "minecraft:damage_resistant",
                Some(
                    horizon_core::nbt::parse_snbt("{types:\"minecraft:lightning_bolt\"}")
                        .unwrap()
                        .0,
                ),
            );
            game.spawn_item_own(resistant, [0.0, 64.0, 0.0], 10);
            let resistant_id = game.dropped.last().unwrap().id;
            let mut box_stack = ItemStack::new("minecraft:shulker_box", 1);
            box_stack.data_mut().container = vec![
                (0, ItemStack::new("minecraft:stone", 2)),
                (1, ItemStack::new("minecraft:dirt", 1)),
                (2, ItemStack::new("minecraft:glass", 3)),
            ];
            game.spawn_item_own(box_stack, [0.0, 64.0, 0.0], 10);
            game.spawn_item_own(ItemStack::new("minecraft:stone", 1), [0.0, 64.0, 0.0], 10);
            let invulnerable_id = game.dropped.last().unwrap().id;
            game.persist.raw.insert(
                invulnerable_id,
                horizon_core::nbt::parse_snbt("{Invulnerable:1b}")
                    .unwrap()
                    .0,
            );
            game.thunder_hit(&mut bolt);
            assert_eq!(game.dropped.len(), 6);
            for id in [survivor, resistant_id, invulnerable_id] {
                assert_eq!(game.dropped.iter().find(|e| e.id == id).unwrap().health, 5);
            }
            for (item, count) in [
                ("minecraft:stone", 2),
                ("minecraft:dirt", 1),
                ("minecraft:glass", 3),
            ] {
                let e = game
                    .dropped
                    .iter()
                    .find(|e| e.stack.item.as_ref() == item && e.stack.count == count)
                    .unwrap();
                assert_eq!(e.position, Vec3::new(0.0, 64.0, 0.0));
            }
        })
    });
}

struct Spare;
#[mod_api::__subscriptions]
impl mod_api::Hooks for Spare {
    fn entity_invulnerability_check(
        &self,
        _: mod_api::LevelSide<'_>,
        event: &mut mod_api::EntityInvulnerabilityCheckEvent<'_>,
    ) {
        event.invulnerable = true;
    }
}
#[test]
#[ignore = "daily"]
fn lightning_item_listener_can_refuse_damage() {
    mod_platform::testing::with_mods(
        vec![mod_api::ModEntry::new("spare", Box::new(Spare))].into(),
        || {
            with_game(|game| {
                game.options.dedicated = Some(Vec::new());
                game.spawn_item_own(ItemStack::new("minecraft:stone", 1), [0.0, 64.0, 0.0], 10);
                let mut bolt = Bolt::struck_at(77, [0.5, 64.0, 0.5]);
                game.thunder_hit(&mut bolt);
                assert_eq!(game.dropped.len(), 1);
                assert_eq!(game.dropped[0].health, 5);
            })
        },
    );
}
