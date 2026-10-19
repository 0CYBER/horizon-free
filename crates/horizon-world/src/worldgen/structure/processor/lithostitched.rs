//! Lithostitched's processors that pick processors, carried as data
//! (`horizon:apply_random` and `horizon:condition`): one runs a processor
//! list drawn by weight, the other one list or another by a test of the
//! block. Both draw from Lithostitched's RandomSettings: a random seeded from
//! the world seed and a name, at the block, the piece or the structure.
//! `horizon:set_block` is its `set_block`, which the conditions mostly run.
use super::{Processor, ProcessorList};
use crate::{
    registry::{BlockRegistry, BlockStateId},
    worldgen::{DecodeError, block_feature::FeatureBlockBindings, structure::StructureResources},
};
use horizon_core::{
    position::BlockPos,
    random::{LegacyRandom, PositionalRandom, RandomSource},
};
use serde_json::Value;
use std::{collections::BTreeSet, sync::Arc};

/// RandomSettings: where the random is taken, and the name salting it.
#[derive(Clone, Debug, PartialEq)]
pub struct RandomSettings {
    mode: Mode,
    /// ResourceLocation.hashCode of the name (`lithostitched:default`).
    salt: i32,
}
#[derive(Clone, Copy, Debug, PartialEq)]
enum Mode {
    Block,
    Piece,
    Structure,
}
impl RandomSettings {
    /// A mode alone, or `{"mode": ..., "name": ...}`; per block when unset.
    fn decode(v: Option<&Value>) -> Result<Self, DecodeError> {
        let mode = Mode::decode;
        let (mode, name) = match v {
            None => (Mode::Block, "lithostitched:default"),
            Some(Value::String(m)) => (mode(Some(m))?, "lithostitched:default"),
            Some(o) => (
                mode(o["mode"].as_str())?,
                o["name"]
                    .as_str()
                    .ok_or(DecodeError::Field("random name"))?,
            ),
        };
        let (namespace, path) = name.split_once(':').unwrap_or(("minecraft", name));
        Ok(Self {
            mode,
            salt: java_hash(namespace)
                .wrapping_mul(31)
                .wrapping_add(java_hash(path)),
        })
    }
    /// RandomSource.create(seed + hash).forkPositional().at(the chosen
    /// position): the piece's position, the structure's pivot or the block.
    pub(super) fn create(
        &self,
        seed: i64,
        piece: [i32; 3],
        pivot: [i32; 3],
        block: [i32; 3],
    ) -> LegacyRandom {
        let [x, y, z] = self.mode.select(piece, pivot, block);
        LegacyRandom::new(seed.wrapping_add(i64::from(self.salt)))
            .fork_positional()
            .at(BlockPos::new(x, y, z))
    }
}
impl Mode {
    fn decode(m: Option<&str>) -> Result<Self, DecodeError> {
        match m {
            None | Some("per_block") => Ok(Self::Block),
            Some("per_piece") => Ok(Self::Piece),
            Some("per_structure") => Ok(Self::Structure),
            _ => Err(DecodeError::Field("random mode")),
        }
    }
    fn select(self, piece: [i32; 3], pivot: [i32; 3], block: [i32; 3]) -> [i32; 3] {
        match self {
            Self::Block => block,
            Self::Piece => piece,
            Self::Structure => pivot,
        }
    }
}

/// SetBlockProcessor with a simple or weighted state provider and no block
/// entity change: the block becomes the state, keeping the properties it
/// shares with the old one unless `preserve_state` is off.
#[derive(Clone, Debug)]
pub struct SetBlock {
    /// The states and their weights; one state alone is a simple provider,
    /// which draws nothing.
    states: Vec<(BlockStateId, i32)>,
    weighted: bool,
    preserve: bool,
    mode: Mode,
}
impl SetBlock {
    pub(super) fn decode(
        v: &Value,
        state: impl Fn(&Value) -> Result<BlockStateId, DecodeError>,
    ) -> Result<Self, DecodeError> {
        let provider = &v["state_provider"];
        let (states, weighted) = match provider["type"].as_str() {
            Some("minecraft:simple_state_provider") => {
                (vec![(state(&provider["state"])?, 1)], false)
            }
            Some("minecraft:weighted_state_provider") => (
                provider["entries"]
                    .as_array()
                    .ok_or(DecodeError::Field("set_block entries"))?
                    .iter()
                    .map(|e| {
                        let weight = e["weight"]
                            .as_i64()
                            .ok_or(DecodeError::Field("set_block weight"))?;
                        Ok((state(&e["data"])?, weight as i32))
                    })
                    .collect::<Result<_, DecodeError>>()?,
                true,
            ),
            _ => return Err(DecodeError::Field("set_block state_provider")),
        };
        if states.is_empty() {
            return Err(DecodeError::Field("set_block entries"));
        }
        Ok(Self {
            states,
            weighted,
            preserve: v["preserve_state"].as_bool().unwrap_or(true),
            mode: Mode::decode(v["random_mode"].as_str())?,
        })
    }
    /// The state to set, before `preserve`: drawn with
    /// RandomSource.create(seed).forkPositional().at(the chosen position).
    pub(super) fn state(
        &self,
        seed: i64,
        piece: [i32; 3],
        pivot: [i32; 3],
        block: [i32; 3],
    ) -> BlockStateId {
        if !self.weighted {
            return self.states[0].0;
        }
        let [x, y, z] = self.mode.select(piece, pivot, block);
        let mut random = LegacyRandom::new(seed)
            .fork_positional()
            .at(BlockPos::new(x, y, z));
        let total: i32 = self.states.iter().map(|s| s.1).sum();
        let mut value = random.next_int_bound(total.max(1)).unwrap_or(0);
        for (state, weight) in &self.states {
            if value < *weight {
                return *state;
            }
            value -= weight;
        }
        self.states[0].0
    }
    pub(super) fn preserves(&self) -> bool {
        self.preserve
    }
}

