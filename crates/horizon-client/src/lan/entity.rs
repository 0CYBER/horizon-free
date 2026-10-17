//! Entity tracking (ChunkMap.TrackedEntity / ServerEntity): the game describes
//! every entity it simulates as an `EntityState` each tick; a `Tracker`
//! remembers what one client has been told and produces the spawn, movement,
//! motion, metadata, equipment and removal packets that bring it up to date,
//! at the entity type's update interval (`tracking`).
use super::tracking;
use horizon_protocol::{
    buf::pack_degrees,
    entity_data::Value,
    item::Stack,
    packets::play::{
        self, AddEntity, EntityPositionSync, MoveEntity, MoveEntityPos, MoveEntityPosRot,
        MoveEntityRot, PositionPath, RotateHead, SetEntityData, SetEntityMotion, SetEquipment,
        SetPassengers, VecDelta,
    },
};

/// What the game says about one entity this tick.
#[derive(Clone, Debug)]
pub struct EntityState {
    pub id: i32,
    pub uuid: u128,
    /// Registry id of the entity type (`minecraft:zombie`).
    pub kind: String,
    pub position: [f64; 3],
    pub yaw: f32,
    pub pitch: f32,
    pub head_yaw: f32,
    pub on_ground: bool,
    pub velocity: [f64; 3],
    /// ClientboundAddEntityPacket.data (item frame direction, block state ...).
    pub data: i32,
    /// SynchedEntityData entries that differ from the defaults.
    pub meta: Vec<(u8, i32, Value)>,
    /// EquipmentSlot ordinals (0 main hand, 1 off hand, 2 feet, 3 legs,
    /// 4 chest, 5 head, 6 body, 7 saddle) and their stacks.
    pub equipment: Vec<(u8, Option<Stack>)>,
    /// `Entity.getRequiresPrecisePosition` (primed TNT and falling blocks
    /// touching a block): moves are sent as full position syncs.
    pub precise: bool,
}

/// `AbstractArrow`: every update sends its position and rotation.
fn is_arrow(kind: &str) -> bool {
    matches!(
        kind,
        "minecraft:arrow" | "minecraft:spectral_arrow" | "minecraft:trident"
    )
}

/// `AbstractHurtingProjectile.accelerationPower` (0.1, none for wind
/// charges), sent with each velocity update.
fn acceleration_power(kind: &str) -> Option<f64> {
    match kind {
        "minecraft:fireball"
        | "minecraft:small_fireball"
        | "minecraft:dragon_fireball"
        | "minecraft:wither_skull" => Some(0.1),
        "minecraft:wind_charge" | "minecraft:breeze_wind_charge" => Some(0.0),
        _ => None,
    }
}

/// What a client was last told about one entity (ServerEntity's fields).
struct Sent {
    /// VecDeltaCodec's base: the last position sent.
    base: [f64; 3],
    /// lastSentYRot / lastSentXRot / lastSentYHeadRot.
    rotation: (i8, i8),
    head: i8,
    /// lastSentMovement.
    velocity: [f64; 3],
    meta: Vec<(u8, i32, Value)>,
    equipment: Vec<(u8, Option<Stack>)>,
    was_on_ground: bool,
    tick: u32,
    teleport_delay: u32,
    /// The type's update interval (0: never) and whether velocity is tracked.
    interval: u32,
    track_deltas: bool,
    /// lastPassengers, and wasRiding: a passenger's position isn't synced,
    /// and the first update after it dismounts is a full position sync.
    passengers: Vec<i32>,
    was_riding: bool,
}

/// Who rides what this tick (`Entity.getPassengers`, `isPassenger`).
#[derive(Default)]
pub struct Riding {
    /// Each vehicle's passengers, in order.
    pub passengers: crate::fast_hash::FxHashMap<i32, Vec<i32>>,
    /// Each passenger's vehicle.
    pub vehicle: crate::fast_hash::FxHashMap<i32, i32>,
}

impl Riding {
    pub fn clear(&mut self) {
        self.passengers.clear();
        self.vehicle.clear();
    }
    /// `passenger` rides `vehicle`, after the ones already riding it.
    pub fn add(&mut self, vehicle: i32, passenger: i32) {
        self.passengers.entry(vehicle).or_default().push(passenger);
        self.vehicle.insert(passenger, vehicle);
    }
    pub fn passengers(&self, id: i32) -> &[i32] {
        self.passengers.get(&id).map_or(&[], Vec::as_slice)
    }
    fn set_passengers(&self, vehicle: i32) -> play::Clientbound {
        play::Clientbound::SetPassengers(SetPassengers {
            vehicle,
            passengers: self.passengers(vehicle).to_vec(),
        })
    }
}

