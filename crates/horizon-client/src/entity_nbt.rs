//! Vanilla 26.3 entity NBT (`Entity.saveWithoutId` and the subclasses'
//! `addAdditionalSaveData`) for the entities Horizon simulates: mobs, item
//! entities, experience orbs, arrows and falling blocks. Encoding starts from
//! the compound the entity was loaded with, when there is one, so fields
//! Horizon does not model (variants, attributes, equipment, custom names)
//! survive a Horizon save.
use crate::{
    arrows::Pickup,
    entities::{ArmorStand, Cushion, ExperienceOrb, ItemEntity, ItemFrame, Painting},
    mobs::{Mob, MobType, Who},
    nbt_path::int_value,
    projectiles::{Projectile, ProjectileKind},
    save::{
        doubles, float, floats, item_from_nbt, item_to_nbt, number, put, read_doubles, text,
        uuid_from_tag, uuid_tag,
    },
};
use horizon_core::{geometry::Vec3, random::LegacyRandom};
use std::{collections::HashMap, sync::Arc};
mod boat_load;
pub use boat_load::boat_from_nbt;
mod projectile_fields;
use projectile_fields::{load_projectile_entity_fields, put_projectile_entity_fields};
mod ai_pause;
pub use ai_pause::write_ai_paused;
mod mob_death;
mod refs;
mod saved_attributes;
pub use refs::who_by_uuid;

use {
    horizon_core::nbt::{Compound, Tag},
    horizon_world::{
        chunk::BlockState,
        registry::{BlockRegistry, BlockStateId},
    },
};

/// Saved data carried by `ConversionType.convertCommon` outside the modelled
/// mob fields. The converted entity has its own identity and type-specific data;
/// only its command tags are inherited from the old entity's raw compound.
pub fn conversion_tags(raw: Option<&Tag>) -> Option<Tag> {
    let Tag::List(tags) = raw?.get("Tags")? else {
        return None;
    };
    (!tags.is_empty()).then(|| Tag::compound([("Tags", Tag::List(tags.clone()))]))
}

fn string(s: &str) -> Tag {
    Tag::String(s.into())
}

fn hidden_effect_nbt(effect: &crate::effects::EffectInstance) -> Tag {
    let mut fields = vec![("duration", Tag::Int(effect.duration))];
    if effect.amplifier != 0 {
        fields.push((
            "amplifier",
            Tag::Byte(effect.amplifier.clamp(0, 255) as u8 as i8),
        ));
    }
    if effect.ambient {
        fields.push(("ambient", Tag::Byte(1)));
    }
    if !effect.visible {
        fields.push(("show_particles", Tag::Byte(0)));
    }
    fields.push(("show_icon", Tag::Byte(i8::from(effect.show_icon))));
    if let Some(hidden) = &effect.hidden {
        fields.push(("hidden_effect", hidden_effect_nbt(hidden)));
    }
    Tag::compound(fields)
}

fn hidden_effect_from_nbt(effect: &'static str, tag: &Tag) -> crate::effects::EffectInstance {
    let mut instance = crate::effects::EffectInstance::new(
        effect,
        number(tag.get("duration")).unwrap_or(0) as i32,
        number(tag.get("amplifier")).unwrap_or(0) as i32,
    );
    instance.ambient = number(tag.get("ambient")).is_some_and(|v| v != 0);
    instance.visible = number(tag.get("show_particles")) != Some(0);
    instance.show_icon = number(tag.get("show_icon")).map_or(instance.visible, |v| v != 0);
    instance.hidden = tag
        .get("hidden_effect")
        .map(|hidden| Box::new(hidden_effect_from_nbt(effect, hidden)));
    instance
}
/// Entity type ID (`id`).
pub fn kind(tag: &Tag) -> Option<String> {
    horizon_client_core::identifier::normalize_id(&text(tag.get("id"))?)
}
/// The clamp `Entity.load` applies to every saved position: ±3.0000512E7 on x
/// and z, ±2.0E7 on y.
pub fn clamp_loaded_position(p: [f64; 3]) -> [f64; 3] {
    [
        p[0].clamp(-30_000_512.0, 30_000_512.0),
        p[1].clamp(-20_000_000.0, 20_000_000.0),
        p[2].clamp(-30_000_512.0, 30_000_512.0),
    ]
}
/// `Entity.load`'s `Pos`: the saved coordinates, clamped to the world's limits.
pub fn position(tag: &Tag) -> Option<Vec3> {
    read_doubles::<3>(tag.get("Pos")).map(|p| {
        let p = clamp_loaded_position(p);
        Vec3::new(p[0], p[1], p[2])
    })
}
pub fn uuid(tag: &Tag) -> Option<[i32; 4]> {
    uuid_from_tag(tag.get("UUID"))
}
pub fn motion(tag: &Tag) -> Vec3 {
    read_doubles::<3>(tag.get("Motion")).map_or(Vec3::ZERO, |m| Vec3::new(m[0], m[1], m[2]))
}
/// A saved Motion as `Entity.load` takes it: a component faster than 10 blocks
/// a tick loads as 0.
pub fn loaded_motion(tag: &Tag) -> Vec3 {
    let m = motion(tag);
    let clamp = |v: f64| if v.abs() > 10.0 { 0.0 } else { v };
    Vec3::new(clamp(m.x), clamp(m.y), clamp(m.z))
}
pub fn rotation(tag: &Tag) -> [f32; 2] {
    read_doubles::<2>(tag.get("Rotation")).map_or([0.0; 2], |r| [r[0] as f32, r[1] as f32])
}
/// `Entity.baseTick` for a burning entity Horizon keeps as its loaded compound:
/// one tick off a positive `Fire`, which an unset or spent `Fire` leaves alone.
pub fn count_down_fire(tag: &mut Tag) {
    let Tag::Compound(c) = tag else { return };
    if let Some(ticks) = number(c.get(&"Fire".into())).map(|v| v as i16)
        && ticks > 0
    {
        c.insert("Fire".into(), Tag::Short(ticks - 1));
    }
}
/// `Entity.baseTick` of a fire-immune entity (an `AreaEffectCloud`): a positive
/// `Fire` is cleared on its tick, while a loaded 0 or negative value stays.
pub fn clear_positive_fire(tag: &mut Tag) {
    let Tag::Compound(c) = tag else { return };
    if let Some(ticks) = number(c.get(&"Fire".into())).map(|v| v as i16)
        && ticks > 0
    {
        c.insert("Fire".into(), Tag::Short(0));
    }
}
/// A saved Rotation as `Entity.load` takes it (`setRot`, read off vanilla 26.3 by the parity rows): the yaw `% 360`
/// (so 400 loads as 40 but 270 stays 270 and -190 stays -190: Java's remainder keeps the sign) and the pitch
/// `% 360` clamped to [-90, 90]; a component that is not finite is discarded (`setYRot`/`setXRot`) and loads as
/// 0. What a save writes is read with [`rotation`].
pub fn loaded_rotation(tag: &Tag) -> [f32; 2] {
    let [yaw, pitch] = rotation(tag);
    [
        if yaw.is_finite() { yaw % 360.0 } else { 0.0 },
        if pitch.is_finite() {
            (pitch % 360.0).clamp(-90.0, 90.0)
        } else {
            0.0
        },
    ]
}
/// `TagValueInput.getBooleanOr`: a numeric tag's byteValue is nonzero, with a false default.
pub fn flag(tag: &Tag, key: &str) -> bool {
    flag_or(tag, key, false)
}
/// `getBooleanOr(key, default)`: any numeric tag's nonzero test, else `default`.
pub fn flag_or(tag: &Tag, key: &str, default: bool) -> bool {
    // NBT floats floor before int narrowing; the final byte keeps the low bits.
    tag.get(key)
        .and_then(int_value)
        .map_or(default, |value| value as i8 != 0)
}

/// `Entity.saveWithoutId`: write the movement flags and the flags retained
/// from `Entity.load` as booleans. Only OnGround and Invulnerable write false.
pub fn write_saved_flags(tag: &mut Tag, on_ground: bool, no_gravity: bool) {
    let invulnerable = flag(tag, "Invulnerable");
    let optional = [
        ("NoGravity", no_gravity),
        ("CustomNameVisible", flag(tag, "CustomNameVisible")),
        ("Silent", flag(tag, "Silent")),
        ("Glowing", flag(tag, "Glowing")),
        ("HasVisualFire", flag(tag, "HasVisualFire")),
    ];
    let Tag::Compound(map) = tag else {
        return;
    };
    map.insert("OnGround".into(), Tag::Byte(i8::from(on_ground)));
    map.insert("Invulnerable".into(), Tag::Byte(i8::from(invulnerable)));
    for (key, value) in optional {
        let key = key.into();
        if value {
            map.insert(key, Tag::Byte(1));
        } else {
            map.remove(&key);
        }
    }
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
            // Entity.saveWithoutId writes Tags only for a nonempty live set.
            if matches!(map.get(&"Tags".into()), Some(Tag::List(tags)) if tags.is_empty()) {
                map.remove(&"Tags".into());
            }
            for (key, value) in [
                // Entity.remainingFireTicks starts at 0 (-1 was the old default).
                ("Fire", Tag::Short(0)),
                ("Air", Tag::Short(300)),
                ("Invulnerable", Tag::Byte(0)),
                ("PortalCooldown", Tag::Int(0)),
            ] {
                map.entry(key.into()).or_insert(value);
            }
        }
        let no_gravity = flag(&tag, "NoGravity");
        write_saved_flags(&mut tag, self.on_ground, no_gravity);
        tag
    }
    /// Preserve Entity data without flags owned by Mob's save writer.
    /// Only the dedicated boat, item and armor-stand writers use this path.
    fn non_mob_tag(&self, raw: Option<&Tag>) -> Tag {
        let mut tag = self.tag(raw);
        if let Tag::Compound(map) = &mut tag {
            map.remove(&"NoAI".into());
            map.remove(&"PersistenceRequired".into());
        }
        tag
    }
}

/// `Entity.saveWithoutId`'s common fields for entities Horizon keeps as raw
/// compounds: each one is inserted only when the loaded compound has no value
/// for it, so explicit values survive.
pub fn fill_common_defaults(tag: &mut Tag) {
    let Tag::Compound(map) = tag else {
        return;
    };
    for (key, value) in [
        ("Motion", doubles(&[0.0, 0.0, 0.0])),
        ("OnGround", Tag::Byte(0)),
        ("fall_distance", Tag::Double(0.0f64.to_bits())),
        ("Air", Tag::Short(300)),
        ("Invulnerable", Tag::Byte(0)),
        ("PortalCooldown", Tag::Int(0)),
    ] {
        map.entry(key.into()).or_insert(value);
    }
}

