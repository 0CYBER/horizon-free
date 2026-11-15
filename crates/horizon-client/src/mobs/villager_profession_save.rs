//! Saved profession identity is independent of the active registry's slot order.
use crate::{
    mobs::{Mob, Species, villager::ProfessionState},
    save::text,
};
use horizon_core::nbt::Tag;
/// `VillagerData`: type, profession and level.
pub fn villager_data(m: &Mob) -> Tag {
    use crate::mobs::render::VILLAGER_TYPES;
    Tag::compound([
        (
            "type",
            string(&format!(
                "minecraft:{}",
                VILLAGER_TYPES[m.extra.variant as usize % VILLAGER_TYPES.len()]
            )),
        ),
        ("profession", string(&m.extra.profession.id())),
        ("level", Tag::Int(m.extra.villager_level.max(1))),
    ])
}

/// Decode a stored identity without consulting a process-global custom registry.
/// The game resolves it against its owning world only after entity reconstruction.
pub fn read_identity(m: &mut Mob, data: &Tag) {
    m.extra.profession = text(data.get("profession"))
        .as_deref()
        .map(ProfessionState::from_id)
        .unwrap_or_default();
}

impl super::ResolvedProfessions {
    /// Rebind a loaded villager or zombie villager without changing any trade state.
    pub fn resolve_loaded(&self, mob: &mut Mob) {
        if !matches!(mob.species(), Species::Villager | Species::ZombieVillager) {
            return;
        }
        match &mob.extra.profession {
            ProfessionState::Unresolved(name) => {
                if let Some(index) = self.resolve(name)
                    && let Some(state) = self.state(index)
                {
                    mob.extra.profession = state;
                }
            }
            ProfessionState::Registered { id: name, .. } => {
                mob.extra.profession = self
                    .resolve(name)
                    .and_then(|slot| self.state(slot))
                    .unwrap_or_else(|| ProfessionState::Unresolved(name.to_string()));
            }
            _ => {}
        }
    }
}

fn string(value: &str) -> Tag {
    Tag::String(value.into())
}
