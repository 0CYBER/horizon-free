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

/// The saved player's stacks: `level.dat`'s read, before the world's tables are
/// staged, keeps every saved value; the world's load then builds them against its
/// item prototypes (`ItemStack.CODEC`, `applyPatch`): a value equal to the
/// prototype leaves the patch, another value and a removal of the prototype's own
/// component stay.
#[test]
#[ignore = "daily"]
fn saved_player_stacks_load_against_the_world_prototypes() {
    let mut resources = None;
    let mut game = server_game(&mut resources);
    let stack = |components: &str| {
        let tag = horizon_core::nbt::parse_snbt(&format!(
            r#"{{id:"minecraft:diamond_pickaxe",count:1,components:{components}}}"#
        ))
        .unwrap()
        .0;
        let saved = horizon_client::save::item_from_nbt(&tag).unwrap();
        (tag, saved)
    };
    let (plain, plain_saved) =
        stack(r#"{"minecraft:damage":0,"minecraft:max_damage":1561,"minecraft:repair_cost":0}"#);
    let (custom, _) =
        stack(r#"{"minecraft:max_damage":100,"!minecraft:damage":{},"!minecraft:custom_name":{}}"#);
    let slotted = |slot: i8, tag: &horizon_core::nbt::Tag| {
        let mut tag = tag.clone();
        if let horizon_core::nbt::Tag::Compound(map) = &mut tag {
            map.insert("Slot".into(), horizon_core::nbt::Tag::Byte(slot));
        }
        tag
    };
    let player = horizon_core::nbt::Tag::compound([
        (
            "Inventory",
            horizon_core::nbt::Tag::List(vec![slotted(0, &plain), slotted(1, &custom)]),
        ),
        (
            "EnderItems",
            horizon_core::nbt::Tag::List(vec![slotted(0, &plain)]),
        ),
    ]);
    let mut level = horizon_client::save::Level::default();
    level.read_player(&player);
    // Read before staging: nothing is left out.
    assert_eq!(level.ender_items[0].as_ref(), Some(&plain_saved));
    let (_, id, count, damage) = level.inventory.iter().find(|row| row.0 == 0).unwrap();
    let read = ItemStack {
        item: id.as_str().into(),
        count: *count,
        damage: *damage,
        extra: level.inventory_components.get(&0).cloned(),
    };
    assert_eq!(read, plain_saved);
    let inventory = game.load_saved_inventory(&level);
    let fresh = ItemStack::new("minecraft:diamond_pickaxe", 1);
    assert_eq!(inventory[0].as_ref(), Some(&fresh));
    assert_eq!(game.ender_items[0].as_ref(), Some(&fresh));
    assert_eq!(
        horizon_client::item_data::encode_components(inventory[1].as_ref().unwrap()),
        Some(
            horizon_core::nbt::parse_snbt(r#"{"minecraft:max_damage":100,"!minecraft:damage":{}}"#)
                .unwrap()
                .0
        )
    );
}

/// Stacks a command's NBT (`/setblock`, `/data merge block`) or a loaded item entity
/// or item frame brings in are built against the world's item prototypes: a value
/// equal to the prototype leaves the patch, an override stays.
#[test]
#[ignore = "daily"]
fn command_and_entity_stacks_are_built_against_the_world_prototypes() {
    let mut resources = None;
    let mut game = server_game(&mut resources);
    game.build_registries();
    game.set_allow_commands(true);
    game.block_ticks
        .add_empty_chunk(BlockPos::new(0, 0, 0).chunk())
        .unwrap();
    assert!(game.run_command("forceload add 0 0 15 15"));
    assert!(game.run_command("setblock 4 199 4 stone"));
    let snbt = |text: &str| horizon_core::nbt::parse_snbt(text).unwrap().0;
    let ore =
        |components: &str| format!(r#"id:"minecraft:iron_ore",count:3,components:{components}"#);
    let (redundant, custom) = (
        r#"{"minecraft:max_stack_size":64}"#,
        r#"{"minecraft:max_stack_size":16}"#,
    );
    let plain = ItemStack::new("minecraft:iron_ore", 3);
    let canon = |stack: Option<&ItemStack>| {
        stack
            .and_then(horizon_client::item_data::encode_components)
            .map(|c| horizon_client::item_data::canon_tag(&c))
    };
    let chest = |game: &crate::game::Game<'_>| {
        let horizon_client::containers::BlockEntity::Chest(slots) =
            game.world.block_entities.get(&(4, 200, 4)).unwrap()
        else {
            panic!("actual chest model missing")
        };
        slots.clone()
    };
    assert!(game.run_command(&format!(
        "setblock 4 200 4 chest{{Items:[{{Slot:0b,{}}},{{Slot:1b,{}}}]}}",
        ore(redundant),
        ore(custom)
    )));
    let slots = chest(&game);
    assert_eq!(slots[0].as_ref(), Some(&plain));
    assert_eq!(
        canon(slots[1].as_ref()),
        Some(horizon_client::item_data::canon_tag(&snbt(custom)))
    );
    assert!(game.run_command(&format!(
        "data merge block 4 200 4 {{Items:[{{Slot:2b,{}}}]}}",
        ore(redundant)
    )));
    assert_eq!(chest(&game)[2].as_ref(), Some(&plain));
    // A saved item entity and item frame.
    game.load_item(
        snbt(&format!(
            r#"{{id:"minecraft:item",Pos:[4.5d,201d,4.5d],Item:{{{}}}}}"#,
            ore(redundant)
        )),
        None,
    );
    assert_eq!(game.dropped.last().map(|e| &e.stack), Some(&plain));
    game.load_item_frame(
        snbt(&format!(
            r#"{{id:"minecraft:item_frame",block_pos:[I;4,201,5],Facing:2b,Item:{{{}}}}}"#,
            ore(redundant)
        )),
        None,
    );
    let frame = game.item_frames.last().expect("the frame loads");
    let item = frame.item.as_ref().expect("the frame keeps its item");
    assert!(item.same_item_same_components(&plain), "{item:?}");
}
