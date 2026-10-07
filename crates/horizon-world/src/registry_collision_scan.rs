//! Bind the native cursor to actual registry behavior and chunk providers.
use crate::{
    block_collision::{CollisionQueryError, DynamicCollisionWorld},
    block_collision_scan::{BlockCollisionWorld, CollisionBlock},
    collision_context::CollisionContext,
    liquid_collision::LiquidCollisionWorld,
    powder_snow_collision::PowderSnowWorld,
    registry::BlockStateId,
    suffocation::SuffocationRegistry,
};
use horizon_core::{
    geometry::VoxelShape,
    position::{BlockPos, ChunkPos},
};

/// A collision world's resident chunk access. State IDs must belong to the
/// binding's registry. Handles remain valid for the
/// duration of a scan. The chunk-local block-entity provider is deliberately
/// distinct from the global world used for contextual collision shapes.
pub trait CollisionChunks: DynamicCollisionWorld {
    type Chunk: Copy;
    type ChunkWorld: DynamicCollisionWorld<Error = Self::Error>;
    fn collision_chunk(&mut self, position: ChunkPos) -> Result<Option<Self::Chunk>, Self::Error>;
    fn block_state(
        &mut self,
        chunk: Self::Chunk,
        position: BlockPos,
    ) -> Result<BlockStateId, Self::Error>;
    /// Borrowing the provider must not perform a world query. Constant
    /// suffocation predicates do not read any block entity.
    fn chunk_world(&mut self, chunk: Self::Chunk) -> &mut Self::ChunkWorld;
}

pub struct RegistryCollisionWorld<'a, 'r, W> {
    binding: &'a SuffocationRegistry<'r>,
    world: &'a mut W,
}
impl<'a, 'r, W> RegistryCollisionWorld<'a, 'r, W> {
    pub fn new(binding: &'a SuffocationRegistry<'r>, world: &'a mut W) -> std::io::Result<Self> {
        if !binding
            .collisions()
            .cached_shapes()
            .has_canonical_identities()
        {
            return Err(crate::compression::invalid(
                "collision scanning requires canonical shape identities",
            ));
        }
        Ok(Self { binding, world })
    }
}
impl<W, E> BlockCollisionWorld for RegistryCollisionWorld<'_, '_, W>
where
    W: CollisionChunks<Error = E> + LiquidCollisionWorld<Error = E> + PowderSnowWorld<Error = E>,
{
    type Error = CollisionQueryError<E>;
    type Chunk = W::Chunk;
    fn collision_chunk(&mut self, position: ChunkPos) -> Result<Option<Self::Chunk>, Self::Error> {
        self.world
            .collision_chunk(position)
            .map_err(CollisionQueryError::Context)
    }
    fn collision_block(
        &mut self,
        chunk: Self::Chunk,
        position: BlockPos,
    ) -> Result<CollisionBlock, Self::Error> {
        let state = self
            .world
            .block_state(chunk, position)
            .map_err(CollisionQueryError::Context)?;
        self.binding
            .collisions()
            .collision_block(state)
            .map_err(CollisionQueryError::Data)
    }
    fn is_suffocating(
        &mut self,
        chunk: Self::Chunk,
        block: CollisionBlock,
        position: BlockPos,
    ) -> Result<bool, Self::Error> {
        self.binding.is_suffocating(
            self.world.chunk_world(chunk),
            block.state,
            [position.x, position.y, position.z],
        )
    }
    fn context_shape(
        &mut self,
        block: CollisionBlock,
        position: BlockPos,
        context: &CollisionContext<'_>,
    ) -> Result<VoxelShape, Self::Error> {
        self.binding.collisions().collision_shape_with_context(
            self.world,
            block.state,
            position,
            context,
        )
    }
}

impl<W: crate::world_movement::EntityMovementWorld> crate::world_movement::EntityMovementWorld
    for RegistryCollisionWorld<'_, '_, W>
{
    type Error = CollisionQueryError<W::Error>;
    fn entity_collisions(
        &mut self,
        entity: crate::entity_obstruction::EntityIdentity,
        bounds: horizon_core::geometry::Aabb,
    ) -> Result<Vec<VoxelShape>, Self::Error> {
        self.world
            .entity_collisions(entity, bounds)
            .map_err(CollisionQueryError::Context)
    }
    fn movement_border(
        &mut self,
        entity: crate::entity_obstruction::EntityIdentity,
        bounds: horizon_core::geometry::Aabb,
    ) -> Result<Option<VoxelShape>, Self::Error> {
        self.world
            .movement_border(entity, bounds)
            .map_err(CollisionQueryError::Context)
    }
}

impl<W: crate::fluid_occupancy::FluidOccupancyWorld> crate::fluid_occupancy::FluidOccupancyWorld
    for RegistryCollisionWorld<'_, '_, W>
{
    type Error = CollisionQueryError<W::Error>;
    type Chunk = W::Chunk;
    fn minimum_section_y(&mut self) -> Result<i32, Self::Error> {
        self.world
            .minimum_section_y()
            .map_err(CollisionQueryError::Context)
    }
    fn maximum_section_y(&mut self) -> Result<i32, Self::Error> {
        self.world
            .maximum_section_y()
            .map_err(CollisionQueryError::Context)
    }
    fn chunk(&mut self, x: i32, z: i32) -> Result<Self::Chunk, Self::Error> {
        self.world.chunk(x, z).map_err(CollisionQueryError::Context)
    }
    fn section(
        &mut self,
        chunk: Self::Chunk,
        y: i32,
    ) -> Result<&crate::chunk::BlockSection<'_>, Self::Error> {
        self.world
            .section(chunk, y)
            .map_err(CollisionQueryError::Context)
    }
    fn minimum_y(&mut self) -> Result<i32, Self::Error> {
        self.world.minimum_y().map_err(CollisionQueryError::Context)
    }
    fn maximum_y(&mut self) -> Result<i32, Self::Error> {
        self.world.maximum_y().map_err(CollisionQueryError::Context)
    }
}
