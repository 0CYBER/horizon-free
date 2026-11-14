//! World-owned biome registry identities.
use crate::Host;
use std::collections::BTreeMap;
use std::sync::{Arc, LazyLock, Mutex, Weak};

/// A biome registry entry in one installed world data generation.
/// Obtain it with `host.biome_key("minecraft:plains")`; reacquire after reload.
// no wire: explicit u32 slot codec in wire_types.rs validates host-issued tokens on the host.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct BiomeKey(pub(crate) u32);

impl BiomeKey {
    /// This entry's canonical id. A stale key is a boundary error.
    pub fn id(self, host: &dyn Host) -> String {
        crate::backend_of(host)
            .map(|backend| backend.biome_key_id(self))
            .unwrap_or_default()
    }
    #[cfg(any(not(target_arch = "wasm32"), test))]
    pub(crate) fn from_slot(slot: u32) -> Result<Self, &'static str> {
        let tokens = TOKENS.lock().unwrap();
        tokens
            .get(&slot)
            .and_then(Weak::upgrade)
            .map(|_| Self(slot))
            .ok_or("stale or unknown BiomeKey slot")
    }
}

static TOKENS: LazyLock<Mutex<BTreeMap<u32, Weak<String>>>> = LazyLock::new(Default::default);
static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Engine-owned identities of one world data snapshot; dropping it invalidates its tokens.
// no wire: engine-owned snapshot storage, never passed to mods.
#[non_exhaustive]
#[derive(Default)]
pub struct BiomeKeys {
    names: BTreeMap<String, BiomeKey>,
    ids: BTreeMap<u32, Arc<String>>,
}
impl BiomeKeys {
    /// Prepare a new generation from the world's registry, before installation.
    pub fn new<'a>(ids: impl IntoIterator<Item = &'a str>) -> Self {
        let mut result = Self::default();
        let mut tokens = TOKENS.lock().unwrap();
        tokens.retain(|_, id| id.strong_count() != 0);
        for id in ids {
            if result.names.contains_key(id) {
                continue;
            }
            let slot = NEXT
                .try_update(
                    std::sync::atomic::Ordering::Relaxed,
                    std::sync::atomic::Ordering::Relaxed,
                    |n| n.checked_add(1),
                )
                .expect("biome key slots exhausted");
            let name = Arc::new(id.to_owned());
            tokens.insert(slot, Arc::downgrade(&name));
            result.names.insert(id.to_owned(), BiomeKey(slot));
            result.ids.insert(slot, name);
        }
        result
    }
    /// Find a name in this installed generation.
    pub fn key(&self, id: &str) -> Option<BiomeKey> {
        self.names.get(id).copied()
    }
    /// Refuse tokens issued by any other world or data generation.
    pub fn id(&self, key: BiomeKey) -> Option<&str> {
        self.ids.get(&key.0).map(|id| id.as_str())
    }
}

#[cfg(test)]
#[path = "biome_key_tests.rs"]
mod tests;
