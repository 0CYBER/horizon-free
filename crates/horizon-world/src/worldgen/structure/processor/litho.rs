//! Lithostitched's processors (spec-lithostitched §5), read as its data
//! writes them: `lithostitched:block_swap` (in `processor.rs`), `reference`,
//! `apply_random`, `condition`, `set_block` and `discard_input`.
//!
//! The draws of `random_chance`, `apply_random` and `set_block` follow
//! Lithostitched's `random_mode` (per block or per piece), but its seeding is
//! not known (OPEN-L6): a positional random of the world seed, the block's or
//! the piece's position and the processor itself, so the shares match while
//! the blocks chosen do not.
use super::{BlockStateId, DecodeError, Modifier, ProcessorList, ProcessorRandom};
use crate::worldgen::{block_feature::FeatureBlockBindings, structure::StructureResources};
use horizon_core::{
    position::BlockPos,
    random::{LegacyRandom, position_seed},
};
use serde_json::Value;
use std::collections::BTreeSet;

/// `random_mode` (`condition`, `set_block`) or `mode` (`apply_random`).
#[derive(Clone, Copy, Debug)]
pub(super) struct Draws {
    per_piece: bool,
    /// The processor's own salt, so two processors on a block draw apart.
    salt: i64,
}
impl Draws {
    fn decode(mode: Option<&Value>, processor: &Value) -> Result<Self, DecodeError> {
        let per_piece = match mode.and_then(Value::as_str) {
            None | Some("per_block") => false,
            Some("per_piece") => true,
            _ => return Err(DecodeError::Field("random_mode")),
        };
        // FNV-1a over the processor's JSON: the same in every run.
        let salt = (processor.to_string().bytes()).fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
            (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
        }) as i64;
        Ok(Self { per_piece, salt })
    }
    /// The random for the block at `pos` of the piece at `piece`.
    pub(super) fn random(self, world_seed: i64, piece: [i32; 3], pos: [i32; 3]) -> LegacyRandom {
        let [x, y, z] = if self.per_piece { piece } else { pos };
        LegacyRandom::new(world_seed ^ self.salt ^ position_seed(BlockPos::new(x, y, z)))
    }
}

/// A processor or a list of them (`then`, `else`, `apply_random`'s data):
/// a list holder (`ProcessorList::decode`), or one processor object.
fn processors(
    value: &Value,
    resources: &StructureResources,
    bindings: &FeatureBlockBindings<'_>,
) -> Result<ProcessorList, DecodeError> {
    let one = value.get("processor_type").is_some();
    let holder = if one {
        Value::Array(vec![value.clone()])
    } else {
        value.clone()
    };
    ProcessorList::decode(&holder.to_string(), resources, bindings)
}

/// `reference`: the named lists, run in turn on the block.
pub(super) fn reference(
    value: &Value,
    resources: &StructureResources,
    bindings: &FeatureBlockBindings<'_>,
) -> Result<Vec<ProcessorList>, DecodeError> {
    let names = match &value["processor_lists"] {
        Value::Array(a) => a.clone(),
        other => vec![other.clone()],
    };
    (names.iter())
        .map(|name| match name.as_str() {
            Some(n) if n.starts_with('#') => Err(DecodeError::Field("processor list tag")),
            Some(_) => ProcessorList::decode(&name.to_string(), resources, bindings),
            None => Err(DecodeError::Field("processor_lists")),
        })
        .collect()
}

/// `apply_random`: one list drawn by weight runs on the block.
#[derive(Clone, Debug)]
pub struct ApplyRandom {
    pub(super) draws: Draws,
    lists: Vec<(i32, ProcessorList)>,
    total: i32,
}
impl ApplyRandom {
    pub(super) fn decode(
        value: &Value,
        resources: &StructureResources,
        bindings: &FeatureBlockBindings<'_>,
    ) -> Result<Self, DecodeError> {
        let entries = value["processor_lists"]
            .as_array()
            .ok_or(DecodeError::Field("apply_random processor_lists"))?;
        let lists = (entries.iter())
            .map(|e| match e.get("data") {
                Some(data) => {
                    let weight = e["weight"]
                        .as_i64()
                        .unwrap_or(1)
                        .clamp(0, i64::from(i32::MAX));
                    Ok((weight as i32, processors(data, resources, bindings)?))
                }
                None => Ok((1, processors(e, resources, bindings)?)),
            })
            .collect::<Result<Vec<_>, DecodeError>>()?;
        let total = lists
            .iter()
            .map(|(w, _)| *w)
            .fold(0i32, i32::saturating_add);
        Ok(Self {
            draws: Draws::decode(value.get("mode"), value)?,
            lists,
            total,
        })
    }
    /// The list drawn: one bounded draw over the total weight.
    pub(super) fn pick(&self, random: &mut impl ProcessorRandom) -> Option<&ProcessorList> {
        if self.total <= 0 {
            return None;
        }
        let mut at = random.int(self.total);
        for (weight, list) in &self.lists {
            at -= weight;
            if at < 0 {
                return Some(list);
            }
        }
        None
    }
}

