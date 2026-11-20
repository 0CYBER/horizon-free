use super::*;
use mod_api::ItemEnchantments;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// The game's target for `get_enchantment_level_for`: a thin forwarder
    /// that passes an immutable host and the caller's owner and place to the
    /// mod API's wearer-aware entry point. The engine never calls this on its
    /// own; a mod calls that entry point from its own equip moment.
    #[inline(never)]
    pub(in crate::game) fn native_get_enchantment_level_for(
        &self,
        stack: &ItemStack,
        target: Option<&Identifier>,
        owner: Option<EntityRef>,
        mod_slot: Option<&str>,
        enchantments: &mut ItemEnchantments,
    ) {
        self.fire_get_enchantment_level_for(stack, target, owner, mod_slot, enchantments);
    }
}

/// The host helper a mod calls from its own equip moment.
impl Game<'_> {
    pub(in crate::game) fn enchantment_level_for(
        &self,
        owner: EntityRef,
        mod_slot: Option<&str>,
        stack: &ItemStack,
        target: Option<&Identifier>,
        enchantments: &mut ItemEnchantments,
    ) {
        self.native_get_enchantment_level_for(stack, target, Some(owner), mod_slot, enchantments);
    }
}
