//! `LocalPlayer.moveTowardsClosestSpace`: the push out of a wall the player's body is in.
use horizon_client::{blocks::BlockFacts, motion::PlayerMotion, world::World};
use horizon_core::position::BlockPos;

/// `LocalPlayer.moveTowardsClosestSpace` at the body's four corners (aiStep, unless a spectator or a
/// passenger): a corner inside a suffocating block sets a 0.1 speed towards the nearest side that is
/// free, on that side's axis; a later corner overwrites an earlier one. Suffocating is a full
/// collision cube that is also a full render cube, as `in_wall` has it, outside the player's
/// collision exclusions (arismoko/horizon#3717).
pub(crate) fn closest_space_nudge(
    p: &PlayerMotion,
    world: &World,
    facts: &BlockFacts,
) -> [Option<f64>; 2] {
    use horizon_core::geometry::Axis;
    let mut out = [None; 2];
    let body = p.bounds();
    let (min_y, max_y) = (body.min(Axis::Y) + 1.0e-7, body.max(Axis::Y) - 1.0e-7);
    let suffocates_at = |x: i32, z: i32| {
        ((min_y.floor() as i32 - 1)..=(max_y.ceil() as i32 + 1)).any(|y| {
            f64::from(y) < max_y
                && f64::from(y + 1) > min_y
                && !p.collision_exclusions.spares(BlockPos::new(x, y, z))
                && {
                    let f = facts.get(world.block(x, y, z));
                    f.full_collision && f.solid_render
                }
        })
    };
    let side = f64::from(p.dimensions.width) * 0.35;
    let (px, pz) = (p.position.x, p.position.z);
    for (x, z) in [
        (px - side, pz + side),
        (px - side, pz - side),
        (px + side, pz - side),
        (px + side, pz + side),
    ] {
        let (bx, bz) = (x.floor() as i32, z.floor() as i32);
        if !suffocates_at(bx, bz) {
            continue;
        }
        let (xd, zd) = (x - f64::from(bx), z - f64::from(bz));
        // WEST, EAST, NORTH, SOUTH: (axis, step, distance to that edge).
        let mut closest = 1.797_693_134_862_315_7e308; // Double.MAX_VALUE
        let mut chosen = None;
        for (axis, step, distance) in [(0, -1, xd), (0, 1, 1.0 - xd), (1, -1, zd), (1, 1, 1.0 - zd)]
        {
            let (nx, nz) = if axis == 0 {
                (bx + step, bz)
            } else {
                (bx, bz + step)
            };
            if distance < closest && !suffocates_at(nx, nz) {
                closest = distance;
                chosen = Some((axis, step));
            }
        }
        if let Some((axis, step)) = chosen {
            out[axis] = Some(0.1 * f64::from(step));
        }
    }
    out
}
