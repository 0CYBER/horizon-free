//! `BlockCapabilityCache` in a real game: kept while nothing it depends on changes, looked up
//! again for each way the answer can change, and a move through kept handlers that aborts
//! loses nothing.
use super::Dim;
use horizon_client::{
    chunk::Chunk,
    containers::{BlockEntity, ModBlockEntity},
    items::ItemStack,
};
use mod_api::{
    BlockCapabilityCache, CapabilityKey, Hooks, HostTransaction, ITEMS, ItemResource, Registrar,
    ResourceHandler, ServerHost, Transaction,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// A capability only the test mod answers, counting each time its provider is asked.
const COUNTED: CapabilityKey<dyn ResourceHandler<ItemResource>> =
    CapabilityKey::new("cache_bench:counted");
static ASKED: AtomicUsize = AtomicUsize::new(0);
/// How many slots the provider's next handler has: a provider whose answer changes with nothing
/// the cache sees.
static SLOTS: AtomicUsize = AtomicUsize::new(0);

/// A handler with nothing in it, of the size the provider was set to when it answered.
struct Empty(usize);
impl ResourceHandler<ItemResource> for Empty {
    fn size(&self, _: &mut dyn ServerHost) -> usize {
        self.0
    }
    fn resource(&self, _: &mut dyn ServerHost, _: usize) -> Option<ItemResource> {
        None
    }
    fn amount(&self, _: &mut dyn ServerHost, _: usize) -> u32 {
        0
    }
    fn capacity(&self, _: &mut dyn ServerHost, _: usize, _: &ItemResource) -> u32 {
        0
    }
    fn insert(
        &self,
        _: &mut dyn ServerHost,
        _: &Transaction,
        _: usize,
        _: &ItemResource,
        _: u32,
    ) -> u32 {
        0
    }
    fn extract(
        &self,
        _: &mut dyn ServerHost,
        _: &Transaction,
        _: usize,
        _: &ItemResource,
        _: u32,
    ) -> u32 {
        0
    }
}

struct CacheBench;
#[mod_api::__subscriptions]
impl Hooks for CacheBench {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        let kind = r.block_entity_type("holder").blocks(&[]).build();
        r.block_entity_capability(&COUNTED, &kind, |_, _| {
            ASKED.fetch_add(1, Ordering::Relaxed);
            Some(Box::new(Empty(SLOTS.load(Ordering::Relaxed))))
        });
        // A block with no block entity, whose provider counts in the same way.
        let lodestone = r.block_key("minecraft:lodestone");
        r.block_capability(&COUNTED, &lodestone, |_, _, _, entity| {
            ASKED.fetch_add(1, Ordering::Relaxed);
            entity
                .is_none()
                .then(|| Box::new(Empty(SLOTS.load(Ordering::Relaxed))) as _)
        });
    }
}

const POS: (i32, i32, i32) = (1, 64, 1);

fn with_cache_mod(name: &str, test: impl FnOnce(&mut super::Game<'_>, mod_api::LevelRef)) {
    if super::native_entity_refs::tests::run_fresh_cases(
        name,
        "HORIZON_CAPABILITY_CACHE_CHILD",
        &["valid"],
    ) {
        return;
    }
    let mods: mod_api::Mods =
        vec![mod_api::ModEntry::new("cache_bench", Box::new(CacheBench))].into();
    mod_platform::testing::with_mods(mods.clone(), || {
        super::api_values::install_parts(&mods).unwrap();
        mod_api::__private::mark_game_thread();
        let base = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
        let root = super::mods::content_root(&base, &Default::default())
            .unwrap()
            .prepared()
            .to_owned();
        super::native_entity_refs::tests::with_game_at(root, |game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            place_holder(game, "minecraft:stone");
            let level = game.level("minecraft:overworld").unwrap();
            test(game, level);
        });
    });
}

/// The test mod's block entity at `POS`, on `block`.
fn place_holder(game: &mut super::Game<'_>, block: &str) {
    let state = game.pack.assets.registry.default_state(block).unwrap();
    game.world.set(POS.0, POS.1, POS.2, state).unwrap();
    let holder = ModBlockEntity::new("cache_bench:holder".into(), Default::default());
    game.world
        .block_entities
        .insert(POS, BlockEntity::Mod(Box::new(holder)));
}

fn cache(level: mod_api::LevelRef) -> BlockCapabilityCache<dyn ResourceHandler<ItemResource>> {
    BlockCapabilityCache::new(&COUNTED, level, [POS.0, POS.1, POS.2], None)
}

