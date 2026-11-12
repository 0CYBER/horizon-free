//! Jukeboxes against `JukeboxBlock`, `JukeboxBlockEntity` and `JukeboxPlayable`:
//! what a load brings back of a saved song, the `BLOCK_CHANGE` of an insert and
//! of an eject, and which click the `has_record` state allows.
use super::Game;
use crate::game::{Audio, Options, Progress, Settings, load_resources};
use horizon_client::{
    chunk::Chunk,
    containers::{BlockEntity, Jukebox},
    items::ItemStack,
    mobs::Who,
};
use horizon_world::registry::BlockStateId;
use std::sync::Arc;

/// The game events posted while `f` runs, as `(event, position, source)`.
fn record(f: impl FnOnce()) -> Vec<(&'static str, [f64; 3], Option<Who>)> {
    crate::game::sculk::TEST_POSTED_EVENTS.with(|e| *e.borrow_mut() = Some(Vec::new()));
    f();
    crate::game::sculk::TEST_POSTED_EVENTS.with(|e| e.borrow_mut().take().unwrap_or_default())
}

/// A windowless integrated-server game from the prepared resources with a
/// resident chunk; `None` without them.
fn with_game<R>(f: impl FnOnce(&mut Game<'static>) -> R) -> Option<R> {
    let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES")?);
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
    .ok()?;
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.execution_role = crate::game::execution_role::ExecutionRole::IntegratedServer;
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
    game.world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    Some(f(&mut game))
}

const AT: [f64; 3] = [0.5, 64.5, 0.5];

/// The jukebox at (0, 64, 0) in the state `record` names; its state id.
fn place_jukebox(game: &mut Game<'_>, record: bool) -> BlockStateId {
    let registry = game.pack.assets.registry;
    let mut state = registry
        .state(registry.default_state("minecraft:jukebox").unwrap())
        .unwrap()
        .clone();
    state.properties.insert(
        "has_record".into(),
        if record { "true" } else { "false" }.into(),
    );
    let id = registry.id(&state).unwrap();
    game.world.set(0, 64, 0, id).unwrap();
    id
}

/// A jukebox block entity as a save writes it, loaded back into the world at
/// (0, 64, 0): `RecordItem` and, with a `clock`, `ticks_since_song_started`.
fn load_saved(game: &mut Game<'_>, clock: Option<i64>) {
    let clock = clock.map_or(String::new(), |n| format!("ticks_since_song_started:{n}L,"));
    let (tag, _) = horizon_world::nbt::parse_snbt(&format!(
        "{{id:\"minecraft:jukebox\",x:0,y:64,z:0,\
         RecordItem:{{id:\"minecraft:music_disc_11\",count:1,}},{clock}}}"
    ))
    .unwrap();
    let (pos, model) = horizon_client::save::decode_block_entity(&tag).unwrap();
    game.world.block_entities.insert(pos, model);
}

/// `JukeboxBlockEntity.setTheItem` posts `BLOCK_CHANGE` with the state the
/// block now has, and `JukeboxPlayable.tryInsertIntoJukebox` then posts the
/// same change again with the player as its source (against the state the
/// click found). `popOutTheItem` -> `setTheItem(EMPTY)` posts one on the way
/// out, before the song's own `JUKEBOX_STOP_PLAY`. A sculk sensor beside a
/// jukebox heard neither insert nor eject: only the song's end.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn an_insert_posts_two_block_changes_and_an_eject_one() {
    let Some((inserted, popped)) = with_game(|game| {
        place_jukebox(game, false);
        let disc = ItemStack::new("minecraft:music_disc_cat", 1);
        let inserted = record(|| {
            assert!(
                game.use_jukebox([0, 64, 0], Some(&disc)),
                "the disc goes in"
            );
        });
        let popped = record(|| {
            assert!(
                game.use_jukebox([0, 64, 0], None),
                "the empty hand pops it out"
            );
        });
        (inserted, popped)
    }) else {
        return;
    };
    assert_eq!(
        inserted,
        vec![
            ("minecraft:block_change", AT, None),
            ("minecraft:block_change", AT, Some(Who::Player)),
        ],
        "the slot's change, then the player's"
    );
    assert_eq!(
        popped,
        vec![
            ("minecraft:block_change", AT, None),
            ("minecraft:jukebox_stop_play", AT, None),
        ],
        "the record leaves the block before its song stops"
    );
}

