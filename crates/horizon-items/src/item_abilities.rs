//! Default item abilities, shared by loot and future action-site queries (arismoko/horizon#3330).
use crate::items::Items;
use horizon_world::loot::LootTags;

/// Vanilla's complete default set. Ability names are plain and case-sensitive.
/// Uses resolved tag membership and borrowed strings; performs no allocations.
pub fn vanilla_can_perform(item: &str, ability: &str, tags: &dyn LootTags) -> bool {
    match ability {
        "sword_sweep" => tags.contains("item", "minecraft:swords", item),
        "shovel_douse" => tags.contains("item", "minecraft:douses_campfires", item),
        "shears_dig"
        | "shears_harvest"
        | "shears_remove_armor"
        | "shears_carve"
        | "shears_disarm"
        | "shears_trim" => item == "minecraft:shears",
        "fishing_rod_cast" => item == "minecraft:fishing_rod",
        "trident_throw" => item == "minecraft:trident",
        "brush_brush" => item == "minecraft:brush",
        "firestarter_light" => {
            matches!(item, "minecraft:flint_and_steel" | "minecraft:fire_charge")
        }
        "spyglass_scope" => item == "minecraft:spyglass",
        _ => false,
    }
}
impl Items {
    /// The item-layer default before any `can_perform_action` part override.
    pub fn vanilla_can_perform(&self, item: &str, ability: &str) -> bool {
        vanilla_can_perform(item, ability, self)
    }
}
