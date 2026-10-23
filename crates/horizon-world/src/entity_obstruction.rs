//! Entity obstruction against a voxel shape. Entity storage, lifecycle and
//! vehicle-tree maintenance supply the immutable candidate snapshots here.
use horizon_core::geometry::{Aabb, ShapeError, ShapeOp, VoxelShape};

/// Object identity within the caller's entity snapshot, including its generation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EntityIdentity(pub u64);
#[derive(Clone, Copy, Debug)]
pub struct ObstructionEntity {
    pub identity: EntityIdentity,
    pub root_vehicle: EntityIdentity,
    pub bounds: Aabb,
    pub removed: bool,
    pub spectator: bool,
    pub blocks_building: bool,
}
pub trait EntityObstructionWorld {
    type Error;
    /// Preserve native query order. Implementations may return a broad-phase
    /// superset: exclusion, spectator and exact AABB filters are enforced below.
    fn obstruction_candidates(
        &mut self,
        excluded: Option<EntityIdentity>,
        bounds: Aabb,
    ) -> Result<Vec<ObstructionEntity>, Self::Error>;
}
#[derive(Debug)]
pub enum ObstructionError<E> {
    World(E),
    Shape(ShapeError),
}
impl<E: std::fmt::Display> std::fmt::Display for ObstructionError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::World(e) => e.fmt(f),
            Self::Shape(e) => e.fmt(f),
        }
    }
}
impl<E: std::error::Error + 'static> std::error::Error for ObstructionError<E> {}

pub fn is_unobstructed<W: EntityObstructionWorld>(
    world: &mut W,
    excluded: Option<&ObstructionEntity>,
    shape: &VoxelShape,
) -> Result<bool, ObstructionError<W::Error>> {
    let Some(bounds) = shape.bounds() else {
        return Ok(true);
    };
    let candidates = world
        .obstruction_candidates(excluded.map(|e| e.identity), bounds)
        .map_err(ObstructionError::World)?;
    for entity in candidates {
        if excluded.is_some_and(|e| e.identity == entity.identity)
            || entity.spectator
            || !entity.bounds.intersects(bounds)
        {
            continue;
        }
        if entity.removed
            || !entity.blocks_building
            || excluded.is_some_and(|e| e.root_vehicle == entity.root_vehicle)
        {
            continue;
        }
        let other = VoxelShape::from_box(entity.bounds).map_err(ObstructionError::Shape)?;
        if shape
            .join_is_not_empty(&other, ShapeOp::AND)
            .map_err(ObstructionError::Shape)?
        {
            return Ok(false);
        }
    }
    Ok(true)
}
