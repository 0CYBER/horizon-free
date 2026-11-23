use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// The Ender Dragon `dragon` is about to change phase from `current` to
    /// `new_phase` (`EnderDragonPhaseManager.setPhase`). A listener may write
    /// another phase to `new_phase`; set `cancel` to keep `current`. Server-side.
    pub(in crate::game) fn native_dragon_change_phase(
        &mut self,
        dragon: mod_api::DragonRef,
        current: mod_api::DragonPhase,
        new_phase: &mut mod_api::DragonPhase,
        cancel: &mut bool,
    ) {
        self.fire_dragon_change_phase(dragon, current, new_phase, cancel);
    }
}