/// The mobs that save `Age` and `AgeLocked`.
fn saves_age(s: crate::mobs::Species) -> bool {
    use crate::mobs::{Species, finalize::ageable, species::can_be_a_baby};
    s == Species::Tadpole || (ageable(s) && can_be_a_baby(s))
}
/// `LivingEntity` + `Mob` (+ `AgeableMob`, creeper and skeleton specifics).
/// The mobs whose class extends vanilla's `Animal` (and so save `InLove`):
/// not allays, villagers, golems, dolphins or fish.
fn is_animal(id: &str) -> bool {
    matches!(
        id.strip_prefix("minecraft:").unwrap_or(id),
        "pig"
            | "cow"
            | "mooshroom"
            | "sheep"
            | "chicken"
            | "rabbit"
            | "horse"
            | "donkey"
            | "mule"
            | "zombie_horse"
            | "skeleton_horse"
            | "llama"
            | "trader_llama"
            | "goat"
            | "panda"
            | "polar_bear"
            | "fox"
            | "wolf"
            | "cat"
            | "ocelot"
            | "parrot"
            | "bee"
            | "turtle"
            | "armadillo"
            | "frog"
            | "axolotl"
            | "hoglin"
            | "strider"
            | "camel"
            | "camel_husk"
            | "sniffer"
            | "happy_ghast"
            | "nautilus"
            | "zombie_nautilus"
    )
}
use crate::mobs::persistent_anger;
/// The single player's UUID (`offline_uuid("Player")`): the entity the
/// neutral mobs are angry at.
pub(crate) fn player_uuid() -> [i32; 4] {
    let u = horizon_protocol::crypto::offline_uuid("Player");
    [
        (u >> 96) as u32 as i32,
        (u >> 64) as u32 as i32,
        (u >> 32) as u32 as i32,
        u as u32 as i32,
    ]
}
/// The UUID a save writes for a reference to `who`. A mob is only an id here:
/// its UUID lives in the game layer (`EntityMemory.uuids`), which is what
/// `uuid_of` answers from. When it answers `None` (the mob is no longer in the
/// level) nothing is written, rather than a made-up UUID that would name nothing
/// when the save is read back. The reference comes back with [`who_by_uuid`].
///
/// `mob_to_nbt`'s `uuid_of` is this for a field of a mob's own compound; the
/// game layer's `Game::save_who_uuid` is the same rule for a field written
/// outside one (a bullet's `Target`), answered from `EntityMemory::uuid_of`.
pub fn who_uuid(who: Who, uuid_of: &mut impl FnMut(u32) -> Option<[i32; 4]>) -> Option<[i32; 4]> {
    match who {
        Who::Player => Some(player_uuid()),
        Who::Mob(id) => uuid_of(id),
    }
}
/// Replace every reference to the player `old` in a saved entity with `new`. Older
/// Horizon worlds named their single player by the offline UUID (`Owner` on tamed
/// animals and horses, a projectile's `Owner` and an item's `Thrower`, a fox's
/// `Trusted`, `angry_at`, a leash's `UUID`); when such a world opens under the
/// account (`lan::playerdata::migrate_owner`), a chunk's entities are rewritten as
/// they load, and their next save keeps the account's UUID. A reference is the
/// player's four ints, wherever the entity keeps it; an entity's own `UUID` never
/// names the player.
pub fn migrate_player_uuid(tag: &mut Tag, old: [i32; 4], new: [i32; 4]) {
    if old == new {
        return;
    }
    match tag {
        Tag::IntArray(uuid) if uuid.as_slice() == old.as_slice() => uuid.copy_from_slice(&new),
        Tag::List(list) => {
            for tag in list {
                migrate_player_uuid(tag, old, new);
            }
        }
        Tag::Compound(map) => {
            for tag in map.values_mut() {
                migrate_player_uuid(tag, old, new);
            }
        }
        _ => {}
    }
}
/// `AngerManagement.SUSPECT_ANGER_PAIR`: one suspect's UUID and how angry the
/// warden is at it.
fn suspect_anger(uuid: [i32; 4], anger: i32) -> Tag {
    Tag::compound([
        ("uuid", Tag::IntArray(uuid.to_vec())),
        ("anger", Tag::Int(anger)),
    ])
}
/// `AngerManagement.codec`: `SUSPECT_ANGER_PAIR.listOf()` under `suspects`, in
/// the order `createUuidAngerPairs` builds it - the suspects that are in the
/// level, by the UUID each entity has, then the ones that are not, which are
/// already UUIDs. Nothing else is stored: the highest anger, the conversion
/// delay and the `NeutralMob` anger times are runtime only. An anger with no
/// suspect at all writes no `suspects` key, `lenientOptionalFieldOf`'s default.
fn warden_anger_to_nbt(
    anger: &crate::mobs::warden::AngerManagement,
    uuid_of: &mut impl FnMut(u32) -> Option<[i32; 4]>,
) -> Tag {
    let loaded = anger.suspects().iter().filter_map(|who| {
        let uuid = who_uuid(*who, uuid_of)?;
        Some(suspect_anger(uuid, anger.anger_of(*who)))
    });
    let away = anger
        .unloaded()
        .map(|(uuid, anger)| suspect_anger(uuid, anger));
    let pairs: Vec<Tag> = loaded.chain(away).collect();
    if pairs.is_empty() {
        Tag::compound(Vec::<(&str, Tag)>::new())
    } else {
        Tag::compound([("suspects", Tag::List(pairs))])
    }
}
/// [`warden_anger_to_nbt`] read back: every pair goes into the UUID map, where
/// the anger ticks resolve the ones whose entity is in the level again. A pair
/// with no four-int UUID or a negative anger drops the whole list, as
/// `lenientOptionalFieldOf` does with a value that does not decode.
fn warden_anger_from_nbt(tag: &Tag) -> Vec<([i32; 4], i32)> {
    let Some(Tag::List(pairs)) = tag.get("suspects") else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(pairs.len());
    for pair in pairs {
        let (Some(uuid), Some(anger)) =
            (uuid_from_tag(pair.get("uuid")), number(pair.get("anger")))
        else {
            return Vec::new();
        };
        let Ok(anger) = i32::try_from(anger) else {
            return Vec::new();
        };
        if anger < 0 {
            return Vec::new();
        }
        out.push((uuid, anger));
    }
    out
}
/// `ResolvableProfile.CODEC`: the name, id and properties of a profile, then the
/// skin's texture ids and model (`PlayerSkin.Patch`: "texture", "cape", "elytra", "model").
fn profile_nbt(p: &horizon_protocol::entity_data::Profile) -> Tag {
    let mut entries: Vec<(&'static str, Tag)> = Vec::new();
    if let Some(name) = &p.name {
        entries.push(("name", string(name)));
    }
    if let Some(id) = p.uuid {
        entries.push((
            "id",
            Tag::IntArray(vec![
                (id >> 96) as i32,
                (id >> 64) as i32,
                (id >> 32) as i32,
                id as i32,
            ]),
        ));
    }
    if !p.properties.is_empty() {
        let properties = p
            .properties
            .iter()
            .map(|property| {
                let mut entries: Vec<(&'static str, Tag)> = vec![
                    ("name", string(&property.name)),
                    ("value", string(&property.value)),
                ];
                if let Some(signature) = &property.signature {
                    entries.push(("signature", string(signature)));
                }
                Tag::compound(entries)
            })
            .collect();
        entries.push(("properties", Tag::List(properties)));
    }
    for (key, texture) in [
        ("texture", &p.body),
        ("cape", &p.cape),
        ("elytra", &p.elytra),
    ] {
        if let Some(texture) = texture {
            entries.push((key, string(texture)));
        }
    }
    if let Some(slim) = p.slim {
        entries.push(("model", string(if slim { "slim" } else { "wide" })));
    }
    Tag::compound(entries)
}
/// `ResolvableProfile.CODEC` read back: the compound `profile_nbt` writes, or a
/// bare player name (`Codec.withAlternative(FULL_CODEC, PLAYER_NAME, ...)`).
fn profile_from_nbt(tag: &Tag) -> Option<horizon_protocol::entity_data::Profile> {
    if let Tag::String(name) = tag {
        return Some(horizon_protocol::entity_data::Profile {
            name: Some(name.to_string_lossy()),
            uuid: None,
            properties: Vec::new(),
            body: None,
            cape: None,
            elytra: None,
            slim: None,
        });
    }
    let property = |name: String, value: &Tag, signature: Option<String>| match value {
        Tag::String(v) => Some(horizon_protocol::packets::login::Property {
            name,
            value: v.to_string_lossy(),
            signature,
        }),
        _ => None,
    };
    let properties: Vec<horizon_protocol::packets::login::Property> = match tag.get("properties") {
        // `ExtraCodecs.PROPERTY_MAP`: a list of properties ...
        Some(Tag::List(list)) => list
            .iter()
            .filter_map(|p| {
                let name = match p.get("name")? {
                    Tag::String(s) => s.to_string_lossy(),
                    _ => return None,
                };
                let signature = match p.get("signature") {
                    Some(Tag::String(s)) => Some(s.to_string_lossy()),
                    _ => None,
                };
                property(name, p.get("value")?, signature)
            })
            .collect(),
        // ... or a map from a name to its values.
        Some(Tag::Compound(map)) => map
            .iter()
            .flat_map(|(name, values)| match values {
                Tag::List(values) => values
                    .iter()
                    .filter_map(|v| property(name.to_string_lossy(), v, None))
                    .collect::<Vec<_>>(),
                _ => Vec::new(),
            })
            .collect(),
        _ => Vec::new(),
    };
    let text = |key: &str| match tag.get(key) {
        Some(Tag::String(s)) => Some(s.to_string_lossy()),
        _ => None,
    };
    Some(horizon_protocol::entity_data::Profile {
        name: text("name"),
        uuid: uuid_from_tag(tag.get("id")).map(|a| {
            a.iter()
                .fold(0u128, |u, &x| (u << 32) | u128::from(x as u32))
        }),
        properties,
        body: text("texture"),
        cape: text("cape"),
        elytra: text("elytra"),
        slim: text("model").map(|model| model == "slim"),
    })
}
/// `ConversionTracker.addAdditionalSaveData`: vanilla's conversion time, -1
/// while the mob is not converting. `counter2` is that time plus one while it
/// converts and 0 when it does not (see `mobs/undead.rs`).
fn conversion_time(m: &Mob) -> i32 {
    if m.extra.counter2 > 0 {
        m.extra.counter2 - 1
    } else {
        -1
    }
}
/// `BlockState.CODEC`: the block's name when the state is that block's
/// default, `{id, properties}` for any other state.
pub fn state_to_nbt(registry: &BlockRegistry, state: BlockStateId) -> Tag {
    match registry.state(state) {
        Some(s) if registry.default_state(&s.name) == Some(state) => string(&s.name),
        Some(s) => s.to_nbt(),
        None => string("minecraft:air"),
    }
}
/// `BlockState.CODEC` read back: a block name or `{id, properties}` (or the
/// legacy `{Name, Properties}`). A name the registry knows only as a block
/// reads as its default state, and one it does not know at all as `None`.
pub fn state_from_nbt(registry: &BlockRegistry, tag: &Tag) -> Option<BlockStateId> {
    let state = match tag.get("Name") {
        Some(Tag::String(name)) => BlockState {
            name: name.to_string_lossy(),
            properties: match tag.get("Properties") {
                Some(Tag::Compound(p)) => p
                    .iter()
                    .filter_map(|(k, v)| match v {
                        Tag::String(v) => Some((k.to_string_lossy(), v.to_string_lossy())),
                        _ => None,
                    })
                    .collect(),
                _ => Default::default(),
            },
        },
        _ => BlockState::from_nbt(tag).ok()?,
    };
    registry
        .id(&state)
        .or_else(|| registry.default_state(&state.name))
}
/// `BlockState.isAir`: air, cave air and void air.
fn is_air(registry: &BlockRegistry, state: BlockStateId) -> bool {
    registry.state(state).is_some_and(|s| {
        matches!(
            s.name.as_str(),
            "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air"
        )
    })
}
/// The mob's entity compound. `registry` is what writes a field holding a block
/// state (an enderman's `carriedBlockState`): without one, a caller that has no
/// resources (unit tests, structure templates) keeps whatever the loaded
/// compound had for it.
///
/// `uuid_of` is how a field that names another entity by UUID (a mob's
/// `angry_at`) is written: it answers the UUID of a mob by its id, from the game
/// layer's map, and `None` for one the level no longer has. A caller with no
/// such map passes `|_| None` and writes no reference.
pub fn mob_to_nbt(
    m: &Mob,
    uuid: [i32; 4],
    raw: Option<&Tag>,
    registry: Option<&BlockRegistry>,
    mut uuid_of: impl FnMut(u32) -> Option<[i32; 4]>,
) -> Tag {
    use crate::mobs::Species as S;
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
        (
            "Fire",
            Tag::Short(m.fire_ticks.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16),
        ),
        ("Air", Tag::Short(m.air as i16)),
        ("AbsorptionAmount", Tag::Float(m.absorption.to_bits())),
        // Mob.addAdditionalSaveData writes the current virtual getter, not raw input.
        ("CanPickUpLoot", Tag::Byte(i8::from(m.can_pick_up_loot()))),
        ("PersistenceRequired", Tag::Byte(i8::from(m.persistent))),
    ] {
        put(&mut tag, key, value);
    }
    mob_death::write(m, &mut tag);
    ai_pause::write_ai_paused(m.ai.ai_paused, &mut tag);
    // Mob.addAdditionalSaveData: hasHome means radius != -1, including -2.
    // Species writers below retain their own home_pos override.
    let Tag::Compound(map) = &mut tag else {
        unreachable!()
    };
    if let Some((pos, radius)) = m.ai.home.filter(|(_, radius)| *radius != -1) {
        map.insert("home_radius".into(), Tag::Int(radius));
        map.insert("home_pos".into(), Tag::IntArray(pos.to_vec()));
    } else {
        map.remove(&"home_radius".into());
        map.remove(&"home_pos".into());
    }
    // AbstractFish/Axolotl.addAdditionalSaveData always write the flag.
    // Tadpole.fromBucket is always true, and its setter ignores loaded input.
    let from_bucket = match m.species() {
        S::Tadpole => Some(true),
        S::Cod | S::Salmon | S::Pufferfish | S::TropicalFish | S::Axolotl => {
            Some(m.extra.from_bucket)
        }
        _ => None,
    };
    if let Some(value) = from_bucket {
        put(&mut tag, "FromBucket", Tag::Byte(i8::from(value)));
    }
    // Mob.addAdditionalSaveData: the death loot table, and its seed when set.
    if let Some(table) = &m.extra.death_loot_table {
        put(&mut tag, "DeathLootTable", string(table));
    }
    if m.extra.death_table_seed != 0 {
        put(
            &mut tag,
            "DeathLootTableSeed",
            Tag::Long(m.extra.death_table_seed),
        );
    }
    // `NeutralMob.addPersistentAngerSaveData`: the anger ends at an
    // absolute game time, and the target is the entity the mob is angry at.
    if persistent_anger(m.species()) {
        put(
            &mut tag,
            "anger_end_time",
            // The absolute value as it is, also zero or one already past.
            Tag::Long(m.extra.anger_end),
        );
        // `angry_at` only while there is an anger target: the UUID of the one
        // the level has (`uuid_of`). A mob target read from the save that is
        // not in the world (`Who::Mob(0)`) keeps the loaded UUID.
        match m.extra.angry_at {
            Some(crate::mobs::Who::Mob(0)) => {}
            target => match target.and_then(|who| who_uuid(who, &mut uuid_of)) {
                Some(uuid) => put(&mut tag, "angry_at", Tag::IntArray(uuid.to_vec())),
                None => {
                    if let Tag::Compound(map) = &mut tag {
                        map.remove(&"angry_at".into());
                    }
                }
            },
        }
    }
    // `Mannequin.addAdditionalSaveData`: the profile whose skin it wears, whether it
    // is `immovable`, and its description (the default is not written; none is `hide_description`).
    if m.species() == crate::mobs::Species::Mannequin {
        // The empty static profile (`DEFAULT_PROFILE`) when it has none.
        let profile = m
            .profile
            .as_ref()
            .map_or_else(|| Tag::Compound(Compound::new()), profile_nbt);
        put(&mut tag, "profile", profile);
        // `LAYERS_CODEC`: the hidden layers, in `PlayerModelPart` order.
        let hidden = MODEL_PARTS
            .iter()
            .enumerate()
            .filter(|(bit, _)| m.extra.hidden_model_parts & (1 << bit) != 0)
            .map(|(_, part)| string(part))
            .collect();
        put(&mut tag, "hidden_layers", Tag::List(hidden));
        let hand = if m.extra.main_hand_left {
            "left"
        } else {
            "right"
        };
        put(&mut tag, "main_hand", string(hand));
        let pose = match m.extra.avatar_pose {
            crate::pose::Pose::Crouching => "crouching",
            crate::pose::Pose::Swimming => "swimming",
            crate::pose::Pose::FallFlying => "fall_flying",
            crate::pose::Pose::Sleeping => "sleeping",
            _ => "standing",
        };
        put(&mut tag, "pose", string(pose));
        put(
            &mut tag,
            "immovable",
            Tag::Byte(i8::from(m.extra.immovable)),
        );
        match &m.extra.description {
            crate::mobs::MannequinDescription::Default => {}
            crate::mobs::MannequinDescription::Hidden => {
                put(&mut tag, "hide_description", Tag::Byte(1));
            }
            crate::mobs::MannequinDescription::Custom(component) => {
                put(&mut tag, "description", component.clone());
            }
        }
    }
    // `Entity.saveWithoutId` writes the flag only when set.
    if m.name_visible {
        put(&mut tag, "CustomNameVisible", Tag::Byte(1));
    } else if let Tag::Compound(map) = &mut tag {
        map.remove(&"CustomNameVisible".into());
    }
    // A styled name from the save stays unless the name changed.
    if m.custom_name_changed(tag.get("CustomName"))
        && let Some(name) = m.custom_name_tag()
    {
        put(&mut tag, "CustomName", name);
    }
    let Tag::Compound(map) = &mut tag else {
        return tag;
    };
    for (key, on) in [
        ("NoAI", m.no_ai),
        ("Silent", m.silent),
        ("NoGravity", m.no_gravity),
        ("Glowing", m.glowing),
    ] {
        if on {
            map.insert(key.into(), Tag::Byte(1));
        } else {
            map.remove(&horizon_core::nbt::JavaString::from(key));
        }
    }
    // LivingEntity.addAdditionalSaveData: `active_effects` only when there are
    // any (MobEffectInstance.CODEC), including their modeled weaker instances.
    if m.extra.effects.is_empty() {
        map.remove(&horizon_core::nbt::JavaString::from("active_effects"));
    } else {
        let effects = m.extra.effects.iter().map(|effect| {
            let show_icon = effect.show_icon;
            let (effect, duration, amplifier) = (effect.id, effect.duration, effect.amplifier);
            let shown = !m.extra.hidden_effects.contains(&effect);
            let ambient = m.extra.ambient_effects.contains(&effect);
            let mut e = vec![("id", string(crate::brewing::namespaced(effect)))];
            // The codec leaves out an amplifier of 0 (its default).
            if amplifier != 0 {
                e.push(("amplifier", Tag::Byte(amplifier.clamp(0, 255) as u8 as i8)));
            }
            e.push(("duration", Tag::Int(duration)));
            if ambient {
                e.push(("ambient", Tag::Byte(1)));
            }
            if !shown {
                e.push(("show_particles", Tag::Byte(0)));
            }
            e.push(("show_icon", Tag::Byte(i8::from(show_icon))));
            if let Some(hidden) = m.extra.effect_chains.iter().find(|e| e.id == effect) {
                e.push(("hidden_effect", hidden_effect_nbt(hidden)));
            }
            Tag::compound(e)
        });
        map.insert("active_effects".into(), Tag::List(effects.collect()));
    }
    map.insert(
        "Invulnerable".into(),
        Tag::Byte(i8::from(m.invulnerable_flag)),
    );
    // Entity.saveWithoutId: `invulnerable_time` — the entity-level window
    // (arismoko/horizon#916), not the hurt cooldown (LivingEntity saves none) — only while
    // it runs.
    if m.invulnerable_time > 0 {
        map.insert("invulnerable_time".into(), Tag::Int(m.invulnerable_time));
    } else {
        map.remove(&horizon_core::nbt::JavaString::from("invulnerable_time"));
    }
    map.insert(
        "LeftHanded".into(),
        Tag::Byte(i8::from(m.extra.left_handed)),
    );
    // Keys of older formats that structure templates still carry.
    for key in [
        "HandItems",
        "ArmorItems",
        "HandDropChances",
        "ArmorDropChances",
        "Dimension",
        "HurtByTimestamp",
        "Leashed",
    ] {
        map.remove(&horizon_core::nbt::JavaString::from(key));
    }
    if m.ticks_frozen > 0 {
        map.insert("TicksFrozen".into(), Tag::Int(m.ticks_frozen));
    } else {
        map.remove(&horizon_core::nbt::JavaString::from("TicksFrozen"));
    }
    // LivingEntity.addAdditionalSaveData: `putBoolean(FallFlying, isFallFlying())`.
    map.insert(
        "FallFlying".into(),
        Tag::Byte(i8::from(m.extra.fall_flying)),
    );
    for (key, value) in [
        ("AbsorptionAmount", Tag::Float(0)),
        (
            "Brain",
            Tag::compound([("memories", Tag::Compound(Compound::new()))]),
        ),
    ] {
        map.entry(key.into()).or_insert(value);
    }
    map.insert(
        "current_impulse_context_reset_grace_time".into(),
        Tag::Int(m.impulse_grace_time),
    );
    // What the mob's class saves: a content mod's living type saves as its
    // template does (the `id` written above stays its own).
    let id = m.kind.class_id();
    // AgeableMob.addAdditionalSaveData, only if canBeABaby (not a frog or a
    // parrot); a tadpole saves its own age.
    let species = m.species();
    if saves_age(species) {
        map.insert("Age".into(), Tag::Int(m.extra.age));
        map.insert("AgeLocked".into(), Tag::Byte(i8::from(m.extra.age_locked)));
        if species != crate::mobs::Species::Tadpole {
            map.insert("ForcedAge".into(), Tag::Int(m.extra.forced_age));
        }
    } else if crate::mobs::finalize::ageable(species) {
        // AgeableMob.addAdditionalSaveData writes nothing when canBeABaby is
        // false — the frog, the parrot, the zombie nautilus, the camel husk,
        // the slime and the magma cube: drop the ignored values a raw input
        // compound carried rather than writing them back out.
        for key in ["Age", "ForcedAge", "AgeLocked"] {
            map.remove(&key.into());
        }
    }
    // Animal.addAdditionalSaveData: the love ticks left, whatever the save had.
    if is_animal(id) {
        map.insert("InLove".into(), Tag::Int(m.extra.in_love));
        // Animal.addAdditionalSaveData: the player who fed it.
        if let Some(cause) = m.extra.love_cause {
            map.insert("LoveCause".into(), Tag::IntArray(cause.to_vec()));
        } else {
            map.remove(&"LoveCause".into());
        }
    }
    // InventoryCarrier.writeInventoryToTag: the occupied stacks, even when empty.
    if matches!(id, "minecraft:pillager" | "minecraft:wandering_trader") {
        map.insert(
            "Inventory".into(),
            Tag::List(
                m.extra
                    .inventory
                    .iter()
                    .map(crate::save::item_to_nbt)
                    .collect(),
            ),
        );
    }
    let species = m.species();
    // `VariantUtils.writeVariant`: only the species that store the name. A
    // mooshroom saves `Type` (below), a fox `Type`, a horse and a llama a
    // packed `Variant` int, so those report the component but save no name.
    if crate::mobs::finalize::saves_variant_name(species)
        && let Some((_, variant)) =
            crate::mobs::finalize::species_variant_component(species, m.extra.variant)
    {
        map.insert("variant".into(), string(&variant));
    }
    if species == crate::mobs::Species::Chicken {
        map.insert("EggLayTime".into(), Tag::Int(m.extra.egg_time));
    }
    let sounds = crate::mobs::finalize::sound_variants(species);
    if !sounds.is_empty() {
        let sound = m.extra.sound_variant.and_then(|i| sounds.get(i as usize));
        let sound = sound.copied().unwrap_or("classic");
        map.insert(
            "sound_variant".into(),
            string(&format!("minecraft:{sound}")),
        );
    }
    match id {
        // Fox.addAdditionalSaveData; Horizon models only the type, the
        // flags and no trusted players yet.
        "minecraft:fox" => {
            let kind = if m.extra.variant == 1 { "snow" } else { "red" };
            map.insert("Type".into(), string(kind));
            // The trusted UUIDs (`Fox.TRUSTED_LIST_CODEC`: a list of int arrays).
            map.insert(
                "Trusted".into(),
                Tag::List(
                    crate::mobs::fox::trusted_entities(m)
                        .map(|uuid| Tag::IntArray(uuid.to_vec()))
                        .collect(),
                ),
            );
            for (key, bit) in [
                ("Sleeping", crate::mobs::fox::SLEEPING),
                ("Sitting", crate::mobs::fox::SITTING),
                ("Crouching", crate::mobs::fox::CROUCHING),
            ] {
                map.insert(
                    key.into(),
                    Tag::Byte(i8::from(crate::mobs::fox::flag(m, bit))),
                );
            }
        }
        // MushroomCow.addAdditionalSaveData: the Variant codec's name and the
        // stew effects, written only while it holds any (storeNullable).
        "minecraft:mooshroom" => {
            let kind = if m.extra.variant == 1 { "brown" } else { "red" };
            map.insert("Type".into(), string(kind));
            match &m.extra.stew_effects {
                Some(effects) => {
                    let list = effects.iter().map(|(id, duration)| {
                        let mut entry = vec![("id", string(id))];
                        // SuspiciousStewEffects.Entry leaves out the default.
                        if *duration != 160 {
                            entry.push(("duration", Tag::Int(*duration)));
                        }
                        Tag::compound(entry)
                    });
                    map.insert("stew_effects".into(), Tag::List(list.collect()));
                }
                None => {
                    map.remove(&horizon_core::nbt::JavaString::from("stew_effects"));
                }
            }
        }
        // Rabbit.addAdditionalSaveData (the legacy id; 99 is the killer bunny).
        "minecraft:rabbit" => {
            map.insert("RabbitType".into(), Tag::Int(i32::from(m.extra.variant)));
            map.insert(
                "MoreCarrotTicks".into(),
                Tag::Int(m.extra.more_carrot_ticks),
            );
        }
        "minecraft:goat" => crate::mobs::goat::write_nbt(m, map),
        "minecraft:creeper" => {
            map.insert("Fuse".into(), Tag::Short(m.max_swell as i16));
            map.insert(
                "ExplosionRadius".into(),
                Tag::Byte(m.extra.explosion_radius),
            );
            map.insert("powered".into(), Tag::Byte(i8::from(m.extra.charged)));
            map.insert("ignited".into(), Tag::Byte(i8::from(m.extra.flag)));
        }
        // `Ocelot.addAdditionalSaveData`.
        "minecraft:ocelot" => {
            map.insert("Trusting".into(), Tag::Byte(i8::from(m.extra.trusting)));
        }
        "minecraft:cat" => {
            if let Some(owner) = m.extra.owner {
                map.insert("Owner".into(), Tag::IntArray(owner.to_vec()));
            } else {
                map.remove(&"Owner".into());
            }
            map.insert("Sitting".into(), Tag::Byte(i8::from(m.extra.sitting)));
            map.insert("CollarColor".into(), Tag::Byte(m.extra.color as i8));
        }
        "minecraft:wolf" => {
            if let Some(owner) = m.extra.owner {
                map.insert("Owner".into(), Tag::IntArray(owner.to_vec()));
            } else {
                map.remove(&"Owner".into());
            }
            map.insert("Sitting".into(), Tag::Byte(i8::from(m.extra.sitting)));
            map.insert("CollarColor".into(), Tag::Byte(m.extra.color as i8));
        }
        "minecraft:panda" => {
            use crate::mobs::panda::GENES;
            for (key, gene) in [
                ("MainGene", m.extra.variant),
                ("HiddenGene", m.extra.variant2),
            ] {
                map.insert(
                    key.into(),
                    string(GENES[usize::from(gene).min(GENES.len() - 1)]),
                );
            }
        }
        "minecraft:piglin" | "minecraft:piglin_brute" | "minecraft:hoglin" => {
            if id == "minecraft:piglin_brute" {
                map.remove(&"CannotHunt".into());
            } else if id == "minecraft:piglin" {
                let hunt = m.extra.piglin.cannot_hunt as i8;
                map.insert("CannotHunt".into(), Tag::Byte(hunt));
            }
            map.insert(
                "IsImmuneToZombification".into(),
                Tag::Byte(i8::from(m.extra.flag2)),
            );
            map.insert("TimeInOverworld".into(), Tag::Int(m.extra.counter2));
            if id == "minecraft:hoglin" {
                map.insert(
                    "CannotBeHunted".into(),
                    Tag::Byte(m.extra.cannot_be_hunted as i8),
                );
            }
            if id == "minecraft:piglin" {
                crate::mobs::piglin::write_guarded_anger(m, map);
                // InventoryCarrier.writeInventoryToTag.
                map.insert(
                    "Inventory".into(),
                    Tag::List(
                        m.extra
                            .inventory
                            .iter()
                            .map(crate::save::item_to_nbt)
                            .collect(),
                    ),
                );
            }
        }
        "minecraft:horse"
        | "minecraft:donkey"
        | "minecraft:mule"
        | "minecraft:skeleton_horse"
        | "minecraft:zombie_horse"
        | "minecraft:llama"
        | "minecraft:trader_llama" => {
            map.insert("Tame".into(), Tag::Byte(i8::from(m.extra.tame)));
            map.insert("Temper".into(), Tag::Int(m.extra.equine_temper));
            map.insert("Bred".into(), Tag::Byte(i8::from(m.extra.bred)));
            // AbstractHorse.addAdditionalSaveData; the eating haystack is not modelled yet.
            map.entry("EatingHaystack".into()).or_insert(Tag::Byte(0));
            if let Some(owner) = m.extra.owner {
                map.insert("Owner".into(), Tag::IntArray(owner.to_vec()));
            }
            if id == "minecraft:horse" {
                let loaded = map.get(&"Variant".into()).and_then(int_value);
                let look = (m.extra.variant, m.extra.variant2);
                let v = crate::mobs::equine::saved_variant(loaded, look.0, look.1);
                map.insert("Variant".into(), Tag::Int(v));
            }
            let llama = crate::mobs::llama::is_llama(m.species());
            if llama {
                crate::mobs::llama::to_nbt(m, map);
            }
            // Only llamas, donkeys and mules carry chests.
            if llama || matches!(id, "minecraft:donkey" | "minecraft:mule") {
                map.insert("ChestedHorse".into(), Tag::Byte(i8::from(m.extra.chested)));
            }
            if m.extra.chested && !m.extra.items.is_empty() {
                map.insert("Items".into(), crate::save::encode_items(&m.extra.items));
            } else {
                map.remove(&"Items".into());
            }
            // The `equipment` compound is written below, for every mob.
            // AttributeMap.pack writes the instances that exist: the ones the
            // entity was loaded with. When the tag had no attributes list,
            // instances are not synthesized (SummonCommand without finalizeSpawn
            // saves attributes: []).
            if let Some(Tag::List(list)) = map.get(&"attributes".into()) {
                let modelled = [
                    ("minecraft:max_health", f64::from(m.attrs.max_health)),
                    ("minecraft:movement_speed", m.attrs.speed),
                    ("minecraft:jump_strength", m.extra.vector[0]),
                ];
                let attributes = list
                    .iter()
                    .map(|a| {
                        let id = text(a.get("id"));
                        let value = modelled.iter().find(|(m, _)| Some(*m) == id.as_deref());
                        match (a, value) {
                            (Tag::Compound(c), Some((_, v))) => {
                                let mut c = c.clone();
                                c.insert("base".into(), Tag::Double(v.to_bits()));
                                Tag::Compound(c)
                            }
                            _ => a.clone(),
                        }
                    })
                    .collect();
                map.insert("attributes".into(), Tag::List(attributes));
            }
        }
        "minecraft:bat" => {
            map.insert("BatFlags".into(), Tag::Byte(i8::from(m.extra.flag)));
        }
        "minecraft:turtle" => {
            map.insert("has_egg".into(), Tag::Byte(i8::from(m.extra.flag)));
            if let Some(home) = m.extra.home {
                map.insert("home_pos".into(), Tag::IntArray(home.to_vec()));
            }
        }
        "minecraft:creaking" => {
            // home_pos: the heart it is bound to.
            if let Some(home) = m.extra.home {
                map.insert("home_pos".into(), Tag::IntArray(home.to_vec()));
            } else {
                map.remove(&"home_pos".into());
            }
        }
        "minecraft:zombie_villager" => {
            map.insert("VillagerData".into(), villager_data(m));
            let xp = m.extra.trade.as_deref().map_or(0, |t| t.xp);
            map.insert("Xp".into(), Tag::Int(xp));
            put_offers(map, m);
            crate::mobs::zombie_villager::write_nbt(m, map);
        }
        "minecraft:villager" => {
            map.insert("VillagerData".into(), villager_data(m));
            // A loaded villager with VillagerData is finalized.
            map.insert("VillagerDataFinalized".into(), Tag::Byte(1));
            let t = m.extra.trade.as_deref().cloned().unwrap_or_default();
            map.insert("Xp".into(), Tag::Int(t.xp));
            map.insert("LastRestock".into(), Tag::Long(t.last_restock));
            map.insert("RestocksToday".into(), Tag::Int(t.restocks_today));
            put_offers(map, m);
            put_memories(map, m);
            crate::mobs::villager::write_nbt(m, map);
        }
        // Dolphin.addAdditionalSaveData: the fish it was fed, and how wet it is.
        "minecraft:dolphin" => {
            map.insert("GotFish".into(), Tag::Byte(i8::from(m.extra.flag)));
            map.insert("Moistness".into(), Tag::Int(m.extra.counter));
        }
        "minecraft:wandering_trader" => {
            put_offers(map, m);
            // WanderingTrader.addAdditionalSaveData: the visit's countdown and
            // the place it is walking to, a BlockPos codec (an int array).
            map.insert("DespawnDelay".into(), Tag::Int(m.extra.ints[5]));
            if let Some(p) = m.extra.point {
                // The trader keeps its target as a position; the tag is a
                // BlockPos, so only the block it is in is saved.
                map.insert(
                    "wander_target".into(),
                    Tag::IntArray(vec![
                        p[0].floor() as i32,
                        p[1].floor() as i32,
                        p[2].floor() as i32,
                    ]),
                );
            }
        }
        "minecraft:axolotl" => {
            write_axolotl_memories(m, map);
            map.insert(
                "Variant".into(),
                Tag::Int(i32::from(m.extra.variant.min(4))),
            );
        }
        "minecraft:parrot" => {
            map.insert(
                "Variant".into(),
                Tag::Int(i32::from(m.extra.variant.min(4))),
            );
            if let Some(owner) = m.extra.owner {
                map.insert("Owner".into(), Tag::IntArray(owner.to_vec()));
            } else {
                map.remove(&"Owner".into());
            }
            map.insert("Sitting".into(), Tag::Byte(i8::from(m.extra.sitting)));
        }
        "minecraft:salmon" => {
            let size = SALMON_SIZES[usize::from(m.extra.variant).min(2)];
            map.insert("type".into(), string(size));
        }
        // Pufferfish.addAdditionalSaveData: the puff state (0 small, 1 mid,
        // 2 full).
        "minecraft:pufferfish" => {
            map.insert("PuffState".into(), Tag::Int(m.extra.size));
        }
        // TropicalFish.addAdditionalSaveData: the packed variant.
        "minecraft:tropical_fish" => {
            let pattern = m.extra.variant.min(11);
            let packed = i32::from(pattern >= 6)
                | i32::from(pattern % 6) << 8
                | i32::from(m.extra.variant2 & 15) << 16
                | i32::from(m.extra.variant3 & 15) << 24;
            map.insert("Variant".into(), Tag::Int(packed));
        }
        "minecraft:allay" => {
            // InventoryCarrier: the fetched stack; the liked player memory.
            let inv = crate::mobs::allay::inventory(m)
                .map(item_to_nbt)
                .into_iter()
                .collect();
            map.insert("Inventory".into(), Tag::List(inv));
            if let Some(st) = &m.extra.allay {
                map.insert("listener".into(), st.vibration.to_nbt());
            }
            map.insert(
                "DuplicationCooldown".into(),
                Tag::Long(i64::from(m.extra.egg_time)),
            );
            let mut memories: Vec<(&str, Tag)> = Vec::new();
            if m.extra.tame {
                // `MemorySlot.visit`: the time to live only when it can expire.
                let mut liked = vec![(
                    "value",
                    Tag::IntArray(m.extra.owner.unwrap_or([0, 0, 0, 1]).to_vec()),
                )];
                if let Some(ttl) = m.extra.allay.as_ref().and_then(|st| st.liked_player_ttl) {
                    liked.push(("ttl", Tag::Long(ttl)));
                }
                memories.push(("minecraft:liked_player", Tag::compound(liked)));
            }
            // GlobalPos.CODEC, and the cooldown while it runs.
            if let Some(liked) = m.extra.allay.as_ref().and_then(|st| st.noteblock) {
                memories.push((
                    "minecraft:liked_noteblock",
                    Tag::compound([(
                        "value",
                        Tag::compound([
                            ("dimension", string(liked.dimension)),
                            ("pos", Tag::IntArray(liked.pos.to_vec())),
                        ]),
                    )]),
                ));
            }
            if m.extra.ints[2] > 0 {
                memories.push((
                    "minecraft:liked_noteblock_cooldown_ticks",
                    Tag::compound([("value", Tag::Int(m.extra.ints[2]))]),
                ));
            }
            // ITEM_PICKUP_COOLDOWN_TICKS (`Codec.INT`), which a throw sets to 60,
            // while the memory is there (its value 0 included).
            if let Some(ticks) = crate::mobs::allay::pickup_cooldown(m) {
                memories.push((
                    "minecraft:item_pickup_cooldown_ticks",
                    Tag::compound([("value", Tag::Int(ticks))]),
                ));
            }
            // Always written, so a forgotten note block does not come back
            // from the compound the allay was loaded with.
            map.insert(
                "Brain".into(),
                Tag::compound([("memories", Tag::compound(memories))]),
            );
        }
        "minecraft:camel" | "minecraft:camel_husk" => {
            // AbstractHorse.addAdditionalSaveData, including Camel.isTamed.
            map.insert("Tame".into(), Tag::Byte(i8::from(m.extra.tame)));
            map.insert("Temper".into(), Tag::Int(m.extra.equine_temper));
            map.insert("Bred".into(), Tag::Byte(i8::from(m.extra.bred)));
            map.insert("EatingHaystack".into(), Tag::Byte(i8::from(m.extra.flag)));
            // LastPoseTick: the pose change time, negative while sitting. The
            // game time is not known here; the world's save overwrites it
            // with the real one (`camel::synced_pose_tick`).
            map.insert(
                "LastPoseTick".into(),
                Tag::Long(crate::mobs::camel::synced_pose_tick(m, 0)),
            );
        }
        "minecraft:nautilus" | "minecraft:zombie_nautilus" => {
            crate::mobs::nautilus::to_nbt(m, map);
        }
        "minecraft:sulfur_cube" => crate::mobs::sulfur_cube::to_nbt(m, map),
        "minecraft:bee" => crate::mobs::bee::to_nbt(m, map, &mut uuid_of),
        "minecraft:sniffer" => crate::mobs::sniffer::to_nbt(m, map),
        "minecraft:wither" => {
            map.insert("Invul".into(), Tag::Int(m.extra.counter));
        }
        // IronGolem.addAdditionalSaveData.
        "minecraft:iron_golem" => {
            map.insert(
                "PlayerCreated".into(),
                Tag::Byte(i8::from(m.extra.player_created)),
            );
        }
        // SnowGolem.addAdditionalSaveData: whether it still wears its pumpkin.
        "minecraft:snow_golem" => {
            map.insert("Pumpkin".into(), Tag::Byte(i8::from(!m.extra.sheared)));
        }
        // `Warden.addAdditionalSaveData`: the anger - one `{uuid, anger}` pair
        // per suspect - and the vibration listener, and nothing else. A warden
        // is no `NeutralMob`, so there is no `angry_at` or `anger_end_time`: the
        // entity a suspect is is named by its UUID, not by a live id.
        "minecraft:warden" => {
            if let Some(st) = &m.extra.warden {
                map.insert("listener".into(), st.vibration.to_nbt());
                map.insert("anger".into(), warden_anger_to_nbt(&st.anger, &mut uuid_of));
            }
            write_warden_owned_memories(m, map);
        }
        // Armadillo.addAdditionalSaveData: the time left to the next scute, and
        // the state it is in, which is always written (`idle` when it is not
        // rolled up).
        "minecraft:armadillo" => {
            map.insert("scute_time".into(), Tag::Int(m.extra.egg_time));
            map.insert(
                "state".into(),
                string(crate::mobs::armadillo::state_name(
                    crate::mobs::armadillo::state(m),
                )),
            );
        }
        "minecraft:ravager" => {
            map.insert("AttackTick".into(), Tag::Int(m.extra.counter));
            map.insert("StunTick".into(), Tag::Int(m.extra.counter2));
            map.insert("RoarTick".into(), Tag::Int(m.extra.anim_ticks));
        }
        "minecraft:vex" => crate::mobs::vex::write_nbt(m, map, &mut uuid_of),
        "minecraft:endermite" => {
            map.insert("Lifetime".into(), Tag::Int(m.extra.counter));
        }
        // Enderman.addAdditionalSaveData: the block it carries, saved only
        // while it carries one.
        "minecraft:enderman" => match (m.extra.carried, registry) {
            (Some(state), Some(registry)) => {
                map.insert("carriedBlockState".into(), state_to_nbt(registry, state));
            }
            // Nothing carries a block state out without the registry, and a
            // caller without one keeps what the loaded compound had, so the
            // key is only dropped when a registry could have written it.
            (Some(_), None) | (None, None) => {}
            (None, Some(_)) => {
                map.remove(&"carriedBlockState".into());
            }
        },
        "minecraft:shulker" => {
            map.insert("AttachFace".into(), Tag::Byte(m.extra.variant3 as i8));
            map.insert("Peek".into(), Tag::Byte(m.extra.counter as i8));
            map.insert("Color".into(), Tag::Byte(m.extra.color as i8));
        }
        // HappyGhast.addAdditionalSaveData: the still timeout (`serverStillTimeout`,
        // kept in `counter`), which vanilla writes even at 0.
        "minecraft:happy_ghast" => {
            map.insert("still_timeout".into(), Tag::Int(m.extra.counter));
        }
        // GlowSquid.addAdditionalSaveData.
        "minecraft:glow_squid" => {
            map.insert(
                "DarkTicksRemaining".into(),
                Tag::Int(crate::mobs::aquatic::dark_ticks(m)),
            );
        }
        // Phantom.addAdditionalSaveData: the size, and the circling anchor.
        // `storeNullable` writes nothing for a phantom that has none, so its
        // save carries no `anchor_pos` at all.
        "minecraft:phantom" => {
            map.insert("size".into(), Tag::Int(m.extra.size));
            match m.extra.home {
                Some(anchor) => {
                    map.insert("anchor_pos".into(), Tag::IntArray(anchor.to_vec()));
                }
                None => {
                    map.remove(&"anchor_pos".into());
                }
            }
        }
        "minecraft:slime" | "minecraft:magma_cube" => {
            map.insert("Size".into(), Tag::Int(m.extra.size.max(1) - 1));
            map.insert("wasOnGround".into(), Tag::Byte(i8::from(m.extra.flag2)));
        }
        // Zombie and Skeleton.addAdditionalSaveData (a husk keeps Zombie's):
        // `ConversionTracker`'s conversion and affliction times, -1 when it is
        // not converting or not afflicted.
        // The tracker ticks for these, so the live counters win over the loaded
        // values: one that left the water or the snow saves -1.
        "minecraft:zombie" | "minecraft:husk" => {
            for (key, value) in [
                ("DrownedConversionTime", conversion_time(m)),
                ("InWaterTime", m.extra.counter),
            ] {
                map.insert(key.into(), Tag::Int(value));
            }
        }
        "minecraft:skeleton" => {
            for (key, value) in [
                ("StrayConversionTime", conversion_time(m)),
                ("FreezingTime", m.extra.counter),
            ] {
                map.insert(key.into(), Tag::Int(value));
            }
        }
        // Drowned.convertsInWater is false, so its tracker never ticks: vanilla
        // saves -1 for both.
        "minecraft:drowned" => {
            for (key, value) in [
                ("DrownedConversionTime", Tag::Int(-1)),
                ("InWaterTime", Tag::Int(-1)),
            ] {
                map.entry(key.into()).or_insert(value);
            }
        }
        // Sheep.addAdditionalSaveData: its wool as it is now.
        "minecraft:sheep" => {
            map.insert("Color".into(), Tag::Byte((m.extra.color & 15) as i8));
            map.insert("Sheared".into(), Tag::Byte(i8::from(m.extra.sheared)));
        }
        // CopperGolem.addAdditionalSaveData.
        "minecraft:copper_golem" => {
            map.insert(
                "next_weather_age".into(),
                Tag::Long(m.extra.vector[0] as i64),
            );
            map.insert(
                "weather_state".into(),
                string(crate::mobs::copper_golem::WEATHER[m.extra.variant.min(3) as usize]),
            );
        }
        "minecraft:ghast" => crate::mobs::nether::to_nbt(m, map),
        _ => {}
    }
    if m.species() == S::Frog {
        crate::mobs::brain::write_frog(m, map);
    }
    // Bogged.addAdditionalSaveData: whether its mushrooms were sheared.
    if m.species() == S::Bogged {
        map.insert("sheared".into(), Tag::Byte(i8::from(m.extra.sheared)));
    }
    // Zombie.addAdditionalSaveData: CanBreakDoors.
    if crate::mobs::doors::is_zombie(m.species()) {
        map.insert(
            "CanBreakDoors".into(),
            Tag::Byte(i8::from(m.extra.can_break_doors)),
        );
        // ConversionTracker.addAdditionalSaveData reads isAfflicted at save
        // time. Zombie supplies its cached isEyeInFluid(WATER), even with NoAI.
        if !m.fluids.eye_in_water {
            map.insert("InWaterTime".into(), Tag::Int(-1));
        }
        // ZombieVillager and ZombifiedPiglin save Zombie's tracker too; Horizon
        // never starts their water conversion, so its time is -1.
        if matches!(m.species(), S::ZombieVillager | S::ZombifiedPiglin) {
            map.entry("DrownedConversionTime".into())
                .or_insert(Tag::Int(-1));
        }
    }
    // Skeleton supplies its cached isInPowderSnow to the same isAfflicted check.
    if m.species() == S::Skeleton && !m.in_powder_snow {
        map.insert("FreezingTime".into(), Tag::Int(-1));
    }
    if matches!(
        id,
        "minecraft:zombie"
            | "minecraft:husk"
            | "minecraft:drowned"
            | "minecraft:zombie_villager"
            | "minecraft:zombified_piglin"
            | "minecraft:piglin"
            | "minecraft:zoglin"
    ) {
        map.insert("IsBaby".into(), Tag::Byte(i8::from(m.is_baby())));
    }
    // NeutralMob.addPersistentAngerSaveData; the bee saves its own.
    if matches!(
        id,
        "minecraft:iron_golem"
            | "minecraft:wolf"
            | "minecraft:polar_bear"
            | "minecraft:zombified_piglin"
            | "minecraft:enderman"
    ) {
        map.entry("anger_end_time".into()).or_insert(Tag::Long(-1));
    }
    if m.extra.attributes.is_some() {
        put_attributes(map, m);
    }
    put_sulfur_attributes(map, m);
    // The instances their own construction and load make (finalize::made_at_load),
    // which AttributeMap.pack saves although the loaded tag had none.
    let made = crate::mobs::finalize::made_at_load(m);
    // AttributeMap.pack writes the list even when no instance exists.
    map.entry("attributes".into())
        .or_insert_with(|| Tag::List(Vec::new()));
    for attr in made {
        let base = crate::mobs::finalize::made_base(m, attr);
        let mut list = match map.get(&"attributes".into()) {
            Some(Tag::List(l)) => l.clone(),
            _ => Vec::new(),
        };
        let attr_id = format!("minecraft:{}", attr.name());
        if !list
            .iter()
            .any(|t| text(t.get("id")).as_deref() == Some(attr_id.as_str()))
        {
            list.push(Tag::compound([
                ("id", string(&attr_id)),
                ("base", Tag::Double(base.to_bits())),
            ]));
        }
        map.insert("attributes".into(), Tag::List(list));
    }
    // Mob.addAdditionalSaveData: the chances that are not the default.
    if let Some(chances) = &m.extra.drop_chances {
        let set: Vec<(&str, Tag)> = crate::mobs::drops::SLOTS
            .iter()
            .zip(chances.iter())
            .filter(|(_, c)| **c != crate::mobs::drops::DEFAULT_DROP_CHANCE)
            .map(|(slot, c)| (*slot, Tag::Float(c.to_bits())))
            .collect();
        if set.is_empty() {
            map.remove(&"drop_chances".into());
        } else {
            map.insert("drop_chances".into(), Tag::compound(set));
        }
    }
    // SpellcasterIllager.addAdditionalSaveData: save the live casting timer,
    // including zero and negative values, rather than stale loaded NBT.
    if matches!(m.species(), S::Evoker | S::Illusioner) {
        map.insert("SpellTicks".into(), Tag::Int(m.extra.counter));
    }
    // Vindicator.addAdditionalSaveData: `Johnny` only while the flag is set
    // (`if (this.isJohnny) output.putBoolean("Johnny", true)`).
    if m.species() == S::Vindicator {
        if m.extra.johnny {
            map.insert("Johnny".into(), Tag::Byte(1));
        } else {
            map.remove(&"Johnny".into());
        }
    }
    crate::mobs::raider::write_nbt(m, map);
    crate::mobs::riders::write_nbt(m, map);
    // LivingEntity equipment: what the mob holds and wears now.
    {
        let mut equipment = m.extra.equipment.clone();
        let head = horizon_items::equipment::HEAD;
        // Held and worn items are saved with their enchantments and trim.
        for slot in horizon_items::equipment::MAINHAND..=head {
            equipment[slot] = crate::mobs::finalize::item_stack(m, slot);
        }
        // The captain's ominous banner keeps its patterns.
        if equipment[head].as_ref().is_some_and(|banner| {
            &*banner.item == crate::mobs::raider::BANNER
                && banner.extra.is_none()
                && crate::mobs::raider::is_raider(m.species())
        }) {
            equipment[head] = Some(crate::mobs::raider::ominous_banner());
        }
        equipment.store(map);
        put_live_attributes(map, &m.extra.live);
    }
    // A mannequin is an `Avatar`, not a `Mob`: none of
    // `Mob.addAdditionalSaveData`'s fields are its, written or loaded.
    if m.species() == crate::mobs::Species::Mannequin
        && let Tag::Compound(map) = &mut tag
    {
        for key in MOB_ONLY_KEYS {
            map.remove(&horizon_core::nbt::JavaString::from(key));
        }
    }
    tag
}
const MODEL_PARTS: [&str; 7] = [
    "cape",
    "jacket",
    "left_sleeve",
    "right_sleeve",
    "left_pants_leg",
    "right_pants_leg",
    "hat",
];
const MOB_ONLY_KEYS: [&str; 10] = [
    "NoAI",
    "PersistenceRequired",
    "CanPickUpLoot",
    "LeftHanded",
    "drop_chances",
    "leash",
    "home_radius",
    "home_pos",
    "DeathLootTable",
    "DeathLootTableSeed",
];
/// The live map's instances and the loaded attributes nothing else models,
/// over the saved `attributes` list (`AttributeMap.pack`).
pub fn put_live_attributes(map: &mut Compound, live: &crate::living_attributes::LivingAttributes) {
    let mut list = match map.get(&"attributes".into()) {
        Some(Tag::List(l)) => l.clone(),
        _ => Vec::new(),
    };
    let before = list.clone();
    live.save_into(&mut list);
    if list != before {
        map.insert("attributes".into(), Tag::List(list));
    }
}
/// `AttributeInstance.Packed` of the attributes with a base or permanent
/// modifier finalizeSpawn set (or a command or a load did), with their ids.
pub fn attribute_entries(m: &Mob) -> Vec<(String, Tag)> {
    use crate::mobs::finalize::{self, Attr, Op};
    let Some(a) = m.extra.attributes.as_deref() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for attr in Attr::ALL {
        let modifiers: Vec<Tag> = a
            .modifiers
            .iter()
            .filter(|x| x.attr == attr)
            .map(|x| {
                let op = match x.op {
                    Op::AddValue => "add_value",
                    Op::AddMultipliedBase => "add_multiplied_base",
                    Op::AddMultipliedTotal => "add_multiplied_total",
                };
                Tag::compound([
                    ("id", string(&x.id)),
                    ("amount", Tag::Double(x.amount.to_bits())),
                    ("operation", string(op)),
                ])
            })
            .collect();
        if modifiers.is_empty() && !a.bases.iter().any(|(b, _)| *b == attr) {
            continue;
        }
        let id = format!("minecraft:{}", attr.name());
        let mut entry = vec![
            ("id", string(&id)),
            ("base", Tag::Double(finalize::base_value(m, attr).to_bits())),
        ];
        if !modifiers.is_empty() {
            entry.push(("modifiers", Tag::List(modifiers)));
        }
        out.push((id, Tag::compound(entry)));
    }
    out
}
/// `AttributeMap.pack` for the attributes with a base or permanent modifier
/// finalizeSpawn set, over the saved list.
fn put_attributes(map: &mut Compound, m: &Mob) {
    let mut list = match map.get(&"attributes".into()) {
        Some(Tag::List(l)) => l.clone(),
        _ => Vec::new(),
    };
    if let Some(attributes) = m.extra.attributes.as_deref() {
        for attr in crate::mobs::finalize::Attr::ALL {
            if !attributes
                .modifiers
                .iter()
                .any(|modifier| modifier.attr == attr)
            {
                saved_attributes::forget_stale_modifiers(&mut list, attr);
            }
        }
    }
    for (id, entry) in attribute_entries(m) {
        match list
            .iter_mut()
            .find(|t| text(t.get("id")).as_deref() == Some(id.as_str()))
        {
            Some(t) => *t = entry,
            None => list.push(entry),
        }
    }
    map.insert("attributes".into(), Tag::List(list));
}
/// `AttributeMap.pack` of a sulfur cube's body archetype attributes: the
/// base and permanent modifiers of each instance its own map holds (the
/// archetype's transient modifiers are not saved).
fn put_sulfur_attributes(map: &mut Compound, m: &Mob) {
    use horizon_world::attributes::Operation;
    if m.species() != crate::mobs::Species::SulfurCube {
        return;
    }
    let Some(attributes) = m.extra.armor_attributes.as_deref() else {
        return;
    };
    let mut list = match map.get(&"attributes".into()) {
        Some(Tag::List(l)) => l.clone(),
        _ => Vec::new(),
    };
    let mut changed = false;
    for id in crate::mobs::sulfur_cube::ARCHETYPE_ATTRIBUTES {
        let Some(instance) = attributes
            .catalog()
            .id(id)
            .and_then(|k| attributes.instance(k))
        else {
            continue;
        };
        let packed = instance.pack();
        let mut entry = vec![
            ("id", string(id)),
            ("base", Tag::Double(packed.base.to_bits())),
        ];
        if !packed.modifiers.is_empty() {
            let modifiers = packed
                .modifiers
                .iter()
                .map(|x| {
                    let op = match x.operation() {
                        Operation::AddValue => "add_value",
                        Operation::AddMultipliedBase => "add_multiplied_base",
                        Operation::AddMultipliedTotal => "add_multiplied_total",
                    };
                    Tag::compound([
                        ("id", string(x.id())),
                        ("amount", Tag::Double(x.amount().to_bits())),
                        ("operation", string(op)),
                    ])
                })
                .collect();
            entry.push(("modifiers", Tag::List(modifiers)));
        }
        let entry = Tag::compound(entry);
        match list
            .iter_mut()
            .find(|t| text(t.get("id")).as_deref() == Some(id))
        {
            Some(t) => *t = entry,
            None => list.push(entry),
        }
        changed = true;
    }
    if changed {
        map.insert("attributes".into(), Tag::List(list));
    }
}
/// `Brain.Packed`: the memories finalizeSpawn set, over the saved ones.
/// `CustomName`: a text component saved as a string, a compound or a list,
/// flattened as `Component.getString` does (`translate`, `extra`).
fn custom_name(tag: &Tag) -> Option<String> {
    match tag.get("CustomName")? {
        Tag::String(s) => Some(s.to_string_lossy()),
        t @ (Tag::Compound(_) | Tag::List(_)) => Some(name_text(t).plain()),
        _ => None,
    }
}
pub use horizon_client_core::component::name_text;
/// `Mob.readAdditionalSaveData`: negative radii retain the live homePosition.
/// Used after the command adapter restores runtime AI, as well as fresh load.
pub fn read_mob_home(m: &mut Mob, tag: &Tag) {
    let radius = match tag.get("home_radius") {
        Some(Tag::Float(v)) => Some(f32::from_bits(*v).floor() as i32),
        Some(Tag::Double(v)) => Some(f64::from_bits(*v).floor() as i32),
        other => number(other).map(|v| v as i32),
    }
    .unwrap_or(-1);
    let pos = if radius >= 0 {
        home_block_pos(tag.get("home_pos")).unwrap_or([0; 3])
    } else {
        m.ai.home_position()
    };
    m.ai.restore_home(pos, radius);
}
/// `BlockPos.CODEC` through NbtOps.getIntStream: numeric collection entries
/// use Java Number's int conversion (floating entries truncate, unlike getIntOr above).
fn home_block_pos(tag: Option<&Tag>) -> Option<[i32; 3]> {
    match tag? {
        Tag::IntArray(v) => v.as_slice().try_into().ok(),
        Tag::ByteArray(v) if v.len() == 3 => Some([
            i32::from(v[0] as i8),
            i32::from(v[1] as i8),
            i32::from(v[2] as i8),
        ]),
        Tag::LongArray(v) if v.len() == 3 => Some([v[0] as i32, v[1] as i32, v[2] as i32]),
        Tag::List(v) if v.len() == 3 => {
            let int = |t: &Tag| match t {
                Tag::Float(v) => Some(f32::from_bits(*v) as i32),
                Tag::Double(v) => Some(f64::from_bits(*v) as i32),
                other => number(Some(other)).map(|v| v as i32),
            };
            Some([int(&v[0])?, int(&v[1])?, int(&v[2])?])
        }
        _ => None,
    }
}
mod mob_load;
pub use mob_load::{mob_from_nbt, mob_from_nbt_remote, mob_from_nbt_with_random};

