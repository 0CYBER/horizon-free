//! `ClientboundUpdateAttributesPacket`: the attribute instances a server syncs
//! to its clients. `ServerEntity.sendPairingData` sends every client-syncable
//! instance an entity has (`AttributeMap.getSyncableAttributes`: the instances
//! created so far, not the type's defaults), `sendDirtyEntityData` the ones
//! changed since (`getAttributesToSync`), to the trackers and the player itself.
use horizon_protocol::packets::play::{AttributeModifier, AttributeSnapshot};
use horizon_world::attributes::{
    Instance, Modifier, Operation,
    entity::{AttributeId, EntityAttributes},
    live::LiveAttributes,
};

/// `AttributeModifier.Operation.id`.
pub fn operation_id(op: Operation) -> i32 {
    match op {
        Operation::AddValue => 0,
        Operation::AddMultipliedBase => 1,
        Operation::AddMultipliedTotal => 2,
    }
}

/// An `Operation`'s serialized name (`add_value` ...) as its id.
pub fn operation_named(name: &str) -> i32 {
    match name {
        "add_multiplied_base" => 1,
        "add_multiplied_total" => 2,
        _ => 0,
    }
}

/// `AttributeSnapshot`: the base and every modifier, in the order they were added.
pub fn snapshot(attribute: AttributeId, instance: &Instance) -> AttributeSnapshot {
    AttributeSnapshot {
        attribute: attribute.registry_id(),
        base: instance.base_value(),
        modifiers: instance
            .modifiers()
            .iter()
            .map(|m| AttributeModifier {
                id: m.id().to_owned(),
                amount: m.amount(),
                operation: operation_id(m.operation()),
            })
            .collect(),
    }
}

/// `AttributeMap.getSyncableAttributes` of a live map (a player's).
pub fn syncable<G: FnMut() -> i32>(map: &LiveAttributes<G>) -> Vec<AttributeSnapshot> {
    let catalog = map.catalog();
    map.instances()
        .filter(|(_, key, _)| catalog.attribute(key.attribute()).syncable)
        .map(|(_, key, instance)| snapshot(key.attribute(), instance))
        .collect()
}

/// `getAttributesToSync`, cleared once read (`ServerEntity.sendDirtyEntityData`).
pub fn take_dirty<G: FnMut() -> i32>(map: &mut LiveAttributes<G>) -> Vec<AttributeSnapshot> {
    if map.sync().is_empty() {
        return Vec::new();
    }
    let handles: Vec<_> = map.sync().values().flatten().cloned().collect();
    map.sync_mut().clear();
    handles
        .iter()
        .filter_map(|h| {
            let key = map.attribute(h).ok()?;
            Some(snapshot(key.attribute(), map.instance(h).ok()?))
        })
        .collect()
}

/// `ClientPacketListener.handleUpdateAttributes`: each instance the server
/// synced takes its base and exactly its modifiers (`removeModifiers`, then
/// `addTransientModifier` for each). An attribute the map doesn't know is
/// skipped, as vanilla warns and skips it.
pub fn apply<G: FnMut() -> i32>(map: &mut LiveAttributes<G>, values: &[AttributeSnapshot]) {
    for s in values {
        let Some(key) = map
            .keys()
            .iter()
            .find(|k| k.attribute().registry_id() == s.attribute)
            .cloned()
        else {
            continue;
        };
        let Ok(Some(handle)) = map.get_instance(Some(key)) else {
            continue;
        };
        let modifiers: Vec<_> = s
            .modifiers
            .iter()
            .filter_map(|m| Modifier::new(&m.id, m.amount, operation(m.operation)).ok())
            .collect();
        let _ = map.edit(&handle, |instance, observer| {
            instance.set_base_value(s.base, observer)?;
            instance.remove_all(observer)?;
            for m in modifiers {
                instance.add_transient(m, observer)?;
            }
            Ok(())
        });
    }
}

/// `AttributeModifier.Operation.BY_ID`.
fn operation(id: i32) -> Operation {
    match id {
        1 => Operation::AddMultipliedBase,
        2 => Operation::AddMultipliedTotal,
        _ => Operation::AddValue,
    }
}

/// The client-syncable instances of a generic attribute map.
pub fn generic_syncable(map: &EntityAttributes) -> impl Iterator<Item = AttributeSnapshot> + '_ {
    let catalog = map.catalog();
    catalog
        .attributes()
        .filter(|(_, a)| a.syncable)
        .filter_map(move |(id, _)| map.instance(id).map(|i| snapshot(id, i)))
}

/// Adds a modifier to the snapshot of `attribute`, creating it with `base`.
pub fn add_modifier(
    out: &mut Vec<AttributeSnapshot>,
    attribute: i32,
    base: f64,
    modifier: AttributeModifier,
) {
    let snapshot = instance(out, attribute, base);
    match snapshot.modifiers.iter_mut().find(|m| m.id == modifier.id) {
        Some(m) => *m = modifier,
        None => snapshot.modifiers.push(modifier),
    }
}

/// The snapshot of `attribute`, created with `base` if it isn't there yet.
pub fn instance(
    out: &mut Vec<AttributeSnapshot>,
    attribute: i32,
    base: f64,
) -> &mut AttributeSnapshot {
    let i = match out.iter().position(|a| a.attribute == attribute) {
        Some(i) => i,
        None => {
            out.push(AttributeSnapshot {
                attribute,
                base,
                modifiers: Vec::new(),
            });
            out.len() - 1
        }
    };
    &mut out[i]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modifiers_replace_by_id_and_instances_are_created_once() {
        let mut out = Vec::new();
        let m = |id: &str, amount| AttributeModifier {
            id: id.into(),
            amount,
            operation: 0,
        };
        add_modifier(&mut out, 1, 0.0, m("minecraft:armor.helmet", 3.0));
        add_modifier(&mut out, 1, 5.0, m("minecraft:armor.chestplate", 8.0));
        add_modifier(&mut out, 1, 5.0, m("minecraft:armor.helmet", 2.0));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].base, 0.0);
        assert_eq!(
            out[0].modifiers,
            [
                m("minecraft:armor.helmet", 2.0),
                m("minecraft:armor.chestplate", 8.0)
            ]
        );
        instance(&mut out, 26, 0.7);
        assert_eq!(out.len(), 2);
    }
}
