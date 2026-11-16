//! Immutable imported registry data, separate from per-installation worker state.
use super::{BlockState, BlockStateId, state_key};
use std::collections::{BTreeMap, HashMap};

#[derive(PartialEq, Eq)]
pub(super) struct Data {
    pub(super) states: Vec<BlockState>,
    /// Ids by a hash of their state, checked against `states`: 12 bytes per
    /// state instead of a second copy of every name and property map (about
    /// 125 MB for a registry of 190,000 states).
    pub(super) ids: HashMap<u64, u32>,
    /// States whose hash an earlier state already took (none in vanilla).
    pub(super) collided: BTreeMap<BlockState, BlockStateId>,
    pub(super) defaults: BTreeMap<String, BlockStateId>,
}

impl Data {
    pub(super) fn id(&self, state: &BlockState) -> Option<BlockStateId> {
        match self.ids.get(&state_key(state)) {
            Some(&id) if self.states[id as usize] == *state => Some(BlockStateId(id)),
            _ => self.collided.get(state).copied(),
        }
    }
}
