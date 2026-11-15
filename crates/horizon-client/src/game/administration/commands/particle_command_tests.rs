//! `/particle` hands each type's options to the client draw: `ParticleArgument`
//! parses the SNBT and `ParticleTypes.CODEC` (the type's own options codec)
//! turns it into what `ParticleProvider.createParticle` reads — a block
//! crumble's block state, a dust transition's colours and scale, a spell's
//! colour and power, a sculk charge's roll, a vibration's destination.
use super::*;
use crate::game::{
    execution_role,
    launch_options::Options,
    resource_load::{Progress, load_resources},
};
use horizon_client::{
    audio::Audio,
    chunk::Chunk,
    motion::PlayerMotion,
    particles::{ParticleLevel, ParticleOptions, ParticleType},
    settings::Settings,
};
use horizon_core::geometry::Vec3;
use mod_api::client::particles::PositionSource;
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
    game.build_registries();
    game.set_allow_commands(true);
    game.world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    game.block_ticks
        .add_empty_chunk(horizon_core::position::ChunkPos::new(0, 0))
        .unwrap();
    assert!(game.run_command("fill 0 99 0 15 99 15 stone"));
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 100.0, 0.5)).unwrap());
    game
}

/// The one particle the window asks the engine for with `command`, at the
/// player's own block and with no spread.
fn drawn(game: &mut Game<'_>, command: &str) -> ParticleOptions {
    game.particles.created_requests = Some(Vec::new());
    assert!(game.run_command(command), "{command}");
    let mut made = game.particles.created_requests.take().unwrap();
    assert_eq!(made.len(), 1, "{command}");
    made.remove(0).0
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn particle_options_carry_their_codec_data() {
    let mut game = game();
    // BlockParticleOption: `block_crumble` is the crumbs of the named block.
    let crumble = drawn(
        &mut game,
        "particle minecraft:block_crumble{block_state:\"minecraft:stone\"} 0.5 100.5 0.5 0 0 0 0 1",
    );
    assert_eq!(crumble.ty, ParticleType::BlockCrumble);
    let (sprite, _) = crumble.block.expect("the crumbs carry their block");
    assert_eq!(sprite, "block/stone");
    // DustColorTransitionOptions: `from_color`, `to_color` and `scale`.
    let transition = drawn(
        &mut game,
        "particle minecraft:dust_color_transition{from_color:0,to_color:0,scale:2} 0.5 100.5 0.5 0 0 0 0 1",
    );
    assert_eq!(transition.ty, ParticleType::DustColorTransition);
    assert_eq!(transition.color[..3], [0.0; 3]);
    assert_eq!(transition.to_color, [0.0; 3]);
    assert_eq!(transition.scale, 2.0);
    // SpellParticleOption: its colour and power.
    let effect = drawn(
        &mut game,
        "particle minecraft:effect{color:16711680,power:2} 0.5 100.5 0.5 0 0 0 0 1",
    );
    assert_eq!(effect.ty, ParticleType::Effect);
    assert_eq!(effect.color[..3], [1.0, 0.0, 0.0]);
    assert_eq!(effect.scale, 2.0);
    // SculkChargeParticleOptions.roll.
    let charge = drawn(
        &mut game,
        "particle minecraft:sculk_charge{roll:0.5} 0.5 100.5 0.5 0 0 0 0 1",
    );
    assert_eq!(charge.ty, ParticleType::SculkCharge);
    assert_eq!(charge.scale, 0.5);
    // VibrationParticleOption: its block destination and arrival ticks.
    let vibration = drawn(
        &mut game,
        "particle minecraft:vibration{destination:{type:\"minecraft:block\",pos:[0,64,0]},arrival_in_ticks:20} 0.5 100.5 0.5 0 0 0 0 1",
    );
    assert_eq!(vibration.ty, ParticleType::Vibration);
    assert_eq!(
        vibration.vibration,
        Some((PositionSource::Block([0, 64, 0]), [0.5, 64.5, 0.5], 20))
    );
}

/// The crumbs the command names are drawn: `TerrainParticle.CrumblingProvider`
/// keeps no particle for options that carry no block.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn block_crumble_draws_the_block_it_names() {
    let mut game = game();
    game.particles.created_requests = None;
    game.particles.clear();
    assert!(game.run_command(
        "particle minecraft:block_crumble{block_state:\"minecraft:stone\"} 0.5 100.5 0.5 0 0 0 0 1"
    ));
    let level = ParticleLevel {
        world: &game.world,
        facts: game.res.facts(),
        collision: game.collision,
        player: None,
        cached_chunk: std::cell::Cell::new(None),
    };
    game.particles.tick(&level);
    assert_eq!(game.particles.len(), 1, "the crumble is drawn");
}
