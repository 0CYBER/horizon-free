//! Moved here from the game so mods can name it.
#[allow(unused_imports)]
use crate::*;
// no wire: borrowed call-scoped view; transport uses owned snapshots rather than serializing references.
/// One offer row as `EnchantmentLevelSetEvent` reports it: the server player
/// whose table it is (UUID and entity id), the row (0 to 2), the power (the
/// bookshelves counted), the item and the level `getEnchantmentCost` drew.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
pub struct EnchantmentRow<'a> {
    pub player: u128,
    pub player_entity_id: i32,
    pub row: i32,
    pub power: i32,
    pub item: &'a ItemStack,
    pub original_level: i32,
}
#[derive(Clone, Debug, Default, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(horizon_mod_api_macros::Wire)]
pub struct AnvilResult {
    pub result: Option<ItemStack>,
    /// Level cost (the `cost` data slot).
    pub cost: i32,
    /// Material items consumed from the second slot on take.
    pub repair_item_count_cost: u32,
    pub only_renaming: bool,
}
