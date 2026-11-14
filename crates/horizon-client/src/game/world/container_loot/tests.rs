//! Production gameplay triggers, with one isolated game process per group of scenarios.
use super::*;
use crate::game::minecarts::{CartKind, Minecart};
use horizon_client::save::PendingContainerLoot;
use horizon_core::{geometry::Vec3, nbt::Tag, position::BlockPos};
use horizon_world::loot::{LootData, PackOverlay};
use std::sync::{Arc, Mutex};

const POS: (i32, i32, i32) = (8, 64, 8);
const TABLE: &str = "test:first_access";

/// Runs the group in one child process, so the game resources load once per group and the
/// memory returns to the system when the child exits. Returns true in the parent, once the
/// child passed; false inside the child, which then runs the group's scenarios itself.
fn in_child(group: &str) -> bool {
    if std::env::var("HORIZON_CONTAINER_LOOT_TEST").as_deref() == Ok(group) {
        std::fs::write(
            std::env::var_os("HORIZON_CONTAINER_LOOT_MARKER").unwrap(),
            group,
        )
        .unwrap();
        return false;
    }
    // Game fixtures use process-global native registration. Also bound fixture
    // memory while retaining the ordinary test runner's default parallelism.
    static FIXTURES: Mutex<()> = Mutex::new(());
    let _lock = FIXTURES.lock().unwrap_or_else(|e| e.into_inner());
    let root = std::env::temp_dir().join(format!("container-loot-{}-{group}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    let result = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            &format!("game::container_loot::tests::{group}"),
            "--include-ignored",
            "--nocapture",
        ])
        .env("HORIZON_CONTAINER_LOOT_TEST", group)
        .env("HORIZON_CONTAINER_LOOT_MARKER", root.join("executed"))
        .env("HORIZON_HOME", root.join("home"))
        .env("HORIZON_LAUNCHER_HOME", root.join("launcher"))
        .env("XDG_DATA_HOME", root.join("data"))
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("HORIZON_NO_AUDIO", "1")
        .current_dir(&root)
        .output()
        .unwrap();
    let executed = root.join("executed").exists();
    std::fs::remove_dir_all(root).unwrap();
    assert!(
        result.status.success() && executed,
        "{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    true
}

/// Runs each scenario in turn, naming it first so a failure shows which one it was.
fn run_group(scenarios: &[(&str, fn())]) {
    for (name, scenario) in scenarios {
        println!("scenario {name}");
        scenario();
    }
}

fn with_game(test: impl FnOnce(&mut Game<'_>)) {
    crate::game::native_guest_turn_tests::with_guests(|game| {
        game.build_registries();
        game.set_allow_commands(true);
        game.world
            .chunks
            .insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
        let table = |item: &str, count: i32, bonus: i32| serde_json::json!({"pools":[{"rolls":1,"bonus_rolls":bonus,"entries":[{"type":"minecraft:item","name":item,"modifier":{"type":"minecraft:set_count","count":count}}]}]});
        let files = [
            ("loot_table/test:sequence_position".into(), vec![serde_json::to_vec(&serde_json::json!({"random_sequence":"test:seq","pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:diamond"}]}]})).unwrap()]),
            ("loot_table/test:first_access".into(), vec![serde_json::to_vec(&table("minecraft:diamond",1,1)).unwrap()]),
            ("loot_table/test:left".into(), vec![serde_json::to_vec(&table("minecraft:diamond",1,0)).unwrap()]),
            ("loot_table/test:right".into(), vec![serde_json::to_vec(&table("minecraft:emerald",2,0)).unwrap()]),
            ("loot_table/test:entity".into(), vec![serde_json::to_vec(&serde_json::json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:emerald","condition":{"type":"minecraft:entity_properties","entity":"this","predicate":{"minecraft:entity_type":"minecraft:player"}}}]}]})).unwrap()]),
        ];
        let loot = LootData::load(
            &game.options.resources,
            &PackOverlay {
                files: &files,
                ..PackOverlay::NONE
            },
            game.res.item_defaults(),
        )
        .unwrap();
        assert!(loot.has_table(TABLE));
        game.server_loot.set(Some(Arc::new(loot)));
        test(game);
    });
}

fn place(game: &mut Game<'_>, pos: (i32, i32, i32), table: &str, seed: i64) {
    assert!(game.run_command(&format!(
        "setblock {} {} {} minecraft:chest{{LootTable:\"{table}\",LootTableSeed:{seed}L}}",
        pos.0, pos.1, pos.2
    )));
    pending_seed(game, pos, &[seed]);
}
fn pending(game: &Game<'_>, pos: (i32, i32, i32)) {
    pending_seed(game, pos, &[37, 91]);
}
fn pending_seed(game: &Game<'_>, pos: (i32, i32, i32), seeds: &[i64]) {
    let pending = game.world.chunks[&(pos.0 >> 4, pos.2 >> 4)]
        .saved
        .as_ref()
        .unwrap()
        .pending_loot
        .get(&pos)
        .unwrap();
    assert!(
        matches!(pending.seed, Some(Tag::Long(seed)) if seeds.contains(&seed)),
        "{:?}",
        pending.seed
    );
    assert!(
        game.world
            .block_entities
            .get(&pos)
            .unwrap()
            .slots()
            .iter()
            .all(Option::is_none)
    );
}
fn consumed(game: &Game<'_>, pos: (i32, i32, i32)) {
    assert!(
        !game.world.chunks[&(pos.0 >> 4, pos.2 >> 4)]
            .saved
            .as_ref()
            .unwrap()
            .pending_loot
            .contains_key(&pos)
    );
}
fn count(game: &Game<'_>, pos: (i32, i32, i32), item: &str) -> u32 {
    game.world.block_entities.get(&pos).map_or(0, |e| {
        e.slots()
            .iter()
            .flatten()
            .filter(|s| &*s.item == item)
            .map(|s| s.count)
            .sum()
    })
}
fn dropped(game: &Game<'_>, item: &str) -> u32 {
    game.dropped
        .iter()
        .filter(|d| &*d.stack.item == item)
        .map(|d| d.stack.count)
        .sum()
}
fn open(game: &mut Game<'_>, pos: (i32, i32, i32), luck: f64) {
    game.as_guest(12, |g| {
        let p = g.player.as_mut().unwrap();
        let key = p.attributes.key("minecraft:luck").unwrap();
        let h = p.attributes.get_instance(Some(key)).unwrap().unwrap();
        p.attributes
            .edit(&h, |a, o| a.set_base_value(luck, o))
            .unwrap();
        assert!(
            g.native_acting_player().is_some(),
            "the opener must be admitted"
        );
        g.open_block_effect_menu([pos.0, pos.1, pos.2]);
    })
    .unwrap();
}

fn player_open_uses_admitted_guest_luck_and_entity() {
    with_game(|g| {
        for (pos, luck, want) in [(POS, 2.0, 3), ((10, 64, 8), 0.0, 1)] {
            place(g, pos, TABLE, 37);
            open(g, pos, luck);
            consumed(g, pos);
            assert_eq!(count(g, pos, "minecraft:diamond"), want);
            open(g, pos, luck);
            assert_eq!(count(g, pos, "minecraft:diamond"), want);
        }
        let pos = (12, 64, 8);
        place(g, pos, "test:entity", 37);
        open(g, pos, 0.0);
        consumed(g, pos);
        assert_eq!(count(g, pos, "minecraft:emerald"), 1);
    });
}

fn hopper_pull_unpacks_source_once() {
    with_game(|g| {
        assert!(g.run_command("setblock 8 63 8 minecraft:hopper"));
        place(g, POS, TABLE, 37);
        g.hopper_try_move_items((8, 63, 8));
        consumed(g, POS);
        assert_eq!(count(g, (8, 63, 8), "minecraft:diamond"), 1);
        g.hoppers.insert(
            (8, 63, 8),
            crate::game::machines::HopperState::with_cooldown(0),
        );
        g.hopper_try_move_items((8, 63, 8));
        assert_eq!(
            count(g, (8, 63, 8), "minecraft:diamond") + count(g, POS, "minecraft:diamond"),
            1
        );
    });
}

fn hopper_push_unpacks_destination_once() {
    with_game(|g| {
        assert!(g.run_command("setblock 7 64 8 minecraft:hopper[facing=east]{Items:[{Slot:0b,id:\"minecraft:cobblestone\",count:1}]}"));
        place(g, POS, TABLE, 37);
        g.hopper_try_move_items((7, 64, 8));
        consumed(g, POS);
        assert_eq!(count(g, POS, "minecraft:diamond"), 1);
        assert_eq!(count(g, POS, "minecraft:cobblestone"), 1);
        assert!(g.run_command("item replace block 7 64 8 container.0 with minecraft:cobblestone"));
        g.hoppers.insert(
            (7, 64, 8),
            crate::game::machines::HopperState::with_cooldown(0),
        );
        g.hopper_try_move_items((7, 64, 8));
        assert_eq!(count(g, POS, "minecraft:diamond"), 1);
    });
}

fn comparator_first_read_unpacks_and_measures() {
    with_game(|g| {
        place(g, POS, TABLE, 37);
        assert!(g.run_command("setblock 9 64 8 minecraft:comparator[facing=west]"));
        let tick = |g: &mut Game<'_>| {
            g.with_level(|b, l, t| b.scheduled_block_tick(l, t, BlockPos::new(9, 64, 8)));
        };
        tick(g);
        consumed(g, POS);
        assert_eq!(count(g, POS, "minecraft:diamond"), 1);
        assert!(
            g.block_ticks
                .comparator_outputs
                .get(&BlockPos::new(9, 64, 8))
                .copied()
                .unwrap_or(0)
                > 0
        );
        tick(g);
        assert_eq!(count(g, POS, "minecraft:diamond"), 1);
    });
}

