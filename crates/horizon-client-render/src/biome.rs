//! Biome colours from the pinned biome definitions and colormaps
//! (`Biome.getGrassColor`/`getFoliageColor`/..., `BlockTintSources`).
use crate::Result;
use horizon_assets::resource_source::{ResourceRoot, ResourceSource};
use horizon_client_core::particle_type::ParticleType;
use horizon_core::{noise::SimplexNoise, position::BlockPos, random::LegacyRandom};
pub use mod_api::world::biome::{BiomeColors, SpawnEntry};
use serde_json::Value;
use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::{Arc, Mutex, OnceLock},
};

/// The missing saved biomes already warned about in the open world: each dimension's table
/// keeps its own placeholders, but a name is reported once per world.
static WARNED_MISSING: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GrassModifier {
    None,
    DarkForest,
    Swamp,
}
/// A `natural_mob_spawns` / `spawn_overrides` entry list.
pub fn parse_entries(list: &Value) -> Vec<SpawnEntry> {
    list.as_array()
        .into_iter()
        .flatten()
        .filter_map(|e| {
            let count = &e["count"];
            let (min, max, constant) = match count {
                Value::Number(n) => {
                    let n = n.as_u64().unwrap_or(1) as u32;
                    (n, n, true)
                }
                c if c["type"].as_str() == Some("minecraft:constant") => {
                    let n = c["value"].as_u64().unwrap_or(1) as u32;
                    (n, n, true)
                }
                c => (
                    c["min_inclusive"].as_u64().unwrap_or(1) as u32,
                    c["max_inclusive"].as_u64().unwrap_or(1) as u32,
                    false,
                ),
            };
            Some(SpawnEntry {
                entity: e["type"].as_str()?.to_owned(),
                weight: e["weight"].as_u64().unwrap_or(1) as u32,
                min,
                max,
                constant,
            })
        })
        .collect()
}
/// `MobSpawnSettings.MobSpawnCost`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpawnCost {
    pub energy_budget: f64,
    pub charge: f64,
}
/// The `spawn_costs` of a `natural_mob_spawns` value.
pub fn parse_costs(costs: &Value) -> HashMap<String, SpawnCost> {
    costs
        .as_object()
        .into_iter()
        .flatten()
        .map(|(k, c)| {
            (
                k.clone(),
                SpawnCost {
                    energy_budget: c["energy_budget"].as_f64().unwrap_or(0.0),
                    charge: c["charge"].as_f64().unwrap_or(0.0),
                },
            )
        })
        .collect()
}

#[derive(Clone, Debug)]
pub struct BiomeInfo {
    /// `natural_mob_spawns` by category (creature, monster, ...).
    pub spawns: HashMap<String, Vec<SpawnEntry>>,
    /// `natural_mob_spawns` spawn costs by entity type.
    pub spawn_costs: HashMap<String, SpawnCost>,
    /// The `gameplay/creature_world_gen_spawn_probability` attribute.
    pub creature_probability: f32,
    /// The `gameplay/increased_fire_burnout` attribute (jungles, swamps, peaks,
    /// mushroom fields).
    pub increased_fire_burnout: bool,
    pub name: String,
    pub temperature: f32,
    /// `temperature_modifier: frozen` (the frozen oceans).
    pub frozen: bool,
    pub downfall: f32,
    pub precipitation: bool,
    pub grass: u32,
    pub modifier: GrassModifier,
    pub foliage: u32,
    pub dry_foliage: u32,
    pub water: u32,
    pub sky: Option<u32>,
    /// The biome's own fog colour attribute; otherwise the dimension's applies.
    pub fog: Option<u32>,
    pub water_fog: u32,
    /// `minecraft:visual/ambient_particles` entries and their probabilities.
    pub ambient_particles: Vec<(ParticleType, f32)>,
    /// `BiomeGenerationSettings.features`: the placed feature ids of each
    /// `GenerationStep.Decoration` step, in order.
    pub features: Vec<Vec<String>>,
    /// The biome's environment attributes (`Biome.getAttributes`): its layer in the
    /// environment attribute stack.
    pub attributes: horizon_world::environment_attributes::AttributeMap,
}
impl BiomeInfo {
    fn ambient_particles(v: &Value) -> Vec<(ParticleType, f32)> {
        let list = &v["attributes"]["minecraft:visual/ambient_particles"];
        let entries = list["argument"].as_array().or_else(|| list.as_array());
        entries
            .into_iter()
            .flatten()
            .filter_map(|entry| {
                Some((
                    ParticleType::from_name(entry["particle"]["type"].as_str()?)?,
                    entry["probability"].as_f64().unwrap_or(0.0) as f32,
                ))
            })
            .collect()
    }

