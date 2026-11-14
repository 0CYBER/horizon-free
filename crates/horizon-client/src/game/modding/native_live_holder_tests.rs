//! Real-Game holder-boundary regressions and the ignored 1,000-ticker measurement.
use std::{sync::Arc, time::Instant};

use horizon_client::{
    chunk::Chunk,
    containers::{BlockEntity, ModBlockEntity},
};
use mod_api::{
    BlockEntityPart, BlockEntityTickEvent, BlockEntityTicksEvent, BlockPart, BlocksEvent, Hooks,
    Host, HostTransaction, Registrar, ServerHost,
};

struct Ticker(bool);
#[mod_api::horizon_part]
impl BlockPart for Ticker {}
#[mod_api::horizon_part]
impl BlockEntityPart for Ticker {
    fn block_entity_tick(&self, host: &mut dyn ServerHost, event: &mut BlockEntityTickEvent<'_>) {
        if self.0 {
            let level = host.level("minecraft:overworld").unwrap();
            let sink = host.block_entity(level, [0, 63, 0]).unwrap();
            let value = sink.data_slots(host).unwrap()[0];
            let result: Result<(), ()> = host.transaction(|host, _| {
                assert!(sink.set_data_slot(host, 0, value + 1).unwrap());
                Ok(())
            });
            result.unwrap();
        }
        std::hint::black_box(event.pos);
    }
}
struct BenchMod(&'static str);
#[mod_api::__subscriptions]
impl Hooks for BenchMod {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        let kind = r.block_entity_type("ticker").blocks(&[]).build();
        if self.0 != "none" {
            r.block_entity_part(&kind, Ticker(self.0 == "write"))
                .unwrap();
        }
    }
}

#[test]
#[ignore = "measurement; needs HORIZON_TEST_RESOURCES and HORIZON_LIVE_BENCH=none|trivial|write"]
fn thousand_live_block_entity_ticks() {
    let mode = std::env::var("HORIZON_LIVE_BENCH").unwrap();
    let mode = match mode.as_str() {
        "none" => "none",
        "trivial" => "trivial",
        "write" => "write",
        _ => panic!("unknown measurement mode"),
    };
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "live_bench",
        Box::new(BenchMod(mode)),
    )]
    .into();
    mod_platform::testing::with_mods(mods.clone(), || {
        crate::game::api_values::install_parts(&mods).unwrap();
        mod_api::__private::mark_game_thread();
        let base = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
        let root = super::mods::content_root(&base, &Default::default()).unwrap();
        super::native_entity_refs::tests::with_game_at(root.prepared().to_path_buf(), |game| {
            for x in 0..4 {
                for z in 0..4 {
                    game.world
                        .chunks
                        .insert((x, z), Arc::new(Chunk::empty(x, z)));
                }
            }
            let positions: Vec<_> = (0..1000).map(|i| (i % 40, 64, i / 40)).collect();
            for &pos in positions.iter().chain(std::iter::once(&(0, 63, 0))) {
                let mut entity =
                    ModBlockEntity::new("live_bench:ticker".into(), Default::default());
                entity.data = vec![0];
                game.world
                    .block_entities
                    .insert(pos, BlockEntity::Mod(Box::new(entity)));
            }
            let mut times = Vec::with_capacity(240);
            for round in 0..260 {
                let start = Instant::now();
                for &pos in &positions {
                    game.tick_mod_block_entity(pos);
                }
                let elapsed = start.elapsed().as_nanos();
                if round >= 20 {
                    times.push(elapsed);
                }
            }
            times.sort_unstable();
            if mode == "write" {
                let BlockEntity::Mod(sink) = game.world.block_entities.get(&(0, 63, 0)).unwrap()
                else {
                    panic!("sink removed")
                };
                assert_eq!(sink.data, [260_000]);
            }
            println!(
                "LIVE_BENCH mode={mode} holders=1000 samples={} median_ns={} p95_ns={} p99_ns={}",
                times.len(),
                times[120],
                times[228],
                times[237]
            );
        });
    });
}

/// Run the test `name` with this file's mod installed, in a fresh process: its content root
/// freezes the process's block entity types, which another test may already have frozen.
pub(super) fn with_mod_parts(name: &str, test: impl FnOnce(std::path::PathBuf)) {
    if super::native_entity_refs::tests::run_fresh_cases(
        name,
        "HORIZON_LIVE_HOLDER_GAME_CHILD",
        &["valid"],
    ) {
        return;
    }
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "live_bench",
        Box::new(BenchMod("none")),
    )]
    .into();
    mod_platform::testing::with_mods(mods.clone(), || {
        crate::game::api_values::install_parts(&mods).unwrap();
        mod_api::__private::mark_game_thread();
        let base = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
        test(
            super::mods::content_root(&base, &Default::default())
                .unwrap()
                .prepared()
                .to_owned(),
        );
    });
}

