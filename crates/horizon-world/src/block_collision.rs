//! No-entity-context block collision queries over cached and dynamic definitions.
//! Entity movement contexts select different shapes and are separate queries.
use crate::{
    collision::CachedCollisionRegistry,
    dynamic_support::{DynamicSupport, DynamicSupportError, DynamicSupportWorld},
    registry::{BlockRegistry, BlockStateId},
};
use horizon_core::geometry::{Aabb, Direction, Vec3, VoxelShape};
use std::{collections::HashMap, io, sync::Arc};

#[derive(Clone, Copy, Debug)]
pub struct ShulkerCollisionState {
    pub previous_progress: f32,
    pub progress: f32,
}
impl ShulkerCollisionState {
    pub fn bounding_box(self, direction: Direction) -> Aabb {
        // Native getProgress(1) retains this interpolation, including rounding.
        let progress = 0.5f32 * (self.previous_progress + (self.progress - self.previous_progress));
        let maximum = if progress.is_nan() {
            progress
        } else {
            (-1.0f32).max(progress)
        } as f64;
        let minimum = if progress.is_nan() {
            progress
        } else {
            (-1.0f32).min(progress)
        } as f64;
        let [x, y, z] = direction.normal();
        Aabb::new(Vec3::new(-0.5, 0.0, -0.5), Vec3::new(0.5, 1.0, 0.5))
            .expand_towards(Vec3::new(
                f64::from(x) * maximum,
                f64::from(y) * maximum,
                f64::from(z) * maximum,
            ))
            .contract(Vec3::new(
                f64::from(-x) * (1.0 + minimum),
                f64::from(-y) * (1.0 + minimum),
                f64::from(-z) * (1.0 + minimum),
            ))
            .move_by(Vec3::new(0.5, 0.0, 0.5))
    }
}

pub trait DynamicCollisionWorld: DynamicSupportWorld {
    /// One block-entity lookup. None denotes absence or another entity type.
    fn shulker_collision_state(
        &mut self,
        position: [i32; 3],
    ) -> Result<Option<ShulkerCollisionState>, Self::Error>;
}

#[derive(Debug)]
pub enum CollisionQueryError<E> {
    Data(io::Error),
    Dynamic(DynamicSupportError<E>),
    Unsupported(BlockStateId),
}
impl<E: std::fmt::Display> std::fmt::Display for CollisionQueryError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Data(error) => error.fmt(f),
            Self::Dynamic(error) => error.fmt(f),
            Self::Unsupported(state) => write!(f, "unsupported block collision state {}", state.0),
        }
    }
}
impl<E: std::error::Error + 'static> std::error::Error for CollisionQueryError<E> {}

pub struct BlockCollisionRegistry<'r> {
    cached: Arc<CachedCollisionRegistry<'r>>,
    dynamic: HashMap<BlockStateId, DynamicSupport>,
}
impl<'r> BlockCollisionRegistry<'r> {
    pub fn new(cached: Arc<CachedCollisionRegistry<'r>>) -> io::Result<Self> {
        let mut dynamic = HashMap::new();
        for index in 0..cached.registry().len() {
            let id = BlockStateId(index as u32);
            if cached.cached_shape(id)?.is_none() {
                if let Some(rule) =
                    DynamicSupport::from_state(cached.registry().state(id).unwrap())?
                {
                    dynamic.insert(id, rule);
                }
            }
        }
        Ok(Self { cached, dynamic })
    }
    pub fn registry(&self) -> &'r BlockRegistry {
        self.cached.registry()
    }
    pub fn collision_shape<W: DynamicCollisionWorld>(
        &self,
        world: &mut W,
        state: BlockStateId,
        position: [i32; 3],
    ) -> Result<VoxelShape, CollisionQueryError<W::Error>> {
        if let Some(shape) = self
            .cached
            .cached_shape(state)
            .map_err(CollisionQueryError::Data)?
        {
            return Ok(shape.clone());
        }
        self.dynamic
            .get(&state)
            .ok_or(CollisionQueryError::Unsupported(state))?
            .collision_shape(world, position)
            .map_err(CollisionQueryError::Dynamic)
    }
}
