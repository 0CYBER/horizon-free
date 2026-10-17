//! The entities beyond mobs and items that a server tracks to its clients
//! (`ChunkMap.TrackedEntity`, `ServerEntity`): boats, minecarts, armor
//! stands, item frames, paintings and the entities Horizon keeps without
//! simulating them, each as vanilla pairs it (`getAddEntityPacket` and the
//! synched data that differs from the defaults), and who rides what.
use super::Game;
use horizon_client::lan::{
    entity::{EntityState, Riding},
    hub::Hub,
};
use horizon_protocol::entity_data::{Value, serializer};
use horizon_world::nbt::Tag;

/// `ArmorStand`'s default poses (head, body, left arm, right arm, left leg,
/// right leg), DATA_HEAD_POSE to DATA_RIGHT_LEG_POSE (entries 16 to 21),
/// under their `Pose` keys.
const STAND_POSES: [(&str, [f32; 3]); 6] = [
    ("Head", [0.0, 0.0, 0.0]),
    ("Body", [0.0, 0.0, 0.0]),
    ("LeftArm", [-10.0, 0.0, -10.0]),
    ("RightArm", [-15.0, 0.0, 10.0]),
    ("LeftLeg", [-1.0, 0.0, -1.0]),
    ("RightLeg", [1.0, 0.0, 1.0]),
];

fn state(id: u32, kind: &str, position: [f64; 3], yaw: f32, on_ground: bool) -> EntityState {
    EntityState {
        id: id as i32,
        uuid: super::lan::entity_uuid(id),
        kind: kind.to_owned(),
        position,
        yaw,
        pitch: 0.0,
        head_yaw: 0.0,
        on_ground,
        velocity: [0.0; 3],
        data: 0,
        meta: Vec::new(),
        equipment: Vec::new(),
    }
}

/// `VehicleEntity`'s DATA_ID_HURT, DATA_ID_HURTDIR and DATA_ID_DAMAGE
/// (entries 8 to 10) when they differ from their defaults (0, 1, 0).
fn vehicle_meta(hurt_time: i32, hurt_dir: i32, damage: f32) -> Vec<(u8, i32, Value)> {
    let mut meta = Vec::new();
    if hurt_time != 0 {
        meta.push((8, serializer::INT, Value::Int(hurt_time)));
    }
    if hurt_dir != 1 {
        meta.push((9, serializer::INT, Value::Int(hurt_dir)));
    }
    if damage != 0.0 {
        meta.push((10, serializer::FLOAT, Value::Float(damage)));
    }
    meta
}

/// An armor stand's pose from its compound (`ArmorStandPose`), each part
/// that differs from the default.
fn stand_poses(raw: Option<&Tag>) -> Vec<(u8, i32, Value)> {
    let Some(pose) = raw.and_then(|t| t.get("Pose")) else {
        return Vec::new();
    };
    let mut meta = Vec::new();
    for (i, (key, default)) in STAND_POSES.iter().enumerate() {
        let Some(Tag::List(values)) = pose.get(key) else {
            continue;
        };
        let mut r = *default;
        for (k, v) in values.iter().take(3).enumerate() {
            if let Tag::Float(bits) = v {
                r[k] = f32::from_bits(*bits);
            }
        }
        if r != *default {
            meta.push((
                16 + i as u8,
                serializer::ROTATIONS,
                Value::Floats(r.to_vec()),
            ));
        }
    }
    meta
}

