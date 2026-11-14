//! `LivingEntity.aiStep` -> `EntityEquipment.tick` -> `CompassItem.inventoryTick` for mobs, and
//! the compass-only rule a player's `Inventory.tick` shares with them (#3221).
use super::Game;
use horizon_client::{items::ItemStack, motion::PlayerMotion};
use horizon_core::{geometry::Vec3, nbt::Tag, position::BlockPos};

const UUID_HIGH: i32 = 3221;

/// A server with chunk (0, 0) loaded and entity-ticking, a stone floor at y 63 and lodestones
/// at (5, 64, 5) and (7, 64, 7).
fn lodestone_world(resources: &mut Option<std::sync::Arc<crate::game::Resources>>) -> Game<'_> {
    let mut game = crate::game::game_tests::tests::server_game(resources);
    game.build_registries();
    game.set_allow_commands(true);
    game.level.health = 20.0;
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(14.5, 64.0, 14.5)).unwrap());
    game.block_ticks
        .add_empty_chunk(BlockPos::new(0, 0, 0).chunk())
        .unwrap();
    assert!(game.run_command("forceload add 0 0"));
    assert!(game.run_command("fill 0 63 0 15 63 15 stone"));
    assert!(game.run_command("setblock 5 64 5 lodestone"));
    assert!(game.run_command("setblock 7 64 7 lodestone"));
    game
}

fn tracker(dimension: &str, pos: [i32; 3]) -> String {
    let [x, y, z] = pos;
    format!(
        "minecraft:lodestone_tracker={{target:{{dimension:\"{dimension}\",pos:[I;{x},{y},{z}]}},tracked:true}}"
    )
}

/// The stack's `minecraft:lodestone_tracker`.
fn lodestone_tracker(stack: &ItemStack) -> &Tag {
    stack
        .data()
        .and_then(|d| d.other("minecraft:lodestone_tracker"))
        .expect("the stack keeps its lodestone_tracker")
}

/// `{target: {dimension, pos}}` of a tracker, as vanilla's codec writes it.
fn target(stack: &ItemStack) -> Option<(String, Vec<i32>)> {
    let target = lodestone_tracker(stack).get("target")?;
    let dimension =
        horizon_client::save::text(target.get("dimension")).expect("a target with a dimension");
    let Some(Tag::IntArray(pos)) = target.get("pos") else {
        panic!("a target without a pos");
    };
    Some((dimension.to_string(), pos.clone()))
}

/// `tracked` reads true when it is left out (`Codec.BOOL.optionalFieldOf("tracked", true)`).
fn tracked(stack: &ItemStack) -> bool {
    horizon_client::save::number(lodestone_tracker(stack).get("tracked")).is_none_or(|v| v != 0)
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES and scratch homes/cwd"]
fn a_mobs_compass_loses_a_broken_lodestone_and_keeps_the_others() {
    let mut resources = None;
    let mut game = lodestone_world(&mut resources);
    // A monster can't be summoned on peaceful.
    assert!(game.run_command("difficulty easy"));
    let overworld = "minecraft:overworld";
    let nether = "minecraft:the_nether";
    // n, held compass's target, worn (head) compass's target.
    let zombies = [
        (1, (overworld, [5, 64, 5]), Some((overworld, [5, 64, 5]))),
        (2, (overworld, [7, 64, 7]), None),
        (3, (nether, [5, 64, 5]), None),
    ];
    let mut ids = Vec::new();
    for (n, (dim, pos), head) in zombies {
        let x = f64::from(n * 2) + 0.5;
        assert!(game.run_command(&format!(
            "summon zombie {x} 64.0 2.5 {{UUID:[I;{UUID_HIGH},0,0,{n}],NoAI:1b,PersistenceRequired:1b,Silent:1b}}"
        )));
        let who = format!("00000c95-0000-0000-0000-{n:012x}");
        assert!(game.run_command(&format!(
            "item replace entity {who} weapon.mainhand with compass[{}] 1",
            tracker(dim, pos)
        )));
        if let Some((dim, pos)) = head {
            assert!(game.run_command(&format!(
                "item replace entity {who} armor.head with compass[{}] 1",
                tracker(dim, pos)
            )));
        }
        let id = game
            .mobs
            .iter()
            .find(|m| game.persist.uuid_of(m.id) == Some([UUID_HIGH, 0, 0, n]))
            .expect("the summoned zombie")
            .id;
        ids.push((n, id));
    }
    assert!(game.run_command("setblock 5 64 5 stone"));
    assert!(game.entity_ticking_area()(2, 2));
    game.tick_creature_entities().unwrap();
    let slot = |id: u32, slot: usize| -> ItemStack {
        let m = game.mobs.iter().find(|m| m.id == id).unwrap();
        m.extra.equipment[slot]
            .clone()
            .expect("the zombie keeps its compass")
    };
    for &(_, id) in &ids {
        assert!(game.creatures.ticked.contains(&id), "the zombie ticked");
    }
    let mainhand = horizon_items::equipment::MAINHAND;
    let head = horizon_items::equipment::HEAD;
    // `new LodestoneTracker(Optional.empty(), true)`: the target goes, tracking stays.
    for s in [slot(ids[0].1, mainhand), slot(ids[0].1, head)] {
        assert_eq!(&*s.item, "minecraft:compass");
        assert_eq!(target(&s), None);
        assert!(tracked(&s));
    }
    // The lodestone at (7, 64, 7) is still there.
    assert_eq!(
        target(&slot(ids[1].1, mainhand)),
        Some((overworld.to_owned(), vec![7, 64, 7]))
    );
    // `LodestoneTracker.tick` leaves a target in another dimension alone.
    assert_eq!(
        target(&slot(ids[2].1, mainhand)),
        Some((nether.to_owned(), vec![5, 64, 5]))
    );
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES and scratch homes/cwd"]
fn a_players_non_compass_keeps_its_lodestone_tracker() {
    let mut resources = None;
    let mut game = lodestone_world(&mut resources);
    let target_5 = tracker("minecraft:overworld", [5, 64, 5]);
    assert!(game.run_command(&format!(
        "item replace entity @s weapon.mainhand with stick[{target_5}] 1"
    )));
    assert!(game.run_command(&format!(
        "item replace entity @s weapon.offhand with compass[{target_5}] 1"
    )));
    assert!(game.run_command("setblock 5 64 5 stone"));
    game.tick_lodestone_compasses();
    let held = game.stack_in(0).unwrap();
    assert_eq!(&*held.item, "minecraft:stick");
    // Only `CompassItem` overrides `inventoryTick` with `LodestoneTracker.tick`.
    assert_eq!(
        target(held),
        Some(("minecraft:overworld".to_owned(), vec![5, 64, 5]))
    );
    let offhand = game.stack_in(crate::game::shield::OFFHAND_SLOT).unwrap();
    assert_eq!(target(offhand), None);
    assert!(tracked(offhand));
}
