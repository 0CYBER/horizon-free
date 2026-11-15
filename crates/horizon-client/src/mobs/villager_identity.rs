//! Borrowed profession names and command-name construction.
use crate::mobs::{Mob, Species, villager};
use std::sync::Arc;

/// The vanilla render name used by the existing numeric profession adapter.
pub fn profession_id(index: u8) -> &'static str {
    villager::profession(index).id.as_str()
}

pub fn command_name(mob: &Mob) -> crate::text::Text {
    if let Some(name) = mob.display_custom_name() {
        return name;
    }
    // Villager.getTypeName returns its registered profession's name.
    if mob.species() == Species::Villager {
        let profession = reported_profession(mob);
        return crate::text::Text::translate(
            &format!("entity.minecraft.villager.{profession}"),
            &[],
        );
    }
    let id = crate::command::args::normalize(&mob.kind.id);
    crate::text::Text::translate(&format!("entity.{}", id.replace(':', ".")), &[])
}

pub fn seen_profession_name(mob: &Mob, is_villager: bool) -> Option<Arc<str>> {
    match &mob.extra.profession {
        villager::ProfessionState::Registered { id, .. } if is_villager => Some(id.clone()),
        _ => None,
    }
}

/// Profession identity for command and agent reports, preserving vanilla bare names.
pub fn reported_profession(mob: &Mob) -> String {
    mob.extra.profession.index().map_or_else(
        || mob.extra.profession.id(),
        |index| profession_id(index).to_owned(),
    )
}
