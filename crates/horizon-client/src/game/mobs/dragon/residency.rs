//! Dragon-owned identity and attachment metadata.
use super::*;
#[derive(Default)]
pub(in crate::game) struct DragonResidency {
    pub(super) lifetime: horizon_mod_runtime::attachments::AttachmentLifetime,
}

/// A real sub-entity: geometry, identity and attachment authority move and
/// retire together with this part, never with a shared parent attachment map.
pub(in crate::game) struct DragonPart(
    pub Vec3,
    pub Vec3,
    pub(in crate::game) horizon_mod_runtime::attachments::AttachmentLifetime,
    pub(in crate::game) [i32; 4],
);
impl Default for DragonPart {
    fn default() -> Self {
        Self(
            Vec3::ZERO,
            Vec3::ZERO,
            Default::default(),
            horizon_client::save::insecure_uuid(&mut random_source::entity()),
        )
    }
}
impl horizon_mod_runtime::attachments::HasAttachmentLifetime for DragonPart {
    fn attachment_lifetime(&mut self) -> &mut horizon_mod_runtime::attachments::AttachmentLifetime {
        &mut self.2
    }
}
impl horizon_mod_runtime::attachments::HasAttachmentLifetime for Dragon {
    fn attachment_lifetime(&mut self) -> &mut horizon_mod_runtime::attachments::AttachmentLifetime {
        &mut self.residency.lifetime
    }
}

impl Game<'_> {
    /// Install a replacement only after retiring the previous parts' sidecars.
    pub(in crate::game) fn replace_resident_dragon(&mut self, dragon: Dragon) {
        self.discard_resident_dragon();
        self.dragon.replace(dragon);
    }

    /// Physical retirement must forget sidecar data before IDs can be reused,
    /// not merely unregister the attachment index when the parts are dropped.
    pub(in crate::game) fn discard_resident_dragon(&mut self) {
        if let Some(id) = self
            .dragon
            .as_ref()
            .map(|dragon| dragon.id)
            .filter(|id| *id != 0)
        {
            for offset in 1..=8 {
                if let Some(part) = id.checked_add(offset) {
                    self.forget_entity(part);
                }
            }
            // The parent's own sidecars go with it too, its `noPhysics` write
            // included; the parts above are its separate entity ids.
            self.forget_entity(id);
        }
        self.dragon.discard();
    }

    /// The dragon the level holds and the index (`getSubEntities` order) of
    /// its part with that entity id, while the dragon is there, dying or
    /// not: a part is an entity for as long as its parent is.
    pub(in crate::game) fn resident_dragon_part_source(&self, id: u32) -> Option<(&Dragon, usize)> {
        use horizon_client::join_seq::residency::{Source, table::EntityLookup};
        let EntityLookup::Found {
            location,
            value: Target::DragonPart(_),
        } = self.server_resident_entity(id)
        else {
            return None;
        };
        if location.owner.dimension != self.dim.key() {
            return None;
        }
        let Source::DragonPart(offset) = location.source else {
            return None;
        };
        Some((self.dragon.as_ref()?, usize::from(offset)))
    }

    pub(super) fn assign_resident_crystal_ids(&mut self) {
        crate::game::entity_residency::assign_resident_ids!(
            self.crystals,
            c,
            &mut self.next_entity,
            None
        );
    }
}

impl Game<'_> {
    /// The box of the dragon part or end crystal guests know as entity `id`
    /// (`EnderDragonPart`s follow the dragon's id).
    pub(in crate::game) fn tracked_special_box(&self, id: i32) -> Option<Aabb> {
        let id = u32::try_from(id).ok().filter(|&id| id != 0)?;
        if let Some(d) = self.dragon.as_ref().filter(|d| d.alive() && d.id != 0)
            && (d.id + 1..=d.id + 8).contains(&id)
        {
            return Some(d.part_box((id - d.id - 1) as usize));
        }
        let i = self.crystals.iter().position(|c| c.id == id)?;
        self.special_targets()
            .into_iter()
            .find(|(s, _)| *s == CRYSTAL_IDS + i as u32)
            .map(|(_, b)| b)
    }
}
