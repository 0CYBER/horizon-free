//! `/attribute <mob> minecraft:friction_modifier …` reaches the mob's physics:
//! `LivingEntity.travelInAir` scales the supporting block's friction by
//! `getAttributeValue(Attributes.FRICTION_MODIFIER)` every tick, so a cow whose
//! modifier is 2 slides a third as far on stone as one whose is the type's 1.
use super::super::*;
use horizon_client::chunk::Chunk;
use horizon_core::position::ChunkPos;
use std::sync::Arc;

fn game() -> Game<'static> {
    let root = std::path::PathBuf::from(
        std::env::var_os("HORIZON_TEST_RESOURCES")
            .expect("set HORIZON_TEST_RESOURCES to prepared resources"),
    );
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
    options.execution_role = execution_role::ExecutionRole::IntegratedServer;
    let res: &'static _ = Box::leak(Box::new(loaded.resources));
    let mut game = Game::new(
        res,
        loaded.pack,
        loaded.gui.unwrap(),
        options,
        settings,
        Audio::silent(&root),
        None,
    )
    .unwrap();
    game.build_registries();
    game.set_allow_commands(true);
    game.world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    game.block_ticks
        .add_empty_chunk(ChunkPos::new(0, 0))
        .unwrap();
    game.load_ticking_neighbourhood(0, 0);
    for command in [
        "forceload add 0 0",
        "fill 0 99 0 15 99 15 stone",
        "tick freeze",
        // A cow that cannot walk, so only its slide moves it.
        "summon cow 8.5 100 8.5 {PersistenceRequired:1b,Silent:1b,\
         attributes:[{id:\"minecraft:movement_speed\",base:0.0d}],UUID:[I;24725,0,0,3]}",
    ] {
        assert!(game.run_command(command), "{command}");
    }
    game
}

/// Whether the cow is on the ground, and how fast it slides sideways.
fn cow_state(game: &Game<'_>) -> (bool, f64) {
    let cow = game
        .mobs
        .iter()
        .find(|m| m.species() == horizon_client::mobs::Species::Cow)
        .expect("the cow");
    (cow.on_ground, cow.velocity.x)
}

/// The cow's horizontal speed after one tick on stone, `setup` run first.
fn slide(setup: &[&str]) -> f64 {
    let mut game = game();
    // `Entity.move` grounds it on the second tick: the first moves it nowhere,
    // so its motion is only the gravity the travel adds after the move.
    for _ in 0..2 {
        assert!(game.run_command("tick step 1"));
        game.tick_server(false).unwrap();
    }
    assert!(cow_state(&game).0, "the cow stands on the stone");
    for command in setup {
        assert!(game.run_command(command), "{command}");
    }
    assert!(game.run_command("data merge entity @e[type=cow,limit=1] {Motion:[0.5d,0.0d,0.0d]}"));
    assert!(game.run_command("tick step 1"));
    game.tick_server(false).unwrap();
    cow_state(&game).1
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn the_friction_modifier_attribute_slides_a_cow_less() {
    // Stone's 0.6 untouched: 0.5 times 0.6 and the 0.91 air drag.
    let plain = slide(&[]);
    assert!((plain - 0.273).abs() < 1.0e-6, "{plain}");
    // `computeModifiedFriction(0.6F, 2.0F)` is 0.2, a third of the plain slide.
    let held = slide(&["attribute @e[type=cow,limit=1] minecraft:friction_modifier base set 2.0"]);
    assert!((held - 0.091).abs() < 1.0e-6, "{held}");
}
