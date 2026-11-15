use horizon_core::geometry::Vec3;

#[test]
#[ignore = "daily"]
fn warning_dispatch_uses_neutral_at_tick_four() {
    use crate::game::{Audio, Game, Options, Progress, Settings, load_resources};
    use horizon_client::motion::PlayerMotion;
    let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
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
    let scratch = std::env::temp_dir().join(format!(
        "horizon-ominous-sound-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&scratch).unwrap();
    game.world_root = scratch.clone();
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(4.5, 100.0, 4.5)).unwrap());

    const WARNING: &str = "minecraft:block.trial_spawner.about_to_spawn_item";
    let center = [6.5, 102.5, 4.5];
    game.audio.capture = Some(Vec::new());
    game.audio
        .play("minecraft:block.stone.break", Some(center), 1.0, 1.0);
    game.audio.play_on(
        6,
        "minecraft:block.note_block.pling",
        Some(center),
        1.0,
        1.0,
    );
    assert_eq!(
        game.audio
            .capture
            .as_ref()
            .unwrap()
            .iter()
            .map(|s| s.source)
            .collect::<Vec<_>>(),
        [4, 6]
    );
    game.audio.capture.as_mut().unwrap().clear();
    let (tag, _) = horizon_core::nbt::parse_snbt(r#"{id:"minecraft:ominous_item_spawner",UUID:[I;128,0,0,1],Pos:[6.25d,102.75d,4.875d],item:{id:"minecraft:diamond",count:1},spawn_item_after_ticks:40L}"#).unwrap();
    game.load_ominous_item_spawner(tag);
    assert_eq!(game.ominous_spawners.len(), 1);
    let s = &game.ominous_spawners[0];
    assert_eq!(s.uuid, [128, 0, 0, 1]);
    assert_eq!(s.position, [6.25, 102.75, 4.875]);
    assert_eq!(&*s.item.item, "minecraft:diamond");
    assert_eq!(s.item.count, 1);
    assert_eq!(s.spawn_item_after, 40);
    assert_eq!(s.tick_count, 0);
    for tick in 1..=5 {
        game.tick_ominous_item_spawners();
        assert_eq!(game.ominous_spawners[0].tick_count, tick);
        let warning: Vec<_> = game
            .audio
            .capture
            .as_ref()
            .unwrap()
            .iter()
            .filter(|s| s.event == WARNING)
            .collect();
        assert_eq!(warning.len(), if tick >= 4 { 1 } else { 0 });
        if tick == 4 {
            let sound = warning[0];
            assert_eq!(sound.position, center);
            assert_eq!(sound.volume.to_bits(), 1.0f32.to_bits());
            assert_eq!(sound.pitch.to_bits(), 1.0f32.to_bits());
            assert_eq!(
                sound.source, 6,
                "OminousItemSpawner.tickServer explicitly selects NEUTRAL"
            );
        }
    }
    game.audio
        .play("minecraft:block.stone.break", Some(center), 1.0, 1.0);
    assert_eq!(
        game.audio.capture.as_ref().unwrap().last().unwrap().source,
        4,
        "the explicit warning source must not change later event-family inference"
    );
    drop(game);
    std::fs::remove_dir_all(scratch).unwrap();
}

#[test]
#[ignore = "daily"]
fn residency_spawner_load_expiry_and_spawn_order() {
    crate::game::native_entity_refs::tests::with_game(|game| {
        game.ominous_spawners.clear();
        game.dropped.clear();
        for (item, delay) in [("diamond", 1), ("emerald", 3), ("gold_ingot", 1)] {
            let (tag, _) = horizon_core::nbt::parse_snbt(&format!(
                "{{Pos:[0d,80d,0d],item:{{id:\"minecraft:{item}\",count:1}},spawn_item_after_ticks:{delay}L}}"
            )).unwrap();
            game.load_ominous_item_spawner(tag);
        }
        let survivor = game.ominous_spawners[1].id;
        game.tick_ominous_item_spawners();
        assert_eq!(game.ominous_spawners.len(), 1);
        assert_eq!(game.ominous_spawners[0].id, survivor);
        assert_eq!(game.ominous_spawners[0].tick_count, 1);
        assert_eq!(
            game.dropped
                .iter()
                .map(|e| e.stack.item.as_ref())
                .collect::<Vec<_>>(),
            ["minecraft:diamond", "minecraft:gold_ingot"]
        );
        game.tick_ominous_item_spawners();
        game.tick_ominous_item_spawners();
        assert!(game.ominous_spawners.is_empty());
        assert_eq!(
            game.dropped.last().unwrap().stack.item.as_ref(),
            "minecraft:emerald"
        );
    });
}

#[test]
#[ignore = "daily"]
fn taken_spawner_tick_keeps_survivors_before_both_spawns() {
    use horizon_client::items::ItemStack;
    crate::game::native_entity_refs::tests::with_game(|game| {
        game.ominous_spawners.clear();
        for _ in 0..2 {
            game.add_ominous_item_spawner(
                ItemStack::new("minecraft:air", 1),
                [0.0, 80.0, 0.0],
                100,
            );
        }
        let first = game.ominous_spawners[0].id;
        let second = game.ominous_spawners[1].id;
        let next = game.next_entity;
        crate::game::entity::taken_pass_tests::with_spawn(
            |game| {
                assert!(game.ominous_spawners.is_empty());
                for delay in [200, 300] {
                    game.add_ominous_item_spawner(
                        ItemStack::new("minecraft:air", 1),
                        [0.0, 80.0, 0.0],
                        delay,
                    );
                }
            },
            || game.tick_ominous_item_spawners(),
        );
        assert_eq!(
            game.ominous_spawners
                .iter()
                .map(|s| (s.id, s.tick_count))
                .collect::<Vec<_>>(),
            [(first, 1), (second, 1), (next + 1, 0), (next + 2, 0)]
        );
    });
}

#[test]
#[ignore = "daily"]
fn lifecycle_spawner_commands_unload_park_and_remote_metadata() {
    use crate::game::{
        commands::Target,
        entity::taken_pass_tests::{LifecycleFixture, RemoteLifecycle},
    };
    use horizon_protocol::{
        connection::Pipe,
        entity_data::{Value, serializer},
        item::Stack,
    };
    LifecycleFixture {
        seed: |game, entries| {
            game.ominous_spawners.clear();
            for &(id, at) in entries {
                game.add_remote_ominous_item_spawner(id, at);
            }
        },
        ids: |game| game.ominous_spawners.iter().map(|s| s.id).collect(),
        target: Target::Spawner,
        unloads_with_chunk: true,
        remote: Some(RemoteLifecycle {
            prepare: |game| {
                let (client, _server) = Pipe::pair();
                game.connect_local(client, "Lifecycle").unwrap();
            },
            apply: |game, remove, update| {
                let item = game
                    .remote
                    .as_ref()
                    .unwrap()
                    .builtin
                    .get("minecraft:item")
                    .unwrap()
                    .id("minecraft:emerald")
                    .unwrap();
                game.remote_object_data(
                    update as i32,
                    &[(
                        8,
                        serializer::ITEM_STACK,
                        Value::Item(Some(Stack::new(item, 7))),
                    )],
                );
                let changed = game
                    .ominous_spawners
                    .iter()
                    .find(|s| s.id == update)
                    .unwrap();
                assert_eq!(changed.item.item.as_ref(), "minecraft:emerald");
                assert_eq!(changed.item.count, 7);
                game.remove_remote_object(remove);
            },
        }),
    }
    .run();
}

#[test]
#[ignore = "daily"]
fn residency_command_positions_and_client_spawner_clock() {
    crate::game::native_entity_refs::tests::with_game(|game| {
        game.set_allow_commands(true);
        game.add_remote_ominous_item_spawner(701, [1.0, 80.0, 1.0]);
        game.spawn_creeper_cloud([2.0, 80.0, 2.0], Vec::new(), Vec::new(), Vec::new(), 0);
        assert!(game.run_command("tp @e[type=minecraft:ominous_item_spawner] 64.0 80.0 64.0"));
        assert_eq!(game.ominous_spawners[0].position, [64.0, 80.0, 64.0]);
        assert!(game.run_command("tp @e[type=minecraft:area_effect_cloud] -32.0 90.0 -48.0"));
        assert_eq!(game.breath_clouds[0].at, [-32.0, 90.0, -48.0]);
        game.tick_remote_ominous_item_spawners();
        assert_eq!(game.ominous_spawners[0].tick_count, 1);
    });
}
