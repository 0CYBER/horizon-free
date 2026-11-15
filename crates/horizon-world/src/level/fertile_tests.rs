//! `IBlockExtension.isFertile` in `CropBlock.getGrowthSpeed` and `canBeHydrated` in
//! `FarmlandBlock.randomTick`, over an in-memory level. Needs the prepared client resources
//! (skipped when absent), as `tests` does.
use super::tests::{TestLevel, fixture};
use super::*;
use std::cell::{Cell, RefCell};

/// A level whose soil and farmland parts answer, recording what they were asked.
struct Growth {
    inner: TestLevel,
    b: &'static Behaviours,
    /// A soil block and the `fertile` it sets.
    fertile: Option<(BlockId, bool)>,
    fertile_asked: RefCell<Vec<BlockId>>,
    /// A fluid kind and the farmland's answer for it.
    hydrates: Option<(FluidKind, bool)>,
    hydrate_asked: Cell<u32>,
}
impl BlockEntityNeighbors for Growth {}

impl LevelAccess for Growth {
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
    fn emit(&mut self, event: LevelEvent) {
        self.inner.emit(event);
    }
    fn asks_is_fertile(&self) -> bool {
        true
    }
    fn before_is_fertile(&self, state: BlockStateId, _pos: BlockPos, fertile: &mut bool) {
        let block = self.b.block(state);
        self.fertile_asked.borrow_mut().push(block);
        if let Some((soil, answer)) = self.fertile
            && soil == block
        {
            *fertile = answer;
        }
    }
    fn asks_can_be_hydrated(&self, _state: BlockStateId) -> bool {
        true
    }
    fn before_can_be_hydrated(
        &self,
        _state: BlockStateId,
        _pos: BlockPos,
        fluid: FluidState,
        _fluid_pos: BlockPos,
        hydrates: &mut Option<bool>,
    ) {
        self.hydrate_asked.set(self.hydrate_asked.get() + 1);
        if let Some((kind, answer)) = self.hydrates
            && fluid.kind() == kind
        {
            *hydrates = Some(answer);
        }
    }
}

fn level(b: &'static Behaviours) -> (Growth, Ticks) {
    let mut ticks = Ticks::new(0);
    for x in -1..=1 {
        for z in -1..=1 {
            ticks.add_empty_chunk(ChunkPos::new(x, z)).unwrap();
        }
    }
    let level = Growth {
        inner: TestLevel::new(b),
        b,
        fertile: None,
        fertile_asked: RefCell::new(Vec::new()),
        hydrates: None,
        hydrate_asked: Cell::new(0),
    };
    (level, ticks)
}

#[test]
fn a_soils_fertile_answer_sets_the_crops_growth_speed() {
    let Some(f) = fixture() else { return };
    let b = &f.behaviours;
    let state = |name: &str| f.registry.default_state(name).unwrap();
    let farmland = |moisture: i32| {
        b.table
            .set_int(state("minecraft:farmland"), b.props.moisture, moisture)
    };
    let (wheat, stone) = (state("minecraft:wheat"), state("minecraft:stone"));
    let (mut level, mut ticks) = level(b);
    let crop = BlockPos::new(0, 2, 0);
    let under = BlockPos::new(0, 1, 0);
    let ground = |level: &mut Growth, centre: BlockPos, soil: BlockStateId, all: bool| {
        for x in -1..=1 {
            for z in -1..=1 {
                let at = centre.offset(x, 0, z);
                level.set_raw(at, if all || at == centre { soil } else { stone });
            }
        }
    };
    level.set_raw(crop, wheat);
    let speed = |level: &mut Growth, ticks: &mut Ticks| {
        b.sim(level, ticks).crop_growth_speed(b.block(wheat), crop)
    };

    // One farmland under the wheat, stone around it: 1 + 1 dry, 1 + 3 moist.
    ground(&mut level, under, farmland(0), false);
    assert_eq!(speed(&mut level, &mut ticks), 2.0);
    ground(&mut level, under, farmland(7), false);
    assert_eq!(speed(&mut level, &mut ticks), 4.0);
    // Only the tagged soil is asked, never the stone around it.
    assert!(
        level
            .fertile_asked
            .borrow()
            .iter()
            .all(|&block| block == b.block(farmland(0)))
    );
    assert_eq!(level.fertile_asked.borrow().len(), 2);

    // A part setting true makes dry farmland count as moist.
    level.fertile = Some((b.block(farmland(0)), true));
    ground(&mut level, under, farmland(0), false);
    assert_eq!(speed(&mut level, &mut ticks), 4.0);

    // Under nine moist farmland, a part setting false: 1 + 1 + 8·¼ instead of 1 + 3 + 8·¾.
    level.fertile = None;
    ground(&mut level, under, farmland(7), true);
    assert_eq!(speed(&mut level, &mut ticks), 10.0);
    level.fertile = Some((b.block(farmland(7)), false));
    assert_eq!(speed(&mut level, &mut ticks), 4.0);

    // A part on stone under the crop is never asked: stone isn't in #grows_crops.
    level.fertile = Some((b.block(stone), true));
    level.fertile_asked.borrow_mut().clear();
    ground(&mut level, under, stone, true);
    assert_eq!(speed(&mut level, &mut ticks), 1.0);
    assert!(level.fertile_asked.borrow().is_empty());
}