impl Game<'_> {
    /// Who rides what (`Entity.getPassengers`): mobs on mobs, and the
    /// host in its boat or minecart.
    pub(super) fn lan_riding(&self, host_entity: Option<i32>, riding: &mut Riding) {
        riding.clear();
        for m in self.mobs.iter().filter(|m| !m.removed) {
            if let Some(vehicle) = m.extra.mount {
                riding.add(vehicle as i32, m.id as i32);
            }
        }
        if let (Some(host), Some(vehicle)) = (host_entity, self.riding) {
            riding.add(vehicle as i32, host);
        }
    }

    /// Boats, minecarts, armor stands, item frames, paintings and the
    /// entities kept unsimulated, as guests track them.
    pub(super) fn lan_object_states(&self, hub: &mut Hub, out: &mut Vec<EntityState>) {
        for b in &self.boats {
            let p = b.position;
            let mut e = state(b.id, &b.entity_type(), [p.x, p.y, p.z], b.yaw, b.on_ground);
            e.velocity = [b.velocity.x, b.velocity.y, b.velocity.z];
            e.meta = vehicle_meta(b.hurt_time, b.hurt_dir, b.damage);
            // AbstractBoat.DATA_ID_PADDLE_LEFT and DATA_ID_PADDLE_RIGHT.
            for (k, paddling) in b.paddling.iter().enumerate() {
                if *paddling {
                    e.meta
                        .push((11 + k as u8, serializer::BOOLEAN, Value::Bool(true)));
                }
            }
            out.push(e);
        }
        for c in &self.carts {
            let p = c.position;
            let mut e = state(c.id, c.kind.id(), [p.x, p.y, p.z], c.yaw, c.on_ground());
            e.velocity = [c.velocity.x, c.velocity.y, c.velocity.z];
            e.meta = vehicle_meta(c.hurt_time, c.hurt_dir, c.damage);
            match c.kind {
                // MinecartFurnace.DATA_ID_FUEL.
                super::minecarts::CartKind::Furnace if c.fuel > 0 => {
                    e.meta.push((13, serializer::BOOLEAN, Value::Bool(true)));
                }
                // MinecartCommandBlock.DATA_ID_COMMAND_NAME.
                super::minecarts::CartKind::CommandBlock => {
                    if let Some(Tag::String(command)) =
                        self.persist.raw.get(&c.id).and_then(|t| t.get("Command"))
                    {
                        let command = command.to_string_lossy();
                        if !command.is_empty() {
                            e.meta
                                .push((13, serializer::STRING, Value::String(command)));
                        }
                    }
                }
                _ => {}
            }
            out.push(e);
        }
        for a in &self.armor_stands {
            let p = a.position;
            let mut e = state(
                a.id,
                "minecraft:armor_stand",
                [p.x, p.y, p.z],
                a.yaw,
                a.on_ground,
            );
            e.head_yaw = a.yaw;
            e.velocity = [a.velocity.x, a.velocity.y, a.velocity.z];
            if a.fire_ticks > 0 {
                e.meta.push((0, serializer::BYTE, Value::Byte(1)));
            }
            // LivingEntity.DATA_HEALTH_ID: 1 until the stand's max health is set.
            e.meta.push((9, serializer::FLOAT, Value::Float(a.health)));
            // ArmorStand.DATA_CLIENT_FLAGS: small 1, arms 4, no base plate 8, marker 16.
            let flags = i8::from(a.small)
                | i8::from(a.show_arms) << 2
                | i8::from(a.no_base_plate) << 3
                | i8::from(a.marker) << 4;
            if flags != 0 {
                e.meta.push((15, serializer::BYTE, Value::Byte(flags)));
            }
            e.meta.extend(stand_poses(self.persist.raw.get(&a.id)));
            // Stored main hand, off hand, feet, legs, chest, head: the wire's slots 0 to 5.
            for (slot, item) in a.equipment.iter().enumerate() {
                if item.is_some() {
                    e.equipment.push((slot as u8, hub.stack(item)));
                }
            }
            out.push(e);
        }
        for f in &self.item_frames {
            // ItemFrame.getAddEntityPacket: the block it hangs in and its direction.
            let mut e = state(f.id, f.kind(), f.pos.map(f64::from), 0.0, false);
            e.pitch = match f.facing {
                0 => 90.0,
                1 => -90.0,
                _ => 0.0,
            };
            e.yaw = match f.facing {
                2 => 180.0,
                4 => 90.0,
                5 => 270.0,
                _ => 0.0,
            };
            e.data = i32::from(f.facing);
            // HangingEntity.DATA_DIRECTION, south by default.
            if f.facing != 3 {
                e.meta
                    .push((8, serializer::DIRECTION, Value::Id(i32::from(f.facing))));
            }
            if f.item.is_some() {
                e.meta
                    .push((9, serializer::ITEM_STACK, Value::Item(hub.stack(&f.item))));
            }
            if f.rotation != 0 {
                e.meta
                    .push((10, serializer::INT, Value::Int(i32::from(f.rotation))));
            }
            out.push(e);
        }
        for p in &self.paintings {
            let mut e = state(p.id, "minecraft:painting", p.pos.map(f64::from), 0.0, false);
            // Direction.get2DDataValue (south 0, west 1, north 2, east 3) to its
            // 3D data value and yaw.
            let facing = [3, 4, 2, 5][usize::from(p.facing & 3)];
            e.yaw = f32::from(p.facing & 3) * 90.0;
            e.data = facing;
            if facing != 3 {
                e.meta.push((8, serializer::DIRECTION, Value::Id(facing)));
            }
            // Painting.DATA_PAINTING_VARIANT_ID, a registry holder (id + 1),
            // set unless it is the registry's first variant (VariantUtils.getAny).
            if let Some(id) = hub.registry_id("minecraft:painting_variant", &p.variant)
                && id != 0
            {
                e.meta
                    .push((9, serializer::PAINTING_VARIANT, Value::Id(id + 1)));
            }
            out.push(e);
        }
        for o in &self.others {
            let p = o.position;
            let mut e = state(o.id, &o.kind, [p.x, p.y, p.z], o.yaw, false);
            e.pitch = o.pitch;
            e.head_yaw = o.yaw;
            out.push(e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn armor_stand_poses_send_only_what_differs() {
        let tag =
            horizon_world::nbt::parse_snbt("{Pose:{Head:[10f,20f,0f],LeftArm:[-10f,0f,-10f]}}")
                .unwrap()
                .0;
        let meta = stand_poses(Some(&tag));
        assert_eq!(
            meta,
            vec![(
                16,
                serializer::ROTATIONS,
                Value::Floats(vec![10.0, 20.0, 0.0])
            )]
        );
        assert!(stand_poses(None).is_empty());
    }
}
