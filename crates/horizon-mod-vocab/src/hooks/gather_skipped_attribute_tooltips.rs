//! The `gather_skipped_attribute_tooltips` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `GatherSkippedAttributeTooltipsEvent`: the attribute modifiers a stack's tooltip leaves out
    /// (`ItemStack.addAttributeTooltips`), asked once before its attribute lines are built. Set
    /// `skip_all` to hide every one, push modifier ids (`minecraft:base_attack_damage`, a bare
    /// name is `minecraft:`'s) to `skipped_ids`, or `EquipmentSlotGroup` names (`mainhand`,
    /// `armor`, `any`) to `skipped_groups`; a slot group with nothing left shows no heading. Not
    /// asked for a stack whose `minecraft:tooltip_display` hides the attribute modifiers. `player`
    /// and `player_entity_id` are the local player's, `None` for the recipe book's search text and for a stack nested in another's tooltip (a
    /// crossbow's loaded projectile),
    /// which has none; `advanced` (F3+H) and `creative` are the tooltip's `TooltipFlag`. Asked each
    /// frame the tooltip shows, only while a mod listens. Client-side.
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "GatherSkippedAttributeTooltipsEvent",
        cancellable = false
    )]
    fn gather_skipped_attribute_tooltips(
        player: Option<u128>,
        player_entity_id: Option<i32>,
        stack: &ItemStack,
        advanced: bool,
        creative: bool,
        skip_all: &mut bool,
        skipped_ids: &mut Vec<String>,
        skipped_groups: &mut Vec<String>,
    );
}
