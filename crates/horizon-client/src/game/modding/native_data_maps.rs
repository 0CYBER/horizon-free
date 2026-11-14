//! Data maps for native server mods (docs/modding/src/how-to/native-mods.md): values a data pack
//! attaches to the entries of a registry, under the types a mod registered
//! (`native_data_map_types`). Vanilla has no data maps; the contract is
//! NeoForge's `DataMapType`: files at
//! `data/<map namespace>/data_maps/<registry folder>/<map path>.json` hold
//! `replace`, `values` (entry ids and `#tags` to values, each value optionally
//! guarded by its own `neoforge:conditions`) and `remove`, and a newer value
//! overrides an older one for the same entry. A registry's folder is its path
//! for a `minecraft` registry and `<namespace>/<path>` otherwise. Any registry
//! the game names takes maps: built in, the packs' and content mods', and the
//! datapack registries mods declare. The files are the world's data packs' and
//! what mods contribute through `native_data_pack_files` (key
//! `data_maps/<ns>:<registry folder>/<map path>`), merged each time the
//! server's tables are built: a world's first open and every `/reload`.
use super::Game;
use super::data_map_json::to_tag;
use horizon_core::nbt::Tag;
pub use mod_api::game::DataMapType;
use serde_json::Value;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

/// The folder a registry's data maps are under (`DataMapLoader`): its path for
/// a `minecraft` registry, `<namespace>/<path>` for another.
fn folder(registry: &str) -> String {
    match registry.split_once(':') {
        Some(("minecraft", path)) => path.to_owned(),
        Some((namespace, path)) => format!("{namespace}/{path}"),
        None => registry.to_owned(),
    }
}

/// The registries mods read through a typed key (`DataMapEntry`): always
/// known, even where the command registries list no ids.
const TYPED: [&str; 8] = [
    "minecraft:item",
    "minecraft:block",
    "minecraft:fluid",
    "minecraft:mob_effect",
    "minecraft:entity_type",
    "minecraft:enchantment",
    "minecraft:block_entity_type",
    "minecraft:villager_profession",
];

/// The registries a data load resolves maps against.
pub(crate) trait MapRegistries {
    /// Whether the game names `registry`: built in, a pack's or a datapack
    /// registry a mod declared.
    fn known(&self, registry: &str) -> bool;
    /// Whether `entry` is in `registry`; true where the registry's ids aren't listed.
    fn contains(&self, registry: &str, entry: &str) -> bool;
    /// The members of `tag` (`minecraft:logs`) in `registry`; none for an unknown tag.
    fn tag(&self, registry: &str, tag: &str) -> Vec<String>;
}