    /// `Biome.getHeightAdjustedTemperature`: the frozen modifier, then cooler
    /// with height above `sea_level + 17`.
    pub fn temperature_at(&self, pos: BlockPos, sea_level: i32) -> f32 {
        use horizon_world::worldgen::biome_temperature::{
            BiomeTemperatureSettings, TemperatureModifier,
        };
        BiomeTemperatureSettings {
            base_temperature: self.temperature,
            modifier: if self.frozen {
                TemperatureModifier::Frozen
            } else {
                TemperatureModifier::None
            },
            has_precipitation: self.precipitation,
            downfall: self.downfall,
        }
        .height_adjusted(pos, sea_level)
    }
    /// `Biome.getPrecipitationAt` in the Overworld (sea level 63): None
    /// without precipitation, else whether it falls as snow
    /// (`getHeightAdjustedTemperature` below 0.15).
    pub fn snows_at(&self, x: i32, y: i32, z: i32) -> Option<bool> {
        self.precipitation
            .then(|| self.temperature_at(BlockPos::new(x, y, z), 63) < 0.15)
    }
}
/// The game's biomes, by id. The built-in (vanilla) biomes come first, in
/// file order, and keep their ids whatever a world's data packs do; a pack's
/// new biomes follow them (`with_packs`). Those are the client's own ids:
/// the network registry's are `registry_order`'s.
#[derive(Clone)]
pub struct Biomes {
    pub list: Vec<BiomeInfo>,
    pub index: HashMap<String, u16>,
    info_noise: SimplexNoise,
    colormaps: Arc<Colormaps>,
    /// The built-in biomes, the first of `list` unless packs replaced them.
    builtin: Arc<Vec<BiomeInfo>>,
    /// The data packs' biome files (new biomes and replaced built-in ones),
    /// for the tables that read more of a biome than `BiomeInfo` keeps.
    packs: BTreeMap<String, Arc<Value>>,
    /// The built-in biomes' own files, to tell a server's data that differs.
    files: Arc<BTreeMap<String, Value>>,
    /// Biome names read from saved chunks that this table lacks (a removed mod's biome), in
    /// first-read order: placeholder `i` has id `list.len() + i`. A placeholder plays as
    /// plains, but a chunk saves its name back unchanged. Shared by clones (recoloured
    /// copies keep the ids); a table rebuilt with other packs starts empty.
    placeholders: Arc<Mutex<Vec<String>>>,
    exhausted: Arc<std::sync::atomic::AtomicBool>,
}
/// The grass, foliage and dry foliage colormaps.
struct Colormaps {
    grass: Vec<u32>,
    foliage: Vec<u32>,
    dry: Vec<u32>,
}
fn color(v: &Value) -> Option<u32> {
    match v {
        Value::String(s) => u32::from_str_radix(s.trim_start_matches('#'), 16)
            .ok()
            .map(|c| c & 0xFF_FFFF),
        Value::Number(n) => n.as_i64().map(|c| c as u32 & 0xFF_FFFF),
        _ => None,
    }
}
fn colormap(root: &ResourceSource, name: &str) -> Result<Vec<u32>> {
    let path = format!("assets/minecraft/textures/colormap/{name}.png");
    let mut decoder = png::Decoder::new(std::io::Cursor::new(root.read(&path)?));
    decoder.ignore_checksums(true);
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info()?;
    let mut bytes = vec![0; reader.output_buffer_size().ok_or("colormap too large")?];
    let info = reader.next_frame(&mut bytes)?;
    let channels = info.color_type.samples();
    Ok(bytes[..info.buffer_size()]
        .chunks_exact(channels)
        .map(|p| u32::from(p[0]) << 16 | u32::from(p[1]) << 8 | u32::from(p[2]))
        .collect())
}
/// `GrassColor.get`/`FoliageColor.get`: temperature and downfall index a 256x256 map.
fn sample(map: &[u32], temperature: f32, downfall: f32, fallback: u32) -> u32 {
    let temperature = f64::from(temperature.clamp(0.0, 1.0));
    let downfall = f64::from(downfall.clamp(0.0, 1.0)) * temperature;
    let x = ((1.0 - temperature) * 255.0) as i32;
    let y = ((1.0 - downfall) * 255.0) as i32;
    let index = (y << 8 | x) as usize;
    map.get(index).copied().unwrap_or(fallback)
}
impl BiomeInfo {
    /// A biome file's (`worldgen/biome/<name>.json`) values, the colours it
    /// leaves out sampled from the colormaps.
    fn from_json(name: String, v: &Value, maps: &Colormaps) -> Self {
        let effects = &v["effects"];
        let attributes = &v["attributes"];
        let temperature = v["temperature"].as_f64().unwrap_or(0.5) as f32;
        let downfall = v["downfall"].as_f64().unwrap_or(0.5) as f32;
        let modifier = match effects["grass_color_modifier"].as_str() {
            Some("dark_forest") => GrassModifier::DarkForest,
            Some("swamp") => GrassModifier::Swamp,
            _ => GrassModifier::None,
        };
        let mut spawns: HashMap<String, Vec<SpawnEntry>> = HashMap::new();
        let natural = &attributes["minecraft:gameplay/natural_mob_spawns"];
        // A modifier object ({argument, modifier: overlay}) or the plain value;
        // the overlay over the dimension's empty default is the biome's value.
        let natural = if natural["argument"].is_object() {
            &natural["argument"]
        } else {
            natural
        };
        let categories = &natural["spawns_by_category"];
        for (category, entries) in categories.as_object().into_iter().flatten() {
            let list = parse_entries(entries);
            spawns.insert(category.clone(), list);
        }
        BiomeInfo {
            spawns,
            spawn_costs: parse_costs(&natural["spawn_costs"]),
            creature_probability:
                attributes["minecraft:gameplay/creature_world_gen_spawn_probability"]
                    .as_f64()
                    .unwrap_or(0.1) as f32,
            increased_fire_burnout: attributes["minecraft:gameplay/increased_fire_burnout"]
                .as_bool()
                .unwrap_or(false),
            temperature,
            frozen: v["temperature_modifier"].as_str() == Some("frozen"),
            downfall,
            precipitation: v["has_precipitation"].as_bool().unwrap_or(true),
            grass: color(&effects["grass_color"])
                .unwrap_or_else(|| sample(&maps.grass, temperature, downfall, 0xFF00FF)),
            modifier,
            foliage: color(&effects["foliage_color"])
                .unwrap_or_else(|| sample(&maps.foliage, temperature, downfall, 0x48B518)),
            dry_foliage: color(&effects["dry_foliage_color"])
                .unwrap_or_else(|| sample(&maps.dry, temperature, downfall, 0x5D4930)),
            water: color(&effects["water_color"]).unwrap_or(0x3F76E4),
            sky: color(&attributes["minecraft:visual/sky_color"]),
            fog: color(&attributes["minecraft:visual/fog_color"]),
            water_fog: color(&attributes["minecraft:visual/water_fog_color"]).unwrap_or(0x050533),
            ambient_particles: Self::ambient_particles(v),
            features: v["features"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|step| {
                    step.as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .collect(),
            attributes: horizon_world::environment_attributes::AttributeMap::parse(attributes)
                .unwrap_or_else(|e| {
                    log::warn!("biome attributes: {e}");
                    Default::default()
                }),
            name,
        }
    }
}
/// `Identifier.compareTo`: the path first, then the namespace.
pub fn identifier_order(a: &str, b: &str) -> std::cmp::Ordering {
    fn split(id: &str) -> (&str, &str) {
        let (ns, path) = id.split_once(':').unwrap_or(("minecraft", id));
        (path, ns)
    }
    split(a).cmp(&split(b))
}
/// The content mods' own biomes (every id outside `minecraft`) from the content root's
/// `worldgen/biomes.json` bundle (id to file text), in registry order; none when the root has
/// no bundle. A file that does not parse is warned and left out.
pub fn mod_biome_files(prepared: &std::path::Path) -> Vec<(String, Value)> {
    let Ok(bytes) = std::fs::read(prepared.join("worldgen/biomes.json")) else {
        return Vec::new();
    };
    let bundle: BTreeMap<String, Value> = serde_json::from_slice(&bytes).unwrap_or_else(|e| {
        log::warn!("worldgen/biomes.json: {e}");
        Default::default()
    });
    let mut out: Vec<(String, Value)> = bundle
        .into_iter()
        .filter(|(id, _)| !id.starts_with("minecraft:"))
        .filter_map(|(id, text)| match serde_json::from_str(text.as_str()?) {
            Ok(v) => Some((id, v)),
            Err(e) => {
                log::warn!("mod biome {id}: {e}");
                None
            }
        })
        .collect();
    out.sort_by(|a, b| identifier_order(&a.0, &b.0));
    out
}
impl Biomes {
    pub fn load(root: &ResourceRoot) -> Result<Self> {
        let maps = Colormaps {
            grass: colormap(root.source(), "grass")?,
            foliage: colormap(root.source(), "foliage")?,
            dry: colormap(root.source(), "dry_foliage")?,
        };
        let mut list = Vec::new();
        let mut files_by_name = BTreeMap::new();
        // The folder's own files (not its subfolders'), by file name: through `json_folder`,
        // from the prepared data file when there is one.
        let folder = horizon_world::json_folder::read(
            &root.prepared().join("data/minecraft/worldgen/biome"),
        )?;
        let mut files: Vec<_> = folder
            .files
            .iter()
            .filter(|f| !f.name.contains('/'))
            .collect();
        files.sort_by_cached_key(|f| format!("{}.json", f.name));
        for file in files {
            let v: Value = horizon_world::json_file::parse(&file.path, file.bytes()?)?;
            list.push(BiomeInfo::from_json(
                format!("minecraft:{}", file.name),
                &v,
                &maps,
            ));
            files_by_name.insert(format!("minecraft:{}", file.name), v);
        }
        // Content mods' own biomes follow the built-in ones, so the built-in ids never move.
        for (name, v) in mod_biome_files(root.prepared()) {
            list.push(BiomeInfo::from_json(name.clone(), &v, &maps));
            files_by_name.insert(name, v);
        }
        let index = list
            .iter()
            .enumerate()
            .map(|(i, b)| (b.name.clone(), i as u16))
            .collect();
        Ok(Self {
            builtin: Arc::new(list.clone()),
            list,
            index,
            colormaps: Arc::new(maps),
            packs: BTreeMap::new(),
            files: Arc::new(files_by_name),
            info_noise: SimplexNoise::with_skipped_offsets(&mut LegacyRandom::new(2345), true),
            placeholders: Default::default(),
            exhausted: Default::default(),
        })
    }
    /// The built-in biomes with a world's data packs' biome files (id →
    /// JSON) over them, as `RegistryDataLoader` builds the registry: a file
    /// for a built-in biome replaces it, keeping its id, and new biomes join
    /// after the built-in ones, in registry order. Starts from the built-in
    /// table whatever packs `self` has.
    pub fn with_packs<'a>(&self, files: impl IntoIterator<Item = (&'a str, &'a Value)>) -> Self {
        let mut out = self.builtin_only();
        out.placeholders = Arc::default();
        out.exhausted = Arc::default();
        let mut files: Vec<(&str, &Value)> = files.into_iter().collect();
        files.sort_by(|a, b| identifier_order(a.0, b.0));
        for (name, json) in files {
            let info = BiomeInfo::from_json(name.to_owned(), json, &out.colormaps);
            match out.index.get(name) {
                Some(&id) => out.list[usize::from(id)] = info,
                None if out.list.len() < usize::from(u16::MAX) => {
                    out.index.insert(name.to_owned(), out.list.len() as u16);
                    out.list.push(info);
                }
                None => continue,
            }
            out.packs.insert(name.to_owned(), Arc::new(json.clone()));
        }
        out
    }
    /// The built-in biomes alone: this table without what packs gave it.
    pub fn builtin_only(&self) -> Self {
        let mut out = self.clone();
        if self.packs.is_empty() {
            return out;
        }
        out.list = self.builtin.to_vec();
        let builtin = out.list.len();
        out.index.retain(|_, id| usize::from(*id) < builtin);
        out.packs.clear();
        out
    }
    /// The file a world's biome `name` came from, if a pack added or replaced it or a content
    /// mod added it (a mod's biome is no built-in: no client has its file).
    pub fn pack_file(&self, name: &str) -> Option<&Value> {
        self.packs.get(name).map(|v| &**v).or_else(|| {
            self.files
                .get(name)
                .filter(|_| !name.starts_with("minecraft:"))
        })
    }
    /// The file the game's own copy of built-in biome `name` was read from.
    pub fn builtin_file(&self, name: &str) -> Option<&Value> {
        self.files.get(name)
    }
    /// Whether data packs added or replaced any biome.
    pub fn has_packs(&self) -> bool {
        !self.packs.is_empty()
    }
    /// The biome names in the network registry's order: `RegistryDataLoader`
    /// sorts a registry's files by `Identifier` (path, then namespace).
    pub fn registry_order(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self.list.iter().map(|b| b.name.as_str()).collect();
        names.sort_by(|a, b| identifier_order(a, b));
        names
    }
    pub fn id(&self, name: &str) -> u16 {
        self.index
            .get(name)
            .or_else(|| self.index.get("minecraft:plains"))
            .copied()
            .unwrap_or(0)
    }
    /// Whether the game has the biome `name`.
    pub fn contains(&self, name: &str) -> bool {
        self.index.contains_key(name)
    }
    /// Biome `id`, or plains (the last entry without one) when the id is past this table:
    /// a placeholder, or a column a server sent under its earlier, longer registry (a
    /// proxy's previous server), which keeps its ids until the next login replaces it.
    /// An empty table returns the shared default plains biome.
    pub fn get(&self, id: u16) -> &BiomeInfo {
        if self.list.is_empty() {
            static DEFAULT: OnceLock<BiomeInfo> = OnceLock::new();
            return DEFAULT.get_or_init(|| BiomeInfo {
                spawns: HashMap::new(),
                spawn_costs: HashMap::new(),
                creature_probability: 0.0,
                increased_fire_burnout: false,
                name: "minecraft:plains".to_owned(),
                temperature: 0.8,
                frozen: false,
                downfall: 0.4,
                precipitation: false,
                grass: 0x7f_c0_55,
                modifier: GrassModifier::None,
                foliage: 0x6a_ab_37,
                dry_foliage: 0x79_75_54,
                water: 0x3f_76_e4,
                sky: None,
                fog: None,
                water_fog: 0x3f_76_e4,
                ambient_particles: Vec::new(),
                features: Vec::new(),
                attributes: Default::default(),
            });
        }
        self.list.get(usize::from(id)).unwrap_or_else(|| {
            &self.list[usize::from(self.id("minecraft:plains")).min(self.list.len() - 1)]
        })
    }
    /// The id a saved chunk's biome `name` loads as: its own, or a placeholder that keeps the
    /// name for the next save when this table lacks it (`id` would make it plains for good).
    /// Exhaustion makes this table unsaveable; the original on-disk data must remain intact.
    pub fn saved_id(&self, name: &str) -> u16 {
        if let Some(&id) = self.index.get(name) {
            return id;
        }
        let mut placeholders = self.placeholders.lock().unwrap_or_else(|e| e.into_inner());
        let mut warned = WARNED_MISSING.lock().unwrap_or_else(|e| e.into_inner());
        if warned.insert(name.to_owned()) {
            log::warn!(
                "saved biome {name} is not in this world's biomes; it plays as plains and is saved unchanged"
            );
        }
        let slot = match placeholders.iter().position(|p| p == name) {
            Some(slot) => slot,
            None => {
                if self.list.len() + placeholders.len() > usize::from(u16::MAX) {
                    self.exhausted
                        .store(true, std::sync::atomic::Ordering::Release);
                    log::error!(
                        "{BiomeIdExhausted}: cannot retain saved biome {name}; saving is refused"
                    );
                    return self.id("minecraft:plains");
                }
                placeholders.push(name.to_owned());
                placeholders.len() - 1
            }
        };
        (self.list.len() + slot) as u16
    }
    /// Refuse writes after any saved name could not be represented. This is sticky for
    /// the world table: subsequent loads cannot make a destructive save safe again.
    pub fn ensure_saveable(&self) -> std::result::Result<(), BiomeIdExhausted> {
        if self.exhausted.load(std::sync::atomic::Ordering::Acquire) {
            Err(BiomeIdExhausted)
        } else {
            Ok(())
        }
    }
    /// A world is opening: its missing saved biomes are reported again, once each.
    pub fn forget_missing_warnings() {
        WARNED_MISSING
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
    }
    /// The name a chunk saves for biome `id`: a placeholder's saved name, else `name`'s.
    pub fn saved_name(&self, id: u16) -> Cow<'_, str> {
        let slot = usize::from(id).checked_sub(self.list.len());
        let placeholders = self.placeholders.lock().unwrap_or_else(|e| e.into_inner());
        match slot.and_then(|s| placeholders.get(s)) {
            Some(name) => Cow::Owned(name.clone()),
            None => Cow::Borrowed(self.name(id)),
        }
    }
    /// The name of biome `id`, plains (as for an unknown name) when the id is
    /// past this table, as for `get`: a placeholder, or a stale server column.
    pub fn name(&self, id: u16) -> &str {
        self.list
            .get(usize::from(id))
            .map_or("minecraft:plains", |b| b.name.as_str())
    }
    /// A biome's natural spawners by category, sorted by category name; None for a name this
    /// table lacks.
    pub fn spawners(&self, name: &str) -> Option<Vec<mod_api::world::biome::BiomeSpawners>> {
        let info = self.list.get(usize::from(*self.index.get(name)?))?;
        let mut categories: Vec<_> = info
            .spawns
            .iter()
            .map(|(category, entries)| mod_api::world::biome::BiomeSpawners {
                category: category.clone(),
                entries: entries.clone(),
            })
            .collect();
        categories.sort_by(|a, b| a.category.cmp(&b.category));
        Some(categories)
    }
    /// Biome colours, with omitted water fog defaulted to 0x050533; not dimension-resolved.
    /// Returns None for a name this table lacks.
    pub fn effects(&self, name: &str) -> Option<mod_api::world::biome::BiomeEffects> {
        let info = self.list.get(usize::from(*self.index.get(name)?))?;
        Some(mod_api::world::biome::BiomeEffects {
            sky: info.sky,
            fog: info.fog,
            water: info.water,
            water_fog: info.water_fog,
        })
    }
    /// A biome's `Biome.ClimateSettings` by name; None for a name this table lacks.
    pub fn properties(&self, name: &str) -> Option<mod_api::BiomeProperties> {
        let info = self.list.get(usize::from(*self.index.get(name)?))?;
        Some(mod_api::BiomeProperties {
            temperature: info.temperature,
            downfall: info.downfall,
            has_precipitation: info.precipitation,
            frozen: info.frozen,
        })
    }
    pub fn grass(&self, id: u16, x: f64, z: f64) -> u32 {
        let b = self.get(id);
        match b.modifier {
            GrassModifier::None => b.grass,
            GrassModifier::DarkForest => ((b.grass & 0xFE_FEFE) + 0x28_340A) >> 1,
            GrassModifier::Swamp => {
                if f64::from(self.info_noise.get2(x * 0.0225, z * 0.0225)) < -0.1 {
                    0x4C_763C
                } else {
                    0x6A_7039
                }
            }
        }
    }
}