/// Lithostitched's processor conditions.
#[derive(Clone, Debug)]
pub(super) enum Condition {
    True,
    AllOf(Vec<Condition>),
    AnyOf(Vec<Condition>),
    Not(Box<Condition>),
    Chance(f32),
    /// `matching_blocks`: the template's block (`input`) or the world's
    /// (`location`) is one of `blocks`, with `properties`.
    Matching {
        blocks: BTreeSet<String>,
        properties: Vec<(String, String)>,
        location: bool,
    },
}
impl Condition {
    /// One condition, or a list of them that must all pass (`if_true`).
    pub(super) fn decode(
        value: &Value,
        bindings: &FeatureBlockBindings<'_>,
    ) -> Result<Self, DecodeError> {
        let list = |v: &Value| -> Result<Vec<Self>, DecodeError> {
            (v.as_array().ok_or(DecodeError::Field("conditions"))?.iter())
                .map(|c| Self::decode(c, bindings))
                .collect()
        };
        if value.is_array() {
            return Ok(Self::AllOf(list(value)?));
        }
        let kind = value["type"].as_str().unwrap_or_default();
        Ok(match crate::worldgen::tags::identifier(kind)?.as_str() {
            "lithostitched:true" => Self::True,
            "lithostitched:all_of" => Self::AllOf(list(&value["conditions"])?),
            "lithostitched:any_of" => Self::AnyOf(list(&value["conditions"])?),
            "lithostitched:not" => {
                Self::Not(Box::new(Self::decode(&value["condition"], bindings)?))
            }
            "lithostitched:random_chance" => Self::Chance(
                value["chance"]
                    .as_f64()
                    .ok_or(DecodeError::Field("random_chance chance"))? as f32,
            ),
            "lithostitched:matching_blocks" => Self::Matching {
                blocks: blocks(&value["blocks"], bindings)?,
                properties: (value
                    .get("properties")
                    .and_then(Value::as_object)
                    .into_iter())
                .flatten()
                .map(|(k, v)| {
                    (
                        k.clone(),
                        v.as_str().map_or_else(|| v.to_string(), str::to_owned),
                    )
                })
                .collect(),
                location: match value.get("match_type").and_then(Value::as_str) {
                    None | Some("input") => false,
                    Some("location") => true,
                    _ => return Err(DecodeError::Field("matching_blocks match_type")),
                },
            },
            _ => return Err(DecodeError::Field("lithostitched condition type")),
        })
    }
    pub(super) fn test(
        &self,
        input: BlockStateId,
        location: BlockStateId,
        registry: &crate::registry::BlockRegistry,
        random: &mut impl ProcessorRandom,
    ) -> bool {
        match self {
            Self::True => true,
            Self::AllOf(all) => all
                .iter()
                .all(|c| c.test(input, location, registry, random)),
            Self::AnyOf(any) => any
                .iter()
                .any(|c| c.test(input, location, registry, random)),
            Self::Not(c) => !c.test(input, location, registry, random),
            Self::Chance(chance) => random.float() < *chance,
            Self::Matching {
                blocks,
                properties,
                location: at_location,
            } => {
                let Some(state) = registry.state(if *at_location { location } else { input })
                else {
                    return false;
                };
                blocks.contains(&state.name)
                    && (properties.iter()).all(|(k, v)| state.properties.get(k) == Some(v))
            }
        }
    }
}
/// A block set (an id, a list, `#tag`) by block name. Blocks the game does
/// not have are left out: no template block can be one.
fn blocks(
    value: &Value,
    bindings: &FeatureBlockBindings<'_>,
) -> Result<BTreeSet<String>, DecodeError> {
    let names: Vec<&str> = match value {
        Value::String(s) => vec![s.as_str()],
        Value::Array(a) => a.iter().filter_map(Value::as_str).collect(),
        _ => return Err(DecodeError::Field("matching_blocks blocks")),
    };
    let registry = bindings.registry();
    let mut out = BTreeSet::new();
    for name in names {
        match name.strip_prefix('#') {
            Some(tag) => {
                let tag = crate::worldgen::tags::identifier(tag)?;
                let states = bindings.tags.get(&tag).into_iter().flat_map(|s| s.iter());
                out.extend(states.filter_map(|s| Some(registry.state(*s)?.name.clone())));
            }
            None => {
                out.insert(crate::worldgen::tags::identifier(name)?.to_string());
            }
        }
    }
    Ok(out)
}

