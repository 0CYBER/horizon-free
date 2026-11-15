use super::*;

struct RemovalLevel {
    inner: TestLevel,
    answer: Option<BlockStateId>,
    asks: std::cell::RefCell<Vec<(BlockPos, BlockStateId, bool)>>,
    updates: usize,
}
impl BlockEntityNeighbors for RemovalLevel {}

impl LevelAccess for RemovalLevel {
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
        0
    }
    fn send_block_updated(&mut self, _: BlockPos) {
        self.updates += 1;
    }
    fn adjust_remove_block(
        &self,
        pos: BlockPos,
        state: BlockStateId,
        moved: bool,
        vanilla: BlockStateId,
    ) -> BlockStateId {
        self.asks.borrow_mut().push((pos, state, moved));
        self.answer.unwrap_or(vanilla)
    }
}

#[test]
#[ignore = "daily"]
fn removal_writes_chosen_state_or_vanilla_and_reports_original_arguments() {
    let f = fixture().expect("prepared resources for removal contract test");
    let stone = f.registry.default_state("minecraft:stone").unwrap();
    let dirt = f.registry.default_state("minecraft:dirt").unwrap();
    let water = f.registry.default_state("minecraft:water").unwrap();
    let pos = BlockPos::new(0, 64, 0);
    for (original, answer, moved, expected) in [
        (stone, None, false, f.behaviours.air()),
        (water, None, true, water),
        (stone, Some(dirt), false, dirt),
        (water, Some(dirt), true, dirt),
    ] {
        let mut level = RemovalLevel {
            inner: TestLevel::new(&f.behaviours),
            answer,
            asks: Default::default(),
            updates: 0,
        };
        level.inner.blocks.insert(pos, original);
        f.behaviours
            .remove_block(&mut level, &mut Ticks::new(0), pos, moved);
        assert_eq!(level.block_state(pos), expected);
        assert_eq!(*level.asks.borrow(), [(pos, original, moved)]);
        assert_eq!(level.updates, usize::from(original != expected));
    }
}
