//! The `add_attribute_tooltips` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `AddAttributeTooltipsEvent`: the lines that follow a stack's attribute lines
    /// (`ItemStack.addAttributeTooltips`), after the modifiers the stack shows and
    /// `gather_skipped_attribute_tooltips` left in. Push lines to `lines`, each with a colour as
    /// `rgba` packs it (`u32::MAX` keeps the text's own); they land beside the attribute lines
    /// without finding where those end. `shown` is whether the stack's
    /// `minecraft:tooltip_display` shows the attribute modifiers (`shouldShow`); the hook is
    /// asked either way. `player` and `player_entity_id` are the local player's,
    /// `None` for the recipe book's search text, which has none and drops the colours, and for a stack
    /// nested in another's tooltip (a crossbow's loaded projectile); `advanced`
    /// (F3+H) and `creative` (a creative tab's own slot) are the tooltip's `TooltipFlag`. Asked
    /// each frame the tooltip shows, only while a mod listens. Client-side.
    /// A component deeper than 512 nodes refuses capture or the whole writeback;
    /// no event field is applied if any Text field fails conversion.
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "AddAttributeTooltipsEvent",
        cancellable = false
    )]
    fn add_attribute_tooltips(
        player: Option<u128>,
        player_entity_id: Option<i32>,
        stack: &ItemStack,
        shown: bool,
        advanced: bool,
        creative: bool,
        lines: &mut Vec<(Text, u32)>,
    );
}
