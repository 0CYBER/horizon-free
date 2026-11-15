//! The trail of a rocket boosting this client's own player (#16175).
//!
//! Vanilla 26.3, `Minecraft.tick`: `ClientLevel.tickEntities` ticks the
//! `LocalPlayer` (it glides on), then the rocket attached to it
//! (`FireworkRocketEntity.tick`: `setPos(attached + getHandHoldingItemAngle)`,
//! then `addParticle(FIREWORK)` into `ParticleEngine.particlesToAdd`). Later in
//! the same tick `ParticleEngine.tick` ticks the live particles and only then
//! adds that queue, without ticking the new ones. So the spark is drawn from the
//! frame right after the tick it spawns in, at the hand of the moved player.
//! Prepared resources only.
use super::{Game, Rocket};
use crate::game::{
    Audio, Options, Progress, Settings, execution_role::ExecutionRole, load_resources,
};
use horizon_client::{chunk::Chunk, motion::PlayerMotion, particles::ParticleType};
use horizon_core::geometry::Vec3;

const ME: i32 = 7;

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn an_own_rockets_spark_is_live_the_tick_it_spawns_at_the_moved_hand() {
    let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
    let settings = Settings::default();
    let loaded = load_resources(
        root.clone(),
        None,
        &root,
        &settings,
        std::time::Instant::now(),
        false,
        true,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.execution_role = ExecutionRole::Client;
    let mut game = Game::new(
        &loaded.resources,
        loaded.pack,
        loaded.gui.unwrap(),
        options,
        settings,
        Audio::silent(&root),
        None,
    )
    .unwrap();
    let (client, _server) = horizon_protocol::connection::Pipe::pair();
    game.connect_local(client, "SparkTest").unwrap();
    game.remote.as_mut().unwrap().entity_id = ME;
    // Lit, empty columns around the player: LocalPlayer.tick runs, in the air.
    for x in -1..=1 {
        for z in -1..=1 {
            game.streaming
                .insert_remote(&mut game.world, Chunk::empty(x, z));
        }
    }
    let start = Vec3::new(0.5, 100.0, 0.5);
    let mut player = PlayerMotion::new(game.motion, start).unwrap();
    player.fall_flying = true;
    player.server_owns_glide = true;
    player.velocity = Vec3::new(0.0, 0.0, 1.5);
    game.player = Some(player);
    (game.yaw, game.pitch) = (0.0, 0.0);
    let mut rocket = Rocket::remote(8, start, Vec3::ZERO);
    rocket.attached_to = Some(ME as u32);
    game.rockets.push(rocket);

    game.tick_remote().unwrap();

    let moved = game.player.as_ref().unwrap().position;
    assert!(
        moved.z - start.z > 1.0,
        "the player glides on during the tick: {moved:?}"
    );
    let sparks: Vec<_> = game
        .particles
        .live()
        .iter()
        .filter(|p| p.ty == ParticleType::Firework)
        .collect();
    assert_eq!(
        sparks.len(),
        1,
        "the spark joins the engine in the tick it spawns"
    );
    let spark = sparks[0];
    // Entity.getHandHoldingItemAngle for the right main arm at yRot 0:
    // calculateViewVector(0, 80).scale(0.5) = (-sin 80°, 0, cos 80°) / 2.
    let hand = [-0.492_403_876_5, 0.0, 0.086_824_088_8];
    let want = [moved.x + hand[0], moved.y + hand[1], moved.z + hand[2]];
    for axis in 0..3 {
        assert!(
            (spark.position[axis] - want[axis]).abs() < 1e-3,
            "spark at {:?}, the moved player's hand at {want:?}",
            spark.position
        );
    }
    // Added after the live ones ticked: not ticked yet (xo == x, age 0).
    assert_eq!(spark.previous, spark.position);
    assert_eq!(spark.age, 0);
}
