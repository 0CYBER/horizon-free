//! Registry id tables. Built-in registries (items, entity types, menus, sound
//! events ...) have fixed network ids, read from the data generator's
//! registries report. Data-driven registries (dimension types, biomes, chat
//! types, damage types ...) are numbered in the order the server sends them
//! during configuration.
use crate::{
    Error, Result,
    packets::configuration::{RegistryData, UpdateTags},
};
use horizon_core::identifier::Identifier;
use horizon_core::nbt::{JavaString, Tag};
use std::{collections::HashMap, path::Path};

/// One registry: names by network id and the reverse lookup.
#[derive(Clone, Debug, Default)]
pub struct Ids {
    pub names: Vec<String>,
    pub ids: HashMap<String, i32>,
}
impl Ids {
    pub fn from_names(names: Vec<String>) -> Self {
        let ids = names
            .iter()
            .enumerate()
            .map(|(i, n)| (n.clone(), i as i32))
            .collect();
        Self { names, ids }
    }
    pub fn name(&self, id: i32) -> Option<&str> {
        usize::try_from(id)
            .ok()
            .and_then(|i| self.names.get(i))
            .map(String::as_str)
    }
    pub fn id(&self, name: &str) -> Option<i32> {
        self.ids.get(name).copied()
    }
}

/// The built-in registries from `registries.json` (`--reports`).
#[derive(Clone, Debug, Default)]
pub struct Builtin {
    pub registries: HashMap<String, Ids>,
}
/// The names of both kinds of registry a connection knows: the built-in ones
/// first (as a hub's `SyncData::id` does), then those the server synced.
pub struct SyncedNames<'a> {
    pub builtin: &'a Builtin,
    pub dynamic: &'a Dynamic,
}
impl crate::item::Names for SyncedNames<'_> {
    fn name(&self, registry: &str, id: i32) -> Option<String> {
        self.builtin
            .get(registry)
            .and_then(|ids| ids.name(id))
            .or_else(|| self.dynamic.name(registry, id))
            .or_else(|| {
                (self.dynamic.custom_components && registry == "minecraft:data_component_type")
                    .then(|| crate::item::custom::name(id))
                    .flatten()
            })
            .map(str::to_owned)
    }
}

impl SyncedNames<'_> {
    /// The network id of `name` in `registry`, the reverse of `Names::name`: the built-in
    /// registries first, then those the server synced, then the custom data component types.
    /// None for an unknown registry or name. Compares the identifiers' parts in place: nothing is
    /// allocated, and a gap in a built-in report (an empty name) matches nothing.
    pub fn id(&self, registry: &Identifier, name: &Identifier) -> Option<i32> {
        let index = |i: usize| i32::try_from(i).ok();
        self.builtin
            .registries
            .iter()
            .find(|(r, _)| names(r, registry))
            .and_then(|(_, ids)| ids.names.iter().position(|n| names(n, name)))
            .and_then(index)
            .or_else(|| {
                self.dynamic
                    .registries
                    .iter()
                    .find(|(r, _)| names(r, registry))
                    .and_then(|(_, entries)| entries.iter().position(|(n, _)| names(n, name)))
                    .and_then(index)
            })
            .or_else(|| {
                (self.dynamic.custom_components && names("minecraft:data_component_type", registry))
                    .then(|| {
                        crate::item::custom::entries()
                            .find(|(_, n)| names(n, name))
                            .map(|(id, _)| id)
                    })
                    .flatten()
            })
    }
}

/// Whether the full id `text` (`namespace:path`) is `id`.
fn names(text: &str, id: &Identifier) -> bool {
    text.split_once(':') == Some((id.namespace(), id.path()))
}

