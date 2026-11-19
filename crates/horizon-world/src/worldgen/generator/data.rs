//! Seed-independent registry resources and their load boundary.
use super::*;
/// Feature resources for the FEATURES status, keyed by resource identifier.
pub(in crate::worldgen) struct FeatureResources {
    pub(in crate::worldgen) features: BTreeMap<String, String>,
    pub(in crate::worldgen) placed: BTreeMap<String, String>,
    pub(in crate::worldgen) feature_tags: BTreeMap<String, String>,
    pub(in crate::worldgen) fluid_tags: BTreeMap<String, Vec<String>>,
    pub(in crate::worldgen) state_providers: BTreeMap<String, String>,
    /// Block properties, support, collision and survival for placement checks.
    pub(in crate::worldgen) blocks: Option<super::super::generator_blocks::BlockBehavior>,
}

/// Raw structure, structure-set and template-pool resources.
pub(in crate::worldgen) struct StructureBundles {
    pub(in crate::worldgen) structures: BTreeMap<String, String>,
    pub(in crate::worldgen) sets: BTreeMap<String, String>,
    pub(in crate::worldgen) pools: BTreeMap<String, String>,
}

type RegistryNames = BTreeSet<String>;
type RegistryDecoder = fn(&GeneratorData) -> Option<RegistryNames>;
type RegistryCache = BTreeMap<&'static str, (OnceLock<Option<RegistryNames>>, RegistryDecoder)>;

/// Seed-independent generation resources, loaded once and shared by every
/// generator. Biome identities are indices into the sorted biome registry.
pub struct GeneratorData {
    pub(super) registry: &'static BlockRegistry,
    pub(super) states: GenBlockStates,
    pub(super) integers: BTreeSet<(String, String)>,
    pub(super) noises: BTreeMap<String, String>,
    pub(super) densities: BTreeMap<String, String>,
    pub(super) rules: BTreeMap<String, String>,
    pub(super) settings: BTreeMap<String, String>,
    pub(super) presets: BTreeMap<String, String>,
    pub(super) world_presets: BTreeMap<String, String>,
    /// Data packs' level stems (`dimension/<id>.json`), by dimension id:
    /// they replace the world preset's.
    pub(super) dimensions: BTreeMap<String, String>,
    /// Data packs' dimension types (`dimension_type/<id>.json`), by id:
    /// they replace the vanilla ones' build ranges.
    pub(super) dimension_types: BTreeMap<String, String>,
    /// What loading the data packs' entries left out, for the log.
    pub(super) warnings: Vec<String>,
    pub(super) conditions: MaterialConditionRegistry,
    pub(super) source_catalog: BiomeSourceCatalog,
    pub(super) biome_names: Vec<Arc<str>>,
    pub(super) biome_ids: BTreeMap<String, BiomeId>,
    pub(super) biome_json: Vec<String>,
    pub(super) temperatures: Vec<BiomeTemperatureSettings>,
    pub(super) biome_carvers: Vec<Vec<Arc<WorldCarver>>>,
    carvers: CarverCatalog,
    accepted_registries: OnceLock<RegistryCache>,
    decoded_structures: OnceLock<Option<super::super::structure::start::StructureRegistry>>,
    pub(super) carving_blocks: CarvingBlocks,
    pub(in crate::worldgen) block_tags: BTreeMap<String, Vec<String>>,
    /// `block_tags` bound to block states, once for the feature bindings of
    /// decoration, structures and live features (`feature_block_bindings`).
    pub(super) bound_block_tags: std::sync::OnceLock<super::super::block_feature::BoundBlockTags>,
    pub(in crate::worldgen) biome_tags: BTreeMap<String, Vec<String>>,
    pub(in crate::worldgen) feature_resources: Option<FeatureResources>,
    /// Templates, processor lists and per-state template transforms.
    pub(in crate::worldgen) structures: Option<Arc<super::super::structure::StructureResources>>,
    pub(in crate::worldgen) structure_resources: Option<StructureBundles>,
    /// Block shape-update behaviour (`block-behaviours.json` next to the
    /// worldgen directory) for template placement's neighbor updates.
    pub(in crate::worldgen) behaviours: Option<Arc<crate::level::Behaviours>>,
    /// Concentric ring positions by seed and dimension generator (see
    /// `rings_for`), most recent last.
    pub(super) ring_cache: Mutex<Vec<RingCacheEntry>>,
    /// Placed-feature decoration by biome source (see `decoration_for`),
    /// most recent last.
    pub(super) decorations: Mutex<Vec<(String, DecorationSlot)>>,
    /// What `BlendingData` asks of the blocks of an old chunk, by state.
    pub(super) old_chunk_facts: OnceLock<OldChunkFacts>,
}
/// `BlendingData.isGround` and `SURFACE_BLOCKS`, by block state id.
pub(super) struct OldChunkFacts {
    ground: Vec<bool>,
    surface: Vec<bool>,
}
type RingCacheEntry = (
    (i64, String),
    Arc<OnceLock<super::super::structure::start::Rings>>,
);
/// Generators whose ring positions are kept: the dimensions of a few worlds.
pub(super) const RING_CACHE_LIMIT: usize = 8;
type DecorationSlot =
    Arc<OnceLock<Result<Option<Arc<super::super::generator_decoration::Decoration>>, String>>>;
