//! Before components: FriendlyByteBuf's present flag, item id, byte count and unnamed NBT.
//! Saved NBT upgrades use Horizon's vanilla datafix implementation; values stored here are pinned.
use super::*;
use crate::Error;
use horizon_world::datafix::{self, dynamic as d, schema::Ty};
use std::collections::{HashMap, VecDeque};

const CACHE_ENTRIES: usize = 1024;
const CACHE_BYTES: usize = 8 * 1024 * 1024;
const CACHE_ENTRY_BYTES: usize = 256 * 1024;
#[derive(Debug)]
struct Original {
    tag: Option<Tag>,
    bytes: usize,
}
/// Original NBT for unchanged stacks. Two old encodings that normalize to the same value are
/// ambiguous: neither original is reused. Every session owns its cache; nothing is persisted.
#[derive(Debug, Default)]
pub(crate) struct Cache {
    entries: HashMap<Vec<u8>, Original>,
    order: VecDeque<Vec<u8>>,
    bytes: usize,
}
impl Cache {
    fn key(stack: &Stack) -> Option<Vec<u8>> {
        if stack.item < 0 {
            return None;
        }
        let _pinned = ids::unrenumbered();
        let mut value = stack.clone();
        value.count = 1;
        value.wire = WirePatch::default();
        value.added.sort_by_key(|(kind, _)| *kind);
        value.removed.sort_unstable();
        let mut w = Writer::new();
        Slot::write(&Some(value), &mut w);
        (w.data.len() <= CACHE_ENTRY_BYTES).then_some(w.data)
    }
    fn remember(&mut self, stack: &Stack, tag: Tag) {
        let Some(key) = Self::key(stack) else {
            return;
        };
        if let Some(original) = self.entries.get_mut(&key) {
            if original.tag.as_ref().is_some_and(|old| *old != tag) {
                self.bytes -= original.bytes;
                original.tag = None;
                original.bytes = key.len() * 2;
                self.bytes += original.bytes;
            }
            return;
        }
        let mut w = Writer::new();
        w.nbt(&tag);
        // Count both copies of the FIFO key, conservatively, in the serialized byte budget.
        let bytes = key.len() * 2 + w.data.len();
        if bytes > CACHE_ENTRY_BYTES {
            return;
        }
        while self.entries.len() >= CACHE_ENTRIES || self.bytes + bytes > CACHE_BYTES {
            let Some(old) = self.order.pop_front() else {
                break;
            };
            if let Some(old) = self.entries.remove(&old) {
                self.bytes -= old.bytes;
            }
        }
        self.bytes += bytes;
        self.order.push_back(key.clone());
        self.entries.insert(
            key,
            Original {
                tag: Some(tag),
                bytes,
            },
        );
    }
    fn original(&self, stack: &Stack) -> Option<Tag> {
        self.entries.get(&Self::key(stack)?)?.tag.clone()
    }
}

const DYES: &[&str] = &[
    "white",
    "orange",
    "magenta",
    "light_blue",
    "yellow",
    "lime",
    "pink",
    "gray",
    "light_gray",
    "cyan",
    "purple",
    "blue",
    "brown",
    "green",
    "red",
    "black",
];
const SLOTS: &[&str] = &[
    "any", "mainhand", "offhand", "hand", "feet", "legs", "chest", "head", "armor", "body",
];
const KINDS: &[(&str, i32)] = &[
    ("custom_data", 0),
    ("max_stack_size", 1),
    ("max_damage", 2),
    ("damage", 3),
    ("unbreakable", 4),
    ("custom_name", 6),
    ("item_name", 9),
    ("lore", 11),
    ("enchantments", 13),
    ("can_place_on", 14),
    ("can_break", 15),
    ("attribute_modifiers", 16),
    ("custom_model_data", 17),
    ("tooltip_display", 18),
    ("repair_cost", 19),
    ("enchantment_glint_override", 21),
    ("stored_enchantments", 45),
    ("dyed_color", 47),
    ("map_id", 48),
    ("map_decorations", 49),
    ("charged_projectiles", 51),
    ("bundle_contents", 52),
    ("potion_contents", 53),
    ("suspicious_stew_effects", 55),
    ("writable_book_content", 56),
    ("written_book_content", 57),
    ("trim", 58),
    ("debug_stick_state", 59),
    ("entity_data", 60),
    ("bucket_entity_data", 61),
    ("block_entity_data", 62),
    ("instrument", 63),
    ("recipes", 68),
    ("lodestone_tracker", 69),
    ("firework_explosion", 70),
    ("fireworks", 71),
    ("profile", 72),
    ("banner_patterns", 74),
    ("base_color", 75),
    ("pot_decorations", 76),
    ("container", 77),
    ("block_state", 78),
    ("bees", 79),
    ("lock", 81),
    ("container_loot", 82),
];
pub(super) fn reading() -> bool {
    matches!(ids::active(),Some((ids,ids::Way::Up)) if ids.quirks.legacy_item_nbt)
}
pub(super) fn writing() -> bool {
    matches!(ids::active(),Some((ids,ids::Way::Down)) if ids.quirks.legacy_item_nbt)
}
fn component_id(name: &str) -> Option<i32> {
    let name = name.strip_prefix("minecraft:")?;
    KINDS.iter().find(|(n, _)| *n == name).map(|(_, id)| *id)
}
fn text(value: Option<&Tag>) -> Option<String> {
    d::as_str(value)
}
fn number(value: Option<&Tag>, default: i32) -> i32 {
    d::as_int(value, default)
}
fn list(value: Option<&Tag>) -> &[Tag] {
    match value {
        Some(Tag::List(values)) => values,
        _ => &[],
    }
}
fn int_array(value: Option<&Tag>) -> Vec<i32> {
    match value {
        Some(Tag::IntArray(values)) => values.clone(),
        _ => list(value).iter().map(|v| number(Some(v), 0)).collect(),
    }
}
fn registry_id(registry: &str, name: &str) -> Result<i32> {
    ids::legacy_registry_id(registry, name)
        .ok_or_else(|| Error::new(format!("legacy item entry missing from {registry}")))
}
fn registry_name(registry: &str, id: i32) -> Result<String> {
    ids::legacy_registry_name(registry, id)
        .filter(|name| !name.is_empty())
        .ok_or_else(|| Error::new(format!("legacy item id missing from {registry}")))
}
fn id_tag(registry: &str, value: Option<&Tag>) -> Result<i32> {
    registry_id(
        registry,
        &text(value).ok_or_else(|| Error::new("missing legacy item identifier"))?,
    )
}
fn raw_nbt(value: &Tag) -> Component {
    let mut w = Writer::new();
    w.nbt(value);
    Component::Raw(w.data)
}
pub(super) fn read_stack(r: &mut Reader) -> Result<Option<Stack>> {
    if !r.bool()? {
        return Ok(None);
    }
    let item = ids::item(r.varint()?);
    let count = i32::from(r.i8()?);
    let tag = r.nbt()?;
    if !matches!(tag, Tag::End | Tag::Compound(_)) {
        return Err(Error::new("legacy item tag is not a compound"));
    }
    if count <= 0 || item < 0 {
        return Ok(None);
    }
    let mut saved = Tag::compound([
        ("id", d::string(&registry_name("minecraft:item", item)?)),
        ("Count", Tag::Byte(count as i8)),
    ]);
    if !matches!(tag, Tag::End) {
        d::set(&mut saved, "tag", tag.clone());
    }
    let from = ids::quirks().legacy_item_data_version;
    if from <= 0 {
        return Err(Error::new("missing legacy item data version"));
    }
    datafix::update(Ty::ItemStack, &mut saved, from, datafix::CURRENT);
    let stack = from_tag(&saved, Some((item, count)))?;
    if let Some(cache) = ids::legacy_item_cache() {
        cache.lock().unwrap().remember(&stack, tag);
    }
    Ok(Some(stack))
}

