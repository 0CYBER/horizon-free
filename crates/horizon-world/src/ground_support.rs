//! Supporting-block selection and Entity ground-state transitions.
use crate::{
    block_collision_scan::{BlockCollisionScan, BlockCollisionWorld, CollisionScanError},
    collision_context::CollisionContext,
};
use horizon_core::{
    geometry::{Aabb, Axis, Vec3},
    position::BlockPos,
};

fn compare_positions(left: BlockPos, right: BlockPos) -> i32 {
    // Vec3i.compareTo uses signed wrapping subtraction in Y/Z/X order.
    if left.y != right.y {
        left.y.wrapping_sub(right.y)
    } else if left.z != right.z {
        left.z.wrapping_sub(right.z)
    } else {
        left.x.wrapping_sub(right.x)
    }
}
pub fn find_supporting_block<W: BlockCollisionWorld>(
    world: &mut W,
    context: CollisionContext<'_>,
    entity_position: Vec3,
    bounds: Aabb,
    candidate_budget: usize,
) -> Result<Option<BlockPos>, CollisionScanError<W::Error>> {
    let mut selected = None;
    let mut closest = f64::MAX;
    for hit in BlockCollisionScan::new(world, context, bounds, false, candidate_budget)? {
        let p = hit?.position;
        let dx = f64::from(p.x) + 0.5 - entity_position.x;
        let dy = f64::from(p.y) + 0.5 - entity_position.y;
        let dz = f64::from(p.z) + 0.5 - entity_position.z;
        let distance = dx * dx + dy * dy + dz * dz;
        if distance < closest
            || distance == closest && selected.is_none_or(|old| compare_positions(old, p) < 0)
        {
            selected = Some(p);
            closest = distance;
        }
    }
    Ok(selected)
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GroundState {
    pub on_ground: bool,
    pub horizontal_collision: bool,
    pub on_ground_no_blocks: bool,
    pub supporting_block: Option<BlockPos>,
}
impl GroundState {
    /// Entity.setOnGroundWithMovement writes these flags before support queries.
    /// A failed query preserves any earlier writes, as in the native method.
    pub fn set_with_movement<E>(
        &mut self,
        on_ground: bool,
        horizontal_collision: bool,
        bounds: Aabb,
        movement: Option<Vec3>,
        find: impl FnMut(Aabb) -> Result<Option<BlockPos>, E>,
    ) -> Result<(), E> {
        self.on_ground = on_ground;
        self.horizontal_collision = horizontal_collision;
        self.check_support(on_ground, bounds, movement, find)
    }
    /// The optional movement is native's nullable movement argument. When the
    /// first query misses, it can retry at the pre-horizontal-movement footprint.
    pub fn check_support<E>(
        &mut self,
        on_ground: bool,
        bounds: Aabb,
        movement: Option<Vec3>,
        mut find: impl FnMut(Aabb) -> Result<Option<BlockPos>, E>,
    ) -> Result<(), E> {
        if on_ground {
            let min = bounds.min_position();
            let max = bounds.max_position();
            let foot = Aabb::new(
                Vec3::new(min.x, min.y - 1.0e-6, min.z),
                Vec3::new(max.x, bounds.min(Axis::Y), max.z),
            );
            let mut found = find(foot)?;
            if found.is_some() || self.on_ground_no_blocks {
                self.supporting_block = found;
            } else if let Some(movement) = movement {
                found = find(foot.move_by(Vec3::new(-movement.x, 0.0, -movement.z)))?;
                self.supporting_block = found;
            }
            self.on_ground_no_blocks = found.is_none();
        } else {
            self.on_ground_no_blocks = false;
            self.supporting_block = None;
        }
        Ok(())
    }
}
