use crate::game::game_tests::tests::server_game;
use horizon_client::{
    brewing::BrewingStand, containers::BlockEntity, gamerules::Value, items::ItemStack,
};

/// A stand whose block state already shows the bottles it holds, so a tick
/// only reports what it changes.
fn stand(items: [Option<ItemStack>; 5], brew_time: i32, fuel: i32) -> BlockEntity {
    let bottles = [items[0].is_some(), items[1].is_some(), items[2].is_some()];
    BlockEntity::Brewing(Box::new(BrewingStand {
        items,
        brew_time,
        total_brew_time: 400,
        fuel,
        total_fuel: 20,
        speed: 1.0,
        ingredient: None,
        last_bottles: Some(bottles),
    }))
}

fn place(game: &mut crate::game::Game<'_>, pos: (i32, i32, i32), entity: BlockEntity) {
    let state = game
        .pack
        .assets
        .registry
        .default_state("minecraft:brewing_stand")
        .unwrap();
    game.world.set(pos.0, pos.1, pos.2, state).unwrap();
    game.world.block_entities.insert(pos, entity);
}

/// `BrewingStandBlockEntity.doBrew` drops the ingredient's remainder with
/// `Containers.dropItemStack`: no `block_drops` test (unlike `Block.popResource`),
/// and the item leaves with no pickup delay.
#[test]
#[ignore = "daily"]
fn a_brewed_remainder_ignores_the_block_drops_rule() {
    let mut resources = None;
    let mut game = server_game(&mut resources);
    assert!(game.ui.rules.set("block_drops", Value::Bool(false)));
    let pos = (8, 64, 8);
    place(
        &mut game,
        pos,
        stand(
            [
                Some(ItemStack::new("minecraft:splash_potion", 1).with_potion("minecraft:awkward")),
                None,
                None,
                // Two: the second one leaves the glass bottle behind.
                Some(ItemStack::new("minecraft:dragon_breath", 2)),
                None,
            ],
            1,
            5,
        ),
    );

    game.tick_brewing_stand(pos).expect("the stand ticks");

    let bottle = game
        .dropped
        .iter()
        .find(|i| &*i.stack.item == "minecraft:glass_bottle")
        .expect("the dragon's breath left its bottle");
    assert_eq!(bottle.pickup_delay, 0, "Containers.dropItemStack has none");
    let Some(BlockEntity::Brewing(b)) = game.world.block_entities.get(&pos) else {
        panic!("no brewing stand");
    };
    let brewed = b.items[0]
        .as_ref()
        .expect("the potion slot still holds one");
    assert_eq!(&*brewed.item, "minecraft:lingering_potion");
    assert_eq!(brewed.potion(), Some("minecraft:awkward"));
    assert_eq!(b.items[3].as_ref().unwrap().count, 1, "one breath is left");
}

/// `BrewingStandBlockEntity.serverTick` calls `setChanged` whenever it takes a
/// fuel item, even when the item's `brewing_fuel` gives 0 uses: the eaten item
/// must not come back from the next save.
#[test]
#[ignore = "daily"]
fn a_fuel_item_with_no_uses_marks_the_chunk() {
    let mut resources = None;
    let mut game = server_game(&mut resources);
    let pos = (8, 64, 8);
    let mut fuel = ItemStack::new("minecraft:blaze_powder", 1);
    fuel.data_mut().set_loot_component(
        "minecraft:brewing_fuel",
        &Some(horizon_client::scoreboard::json_to_tag(
            &serde_json::json!({"uses": 0, "speed_multiplier": 1.0}),
        )),
    );
    place(
        &mut game,
        pos,
        stand([None, None, None, None, Some(fuel)], 0, 0),
    );
    game.world.modified.clear();

    game.tick_brewing_stand(pos).expect("the stand ticks");

    let Some(BlockEntity::Brewing(b)) = game.world.block_entities.get(&pos) else {
        panic!("no brewing stand");
    };
    assert!(b.items[4].is_none(), "the fuel item was taken");
    assert_eq!((b.fuel, b.brew_time), (0, 0), "0 uses brew nothing");
    assert!(
        game.world.modified.contains(&(0, 0)),
        "the taken fuel left the chunk unmarked"
    );
}
