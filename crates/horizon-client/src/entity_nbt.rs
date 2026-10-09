//! Vanilla 26.3 entity NBT (`Entity.saveWithoutId` and the subclasses'
//! `addAdditionalSaveData`) for the entities Horizon simulates: mobs, item
//! entities, experience orbs, arrows and falling blocks. Encoding starts from
//! the compound the entity was loaded with, when there is one, so fields
//! Horizon does not model (variants, attributes, equipment, custom names)
//! survive a Horizon save.
use crate::{
    entities::{ArmorStand, Arrow, ExperienceOrb, ItemEntity, ItemFrame, Painting},
    mobs::{Mob, MobType},
    save::{
        doubles, float, floats, item_from_nbt, item_to_nbt, number, put, read_doubles, text,
        uuid_from_tag,
    },
};
use horizon_core::geometry::Vec3;
use horizon_world::nbt::{Compound, Tag};
use std::{collections::HashMap, sync::Arc};

fn string(s: &str) -> Tag {
    Tag::String(s.into())
}
/// Entity type ID (`id`).
pub fn kind(tag: &Tag) -> Option<String> {
    text(tag.get("id"))
}
pub fn position(tag: &Tag) -> Option<Vec3> {
    read_doubles::<3>(tag.get("Pos")).map(|p| Vec3::new(p[0], p[1], p[2]))
}
pub fn uuid(tag: &Tag) -> Option<[i32; 4]> {
    uuid_from_tag(tag.get("UUID"))
}
fn motion(tag: &Tag) -> Vec3 {
    read_doubles::<3>(tag.get("Motion")).map_or(Vec3::ZERO, |m| Vec3::new(m[0], m[1], m[2]))
}
fn rotation(tag: &Tag) -> [f32; 2] {
    read_doubles::<2>(tag.get("Rotation")).map_or([0.0; 2], |r| [r[0] as f32, r[1] as f32])
}
fn flag(tag: &Tag, key: &str) -> bool {
    number(tag.get(key)).unwrap_or(0) != 0
}
/// `Entity.saveWithoutId` fields every entity has, over the loaded compound.
struct Base<'a> {
    id: &'a str,
    uuid: [i32; 4],
    position: Vec3,
    motion: Vec3,
    rotation: [f32; 2],
    on_ground: bool,
    fall_distance: f64,
}
impl Base<'_> {
    fn tag(&self, raw: Option<&Tag>) -> Tag {
        let mut tag = match raw {
            Some(t @ Tag::Compound(_)) => t.clone(),
            _ => Tag::Compound(Compound::new()),
        };
        for (key, value) in [
            ("id", string(self.id)),
            ("UUID", Tag::IntArray(self.uuid.to_vec())),
            (
                "Pos",
                doubles(&[self.position.x, self.position.y, self.position.z]),
            ),
            (
                "Motion",
                doubles(&[self.motion.x, self.motion.y, self.motion.z]),
            ),
            ("Rotation", floats(&self.rotation)),
            ("OnGround", Tag::Byte(i8::from(self.on_ground))),
            ("fall_distance", Tag::Double(self.fall_distance.to_bits())),
        ] {
            put(&mut tag, key, value);
        }
        if let Tag::Compound(map) = &mut tag {
            for (key, value) in [
                ("Fire", Tag::Short(-1)),
                ("Air", Tag::Short(300)),
                ("Invulnerable", Tag::Byte(0)),
                ("PortalCooldown", Tag::Int(0)),
            ] {
                map.entry(key.into()).or_insert(value);
            }
        }
        tag
    }
}