/// Whether the cache answers, and how many lookups reached the provider so far.
fn ask(
    game: &mut super::Game<'_>,
    cache: &mut BlockCapabilityCache<dyn ResourceHandler<ItemResource>>,
) -> (bool, usize) {
    let found = cache.get(game).is_some();
    (found, ASKED.load(Ordering::Relaxed))
}

/// The size of the handler the cache lends, if it lends one.
fn slots(
    game: &mut super::Game<'_>,
    cache: &mut BlockCapabilityCache<dyn ResourceHandler<ItemResource>>,
) -> Option<usize> {
    let handler = cache.get(game)?;
    Some(handler.size(game))
}

#[test]
#[ignore = "daily"]
fn a_cache_asks_once_while_nothing_changes_and_again_when_invalidated() {
    with_cache_mod(
        "game::native_capability_cache_tests::a_cache_asks_once_while_nothing_changes_and_again_when_invalidated",
        |game, level| {
            let mut cache = cache(level);
            let start = ASKED.load(Ordering::Relaxed);
            SLOTS.store(1, Ordering::Relaxed);
            for _ in 0..100 {
                assert_eq!(ask(game, &mut cache), (true, start + 1), "kept: asked once");
            }
            // The provider would answer differently now, but nothing the cache sees changed.
            SLOTS.store(2, Ordering::Relaxed);
            assert_eq!(slots(game, &mut cache), Some(1), "the kept answer");
            cache.invalidate();
            assert_eq!(
                slots(game, &mut cache),
                Some(2),
                "the provider's own invalidate"
            );
            assert_eq!(ask(game, &mut cache), (true, start + 2));
        },
    );
}

#[test]
#[ignore = "daily"]
fn a_cache_lends_nothing_while_its_level_is_parked_and_asks_again_once_back() {
    with_cache_mod(
        "game::native_capability_cache_tests::a_cache_lends_nothing_while_its_level_is_parked_and_asks_again_once_back",
        |game, level| {
            let mut cache = cache(level);
            let start = ask(game, &mut cache).1;
            game.enter_level(Dim::Nether).unwrap();
            assert!(!level.in_place(&*game), "the overworld is parked");
            assert!(
                game.block_entity(level, [POS.0, POS.1, POS.2]).is_some(),
                "a parked level's block entities still read"
            );
            assert_eq!(
                ask(game, &mut cache),
                (false, start),
                "parked: as a fresh lookup finds"
            );
            game.enter_level(Dim::Overworld).unwrap();
            assert_eq!(
                ask(game, &mut cache),
                (true, start + 1),
                "in place again: asked again"
            );
        },
    );
}

#[test]
#[ignore = "daily"]
fn a_cache_first_asked_while_its_level_is_parked_asks_again_once_in_place() {
    with_cache_mod(
        "game::native_capability_cache_tests::a_cache_first_asked_while_its_level_is_parked_asks_again_once_in_place",
        |game, level| {
            let mut cache = cache(level);
            let start = ASKED.load(Ordering::Relaxed);
            game.enter_level(Dim::Nether).unwrap();
            assert_eq!(
                ask(game, &mut cache),
                (false, start),
                "parked: no provider is asked"
            );
            game.enter_level(Dim::Overworld).unwrap();
            assert_eq!(
                ask(game, &mut cache),
                (true, start + 1),
                "the same entity and state, now in place"
            );
        },
    );
}

#[test]
#[ignore = "daily"]
fn a_cache_asks_again_when_the_block_state_changes() {
    with_cache_mod(
        "game::native_capability_cache_tests::a_cache_asks_again_when_the_block_state_changes",
        |game, level| {
            let mut cache = cache(level);
            let start = ask(game, &mut cache).1;
            let granite = game
                .pack
                .assets
                .registry
                .default_state("minecraft:granite")
                .unwrap();
            game.world.set(POS.0, POS.1, POS.2, granite).unwrap();
            assert!(
                game.world.block_entities.get(&POS).is_some(),
                "the block entity stays"
            );
            assert_eq!(ask(game, &mut cache), (true, start + 1));
            assert_eq!(ask(game, &mut cache), (true, start + 1));
        },
    );
}

#[test]
#[ignore = "daily"]
fn a_cache_follows_its_block_entity_removed_and_replaced() {
    with_cache_mod(
        "game::native_capability_cache_tests::a_cache_follows_its_block_entity_removed_and_replaced",
        |game, level| {
            let mut cache = cache(level);
            let start = ask(game, &mut cache).1;
            game.world.block_entities.remove(&POS);
            assert_eq!(
                ask(game, &mut cache),
                (false, start),
                "removed: nothing there"
            );
            place_holder(game, "minecraft:stone");
            assert_eq!(
                ask(game, &mut cache),
                (true, start + 1),
                "a new block entity: asked again"
            );
            // Replaced by another holder of the same type: a new block entity, a new answer.
            game.world.block_entities.remove(&POS);
            place_holder(game, "minecraft:stone");
            assert_eq!(ask(game, &mut cache), (true, start + 2));
        },
    );
}

