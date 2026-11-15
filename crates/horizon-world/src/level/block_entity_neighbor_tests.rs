use super::tests::{TestLevel, fixture};
use super::*;
use std::cell::RefCell;

#[test]
fn block_entity_neighbor_notices_and_weak_changes() {
    struct Observer {
        inner: TestLevel,
        seen: RefCell<Vec<(BlockPos, BlockPos)>>,
        weak: bool,
        unloaded: Option<BlockPos>,
        queries: RefCell<Vec<BlockPos>>,
    }
    impl BlockEntityNeighbors for Observer {
        fn observe_block_entity_neighbor(
            &self,
            pos: BlockPos,
            _: BlockStateId,
            neighbor: BlockPos,
        ) {
            self.seen.borrow_mut().push((pos, neighbor));
        }
        fn receives_weak_changes(&self, pos: BlockPos, _: BlockStateId) -> bool {
            self.queries.borrow_mut().push(pos);
            self.weak
        }
    }
    impl LevelAccess for Observer {
        fn block_state(&self, pos: BlockPos) -> BlockStateId {
            self.inner.block_state(pos)
        }
        fn set_raw(&mut self, pos: BlockPos, state: BlockStateId) -> Option<BlockStateId> {
            self.inner.set_raw(pos, state)
        }
        fn is_loaded(&self, pos: BlockPos) -> bool {
            Some(pos) != self.unloaded && self.inner.is_loaded(pos)
        }
        fn random(&mut self) -> &mut LegacyRandom {
            self.inner.random()
        }
        fn game_time(&self) -> i64 {
            0
        }

        fn observe_neighbor_changed(
            &self,
            _: &mut Ticks,
            pos: BlockPos,
            _: BlockStateId,
            _: BlockId,
            _: bool,
        ) {
            self.seen.borrow_mut().push((pos, BlockPos::new(-99, 0, 0)));
        }
    }
    let Some(f) = fixture() else { return };
    let b = &f.behaviours;
    let origin = BlockPos::new(0, 64, 0);
    let adjacent = BlockPos::new(1, 64, 0);
    let far = BlockPos::new(2, 64, 0);
    // Independent coordinates pin every direction, not the production direction list.
    for (dx, dz) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
        let adjacent = BlockPos::new(dx, 64, dz);
        let far = BlockPos::new(2 * dx, 64, 2 * dz);
        for unloaded in [adjacent, far] {
            let mut level = Observer {
                inner: TestLevel::new(b),
                seen: RefCell::new(Vec::new()),
                weak: true,
                unloaded: Some(unloaded),
                queries: RefCell::new(Vec::new()),
            };
            level.set_raw(
                adjacent,
                f.registry.default_state("minecraft:stone").unwrap(),
            );
            level.set_raw(origin, f.registry.default_state("minecraft:chest").unwrap());
            let mut ticks = Ticks::new(0);
            b.container_changed(&mut level, &mut ticks, origin);
            let seen = level.seen.borrow();
            if unloaded == adjacent {
                assert!(!seen.contains(&(adjacent, origin)));
                assert!(!seen.contains(&(far, origin)));
                assert!(!level.queries.borrow().contains(&far));
            } else {
                assert!(seen.contains(&(adjacent, origin)), "direction {dx}, {dz}");
                assert!(
                    level.queries.borrow().contains(&far),
                    "unloaded far cell is queried"
                );
                assert!(
                    seen.contains(&(far, origin)),
                    "unloaded far cell is notified"
                );
                assert!(!level.is_loaded(far), "notification must not load its cell");
            }
        }
    }
    for weak in [false, true] {
        let mut level = Observer {
            inner: TestLevel::new(b),
            seen: RefCell::new(Vec::new()),
            weak,
            unloaded: None,
            queries: RefCell::new(Vec::new()),
        };
        level.set_raw(
            adjacent,
            f.registry.default_state("minecraft:stone").unwrap(),
        );
        let mut ticks = Ticks::new(0);
        level.set_raw(origin, f.registry.default_state("minecraft:chest").unwrap());
        b.container_changed(&mut level, &mut ticks, origin);
        let seen = level.seen.borrow();
        assert_eq!(
            seen.iter()
                .filter(|&&(p, n)| p == adjacent && n == origin)
                .count(),
            1
        );
        assert_eq!(
            seen.iter()
                .filter(|&&(p, n)| p == far && n == origin)
                .count(),
            usize::from(weak)
        );
        drop(seen);
        level.seen.borrow_mut().clear();
        level.set_raw(
            adjacent,
            f.registry.default_state("minecraft:comparator").unwrap(),
        );
        level.set_raw(
            origin,
            f.registry.default_state("minecraft:command_block").unwrap(),
        );
        b.update_neighbour_for_output_signal(&mut level, &mut ticks, origin);
        let seen = level.seen.borrow();
        let vanilla = seen
            .iter()
            .position(|&(p, n)| p == adjacent && n == BlockPos::new(-99, 0, 0))
            .unwrap();
        let notice = seen
            .iter()
            .position(|&(p, n)| p == adjacent && n == origin)
            .unwrap();
        assert!(vanilla < notice);
        assert_eq!(
            seen.iter()
                .filter(|&&(p, n)| p == adjacent && n == origin)
                .count(),
            1
        );
    }
}

#[test]
fn block_entity_neighbor_default_is_vanilla() {
    let Some(f) = fixture() else { return };
    let b = &f.behaviours;
    let mut level = TestLevel::new(b);
    let origin = BlockPos::new(0, 64, 0);
    level.set_raw(
        origin,
        f.registry.default_state("minecraft:command_block").unwrap(),
    );
    let mut ticks = Ticks::new(0);
    b.update_neighbour_for_output_signal(&mut level, &mut ticks, origin);
    assert_eq!(
        level.block_state(origin),
        f.registry.default_state("minecraft:command_block").unwrap()
    );
    assert!(!level.receives_weak_changes(BlockPos::new(2, 64, 0), b.air()));
}
