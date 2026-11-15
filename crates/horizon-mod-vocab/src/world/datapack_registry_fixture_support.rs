//! Fault injection available only to scoped engine tests.
use super::DatapackRegistrySet;
impl DatapackRegistrySet {
    /// Exhaust the generation clock to exercise the engine publication failure path.
    // hidden: fault injection for engine tests, gated by the testing feature.
    #[doc(hidden)]
    pub fn exhaust_generation_for_testing(&mut self) {
        self.generation = u64::MAX;
    }
}
impl DatapackRegistrySet {
    /// A set whose `registry` holds exactly `entries`, for engine tests of reads against a
    /// world's loaded datapack registries.
    // hidden: fault injection for engine tests, gated by the testing feature.
    #[doc(hidden)]
    pub fn with_entries_for_testing(registry: &str, entries: &[&str]) -> Self {
        use crate::world::datapack_registry::DatapackEntry;
        Self {
            session: 0,
            generation: 1,
            registries: std::collections::BTreeMap::from([(
                registry.to_owned(),
                entries
                    .iter()
                    .map(|entry| ((*entry).to_owned(), DatapackEntry { values: Vec::new() }))
                    .collect(),
            )]),
            tags: std::collections::BTreeMap::new(),
            received_registries: None,
        }
    }
}
