//! Pistons carry a mod block entity while its block moves (arismoko/horizon#3184), when the
//! block's `piston_carries_block_entity` lets it move. The level holds the block entities;
//! these tests stand in for it with a map. They need the prepared resources,
//! as the other level tests do.
use super::tests::{Fixture, TestLevel, fixture};
use super::*;
use crate::chunk::BlockState;
use std::collections::HashMap;

/// A mod block entity's contents: diamonds in its slots and one attachment.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Contents {
    diamonds: u32,
    attachment: i32,
}

struct CarryLevel {
    inner: TestLevel,
    /// What the block's part answers to `piston_carries_block_entity`.
    carries: fn(BlockPos) -> Option<bool>,
    entities: HashMap<BlockPos, Contents>,
    carried: HashMap<BlockPos, Contents>,
    /// The diamonds each removal with side effects dropped, and where.
    dropped: Vec<(BlockPos, u32)>,
}
impl BlockEntityNeighbors for CarryLevel {}
impl LevelAccess for CarryLevel {
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
        self.inner.time
    }
    fn has_mod_block_entity(&self, pos: BlockPos) -> bool {
        self.entities.contains_key(&pos)
    }
    fn adjust_piston_carries_block_entity(
        &self,
        pos: BlockPos,
        _state: BlockStateId,
    ) -> Option<bool> {
        (self.carries)(pos)
    }
    fn carry_block_entity(&mut self, from: BlockPos, to: BlockPos) {
        let contents = self
            .entities
            .remove(&from)
            .expect("a block entity to carry");
        assert!(self.carried.insert(to, contents).is_none());
    }
    fn land_block_entity(&mut self, pos: BlockPos) -> bool {
        let Some(contents) = self.carried.remove(&pos) else {
            return false;
        };
        assert!(self.entities.insert(pos, contents).is_none());
        true
    }
    fn discard_carried_block_entity(&mut self, pos: BlockPos) {
        self.carried.remove(&pos);
    }
    fn remove_block_entity(&mut self, pos: BlockPos, _old: BlockStateId, side_effects: bool) -> u8 {
        if let Some(contents) = self.entities.remove(&pos)
            && side_effects
        {
            self.dropped.push((pos, contents.diamonds));
        }
        0
    }
}

struct World {
    f: &'static Fixture,
    level: CarryLevel,
    ticks: Ticks,
}
impl World {
    fn new(f: &'static Fixture, carries: fn(BlockPos) -> Option<bool>) -> Self {
        let mut ticks = Ticks::new(0);
        for x in -4..4 {
            for z in -4..4 {
                ticks.add_empty_chunk(ChunkPos::new(x, z)).unwrap();
            }
        }
        let mut w = Self {
            level: CarryLevel {
                inner: TestLevel::new(&f.behaviours),
                carries,
                entities: HashMap::new(),
                carried: HashMap::new(),
                dropped: Vec::new(),
            },
            f,
            ticks,
        };
        let stone = w.id("stone", &[]);
        for x in -8..=8 {
            for z in -8..=8 {
                w.level.set_raw(BlockPos::new(x, 0, z), stone);
            }
        }
        w
    }
    fn id(&self, name: &str, props: &[(&str, &str)]) -> BlockStateId {
        let state = BlockState {
            name: format!("minecraft:{name}"),
            properties: props
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        };
        self.f.registry.resolve(&state).unwrap()
    }
    fn set(&mut self, pos: BlockPos, state: BlockStateId) {
        self.f.behaviours.set_block(
            &mut self.level,
            &mut self.ticks,
            pos,
            state,
            flags::UPDATE_ALL,
        );
    }
    fn get(&self, pos: BlockPos) -> BlockStateId {
        self.level.block_state(pos)
    }
    /// Scheduled ticks, block events and block entities, as `ServerLevel.tick`.
    fn full_tick(&mut self) {
        let b = &self.f.behaviours;
        self.level.inner.time += 1;
        b.tick_scheduled(&mut self.level, &mut self.ticks);
        b.run_block_events(&mut self.level, &mut self.ticks);
        b.tick_block_entities(&mut self.level, &mut self.ticks);
    }
    fn progress(&self, pos: BlockPos) -> Option<f32> {
        self.ticks
            .moving_pistons
            .iter()
            .find(|(p, _)| *p == pos)
            .map(|(_, piston)| piston.progress)
    }
}
macro_rules! carry_world {
    ($carries:expr) => {{
        let Some(f) = fixture() else {
            return;
        };
        World::new(f, $carries)
    }};
}

