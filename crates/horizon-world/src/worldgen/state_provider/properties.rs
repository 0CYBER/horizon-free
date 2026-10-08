//! Compact property transitions derived from the pinned Cartesian state tables.
use crate::registry::{BlockRegistry, BlockStateId};
use std::collections::{BTreeMap, BTreeSet};
#[derive(Clone, Copy, Debug)]
struct Address {
    block: usize,
    local: usize,
}
#[derive(Debug)]
struct Property {
    identity: usize,
    name: String,
    values: Vec<String>,
    lookup: BTreeMap<String, usize>,
    numeric: Option<BTreeMap<i32, usize>>,
    stride: usize,
}
#[derive(Debug)]
struct Block {
    states: Vec<BlockStateId>,
    properties: Vec<Property>,
}
#[derive(Debug)]
pub(crate) struct StateProperties {
    blocks: Vec<Block>,
    addresses: Vec<Address>,
}
impl StateProperties {
    pub fn build(
        registry: &BlockRegistry,
        integers: &BTreeSet<(String, String)>,
    ) -> Result<Self, String> {
        let mut grouped = BTreeMap::<String, Vec<BlockStateId>>::new();
        for index in 0..registry.len() {
            let id = BlockStateId(index as u32);
            grouped
                .entry(registry.state(id).unwrap().name.clone())
                .or_default()
                .push(id);
        }
        let mut identities = BTreeMap::new();
        let mut result = Self {
            blocks: Vec::new(),
            addresses: vec![Address { block: 0, local: 0 }; registry.len()],
        };
        for (name, states) in grouped {
            let mut properties = Vec::new();
            let first = registry.state(states[0]).unwrap();
            for (property, initial) in &first.properties {
                let mut values = Vec::new();
                let mut lookup = BTreeMap::new();
                let mut stride = None;
                for (index, id) in states.iter().enumerate() {
                    let state = registry.state(*id).unwrap();
                    if state.properties.len() != first.properties.len() {
                        return Err("inconsistent block properties".into());
                    }
                    let value = state
                        .properties
                        .get(property)
                        .ok_or("missing block property")?;
                    if value != initial && stride.is_none() {
                        stride = Some(index);
                    }
                    if !lookup.contains_key(value) {
                        lookup.insert(value.clone(), values.len());
                        values.push(value.clone());
                    }
                }
                let stride = stride.unwrap_or(1);
                for (index, id) in states.iter().enumerate() {
                    if registry.state(*id).unwrap().properties[property]
                        != values[(index / stride) % values.len()]
                    {
                        return Err("non-Cartesian property state order".into());
                    }
                }
                let integer = integers.contains(&(name.clone(), property.clone()));
                // In the pinned registry these signatures uniquely identify
                // Java Property.equals families; the native harness checks this.
                let signature = (
                    property.clone(),
                    integer,
                    lookup.keys().cloned().collect::<Vec<_>>(),
                );
                let next = identities.len();
                let identity = *identities.entry(signature).or_insert(next);
                let numeric = if integer {
                    Some(
                        values
                            .iter()
                            .enumerate()
                            .map(|(i, s)| {
                                s.parse::<i32>()
                                    .map(|v| (v, i))
                                    .map_err(|_| "invalid integer property".to_owned())
                            })
                            .collect::<Result<_, _>>()?,
                    )
                } else {
                    None
                };
                properties.push(Property {
                    identity,
                    name: property.clone(),
                    values,
                    lookup,
                    numeric,
                    stride,
                });
            }
            let mut dimensions = properties
                .iter()
                .filter(|p| p.values.len() > 1)
                .collect::<Vec<_>>();
            dimensions.sort_by_key(|p| p.stride);
            let mut size = 1usize;
            for p in dimensions {
                if p.stride != size {
                    return Err("overlapping property state dimensions".into());
                }
                size = size
                    .checked_mul(p.values.len())
                    .ok_or("property state size overflow")?;
            }
            if size != states.len() {
                return Err("incomplete Cartesian state table".into());
            }
            for (local, id) in states.iter().enumerate() {
                result.addresses[id.0 as usize] = Address {
                    block: result.blocks.len(),
                    local,
                };
            }
            result.blocks.push(Block { states, properties });
        }
        Ok(result)
    }
    pub fn copy(&self, target: BlockStateId, source: BlockStateId) -> Option<BlockStateId> {
        let mut target = *self.addresses.get(target.0 as usize)?;
        let source = *self.addresses.get(source.0 as usize)?;
        let block = &self.blocks[target.block];
        for source_property in &self.blocks[source.block].properties {
            if let Some(property) = block
                .properties
                .iter()
                .find(|p| p.identity == source_property.identity)
            {
                let value = &source_property.values
                    [(source.local / source_property.stride) % source_property.values.len()];
                let index = *property.lookup.get(value)?;
                let previous = (target.local / property.stride) % property.values.len();
                target.local = target.local - previous * property.stride + index * property.stride;
            }
        }
        block.states.get(target.local).copied()
    }
    pub fn has_integer(&self, state: BlockStateId, name: &str) -> Option<bool> {
        let address = self.addresses.get(state.0 as usize)?;
        Some(
            self.blocks[address.block]
                .properties
                .iter()
                .any(|p| p.name == name && p.numeric.is_some()),
        )
    }
    pub fn set_integer(&self, state: BlockStateId, name: &str, value: i32) -> Option<BlockStateId> {
        let address = self.addresses.get(state.0 as usize)?;
        let block = &self.blocks[address.block];
        let property = block.properties.iter().find(|p| p.name == name)?;
        let index = property.numeric.as_ref()?.get(&value)?;
        let previous = (address.local / property.stride) % property.values.len();
        block
            .states
            .get(address.local - previous * property.stride + index * property.stride)
            .copied()
    }

    fn property(&self, state: BlockStateId, name: &str) -> Option<(Address, &Property)> {
        let address = *self.addresses.get(state.0 as usize)?;
        let property = self.blocks[address.block]
            .properties
            .iter()
            .find(|p| p.name == name)?;
        Some((address, property))
    }
    /// The state's value for a named property, or None when it has no such property.
    pub fn value(&self, state: BlockStateId, name: &str) -> Option<&str> {
        let (address, property) = self.property(state, name)?;
        Some(&property.values[(address.local / property.stride) % property.values.len()])
    }
    /// All values of the state's named property in registry encounter order.
    pub fn values(&self, state: BlockStateId, name: &str) -> Option<&[String]> {
        self.property(state, name).map(|(_, p)| p.values.as_slice())
    }
    /// Replace one property value, or None for a missing property or value.
    pub fn with_value(&self, state: BlockStateId, name: &str, value: &str) -> Option<BlockStateId> {
        let (address, property) = self.property(state, name)?;
        let index = *property.lookup.get(value)?;
        let previous = (address.local / property.stride) % property.values.len();
        self.blocks[address.block]
            .states
            .get(address.local - previous * property.stride + index * property.stride)
            .copied()
    }
}
