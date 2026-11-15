//! A block's parts decide whether a player breaking it destroyed it (`on_destroyed_by_player`),
//! in place of `playerWillDestroy` and the removal.
use crate::game::{BlockEntity, Game, native_entity_refs::tests::with_game};
use horizon_client::{chunk::Chunk, items::ItemStack, motion::PlayerMotion};
use horizon_core::{geometry::Vec3, position::ChunkPos};
use horizon_world::registry::BlockStateId;
use std::{path::PathBuf, sync::Arc};

const POS: [i32; 3] = [5, 63, 4];

/// A survival player standing in an empty chunk of a serving world (the parts' writes need a
/// server), with the destroy fixture's parts installed on this thread. Returns the scratch
/// world folder to remove.
fn breaking_player(game: &mut Game<'_>) -> PathBuf {
    game.options.dedicated = Some(Vec::new());
    let scratch = std::env::temp_dir().join(format!(
        "horizon-destroyed-part-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&scratch).unwrap();
    game.world_root = scratch.clone();
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(4.5, 62.0, 4.5)).unwrap());
    game.player.as_mut().unwrap().ground.on_ground = true;
    game.creative = false;
    game.build_registries();
    game.set_allow_commands(true);
    assert!(game.run_command("gamemode survival"));
    game.world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    game.block_ticks
        .add_empty_chunk(ChunkPos::new(0, 0))
        .unwrap();
    scratch
}

fn place(game: &mut Game<'_>, block: &str) -> BlockStateId {
    let state = game.pack.assets.registry.default_state(block).unwrap();
    game.world.set(POS[0], POS[1], POS[2], state).unwrap();
    state
}

fn at(game: &Game<'_>) -> BlockStateId {
    game.world.block(POS[0], POS[1], POS[2])
}

fn dropped(game: &Game<'_>, item: &str) -> u32 {
    game.dropped
        .iter()
        .filter(|dropped| &*dropped.stack.item == item)
        .map(|dropped| dropped.stack.count)
        .sum()
}

#[test]
#[ignore = "daily"]
fn a_block_part_decides_whether_a_player_breaking_a_block_destroyed_it() {
    // The fixture's parts: snow loses one layer per break and stays, down to the last, which
    // vanilla removes; glass is removed by its part, which answers that it was destroyed.
    mod_platform::__private::part_tests::destroyed_by_player_game_thread();
    with_game(|game| {
        let scratch = breaking_player(game);
        let registry = game.pack.assets.registry;
        let snow = |layers: u32| {
            registry
                .resolve(&horizon_world::chunk::BlockState {
                    name: "minecraft:snow".into(),
                    properties: [("layers".to_string(), layers.to_string())].into(),
                })
                .unwrap()
        };
        let air = registry.default_state("minecraft:air").unwrap();

        // Three layers, broken by a bare hand: each break takes a layer, drops its snowball and
        // leaves the block; the last layer is vanilla's removal.
        game.world.set(POS[0], POS[1], POS[2], snow(3)).unwrap();
        game.dropped.clear();
        for (layers, snowballs) in [(2, 1), (1, 2)] {
            game.break_block(POS).unwrap();
            assert_eq!(at(game), snow(layers), "{layers} layers are left");
            assert_eq!(dropped(game, "minecraft:snowball"), snowballs);
        }
        game.break_block(POS).unwrap();
        assert_eq!(at(game), air);
        // The bare hand never harvests snow, and the stack it holds is empty: the part's own
        // snowballs are the only ones.
        assert_eq!(dropped(game, "minecraft:snowball"), 2);
        assert_eq!(
            mod_platform::__private::part_tests::take_destroys(),
            vec![(false, 0); 3]
        );

        // The part removes glass itself and answers that it did: a destroyed block.
        place(game, "minecraft:glass");
        game.break_block(POS).unwrap();
        assert_eq!(at(game), air);
        // Glass needs no tool, so a bare hand harvests it.
        assert_eq!(
            mod_platform::__private::part_tests::take_destroys(),
            vec![(true, 0)]
        );

        // No part: vanilla's removal and drops.
        place(game, "minecraft:dirt");
        game.dropped.clear();
        game.break_block(POS).unwrap();
        assert_eq!(at(game), air);
        assert_eq!(dropped(game, "minecraft:dirt"), 1);
        assert_eq!(mod_platform::__private::part_tests::take_destroys(), vec![]);
        std::fs::remove_dir_all(scratch).unwrap();
    });
}

