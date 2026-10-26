//! Cushion placement and support checks (`Cushion.canBePlacedAt`/`wouldSurviveAt`).
use horizon_core::geometry::{Aabb, ShapeOp, Vec3, VoxelShape};

pub trait World {
    fn outline_bounds(&self, pos: [i32; 3]) -> Option<Aabb>;
    fn collision_shapes(&self, bounds: Aabb) -> Vec<VoxelShape>;
    fn suffocating(&self, pos: [i32; 3]) -> bool;
}

pub fn blocks_in(a: Aabb) -> impl Iterator<Item = [i32; 3]> {
    let (lo, hi) = (a.min_position(), a.max_position());
    let (x0, y0, z0) = (
        lo.x.floor() as i32,
        lo.y.floor() as i32,
        lo.z.floor() as i32,
    );
    let (x1, y1, z1) = (
        hi.x.floor() as i32,
        hi.y.floor() as i32,
        hi.z.floor() as i32,
    );
    (y0..=y1).flat_map(move |y| (z0..=z1).flat_map(move |z| (x0..=x1).map(move |x| [x, y, z])))
}

pub fn survives(world: &impl World, bounds: Aabb) -> bool {
    let (lo, hi) = (bounds.min_position(), bounds.max_position());
    let anchor = Aabb::new(
        Vec3::new(lo.x, lo.y - 0.015625, lo.z),
        Vec3::new(hi.x.next_down(), lo.y, hi.z.next_down()),
    );
    // The extra eighth block expands the scan, not the intersection test.
    let supported = blocks_in(anchor.expand_towards(Vec3::new(0.0, -0.125, 0.0))).any(|p| {
        world
            .outline_bounds(p)
            .is_some_and(|shape| shape.intersects(anchor))
    });
    supported && !blocks_in(bounds.next_deflated()).all(|p| world.suffocating(p))
}

pub fn can_place(world: &impl World, bounds: Aabb) -> bool {
    if !survives(world, bounds) {
        return false;
    }
    let (lo, hi) = (bounds.min_position(), bounds.max_position());
    let slab = Aabb::new(lo, Vec3::new(hi.x, lo.y + 0.015625, hi.z)).next_deflated();
    let Ok(mut remaining) = VoxelShape::from_box(slab) else {
        return false;
    };
    for shape in world.collision_shapes(slab) {
        let Ok(next) = remaining.join(&shape, ShapeOp::ONLY_FIRST) else {
            return false;
        };
        remaining = next;
        if remaining.is_empty() {
            return false;
        }
    }
    true
}
