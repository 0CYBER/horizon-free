//! Collision queries against the same resident chunks used by the renderer.
//! The development scene has no entities, block entities or world border.
//! Entity-dependent block rules fail explicitly until an entity runtime exists.
use crate::{Result, world::World};
use horizon_core::{
    geometry::{Aabb, Vec3, VoxelShape},
    position::{BlockPos, ChunkPos},
};
use horizon_world::{
    block_collision::{BlockCollisionRegistry, DynamicCollisionWorld, ShulkerCollisionState},
    block_collision_scan::{BlockCollisionWorld, CollisionBlock},
    collision::CachedCollisionRegistry,
    collision_context::CollisionContext,
    dynamic_support::DynamicSupportWorld,
    entity_obstruction::EntityIdentity,
    fluid::FluidState,
    liquid_collision::LiquidCollisionWorld,
    nbt::{self, Limits},
    powder_snow_collision::{PowderSnowEntity, PowderSnowWorld},
    registry::BlockRegistry,
    world_movement::{
        CollisionCheckRequest, EntityMovementWorld, MovementRequest, no_world_collision,
        resolve_world_movement,
    },
};
use std::{fs, io, path::Path, sync::Arc};

pub fn load<'r>(root: &Path, registry: &'r BlockRegistry) -> Result<BlockCollisionRegistry<'r>> {
    let data = nbt::read_named(
        &fs::read(root.join("cached-collision.nbt"))?,
        Limits::default(),
    )?
    .0
    .tag;
    let cached = CachedCollisionRegistry::from_nbt(&data, registry)?;
    if !cached.has_canonical_identities() {
        return Err("scene collisions require canonical shape identities".into());
    }
    Ok(BlockCollisionRegistry::new(Arc::new(cached))?)
}

pub struct SceneCollisions<'w, 'r> {
    pub world: &'w World,
    pub binding: &'w BlockCollisionRegistry<'r>,
}
impl SceneCollisions<'_, '_> {
    /// Position contexts permit geometric movement probes without inventing a
    /// player, inventory, effects or fluid-standing behavior.
    pub fn resolve(
        &mut self,
        bounds: Aabb,
        movement: Vec3,
        max_step: f32,
        on_ground: bool,
    ) -> Result<Vec3> {
        Ok(resolve_world_movement(
            self,
            MovementRequest {
                entity: EntityIdentity(0),
                context: CollisionContext::position(bounds.min_position().y),
                bounds,
                movement,
                max_step,
                on_ground,
                candidate_budget: 1_000_000,
            },
        )?)
    }
    pub fn unobstructed(&mut self, bounds: Aabb) -> Result<bool> {
        Ok(no_world_collision(
            self,
            CollisionCheckRequest {
                entity: EntityIdentity(0),
                context: CollisionContext::position(bounds.min_position().y),
                bounds,
                candidate_budget: 1_000_000,
            },
        )?)
    }
}
impl BlockCollisionWorld for SceneCollisions<'_, '_> {
    type Error = io::Error;
    type Chunk = ChunkPos;
    fn collision_chunk(&mut self, p: ChunkPos) -> io::Result<Option<ChunkPos>> {
        Ok(self.world.chunks.contains_key(&(p.x, p.z)).then_some(p))
    }
    fn collision_block(&mut self, chunk: ChunkPos, p: BlockPos) -> io::Result<CollisionBlock> {
        let chunk = self
            .world
            .chunks
            .get(&(chunk.x, chunk.z))
            .ok_or_else(|| io::Error::other("resident collision chunk disappeared"))?;
        self.binding.collision_block(chunk.block(p.x, p.y, p.z))
    }
    fn is_suffocating(&mut self, _: ChunkPos, _: CollisionBlock, _: BlockPos) -> io::Result<bool> {
        Err(io::Error::other(
            "scene does not provide suffocation queries",
        ))
    }
    fn context_shape(
        &mut self,
        block: CollisionBlock,
        p: BlockPos,
        context: &CollisionContext<'_>,
    ) -> io::Result<VoxelShape> {
        self.binding
            .collision_shape_with_context(self, block.state, p, context)
            .map_err(io::Error::other)
    }
}
impl EntityMovementWorld for SceneCollisions<'_, '_> {
    type Error = io::Error;
    fn entity_collisions(&mut self, _: EntityIdentity, _: Aabb) -> io::Result<Vec<VoxelShape>> {
        Ok(Vec::new())
    }
    fn movement_border(&mut self, _: EntityIdentity, _: Aabb) -> io::Result<Option<VoxelShape>> {
        Ok(None)
    }
}
impl DynamicSupportWorld for SceneCollisions<'_, '_> {
    type Error = io::Error;
    fn shulker_is_closed(&mut self, _: [i32; 3]) -> io::Result<Option<bool>> {
        Ok(None)
    }
    fn moving_piston_shape(&mut self, _: [i32; 3]) -> io::Result<Option<VoxelShape>> {
        Ok(None)
    }
}
impl DynamicCollisionWorld for SceneCollisions<'_, '_> {
    fn shulker_collision_state(
        &mut self,
        _: [i32; 3],
    ) -> io::Result<Option<ShulkerCollisionState>> {
        Ok(None)
    }
}
fn missing_entity<T>() -> io::Result<T> {
    Err(io::Error::other(
        "scene collision query requires an entity runtime",
    ))
}
impl LiquidCollisionWorld for SceneCollisions<'_, '_> {
    type Error = io::Error;
    fn entity_liquid_shape(&mut self, _: EntityIdentity) -> io::Result<VoxelShape> {
        missing_entity()
    }
    fn fluid(&mut self, p: BlockPos) -> io::Result<FluidState> {
        FluidState::from_block(
            self.binding
                .registry()
                .state(self.world.block(p.x, p.y, p.z))
                .ok_or_else(|| io::Error::other("scene fluid state outside registry"))?,
        )
    }
    fn entity_can_stand(&mut self, _: EntityIdentity, _: FluidState) -> io::Result<bool> {
        missing_entity()
    }
}
impl PowderSnowWorld for SceneCollisions<'_, '_> {
    type Error = io::Error;
    fn powder_snow_entity(&mut self, _: EntityIdentity) -> io::Result<PowderSnowEntity> {
        missing_entity()
    }
    fn powder_snow_walkable_tag(&mut self, _: EntityIdentity) -> io::Result<bool> {
        missing_entity()
    }
    fn feet_are_leather_boots(&mut self, _: EntityIdentity) -> io::Result<bool> {
        missing_entity()
    }
}
