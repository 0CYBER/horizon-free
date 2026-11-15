//! The `tooltip` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0036

pub trait Hooks {
    /// `Item.appendHoverText`: an item's own lines in its tooltip, after its
    /// name and before its components' (enchantments, lore, attributes). Add
    /// lines to `lines`, which arrives empty, each with a colour as `rgba` packs
    /// it (`u32::MAX` keeps the text's own); `Text::legacy` reads `§` codes.
    /// The stack has its components (`stack.data()`). Asked each frame the
    /// tooltip shows, never for one the stack hides or a crossbow's loaded
    /// projectile. This is the scoped `Item.appendHoverText` moment, not
    /// NeoForge's global `ItemTooltipEvent`: attached parts add their lines here
    /// before component lines are assembled. Afterwards [`Hooks::item_tooltip`]
    /// sees the finished list, including these parts' lines.
    #[hook(
        side = "client",
        contract = "collect",
        out = "lines",
        scope = "item",
        key = "stack",
        neoforge = "none (vanilla Item.appendHoverText; not NeoForge's global ItemTooltipEvent)"
    )]
    fn tooltip();
}
