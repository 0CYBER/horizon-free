//! `/particle` decodes its options through the particle type's codec before it
//! makes anything, and its success line names the type's registry key
//! (`ParticleArgument.readParticle`, `ParticleCommand.sendParticles`).
use crate::motion::PlayerMotion;
use horizon_client::particles::ParticleType;
use horizon_core::geometry::Vec3;

/// The owner standing at the origin of a server game, or `None` when this
/// machine has no prepared vanilla data.
fn server_game() -> Option<crate::game::Game<'static>> {
    let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES")?);
    // The mob tests' guard: without the prepared data there is nothing to run.
    if !root.join("entity-types.json").exists() {
        return None;
    }
    let mut game = crate::game::native_machines::tests::server_game();
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap());
    game.build_registries();
    game.set_allow_commands(true);
    Some(game)
}

/// Runs a command and returns whether it succeeded, what the player was told
/// and the particles the level was asked for.
fn run(
    game: &mut crate::game::Game<'_>,
    command: &str,
) -> (
    bool,
    Vec<String>,
    Vec<(
        horizon_client::particles::ParticleOptions,
        [f64; 3],
        [f64; 3],
    )>,
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
fn particle_options_are_decoded_before_a_particle_is_made() {
    let Some(mut game) = server_game() else {
        return;
    };
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
        let (ok, told, made) = run(&mut game, command);
        assert!(!ok, "{command}");
        assert_eq!(told, [message], "{command}");
        assert!(made.is_empty(), "{command} made a particle");
    }
}

#[test]
fn the_success_line_names_the_registry_key() {
    let Some(mut game) = server_game() else {
        return;
    };
    let (ok, told, made) = run(&mut game, "particle flame 0.5 65.0 0.5");
    assert!(ok);
    assert_eq!(told, ["Displaying particle minecraft:flame"]);
    assert_eq!(made.len(), 1);
    assert_eq!(made[0].0.ty, ParticleType::Flame);

    // A scale inside the codec's range reaches the particle as it was typed.
    let (ok, told, made) = run(
        &mut game,
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
        &mut game,
        "particle minecraft:item{item:\"minecraft:stone\"} 0.5 65.0 0.5",
    );
    assert!(ok);
    assert_eq!(told, ["Displaying particle minecraft:item"]);
    assert_eq!(made.len(), 1);
    assert_eq!(made[0].0.ty, ParticleType::Item);
}