#[derive(Default)]
pub struct Tracker {
    sent: crate::fast_hash::FxHashMap<i32, Sent>,
}

/// VecDeltaCodec.encode.
fn encode(v: f64) -> i64 {
    (v * 4096.0).round() as i64
}

/// The type's tracking range in chunks: a client sees an entity of `kind`
/// within it, capped by the view distance, horizontally
/// (TrackedEntity.updatePlayer).
pub fn tracking_chunks(kind: &str) -> i32 {
    i32::from(tracking::settings(kind).0)
}

impl Tracker {
    pub fn is_tracking(&self, id: i32) -> bool {
        self.sent.contains_key(&id)
    }
    pub fn ids(&self) -> Vec<i32> {
        self.sent.keys().copied().collect()
    }
    pub fn forget(&mut self, id: i32) {
        self.sent.remove(&id);
    }
    pub fn clear(&mut self) {
        self.sent.clear();
    }
    pub fn len(&self) -> usize {
        self.sent.len()
    }
    pub fn is_empty(&self) -> bool {
        self.sent.is_empty()
    }

    /// The packets that introduce an entity (ServerEntity.sendPairingData).
    pub fn spawn(
        &mut self,
        e: &EntityState,
        entity_type: i32,
        riding: &Riding,
    ) -> Vec<play::Clientbound> {
        let rotation = (pack_degrees(e.pitch), pack_degrees(e.yaw));
        let head = pack_degrees(e.head_yaw);
        let (_, interval, track_deltas) = tracking::settings(&e.kind);
        let mut out = vec![play::Clientbound::AddEntity(AddEntity {
            id: e.id,
            uuid: e.uuid,
            kind: entity_type,
            position: e.position,
            velocity: e.velocity,
            pitch: rotation.0,
            yaw: rotation.1,
            head_yaw: head,
            data: e.data,
        })];
        if !e.meta.is_empty() {
            out.push(play::Clientbound::SetEntityData(SetEntityData {
                id: e.id,
                values: e.meta.clone(),
            }));
        }
        if !e.equipment.is_empty() {
            out.push(play::Clientbound::SetEquipment(SetEquipment {
                id: e.id,
                slots: e.equipment.clone(),
            }));
        }
        let passengers = riding.passengers(e.id).to_vec();
        if !passengers.is_empty() {
            out.push(riding.set_passengers(e.id));
        }
        if let Some(&vehicle) = riding.vehicle.get(&e.id) {
            out.push(riding.set_passengers(vehicle));
        }
        self.sent.insert(
            e.id,
            Sent {
                base: e.position,
                rotation,
                head,
                velocity: e.velocity,
                meta: e.meta.clone(),
                equipment: e.equipment.clone(),
                was_on_ground: e.on_ground,
                tick: 0,
                teleport_delay: 0,
                interval,
                track_deltas,
                passengers,
                was_riding: false,
            },
        );
        out
    }