#[test]
fn a_farmlands_hydrated_answer_picks_the_fluids_that_water_it() {
    let Some(f) = fixture() else { return };
    let b = &f.behaviours;
    let state = |name: &str| f.registry.default_state(name).unwrap();
    let farmland = |moisture: i32| {
        b.table
            .set_int(state("minecraft:farmland"), b.props.moisture, moisture)
    };
    let (stone, lava, water) = (
        state("minecraft:stone"),
        state("minecraft:lava"),
        state("minecraft:water"),
    );
    let pos = BlockPos::new(0, 1, 0);
    let moisture = |level: &Growth| b.table.int(level.block_state(pos), b.props.moisture);
    // A source in its own stone cell, so nothing flows during the test.
    let cell = |level: &mut Growth, at: BlockPos, fluid: BlockStateId| {
        for d in [
            (1, 0, 0),
            (-1, 0, 0),
            (0, 1, 0),
            (0, -1, 0),
            (0, 0, 1),
            (0, 0, -1),
        ] {
            level.set_raw(at.offset(d.0, d.1, d.2), stone);
        }
        level.set_raw(at, fluid);
    };
    let tick = |level: &mut Growth, ticks: &mut Ticks| {
        level.set_raw(pos, farmland(7));
        b.random_tick(level, ticks, pos);
    };

    // Lava at (+4, +1, 0), no water, no rain: the farmland dries to 6.
    let (mut level, mut ticks) = level(b);
    cell(&mut level, pos.offset(4, 1, 0), lava);
    tick(&mut level, &mut ticks);
    assert_eq!(moisture(&level), 6);
    assert_eq!(level.hydrate_asked.get(), 1, "asked once, at the lava");
    // A part answering Some(true) for lava keeps it at 7.
    level.hydrates = Some((FluidKind::Lava, true));
    tick(&mut level, &mut ticks);
    assert_eq!(moisture(&level), 7);

    // The lava at (+5, 0, 0), outside the box: never asked, so it dries to 6.
    let (mut level, mut ticks) = self::level(b);
    level.hydrates = Some((FluidKind::Lava, true));
    cell(&mut level, pos.offset(5, 0, 0), lava);
    tick(&mut level, &mut ticks);
    assert_eq!(moisture(&level), 6);
    assert_eq!(level.hydrate_asked.get(), 0);

    // Beside water, a part answering Some(false) makes it dry to 6.
    let (mut level, mut ticks) = self::level(b);
    cell(&mut level, pos.offset(2, 0, 0), water);
    tick(&mut level, &mut ticks);
    assert_eq!(moisture(&level), 7, "vanilla: water keeps it moist");
    level.hydrates = Some((FluidKind::Water, false));
    tick(&mut level, &mut ticks);
    assert_eq!(moisture(&level), 6);
}