#[test]
#[ignore = "daily"]
fn a_part_keeping_the_block_replaces_player_will_destroy_in_creative() {
    // The fixture's parts keep a shulker box and a tripwire (`Some(false)`).
    mod_platform::__private::part_tests::destroyed_by_player_game_thread();
    with_game(|game| {
        let scratch = breaking_player(game);
        assert!(game.run_command("gamemode creative"));
        game.creative = true;
        let air = game
            .pack
            .assets
            .registry
            .default_state("minecraft:air")
            .unwrap();
        let key = (POS[0], POS[1], POS[2]);
        // A creative break of a filled shulker box drops it with its contents
        // (ShulkerBoxBlock.playerWillDestroy); the part's box keeps its block and drops nothing.
        for (block, kept) in [
            ("minecraft:shulker_box", true),
            ("minecraft:white_shulker_box", false),
        ] {
            let state = place(game, block);
            let mut slots = vec![None; 27];
            slots[0] = Some(ItemStack::new("minecraft:stone", 3));
            game.world
                .block_entities
                .insert(key, BlockEntity::Chest(slots));
            game.dropped.clear();
            game.break_block(POS).unwrap();
            assert_eq!(at(game), if kept { state } else { air }, "{block}");
            assert_eq!(dropped(game, block), u32::from(!kept), "{block}: dropped");
            game.world.block_entities.remove(&key);
        }
        // Shears disarm a tripwire before vanilla removes it (TripWireBlock.playerWillDestroy);
        // the part's tripwire is neither disarmed nor removed.
        *game.held_mut() = Some(ItemStack::new("minecraft:shears", 1));
        let tripwire = place(game, "minecraft:tripwire");
        game.break_block(POS).unwrap();
        assert_eq!(at(game), tripwire);
        // Creative never harvests; the shears are the tool.
        assert_eq!(
            mod_platform::__private::part_tests::take_destroys(),
            vec![(false, 0), (false, 1)]
        );
        std::fs::remove_dir_all(scratch).unwrap();
    });
}

#[test]
#[ignore = "daily"]
fn the_parts_answer_decides_the_tool_wear_and_the_statistics() {
    // The fixture's parts: snow of two layers or more loses one and stays (`Some(false)`), glass
    // is removed by its part (`Some(true)`), dirt has none.
    mod_platform::__private::part_tests::destroyed_by_player_game_thread();
    with_game(|game| {
        let scratch = breaking_player(game);
        let registry = game.pack.assets.registry;
        let two_layers = registry
            .resolve(&horizon_world::chunk::BlockState {
                name: "minecraft:snow".into(),
                properties: [("layers".to_string(), "2".to_string())].into(),
            })
            .unwrap();
        // (block, tool, whether the break wears the tool and counts): vanilla's
        // ItemStack.mineBlock takes one durability for a block that is not instant to break
        // and counts the tool used, and Block.playerDestroy counts the block mined.
        for (block, tool, counts) in [
            ("minecraft:snow", "minecraft:iron_shovel", false),
            ("minecraft:glass", "minecraft:iron_pickaxe", true),
            ("minecraft:dirt", "minecraft:iron_shovel", true),
        ] {
            if block == "minecraft:snow" {
                game.world.set(POS[0], POS[1], POS[2], two_layers).unwrap();
            } else {
                place(game, block);
            }
            *game.held_mut() = Some(ItemStack::new(tool, 1));
            let used = game.stats.get("used", tool);
            let mined = game.stats.get("mined", block);
            game.break_block(POS).unwrap();
            let n = i32::from(counts);
            assert_eq!(
                game.held().unwrap().damage,
                u32::from(counts),
                "{block}: wear"
            );
            assert_eq!(game.stats.get("used", tool) - used, n, "{block}: used");
            assert_eq!(game.stats.get("mined", block) - mined, n, "{block}: mined");
        }
        // The shovel harvests snow, the pickaxe glass; dirt has no part to ask.
        assert_eq!(
            mod_platform::__private::part_tests::take_destroys(),
            vec![(true, 1), (true, 1)]
        );
        std::fs::remove_dir_all(scratch).unwrap();
    });
}

