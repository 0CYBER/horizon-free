//! Entity tracking (ServerEntity / TrackedEntity): the game describes every
//! entity it simulates as an `EntityState` each tick; a `Tracker` remembers
//! what one client has been told and produces the spawn, movement, metadata,
//! equipment and removal packets that bring it up to date.
use horizon_protocol::{
    buf::pack_degrees,
    entity_data::Value,
    item::Stack,
    packets::play::{
        self, AddEntity, MoveEntity, MoveEntityPos, MoveEntityPosRot, MoveEntityRot, RotateHead,
        SetEntityData, SetEntityMotion, SetEquipment, TeleportEntity, VecDelta,
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
    /// 4 chest, 5 head) and their stacks.
    pub equipment: Vec<(u8, Option<Stack>)>,
    /// Whether movement packets should include a velocity (items, projectiles).
    pub send_velocity: bool,
}

/// What a client was last told about one entity.
struct Sent {
    /// The position in 1/4096 blocks (VecDeltaCodec base).
    base: [i64; 3],
    position: [f64; 3],
    rotation: (i8, i8),
    head: i8,
    velocity: [f64; 3],
    meta: Vec<(u8, i32, Value)>,
    equipment: Vec<(u8, Option<Stack>)>,
    on_ground: bool,
    ticks: u32,
}

#[derive(Default)]
pub struct Tracker {
    sent: std::collections::HashMap<i32, Sent>,
}

fn quantize(p: [f64; 3]) -> [i64; 3] {
    [
        (p[0] * 4096.0).round() as i64,
        (p[1] * 4096.0).round() as i64,
        (p[2] * 4096.0).round() as i64,
    ]
}
fn dequantize(q: [i64; 3]) -> [f64; 3] {
    [
        q[0] as f64 / 4096.0,
        q[1] as f64 / 4096.0,
        q[2] as f64 / 4096.0,
    ]
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
    pub fn spawn(&mut self, e: &EntityState, entity_type: i32) -> Vec<play::Clientbound> {
        let rotation = (pack_degrees(e.pitch), pack_degrees(e.yaw));
        let head = pack_degrees(e.head_yaw);
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
        self.sent.insert(
            e.id,
            Sent {
                base: quantize(e.position),
                position: e.position,
                rotation,
                head,
                velocity: e.velocity,
                meta: e.meta.clone(),
                equipment: e.equipment.clone(),
                on_ground: e.on_ground,
                ticks: 0,
            },
        );
        out
    }

    /// Changes since the last update (ServerEntity.sendChanges).
    pub fn update(&mut self, e: &EntityState) -> Vec<play::Clientbound> {
        let Some(s) = self.sent.get_mut(&e.id) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        s.ticks += 1;
        let rotation = (pack_degrees(e.pitch), pack_degrees(e.yaw));
        let rotated = rotation != s.rotation;
        let now = quantize(e.position);
        let delta = [now[0] - s.base[0], now[1] - s.base[1], now[2] - s.base[2]];
        let moved = delta != [0; 3];
        let fits = delta.iter().all(|d| i16::try_from(*d).is_ok());
        if moved && !fits {
            out.push(play::Clientbound::TeleportEntity(TeleportEntity {
                id: e.id,
                position: e.position,
                delta: e.velocity,
                yaw: e.yaw,
                pitch: e.pitch,
                relatives: 0,
                on_ground: e.on_ground,
            }));
            s.base = now;
        } else if moved || rotated || e.on_ground != s.on_ground {
            let d = VecDelta::Linear([delta[0] as i16, delta[1] as i16, delta[2] as i16]);
            let rot = Some((rotation.1, rotation.0));
            let packet = if moved && rotated {
                play::Clientbound::MoveEntityPosRot(MoveEntityPosRot(MoveEntity {
                    id: e.id,
                    delta: Some(d),
                    rotation: rot,
                    on_ground: e.on_ground,
                }))
            } else if moved {
                play::Clientbound::MoveEntityPos(MoveEntityPos(MoveEntity {
                    id: e.id,
                    delta: Some(d),
                    rotation: None,
                    on_ground: e.on_ground,
                }))
            } else {
                play::Clientbound::MoveEntityRot(MoveEntityRot(MoveEntity {
                    id: e.id,
                    delta: None,
                    rotation: rot,
                    on_ground: e.on_ground,
                }))
            };
            out.push(packet);
            if moved {
                s.base = now;
            }
        }
        if moved || rotated || e.on_ground != s.on_ground {
            s.rotation = rotation;
            s.on_ground = e.on_ground;
        }
        s.position = dequantize(s.base);
        let head = pack_degrees(e.head_yaw);
        if head != s.head {
            out.push(play::Clientbound::RotateHead(RotateHead {
                id: e.id,
                head_yaw: head,
            }));
            s.head = head;
        }
        if e.send_velocity {
            let dv: f64 = (0..3)
                .map(|i| (e.velocity[i] - s.velocity[i]).powi(2))
                .sum();
            if dv > 1.0e-7 || (e.velocity == [0.0; 3] && s.velocity != [0.0; 3]) {
                out.push(play::Clientbound::SetEntityMotion(SetEntityMotion {
                    id: e.id,
                    velocity: e.velocity,
                }));
                s.velocity = e.velocity;
            }
        }
        if e.meta != s.meta {
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
        out
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
            send_velocity: false,
        }
    }

    #[test]
    fn small_moves_are_relative_and_large_ones_teleport() {
        let mut t = Tracker::default();
        let spawn = t.spawn(&state(0.0), 100);
        assert!(matches!(spawn[0], play::Clientbound::AddEntity(_)));
        assert!(t.update(&state(0.0)).is_empty());
        let step = t.update(&state(0.5));
        assert!(matches!(step[0], play::Clientbound::MoveEntityPos(_)));
        let jump = t.update(&state(40.0));
        assert!(matches!(jump[0], play::Clientbound::TeleportEntity(_)));
    }

    #[test]
    fn metadata_and_equipment_send_only_the_changes() {
        let mut t = Tracker::default();
        let mut e = state(0.0);
        t.spawn(&e, 100);
        e.meta.push((0, 0, Value::Byte(1)));
        e.equipment.push((0, Some(Stack::new(5, 1))));
        let out = t.update(&e);
        assert!(
            out.iter()
                .any(|p| matches!(p, play::Clientbound::SetEntityData(_)))
        );
        assert!(
            out.iter()
                .any(|p| matches!(p, play::Clientbound::SetEquipment(_)))
        );
        assert!(t.update(&e).is_empty());
        e.equipment.clear();
        let out = t.update(&e);
        let Some(play::Clientbound::SetEquipment(eq)) = out.first() else {
            panic!("expected equipment");
        };
        assert_eq!(eq.slots, vec![(0, None)]);
    }
}
