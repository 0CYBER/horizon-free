//! EntityGetter collision shape selection over an ordered entity snapshot.
use crate::entity_obstruction::EntityIdentity;
use horizon_core::geometry::{Aabb, ShapeError, Vec3, VoxelShape};

#[derive(Clone, Copy, Debug)]
pub struct CollisionCandidate {
    pub identity: EntityIdentity,
    pub bounds: Aabb,
    pub spectator: bool,
}
pub trait EntityCollisionWorld {
    type Error;
    /// Query the inflated bounds, preserving native entity iteration order.
    /// The implementation may return a broad-phase superset. Snapshot entities
    /// must remain available for the following collision-policy callbacks.
    fn collision_candidates(
        &mut self,
        source: Option<EntityIdentity>,
        bounds: Aabb,
    ) -> Result<Vec<CollisionCandidate>, Self::Error>;
    /// With a source, invoke source.canCollideWith(candidate). Without one,
    /// invoke candidate.canBeCollidedWith(null). Entity-specific policies and
    /// vehicle relationships belong to the provider, and run only after filters.
    fn can_collide(
        &mut self,
        source: Option<EntityIdentity>,
        candidate: EntityIdentity,
    ) -> Result<bool, Self::Error>;
}
#[derive(Debug)]
pub enum EntityCollisionError<E> {
    World(E),
    Shape(ShapeError),
}
impl<E: std::fmt::Display> std::fmt::Display for EntityCollisionError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::World(e) => e.fmt(f),
            Self::Shape(e) => e.fmt(f),
        }
    }
}
impl<E: std::error::Error + 'static> std::error::Error for EntityCollisionError<E> {}

pub fn entity_collision_shapes<W: EntityCollisionWorld>(
    world: &mut W,
    source: Option<EntityIdentity>,
    bounds: Aabb,
) -> Result<Vec<VoxelShape>, EntityCollisionError<W::Error>> {
    let size = bounds.size();
    if (size.x + size.y + size.z) / 3.0 < 1.0e-7 {
        return Ok(Vec::new());
    }
    let query = bounds.inflate(Vec3::new(1.0e-7, 1.0e-7, 1.0e-7));
    let candidates = world
        .collision_candidates(source, query)
        .map_err(EntityCollisionError::World)?;
    let mut selected = Vec::new();
    for candidate in candidates {
        if source == Some(candidate.identity)
            || !candidate.bounds.intersects(query)
            || candidate.spectator
        {
            continue;
        }
        if world
            .can_collide(source, candidate.identity)
            .map_err(EntityCollisionError::World)?
        {
            selected.push(candidate.bounds);
        }
    }
    selected
        .into_iter()
        .map(|bounds| VoxelShape::from_box(bounds).map_err(EntityCollisionError::Shape))
        .collect()
}