/// `LivingEntity` + `Mob` (+ `AgeableMob`, creeper and skeleton specifics).
pub fn mob_to_nbt(m: &Mob, uuid: [i32; 4], raw: Option<&Tag>) -> Tag {
    let base = Base {
        id: &m.kind.id,
        uuid,
        position: m.position,
        motion: m.velocity,
        rotation: [m.yaw, m.head_pitch],
        on_ground: m.on_ground,
        fall_distance: m.fall_distance,
    };
    let mut tag = base.tag(raw);
    for (key, value) in [
        ("Health", Tag::Float(m.health.max(0.0).to_bits())),
        (
            "HurtTime",
            Tag::Short(m.hurt_time.clamp(0, i32::from(i16::MAX)) as i16),
        ),
        (
            "DeathTime",
            Tag::Short(m.death_time.clamp(0, i32::from(i16::MAX)) as i16),
        ),
        ("PersistenceRequired", Tag::Byte(i8::from(m.persistent))),
        ("CustomNameVisible", Tag::Byte(i8::from(m.name_visible))),
    ] {
        put(&mut tag, key, value);
    }
    // A styled name from the save stays unless the name changed.
    if custom_name(&tag) != m.custom_name
        && let Some(name) = &m.custom_name
    {
        put(&mut tag, "CustomName", Tag::String(name.as_str().into()));
    }
    let Tag::Compound(map) = &mut tag else {
        return tag;
    };
    for (key, value) in [
        ("AbsorptionAmount", Tag::Float(0)),
        ("FallFlying", Tag::Byte(0)),
        ("CanPickUpLoot", Tag::Byte(0)),
        ("LeftHanded", Tag::Byte(0)),
        (
            "Brain",
            Tag::compound([("memories", Tag::Compound(Compound::new()))]),
        ),
    ] {
        map.entry(key.into()).or_insert(value);
    }
    let id = m.kind.id.as_str();
    if m.kind.category == "creature" {
        map.insert("Age".into(), Tag::Int(m.extra.age));
        map.insert("AgeLocked".into(), Tag::Byte(i8::from(m.extra.age_locked)));
        for (key, value) in [
            ("Age", Tag::Int(0)),
            ("ForcedAge", Tag::Int(0)),
            ("InLove", Tag::Int(0)),
            ("AgeLocked", Tag::Byte(0)),
        ] {
            map.entry(key.into()).or_insert(value);
        }
    }
    match id {
        "minecraft:creeper" => {
            for (key, value) in [
                ("Fuse", Tag::Short(30)),
                ("ExplosionRadius", Tag::Byte(3)),
                ("ignited", Tag::Byte(0)),
            ] {
                map.entry(key.into()).or_insert(value);
            }
            map.insert("powered".into(), Tag::Byte(i8::from(m.extra.charged)));
        }
        "minecraft:cat" => {
            if let Some(owner) = m.extra.owner {
                map.insert("Owner".into(), Tag::IntArray(owner.to_vec()));
            } else {
                map.remove(&"Owner".into());
            }
            map.insert("Sitting".into(), Tag::Byte(i8::from(m.extra.sitting)));
            map.insert("CollarColor".into(), Tag::Byte(m.extra.color as i8));
            let variant = crate::mobs::tame::CAT_VARIANTS[(m.extra.variant as usize).min(10)];
            map.insert("variant".into(), string(&format!("minecraft:{variant}")));
            map.entry("sound_variant".into())
                .or_insert_with(|| string("minecraft:classic"));
        }
        "minecraft:wolf" => {
            if let Some(owner) = m.extra.owner {
                map.insert("Owner".into(), Tag::IntArray(owner.to_vec()));
            } else {
                map.remove(&"Owner".into());
            }
            map.insert("Sitting".into(), Tag::Byte(i8::from(m.extra.sitting)));
            map.insert("CollarColor".into(), Tag::Byte(m.extra.color as i8));
            let variant = crate::mobs::tame::WOLF_VARIANTS[(m.extra.variant as usize).min(8)].0;
            map.insert("variant".into(), string(&format!("minecraft:{variant}")));
            map.entry("sound_variant".into())
                .or_insert_with(|| string("minecraft:classic"));
        }
        "minecraft:piglin" | "minecraft:piglin_brute" | "minecraft:hoglin" => {
            map.insert(
                "IsImmuneToZombification".into(),
                Tag::Byte(i8::from(m.extra.flag2)),
            );
            map.insert("TimeInOverworld".into(), Tag::Int(m.extra.counter2));
        }
        "minecraft:slime" | "minecraft:magma_cube" => {
            map.insert("Size".into(), Tag::Int(m.extra.size.max(1) - 1));
            map.insert("wasOnGround".into(), Tag::Byte(i8::from(m.extra.flag2)));
        }
        // Skeletons are spawned holding a bow (finalizeSpawn); a mob loaded
        // from NBT keeps only its saved equipment.
        "minecraft:skeleton" | "minecraft:stray" | "minecraft:bogged" => {
            map.entry("equipment".into()).or_insert_with(|| {
                Tag::compound([(
                    "mainhand",
                    Tag::compound([("id", string("minecraft:bow")), ("count", Tag::Int(1))]),
                )])
            });
        }
        "minecraft:zombie" => {
            for (key, value) in [
                ("IsBaby", Tag::Byte(0)),
                ("CanBreakDoors", Tag::Byte(0)),
                ("DrownedConversionTime", Tag::Int(-1)),
                ("InWaterTime", Tag::Int(-1)),
            ] {
                map.entry(key.into()).or_insert(value);
            }
        }
        "minecraft:sheep" => {
            for (key, value) in [("Color", Tag::Byte(0)), ("Sheared", Tag::Byte(0))] {
                map.entry(key.into()).or_insert(value);
            }
        }
        _ => {}
    }
    tag
}
/// `CustomName`: a text component saved as a string or a `{text: ..}` compound.
fn custom_name(tag: &Tag) -> Option<String> {
    match tag.get("CustomName")? {
        Tag::String(s) => Some(s.to_string_lossy()),
        t @ Tag::Compound(_) => match t.get("text") {
            Some(Tag::String(s)) => Some(s.to_string_lossy()),
            _ => None,
        },
        _ => None,
    }
}
pub fn mob_from_nbt(tag: &Tag, types: &HashMap<String, Arc<MobType>>, id: u32) -> Option<Mob> {
    let kind = types.get(&self::kind(tag)?)?.clone();
    let position = position(tag)?;
    let [yaw, pitch] = rotation(tag);
    let mut m = Mob::new(id, kind, position, yaw);
    m.velocity = motion(tag);
    m.head_pitch = pitch;
    m.on_ground = flag(tag, "OnGround");
    m.fall_distance = float(tag.get("fall_distance")).unwrap_or(0.0);
    if let Some(h) = float(tag.get("Health")) {
        m.health = h as f32;
    }
    m.hurt_time = number(tag.get("HurtTime")).unwrap_or(0) as i32;
    m.death_time = number(tag.get("DeathTime")).unwrap_or(0) as i32;
    m.persistent = flag(tag, "PersistenceRequired");
    m.custom_name = custom_name(tag);
    if m.kind.category == "creature" {
        m.extra.age = number(tag.get("Age")).unwrap_or(0) as i32;
        m.extra.age_locked = flag(tag, "AgeLocked");
    }
    m.name_visible = flag(tag, "CustomNameVisible");
    if m.species() == crate::mobs::Species::Creeper {
        m.extra.charged = flag(tag, "powered");
    }
    if matches!(
        m.species(),
        crate::mobs::Species::Piglin
            | crate::mobs::Species::PiglinBrute
            | crate::mobs::Species::Hoglin
    ) {
        m.extra.flag2 = flag(tag, "IsImmuneToZombification");
        m.extra.counter2 = number(tag.get("TimeInOverworld")).unwrap_or(0) as i32;
    }
    if m.species() == crate::mobs::Species::Cat {
        m.extra.owner = crate::save::uuid_from_tag(tag.get("Owner"));
        m.extra.tame = m.extra.owner.is_some();
        m.extra.sitting = flag(tag, "Sitting");
        m.extra.color =
            number(tag.get("CollarColor")).map_or(crate::mobs::tame::RED, |c| (c as u8) & 15);
        if let Some(Tag::String(v)) = tag.get("variant") {
            let v = v.to_string().unwrap_or_default();
            let v = v.trim_start_matches("minecraft:");
            m.extra.variant = crate::mobs::tame::CAT_VARIANTS
                .iter()
                .position(|n| *n == v)
                .unwrap_or(0) as u8;
        }
    }
    if m.species() == crate::mobs::Species::Wolf {
        let health = m.health;
        m.extra.owner = crate::save::uuid_from_tag(tag.get("Owner"));
        let tame = m.extra.owner.is_some();
        crate::mobs::tame::set_tame(&mut m, tame);
        m.health = health;
        m.extra.sitting = flag(tag, "Sitting");
        m.extra.color =
            number(tag.get("CollarColor")).map_or(crate::mobs::tame::RED, |c| (c as u8) & 15);
        if let Some(Tag::String(v)) = tag.get("variant") {
            let v = v.to_string().unwrap_or_default();
            let v = v.trim_start_matches("minecraft:");
            m.extra.variant = crate::mobs::tame::WOLF_VARIANTS
                .iter()
                .position(|(n, _)| *n == v)
                .unwrap_or(0) as u8;
        }
    }
    if matches!(
        m.species(),
        crate::mobs::Species::Slime | crate::mobs::Species::MagmaCube
    ) {
        let health = m.health;
        crate::mobs::cube::set_size(&mut m, number(tag.get("Size")).unwrap_or(0) as i32 + 1);
        m.health = health;
        m.extra.flag2 = flag(tag, "wasOnGround");
    }
    // A mob saved mid-death finishes dying; one saved dead is dropped.
    if m.health <= 0.0 && m.death_time == 0 {
        return None;
    }
    Some(m)
}