fn mob_from_nbt_at(
    tag: &Tag,
    types: &HashMap<String, Arc<MobType>>,
    id: u32,
    registry: Option<&BlockRegistry>,
    random: horizon_core::random::EntityRandom,
    position: Vec3,
    authoritative: bool,
) -> Option<Mob> {
    let kind = types.get(&self::kind(tag)?)?.clone();
    let [yaw, pitch] = loaded_rotation(tag);
    let mut m = if authoritative {
        Mob::with_random(id, kind, position, yaw, random)
    } else {
        Mob::remote_with_random(id, kind, position, yaw, random)
    };
    // `Entity.load` runs after the constructor, so the saved yaw wins over
    // what a type's register does to it (a shulker's register zeroes it).
    m.yaw = yaw;
    m.velocity = motion(tag);
    m.head_pitch = pitch;
    // LivingEntity.readAdditionalSaveData: `setSharedFlag(7, getBooleanOr(FallFlying, false))`.
    m.extra.fall_flying = flag(tag, "FallFlying");
    read_mob_home(&mut m, tag);
    m.on_ground = flag(tag, "OnGround");
    m.fall_distance = float(tag.get("fall_distance")).unwrap_or(0.0);
    let cube = matches!(
        m.species(),
        crate::mobs::Species::Slime | crate::mobs::Species::MagmaCube
    );
    if cube {
        // AbstractCubeMob.readAdditionalSaveData: the size's attributes first,
        // so the saved ones replace them, the type's default too.
        crate::mobs::cube::set_size(&mut m, number(tag.get("Size")).unwrap_or(0) as i32 + 1);
    }
    // LivingEntity.readAdditionalSaveData: the saved attribute bases, then
    // the health, the max health when there is none.
    if let Some(Tag::List(attrs)) = tag.get("attributes") {
        for a in attrs {
            let (Some(id), Some(v)) = (text(a.get("id")), float(a.get("base"))) else {
                continue;
            };
            let v = m.kind.attribute_bounds.loaded_base(&id, v);
            // A mount's rolled health and speed stay attribute bases, which the server syncs.
            let equine = crate::mobs::equine::is_equine(m.species());
            match id.as_str() {
                "minecraft:max_health" if equine => crate::mobs::finalize::set_base(
                    &mut m,
                    crate::mobs::finalize::Attr::MaxHealth,
                    v,
                ),
                "minecraft:movement_speed" if equine => crate::mobs::finalize::set_base(
                    &mut m,
                    crate::mobs::finalize::Attr::MovementSpeed,
                    v,
                ),
                // A saved base other than the type's default is the
                // attribute's base, which `/attribute ... base get` reads.
                "minecraft:max_health"
                    if cube
                        || crate::mobs::finalize::default_value(
                            &m,
                            crate::mobs::finalize::Attr::MaxHealth,
                        ) != Some(v) =>
                {
                    crate::mobs::finalize::set_base(
                        &mut m,
                        crate::mobs::finalize::Attr::MaxHealth,
                        v,
                    )
                }
                "minecraft:max_health" => m.attrs.max_health = v as f32,
                "minecraft:movement_speed"
                    if cube
                        || crate::mobs::finalize::default_value(
                            &m,
                            crate::mobs::finalize::Attr::MovementSpeed,
                        ) != Some(v) =>
                {
                    crate::mobs::finalize::set_base(
                        &mut m,
                        crate::mobs::finalize::Attr::MovementSpeed,
                        v,
                    )
                }
                "minecraft:movement_speed" => m.attrs.speed = v,
                "minecraft:attack_damage"
                    if cube
                        || crate::mobs::finalize::default_value(
                            &m,
                            crate::mobs::finalize::Attr::AttackDamage,
                        ) != Some(v) =>
                {
                    crate::mobs::finalize::set_base(
                        &mut m,
                        crate::mobs::finalize::Attr::AttackDamage,
                        v,
                    )
                }
                "minecraft:attack_damage" => m.attrs.attack_damage = v as f32,
                "minecraft:attack_knockback" => m.attrs.attack_knockback = v,
                "minecraft:armor" if cube => {
                    crate::mobs::finalize::set_base(&mut m, crate::mobs::finalize::Attr::Armor, v)
                }
                "minecraft:flying_speed"
                    if crate::mobs::finalize::default_value(
                        &m,
                        crate::mobs::finalize::Attr::FlyingSpeed,
                    ) != Some(v) =>
                {
                    crate::mobs::finalize::set_base(
                        &mut m,
                        crate::mobs::finalize::Attr::FlyingSpeed,
                        v,
                    )
                }
                _ => {}
            }
        }
    }
    // `AttributeMap.apply`: the saved permanent modifiers of the attributes the
    // game models (the follow range bonus finalizeSpawn made among them), and
    // the other saved attributes for the live map.
    if let Some(Tag::List(_)) = tag.get("attributes") {
        use crate::mobs::finalize::Attr;
        saved_attributes::read_attribute_modifiers(&mut m, tag);
        // An attribute the type lacks is skipped (`AttributeMap.apply`).
        let kind = m.kind.clone();
        m.extra.live.load(tag.get("attributes"), |id| {
            kind.attributes.contains_key(id)
                && !Attr::ALL.iter().any(|a| *a != Attr::Armor && a.id() == id)
        });
    }
    m.health = float(tag.get("Health")).map_or(m.attrs.max_health, |h| h as f32);
    // setHealth clamps it to the max health the save left, before anything
    // later in the load (a wolf's taming) changes that.
    m.health = m.health.min(crate::mobs::finalize::loaded_max_health(&m));
    // LivingEntity.readAdditionalSaveData: `internalSetAbsorptionAmount`
    // stores the value as read; only `setAbsorptionAmount` clamps.
    m.absorption = float(tag.get("AbsorptionAmount")).map_or(0.0, |h| h as f32);
    // LivingEntity.setHealth clamps to the max health: a horse or llama is made
    // with the type's and saved before it drops to the one it rolled. (Only
    // the base is read here, so the clamp would cut a leader zombie's bonus.)
    if crate::mobs::equine::is_equine(m.species()) {
        m.health = m.health.min(m.attrs.max_health);
    }
    m.hurt_time = i32::from(number(tag.get("HurtTime")).unwrap_or(0) as i16);
    // What it holds (equipment.mainhand / offhand), when saved.
    if let Some(eq) = tag.get("equipment") {
        // The hands, armour, body and saddle: the one `equipment` codec.
        m.extra.equipment = horizon_items::equipment::Equipment::read(eq);
        // Their enchantments and trims.
        // (`gear` slot order: hands, then head to feet.)
        use horizon_items::equipment::{CHEST, FEET, HEAD, LEGS, MAINHAND, OFFHAND};
        for (index, slot) in [MAINHAND, OFFHAND, HEAD, CHEST, LEGS, FEET]
            .into_iter()
            .enumerate()
        {
            if let Some(stack) = m.extra.slot_stack(slot) {
                let levels = stack.enchantments().to_vec();
                let trim = stack.data().and_then(|d| d.trim.clone());
                crate::mobs::gear::set(&mut m, index, levels);
                crate::mobs::gear::set_trim(&mut m, index, trim);
            }
        }
    } else {
        // Registration hands out default weapons (finalizeSpawn does in
        // vanilla); a loaded mob holds only what its file lists.
        m.extra.set_main_hand(None);
    }
    mob_death::read(&mut m, tag);
    m.ai.ai_paused = ai_pause::read(tag);
    m.impulse_grace_time =
        number(tag.get("current_impulse_context_reset_grace_time")).unwrap_or(0) as i32;
    // Mob.readAdditionalSaveData overrides what the constructor set (copper
    // golems and elder guardians start persistent).
    m.persistent = flag(tag, "PersistenceRequired");
    m.extra.death_loot_table = text(tag.get("DeathLootTable"))
        .and_then(|t| horizon_client_core::identifier::normalize_id(&t));
    m.extra.death_table_seed = number(tag.get("DeathLootTableSeed")).unwrap_or(0);
    refs::read_last_hurt_by(&mut m, tag);
    // `NeutralMob.readPersistentAngerSaveData`: the anger end time and the
    // target come back with the mob.
    if persistent_anger(m.species()) {
        m.extra.anger_end = number(tag.get("anger_end_time")).unwrap_or(-1);
        // Anger at an entity other than the player is anger at a mob that may
        // not be here (`EntityReference`): it still is not `isAngryAtAllPlayers`.
        m.extra.angry_at = match uuid_from_tag(tag.get("angry_at")) {
            Some(uuid) if uuid == player_uuid() => Some(crate::mobs::Who::Player),
            Some(_) => Some(crate::mobs::Who::Mob(0)),
            None => None,
        };
    }
    // `Mannequin.readAdditionalSaveData`: the profile comes back with the skin, then
    // validated pose, `immovable`, `hide_description` and `description` (`updateDescription`).
    if m.species() == crate::mobs::Species::Mannequin {
        m.profile = tag.get("profile").and_then(profile_from_nbt);
        // Mannequin.POSE_CODEC accepts only these Avatar poses; an invalid
        // or absent value defaults to standing before the live dimensions read it.
        m.extra.avatar_pose = match text(tag.get("pose")).as_deref() {
            Some("crouching") => crate::pose::Pose::Crouching,
            Some("swimming") => crate::pose::Pose::Swimming,
            Some("fall_flying") => crate::pose::Pose::FallFlying,
            Some("sleeping") => crate::pose::Pose::Sleeping,
            _ => crate::pose::Pose::Standing,
        };
        // `HumanoidArm.CODEC`, right when absent or not an arm.
        m.extra.main_hand_left = text(tag.get("main_hand")).as_deref() == Some("left");
        // `LAYERS_CODEC`: every listed part hidden; an entry that is not a
        // part fails the whole list, which reads as `ALL_LAYERS` (none hidden).
        m.extra.hidden_model_parts = match tag.get("hidden_layers") {
            Some(Tag::List(parts)) => parts
                .iter()
                .try_fold(0u8, |mask, part| {
                    let name = text(Some(part))?;
                    let bit = MODEL_PARTS.iter().position(|p| *p == name)?;
                    Some(mask | 1 << bit)
                })
                .unwrap_or(0),
            _ => 0,
        };
        m.extra.immovable = flag(tag, "immovable");
        m.extra.description = if flag(tag, "hide_description") {
            crate::mobs::MannequinDescription::Hidden
        } else {
            crate::mobs::MannequinDescription::from_component(Some(
                &tag.get("description")
                    .cloned()
                    .or_else(|| crate::mobs::MannequinDescription::Default.component())
                    .unwrap_or(Tag::End),
            ))
        };
    }
    // AbstractPiglin.readAdditionalSaveData: getBooleanOr("CanPickUpLoot", true).
    let piglin = matches!(
        m.species(),
        crate::mobs::Species::Piglin | crate::mobs::Species::PiglinBrute
    );
    m.extra.pick_up_loot = tag
        .get("CanPickUpLoot")
        .and_then(int_value)
        .map_or(piglin, |value| value as i8 != 0);
    m.custom_name = custom_name(tag);
    m.custom_name_text = tag.get("CustomName").map(name_text);
    m.extra.left_handed = flag(tag, "LeftHanded");
    if let Some(Tag::Compound(c)) = tag.get("drop_chances") {
        let mut chances = [crate::mobs::drops::DEFAULT_DROP_CHANCE; 8];
        for (chance, slot) in chances.iter_mut().zip(crate::mobs::drops::SLOTS) {
            if let Some(Tag::Float(v)) = c.get(&slot.into()) {
                *chance = f32::from_bits(*v);
            }
        }
        m.extra.drop_chances = Some(Box::new(chances));
    }
    if saves_age(m.species()) {
        m.extra.age = tag.get("Age").and_then(int_value).unwrap_or(0);
        m.extra.forced_age = tag.get("ForcedAge").and_then(int_value).unwrap_or(0);
        m.extra.age_locked = flag(tag, "AgeLocked");
    }
    if is_animal(&m.kind.id) {
        m.extra.in_love = tag.get("InLove").and_then(int_value).unwrap_or(0);
        // Animal.readAdditionalSaveData: the player who fed it into love.
        m.extra.love_cause = crate::save::uuid_from_tag(tag.get("LoveCause"));
    }
    m.name_visible = flag(tag, "CustomNameVisible");
    m.no_ai = flag(tag, "NoAI");
    m.no_gravity = flag(tag, "NoGravity");
    m.glowing = flag(tag, "Glowing");
    m.ticks_frozen = number(tag.get("TicksFrozen")).unwrap_or(0) as i32;
    // LivingEntity.readAdditionalSaveData: the saved `active_effects`, the last
    // entry of an effect winning.
    if let Some(Tag::List(effects)) = tag.get("active_effects") {
        for e in effects {
            let Some(def) = text(e.get("id")).and_then(|id| crate::brewing::effect(&id)) else {
                continue;
            };
            let effect = crate::brewing::namespaced(def.id);
            let duration = number(e.get("duration")).unwrap_or(0) as i32;
            // `MobEffectInstance.Details`: Codec.BYTE uses boxed Number.byteValue,
            // then UNSIGNED_BYTE interprets the resulting byte as 0..255.
            let amplifier = e
                .get("amplifier")
                .and_then(horizon_world::datafix::dynamic::as_number)
                .map_or(0, |n| i32::from(n.byte_value() as u8));
            // Details.CODEC: `show_particles` defaults to true, `show_icon` to `show_particles`.
            let particles = number(e.get("show_particles")) != Some(0);
            let mut stored = crate::mobs::StoredEffect::new(effect, duration, amplifier);
            stored.show_icon = number(e.get("show_icon")).map_or(particles, |v| v != 0);
            m.load_effect(stored);
            m.extra.effect_chains.retain(|e| e.id != effect);
            if let Some(hidden) = e.get("hidden_effect") {
                m.extra
                    .effect_chains
                    .push(hidden_effect_from_nbt(effect, hidden));
            }
            m.extra.hidden_effects.retain(|h| *h != effect);
            if !particles {
                m.extra.hidden_effects.push(effect);
            }
            // Details.CODEC: `ambient` defaults to false.
            m.extra.ambient_effects.retain(|h| *h != effect);
            if number(e.get("ambient")).is_some_and(|a| a != 0) {
                m.extra.ambient_effects.push(effect);
            }
        }
        if m.extra
            .effects
            .iter()
            .any(|e| e.id == "minecraft:health_boost")
        {
            crate::mobs::finalize::health_boost_changed(&mut m);
        }
    }
    m.silent = flag(tag, "Silent");
    m.invulnerable_flag = flag(tag, "Invulnerable");
    // Entity.load: `getIntOr("invulnerable_time", 0)` — the entity-level
    // window (arismoko/horizon#916); the hurt cooldown never loads.
    m.invulnerable_time = match tag.get("invulnerable_time") {
        Some(Tag::Int(ticks)) => *ticks,
        _ => 0,
    };
    if m.species() == crate::mobs::Species::Bat {
        // Bat.readAdditionalSaveData: BatFlags bit 0, resting.
        m.extra.flag = number(tag.get("BatFlags")).unwrap_or(0) & 1 != 0;
    }
    if m.species() == crate::mobs::Species::Creeper {
        m.extra.charged = flag(tag, "powered");
        // Creeper.readAdditionalSaveData: `ignited` lights the fuse.
        m.extra.flag = flag(tag, "ignited");
        m.max_swell = number(tag.get("Fuse")).unwrap_or(30) as i16 as i32;
        // `getByteOr("ExplosionRadius", 3)`: any numeric tag, as a byte.
        m.extra.explosion_radius = number(tag.get("ExplosionRadius")).unwrap_or(3) as i8;
    }
    if m.species() == crate::mobs::Species::Ghast {
        crate::mobs::nether::from_nbt(&mut m, tag);
    }
    // InventoryCarrier.readInventoryFromTag, using the existing occupied-stack model.
    let carrier_slots = match m.species() {
        crate::mobs::Species::Pillager => 5,
        crate::mobs::Species::WanderingTrader => 8,
        _ => 0,
    };
    if carrier_slots != 0
        && let Some(Tag::List(items)) = tag.get("Inventory")
    {
        m.extra.inventory = items
            .iter()
            .filter_map(crate::save::item_from_nbt)
            .take(carrier_slots)
            .collect();
    }
    if matches!(
        m.species(),
        crate::mobs::Species::Piglin
            | crate::mobs::Species::PiglinBrute
            | crate::mobs::Species::Hoglin
    ) {
        m.extra.flag2 = flag(tag, "IsImmuneToZombification");
        m.extra.counter2 = number(tag.get("TimeInOverworld")).unwrap_or(0) as i32;
        if m.species() == crate::mobs::Species::Piglin {
            crate::mobs::piglin::read_guarded_anger(&mut m, tag);
        }
        if m.species() == crate::mobs::Species::Hoglin {
            m.extra.cannot_be_hunted = flag(tag, "CannotBeHunted");
        }
        if m.species() == crate::mobs::Species::Piglin
            && let Some(Tag::List(items)) = tag.get("Inventory")
        {
            // InventoryCarrier.readInventoryFromTag -> SimpleContainer
            // .fromItemList (eight slots).
            m.extra.inventory = crate::mobs::villager::load_container(&m, items, 8);
        }
    }
    if matches!(
        m.species(),
        crate::mobs::Species::Pig | crate::mobs::Species::Strider
    ) {
        // The old `Saddle` flag, when `equipment` held none.
        if flag(tag, "Saddle") && m.saddle_item().is_none() {
            m.set_saddle(Some(crate::items::ItemStack::new("minecraft:saddle", 1)));
        }
    }
    if crate::mobs::equine::is_equine(m.species()) {
        m.extra.tame = flag(tag, "Tame");
        // AbstractHorse.readAdditionalSaveData: the saved Int as is, camels included.
        m.extra.equine_temper = tag.get("Temper").and_then(int_value).unwrap_or(0);
        m.extra.bred = flag(tag, "Bred");
        m.extra.owner = crate::save::uuid_from_tag(tag.get("Owner"));
        let v = number(tag.get("Variant")).unwrap_or(0) as i32;
        (m.extra.variant, m.extra.variant2) = crate::mobs::equine::horse_look(v);
        if crate::mobs::llama::is_llama(m.species()) {
            crate::mobs::llama::from_nbt(&mut m, tag);
        }
        m.extra.chested = flag(tag, "ChestedHorse");
        if m.extra.chested {
            m.extra.items = crate::save::decode_items(tag.get("Items"), 15);
        }
        if let Some(Tag::List(attrs)) = tag.get("attributes") {
            for a in attrs {
                let (Some(id), Some(v)) = (text(a.get("id")), float(a.get("base"))) else {
                    continue;
                };
                if id == "minecraft:jump_strength" {
                    m.extra.vector[0] = v;
                }
            }
        }
        if m.extra.vector[0] <= 0.0 {
            m.extra.vector[0] = if crate::mobs::equine::is_chested(m.species()) {
                0.5
            } else {
                0.7
            };
        }
    }
    if crate::mobs::finalize::saves_variant_name(m.species())
        && let Some(v) = text(tag.get("variant"))
    {
        m.extra.variant = crate::mobs::finalize::variant_index(m.species(), &v);
    }
    if m.species() == crate::mobs::Species::Chicken
        && let Some(t) = number(tag.get("EggLayTime"))
    {
        m.extra.egg_time = t as i32;
    }
    if let Some(v) = text(tag.get("sound_variant")) {
        let v = v.trim_start_matches("minecraft:");
        let sounds = crate::mobs::finalize::sound_variants(m.species());
        m.extra.sound_variant = sounds.iter().position(|n| *n == v).map(|i| i as u8);
    }
    match m.species() {
        crate::mobs::Species::Fox => {
            m.extra.variant = u8::from(text(tag.get("Type")).as_deref() == Some("snow"));
            // Fox.readAdditionalSaveData: clearTrusted, then addTrustedEntity for
            // each of "Trusted".
            crate::mobs::fox::clear_trusted(&mut m);
            if let Some(Tag::List(trusted)) = tag.get("Trusted") {
                for uuid in trusted
                    .iter()
                    .filter_map(|t| crate::save::uuid_from_tag(Some(t)))
                {
                    crate::mobs::fox::add_trusted_entity(&mut m, uuid);
                }
            }
            // Fox.readAdditionalSaveData: the sleeping, sitting and crouching flags.
            for (key, bit) in [
                ("Sleeping", crate::mobs::fox::SLEEPING),
                ("Sitting", crate::mobs::fox::SITTING),
                ("Crouching", crate::mobs::fox::CROUCHING),
            ] {
                if flag(tag, key) {
                    m.extra.ints[0] |= bit;
                }
            }
        }
        // MushroomCow.readAdditionalSaveData: a missing or unknown name is
        // Variant.DEFAULT, red; stew_effects decodes like the component's list.
        crate::mobs::Species::Mooshroom => {
            m.extra.variant = u8::from(text(tag.get("Type")).as_deref() == Some("brown"));
            if let Some(Tag::List(list)) = tag.get("stew_effects") {
                m.extra.stew_effects = list
                    .iter()
                    .map(|entry| {
                        Some((
                            Arc::from(text(entry.get("id"))?.as_str()),
                            number(entry.get("duration")).unwrap_or(160) as i32,
                        ))
                    })
                    .collect();
            }
        }
        crate::mobs::Species::Rabbit => {
            // Rabbit.Variant.byId: an unknown id is brown.
            m.extra.variant = match number(tag.get("RabbitType")).unwrap_or(0) {
                v @ (0..=5 | 99) => v as u8,
                _ => 0,
            };
        }
        crate::mobs::Species::Goat => crate::mobs::goat::read_nbt(&mut m, tag),
        _ => {}
    }
    // `Rabbit.readAdditionalSaveData`.
    if m.species() == crate::mobs::Species::Rabbit {
        m.extra.more_carrot_ticks = number(tag.get("MoreCarrotTicks")).unwrap_or(0) as i32;
    }
    // `Ocelot.readAdditionalSaveData`.
    if m.species() == crate::mobs::Species::Ocelot {
        m.extra.trusting = flag(tag, "Trusting");
    }
    if m.species() == crate::mobs::Species::Cat {
        m.extra.owner = crate::save::uuid_from_tag(tag.get("Owner"));
        m.extra.tame = m.extra.owner.is_some();
        crate::mobs::tame::restore_sitting(&mut m, flag(tag, "Sitting"));
        let legacy = |c: i64| u8::try_from(c as i8).ok().filter(|c| *c < 16).unwrap_or(0);
        m.extra.color = number(tag.get("CollarColor")).map_or(crate::mobs::tame::RED, legacy);
    }
    if m.species() == crate::mobs::Species::Panda {
        let gene = |key| text(tag.get(key)).map_or(0, |n| crate::mobs::panda::gene_from_name(&n));
        m.extra.variant = gene("MainGene");
        m.extra.variant2 = gene("HiddenGene");
    }
    if m.species() == crate::mobs::Species::Parrot {
        m.extra.owner = crate::save::uuid_from_tag(tag.get("Owner"));
        m.extra.tame = m.extra.owner.is_some();
        crate::mobs::tame::restore_sitting(&mut m, flag(tag, "Sitting"));
    }
    if m.species() == crate::mobs::Species::Wolf {
        m.extra.owner = crate::save::uuid_from_tag(tag.get("Owner"));
        // TamableAnimal.readAdditionalSaveData: with an owner `setTame(true,
        // false)`, the flag alone, so the max health stays as saved; without
        // one `setTame(false, true)`, whose side effect sets its base to 8.
        if m.extra.owner.is_some() {
            m.extra.tame = true;
        } else {
            crate::mobs::tame::set_tame(&mut m, false);
        }
        crate::mobs::tame::restore_sitting(&mut m, flag(tag, "Sitting"));
        let legacy = |c: i64| u8::try_from(c as i8).ok().filter(|c| *c < 16).unwrap_or(0);
        m.extra.color = number(tag.get("CollarColor")).map_or(crate::mobs::tame::RED, legacy);
    }
    if matches!(
        m.species(),
        crate::mobs::Species::Villager | crate::mobs::Species::WanderingTrader
    ) {
        read_villager(&mut m, tag);
    }
    if m.species() == crate::mobs::Species::ZombieVillager {
        read_villager(&mut m, tag);
        let xp = number(tag.get("Xp")).unwrap_or(0) as i32;
        if xp != 0 {
            m.extra.trade.get_or_insert_with(Default::default).xp = xp;
        }
        crate::mobs::zombie_villager::read_nbt(&mut m, tag);
    }
    if m.species() == crate::mobs::Species::Dolphin {
        // Dolphin.readAdditionalSaveData: a save that names neither field is an
        // unfed dolphin at full moistness.
        m.extra.flag = flag(tag, "GotFish");
        m.extra.counter = number(tag.get("Moistness")).unwrap_or(2400) as i32;
    }
    if m.species() == crate::mobs::Species::Axolotl {
        restore_axolotl_memories(&mut m, tag);
    }
    read_variant(&mut m, tag);
    m.fire_ticks = i32::from(number(tag.get("Fire")).unwrap_or(0) as i16);
    // Entity.load: `input.getIntOr("Air", getMaxAirSupply())`, 4800 for a dolphin.
    m.air = number(tag.get("Air")).map_or(m.max_air_supply(), |air| air as i32);
    if m.species() == crate::mobs::Species::Allay {
        if let Some(Tag::List(inv)) = tag.get("Inventory") {
            for stack in inv.iter().filter_map(item_from_nbt) {
                let max = m.kind.inventory_stack_limit(&stack);
                if let Some(st) = m.extra.allay.as_mut() {
                    st.add(stack, max);
                }
            }
        }
        m.extra.egg_time = number(tag.get("DuplicationCooldown")).unwrap_or(0) as i32;
        if let (Some(st), Some(listener)) = (m.extra.allay.as_mut(), tag.get("listener")) {
            st.vibration = crate::sculk::Data::from_nbt(listener);
        }
        let liked = tag
            .get("Brain")
            .and_then(|b| b.get("memories"))
            .and_then(|b| b.get("minecraft:liked_player"));
        m.extra.tame = liked.is_some();
        m.extra.owner = liked
            .and_then(|l| l.get("value"))
            .and_then(|v| crate::save::uuid_from_tag(Some(v)));
        // `Brain.readAdditionalSaveData`: a memory saved with a time to live
        // (`Codec.LONG`, read from any integer tag). `Long.MAX_VALUE` is
        // `MemorySlot.NEVER_EXPIRE`: that memory never expires.
        if let Some(st) = m.extra.allay.as_mut() {
            st.liked_player_ttl = liked
                .and_then(|l| match l.get("ttl")? {
                    Tag::Byte(v) => Some(i64::from(*v)),
                    Tag::Short(v) => Some(i64::from(*v)),
                    Tag::Int(v) => Some(i64::from(*v)),
                    Tag::Long(v) => Some(*v),
                    _ => None,
                })
                .filter(|&ttl| ttl != i64::MAX);
        }
        let memory = |key: &str| {
            tag.get("Brain")
                .and_then(|b| b.get("memories"))
                .and_then(|b| b.get(key))
                .and_then(|l| l.get("value"))
        };
        m.extra.ints[2] =
            number(memory("minecraft:liked_noteblock_cooldown_ticks")).unwrap_or(0) as i32;
        let pickup = number(memory("minecraft:item_pickup_cooldown_ticks")).map(|t| t as i32);
        crate::mobs::allay::set_pickup_cooldown(&mut m, pickup);
        if let Some(st) = m.extra.allay.as_mut()
            && let Some(noteblock) = memory("minecraft:liked_noteblock")
            && let Some(Tag::IntArray(p)) = noteblock.get("pos")
            && p.len() == 3
        {
            st.noteblock = Some(crate::mobs::allay::GlobalPos {
                dimension: crate::mobs::allay::dimension_id(
                    &text(noteblock.get("dimension")).unwrap_or_default(),
                ),
                pos: [p[0], p[1], p[2]],
            });
        }
    }
    if m.species() == crate::mobs::Species::Frog
        && tag
            .get("Brain")
            .and_then(|b| b.get("memories"))
            .and_then(|b| b.get("minecraft:is_pregnant"))
            .is_some()
    {
        crate::mobs::frog::set_pregnant(&mut m, true);
    }
    if crate::mobs::camel::is_camel(m.species()) {
        // AbstractHorse.readAdditionalSaveData: the eating state (Temper is
        // read with the other equines' above).
        m.extra.flag = flag(tag, "EatingHaystack");
        // The tick is a game time: `camel::finish_load` puts it on the mob's
        // clock once the world's time is known.
        let tick = match tag.get("LastPoseTick") {
            Some(Tag::Long(v)) => *v,
            _ => 0,
        };
        crate::mobs::camel::stand_up_instantly(&mut m);
        m.extra.pose_change_tick = tick;
        m.extra.camel_sitting = tick < 0;
        // Camels are always tame (`Camel.register` in Horizon).
        m.extra.tame = true;
    }
    if crate::mobs::nautilus::is_nautilus(m.species()) {
        crate::mobs::nautilus::from_nbt(&mut m, tag);
    }
    if m.species() == crate::mobs::Species::SulfurCube {
        crate::mobs::sulfur_cube::from_nbt(&mut m, tag);
    }
    if m.species() == crate::mobs::Species::Bee {
        crate::mobs::bee::from_nbt(&mut m, tag);
    }
    if m.species() == crate::mobs::Species::Creaking
        && let Some(Tag::IntArray(home)) = tag.get("home_pos")
        && home.len() == 3
    {
        crate::mobs::creaking::set_transient(&mut m, [home[0], home[1], home[2]]);
    }
    if m.species() == crate::mobs::Species::Turtle {
        // Turtle.readAdditionalSaveData: `home_pos` (else where it stands) and `has_egg`.
        m.extra.flag = flag(tag, "has_egg");
        m.extra.home = Some(match tag.get("home_pos") {
            Some(Tag::IntArray(home)) if home.len() == 3 => [home[0], home[1], home[2]],
            _ => [
                m.position.x.floor() as i32,
                m.position.y.floor() as i32,
                m.position.z.floor() as i32,
            ],
        });
    }
    if m.species() == crate::mobs::Species::Wither {
        m.extra.counter = number(tag.get("Invul")).unwrap_or(0) as i32;
    }
    if m.species() == crate::mobs::Species::Sniffer {
        crate::mobs::sniffer::from_nbt(&mut m, tag);
    }
    if m.species() == crate::mobs::Species::CopperGolem {
        // CopperGolem.readAdditionalSaveData: -1 (unset) when missing.
        m.extra.vector[0] = match tag.get("next_weather_age") {
            Some(Tag::Long(v)) => *v as f64,
            _ => -1.0,
        };
        let weather = match tag.get("weather_state") {
            Some(Tag::String(s)) => s.to_string_lossy(),
            _ => String::new(),
        };
        m.extra.variant = crate::mobs::copper_golem::WEATHER
            .iter()
            .position(|w| *w == weather)
            .unwrap_or(0) as u8;
    }
    m.extra.leash_pending = crate::mobs::leash::pending_from_nbt(tag.get("leash"));
    if m.species() == crate::mobs::Species::Warden {
        // The brain starts afresh: a minute before it digs back down.
        crate::mobs::warden::finalize(&mut m, false, &mut Vec::new());
        if let Some(st) = m.extra.warden.as_mut() {
            if let Some(listener) = tag.get("listener") {
                st.vibration = crate::sculk::Data::from_nbt(listener);
            }
            // `Warden.readAdditionalSaveData`: the anger comes back as the list
            // of UUIDs it was saved as. Every suspect starts in the UUID map and
            // the anger ticks move the loaded ones into the live one, since a
            // suspect may be loaded long after the warden.
            if let Some(anger) = tag.get("anger") {
                st.anger =
                    crate::mobs::warden::AngerManagement::from_saved(warden_anger_from_nbt(anger));
            }
        }
        restore_warden_owned_memories(&mut m, tag);
    }
    if m.species() == crate::mobs::Species::Armadillo {
        let state = text(tag.get("state"));
        let scute_time = number(tag.get("scute_time")).map(|t| t as i32);
        crate::mobs::armadillo::load(
            &mut m,
            tag.get("Brain").is_some(),
            state.as_deref(),
            scute_time,
        );
    }
    if m.species() == crate::mobs::Species::Ravager {
        m.extra.counter = number(tag.get("AttackTick")).unwrap_or(0) as i32;
        m.extra.counter2 = number(tag.get("StunTick")).unwrap_or(0) as i32;
        m.extra.anim_ticks = number(tag.get("RoarTick")).unwrap_or(0) as i32;
    }
    if m.species() == crate::mobs::Species::Endermite {
        m.extra.counter = number(tag.get("Lifetime")).unwrap_or(0) as i32;
    }
    if m.species() == crate::mobs::Species::Vex {
        crate::mobs::vex::read_nbt(&mut m, tag);
    }
    // Bogged.readAdditionalSaveData: unsheared when absent.
    if m.species() == crate::mobs::Species::Bogged {
        m.extra.sheared = flag(tag, "sheared");
    }
    // Sheep.readAdditionalSaveData: white and unsheared when absent.
    if m.species() == crate::mobs::Species::Sheep {
        m.extra.color = crate::mobs::farm::saved_color(tag.get("Color"));
        m.extra.sheared = flag(tag, "Sheared");
    }
    // GlowSquid.readAdditionalSaveData: not dimmed when absent.
    if m.species() == crate::mobs::Species::GlowSquid {
        crate::mobs::aquatic::set_dark_ticks(
            &mut m,
            number(tag.get("DarkTicksRemaining")).unwrap_or(0) as i32,
        );
    }
    // IronGolem.readAdditionalSaveData: not player created when absent.
    if m.species() == crate::mobs::Species::IronGolem {
        m.extra.player_created = flag(tag, "PlayerCreated");
    }
    // SnowGolem.readAdditionalSaveData: getBooleanOr("Pumpkin", true).
    if m.species() == crate::mobs::Species::SnowGolem {
        m.extra.sheared = !tag
            .get("Pumpkin")
            .and_then(int_value)
            .is_none_or(|value| value as i8 != 0);
    }
    // Pufferfish.readAdditionalSaveData: `getIntOr("PuffState", 0)`, at most 2.
    if m.species() == crate::mobs::Species::Pufferfish {
        m.extra.size = number(tag.get("PuffState")).unwrap_or(0).min(2) as i32;
    }
    // Enderman.readAdditionalSaveData: the block it carried; an air state
    // reads as no block at all.
    if m.species() == crate::mobs::Species::Enderman
        && let Some(registry) = registry
    {
        m.extra.carried = tag
            .get("carriedBlockState")
            .and_then(|t| state_from_nbt(registry, t))
            .filter(|&state| !is_air(registry, state));
    }
    // HappyGhast.readAdditionalSaveData: the still timeout, 0 when absent (a fresh
    // ghast, or one saved by a version that did not write it).
    if m.species() == crate::mobs::Species::HappyGhast {
        m.extra.counter = number(tag.get("still_timeout")).unwrap_or(0) as i32;
    }
    if m.species() == crate::mobs::Species::Shulker {
        m.extra.variant3 = (number(tag.get("AttachFace")).unwrap_or(0) as i8 % 6).unsigned_abs();
        m.extra.counter = number(tag.get("Peek")).map_or(0, |v| v as i8 as i32);
        m.extra.color = number(tag.get("Color")).map_or(16, |v| v as i8 as u8);
        m.anim.v[0] = m.extra.counter as f32 * 0.01;
        m.anim.vo[0] = m.anim.v[0];
        // Shulker.setPos: square on its block.
        m.position = horizon_core::geometry::Vec3::new(
            m.position.x.floor() + 0.5,
            (m.position.y + 0.5).floor(),
            m.position.z.floor() + 0.5,
        );
        m.previous = m.position;
    }
    if matches!(
        m.species(),
        crate::mobs::Species::Slime | crate::mobs::Species::MagmaCube
    ) {
        m.extra.flag2 = flag(tag, "wasOnGround");
    }
    if m.species() == crate::mobs::Species::Phantom {
        // Phantom.readAdditionalSaveData: setPhantomSize(size), and the circling
        // anchor, absent or unreadable when the save has no anchor at all
        // (`input.read("anchor_pos", BlockPos.CODEC).orElse(null)`).
        crate::mobs::phantom::set_size(&mut m, number(tag.get("size")).unwrap_or(0) as i32);
        m.extra.home = home_block_pos(tag.get("anchor_pos"));
    }
    // Zombie, Piglin and Zoglin.readAdditionalSaveData: IsBaby (the baby
    // zombies of trial spawners).
    if !crate::mobs::finalize::ageable(m.species()) {
        crate::mobs::finalize::set_baby(&mut m, flag(tag, "IsBaby"));
    }
    // Zombie.readAdditionalSaveData: setCanBreakDoors.
    if crate::mobs::doors::is_zombie(m.species()) {
        crate::mobs::doors::set_can_break_doors(&mut m, flag(tag, "CanBreakDoors"));
    }
    // Zombie and Skeleton.readAdditionalSaveData: `ConversionTracker` reads its
    // affliction time (0 when absent) and starts a conversion from a saved time
    // other than -1, -1 stopping one (`setConverting(false)`).
    let tracker = match m.species() {
        crate::mobs::Species::Zombie | crate::mobs::Species::Husk => {
            Some(("InWaterTime", "DrownedConversionTime"))
        }
        crate::mobs::Species::Skeleton => Some(("FreezingTime", "StrayConversionTime")),
        _ => None,
    };
    if let Some((affliction, conversion)) = tracker {
        m.extra.counter = number(tag.get(affliction)).unwrap_or(0) as i32;
        let time = number(tag.get(conversion)).unwrap_or(-1) as i32;
        // Any other time starts a conversion (`startConversion`), a negative
        // one spent on the next tick.
        m.extra.counter2 = if time == -1 { 0 } else { time.max(0) + 1 };
    }
    // SpellcasterIllager.readAdditionalSaveData: this is the same timer used
    // by casting goals and the existing customServerAiStep countdown.
    if matches!(
        m.species(),
        crate::mobs::Species::Evoker | crate::mobs::Species::Illusioner
    ) {
        m.extra.counter = number(tag.get("SpellTicks")).unwrap_or(0) as i32;
    }
    // Vindicator.readAdditionalSaveData: `Johnny`, false when absent. A name
    // that is Johnny latches the flag again on the next AI step.
    if m.species() == crate::mobs::Species::Vindicator {
        m.extra.johnny = flag(tag, "Johnny");
    }
    crate::mobs::raider::read_nbt(&mut m, tag);
    crate::mobs::riders::read_nbt(&mut m, tag);
    // A mannequin is an `Avatar`, not a `Mob`: it reads none of
    // `Mob.readAdditionalSaveData`'s flags, after the common reads above.
    if m.species() == crate::mobs::Species::Mannequin {
        m.persistent = false;
        m.extra.pick_up_loot = false;
        m.extra.left_handed = false;
        m.no_ai = false;
    }
    // setHealth clamps a negative health to 0; a dead mob keeps its body (tickDeath).
    if m.health < 0.0 {
        m.health = 0.0;
    }
    Some(m)
}

