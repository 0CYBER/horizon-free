//! Runtime block-state IDs imported from a version-pinned registry data export.
//! This supplies identities and default properties, not block behavior.
mod json;
use crate::{DATA_VERSION, chunk::BlockState, compression::invalid, nbt::Tag};
pub use json::BlockStateJsonCodec;
pub(crate) use json::java_integer;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    hash::{DefaultHasher, Hash, Hasher},
    io,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BlockStateId(pub u32);

pub struct BlockRegistry {
    states: Vec<BlockState>,
    /// Ids by a hash of their state, checked against `states`: 12 bytes per
    /// state instead of a second copy of every name and property map (about
    /// 125 MB for a registry of 190,000 states).
    ids: HashMap<u64, u32>,
    /// States whose hash an earlier state already took (none in vanilla).
    collided: BTreeMap<BlockState, BlockStateId>,
    defaults: BTreeMap<String, BlockStateId>,
}
/// The hash [`BlockRegistry`] indexes states by (fixed keys: the same in
/// every process).
fn state_key(state: &BlockState) -> u64 {
    let mut hasher = DefaultHasher::new();
    state.name.hash(&mut hasher);
    for (key, value) in &state.properties {
        key.hash(&mut hasher);
        value.hash(&mut hasher);
    }
    hasher.finish()
}
impl BlockRegistry {
    pub fn from_nbt(tag: &Tag) -> io::Result<Self> {
        if tag.get("DataVersion") != Some(&Tag::Int(DATA_VERSION)) {
            return Err(invalid("block registry DataVersion mismatch"));
        }
        let Some(Tag::List(entries)) = tag.get("states") else {
            return Err(invalid("missing registry states"));
        };
        let Some(Tag::Compound(defaults)) = tag.get("defaults") else {
            return Err(invalid("missing registry defaults"));
        };
        if entries.is_empty() || entries.len() > i32::MAX as usize {
            return Err(invalid("invalid registry size"));
        }
        let mut result = Self {
            states: Vec::with_capacity(entries.len()),
            ids: HashMap::with_capacity(entries.len()),
            collided: BTreeMap::new(),
            defaults: BTreeMap::new(),
        };
        for (i, entry) in entries.iter().enumerate() {
            let state = BlockState::from_nbt(entry)?;
            if result.id(&state).is_some() {
                return Err(invalid("duplicate block registry state"));
            }
            match result.ids.entry(state_key(&state)) {
                std::collections::hash_map::Entry::Vacant(slot) => {
                    slot.insert(i as u32);
                }
                std::collections::hash_map::Entry::Occupied(_) => {
                    result
                        .collided
                        .insert(state.clone(), BlockStateId(i as u32));
                }
            }
            result.states.push(state);
        }
        for (name, id) in defaults {
            let name = name.to_string().map_err(|e| invalid(e.to_string()))?;
            let Tag::Int(id) = id else {
                return Err(invalid("default state ID is not an int"));
            };
            if *id < 0
                || result
                    .states
                    .get(*id as usize)
                    .is_none_or(|s| s.name != name)
            {
                return Err(invalid("invalid default block state ID"));
            }
            result.defaults.insert(name, BlockStateId(*id as u32));
        }
        let names: BTreeSet<_> = result.states.iter().map(|s| &s.name).collect();
        if names.len() != result.defaults.len()
            || names
                .iter()
                .any(|name| !result.defaults.contains_key(*name))
        {
            return Err(invalid("registry is missing block defaults"));
        }
        Ok(result)
    }
    pub fn len(&self) -> usize {
        self.states.len()
    }
    pub fn is_empty(&self) -> bool {
        self.states.is_empty()
    }
    pub fn state(&self, id: BlockStateId) -> Option<&BlockState> {
        self.states.get(id.0 as usize)
    }
    pub fn id(&self, state: &BlockState) -> Option<BlockStateId> {
        match self.ids.get(&state_key(state)) {
            Some(&id) if self.states[id as usize] == *state => Some(BlockStateId(id)),
            _ => self.collided.get(state).copied(),
        }
    }
    pub fn default_state(&self, name: &str) -> Option<BlockStateId> {
        self.defaults.get(name).copied()
    }

    /// Resolve a saved descriptor, supplying omitted default properties. Unlike
    /// vanilla's lenient logging paths, invalid properties return a typed error.
    pub fn resolve(&self, descriptor: &BlockState) -> io::Result<BlockStateId> {
        let default = self
            .default_state(&descriptor.name)
            .ok_or_else(|| invalid("unknown block name"))?;
        let mut state = self.state(default).unwrap().clone();
        for (key, value) in &descriptor.properties {
            if !state.properties.contains_key(key) {
                return Err(invalid("unknown block property"));
            }
            state.properties.insert(key.clone(), value.clone());
        }
        self.id(&state)
            .ok_or_else(|| invalid("invalid block property value"))
    }
}