    /// Changes since the last update (ServerEntity.sendChanges): movement and
    /// rotation every update interval (positions at least every 60 ticks, a
    /// full position sync every 400 or when the entity lands or leaves the
    /// ground), velocity for types that track it, changed data at once.
    pub fn update(&mut self, e: &EntityState, riding: &Riding) -> Vec<play::Clientbound> {
        let Some(s) = self.sent.get_mut(&e.id) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        let passengers = riding.passengers(e.id);
        if passengers != s.passengers.as_slice() {
            out.push(riding.set_passengers(e.id));
            s.passengers = passengers.to_vec();
        }
        let dirty = e.meta != s.meta;
        let due = s.interval != 0 && s.tick.is_multiple_of(s.interval);
        if due || dirty {
            let rotation = (pack_degrees(e.pitch), pack_degrees(e.yaw));
            let turned = |a: i8, b: i8| (i32::from(a) - i32::from(b)).abs() >= 1;
            let arrow = is_arrow(&e.kind);
            let rotate =
                arrow || turned(rotation.1, s.rotation.1) || turned(rotation.0, s.rotation.0);
            if riding.vehicle.contains_key(&e.id) {
                // A passenger: its rotation alone (its vehicle places it).
                if rotate {
                    out.push(play::Clientbound::MoveEntityRot(MoveEntityRot(
                        MoveEntity {
                            id: e.id,
                            delta: None,
                            rotation: Some((rotation.1, rotation.0)),
                            on_ground: e.on_ground,
                        },
                    )));
                    s.rotation = rotation;
                }
                s.base = e.position;
                s.was_riding = true;
                Self::send_dirty(e, s, dirty, &mut out);
                Self::send_head(e, s, &mut out);
                s.tick += 1;
                Self::send_equipment(e, s, &mut out);
                return out;
            }
            s.teleport_delay += 1;
            let d: f64 = (0..3).map(|k| (e.position[k] - s.base[k]).powi(2)).sum();
            let send_position = arrow || d >= 7.629_394_5e-6 || s.tick.is_multiple_of(60);
            let sync = EntityPositionSync {
                id: e.id,
                path: PositionPath::Linear(e.position),
                yaw: e.yaw,
                pitch: e.pitch,
                on_ground: e.on_ground,
            };
            // ServerEntity.createMovePacket.
            let packet = if e.precise
                || s.teleport_delay > 400
                || s.was_riding
                || s.was_on_ground != e.on_ground
            {
                s.was_on_ground = e.on_ground;
                s.teleport_delay = 0;
                s.was_riding = false;
                Some(play::Clientbound::EntityPositionSync(sync))
            } else if send_position {
                let delta = [0, 1, 2].map(|k| encode(e.position[k]) - encode(s.base[k]));
                if delta.iter().any(|v| i16::try_from(*v).is_err()) {
                    Some(play::Clientbound::EntityPositionSync(sync))
                } else {
                    let m = MoveEntity {
                        id: e.id,
                        delta: Some(VecDelta::Linear(delta.map(|v| v as i16))),
                        rotation: rotate.then_some((rotation.1, rotation.0)),
                        on_ground: e.on_ground,
                    };
                    Some(if rotate {
                        play::Clientbound::MoveEntityPosRot(MoveEntityPosRot(m))
                    } else {
                        play::Clientbound::MoveEntityPos(MoveEntityPos(m))
                    })
                }
            } else if rotate {
                Some(play::Clientbound::MoveEntityRot(MoveEntityRot(
                    MoveEntity {
                        id: e.id,
                        delta: None,
                        rotation: Some((rotation.1, rotation.0)),
                        on_ground: e.on_ground,
                    },
                )))
            } else {
                None
            };
            if s.track_deltas {
                let diff: f64 = (0..3)
                    .map(|k| (e.velocity[k] - s.velocity[k]).powi(2))
                    .sum();
                let still = e.velocity.iter().all(|v| *v == 0.0);
                if diff > 1.0e-7 || diff > 0.0 && still {
                    s.velocity = e.velocity;
                    let motion = play::Clientbound::SetEntityMotion(SetEntityMotion {
                        id: e.id,
                        velocity: e.velocity,
                    });
                    match acceleration_power(&e.kind) {
                        // A bundle: the motion and the projectile's power.
                        Some(power) => out.extend([
                            play::Clientbound::BundleDelimiter(horizon_protocol::packets::Empty),
                            motion,
                            play::Clientbound::ProjectilePower(play::ProjectilePower {
                                id: e.id,
                                power,
                            }),
                            play::Clientbound::BundleDelimiter(horizon_protocol::packets::Empty),
                        ]),
                        None => out.push(motion),
                    }
                }
            }
            if let Some(packet) = packet {
                let (position, rotated) = match &packet {
                    play::Clientbound::EntityPositionSync(_)
                    | play::Clientbound::MoveEntityPosRot(_) => (true, true),
                    play::Clientbound::MoveEntityPos(_) => (true, false),
                    _ => (false, true),
                };
                out.push(packet);
                if position {
                    s.base = e.position;
                }
                if rotated {
                    s.rotation = rotation;
                }
            }
            Self::send_dirty(e, s, dirty, &mut out);
            Self::send_head(e, s, &mut out);
        }
        s.tick += 1;
        Self::send_equipment(e, s, &mut out);
        out
    }

    /// ServerEntity.sendDirtyEntityData.
    fn send_dirty(e: &EntityState, s: &mut Sent, dirty: bool, out: &mut Vec<play::Clientbound>) {
        if !dirty {
            return;
        }
        let changed: Vec<_> = e
            .meta
            .iter()
            .filter(|m| !s.meta.contains(m))
            .cloned()
            .collect();
        if !changed.is_empty() {
            out.push(play::Clientbound::SetEntityData(SetEntityData {
                id: e.id,
                values: changed,
            }));
        }
        s.meta = e.meta.clone();
    }

