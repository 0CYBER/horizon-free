//! `Entity.baseTick`'s `spawnSprintParticle` on a window: the crumb of the
//! block under a runner's feet, drawn from the runner's own random, for the
//! window's own player (never while its abilities say it is flying,
//! `LocalPlayer.canSpawnSprintParticle`) and for every other client entity
//! whose synced sprint flag is set — a dashing cat or ocelot as much as
//! another player, but never a spectator (`!isSpectator()`).
//! `Rabbit.canSpawnSprintParticle` is false whatever its flag.
use super::*;
use crate::game::{Audio, Options, Progress, Settings, execution_role, load_resources};
use horizon_client::chunk::Chunk;
use horizon_client::mobs::Mob;
use horizon_client::motion::PlayerMotion;
use horizon_core::geometry::Vec3;
use horizon_core::random::RandomSource;
use std::sync::Arc;

/// A window's game on the prepared resources (None when they are absent: CI
/// has none and the tests then skip quietly) with a stone floor at y = 99
/// under the whole origin chunk and the camera over it, so a crumb of that
/// floor's dust reaches the particle engine.
fn game() -> Option<Game<'static>> {
    let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES")?);
    let settings = Settings::default();
    let loaded = load_resources(
        horizon_assets::resource_source::ResourceRoot::directory(root.clone()),
        Vec::new(),
        None,
        &root,
        &settings,
        std::time::Instant::now(),
        false,
        execution_role::ExecutionRole::Client,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.execution_role = execution_role::ExecutionRole::Client;
    let res: &'static _ = Box::leak(Box::new(loaded.resources));
    let mut game = Game::new(
        res,
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
    game.world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
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
    game.particles.camera = [8.5, 100.5, 8.5];
    Some(game)
}

/// A client copy of `species` whose feet are at `x`, 100, `z` and that moved
/// 0.2 south this tick (its position less its previous one, the movement the
/// crumb is thrown back against).
fn runner(game: &mut Game<'_>, id: u32, species: &str, x: f64, z: f64) -> usize {
    let kind = game.res.mob_types()[species].clone();
    let feet = Vec3::new(x, 100.0, z);
    let mut m = Mob::new(id, kind, feet, 0.0);
    m.previous = Vec3::new(feet.x, feet.y, feet.z - 0.2);
    game.mobs.push(m);
    game.mobs.len() - 1
}

/// A client copy of another player whose feet are at `x`, 100, `z` and that
/// moved 0.2 south this tick.
fn player_runner(game: &mut Game<'_>, id: u32, x: f64, z: f64) -> usize {
    let kind = game.res.player_type().clone().expect("the player model");
    let feet = Vec3::new(x, 100.0, z);
    let mut m = Mob::new(id, kind, feet, 0.0);
    m.previous = Vec3::new(feet.x, feet.y, feet.z - 0.2);
    game.mobs.push(m);
    game.mobs.len() - 1
}

/// `Entity.spawnSprintParticle`: one crumb of the block under `feet`, offset
/// by the first two `nextDouble`s of `random` across the runner's box and
/// thrown back against its movement.
fn crumb<R: RandomSource>(
    feet: Vec3,
    movement: Vec3,
    width: f64,
    random: &mut R,
) -> ([f64; 3], [f64; 3]) {
    let (rx, rz) = (random.next_double(), random.next_double());
    (
        [
            feet.x + (rx - 0.5) * width,
            feet.y + 0.1,
            feet.z + (rz - 0.5) * width,
        ],
        [movement.x * -4.0, 1.5, movement.z * -4.0],
    )
}

/// The crumbs the window asked the particle engine for, and the stone they
/// are made of.
fn crumbs(game: &mut Game<'_>) -> Vec<([f64; 3], [f64; 3])> {
    let made = game.particles.created_requests.take().unwrap_or_default();
    for (options, _, _) in &made {
        assert_eq!(options.ty, ParticleType::Block);
        assert_eq!(options.source.as_deref(), Some("minecraft:stone"));
    }
    made.into_iter()
        .map(|(_, at, velocity)| (at, velocity))
        .collect()
}

/// `LocalPlayer.canSpawnSprintParticle` is `!getAbilities().flying &&
/// super.canSpawnSprintParticle()`: a creative player skimming a block with
/// the fly toggle on trails nothing, and the same player walking on it
/// leaves its crumb, from its own `Entity.random`.
#[test]
fn a_flying_player_leaves_no_sprint_dust() {
    let Some(mut game) = game() else { return };
    let mut player = PlayerMotion::new(game.motion, Vec3::new(0.5, 100.0, 0.5)).unwrap();
    player.sprinting = true;
    player.flying = true;
    player.velocity = Vec3::new(0.0, 0.0, 0.2);
    game.player = Some(player);
    game.particles.created_requests = Some(Vec::new());
    game.tick_client_ambient();
    assert!(
        crumbs(&mut game).is_empty(),
        "a player whose abilities say it is flying leaves no dust"
    );
    // The same player, feet on the floor and no longer flying.
    game.player.as_mut().unwrap().flying = false;
    let expected = {
        let p = game.player.as_ref().unwrap();
        crumb(
            p.position,
            p.velocity,
            f64::from(0.6f32),
            &mut p.random.clone(),
        )
    };
    game.particles.created_requests = Some(Vec::new());
    game.tick_client_ambient();
    let made = crumbs(&mut game);
    assert_eq!(made.len(), 1, "the walking player leaves one crumb");
    assert_eq!(made[0], expected, "the player's crumb from its own random");
}

/// `Entity.baseTick` draws the crumb for every client entity whose synced
/// sprint flag is set, not only for players: a sprinting cat, ocelot and the
/// window's player each leave one, from their own random; a sprinting rabbit
/// leaves none, and neither does a cat that is not sprinting nor a spectator.
#[test]
fn every_sprinting_client_entity_leaves_a_crumb() {
    let Some(mut game) = game() else { return };
    let cat = runner(&mut game, 13001, "minecraft:cat", 2.5, 2.5);
    game.mobs[cat].extra.sprinting = true;
    let ocelot = runner(&mut game, 13002, "minecraft:ocelot", 4.5, 4.5);
    game.mobs[ocelot].extra.sprinting = true;
    // Rabbit.canSpawnSprintParticle is false, whatever its flag says.
    let rabbit = runner(&mut game, 13003, "minecraft:rabbit", 6.5, 6.5);
    game.mobs[rabbit].extra.sprinting = true;
    // A cat that walks: its flag is off, so nothing is drawn for it.
    runner(&mut game, 13004, "minecraft:cat", 8.5, 8.5);
    // Entity.canSpawnSprintParticle's !isSpectator(): a spectator's flag is
    // synced too, and draws nothing.
    let spectator = player_runner(&mut game, 13005, 10.5, 10.5);
    game.mobs[spectator].extra.sprinting = true;
    game.mobs[spectator].extra.spectator = true;
    let mut player = PlayerMotion::new(game.motion, Vec3::new(0.5, 100.0, 0.5)).unwrap();
    player.sprinting = true;
    player.velocity = Vec3::new(0.0, 0.0, 0.2);
    game.player = Some(player);
    // The crumbs, from the first two doubles of each runner's own random.
    let mut expected = Vec::new();
    let p = game.player.as_ref().unwrap();
    expected.push(crumb(
        p.position,
        p.velocity,
        f64::from(0.6f32),
        &mut p.random.clone(),
    ));
    for &i in &[cat, ocelot] {
        let m = &game.mobs[i];
        expected.push(crumb(
            m.position,
            m.position.subtract(m.previous),
            f64::from(m.width()),
            &mut m.random.clone(),
        ));
    }
    game.particles.created_requests = Some(Vec::new());
    game.tick_client_ambient();
    let made = crumbs(&mut game);
    assert_eq!(
        made.len(),
        3,
        "the player's, the cat's and the ocelot's: not the rabbit's, not the walking cat's, not the spectator's"
    );
    assert_eq!(made[0], expected[0], "the player's crumb");
    assert_eq!(made[1], expected[1], "the cat's crumb");
    assert_eq!(made[2], expected[2], "the ocelot's crumb");
}
