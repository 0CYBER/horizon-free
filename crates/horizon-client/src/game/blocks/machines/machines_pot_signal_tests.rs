//! `DecoratedPotBlock.getAnalogOutputSignal` (`getRedstoneSignalFromBlockEntity`
//! → `getRedstoneSignalFromContainer`): a pot's one slot lives in the chunk's
//! saved block entity, so a comparator beside it has to read that tag.
use super::*;
use crate::game::{game_tests::tests::server_game, spill};
use horizon_client::save;
use horizon_world::level::LevelAccess;

/// `getRedstoneSignalFromBlockEntity` for the block at `pos`, the path a
/// comparator's `getAnalogOutputSignal` takes.
fn signal(game: &mut Game<'_>, pos: Pos) -> i32 {
    let state = game.world.block(pos.0, pos.1, pos.2);
    game.with_level(|_, level, _| {
        level.block_entity_analog_signal(BlockPos::new(pos.0, pos.1, pos.2), state, Direction::West)
    })
}

fn saved_count(game: &Game<'_>, pos: Pos) -> u32 {
    let tag = spill::saved_entity(&game.world, [pos.0, pos.1, pos.2]).expect("a saved pot");
    save::item_from_nbt(tag.get("item").expect("the pot's item"))
        .expect("a decodable pot item")
        .count
}

#[test]
#[ignore = "daily"]
fn a_pot_feeds_a_comparator_from_its_saved_slot_like_a_chest() {
    let mut resources = None;
    let mut game = server_game(&mut resources);
    game.build_registries();
    game.set_allow_commands(true);
    assert!(game.run_command("fill 0 199 0 15 199 15 stone"));
    let pot = (8, 200, 8);
    let chest = (8, 200, 10);
    assert!(game.run_command("setblock 8 200 8 minecraft:decorated_pot"));
    // Vanilla's signal for one slot (`Mth.lerpDiscrete(count / 64, 0, 15)`); a
    // chest at the same fill fraction in all 27 slots reads the same.
    for (count, expected) in [(64, 15), (32, 8), (1, 1)] {
        assert!(game.run_command(&format!(
            "data merge block 8 200 8 {{item:{{id:\"minecraft:stone\",count:{count}}}}}"
        )));
        let items: Vec<String> = (0..27)
            .map(|slot| format!("{{Slot:{slot}b,id:\"minecraft:stone\",count:{count}}}"))
            .collect();
        assert!(game.run_command(&format!(
            "setblock 8 200 10 minecraft:chest{{Items:[{}]}}",
            items.join(",")
        )));
        assert_eq!(signal(&mut game, pot), expected, "a pot holding {count}");
        assert_eq!(
            signal(&mut game, chest),
            expected,
            "a chest holding {count} in every slot"
        );
    }
    assert!(game.run_command("data remove block 8 200 8 item"));
    assert!(game.run_command("data merge block 8 200 10 {Items:[]}"));
    assert_eq!(signal(&mut game, pot), 0, "an empty pot");
    assert_eq!(signal(&mut game, chest), 0, "an empty chest");
}

#[test]
#[ignore = "daily"]
fn a_hopper_inserted_item_updates_the_comparator_beside_the_pot() {
    let mut resources = None;
    let mut game = server_game(&mut resources);
    game.build_registries();
    game.set_allow_commands(true);
    assert!(game.run_command("fill 0 199 0 15 199 15 stone"));
    let hopper = (7, 200, 8);
    let pot = (8, 200, 8);
    let comparator = BlockPos::new(9, 200, 8);
    assert!(game.run_command(
        "setblock 7 200 8 minecraft:hopper[facing=east,enabled=true]{Items:[{Slot:0b,id:\"minecraft:stone\",count:1}],TransferCooldown:0}"
    ));
    assert!(game.run_command(
        "setblock 8 200 8 minecraft:decorated_pot{item:{id:\"minecraft:stone\",count:63}}"
    ));
    assert!(game.run_command("setblock 9 200 8 minecraft:comparator[facing=west]"));
    let output = |game: &Game<'_>| {
        game.block_ticks
            .comparator_outputs
            .get(&comparator)
            .copied()
            .unwrap_or(0)
    };
    // `ComparatorBlock.tick` on the read side of the comparator.
    let tick = |game: &mut Game<'_>| {
        game.with_level(|b, level, ticks| b.scheduled_block_tick(level, ticks, comparator));
    };
    tick(&mut game);
    assert_eq!(output(&game), 14, "63 items: lerpDiscrete(63/64, 0, 15)");
    for _ in 0..8 {
        game.world.game_time += 1;
        game.tick_hopper(hopper);
    }
    assert_eq!(saved_count(&game, pot), 64, "the hopper filled the pot");
    tick(&mut game);
    assert_eq!(output(&game), 15, "the comparator re-read the saved pot");
}