const PISTON: BlockPos = BlockPos::new(0, 1, 0);
const POWER: BlockPos = BlockPos::new(-1, 1, 0);
const BLOCK: BlockPos = BlockPos::new(1, 1, 0);
const PUSHED: BlockPos = BlockPos::new(2, 1, 0);
const CONTENTS: Contents = Contents {
    diamonds: 3,
    attachment: 7,
};

/// A sticky piston facing east at `PISTON`, and an iron block standing for a
/// mod's block at `BLOCK` holding `CONTENTS`.
fn sticky_piston_and_block(w: &mut World) -> BlockStateId {
    let piston = w.id("sticky_piston", &[("facing", "east")]);
    let block = w.id("iron_block", &[]);
    w.level.set_raw(PISTON, piston);
    w.level.set_raw(BLOCK, block);
    w.level.entities.insert(BLOCK, CONTENTS);
    block
}

#[test]
fn an_allowed_block_entity_moves_with_its_block_and_back() {
    let mut w = carry_world!(|_| Some(true));
    let block = sticky_piston_and_block(&mut w);
    w.set(POWER, w.id("redstone_block", &[]));
    for _ in 0..4 {
        w.full_tick();
    }
    assert_eq!(w.get(PUSHED), block);
    assert_eq!(w.level.entities.get(&PUSHED), Some(&CONTENTS));
    assert!(!w.level.entities.contains_key(&BLOCK));
    assert!(w.level.carried.is_empty());
    assert!(w.level.dropped.is_empty(), "the move drops nothing");

    w.set(POWER, w.f.behaviours.air());
    for _ in 0..4 {
        w.full_tick();
    }
    assert_eq!(w.get(BLOCK), block, "the sticky piston pulled it back");
    assert_eq!(w.level.entities.get(&BLOCK), Some(&CONTENTS));
    assert_eq!(w.level.entities.len(), 1);
    assert!(w.level.dropped.is_empty());
}

#[test]
fn without_an_answer_a_block_entity_keeps_the_piston_retracted() {
    let mut w = carry_world!(|_| None);
    let block = sticky_piston_and_block(&mut w);
    w.set(POWER, w.id("redstone_block", &[]));
    for _ in 0..4 {
        w.full_tick();
    }
    let extended = w.f.behaviours.props.extended;
    assert!(!w.f.behaviours.table.bool(w.get(PISTON), extended));
    assert_eq!(w.get(BLOCK), block);
    assert_eq!(w.level.entities.get(&BLOCK), Some(&CONTENTS));
}

#[test]
fn a_refusal_holds_a_block_at_its_position_only() {
    // The answer is the part's for one position: y 1 refuses, y 2 says nothing.
    let mut w = carry_world!(|pos| (pos.y == 1).then_some(false));
    let piston = w.id("piston", &[("facing", "east")]);
    let dirt = w.id("dirt", &[]);
    let power = w.id("redstone_block", &[]);
    let lower = (PISTON, BLOCK, POWER);
    let upper = (
        BlockPos::new(0, 2, 5),
        BlockPos::new(1, 2, 5),
        BlockPos::new(-1, 2, 5),
    );
    for (at, block, _) in [lower, upper] {
        w.level.set_raw(at, piston);
        w.level.set_raw(block, dirt);
    }
    w.set(lower.2, power);
    w.set(upper.2, power);
    for _ in 0..4 {
        w.full_tick();
    }
    assert_eq!(w.get(BLOCK), dirt, "the refused dirt stays");
    assert_eq!(w.get(BlockPos::new(2, 2, 5)), dirt, "the other dirt moved");
}

