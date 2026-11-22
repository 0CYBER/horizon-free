//! The components Horizon keeps as their stream bytes ([`Component::Raw`]),
//! decoded into the `Codec` form that vanilla's `HashOps` sees. Each function
//! reads exactly what the component's `STREAM_CODEC` writes (the readers in
//! `item/skip.rs` step over the same bytes) and builds what its `CODEC`
//! encodes: a component without a `networkSynchronized` stream codec goes as
//! its codec's NBT, so that NBT converts to hashes as it is (`NbtOps.convertTo`).
use super::*;
use crate::buf::{Codec, Reader};
use crate::item::{Profile, Template};
use horizon_core::nbt::Tag;

/// `DyeColor` names by id.
fn dye(id: i32) -> Option<HashCode> {
    pick(&DYE, id)
}
/// The `StringRepresentable` names of an enum whose stream codec is an `idMapper`
/// of the ids `0..`.
fn named(table: &[&str], r: &mut Reader) -> Option<HashCode> {
    pick(table, r.varint().ok()?)
}
/// `ByteBufCodecs.holderRegistry`: the registry id itself; the codec writes the
/// entry's identifier.
fn registry_entry(registry: &str, names: &dyn Names, r: &mut Reader) -> Option<HashCode> {
    entry(names, registry, r.varint().ok()?)
}
fn float(r: &mut Reader) -> Option<HashCode> {
    Some(HashOps::create_f32(r.f32().ok()?))
}
fn int(r: &mut Reader) -> Option<HashCode> {
    Some(HashOps::create_int(r.varint().ok()?))
}
/// A one-field record `{name: VAR_INT}`.
fn int_record(name: &str, r: &mut Reader) -> Option<HashCode> {
    Some(RecordHash::new().field(name, int(r)?).build())
}

const FOX: [&str; 2] = ["red", "snow"];
const SALMON: [&str; 3] = ["small", "medium", "large"];
const PARROT: [&str; 5] = ["red_blue", "blue", "green", "yellow_blue", "gray"];
const MOOSHROOM: [&str; 2] = ["red", "brown"];
const HORSE: [&str; 7] = [
    "white",
    "creamy",
    "chestnut",
    "brown",
    "black",
    "gray",
    "dark_brown",
];
const LLAMA: [&str; 4] = ["creamy", "white", "brown", "gray"];
const AXOLOTL: [&str; 5] = ["lucy", "wild", "gold", "cyan", "blue"];
const RABBIT: [&str; 6] = ["brown", "white", "black", "white_splotched", "gold", "salt"];
/// `TropicalFish.Pattern` by packed id (`base | index << 8`).
const TROPICAL: [(i32, &str); 12] = [
    (0, "kob"),
    (256, "sunstreak"),
    (512, "snooper"),
    (768, "dasher"),
    (1024, "brinely"),
    (1280, "spotty"),
    (1, "flopper"),
    (257, "stripey"),
    (513, "glitter"),
    (769, "blockfish"),
    (1025, "betty"),
    (1281, "clayfish"),
];

/// `TypedEntityData.codec`: the NBT with the type's identifier as `id`.
fn typed_data(registry: &str, names: &dyn Names, r: &mut Reader) -> Option<HashCode> {
    let id = registry_entry(registry, names, r)?;
    let Tag::Compound(mut tag) = r.nbt().ok()? else {
        return None;
    };
    // `stripId` leaves the data without its own id; the codec writes the type's.
    tag.remove(&"id".into());
    Some(typed_with_id(id, &tag))
}
fn typed_with_id(id: HashCode, tag: &horizon_core::nbt::Compound) -> HashCode {
    let mut entries: Vec<(HashCode, HashCode)> = tag
        .iter()
        .map(|(k, v)| {
            (
                HashOps::create_string_units(&k.0),
                HashOps::from_nbt(v, &[]),
            )
        })
        .collect();
    entries.push((string("id"), id));
    HashOps::create_map(entries)
}
/// A list written as a count and its elements, hashed as a list.
fn list(
    r: &mut Reader,
    mut element: impl FnMut(&mut Reader) -> Option<HashCode>,
) -> Option<HashCode> {
    let n = r.count(r.remaining()).ok()?;
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        out.push(element(r)?);
    }
    Some(HashOps::create_list(out))
}
fn optional(
    r: &mut Reader,
    element: impl FnOnce(&mut Reader) -> Option<HashCode>,
) -> Option<Option<HashCode>> {
    if r.bool().ok()? {
        Some(Some(element(r)?))
    } else {
        Some(None)
    }
}
pub(super) fn template_code(r: &mut Reader, names: &dyn Names) -> Option<HashCode> {
    let stack = <Template as Codec<Stack>>::read(r).ok()?;
    template(&stack, names)
}
/// `ByteBufCodecs.fixedSizeList(4)` of text components.
fn four_texts(r: &mut Reader) -> Option<(Vec<Tag>, HashCode)> {
    let tags: Vec<Tag> = (0..4).map(|_| r.nbt().ok()).collect::<Option<_>>()?;
    let code = HashOps::create_list(tags.iter().map(text));
    Some((tags, code))
}

