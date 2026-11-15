//! The `get_enchantment_level` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
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
    #[hook(
        side = "both",
        contract = "listen",
        neoforge = "GetEnchantmentLevelEvent",
        target = "mod_api::item::enchantments::native_get_enchantment_level"
    )]
    fn get_enchantment_level();
}
