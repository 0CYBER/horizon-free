use horizon_client::motion::PlayerMotion;
use horizon_core::geometry::Vec3;

#[test]
#[ignore = "daily"]
fn guest_replay_synchronizes_worn_boots() {
    mod_platform::testing::with_mods(Default::default(), || {
        crate::game::native_guest_turn_tests::with_guests(|game| {
            let mut config = horizon_anticheat::Config::default();
            config.log.clear();
            config.strikes.clear();
            config.prediction.enabled = true;
            game.lan.as_mut().unwrap().hub.set_anticheat(config);
            assert!(game.lan.as_ref().unwrap().hub.recording());
            let body = PlayerMotion::new(game.motion, Vec3::new(3.0, 64.0, 5.0)).unwrap();
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(12, |guest| {
                    guest.motion = Some(Box::new(body));
                })
                .unwrap();
            for (item, count, expected) in [
                ("minecraft:leather_boots", 1, true),
                ("minecraft:iron_boots", 1, false),
                ("minecraft:leather_boots", 0, false),
            ] {
                game.lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .update_guest(12, |guest| {
                        guest.armor[3] = Some(crate::game::ItemStack::new(item, count));
                        guest.motion.as_deref_mut().unwrap().leather_boots = !expected;
                    })
                    .unwrap();
                game.guest_replay(12);
                let actual = game
                    .lan
                    .as_ref()
                    .unwrap()
                    .hub
                    .guest(12)
                    .unwrap()
                    .motion
                    .as_deref()
                    .unwrap()
                    .leather_boots;
                assert_eq!(actual, expected, "{item}, count {count}");
                println!("{item}, count {count}: motion.leather_boots={actual}");
            }
        });
    });
}

/// Feed moves exactly as lan::prediction's steps_in does: pong, moved, tick_end,
/// then replay. Both clients walk on top; only the server's equipment differs.
fn powder_snow_walk(boots: &str) -> Vec<f64> {
    let mut offsets = Vec::new();
    mod_platform::testing::with_mods(Default::default(), || {
        crate::game::native_guest_turn_tests::with_guests(|game| {
            let log_dir = tempfile::tempdir().unwrap();
            let log_path = log_dir.path().join("anticheat.log");
            let mut config = horizon_anticheat::Config {
                log: log_path.to_str().unwrap().to_owned(),
                ..Default::default()
            };
            config.strikes.clear();
            config.prediction.enabled = true;
            assert_eq!(config.prediction.tolerance, 0.03);
            game.lan.as_mut().unwrap().hub.set_anticheat(config);
            let root =
                std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
            let render = game
                .pack
                .assets
                .reload(&horizon_assets::resource_source::ResourceRoot::directory(
                    root,
                ))
                .unwrap();
            game.world =
                horizon_client::world::World::test_scene(&game.pack.assets, &render, 2).unwrap();
            let snow = game
                .pack
                .assets
                .registry
                .default_state("minecraft:powder_snow")
                .unwrap();
            let air = game
                .pack
                .assets
                .registry
                .default_state("minecraft:air")
                .unwrap();
            for x in 0..=7 {
                for z in 0..=15 {
                    game.world.set(x, 63, z, snow).unwrap();
                    for y in 64..=67 {
                        game.world.set(x, y, z, air).unwrap();
                    }
                }
            }
            let start = Vec3::new(3.5, 64.0, 5.5);
            let mut client = PlayerMotion::new(game.motion, start).unwrap();
            client.client_side = true;
            client.leather_boots = true;
            client.ground.on_ground = true;
            let mut server = PlayerMotion::new(game.motion, start).unwrap();
            server.ground.on_ground = true;
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(12, |guest| {
                    guest.loaded = true;
                    guest.load_timeout = 0;
                    guest.loading = false;
                    guest.position = [start.x, start.y, start.z];
                    guest.motion = Some(Box::new(server));
                    guest.armor[3] = Some(crate::game::ItemStack::new(boots, 1));
                    let mut record = horizon_client::lan::prediction::record::Record::default();
                    record.teleported(guest.position, false);
                    guest.record = Some(Box::new(record));
                    for x in -1..=1 {
                        for z in -1..=1 {
                            guest.sent_chunks.insert((x, z));
                        }
                    }
                })
                .unwrap();
            // The other fixture guest must not excuse this walk as an entity push.
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(13, |guest| guest.position = [1000.0; 3])
                .unwrap();
            game.world.block_log = None;
            game.world.block_log_held.clear();
            for tick in 0..20 {
                horizon_client::motion::MotionWorld {
                    entities: &[],
                    player: &mut client,
                    scene: &mut game.world,
                    assets: &game.pack.assets,
                    collision_binding: game.collision,
                    resources: game.motion,
                    health: &game.health_resources,
                    loop_context: &game.loop_context,
                    damage_hooks: &game.damage_hooks,
                    tag_items: Some(game.res.items()),
                    block_registries: Some(&game.pack.block_registries),
                }
                .controlled_simulation_tick(horizon_client::sprint::Keys {
                    forward: true,
                    ..Default::default()
                })
                .unwrap();
                assert!((client.position.y - 64.0).abs() < 1e-6);
                let p = client.position;
                game.lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .update_guest(12, |guest| {
                        let record = guest.record.as_deref_mut().unwrap();
                        record.pong(tick, tick);
                        record.moved(
                            Some([p.x, p.y, p.z]),
                            Some((0.0, 0.0)),
                            u8::from(client.ground.on_ground),
                        );
                        record.tick_end(1, client.sprinting);
                        guest.previous_position = guest.position;
                        guest.position = [p.x, p.y, p.z];
                    })
                    .unwrap();
                game.guest_replay(12);
                let guest = game.lan.as_ref().unwrap().hub.guest(12).unwrap();
                offsets.push(
                    guest
                        .replay
                        .as_deref()
                        .expect("active replay")
                        .ticks
                        .back()
                        .unwrap()
                        .offset,
                );
            }
            assert!(client.position.z > start.z + 1.0, "walked horizontally");
            drop(game.lan.as_mut().unwrap().hub.anticheat.take());
            let log = std::fs::read_to_string(log_path).unwrap_or_default();
            let failed = log.matches("failed prediction").count();
            println!("{boots}: failed prediction records={failed}");
            if boots == "minecraft:leather_boots" {
                assert_eq!(failed, 0, "{log}");
            }
        });
    });
    println!("{boots}: offsets={offsets:?}");
    offsets
}

#[test]
#[ignore = "daily"]
fn honest_powder_snow_prediction() {
    let offsets = powder_snow_walk("minecraft:leather_boots");
    assert_eq!(offsets.len(), 20);
    assert!(offsets.iter().all(|&offset| offset <= 0.03), "{offsets:?}");
}

#[test]
#[ignore = "daily"]
fn lying_powder_snow_prediction() {
    let offsets = powder_snow_walk("minecraft:iron_boots");
    assert_eq!(offsets.len(), 20);
    assert!(
        offsets.iter().skip(1).all(|&offset| offset > 0.03),
        "{offsets:?}"
    );
}
