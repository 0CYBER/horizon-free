//! `LivingEntity.collectEquipmentChanges` for `/attribute`'s reads: a mob's
//! attribute map with its gear's modifiers in it.
use horizon_client::mobs::finalize;
use horizon_world::attributes::entity::{AttributeId, EntityAttributes};
use horizon_world::attributes::{Modifier, Operation};

/// What `/attribute` reads for a mob wearing `gear`: `map` with each gear
/// modifier put in as `collectEquipmentChanges` does (`removeModifier(id)`,
/// then `addTransientModifier`), so one id is there once. `None` without gear.
pub(super) fn with_gear(
    map: &EntityAttributes,
    key: AttributeId,
    gear: Option<Vec<(String, f64, finalize::Op)>>,
) -> Option<EntityAttributes> {
    let gear = gear.filter(|g| !g.is_empty())?;
    let mut worn = map.clone();
    worn.edit(key, |a, o| {
        for (id, amount, op) in gear {
            let operation = match op {
                finalize::Op::AddValue => Operation::AddValue,
                finalize::Op::AddMultipliedBase => Operation::AddMultipliedBase,
                finalize::Op::AddMultipliedTotal => Operation::AddMultipliedTotal,
            };
            let _ = a.remove(&id, o);
            if let Ok(m) = Modifier::new(&id, amount, operation) {
                let _ = a.add_transient(m, o);
            }
        }
    });
    Some(worn)
}