fn block_break_unpacks_before_contents_drop() {
    with_game(|g| {
        place(g, POS, TABLE, 37);
        g.as_guest(12, |g| {
            g.creative = false;
            g.break_block([8, 64, 8]).unwrap();
        })
        .unwrap();
        consumed(g, POS);
        assert_eq!(dropped(g, "minecraft:diamond"), 1);
        g.as_guest(12, |g| g.break_block([8, 64, 8]).unwrap())
            .unwrap();
        assert_eq!(dropped(g, "minecraft:diamond"), 1);
    });
}

fn explosion_unpacks_before_contents_drop() {
    with_game(|g| {
        place(g, POS, TABLE, 37);
        let explode = |g: &mut Game<'_>| {
            g.server_explode(
                [8.5, 64.5, 8.5],
                4.0,
                crate::game::explosion::BlockInteraction::Destroy,
                false,
            )
            .unwrap()
        };
        explode(g);
        consumed(g, POS);
        assert_eq!(dropped(g, "minecraft:diamond"), 1);
        g.dropped.clear();
        explode(g);
        assert_eq!(dropped(g, "minecraft:diamond"), 0);
    });
}

fn item_replace_unpacks_before_writing() {
    with_game(|g| {
        place(g, POS, TABLE, 37);
        assert!(g.run_command("item replace block 8 64 8 container.0 with minecraft:stone"));
        consumed(g, POS);
        assert_eq!(count(g, POS, "minecraft:diamond"), 1);
        assert_eq!(
            g.world.block_entities.get(&POS).unwrap().slots()[0]
                .as_ref()
                .unwrap()
                .item
                .as_ref(),
            "minecraft:stone"
        );
        assert!(g.run_command("item replace block 8 64 8 container.0 with minecraft:stone"));
        assert_eq!(count(g, POS, "minecraft:diamond"), 1);
    });
}

