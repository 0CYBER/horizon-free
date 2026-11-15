//! The `item_tooltip` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `ItemTooltipEvent`: the whole tooltip of `stack` (`ItemStack.getTooltipLines`),
    /// once its name, the item's own lines, the components' lines, the
    /// advanced lines and the operator warning are in `lines`, each with a
    /// colour as `rgba` packs it (`u32::MAX` keeps the text's own). Add,
    /// change, reorder or remove lines. `player` and `player_entity_id` are
    /// the local player's, `display` the stack's `minecraft:tooltip_display`,
    /// `advanced` (F3+H) and `creative` (a creative tab's own slot) its
    /// `TooltipFlag`. Asked each frame the tooltip shows, never for one the
    /// stack hides or a crossbow's loaded projectile. Client-side.
    /// This global event is distinct from [`ItemPart::tooltip`], which runs at
    /// the earlier `Item.appendHoverText` position, before component lines.
    /// Parts' lines therefore come first; this event sees the finished list
    /// including them, for any item, even one with no attached parts.
    /// A component deeper than 512 nodes refuses capture or the whole writeback;
    /// no event field is applied if any Text field fails conversion.
    #[hook(side = "client", contract = "listen", neoforge = "ItemTooltipEvent")]
    fn item_tooltip(
        player: u128,
        player_entity_id: i32,
        stack: &ItemStack,
        display: &TooltipDisplay<'_>,
        advanced: bool,
        creative: bool,
        lines: &mut Vec<(Text, u32)>,
    );
}
