//! The effective damage-resistant component at its actual item-damage callers.
use super::*;
use crate::game::native_entity_refs::tests::{with_game, with_game_at};
use horizon_client::{chunk::Chunk, motion::PlayerMotion};
use horizon_core::{geometry::Vec3, position::BlockPos};

fn stack(input: &str) -> ItemStack {
    let (item, patch) = input
        .split_once('[')
        .map_or((input, None), |(item, patch)| {
            (item, Some(patch.strip_suffix(']').unwrap()))
        });
    let item = horizon_core::identifier::Identifier::parse(item).unwrap();
    let mut stack = ItemStack::new(&item.to_string(), 1);
    if let Some(patch) = patch {
        if patch == "!damage_resistant" {
            stack.data_mut().set_other(
                "!minecraft:damage_resistant",
                Some(horizon_core::nbt::Tag::Compound(Default::default())),
            );
        } else {
            let value = patch.strip_prefix("damage_resistant=").unwrap();
            let (value, _) = horizon_core::nbt::parse_snbt(value).unwrap();
            stack
                .data_mut()
                .set_other("minecraft:damage_resistant", Some(value));
        }
    }
    stack
}

fn prepare(game: &mut Game<'_>) {
    game.options.dedicated = Some(Vec::new());
    game.world = World::new(game.res.facts().air, 1);
    game.world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(8.5, 70.0, 8.5)).unwrap());
    game.update_chunk_tickets();
    assert!(game.entity_ticking_area()(4, 4));
}

fn drop_stack(game: &mut Game<'_>, input: &str, position: [f64; 3]) -> u32 {
    game.spawn_item(stack(input), position, [0.0; 3], 100);
    let item = game.dropped.last_mut().unwrap();
    item.no_gravity = true;
    let id = item.id;
    game.ticked_entities_through = game.next_entity;
    id
}

fn lava(game: &mut Game<'_>) {
    let lava = game
        .pack
        .assets
        .registry
        .default_state("minecraft:lava")
        .unwrap();
    for y in 64..=68 {
        game.world.set(4, y, 4, lava).unwrap();
    }
}

#[test]
#[ignore = "daily"]
fn effective_resistance_matches_the_six_stack_five_damage_table() {
    with_game(|game| {
        let damage = [
            "minecraft:in_fire",
            "minecraft:lava",
            "minecraft:cactus",
            "minecraft:explosion",
            "minecraft:lightning_bolt",
        ];
        let cases = [
            ("netherite_sword", [false, false, true, true, true]),
            (
                "netherite_sword[!damage_resistant]",
                [true, true, true, true, true],
            ),
            (
                r##"stone[damage_resistant={types:"#minecraft:panic_environmental_causes"}]"##,
                [false, false, false, true, false],
            ),
            ("nether_star", [true, true, true, false, true]),
            (
                "nether_star[!damage_resistant]",
                [true, true, true, true, true],
            ),
            (
                r##"diamond[damage_resistant={types:"#minecraft:is_explosion"}]"##,
                [true, true, true, false, true],
            ),
        ];
        for (input, expected) in cases {
            let stack = stack(input);
            for (damage, expected) in damage.into_iter().zip(expected) {
                assert_eq!(
                    game.stack_can_be_hurt_by(&stack, damage),
                    Some(expected),
                    "{input} against {damage}"
                );
            }
        }
        // Identifier::parse also gives an empty namespace the minecraft default.
        for types in [":lava", "#:is_fire"] {
            let input = format!("stone[damage_resistant={{types:\"{types}\"}}]");
            assert_eq!(
                game.stack_can_be_hurt_by(&stack(&input), "minecraft:lava"),
                Some(false)
            );
        }
    });
}

#[test]
#[ignore = "daily"]
fn removed_netherite_resistance_dies_on_the_second_lava_hurt() {
    with_game(|game| {
        prepare(game);
        lava(game);
        let id = drop_stack(game, "netherite_sword[!damage_resistant]", [4.5, 64.0, 4.5]);
        assert_eq!(game.dropped[0].health, 5);
        game.tick_items();
        assert_eq!(game.dropped[0].id, id);
        assert_eq!(game.dropped[0].health, 1);
        assert_eq!(game.dropped[0].age, 1);
        game.tick_items();
        assert!(game.dropped.iter().all(|item| item.id != id));
    });
}

#[test]
#[ignore = "daily"]
fn environmental_tag_stone_keeps_five_health_for_forty_lava_ticks() {
    with_game(|game| {
        prepare(game);
        lava(game);
        let id = drop_stack(
            game,
            r##"stone[damage_resistant={types:"#minecraft:panic_environmental_causes"}]"##,
            [4.5, 64.0, 4.5],
        );
        for _ in 0..40 {
            game.tick_items();
            assert_eq!(game.dropped[0].id, id);
            assert_eq!(game.dropped[0].health, 5);
        }
        assert_eq!(game.dropped[0].age, 40);
    });
}

