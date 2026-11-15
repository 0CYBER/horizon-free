//! `Block.popResource` refuses its item while `block_drops` is off
//! (`Block.java:434-440`): it draws its three offsets, then the private overload
//! every behaviour-level pop goes through reads the rule. Tall-flower and
//! flower-bed bone meal (`TallFlowerBlock.java:29`, `FlowerBedBlock.java:95`)
//! and the cave-vine and sweet-berry harvests (`CaveVines.java:34`,
//! `SweetBerryBushBlock.java:122`) all pop through it, so they drop nothing with
//! the rule off and their usual item with it on.
use super::*;
use crate::game::game_tests::tests::server_game;
use horizon_client::items::ItemStack;
use horizon_core::position::ChunkPos;

fn game(resources: &mut Option<std::sync::Arc<crate::game::Resources>>) -> Game<'_> {
    let mut game = server_game(resources);
    game.build_registries();
    game.set_allow_commands(true);
    game.block_ticks
        .add_empty_chunk(ChunkPos::new(0, 0))
        .unwrap();
    assert!(game.run_command("fill 0 99 0 15 99 15 stone"));
    game
}

/// `ServerPlayerGameMode.useItemOn` on the block's top face.
fn hit(pos: [i32; 3]) -> Hit {
    Hit {
        pos,
        inside: false,
        face: [0, 1, 0],
        point: [
            f64::from(pos[0]) + 0.5,
            f64::from(pos[1]) + 1.0,
            f64::from(pos[2]) + 0.5,
        ],
    }
}

/// Clicks the block with `stack` in the main hand: the block's own use, then
/// the item's (`BoneMealItem.useOn`). True when something was used.
fn use_block(game: &mut Game<'_>, pos: [i32; 3], stack: Option<ItemStack>) -> bool {
    game.dropped.clear();
    let hit = hit(pos);
    game.forced_hit = Some(hit);
    *game.held_mut() = stack;
    let used = game.use_hand(Some(&hit)).unwrap();
    game.forced_hit = None;
    *game.held_mut() = None;
    used
}

fn place(game: &mut Game<'_>, pos: [i32; 3], block: &str) {
    assert!(game.run_command(&format!(
        "setblock {} {} {} {block}",
        pos[0], pos[1], pos[2]
    )));
}

/// (block, item it pops, how many stacks, is bone meal its trigger,
/// rule-off spot, rule-on spot)
const CASES: [(&str, &str, usize, bool, [i32; 3], [i32; 3]); 4] = [
    (
        "minecraft:peony",
        "minecraft:peony",
        1,
        true,
        [2, 100, 2],
        [2, 100, 6],
    ),
    (
        "minecraft:pink_petals[flower_amount=4]",
        "minecraft:pink_petals",
        1,
        true,
        [6, 100, 2],
        [6, 100, 6],
    ),
    (
        "minecraft:cave_vines[berries=true]",
        "minecraft:glow_berries",
        1,
        false,
        [10, 100, 2],
        [10, 100, 6],
    ),
    (
        // `harvest/sweet_berry_bush` has two pools at age 3.
        "minecraft:sweet_berry_bush[age=3]",
        "minecraft:sweet_berries",
        2,
        false,
        [14, 100, 2],
        [14, 100, 6],
    ),
];

fn stack(meal: bool) -> Option<ItemStack> {
    meal.then(|| ItemStack::new("minecraft:bone_meal", 1))
}

#[test]
#[ignore = "daily"]
fn block_drops_off_stops_the_pop_of_bone_meal_and_harvests() {
    let mut resources = None;
    let mut game = game(&mut resources);
    assert!(game.run_command("gamerule block_drops false"));
    for (block, _, _, meal, off, _) in CASES {
        place(&mut game, off, block);
        let before = game.world.random.clone();
        assert!(use_block(&mut game, off, stack(meal)), "{block} was used");
        assert!(
            game.dropped.is_empty(),
            "{block} drops nothing with block_drops off"
        );
        if meal {
            // `Block.popResource` draws its three offsets before the private
            // overload reads the rule; bone meal rolls no loot in between.
            let mut expected = before;
            for _ in 0..3 {
                expected.next_double();
            }
            assert_eq!(
                game.world.random.next_double(),
                expected.next_double(),
                "{block}: the gated pop still draws its offsets"
            );
        }
    }
    // The harvests still take the berries and leave the bush picked.
    assert!(game.run_command("execute if block 10 100 2 minecraft:cave_vines[berries=false]"));
    assert!(game.run_command("execute if block 14 100 2 minecraft:sweet_berry_bush[age=1]"));
    // Turning the rule back on restores the pop in the same game: the rule
    // is read per use, not taken once when the level is built.
    assert!(game.run_command("gamerule block_drops true"));
    let (block, item, _, meal, _, on) = CASES[0];
    place(&mut game, on, block);
    assert!(use_block(&mut game, on, stack(meal)));
    assert_eq!(game.dropped.len(), 1, "the rule on pops {item} again");
    assert_eq!(&*game.dropped[0].stack.item, item);
}

#[test]
#[ignore = "daily"]
fn block_drops_on_keeps_the_pop() {
    let mut resources = None;
    let mut game = game(&mut resources);
    assert!(
        game.ui
            .rules
            .bool_at(horizon_client::gamerules::vanilla::BLOCK_DROPS)
    );
    for (block, item, drops, meal, _, on) in CASES {
        place(&mut game, on, block);
        let mut expected = game.world.random.clone();
        assert!(use_block(&mut game, on, stack(meal)), "{block} was used");
        assert_eq!(game.dropped.len(), drops, "{block} pops its stacks");
        for dropped in &game.dropped {
            assert_eq!(&*dropped.stack.item, item);
        }
        if meal {
            // `Block.popResource`'s own offsets from the level random.
            let at = super::spill::pop_position(&mut expected, on);
            let p = game.dropped[0].position;
            assert_eq!([p.x, p.y, p.z], at, "{block} pops where vanilla pops it");
        }
    }
}