fn double_chest_rolls_each_half_once() {
    with_game(|g| {
        assert!(g.run_command("setblock 8 64 8 minecraft:chest[facing=north,type=left]{LootTable:\"test:left\",LootTableSeed:37L}"));
        assert!(g.run_command("setblock 9 64 8 minecraft:chest[facing=north,type=right]{LootTable:\"test:right\",LootTableSeed:91L}"));
        pending(g, POS);
        pending(g, (9, 64, 8));
        open(g, POS, 2.0);
        consumed(g, POS);
        consumed(g, (9, 64, 8));
        assert_eq!(count(g, POS, "minecraft:diamond"), 1);
        assert_eq!(count(g, (9, 64, 8), "minecraft:emerald"), 2);
        open(g, POS, 2.0);
        assert_eq!(count(g, POS, "minecraft:diamond"), 1);
        assert_eq!(count(g, (9, 64, 8), "minecraft:emerald"), 2);
    });
}

fn cart(g: &mut Game<'_>) -> u32 {
    g.next_entity += 1;
    let id = g.next_entity;
    let mut c = Minecart::new(id, CartKind::Chest, Vec3::new(8.5, 64.0, 8.5));
    c.loot = Some(PendingContainerLoot {
        table: Tag::String(TABLE.into()),
        seed: Some(Tag::Long(37)),
    });
    assert!(c.items.iter().all(Option::is_none));
    g.carts.push(c);
    id
}
fn chest_minecart_open_uses_opener_once() {
    with_game(|g| {
        let id = cart(g);
        g.as_guest(12, |g| {
            let p = g.player.as_mut().unwrap();
            let k = p.attributes.key("minecraft:luck").unwrap();
            let h = p.attributes.get_instance(Some(k)).unwrap().unwrap();
            p.attributes
                .edit(&h, |a, o| a.set_base_value(2.0, o))
                .unwrap();
            let i = g.carts.iter().position(|c| c.id == id).unwrap();
            g.open_cart_container(i);
            g.open_cart_container(i);
        })
        .unwrap();
        let c = g.carts.iter().find(|c| c.id == id).unwrap();
        assert!(c.loot.is_none());
        assert_eq!(
            c.items
                .iter()
                .flatten()
                .filter(|s| &*s.item == "minecraft:diamond")
                .map(|s| s.count)
                .sum::<u32>(),
            3
        );
    });
}
fn chest_minecart_break_unpacks_before_drop() {
    with_game(|g| {
        let id = cart(g);
        for _ in 0..2 {
            if let Some(i) = g.carts.iter().position(|c| c.id == id) {
                g.hurt_cart(i, 100.0, false, false);
            }
        }
        assert!(!g.carts.iter().any(|c| c.id == id));
        assert_eq!(dropped(g, "minecraft:diamond"), 1);
    });
}

fn hopper_minecart_pull_unpacks_source_once() {
    with_game(|g| {
        place(g, POS, TABLE, 37);
        let mut hopper = Minecart::new(900, CartKind::Hopper, Vec3::new(8.5, 62.5, 8.5));
        g.hopper_cart_suck(&mut hopper);
        consumed(g, POS);
        assert_eq!(
            hopper
                .items
                .iter()
                .flatten()
                .filter(|s| &*s.item == "minecraft:diamond")
                .map(|s| s.count)
                .sum::<u32>(),
            1
        );
        g.hopper_cart_suck(&mut hopper);
        assert_eq!(
            hopper
                .items
                .iter()
                .flatten()
                .filter(|s| &*s.item == "minecraft:diamond")
                .map(|s| s.count)
                .sum::<u32>(),
            1
        );
    });
}

fn dropper_push_unpacks_destination_once() {
    with_game(|g| {
        assert!(g.run_command("setblock 7 64 8 minecraft:dropper[facing=east]{Items:[{Slot:0b,id:\"minecraft:cobblestone\",count:1}]}"));
        place(g, POS, TABLE, 37);
        let state = g.world.block(7, 64, 8);
        g.dispense_from(BlockPos::new(7, 64, 8), state).unwrap();
        consumed(g, POS);
        assert_eq!(count(g, POS, "minecraft:diamond"), 1);
        assert_eq!(count(g, POS, "minecraft:cobblestone"), 1);
        assert!(g.run_command("item replace block 7 64 8 container.0 with minecraft:cobblestone"));
        g.dispense_from(BlockPos::new(7, 64, 8), state).unwrap();
        assert_eq!(count(g, POS, "minecraft:diamond"), 1);
    });
}

fn dispenser_selection_unpacks_source_once() {
    with_game(|g| {
        assert!(g.run_command(
            "setblock 8 64 8 minecraft:dispenser{LootTable:\"test:first_access\",LootTableSeed:37L}"
        ));
        pending(g, POS);
        let state = g.world.block(8, 64, 8);
        g.dispense_from(BlockPos::new(8, 64, 8), state).unwrap();
        consumed(g, POS);
        assert_eq!(dropped(g, "minecraft:diamond"), 1);
        g.dispense_from(BlockPos::new(8, 64, 8), state).unwrap();
        assert_eq!(dropped(g, "minecraft:diamond"), 1);
    });
}

fn execute_if_items_unpacks_before_reading() {
    with_game(|g| {
        place(g, POS, TABLE, 37);
        for _ in 0..2 {
            assert!(g.run_command(
                "execute if items block 8 64 8 container.* minecraft:diamond run say found"
            ));
            consumed(g, POS);
            assert_eq!(count(g, POS, "minecraft:diamond"), 1);
        }
    });
}

fn loot_insert_unpacks_before_inserting() {
    with_game(|g| {
        place(g, POS, TABLE, 37);
        for _ in 0..2 {
            assert!(g.run_command("loot insert 8 64 8 loot {pools:[{rolls:1,entries:[{type:\"minecraft:item\",name:\"minecraft:stone\"}]}]}"));
            consumed(g, POS);
            assert_eq!(count(g, POS, "minecraft:diamond"), 1);
        }
        assert_eq!(count(g, POS, "minecraft:stone"), 2);
    });
}