impl Biomes {
    /// This table with each biome's colours passed through `f`, given the
    /// biome's name (`minecraft:plains`) and its colours; None when `f`
    /// changed none. Bits past 0xFFFFFF are dropped.
    pub fn with_colors(&self, mut f: impl FnMut(&str, &mut BiomeColors)) -> Option<Self> {
        let mut out = self.clone();
        let mut changed = false;
        for b in &mut out.list {
            let before = BiomeColors {
                grass: b.grass,
                foliage: b.foliage,
                dry_foliage: b.dry_foliage,
                water: b.water,
            };
            let mut colors = before;
            f(&b.name, &mut colors);
            let after = BiomeColors {
                grass: colors.grass & 0xFF_FFFF,
                foliage: colors.foliage & 0xFF_FFFF,
                dry_foliage: colors.dry_foliage & 0xFF_FFFF,
                water: colors.water & 0xFF_FFFF,
            };
            if after != before {
                changed = true;
                b.grass = after.grass;
                b.foliage = after.foliage;
                b.dry_foliage = after.dry_foliage;
                b.water = after.water;
            }
        }
        changed.then_some(out)
    }

    /// This table (the biome files') with its colours passed through `f`, when
    /// they differ from `current`'s: what a refresh of a table already
    /// coloured by `f` once must not compound.
    pub fn recolored_from(
        &self,
        current: &Self,
        f: impl FnMut(&str, &mut BiomeColors),
    ) -> Option<Self> {
        let out = self.with_colors(f).unwrap_or_else(|| self.clone());
        let tints = |b: &BiomeInfo| (b.grass, b.foliage, b.dry_foliage, b.water);
        let same = out.list.len() == current.list.len()
            && out
                .list
                .iter()
                .zip(&current.list)
                .all(|(a, b)| tints(a) == tints(b));
        (!same).then_some(out)
    }
}

