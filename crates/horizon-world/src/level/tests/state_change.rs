use super::tests::{Fixture, TestLevel, fixture};
use super::*;
use std::cell::RefCell;

/// A level that records, at each moment, the state change and the east side of the fence at
/// the origin's west.
struct StateChangeLevel {
    inner: TestLevel,
    fence: BlockPos,
    /// `observe_block_changed`: the change and the fence's east side then.
    changed: RefCell<Vec<(BlockStateId, BlockStateId, String)>>,
    /// `observe_block_state_change`: the change and the fence's east side then.
    state_changes: RefCell<Vec<(BlockStateId, BlockStateId, String)>>,
    /// The east side of the fence at `beside`, at each `observe_block_state_change`.
    beside: BlockPos,
    beside_east: RefCell<Vec<String>>,
    /// `observe_block_state_change`: the x of the position and the new state, in the order
    /// heard.
    heard: RefCell<Vec<(i32, BlockStateId)>>,
    registry: &'static BlockRegistry,
}
impl StateChangeLevel {
    fn east(&self) -> String {
        self.east_at(self.fence)
    }
    fn east_at(&self, pos: BlockPos) -> String {
        self.registry
            .state(self.block_state(pos))
            .and_then(|s| s.properties.get("east").cloned())
            .unwrap_or_default()
    }
}
impl crate::level::BlockEntityNeighbors for StateChangeLevel {}
impl LevelAccess for StateChangeLevel {
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
    fn observe_block_changed(&self, _pos: BlockPos, old: BlockStateId, new: BlockStateId) {
        let east = self.east();
        self.changed.borrow_mut().push((old, new, east));
    }
    fn observes_block_state_change(&self, _old: BlockStateId, _new: BlockStateId) -> bool {
        true
    }
    fn observe_block_state_change(&self, pos: BlockPos, old: BlockStateId, new: BlockStateId) {
        self.heard.borrow_mut().push((pos.x, new));
        let east = self.east();
        self.state_changes.borrow_mut().push((old, new, east));
        let beside = self.east_at(self.beside);
        self.beside_east.borrow_mut().push(beside);
    }
}

fn state(f: &Fixture, name: &str) -> BlockStateId {
    f.registry
        .default_state(&format!("minecraft:{name}"))
        .unwrap_or_else(|| panic!("unknown block {name}"))
}

#[test]
fn a_state_change_is_heard_after_the_neighbours_settled_and_only_when_the_write_held() {
    let Some(f) = fixture() else {
        return;
    };
    let b = &f.behaviours;
    let registry: &'static BlockRegistry = &f.registry;
    let mut level = StateChangeLevel {
        inner: TestLevel::new(b),
        fence: BlockPos::new(-1, 1, 0),
        changed: RefCell::new(Vec::new()),
        state_changes: RefCell::new(Vec::new()),
        beside: BlockPos::new(-2, 1, 0),
        beside_east: RefCell::new(Vec::new()),
        heard: RefCell::new(Vec::new()),
        registry,
    };
    let mut ticks = Ticks::new(0);
    let [fence, stone, sponge, wet_sponge, water] =
        ["oak_fence", "stone", "sponge", "wet_sponge", "water"].map(|name| state(f, name));
    let air = b.air();
    level.inner.set_raw(BlockPos::new(-1, 1, 0), fence);
    // Stone east of the fence, flags 3: `block_changed` hears it before the fence connects,
    // the state change after.
    let origin = BlockPos::new(0, 1, 0);
    assert!(b.set_block(&mut level, &mut ticks, origin, stone, flags::UPDATE_ALL));
    assert_eq!(
        level.changed.borrow().first(),
        Some(&(air, stone, "false".to_owned()))
    );
    // The fence's own shape update is a write inside the stone's, heard first.
    let connected = level.block_state(BlockPos::new(-1, 1, 0));
    assert_eq!(
        *level.state_changes.borrow(),
        [
            (fence, connected, "true".to_owned()),
            (air, stone, "true".to_owned())
        ]
    );
    // Writing the same state again does not hold, and is not heard.
    assert!(!b.set_block(&mut level, &mut ticks, origin, stone, flags::UPDATE_ALL));
    assert_eq!(level.state_changes.borrow().len(), 2);
    // A sponge beside water turns wet in its `on_place`: the sponge's own write is not heard,
    // the write that replaced it is.
    level.state_changes.borrow_mut().clear();
    level.inner.set_raw(BlockPos::new(6, 1, 0), water);
    let at = BlockPos::new(5, 1, 0);
    b.set_block(&mut level, &mut ticks, at, sponge, flags::UPDATE_ALL);
    assert_eq!(level.block_state(at), wet_sponge);
    let heard: Vec<_> = level
        .state_changes
        .borrow()
        .iter()
        .map(|(old, new, _)| (*old, *new))
        .filter(|(old, new)| [*old, *new].contains(&sponge))
        .collect();
    assert_eq!(heard, [(sponge, wet_sponge)]);
}

