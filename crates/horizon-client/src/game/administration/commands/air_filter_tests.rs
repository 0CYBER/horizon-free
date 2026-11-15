//! `SetBlockCommand`'s and `FillCommand`'s keep filters are `LevelReader.isEmptyBlock`
//! (`getBlockState(pos).isAir()`), and `CloneCommands.FILTER_AIR` is `!state.isAir()`. cave_air
//! and void_air are air blocks, so all three filters treat them as air: a cell the filter takes
//! is replaced by keep and skipped by masked. `/fill ... destroy` reads the same fact, because
//! `Level.destroyBlock` returns false for air and `LevelChunk.setBlockState` does nothing when
//! the state is already there.
use crate::game::game_tests::tests::server_game;
use horizon_core::position::ChunkPos;

#[test]
#[ignore = "daily"]
fn cave_air_and_void_air_are_air_to_the_command_filters() {
    let mut resources = None;
    let mut game = server_game(&mut resources);
    game.build_registries();
    game.set_allow_commands(true);
    game.block_ticks
        .add_empty_chunk(ChunkPos::new(0, 0))
        .unwrap();
    assert!(game.run_command("forceload add 0 0"));
    assert!(game.run_command("tick freeze"));
    // A stone slab under the cells, so no cell of this test sits in an all-air chunk section.
    assert!(game.run_command("fill 0 198 0 15 199 15 stone"));

    let (air, cave_air, void_air, stone) = {
        let registry = &game.pack.assets.registry;
        let id = |name: &str| registry.default_state(name).unwrap();
        (
            id("minecraft:air"),
            id("minecraft:cave_air"),
            id("minecraft:void_air"),
            id("minecraft:stone"),
        )
    };
    let airs = [("air", air), ("cave_air", cave_air), ("void_air", void_air)];
    let mut failures = Vec::new();

    // /setblock ... keep: `isEmptyBlock(pos)` lets the write through, and still refuses a cell
    // that holds a block.
    for (i, (name, state)) in airs.into_iter().enumerate() {
        let (x, z) = (2 + i as i32, 2);
        game.world.set(x, 200, z, state).unwrap();
        if !game.run_command(&format!("setblock {x} 200 {z} minecraft:stone keep")) {
            failures.push(format!("{name}: /setblock keep refused"));
        }
        if game.world.block(x, 200, z) != stone {
            failures.push(format!("{name}: /setblock keep left the cell"));
        }
    }
    game.world.set(6, 200, 2, stone).unwrap();
    if game.run_command("setblock 6 200 2 minecraft:dirt keep") {
        failures.push("/setblock keep replaced a stone".into());
    }
    if game.world.block(6, 200, 2) != stone {
        failures.push("/setblock keep changed a stone".into());
    }

    // /fill ... keep: the same isEmptyBlock predicate.
    for (i, (_, state)) in airs.into_iter().enumerate() {
        game.world.set(2 + i as i32, 200, 4, state).unwrap();
    }
    if !game.run_command("fill 2 200 4 4 200 4 minecraft:stone keep") {
        failures.push("/fill keep refused".into());
    }
    for (i, (name, _)) in airs.into_iter().enumerate() {
        if game.world.block(2 + i as i32, 200, 4) != stone {
            failures.push(format!("{name}: /fill keep left the cell"));
        }
    }
    for i in 0..3 {
        game.world.set(2 + i, 200, 5, stone).unwrap();
    }
    if game.run_command("fill 2 200 5 4 200 5 minecraft:dirt keep") {
        failures.push("/fill keep replaced blocks".into());
    }
    for i in 0..3 {
        if game.world.block(2 + i, 200, 5) != stone {
            failures.push("/fill keep changed a block".into());
        }
    }

    // /clone ... masked: FILTER_AIR is `!isAir()`, so an all-air source copies nothing and the
    // command fails; the destination keeps what it had.
    for (i, (_, state)) in airs.into_iter().enumerate() {
        game.world.set(2 + i as i32, 200, 6, state).unwrap();
        game.world.set(2 + i as i32, 200, 8, stone).unwrap();
    }
    if game.run_command("clone 2 200 6 4 200 6 2 200 8 masked") {
        failures.push("an all-air masked clone succeeded".into());
    }
    for (i, (name, _)) in airs.into_iter().enumerate() {
        if game.world.block(2 + i as i32, 200, 8) != stone {
            failures.push(format!("{name}: /clone masked wrote over the destination"));
        }
    }
    // A source that holds blocks is still copied.
    for i in 0..3 {
        game.world.set(2 + i, 200, 12, stone).unwrap();
        game.world.set(2 + i, 200, 14, air).unwrap();
    }
    if !game.run_command("clone 2 200 12 4 200 12 2 200 14 masked") {
        failures.push("a stone masked clone failed".into());
    }
    for i in 0..3 {
        if game.world.block(2 + i, 200, 14) != stone {
            failures.push("a stone masked clone wrote nothing".into());
        }
    }

    // /fill ... destroy: `Level.destroyBlock` returns false for air, so `affected` stays false
    // and a fill that places nothing counts nothing: it fails.
    for (i, (name, state)) in airs.into_iter().enumerate() {
        let (x, z) = (2 + i as i32, 10);
        game.world.set(x, 200, z, state).unwrap();
        let command = format!("fill {x} 200 {z} {x} 200 {z} minecraft:{name} destroy");
        if game.run_command(&command) {
            failures.push(format!("{name}: /fill destroy counted a block"));
        }
        if game.world.block(x, 200, z) != state {
            failures.push(format!("{name}: /fill destroy changed the cell"));
        }
    }
    game.world.set(6, 200, 10, stone).unwrap();
    if !game.run_command("fill 6 200 10 6 200 10 minecraft:air destroy") {
        failures.push("a stone /fill destroy failed".into());
    }
    if game.world.block(6, 200, 10) != air {
        failures.push("a stone /fill destroy left the block".into());
    }

    assert!(failures.is_empty(), "{failures:#?}");
}
