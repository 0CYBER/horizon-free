//! The target of `item_attribute_modifier` (`ItemAttributeModifierEvent`).
use crate::item::item_stack::ItemStack;
#[allow(unused_imports)]
use crate::*;
/// `ItemAttributeModifierEvent`: the attribute modifiers `stack` grants
/// (`ItemStack.getAttributeModifiers`), starting from its own
/// `minecraft:attribute_modifiers` or its item's. Add, replace or remove
/// entries in `modifiers`; what is left is what every reader of the stack's
/// modifiers uses: the equipment of players, mobs, armor stands and the
/// dragon when it changes, mobs' gear knockback and pickup choices, the
/// attribute sync and the tooltip. A worn stack is asked again only when it
/// changes (`ItemStack.matches`), so the answer must be the same for the same
/// stack. Asked on the server and on a client (its tooltips).
#[inline(never)]
pub fn native_item_attribute_modifier(stack: &ItemStack, modifiers: &mut ItemAttributeModifiers) {
    crate::dispatch_item_attribute_modifier(stack, modifiers);
}
