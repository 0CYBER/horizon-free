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
        attributes: Vec::new(),
        precise: false,
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
    pub(super) fn lan_object_states(
        &self,
        host_entity: Option<i32>,
        hub: &mut Hub,
        out: &mut Vec<EntityState>,
    ) {
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
            e.attributes = self.stand_attributes(a.id, &a.equipment);
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
        self.lan_projectile_states(host_entity, hub, out);
        for o in &self.others {
            let p = o.position;
            let mut e = state(o.id, &o.kind, [p.x, p.y, p.z], o.yaw, false);
            e.pitch = o.pitch;
            e.head_yaw = o.yaw;
            out.push(e);
        }
    }
}

impl Game<'_> {
    /// Gives the entities Horizon keeps without an id one (`Entity.id`, from
    /// the same counter as the rest) before they are first tracked.
    pub(super) fn lan_assign_ids(&mut self) {
        let next = &mut self.next_entity;
        let mut give = |id: &mut u32| {
            if *id == 0 {
                *next += 1;
                *id = *next;
            }
        };
        self.tnts.iter_mut().for_each(|t| give(&mut t.id));
        self.falling.iter_mut().for_each(|f| give(&mut f.id));
        self.creatures
            .projectiles
            .iter_mut()
            .for_each(|p| give(&mut p.id));
        self.tridents.iter_mut().for_each(|t| give(&mut t.id));
        self.rockets.iter_mut().for_each(|r| give(&mut r.id));
        self.crystals.iter_mut().for_each(|c| give(&mut c.id));
    }

    /// Primed TNT, falling blocks, projectiles, tridents, firework rockets and
    /// end crystals, as guests track them.
    fn lan_projectile_states(
        &self,
        host_entity: Option<i32>,
        hub: &mut Hub,
        out: &mut Vec<EntityState>,
    ) {
        let owner = |who: Option<horizon_client::mobs::Who>| match who {
            Some(horizon_client::mobs::Who::Mob(id)) => id as i32,
            Some(horizon_client::mobs::Who::Player) => host_entity.unwrap_or(0),
            None => 0,
        };
        let at = |p: horizon_core::geometry::Vec3| [p.x, p.y, p.z];
        for t in self.tnts.iter().filter(|t| t.id != 0) {
            let mut e = state(t.id, "minecraft:tnt", at(t.position), 0.0, t.on_ground);
            e.velocity = at(t.velocity);
            // PrimedTnt.DATA_FUSE_ID, 80 by default; requiresPrecisePosition
            // while it touches a block.
            if t.fuse != 80 {
                e.meta.push((8, serializer::INT, Value::Int(t.fuse)));
            }
            e.precise = t.on_ground;
            out.push(e);
        }
        for f in self.falling.iter().filter(|f| f.id != 0) {
            let mut e = state(f.id, "minecraft:falling_block", at(f.position), 0.0, false);
            e.velocity = at(f.velocity);
            // getAddEntityPacket: the block state; DATA_START_POS.
            e.data = f.state.0 as i32;
            if f.no_gravity {
                e.meta.push((5, serializer::BOOLEAN, Value::Bool(true)));
            }
            let start = [f.start_pos.x, f.start_pos.y, f.start_pos.z];
            if start != [0; 3] {
                e.meta.push((8, serializer::BLOCK_POS, Value::Pos(start)));
            }
            out.push(e);
        }
        for p in self.creatures.projectiles.iter().filter(|p| p.id != 0) {
            use horizon_client::projectiles::ProjectileKind as K;
            let kind = match &p.kind {
                K::Arrow { .. } => "minecraft:arrow",
                K::Snowball => "minecraft:snowball",
                K::Egg => "minecraft:egg",
                K::EnderPearl => "minecraft:ender_pearl",
                K::SmallFireball => "minecraft:small_fireball",
                K::LargeFireball { .. } => "minecraft:fireball",
                K::LlamaSpit => "minecraft:llama_spit",
                K::WindCharge => "minecraft:wind_charge",
                K::BreezeWindCharge => "minecraft:breeze_wind_charge",
                K::ShulkerBullet { .. } => "minecraft:shulker_bullet",
                K::WitherSkull { .. } => "minecraft:wither_skull",
                K::Potion { .. } => "minecraft:splash_potion",
            };
            let mut e = state(p.id, kind, at(p.position), p.yaw, false);
            e.pitch = p.pitch;
            e.velocity = at(p.velocity);
            // Projectile.getAddEntityPacket: the owner's id.
            e.data = match p.guest.and_then(|conn| hub.guest(conn)) {
                Some(g) => g.entity_id,
                None => owner(p.owner),
            };
            match &p.kind {
                K::Arrow { critical, .. } => {
                    let a = p.arrow.as_deref();
                    e.meta.extend(arrow_meta(
                        *critical,
                        a.is_some_and(|a| a.no_physics),
                        a.map_or(0, |a| a.pierce),
                        a.is_some_and(|a| a.in_ground) || p.stuck,
                    ));
                }
                // WitherSkull.DATA_DANGEROUS.
                K::WitherSkull { dangerous: true } => {
                    e.meta.push((8, serializer::BOOLEAN, Value::Bool(true)));
                }
                _ => {}
            }
            out.push(e);
        }
        for t in self.tridents.iter().filter(|t| t.id != 0) {
            let mut e = state(t.id, "minecraft:trident", at(t.position), t.yaw, false);
            e.pitch = t.pitch;
            e.velocity = at(t.velocity);
            e.meta = arrow_meta(false, t.no_physics, 0, t.in_ground);
            // ThrownTrident.ID_LOYALTY and ID_FOIL.
            if t.loyalty != 0 {
                e.meta
                    .push((11, serializer::BYTE, Value::Byte(t.loyalty as i8)));
            }
            if t.stack.has_any_enchantments() {
                e.meta.push((12, serializer::BOOLEAN, Value::Bool(true)));
            }
            out.push(e);
        }
        for r in self.rockets.iter().filter(|r| r.id != 0) {
            let mut e = state(
                r.id,
                "minecraft:firework_rocket",
                at(r.position),
                0.0,
                false,
            );
            e.velocity = at(r.velocity);
            // DATA_ID_FIREWORKS_ITEM once set from a stack, DATA_ATTACHED_TO_TARGET,
            // DATA_SHOT_AT_ANGLE.
            let plain = &*r.item.item == "minecraft:firework_rocket"
                && r.item.count == 1
                && r.item.extra.is_none();
            if !plain {
                e.meta.push((
                    8,
                    serializer::ITEM_STACK,
                    Value::Item(hub.stack(&Some(r.item.clone()))),
                ));
            }
            if r.attached
                && let Some(host) = host_entity
            {
                e.meta
                    .push((9, serializer::OPTIONAL_UNSIGNED_INT, Value::Id(host + 1)));
            }
            if r.angled {
                e.meta.push((10, serializer::BOOLEAN, Value::Bool(true)));
            }
            out.push(e);
        }
        for c in self.crystals.iter().filter(|c| c.id != 0) {
            let mut e = state(c.id, "minecraft:end_crystal", at(c.position), 0.0, false);
            // EndCrystal.DATA_BEAM_TARGET and DATA_SHOW_BOTTOM (true by default).
            if c.beam.is_some() {
                e.meta
                    .push((8, serializer::OPTIONAL_BLOCK_POS, Value::OptPos(c.beam)));
            }
            if !c.show_bottom {
                e.meta.push((9, serializer::BOOLEAN, Value::Bool(false)));
            }
            out.push(e);
        }
        out.extend(self.lan_dragon_state());
    }
}

/// `AbstractArrow`'s ID_FLAGS (critical 1, no physics 2), PIERCE_LEVEL and
/// IN_GROUND, when they differ from the defaults.
fn arrow_meta(
    critical: bool,
    no_physics: bool,
    pierce: u8,
    in_ground: bool,
) -> Vec<(u8, i32, Value)> {
    let mut meta = Vec::new();
    let flags = i8::from(critical) | i8::from(no_physics) << 1;
    if flags != 0 {
        meta.push((8, serializer::BYTE, Value::Byte(flags)));
    }
    if pierce != 0 {
        meta.push((9, serializer::BYTE, Value::Byte(pierce as i8)));
    }
    if in_ground {
        meta.push((10, serializer::BOOLEAN, Value::Bool(true)));
    }
    meta
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