pub(super) fn mod_holder(game: &mut super::Game<'_>) -> mod_api::BlockEntityRef {
    game.options.dedicated = Some(Vec::new());
    game.world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    let stone = game
        .pack
        .assets
        .registry
        .default_state("minecraft:stone")
        .unwrap();
    game.world.set(1, 64, 1, stone).unwrap();
    let holder = ModBlockEntity::new("live_bench:ticker".into(), Default::default());
    game.world
        .block_entities
        .insert((1, 64, 1), BlockEntity::Mod(Box::new(holder)));
    let level = game.level("minecraft:overworld").unwrap();
    game.block_entity(level, [1, 64, 1]).unwrap()
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES; runs in an isolated scratch process"]
fn data_slot_sizing_after_creation_and_load_joins_the_existing_journal() {
    use mod_api::BlockEntityDataError;
    const NAME: &str = "game::native_live_holder_tests::data_slot_sizing_after_creation_and_load_joins_the_existing_journal";
    with_mod_parts(NAME, |root| {
        super::native_entity_refs::tests::with_game_at(root, |game| {
            let entity = mod_holder(game);
            let pos = (1, 64, 1);
            assert_eq!(entity.data_slots(game), Some(vec![]));
            assert!(
                !entity.set_data_slot(game, 0, 1).unwrap(),
                "scalar writes cannot declare slots"
            );
            assert_eq!(entity.set_data_slots(game, vec![0]), Ok(true));
            let revision = |game: &super::Game<'_>| {
                let BlockEntity::Mod(entity) = game.world.block_entities.get(&pos).unwrap() else {
                    panic!("mod holder missing")
                };
                entity.revision()
            };
            let before_revision = revision(game);
            game.world.modified.clear();
            assert!(!entity.set_data_slot(game, 0, 0).unwrap());
            assert!(!entity.set_data_slot(game, 1, 1).unwrap());
            assert_eq!(entity.set_data_slots(game, vec![0]), Ok(false));
            assert_eq!(revision(game), before_revision);
            assert!(
                game.world.modified.is_empty(),
                "no-op sends no Changed notice"
            );
            assert_eq!(entity.data_slots(game), Some(vec![0]));
            let aborted: Result<(), ()> = (game as &mut dyn ServerHost).transaction(|host, _| {
                assert_eq!(entity.set_data_slots(host, vec![30, 40]), Ok(true));
                host.transaction(|host, _| {
                    assert!(entity.set_data_slot(host, 0, 50).unwrap());
                    Ok(())
                })?;
                Err(())
            });
            assert_eq!(aborted, Err(()));
            assert_eq!(entity.data_slots(game), Some(vec![0]));
            assert!(game.transactions.is_empty());
            assert_eq!(entity.set_data_slots(game, vec![0; 256]), Ok(true));
            assert!(entity.set_data_slot(game, 255, 7).unwrap());
            let before = entity.data_slots(game).unwrap();
            let before_revision = revision(game);
            game.world.modified.clear();
            assert_eq!(
                entity.set_data_slots(game, vec![0; 257]),
                Err(BlockEntityDataError::TooManySlots {
                    count: 257,
                    limit: 256
                })
            );
            assert_eq!(entity.data_slots(game), Some(before));
            assert_eq!(revision(game), before_revision);
            assert!(
                game.world.modified.is_empty(),
                "refusal sends no Changed notice"
            );
            let saved = horizon_client::save::block_entity_encode::encode(
                pos,
                game.world.block_entities.get(&pos).unwrap(),
                "minecraft:stone",
            );
            let (loaded_pos, loaded) = horizon_client::save::decode_block_entity(&saved).unwrap();
            game.world.block_entities.remove(&pos);
            assert_eq!(
                entity.set_data_slots(game, vec![8]),
                Err(BlockEntityDataError::Stale)
            );
            assert_eq!(loaded_pos, pos);
            game.world.block_entities.insert(loaded_pos, loaded);
            assert_eq!(
                entity.data_slots(game),
                None,
                "old identity stays stale after load"
            );
            let entity = game
                .block_entity(game.level("minecraft:overworld").unwrap(), [1, 64, 1])
                .unwrap();
            assert_eq!(
                entity.data_slots(game),
                Some(vec![]),
                "menu data is transient, not saved NBT"
            );
            assert_eq!(entity.set_data_slots(game, vec![0]), Ok(true));
            assert!(entity.set_data_slot(game, 0, 1).unwrap());
            assert_eq!(entity.data_slots(game), Some(vec![1]));
        });
    });
}

