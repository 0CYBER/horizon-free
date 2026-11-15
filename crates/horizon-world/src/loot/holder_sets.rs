//! Bind only engine-owned holder-set fields, before publishing a loot snapshot.
use super::*;

#[derive(Default)]
pub(super) struct Binding {
    universes: HashMap<&'static str, Vec<String>>,
}
impl Binding {
    pub(super) fn load(
        root: &Path,
        overlay: &PackOverlay,
        items: &HashMap<String, ItemFacts>,
        enchantments: &[Enchantment],
    ) -> io::Result<Self> {
        let mut universes = HashMap::from([
            ("item", items.keys().cloned().collect::<Vec<_>>()),
            (
                "enchantment",
                enchantments.iter().map(|e| e.name.clone()).collect(),
            ),
        ]);
        for registry in ["worldgen/biome", "instrument"] {
            let mut entries = HashMap::new();
            load_kind(&root.join("data"), registry, &mut entries)?;
            overlay_map(overlay, registry, &mut entries);
            if registry == "worldgen/biome" {
                match fs::read(root.join("worldgen/biomes.json")) {
                    Ok(bytes) => {
                        let bundle: HashMap<String, Value> = serde_json::from_slice(&bytes)?;
                        for (id, value) in bundle {
                            entries.entry(id).or_insert(value);
                        }
                    }
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error),
                }
            }
            if !entries.is_empty() {
                universes.insert(registry, entries.into_keys().collect());
            }
        }
        for (registry, ids) in &mut universes {
            if matches!(*registry, "worldgen/biome" | "instrument") {
                ids.sort_by_cached_key(|id| {
                    let (ns, path) = id.split_once(':').unwrap_or(("minecraft", id));
                    (path.to_owned(), ns.to_owned())
                });
            } else {
                ids.sort();
            }
            ids.dedup();
        }
        Ok(Self { universes })
    }
    fn field(
        &self,
        owner: &str,
        registry: &str,
        value: &mut Value,
        tags: &HashMap<(String, String), crate::tag_layers::TagLayers>,
    ) -> io::Result<bool> {
        if !value.is_object() {
            return Ok(false);
        }
        let universe = || self.universes[registry].clone();
        let tag = |tag: &str| Ok(loot_tags::members(tags, registry, tag));
        let entry = |id: &str| Ok(vec![id.to_owned()]);
        let lookup = crate::holder_set::StringLookup {
            what: registry,
            ids: self
                .universes
                .contains_key(registry)
                .then_some(&universe as &dyn Fn() -> Vec<String>),
            tag: &tag,
            entry: &entry,
        };
        let ids = crate::holder_set::resolve(value, &lookup).map_err(|error| {
            io::Error::new(io::ErrorKind::InvalidData, format!("{owner}: {error}"))
        })?;
        *value = Value::Array(ids.into_iter().map(Value::String).collect());
        Ok(true)
    }
    fn item(
        &self,
        owner: &str,
        value: &mut Value,
        tags: &HashMap<(String, String), crate::tag_layers::TagLayers>,
    ) -> io::Result<bool> {
        let mut changed = false;
        if let Some(items) = value.get_mut("items") {
            changed |= self.field(&format!("{owner}.items"), "item", items, tags)?;
        }
        if let Some(entries) = value
            .pointer_mut("/predicates/minecraft:enchantments")
            .and_then(Value::as_array_mut)
        {
            for (i, entry) in entries.iter_mut().enumerate() {
                if let Some(set) = entry.get_mut("enchantments") {
                    changed |= self.field(
                        &format!("{owner}.predicates.minecraft:enchantments[{i}].enchantments"),
                        "enchantment",
                        set,
                        tags,
                    )?;
                }
            }
        }
        Ok(changed)
    }
    fn entity(
        &self,
        owner: &str,
        value: &mut Value,
        tags: &HashMap<(String, String), crate::tag_layers::TagLayers>,
    ) -> io::Result<bool> {
        let mut changed = false;
        if let Some(equipment) = value
            .get_mut("minecraft:equipment")
            .and_then(Value::as_object_mut)
        {
            for (slot, item) in equipment {
                changed |= self.item(&format!("{owner}.minecraft:equipment.{slot}"), item, tags)?;
            }
        }
        if let Some(vehicle) = value.get_mut("minecraft:vehicle") {
            changed |= self.entity(&format!("{owner}.minecraft:vehicle"), vehicle, tags)?;
        }
        Ok(changed)
    }
    /// Visit engine structural fields only, never custom-data/component/mod payloads.
    pub(super) fn walk(
        &self,
        owner: &str,
        value: &mut Value,
        tags: &HashMap<(String, String), crate::tag_layers::TagLayers>,
    ) -> io::Result<bool> {
        if let Value::Array(values) = value {
            let mut changed = false;
            for (i, value) in values.iter_mut().enumerate() {
                changed |= self.walk(&format!("{owner}[{i}]"), value, tags)?;
            }
            return Ok(changed);
        }
        let mut changed = false;
        let kind = value["type"].as_str().unwrap_or("").to_owned();
        match kind.as_str() {
            "minecraft:tag" => {
                if let Some(set) = value.get_mut("items") {
                    changed |= self.field(&format!("{owner}.items"), "item", set, tags)?;
                }
            }
            "minecraft:enchant_randomly"
            | "minecraft:enchant_with_levels"
            | "minecraft:set_instrument" => {
                let registry = if kind == "minecraft:set_instrument" {
                    "instrument"
                } else {
                    "enchantment"
                };
                if let Some(set) = value.get_mut("options") {
                    changed |= self.field(&format!("{owner}.options"), registry, set, tags)?;
                }
            }
            "minecraft:location_check" => {
                if let Some(set) = value.pointer_mut("/predicate/biomes") {
                    changed |= self.field(
                        &format!("{owner}.predicate.biomes"),
                        "worldgen/biome",
                        set,
                        tags,
                    )?;
                }
            }
            "minecraft:filtered" => {
                if let Some(item) = value.get_mut("item_filter") {
                    changed |= self.item(&format!("{owner}.item_filter"), item, tags)?;
                }
            }
            "minecraft:entity_properties" => {
                if let Some(entity) = value.get_mut("predicate") {
                    changed |= self.entity(&format!("{owner}.predicate"), entity, tags)?;
                }
            }
            "minecraft:damage_source_properties" => {
                for field in ["direct_entity", "source_entity"] {
                    if let Some(entity) = value.get_mut("predicate").and_then(|v| v.get_mut(field))
                    {
                        changed |=
                            self.entity(&format!("{owner}.predicate.{field}"), entity, tags)?;
                    }
                }
            }
            _ => {}
        }
        // A mod kind owns its payload, but its enclosing condition remains engine-owned.
        if kind.contains(':') && !kind.starts_with("minecraft:") {
            if let Some(condition) = value.get_mut("condition") {
                changed |= self.walk(&format!("{owner}.condition"), condition, tags)?;
            }
            return Ok(changed);
        }
        for field in [
            "pools",
            "entries",
            "children",
            "condition",
            "conditions",
            "terms",
            "term",
            "modifier",
            "functions",
            "on_pass",
            "on_fail",
            "requirements",
            "effect",
            "effects",
        ] {
            if let Some(child) = value.get_mut(field) {
                if field == "effects" && child.is_object() {
                    for (name, effect) in child.as_object_mut().unwrap() {
                        changed |= self.walk(&format!("{owner}.effects.{name}"), effect, tags)?;
                    }
                } else {
                    changed |= self.walk(&format!("{owner}.{field}"), child, tags)?;
                }
            }
        }
        Ok(changed)
    }
    pub(super) fn snapshot(&self, data: &mut LootData) -> io::Result<()> {
        let mut tables = None;
        for (id, lazy) in &data.tables.0 {
            let mut value = lazy.read().into_owned();
            if self.walk(&format!("loot_table {id}"), &mut value, &data.tags)? {
                tables
                    .get_or_insert_with(|| data.tables.0.clone())
                    .insert(id.clone(), LazyJson::new(&value));
            }
        }
        if let Some(tables) = tables {
            data.tables = Arc::new(LootTables(tables));
        }
        for (kind, entries) in [
            ("item_modifier", &mut data.modifiers),
            ("predicate", &mut data.predicates),
        ] {
            for (id, value) in entries {
                self.walk(&format!("{kind} {id}"), value, &data.tags)?;
            }
        }
        for enchantment in &mut data.enchantments {
            for (field, registry, value) in [
                ("supported_items", "item", &mut enchantment.supported),
                ("exclusive_set", "enchantment", &mut enchantment.exclusive),
            ] {
                self.field(
                    &format!("enchantment {}.{field}", enchantment.name),
                    registry,
                    value,
                    &data.tags,
                )?;
            }
            if let Some(primary) = &mut enchantment.primary {
                self.field(
                    &format!("enchantment {}.primary_items", enchantment.name),
                    "item",
                    primary,
                    &data.tags,
                )?;
            }
            for (kind, effects) in enchantment.effects.as_object_mut().into_iter().flatten() {
                self.walk(
                    &format!("enchantment {}.effects.{kind}", enchantment.name),
                    effects,
                    &data.tags,
                )?;
            }
        }
        for (i, (ingredient, _, _)) in data.smelting.iter_mut().enumerate() {
            self.field(
                &format!("smelting[{i}].ingredient"),
                "item",
                ingredient,
                &data.tags,
            )?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn loaded_biome_and_instrument_universes_bind_before_the_first_roll() {
        let root = std::env::temp_dir().join(format!(
            "loot-holder-bind-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        for (path, value) in [
            ("items.json", json!({"minecraft:goat_horn": {}})),
            ("data/minecraft/worldgen/biome/plains.json", json!({})),
            ("data/minecraft/worldgen/biome/desert.json", json!({})),
            ("data/minecraft/instrument/ponder.json", json!({})),
            ("data/minecraft/instrument/sing.json", json!({})),
            (
                "data/test/loot_table/horn.json",
                json!({"pools": [{"rolls": 1,
                    "condition": {"type": "minecraft:location_check", "predicate": {"biomes": {"type": "neoforge:not", "value": "minecraft:plains"}}},
                    "entries": [{"type": "minecraft:item", "name": "minecraft:goat_horn", "modifier": {"type": "minecraft:set_instrument", "options": {"type": "neoforge:not", "value": "minecraft:ponder"}}}]
                }]}),
            ),
        ] {
            let path = root.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, value.to_string()).unwrap();
        }
        let defaults = crate::item_defaults::read(&root).unwrap();
        let data = LootData::load(&root, &PackOverlay::NONE, &defaults).unwrap();
        let published = data.tables.get("test:horn").unwrap();
        assert_eq!(
            published["pools"][0]["condition"]["predicate"]["biomes"],
            json!(["minecraft:desert"])
        );
        assert_eq!(
            published["pools"][0]["entries"][0]["modifier"]["options"],
            json!(["minecraft:sing"])
        );
        assert!(
            data.roll(
                "test:horn",
                &mut LegacyRandom::new(1),
                0.0,
                Some("minecraft:plains"),
                None
            )
            .is_empty()
        );
        let drops = data.roll(
            "test:horn",
            &mut LegacyRandom::new(1),
            0.0,
            Some("minecraft:desert"),
            None,
        );
        assert_eq!(drops.len(), 1);
        assert_eq!(
            drops[0].components,
            vec![(
                "minecraft:instrument".into(),
                Some(Tag::String("minecraft:sing".into()))
            )]
        );
        let mut any = json!({"type": "neoforge:any"});
        data.holder_sets
            .field("test:any.biomes", "worldgen/biome", &mut any, &data.tags)
            .unwrap();
        assert_eq!(any, json!(["minecraft:desert", "minecraft:plains"]));
        let mut any = json!({"type": "neoforge:any"});
        data.holder_sets
            .field("test:any.options", "instrument", &mut any, &data.tags)
            .unwrap();
        assert_eq!(any, json!(["minecraft:ponder", "minecraft:sing"]));
        fs::write(root.join("data/test/loot_table/bad.json"), json!({"pools": [{"entries": [{"type": "minecraft:tag", "items": {"type": "neoforge:or", "values": 42}}]}]}).to_string()).unwrap();
        let error = LootData::load(&root, &PackOverlay::NONE, &defaults)
            .err()
            .unwrap()
            .to_string();
        assert!(
            error.contains("loot_table test:bad")
                && error.contains("items")
                && error.contains("no values"),
            "{error}"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unavailable_universes_refuse_and_opaque_mod_payloads_remain_untouched() {
        let binding = Binding::default();
        let tags = HashMap::new();
        let mut location = json!({"type": "minecraft:location_check", "predicate": {"biomes": {"type": "neoforge:any"}}});
        let error = binding
            .walk("predicate test:where", &mut location, &tags)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("test:where.predicate.biomes")
                && error.contains("worldgen/biome universe is unavailable"),
            "{error}"
        );
        let mut custom = json!({"type": "test:opaque", "items": {"type": "neoforge:unknown"}, "effects": {"type": "minecraft:tag", "items": {"type": "neoforge:unknown"}}});
        let expected = custom.clone();
        assert!(
            !binding
                .walk("loot_table test:opaque", &mut custom, &tags)
                .unwrap()
        );
        assert_eq!(custom, expected);
    }
}
