//! The `minecraft:block_transformer` registry (axe, hoe, shovel) that
//! `BlockTransformer.transformBlock` reads: each transform's rule based state
//! provider is evaluated against the clicked block and the block above.
//! `tools/test_block_transformer_parity.py` compares every block state with
//! vanilla.
use crate::{Result, items::Items};
use horizon_world::{chunk::BlockState, registry::BlockRegistry};
use serde_json::Value;
use std::{collections::HashMap, fs, path::Path};

enum Predicate {
    Blocks(Vec<String>),
    Tag { tag: String, offset: [i32; 3] },
    AllOf(Vec<Predicate>),
    AnyOf(Vec<Predicate>),
}

enum Provider {
    Rules(Vec<(Predicate, Provider)>),
    State(BlockState),
    /// `CopyPropertiesProvider`: the source with the clicked block's properties.
    Copy(Box<Provider>),
}

/// One `BlockTransformData`.
pub struct Transform {
    provider: Provider,
    pub disallowed_faces: Vec<String>,
    pub sound: String,
    pub particle: String,
    pub loot: Option<String>,
    pub drop_strategy: String,
    pub update_from_neighbors: bool,
    pub transform_type: String,
    pub consume_on_use: bool,
    pub damage: u32,
}

pub struct Transformers {
    transformers: HashMap<String, Vec<Transform>>,
}

/// What the transformer reads of the world: the block state at an offset from
/// the clicked position.
pub trait Around {
    fn state(&self, offset: [i32; 3]) -> BlockState;
}

fn state_of(value: &Value, registry: &BlockRegistry) -> Option<BlockState> {
    let name = value["id"].as_str().or_else(|| value["Name"].as_str())?;
    let default = registry.default_state(name)?;
    let mut state = registry.state(default)?.clone();
    for (k, v) in value["properties"].as_object().into_iter().flatten() {
        state.properties.insert(k.clone(), v.as_str()?.to_owned());
    }
    Some(state)
}

fn predicate(value: &Value) -> Option<Predicate> {
    Some(match value["type"].as_str()? {
        "minecraft:matching_blocks" => Predicate::Blocks(match &value["blocks"] {
            Value::String(s) => vec![s.clone()],
            Value::Array(a) => a
                .iter()
                .filter_map(|b| b.as_str().map(str::to_owned))
                .collect(),
            _ => return None,
        }),
        "minecraft:matching_block_tag" => {
            let o = value["offset"].as_array();
            let at = |i: usize| {
                o.and_then(|o| o.get(i))
                    .and_then(Value::as_i64)
                    .unwrap_or(0) as i32
            };
            Predicate::Tag {
                tag: value["tag"].as_str()?.to_owned(),
                offset: [at(0), at(1), at(2)],
            }
        }
        "minecraft:all_of" => Predicate::AllOf(
            value["predicates"]
                .as_array()?
                .iter()
                .map(predicate)
                .collect::<Option<_>>()?,
        ),
        "minecraft:any_of" => Predicate::AnyOf(
            value["predicates"]
                .as_array()?
                .iter()
                .map(predicate)
                .collect::<Option<_>>()?,
        ),
        _ => return None,
    })
}

fn provider(value: &Value, registry: &BlockRegistry) -> Option<Provider> {
    match value["type"].as_str() {
        Some("minecraft:rule_based") => {
            let mut rules = Vec::new();
            for rule in value["rules"].as_array()? {
                rules.push((
                    predicate(&rule["if_true"])?,
                    provider(&rule["then"], registry)?,
                ));
            }
            Some(Provider::Rules(rules))
        }
        Some("minecraft:copy_properties") => Some(Provider::Copy(Box::new(provider(
            &value["source"],
            registry,
        )?))),
        _ => state_of(value, registry).map(Provider::State),
    }
}

/// `BlockState.withPropertiesOf`: the properties both blocks have.
fn with_properties_of(
    registry: &BlockRegistry,
    mut state: BlockState,
    old: &BlockState,
) -> Option<BlockState> {
    let defaults = registry
        .default_state(&state.name)
        .and_then(|id| registry.state(id))?;
    for (k, v) in &old.properties {
        if defaults.properties.contains_key(k) {
            let mut candidate = state.clone();
            candidate.properties.insert(k.clone(), v.clone());
            if registry.resolve(&candidate).is_ok() {
                state = candidate;
            }
        }
    }
    Some(state)
}