/// `ItemEntity`: Health, Age, PickupDelay, Item.
pub fn item_entity_to_nbt(e: &ItemEntity, uuid: [i32; 4], raw: Option<&Tag>) -> Tag {
    let base = Base {
        id: "minecraft:item",
        uuid,
        position: e.position,
        motion: e.velocity,
        rotation: [0.0, 0.0],
        on_ground: e.on_ground,
        fall_distance: 0.0,
    };
    let mut tag = base.tag(raw);
    for (key, value) in [
        ("Item", item_to_nbt(&e.stack)),
        ("Age", Tag::Short(e.age.min(i16::MAX as u32) as i16)),
        (
            "PickupDelay",
            Tag::Short(e.pickup_delay.min(i16::MAX as u32) as i16),
        ),
    ] {
        put(&mut tag, key, value);
    }
    if let Tag::Compound(map) = &mut tag {
        map.entry("Health".into()).or_insert(Tag::Short(5));
    }
    tag
}
pub fn item_entity_from_nbt(tag: &Tag, id: u32, bob_offset: f32) -> Option<ItemEntity> {
    let stack = item_from_nbt(tag.get("Item")?)?;
    let position = position(tag)?;
    Some(ItemEntity {
        id,
        stack,
        position,
        previous: position,
        velocity: motion(tag),
        on_ground: flag(tag, "OnGround"),
        // Age -32768 marks items that never despawn; Horizon counts upwards.
        age: number(tag.get("Age")).unwrap_or(0).max(0) as u32,
        pickup_delay: number(tag.get("PickupDelay")).unwrap_or(0).clamp(0, 32767) as u32,
        bob_offset,
    })
}

