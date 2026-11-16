#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlockState {
    block: BlockKey,
    ordinal: u16,
}
impl BlockState {
    pub fn new(block: BlockKey, ordinal: u16) -> Self {
        Self { block, ordinal }
    }
    pub fn block(self) -> BlockKey {
        self.block
    }
    pub fn ordinal(self) -> u16 {
        self.ordinal
    }
}
