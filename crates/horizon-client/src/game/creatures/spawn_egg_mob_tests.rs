//! A spawn egg used on a mob of its own kind: `Mob.checkAndHandleImportantInteractions`
//! answers `SUCCESS_SERVER` for it, so `Mob.interact` posts `ENTITY_INTERACT` with the
//! player as its source, at the mob. The offspring is `getBreedOffspring(level, this)`,
//! so a mooshroom's 1-in-1024 colour flip is rolled with the parent as its own mate.
use crate::game::{
    Audio, Game, InteractTarget, Options, Progress, Screen, Settings, execution_role,
    load_resources,
};
use horizon_client::chunk::Chunk;
use horizon_client::items::ItemStack;
use horizon_client::mobs::{Mob, Species, Who};
use horizon_client::motion::PlayerMotion;
use horizon_core::geometry::Vec3;
use horizon_core::position::ChunkPos;
use horizon_core::random::{LegacyRandom, RandomSource};
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
        .add_empty_chunk(ChunkPos::new(0, 0))
        .unwrap();
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap());
    game.screen = Screen::Playing;
    game.selected = 0;
    game.set_mode_id(0);
    game
}

fn spawn(game: &mut Game<'_>, kind: &str, id: u32, at: [f64; 3]) {
    game.mobs.clear();
    let kind = game.res.mob_types().get(kind).unwrap().clone();
    game.mobs
        .push(Mob::new(id, kind, Vec3::new(at[0], at[1], at[2]), 0.0));
}

fn target(id: u32) -> Option<InteractTarget> {
    Some(InteractTarget {
        id: id as i32,
        location: [0.0, 1.0, 0.0],
    })
}

fn watch() {
    crate::game::sculk::TEST_POSTED_EVENTS.with(|events| *events.borrow_mut() = Some(Vec::new()));
}

fn posted() -> Vec<(&'static str, [f64; 3], Option<Who>)> {
    crate::game::sculk::TEST_POSTED_EVENTS
        .with(|events| std::mem::take(events.borrow_mut().as_mut().unwrap()))
}

#[test]
#[ignore = "daily"]
fn an_egg_used_on_a_mob_posts_entity_interact() {
    let mut game = game();
    watch();
    spawn(&mut game, "minecraft:cow", 7, [2.5, 64.0, 2.5]);
    game.inventory[0] = Some(ItemStack::new("minecraft:cow_spawn_egg", 1));
    assert!(game.interact_mob(target(7)).unwrap());
    // `Mob.interact`: `this.gameEvent(GameEvent.ENTITY_INTERACT, player)` at
    // `this.position`.
    assert_eq!(
        posted(),
        vec![(
            "minecraft:entity_interact",
            [2.5, 64.0, 2.5],
            Some(Who::Player)
        )]
    );
    let babies = game
        .mobs
        .iter()
        .filter(|m| m.species() == Species::Cow && m.is_baby())
        .count();
    assert_eq!(babies, 1, "the egg's baby");
    // A click that consumes nothing (a stick on a cow) posts nothing.
    spawn(&mut game, "minecraft:cow", 8, [2.5, 64.0, 2.5]);
    game.inventory[0] = Some(ItemStack::new("minecraft:stick", 1));
    assert!(!game.interact_mob(target(8)).unwrap());
    assert!(posted().is_empty());
    crate::game::sculk::TEST_POSTED_EVENTS.with(|events| *events.borrow_mut() = None);
}

#[test]
#[ignore = "daily"]
fn a_mooshroom_egg_child_rolls_the_colour_flip() {
    let mut game = game();
    // `MushroomCow.getBreedOffspring(level, this)` on the egg path: the mate is
    // the parent, so `nextInt(1024) == 0` always changes the colour, and every
    // other roll spends the coin flip that can only pick the same colour.
    let flip = (0..)
        .find(|&s| LegacyRandom::new(s).next_int_bound(1024).unwrap() == 0)
        .unwrap();
    let keep = (0..)
        .find(|&s| LegacyRandom::new(s).next_int_bound(1024).unwrap() != 0)
        .unwrap();
    for (seed, parent) in [(flip, 0), (flip, 1), (keep, 0), (keep, 1)] {
        spawn(&mut game, "minecraft:mooshroom", 9, [2.5, 64.0, 2.5]);
        game.mobs[0].extra.variant = parent;
        game.mobs[0].random = horizon_core::random::EntityRandom::own(LegacyRandom::new(seed));
        game.inventory[0] = Some(ItemStack::new("minecraft:mooshroom_spawn_egg", 1));
        assert!(game.interact_mob(target(9)).unwrap());
        let child = game
            .mobs
            .iter()
            .find(|m| m.id != 9)
            .expect("the egg's baby");
        assert_eq!(child.species(), Species::Mooshroom);
        assert!(child.is_baby(), "seed {seed}");
        let flipped = seed == flip;
        let expected = if flipped { 1 - parent } else { parent };
        assert_eq!(
            child.extra.variant, expected,
            "seed {seed}, parent colour {parent}"
        );
        // The parent's own random rolled it: `nextInt(1024)`, then the coin
        // flip when the colour stayed.
        let mut rolled = LegacyRandom::new(seed);
        let _ = rolled.next_int_bound(1024).unwrap();
        if !flipped {
            let _ = rolled.next_boolean();
        }
        assert_eq!(
            game.mobs[0].random.state(),
            rolled.state(),
            "seed {seed}: the draws"
        );
    }
}
