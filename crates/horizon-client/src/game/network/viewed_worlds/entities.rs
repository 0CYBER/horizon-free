//! The entities of a viewed level (arismoko/horizon#3716, 9d): what its wrapped entity packets leave on the client,
//! as `ClientPacketListener` handles them with that level current. They live in their viewed
//! world only: nothing reaches the player's own entities, the renderer, sounds or a mod's hooks
//! (drawing a viewed level is gap 13).
//!
//! A server cannot fill the client's memory through them: a world keeps at most
//! `viewed_levels::MAX_ENTITIES` entities, each with at most [`MAX_TABLE`] effects and as many
//! attributes, its entity data keyed by an index byte and its equipment by a slot byte. Past a
//! cap the connection closes with a fixed reason.
use super::routing::RouteError;
use horizon_client::fast_hash::FxHashMap;
use horizon_protocol::{
    buf::unpack_degrees,
    entity_data::Value,
    item::Stack,
    packets::play::{self, AttributeSnapshot, Clientbound as C, relative},
    viewed_levels::MAX_ENTITIES,
};

/// The most effects one entity keeps, and the most attributes.
pub(in crate::game) const MAX_TABLE: usize = 256;

/// One entity of a viewed level, as its packets left it. Nothing reads it but the tests until a
/// viewed level is drawn (gap 13).
#[cfg_attr(not(test), allow(dead_code))]
pub(in crate::game) struct ViewedEntity {
    pub(in crate::game) uuid: u128,
    /// Its type's id in the `minecraft:entity_type` registry.
    pub(in crate::game) kind: i32,
    /// `AddEntity`'s data field (a falling block's state, a projectile's owner...).
    pub(in crate::game) data: i32,
    pub(in crate::game) position: [f64; 3],
    /// Where the next relative move counts from (`VecDeltaCodec.base`).
    base: [f64; 3],
    /// Yaw, pitch and head yaw, in degrees.
    pub(in crate::game) rotation: (f32, f32, f32),
    pub(in crate::game) on_ground: bool,
    pub(in crate::game) velocity: [f64; 3],
    /// Its synched data, by index: serializer and value.
    pub(in crate::game) values: FxHashMap<u8, (i32, Value)>,
    pub(in crate::game) equipment: FxHashMap<u8, Option<Stack>>,
    pub(in crate::game) attributes: Vec<AttributeSnapshot>,
    /// Effect id, amplifier, duration and flags.
    pub(in crate::game) effects: Vec<(i32, i32, i32, i8)>,
    pub(in crate::game) vehicle: Option<i32>,
    pub(in crate::game) passengers: Vec<i32>,
    /// The entity holding its lead.
    pub(in crate::game) leash: Option<i32>,
}

/// A viewed world's entities, by id.
#[derive(Default)]
pub(in crate::game) struct ViewedEntities {
    entities: FxHashMap<i32, ViewedEntity>,
}

impl ViewedEntities {
    /// Entity `id`, if the world holds it: where a sound bound to it is heard (`sounds`).
    pub(in crate::game) fn get(&self, id: i32) -> Option<&ViewedEntity> {
        self.entities.get(&id)
    }
    #[cfg(test)]
    pub(in crate::game) fn len(&self) -> usize {
        self.entities.len()
    }

