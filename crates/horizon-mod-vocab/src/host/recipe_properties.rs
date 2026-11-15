//! Membership queries for mod-created recipe input sets.
#![allow(unused_variables)]
use crate::ItemKey;
/// The host's read-only mod recipe property membership service.
#[horizon_mod_api_macros::host_area]
pub trait RecipePropertiesHost {
    /// Side: both.
    /// Whether a mod-created recipe property set accepts an item ID. Unknown
    /// sets/items and vanilla keys return false. The server uses its current
    /// load, and clients use the server's last synchronized generation (local
    /// preload before the first packet). The same read on `LoopContext` lets
    /// `menu_slot_rules` filter a slot identically for server and prediction.
    /// Membership reads allocate nothing and perform no registry scans.
    #[access(PublishedResources)]
    fn mod_recipe_property_contains(&self, set: &str, item: ItemKey) -> bool;
}
