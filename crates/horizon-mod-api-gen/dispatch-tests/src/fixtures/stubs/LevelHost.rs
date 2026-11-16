pub(super) trait LevelHost {
    fn state_block(&self, _: BlockStateId) -> Option<&str> {
        None
    }
}