#[test]
fn replacing_the_moving_block_drops_its_block_entitys_contents_once() {
    let mut w = carry_world!(|_| Some(true));
    sticky_piston_and_block(&mut w);
    w.set(POWER, w.id("redstone_block", &[]));
    for _ in 0..4 {
        if w.progress(PUSHED) == Some(0.5) {
            break;
        }
        w.full_tick();
    }
    assert_eq!(w.progress(PUSHED), Some(0.5));
    assert_eq!(w.level.carried.get(&PUSHED), Some(&CONTENTS));

    // `/setblock` over the moving block.
    w.set(PUSHED, w.id("stone", &[]));
    for _ in 0..4 {
        w.full_tick();
    }
    assert_eq!(w.level.dropped, [(PUSHED, 3)]);
    assert!(w.level.entities.is_empty());
    assert!(w.level.carried.is_empty());
}

#[test]
fn power_lost_mid_extension_leaves_one_block_entity_where_its_block_ends() {
    let mut w = carry_world!(|_| Some(true));
    let block = sticky_piston_and_block(&mut w);
    w.set(POWER, w.id("redstone_block", &[]));
    for _ in 0..4 {
        if w.progress(PUSHED) == Some(0.5) {
            break;
        }
        w.full_tick();
    }
    assert_eq!(w.progress(PUSHED), Some(0.5));

    // The retraction finishes the move first; wherever the block ends, its
    // block entity is there, once, and nothing drops.
    w.set(POWER, w.f.behaviours.air());
    for _ in 0..6 {
        w.full_tick();
    }
    let at = [BLOCK, PUSHED]
        .into_iter()
        .find(|p| w.get(*p) == block)
        .expect("the block lands");
    assert_eq!(w.level.entities.len(), 1);
    assert_eq!(w.level.entities.get(&at), Some(&CONTENTS));
    assert!(w.level.carried.is_empty());
    assert!(w.level.dropped.is_empty());
}

#[test]
fn a_block_stuck_to_slime_takes_its_block_entity_along() {
    let mut w = carry_world!(|_| Some(true));
    let (piston, slime, power) = (
        BlockPos::new(0, 3, 0),
        BlockPos::new(1, 3, 0),
        BlockPos::new(-1, 3, 0),
    );
    let (stuck, landed) = (BlockPos::new(1, 4, 0), BlockPos::new(2, 4, 0));
    let block = w.id("iron_block", &[]);
    let (piston_state, slime_state) = (
        w.id("piston", &[("facing", "east")]),
        w.id("slime_block", &[]),
    );
    w.level.set_raw(piston, piston_state);
    w.level.set_raw(slime, slime_state);
    w.level.set_raw(stuck, block);
    w.level.entities.insert(stuck, CONTENTS);
    w.set(power, w.id("redstone_block", &[]));
    for _ in 0..4 {
        w.full_tick();
    }
    assert_eq!(w.get(landed), block);
    assert_eq!(w.level.entities.get(&landed), Some(&CONTENTS));
    assert_eq!(w.level.entities.len(), 1);
    assert!(w.level.carried.is_empty());
    assert!(w.level.dropped.is_empty());
}

#[test]
fn a_block_stuck_to_honey_takes_its_block_entity_along() {
    let mut w = carry_world!(|_| Some(true));
    let (piston, honey, power) = (
        BlockPos::new(0, 3, 0),
        BlockPos::new(1, 3, 0),
        BlockPos::new(-1, 3, 0),
    );
    let (stuck, landed) = (BlockPos::new(1, 4, 0), BlockPos::new(2, 4, 0));
    let block = w.id("iron_block", &[]);
    let (piston_state, honey_state) = (
        w.id("piston", &[("facing", "east")]),
        w.id("honey_block", &[]),
    );
    w.level.set_raw(piston, piston_state);
    w.level.set_raw(honey, honey_state);
    w.level.set_raw(stuck, block);
    w.level.entities.insert(stuck, CONTENTS);
    w.set(power, w.id("redstone_block", &[]));
    for _ in 0..4 {
        w.full_tick();
    }
    assert_eq!(w.get(landed), block);
    assert_eq!(w.level.entities.get(&landed), Some(&CONTENTS));
    assert_eq!(w.level.entities.len(), 1);
    assert!(w.level.carried.is_empty());
    assert!(w.level.dropped.is_empty());
}
