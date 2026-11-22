//! `/particle` hands each type's options to the client draw: `ParticleArgument`
//! parses the SNBT and `ParticleTypes.CODEC` (the type's own options codec)
//! turns it into what `ParticleProvider.createParticle` reads — a block
//! crumble's block state, a dust transition's colours and scale, a spell's
//! colour and power, a sculk charge's roll, a vibration's destination.
//!
//! The options are also decoded before anything is made, and the success line names the
//! type's registry key (`ParticleArgument.readParticle`, `ParticleCommand.sendParticles`).
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
#[ignore = "daily"]
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
#[ignore = "daily"]
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

/// The owner standing at the origin of a server game with the commands allowed.
fn with_owner_game(test: impl FnOnce(&mut Game<'_>)) {
    crate::game::native_entity_refs::tests::with_game(|game| {
        game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap());
        game.build_registries();
        game.set_allow_commands(true);
        test(game);
    });
}

/// Runs a command and returns whether it succeeded, what the player was told
/// and the particles the level was asked for.
fn run(
    game: &mut Game<'_>,
    command: &str,
) -> (
    bool,
    Vec<String>,
    Vec<(ParticleOptions, [f64; 3], [f64; 3])>,
) {
    game.particles.created_requests = Some(Vec::new());
    game.chat_capture = Some(Vec::new());
    let ok = game.run_command(command);
    let told = game
        .chat_capture
        .take()
        .unwrap()
        .iter()
        .map(|t| t.plain())
        .collect();
    (ok, told, game.particles.created_requests.take().unwrap())
}

#[test]
#[ignore = "daily"]
fn particle_options_are_decoded_before_a_particle_is_made() {
    with_owner_game(|game| {
        for (command, message) in [
            (
                "particle minecraft:dust{color:0,scale:10} 0.5 65.0 0.5",
                "Can't parse particle options: Value must be within range [0.01;4.0]: 10.0",
            ),
            (
                "particle minecraft:dust{color:\"red\",scale:1} 0.5 65.0 0.5",
                "Can't parse particle options: Failed to parse either. First: Not a number; Second: Not a list: \"red\"",
            ),
            (
                "particle minecraft:item{item:\"minecraft:not_an_item\"} 0.5 65.0 0.5",
                "Can't parse particle options: Failed to parse either. First: Not a map: \"minecraft:not_an_item\"; Second: Unknown registry key in ResourceKey[minecraft:root / minecraft:item]: minecraft:not_an_item",
            ),
            (
                "particle minecraft:item{item:{}} 0.5 65.0 0.5",
                "Can't parse particle options: Failed to parse either. First: No key id in MapLike[{}]; Second: Not a string",
            ),
        ] {
            let (ok, told, made) = run(game, command);
            assert!(!ok, "{command}");
            assert_eq!(told, [message], "{command}");
            assert!(made.is_empty(), "{command} made a particle");
        }
    });
}

#[test]
#[ignore = "daily"]
fn the_success_line_names_the_registry_key() {
    with_owner_game(|game| {
        let (ok, told, made) = run(game, "particle flame 0.5 65.0 0.5");
        assert!(ok);
        assert_eq!(told, ["Displaying particle minecraft:flame"]);
        assert_eq!(made.len(), 1);
        assert_eq!(made[0].0.ty, ParticleType::Flame);

        // A scale inside the codec's range reaches the particle as it was typed.
        let (ok, told, made) = run(
            game,
            "particle minecraft:dust{color:0,scale:4.0} 0.5 65.0 0.5",
        );
        assert!(ok);
        assert_eq!(told, ["Displaying particle minecraft:dust"]);
        assert_eq!(made.len(), 1);
        assert_eq!(made[0].0.ty, ParticleType::Dust);
        assert_eq!(made[0].0.scale, 4.0);

        // An item id resolves through the registries (`Item.CODEC`), and the
        // particle it names is made.
        let (ok, told, made) = run(
            game,
            "particle minecraft:item{item:\"minecraft:stone\"} 0.5 65.0 0.5",
        );
        assert!(ok);
        assert_eq!(told, ["Displaying particle minecraft:item"]);
        assert_eq!(made.len(), 1);
        assert_eq!(made[0].0.ty, ParticleType::Item);
    });
}