/// Biome sources whose decoration is kept: the three dimensions and a spare.
const DECORATION_CACHE_LIMIT: usize = 4;
type Bundle = BTreeMap<String, String>;
/// The bundle `name` with the data packs' entries over it.
fn read_bundle(
    dir: &Path,
    overrides: &WorldgenOverrides,
    name: &str,
) -> Result<Bundle, GeneratorError> {
    let path = dir.join(format!("{name}.json"));
    let text =
        fs::read_to_string(&path).map_err(|e| resource(format!("{}: {e}", path.display())))?;
    Ok(overrides.apply(name, serde_json::from_str(&text)?))
}
fn read_optional_bundle(
    dir: &Path,
    overrides: &WorldgenOverrides,
    name: &str,
) -> Result<Option<Bundle>, GeneratorError> {
    if dir.join(format!("{name}.json")).exists() {
        read_bundle(dir, overrides, name).map(Some)
    } else {
        Ok(None)
    }
}
/// The tags of the bundle `name`, the data packs' tag files after the
/// prepared ones. A tag naming what is not there fails on its own, as
/// vanilla's TagLoader leaves it out. Without packs the prepared tags must
/// all resolve; with them (a pack's tag can also break a prepared tag that
/// includes it), each failed tag goes into `dropped` for the log.
pub(super) fn build_tags(
    raw: &Bundle,
    overrides: &WorldgenOverrides,
    name: &str,
    dropped: &mut Vec<String>,
    known: impl Fn(&str) -> bool,
) -> Result<BTreeMap<String, Vec<String>>, GeneratorError> {
    let mut definitions = TagDefinitions::default();
    for (name, json) in raw {
        definitions.apply_json(name, "vanilla", json)?;
    }
    for (id, pack, json) in overrides.tags(name) {
        definitions
            .apply_json(id, pack, json)
            .map_err(|e| resource(format!("{pack}: tag {id}: {e}")))?;
    }
    let built = definitions
        .build(|id, _| known(id).then(|| id.to_owned()))
        .map_err(|e| resource(format!("tags: {e:?}")))?;
    if overrides.tags(name).next().is_none() && !built.missing.is_empty() {
        return Err(resource(format!(
            "unresolved tag entries in {:?}",
            built.missing.keys().collect::<Vec<_>>()
        )));
    }
    for (tag, entries) in &built.missing {
        dropped.push(format!(
            "{name}: tag {tag} names what is missing: {}",
            entries
                .iter()
                .map(|e| format!("{:?} ({})", e.entry, e.source))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    Ok(built.tags)
}
impl GeneratorData {
    /// Load a worldgen bundle directory. Required files are JSON objects mapping
    /// resource identifiers to the resource's JSON text: `noises`, `densities`,
    /// `rules` (material rules), `conditions` (material conditions), `settings`
    /// (noise settings), `carvers`, `biomes`, `biome_tags`, `block_tags`,
    /// `world_presets`; `block_flags.json` is an array of `[is_air, has_fluid]`
    /// per registry state. Multi-noise tables come from `presets.json`
    /// (quantized) or `biome_parameters/<name>.json` data-generator reports.
    /// Optional decoration inputs: `features`, `placed_features`,
    /// `feature_tags`, `fluid_tags`, `state_providers` and the Java-exported
    /// block behavior files in `blocks/` (see `generator_blocks`). Integer block properties are derived from
    /// the registry: properties whose every value is a decimal integer.
    pub fn load(dir: &Path, registry: &'static BlockRegistry) -> Result<Arc<Self>, GeneratorError> {
        Self::load_with(dir, registry, &WorldgenOverrides::default())
    }
    /// `load` with a world's data packs over the bundles (see `overrides`).
    pub fn load_with(
        dir: &Path,
        registry: &'static BlockRegistry,
        ov: &WorldgenOverrides,
    ) -> Result<Arc<Self>, GeneratorError> {
        let mut warnings = Vec::new();
        let flags: Vec<(bool, bool)> =
            serde_json::from_str(&fs::read_to_string(dir.join("block_flags.json"))?)?;
        if flags.len() != registry.len() {
            return Err(resource("block_flags.json does not match the registry"));
        }
        let block_tags = build_tags(
            &read_bundle(dir, ov, "block_tags")?,
            ov,
            "block_tags",
            &mut warnings,
            |id| registry.default_state(id).is_some(),
        )?;
        let fluid_tags = match read_optional_bundle(dir, ov, "fluid_tags")? {
            Some(raw) => build_tags(&raw, ov, "fluid_tags", &mut warnings, |id| {
                matches!(
                    id,
                    "minecraft:water"
                        | "minecraft:flowing_water"
                        | "minecraft:lava"
                        | "minecraft:flowing_lava"
                        | "minecraft:empty"
                )
            })?,
            None => BTreeMap::new(),
        };
        // The three biggest reads (the block tables, the structure templates
        // and the block behaviours) need only the tags: they run on threads of
        // their own while this one reads the rest. A new world's first chunks
        // wait for all of it, with the chunk workers idle.
        let blocks = loading({
            let (dir, tags, fluids) = (dir.join("blocks"), block_tags.clone(), fluid_tags.clone());
            move || {
                super::super::generator_blocks::BlockBehavior::load(&dir, registry, &tags, &fluids)
            }
        })?;
        let structures = loading({
            let (dir, tags, ov) = (dir.to_owned(), block_tags.clone(), ov.clone());
            move || -> Result<_, GeneratorError> {
                let Ok(json) = fs::read_to_string(dir.join("blocks/structure-blocks.json")) else {
                    return Ok(None);
                };
                Ok(Some(Arc::new(
                    super::super::structure::StructureResources::new(
                        registry,
                        super::super::structure::StateTransforms::from_json(&json, registry.len())?,
                        &dir.join("structure"),
                        read_optional_bundle(&dir, &ov, "processor_lists")?.unwrap_or_default(),
                        &tags,
                    )
                    .with_templates(ov.templates().clone()),
                )))
            }
        })?;
        let behaviours = loading({
            let root = dir.parent().map(Path::to_owned);
            move || {
                root.filter(|root| root.join("block-behaviours.json").exists())
                    .and_then(|root| crate::level::Behaviours::load(registry, &root).ok())
                    .map(Arc::new)
            }
        })?;
        let tag_set = |name: &str| -> Result<BTreeSet<&str>, GeneratorError> {
            Ok(block_tags
                .get(name)
                .ok_or_else(|| resource(format!("missing block tag {name}")))?
                .iter()
                .map(String::as_str)
                .collect())
        };
        let motion = tag_set("minecraft:blocks_motion_in_heightmap")?;
        let no_leaves = tag_set("minecraft:blocks_motion_in_heightmap_no_leaves")?;
        let states = (0..registry.len())
            .map(|i| {
                let state = registry.state(BlockStateId(i as u32)).unwrap();
                GenBlockState {
                    air: flags[i].0,
                    air_block: state.name == "minecraft:air",
                    fluid: flags[i].1,
                    motion: motion.contains(state.name.as_str()),
                    no_leaves: no_leaves.contains(state.name.as_str()),
                }
            })
            .collect::<Vec<_>>();
        let integers = super::super::block_feature::integer_properties(registry);
        let biomes = read_bundle(dir, ov, "biomes")?;
        let biome_tags = build_tags(
            &read_bundle(dir, ov, "biome_tags")?,
            ov,
            "biome_tags",
            &mut warnings,
            |id| biomes.contains_key(id),
        )?;
        let biome_set = biomes.keys().cloned().collect::<BTreeSet<_>>();
        let conditions = MaterialConditionRegistry::from_json_with_biome_tags(
            &read_bundle(dir, ov, "conditions")?,
            &biome_set,
            &biome_tags,
        )?;
        let mut presets = read_optional_bundle(dir, ov, "presets")?.unwrap_or_default();
        let reports = dir.join("biome_parameters");
        if reports.is_dir() {
            for entry in fs::read_dir(&reports)? {
                let path = entry?.path();
                let Some(stem) = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .filter(|_| path.extension().is_some_and(|e| e == "json"))
                else {
                    continue;
                };
                presets
                    .entry(format!("minecraft:{stem}"))
                    .or_insert(quantize_parameter_report(&fs::read_to_string(&path)?)?);
            }
        }
        let source_catalog =
            BiomeSourceCatalog::new(&biome_set, &biome_tags, &presets.keys().cloned().collect())?;
        let carvers = CarverCatalog::from_json(&read_bundle(dir, ov, "carvers")?)?;
        let left_out: BTreeSet<String> =
            match fs::read_to_string(dir.join(crate::worldgen::carver::LEFT_OUT)) {
                Ok(text) => serde_json::from_str(&text)?,
                Err(_) => BTreeSet::new(),
            };
        let mut biome_names = Vec::new();
        let mut biome_ids = BTreeMap::new();
        let mut biome_json = Vec::new();
        let mut temperatures = Vec::new();
        let mut biome_carvers = Vec::new();
        for (name, json) in &biomes {
            biome_ids.insert(name.clone(), BiomeId(biome_names.len() as u32));
            biome_names.push(Arc::<str>::from(name.as_str()));
            temperatures.push(BiomeTemperatureSettings::from_json(json)?);
            let value: Value = serde_json::from_str(json)?;
            let (set, dropped) =
                carvers.decode_set_leaving_out(&value["carvers"].to_string(), &left_out)?;
            warnings.extend(dropped.into_iter().map(|id| {
                format!("biome {name}: carver {id} was left out of the mods' content, dropped")
            }));
            biome_carvers.push(set);
            biome_json.push(json.clone());
        }
        let carving_blocks = CarvingBlocks::from_registry(
            registry,
            &tag_set("minecraft:uncarvable")?
                .into_iter()
                .map(str::to_owned)
                .collect(),
        )
        .map_err(|e| resource(format!("carving blocks: {e}")))?;
        let feature_resources = match (
            read_optional_bundle(dir, ov, "features")?,
            read_optional_bundle(dir, ov, "placed_features")?,
        ) {
            (Some(features), Some(placed)) => Some(FeatureResources {
                features,
                placed,
                feature_tags: read_optional_bundle(dir, ov, "feature_tags")?.unwrap_or_default(),
                blocks: joined(blocks)?,
                fluid_tags,
                state_providers: read_optional_bundle(dir, ov, "state_providers")?
                    .unwrap_or_default(),
            }),
            _ => None,
        };
        let structures = joined(structures)?;
        let structure_resources = match (
            read_optional_bundle(dir, ov, "structures")?,
            read_optional_bundle(dir, ov, "structure_sets")?,
            read_optional_bundle(dir, ov, "template_pools")?,
        ) {
            (Some(structures), Some(sets), Some(pools)) => Some(StructureBundles {
                structures,
                sets,
                pools,
            }),
            _ => None,
        };
        let behaviours = joined(behaviours);
        Ok(Arc::new(Self {
            behaviours,
            structure_resources,
            structures,
            registry,
            states: GenBlockStates(states.into()),
            integers,
            noises: read_bundle(dir, ov, "noises")?,
            densities: read_bundle(dir, ov, "densities")?,
            rules: read_bundle(dir, ov, "rules")?,
            settings: read_bundle(dir, ov, "settings")?,
            world_presets: read_bundle(dir, ov, "world_presets")?,
            dimensions: ov.apply(overrides::DIMENSIONS, BTreeMap::new()),
            dimension_types: ov.apply(overrides::DIMENSION_TYPES, BTreeMap::new()),
            warnings,
            presets,
            conditions,
            source_catalog,
            biome_names,
            biome_ids,
            biome_json,
            temperatures,
            biome_carvers,
            carvers,
            accepted_registries: OnceLock::new(),
            decoded_structures: OnceLock::new(),
            carving_blocks,
            block_tags,
            biome_tags,
            feature_resources,
            ring_cache: Mutex::new(Vec::new()),
            decorations: Mutex::new(Vec::new()),
            old_chunk_facts: OnceLock::new(),
            bound_block_tags: std::sync::OnceLock::new(),
        }))
    }
    /// The concentric ring positions (strongholds) of the generator for
    /// `seed` and the dimension `generator` definition, shared by every
    /// generator built for them. They depend on nothing else, so returning
    /// to a dimension, which builds its generator again, finds them done.
    pub(in crate::worldgen) fn rings_for(
        &self,
        seed: i64,
        generator: &Value,
    ) -> Arc<OnceLock<super::super::structure::start::Rings>> {
        let key = (seed, generator.to_string());
        let mut cache = self.ring_cache.lock().unwrap_or_else(|e| e.into_inner());
        let rings = match cache.iter().position(|(k, _)| *k == key) {
            Some(i) => cache.remove(i).1,
            None => Arc::new(OnceLock::new()),
        };
        if cache.len() >= RING_CACHE_LIMIT {
            cache.remove(0);
        }
        cache.push((key, rings.clone()));
        rings
    }
    /// The placed-feature decoration of the biome source `source_json`
    /// (resolved: `source`), shared by every generator that uses it. It
    /// depends on these resources and the source alone, not on the seed, and
    /// building it (decoding the placed features of every biome the source
    /// can place) took about 140 ms of each world opening. A caller that
    /// arrives while another builds it waits for that build.
    pub(in crate::worldgen) fn decoration_for(
        self: &Arc<Self>,
        source_json: &str,
        source: &ResolvedBiomeSource,
    ) -> Result<Option<Arc<super::super::generator_decoration::Decoration>>, GeneratorError> {
        let slot = {
            let mut cache = self.decorations.lock().unwrap_or_else(|e| e.into_inner());
            let slot = match cache.iter().position(|(k, _)| k == source_json) {
                Some(i) => cache.remove(i).1,
                None => Arc::new(OnceLock::new()),
            };
            if cache.len() >= DECORATION_CACHE_LIMIT {
                cache.remove(0);
            }
            cache.push((source_json.to_owned(), slot.clone()));
            slot
        };
        slot.get_or_init(|| {
            super::super::generator_decoration::Decoration::new(self, source)
                .map(|d| d.map(Arc::new))
                .map_err(|e| e.to_string())
        })
        .clone()
        .map_err(resource)
    }
    /// Builds the decoration of `preset`'s `dimension` generator before a
    /// world needs it (the world screens' preload), so opening the world
    /// finds it ready.
    pub fn prepare_decoration(
        self: &Arc<Self>,
        preset: &str,
        dimension: &str,
    ) -> Result<(), GeneratorError> {
        let preset_json: Value = serde_json::from_str(
            self.world_presets
                .get(preset)
                .ok_or_else(|| resource(format!("missing world preset {preset}")))?,
        )?;
        let source_json =
            preset_json["dimensions"][dimension]["generator"]["biome_source"].to_string();
        let source = self
            .source_catalog
            .decode(&source_json)?
            .resolve(&self.presets, 0)
            .map_err(|e| resource(format!("biome source: {e}")))?;
        self.decoration_for(&source_json, &source).map(drop)
    }
    pub fn registry(&self) -> &'static BlockRegistry {
        self.registry
    }
    /// What loading the data packs' entries left out (tags naming what is
    /// missing), for the log.
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }
    pub fn block_states(&self) -> &GenBlockStates {
        &self.states
    }
    /// Biome identifiers in registry (identifier-sorted) order.
    pub fn configured_feature_json(&self, name: &str) -> Option<&str> {
        self.feature_resources
            .as_ref()?
            .features
            .get(name)
            .map(String::as_str)
    }
    pub fn placed_feature_json(&self, name: &str) -> Option<&str> {
        self.feature_resources
            .as_ref()?
            .placed
            .get(name)
            .map(String::as_str)
    }
    pub fn biome_names(&self) -> &[Arc<str>] {
        &self.biome_names
    }
    fn old_chunk_facts(&self) -> &OldChunkFacts {
        self.old_chunk_facts.get_or_init(|| {
            let members = |tag: &str| -> std::collections::HashSet<&str> {
                self.block_tags
                    .get(tag)
                    .into_iter()
                    .flatten()
                    .map(String::as_str)
                    .collect()
            };
            let (leaves, logs) = (members("minecraft:leaves"), members("minecraft:logs"));
            let collision = self
                .feature_resources
                .as_ref()
                .and_then(|resources| resources.blocks.as_ref())
                .map(|blocks| &blocks.collision);
            let surface_names = [
                "minecraft:podzol",
                "minecraft:gravel",
                "minecraft:grass_block",
                "minecraft:stone",
                "minecraft:coarse_dirt",
                "minecraft:sand",
                "minecraft:red_sand",
                "minecraft:mycelium",
                "minecraft:snow_block",
                "minecraft:terracotta",
                "minecraft:dirt",
            ];
            let (mut ground, mut surface) = (Vec::new(), Vec::new());
            for index in 0..self.registry.len() {
                let id = BlockStateId(index as u32);
                let name = self.registry.state(id).map_or("", |s| s.name.as_str());
                surface.push(surface_names.contains(&name));
                // An empty collision shape: air, the liquids (no entity stands
                // on them), powder snow and the like; and leaves, logs and
                // huge mushrooms, which `isGround` leaves out.
                let excluded = self.states.get(id).air
                    || leaves.contains(name)
                    || logs.contains(name)
                    || matches!(
                        name,
                        "minecraft:brown_mushroom_block"
                            | "minecraft:red_mushroom_block"
                            | "minecraft:water"
                            | "minecraft:lava"
                            | "minecraft:powder_snow"
                    );
                ground.push(
                    !excluded
                        && match collision {
                            Some(registry) => registry
                                .collision_block(id)
                                .map_or(true, |block| !block.empty_shape),
                            None => self.states.get(id).motion,
                        },
                );
            }
            OldChunkFacts { ground, surface }
        })
    }
    /// `BlendingData.isGround` of a block state of an old chunk.
    pub fn is_old_chunk_ground(&self, state: BlockStateId) -> bool {
        self.old_chunk_facts()
            .ground
            .get(state.0 as usize)
            .copied()
            .unwrap_or(false)
    }
    /// Whether the block state is one of `BlendingData.SURFACE_BLOCKS`.
    pub fn is_old_chunk_surface(&self, state: BlockStateId) -> bool {
        self.old_chunk_facts()
            .surface
            .get(state.0 as usize)
            .copied()
            .unwrap_or(false)
    }
    pub fn biome_name(&self, id: BiomeId) -> &str {
        &self.biome_names[id.0 as usize]
    }
    pub fn biome_id(&self, name: &str) -> Option<BiomeId> {
        self.biome_ids.get(name).copied()
    }
    /// The biome's resource JSON (climate, effects such as water/sky/foliage
    /// colors, attributes, carvers and features).
    pub fn biome_json(&self, id: BiomeId) -> &str {
        &self.biome_json[id.0 as usize]
    }
    pub(in crate::worldgen) fn integers(&self) -> &BTreeSet<(String, String)> {
        &self.integers
    }
    /// Feature block bindings over this data's blocks and block tags. The tags'
    /// state sets are bound once and shared: each binding held 5 MB of them.
    pub(in crate::worldgen) fn feature_block_bindings(
        &self,
    ) -> Result<super::super::block_feature::FeatureBlockBindings<'static>, DecodeError> {
        use super::super::block_feature::FeatureBlockBindings;
        let bound = match self.bound_block_tags.get() {
            Some(bound) => bound.clone(),
            None => {
                let bound = FeatureBlockBindings::bind_tags(self.registry, &self.block_tags)?;
                self.bound_block_tags.get_or_init(|| bound).clone()
            }
        };
        FeatureBlockBindings::with_bound_tags(self.registry, self.integers.clone(), bound)
    }
}