#[test]
#[ignore = "daily"]
fn the_mods_harvest_answer_is_read_once_after_the_removal() {
    // The fixture's part on stone records whether stone is still in the world when its harvest
    // is read, and leaves the removal to vanilla. The harvest is read once, after
    // playerWillDestroy and the removal; the part is told `will_harvest` from the tool alone
    // before it.
    mod_platform::__private::part_tests::destroyed_by_player_game_thread();
    with_game(|game| {
        let scratch = breaking_player(game);
        let air = game
            .pack
            .assets
            .registry
            .default_state("minecraft:air")
            .unwrap();
        // (tool, whether it harvests stone: vanilla's pickaxe requirement)
        for (tool, harvests) in [(None, false), (Some("minecraft:iron_pickaxe"), true)] {
            place(game, "minecraft:stone");
            *game.held_mut() = tool.map(|tool| ItemStack::new(tool, 1));
            game.dropped.clear();
            game.break_block(POS).unwrap();
            assert_eq!(at(game), air, "{tool:?}: vanilla's removal");
            assert_eq!(
                mod_platform::__private::part_tests::take_harvest_reads(),
                vec![false],
                "{tool:?}: one harvest read, after the removal"
            );
            assert_eq!(
                mod_platform::__private::part_tests::take_destroys(),
                vec![(harvests, u32::from(tool.is_some()))],
                "{tool:?}"
            );
            assert_eq!(
                dropped(game, "minecraft:cobblestone"),
                u32::from(harvests),
                "{tool:?}: drops"
            );
        }
        std::fs::remove_dir_all(scratch).unwrap();
    });
}

#[test]
#[ignore = "daily"]
fn a_pots_harvest_listener_runs_once_and_sees_it_cracked() {
    // playerWillDestroy cracks a pot broken with a pickaxe before the removal, and the harvest is
    // asked of that adjusted state, once.
    mod_platform::__private::part_tests::destroyed_by_player_game_thread();
    with_game(|game| {
        let scratch = breaking_player(game);
        place(game, "minecraft:decorated_pot");
        *game.held_mut() = Some(ItemStack::new("minecraft:iron_pickaxe", 1));
        game.break_block(POS).unwrap();
        assert_eq!(
            mod_platform::__private::part_tests::take_pot_harvest_reads(),
            vec![true],
            "one harvest read, of the cracked pot"
        );
        std::fs::remove_dir_all(scratch).unwrap();
    });
}

#[test]
#[ignore = "daily"]
fn denied_ice_does_not_melt_above_solid_ground() {
    mod_platform::__private::part_tests::destroyed_by_player_game_thread();
    with_game(|game| {
        let scratch = breaking_player(game);
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        game.world.set(POS[0], POS[1] - 1, POS[2], stone).unwrap();
        let ice = place(game, "minecraft:ice");
        game.break_block(POS).unwrap();
        assert_eq!(at(game), ice);
        assert_eq!(dropped(game, "minecraft:ice"), 0);
        std::fs::remove_dir_all(scratch).unwrap();
    });
}