fn detector_rail_comparator_unpacks_cart_once() {
    with_game(|g| {
        assert!(g.run_command("fill 7 63 7 10 63 9 minecraft:stone"));
        let id = cart(g);
        assert!(g.run_command("setblock 8 64 8 minecraft:detector_rail[powered=true]"));
        assert!(g.carts.iter().find(|c| c.id == id).unwrap().loot.is_some());
        assert!(g.run_command("setblock 9 64 8 minecraft:comparator[facing=west]"));
        for _ in 0..2 {
            g.with_level(|b, l, t| b.scheduled_block_tick(l, t, BlockPos::new(9, 64, 8)));
            let c = g.carts.iter().find(|c| c.id == id).unwrap();
            assert!(c.loot.is_none());
            assert_eq!(
                c.items
                    .iter()
                    .flatten()
                    .filter(|s| &*s.item == "minecraft:diamond")
                    .map(|s| s.count)
                    .sum::<u32>(),
                1
            );
            assert!(
                g.block_ticks
                    .comparator_outputs
                    .get(&BlockPos::new(9, 64, 8))
                    .copied()
                    .unwrap_or(0)
                    > 0
            );
        }
    });
}

fn double(game: &mut Game<'_>) {
    assert!(game.run_command("setblock 8 64 8 minecraft:chest[facing=north,type=left]{LootTable:\"test:left\",LootTableSeed:37L}"));
    assert!(game.run_command("setblock 9 64 8 minecraft:chest[facing=north,type=right]{LootTable:\"test:right\",LootTableSeed:91L}"));
    pending(game, POS);
    pending(game, (9, 64, 8));
}
fn double_chest_hopper_pull_reads_only_needed_half() {
    with_game(|g| {
        assert!(g.run_command("setblock 8 63 8 minecraft:hopper"));
        double(g);
        // Vanilla's compound visits RIGHT (x=9) before LEFT (x=8), even
        // when the hopper is under the left block. RIGHT holds two emeralds.
        g.hopper_try_move_items((8, 63, 8));
        consumed(g, (9, 64, 8));
        pending(g, POS);
        assert_eq!(count(g, (8, 63, 8), "minecraft:emerald"), 1);
        g.hoppers.insert(
            (8, 63, 8),
            crate::game::machines::HopperState::with_cooldown(0),
        );
        g.hopper_try_move_items((8, 63, 8));
        pending(g, POS);
        assert_eq!(count(g, (8, 63, 8), "minecraft:emerald"), 2);
        for _ in 0..2 {
            g.hoppers.insert(
                (8, 63, 8),
                crate::game::machines::HopperState::with_cooldown(0),
            );
            g.hopper_try_move_items((8, 63, 8));
        }
        consumed(g, POS);
        assert_eq!(count(g, (8, 63, 8), "minecraft:diamond"), 1);
        assert_eq!(count(g, (8, 63, 8), "minecraft:emerald"), 2);
    });
}

fn double_chest_hopper_push_keeps_other_half_pending() {
    with_game(|g| {
        assert!(g.run_command("setblock 7 64 8 minecraft:hopper[facing=east]{Items:[{Slot:0b,id:\"minecraft:cobblestone\",count:1}]}"));
        double(g);
        g.hopper_try_move_items((7, 64, 8));
        consumed(g, (9, 64, 8));
        pending(g, POS);
        assert_eq!(count(g, (9, 64, 8), "minecraft:emerald"), 2);
        assert_eq!(count(g, (9, 64, 8), "minecraft:cobblestone"), 1);
        assert!(g.run_command("item replace block 7 64 8 container.0 with minecraft:cobblestone"));
        g.hoppers.insert(
            (7, 64, 8),
            crate::game::machines::HopperState::with_cooldown(0),
        );
        g.hopper_try_move_items((7, 64, 8));
        pending(g, POS);
        assert_eq!(count(g, (9, 64, 8), "minecraft:emerald"), 2);
    });
}