#[test]
#[ignore = "daily"]
fn a_nested_write_is_heard_after_the_updates_it_queued() {
    let Some(f) = fixture() else {
        return;
    };
    let b = &f.behaviours;
    let registry: &'static BlockRegistry = &f.registry;
    let mut level = StateChangeLevel {
        inner: TestLevel::new(b),
        fence: BlockPos::new(-1, 1, 0),
        changed: RefCell::new(Vec::new()),
        state_changes: RefCell::new(Vec::new()),
        beside: BlockPos::new(-2, 1, 0),
        beside_east: RefCell::new(Vec::new()),
        heard: RefCell::new(Vec::new()),
        registry,
    };
    let mut ticks = Ticks::new(0);
    let [fence, stone] = ["oak_fence", "stone"].map(|name| state(f, name));
    level.inner.set_raw(BlockPos::new(-1, 1, 0), fence);
    level.inner.set_raw(BlockPos::new(-2, 1, 0), fence);
    // Stone east of the first fence: its shape update is a write inside the stone's, and that
    // write queues a shape update to the second fence, which then connects to the first.
    assert!(b.set_block(
        &mut level,
        &mut ticks,
        BlockPos::new(0, 1, 0),
        stone,
        flags::UPDATE_ALL
    ));
    assert_eq!(level.east_at(BlockPos::new(-2, 1, 0)), "true");
    // Each fence connects to the other, then the stone's own change is heard. The first
    // fence's change is heard once the second fence has settled: no notification sees the
    // second fence's east side unsettled.
    assert_eq!(level.state_changes.borrow().len(), 4);
    assert_eq!(*level.beside_east.borrow(), ["true"; 4]);
}

#[test]
#[ignore = "daily"]
fn nested_writes_are_heard_in_the_order_they_were_made() {
    let Some(f) = fixture() else {
        return;
    };
    let b = &f.behaviours;
    let registry: &'static BlockRegistry = &f.registry;
    let mut level = StateChangeLevel {
        inner: TestLevel::new(b),
        fence: BlockPos::new(-1, 1, 0),
        changed: RefCell::new(Vec::new()),
        state_changes: RefCell::new(Vec::new()),
        beside: BlockPos::new(-2, 1, 0),
        beside_east: RefCell::new(Vec::new()),
        heard: RefCell::new(Vec::new()),
        registry,
    };
    let mut ticks = Ticks::new(0);
    let [fence, stone] = ["oak_fence", "stone"].map(|name| state(f, name));
    level.inner.set_raw(BlockPos::new(-1, 1, 0), fence);
    level.inner.set_raw(BlockPos::new(-2, 1, 0), fence);
    // Stone at x 0. Written in this order: the first fence connects east (A), the second
    // fence connects east to it (B, made while A's updates run), the first connects west back
    // (C, made while B's updates run), then the stone's own write (D).
    b.set_block(
        &mut level,
        &mut ticks,
        BlockPos::new(0, 1, 0),
        stone,
        flags::UPDATE_ALL,
    );
    let heard = level.heard.borrow();
    assert_eq!(
        heard.iter().map(|(x, _)| *x).collect::<Vec<_>>(),
        [-1, -2, -1, 0]
    );
    // A is the first fence with its east side only; C is the same fence with both sides.
    let both = level.block_state(BlockPos::new(-1, 1, 0));
    assert_ne!(heard[0].1, both);
    assert_eq!(heard[2].1, both);
}
