//! Immutable biome identities and resolved holder sets for world predicates.
use super::{DecodeError, biome::BiomeId, density::identifier};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
pub struct BiomePredicateBindings {
    names: BTreeMap<String, BiomeId>,
    tags: BTreeMap<String, Arc<BTreeSet<BiomeId>>>,
}
impl BiomePredicateBindings {
    /// IDs must belong to the same registry snapshot as the world's biome query.
    pub fn new(
        names: &BTreeMap<String, BiomeId>,
        tags: &BTreeMap<String, Vec<String>>,
    ) -> Result<Self, DecodeError> {
        if names.len() > 8192 || tags.len() > 8192 {
            return Err(DecodeError::Field("predicate biome registry size"));
        }
        let mut canonical = BTreeMap::new();
        let mut ids = BTreeSet::new();
        for (name, id) in names {
            if canonical.insert(identifier(name)?, *id).is_some() || !ids.insert(*id) {
                return Err(DecodeError::Field("duplicate predicate biome identity"));
            }
        }
        let mut bound_tags = BTreeMap::new();
        for (name, entries) in tags {
            if entries.len() > 8192 {
                return Err(DecodeError::Field("predicate biome tag size"));
            }
            let values = entries
                .iter()
                .map(|entry| {
                    canonical
                        .get(&identifier(entry)?)
                        .copied()
                        .ok_or_else(|| DecodeError::Registry(format!("unknown biome {entry}")))
                })
                .collect::<Result<BTreeSet<_>, _>>()?;
            if bound_tags
                .insert(super::tags::identifier(name)?, Arc::new(values))
                .is_some()
            {
                return Err(DecodeError::Field("duplicate predicate biome tag"));
            }
        }
        Ok(Self {
            names: canonical,
            tags: bound_tags,
        })
    }
    pub fn resolve(&self, json: &str) -> Result<Arc<BTreeSet<BiomeId>>, DecodeError> {
        if let Ok(name) = serde_json::from_str::<String>(json) {
            if let Some(tag) = name.strip_prefix('#') {
                return self
                    .tags
                    .get(&super::tags::identifier(tag)?)
                    .cloned()
                    .ok_or_else(|| DecodeError::Registry(format!("unknown biome tag {tag}")));
            }
            return Ok(Arc::new(BTreeSet::from([self.biome(&name)?])));
        }
        let names: Vec<String> = serde_json::from_str(json)?;
        if names.len() > 8192 {
            return Err(DecodeError::Field("predicate biome holder list"));
        }
        Ok(Arc::new(
            names
                .iter()
                .map(|name| self.biome(name))
                .collect::<Result<_, _>>()?,
        ))
    }
    fn biome(&self, name: &str) -> Result<BiomeId, DecodeError> {
        self.names
            .get(&identifier(name)?)
            .copied()
            .ok_or_else(|| DecodeError::Registry(format!("unknown biome {name}")))
    }
}
