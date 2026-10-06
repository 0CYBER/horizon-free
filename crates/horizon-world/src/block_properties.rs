//! Immutable block-definition flags and legacy solidity calculated from cached
//! collision shapes. Replacement with an item/fluid context is separate behavior.
use crate::{
    DATA_VERSION,
    chunk::BlockState,
    collision::CachedCollisionRegistry,
    compression::invalid,
    nbt::Tag,
    registry::{BlockRegistry, BlockStateId},
};
use horizon_core::geometry::{ShapeError, ShapeOp, VoxelShape};
use std::{collections::BTreeMap, io, sync::Arc};

#[derive(Clone, Copy, Debug)]
pub struct BlockDefinitionProperties {
    pub force_solid_on: bool,
    pub force_solid_off: bool,
    pub replaceable: bool,
    pub liquid: bool,
    pub can_occlude: bool,
}
impl BlockDefinitionProperties {
    pub fn calculate_solid_render(self, occlusion: &VoxelShape) -> Result<bool, ShapeError> {
        Ok(self.can_occlude && !occlusion.join_is_not_empty(&VoxelShape::block(), ShapeOp::XOR)?)
    }

    /// Force-on wins even if both definition flags are set. A missing cache
    /// counts as non-solid; it is distinct from a missing collision implementation.
    pub fn calculate_solid(self, cached_shape: Option<&VoxelShape>) -> bool {
        if self.force_solid_on {
            return true;
        }
        if self.force_solid_off {
            return false;
        }
        cached_shape
            .and_then(VoxelShape::bounds)
            .is_some_and(|bounds| {
                bounds.average_size() >= 0.7291666666666666 || bounds.size().y >= 1.0
            })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockProperty {
    Solid,
    Replaceable,
    Liquid,
    SolidRender,
}
impl BlockProperty {
    fn mask(self) -> u8 {
        match self {
            Self::Solid => 1,
            Self::Replaceable => 2,
            Self::Liquid => 4,
            Self::SolidRender => 8,
        }
    }
}
/// An immutable property selection, shared by decoded block predicates.
#[derive(Clone, Debug)]
pub struct BoundBlockProperty {
    states: Arc<[u8]>,
    property: BlockProperty,
}
impl BoundBlockProperty {
    pub fn contains(&self, id: BlockStateId) -> bool {
        self.states
            .get(id.0 as usize)
            .is_some_and(|flags| flags & self.property.mask() != 0)
    }
}

pub struct BlockPropertiesRegistry<'r> {
    registry: &'r BlockRegistry,
    definitions: BTreeMap<String, BlockDefinitionProperties>,
    states: Arc<[u8]>,
}
impl<'r> BlockPropertiesRegistry<'r> {
    pub fn from_nbt(tag: &Tag, collisions: &CachedCollisionRegistry<'r>) -> io::Result<Self> {
        let registry = collisions.registry();
        if tag.get("DataVersion") != Some(&Tag::Int(DATA_VERSION)) {
            return Err(invalid("block properties DataVersion mismatch"));
        }
        let Some(Tag::List(states)) = tag.get("states") else {
            return Err(invalid("missing block property state identities"));
        };
        if states.len() != registry.len() {
            return Err(invalid("block property registry size mismatch"));
        }
        for (id, state) in states.iter().enumerate() {
            if registry.state(BlockStateId(id as u32)) != Some(&BlockState::from_nbt(state)?) {
                return Err(invalid("block property state identities differ"));
            }
        }
        let Some(Tag::Compound(blocks)) = tag.get("blocks") else {
            return Err(invalid("missing block definition properties"));
        };
        let mut definitions = BTreeMap::new();
        for (name, flags) in blocks {
            let name = name.to_string().map_err(|e| invalid(e.to_string()))?;
            let Tag::Byte(flags @ 0..=31) = flags else {
                return Err(invalid("invalid block definition flags"));
            };
            if registry.default_state(&name).is_none() {
                return Err(invalid("unknown block definition properties"));
            }
            definitions.insert(
                name,
                BlockDefinitionProperties {
                    force_solid_on: flags & 1 != 0,
                    force_solid_off: flags & 2 != 0,
                    replaceable: flags & 4 != 0,
                    liquid: flags & 8 != 0,
                    can_occlude: flags & 16 != 0,
                },
            );
        }
        let occlusion = CachedCollisionRegistry::from_nbt(
            tag.get("occlusion")
                .ok_or_else(|| invalid("missing block occlusion shapes"))?,
            registry,
        )?;
        let mut result = Vec::with_capacity(registry.len());
        for id in 0..registry.len() {
            let id = BlockStateId(id as u32);
            let state = registry.state(id).unwrap();
            let properties = definitions
                .get(&state.name)
                .ok_or_else(|| invalid("missing block definition properties"))?;
            result.push(
                u8::from(properties.calculate_solid(collisions.cached_shape(id)?))
                    | (u8::from(properties.replaceable) << 1)
                    | (u8::from(properties.liquid) << 2)
                    | (u8::from(
                        properties
                            .calculate_solid_render(
                                occlusion
                                    .cached_shape(id)?
                                    .ok_or_else(|| invalid("missing block occlusion shape"))?,
                            )
                            .map_err(|error| invalid(error.to_string()))?,
                    ) << 3),
            );
        }
        Ok(Self {
            registry,
            definitions,
            states: result.into(),
        })
    }
    pub fn registry(&self) -> &'r BlockRegistry {
        self.registry
    }
    pub fn definition(&self, name: &str) -> Option<BlockDefinitionProperties> {
        self.definitions.get(name).copied()
    }
    pub fn property(&self, id: BlockStateId, property: BlockProperty) -> io::Result<bool> {
        self.states
            .get(id.0 as usize)
            .map(|flags| flags & property.mask() != 0)
            .ok_or_else(|| invalid("block property state outside registry"))
    }
    pub fn bind(&self, property: BlockProperty) -> BoundBlockProperty {
        BoundBlockProperty {
            states: Arc::clone(&self.states),
            property,
        }
    }
}
