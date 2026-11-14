//! Data-driven enchantments (`data/minecraft/enchantment/*.json`): the
//! registry, ordered enchantment tags, `EnchantmentHelper` selection and cost
//! rules used by the enchanting table, display names, and evaluation of the
//! value/attribute effect components (`LevelBasedValue`, `EnchantmentValueEffect`
//! and the loot-condition subset that enchantment requirements use).
mod holder_sets;
use crate::item_data::Levels;
use crate::items::{ItemStack, Items};
pub use holder_sets::HolderSetBinding;
use horizon_core::nbt::Tag;
use horizon_core::random::{LegacyRandom, RandomSource};
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

impl Items {
    /// Vanilla's enchantment-provider tag gives 1.0; non-members give 0.0.
    /// Parts may override this answer; tag membership is not a separate particle-provider test.
    pub fn default_enchant_power(&self, block: &str) -> f32 {
        if self.block_in_tag("minecraft:enchantment_power_provider", block) {
            1.0
        } else {
            0.0
        }
    }
}

/// Load-owned custom damage effect slots. Installed before resources load.
static DAMAGE_EFFECT_IDS: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();

/// Publish registered identifiers before enchantment content is read.
pub fn register_damage_effects(ids: Vec<String>) {
    assert!(
        DAMAGE_EFFECT_IDS.set(ids).is_ok(),
        "damage effects already registered"
    );
}

/// A value node compiled at content load. Slots never enter content JSON or network data.
#[derive(Clone)]
enum DamageValueEffect {
    Builtin(Value),
    AllOf(Vec<Self>),
    Custom {
        slot: usize,
        config: horizon_core::nbt::Tag,
    },
}
impl DamageValueEffect {
    fn resolve(effect: &Value, ids: &[String]) -> Result<Self, String> {
        match effect["type"].as_str().unwrap_or("") {
            "minecraft:all_of" => effect["effects"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|effect| Self::resolve(effect, ids))
                .collect::<Result<Vec<_>, _>>()
                .map(Self::AllOf),
            kind if kind.is_empty() || !kind.contains(':') || kind.starts_with("minecraft:") => {
                Ok(Self::Builtin(effect.clone()))
            }
            kind => ids
                .iter()
                .position(|id| id == kind)
                .ok_or_else(|| format!("unknown enchantment damage value effect {kind}"))
                .and_then(|slot| {
                    horizon_core::json_nbt::json_to_tag(effect)
                        .map(|config| Self::Custom { slot, config })
                        .map_err(|error| error.to_string())
                }),
        }
    }
    fn is_custom(&self) -> bool {
        match self {
            Self::Custom { .. } => true,
            Self::AllOf(nodes) => nodes.iter().any(Self::is_custom),
            Self::Builtin(_) => false,
        }
    }
    fn apply(
        &self,
        level: u32,
        random: &mut LegacyRandom,
        value: f32,
        custom: &mut impl FnMut(usize, &horizon_core::nbt::Tag, u32, f32) -> f32,
    ) -> f32 {
        match self {
            Self::Builtin(effect) => apply_value_effect(effect, level, random, value),
            Self::AllOf(nodes) => nodes.iter().fold(value, |value, node| {
                node.apply(level, random, value, custom)
            }),
            Self::Custom { slot, config } => {
                let result = custom(*slot, config, level, value);
                // Bound mod output before a subsequent per-unit built-in effect.
                if result.is_nan() {
                    value
                } else {
                    result.clamp(0.0, 1024.0)
                }
            }
        }
    }
}

fn resolve_damage_effects(
    effects: &Value,
    ids: &[String],
) -> Result<Option<Vec<DamageValueEffect>>, String> {
    let nodes = effects["minecraft:damage"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|entry| DamageValueEffect::resolve(&entry["effect"], ids))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(nodes
        .iter()
        .any(DamageValueEffect::is_custom)
        .then_some(nodes))
}

/// Levels as numbers in enchantment names ("Sharpness 5"): Horizon's PvP & HUD option, off by
/// default (vanilla's numerals).
static NUMERIC_LEVELS: AtomicBool = AtomicBool::new(false);
pub fn set_numeric_levels(on: bool) {
    NUMERIC_LEVELS.store(on, Ordering::Relaxed);
}

/// `Enchantment.Cost`: base + per_level_above_first * (level - 1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cost {
    pub base: i32,
    pub per_level: i32,
}
impl Cost {
    fn parse(v: &Value) -> Self {
        Self {
            base: v["base"].as_i64().unwrap_or(1) as i32,
            per_level: v["per_level_above_first"].as_i64().unwrap_or(0) as i32,
        }
    }
    pub fn calculate(self, level: u32) -> i32 {
        self.base + self.per_level * (level as i32 - 1)
    }
}
#[derive(Clone, Debug)]
pub struct Enchantment {
    pub id: Arc<str>,
    /// The name the pinned English map gave at load: the fallback for a
    /// language that has no entry for the key below.
    pub description: String,
    /// `description`'s translation key (`enchantment.minecraft.<name>`).
    pub description_key: String,
    pub max_level: u32,
    pub weight: u32,
    pub anvil_cost: u32,
    pub min_cost: Cost,
    pub max_cost: Cost,
    /// `supported_items` / `primary_items`: item ids or `#tags`.
    pub supported: Vec<String>,
    pub primary: Option<Vec<String>>,
    pub exclusive: HashSet<Arc<str>>,
    pub slots: Vec<String>,
    pub effects: Value,
    /// Mod-owned post-attack payloads, converted once when the definition loads.
    pub native_post_attack: HashMap<usize, Arc<horizon_core::nbt::Tag>>,
}
/// Item components the enchanting, anvil and smithing rules read.
#[derive(Clone, Debug, Default)]
pub struct ItemProps {
    /// `minecraft:enchantable.value`.
    pub enchantable: u32,
    /// `minecraft:repairable.items` (ids or a `#tag`).
    pub repairable: Vec<String>,
    /// The prototype override; absence differs from an explicit false.
    pub glint: Option<bool>,
    pub rarity: String,
    /// `minecraft:provides_trim_material`.
    pub trim_material: Option<String>,
    /// `minecraft:brewing_fuel` present.
    pub brewing_fuel: bool,
}
#[derive(Clone, Default)]
pub struct Enchantments {
    pub list: Vec<Enchantment>,
    index: HashMap<Arc<str>, usize>,
    custom_damage: HashMap<Arc<str>, Vec<DamageValueEffect>>,
    /// Enchantment tags in vanilla `HolderSet` order (nested tags expanded in place).
    pub tags: HashMap<String, Vec<Arc<str>>>,
    pub entity_tags: HashMap<String, HashSet<String>>,
    pub damage_tags: HashMap<String, HashSet<String>>,
    pub props: HashMap<Arc<str>, ItemProps>,
    lang: HashMap<String, String>,
}

fn string_list(v: &Value) -> Vec<String> {
    match v {
        Value::String(s) => vec![s.clone()],
        Value::Array(a) => a
            .iter()
            .filter_map(|e| e.as_str().map(str::to_owned))
            .collect(),
        _ => vec![],
    }
}
fn read_tag_dir(dir: &Path) -> HashMap<String, horizon_world::tag_layers::TagLayers> {
    read_tag_dir_in(dir, "minecraft")
}
/// The resource root's `data/` namespaces other than `minecraft`, in name
/// order: content mods' files land there (`horizon_client::mod_content`'s
/// overlay), as a data pack's namespaces do in `RegistryDataLoader`.
fn other_namespaces(root: &Path) -> Vec<String> {
    let mut out = horizon_world::json_folder::namespaces(&root.join("data"));
    out.retain(|ns| ns != "minecraft");
    out
}
fn read_tag_dir_in(
    dir: &Path,
    namespace: &str,
) -> HashMap<String, horizon_world::tag_layers::TagLayers> {
    let Ok(folder) = horizon_world::json_folder::read(dir) else {
        return HashMap::new();
    };
    folder
        .files
        .iter()
        .filter_map(|file| {
            let v = serde_json::from_slice::<Value>(file.bytes().ok()?).ok()?;
            Some((
                format!("{namespace}:{}", file.name),
                horizon_world::tag_layers::TagLayers::from_file(&v),
            ))
        })
        .collect()
}
/// Expand a tag keeping first-seen order and applying removals before its parent uses it.
fn expand_ordered(
    name: &str,
    raw: &HashMap<String, horizon_world::tag_layers::TagLayers>,
    out: &mut Vec<String>,
    depth: u32,
) {
    if depth > 16 {
        return;
    }
    if let Some(layers) = raw.get(name) {
        let members = layers.build(|tag| {
            let mut child = Vec::new();
            expand_ordered(tag, raw, &mut child, depth + 1);
            child
        });
        for member in members {
            if !out.contains(&member) {
                out.push(member);
            }
        }
    }
}
/// Roman numerals as in `enchantment.level.N` (falls back to the number).
pub fn level_name(level: u32) -> String {
    const NAMES: [&str; 10] = ["I", "II", "III", "IV", "V", "VI", "VII", "VIII", "IX", "X"];
    NAMES
        .get(level.wrapping_sub(1) as usize)
        .map_or_else(|| level.to_string(), |s| (*s).to_owned())
}