fn from_tag(saved: &Tag, known: Option<(i32, i32)>) -> Result<Stack> {
    let _nested = super::Nested::enter()?;
    let _pinned = ids::unrenumbered();
    let (item, count) = match known {
        Some(values) => values,
        None => (
            id_tag("minecraft:item", saved.get("id"))?,
            number(saved.get("count"), 1),
        ),
    };
    let mut stack = Stack::new(item, count);
    let Some(Tag::Compound(components)) = saved.get("components") else {
        return Ok(stack);
    };
    for (name, value) in components {
        let name = name.to_string_lossy();
        if let Some(removed) = name.strip_prefix('!') {
            if let Some(kind) = component_id(removed) {
                stack.removed.push(kind);
            }
            continue;
        }
        let Some(kind) = component_id(&name) else {
            continue;
        };
        let value = match kind {
            0 => Component::Nbt(value.clone()),
            1..=3 | 19 | 48 => Component::Int(number(Some(value), 0)),
            4 => Component::Unit,
            6 | 9 => Component::Text(text_component(value)),
            11 => Component::Lore(list(Some(value)).iter().map(text_component).collect()),
            13 | 45 => {
                let levels = value.get("levels").unwrap_or(value);
                let mut out = Vec::new();
                if let Tag::Compound(levels) = levels {
                    for (name, level) in levels {
                        if let Some(id) = ids::legacy_registry_id(
                            "minecraft:enchantment",
                            &name.to_string_lossy(),
                        ) {
                            out.push((id, number(Some(level), 0)));
                        }
                    }
                }
                Component::Enchantments(out)
            }
            14 | 15 => predicates(value)?,
            16 => attributes(value)?,
            17 => Component::CustomModelData {
                floats: list(value.get("floats"))
                    .iter()
                    .map(|v| d::as_number(v).map_or(0.0, |v| v.float_value()))
                    .collect(),
                flags: list(value.get("flags"))
                    .iter()
                    .map(|v| d::as_bool(Some(v), false))
                    .collect(),
                strings: list(value.get("strings"))
                    .iter()
                    .filter_map(|v| text(Some(v)))
                    .collect(),
                colors: int_array(value.get("colors")),
            },
            18 => Component::TooltipDisplay {
                hide: d::as_bool(value.get("hide_tooltip"), false),
                hidden: list(value.get("hidden_components"))
                    .iter()
                    .filter_map(|v| component_id(&text(Some(v))?))
                    .collect(),
            },
            21 => Component::Bool(d::as_bool(Some(value), false)),
            47 => Component::Int(number(Some(value), 0)),
            49 => {
                let mut value = value.clone();
                if let Tag::Compound(entries) = &mut value {
                    for entry in entries.values_mut() {
                        if let Some(name) = text(entry.get("type")) {
                            d::set(entry, "type", d::string(&d::namespaced(&name)));
                        }
                    }
                }
                raw_nbt(&value)
            }
            59 | 61 | 68 | 81 | 82 => raw_nbt(value),
            51 | 52 => Component::Stacks(
                list(Some(value))
                    .iter()
                    .map(|v| from_tag(v, None))
                    .collect::<Result<_>>()?,
            ),
            53 => potion(value)?,
            55 => Component::StewEffects(
                list(Some(value))
                    .iter()
                    .map(|v| {
                        Ok((
                            id_tag("minecraft:mob_effect", v.get("id"))?,
                            number(v.get("duration"), 160),
                        ))
                    })
                    .collect::<Result<_>>()?,
            ),
            56 => {
                let pages = list(value.get("pages"));
                let filtered = pages.iter().any(|p| p.get("filtered").is_some());
                Component::WritableBook {
                    pages: pages
                        .iter()
                        .map(|p| text(p.get("raw").or(Some(p))).unwrap_or_default())
                        .collect(),
                    filtered: if filtered {
                        pages.iter().map(|p| text(p.get("filtered"))).collect()
                    } else {
                        Vec::new()
                    },
                }
            }
            57 => {
                let title = value.get("title").unwrap_or(&Tag::End);
                let pages = list(value.get("pages"));
                let filtered = pages.iter().any(|p| p.get("filtered").is_some());
                Component::WrittenBook(WrittenBook {
                    title: text(title.get("raw").or(Some(title))).unwrap_or_default(),
                    title_filtered: text(title.get("filtered")),
                    author: text(value.get("author")).unwrap_or_default(),
                    generation: number(value.get("generation"), 0),
                    resolved: d::as_bool(value.get("resolved"), false),
                    pages: pages
                        .iter()
                        .map(|p| text_component(p.get("raw").unwrap_or(p)))
                        .collect(),
                    pages_filtered: if filtered {
                        pages
                            .iter()
                            .map(|p| p.get("filtered").map(text_component))
                            .collect()
                    } else {
                        Vec::new()
                    },
                })
            }
            58 => Component::Trim {
                material: id_tag("minecraft:trim_material", value.get("material"))?,
                pattern: id_tag("minecraft:trim_pattern", value.get("pattern"))?,
            },
            60 => typed_data(value, "minecraft:entity_type")?,
            62 => typed_data(value, "minecraft:block_entity_type")?,
            63 => Component::Instrument(id_tag("minecraft:instrument", Some(value))?),
            69 => {
                let target = if let Some(target) = value.get("target") {
                    let coords = int_array(target.get("pos"));
                    if coords.len() == 3 {
                        Some((
                            text(target.get("dimension"))
                                .unwrap_or_else(|| "minecraft:overworld".into()),
                            [coords[0], coords[1], coords[2]],
                        ))
                    } else {
                        None
                    }
                } else {
                    None
                };
                Component::LodestoneTracker {
                    target,
                    tracked: d::as_bool(value.get("tracked"), true),
                }
            }
            70 => Component::FireworkExplosion(explosion(value)),
            71 => Component::Fireworks {
                flight: number(value.get("flight_duration"), 0),
                explosions: list(value.get("explosions"))
                    .iter()
                    .map(explosion)
                    .collect(),
            },
            72 => Component::Profile(profile(value)),
            74 => Component::BannerPatterns(
                list(Some(value))
                    .iter()
                    .map(|v| {
                        Ok((
                            id_tag("minecraft:banner_pattern", v.get("pattern"))?,
                            dye(v.get("color")),
                        ))
                    })
                    .collect::<Result<_>>()?,
            ),
            75 => Component::Int(dye(Some(value))),
            76 => decorations(value)?,
            77 => container(value)?,
            78 => {
                let mut w = Writer::new();
                let Tag::Compound(map) = value else {
                    return Err(Error::new("block state is not a map"));
                };
                w.varint(map.len() as i32);
                for (k, v) in map {
                    w.string(&k.to_string_lossy());
                    w.string(&text(Some(v)).unwrap_or_default());
                }
                Component::Raw(w.data)
            }
            79 => bees(value)?,
            _ => continue,
        };
        stack.added.push((kind, value));
    }
    Ok(stack)
}