/// `JukeboxBlock.useWithoutItem` only pops when the state says a record is
/// there, and `popOutTheItem` pops nothing from an empty slot; `useItemOn`
/// only inserts when it says there is none, and replacing the slot's item is
/// `setTheItem`'s own doing. Horizon decided both by the slot.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn the_has_record_state_decides_both_clicks() {
    let Some((ignored, replaced)) = with_game(|game| {
        // has_record with nothing in the slot: the click pops nothing and the
        // disc stays in hand, so the state stays as it is.
        let recorded = place_jukebox(game, true);
        game.world
            .block_entities
            .insert((0, 64, 0), BlockEntity::Jukebox(Jukebox::default()));
        let disc = ItemStack::new("minecraft:music_disc_cat", 1);
        let ignored = record(|| {
            assert!(
                game.use_jukebox([0, 64, 0], Some(&disc)),
                "the click is consumed"
            );
        });
        assert_eq!(game.world.block(0, 64, 0), recorded, "keeps its record");
        assert!(
            game.jukebox([0, 64, 0]).unwrap().record().is_none(),
            "the held disc is not inserted"
        );

        // No record in the state with a disc in the slot: the held one replaces
        // it (`JukeboxBlockEntity.setTheItem`), it does not eject the old one.
        place_jukebox(game, false);
        let old = ItemStack::new("minecraft:music_disc_13", 1);
        game.world.block_entities.insert(
            (0, 64, 0),
            BlockEntity::Jukebox(Jukebox {
                items: [Some(old)],
                ..Default::default()
            }),
        );
        let replaced = record(|| {
            assert!(
                game.use_jukebox([0, 64, 0], Some(&disc)),
                "the disc goes in"
            );
        });
        let j = game.jukebox([0, 64, 0]).unwrap();
        assert_eq!(&*j.record().unwrap().item, "minecraft:music_disc_cat");
        assert!(j.playing, "the new disc plays");
        (ignored, replaced)
    }) else {
        return;
    };
    assert!(ignored.is_empty(), "nothing happened: {ignored:?}");
    assert_eq!(
        replaced,
        vec![
            ("minecraft:block_change", AT, None),
            ("minecraft:block_change", AT, Some(Who::Player)),
        ],
        "the old disc is replaced, not popped out"
    );
}

/// `JukeboxBlockEntity.loadAdditional` hands a saved clock to
/// `JukeboxSongPlayer.setSongWithoutPlaying`: a song under its length comes
/// back playing, without `play`'s level event 1010 and its "Now Playing", a
/// clock past the song's end is dropped without a `JUKEBOX_STOP_PLAY`, and a
/// record saved without a clock is no song at all.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_saved_song_comes_back_without_its_sound() {
    let Some(cases) = with_game(|game| {
        place_jukebox(game, true);
        let signal = |game: &Game<'_>| {
            let p = horizon_core::position::BlockPos::new(0, 64, 0);
            super::jukebox_signals(&game.world, game.items(), p)
                .unwrap()
                .0
        };
        let mut cases = Vec::new();
        for clock in [Some(41), Some(10_000), None, Some(0)] {
            load_saved(game, clock);
            let events = record(|| game.tick_jukebox((0, 64, 0)));
            let j = game.jukebox([0, 64, 0]).unwrap();
            cases.push((
                clock,
                (j.playing, j.ticks, j.song_clock_present),
                signal(game),
                game.titles.overlay.is_some(),
                events,
            ));
        }
        cases
    }) else {
        return;
    };
    let at = |clock: Option<i64>| cases.iter().find(|c| c.0 == clock).unwrap();
    // Mid-song and at its start: the clock goes on, at 15, quietly; only the
    // twenty-tick chatter of a song that is running.
    assert_eq!(at(Some(41)).1, (true, 42, true), "the clock goes on");
    assert_eq!(at(Some(41)).2, 15);
    assert!(!at(Some(41)).3, "nothing says \"Now Playing\" on a load");
    assert!(at(Some(41)).4.is_empty(), "{:?}", at(Some(41)).4);
    assert_eq!(at(Some(0)).1, (true, 1, true));
    assert!(!at(Some(0)).3, "a clock saved at its start stays quiet");
    assert_eq!(
        at(Some(0)).4,
        vec![("minecraft:jukebox_play", AT, None)],
        "the first tick's game event is vanilla's"
    );
    // Past the end: gone, and silent about going.
    assert_eq!(at(Some(10_000)).1, (false, 0, false));
    assert_eq!(at(Some(10_000)).2, 0, "a dropped song powers nothing");
    assert!(at(Some(10_000)).4.is_empty(), "{:?}", at(Some(10_000)).4);
    // A record saved without a clock: no song, no signal, no event.
    assert_eq!(at(None).1, (false, 0, false));
    assert_eq!(at(None).2, 0);
    assert!(!at(None).3, "a record alone starts no song");
    assert!(at(None).4.is_empty(), "{:?}", at(None).4);
}

/// The same three cases on the load path itself, which needs no resources: only
/// a saved clock brings a song back, and it comes back unstarted.
#[test]
fn only_a_saved_clock_brings_a_song_back() {
    let load = |clock: Option<i64>| {
        let clock = clock.map_or(String::new(), |n| format!("ticks_since_song_started:{n}L,"));
        let (tag, _) = horizon_world::nbt::parse_snbt(&format!(
            "{{id:\"minecraft:jukebox\",x:0,y:64,z:0,\
             RecordItem:{{id:\"minecraft:music_disc_11\",count:1,}},{clock}}}"
        ))
        .unwrap();
        match horizon_client::save::decode_block_entity(&tag).unwrap().1 {
            BlockEntity::Jukebox(j) => j,
            other => panic!("not a jukebox: {other:?}"),
        }
    };
    let mid = load(Some(41));
    assert_eq!(mid.ticks, 41);
    assert!(
        mid.playing && mid.song_clock_present,
        "a saved clock brings the song back"
    );
    let unclocked = load(None);
    assert_eq!(
        unclocked.record().map(|r| &*r.item),
        Some("minecraft:music_disc_11")
    );
    assert_eq!(unclocked.ticks, 0);
    assert!(
        !unclocked.playing && !unclocked.song_clock_present,
        "a record alone is no song"
    );
}
