//! The game's answer to `get_enchantment_level` (NeoForge's
//! `GetEnchantmentLevelEvent`), installed at startup so a stack's gameplay
//! enchantment reads ([`ItemStack::all_enchantments`],
//! [`ItemStack::enchantment_level`]) can ask the mods from this crate.
use crate::item_data::Levels;
use crate::items::ItemStack;
use std::sync::{Arc, OnceLock};

/// The mods' answer for `stack`: `target` is the enchantment one read asks
/// for, or `None` when every one is read; `levels` is the stack's own
/// `minecraft:enchantments`. `None` when no mod changed them.
pub type GetEnchantmentLevel = fn(&ItemStack, Option<&str>, &[(Arc<str>, u32)]) -> Option<Levels>;

static HOOK: OnceLock<GetEnchantmentLevel> = OnceLock::new();

/// Install the callback once; a second installation keeps the first.
pub fn install(hook: GetEnchantmentLevel) -> Result<(), GetEnchantmentLevel> {
    HOOK.set(hook)
}

pub(crate) fn get_enchantment_level(
    stack: &ItemStack,
    target: Option<&str>,
    levels: &[(Arc<str>, u32)],
) -> Option<Levels> {
    HOOK.get().and_then(|hook| hook(stack, target, levels))
}
