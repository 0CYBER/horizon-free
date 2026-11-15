use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// `EntityMobGriefingEvent` (`EventHooks.canEntityGrief`): may `entity`
    /// change the world? `can_grief` starts as the `mob_griefing` rule.
    pub(in crate::game) fn native_entity_mob_griefing(
        &self,
        entity: EntityRef,
        can_grief: &mut bool,
    ) {
        self.fire_entity_mob_griefing(entity, can_grief);
    }
}