impl Builtin {
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| Error::new(format!("{}: {e}", path.display())))?;
        Self::parse(&text)
    }
    pub fn parse(text: &str) -> Result<Self> {
        let json: serde_json::Value = serde_json::from_str(text)
            .map_err(|e| Error::new(format!("registries report: {e}")))?;
        let mut registries = HashMap::new();
        for (name, registry) in json
            .as_object()
            .ok_or_else(|| Error::new("registries report is not an object"))?
        {
            let Some(entries) = registry["entries"].as_object() else {
                continue;
            };
            let mut names = vec![String::new(); entries.len()];
            for (entry, value) in entries {
                let id = value["protocol_id"]
                    .as_u64()
                    .ok_or_else(|| Error::new(format!("{entry}: no protocol_id")))?
                    as usize;
                // No registry has a million entries; an id beyond that is
                // a crafted report, and resizing to it would allocate
                // gigabytes for one entry.
                if id > 1_000_000 {
                    return Err(Error::new(format!(
                        "{entry}: protocol_id {id} out of range"
                    )));
                }
                if id >= names.len() {
                    names.resize(id + 1, String::new());
                }
                if !names[id].is_empty() {
                    return Err(Error::new(format!("{entry}: duplicate protocol_id {id}")));
                }
                names[id] = entry.clone();
            }
            registries.insert(name.clone(), Ids::from_names(names));
        }
        Ok(Self { registries })
    }
    pub fn get(&self, registry: &str) -> Option<&Ids> {
        self.registries.get(registry)
    }
    /// Look for the report next to prepared resources, then in the data
    /// generator output of a development checkout.
    pub fn find(resources: &Path) -> Option<std::path::PathBuf> {
        let candidates = [
            resources.join("registries.json"),
            resources.join("../../datagen/out/reports/registries.json"),
            resources.join("../datagen/out/reports/registries.json"),
        ];
        candidates.into_iter().find(|p| p.is_file())
    }
}

/// Registries synchronized during configuration, in arrival order.
#[derive(Clone, Debug, Default)]
pub struct Dynamic {
    pub registries: HashMap<String, Vec<(String, Option<Tag>)>>,
    /// The tags the server sent, by registry: each tag's entries by id.
    pub tags: HashMap<String, HashMap<String, Vec<i32>>>,
    /// The peer confirmed the native component registry identity.
    pub custom_components: bool,
    /// The `horizon:menu_extra` payload layout the peer serves: the revision its
    /// configuration query carried, 0 when it never asked (the legacy layout).
    pub menu_extra_revision: u8,
}
/// The most registries whose tags one connection keeps. Vanilla names well under a hundred; a
/// server naming more is not a registry list, and the bound keeps a hostile one from growing the
/// client's memory with every packet.
pub const MAX_TAG_REGISTRIES: usize = 512;

impl Dynamic {
    /// Retained heap charge for a frozen configuration snapshot built by
    /// [`Self::add`] and [`Self::add_tags`] with insertion/replacement only.
    ///
    /// All outer and nested hash tables must be fresh, with no erasure history.
    /// This method does not enforce that builder precondition on public fields.
    /// It charges conservative hash table storage plus actual owned string/vector
    /// capacities and recursive NBT allocations. It excludes inline `Dynamic`
    /// storage, a containing Arc header and allocator metadata. Builtin registries
    /// and other global caches are not traversed. This is not an allocator census.
    pub fn fresh_owned_heap_bytes(&self) -> usize {
        use crate::retained::{OwnedHeap, fresh_hash_table_bytes};
        let Self {
            registries,
            tags,
            custom_components: _,
            menu_extra_revision: _,
        } = self;
        let registry_bytes = registries.iter().fold(
            fresh_hash_table_bytes::<(String, Vec<(String, Option<Tag>)>)>(registries.capacity()),
            |sum, (name, entries)| {
                sum.saturating_add(name.owned_heap_bytes())
                    .saturating_add(entries.owned_heap_bytes())
            },
        );
        tags.iter().fold(
            registry_bytes
                .saturating_add(
                    fresh_hash_table_bytes::<(String, HashMap<String, Vec<i32>>)>(tags.capacity()),
                ),
            |sum, (registry, entries)| {
                entries.iter().fold(
                    sum.saturating_add(registry.owned_heap_bytes())
                        .saturating_add(fresh_hash_table_bytes::<(String, Vec<i32>)>(
                            entries.capacity(),
                        )),
                    |sum, (name, ids)| {
                        sum.saturating_add(name.owned_heap_bytes())
                            .saturating_add(ids.owned_heap_bytes())
                    },
                )
            },
        )
    }

