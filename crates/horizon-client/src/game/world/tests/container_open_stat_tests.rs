//! `ChestBlock.useWithoutItem` awards `getOpenChestStat()` for every chest
//! block it serves; `CopperChestBlock` extends `ChestBlock` without overriding
//! it, so a copper chest counts as one opened, waxed or not
//! (`TrappedChestBlock.getOpenChestStat` is the only override). A barrel,
//! hopper, dropper, dispenser and shulker box award their own block's stat.
use super::Game;
use crate::game::native_machines::tests::server_game;
use horizon_client::containers::BlockEntity;

/// Opens `block` at `pos` through the container path with an empty hand, and
/// leaves it open.
fn open(game: &mut Game<'_>, pos: [i32; 3], block: &str) {
    let state = game
        .pack
        .assets
        .registry
        .default_state(block)
        .unwrap_or_else(|| panic!("{block} has no default state"));
    game.world.set(pos[0], pos[1], pos[2], state).unwrap();
    game.world.block_entities.insert(
        (pos[0], pos[1], pos[2]),
        BlockEntity::for_block(block).unwrap(),
    );
    game.open_block_effect_menu(pos);
    assert_eq!(
        game.container,
        Some((pos[0], pos[1], pos[2])),
        "{block} did not open"
    );
}

#[test]
#[ignore = "daily"]
fn a_copper_chest_counts_as_a_chest_opened() {
    let mut game = server_game();
    // `ChestBlock.useWithoutItem` -> `getOpenChestStat()`: `Stats.OPEN_CHEST`
    // for every chest block, `Stats.TRIGGER_TRAPPED_CHEST` for the trapped one.
    let chests = [
        "minecraft:chest",
        "minecraft:copper_chest",
        "minecraft:exposed_copper_chest",
        "minecraft:weathered_copper_chest",
        "minecraft:oxidized_copper_chest",
        "minecraft:waxed_copper_chest",
        "minecraft:waxed_exposed_copper_chest",
        "minecraft:waxed_weathered_copper_chest",
        "minecraft:waxed_oxidized_copper_chest",
    ];
    for (i, block) in chests.iter().enumerate() {
        let pos = [i as i32, 64, 8];
        open(&mut game, pos, block);
        assert_eq!(
            game.stats.get("custom", "minecraft:open_chest"),
            i as i32 + 1,
            "{block} did not count as a chest opened"
        );
    }
    // The trapped chest overrides the stat; the others have their own.
    open(&mut game, [0, 65, 8], "minecraft:trapped_chest");
    assert_eq!(
        game.stats.get("custom", "minecraft:trigger_trapped_chest"),
        1
    );
    for (i, (block, stat)) in [
        ("minecraft:barrel", "minecraft:open_barrel"),
        ("minecraft:shulker_box", "minecraft:open_shulker_box"),
        ("minecraft:hopper", "minecraft:inspect_hopper"),
        ("minecraft:dropper", "minecraft:inspect_dropper"),
        ("minecraft:dispenser", "minecraft:inspect_dispenser"),
    ]
    .iter()
    .enumerate()
    {
        let pos = [i as i32, 66, 8];
        open(&mut game, pos, block);
        assert_eq!(game.stats.get("custom", stat), 1, "{block}: no {stat}");
    }
    // No copper chest counted as a trapped chest.
    assert_eq!(
        game.stats.get("custom", "minecraft:trigger_trapped_chest"),
        1
    );
}
