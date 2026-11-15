//! Part state (arismoko/horizon#781) on real block entities: one value each, kept with its block entity
//! through ticks, transactions, replacement, saving and unloading.
use std::{panic::AssertUnwindSafe, sync::Arc};

use horizon_client::{
    chunk::Chunk,
    containers::{BlockEntity, ModBlockEntity},
};
use horizon_core::nbt::{Compound, Tag};
use horizon_mod_runtime::attachments::{AttachmentCodec, AttachmentError, Attachments, Holder};
use mod_api::{
    AttachmentType, BlockEntityPart, BlockEntityPartState, BlockEntityRef, BlockEntityTickEvent,
    BlockPart, Hooks, HostTransaction, PartStateError, Registrar, ServerHost, edit_part_state,
    read_state,
};

use super::{Dim, Game};

static PROGRESS: AttachmentType<i32> = AttachmentType {
    namespace: "part_state",
    name: "progress",
    default: || 0,
    codec: Some(AttachmentCodec {
        encode: |n| Ok([("n".into(), Tag::Int(*n))].into()),
        decode: |saved: &Compound| match saved.get(&"n".into()) {
            Some(Tag::Int(n)) => Ok(*n),
            _ => Err(AttachmentError::Decode),
        },
    }),
    copy_on_death: false,
    sync: false,
};

/// Kept for the session only: no codec.
static SCRATCH: AttachmentType<i32> = AttachmentType {
    namespace: "part_state",
    name: "scratch",
    default: || 0,
    codec: None,
    copy_on_death: false,
    sync: false,
};

/// Counts its block entity's ticks in its state.
struct Press;
#[mod_api::horizon_part]
impl BlockPart for Press {}
#[mod_api::horizon_part]
impl BlockEntityPart for Press {
    fn block_entity_tick(&self, host: &mut dyn ServerHost, event: &mut BlockEntityTickEvent<'_>) {
        let level = host.level(host.dimension()).unwrap();
        assert_eq!(
            host.block_entity(level, event.pos),
            None,
            "out for its tick"
        );
        let entity = host.block_entity_identity(level, event.pos).unwrap();
        Press::edit_state(host, entity, |_, ticks| *ticks += 1).unwrap();
    }
}
impl BlockEntityPartState for Press {
    type Data = i32;
    const DATA: &'static AttachmentType<i32> = &PROGRESS;
}

struct StateMod;
#[mod_api::__subscriptions]
impl Hooks for StateMod {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        let kind = r
            .block_entity_type("press")
            .blocks(&["part_state:press"])
            .build();
        r.block_entity_part_with_state(&kind, Press).unwrap();
    }
    fn blocks(&self, _: &mod_api::RegistrationContext, e: &mut mod_api::BlocksEvent<'_>) {
        e.blocks
            .push(mod_api::Block::new("part_state:press", "minecraft:barrel").with_item(false));
    }
}

/// Run the test `name` with `StateMod` installed, in a fresh process: its content root
/// freezes the process's block entity types, which another test may already have frozen.
fn with_state_mod(name: &str, test: impl FnOnce(&mut Game<'_>)) {
    if super::native_entity_refs::tests::run_fresh_cases(
        name,
        "HORIZON_PART_STATE_GAME_CHILD",
        &["valid"],
    ) {
        return;
    }
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new("part_state", Box::new(StateMod))].into();
    mod_platform::testing::with_mods(mods.clone(), || {
        crate::game::api_values::install_parts(&mods).unwrap();
        mod_api::__private::mark_game_thread();
        let base = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
        let root = super::mods::content_root(&base, &Default::default()).unwrap();
        super::native_entity_refs::tests::with_game_at(root.prepared().to_owned(), |game| {
            game.options.dedicated = Some(Vec::new());
            game.native_level_opened(Dim::Overworld);
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            test(game);
        });
    });
}

/// A fresh press at `pos` in the level the game is in, and its handle.
fn press(game: &mut Game<'_>, pos: (i32, i32, i32)) -> BlockEntityRef {
    let block = game
        .pack
        .assets
        .registry
        .default_state("part_state:press")
        .unwrap();
    game.world.set(pos.0, pos.1, pos.2, block).unwrap();
    let entity = ModBlockEntity::new("part_state:press".into(), Default::default());
    game.world
        .block_entities
        .insert(pos, BlockEntity::Mod(Box::new(entity)));
    let level = game.level(game.dim.id()).unwrap();
    game.block_entity(level, [pos.0, pos.1, pos.2]).unwrap()
}