fn pending_bytes(game: &Game<'_>, pos: (i32, i32, i32)) -> Vec<u8> {
    pending(game, pos);
    let pending = game.world.block_entity_extras(pos).pending_loot.unwrap();
    let mut fields = horizon_core::nbt::Compound::new();
    pending.write_tags(&mut fields);
    horizon_core::nbt::write_named(
        &horizon_core::nbt::NamedTag {
            name: "".into(),
            tag: Tag::Compound(fields),
        },
        horizon_core::nbt::Limits::default(),
    )
    .unwrap()
}
fn clone_copy_preserves_pending_loot_independently() {
    with_game(|g| {
        place(g, POS, TABLE, 37);
        let original = pending_bytes(g, POS);
        // An overwritten destination's different table must not survive.
        place(g, (12, 64, 8), "test:right", 91);
        assert!(g.run_command("clone 8 64 8 8 64 8 12 64 8"));
        assert_eq!(pending_bytes(g, POS), original);
        assert_eq!(pending_bytes(g, (12, 64, 8)), original);
        open(g, (12, 64, 8), 0.0);
        consumed(g, (12, 64, 8));
        assert_eq!(count(g, (12, 64, 8), "minecraft:diamond"), 1);
        assert_eq!(count(g, (12, 64, 8), "minecraft:emerald"), 0);
        assert_eq!(pending_bytes(g, POS), original);
        open(g, (12, 64, 8), 0.0);
        assert_eq!(count(g, (12, 64, 8), "minecraft:diamond"), 1);
    });
}
fn clone_move_transfers_pending_loot() {
    with_game(|g| {
        place(g, POS, TABLE, 37);
        let original = pending_bytes(g, POS);
        place(g, (12, 64, 8), "test:right", 91);
        assert!(g.run_command("clone 8 64 8 8 64 8 12 64 8 replace move"));
        consumed(g, POS);
        assert!(g.world.block_entities.get(&POS).is_none());
        assert_eq!(pending_bytes(g, (12, 64, 8)), original);
        assert_eq!(dropped(g, "minecraft:diamond"), 0);
        assert_eq!(dropped(g, "minecraft:emerald"), 0);
        open(g, (12, 64, 8), 0.0);
        consumed(g, (12, 64, 8));
        assert_eq!(count(g, (12, 64, 8), "minecraft:diamond"), 1);
    });
}
fn structure_capture_disk_round_trip_keeps_pending_loot() {
    with_game(|g| {
        crate::game::growth::install_live_features(
            &g.res,
            &g.pack,
            &g.options,
            &Default::default(),
        );
        place(g, POS, TABLE, 37);
        let original = pending_bytes(g, POS);
        // fresh() supplies a unique scratch working directory for this world.
        g.world_root = std::env::current_dir().unwrap().join("world");
        assert!(g.save_structure_area(
            "test:pending",
            [8, 64, 8],
            [1, 1, 1],
            true,
            "fixture",
            true,
            false
        ));
        let captured = g.template_entry("test:pending").unwrap();
        let Tag::List(blocks) = captured.tag.get("blocks").unwrap() else {
            panic!("captured blocks")
        };
        let nbt = blocks[0].get("nbt").expect("captured block entity");
        assert_eq!(nbt.get("LootTable"), Some(&Tag::String(TABLE.into())));
        assert_eq!(nbt.get("LootTableSeed"), Some(&Tag::Long(37)));
        g.structure_templates.clear();
        let entry = g
            .template_entry("test:pending")
            .expect("read saved structure from disk");
        assert_eq!(entry.tag, captured.tag);
        g.world
            .chunks
            .insert((1, 0), Arc::new(horizon_client::chunk::Chunk::empty(1, 0)));
        assert!(g.place_template_entry(
            &entry,
            BlockPos::new(20, 64, 8),
            crate::game::structure_blocks::load_settings(
                horizon_world::worldgen::structure::Mirror::None,
                horizon_world::worldgen::structure::Rotation::None,
                1.0,
                37,
                false
            ),
            horizon_world::worldgen::structure::Mirror::None,
            horizon_world::worldgen::structure::Rotation::None
        ));
        let placed = g
            .world
            .block_entity_extras((20, 64, 8))
            .pending_loot
            .unwrap();
        assert_eq!(placed.table, Tag::String(TABLE.into()));
        assert!(matches!(placed.seed, Some(Tag::Long(_))));
        assert!(
            g.world
                .block_entities
                .get(&(20, 64, 8))
                .unwrap()
                .slots()
                .iter()
                .all(Option::is_none)
        );
        assert_eq!(pending_bytes(g, POS), original);
        open(g, (20, 64, 8), 0.0);
        consumed(g, (20, 64, 8));
        assert_eq!(count(g, (20, 64, 8), "minecraft:diamond"), 1);
        assert_eq!(pending_bytes(g, POS), original);
        open(g, (20, 64, 8), 0.0);
        assert_eq!(count(g, (20, 64, 8), "minecraft:diamond"), 1);
    });
}

fn clone_selection_modes_match_pending_payloads() {
    with_game(|g| {
        place(g, POS, TABLE, 37);
        let original = pending_bytes(g, POS);
        place(g, (13, 64, 8), "test:right", 91);
        let skipped = pending_bytes(g, (13, 64, 8));
        for mode in ["masked", "filtered minecraft:chest", "replace"] {
            place(g, (12, 64, 8), "test:right", 91);
            assert!(g.run_command(&format!("clone 8 64 8 9 64 8 12 64 8 {mode}")));
            assert_eq!(pending_bytes(g, (12, 64, 8)), original);
            assert_eq!(pending_bytes(g, POS), original);
            if mode == "replace" {
                consumed(g, (13, 64, 8));
                assert!(g.world.block_entities.get(&(13, 64, 8)).is_none());
            } else {
                assert_eq!(pending_bytes(g, (13, 64, 8)), skipped);
            }
            assert_eq!(dropped(g, "minecraft:emerald"), 0);
        }
        let before = pending_bytes(g, (12, 64, 8));
        assert!(!g.run_command("clone 8 64 8 8 64 8 12 64 8 filtered minecraft:stone"));
        assert_eq!(pending_bytes(g, (12, 64, 8)), before);
    });
}

#[test]
#[ignore = "daily"]
fn random_selection_streams() {
    use horizon_client::random_sequences::RandomSequences;
    let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
    let table = serde_json::json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:diamond"}]}]});
    let mut sequence_table = table.clone();
    sequence_table["random_sequence"] = "test:seq".into();
    let files = [
        (
            "loot_table/test:plain".into(),
            vec![serde_json::to_vec(&table).unwrap()],
        ),
        (
            "loot_table/test:sequence".into(),
            vec![serde_json::to_vec(&sequence_table).unwrap()],
        ),
    ];
    let loot = LootData::load(
        &root,
        &PackOverlay {
            files: &files,
            ..PackOverlay::NONE
        },
        &horizon_world::item_defaults::read(&root).unwrap(),
    )
    .unwrap();
    let roll = |table: &str, seed, random: &mut LegacyRandom, sequences: &mut RandomSequences| {
        let pending = PendingContainerLoot {
            table: Tag::String(table.into()),
            seed,
        };
        let mut slots = vec![None; 27];
        fill_with_random(
            &pending,
            &mut slots,
            &loot,
            random,
            sequences,
            123,
            Default::default(),
            |_| {},
        );
        slots
            .iter()
            .enumerate()
            .filter_map(|(i, s)| s.as_ref().map(|s| (i, s.item.to_string(), s.count)))
            .collect::<Vec<_>>()
    };
    let plain = |seed| {
        roll(
            "test:plain",
            None,
            &mut LegacyRandom::new(seed),
            &mut RandomSequences::default(),
        )
    };
    let first = plain(1);
    assert_eq!(first, vec![(24, "minecraft:diamond".to_owned(), 1)]);
    assert_eq!(first, plain(1));
    assert_ne!(first, plain(2));
    let mut level = LegacyRandom::new(1);
    let mut unchanged = level.clone();
    let mut sequences = RandomSequences::default();
    let before = sequences.get("test:seq", 123).state_words();
    let result = roll("test:sequence", None, &mut level, &mut sequences);
    assert_ne!(before, sequences.get("test:seq", 123).state_words());
    assert_eq!(level.next_int(), unchanged.next_int());
    assert_eq!(
        result,
        roll(
            "test:sequence",
            Some(Tag::Long(0)),
            &mut LegacyRandom::new(2),
            &mut RandomSequences::default()
        )
    );
    assert_eq!(
        roll(
            "test:sequence",
            Some(Tag::Long(37)),
            &mut LegacyRandom::new(1),
            &mut sequences
        ),
        roll(
            "test:sequence",
            Some(Tag::Long(37)),
            &mut LegacyRandom::new(2),
            &mut RandomSequences::default()
        )
    );
}