/// The two block entity types of #2051's fixture: `tick_paths:both` is
/// named by both paths (a `block_entity_type` declaration and a Registrar
/// part carrying the ticker), `tick_paths:bare` is a declaration alone.
const BOTH_BLOCK: &str = "tick_paths:press";
const BARE_BLOCK: &str = "tick_paths:bare";
struct TickPaths;
#[mod_api::__subscriptions]
impl Hooks for TickPaths {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        let both = r.block_entity_type("both").blocks(&[BOTH_BLOCK]).build();
        r.block_entity_part(&both, BothPaths).unwrap();
        r.block_entity_type("bare").blocks(&[BARE_BLOCK]).build();
    }
    fn blocks(&self, _: &mod_api::RegistrationContext, event: &mut BlocksEvent<'_>) {
        for id in [BOTH_BLOCK, BARE_BLOCK] {
            event
                .blocks
                .push(mod_api::Block::new(id, "minecraft:stone"));
        }
    }
}
/// Carries `tick_paths:both`'s ticker: its block ticks, and each tick adds
/// one data slot.
struct BothPaths;
#[mod_api::horizon_part]
impl BlockPart for BothPaths {}
#[mod_api::horizon_part]
impl BlockEntityPart for BothPaths {
    fn block_entity_ticks(&self, _: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
        if event.block == BOTH_BLOCK {
            event.ticks = true;
        }
    }
    fn block_entity_tick(&self, _: &mut dyn ServerHost, event: &mut BlockEntityTickEvent<'_>) {
        event.entity.data.push(1);
    }
}

/// #2051: the one decision whether a mod block entity ticks is its type's
/// own ticker, whichever path named the type — a type both a declaration
/// and a Registrar part carry ticks exactly once per tick, and a bare
/// declaration, whose type declares no ticker, never reaches the level's
/// ticking list.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES; runs in an isolated scratch process"]
fn a_declared_type_ticks_once_whichever_path_named_it() {
    // The overlay's `.horizon-cache` lives under the working directory and
    // `native_declared()` caches the first fire per process, so tests sharing
    // one process poison each other's mod content (`mods::tests::run_in_child`).
    if super::native_entity_refs::tests::run_fresh_cases(
        "game::native_live_holder_tests::a_declared_type_ticks_once_whichever_path_named_it",
        "HORIZON_TICK_PATHS_GAME_CHILD",
        &["run"],
    ) {
        return;
    }
    let mods: mod_api::Mods =
        vec![mod_api::ModEntry::new("tick_paths", Box::new(TickPaths))].into();
    mod_platform::testing::with_mods(mods.clone(), || {
        crate::game::api_values::install_parts(&mods).unwrap();
        mod_api::__private::mark_game_thread();
        let base = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
        let root = super::mods::content_root(&base, &Default::default()).unwrap();
        super::native_entity_refs::tests::with_game_at(root.prepared().to_path_buf(), |game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            let press = registry.default_state(BOTH_BLOCK).unwrap();
            let bare = registry.default_state(BARE_BLOCK).unwrap();
            game.world.set(1, 64, 1, press).unwrap();
            game.world.set(2, 64, 1, bare).unwrap();
            for (pos, kind) in [
                ((1, 64, 1), "tick_paths:both"),
                ((2, 64, 1), "tick_paths:bare"),
            ] {
                game.world.block_entities.insert(
                    pos,
                    BlockEntity::Mod(Box::new(ModBlockEntity::new(
                        kind.into(),
                        Default::default(),
                    ))),
                );
            }
            // The type's own ticker is the one decision: the part's answer
            // for the press block, nothing for the bare declaration.
            assert!(game.native_ticks(BOTH_BLOCK));
            assert!(!game.native_ticks(BARE_BLOCK));
            // The level's ticking list carries exactly the ticker's answer:
            // one slot for the press, none for the bare declaration.
            game.world.block_entities.set_states(game.ticking_states());
            let tickers = game.world.block_entity_tickers();
            assert_eq!(
                tickers.iter().filter(|(pos, _)| *pos == (1, 64, 1)).count(),
                1
            );
            assert!(
                !tickers.iter().any(|(pos, _)| *pos == (2, 64, 1)),
                "a type with no ticker never reaches the ticking list"
            );
            // Exactly one tick per game tick, the type named by both paths:
            // the declaration and the part each count once, not twice.
            for expected in [1, 2, 3] {
                game.tick_mod_block_entity((1, 64, 1));
                let BlockEntity::Mod(entity) = game.world.block_entities.get(&(1, 64, 1)).unwrap()
                else {
                    panic!("press entity missing")
                };
                assert_eq!(entity.data, vec![1; expected]);
            }
            // A declared type with no listener stays inert through its tick
            // call: no data, no write.
            game.tick_mod_block_entity((2, 64, 1));
            let BlockEntity::Mod(entity) = game.world.block_entities.get(&(2, 64, 1)).unwrap()
            else {
                panic!("bare entity missing")
            };
            assert!(
                entity.data.is_empty(),
                "a type no path gave a ticker changes nothing"
            );
            assert_eq!(entity.revision().changes(), 0, "no listener, no write");
        });
    });
}