/// `condition`: `then` when the conditions pass, else `else` (none: the
/// block as it is).
#[derive(Clone, Debug)]
pub struct Conditional {
    pub(super) draws: Draws,
    pub(super) condition: Condition,
    pub(super) then: ProcessorList,
    pub(super) otherwise: ProcessorList,
}
impl Conditional {
    pub(super) fn decode(
        value: &Value,
        resources: &StructureResources,
        bindings: &FeatureBlockBindings<'_>,
    ) -> Result<Self, DecodeError> {
        let branch = |key| match value.get(key) {
            Some(v) => processors(v, resources, bindings),
            None => Ok(ProcessorList(Vec::new())),
        };
        Ok(Self {
            draws: Draws::decode(value.get("random_mode"), value)?,
            condition: Condition::decode(&value["if_true"], bindings)?,
            then: branch("then")?,
            otherwise: branch("else")?,
        })
    }
}

/// `set_block`: the block from a state provider (simple or weighted).
#[derive(Clone, Debug)]
pub struct SetBlock {
    pub(super) draws: Draws,
    states: Vec<(i32, BlockStateId)>,
    total: i32,
    /// `preserve_state` (true by default): the template block's properties
    /// the new block has too are kept.
    pub(super) preserve: bool,
    pub(super) modifier: Modifier,
}
impl SetBlock {
    pub(super) fn decode(
        value: &Value,
        state: impl Fn(&Value) -> Result<BlockStateId, DecodeError>,
    ) -> Result<Self, DecodeError> {
        let provider = &value["state_provider"];
        let kind = provider["type"].as_str().unwrap_or_default();
        let states = match crate::worldgen::tags::identifier(kind)?.as_str() {
            "minecraft:simple_state_provider" => vec![(1, state(&provider["state"])?)],
            "minecraft:weighted_state_provider" => (provider["entries"].as_array().into_iter())
                .flatten()
                .map(|e| {
                    Ok((
                        e["weight"].as_i64().unwrap_or(1).clamp(0, 1 << 30) as i32,
                        state(&e["data"])?,
                    ))
                })
                .collect::<Result<_, DecodeError>>()?,
            _ => return Err(DecodeError::Field("set_block state_provider")),
        };
        let total = states.iter().map(|(w, _)| *w).sum();
        if total <= 0 {
            return Err(DecodeError::Field("set_block state_provider"));
        }
        Ok(Self {
            draws: Draws::decode(value.get("random_mode"), value)?,
            states,
            total,
            preserve: value
                .get("preserve_state")
                .and_then(Value::as_bool)
                .unwrap_or(true),
            modifier: Modifier::decode(value.get("block_entity_modifier"))?,
        })
    }
    /// The provider's state: WeightedList.getRandom, one bounded draw.
    pub(super) fn state(&self, random: &mut impl ProcessorRandom) -> BlockStateId {
        let mut at = random.int(self.total);
        for (weight, state) in &self.states {
            at -= weight;
            if at < 0 {
                return *state;
            }
        }
        self.states[0].1
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::BlockRegistry;
    use serde_json::json;

    // 0 air, 1 cobblestone, 2..=9 wheat age 0..7, 10..=13 beetroots age 0..3,
    // 14..=21 carrots age 0..7, 22 water.
    fn registry() -> BlockRegistry {
        let row = |name: &str, age: u32| json!([name, false, false, "", {"id": name, "properties": {"age": age.to_string()}}]);
        let mut states = vec![
            json!(["minecraft:air", false, false, "", "minecraft:air"]),
            json!([
                "minecraft:cobblestone",
                false,
                false,
                "",
                "minecraft:cobblestone"
            ]),
        ];
        states.extend((0..8).map(|a| row("minecraft:wheat", a)));
        states.extend((0..4).map(|a| row("minecraft:beetroots", a)));
        states.extend((0..8).map(|a| row("minecraft:carrots", a)));
        states.push(json!([
            "minecraft:water",
            false,
            false,
            "",
            "minecraft:water"
        ]));
        let facts = json!({"defaults": {"minecraft:air": 0, "minecraft:cobblestone": 1, "minecraft:wheat": 2,
            "minecraft:beetroots": 10, "minecraft:carrots": 14, "minecraft:water": 22}, "states": states});
        BlockRegistry::from_facts_json(facts.to_string().as_bytes()).unwrap()
    }

    #[test]
    fn swaps_keep_only_the_properties_the_new_block_can_have() {
        // spec-ctov §4: wheat[age=7] becomes carrots[age=7] but beetroots[age=0].
        let registry = registry();
        let ripe_wheat = BlockStateId(9);
        let swap = |to| super::super::with_properties_of(&registry, to, ripe_wheat);
        assert_eq!(swap("minecraft:carrots"), Some(BlockStateId(21)));
        assert_eq!(swap("minecraft:beetroots"), Some(BlockStateId(10)));
        // A block the game does not have: the template's block stays.
        assert_eq!(swap("natures_spirit:olive_log"), None);
    }

    #[test]
    fn conditions_read_the_template_or_the_world() {
        let registry = registry();
        let mut random = LegacyRandom::new(1);
        let matching = |location| Condition::Matching {
            blocks: BTreeSet::from(["minecraft:water".to_owned()]),
            properties: Vec::new(),
            location,
        };
        let (cobblestone, water) = (BlockStateId(1), BlockStateId(22));
        assert!(!matching(false).test(cobblestone, water, &registry, &mut random));
        assert!(matching(true).test(cobblestone, water, &registry, &mut random));
        let ripe = Condition::Matching {
            blocks: BTreeSet::from(["minecraft:wheat".to_owned()]),
            properties: vec![("age".into(), "7".into())],
            location: false,
        };
        assert!(ripe.test(BlockStateId(9), water, &registry, &mut random));
        assert!(!ripe.test(BlockStateId(8), water, &registry, &mut random));
        let both = Condition::AllOf(vec![
            matching(true),
            Condition::Not(Box::new(Condition::True)),
        ]);
        assert!(!both.test(cobblestone, water, &registry, &mut random));
        assert!(
            Condition::AnyOf(vec![Condition::Chance(0.), matching(true)]).test(
                cobblestone,
                water,
                &registry,
                &mut random
            )
        );
    }

    #[test]
    fn per_piece_draws_once_per_piece() {
        let p = json!({"processor_type": "lithostitched:apply_random"});
        let piece = Draws::decode(Some(&json!("per_piece")), &p).unwrap();
        let block = Draws::decode(None, &p).unwrap();
        let first = |d: Draws, pos| d.random(42, [0, 64, 0], pos).int(1 << 30);
        assert_eq!(first(piece, [1, 64, 1]), first(piece, [9, 70, 3]));
        assert_ne!(first(block, [1, 64, 1]), first(block, [9, 70, 3]));
        // Another processor draws apart on the same block.
        let other =
            Draws::decode(None, &json!({"processor_type": "lithostitched:set_block"})).unwrap();
        assert_ne!(first(block, [1, 64, 1]), first(other, [1, 64, 1]));
        assert!(Draws::decode(Some(&json!("per_structure")), &p).is_err());
    }

    #[test]
    fn set_block_draws_by_weight() {
        let set = SetBlock::decode(
            &json!({"processor_type": "lithostitched:set_block", "preserve_state": false,
                "state_provider": {"type": "minecraft:weighted_state_provider", "entries": [
                    {"weight": 1, "data": {"id": "minecraft:cobblestone"}},
                    {"weight": 3, "data": {"id": "minecraft:water"}}]}}),
            |v| {
                Ok(BlockStateId(if v["id"] == "minecraft:water" {
                    22
                } else {
                    1
                }))
            },
        )
        .unwrap();
        assert!(!set.preserve);
        let water = (0..4000)
            .filter(|x| {
                set.state(&mut set.draws.random(7, [0; 3], [*x, 64, 0])) == BlockStateId(22)
            })
            .count();
        assert!((2850..3150).contains(&water), "{water}");
        let unknown = json!({"state_provider": {"type": "minecraft:noise_provider"}});
        assert!(SetBlock::decode(&unknown, |_| Ok(BlockStateId(0))).is_err());
    }
}