/// `MobEffectInstance.Details.MAP_CODEC` (the fields of a record, flattened into
/// `rec`) from `STREAM_CODEC`: amplifier, duration, ambient, show_particles,
/// show_icon, then the hidden effect's details.
fn effect_details(rec: RecordHash, r: &mut Reader) -> Option<RecordHash> {
    let amplifier = r.varint().ok()?;
    let duration = r.varint().ok()?;
    let ambient = r.bool().ok()?;
    let particles = r.bool().ok()?;
    let icon = r.bool().ok()?;
    // `ExtraCodecs.UNSIGNED_BYTE` is a byte tag.
    let rec = or_default(
        rec,
        "amplifier",
        amplifier,
        0,
        HashOps::create_byte(amplifier as i8),
    );
    let rec = or_default(rec, "duration", duration, 0, HashOps::create_int(duration));
    let rec = or_default(
        rec,
        "ambient",
        ambient,
        false,
        HashOps::create_boolean(ambient),
    );
    let rec = or_default(
        rec,
        "show_particles",
        particles,
        true,
        HashOps::create_boolean(particles),
    );
    // `show_icon` is always written.
    let rec = rec.field("show_icon", HashOps::create_boolean(icon));
    Some(if r.bool().ok()? {
        let hidden = effect_details(RecordHash::new(), r)?.build();
        rec.field("hidden_effect", hidden)
    } else {
        rec
    })
}
/// `MobEffectInstance.CODEC`: the effect's identifier and its details.
pub(super) fn mob_effect(r: &mut Reader, names: &dyn Names) -> Option<HashCode> {
    let rec = RecordHash::new().field("id", registry_entry("minecraft:mob_effect", names, r)?);
    Some(effect_details(rec, r)?.build())
}

/// `PotionContents` after its potion and colour: the custom effects and name
/// (`custom_effects` is absent when empty).
pub(super) fn potion_tail(rec: RecordHash, rest: &[u8], names: &dyn Names) -> Option<RecordHash> {
    let mut r = Reader::new(rest);
    let n = r.count(r.remaining()).ok()?;
    let mut effects = Vec::with_capacity(n);
    for _ in 0..n {
        effects.push(mob_effect(&mut r, names)?);
    }
    let name = if r.bool().ok()? {
        Some(string(&r.string(32767).ok()?))
    } else {
        None
    };
    (r.remaining() == 0).then_some(())?;
    let rec = if effects.is_empty() {
        rec
    } else {
        rec.field("custom_effects", HashOps::create_list(effects))
    };
    Some(rec.optional("custom_name", name))
}

/// `ResolvableProfile.CODEC` (`FULL_CODEC`): the game profile's or the partial's
/// fields, then the skin patch's. Both forms write `id`, `name` and a
/// `properties` list that is absent when empty.
pub(super) fn profile(p: &Profile) -> HashCode {
    let rec = RecordHash::new()
        .optional("name", p.name.as_deref().map(string))
        .optional(
            "id",
            // `UUIDUtil.CODEC`: an int array.
            p.id.map(|id| {
                HashOps::create_int_list(&[
                    (id >> 96) as i32,
                    (id >> 64) as i32,
                    (id >> 32) as i32,
                    id as i32,
                ])
            }),
        );
    let rec = if p.properties.is_empty() {
        rec
    } else {
        rec.field(
            "properties",
            HashOps::create_list(p.properties.iter().map(|(name, value, signature)| {
                RecordHash::new()
                    .field("name", string(name))
                    .field("value", string(value))
                    .optional("signature", signature.as_deref().map(string))
                    .build()
            })),
        )
    };
    rec.optional("texture", p.skin.body.as_deref().map(string))
        .optional("cape", p.skin.cape.as_deref().map(string))
        .optional("elytra", p.skin.elytra.as_deref().map(string))
        .optional(
            "model",
            p.skin
                .slim
                .map(|slim| string(if slim { "slim" } else { "wide" })),
        )
        .build()
}

