impl BlockEntityRef {
    fn kind(self, _: &dyn ClientHost) -> Option<BlockEntityTypeKey> {
        Some(BlockEntityTypeKey(0))
    }
}

mod stubs {
    #[derive(Clone, Copy)]
    pub(super) enum BlockEntityDataSource {
        Chunk,
        UpdatePacket,
    }
}
