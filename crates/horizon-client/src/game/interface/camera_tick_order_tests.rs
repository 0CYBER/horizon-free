//! The camera ticks before the player it follows (#16178).
//!
//! Vanilla 26.3, `Minecraft.tick`: `gameRenderer.tick()` runs before
//! `level.tickEntities()`. `GameRenderer.tick` runs `mainCamera.tick()`, and
//! `Camera.tick` eases `eyeHeight` halfway toward `entity.getEyeHeight()` and
//! then `tickFov`, both from the pose the player's last tick left
//! (`Player.updatePlayerPose` ends `Player.tick`). A player who starts gliding
//! in tick N has the FALL_FLYING pose (eye 0.4) from the end of tick N: the
//! camera keeps 1.62 through tick N and starts down in tick N + 1,
//! 1.62 + (0.4 - 1.62) * 0.5 = 1.01. Prepared resources only.
use super::Game;
use crate::game::{
    Audio, Options, Progress, Settings, execution_role::ExecutionRole, load_resources,
};
use horizon_client::{chunk::Chunk, motion::PlayerMotion, pose::Pose};
use horizon_core::geometry::Vec3;

#[test]
#[ignore = "daily"]
fn the_camera_eases_toward_a_new_glide_pose_from_the_next_tick() {
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
        ExecutionRole::IntegratedServer,
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
        Audio::silent(&horizon_assets::resource_source::ResourceSource::directory(
            root.clone(),
        )),
        None,
    )
    .unwrap();
    let (client, _server) = horizon_protocol::connection::Pipe::pair();
    game.connect_local(client, "CameraTickTest").unwrap();
    game.remote.as_mut().unwrap().entity_id = 7;
    // Lit, empty columns around the player: LocalPlayer.tick runs, in the air.
    for x in -1..=1 {
        for z in -1..=1 {
            game.streaming
                .insert_remote(&mut game.world, Chunk::empty(x, z));
        }
    }
    // Standing, the camera settled at its eyes; the server has just started
    // the glide, so this tick's updatePlayerPose lies the player down.
    let mut player = PlayerMotion::new(game.motion, Vec3::new(0.5, 100.0, 0.5)).unwrap();
    assert_eq!(player.pose, Pose::Standing);
    player.fall_flying = true;
    player.server_owns_glide = true;
    player.velocity = Vec3::new(0.0, 0.0, 1.5);
    game.player = Some(player);
    game.view.eye_height = 1.62;
    game.view.eye_height_old = 1.62;

    game.tick().unwrap();
    assert_eq!(game.player.as_ref().unwrap().pose, Pose::FallFlying);
    // Camera.tick read the standing eyes, before the player ticked.
    assert_eq!(
        (game.view.eye_height_old, game.view.eye_height),
        (1.62, 1.62)
    );

    game.tick().unwrap();
    assert_eq!(game.view.eye_height_old, 1.62);
    assert!(
        (game.view.eye_height - 1.01).abs() < 1e-6,
        "halfway from 1.62 to the glide's 0.4: {}",
        game.view.eye_height
    );
}