fn stored(game: &mut Game<'_>, entity: BlockEntityRef) -> Option<i32> {
    Press::state_existing(game, entity).unwrap()
}

#[test]
#[ignore = "daily"]
fn part_state_is_per_block_entity_through_ticks_transactions_and_replacement() {
    const NAME: &str = "game::block_entity_part_state_tests::part_state_is_per_block_entity_through_ticks_transactions_and_replacement";
    with_state_mod(NAME, |game| {
        let (a, b) = (press(game, (1, 64, 1)), press(game, (2, 64, 1)));
        // Reads insert nothing.
        assert_eq!(Press::state(game, a), Ok(0));
        assert_eq!(stored(game, a), None);
        assert_eq!(
            PROGRESS.has(game, Holder::BlockEntity([1, 64, 1])),
            Ok(false)
        );
        assert_eq!(Press::edit_state(game, a, |_, n| *n = 7), Ok(()));
        assert_eq!((stored(game, a), stored(game, b)), (Some(7), None));

        // Its own tick reaches it while it is out of the world, under the same handle.
        game.tick_mod_block_entity((1, 64, 1));
        game.tick_mod_block_entity((1, 64, 1));
        assert_eq!(stored(game, a), Some(9));
        assert_eq!(Press::state(game, b), Ok(0));

        // An inner edit commits into the outer transaction, which then aborts: the value and
        // the chunk's unsaved mark go back, so a chunk that was clean is clean again.
        game.world.modified.clear();
        let aborted: Result<(), ()> = (game as &mut dyn ServerHost).transaction(|host, _| {
            assert_eq!(Press::edit_state(host, a, |_, n| *n = 11), Ok(()));
            assert_eq!(Press::state(host, a), Ok(11), "later steps see it");
            assert!(
                !horizon_client::join_seq::visibility::passes_settled(),
                "no save while it is tentative"
            );
            Err(())
        });
        assert_eq!(aborted, Err(()));
        assert_eq!(stored(game, a), Some(9));
        assert!(
            !game.world.modified.contains(&(0, 0)),
            "clean after the abort"
        );

        // A panic inside an edit leaves no default behind.
        let host: &mut dyn ServerHost = game;
        let panicked = std::panic::catch_unwind(AssertUnwindSafe(|| {
            Press::edit_state(host, b, |_, n| {
                *n = 5;
                panic!("edit fails");
            })
        }));
        assert!(panicked.is_err());
        assert!(game.transactions.is_empty());
        assert_eq!(stored(game, b), None);
        assert_eq!(Press::edit_state(game, b, |_, n| *n = 8), Ok(()));
        assert_eq!(stored(game, b), Some(8));

        // The same state can't be edited from inside its own edit; another key can.
        let inner = Press::edit_state(game, a, |host, n| {
            *n = 10;
            (
                Press::edit_state(host, a, |_, n| *n = 99),
                edit_part_state(host, a, &SCRATCH, |_, n| *n = 1),
            )
        });
        assert_eq!(inner, Ok((Err(PartStateError::Reentrant), Ok(()))));
        assert_eq!(stored(game, a), Some(10));
        assert_eq!(read_state(game, a, &SCRATCH), Ok(Some(1)));

        // A value replaced while the closure ran refuses the edit and keeps the old one.
        let changed = Press::edit_state(game, a, |host, n| {
            *n = 12;
            PROGRESS
                .set(host.attachments(), Holder::BlockEntity([1, 64, 1]), 50)
                .unwrap();
        });
        assert_eq!(changed, Err(PartStateError::Changed));
        assert_eq!(stored(game, a), Some(10));

        // An undecodable saved value refuses before the closure runs and keeps its bytes.
        let mut bad = Compound::new();
        bad.insert("n".into(), Tag::String("ten".into()));
        let mut saved = Attachments::default();
        PROGRESS.set(&mut saved, Holder::Level, 0).unwrap();
        let mut raw = saved.to_nbt();
        let Some(Tag::Compound(names)) = raw.get_mut(&"part_state".into()) else {
            unreachable!()
        };
        let Some(Tag::Compound(entry)) = names.get_mut(&"progress".into()) else {
            unreachable!()
        };
        entry.insert("value".into(), Tag::Compound(bad));
        game.world
            .block_entities
            .restore_attachments((2, 64, 1), Attachments::from_nbt(raw.clone()));
        let decode = PartStateError::Attachment {
            namespace: "part_state",
            name: "progress",
            error: AttachmentError::Decode,
        };
        assert_eq!(Press::state(game, b), Err(decode));
        let mut ran = false;
        game.world.modified.clear();
        assert_eq!(Press::edit_state(game, b, |_, _| ran = true), Err(decode));
        assert!(!ran);
        assert!(
            !game.world.modified.contains(&(0, 0)),
            "a refused edit marks nothing"
        );
        assert_eq!(
            game.world
                .block_entities
                .attachments((2, 64, 1))
                .unwrap()
                .to_nbt(),
            raw
        );

        // A block entity of the same place in another level has its own value, and a
        // handle answers only in its own level.
        game.enter_level(Dim::Nether).unwrap();
        assert_eq!(Press::state(game, a), Err(PartStateError::OtherLevel));
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        let nether = press(game, (1, 64, 1));
        assert_eq!(Press::state(game, nether), Ok(0));
        assert_eq!(Press::edit_state(game, nether, |_, n| *n = 3), Ok(()));
        game.enter_level(Dim::Overworld).unwrap();
        assert_eq!(stored(game, a), Some(10));

        // A replacement at the same place is a new block entity with no value.
        game.world.block_entities.remove(&(1, 64, 1));
        let replacement = press(game, (1, 64, 1));
        assert_ne!(replacement, a);
        assert_eq!(Press::state(game, a), Err(PartStateError::Stale));
        assert_eq!(
            Press::edit_state(game, a, |_, n| *n = 1),
            Err(PartStateError::Stale)
        );
        assert_eq!(stored(game, replacement), None);
    });
}