/// `ItemEntity`: Health, Age, PickupDelay, Item, Thrower.
pub fn item_entity_to_nbt(e: &ItemEntity, uuid: [i32; 4], raw: Option<&Tag>) -> Tag {
    item_entity_to_nbt_saved(e, uuid, raw, item_to_nbt(&e.stack))
}

/// ItemEntity saved with the actual loaded item prototype, including transfers.
pub fn item_entity_to_nbt_with_items(
    e: &ItemEntity,
    uuid: [i32; 4],
    raw: Option<&Tag>,
    items: &crate::items::Items,
) -> Tag {
    item_entity_to_nbt_saved(
        e,
        uuid,
        raw,
        horizon_items::item_nbt::item_to_nbt_with_items(&e.stack, items),
    )
}

fn item_entity_to_nbt_saved(e: &ItemEntity, uuid: [i32; 4], raw: Option<&Tag>, item: Tag) -> Tag {
    let base = Base {
        id: "minecraft:item",
        uuid,
        position: e.position,
        motion: e.velocity,
        rotation: e.rotation,
        on_ground: e.on_ground,
        fall_distance: e.fall_distance,
    };
    let mut tag = base.non_mob_tag(raw);
    for (key, value) in [
        ("Item", item),
        ("Health", Tag::Short(e.health as i16)),
        ("Fire", Tag::Short(e.fire_ticks as i16)),
        (
            "Age",
            Tag::Short(e.age.clamp(i16::MIN as i32, i16::MAX as i32) as i16),
        ),
        (
            "PickupDelay",
            Tag::Short(
                e.pickup_delay
                    .clamp(i64::from(i16::MIN), i64::from(i16::MAX)) as i16,
            ),
        ),
    ] {
        put(&mut tag, key, value);
    }
    if let Some(thrower) = e.thrower {
        put(&mut tag, "Thrower", Tag::IntArray(thrower.to_vec()));
    } else if let Tag::Compound(map) = &mut tag {
        map.remove(&"Thrower".into());
    }
    if let Some(target) = e.target {
        put(&mut tag, "Owner", Tag::IntArray(target.to_vec()));
    } else if let Tag::Compound(map) = &mut tag {
        map.remove(&"Owner".into());
    }
    tag
}
/// `PrimedTnt.readAdditionalSaveData`: `explosion_power`, 4 by default and
/// clamped to 0..128.
pub fn tnt_power(tag: &Tag) -> f32 {
    // `getFloatOr` accepts any numeric tag, not only a float.
    let power = match tag.get("explosion_power") {
        Some(Tag::Byte(v)) => f32::from(*v),
        Some(Tag::Short(v)) => f32::from(*v),
        Some(Tag::Int(v)) => *v as f32,
        Some(Tag::Long(v)) => *v as f32,
        Some(Tag::Float(bits)) => f32::from_bits(*bits),
        Some(Tag::Double(bits)) => f64::from_bits(*bits) as f32,
        _ => 4.0,
    };
    power.clamp(0.0, 128.0)
}

