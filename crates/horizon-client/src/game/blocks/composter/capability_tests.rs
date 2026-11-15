use super::Game;
use crate::game::{api_values::TryIntoApi, native_entity_refs::tests::with_game};
use horizon_client::{chunk::Chunk, items::ItemStack};
use horizon_core::{
    geometry::Direction,
    position::{BlockPos, ChunkPos},
    random::RandomSource,
};
use mod_api::{BlockUpdateFlags, Flow, Hooks, ItemResource, ServerHost, run_transaction};
use std::sync::{Arc, Mutex};

const POS: [i32; 3] = [2, 64, 2];

fn resource(item: &str) -> ItemResource {
    ItemResource::of(&ItemStack::new(item, 1).try_into_api().unwrap())
}

fn composter(game: &mut Game<'_>, level: i32) {
    let state = game
        .pack
        .assets
        .registry
        .resolve(&horizon_world::chunk::BlockState {
            name: "minecraft:composter".into(),
            properties: [("level".into(), level.to_string())].into_iter().collect(),
        })
        .unwrap();
    game.world.set(POS[0], POS[1], POS[2], state).unwrap();
}

fn quiet(game: &mut Game<'_>, heard: &Mutex<Vec<String>>) {
    game.world.modified.clear();
    game.world.dirty_sections.clear();
    game.world.block_log = Some(Default::default());
    game.audio.capture = Some(Vec::new());
    heard.lock().unwrap().clear();
}

fn assert_quiet(game: &Game<'_>, heard: &Mutex<Vec<String>>) {
    assert!(
        heard.lock().unwrap().is_empty(),
        "game events escaped a transaction"
    );
    assert!(game.level_events.is_empty());
    assert!(
        game.audio.capture.as_ref().unwrap().is_empty(),
        "sound escaped a transaction"
    );
    assert!(
        game.world.modified.is_empty(),
        "tentative state dirtied the save"
    );
    assert!(
        game.world.dirty_sections.is_empty(),
        "tentative state requested a remesh"
    );
    assert!(
        game.world.block_log.as_ref().unwrap().is_empty(),
        "tentative state was published"
    );
    assert!(
        game.dropped.is_empty(),
        "a container extraction spawned bone meal"
    );
}