#[test]
#[ignore = "daily"]
fn part_state_saves_with_its_chunk_and_reloads_under_a_fresh_handle() {
    const NAME: &str = "game::block_entity_part_state_tests::part_state_saves_with_its_chunk_and_reloads_under_a_fresh_handle";
    with_state_mod(NAME, |game| {
        // A column far from the player, kept by a forced ticket while it is edited.
        let key = (80, 80);
        let pos = (1283, 100, 1283);
        // The test game starts with streaming paused, which never unloads.
        game.streaming.paused = false;
        assert!(game.streaming.set_forced(key.0, key.1, true));
        game.world
            .chunks
            .insert(key, Arc::new(Chunk::empty(key.0, key.1)));
        game.update_world();
        let entity = press(game, pos);
        game.world.modified.clear();
        assert_eq!(Press::edit_state(game, entity, |_, n| *n = 7), Ok(()));
        assert!(
            game.world.modified.contains(&key),
            "an edit marks its chunk for the unload's save"
        );
        assert_eq!(
            edit_part_state(game, entity, &SCRATCH, |_, n| *n = 4),
            Ok(())
        );

        // The ticket goes and the residency owner (`update_world`) unloads the column: it
        // saves it, then drops its block entities and their live bindings. Its sweep runs every
        // eighth update.
        assert!(game.streaming.set_forced(key.0, key.1, false));
        for _ in 0..9 {
            game.update_world();
        }
        assert!(!game.world.chunks.contains_key(&key), "unloaded");
        assert!(game.world.block_entities.attachments(pos).is_none());
        assert_eq!(Press::state(game, entity), Err(PartStateError::Stale));

        // Loaded back from the region file it was saved to, as streaming installs a column.
        game.save_level(true).unwrap();
        let mut chunk = game
            .save
            .load(key.0, key.1, game.pack.assets.registry, world_biomes!(game))
            .unwrap();
        game.world.install_chunk_block_entities(&mut chunk);
        game.world.chunks.insert(key, Arc::new(chunk));
        let level = game.level(game.dim.id()).unwrap();
        let reloaded = game.block_entity(level, [pos.0, pos.1, pos.2]).unwrap();
        assert_ne!(reloaded, entity);
        assert_eq!(Press::state(game, entity), Err(PartStateError::Stale));
        assert_eq!(stored(game, reloaded), Some(7));
        assert_eq!(
            read_state(game, reloaded, &SCRATCH),
            Ok(None),
            "session only"
        );
    });
}

/// What the attachment index answers for `entity`'s `PROGRESS`, without the handle check.
fn indexed(game: &Game<'_>, entity: BlockEntityRef) -> Option<i32> {
    use mod_api::AttachmentQueries;
    let host: &dyn mod_api::Host = game;
    host.query(&PROGRESS)
        .unwrap()
        .get(mod_api::AttachmentHolder::BlockEntity(entity))
        .ok()
        .flatten()
}

