//! Entity fluid scan bounds and the boat passenger waterline restriction.
use horizon_core::geometry::{Aabb, Vec3};
pub trait FluidBoxWorld {
    type Error;
    type Vehicle: Copy;
    fn bounding_box(&self) -> Aabb;
    fn vehicle(&mut self) -> Result<Option<Self::Vehicle>, Self::Error>;
    fn modify_passenger_box(
        &mut self,
        vehicle: Self::Vehicle,
        bounds: Aabb,
    ) -> Result<Option<Aabb>, Self::Error>;
}
pub fn interaction_box<W: FluidBoxWorld>(world: &mut W) -> Result<Option<Aabb>, W::Error> {
    let bounds = world.bounding_box().deflate(Vec3::new(0.001, 0.001, 0.001));
    match world.vehicle()? {
        None => Ok(Some(bounds)),
        Some(vehicle) => world.modify_passenger_box(vehicle, bounds),
    }
}
pub trait BoatFluidBoxWorld {
    type Error;
    fn is_under_water(&mut self) -> Result<bool, Self::Error>;
    fn boat_bounds(&self) -> Aabb;
}
/// Submerged boats preserve the passenger bounds. Otherwise only the part
/// strictly above the boat's top can interact with fluid; it may be absent.
pub fn boat_passenger_box<W: BoatFluidBoxWorld>(
    world: &mut W,
    bounds: Aabb,
) -> Result<Option<Aabb>, W::Error> {
    if world.is_under_water()? {
        return Ok(Some(bounds));
    }
    let boat_top = world.boat_bounds().max_position().y;
    let mut min = bounds.min_position();
    let max = bounds.max_position();
    if boat_top >= max.y {
        return Ok(None);
    }
    min.y = java_max(min.y, boat_top);
    Ok(Some(Aabb::new(min, max)))
}
fn java_max(a: f64, b: f64) -> f64 {
    if a.is_nan() {
        a
    } else if b.is_nan() {
        b
    } else if a == 0.0 && b == 0.0 {
        f64::from_bits(a.to_bits() & b.to_bits())
    } else {
        a.max(b)
    }
}
