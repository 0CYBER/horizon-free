use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// `GatherSkippedAttributeTooltipsEvent`: the attribute modifiers a stack's tooltip leaves
    /// out. Set `skip_all`, or push modifier ids to `skipped_ids` or `EquipmentSlotGroup` names
    /// to `skipped_groups`. Asked once before the attribute lines are built, not for a stack
    /// whose display hides the attribute modifiers. `player` and `player_entity_id` are the
    /// local player's, `None` for the recipe book's search text and a stack nested in another's tooltip (a
    /// crossbow's projectile). Asked each frame the tooltip
    /// shows, only while a mod listens. Client-side.
    pub(in crate::game) fn native_gather_skipped_attribute_tooltips(
        &self,
        call: mod_api::__private::GatherSkippedAttributeTooltipsCall<'_>,
    ) {
        self.fire_gather_skipped_attribute_tooltips(call);
    }
}