#[test]
#[ignore = "daily"]
fn a_cache_finds_nothing_while_its_chunk_is_unloaded() {
    with_cache_mod(
        "game::native_capability_cache_tests::a_cache_finds_nothing_while_its_chunk_is_unloaded",
        |game, level| {
            let mut cache = cache(level);
            let start = ask(game, &mut cache).1;
            game.world.chunks.remove(&(0, 0));
            game.world
                .block_entities
                .unload(|pos| (pos.0 >> 4, pos.2 >> 4) == (0, 0));
            assert_eq!(
                ask(game, &mut cache),
                (false, start),
                "unloaded: nothing there"
            );
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            place_holder(game, "minecraft:stone");
            assert_eq!(
                ask(game, &mut cache),
                (true, start + 1),
                "reloaded: asked again"
            );
        },
    );
}

#[test]
#[ignore = "daily"]
fn a_cache_finds_nothing_once_its_level_closes() {
    with_cache_mod(
        "game::native_capability_cache_tests::a_cache_finds_nothing_once_its_level_closes",
        |game, level| {
            let mut cache = cache(level);
            assert!(ask(game, &mut cache).0);
            game.native_lifecycle
                .begin_open(mod_api::Authority::IntegratedServer);
            assert!(
                !ask(game, &mut cache).0,
                "a closed level's handles are stale"
            );
        },
    );
}

#[test]
#[ignore = "daily"]
fn a_cache_asks_again_when_a_mod_is_switched_off_or_on() {
    with_cache_mod(
        "game::native_capability_cache_tests::a_cache_asks_again_when_a_mod_is_switched_off_or_on",
        |game, level| {
            let mut cache = cache(level);
            let start = ask(game, &mut cache).1;
            assert!(mod_api::__private::set_mod_enabled("cache_bench", false));
            assert_eq!(ask(game, &mut cache), (false, start), "its provider is off");
            assert!(mod_api::__private::set_mod_enabled("cache_bench", true));
            assert_eq!(
                ask(game, &mut cache),
                (true, start + 1),
                "back on: asked again"
            );
            assert!(mod_api::__private::set_mod_enabled("cache_bench", true));
            assert_eq!(
                ask(game, &mut cache),
                (true, start + 1),
                "on again changes nothing"
            );
        },
    );
}

#[test]
#[ignore = "daily"]
fn moves_through_kept_handlers_conserve_every_item_when_some_abort() {
    with_cache_mod(
        "game::native_capability_cache_tests::moves_through_kept_handlers_conserve_every_item_when_some_abort",
        |game, level| {
            let chest = game
                .pack
                .assets
                .registry
                .default_state("minecraft:chest")
                .unwrap();
            for (x, full) in [(3, true), (5, false)] {
                game.world.set(x, 64, 1, chest).unwrap();
                let mut entity = BlockEntity::for_block("minecraft:chest").unwrap();
                if full {
                    for (slot, stack) in entity.slots_mut().iter_mut().enumerate() {
                        *stack = Some(ItemStack::new(
                            ["minecraft:stone", "minecraft:dirt"][slot % 2],
                            64,
                        ));
                    }
                }
                game.world.block_entities.insert((x, 64, 1), entity);
            }
            let mut from = BlockCapabilityCache::new(&ITEMS, level, [3, 64, 1], None);
            let mut to = BlockCapabilityCache::new(&ITEMS, level, [5, 64, 1], None);
            let count = |game: &super::Game<'_>| {
                [(3, 64, 1), (5, 64, 1)]
                    .iter()
                    .flat_map(|pos| game.world.block_entities.get(pos).unwrap().slots().to_vec())
                    .flatten()
                    .map(|stack| u64::from(stack.count))
                    .sum::<u64>()
            };
            let before = count(game);
            // Where each cache's kept handler lives: an aborted move must not make it look again.
            let kept = |cache: &mut BlockCapabilityCache<dyn ResourceHandler<ItemResource>>,
                        game: &mut super::Game<'_>| {
                cache.get(game).map(|handler| {
                    &**handler as *const dyn ResourceHandler<ItemResource> as *const ()
                })
            };
            let kept_before = (kept(&mut from, game), kept(&mut to, game));
            let entities = |game: &super::Game<'_>| {
                [[3, 64, 1], [5, 64, 1]].map(|pos| game.block_entity(level, pos))
            };
            let entities_before = entities(game);
            let (mut moved, mut aborted) = (0, 0);
            for round in 0..300u32 {
                // Each cache lends its own handler; the game is free for the move.
                let source = from.get(game).expect("the source chest answers");
                let sink = to.get(game).expect("the sink chest answers");
                let abort = round.is_multiple_of(3);
                let result = (game as &mut dyn ServerHost).transaction(|host, tx| {
                    let n = mod_api::move_resources(host, source, sink, |_| true, 7, tx)
                        .map_err(|_| 0u32)?;
                    if abort && n > 0 { Err(n) } else { Ok(n) }
                });
                match result {
                    Ok(n) => moved += n,
                    Err(n) if n > 0 => aborted += 1,
                    Err(_) => {}
                }
                assert_eq!(count(game), before, "round {round}");
                assert_eq!(
                    entities(game),
                    entities_before,
                    "round {round}: the same block entities"
                );
                assert_eq!(
                    (kept(&mut from, game), kept(&mut to, game)),
                    kept_before,
                    "round {round}: kept, not looked up again"
                );
            }
            assert!(moved > 0 && aborted > 0);
        },
    );
}