/// One registry entry from its definition (`Enchantment.DIRECT_CODEC`).
fn parse_entry(
    id: &str,
    v: &Value,
    tags: &HashMap<String, Vec<Arc<str>>>,
    lang: &HashMap<String, String>,
) -> crate::Result<Enchantment> {
    if let Some(field) = ["supported_items", "primary_items", "exclusive_set"]
        .into_iter()
        .find(|field| v[*field].is_object())
    {
        return Err(format!(
            "enchantment {id} {field}: holder-set object has no owning registry binding"
        )
        .into());
    }
    let native_post_attack = v["effects"]["minecraft:post_attack"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
        .filter(|(_, e)| {
            e["effect"]["type"].as_str().is_some_and(|k| {
                k.contains(':') && !k.starts_with("minecraft:") && !k.starts_with(':')
            })
        })
        .map(|(i, e)| {
            horizon_core::json_nbt::json_to_tag(&e["effect"]).map(|tag| (i, Arc::new(tag)))
        })
        .collect::<Result<HashMap<_, _>, _>>()
        .map_err(|error| format!("{id}: {error}"))?;
    let exclusive = match &v["exclusive_set"] {
        Value::String(s) if s.starts_with('#') => tags.get(&s[1..]).cloned().unwrap_or_default(),
        other => string_list(other)
            .into_iter()
            .map(|s| Arc::from(s.as_str()))
            .collect(),
    };
    let key = v["description"]["translate"].as_str().unwrap_or("");
    Ok(Enchantment {
        native_post_attack,
        id: Arc::from(id),
        description: lang.get(key).cloned().unwrap_or_else(|| {
            // A data pack's entry may name itself with a literal component
            // rather than a translation key.
            if key.is_empty() && !v["description"].is_null() {
                horizon_client_core::scoreboard::comp_text(&v["description"]).plain()
            } else {
                id.trim_start_matches("minecraft:").replace('_', " ")
            }
        }),
        description_key: key.to_owned(),
        max_level: v["max_level"].as_u64().unwrap_or(1) as u32,
        weight: v["weight"].as_u64().unwrap_or(1) as u32,
        anvil_cost: v["anvil_cost"].as_u64().unwrap_or(0) as u32,
        min_cost: Cost::parse(&v["min_cost"]),
        max_cost: Cost::parse(&v["max_cost"]),
        supported: string_list(&v["supported_items"]),
        primary: v.get("primary_items").map(string_list),
        exclusive: exclusive.into_iter().collect(),
        slots: string_list(&v["slots"]),
        effects: v["effects"].clone(),
    })
}

impl Enchantments {
    /// Use the same frozen registry and resolved tags as player damage.
    pub fn bind_damage_catalog(&mut self, catalog: &horizon_world::damage_source::Catalog) {
        self.damage_tags = catalog
            .tags()
            .map(|tag| {
                (
                    tag.to_owned(),
                    catalog.tag_members(tag).map(str::to_owned).collect(),
                )
            })
            .collect();
    }
    /// Refuse the load if a mod effect payload cannot be represented as NBT.
    /// Errors identify the enchantment; arbitrary JSON payloads had no prior refusal message.
    pub fn load(
        root: &Path,
        defaults: &horizon_world::item_defaults::ItemDefaults,
    ) -> crate::Result<Self> {
        Self::load_with(root, &horizon_world::loot::PackOverlay::NONE, defaults)
    }
    /// `RegistryDataLoader.load`: load from a resource root; `overlay` (world-pack content
    /// files) overrides definitions and joins enchantment tags in pack order, a
    /// `replace` file dropping the lower sources' (`TagLoader.load`).
    pub fn load_with(
        root: &Path,
        overlay: &horizon_world::loot::PackOverlay,
        defaults: &horizon_world::item_defaults::ItemDefaults,
    ) -> crate::Result<Self> {
        let data = root.join("data/minecraft");
        let mut raw_tags = read_tag_dir(&data.join("tags/enchantment"));
        let namespaces = other_namespaces(root);
        // Mods' own enchantment tags (`data/<ns>/tags/enchantment/`): an
        // `exclusive_set` or a tag of the mod's enchantments.
        for ns in &namespaces {
            let dir = root.join("data").join(ns).join("tags/enchantment");
            for (id, values) in read_tag_dir_in(&dir, ns) {
                raw_tags.insert(id, values);
            }
        }
        for (id, files) in overlay.tag_kind("enchantment") {
            for bytes in files {
                let Ok(v) = serde_json::from_slice::<Value>(bytes) else {
                    continue;
                };
                raw_tags.entry(id.clone()).or_default().push(&v);
            }
        }
        let tags: HashMap<String, Vec<Arc<str>>> = raw_tags
            .keys()
            .map(|k| {
                let mut v = Vec::new();
                expand_ordered(k, &raw_tags, &mut v, 0);
                (
                    k.clone(),
                    v.into_iter().map(|s| Arc::from(s.as_str())).collect(),
                )
            })
            .collect();
        let unordered = |raw: HashMap<String, horizon_world::tag_layers::TagLayers>| -> HashMap<String, HashSet<String>> {
            raw.keys()
                .map(|k| {
                    let mut v = Vec::new();
                    expand_ordered(k, &raw, &mut v, 0);
                    (k.clone(), v.into_iter().collect())
                })
                .collect()
        };
        let entity_tags = unordered(read_tag_dir(&data.join("tags/entity_type")));
        let damage_tags = match horizon_world::damage_source::Catalog::load(root, overlay) {
            Ok(catalog) => catalog
                .tags()
                .map(|tag| {
                    (
                        tag.to_owned(),
                        catalog.tag_members(tag).map(str::to_owned).collect(),
                    )
                })
                .collect(),
            // Small tools/fixtures may not carry the prepared catalog.
            Err(_) => unordered(read_tag_dir(&data.join("tags/damage_type"))),
        };
        // The registry lists entries sorted by id, like the data-pack loader.
        let mut files: BTreeMap<String, Value> =
            crate::items::data_files(root, "enchantment", overlay)
                .into_iter()
                .collect();
        if files.values().any(HolderSetBinding::needed) {
            let binding =
                HolderSetBinding::load(root, overlay, defaults, files.keys().cloned().collect())?;
            for (id, value) in &mut files {
                binding.bind(id, value)?;
            }
        }
        let lang: HashMap<String, String> =
            horizon_core::prepared_files::read(&root.join("assets/minecraft/lang/en_us.json"))
                .ok()
                .and_then(|b| serde_json::from_slice::<HashMap<String, String>>(&b).ok())
                .unwrap_or_default()
                .into_iter()
                .filter(|(k, _)| {
                    k.starts_with("enchantment.")
                        || k.starts_with("attribute.")
                        || k.starts_with("item.modifiers.")
                        || k.starts_with("effect.")
                        || k.starts_with("item.minecraft.potion")
                        || k.starts_with("item.minecraft.splash_potion")
                        || k.starts_with("item.minecraft.lingering_potion")
                        || k.starts_with("item.minecraft.tipped_arrow")
                        || k.starts_with("item.minecraft.firework_rocket.")
                        || k.starts_with("item.minecraft.firework_star.")
                        || k.starts_with("trim_")
                        || k.starts_with("potion.")
                        || k.starts_with("container.")
                        || k.starts_with("merchant.")
                        || k.starts_with("entity.minecraft.")
                        || k.starts_with("gui.recipebook.")
                        || k.starts_with("gui.togglable_slot")
                        || k.starts_with("block.minecraft.banner.")
                        || k.starts_with("recipe.toast.")
                })
                .collect();
        // Mods' English names for their enchantments (`assets/<ns>/lang/`),
        // under vanilla's, which win a clash.
        let mut lang = lang;
        for ns in &namespaces {
            let path = root.join("assets").join(ns).join("lang/en_us.json");
            let Some(map) = horizon_core::prepared_files::read(&path)
                .ok()
                .and_then(|b| serde_json::from_slice::<HashMap<String, String>>(&b).ok())
            else {
                continue;
            };
            for (k, v) in map {
                if k.starts_with("enchantment.") {
                    lang.entry(k).or_insert(v);
                }
            }
        }
        let mut custom_damage = HashMap::new();
        let list = files
            .iter()
            .filter_map(|(id, v)| {
                match resolve_damage_effects(
                    &v["effects"],
                    DAMAGE_EFFECT_IDS.get().map_or(&[], Vec::as_slice),
                ) {
                    Ok(Some(nodes)) => {
                        custom_damage.insert(Arc::from(id.as_str()), nodes);
                    }
                    Ok(None) => {}
                    Err(error) => {
                        eprintln!("enchantment {id}: {error}");
                        return None;
                    }
                }
                Some(parse_entry(id, v, &tags, &lang))
            })
            .collect::<crate::Result<Vec<_>>>()?;
        let index = list
            .iter()
            .enumerate()
            .map(|(i, e)| (e.id.clone(), i))
            .collect();
        let mut props = HashMap::new();
        for (id, c) in defaults.entries() {
            props.insert(
                Arc::from(id.as_str()),
                ItemProps {
                    enchantable: c["minecraft:enchantable"]["value"].as_u64().unwrap_or(0) as u32,
                    repairable: string_list(&c["minecraft:repairable"]["items"]),
                    glint: c["minecraft:enchantment_glint_override"].as_bool(),
                    rarity: c["minecraft:rarity"]
                        .as_str()
                        .unwrap_or("common")
                        .to_owned(),
                    trim_material: c["minecraft:provides_trim_material"]
                        .as_str()
                        .map(str::to_owned),
                    brewing_fuel: c.get("minecraft:brewing_fuel").is_some(),
                },
            );
        }
        Ok(Self {
            list,
            index,
            custom_damage,
            tags,
            entity_tags,
            damage_tags,
            props,
            lang,
        })
    }
    pub fn get(&self, id: &str) -> Option<&Enchantment> {
        self.index.get(id).map(|&i| &self.list[i])
    }
    /// The table a client plays a server with: this one and the server's
    /// entries it lacks (a data pack's), from the definitions the registry
    /// sync sent; `None` when there are none. The names then come from the
    /// entries' own `description`, as `Enchantment.getFullname` reads it.
    pub fn with_entries<'a>(
        &self,
        entries: impl IntoIterator<Item = (&'a str, &'a Value)>,
    ) -> crate::Result<Option<Self>> {
        let mut table = None::<Self>;
        for (id, v) in entries {
            if self.index.contains_key(id) {
                continue;
            }
            let nodes = match resolve_damage_effects(
                &v["effects"],
                DAMAGE_EFFECT_IDS.get().map_or(&[], Vec::as_slice),
            ) {
                Ok(nodes) => nodes,
                Err(error) => {
                    eprintln!("synced enchantment {id}: {error}; retaining display definition");
                    None
                }
            };
            let t = table.get_or_insert_with(|| self.clone());
            let e = parse_entry(id, v, &t.tags, &t.lang)?;
            if let Some(nodes) = nodes {
                t.custom_damage.insert(e.id.clone(), nodes);
            }
            t.index.insert(e.id.clone(), t.list.len());
            t.list.push(e);
        }
        Ok(table)
    }
    /// The longest `minecraft:ignite` the attacker's `minecraft:post_attack`
    /// effects (Fire Aspect) set on the victim, in seconds: the effects whose
    /// requirements hold for `ctx` (0 when none).
    pub fn ignite_seconds(
        &self,
        enchants: &[(Arc<str>, u32)],
        ctx: &EffectContext<'_>,
        random: &mut LegacyRandom,
    ) -> f32 {
        let mut seconds = 0.0f32;
        for (id, level) in enchants {
            let Some(def) = self.get(id) else { continue };
            for effect in def.effects["minecraft:post_attack"]
                .as_array()
                .into_iter()
                .flatten()
            {
                let e = &effect["effect"];
                let is_ignite = matches!(e["type"].as_str(), Some("minecraft:ignite" | "ignite"));
                if !is_ignite
                    || effect["enchanted"] != "attacker"
                    || effect["affected"] != "victim"
                    || !effect
                        .get("requirements")
                        .is_none_or(|r| self.condition(r, *level, ctx, random))
                {
                    continue;
                }
                seconds = seconds.max(level_value(&e["duration"], *level));
            }
        }
        seconds
    }
    pub fn tag(&self, name: &str) -> &[Arc<str>] {
        self.tags
            .get(name.trim_start_matches('#'))
            .map_or(&[], Vec::as_slice)
    }
    pub fn in_tag(&self, name: &str, id: &str) -> bool {
        self.tag(name).iter().any(|e| &**e == id)
    }
    pub fn is_curse(&self, id: &str) -> bool {
        self.in_tag("minecraft:curse", id)
    }
    pub fn translate(&self, key: &str) -> Option<&str> {
        self.lang.get(key).map(String::as_str)
    }
    pub fn props(&self, item: &str) -> Option<&ItemProps> {
        self.props.get(item)
    }
    pub fn enchantable(&self, item: &str) -> u32 {
        self.props(item).map_or(0, |p| p.enchantable)
    }
    /// `ItemStack.get(DataComponents.ENCHANTABLE)`: the stack's component,
    /// else the item's default; a removed component leaves none.
    pub fn enchantable_of(&self, stack: &ItemStack) -> u32 {
        let data = stack.data();
        if data.is_some_and(|d| d.other("!minecraft:enchantable").is_some()) {
            return 0;
        }
        if let Some(tag) = data.and_then(|d| d.other("minecraft:enchantable")) {
            return match tag.get("value") {
                Some(Tag::Byte(v)) => u32::try_from(i32::from(*v)),
                Some(Tag::Short(v)) => u32::try_from(i32::from(*v)),
                Some(Tag::Int(v)) => u32::try_from(*v),
                _ => Ok(0),
            }
            .unwrap_or(0);
        }
        self.enchantable(&stack.item)
    }
    /// Whether `item` matches an id / `#tag` list.
    pub fn item_in(items: &Items, specs: &[String], item: &str) -> bool {
        specs.iter().any(|spec| {
            crate::properties::item_matches(spec, item, |tag| items.item_in_tag(tag, item))
        })
    }
    /// `Enchantment.canEnchant` / `isSupportedItem`.
    pub fn can_enchant(&self, items: &Items, id: &str, item: &str) -> bool {
        self.get(id)
            .is_some_and(|e| Self::item_in(items, &e.supported, item))
    }
    /// `Enchantment.isPrimaryItem`.
    pub fn is_primary(&self, items: &Items, e: &Enchantment, item: &str) -> bool {
        Self::item_in(items, &e.supported, item)
            && e.primary
                .as_ref()
                .is_none_or(|p| Self::item_in(items, p, item))
    }
    /// `Enchantment.areCompatible`.
    pub fn compatible(&self, a: &str, b: &str) -> bool {
        if a == b {
            return false;
        }
        let excl = |x: &str, y: &str| self.get(x).is_some_and(|e| e.exclusive.contains(y));
        !excl(a, b) && !excl(b, a)
    }
    /// `Enchantment.getFullname` as plain text and whether it is a curse (red).
    pub fn full_name(&self, id: &str, level: u32) -> (String, bool) {
        let language = horizon_client_core::lang::current();
        self.full_name_in(language.as_deref(), id, level)
    }
    /// `Enchantment.getFullname` against `language`; `None` (no language loaded: a
    /// dedicated server, the probes) keeps the English names loaded above.
    pub fn full_name_in(
        &self,
        language: Option<&horizon_client_core::lang::Language>,
        id: &str,
        level: u32,
    ) -> (String, bool) {
        let numeric = NUMERIC_LEVELS.load(Ordering::Relaxed);
        self.full_name_with(language, id, level, numeric)
    }
    /// [`Enchantments::full_name_in`] with the level as a number when `numeric`.
    fn full_name_with(
        &self,
        language: Option<&horizon_client_core::lang::Language>,
        id: &str,
        level: u32,
        numeric: bool,
    ) -> (String, bool) {
        let tr = |key: &str| language.and_then(|l| l.get(key).map(str::to_owned));
        let Some(e) = self.get(id) else {
            return (format!("{id} {}", level_name(level)), false);
        };
        let mut name = tr(&e.description_key).unwrap_or_else(|| e.description.clone());
        if (level != 1 || e.max_level != 1) && numeric {
            name.push(' ');
            name.push_str(&level.to_string());
        } else if level != 1 || e.max_level != 1 {
            name.push(' ');
            let key = format!("enchantment.level.{level}");
            let suffix = tr(&key)
                .or_else(|| self.lang.get(&key).cloned())
                .unwrap_or_else(|| {
                    if (1..=10).contains(&level) {
                        level_name(level)
                    } else {
                        key
                    }
                });
            name.push_str(&suffix);
        }
        (name, self.is_curse(id))
    }
    /// Tooltip lines for a level list, in `#tooltip_order` then remaining order.
    pub fn tooltip(&self, levels: &[(Arc<str>, u32)]) -> Vec<(String, bool)> {
        let order = self.tag("minecraft:tooltip_order");
        let mut out = Vec::new();
        for id in order {
            if let Some((_, l)) = levels.iter().find(|(e, _)| e == id) {
                out.push(self.full_name(id, *l));
            }
        }
        for (id, l) in levels {
            if !order.contains(id) {
                out.push(self.full_name(id, *l));
            }
        }
        out
    }

    // ---- Enchanting table (EnchantmentHelper) ----

    /// `EnchantmentHelper.getEnchantmentCost`.
    pub fn enchantment_cost<R: RandomSource>(
        &self,
        random: &mut R,
        slot: usize,
        bookcases: i32,
        item: &str,
    ) -> i32 {
        self.enchantment_cost_with(random, slot, bookcases, self.enchantable(item))
    }
    /// `EnchantmentHelper.getEnchantmentCost` for a stack's own `ENCHANTABLE`.
    pub fn enchantment_cost_of<R: RandomSource>(
        &self,
        random: &mut R,
        slot: usize,
        bookcases: i32,
        stack: &ItemStack,
    ) -> i32 {
        self.enchantment_cost_with(random, slot, bookcases, self.enchantable_of(stack))
    }
    fn enchantment_cost_with<R: RandomSource>(
        &self,
        random: &mut R,
        slot: usize,
        bookcases: i32,
        enchantable: u32,
    ) -> i32 {
        if enchantable == 0 {
            return 0;
        }
        let bookcases = bookcases.min(15);
        let selected = next_int(random, 8) + 1 + (bookcases >> 1) + next_int(random, bookcases + 1);
        match slot {
            0 => (selected / 3).max(1),
            1 => selected * 2 / 3 + 1,
            _ => selected.max(bookcases * 2),
        }
    }
    /// `EnchantmentHelper.getAvailableEnchantmentResults`.
    pub fn available_results(
        &self,
        items: &Items,
        value: i32,
        item: &str,
        pool: &[Arc<str>],
    ) -> Vec<(Arc<str>, u32)> {
        let book = item == "minecraft:book";
        let mut out = Vec::new();
        for id in pool {
            let Some(e) = self.get(id) else { continue };
            if !(book || self.is_primary(items, e, item)) {
                continue;
            }
            for level in (1..=e.max_level).rev() {
                if value >= e.min_cost.calculate(level) && value <= e.max_cost.calculate(level) {
                    out.push((e.id.clone(), level));
                    break;
                }
            }
        }
        out
    }
    fn weighted<R: RandomSource>(
        &self,
        random: &mut R,
        list: &[(Arc<str>, u32)],
    ) -> Option<(Arc<str>, u32)> {
        let total: i64 = list
            .iter()
            .map(|(id, _)| self.get(id).map_or(0, |e| i64::from(e.weight)))
            .sum();
        if total <= 0 {
            return None;
        }
        // Preserve vanilla's int draw. Oversized pack totals use one unsigned
        // long modulo the total, a bounded, slightly biased fallback.
        let mut index = if total <= i64::from(i32::MAX) {
            i64::from(next_int(random, total as i32))
        } else {
            ((random.next_long() as u64) % total as u64) as i64
        };
        for (id, l) in list {
            index -= self.get(id).map_or(0, |e| i64::from(e.weight));
            if index < 0 {
                return Some((id.clone(), *l));
            }
        }
        None
    }
    /// Select enchantments using the stack's enchantable component.
    pub fn select<R: RandomSource>(
        &self,
        items: &Items,
        random: &mut R,
        stack: &ItemStack,
        cost: i32,
        pool: &[Arc<str>],
    ) -> Levels {
        self.select_with_value(
            items,
            random,
            &stack.item,
            cost,
            pool,
            self.enchantable_of(stack),
        )
    }
    /// Select with the enchantability of the stack after its component patch.
    pub fn select_with_value<R: RandomSource>(
        &self,
        items: &Items,
        random: &mut R,
        item: &str,
        cost: i32,
        pool: &[Arc<str>],
        value: u32,
    ) -> Levels {
        let mut results = Vec::new();
        let Ok(value) = i32::try_from(value) else {
            return results;
        };
        if value == 0 {
            return results;
        }
        let bound = value / 4 + 1;
        let mut cost = cost
            .wrapping_add(1)
            .wrapping_add(next_int(random, bound))
            .wrapping_add(next_int(random, bound));
        let span = (random.next_float() + random.next_float() - 1.0) * 0.15;
        cost = ((cost as f32 + cost as f32 * span).round() as i32).max(1);
        let mut available = self.available_results(items, cost, item, pool);
        if !available.is_empty() {
            if let Some(first) = self.weighted(random, &available) {
                results.push(first);
            }
            while next_int(random, 50) <= cost {
                if let Some((last, _)) = results.last() {
                    let last = last.clone();
                    available.retain(|(e, _)| self.compatible(&last, e));
                }
                if available.is_empty() {
                    break;
                }
                if let Some(next) = self.weighted(random, &available) {
                    results.push(next);
                }
                cost /= 2;
            }
        }
        results
    }

    // ---- Effect components ----

    /// Run `component` value effects of every enchantment on `stack` over `value`.
    pub fn modify_value(
        &self,
        stack: Option<&ItemStack>,
        component: &str,
        ctx: &EffectContext<'_>,
        random: &mut LegacyRandom,
        value: f32,
    ) -> f32 {
        let Some(stack) = stack else { return value };
        let mut value = value;
        for (id, level) in stack.all_enchantments().iter() {
            value = self.modify_value_with(id, *level, component, ctx, random, value);
        }
        value
    }
    pub fn modify_value_with(
        &self,
        id: &str,
        level: u32,
        component: &str,
        ctx: &EffectContext<'_>,
        random: &mut LegacyRandom,
        mut value: f32,
    ) -> f32 {
        let Some(e) = self.get(id) else { return value };
        for effect in e.effects[component].as_array().into_iter().flatten() {
            if let Some(req) = effect.get("requirements")
                && !self.condition(req, level, ctx, random)
            {
                continue;
            }
            value = apply_value_effect(&effect["effect"], level, random, value);
        }
        value
    }
    /// Evaluate damage with a gameplay-owned callback for resolved custom nodes.
    pub fn modify_damage(
        &self,
        levels: &[(Arc<str>, u32)],
        ctx: &EffectContext<'_>,
        random: &mut LegacyRandom,
        mut value: f32,
        custom: &mut impl FnMut(usize, &horizon_core::nbt::Tag, u32, f32) -> f32,
    ) -> f32 {
        for (id, level) in levels.iter() {
            let Some(nodes) = self.custom_damage.get(id.as_ref()) else {
                value = self.modify_value_with(id, *level, "minecraft:damage", ctx, random, value);
                continue;
            };
            let enchantment = self.get(id).expect("compiled enchantment exists");
            for (entry, node) in enchantment.effects["minecraft:damage"]
                .as_array()
                .into_iter()
                .flatten()
                .zip(nodes)
            {
                if entry
                    .get("requirements")
                    .is_none_or(|req| self.condition(req, *level, ctx, random))
                {
                    value = node.apply(*level, random, value, custom);
                }
            }
        }
        value
    }
    /// Whether this stack needs the gameplay callback evaluator. Keys were resolved at load.
    pub fn has_custom_damage(&self, levels: &[(Arc<str>, u32)]) -> bool {
        if self.custom_damage.is_empty() {
            return false;
        }
        levels
            .iter()
            .any(|(id, _)| self.custom_damage.contains_key(id.as_ref()))
    }
    /// Whether loaded content contains any custom damage nodes.
    pub fn has_custom_damage_effects(&self) -> bool {
        !self.custom_damage.is_empty()
    }
    /// `EnchantmentHelper.has`: whether `stack` has any enchantment providing
    /// `component`. It reads the stack's own component.
    pub fn has_component(&self, stack: &ItemStack, component: &str) -> bool {
        stack.enchantments().iter().any(|(id, _)| {
            self.get(id)
                .is_some_and(|e| e.effects.get(component).is_some())
        })
    }
    /// `EnchantmentHelper.has(stack, PREVENT_ARMOR_CHANGE)`: whether an enchantment on `stack`
    /// keeps it on as worn armour (Curse of Binding, or a data pack's own).
    pub fn prevents_armor_change(&self, stack: &ItemStack) -> bool {
        self.has_component(stack, "minecraft:prevent_armor_change")
    }
    /// `minecraft:attributes` contributions of `stack`: (attribute, id, amount, operation).
    pub fn attributes(&self, stack: &ItemStack) -> Vec<(String, String, f64, String)> {
        let mut out = Vec::new();
        for (id, level) in stack.all_enchantments().iter() {
            let Some(e) = self.get(id) else { continue };
            for a in e.effects["minecraft:attributes"]
                .as_array()
                .into_iter()
                .flatten()
            {
                out.push((
                    a["attribute"].as_str().unwrap_or("").to_owned(),
                    a["id"].as_str().unwrap_or("").to_owned(),
                    f64::from(level_value(&a["amount"], *level)),
                    a["operation"].as_str().unwrap_or("add_value").to_owned(),
                ));
            }
        }
        out
    }
    /// `EnchantmentHelper.forEachModifier` as an item's tooltip lists them: the
    /// `minecraft:attributes` effects of `stack`'s enchantments, one row per
    /// slot group the enchantment lists.
    pub fn modifier_rows(&self, stack: &ItemStack) -> Vec<crate::item_data::ModifierRow> {
        let mut rows = Vec::new();
        for (id, level) in stack.all_enchantments().iter() {
            let Some(e) = self.get(id) else { continue };
            for a in e.effects["minecraft:attributes"]
                .as_array()
                .into_iter()
                .flatten()
            {
                for slot in &e.slots {
                    rows.push(crate::item_data::ModifierRow::new(
                        a["attribute"].as_str().unwrap_or(""),
                        a["id"].as_str().unwrap_or(""),
                        f64::from(level_value(&a["amount"], *level)),
                        a["operation"].as_str().unwrap_or("add_value"),
                        slot,
                    ));
                }
            }
        }
        rows
    }
    /// Sum of `add_value` amounts of one attribute from enchantments on `stack`.
    pub fn attribute_add(&self, stack: Option<&ItemStack>, attribute: &str) -> f64 {
        stack.map_or(0.0, |s| {
            self.attributes(s)
                .iter()
                .filter(|a| a.0 == attribute && a.3 == "add_value")
                .map(|a| a.2)
                .sum()
        })
    }
    fn entity_matches(&self, spec: &Value, entity: Option<&str>) -> bool {
        let Some(entity) = entity else { return false };
        match spec.as_str() {
            Some(s) => match s.strip_prefix('#') {
                Some(tag) => self
                    .entity_tags
                    .get(tag)
                    .is_some_and(|t| t.contains(entity)),
                None => s == entity,
            },
            None => string_list(spec).iter().any(|s| s == entity),
        }
    }
    /// The loot-condition subset enchantment `requirements` use.
    pub fn condition(
        &self,
        c: &Value,
        level: u32,
        ctx: &EffectContext<'_>,
        random: &mut LegacyRandom,
    ) -> bool {
        match c["type"].as_str().or(c["condition"].as_str()).unwrap_or("") {
            "minecraft:inverted" => !self.condition(&c["term"], level, ctx, random),
            "minecraft:all_of" => c["terms"]
                .as_array()
                .into_iter()
                .flatten()
                .all(|t| self.condition(t, level, ctx, random)),
            "minecraft:any_of" => c["terms"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|t| self.condition(t, level, ctx, random)),
            "minecraft:match_tool" => {
                let Some(tool) = ctx.tool else { return false };
                let items = &c["predicate"]["items"];
                if items.is_null() {
                    return true;
                }
                string_list(items)
                    .iter()
                    .any(|s| match s.strip_prefix('#') {
                        Some(tag) => ctx.items.is_some_and(|i| {
                            i.item_tags.get(tag).is_some_and(|t| t.contains(tool))
                        }),
                        None => s == tool,
                    })
            }
            "minecraft:damage_source_properties" => {
                let p = &c["predicate"];
                if let Some(direct) = p["is_direct"].as_bool()
                    && direct != ctx.is_direct
                {
                    return false;
                }
                p["tags"].as_array().into_iter().flatten().all(|t| {
                    let tag = t["id"].as_str().unwrap_or("").trim_start_matches('#');
                    let has = ctx
                        .damage_type
                        .is_some_and(|d| self.damage_tags.get(tag).is_some_and(|s| s.contains(d)));
                    has == t["expected"].as_bool().unwrap_or(true)
                })
            }
            "minecraft:entity_properties" => {
                let entity = match c["entity"].as_str().unwrap_or("this") {
                    "this" => ctx.this,
                    "attacker" => ctx.attacker,
                    "direct_attacker" => ctx.direct_attacker,
                    _ => None,
                };
                let spec = &c["predicate"]["minecraft:entity_type"];
                let spec = if spec.is_null() {
                    &c["predicate"]["type"]
                } else {
                    spec
                };
                (spec.is_null() && entity.is_some() || self.entity_matches(spec, entity))
                    && movement_matches(&c["predicate"], role_state(c, ctx))
            }
            "minecraft:random_chance" => {
                let chance = match &c["chance"] {
                    Value::Object(o)
                        if o.get("type").and_then(Value::as_str)
                            == Some("minecraft:enchantment_level") =>
                    {
                        level_value(&c["chance"]["amount"], level)
                    }
                    v => v
                        .as_f64()
                        .map_or_else(|| level_value(v, level), |f| f as f32),
                };
                random.next_float() < chance
            }
            _ => false,
        }
    }
}
/// What enchantment requirement predicates can see.
#[derive(Clone, Copy, Default)]
pub struct EffectContext<'a> {
    pub items: Option<&'a Items>,
    /// The tool for `match_tool` (the enchanted item, or the ammo for ammo_use).
    pub tool: Option<&'a str>,
    /// Damage type id (e.g. `minecraft:fall`).
    pub damage_type: Option<&'a str>,
    pub is_direct: bool,
    /// Entity type ids: the victim/user ("this"), attacker and direct attacker.
    pub this: Option<&'a str>,
    pub attacker: Option<&'a str>,
    pub direct_attacker: Option<&'a str>,
    /// The direct attacker's fall distance and whether it flies (elytra or
    /// abilities), for `movement` and `flags` predicates.
    pub direct_motion: Option<(f64, bool)>,
}
/// The fall distance and flying flag known for an `entity_properties` role.
fn role_state(c: &Value, ctx: &EffectContext<'_>) -> Option<(f64, bool)> {
    match c["entity"].as_str().unwrap_or("this") {
        "direct_attacker" => ctx.direct_motion,
        _ => None,
    }
}
/// `EntityPredicate`'s `minecraft:movement` fall distance and
/// `minecraft:flags` is_flying (the parts enchantment requirements use); an
/// entity whose state is unknown matches neither.
fn movement_matches(predicate: &Value, state: Option<(f64, bool)>) -> bool {
    let fall = &predicate["minecraft:movement"]["fall_distance"];
    let flying = predicate["minecraft:flags"]["is_flying"].as_bool();
    if fall.is_null() && flying.is_none() {
        return true;
    }
    let Some((distance, is_flying)) = state else {
        return false;
    };
    // MinMaxBounds.Doubles: a number is an exact bound.
    let bound = |key: &str| {
        fall.as_f64()
            .or_else(|| fall.get(key).and_then(Value::as_f64))
    };
    // MinMaxBounds.Doubles.matches
    bound("min").is_none_or(|min| min <= distance)
        && bound("max").is_none_or(|max| distance <= max)
        && flying.is_none_or(|f| f == is_flying)
}
fn next_int<R: RandomSource>(random: &mut R, bound: i32) -> i32 {
    random.next_int_bound(bound.max(1)).unwrap_or(0)
}
/// `LevelBasedValue.calculate`.
pub fn level_value(v: &Value, level: u32) -> f32 {
    let l = level as f32;
    match v {
        Value::Number(n) => n.as_f64().unwrap_or(0.0) as f32,
        Value::Object(o) => {
            let f = |k: &str| o.get(k).and_then(Value::as_f64).unwrap_or(0.0) as f32;
            match o.get("type").and_then(Value::as_str).unwrap_or("") {
                "minecraft:linear" => f("base") + f("per_level_above_first") * (l - 1.0),
                "minecraft:levels_squared" => l * l + f("added"),
                "minecraft:clamped" => level_value(&o["value"], level).clamp(f("min"), f("max")),
                "minecraft:fraction" => {
                    let d = level_value(&o["denominator"], level);
                    if d == 0.0 {
                        0.0
                    } else {
                        level_value(&o["numerator"], level) / d
                    }
                }
                "minecraft:exponent" => {
                    level_value(&o["base"], level).powf(level_value(&o["power"], level))
                }
                "minecraft:lookup" => {
                    let values = o.get("values").and_then(Value::as_array);
                    match values.and_then(|a| a.get(level as usize - 1)) {
                        Some(v) => v.as_f64().unwrap_or(0.0) as f32,
                        None => level_value(&o["fallback"], level),
                    }
                }
                _ => f("value"),
            }
        }
        _ => 0.0,
    }
}
/// `EnchantmentValueEffect.process`.
pub fn apply_value_effect(
    effect: &Value,
    level: u32,
    random: &mut LegacyRandom,
    value: f32,
) -> f32 {
    match effect["type"].as_str().unwrap_or("") {
        "minecraft:add" => value + level_value(&effect["value"], level),
        "minecraft:multiply" => value * level_value(&effect["factor"], level),
        "minecraft:set" => level_value(&effect["value"], level),
        "minecraft:remove_binomial" => {
            let p = level_value(&effect["chance"], level);
            let n = value;
            // Normal approximation for large counts (written as vanilla's negated tests).
            let large = n > 128.0 && n * p >= 20.0 && n * (1.0 - p) >= 20.0;
            let removed = if large {
                let miu = (f64::from(n) * f64::from(p)).floor();
                let sigma = (f64::from(n) * f64::from(p) * f64::from(1.0 - p)).sqrt();
                ((miu + random.next_gaussian() * sigma).round() as i32).clamp(0, n as i32)
            } else {
                let mut d = 0;
                for _ in 0..n as i32 {
                    if random.next_float() < p {
                        d += 1;
                    }
                }
                d
            };
            n - removed as f32
        }
        "minecraft:all_of" => effect["effects"]
            .as_array()
            .into_iter()
            .flatten()
            .fold(value, |v, e| apply_value_effect(e, level, random, v)),
        _ => value,
    }
}

