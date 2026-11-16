//! The table a server publishes for its world generation, as the worldgen Part methods read it on
//! threads with no game. The published table itself is tested with the engine's tree-grow
//! adapter (`horizon-client` `native_tree_grow`), where Parts can be registered.
use super::*;
use crate::*;

struct Draws;
impl StructureRandom for Draws {
    fn next_int_bound(&mut self, _: i32) -> i32 {
        0
    }
    fn next_long(&mut self) -> i64 {
        0
    }
    fn next_float(&mut self) -> f32 {
        0.0
    }
    fn next_double(&mut self) -> f64 {
        0.0
    }
    fn next_boolean(&mut self) -> bool {
        false
    }
}

#[test]
fn with_no_part_subscribed_a_worldgen_dispatch_looks_nothing_up() {
    super::super::install_input(super::super::installation::InstalledInput::default());
    TEST_PART_LOOKUPS.set(0);
    let mut soil = None;
    crate::world::ground::native_on_tree_grow(
        &OwningWorldgen::new(&|_| None, &|_, _| None, &|_, _, _| None),
        BlockState::from(BlockStateRecord::new(BlockKey(0), 0)),
        [0, 63, 0],
        &mut Draws,
        &mut soil,
    );
    assert_eq!((soil, TEST_PART_LOOKUPS.get()), (None, 0));
}