impl GeneratorData {
    /// The loaded worldgen registry's retained table, not files present in a resource pack.
    /// Lazily retain successful codec results. Raw bundles are resources, not registry entries.
    fn named_registry(&self, registry: &str) -> Option<&RegistryNames> {
        let decoders = self.accepted_registries.get_or_init(|| {
            let decoders: [(&str, RegistryDecoder); 9] = [
                ("minecraft:worldgen/noise", Self::decode_noise_names),
                (
                    "minecraft:worldgen/density_function",
                    Self::decode_density_names,
                ),
                ("minecraft:worldgen/material_rule", Self::decode_rule_names),
                (
                    "minecraft:worldgen/noise_settings",
                    Self::decode_settings_names,
                ),
                (
                    "minecraft:worldgen/multi_noise_biome_source_parameter_list",
                    Self::decode_preset_names,
                ),
                (
                    "minecraft:worldgen/processor_list",
                    Self::decode_processor_names,
                ),
                ("minecraft:worldgen/structure", Self::decode_structure_names),
                (
                    "minecraft:worldgen/structure_set",
                    Self::decode_structure_set_names,
                ),
                ("minecraft:worldgen/template_pool", Self::decode_pool_names),
            ];
            decoders
                .into_iter()
                .map(|(name, decode)| (name, (OnceLock::new(), decode)))
                .collect()
        });
        let (names, decode) = decoders.get(registry)?;
        names.get_or_init(|| decode(self)).as_ref()
    }