/// `PrimedTnt.addAdditionalSaveData`: its `fuse` and, when it is not the
/// default, its `explosion_power`, the encoded live block state, and its
/// `owner` (`EntityReference.store`). With no live owner, a loaded `owner`
/// stays as the raw compound has it.
#[allow(clippy::too_many_arguments)]
pub fn tnt_to_nbt(
    position: Vec3,
    motion: Vec3,
    on_ground: bool,
    fall_distance: f64,
    fuse: i32,
    power: f32,
    block_state: Tag,
    rotation: [f32; 2],
    uuid: [i32; 4],
    owner: Option<[i32; 4]>,
    raw: Option<&Tag>,
) -> Tag {
    let mut tag = Base {
        id: "minecraft:tnt",
        uuid,
        position,
        motion,
        rotation,
        on_ground,
        fall_distance,
    }
    .tag(raw);
    put(&mut tag, "fuse", Tag::Short(fuse as i16));
    if power != 4.0 {
        put(&mut tag, "explosion_power", Tag::Float(power.to_bits()));
    } else if let Tag::Compound(map) = &mut tag {
        map.remove(&"explosion_power".into());
    }
    put(&mut tag, "block_state", block_state);
    if let Some(owner) = owner {
        put(&mut tag, "owner", Tag::IntArray(owner.to_vec()));
    }
    tag
}
/// `PrimedTnt.readAdditionalSaveData`: `EntityReference.read(input, "owner")`.
pub fn tnt_owner(tag: &Tag) -> Option<[i32; 4]> {
    uuid_from_tag(tag.get("owner"))
}
/// `EvokerFangs.addAdditionalSaveData`: always save the live warmup counter.
/// Known Owner UUIDs use `EntityReference.store` encoding; unresolved
/// loaded Owner data remains in the raw compound.
pub fn evoker_fangs_to_nbt(
    position: Vec3,
    yaw: f32,
    warmup: i32,
    owner: Option<[i32; 4]>,
    uuid: [i32; 4],
    raw: Option<&Tag>,
    live_motion: Option<Vec3>,
) -> Tag {
    let mut tag = Base {
        id: "minecraft:evoker_fangs",
        uuid,
        position,
        motion: live_motion.unwrap_or_else(|| raw.map(motion).unwrap_or(Vec3::ZERO)),
        rotation: [yaw, raw.map(rotation).unwrap_or([0.0; 2])[1]],
        on_ground: raw.is_some_and(|t| flag(t, "OnGround")),
        fall_distance: raw.map_or(0.0, |t| ground_from_nbt(t).1),
    }
    .tag(raw);
    put(&mut tag, "Warmup", Tag::Int(warmup));
    if let Some(owner) = owner {
        put(&mut tag, "Owner", Tag::IntArray(owner.to_vec()));
    }
    tag
}

/// `UUIDUtil.CODEC` via `NbtOps.getIntStream`: boxed Number conversions,
/// with the first-four partial value retained by `TagValueInput.read`.
fn item_uuid_from_tag(tag: Option<&Tag>) -> Option<[i32; 4]> {
    let values = horizon_world::datafix::dynamic::as_int_stream(tag)?;
    values.get(..4)?.try_into().ok()
}

/// The `Entity` fields a saved non-mob entity carries (`Base::tag`), for a
/// caller that writes its own kind's fields around them.
pub fn entity_base_tag(
    id: &str,
    position: Vec3,
    motion: Vec3,
    rotation: [f32; 2],
    uuid: [i32; 4],
    raw: Option<&Tag>,
) -> Tag {
    Base {
        id,
        uuid,
        position,
        motion,
        rotation,
        on_ground: raw.is_some_and(|t| flag(t, "OnGround")),
        fall_distance: raw.map_or(0.0, |t| ground_from_nbt(t).1),
    }
    .tag(raw)
}

/// `ItemEntity.readAdditionalSaveData`: `getShortOr` (floored, then narrowed) for three;
/// Owner and Thrower use the numeric UUID codec.
pub fn item_entity_from_nbt(tag: &Tag, id: u32, bob_offset: f32) -> Option<ItemEntity> {
    let stack = item_from_nbt(tag.get("Item")?)?;
    let position = position(tag)?;
    let short = |key, default| match tag.get(key) {
        Some(Tag::Float(v)) => f32::from_bits(*v).floor() as i32 as i16,
        Some(Tag::Double(v)) => f64::from_bits(*v).floor() as i32 as i16,
        other => number(other).unwrap_or(default) as i16,
    };
    Some(ItemEntity {
        attachment_lifetime: Default::default(),
        rotation: rotation(tag),
        join: Default::default(),
        no_gravity: flag(tag, "NoGravity"),
        health: i32::from(short("Health", 5)),
        fire_ticks: i32::from(short("Fire", 0)),
        id,
        stack,
        thrower: item_uuid_from_tag(tag.get("Thrower")),
        target: item_uuid_from_tag(tag.get("Owner")),
        position,
        previous: position,
        velocity: motion(tag),
        on_ground: flag(tag, "OnGround"),
        vertical_collision: false,
        horizontal_collision: false,
        // Entity.load: getDoubleOr, any number widened.
        fall_distance: float(tag.get("fall_distance")).unwrap_or(0.0),
        age: i32::from(short("Age", 0)),
        pickup_delay: i64::from(short("PickupDelay", 0)),
        bob_offset,
        ticks: 0,
        needs_sync: false,
        stuck: Vec3::ZERO,
        was_touching_water: false,
        // The caller gives it the own random that drew its UUID and bobOffs.
        random: horizon_core::random::EntityRandom::own(LegacyRandom::new(0)),
    })
}

/// `OminousItemSpawner.addAdditionalSaveData`: its item (left out when
/// empty) and `spawn_item_after_ticks`. Its tick count is the entity's
/// `tickCount`, which isn't saved: a reloaded spawner counts from 0 again.
/// It never moves or turns: it saves the Motion, Rotation, OnGround and
/// fall_distance it was loaded with (`Entity.load`).
pub fn ominous_item_spawner_to_nbt(
    item: &crate::items::ItemStack,
    position: Vec3,
    spawn_item_after: i64,
    uuid: [i32; 4],
    raw: Option<&Tag>,
) -> Tag {
    let base = Base {
        id: "minecraft:ominous_item_spawner",
        uuid,
        position,
        motion: raw.map_or(Vec3::ZERO, loaded_motion),
        rotation: raw.map_or([0.0, 0.0], rotation),
        on_ground: raw.is_some_and(|t| flag(t, "OnGround")),
        fall_distance: float(raw.and_then(|t| t.get("fall_distance"))).unwrap_or(0.0),
    };
    let mut tag = base.tag(raw);
    // `ItemStack.CODEC` can't encode `ItemStack.EMPTY`: an empty item writes no key.
    if let Tag::Compound(map) = &mut tag {
        map.remove(&"item".into());
    }
    if &*item.item != "minecraft:air" {
        put(&mut tag, "item", item_to_nbt(item));
    }
    put(
        &mut tag,
        "spawn_item_after_ticks",
        Tag::Long(spawn_item_after),
    );
    tag
}
/// `OminousItemSpawner.readAdditionalSaveData`: the item, where it floats
/// and `spawn_item_after_ticks` (0 when absent). None without an item:
/// such a spawner would spawn nothing.
pub fn ominous_item_spawner_from_nbt(tag: &Tag) -> Option<(crate::items::ItemStack, Vec3, i64)> {
    // `readAdditionalSaveData`: `ItemStack.EMPTY` when it has none.
    let item = tag
        .get("item")
        .and_then(item_from_nbt)
        .unwrap_or_else(|| crate::items::ItemStack::new("minecraft:air", 1));
    let after = number(tag.get("spawn_item_after_ticks")).unwrap_or(0);
    Some((item, position(tag)?, after))
}

/// `ExperienceOrb`: Fire, Health, Age, Value, Count.
pub fn orb_to_nbt(o: &ExperienceOrb, uuid: [i32; 4]) -> Tag {
    let base = Base {
        id: "minecraft:experience_orb",
        uuid,
        position: o.position,
        motion: o.velocity,
        rotation: o.rotation,
        on_ground: o.on_ground,
        fall_distance: o.fall_distance,
    };
    let mut tag = base.tag(None);
    for (key, value) in [
        ("Fire", Tag::Short(o.fire_ticks as i16)),
        ("Health", Tag::Short(o.health as i16)),
        ("Air", Tag::Short(o.air as i16)),
        ("Age", Tag::Short(o.age.min(i16::MAX as u32) as i16)),
        ("Value", Tag::Short(o.value as i16)),
        ("Count", Tag::Int(o.count.min(i32::MAX as u32) as i32)),
    ] {
        put(&mut tag, key, value);
    }
    // Entity.saveWithoutId writes NoGravity only when set.
    if o.no_gravity {
        put(&mut tag, "NoGravity", Tag::Byte(1));
    }
    tag
}
/// One orb with its merged `Count`.
pub fn orbs_from_nbt(tag: &Tag) -> Vec<ExperienceOrb> {
    let Some(position) = position(tag) else {
        return Vec::new();
    };
    let count = number(tag.get("Count")).unwrap_or(1).max(1) as u32;
    let orb = ExperienceOrb {
        fire_ticks: number(tag.get("Fire")).unwrap_or(0) as i16 as i32,
        attachment_lifetime: Default::default(),
        fall_distance: float(tag.get("fall_distance")).unwrap_or(0.0),
        rotation: rotation(tag),
        join: Default::default(),
        no_gravity: flag(tag, "NoGravity"),
        id: 0,
        position,
        previous: position,
        velocity: motion(tag),
        // `getShortOr("Value", 0)`, unclamped.
        value: tag.get("Value").and_then(int_value).unwrap_or(0) as i16 as i32,
        age: number(tag.get("Age")).unwrap_or(0).max(0) as u32,
        on_ground: flag(tag, "OnGround"),
        count,
        ticks: 0,
        health: number(tag.get("Health")).unwrap_or(5) as i32,
        air: number(tag.get("Air")).map_or(300, |air| air as i32), // getIntOr("Air", max)
        // ExperienceOrb's followingPlayer is transient: never saved, so a
        // loaded orb starts without one.
        following: None,
    };
    vec![orb]
}