#[test]
#[ignore = "daily"]
fn tnt_destroys_an_unprotected_star_and_spares_an_explosion_resistant_diamond() {
    with_game(|game| {
        prepare(game);
        let star = drop_stack(game, "nether_star[!damage_resistant]", [4.5, 64.0, 4.5]);
        let diamond = drop_stack(
            game,
            r##"diamond[damage_resistant={types:"#minecraft:is_explosion"}]"##,
            [4.5, 64.0, 4.5],
        );
        game.spawn_primed_tnt(BlockPos::new(4, 64, 4), false, None, None);
        game.tnts[0].fuse = 1;
        game.tnts[0].no_gravity = true;
        game.tnts[0].velocity = Vec3::ZERO;
        game.tick_one_tnt(0, &[]).unwrap();
        assert!(game.tnts.is_empty(), "the primed TNT actually detonated");
        assert!(game.dropped.iter().all(|item| item.id != star));
        let survivor = game.dropped.iter().find(|item| item.id == diamond).unwrap();
        assert_eq!(survivor.health, 5);
    });
}

#[test]
#[ignore = "daily"]
fn a_mod_items_default_cactus_resistance_keeps_five_health_for_ten_ticks() {
    // Give the owning resource load a private mod-item report and registry, as
    // native_components' changed-default fixture does. Shared resources are read-only.
    let root = tempfile::tempdir().unwrap();
    let source = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
    for entry in std::fs::read_dir(&source).unwrap() {
        let entry = entry.unwrap();
        if entry.file_name() != "items.json" && entry.file_name() != "command-registries.json" {
            #[cfg(unix)]
            std::os::unix::fs::symlink(entry.path(), root.path().join(entry.file_name())).unwrap();
            #[cfg(not(unix))]
            panic!("this resource-overlay fixture requires Unix symlinks");
        }
    }
    let mut defaults: serde_json::Value =
        serde_json::from_slice(&std::fs::read(source.join("items.json")).unwrap()).unwrap();
    defaults["fixture:cactus_item"] = defaults["minecraft:stone"].clone();
    defaults["fixture:cactus_item"]["minecraft:damage_resistant"] =
        serde_json::json!({"types": "minecraft:cactus"});
    std::fs::write(
        root.path().join("items.json"),
        serde_json::to_vec(&defaults).unwrap(),
    )
    .unwrap();
    let mut registries: serde_json::Value =
        serde_json::from_slice(&std::fs::read(source.join("command-registries.json")).unwrap())
            .unwrap();
    registries["items"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!("fixture:cactus_item"));
    std::fs::write(
        root.path().join("command-registries.json"),
        serde_json::to_vec(&registries).unwrap(),
    )
    .unwrap();
    with_game_at(root.path().to_path_buf(), |game| {
        prepare(game);
        let cactus = game
            .pack
            .assets
            .registry
            .default_state("minecraft:cactus")
            .unwrap();
        game.world.set(4, 64, 4, cactus).unwrap();
        drop_stack(game, "minecraft:stone", [5.08, 64.0, 4.5]);
        game.tick_items();
        assert_eq!(game.dropped[0].health, 4, "the tick really touches cactus");
        game.dropped.clear();
        let id = drop_stack(game, "fixture:cactus_item", [5.08, 64.0, 4.5]);
        assert!(
            game.dropped[0].stack.data().is_none(),
            "resistance comes from defaults"
        );
        for _ in 0..10 {
            // Keep the item touching the cactus block while outside its inset collision box.
            game.dropped[0].position = Vec3::new(5.08, 64.0, 4.5);
            game.dropped[0].velocity = Vec3::ZERO;
            game.tick_items();
            assert_eq!(game.dropped[0].id, id);
            assert_eq!(game.dropped[0].health, 5);
        }
        assert_eq!(game.dropped[0].age, 10);
    });
}

#[test]
#[ignore = "daily"]
fn damage_command_uses_effective_item_resistance() {
    with_game(|game| {
        prepare(game);
        game.build_registries();
        for (input, damage, expected_health) in [
            ("netherite_sword", "lava", 5),
            ("netherite_sword[!damage_resistant]", "lava", 1),
            (
                r##"stone[damage_resistant={types:"#minecraft:panic_environmental_causes"}]"##,
                "lava",
                5,
            ),
            ("nether_star", "explosion", 5),
            ("nether_star[!damage_resistant]", "explosion", 1),
            (
                r##"diamond[damage_resistant={types:"#minecraft:is_explosion"}]"##,
                "explosion",
                5,
            ),
        ] {
            game.dropped.clear();
            drop_stack(game, input, [4.5, 64.0, 4.5]);
            let command = format!("damage @e[type=minecraft:item,limit=1] 4 minecraft:{damage}");
            let success = game.as_console(|game| game.run_command(&command));
            assert_eq!(success, expected_health == 1, "{input}: {command}");
            assert_eq!(
                game.dropped[0].health, expected_health,
                "{input}: {command}"
            );
        }
    });
}
