//! The `apply_enchantments` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// NeoForge's `IItemExtension.applyEnchantments`: the enchanting table applies
    /// `enchantments`, the list it chose for the clicked row as (id, level), to `stack`, the
    /// item in its input slot (`EnchantmentMenu.clickMenuButton`). `result` arrives with
    /// vanilla's result: a book becomes an enchanted book with the same count and components,
    /// then each level is added as `ItemStack.enchant` adds it, keeping the higher one.
    /// Replace or change it, and the input slot holds it; `player_enchant_item`, the
    /// `enchanted_item` trigger and `enchant_item` then see it. The levels and lapis paid stay
    /// as without the part. The result goes through the stack boundary: an unknown item,
    /// invalid damage or an oversized count is refused, and so is a count other than the
    /// input's, since the table holds one input; a refused result keeps vanilla's. Only the
    /// `ItemPart`s attached to `stack`'s item are asked, on the server, with a read-only
    /// `Host`. Loot, trades and spawn gear (`EnchantmentHelper.enchantItem`) don't ask. The
    /// contract beyond the method's name and parameters is Horizon's reading, not checked
    /// against NeoForge's.
    #[hook(
        side = "server",
        contract = "listen",
        scope = "item",
        key = "stack",
        host = "read",
        neoforge = "IItemExtension.applyEnchantments"
    )]
    fn apply_enchantments();
}
