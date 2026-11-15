mod stubs {
    #[derive(Clone, Copy)]
    pub(super) enum UsePhase {
        ItemBeforeBlock,
        Block,
        ItemAfterBlock,
    }
}
