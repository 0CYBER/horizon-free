//! Native suffocation predicates bound to a registry and a resolved block-tag set.
use crate::{
    block_collision::{BlockCollisionRegistry, CollisionQueryError, DynamicCollisionWorld},
    compression::invalid,
    registry::BlockStateId,
};
use horizon_core::geometry::{ShapeOp, VoxelShape};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::Arc,
};

#[derive(Clone, Copy, Debug)]
pub enum SuffocationDefinition {
    TaggedFullCollision,
    Always,
    Never,
    NotExtendedPiston,
    ClosedShulker,
}
#[derive(Clone, Copy, Debug)]
enum Rule {
    Constant(bool),
    DynamicFull,
    Shulker,
}

pub struct SuffocationRegistry<'r> {
    collisions: Arc<BlockCollisionRegistry<'r>>,
    rules: Vec<Rule>,
}
impl<'r> SuffocationRegistry<'r> {
    pub fn collisions(&self) -> &BlockCollisionRegistry<'r> {
        &self.collisions
    }

    /// Rebind after the causes_suffocation tag changes. Definition keys and tag
    /// members are canonical block identifiers from the same pinned registry.
    pub fn new(
        collisions: Arc<BlockCollisionRegistry<'r>>,
        definitions: &BTreeMap<String, SuffocationDefinition>,
        causes_suffocation: &BTreeSet<String>,
    ) -> io::Result<Self> {
        let registry = collisions.registry();
        if definitions
            .keys()
            .chain(causes_suffocation)
            .any(|name| registry.default_state(name).is_none())
        {
            return Err(invalid("unknown suffocation definition or tag member"));
        }
        let mut full_shapes = HashMap::new();
        let mut rules = Vec::with_capacity(registry.len());
        for index in 0..registry.len() {
            let id = BlockStateId(index as u32);
            let state = registry.state(id).unwrap();
            let definition = definitions
                .get(&state.name)
                .ok_or_else(|| invalid("missing suffocation definition"))?;
            let rule = match definition {
                SuffocationDefinition::Always => Rule::Constant(true),
                SuffocationDefinition::Never => Rule::Constant(false),
                SuffocationDefinition::NotExtendedPiston => {
                    Rule::Constant(match state.properties.get("extended").map(String::as_str) {
                        Some("false") => true,
                        Some("true") => false,
                        _ => return Err(invalid("invalid piston suffocation property")),
                    })
                }
                SuffocationDefinition::ClosedShulker => Rule::Shulker,
                SuffocationDefinition::TaggedFullCollision => {
                    if !causes_suffocation.contains(&state.name) {
                        Rule::Constant(false)
                    } else if let Some(shape) = collisions.cached_shapes().cached_shape(id)? {
                        let identity = std::ptr::from_ref(shape);
                        let full = if let Some(full) = full_shapes.get(&identity) {
                            *full
                        } else {
                            let full = !shape
                                .join_is_not_empty(&VoxelShape::block(), ShapeOp::XOR)
                                .map_err(|e| invalid(e.to_string()))?;
                            full_shapes.insert(identity, full);
                            full
                        };
                        Rule::Constant(full)
                    } else {
                        Rule::DynamicFull
                    }
                }
            };
            rules.push(rule);
        }
        Ok(Self { collisions, rules })
    }

    pub fn is_suffocating<W: DynamicCollisionWorld>(
        &self,
        world: &mut W,
        state: BlockStateId,
        position: [i32; 3],
    ) -> Result<bool, CollisionQueryError<W::Error>> {
        let rule = self.rules.get(state.0 as usize).ok_or_else(|| {
            CollisionQueryError::Data(invalid("suffocation state outside registry"))
        })?;
        match rule {
            Rule::Constant(value) => Ok(*value),
            Rule::Shulker => world
                .shulker_is_closed(position)
                .map(|closed| closed != Some(false))
                .map_err(CollisionQueryError::Context),
            Rule::DynamicFull => self
                .collisions
                .collision_shape(world, state, position)?
                .join_is_not_empty(&VoxelShape::block(), ShapeOp::XOR)
                .map(|different| !different)
                .map_err(|error| CollisionQueryError::Data(invalid(error.to_string()))),
        }
    }
}
