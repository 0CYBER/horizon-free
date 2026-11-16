//! Biome modifiers: a content mod's placed features and mob spawns in biomes
//! it does not own (vanilla's), as NeoForge's `add_features`,
//! `remove_features`, `add_spawns`, `remove_spawns`, `add_spawn_costs` and
//! `remove_spawn_costs` biome modifiers do. Each is a file in
//! `data/<ns>/neoforge/biome_modifier/` (older Forge mods:
//! `data/<ns>/forge/biome_modifier/`, type `forge:...`).
//!
//! They rewrite the biomes' `features` lists and
//! `minecraft:gameplay/natural_mob_spawns` attributes when the content root
//! builds: an added feature goes at the end of its step's list, and the generator orders every biome's lists as vanilla's
//! `FeatureSorter` does; an added spawner goes at the end of its entity type's
//! spawn category with `minCount` and `maxCount` converted to 26.3's `count`.
//! Biomes no modifier names keep their text byte for byte, and a root with no
//! modifiers keeps vanilla's bundle, so vanilla features keep their indices
//! and their random seeds.
use super::{J, Result};
use horizon_world::worldgen::structure::start::STEPS;
use serde::Deserialize;
use serde_json::{Number, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// The folders a biome modifier sits in, under `data/<ns>`.
const FOLDERS: [&str; 2] = ["/neoforge/biome_modifier/", "/forge/biome_modifier/"];

/// Whether a mod file is a biome modifier.
pub(super) fn is_modifier(file: &str) -> bool {
    file.starts_with("data/") && file.ends_with(".json") && FOLDERS.iter().any(|f| file.contains(f))
}

/// One mob spawner entry in an `add_spawns` modifier.
#[derive(Debug, PartialEq)]
pub(super) struct Spawner {
    pub entity: String,
    weight: i64,
    min_count: i64,
    max_count: i64,
}

impl Spawner {
    /// The canonical 26.3 weighted spawn entry, shared by biome and structure modifiers.
    pub(super) fn entry(&self) -> J {
        let count = if self.min_count == self.max_count {
            J::Num(self.min_count.into())
        } else {
            J::Obj(vec![
                ("type".into(), J::Str("minecraft:uniform".into())),
                ("min_inclusive".into(), J::Num(self.min_count.into())),
                ("max_inclusive".into(), J::Num(self.max_count.into())),
            ])
        };
        J::Obj(vec![
            ("type".into(), J::Str(self.entity.clone())),
            ("count".into(), count),
            ("weight".into(), J::Num(self.weight.into())),
        ])
    }
}

/// What a modifier does to its biomes.
#[derive(Debug, PartialEq)]
enum Action {
    /// Appends features to one generation step.
    AddFeatures { features: Value, step: usize },
    /// Takes features out of these steps (every step when the file names none).
    RemoveFeatures { features: Value, steps: Vec<usize> },
    /// Appends spawners to their entity types' spawn categories.
    AddSpawns(Vec<Spawner>),
    /// Takes these entity types out of every spawn category.
    RemoveSpawns(Value),
    /// Sets `spawn_costs` for these entity types.
    AddSpawnCosts {
        entity_types: Value,
        energy_budget: Number,
        charge: Number,
    },
    /// Takes these entity types out of `spawn_costs`.
    RemoveSpawnCosts(Value),
    /// Appends configured carvers to the biome's `carvers` (`add_carvers`).
    AddCarvers { carvers: Value },
    /// Takes configured carvers out of the biome's `carvers` (`remove_carvers`).
    RemoveCarvers { carvers: Value },
    /// Sets single values of the biome, in the MODIFY phase (`horizon:modify_biome`).
    Modify(Changes),
}

/// The single values a `horizon:modify_biome` modifier sets; what it leaves out stays.
#[derive(Debug, Default, PartialEq)]
pub(super) struct Changes {
    temperature: Option<Number>,
    downfall: Option<Number>,
    has_precipitation: Option<bool>,
    temperature_modifier: Option<String>,
    /// `effects` colours by name, as 24-bit RGB; `None` removes the override.
    colours: Vec<(&'static str, Option<u32>)>,
    /// Whole `attributes` values (ambient particles, ambient sounds, music), checked.
    attributes: Vec<(&'static str, Value)>,
}

/// The `attributes` that `horizon:modify_biome` sets, each replacing the biome's value.
const ATTRIBUTES: [&str; 4] = [
    "minecraft:visual/ambient_particles",
    "minecraft:audio/ambient_sounds",
    "minecraft:audio/background_music",
    "minecraft:audio/music_volume",
];

/// A namespaced id (`minecraft:ash`).
fn id(s: &str) -> bool {
    s.split_once(':')
        .is_some_and(|(ns, path)| !ns.is_empty() && !path.is_empty())
}

/// A whole number of ticks or blocks, as an `i32`.
fn count(v: Option<&Value>) -> Option<u64> {
    v?.as_u64().filter(|n| i32::try_from(*n).is_ok())
}

/// A chance or volume, 0.0..=1.0.
fn unit(v: Option<&Value>) -> bool {
    v.and_then(Value::as_f64)
        .is_some_and(|n| (0.0..=1.0).contains(&n))
}

/// One attribute's value as vanilla's biome JSON writes it, or why it is refused. Ambient
/// particles may come in the `{"argument": [...]}` form; the list is what is kept.
fn attribute(key: &str, v: &Value) -> std::result::Result<Value, String> {
    let sound = |m: &Value| m.get("sound").and_then(Value::as_str).is_some_and(id);
    let bad = |what: &str| format!("attributes.{key}: {what}");
    match key {
        "minecraft:visual/ambient_particles" => {
            let list = v.get("argument").unwrap_or(v);
            let entries = list.as_array().ok_or_else(|| bad("not a list"))?;
            let entry = |e: &Value| {
                let particle = e["particle"]["type"].as_str();
                particle.is_some_and(id) && unit(e.get("probability"))
            };
            match entries.iter().all(entry) {
                true => Ok(list.clone()),
                false => Err(bad("a particle needs a type and a probability 0..=1")),
            }
        }
        "minecraft:audio/ambient_sounds" => {
            let object = v.as_object().ok_or_else(|| bad("not an object"))?;
            let addition = |a: &Value| {
                sound(a)
                    && a.get("tick_chance")
                        .and_then(Value::as_f64)
                        .is_some_and(f64::is_finite)
            };
            for (name, value) in object {
                let ok = match name.as_str() {
                    "loop" => value.as_str().is_some_and(id),
                    "additions" => match value {
                        Value::Array(list) => list.iter().all(addition),
                        one => addition(one),
                    },
                    "mood" => {
                        sound(value)
                            && count(value.get("tick_delay")).is_some()
                            && count(value.get("block_search_extent")).is_some()
                            && value.get("offset").is_some_and(Value::is_number)
                    }
                    _ => false,
                };
                if !ok {
                    return Err(bad(&format!("{name} is not valid")));
                }
            }
            Ok(v.clone())
        }
        "minecraft:audio/background_music" => {
            let object = v.as_object().ok_or_else(|| bad("not an object"))?;
            for (name, music) in object {
                let (min, max) = (count(music.get("min_delay")), count(music.get("max_delay")));
                let ok = matches!(name.as_str(), "default" | "creative" | "underwater")
                    && sound(music)
                    && min.is_some()
                    && max.is_some();
                if !ok {
                    return Err(bad(&format!("{name} needs a sound and nonnegative delays")));
                }
            }
            Ok(v.clone())
        }
        _ => match unit(Some(v)) {
            true => Ok(v.clone()),
            false => Err(bad("not a number 0..=1")),
        },
    }
}

/// The colours of `effects` that `horizon:modify_biome` sets.
const COLOURS: [&str; 7] = [
    "sky_color",
    "fog_color",
    "water_color",
    "water_fog_color",
    "foliage_color",
    "dry_foliage_color",
    "grass_color",
];

/// The colours that are optional in 26.3's `BiomeSpecialEffects`: a `null` removes them. The
/// others are required or are attributes, so they cannot be removed.
const REMOVABLE: [&str; 3] = ["foliage_color", "dry_foliage_color", "grass_color"];

/// A colour as an integer 0..=0xFFFFFF or a `#rrggbb` string.
fn colour(v: &Value) -> Option<u32> {
    match v {
        Value::Number(n) => n
            .as_u64()
            .filter(|c| *c <= 0xFF_FFFF)
            .and_then(|c| u32::try_from(c).ok()),
        Value::String(s) => s
            .strip_prefix('#')
            .filter(|h| h.len() == 6 && h.bytes().all(|b| b.is_ascii_hexdigit()))
            .and_then(|h| u32::from_str_radix(h, 16).ok()),
        _ => None,
    }
}

impl Changes {
    /// The values in a `horizon:modify_biome` file, vanilla biome JSON's names.
    fn read(doc: &Value) -> std::result::Result<Self, String> {
        let num = |k: &str| match doc.get(k) {
            None => Ok(None),
            Some(Value::Number(n)) => Ok(Some(n.clone())),
            Some(_) => Err(format!("{k}: not a number")),
        };
        let has_precipitation = match doc.get("has_precipitation") {
            None => None,
            Some(Value::Bool(b)) => Some(*b),
            Some(_) => return Err("has_precipitation: not a boolean".into()),
        };
        let temperature_modifier = match doc.get("temperature_modifier") {
            None => None,
            Some(Value::String(s)) if s == "none" || s == "frozen" => Some(s.clone()),
            Some(_) => return Err("temperature_modifier: not none or frozen".into()),
        };
        let mut colours = Vec::new();
        match doc.get("effects") {
            None => {}
            Some(Value::Object(effects)) => {
                for (key, value) in effects {
                    let name = COLOURS
                        .iter()
                        .find(|c| *c == key)
                        .ok_or_else(|| format!("effects: unknown colour {key}"))?;
                    let rgb =
                        match value {
                            Value::Null if REMOVABLE.contains(name) => None,
                            Value::Null => return Err(format!("effects.{key}: cannot be removed")),
                            value => Some(colour(value).ok_or_else(|| {
                                format!("effects.{key}: not a colour 0..=0xFFFFFF")
                            })?),
                        };
                    colours.push((*name, rgb));
                }
            }
            Some(_) => return Err("effects: not an object".into()),
        }
        let mut attributes = Vec::new();
        match doc.get("attributes") {
            None => {}
            Some(Value::Object(map)) => {
                for (key, value) in map {
                    let name = ATTRIBUTES
                        .iter()
                        .find(|a| *a == key)
                        .ok_or_else(|| format!("attributes: unknown attribute {key}"))?;
                    attributes.push((*name, attribute(key, value)?));
                }
            }
            Some(_) => return Err("attributes: not an object".into()),
        }
        Ok(Self {
            temperature: num("temperature")?,
            downfall: num("downfall")?,
            has_precipitation,
            temperature_modifier,
            colours,
            attributes,
        })
    }
}

impl Action {
    fn is_modify(&self) -> bool {
        matches!(self, Self::Modify(_))
    }

    fn is_add(&self) -> bool {
        matches!(
            self,
            Self::AddFeatures { .. }
                | Self::AddSpawns(_)
                | Self::AddSpawnCosts { .. }
                | Self::AddCarvers { .. }
        )
    }
}

/// One biome modifier file.
#[derive(Debug)]
pub(super) struct Modifier {
    /// `<mod>: <file>`, for errors.
    name: String,
    biomes: Selector,
    action: Action,
    /// The mod, the file and its text, kept in the content root so the
    /// modifier is applied again over a world's data packs (`over_packs`).
    pub source: [String; 3],
}

/// A generation step's index from its name.
fn step(v: &Value) -> std::result::Result<usize, String> {
    let name = v.as_str().ok_or("step: not a name")?;
    STEPS
        .iter()
        .position(|s| *s == name)
        .ok_or_else(|| format!("unknown generation step {name:?}"))
}

/// One NeoForge `SpawnerData` object from `spawners`.
pub(super) fn spawner(v: &Value) -> std::result::Result<Spawner, String> {
    let o = v.as_object().ok_or("spawner: not an object")?;
    let entity = o
        .get("type")
        .and_then(Value::as_str)
        .ok_or("spawner: no type")?;
    let int = |k: &str| {
        o.get(k)
            .and_then(Value::as_i64)
            .filter(|value| i32::try_from(*value).is_ok())
            .ok_or_else(|| format!("spawner: no {k}"))
    };
    let weight = int("weight")?;
    let min_count = int("minCount")?;
    let max_count = int("maxCount")?;
    if weight < 0 {
        return Err(format!("spawner: weight ({weight}) < 0"));
    }
    if min_count <= 0 {
        return Err(format!("spawner: minCount ({min_count}) must be positive"));
    }
    if min_count > max_count {
        return Err(format!(
            "spawner: minCount ({min_count}) > maxCount ({max_count})"
        ));
    }
    Ok(Spawner {
        entity: full(entity),
        weight,
        min_count,
        max_count,
    })
}

impl Modifier {
    /// The modifier in `text`; `None` for `neoforge:none`, which does nothing,
    /// and for a type Horizon doesn't read yet (a mod's own
    /// serializer), which is skipped with a line in `warnings`: real mods ship
    /// those beside their feature and spawn modifiers, and one must not drop
    /// every content mod.
    pub(super) fn read(
        mod_id: &str,
        file: &str,
        text: &str,
        warnings: &mut Vec<String>,
    ) -> Result<Option<Self>> {
        let name = format!("mod {mod_id}: {file}");
        let at = |e: String| format!("{name}: {e}");
        let doc: Value = serde_json::from_str(text).map_err(|e| at(e.to_string()))?;
        let kind = doc["type"].as_str().ok_or_else(|| at("no type".into()))?;
        let kind = kind
            .strip_prefix("neoforge:")
            .or_else(|| kind.strip_prefix("forge:"))
            .unwrap_or(kind);
        let field = |key: &str| -> std::result::Result<Value, String> {
            doc.get(key).cloned().ok_or_else(|| at(format!("no {key}")))
        };
        let action = match kind {
            "none" => return Ok(None),
            "add_features" => Action::AddFeatures {
                features: field("features")?,
                step: step(&field("step")?).map_err(at)?,
            },
            "remove_features" => Action::RemoveFeatures {
                features: field("features")?,
                steps: match doc.get("steps") {
                    None => (0..STEPS.len()).collect(),
                    Some(Value::Array(steps)) => steps
                        .iter()
                        .map(step)
                        .collect::<std::result::Result<_, _>>()
                        .map_err(at)?,
                    Some(one) => vec![step(one).map_err(at)?],
                },
            },
            "add_spawns" => {
                let raw = field("spawners")?;
                let spawners = match &raw {
                    Value::Array(list) => list
                        .iter()
                        .map(spawner)
                        .collect::<std::result::Result<_, _>>()
                        .map_err(at)?,
                    one @ Value::Object(_) => vec![spawner(one).map_err(at)?],
                    _ => return Err(at("spawners: not a spawner or list".into()).into()),
                };
                Action::AddSpawns(spawners)
            }
            "remove_spawns" => Action::RemoveSpawns(field("entity_types")?),
            "add_spawn_costs" => {
                let entity_types = field("entity_types")?;
                let cost = field("spawn_cost")?;
                let num = |k: &str| {
                    cost.get(k)
                        .and_then(Value::as_number)
                        .cloned()
                        .ok_or_else(|| at(format!("spawn_cost: no {k}")))
                };
                Action::AddSpawnCosts {
                    entity_types,
                    energy_budget: num("energy_budget")?,
                    charge: num("charge")?,
                }
            }
            "remove_spawn_costs" => Action::RemoveSpawnCosts(field("entity_types")?),
            "add_carvers" => Action::AddCarvers {
                carvers: field("carvers")?,
            },
            "remove_carvers" => Action::RemoveCarvers {
                carvers: field("carvers")?,
            },
            "horizon:modify_biome" => match Changes::read(&doc) {
                Ok(changes) => Action::Modify(changes),
                Err(e) => {
                    warnings.push(at(format!("{e}: skipped")));
                    return Ok(None);
                }
            },
            other => {
                warnings.push(at(format!(
                    "biome modifier type {other:?} skipped: only feature, spawn, carver, \
                     horizon:modify_biome and none modifiers are read"
                )));
                return Ok(None);
            }
        };
        Ok(Some(Self {
            biomes: Selector::read(field("biomes")?, true).map_err(at)?,
            action,
            name,
            source: [mod_id.to_owned(), file.to_owned(), text.to_owned()],
        }))
    }
}

/// `ns:path` for an id that may leave out `minecraft:`.
fn full(id: &str) -> String {
    match id.contains(':') {
        true => id.to_owned(),
        false => format!("minecraft:{id}"),
    }
}

/// The registries the modifiers name entries of.
pub(super) struct Registries<'a> {
    pub biome: Registry<'a>,
    pub feature: Registry<'a>,
    pub entity: Registry<'a>,
    pub carver: Registry<'a>,
}

impl<'a> Registries<'a> {
    /// The registries a content build or a world open names entries of.
    pub(super) fn of(
        biome_ids: &'a BTreeSet<String>,
        biome_tags: &'a BTreeMap<String, String>,
        placed: &'a BTreeSet<String>,
        feature_tags: &'a BTreeMap<String, String>,
        entities: (&'a BTreeSet<String>, &'a BTreeMap<String, String>),
        carvers: (&'a BTreeSet<String>, &'a BTreeMap<String, String>),
    ) -> Self {
        let registry = |what, ids, tags| Registry { what, ids, tags };
        Self {
            biome: registry("biome", biome_ids, biome_tags),
            feature: registry("placed feature", placed, feature_tags),
            entity: registry("entity type", entities.0, entities.1),
            carver: registry("carver", carvers.0, carvers.1),
        }
    }
}

/// The ids of the configured carvers in the bundle `path` (none when it is absent).
pub(super) fn carver_ids(path: &Path) -> Result<BTreeSet<String>> {
    if !path.exists() {
        return Ok(BTreeSet::new());
    }
    Ok(match super::read_json(path)? {
        J::Obj(entries) => entries.into_iter().map(|(id, _)| id).collect(),
        _ => BTreeSet::new(),
    })
}

/// A registry the modifiers name entries of: its ids and its tags (tag id →
/// the tag file's text), as the worldgen loader's bundles hold them.
pub(super) struct Registry<'a> {
    pub what: &'static str,
    pub ids: &'a BTreeSet<String>,
    pub tags: &'a BTreeMap<String, String>,
}

impl Registry<'_> {
    /// The ids a holder set names, in its order without repeats: an id,
    /// `#tag`, a list of ids, or one of NeoForge's `any`, `or`, `and` and
    /// `not` objects. Unknown ids and tags are errors; inside a tag, entries
    /// nobody registers are left out, as the worldgen loader leaves them out.
    pub(super) fn resolve(&self, set: &Value) -> std::result::Result<Vec<String>, String> {
        Selector::read(set.clone(), false)?.resolve(self, None)
    }

    /// A set with no climate in it, through the shared holder-set decoder.
    fn resolve_leaf(&self, set: &Value) -> std::result::Result<Vec<String>, String> {
        horizon_world::holder_set::resolve(set, self)
    }

    // Determine the edge to drop from the graph, not from traversal order.
    fn removal_reaches(&self, start: &str, goal: &str) -> bool {
        let mut pending = vec![start.to_owned()];
        let mut visited = BTreeSet::new();
        while let Some(tag) = pending.pop() {
            if tag == goal {
                return true;
            }
            if !visited.insert(tag.clone()) {
                continue;
            }
            let Some(doc) = self
                .tags
                .get(&tag)
                .and_then(|text| serde_json::from_str::<Value>(text).ok())
            else {
                continue;
            };
            for value in doc["remove"].as_array().into_iter().flatten().chain(
                doc["values"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|v| v["remove"].as_bool() == Some(true)),
            ) {
                if let Some(inner) =
                    horizon_world::tag_layers::entry_id(value).and_then(|id| id.strip_prefix('#'))
                {
                    pending.push(full(inner));
                }
            }
        }
        false
    }

    fn expand(
        &self,
        tag: &str,
        out: &mut Vec<String>,
        seen: &mut BTreeSet<String>,
    ) -> std::result::Result<(), String> {
        if !seen.insert(tag.to_owned()) {
            return Ok(());
        }
        let text = self
            .tags
            .get(tag)
            .ok_or_else(|| format!("unknown {} tag #{tag}", self.what))?;
        let doc: Value = serde_json::from_str(text).map_err(|e| format!("#{tag}: {e}"))?;
        let mut own = Vec::new();
        let operations = doc["values"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|v| (v, v["remove"].as_bool() == Some(true)))
            .chain(
                doc["remove"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|v| (v, true)),
            );
        for (value, removes) in operations {
            let Some(id) = horizon_world::tag_layers::entry_id(value) else {
                continue;
            };
            let required = !removes && value["required"].as_bool() != Some(false);
            let mut members = Vec::new();
            if let Some(inner) = id.strip_prefix('#') {
                let target = full(inner);
                if removes && target.as_str() > tag && self.removal_reaches(&target, tag) {
                    log::warn!(
                        "Horizon: tags {tag} and {target} form a removal cycle; only removal edge {tag} -> {target} is ignored"
                    );
                    continue;
                }
                if let Err(error) = self.expand(&target, &mut members, seen) {
                    if required {
                        return Err(error);
                    }
                    continue;
                }
            } else if self.ids.contains(&full(id)) {
                members.push(full(id));
            }
            if removes {
                own.retain(|id| !members.contains(id));
            } else {
                extend(&mut own, members);
            }
        }
        seen.remove(tag);
        extend(out, own);
        Ok(())
    }
}

