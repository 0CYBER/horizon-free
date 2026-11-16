//! Reload-stable definitions, shared by native and wasm mods.
use mod_api::*;
use std::sync::RwLock;

// ANCHOR: cache
/// Definitions owned by the one authoritative server.
#[derive(Default)]
pub struct Processing {
    recipes: RwLock<Vec<Definition>>,
}

struct Definition {
    ingredient: Ingredient,
    result: ItemStack,
}

impl Processing {
    fn clear(&self) {
        self.recipes.write().unwrap().clear();
    }

    fn publish(&self, host: &dyn Host) {
        let next = host
            .recipes_of_type("example:processing")
            .iter()
            .filter_map(|recipe| {
                Some(Definition {
                    ingredient: host
                        .recipe_ingredient(&recipe.json["ingredient"].to_string())
                        .ok()??,
                    result: host
                        .recipe_result(&recipe.json["result"].to_string())
                        .ok()??,
                })
            })
            .collect();
        *self.recipes.write().unwrap() = next;
    }

    /// Select against the current input and current host, never a remembered match.
    pub fn result(&self, host: &dyn Host, input: &ItemStack) -> Option<ItemStack> {
        let recipes = self.recipes.read().unwrap();
        recipes
            .iter()
            .find(|recipe| recipe.ingredient.test(host, input))
            .map(|recipe| recipe.result.clone())
    }
}
// ANCHOR_END: cache

// ANCHOR: publication
#[mod_api::horizon_mod]
impl Hooks for Processing {
    fn server_about_to_start(&self, _: &mut dyn ServerHost, _: &mut ServerAboutToStartEvent) {
        self.clear();
    }

    fn tags_updated(&self, host: HostSide<'_>, _: &mut TagsUpdatedEvent) {
        if let HostSide::Server(host) = host {
            self.publish(host);
        }
    }

    fn server_stopped(&self, _: &mut dyn ServerHost, _: &mut ServerStoppedEvent) {
        self.clear();
    }
}
// ANCHOR_END: publication