/// String.hashCode over UTF-16 units.
fn java_hash(s: &str) -> i32 {
    s.encode_utf16()
        .fold(0i32, |h, c| h.wrapping_mul(31).wrapping_add(i32::from(c)))
}

fn list(
    v: &Value,
    resources: &StructureResources,
    bindings: &FeatureBlockBindings<'_>,
) -> Result<ProcessorList, DecodeError> {
    ProcessorList::decode(&v.to_string(), resources, bindings)
}

/// ApplyRandomStructureProcessor: a weighted list of processor lists, one
/// drawn per block with `nextInt(total)`.
#[derive(Clone, Debug)]
pub struct ApplyRandom {
    pub(super) random: RandomSettings,
    lists: Vec<(ProcessorList, i32)>,
    total: i32,
}
impl ApplyRandom {
    pub(super) fn decode(
        v: &Value,
        resources: &StructureResources,
        bindings: &FeatureBlockBindings<'_>,
    ) -> Result<Self, DecodeError> {
        let lists = v["processor_lists"]
            .as_array()
            .ok_or(DecodeError::Field("apply_random processor_lists"))?
            .iter()
            .map(|e| {
                let weight = e["weight"]
                    .as_i64()
                    .ok_or(DecodeError::Field("apply_random weight"))?;
                Ok((list(&e["data"], resources, bindings)?, weight as i32))
            })
            .collect::<Result<Vec<_>, DecodeError>>()?;
        Ok(Self {
            random: RandomSettings::decode(v.get("mode"))?,
            total: lists.iter().map(|l| l.1).sum(),
            lists,
        })
    }
    /// WeightedList.getRandom: none when the weights add up to 0.
    pub(super) fn pick(&self, random: &mut impl RandomSource) -> Option<&ProcessorList> {
        if self.total <= 0 {
            return None;
        }
        let mut value = random.next_int_bound(self.total).unwrap_or(0);
        for (list, weight) in &self.lists {
            if value < *weight {
                return Some(list);
            }
            value -= weight;
        }
        None
    }
}

/// ConditionProcessor: `then` when the condition holds for the block, else
/// `else`.
#[derive(Clone, Debug)]
pub struct Conditional {
    pub(super) random: RandomSettings,
    pub(super) condition: Condition,
    pub(super) then: ProcessorList,
    pub(super) otherwise: ProcessorList,
}
impl Conditional {
    pub(super) fn decode(
        v: &Value,
        resources: &StructureResources,
        bindings: &FeatureBlockBindings<'_>,
    ) -> Result<Self, DecodeError> {
        // A processor alone or a list of them.
        let processors = |key: &str| -> Result<ProcessorList, DecodeError> {
            match v.get(key) {
                None => Ok(ProcessorList::default()),
                Some(Value::Object(p)) if p.contains_key("processor_type") => Ok(ProcessorList(
                    vec![Processor::decode(&v[key], resources, bindings)?],
                )),
                Some(other) => list(other, resources, bindings),
            }
        };
        Ok(Self {
            random: RandomSettings::decode(v.get("random_mode"))?,
            condition: Condition::decode(&v["if_true"], bindings)?,
            then: processors("then")?,
            otherwise: processors("else")?,
        })
    }
}

