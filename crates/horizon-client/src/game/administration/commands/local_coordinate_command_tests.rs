//! Tests verifying that registered local coordinate commands preserve exact saved positions.

use super::*;
use crate::game::native_entity_refs::tests::with_game;
use horizon_core::nbt::Tag;
use horizon_core::position::BlockPos;

fn pos_bits(tag: &Tag) -> [u64; 3] {
    let Some(Tag::List(pos)) = tag.get("Pos") else {
        panic!("actual saved Double position missing")
    };
    assert_eq!(pos.len(), 3);
    std::array::from_fn(|i| match &pos[i] {
        Tag::Double(bits) => *bits,
        other => panic!("actual Pos must be Double: {other:?}"),
    })
}

#[test]
#[ignore = "daily"]
fn h149_actual_registered_local_commands_preserve_exact_saved_positions() {
    with_game(|game| {
        game.build_registries();
        game.set_allow_commands(true);
        game.world.chunks.insert(
            (0, 0),
            std::sync::Arc::new(horizon_client::chunk::Chunk::empty(0, 0)),
        );
        game.block_ticks
            .add_empty_chunk(BlockPos::new(0, 0, 0).chunk())
            .unwrap();
        assert!(game.run_command("forceload add 0 0"));
        assert!(game.run_command("fill 0 199 0 15 199 15 stone"));
        assert!(game.run_command("scoreboard objectives add h149 dummy"));
        let clock = game.world.game_time;
        let mut failures = Vec::new();
        let cases = [
            (
                "south",
                "execute positioned 8.5 202.25 8.5 rotated 0.0 0.0 run tp 00000095-0000-0001-0000-000000000001 ^0.75 ^0.5 ^1.25",
                [
                    0x4022800000000000u64,
                    0x4069580000000000u64,
                    0x4023800000000000u64,
                ],
                false,
            ),
            (
                "west",
                "execute positioned 8.5 202.25 8.5 rotated 90.0 0.0 run tp 00000095-0000-0001-0000-000000000002 ^0.75 ^0.5 ^1.25",
                [
                    0x401d000000000000u64,
                    0x4069580000000000u64,
                    0x4022800000000000u64,
                ],
                false,
            ),
            (
                "north",
                "execute positioned 8.5 202.25 8.5 rotated 180.0 0.0 run tp 00000095-0000-0001-0000-000000000003 ^0.75 ^0.5 ^1.25",
                [
                    0x401f000000000000u64,
                    0x4069580000000000u64,
                    0x401d000000000000u64,
                ],
                false,
            ),
            (
                "east",
                "execute positioned 8.5 202.25 8.5 rotated 270.0 0.0 run tp 00000095-0000-0001-0000-000000000004 ^0.75 ^0.5 ^1.25",
                [
                    0x4023800000000000u64,
                    0x4069580000000000u64,
                    0x401f000000000000u64,
                ],
                false,
            ),
            (
                "oblique",
                "execute positioned 8.5 202.25 8.5 rotated 37.5 -22.25 run tp 00000095-0000-0001-0000-000000000005 ^0.1 ^0.1 ^0.1",
                [
                    0x402117934a6314b9u64,
                    0x40694c2c5ee98f58u64,
                    0x40213562572e889au64,
                ],
                false,
            ),
            (
                "positioned_oblique",
                "execute positioned 8.5 202.25 8.5 rotated 123.75 47.125 positioned ^0.1 ^0.1 ^0.1 run tp 00000095-0000-0001-0000-000000000006 ~ ~ ~",
                [
                    0x4020a7648c7aa86bu64,
                    0x406947d514129374u64,
                    0x4021025e8b180036u64,
                ],
                false,
            ),
            (
                "world",
                "execute positioned 8.5 202.25 8.5 rotated 37.5 -22.25 run tp 00000095-0000-0001-0000-000000000007 9.25 202.75 9.75",
                [
                    0x4022800000000000u64,
                    0x4069580000000000u64,
                    0x4023800000000000u64,
                ],
                true,
            ),
            (
                "relative_eyes",
                "execute positioned 8.5 202.25 8.5 rotated 37.5 -22.25 anchored eyes run tp 00000095-0000-0001-0000-000000000008 ~0.75 ~0.5 ~1.25",
                [
                    0x4022800000000000u64,
                    0x4069580000000000u64,
                    0x4023800000000000u64,
                ],
                true,
            ),
            (
                "feet",
                "execute as 00000095-0000-0001-0000-000000000009 at @s anchored feet rotated 37.5 -22.25 run tp @s ^ ^ ^",
                [
                    0x4012000000000000u64,
                    0x4069000000000000u64,
                    0x4012000000000000u64,
                ],
                true,
            ),
            (
                "eyes",
                "execute as 00000095-0000-0001-0000-00000000000a at @s anchored eyes rotated 37.5 -22.25 run tp @s ^ ^ ^",
                [
                    0x4012000000000000u64,
                    0x4069187ae1400000u64,
                    0x4012000000000000u64,
                ],
                true,
            ),
        ];
        for (i, (name, command, want, control)) in cases.into_iter().enumerate() {
            let number = i as i32 + 1;
            let actor = format!("00000095-0000-0001-0000-{number:012x}");
            let uuid = [149, 1, 0, number];
            assert!(game.run_command(&format!(r#"summon pig 4.5 200 4.5 {{UUID:[I;149,1,0,{number}],Tags:["h149"],Health:10f,NoAI:1b,NoGravity:1b,Silent:1b,PersistenceRequired:1b,Motion:[0d,0d,0d],Rotation:[11f,7f]}}"#)));
            let id = *game
                .persist
                .uuids
                .iter()
                .find(|(_, u)| **u == uuid)
                .unwrap()
                .0;
            assert!(game.mobs.iter().any(|m| m.id == id));
            assert!(game.run_command(&format!("execute if entity @e[type=pig,nbt={{UUID:[I;149,1,0,{number}],Health:10f,NoAI:1b,NoGravity:1b,Pos:[4.5d,200d,4.5d],Motion:[0d,0d,0d],Rotation:[11f,7f]}}]")));
            assert!(game.world.loaded(4, 4));
            let before = game.command_target_nbt(Target::Mob(id)).unwrap();
            assert_eq!(pos_bits(&before), [4.5f64, 200.0, 4.5].map(f64::to_bits));
            assert_eq!(before.get("UUID"), Some(&Tag::IntArray(uuid.to_vec())));
            assert_eq!(before.get("Health"), Some(&Tag::Float(10f32.to_bits())));
            // Discard retained raw input: only actual modeled position may prove this.
            game.persist.raw.remove(&id);
            assert!(game.run_command("scoreboard players set #r h149 91"));
            assert!(game.run_command("scoreboard players set #s h149 17"));
            assert!(
                game.run_command(&format!(
                    "execute store result score #r h149 store success score #s h149 run {command}"
                )),
                "actual command accepted: {name}"
            );
            assert_eq!(game.score.board.score("#r", "h149").unwrap().value, 1);
            assert_eq!(game.score.board.score("#s", "h149").unwrap().value, 1);
            let model = game.mobs.iter().find(|m| m.id == id).unwrap();
            let model_bits =
                [model.position.x, model.position.y, model.position.z].map(f64::to_bits);
            let saved = game.command_target_nbt(Target::Mob(id)).unwrap();
            let got = pos_bits(&saved);
            assert_eq!(
                model_bits, got,
                "actual model and typed canonical save agree: {name}"
            );
            assert_eq!(saved.get("UUID"), before.get("UUID"));
            for key in ["Health", "NoAI", "NoGravity", "Rotation", "Motion"] {
                assert_eq!(saved.get(key), before.get(key), "unchanged {key}: {name}");
            }
            assert!(
                game.run_command(&format!("execute as {actor} at @s if block ~ 199 ~ stone"))
            );
            assert!(game.world.loaded(
                f64::from_bits(got[0]).floor() as i32,
                f64::from_bits(got[2]).floor() as i32,
            ));
            if control {
                assert_eq!(
                    got, want,
                    "independent world/relative/anchor control {name}"
                );
            } else if got != want {
                failures.push((name, got, want));
            }
            assert_eq!(game.world.game_time, clock, "zero observation ticks");
        }
        assert!(
            failures.is_empty(),
            "actual registered local-coordinate bit mismatches: {failures:#?}"
        );
    });
}