/// A 24-bit colour as sRGB components in 0..1.
pub fn rgb(c: u32) -> [f32; 3] {
    [16, 8, 0].map(|shift| (c >> shift & 255) as f32 / 255.0)
}

/// `GaussianSampler.GAUSSIAN_SAMPLE_KERNEL`.
const GAUSSIAN_KERNEL: [f64; 7] = [0.0, 1.0, 4.0, 6.0, 4.0, 1.0, 0.0];

/// `EnvironmentAttributeProbe.tick`: `GaussianSampler.sample` over the noise
/// biomes (quart cells) around `position`, in blocks. Each biome comes once
/// with its summed weight, in the order it was first sampled, as
/// `SpatialAttributeInterpolator` keeps them (zero weights included).
pub fn gaussian_weights(
    position: [f64; 3],
    mut biome_at_quart: impl FnMut(i32, i32, i32) -> u16,
) -> Vec<(u16, f64)> {
    let p = position.map(|c| c * 0.25 - 0.5);
    let integral = p.map(|c| c.floor() as i32);
    let relative: [f64; 3] = std::array::from_fn(|i| p[i] - f64::from(integral[i]));
    let weight = |axis: usize, i: usize| {
        let (start, end) = (GAUSSIAN_KERNEL[i + 1], GAUSSIAN_KERNEL[i]);
        start + relative[axis] * (end - start)
    };
    let mut weights: Vec<(u16, f64)> = Vec::new();
    for z in 0..6 {
        let wz = weight(2, z);
        for x in 0..6 {
            let wx = weight(0, x);
            for y in 0..6 {
                let wy = weight(1, y);
                let biome = biome_at_quart(
                    integral[0] - 2 + x as i32,
                    integral[1] - 2 + y as i32,
                    integral[2] - 2 + z as i32,
                );
                let w = wx * wy * wz;
                match weights.iter_mut().find(|(b, _)| *b == biome) {
                    Some((_, total)) => *total += w,
                    None => weights.push((biome, w)),
                }
            }
        }
    }
    weights
}

