//! `LivingEntity.updateSynchronizedMobEffectParticles` for the tracked
//! entities: the particle each visible effect shows, looked up once per
//! (effect, ambient) and kept in the hub.

use super::Hub;
use horizon_protocol::entity_data::{Particle, ParticleData, Value, serializer};
use std::collections::HashMap;

/// The particle of each effect, for each `ambient` flag (index 0 false,
/// 1 true). A `None` entry is an effect that shows nothing.
#[derive(Default)]
pub(super) struct ParticleCache {
    shown: [HashMap<String, Option<Particle>>; 2],
}

impl ParticleCache {
    /// The particle for `effect` (without `minecraft:`), from the cache or
    /// else from `lookup`, which then runs once for that effect and flag.
    pub(super) fn get_or_insert_with(
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
pub(super) fn particle_of(
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

impl Hub {
    /// `LivingEntity.updateSynchronizedMobEffectParticles`: DATA_EFFECT_PARTICLES
    /// (10) and DATA_EFFECT_AMBIENCE_ID (11) for the visible `effects`, as
    /// (effect, ambient). The entries are left out when there is nothing to
    /// show, which is their default.
    pub fn effect_meta<'a>(
        &self,
        effects: impl IntoIterator<Item = (&'a str, bool)>,
    ) -> Vec<(u8, i32, Value)> {
        let mut cache = self
            .effect_particles
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let mut particles = Vec::new();
        let mut all_ambient = true;
        for (effect, ambient) in effects {
            let effect = effect.trim_start_matches("minecraft:");
            let particle = cache.get_or_insert_with(effect, ambient, || {
                particle_of(effect, ambient, |name| {
                    self.registry_id("minecraft:particle_type", name)
                })
            });
            let Some(particle) = particle else {
                continue;
            };
            particles.push(particle);
            all_ambient &= ambient;
        }
        if particles.is_empty() {
            return Vec::new();
        }
        let mut meta = vec![(10, serializer::PARTICLES, Value::Particles(particles))];
        if all_ambient {
            meta.push((11, serializer::BOOLEAN, Value::Bool(true)));
        }
        meta
    }
}