fn dye(value: Option<&Tag>) -> i32 {
    text(value)
        .and_then(|s| DYES.iter().position(|v| *v == s))
        .map_or(0, |i| i as i32)
}
fn text_component(value: &Tag) -> Tag {
    match value {
        Tag::Compound(map) if map.len() == 1 => {
            value.get("text").cloned().unwrap_or_else(|| value.clone())
        }
        Tag::List(values) if values.len() == 1 => text_component(&values[0]),
        _ => value.clone(),
    }
}
fn explosion(value: &Tag) -> FireworkExplosion {
    FireworkExplosion {
        shape: text(value.get("shape"))
            .and_then(|s| {
                ["small_ball", "large_ball", "star", "creeper", "burst"]
                    .iter()
                    .position(|v| *v == s)
            })
            .unwrap_or(0) as i32,
        colors: int_array(value.get("colors")),
        fade: int_array(value.get("fade_colors")),
        trail: d::as_bool(value.get("has_trail"), false),
        twinkle: d::as_bool(value.get("has_twinkle"), false),
    }
}
fn uuid(value: Option<&Tag>) -> Option<u128> {
    let parts = int_array(value);
    (parts.len() == 4).then(|| {
        parts
            .iter()
            .fold(0u128, |n, part| (n << 32) | u128::from(*part as u32))
    })
}
fn profile(value: &Tag) -> Profile {
    Profile {
        name: text(value.get("name")),
        id: uuid(value.get("id")),
        skin: SkinPatch::default(),
        properties: list(value.get("properties"))
            .iter()
            .map(|v| {
                (
                    text(v.get("name")).unwrap_or_default(),
                    text(v.get("value")).unwrap_or_default(),
                    text(v.get("signature")),
                )
            })
            .collect(),
    }
}
fn typed_data(value: &Tag, registry: &str) -> Result<Component> {
    let mut data = value.clone();
    let id = id_tag(registry, data.get("id"))?;
    d::remove(&mut data, "id");
    let mut w = Writer::new();
    w.varint(id);
    w.nbt(&data);
    Ok(Component::Raw(w.data))
}
fn container(value: &Tag) -> Result<Component> {
    let mut contents = Vec::new();
    for entry in list(Some(value)) {
        let slot = number(entry.get("slot"), -1);
        if (0..256).contains(&slot)
            && let Some(item) = entry.get("item")
        {
            contents.push((slot as usize, from_tag(item, None)?));
        }
    }
    let mut w = Writer::new();
    let n = contents.iter().map(|(slot, _)| slot + 1).max().unwrap_or(0);
    w.varint(n as i32);
    for slot in 0..n {
        let item = contents.iter().find(|(i, _)| *i == slot);
        w.bool(item.is_some());
        if let Some((_, item)) = item {
            Template::write(item, &mut w);
        }
    }
    Ok(Component::Raw(w.data))
}
fn decorations(value: &Tag) -> Result<Component> {
    let mut w = Writer::new();
    for side in ["back", "left", "right", "front"] {
        let entry = value.get(side);
        w.bool(entry.is_some());
        if let Some(entry) = entry {
            let item = if let Tag::String(_) = entry {
                Stack::new(id_tag("minecraft:item", Some(entry))?, 1)
            } else {
                from_tag(entry, None)?
            };
            Template::write(&item, &mut w);
        }
    }
    Ok(Component::Raw(w.data))
}

