//! A validated span of API-local block state ordinals.
use crate::{BlockKey, BlockState, BlockStateError, BlockStateRecord};

/// States of one block in immutable definition order, not world numeric ID order.
// no wire: decoding calls new to reject spans whose last ordinal would overflow u16; zero-length spans remain valid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct BlockStateRange {
    first: BlockState,
    count: u32,
}

impl BlockStateRange {
    /// Construct a span, refusing a count that would exceed the u16 ordinal domain.
    /// Key/definition membership is checked by the metadata or write boundary that uses it.
    pub const fn new(first: BlockState, count: u32) -> Result<Self, BlockStateError> {
        if count > u16::MAX as u32 + 1 - first.ordinal() as u32 {
            return Err(BlockStateError::UnknownState);
        }
        Ok(Self { first, count })
    }
    /// The first API state, independent of world numeric IDs.
    pub const fn first(self) -> BlockState {
        self.first
    }
    /// How many local ordinals the span contains.
    pub const fn len(self) -> u32 {
        self.count
    }
    /// Whether the span contains no states.
    pub const fn is_empty(self) -> bool {
        self.count == 0
    }
    /// Whether both the block key and local ordinal belong to this span.
    pub const fn contains(self, state: BlockState) -> bool {
        state.block().0 == self.first.block().0
            && state.ordinal() >= self.first.ordinal()
            && ((state.ordinal() - self.first.ordinal()) as u32) < self.count
    }
    /// Every state in validated local ordinal order.
    pub fn iter(self) -> impl ExactSizeIterator<Item = BlockState> + DoubleEndedIterator {
        (0..self.count).map(move |offset| {
            BlockState::from(BlockStateRecord::new(
                self.first.block(),
                (self.first.ordinal() as u32 + offset) as u16,
            ))
        })
    }
}
/// Resolve a block's immutable definition span without a world-number mapping.
// hidden: engine metadata service implementation, re-exported only through __private.
#[doc(hidden)]
pub fn block_state_range(block: BlockKey) -> Result<BlockStateRange, BlockStateError> {
    let blocks =
        crate::content::registry_keys::block_definitions().ok_or(BlockStateError::Unavailable)?;
    let definition = blocks
        .definitions
        .get(block.0 as usize)
        .ok_or(BlockStateError::UnknownBlock)?;
    BlockStateRange::new(
        BlockStateRecord::new(block, 0).into(),
        definition.state_count(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn state(block: u32, ordinal: u16) -> BlockState {
        BlockStateRecord::new(BlockKey(block), ordinal).into()
    }
    #[test]
    fn local_range_validates_spans_and_never_wraps_ordinals() {
        let range = BlockStateRange::new(state(3, 65534), 2).unwrap();
        assert_eq!(
            range.iter().map(BlockState::ordinal).collect::<Vec<_>>(),
            [65534, 65535]
        );
        assert!(range.contains(state(3, 65535)));
        assert!(!range.contains(state(4, 65535)));
        assert!(!range.contains(state(3, 65533)));
        assert_eq!(
            BlockStateRange::new(state(3, 65535), 2),
            Err(BlockStateError::UnknownState)
        );
        assert_eq!(
            BlockStateRange::new(state(3, 0), u32::MAX),
            Err(BlockStateError::UnknownState)
        );
        assert_eq!(
            BlockStateRange::new(state(3, 65535), 0)
                .unwrap()
                .iter()
                .count(),
            0
        );
        let full = BlockStateRange::new(state(3, 0), 65536).unwrap();
        assert_eq!(full.iter().count(), 65536);
        assert_eq!(full.iter().next_back(), Some(state(3, 65535)));
    }
}