    /// A `ClientboundUpdateTagsPacket`: the registries it names replace their
    /// tags, the others keep theirs. A registry beyond [`MAX_TAG_REGISTRIES`] is dropped.
    pub fn add_tags(&mut self, packet: &UpdateTags) {
        for r in &packet.registries {
            if self.tags.len() >= MAX_TAG_REGISTRIES && !self.tags.contains_key(&r.registry) {
                continue;
            }
            let tags = r.tags.iter().map(|t| (t.name.clone(), t.ids.clone()));
            self.tags.insert(r.registry.clone(), tags.collect());
        }
    }
    /// The entries of a tag by id.
    pub fn tag(&self, registry: &str, tag: &str) -> Option<&[i32]> {
        self.tags.get(registry)?.get(tag).map(Vec::as_slice)
    }
    pub fn add(&mut self, packet: RegistryData) {
        let entries = packet.entries.into_iter().map(|e| (e.id, e.data)).collect();
        self.registries.insert(packet.registry, entries);
    }
    pub fn name(&self, registry: &str, id: i32) -> Option<&str> {
        let i = usize::try_from(id).ok()?;
        self.registries
            .get(registry)?
            .get(i)
            .map(|(n, _)| n.as_str())
    }
    /// The id of an entry by name, the reverse of `name`. A registry has one
    /// entry per id, so the first match is the only one.
    pub fn id(&self, registry: &str, name: &str) -> Option<i32> {
        let list = self.registries.get(registry)?;
        list.iter().position(|(n, _)| n == name).map(|i| i as i32)
    }
    pub fn data(&self, registry: &str, id: i32) -> Option<&Tag> {
        let i = usize::try_from(id).ok()?;
        self.registries.get(registry)?.get(i)?.1.as_ref()
    }
    pub fn len(&self, registry: &str) -> usize {
        self.registries.get(registry).map_or(0, Vec::len)
    }
    pub fn is_empty(&self) -> bool {
        self.registries.is_empty()
    }
    /// The synchronized dimension type `id` as the JSON a data pack writes it, with its name:
    /// what `DimensionFacts::patched` reads. None without data. A byte is a flag (every one the
    /// type has is), and a number with no fraction is an integer, as Horizon's server sends a
    /// pack's JSON numbers as doubles.
    pub fn dimension_json(&self, id: i32) -> Option<(String, serde_json::Value)> {
        let name = self.name("minecraft:dimension_type", id)?.to_owned();
        let tag = self.data("minecraft:dimension_type", id)?;
        Some((name, tag_json(tag)))
    }
    /// The dimension type's vertical range and sky light, from its
    /// synchronized data or the vanilla defaults for known names.
    pub fn dimension(&self, id: i32) -> DimensionType {
        let name = self
            .name("minecraft:dimension_type", id)
            .unwrap_or("minecraft:overworld");
        let mut d = DimensionType::vanilla(name);
        if let Some(Tag::Compound(map)) = self.data("minecraft:dimension_type", id) {
            let int = |k: &str| match map.get(&JavaString::from(k)) {
                Some(Tag::Int(v)) => Some(*v),
                _ => None,
            };
            if let Some(v) = int("min_y") {
                d.min_y = v;
            }
            if let Some(v) = int("height") {
                d.height = v;
            }
            if let Some(v) = int("logical_height") {
                d.logical_height = v;
            }
            // DimensionType's codec refuses a shape outside these; the client sizes
            // its columns and light from them, so keep the named type's own.
            if !DimensionType::valid_shape(d.min_y, d.height) {
                let vanilla = DimensionType::vanilla(name);
                d.min_y = vanilla.min_y;
                d.height = vanilla.height;
            }
            if let Some(Tag::Byte(v)) = map.get(&JavaString::from("has_skylight")) {
                d.has_skylight = *v != 0;
            }
            if let Some(Tag::Byte(v)) = map.get(&JavaString::from("has_ceiling")) {
                d.has_ceiling = *v != 0;
            }
            // Any number, as vanilla's codec reads it (Horizon's own server sends a
            // world pack's JSON numbers as doubles).
            d.ambient_light = match map.get(&JavaString::from("ambient_light")) {
                Some(Tag::Float(v)) => f32::from_bits(*v),
                Some(Tag::Double(v)) => f64::from_bits(*v) as f32,
                Some(Tag::Int(v)) => *v as f32,
                Some(Tag::Byte(v)) => f32::from(*v),
                _ => d.ambient_light,
            };
            // DimensionType.defaultClock: the world clock this level's day
            // time follows (absent in the Nether).
            d.default_clock = match map.get(&JavaString::from("default_clock")) {
                Some(Tag::String(s)) => Some(s.to_string_lossy()),
                Some(Tag::Compound(c)) => match c.get(&JavaString::from("id")) {
                    Some(Tag::String(s)) => Some(s.to_string_lossy()),
                    _ => d.default_clock,
                },
                _ => d.default_clock,
            };
        }
        d
    }
}
/// A dimension type's tag as JSON (see `Dynamic::dimension_json`).
fn tag_json(tag: &Tag) -> serde_json::Value {
    use serde_json::{Number, Value};
    let number = |v: f64| {
        if v.fract() == 0.0 && v.abs() < 9e15 {
            Value::from(v as i64)
        } else {
            Number::from_f64(v).map_or(Value::Null, Value::Number)
        }
    };
    match tag {
        Tag::End => Value::Null,
        Tag::Byte(v) => Value::Bool(*v != 0),
        Tag::Short(v) => Value::from(*v),
        Tag::Int(v) => Value::from(*v),
        Tag::Long(v) => Value::from(*v),
        Tag::Float(v) => number(f64::from(f32::from_bits(*v))),
        Tag::Double(v) => number(f64::from_bits(*v)),
        Tag::String(s) => Value::String(s.to_string_lossy()),
        Tag::List(items) => Value::Array(items.iter().map(tag_json).collect()),
        Tag::Compound(map) => Value::Object(
            map.iter()
                .map(|(k, v)| (k.to_string_lossy(), tag_json(v)))
                .collect(),
        ),
        Tag::ByteArray(v) => Value::Array(v.iter().map(|b| Value::from(*b)).collect()),
        Tag::IntArray(v) => Value::Array(v.iter().map(|n| Value::from(*n)).collect()),
        Tag::LongArray(v) => Value::Array(v.iter().map(|n| Value::from(*n)).collect()),
    }
}