    /// An entity packet of the level; other kinds change nothing. A packet about an entity the
    /// world does not hold is dropped, as the vanilla client drops it.
    pub(in crate::game) fn apply(&mut self, packet: C) -> Result<(), RouteError> {
        match packet {
            // `handleAddEntity`: an entity of an id held already replaces it.
            C::AddEntity(p) => {
                if self.entities.contains_key(&p.id) {
                    self.remove(p.id);
                } else if self.entities.len() >= MAX_ENTITIES {
                    return Err(RouteError::TooManyEntities);
                }
                let rotation = (
                    unpack_degrees(p.yaw),
                    unpack_degrees(p.pitch),
                    unpack_degrees(p.head_yaw),
                );
                self.entities.insert(
                    p.id,
                    ViewedEntity {
                        uuid: p.uuid,
                        kind: p.kind,
                        data: p.data,
                        position: p.position,
                        base: p.position,
                        rotation,
                        on_ground: false,
                        velocity: p.velocity,
                        values: FxHashMap::default(),
                        equipment: FxHashMap::default(),
                        attributes: Vec::new(),
                        effects: Vec::new(),
                        vehicle: None,
                        passengers: Vec::new(),
                        leash: None,
                    },
                );
            }
            C::RemoveEntities(p) => {
                for id in p.ids {
                    self.remove(id);
                }
            }
            // `handleMoveEntity`: the delta counts from the position codec's base, which then
            // moves to its end.
            C::MoveEntityPos(play::MoveEntityPos(m))
            | C::MoveEntityPosRot(play::MoveEntityPosRot(m))
            | C::MoveEntityRot(play::MoveEntityRot(m)) => {
                if let Some(e) = self.entities.get_mut(&m.id) {
                    if let Some(delta) = &m.delta {
                        e.base = delta.end(e.base);
                        e.position = e.base;
                    }
                    if let Some((yaw, pitch)) = m.rotation {
                        e.rotation.0 = unpack_degrees(yaw);
                        e.rotation.1 = unpack_degrees(pitch);
                    }
                    e.on_ground = m.on_ground;
                }
            }
            C::RotateHead(p) => {
                if let Some(e) = self.entities.get_mut(&p.id) {
                    e.rotation.2 = unpack_degrees(p.head_yaw);
                }
            }
            // `handleEntityPositionSync`: the position codec's base is set.
            C::EntityPositionSync(p) => {
                if let Some(e) = self.entities.get_mut(&p.id) {
                    e.base = p.path.end();
                    e.position = e.base;
                    (e.rotation.0, e.rotation.1) = (p.yaw, p.pitch);
                    e.on_ground = p.on_ground;
                }
            }
            // `handleTeleportEntity`: `PositionMoveRotation.calculateAbsolute` over where it is.
            C::TeleportEntity(p) => {
                if let Some(e) = self.entities.get_mut(&p.id) {
                    let rel = |bit: i32, base: f64, v: f64| {
                        if p.relatives & bit != 0 { base + v } else { v }
                    };
                    let axes = [relative::X, relative::Y, relative::Z];
                    let deltas = [relative::DELTA_X, relative::DELTA_Y, relative::DELTA_Z];
                    for i in 0..3 {
                        e.position[i] = rel(axes[i], e.position[i], p.position[i]);
                        e.velocity[i] = rel(deltas[i], e.velocity[i], p.delta[i]);
                    }
                    e.rotation.0 = rel(relative::Y_ROT, e.rotation.0.into(), p.yaw.into()) as f32;
                    e.rotation.1 = rel(relative::X_ROT, e.rotation.1.into(), p.pitch.into()) as f32;
                    e.on_ground = p.on_ground;
                }
            }
            C::SetEntityData(p) => {
                if let Some(e) = self.entities.get_mut(&p.id) {
                    for (index, serializer, value) in p.values {
                        e.values.insert(index, (serializer, value));
                    }
                }
            }
            C::SetEquipment(p) => {
                if let Some(e) = self.entities.get_mut(&p.id) {
                    e.equipment.extend(p.slots);
                }
            }
            // `handleUpdateAttributes`: each instance named takes the packet's base and
            // modifiers.
            C::UpdateAttributes(p) => {
                if let Some(e) = self.entities.get_mut(&p.id) {
                    for a in p.values {
                        keep(&mut e.attributes, a, |b| b.attribute)?;
                    }
                }
            }
            C::SetEntityMotion(p) => {
                if let Some(e) = self.entities.get_mut(&p.id) {
                    e.velocity = p.velocity;
                }
            }
            // `handleSetEntityPassengersPacket`: the vehicle's riders get off, then each one the
            // world holds starts riding it, leaving any vehicle it was on.
            C::SetPassengers(p) => {
                if !self.entities.contains_key(&p.vehicle) {
                    return Ok(());
                }
                self.eject(p.vehicle);
                for id in p.passengers {
                    if id != p.vehicle && self.entities.contains_key(&id) {
                        self.dismount(id);
                        if let Some(e) = self.entities.get_mut(&id) {
                            e.vehicle = Some(p.vehicle);
                        }
                        if let Some(v) = self.entities.get_mut(&p.vehicle) {
                            v.passengers.push(id);
                        }
                    }
                }
            }
            // `handleEntityLinkPacket`: who holds the lead; 0 is nobody.
            C::SetEntityLink(p) => {
                if let Some(e) = self.entities.get_mut(&p.source) {
                    e.leash = (p.dest != 0).then_some(p.dest);
                }
            }
            // `handleUpdateMobEffect`: the effect replaces any of its kind.
            C::UpdateMobEffect(p) => {
                if let Some(e) = self.entities.get_mut(&p.id) {
                    let effect = (p.effect, p.amplifier, p.duration, p.flags);
                    keep(&mut e.effects, effect, |e| e.0)?;
                }
            }
            C::RemoveMobEffect(p) => {
                if let Some(e) = self.entities.get_mut(&p.id) {
                    e.effects.retain(|e| e.0 != p.effect);
                }
            }
            // Entity events and animations show something for a moment: drawing is gap 13.
            _ => {}
        }
        Ok(())
    }

    /// `ClientLevel.removeEntity`: it gets off its vehicle and its riders get off it.
    fn remove(&mut self, id: i32) {
        self.dismount(id);
        self.eject(id);
        self.entities.remove(&id);
    }