/// A world's registries while its server tables build: every registry's ids
/// as the command registries hold them (built in, the packs' and content
/// mods'), the world's loaded datapack registries (the authority for a
/// mod-declared one the command ids do not list), the loaded item and block
/// tags, and another registry's tags read from the same packs on first use.
pub(crate) struct WorldRegistries<'a> {
    ids: BTreeMap<&'a str, &'a [String]>,
    datapack: Option<&'a mod_api::DatapackRegistrySet>,
    item_tags: &'a HashMap<String, HashSet<String>>,
    block_tags: &'a HashMap<String, HashSet<String>>,
    root: &'a std::path::Path,
    overlay: &'a horizon_world::loot::PackOverlay<'a>,
    id_sets: RefCell<HashMap<String, HashSet<String>>>,
    datapack_ids: RefCell<HashMap<String, HashSet<String>>>,
    tags: RefCell<HashMap<String, HashMap<String, HashSet<String>>>>,
}
impl<'a> WorldRegistries<'a> {
    pub(crate) fn new(
        ids: impl IntoIterator<Item = (&'a str, &'a [String])>,
        datapack: Option<&'a mod_api::DatapackRegistrySet>,
        items: &'a horizon_client::items::Items,
        root: &'a std::path::Path,
        overlay: &'a horizon_world::loot::PackOverlay<'a>,
    ) -> Self {
        Self {
            ids: ids.into_iter().collect(),
            datapack,
            item_tags: &items.item_tags,
            block_tags: &items.block_tags,
            root,
            overlay,
            id_sets: RefCell::default(),
            datapack_ids: RefCell::default(),
            tags: RefCell::default(),
        }
    }
    /// Whether a mod's declaration names `registry`.
    fn declared(&self, registry: &str) -> bool {
        mod_api::__private::datapack_registry_declarations()
            .iter()
            .any(|declaration| declaration.key == registry)
    }
    /// The loaded entries of a mod-declared datapack registry, read once.
    fn datapack_entries(&self, registry: &str) -> Option<HashSet<String>> {
        let mut sets = self.datapack_ids.borrow_mut();
        Some(
            sets.entry(registry.to_owned())
                .or_insert_with(|| {
                    self.datapack
                        .and_then(|set| set.entries(registry).ok())
                        .map(|entries| entries.into_iter().map(String::from).collect())
                        .unwrap_or_default()
                })
                .clone(),
        )
    }
}
impl MapRegistries for WorldRegistries<'_> {
    fn known(&self, registry: &str) -> bool {
        self.ids.contains_key(registry) || TYPED.contains(&registry) || self.declared(registry)
    }
    fn contains(&self, registry: &str, entry: &str) -> bool {
        if let Some(ids) = self.ids.get(registry).filter(|ids| !ids.is_empty()) {
            let mut sets = self.id_sets.borrow_mut();
            let set = sets
                .entry(registry.to_owned())
                .or_insert_with(|| ids.iter().cloned().collect());
            return set.contains(entry);
        }
        // A mod-declared datapack registry the command ids do not list: the
        // world's loaded entries are the authority, and an id not held is
        // refused (#3357's unknown-id rejection).
        if self.declared(registry) {
            return self
                .datapack_entries(registry)
                .is_some_and(|set| set.contains(entry));
        }
        // A typed registry the sync did not list: no source says its entries.
        true
    }
    fn tag(&self, registry: &str, tag: &str) -> Vec<String> {
        let members = |tags: &HashMap<String, HashSet<String>>| -> Vec<String> {
            tags.get(tag)
                .map(|set| set.iter().cloned().collect())
                .unwrap_or_default()
        };
        match registry {
            "minecraft:item" => members(self.item_tags),
            "minecraft:block" => members(self.block_tags),
            _ => {
                let mut loaded = self.tags.borrow_mut();
                let tags = loaded.entry(registry.to_owned()).or_insert_with(|| {
                    horizon_client::items::load_tags(self.root, &folder(registry), self.overlay)
                        .unwrap_or_else(|e| {
                            log::warn!("Horizon: the {registry} tags data maps read: {e}");
                            HashMap::new()
                        })
                });
                members(tags)
            }
        }
    }
}

/// The merged values of every registered type: registry, then type id, to the
/// value of each entry that has one.
pub(crate) enum MapValues {
    Json(BTreeMap<String, Value>),
    Decoded(BTreeMap<String, Tag>),
}
#[derive(Default)]
pub(crate) struct DataMaps {
    maps: BTreeMap<String, HashMap<String, MapValues>>,
    pub(super) mob_maps: std::sync::Arc<horizon_client::neoforge_mob_maps::NeoForgeMobMaps>,
}

/// `minecraft:` for an id written without a namespace.
fn full_id(id: &str) -> String {
    if id.contains(':') {
        id.to_owned()
    } else {
        format!("minecraft:{id}")
    }
}

/// A file's value for an entry: the data itself, or the wrapper
/// `{"value": ..., "replace": false}` (`DataMapEntry`'s codec), which may also
/// carry `neoforge:conditions` (`WithConditions`): a value whose conditions
/// fail is no value. `replace` only matters to mergers, which types don't have
/// yet: a newer value overrides.
fn entry_value(
    raw: &Value,
    conditions: &horizon_world::conditions::LoadConditions,
    path: &str,
) -> Option<Value> {
    if let Value::Object(map) = raw
        && let Some(value) = map.get("value")
        && map.iter().all(|(k, v)| {
            k == "value" || k == "neoforge:conditions" || (k == "replace" && v.is_boolean())
        })
    {
        return conditions.allows_file(raw, path).then(|| value.clone());
    }
    (!raw.is_null()).then(|| raw.clone())
}