/// The parts of a DimensionType the client needs for chunk layout.
#[derive(Clone, Debug, PartialEq)]
pub struct DimensionType {
    pub name: String,
    pub min_y: i32,
    pub height: i32,
    pub logical_height: i32,
    pub has_skylight: bool,
    /// `DimensionType.hasCeiling`: weather, spawning and maps read it.
    pub has_ceiling: bool,
    /// `DimensionType.ambientLight`: the light every block gets, whatever its light level.
    pub ambient_light: f32,
    /// `DimensionType.defaultClock`: the `minecraft:world_clock` this level
    /// samples for day time (`None` in the Nether).
    pub default_clock: Option<String>,
}
impl DimensionType {
    /// `DimensionType.MIN_Y`, `MAX_Y` and `MIN_HEIGHT`: a type's `min_y` and
    /// `height` are multiples of 16, at least 16 high, inside -2032..=2031.
    pub fn valid_shape(min_y: i32, height: i32) -> bool {
        min_y % 16 == 0
            && height % 16 == 0
            && height >= 16
            && min_y >= -2032
            && i64::from(min_y) + i64::from(height) <= 2032
    }
    pub fn vanilla(name: &str) -> Self {
        let (min_y, height, logical_height, has_skylight, has_ceiling, default_clock) = match name {
            "minecraft:the_nether" => (0, 256, 128, false, true, None),
            "minecraft:the_end" => (0, 256, 256, true, false, Some("minecraft:the_end")),
            // overworld_caves builds like the Overworld but has a ceiling.
            "minecraft:overworld_caves" => (-64, 384, 384, true, true, Some("minecraft:overworld")),
            _ => (-64, 384, 384, true, false, Some("minecraft:overworld")),
        };
        let ambient_light = match name {
            "minecraft:the_nether" => 0.1,
            "minecraft:the_end" => 0.25,
            _ => 0.0,
        };
        Self {
            name: name.to_owned(),
            min_y,
            height,
            logical_height,
            has_skylight,
            has_ceiling,
            ambient_light,
            default_clock: default_clock.map(str::to_owned),
        }
    }
    pub fn sections(&self) -> usize {
        (self.height / 16) as usize
    }
}