#[test]
#[ignore = "daily"]
fn a_removal_retires_its_state_only_at_the_root_commit() {
    const NAME: &str =
        "game::block_entity_part_state_tests::a_removal_retires_its_state_only_at_the_root_commit";
    with_state_mod(NAME, |game| {
        let pos = (1, 64, 1);
        let a = press(game, pos);
        assert_eq!(Press::edit_state(game, a, |_, n| *n = 7), Ok(()));

        // Removed and replaced inside a transaction that then aborts: the handle is stale
        // while it is open, but the index keeps the map bound; the abort brings the very
        // binding back, and the tentative block entity's handle stays stale.
        let seal = game.transaction_open();
        game.journal_remove_block_entity(pos);
        assert_eq!(Press::state(game, a), Err(PartStateError::Stale));
        assert_eq!(
            indexed(game, a),
            Some(7),
            "still bound until the root commits"
        );
        let tentative = press(game, pos);
        assert_eq!(Press::edit_state(game, tentative, |_, n| *n = 5), Ok(()));
        game.transaction_close(&seal, false);
        assert_eq!(Press::state(game, a), Ok(7));
        assert_eq!(indexed(game, a), Some(7));
        assert_eq!(Press::state(game, tentative), Err(PartStateError::Stale));
        assert_eq!(indexed(game, tentative), None);

        // A nested removal committed into its parent is still not published.
        let outer = game.transaction_open();
        let inner = game.transaction_open();
        game.journal_remove_block_entity(pos);
        game.transaction_close(&inner, true);
        assert_eq!(indexed(game, a), Some(7));
        // The root commit retires it.
        game.transaction_close(&outer, true);
        assert_eq!(Press::state(game, a), Err(PartStateError::Stale));
        assert_eq!(indexed(game, a), None);
        assert!(game.world.block_entities.attachments(pos).is_none());
    });
}

#[test]
#[ignore = "daily"]
fn a_save_or_unload_during_a_transaction_writes_only_committed_state() {
    const NAME: &str = "game::block_entity_part_state_tests::a_save_or_unload_during_a_transaction_writes_only_committed_state";
    with_state_mod(NAME, |game| {
        let (key, pos) = ((80, 80), (1283, 100, 1283));
        game.streaming.paused = false;
        assert!(game.streaming.set_forced(key.0, key.1, true));
        game.world
            .chunks
            .insert(key, Arc::new(Chunk::empty(key.0, key.1)));
        game.update_world();
        let entity = press(game, pos);
        assert_eq!(Press::edit_state(game, entity, |_, n| *n = 7), Ok(()));
        let reload = |game: &mut Game<'_>| {
            let chunk = game
                .save
                .load(key.0, key.1, game.pack.assets.registry, world_biomes!(game))
                .unwrap();
            let saved = chunk.saved.as_ref().unwrap();
            let (_, map) = saved
                .block_entity_attachments
                .iter()
                .find(|(at, _)| *at == pos)
                .unwrap();
            PROGRESS.get_existing(&mut map.clone(), Holder::Level)
        };

        // A save asked for while an edit is tentative is refused; once it aborts, the save
        // writes the committed value.
        let seal = game.transaction_open();
        assert_eq!(Press::edit_state(game, entity, |_, n| *n = 9), Ok(()));
        assert!(game.save_level(true).is_err(), "refused while tentative");
        game.transaction_close(&seal, false);
        game.save_level(true).unwrap();
        assert_eq!(reload(game), Ok(Some(7)));

        // An unload due while a transaction is open waits for it to close, then saves the
        // committed value.
        let seal = game.transaction_open();
        assert_eq!(Press::edit_state(game, entity, |_, n| *n = 11), Ok(()));
        assert!(game.streaming.set_forced(key.0, key.1, false));
        for _ in 0..9 {
            game.update_world();
        }
        assert!(game.world.chunks.contains_key(&key), "kept while tentative");
        assert_eq!(Press::state(game, entity), Ok(11));
        game.transaction_close(&seal, false);
        game.update_world();
        assert!(!game.world.chunks.contains_key(&key), "unloaded");
        game.save_level(true).unwrap();
        assert_eq!(reload(game), Ok(Some(7)));
    });
}

