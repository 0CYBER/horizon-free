//! Initial world-position predicates with ordered short-circuit evaluation.
use super::anchor::VerticalAnchor;
use super::placement::{PlacementContext, PlacementQueryError};
use super::{DecodeError, Object, density::identifier, integer, optional, required};
use crate::{
    block_properties::{BlockProperty, BoundBlockProperty},
    registry::BlockStateId,
};
use horizon_core::geometry::Direction;
use serde_json::value::RawValue;
use std::{collections::BTreeSet, sync::Arc};

/// `predicate_property` for resolvers that hold the bound physical properties
/// (`block_feature` and `state_provider`), in `BlockProperty` order.
pub(super) fn bound_property(
    values: Option<&[BoundBlockProperty; 4]>,
    property: BlockProperty,
) -> Result<BoundBlockProperty, DecodeError> {
    let values = values.ok_or(DecodeError::Field("predicate block property registry"))?;
    Ok(values[match property {
        BlockProperty::Solid => 0,
        BlockProperty::Replaceable => 1,
        BlockProperty::Liquid => 2,
        BlockProperty::SolidRender => 3,
    }]
    .clone())
}

/// Resolve a block holder set against the same immutable registry used by world
/// queries. Unknown blocks/tags fail during decoding, even in unused branches.
pub trait PredicateBlockResolver {
    fn predicate_state(&self, _json: &str) -> Result<BlockStateId, DecodeError> {
        Err(DecodeError::Field("predicate block state registry"))
    }
    fn predicate_property(
        &self,
        _property: BlockProperty,
    ) -> Result<BoundBlockProperty, DecodeError> {
        Err(DecodeError::Field("predicate block property registry"))
    }
    fn predicate_biome_set(
        &self,
        _json: &str,
    ) -> Result<Arc<BTreeSet<super::biome::BiomeId>>, DecodeError> {
        Err(DecodeError::Field("predicate biome registry"))
    }
    fn predicate_fluid_set(
        &self,
        _json: &str,
    ) -> Result<super::fluid_predicate::BoundFluidSet, DecodeError> {
        Err(DecodeError::Field("predicate fluid registry"))
    }
    fn predicate_block_set(&self, json: &str) -> Result<Arc<BTreeSet<BlockStateId>>, DecodeError>;
}

