//! The effect particle cache gives what the uncached lookup gives, and looks
//! each (effect, ambient) up once.
use crate::lan::hub::effect_meta::{ParticleCache, particle_of};
use horizon_protocol::entity_data::Particle;

/// The `minecraft:particle_type` names the test registry knows, by id.
const PARTICLE_TYPES: &[&str] = &[
    "minecraft:entity_effect",
    "minecraft:trial_omen",
    "minecraft:raid_omen",
];

fn particle_type_id(name: &str) -> Option<i32> {
    PARTICLE_TYPES
        .iter()
        .position(|known| *known == name)
        .map(|i| i as i32)
}

#[test]
fn cached_effect_particles_equal_the_uncached_lookup_and_are_looked_up_once() {
    let effects = [
        "speed",
        "slowness",
        "trial_omen",
        "raid_omen",
        "not_an_effect",
    ];
    let mut cache = ParticleCache::default();
    let mut lookups = 0;
    for _round in 0..2 {
        for effect in effects {
            for ambient in [false, true] {
                let uncached = particle_of(effect, ambient, particle_type_id);
                let cached = cache.get_or_insert_with(effect, ambient, || {
                    lookups += 1;
                    particle_of(effect, ambient, particle_type_id)
                });
                assert_eq!(cached, uncached, "{effect} ambient {ambient}");
            }
        }
    }
    // Ten (effect, ambient) pairs: the second round finds them all cached.
    assert_eq!(lookups, 10);
    // The omens have their own particle types, without data; an unknown
    // effect shows nothing.
    assert_eq!(
        particle_of("trial_omen", false, particle_type_id),
        Some(Particle { id: 1, data: None })
    );
    assert_eq!(particle_of("not_an_effect", false, particle_type_id), None);
}
