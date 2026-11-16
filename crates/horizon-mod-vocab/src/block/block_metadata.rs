//! The read-only metadata resource behind block value operations.
use crate::{BlockKey, StateConstraint};

/// Transport form for a process-local state. Mods use the BlockState facade, not this record.
/// Receiving metadata and write boundaries validate the ordinal against the block definition.
// hidden: generated native/wasm transport for the private-field BlockState facade.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct BlockStateRecord {
    /// Process-local block key, not a saved block name.
    pub block: BlockKey,
    /// State ordinal within the block's definition.
    pub state: u16,
}
impl BlockStateRecord {
    /// Construct transport data; registry-dependent bounds belong to the receiving boundary.
    pub const fn new(block: BlockKey, state: u16) -> Self {
        Self { block, state }
    }
}

/// A refused metadata read or state operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum BlockStateError {
    /// Metadata is unavailable in this context.
    Unavailable,
    /// The context does not contain the block key.
    UnknownBlock,
    /// The ordinal is outside the block definition.
    UnknownState,
    /// Constraint index naming a missing property.
    UnknownProperty(u32),
    /// Constraint index naming an invalid value.
    InvalidValue(u32),
    /// Constraint index repeating a property.
    DuplicateProperty(u32),
    /// The requested combination is not a registered state.
    InvalidCombination,
}
impl crate::NoBackend for BlockStateError {
    fn no_backend() -> Self {
        Self::Unavailable
    }
}
impl From<horizon_mod_runtime::block_definitions::StateError> for BlockStateError {
    fn from(error: horizon_mod_runtime::block_definitions::StateError) -> Self {
        use horizon_mod_runtime::block_definitions::StateError;
        match error {
            StateError::UnknownState => Self::UnknownState,
            StateError::UnknownProperty(index) => Self::UnknownProperty(index as u32),
            StateError::InvalidValue(index) => Self::InvalidValue(index as u32),
            StateError::DuplicateProperty(index) => Self::DuplicateProperty(index as u32),
            StateError::InvalidCombination => Self::InvalidCombination,
        }
    }
}

/// Read-only metadata, lent by a host's block-states service or a tint worker.
/// This is the low-level resource contract; mods call the BlockState facade instead.
/// Native and generated WebAssembly forwards invoke the same immutable table implementation.
// hidden: low-level shared resource contract; BlockState methods are the mod-facing operations.
#[doc(hidden)]
pub trait BlockMetadata {
    /// Borrow the canonical block name from this context's retained registry.
    fn name(&self, block: BlockKey) -> Result<&str, BlockStateError>;
    /// Borrow a property's serialized value; a missing property returns None.
    fn property(
        &self,
        state: BlockStateRecord,
        name: &str,
    ) -> Result<Option<&str>, BlockStateError>;
    /// Change one property while retaining the others.
    fn with(
        &self,
        state: BlockStateRecord,
        name: &str,
        value: &str,
    ) -> Result<BlockStateRecord, BlockStateError>;
    /// Apply partial constraints to the block's default state.
    fn of(
        &self,
        block: BlockKey,
        properties: &[StateConstraint],
    ) -> Result<BlockStateRecord, BlockStateError>;
}

impl BlockMetadata for horizon_mod_runtime::block_definitions::Blocks {
    fn name(&self, block: BlockKey) -> Result<&str, BlockStateError> {
        self.names
            .entries
            .get(block.0 as usize)
            .map(|entry| entry.id.as_str())
            .ok_or(BlockStateError::UnknownBlock)
    }
    fn property(
        &self,
        state: BlockStateRecord,
        name: &str,
    ) -> Result<Option<&str>, BlockStateError> {
        self.definitions
            .get(state.block.0 as usize)
            .ok_or(BlockStateError::UnknownBlock)?
            .property(state.state, name)
            .map_err(Into::into)
    }
    fn with(
        &self,
        state: BlockStateRecord,
        name: &str,
        value: &str,
    ) -> Result<BlockStateRecord, BlockStateError> {
        let ordinal = self
            .definitions
            .get(state.block.0 as usize)
            .ok_or(BlockStateError::UnknownBlock)?
            .with(state.state, name, value)?;
        Ok(BlockStateRecord::new(state.block, ordinal))
    }
    fn of(
        &self,
        block: BlockKey,
        properties: &[StateConstraint],
    ) -> Result<BlockStateRecord, BlockStateError> {
        let definition = self
            .definitions
            .get(block.0 as usize)
            .ok_or(BlockStateError::UnknownBlock)?;
        let constraints: Vec<_> = properties
            .iter()
            .map(|p| (p.name.as_str(), p.value.as_str()))
            .collect();
        let ordinal = definition.of(&constraints)?;
        Ok(BlockStateRecord::new(block, ordinal))
    }
}