fn attributes(value: &Tag) -> Result<Component> {
    let entries = list(value.get("modifiers").or(Some(value)));
    let mut w = Writer::new();
    w.varint(entries.len() as i32);
    for entry in entries {
        w.varint(id_tag("minecraft:attribute", entry.get("type"))?);
        w.string(&text(entry.get("id")).ok_or_else(|| Error::new("attribute modifier has no id"))?);
        w.f64(d::as_double(entry.get("amount"), 0.0));
        w.varint(match text(entry.get("operation")).as_deref() {
            Some("add_multiplied_base") => 1,
            Some("add_multiplied_total") => 2,
            _ => 0,
        });
        w.varint(
            text(entry.get("slot"))
                .and_then(|s| SLOTS.iter().position(|v| *v == s))
                .unwrap_or(0) as i32,
        );
        w.varint(0);
    }
    Ok(Component::Raw(w.data))
}
fn predicates(value: &Tag) -> Result<Component> {
    let predicates = list(value.get("predicates").or(Some(value)));
    let mut w = Writer::new();
    w.varint(predicates.len() as i32);
    for predicate in predicates {
        let blocks = predicate.get("blocks");
        w.bool(blocks.is_some());
        if let Some(blocks) = blocks {
            if let Some(tag) =
                text(Some(blocks)).and_then(|s| s.strip_prefix('#').map(str::to_owned))
            {
                w.varint(0);
                w.string(&tag);
            } else {
                let names = if matches!(blocks, Tag::String(_)) {
                    vec![blocks]
                } else {
                    list(Some(blocks)).iter().collect()
                };
                w.varint(names.len() as i32 + 1);
                for name in names {
                    w.varint(id_tag("minecraft:block", Some(name))?);
                }
            }
        }
        let properties = predicate.get("state");
        w.bool(properties.is_some());
        if let Some(Tag::Compound(properties)) = properties {
            w.varint(properties.len() as i32);
            for (name, matcher) in properties {
                w.string(&name.to_string_lossy());
                let exact = text(Some(matcher));
                w.bool(exact.is_some());
                if let Some(exact) = exact {
                    w.string(&exact);
                } else {
                    for field in ["min", "max"] {
                        let bound = text(matcher.get(field));
                        w.bool(bound.is_some());
                        if let Some(bound) = bound {
                            w.string(&bound);
                        }
                    }
                }
            }
        } else if properties.is_some() {
            return Err(Error::new("invalid predicate properties"));
        }
        let nbt = predicate.get("nbt");
        w.bool(nbt.is_some());
        if let Some(nbt) = nbt {
            let parsed;
            let nbt = if let Some(snbt) = text(Some(nbt)) {
                let (value, used) = horizon_core::nbt::parse_snbt(&snbt).map_err(Error::new)?;
                if used != snbt.len() {
                    return Err(Error::new("trailing predicate SNBT"));
                }
                parsed = value;
                &parsed
            } else {
                nbt
            };
            w.nbt(nbt);
        }
        w.varint(0);
        w.varint(0); // No component matchers existed in legacy predicates.
    }
    Ok(Component::Raw(w.data))
}
fn effect(value: &Tag, w: &mut Writer) -> Result<()> {
    w.varint(id_tag("minecraft:mob_effect", value.get("id"))?);
    let mut current = value;
    loop {
        w.varint(number(current.get("amplifier"), 0).clamp(0, 255));
        w.varint(number(current.get("duration"), 0));
        w.bool(d::as_bool(current.get("ambient"), false));
        let particles = d::as_bool(current.get("show_particles"), true);
        w.bool(particles);
        w.bool(d::as_bool(current.get("show_icon"), particles));
        let hidden = current.get("hidden_effect");
        w.bool(hidden.is_some());
        let Some(hidden) = hidden else {
            break;
        };
        current = hidden;
    }
    Ok(())
}
fn potion(value: &Tag) -> Result<Component> {
    let potion = value
        .get("potion")
        .map(|v| id_tag("minecraft:potion", Some(v)))
        .transpose()?;
    let color = value.get("custom_color").map(|v| number(Some(v), 0));
    let effects = list(value.get("custom_effects"));
    let name = text(value.get("custom_name"));
    let mut w = Writer::new();
    w.varint(effects.len() as i32);
    for entry in effects {
        effect(entry, &mut w)?;
    }
    w.bool(name.is_some());
    if let Some(name) = &name {
        w.string(name);
    }
    Ok(Component::Potion {
        potion,
        color,
        rest: if effects.is_empty() && name.is_none() {
            Vec::new()
        } else {
            w.data
        },
    })
}

