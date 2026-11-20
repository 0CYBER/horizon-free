//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;

/// The server host's methods of the `mod_enchantments` area.
#[horizon_mod_api_macros::host_area]
pub trait ModEnchantmentsServerHost {
    /// Side: server. Ask `get_enchantment_level_for` with the wearer `owner`
    /// and the caller's `mod_slot` named, so a mod can apply a per-wearer
    /// level. Call this from the mod's own equip moment; the engine's
    /// ordinary stack reads keep using `get_enchantment_level`.
    #[access(WorldOnly)]
    fn enchantment_level_for(
        &self,
        owner: EntityRef,
        mod_slot: Option<&str>,
        stack: &ItemStack,
        target: Option<&Identifier>,
        enchantments: &mut ItemEnchantments,
    );
}
