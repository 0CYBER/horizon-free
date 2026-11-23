use super::*;
use crate::game::game_tests::tests::server_game;
use horizon_core::position::BlockPos;

#[test]
#[ignore = "daily"]
fn count_equality_uses_live_stack_bounds_and_command_flow() {
    let mut resources = None;
    let mut game = server_game(&mut resources);
    game.build_registries();
    game.set_allow_commands(true);
    game.block_ticks
        .add_empty_chunk(BlockPos::new(0, 0, 0).chunk())
        .unwrap();
    assert!(game.run_command("forceload add 0 0 15 15"));
    assert!(game.world.loaded(4, 4), "actual loaded world chunk");
    assert!(game.run_command("setblock 4 199 4 stone"));
    assert!(game.run_command("setblock 4 200 4 chest{Items:[{Slot:0b,id:\"minecraft:diamond\",count:1},{Slot:1b,id:\"minecraft:diamond\",count:3},{Slot:2b,id:\"minecraft:diamond\",count:4},{Slot:3b,id:\"minecraft:dirt\",count:3},{Slot:5b,id:\"minecraft:stone\",count:2,components:{\"minecraft:custom_data\":{h194:1}}}]}"));
    assert!(game.run_command("scoreboard objectives add h194 dummy"));
    let clock = game.world.game_time;
    let counts = |g: &Game<'_>| {
        let horizon_client::containers::BlockEntity::Chest(slots) =
            g.world.block_entities.get(&(4, 200, 4)).unwrap()
        else {
            panic!("actual chest model missing")
        };
        assert!(slots[4].is_none(), "actual empty slot");
        [
            slots[0].as_ref().unwrap().count,
            slots[1].as_ref().unwrap().count,
            slots[2].as_ref().unwrap().count,
        ]
    };
    assert_eq!(counts(&game), [1, 3, 4]);
    let mut failed = Vec::new();
    for (slot, count) in [1, 3, 4].into_iter().enumerate() {
        assert!(
            game.run_command(&format!(
                "execute if items block 4 200 4 container.{slot} diamond"
            )),
            "plain admitted command path"
        );
        let stack = ItemStack::new("minecraft:diamond", count);
        assert!(
            horizon_client::item_data::encode_components(&stack)
                .is_none_or(|t| t.get("minecraft:count").is_none()),
            "count is not a component patch"
        );
        for (term, expected) in [
            ("count=3", count == 3),
            ("count=4", count == 4),
            ("!count=3", count != 3),
            ("count={min:2}", count >= 2),
            ("count~3", count == 3),
            ("minecraft:count={min:2,max:3}", (2..=3).contains(&count)),
        ] {
            let predicate = ItemInput {
                id: "minecraft:diamond".into(),
                tag: false,
                any: false,
                components: Some(format!("[{term}]")),
            };
            let actual = game.stack_matches(&predicate, &stack);
            if actual != expected {
                failed.push(format!(
                    "direct count{count} {term}: {actual}, expected {expected}"
                ));
            }
            assert_eq!(stack.count, count, "read-only evaluator");
            assert!(game.run_command("scoreboard players set #r h194 91"));
            assert!(game.run_command("scoreboard players set #s h194 17"));
            let accepted = game.run_command(&format!("execute store result score #r h194 store success score #s h194 if items block 4 200 4 container.{slot} diamond[{term}]"));
            let score = |name| game.score.board.score(name, "h194").unwrap().value;
            let got = (accepted, score("#r"), score("#s"));
            let want = if expected {
                (true, count as i32, 1)
            } else {
                (false, 0, 0)
            };
            if got != want {
                failed.push(format!(
                    "command slot{slot} {term}: {got:?}, expected {want:?}"
                ));
            }
        }
    }
    assert!(game.run_command("execute unless items block 4 200 4 container.3 diamond[count=3]"));
    assert!(game.run_command("execute unless items block 4 200 4 container.4 diamond[count=3]"));
    assert!(
        game.run_command("execute if items block 4 200 4 container.5 stone[custom_data={h194:1}]")
    );
    assert_eq!(counts(&game), [1, 3, 4], "no inventory mutation");
    assert_eq!(game.world.game_time, clock, "zero actor ticks");
    assert!(
        failed.is_empty(),
        "actual count predicate mismatches: {failed:?}"
    );
}

/// A component value equal to the prototype leaves the built stack's patch
/// (`ItemInput.createItemStack`), yet `[id=value]` still matches it: the
/// predicate reads the effective value, and a removal takes precedence.
#[test]
#[ignore = "daily"]
fn component_equality_reads_the_effective_value() {
    let mut resources = None;
    let mut game = server_game(&mut resources);
    game.build_registries();
    let input = |id: &str, components: &str| ItemInput {
        id: id.into(),
        tag: false,
        any: false,
        components: Some(components.into()),
    };
    let built = game
        .command_item_stack(
            &input("minecraft:iron_pickaxe", "[max_damage=250,damage=0]"),
            1,
        )
        .unwrap_or_else(|_| panic!("built"));
    assert_eq!(horizon_client::item_data::encode_components(&built), None);
    // /give builds through the same ItemInput: the prototype's 1561 leaves the
    // patch, another maximum stays.
    game.player = Some(
        horizon_client::motion::PlayerMotion::new(game.motion, Vec3::new(8.5, 70.0, 8.5)).unwrap(),
    );
    game.world.chunks.insert(
        (0, 0),
        std::sync::Arc::new(horizon_client::chunk::Chunk::empty(0, 0)),
    );
    game.set_allow_commands(true);
    for (components, patch) in [
        ("[max_damage=1561]", None),
        ("[max_damage=100]", Some(r#"{"minecraft:max_damage":100}"#)),
    ] {
        game.inventory.iter_mut().for_each(|slot| *slot = None);
        assert!(game.run_command(&format!("give @s minecraft:diamond_pickaxe{components}")));
        let given = game.inventory.iter().flatten().next().expect("given");
        assert_eq!(&*given.item, "minecraft:diamond_pickaxe");
        assert_eq!(
            horizon_client::item_data::encode_components(given),
            patch.map(|p| horizon_core::nbt::parse_snbt(p).unwrap().0),
            "{components}"
        );
    }
    let removed = game
        .command_item_stack(&input("minecraft:iron_pickaxe", "[!damage]"), 1)
        .unwrap_or_else(|_| panic!("built"));
    for (stack, term, expected) in [
        (&built, "[max_damage=250]", true),
        (&built, "[damage=0]", true),
        (&built, "[max_damage=100]", false),
        (&built, "[max_damage]", true),
        (&removed, "[damage=0]", false),
        (&removed, "[!damage]", true),
    ] {
        assert_eq!(
            game.stack_matches(&input("minecraft:iron_pickaxe", term), stack),
            expected,
            "{term}"
        );
    }
}
