//! Block states mods add to vanilla point-of-interest types.
use crate::{BlockMetadata, Identifier, StatePredicate};

/// Registrar of the block states a mod adds to vanilla POI types (`extend_poi_types`).
pub trait PoiTypeExtensions {
    /// Block metadata for building the predicates, with every block resolved.
    fn metadata(&self) -> &dyn BlockMetadata;
    /// Add the states `states` selects to the POI type `poi_type` (`minecraft:home`).
    /// A block predicate with no constraints means all of the block's states.
    fn extend(&mut self, poi_type: Identifier, states: StatePredicate);
}

// no wire: borrowed registrar backend; access stays scoped to the owning call.
/// Engine-owned storage behind the registrar.
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct PoiTypeExtensionRegistry<'a> {
    metadata: &'a dyn BlockMetadata,
    entries: Vec<(Identifier, StatePredicate)>,
}
impl<'a> PoiTypeExtensionRegistry<'a> {
    /// An empty registrar over the resolved block metadata.
    // hidden: the engine builds the registrar before collecting the hook; not part of the mod API
    #[doc(hidden)]
    pub fn new(metadata: &'a dyn BlockMetadata) -> Self {
        Self {
            metadata,
            entries: Vec::new(),
        }
    }
    /// Engine-owned entries, in registration order.
    // hidden: the engine consumes registrations after collecting the hook; not part of the mod API
    #[doc(hidden)]
    pub fn into_entries(self) -> Vec<(Identifier, StatePredicate)> {
        self.entries
    }
}
impl PoiTypeExtensions for PoiTypeExtensionRegistry<'_> {
    fn metadata(&self) -> &dyn BlockMetadata {
        self.metadata
    }
    fn extend(&mut self, poi_type: Identifier, states: StatePredicate) {
        self.entries.push((poi_type, states));
    }
}