#[cfg(any(test, feature = "test-support"))]
pub mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn custom_damage_output_is_bounded_before_builtin_nodes() {
        let node = DamageValueEffect::resolve(
            &json!({"type":"minecraft:all_of", "effects":[
                {"type":"test:damage"}, {"type":"minecraft:remove_binomial","chance":0},
                {"type":"minecraft:add","value":2}
            ]}),
            &["test:damage".into()],
        )
        .unwrap();
        for (output, expected) in [
            (f32::NAN, 9.0),
            (f32::INFINITY, 1026.0),
            (f32::NEG_INFINITY, 2.0),
            (-1.0, 2.0),
            (f32::MAX, 1026.0),
        ] {
            assert_eq!(
                node.apply(1, &mut LegacyRandom::new(0), 7.0, &mut |_, _, _, _| output),
                expected
            );
        }
    }

    #[test]
    fn synced_unknown_damage_retains_enchantment_definition() {
        let definition = json!({"description":"Remote", "effects":{"minecraft:damage":[
            {"effect":{"type":"remote:damage"}}
        ]}});
        let table = sample()
            .with_entries([("remote:enchantment", &definition)])
            .unwrap()
            .unwrap();
        assert!(table.get("remote:enchantment").is_some());
    }

    #[test]
    fn legacy_value_types_remain_noops() {
        for kind in ["minecraft:exponential", "minecraft:unknown", "add", ""] {
            let node = DamageValueEffect::resolve(&json!({"type":kind}), &[]).unwrap();
            assert_eq!(
                node.apply(1, &mut LegacyRandom::new(0), 7.0, &mut |_, _, _, _| panic!(
                    "not custom"
                )),
                7.0
            );
        }
    }

    #[test]
    fn custom_damage_nodes_compose_with_vanilla_in_order() {
        let mut enchantments = sample();
        let node = json!({"type":"minecraft:all_of", "effects":[
            {"type":"minecraft:add", "value":2},
            {"type":"test:scale", "factor":3},
            {"type":"minecraft:all_of", "effects":[
                {"type":"test:offset", "value":4},
                {"type":"minecraft:multiply", "factor":2}
            ]}
        ]});
        let compiled =
            DamageValueEffect::resolve(&node, &["test:scale".into(), "test:offset".into()])
                .unwrap();
        enchantments
            .custom_damage
            .insert("minecraft:sharpness".into(), vec![compiled]);
        let index = enchantments.index["minecraft:sharpness"];
        enchantments.list[index].effects = json!({"minecraft:damage":[{"effect":node}]});
        let mut stack = ItemStack::new("minecraft:diamond_sword", 1);
        stack.data_mut().enchantments = vec![("minecraft:sharpness".into(), 2)];
        let mut calls = Vec::new();
        let result = enchantments.modify_damage(
            &stack.all_enchantments(),
            &EffectContext::default(),
            &mut LegacyRandom::new(0),
            5.0,
            &mut |slot, config, level, value| {
                calls.push((slot, level, value));
                let config = horizon_core::json_nbt::tag_to_json(config).unwrap();
                match slot {
                    0 => value * config["factor"].as_f64().unwrap() as f32,
                    1 => value + config["value"].as_f64().unwrap() as f32,
                    _ => panic!("unresolved slot"),
                }
            },
        );
        // ((5 + 2) * 3 + 4) * 2, including both nested custom nodes.
        assert_eq!(result, 50.0);
        assert_eq!(calls, [(0, 2, 7.0), (1, 2, 21.0)]);
    }

    #[test]
    fn custom_damage_requirements_skip_callbacks_and_keep_random_order() {
        let mut enchantments = sample();
        let effect = json!({"type":"test:damage"});
        enchantments.custom_damage.insert(
            "minecraft:sharpness".into(),
            vec![DamageValueEffect::resolve(&effect, &["test:damage".into()]).unwrap()],
        );
        let index = enchantments.index["minecraft:sharpness"];
        enchantments.list[index].effects = json!({"minecraft:damage":[{
            "effect":effect, "requirements":{"condition":"minecraft:random_chance", "chance":0.0}
        }]});
        let mut stack = ItemStack::new("minecraft:diamond_sword", 1);
        stack.data_mut().enchantments = vec![("minecraft:sharpness".into(), 1)];
        let mut random = LegacyRandom::new(42);
        let value = enchantments.modify_damage(
            &stack.all_enchantments(),
            &EffectContext::default(),
            &mut random,
            7.0,
            &mut |_, _, _, _| panic!("a refused node must not run"),
        );
        assert_eq!(value, 7.0);
        // java.util.Random(42): nextFloat for the condition, then nextInt.
        assert_eq!(random.next_int(), 234_785_527);
    }

    #[test]
    fn unknown_custom_damage_is_refused_with_its_identifier() {
        let node = json!({"type":"minecraft:all_of", "effects":[{"type":"missing:damage"}]});
        assert_eq!(
            DamageValueEffect::resolve(&node, &[]).err().unwrap(),
            "unknown enchantment damage value effect missing:damage"
        );
    }

    #[test]
    fn damage_without_custom_nodes_preserves_vanilla_values_and_random() {
        let enchantments = sample();
        let mut stack = ItemStack::new("minecraft:diamond_sword", 1);
        let mut calls = 0;
        for (levels, expected) in [
            (vec![], 7.0),
            (vec![("minecraft:sharpness".into(), 3)], 9.0),
            (vec![("minecraft:smite".into(), 2)], 12.0),
        ] {
            stack.data_mut().enchantments = levels;
            let context = EffectContext {
                this: Some("minecraft:zombie"),
                ..Default::default()
            };
            let mut random = LegacyRandom::new(42);
            assert_eq!(
                enchantments.modify_damage(
                    &stack.all_enchantments(),
                    &context,
                    &mut random,
                    7.0,
                    &mut |_, _, _, value| {
                        calls += 1;
                        value
                    }
                ),
                expected
            );
            assert_eq!(random.next_int(), -1_170_105_035);
        }
        assert_eq!(calls, 0);
    }

    #[test]
    fn world_pack_definitions_join_catalog_in_overlay_priority() {
        let root = std::env::temp_dir().join(format!(
            "horizon-enchantment-overlay-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let write = |path: &str, value: &Value| {
            let path = root.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, serde_json::to_vec(value).unwrap()).unwrap();
            std::fs::read(path).unwrap()
        };
        let definition = |description: &str, max_level| {
            json!({
                "description": description, "supported_items": "minecraft:diamond_sword",
                "weight": 1, "max_level": max_level,
                "min_cost": {"base": 1, "per_level_above_first": 0},
                "max_cost": {"base": 1, "per_level_above_first": 0},
                "anvil_cost": 1, "slots": ["mainhand"],
                "effects": {"minecraft:damage": [{"effect": {"type": "minecraft:add", "value": 2.0}}]}
            })
        };
        write(
            "data/minecraft/enchantment/mending.json",
            &definition("Vanilla retained", 1),
        );
        write(
            "data/minecraft/enchantment/sharpness.json",
            &definition("Vanilla base", 5),
        );
        write(
            "data/minecraft/tags/enchantment/fixture.json",
            &json!({"values": ["minecraft:mending"]}),
        );
        write(
            "data/minecraft/tags/damage_type/fixture.json",
            &json!({"values": ["minecraft:generic"]}),
        );
        write(
            "data/minecraft/tags/entity_type/fixture.json",
            &json!({"values": ["minecraft:pig"]}),
        );
        let custom = write(
            "packs/custom/data/wpenchant/enchantment/loaded.json",
            &definition("Custom loaded", 1),
        );
        let low = write(
            "packs/low/data/minecraft/enchantment/sharpness.json",
            &definition("Lower override", 2),
        );
        let high = write(
            "packs/high/data/minecraft/enchantment/sharpness.json",
            &definition("Higher override", 3),
        );
        let tag = write(
            "packs/custom/data/wpenchant/tags/enchantment/fixture.json",
            &json!({"values": ["wpenchant:loaded"]}),
        );
        let files = vec![
            ("enchantment/wpenchant:loaded".to_owned(), vec![custom]),
            (
                "enchantment/minecraft:sharpness".to_owned(),
                vec![low, high],
            ),
            ("tags/wpenchant:enchantment/fixture".to_owned(), vec![tag]),
        ];
        write("items.json", &json!({}));
        let defaults = horizon_world::item_defaults::read(&root).unwrap();
        let base = Enchantments::load(&root, &defaults).unwrap();
        let loaded = Enchantments::load_with(
            &root,
            &horizon_world::loot::PackOverlay {
                files: &files,
                ..horizon_world::loot::PackOverlay::NONE
            },
            &defaults,
        )
        .unwrap();
        assert_eq!(
            loaded.get("minecraft:mending").unwrap().description,
            "Vanilla retained"
        );
        assert_eq!(loaded.damage_tags, base.damage_tags);
        assert_eq!(loaded.entity_tags, base.entity_tags);
        assert_eq!(
            loaded.tag("minecraft:fixture"),
            base.tag("minecraft:fixture")
        );
        assert!(loaded.in_tag("wpenchant:fixture", "wpenchant:loaded"));
        assert!(loaded.get("wpenchant:missing").is_none());
        let custom = loaded
            .get("wpenchant:loaded")
            .expect("actual overlay definition must enter the catalog");
        assert_eq!(custom.description, "Custom loaded");
        assert_eq!(custom.max_level, 1);
        assert_eq!(
            loaded.get("minecraft:sharpness").unwrap().description,
            "Higher override"
        );
        assert_eq!(loaded.get("minecraft:sharpness").unwrap().max_level, 3);
        assert_eq!(
            loaded.modify_value_with(
                "wpenchant:loaded",
                1,
                "minecraft:damage",
                &EffectContext::default(),
                &mut LegacyRandom::new(0),
                1.0
            ),
            3.0
        );
        assert!(
            Enchantments::load(&root, &defaults)
                .unwrap()
                .get("wpenchant:loaded")
                .is_none()
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    /// A content mod's enchantments (`data/<ns>/enchantment/`, copied into the
    /// resource root beside vanilla's) join the catalog with their own tags and
    /// English names, nested files keep their path, and a world's pack still
    /// replaces one.
    #[test]
    fn mod_namespace_definitions_join_catalog() {
        let root = std::env::temp_dir().join(format!(
            "horizon-enchantment-mods-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let write = |path: &str, value: &Value| {
            let path = root.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, serde_json::to_vec(value).unwrap()).unwrap();
            std::fs::read(path).unwrap()
        };
        std::fs::create_dir_all(root.join("data/harvestmod/enchantment/tools")).unwrap();
        std::fs::write(
            root.join("data/harvestmod/enchantment/tools/broken.json"),
            b"{",
        )
        .unwrap();
        let definition = |description: Value, max_level| {
            json!({
                "description": description, "supported_items": "minecraft:diamond_hoe",
                "weight": 1, "max_level": max_level,
                "min_cost": {"base": 1, "per_level_above_first": 0},
                "max_cost": {"base": 1, "per_level_above_first": 0},
                "anvil_cost": 1, "slots": ["mainhand"],
                "exclusive_set": "#harvestmod:exclusive/area"
            })
        };
        write(
            "data/minecraft/enchantment/mending.json",
            &json!({"description": "Mending", "supported_items": "minecraft:diamond_hoe",
                "max_level": 1, "weight": 2, "anvil_cost": 4, "slots": ["any"],
                "min_cost": {"base": 1, "per_level_above_first": 0},
                "max_cost": {"base": 1, "per_level_above_first": 0}}),
        );
        write(
            "data/harvestmod/enchantment/scythe.json",
            &definition(json!({"translate": "enchantment.harvestmod.scythe"}), 3),
        );
        write(
            "data/harvestmod/enchantment/tools/sweep.json",
            &definition(json!("Sweep"), 1),
        );
        write(
            "data/harvestmod/tags/enchantment/exclusive/area.json",
            &json!({"values": ["harvestmod:scythe", "harvestmod:tools/sweep"]}),
        );
        write(
            "data/minecraft/tags/enchantment/in_enchanting_table.json",
            &json!({"values": ["minecraft:mending", "harvestmod:scythe"]}),
        );
        write(
            "assets/harvestmod/lang/en_us.json",
            &json!({"enchantment.harvestmod.scythe": "Scythe", "item.harvestmod.x": "X"}),
        );
        write("items.json", &json!({}));
        let defaults = horizon_world::item_defaults::read(&root).unwrap();
        let loaded = Enchantments::load(&root, &defaults).unwrap();
        let scythe = loaded
            .get("harvestmod:scythe")
            .expect("a mod's own enchantment must enter the catalog");
        assert_eq!(scythe.description, "Scythe");
        assert_eq!(scythe.max_level, 3);
        assert!(scythe.exclusive.contains("harvestmod:tools/sweep"));
        assert_eq!(
            loaded.get("harvestmod:tools/sweep").unwrap().description,
            "Sweep"
        );
        assert!(loaded.in_tag("harvestmod:exclusive/area", "harvestmod:scythe"));
        assert!(loaded.in_tag("minecraft:in_enchanting_table", "harvestmod:scythe"));
        assert!(loaded.get("minecraft:mending").is_some());
        assert!(loaded.get("minecraft:scythe").is_none());
        assert!(loaded.get("harvestmod:tools/broken").is_none());

        // A world's pack replaces the mod's definition for its id.
        let pack = write(
            "packs/p/data/harvestmod/enchantment/scythe.json",
            &definition(json!("Pack scythe"), 5),
        );
        let files = vec![("enchantment/harvestmod:scythe".to_owned(), vec![pack])];
        let packed = Enchantments::load_with(
            &root,
            &horizon_world::loot::PackOverlay::new(&files),
            &defaults,
        )
        .unwrap();
        let scythe = packed.get("harvestmod:scythe").unwrap();
        assert_eq!(scythe.description, "Pack scythe");
        assert_eq!(scythe.max_level, 5);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn native_payloads_are_retained_and_invalid_definitions_refused() {
        let tags = HashMap::new();
        let lang = HashMap::new();
        let definition = json!({"effects":{"minecraft:post_attack":[
            {"effect":{"type":"minecraft:ignite","duration":4}},
            {"effect":{"type":"example:charge","amount":42}}
        ]}});
        let entry = parse_entry("example:charge", &definition, &tags, &lang).unwrap();
        assert!(!entry.native_post_attack.contains_key(&0));
        let payload = &entry.native_post_attack[&1];
        assert_eq!(
            payload.get("amount"),
            Some(&horizon_core::nbt::Tag::Byte(42))
        );
        assert!(Arc::ptr_eq(payload, &entry.clone().native_post_attack[&1]));
        let bad = json!({"effects":{"minecraft:post_attack":[{"effect":{"type":"example:charge","bad":null}}]}});
        assert!(parse_entry("example:charge", &bad, &tags, &lang).is_err());
        let base = sample();
        let before = base.list.len();
        let error = base
            .with_entries([("example:valid", &definition), ("example:invalid", &bad)])
            .err()
            .unwrap();
        assert!(error.to_string().contains("example:invalid"));
        assert_eq!(base.list.len(), before);
        assert!(base.get("example:valid").is_none());

        let root = std::env::temp_dir().join(format!(
            "horizon-enchant-refusal-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join("data/example/enchantment")).unwrap();
        std::fs::write(root.join("items.json"), "{}").unwrap();
        std::fs::write(
            root.join("data/example/enchantment/invalid.json"),
            bad.to_string(),
        )
        .unwrap();
        let defaults = horizon_world::item_defaults::read(&root).unwrap();
        let error = Enchantments::load(&root, &defaults).err().unwrap();
        assert!(error.to_string().contains("example:invalid"));
        std::fs::remove_file(root.join("data/example/enchantment/invalid.json")).unwrap();
        let files = vec![(
            "enchantment/example:charge".to_owned(),
            vec![serde_json::to_vec(&definition).unwrap()],
        )];
        let overlay = horizon_world::loot::PackOverlay {
            files: &files,
            ..horizon_world::loot::PackOverlay::NONE
        };
        let loaded = Enchantments::load_with(&root, &overlay, &defaults).unwrap();
        assert_eq!(
            loaded.get("example:charge").unwrap().native_post_attack[&1].get("amount"),
            Some(&horizon_core::nbt::Tag::Byte(42))
        );
        let files = vec![(
            "enchantment/example:invalid".to_owned(),
            vec![serde_json::to_vec(&bad).unwrap()],
        )];
        let overlay = horizon_world::loot::PackOverlay {
            files: &files,
            ..horizon_world::loot::PackOverlay::NONE
        };
        assert!(
            Enchantments::load_with(&root, &overlay, &defaults)
                .err()
                .unwrap()
                .to_string()
                .contains("example:invalid")
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    /// A small registry built from literal JSON matching the vanilla files.
    pub fn sample() -> Enchantments {
        let defs = [
            (
                "minecraft:sharpness",
                json!({"anvil_cost":1,"description":{"translate":"enchantment.minecraft.sharpness"},"exclusive_set":"#minecraft:exclusive_set/damage","max_cost":{"base":21,"per_level_above_first":11},"max_level":5,"min_cost":{"base":1,"per_level_above_first":11},"primary_items":"#minecraft:enchantable/melee_weapon","supported_items":"#minecraft:enchantable/sharp_weapon","weight":10,"slots":["mainhand"],"effects":{"minecraft:damage":[{"effect":{"type":"minecraft:add","value":{"type":"minecraft:linear","base":1.0,"per_level_above_first":0.5}}}]}}),
            ),
            (
                "minecraft:smite",
                json!({"anvil_cost":2,"exclusive_set":"#minecraft:exclusive_set/damage","max_cost":{"base":25,"per_level_above_first":8},"max_level":5,"min_cost":{"base":5,"per_level_above_first":8},"primary_items":"#minecraft:enchantable/melee_weapon","supported_items":"#minecraft:enchantable/weapon","weight":5,"effects":{"minecraft:damage":[{"effect":{"type":"minecraft:add","value":{"type":"minecraft:linear","base":2.5,"per_level_above_first":2.5}},"requirements":{"type":"minecraft:entity_properties","entity":"this","predicate":{"minecraft:entity_type":"#minecraft:sensitive_to_smite"}}}]}}),
            ),
            (
                "minecraft:unbreaking",
                json!({"anvil_cost":2,"max_cost":{"base":55,"per_level_above_first":8},"max_level":3,"min_cost":{"base":5,"per_level_above_first":8},"supported_items":"#minecraft:enchantable/durability","weight":5,"effects":{"minecraft:item_damage":[{"effect":{"type":"minecraft:remove_binomial","chance":{"type":"minecraft:fraction","denominator":{"type":"minecraft:linear","base":2.0,"per_level_above_first":1.0},"numerator":{"type":"minecraft:linear","base":1.0,"per_level_above_first":1.0}}}}]}}),
            ),
            (
                "minecraft:mending",
                json!({"anvil_cost":4,"max_cost":{"base":75,"per_level_above_first":25},"max_level":1,"min_cost":{"base":25,"per_level_above_first":25},"supported_items":"#minecraft:enchantable/durability","weight":2}),
            ),
            (
                "minecraft:feather_falling",
                json!({"anvil_cost":2,"max_cost":{"base":11,"per_level_above_first":6},"max_level":4,"min_cost":{"base":5,"per_level_above_first":6},"supported_items":"#minecraft:enchantable/foot_armor","weight":5,"effects":{"minecraft:damage_protection":[{"effect":{"type":"minecraft:add","value":{"type":"minecraft:linear","base":3.0,"per_level_above_first":3.0}},"requirements":{"type":"minecraft:damage_source_properties","predicate":{"tags":[{"expected":true,"id":"#minecraft:is_fall"},{"expected":false,"id":"#minecraft:bypasses_invulnerability"}]}}}]}}),
            ),
            (
                "minecraft:binding_curse",
                json!({"anvil_cost":8,"description":{"translate":"enchantment.minecraft.binding_curse"},"effects":{"minecraft:prevent_armor_change":{}},"max_cost":{"base":50,"per_level_above_first":0},"max_level":1,"min_cost":{"base":25,"per_level_above_first":0},"slots":["armor"],"supported_items":"#minecraft:enchantable/equippable","weight":1}),
            ),
            // A data pack's own enchantment with Curse of Binding's effect under another id.
            (
                "test:clinging",
                json!({"anvil_cost":8,"description":"Clinging","effects":{"minecraft:prevent_armor_change":{}},"max_cost":{"base":50,"per_level_above_first":0},"max_level":1,"min_cost":{"base":25,"per_level_above_first":0},"slots":["armor"],"supported_items":"#minecraft:enchantable/equippable","weight":1}),
            ),
        ];
        let list: Vec<Enchantment> = defs
            .iter()
            .map(|(id, v)| Enchantment {
                id: Arc::from(*id),
                description: id.trim_start_matches("minecraft:").into(),
                description_key: v["description"]["translate"].as_str().unwrap_or("").into(),
                max_level: v["max_level"].as_u64().unwrap() as u32,
                weight: v["weight"].as_u64().unwrap() as u32,
                anvil_cost: v["anvil_cost"].as_u64().unwrap() as u32,
                min_cost: Cost::parse(&v["min_cost"]),
                max_cost: Cost::parse(&v["max_cost"]),
                supported: string_list(&v["supported_items"]),
                primary: v.get("primary_items").map(string_list),
                exclusive: if v["exclusive_set"].is_null() {
                    HashSet::new()
                } else {
                    [
                        Arc::from("minecraft:sharpness"),
                        Arc::from("minecraft:smite"),
                    ]
                    .into_iter()
                    .collect()
                },
                slots: vec![],
                effects: v["effects"].clone(),
                native_post_attack: HashMap::new(),
            })
            .collect();
        let index = list
            .iter()
            .enumerate()
            .map(|(i, e)| (e.id.clone(), i))
            .collect();
        let pool: Vec<Arc<str>> = [
            "minecraft:feather_falling",
            "minecraft:sharpness",
            "minecraft:smite",
            "minecraft:unbreaking",
            "minecraft:mending",
        ]
        .iter()
        .map(|s| Arc::from(*s))
        .collect();
        let mut tags = HashMap::new();
        tags.insert(
            "minecraft:in_enchanting_table".to_owned(),
            pool[..4].to_vec(),
        );
        tags.insert(
            "minecraft:tooltip_order".to_owned(),
            vec![
                Arc::from("minecraft:sharpness"),
                Arc::from("minecraft:unbreaking"),
            ],
        );
        let mut entity_tags = HashMap::new();
        entity_tags.insert(
            "minecraft:sensitive_to_smite".to_owned(),
            ["minecraft:zombie".to_owned()].into_iter().collect(),
        );
        let mut damage_tags = HashMap::new();
        damage_tags.insert(
            "minecraft:is_fall".to_owned(),
            ["minecraft:fall".to_owned()].into_iter().collect(),
        );
        let mut props = HashMap::new();
        for (item, value) in [
            ("minecraft:diamond_sword", 10),
            ("minecraft:iron_sword", 14),
            ("minecraft:book", 1),
            ("minecraft:iron_boots", 9),
        ] {
            props.insert(
                Arc::from(item),
                ItemProps {
                    enchantable: value,
                    ..Default::default()
                },
            );
        }
        Enchantments {
            list,
            index,
            custom_damage: HashMap::new(),
            tags,
            entity_tags,
            damage_tags,
            props,
            lang: HashMap::new(),
        }
    }
    pub fn sample_items() -> Items {
        let mut item_tags: HashMap<String, HashSet<String>> = HashMap::new();
        let swords = ["minecraft:diamond_sword", "minecraft:iron_sword"];
        for tag in [
            "enchantable/melee_weapon",
            "enchantable/sharp_weapon",
            "enchantable/weapon",
            "enchantable/durability",
        ] {
            item_tags.insert(
                format!("minecraft:{tag}"),
                swords.iter().map(|s| (*s).to_owned()).collect(),
            );
        }
        item_tags
            .get_mut("minecraft:enchantable/durability")
            .unwrap()
            .insert("minecraft:iron_boots".into());
        item_tags.insert(
            "minecraft:enchantable/foot_armor".into(),
            ["minecraft:iron_boots".to_owned()].into_iter().collect(),
        );
        Items::for_tests(item_tags)
    }

    #[test]
    fn level_based_values() {
        assert_eq!(
            level_value(
                &json!({"type":"minecraft:linear","base":1.0,"per_level_above_first":0.5}),
                5
            ),
            3.0
        );
        assert_eq!(
            level_value(&json!({"type":"minecraft:levels_squared","added":1.0}), 3),
            10.0
        );
        assert_eq!(
            level_value(
                &json!({"type":"minecraft:fraction","numerator":{"type":"minecraft:linear","base":1.0,"per_level_above_first":1.0},"denominator":{"type":"minecraft:linear","base":2.0,"per_level_above_first":1.0}}),
                3
            ),
            0.75
        );
        assert_eq!(level_name(4), "IV");
        assert_eq!(
            Cost {
                base: 1,
                per_level: 11
            }
            .calculate(5),
            45
        );
    }

    #[test]
    fn enchanting_cost_matches_vanilla_formula() {
        let e = sample();
        // Replays EnchantmentHelper.getEnchantmentCost draws by hand.
        for seed in [0i64, 1, 42, -7, 123456] {
            for bookcases in [0, 5, 15, 20] {
                let mut a = LegacyRandom::new(seed);
                let mut b = LegacyRandom::new(seed);
                let costs: Vec<i32> = (0..3)
                    .map(|slot| {
                        e.enchantment_cost(&mut a, slot, bookcases, "minecraft:diamond_sword")
                    })
                    .collect();
                let bc = bookcases.min(15);
                let expect: Vec<i32> = (0..3)
                    .map(|slot| {
                        let s = b.next_int_bound(8).unwrap()
                            + 1
                            + (bc >> 1)
                            + b.next_int_bound(bc + 1).unwrap();
                        match slot {
                            0 => (s / 3).max(1),
                            1 => s * 2 / 3 + 1,
                            _ => s.max(bc * 2),
                        }
                    })
                    .collect();
                assert_eq!(costs, expect);
                assert!(costs[2] <= 30);
            }
        }
        assert_eq!(
            e.enchantment_cost(&mut LegacyRandom::new(0), 0, 15, "minecraft:stick"),
            0
        );
    }

    #[test]
    fn selection_respects_costs_and_exclusivity() {
        let e = sample();
        let items = sample_items();
        let pool = e.tag("minecraft:in_enchanting_table").to_vec();
        for seed in 0..200 {
            let mut r = LegacyRandom::new(seed);
            let picked = e.select(
                &items,
                &mut r,
                &ItemStack::new("minecraft:diamond_sword", 1),
                30,
                &pool,
            );
            assert!(!picked.is_empty());
            let has = |id: &str| picked.iter().any(|(p, _)| &**p == id);
            assert!(
                !(has("minecraft:sharpness") && has("minecraft:smite")),
                "{picked:?}"
            );
            assert!(!has("minecraft:feather_falling"));
            for (id, l) in &picked {
                assert!(*l >= 1 && *l <= e.get(id).unwrap().max_level);
            }
        }
        // Books accept every enchantment in the pool.
        let mut r = LegacyRandom::new(3);
        let available = e.available_results(&items, 20, "minecraft:book", &pool);
        assert!(
            available
                .iter()
                .any(|(id, _)| &**id == "minecraft:feather_falling")
        );
        assert!(
            e.select(
                &items,
                &mut r,
                &ItemStack::new("minecraft:stick", 1),
                30,
                &pool
            )
            .is_empty()
        );
    }

    #[test]
    fn value_effects_and_requirements() {
        let e = sample();
        let items = sample_items();
        let mut sword = ItemStack::new("minecraft:diamond_sword", 1);
        sword.enchant("minecraft:sharpness", 5);
        let mut r = LegacyRandom::new(1);
        let ctx = EffectContext {
            items: Some(&items),
            this: Some("minecraft:zombie"),
            ..Default::default()
        };
        assert_eq!(
            e.modify_value(Some(&sword), "minecraft:damage", &ctx, &mut r, 0.0),
            3.0
        );
        let mut smite = ItemStack::new("minecraft:diamond_sword", 1);
        smite.enchant("minecraft:smite", 2);
        assert_eq!(
            e.modify_value(Some(&smite), "minecraft:damage", &ctx, &mut r, 0.0),
            5.0
        );
        let pig = EffectContext {
            this: Some("minecraft:pig"),
            ..ctx
        };
        assert_eq!(
            e.modify_value(Some(&smite), "minecraft:damage", &pig, &mut r, 0.0),
            0.0
        );
        let mut boots = ItemStack::new("minecraft:iron_boots", 1);
        boots.enchant("minecraft:feather_falling", 4);
        let fall = EffectContext {
            damage_type: Some("minecraft:fall"),
            ..Default::default()
        };
        assert_eq!(
            e.modify_value(
                Some(&boots),
                "minecraft:damage_protection",
                &fall,
                &mut r,
                0.0
            ),
            12.0
        );
        let lava = EffectContext {
            damage_type: Some("minecraft:lava"),
            ..Default::default()
        };
        assert_eq!(
            e.modify_value(
                Some(&boots),
                "minecraft:damage_protection",
                &lava,
                &mut r,
                0.0
            ),
            0.0
        );
        // Unbreaking III keeps about 1/4 of durability hits.
        let mut tool = ItemStack::new("minecraft:diamond_sword", 1);
        tool.enchant("minecraft:unbreaking", 3);
        let kept: f32 = (0..4000)
            .map(|_| e.modify_value(Some(&tool), "minecraft:item_damage", &ctx, &mut r, 1.0))
            .sum();
        assert!((800.0..1200.0).contains(&kept), "{kept}");
    }

    #[test]
    fn names_and_tooltip_order() {
        let e = sample();
        assert_eq!(e.full_name("minecraft:mending", 1).0, "mending");
        assert_eq!(e.full_name("minecraft:sharpness", 1).0, "sharpness I");
        // PvP & HUD's numbers: any level, past vanilla's numerals too; a one-level
        // enchantment still has none.
        let number = |id, level| e.full_name_with(None, id, level, true).0;
        assert_eq!(number("minecraft:sharpness", 5), "sharpness 5");
        assert_eq!(number("minecraft:sharpness", 255), "sharpness 255");
        assert_eq!(number("minecraft:mending", 1), "mending");
        let levels = vec![
            (Arc::from("minecraft:unbreaking"), 3),
            (Arc::from("minecraft:mending"), 1),
            (Arc::from("minecraft:sharpness"), 2),
        ];
        let lines: Vec<String> = e.tooltip(&levels).into_iter().map(|l| l.0).collect();
        assert_eq!(lines, ["sharpness II", "unbreaking III", "mending"]);
        assert!(!e.compatible("minecraft:sharpness", "minecraft:smite"));
        assert!(e.compatible("minecraft:sharpness", "minecraft:unbreaking"));
    }

    #[test]
    fn high_enchantment_names_keep_the_untranslated_key() {
        let mut e = sample();
        assert_eq!(
            e.full_name_in(None, "minecraft:sharpness", 255).0,
            "sharpness enchantment.level.255"
        );
        // A pack may supply names beyond vanilla's I through X, including
        // the English fallback used when no client language is loaded.
        e.lang.insert("enchantment.level.11".into(), "XI".into());
        assert_eq!(
            e.full_name_in(None, "minecraft:sharpness", 11).0,
            "sharpness XI"
        );
    }

    /// A server's data-pack enchantment the game lacks is named by the
    /// `description` it sent (`Enchantment.getFullname`), with the level
    /// numeral unless both the level and the maximum are 1; the game's own
    /// entries keep their tables.
    #[test]
    fn synced_entries_name_themselves() {
        let e = sample();
        assert_eq!(e.full_name("enchex:drill", 2).0, "enchex:drill II");
        let drill = json!({"description":{"text":"Drill"},"max_level":3,"supported_items":"#minecraft:enchantable/mining"});
        let hex = json!({"description":"Hex","max_level":1});
        let other = json!({"description":{"text":"Other"},"max_level":5});
        let synced = e
            .with_entries([
                ("enchex:drill", &drill),
                ("enchex:hex", &hex),
                ("minecraft:sharpness", &other),
            ])
            .unwrap()
            .unwrap();
        assert_eq!(synced.full_name("enchex:drill", 2).0, "Drill II");
        assert_eq!(synced.full_name("enchex:drill", 1).0, "Drill I");
        assert_eq!(synced.full_name("enchex:hex", 1).0, "Hex");
        assert_eq!(synced.full_name("minecraft:sharpness", 1).0, "sharpness I");
        assert_eq!(synced.list.len(), e.list.len() + 2);
        // Nothing new: the game's table stays in use.
        assert!(
            e.with_entries([("minecraft:sharpness", &other)])
                .unwrap()
                .is_none()
        );
    }

    /// The enchantment's name and its level come from the selected language,
    /// with the English name loaded above as the fallback.
    #[test]
    fn names_follow_the_language() {
        struct Mem(HashMap<String, Vec<u8>>);
        impl horizon_client_core::lang::ResourceSource for Mem {
            fn read(&self, path: &str) -> Option<Vec<u8>> {
                self.0.get(path).cloned()
            }
        }
        let ru = Mem(HashMap::from([(
            "assets/minecraft/lang/ru_ru.json".to_owned(),
            r#"{"enchantment.minecraft.sharpness":"Острота","enchantment.level.5":"V"}"#
                .as_bytes()
                .to_vec(),
        )]));
        let ru = horizon_client_core::lang::Language::load(&[&ru], "ru_ru");
        let e = sample();
        assert_eq!(
            e.full_name_in(Some(&ru), "minecraft:sharpness", 5).0,
            "Острота V"
        );
        // mending carries no key in the fixture: the English name stays.
        assert_eq!(
            e.full_name_in(Some(&ru), "minecraft:mending", 1).0,
            "mending"
        );
    }
}

#[cfg(test)]
mod ignite_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn false_conditions_do_not_suppress_enchantment_tag_operations() {
        let root = std::env::temp_dir().join(format!(
            "horizon-enchantment-tag-conditions-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(root.join("data/minecraft/tags/enchantment")).unwrap();
        std::fs::write(root.join("items.json"), "{}").unwrap();
        std::fs::write(root.join("data/minecraft/tags/enchantment/conditional.json"), json!({"neoforge:conditions":[{"type":"neoforge:false"}],"values":["minecraft:sharpness"]}).to_string()).unwrap();
        let files = vec![("tags/minecraft:enchantment/conditional".into(), vec![json!({"neoforge:conditions":[{"type":"neoforge:false"}],"values":["minecraft:smite"],"remove":["minecraft:sharpness"]}).to_string().into_bytes()])];
        let defaults = horizon_world::item_defaults::read(&root).unwrap();
        let base = Enchantments::load(&root, &defaults).unwrap();
        assert_eq!(
            base.tag("minecraft:conditional")
                .iter()
                .map(|id| id.as_ref())
                .collect::<Vec<_>>(),
            ["minecraft:sharpness"]
        );
        let loaded = Enchantments::load_with(
            &root,
            &horizon_world::loot::PackOverlay::new(&files),
            &defaults,
        )
        .unwrap();
        assert_eq!(
            loaded
                .tag("minecraft:conditional")
                .iter()
                .map(|id| id.as_ref())
                .collect::<Vec<_>>(),
            ["minecraft:smite"]
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    /// Fire Aspect as the data pack has it: 4 seconds per level.
    fn registry() -> Enchantments {
        let fire_aspect = Enchantment {
            native_post_attack: HashMap::new(),
            id: Arc::from("minecraft:fire_aspect"),
            description: "fire_aspect".into(),
            description_key: String::new(),
            max_level: 2,
            weight: 2,
            anvil_cost: 4,
            min_cost: Cost::parse(&json!({"base":10,"per_level_above_first":20})),
            max_cost: Cost::parse(&json!({"base":60,"per_level_above_first":20})),
            supported: vec![],
            primary: None,
            exclusive: HashSet::new(),
            slots: vec![],
            effects: json!({"minecraft:post_attack":[{"effect":{"type":"minecraft:ignite","duration":{"type":"minecraft:linear","base":4.0,"per_level_above_first":4.0}},"enchanted":"attacker","affected":"victim"}]}),
        };
        let index = [(fire_aspect.id.clone(), 0)].into_iter().collect();
        Enchantments {
            list: vec![fire_aspect],
            index,
            ..Default::default()
        }
    }

    #[test]
    fn weight_totals_enchantments() {
        for weights in [
            [0, 0, 0],
            [1, 2, 3],
            [7, 0, 11],
            [i32::MAX as u32; 3],
            [u32::MAX; 3],
        ] {
            let mut reg = registry();
            let template = reg.list[0].clone();
            reg.list.clear();
            reg.index.clear();
            let mut list = vec![];
            for (i, weight) in weights.into_iter().enumerate() {
                let mut e = template.clone();
                e.id = Arc::from(format!("test:{i}"));
                e.weight = weight;
                list.push((e.id.clone(), 1));
                reg.index.insert(e.id.clone(), i);
                reg.list.push(e);
            }
            let total: i64 = weights.iter().map(|&w| i64::from(w)).sum();
            for seed in 0..64 {
                let mut expected = LegacyRandom::new(seed);
                let pick = if total == 0 {
                    None
                } else {
                    let mut at = if total <= i64::from(i32::MAX) {
                        i64::from(expected.next_int_bound(total as i32).unwrap())
                    } else {
                        ((expected.next_long() as u64) % total as u64) as i64
                    };
                    weights
                        .iter()
                        .position(|&w| {
                            at -= i64::from(w);
                            at < 0
                        })
                        .map(|i| list[i].clone())
                };
                let mut random = LegacyRandom::new(seed);
                assert_eq!(reg.weighted(&mut random, &list), pick);
                assert_eq!(random.next_long(), expected.next_long());
            }
            assert!(reg.weighted(&mut LegacyRandom::new(0), &[]).is_none());
        }
    }

    #[test]
    fn fire_aspect_ignites_for_four_seconds_a_level() {
        let reg = registry();
        let mut random = LegacyRandom::new(0);
        let ctx = EffectContext {
            this: Some("minecraft:player"),
            ..Default::default()
        };
        let mut fire = |level| {
            let held = [(Arc::from("minecraft:fire_aspect"), level)];
            reg.ignite_seconds(&held, &ctx, &mut random)
        };
        assert_eq!(fire(1) * 20.0, 80.0);
        assert_eq!(fire(2) * 20.0, 160.0);
        assert_eq!(reg.ignite_seconds(&[], &ctx, &mut random), 0.0);
    }
}