fn explicit_seed_is_position_independent() {
    with_game(|g| {
        let other = (100, 70, -5);
        g.world.chunks.insert(
            (6, -1),
            Arc::new(horizon_client::chunk::Chunk::empty(6, -1)),
        );
        let snapshot = |g: &Game<'_>, pos| g.world.block_entities[&pos].slots().to_vec();
        let mut level_results = Vec::new();
        for seed in [1, 2, 1] {
            g.run_command("setblock 8 64 8 minecraft:air");
            assert!(g.run_command("setblock 8 64 8 minecraft:chest{LootTable:\"test:left\"}"));
            *g.world.random = LegacyRandom::new(seed);
            g.unpack_loot(LootTarget::Block(POS), None);
            level_results.push(snapshot(g, POS));
        }
        assert_eq!(
            level_results[0]
                .iter()
                .enumerate()
                .filter_map(|(i, s)| s.as_ref().map(|s| (i, s.item.to_string(), s.count)))
                .collect::<Vec<_>>(),
            vec![(24, "minecraft:diamond".to_owned(), 1)]
        );
        assert_eq!(level_results[0], level_results[2]);
        assert_ne!(level_results[0], level_results[1]);
        for pos in [POS, other] {
            g.run_command(&format!(
                "setblock {} {} {} minecraft:air",
                pos.0, pos.1, pos.2
            ));
            assert!(g.run_command(&format!(
                "setblock {} {} {} minecraft:chest{{LootTable:\"test:sequence_position\"}}",
                pos.0, pos.1, pos.2
            )));
            g.rand_sequences = Default::default();
            g.unpack_loot(LootTarget::Block(pos), None);
        }
        assert_eq!(snapshot(g, POS), snapshot(g, other));
        place(g, POS, TABLE, 37);
        *g.world.random = LegacyRandom::new(1);
        g.unpack_loot(LootTarget::Block(POS), None);
        let contents = g.world.block_entities[&POS].slots().to_vec();
        place(g, other, TABLE, 37);
        *g.world.random = LegacyRandom::new(2);
        g.unpack_loot(LootTarget::Block(other), None);
        assert_eq!(contents, g.world.block_entities[&other].slots());
    });
}

struct FillListener {
    mode: u32,
    seen: Arc<Mutex<Vec<(String, Option<u128>, [f64; 3], Vec<(String, u32)>)>>>,
}
#[mod_api::__subscriptions]
impl mod_api::Hooks for FillListener {
    fn container_loot_filled(
        &self,
        _: &dyn mod_api::LevelHost,
        event: &mut mod_api::ContainerLootFilledEvent<'_>,
    ) {
        self.seen.lock().unwrap().push((
            event.context.table.into(),
            event.context.opener,
            event.context.origin,
            event
                .drops
                .iter()
                .map(|s| {
                    (
                        mod_api::__private::item_key_id(s.item())
                            .unwrap()
                            .to_string(),
                        s.count(),
                    )
                })
                .collect(),
        ));
        if self.mode == 3 {
            event.drops.clear();
            let key = mod_api::__private::item_key("minecraft:diamond").unwrap();
            event
                .drops
                .extend((0..30).map(|_| mod_api::ItemStack::new(key, 1).unwrap()));
        } else if self.mode != 0 {
            event.drops.clear();
            let id = if self.mode == 1 {
                "minecraft:emerald"
            } else {
                "minecraft:diamond"
            };
            let key = mod_api::__private::item_key(id).unwrap();
            event
                .drops
                .push(mod_api::ItemStack::new(key, if self.mode == 1 { 3 } else { 100 }).unwrap());
        }
    }
}

