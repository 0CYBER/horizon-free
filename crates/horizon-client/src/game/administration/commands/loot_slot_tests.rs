use crate::game::commands::items::Slot;
use crate::game::{Audio, Game, Options, Progress, Settings, load_resources};

fn with_game(check: impl FnOnce(&mut Game<'_>)) {
    let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
    let scratch = tempfile::tempdir().unwrap();
    let settings = Settings::default();
    let loaded = load_resources(
        horizon_assets::resource_source::ResourceRoot::directory(root.clone()),
        Vec::new(),
        None,
        &root,
        &settings,
        std::time::Instant::now(),
        false,
        crate::game::execution_role::ExecutionRole::IntegratedServer,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.game_dir = Some(scratch.path().to_path_buf());
    options.execution_role = crate::game::execution_role::ExecutionRole::IntegratedServer;
    let mut game = Game::new(
        &loaded.resources,
        loaded.pack,
        loaded.gui.unwrap(),
        options,
        settings,
        Audio::silent(&horizon_assets::resource_source::ResourceSource::directory(
            root.clone(),
        )),
        None,
    )
    .unwrap();
    game.build_registries();

    game.set_allow_commands(true);
    game.world.chunks.insert(
        (0, 0),
        std::sync::Arc::new(horizon_client::chunk::Chunk::empty(0, 0)),
    );
    game.block_ticks
        .add_empty_chunk(horizon_core::position::ChunkPos::new(0, 0))
        .unwrap();
    assert!(game.run_command("forceload add 0 0"));
    check(&mut game);
}

#[test]
#[ignore = "daily"]
fn item_replace_jukebox_accepts_music_disc_cat() {
    with_game(|game| {
        assert!(game.run_command("setblock 4 200 4 jukebox"));
        assert!(
            game.run_command(
                "item replace block 4 200 4 container.0 with minecraft:music_disc_cat"
            )
        );
        assert_eq!(
            game.slot_get(&Slot::Block([4, 200, 4], 0))
                .map(|s| (s.item.to_string(), s.count)),
            Some(("minecraft:music_disc_cat".into(), 1))
        );
    });
}

#[test]
#[ignore = "daily"]
fn loot_empty_replace_jukebox_preserves_music_disc_13() {
    with_game(|game| {
        assert!(game.run_command("setblock 4 200 4 jukebox"));
        assert!(
            game.run_command("item replace block 4 200 4 container.0 with minecraft:music_disc_13")
        );
        assert_eq!(
            game.slot_get(&Slot::Block([4, 200, 4], 0))
                .unwrap()
                .item
                .as_ref(),
            "minecraft:music_disc_13"
        );
        assert!(game.run_command(r#"loot replace block 4 200 4 container.0 1 loot {"pools":[]}"#));
        assert_eq!(
            game.slot_get(&Slot::Block([4, 200, 4], 0))
                .map(|s| (s.item.to_string(), s.count)),
            Some(("minecraft:music_disc_13".into(), 1))
        );
    });
}

#[test]
#[ignore = "daily"]
fn loot_replace_furnace_fuel_refuses_non_fuel() {
    with_game(|game| {
        assert!(game.run_command("setblock 4 200 4 furnace"));
        assert!(game.run_command("item replace block 4 200 4 container.1 with minecraft:coal 3"));
        assert!(game.run_command(r#"loot replace block 4 200 4 container.1 1 loot {"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:diamond"}]}]}"#));
        assert_eq!(
            game.slot_get(&Slot::Block([4, 200, 4], 1))
                .map(|s| (s.item.to_string(), s.count)),
            Some(("minecraft:coal".into(), 3))
        );
    });
}

#[test]
#[ignore = "daily"]
fn loot_replace_clears_empty_position_and_writes_following_real_item() {
    with_game(|game| {
        assert!(game.run_command("setblock 4 200 4 chest"));
        assert!(game.run_command("item replace block 4 200 4 container.0 with minecraft:stone 5"));
        assert!(game.run_command("item replace block 4 200 4 container.1 with minecraft:stick 3"));
        assert!(game.run_command(
            r#"loot replace block 4 200 4 container.0 2 loot {"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:iron_ingot","modifier":{"type":"minecraft:set_count","count":0}}]},{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:diamond"}]}]}"#,
        ));
        assert_eq!(game.slot_get(&Slot::Block([4, 200, 4], 0)), None);
        assert_eq!(
            game.slot_get(&Slot::Block([4, 200, 4], 1))
                .map(|s| (s.item.to_string(), s.count)),
            Some(("minecraft:diamond".into(), 1))
        );
    });
}

#[test]
#[ignore = "daily"]
fn item_commands_bypass_block_acceptance() {
    with_game(|game| {
        for slot in [1, 2] {
            // Destroy first so repeated rows also place a fresh container.
            assert!(game.run_command("setblock 4 200 4 furnace destroy"));
            assert!(game.run_command(&format!(
                "item replace block 4 200 4 container.{slot} with minecraft:diamond 2"
            )));
            assert_eq!(
                game.slot_get(&Slot::Block([4, 200, 4], slot))
                    .map(|s| (s.item.to_string(), s.count)),
                Some(("minecraft:diamond".into(), 2))
            );
            assert!(game.run_command(&format!(
                r#"item modify block 4 200 4 container.{slot} {{"type":"minecraft:set_count","count":3}}"#
            )));
            assert_eq!(
                game.slot_get(&Slot::Block([4, 200, 4], slot))
                    .map(|s| (s.item.to_string(), s.count)),
                Some(("minecraft:diamond".into(), 3))
            );
        }
        for modify in [false, true] {
            assert!(game.run_command("setblock 4 200 4 jukebox destroy"));
            assert!(game.run_command(
                "item replace block 4 200 4 container.0 with minecraft:music_disc_cat"
            ));
            let command = if modify {
                r#"item modify block 4 200 4 container.0 {"type":"minecraft:set_count","count":0}"#
            } else {
                "item replace block 4 200 4 container.0 with minecraft:air"
            };
            assert!(game.run_command(command));
            assert!(game.slot_get(&Slot::Block([4, 200, 4], 0)).is_none());
        }
    });
}

#[test]
#[ignore = "daily"]
fn empty_loot_preserves_restricted_slots_but_clears_chest() {
    with_game(|game| {
        for (block, slot, item) in [
            ("furnace", 1, "coal"),
            ("furnace", 2, "iron_ingot"),
            ("brewing_stand", 0, "potion"),
            ("chiseled_bookshelf", 0, "book"),
            ("crafter{disabled_slots:[I;0]}", 0, "diamond"),
            ("chest", 0, "diamond"),
        ] {
            // An unchanged block state makes `setblock ... replace` fail.
            assert!(game.run_command(&format!("setblock 4 200 4 {block} destroy")));
            assert!(game.run_command(&format!(
                "item replace block 4 200 4 container.{slot} with minecraft:{item}"
            )));
            assert!(game.run_command(&format!(
                r#"loot replace block 4 200 4 container.{slot} 1 loot {{"pools":[]}}"#
            )));
            let actual = game
                .slot_get(&Slot::Block([4, 200, 4], slot))
                .map(|s| (s.item.to_string(), s.count));
            let expected = if block == "chest" {
                None
            } else {
                Some((format!("minecraft:{item}"), 1))
            };
            assert_eq!(actual, expected, "{block} slot {slot}");
        }
        assert!(game.run_command("setblock 4 200 4 chest destroy"));
        assert!(game.run_command(r#"loot replace block 4 200 4 container.0 1 loot {"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:diamond"}]}]}"#));
        assert_eq!(
            game.slot_get(&Slot::Block([4, 200, 4], 0))
                .map(|s| (s.item.to_string(), s.count)),
            Some(("minecraft:diamond".into(), 1))
        );
    });
}

/// `loot replace` with a `mine` source takes a drop only into a slot that accepts it
/// (`Container.canPlaceItem`): a furnace's fuel slot takes the coal an ore drops, refuses the
/// raw iron another ore drops and keeps what it held, while the input slot takes the raw iron.
#[test]
#[ignore = "daily"]
fn loot_replace_mine_fills_an_accepting_slot_and_leaves_a_refusing_one() {
    with_game(|game| {
        let slot = |game: &mut Game<'_>, n| {
            game.slot_get(&Slot::Block([4, 200, 4], n))
                .map(|s| (s.item.to_string(), s.count))
        };
        assert!(game.run_command("setblock 4 200 4 furnace"));
        // Accepts: a diamond pickaxe mines one coal out of coal ore, and coal is fuel.
        assert!(game.run_command("setblock 6 200 4 coal_ore"));
        assert!(game.run_command(
            "loot replace block 4 200 4 container.1 1 mine 6 200 4 minecraft:diamond_pickaxe"
        ));
        assert_eq!(slot(game, 1), Some(("minecraft:coal".into(), 1)));
        // Refuses: iron ore drops raw iron, which is not fuel; the coal stays.
        assert!(game.run_command("setblock 6 200 4 iron_ore"));
        assert!(game.run_command(
            "loot replace block 4 200 4 container.1 1 mine 6 200 4 minecraft:diamond_pickaxe"
        ));
        assert_eq!(slot(game, 1), Some(("minecraft:coal".into(), 1)));
        // The same drop goes into the input slot, which takes anything.
        assert!(game.run_command(
            "loot replace block 4 200 4 container.0 1 mine 6 200 4 minecraft:diamond_pickaxe"
        ));
        assert_eq!(slot(game, 0), Some(("minecraft:raw_iron".into(), 1)));
    });
}

/// The same for a `kill` source: a sheared sheep's loot is mutton, which a furnace's input slot
/// takes and its fuel slot refuses, leaving the coal there.
#[test]
#[ignore = "daily"]
fn loot_replace_kill_fills_an_accepting_slot_and_leaves_a_refusing_one() {
    with_game(|game| {
        let slot = |game: &mut Game<'_>, n| {
            game.slot_get(&Slot::Block([4, 200, 4], n))
                .map(|s| (s.item.to_string(), s.count))
        };
        assert!(game.run_command("setblock 4 200 4 furnace"));
        assert!(game.run_command("summon minecraft:sheep 8 200 8 {Sheared:1b,NoAI:1b}"));
        assert!(game.run_command("item replace block 4 200 4 container.1 with minecraft:coal 3"));
        // Refuses: mutton is not fuel.
        assert!(game.run_command(
            "loot replace block 4 200 4 container.1 1 kill @e[type=minecraft:sheep,limit=1]"
        ));
        assert_eq!(slot(game, 1), Some(("minecraft:coal".into(), 3)));
        // Accepts: the input slot takes the mutton (one or two of it).
        assert!(game.run_command(
            "loot replace block 4 200 4 container.0 1 kill @e[type=minecraft:sheep,limit=1]"
        ));
        let (item, count) = slot(game, 0).expect("the input slot took the drop");
        assert_eq!(item, "minecraft:mutton");
        assert!((1..=2).contains(&count), "{count}");
    });
}

fn run(game: &mut Game<'_>, command: &str) -> (bool, String) {
    game.chat_capture = Some(Vec::new());
    let ok = game.run_command(command);
    let said = game.chat_capture.take().unwrap_or_default();
    let said = said
        .iter()
        .map(|t| t.component().to_string())
        .collect::<Vec<_>>()
        .join("\n");
    (ok, said)
}

/// `LootCommand`: `mine <pos>` with no tool mines with an empty hand (`ItemStack.EMPTY`), whoever runs it;
/// `mainhand` reads the source's hand (`getSourceHandItem`); a source that is not a living entity gets
/// `commands.drop.no_held_items`, and one without an entity `permissions.requires.entity`.
#[test]
#[ignore = "daily"]
fn loot_mine_reads_a_hand_only_when_named_and_only_a_living_ones() {
    use crate::game::commands::{Target, execute::Source};
    use horizon_client::containers::BlockEntity;

    with_game(|game| {
        let slot = |game: &Game<'_>| {
            let Some(BlockEntity::Chest(items)) = game.world.block_entities.get(&(12, 200, 4))
            else {
                panic!("the chest the test placed");
            };
            items[0].clone()
        };
        assert!(game.run_command("fill 0 199 0 15 199 15 stone"));
        game.command_source = Some(Source {
            silent: false,
            ..game.server_source()
        });
        assert!(game.run_command("setblock 12 200 4 chest strict"));
        assert!(game.run_command("setblock 4 200 4 grass_block strict"));
        assert!(
            game.run_command("summon armor_stand 8.5 200 8.5 {UUID:[I;62,0,0,1],NoGravity:1b}")
        );
        assert!(game.run_command("summon marker 9.5 200 9.5 {UUID:[I;63,0,0,1]}"));
        let stand = "execute as 0000003e-0000-0000-0000-000000000001 run ";
        assert!(game.run_command(
            "item replace entity 0000003e-0000-0000-0000-000000000001 weapon.mainhand with \
             diamond_shovel[enchantments={silk_touch:1}]"
        ));
        let mine = "loot replace block 12 200 4 container.0 mine 4 200 4";
        // The stand's silk touch shovel, named: the grass block itself.
        assert!(run(game, &format!("{stand}{mine} mainhand")).0);
        assert_eq!(
            slot(game).map(|s| s.item.to_string()).as_deref(),
            Some("minecraft:grass_block")
        );
        // No tool named: an empty hand, so dirt, though the stand holds the shovel.
        assert!(run(game, &format!("{stand}{mine}")).0);
        assert_eq!(
            slot(game).map(|s| s.item.to_string()).as_deref(),
            Some("minecraft:dirt")
        );
        // A marker holds nothing. The marker is made the command's source directly: run through
        // `execute as`, the failed command prints nothing here, so its message could not be read.
        let marker = game
            .others
            .iter()
            .find(|o| o.kind == "minecraft:marker")
            .unwrap()
            .id;
        let server = game.command_source;
        game.command_source = server.map(|s| Source {
            entity: Some(Target::Other(marker)),
            ..s
        });
        let (ok, said) = run(game, &format!("{mine} mainhand"));
        assert!(
            !ok && said.contains("commands.drop.no_held_items"),
            "{said}"
        );
        // The server's own source has no entity.
        game.command_source = server;
        let (ok, said) = run(game, &format!("{mine} offhand"));
        assert!(
            !ok && said.contains("permissions.requires.entity"),
            "{said}"
        );
        assert_eq!(
            slot(game).map(|s| s.item.to_string()).as_deref(),
            Some("minecraft:dirt")
        );
    });
}

#[test]
#[ignore = "daily"]
fn loot_replace_uses_live_world_pack_tables() {
    use horizon_client::command::{Registries, network, vanilla};
    use horizon_client::lan::commands::CommandTree;
    use horizon_items::equipment::MAINHAND;
    use horizon_protocol::{
        buf::{Codec, Reader, Writer},
        packets::play::Commands,
        registry::Builtin,
    };

    const TABLE: &str = "loot_fixture:biome/plains";
    const COMMAND: &str = "loot replace entity @s weapon.mainhand loot ";

    with_game(|game| {
        let scratch = tempfile::tempdir().unwrap();
        game.world_root = scratch.path().to_path_buf();
        let pack = scratch.path().join("datapacks/loot_fixture");
        let tables = pack.join("data/loot_fixture/loot_table/biome");
        std::fs::create_dir_all(&tables).unwrap();
        std::fs::write(
            pack.join("pack.mcmeta"),
            r#"{"pack":{"description":"Loot command fixture"}}"#,
        )
        .unwrap();
        std::fs::write(
            tables.join("plains.json"),
            r#"{"type":"minecraft:chest","pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:dirt"}]}]}"#,
        )
        .unwrap();

        let root = game.options.resources.clone();
        let packs = game.open_world_datapacks(&[]).unwrap();
        game.open_world_tables(&packs, &root).unwrap();
        game.load_server_loot();
        game.load_world_state().unwrap();
        game.set_allow_commands(true);

        // The wire tree has the argument type, but not the world's reloadable registry.
        let builtin = Builtin::load(&Builtin::find(&root).unwrap()).unwrap();
        let permission = 2;
        let packet = CommandTree::load(&root)
            .unwrap()
            .packet(permission, false, &builtin);
        let mut wire = Writer::new();
        Commands::write(&packet, &mut wire);
        let packet = Commands::read(&mut Reader::new(&wire.data)).unwrap();
        let client = network::from_packet(&packet, vanilla::dispatcher());
        // The client receives its permission separately from ClientboundCommandsPacket.
        // Without game-master permission, @s is refused before the loot argument is read.
        let reg = Registries {
            permission,
            ..Default::default()
        };
        let parse = client.parse(&format!("{COMMAND}{TABLE}"), &reg);
        assert!(parse.is_valid(), "{parse:#?}");
        let prefix = format!("{COMMAND}loot_fixture:biome/pla");
        let parse = client.parse(&prefix, &reg);
        let (range, local, asks_server) =
            client.client_suggestions_asking(&parse, prefix.len(), &reg);
        assert!(
            asks_server && local.is_empty(),
            "expected a server request, got {local:?} at {range:?} for {parse:#?}"
        );
        let suggestions = |game: &Game<'_>| {
            let d = &game.ui.dispatcher.0;
            let reg = &game.ui.registries;
            let parse = d.parse(&prefix, reg);
            d.client_suggestions(&parse, prefix.len(), reg).1
        };
        assert!(suggestions(game).iter().any(|entry| entry.text == TABLE));

        assert!(
            game.run_command(
                r#"summon armor_stand 8.5 200 8.5 {Tags:["loot_fixture"],NoGravity:1b}"#
            )
        );
        let as_stand = format!("execute as @e[tag=loot_fixture,limit=1] run {COMMAND}");
        let command = format!("{as_stand}{TABLE}");
        assert!(game.run_command(&command));
        let assert_slot = |game: &Game<'_>| {
            let hand = game.armor_stands[0].equipment.get(MAINHAND).unwrap();
            assert_eq!((hand.item.as_ref(), hand.count), ("minecraft:dirt", 1));
        };
        assert_slot(game);

        assert!(game.run_command(r#"datapack disable "file/loot_fixture""#));
        assert!(suggestions(game).is_empty());
        let (ok, feedback) = run(game, &command);
        assert!(!ok);
        assert!(
            feedback.contains("argument.resource_or_id.no_such_element"),
            "{feedback}"
        );
        assert_slot(game);
    });
}