/// `ExperienceOrb`: Health, Age, Value, Count.
pub fn orb_to_nbt(o: &ExperienceOrb, uuid: [i32; 4]) -> Tag {
    let base = Base {
        id: "minecraft:experience_orb",
        uuid,
        position: o.position,
        motion: o.velocity,
        rotation: [0.0, 0.0],
        on_ground: o.on_ground,
        fall_distance: 0.0,
    };
    let mut tag = base.tag(None);
    for (key, value) in [
        ("Health", Tag::Short(5)),
        ("Age", Tag::Short(o.age.min(i16::MAX as u32) as i16)),
        ("Value", Tag::Short(o.value.min(i16::MAX as u32) as i16)),
        ("Count", Tag::Int(1)),
    ] {
        put(&mut tag, key, value);
    }
    tag
}
/// One Horizon orb per counted orb (vanilla merges orbs of equal value).
pub fn orbs_from_nbt(tag: &Tag) -> Vec<ExperienceOrb> {
    let Some(position) = position(tag) else {
        return Vec::new();
    };
    let count = number(tag.get("Count")).unwrap_or(1).clamp(1, 64) as usize;
    let orb = ExperienceOrb {
        position,
        previous: position,
        velocity: motion(tag),
        value: number(tag.get("Value")).unwrap_or(1).max(1) as u32,
        age: number(tag.get("Age")).unwrap_or(0).max(0) as u32,
        on_ground: flag(tag, "OnGround"),
    };
    vec![orb; count]
}