/// `ByteBufCodecs.holder(key, direct)` as `RegistryCodecs.holder(key, directCodec)`
/// writes it: a reference by identifier, a direct value as its record.
fn holder_or(
    registry: &str,
    names: &dyn Names,
    r: &mut Reader,
    direct: impl FnOnce(&mut Reader) -> Option<HashCode>,
) -> Option<HashCode> {
    match r.varint().ok()? {
        0 => direct(r),
        id => entry(names, registry, id.wrapping_sub(1)),
    }
}
/// `TrimMaterial.DIRECT_CODEC`.
pub(super) fn trim_material(r: &mut Reader) -> Option<HashCode> {
    let palette = string(&r.string(32767).ok()?);
    let description = text(&r.nbt().ok()?);
    Some(
        RecordHash::new()
            .field("palette_id", palette)
            .field("description", description)
            .build(),
    )
}
/// `TrimPattern.DIRECT_CODEC` (`decal` is always written).
fn trim_pattern(r: &mut Reader) -> Option<HashCode> {
    let asset = string(&r.string(32767).ok()?);
    let description = text(&r.nbt().ok()?);
    let decal = r.bool().ok()?;
    Some(
        RecordHash::new()
            .field("asset_id", asset)
            .field("description", description)
            .field("decal", HashOps::create_boolean(decal))
            .build(),
    )
}
/// `Instrument.DIRECT_CODEC`.
fn instrument(r: &mut Reader, names: &dyn Names) -> Option<HashCode> {
    let sound = super::gameplay::sound(r, names)?.code;
    let (duration, range) = (r.f32().ok()?, r.f32().ok()?);
    let damage = r.varint().ok()?;
    let description = text(&r.nbt().ok()?);
    let rec = RecordHash::new()
        .field("sound_event", sound)
        .field("use_duration", HashOps::create_f32(duration))
        .field("range", HashOps::create_f32(range));
    Some(
        or_default(
            rec,
            "durability_damage",
            damage,
            0,
            HashOps::create_int(damage),
        )
        .field("description", description)
        .build(),
    )
}
/// `BannerPattern.DIRECT_CODEC`.
fn banner_pattern(r: &mut Reader) -> Option<HashCode> {
    let asset = string(&r.string(32767).ok()?);
    let key = string(&r.string(32767).ok()?);
    Some(
        RecordHash::new()
            .field("asset_id", asset)
            .field("translation_key", key)
            .build(),
    )
}