    /// `Entity.stopRiding`.
    fn dismount(&mut self, id: i32) {
        let Some(vehicle) = self.entities.get_mut(&id).and_then(|e| e.vehicle.take()) else {
            return;
        };
        if let Some(v) = self.entities.get_mut(&vehicle) {
            v.passengers.retain(|&p| p != id);
        }
    }

    /// `Entity.ejectPassengers`.
    fn eject(&mut self, id: i32) {
        let passengers = (self.entities.get_mut(&id))
            .map(|v| std::mem::take(&mut v.passengers))
            .unwrap_or_default();
        for p in passengers {
            if let Some(e) = self.entities.get_mut(&p) {
                e.vehicle = None;
            }
        }
    }
}

/// Puts `value` in place of the one of its key in `table`, or adds it while the table holds fewer
/// than [`MAX_TABLE`].
fn keep<T>(table: &mut Vec<T>, value: T, key: impl Fn(&T) -> i32) -> Result<(), RouteError> {
    if let Some(t) = table.iter_mut().find(|t| key(t) == key(&value)) {
        *t = value;
    } else if table.len() >= MAX_TABLE {
        return Err(RouteError::EntityTableFull);
    } else {
        table.push(value);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn add(id: i32, x: f64) -> C {
        C::AddEntity(play::AddEntity {
            id,
            uuid: id as u128,
            kind: 7,
            position: [x, 64.0, 0.0],
            velocity: [0.0; 3],
            pitch: 0,
            yaw: 64,
            head_yaw: 0,
            data: 0,
        })
    }

    /// A viewed world holds at most 8192 entities: one more is refused, one added again in
    /// place of itself is not, and one removed frees its place.
    #[test]
    fn a_viewed_world_holds_at_most_8192_entities() {
        let mut entities = ViewedEntities::default();
        for id in 0..8192 {
            entities.apply(add(id, 0.0)).unwrap();
        }
        assert_eq!(entities.len(), 8192);
        assert_eq!(entities.apply(add(17, 3.0)), Ok(()));
        assert_eq!(entities.get(17).unwrap().position, [3.0, 64.0, 0.0]);
        assert_eq!(
            entities.apply(add(8192, 0.0)),
            Err(RouteError::TooManyEntities)
        );
        assert_eq!(
            RouteError::TooManyEntities.to_string(),
            "horizon:levels: a viewed level past 8192 entities"
        );
        assert!(entities.get(8192).is_none());
        let gone = C::RemoveEntities(play::RemoveEntities { ids: vec![5] });
        entities.apply(gone).unwrap();
        assert_eq!(entities.apply(add(8192, 0.0)), Ok(()));
        assert_eq!(entities.len(), 8192);
    }

    /// An entity's effects stop at 256 kinds: a new kind past them is refused, the same kind
    /// again replaces its own.
    #[test]
    fn an_entity_keeps_at_most_256_effects() {
        let mut entities = ViewedEntities::default();
        entities.apply(add(1, 0.0)).unwrap();
        let effect = |effect, amplifier| {
            C::UpdateMobEffect(play::UpdateMobEffect {
                id: 1,
                effect,
                amplifier,
                duration: 200,
                flags: 0,
            })
        };
        for kind in 0..256 {
            entities.apply(effect(kind, 0)).unwrap();
        }
        assert_eq!(entities.apply(effect(9, 3)), Ok(()));
        assert_eq!(entities.get(1).unwrap().effects[9], (9, 3, 200, 0));
        assert_eq!(
            entities.apply(effect(256, 0)),
            Err(RouteError::EntityTableFull)
        );
        assert_eq!(entities.get(1).unwrap().effects.len(), 256);
    }

    /// Passengers ride what the last `SetPassengers` named, leave their old vehicle, and get off
    /// when it is removed.
    #[test]
    fn riders_follow_set_passengers_and_get_off_a_removed_vehicle() {
        let mut entities = ViewedEntities::default();
        for id in 1..=3 {
            entities.apply(add(id, 0.0)).unwrap();
        }
        let ride = |vehicle, passengers: &[i32]| {
            C::SetPassengers(play::SetPassengers {
                vehicle,
                passengers: passengers.to_vec(),
            })
        };
        entities.apply(ride(1, &[2, 3, 99])).unwrap();
        assert_eq!(entities.get(1).unwrap().passengers, vec![2, 3]);
        entities.apply(ride(2, &[3])).unwrap();
        assert_eq!(entities.get(1).unwrap().passengers, vec![2]);
        assert_eq!(entities.get(3).unwrap().vehicle, Some(2));
        let gone = C::RemoveEntities(play::RemoveEntities { ids: vec![2] });
        entities.apply(gone).unwrap();
        assert!(entities.get(1).unwrap().passengers.is_empty());
        assert_eq!(entities.get(3).unwrap().vehicle, None);
    }
}