impl horizon_world::holder_set::HolderLookup for Registry<'_> {
    type Id = String;
    fn what(&self) -> &str {
        self.what
    }
    fn entry(&self, id: &str) -> std::result::Result<Vec<String>, String> {
        let id = full(id);
        match self.ids.contains(&id) {
            true => Ok(vec![id]),
            false => Err(format!("unknown {} {id}", self.what)),
        }
    }
    fn tag(&self, tag: &str) -> std::result::Result<Vec<String>, String> {
        let mut out = Vec::new();
        self.expand(&full(tag), &mut out, &mut BTreeSet::new())?;
        Ok(out)
    }
    fn all(&self) -> std::result::Result<Vec<String>, String> {
        Ok(self.ids.iter().cloned().collect())
    }
}

/// Appends the ids `out` lacks.
fn extend(out: &mut Vec<String>, ids: impl IntoIterator<Item = String>) {
    for id in ids {
        if !out.contains(&id) {
            out.push(id);
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Range {
    #[serde(default, deserialize_with = "present")]
    min: Option<f64>,
    #[serde(default, deserialize_with = "present")]
    max: Option<f64>,
}

fn present<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> std::result::Result<Option<T>, D::Error> {
    T::deserialize(deserializer).map(Some)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Climate {
    #[serde(rename = "type")]
    _kind: String,
    #[serde(default, deserialize_with = "present")]
    temperature: Option<Range>,
    #[serde(default, deserialize_with = "present")]
    downfall: Option<Range>,
}

#[derive(Debug)]
enum Selector {
    Generic(Value),
    Climate(Climate),
    Or(Vec<Selector>),
    And(Vec<Selector>),
    Not(Box<Selector>),
}

impl Selector {
    fn read(mut set: Value, allow_climate: bool) -> std::result::Result<Self, String> {
        let Value::Object(object) = &mut set else {
            return Ok(Self::Generic(set));
        };
        match object.get("type").and_then(Value::as_str) {
            Some("horizon:climate") if allow_climate => {
                let climate: Climate = serde_json::from_value(set).map_err(|e| e.to_string())?;
                if climate.temperature.is_none() && climate.downfall.is_none() {
                    return Err("climate: temperature or downfall required".into());
                }
                for range in [&climate.temperature, &climate.downfall]
                    .into_iter()
                    .flatten()
                {
                    if range.min.is_none() && range.max.is_none()
                        || matches!((range.min, range.max), (Some(a), Some(b)) if a > b)
                    {
                        return Err("climate: invalid range".into());
                    }
                }
                Ok(Self::Climate(climate))
            }
            Some("neoforge:or" | "or" | "neoforge:and" | "and")
                if object.get("values").is_some_and(Value::is_array) =>
            {
                let Value::Array(values) = object.remove("values").unwrap() else {
                    unreachable!()
                };
                let is_or = matches!(
                    object.get("type").and_then(Value::as_str),
                    Some("or" | "neoforge:or")
                );
                let values = values
                    .into_iter()
                    .map(|v| Self::read(v, allow_climate))
                    .collect::<std::result::Result<_, _>>()?;
                Ok(if is_or {
                    Self::Or(values)
                } else {
                    Self::And(values)
                })
            }
            Some("neoforge:not" | "not") if object.contains_key("value") => {
                let value = object.remove("value").unwrap();
                Ok(Self::Not(Box::new(Self::read(value, allow_climate)?)))
            }
            _ => Ok(Self::Generic(set)),
        }
    }

    fn resolve(
        &self,
        registry: &Registry<'_>,
        biomes: Option<&J>,
    ) -> std::result::Result<Vec<String>, String> {
        match self {
            Self::Generic(set) => registry.resolve_leaf(set),
            Self::Climate(climate) => {
                let Some(J::Obj(entries)) = biomes else {
                    return Err("biome bundle is not an object".into());
                };
                let mut selected = Vec::new();
                for (id, text) in entries {
                    let J::Str(text) = text else { continue };
                    let biome: Value =
                        serde_json::from_str(text).map_err(|e| format!("{id}: {e}"))?;
                    let mut matches = true;
                    for (field, range) in [
                        ("temperature", &climate.temperature),
                        ("downfall", &climate.downfall),
                    ] {
                        if let Some(range) = range {
                            let value = biome[field]
                                .as_f64()
                                .ok_or_else(|| format!("{id}: no {field}"))?;
                            matches &= range.min.is_none_or(|min| value >= min)
                                && range.max.is_none_or(|max| value <= max);
                        }
                    }
                    if matches {
                        selected.push(id.clone());
                    }
                }
                Ok(selected)
            }
            Self::Or(values) => {
                let mut out = Vec::new();
                for value in values {
                    extend(&mut out, value.resolve(registry, biomes)?);
                }
                Ok(out)
            }
            Self::And(values) => {
                let mut values = values.iter();
                let mut out = match values.next() {
                    Some(value) => value.resolve(registry, biomes)?,
                    None => Vec::new(),
                };
                for value in values {
                    let ids = value.resolve(registry, biomes)?;
                    out.retain(|id| ids.contains(id));
                }
                Ok(out)
            }
            Self::Not(value) => {
                let excluded = value.resolve(registry, biomes)?;
                Ok(registry
                    .ids
                    .iter()
                    .filter(|id| !excluded.contains(id))
                    .cloned()
                    .collect())
            }
        }
    }
}

/// Applies `modifiers` to the biome bundle `biomes` (id → the biome's text):
/// every addition first, then every removal, then every `horizon:modify_biome`, each in mod and file order. A modifier naming a
/// biome, feature, entity type or tag the root doesn't have (another mod's, a
/// `#c:` convention tag) is skipped with a line in `warnings`, instead of dropping every content mod.
pub(super) fn apply(
    biomes: &mut J,
    modifiers: &[Modifier],
    registries: &Registries<'_>,
    entity_categories: &BTreeMap<String, String>,
    warnings: &mut Vec<String>,
) -> Result<Vec<(String, String)>> {
    let Registries {
        biome: biome_reg,
        feature: feature_reg,
        entity: entity_reg,
        carver: carver_reg,
    } = registries;
    let adds = modifiers.iter().filter(|m| m.action.is_add());
    let removes = modifiers
        .iter()
        .filter(|m| !m.action.is_add() && !m.action.is_modify());
    // NeoForge's MODIFY phase follows ADD and REMOVE.
    let modifies = modifiers.iter().filter(|m| m.action.is_modify());
    // Each biome changed, parsed once, in the bundle's order.
    let mut changed: BTreeMap<String, J> = BTreeMap::new();
    for m in adds.chain(removes).chain(modifies) {
        let at = |e: String| format!("{}: {e}", m.name);
        let resolved: std::result::Result<(Vec<String>, Vec<String>), String> = m
            .biomes
            .resolve(biome_reg, Some(biomes))
            .and_then(|targets| {
                let ids = match &m.action {
                    Action::AddFeatures { features, .. }
                    | Action::RemoveFeatures { features, .. } => feature_reg.resolve(features)?,
                    Action::AddCarvers { carvers } | Action::RemoveCarvers { carvers } => {
                        carver_reg.resolve(carvers)?
                    }
                    Action::Modify(_) => Vec::new(),
                    Action::AddSpawns(spawners) => {
                        if let Some(s) = spawners
                            .iter()
                            .find(|s| !entity_reg.ids.contains(&s.entity))
                        {
                            return Err(format!("unknown {} {}", entity_reg.what, s.entity));
                        }
                        Vec::new()
                    }
                    Action::RemoveSpawns(entity_types)
                    | Action::AddSpawnCosts { entity_types, .. }
                    | Action::RemoveSpawnCosts(entity_types) => entity_reg.resolve(entity_types)?,
                };
                Ok((targets, ids))
            });
        let (targets, ids) = match resolved {
            Ok(both) => both,
            Err(e) => {
                warnings.push(at(format!("{e}: skipped")));
                continue;
            }
        };
        if matches!(&m.action, Action::AddSpawns(s) if s.is_empty()) {
            continue;
        }
        for biome in targets {
            let doc = match changed.entry(biome.clone()) {
                std::collections::btree_map::Entry::Occupied(e) => e.into_mut(),
                std::collections::btree_map::Entry::Vacant(e) => {
                    let Some(J::Str(text)) = biomes.get(&biome) else {
                        continue;
                    };
                    e.insert(serde_json::from_str(text).map_err(|e| at(format!("{biome}: {e}")))?)
                }
            };
            if matches!(
                m.action,
                Action::AddCarvers { .. } | Action::RemoveCarvers { .. }
            ) {
                super::carver_registry::expand(doc, carver_reg)?;
            }
            edit(doc, &m.action, &ids, entity_categories)
                .map_err(|e| at(format!("{biome}: {e}")))?;
        }
    }
    let mut out = Vec::with_capacity(changed.len());
    for (id, doc) in changed {
        let text = serde_json::to_string(&doc)?;
        biomes.set(&id, J::Str(text.clone()));
        out.push((id, text));
    }
    Ok(out)
}

/// One modifier's change to one biome.
fn edit(
    biome: &mut J,
    action: &Action,
    ids: &[String],
    entity_categories: &BTreeMap<String, String>,
) -> std::result::Result<(), String> {
    match action {
        Action::AddFeatures { step, .. } => {
            edit_features(biome, std::slice::from_ref(step), true, ids)
        }
        Action::RemoveFeatures { steps, .. } => edit_features(biome, steps, false, ids),
        Action::AddSpawns(spawners) => add_spawns(biome, spawners, entity_categories),
        Action::RemoveSpawns(_) => remove_spawns(biome, ids),
        Action::AddSpawnCosts {
            energy_budget,
            charge,
            ..
        } => add_spawn_costs(biome, ids, energy_budget, charge),
        Action::RemoveSpawnCosts(_) => remove_spawn_costs(biome, ids),
        Action::AddCarvers { .. } => edit_carvers(biome, true, ids),
        Action::RemoveCarvers { .. } => edit_carvers(biome, false, ids),
        Action::Modify(changes) => {
            modify(biome, changes);
            Ok(())
        }
    }
}

/// One `horizon:modify_biome` modifier's change to one biome. In 26.3 the sky, fog and water
/// fog colours are the biome's `minecraft:visual/*` attributes; the others stay in `effects`.
fn modify(biome: &mut J, changes: &Changes) {
    if let Some(n) = &changes.temperature {
        biome.set("temperature", J::Num(n.clone()));
    }
    if let Some(n) = &changes.downfall {
        biome.set("downfall", J::Num(n.clone()));
    }
    if let Some(b) = changes.has_precipitation {
        biome.set("has_precipitation", J::Bool(b));
    }
    if let Some(m) = &changes.temperature_modifier {
        biome.set("temperature_modifier", J::Str(m.clone()));
    }
    for (name, rgb) in &changes.colours {
        let (section, key) = match *name {
            "sky_color" | "fog_color" | "water_fog_color" => {
                ("attributes", format!("minecraft:visual/{name}"))
            }
            _ => ("effects", (*name).to_owned()),
        };
        let Some(rgb) = rgb else {
            if let Some(map) = biome.get_mut(section) {
                map.remove(&key);
            }
            continue;
        };
        if biome.get(section).is_none() {
            biome.set(section, J::Obj(Vec::new()));
        }
        if let Some(map) = biome.get_mut(section) {
            map.set(&key, J::Str(format!("#{rgb:06x}")));
        }
    }
    for (key, value) in &changes.attributes {
        if biome.get("attributes").is_none() {
            biome.set("attributes", J::Obj(Vec::new()));
        }
        // A checked value always converts; the whole value replaces the biome's.
        if let (Some(map), Ok(value)) = (
            biome.get_mut("attributes"),
            serde_json::from_value::<J>(value.clone()),
        ) {
            map.set(key, value);
        }
    }
}

/// One carver modifier's change to one biome's `carvers` (a list of configured carver ids).
fn edit_carvers(biome: &mut J, add: bool, carvers: &[String]) -> std::result::Result<(), String> {
    if biome.get("carvers").is_none() {
        if !add {
            return Ok(());
        }
        biome.set("carvers", J::Arr(Vec::new()));
    }
    let Some(list) = biome.get_mut("carvers") else {
        return Ok(());
    };
    // The list may be one id; a tag cannot take more carvers.
    if let J::Str(one) = list {
        if one.starts_with('#') {
            return Err(format!("carvers is the carver tag {one}"));
        }
        *list = J::Arr(vec![J::Str(one.clone())]);
    }
    let J::Arr(list) = list else {
        return Err("carvers: not a list".into());
    };
    let held = |v: &J| match v {
        J::Str(s) => Some(full(s)),
        _ => None,
    };
    if add {
        for carver in carvers {
            if !list.iter().any(|v| held(v).as_deref() == Some(carver)) {
                list.push(J::Str(carver.clone()));
            }
        }
    } else {
        list.retain(|v| !held(v).is_some_and(|h| carvers.contains(&h)));
    }
    Ok(())
}

/// One feature modifier's change to one biome's `features` (a list per step).
fn edit_features(
    biome: &mut J,
    wanted: &[usize],
    add: bool,
    features: &[String],
) -> std::result::Result<(), String> {
    if biome.get("features").is_none() {
        biome.set("features", J::Arr(Vec::new()));
    }
    let Some(J::Arr(steps)) = biome.get_mut("features") else {
        return Err("features: not a list of steps".into());
    };
    if add && let Some(&step) = wanted.first() {
        while steps.len() <= step {
            steps.push(J::Arr(Vec::new()));
        }
    }
    for &i in wanted {
        let Some(list) = steps.get_mut(i) else {
            continue;
        };
        // A step may be one id; a tag cannot take more features.
        if let J::Str(one) = list {
            if one.starts_with('#') {
                return Err(format!("step {} is the feature tag {one}", STEPS[i]));
            }
            *list = J::Arr(vec![J::Str(one.clone())]);
        }
        let J::Arr(list) = list else {
            return Err(format!("step {}: not a list", STEPS[i]));
        };
        let held = |v: &J| match v {
            J::Str(s) => Some(full(s)),
            _ => None,
        };
        if add {
            for f in features {
                // Twice in one step would be a cycle for the sorter.
                if !list.iter().any(|v| held(v).as_ref() == Some(f)) {
                    list.push(J::Str(f.clone()));
                }
            }
        } else {
            list.retain(|v| held(v).is_none_or(|id| !features.contains(&id)));
        }
    }
    Ok(())
}

/// The `minecraft:gameplay/natural_mob_spawns` argument object on `biome`,
/// creating it when the biome does not yet have one.
fn natural_spawns(biome: &mut J) -> std::result::Result<&mut J, String> {
    if biome.get("attributes").is_none() {
        biome.set("attributes", J::Obj(Vec::new()));
    }
    let Some(attrs @ J::Obj(_)) = biome.get_mut("attributes") else {
        return Err("attributes: not an object".into());
    };
    const KEY: &str = "minecraft:gameplay/natural_mob_spawns";
    if attrs.get(KEY).is_none() {
        attrs.set(
            KEY,
            J::Obj(vec![
                (
                    "argument".into(),
                    J::Obj(vec![
                        ("spawn_costs".into(), J::Obj(Vec::new())),
                        ("spawns_by_category".into(), J::Obj(Vec::new())),
                    ]),
                ),
                ("modifier".into(), J::Str("overlay".into())),
            ]),
        );
    }
    let Some(nat) = attrs.get_mut(KEY) else {
        unreachable!()
    };
    if nat.get("argument").is_some() {
        match nat.get_mut("argument") {
            Some(arg @ J::Obj(_)) => Ok(arg),
            _ => Err(format!("{KEY}.argument: not an object")),
        }
    } else {
        match nat {
            J::Obj(_) => Ok(nat),
            _ => Err(format!("{KEY}: not an object")),
        }
    }
}

fn add_spawns(
    biome: &mut J,
    spawners: &[Spawner],
    entity_categories: &BTreeMap<String, String>,
) -> std::result::Result<(), String> {
    let nat = natural_spawns(biome)?;
    if nat.get("spawns_by_category").is_none() {
        nat.set("spawns_by_category", J::Obj(Vec::new()));
    }
    let Some(by_cat @ J::Obj(_)) = nat.get_mut("spawns_by_category") else {
        return Err("spawns_by_category: not an object".into());
    };
    for s in spawners {
        let cat = entity_categories
            .get(&s.entity)
            .map_or("misc", String::as_str);
        if by_cat.get(cat).is_none() {
            by_cat.set(cat, J::Arr(Vec::new()));
        }
        let Some(J::Arr(list)) = by_cat.get_mut(cat) else {
            return Err(format!("spawns_by_category.{cat}: not a list"));
        };
        list.push(s.entry());
    }
    Ok(())
}

/// Remove entity ids from a canonical weighted spawn list.
pub(super) fn remove_entities(entries: &mut Vec<J>, entities: &[String]) {
    entries.retain(|entry| {
        entry
            .get("type")
            .and_then(J::as_str)
            .is_none_or(|t| !entities.contains(&full(t)))
    });
}

fn remove_spawns(biome: &mut J, entities: &[String]) -> std::result::Result<(), String> {
    let Some(nat) = biome
        .get_mut("attributes")
        .and_then(|a| a.get_mut("minecraft:gameplay/natural_mob_spawns"))
    else {
        return Ok(());
    };
    let nat = if nat.get("argument").is_some() {
        nat.get_mut("argument").unwrap()
    } else {
        nat
    };
    let Some(by_cat) = nat.get_mut("spawns_by_category") else {
        return Ok(());
    };
    let J::Obj(cats) = by_cat else {
        return Err("spawns_by_category: not an object".into());
    };
    for (cat, list) in cats {
        let J::Arr(entries) = list else {
            return Err(format!("spawns_by_category.{cat}: not a list"));
        };
        remove_entities(entries, entities);
    }
    Ok(())
}

fn add_spawn_costs(
    biome: &mut J,
    entities: &[String],
    energy_budget: &Number,
    charge: &Number,
) -> std::result::Result<(), String> {
    let nat = natural_spawns(biome)?;
    if nat.get("spawn_costs").is_none() {
        nat.set("spawn_costs", J::Obj(Vec::new()));
    }
    let Some(costs @ J::Obj(_)) = nat.get_mut("spawn_costs") else {
        return Err("spawn_costs: not an object".into());
    };
    for entity in entities {
        if let J::Obj(entries) = costs {
            entries.retain(|(k, _)| k == entity || full(k) != *entity);
        }
        costs.set(
            entity,
            J::Obj(vec![
                ("energy_budget".into(), J::Num(energy_budget.clone())),
                ("charge".into(), J::Num(charge.clone())),
            ]),
        );
    }
    Ok(())
}

fn remove_spawn_costs(biome: &mut J, entities: &[String]) -> std::result::Result<(), String> {
    let Some(nat) = biome
        .get_mut("attributes")
        .and_then(|a| a.get_mut("minecraft:gameplay/natural_mob_spawns"))
    else {
        return Ok(());
    };
    let nat = if nat.get("argument").is_some() {
        nat.get_mut("argument").unwrap()
    } else {
        nat
    };
    let Some(costs) = nat.get_mut("spawn_costs") else {
        return Ok(());
    };
    let J::Obj(entries) = costs else {
        return Err("spawn_costs: not an object".into());
    };
    entries.retain(|(k, _)| !entities.contains(&full(k)));
    Ok(())
}

/// Writes the state a content build saves for `biome_modifiers_over_packs` (the modifiers and
/// the registries packs leave alone) and, when a file modifier changed the biomes, the biome
/// bundle. Without file modifiers the bundle stands byte for byte; the saved state is what
/// modifiers declared in code are applied with at world open.
pub(super) fn write_state(
    overlay: &super::Overlay<'_>,
    saved: &super::SavedModifiers,
    doc: &J,
    modifiers: &[Modifier],
) -> Result<()> {
    overlay.write(super::MODIFIERS, &serde_json::to_vec(saved)?)?;
    if modifiers.is_empty() {
        return Ok(());
    }
    overlay.write(super::BIOMES, &serde_json::to_vec(doc)?)
}

/// The saved modifier state of the content root `root` (`write_state`), unless a world's packs
/// and the modifiers mods declared in code leave the root's biomes as they are: nothing
/// declared, and no pack brings a biome or a biome tag, or the root saved none.
pub(super) fn saved_bytes(
    root: &std::path::Path,
    packs: &horizon_world::worldgen::overrides::WorldgenOverrides,
    declared: &[[String; 3]],
) -> Option<Vec<u8>> {
    if declared.is_empty()
        && packs.entries("biomes").next().is_none()
        && packs.tags("biome_tags").next().is_none()
        && packs.entries("carvers").next().is_none()
        && packs.tags("carver_tags").next().is_none()
    {
        return None;
    }
    std::fs::read(root.join(super::MODIFIERS)).ok()
}

/// The ids of the biomes the biome modifiers of the content root `root` changed
/// (`write_state`'s `unmodified`): a client has none of their changes in its own files.
pub fn modified(root: &Path) -> BTreeSet<String> {
    let saved = std::fs::read(root.join(super::MODIFIERS))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok());
    saved
        .as_ref()
        .and_then(|saved| saved.get("unmodified")?.as_object())
        .map(|ids| ids.keys().cloned().collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod removal_cycles {
    use super::*;

    /// Two tags that remove each other resolve without an overflow, and the
    /// edge toward the later id is ignored, independently of the starting tag.
    #[test]
    fn mutual_removals_resolve_without_an_overflow() {
        let tags: BTreeMap<String, String> = BTreeMap::from([
            (
                "test:a".into(),
                r##"{"values": ["test:one"], "remove": ["#test:b"]}"##.into(),
            ),
            (
                "test:b".into(),
                r##"{"values": ["test:one", "test:two"], "remove": ["#test:a"]}"##.into(),
            ),
        ]);
        let ids: BTreeSet<String> = ["test:one", "test:two"]
            .into_iter()
            .map(String::from)
            .collect();
        let registry = Registry {
            what: "placed feature",
            ids: &ids,
            tags: &tags,
        };
        crate::test_log::install();
        for (start, expected) in [("test:a", "test:one"), ("test:b", "test:two")] {
            crate::test_log::clear();
            let out = registry
                .resolve(&serde_json::json!(format!("#{start}")))
                .unwrap();
            assert_eq!(out, [expected]);
            {
                let lines = crate::test_log::records();
                assert!(
                    lines.iter().any(|(_, line)| line.contains("test:a")
                        && line.contains("test:b")
                        && line.contains("only removal edge test:a -> test:b is ignored")),
                    "{lines:?}"
                );
            }
        }
    }
}