/// The hash of component `kind` from its stream bytes, or `None` when Horizon
/// has no codec form for it (no persistent codec in vanilla, a direct holder, an
/// id with no name, a component still to be ported).
pub(super) fn code(kind: i32, bytes: &[u8], names: &dyn Names) -> Option<HashCode> {
    let mut r = Reader::new(bytes);
    let r = &mut r;
    let code = match kind {
        // `Unit.CODEC`: an empty map.
        34 | 120 => HashOps::empty_map(),
        // Persistent codecs only: the stream form is the codec's NBT.
        // intangible_projectile, map_decorations, debug_stick_state, recipes,
        // lock (an `ItemPredicate`) and container_loot.
        22 | 49 | 59 | 68 | 81 | 82 => HashOps::from_nbt(&r.nbt().ok()?, &[]),
        // `Codec.FLOAT`: minimum_attack_charge, potion_duration_scale.
        7 | 54 => float(r)?,
        // `Identifier.CODEC`: tooltip_style, note_block_sound.
        35 | 73 => string(&r.string(32767).ok()?),
        // `Enchantable`, `VillagerFood`.
        31 => int_record("value", r)?,
        44 => int_record("nutrition", r)?,
        // `DyeColor.CODEC`: dye, wolf/collar, tropical_fish/base_color and
        // pattern_color, cat/collar, sheep/color, shulker/color, cushion/color.
        46 | 91 | 96 | 97 | 114 | 115 | 116 | 121 => dye(r.varint().ok()?)?,
        8 => registry_entry("minecraft:damage_type", names, r)?,
        43 => registry_entry("minecraft:block_transformer", names, r)?,
        88 => registry_entry("minecraft:villager_type", names, r)?,
        89 => registry_entry("minecraft:wolf_variant", names, r)?,
        90 => registry_entry("minecraft:wolf_sound_variant", names, r)?,
        100 => registry_entry("minecraft:pig_variant", names, r)?,
        101 => registry_entry("minecraft:pig_sound_variant", names, r)?,
        102 => registry_entry("minecraft:cow_variant", names, r)?,
        103 => registry_entry("minecraft:cow_sound_variant", names, r)?,
        104 => registry_entry("minecraft:chicken_variant", names, r)?,
        105 => registry_entry("minecraft:chicken_sound_variant", names, r)?,
        106 => registry_entry("minecraft:zombie_nautilus_variant", names, r)?,
        107 => registry_entry("minecraft:frog_variant", names, r)?,
        112 => registry_entry("minecraft:cat_variant", names, r)?,
        113 => registry_entry("minecraft:cat_sound_variant", names, r)?,
        117 => registry_entry("minecraft:decorated_pot_pattern", names, r)?,
        // The mob variants that are enums.
        92 => named(&FOX, r)?,
        93 => named(&SALMON, r)?,
        94 => named(&PARROT, r)?,
        95 => {
            let id = r.varint().ok()?;
            string(TROPICAL.iter().find(|(packed, _)| *packed == id)?.1)
        }
        98 => named(&MOOSHROOM, r)?,
        99 => match r.varint().ok()? {
            99 => string("evil"),
            id => pick(&RABBIT, id)?,
        },
        108 => named(&HORSE, r)?,
        110 => named(&LLAMA, r)?,
        111 => named(&AXOLOTL, r)?,
        // `TypedEntityData` (entity_data, block_entity_data) and `CustomData`.
        60 => typed_data("minecraft:entity_type", names, r)?,
        62 => typed_data("minecraft:block_entity_type", names, r)?,
        61 => HashOps::from_nbt(&r.nbt().ok()?, &[]),
        // `BlockItemStateProperties`: a string map.
        78 => {
            let n = r.count(r.remaining()).ok()?;
            let mut entries = Vec::with_capacity(n);
            for _ in 0..n {
                let k = string(&r.string(32767).ok()?);
                entries.push((k, string(&r.string(32767).ok()?)));
            }
            HashOps::create_map(entries)
        }
        // `ArmorTrim`, `InstrumentComponent`, `provides_trim_material` and
        // `BannerPatternLayers` holders (a reference stays typed in Horizon but
        // a direct one comes here).
        58 => {
            let material = holder_or("minecraft:trim_material", names, r, trim_material)?;
            let pattern = holder_or("minecraft:trim_pattern", names, r, trim_pattern)?;
            RecordHash::new()
                .field("material", material)
                .field("pattern", pattern)
                .build()
        }
        63 => holder_or("minecraft:instrument", names, r, |r| instrument(r, names))?,
        64 => holder_or("minecraft:trim_material", names, r, trim_material)?,
        74 => list(r, |r| {
            let pattern = holder_or("minecraft:banner_pattern", names, r, banner_pattern)?;
            let color = dye(r.varint().ok()?)?;
            Some(
                RecordHash::new()
                    .field("pattern", pattern)
                    .field("color", color)
                    .build(),
            )
        })?,
        // `SulfurCubeContent`: one template.
        80 => template_code(r, names)?,
        // `PotDecorations`: back, left, right, front.
        76 => {
            let mut rec = RecordHash::new();
            for side in ["back", "left", "right", "front"] {
                rec = rec.optional(side, optional(r, |r| template_code(r, names))?);
            }
            rec.build()
        }
        // `ItemContainerContents`: the occupied slots as `{slot, item}`.
        77 => {
            let n = r.count(256).ok()?;
            let mut slots = Vec::new();
            for slot in 0..n {
                if let Some(item) = optional(r, |r| template_code(r, names))? {
                    slots.push(
                        RecordHash::new()
                            .field("slot", HashOps::create_int(slot as i32))
                            .field("item", item)
                            .build(),
                    );
                }
            }
            HashOps::create_list(slots)
        }
        // `Bees`: occupants.
        79 => list(r, |r| {
            let data = typed_data("minecraft:entity_type", names, r)?;
            let ticks = r.varint().ok()?;
            let min = r.varint().ok()?;
            Some(
                RecordHash::new()
                    .field("entity_data", data)
                    .field("ticks_in_hive", HashOps::create_int(ticks))
                    .field("min_ticks_in_hive", HashOps::create_int(min))
                    .build(),
            )
        })?,
        // `SignText`: messages, the filtered ones when they differ, colour, glow.
        118 | 119 => {
            let (messages, code) = four_texts(r)?;
            // `filteredMessagesForSerialization`: absent when equal to the messages.
            let filtered = if r.bool().ok()? {
                let (tags, code) = four_texts(r)?;
                (tags != messages).then_some(code)
            } else {
                None
            };
            let color = dye(r.varint().ok()?)?;
            let glow = r.bool().ok()?;
            RecordHash::new()
                .field("messages", code)
                .optional("filtered_messages", filtered)
                .field("color", color)
                .field("has_glowing_text", HashOps::create_boolean(glow))
                .build()
        }
        _ => super::gameplay::code(kind, r, names)?,
    };
    (r.remaining() == 0).then_some(code)
}

#[cfg(test)]
mod tests;