pub(super) fn write_stack(stack: Option<&Stack>, w: &mut Writer) {
    let Some(stack) = stack else {
        w.bool(false);
        return;
    };
    let item = ids::item(stack.item);
    if item < 0 || !(1..=127).contains(&stack.count) {
        w.bool(false);
        return;
    }
    let cached = ids::legacy_item_cache().and_then(|cache| cache.lock().unwrap().original(stack));
    let from_cache = cached.is_some();
    let tag = if let Some(tag) = cached {
        tag
    } else {
        let Ok(tag) = legacy_tag(stack) else {
            w.bool(false);
            return;
        };
        tag
    };
    w.bool(true);
    w.varint(item);
    w.i8(stack.count as i8);
    if !from_cache && matches!(&tag, Tag::Compound(map) if map.is_empty()) {
        w.nbt(&Tag::End);
    } else {
        w.nbt(&tag);
    }
}
fn set_sub(tag: &mut Tag, outer: &str, key: &str, value: Tag) {
    if !matches!(tag.get(outer), Some(Tag::Compound(_))) {
        d::set(tag, outer, d::empty_map());
    }
    d::set(tag.get_mut(outer).unwrap(), key, value);
}
fn json_text(value: &Tag) -> Tag {
    fn fix(value: &mut datafix::json::Json) {
        use datafix::json::Json;
        match value {
            Json::Obj(entries) => {
                for (name, value) in entries {
                    let name = String::from_utf16_lossy(name);
                    if matches!(
                        name.as_str(),
                        "bold" | "italic" | "underlined" | "strikethrough" | "obfuscated"
                    ) {
                        if let Json::Num(n) = value {
                            *value = Json::Bool(n != "0");
                        }
                    } else {
                        fix(value);
                    }
                }
            }
            Json::Arr(entries) => entries.iter_mut().for_each(fix),
            _ => {}
        }
    }
    let mut json = datafix::json::from_nbt(value);
    fix(&mut json);
    Tag::String(horizon_core::nbt::JavaString(
        datafix::json::to_stable_string(&json),
    ))
}
fn uuid_tag(uuid: u128) -> Tag {
    Tag::IntArray((0..4).map(|i| (uuid >> (96 - i * 32)) as i32).collect())
}
fn legacy_saved(stack: &Stack) -> Result<Tag> {
    let tag = legacy_tag(stack)?;
    let mut result = Tag::compound([
        (
            "id",
            d::string(&registry_name("minecraft:item", stack.item)?),
        ),
        ("Count", Tag::Byte(stack.count as i8)),
    ]);
    if !matches!(&tag, Tag::Compound(m) if m.is_empty()) {
        d::set(&mut result, "tag", tag);
    }
    Ok(result)
}
fn legacy_tag(stack: &Stack) -> Result<Tag> {
    let _pinned = ids::unrenumbered();
    let mut tag = match stack.component(component::CUSTOM_DATA) {
        Some(Component::Nbt(tag @ Tag::Compound(_))) => tag.clone(),
        _ => d::empty_map(),
    };
    let mut hide = 0;
    for (kind, value) in &stack.added {
        match (*kind, value) {
            (3, Component::Int(value)) => d::set(&mut tag, "Damage", Tag::Int(*value)),
            (19, Component::Int(value)) => d::set(&mut tag, "RepairCost", Tag::Int(*value)),
            (4, Component::Unit) => d::set(&mut tag, "Unbreakable", Tag::Byte(1)),
            (6 | 9, Component::Text(value)) => {
                set_sub(&mut tag, "display", "Name", json_text(value))
            }
            (11, Component::Lore(values)) => set_sub(
                &mut tag,
                "display",
                "Lore",
                Tag::List(values.iter().map(json_text).collect()),
            ),
            (13 | 45, Component::Enchantments(values)) => {
                let values = values
                    .iter()
                    .map(|(id, level)| {
                        Ok(Tag::compound([
                            (
                                "id",
                                d::string(&registry_name("minecraft:enchantment", *id)?),
                            ),
                            ("lvl", Tag::Short(*level as i16)),
                        ]))
                    })
                    .collect::<Result<Vec<_>>>()?;
                d::set(
                    &mut tag,
                    if *kind == 13 {
                        "Enchantments"
                    } else {
                        "StoredEnchantments"
                    },
                    Tag::List(values),
                );
            }
            (17, Component::CustomModelData { floats, .. }) => {
                if let Some(value) = floats.first() {
                    d::set(&mut tag, "CustomModelData", Tag::Int(*value as i32));
                }
            }
            (18, Component::TooltipDisplay { hide: all, hidden }) => {
                if *all {
                    hide |= 255;
                }
                for kind in hidden {
                    hide |= match kind {
                        13 => 1,
                        16 => 2,
                        4 => 4,
                        15 => 8,
                        14 => 16,
                        45 => 32,
                        47 => 64,
                        58 => 128,
                        k if v1_21_4::ADDITIONAL.contains(k) => 32,
                        _ => 0,
                    };
                }
            }
            (21, Component::Bool(true)) => {
                if stack.component(component::ENCHANTMENTS).is_none() {
                    d::set(&mut tag, "Enchantments", Tag::List(Vec::new()));
                }
            }
            (47, Component::Int(value)) => set_sub(&mut tag, "display", "color", Tag::Int(*value)),
            (48, Component::Int(value)) => d::set(&mut tag, "map", Tag::Int(*value)),
            (51 | 52, Component::Stacks(values)) => {
                d::set(
                    &mut tag,
                    if *kind == 51 {
                        "ChargedProjectiles"
                    } else {
                        "Items"
                    },
                    Tag::List(values.iter().map(legacy_saved).collect::<Result<_>>()?),
                );
                if *kind == 51 {
                    d::set(&mut tag, "Charged", Tag::Byte(i8::from(!values.is_empty())));
                }
            }
            (
                53,
                Component::Potion {
                    potion,
                    color,
                    rest,
                },
            ) => {
                if let Some(id) = potion {
                    d::set(
                        &mut tag,
                        "Potion",
                        d::string(&registry_name("minecraft:potion", *id)?),
                    );
                }
                if let Some(color) = color {
                    d::set(&mut tag, "CustomPotionColor", Tag::Int(*color));
                }
                if !rest.is_empty() {
                    let mut r = Reader::new(rest);
                    let n = r.count(65536)?;
                    let effects = (0..n)
                        .map(|_| legacy_effect(&mut r))
                        .collect::<Result<Vec<_>>>()?;
                    if !effects.is_empty() {
                        d::set(&mut tag, "custom_potion_effects", Tag::List(effects));
                    }
                    if r.bool()? {
                        r.string(32767)?;
                    } // New custom potion names have no old field.
                }
            }
            (55, Component::StewEffects(values)) => d::set(
                &mut tag,
                "effects",
                Tag::List(
                    values
                        .iter()
                        .map(|(id, duration)| {
                            Ok(Tag::compound([
                                (
                                    "id",
                                    d::string(&registry_name("minecraft:mob_effect", *id)?),
                                ),
                                ("duration", Tag::Int(*duration)),
                            ]))
                        })
                        .collect::<Result<_>>()?,
                ),
            ),
            (56, Component::WritableBook { pages, filtered }) => {
                d::set(
                    &mut tag,
                    "pages",
                    Tag::List(pages.iter().map(|s| d::string(s)).collect()),
                );
                let mut filtered_pages = d::empty_map();
                for (i, value) in filtered.iter().enumerate() {
                    if let Some(value) = value {
                        d::set(&mut filtered_pages, &i.to_string(), d::string(value));
                    }
                }
                if !matches!(&filtered_pages,Tag::Compound(m) if m.is_empty()) {
                    d::set(&mut tag, "filtered_pages", filtered_pages);
                }
            }
            (57, Component::WrittenBook(book)) => {
                d::set(&mut tag, "title", d::string(&book.title));
                d::set(&mut tag, "author", d::string(&book.author));
                d::set(&mut tag, "generation", Tag::Int(book.generation));
                d::set(&mut tag, "resolved", Tag::Byte(i8::from(book.resolved)));
                if let Some(filtered) = &book.title_filtered {
                    d::set(&mut tag, "filtered_title", d::string(filtered));
                }
                d::set(
                    &mut tag,
                    "pages",
                    Tag::List(book.pages.iter().map(json_text).collect()),
                );
                let mut filtered_pages = d::empty_map();
                for (i, value) in book.pages_filtered.iter().enumerate() {
                    if let Some(value) = value {
                        d::set(&mut filtered_pages, &i.to_string(), json_text(value));
                    }
                }
                if !matches!(&filtered_pages,Tag::Compound(m) if m.is_empty()) {
                    d::set(&mut tag, "filtered_pages", filtered_pages);
                }
            }
            (58, Component::Trim { material, pattern }) => d::set(
                &mut tag,
                "Trim",
                Tag::compound([
                    (
                        "material",
                        d::string(&registry_name("minecraft:trim_material", *material)?),
                    ),
                    (
                        "pattern",
                        d::string(&registry_name("minecraft:trim_pattern", *pattern)?),
                    ),
                ]),
            ),
            (63, Component::Instrument(id)) => d::set(
                &mut tag,
                "instrument",
                d::string(&registry_name("minecraft:instrument", *id)?),
            ),
            (69, Component::LodestoneTracker { target, tracked }) => {
                if let Some((dimension, [x, y, z])) = target {
                    d::set(&mut tag, "LodestoneDimension", d::string(dimension));
                    d::set(
                        &mut tag,
                        "LodestonePos",
                        Tag::compound([
                            ("X", Tag::Int(*x)),
                            ("Y", Tag::Int(*y)),
                            ("Z", Tag::Int(*z)),
                        ]),
                    );
                }
                d::set(&mut tag, "LodestoneTracked", Tag::Byte(i8::from(*tracked)));
            }
            (70, Component::FireworkExplosion(value)) => {
                d::set(&mut tag, "Explosion", legacy_explosion(value))
            }
            (71, Component::Fireworks { flight, .. }) => {
                set_sub(&mut tag, "Fireworks", "Flight", Tag::Byte(*flight as i8))
            }
            (72, Component::Profile(profile)) => {
                d::set(&mut tag, "SkullOwner", legacy_profile(profile))
            }
            (74, Component::BannerPatterns(layers)) => {
                let layers = layers
                    .iter()
                    .map(|(id, color)| {
                        Ok(Tag::compound([
                            (
                                "Pattern",
                                d::string(banner_code(&registry_name(
                                    "minecraft:banner_pattern",
                                    *id,
                                )?)),
                            ),
                            ("Color", Tag::Int(*color)),
                        ]))
                    })
                    .collect::<Result<Vec<_>>>()?;
                set_sub(&mut tag, "BlockEntityTag", "Patterns", Tag::List(layers));
            }
            (75, Component::Int(color)) => {
                set_sub(&mut tag, "BlockEntityTag", "Base", Tag::Int(*color))
            }
            (kind, Component::Raw(bytes)) => legacy_raw(&mut tag, kind, bytes)?,
            _ => {}
        }
        if let (71, Component::Fireworks { explosions, .. }) = (*kind, value)
            && !explosions.is_empty()
        {
            set_sub(
                &mut tag,
                "Fireworks",
                "Explosions",
                Tag::List(explosions.iter().map(legacy_explosion).collect()),
            );
        }
    }
    if hide != 0 {
        d::set(&mut tag, "HideFlags", Tag::Int(hide));
    }
    Ok(tag)
}
fn legacy_explosion(value: &FireworkExplosion) -> Tag {
    let mut out = Tag::compound([
        ("Type", Tag::Byte(value.shape as i8)),
        ("Colors", Tag::IntArray(value.colors.clone())),
    ]);
    if !value.fade.is_empty() {
        d::set(&mut out, "FadeColors", Tag::IntArray(value.fade.clone()));
    }
    if value.trail {
        d::set(&mut out, "Trail", Tag::Byte(1));
    }
    if value.twinkle {
        d::set(&mut out, "Flicker", Tag::Byte(1));
    }
    out
}
fn legacy_profile(profile: &Profile) -> Tag {
    let mut tag = d::empty_map();
    if let Some(name) = &profile.name {
        d::set(&mut tag, "Name", d::string(name));
    }
    if let Some(id) = profile.id {
        d::set(&mut tag, "Id", uuid_tag(id));
    }
    let mut properties = d::empty_map();
    for (name, value, signature) in &profile.properties {
        let mut entry = Tag::compound([("Value", d::string(value))]);
        if let Some(signature) = signature {
            d::set(&mut entry, "Signature", d::string(signature));
        }
        if properties.get(name).is_none() {
            d::set(&mut properties, name, Tag::List(Vec::new()));
        }
        if let Some(Tag::List(values)) = properties.get_mut(name) {
            values.push(entry);
        }
    }
    if !profile.properties.is_empty() {
        d::set(&mut tag, "Properties", properties);
    }
    tag
}
fn legacy_effect(r: &mut Reader) -> Result<Tag> {
    let id = r.varint()?;
    let mut details = Vec::new();
    loop {
        let amplifier = r.varint()?;
        let duration = r.varint()?;
        let ambient = r.bool()?;
        let particles = r.bool()?;
        let icon = r.bool()?;
        details.push(Tag::compound([
            ("amplifier", Tag::Byte(amplifier as i8)),
            ("duration", Tag::Int(duration)),
            ("ambient", Tag::Byte(i8::from(ambient))),
            ("show_particles", Tag::Byte(i8::from(particles))),
            ("show_icon", Tag::Byte(i8::from(icon))),
        ]));
        if !r.bool()? {
            break;
        }
    }
    let mut result = details.pop().unwrap();
    while let Some(mut outer) = details.pop() {
        d::set(&mut outer, "hidden_effect", result);
        result = outer;
    }
    d::set(
        &mut result,
        "id",
        d::string(&registry_name("minecraft:mob_effect", id)?),
    );
    Ok(result)
}

