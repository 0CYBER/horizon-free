//! World-owned biome registry identities.
use crate::Host;
use std::collections::BTreeMap;

/// A biome registry entry in one installed world data generation.
/// Obtain it with `host.biome_key("minecraft:plains")`; reacquire after reload.
// no wire: explicit slot and generation codec in wire_types.rs carries opaque tokens; the receiving backend validates its own snapshot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct BiomeKey(pub(crate) u32, pub(crate) u64);

impl BiomeKey {
    /// This entry's canonical id, or a named refusal from the receiving world's snapshot.
    pub fn id(self, host: &dyn Host) -> Result<String, BiomeKeyError> {
        crate::backend_of(host)
            .ok_or(BiomeKeyError::Unavailable)?
            .biome_key_id(self)
    }
}

/// Why a biome key cannot be read against this host.
#[derive(Clone, Copy, Debug, PartialEq, Eq, horizon_mod_api_macros::Wire)]
#[non_exhaustive]
pub enum BiomeKeyError {
    /// The token is not in this installed generation: expired, from another world,
    /// or never issued. These cases cannot be distinguished from the opaque token.
    WrongGeneration,
    /// No server world registry is installed on this host.
    Unavailable,
}
impl crate::NoBackend for BiomeKeyError {
    fn no_backend() -> Self {
        Self::Unavailable
    }
}

/// Engine-owned identities of one world data snapshot; dropping it invalidates its tokens.
// no wire: engine-owned snapshot storage, never passed to mods.
#[non_exhaustive]
#[derive(Default)]
pub struct BiomeKeys {
    generation: u64,
    names: BTreeMap<String, BiomeKey>,
    ids: BTreeMap<u32, String>,
}
impl BiomeKeys {
    /// Prepare the engine's unique world-data generation from its registry before installation.
    /// The engine must supply a fresh generation for every candidate, across worlds and reloads.
    pub fn new<'a>(generation: u64, ids: impl IntoIterator<Item = &'a str>) -> Self {
        let mut result = Self {
            generation,
            ..Self::default()
        };
        for (index, id) in ids.into_iter().enumerate() {
            if result.names.contains_key(id) {
                continue;
            }
            let Ok(slot) = u32::try_from(index) else {
                break;
            };
            let name = id.to_owned();
            result
                .names
                .insert(id.to_owned(), BiomeKey(slot, generation));
            result.ids.insert(slot, name);
        }
        result
    }
    /// Find a name in this installed generation.
    pub fn key(&self, id: &str) -> Option<BiomeKey> {
        self.names.get(id).copied()
    }
    /// Refuse tokens issued by any other world or data generation.
    pub fn id(&self, key: BiomeKey) -> Result<&str, BiomeKeyError> {
        if key.1 != self.generation {
            return Err(BiomeKeyError::WrongGeneration);
        }
        self.ids
            .get(&key.0)
            .map(String::as_str)
            .ok_or(BiomeKeyError::WrongGeneration)
    }
}

#[cfg(test)]
#[path = "biome_key_tests.rs"]
mod tests;
