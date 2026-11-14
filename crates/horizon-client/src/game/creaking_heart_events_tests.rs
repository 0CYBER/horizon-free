//! `CreakingHeartBlockEntity`: the creaking a heart spawns posts
//! `ENTITY_PLACE` and entity event 60 (the poof cloud) at its position
//! (`spawnProtector`), each resin clump the hurt heart spreads posts
//! `BLOCK_PLACE` from the heart's own state (`creakingHurt`), and shearing a
//! copper golem posts `SHEAR` at the golem from the player
//! (`CopperGolem.mobInteract`).
use super::super::*;
use horizon_client::chunk::Chunk;
use horizon_client::mobs::{Species, Who};
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
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(4.5, 200.0, 4.5)).unwrap());
    game.build_registries();
    game.set_allow_commands(true);
    for x in -1..=1 {
        for z in -1..=1 {
            game.world
                .chunks
                .insert((x, z), Arc::new(Chunk::empty(x, z)));
            game.block_ticks
                .add_empty_chunk(ChunkPos::new(x, z))
                .unwrap();
        }
    }
    for command in [
        "difficulty easy",
        "forceload add -16 -16 16 16",
        // A floor under the whole spawn search of a heart at y 202, and night
        // for `CREAKING_ACTIVE`.
        "fill -16 199 -16 16 199 16 stone",
        "time set night",
        "tick freeze",
    ] {
        assert!(game.run_command(command), "{command}");
    }
    game
}

/// The game events posted while `f` runs, as `(event, position, source)`.
fn posted(
    game: &mut Game<'_>,
    f: impl FnOnce(&mut Game<'_>),
) -> Vec<(&'static str, [f64; 3], Option<Who>)> {
    crate::game::sculk::TEST_POSTED_EVENTS.with(|e| *e.borrow_mut() = Some(Vec::new()));
    f(game);
    crate::game::sculk::TEST_POSTED_EVENTS.with(|e| e.borrow_mut().take().unwrap_or_default())
}

/// A heart between pale oak logs, its ticker run until it spawns its creaking.
fn heart_with_a_creaking(game: &mut Game<'_>) -> (u32, [f64; 3]) {
    for command in [
        "setblock 0 201 0 minecraft:pale_oak_log[axis=y]",
        // The state a neighbour update leaves it in at night (awake), not the
        // default the block is placed as (uprooted, which never ticks).
        "setblock 0 202 0 minecraft:creaking_heart[axis=y,creaking_heart_state=awake]",
        "setblock 0 203 0 minecraft:pale_oak_log[axis=y]",
    ] {
        assert!(game.run_command(command), "{command}");
    }
    for _ in 0..300 {
        game.tick_creaking_heart((0, 202, 0)).unwrap();
        if let Some(m) = game.mobs.iter().find(|m| m.species() == Species::Creaking) {
            return (m.id, [m.position.x, m.position.y, m.position.z]);
        }
    }
    panic!("the heart spawned no creaking: {:?}", game.mobs.len());
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_heart_spawn_posts_entity_place_and_the_poof() {
    let mut game = game();
    game.audio.capture = Some(Vec::new());
    crate::game::sculk::TEST_POSTED_EVENTS.with(|e| *e.borrow_mut() = Some(Vec::new()));
    let (id, at) = heart_with_a_creaking(&mut game);
    let events =
        crate::game::sculk::TEST_POSTED_EVENTS.with(|e| e.borrow_mut().take().unwrap_or_default());
    let places: Vec<_> = events
        .iter()
        .filter(|(event, ..)| *event == "minecraft:entity_place")
        .collect();
    assert_eq!(
        places,
        vec![&("minecraft:entity_place", at, Some(Who::Mob(id)))],
        "the creaking's ENTITY_PLACE: {events:?}"
    );
    let poofs: Vec<_> = game
        .lan_entity_events
        .iter()
        .filter(|(mob, event, _)| *mob == id as i32 && *event == 60)
        .collect();
    assert_eq!(
        poofs,
        vec![&(id as i32, 60, at)],
        "{:?}",
        game.lan_entity_events
    );
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn each_resin_clump_posts_block_place() {
    let mut game = game();
    let (id, _) = heart_with_a_creaking(&mut game);
    let heart = game.world.block(0, 202, 0);
    assert_eq!(
        game.pack
            .assets
            .registry
            .state(heart)
            .and_then(|s| s.properties.get("creaking_heart_state").cloned()),
        Some("awake".into()),
        "night left the heart awake"
    );
    let events = posted(&mut game, |g| g.creaking_hurt([0, 202, 0], id, true));
    let clumps: Vec<_> = events
        .iter()
        .filter(|(event, ..)| *event == "minecraft:block_place")
        .collect();
    assert!(
        (2..=3).contains(&clumps.len()),
        "one BLOCK_PLACE per clump: {events:?}"
    );
    for (_, pos, source) in &clumps {
        assert_eq!(*source, None, "Context.of(state) has no source");
        let p = [
            pos[0].floor() as i32,
            pos[1].floor() as i32,
            pos[2].floor() as i32,
        ];
        let name = &*game
            .res
            .facts()
            .get(game.world.block(p[0], p[1], p[2]))
            .name;
        assert_eq!(name, "minecraft:resin_clump", "{p:?} holds {name}");
    }
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn shearing_a_copper_golem_posts_shear_at_the_golem() {
    let mut game = game();
    assert!(game.run_command(
        "summon minecraft:copper_golem 4.5 200 4.5 \
         {NoAI:1b,PersistenceRequired:1b,next_weather_age:-2L}"
    ));
    let index = game
        .mobs
        .iter()
        .position(|m| m.species() == Species::CopperGolem)
        .expect("the golem");
    // OfferFlowerGoal.stop: the poppy on the antenna.
    game.receive_gift(game.mobs[index].id, "minecraft:poppy");
    assert!(game.golem_shearable(index), "the antenna holds the poppy");
    let at = {
        let p = game.mobs[index].position;
        [p.x, p.y, p.z]
    };
    let events = posted(&mut game, |g| {
        assert!(g.interact_copper_golem(index, "minecraft:shears"));
    });
    let shears: Vec<_> = events
        .iter()
        .filter(|(event, ..)| *event == "minecraft:shear")
        .collect();
    assert_eq!(
        shears,
        vec![&("minecraft:shear", at, Some(Who::Player))],
        "the golem's SHEAR from the player: {events:?}"
    );
    assert!(
        game.mobs[index].saddle_item().is_none(),
        "the antenna is empty"
    );
}