/// `AbstractArrow`: damage, crit, inGround, life, pickup, item.
pub fn arrow_to_nbt(a: &Arrow, uuid: [i32; 4], owner: Option<[i32; 4]>) -> Tag {
    let v = a.velocity;
    let horizontal = (v.x * v.x + v.z * v.z).sqrt();
    let rotation = [
        (v.x.atan2(v.z).to_degrees()) as f32,
        (v.y.atan2(horizontal).to_degrees()) as f32,
    ];
    let base = Base {
        id: "minecraft:arrow",
        uuid,
        position: a.position,
        motion: a.velocity,
        rotation,
        on_ground: false,
        fall_distance: 0.0,
    };
    let mut tag = base.tag(None);
    for (key, value) in [
        ("damage", Tag::Double(a.base_damage.to_bits())),
        ("crit", Tag::Byte(i8::from(a.critical))),
        ("inGround", Tag::Byte(i8::from(a.stuck))),
        (
            "life",
            Tag::Short(if a.stuck { a.age.min(1200) as i16 } else { 0 }),
        ),
        // Pickup: allowed for the player's arrows, disallowed for mobs'.
        ("pickup", Tag::Byte(i8::from(a.from_player))),
        ("shake", Tag::Byte(0)),
        ("PierceLevel", Tag::Byte(0)),
        ("HasBeenShot", Tag::Byte(1)),
        ("SoundEvent", string("minecraft:entity.arrow.hit")),
        (
            "item",
            Tag::compound([("id", string("minecraft:arrow")), ("count", Tag::Int(1))]),
        ),
    ] {
        put(&mut tag, key, value);
    }
    if let Some(owner) = owner {
        put(&mut tag, "Owner", Tag::IntArray(owner.to_vec()));
    }
    tag
}
pub fn arrow_from_nbt(tag: &Tag, player: Option<[i32; 4]>) -> Option<Arrow> {
    let position = position(tag)?;
    let stuck = flag(tag, "inGround");
    Some(Arrow {
        from_player: player.is_some() && uuid_from_tag(tag.get("Owner")) == player
            || number(tag.get("pickup")) == Some(1),
        critical: flag(tag, "crit"),
        position,
        previous: position,
        velocity: motion(tag),
        stuck,
        age: if stuck {
            number(tag.get("life")).unwrap_or(0).max(0) as u32
        } else {
            0
        },
        base_damage: float(tag.get("damage")).unwrap_or(2.0),
    })
}

/// `FallingBlockEntity`: BlockState, Time, DropItem, HurtEntities.
pub fn falling_block_to_nbt(
    state: Tag,
    position: Vec3,
    motion: Vec3,
    time: u32,
    uuid: [i32; 4],
) -> Tag {
    let base = Base {
        id: "minecraft:falling_block",
        uuid,
        position,
        motion,
        rotation: [0.0, 0.0],
        on_ground: false,
        fall_distance: 0.0,
    };
    let mut tag = base.tag(None);
    for (key, value) in [
        ("BlockState", state),
        ("Time", Tag::Int(time as i32)),
        ("DropItem", Tag::Byte(1)),
        ("HurtEntities", Tag::Byte(0)),
        ("CancelDrop", Tag::Byte(0)),
        ("FallHurtAmount", Tag::Float(0)),
        ("FallHurtMax", Tag::Int(40)),
    ] {
        put(&mut tag, key, value);
    }
    tag
}
/// (block state tag, position, motion, time)
pub fn falling_block_from_nbt(tag: &Tag) -> Option<(Tag, Vec3, Vec3, u32)> {
    Some((
        tag.get("BlockState")?.clone(),
        position(tag)?,
        motion(tag),
        number(tag.get("Time")).unwrap_or(0).max(0) as u32,
    ))
}

