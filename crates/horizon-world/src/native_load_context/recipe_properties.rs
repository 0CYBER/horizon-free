//! Immutable property membership carried by menu loop generations.
use super::LoopContext;
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};
impl LoopContext {
    /// Install the owning resource generation's mod-created property sets.
    /// Readers already holding another context keep their previous generation.
    pub fn set_recipe_properties(&mut self, sets: Arc<HashMap<String, HashSet<String>>>) {
        self.recipe_properties = sets;
    }
    /// Membership of a mod-created set, without allocation or registry scans.
    /// Vanilla acceptance is handled by the owning station's recipe projection.
    pub fn mod_recipe_property_contains(&self, set: &str, item: &str) -> bool {
        self.recipe_properties
            .get(set)
            .is_some_and(|items| items.contains(item))
    }
}

#[cfg(test)]
mod tests {
    use super::super::{DataLoadOrigin, LoadContext, LoadInputs, LoopSide};
    use super::*;
    #[test]
    fn captured_loop_keeps_its_membership_after_replacement() {
        let load = LoadContext::new(
            DataLoadOrigin::ServerDataLoad,
            Arc::new(LoadInputs::new(Default::default(), Default::default())),
            [],
        );
        let mut current = load.loop_context(LoopSide::Server, []);
        current.set_recipe_properties(Arc::new(HashMap::from([(
            "pot:input".into(),
            HashSet::from(["minecraft:emerald".into()]),
        )])));
        let captured = current.clone();
        current.set_recipe_properties(Default::default());
        assert!(!current.mod_recipe_property_contains("pot:input", "minecraft:emerald"));
        assert!(captured.mod_recipe_property_contains("pot:input", "minecraft:emerald"));
        assert!(!captured.mod_recipe_property_contains("pot:input", "minecraft:diamond"));
        assert!(!captured.mod_recipe_property_contains("missing:input", "minecraft:emerald"));
    }
}
