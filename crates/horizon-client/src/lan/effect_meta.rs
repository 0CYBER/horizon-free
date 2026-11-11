//! `LivingEntity.updateSynchronizedMobEffectParticles` for the tracked
//! entities: the particle each visible effect shows, looked up once per
//! (effect, ambient) and kept in the synced data (`SyncData::effect_meta`).

use horizon_protocol::entity_data::{Particle, ParticleData};
use std::collections::HashMap;

/// The particle of each effect, for each `ambient` flag (index 0 false,
/// 1 true). A `None` entry is an effect that shows nothing.
#[derive(Default)]
pub(crate) struct ParticleCache {
    shown: [HashMap<String, Option<Particle>>; 2],
}

impl ParticleCache {
    /// The particle for `effect` (without `minecraft:`), from the cache or
    /// else from `lookup`, which then runs once for that effect and flag.
    pub(crate) fn get_or_insert_with(
        &mut self,
        effect: &str,
        ambient: bool,
        lookup: impl FnOnce() -> Option<Particle>,
    ) -> Option<Particle> {
        let map = &mut self.shown[usize::from(ambient)];
        if let Some(particle) = map.get(effect) {
            return particle.clone();
        }
        let particle = lookup();
        map.insert(effect.to_owned(), particle.clone());
        particle
    }
}

/// The particle an effect shows, or none when the effect is unknown or its
/// particle type is not in the registry (`registry_id`).
pub(crate) fn particle_of(
    effect: &str,
    ambient: bool,
    registry_id: impl Fn(&str) -> Option<i32>,
) -> Option<Particle> {
    // MobEffect.particleFactory: the omens have their own particles,
    // the others an entity effect in their colour (faint if ambient).
    let (name, data) = match effect {
        "trial_omen" | "raid_omen" => (effect, None),
        _ => {
            let def = crate::brewing::effect(effect)?;
            let alpha = if ambient { 38 } else { 255 };
            let argb = (alpha << 24) | def.shown_color() as i32;
            ("entity_effect", Some(ParticleData::Color(argb)))
        }
    };
    let id = registry_id(&format!("minecraft:{name}"))?;
    Some(Particle { id, data })
}
