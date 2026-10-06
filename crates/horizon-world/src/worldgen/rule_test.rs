//! Registry-bound replacement predicates with ordered short-circuit evaluation.
use super::{
    DecodeError, Object, block_feature::FeatureBlockBindings, density::identifier, integer,
    number_text, required,
};
use crate::registry::BlockStateId;
use horizon_core::random::RandomSource;
use serde_json::value::RawValue;
use std::{collections::BTreeSet, sync::Arc};

#[derive(Clone, Debug)]
pub struct RuleTest(Node);
#[derive(Clone, Debug)]
enum Node {
    True,
    Not(Box<RuleTest>),
    All(Vec<RuleTest>),
    Any(Vec<RuleTest>),
    States(Arc<BTreeSet<BlockStateId>>),
    State(BlockStateId),
    RandomStates(Arc<BTreeSet<BlockStateId>>, f32),
    RandomState(BlockStateId, f32),
    Height(i32, i32),
}
impl RuleTest {
    pub fn from_json(json: &str, bindings: &FeatureBlockBindings<'_>) -> Result<Self, DecodeError> {
        Self::decode(json, bindings, 0, &mut 65536)
    }
    pub(crate) fn decode(
        json: &str,
        bindings: &FeatureBlockBindings<'_>,
        depth: usize,
        remaining: &mut usize,
    ) -> Result<Self, DecodeError> {
        if depth > 128 || *remaining == 0 {
            return Err(DecodeError::Field("replacement rule depth or node count"));
        }
        *remaining -= 1;
        let object: Object = serde_json::from_str(json)?;
        let name = |field| -> Result<String, DecodeError> {
            identifier(&serde_json::from_str::<String>(
                required(&object, field)?.get(),
            )?)
        };
        let state = || {
            bindings
                .codec
                .decode(required(&object, "block_state")?.get(), false)
                .map_err(|e| DecodeError::Registry(e.to_string()))
        };
        let block = || {
            let name = name("block")?;
            bindings
                .blocks
                .get(&name)
                .cloned()
                .ok_or_else(|| DecodeError::Registry(format!("unknown block {name}")))
        };
        // These native codecs use unrestricted FLOAT, including out-of-range
        // probabilities. Matching the state still consumes a draw at zero/one.
        let probability = || {
            number_text(required(&object, "probability")?)?
                .parse::<f32>()
                .map_err(|_| DecodeError::Field("probability"))
        };
        let node = match name("predicate_type")?.as_str() {
            "minecraft:always_true" => Node::True,
            "minecraft:block_match" => Node::States(block()?),
            "minecraft:blockstate_match" => Node::State(state()?),
            "minecraft:tag_match" => Node::States(
                bindings
                    .tags
                    .get(&super::tags::identifier(&serde_json::from_str::<String>(
                        required(&object, "tag")?.get(),
                    )?)?)
                    .cloned()
                    .unwrap_or_default(),
            ),
            "minecraft:random_block_match" => Node::RandomStates(block()?, probability()?),
            "minecraft:random_blockstate_match" => Node::RandomState(state()?, probability()?),
            "minecraft:height_match" => Node::Height(
                integer(required(&object, "min_inclusive")?)?,
                integer(required(&object, "max_inclusive")?)?,
            ),
            "minecraft:not" => Node::Not(Box::new(Self::decode(
                required(&object, "rule")?.get(),
                bindings,
                depth + 1,
                remaining,
            )?)),
            kind @ ("minecraft:all_of" | "minecraft:any_of") => {
                let values: Vec<Box<RawValue>> =
                    serde_json::from_str(required(&object, "rules")?.get())?;
                if values.len() > 8192 {
                    return Err(DecodeError::Field("replacement rule list"));
                }
                let values = values
                    .iter()
                    .map(|v| Self::decode(v.get(), bindings, depth + 1, remaining))
                    .collect::<Result<_, _>>()?;
                if kind == "minecraft:all_of" {
                    Node::All(values)
                } else {
                    Node::Any(values)
                }
            }
            _ => return Err(DecodeError::Field("replacement rule type")),
        };
        Ok(Self(node))
    }
    pub fn test(
        &self,
        state: BlockStateId,
        position: [i32; 3],
        random: &mut impl RandomSource,
    ) -> bool {
        match &self.0 {
            Node::True => true,
            Node::Not(rule) => !rule.test(state, position, random),
            Node::All(rules) => rules.iter().all(|rule| rule.test(state, position, random)),
            Node::Any(rules) => rules.iter().any(|rule| rule.test(state, position, random)),
            Node::States(states) => states.contains(&state),
            Node::State(expected) => state == *expected,
            Node::RandomStates(states, probability) => {
                states.contains(&state) && random.next_float() < *probability
            }
            Node::RandomState(expected, probability) => {
                state == *expected && random.next_float() < *probability
            }
            Node::Height(min, max) => *min <= position[1] && position[1] <= *max,
        }
    }
}
