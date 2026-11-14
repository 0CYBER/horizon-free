//! `LevelEventHandler` 1039 (`LevelEvent.SOUND_PHANTOM_BITE`): the sound a
//! phantom's bite posts.
use super::*;
use crate::game::*;
use horizon_core::random::{LegacyRandom, RandomSource};

fn with_game(test: impl FnOnce(&mut Game<'_>)) {
    let root = PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
    let settings = Settings::default();
    let loaded = load_resources(
        root.clone(),
        None,
        &root,
        &settings,
        Instant::now(),
        false,
        true,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.execution_role = execution_role::ExecutionRole::IntegratedServer;
    let mut g = Game::new(
        &loaded.resources,
        loaded.pack,
        loaded.gui.unwrap(),
        options,
        settings,
        Audio::silent(&root),
        None,
    )
    .unwrap();
    g.options.dedicated = Some(Vec::new());
    g.world = World::new(g.res.facts().air, 1);
    // The guests' copy of a level event is recorded only while capturing.
    g.audio.capture = Some(Vec::new());
    test(&mut g);
}

/// Level event 1039 is `playLocalSound(pos, SoundEvents.PHANTOM_BITE,
/// SoundSource.HOSTILE, 0.3F, random.nextFloat() * 0.1F + 0.9F, false)` with
/// `random` the client level's, at the block's centre.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_phantom_bite_level_event_plays_the_bite_at_0_3_volume() {
    with_game(|g| {
        // The client level's random, seeded so the pitch has an expected value.
        *g.particles.level_random() = LegacyRandom::new(7);
        let pitch = LegacyRandom::new(7).next_float() * 0.1 + 0.9;
        g.audio.engine.loaded = true;
        g.level_event_at(1039, [0, 64, 0], 0);
        let bite = (0..=8u64)
            .filter_map(|id| g.audio.engine.instance(id))
            .find(|i| i.identifier == "minecraft:entity.phantom.bite")
            .expect("level event 1039 plays the phantom's bite");
        assert_eq!(bite.volume, 0.3);
        assert_eq!(bite.pitch, pitch);
        assert_eq!(bite.position, [0.5, 64.5, 0.5]);
        assert_eq!(
            horizon_client::settings::SOUND_SOURCES[bite.source],
            "hostile"
        );
        // It goes to the guests as the level event itself, which they play.
        assert_eq!(
            g.lan_events,
            vec![(1039, [0, 64, 0], 0, None)],
            "the bite reaches the guests as level event 1039"
        );
    });
}
