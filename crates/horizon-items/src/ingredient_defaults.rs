//! Tag-only item table construction.
use super::*;
impl Items {
    /// Only the block and item tags (and the enchantment registry, which the
    /// station rules and loot functions are built from), for tools that need
    /// tag membership.
    pub fn tags_only(root: &Path, overlay: &PackOverlay, defaults: &ItemDefaults) -> Result<Self> {
        let enchantments = Arc::new(crate::enchant::Enchantments::load_with(
            root, overlay, defaults,
        )?);
        let mut items = Self {
            items: HashMap::new(),
            block_items: Default::default(),
            component_registry: component_registry(
                root,
                &enchantments,
                &load_tags(root, "entity_type", overlay)?,
            )?,
            component_defaults: component_defaults(defaults)?,
            mob_visibility: HashMap::new(),
            order: Vec::new(),
            registry_order: Vec::new(),
            block_tags: load_tags(root, "block", overlay)?,
            item_tags: load_tags(root, "item", overlay)?,
            fluid_tags: load_tags(root, "fluid", overlay)?,
            item_tag_order: load_tag_lists(root, "item", overlay)?,
            block_tag_order: load_tag_lists(root, "block", overlay)?,
            fluid_tag_order: load_tag_lists(root, "fluid", overlay)?,
            loot: Default::default(),
            predicates: Default::default(),
            songs: HashMap::new(),
            instruments: HashMap::new(),
            enchantments,
        };
        load_component_damage_registry(root, overlay, &mut items)?;
        Ok(items)
    }

    /// Bind ingredient codecs to the world's frozen damage registry and reloaded tags.
    /// Do this before preparing recipes, alongside binding item damage predicates.
    pub fn bind_component_damage_catalog(
        &mut self,
        catalog: &horizon_world::damage_source::Catalog,
    ) {
        let mut ids: Vec<_> = catalog
            .entries()
            .map(|(_, entry)| entry.name.clone())
            .collect();
        ids.sort();
        let registry = Arc::make_mut(&mut self.component_registry);
        registry.by_registry.insert(
            "minecraft:damage_type".into(),
            horizon_client_core::registries::RegistryIds {
                ids,
                tags: catalog.tags().map(str::to_owned).collect(),
            },
        );
    }
}

/// Standalone catalogs have no world-owned snapshot; load their complete pack registry.
pub(super) fn load_component_damage_registry(
    root: &Path,
    overlay: &PackOverlay,
    items: &mut Items,
) -> Result<()> {
    if root.join("damage-catalog.json").exists() {
        let catalog = horizon_world::damage_source::Catalog::load(root, overlay)?;
        items.bind_component_damage_catalog(&catalog);
    }
    Ok(())
}

/// Preserve the exact loaded prototype, including native default-component changes.
pub(super) fn component_defaults(
    defaults: &ItemDefaults,
) -> Result<Arc<HashMap<String, horizon_core::nbt::Compound>>> {
    defaults
        .entries()
        .iter()
        .map(|(id, value)| {
            let object = value
                .as_object()
                .ok_or_else(|| format!("{id}: component prototype is not an object"))?;
            Ok((
                id.clone(),
                horizon_core::json_nbt::json_to_compound(object)?,
            ))
        })
        .collect::<Result<_>>()
        .map(Arc::new)
}

/// Catalogs also need Block.getItem for block-tag ingredients, without full block facts.
pub(super) fn block_items(root: &Path) -> Result<HashMap<String, String>> {
    #[derive(serde::Deserialize)]
    struct Registry {
        defaults: HashMap<String, usize>,
    }
    #[derive(serde::Deserialize)]
    struct Gameplay {
        states: Vec<State>,
    }
    #[derive(serde::Deserialize)]
    struct State {
        item: Option<String>,
    }
    let open = |name: &str| match fs::File::open(root.join(name)) {
        Ok(file) => Ok(Some(file)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    };
    let (Some(registry), Some(gameplay)) = (open("facts.json")?, open("gameplay-facts.json")?)
    else {
        return Ok(HashMap::new());
    };
    let registry: Registry = serde_json::from_reader(std::io::BufReader::new(registry))?;
    let gameplay: Gameplay = serde_json::from_reader(std::io::BufReader::new(gameplay))?;
    registry
        .defaults
        .into_iter()
        .map(|(block, state)| {
            let item = gameplay
                .states
                .get(state)
                .ok_or("block default state is missing gameplay facts")?
                .item
                .as_deref()
                .unwrap_or("minecraft:air");
            Ok((block, item.to_owned()))
        })
        .collect()
}

/// Registry metadata used by ingredient component codecs at the load boundary.
pub(super) fn component_registry(
    root: &Path,
    enchantments: &crate::enchant::Enchantments,
    entity_tags: &HashMap<String, HashSet<String>>,
) -> Result<Arc<horizon_client_core::registries::Registries>> {
    let mut registry = horizon_client_core::registries::Registries::default();
    let path = root.join("command-registries.json");
    if path.exists() {
        registry.fill_from_export(&serde_json::from_slice(&fs::read(path)?)?);
    }
    let path = root.join("registries.json");
    if path.exists() {
        let json: Value = serde_json::from_slice(&fs::read(path)?)?;
        if let Some(entries) = json["minecraft:data_component_type"]["entries"].as_object() {
            registry
                .by_registry
                .entry("minecraft:data_component_type".into())
                .or_default()
                .ids = entries.keys().cloned().collect();
        }
    }
    registry.enchantments = enchantments
        .list
        .iter()
        .map(|entry| entry.id.to_string())
        .collect();
    // The prepared entity table includes content-mod types, unlike a vanilla-only
    // command export. Re-read it for each catalog/load, including pack reload.
    let path = root.join("entity-types.json");
    let entity_ids = match fs::read(&path) {
        Ok(bytes) => {
            let rows: serde_json::Map<String, Value> = serde_json::from_slice(&bytes)?;
            Some(
                rows.keys()
                    .map(|name| {
                        horizon_core::identifier::Identifier::parse(name).map(|id| id.to_string())
                    })
                    .collect::<std::result::Result<Vec<_>, _>>()?,
            )
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };
    let entity_ids = entity_ids.unwrap_or_else(|| registry.ids("minecraft:entity_type").to_vec());
    let entities = registry
        .by_registry
        .entry("minecraft:entity_type".into())
        .or_default();
    entities.ids = entity_ids;
    entities.ids.sort();
    entities.ids.dedup();
    // These are the current merged pack tags, not the export's stale tag names.
    entities.tags = entity_tags.keys().cloned().collect();
    entities.tags.sort();
    // The command export is vanilla-only. World recipe loads bind their active
    // frozen catalog; standalone catalogs load the full pack/mod catalog above.
    registry.damage_types.clear();
    registry.by_registry.insert(
        "minecraft:damage_type".into(),
        horizon_client_core::registries::RegistryIds::default(),
    );
    Ok(Arc::new(registry))
}