/// One file over what earlier files made of the type: `replace` clears it,
/// `values` attach (a tag to every entry in it), then `remove` takes entries
/// away, as `DataMapFile` is applied. False when the file isn't a data map.
fn apply_file(
    map: &mut BTreeMap<String, Value>,
    file: &Value,
    registry: &str,
    registries: &dyn MapRegistries,
    conditions: &horizon_world::conditions::LoadConditions,
    path: &str,
) -> bool {
    let (Some(root), Some(values)) = (file.as_object(), file.get("values")) else {
        return false;
    };
    let Some(values) = values.as_object() else {
        return false;
    };
    if root.get("replace").and_then(Value::as_bool) == Some(true) {
        map.clear();
    }
    let mut unknown = BTreeSet::new();
    // The members of a tag or the entry itself, if the registry holds it.
    let mut members = |key: &str| -> Vec<String> {
        match key.strip_prefix('#') {
            // A tag's members go through the same membership check the entry
            // does: a tag naming an id the registry doesn't hold publishes
            // nothing for it.
            Some(tag) => registries
                .tag(registry, &full_id(tag))
                .into_iter()
                .filter(|member| {
                    let held = registries.contains(registry, member);
                    if !held {
                        unknown.insert(member.clone());
                    }
                    held
                })
                .collect(),
            None if registries.contains(registry, &full_id(key)) => vec![full_id(key)],
            None => {
                unknown.insert(full_id(key));
                Vec::new()
            }
        }
    };
    for (key, raw) in values {
        let Some(value) = entry_value(raw, conditions, path) else {
            continue;
        };
        for entry in members(key) {
            map.insert(entry, value.clone());
        }
    }
    for removal in root
        .get("remove")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        // An entry or `{"key": entry}`, as `DataMapEntry.Removal` reads it.
        let key = removal.as_str().or_else(|| removal["key"].as_str());
        for entry in key.map(&mut members).unwrap_or_default() {
            map.remove(&entry);
        }
    }
    if !unknown.is_empty() {
        log::warn!(
            "Horizon: {path}: {registry} has no {}: skipped",
            unknown.into_iter().collect::<Vec<_>>().join(", ")
        );
    }
    true
}