/// `SpatialAttributeInterpolator.applyAttributeLayer` for an RGB colour
/// (`ARGB.srgbLerp`: a component-wise lerp of the sRGB values).
pub fn blend_rgb(weights: &[(u16, f64)], mut value: impl FnMut(u16) -> [f32; 3]) -> [f32; 3] {
    let mut result: Option<[f32; 3]> = None;
    let mut accumulated = 0.0;
    for &(biome, weight) in weights {
        let v = value(biome);
        accumulated += weight;
        result = Some(match result {
            None => v,
            Some(r) => {
                let t = (weight / accumulated) as f32;
                std::array::from_fn(|i| r[i] + (v[i] - r[i]) * t)
            }
        });
    }
    result.unwrap_or([0.0; 3])
}

#[cfg(test)]
mod pack_tests {
    use super::*;
    use serde_json::json;

    /// A built-in table of `names` (no colormaps: sampled colours fall back).
    fn builtin(names: &[&str]) -> Biomes {
        let maps = Colormaps {
            grass: Vec::new(),
            foliage: Vec::new(),
            dry: Vec::new(),
        };
        let list: Vec<BiomeInfo> = names
            .iter()
            .map(|n| BiomeInfo::from_json((*n).to_owned(), &json!({}), &maps))
            .collect();
        Biomes {
            builtin: Arc::new(list.clone()),
            index: list
                .iter()
                .enumerate()
                .map(|(i, b)| (b.name.clone(), i as u16))
                .collect(),
            list,
            info_noise: SimplexNoise::with_skipped_offsets(&mut LegacyRandom::new(2345), true),
            placeholders: Default::default(),
            exhausted: Default::default(),
            colormaps: Arc::new(maps),
            packs: BTreeMap::new(),
            files: Arc::default(),
        }
    }

    #[test]
    fn registry_order_is_path_then_namespace() {
        use std::cmp::Ordering::*;
        assert_eq!(identifier_order("minecraft:a", "minecraft:b"), Less);
        assert_eq!(identifier_order("z:a", "a:b"), Less);
        assert_eq!(identifier_order("b:same", "a:same"), Greater);
        assert_eq!(identifier_order("minecraft:a", "minecraft:a_b"), Less);
        assert_eq!(identifier_order("minecraft:a-b", "minecraft:a"), Greater);
    }

