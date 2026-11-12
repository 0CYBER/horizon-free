//! The effect particles built through the particle type id cache equal the
//! uncached ones, each particle type is looked up once, and a resource pack's
//! new effect colour shows at once although the ids are cached.
use crate::lan::effect_meta::{ParticleTypeIds, particle_of};
use horizon_protocol::entity_data::{Particle, ParticleData};

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
fn cached_particle_type_ids_give_the_uncached_particles_and_are_looked_up_once() {
    let effects = [
        "speed",
        "slowness",
        "trial_omen",
        "raid_omen",
        "not_an_effect",
    ];
    let mut ids = ParticleTypeIds::default();
    let mut lookups = 0;
    for _round in 0..2 {
        for effect in effects {
            for ambient in [false, true] {
                let uncached = particle_of(effect, ambient, particle_type_id);
                let cached = particle_of(effect, ambient, |name| {
                    ids.get_or_insert_with(name, || {
                        lookups += 1;
                        particle_type_id(name)
                    })
                });
                assert_eq!(cached, uncached, "{effect} ambient {ambient}");
            }
        }
    }
    // Three particle types (entity_effect, trial_omen, raid_omen); an
    // unknown effect never asks for one.
    assert_eq!(lookups, 3);
    // The omens have their own particle types, without data; an unknown
    // effect shows nothing.
    assert_eq!(
        particle_of("trial_omen", false, particle_type_id),
        Some(Particle { id: 1, data: None })
    );
    assert_eq!(particle_of("not_an_effect", false, particle_type_id), None);
}

#[test]
fn a_pack_colour_change_shows_although_the_particle_type_id_is_cached() {
    let mut ids = ParticleTypeIds::default();
    let shown = |ids: &mut ParticleTypeIds| {
        particle_of("luck", false, |name| {
            ids.get_or_insert_with(name, || particle_type_id(name))
        })
    };
    let colour = |particle: Option<Particle>| match particle {
        Some(Particle {
            data: Some(ParticleData::Color(argb)),
            ..
        }) => argb & 0xFF_FFFF,
        other => panic!("not an entity effect: {other:?}"),
    };
    crate::brewing::set_pack_colors(Vec::new());
    let vanilla = colour(shown(&mut ids));
    let pack = if vanilla == 0x12_3456 {
        0x65_4321
    } else {
        0x12_3456
    };
    crate::brewing::set_pack_colors(vec![("luck".to_owned(), pack as u32)]);
    let after = colour(shown(&mut ids));
    crate::brewing::set_pack_colors(Vec::new());
    assert_eq!(after, pack, "the pack's colour, not the one first cached");
    assert_eq!(colour(shown(&mut ids)), vanilla);
}
