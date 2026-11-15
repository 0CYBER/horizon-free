//! Authoritative server recipe properties, replaced as one resource generation.
use horizon_protocol::packets::recipes::{HolderSet, SlotDisplay, UpdateRecipes};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

/// The client's current server generation. None is distinct from an empty
/// generation: before the first packet local preload data may be used, but
/// after an empty replacement no previous set may survive.
#[derive(Default)]
pub struct ServerRecipeProperties {
    pub sets: Option<Arc<HashMap<String, HashSet<i32>>>>,
    pub stonecutter: Arc<[(HolderSet, SlotDisplay)]>,
}
impl ServerRecipeProperties {
    /// Replace, never merge, including on data-pack reload.
    pub fn replace(&mut self, packet: UpdateRecipes) {
        self.sets = Some(Arc::new(
            packet
                .properties
                .into_iter()
                .map(|(key, items)| (key, items.into_iter().collect()))
                .collect(),
        ));
        self.stonecutter = packet.stonecutter.into();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reload_replaces_sets_and_choices_without_mutating_captured_generation() {
        let mut state = ServerRecipeProperties::default();
        assert!(state.sets.is_none());
        state.replace(UpdateRecipes {
            properties: vec![("mod:input".into(), vec![4, 7, 4])],
            stonecutter: vec![(HolderSet::Items(vec![4]), SlotDisplay::Empty)],
        });
        let captured = state.sets.clone().unwrap();
        assert_eq!(captured["mod:input"], HashSet::from([4, 7]));
        state.replace(UpdateRecipes::default());
        assert!(state.sets.as_ref().unwrap().is_empty());
        assert!(state.stonecutter.is_empty());
        assert!(captured["mod:input"].contains(&7));
    }
}