    #[test]
    fn packs_replace_in_place_and_add_after_the_builtin_biomes() {
        let base = builtin(&["minecraft:meadow", "minecraft:plains", "minecraft:river"]);
        let red = json!({"temperature": 2.0, "effects": {"water_color": "#ff0000"},
            "attributes": {"minecraft:visual/ambient_particles": {
                "argument": [{"particle": {"type": "minecraft:ash"}, "probability": 0.25}]}}});
        let hot = json!({"effects": {"grass_color": 0x123456}});
        let files = [
            ("pack:zeta", &hot),
            ("minecraft:molten_peaks", &hot),
            ("minecraft:river", &red),
        ];
        let world = base.with_packs(files);
        // The built-in ids stay; the river takes the pack's values.
        assert_eq!(world.id("minecraft:plains"), 1);
        assert_eq!(world.id("minecraft:river"), 2);
        assert_eq!(world.get(2).water, 0xFF0000);
        assert_eq!(world.get(2).temperature, 2.0);
        assert_eq!(world.get(2).ambient_particles.len(), 1);
        assert_eq!(world.get(2).ambient_particles[0].0.name(), "ash");
        assert_eq!(world.get(2).ambient_particles[0].1, 0.25);
        // New biomes follow, in registry order.
        assert_eq!(world.id("minecraft:molten_peaks"), 3);
        assert_eq!(world.id("pack:zeta"), 4);
        assert_eq!(world.get(3).grass, 0x123456);
        assert!(world.pack_file("minecraft:river").is_some());
        assert!(world.pack_file("minecraft:plains").is_none());
        // The network registry sorts every name by path, then namespace.
        assert_eq!(
            world.registry_order(),
            [
                "minecraft:meadow",
                "minecraft:molten_peaks",
                "minecraft:plains",
                "minecraft:river",
                "pack:zeta"
            ]
        );
        // Another world starts again from the built-in biomes.
        let other = world.with_packs([]);
        assert_eq!(other.list.len(), 3);
        assert!(!other.contains("minecraft:molten_peaks"));
        assert_eq!(other.get(2).water, 0x3F76E4);
        assert!(!other.has_packs());
    }

    /// A column keeps the ids of the table it was decoded with. When a
    /// proxy's next server lacks a biome the last one added, that id is past
    /// the next table, and it names plains instead of indexing past the end
    /// (#12148: "the len is 67 but the index is 67").
    #[test]
    fn an_id_past_the_table_names_plains() {
        let next = builtin(&["minecraft:meadow", "minecraft:plains", "minecraft:river"]);
        let last = next.with_packs([("limbo:void", &json!({}))]);
        let stale = last.id("limbo:void");
        assert_eq!((stale, last.name(stale)), (3, "limbo:void"));
        assert_eq!(next.name(stale), "minecraft:plains");
        assert_eq!(next.name(u16::MAX), "minecraft:plains");
        assert_eq!(next.name(2), "minecraft:river");
    }
}

#[cfg(test)]
impl Biomes {
    /// Biomes from their files' JSON, by name, in id order: for tests elsewhere.
    pub(crate) fn from_jsons(entries: &[(&str, Value)]) -> Self {
        let maps = Colormaps {
            grass: Vec::new(),
            foliage: Vec::new(),
            dry: Vec::new(),
        };
        let list: Vec<_> = entries
            .iter()
            .map(|(name, v)| BiomeInfo::from_json((*name).to_owned(), v, &maps))
            .collect();
        Self {
            builtin: Arc::new(list.clone()),
            index: list
                .iter()
                .enumerate()
                .map(|(i, b)| (b.name.clone(), i as u16))
                .collect(),
            list,
            info_noise: SimplexNoise::with_skipped_offsets(&mut LegacyRandom::new(2345), true),
            placeholders: Default::default(),
            exhausted: Default::default(),
            colormaps: Arc::new(maps),
            packs: BTreeMap::new(),
            files: Arc::new(
                entries
                    .iter()
                    .map(|(n, v)| ((*n).to_owned(), v.clone()))
                    .collect(),
            ),
        }
    }
}

#[cfg(test)]
mod color_tests {
    use super::*;
    use serde_json::json;

    fn biomes() -> Biomes {
        let maps = Colormaps {
            grass: Vec::new(),
            foliage: Vec::new(),
            dry: Vec::new(),
        };
        let effects = |grass: u32, modifier: &str| {
            json!({"effects": {"grass_color": grass, "foliage_color": 0x112233,
                "dry_foliage_color": 0x445566, "water_color": 0x3F76E4,
                "grass_color_modifier": modifier}})
        };
        let list = vec![
            BiomeInfo::from_json("minecraft:plains".into(), &effects(0x91BD59, "none"), &maps),
            BiomeInfo::from_json(
                "minecraft:dark_forest".into(),
                &effects(0x91BD59, "dark_forest"),
                &maps,
            ),
        ];
        Biomes {
            builtin: Arc::new(list.clone()),
            index: list
                .iter()
                .enumerate()
                .map(|(i, b)| (b.name.clone(), i as u16))
                .collect(),
            list,
            info_noise: SimplexNoise::with_skipped_offsets(&mut LegacyRandom::new(2345), true),
            placeholders: Default::default(),
            exhausted: Default::default(),
            colormaps: Arc::new(maps),
            packs: BTreeMap::new(),
            files: Arc::default(),
        }
    }

    #[test]
    fn a_change_reaches_the_tints_the_mesher_reads() {
        let autumn = biomes()
            .with_colors(|name, c| {
                c.foliage = 0xD2691E;
                if name == "minecraft:plains" {
                    c.grass = 0xBFA055;
                }
            })
            .expect("changed");
        let plains = autumn.id("minecraft:plains");
        assert_eq!(autumn.grass(plains, 0.0, 0.0), 0xBFA055);
        assert_eq!(autumn.get(plains).foliage, 0xD2691E);
        assert_eq!(autumn.get(plains).dry_foliage, 0x445566);
        assert_eq!(autumn.get(plains).water, 0x3F76E4);
        // The dark forest's modifier still darkens its (unchanged) grass:
        // ((0x91BD59 & 0xFEFEFE) + 0x28340A) >> 1.
        let dark = autumn.id("minecraft:dark_forest");
        assert_eq!(autumn.grass(dark, 0.0, 0.0), 0x5C7831);
        assert_eq!(autumn.get(dark).foliage, 0xD2691E);
    }

