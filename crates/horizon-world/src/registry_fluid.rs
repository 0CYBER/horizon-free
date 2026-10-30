//! Registered fluid contents, flow tags and native block support composition.
use crate::{
    block_support::{BlockSupportRegistry, SupportQueryError, SupportType},
    compression::invalid,
    dynamic_support::DynamicSupportWorld,
    fluid::{FluidHeightWorld, FluidState},
    fluid_flow::FluidFlowWorld,
    registry::{BlockRegistry, BlockStateId},
};
use horizon_core::{geometry::Direction, position::BlockPos};
use std::{collections::BTreeSet, fmt, io};
#[derive(Clone, Copy)]
pub struct FluidBlock {
    id: BlockStateId,
    fluid: FluidState,
    blocks_flow: bool,
    ice: bool,
}
pub struct FluidRegistry<'r> {
    registry: &'r BlockRegistry,
    states: Vec<FluidBlock>,
}
impl<'r> FluidRegistry<'r> {
    /// Rebuild on BLOCKS_FLUID_FLOW tag reload. Ice names come from actual class
    /// membership; name suffixes do not distinguish ice from packed/blue ice.
    pub fn new(
        registry: &'r BlockRegistry,
        blocks_flow: &BTreeSet<String>,
        ice: &BTreeSet<String>,
    ) -> io::Result<Self> {
        for names in [blocks_flow, ice] {
            if names.iter().any(|n| registry.default_state(n).is_none()) {
                return Err(invalid("unknown fluid block member"));
            }
        }
        let states = (0..registry.len())
            .map(|i| {
                let id = BlockStateId(i as u32);
                let s = registry.state(id).unwrap();
                Ok(FluidBlock {
                    id,
                    fluid: FluidState::from_block(s)?,
                    blocks_flow: blocks_flow.contains(&s.name),
                    ice: ice.contains(&s.name),
                })
            })
            .collect::<io::Result<Vec<_>>>()?;
        Ok(Self { registry, states })
    }
    pub fn registry(&self) -> &'r BlockRegistry {
        self.registry
    }
    fn state(&self, id: BlockStateId) -> io::Result<FluidBlock> {
        self.states
            .get(id.0 as usize)
            .copied()
            .ok_or_else(|| invalid("fluid block state outside registry"))
    }
    pub fn fluid(&self, id: BlockStateId) -> io::Result<FluidState> {
        Ok(self.state(id)?.fluid)
    }
    pub fn blocks_flow(&self, id: BlockStateId) -> io::Result<bool> {
        Ok(self.state(id)?.blocks_flow)
    }
}
/// Both queries return state IDs from the same registry. Separate entry points
/// preserve native world dispatch when reading a block versus its fluid cell.
pub trait FluidBlockWorld: DynamicSupportWorld {
    fn block_state(&mut self, position: BlockPos) -> Result<BlockStateId, Self::Error>;
    fn fluid_block_state(&mut self, position: BlockPos) -> Result<BlockStateId, Self::Error>;
}
#[derive(Debug)]
pub enum FluidQueryError<E> {
    World(E),
    Data(io::Error),
    Support(SupportQueryError<E>),
}
impl<E: fmt::Display> fmt::Display for FluidQueryError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::World(e) => e.fmt(f),
            Self::Data(e) => e.fmt(f),
            Self::Support(e) => e.fmt(f),
        }
    }
}
impl<E: std::error::Error + 'static> std::error::Error for FluidQueryError<E> {}
pub struct RegistryFluidWorld<'a, 'r, W> {
    registry: &'a FluidRegistry<'r>,
    support: &'a BlockSupportRegistry<'r>,
    world: &'a mut W,
}
impl<'a, 'r, W> RegistryFluidWorld<'a, 'r, W> {
    pub fn new(
        registry: &'a FluidRegistry<'r>,
        support: &'a BlockSupportRegistry<'r>,
        world: &'a mut W,
    ) -> io::Result<Self> {
        if !std::ptr::eq(registry.registry(), support.registry()) {
            return Err(invalid("fluid and support registry identities differ"));
        }
        Ok(Self {
            registry,
            support,
            world,
        })
    }
}
impl<W: FluidBlockWorld> FluidFlowWorld for RegistryFluidWorld<'_, '_, W> {
    type Error = FluidQueryError<W::Error>;
    type Block = FluidBlock;
    fn fluid(&mut self, p: BlockPos) -> Result<FluidState, Self::Error> {
        let id = self
            .world
            .fluid_block_state(p)
            .map_err(FluidQueryError::World)?;
        self.registry.fluid(id).map_err(FluidQueryError::Data)
    }
    fn block(&mut self, p: BlockPos) -> Result<FluidBlock, Self::Error> {
        let id = self.world.block_state(p).map_err(FluidQueryError::World)?;
        self.registry.state(id).map_err(FluidQueryError::Data)
    }
    fn blocks_fluid_flow(&self, s: FluidBlock) -> bool {
        s.blocks_flow
    }
    fn is_ice_class(&self, s: FluidBlock) -> bool {
        s.ice
    }
    fn is_face_sturdy(
        &mut self,
        s: FluidBlock,
        p: BlockPos,
        d: Direction,
    ) -> Result<bool, Self::Error> {
        self.support
            .is_face_sturdy(self.world, s.id, [p.x, p.y, p.z], d, SupportType::Full)
            .map_err(FluidQueryError::Support)
    }
}
impl<W: FluidBlockWorld> FluidHeightWorld for RegistryFluidWorld<'_, '_, W> {
    type Error = FluidQueryError<W::Error>;
    fn fluid(&mut self, p: [i32; 3]) -> Result<FluidState, Self::Error> {
        FluidFlowWorld::fluid(self, BlockPos::new(p[0], p[1], p[2]))
    }
    fn sturdy_down(&mut self, p: [i32; 3]) -> Result<bool, Self::Error> {
        let p = BlockPos::new(p[0], p[1], p[2]);
        let s = FluidFlowWorld::block(self, p)?;
        self.is_face_sturdy(s, p, Direction::Down)
    }
}

