//! Entity Part state (arismoko/horizon#796 stage 3) on the game side: where a handle's entity is now, and
//! which entity types keep Part state, checked once every type is known.
use super::Game;
use horizon_client::join_seq::residency::table::EntityLookup;
use mod_api::{EntityRef, EntityResidency};

impl Game<'_> {
    /// Where the entity `entity` names is now, for its Part state: the world open is the
    /// handle's, then the residency table (the same one `with_resident_slot` resolves through)
    /// finds the entity and its level. A removal, pending or done, an unload and a move under a
    /// new id all leave the id unresolved. On a client, in place for an entity the client has.
    pub(super) fn entity_residency(&self, entity: EntityRef) -> EntityResidency {
        if self.native_lifecycle.epoch() != Some(entity.epoch()) {
            return EntityResidency::Gone;
        }
        if !self.is_logical_server() {
            return match self.native_client_entity_target(entity) {
                Some(_) => EntityResidency::InPlace,
                None => EntityResidency::Gone,
            };
        }
        match self.server_resident_entity(entity.id()) {
            EntityLookup::Found { location, .. } if location.owner.dimension != self.dim.key() => {
                EntityResidency::OtherLevel
            }
            EntityLookup::Found { .. } => EntityResidency::InPlace,
            EntityLookup::Taken => EntityResidency::Busy,
            EntityLookup::Missing => EntityResidency::Gone,
        }
    }
}

/// Refuses each entity Part state declared on an entity type that keeps none: only the mob
/// types (`mob`, vanilla and mod) and the mods' general entity types do. Checked when the
/// mob types load, after every registration, since a type may be declared after its Part;
/// the error reads as the startup's other Part refusals (`part_admission`).
pub(in crate::game) fn refuse_entity_part_state_kinds(
    mob: impl Fn(&str) -> bool,
) -> Result<(), String> {
    let general = mod_api::__private::general_entity_type_declarations();
    let refusals: Vec<String> = mod_api::__private::entity_part_state_declarations()
        .iter()
        .filter_map(|declaration| {
            let reason = declaration.kind_refusal(
                &mob,
                |kind| general.iter().any(|declared| declared.id == kind),
                |kind| mod_api::__private::resolved_entity_type_key(kind).is_some(),
            )?;
            Some(format!(
                "{}: {}",
                declaration.mod_id,
                declaration.refused(reason)
            ))
        })
        .collect();
    if refusals.is_empty() {
        Ok(())
    } else {
        Err(format!("Mod parts refused:\n{}", refusals.join("\n")))
    }
}
