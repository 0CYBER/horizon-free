//! Per-stack crafting remainders.
pub trait Hooks {
    /// NeoForge's `IItemExtension.getCraftingRemainder(ItemStack)`: what `stack`
    /// leaves when consumed by crafting, furnace fuel or brewing. `remainder` arrives
    /// holding the item's data remainder. Replace it, or set it to `None` to
    /// leave nothing. A part that leaves it alone preserves the data remainder.
    /// Recipe-specific book and banner copies are handled by the recipe.
    /// Asked for each consumption on the authoritative server, with a read-only
    /// `Host`. Oversized counts, unknown items and invalid damage are refused by
    /// the stack boundary, retaining the original data remainder. API stacks
    /// cannot have count zero; use `None` to leave nothing. Native panics follow
    /// the normal Part crash path, attributed to the owning mod and Part.
    #[notes = r#"### Per-stack crafting remainders
`crafting_remainder` is an item Part listen query on the authoritative server,
with a read-only `Host`. Its event starts with the item's data remainder; changing
it to `None` leaves nothing. Grid consumption (including repeated quick moves),
the crafter, furnace fuel, and brewing fuel/ingredients use the same stack-aware resolver. Book and banner
recipe copies remain recipe-owned. `Host::item_crafting_remainder` reads data only.
The resolver checks the prepared item subscription table before converting or
cloning stack components. Invalid returned stacks are refused at `ItemBoundary`,
retaining the initial data remainder; native panics retain normal Part attribution."#]
    #[hook(
        side = "server",
        host = "read",
        contract = "listen",
        scope = "item",
        key = "stack",
        neoforge = "IItemExtension.getCraftingRemainder(ItemStack)"
    )]
    fn crafting_remainder();
}
