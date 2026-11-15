//! World-owned POI identities and compiled custom block-state membership.
use super::{PoiType, saved_poi};
use crate::chunk::BlockState;
use std::collections::{BTreeMap, HashMap};

/// A POI identity scoped to one prepared snapshot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PoiIdentity {
    Vanilla(PoiType),
    Custom(u32),
}
impl From<PoiType> for PoiIdentity {
    fn from(value: PoiType) -> Self {
        Self::Vanilla(value)
    }
}
impl PoiIdentity {
    pub fn is_job(self) -> bool {
        match self {
            Self::Vanilla(p) => p.is_job(),
            Self::Custom(_) => true,
        }
    }
    pub fn max_tickets(self) -> usize {
        match self {
            Self::Vanilla(p) => p.max_tickets(),
            Self::Custom(_) => 1,
        }
    }
    pub fn valid_range(self) -> i32 {
        match self {
            Self::Vanilla(p) => p.valid_range(),
            Self::Custom(_) => 1,
        }
    }
}

/// Custom membership is resolved before publication, never on a villager tick.
#[derive(Debug, Default)]
pub struct PoiSnapshot {
    names: Vec<String>,
    by_name: HashMap<String, u32>,
    states: BTreeMap<BlockState, PoiIdentity>,
}
impl PoiSnapshot {
    /// Compile all custom types in canonical order before assigning their slots.
    pub fn prepare(mut types: Vec<(String, Vec<BlockState>)>) -> Result<Self, String> {
        types.sort_by(|a, b| a.0.cmp(&b.0));
        let mut snapshot = Self::default();
        for (name, states) in types {
            snapshot.add(&name, states)?;
        }
        Ok(snapshot)
    }
    pub fn resolve(&self, name: &str) -> Option<PoiIdentity> {
        PoiType::from_id(name)
            .map(PoiIdentity::Vanilla)
            .or_else(|| self.by_name.get(name).copied().map(PoiIdentity::Custom))
    }
    pub fn name(&self, poi: PoiIdentity) -> Option<&str> {
        match poi {
            PoiIdentity::Vanilla(p) => Some(p.id()),
            PoiIdentity::Custom(i) => self.names.get(i as usize).map(String::as_str),
        }
    }
    pub fn custom(&self, state: &BlockState) -> Option<PoiIdentity> {
        if self.states.is_empty() {
            None
        } else {
            self.states.get(state).copied()
        }
    }
    pub fn classify(&self, state: &BlockState) -> Option<PoiIdentity> {
        saved_poi(state)
            .map(PoiIdentity::Vanilla)
            .or_else(|| self.custom(state))
    }
    pub fn add(
        &mut self,
        name: &str,
        states: impl IntoIterator<Item = BlockState>,
    ) -> Result<(), String> {
        if name.starts_with("minecraft:") {
            return Err(format!("reserved POI {name}"));
        }
        let index = if let Some(index) = self.by_name.get(name) {
            *index
        } else {
            let index = u32::try_from(self.names.len()).map_err(|_| "POI slots exhausted")?;
            self.names.push(name.into());
            self.by_name.insert(name.into(), index);
            index
        };
        let identity = PoiIdentity::Custom(index);
        for state in states {
            if let Some(old) = self.classify(&state)
                && old != identity
            {
                return Err(format!(
                    "{} already belongs to {}",
                    state.name,
                    self.name(old).unwrap_or("unknown POI")
                ));
            }
            self.states.insert(state, identity);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn custom_poi_slots_are_canonical() {
        for names in [["test:z", "other:a"], ["other:a", "test:z"]] {
            let snapshot = PoiSnapshot::prepare(
                names
                    .into_iter()
                    .map(|name| {
                        (
                            name.into(),
                            vec![BlockState {
                                name: name.into(),
                                properties: Default::default(),
                            }],
                        )
                    })
                    .collect(),
            )
            .unwrap();
            assert_eq!(snapshot.resolve("other:a"), Some(PoiIdentity::Custom(0)));
            assert_eq!(snapshot.resolve("test:z"), Some(PoiIdentity::Custom(1)));
            assert_eq!(
                snapshot.custom(&BlockState {
                    name: "test:z".into(),
                    properties: Default::default()
                }),
                Some(PoiIdentity::Custom(1))
            );
        }
    }
}
