//! `AbstractNautilus.interact` keeps a clicked nautilus (`setPersistenceRequired`,
//! used or not), and `AbstractNautilus.mobInteract`'s fall through to
//! `Animal.mobInteract` gives a tame nautilus at full health the 600-tick love
//! and the species' eating sound, the zombie nautilus too: `ZombieNautilus`
//! overrides neither `isFood` nor `mobInteract`.
use super::super::*;
use horizon_client::chunk::Chunk;
use horizon_client::mobs::Mob;
use horizon_client::motion::PlayerMotion;
use horizon_core::{geometry::Vec3, position::ChunkPos};
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
    let mut m = Mob::new(id, kind, Vec3::new(at[0], at[1], at[2]), 0.0);
    m.health = m.attrs.max_health;
    game.mobs.push(m);
}

fn target(id: u32) -> Option<InteractTarget> {
    Some(InteractTarget {
        id: id as i32,
        location: [0.0, 0.0, 0.0],
    })
}

/// `AbstractNautilus.interact` runs `setPersistenceRequired` before
/// `Mob.interact`, so a click that does nothing (an empty hand, or something
/// a wild nautilus does not tame with) still keeps it.
#[test]
#[ignore = "daily"]
fn a_click_that_does_nothing_still_keeps_the_nautilus() {
    let mut game = game();
    for (id, item) in [(7, ""), (8, "minecraft:dirt")] {
        spawn(&mut game, "minecraft:nautilus", id, [2.5, 64.0, 2.5]);
        game.inventory[0] = if item.is_empty() {
            None
        } else {
            Some(ItemStack::new(item, 2))
        };
        assert!(!game.mobs[0].persistent, "{item:?} starts unkept");
        assert!(
            !game.interact_entity(target(id)).unwrap(),
            "{item:?}: the click is refused"
        );
        assert!(game.mobs[0].persistent, "{item:?}: the click keeps it");
        assert_eq!(game.mobs[0].extra.in_love, 0, "{item:?}: no love");
        assert_eq!(
            game.inventory[0].as_ref().map(|s| s.count),
            if item.is_empty() { None } else { Some(2) },
            "{item:?}: nothing was eaten"
        );
    }
}

/// `Animal.mobInteract` through `ZombieNautilus` (which overrides neither
/// `isFood` nor `mobInteract`): a tame zombie nautilus at full health fed a
/// fish eats it, with its own `playEatingSound`, and falls in love for 600
/// ticks.
#[test]
#[ignore = "daily"]
fn a_tame_zombie_nautilus_at_full_health_falls_in_love() {
    let mut game = game();
    for (id, kind, eat) in [
        (
            11,
            "minecraft:zombie_nautilus",
            "minecraft:entity.zombie_nautilus.eat",
        ),
        (12, "minecraft:nautilus", "minecraft:entity.nautilus.eat"),
    ] {
        spawn(&mut game, kind, id, [2.5, 64.0, 2.5]);
        game.mobs[0].extra.tame = true;
        game.mobs[0].extra.owner = Some([1, 2, 3, 4]);
        game.inventory[0] = Some(ItemStack::new("minecraft:cod", 2));
        game.audio.capture = Some(Vec::new());
        assert!(game.interact_entity(target(id)).unwrap(), "{kind}");
        assert_eq!(game.mobs[0].extra.in_love, 600, "{kind}");
        assert_eq!(game.mobs[0].extra.age, 0, "{kind}");
        assert_eq!(
            game.inventory[0].as_ref().map(|s| s.count),
            Some(1),
            "{kind}: the fish is eaten"
        );
        let sounds = game.audio.capture.take().unwrap();
        assert!(
            sounds.iter().any(|s| s.event == eat),
            "{kind}: it eats: {sounds:?}"
        );
    }
}
