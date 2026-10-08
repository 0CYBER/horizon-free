//! Immutable resource snapshots for named state-provider holders.
use super::{BlockStateProvider, DecodeError, StateProviderBindings, identifier};
use serde_json::value::RawValue;
use std::collections::BTreeMap;

pub struct StateProviderCatalog {
    resources: BTreeMap<String, Box<RawValue>>,
}
impl StateProviderCatalog {
    /// Store resources without evaluating unused definitions. Resolving a root
    /// validates its reachable providers, including branches not sampled at run
    /// time. Cycles and excessive expansion fail the shared depth/work budget.
    pub fn from_resources(resources: &BTreeMap<String, String>) -> Result<Self, DecodeError> {
        if resources.len() > 8192 {
            return Err(DecodeError::Field("state provider resource count"));
        }
        let mut result = BTreeMap::new();
        for (name, json) in resources {
            if result
                .insert(identifier(name)?, serde_json::from_str(json)?)
                .is_some()
            {
                return Err(DecodeError::Field("duplicate state provider resource"));
            }
        }
        Ok(Self { resources: result })
    }
    pub(super) fn lookup(&self, name: &str) -> Option<&str> {
        self.resources.get(name).map(|v| v.get())
    }
    /// Decode the holder codec: an inline direct provider or a registry name.
    pub fn provider(
        &self,
        json: &str,
        bindings: &StateProviderBindings<'_>,
    ) -> Result<BlockStateProvider, DecodeError> {
        BlockStateProvider::decode_holder(json, bindings, 0, &mut 65536, Some(self))
    }
    pub(crate) fn provider_with_budget(
        &self,
        json: &str,
        bindings: &StateProviderBindings<'_>,
        depth: usize,
        remaining: &mut usize,
    ) -> Result<BlockStateProvider, DecodeError> {
        BlockStateProvider::decode_holder(json, bindings, depth, remaining, Some(self))
    }
    /// Decode DIRECT_CODEC while allowing nested named holder references.
    pub fn direct_provider(
        &self,
        json: &str,
        bindings: &StateProviderBindings<'_>,
    ) -> Result<BlockStateProvider, DecodeError> {
        BlockStateProvider::decode(json, bindings, 0, &mut 65536, Some(self))
    }
    /// Decode TYPED_CODEC while allowing nested named holder references.
    pub fn typed_provider(
        &self,
        json: &str,
        bindings: &StateProviderBindings<'_>,
    ) -> Result<BlockStateProvider, DecodeError> {
        BlockStateProvider::decode_typed(json, bindings, 0, &mut 65536, Some(self))
    }
}