impl Predicate {
    fn test(&self, items: &Items, around: &dyn Around) -> bool {
        match self {
            Self::Blocks(names) => {
                let state = around.state([0, 0, 0]);
                names.contains(&state.name)
            }
            Self::Tag { tag, offset } => {
                let state = around.state(*offset);
                items
                    .block_tags
                    .get(tag)
                    .is_some_and(|members| members.contains(&state.name))
            }
            Self::AllOf(all) => all.iter().all(|p| p.test(items, around)),
            Self::AnyOf(any) => any.iter().any(|p| p.test(items, around)),
        }
    }
}

impl Provider {
    fn state(
        &self,
        items: &Items,
        registry: &BlockRegistry,
        around: &dyn Around,
    ) -> Option<BlockState> {
        match self {
            Self::Rules(rules) => rules
                .iter()
                .find(|(p, _)| p.test(items, around))
                .and_then(|(_, then)| then.state(items, registry, around)),
            Self::State(state) => Some(state.clone()),
            Self::Copy(source) => {
                let state = source.state(items, registry, around)?;
                with_properties_of(registry, state, &around.state([0, 0, 0]))
            }
        }
    }
}

impl Transformers {
    /// Reads `data/minecraft/block_transformer/*.json`.
    pub fn load(root: &Path, registry: &BlockRegistry) -> Result<Self> {
        let mut transformers = HashMap::new();
        let dir = root.join("data/minecraft/block_transformer");
        if dir.exists() {
            for entry in fs::read_dir(&dir)?.flatten() {
                let path = entry.path();
                let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                    continue;
                };
                let list: Value = serde_json::from_slice(&fs::read(&path)?)?;
                let mut transforms = Vec::new();
                for t in list.as_array().into_iter().flatten() {
                    let text = |k: &str, d: &str| t[k].as_str().unwrap_or(d).to_owned();
                    let Some(provider) = provider(&t["block_state_provider"], registry) else {
                        continue;
                    };
                    transforms.push(Transform {
                        provider,
                        disallowed_faces: t["disallowed_faces"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(|f| f.as_str().map(str::to_owned))
                            .collect(),
                        sound: text("sound", "minecraft:empty"),
                        particle: text("particle", "none"),
                        loot: t["loot"].as_str().map(str::to_owned),
                        drop_strategy: text("drop_strategy", "from_middle"),
                        update_from_neighbors: t["update_from_neighbors"].as_bool().unwrap_or(true),
                        transform_type: text("transform_type", "single_block"),
                        consume_on_use: t["consume_on_use"].as_bool().unwrap_or(true),
                        damage: t["item_damage_per_use"].as_u64().unwrap_or(0) as u32,
                    });
                }
                transformers.insert(format!("minecraft:{stem}"), transforms);
            }
        }
        Ok(Self { transformers })
    }

    /// `BlockTransformer.transformBlock` without the world writes: the first
    /// transform that allows the clicked face and provides a state, with its
    /// index.
    pub fn apply<'a>(
        &'a self,
        items: &Items,
        registry: &BlockRegistry,
        id: &str,
        face: &str,
        around: &dyn Around,
    ) -> Option<(usize, &'a Transform, BlockState)> {
        self.transformers
            .get(id)?
            .iter()
            .enumerate()
            .filter(|(_, t)| !t.disallowed_faces.iter().any(|f| f == face))
            .find_map(|(i, t)| Some((i, t, t.provider.state(items, registry, around)?)))
    }

    /// `HoneycombItem.getWaxed`: the waxed twin of a copper block, found from
    /// the axe's wax-off transforms (`WAX_OFF_BY_BLOCK` is the inverse of
    /// `WAXABLES`).
    pub fn waxed(&self, registry: &BlockRegistry, old: &BlockState) -> Option<BlockState> {
        for transform in self.transformers.get("minecraft:axe")? {
            if transform.particle != "wax_off" {
                continue;
            }
            let Provider::Rules(rules) = &transform.provider else {
                continue;
            };
            for (predicate, then) in rules {
                if let (Predicate::Blocks(names), Provider::Copy(source)) = (predicate, then)
                    && let Provider::State(unwaxed) = &**source
                    && unwaxed.name == old.name
                    && let Some(waxed) = names.first()
                {
                    let target = registry
                        .default_state(waxed)
                        .and_then(|id| registry.state(id))?
                        .clone();
                    return with_properties_of(registry, target, old);
                }
            }
        }
        None
    }
}
