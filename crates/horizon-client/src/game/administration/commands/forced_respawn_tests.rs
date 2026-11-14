#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn overworld_spawnpoint_is_validated_at_both_player_blocks() {
    use crate::game::{Audio, Game, Options, Progress, Settings, load_resources};
    use horizon_client::chunk::Chunk;
    use horizon_core::geometry::Vec3;
    use std::sync::Arc;
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
    game.player = Some(
        horizon_client::motion::PlayerMotion::new(game.motion, Vec3::new(5.5, 64.0, 5.5)).unwrap(),
    );
    game.build_registries();
    game.set_allow_commands(true);
    game.world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    game.level.world_spawn = Some([0, 80, 0]);
    let registry = game.pack.assets.registry;
    let air = registry.default_state("minecraft:air").unwrap();
    // Liquid is a block property: aquatic plants such as kelp still permit
    // a forced point, while a bubble column does not, at feet or head.
    for (block, allowed) in [
        ("air", true),
        ("water", false),
        ("lava", false),
        ("bubble_column", false),
        ("kelp", true),
        ("stone", false),
    ] {
        let state = registry
            .default_state(&format!("minecraft:{block}"))
            .unwrap();
        for height in [0, 1] {
            game.world.set(5, 64, 5, air).unwrap();
            game.world.set(5, 65, 5, air).unwrap();
            game.world.set(5, 64 + height, 5, state).unwrap();
            assert!(game.run_command("spawnpoint @s 5 64 5"));
            let config = game
                .respawn_config
                .expect("Overworld commands must retain a forced configuration");
            assert!(config.forced);
            assert_eq!(config.pos, [5, 64, 5]);
            let actual = game.resolve_server_respawn(config, true);
            assert_eq!(actual.is_some(), allowed, "{block} at height {height}");
            assert_eq!(game.respawn_config.is_some(), allowed);
            assert_eq!(game.level.world_spawn, Some([0, 80, 0]));
            if let Some((at, yaw, pitch)) = actual {
                assert_eq!(at, Vec3::new(5.5, 64.1, 5.5));
                assert_eq!(yaw, 0.0);
                assert_eq!(pitch.to_bits(), 0.0f32.to_bits());
            }
        }
    }
}

/// A server's game on the prepared resources, or None when they are absent
/// (CI has none: the test then skips quietly).
fn server(
    resources: &mut Option<std::sync::Arc<crate::game::Resources>>,
) -> Option<crate::game::Game<'_>> {
    std::env::var_os("HORIZON_TEST_RESOURCES")?;
    Some(crate::game::game_tests::tests::server_game(resources))
}

#[test]
fn a_forced_spawnpoint_on_a_respawn_anchor_still_depletes_with_its_sound() {
    // PlayerList.respawn: the deplete sound has no `forced` test, so a point
    // that `/spawnpoint` kept (a forced anchor config) whooshes on every
    // respawn at it, while the anchor keeps its charges
    // (ServerPlayer.findRespawnAndUseSpawnBlock only consumes one when the
    // config is not forced).
    use crate::game::{Dim, respawn_anchor::RespawnConfig};
    let mut resources = None;
    let Some(mut game) = server(&mut resources) else {
        return;
    };
    let stone = game
        .pack
        .assets
        .registry
        .default_state("minecraft:stone")
        .unwrap();
    for x in 0..16 {
        for z in 0..16 {
            game.world.set(x, 99, z, stone).unwrap();
        }
    }
    game.dim = Dim::Nether;
    let mut anchor = game
        .pack
        .assets
        .registry
        .state(
            game.pack
                .assets
                .registry
                .default_state("minecraft:respawn_anchor")
                .unwrap(),
        )
        .unwrap()
        .clone();
    anchor.properties.insert("charges".into(), "2".into());
    let anchor = game.pack.assets.registry.resolve(&anchor).unwrap();
    let pos = [5, 100, 5];
    game.world.set(pos[0], pos[1], pos[2], anchor).unwrap();
    game.audio.capture = Some(Vec::new());
    let config = RespawnConfig::forced(Dim::Nether, pos, 0.0, 0.0);
    game.respawn_config = Some(config);
    game.resolve_server_respawn(config, true)
        .expect("a forced anchor keeps the point");
    // The whoosh is the anchor's own block position, at full volume and pitch.
    let played = game.audio.capture.take().expect("capture");
    let sound = played
        .iter()
        .find(|s| s.event == "minecraft:block.respawn_anchor.deplete")
        .expect("the deplete sound plays for a forced anchor too");
    assert_eq!(sound.position, [5.0, 100.0, 5.0]);
    assert_eq!((sound.volume, sound.pitch), (1.0, 1.0));
    assert_eq!(
        game.audio
            .heard
            .get("minecraft:block.respawn_anchor.deplete"),
        Some(&1)
    );
    assert_eq!(
        game.world.block(pos[0], pos[1], pos[2]),
        anchor,
        "a forced config does not use a charge"
    );
    assert_eq!(game.respawn_config, Some(config));
}
