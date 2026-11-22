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
    total: i64,
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
        let total = lists.iter().map(|(w, _)| i64::from(*w)).sum();
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
        let mut at = draw(self.total, random);
        for (weight, list) in &self.lists {
            at -= i64::from(*weight);
            if at < 0 {
                return Some(list);
            }
        }
        None
    }
}

// Preserve bounded-int draws for ordinary packs. Oversized totals use one
// unsigned nextLong modulo the total, as in the other wide weight selectors.
fn draw(total: i64, random: &mut impl ProcessorRandom) -> i64 {
    if total <= i64::from(i32::MAX) {
        i64::from(random.int(total as i32))
    } else {
        ((random.long() as u64) % total as u64) as i64
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
    total: i64,
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
                        e["weight"]
                            .as_i64()
                            .unwrap_or(1)
                            .clamp(0, i64::from(i32::MAX)) as i32,
                        state(&e["data"])?,
                    ))
                })
                .collect::<Result<_, DecodeError>>()?,
            _ => return Err(DecodeError::Field("set_block state_provider")),
        };
        let total = states.iter().map(|(w, _)| i64::from(*w)).sum();
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
        let mut at = draw(self.total, random);
        for (weight, state) in &self.states {
            at -= i64::from(*weight);
            if at < 0 {
                return *state;
            }
        }
        self.states[0].1
    }
}

#[cfg(test)]
mod tests;
