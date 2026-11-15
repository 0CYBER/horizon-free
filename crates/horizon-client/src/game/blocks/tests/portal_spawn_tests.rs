//! `NetherPortalBlock.randomTick`'s spawn, the rest of it after the won roll:
//! `isValidSpawn` refuses the block under the column when vanilla's table says
//! so (glowstone's 15 light, bedrock's `never`), and `EntityType.spawn` plays
//! the new piglin's ambient sound once it is in the level, before the spawn
//! sets its portal cooldown.
use super::super::*;
use horizon_client::chunk::Chunk;
use horizon_client::motion::PlayerMotion;
use horizon_core::geometry::Vec3;
use horizon_core::position::ChunkPos;
use std::sync::Arc;

fn game() -> Game<'static> {
    let root = std::path::PathBuf::from(
        std::env::var_os("HORIZON_TEST_RESOURCES")
            .expect("set HORIZON_TEST_RESOURCES to prepared resources"),
    );
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
    options.execution_role = execution_role::ExecutionRole::IntegratedServer;
    let res: &'static _ = Box::leak(Box::new(loaded.resources));
    let mut game = Game::new(
        res,
        loaded.pack,
        loaded.gui.unwrap(),
        options,
        settings,
        Audio::silent(&horizon_assets::resource_source::ResourceSource::directory(
            root.to_path_buf(),
        )),
        None,
    )
    .unwrap();
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap());
    game.build_registries();
    game.set_allow_commands(true);
    game.world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    game.block_ticks
        .add_empty_chunk(ChunkPos::new(0, 0))
        .unwrap();
    game
}

/// A portal block at (0, 64, 0) standing on `floor`.
fn portal_on(game: &mut Game<'_>, floor: &str) {
    assert!(game.run_command(&format!("setblock 0 63 0 minecraft:{floor}")));
    assert!(game.run_command("setblock 0 64 0 minecraft:nether_portal"));
    let state = game.world.block(0, 64, 0);
    assert_eq!(
        &*game.res.facts().get(state).name,
        "minecraft:nether_portal",
        "the portal block stands"
    );
}

fn spawned(game: &Game<'_>) -> Vec<String> {
    game.mobs.iter().map(|m| m.kind.id.clone()).collect()
}

/// `BlockState.isValidSpawn` is "sturdy top face and `getLightEmission() < 14`"
/// plus the blocks that override it: a portal on glowstone (15 light) or on
/// bedrock (`isValidSpawn(never)`) spawns nothing.
#[test]
#[ignore = "daily"]
fn a_portal_over_glowstone_or_bedrock_spawns_no_piglin() {
    let mut game = game();
    for floor in ["glowstone", "bedrock"] {
        portal_on(&mut game, floor);
        game.portal_spawn([0, 64, 0]);
        assert_eq!(
            spawned(&game),
            Vec::<String>::new(),
            "a portal over {floor} spawns no piglin"
        );
    }
}

/// On obsidian the piglin spawns, grunts (`EntityType.spawn`'s
/// `Mob.playAmbientSound`: its ambient sound at its feet, volume 1.0, at its
/// voice pitch 1.0 ± 0.2) and waits out its 300-tick portal cooldown.
#[test]
#[ignore = "daily"]
fn a_portal_over_obsidian_spawns_a_grunting_piglin() {
    let mut game = game();
    portal_on(&mut game, "obsidian");
    game.audio.capture = Some(Vec::new());
    game.portal_spawn([0, 64, 0]);
    assert_eq!(
        spawned(&game),
        vec!["minecraft:zombified_piglin".to_owned()],
        "obsidian floor"
    );
    let id = game.mobs[0].id;
    // setPortalCooldown: Entity.getDimensionChangingDelay.
    assert_eq!(
        game.portals.entities.get(&id).map(|p| p.cooldown),
        Some(300),
        "the new piglin's portal cooldown"
    );
    let sounds = game.audio.capture.take().unwrap_or_default();
    assert_eq!(sounds.len(), 1, "{sounds:?}");
    assert_eq!(sounds[0].event, "minecraft:entity.zombified_piglin.ambient");
    assert_eq!(sounds[0].position, [0.5, 64.0, 0.5], "at the piglin's feet");
    assert_eq!(sounds[0].volume, 1.0);
    // LivingEntity.getVoicePitch of an adult: 1.0 ± 0.2. (Zombie.finalizeSpawn's
    // 5 % baby roll is not modelled yet, so a portal piglin is always grown.)
    assert!(
        (0.8..=1.2).contains(&sounds[0].pitch),
        "voice pitch {}",
        sounds[0].pitch
    );
}