fn bees(value: &Tag) -> Result<Component> {
    let entries = list(Some(value));
    let mut w = Writer::new();
    w.varint(entries.len() as i32);
    for entry in entries {
        let data = entry
            .get("entity_data")
            .ok_or_else(|| Error::new("bee has no entity data"))?;
        let Component::Raw(data) = typed_data(data, "minecraft:entity_type")? else {
            unreachable!();
        };
        w.bytes(&data);
        w.varint(number(entry.get("ticks_in_hive"), 0));
        w.varint(number(entry.get("min_ticks_in_hive"), 0));
    }
    Ok(Component::Raw(w.data))
}
fn legacy_raw(tag: &mut Tag, kind: i32, bytes: &[u8]) -> Result<()> {
    let mut r = Reader::new(bytes);
    match kind {
        49 => {
            const ICONS: &[&str] = &[
                "player",
                "frame",
                "red_marker",
                "blue_marker",
                "target_x",
                "target_point",
                "player_off_map",
                "player_off_limits",
                "mansion",
                "monument",
                "banner_white",
                "banner_orange",
                "banner_magenta",
                "banner_light_blue",
                "banner_yellow",
                "banner_lime",
                "banner_pink",
                "banner_gray",
                "banner_light_gray",
                "banner_cyan",
                "banner_purple",
                "banner_blue",
                "banner_brown",
                "banner_green",
                "banner_red",
                "banner_black",
                "red_x",
                "village_desert",
                "village_plains",
                "village_savanna",
                "village_snowy",
                "village_taiga",
                "jungle_temple",
                "swamp_hut",
            ];
            let mut entries = Vec::new();
            if let Tag::Compound(values) = r.nbt()? {
                for (id, value) in values {
                    let ty = text(value.get("type")).and_then(|n| {
                        ICONS
                            .iter()
                            .position(|v| *v == n.strip_prefix("minecraft:").unwrap_or(&n))
                    });
                    if let Some(ty) = ty {
                        entries.push(Tag::compound([
                            ("id", Tag::String(id)),
                            ("type", Tag::Byte(ty as i8)),
                            (
                                "x",
                                Tag::Double(d::as_double(value.get("x"), 0.0).to_bits()),
                            ),
                            (
                                "z",
                                Tag::Double(d::as_double(value.get("z"), 0.0).to_bits()),
                            ),
                            (
                                "rot",
                                Tag::Double(d::as_double(value.get("rotation"), 0.0).to_bits()),
                            ),
                        ]));
                    }
                }
            }
            d::set(tag, "Decorations", Tag::List(entries));
        }
        14 | 15 => d::set(
            tag,
            if kind == 14 {
                "CanPlaceOn"
            } else {
                "CanDestroy"
            },
            Tag::List(
                legacy_predicates(&mut r)?
                    .iter()
                    .map(|s| d::string(s))
                    .collect(),
            ),
        ),
        16 => d::set(
            tag,
            "AttributeModifiers",
            Tag::List(legacy_attributes(&mut r)?),
        ),
        59 => d::set(tag, "DebugProperty", r.nbt()?),
        60 | 62 => {
            let id = r.varint()?;
            let mut data = r.nbt()?;
            if let Some(name) = data.get("CustomName").cloned() {
                d::set(&mut data, "CustomName", json_text(&name));
            }
            d::set(
                &mut data,
                "id",
                d::string(&registry_name(
                    if kind == 60 {
                        "minecraft:entity_type"
                    } else {
                        "minecraft:block_entity_type"
                    },
                    id,
                )?),
            );
            d::set(
                tag,
                if kind == 60 {
                    "EntityTag"
                } else {
                    "BlockEntityTag"
                },
                data,
            );
        }
        61 => {
            if let Tag::Compound(data) = r.nbt()? {
                for (key, value) in data {
                    d::set(tag, &key.to_string_lossy(), value);
                }
            }
        }
        68 => d::set(tag, "Recipes", r.nbt()?),
        76 => {
            let mut sherds = Vec::new();
            for _ in 0..4 {
                sherds.push(if r.bool()? {
                    d::string(&registry_name(
                        "minecraft:item",
                        Template::read(&mut r)?.item,
                    )?)
                } else {
                    d::string("minecraft:brick")
                });
            }
            set_sub(tag, "BlockEntityTag", "sherds", Tag::List(sherds));
        }
        77 => {
            let mut items = Vec::new();
            for slot in 0..r.count(256)? {
                if r.bool()? {
                    let mut item = legacy_saved(&Template::read(&mut r)?)?;
                    d::set(&mut item, "Slot", Tag::Byte(slot as i8));
                    items.push(item);
                }
            }
            set_sub(tag, "BlockEntityTag", "Items", Tag::List(items));
        }
        78 => {
            let mut state = d::empty_map();
            for _ in 0..r.count(65536)? {
                let key = r.string(32767)?;
                d::set(&mut state, &key, d::string(&r.string(32767)?));
            }
            d::set(tag, "BlockStateTag", state);
        }
        79 => {
            let mut bees = Vec::new();
            for _ in 0..r.count(65536)? {
                let id = r.varint()?;
                let mut data = r.nbt()?;
                if let Some(name) = data.get("CustomName").cloned() {
                    d::set(&mut data, "CustomName", json_text(&name));
                }
                d::set(
                    &mut data,
                    "id",
                    d::string(&registry_name("minecraft:entity_type", id)?),
                );
                bees.push(Tag::compound([
                    ("EntityData", data),
                    ("TicksInHive", Tag::Int(r.varint()?)),
                    ("MinOccupationTicks", Tag::Int(r.varint()?)),
                ]));
            }
            set_sub(tag, "BlockEntityTag", "Bees", Tag::List(bees));
        }
        81 => {
            let value = r.nbt()?;
            if let Some(Tag::String(value)) = value
                .get("components")
                .and_then(|c| c.get("minecraft:custom_name"))
                && let Some(datafix::json::Json::Str(name)) = datafix::json::parse_strict(&value.0)
            {
                set_sub(
                    tag,
                    "BlockEntityTag",
                    "Lock",
                    Tag::String(horizon_core::nbt::JavaString(name)),
                );
            }
        }
        82 => {
            let value = r.nbt()?;
            if let Some(table) = value.get("loot_table") {
                set_sub(tag, "BlockEntityTag", "LootTable", table.clone());
            }
            if let Some(seed) = value.get("seed") {
                set_sub(tag, "BlockEntityTag", "LootTableSeed", seed.clone());
            }
        }
        // New components without an old NBT equivalent cannot be represented by this wire version.
        _ => {}
    }
    Ok(())
}
fn legacy_attributes(r: &mut Reader) -> Result<Vec<Tag>> {
    let mut attributes = Vec::new();
    for _ in 0..r.count(65536)? {
        let attribute = registry_name("minecraft:attribute", r.varint()?)?;
        let id = r.string(32767)?;
        let amount = r.f64()?;
        let operation = r.varint()?;
        let slot = r.varint()?;
        match r.varint()? {
            0 | 1 => {}
            2 => {
                r.nbt()?;
            }
            _ => return Err(Error::new("unknown modifier display")),
        }
        let mut value = Tag::compound([
            ("AttributeName", d::string(&attribute)),
            ("Name", d::string(v1_20_5::modifier_name(&id))),
            ("UUID", uuid_tag(v1_20_5::modifier_uuid(&id))),
            ("Amount", Tag::Double(amount.to_bits())),
            ("Operation", Tag::Int(operation)),
        ]);
        if let Some(name) = usize::try_from(slot).ok().and_then(|i| SLOTS.get(i))
            && slot != 0
        {
            d::set(&mut value, "Slot", d::string(name));
        }
        attributes.push(value);
    }
    Ok(attributes)
}
fn legacy_predicates(r: &mut Reader) -> Result<Vec<String>> {
    let mut result = Vec::new();
    for _ in 0..r.count(65536)? {
        let mut blocks = Vec::new();
        if r.bool()? {
            let n = r.varint()?;
            if n == 0 {
                blocks.push(format!("#{}", r.string(32767)?));
            } else {
                let count =
                    usize::try_from(n - 1).map_err(|_| Error::new("negative holder set"))?;
                if count > r.remaining() {
                    return Err(Error::new("holder set past end"));
                }
                for _ in 0..count {
                    blocks.push(registry_name("minecraft:block", r.varint()?)?);
                }
            }
        }
        let mut states = Vec::new();
        let mut representable = true;
        if r.bool()? {
            for _ in 0..r.count(65536)? {
                let key = r.string(32767)?;
                if r.bool()? {
                    states.push(format!("{key}={}", r.string(32767)?));
                } else {
                    for _ in 0..2 {
                        if r.bool()? {
                            r.string(32767)?;
                        }
                    }
                    representable = false;
                }
            }
        }
        let nbt = if r.bool()? {
            horizon_world::snbt_text::to_snbt(&r.nbt()?)
        } else {
            String::new()
        };
        // Consume modern matchers even though this older syntax cannot express them.
        let start = r.position();
        skip::component_matchers(r)?;
        if r.since(start) != [0, 0] {
            representable = false;
        }
        if representable {
            let state = if states.is_empty() {
                String::new()
            } else {
                format!("[{}]", states.join(","))
            };
            for block in blocks {
                result.push(format!("{block}{state}{nbt}"));
            }
        }
    }
    Ok(result)
}
fn banner_code(name: &str) -> &str {
    match name.strip_prefix("minecraft:").unwrap_or(name) {
        "base" => "b",
        "square_bottom_left" => "bl",
        "square_bottom_right" => "br",
        "square_top_left" => "tl",
        "square_top_right" => "tr",
        "stripe_bottom" => "bs",
        "stripe_top" => "ts",
        "stripe_left" => "ls",
        "stripe_right" => "rs",
        "stripe_center" => "cs",
        "stripe_middle" => "ms",
        "stripe_downright" => "drs",
        "stripe_downleft" => "dls",
        "small_stripes" => "ss",
        "cross" => "cr",
        "straight_cross" => "sc",
        "triangle_bottom" => "bt",
        "triangle_top" => "tt",
        "triangles_bottom" => "bts",
        "triangles_top" => "tts",
        "diagonal_left" => "ld",
        "diagonal_up_right" => "rd",
        "diagonal_up_left" => "lud",
        "diagonal_right" => "rud",
        "circle" => "mc",
        "rhombus" => "mr",
        "half_vertical" => "vh",
        "half_horizontal" => "hh",
        "half_vertical_right" => "vhr",
        "half_horizontal_bottom" => "hhb",
        "border" => "bo",
        "curly_border" => "cbo",
        "gradient" => "gra",
        "gradient_up" => "gru",
        "bricks" => "bri",
        "globe" => "glb",
        "creeper" => "cre",
        "skull" => "sku",
        "flower" => "flo",
        "mojang" => "moj",
        "piglin" => "pig",
        _ => name,
    }
}

#[cfg(test)]
mod tests;
