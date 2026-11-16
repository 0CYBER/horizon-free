use super::tests::{Fixture, TestLevel, fixture};
use super::*;
use std::cell::RefCell;
use std::collections::HashMap;

/// A target ask: the kind, the target's position and state, the connector and the way from the
/// target to it.
type Ask = (
    ConnectionKind,
    BlockPos,
    BlockStateId,
    BlockStateId,
    Direction,
);

/// A level whose block parts answer connections: the target's (`adjust_can_be_connected_to`)
/// by its state, then the connector's (`adjust_connects_to`) by its own.
struct ConnectionLevel {
    inner: TestLevel,
    /// What a target state's part answers.
    targets: HashMap<BlockStateId, bool>,
    /// What a connector state's part answers, whatever it was handed.
    connectors: HashMap<BlockStateId, bool>,
    asks: RefCell<Vec<Ask>>,
}
impl crate::level::BlockEntityNeighbors for ConnectionLevel {}
impl LevelAccess for ConnectionLevel {
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
    fn has_connection_hook(&self) -> bool {
        true
    }
    fn adjust_can_be_connected_to(
        &self,
        kind: ConnectionKind,
        pos: BlockPos,
        state: BlockStateId,
        connector: BlockStateId,
        direction: Direction,
        _face_solid: bool,
        vanilla: bool,
    ) -> bool {
        self.asks
            .borrow_mut()
            .push((kind, pos, state, connector, direction));
        self.targets.get(&state).copied().unwrap_or(vanilla)
    }
    fn adjust_connects_to(
        &self,
        _kind: ConnectionKind,
        _pos: BlockPos,
        state: BlockStateId,
        _neighbour: BlockStateId,
        _direction: Direction,
        _face_solid: bool,
        vanilla: bool,
    ) -> bool {
        self.connectors.get(&state).copied().unwrap_or(vanilla)
    }
}

fn state(f: &Fixture, name: &str) -> BlockStateId {
    f.registry
        .default_state(&format!("minecraft:{name}"))
        .unwrap_or_else(|| panic!("unknown block {name}"))
}

/// The side property `side` of the block at `pos`.
fn prop(f: &Fixture, level: &ConnectionLevel, pos: BlockPos, side: &str) -> String {
    f.registry.state(level.block_state(pos)).unwrap().properties[side].clone()
}

#[test]
fn the_target_answers_a_connection_before_the_connectors_last_word() {
    let Some(f) = fixture() else {
        return;
    };
    let b = &f.behaviours;
    let [fence, wall, pane, lantern, stone, soul_lantern] = [
        "oak_fence",
        "cobblestone_wall",
        "glass_pane",
        "lantern",
        "stone",
        "soul_lantern",
    ]
    .map(|name| state(f, name));
    let mut level = ConnectionLevel {
        inner: TestLevel::new(b),
        // A lantern's part connects; a stone's refuses.
        targets: [(lantern, true), (stone, false)].into(),
        connectors: HashMap::new(),
        asks: RefCell::new(Vec::new()),
    };
    let mut ticks = Ticks::new(0);
    // Each connector at x 0 with its target placed east of it at x 1, on its own z.
    let mut place = |level: &mut ConnectionLevel, connector, target, z| {
        let (at, east) = (BlockPos::new(0, 1, z), BlockPos::new(1, 1, z));
        b.set_block(level, &mut ticks, at, connector, flags::UPDATE_ALL);
        b.set_block(level, &mut ticks, east, target, flags::UPDATE_ALL);
        prop(f, level, at, "east")
    };
    // A fence beside the lantern connects, beside a soul lantern (no part) it does not.
    assert_eq!(place(&mut level, fence, lantern, 0), "true");
    assert_eq!(
        *level.asks.borrow().last().unwrap(),
        (
            ConnectionKind::Fence,
            BlockPos::new(1, 1, 0),
            lantern,
            fence,
            Direction::West
        )
    );
    assert_eq!(place(&mut level, fence, soul_lantern, 2), "false");
    // Stone's sturdy face is refused.
    assert_eq!(place(&mut level, fence, stone, 4), "false");
    // A wall beside the lantern gets a low side; bars and panes connect too.
    assert_eq!(place(&mut level, wall, lantern, 6), "low");
    assert_eq!(place(&mut level, pane, lantern, 8), "true");
    // The connector's own rule has the last word: a fence that refuses stays unconnected.
    level.connectors.insert(fence, false);
    assert_eq!(place(&mut level, fence, lantern, 10), "false");
}