    #[test]
    fn no_change_gives_none_and_alpha_bits_are_dropped() {
        assert!(biomes().with_colors(|_, _| {}).is_none());
        assert!(
            biomes()
                .with_colors(|_, c| c.water |= 0xFF00_0000)
                .is_none()
        );
        let blue = biomes()
            .with_colors(|_, c| c.water = 0xFF00_00FF)
            .expect("changed");
        assert_eq!(blue.get(0).water, 0x0000FF);
    }
}

#[cfg(test)]
mod blend_tests {
    use super::*;

    #[test]
    fn one_biome_keeps_its_colour() {
        let weights = gaussian_weights([10.3, 64.0, -7.9], |_, _, _| 4);
        assert_eq!(weights.len(), 1);
        // The kernel sums to 16 per axis whatever the offset.
        assert!((weights[0].1 - 16f64.powi(3)).abs() < 1e-9);
        assert_eq!(blend_rgb(&weights, |_| [0.2, 0.4, 0.6]), [0.2, 0.4, 0.6]);
    }

    #[test]
    fn colour_changes_gradually_across_a_boundary() {
        // Biome 1 for quart x >= 0 (block x >= 0), biome 0 below.
        let at = |x: f64| {
            let w = gaussian_weights([x, 40.0, 0.0], |qx, _, _| u16::from(qx >= 0));
            blend_rgb(&w, |b| if b == 1 { [1.0, 0.0, 0.0] } else { [0.0; 3] })[0]
        };
        assert_eq!(at(-12.0), 0.0);
        assert_eq!(at(12.0), 1.0);
        // Halfway at the boundary, rising steadily through it.
        assert!((at(0.0) - 0.5).abs() < 1e-5, "{}", at(0.0));
        let mut last = 0.0;
        for i in -16..=16 {
            let v = at(f64::from(i) * 0.5);
            assert!(v >= last - 1e-6, "{v} < {last} at {i}");
            last = v;
        }
        assert!(at(-2.0) > 0.0 && at(-2.0) < 0.5);
    }

    /// An empty biome table (a resource pack with no biome files) must not
    /// panic on `get`: before the fix, `list.len() - 1` underflowed.
    #[test]
    fn get_on_empty_biome_table_returns_default_instead_of_panicking() {
        use std::sync::Arc;

        let empty = Biomes {
            builtin: Arc::new(Vec::new()),
            files: Arc::default(),
            index: HashMap::new(),
            list: Vec::new(),
            info_noise: SimplexNoise::with_skipped_offsets(&mut LegacyRandom::new(2345), true),
            colormaps: Arc::new(Colormaps {
                grass: Vec::new(),
                foliage: Vec::new(),
                dry: Vec::new(),
            }),
            packs: BTreeMap::new(),
            placeholders: Default::default(),
            exhausted: Default::default(),
        };
        // Any id returns the shared default plains biome, no panic.
        let b = empty.get(0);
        assert_eq!(b.name, "minecraft:plains");
        let b = empty.get(65535);
        assert_eq!(b.name, "minecraft:plains");
        // Colours and temperature come from the default.
        assert_eq!(b.temperature, 0.8);
        assert_eq!(b.grass, 0x7f_c0_55);
        assert_eq!(b.foliage, 0x6a_ab_37);
        assert_eq!(b.water, 0x3f_76_e4);
    }
}

#[cfg(test)]
mod source_tests {
    use super::*;
    #[test]
    fn colormap_pixels_read_from_resolved_resources_without_files() {
        use horizon_assets::resource_source::Resource;
        let mut png = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut png, 1, 1);
            encoder.set_color(png::ColorType::Rgb);
            encoder.set_depth(png::BitDepth::Eight);
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&[12, 34, 56])
                .unwrap();
        }
        let source = ResourceSource::indexed(std::collections::BTreeMap::from([(
            "assets/minecraft/textures/colormap/grass.png".into(),
            Resource::bytes(png),
        )]));
        assert_eq!(colormap(&source, "grass").unwrap(), [0x0c2238]);
        assert!(colormap(&source, "missing").is_err());
    }
}

#[cfg(test)]
mod read_tests {
    use super::*;
    use mod_api::world::biome::{BiomeEffects, SpawnEntry};
    use serde_json::json;

    fn table() -> Biomes {
        Biomes::from_jsons(&[
            (
                "minecraft:plains",
                json!({
                    "effects": {"water_color": 0x3F76E4},
                    "attributes": {
                        "minecraft:gameplay/natural_mob_spawns": {"spawns_by_category": {
                            "monster": [
                                {"type": "minecraft:zombie", "weight": 95, "count": {"min_inclusive": 4, "max_inclusive": 4}},
                                {"type": "minecraft:spider", "weight": 100, "count": 2}
                            ],
                            "creature": [{"type": "minecraft:horse", "weight": 5,
                                "count": {"min_inclusive": 2, "max_inclusive": 6}}]
                        }}
                    }
                }),
            ),
            (
                "minecraft:the_void",
                json!({
                    "effects": {"water_color": 0x123456},
                    "attributes": {
                        "minecraft:visual/sky_color": "#78A7FF",
                        "minecraft:visual/fog_color": 0xC0D8FF,
                        "minecraft:visual/water_fog_color": 0x123456
                    }
                }),
            ),
        ])
    }

    #[test]
    fn spawners_are_the_biome_files_by_category_name() {
        let spawners = table().spawners("minecraft:plains").unwrap();
        let entry = |entity: &str, weight, min, max, constant| SpawnEntry {
            entity: entity.into(),
            weight,
            min,
            max,
            constant,
        };
        assert_eq!(
            spawners
                .iter()
                .map(|s| s.category.as_str())
                .collect::<Vec<_>>(),
            ["creature", "monster"]
        );
        assert_eq!(
            spawners[0].entries,
            [entry("minecraft:horse", 5, 2, 6, false)]
        );
        assert_eq!(
            spawners[1].entries,
            [
                entry("minecraft:zombie", 95, 4, 4, false),
                entry("minecraft:spider", 100, 2, 2, true)
            ]
        );
        // A biome with no natural spawns has an empty list; an unknown one has none.
        assert_eq!(table().spawners("minecraft:the_void"), Some(Vec::new()));
        assert_eq!(table().spawners("minecraft:nowhere"), None);
    }

