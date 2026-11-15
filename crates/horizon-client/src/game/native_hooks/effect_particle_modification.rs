use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// NeoForge's `EffectParticleModificationEvent`, vanilla's
    /// `LivingEntity.updateSynchronizedMobEffectParticles`: one active
    /// `effect` of the living entity `entity`, hidden or not, as the server
    /// is about to send its swirl. `particle` starts as `original_particle`:
    /// replace it, or set it to None to go back to the original. Set
    /// `visible` to show or hide the swirl. Asked when the entity's effects
    /// change, only while a mod listens; the answers are sent until the next
    /// change. Server-side.
    pub(in crate::game) fn native_effect_particle_modification(
        &self,
        entity: LivingRef,
        effect: &ActiveEffect,
        original_particle: &ParticleOptions,
        particle: &mut Option<ParticleOptions>,
        visible: &mut bool,
    ) {
        self.fire_effect_particle_modification(
            entity,
            effect,
            original_particle,
            particle,
            visible,
        );
    }
}
