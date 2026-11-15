//! Named registry entries shared unchanged by native and wasm mods.
use crate::Identifier;

/// A registry and entry name, not a process-local key or a protocol numeric id.
/// Fields may be retained across reloads; resolve them against the current host before use.
#[derive(Clone, Debug, PartialEq, Eq, horizon_mod_api_macros::Wire)]
#[non_exhaustive]
pub struct RegistryEntry {
    /// The registry's canonical identifier.
    pub registry: Identifier,
    /// The entry's canonical identifier within that registry.
    pub id: Identifier,
}

impl RegistryEntry {
    /// Construct a plain named value. This does not claim that the entry exists.
    pub fn new(registry: Identifier, id: Identifier) -> Self {
        Self { registry, id }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::{Reader, Writer};

    #[test]
    fn named_entries_roundtrip_without_live_registries() {
        let entry = RegistryEntry::new(
            Identifier::parse("example:unknown_registry").unwrap(),
            Identifier::parse("other:unknown_entry").unwrap(),
        );
        let mut out = Writer::new();
        out.put(&entry);
        let mut input = Reader::new(out.bytes());
        assert_eq!(input.take::<RegistryEntry>().unwrap(), entry);
        for end in 0..out.bytes().len() {
            assert!(
                Reader::new(&out.bytes()[..end])
                    .take::<RegistryEntry>()
                    .is_err()
            );
        }
    }
}
