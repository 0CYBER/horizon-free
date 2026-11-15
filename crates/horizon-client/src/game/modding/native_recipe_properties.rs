//! One mod-created property-set generation shared by Host and menu prediction.
use super::*;
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

impl Game<'_> {
    /// Mod-created recipe input membership in the current owning generation.
    pub(in crate::game) fn mod_recipe_property_contains(
        &self,
        set: &str,
        item: mod_api::ItemKey,
    ) -> bool {
        mod_api::__private::item_key_id(item)
            .is_some_and(|name| self.loop_context.mod_recipe_property_contains(set, name))
    }

    /// Publish once per owning load or server update, never from a slot read.
    pub(super) fn refresh_recipe_property_context(&mut self) {
        let mut sets = HashMap::<String, HashSet<String>>::new();
        if let Some(server) = &self.recipe_ui.server_properties.sets {
            if let Some(remote) = &self.remote {
                for (key, ids) in server
                    .iter()
                    .filter(|(key, _)| !key.starts_with("minecraft:"))
                {
                    let items = ids
                        .iter()
                        .filter_map(|id| remote.builtin.get("minecraft:item")?.name(*id))
                        .filter(|name| *name != "minecraft:air")
                        .map(str::to_owned)
                        .collect();
                    sets.insert(key.clone(), items);
                }
            }
        } else if self.world_tables.is_some() || self.res.rest.get().is_some() {
            for (key, names) in self.recipes().custom_property_sets() {
                let items = names
                    .iter()
                    .filter(|name| self.res.items().get(name).is_some())
                    .cloned()
                    .collect();
                sets.insert(key.to_owned(), items);
            }
        }
        Arc::make_mut(&mut self.loop_context).set_recipe_properties(Arc::new(sets));
        if let Some(host) = self.lan.as_mut() {
            host.hub.loop_context = self.loop_context.clone();
        }
    }
}