/// Mth.ceillog2(n): bits for a global palette over `n` values.
pub fn ceil_log2(n: usize) -> u8 {
    if n <= 1 {
        0
    } else {
        (usize::BITS - (n - 1).leading_zeros()) as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packets::configuration::RegistryEntry;

    #[test]
    fn tags_replace_only_the_registries_a_packet_names() {
        let packet = |registry: &str, name: &str, ids: Vec<i32>| UpdateTags {
            registries: vec![crate::packets::configuration::RegistryTags {
                registry: registry.into(),
                tags: vec![crate::packets::configuration::TagEntry {
                    name: name.into(),
                    ids,
                }],
            }],
        };
        let mut dynamic = Dynamic::default();
        dynamic.add_tags(&packet("minecraft:dialog", "minecraft:a", vec![1, 2]));
        dynamic.add_tags(&packet("minecraft:item", "minecraft:b", vec![3]));
        dynamic.add_tags(&packet("minecraft:item", "minecraft:c", vec![4]));
        assert_eq!(
            dynamic.tag("minecraft:dialog", "minecraft:a"),
            Some(&[1, 2][..])
        );
        assert_eq!(dynamic.tag("minecraft:item", "minecraft:b"), None);
        assert_eq!(dynamic.tag("minecraft:item", "minecraft:c"), Some(&[4][..]));
    }

    #[test]
    fn tag_registries_are_bounded_and_known_ones_still_update() {
        let packet = |registry: String, id: i32| UpdateTags {
            registries: vec![crate::packets::configuration::RegistryTags {
                registry,
                tags: vec![crate::packets::configuration::TagEntry {
                    name: "minecraft:t".into(),
                    ids: vec![id],
                }],
            }],
        };
        let mut dynamic = Dynamic::default();
        dynamic.add_tags(&packet("minecraft:item".into(), 1));
        for i in 0..2 * MAX_TAG_REGISTRIES {
            dynamic.add_tags(&packet(format!("attack:registry_{i}"), 0));
        }
        assert_eq!(dynamic.tags.len(), MAX_TAG_REGISTRIES);
        // A registry kept before the bound is still replaced, and the others keep theirs.
        dynamic.add_tags(&packet("minecraft:item".into(), 7));
        assert_eq!(dynamic.tag("minecraft:item", "minecraft:t"), Some(&[7][..]));
        assert_eq!(dynamic.tags.len(), MAX_TAG_REGISTRIES);
        assert_eq!(
            dynamic.tag("attack:registry_0", "minecraft:t"),
            Some(&[0][..])
        );
        assert_eq!(dynamic.tag("attack:registry_600", "minecraft:t"), None);
    }

    #[test]
    fn synced_names_look_up_builtin_then_synced_registries() {
        use crate::item::Names;
        let report = r#"{"minecraft:item":{"default":"minecraft:air","entries":{"minecraft:stone":{"protocol_id":1},"minecraft:air":{"protocol_id":0}},"protocol_id":5}}"#;
        let builtin = Builtin::parse(report).unwrap();
        let dynamic = Dynamic::default();
        let names = SyncedNames {
            builtin: &builtin,
            dynamic: &dynamic,
        };
        assert_eq!(
            names.name("minecraft:item", 1).as_deref(),
            Some("minecraft:stone")
        );
        assert_eq!(names.name("minecraft:item", 2), None);
        assert_eq!(names.name("minecraft:item", -1), None);
        assert_eq!(names.name("minecraft:unknown", 0), None);
    }

    #[test]
    fn synced_ids_are_the_reverse_of_synced_names() {
        let id = |text: &str| Identifier::parse_namespaced(text).unwrap();
        // Ids 0, 1, 2 and 4 are gaps: their empty names must not answer an id.
        let report = r#"{"minecraft:particle_type":{"entries":{"minecraft:flame":{"protocol_id":3},"mymod:spark":{"protocol_id":5}}}}"#;
        let builtin = Builtin::parse(report).unwrap();
        let mut dynamic = Dynamic::default();
        let names = SyncedNames {
            builtin: &builtin,
            dynamic: &dynamic,
        };
        let particles = id("minecraft:particle_type");
        let damage = id("minecraft:damage_type");
        assert_eq!(names.id(&particles, &id("minecraft:flame")), Some(3));
        assert_eq!(names.id(&particles, &id("mymod:spark")), Some(5));
        assert_eq!(names.id(&particles, &id("minecraft:unknown")), None);
        assert_eq!(
            names.id(&id("minecraft:unknown"), &id("minecraft:flame")),
            None
        );
        // Before the server synced it, a data registry answers nothing.
        assert_eq!(names.id(&damage, &id("minecraft:cactus")), None);
        // Synced out of alphabetical order: the ids are the order sent.
        dynamic.add(RegistryData {
            registry: "minecraft:damage_type".into(),
            entries: ["minecraft:in_fire", "minecraft:cactus", "minecraft:arrow"]
                .into_iter()
                .map(|name| RegistryEntry {
                    id: name.into(),
                    data: None,
                })
                .collect(),
        });
        let names = SyncedNames {
            builtin: &builtin,
            dynamic: &dynamic,
        };
        assert_eq!(names.id(&damage, &id("minecraft:in_fire")), Some(0));
        assert_eq!(names.id(&damage, &id("minecraft:cactus")), Some(1));
        assert_eq!(names.id(&damage, &id("minecraft:arrow")), Some(2));
        assert_eq!(names.id(&damage, &id("minecraft:unknown")), None);
    }

    #[test]
    fn parses_registry_reports() {
        let report = r#"{"minecraft:item":{"default":"minecraft:air","entries":{"minecraft:stone":{"protocol_id":1},"minecraft:air":{"protocol_id":0}},"protocol_id":5}}"#;
        let b = Builtin::parse(report).unwrap();
        let items = b.get("minecraft:item").unwrap();
        assert_eq!(items.name(1), Some("minecraft:stone"));
        assert_eq!(items.id("minecraft:air"), Some(0));
        assert_eq!(items.name(2), None);
    }

    /// A crafted huge protocol_id must error instead of allocating a
    /// name table of that size, and duplicate ids must error instead of
    /// silently overwriting.
    #[test]
    fn huge_or_duplicate_protocol_ids_are_rejected() {
        let huge =
            r#"{"minecraft:item":{"entries":{"minecraft:stone":{"protocol_id":2000000000}}}}"#;
        assert!(Builtin::parse(huge).is_err());
        let dup = r#"{"minecraft:item":{"entries":{"minecraft:stone":{"protocol_id":1},"minecraft:air":{"protocol_id":1}}}}"#;
        assert!(Builtin::parse(dup).is_err());
        // Sparse but sane ids (1 and 7, as LAN tests use) still parse.
        let sparse = r#"{"minecraft:block_entity_type":{"entries":{"minecraft:chest":{"protocol_id":1},"minecraft:sign":{"protocol_id":7}}}}"#;
        let b = Builtin::parse(sparse).unwrap();
        let x = b.get("minecraft:block_entity_type").unwrap();
        assert_eq!(x.name(7), Some("minecraft:sign"));
    }

    #[test]
    fn dynamic_registries_follow_arrival_order() {
        let mut d = Dynamic::default();
        d.add(RegistryData {
            registry: "minecraft:dimension_type".into(),
            entries: vec![
                RegistryEntry {
                    id: "minecraft:overworld".into(),
                    data: None,
                },
                RegistryEntry {
                    id: "minecraft:the_nether".into(),
                    data: Some(Tag::compound([
                        ("min_y", Tag::Int(0)),
                        ("height", Tag::Int(256)),
                        ("has_skylight", Tag::Byte(0)),
                    ])),
                },
            ],
        });
        assert_eq!(
            d.dimension(0),
            DimensionType::vanilla("minecraft:overworld")
        );
        assert_eq!(d.dimension(1).sections(), 16);
        assert!(!d.dimension(1).has_skylight);
        // The type's data leaves has_ceiling out; the Nether default holds.
        assert!(d.dimension(1).has_ceiling);
        assert_eq!(d.dimension(1).default_clock, None);
        // ...and so does its ambient light.
        assert_eq!(d.dimension(1).ambient_light, 0.1);
        assert_eq!(ceil_log2(35723), 16);
        assert_eq!(ceil_log2(65), 7);
        assert_eq!(ceil_log2(64), 6);
    }

    #[test]
    fn a_dimension_shape_the_codec_refuses_keeps_the_named_types() {
        let shape = |min_y: i32, height: i32| {
            let mut d = Dynamic::default();
            d.add(RegistryData {
                registry: "minecraft:dimension_type".into(),
                entries: vec![RegistryEntry {
                    id: "example:tall".into(),
                    data: Some(Tag::compound([
                        ("min_y", Tag::Int(min_y)),
                        ("height", Tag::Int(height)),
                    ])),
                }],
            });
            let t = d.dimension(0);
            (t.min_y, t.height)
        };
        // The tallest the codec takes, and a few it refuses.
        assert_eq!(shape(-2032, 4064), (-2032, 4064));
        assert_eq!(shape(0, 2_147_483_632), (-64, 384));
        assert_eq!(shape(0, 8), (-64, 384));
        assert_eq!(shape(-64, 100), (-64, 384));
        assert_eq!(shape(-2048, 256), (-64, 384));
        assert_eq!(shape(1024, 1024), (-64, 384));
    }

    #[test]
    fn dimension_reads_default_clock() {
        let mut d = Dynamic::default();
        d.add(RegistryData {
            registry: "minecraft:dimension_type".into(),
            entries: vec![RegistryEntry {
                id: "pack:custom".into(),
                data: Some(Tag::compound([
                    ("min_y", Tag::Int(-64)),
                    ("height", Tag::Int(384)),
                    ("logical_height", Tag::Int(384)),
                    ("has_skylight", Tag::Byte(1)),
                    ("default_clock", Tag::String("minecraft:overworld".into())),
                ])),
            }],
        });
        let dim = d.dimension(0);
        assert_eq!(dim.name, "pack:custom");
        assert_eq!(dim.default_clock.as_deref(), Some("minecraft:overworld"));
    }

    /// The server's dimension type reaches the game's facts through its JSON: a flag byte is a
    /// bool, a whole double an integer, and the attributes keep their keys.
    #[test]
    fn a_synchronized_type_becomes_the_games_facts() {
        use horizon_world::dimension_type::DimensionFacts;
        let mut d = Dynamic::default();
        d.add(RegistryData {
            registry: "minecraft:dimension_type".into(),
            entries: vec![RegistryEntry {
                id: "mymod:pocket".into(),
                data: Some(Tag::compound([
                    ("min_y", Tag::Int(0)),
                    ("height", Tag::Int(128)),
                    ("logical_height", Tag::Int(128)),
                    ("has_fixed_time", Tag::Byte(1)),
                    ("coordinate_scale", Tag::Double(2.0f64.to_bits())),
                    (
                        "attributes",
                        Tag::compound([
                            ("minecraft:gameplay/fast_lava", Tag::Byte(1)),
                            (
                                "minecraft:gameplay/sky_light_level",
                                Tag::Float(4.0f32.to_bits()),
                            ),
                        ]),
                    ),
                ])),
            }],
        });
        let (name, json) = d.dimension_json(0).unwrap();
        assert_eq!(name, "mymod:pocket");
        assert_eq!(json["has_fixed_time"], true);
        assert_eq!(json["coordinate_scale"], 2);
        let facts =
            DimensionFacts::patched(DimensionFacts::base_for_type(&name), &name, &json).unwrap();
        assert_eq!(facts.coordinate_scale, 2.0);
        assert!(facts.has_fixed_time && facts.fast_lava);
        assert_eq!(facts.sky_light_level, 4.0);
        assert_eq!(facts.height.height, 128);
        assert!(Dynamic::default().dimension_json(0).is_none());
    }

    #[test]
    fn dimension_reads_ambient_light() {
        let mut d = Dynamic::default();
        d.add(RegistryData {
            registry: "minecraft:dimension_type".into(),
            entries: vec![RegistryEntry {
                id: "pack:custom".into(),
                data: Some(Tag::compound([
                    ("height", Tag::Int(384)),
                    ("ambient_light", Tag::Float(0.375f32.to_bits())),
                ])),
            }],
        });
        assert_eq!(d.dimension(0).ambient_light, 0.375);
        // A world pack's JSON number, as Horizon's server sends it.
        d.add(RegistryData {
            registry: "minecraft:dimension_type".into(),
            entries: vec![RegistryEntry {
                id: "pack:custom".into(),
                data: Some(Tag::compound([(
                    "ambient_light",
                    Tag::Double(0.5f64.to_bits()),
                )])),
            }],
        });
        assert_eq!(d.dimension(0).ambient_light, 0.5);
    }
}
