//! `on_tree_grow`: a worldgen Part method takes the worldgen context and reads the table the
//! server published for its world generation, not this thread's.
mod stubs {
    pub(super) trait StructureRandom {}
}
thread_local! {
    /// The table the fixture's server published for its world generation.
    static WORLDGEN_PARTS: std::cell::Cell<&'static Parts> = const { std::cell::Cell::new(&NO_PARTS) };
}
fn worldgen_parts() -> &'static Parts {
    WORLDGEN_PARTS.get()
}

#[test]
fn a_worldgen_part_method_reads_the_published_table() {
    struct Worldgen;
    impl WorldgenHost for Worldgen {}
    struct Draws;
    impl StructureRandom for Draws {}
    struct Soil(Option<BlockState>);
    impl BlockPart for Soil {
        fn block_subscribed(&self) -> &'static [Option<HookId>] {
            &[]
        }
        fn on_tree_grow(
            &self,
            _: &dyn WorldgenHost,
            event: &mut OnTreeGrowEvent<'_>,
        ) -> Option<Option<BlockState>> {
            assert_eq!(event.pos, [1, 64, 2]);
            Some(self.0)
        }
    }
    let coarse = BlockState::new(BlockKey::Test, 1);
    // This thread's game table answers nothing; the published one answers coarse dirt.
    install_parts(Parts {
        blocks: vec![(BlockKey::Test, vec![leak(Soil(None))])],
        ..Parts::default()
    });
    WORLDGEN_PARTS.set(leak(Parts {
        blocks: vec![(BlockKey::Test, vec![leak(Soil(Some(coarse)))])],
        ..Parts::default()
    }));
    let mut soil = None;
    dispatch_on_tree_grow(
        &Worldgen,
        BlockState::new(BlockKey::Test, 0),
        [1, 64, 2],
        &mut Draws,
        &mut soil,
    );
    assert_eq!(soil, Some(Some(coarse)));
}