/// `Projectile` + `AbstractArrow.addAdditionalSaveData` of a live arrow;
/// `in_block` is its `inBlockState`, `raw` the compound it was loaded with.
pub fn arrow_to_nbt(
    p: &Projectile,
    uuid: [i32; 4],
    owner: Option<[i32; 4]>,
    in_block: Option<Tag>,
    raw: Option<&Tag>,
) -> Option<Tag> {
    let a = p.arrow.as_deref()?;
    let ProjectileKind::Arrow {
        base_damage,
        critical,
        ..
    } = p.kind
    else {
        return None;
    };
    let base = Base {
        // A spectral arrow is its own entity type.
        id: if a.is_spectral_type() {
            "minecraft:spectral_arrow"
        } else {
            "minecraft:arrow"
        },
        uuid,
        position: p.position,
        motion: p.velocity,
        rotation: [p.yaw, p.pitch],
        on_ground: p.on_ground,
        fall_distance: a.fall_distance,
    };
    let mut tag = base.tag(raw);
    // `SpectralArrow.addAdditionalSaveData` always writes its integer duration.
    if base.id == "minecraft:spectral_arrow" {
        put(&mut tag, "Duration", Tag::Int(a.spectral_duration));
    }
    for (key, value) in [
        ("Fire", Tag::Short(a.fire_ticks as i16)),
        ("Air", Tag::Short(p.air_supply as i16)),
        ("life", Tag::Short(a.life as i16)),
        ("shake", Tag::Byte(a.shake as i8)),
        ("inGround", Tag::Byte(i8::from(a.in_ground))),
        ("pickup", Tag::Byte(a.pickup.byte())),
        ("damage", Tag::Double(base_damage.to_bits())),
        ("crit", Tag::Byte(i8::from(critical))),
        ("PierceLevel", Tag::Byte(a.pierce as i8)),
        ("HasBeenShot", Tag::Byte(i8::from(a.has_been_shot))),
        ("item", item_to_nbt(&a.pickup_item)),
    ] {
        put(&mut tag, key, value);
    }
    if let Tag::Compound(map) = &mut tag {
        let sound = a.saved_sound_event(map.get(&"SoundEvent".into()));
        map.insert("SoundEvent".into(), sound);
        // Projectile: LeftOwner only when set; Owner as the UUID.
        map.remove(&"LeftOwner".into());
        map.remove(&"inBlockState".into());
        map.remove(&"weapon".into());
        map.remove(&"can_break".into());
    }
    if let Some(weapon) = &a.fired_from_weapon {
        put(&mut tag, "weapon", item_to_nbt(weapon));
    }
    // `Projectile.addAdditionalSaveData`: the actual decoded predicate,
    // never a stale source compound or the pickup item's component patch.
    if let Some(predicate) = &a.can_break {
        put(&mut tag, "can_break", predicate.clone());
    }
    if p.left_owner {
        put(&mut tag, "LeftOwner", Tag::Byte(1));
    }
    if let Some(state) = in_block {
        put(&mut tag, "inBlockState", state);
    }
    // Entity.saveWithoutId writes NoGravity only when set.
    if p.no_gravity {
        put(&mut tag, "NoGravity", Tag::Byte(1));
    }
    if let Some(owner) = owner {
        put(&mut tag, "Owner", Tag::IntArray(owner.to_vec()));
    }
    Some(tag)
}
/// A saved arrow as a live one (`readAdditionalSaveData`), with its
/// `inBlockState` compound for the caller to resolve. `effect` is the tipped
/// arrow's lasting effect from its item.
pub fn arrow_from_nbt(
    tag: &Tag,
    player: Option<[i32; 4]>,
    effect: impl FnOnce(&crate::items::ItemStack) -> Option<(&'static str, i32)>,
) -> Option<(Projectile, Option<Tag>)> {
    let position = position(tag)?;
    let owner = uuid_from_tag(tag.get("Owner"));
    let from_player = player.is_some() && owner == player;
    // `getDefaultPickupItem`: a `SpectralArrow` is picked up as its own item.
    let spectral = kind(tag).as_deref() == Some("minecraft:spectral_arrow");
    let default_item = if spectral {
        "minecraft:spectral_arrow"
    } else {
        "minecraft:arrow"
    };
    let item = tag
        .get("item")
        .and_then(item_from_nbt)
        .unwrap_or_else(|| crate::items::ItemStack::new(default_item, 1));
    let kind = ProjectileKind::Arrow {
        base_damage: float(tag.get("damage")).unwrap_or(2.0),
        critical: flag(tag, "crit"),
        effect: effect(&item),
    };
    let mut p = Projectile::new(kind, position, from_player.then_some(Who::Player));
    p.velocity = motion(tag);
    [p.yaw, p.pitch] = rotation(tag);
    p.left_owner = flag(tag, "LeftOwner");
    p.owner_uuid = owner;
    p.no_gravity = flag(tag, "NoGravity");
    p.on_ground = flag(tag, "OnGround");
    projectile_fields::load_arrow_entity_fields(&mut p, tag);
    let a = p.arrow.as_mut()?;
    a.life = number(tag.get("life")).unwrap_or(0) as i16 as i32;
    a.shake = (number(tag.get("shake")).unwrap_or(0) as i8 as u8).into();
    a.in_ground = flag(tag, "inGround");
    a.pickup = Pickup::from_byte(number(tag.get("pickup")).unwrap_or(0));
    a.pierce = number(tag.get("PierceLevel")).unwrap_or(0) as u8;
    a.has_been_shot = flag(tag, "HasBeenShot");
    // `Projectile.readAdditionalSaveData`: one block predicate or a nonempty
    // list. Unsupported conjuncts remain saved, but cannot grant permission.
    a.can_break = tag
        .get("can_break")
        .filter(|predicate| match predicate {
            Tag::Compound(_) => true,
            Tag::List(list) => {
                !list.is_empty() && list.iter().all(|p| matches!(p, Tag::Compound(_)))
            }
            _ => false,
        })
        .cloned();
    a.spectral = spectral;
    // `SpectralArrow.readAdditionalSaveData`: absent Duration defaults to 200.
    if spectral {
        a.read_spectral_duration(tag);
    }
    a.pickup_item = item;
    a.load_weapon_and_sound(tag, item_from_nbt);
    p.stuck = a.in_ground;
    Some((p, tag.get("inBlockState").cloned()))
}

/// `LargeFireball`, `SmallFireball` and `WitherSkull`: `AbstractHurtingProjectile`'s
/// `acceleration_power`, `Projectile`'s owner and flags, `ExplosionPower`,
/// and a skull's `dangerous`.
pub fn fireball_to_nbt(
    p: &Projectile,
    uuid: [i32; 4],
    owner: Option<[i32; 4]>,
    raw: Option<&Tag>,
) -> Option<Tag> {
    let power = match p.kind {
        ProjectileKind::LargeFireball { power } => Some(power),
        ProjectileKind::SmallFireball
        | ProjectileKind::DragonFireball
        | ProjectileKind::WindCharge
        | ProjectileKind::BreezeWindCharge
        | ProjectileKind::WitherSkull { .. } => None,
        _ => return None,
    };
    let base = Base {
        id: p.kind.id(),
        uuid,
        position: p.position,
        motion: p.velocity,
        rotation: [p.yaw, p.pitch],
        on_ground: p.on_ground,
        fall_distance: p.fall_distance,
    };
    let mut tag = base.tag(raw);
    put(
        &mut tag,
        "acceleration_power",
        Tag::Double(p.acceleration.to_bits()),
    );
    put_projectile(&mut tag, p, owner);
    if projectile_fields::keeps_entity_fields(p) {
        put_projectile_entity_fields(&mut tag, p);
    }
    if let Some(power) = power {
        put(&mut tag, "ExplosionPower", Tag::Byte(power as i8));
    }
    // WitherSkull.addAdditionalSaveData: `dangerous`, false for a black skull.
    if let ProjectileKind::WitherSkull { dangerous } = p.kind {
        put(&mut tag, "dangerous", Tag::Byte(i8::from(dangerous)));
    }
    // Fireball.addAdditionalSaveData: the item `getItem` returns, a fire
    // charge unless `setItem` was given another (kept as a single one).
    if let Some(item) = &p.item {
        put(&mut tag, "Item", item_to_nbt(item));
    }
    Some(tag)
}

/// `Projectile.addAdditionalSaveData`: Owner, LeftOwner only when set, and
/// the explicit HasBeenShot flag.
fn put_projectile(tag: &mut Tag, p: &Projectile, owner: Option<[i32; 4]>) {
    if let Tag::Compound(map) = tag {
        map.remove(&"LeftOwner".into());
    }
    if p.left_owner {
        put(tag, "LeftOwner", Tag::Byte(1));
    }
    // Entity.saveWithoutId writes NoGravity only when set.
    if p.no_gravity {
        put(tag, "NoGravity", Tag::Byte(1));
    }
    put(tag, "HasBeenShot", Tag::Byte(i8::from(p.has_been_shot)));
    if let Some(owner) = owner {
        put(tag, "Owner", Tag::IntArray(owner.to_vec()));
    }
}

/// The thrown item projectiles Horizon saves: each entity type's id is also
/// the item it saves when none was loaded (`getDefaultItem`).
const THROWABLE_ITEMS: [&str; 6] = [
    "minecraft:snowball",
    "minecraft:egg",
    "minecraft:ender_pearl",
    "minecraft:splash_potion",
    "minecraft:lingering_potion",
    "minecraft:experience_bottle",
];
/// The item a throwable flies as: its stack (a potion's), the mod type's
/// item, or the item its entity type is named after.
fn throwable_item(kind: &ProjectileKind) -> Option<&str> {
    match kind {
        ProjectileKind::Thrown(t) => Some(t.item),
        ProjectileKind::Potion(stack, _) => Some(&stack.item),
        _ => THROWABLE_ITEMS.iter().find(|id| **id == kind.id()).copied(),
    }
}

/// The thrown kind an entity id names: vanilla's, or a content mod's type.
/// A potion flies as the item its entity type names, which is the item a
/// summoned one has when it names none (`getDefaultItem`).
pub fn throwable_kind(id: &str) -> Option<ProjectileKind> {
    if !THROWABLE_ITEMS.contains(&id) {
        return crate::projectiles::thrown_type(id).map(ProjectileKind::Thrown);
    }
    match id {
        "minecraft:snowball" => Some(ProjectileKind::Snowball),
        "minecraft:egg" => Some(ProjectileKind::Egg),
        "minecraft:ender_pearl" => Some(ProjectileKind::EnderPearl),
        // The entity type is the id; the `Item` read later may be another potion.
        "minecraft:experience_bottle" => Some(ProjectileKind::ExperienceBottle),
        _ => Some(ProjectileKind::Potion(
            Box::new(crate::items::ItemStack::new(id, 1)),
            id == "minecraft:lingering_potion",
        )),
    }
}

/// Whether its chunk saves it: the kinds `arrow_to_nbt`, `fireball_to_nbt`
/// and [`throwable_to_nbt`] write.
pub fn saved_with_chunk(p: &Projectile) -> bool {
    // No wildcard: a kind added to `ProjectileKind` must say here whether its
    // chunk saves it (`Entity.shouldBeSaved` is true for every projectile in
    // vanilla), and a `true` needs a writer in `entity_chunks`.
    match &p.kind {
        ProjectileKind::LargeFireball { .. }
        | ProjectileKind::SmallFireball
        | ProjectileKind::DragonFireball
        | ProjectileKind::WindCharge
        | ProjectileKind::BreezeWindCharge
        | ProjectileKind::WitherSkull { .. }
        | ProjectileKind::Snowball
        | ProjectileKind::Egg
        | ProjectileKind::EnderPearl
        | ProjectileKind::Potion(..)
        | ProjectileKind::ExperienceBottle
        | ProjectileKind::LlamaSpit
        | ProjectileKind::ShulkerBullet { .. }
        | ProjectileKind::Thrown(_) => true,
        ProjectileKind::Arrow { .. } => p.arrow.is_some(),
        // Not written yet: tridents save from their own list. Wither skulls and
        // bullets are written now.
        ProjectileKind::Trident => false,
    }
}

/// `ThrowableItemProjectile.addAdditionalSaveData`: `Projectile`'s keys and
/// the `Item` it flies as.
pub fn throwable_to_nbt(
    p: &Projectile,
    uuid: [i32; 4],
    owner: Option<[i32; 4]>,
    raw: Option<&Tag>,
) -> Option<Tag> {
    let item = throwable_item(&p.kind)?;
    let base = Base {
        id: p.kind.id(),
        uuid,
        position: p.position,
        motion: p.velocity,
        rotation: [p.yaw, p.pitch],
        on_ground: p.on_ground,
        fall_distance: p.fall_distance,
    };
    let mut tag = base.tag(raw);
    put_projectile(&mut tag, p, owner);
    put_projectile_entity_fields(&mut tag, p);
    // Entity.saveWithoutId writes NoGravity only when set.
    if p.no_gravity {
        put(&mut tag, "NoGravity", Tag::Byte(1));
    }
    // the loaded item stays (blue egg, potion effects) at `setItem`'s count 1.
    if let Tag::Compound(m) = &mut tag {
        let st = match (&p.kind, &p.item) {
            (ProjectileKind::Potion(s, _), _) => (**s).clone(),
            (_, Some(thrown)) => thrown.clone(),
            _ => crate::items::ItemStack::new(item, 1),
        };
        if let Tag::Compound(i) = m.entry("Item".into()).or_insert_with(|| item_to_nbt(&st)) {
            i.insert("count".into(), Tag::Int(1));
        }
    }
    Some(tag)
}

/// `ThrowableItemProjectile.readAdditionalSaveData` for the kinds
/// [`throwable_to_nbt`] writes; the owner is the player when it is theirs.
pub fn throwable_from_nbt(tag: &Tag, player: Option<[i32; 4]>) -> Option<Projectile> {
    let mut kind = throwable_kind(&kind(tag)?)?;
    if let ProjectileKind::Potion(stack, _) = &mut kind
        && let Some(item) = tag.get("Item").and_then(item_from_nbt)
    {
        **stack = item;
        stack.count = 1;
    }
    let owner = owner_uuid(tag);
    let from_player = player.is_some() && owner == player;
    let mut p = Projectile::new(kind, position(tag)?, from_player.then_some(Who::Player));
    p.velocity = motion(tag);
    [p.yaw, p.pitch] = rotation(tag);
    p.left_owner = flag(tag, "LeftOwner");
    p.has_been_shot = flag(tag, "HasBeenShot");
    p.owner_uuid = owner;
    p.no_gravity = flag(tag, "NoGravity");
    load_projectile_entity_fields(&mut p, tag);
    p.on_ground = flag(tag, "OnGround");
    if !matches!(p.kind, ProjectileKind::Potion(..))
        && let Some(item) = tag.get("Item").and_then(item_from_nbt)
    {
        // Kept on the projectile: the tag it was loaded from is found by its
        // position, so it is gone once the projectile has moved.
        p.set_thrown_item(&item);
    }
    Some(p)
}

/// `LlamaSpit` and `ShulkerBullet` as `/summon` or a chunk loads them, with
/// `ShulkerBullet.readAdditionalSaveData`'s `Dir`, `Steps` and per-axis target
/// deltas: `Projectile`'s `Owner`, `LeftOwner`, `NoGravity` and the motion.
/// `target` is the entity the caller resolved from `Target`; a bullet loaded
/// without one falls (`ShulkerBullet.tick`).
pub fn spit_or_bullet_from_nbt(
    tag: &Tag,
    player: Option<[i32; 4]>,
    target: Option<Who>,
) -> Option<Projectile> {
    let at = position(tag)?;
    let kind = match kind(tag)?.as_str() {
        "minecraft:llama_spit" => ProjectileKind::LlamaSpit,
        "minecraft:shulker_bullet" => ProjectileKind::ShulkerBullet {
            target,
            // `Dir`: `Direction.LEGACY_ID_CODEC`, the step the bullet is on,
            // absent while it has not chosen one. A byte read through
            // `Direction.from3DDataValue`, which wraps it: `abs(d % 6)`.
            dir: number(tag.get("Dir")).map(|d| ((d as i8) % 6).unsigned_abs()),
            // `getIntOr("Steps", 0)`: any int, a negative one included.
            steps: number(tag.get("Steps")).unwrap_or(0) as i32,
            delta: [
                float(tag.get("TXD")).unwrap_or(0.0),
                float(tag.get("TYD")).unwrap_or(0.0),
                float(tag.get("TZD")).unwrap_or(0.0),
            ],
            // `Entity.random` is not saved: a loaded bullet gets a stream of
            // its own from where it is, as `Projectile::new` does for an
            // arrow.
            seed: (at.x.to_bits() ^ at.z.to_bits().rotate_left(17)) | 1,
        },
        _ => return None,
    };
    let owner = owner_uuid(tag);
    let from_player = player.is_some() && owner == player;
    let mut p = Projectile::new(kind, at, from_player.then_some(Who::Player));
    p.velocity = motion(tag);
    [p.yaw, p.pitch] = rotation(tag);
    p.owner_uuid = owner;
    projectile_fields::load_projectile_flags_and_entity_fields(&mut p, tag);
    p.on_ground = flag(tag, "OnGround");
    Some(p)
}

/// The save of [`spit_or_bullet_from_nbt`]'s kinds: `Projectile`'s keys, and
/// for a bullet the `Target`, `Dir`, `Steps` and target deltas vanilla writes.
pub fn spit_or_bullet_to_nbt(
    p: &Projectile,
    uuid: [i32; 4],
    owner: Option<[i32; 4]>,
    target: Option<[i32; 4]>,
    raw: Option<&Tag>,
) -> Option<Tag> {
    if !matches!(
        p.kind,
        ProjectileKind::LlamaSpit | ProjectileKind::ShulkerBullet { .. }
    ) {
        return None;
    }
    let base = Base {
        id: p.kind.id(),
        uuid,
        position: p.position,
        motion: p.velocity,
        rotation: [p.yaw, p.pitch],
        on_ground: p.on_ground,
        fall_distance: p.fall_distance,
    };
    let mut tag = base.tag(raw);
    put_projectile(&mut tag, p, owner);
    put_projectile_entity_fields(&mut tag, p);
    // `ShulkerBullet.addAdditionalSaveData`: the target it homes on (written
    // only while it has one), the step it is on (omitted while it has not
    // chosen one), the steps left and the per-axis target deltas.
    if let ProjectileKind::ShulkerBullet {
        dir, steps, delta, ..
    } = &p.kind
    {
        if let Tag::Compound(map) = &mut tag {
            map.remove(&"Target".into());
            map.remove(&"Dir".into());
        }
        if let Some(target) = target {
            put(&mut tag, "Target", uuid_tag(target));
        }
        if let Some(dir) = dir {
            put(&mut tag, "Dir", Tag::Byte(*dir as i8));
        }
        put(&mut tag, "Steps", Tag::Int(*steps));
        for (key, v) in [("TXD", delta[0]), ("TYD", delta[1]), ("TZD", delta[2])] {
            put(&mut tag, key, Tag::Double(v.to_bits()));
        }
    }
    Some(tag)
}

/// `ShulkerBullet.readAdditionalSaveData`'s `Target`: the UUID of the entity
/// the bullet homes on, which vanilla keeps even while that entity is away.
pub fn bullet_target(tag: &Tag) -> Option<[i32; 4]> {
    uuid_from_tag(tag.get("Target"))
}

/// `Projectile`'s `Owner`.
pub fn owner_uuid(tag: &Tag) -> Option<[i32; 4]> {
    uuid_from_tag(tag.get("Owner"))
}

/// `Fireball.readAdditionalSaveData` (and, around it, `Projectile`'s Owner and
/// LeftOwner and `AbstractHurtingProjectile`'s acceleration power): a fireball
/// also reads the item it is drawn as through `setItem`, whose default is the
/// fire charge; a wind charge is no `Fireball` and has none.
pub fn fireball_from_nbt(tag: &Tag, player: Option<[i32; 4]>) -> Option<Projectile> {
    let kind = match kind(tag)?.as_str() {
        "minecraft:fireball" => ProjectileKind::LargeFireball {
            power: i32::from(number(tag.get("ExplosionPower")).unwrap_or(1) as i8),
        },
        "minecraft:small_fireball" => ProjectileKind::SmallFireball,
        "minecraft:dragon_fireball" => ProjectileKind::DragonFireball,
        "minecraft:wind_charge" => ProjectileKind::WindCharge,
        "minecraft:breeze_wind_charge" => ProjectileKind::BreezeWindCharge,
        // WitherSkull.readAdditionalSaveData: an absent `dangerous` is a black skull.
        "minecraft:wither_skull" => ProjectileKind::WitherSkull {
            dangerous: flag(tag, "dangerous"),
        },
        _ => return None,
    };
    let owner = uuid_from_tag(tag.get("Owner"));
    let from_player = player.is_some() && owner == player;
    let mut p = Projectile::new(kind, position(tag)?, from_player.then_some(Who::Player));
    p.velocity = motion(tag);
    [p.yaw, p.pitch] = rotation(tag);
    p.left_owner = flag(tag, "LeftOwner");
    p.has_been_shot = flag(tag, "HasBeenShot");
    p.owner_uuid = owner;
    p.no_gravity = flag(tag, "NoGravity");
    if projectile_fields::keeps_entity_fields(&p) {
        load_projectile_entity_fields(&mut p, tag);
    }
    p.on_ground = flag(tag, "OnGround");
    p.fall_distance = float(tag.get("fall_distance")).unwrap_or(0.0);
    p.acceleration = float(tag.get("acceleration_power")).unwrap_or(0.1);
    if let Some(item) = tag.get("Item").and_then(item_from_nbt) {
        p.set_item(item);
    }
    Some(p)
}

mod names;
/// `FireworkRocketEntity.readAdditionalSaveData`: Life, LifeTime (0 when
/// absent: a bare `/summon` explodes at once), FireworksItem and
/// ShotAtAngle, with `Projectile`'s Owner and LeftOwner.
pub struct RocketNbt {
    pub position: Vec3,
    pub motion: Vec3,
    pub life: i32,
    pub lifetime: i32,
    pub item: Option<crate::items::ItemStack>,
    pub angled: bool,
    pub owner: Option<Who>,
    /// The saved `Owner`, whoever it names.
    pub owner_uuid: Option<[i32; 4]>,
    pub left_owner: bool,
    pub on_ground: bool,
    /// `Entity.load`'s Rotation.
    pub rotation: [f32; 2],
    /// `Projectile.hasBeenShot`: false until its first tick posts `projectile_shoot`.
    pub has_been_shot: bool,
    /// `Entity.isSilent`: no launch sound.
    pub silent: bool,
    /// `Entity.load`'s fall_distance.
    pub fall_distance: f64,
}
pub fn firework_rocket_from_nbt(tag: &Tag, player: Option<[i32; 4]>) -> Option<RocketNbt> {
    let owner = uuid_from_tag(tag.get("Owner"));
    Some(RocketNbt {
        position: position(tag)?,
        motion: motion(tag),
        life: number(tag.get("Life")).unwrap_or(0) as i32,
        lifetime: number(tag.get("LifeTime")).unwrap_or(0) as i32,
        item: tag.get("FireworksItem").and_then(item_from_nbt),
        angled: flag(tag, "ShotAtAngle"),
        owner: (player.is_some() && owner == player).then_some(Who::Player),
        owner_uuid: owner,
        left_owner: flag(tag, "LeftOwner"),
        on_ground: flag(tag, "OnGround"),
        rotation: rotation(tag),
        has_been_shot: flag(tag, "HasBeenShot"),
        silent: flag(tag, "Silent"),
        fall_distance: float(tag.get("fall_distance")).unwrap_or(0.0),
    })
}

/// What `FireworkRocketEntity.addAdditionalSaveData` writes, over `Projectile`'s
/// Owner and LeftOwner.
pub struct RocketSave<'a> {
    pub rotation: [f32; 2],
    pub position: Vec3,
    pub motion: Vec3,
    pub life: i32,
    pub lifetime: i32,
    pub item: &'a crate::items::ItemStack,
    pub angled: bool,
    pub owner: Option<[i32; 4]>,
    pub left_owner: bool,
    /// `Entity.onGround`, from the rocket's last `Entity.move`.
    pub on_ground: bool,
    pub has_been_shot: bool,
    /// `Entity.fallDistance`.
    pub fall_distance: f64,
}
pub fn firework_rocket_to_nbt(r: &RocketSave, uuid: [i32; 4], raw: Option<&Tag>) -> Tag {
    let mut tag = Base {
        id: "minecraft:firework_rocket",
        uuid,
        position: r.position,
        motion: r.motion,
        rotation: r.rotation,
        on_ground: r.on_ground,
        fall_distance: r.fall_distance,
    }
    .tag(raw);
    for (key, value) in [
        ("Life", Tag::Int(r.life)),
        ("LifeTime", Tag::Int(r.lifetime)),
        ("FireworksItem", item_to_nbt(r.item)),
        ("ShotAtAngle", Tag::Byte(i8::from(r.angled))),
        ("HasBeenShot", Tag::Byte(i8::from(r.has_been_shot))),
    ] {
        put(&mut tag, key, value);
    }
    if r.left_owner {
        put(&mut tag, "LeftOwner", Tag::Byte(1));
    }
    if let Some(owner) = r.owner {
        put(&mut tag, "Owner", Tag::IntArray(owner.to_vec()));
    }
    tag
}

mod trident;
pub use trident::{TRIDENT_HIT_GROUND, TridentNbt, TridentSave, trident_from_nbt, trident_to_nbt};
mod hanging;
pub use hanging::*;

/// `FallingBlockEntity`: BlockState, Time, DropItem, HurtEntities.
#[allow(clippy::too_many_arguments)]
pub fn falling_block_to_nbt(
    state: Tag,
    position: Vec3,
    motion: Vec3,
    time: i32,
    drop_item: bool,
    damage: crate::falling_block::Damage,
    fall_distance: f64,
    on_ground: bool,
    rotation: [f32; 2],
    uuid: [i32; 4],
) -> Tag {
    let base = Base {
        id: "minecraft:falling_block",
        uuid,
        position,
        motion,
        rotation,
        on_ground,
        fall_distance,
    };
    let mut tag = base.tag(None);
    for (key, value) in [
        ("BlockState", state),
        ("Time", Tag::Int(time)),
        ("DropItem", Tag::Byte(i8::from(drop_item))),
    ] {
        put(&mut tag, key, value);
    }
    damage.save(&mut tag);
    tag
}
/// (block state tag, position, motion, time, fall distance)
pub fn falling_block_from_nbt(tag: &Tag) -> Option<(Tag, Vec3, Vec3, i32, f64)> {
    Some((
        tag.get("BlockState")
            .cloned()
            .unwrap_or_else(|| string("minecraft:sand")),
        position(tag)?,
        motion(tag),
        number(tag.get("Time")).unwrap_or(0) as i32,
        float(tag.get("fall_distance")).unwrap_or(0.0),
    ))
}