    #[test]
    fn effects_are_the_biome_files_own_colours() {
        assert_eq!(
            table().effects("minecraft:the_void"),
            Some(BiomeEffects {
                sky: Some(0x78A7FF),
                fog: Some(0xC0D8FF),
                water: 0x123456,
                water_fog: 0x123456,
            })
        );
        // A biome that leaves sky and fog out has none, and the default water fog.
        assert_eq!(
            table().effects("minecraft:plains"),
            Some(BiomeEffects {
                sky: None,
                fog: None,
                water: 0x3F76E4,
                water_fog: 0x050533,
            })
        );
        assert_eq!(table().effects("minecraft:nowhere"), None);
    }
}

#[cfg(test)]
mod climate_tests {
    use super::*;
    use mod_api::BiomeProperties;
    use serde_json::json;

    /// Climates as Mojang's biome files declare them, so a dropped or swapped field fails.
    #[test]
    fn properties_are_the_biome_files_climate() {
        let biomes = Biomes::from_jsons(&[
            (
                "minecraft:plains",
                json!({"temperature": 0.8, "downfall": 0.4, "has_precipitation": true}),
            ),
            (
                "minecraft:desert",
                json!({"temperature": 2.0, "downfall": 0.0, "has_precipitation": false}),
            ),
            (
                "minecraft:snowy_plains",
                json!({"temperature": 0.0, "downfall": 0.5, "has_precipitation": true}),
            ),
            (
                "minecraft:frozen_ocean",
                json!({"temperature": 0.0, "downfall": 0.5, "has_precipitation": true,
                    "temperature_modifier": "frozen"}),
            ),
            (
                "minecraft:ocean",
                json!({"temperature": 0.5, "downfall": 0.5, "has_precipitation": true}),
            ),
        ]);
        let climate = |name: &str| biomes.properties(name).unwrap();
        let want = |temperature, downfall, has_precipitation, frozen| BiomeProperties {
            temperature,
            downfall,
            has_precipitation,
            frozen,
        };
        assert_eq!(climate("minecraft:plains"), want(0.8, 0.4, true, false));
        assert_eq!(climate("minecraft:desert"), want(2.0, 0.0, false, false));
        assert_eq!(
            climate("minecraft:snowy_plains"),
            want(0.0, 0.5, true, false)
        );
        assert_eq!(climate("minecraft:ocean"), want(0.5, 0.5, true, false));
        // The only biome that catches a dropped `frozen`.
        assert_eq!(
            climate("minecraft:frozen_ocean"),
            want(0.0, 0.5, true, true)
        );
        assert_eq!(biomes.properties("minecraft:nowhere"), None);
        assert_eq!(biomes.properties(""), None);
    }
}

#[cfg(test)]
mod placeholder_tests {
    use super::*;
    use serde_json::json;

    fn table() -> Biomes {
        Biomes::from_jsons(&[
            ("minecraft:forest", json!({"effects": {"water_color": 1}})),
            ("minecraft:plains", json!({"effects": {"water_color": 2}})),
        ])
    }

    #[test]
    fn an_existing_placeholder_warns_again_in_the_next_world() {
        let biomes = table();
        let name = "gone_mod:warning_reset";
        let id = biomes.saved_id(name);
        // Simulate the next world's cleared warning set for this name only.
        assert!(WARNED_MISSING.lock().unwrap().remove(name));
        assert_eq!(biomes.saved_id(name), id);
        assert!(WARNED_MISSING.lock().unwrap().contains(name));
    }

    #[test]
    fn a_missing_saved_biome_plays_as_plains_and_keeps_its_name() {
        let biomes = table();
        assert_eq!(biomes.saved_id("minecraft:forest"), 0);
        let gone = biomes.saved_id("gone_mod:highland");
        let other = biomes.saved_id("gone_mod:lowland");
        assert_ne!(gone, other);
        assert!(usize::from(gone) >= biomes.list.len());
        assert_eq!(
            biomes.saved_id("gone_mod:highland"),
            gone,
            "one id per name"
        );
        assert_eq!(biomes.name(gone), "minecraft:plains");
        assert_eq!(biomes.get(gone).name, "minecraft:plains");
        assert_eq!(biomes.get(gone).water, 2);
        assert_eq!(biomes.saved_name(gone), "gone_mod:highland");
        assert_eq!(biomes.saved_name(other), "gone_mod:lowland");
        assert_eq!(biomes.saved_name(0), "minecraft:forest");
        // A recoloured copy keeps the ids; a table rebuilt from packs does not share them.
        let recolored = biomes.with_colors(|_, c| c.water = 3).unwrap();
        assert_eq!(recolored.saved_name(gone), "gone_mod:highland");
        let rebuilt = biomes.with_packs([]);
        assert_eq!(rebuilt.saved_name(gone), "minecraft:plains");
    }
}

/// The resident biome identifier space cannot retain another saved name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BiomeIdExhausted;
impl std::fmt::Display for BiomeIdExhausted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "biome ID limit of 65536 exhausted; refusing destructive save"
        )
    }
}
impl std::error::Error for BiomeIdExhausted {}

#[cfg(test)]
mod exhaustion_tests {
    use super::*;
    #[test]
    fn last_id_is_retained_and_exhaustion_refuses_saving() {
        let biomes = Biomes::from_jsons(&[]);
        *biomes.placeholders.lock().unwrap() =
            (0..65535).map(|i| format!("missing:biome_{i}")).collect();
        assert_eq!(biomes.saved_id("missing:last"), 65535);
        assert_eq!(biomes.saved_name(65535), "missing:last");
        assert!(biomes.ensure_saveable().is_ok());
        biomes.saved_id("missing:overflow");
        assert_eq!(biomes.placeholders.lock().unwrap().len(), 65536);
        assert_eq!(biomes.ensure_saveable(), Err(BiomeIdExhausted));
        assert!(BiomeIdExhausted.to_string().contains("65536"));
        assert_eq!(biomes.saved_name(65535), "missing:last");
    }
}