/// Painting.addAdditionalSaveData: `facing`, `variant` and `block_pos`.
pub fn painting_to_nbt(p: &Painting, uuid: [i32; 4], raw: Option<&Tag>) -> Tag {
    let (lo, hi) = p.bounds();
    let base = Base {
        id: "minecraft:painting",
        uuid,
        position: Vec3::new(
            (lo[0] + hi[0]) / 2.0,
            (lo[1] + hi[1]) / 2.0,
            (lo[2] + hi[2]) / 2.0,
        ),
        motion: Vec3::ZERO,
        rotation: [f32::from(p.facing) * 90.0, 0.0],
        on_ground: false,
        fall_distance: 0.0,
    };
    let mut tag = base.tag(raw);
    put(&mut tag, "facing", Tag::Byte(p.facing as i8));
    put(&mut tag, "variant", string(&p.variant));
    put(&mut tag, "block_pos", Tag::IntArray(p.pos.to_vec()));
    tag
}
/// The block position, facing and variant of a saved painting.
pub fn painting_from_nbt(tag: &Tag) -> Option<([i32; 3], u8, String)> {
    let pos = match tag.get("block_pos") {
        Some(Tag::IntArray(v)) if v.len() == 3 => [v[0], v[1], v[2]],
        _ => return None,
    };
    let facing = (number(tag.get("facing")).unwrap_or(0) & 3) as u8;
    Some((pos, facing, text(tag.get("variant"))?))
}
/// ItemFrame.addAdditionalSaveData.
pub fn item_frame_to_nbt(f: &ItemFrame, uuid: [i32; 4], raw: Option<&Tag>) -> Tag {
    let (lo, hi) = f.bounds();
    let base = Base {
        id: f.kind(),
        uuid,
        position: Vec3::new(
            (lo[0] + hi[0]) / 2.0,
            (lo[1] + hi[1]) / 2.0,
            (lo[2] + hi[2]) / 2.0,
        ),
        motion: Vec3::ZERO,
        rotation: [0.0, 0.0],
        on_ground: false,
        fall_distance: 0.0,
    };
    let mut tag = base.tag(raw);
    put(&mut tag, "Facing", Tag::Byte(f.facing as i8));
    put(&mut tag, "block_pos", Tag::IntArray(f.pos.to_vec()));
    put(&mut tag, "ItemRotation", Tag::Byte(f.rotation as i8));
    put(
        &mut tag,
        "ItemDropChance",
        Tag::Float(f.drop_chance.to_bits()),
    );
    put(&mut tag, "Invisible", Tag::Byte(i8::from(f.invisible)));
    put(&mut tag, "Fixed", Tag::Byte(i8::from(f.fixed)));
    if let Tag::Compound(map) = &mut tag {
        map.remove(&horizon_world::nbt::JavaString::from("Item"));
    }
    if let Some(item) = &f.item {
        put(&mut tag, "Item", item_to_nbt(item));
    }
    tag
}
pub fn item_frame_from_nbt(tag: &Tag, id: u32) -> Option<ItemFrame> {
    let pos = match tag.get("block_pos") {
        Some(Tag::IntArray(v)) if v.len() == 3 => [v[0], v[1], v[2]],
        _ => return None,
    };
    Some(ItemFrame {
        id,
        pos,
        facing: number(tag.get("Facing")).unwrap_or(2).clamp(0, 5) as u8,
        glow: kind(tag).as_deref() == Some("minecraft:glow_item_frame"),
        item: tag.get("Item").and_then(item_from_nbt),
        rotation: (number(tag.get("ItemRotation")).unwrap_or(0) & 7) as u8,
        drop_chance: float(tag.get("ItemDropChance")).map_or(1.0, |v| v as f32),
        fixed: flag(tag, "Fixed"),
        invisible: flag(tag, "Invisible"),
        check_interval: 0,
    })
}
/// ArmorStand.addAdditionalSaveData with LivingEntity's `equipment`.
pub fn armor_stand_to_nbt(a: &ArmorStand, uuid: [i32; 4], raw: Option<&Tag>) -> Tag {
    let base = Base {
        id: "minecraft:armor_stand",
        uuid,
        position: a.position,
        motion: a.velocity,
        rotation: [a.yaw, 0.0],
        on_ground: a.on_ground,
        fall_distance: 0.0,
    };
    let mut tag = base.tag(raw);
    let equipment: Vec<(&str, Tag)> = ArmorStand::SLOTS
        .iter()
        .zip(&a.equipment)
        .filter_map(|(slot, stack)| Some((*slot, item_to_nbt(stack.as_ref()?))))
        .collect();
    put(&mut tag, "equipment", Tag::compound(equipment));
    for (key, value) in [
        ("Small", a.small),
        ("ShowArms", a.show_arms),
        ("NoBasePlate", a.no_base_plate),
        ("Invisible", a.invisible),
        ("Marker", a.marker),
    ] {
        put(&mut tag, key, Tag::Byte(i8::from(value)));
    }
    if let Tag::Compound(map) = &tag
        && !map.contains_key(&horizon_world::nbt::JavaString::from("Health"))
    {
        put(&mut tag, "Health", Tag::Float(20f32.to_bits()));
    }
    tag
}
pub fn armor_stand_from_nbt(tag: &Tag, id: u32) -> Option<ArmorStand> {
    let mut a = ArmorStand::new(id, position(tag)?, rotation(tag)[0]);
    a.velocity = motion(tag);
    if let Some(eq) = tag.get("equipment") {
        for (i, slot) in ArmorStand::SLOTS.iter().enumerate() {
            a.equipment[i] = eq.get(slot).and_then(item_from_nbt);
        }
    }
    a.small = flag(tag, "Small");
    a.show_arms = flag(tag, "ShowArms");
    a.no_base_plate = flag(tag, "NoBasePlate");
    a.invisible = flag(tag, "Invisible");
    a.marker = flag(tag, "Marker");
    a.on_ground = flag(tag, "OnGround");
    Some(a)
}
/// A boat entity (`minecraft:<wood>_boat`, `minecraft:bamboo_raft`).
pub fn boat_to_nbt(
    wood: &str,
    position: Vec3,
    motion: Vec3,
    yaw: f32,
    uuid: [i32; 4],
    raw: Option<&Tag>,
) -> Tag {
    let id = if wood == "bamboo" {
        "minecraft:bamboo_raft".to_owned()
    } else {
        format!("minecraft:{wood}_boat")
    };
    Base {
        id: &id,
        uuid,
        position,
        motion,
        rotation: [yaw, 0.0],
        on_ground: false,
        fall_distance: 0.0,
    }
    .tag(raw)
}
/// A vehicle's base entity tag (boats and minecarts).
pub fn vehicle_to_nbt(
    id: &str,
    position: Vec3,
    motion: Vec3,
    yaw: f32,
    uuid: [i32; 4],
    raw: Option<&Tag>,
) -> Tag {
    Base {
        id,
        uuid,
        position,
        motion,
        rotation: [yaw, 0.0],
        on_ground: false,
        fall_distance: 0.0,
    }
    .tag(raw)
}
pub fn boat_from_nbt(tag: &Tag) -> Option<(Vec3, Vec3, f32)> {
    Some((position(tag)?, motion(tag), rotation(tag)[0]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::items::ItemStack;

    #[test]
    fn item_entities_round_trip() {
        let e = ItemEntity {
            id: 7,
            stack: ItemStack {
                item: Arc::from("minecraft:diamond_pickaxe"),
                count: 1,
                damage: 5,
                extra: None,
            },
            position: Vec3::new(100.5, 64.0, -31.5),
            previous: Vec3::ZERO,
            velocity: Vec3::new(0.0, -0.04, 0.0),
            on_ground: true,
            age: 120,
            pickup_delay: 10,
            bob_offset: 0.0,
        };
        let tag = item_entity_to_nbt(&e, [1, 2, 3, 4], None);
        assert_eq!(kind(&tag).as_deref(), Some("minecraft:item"));
        assert_eq!(uuid(&tag), Some([1, 2, 3, 4]));
        let back = item_entity_from_nbt(&tag, 9, 0.0).unwrap();
        assert_eq!(back.stack.damage, 5);
        assert_eq!(
            (back.age, back.pickup_delay, back.on_ground),
            (120, 10, true)
        );
        assert_eq!(back.position.x, 100.5);
    }
    #[test]
    fn unknown_fields_survive() {
        let raw = Tag::compound([
            ("id", string("minecraft:item")),
            ("CustomName", string("x")),
            ("Pos", doubles(&[0.5, 64.0, 0.5])),
            (
                "Item",
                Tag::compound([("id", string("minecraft:stone")), ("count", Tag::Int(3))]),
            ),
        ]);
        let e = item_entity_from_nbt(&raw, 1, 0.0).unwrap();
        let tag = item_entity_to_nbt(&e, [0; 4], Some(&raw));
        assert_eq!(text(tag.get("CustomName")).as_deref(), Some("x"));
        assert_eq!(number(tag.get("Health")), Some(5));
    }
    #[test]
    fn orbs_expand_their_count() {
        let o = ExperienceOrb {
            position: Vec3::new(1.0, 2.0, 3.0),
            previous: Vec3::ZERO,
            velocity: Vec3::ZERO,
            value: 7,
            age: 3,
            on_ground: false,
        };
        let mut tag = orb_to_nbt(&o, [0; 4]);
        put(&mut tag, "Count", Tag::Int(3));
        let back = orbs_from_nbt(&tag);
        assert_eq!(back.len(), 3);
        assert_eq!(back[0].value, 7);
    }
}