#[test]
#[ignore = "daily"]
fn one_position_replaced_twice_in_an_aborted_transaction_gets_its_original_back() {
    const NAME: &str = "game::block_entity_part_state_tests::one_position_replaced_twice_in_an_aborted_transaction_gets_its_original_back";
    with_state_mod(NAME, |game| {
        let pos = (1, 64, 1);
        let original = press(game, pos);
        assert_eq!(Press::edit_state(game, original, |_, n| *n = 7), Ok(()));
        let seal = game.transaction_open();
        let mut tentative = Vec::new();
        for value in [20, 30] {
            game.journal_remove_block_entity(pos);
            let replacement = press(game, pos);
            assert_eq!(
                Press::edit_state(game, replacement, |_, n| *n = value),
                Ok(())
            );
            tentative.push(replacement);
        }
        assert_eq!(
            indexed(game, original),
            Some(7),
            "bound until the root commits"
        );
        game.transaction_close(&seal, false);
        assert_eq!(Press::state(game, original), Ok(7));
        assert_eq!(indexed(game, original), Some(7));
        for handle in tentative {
            assert_eq!(Press::state(game, handle), Err(PartStateError::Stale));
            assert_eq!(indexed(game, handle), None);
        }
    });
}

#[path = "../../../../horizon-mod-wasm/tests/fixture/src/part_state.rs"]
mod fixture;

struct NativeFixture;
#[mod_api::__subscriptions]
impl Hooks for NativeFixture {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        fixture::register(r);
    }
    fn blocks(&self, _: &mod_api::RegistrationContext, e: &mut mod_api::BlocksEvent<'_>) {
        fixture::blocks(e);
    }
}

/// The fixture's `wasm_fixture:<name>` value at `pos`, from its saved form: the host's view of
/// a wasm value.
fn saved_value(game: &Game<'_>, pos: (i32, i32, i32), name: &str) -> Option<i32> {
    let saved = game.world.block_entities.attachments(pos)?.to_nbt();
    let Some(Tag::Compound(names)) = saved.get(&"wasm_fixture".into()) else {
        return None;
    };
    let Some(Tag::Compound(entry)) = names.get(&name.into()) else {
        return None;
    };
    match entry.get(&"value".into()) {
        Some(Tag::Compound(value)) => match value.get(&"n".into()) {
            Some(Tag::Int(n)) => Some(*n),
            _ => None,
        },
        _ => None,
    }
}

/// Ticks the fixture's block entity at `pos` once in `mode`, its only data slot (data slots are
/// not saved, so a reloaded one has none).
fn tick_in(game: &mut Game<'_>, pos: (i32, i32, i32), mode: i32) {
    let Some(BlockEntity::Mod(entity)) = game.world.block_entities.get_mut(&pos) else {
        panic!("no fixture block entity at {pos:?}")
    };
    entity.data = vec![mode];
    game.tick_mod_block_entity(pos);
    assert!(game.transactions.is_empty());
}

