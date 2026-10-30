//! Bounded numeric-condition registry loading and direct/holder JSON codecs.
use super::MaterialCondition;
use crate::worldgen::{
    DecodeError, Object, anchor::VerticalAnchor, double, int_range, integer, optional, required,
};
use serde_json::{Value, value::RawValue};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

pub(super) fn identifier(name: &str) -> Result<String, DecodeError> {
    let (namespace, path) = name.split_once(':').unwrap_or(("minecraft", name));
    let namespace = if namespace.is_empty() {
        "minecraft"
    } else {
        namespace
    };
    let valid = |c: u8| c.is_ascii_lowercase() || c.is_ascii_digit() || b"_.-".contains(&c);
    if !namespace.bytes().all(valid) || !path.bytes().all(|c| valid(c) || c == b'/') {
        return Err(DecodeError::Field("material identifier"));
    }
    Ok(format!("{namespace}:{path}"))
}
#[derive(Clone, Debug)]
pub struct MaterialConditionReference {
    name: String,
    value: Arc<MaterialCondition>,
}
impl MaterialConditionReference {
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn value(&self) -> &MaterialCondition {
        &self.value
    }
}
/// A named biome-tag snapshot for a fixed world-generation configuration.
#[derive(Clone, Debug)]
pub struct MaterialBiomeTag {
    name: String,
    members: Arc<[String]>,
}
impl MaterialBiomeTag {
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn members(&self) -> &[String] {
        &self.members
    }
}
#[derive(Debug)]
struct BiomeCatalog {
    names: BTreeSet<String>,
    tags: BTreeMap<String, Arc<[String]>>,
}
#[derive(Clone, Debug, Default)]
pub struct MaterialConditionRegistry {
    values: BTreeMap<String, Arc<MaterialCondition>>,
    biomes: Option<Arc<BiomeCatalog>>,
}
impl MaterialConditionRegistry {
    /// Load an immutable set atomically, including forward references. Cycles,
    /// unresolved references and resource-limit violations are typed errors.
    pub fn from_json(definitions: &BTreeMap<String, String>) -> Result<Self, DecodeError> {
        Self::load(definitions, None)
    }
    /// Bind resolved biome tags, such as `TagDefinitions::build().tags`.
    /// Names and memberships remain immutable for the lifetime of this registry.
    pub fn from_json_with_biome_tags(
        definitions: &BTreeMap<String, String>,
        biomes: &BTreeSet<String>,
        tags: &BTreeMap<String, Vec<String>>,
    ) -> Result<Self, DecodeError> {
        if biomes.len() > 8192 || tags.len() > 8192 {
            return Err(DecodeError::Field("material biome catalog size"));
        }
        let names: BTreeSet<_> = biomes
            .iter()
            .map(|name| identifier(name))
            .collect::<Result<_, _>>()?;
        let mut values = BTreeMap::new();
        let mut count = 0usize;
        for (name, members) in tags {
            count = count.saturating_add(members.len());
            if count > 65536 {
                return Err(DecodeError::Field("material biome tag size"));
            }
            let mut bound = Vec::with_capacity(members.len());
            for member in members {
                let member = identifier(member)?;
                if !names.contains(&member) {
                    return Err(DecodeError::Field("unknown tagged biome"));
                }
                bound.push(member);
            }
            if values.insert(identifier(name)?, Arc::from(bound)).is_some() {
                return Err(DecodeError::Field("duplicate biome tag"));
            }
        }
        Self::load(
            definitions,
            Some(Arc::new(BiomeCatalog {
                names,
                tags: values,
            })),
        )
    }
    fn load(
        definitions: &BTreeMap<String, String>,
        biomes: Option<Arc<BiomeCatalog>>,
    ) -> Result<Self, DecodeError> {
        if definitions.len() > 8192 {
            return Err(DecodeError::Field("material registry size"));
        }
        let mut loader = Loader {
            biomes,
            ..Loader::default()
        };
        for (name, json) in definitions {
            if loader
                .definitions
                .insert(identifier(name)?, serde_json::from_str(json)?)
                .is_some()
            {
                return Err(DecodeError::Field("duplicate material condition"));
            }
        }
        let names: Vec<_> = loader.definitions.keys().cloned().collect();
        for name in names {
            loader.resolve(&name, 0)?;
        }
        Ok(Self {
            values: loader.values,
            biomes: loader.biomes,
        })
    }
    pub fn decode(&self, json: &str) -> Result<MaterialCondition, DecodeError> {
        let raw: Box<RawValue> = serde_json::from_str(json)?;
        Loader {
            values: self.values.clone(),
            biomes: self.biomes.clone(),
            ..Loader::default()
        }
        .node(&raw, 0)
    }
    pub fn get(&self, name: &str) -> Option<MaterialConditionReference> {
        let name = identifier(name).ok()?;
        self.values
            .get(&name)
            .map(|value| MaterialConditionReference {
                name,
                value: value.clone(),
            })
    }
    pub fn entries(&self) -> impl Iterator<Item = (&str, &MaterialCondition)> {
        self.values
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_ref()))
    }
}
#[derive(Default)]
struct Loader {
    definitions: BTreeMap<String, Box<RawValue>>,
    values: BTreeMap<String, Arc<MaterialCondition>>,
    active: BTreeSet<String>,
    biomes: Option<Arc<BiomeCatalog>>,
    nodes: usize,
}
impl Loader {
    fn resolve(&mut self, name: &str, depth: usize) -> Result<Arc<MaterialCondition>, DecodeError> {
        if depth >= 256 {
            return Err(DecodeError::Field("material condition depth"));
        }
        if let Some(value) = self.values.get(name) {
            return Ok(value.clone());
        }
        if !self.active.insert(name.to_owned()) {
            return Err(DecodeError::Field("cyclic material condition"));
        }
        let json = self
            .definitions
            .get(name)
            .ok_or(DecodeError::Field("missing material condition"))?
            .clone();
        if !json.get().starts_with('{') {
            return Err(DecodeError::Field("material definition must be direct"));
        }
        let value = Arc::new(self.node(&json, depth + 1)?);
        self.active.remove(name);
        self.values.insert(name.to_owned(), value.clone());
        Ok(value)
    }
    fn node(&mut self, raw: &RawValue, depth: usize) -> Result<MaterialCondition, DecodeError> {
        self.nodes += 1;
        if depth >= 256 || self.nodes > 65536 {
            return Err(DecodeError::Field("material condition size"));
        }
        if raw.get().starts_with('"') {
            let name = identifier(&serde_json::from_str::<String>(raw.get())?)?;
            let value = self.resolve(&name, depth + 1)?;
            return Ok(MaterialCondition::Reference(MaterialConditionReference {
                name,
                value,
            }));
        }
        let object: Object = serde_json::from_str(raw.get())?;
        let kind = identifier(&serde_json::from_str::<String>(
            required(&object, "type")?.get(),
        )?)?;
        let number = |name| integer(required(&object, name)?);
        let boolean = |name| {
            serde_json::from_str::<bool>(required(&object, name)?.get()).map_err(DecodeError::from)
        };
        let anchor = |name| VerticalAnchor::from_json(required(&object, name)?.get());
        let multiplier = || {
            int_range(
                required(&object, "surface_depth_multiplier")?,
                "surface_depth_multiplier",
                -20,
                20,
            )
        };
        Ok(match kind.as_str() {
            "minecraft:noise_threshold" => MaterialCondition::NoiseThreshold {
                noise: identifier(&serde_json::from_str::<String>(
                    required(&object, "noise")?.get(),
                )?)?,
                min: double(required(&object, "min_threshold")?)?,
                max: double(required(&object, "max_threshold")?)?,
                is_3d: optional(&object, "is_3d")
                    .map(|raw| serde_json::from_str(raw.get()))
                    .transpose()?
                    .unwrap_or(false),
            },
            "minecraft:temperature" => MaterialCondition::Temperature,
            "minecraft:above_preliminary_surface" => MaterialCondition::AbovePreliminarySurface,
            "minecraft:biome" => {
                let catalog = self
                    .biomes
                    .as_ref()
                    .ok_or(DecodeError::Field("missing biome catalog"))?;
                let raw = required(&object, "biome_is")?;
                let names: Vec<String> = if raw.get().starts_with('"') {
                    let name: String = serde_json::from_str(raw.get())?;
                    if let Some(tag) = name.strip_prefix('#') {
                        let name = identifier(tag)?;
                        let members = catalog
                            .tags
                            .get(&name)
                            .ok_or(DecodeError::Field("unknown biome tag"))?
                            .clone();
                        return Ok(MaterialCondition::BiomeTag(MaterialBiomeTag {
                            name,
                            members,
                        }));
                    }
                    vec![name]
                } else {
                    serde_json::from_str(raw.get())?
                };
                if names.len() > 8192 {
                    return Err(DecodeError::Field("biome condition size"));
                }
                let mut resolved = Vec::with_capacity(names.len());
                for name in names {
                    let name = identifier(&name)?;
                    if !catalog.names.contains(&name) {
                        return Err(DecodeError::Field("unknown material biome"));
                    }
                    resolved.push(name);
                }
                MaterialCondition::Biome(resolved)
            }
            "minecraft:hole" => MaterialCondition::Hole,
            "minecraft:steep" => MaterialCondition::Steep,
            "minecraft:y_above" => MaterialCondition::YAbove {
                anchor: anchor("anchor")?,
                surface_depth_multiplier: multiplier()?,
                add_stone_depth: boolean("add_stone_depth")?,
            },
            "minecraft:water" => MaterialCondition::Water {
                offset: number("offset")?,
                surface_depth_multiplier: multiplier()?,
                add_stone_depth: boolean("add_stone_depth")?,
            },
            "minecraft:stone_depth" => {
                let surface: String =
                    serde_json::from_str(required(&object, "surface_type")?.get())?;
                let ceiling = match surface.as_str() {
                    "ceiling" => true,
                    "floor" => false,
                    _ => return Err(DecodeError::Field("surface_type")),
                };
                MaterialCondition::StoneDepth {
                    offset: number("offset")?,
                    add_surface_depth: boolean("add_surface_depth")?,
                    secondary_depth_range: number("secondary_depth_range")?,
                    ceiling,
                }
            }
            "minecraft:vertical_gradient" => MaterialCondition::VerticalGradient {
                random_name: identifier(&serde_json::from_str::<String>(
                    required(&object, "random_name")?.get(),
                )?)?,
                true_at_and_below: anchor("true_at_and_below")?,
                false_at_and_above: anchor("false_at_and_above")?,
            },
            "minecraft:not" => MaterialCondition::Not(Box::new(
                self.node(required(&object, "invert")?, depth + 1)?,
            )),
            _ => return Err(DecodeError::Field("unsupported material condition type")),
        })
    }
}
impl MaterialCondition {
    pub fn from_json(json: &str) -> Result<Self, DecodeError> {
        MaterialConditionRegistry::default().decode(json)
    }
    /// Encode the vanilla codec value. Nonfinite double thresholds use the
    /// same NaN/Infinity tokens as Gson; strict JSON decoding rejects those tokens.
    pub fn to_json(&self) -> Result<String, DecodeError> {
        self.encoded(0)
    }
    fn encoded(&self, depth: usize) -> Result<String, DecodeError> {
        if depth >= 256 {
            return Err(DecodeError::Field("material condition depth"));
        }
        match self {
            Self::Not(child) => Ok(format!(
                "{{\"type\":\"minecraft:not\",\"invert\":{}}}",
                child.encoded(depth + 1)?
            )),
            Self::NoiseThreshold {
                noise,
                min,
                max,
                is_3d,
            } => {
                let number = |v: f64| {
                    if v.is_nan() {
                        "NaN".to_owned()
                    } else if v == f64::INFINITY {
                        "Infinity".to_owned()
                    } else if v == f64::NEG_INFINITY {
                        "-Infinity".to_owned()
                    } else {
                        serde_json::to_string(&v).unwrap()
                    }
                };
                Ok(format!(
                    "{{\"type\":\"minecraft:noise_threshold\",\"noise\":{},\"min_threshold\":{},\"max_threshold\":{}{}}}",
                    serde_json::to_string(&identifier(noise)?)?,
                    number(*min),
                    number(*max),
                    if *is_3d { ",\"is_3d\":true" } else { "" }
                ))
            }
            _ => Ok(serde_json::to_string(&self.json_value(depth)?)?),
        }
    }
    fn json_value(&self, depth: usize) -> Result<Value, DecodeError> {
        if depth >= 256 {
            return Err(DecodeError::Field("material condition depth"));
        }
        let multiplier = |value: i32| {
            if (-20..=20).contains(&value) {
                Ok(value)
            } else {
                Err(DecodeError::Field("surface_depth_multiplier"))
            }
        };
        let anchor = |value: VerticalAnchor| -> Result<Value, DecodeError> {
            Ok(serde_json::from_str(&value.to_json()?)?)
        };
        Ok(match self {
            Self::Temperature => serde_json::json!({"type":"minecraft:temperature"}),
            Self::AbovePreliminarySurface => {
                serde_json::json!({"type":"minecraft:above_preliminary_surface"})
            }
            Self::BiomeTag(tag) => {
                serde_json::json!({"type":"minecraft:biome","biome_is":format!("#{}",tag.name)})
            }
            Self::Biome(names) => {
                let names = names
                    .iter()
                    .map(|name| identifier(name))
                    .collect::<Result<Vec<_>, _>>()?;
                let value = if names.len() == 1 {
                    Value::String(names[0].clone())
                } else {
                    serde_json::to_value(names)?
                };
                serde_json::json!({"type":"minecraft:biome","biome_is":value})
            }
            Self::Hole => serde_json::json!({"type":"minecraft:hole"}),
            Self::Steep => serde_json::json!({"type":"minecraft:steep"}),
            Self::YAbove {
                anchor: a,
                surface_depth_multiplier,
                add_stone_depth,
            } => {
                serde_json::json!({"type":"minecraft:y_above","anchor":anchor(*a)?,"surface_depth_multiplier":multiplier(*surface_depth_multiplier)?,"add_stone_depth":add_stone_depth})
            }
            Self::Water {
                offset,
                surface_depth_multiplier,
                add_stone_depth,
            } => {
                serde_json::json!({"type":"minecraft:water","offset":offset,"surface_depth_multiplier":multiplier(*surface_depth_multiplier)?,"add_stone_depth":add_stone_depth})
            }
            Self::StoneDepth {
                offset,
                add_surface_depth,
                secondary_depth_range,
                ceiling,
            } => {
                serde_json::json!({"type":"minecraft:stone_depth","offset":offset,"add_surface_depth":add_surface_depth,"secondary_depth_range":secondary_depth_range,"surface_type":if *ceiling{"ceiling"}else{"floor"}})
            }
            Self::VerticalGradient {
                random_name,
                true_at_and_below,
                false_at_and_above,
            } => {
                serde_json::json!({"type":"minecraft:vertical_gradient","random_name":identifier(random_name)?,"true_at_and_below":anchor(*true_at_and_below)?,"false_at_and_above":anchor(*false_at_and_above)?})
            }
            Self::Not(_) | Self::NoiseThreshold { .. } => {
                return Err(DecodeError::Field("condition requires direct encoding"));
            }
            Self::Reference(reference) => Value::String(reference.name.clone()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn forward_references_are_stable_and_invalid_graphs_fail_atomically() {
        let definitions = BTreeMap::from([
            ("a".to_owned(), r#"{"type":"not","invert":"z"}"#.to_owned()),
            ("z".to_owned(), r#"{"type":"hole"}"#.to_owned()),
        ]);
        let registry = MaterialConditionRegistry::from_json(&definitions).unwrap();
        let reference = registry.decode(r#""a""#).unwrap();
        assert_eq!(reference.to_json().unwrap(), r#""minecraft:a""#);
        assert!(
            registry
                .decode(r#"{"type":"not","invert":"missing"}"#)
                .is_err()
        );
        assert_eq!(registry.entries().count(), 2);
        assert_eq!(
            registry.get("z").unwrap().value().to_json().unwrap(),
            r#"{"type":"minecraft:hole"}"#
        );
        let mut cyclic = definitions.clone();
        cyclic.insert("z".to_owned(), r#"{"type":"not","invert":"a"}"#.to_owned());
        assert!(MaterialConditionRegistry::from_json(&cyclic).is_err());
        let mut alias = definitions.clone();
        alias.insert("z".to_owned(), r#""a""#.to_owned());
        assert!(MaterialConditionRegistry::from_json(&alias).is_err());
        let mut duplicate = definitions;
        duplicate.insert("minecraft:z".to_owned(), r#"{"type":"steep"}"#.to_owned());
        assert!(MaterialConditionRegistry::from_json(&duplicate).is_err());
        assert_eq!(reference.to_json().unwrap(), r#""minecraft:a""#);
    }
}