    fn decode_noise_names(&self) -> Option<RegistryNames> {
        DensityRegistry::from_json(&self.noises, &BTreeMap::new()).ok()?;
        Some(
            self.noises
                .keys()
                .filter_map(|name| super::super::density::identifier(name).ok())
                .collect(),
        )
    }

    fn decode_density_names(&self) -> Option<RegistryNames> {
        let densities = DensityRegistry::from_json(&self.noises, &self.densities).ok()?;
        Some(
            densities
                .entries()
                .map(|(name, _)| name.to_owned())
                .collect(),
        )
    }

    fn decode_rule_names(&self) -> Option<RegistryNames> {
        let densities = DensityRegistry::from_json(&self.noises, &self.densities).ok()?;
        let codec =
            crate::registry::BlockStateJsonCodec::new(self.registry, self.integers.clone()).ok()?;
        let rules = MaterialRuleLibrary::from_json(
            &self.rules,
            &self.conditions,
            &codec,
            densities,
            0,
            false,
        )
        .ok()?;
        Some(rules.names().map(str::to_owned).collect())
    }

    fn decode_settings_names(&self) -> Option<RegistryNames> {
        Some(
            self.settings
                .iter()
                .filter_map(|(name, json)| {
                    let value: Value = serde_json::from_str(json).ok()?;
                    self.decode_noise_settings(&value, 0, None).ok()?;
                    super::super::density::identifier(name).ok()
                })
                .collect(),
        )
    }