/// FallingBlockEntity.readAdditionalSaveData uses the modern state codec.
pub fn falling_state_from_nbt(registry: &BlockRegistry, tag: &Tag) -> Option<BlockStateId> {
    BlockState::from_nbt(tag)
        .ok()
        .and_then(|mut state| {
            state.name = horizon_core::identifier::Identifier::parse(&state.name)
                .ok()?
                .to_string();
            registry
                .id(&state)
                .or_else(|| registry.default_state(&state.name))
        })
        .or_else(|| registry.default_state("minecraft:sand"))
}

/// `NoGravity` and `DropItem` (true when absent) of a saved falling block.
pub fn falling_block_flags(tag: &Tag) -> (bool, bool) {
    (flag(tag, "NoGravity"), flag_or(tag, "DropItem", true))
}

/// `Cushion.addAdditionalSaveData`: the block it sits on and its colour.
pub fn cushion_to_nbt(c: &Cushion, uuid: [i32; 4], raw: Option<&Tag>) -> Tag {
    // A block-attached cushion does not update these: they stay as loaded.
    let (on_ground, fall_distance) = raw.map_or((false, 0.0), ground_from_nbt);
    let base = Base {
        id: "minecraft:cushion",
        uuid,
        position: c.position,
        motion: Vec3::ZERO,
        // The cushion never turns: its saved pitch is the one it was loaded with.
        rotation: [c.yaw, raw.map_or(0.0, |t| loaded_rotation(t)[1])],
        on_ground,
        fall_distance,
    };
    let mut tag = base.tag(raw);
    put(&mut tag, "block_pos", Tag::IntArray(c.pos.to_vec()));
    put(&mut tag, "color", string(c.color_name()));
    if let Some(name) = &c.custom_name {
        put(&mut tag, "CustomName", name.clone());
    }
    tag
}
/// The position, block position, yaw and colour of a saved cushion.
pub fn cushion_from_nbt(tag: &Tag, id: u32) -> Option<Cushion> {
    let position = position(tag)?;
    let pos = match tag.get("block_pos") {
        Some(Tag::IntArray(v)) if v.len() == 3 => [v[0], v[1], v[2]],
        _ => [
            position.x.floor() as i32,
            position.y.floor() as i32,
            position.z.floor() as i32,
        ],
    };
    // `Entity.load`: the yaw `% 360` (a cushion keeps no pitch of its own: the saved one is read as loaded).
    let yaw = loaded_rotation(tag)[0];
    let color =
        crate::entities::dye_index(&text(tag.get("color")).unwrap_or_else(|| "white".into()));
    Some(Cushion {
        attachment_lifetime: Default::default(),
        join: Default::default(),
        id,
        position,
        pos,
        yaw,
        color,
        check_interval: 0,
        custom_name: tag.get("CustomName").cloned(),
    })
}

#[cfg(test)]
mod cushion_tests;
/// `ItemFrame.setDirection`: a frame on a wall turns to face away from it
/// (`yRot` = the 2D data value times 90, `xRot` 0); one on a floor or a
/// ceiling looks straight down or up (`xRot` = -90 times the axis step).
/// `facing` is the 3D data value (down, up, north, south, west, east).
fn frame_rotation(facing: u8) -> [f32; 2] {
    match facing {
        0 => [0.0, 90.0],
        1 => [0.0, -90.0],
        2 => [180.0, 0.0],
        3 => [0.0, 0.0],
        4 => [90.0, 0.0],
        _ => [270.0, 0.0],
    }
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
        motion: raw.map_or(Vec3::ZERO, loaded_motion),
        rotation: frame_rotation(f.facing),
        on_ground: raw.is_some_and(|raw| flag(raw, "OnGround")),
        fall_distance: raw.map_or(0.0, |raw| ground_from_nbt(raw).1),
    };
    let mut tag = base.tag(raw);
    // `Entity.saveWithoutId`: `CustomName` before the frame's own fields.
    if let Some(name) = &f.custom_name {
        put(&mut tag, "CustomName", name.clone());
    } else if let Tag::Compound(map) = &mut tag {
        map.remove(&"CustomName".into());
    }
    put(&mut tag, "Facing", Tag::Byte(f.facing as i8));
    put(&mut tag, "block_pos", Tag::IntArray(f.pos.to_vec()));
    put(&mut tag, "ItemRotation", Tag::Byte(f.rotation));
    put(
        &mut tag,
        "ItemDropChance",
        Tag::Float(f.drop_chance.to_bits()),
    );
    put(&mut tag, "Invisible", Tag::Byte(i8::from(f.invisible)));
    put(&mut tag, "Fixed", Tag::Byte(i8::from(f.fixed)));
    if let Tag::Compound(map) = &mut tag {
        map.remove(&horizon_core::nbt::JavaString::from("Item"));
    }
    if let Some(item) = &f.item {
        put(&mut tag, "Item", item_to_nbt(item));
    }
    tag
}
/// `ItemFrame.readAdditionalSaveData`: an absent Facing defaults to Direction.DOWN.
pub fn item_frame_from_nbt(tag: &Tag, id: u32) -> Option<ItemFrame> {
    let pos = hanging::attached_block_pos(tag)?;
    Some(ItemFrame {
        attachment_lifetime: Default::default(),
        join: Default::default(),
        id,
        pos,
        facing: hanging::wrap_data_value(number(tag.get("Facing")).unwrap_or(0), 6),
        glow: kind(tag).as_deref() == Some("minecraft:glow_item_frame"),
        item: tag.get("Item").and_then(item_from_nbt),
        // `TagValueInput.getByteOr` narrows before `ItemFrame.setRotation`'s remainder.
        rotation: (tag.get("ItemRotation").and_then(int_value).unwrap_or(0) as i8) % 8,
        drop_chance: float(tag.get("ItemDropChance")).map_or(1.0, |v| v as f32),
        fixed: flag(tag, "Fixed"),
        invisible: flag(tag, "Invisible"),
        check_interval: 0,
        custom_name: tag.get("CustomName").cloned(),
        random: horizon_core::random::EntityRandom::own(crate::random_source::entity()),
    })
}
/// ArmorStand.addAdditionalSaveData with LivingEntity's `equipment`.
pub fn armor_stand_to_nbt(a: &ArmorStand, uuid: [i32; 4], raw: Option<&Tag>) -> Tag {
    let base = Base {
        id: "minecraft:armor_stand",
        uuid,
        position: a.position,
        motion: a.velocity,
        rotation: [a.yaw, a.pitch],
        on_ground: a.on_ground,
        fall_distance: a.enchantment_fall_distance,
    };
    let mut tag = base.non_mob_tag(raw);
    // `equipment` is saved only when something is worn, `Marker` only when set.
    if let Tag::Compound(map) = &mut tag {
        a.equipment.store(map);
    }
    put(&mut tag, "DisabledSlots", Tag::Int(a.disabled_slots));
    for (key, value) in [
        ("Small", a.small),
        ("ShowArms", a.show_arms),
        ("NoBasePlate", a.no_base_plate),
        ("Invisible", a.invisible),
    ] {
        put(&mut tag, key, Tag::Byte(i8::from(value)));
    }
    if a.marker {
        put(&mut tag, "Marker", Tag::Byte(1));
    } else if let Tag::Compound(map) = &mut tag {
        map.remove(&"Marker".into());
    }
    // Entity.saveWithoutId writes NoGravity only when set.
    if a.no_gravity {
        put(&mut tag, "NoGravity", Tag::Byte(1));
    } else if let Tag::Compound(map) = &mut tag {
        map.remove(&"NoGravity".into());
    }
    if a.name_visible {
        put(&mut tag, "CustomNameVisible", Tag::Byte(1));
    } else if let Tag::Compound(map) = &mut tag {
        map.remove(&"CustomNameVisible".into());
    }
    // A name from the save keeps its own shape unless it changed.
    if let Some(name) = &a.custom_name
        && !tag.get("CustomName").is_some_and(|t| name_text(t) == *name)
    {
        put(
            &mut tag,
            "CustomName",
            crate::scoreboard::json_to_tag(&name.component()),
        );
    }
    put(&mut tag, "Health", Tag::Float(a.health.to_bits()));
    put(&mut tag, "Fire", Tag::Short(a.fire_ticks as i16));
    // ArmorStand.addAdditionalSaveData: the `Pose` parts that are not default.
    let pose: Vec<(&str, Tag)> = ArmorStand::POSE_KEYS
        .into_iter()
        .zip(a.pose)
        .zip(ArmorStand::DEFAULT_POSE)
        .filter(|((_, rotation), default)| rotation != default)
        .map(|((key, rotation), _)| (key, floats(&rotation)))
        .collect();
    put(&mut tag, "Pose", Tag::compound(pose));
    // LivingEntity.addAdditionalSaveData and ArmorStand's own fields.
    if let Tag::Compound(map) = &mut tag {
        for (key, value) in [
            ("HurtTime", Tag::Short(0)),
            ("DeathTime", Tag::Short(0)),
            ("AbsorptionAmount", Tag::Float(0)),
            ("FallFlying", Tag::Byte(0)),
            ("current_impulse_context_reset_grace_time", Tag::Int(0)),
            ("attributes", Tag::List(Vec::new())),
            (
                "Brain",
                Tag::compound([("memories", Tag::Compound(Compound::new()))]),
            ),
            ("DisabledSlots", Tag::Int(0)),
            ("Pose", Tag::Compound(Compound::new())),
        ] {
            map.entry(key.into()).or_insert(value);
        }
        // `removeFrost`'s first-tick movement_speed, over a saved list without it.
        if a.speed_instance
            && let Some(Tag::List(list)) = map.get_mut(&"attributes".into())
            && !list
                .iter()
                .any(|t| text(t.get("id")).as_deref() == Some(SPEED_ATTRIBUTE))
        {
            list.push(Tag::compound([
                ("id", string(SPEED_ATTRIBUTE)),
                ("base", Tag::Double(0.7f64.to_bits())),
            ]));
        }
        put_live_attributes(map, &a.live);
    }
    tag
}
/// The attribute every living entity's first tick makes (`removeFrost`).
const SPEED_ATTRIBUTE: &str = "minecraft:movement_speed";
/// Vanilla 26.3 `Entity.setXRot`: finite pitch is clamped after modulo 360,
/// non-finite pitch resets to 0.0.
fn normalize_pitch(pitch: f32) -> f32 {
    if pitch.is_finite() {
        (pitch % 360.0).clamp(-90.0, 90.0)
    } else {
        0.0
    }
}

pub fn armor_stand_from_nbt(tag: &Tag, id: u32) -> Option<ArmorStand> {
    let rot = rotation(tag);
    let mut a = ArmorStand::new(id, position(tag)?, rot[0]);
    a.pitch = normalize_pitch(rot[1]);
    a.enchantment_fall_distance = float(tag.get("fall_distance")).unwrap_or(0.0);
    // AttributeMap.load makes an instance for each saved attribute.
    a.speed_instance = matches!(tag.get("attributes"), Some(Tag::List(list))
        if list.iter().any(|attr| text(attr.get("id")).as_deref() == Some(SPEED_ATTRIBUTE)));
    a.velocity = loaded_motion(tag);
    a.health = float(tag.get("Health")).unwrap_or(20.0) as f32;
    a.fire_ticks = i32::from(number(tag.get("Fire")).unwrap_or(0) as i16);
    a.custom_name = tag.get("CustomName").map(name_text);
    a.name_visible = flag(tag, "CustomNameVisible");
    if let Some(eq) = tag.get("equipment") {
        a.equipment = horizon_items::equipment::Equipment::read(eq);
    }
    a.live.load(tag.get("attributes"), |_| true);
    a.small = flag(tag, "Small");
    a.show_arms = flag(tag, "ShowArms");
    a.disabled_slots = number(tag.get("DisabledSlots")).unwrap_or(0) as i32;
    a.no_base_plate = flag(tag, "NoBasePlate");
    a.invisible = flag(tag, "Invisible");
    a.marker = flag(tag, "Marker");
    a.no_gravity = flag(tag, "NoGravity");
    a.on_ground = flag(tag, "OnGround");
    // ArmorStand.readAdditionalSaveData: each part of `Pose` that is a list of
    // three numbers, the others staying at their defaults.
    if let Some(pose) = tag.get("Pose") {
        for (rotation, key) in a.pose.iter_mut().zip(ArmorStand::POSE_KEYS) {
            if let Some(Tag::List(values)) = pose.get(key)
                && values.len() == 3
            {
                for (r, v) in rotation.iter_mut().zip(values) {
                    *r = float(Some(v)).unwrap_or(0.0) as f32;
                }
            }
        }
    }
    Some(a)
}
/// `AbstractBoat.addAdditionalSaveData` / `ContainerEntity.addChestVehicleSaveData`:
/// only chest boats own container fields; a pending table replaces `Items`.
/// Ordinary slots use `ContainerHelper.saveAllItems` through the existing item encoder.
/// `ground` is the live `OnGround` and `fall_distance` (`Entity.saveWithoutId`).
#[allow(clippy::too_many_arguments)]
pub fn boat_to_nbt(
    id: &str,
    position: Vec3,
    motion: Vec3,
    rotation: [f32; 2],
    ground: (bool, f64),
    uuid: [i32; 4],
    raw: Option<&Tag>,
    items: Option<&[Option<crate::items::ItemStack>]>,
) -> Tag {
    let mut tag = Base {
        id,
        uuid,
        position,
        motion,
        rotation,
        on_ground: ground.0,
        fall_distance: ground.1,
    }
    .non_mob_tag(raw);
    if let Tag::Compound(map) = &mut tag {
        map.remove(&"Items".into());
        if let Some(items) = items {
            let table = crate::save::text(map.get(&"LootTable".into()))
                .and_then(|id| horizon_client_core::identifier::normalize_id(&id));
            if let Some(table) = table {
                map.insert("LootTable".into(), Tag::String(table.as_str().into()));
            } else {
                map.remove(&"LootTable".into());
                map.remove(&"LootTableSeed".into());
                map.insert("Items".into(), crate::save::encode_items(items));
            }
        } else {
            map.remove(&"LootTable".into());
            map.remove(&"LootTableSeed".into());
        }
    }
    tag
}
/// A vehicle's base entity tag (minecarts, the dragon and end crystals), with
/// the live `OnGround` and `fall_distance` in `ground`.
pub fn vehicle_to_nbt(
    id: &str,
    position: Vec3,
    motion: Vec3,
    rotation: [f32; 2],
    ground: (bool, f64),
    uuid: [i32; 4],
    raw: Option<&Tag>,
) -> Tag {
    Base {
        id,
        uuid,
        position,
        motion,
        rotation,
        on_ground: ground.0,
        fall_distance: ground.1,
    }
    .tag(raw)
}
/// `Entity.load`'s `OnGround` (`getBooleanOr`: `byteValue() != 0`) and
/// `fall_distance` (`getDoubleOr`: any number's `doubleValue`), defaulting to
/// false and 0.
pub fn ground_from_nbt(tag: &Tag) -> (bool, f64) {
    (
        flag(tag, "OnGround"),
        float(tag.get("fall_distance")).unwrap_or(0.0),
    )
}

use crate::mobs::villager::professions::save::villager_data;
/// `AbstractVillager` "Offers" (`MerchantOffers.CODEC`).
fn put_offers(map: &mut Compound, m: &Mob) {
    let Some(t) = m.extra.trade.as_ref().filter(|t| !t.unset) else {
        return;
    };
    map.insert("Offers".into(), offers_to_nbt(&t.offers));
}
/// `ItemCost.CODEC`: id, count (always written) and the exact components.
fn item_cost_to_nbt(stack: &crate::items::ItemStack) -> Tag {
    let mut tag = item_to_nbt(stack);
    if let Tag::Compound(c) = &mut tag
        && !c.contains_key(&"count".into())
    {
        c.insert("count".into(), Tag::Int(stack.count as i32));
    }
    tag
}
/// `MerchantOffers.CODEC`: `{Recipes: [...]}`, each offer without the
/// fields at their defaults.
pub fn offers_to_nbt(offers: &[crate::merchant::MerchantOffer]) -> Tag {
    let recipes = offers
        .iter()
        .map(|o| {
            let mut entries = vec![
                ("buy", item_cost_to_nbt(&o.base_cost_a.stack)),
                ("sell", item_to_nbt(&o.result)),
            ];
            // `MerchantOffer.CODEC` omits fields at their defaults.
            if o.uses != 0 {
                entries.push(("uses", Tag::Int(o.uses)));
            }
            if o.max_uses != 4 {
                entries.push(("maxUses", Tag::Int(o.max_uses)));
            }
            if o.price_multiplier != 0.0 {
                entries.push(("priceMultiplier", Tag::Float(o.price_multiplier.to_bits())));
            }
            if o.xp != 1 {
                entries.push(("xp", Tag::Int(o.xp)));
            }
            if !o.reward_exp {
                entries.push(("rewardExp", Tag::Byte(0)));
            }
            if o.special_price_diff != 0 {
                entries.push(("specialPrice", Tag::Int(o.special_price_diff)));
            }
            if o.demand != 0 {
                entries.push(("demand", Tag::Int(o.demand)));
            }
            if let Some(b) = &o.cost_b {
                entries.push(("buyB", item_cost_to_nbt(&b.stack)));
            }
            Tag::compound(entries)
        })
        .collect::<Vec<_>>();
    // An empty list is the field's default: left out.
    if recipes.is_empty() {
        return Tag::Compound(Compound::new());
    }
    Tag::compound([("Recipes", Tag::List(recipes))])
}
/// The offers of a `MerchantOffers.CODEC` tag.
pub fn offers_from_nbt(tag: &Tag) -> Vec<crate::merchant::MerchantOffer> {
    use crate::merchant::{ItemCost, MerchantOffer};
    let Some(Tag::List(recipes)) = tag.get("Recipes") else {
        return Vec::new();
    };
    recipes
        .iter()
        .filter_map(|r| {
            let cost = |key| {
                r.get(key)
                    .and_then(item_from_nbt)
                    .map(|stack| ItemCost { stack })
            };
            Some(MerchantOffer {
                base_cost_a: cost("buy")?,
                cost_b: cost("buyB"),
                result: r.get("sell").and_then(item_from_nbt)?,
                uses: number(r.get("uses")).unwrap_or(0) as i32,
                max_uses: number(r.get("maxUses")).unwrap_or(4) as i32,
                reward_exp: number(r.get("rewardExp")).unwrap_or(1) != 0,
                special_price_diff: number(r.get("specialPrice")).unwrap_or(0) as i32,
                demand: number(r.get("demand")).unwrap_or(0) as i32,
                price_multiplier: float(r.get("priceMultiplier")).unwrap_or(0.0) as f32,
                xp: number(r.get("xp")).unwrap_or(1) as i32,
            })
        })
        .collect()
}
/// The villager brain's `job_site`, `home` and `meeting_point` memories.
fn put_memories(map: &mut Compound, m: &Mob) {
    let mut memories: Vec<(&str, Tag)> = [
        ("minecraft:job_site", m.extra.home),
        ("minecraft:home", m.extra.bed),
        ("minecraft:meeting_point", m.extra.meeting),
    ]
    .into_iter()
    .filter_map(|(key, p)| {
        let pos = Tag::compound([
            ("dimension", string("minecraft:overworld")),
            ("pos", Tag::IntArray(p?.to_vec())),
        ]);
        Some((key, Tag::compound([("value", pos)])))
    })
    .collect();
    crate::mobs::villager::write_memories(m, &mut memories);
    map.insert(
        "Brain".into(),
        Tag::compound([("memories", Tag::compound(memories))]),
    );
}
fn read_villager(m: &mut Mob, tag: &Tag) {
    use crate::{merchant::Trader, mobs::render::VILLAGER_TYPES};
    if let Some(data) = tag.get("VillagerData") {
        let name = |key| text(data.get(key)).map(|s| s.trim_start_matches("minecraft:").to_owned());
        if let Some(t) = name("type").and_then(|t| VILLAGER_TYPES.iter().position(|v| *v == t)) {
            m.extra.variant = t as u8;
        }
        crate::mobs::villager::professions::save::read_identity(m, data);
        m.extra.villager_level = number(data.get("level"))
            .unwrap_or(1)
            .clamp(1, i64::from(i32::MAX)) as i32;
    }
    let memory = |key: &str| {
        tag.get("Brain")
            .and_then(|b| b.get("memories"))
            .and_then(|b| b.get(key))
            .and_then(|j| j.get("value"))
            .and_then(|v| match v.get("pos") {
                Some(Tag::IntArray(a)) if a.len() == 3 => Some([a[0], a[1], a[2]]),
                _ => None,
            })
    };
    m.extra.home = memory("minecraft:job_site");
    m.extra.bed = memory("minecraft:home");
    m.extra.meeting = memory("minecraft:meeting_point");
    if m.species() == crate::mobs::Species::Villager {
        crate::mobs::villager::read_nbt(m, tag);
    }
    if m.species() == crate::mobs::Species::WanderingTrader {
        // WanderingTrader.readAdditionalSaveData: both default to idle.
        m.extra.ints[5] = number(tag.get("DespawnDelay")).unwrap_or(0) as i32;
        m.extra.point = match tag.get("wander_target") {
            Some(Tag::IntArray(a)) if a.len() == 3 => {
                Some([f64::from(a[0]), f64::from(a[1]), f64::from(a[2])])
            }
            _ => None,
        };
    }
    let offers_tag = tag.get("Offers");
    let xp = number(tag.get("Xp")).unwrap_or(0) as i32;
    if offers_tag.is_none() && xp == 0 && tag.get("LastRestock").is_none() {
        return;
    }
    let offers = offers_tag.map(offers_from_nbt).unwrap_or_default();
    m.extra.trade = Some(Box::new(Trader {
        unset: offers_tag.is_none(),
        xp: number(tag.get("Xp")).unwrap_or(0) as i32,
        offers,
        last_restock: number(tag.get("LastRestock")).unwrap_or(0),
        restocks_today: number(tag.get("RestocksToday")).unwrap_or(0) as i32,
        check_day: 0,
    }));
}

#[cfg(test)]
mod tnt_power_tests;

#[cfg(test)]
mod spell_ticks_nbt_tests;

#[cfg(test)]
mod tests;

/// `Brain`'s two measured Warden Unit memories, using actual existing slots.
fn write_warden_owned_memories(m: &Mob, map: &mut Compound) {
    let brain = map
        .entry("Brain".into())
        .or_insert_with(|| Tag::Compound(Compound::new()));
    let Tag::Compound(brain) = brain else { return };
    let memories = brain
        .entry("memories".into())
        .or_insert_with(|| Tag::Compound(Compound::new()));
    let Tag::Compound(memories) = memories else {
        return;
    };
    let mem = m.extra.warden.as_ref().map(|st| &st.brain.mem);
    for (key, slot) in [
        ("minecraft:is_emerging", mem.map(|mem| &mem.is_emerging)),
        ("minecraft:dig_cooldown", mem.map(|mem| &mem.dig_cooldown)),
    ] {
        if let Some(slot) = slot.filter(|slot| slot.present()) {
            let mut entry = Tag::compound([("value", Tag::Compound(Compound::new()))]);
            if slot.ttl() != i64::MAX {
                put(&mut entry, "ttl", Tag::Long(slot.ttl()));
            }
            memories.insert(key.into(), entry);
        } else {
            memories.remove(&key.into());
        }
    }
}
/// `Brain` / `ExpirableValue`: admitted Unit values with absent or Long TTL.
fn restore_warden_owned_memories(m: &mut Mob, tag: &Tag) {
    let memories = tag.get("Brain").and_then(|brain| brain.get("memories"));
    let Some(st) = m.extra.warden.as_mut() else {
        return;
    };
    for (key, slot) in [
        ("minecraft:is_emerging", &mut st.brain.mem.is_emerging),
        ("minecraft:dig_cooldown", &mut st.brain.mem.dig_cooldown),
    ] {
        let Some(entry) = memories.and_then(|memories| memories.get(key)) else {
            // Fresh Java read/load: absent entries do not retain finalize's slot.
            slot.clear();
            continue;
        };
        if !matches!(entry.get("value"), Some(Tag::Compound(value)) if value.is_empty()) {
            continue; // Unmeasured malformed shapes retain existing initialization.
        }
        match entry.get("ttl") {
            None => slot.set(()),
            Some(Tag::Long(ttl)) => slot.set_ttl((), *ttl),
            _ => {} // No new numeric/malformed TTL policy.
        }
    }
}