fn container_fill_listener_contract() {
    with_game(|g| {
        g.options.dedicated = Some(Vec::new());
        place(g, POS, TABLE, 37);
        open(g, POS, 0.0);
        let baseline = g.world.block_entities[&POS].slots().to_vec();
        let baseline_random = g.world.random.state();
        mod_platform::testing::with_mods(
            vec![mod_api::ModEntry::new("empty", Box::new(EmptyFillMod))].into(),
            || {
                place(g, POS, TABLE, 37);
                open(g, POS, 0.0);
                assert_eq!(g.world.block_entities[&POS].slots(), baseline);
                assert_eq!(g.world.random.state(), baseline_random);
            },
        );
        let seen = Arc::new(Mutex::new(Vec::new()));
        let mods = |mode| {
            vec![mod_api::ModEntry::new(
                "fill",
                Box::new(FillListener {
                    mode,
                    seen: seen.clone(),
                }),
            )]
            .into()
        };
        // A subscribed observer must preserve both the seedless roll and RNG state.
        *g.world.random = LegacyRandom::new(123);
        place(g, POS, TABLE, 0);
        open(g, POS, 0.0);
        let seedless_slots = g.world.block_entities[&POS].slots().to_vec();
        let seedless_random = g.world.random.state();
        mod_platform::testing::with_mods(mods(0), || {
            *g.world.random = LegacyRandom::new(123);
            place(g, POS, TABLE, 0);
            open(g, POS, 0.0);
            assert_eq!(g.world.block_entities[&POS].slots(), seedless_slots);
            assert_eq!(g.world.random.state(), seedless_random);
        });
        seen.lock().unwrap().clear();
        mod_platform::testing::with_mods(mods(3), || {
            place(g, POS, TABLE, 37);
            open(g, POS, 0.0);
            let slots = g.world.block_entities[&POS].slots();
            assert_eq!(slots.len(), 27);
            for slot in slots {
                let stack = slot.as_ref().unwrap();
                assert_eq!(&*stack.item, "minecraft:diamond");
                assert_eq!(stack.count, 1);
            }
            assert_eq!(30 - count(g, POS, "minecraft:diamond"), 3);
        });
        seen.lock().unwrap().clear();
        mod_platform::testing::with_mods(mods(1), || {
            place(g, POS, TABLE, 37);
            open(g, POS, 0.0);
            assert_eq!(count(g, POS, "minecraft:emerald"), 3);
            assert_eq!(
                g.world.block_entities[&POS]
                    .slots()
                    .iter()
                    .enumerate()
                    .filter_map(|(i, s)| s.as_ref().map(|s| (i, s.count)))
                    .collect::<Vec<_>>(),
                vec![(5, 1), (17, 1), (25, 1)]
            );
            assert_eq!(count(g, POS, "minecraft:diamond"), 0);
            open(g, POS, 0.0);
            assert_eq!(seen.lock().unwrap().len(), 1);
            assert!(g.run_command("setblock 12 64 8 minecraft:stone"));
            assert!(g.run_command("setblock 12 64 8 minecraft:air destroy"));
            assert_eq!(seen.lock().unwrap().len(), 1);
        });
        let first = seen.lock().unwrap().remove(0);
        assert_eq!(first.0, TABLE);
        assert_eq!(first.1, Some(12));
        assert_eq!(first.2, [8.5, 64.5, 8.5]);
        assert_eq!(first.3, vec![("minecraft:diamond".into(), 1)]);
        mod_platform::testing::with_mods(mods(0), || {
            assert!(g.run_command("setblock 8 63 8 minecraft:hopper"));
            place(g, POS, TABLE, 37);
            g.hopper_try_move_items((8, 63, 8));
            assert_eq!(seen.lock().unwrap()[0].1, None);
        });
        seen.lock().unwrap().clear();
        mod_platform::testing::with_mods(mods(0), || {
            assert!(g.run_command("setblock 8 64 8 minecraft:chest[facing=north,type=left]{LootTable:\"test:left\",LootTableSeed:37L}"));
            assert!(g.run_command("setblock 9 64 8 minecraft:chest[facing=north,type=right]{LootTable:\"test:right\",LootTableSeed:91L}"));
            open(g, POS, 0.0);
            let reads = seen.lock().unwrap();
            assert_eq!(reads.len(), 2);
            assert_eq!(reads[0].0, "test:left");
            assert_eq!(reads[0].3, vec![("minecraft:diamond".into(), 1)]);
            assert_eq!(reads[1].0, "test:right");
            assert_eq!(reads[1].3, vec![("minecraft:emerald".into(), 2)]);
        });
        seen.lock().unwrap().clear();
        mod_platform::testing::with_mods(mods(2), || {
            place(g, POS, TABLE, 37);
            open(g, POS, 0.0);
            assert_eq!(count(g, POS, "minecraft:diamond"), 64);
            assert_eq!(
                g.world.block_entities[&POS]
                    .slots()
                    .iter()
                    .enumerate()
                    .filter_map(|(i, s)| s.as_ref().map(|s| (i, s.count)))
                    .collect::<Vec<_>>(),
                vec![(5, 1), (6, 1), (10, 4), (16, 27), (17, 12), (25, 19)]
            );
        });
    });
}

struct EmptyFillMod;
#[mod_api::__subscriptions]
impl mod_api::Hooks for EmptyFillMod {}