use crate::entity_fluid::{FluidInteractionWorld, FluidMembership, FluidSections, FluidTagId};
use horizon_core::geometry::{Aabb, Vec3};
/// Entity fields and loaded chunk views needed by the registered immersion scan.
/// Cell height/current behavior comes from this module's registry/support binding.
pub trait FluidEntityWorld: FluidBlockWorld + FluidMembership {
    type Chunk: Copy;
    fn interaction_box(&mut self) -> Result<Option<Aabb>, Self::Error>;
    fn loaded_chunk(&mut self, x: i32, z: i32) -> Result<Option<Self::Chunk>, Self::Error>;
    fn chunk_sections(&mut self, chunk: Self::Chunk) -> Result<FluidSections<'_>, Self::Error>;
    fn bounding_box_min_y(&self) -> f64;
    fn block_x(&self) -> i32;
    fn eye_y(&mut self) -> Result<f64, Self::Error>;
    fn block_z(&self) -> i32;
}
impl<W: FluidMembership> FluidMembership for RegistryFluidWorld<'_, '_, W> {
    fn fluid_in_tag(&self, fluid: crate::fluid::FluidKind, tag: FluidTagId) -> bool {
        self.world.fluid_in_tag(fluid, tag)
    }
}
impl<W: FluidEntityWorld> FluidInteractionWorld for RegistryFluidWorld<'_, '_, W> {
    type Error = FluidQueryError<W::Error>;
    type Chunk = W::Chunk;
    fn interaction_box(&mut self) -> Result<Option<Aabb>, Self::Error> {
        self.world.interaction_box().map_err(FluidQueryError::World)
    }
    fn loaded_chunk(&mut self, x: i32, z: i32) -> Result<Option<Self::Chunk>, Self::Error> {
        self.world
            .loaded_chunk(x, z)
            .map_err(FluidQueryError::World)
    }
    fn chunk_sections(&mut self, c: Self::Chunk) -> Result<FluidSections<'_>, Self::Error> {
        self.world.chunk_sections(c).map_err(FluidQueryError::World)
    }
    fn bounding_box_min_y(&self) -> f64 {
        self.world.bounding_box_min_y()
    }
    fn block_x(&self) -> i32 {
        self.world.block_x()
    }
    fn eye_y(&mut self) -> Result<f64, Self::Error> {
        self.world.eye_y().map_err(FluidQueryError::World)
    }
    fn block_z(&self) -> i32 {
        self.world.block_z()
    }
    fn fluid_state(&mut self, p: BlockPos) -> Result<FluidState, Self::Error> {
        FluidFlowWorld::fluid(self, p)
    }
    fn fluid_height(&mut self, s: FluidState, p: BlockPos) -> Result<f32, Self::Error> {
        s.height_at(self, [p.x, p.y, p.z])
    }
    fn camera_height(&mut self, s: FluidState, p: BlockPos) -> Result<f32, Self::Error> {
        s.camera_height_at(self, [p.x, p.y, p.z])
    }
    fn flow(&mut self, s: FluidState, p: BlockPos) -> Result<Vec3, Self::Error> {
        crate::fluid_flow::flow_at(self, s, p)
    }
}