    fn decode_preset_names(&self) -> Option<RegistryNames> {
        Some(
            self.presets
                .keys()
                .filter_map(|name| {
                    let source = BiomeSourceDefinition::Preset {
                        name: name.clone(),
                        inline: false,
                    };
                    source.resolve(&self.presets, 0).ok()?;
                    super::super::density::identifier(name).ok()
                })
                .collect(),
        )
    }

    fn decode_processor_names(&self) -> Option<RegistryNames> {
        let resources = self.structures.as_ref()?;
        let bindings = self.structure_block_bindings().ok()?;
        Some(
            resources
                .processor_lists
                .iter()
                .filter_map(|(name, json)| {
                    resources.processor_list(json, &bindings).ok()?;
                    super::super::density::identifier(name).ok()
                })
                .collect(),
        )
    }

    fn decoded_structure_registry(
        &self,
    ) -> Option<&super::super::structure::start::StructureRegistry> {
        self.decoded_structures
            .get_or_init(|| {
                let raw = self.structure_resources.as_ref()?;
                super::super::structure::start::StructureRegistry::new(
                    &raw.structures,
                    &raw.sets,
                    raw.pools.clone(),
                    &|value| self.structure_biome_set(value),
                    super::super::structure::mod_types::mod_structure_types(),
                )
                .ok()
            })
            .as_ref()
    }

