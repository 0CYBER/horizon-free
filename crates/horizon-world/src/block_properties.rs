//! Immutable block-definition flags and legacy solidity calculated from cached
//! collision shapes. Replacement with an item/fluid context is separate behavior.
use crate::{
    DATA_VERSION,
    collision::CachedCollisionRegistry,
    compression::invalid,
    nbt::Tag,
    registry::{BlockRegistry, BlockStateId, BlockTable},
};
use horizon_core::geometry::{Direction, ShapeError, ShapeOp, VoxelShape};
use std::{
    collections::{BTreeMap, HashMap},
    io,
    sync::Arc,
};

#[derive(Clone, Copy, Debug)]
pub struct BlockDefinitionProperties {
    pub force_solid_on: bool,
    pub force_solid_off: bool,
    pub replaceable: bool,
    pub liquid: bool,
    pub can_occlude: bool,
}
impl BlockDefinitionProperties {
    pub fn calculate_full_occlusion_faces(self, occlusion: &VoxelShape) -> Result<u8, ShapeError> {
        let mut mask = 0;
        if self.can_occlude {
            for direction in Direction::ALL {
                if !occlusion
                    .face_shape(direction)?
                    .join_is_not_empty(&VoxelShape::block(), ShapeOp::XOR)?
                {
                    mask |= 1 << direction as u8;
                }
            }
        }
        Ok(mask)
    }
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
    mask: u8,
}
impl BoundBlockProperty {
    pub fn contains(&self, id: BlockStateId) -> bool {
        self.states
            .get(id.0 as usize)
            .is_some_and(|flags| flags & self.mask != 0)
    }
}
/// The per-state flag of the block definition's `canOcclude`, past the
/// [`BlockProperty`] bits.
const CAN_OCCLUDE: u8 = 16;

/// Immutable FULL face-occlusion results, independent of support/collision.
#[derive(Clone, Debug)]
pub struct BoundOcclusionFaces {
    states: Arc<[u8]>,
}
impl BoundOcclusionFaces {
    pub fn is_full(&self, state: BlockStateId, direction: Direction) -> io::Result<bool> {
        self.states
            .get(state.0 as usize)
            .map(|mask| mask & (1 << direction as u8) != 0)
            .ok_or_else(|| invalid("unknown occlusion state ID"))
    }
}

pub struct BlockPropertiesRegistry<'r> {
    registry: &'r BlockRegistry,
    definitions: BTreeMap<String, BlockDefinitionProperties>,
    states: Arc<[u8]>,
    occlusion_faces: BoundOcclusionFaces,
}
impl<'r> BlockPropertiesRegistry<'r> {
    pub fn from_nbt(tag: &Tag, collisions: &CachedCollisionRegistry<'r>) -> io::Result<Self> {
        Self::from_table(
            BlockTable::check(tag, collisions.registry(), "block property")?,
            collisions,
        )
    }
    pub fn from_table(
        table: BlockTable<'_, '_>,
        collisions: &CachedCollisionRegistry<'r>,
    ) -> io::Result<Self> {
        let registry = collisions.registry();
        if !std::ptr::eq(table.registry(), registry) {
            return Err(invalid(
                "block properties and collisions use other registries",
            ));
        }
        let tag = table.tag();
        if tag.get("DataVersion") != Some(&Tag::Int(DATA_VERSION)) {
            return Err(invalid("block properties DataVersion mismatch"));
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
        let occlusion = CachedCollisionRegistry::from_table(
            table.nested("occlusion", "block occlusion shapes")?,
        )?;
        let mut result = Vec::with_capacity(registry.len());
        let mut faces = Vec::with_capacity(registry.len());
        let mut face_cache = HashMap::new();
        for id in 0..registry.len() {
            let id = BlockStateId(id as u32);
            let state = registry.state(id).unwrap();
            let properties = definitions
                .get(&state.name)
                .ok_or_else(|| invalid("missing block definition properties"))?;
            let shape = occlusion
                .cached_shape(id)?
                .ok_or_else(|| invalid("missing block occlusion shape"))?;
            let mask = if properties.can_occlude {
                let key = std::ptr::from_ref(shape);
                if let Some(mask) = face_cache.get(&key) {
                    *mask
                } else {
                    let mask = properties
                        .calculate_full_occlusion_faces(shape)
                        .map_err(|e| invalid(e.to_string()))?;
                    face_cache.insert(key, mask);
                    mask
                }
            } else {
                0
            };
            faces.push(mask);
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
                    ) << 3)
                    | if properties.can_occlude {
                        CAN_OCCLUDE
                    } else {
                        0
                    },
            );
        }
        Ok(Self {
            registry,
            definitions,
            states: result.into(),
            occlusion_faces: BoundOcclusionFaces {
                states: faces.into(),
            },
        })
    }
    pub fn bind_occlusion_faces(&self) -> BoundOcclusionFaces {
        self.occlusion_faces.clone()
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
            mask: property.mask(),
        }
    }
    /// The states whose block definition can occlude (BlockState.canOcclude).
    pub fn bind_can_occlude(&self) -> BoundBlockProperty {
        BoundBlockProperty {
            states: Arc::clone(&self.states),
            mask: CAN_OCCLUDE,
        }
    }
}