/// A server's game whose chunk (0, 0) and its neighbours are lit, so its
/// scheduled ticks run, with a player standing beside the pot at (8, 64, 8)
/// and a settled comparator facing it from (9, 64, 8), the pot preset to
/// `item` (an SNBT compound) when given.
fn hand_game<'a>(
    resources: &'a mut Option<std::sync::Arc<crate::game::Resources>>,
    item: Option<&str>,
) -> Game<'a> {
    let mut game = server_game(resources);
    game.settings.simulation_distance = 4;
    game.settings.render_distance = 8;
    game.player = Some(
        horizon_client::motion::PlayerMotion::new(
            game.motion,
            horizon_core::geometry::Vec3::new(6.5, 64.0, 8.5),
        )
        .unwrap(),
    );
    for x in -1..=1 {
        for z in -1..=1 {
            let mut chunk = horizon_client::chunk::Chunk::empty(x, z);
            chunk.lit = true;
            game.world.chunks.insert((x, z), std::sync::Arc::new(chunk));
        }
    }
    game.update_chunk_tickets();
    game.build_registries();
    game.set_allow_commands(true);
    assert!(game.run_command("fill 0 63 0 15 63 15 stone"));
    let nbt = item.map_or(String::new(), |item| format!("{{item:{item}}}"));
    assert!(game.run_command(&format!("setblock 8 64 8 minecraft:decorated_pot{nbt}")));
    assert!(game.run_command("setblock 9 64 8 minecraft:comparator[facing=west]"));
    // Setup only: the comparator's first read, as the tests above take it; a
    // comparator placed by `/setblock` never schedules one itself.
    game.with_level(|b, level, ticks| b.scheduled_block_tick(level, ticks, HAND_COMPARATOR));
    level_ticks(&mut game, 4);
    assert!(!comparator_pending(&game), "the comparator settled");
    game
}

/// Whether the comparator has a scheduled tick waiting.
fn comparator_pending(game: &Game<'_>) -> bool {
    let state = game
        .world
        .block(HAND_COMPARATOR.x, HAND_COMPARATOR.y, HAND_COMPARATOR.z);
    let kind = horizon_core::ticks::TickTypeId(u32::from(game.res.behaviours().block(state).0));
    game.block_ticks.blocks.has_scheduled(HAND_COMPARATOR, kind)
}

const HAND_POT: Pos = (8, 64, 8);
const HAND_COMPARATOR: BlockPos = BlockPos::new(9, 64, 8);

/// `n` ordinary level ticks of the block phases: the comparator's own
/// scheduled tick runs only if something scheduled it.
fn level_ticks(game: &mut Game<'_>, n: u32) {
    for _ in 0..n {
        game.world.game_time += 1;
        game.tick_blocks().unwrap();
    }
}

fn comparator_output(game: &Game<'_>) -> i32 {
    game.block_ticks
        .comparator_outputs
        .get(&HAND_COMPARATOR)
        .copied()
        .unwrap_or(0)
}

/// `ServerPlayerGameMode.useItemOn` on the pot's west face with `stack` in
/// the main hand.
fn insert_by_hand(game: &mut Game<'_>, stack: ItemStack) -> bool {
    let hit = crate::game::Hit {
        pos: [HAND_POT.0, HAND_POT.1, HAND_POT.2],
        inside: false,
        face: [-1, 0, 0],
        point: [8.0, 64.5, 8.5],
    };
    game.forced_hit = Some(hit);
    *game.held_mut() = Some(stack);
    let used = game.use_hand(Some(&hit)).unwrap();
    game.forced_hit = None;
    *game.held_mut() = None;
    used
}

#[test]
#[ignore = "daily"]
fn a_hand_inserted_item_wakes_the_unpowered_comparator_beside_the_pot() {
    let mut resources = None;
    let mut game = hand_game(&mut resources, None);
    assert_eq!(comparator_output(&game), 0, "an empty pot");
    assert!(insert_by_hand(
        &mut game,
        ItemStack::new("minecraft:stone", 1)
    ));
    assert_eq!(saved_count(&game, HAND_POT), 1, "the hand filled the pot");
    level_ticks(&mut game, 4);
    assert_eq!(comparator_output(&game), 1, "lerpDiscrete(1/64, 0, 15)");
}

#[test]
#[ignore = "daily"]
fn a_hand_inserted_item_raises_an_already_powered_comparator() {
    let mut resources = None;
    let mut game = hand_game(&mut resources, Some("{id:\"minecraft:stone\",count:63}"));
    assert_eq!(comparator_output(&game), 14, "lerpDiscrete(63/64, 0, 15)");
    assert!(insert_by_hand(
        &mut game,
        ItemStack::new("minecraft:stone", 1)
    ));
    assert_eq!(saved_count(&game, HAND_POT), 64, "the hand filled the pot");
    level_ticks(&mut game, 4);
    assert_eq!(comparator_output(&game), 15, "a full pot");
}

#[test]
#[ignore = "daily"]
fn a_refused_hand_insert_leaves_the_comparator_beside_the_pot_alone() {
    let mut resources = None;
    let mut game = hand_game(&mut resources, Some("{id:\"minecraft:stone\",count:63}"));
    assert_eq!(comparator_output(&game), 14);
    // `useItemOn` passes to `useWithoutItem`, which only wobbles the pot.
    assert!(insert_by_hand(
        &mut game,
        ItemStack::new("minecraft:dirt", 1)
    ));
    assert_eq!(saved_count(&game, HAND_POT), 63, "the pot kept its stone");
    let beside = game.world.block(7, 64, 8);
    assert_eq!(
        &*game.res.facts().get(beside).name,
        "minecraft:air",
        "no dirt placed"
    );
    level_ticks(&mut game, 4);
    assert_eq!(comparator_output(&game), 14);
}