    fn send_head(e: &EntityState, s: &mut Sent, out: &mut Vec<play::Clientbound>) {
        let head = pack_degrees(e.head_yaw);
        if (i32::from(head) - i32::from(s.head)).abs() >= 1 {
            out.push(play::Clientbound::RotateHead(RotateHead {
                id: e.id,
                head_yaw: head,
            }));
            s.head = head;
        }
    }

    /// LivingEntity.detectEquipmentUpdates: changed slots every tick.
    fn send_equipment(e: &EntityState, s: &mut Sent, out: &mut Vec<play::Clientbound>) {
        if e.equipment != s.equipment {
            // Cleared slots are sent as empty stacks.
            let mut slots: Vec<(u8, Option<Stack>)> = e
                .equipment
                .iter()
                .filter(|m| !s.equipment.contains(m))
                .cloned()
                .collect();
            for (slot, _) in &s.equipment {
                if !e.equipment.iter().any(|(k, _)| k == slot) {
                    slots.push((*slot, None));
                }
            }
            if !slots.is_empty() {
                out.push(play::Clientbound::SetEquipment(SetEquipment {
                    id: e.id,
                    slots,
                }));
            }
            s.equipment = e.equipment.clone();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(x: f64) -> EntityState {
        EntityState {
            id: 7,
            uuid: 1,
            kind: "minecraft:zombie".into(),
            position: [x, 64.0, 0.0],
            yaw: 0.0,
            pitch: 0.0,
            head_yaw: 0.0,
            on_ground: true,
            velocity: [0.0; 3],
            data: 0,
            meta: Vec::new(),
            equipment: Vec::new(),
            precise: false,
        }
    }

    #[test]
    fn moves_go_out_every_update_interval() {
        let mut t = Tracker::default();
        let spawn = t.spawn(&state(0.0), 100, &Riding::default());
        assert!(matches!(spawn[0], play::Clientbound::AddEntity(_)));
        // Tick 0 is due and a multiple of 60: the position goes out unchanged.
        assert!(matches!(
            t.update(&state(0.0), &Riding::default())[..],
            [play::Clientbound::MoveEntityPos(_)]
        ));
        // A zombie updates every third tick.
        assert!(t.update(&state(0.5), &Riding::default()).is_empty());
        assert!(t.update(&state(0.5), &Riding::default()).is_empty());
        assert!(matches!(
            t.update(&state(0.5), &Riding::default())[..],
            [play::Clientbound::MoveEntityPos(_)]
        ));
        for _ in 0..2 {
            t.update(&state(40.0), &Riding::default());
        }
        let jump = t.update(&state(40.0), &Riding::default());
        assert!(matches!(jump[0], play::Clientbound::EntityPositionSync(_)));
    }

    #[test]
    fn landing_sends_a_position_sync() {
        let mut t = Tracker::default();
        let mut e = state(0.0);
        t.spawn(&e, 100, &Riding::default());
        t.update(&e, &Riding::default());
        e.on_ground = false;
        t.update(&e, &Riding::default());
        t.update(&e, &Riding::default());
        assert!(matches!(
            t.update(&e, &Riding::default())[..],
            [play::Clientbound::EntityPositionSync(_)]
        ));
    }

    #[test]
    fn metadata_and_equipment_send_only_the_changes() {
        let mut t = Tracker::default();
        let mut e = state(0.0);
        t.spawn(&e, 100, &Riding::default());
        t.update(&e, &Riding::default());
        e.meta.push((0, 0, Value::Byte(1)));
        e.equipment.push((0, Some(Stack::new(5, 1))));
        let out = t.update(&e, &Riding::default());
        assert!(
            out.iter()
                .any(|p| matches!(p, play::Clientbound::SetEntityData(_)))
        );
        assert!(
            out.iter()
                .any(|p| matches!(p, play::Clientbound::SetEquipment(_)))
        );
        assert!(t.update(&e, &Riding::default()).is_empty());
        e.equipment.clear();
        let out = t.update(&e, &Riding::default());
        let Some(play::Clientbound::SetEquipment(eq)) = out.first() else {
            panic!("expected equipment");
        };
        assert_eq!(eq.slots, vec![(0, None)]);
    }
}