impl DataMaps {
    /// The types' values from `files` (the server's tables' pack files, in
    /// pack order, mods' below the packs'), ids and tags resolved against
    /// `registries`. A type for a registry the game doesn't name, or with a
    /// bad id, is left out with a warning; a file that isn't a data map is
    /// skipped alone.
    pub(crate) fn load(
        types: &[DataMapType],
        overlay: &horizon_world::loot::PackOverlay<'_>,
        registries: &dyn MapRegistries,
    ) -> Self {
        let mut maps: BTreeMap<String, HashMap<String, MapValues>> = BTreeMap::new();
        for ty in types {
            if !registries.known(&ty.registry) {
                log::warn!(
                    "Horizon: data map {} is for {}, a registry the game doesn't name: left out",
                    ty.id,
                    ty.registry
                );
                continue;
            }
            let valid = |s: &str| {
                !s.is_empty()
                    && s.chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "_-./".contains(c))
            };
            let ok = ty
                .id
                .split_once(':')
                .is_some_and(|(ns, path)| ns != "minecraft" && valid(ns) && valid(path));
            if !ok {
                log::warn!(
                    "Horizon: data map type {} is not a mod's id: left out",
                    ty.id
                );
                continue;
            }
            let registered = maps.entry(ty.registry.clone()).or_default();
            if registered.contains_key(&ty.id) {
                log::warn!(
                    "Horizon: data map {} registered twice: the first is kept",
                    ty.id
                );
                continue;
            }
            let (ns, path) = ty.id.split_once(':').unwrap_or_default();
            let wanted = format!("data_maps/{ns}:{}/{path}", folder(&ty.registry));
            let mut map = BTreeMap::new();
            if let Some((_, versions)) = overlay.files.iter().find(|(k, _)| *k == wanted) {
                for bytes in versions {
                    let file = serde_json::from_slice::<Value>(bytes).ok();
                    if file
                        .as_ref()
                        .is_some_and(|file| !overlay.conditions.allows_file(file, &wanted))
                    {
                        continue;
                    }
                    let applied = file.is_some_and(|file| {
                        apply_file(
                            &mut map,
                            &file,
                            &ty.registry,
                            registries,
                            overlay.conditions,
                            &wanted,
                        )
                    });
                    if !applied {
                        log::warn!(
                            "Horizon: a data map file of {} is not a data map, skipped",
                            ty.id
                        );
                    }
                }
            }
            registered.insert(ty.id.clone(), MapValues::Json(map));
        }
        Self {
            maps,
            ..Self::default()
        }
    }

    pub(crate) fn with_builtins(mut self, builtin: super::neoforge_mob_maps::Loaded) -> Self {
        for (definition, values) in builtin.values {
            if self
                .maps
                .entry(definition.registry.to_owned())
                .or_default()
                .insert(definition.id.to_owned(), values)
                .is_some()
            {
                log::warn!(
                    "Horizon: builtin data map {} is reserved: engine definition kept",
                    definition.id
                );
            }
        }
        self.mob_maps = builtin.snapshot;
        self
    }

    /// Every entry map `type_id` of `registry` gives a value, with its merged
    /// JSON; none where the map wasn't loaded.
    pub(crate) fn json_values(
        &self,
        registry: &str,
        type_id: &str,
    ) -> impl Iterator<Item = (&str, &Value)> {
        self.maps
            .get(registry)
            .and_then(|maps| maps.get(type_id))
            .and_then(|values| match values {
                MapValues::Json(values) => Some(values),
                MapValues::Decoded(_) => None,
            })
            .into_iter()
            .flatten()
            .map(|(entry, value)| (entry.as_str(), value))
    }

    /// The registries with a loaded type, in id order: what `data_maps_updated` names.
    pub(crate) fn registries(&self) -> Vec<String> {
        self.maps.keys().cloned().collect()
    }

    /// Map `type_id` of `registry`, refused where no mod registered it there.
    fn map(&self, registry: &str, type_id: &str) -> Result<&MapValues, mod_api::DataMapError> {
        self.maps
            .get(registry)
            .and_then(|maps| maps.get(type_id))
            .ok_or(mod_api::DataMapError::UnknownType)
    }

    /// `entry`'s value in map `type_id` of `registry`, as NBT: none where the
    /// map gives it none, refused where no mod registered the map there.
    fn value(
        &self,
        registry: &str,
        type_id: &str,
        entry: &str,
    ) -> Result<Option<Tag>, mod_api::DataMapError> {
        match self.map(registry, type_id)? {
            MapValues::Json(values) => values.get(entry).map(to_tag).transpose(),
            MapValues::Decoded(values) => Ok(values.get(entry).cloned()),
        }
    }

    /// Every entry map `type_id` of `registry` gives a value, in id order.
    fn entries(
        &self,
        registry: &str,
        type_id: &str,
    ) -> Result<Vec<(String, Tag)>, mod_api::DataMapError> {
        match self.map(registry, type_id)? {
            MapValues::Json(values) => values
                .iter()
                .map(|(entry, value)| Ok((entry.clone(), to_tag(value)?)))
                .collect(),
            MapValues::Decoded(values) => Ok(values
                .iter()
                .map(|(entry, value)| (entry.clone(), value.clone()))
                .collect()),
        }
    }
}

impl Game<'_> {
    /// Tell the mods the data maps of every registry with a type were merged
    /// (`DataMapsUpdatedEvent`), once the server's tables hold them.
    pub(in crate::game) fn announce_data_maps_updated(
        &mut self,
        cause: mod_api::DataMapsUpdateCause,
    ) {
        for registry in self.data_map_registries.clone() {
            self.native_data_maps_updated(&registry, cause);
        }
    }
}

