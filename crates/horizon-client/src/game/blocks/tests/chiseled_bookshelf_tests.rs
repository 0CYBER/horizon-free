//! Chiseled bookshelves: `ChiseledBookShelfBlockEntity.updateState` follows the
//! slot a container write names, and `ChiseledBookShelfBlock.removeBook` on a
//! slot the state calls occupied but that holds nothing.
use crate::game::machines::shelf_signal;
use crate::game::sculk::TEST_POSTED_EVENTS;
use crate::game::{Audio, Game, Hit, Options, Progress, Settings, load_resources};
use horizon_client::{
    chunk::Chunk,
    containers::{BlockEntity, ChiseledBookshelf},
    items::ItemStack,
    mobs::Who,
};
use horizon_core::geometry::Direction;
use horizon_core::position::BlockPos;
use std::sync::Arc;

const POS: [i32; 3] = [8, 64, 8];
const CENTRE: [f64; 3] = [8.5, 64.5, 8.5];

/// A windowless integrated-server game from the prepared resources with a
/// resident chunk; `None` without them.
fn with_game<R>(f: impl FnOnce(&mut Game<'static>) -> R) -> Option<R> {
    let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES")?);
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
    Some(f(&mut game))
}

/// The game events posted while `f` runs, as `(event, position, source)`.
fn record(f: impl FnOnce()) -> Vec<(&'static str, [f64; 3], Option<Who>)> {
    TEST_POSTED_EVENTS.with(|e| *e.borrow_mut() = Some(Vec::new()));
    f();
    TEST_POSTED_EVENTS.with(|e| e.borrow_mut().take().unwrap_or_default())
}

/// A chiseled bookshelf facing south at `POS`: the state's `occupied` slots
/// are full and the block entity holds a book in `books`. They may disagree,
/// as `/setblock` leaves them.
fn place_shelf(game: &mut Game<'_>, occupied: &[usize], books: &[usize]) {
    let registry = game.pack.assets.registry;
    let default = registry
        .default_state("minecraft:chiseled_bookshelf")
        .unwrap();
    let mut state = registry.state(default).unwrap().clone();
    state.properties.insert("facing".into(), "south".into());
    for slot in 0..6 {
        state.properties.insert(
            format!("slot_{slot}_occupied"),
            occupied.contains(&slot).to_string(),
        );
    }
    let id = registry.id(&state).unwrap();
    game.world.set(POS[0], POS[1], POS[2], id).unwrap();
    let mut shelf = ChiseledBookshelf::default();
    for &slot in books {
        shelf.items[slot] = Some(ItemStack::new("minecraft:book", 1));
    }
    game.world.block_entities.insert(
        (POS[0], POS[1], POS[2]),
        BlockEntity::ChiseledBookshelf(Box::new(shelf)),
    );
}

/// `ChiseledBookShelfBlock.getAnalogOutputSignal`: the last slot touched, plus
/// one, as a comparator beside the shelf reads it.
fn signal(game: &Game<'_>) -> i32 {
    let registry = game.pack.assets.registry;
    let state = game.world.block(POS[0], POS[1], POS[2]);
    let at = BlockPos::new(POS[0], POS[1], POS[2]);
    shelf_signal(&game.world, registry, state, at, Direction::South).expect("a chiseled bookshelf")
}

/// The slot `updateState` last stored.
fn last_slot(game: &Game<'_>) -> i32 {
    match game.world.block_entities.get(&(POS[0], POS[1], POS[2])) {
        Some(BlockEntity::ChiseledBookshelf(b)) => b.last_slot,
        other => panic!("no chiseled bookshelf: {other:?}"),
    }
}

/// `ChiseledBookShelfBlockEntity.setItem` -> `updateState`: a book written
/// over a book still makes its slot the last one touched, rewrites the state
/// and posts `BLOCK_CHANGE`, so the comparator follows the write.
#[test]
#[ignore = "daily"]
fn a_book_written_over_a_book_makes_its_slot_the_last_touched() {
    let Some((posted, slot, signal)) = with_game(|game| {
        place_shelf(game, &[2], &[2]);
        assert!(
            game.run_command("item replace block 8 64 8 container.0 with minecraft:book"),
            "slot 0 takes a book"
        );
        assert_eq!(last_slot(game), 0);
        assert_eq!(signal(game), 1);
        let posted = record(|| {
            assert!(
                game.run_command("item replace block 8 64 8 container.2 with minecraft:book"),
                "slot 2 takes a book over its own"
            );
        });
        (posted, last_slot(game), signal(game))
    }) else {
        return;
    };
    assert_eq!(posted, vec![("minecraft:block_change", CENTRE, None)]);
    assert_eq!(slot, 2, "the write names its slot, whatever the occupancy");
    assert_eq!(signal, 3, "the comparator follows the last slot touched");
}

/// `ChiseledBookShelfBlock.removeBook` with a slot the state calls occupied
/// but the block entity does not: no `removeItem`, so no `updateState`, but
/// the pickup sound, `BLOCK_CHANGE` from the player and SUCCESS still come.
#[test]
#[ignore = "daily"]
fn an_empty_slot_clicked_as_occupied_still_plays_the_pickup() {
    let Some((posted, sounds)) = with_game(|game| {
        place_shelf(game, &[2], &[]);
        // `SelectableSlotContainer.getHitSlot` on the front face, three columns
        // by two rows: column 2, row 0 is slot 2.
        let hit = Hit {
            pos: POS,
            face: [0, 0, 1],
            point: [8.8, 64.75, 9.0],
            inside: false,
        };
        game.audio.capture = Some(Vec::new());
        let posted = record(|| {
            assert!(
                game.use_hand(Some(&hit)).unwrap(),
                "removeBook answers SUCCESS"
            );
        });
        let sounds = game.audio.capture.take().unwrap_or_default();
        assert!(game.dropped.is_empty(), "an empty stack drops nothing");
        assert_eq!(last_slot(game), -1, "nothing came out of the slot");
        assert_eq!(signal(game), 0);
        (posted, sounds)
    }) else {
        return;
    };
    assert_eq!(sounds.len(), 1, "{sounds:?}");
    assert_eq!(sounds[0].event, "minecraft:block.chiseled_bookshelf.pickup");
    assert_eq!(sounds[0].pitch, 1.0);
    assert_eq!(
        posted,
        vec![("minecraft:block_change", CENTRE, Some(Who::Player))]
    );
}
