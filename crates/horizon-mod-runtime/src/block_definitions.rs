//! Immutable block definitions shared by metadata services and worker callbacks.
use std::collections::BTreeMap;

/// A malformed imported definition, refused before publishing registry metadata.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DefinitionError {
    Empty,
    TooManyStates,
    InvalidDefault,
    InconsistentProperties,
    DuplicateState,
}

/// A refused state operation; indices identify the offending input constraint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StateError {
    UnknownState,
    UnknownProperty(usize),
    InvalidValue(usize),
    DuplicateProperty(usize),
    InvalidCombination,
}

/// One block's immutable properties and state combinations, in imported ordinal order.
pub struct BlockDefinition {
    properties: Vec<(String, Vec<String>)>,
    states: Vec<Box<[u16]>>,
    ordinals: BTreeMap<Box<[u16]>, u16>,
    default: u16,
}

impl BlockDefinition {
    /// Import and validate a definition once. State ordinals retain the supplied order.
    pub fn new(
        states: &[&BTreeMap<String, String>],
        default: u16,
    ) -> Result<Self, DefinitionError> {
        let first = states.first().ok_or(DefinitionError::Empty)?;
        if states.len() > usize::from(u16::MAX) + 1 {
            return Err(DefinitionError::TooManyStates);
        }
        if usize::from(default) >= states.len() {
            return Err(DefinitionError::InvalidDefault);
        }
        let mut properties: Vec<_> = first
            .keys()
            .map(|name| (name.clone(), Vec::new()))
            .collect();
        let mut encoded = Vec::with_capacity(states.len());
        let mut ordinals = BTreeMap::new();
        for (ordinal, state) in states.iter().enumerate() {
            if !state.keys().eq(first.keys()) {
                return Err(DefinitionError::InconsistentProperties);
            }
            let mut values = Vec::with_capacity(properties.len());
            for ((_, legal), value) in properties.iter_mut().zip(state.values()) {
                let index = match legal.iter().position(|v| v == value) {
                    Some(index) => index,
                    None => {
                        legal.push(value.clone());
                        legal.len() - 1
                    }
                };
                values.push(index as u16);
            }
            let values = values.into_boxed_slice();
            if ordinals.insert(values.clone(), ordinal as u16).is_some() {
                return Err(DefinitionError::DuplicateState);
            }
            encoded.push(values);
        }
        Ok(Self {
            properties,
            states: encoded,
            ordinals,
            default,
        })
    }

    /// Number of immutable state combinations, including every local ordinal.
    pub fn state_count(&self) -> u32 {
        self.states.len() as u32
    }

    /// Read a borrowed property value without allocation, locks or mutable state.
    pub fn property(&self, state: u16, name: &str) -> Result<Option<&str>, StateError> {
        let values = self
            .states
            .get(usize::from(state))
            .ok_or(StateError::UnknownState)?;
        Ok(self
            .properties
            .iter()
            .enumerate()
            .find(|(_, (key, _))| key == name)
            .map(|(index, (_, legal))| legal[usize::from(values[index])].as_str()))
    }

    /// Build a state from the default and partial property constraints.
    pub fn of(&self, properties: &[(&str, &str)]) -> Result<u16, StateError> {
        if properties.is_empty() {
            return Ok(self.default);
        }
        self.apply(self.default, properties)
    }

    /// Change one property, retaining every other value in the supplied state.
    pub fn with(&self, state: u16, name: &str, value: &str) -> Result<u16, StateError> {
        self.apply(state, &[(name, value)])
    }

    fn apply(&self, state: u16, properties: &[(&str, &str)]) -> Result<u16, StateError> {
        let mut values = self
            .states
            .get(usize::from(state))
            .ok_or(StateError::UnknownState)?
            .to_vec();
        for (input, &(name, value)) in properties.iter().enumerate() {
            if properties[..input]
                .iter()
                .any(|(previous, _)| *previous == name)
            {
                return Err(StateError::DuplicateProperty(input));
            }
            let (index, (_, legal)) = self
                .properties
                .iter()
                .enumerate()
                .find(|(_, (key, _))| key == name)
                .ok_or(StateError::UnknownProperty(input))?;
            values[index] = legal
                .iter()
                .position(|v| v == value)
                .ok_or(StateError::InvalidValue(input))? as u16;
        }
        self.ordinals
            .get(values.as_slice())
            .copied()
            .ok_or(StateError::InvalidCombination)
    }
}

/// Completed names and definitions are published together by the installed registry.
pub struct Blocks {
    pub names: crate::registry_keys::Names,
    pub definitions: Vec<BlockDefinition>,
}

impl Blocks {
    /// Resolve every definition against completed names, refusing missing entries atomically.
    pub fn prepare(
        names: crate::registry_keys::Names,
        mut definition: impl FnMut(&str) -> Result<BlockDefinition, String>,
    ) -> Result<Self, Vec<String>> {
        let definitions = names
            .entries
            .iter()
            .map(|entry| definition(&entry.id))
            .collect::<Result<_, _>>()
            .map_err(|error| vec![error])?;
        Ok(Self { names, definitions })
    }
}