impl Game<'_> {
    /// The server's merged data maps, none unless this is a server whose
    /// tables were built from packs and mods' files.
    fn loaded_data_maps(&self) -> Result<&DataMaps, mod_api::DataMapError> {
        if self.serving.is_none() && self.options.dedicated.is_none() {
            return Err(mod_api::DataMapError::Unavailable);
        }
        self.world_tables
            .as_ref()
            .map(|tables| &tables.data_maps)
            .ok_or(mod_api::DataMapError::Unavailable)
    }
    /// Entry `entry` of `registry`: its value in data map `map`, before the map decodes it:
    /// the merged JSON the server's data packs and mods' files gave it, as NBT. `Ok(None)`
    /// where it has none.
    pub(in crate::game) fn data_map_value(
        &self,
        registry: &str,
        entry: &str,
        map: &str,
    ) -> Result<Option<Tag>, mod_api::DataMapError> {
        self.loaded_data_maps()?
            .value(registry, map, &full_id(entry))
    }
    /// Every entry data map `map` of `registry` gives a value, with that value, in id order.
    pub(in crate::game) fn data_map_entries(
        &self,
        registry: &str,
        map: &str,
    ) -> Result<Vec<(String, Tag)>, mod_api::DataMapError> {
        self.loaded_data_maps()?.entries(registry, map)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn data_map_read_refuses_lossy_payloads_instead_of_successful_substitutes() {
        for value in [
            json!({"nested": [null]}),
            json!(9223372036854775809_u64),
            json!(u64::MAX),
        ] {
            let maps = DataMaps {
                maps: BTreeMap::from([(
                    "minecraft:item".to_owned(),
                    HashMap::from([(
                        "test:map".to_owned(),
                        MapValues::Json(BTreeMap::from([("minecraft:stone".to_owned(), value)])),
                    )]),
                )]),
                ..DataMaps::default()
            };
            assert_eq!(
                maps.value("minecraft:item", "test:map", "minecraft:stone"),
                Err(mod_api::DataMapError::Undecodable)
            );
            assert_eq!(
                maps.value("minecraft:item", "test:map", "minecraft:air"),
                Ok(None)
            );
        }
    }

    #[test]
    fn builtin_store_is_registered_without_mod_types() {
        let builtin = super::super::neoforge_mob_maps::load(
            &horizon_world::loot::PackOverlay::NONE,
            |_| None,
            |_, _| true,
        )
        .unwrap();
        let maps = DataMaps::default().with_builtins(builtin);
        assert_eq!(maps.registries().len(), 5);
        assert_eq!(
            maps.value(
                "minecraft:item",
                "neoforge:villager_compostables",
                "minecraft:wheat_seeds"
            ),
            Ok(Some(Tag::compound([(
                "can_villager_compost",
                Tag::Byte(1)
            )])))
        );
    }

    /// Registries for tests: items, blocks and entity types; entity type ids
    /// listed, item and block ids not, and the tags given by registry.
    #[derive(Default)]
    struct Fixture {
        tags: HashMap<(&'static str, &'static str), Vec<&'static str>>,
    }
    impl Fixture {
        fn with(tags: &[(&'static str, &'static str, &[&'static str])]) -> Self {
            Self {
                tags: tags
                    .iter()
                    .map(|(registry, tag, members)| ((*registry, *tag), members.to_vec()))
                    .collect(),
            }
        }
    }
    impl MapRegistries for Fixture {
        fn known(&self, registry: &str) -> bool {
            ["minecraft:item", "minecraft:block", "minecraft:entity_type"].contains(&registry)
        }
        fn contains(&self, registry: &str, entry: &str) -> bool {
            registry != "minecraft:entity_type"
                || [
                    "minecraft:pig",
                    "minecraft:stray",
                    "minecraft:skeleton",
                    "minecraft:cow",
                ]
                .contains(&entry)
        }
        fn tag(&self, registry: &str, tag: &str) -> Vec<String> {
            self.tags
                .iter()
                .filter(|((r, t), _)| *r == registry && *t == tag)
                .flat_map(|(_, members)| members.iter().map(|m| (*m).to_owned()))
                .collect()
        }
    }

    fn ty(id: &str, registry: &str) -> DataMapType {
        DataMapType {
            id: id.to_owned(),
            registry: registry.to_owned(),
        }
    }

    fn files(list: &[(&str, &[Value])]) -> Vec<(String, Vec<Vec<u8>>)> {
        list.iter()
            .map(|(key, versions)| {
                (
                    (*key).to_owned(),
                    versions
                        .iter()
                        .map(|v| v.to_string().into_bytes())
                        .collect(),
                )
            })
            .collect()
    }

    fn load(files: &[(String, Vec<Vec<u8>>)]) -> DataMaps {
        let registries = Fixture::with(&[(
            "minecraft:item",
            "minecraft:logs",
            &["minecraft:oak_log", "minecraft:birch_log"],
        )]);
        DataMaps::load(
            &[ty("mymod:fuel", "minecraft:item")],
            &horizon_world::loot::PackOverlay {
                files,
                ..horizon_world::loot::PackOverlay::NONE
            },
            &registries,
        )
    }

    fn get<'a>(maps: &'a DataMaps, entry: &str) -> Option<&'a Value> {
        let MapValues::Json(values) = &maps.maps["minecraft:item"]["mymod:fuel"] else {
            panic!("JSON map");
        };
        values.get(entry)
    }

    #[test]
    fn a_value_reads_as_nbt_and_a_map_no_mod_registered_is_refused() {
        let f = files(&[(
            "data_maps/mymod:item/fuel",
            &[json!({"values": {
                "minecraft:coal": 1600,
                "minecraft:stick": {"burn": 3_000_000_000_i64}
            }})],
        )]);
        let maps = load(&f);
        let value =
            |registry: &str, type_id: &str, entry: &str| maps.value(registry, type_id, entry);
        assert_eq!(
            value("minecraft:item", "mymod:fuel", "minecraft:coal"),
            Ok(Some(Tag::Int(1600)))
        );
        assert_eq!(
            value("minecraft:item", "mymod:fuel", "minecraft:stick"),
            Ok(Some(Tag::Compound(
                [("burn".into(), Tag::Long(3_000_000_000))]
                    .into_iter()
                    .collect()
            )))
        );
        assert_eq!(
            value("minecraft:item", "mymod:fuel", "minecraft:stone"),
            Ok(None)
        );
        assert_eq!(
            value("minecraft:block", "mymod:fuel", "minecraft:coal"),
            Err(mod_api::DataMapError::UnknownType)
        );
        assert_eq!(
            value("minecraft:item", "mymod:other", "minecraft:coal"),
            Err(mod_api::DataMapError::UnknownType)
        );
    }

    #[test]
    fn registries_with_a_type_are_listed_once_in_id_order() {
        let maps = DataMaps::load(
            &[
                ty("mymod:b", "minecraft:block"),
                ty("mymod:a", "minecraft:item"),
                ty("mymod:c", "minecraft:block"),
                ty("mymod:d", "test:nowhere"),
            ],
            &horizon_world::loot::PackOverlay::NONE,
            &Fixture::default(),
        );
        assert_eq!(maps.registries(), ["minecraft:block", "minecraft:item"]);
    }

    #[test]
    fn a_map_on_any_registry_resolves_ids_tags_and_lists_whole() {
        // `minecraft:ghost` is not in the fixture's entity types: a tag naming
        // an id the registry doesn't hold publishes nothing for it.
        let registries = Fixture::with(&[(
            "minecraft:entity_type",
            "minecraft:skeletons",
            &["minecraft:skeleton", "minecraft:stray", "minecraft:ghost"],
        )]);
        let f = files(&[(
            "data_maps/test:entity_type/weight",
            &[json!({"values": {
                "minecraft:pig": 3,
                "#minecraft:skeletons": 5,
                "minecraft:missing": 9
            }})],
        )]);
        let maps = DataMaps::load(
            &[ty("test:weight", "minecraft:entity_type")],
            &horizon_world::loot::PackOverlay {
                files: &f,
                ..horizon_world::loot::PackOverlay::NONE
            },
            &registries,
        );
        let value = |entry| maps.value("minecraft:entity_type", "test:weight", entry);
        assert_eq!(value("minecraft:pig"), Ok(Some(Tag::Int(3))));
        assert_eq!(value("minecraft:stray"), Ok(Some(Tag::Int(5))));
        assert_eq!(value("minecraft:cow"), Ok(None));
        // An id the registry doesn't hold is skipped.
        assert_eq!(value("minecraft:missing"), Ok(None));
        // A tag member the registry doesn't hold is skipped with the plain ids.
        assert_eq!(value("minecraft:ghost"), Ok(None));
        assert_eq!(
            maps.entries("minecraft:entity_type", "test:weight"),
            Ok(vec![
                ("minecraft:pig".to_owned(), Tag::Int(3)),
                ("minecraft:skeleton".to_owned(), Tag::Int(5)),
                ("minecraft:stray".to_owned(), Tag::Int(5)),
            ])
        );
        assert_eq!(maps.registries(), ["minecraft:entity_type"]);
    }

    /// A mod-declared datapack registry the command ids do not list is checked
    /// against the world's loaded entries; a registry with no source of ids
    /// still accepts an entry, as before.
    #[test]
    fn a_declared_datapack_registry_rejects_unknown_ids() {
        // The declaration the process reads, as a registering mod's would be.
        mod_api::__private::install_process_datapack_registry_declarations(vec![
            mod_api::DatapackRegistryDeclaration {
                mod_id: "example".into(),
                key: "example:spell".into(),
                fields: vec![mod_api::DatapackField::required(
                    "power",
                    mod_api::DatapackFieldKind::Int { min: 0, max: 100 },
                )],
                sync: mod_api::DatapackRegistrySync::Required,
            },
        ]);
        let set = mod_api::DatapackRegistrySet::with_entries_for_testing(
            "example:spell",
            &["example:fire"],
        );
        let items = horizon_client::items::Items::empty();
        let registries = WorldRegistries::new(
            [],
            Some(&set),
            &items,
            std::path::Path::new("unused"),
            &horizon_world::loot::PackOverlay::NONE,
        );
        assert!(
            !registries.contains("example:spell", "example:ice"),
            "an id the world never loaded is refused"
        );
        assert!(registries.contains("example:spell", "example:fire"));
        // No source of ids, no refusal: a typed registry the sync did not
        // list (the ids branch above covers the listed ones).
        assert!(registries.contains("minecraft:some_typed", "whatever"));
    }

    #[test]
    fn a_value_counts_only_when_its_own_conditions_hold() {
        let f = files(&[(
            "data_maps/mymod:item/fuel",
            &[json!({"values": {
                "minecraft:coal": 1,
                "minecraft:stick": {"neoforge:conditions": [
                    {"type": "neoforge:mod_loaded", "modid": "absent"}], "value": 2},
                "minecraft:planks": {"neoforge:conditions": [
                    {"type": "neoforge:true"}], "value": 3}
            }})],
        )]);
        let maps = load(&f);
        assert_eq!(get(&maps, "minecraft:coal"), Some(&json!(1)));
        assert_eq!(get(&maps, "minecraft:stick"), None);
        assert_eq!(get(&maps, "minecraft:planks"), Some(&json!(3)));
    }

    #[test]
    fn values_attach_to_entries_and_to_every_member_of_a_tag() {
        let f = files(&[(
            "data_maps/mymod:item/fuel",
            &[json!({"values": {
                "minecraft:coal": {"burn": 1600},
                "stick": 100,
                "#minecraft:logs": {"burn": 300}
            }})],
        )]);
        let maps = load(&f);
        assert_eq!(get(&maps, "minecraft:coal"), Some(&json!({"burn": 1600})));
        // An id without a namespace is a `minecraft` one.
        assert_eq!(get(&maps, "minecraft:stick"), Some(&json!(100)));
        assert_eq!(get(&maps, "minecraft:oak_log"), Some(&json!({"burn": 300})));
        assert_eq!(
            get(&maps, "minecraft:birch_log"),
            Some(&json!({"burn": 300}))
        );
        assert_eq!(get(&maps, "minecraft:stone"), None);
    }

    #[test]
    fn a_newer_file_overrides_and_replace_clears_what_came_before() {
        let f = files(&[(
            "data_maps/mymod:item/fuel",
            &[
                json!({"values": {"minecraft:coal": 1, "minecraft:stick": 2}}),
                json!({"values": {"minecraft:coal": 10}}),
            ],
        )]);
        let maps = load(&f);
        assert_eq!(get(&maps, "minecraft:coal"), Some(&json!(10)));
        assert_eq!(get(&maps, "minecraft:stick"), Some(&json!(2)));
        let f = files(&[(
            "data_maps/mymod:item/fuel",
            &[
                json!({"values": {"minecraft:coal": 1, "minecraft:stick": 2}}),
                json!({"replace": true, "values": {"minecraft:coal": 10}}),
            ],
        )]);
        let maps = load(&f);
        assert_eq!(get(&maps, "minecraft:coal"), Some(&json!(10)));
        assert_eq!(get(&maps, "minecraft:stick"), None);
    }

    #[test]
    fn remove_takes_entries_and_tags_away_after_the_files_own_values() {
        let f = files(&[(
            "data_maps/mymod:item/fuel",
            &[
                json!({"values": {"#minecraft:logs": 5, "minecraft:coal": 1}}),
                // The file's own value for coal is attached, then removed.
                json!({"values": {"minecraft:coal": 9}, "remove": ["minecraft:coal", "#minecraft:logs"]}),
            ],
        )]);
        let maps = load(&f);
        assert_eq!(get(&maps, "minecraft:coal"), None);
        assert_eq!(get(&maps, "minecraft:oak_log"), None);
    }

    #[test]
    fn the_wrapper_form_gives_its_value_and_a_plain_object_is_a_value() {
        let f = files(&[(
            "data_maps/mymod:item/fuel",
            &[json!({"values": {
                "minecraft:coal": {"value": {"burn": 3}, "replace": true},
                "minecraft:stick": {"value": 1, "other": 2}
            }})],
        )]);
        let maps = load(&f);
        assert_eq!(get(&maps, "minecraft:coal"), Some(&json!({"burn": 3})));
        // More than `value` and `replace`: the object is the data.
        assert_eq!(
            get(&maps, "minecraft:stick"),
            Some(&json!({"value": 1, "other": 2}))
        );
    }

    #[test]
    fn a_bad_file_is_skipped_alone_and_bad_types_are_left_out() {
        let mut f = files(&[(
            "data_maps/mymod:item/fuel",
            &[json!({"values": {"minecraft:coal": 1}})],
        )]);
        f[0].1.insert(0, b"{nope".to_vec());
        f[0].1
            .push(json!({"nothing": true}).to_string().into_bytes());
        let maps = load(&f);
        assert_eq!(get(&maps, "minecraft:coal"), Some(&json!(1)));
        let maps = DataMaps::load(
            &[
                ty("minecraft:fuel", "minecraft:item"),
                ty("mymod:x", "test:nowhere"),
                ty("nonamespace", "minecraft:item"),
                ty("mymod:a", "minecraft:block"),
                ty("mymod:a", "minecraft:block"),
            ],
            &horizon_world::loot::PackOverlay::NONE,
            &Fixture::default(),
        );
        // Only the one good type, registered once, empty.
        assert_eq!(maps.maps.len(), 1);
        let blocks = &maps.maps["minecraft:block"];
        assert_eq!(blocks.len(), 1);
        assert!(matches!(&blocks["mymod:a"], MapValues::Json(values) if values.is_empty()));
    }

    #[test]
    fn conditional_layers_are_skipped_before_replace_and_remove() {
        let f = files(&[(
            "data_maps/mymod:item/fuel",
            &[
                json!({"values":{"minecraft:coal":1}}),
                json!({"neoforge:conditions":[{"type":"unsupported"}],"replace":true}),
                json!({"neoforge:conditions":[{"type":"neoforge:mod_loaded","modid":"provider"}],"remove":["minecraft:coal"],"values":{"minecraft:stick":2}}),
            ],
        )]);
        let off = load(&f);
        assert_eq!(get(&off, "minecraft:coal"), Some(&json!(1)));
        assert!(get(&off, "minecraft:stick").is_none());
        let enabled = horizon_world::conditions::LoadConditions::new(["provider"]);
        let on = DataMaps::load(
            &[ty("mymod:fuel", "minecraft:item")],
            &horizon_world::loot::PackOverlay {
                files: &f,
                conditions: &enabled,
            },
            &Fixture::default(),
        );
        assert!(get(&on, "minecraft:coal").is_none());
        assert_eq!(get(&on, "minecraft:stick"), Some(&json!(2)));
    }

    #[test]
    fn block_maps_read_the_block_tags() {
        let registries =
            Fixture::with(&[("minecraft:block", "minecraft:logs", &["minecraft:oak_log"])]);
        let f = files(&[(
            "data_maps/mymod:block/hardness",
            &[json!({"values": {"#minecraft:logs": 2}})],
        )]);
        let maps = DataMaps::load(
            &[ty("mymod:hardness", "minecraft:block")],
            &horizon_world::loot::PackOverlay {
                files: &f,
                ..horizon_world::loot::PackOverlay::NONE
            },
            &registries,
        );
        let MapValues::Json(map) = &maps.maps["minecraft:block"]["mymod:hardness"] else {
            panic!("JSON map");
        };
        assert_eq!(map.get("minecraft:oak_log"), Some(&json!(2)));
    }
}
