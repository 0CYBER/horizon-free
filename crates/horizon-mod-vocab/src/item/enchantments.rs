//! The target of `get_enchantment_level` (`GetEnchantmentLevelEvent`).
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