    fn decode_structure_names(&self) -> Option<RegistryNames> {
        Some(
            self.decoded_structure_registry()?
                .structures
                .iter()
                .map(|entry| entry.name.clone())
                .collect(),
        )
    }

    fn decode_structure_set_names(&self) -> Option<RegistryNames> {
        Some(
            self.decoded_structure_registry()?
                .sets
                .iter()
                .map(|entry| entry.name.clone())
                .collect(),
        )
    }

    fn decode_pool_names(&self) -> Option<RegistryNames> {
        let structures = self.decoded_structure_registry()?;
        let raw = self.structure_resources.as_ref()?;
        let resources = self.structures.as_ref()?;
        let bindings = self.structure_block_bindings().ok()?;
        Some(
            raw.pools
                .keys()
                .filter_map(|name| {
                    structures.pools.get(name, resources, &bindings)?;
                    super::super::density::identifier(name).ok()
                })
                .collect(),
        )
    }

    /// `RegistryAccess.lookup` / `Registry.containsKey` on this generator's immutable snapshot.
    /// None distinguishes an unavailable registry from an absent entry in a loaded registry.
    pub fn registry_contains(&self, registry: &str, id: &str) -> Option<bool> {
        Some(match registry {
            "minecraft:worldgen/biome" => self.biome_ids.contains_key(id),
            "minecraft:worldgen/carver" => self.carvers.contains(id),
            "minecraft:worldgen/material_condition" => self.conditions.contains(id),
            _ => self.named_registry(registry)?.contains(id),
        })
    }

    /// `HolderLookup.listElementIds` from the same table used by `registry_contains`.
    pub fn registry_ids(&self, registry: &str) -> Option<Vec<&str>> {
        Some(match registry {
            "minecraft:worldgen/biome" => self.biome_names.iter().map(AsRef::as_ref).collect(),
            "minecraft:worldgen/carver" => self.carvers.names().collect(),
            "minecraft:worldgen/material_condition" => {
                self.conditions.entries().map(|(id, _)| id).collect()
            }
            _ => self
                .named_registry(registry)?
                .iter()
                .map(String::as_str)
                .collect(),
        })
    }

    /// Tags retained by the generator, resolved against its own accepted entry names.
    pub fn registry_tag_contains(&self, registry: &str, id: &str, tag: &str) -> Option<bool> {
        if !self.registry_contains(registry, id)? {
            return None;
        }
        // Only biome tags are the generator's own; the owner of another
        // registry's tags answers for them.
        if registry != "minecraft:worldgen/biome" {
            return None;
        }
        Some(
            self.biome_tags
                .get(tag)
                .is_some_and(|members| members.iter().any(|member| member == id)),
        )
    }
}
