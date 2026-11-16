use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// `LivingVisibilityEvent` (`LivingEntity.getVisibilityPercent`): how
    /// visible `entity` is to `looking`, which scales the range a targeting
    /// check sees it from. `visibility` comes in as vanilla's percent.
    pub(in crate::game) fn native_living_visibility(
        &self,
        entity: LivingRef,
        looking: Option<EntityRef>,
        visibility: &mut f64,
    ) {
        self.fire_living_visibility(entity, looking, visibility);
    }

    /// What a mod's `living_visibility` makes of `visibility` for the entity
    /// `target` gives from the lifecycle's epoch, as mob `looking` sees it:
    /// `visibility` unchanged when no mod listens.
    pub(in crate::game) fn scaled_visibility(
        &self,
        target: impl FnOnce(mod_api::WorldEpoch) -> EntityRef,
        looking: u32,
        visibility: f64,
    ) -> f64 {
        if !mod_api::__private::listened(mod_api::HookId::LivingVisibility) {
            return visibility;
        }
        let Some(epoch) = self.native_lifecycle.epoch() else {
            return visibility;
        };
        let mut visibility = visibility;
        self.native_living_visibility(
            LivingRef::new(target(epoch)),
            Some(EntityRef::new(epoch, looking)),
            &mut visibility,
        );
        // CommonHooks.getEntityVisibilityMultiplier: never below 0.
        visibility.max(0.0)
    }
}