fn write_axolotl_memories(m: &Mob, map: &mut Compound) {
    let brain = map
        .entry("Brain".into())
        .or_insert_with(|| Tag::Compound(Compound::new()));
    if !matches!(brain, Tag::Compound(_)) {
        *brain = Tag::Compound(Compound::new());
    }
    let Tag::Compound(brain) = brain else { return };
    let memories = brain
        .entry("memories".into())
        .or_insert_with(|| Tag::Compound(Compound::new()));
    if !matches!(memories, Tag::Compound(_)) {
        *memories = Tag::Compound(Compound::new());
    }
    let Tag::Compound(memories) = memories else {
        return;
    };
    let mm = m.extra.animal_brain.as_deref().map(|b| &b.mem);
    let hunting = mm.and_then(|mem| {
        let slot = &mem.hunting_cooldown;
        slot.value().map(|value| {
            let mut entry = Tag::compound([("value", Tag::Byte(i8::from(value)))]);
            if slot.ttl() != i64::MAX {
                put(&mut entry, "ttl", Tag::Long(slot.ttl()));
            }
            entry
        })
    });
    // ExpirableValue.codec: integer value and optional finite long TTL.
    let integer_memory = |slot: &crate::mobs::animal_brain::Slot<i32>| {
        slot.value().map(|v| {
            let mut entry = Tag::compound([("value", Tag::Int(v))]);
            if slot.ttl() != i64::MAX {
                put(&mut entry, "ttl", Tag::Long(slot.ttl()));
            }
            entry
        })
    };
    let play_dead = mm.and_then(|m| integer_memory(&m.play_dead_ticks));
    let temptation = mm.and_then(|m| integer_memory(&m.temptation_cooldown_ticks));
    for (key, value) in [
        ("minecraft:has_hunting_cooldown", hunting),
        ("minecraft:play_dead_ticks", play_dead),
        ("minecraft:temptation_cooldown_ticks", temptation),
    ] {
        if let Some(value) = value {
            memories.insert(key.into(), value);
        } else {
            memories.remove(&key.into());
        }
    }
}
/// Saved variants and fish/axolotl bucket flags. Missing variants use a
/// medium salmon or index 0; a missing bucket flag is false.
fn read_variant(m: &mut Mob, tag: &Tag) {
    use crate::mobs::Species as S;
    // AbstractFish/Axolotl: false when absent. Tadpole.setFromBucket ignores the input.
    if matches!(
        m.species(),
        S::Cod | S::Salmon | S::Pufferfish | S::TropicalFish | S::Axolotl
    ) {
        m.extra.from_bucket = flag(tag, "FromBucket");
    }
    let int = |key| number(tag.get(key)).unwrap_or(0) as i32;
    match m.species() {
        S::Parrot => m.extra.variant = int("Variant").clamp(0, 4) as u8, // CLAMP
        S::Axolotl => m.extra.variant = ((int("Variant") as u32).min(5) % 5) as u8, // ZERO: lucy
        // Rabbit.Variant.LEGACY_CODEC: an unknown id is brown.
        S::Rabbit => {
            m.extra.variant = match int("RabbitType") {
                v @ (0..=5 | 99) => v as u8,
                _ => 0,
            }
        }
        S::Salmon => {
            m.extra.variant = text(tag.get("type"))
                .and_then(|t| SALMON_SIZES.iter().position(|v| *v == t))
                .unwrap_or(1) as u8;
        }
        S::TropicalFish => {
            let packed = int("Variant");
            let (size, index) = (packed & 0xFF, (packed >> 8) & 0xFF);
            // TropicalFish.Pattern.byId: an unknown id is a kob.
            m.extra.variant = if size <= 1 && index < 6 {
                (size * 6 + index) as u8
            } else {
                0
            };
            // TropicalFish.getBaseColor/getPatternColor use DyeColor.byId:
            // read the full byte, then default invalid color ids to white.
            let color = |shift| match (packed >> shift) & 0xFF {
                value @ 0..=15 => value as u8,
                _ => 0,
            };
            m.extra.variant2 = color(16);
            m.extra.variant3 = color(24);
        }
        _ => {}
    }
}
fn restore_axolotl_memories(m: &mut Mob, tag: &Tag) {
    let memories = tag.get("Brain").and_then(|b| b.get("memories"));
    // Initialize/clear the slot without applying the runtime setter's positive-TTL rule to saved data.
    crate::mobs::animal_brain::set_hunting_cooldown(m, 0);
    if let Some(b) = m.extra.animal_brain.as_mut() {
        if let Some(entry) = memories.and_then(|v| v.get("minecraft:has_hunting_cooldown"))
            && let Some(value) = float(entry.get("value"))
        {
            // NbtOps boolean numbers use doubleValue()!=0; ExpirableValue's optional TTL is lenient.
            let ttl = number(entry.get("ttl")).unwrap_or(i64::MAX);
            b.mem.hunting_cooldown.set_ttl(value != 0.0, ttl);
        }
        for (key, slot) in [
            ("minecraft:play_dead_ticks", &mut b.mem.play_dead_ticks),
            (
                "minecraft:temptation_cooldown_ticks",
                &mut b.mem.temptation_cooldown_ticks,
            ),
        ] {
            if let Some(entry) = memories.and_then(|b| b.get(key))
                && let Some(value) = number(entry.get("value"))
            {
                // ExpirableValue.codec's lenient optional TTL defaults to permanent.
                slot.set_ttl(value as i32, number(entry.get("ttl")).unwrap_or(i64::MAX));
            }
        }
    }
}
/// `Axolotl.Variant` names, by id.
pub const AXOLOTL_VARIANTS: [&str; 5] = ["lucy", "wild", "gold", "cyan", "blue"];
/// `Salmon.Variant` names, by id.
const SALMON_SIZES: [&str; 3] = ["small", "medium", "large"];
/// `TropicalFish.Pattern` names in enum order: the six small patterns, then
/// the six large ones (the order of `extra.variant`).
pub const TROPICAL_PATTERNS: [&str; 12] = [
    "kob",
    "sunstreak",
    "snooper",
    "dasher",
    "brinely",
    "spotty",
    "flopper",
    "stripey",
    "glitter",
    "blockfish",
    "betty",
    "clayfish",
];
/// `Bucketable.saveDefaultDataToBucketTag`, with the age an axolotl and a
/// tadpole add (`saveToBucketTag`).
pub fn bucket_data(m: &Mob) -> Tag {
    let mut data = Tag::compound([("Health", Tag::Float(m.health.to_bits()))]);
    if matches!(
        m.species(),
        crate::mobs::Species::Axolotl | crate::mobs::Species::Tadpole
    ) {
        put(&mut data, "Age", Tag::Int(m.extra.age));
        put(
            &mut data,
            "AgeLocked",
            Tag::Byte(i8::from(m.extra.age_locked)),
        );
    }
    for (key, on) in [
        ("NoAI", m.no_ai),
        ("Silent", m.silent),
        ("NoGravity", m.no_gravity),
        ("Glowing", m.glowing),
        ("Invulnerable", m.invulnerable_flag),
        ("PersistenceRequired", m.persistent),
    ] {
        if on {
            put(&mut data, key, Tag::Byte(1));
        }
    }
    if let Some(ttl) = crate::mobs::animal_brain::hunting_cooldown(m) {
        put(&mut data, "HuntingCooldown", Tag::Long(ttl));
    }
    // SulfurCube.saveToBucketTag: after the default data, its age.
    if m.species() == crate::mobs::Species::SulfurCube {
        put(&mut data, "age", Tag::Int(m.extra.age));
        put(
            &mut data,
            "age_locked",
            Tag::Byte(i8::from(m.extra.age_locked)),
        );
    }
    data
}
pub fn bucket_components(m: &Mob) -> crate::item_data::StackData {
    let mut data = crate::item_data::StackData::default();
    data.custom_name = names::bucket_custom_name(m);
    data.set_other("minecraft:bucket_entity_data", Some(bucket_data(m)));
    // SulfurCube.saveToBucketTag: the body block, when it has one.
    if m.species() == crate::mobs::Species::SulfurCube
        && let Some(body) = m.extra.body_id()
    {
        data.set_other(
            "minecraft:sulfur_cube_content",
            Some(Tag::compound([("id", string(body))])),
        );
    }
    if m.species() == crate::mobs::Species::Axolotl {
        let variant = AXOLOTL_VARIANTS
            .get(usize::from(m.extra.variant))
            .unwrap_or(&"lucy");
        data.set_other("minecraft:axolotl/variant", Some(string(variant)));
    }
    // Salmon.saveToBucketTag: the size.
    if m.species() == crate::mobs::Species::Salmon {
        let size = SALMON_SIZES
            .get(usize::from(m.extra.variant))
            .unwrap_or(&"medium");
        data.set_other("minecraft:salmon/size", Some(string(size)));
    }
    // TropicalFish.saveToBucketTag: the pattern and both colours.
    if m.species() == crate::mobs::Species::TropicalFish {
        let dye = |i: u8| crate::mobs::render::DYE_NAMES[usize::from(i) & 15];
        let pattern = TROPICAL_PATTERNS
            .get(usize::from(m.extra.variant))
            .unwrap_or(&"kob");
        data.set_other("minecraft:tropical_fish/pattern", Some(string(pattern)));
        data.set_other(
            "minecraft:tropical_fish/base_color",
            Some(string(dye(m.extra.variant2))),
        );
        data.set_other(
            "minecraft:tropical_fish/pattern_color",
            Some(string(dye(m.extra.variant3))),
        );
    }
    data
}
/// `Entity.applyComponentsFromItemStack` → the spawned mob's
/// `applyImplicitComponents`: a bucket's contents (`MobBucketItem.spawn`) and
/// a spawn egg (`PostSpawnProcessor.apply`) both land here. Vanilla applies
/// these after `finalizeSpawn`, so a component wins over the biome variant.
pub fn apply_item_components(m: &mut Mob, stack: &crate::items::ItemStack) {
    let Some(components) = stack.data() else {
        return;
    };
    // SulfurCube.applyImplicitComponent(SULFUR_CUBE_CONTENT), before the
    // custom name: setItemSlotAndDropWhenKilled(BODY, the absorbed block).
    if m.species() == crate::mobs::Species::SulfurCube
        && let Some(body) = components
            .other("minecraft:sulfur_cube_content")
            .and_then(item_from_nbt)
    {
        m.extra.set_body_stack(Some(body));
    }
    if let Some(name) = stack.custom_name() {
        m.custom_name = Some(name);
        // Entity.applyImplicitComponent(CUSTOM_NAME): the whole text.
        m.custom_name_text = components.custom_name.as_ref().map(name_text);
    }
    if let Some(data) = components.other("minecraft:bucket_entity_data") {
        if let Some(v) = number(data.get("Age")) {
            m.extra.age = v as i32;
        }
        if let Some(v) = number(data.get("AgeLocked")) {
            m.extra.age_locked = v != 0;
        }
        // Keep old Horizon buckets readable; the modern component below takes precedence.
        if let Some(v) = number(data.get("Variant")) {
            m.extra.variant = v as u8;
        }
        if let Some(v) = float(data.get("Health")) {
            m.health = v as f32;
        }
        for (key, flag) in [
            ("NoAI", &mut m.no_ai),
            ("Silent", &mut m.silent),
            ("NoGravity", &mut m.no_gravity),
            ("Glowing", &mut m.glowing),
            ("Invulnerable", &mut m.invulnerable_flag),
        ] {
            if let Some(v) = number(data.get(key)) {
                *flag = v != 0;
            }
        }
        m.persistent |= flag(data, "PersistenceRequired");
        if m.species() == crate::mobs::Species::Axolotl {
            crate::mobs::animal_brain::set_hunting_cooldown(
                m,
                number(data.get("HuntingCooldown")).unwrap_or(0),
            );
        }
    }
    if m.species() == crate::mobs::Species::Axolotl
        && let Some(name) = text(components.other("minecraft:axolotl/variant"))
        && let Some(id) = AXOLOTL_VARIANTS
            .iter()
            .position(|&v| v == name.trim_start_matches("minecraft:"))
    {
        m.extra.variant = id as u8;
    }
    // Salmon.applyImplicitComponents: the size.
    if m.species() == crate::mobs::Species::Salmon
        && let Some(name) = text(components.other("minecraft:salmon/size"))
        && let Some(id) = SALMON_SIZES
            .iter()
            .position(|&v| v == name.trim_start_matches("minecraft:"))
    {
        m.extra.variant = id as u8;
    }
    // TropicalFish.applyImplicitComponent: the pattern and both colours.
    if m.species() == crate::mobs::Species::TropicalFish {
        let name = |key| text(components.other(key));
        let dye = |key| {
            name(key).and_then(|n| {
                crate::mobs::render::DYE_NAMES
                    .iter()
                    .position(|&v| v == n.trim_start_matches("minecraft:"))
            })
        };
        if let Some(id) = name("minecraft:tropical_fish/pattern").and_then(|n| {
            TROPICAL_PATTERNS
                .iter()
                .position(|&v| v == n.trim_start_matches("minecraft:"))
        }) {
            m.extra.variant = id as u8;
        }
        if let Some(id) = dye("minecraft:tropical_fish/base_color") {
            m.extra.variant2 = id as u8;
        }
        if let Some(id) = dye("minecraft:tropical_fish/pattern_color") {
            m.extra.variant3 = id as u8;
        }
    }
    // Chicken, Cow and Pig.applyImplicitComponents: their variant and sound
    // variant components, both registry names (`minecraft:warm`,
    // `minecraft:picky`).
    let species = m.species();
    let (variant, sound) = match species {
        crate::mobs::Species::Chicken => (
            Some("minecraft:chicken/variant"),
            Some("minecraft:chicken/sound_variant"),
        ),
        crate::mobs::Species::Cow => (
            Some("minecraft:cow/variant"),
            Some("minecraft:cow/sound_variant"),
        ),
        crate::mobs::Species::Pig => (
            Some("minecraft:pig/variant"),
            Some("minecraft:pig/sound_variant"),
        ),
        // ZombieNautilus.applyImplicitComponents: the variant only.
        crate::mobs::Species::ZombieNautilus => (Some("minecraft:zombie_nautilus/variant"), None),
        // Fox and Frog.applyImplicitComponent: `setVariant` from the component
        // alone, so a spawn egg carrying one hatches that variant.
        crate::mobs::Species::Fox => (Some("minecraft:fox/variant"), None),
        crate::mobs::Species::Frog => (Some("minecraft:frog/variant"), None),
        // Horse.applyImplicitComponent: `setVariant` sets the coat colour and
        // keeps the markings the spawn picked (`getTypeVariant() & -256`),
        // which Horizon keeps in `extra.variant2` and leaves alone here.
        crate::mobs::Species::Horse => (Some("minecraft:horse/variant"), None),
        // Llama.applyImplicitComponent, which a trader llama inherits.
        crate::mobs::Species::Llama | crate::mobs::Species::TraderLlama => {
            (Some("minecraft:llama/variant"), None)
        }
        _ => (None, None),
    };
    if let Some(name) = variant.and_then(|id| text(components.other(id))) {
        m.extra.variant = crate::mobs::finalize::variant_index(species, &name);
    }
    if let Some(name) = sound.and_then(|id| text(components.other(id)))
        && let Some(i) = crate::mobs::finalize::sound_variants(species)
            .iter()
            .position(|v| *v == name.trim_start_matches("minecraft:"))
    {
        m.extra.sound_variant = Some(i as u8);
    }
    // MushroomCow.applyImplicitComponents: `Variant.DEFAULT` is red, as the
    // save's `Type` reads it.
    if species == crate::mobs::Species::Mooshroom
        && let Some(name) = text(components.other("minecraft:mooshroom/variant"))
    {
        m.extra.variant = u8::from(name.trim_start_matches("minecraft:") == "brown");
    }
    // Sheep.applyImplicitComponents: the wool colour, a DyeColor id.
    if species == crate::mobs::Species::Sheep
        && let Some(name) = text(components.other("minecraft:sheep/color"))
        && let Some(i) = crate::mobs::render::DYE_NAMES
            .iter()
            .position(|c| *c == name.trim_start_matches("minecraft:"))
    {
        m.extra.color = i as u8;
    }
}

#[cfg(test)]
mod axolotl_nbt_tests;

#[cfg(test)]
mod sheep_nbt_tests;

#[cfg(test)]
mod item_component_tests;

#[cfg(test)]
mod fireball_nbt_tests;

#[cfg(test)]
mod neutral_anger_tests;

#[cfg(test)]
mod uuid_bridge_tests;

#[cfg(test)]
mod warden_anger_tests;

#[cfg(test)]
mod held_item_name_tests;

#[cfg(test)]
mod shulker_bullet_nbt_tests;

#[cfg(test)]
#[path = "spectral_duration_nbt_tests.rs"]
mod spectral_duration_nbt_tests;

#[cfg(test)]
mod mod_thrown_nbt_tests;

#[cfg(test)]
mod equipment_tests;

#[cfg(test)]
mod axolotl_memory_ttl_tests;

#[cfg(test)]
mod generic_home_nbt_tests;

#[cfg(test)]
mod piglin_live_anger_memory_tests;

#[cfg(test)]
mod pickup_loot_save_tests;

#[cfg(test)]
mod stand_fall_save_tests;

#[cfg(test)]
mod animal_love_cause_tests;

#[cfg(test)]
mod carrier_inventory_tests;

/// Added controls (H192): the sulfur cube's saved archetype attributes are
/// its map's base and permanent modifiers, replacing a stale entry of the
/// same id once, never its transient modifiers.
#[cfg(test)]
mod sulfur_attribute_save_tests;

#[cfg(test)]
mod mob_hidden_speed_tests {
    use super::*;
    use crate::mobs::{Species, sensor_scan_tests::kind};
    fn types() -> HashMap<String, Arc<MobType>> {
        let mut pig = (*kind()).clone();
        pig.id = "minecraft:pig".into();
        pig.species = Species::Pig;
        HashMap::from([("minecraft:pig".to_owned(), Arc::new(pig))])
    }
    fn saved(m: &Mob) -> Tag {
        mob_to_nbt(m, [202, 0, 0, 1], None, None, |_| None)
    }
    #[test]
    fn loaded_speed_fallback_survives_a_no_raw_save() {
        let input = horizon_core::nbt::parse_snbt(r#"{id:"minecraft:pig",UUID:[I;202,0,0,1],Pos:[2.5d,200d,4.5d],Health:10f,NoAI:1b,NoGravity:1b,active_effects:[{id:"minecraft:speed",amplifier:1b,duration:40,show_icon:1b,hidden_effect:{duration:200,ambient:1b,show_particles:0b,show_icon:0b}}]}"#).unwrap().0;
        let m = mob_from_nbt(&input, &types(), 7, None).unwrap();
        assert_eq!(m.id, 7);
        assert_eq!(m.kind.id, "minecraft:pig");
        assert!(m.no_ai && m.no_gravity);
        assert_eq!(m.health, 10.0);
        assert_eq!(
            m.extra
                .effects
                .iter()
                .map(|e| (e.id, e.duration, e.amplifier))
                .collect::<Vec<_>>(),
            [("minecraft:speed", 40, 1)]
        );
        let tag = saved(&m);
        let hidden = tag
            .get("active_effects")
            .and_then(|t| match t {
                Tag::List(v) => v.first(),
                _ => None,
            })
            .and_then(|e| e.get("hidden_effect"));
        assert_eq!(
            hidden.and_then(|t| t.get("duration")),
            Some(&Tag::Int(200)),
            "real decoder must retain weaker duration without raw input"
        );
    }
    // Added after the frozen old proof: modeled mutation and stale raw controls.
    #[test]
    fn fallback_model_wins_over_stale_raw_and_clears_without_resurrection() {
        let input = horizon_core::nbt::parse_snbt(r#"{id:"minecraft:pig",Pos:[2.5d,200d,4.5d],Tags:["untouched"],active_effects:[{id:"minecraft:speed",amplifier:1b,duration:40,hidden_effect:{duration:200,ambient:1b,show_particles:0b,show_icon:0b}}]}"#).unwrap().0;
        let mut m = mob_from_nbt(&input, &types(), 7, None).unwrap();
        let h = &m.extra.effect_chains[0];
        assert_eq!(
            (
                h.id,
                h.duration,
                h.amplifier,
                h.ambient,
                h.visible,
                h.show_icon
            ),
            ("minecraft:speed", 200, 0, true, false, false)
        );
        m.extra.effect_chains[0].duration = 123;
        m.extra.effects[0].duration = 30;
        let actual = mob_to_nbt(&m, [202, 0, 0, 1], Some(&input), None, |_| None);
        let first = |t: &Tag| match t.get("active_effects").unwrap() {
            Tag::List(v) => v[0].clone(),
            _ => panic!(),
        };
        assert_eq!(first(&actual).get("duration"), Some(&Tag::Int(30)));
        assert_eq!(
            first(&actual).get("hidden_effect").unwrap().get("duration"),
            Some(&Tag::Int(123))
        );
        assert_eq!(actual.get("Tags"), input.get("Tags"));
        m.extra.effect_chains.clear();
        for raw in [None, Some(&input)] {
            let actual = mob_to_nbt(&m, [202, 0, 0, 1], raw, None, |_| None);
            assert!(first(&actual).get("hidden_effect").is_none());
        }
    }
}

#[cfg(test)]
mod mob_effect_amplifier_codec;

#[cfg(test)]
mod camel_inherited_saved_tests;

#[cfg(test)]
mod equine_temper_preservation_tests;

#[cfg(test)]
mod sulfur_bucket_content_tests {
    use super::*;
    use crate::mobs::Species;

    fn mob(species: Species, id: &str) -> Mob {
        let mut kind = crate::mobs::sensor_scan_tests::kind();
        let ty = Arc::make_mut(&mut kind);
        ty.species = species;
        ty.id = id.into();
        Mob::new(1, kind, Vec3::new(0.5, 64.0, 0.5), 0.0)
    }

    /// `MobBucketItem.spawn` → `SulfurCube.applyImplicitComponent`: a cube
    /// released from its bucket wears the block the bucket recorded
    /// (`minecraft:sulfur_cube_content`) as its body again (#17990).
    #[test]
    fn a_released_sulfur_cube_keeps_the_block_its_bucket_carried() {
        let mut cube = mob(Species::SulfurCube, "minecraft:sulfur_cube");
        cube.extra.set_body(Some("minecraft:oak_planks".into()));
        let mut bucket = crate::items::ItemStack::new("minecraft:sulfur_cube_bucket", 1);
        bucket.extra = Some(Box::new(bucket_components(&cube)));
        let bucket = item_from_nbt(&item_to_nbt(&bucket)).unwrap();

        let mut released = mob(Species::SulfurCube, "minecraft:sulfur_cube");
        apply_item_components(&mut released, &bucket);
        assert_eq!(released.extra.body_id(), Some("minecraft:oak_planks"));

        let empty = crate::items::ItemStack::new("minecraft:sulfur_cube_bucket", 1);
        let mut released = mob(Species::SulfurCube, "minecraft:sulfur_cube");
        apply_item_components(&mut released, &empty);
        assert_eq!(released.extra.body_id(), None, "a bucket without content");

        let mut cod = mob(Species::Cod, "minecraft:cod");
        apply_item_components(&mut cod, &bucket);
        assert_eq!(cod.extra.body_id(), None, "only a sulfur cube reads it");
    }
}

#[cfg(test)]
mod loaded_rotation_tests {
    use super::*;

    fn tag(yaw: f32, pitch: f32) -> Tag {
        Tag::compound([(
            "Rotation",
            Tag::List(vec![Tag::Float(yaw.to_bits()), Tag::Float(pitch.to_bits())]),
        )])
    }

    /// vanilla 26.3's values (the parity rows): yaw `% 360`, pitch `% 360` clamped to +-90.
    #[test]
    fn a_saved_rotation_loads_like_vanilla() {
        assert_eq!(loaded_rotation(&tag(400.0, 100.0)), [40.0, 90.0]);
        assert_eq!(loaded_rotation(&tag(270.0, 370.0)), [270.0, 10.0]);
        assert_eq!(loaded_rotation(&tag(-190.0, -100.0)), [-190.0, -90.0]);
        assert_eq!(loaded_rotation(&tag(180.0, 45.0)), [180.0, 45.0]);
        assert_eq!(loaded_rotation(&tag(73.0, -24.0)), [73.0, -24.0]);
        // a non-finite component is discarded: it loads as 0 and a save writes 0
        assert_eq!(loaded_rotation(&tag(f32::NAN, 20.0)), [0.0, 20.0]);
        assert_eq!(loaded_rotation(&tag(30.0, f32::INFINITY)), [30.0, 0.0]);
        // a save writes the raw value (a painting's yaw 270 from its facing) and reads it back as it is
        assert_eq!(rotation(&tag(270.0, 10.0)), [270.0, 10.0]);
        assert_eq!(
            loaded_rotation(&Tag::compound(Vec::<(&str, Tag)>::new())),
            [0.0, 0.0]
        );
    }
}
