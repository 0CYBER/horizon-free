//! `execute facing` (`CommandSourceStack.facing`) wraps its yaw with
//! `Mth.wrapDegrees`, and `execute positioned over` uses `Heightmap.NOT_AIR`,
//! which skips cave air and void air as well as air.
use super::*;
use crate::game::game_tests::tests::server_game;

/// A pig summoned at a known place takes the rotation `rotate` gives it.
fn summoned_pig(game: &mut Game<'_>) -> u32 {
    assert!(game.run_command("summon minecraft:pig 0.5 100 0.5"));
    game.mobs.last().expect("the pig is in the level").id
}

#[test]
#[ignore = "daily"]
fn facing_wraps_and_uses_mth_atan2() {
    let mut resources = None;
    let mut game = server_game(&mut resources);
    game.build_registries();
    game.set_allow_commands(true);
    let pig = summoned_pig(&mut game);
    // `Entity.forceSetRotation` of a relative `~` gives back the source's
    // rotation, so the mob's yaw is the one `facing` computed. The bits are
    // `CommandSourceStack.facing`'s output on vanilla 26.3. A horizontal
    // target's pitch is not checked: `rotate ~ ~` adds +0.0 to it, which
    // vanilla does too and which turns -0.0 into +0.0.
    for (target, yaw, pitch) in [
        // atan2(-1, 1) - 90 = -135, which `Mth.wrapDegrees` keeps: 225 is the
        // same direction but not the value vanilla stores.
        ("1.5 64 -0.5", 0xc307_0000u32, None),
        // -225 wraps to 135.
        ("-0.5 64 -0.5", 0x4307_0000, None),
        // `Mth.atan2` is the table approximation, not `Math.atan2`: -45.000004.
        ("1.5 64 1.5", 0xc234_0001, None),
        // A 3-4-5 target: -36.86964.
        ("3.5 64 4.5", 0xc213_7a83, None),
        // Above the source: -26.565033, -77.39556.
        ("1.5 74 2.5", 0xc1d4_8530, Some(0xc29a_ca87u32)),
    ] {
        assert!(game.run_command("rotate @e[type=minecraft:pig,limit=1] 0 0"));
        assert!(
            game.run_command(&format!(
                "execute positioned 0.5 64 0.5 facing {target} run rotate @e[type=minecraft:pig,limit=1] ~ ~"
            )),
            "{target}"
        );
        let mob = game.mobs.iter().find(|m| m.id == pig).unwrap();
        assert_eq!(mob.yaw.to_bits(), yaw, "yaw for {target}");
        if let Some(pitch) = pitch {
            assert_eq!(mob.head_pitch.to_bits(), pitch, "pitch for {target}");
        }
    }
}

#[test]
#[ignore = "daily"]
fn over_world_surface_skips_cave_air_and_void_air() {
    let mut resources = None;
    let mut game = server_game(&mut resources);
    game.build_registries();
    game.set_allow_commands(true);
    let registry = game.pack.assets.registry;
    let stone = registry.default_state("minecraft:stone").unwrap();
    let cave_air = registry.default_state("minecraft:cave_air").unwrap();
    let void_air = registry.default_state("minecraft:void_air").unwrap();
    // Stone at y=60, then cave air, void air or nothing up to y=70: the
    // `world_surface` height is 61 in all three columns.
    for (x, above) in [(0, Some(cave_air)), (1, Some(void_air)), (2, None)] {
        game.world.set(x, 60, 0, stone).unwrap();
        if let Some(state) = above {
            for y in 61..=70 {
                game.world.set(x, y, 0, state).unwrap();
            }
        }
    }
    for x in [0, 1, 2] {
        assert!(
            game.run_command(&format!(
                "execute positioned {x}.5 100 0.5 positioned over world_surface run summon minecraft:pig ~ ~ ~"
            )),
            "column {x}"
        );
    }
    let heights: Vec<f64> = game.mobs.iter().map(|m| m.position.y).collect();
    assert_eq!(heights, [61.0, 61.0, 61.0]);
}