#[test]
#[ignore = "daily"]
fn a_providers_invalidation_counts_once_after_its_commit_and_not_when_aborted() {
    with_cache_mod(
        "game::native_capability_cache_tests::a_providers_invalidation_counts_once_after_its_commit_and_not_when_aborted",
        |game, level| {
            let pos = [POS.0, POS.1, POS.2];
            let mut cache = cache(level);
            SLOTS.store(1, Ordering::Relaxed);
            let start = ask(game, &mut cache).1;
            SLOTS.store(2, Ordering::Relaxed);
            let aborted = (game as &mut dyn ServerHost).transaction(|host, _| {
                assert!(level.invalidate_capabilities(host, pos));
                assert_eq!(
                    cache.get(host).map(|h| h.size(host)),
                    Some(1),
                    "not before the commit"
                );
                Err::<(), ()>(())
            });
            assert!(aborted.is_err());
            assert_eq!(
                slots(game, &mut cache),
                Some(1),
                "an aborted invalidation changes nothing"
            );
            assert_eq!(ask(game, &mut cache), (true, start));
            let committed = (game as &mut dyn ServerHost).transaction(|host, _| {
                assert!(level.invalidate_capabilities(host, pos));
                assert!(level.invalidate_capabilities(host, pos));
                assert_eq!(
                    cache.get(host).map(|h| h.size(host)),
                    Some(1),
                    "not before the commit"
                );
                Ok::<(), ()>(())
            });
            assert!(committed.is_ok());
            assert_eq!(
                slots(game, &mut cache),
                Some(2),
                "the provider's new answer"
            );
            assert_eq!(
                ask(game, &mut cache),
                (true, start + 1),
                "asked once for both"
            );
            assert_eq!(ask(game, &mut cache), (true, start + 1));
        },
    );
}

#[test]
#[ignore = "daily"]
fn a_block_invalidated_before_its_chunk_unloads_is_asked_again_after_it_reloads() {
    with_cache_mod(
        "game::native_capability_cache_tests::a_block_invalidated_before_its_chunk_unloads_is_asked_again_after_it_reloads",
        |game, level| {
            let pos = [8, 64, 8];
            let lodestone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:lodestone")
                .unwrap();
            game.world.set(pos[0], pos[1], pos[2], lodestone).unwrap();
            let mut cache = BlockCapabilityCache::new(&COUNTED, level, pos, None);
            let start = ASKED.load(Ordering::Relaxed);
            assert_eq!(
                ask(game, &mut cache),
                (true, start + 1),
                "a block-only answer"
            );
            assert!(level.invalidate_capabilities(game, pos));
            // The chunk leaves before the cache is asked again, taking the invalidation with it.
            game.world.unload_chunk((0, 0));
            assert!(
                !level.invalidate_capabilities(game, pos),
                "an unloaded position refuses"
            );
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            game.world.set(pos[0], pos[1], pos[2], lodestone).unwrap();
            assert!(
                game.block_entity(level, pos).is_none(),
                "nothing but the block to compare"
            );
            assert_eq!(
                ask(game, &mut cache),
                (true, start + 2),
                "asked again, not a stale hit"
            );
            assert_eq!(ask(game, &mut cache), (true, start + 2));
        },
    );
}
