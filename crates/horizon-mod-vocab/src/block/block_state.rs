//! Process-local block values, independent of the game's numeric registry.
use crate::{BlockKey, BlockMetadata, BlockStateError, BlockStateRecord, StateConstraint};

/// A block and its local state ordinal. Persist canonical names and properties, not this value.
#[derive(horizon_mod_api_macros::Wire)]
#[wire(internal_field_layouts)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct BlockState {
    block: BlockKey,
    state: u16,
}
impl BlockState {
    /// The block this state belongs to, without needing a host or metadata lookup.
    pub const fn block(self) -> BlockKey {
        self.block
    }
    /// Its process-local ordinal, not the game's numeric state identifier.
    pub const fn ordinal(self) -> u16 {
        self.state
    }
    /// Borrow a property from the supplied immutable metadata; a missing property is `None`.
    pub fn property<'a>(
        self,
        metadata: &'a dyn BlockMetadata,
        name: &str,
    ) -> Result<Option<&'a str>, BlockStateError> {
        metadata.property((&self).into(), name)
    }
    /// Return a state with one property changed, without mutating metadata or the world.
    pub fn with(
        self,
        metadata: &dyn BlockMetadata,
        name: &str,
        value: &str,
    ) -> Result<Self, BlockStateError> {
        metadata.with((&self).into(), name, value).map(Into::into)
    }
    /// Start at the block's default and apply partial property constraints.
    pub fn of(
        metadata: &dyn BlockMetadata,
        block: BlockKey,
        properties: &[StateConstraint],
    ) -> Result<Self, BlockStateError> {
        metadata.of(block, properties).map(Into::into)
    }
}
impl From<BlockStateRecord> for BlockState {
    fn from(value: BlockStateRecord) -> Self {
        Self {
            block: value.block,
            state: value.state,
        }
    }
}
impl From<&BlockState> for BlockStateRecord {
    fn from(value: &BlockState) -> Self {
        Self::new(value.block, value.state)
    }
}