/// The fixture's Part runs natively and compiled to wasm alike against real `Game` storage:
/// - it counts its ticks after an aborted edit and a refused recursive one;
/// - the guest refuses a state type holding a resource handle;
/// - its chunk unloads through `update_world` and loads back from the region file, and the
///   guest goes on from the saved count under the fresh handle;
/// - an undecodable saved value is refused before the closure runs and keeps its bytes;
/// - a closure that traps (wasm) leaves the stored value and no open transaction.
#[test]
#[ignore = "needs prepared resources and compiled HORIZON_WASM_FIXTURE/HORIZON_WASM_STD"]
fn native_and_compiled_wasm_part_state_count_ticks() {
    const NAME: &str =
        "game::block_entity_part_state_tests::native_and_compiled_wasm_part_state_count_ticks";
    if super::native_entity_refs::tests::run_fresh_cases(
        NAME,
        "PART_STATE_TARGET",
        &["native", "wasm"],
    ) {
        return;
    }
    horizon_guest_stack::run_on_guest_stack(|| {
        let wasm = std::env::var("PART_STATE_TARGET").unwrap() == "wasm";
        let mods: mod_api::Mods = if wasm {
            super::block_entity_tick_reply_tests::wasm_mods()
        } else {
            vec![mod_api::ModEntry::new(
                "wasm_fixture",
                Box::new(NativeFixture),
            )]
        }
        .into();
        mod_platform::testing::with_mods(mods.clone(), || {
            crate::game::api_values::install_parts(&mods).unwrap();
            mod_api::__private::mark_game_thread();
            let base =
                std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
            let root = super::mods::content_root(&base, &Default::default()).unwrap();
            super::native_entity_refs::tests::with_game_at(root.prepared().to_owned(), |game| {
                game.options.dedicated = Some(Vec::new());
                let (key, pos) = ((80, 80), (1283, 100, 1283));
                game.streaming.paused = false;
                assert!(game.streaming.set_forced(key.0, key.1, true));
                game.world
                    .chunks
                    .insert(key, Arc::new(Chunk::empty(key.0, key.1)));
                game.update_world();
                let block = game
                    .pack
                    .assets
                    .registry
                    .default_state("wasm_fixture:part_state")
                    .unwrap();
                game.world.set(pos.0, pos.1, pos.2, block).unwrap();
                let entity = ModBlockEntity::new(
                    "wasm_fixture:part_state".into(),
                    horizon_client::containers::ModBlockEntityFields {
                        data: vec![0],
                        ..Default::default()
                    },
                );
                game.world
                    .block_entities
                    .insert(pos, BlockEntity::Mod(Box::new(entity)));
                for _ in 0..3 {
                    tick_in(game, pos, 0);
                }
                assert_eq!(saved_value(game, pos, "part_state_count"), Some(3));

                tick_in(game, pos, 3);
                assert_eq!(saved_value(game, pos, "part_state_mark"), Some(3));

                // Unloaded by the owner, then loaded back from the region file.
                assert!(game.streaming.set_forced(key.0, key.1, false));
                for _ in 0..9 {
                    game.update_world();
                }
                assert!(!game.world.chunks.contains_key(&key), "unloaded");
                assert!(game.world.block_entities.attachments(pos).is_none());
                game.save_level(true).unwrap();
                let mut chunk = game
                    .save
                    .load(key.0, key.1, game.pack.assets.registry, world_biomes!(game))
                    .unwrap();
                game.world.install_chunk_block_entities(&mut chunk);
                game.world.chunks.insert(key, Arc::new(chunk));
                tick_in(game, pos, 0);
                assert_eq!(
                    saved_value(game, pos, "part_state_count"),
                    Some(4),
                    "reloaded"
                );

                // An undecodable saved count: refused, bytes kept, and the guest went on.
                let mut raw = game.world.block_entities.attachments(pos).unwrap().to_nbt();
                let Some(Tag::Compound(names)) = raw.get_mut(&"wasm_fixture".into()) else {
                    unreachable!()
                };
                let Some(Tag::Compound(count)) = names.get_mut(&"part_state_count".into()) else {
                    unreachable!()
                };
                count.insert(
                    "value".into(),
                    Tag::Compound([("n".into(), Tag::String("five".into()))].into()),
                );
                game.world
                    .block_entities
                    .restore_attachments(pos, Attachments::from_nbt(raw.clone()));
                tick_in(game, pos, 1);
                let after = game.world.block_entities.attachments(pos).unwrap().to_nbt();
                let Some(Tag::Compound(names)) = after.get(&"wasm_fixture".into()) else {
                    unreachable!()
                };
                let Some(Tag::Compound(before)) = raw.get(&"wasm_fixture".into()) else {
                    unreachable!()
                };
                assert_eq!(
                    names.get(&"part_state_count".into()),
                    before.get(&"part_state_count".into()),
                    "its bytes are kept"
                );
                assert_eq!(saved_value(game, pos, "part_state_mark"), Some(1));

                // Last, as a trap poisons the guest's link group: a fault may leave retained
                // guest state, so the runtime calls it no more (`runtime.rs`). The stored value
                // and the transactions are as before the call, and the game goes on. Natively
                // the panic unwinds out of the tick instead: that side is the unwinding edit in
                // the test above, where the next edit works.
                if wasm {
                    game.world
                        .block_entities
                        .restore_attachments(pos, Attachments::default());
                    tick_in(game, pos, 0);
                    assert_eq!(saved_value(game, pos, "part_state_count"), Some(1));
                    tick_in(game, pos, 2);
                    assert_eq!(saved_value(game, pos, "part_state_count"), Some(1), "kept");
                    tick_in(game, pos, 0);
                    assert_eq!(
                        saved_value(game, pos, "part_state_count"),
                        Some(1),
                        "poisoned"
                    );
                }
            });
        });
    });
}
