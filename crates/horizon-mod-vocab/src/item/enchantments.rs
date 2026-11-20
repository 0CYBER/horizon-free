//! The targets of `get_enchantment_level` and `get_enchantment_level_for`
//! (`GetEnchantmentLevelEvent` and `GetEnchantmentLevelForEvent`).
use crate::item::item_stack::ItemStack;
#[allow(unused_imports)]
use crate::*;
/// `GetEnchantmentLevelEvent`: the enchantments gameplay sees on `stack`,
/// starting from its own `minecraft:enchantments`. `target` is the one
/// enchantment a read asks for (`ItemStack.getEnchantmentLevel`), and then
/// only its level counts; `None` when every one is read
/// (`ItemStack.getAllEnchantments`). Set, upgrade or remove levels in
/// `enchantments`: what is left is what fights, mining, fishing, tridents,
/// crossbows, armor, loot and every other enchantment effect use. Tooltips,
/// the anvil, the grindstone, the enchanting table, saves and the network
/// keep the stack's own component. Asked on every such read, often many
/// times a tick, on the server and on a client (its tooltip modifiers).
#[inline(never)]
pub fn native_get_enchantment_level(
    stack: &ItemStack,
    target: Option<&Identifier>,
    enchantments: &mut ItemEnchantments,
) {
    crate::dispatch_get_enchantment_level(stack, target, enchantments);
}

/// `GetEnchantmentLevelForEvent`: the same read with the wearer `owner` and
/// the caller's `mod_slot` (`curios:ring`) named. Both are optional: `None`
/// for a caller that only has a stack. The game never calls this on its own;
/// a mod calls it from the moment it knows the wearer to apply a per-wearer
/// level. The original three-argument entry point above is unchanged and
/// keeps asking `get_enchantment_level` alone.
#[inline(never)]
pub fn native_get_enchantment_level_for(
    host: &dyn Host,
    stack: &ItemStack,
    target: Option<&Identifier>,
    owner: Option<EntityRef>,
    mod_slot: Option<&str>,
    enchantments: &mut ItemEnchantments,
) {
    crate::dispatch_get_enchantment_level_for(host, stack, target, owner, mod_slot, enchantments);
}
