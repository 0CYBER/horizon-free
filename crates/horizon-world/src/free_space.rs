//! Entity.isFree: offset collision clearance followed by resident liquid checks.
use crate::{
    block_collision_scan::{BlockCollisionWorld, CollisionScanError},
    fluid_occupancy::{
        FluidOccupancyError, FluidOccupancyWorld, FluidPresence, contains_any_liquid,
    },
    world_movement::{CollisionCheckRequest, EntityMovementWorld, no_world_collision},
};
use horizon_core::geometry::Vec3;
use std::fmt;
#[derive(Debug)]
pub enum FreeSpaceError<E> {
    Collision(CollisionScanError<E>),
    Fluid(FluidOccupancyError<E>),
}
impl<E: fmt::Display> fmt::Display for FreeSpaceError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Collision(e) => e.fmt(f),
            Self::Fluid(e) => e.fmt(f),
        }
    }
}
impl<E: std::error::Error + 'static> std::error::Error for FreeSpaceError<E> {}
pub fn offset_is_free<W, E>(
    world: &mut W,
    fluids: &FluidPresence<'_>,
    mut request: CollisionCheckRequest<'_>,
    offset: Vec3,
    fluid_budget: usize,
) -> Result<bool, FreeSpaceError<E>>
where
    W: BlockCollisionWorld<Error = E>
        + EntityMovementWorld<Error = E>
        + FluidOccupancyWorld<Error = E>,
{
    request.bounds = request.bounds.move_by(offset);
    let bounds = request.bounds;
    if !no_world_collision(world, request).map_err(FreeSpaceError::Collision)? {
        return Ok(false);
    }
    Ok(!contains_any_liquid(world, fluids, bounds, fluid_budget).map_err(FreeSpaceError::Fluid)?)
}
