//! Dimension-owned fire sources, including missing tags and a refreshed snapshot.
use super::tests::{Fixture, World, fixture};
use super::*;

fn tagged_fixture(stone: bool) -> Fixture {
    // Where the level tests' shared fixture finds them: `HORIZON_RESOURCES`, else the checkout's
    // prepared resources (the ignored-test shards set neither variable).
    let root = std::env::var_os("HORIZON_RESOURCES")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../.local/horizon-client")
                .join(horizon_core::VANILLA_VERSION)
        });
    let root = root.as_path();
    let registry = data::load_registry(root).unwrap();
    let mut behaviours = Behaviours::load(&registry, root).unwrap();
    let mut set = data::BlockSet::default();
    if stone {
        set.insert(behaviours.table.block_by_name("minecraft:stone").unwrap());
    }
    behaviours
        .all_tags
        .insert("mymod:eternal_flame".into(), Arc::new(set));
    Fixture {
        registry,
        behaviours,
    }
}

fn burns(
    w: &mut World<'_>,
    facts: &DimensionFacts,
    set: Option<&data::BlockSet>,
    base: &str,
) -> bool {
    let ground = BlockPos::new(0, 64, 0);
    let fire = BlockPos::new(0, 65, 0);
    w.put(ground, w.id(base, &[]));
    w.put(fire, w.id("fire", &[("age", "5")]));
    let b = w.b();
    let mut level = FireLevel {
        inner: &mut w.level,
        facts,
        set,
    };
    b.scheduled_block_tick(&mut level, &mut w.ticks, fire);
    w.b().block(w.get(fire)) == w.b().block(w.id("fire", &[]))
}

#[test]
#[ignore = "daily"]
fn custom_and_missing_dimension_infiniburn() {
    let f = tagged_fixture(true);
    let mut w = World::new(&f);
    let mut facts = DimensionFacts::overworld().clone();
    facts.infiniburn = "#mymod:eternal_flame".into();

    let set = w.b().tag("mymod:eternal_flame").unwrap().clone();

    assert!(burns(&mut w, &facts, None, "stone"));
    assert!(burns(&mut w, &facts, Some(&set), "stone"));
    assert!(!burns(&mut w, &facts, Some(&set), "netherrack"));
    facts.infiniburn = "#mymod:missing".into();

    assert!(!burns(&mut w, &facts, None, "netherrack"));
}

#[test]
#[ignore = "daily"]
fn refreshed_and_builtin_dimension_infiniburn() {
    let f = fixture().expect("prepared resources");
    let mut w = World::new(f);
    let mut custom = DimensionFacts::overworld().clone();
    custom.infiniburn = "#mymod:eternal_flame".into();
    let facts = &custom;
    let empty = data::BlockSet::default();
    assert!(!burns(&mut w, facts, Some(&empty), "stone"));
    let reloaded = tagged_fixture(true);
    let refreshed = reloaded
        .behaviours
        .tag("mymod:eternal_flame")
        .unwrap()
        .clone();
    assert!(burns(&mut w, facts, Some(&refreshed), "stone"));
    let facts = DimensionFacts::overworld();
    assert!(burns(&mut w, facts, None, "netherrack"));
    assert!(burns(&mut w, facts, None, "magma_block"));
    let nether = DimensionFacts::vanilla("minecraft:the_nether").unwrap();
    assert!(burns(&mut w, nether, None, "netherrack"));
    let end = DimensionFacts::vanilla("minecraft:the_end").unwrap();
    assert!(burns(&mut w, end, None, "bedrock"));
}

struct FireLevel<'a> {
    inner: &'a mut super::tests::TestLevel,
    facts: &'a DimensionFacts,
    set: Option<&'a data::BlockSet>,
}
impl BlockEntityNeighbors for FireLevel<'_> {}
impl LevelAccess for FireLevel<'_> {
    fn dimension_type(&self) -> &DimensionFacts {
        self.facts
    }
    fn infiniburn(&self) -> Option<&data::BlockSet> {
        self.set
    }
    fn block_state(&self, pos: BlockPos) -> BlockStateId {
        self.inner.block_state(pos)
    }
    fn set_raw(&mut self, pos: BlockPos, state: BlockStateId) -> Option<BlockStateId> {
        self.inner.set_raw(pos, state)
    }
    fn is_loaded(&self, pos: BlockPos) -> bool {
        self.inner.is_loaded(pos)
    }
    fn random(&mut self) -> &mut LegacyRandom {
        self.inner.random()
    }
    fn game_time(&self) -> i64 {
        self.inner.game_time()
    }
}