#[test]
#[ignore = "daily"]
fn sided_composter_transactions_preserve_literal_chance_and_defer_all_effects() {
    struct Heard(Arc<Mutex<Vec<String>>>);
    #[mod_api::__subscriptions]
    impl Hooks for Heard {
        fn vanilla_game_event(
            &self,
            _: &mut dyn ServerHost,
            event: &mut mod_api::VanillaGameEventEvent<'_>,
        ) -> Flow {
            self.0.lock().unwrap().push(event.event.to_owned());
            Flow::Continue
        }
    }
    let heard = Arc::new(Mutex::new(Vec::new()));
    mod_platform::testing::with_mods(
        vec![mod_api::ModEntry::new(
            "composter_observer",
            Box::new(Heard(heard.clone())),
        )]
        .into(),
        || {
            with_game(|game| {
                for (x, z) in (-1..=1).flat_map(|x| (-1..=1).map(move |z| (x, z))) {
                    let mut chunk = Chunk::empty(x, z);
                    chunk.lit = true;
                    game.world.chunks.insert((x, z), Arc::new(chunk));
                }
                game.block_ticks
                    .add_empty_chunk(ChunkPos::new(0, 0))
                    .unwrap();
                composter(game, 0);
                let level = game.level("minecraft:overworld").unwrap();
                let input = level
                    .capability(game, &mod_api::ITEMS, POS, Some(Direction::Up))
                    .unwrap();
                let unsided = level.capability(game, &mod_api::ITEMS, POS, None).unwrap();
                let output = level
                    .capability(game, &mod_api::ITEMS, POS, Some(Direction::Down))
                    .unwrap();
                let storage_input = level
                    .capability(game, &mod_api::STORAGE, POS, Some(Direction::Up))
                    .unwrap();
                let storage_output = level
                    .capability(game, &mod_api::STORAGE, POS, Some(Direction::Down))
                    .unwrap();
                for face in [
                    Direction::North,
                    Direction::South,
                    Direction::West,
                    Direction::East,
                ] {
                    assert!(
                        level
                            .capability(game, &mod_api::STORAGE, POS, Some(face))
                            .is_none()
                    );
                    assert!(
                        level
                            .capability(game, &mod_api::ITEMS, POS, Some(face))
                            .is_none()
                    );
                }
                let seeds = resource("minecraft:wheat_seeds");
                let pie = resource("minecraft:pumpkin_pie");
                game.options.dedicated = Some(Vec::new());
                let bone = resource("minecraft:bone_meal");
                let stone = resource("minecraft:stone");
                assert_eq!(storage_input.amount(game, &seeds), Ok(0));
                assert_eq!(storage_input.space_for(game, &seeds), Ok(1));
                assert_eq!(storage_input.space_for(game, &stone), Ok(0));
                assert_eq!(storage_output.amount(game, &bone), Ok(0));
                assert_eq!(storage_output.space_for(game, &seeds), Ok(0));
                assert_eq!(input.size(game), 1);
                assert_eq!(unsided.size(game), 1);
                assert_eq!(output.size(game), 1);
                assert_eq!(input.capacity(game, 0, &seeds), 1);
                assert_eq!(input.capacity(game, 0, &stone), 0);
                assert_eq!(input.capacity(game, 1, &seeds), 0);
                assert_eq!(output.capacity(game, 0, &seeds), 0);
                assert_eq!(output.amount(game, 0), 0);
                assert!(input.resource(game, 0).is_none());
                quiet(game, &heard);
                run_transaction(game, |game, tx| {
                    assert_eq!(input.insert(game, tx, 0, &stone, 64), 0);
                    assert_eq!(input.insert(game, tx, 1, &seeds, 64), 0);
                    assert_eq!(input.insert(game, tx, 0, &seeds, 0), 0);
                    assert_eq!(output.insert(game, tx, 0, &seeds, 64), 0);
                    assert_eq!(input.extract(game, tx, 0, &seeds, 64), 0);
                    assert!(!input.set_max_stack_size(game, tx, 64));
                    assert_eq!(storage_input.insert(game, tx, &stone, 64), 0);
                    assert_eq!(storage_input.insert(game, tx, &seeds, 0), 0);
                    assert_eq!(storage_output.insert(game, tx, &seeds, 64), 0);
                    assert_eq!(storage_input.extract(game, tx, &seeds, 64), 0);
                    Ok::<_, ()>(())
                })
                .unwrap();
                assert_eq!(game.composter_level(POS), Some(0));
                assert_quiet(game, &heard);

                // A source container and the composter participate in the SAME transaction.
                let source_pos = [4, 64, 2];
                let chest = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:chest")
                    .unwrap();
                game.level_set_block(level, source_pos, chest, BlockUpdateFlags::NEIGHBORS)
                    .unwrap();
                let source = level
                    .capability(game, &mod_api::ITEMS, source_pos, None)
                    .unwrap();
                run_transaction(game, |game, tx| {
                    assert_eq!(source.insert(game, tx, 0, &pie, 64), 64);
                    Ok::<_, ()>(())
                })
                .unwrap();
                // Each position's capability-local owner takes its initial seed from the level.
                game.world.random.set_seed(3448);
                composter(game, 6);
                quiet(game, &heard);
                let state = game.world.block(POS[0], POS[1], POS[2]);
                let block = game.res.behaviours().block(state);
                let at = BlockPos::new(POS[0], POS[1], POS[2]);
                let aborted = run_transaction(game, |game, tx| {
                    assert_eq!(source.extract(game, tx, 0, &pie, 1), 1);
                    run_transaction(game, |game, tx| {
                        assert_eq!(storage_input.insert(game, tx, &pie, 64), 1);
                        Ok::<_, ()>(())
                    })?;
                    assert_eq!(game.composter_level(POS), Some(7));
                    assert_eq!(source.amount(game, 0), 63);
                    assert!(!game.block_ticks.has_block_tick(at, block));
                    assert_quiet(game, &heard);
                    Err::<(), ()>(())
                });
                assert_eq!(aborted, Err(()));
                assert_eq!(game.composter_level(POS), Some(6));
                assert_eq!(source.amount(game, 0), 64);
                assert!(!game.block_ticks.has_block_tick(at, block));
                assert_quiet(game, &heard);

                // Inner abort does not discard an outer insert, and an outer commit runs only
                // that insert's feedback. This also exercises the unsided input port.
                composter(game, 0);
                quiet(game, &heard);
                run_transaction(game, |game, tx| {
                    assert_eq!(unsided.insert(game, tx, 0, &pie, 64), 1);
                    let aborted = run_transaction(game, |game, tx| {
                        assert_eq!(input.insert(game, tx, 0, &pie, 64), 1);
                        assert_eq!(game.composter_level(POS), Some(2));
                        Err::<(), ()>(())
                    });
                    assert_eq!(aborted, Err(()));
                    assert_eq!(game.composter_level(POS), Some(1));
                    assert_quiet(game, &heard);
                    Ok::<_, ()>(())
                })
                .unwrap();
                assert_eq!(game.composter_level(POS), Some(1));
                assert_eq!(heard.lock().unwrap().as_slice(), ["minecraft:block_change"]);

                composter(game, 0);
                quiet(game, &heard);
                // Stock compostable/low: level zero adds one WITHOUT a draw. Then Java
                // Random(3448).nextInt(100): 4,36,35,20,2,56,93,4,7,54,55,18 (success <30).
                // Literal expectations, not a second run through Horizon's layer algorithm.
                run_transaction(game, |game, tx| {
                    for expected in [1, 2, 2, 2, 3, 4, 4, 4, 5, 6, 6, 6, 7] {
                        assert_eq!(input.insert(game, tx, 0, &seeds, 64), 1);
                        assert_eq!(game.composter_level(POS), Some(expected));
                        assert_eq!(input.amount(game, 0), 0);
                        assert_eq!(output.amount(game, 0), 0);
                        assert!(!game.block_ticks.has_block_tick(at, block));
                        assert_quiet(game, &heard);
                    }
                    assert_eq!(input.insert(game, tx, 0, &seeds, 64), 0);
                    Ok::<_, ()>(())
                })
                .unwrap();
                assert_eq!(
                    game.composter_level(POS),
                    Some(7),
                    "feedback must not replay intermediate states"
                );
                assert_eq!(heard.lock().unwrap().len(), 7);
                assert!(game.block_ticks.has_block_tick(at, block));
                assert_eq!(input.capacity(game, 0, &seeds), 0);
                let start = game.world.game_time;
                game.world.game_time = start + 19;
                game.tick_scheduled().unwrap();
                assert_eq!(game.composter_level(POS), Some(7));
                game.world.game_time = start + 20;
                game.tick_scheduled().unwrap();
                assert_eq!(game.composter_level(POS), Some(8));
                assert!(
                    game.audio
                        .capture
                        .as_ref()
                        .unwrap()
                        .iter()
                        .any(|s| s.event == "minecraft:block.composter.ready")
                );
                assert_eq!(input.amount(game, 0), 0);
                assert_eq!(unsided.amount(game, 0), 0);
                assert_eq!(output.amount(game, 0), 1);
                assert_eq!(output.resource(game, 0), Some(bone.clone()));
                assert_eq!(storage_input.amount(game, &bone), Ok(0));
                assert_eq!(storage_output.amount(game, &bone), Ok(1));
                assert_eq!(storage_output.space_for(game, &bone), Ok(0));

                quiet(game, &heard);
                let aborted = run_transaction(game, |game, tx| {
                    assert_eq!(output.extract(game, tx, 0, &pie, 1), 0);
                    assert_eq!(output.extract(game, tx, 0, &bone, 0), 0);
                    assert_eq!(storage_output.extract(game, tx, &bone, 64), 1);
                    assert_eq!(output.extract(game, tx, 0, &bone, 64), 0);
                    assert_eq!(game.composter_level(POS), Some(0));
                    assert_quiet(game, &heard);
                    Err::<(), ()>(())
                });
                assert_eq!(aborted, Err(()));
                assert_eq!(game.composter_level(POS), Some(8));
                assert_eq!(output.amount(game, 0), 1);
                assert_quiet(game, &heard);
                run_transaction(game, |game, tx| {
                    assert_eq!(output.extract(game, tx, 0, &bone, 64), 1);
                    Ok::<_, ()>(())
                })
                .unwrap();
                assert_eq!(game.composter_level(POS), Some(0));
                assert_eq!(heard.lock().unwrap().as_slice(), ["minecraft:block_change"]);
                assert!(game.audio.capture.as_ref().unwrap().is_empty());
                assert!(game.dropped.is_empty());

                // A real bulk consumer offers all 64, not a pre-trimmed single item.
                // It must preserve the literal sequence above across its nested partial
                // attempts, rather than drawing a second chance on each accepted retry.
                run_transaction(game, |game, tx| {
                    assert_eq!(source.extract(game, tx, 0, &pie, 64), 64);
                    assert_eq!(source.insert(game, tx, 0, &seeds, 64), 64);
                    Ok::<_, ()>(())
                })
                .unwrap();
                let bulk_pos = [6, 64, 2];
                let empty = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:composter")
                    .unwrap();
                game.world
                    .set(bulk_pos[0], bulk_pos[1], bulk_pos[2], empty)
                    .unwrap();
                let bulk = level
                    .capability(game, &mod_api::ITEMS, bulk_pos, None)
                    .unwrap();
                let bulk_at = BlockPos::new(bulk_pos[0], bulk_pos[1], bulk_pos[2]);
                quiet(game, &heard);
                game.world.random.set_seed(3448);
                let mut unrelated_world = game.world.random.clone();
                let aborted = run_transaction(game, |game, tx| {
                    assert_eq!(
                        mod_api::move_resources(game, &source, &bulk, |_| true, 64, tx),
                        Ok(13),
                    );
                    assert_eq!(game.composter_level(bulk_pos), Some(7));
                    assert_eq!(source.amount(game, 0), 51);
                    assert!(!game.block_ticks.has_block_tick(bulk_at, block));
                    assert_quiet(game, &heard);
                    Err::<(), ()>(())
                });
                assert_eq!(aborted, Err(()));
                assert_eq!(game.composter_level(bulk_pos), Some(0));
                assert_eq!(source.amount(game, 0), 64);
                assert!(!game.block_ticks.has_block_tick(bulk_at, block));
                assert_quiet(game, &heard);

                // Another dimension's unload must leave this cached owner intact, even
                // after a first aborted insertion and a different seed in the level stream.
                game.world.random.set_seed(0);
                unrelated_world.set_seed(0);
                game.enter_level(crate::game::Dim::Nether).unwrap();
                game.enter_level(crate::game::Dim::Overworld).unwrap();
                assert!(game.level("minecraft:the_nether").is_some());
                game.native_chunk_unload("minecraft:the_nether", [0, 0]);

                // A nested abort rewinds only this composter's chance owner. An unrelated
                // level draw between the aborted attempt and its retry must survive.
                let aborted = run_transaction(game, |game, tx| {
                    assert_eq!(bulk.insert(game, tx, 0, &seeds, 1), 1);
                    let nested = run_transaction(game, |game, tx| {
                        assert_eq!(bulk.insert(game, tx, 0, &seeds, 1), 1);
                        assert_eq!(game.composter_level(bulk_pos), Some(2));
                        Err::<(), ()>(())
                    });
                    assert_eq!(nested, Err(()));
                    assert_eq!(game.composter_level(bulk_pos), Some(1));
                    assert_eq!(game.world.random.next_long(), unrelated_world.next_long());
                    assert_eq!(bulk.insert(game, tx, 0, &seeds, 1), 1);
                    assert_eq!(game.composter_level(bulk_pos), Some(2));
                    assert_quiet(game, &heard);
                    Err::<(), ()>(())
                });
                assert_eq!(aborted, Err(()));
                assert_eq!(game.composter_level(bulk_pos), Some(0));
                assert_eq!(game.world.random.next_long(), unrelated_world.next_long());
                run_transaction(game, |game, tx| {
                    assert_eq!(
                        mod_api::move_resources(game, &source, &bulk, |_| true, 64, tx),
                        Ok(13),
                    );
                    assert_eq!(game.composter_level(bulk_pos), Some(7));
                    assert_eq!(source.amount(game, 0), 51);
                    assert!(!game.block_ticks.has_block_tick(bulk_at, block));
                    assert_quiet(game, &heard);
                    Ok::<_, ()>(())
                })
                .unwrap();
                assert_eq!(game.composter_level(bulk_pos), Some(7));
                assert_eq!(source.amount(game, 0), 51);
                assert_eq!(heard.lock().unwrap().len(), 7);
                assert!(game.block_ticks.has_block_tick(bulk_at, block));
                assert_eq!(game.world.random.next_long(), unrelated_world.next_long());

                // The existing unload lifecycle retires the chunk's position-local owners.
                // Reusing a retained handler after reload starts from the new level seed,
                // not the previous owner's already-consumed chance sequence.
                game.native_chunk_unload("minecraft:overworld", [0, 0]);
                // A pending outer savepoint must retire too: aborting after unload cannot
                // resurrect the previous owner's consumed chance sequence.
                let aborted = run_transaction(game, |game, tx| {
                    game.world
                        .set(bulk_pos[0], bulk_pos[1], bulk_pos[2], empty)
                        .unwrap();
                    assert_eq!(bulk.insert(game, tx, 0, &seeds, 1), 1);
                    game.native_chunk_unload("minecraft:overworld", [0, 0]);
                    Err::<(), ()>(())
                });
                assert_eq!(aborted, Err(()));
                game.world
                    .set(bulk_pos[0], bulk_pos[1], bulk_pos[2], empty)
                    .unwrap();
                game.world.random.set_seed(3448);
                quiet(game, &heard);
                run_transaction(game, |game, tx| {
                    assert_eq!(
                        mod_api::move_resources(game, &source, &bulk, |_| true, 64, tx),
                        Ok(13),
                    );
                    Ok::<_, ()>(())
                })
                .unwrap();
                assert_eq!(game.composter_level(bulk_pos), Some(7));
                assert_eq!(source.amount(game, 0), 38);

                // A later write in the same transaction wins: deferred fill feedback must not
                // resurrect the composter, even when insertion reached maturation level seven.
                composter(game, 6);
                run_transaction(game, |game, tx| {
                    assert_eq!(input.insert(game, tx, 0, &pie, 64), 1);
                    let stone = game
                        .pack
                        .assets
                        .registry
                        .default_state("minecraft:stone")
                        .unwrap();
                    assert!(game.journal_set_block(POS, stone));
                    Ok::<_, ()>(())
                })
                .unwrap();
                assert_eq!(game.composter_level(POS), None);
                assert_eq!(input.size(game), 0);

                // A reopened world gets new owners and rejects retained old-epoch handlers.
                game.native_lifecycle_open_started();
                let reopened = game.level("minecraft:overworld").unwrap();
                assert_ne!(reopened.epoch(), level.epoch());
                assert_eq!(bulk.size(game), 0);
                assert_eq!(source.size(game), 0);
                let source = reopened
                    .capability(game, &mod_api::ITEMS, source_pos, None)
                    .unwrap();
                let bulk = reopened
                    .capability(game, &mod_api::ITEMS, bulk_pos, None)
                    .unwrap();
                game.world
                    .set(bulk_pos[0], bulk_pos[1], bulk_pos[2], empty)
                    .unwrap();
                game.world.random.set_seed(3448);
                run_transaction(game, |game, tx| {
                    assert_eq!(source.insert(game, tx, 0, &seeds, 26), 26);
                    assert_eq!(
                        mod_api::move_resources(game, &source, &bulk, |_| true, 64, tx),
                        Ok(13),
                    );
                    Ok::<_, ()>(())
                })
                .unwrap();
                assert_eq!(game.composter_level(bulk_pos), Some(7));
                assert_eq!(source.amount(game, 0), 51);
            })
        },
    );
}

#[test]
#[ignore = "daily"]
fn a_client_copy_never_answers_composter_items() {
    crate::game::native_entity_refs::tests::with_client_game(|game| {
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        composter(game, 0);
        let epoch = game.native_lifecycle.epoch().unwrap();
        let level = mod_api::LevelRef::new(epoch, game.dim.key());
        for side in [None, Some(Direction::Up), Some(Direction::Down)] {
            assert!(level.capability(game, &mod_api::ITEMS, POS, side).is_none());
            assert!(
                level
                    .capability(game, &mod_api::STORAGE, POS, side)
                    .is_none()
            );
        }
    });
}
