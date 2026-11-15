//! The `is_primary_item_for` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// NeoForge's `IItemExtension.isPrimaryItemFor`: is `stack`'s item primary for
    /// `enchantment` (an id such as `minecraft:sharpness`), so that enchantment selection may
    /// offer it? Selection (`EnchantmentHelper.getAvailableEnchantmentResults`) chooses the
    /// enchanting table's offers and clues, loot `enchant_with_levels`, trades and spawn gear.
    /// A book skips the question and is offered every enchantment. `primary` arrives with
    /// vanilla's `Enchantment.isPrimaryItem`: the item is in the enchantment's
    /// `supported_items`, and in its `primary_items` when that is set; set it `true` to offer
    /// the enchantment to the item or `false` to withhold it. The answer is per item and
    /// enchantment: selection runs inside rolls that can't ask a part, so each item's parts
    /// are asked once per registered enchantment when parts install (world open and
    /// `/reload`), with `stack` the item's default stack, and selection reads the answers
    /// stored on that server's enchantment registry until its next install or its world
    /// closes. Answer from the item and `enchantment` alone. Only the `ItemPart`s attached to
    /// `stack`'s item are asked, on the logical server, with a read-only `Host`. The contract
    /// beyond the method's name and parameters is Horizon's reading, not checked against
    /// NeoForge's.
    #[hook(
        side = "server",
        contract = "listen",
        scope = "item",
        key = "stack",
        host = "read",
        neoforge = "IItemExtension.isPrimaryItemFor"
    )]
    fn is_primary_item_for();
}
