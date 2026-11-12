//! `LivingEntity.updateSynchronizedMobEffectParticles` for the tracked
//! entities: the particle each visible effect shows. The particle types'
//! registry ids are looked up once and kept in the synced data
//! (`SyncData::effect_meta`); an effect's colour is read on every call, as
//! resource packs can change it while the world is open (`set_pack_colors`).

use horizon_protocol::entity_data::{Particle, ParticleData};
use std::collections::HashMap;

/// The `minecraft:particle_type` registry id of each particle type an effect
/// can show, by its name with namespace. A `None` entry is a name the
/// registry lacks.
#[derive(Default)]
pub(crate) struct ParticleTypeIds {
    ids: HashMap<String, Option<i32>>,
}

impl ParticleTypeIds {
    /// The id of particle type `name`, from the cache or else from `lookup`,
    /// which then runs once for that name.
    pub(crate) fn get_or_insert_with(
        &mut self,
        name: &str,
        lookup: impl FnOnce() -> Option<i32>,
    ) -> Option<i32> {
        if let Some(id) = self.ids.get(name) {
            return *id;
        }
        let id = lookup();
        self.ids.insert(name.to_owned(), id);
        id
    }
}

/// The particle an effect shows, or none when the effect is unknown or its
/// particle type is not in the registry (`registry_id`). The colour is the
/// effect's as the packs show it now.
pub(crate) fn particle_of(
    effect: &str,
    ambient: bool,
    registry_id: impl FnOnce(&str) -> Option<i32>,
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