/// Lithostitched's processor conditions the game runs.
#[derive(Clone, Debug)]
pub(super) enum Condition {
    True,
    AllOf(Vec<Condition>),
    AnyOf(Vec<Condition>),
    Not(Box<Condition>),
    RandomChance(f32),
    /// The block placed (`input`) or the one in the world (`location`) is
    /// one of these, with these property values.
    MatchingBlocks {
        blocks: Arc<BTreeSet<BlockStateId>>,
        properties: Vec<(String, String)>,
        location: bool,
    },
}
impl Condition {
    fn decode(v: &Value, bindings: &FeatureBlockBindings<'_>) -> Result<Self, DecodeError> {
        let all = |a: &Value| -> Result<Vec<Condition>, DecodeError> {
            a.as_array()
                .ok_or(DecodeError::Field("processor conditions"))?
                .iter()
                .map(|c| Self::decode(c, bindings))
                .collect()
        };
        if v.is_array() {
            return Ok(Self::AllOf(all(v)?));
        }
        let kind = v["type"].as_str().unwrap_or_default();
        Ok(match kind.strip_prefix("lithostitched:").unwrap_or(kind) {
            "true" => Self::True,
            "all_of" => Self::AllOf(all(&v["conditions"])?),
            "any_of" => Self::AnyOf(all(&v["conditions"])?),
            "not" => Self::Not(Box::new(Self::decode(&v["condition"], bindings)?)),
            "random_chance" => Self::RandomChance(super::float(v, "chance")?),
            "matching_blocks" => Self::MatchingBlocks {
                blocks: super::holder_set(&v["blocks"], bindings)?,
                properties: match v.get("properties") {
                    None => Vec::new(),
                    Some(p) => p
                        .as_object()
                        .ok_or(DecodeError::Field("matching_blocks properties"))?
                        .iter()
                        .map(|(k, x)| {
                            let x = x
                                .as_str()
                                .ok_or(DecodeError::Field("matching_blocks property"))?;
                            Ok((k.clone(), x.to_owned()))
                        })
                        .collect::<Result<_, DecodeError>>()?,
                },
                location: match v["match_type"].as_str() {
                    None | Some("input") => false,
                    Some("location") => true,
                    _ => return Err(DecodeError::Field("matching_blocks match_type")),
                },
            },
            _ => return Err(DecodeError::Field("processor condition type")),
        })
    }
    /// ProcessorCondition.test with the block placed and the one there.
    pub(super) fn test(
        &self,
        input: BlockStateId,
        location: BlockStateId,
        registry: &BlockRegistry,
        random: &mut impl RandomSource,
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
            Self::RandomChance(chance) => random.next_float() < *chance,
            Self::MatchingBlocks {
                blocks,
                properties,
                location: at_location,
            } => {
                let state = if *at_location { location } else { input };
                blocks.contains(&state)
                    && properties.iter().all(|(k, x)| {
                        registry
                            .state(state)
                            .and_then(|s| s.properties.get(k))
                            .is_some_and(|v| v == x)
                    })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn random_settings_salt_and_position() {
        // ResourceLocation.hashCode: 31 * namespace + path.
        let default = RandomSettings::decode(Some(&"per_piece".into())).unwrap();
        assert_eq!(
            default.salt,
            java_hash("lithostitched")
                .wrapping_mul(31)
                .wrapping_add(java_hash("default"))
        );
        assert_eq!(java_hash("default"), 1544803905);
        let (seed, piece, pivot, block) = (12345, [16, 64, -32], [8, 70, 8], [17, 65, -30]);
        let mut ours = default.create(seed, piece, pivot, block);
        let mut theirs = LegacyRandom::new(seed + i64::from(default.salt))
            .fork_positional()
            .at(BlockPos::new(16, 64, -32));
        assert_eq!(ours.next_long(), theirs.next_long());
        let named: Value = serde_json::json!({"mode": "per_structure", "name": "v:roof"});
        let named = RandomSettings::decode(Some(&named)).unwrap();
        assert_eq!(named.mode, Mode::Structure);
        assert_eq!(
            named.salt,
            java_hash("v")
                .wrapping_mul(31)
                .wrapping_add(java_hash("roof"))
        );
    }

    #[test]
    fn apply_random_draws_by_weight() {
        let apply = ApplyRandom {
            random: RandomSettings {
                mode: Mode::Block,
                salt: 0,
            },
            lists: vec![
                (ProcessorList(vec![Processor::Nop]), 3),
                (ProcessorList::default(), 1),
            ],
            total: 4,
        };
        let mut ours = LegacyRandom::new(9);
        let mut theirs = LegacyRandom::new(9);
        for _ in 0..32 {
            let draw = theirs.next_int_bound(4).unwrap();
            let picked = apply.pick(&mut ours).unwrap();
            assert_eq!(picked.0.len(), usize::from(draw < 3));
        }
    }

    #[test]
    fn set_block_draws_from_the_unsalted_seed() {
        let (a, b) = (BlockStateId(1), BlockStateId(2));
        let set = SetBlock::decode(
            &serde_json::json!({"state_provider": {"type": "minecraft:weighted_state_provider",
                "entries": [{"weight": 1, "data": 1}, {"weight": 2, "data": 2}]},
                "random_mode": "per_piece"}),
            |v| Ok(BlockStateId(v.as_u64().unwrap() as u32)),
        )
        .unwrap();
        assert!(set.preserves());
        for x in 0..16 {
            let mut java = LegacyRandom::new(77)
                .fork_positional()
                .at(BlockPos::new(x, 5, 3));
            let want = if java.next_int_bound(3).unwrap() < 1 {
                a
            } else {
                b
            };
            assert_eq!(set.state(77, [x, 5, 3], [0; 3], [9, 9, 9]), want);
        }
    }
}