fn saved_carts_keep_a_loot_table_only_when_they_hold_a_container() {
    with_game(|game| {
        let keys: std::collections::HashSet<_> = [(0, 0)].into();
        let cases = [
            (
                "minecraft:minecart",
                "minecraft:chests/simple_dungeon",
                None,
            ),
            (
                "minecraft:tnt_minecart",
                "minecraft:chests/simple_dungeon",
                None,
            ),
            (
                "minecraft:chest_minecart",
                "minecraft:chests/simple_dungeon",
                Some("minecraft:chests/simple_dungeon"),
            ),
            ("minecraft:hopper_minecart", "", Some("minecraft:")),
            // readChestVehicleSaveData: a table that is not an identifier reads as none, so
            // neither it nor the seed is saved.
            ("minecraft:chest_minecart", "Bad Name!", None),
        ];
        for (i, (kind, table, kept)) in cases.into_iter().enumerate() {
            let source =
                format!(r#"{{Pos:[8.5d,64d,8.5d],LootTable:"{table}",LootTableSeed:73L}}"#);
            let (tag, _) = horizon_core::nbt::parse_snbt(&source).unwrap();
            game.load_cart(tag, Some([61, 0, 0, i as i32 + 1]), kind);
            // The entity chunk's own writer, as a save writes it to disk.
            let (_, saved) = game.save_cart(game.carts.len() - 1, &keys).unwrap();
            assert_eq!(
                (
                    saved.get("LootTable").cloned(),
                    saved.get("LootTableSeed").cloned()
                ),
                (
                    kept.map(|table| Tag::String(table.into())),
                    kept.map(|_| Tag::Long(73))
                ),
                "{kind}"
            );
        }
    });
}

struct DurableStickListener;
#[mod_api::__subscriptions]
impl mod_api::Hooks for DurableStickListener {
    fn container_loot_filled(
        &self,
        _: &dyn mod_api::LevelHost,
        event: &mut mod_api::ContainerLootFilledEvent<'_>,
    ) {
        let key = mod_api::__private::item_key("minecraft:stick").unwrap();
        let mut stick = mod_api::ItemStack::new(key, 1).unwrap();
        stick
            .set_components(
                [
                    ("minecraft:max_stack_size".into(), Tag::Int(1)),
                    ("minecraft:max_damage".into(), Tag::Int(10)),
                ]
                .into(),
            )
            .unwrap();
        stick.set_damage(0);
        event.drops.clear();
        event.drops.push(stick);
    }
}

fn container_fill_preserves_explicit_zero_damage_on_custom_durable_item() {
    with_game(|game| {
        game.options.dedicated = Some(Vec::new());
        mod_platform::testing::with_mods(
            vec![mod_api::ModEntry::new(
                "durable_stick",
                Box::new(DurableStickListener),
            )]
            .into(),
            || {
                place(game, POS, TABLE, 37);
                open(game, POS, 0.0);
                let stacks: Vec<_> = game.world.block_entities[&POS]
                    .slots()
                    .iter()
                    .flatten()
                    .collect();
                assert_eq!(stacks.len(), 1);
                let stick = stacks[0];
                assert_eq!(&*stick.item, "minecraft:stick");
                assert_eq!(stick.count, 1);
                assert_eq!(stick.damage, 0);
                assert_eq!(game.items().stack_max_damage(stick), 10);
            },
        );
    });
}

#[test]
#[ignore = "daily"]
fn transfers_between_containers() {
    if in_child("transfers_between_containers") {
        return;
    }
    run_group(&[
        (
            "hopper_pull_unpacks_source_once",
            hopper_pull_unpacks_source_once as fn(),
        ),
        (
            "hopper_push_unpacks_destination_once",
            hopper_push_unpacks_destination_once as fn(),
        ),
        (
            "hopper_minecart_pull_unpacks_source_once",
            hopper_minecart_pull_unpacks_source_once as fn(),
        ),
        (
            "dropper_push_unpacks_destination_once",
            dropper_push_unpacks_destination_once as fn(),
        ),
        (
            "dispenser_selection_unpacks_source_once",
            dispenser_selection_unpacks_source_once as fn(),
        ),
        (
            "double_chest_hopper_pull_reads_only_needed_half",
            double_chest_hopper_pull_reads_only_needed_half as fn(),
        ),
        (
            "double_chest_hopper_push_keeps_other_half_pending",
            double_chest_hopper_push_keeps_other_half_pending as fn(),
        ),
    ]);
}

#[test]
#[ignore = "daily"]
fn player_and_world_triggers() {
    if in_child("player_and_world_triggers") {
        return;
    }
    run_group(&[
        (
            "player_open_uses_admitted_guest_luck_and_entity",
            player_open_uses_admitted_guest_luck_and_entity as fn(),
        ),
        (
            "comparator_first_read_unpacks_and_measures",
            comparator_first_read_unpacks_and_measures as fn(),
        ),
        (
            "block_break_unpacks_before_contents_drop",
            block_break_unpacks_before_contents_drop as fn(),
        ),
        (
            "explosion_unpacks_before_contents_drop",
            explosion_unpacks_before_contents_drop as fn(),
        ),
        (
            "double_chest_rolls_each_half_once",
            double_chest_rolls_each_half_once as fn(),
        ),
        (
            "chest_minecart_open_uses_opener_once",
            chest_minecart_open_uses_opener_once as fn(),
        ),
        (
            "chest_minecart_break_unpacks_before_drop",
            chest_minecart_break_unpacks_before_drop as fn(),
        ),
        (
            "detector_rail_comparator_unpacks_cart_once",
            detector_rail_comparator_unpacks_cart_once as fn(),
        ),
    ]);
}

#[test]
#[ignore = "daily"]
fn commands_clone_and_structures() {
    if in_child("commands_clone_and_structures") {
        return;
    }
    run_group(&[
        (
            "item_replace_unpacks_before_writing",
            item_replace_unpacks_before_writing as fn(),
        ),
        (
            "execute_if_items_unpacks_before_reading",
            execute_if_items_unpacks_before_reading as fn(),
        ),
        (
            "loot_insert_unpacks_before_inserting",
            loot_insert_unpacks_before_inserting as fn(),
        ),
        (
            "clone_copy_preserves_pending_loot_independently",
            clone_copy_preserves_pending_loot_independently as fn(),
        ),
        (
            "clone_move_transfers_pending_loot",
            clone_move_transfers_pending_loot as fn(),
        ),
        (
            "clone_selection_modes_match_pending_payloads",
            clone_selection_modes_match_pending_payloads as fn(),
        ),
        (
            "structure_capture_disk_round_trip_keeps_pending_loot",
            structure_capture_disk_round_trip_keeps_pending_loot as fn(),
        ),
        (
            "saved_carts_keep_a_loot_table_only_when_they_hold_a_container",
            saved_carts_keep_a_loot_table_only_when_they_hold_a_container as fn(),
        ),
    ]);
}

#[test]
#[ignore = "daily"]
fn seeds_and_fill_listeners() {
    if in_child("seeds_and_fill_listeners") {
        return;
    }
    run_group(&[
        (
            "explicit_seed_is_position_independent",
            explicit_seed_is_position_independent as fn(),
        ),
        (
            "container_fill_listener_contract",
            container_fill_listener_contract as fn(),
        ),
        (
            "container_fill_preserves_explicit_zero_damage_on_custom_durable_item",
            container_fill_preserves_explicit_zero_damage_on_custom_durable_item as fn(),
        ),
    ]);
}