#[derive(Clone, Debug)]
pub enum BlockPredicate {
    True,
    Unobstructed,
    WouldSurvive {
        state: BlockStateId,
        offset: [i32; 3],
    },
    HasSturdyFace {
        direction: Direction,
        offset: [i32; 3],
    },
    Property {
        property: BoundBlockProperty,
        offset: [i32; 3],
    },
    MatchingBiomes(Arc<BTreeSet<super::biome::BiomeId>>),
    MatchingFluids {
        fluids: super::fluid_predicate::BoundFluidSet,
        offset: [i32; 3],
    },
    MatchingBlocks {
        states: Arc<BTreeSet<BlockStateId>>,
        offset: [i32; 3],
    },
    MatchingBlockTag {
        tag: String,
        offset: [i32; 3],
    },
    HeightRange {
        min: VerticalAnchor,
        max: VerticalAnchor,
    },
    Volume {
        min: [i32; 3],
        max: [i32; 3],
        predicate: Box<Self>,
    },
    Not(Box<Self>),
    All(Vec<Self>),
    Any(Vec<Self>),
    InsideWorldBounds([i32; 3]),
}
impl BlockPredicate {
    pub fn from_json(json: &str) -> Result<Self, DecodeError> {
        Self::decode(json, 0, &mut 65536)
    }
    pub(crate) fn decode(
        json: &str,
        depth: usize,
        remaining: &mut usize,
    ) -> Result<Self, DecodeError> {
        Self::decode_with_blocks(json, depth, remaining, None)
    }
    pub fn from_json_with_blocks(
        json: &str,
        blocks: &impl PredicateBlockResolver,
    ) -> Result<Self, DecodeError> {
        Self::decode_with_blocks(json, 0, &mut 65536, Some(blocks))
    }
    pub(crate) fn decode_with_blocks(
        json: &str,
        depth: usize,
        remaining: &mut usize,
        blocks: Option<&dyn PredicateBlockResolver>,
    ) -> Result<Self, DecodeError> {
        if depth > 256 || *remaining == 0 {
            return Err(DecodeError::Field("block predicate depth or node count"));
        }
        *remaining -= 1;
        let object: Object = serde_json::from_str(json)?;
        let kind = identifier(&serde_json::from_str::<String>(
            required(&object, "type")?.get(),
        )?)?;
        match kind.as_str() {
            "minecraft:would_survive" => Ok(Self::WouldSurvive {
                state: blocks
                    .ok_or(DecodeError::Field("predicate block state registry"))?
                    .predicate_state(required(&object, "state")?.get())?,
                offset: optional(&object, "offset")
                    .map(offset)
                    .transpose()?
                    .unwrap_or([0; 3]),
            }),
            "minecraft:has_sturdy_face" => {
                let name: String = serde_json::from_str(required(&object, "direction")?.get())?;
                let direction = match name.as_str() {
                    "down" => Direction::Down,
                    "up" => Direction::Up,
                    "north" => Direction::North,
                    "south" => Direction::South,
                    "west" => Direction::West,
                    "east" => Direction::East,
                    _ => return Err(DecodeError::Field("predicate direction")),
                };
                Ok(Self::HasSturdyFace {
                    direction,
                    offset: optional(&object, "offset")
                        .map(offset)
                        .transpose()?
                        .unwrap_or([0; 3]),
                })
            }

            "minecraft:solid" | "minecraft:replaceable" => Ok(Self::Property {
                property: blocks
                    .ok_or(DecodeError::Field("predicate block property registry"))?
                    .predicate_property(if kind == "minecraft:solid" {
                        BlockProperty::Solid
                    } else {
                        BlockProperty::Replaceable
                    })?,
                offset: optional(&object, "offset")
                    .map(offset)
                    .transpose()?
                    .unwrap_or([0; 3]),
            }),
            "minecraft:unobstructed" => {
                if let Some(raw) = optional(&object, "offset") {
                    vector(raw)?;
                }
                // The pinned evaluator deliberately ignores its decoded offset.
                Ok(Self::Unobstructed)
            }
            "minecraft:matching_biomes" => Ok(Self::MatchingBiomes(
                blocks
                    .ok_or(DecodeError::Field("predicate biome registry"))?
                    .predicate_biome_set(required(&object, "biomes")?.get())?,
            )),
            "minecraft:matching_fluids" => Ok(Self::MatchingFluids {
                fluids: blocks
                    .ok_or(DecodeError::Field("predicate fluid registry"))?
                    .predicate_fluid_set(required(&object, "fluids")?.get())?,
                offset: optional(&object, "offset")
                    .map(offset)
                    .transpose()?
                    .unwrap_or([0; 3]),
            }),
            "minecraft:matching_blocks" => Ok(Self::MatchingBlocks {
                states: blocks
                    .ok_or(DecodeError::Field("predicate block registry"))?
                    .predicate_block_set(required(&object, "blocks")?.get())?,
                offset: optional(&object, "offset")
                    .map(offset)
                    .transpose()?
                    .unwrap_or([0; 3]),
            }),
            "minecraft:matching_block_tag" => Ok(Self::MatchingBlockTag {
                tag: super::tags::identifier(&serde_json::from_str::<String>(
                    required(&object, "tag")?.get(),
                )?)?,
                offset: optional(&object, "offset")
                    .map(offset)
                    .transpose()?
                    .unwrap_or([0; 3]),
            }),
            "minecraft:height_range" => Ok(Self::HeightRange {
                min: VerticalAnchor::from_json(required(&object, "min_inclusive")?.get())?,
                max: VerticalAnchor::from_json(required(&object, "max_inclusive")?.get())?,
            }),
            "minecraft:volume_match" => {
                let min = offset(required(&object, "min")?)?;
                let max = offset(required(&object, "max")?)?;
                if (0..3).any(|i| min[i] > max[i]) {
                    return Err(DecodeError::Field("predicate volume bounds"));
                }
                Ok(Self::Volume {
                    min,
                    max,
                    predicate: Box::new(Self::decode_with_blocks(
                        required(&object, "match")?.get(),
                        depth + 1,
                        remaining,
                        blocks,
                    )?),
                })
            }
            "minecraft:true" => Ok(Self::True),
            "minecraft:not" => Ok(Self::Not(Box::new(Self::decode_with_blocks(
                required(&object, "predicate")?.get(),
                depth + 1,
                remaining,
                blocks,
            )?))),
            "minecraft:all_of" | "minecraft:any_of" => {
                let values: Vec<Box<RawValue>> =
                    serde_json::from_str(required(&object, "predicates")?.get())?;
                if values.len() > 8192 {
                    return Err(DecodeError::Field("predicate list"));
                }
                let values = values
                    .iter()
                    .map(|v| Self::decode_with_blocks(v.get(), depth + 1, remaining, blocks))
                    .collect::<Result<_, _>>()?;
                Ok(if kind == "minecraft:all_of" {
                    Self::All(values)
                } else {
                    Self::Any(values)
                })
            }
            "minecraft:inside_world_bounds" => Ok(Self::InsideWorldBounds(
                optional(&object, "offset")
                    .map(offset)
                    .transpose()?
                    .unwrap_or([0; 3]),
            )),
            _ => Err(DecodeError::Field("block predicate type")),
        }
    }
    pub fn test(
        &self,
        world: &mut impl PlacementContext,
        position: [i32; 3],
    ) -> Result<bool, PlacementQueryError> {
        self.test_inner(world, position, 0, &mut 65536)
    }
    pub(crate) fn test_inner(
        &self,
        world: &mut impl PlacementContext,
        position: [i32; 3],
        depth: usize,
        remaining: &mut usize,
    ) -> Result<bool, PlacementQueryError> {
        if depth > 256 || *remaining == 0 {
            return Err(PlacementQueryError::WorkLimit);
        }
        *remaining -= 1;
        match self {
            Self::WouldSurvive { state, offset } => world.block_survives(
                *state,
                std::array::from_fn(|i| position[i].wrapping_add(offset[i])),
            ),
            Self::Property { property, offset } => {
                let state = world
                    .block_state(std::array::from_fn(|i| position[i].wrapping_add(offset[i])))?;
                Ok(property.contains(state))
            }
            Self::HasSturdyFace { direction, offset } => world.face_sturdy(
                std::array::from_fn(|i| position[i].wrapping_add(offset[i])),
                *direction,
            ),
            Self::Unobstructed => world.unobstructed_block(position),
            Self::MatchingBiomes(biomes) => Ok(biomes.contains(&world.biome_identity(position)?)),
            Self::MatchingFluids { fluids, offset } => {
                let state = world
                    .block_state(std::array::from_fn(|i| position[i].wrapping_add(offset[i])))?;
                Ok(fluids.contains(state))
            }
            Self::MatchingBlocks { states, offset } => {
                let state = world
                    .block_state(std::array::from_fn(|i| position[i].wrapping_add(offset[i])))?;
                Ok(states.contains(&state))
            }
            Self::MatchingBlockTag { tag, offset } => world.block_matches_tag(
                std::array::from_fn(|i| position[i].wrapping_add(offset[i])),
                tag,
            ),
            Self::HeightRange { min, max } => {
                let context = world.predicate_generation()?;
                let min = min.resolve(context);
                let max = max.resolve(context);
                Ok(position[1] >= min && position[1] <= max)
            }
            Self::Volume {
                min,
                max,
                predicate,
            } => {
                // Native order is X, Z, Y. Each child shares the evaluation
                // budget, including nested volumes and early-failing queries.
                for x in min[0]..=max[0] {
                    for z in min[2]..=max[2] {
                        for y in min[1]..=max[1] {
                            let offset = [x, y, z];
                            let p = std::array::from_fn(|i| position[i].wrapping_add(offset[i]));
                            if !predicate.test_inner(world, p, depth + 1, remaining)? {
                                return Ok(false);
                            }
                        }
                    }
                }
                Ok(true)
            }
            Self::True => Ok(true),
            Self::Not(value) => Ok(!value.test_inner(world, position, depth + 1, remaining)?),
            Self::All(values) | Self::Any(values) => {
                let all = matches!(self, Self::All(_));
                for value in values {
                    let result = value.test_inner(world, position, depth + 1, remaining)?;
                    if result != all {
                        return Ok(result);
                    }
                }
                Ok(all)
            }
            Self::InsideWorldBounds(offset) => world
                .inside_build_height(std::array::from_fn(|i| position[i].wrapping_add(offset[i]))),
        }
    }
}

fn vector(raw: &RawValue) -> Result<[i32; 3], DecodeError> {
    let values: Vec<Box<RawValue>> = serde_json::from_str(raw.get())?;
    if values.len() != 3 {
        return Err(DecodeError::Field("predicate offset"));
    }
    let mut result = [0; 3];
    for (out, value) in result.iter_mut().zip(values.iter()) {
        *out = integer(value)?;
    }
    Ok(result)
}

fn offset(raw: &RawValue) -> Result<[i32; 3], DecodeError> {
    let result = vector(raw)?;
    // Native Math.abs(MIN_VALUE) wraps and passes this codec.
    if result.iter().any(|v| v.wrapping_abs() > 16) {
        return Err(DecodeError::Field("predicate offset"));
    }
    Ok(result)
}
